mod agents;
mod anchor;
mod assist;
mod capture;
mod chord;
mod clipboard;
mod element;
mod history;
mod image_ops;
mod keyframes;
mod live;
mod login;
mod mcp;
mod overlay;
mod permissions;
mod platform;
mod pointer;
mod privacy;
mod prompt;
mod session;
mod sessions;
mod settings;
mod stats;
mod tools;
mod tray;
mod update;
mod voice;

use prompt::{Label, SessionExport};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use session::{now_ms, App, Phase};
use settings::Settings;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager};
use voice::ptt::TalkMode;

/// How long the hidden floating windows (HUD, overlay) are kept after they were last hidden or a session last ended.
/// The speech model has its own setting, free_memory_minutes.
const REAP_AFTER: std::time::Duration = std::time::Duration::from_secs(60);

/// The pulse to the HUD and the menu bar clock while a session runs.
const TICK: std::time::Duration = std::time::Duration::from_millis(100);
/// While an Ask listens, the session's state (its `loudness`) goes out this often: about 30 times a second.
const LEVEL_TICK: std::time::Duration = std::time::Duration::from_millis(33);

/// When the idle scheduler looks again.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Next {
    /// A session runs: pulse again after this long.
    Tick(std::time::Duration),
    /// Nothing runs: sleep until then, or until something wakes the loop.
    At(std::time::Instant),
    /// Nothing is held: sleep until something wakes the loop.
    Never,
}

/// What the idle scheduler sees.
#[derive(Debug, Clone, Copy, Default)]
struct Quiet {
    /// A session records, loads or finishes.
    busy: bool,
    /// A speech setup the person started runs: nothing is let go meanwhile, and its end wakes the loop.
    hold: bool,
    /// When the HUD or overlay was last hidden, or a session last ended.
    hidden: Option<std::time::Instant>,
    /// `App::engine_used`: when the model in memory was last used, none when none is.
    used: Option<std::time::Instant>,
    /// free_memory_minutes.
    minutes: u32,
    /// `App::busy`: also a History entry transcribed again; its end notes a use, which wakes the loop.
    engine_busy: bool,
}

/// What is due `now`, and when to look again. The next time is never already past, so the loop cannot spin.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Due {
    reap: bool,
    free: bool,
    next: Next,
}

fn due(now: std::time::Instant, q: &Quiet) -> Due {
    if q.busy {
        return Due { reap: false, free: false, next: Next::Tick(TICK) };
    }
    if q.hold {
        return Due { reap: false, free: false, next: Next::Never };
    }
    let reap_at = q.hidden.map(|h| h + REAP_AFTER);
    let reap = reap_at.is_some_and(|t| now >= t);
    let free = free_due(now, q.used, q.minutes, q.engine_busy);
    // The time `free_due` turns true, from the same inputs.
    let after = std::time::Duration::from_secs(u64::from(q.minutes) * 60);
    let free_at = q.used.filter(|_| !q.engine_busy && q.minutes > 0).map(|u| u + after);
    let next = [reap_at.filter(|_| !reap), free_at.filter(|_| !free)].into_iter().flatten().min();
    Due { reap, free, next: next.map_or(Next::Never, Next::At) }
}

/// The AI's helper, last used at `used` (`Live::idle_since`): whether it is due to be let go `now`, and else when it
/// will be.
fn assist_due(now: std::time::Instant, used: Option<std::time::Instant>) -> (bool, Option<std::time::Instant>) {
    match used {
        Some(u) if assist::idle_due(now, u) => (true, None),
        Some(u) => (false, Some(u + assist::IDLE)),
        None => (false, None),
    }
}

/// `next`, or `at` when that comes sooner. A pulse stays a pulse.
fn sooner(next: Next, at: Option<std::time::Instant>) -> Next {
    match (next, at) {
        (Next::Tick(d), _) => Next::Tick(d),
        (n, None) => n,
        (Next::Never, Some(t)) => Next::At(t),
        (Next::At(a), Some(t)) => Next::At(a.min(t)),
    }
}

/// Wakes the idle scheduler: a session phase changed, the engine was used, a window hid, a setting changed.
struct Waker {
    seen: std::sync::Mutex<u64>,
    cv: std::sync::Condvar,
}

impl Waker {
    const fn new() -> Self {
        Self { seen: std::sync::Mutex::new(0), cv: std::sync::Condvar::new() }
    }

    fn gen(&self) -> u64 {
        *self.seen.lock().unwrap()
    }

    fn wake(&self) {
        *self.seen.lock().unwrap() += 1;
        self.cv.notify_all();
    }

    /// Sleep as `next` says, cut short by a wake after `gen` was read. True when woken.
    fn wait(&self, gen: u64, next: Next) -> bool {
        let g = self.seen.lock().unwrap();
        match next {
            Next::Tick(d) => {
                drop(g);
                std::thread::sleep(d);
                false
            }
            Next::At(t) => {
                let left = t.saturating_duration_since(std::time::Instant::now());
                !self.cv.wait_timeout_while(g, left, |s| *s == gen).unwrap().1.timed_out()
            }
            Next::Never => {
                drop(self.cv.wait_while(g, |s| *s == gen).unwrap());
                true
            }
        }
    }
}

static WAKE: Waker = Waker::new();
static HIDDEN: std::sync::Mutex<Option<std::time::Instant>> = std::sync::Mutex::new(None);

/// Something the idle scheduler decides on changed: it looks again now.
pub(crate) fn wake() {
    WAKE.wake();
}

/// The HUD or overlay just hid (or a session ended): it is let go REAP_AFTER from now unless shown again.
fn note_hidden() {
    *HIDDEN.lock().unwrap() = Some(std::time::Instant::now());
    wake();
}

/// Whether the speech model should be let go `now`: one is in memory, last `used` at least the `minutes` the person chose
/// ago (0 means never), and nothing is recording, finishing or transcribed again. However it was loaded.
fn free_due(now: std::time::Instant, used: Option<std::time::Instant>, minutes: u32, busy: bool) -> bool {
    let after = std::time::Duration::from_secs(u64::from(minutes) * 60);
    !busy && minutes > 0 && used.is_some_and(|u| now.saturating_duration_since(u) >= after)
}

/// A hold shorter than this is a tap: the session keeps recording until the next press.
const HOLD_MS: u64 = 350;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    Session,
    Full,
    Region,
    Zoom,
    Ask,
}

pub fn show_window(app: &AppHandle, label: &str) {
    if let Some(w) = platform::ensure(app, label) {
        // History's Paste into App goes back to the app that was in front when it opened.
        if label == "result" && !w.is_visible().unwrap_or(false) {
            platform::remember_front();
        }
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
        if platform::DESK.contains(&label) {
            platform::dock(app, None);
        }
    }
}

fn hide_window(app: &AppHandle, label: &str) {
    if let Some(w) = app.get_webview_window(label) {
        let _ = w.hide();
        if platform::DESK.contains(&label) {
            platform::dock(app, Some(label));
        }
        if matches!(label, "indicator" | "overlay" | "live") {
            note_hidden();
        }
    }
}

pub fn clock(ms: u64) -> String {
    let s = ms / 1000;
    format!("{}:{:02}", s / 60, s % 60)
}

fn emit_state(app: &AppHandle, svc: &App) {
    let st = svc.ui();
    tray::refresh(app, &st, svc.last_export().is_some());
    let _ = app.emit("session://state", &st);
}

fn emit_state_now(app: &AppHandle, svc: &App) {
    let _ = app.emit("session://state", &svc.ui());
}

fn recording(svc: &App) -> bool {
    matches!(svc.ui().phase, Phase::Recording | Phase::Loading)
}

/// No recording runs or is finishing.
fn idle(svc: &App) -> bool {
    matches!(svc.ui().phase, Phase::Idle | Phase::Ready | Phase::Error)
}

/// A question is being said: the recording under way is the Ask loop's.
fn asking(svc: &App) -> bool {
    recording(svc) && svc.purpose() == session::Purpose::Ask
}

/// What a chord does, given what is recording: the Ask key asks unless a dictation records or onboarding is testing
/// keys, and every other chord leaves a question being said alone. None when the chord does nothing.
fn routed(action: Action, rec: bool, asking: bool, testing: bool) -> Option<bool> {
    match action {
        Action::Ask if testing || (rec && !asking) => None,
        Action::Ask => Some(true),
        _ if asking => None,
        _ => Some(false),
    }
}

/// The Ask key: down starts a question, or ends the one being said; up ends a held one, as dictation's key does.
fn ask_key(app: &AppHandle, svc: &Arc<App>, pressed: bool) {
    let Some(live) = app.try_state::<Arc<live::Live>>() else {
        return;
    };
    let live = live.inner().clone();
    if pressed {
        if live.listening() {
            *svc.hold_from.lock().unwrap() = None;
            live.release();
        } else if !update::installing() && live.press() {
            *svc.hold_from.lock().unwrap() = Some(now_ms());
        }
    } else {
        let from = svc.hold_from.lock().unwrap().take();
        let mode = svc.settings.lock().unwrap().talk_mode;
        if live.listening() && release_stops(mode, from, now_ms()) {
            live.release();
        }
    }
}

/// A frame of the screen for a question, with the settings it is cut by.
struct AskTaken(capture::AskFrame, settings::Settings);

impl live::Taken for AskTaken {
    fn look(&self) -> capture::Look {
        self.0.look()
    }

    fn thumb(&self) -> keyframes::Thumb {
        self.0.thumb()
    }

    fn cut(self: Box<Self>) -> Result<capture::Asked, String> {
        let AskTaken(frame, s) = *self;
        let target = s.ask_target;
        frame.finish(&s, target, capture::ASK_EDGE)
    }
}

/// The Ask loop as the app runs it: the recording path for its words, one frame of the screen for its picture, and
/// the panel for what it says.
fn live_for(app: &AppHandle, svc: &Arc<App>, held: Arc<assist::catalog::Held>) -> Arc<live::Live> {
    let (handle, shots, tidy, chooser) = (app.clone(), svc.clone(), svc.clone(), svc.clone());
    live::Live::new(
        live::Deps {
            voice: Arc::new(live::AppVoice(svc.clone())),
            grab: Box::new(move || {
                let s = shots.settings.lock().unwrap().clone();
                let frame: Box<dyn live::Taken> = Box::new(AskTaken(capture::grab_ask_frame(&s)?, s));
                Ok(frame)
            }),
            look: Box::new(capture::look),
            // The AI the settings name at this Ask: a choice made in Settings needs no restart.
            provider: Box::new(move || {
                let s = chooser.settings.lock().unwrap().clone();
                held.provider(&s)
            }),
            out: Box::new(move |o| match o {
                live::Out::State(s) => {
                    let _ = handle.emit("live://state", &s);
                }
                live::Out::Delta { turn, text } => {
                    let _ = handle.emit("live://delta", json!({ "turn": turn, "text": text }));
                }
                live::Out::Show => platform::show_panel(&handle),
                live::Out::Hide => hide_window(&handle, platform::PANEL),
            }),
            now: Box::new(now_ms),
            tidy: Box::new(move || tidy.settings.lock().unwrap().tidy),
        },
        &settings::data_dir(),
    )
}

#[tauri::command]
fn live_state(live: tauri::State<'_, Arc<live::Live>>) -> live::Snapshot {
    live.snapshot()
}

#[tauri::command]
fn live_drop_picture(live: tauri::State<'_, Arc<live::Live>>) {
    live.drop_picture();
}

/// Async: discarding a question waits for the speech worker, which must not hold the event loop.
#[tauri::command]
async fn live_stop(live: tauri::State<'_, Arc<live::Live>>) -> Result<(), String> {
    live.stop();
    Ok(())
}

#[tauri::command]
async fn live_close(live: tauri::State<'_, Arc<live::Live>>) -> Result<(), String> {
    live.close();
    Ok(())
}

/// Where the assistant keeps its own files: Codex's empty folder and its pid file.
fn assist_home() -> std::path::PathBuf {
    settings::data_dir().join("assistant")
}

/// Every AI Pointcast offers and where each stands. Spends nothing: Codex is asked its version and whether it is
/// signed in, the store which keys it holds, and the usual ports on this computer for their model lists (2 s).
#[tauri::command]
async fn assist_providers(svc: tauri::State<'_, Arc<App>>, held: tauri::State<'_, Arc<assist::catalog::Held>>) -> Result<assist::catalog::List, String> {
    let s = svc.settings.lock().unwrap().clone();
    let held = held.inner().clone();
    tauri::async_runtime::spawn_blocking(move || assist::catalog::providers(&s, &assist::catalog::look(&held))).await.map_err(|e| e.to_string())
}

/// The models `id` offers, asked of it now (Codex is started for its list). Never a question.
#[tauri::command]
async fn assist_models(svc: tauri::State<'_, Arc<App>>, held: tauri::State<'_, Arc<assist::catalog::Held>>, id: String) -> Result<Vec<assist::catalog::Model>, String> {
    let s = svc.settings.lock().unwrap().clone();
    let held = held.inner().clone();
    tauri::async_runtime::spawn_blocking(move || assist::catalog::models_of(&id, &s, &held, &assist_home())).await.map_err(|e| e.to_string())?
}

/// A change to what the assistant keeps in the settings, saved.
fn assist_change(svc: &App, change: impl FnOnce(&mut Settings) -> Result<(), String>) -> Result<(), String> {
    let mut s = svc.settings.lock().unwrap().clone();
    let said = change(&mut s);
    svc.save_settings(s)?;
    said
}

#[tauri::command]
fn assist_choose(svc: tauri::State<'_, Arc<App>>, id: String, base: Option<String>, model: Option<String>, consent: bool) -> Result<(), String> {
    // The typed address of the custom route; wired when the catalogue's choose_at lands.
    let _ = base;
    let mut s = svc.settings.lock().unwrap().clone();
    assist::catalog::choose(&mut s, &id, model, consent, now_ms())?;
    svc.save_settings(s)
}

#[tauri::command]
fn assist_forget(svc: tauri::State<'_, Arc<App>>, id: String) -> Result<(), String> {
    assist_change(&svc, |s| assist::catalog::forget(s, &assist::catalog::Keys::store(), &id))
}

/// The key goes to the system's store and nowhere else.
#[tauri::command]
fn assist_key_set(id: String, key: String) -> Result<(), String> {
    assist::catalog::Keys::store().set(&id, key)
}

#[tauri::command]
fn assist_key_delete(id: String) -> Result<(), String> {
    assist::catalog::Keys::store().delete(&id)
}

