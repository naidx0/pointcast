//! Menu bar extra on macOS (template glyph, elapsed time beside it while recording), tray icon on
//! Windows. The menu names the user's real shortcuts and follows the session state.
use crate::live::State as Ask;
use crate::session::{App, Phase, UiState};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::image::Image;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, Wry};

pub struct Tray {
    record: MenuItem<Wry>,
    full: MenuItem<Wry>,
    region: MenuItem<Wry>,
    zoom: MenuItem<Wry>,
    paste: MenuItem<Wry>,
    update: MenuItem<Wry>,
    shown: Mutex<Option<bool>>,
    menu: Menu<Wry>,
    /// Today's words, a line under History that is in the menu only while there are some.
    today: MenuItem<Wry>,
    today_shown: Mutex<Option<String>>,
    /// Pip's face: the session's last phase, the Ask panel's state, until when "done" shows after a paste, and the
    /// glyph on screen now (face, light taskbar, size in pixels).
    phase: Mutex<Phase>,
    ask: Mutex<Ask>,
    done_until: Mutex<Option<Instant>>,
    face_shown: Mutex<Option<(Face, bool, u32)>>,
}

/// Where today's line goes in the menu: after History.
const TODAY_AT: usize = 9;

/// Pip's face in the tray, which follows the app: ready, listening (a recording, dictated or asked), thinking (the
/// last words transcribed, or the AI thinking and answering) and done (briefly, after a paste).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Face {
    Idle,
    Listen,
    Think,
    Done,
}

/// How long the done face shows after a paste.
const DONE_FOR: Duration = Duration::from_millis(1500);

/// The face for the session's phase and the Ask panel's state; `done` while a paste is fresh.
fn face(phase: Phase, ask: Ask, done: bool) -> Face {
    match (phase, ask) {
        (Phase::Recording | Phase::Loading, _) | (_, Ask::Listening) => Face::Listen,
        (Phase::Transcribing, _) | (_, Ask::Finishing | Ask::Sending | Ask::Answering) => Face::Think,
        _ if done => Face::Done,
        _ => Face::Idle,
    }
}

/// A dictation was just pasted: the session left recording or transcribing for Ready with "pasted".
fn just_pasted(before: Phase, now: Phase, notice: Option<&str>) -> bool {
    matches!(before, Phase::Recording | Phase::Loading | Phase::Transcribing) && now == Phase::Ready && notice == Some("pasted")
}

macro_rules! win_sizes {
    ($face:literal, $colour:literal) => {
        [
            include_bytes!(concat!("../icons/tray/win-", $face, "-", $colour, "-16.png")) as &[u8],
            include_bytes!(concat!("../icons/tray/win-", $face, "-", $colour, "-20.png")) as &[u8],
            include_bytes!(concat!("../icons/tray/win-", $face, "-", $colour, "-24.png")) as &[u8],
            include_bytes!(concat!("../icons/tray/win-", $face, "-", $colour, "-32.png")) as &[u8],
        ]
    };
}

/// The Windows tray's small icon at 100, 125, 150 and 200 %: one image per scale, so the shell does not resample.
const WIN_SIZES: [u32; 4] = [16, 20, 24, 32];

/// The PNG for a face. macOS: a black template the menu bar tints. Windows: white on a dark taskbar, ink on a light
/// one, at the smallest drawn size that covers `px` (the system's small icon size).
fn glyph_png(face: Face, mac: bool, light: bool, px: u32) -> &'static [u8] {
    let i = face as usize;
    if mac {
        const MAC: [&[u8]; 4] = [
            include_bytes!("../icons/tray/mac-idle.png"),
            include_bytes!("../icons/tray/mac-listen.png"),
            include_bytes!("../icons/tray/mac-think.png"),
            include_bytes!("../icons/tray/mac-done.png"),
        ];
        return MAC[i];
    }
    const WHITE: [[&[u8]; 4]; 4] =
        [win_sizes!("idle", "white"), win_sizes!("listen", "white"), win_sizes!("think", "white"), win_sizes!("done", "white")];
    const INK: [[&[u8]; 4]; 4] =
        [win_sizes!("idle", "ink"), win_sizes!("listen", "ink"), win_sizes!("think", "ink"), win_sizes!("done", "ink")];
    let at = WIN_SIZES.iter().position(|&s| s >= px).unwrap_or(WIN_SIZES.len() - 1);
    (if light { INK } else { WHITE })[i][at]
}

