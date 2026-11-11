//! Persisted Pointcast settings (`<data dir>/settings.json`).
use crate::chord::{self, Chord};
use crate::privacy;
use crate::voice::ptt::TalkMode;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutputMode {
    #[default]
    Clipboard,
    AutoPaste,
    Folder,
}

/// What a capture draws where the person pointed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MarkPoint {
    Off,
    #[default]
    Ring,
    /// The ring and the last few seconds of the path.
    Trail,
}

/// What happens, at stop, where the person pointed when they said "this" or "here".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpokenAnchors {
    Off,
    /// A ring on the picture nearest in time.
    #[default]
    Ring,
}

/// What a close-up centres on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ZoomTarget {
    Pointer,
    /// Where the pointer rested in the last moment, else the pointer.
    #[default]
    Dwell,
    /// The window under the pointer.
    Window,
}

/// What the picture of a question shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AskTarget {
    /// The window under the pointer, else the pointer's screen.
    #[default]
    Window,
    /// The pointer's whole screen.
    Screen,
}

/// Permissions this computer has given Pointcast at some point.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Granted {
    #[serde(default)]
    pub screen: bool,
    #[serde(default)]
    pub accessibility: bool,
}

impl Granted {
    /// Add what is granted now. True when that is news.
    pub fn note(&mut self, screen: bool, accessibility: bool) -> bool {
        let was = *self;
        self.screen |= screen;
        self.accessibility |= accessibility;
        *self != was
    }