/// Sign in to ChatGPT through Codex's own flow: its page opens in the browser, and this ends when Codex says the
/// person is done, or after 5 minutes.
#[tauri::command]
async fn assist_sign_in(app: AppHandle, id: String) -> Result<(), String> {
    if id != assist::codex::ID {
        return Err(format!("{} has no sign-in here.", assist::catalog::route(&id).map_or("That AI", |r| r.name)));
    }
    tauri::async_runtime::spawn_blocking(move || {
        use tauri_plugin_opener::OpenerExt;
        let open = |page: &str| app.opener().open_url(page, None::<&str>).map_err(|e| format!("The sign-in page did not open: {e}"));
        assist::codex::Codex::new(assist_home().join("codex")).sign_in(&open, std::time::Duration::from_secs(300))
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
fn assist_offline_only(svc: tauri::State<'_, Arc<App>>, on: bool) -> Result<(), String> {
    assist_change(&svc, |s| {
        s.assist_offline_only = on;
        Ok(())
    })
}

/// Ask the question that failed again, as it was kept.
#[tauri::command]
async fn live_retry(live: tauri::State<'_, Arc<live::Live>>) -> Result<(), String> {
    live.retry()
}

fn begin(app: &AppHandle, svc: &Arc<App>) -> Result<(), String> {
    // The installer closes Pointcast: a recording started now would stop mid-sentence.
    if update::installing() {
        svc.refuse(session::UPDATING);
        // A HUD already built shows the new state from its first frame, not the last recording's.
        emit_state(app, svc);
        platform::show_indicator(app, &svc.ui());
        hide_hud_soon(app, svc);
        return Err(session::UPDATING.into());
    }
    platform::forget_front();
    svc.start()?;
    if svc.settings.lock().unwrap().sounds {
        platform::chime(true);
    }
    emit_state(app, svc);
    platform::show_indicator(app, &svc.ui());
    emit_state_now(app, svc);
    Ok(())
}

/// The capsule's last word: which one it is (`gen`), whether the pointer is over it or it has focus, and when that
/// last ended, in ms since it showed.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Hold {
    gen: u64,
    over: bool,
    left_ms: Option<u64>,
}

static HOLD: std::sync::Mutex<(Hold, Option<std::time::Instant>)> =
    std::sync::Mutex::new((Hold { gen: 0, over: false, left_ms: None }, None));

/// How long a notice shows on its own, and how long it stays after the pointer leaves it. A notice with something to
/// act on (kept, partial, cut) has two lines to read and stays 4 s after.
fn notice_times(notice: Option<&str>) -> (u64, u64) {
    match notice {
        Some("kept" | "partial" | "cut") => (4500, 4000),
        _ => (1600, 1600),
    }
}

/// When the capsule's last word may hide, in ms since it showed: never while the pointer is over it or it has focus,
/// and not before its own time nor before the linger after the pointer left.
fn hide_at(notice: Option<&str>, over: bool, left_ms: Option<u64>) -> Option<u64> {
    if over {
        return None;
    }
    let (show, linger) = notice_times(notice);
    Some(left_ms.map_or(show, |left| show.max(left + linger)))
}

/// The pointer came onto the notice (or it took focus), or went off it at `at_ms`.
fn hold_step(h: Hold, over: bool, at_ms: u64) -> Hold {
    match (h.over, over) {
        (false, true) => Hold { over: true, ..h },
        (true, false) => Hold { over: false, left_ms: Some(at_ms), ..h },
        _ => h,
    }
}

/// The HUD's last word stays a moment, then it goes unless a new recording took it or the pointer is on it.
fn hide_hud_soon(app: &AppHandle, svc: &Arc<App>) {
    let (app, svc) = (app.clone(), svc.clone());
    let notice = svc.ui().notice;
    let gen = {
        let mut h = HOLD.lock().unwrap();
        let gen = h.0.gen + 1;
        *h = (Hold { gen, ..Hold::default() }, Some(std::time::Instant::now()));
        gen
    };
    std::thread::spawn(move || loop {
        std::thread::sleep(std::time::Duration::from_millis(100));
        let (h, shown) = *HOLD.lock().unwrap();
        // A newer notice runs its own clock; a new recording took the capsule.
        if h.gen != gen || svc.ui().phase != Phase::Ready {
            return;
        }
        let elapsed = shown.map_or(0, |t| t.elapsed().as_millis() as u64);
        if hide_at(notice, h.over, h.left_ms).is_some_and(|at| elapsed >= at) {
            hide_window(&app, "indicator");
            return;
        }
    });
}

/// The capsule's notice has the pointer or focus (`over`), or lost it.
#[tauri::command]
fn hud_hold(over: bool) {
    let mut h = HOLD.lock().unwrap();
    let at = h.1.map_or(0, |t| t.elapsed().as_millis() as u64);
    h.0 = hold_step(h.0, over, at);
}

/// Open History, on entry `id` when given (the capsule's notice for the prompt just saved). A History window built just
/// now selects the latest entry, which is that one; one already open hears `history://select`.
#[tauri::command]
fn history_open(app: AppHandle, id: Option<String>) {
    show_window(&app, "result");
    if let Some(id) = id {
        let _ = app.emit("history://select", json!({ "id": id }));
    }
    if app.try_state::<Arc<App>>().is_some_and(|svc| svc.ui().phase == Phase::Ready) {
        hide_window(&app, "indicator");
    }
}

/// Every way the overlay goes away (save, cancel, stop, discard, Alt+F4): hide it, give focus
/// back to the app it was over, and forget its frame and any History edit.
fn close_overlay(app: &AppHandle) {
    EDIT.cancel();
    hide_window(app, "overlay");
    platform::refocus_front();
    FRAME.lock().unwrap().take();
}

fn finish(app: &AppHandle, svc: &Arc<App>) -> Result<Option<SessionExport>, String> {
    // A question being said ends in the Ask loop, whatever asked for the stop: never in a paste.
    if asking(svc) {
        if let Some(live) = app.try_state::<Arc<live::Live>>() {
            live.release();
        }
        return Ok(None);
    }
    close_overlay(app);
    if svc.settings.lock().unwrap().sounds {
        platform::chime(false);
    }
    // The HUD reads "Finishing…" while the last words are transcribed, then where the prompt went.
    emit_state_now(app, svc);
    let practice = svc.practice.load(Ordering::SeqCst);
    let exp = svc.stop(false);
    platform::forget_front();
    if practice || exp.is_err() {
        hide_window(app, "indicator");
    } else {
        emit_state_now(app, svc);
        hide_hud_soon(app, svc);
    }
    if practice {
        svc.practice.store(false, Ordering::SeqCst);
    }
    let exp = exp?;
    if let Some(ref e) = exp {
        if practice {
            let _ = app.emit("practice://result", e);
        } else {
            let _ = app.emit("session://result", e);
        }
    }
    emit_state(app, svc);
    Ok(exp)
}

/// Stop without holding the caller (the event loop) while the last words are transcribed.
fn finish_soon(app: &AppHandle, svc: &Arc<App>) {
    let (app, svc) = (app.clone(), svc.clone());
    std::thread::spawn(move || {
        let _ = finish(&app, &svc);
    });
}

/// Quit, even mid-recording: what was said is transcribed and kept for the next launch.
pub fn quit(app: &AppHandle) {
    if let Some(svc) = app.try_state::<Arc<App>>() {
        svc.flush_for_quit();
    }
    app.exit(0);
}

pub fn toggle(app: &AppHandle, svc: &Arc<App>) -> Result<(), String> {
    if recording(svc) {
        finish_soon(app, svc);
    } else {
        begin(app, svc)?;
    }
    Ok(())
}

/// The frozen frame for the overlay, with where the person pointed (frame pixels) when they want it marked and the
/// control there when they want outlines; the overlay places a ring and an outline as shapes they can move or remove.
fn frame_payload(shot: &capture::Shot, mark: settings::MarkPoint) -> Value {
    let pointer = shot.pointer.filter(|_| mark != settings::MarkPoint::Off);
    json!({
        "png_b64": base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &shot.png),
        "width": shot.width,
        "height": shot.height,
        "pointer": pointer.map(|p| json!({ "x": p.x, "y": p.y })),
        "element": shot.element,
    })
}

fn overlay_open(app: &AppHandle) -> bool {
    app.get_webview_window("overlay")
        .and_then(|w| w.is_visible().ok())
        .unwrap_or(false)
}

/// Show a frame in the overlay, remembering the app in front unless the overlay already is.
fn show_frame(app: &AppHandle, payload: &Value) {
    if !overlay_open(app) {
        platform::remember_front();
    }
    *FRAME.lock().unwrap() = Some(payload.clone());
    let _ = app.emit("overlay://frame", payload);
    platform::show_overlay(app);
}

fn open_region(app: &AppHandle, svc: &Arc<App>) -> Result<Value, String> {
    let shot = svc.begin_region()?;
    EDIT.cancel();
    let mark = svc.settings.lock().unwrap().mark_point;
    let payload = frame_payload(&shot, mark);
    show_frame(app, &payload);
    Ok(payload)
}

/// The frozen frame the overlay is showing; an overlay built just now asks for it on load,
/// since the event may have fired before its page was listening.
static FRAME: std::sync::Mutex<Option<Value>> = std::sync::Mutex::new(None);

#[tauri::command]
fn region_frame() -> Option<Value> {
    FRAME.lock().unwrap().clone()
}

/// A picture from History reopened in the overlay: (entry id, picture n). Committing writes back into it.
struct Editing(std::sync::Mutex<Option<(String, usize)>>);

static EDIT: Editing = Editing(std::sync::Mutex::new(None));

impl Editing {
    /// The overlay frame for picture `n` of entry `id`; the next commit goes back into that picture.
    fn begin(&self, store: &history::Store, id: &str, n: usize) -> Result<Value, String> {
        let cap = store.picture(id, n)?;
        let (width, height) = image_ops::png_size(&cap.png).unwrap_or((cap.width, cap.height));
        *self.0.lock().unwrap() = Some((id.to_string(), n));
        Ok(json!({
            "png_b64": base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &cap.png),
            "width": width,
            "height": height,
            "pointer": null,
            "element": null,
            "edit": true,
            // New pins are numbered on from the labels already drawn into the picture.
            "base": cap.labels.iter().map(|l| l.n).max().unwrap_or(0),
        }))
    }

    /// Write a commit back into the picture being edited; None when the overlay is not editing one. A failed write
    /// keeps the edit open, so a second try never falls through into a live recording.
    fn commit(&self, store: &history::Store, png: &[u8], labels: Vec<Label>) -> Option<Result<(String, prompt::Capture), String>> {
        let mut g = self.0.lock().unwrap();
        let (id, n) = g.clone()?;
        let r = store.replace(&id, n, png, labels);
        if r.is_ok() {
            *g = None;
        }
        Some(r.map(|c| (id, c)))
    }

    fn cancel(&self) {
        self.0.lock().unwrap().take();
    }
}

/// Reopen picture `n` of a past prompt in the overlay to mark it up again.
#[tauri::command]
async fn history_edit(app: AppHandle, svc: tauri::State<'_, Arc<App>>, id: String, n: usize) -> Result<(), String> {
    let payload = EDIT.begin(&svc.history, &id, n)?;
    show_frame(&app, &payload);
    Ok(())
}

/// What the session chord's release does: stop only a held (not tapped) push-to-talk.
fn release_stops(mode: TalkMode, held_from: Option<u64>, now: u64) -> bool {
    mode == TalkMode::Hold && held_from.is_some_and(|t| now.saturating_sub(t) >= HOLD_MS)
}

#[derive(Debug, PartialEq, Eq)]
enum Press {
    Start,
    Stop,
    /// The overlay is open: what is marked there goes in first, then the recording stops.
    CommitThenStop,
}

/// What pressing the session chord does.
fn session_press(rec: bool, overlay: bool) -> Press {
    match (rec, overlay) {
        (false, _) => Press::Start,
        (true, false) => Press::Stop,
        (true, true) => Press::CommitThenStop,
    }
}

/// Every shortcut, from the keyboard or the tray. A capture chord with no session starts one.
pub fn hotkey(app: &AppHandle, action: Action, pressed: bool) {
    let _ = app.emit("hotkey://fired", json!({ "action": action, "pressed": pressed }));
    let Some(svc) = app.try_state::<Arc<App>>() else {
        return;
    };
    let svc = svc.inner().clone();
    let rec = recording(&svc);
    match routed(action, rec, asking(&svc), svc.hotkey_test.load(Ordering::SeqCst)) {
        Some(true) => return ask_key(app, &svc, pressed),
        Some(false) => {}
        None => return,
    }
    // While a recording runs, its shortcuts still work: the test must not trap it.
    if svc.hotkey_test.load(Ordering::SeqCst) && !rec {
        return;
    }
    match (action, pressed) {
        (Action::Session, true) => match session_press(rec, rec && overlay_open(app)) {
            Press::CommitThenStop => {
                *svc.hold_from.lock().unwrap() = None;
                let _ = app.emit_to("overlay", "overlay://finish", ());
            }
            Press::Stop => {
                *svc.hold_from.lock().unwrap() = None;
                finish_soon(app, &svc);
            }
            Press::Start => {
                if begin(app, &svc).is_ok() {
                    *svc.hold_from.lock().unwrap() = Some(now_ms());
                }
            }
        },
        (Action::Session, false) => {
            let from = svc.hold_from.lock().unwrap().take();
            let mode = svc.settings.lock().unwrap().talk_mode;
            // A release while the overlay is open keeps recording, so the box being drawn is not lost.
            if rec && release_stops(mode, from, now_ms()) && !overlay_open(app) {
                finish_soon(app, &svc);
            }
        }
        (_, false) => {}
        (action, true) => {
            if !rec && begin(app, &svc).is_err() {
                return;
            }
            let _ = match action {
                Action::Full => svc.capture_full().map(|_| Value::Null),
                Action::Zoom => svc.capture_zoom().map(|_| Value::Null),
                _ => open_region(app, &svc),
            };
            emit_state(app, &svc);
        }
    }
}

fn installed_models<'a>(root: &std::path::Path, cat: &'a voice::models::Catalog) -> Vec<&'a str> {
    voice::engine::installed(root, cat)
}

