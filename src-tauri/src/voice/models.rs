//! Speech models and the sherpa-onnx runtime, downloaded on first use over HTTPS.
//! Archives unpack into `<data dir>/speech/<id>/`. Every archive and model file is pinned by
//! size and SHA-256 in models.json; an install only counts once it is whole.
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

pub const CATALOG: &str = include_str!("../../models.json");

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileEntry {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(default)]
    pub bytes: u64,
    #[serde(default)]
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelEntry {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub family: String,
    #[serde(default)]
    pub license: String,
    pub urls: Vec<String>,
    #[serde(default)]
    pub bytes: u64,
    #[serde(default)]
    pub sha256: String,
    #[serde(default)]
    pub root: String,
    pub files: Vec<FileEntry>,
    #[serde(default)]
    pub bench: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeEntry {
    pub id: String,
    pub target: String,
    pub urls: Vec<String>,
    #[serde(default)]
    pub bytes: u64,
    #[serde(default)]
    pub sha256: String,
    pub lib: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Catalog {
    pub version: u32,
    pub models: Vec<ModelEntry>,
    pub runtimes: Vec<RuntimeEntry>,
    #[serde(default)]
    pub reference: Value,
}

pub fn catalog() -> Catalog {
    serde_json::from_str(CATALOG).expect("models.json")
}

impl Catalog {
    pub fn model(&self, id: &str) -> Option<&ModelEntry> {
        self.models.iter().find(|m| m.id == id)
    }
    pub fn runtime_for_host(&self) -> Option<&RuntimeEntry> {
        let t = target_triple();
        self.runtimes
            .iter()
            .find(|r| r.target == t)
            .or_else(|| self.runtimes.iter().find(|r| r.target.split('-').next() == t.split('-').next()))
    }
}

pub fn target_triple() -> &'static str {
    if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        "x86_64-unknown-linux-gnu"
    } else if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        "aarch64-apple-darwin"
    } else if cfg!(all(target_os = "macos", target_arch = "x86_64")) {
        "x86_64-apple-darwin"
    } else if cfg!(all(windows, target_arch = "x86_64")) {
        "x86_64-pc-windows-msvc"
    } else if cfg!(all(target_os = "linux", target_arch = "aarch64")) {
        "aarch64-unknown-linux-gnu"
    } else {
        "unknown"
    }
}

impl ModelEntry {
    pub fn file(&self, role: &str) -> Option<&FileEntry> {
        self.files.iter().find(|f| f.role.as_deref() == Some(role))
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum Progress {
    Downloading { received: u64, total: Option<u64> },
    Unpacking,
    Verifying,
    Ready,
    Failed { reason: String },
}

pub fn speech_dir() -> PathBuf {
    if let Some(d) = std::env::var_os("POINTCAST_SPEECH_DIR") {
        return d.into();
    }
    crate::settings::data_dir().join("speech")
}

pub fn install_dir(root: &Path, id: &str) -> PathBuf {
    root.join(id)
}

pub fn model_installed(root: &Path, e: &ModelEntry) -> bool {
    let d = install_dir(root, &e.id);
    !e.files.is_empty()
        && e.files.iter().all(|f| {
            let p = d.join(&f.name);
            p.is_file() && (f.bytes == 0 || std::fs::metadata(&p).map(|m| m.len() == f.bytes).unwrap_or(false))
        })
}

/// A runtime counts once its library is there and the install finished (marker written last).
pub fn runtime_installed(root: &Path, e: &RuntimeEntry) -> bool {
    find_lib(root, e).is_some() && done_marker_ok(&install_dir(root, &e.id), &e.sha256)
}

const DONE: &str = ".pointcast-installed";

fn done_marker_ok(dir: &Path, sha: &str) -> bool {
    std::fs::read_to_string(dir.join(DONE)).is_ok_and(|s| s.trim() == sha)
}

pub fn find_lib(root: &Path, e: &RuntimeEntry) -> Option<PathBuf> {
    let d = install_dir(root, &e.id);
    let direct = d.join(&e.lib);
    if direct.is_file() {
        return Some(direct);
    }
    find_named(&d, &e.lib)
}

fn find_named(dir: &Path, name: &str) -> Option<PathBuf> {
    let walker = walkdir_files(dir);
    walker.into_iter().find(|p| p.file_name().and_then(|s| s.to_str()) == Some(name))
}

fn walkdir_files(dir: &Path) -> Vec<PathBuf> {
    let mut out = vec![];
    fn rec(d: &Path, out: &mut Vec<PathBuf>, depth: u8) {
        if depth > 6 {
            return;
        }
        let Ok(rd) = std::fs::read_dir(d) else {
            return;
        };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                rec(&p, out, depth + 1);
            } else {
                out.push(p);
            }
        }
    }
    rec(dir, &mut out, 0);
    out
}