    /// Launch reopens the assistant only for a permission given before and off now (a macOS update takes them
    /// away), never for one that was never given. Accessibility counts only while the output is Paste.
    pub fn lost(&self, screen: bool, accessibility: bool, paste: bool) -> bool {
        (self.screen && !screen) || (paste && self.accessibility && !accessibility)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Settings {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_hotkey: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub full_hotkey: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub region_hotkey: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub zoom_hotkey: Option<String>,
    #[serde(default)]
    pub talk_mode: TalkMode,
    #[serde(default = "default_model")]
    pub model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device: Option<String>,
    #[serde(default)]
    pub output: OutputMode,
    /// Also copy to clipboard when the primary output is folder or auto-paste.
    #[serde(default = "yes")]
    pub also_clipboard: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub export_dir: Option<String>,
    /// Zoom crop size in pixels (square, N×N around the pointer).
    #[serde(default = "default_zoom_px")]
    pub zoom_px: u32,
    #[serde(default)]
    pub mark_point: MarkPoint,
    #[serde(default)]
    pub zoom_target: ZoomTarget,
    /// Enlarge small close-ups 2x so small text reads.
    #[serde(default = "yes")]
    pub zoom_sharpen: bool,
    /// Outline the control under where the person pointed and name it in the prompt.
    #[serde(default = "yes")]
    pub outline_element: bool,
    #[serde(default)]
    pub spoken_anchors: SpokenAnchors,
    /// Keep the screen once a second in memory while recording and add the moments talked about.
    #[serde(default)]
    pub key_frames: bool,
    /// Opens at login unless the person turns it off; set up when onboarding finishes.
    #[serde(default = "yes")]
    pub start_at_login: bool,
    /// Ask GitHub once a day whether a newer Pointcast is out.
    #[serde(default = "yes")]
    pub check_updates: bool,
    #[serde(default)]
    pub include_app: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hidden: Option<Vec<String>>,
    /// Which never-capture defaults a saved `hidden` list has taken: 0 before the private-window titles were added, 1
    /// since.
    #[serde(default)]
    pub hidden_default: u8,
    /// MCP HTTP bind, default 127.0.0.1:47821.
    #[serde(default = "default_mcp")]
    pub mcp_bind: String,
    #[serde(default = "yes")]
    pub mcp_enabled: bool,
    /// First-run setup finished; until then the onboarding window opens at launch.
    #[serde(default)]
    pub onboarded: bool,
    /// Soft start and stop sounds.
    #[serde(default = "yes")]
    pub sounds: bool,
    #[serde(default)]
    pub granted: Granted,
    /// Keep each recording's audio in History (the newest ten), so it can be transcribed again.
    #[serde(default = "yes")]
    pub keep_audio: bool,
    /// The plain-text prompt names each picture's file, for agents in a terminal that cannot take pasted images.
    #[serde(default = "yes")]
    pub picture_paths: bool,
    /// Tidy the text when the prompt is composed: paragraphs at the pauses, no ums and no repeated words. Nothing is
    /// summarized or reworded, and History keeps the words as they were said; off joins the pieces with single spaces.
    #[serde(default = "yes")]
    pub tidy: bool,
    /// Clean up room noise and even out the volume before the voice detector and the model hear it, live and when a
    /// recording is transcribed again; the kept audio stays as it came.
    #[serde(default = "yes")]
    pub voice_boost: bool,
    /// Get the faster GPU version of Whisper Turbo by itself, in the background, where a graphics card can run it.
    #[serde(default = "yes")]
    pub auto_gpu: bool,
    /// Minutes after the speech model's last use before it is let go and its memory (about 0.9 GB of graphics memory and
    /// 1.5 GB of RAM) returned to the system. The next recording loads it again. 0 keeps it loaded.
    #[serde(default = "default_free_minutes")]
    pub free_memory_minutes: u32,
    /// Which default `free_memory_minutes` was written under: 0 before 1 minute was (every file then held 10), 1 since.
    #[serde(default)]
    pub free_default: u8,
    /// Where each recording's pictures are copied for agents, one folder per recording; none is `~/Pointcast`.
    #[serde(default)]
    pub handoff_dir: Option<String>,
    /// What the picture of a question shows: the window under the pointer or the whole screen.
    #[serde(default)]
    pub ask_target: AskTarget,
}

fn default_model() -> String {
    "auto".into()
}
fn yes() -> bool {
    true
}
fn default_zoom_px() -> u32 {
    512
}
fn default_free_minutes() -> u32 {
    1
}
fn default_mcp() -> String {
    "127.0.0.1:47821".into()
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            session_hotkey: None,
            full_hotkey: None,
            region_hotkey: None,
            zoom_hotkey: None,
            talk_mode: TalkMode::default(),
            model: default_model(),
            device: None,
            output: OutputMode::AutoPaste,
            also_clipboard: true,
            export_dir: None,
            zoom_px: default_zoom_px(),
            mark_point: MarkPoint::default(),
            zoom_target: ZoomTarget::default(),
            zoom_sharpen: true,
            outline_element: true,
            spoken_anchors: SpokenAnchors::default(),
            key_frames: false,
            start_at_login: true,
            check_updates: true,
            include_app: false,
            hidden: None,
            hidden_default: 1,
            mcp_bind: default_mcp(),
            mcp_enabled: true,
            onboarded: false,
            sounds: true,
            granted: Granted::default(),
            keep_audio: true,
            picture_paths: true,
            tidy: true,
            voice_boost: true,
            auto_gpu: true,
            free_memory_minutes: default_free_minutes(),
            free_default: 1,
            handoff_dir: None,
            ask_target: AskTarget::default(),
        }
    }
}

impl Settings {
    pub fn load(path: &Path) -> Self {
        let mut s: Self = std::fs::read(path)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        // The old default, 10, becomes the new one once; 10 picked after that is kept.
        if s.free_default < 1 {
            if s.free_memory_minutes == 10 {
                s.free_memory_minutes = default_free_minutes();
            }
            s.free_default = 1;
        }
        // A list saved before the private-window titles were defaults gains them once; one taken out after that
        // stays out, and the person's own entries are kept as they are.
        if s.hidden_default < 1 {
            if let Some(list) = s.hidden.as_mut() {
                for p in privacy::PRIVATE_WINDOWS {
                    if !list.iter().any(|h| h.trim().eq_ignore_ascii_case(p)) {
                        list.push((*p).to_string());
                    }
                }
            }
            s.hidden_default = 1;
        }
        s
    }
    pub fn save(&self, path: &Path) -> Result<(), String> {
        if let Some(d) = path.parent() {
            std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
        }
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, path).map_err(|e| e.to_string())
    }

    fn parse_ok(s: &Option<String>, def: fn() -> Chord) -> Chord {
        s.as_deref()
            .and_then(|x| Chord::parse(x).ok())
            .filter(|c| chord::refusal(c).is_none())
            .unwrap_or_else(def)
    }

    pub fn session_chord(&self) -> Chord {
        Self::parse_ok(&self.session_hotkey, chord::default_session)
    }
    pub fn full_chord(&self) -> Chord {
        Self::parse_ok(&self.full_hotkey, chord::default_full)
    }
    pub fn region_chord(&self) -> Chord {
        Self::parse_ok(&self.region_hotkey, chord::default_region)
    }
    pub fn zoom_chord(&self) -> Chord {
        Self::parse_ok(&self.zoom_hotkey, chord::default_zoom)
    }
    pub fn hidden_list(&self) -> Vec<String> {
        self.hidden
            .clone()
            .unwrap_or_else(privacy::default_hidden_list)
    }
    pub fn zoom_px(&self) -> u32 {
        self.zoom_px.clamp(128, 2048)
    }
}