fn settings_view(svc: &App) -> Value {
    let s = svc.settings.lock().unwrap().clone();
    let speech = (voice::models::speech_dir(), voice::models::catalog());
    let errors: Vec<Value> = svc
        .hotkey_errors
        .lock()
        .unwrap()
        .iter()
        .map(|(a, e)| json!({ "action": a, "error": e }))
        .collect();
    let chord = |c: chord::Chord, d: chord::Chord| {
        json!({ "label": c.label(), "keys": c.keys(), "default": d.label() })
    };
    let mut v = json!({
        "platform": std::env::consts::OS,
        "materials": platform::materials(),
        "version": env!("CARGO_PKG_VERSION"),
        "session_hotkey": s.session_chord().label(),
        "full_hotkey": s.full_chord().label(),
        "region_hotkey": s.region_chord().label(),
        "zoom_hotkey": s.zoom_chord().label(),
        "hotkeys": {
            "session": chord(s.session_chord(), chord::default_session()),
            "full": chord(s.full_chord(), chord::default_full()),
            "region": chord(s.region_chord(), chord::default_region()),
            "zoom": chord(s.zoom_chord(), chord::default_zoom()),
            "ask": chord(s.ask_chord(), chord::default_ask()),
        },
        "hotkey_errors": errors,
        "talk_mode": s.talk_mode,
        "model": s.model,
        "device": s.device,
        "output": s.output,
        "also_clipboard": s.also_clipboard,
        "export_dir": s.export_dir,
        "zoom_px": s.zoom_px(),
        "mark_point": s.mark_point,
        "zoom_target": s.zoom_target,
        "zoom_sharpen": s.zoom_sharpen,
        "outline_element": s.outline_element,
        "spoken_anchors": s.spoken_anchors,
        "key_frames": s.key_frames,
        "start_at_login": s.start_at_login,
        "check_updates": s.check_updates,
        "update": update::available(),
        "update_installing": update::installing(),
        "update_failure": update::last_failure(),
        "include_app": s.include_app,
        "hidden": s.hidden_list(),
        "mcp_bind": s.mcp_bind,
        "mcp_enabled": s.mcp_enabled,
        "mcp_error": MCP_ERROR.lock().unwrap().clone(),
        "onboarded": s.onboarded,
        "sounds": s.sounds,
        "reserved": chord::reserved().into_iter().map(|(k, why)| json!({"keys": k, "why": why})).collect::<Vec<_>>(),
        "speech": voice::engine::view(&speech.0, &speech.1),
        "speech_check": svc.speech_check().map(|r| json!({ "ok": r.is_ok(), "error": r.err() })),
        // The model a recording loads, which may not be the one picked (that one is not downloaded).
        "speech_in_use": voice::machine::route(&s.model, voice::machine::recommended(), &installed_models(&speech.0, &speech.1)),
        "devices": voice::audio::devices(),
        "permissions": permissions::probe(),
        "recommended": voice::machine::recommended(),
    });
    // Set apart: one more key in the literal above passes json!'s recursion limit.
    v["update_checked"] = update::checked();
    v["ask_hotkey"] = json!(s.ask_chord().label());
    v["keep_audio"] = json!(s.keep_audio);
    v["picture_paths"] = json!(s.picture_paths);
    v["voice_boost"] = json!(s.voice_boost);
    v["auto_gpu"] = json!(s.auto_gpu);
    v["free_memory_minutes"] = json!(s.free_memory_minutes);
    v["gpu_upgrade"] = json!(voice::upgrade::status());
    // What Settings › Speech says about this computer and its microphone.
    v["machine"] = json!(voice::machine::this());
    v["gpu_declined"] = json!(voice::upgrade::load_state(&settings::data_dir()).declined);
    v["default_device"] = json!(voice::audio::default_device());
    v["mic_recent"] = svc.history.recent_mics(20).into_iter().map(|r| json!({ "mic": r.mic, "open_ms": r.open_ms, "peak_db": r.peak_db })).collect();
    if let Some(why) = voice::upgrade::unusable_here(&settings::data_dir(), &speech.1) {
        for m in v["speech"]["models"].as_array_mut().into_iter().flatten().filter(|m| m["id"] == voice::upgrade::MODEL) {
            m["gpu"]["unusable"] = json!(why);
        }
    }
    mark_slow(&mut v["speech"], &svc.slow_models());
    mark_backend(&mut v["speech"], &svc.backends(), svc.device());
    mark_speed(&mut v["speech"], &svc.speeds());
    v
}

/// The model in memory gets `backend`: "gpu" when it runs on the graphics card, "cpu" on the processor, "unknown"
/// while the GPU helper has not said; and `device`, the graphics card it runs on.
fn mark_backend(speech: &mut Value, on: &std::collections::HashMap<String, &'static str>, device: Option<String>) {
    for m in speech["models"].as_array_mut().into_iter().flatten() {
        if let Some(b) = m["id"].as_str().and_then(|id| on.get(id)) {
            m["backend"] = json!(b);
            if let Some(d) = device.as_ref().filter(|_| *b == "gpu") {
                m["device"] = json!(d);
            }
        }
    }
}

/// Each model this run has measured gets `speed_x`: how many times faster than real time it transcribes here.
fn mark_speed(speech: &mut Value, speeds: &std::collections::HashMap<String, f64>) {
    for m in speech["models"].as_array_mut().into_iter().flatten() {
        if let Some(x) = m["id"].as_str().and_then(|id| speeds.get(id)) {
            m["speed_x"] = json!((x * 10.0).round() / 10.0);
        }
    }
}

/// Each model this run has judged gets `slow_engine`: whether it transcribes slower than real time on this computer.
fn mark_slow(speech: &mut Value, slow: &std::collections::HashMap<String, bool>) {
    for m in speech["models"].as_array_mut().into_iter().flatten() {
        if let Some(s) = m["id"].as_str().and_then(|id| slow.get(id)).copied() {
            m["slow_engine"] = json!(s);
        }
    }
}

/// Async, like permissions_get: off the event loop, since it reads the registry and lists devices.
#[tauri::command]
async fn settings_get(svc: tauri::State<'_, Arc<App>>) -> Result<Value, String> {
    Ok(settings_view(&svc))
}

/// Merge a partial change into the saved settings. Shortcuts are checked before anything is kept.
#[tauri::command]
fn settings_patch(
    app: AppHandle,
    svc: tauri::State<'_, Arc<App>>,
    patch: serde_json::Map<String, Value>,
) -> Result<Value, String> {
    let cur = svc.settings.lock().unwrap().clone();
    let next = merge(&cur, patch)?;
    svc.save_settings(next.clone())?;
    // A new free_memory_minutes moves the time the model goes.
    if cur.free_memory_minutes != next.free_memory_minutes {
        wake();
    }
    if keys_changed(&cur, &next) {
        rebind(&app, &svc);
    }
    if cur.include_app != next.include_app {
        platform::protect(&app, !next.include_app);
    }
    // Sharing with agents starts and stops with its switch, no reopening.
    if cur.mcp_enabled != next.mcp_enabled || cur.mcp_bind != next.mcp_bind {
        mcp_stop();
        if next.mcp_enabled {
            let _ = mcp_start(&svc, &next.mcp_bind);
        }
    }
    if cur.model != next.model && !recording(&svc) {
        let svc = svc.inner().clone();
        std::thread::spawn(move || {
            let _ = svc.ensure_engine();
        });
    }
    // Whisper Turbo picked, or the setting turned on: the faster GPU version is set up now, not in a while.
    if next.model == voice::upgrade::MODEL && cur.model != next.model {
        voice::upgrade::welcome(&settings::data_dir());
        kick_upgrade(&app, svc.inner());
    }
    if cur.auto_gpu != next.auto_gpu {
        if next.auto_gpu {
            kick_upgrade(&app, svc.inner());
        } else {
            voice::upgrade::pause();
        }
    }
    Ok(settings_view(&svc))
}

fn merge(cur: &Settings, patch: serde_json::Map<String, Value>) -> Result<Settings, String> {
    let mut v = serde_json::to_value(cur).map_err(|e| e.to_string())?;
    let obj = v.as_object_mut().ok_or("settings are not an object")?;
    // The AI and the person's agreement change only through the assist_ commands, which ask before they keep a yes.
    if patch.keys().any(|k| k.starts_with("assist_")) {
        return Err("The AI is chosen in Settings, not by a patch.".into());
    }
    for (k, val) in patch {
        if val.is_null() {
            obj.remove(&k);
        } else {
            obj.insert(k, val);
        }
    }
    let next: Settings = serde_json::from_value(v).map_err(|e| e.to_string())?;
    let chords = [
        ("Start / stop", &cur.session_hotkey, &next.session_hotkey),
        ("Capture screen", &cur.full_hotkey, &next.full_hotkey),
        ("Capture region", &cur.region_hotkey, &next.region_hotkey),
        ("Capture around pointer", &cur.zoom_hotkey, &next.zoom_hotkey),
        ("Ask about this", &cur.ask_hotkey, &next.ask_hotkey),
    ];
    let mut seen: Vec<(&str, chord::Chord)> = vec![];
    for (name, was, raw) in chords {
        let Some(raw) = raw else { continue };
        // A saved chord this patch leaves alone and this version refuses is not in use (the default is), so it never
        // blocks changing something else.
        let changed = was.as_deref() != Some(raw.as_str());
        let c = match chord::Chord::parse(raw) {
            Ok(c) => c,
            Err(e) if changed => return Err(e),
            Err(_) => continue,
        };
        if let Some(why) = chord::refusal(&c) {
            if !changed {
                continue;
            }
            return Err(format!("{}: {why}", c.label()));
        }
        if let Some((other, _)) = seen.iter().find(|(_, x)| *x == c) {
            return Err(format!("{} is already {other}", c.label()));
        }
        seen.push((name, c));
    }
    Ok(next)
}

fn keys_changed(a: &Settings, b: &Settings) -> bool {
    (a.session_chord(), a.full_chord(), a.region_chord(), a.zoom_chord(), a.ask_chord())
        != (b.session_chord(), b.full_chord(), b.region_chord(), b.zoom_chord(), b.ask_chord())
}

fn rebind(app: &AppHandle, svc: &Arc<App>) {
    let s = svc.settings.lock().unwrap().clone();
    *svc.hotkey_errors.lock().unwrap() = register_shortcuts(app, &s);
    tray::rekey(app, svc);
}

#[tauri::command]
fn session_state(svc: tauri::State<'_, Arc<App>>) -> session::UiState {
    svc.ui()
}

#[tauri::command]
async fn session_toggle(app: AppHandle, svc: tauri::State<'_, Arc<App>>) -> Result<session::UiState, String> {
    toggle(&app, &svc)?;
    Ok(svc.ui())
}

/// Async: discarding waits for the speech worker, which must not hold the event loop.
#[tauri::command]
async fn session_cancel(app: AppHandle, svc: tauri::State<'_, Arc<App>>) -> Result<(), String> {
    let _ = svc.stop(true)?;
    hide_window(&app, "indicator");
    close_overlay(&app);
    platform::forget_front();
    emit_state(&app, &svc);
    Ok(())
}

/// Cancel while the last words are transcribed: the recording is saved now with what is done, pasted or copied as
/// usual, and its audio kept for Transcribe again. Off the event loop: it returns once that is saved.
#[tauri::command]
async fn session_finish_now(svc: tauri::State<'_, Arc<App>>) -> Result<(), String> {
    let svc = svc.inner().clone();
    tauri::async_runtime::spawn_blocking(move || svc.finish_now()).await.map_err(|e| e.to_string())
}

#[tauri::command]
fn capture_full(app: AppHandle, svc: tauri::State<'_, Arc<App>>) -> Result<Value, String> {
    let cap = svc.capture_full()?;
    emit_state(&app, &svc);
    Ok(json!({"kind": cap.kind, "width": cap.width, "height": cap.height}))
}

#[tauri::command]
fn capture_zoom(app: AppHandle, svc: tauri::State<'_, Arc<App>>) -> Result<Value, String> {
    let cap = svc.capture_zoom()?;
    emit_state(&app, &svc);
    Ok(json!({"kind": cap.kind, "width": cap.width, "height": cap.height}))
}

#[tauri::command]
async fn region_begin(app: AppHandle, svc: tauri::State<'_, Arc<App>>) -> Result<Value, String> {
    open_region(&app, &svc)
}

#[derive(Deserialize)]
struct RegionCommit {
    png_b64: String,
    #[serde(default)]
    labels: Vec<Label>,
    /// The outlined control, in the cropped picture's pixels, when the person kept it.
    #[serde(default)]
    element: Option<element::ElementMark>,
    /// How many areas the person blacked out; the prompt carries it as a note on the picture.
    #[serde(default)]
    hidden: u32,
}

#[tauri::command]
fn region_commit(
    app: AppHandle,
    svc: tauri::State<'_, Arc<App>>,
    body: RegionCommit,
) -> Result<Value, String> {
    use base64::Engine;
    let png = base64::engine::general_purpose::STANDARD
        .decode(body.png_b64.trim())
        .map_err(|e| e.to_string())?;
    let mut labels = body.labels;
    prompt::note_hidden(&mut labels, body.hidden);
    // A picture from History goes back into its entry, which becomes the last prompt and is copied again.
    let (cap, edited) = match EDIT.commit(&svc.history, &png, labels.clone()) {
        Some(r) => {
            let (id, cap) = r?;
            (cap, Some(id))
        }
        None => (svc.commit_region(png, labels, body.element)?, None),
    };
    close_overlay(&app);
    let last = match edited {
        Some(id) => {
            let exp = svc.copy_from_history(&id)?;
            let _ = app.emit("history://changed", json!({ "id": id }));
            Some(exp)
        }
        None => None,
    };
    emit_state(&app, &svc);
    if let Some(exp) = last {
        copy_export(&exp)?;
    }
    Ok(json!({"kind": cap.kind, "width": cap.width, "height": cap.height, "labels": cap.labels}))
}

#[tauri::command]
fn region_cancel(app: AppHandle) {
    close_overlay(&app);
}

#[tauri::command]
fn last_prompt(svc: tauri::State<'_, Arc<App>>) -> Option<SessionExport> {
    svc.last_export()
}

#[tauri::command]
fn last_folder(svc: tauri::State<'_, Arc<App>>) -> Option<String> {
    svc.last_dir().map(|p| p.to_string_lossy().into_owned())
}

#[tauri::command]
fn open_folder(path: String) -> Result<(), String> {
    tauri_plugin_opener::open_path(path, None::<&str>).map_err(|e| e.to_string())
}

fn copy_last(svc: &App) -> Result<(), String> {
    let exp = svc.last_export().ok_or("no session yet")?;
    copy_export(&exp)
}

fn copy_export(exp: &SessionExport) -> Result<(), String> {
    let mut cb = arboard::Clipboard::new().map_err(|e| e.to_string())?;
    if exp.images.is_empty() {
        return cb.set_text(&exp.plain).map_err(|e| e.to_string());
    }
    cb.set_html(&exp.html, Some(&exp.plain)).map_err(|e| e.to_string())
}

#[tauri::command]
fn history_list(svc: tauri::State<'_, Arc<App>>) -> Value {
    json!({ "entries": svc.history.list(), "last": *svc.last_id.lock().unwrap() })
}

#[tauri::command]
fn history_get(svc: tauri::State<'_, Arc<App>>, id: String) -> Result<SessionExport, String> {
    let s = svc.settings.lock().unwrap().clone();
    svc.history.export_with(&id, s.picture_paths, s.tidy)
}

