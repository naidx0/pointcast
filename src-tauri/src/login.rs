//! Open at login, for this user only: a Run entry under the current user on Windows (never the
//! machine-wide one, which Pointcast without admin rights could not turn off again) and a
//! LaunchAgent on macOS. The entry starts Pointcast with `--login`, so a second start at login
//! (macOS session restore) is not taken for someone opening the app.
use auto_launch::{AutoLaunch, AutoLaunchBuilder};
use serde::Serialize;
use tauri::AppHandle;

pub const ARG: &str = "--login";

#[derive(Debug, Clone, Serialize)]
pub struct Login {
    pub enabled: bool,
    /// A machine-wide entry for this app, left by an earlier version run as administrator.
    pub machine: bool,
}

/// Why this copy cannot open at login: the login item would point at a disk image or at the
/// read-only copy macOS runs an app from before it is moved, which are gone after a restart.
pub fn refusal(exe: &str) -> Option<&'static str> {
    refusal_on(exe, cfg!(target_os = "macos"))
}

fn refusal_on(exe: &str, mac: bool) -> Option<&'static str> {
    if exe.starts_with("/Volumes/") || exe.contains("/AppTranslocation/") {
        return Some("Move Pointcast to Applications first, then turn this on.");
    }
    // The LaunchAgent is a plist written without escaping, which these would break.
    (mac && exe.contains(['&', '<', '>'])).then_some("Move Pointcast to a folder without & < > in its path, then turn this on.")
}

/// Started by the login entry.
pub fn at_login(args: &[String]) -> bool {
    args.iter().any(|a| a == ARG)
}

fn exe() -> Result<String, String> {
    let p = std::env::current_exe().map_err(|e| e.to_string())?;
    #[cfg(target_os = "macos")]
    let p = p.canonicalize().map_err(|e| e.to_string())?;
    Ok(p.to_string_lossy().into_owned())
}

fn entry(app: &AppHandle, exe: &str) -> Result<AutoLaunch, String> {
    let mut b = AutoLaunchBuilder::new();
    b.set_app_name(&app.package_info().name).set_args(&[ARG]);
    #[cfg(windows)]
    {
        // The Run entry is one command line: quoted, a path with spaces still starts the app.
        b.set_app_path(&format!("{q}{exe}{q}", q = '"'));
        b.set_windows_enable_mode(auto_launch::WindowsEnableMode::CurrentUser);
    }
    #[cfg(not(windows))]
    b.set_app_path(exe);
    #[cfg(target_os = "macos")]
    b.set_macos_launch_mode(auto_launch::MacOSLaunchMode::LaunchAgent);
    b.build().map_err(|e| e.to_string())
}

pub fn get(app: &AppHandle) -> Result<Login, String> {
    let exe = exe()?;
    let enabled = entry(app, &exe)?.is_enabled().map_err(|e| e.to_string())?;
    Ok(Login { enabled, machine: machine_wide(&app.package_info().name, &exe) })
}

pub fn set(app: &AppHandle, on: bool) -> Result<Login, String> {
    let exe = exe()?;
    if let Some(why) = refusal(&exe).filter(|_| on) {
        return Err(why.into());
    }
    let e = entry(app, &exe)?;
    // Off first: that also removes an older entry, machine-wide too where this copy may.
    e.disable().map_err(|e| e.to_string())?;
    if on {
        e.enable().map_err(|e| e.to_string())?;
    }
    get(app)
}

#[cfg(windows)]
fn machine_wide(name: &str, exe: &str) -> bool {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let key = ["HKLM", "SOFTWARE", "Microsoft", "Windows", "CurrentVersion", "Run"].join(std::path::MAIN_SEPARATOR_STR);
    std::process::Command::new("reg")
        .args(["query", &key, "/v", name])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .is_some_and(|o| String::from_utf8_lossy(&o.stdout).to_lowercase().contains(&exe.to_lowercase()))
}

#[cfg(not(windows))]
fn machine_wide(_: &str, _: &str) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_copy_on_a_disk_image_or_translocated_cannot_open_at_login() {
        assert!(refusal("/Volumes/Pointcast/Pointcast.app/Contents/MacOS/pointcast").is_some());
        let moved = "/private/var/folders/x/T/AppTranslocation/1A2B/d/Pointcast.app/Contents/MacOS/pointcast";
        assert!(refusal(moved).is_some());
        assert!(refusal("/Applications/Pointcast.app/Contents/MacOS/pointcast").is_none());
        assert!(refusal("/Users/max/Applications/Pointcast.app/Contents/MacOS/pointcast").is_none());
        assert!(refusal("C:/Users/max/AppData/Local/Pointcast/pointcast.exe").is_none());
    }

    #[test]
    fn a_path_the_launch_agent_cannot_hold_cannot_open_at_login() {
        let odd = "/Users/max/Apps & Tools/Pointcast.app/Contents/MacOS/pointcast";
        assert!(refusal_on(odd, true).unwrap().contains("& < >"));
        assert!(refusal_on("/Users/max/<new>/Pointcast.app/Contents/MacOS/pointcast", true).is_some());
        assert!(refusal_on("/Users/max/a>b/Pointcast.app/Contents/MacOS/pointcast", true).is_some());
        assert!(refusal_on("/Applications/Pointcast.app/Contents/MacOS/pointcast", true).is_none());
        // Windows quotes its Run entry, where & is only a character.
        assert!(refusal_on("C:/Users/Tom & Jerry/AppData/Local/Pointcast/pointcast.exe", false).is_none());
    }

    #[test]
    fn only_a_start_with_the_login_flag_counts_as_one_from_login() {
        let args = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert!(at_login(&args(&["/Applications/Pointcast.app/Contents/MacOS/pointcast", "--login"])));
        assert!(!at_login(&args(&["/Applications/Pointcast.app/Contents/MacOS/pointcast"])));
        assert!(!at_login(&args(&["pointcast.exe", "--login-later"])));
    }
}
