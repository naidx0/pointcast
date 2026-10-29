//! Updates without the install command: ask GitHub for the latest release and, when the person
//! says so, run the same installer the one-line install runs. It closes Pointcast, installs the
//! new version and opens it again.
use serde::Deserialize;
use std::sync::Mutex;
use std::time::Duration;

const LATEST: &str = "https://api.github.com/repos/naidx0/pointcast/releases/latest";

/// The newer version the last check found, if any.
static AVAILABLE: Mutex<Option<String>> = Mutex::new(None);

pub fn available() -> Option<String> {
    AVAILABLE.lock().unwrap().clone()
}

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
    #[serde(default)]
    assets: Vec<Asset>,
}

#[derive(Deserialize)]
struct Asset {
    name: String,
}

/// The file the installer downloads on this system.
fn installs_on(os: &str, asset: &str) -> bool {
    match os {
        "windows" => asset.ends_with("x64-setup.exe"),
        "macos" => asset.ends_with(".dmg"),
        _ => false,
    }
}

/// "v1.2.3" or "1.2" as numbers; a pre-release suffix is ignored.
fn version(v: &str) -> Option<(u64, u64, u64)> {
    let core = v.trim().trim_start_matches('v').split(['-', '+']).next()?;
    let mut n = core.split('.').map(|p| p.parse::<u64>());
    let major = n.next()?.ok()?;
    let minor = n.next().unwrap_or(Ok(0)).ok()?;
    let patch = n.next().unwrap_or(Ok(0)).ok()?;
    Some((major, minor, patch))
}

/// The version to offer: newer than `current`, published, and with a download for `os`.
fn offer(body: &str, current: &str, os: &str) -> Result<Option<String>, String> {
    let r: Release = serde_json::from_str(body).map_err(|_| "GitHub sent an answer Pointcast can't read.".to_string())?;
    let (Some(new), Some(cur)) = (version(&r.tag_name), version(current)) else {
        return Ok(None);
    };
    let ready = !r.draft && !r.prerelease && r.assets.iter().any(|a| installs_on(os, &a.name));
    Ok((ready && new > cur).then(|| r.tag_name.trim_start_matches('v').to_string()))
}

/// Ask GitHub once. `Ok(None)` means up to date, or nothing published for this system yet.
pub fn check() -> Result<Option<String>, String> {
    let found = check_at(LATEST, env!("CARGO_PKG_VERSION"), std::env::consts::OS)?;
    *AVAILABLE.lock().unwrap() = found.clone();
    Ok(found)
}

fn check_at(url: &str, current: &str, os: &str) -> Result<Option<String>, String> {
    let client = reqwest::blocking::Client::builder()
        .user_agent(concat!("Pointcast/", env!("CARGO_PKG_VERSION")))
        .timeout(Duration::from_secs(20))
        .build()
        .map_err(|e| e.to_string())?;
    let resp = client
        .get(url)
        .header("Accept", "application/vnd.github+json")
        .send()
        .map_err(|_| "Can't reach GitHub. Check your connection and try again.".to_string())?;
    match resp.status().as_u16() {
        200 => offer(&resp.text().map_err(|e| e.to_string())?, current, os),
        404 => Ok(None),
        403 | 429 => Err("GitHub is limiting requests from your network. Try again later.".into()),
        code => Err(format!("GitHub answered {code}. Try again later.")),
    }
}

/// Start the installer in the background. Its output goes to `update.log` in the data folder.
pub fn install() -> Result<(), String> {
    let log = crate::settings::data_dir().join("update.log");
    let out = std::fs::File::create(&log).map_err(|e| e.to_string())?;
    let err = out.try_clone().map_err(|e| e.to_string())?;
    let mut cmd = installer()?;
    cmd.stdin(std::process::Stdio::null()).stdout(out).stderr(err);
    cmd.spawn().map(|_| ()).map_err(|e| format!("The installer did not start: {e}"))
}

#[cfg(windows)]
fn installer() -> Result<std::process::Command, String> {
    use std::os::windows::process::CommandExt;
    const SCRIPT: &str = "https://raw.githubusercontent.com/naidx0/pointcast/main/install.ps1";
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    let mut cmd = std::process::Command::new("powershell.exe");
    cmd.args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-Command"])
        .arg(format!("irm {SCRIPT} | iex"))
        .creation_flags(CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP);
    Ok(cmd)
}