/// Copy a past prompt; it also becomes the one "Paste Last Prompt" uses.
#[tauri::command]
fn history_copy(svc: tauri::State<'_, Arc<App>>, id: String) -> Result<(), String> {
    copy_export(&svc.copy_from_history(&id)?)
}

#[tauri::command]
fn history_paste(app: AppHandle, svc: tauri::State<'_, Arc<App>>, id: String) -> Result<(), String> {
    copy_export(&svc.copy_from_history(&id)?)?;
    paste_from_history(&app)
}

/// Hide History so the paste lands in the app underneath it.
fn paste_from_history(app: &AppHandle) -> Result<(), String> {
    paste_out_of(
        permissions::can_paste(),
        || {
            hide_window(app, "result");
            platform::restore_front();
            std::thread::sleep(std::time::Duration::from_millis(220));
        },
        clipboard::paste_now,
        || show_window(app, "result"),
    )
}

/// Paste from a window that must get out of the way first. Without permission to paste it stays, and a paste that
/// fails once it is hidden brings it back, so the person reads that the prompt was only copied.
fn paste_out_of(can_paste: bool, hide: impl FnOnce(), paste: impl FnOnce() -> Result<(), String>, back: impl FnOnce()) -> Result<(), String> {
    if !can_paste {
        return Err(PASTE_YOURSELF.into());
    }
    hide();
    let r = paste_or_say(true, paste);
    if r.is_err() {
        back();
    }
    r
}

pub const PASTE_YOURSELF: &str = "Copied, paste it yourself.";

/// Paste what was just copied, or say it is only on the clipboard: without permission to paste (macOS
/// Accessibility), or when the app in front refuses it (an app run as administrator on Windows).
fn paste_or_say(can_paste: bool, paste: impl FnOnce() -> Result<(), String>) -> Result<(), String> {
    if !can_paste {
        return Err(PASTE_YOURSELF.into());
    }
    paste().map_err(|_| PASTE_YOURSELF.into())
}

fn paste() -> Result<(), String> {
    paste_or_say(permissions::can_paste(), clipboard::paste_now)
}

/// Take a key frame Pointcast picked out of a past prompt, which becomes the last prompt and is copied again.
#[tauri::command]
fn history_remove(app: AppHandle, svc: tauri::State<'_, Arc<App>>, id: String, n: usize) -> Result<SessionExport, String> {
    svc.history.remove(&id, n)?;
    let exp = svc.copy_from_history(&id)?;
    let _ = app.emit("history://changed", json!({ "id": id }));
    copy_export(&exp)?;
    Ok(exp)
}

/// Transcribe a past prompt again from its audio, on the model chosen now, off the event loop. `transcribe://progress`
/// `{id, done_ms, total_ms}` comes at least every 2 s while it runs, and `history://changed` `{id}` when it is done. A
/// refusal or failure comes back as a sentence, and the entry stays as it was.
#[tauri::command]
async fn history_retranscribe(app: AppHandle, svc: tauri::State<'_, Arc<App>>, id: String) -> Result<(), String> {
    let svc = svc.inner().clone();
    let job = tauri::async_runtime::spawn_blocking(move || {
        let tell = |(done, total): (u64, u64)| {
            let _ = app.emit("transcribe://progress", json!({ "id": id, "done_ms": done, "total_ms": total }));
        };
        let last = std::sync::Mutex::new(None);
        let over = std::sync::atomic::AtomicBool::new(false);
        let r = std::thread::scope(|s| {
            // One piece can take longer than a few seconds on a slow model: the last progress is said again meanwhile.
            let beat = s.spawn(|| loop {
                std::thread::park_timeout(std::time::Duration::from_secs(2));
                if over.load(Ordering::SeqCst) {
                    break;
                }
                if let Some(p) = *last.lock().unwrap() {
                    tell(p);
                }
            });
            let r = svc.retranscribe(&id, |done, total| {
                *last.lock().unwrap() = Some((done, total));
                tell((done, total));
            });
            over.store(true, Ordering::SeqCst);
            beat.thread().unpark();
            r
        });
        if r.is_ok() {
            let _ = app.emit("history://changed", json!({ "id": id }));
        }
        r
    });
    job.await.map_err(|e| e.to_string())?
}

/// Stop transcribing a past prompt again after the piece under way; the prompt stays as it was and
/// `history_retranscribe` returns `AGAIN_CANCELLED`. False when none is being transcribed again.
#[tauri::command]
fn history_retranscribe_cancel(svc: tauri::State<'_, Arc<App>>) -> bool {
    svc.again_cancel()
}

#[tauri::command]
fn history_delete(app: AppHandle, svc: tauri::State<'_, Arc<App>>, id: String) -> Result<(), String> {
    svc.history.delete(&id)?;
    forgot(&app, &svc, Some(&id));
    Ok(())
}

#[tauri::command]
fn history_clear(app: AppHandle, svc: tauri::State<'_, Arc<App>>) -> Result<(), String> {
    let r = svc.history.clear();
    forgot(&app, &svc, None);
    r
}

/// What the Usage pane shows: the running counts of what was dictated, read when it opens.
#[tauri::command]
fn stats_get(svc: tauri::State<'_, Arc<App>>) -> stats::Data {
    svc.stats.get()
}

/// Reset in the Usage pane: counting starts again from now. History is left as it is.
#[tauri::command]
fn stats_reset(app: AppHandle, svc: tauri::State<'_, Arc<App>>) -> stats::Data {
    let d = svc.stats.reset();
    tray::today(&app);
    d
}

/// A deleted prompt leaves Paste Last Prompt and what coding agents read too.
fn forgot(app: &AppHandle, svc: &App, gone: Option<&str>) {
    if svc.forget(gone) {
        emit_state(app, svc);
    }
}

pub fn deliver_last(svc: &App) -> Result<(), String> {
    copy_last(svc)?;
    paste()
}

#[tauri::command]
fn copy_again(svc: tauri::State<'_, Arc<App>>) -> Result<(), String> {
    copy_last(&svc)
}

/// Copy the MCP setup for coding agents, token included. The token never passes through a window.
#[tauri::command]
fn mcp_copy_setup(svc: tauri::State<'_, Arc<App>>, agent: String) -> Result<(), String> {
    let agent = agents::Agent::parse(&agent)?;
    let bind = svc.settings.lock().unwrap().mcp_bind.clone();
    let token = mcp::token(&mcp::token_path())?;
    clipboard::copy_secret(&agents::copy_text(agent, &mcp_url(&bind), &token))
}

fn mcp_url(bind: &str) -> String {
    format!("http://{bind}/mcp")
}

fn home() -> Result<std::path::PathBuf, String> {
    settings::home_dir().ok_or_else(|| "Pointcast could not find your home folder.".to_string())
}

/// The running agent server: its address and the flag that stops it.
static MCP_SERVER: std::sync::Mutex<Option<(std::net::SocketAddr, Arc<std::sync::atomic::AtomicBool>)>> = std::sync::Mutex::new(None);

/// Start sharing with coding agents unless it is already running. Errors name the problem, never the token.
fn mcp_start(svc: &App, bind: &str) -> Result<std::net::SocketAddr, String> {
    let mut server = MCP_SERVER.lock().unwrap();
    if let Some((addr, _)) = server.as_ref() {
        return Ok(*addr);
    }
    let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let started = mcp::token(&mcp::token_path()).and_then(|t| mcp::spawn_stoppable(bind, svc.latest.clone(), t, stop.clone()));
    *MCP_ERROR.lock().unwrap() = started.as_ref().err().cloned();
    let addr = started?;
    *server = Some((addr, stop));
    Ok(addr)
}

fn mcp_stop() {
    if let Some((addr, stop)) = MCP_SERVER.lock().unwrap().take() {
        mcp::stop_at(addr, &stop);
    }
    *MCP_ERROR.lock().unwrap() = None;
}

/// Turn sharing on (saved, so it stays on) and make sure the server runs.
fn mcp_on(svc: &App) -> Result<std::net::SocketAddr, String> {
    let mut s = svc.settings.lock().unwrap().clone();
    if !s.mcp_enabled {
        s.mcp_enabled = true;
        svc.save_settings(s.clone())?;
    }
    mcp_start(svc, &s.mcp_bind).map_err(|e| format!("Pointcast's agent server did not start: {e}."))
}

/// Each agent's state for Settings › Output, plus Claude Code's pictures folder and whether the server answers.
fn agents_view(svc: &App, home: &std::path::Path) -> Result<Value, String> {
    let s = svc.settings.lock().unwrap().clone();
    let token = mcp::token(&mcp::token_path())?;
    let url = mcp_url(&s.mcp_bind);
    let folder = prompt::handoff_root(&s).to_string_lossy().into_owned();
    let running = MCP_SERVER.lock().unwrap().as_ref().map(|(a, _)| *a);
    let server = match running {
        Some(addr) => mcp::probe(addr, &token).map(|_| ()),
        None if s.mcp_enabled => Err(MCP_ERROR.lock().unwrap().clone().unwrap_or_else(|| "it is not running".into())),
        None => Err("sharing is off".into()),
    };
    Ok(json!({
        "agents": agents::ALL.iter().map(|a| agents::view(*a, home, settings::home_is_custom(), &url, &token)).collect::<Vec<_>>(),
        "server_ok": server.is_ok(),
        "server_error": server.err(),
        "folder": folder,
        "folder_allowed": agents::folder_allowed(home, &folder),
    }))
}

#[tauri::command]
fn agents_status(svc: tauri::State<'_, Arc<App>>) -> Result<Value, String> {
    agents_view(&svc, &home()?)
}

/// Set up one agent in one click: sharing on, the entry written into the agent's user settings, then checked (the
/// entry is in the file and the server answers an MCP client with its five tools).
#[tauri::command]
fn agent_connect(svc: tauri::State<'_, Arc<App>>, agent: String, folder: bool) -> Result<Value, String> {
    let agent = agents::Agent::parse(&agent)?;
    let home = home()?;
    if !agent.installed(&home) {
        return Err(agent.not_found());
    }
    let addr = mcp_on(&svc)?;
    let token = mcp::token(&mcp::token_path())?;
    let url = mcp_url(&svc.settings.lock().unwrap().mcp_bind);
    agents::set_up(agent, &home, &url, &token)?;
    if agent == agents::Agent::Claude && folder {
        let dir = prompt::handoff_root(&svc.settings.lock().unwrap()).to_string_lossy().into_owned();
        agents::allow_folder(&home, &dir, true)?;
    }
    if agents::entry(agent, &home, &url, &token)? != agents::Entry::Current {
        return Err(format!("Pointcast wrote {} but could not find its entry there afterwards.", agent.shown()));
    }
    mcp::probe(addr, &token)?;
    agents_view(&svc, &home)
}

/// Take Pointcast's entry out of one agent's settings, and nothing else.
#[tauri::command]
fn agent_remove(svc: tauri::State<'_, Arc<App>>, agent: String) -> Result<Value, String> {
    let agent = agents::Agent::parse(&agent)?;
    let home = home()?;
    agents::remove(agent, &home)?;
    agents_view(&svc, &home)
}

/// Let Claude Code open the pictures folder without asking, or stop it.
#[tauri::command]
fn agent_folder(svc: tauri::State<'_, Arc<App>>, on: bool) -> Result<Value, String> {
    let home = home()?;
    let dir = prompt::handoff_root(&svc.settings.lock().unwrap()).to_string_lossy().into_owned();
    agents::allow_folder(&home, &dir, on)?;
    agents_view(&svc, &home)
}

#[tauri::command]
fn paste_again(app: AppHandle, svc: tauri::State<'_, Arc<App>>) -> Result<(), String> {
    copy_last(&svc)?;
    paste_from_history(&app)
}

/// Why sharing with coding agents did not start (the address is taken, say), for Settings.
static MCP_ERROR: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

/// The latest setup progress, so a window opened mid-download picks it up.
static SPEECH: std::sync::Mutex<Option<Value>> = std::sync::Mutex::new(None);
static SPEECH_BUSY: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn speech_emit(app: &AppHandle, v: Value) {
    *SPEECH.lock().unwrap() = Some(v.clone());
    let _ = app.emit("speech://progress", v);
}

/// Download what is missing for `id` (engine, then model) as one job with one progress bar, then
/// load it and decode silence so "ready" means it runs here. A second call while one runs is a no-op.
#[tauri::command]
fn speech_setup(app: AppHandle, svc: tauri::State<'_, Arc<App>>, id: String) -> Result<(), String> {
    if SPEECH_BUSY.swap(true, Ordering::SeqCst) {
        return Ok(());
    }
    voice::models::CANCEL.store(false, Ordering::SeqCst);
    // The background upgrade fetching the same files stops first; this setup goes on from what it has.
    voice::upgrade::pause();
    let svc = svc.inner().clone();
    std::thread::spawn(move || {
        let t = std::time::Instant::now();
        while voice::upgrade::running() && t.elapsed() < std::time::Duration::from_secs(60) {
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        let r = speech_job(&app, &svc, &id);
        if r.as_ref().err().map(String::as_str) == Some(voice::models::CANCELLED) {
            let root = voice::models::speech_dir();
            let cat = voice::models::catalog();
            let parts = [cat.runtime_for_host(), cat.helper_for_host()].into_iter().flatten().map(|r| r.id.as_str());
            for part in parts.chain(cat.gpu_variant(&id).map(|g| g.id.as_str())).chain([id.as_str()]) {
                voice::models::drop_partial(&root, part);
            }
        }
        *SPEECH.lock().unwrap() = None;
        SPEECH_BUSY.store(false, Ordering::SeqCst);
        wake();
        // What is installed decides which model recordings use: an upgrade found "not Whisper Turbo" may now start.
        if r.is_ok() {
            kick_upgrade(&app, &svc);
        }
        let _ = app.emit("speech://done", json!({ "id": id, "ok": r.is_ok(), "error": r.err() }));
    });
    Ok(())
}

fn speech_job(app: &AppHandle, svc: &Arc<App>, id: &str) -> Result<(), String> {
    use voice::models::{self as m, Progress};
    let root = m::speech_dir();
    let cat = m::catalog();
    let gpu = voice::engine::helper_available(&cat) && voice::upgrade::unusable_here(&settings::data_dir(), &cat).is_none();
    let (rt, model) = setup_parts(&cat, id, gpu, voice::engine::dev_helper().is_some())?;
    let rt = rt.filter(|r| !m::runtime_installed(&root, r));
    let need_model = !m::model_installed(&root, model);
    let total = rt.map_or(0, |r| r.bytes) + if need_model { model.bytes } else { 0 };
    if let Some(free) = m::free_bytes(&root) {
        let need = m::space_needed(total);
        if total > 0 && free < need {
            return Err(format!(
                "Not enough free disk space: setup needs about {} MB and {} MB are free.",
                need / 1_000_000,
                free / 1_000_000
            ));
        }
    }
    let step = |offset: u64, part: &'static str| {
        let app = app.clone();
        move |p: Progress| {
            let v = match p {
                Progress::Downloading { received, .. } => json!({
                    "phase": "download", "part": part, "received": offset + received, "total": total,
                }),
                Progress::Unpacking | Progress::Verifying => json!({ "phase": "unpack", "part": part }),
                _ => return,
            };
            speech_emit(&app, v);
        }
    };
    speech_emit(app, json!({ "phase": "download", "part": "runtime", "received": 0, "total": total }));
    if let Some(rt) = rt {
        m::install_runtime(&root, rt, &step(0, "runtime"))?;
    }
    if need_model {
        let done = rt.map_or(0, |r| r.bytes);
        m::install_model(&root, model, &step(done, "model"))?;
    }
    speech_emit(app, json!({ "phase": "check" }));
    // Set up because the person asked: its own check below starts the helper.
    voice::upgrade::verified(&root);
    let mut s = svc.settings.lock().unwrap().clone();
    if s.model != id {
        s.model = id.to_string();
        svc.save_settings(s)?;
    }
    svc.engine_check()
        .map_err(|e| format!("The speech engine didn't start on this computer: {e}"))
}