/// The app identifier: the folder name Tauri's uninstaller clears when asked to delete app data.
pub const IDENTIFIER: &str = "dev.pointcast.app";

/// The home folder Pointcast uses: `POINTCAST_HOME` when set and not empty, else the system's. On Windows the system's
/// comes from the profile known folder and ignores USERPROFILE and HOME, so this is the only way to point it elsewhere.
pub fn home_dir() -> Option<PathBuf> {
    home_override(std::env::var_os("POINTCAST_HOME")).or_else(dirs::home_dir)
}

/// `POINTCAST_HOME` is set (not empty), so the home folder is not the real one.
pub fn home_is_custom() -> bool {
    home_override(std::env::var_os("POINTCAST_HOME")).is_some()
}

/// The override from a `POINTCAST_HOME` value: none when it is missing or empty.
pub fn home_override(value: Option<std::ffi::OsString>) -> Option<PathBuf> {
    value.filter(|v| !v.is_empty()).map(PathBuf::from)
}

pub fn data_dir() -> PathBuf {
    if let Some(d) = std::env::var_os("POINTCAST_DATA_DIR") {
        return d.into();
    }
    // Tests never read, write or migrate the real app's data.
    if cfg!(test) {
        return std::env::temp_dir().join(format!("pointcast-test-data-{}", std::process::id()));
    }
    let base = dirs::data_local_dir().unwrap_or_else(|| PathBuf::from("."));
    let dir = base.join(IDENTIFIER);
    static READY: std::sync::Once = std::sync::Once::new();
    READY.call_once(|| {
        private(&dir);
        migrate(&base.join("Pointcast"), &dir);
    });
    dir
}

/// History holds screenshots and transcripts: on macOS and Linux only this user may open the data folder, one an
/// earlier version made included.
#[cfg_attr(not(unix), allow(unused_variables))]
fn private(dir: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
        let _ = std::fs::DirBuilder::new().recursive(true).mode(0o700).create(dir);
        let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
    }
}

/// Before 0.4 the data sat in `<local data>/Pointcast`, which on Windows is also where the
/// per-user installer puts the app itself. Move our entries out once; leave anything else.
pub fn migrate(old: &Path, new: &Path) {
    for name in ["settings.json", "speech", "history", "recovery"] {
        let (from, to) = (old.join(name), new.join(name));
        if from.exists() && !to.exists() && std::fs::create_dir_all(new).is_ok() {
            let _ = std::fs::rename(&from, &to);
        }
    }
}