fn glyph(face: Face, light: bool, px: u32) -> Option<Image<'static>> {
    Image::from_bytes(glyph_png(face, cfg!(target_os = "macos"), light, px)).ok()
}

/// Whether the Windows taskbar is light. It follows SystemUsesLightTheme, not the apps' own light or dark mode.
#[cfg(windows)]
fn light_taskbar() -> bool {
    use windows::core::HSTRING;
    use windows::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD};
    // The key's path is joined here so this file needs no escaped separators.
    let sep = char::from(92u8).to_string();
    let key = HSTRING::from(["Software", "Microsoft", "Windows", "CurrentVersion", "Themes", "Personalize"].join(&sep));
    let (mut v, mut n) = (0u32, std::mem::size_of::<u32>() as u32);
    let r = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            &key,
            &HSTRING::from("SystemUsesLightTheme"),
            RRF_RT_REG_DWORD,
            None,
            Some(&mut v as *mut u32 as *mut _),
            Some(&mut n),
        )
    };
    r.is_ok() && v == 1
}

#[cfg(not(windows))]
fn light_taskbar() -> bool {
    false
}

/// The tray's small icon size in pixels for the system's scale (16 at 100 %, 24 at 150 %).
#[cfg(windows)]
fn tray_px() -> u32 {
    use windows::Win32::UI::HiDpi::{GetDpiForSystem, GetSystemMetricsForDpi};
    use windows::Win32::UI::WindowsAndMessaging::SM_CXSMICON;
    let dpi = unsafe { GetDpiForSystem() }.max(96);
    (unsafe { GetSystemMetricsForDpi(SM_CXSMICON, dpi) }).max(16) as u32
}

#[cfg(not(windows))]
fn tray_px() -> u32 {
    16
}

/// A menu item that shows its shortcut; falls back to plain text if the OS cannot draw it.
fn item(app: &AppHandle, id: &str, text: &str, accel: Option<String>) -> tauri::Result<MenuItem<Wry>> {
    match accel {
        Some(a) => MenuItem::with_id(app, id, text, true, Some(a.as_str()))
            .or_else(|_| MenuItem::with_id(app, id, text, true, None::<&str>)),
        None => MenuItem::with_id(app, id, text, true, None::<&str>),
    }
}

