//! Speech models and the sherpa-onnx runtime, downloaded on first use over HTTPS.
//! Archives unpack into `<data dir>/speech/<id>/`. Hashes are checked when the catalog
//! lists them; otherwise we require the expected file names after unpack.
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

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
    dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("Pointcast")
        .join("speech")
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

pub fn runtime_installed(root: &Path, e: &RuntimeEntry) -> bool {
    find_lib(root, e).is_some()
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

pub fn download(url: &str, dest: &Path, progress: &dyn Fn(Progress)) -> Result<(), String> {
    if !url.starts_with("https://") {
        return Err("only https downloads".into());
    }
    if let Some(d) = dest.parent() {
        std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
    }
    let tmp = dest.with_extension("partial");
    let mut already = std::fs::metadata(&tmp).map(|m| m.len()).unwrap_or(0);
    let client = reqwest::blocking::Client::builder()
        .user_agent("pointcast/0.1")
        .build()
        .map_err(|e| e.to_string())?;
    let mut req = client.get(url);
    if already > 0 {
        req = req.header(reqwest::header::RANGE, format!("bytes={already}-"));
    }
    let mut resp = req.send().map_err(|e| e.to_string())?;
    let status = resp.status();
    if already > 0 && status == reqwest::StatusCode::OK {
        // Server ignored Range: start over.
        already = 0;
        let _ = std::fs::remove_file(&tmp);
    } else if !status.is_success() && status != reqwest::StatusCode::PARTIAL_CONTENT {
        return Err(format!("download HTTP {status}"));
    }
    let total = resp
        .headers()
        .get(reqwest::header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse::<u64>().ok())
        .map(|n| n + already);
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(already > 0)
        .write(true)
        .truncate(already == 0)
        .open(&tmp)
        .map_err(|e| e.to_string())?;
    let mut received = already;
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = std::io::Read::read(&mut resp, &mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n]).map_err(|e| e.to_string())?;
        received += n as u64;
        progress(Progress::Downloading {
            received,
            total,
        });
    }
    file.flush().map_err(|e| e.to_string())?;
    drop(file);
    std::fs::rename(&tmp, dest).map_err(|e| e.to_string())?;
    Ok(())
}

pub fn unpack_tar_bz2(archive: &Path, dest: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dest).map_err(|e| e.to_string())?;
    let f = std::fs::File::open(archive).map_err(|e| e.to_string())?;
    let dec = bzip2::read::BzDecoder::new(f);
    let mut ar = tar::Archive::new(dec);
    ar.unpack(dest).map_err(|e| e.to_string())
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
    let url = e.urls.first().ok_or("no download url")?;
    let archive = root.join(format!("{}.tar.bz2", e.id));
    download(url, &archive, progress)?;
    progress(Progress::Unpacking);
    let staging = root.join(format!("{}.staging", e.id));
    let _ = std::fs::remove_dir_all(&staging);
    unpack_tar_bz2(&archive, &staging)?;
    progress(Progress::Verifying);
    let dest = install_dir(root, &e.id);
    let _ = std::fs::remove_dir_all(&dest);
    std::fs::create_dir_all(&dest).map_err(|e| e.to_string())?;
    for f in &e.files {
        let src = find_named(&staging, &f.name).ok_or_else(|| format!("{} missing from archive", f.name))?;
        if !f.sha256.is_empty() {
            let h = sha256_file(&src).map_err(|e| e.to_string())?;
            if h != f.sha256 {
                return Err(format!("{} hash mismatch", f.name));
            }
        }
        std::fs::copy(&src, dest.join(&f.name)).map_err(|e| e.to_string())?;
    }
    let _ = std::fs::remove_dir_all(&staging);
    let _ = std::fs::remove_file(&archive);
    progress(Progress::Ready);
    Ok(dest)
}

pub fn install_runtime(
    root: &Path,
    e: &RuntimeEntry,
    progress: &dyn Fn(Progress),
) -> Result<PathBuf, String> {
    if let Some(p) = find_lib(root, e) {
        progress(Progress::Ready);
        return Ok(p);
    }
    let url = e.urls.first().ok_or("no download url")?;
    let archive = root.join(format!("{}.tar.bz2", e.id));
    download(url, &archive, progress)?;
    progress(Progress::Unpacking);
    let dest = install_dir(root, &e.id);
    let _ = std::fs::remove_dir_all(&dest);
    unpack_tar_bz2(&archive, &dest)?;
    let lib = find_lib(root, e).ok_or_else(|| format!("{} not in archive", e.lib))?;
    let _ = std::fs::remove_file(&archive);
    progress(Progress::Ready);
    Ok(lib)
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
}