pub fn settings_path() -> PathBuf {
    data_dir().join("settings.json")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn home_override_is_honoured_and_an_empty_value_is_ignored() {
        let scratch = std::env::temp_dir().join("pc-scratch-home");
        assert_eq!(home_override(Some(scratch.clone().into_os_string())), Some(scratch));
        assert_eq!(home_override(Some(std::ffi::OsString::new())), None);
        assert_eq!(home_override(None), None);
    }

    #[test]
    fn settings_round_trip_and_refused_chords_fall_back() {
        let dir = std::env::temp_dir().join(format!("pc-settings-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("settings.json");
        let s = Settings::load(&path);
        assert_eq!(s.session_chord(), chord::default_session());
        assert!(s.hidden_list().iter().any(|h| h == "1Password"));
        let mut mine = Settings::default();
        mine.session_hotkey = Some("Ctrl+Shift+F8".into());
        mine.zoom_px = 640;
        mine.output = OutputMode::Folder;
        mine.save(&path).unwrap();
        let back = Settings::load(&path);
        assert_eq!(back.session_chord().label(), "Ctrl+Shift+F8");
        assert_eq!(back.zoom_px(), 640);
        assert_eq!(back.output, OutputMode::Folder);
        assert!(!back.onboarded && back.sounds);
        std::fs::write(&path, r#"{"onboarded":true,"sounds":false}"#).unwrap();
        let old = Settings::load(&path);
        assert!(old.onboarded && !old.sounds);
        std::fs::write(&path, r#"{"session_hotkey":"A"}"#).unwrap();
        assert_eq!(Settings::load(&path).session_chord(), chord::default_session());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn capture_marks_round_trip_and_an_old_file_gets_the_defaults() {
        let dir = std::env::temp_dir().join(format!("pc-marks-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("settings.json");
        let s = Settings::default();
        assert_eq!((s.mark_point, s.zoom_target, s.zoom_sharpen), (MarkPoint::Ring, ZoomTarget::Dwell, true));
        let mine = Settings { mark_point: MarkPoint::Trail, zoom_target: ZoomTarget::Window, zoom_sharpen: false, ..s };
        mine.save(&path).unwrap();
        let raw = std::fs::read_to_string(&path).unwrap();
        assert!(raw.contains(r#""mark_point": "trail""#) && raw.contains(r#""zoom_target": "window""#), "{raw}");
        assert_eq!(Settings::load(&path), mine);
        // A file from before these settings existed keeps what it has and gets the defaults.
        std::fs::write(&path, r#"{"zoom_px":640,"onboarded":true}"#).unwrap();
        let old = Settings::load(&path);
        assert!(old.onboarded && old.zoom_px == 640);
        assert_eq!((old.mark_point, old.zoom_target, old.zoom_sharpen), (MarkPoint::Ring, ZoomTarget::Dwell, true));
        assert!(old.outline_element, "outlines are on for a file from before they existed");
        assert_eq!(old.spoken_anchors, SpokenAnchors::Ring, "and so are spoken anchors");
        std::fs::write(&path, r#"{"mark_point":"off","zoom_target":"pointer"}"#).unwrap();
        let off = Settings::load(&path);
        assert_eq!((off.mark_point, off.zoom_target), (MarkPoint::Off, ZoomTarget::Pointer));
        let quiet = Settings { outline_element: false, ..Settings::default() };
        quiet.save(&path).unwrap();
        assert!(std::fs::read_to_string(&path).unwrap().contains(r#""outline_element": false"#));
        assert_eq!(Settings::load(&path), quiet);
        let mute = Settings { spoken_anchors: SpokenAnchors::Off, ..Settings::default() };
        mute.save(&path).unwrap();
        assert!(std::fs::read_to_string(&path).unwrap().contains(r#""spoken_anchors": "off""#));
        assert_eq!(Settings::load(&path), mute);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn key_frames_are_off_until_turned_on_and_round_trip() {
        let dir = std::env::temp_dir().join(format!("pc-keyframes-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("settings.json");
        assert!(!Settings::default().key_frames);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(&path, r#"{"zoom_px":640,"onboarded":true}"#).unwrap();
        let old = Settings::load(&path);
        assert!(old.onboarded && !old.key_frames, "a file from before key frames keeps them off");
        let on = Settings { key_frames: true, ..old };
        on.save(&path).unwrap();
        assert!(std::fs::read_to_string(&path).unwrap().contains(r#""key_frames": true"#));
        assert_eq!(Settings::load(&path), on);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_new_install_opens_at_login_and_checks_for_updates() {
        let s = Settings::default();
        assert!(s.start_at_login && s.check_updates);
        // A settings file from before these fields existed gets the same.
        let old: Settings = serde_json::from_str("{}").unwrap();
        assert!(old.start_at_login && old.check_updates);
        // A choice already saved is kept.
        let off: Settings = serde_json::from_str(r#"{"start_at_login":false,"check_updates":false}"#).unwrap();
        assert!(!off.start_at_login && !off.check_updates);
    }

    #[test]
    fn the_assistant_reopens_only_for_a_permission_that_was_given_and_is_gone() {
        let old: Settings = serde_json::from_str(r#"{"onboarded":true}"#).unwrap();
        assert_eq!(old.granted, Granted::default(), "a file from before this remembers nothing granted");
        let mut g = Granted::default();
        // A dictation-only Mac that never allowed Screen Recording is not asked at every login.
        assert!(!g.lost(false, false, true));
        assert!(g.note(true, false) && !g.note(true, false), "noted once");
        assert!(g.lost(false, false, false), "Screen Recording was given and an update took it");
        assert!(!g.lost(true, false, true), "Accessibility was never given");
        g.note(true, true);
        assert!(g.lost(true, false, true), "Accessibility was given, and Paste needs it");
        assert!(!g.lost(true, false, false), "with Copy or Save it does not matter");
        let s = Settings { granted: g, ..Settings::default() };
        let back: Settings = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!(back.granted, g);
    }

    #[test]
    fn the_speech_model_is_freed_a_minute_after_its_last_use_unless_told_otherwise() {
        let d = std::env::temp_dir().join(format!("pc-settings-free-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let file = d.join("settings.json");
        let load = |json: &str| {
            std::fs::write(&file, json).unwrap();
            Settings::load(&file)
        };
        assert_eq!(Settings::default().free_memory_minutes, 1);
        assert_eq!(load(r#"{"onboarded":true}"#).free_memory_minutes, 1, "a file from before the setting");
        // Every file written before held the old default, 10: it becomes the new one, once.
        let old = load(r#"{"onboarded":true,"free_memory_minutes":10}"#);
        assert_eq!(old.free_memory_minutes, 1, "the old default moves to the new one");
        old.save(&file).unwrap();
        assert_eq!(Settings::load(&file).free_memory_minutes, 1);
        // 10 picked after that is the person's choice: it stays through a save and a load.
        let chosen = Settings { free_memory_minutes: 10, ..Settings::load(&file) };
        chosen.save(&file).unwrap();
        assert_eq!(Settings::load(&file).free_memory_minutes, 10, "an explicit 10 survives");
        assert_eq!(Settings::load(&file).free_memory_minutes, 10, "and every later load");
        let fresh = Settings { free_memory_minutes: 10, ..Settings::default() };
        fresh.save(&file).unwrap();
        assert_eq!(Settings::load(&file).free_memory_minutes, 10, "on a file made with the new default too");
        // Any other stored choice is kept as it was.
        for m in [0, 2, 5, 30] {
            assert_eq!(load(&format!(r#"{{"free_memory_minutes":{m}}}"#)).free_memory_minutes, m);
        }
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_saved_never_capture_list_gains_the_private_window_titles_once_and_keeps_its_own_edits() {
        let d = std::env::temp_dir().join(format!("pc-settings-hidden-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let file = d.join("settings.json");
        let load = |json: &str| {
            std::fs::write(&file, json).unwrap();
            Settings::load(&file)
        };
        let private = |s: &Settings| privacy::PRIVATE_WINDOWS.iter().filter(|p| s.hidden_list().iter().any(|h| h == *p)).count();
        // No list saved: the defaults, which hold them.
        assert_eq!(private(&Settings::default()), 3);
        let unset = load(r#"{"onboarded":true}"#);
        assert_eq!((unset.hidden.clone(), private(&unset)), (None, 3));
        // A list saved before they were defaults, edited by the person: it gains them after its own entries.
        let old = load(r#"{"hidden":["1Password","My Bank","incognito"]}"#);
        assert_eq!(old.hidden_list(), ["1Password", "My Bank", "incognito", "InPrivate", "Private Browsing"], "one already there is not added twice");
        old.save(&file).unwrap();
        assert_eq!(Settings::load(&file).hidden_list(), old.hidden_list(), "once");
        // Taken out after that, they stay out: the list is the person's.
        let edited = Settings { hidden: Some(vec!["My Bank".into()]), ..Settings::load(&file) };
        edited.save(&file).unwrap();
        assert_eq!(Settings::load(&file).hidden_list(), ["My Bank"]);
        assert_eq!(Settings::load(&file).hidden_list(), ["My Bank"], "and on every later load");
        // A list emptied before still gains the private windows, once; one emptied since stays empty.
        assert_eq!(load(r#"{"hidden":[]}"#).hidden_list(), privacy::PRIVATE_WINDOWS);
        let fresh = Settings { hidden: Some(vec![]), ..Settings::default() };
        fresh.save(&file).unwrap();
        assert!(Settings::load(&file).hidden_list().is_empty(), "a list emptied on a new install stays empty");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn audio_is_kept_and_pictures_are_named_by_their_files_unless_turned_off() {
        let old: Settings = serde_json::from_str(r#"{"onboarded":true}"#).unwrap();
        assert!(old.keep_audio && old.picture_paths, "a file from before these gets them on");
        assert!(Settings::default().keep_audio && Settings::default().picture_paths);
        let off: Settings = serde_json::from_str(r#"{"keep_audio":false,"picture_paths":false}"#).unwrap();
        assert!(!off.keep_audio && !off.picture_paths);
        assert!(old.tidy && Settings::default().tidy, "the text is tidied unless turned off");
        assert!(!serde_json::from_str::<Settings>(r#"{"tidy":false}"#).unwrap().tidy);
        assert!(old.auto_gpu && Settings::default().auto_gpu, "the faster engine sets itself up unless turned off");
        assert!(!serde_json::from_str::<Settings>(r#"{"auto_gpu":false}"#).unwrap().auto_gpu);
        let back: Settings = serde_json::from_str(&serde_json::to_string(&off).unwrap()).unwrap();
        assert_eq!(back, off);
    }

    #[test]
    fn quieter_and_farther_speech_is_picked_up_unless_turned_off() {
        let old: Settings = serde_json::from_str(r#"{"onboarded":true}"#).unwrap();
        assert!(old.voice_boost && Settings::default().voice_boost, "on for a file from before it, and for a new one");
        let off: Settings = serde_json::from_str(r#"{"voice_boost":false}"#).unwrap();
        assert!(!off.voice_boost);
        let back: Settings = serde_json::from_str(&serde_json::to_string(&off).unwrap()).unwrap();
        assert_eq!(back, off);
    }

    #[test]
    fn tests_never_use_the_real_data_folder() {
        if std::env::var_os("POINTCAST_DATA_DIR").is_none() {
            assert!(data_dir().starts_with(std::env::temp_dir()), "{}", data_dir().display());
        }
    }

    #[cfg(unix)]
    #[test]
    fn only_this_user_can_open_the_data_folder() {
        use std::os::unix::fs::PermissionsExt;
        let root = std::env::temp_dir().join(format!("pc-private-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let dir = root.join(IDENTIFIER);
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        private(&dir);
        assert_eq!(mode(&dir), 0o700);
        // One an earlier version made, open to everyone, is closed too.
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        private(&dir);
        assert_eq!(mode(&dir), 0o700);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn old_data_moves_out_of_the_install_folder_and_the_app_stays() {
        let root = std::env::temp_dir().join(format!("pc-migrate-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (old, new) = (root.join("Pointcast"), root.join(IDENTIFIER));
        std::fs::create_dir_all(old.join("speech").join("runtime")).unwrap();
        std::fs::create_dir_all(old.join("history")).unwrap();
        std::fs::write(old.join("settings.json"), "{}").unwrap();
        std::fs::write(old.join("pointcast.exe"), "app").unwrap();
        migrate(&old, &new);
        assert!(new.join("settings.json").is_file() && new.join("speech/runtime").is_dir() && new.join("history").is_dir());
        assert!(old.join("pointcast.exe").is_file() && !old.join("settings.json").exists());
        // Already moved: a second run changes nothing, and newer data is never overwritten.
        std::fs::write(old.join("settings.json"), "stale").unwrap();
        migrate(&old, &new);
        assert_eq!(std::fs::read_to_string(new.join("settings.json")).unwrap(), "{}");
        let _ = std::fs::remove_dir_all(&root);
    }
}