pub fn setup(app: &AppHandle, svc: &Arc<App>) -> tauri::Result<()> {
    let s = svc.settings.lock().unwrap().clone();
    let record = item(app, "record", "Start Recording", Some(s.session_chord().accelerator()))?;
    let ask = item(app, "ask", "Type a Question…", None)?;
    let full = item(app, "full", "Capture Screen", Some(s.full_chord().accelerator()))?;
    let region = item(app, "region", "Capture Region…", Some(s.region_chord().accelerator()))?;
    let zoom = item(app, "zoom", "Capture Around Pointer", Some(s.zoom_chord().accelerator()))?;
    let paste = item(app, "paste", "Paste Last Prompt", None)?;
    let last = item(app, "last", "History…", None)?;
    let settings = item(app, "settings", "Settings…", Some("CmdOrCtrl+,".into()))?;
    let setup = item(app, "setup", "Setup Assistant…", None)?;
    let update = item(app, "update", CHECK, None)?;
    let quit = item(app, "quit", "Quit Pointcast", Some("CmdOrCtrl+Q".into()))?;
    let today = MenuItem::with_id(app, "today", "", false, None::<&str>)?;
    let sep = || PredefinedMenuItem::separator(app);
    let menu = Menu::with_items(
        app,
        &[
            &record, &ask, &sep()?, &full, &region, &zoom, &sep()?, &paste, &last, &sep()?, &settings, &setup,
            &update, &sep()?, &quit,
        ],
    )?;
    let mut b = TrayIconBuilder::with_id("main")
        .tooltip("Pointcast")
        .menu(&menu)
        .show_menu_on_left_click(true)
        // Yesterday's count is not today's: the line is looked at again as the pointer reaches the icon.
        .on_tray_icon_event(|t, ev| {
            if matches!(ev, TrayIconEvent::Enter { .. } | TrayIconEvent::Click { .. }) {
                self::today(t.app_handle());
            }
        })
        .on_menu_event(|app, ev| {
            let Some(svc) = app.try_state::<Arc<App>>() else {
                return;
            };
            let svc = svc.inner().clone();
            match ev.id().as_ref() {
                "record" => {
                    let _ = crate::toggle(app, &svc);
                }
                // The Ask panel opens on a new question, its field ready: asking without the microphone.
                "ask" => {
                    if let Some(live) = app.try_state::<Arc<crate::live::Live>>() {
                        live.fresh_shown();
                    }
                }
                "full" => crate::hotkey(app, crate::Action::Full, true),
                "region" => crate::hotkey(app, crate::Action::Region, true),
                "zoom" => crate::hotkey(app, crate::Action::Zoom, true),
                "paste" => {
                    if crate::deliver_last(&svc).is_err() {
                        crate::show_window(app, "result");
                    }
                }
                "last" => crate::show_window(app, "result"),
                "settings" => crate::show_window(app, "settings"),
                "setup" => crate::show_window(app, "onboarding"),
                // Open Settings, then check: it says what an update does before anything is replaced, and reads the
                // answer from settings_view if it arrives before the window listens.
                "update" => {
                    crate::show_window(app, "settings");
                    let _ = app.emit_to("settings", "settings://pane", "general");
                    let app = app.clone();
                    std::thread::spawn(move || {
                        match crate::update::check() {
                            Ok(v) => {
                                offer_update(&app, v.as_deref());
                                let _ = app.emit("update://checked", &v);
                            }
                            Err(e) => {
                                let _ = app.emit("update://failed", e);
                            }
                        }
                    });
                }
                "quit" => crate::quit(app),
                _ => {}
            }
        });
    if let Some(icon) = glyph(Face::Idle, light_taskbar(), tray_px()) {
        b = b.icon(icon).icon_as_template(cfg!(target_os = "macos"));
    } else if let Some(icon) = app.default_window_icon() {
        b = b.icon(icon.clone());
    }
    b.build(app)?;
    app.manage(Tray {
        record,
        full,
        region,
        zoom,
        paste,
        update,
        shown: Mutex::new(None),
        menu,
        today,
        today_shown: Mutex::new(None),
        phase: Mutex::new(Phase::Idle),
        ask: Mutex::new(Ask::Closed),
        done_until: Mutex::new(None),
        face_shown: Mutex::new(Some((Face::Idle, light_taskbar(), tray_px()))),
    });
    refresh(app, &svc.ui(), svc.last_export().is_some());
    Ok(())
}

/// Follow the session: menu wording, icon and (macOS) the elapsed time in the menu bar.
pub fn refresh(app: &AppHandle, st: &UiState, has_last: bool) {
    let (Some(tray), Some(t)) = (app.try_state::<Tray>(), app.tray_by_id("main")) else {
        return;
    };
    let rec = matches!(st.phase, Phase::Recording | Phase::Loading);
    let before = std::mem::replace(&mut *tray.phase.lock().unwrap(), st.phase);
    if just_pasted(before, st.phase, st.notice) {
        *tray.done_until.lock().unwrap() = Some(Instant::now() + DONE_FOR);
        let app = app.clone();
        std::thread::spawn(move || {
            std::thread::sleep(DONE_FOR);
            show_face(&app);
        });
    }
    show_face(app);
    let _ = tray.paste.set_enabled(has_last);
    today(app);
    #[cfg(target_os = "macos")]
    {
        let _ = t.set_title(rec.then(|| crate::clock(st.duration_ms)));
    }
    // Decide under the lock, update outside it: off the main thread each setter waits for the main
    // thread, which may itself be waiting for this lock in another refresh.
    if !flip(&tray.shown, rec) {
        return;
    }
    let _ = tray
        .record
        .set_text(if rec { "Stop Recording" } else { "Start Recording" });
    let _ = t.set_tooltip(Some(if rec { "Pointcast — recording" } else { "Pointcast" }));
}