#[cfg(target_os = "macos")]
fn installer() -> Result<std::process::Command, String> {
    const SCRIPT: &str = "https://raw.githubusercontent.com/naidx0/pointcast/main/install.sh";
    let mut cmd = std::process::Command::new("/bin/bash");
    cmd.arg("-c").arg(format!("curl -fsSL {SCRIPT} | bash"));
    Ok(cmd)
}

#[cfg(not(any(windows, target_os = "macos")))]
fn installer() -> Result<std::process::Command, String> {
    Err("On Linux, update Pointcast from the releases page.".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};

    fn release(tag: &str, assets: &[&str]) -> String {
        let assets: Vec<_> = assets.iter().map(|n| serde_json::json!({ "name": n })).collect();
        serde_json::json!({ "tag_name": tag, "draft": false, "prerelease": false, "assets": assets }).to_string()
    }

    const BOTH: [&str; 2] = ["Pointcast_0.5.0_x64-setup.exe", "Pointcast_0.5.0_universal.dmg"];

    #[test]
    fn a_newer_release_with_a_download_for_this_system_is_offered() {
        for os in ["windows", "macos"] {
            assert_eq!(offer(&release("v0.5.0", &BOTH), "0.4.0", os).unwrap().as_deref(), Some("0.5.0"));
        }
        assert_eq!(offer(&release("v0.4.1", &BOTH), "0.4.0", "windows").unwrap().as_deref(), Some("0.4.1"));
        assert_eq!(offer(&release("v1.0", &BOTH), "0.9.12", "macos").unwrap().as_deref(), Some("1.0"));
    }

    #[test]
    fn the_same_or_an_older_release_is_not_offered() {
        for tag in ["v0.4.0", "0.4.0", "v0.3.9", "v0.4.0-beta.2"] {
            assert_eq!(offer(&release(tag, &BOTH), "0.4.0", "windows").unwrap(), None, "{tag}");
        }
        assert_eq!(offer(&release("v0.10.0", &BOTH), "0.9.0", "windows").unwrap().as_deref(), Some("0.10.0"));
    }

    #[test]
    fn a_release_without_this_systems_download_is_not_offered() {
        // The Windows build goes up first and the Mac one later: a Mac must wait for its .dmg.
        let win_only = release("v0.5.0", &["Pointcast_0.5.0_x64-setup.exe"]);
        assert_eq!(offer(&win_only, "0.4.0", "macos").unwrap(), None);
        assert_eq!(offer(&win_only, "0.4.0", "windows").unwrap().as_deref(), Some("0.5.0"));
        assert_eq!(offer(&release("v0.5.0", &BOTH), "0.4.0", "linux").unwrap(), None);
    }

    #[test]
    fn drafts_prereleases_and_odd_tags_are_not_offered() {
        let mut v: serde_json::Value = serde_json::from_str(&release("v0.5.0", &BOTH)).unwrap();
        v["prerelease"] = true.into();
        assert_eq!(offer(&v.to_string(), "0.4.0", "windows").unwrap(), None);
        v["prerelease"] = false.into();
        v["draft"] = true.into();
        assert_eq!(offer(&v.to_string(), "0.4.0", "windows").unwrap(), None);
        assert_eq!(offer(&release("nightly", &BOTH), "0.4.0", "windows").unwrap(), None);
        assert!(offer("<html>rate limited</html>", "0.4.0", "windows").is_err());
    }

    /// Answer one request with `status` and `body` on a loopback port.
    fn serve(status: u16, body: String) -> String {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/latest", l.local_addr().unwrap());
        std::thread::spawn(move || {
            let (mut s, _) = l.accept().unwrap();
            let mut buf = [0u8; 4096];
            let _ = s.read(&mut buf);
            let _ = write!(s, "HTTP/1.1 {status} X\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
        });
        url
    }

    #[test]
    fn the_check_reads_github_over_the_wire() {
        let url = serve(200, release("v0.5.0", &BOTH));
        assert_eq!(check_at(&url, "0.4.0", "windows").unwrap().as_deref(), Some("0.5.0"));
        // A repo with nothing published answers 404: up to date, not an error.
        assert_eq!(check_at(&serve(404, "{}".into()), "0.4.0", "windows").unwrap(), None);
        let limited = check_at(&serve(403, "{}".into()), "0.4.0", "windows").unwrap_err();
        assert!(limited.contains("limiting"), "{limited}");
    }
}
