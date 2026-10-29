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
            mcp_bind: default_mcp(),
            mcp_enabled: true,
            onboarded: false,
            sounds: true,
        }
    }
}

impl Settings {
    pub fn load(path: &Path) -> Self {
        std::fs::read(path)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
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
    static MOVED: std::sync::Once = std::sync::Once::new();
    MOVED.call_once(|| migrate(&base.join("Pointcast"), &dir));
    dir
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
    fn tests_never_use_the_real_data_folder() {
        if std::env::var_os("POINTCAST_DATA_DIR").is_none() {
            assert!(data_dir().starts_with(std::env::temp_dir()), "{}", data_dir().display());
        }
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