/// What a setup of `id` downloads: a model with a GPU version gets that and its helper where `gpu` (the helper is
/// published and was not found unusable here; a development helper, `dev`, is not downloaded); sherpa's runtime and
/// files otherwise.
fn setup_parts<'a>(
    cat: &'a voice::models::Catalog,
    id: &str,
    gpu: bool,
    dev: bool,
) -> Result<(Option<&'a voice::models::RuntimeEntry>, &'a voice::models::ModelEntry), String> {
    let model = cat.model(id).ok_or("That model isn't in the catalog.")?;
    Ok(match cat.gpu_variant(id).filter(|_| gpu) {
        Some(g) => (cat.helper_for_host().filter(|_| !dev), g),
        None => (Some(cat.runtime_for_host().ok_or("Speech isn't available for this computer yet.")?), model),
    })
}

#[tauri::command]
fn speech_cancel() {
    voice::models::CANCEL.store(true, Ordering::SeqCst);
}

/// Cancel on the Whisper Turbo card while the faster GPU version downloads by itself.
/// Settings › Speech › Try again: forget that the graphics card could not run the engine here, or that its download was
/// stopped, so the next setup tries it.
#[tauri::command]
fn gpu_retry() {
    let data = settings::data_dir();
    let mut s = voice::upgrade::load_state(&data);
    (s.gpu_unusable, s.declined) = (None, false);
    voice::upgrade::save_state(&data, &s);
}

#[tauri::command]
fn gpu_upgrade_cancel() {
    voice::upgrade::cancel(&settings::data_dir());
}

/// The background GPU upgrade as the Whisper Turbo card and the tray's tooltip show it.
fn upgrade_report(app: &AppHandle, s: &voice::upgrade::Status) {
    static TIP: std::sync::Mutex<String> = std::sync::Mutex::new(String::new());
    let _ = app.emit("gpu://upgrade", s);
    let tip = match s.state {
        "downloading" => format!("Pointcast: getting the faster GPU version, {}%", s.percent),
        "checking" => "Pointcast: checking your graphics card".to_string(),
        _ => "Pointcast".to_string(),
    };
    let mut shown = TIP.lock().unwrap();
    if *shown != tip {
        if let Some(t) = app.tray_by_id("main") {
            let _ = t.set_tooltip(Some(tip.as_str()));
        }
        *shown = tip;
    }
}

/// Run the background GPU upgrade if it should run now, or show the note when something the person can fix stops it.
/// Not before the setup assistant is done: it sets up a model of its own. True when the answer is final until
/// `kick_upgrade` asks again (leaving the setup assistant does).
fn try_upgrade(app: &AppHandle, svc: &Arc<App>) -> bool {
    use voice::upgrade as up;
    let s = svc.settings.lock().unwrap().clone();
    if !s.onboarded {
        return true;
    }
    let (root, data, cat) = (voice::models::speech_dir(), settings::data_dir(), voice::models::catalog());
    let busy = || svc.busy() || SPEECH_BUSY.load(Ordering::SeqCst);
    let report = |st: &up::Status| upgrade_report(app, st);
    match up::decide(&up::facts(&root, &data, &cat, &s.model, s.auto_gpu, busy())) {
        up::Decision::Skip(why) => {
            up::noted(&why, &report);
            up::settled(&why)
        }
        up::Decision::Start => {
            let check = || voice::engine::probe(&root, &cat, up::MODEL);
            let out = up::run(&root, &data, &cat, &up::Hooks { busy: &busy, check: &check, report: &report });
            eprintln!("Pointcast GPU upgrade: {out:?}");
            // In memory now, so the next recording starts on the graphics card without waiting for it.
            if matches!(out, up::Outcome::Gpu(_)) && !busy() {
                let _ = svc.ensure_engine();
            }
            up::finished(&out)
        }
    }
}

static UPGRADE_LOOP: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static UPGRADE_KICKS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Ask now, and keep asking in the background while the answer can change by itself.
fn kick_upgrade(app: &AppHandle, svc: &Arc<App>) {
    UPGRADE_KICKS.fetch_add(1, Ordering::SeqCst);
    let (app, svc) = (app.clone(), svc.clone());
    std::thread::spawn(move || {
        if !try_upgrade(&app, &svc) {
            upgrade_loop(&app, &svc);
        }
    });
}

/// The faster GPU engine sets itself up once nothing was recorded for 30 s, and goes on after a recording. The loop
/// ends once the answer is final; `kick_upgrade` starts it again. One at a time.
fn upgrade_loop(app: &AppHandle, svc: &Arc<App>) {
    if UPGRADE_LOOP.swap(true, Ordering::SeqCst) {
        return;
    }
    let (app, svc) = (app.clone(), svc.clone());
    std::thread::spawn(move || {
        let mut quiet_since = std::time::Instant::now();
        loop {
            std::thread::sleep(std::time::Duration::from_secs(5));
            if svc.busy() || SPEECH_BUSY.load(Ordering::SeqCst) {
                quiet_since = std::time::Instant::now();
                continue;
            }
            if quiet_since.elapsed() < std::time::Duration::from_secs(30) {
                continue;
            }
            let kicks = UPGRADE_KICKS.load(Ordering::SeqCst);
            let settled = try_upgrade(&app, &svc);
            quiet_since = std::time::Instant::now();
            if settled {
                UPGRADE_LOOP.store(false, Ordering::SeqCst);
                // A kick meanwhile may have found the loop still running: go on for it, unless another loop took over.
                if UPGRADE_KICKS.load(Ordering::SeqCst) == kicks || UPGRADE_LOOP.swap(true, Ordering::SeqCst) {
                    break;
                }
            }
        }
    });
}

#[tauri::command]
fn speech_status() -> Option<Value> {
    SPEECH.lock().unwrap().clone()
}

/// The model the next recording would load, by the same rule the engine uses.
fn in_use(chosen: &str, recommended: &str, installed: &[&str], id: &str) -> bool {
    voice::machine::route(chosen, recommended, installed) == Some(id)
}

/// Delete a downloaded model to free its disk space. The model in use stays.
#[tauri::command]
fn speech_remove(svc: tauri::State<'_, Arc<App>>, id: String) -> Result<Value, String> {
    let cat = voice::models::catalog();
    let m = cat.model(&id).ok_or("That model isn't in the catalog.")?;
    let root = voice::models::speech_dir();
    let installed = installed_models(&root, &cat);
    let active = in_use(&svc.settings.lock().unwrap().model, voice::machine::recommended(), &installed, &m.id);
    if active || SPEECH_BUSY.load(Ordering::SeqCst) {
        return Err("That model is in use. Pick another one first.".into());
    }
    voice::models::remove_model(&root, m)?;
    // Its GPU version goes with it.
    if let Some(g) = cat.gpu_variant(&m.id) {
        voice::models::remove_model(&root, g)?;
    }
    Ok(settings_view(&svc))
}

/// Async: the setup assistant asks every second and a half, and each probe reads the registry and lists devices.
#[tauri::command]
async fn permissions_get(svc: tauri::State<'_, Arc<App>>) -> Result<permissions::PermissionStatus, String> {
    Ok(noted(&svc, permissions::probe()))
}

#[tauri::command]
fn permission_request(svc: tauri::State<'_, Arc<App>>, kind: String) -> permissions::PermissionStatus {
    permissions::request(&kind);
    noted(&svc, permissions::probe())
}

/// Remember what was ever granted, so launch can tell a permission an update took from one never given.
fn noted(svc: &App, p: permissions::PermissionStatus) -> permissions::PermissionStatus {
    let mut s = svc.settings.lock().unwrap().clone();
    if s.granted.note(p.screen == permissions::Access::Granted, p.accessibility == permissions::Access::Granted) {
        let _ = svc.save_settings(s);
    }
    p
}

#[tauri::command]
fn permission_open(kind: String) -> Result<(), String> {
    let url = permissions::settings_url(&kind).ok_or("no settings page for this")?;
    tauri_plugin_opener::open_url(url, None::<&str>).map_err(|e| e.to_string())
}

/// macOS applies a new Screen Recording grant only to a fresh process.
#[tauri::command]
fn relaunch(app: AppHandle) {
    app.restart();
}

#[tauri::command]
fn detect_tools() -> Vec<tools::DetectedTool> {
    tools::detect()
}

#[tauri::command]
fn open_tool(id: String) -> Result<(), String> {
    tools::open(&id)
}

#[tauri::command]
fn send_to_tool(svc: tauri::State<'_, Arc<App>>, id: String) -> Result<(), String> {
    copy_last(&svc)?;
    tools::open(&id)?;
    std::thread::sleep(std::time::Duration::from_millis(700));
    paste()
}

#[tauri::command]
fn overlay_esc(layer: overlay::OverlayLayer) -> overlay::OverlayLayer {
    overlay::esc(layer)
}

#[tauri::command]
fn overlay_hint(layer: overlay::OverlayLayer, tool: overlay::Tool) -> String {
    overlay::hint(layer, tool).into()
}

#[tauri::command]
fn inject_speech(svc: tauri::State<'_, Arc<App>>, text: String) {
    svc.inject_speech(&text);
}

#[tauri::command]
fn parse_chord(text: String) -> Result<Value, String> {
    let c = chord::Chord::parse(&text)?;
    if let Some(why) = chord::refusal(&c) {
        return Err(why);
    }
    Ok(json!({ "label": c.label(), "accelerator": c.accelerator(), "keys": c.keys() }))
}

/// While a shortcut field is listening, the OS must not swallow the chord being typed.
#[tauri::command]
fn hotkeys_suspend(app: AppHandle, svc: tauri::State<'_, Arc<App>>, on: bool) -> Value {
    use tauri_plugin_global_shortcut::GlobalShortcutExt;
    PAUSED.store(on, Ordering::SeqCst);
    if on {
        let _ = app.global_shortcut().unregister_all();
    } else {
        rebind(&app, &svc);
    }
    settings_view(&svc)
}

/// Shortcuts are unregistered while a field listens for new keys. A window closed mid-listen
/// never runs its own cleanup, so closing it brings them back here.
static PAUSED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Onboarding's "press it now": shortcuts report that they fired and do nothing else.
#[tauri::command]
fn hotkey_test(svc: tauri::State<'_, Arc<App>>, on: bool) {
    svc.hotkey_test.store(on, Ordering::SeqCst);
}

/// Onboarding's practice run: the next session started is a real one whose prompt stays in the onboarding window.
/// Off mid-run discards it (async: that waits for the speech worker).
#[tauri::command]
async fn practice(app: AppHandle, svc: tauri::State<'_, Arc<App>>, on: bool) -> Result<(), String> {
    if on {
        svc.practice_next.store(true, Ordering::SeqCst);
    } else {
        end_practice(&app, &svc);
    }
    Ok(())
}

fn end_practice(app: &AppHandle, svc: &App) {
    if svc.practice_off() {
        let _ = svc.stop(true);
        svc.practice.store(false, Ordering::SeqCst);
        hide_window(app, "indicator");
        close_overlay(app);
        platform::forget_front();
        emit_state(app, svc);
    }
}

/// Live input level outside a session, for the microphone check: `mic://level` (the meter) and `mic://peak` (the
/// loudest sample of the last 50 ms, dBFS). With `lease_ms` the microphone closes by itself unless asked again within
/// that time (Settings asks while its meter shows).
#[tauri::command]
fn mic_monitor(app: AppHandle, svc: tauri::State<'_, Arc<App>>, on: bool, lease_ms: Option<u64>) -> Result<(), String> {
    static EMITTING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if !on {
        svc.monitor_off();
        return Ok(());
    }
    svc.monitor_on(lease_ms.map(std::time::Duration::from_millis))?;
    if !EMITTING.swap(true, Ordering::SeqCst) {
        let svc = svc.inner().clone();
        std::thread::spawn(move || loop {
            while let Some((level, peak)) = svc.monitor_read() {
                let _ = app.emit("mic://level", level);
                let _ = app.emit("mic://peak", peak);
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            EMITTING.store(false, Ordering::SeqCst);
            // Turned on again as this one ended: carry on rather than leave it without a reader.
            if svc.monitor.lock().unwrap().is_none() || EMITTING.swap(true, Ordering::SeqCst) {
                break;
            }
        });
    }
    Ok(())
}

/// Listen briefly to each microphone other than the one chosen and say which pick up sound, loudest first: Settings
/// offers one when the chosen one is silent.
#[tauri::command]
async fn mic_scan(svc: tauri::State<'_, Arc<App>>) -> Result<Vec<Value>, String> {
    let svc = svc.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let chosen = svc.settings.lock().unwrap().device.clone().or_else(voice::audio::default_device);
        let others: Vec<String> = voice::audio::devices().into_iter().filter(|d| Some(d) != chosen.as_ref()).collect();
        let mut v = svc.scan_inputs(&others, 600);
        v.sort_by_key(|(_, db)| -db);
        v.into_iter().map(|(name, peak_db)| json!({ "name": name, "peak_db": peak_db })).collect()
    })
    .await
    .map_err(|e| e.to_string())
}

/// Whatever step the assistant was on, shortcuts and sessions go back to normal. On a thread of its own: discarding a
/// practice run waits for the speech worker, which must not hold the event loop.
fn leave_onboarding(app: &AppHandle, svc: &Arc<App>) {
    let (app, svc) = (app.clone(), svc.clone());
    std::thread::spawn(move || {
        end_practice(&app, &svc);
        svc.hotkey_test.store(false, Ordering::SeqCst);
        svc.monitor_off();
    });
}