pub fn sha256_file(path: &Path) -> std::io::Result<String> {
    let mut f = std::fs::File::open(path)?;
    let mut h = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(format!("{:x}", h.finalize()))
}

/// Set to stop an install between chunks; cleared when the next one starts.
pub static CANCEL: AtomicBool = AtomicBool::new(false);

/// One request streams for at most this long, then the next resumes where it stopped, so a
/// stalled connection costs a minute, never the whole download.
const WINDOW: Duration = Duration::from_secs(60);
/// Rounds in a row that move no bytes before the download gives up.
const STALLS: u32 = 4;

pub const CANCELLED: &str = "Cancelled.";
pub const DAMAGED: &str = "The download arrived damaged, so it was thrown away. Try again; it starts over.";
pub const OFFLINE: &str =
    "Pointcast can't reach GitHub to download the speech files. Check your internet connection, then try again.";

/// Fetch `urls` (tried in order) into `dest`, resuming a `.partial` left by an earlier try.
/// `bytes` and `sha256`, when known, must match or the file is discarded.
pub fn download(
    urls: &[String],
    dest: &Path,
    bytes: u64,
    sha256: &str,
    progress: &dyn Fn(Progress),
) -> Result<(), String> {
    if let Some(d) = dest.parent() {
        std::fs::create_dir_all(d).map_err(|e| io_text(&e, d))?;
    }
    let tmp = dest.with_extension("partial");
    let client = reqwest::blocking::Client::builder()
        .user_agent(concat!("Pointcast/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(20))
        .timeout(WINDOW)
        .build()
        .map_err(|e| e.to_string())?;
    let mut last_err = String::from("no download address");
    // Only HTTPS in the app; tests serve from a loopback server.
    let allowed = |u: &&String| u.starts_with("https://") || (cfg!(test) && u.starts_with("http://127.0.0.1:"));
    for url in urls.iter().filter(allowed) {
        match fetch(&client, url, &tmp, bytes, progress) {
            Ok(()) => match check(&tmp, bytes, sha256) {
                Ok(()) => return std::fs::rename(&tmp, dest).map_err(|e| io_text(&e, dest)),
                Err(e) => {
                    let _ = std::fs::remove_file(&tmp);
                    last_err = e;
                }
            },
            Err(e) if e == CANCELLED => return Err(e),
            Err(e) => last_err = e,
        }
    }
    Err(last_err)
}

/// A finished file is kept only if its size and hash are the pinned ones.
pub fn check(path: &Path, bytes: u64, sha256: &str) -> Result<(), String> {
    let len = std::fs::metadata(path).map_err(|e| io_text(&e, path))?.len();
    if bytes > 0 && len != bytes {
        return Err(DAMAGED.into());
    }
    if !sha256.is_empty() && sha256_file(path).map_err(|e| io_text(&e, path))? != sha256 {
        return Err(DAMAGED.into());
    }
    Ok(())
}

fn fetch(
    client: &reqwest::blocking::Client,
    url: &str,
    tmp: &Path,
    expect: u64,
    progress: &dyn Fn(Progress),
) -> Result<(), String> {
    let mut stalls = 0u32;
    let mut tick = Instant::now() - Duration::from_secs(1);
    loop {
        if CANCEL.load(Ordering::SeqCst) {
            return Err(CANCELLED.into());
        }
        let have = std::fs::metadata(tmp).map(|m| m.len()).unwrap_or(0);
        if expect > 0 && have >= expect {
            if have > expect {
                let _ = std::fs::remove_file(tmp);
                continue;
            }
            return Ok(());
        }
        let mut moved = 0u64;
        match stream(client, url, tmp, have, expect, &mut moved, &mut tick, progress) {
            Ok(true) => return Ok(()),
            Err(Round::Fatal(e)) => return Err(e),
            Ok(false) | Err(Round::Retry(_)) if moved > 0 => stalls = 0,
            Ok(false) => stalls += 1,
            Err(Round::Retry(e)) => {
                stalls += 1;
                if stalls >= STALLS {
                    return Err(e);
                }
            }
        }
        if stalls >= STALLS {
            return Err(OFFLINE.into());
        }
        if stalls > 0 {
            std::thread::sleep(Duration::from_millis(500 << stalls.min(3)));
        }
    }
}

/// Why a round stopped short: the network (try again) or something another try cannot fix.
enum Round {
    Retry(String),
    Fatal(String),
}

/// What a server status means for the next round.
fn status_round(status: reqwest::StatusCode) -> Round {
    let text = format!("The download server answered {status}. Try again in a minute.");
    if status.is_server_error() || status == reqwest::StatusCode::TOO_MANY_REQUESTS {
        Round::Retry(text)
    } else {
        Round::Fatal(text)
    }
}

/// One request: Ok(true) when the file is whole, Ok(false) when the round ended early (window
/// closed, connection dropped) with whatever arrived kept on disk.
#[allow(clippy::too_many_arguments)]
fn stream(
    client: &reqwest::blocking::Client,
    url: &str,
    tmp: &Path,
    have: u64,
    expect: u64,
    moved: &mut u64,
    tick: &mut Instant,
    progress: &dyn Fn(Progress),
) -> Result<bool, Round> {
    let fatal = |s: String| Round::Fatal(s);
    let mut req = client.get(url);
    if have > 0 {
        req = req.header(reqwest::header::RANGE, format!("bytes={have}-"));
    }
    let mut resp = req.send().map_err(|e| Round::Retry(net_text(&e)))?;
    let status = resp.status();
    if status == reqwest::StatusCode::RANGE_NOT_SATISFIABLE {
        // The partial is stale or longer than the file: start clean.
        let _ = std::fs::remove_file(tmp);
        return Ok(false);
    }
    if !status.is_success() {
        return Err(status_round(status));
    }
    let resumed = have > 0 && status == reqwest::StatusCode::PARTIAL_CONTENT;
    let total = if resumed {
        content_range_total(resp.headers())
    } else {
        resp.content_length()
    }
    .or((expect > 0).then_some(expect));
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .append(resumed)
        .truncate(!resumed)
        .open(tmp)
        .map_err(|e| fatal(io_text(&e, tmp)))?;
    let mut received = if resumed { have } else { 0 };
    let mut buf = vec![0u8; 256 * 1024];
    loop {
        if CANCEL.load(Ordering::SeqCst) {
            return Err(fatal(CANCELLED.into()));
        }
        let n = match resp.read(&mut buf) {
            Ok(0) => return Ok(total.is_none_or(|t| received >= t)),
            Ok(n) => n,
            Err(_) => return Ok(false),
        };
        file.write_all(&buf[..n]).map_err(|e| fatal(io_text(&e, tmp)))?;
        received += n as u64;
        *moved += n as u64;
        if tick.elapsed() >= Duration::from_millis(100) {
            *tick = Instant::now();
            progress(Progress::Downloading { received, total });
        }
    }
}

fn content_range_total(h: &reqwest::header::HeaderMap) -> Option<u64> {
    h.get(reqwest::header::CONTENT_RANGE)?
        .to_str()
        .ok()?
        .rsplit('/')
        .next()?
        .parse()
        .ok()
}

fn net_text(e: &reqwest::Error) -> String {
    if e.is_connect() || e.is_timeout() || e.is_request() {
        OFFLINE.into()
    } else {
        format!("The download failed: {e}")
    }
}

/// Plain words for the failures people can fix themselves.
pub fn io_text(e: &std::io::Error, path: &Path) -> String {
    match e.raw_os_error() {
        // ENOSPC on macOS and Linux; ERROR_HANDLE_DISK_FULL and ERROR_DISK_FULL on Windows.
        Some(28) | Some(39) | Some(112) => "The disk is full. Free some space, then try again.".into(),
        _ if e.kind() == std::io::ErrorKind::PermissionDenied => {
            format!("Pointcast can't write to {}. Check that folder's permissions.", path.display())
        }
        _ => format!("{e} ({})", path.display()),
    }
}

pub fn unpack_tar_bz2(archive: &Path, dest: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dest).map_err(|e| io_text(&e, dest))?;
    let f = std::fs::File::open(archive).map_err(|e| io_text(&e, archive))?;
    let dec = bzip2::read::BzDecoder::new(f);
    let mut ar = tar::Archive::new(dec);
    ar.unpack(dest).map_err(|e| match e.raw_os_error() {
        Some(_) => io_text(&e, dest),
        None => DAMAGED.into(),
    })
}

/// Free space on the volume holding `dir` (or its nearest existing parent).
pub fn free_bytes(dir: &Path) -> Option<u64> {
    let mut d = dir;
    while !d.exists() {
        d = d.parent()?;
    }
    #[cfg(windows)]
    {
        use windows::core::HSTRING;
        use windows::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
        let mut free = 0u64;
        unsafe { GetDiskFreeSpaceExW(&HSTRING::from(d.as_os_str()), Some(&mut free), None, None) }.ok()?;
        Some(free)
    }
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        let c = std::ffi::CString::new(d.as_os_str().as_bytes()).ok()?;
        let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
        (unsafe { libc::statvfs(c.as_ptr(), &mut st) } == 0).then(|| st.f_bavail as u64 * st.f_frsize as u64)
    }
}

