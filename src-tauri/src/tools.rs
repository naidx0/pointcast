//! Detect and open companion apps (SuperWhisper, ChatGPT, Claude, Cursor).
use serde::Serialize;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize)]
pub struct DetectedTool {
    pub id: String,
    pub name: String,
    pub path: String,
    pub kind: String,
}

fn exists(p: &Path) -> bool {
    p.exists()
}

fn mac_apps() -> Vec<DetectedTool> {
    let candidates: &[(&str, &str, &str)] = &[
        ("superwhisper", "SuperWhisper", "stt"),
        ("chatgpt", "ChatGPT", "ai"),
        ("claude", "Claude", "ai"),
        ("cursor", "Cursor", "ai"),
    ];
    let names: &[(&str, &[&str])] = &[
        ("superwhisper", &["Superwhisper.app", "SuperWhisper.app"]),
        ("chatgpt", &["ChatGPT.app"]),
        ("claude", &["Claude.app"]),
        ("cursor", &["Cursor.app"]),
    ];
    let mut out = Vec::new();
    for (id, name, kind) in candidates {
        let Some((_, apps)) = names.iter().find(|(k, _)| *k == *id) else {
            continue;
        };
        for app in *apps {
            let p = PathBuf::from("/Applications").join(app);
            if exists(&p) {
                out.push(DetectedTool {
                    id: (*id).into(),
                    name: (*name).into(),
                    path: p.to_string_lossy().into(),
                    kind: (*kind).into(),
                });
                break;
            }
        }
    }
    out
}

fn win_apps() -> Vec<DetectedTool> {
    let mut out = Vec::new();
    let local = std::env::var_os("LOCALAPPDATA").map(PathBuf::from);
    let prog = std::env::var_os("PROGRAMFILES").map(PathBuf::from);
    let candidates: Vec<(&str, &str, &str, PathBuf)> = [
        (
            "superwhisper",
            "SuperWhisper",
            "stt",
            local
                .clone()
                .unwrap_or_default()
                .join("Programs")
                .join("Superwhisper")
                .join("Superwhisper.exe"),
        ),
        (
            "chatgpt",
            "ChatGPT",
            "ai",
            local
                .clone()
                .unwrap_or_default()
                .join("Programs")
                .join("ChatGPT")
                .join("ChatGPT.exe"),
        ),
        (
            "claude",
            "Claude",
            "ai",
            local
                .clone()
                .unwrap_or_default()
                .join("AnthropicClaude")
                .join("claude.exe"),
        ),
        (
            "cursor",
            "Cursor",
            "ai",
            local
                .unwrap_or_default()
                .join("Programs")
                .join("cursor")
                .join("Cursor.exe"),
        ),
        (
            "cursor",
            "Cursor",
            "ai",
            prog.unwrap_or_default().join("Cursor").join("Cursor.exe"),
        ),
    ]
    .into_iter()
    .collect();
    let mut seen = std::collections::HashSet::new();
    for (id, name, kind, path) in candidates {
        if seen.contains(id) {
            continue;
        }
        if exists(&path) {
            seen.insert(id);
            out.push(DetectedTool {
                id: id.into(),
                name: name.into(),
                path: path.to_string_lossy().into(),
                kind: kind.into(),
            });
        }
    }
    out
}

fn linux_apps() -> Vec<DetectedTool> {
    // Best-effort desktop apps via which / flatpak names.
    let bins: &[(&str, &str, &str, &str)] = &[
        ("cursor", "Cursor", "ai", "cursor"),
        ("chatgpt", "ChatGPT", "ai", "chatgpt"),
        ("claude", "Claude", "ai", "claude"),
    ];
    let mut out = Vec::new();
    for (id, name, kind, bin) in bins {
        if let Ok(outp) = std::process::Command::new("which").arg(bin).output() {
            if outp.status.success() {
                let path = String::from_utf8_lossy(&outp.stdout).trim().to_string();
                if !path.is_empty() {
                    out.push(DetectedTool {
                        id: (*id).into(),
                        name: (*name).into(),
                        path,
                        kind: (*kind).into(),
                    });
                }
            }
        }
    }
    out
}

pub fn detect() -> Vec<DetectedTool> {
    if cfg!(target_os = "macos") {
        mac_apps()
    } else if cfg!(windows) {
        win_apps()
    } else {
        linux_apps()
    }
}

pub fn open(id: &str) -> Result<(), String> {
    let tools = detect();
    let t = tools
        .iter()
        .find(|t| t.id == id)
        .ok_or_else(|| format!("{id} was not found on this machine"))?;
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(&t.path)
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("cmd")
            .args(["/C", "start", "", &t.path])
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        std::process::Command::new(&t.path)
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_returns_a_list() {
        let _ = detect();
    }
}