#[tauri::command]
fn onboarding_finish(app: AppHandle, svc: tauri::State<'_, Arc<App>>) -> Result<(), String> {
    leave_onboarding(&app, &svc);
    let mut s = svc.settings.lock().unwrap().clone();
    // Open at login is on unless the person switched it off: register it now, so Pointcast is
    // back after a restart without anyone running the install command again.
    if s.start_at_login {
        if let Err(e) = login::set(&app, true) {
            eprintln!("Pointcast could not set itself to open at login: {e}");
        }
    }
    s.onboarded = true;
    svc.save_settings(s)?;
    kick_upgrade(&app, svc.inner());
    // Closed rather than hidden, so opening it again starts over at the first step that needs attention.
    if let Some(w) = app.get_webview_window("onboarding") {
        let _ = w.close();
    }
    Ok(())
}

#[tauri::command]
fn login_get(app: AppHandle) -> Result<login::Login, String> {
    login::get(&app)
}

#[tauri::command]
fn login_set(app: AppHandle, on: bool) -> Result<login::Login, String> {
    login::set(&app, on)
}

/// Ask GitHub now. Returns the newer version, or null when up to date.
#[tauri::command]
async fn update_check(app: AppHandle) -> Result<Option<String>, String> {
    let found = tauri::async_runtime::spawn_blocking(update::check)
        .await
        .map_err(|e| e.to_string())??;
    tray::offer_update(&app, found.as_deref());
    Ok(found)
}

/// Run the installer: it closes Pointcast, installs the new version and opens it again.
#[tauri::command]
fn update_install(app: AppHandle, svc: tauri::State<'_, Arc<App>>) -> Result<(), String> {
    if !idle(&svc) {
        return Err("Finish the recording first, then update.".into());
    }
    let svc = svc.inner().clone();
    update::install(
        move || idle(&svc),
        move |why| {
            let _ = app.emit("update://failed", why);
        },
    )
}

#[tauri::command]
fn platform_info() -> Value {
    json!({ "os": std::env::consts::OS, "materials": platform::materials() })
}

#[tauri::command]
async fn window_open(app: AppHandle, label: String) {
    show_window(&app, &label);
}

/// Register the four chords. Each one the OS refuses (another app owns it) is reported, not fatal.
fn register_shortcuts(app: &AppHandle, s: &Settings) -> Vec<(String, String)> {
    use std::str::FromStr;
    use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};
    let gs = app.global_shortcut();
    let _ = gs.unregister_all();
    let mut errors = refused(s);
    let chords = [
        (Action::Session, "session", s.session_chord()),
        (Action::Full, "full", s.full_chord()),
        (Action::Region, "region", s.region_chord()),
        (Action::Zoom, "zoom", s.zoom_chord()),
        (Action::Ask, "ask", s.ask_chord()),
    ];
    for (action, name, chord) in chords {
        let res = Shortcut::from_str(&chord.accelerator())
            .map_err(|e| e.to_string())
            .and_then(|sc| {
                gs.on_shortcut(sc, move |app, _s, ev| {
                    hotkey(app, action, ev.state == ShortcutState::Pressed);
                })
                .map_err(|_| format!("{} is taken by another app", chord.label()))
            });
        if let Err(e) = res {
            errors.retain(|(n, _)| n != name);
            errors.push((name.to_string(), e));
        }
    }
    errors
}

/// Saved chords this version refuses (saved before the rule existed): they fall back to the default, and the
/// person is told why rather than that another app took the key.
fn refused(s: &Settings) -> Vec<(String, String)> {
    let saved = [
        ("session", &s.session_hotkey, s.session_chord()),
        ("full", &s.full_hotkey, s.full_chord()),
        ("region", &s.region_hotkey, s.region_chord()),
        ("zoom", &s.zoom_hotkey, s.zoom_chord()),
        ("ask", &s.ask_hotkey, s.ask_chord()),
    ];
    saved
        .into_iter()
        .filter_map(|(name, raw, used)| {
            let c = chord::Chord::parse(raw.as_deref()?).ok()?;
            let why = chord::refusal(&c)?;
            Some((name.to_string(), format!("{}: {why}. Using {} instead.", c.label(), used.label())))
        })
        .collect()
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let settings_path = settings::settings_path();
    let settings = Settings::load(&settings_path);
    let svc = App::new(settings.clone(), settings_path);
    if settings.mcp_enabled {
        // Errors name the problem, never the token.
        match mcp_start(&svc, &settings.mcp_bind) {
            Ok(addr) => eprintln!("Pointcast MCP on http://{addr}"),
            Err(e) => eprintln!("Pointcast MCP did not start: {e}"),
        }
    }

    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
            // Started again at login (macOS session restore had already opened it): nothing to show.
            if login::at_login(&args) {
                return;
            }
            let onboarded = app
                .try_state::<Arc<App>>()
                .is_some_and(|svc| svc.settings.lock().unwrap().onboarded);
            show_window(app, if onboarded { "settings" } else { "onboarding" });
        }))
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .manage(svc.clone())
        .invoke_handler(tauri::generate_handler![
            settings_get,
            settings_patch,
            session_state,
            session_toggle,
            session_cancel,
            session_finish_now,
            hud_hold,
            history_open,
            capture_full,
            capture_zoom,
            region_begin,
            region_commit,
            region_cancel,
            region_frame,
            last_prompt,
            last_folder,
            open_folder,
            copy_again,
            mcp_copy_setup,
            agents_status,
            agent_connect,
            agent_remove,
            agent_folder,
            paste_again,
            speech_setup,
            history_list,
            history_get,
            history_edit,
            history_remove,
            history_retranscribe,
            history_retranscribe_cancel,
            history_copy,
            history_paste,
            history_delete,
            history_clear,
            stats_get,
            stats_reset,
            speech_cancel,
            gpu_upgrade_cancel,
            gpu_retry,
            speech_status,
            speech_remove,
            permissions_get,
            permission_request,
            permission_open,
            relaunch,
            detect_tools,
            open_tool,
            send_to_tool,
            overlay_esc,
            overlay_hint,
            inject_speech,
            parse_chord,
            hotkeys_suspend,
            hotkey_test,
            practice,
            mic_monitor,
            mic_scan,
            onboarding_finish,
            login_get,
            login_set,
            update_check,
            update_install,
            window_open,
            live_state,
            live_drop_picture,
            live_stop,
            live_close,
            live_retry,
            assist_providers,
            assist_models,
            assist_choose,
            assist_forget,
            assist_key_set,
            assist_key_delete,
            assist_sign_in,
            assist_offline_only,
            platform_info,
        ])
        .setup(move |app| {
            #[cfg(target_os = "macos")]
            let _ = app.set_activation_policy(tauri::ActivationPolicy::Accessory);
            let handle = app.handle().clone();
            clipboard::bind(&handle);
            tray::setup(&handle, &svc)?;
            let held = Arc::new(assist::catalog::Held::new(assist_home()));
            app.manage(held.clone());
            app.manage(live_for(&handle, &svc, held));
            *svc.hotkey_errors.lock().unwrap() = register_shortcuts(&handle, &settings);
            platform::protect(&handle, !settings.include_app);
            if let Some(m) = svc.history.recover() {
                svc.stats.count(&m);
                let _ = svc.set_last(&m.id);
                show_window(&handle, "result");
            } else if let Some(m) = svc.history.list().first() {
                let _ = svc.set_last(&m.id);
            }
            if !settings.onboarded {
                show_window(&handle, "onboarding");
            }
            #[cfg(target_os = "macos")]
            if settings.onboarded {
                let p = noted(&svc, permissions::probe());
                let paste = settings.output == settings::OutputMode::AutoPaste;
                let on = |a: permissions::Access| a == permissions::Access::Granted;
                if settings.granted.lost(on(p.screen), on(p.accessibility), paste) {
                    show_window(&handle, "onboarding");
                }
            }
            // An entry an earlier version made is written again in this one's form (with --login).
            if settings.onboarded && settings.start_at_login {
                let handle = handle.clone();
                std::thread::spawn(move || {
                    if login::get(&handle).is_ok_and(|l| l.enabled && !l.machine) {
                        let _ = login::set(&handle, true);
                    }
                });
            }
            // A minute after launch and then once a day, look for a newer release (if allowed).
            {
                let (handle, svc) = (handle.clone(), svc.clone());
                std::thread::spawn(move || {
                    std::thread::sleep(std::time::Duration::from_secs(60));
                    loop {
                        if svc.settings.lock().unwrap().check_updates {
                            if let Ok(found) = update::check() {
                                tray::offer_update(&handle, found.as_deref());
                                let _ = handle.emit("update://checked", &found);
                            }
                        }
                        std::thread::sleep(std::time::Duration::from_secs(24 * 60 * 60));
                    }
                });
            }
            // The faster GPU engine sets itself up once nothing was recorded for 30 s, until its answer is final.
            upgrade_loop(&handle, &svc);
            // Pulse live state to the HUD and the menu bar clock while a session runs. Otherwise sleep until something
            // wakes the loop or the next deadline: the hidden floating windows go REAP_AFTER after they hid or the
            // session ended; the speech model, the chosen time after its last use.
            let svc2 = svc.clone();
            std::thread::spawn(move || {
                // Windows made hidden at launch go too.
                *HIDDEN.lock().unwrap() = Some(std::time::Instant::now());
                let mut was_busy = false;
                let mut told_tray = std::time::Instant::now();
                loop {
                    let gen = WAKE.gen();
                    let busy = matches!(svc2.ui().phase, Phase::Recording | Phase::Loading | Phase::Transcribing);
                    if busy {
                        // While an Ask listens the panel's line hears the microphone 30 times a second; the tray and the
                        // menu bar clock still hear of it 10 times a second.
                        if told_tray.elapsed() >= TICK {
                            emit_state(&handle, &svc2);
                            told_tray = std::time::Instant::now();
                        } else {
                            emit_state_now(&handle, &svc2);
                        }
                        was_busy = true;
                        // The app the prompt goes to at stop is the last one in front that is not Pointcast: a
                        // click on the HUD's Stop can bring Pointcast forward on macOS.
                        #[cfg(target_os = "macos")]
                        if recording(&svc2) && !handle.webview_windows().values().any(|w| w.is_focused().unwrap_or(false)) {
                            platform::remember_front();
                        }
                    } else if was_busy {
                        was_busy = false;
                        *HIDDEN.lock().unwrap() = Some(std::time::Instant::now());
                    }
                    let hidden = *HIDDEN.lock().unwrap();
                    let q = Quiet {
                        busy,
                        hold: SPEECH_BUSY.load(Ordering::SeqCst),
                        hidden,
                        used: svc2.engine_used(),
                        minutes: svc2.settings.lock().unwrap().free_memory_minutes,
                        engine_busy: svc2.busy(),
                    };
                    let d = due(std::time::Instant::now(), &q);
                    if d.reap {
                        platform::reap(&handle);
                        let mut h = HIDDEN.lock().unwrap();
                        // A hide since the look counts from then.
                        if *h == hidden {
                            *h = None;
                        }
                    }
                    if d.free {
                        svc2.unload_engine();
                    }
                    // The AI's helper goes five minutes after its last use.
                    let live = handle.try_state::<Arc<live::Live>>().map(|l| l.inner().clone());
                    let (go, at) = assist_due(std::time::Instant::now(), live.as_ref().and_then(|l| l.idle_since()));
                    if go {
                        if let Some(l) = &live {
                            l.let_go();
                        }
                        // And a helper warmed for a question that never came.
                        if let Some(h) = handle.try_state::<Arc<assist::catalog::Held>>() {
                            h.cool();
                        }
                    }
                    let listening = busy && live.as_ref().is_some_and(|l| l.listening());
                    WAKE.wait(gen, sooner(if listening { Next::Tick(LEVEL_TICK) } else { d.next }, at));
                }
            });
            Ok(())
        })
        .on_window_event(|window, event| {
            // The answer panel sets its own height as the answer grows: it stays docked at the top right.
            if matches!(event, tauri::WindowEvent::Resized(_)) && window.label() == platform::PANEL {
                platform::redock_panel(window);
            }
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                // Closing a window frees it; Pointcast itself stays in the tray (see ExitRequested).
                let _ = api;
                if platform::DESK.contains(&window.label()) {
                    platform::dock(window.app_handle(), Some(window.label()));
                }
                if window.label() == "overlay" {
                    close_overlay(window.app_handle());
                }
                // History closed without pasting: the app it opened over is not where a later paste goes.
                if window.label() == "result" && !window.app_handle().try_state::<Arc<App>>().is_some_and(|svc| recording(&svc)) {
                    platform::forget_front();
                }
                if PAUSED.swap(false, Ordering::SeqCst) {
                    if let Some(svc) = window.app_handle().try_state::<Arc<App>>() {
                        rebind(window.app_handle(), &svc);
                    }
                }
                // The microphone check in Settings never outlives its window.
                if window.label() == "settings" {
                    if let Some(svc) = window.app_handle().try_state::<Arc<App>>() {
                        svc.monitor_off();
                    }
                }
                if window.label() == "onboarding" {
                    let app = window.app_handle();
                    if let Some(svc) = app.try_state::<Arc<App>>() {
                        leave_onboarding(app, &svc);
                    }
                }
            }
        })
        .build(tauri::generate_context!())
        .expect("error while building Pointcast")
        .run(|_app, event| match event {
            // The last window closing is not quitting: only Quit (an explicit exit code) ends it.
            tauri::RunEvent::ExitRequested { api, code: None, .. } => api.prevent_exit(),
            // Any way out (the app menu, logout, restart, the updater): what was said is kept for the next launch.
            tauri::RunEvent::Exit => {
                if let Some(svc) = _app.try_state::<Arc<App>>() {
                    svc.flush_for_quit();
                    svc.release_engine();
                }
            }
            // Clicking the Dock icon (shown while a window is open) brings Settings forward.
            #[cfg(target_os = "macos")]
            tauri::RunEvent::Reopen { .. } => show_window(_app, "settings"),
            _ => {}
        });
}

/// For tests that measure time and tests that keep every core busy for seconds: one at a time, so a timing test
/// measures the code and not its neighbours.
#[cfg(test)]
pub(crate) mod test_support {
    static ALONE: std::sync::Mutex<()> = std::sync::Mutex::new(());