/// The Ask panel moved on: Pip listens while it listens and thinks while the AI thinks and answers.
pub fn ask(app: &AppHandle, state: Ask) {
    let Some(tray) = app.try_state::<Tray>() else {
        return;
    };
    *tray.ask.lock().unwrap() = state;
    show_face(app);
}

/// Put the face the app's state calls for in the tray, if it is not there already. The taskbar's theme and the
/// system's scale are read each time, so a change of either is followed at the next change of state.
fn show_face(app: &AppHandle) {
    let (Some(tray), Some(t)) = (app.try_state::<Tray>(), app.tray_by_id("main")) else {
        return;
    };
    let done = tray.done_until.lock().unwrap().is_some_and(|u| Instant::now() < u);
    let now = face(*tray.phase.lock().unwrap(), *tray.ask.lock().unwrap(), done);
    let (light, px) = (light_taskbar(), tray_px());
    if !flip(&tray.face_shown, (now, light, px)) {
        return;
    }
    if let Some(icon) = glyph(now, light, px) {
        let _ = t.set_icon(Some(icon));
        let _ = t.set_icon_as_template(cfg!(target_os = "macos"));
    }
}

/// Record the new state; true only for the caller that changed it.
fn flip<T: PartialEq + Copy>(shown: &Mutex<Option<T>>, now: T) -> bool {
    let mut s = shown.lock().unwrap();
    if *s == Some(now) {
        return false;
    }
    *s = Some(now);
    true
}

/// Today's line in the menu: "Today: 1,240 words"; none before the first word of the day.
fn today_text(words: u64) -> Option<String> {
    let digits = words.to_string();
    let mut n = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            n.push(',');
        }
        n.push(c);
    }
    (words > 0).then(|| format!("Today: {n} {}", if words == 1 { "word" } else { "words" }))
}

/// Put today's words in the menu, or take the line out when there are none. The count is in memory: nothing is read
/// here after the first time. Decided under the lock and changed outside it, as `refresh` does.
pub fn today(app: &AppHandle) {
    let (Some(tray), Some(svc)) = (app.try_state::<Tray>(), app.try_state::<Arc<App>>()) else {
        return;
    };
    let text = today_text(svc.stats.today_words());
    let was = {
        let mut shown = tray.today_shown.lock().unwrap();
        if *shown == text {
            return;
        }
        std::mem::replace(&mut *shown, text.clone())
    };
    match text {
        Some(t) => {
            let _ = tray.today.set_text(t);
            if was.is_none() {
                let _ = tray.menu.insert(&tray.today, TODAY_AT);
            }
        }
        None => {
            let _ = tray.menu.remove(&tray.today);
        }
    }
}

const CHECK: &str = "Check for Updates…";

/// Name the newer version in the menu once a check finds one.
pub fn offer_update(app: &AppHandle, version: Option<&str>) {
    if let Some(tray) = app.try_state::<Tray>() {
        let text = version.map_or_else(|| CHECK.to_string(), |v| format!("Update to Pointcast {v}…"));
        let _ = tray.update.set_text(text);
    }
}