/// The archive, its unpacked copy and the kept files all sit on disk at once while installing.
pub fn space_needed(archive_bytes: u64) -> u64 {
    archive_bytes.saturating_mul(3) + 50_000_000
}

fn swap_in(staging: &Path, dest: &Path) -> Result<(), String> {
    let _ = std::fs::remove_dir_all(dest);
    std::fs::rename(staging, dest).map_err(|e| io_text(&e, dest))
}

pub fn install_model(
    root: &Path,
    e: &ModelEntry,
    progress: &dyn Fn(Progress),
) -> Result<PathBuf, String> {
    if model_installed(root, e) {
        progress(Progress::Ready);
        return Ok(install_dir(root, &e.id));
    }
    let archive = root.join(format!("{}.tar.bz2", e.id));
    download(&e.urls, &archive, e.bytes, &e.sha256, progress)?;
    progress(Progress::Unpacking);
    let unpacked = root.join(format!("{}.unpack", e.id));
    let staging = root.join(format!("{}.staging", e.id));
    let _ = std::fs::remove_dir_all(&unpacked);
    let _ = std::fs::remove_dir_all(&staging);
    let r = (|| {
        unpack_tar_bz2(&archive, &unpacked)?;
        progress(Progress::Verifying);
        std::fs::create_dir_all(&staging).map_err(|x| io_text(&x, &staging))?;
        for f in &e.files {
            let src = find_named(&unpacked, &f.name).ok_or(DAMAGED)?;
            check(&src, f.bytes, &f.sha256)?;
            let to = staging.join(&f.name);
            std::fs::rename(&src, &to)
                .or_else(|_| std::fs::copy(&src, &to).map(|_| ()))
                .map_err(|x| io_text(&x, &staging))?;
        }
        swap_in(&staging, &install_dir(root, &e.id))
    })();
    let _ = std::fs::remove_dir_all(&unpacked);
    let _ = std::fs::remove_dir_all(&staging);
    let _ = std::fs::remove_file(&archive);
    r?;
    progress(Progress::Ready);
    Ok(install_dir(root, &e.id))
}