    pub fn alone() -> std::sync::MutexGuard<'static, ()> {
        ALONE.lock().unwrap_or_else(|e| e.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_notice_hides_on_its_own_time_unless_the_pointer_is_on_it() {
        assert_eq!(hide_at(Some("pasted"), false, None), Some(1600), "pasted keeps today's timing");
        assert_eq!(hide_at(Some("copied"), false, None), Some(1600));
        assert_eq!(hide_at(Some("partial"), false, None), Some(4500));
        assert_eq!(hide_at(Some("kept"), false, None), Some(4500), "two lines to read");
        for n in ["pasted", "kept", "partial", "cut", "empty"] {
            assert_eq!(hide_at(Some(n), true, None), None, "{n}: never while hovered or focused");
            assert_eq!(hide_at(Some(n), true, Some(100)), None, "{n}: back on it after leaving");
        }
    }

    #[test]
    fn kept_partial_and_cut_stay_four_seconds_after_the_pointer_leaves() {
        for n in ["kept", "partial", "cut"] {
            assert_eq!(hide_at(Some(n), false, Some(10_000)), Some(14_000), "{n}");
            assert_eq!(hide_at(Some(n), false, Some(200)), Some(4_500), "{n}: never before its own time");
        }
        assert_eq!(hide_at(Some("pasted"), false, Some(10_000)), Some(11_600), "others linger their usual time");
    }

    #[test]
    fn the_hold_follows_the_pointer_and_remembers_when_it_left() {
        let h = Hold { gen: 3, ..Hold::default() };
        let on = hold_step(h, true, 500);
        assert_eq!(on, Hold { gen: 3, over: true, left_ms: None });
        let off = hold_step(on, false, 7_000);
        assert_eq!(off, Hold { gen: 3, over: false, left_ms: Some(7_000) });
        assert_eq!(hold_step(off, false, 9_000), off, "a second leave does not move the time");
        let walk = [true, false, true, false].iter().zip([1_000, 2_000, 3_000, 8_000]).fold(h, |h, (o, t)| hold_step(h, *o, t));
        assert_eq!(hide_at(Some("cut"), walk.over, walk.left_ms), Some(12_000));
    }

    #[test]
    fn the_speech_model_is_freed_the_chosen_time_after_its_last_use_however_it_was_loaded() {
        use std::time::{Duration, Instant};
        let t0 = Instant::now();
        let at = |s: u64| t0 + Duration::from_secs(s);
        // Loaded outside a recording (Transcribe again, a model change, the check, the GPU version) at t0.
        let loaded = Some(t0);
        assert!(!free_due(at(59), loaded, 1, false), "not before the chosen time");
        assert!(free_due(at(60), loaded, 1, false), "at it");
        assert!(free_due(at(3_600), loaded, 1, false), "and after it, if it was missed");
        // Loaded again after it was freed: freed again, with no recording in between.
        assert!(free_due(at(7_200), Some(at(7_000)), 1, false), "every load, not once per quiet spell");
        // A recording, a Transcribe again or a piece heard at 50 s pushes the time to 110 s.
        assert!(!free_due(at(60), Some(at(50)), 1, false), "a use in between pushes it back");
        assert!(free_due(at(110), Some(at(50)), 1, false), "to a minute after that use");
        assert!(!free_due(at(3_600), loaded, 1, true), "never while recording, finishing or transcribed again");
        assert!(!free_due(at(36_000), loaded, 0, false), "0 keeps the model loaded however long it is quiet");
        assert!(!free_due(at(3_600), None, 1, false), "nothing in memory, nothing to free");
        assert!(free_due(at(300), loaded, 5, false) && !free_due(at(299), loaded, 5, false), "a longer choice");
    }

    #[test]
    fn the_idle_scheduler_sleeps_until_the_next_deadline_and_ticks_only_while_a_session_runs() {
        use std::time::{Duration, Instant};
        let t0 = Instant::now();
        let at = |s: u64| t0 + Duration::from_secs(s);
        let quiet = Quiet { minutes: 1, ..Default::default() };
        // Nothing loaded, no window hidden: nothing to wake for.
        assert_eq!(due(at(0), &quiet), Due { reap: false, free: false, next: Next::Never });
        assert_eq!(due(at(86_400), &quiet).next, Next::Never, "however long it is quiet");
        // The HUD hid at 10 s: it goes at 70 s, not ten minutes later.
        let hid = Quiet { hidden: Some(at(10)), ..quiet };
        assert_eq!(due(at(11), &hid), Due { reap: false, free: false, next: Next::At(at(70)) });
        assert_eq!(due(at(69), &hid).next, Next::At(at(70)));
        assert_eq!(due(at(70), &hid), Due { reap: true, free: false, next: Next::Never }, "and then nothing is left");
        assert!(due(at(600), &hid).reap, "a late look still lets it go");
        // The engine, at its own time: the chosen minutes after its last use.
        let loaded = Quiet { used: Some(at(5)), ..quiet };
        assert_eq!(due(at(6), &loaded), Due { reap: false, free: false, next: Next::At(at(65)) });
        assert_eq!(due(at(65), &loaded), Due { reap: false, free: true, next: Next::Never });
        let five = Quiet { minutes: 5, ..loaded };
        assert_eq!(due(at(6), &five).next, Next::At(at(305)));
        assert_eq!(due(at(6), &Quiet { minutes: 0, ..loaded }).next, Next::Never, "0 keeps it loaded: no deadline");
        // Both: the earlier first, then the other.
        let both = Quiet { hidden: Some(at(10)), used: Some(at(30)), ..quiet };
        assert_eq!(due(at(11), &both).next, Next::At(at(70)));
        assert_eq!(due(at(70), &both), Due { reap: true, free: false, next: Next::At(at(90)) });
        assert_eq!(due(at(90), &Quiet { hidden: None, ..both }), Due { reap: false, free: true, next: Next::Never });
        let early = Quiet { hidden: Some(at(50)), used: Some(at(0)), ..quiet };
        assert_eq!(due(at(1), &early).next, Next::At(at(60)));
        assert_eq!(due(at(60), &early), Due { reap: false, free: true, next: Next::At(at(110)) });
        // A session runs: the 100 ms pulse, and nothing is let go meanwhile.
        let busy = Quiet { busy: true, engine_busy: true, ..both };
        assert_eq!(due(at(3_600), &busy), Due { reap: false, free: false, next: Next::Tick(Duration::from_millis(100)) });
        assert_eq!(due(at(0), &Quiet { busy: true, ..quiet }).next, Next::Tick(Duration::from_millis(100)));
        // A History entry transcribed again or a speech setup: nothing goes, and their end wakes the loop.
        assert_eq!(due(at(3_600), &Quiet { engine_busy: true, ..loaded }), Due { reap: false, free: false, next: Next::Never });
        assert_eq!(due(at(3_600), &Quiet { hold: true, ..both }), Due { reap: false, free: false, next: Next::Never });
        // Never a deadline already past (the loop would spin), over a spread of inputs.
        for now in (0..200).step_by(7) {
            for q in [quiet, hid, loaded, five, both, early] {
                if let Next::At(t) = due(at(now), &q).next {
                    assert!(t > at(now), "{q:?} at {now}");
                }
            }
        }
    }

    #[test]
    fn the_ais_helper_is_let_go_five_minutes_after_its_last_use_and_the_scheduler_wakes_for_it() {
        use std::time::{Duration, Instant};
        let t0 = Instant::now();
        let at = |s: u64| t0 + Duration::from_secs(s);
        assert_eq!(assist_due(at(10), None), (false, None), "nothing held, nothing to wake for");
        assert_eq!(assist_due(at(10), Some(at(5))), (false, Some(at(305))));
        assert_eq!(assist_due(at(304), Some(at(5))), (false, Some(at(305))));
        assert_eq!(assist_due(at(305), Some(at(5))), (true, None));
        assert_eq!(assist_due(at(9_000), Some(at(5))), (true, None), "a late look still lets it go");
        // The scheduler sleeps until the sooner of its own deadline and the helper's, and a pulse stays a pulse.
        assert_eq!(sooner(Next::Never, Some(at(305))), Next::At(at(305)));
        assert_eq!(sooner(Next::At(at(70)), Some(at(305))), Next::At(at(70)));
        assert_eq!(sooner(Next::At(at(400)), Some(at(305))), Next::At(at(305)));
        assert_eq!(sooner(Next::At(at(400)), None), Next::At(at(400)));
        assert_eq!(sooner(Next::Tick(Duration::from_millis(100)), Some(at(1))), Next::Tick(Duration::from_millis(100)));
        // Together with the rest of the loop: never a deadline already past.
        for now in (0..400).step_by(13) {
            if let (false, Some(t)) = assist_due(at(now), Some(at(5))) {
                assert!(t > at(now), "{now}");
            }
        }
    }

    #[test]
    fn a_settings_patch_cannot_choose_the_ai_or_record_a_yes() {
        let cur = Settings::default();
        for k in ["assist_provider", "assist_consent", "assist_offline_only", "assist_model"] {
            let mut patch = serde_json::Map::new();
            patch.insert(k.into(), json!(if k == "assist_consent" { json!([{ "provider": "openai", "goes_to": "api.openai.com", "at_ms": 1 }]) } else { json!("openai") }));
            assert!(merge(&cur, patch).is_err(), "{k}");
        }
        let mut sounds = serde_json::Map::new();
        sounds.insert("sounds".into(), json!(false));
        assert!(!merge(&cur, sounds).unwrap().sounds, "another setting still changes");
    }

    #[test]
    fn the_session_state_goes_to_every_window_so_the_answer_panel_hears_it() {
        // `session://state` is sent with `emit`, which reaches every window, the panel included; one sent to named
        // windows would have to name it.
        let src = std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src").join("lib.rs")).unwrap();
        let name = ["session:", "//state"].concat();
        let sends: Vec<&str> = src.lines().filter(|l| l.contains(&format!("\"{name}\"")) && !l.contains("let name")).collect();
        assert!(sends.len() >= 2, "the scan found the senders: {sends:?}");
        for l in &sends {
            assert!(l.contains(".emit(") && !l.contains("emit_to") && !l.contains("emit_filter"), "{l}");
        }
        // And the panel's capability lets its page listen: `core:default` holds the event permissions.
        let cap: Value = serde_json::from_slice(&std::fs::read(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("capabilities").join("live.json")).unwrap()).unwrap();
        assert!(cap["permissions"].as_array().unwrap().iter().any(|p| p == "core:default" || p == "core:event:default" || p == "core:event:allow-listen"));
    }

    #[test]
    fn a_wake_cuts_the_idle_sleep_short_and_a_deadline_ends_it() {
        use std::time::{Duration, Instant};
        let w = Arc::new(Waker::new());
        // Woken while asleep with no deadline.
        let gen = w.gen();
        let w2 = w.clone();
        let t = Instant::now();
        let waker = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(50));
            w2.wake();
        });
        assert!(w.wait(gen, Next::Never));
        waker.join().unwrap();
        assert!(t.elapsed() < Duration::from_secs(5));
        // A wake between the look and the sleep is not lost.
        let gen = w.gen();
        w.wake();
        assert!(w.wait(gen, Next::Never), "returns at once");
        // A deadline with no wake ends the sleep, not woken.
        let gen = w.gen();
        let t = Instant::now();
        assert!(!w.wait(gen, Next::At(t + Duration::from_millis(30))));
        assert!(t.elapsed() >= Duration::from_millis(30));
        assert!(!w.wait(w.gen(), Next::Tick(Duration::from_millis(1))));
    }

    #[test]
    fn a_tap_keeps_recording_and_a_hold_stops_on_release() {
        assert!(!release_stops(TalkMode::Tap, Some(0), 5_000));
        assert!(!release_stops(TalkMode::Hold, Some(1_000), 1_000 + HOLD_MS - 1));
        assert!(release_stops(TalkMode::Hold, Some(1_000), 1_000 + HOLD_MS));
        assert!(!release_stops(TalkMode::Hold, None, 9_000));
    }

    #[test]
    fn the_session_chord_over_the_open_overlay_keeps_the_box_then_stops() {
        assert_eq!(session_press(false, false), Press::Start);
        assert_eq!(session_press(true, false), Press::Stop);
        assert_eq!(session_press(true, true), Press::CommitThenStop);
    }

    #[test]
    fn a_patch_keeps_what_it_does_not_name_and_refuses_bad_chords() {
        let mut cur = Settings::default();
        cur.onboarded = true;
        cur.zoom_px = 640;
        let p = |v: Value| v.as_object().unwrap().clone();
        let next = merge(&cur, p(json!({ "sounds": false }))).unwrap();
        assert!(next.onboarded && !next.sounds && next.zoom_px == 640);
        let next = merge(&cur, p(json!({ "session_hotkey": "Ctrl+Alt+F9" })));
        if cfg!(target_os = "macos") {
            assert!(next.is_ok());
        } else {
            assert!(next.unwrap_err().contains("AltGr"));
        }
        let dup = merge(&cur, p(json!({ "full_hotkey": "Ctrl+Shift+F8", "zoom_hotkey": "Ctrl+Shift+F8" })));
        assert!(dup.unwrap_err().contains("already"));
        assert!(merge(&cur, p(json!({ "region_hotkey": "Shift" }))).is_err());
        let back = merge(&cur, p(json!({ "export_dir": "/tmp/x" }))).unwrap();
        let cleared = merge(&back, p(json!({ "export_dir": null }))).unwrap();
        assert_eq!(cleared.export_dir, None);
    }

    #[test]
    fn a_saved_chord_this_version_refuses_blocks_no_other_change() {
        let p = |v: Value| v.as_object().unwrap().clone();
        let old = Settings { zoom_hotkey: Some("F5".into()), ..Settings::default() };
        let next = merge(&old, p(json!({ "sounds": false }))).unwrap();
        assert!(!next.sounds && next.zoom_hotkey.as_deref() == Some("F5"), "kept as saved");
        assert!(merge(&old, p(json!({ "full_hotkey": "Ctrl+Shift+F8" }))).is_ok());
        let set = merge(&old, p(json!({ "zoom_hotkey": "F6" })));
        assert!(set.unwrap_err().contains("Add Ctrl, Alt or Shift"), "a chord being set is still checked");
        assert!(merge(&old, p(json!({ "zoom_hotkey": null }))).is_ok());
        let garbled = Settings { region_hotkey: Some("Ctrl+Shift+Nope+Nope".into()), ..Settings::default() };
        assert!(merge(&garbled, p(json!({ "model": "auto" }))).is_ok());
    }

    #[test]
    fn the_overlay_frame_says_where_the_person_pointed_unless_marking_is_off() {
        let shot = capture::Shot {
            png: vec![1, 2],
            width: 4,
            height: 3,
            kind: prompt::CaptureKind::Full,
            cursor: (0, 0),
            pointer: Some(pointer::Point { t_ms: 9, x: 3, y: 2 }),
            trail: vec![],
            element: None,
            map: Default::default(),
            k: 1.0,
        };
        let v = frame_payload(&shot, settings::MarkPoint::Ring);
        assert_eq!((v["width"].clone(), v["pointer"].clone()), (json!(4), json!({ "x": 3, "y": 2 })));
        assert!(v["element"].is_null());
        assert!(frame_payload(&shot, settings::MarkPoint::Off)["pointer"].is_null());
        let none = capture::Shot { pointer: None, ..shot.clone() };
        assert!(frame_payload(&none, settings::MarkPoint::Trail)["pointer"].is_null());
        let rect = element::Bounds { x: 1, y: 0, w: 2, h: 2 };
        let button = element::ElementMark { role: "button".into(), name: "Place order".into(), rect };
        let with = capture::Shot { element: Some(button), ..shot };
        let v = frame_payload(&with, settings::MarkPoint::Off);
        assert_eq!(v["element"], json!({ "role": "button", "name": "Place order", "rect": { "x": 1, "y": 0, "w": 2, "h": 2 } }));
        assert!(v["pointer"].is_null(), "the outline does not need the ring");
    }

    #[test]
    fn a_region_commit_takes_the_outlined_control_or_none() {
        let body: RegionCommit = serde_json::from_value(json!({
            "png_b64": "",
            "labels": [{ "n": 1, "text": "too far left" }],
            "element": { "role": "button", "name": "Place order", "rect": { "x": 4, "y": 6, "w": 116, "h": 41 } },
        }))
        .unwrap();
        let e = body.element.unwrap();
        assert_eq!((e.role.as_str(), e.name.as_str(), e.rect.w), ("button", "Place order", 116));
        let old: RegionCommit = serde_json::from_value(json!({ "png_b64": "", "labels": [] })).unwrap();
        assert!(old.element.is_none());
        let gone: RegionCommit = serde_json::from_value(json!({ "png_b64": "", "element": null })).unwrap();
        assert!(gone.element.is_none());
        let steps: RegionCommit = serde_json::from_value(json!({ "png_b64": "", "labels": [{ "n": 1, "text": "x" }, { "n": 2, "text": "", "kind": "step" }] })).unwrap();
        let kinds: Vec<_> = steps.labels.iter().map(|l| l.kind).collect();
        assert_eq!(kinds, [prompt::LabelKind::Pin, prompt::LabelKind::Step]);
    }

    #[test]
    fn settings_say_which_models_are_slower_than_real_time_here_once_this_run_knows() {
        let cat = voice::models::catalog();
        let mut speech = voice::models::view(&std::env::temp_dir().join("pc-no-speech"), &cat);
        let slow = [("whisper-turbo", true), ("moonshine-base-en", false), ("not-in-the-catalog", true)];
        mark_slow(&mut speech, &slow.into_iter().map(|(m, s)| (m.to_string(), s)).collect());
        let said = |id: &str| speech["models"].as_array().unwrap().iter().find(|m| m["id"] == id).unwrap()["slow_engine"].clone();
        assert_eq!((said("whisper-turbo"), said("moonshine-base-en")), (json!(true), json!(false)));
        assert!(said("moonshine-tiny-en").is_null(), "a model not judged yet says nothing");
    }

    #[test]
    fn setting_up_whisper_turbo_fetches_the_gpu_version_where_its_helper_can_run() {
        let cat = voice::models::catalog();
        let ids = |r: (Option<&voice::models::RuntimeEntry>, &voice::models::ModelEntry)| (r.0.map(|x| x.id.clone()), r.1.id.clone());
        let helper = cat.helper_for_host().map(|h| h.id.clone());
        let sherpa = cat.runtime_for_host().map(|r| r.id.clone());
        assert_eq!(ids(setup_parts(&cat, "whisper-turbo", true, false).unwrap()), (helper, "whisper-turbo-gpu".into()));
        // Found unusable here (or none published): sherpa's Whisper Turbo.
        assert_eq!(ids(setup_parts(&cat, "whisper-turbo", false, false).unwrap()), (sherpa.clone(), "whisper-turbo".into()));
        assert_eq!(ids(setup_parts(&cat, "whisper-turbo", true, true).unwrap()), (None, "whisper-turbo-gpu".into()), "a development helper is not downloaded");
        assert_eq!(ids(setup_parts(&cat, "moonshine-base-en", true, false).unwrap()), (sherpa, "moonshine-base-en".into()));
        assert!(setup_parts(&cat, "nope", true, false).is_err());
    }

    #[test]
    fn settings_say_where_the_model_in_memory_runs() {
        let cat = voice::models::catalog();
        let mut speech = voice::engine::view(&std::env::temp_dir().join("pc-no-speech"), &cat);
        mark_backend(&mut speech, &[("whisper-turbo".to_string(), "gpu")].into_iter().collect(), Some("Apple M2 via Metal".into()));
        mark_speed(&mut speech, &[("whisper-turbo".to_string(), 19.96)].into_iter().collect());
        let said = |id: &str, k: &str| speech["models"].as_array().unwrap().iter().find(|m| m["id"] == id).unwrap()[k].clone();
        assert_eq!(said("whisper-turbo", "backend"), json!("gpu"));
        assert_eq!((said("whisper-turbo", "device"), said("whisper-turbo", "speed_x")), (json!("Apple M2 via Metal"), json!(20.0)));
        assert!(said("moonshine-base-en", "backend").is_null(), "a model not in memory says nothing");
        assert!(said("moonshine-base-en", "speed_x").is_null() && said("moonshine-base-en", "device").is_null());
    }

    fn png(w: u32, v: u8) -> Vec<u8> {
        use image::ImageEncoder;
        let mut buf = Vec::new();
        let px: Vec<u8> = (0..w).flat_map(|_| [v, 24, 24, 255]).collect();
        image::codecs::png::PngEncoder::new(&mut buf).write_image(&px, w, 1, image::ExtendedColorType::Rgba8).unwrap();
        buf
    }

    #[test]
    fn a_history_picture_opens_for_editing_and_one_commit_goes_back_into_it() {
        let dir = std::env::temp_dir().join(format!("pc-edit-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let store = history::Store::new(&dir);
        let pic = prompt::Capture {
            id: 0,
            kind: prompt::CaptureKind::Region,
            t_ms: 10,
            png: png(2, 200),
            width: 2,
            height: 1,
            labels: vec![Label { n: 1, text: "old".into(), ..Default::default() }],
            pointer: None,
            trail: vec![],
            element: None,
            place: None,
            anchors: vec![],
            auto: false,
        };
        let items = vec![
            prompt::Item::Speech { t_ms: 0, end_ms: 9, text: "look".into() },
            prompt::Item::Image { t_ms: 10, capture: pic },
        ];
        let m = store.save(&items, 1_793_000_000_000, 100).unwrap();
        let ed = Editing(std::sync::Mutex::new(None));
        assert!(ed.commit(&store, &png(3, 1), vec![]).is_none(), "not editing: a commit belongs to the recording");
        assert!(ed.begin(&store, "../settings", 1).is_err() && ed.begin(&store, &m.id, 2).is_err());
        assert!(ed.0.lock().unwrap().is_none(), "a refused edit opens nothing");

        let f = ed.begin(&store, &m.id, 1).unwrap();
        assert_eq!((&f["edit"], &f["width"], &f["height"], &f["base"]), (&json!(true), &json!(2), &json!(1), &json!(1)));
        assert!(f["pointer"].is_null() && f["element"].is_null(), "marks already drawn into it are not placed again");
        let b64 = f["png_b64"].as_str().unwrap();
        assert_eq!(base64::Engine::decode(&base64::engine::general_purpose::STANDARD, b64).unwrap(), png(2, 200));
        ed.cancel();
        assert!(ed.commit(&store, &png(3, 1), vec![]).is_none(), "Esc: nothing is written");
        assert_eq!(store.picture(&m.id, 1).unwrap().png, png(2, 200));

        ed.begin(&store, &m.id, 1).unwrap();
        assert!(ed.commit(&store, b"not a png", vec![]).unwrap().is_err());
        assert!(ed.0.lock().unwrap().is_some(), "a failed write keeps the edit open");
        let (id, cap) = ed.commit(&store, &png(3, 1), vec![Label { n: 1, text: "new".into(), ..Default::default() }]).unwrap().unwrap();
        assert_eq!((id.as_str(), cap.width, cap.labels.len()), (m.id.as_str(), 3, 2));
        assert_eq!(store.picture(&m.id, 1).unwrap().png, png(3, 1));
        assert!(ed.commit(&store, &png(4, 1), vec![]).is_none(), "written once; the next commit is the recording's again");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_patch_sets_the_capture_marks_and_refuses_unknown_ones() {
        let p = |v: Value| v.as_object().unwrap().clone();
        let next = merge(&Settings::default(), p(json!({ "mark_point": "trail", "zoom_target": "window", "zoom_sharpen": false }))).unwrap();
        assert_eq!((next.mark_point, next.zoom_target, next.zoom_sharpen), (settings::MarkPoint::Trail, settings::ZoomTarget::Window, false));
        assert!(merge(&next, p(json!({ "mark_point": "sparkles" }))).is_err());
        let back = merge(&next, p(json!({ "mark_point": null }))).unwrap();
        assert_eq!(back.mark_point, settings::MarkPoint::Ring);
        let quiet = merge(&back, p(json!({ "outline_element": false }))).unwrap();
        assert!(!quiet.outline_element && back.outline_element);
        let mute = merge(&quiet, p(json!({ "spoken_anchors": "off" }))).unwrap();
        assert_eq!((mute.spoken_anchors, quiet.spoken_anchors), (settings::SpokenAnchors::Off, settings::SpokenAnchors::Ring));
        assert!(merge(&mute, p(json!({ "spoken_anchors": "trail" }))).is_err());
    }

    #[test]
    fn only_the_model_a_recording_would_load_counts_as_in_use() {
        let both = ["moonshine-tiny-en", "moonshine-base-en"];
        assert!(in_use("moonshine-tiny-en", "moonshine-base-en", &both, "moonshine-tiny-en"));
        assert!(!in_use("moonshine-tiny-en", "moonshine-base-en", &both, "moonshine-base-en"));
        assert!(in_use("auto", "moonshine-base-en", &both, "moonshine-base-en"));
        assert!(!in_use("auto", "moonshine-base-en", &both, "moonshine-tiny-en"));
        // The chosen one is not downloaded: the engine falls back to what is there.
        assert!(in_use("whisper-turbo", "moonshine-base-en", &["moonshine-tiny-en"], "moonshine-tiny-en"));
    }

    #[test]
    fn a_paste_that_cannot_happen_says_the_prompt_was_only_copied() {
        let tried = std::cell::Cell::new(false);
        let r = paste_or_say(false, || {
            tried.set(true);
            Ok(())
        });
        assert_eq!(r.unwrap_err(), PASTE_YOURSELF);
        assert!(!tried.get(), "without permission no keys are sent");
        assert_eq!(paste_or_say(true, || Err("refused".into())).unwrap_err(), PASTE_YOURSELF);
        assert!(paste_or_say(true, || Ok(())).is_ok());
    }

    #[test]
    fn history_stays_or_comes_back_to_say_a_paste_did_not_happen() {
        let log = std::cell::RefCell::new(Vec::<&str>::new());
        let step = |s| log.borrow_mut().push(s);
        let pasted = || {
            step("paste");
            Ok(())
        };
        let refused = || {
            step("paste");
            Err("refused".to_string())
        };
        let r = paste_out_of(false, || step("hide"), pasted, || step("back"));
        assert_eq!(r.unwrap_err(), PASTE_YOURSELF);
        assert!(log.borrow().is_empty(), "without permission to paste History is not hidden");
        let r = paste_out_of(true, || step("hide"), refused, || step("back"));
        assert_eq!(r.unwrap_err(), PASTE_YOURSELF);
        assert_eq!(*log.borrow(), ["hide", "paste", "back"], "a paste the app refused brings History back");
        log.borrow_mut().clear();
        assert!(paste_out_of(true, || step("hide"), pasted, || step("back")).is_ok());
        assert_eq!(*log.borrow(), ["hide", "paste"]);
    }

    #[test]
    fn a_saved_chord_this_version_refuses_says_why_not_taken() {
        assert!(refused(&Settings::default()).is_empty());
        let old = Settings { zoom_hotkey: Some("F5".into()), ..Settings::default() };
        let r = refused(&old);
        assert_eq!(r.len(), 1, "{r:?}");
        assert_eq!(r[0].0, "zoom");
        assert!(r[0].1.contains("Add Ctrl, Alt or Shift") && r[0].1.contains(&old.zoom_chord().label()), "{}", r[0].1);
    }

    #[test]
    fn the_clock_reads_minutes_and_seconds() {
        assert_eq!(clock(0), "0:00");
        assert_eq!(clock(65_400), "1:05");
    }

    #[test]
    fn the_ask_key_asks_only_when_no_dictation_records_and_other_keys_leave_a_question_alone() {
        use Action::*;
        // Nothing records: the Ask key asks, the others go their usual way.
        assert_eq!(routed(Ask, false, false, false), Some(true));
        assert_eq!([Session, Full, Region, Zoom].map(|a| routed(a, false, false, false)), [Some(false); 4]);
        // A dictation records: the Ask key does nothing, so a dictation never opens the panel.
        assert_eq!(routed(Ask, true, false, false), None);
        assert_eq!(routed(Session, true, false, false), Some(false));
        // A question is being said: the Ask key ends it, and no other chord stops, pastes or adds a picture to it.
        assert_eq!(routed(Ask, true, true, false), Some(true));
        assert_eq!([Session, Full, Region, Zoom].map(|a| routed(a, true, true, false)), [None; 4]);
        // Onboarding's key test only hears the Ask key.
        assert_eq!(routed(Ask, false, false, true), None);
    }

    #[test]
    fn the_ask_key_is_refused_when_it_is_another_shortcut_and_rebinds_like_the_others() {
        let cur = Settings::default();
        let p = |v: Value| v.as_object().unwrap().clone();
        assert_eq!(merge(&cur, p(json!({ "ask_hotkey": "Ctrl+Shift+F7" }))).unwrap().ask_chord().label(), "Ctrl+Shift+F7");
        // Refused the way the others are: a chord saved for another shortcut cannot also be the Ask key.
        let err = merge(&cur, p(json!({ "session_hotkey": "Ctrl+Shift+F9", "ask_hotkey": "Ctrl+Shift+F9" }))).unwrap_err();
        assert!(err.contains("already Start / stop"), "{err}");
        let taken = Settings { zoom_hotkey: Some("Ctrl+Shift+F8".into()), ..Settings::default() };
        let err = merge(&taken, p(json!({ "ask_hotkey": "Ctrl+Shift+F8" }))).unwrap_err();
        assert!(err.contains("already Capture around pointer"), "{err}");
        assert!(merge(&cur, p(json!({ "ask_hotkey": "Shift" }))).is_err(), "a chord needs a key");
        assert!(keys_changed(&cur, &Settings { ask_hotkey: Some("Ctrl+Shift+F7".into()), ..Settings::default() }));
    }
}