/// Rebuild the shortcut hints after the user changes a chord.
pub fn rekey(app: &AppHandle, svc: &Arc<App>) {
    let Some(tray) = app.try_state::<Tray>() else {
        return;
    };
    let s = svc.settings.lock().unwrap().clone();
    let pairs = [
        (&tray.record, s.session_chord()),
        (&tray.full, s.full_chord()),
        (&tray.region, s.region_chord()),
        (&tray.zoom, s.zoom_chord()),
    ];
    for (item, chord) in pairs {
        let _ = item.set_accelerator(Some(chord.accelerator().as_str()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_change_of_state_updates_the_tray_and_the_lock_is_free_after() {
        let shown = Mutex::new(None);
        assert!(flip(&shown, true));
        assert!(!flip(&shown, true));
        assert!(flip(&shown, false));
        assert!(shown.try_lock().is_ok(), "nothing holds the lock while the tray is updated");
    }

    const FACES: [Face; 4] = [Face::Idle, Face::Listen, Face::Think, Face::Done];

    #[test]
    fn every_face_has_its_own_glyph_on_both_platforms_at_every_tray_size() {
        let mut seen = std::collections::HashSet::new();
        for f in FACES {
            let mac = glyph_png(f, true, false, 16);
            assert_eq!(glyph_png(f, true, true, 32), mac, "the menu bar template is one image the system tints");
            assert!(Image::from_bytes(mac).is_ok(), "{f:?} macOS glyph decodes");
            assert!(seen.insert(mac), "{f:?} macOS glyph is its own");
            for light in [false, true] {
                for px in WIN_SIZES {
                    let png = glyph_png(f, false, light, px);
                    let im = Image::from_bytes(png).unwrap_or_else(|e| panic!("{f:?} {light} {px}: {e}"));
                    assert_eq!((im.width(), im.height()), (px, px), "{f:?} light={light} is drawn at {px} px");
                    assert!(seen.insert(png), "{f:?} light={light} {px} px is its own image");
                }
            }
        }
        assert_eq!(seen.len(), 4 + 4 * 2 * WIN_SIZES.len());
    }

    #[test]
    fn the_tray_takes_the_smallest_drawn_size_that_covers_the_system_size() {
        let px = |want| Image::from_bytes(glyph_png(Face::Idle, false, false, want)).unwrap().width();
        assert_eq!([px(16), px(18), px(20), px(24), px(28), px(32), px(48)], [16, 20, 20, 24, 32, 32, 32]);
    }

    #[test]
    fn pip_listens_thinks_and_is_done_as_the_app_is() {
        use Phase::*;
        assert_eq!(face(Idle, Ask::Closed, false), Face::Idle);
        assert_eq!(face(Ready, Ask::Shown, false), Face::Idle);
        assert_eq!(face(Recording, Ask::Closed, false), Face::Listen);
        assert_eq!(face(Loading, Ask::Closed, false), Face::Listen);
        assert_eq!(face(Recording, Ask::Listening, false), Face::Listen, "an Ask listens through a recording");
        assert_eq!(face(Idle, Ask::Listening, false), Face::Listen);
        assert_eq!(face(Transcribing, Ask::Closed, false), Face::Think);
        for a in [Ask::Finishing, Ask::Sending, Ask::Answering] {
            assert_eq!(face(Idle, a, false), Face::Think, "{a:?}");
        }
        assert_eq!(face(Ready, Ask::Closed, true), Face::Done);
        assert_eq!(face(Recording, Ask::Closed, true), Face::Listen, "a new recording ends the done face");
        for a in [Ask::Error, Ask::Discarded, Ask::Shown, Ask::Closed] {
            assert_eq!(face(Idle, a, false), Face::Idle, "{a:?}");
        }
    }

    #[test]
    fn done_shows_only_on_the_step_into_a_paste() {
        assert!(just_pasted(Phase::Transcribing, Phase::Ready, Some("pasted")));
        assert!(just_pasted(Phase::Recording, Phase::Ready, Some("pasted")));
        assert!(!just_pasted(Phase::Ready, Phase::Ready, Some("pasted")), "a later refresh does not show it again");
        assert!(!just_pasted(Phase::Transcribing, Phase::Ready, Some("copied")));
        assert!(!just_pasted(Phase::Transcribing, Phase::Error, None));
    }

    #[test]
    fn todays_line_groups_thousands_and_is_not_there_at_zero() {
        assert_eq!(today_text(0), None);
        assert_eq!(today_text(1).as_deref(), Some("Today: 1 word"));
        assert_eq!(today_text(999).as_deref(), Some("Today: 999 words"));
        assert_eq!(today_text(1_240).as_deref(), Some("Today: 1,240 words"));
        assert_eq!(today_text(1_234_567).as_deref(), Some("Today: 1,234,567 words"));
    }
}
