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
    pub start_at_login: bool,
    #[serde(default)]
    pub include_app: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hidden: Option<Vec<String>>,
    /// MCP HTTP bind, default 127.0.0.1:47821.
    #[serde(default = "default_mcp")]
    pub mcp_bind: String,
    #[serde(default = "yes")]
    pub mcp_enabled: bool,
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
            output: OutputMode::Clipboard,
            also_clipboard: true,
            export_dir: None,
            zoom_px: default_zoom_px(),
            start_at_login: false,
            include_app: false,
            hidden: None,
            mcp_bind: default_mcp(),
            mcp_enabled: true,
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

pub fn data_dir() -> PathBuf {
    if let Some(d) = std::env::var_os("POINTCAST_DATA_DIR") {
        return d.into();
    }
    dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("Pointcast")
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
        std::fs::write(&path, r#"{"session_hotkey":"A"}"#).unwrap();
        assert_eq!(Settings::load(&path).session_chord(), chord::default_session());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