pub fn install_runtime(
    root: &Path,
    e: &RuntimeEntry,
    progress: &dyn Fn(Progress),
) -> Result<PathBuf, String> {
    if runtime_installed(root, e) {
        progress(Progress::Ready);
        return find_lib(root, e).ok_or_else(|| DAMAGED.to_string());
    }
    let archive = root.join(format!("{}.tar.bz2", e.id));
    download(&e.urls, &archive, e.bytes, &e.sha256, progress)?;
    progress(Progress::Unpacking);
    let staging = root.join(format!("{}.staging", e.id));
    let _ = std::fs::remove_dir_all(&staging);
    let r = (|| {
        unpack_tar_bz2(&archive, &staging)?;
        find_named(&staging, &e.lib).ok_or(DAMAGED)?;
        // Written last: an install cut short never reads as finished.
        std::fs::write(staging.join(DONE), &e.sha256).map_err(|x| io_text(&x, &staging))?;
        swap_in(&staging, &install_dir(root, &e.id))
    })();
    let _ = std::fs::remove_dir_all(&staging);
    let _ = std::fs::remove_file(&archive);
    r?;
    progress(Progress::Ready);
    find_lib(root, e).ok_or_else(|| DAMAGED.to_string())
}

pub fn view(root: &Path, c: &Catalog) -> Value {
    let rt = c.runtime_for_host();
    serde_json::json!({
        "runtime": rt.map(|r| serde_json::json!({
            "id": r.id, "lib": r.lib, "installed": runtime_installed(root, r), "bytes": r.bytes
        })),
        "models": c.models.iter().map(|m| serde_json::json!({
            "id": m.id, "name": m.name, "family": m.family, "license": m.license,
            "bytes": m.bytes, "installed": model_installed(root, m), "bench": m.bench
        })).collect::<Vec<_>>(),
        "speech_dir": root,
        "target": target_triple(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_catalog_lists_moonshine_v2_and_a_runtime_per_desktop_os() {
        let c = catalog();
        assert_eq!(c.version, 1);
        let tiny = c.model("moonshine-tiny-en").unwrap();
        assert_eq!(tiny.family, "moonshine");
        assert_eq!(tiny.license, "MIT");
        assert!(tiny.file("encoder").is_some());
        assert!(tiny.file("merged_decoder").is_some());
        assert!(tiny.file("tokens").is_some());
        assert!(tiny.urls[0].starts_with("https://"));
        assert!(c.model("moonshine-base-en").is_some());
        let targets: Vec<_> = c.runtimes.iter().map(|r| r.target.as_str()).collect();
        assert!(targets.iter().any(|t| t.contains("linux")));
        assert!(targets.iter().any(|t| t.contains("apple")));
        assert!(targets.iter().any(|t| t.contains("windows")));
    }

    #[test]
    fn install_dir_nests_under_the_speech_root() {
        let p = install_dir(Path::new("/tmp/speech"), "moonshine-tiny-en");
        assert_eq!(p, PathBuf::from("/tmp/speech/moonshine-tiny-en"));
    }

    #[test]
    fn every_download_in_the_catalog_is_pinned_by_size_and_hash() {
        let c = catalog();
        let pinned = |b: u64, h: &str| b > 0 && h.len() == 64 && h.bytes().all(|x| x.is_ascii_hexdigit());
        for m in &c.models {
            assert!(pinned(m.bytes, &m.sha256), "{}", m.id);
            for f in &m.files {
                assert!(pinned(f.bytes, &f.sha256), "{} {}", m.id, f.name);
            }
        }
        for r in &c.runtimes {
            assert!(pinned(r.bytes, &r.sha256), "{}", r.id);
        }
        // The MD build needs the Visual C++ Redistributable, which a clean Windows lacks.
        let win = c.runtimes.iter().find(|r| r.target.contains("windows")).unwrap();
        assert!(win.urls.iter().all(|u| u.contains("-MT-")), "{:?}", win.urls);
    }

    /// A loopback server for one file. It honours `Range`, can cut the first response short, and
    /// records every Range it was asked for.
    struct Serve {
        url: String,
        ranges: Arc<Mutex<Vec<Option<u64>>>>,
    }

    use std::sync::{Arc, Mutex};

    fn serve(body: Vec<u8>, status: u16, cut_first_at: Option<usize>) -> Serve {
        use std::io::{BufRead, BufReader};
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://127.0.0.1:{}/file.tar.bz2", l.local_addr().unwrap().port());
        let ranges = Arc::new(Mutex::new(vec![]));
        let seen = ranges.clone();
        std::thread::spawn(move || {
            for (i, conn) in l.incoming().enumerate() {
                let Ok(mut c) = conn else { continue };
                let mut r = BufReader::new(c.try_clone().unwrap());
                let mut from = None;
                loop {
                    let mut line = String::new();
                    if r.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                        break;
                    }
                    if let Some(v) = line.to_ascii_lowercase().strip_prefix("range: bytes=") {
                        from = v.trim().trim_end_matches('-').parse::<u64>().ok();
                    }
                }
                seen.lock().unwrap().push(from);
                if status != 200 {
                    let _ = write!(c, "HTTP/1.1 {status} Nope\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
                    continue;
                }
                let start = from.unwrap_or(0) as usize;
                let head = match from {
                    Some(f) => format!(
                        "HTTP/1.1 206 Partial Content\r\nContent-Range: bytes {f}-{}/{}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len() - 1,
                        body.len(),
                        body.len() - start
                    ),
                    None => format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()),
                };
                let _ = c.write_all(head.as_bytes());
                let end = match cut_first_at {
                    Some(k) if i == 0 => k,
                    _ => body.len(),
                };
                let _ = c.write_all(&body[start..end.max(start)]);
                let _ = c.flush();
            }
        });
        Serve { url, ranges }
    }

    fn scratch(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("pc-dl-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn body() -> (Vec<u8>, String) {
        let b: Vec<u8> = (0..300_000u32).map(|i| (i * 31 % 251) as u8).collect();
        let h = format!("{:x}", Sha256::digest(&b));
        (b, h)
    }

    #[test]
    fn a_connection_cut_mid_file_resumes_where_it_stopped() {
        let (b, h) = body();
        let s = serve(b.clone(), 200, Some(100_000));
        let d = scratch("cut");
        let dest = d.join("a.tar.bz2");
        download(&[s.url.clone()], &dest, b.len() as u64, &h, &|_| {}).unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), b);
        assert_eq!(*s.ranges.lock().unwrap(), vec![None, Some(100_000)]);
        assert!(!dest.with_extension("partial").exists());
    }

    #[test]
    fn a_partial_left_by_an_earlier_try_is_continued_not_restarted() {
        let (b, h) = body();
        let s = serve(b.clone(), 200, None);
        let d = scratch("resume");
        let dest = d.join("a.tar.bz2");
        std::fs::write(dest.with_extension("partial"), &b[..123_456]).unwrap();
        download(&[s.url.clone()], &dest, b.len() as u64, &h, &|_| {}).unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), b);
        assert_eq!(*s.ranges.lock().unwrap(), vec![Some(123_456)]);
    }

    #[test]
    fn a_file_with_the_wrong_hash_is_thrown_away() {
        let (b, _) = body();
        let s = serve(b.clone(), 200, None);
        let d = scratch("hash");
        let dest = d.join("a.tar.bz2");
        let e = download(&[s.url.clone()], &dest, b.len() as u64, &"0".repeat(64), &|_| {}).unwrap_err();
        assert_eq!(e, DAMAGED);
        assert!(!dest.exists() && !dest.with_extension("partial").exists());
    }

    #[test]
    fn a_server_refusal_fails_at_once_and_names_the_status() {
        let s = serve(vec![], 404, None);
        let d = scratch("404");
        let t = Instant::now();
        let e = download(&[s.url.clone()], &d.join("a.tar.bz2"), 10, "", &|_| {}).unwrap_err();
        assert!(e.contains("404"), "{e}");
        assert!(t.elapsed() < Duration::from_secs(5));
        assert_eq!(s.ranges.lock().unwrap().len(), 1);
    }

    #[test]
    fn nothing_listening_reads_as_offline_after_a_few_tries() {
        let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        let d = scratch("offline");
        let url = format!("http://127.0.0.1:{port}/x");
        let e = download(&[url], &d.join("a.tar.bz2"), 10, "", &|_| {}).unwrap_err();
        assert_eq!(e, OFFLINE);
    }

    #[test]
    fn a_runtime_counts_as_installed_only_after_its_marker() {
        let d = scratch("marker");
        let c = catalog();
        let rt = c.runtime_for_host().unwrap();
        let dir = install_dir(&d, &rt.id).join("lib");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(&rt.lib), b"x").unwrap();
        assert!(!runtime_installed(&d, rt), "a library without the marker is a cut-short install");
        std::fs::write(install_dir(&d, &rt.id).join(DONE), &rt.sha256).unwrap();
        assert!(runtime_installed(&d, rt));
        std::fs::write(install_dir(&d, &rt.id).join(DONE), "an older runtime").unwrap();
        assert!(!runtime_installed(&d, rt), "a different build is replaced");
    }

    #[test]
    fn a_damaged_archive_leaves_no_half_installed_model() {
        let d = scratch("atomic");
        let c = catalog();
        let mut m = c.model("moonshine-tiny-en").unwrap().clone();
        let junk = b"not a bzip2 archive".to_vec();
        let s = serve(junk.clone(), 200, None);
        m.urls = vec![s.url.clone()];
        m.bytes = junk.len() as u64;
        m.sha256 = format!("{:x}", Sha256::digest(&junk));
        assert_eq!(install_model(&d, &m, &|_| {}).unwrap_err(), DAMAGED);
        assert!(!install_dir(&d, &m.id).exists());
        assert!(std::fs::read_dir(&d).unwrap().next().is_none(), "nothing left behind");
    }

    #[test]
    fn disk_full_reads_as_disk_full() {
        let full = std::io::Error::from_raw_os_error(if cfg!(windows) { 112 } else { 28 });
        assert!(io_text(&full, Path::new("x")).contains("disk is full"));
        assert!(free_bytes(&std::env::temp_dir()).is_some_and(|b| b > 0));
        assert!(space_needed(100) > 300);
    }

    /// Every model in the catalog through the real runtime: unpack each archive from a local folder
    /// (`POINTCAST_ARCHIVES`, holding the files named in the catalog URLs), install its pinned
    /// files, load it and transcribe the sample recording it ships with.
    /// `POINTCAST_ARCHIVES=... cargo test -- --ignored every_model --nocapture`
    #[test]
    #[ignore]
    fn every_model_transcribes_its_sample_recording() {
        let dir = PathBuf::from(std::env::var("POINTCAST_ARCHIVES").expect("set POINTCAST_ARCHIVES"));
        let root = scratch("every");
        let c = catalog();
        let local = |url: &str| dir.join(url.rsplit('/').next().unwrap());
        let rt = c.runtime_for_host().unwrap();
        unpack_tar_bz2(&local(&rt.urls[0]), &install_dir(&root, &rt.id)).unwrap();
        std::fs::write(install_dir(&root, &rt.id).join(DONE), &rt.sha256).unwrap();
        for m in &c.models {
            let archive = local(&m.urls[0]);
            if !archive.exists() {
                eprintln!("{}: archive not in {}, skipped", m.id, dir.display());
                continue;
            }
            let unpacked = root.join(format!("{}.unpack", m.id));
            unpack_tar_bz2(&archive, &unpacked).unwrap();
            let dest = install_dir(&root, &m.id);
            std::fs::create_dir_all(&dest).unwrap();
            for f in &m.files {
                let src = find_named(&unpacked, &f.name).unwrap();
                check(&src, f.bytes, &f.sha256).unwrap();
                std::fs::rename(&src, dest.join(&f.name)).unwrap();
            }
            assert!(model_installed(&root, m), "{}", m.id);
            let wav = ["0.wav", "en.wav"].iter().find_map(|n| find_named(&unpacked, n)).unwrap();
            let t = Instant::now();
            let eng = crate::voice::stt::load(&root, rt, m, 4).unwrap();
            let loaded = t.elapsed();
            let (rate, ch, pcm) = crate::voice::audio::read_wav(&std::fs::read(wav).unwrap()).unwrap();
            let mut mono = vec![];
            crate::voice::audio::downmix(&pcm, ch, &mut mono);
            let mut at16 = vec![];
            crate::voice::audio::Resampler::new(rate).push(&mono, &mut at16);
            let t = Instant::now();
            let text = eng.transcribe(&at16);
            eprintln!(
                "{}: load {:?}, {:.1} s of audio in {:?}: {text}",
                m.id,
                loaded,
                at16.len() as f64 / 16_000.0,
                t.elapsed()
            );
            assert!(text.split_whitespace().count() >= 3, "{}: {text:?}", m.id);
            drop(eng);
            let _ = std::fs::remove_dir_all(&unpacked);
        }
    }

    /// The real thing, over the network: install the engine and the Tiny model for this computer,
    /// load them and transcribe the sample recording that ships in the model archive.
    /// `cargo test -- --ignored real_install` (about 40 MB).
    #[test]
    #[ignore]
    fn real_install_transcribes_the_sample_recording() {
        let root = scratch("real");
        let c = catalog();
        let m = c.model("moonshine-tiny-en").unwrap();
        // Fetch the archive once: keep its sample WAV, then let install_model finish from it.
        let archive = root.join("sample.tar.bz2");
        download(&m.urls, &archive, m.bytes, &m.sha256, &|_| {}).unwrap();
        let unpacked = root.join("sample");
        unpack_tar_bz2(&archive, &unpacked).unwrap();
        let wav = std::fs::read(find_named(&unpacked, "0.wav").unwrap()).unwrap();
        std::fs::rename(&archive, root.join(format!("{}.partial", m.id))).unwrap();
        let _ = std::fs::remove_dir_all(&unpacked);
        let rt = c.runtime_for_host().unwrap();
        install_runtime(&root, rt, &|_| {}).unwrap();
        install_model(&root, m, &|_| {}).unwrap();
        assert!(runtime_installed(&root, rt) && model_installed(&root, m));
        let eng = crate::voice::stt::load(&root, rt, m, 2).unwrap();
        let (rate, ch, pcm) = crate::voice::audio::read_wav(&wav).unwrap();
        let mut mono = vec![];
        crate::voice::audio::downmix(&pcm, ch, &mut mono);
        let mut at16 = vec![];
        crate::voice::audio::Resampler::new(rate).push(&mono, &mut at16);
        let text = eng.transcribe(&at16);
        eprintln!("transcript: {text}");
        assert!(text.split_whitespace().count() >= 3, "{text:?}");
    }
}
