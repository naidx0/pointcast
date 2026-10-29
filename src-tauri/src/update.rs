//! Updates without the install command: ask GitHub for the latest release and, when the person
//! says so, download it, check it against the SHA-256 GitHub publishes for it, and run the
//! installer script built into this copy (never one fetched from the network). It closes
//! Pointcast, installs the new version and opens it again.
use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

const LATEST: &str = "https://api.github.com/repos/naidx0/pointcast/releases/latest";
/// In the data folder: the versions an update was going from and to, written before it starts.
const RECORD: &str = "update.json";
const LOG: &str = "update.log";
const DAMAGED: &str = "The download arrived damaged, so it was thrown away. Try again.";

/// A newer release for this system: its version, its download and that download's SHA-256.
#[derive(Clone, Debug, PartialEq)]
struct Offer {
    version: String,
    url: String,
    sha256: String,
}

/// The newer release the last check found, if any.
static AVAILABLE: Mutex<Option<Offer>> = Mutex::new(None);
static RUNNING: AtomicBool = AtomicBool::new(false);

pub fn available() -> Option<String> {
    AVAILABLE.lock().unwrap().as_ref().map(|o| o.version.clone())
}

/// An update is downloading or installing now.
pub fn installing() -> bool {
    RUNNING.load(Ordering::SeqCst)
}

/// Why the last update did not finish, if it failed while Pointcast was closed. The first call
/// reads and clears the record; later calls give the same answer.
pub fn last_failure() -> Option<String> {
    static LAST: OnceLock<Option<String>> = OnceLock::new();
    LAST.get_or_init(|| failure(&crate::settings::data_dir(), env!("CARGO_PKG_VERSION"))).clone()
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
    #[serde(default)]
    browser_download_url: String,
    /// "sha256:<hex>", or null on files GitHub has not hashed.
    #[serde(default)]
    digest: Option<String>,
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

/// The hex of a "sha256:<hex>" digest.
fn sha256_of(digest: &str) -> Option<String> {
    let hex = digest.strip_prefix("sha256:")?;
    (hex.len() == 64 && hex.bytes().all(|b| b.is_ascii_hexdigit())).then(|| hex.to_ascii_lowercase())
}

/// The release to offer: newer than `current`, published, and with a download for `os` whose
/// SHA-256 GitHub publishes.
fn offer(body: &str, current: &str, os: &str) -> Result<Option<Offer>, String> {
    let r: Release = serde_json::from_str(body).map_err(|_| "GitHub sent an answer Pointcast can't read.".to_string())?;
    let (Some(new), Some(cur)) = (version(&r.tag_name), version(current)) else {
        return Ok(None);
    };
    if r.draft || r.prerelease || new <= cur {
        return Ok(None);
    }
    Ok(r.assets
        .iter()
        .filter(|a| installs_on(os, &a.name) && !a.browser_download_url.is_empty())
        .find_map(|a| {
            Some(Offer {
                version: r.tag_name.trim_start_matches('v').to_string(),
                url: a.browser_download_url.clone(),
                sha256: sha256_of(a.digest.as_deref()?)?,
            })
        }))
}

/// Ask GitHub once. `Ok(None)` means up to date, or nothing published for this system yet.
pub fn check() -> Result<Option<String>, String> {
    let found = check_at(LATEST, env!("CARGO_PKG_VERSION"), std::env::consts::OS)?;
    let version = found.as_ref().map(|o| o.version.clone());
    *AVAILABLE.lock().unwrap() = found;
    Ok(version)
}

fn check_at(url: &str, current: &str, os: &str) -> Result<Option<Offer>, String> {
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

/// The installer an update runs: this repository's script, built into the binary.
#[cfg_attr(not(any(windows, target_os = "macos")), allow(dead_code))]
struct Script {
    file: &'static str,
    text: &'static str,
    /// The name the verified download is saved under.
    package: &'static str,
}

#[cfg(windows)]
const SCRIPT: Option<Script> =
    Some(Script { file: "install.ps1", text: include_str!("../../install.ps1"), package: "Pointcast_x64-setup.exe" });
#[cfg(target_os = "macos")]
const SCRIPT: Option<Script> =
    Some(Script { file: "install.sh", text: include_str!("../../install.sh"), package: "Pointcast.dmg" });
#[cfg(not(any(windows, target_os = "macos")))]
const SCRIPT: Option<Script> = None;

/// Download the update, check it and run the built-in installer, all in the background; the
/// installer's output goes to `update.log` in the data folder. A good update closes Pointcast
/// before the installer ends, so `failed` runs only when the update stopped and this copy is
/// still running.
pub fn install(failed: impl FnOnce(String) + Send + 'static) -> Result<(), String> {
    let Some(script) = SCRIPT else {
        return Err("On Linux, update Pointcast from the releases page.".into());
    };
    let offer = AVAILABLE.lock().unwrap().clone().ok_or("There is no update to install. Check for updates first.")?;
    let job = Job {
        offer,
        script,
        tmp: std::env::temp_dir().join("pointcast-update"),
        data: crate::settings::data_dir(),
        run,
    };
    start(job, &RUNNING, failed)
}

/// One update: what to fetch, where it goes, and what starts the installer (tests swap `run`).
struct Job {
    offer: Offer,
    script: Script,
    /// A folder for the download and the script alone, emptied first.
    tmp: PathBuf,
    /// The data folder: the record and `update.log`.
    data: PathBuf,
    /// Start the script on the verified download with its output in the log; return when it ends.
    run: fn(script: &Path, package: &Path, log: &Path) -> Result<(), String>,
}

/// Refuse a second update while one runs; otherwise do `job` on a thread of its own.
fn start(job: Job, busy: &'static AtomicBool, failed: impl FnOnce(String) + Send + 'static) -> Result<(), String> {
    if busy.swap(true, Ordering::SeqCst) {
        return Err("An update is already running.".into());
    }
    std::thread::spawn(move || {
        let log = job.data.join(LOG);
        let why = match job.go(&log) {
            Ok(()) => reason(&String::from_utf8_lossy(&std::fs::read(&log).unwrap_or_default())),
            Err(why) => why,
        };
        // This copy is still running, so it says why now rather than at its next start.
        let _ = std::fs::remove_file(job.data.join(RECORD));
        busy.store(false, Ordering::SeqCst);
        failed(why);
    });
    Ok(())
}

impl Job {
    /// Fetch, verify and run. `Ok` once the installer has ended.
    fn go(&self, log: &Path) -> Result<(), String> {
        let io = |e: std::io::Error| format!("Pointcast could not save the update: {e}");
        let _ = std::fs::remove_dir_all(&self.tmp);
        std::fs::create_dir_all(&self.tmp).map_err(io)?;
        let package = self.tmp.join(self.script.package);
        download(&self.offer.url, &package)?;
        verify(&package, &self.offer.sha256)?;
        let script = self.tmp.join(self.script.file);
        std::fs::write(&script, self.script.text).map_err(io)?;
        std::fs::create_dir_all(&self.data).map_err(io)?;
        let record = serde_json::json!({ "from": env!("CARGO_PKG_VERSION"), "to": self.offer.version });
        std::fs::write(self.data.join(RECORD), record.to_string()).map_err(io)?;
        (self.run)(&script, &package, log)
    }
}

/// Stream `url` to `to`. Only HTTPS in the app; tests serve from a loopback server.
fn download(url: &str, to: &Path) -> Result<(), String> {
    if !(url.starts_with("https://") || (cfg!(test) && url.starts_with("http://127.0.0.1:"))) {
        return Err("The update's download address is not secure, so it was not used.".into());
    }
    let client = reqwest::blocking::Client::builder()
        .user_agent(concat!("Pointcast/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(20))
        // Each read waits at most this long, so a stalled download stops instead of hanging.
        .timeout(Duration::from_secs(60))
        .build()
        .map_err(|e| e.to_string())?;
    let mut resp = client
        .get(url)
        .send()
        .map_err(|_| "Can't reach GitHub. Check your connection and try again.".to_string())?;
    if !resp.status().is_success() {
        return Err(format!("GitHub answered {} to the download. Try again later.", resp.status().as_u16()));
    }
    let mut file = std::fs::File::create(to).map_err(|e| format!("Pointcast could not save the update: {e}"))?;
    if std::io::copy(&mut resp, &mut file).is_err() {
        drop(file);
        let _ = std::fs::remove_file(to);
        return Err("The download did not finish. Check your connection and try again.".into());
    }
    Ok(())
}

/// Keep the download only if its SHA-256 is the one GitHub published for it.
fn verify(path: &Path, sha256: &str) -> Result<(), String> {
    if crate::voice::models::sha256_file(path).is_ok_and(|h| h == sha256) {
        return Ok(());
    }
    let _ = std::fs::remove_file(path);
    Err(DAMAGED.into())
}

/// Read and clear the record an update left in `data`: the installer's reason when this copy is
/// older than the version it was installing, else nothing.
fn failure(data: &Path, current: &str) -> Option<String> {
    #[derive(Deserialize)]
    struct Record {
        to: String,
    }
    let record = std::fs::read(data.join(RECORD)).ok()?;
    let _ = std::fs::remove_file(data.join(RECORD));
    let to = serde_json::from_slice::<Record>(&record).ok()?.to;
    let (Some(cur), Some(to)) = (version(current), version(&to)) else {
        return None;
    };
    (cur < to).then(|| reason(&String::from_utf8_lossy(&std::fs::read(data.join(LOG)).unwrap_or_default())))
}

/// The installer's own words for why it stopped.
fn reason(log: &str) -> String {
    log.lines()
        .find_map(|l| l.split_once("was not installed:").map(|(_, why)| why.trim().to_string()))
        .filter(|why| !why.is_empty())
        .unwrap_or_else(|| "The update did not finish. Try again in a minute.".into())
}

#[cfg(windows)]
fn run(script: &Path, package: &Path, log: &Path) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    let mut cmd = std::process::Command::new("powershell.exe");
    cmd.args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"])
        .arg(script)
        .env("POINTCAST_SETUP", package)
        .creation_flags(CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP);
    wait(cmd, log)
}

#[cfg(target_os = "macos")]
fn run(script: &Path, package: &Path, log: &Path) -> Result<(), String> {
    let mut cmd = std::process::Command::new("/bin/bash");
    cmd.arg(script).env("POINTCAST_DMG", package);
    wait(cmd, log)
}

#[cfg(not(any(windows, target_os = "macos")))]
fn run(_: &Path, _: &Path, _: &Path) -> Result<(), String> {
    Err("On Linux, update Pointcast from the releases page.".into())
}

/// Start the installer with its output in `log` and wait for it to end.
#[cfg(any(windows, target_os = "macos"))]
fn wait(mut cmd: std::process::Command, log: &Path) -> Result<(), String> {
    let out = std::fs::File::create(log).map_err(|e| e.to_string())?;
    let err = out.try_clone().map_err(|e| e.to_string())?;
    cmd.stdin(std::process::Stdio::null()).stdout(out).stderr(err);
    let mut child = cmd.spawn().map_err(|e| format!("The installer did not start: {e}"))?;
    let _ = child.wait();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};
    use std::io::{Read, Write};

    /// A release whose assets each carry a download address and a SHA-256 (asset i: i repeated).
    fn release(tag: &str, assets: &[&str]) -> String {
        let assets: Vec<_> = assets
            .iter()
            .enumerate()
            .map(|(i, n)| {
                serde_json::json!({
                    "name": n,
                    "browser_download_url": format!("https://github.com/naidx0/pointcast/releases/download/{tag}/{n}"),
                    "digest": format!("sha256:{}", i.to_string().repeat(64)),
                })
            })
            .collect();
        serde_json::json!({ "tag_name": tag, "draft": false, "prerelease": false, "assets": assets }).to_string()
    }

    fn offered(body: &str, current: &str, os: &str) -> Option<String> {
        offer(body, current, os).unwrap().map(|o| o.version)
    }

    const BOTH: [&str; 2] = ["Pointcast_0.5.0_x64-setup.exe", "Pointcast_0.5.0_universal.dmg"];

    #[test]
    fn a_newer_release_with_a_download_for_this_system_is_offered() {
        for os in ["windows", "macos"] {
            assert_eq!(offered(&release("v0.5.0", &BOTH), "0.4.0", os).as_deref(), Some("0.5.0"));
        }
        assert_eq!(offered(&release("v0.4.1", &BOTH), "0.4.0", "windows").as_deref(), Some("0.4.1"));
        assert_eq!(offered(&release("v1.0", &BOTH), "0.9.12", "macos").as_deref(), Some("1.0"));
    }

    #[test]
    fn the_same_or_an_older_release_is_not_offered() {
        for tag in ["v0.4.0", "0.4.0", "v0.3.9", "v0.4.0-beta.2"] {
            assert_eq!(offered(&release(tag, &BOTH), "0.4.0", "windows"), None, "{tag}");
        }
        assert_eq!(offered(&release("v0.10.0", &BOTH), "0.9.0", "windows").as_deref(), Some("0.10.0"));
    }

    #[test]
    fn a_release_without_this_systems_download_is_not_offered() {
        // The Windows build goes up first and the Mac one later: a Mac must wait for its .dmg.
        let win_only = release("v0.5.0", &["Pointcast_0.5.0_x64-setup.exe"]);
        assert_eq!(offered(&win_only, "0.4.0", "macos"), None);
        assert_eq!(offered(&win_only, "0.4.0", "windows").as_deref(), Some("0.5.0"));
        assert_eq!(offered(&release("v0.5.0", &BOTH), "0.4.0", "linux"), None);
    }

    #[test]
    fn drafts_prereleases_and_odd_tags_are_not_offered() {
        let mut v: serde_json::Value = serde_json::from_str(&release("v0.5.0", &BOTH)).unwrap();
        v["prerelease"] = true.into();
        assert_eq!(offered(&v.to_string(), "0.4.0", "windows"), None);
        v["prerelease"] = false.into();
        v["draft"] = true.into();
        assert_eq!(offered(&v.to_string(), "0.4.0", "windows"), None);
        assert_eq!(offered(&release("nightly", &BOTH), "0.4.0", "windows"), None);
        assert!(offer("<html>rate limited</html>", "0.4.0", "windows").is_err());
    }

    #[test]
    fn the_offer_is_this_systems_download_and_its_sha256() {
        let body = release("v0.5.0", &["notes.txt", BOTH[0], BOTH[1]]);
        let win = offer(&body, "0.4.0", "windows").unwrap().unwrap();
        assert_eq!(win.url, "https://github.com/naidx0/pointcast/releases/download/v0.5.0/Pointcast_0.5.0_x64-setup.exe");
        assert_eq!(win.sha256, "1".repeat(64));
        let mac = offer(&body, "0.4.0", "macos").unwrap().unwrap();
        assert_eq!(mac.url, "https://github.com/naidx0/pointcast/releases/download/v0.5.0/Pointcast_0.5.0_universal.dmg");
        assert_eq!(mac.sha256, "2".repeat(64));
        // GitHub writes hex in lower case; an upper-case one still compares equal to the file's.
        let mut v: serde_json::Value = serde_json::from_str(&body).unwrap();
        v["assets"][1]["digest"] = format!("sha256:{}", "AB".repeat(32)).into();
        assert_eq!(offer(&v.to_string(), "0.4.0", "windows").unwrap().unwrap().sha256, "ab".repeat(32));
    }

    #[test]
    fn a_release_without_a_sha256_for_this_systems_download_is_not_offered() {
        let bad = [
            serde_json::Value::Null,
            "".into(),
            format!("sha1:{}", "a".repeat(40)).into(),
            format!("sha256:{}", "a".repeat(63)).into(),
            format!("sha256:{}", "z".repeat(64)).into(),
        ];
        for digest in bad {
            let mut v: serde_json::Value = serde_json::from_str(&release("v0.5.0", &BOTH)).unwrap();
            v["assets"][0]["digest"] = digest.clone();
            assert_eq!(offered(&v.to_string(), "0.4.0", "windows"), None, "{digest}");
            // The Mac download kept its digest, so a Mac is still offered the release.
            assert_eq!(offered(&v.to_string(), "0.4.0", "macos").as_deref(), Some("0.5.0"), "{digest}");
        }
        let mut v: serde_json::Value = serde_json::from_str(&release("v0.5.0", &BOTH)).unwrap();
        v["assets"][0].as_object_mut().unwrap().remove("digest");
        assert_eq!(offered(&v.to_string(), "0.4.0", "windows"), None);
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
    fn a_failed_update_says_what_the_installer_said() {
        let ps1 = "  Pointcast
  Finding the latest version...

  Pointcast was not installed: can't reach GitHub. Check your internet connection and run this again.
";
        assert_eq!(reason(ps1), "can't reach GitHub. Check your internet connection and run this again.");
        let sh = "
  Pointcast
  Downloading Pointcast.dmg...

  Pointcast was not installed: the disk image would not open.

";
        assert_eq!(reason(sh), "the disk image would not open.");
        assert!(reason("").contains("did not finish"));
        assert!(reason("powershell: some other error").contains("did not finish"));
    }

    #[test]
    fn the_check_reads_github_over_the_wire() {
        let url = serve(200, release("v0.5.0", &BOTH));
        let found = check_at(&url, "0.4.0", "windows").unwrap().unwrap();
        assert_eq!(found.version, "0.5.0");
        assert!(found.url.ends_with("x64-setup.exe"), "{}", found.url);
        // A repo with nothing published answers 404: up to date, not an error.
        assert_eq!(check_at(&serve(404, "{}".into()), "0.4.0", "windows").unwrap(), None);
        let limited = check_at(&serve(403, "{}".into()), "0.4.0", "windows").unwrap_err();
        assert!(limited.contains("limiting"), "{limited}");
    }

    /// A folder of the test's own, empty.
    fn scratch(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("pointcast-update-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn the_download_is_kept_only_if_its_sha256_matches() {
        let dir = scratch("verify");
        let file = dir.join("setup.exe");
        std::fs::write(&file, b"the installer").unwrap();
        let good = format!("{:x}", Sha256::digest(b"the installer"));
        assert_eq!(verify(&file, &good), Ok(()));
        assert!(file.exists());
        assert_eq!(verify(&file, &"0".repeat(64)), Err(DAMAGED.to_string()));
        assert!(!file.exists(), "a damaged download is thrown away");
    }

    const PACKAGE: &str = "the new Pointcast";

    /// Stands in for the installer: says what it was given, in the installer's own words.
    fn fake_run(script: &Path, package: &Path, log: &Path) -> Result<(), String> {
        let record = std::fs::read_to_string(log.with_file_name(RECORD)).unwrap_or_default();
        let said = format!(
            "  Pointcast was not installed: ran {} ({}) on {} with record {record}\n",
            script.file_name().unwrap().to_string_lossy(),
            std::fs::read_to_string(script).unwrap(),
            std::fs::read_to_string(package).unwrap(),
        );
        std::fs::write(log, said).map_err(|e| e.to_string())
    }

    fn slow_run(script: &Path, package: &Path, log: &Path) -> Result<(), String> {
        std::thread::sleep(Duration::from_millis(300));
        fake_run(script, package, log)
    }

    fn job(name: &str, url: String, sha256: String, run: fn(&Path, &Path, &Path) -> Result<(), String>) -> Job {
        let dir = scratch(name);
        Job {
            offer: Offer { version: "0.5.0".into(), url, sha256 },
            script: Script { file: "install.ps1", text: "the built-in script", package: "Pointcast_x64-setup.exe" },
            tmp: dir.join("tmp"),
            data: dir.join("data"),
            run,
        }
    }

    /// Start `job` and wait for the reason it reports.
    fn reported(job: Job, busy: &'static AtomicBool) -> String {
        let (tx, rx) = std::sync::mpsc::channel();
        start(job, busy, move |why| tx.send(why).unwrap()).unwrap();
        rx.recv_timeout(Duration::from_secs(30)).unwrap()
    }

    #[test]
    fn an_update_downloads_verifies_and_runs_the_built_in_script() {
        static BUSY: AtomicBool = AtomicBool::new(false);
        let sha = format!("{:x}", Sha256::digest(PACKAGE.as_bytes()));
        let j = job("e2e", serve(200, PACKAGE.into()), sha, fake_run);
        let (tmp, data) = (j.tmp.clone(), j.data.clone());
        // An earlier update's leftovers are cleared first.
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(tmp.join("old.exe"), b"old").unwrap();
        let why = reported(j, &BUSY);
        let record = serde_json::json!({ "from": env!("CARGO_PKG_VERSION"), "to": "0.5.0" }).to_string();
        assert_eq!(why, format!("ran install.ps1 (the built-in script) on {PACKAGE} with record {record}"));
        assert!(!tmp.join("old.exe").exists());
        // The installer ended while this copy runs, so it was told now and the record is gone.
        assert!(!data.join(RECORD).exists());
        assert!(!BUSY.load(Ordering::SeqCst));
    }

    #[test]
    fn a_damaged_download_is_thrown_away_and_never_run() {
        static BUSY: AtomicBool = AtomicBool::new(false);
        let j = job("damaged", serve(200, PACKAGE.into()), "0".repeat(64), fake_run);
        let (tmp, data) = (j.tmp.clone(), j.data.clone());
        assert_eq!(reported(j, &BUSY), DAMAGED);
        assert!(!tmp.join("Pointcast_x64-setup.exe").exists());
        assert!(!tmp.join("install.ps1").exists());
        assert!(!data.join(LOG).exists() && !data.join(RECORD).exists(), "the installer never ran");
    }

    #[test]
    fn a_download_that_cannot_be_had_says_so() {
        static BUSY: AtomicBool = AtomicBool::new(false);
        let why = reported(job("refused", serve(404, "{}".into()), "0".repeat(64), fake_run), &BUSY);
        assert!(why.contains("404"), "{why}");
        let why = reported(job("plain", "http://example.com/setup.exe".into(), "0".repeat(64), fake_run), &BUSY);
        assert!(why.contains("not secure"), "{why}");
    }

    #[test]
    fn a_second_update_is_refused_while_one_runs() {
        let sha = format!("{:x}", Sha256::digest(PACKAGE.as_bytes()));
        assert!(!installing());
        let (tx, rx) = std::sync::mpsc::channel();
        start(job("busy1", serve(200, PACKAGE.into()), sha.clone(), slow_run), &RUNNING, move |why| tx.send(why).unwrap()).unwrap();
        assert!(installing());
        let again = start(job("busy2", "https://example.com/never".into(), sha.clone(), fake_run), &RUNNING, |_| ());
        assert_eq!(again.unwrap_err(), "An update is already running.");
        assert!(rx.recv_timeout(Duration::from_secs(30)).unwrap().starts_with("ran install.ps1"));
        assert!(!installing(), "a failed update frees the way for the next");
        assert!(reported(job("busy3", serve(200, PACKAGE.into()), sha, fake_run), &RUNNING).starts_with("ran install.ps1"));
    }

    #[test]
    fn the_next_start_tells_a_failed_update_from_a_good_one() {
        let dir = scratch("record");
        let record = |to: &str| std::fs::write(dir.join(RECORD), serde_json::json!({ "from": "0.4.0", "to": to }).to_string()).unwrap();
        assert_eq!(failure(&dir, "0.4.0"), None, "no record, nothing to say");

        record("0.5.0");
        assert_eq!(failure(&dir, "0.5.0"), None, "running the new version: it worked");
        assert!(!dir.join(RECORD).exists());

        record("0.5.0");
        std::fs::write(dir.join(LOG), "  Pointcast was not installed: the installer stopped with code 2.\n").unwrap();
        assert_eq!(failure(&dir, "0.4.0").as_deref(), Some("the installer stopped with code 2."));
        assert!(!dir.join(RECORD).exists());
        assert_eq!(failure(&dir, "0.4.0"), None, "read once, then cleared");

        record("0.5.0");
        std::fs::remove_file(dir.join(LOG)).unwrap();
        assert!(failure(&dir, "0.4.0").unwrap().contains("did not finish"));

        std::fs::write(dir.join(RECORD), "not json").unwrap();
        assert_eq!(failure(&dir, "0.4.0"), None);
        assert!(!dir.join(RECORD).exists());
    }

    #[test]
    fn the_built_in_installer_takes_the_verified_download() {
        let Some(s) = SCRIPT else { return };
        let var = if cfg!(windows) { "$env:POINTCAST_SETUP" } else { "POINTCAST_DMG" };
        assert!(s.text.contains(var), "{} must read {var}", s.file);
        // Windows PowerShell reads a script without a byte order mark in the ANSI code page.
        assert!(s.text.is_ascii(), "{} must stay ASCII", s.file);
    }
}
