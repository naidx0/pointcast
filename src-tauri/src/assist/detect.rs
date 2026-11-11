//! Finding the vendor's tool on this computer: where `codex` is, which version it says it is, and whether that is
//! one Pointcast was tested with. Looking spends nothing: no question is sent and no account is touched.

use super::child::Launch;
use super::{Health, State};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// The oldest Codex whose app-server Pointcast was written against.
pub const OLDEST: (u32, u32, u32) = (0, 164, 0);

/// Names another program to run in place of `codex`: a stand-in, in tests and in development.
pub const OVERRIDE: &str = "POINTCAST_CODEX";

pub const GOES_TO: &str = "OpenAI, through your ChatGPT plan";

#[derive(Debug, Clone, PartialEq)]
pub struct Found {
    pub exe: PathBuf,
    pub version: (u32, u32, u32),
}

impl Found {
    /// How the app-server is started: a constant, with nothing of the person's on the command line.
    pub fn launch(&self) -> Launch {
        Launch { exe: self.exe.clone(), args: vec!["app-server".into()], env: Vec::new() }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Missing {
    NotInstalled,
    /// Found, and older than `OLDEST`: the message says so plainly.
    TooOld(String),
    /// Found, and it would not say which version it is.
    Silent(String),
}

/// The version in what `--version` printed ("codex-cli 0.164.0").
pub fn version_in(said: &str) -> Option<(u32, u32, u32)> {
    said.split_whitespace().find_map(|w| {
        let core = w.trim_start_matches('v').split(['-', '+']).next()?;
        let mut n = core.split('.').map(|p| p.parse::<u32>().ok());
        match (n.next().flatten(), n.next().flatten(), n.next().flatten(), n.next()) {
            (Some(a), Some(b), Some(c), None) => Some((a, b, c)),
            _ => None,
        }
    })
}

/// Whether Pointcast works with this version, or the plain words for why not.
pub fn judge(v: (u32, u32, u32)) -> Result<(), String> {
    if v >= OLDEST {
        return Ok(());
    }
    Err(format!(
        "Pointcast needs Codex {}.{}.{} or newer, and this computer has {}.{}.{}. Update Codex, then try again.",
        OLDEST.0, OLDEST.1, OLDEST.2, v.0, v.1, v.2
    ))
}

/// The endings Windows runs a bare name with, of those a program can be started by.
pub fn endings(pathext: Option<&str>) -> Vec<String> {
    pathext
        .unwrap_or(".COM;.EXE;.BAT;.CMD")
        .split(';')
        .map(|e| e.trim().to_ascii_lowercase())
        .filter(|e| matches!(e.as_str(), ".com" | ".exe" | ".bat" | ".cmd"))
        .collect()
}

fn runnable(p: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        p.metadata().map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0).unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        p.is_file()
    }
}

/// The first `name` in `dirs`. With `endings` (Windows) only a name with one of them counts, in their order: npm
/// leaves a shell script called plain `codex` beside `codex.cmd`, and it cannot be started.
pub fn find_in(name: &str, dirs: impl IntoIterator<Item = PathBuf>, endings: &[String]) -> Option<PathBuf> {
    for d in dirs {
        if d.as_os_str().is_empty() {
            continue;
        }
        if endings.is_empty() {
            let p = d.join(name);
            if runnable(&p) {
                return Some(p);
            }
        }
        for e in endings {
            let p = d.join(format!("{name}{e}"));
            if p.is_file() {
                return Some(p);
            }
        }
    }
    None
}

/// Where the tool is usually installed, for when PATH does not say. An app started from the macOS Dock or Finder
/// does not get the PATH a login shell builds, so Homebrew's and npm's folders are looked in by name.
fn usual() -> Vec<PathBuf> {
    let mut all = Vec::new();
    if cfg!(unix) {
        all.extend(["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin"].map(PathBuf::from));
        if let Some(home) = dirs::home_dir() {
            all.extend([".local/bin", ".npm-global/bin", ".volta/bin", ".bun/bin"].map(|d| home.join(d)));
        }
    } else if let Some(roaming) = dirs::config_dir() {
        all.push(roaming.join("npm"));
    }
    all
}

/// The program behind an npm shim: `codex.cmd` (or the script npm links on Unix) starts node, which starts the
/// real program from a package named for the platform. Starting that one directly skips the node hop.
pub fn native_behind(shim: &Path) -> Option<PathBuf> {
    let name = if cfg!(windows) { "codex.exe" } else { "codex" };
    let resolved = std::fs::canonicalize(shim).ok()?;
    // Beside a Windows shim, or up from `node_modules/@openai/codex/bin/codex.js` on Unix. Seen on Windows with
    // 0.164.0: `@openai/codex/node_modules/@openai/codex-win32-x64/vendor/x86_64-pc-windows-msvc/bin/codex.exe`
    // under npm's `node_modules`. The layouts on macOS and Linux are UNCONFIRMED.
    let mut scopes = vec![resolved.parent()?.join("node_modules").join("@openai")];
    scopes.extend(resolved.ancestors().filter(|a| a.file_name().is_some_and(|n| n == "@openai")).map(Path::to_owned));
    for scope in scopes.clone() {
        scopes.push(scope.join("codex").join("node_modules").join("@openai"));
    }
    for scope in scopes {
        for package in list(&scope).into_iter().filter(|p| p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("codex-"))) {
            for triple in list(&package.join("vendor")) {
                for inner in ["bin", "codex"] {
                    let p = triple.join(inner).join(name);
                    if runnable(&p) {
                        return Some(p);
                    }
                }
            }
        }
    }
    None
}

fn list(dir: &Path) -> Vec<PathBuf> {
    let mut all: Vec<PathBuf> = std::fs::read_dir(dir).map(|d| d.flatten().map(|e| e.path()).collect()).unwrap_or_default();
    all.sort();
    all
}

/// Where `codex` is: the override, else PATH and the usual folders, and the program behind a shim when there is one.
pub fn locate() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os(OVERRIDE).filter(|p| !p.is_empty()) {
        return Some(PathBuf::from(p));
    }
    let path = std::env::var_os("PATH").unwrap_or_default();
    let ends = if cfg!(windows) { endings(std::env::var("PATHEXT").ok().as_deref()) } else { Vec::new() };
    let found = find_in("codex", std::env::split_paths(&path).chain(usual()), &ends)?;
    Some(native_behind(&found).unwrap_or(found))
}

/// What `exe --version` prints.
pub fn version_of(exe: &Path) -> Result<String, String> {
    let mut cmd = Command::new(exe);
    cmd.arg("--version").stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let mut child = cmd.spawn().map_err(|e| e.to_string())?;
    let t = Instant::now();
    while matches!(child.try_wait(), Ok(None)) {
        if t.elapsed() > Duration::from_secs(10) {
            let _ = child.kill();
            let _ = child.wait();
            return Err("it did not answer".into());
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    let mut said = String::new();
    if let Some(mut out) = child.stdout.take() {
        let _ = out.read_to_string(&mut said);
    }
    Ok(said)
}

pub fn codex() -> Result<Found, Missing> {
    let exe = locate().ok_or(Missing::NotInstalled)?;
    let said = version_of(&exe).map_err(Missing::Silent)?;
    let version = version_in(&said).ok_or_else(|| Missing::Silent("it named no version".into()))?;
    judge(version).map_err(Missing::TooOld)?;
    Ok(Found { exe, version })
}

pub fn health() -> Health {
    health_of(&codex())
}

pub fn health_of(found: &Result<Found, Missing>) -> Health {
    let (state, line) = match found {
        Ok(f) => (State::Ready, format!("Codex {}.{}.{} is installed. Whether you are signed in is checked when you ask.", f.version.0, f.version.1, f.version.2)),
        Err(Missing::NotInstalled) => (State::NotInstalled, "Codex is not installed on this computer.".into()),
        Err(Missing::TooOld(why)) => (State::TooOld, why.clone()),
        Err(Missing::Silent(why)) => (State::Unknown, format!("Codex is installed, but Pointcast could not read its version: {why}.")),
    };
    Health { state, line, goes_to: GOES_TO }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assist::tests::scratch;

    /// An empty file a program could be started from.
    fn program(p: &Path) {
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, b"").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
    }

    #[test]
    fn the_version_is_read_from_what_the_tool_prints() {
        assert_eq!(version_in("codex-cli 0.164.0\n"), Some((0, 164, 0)));
        assert_eq!(version_in("codex-cli 1.2.3-alpha.4"), Some((1, 2, 3)));
        assert_eq!(version_in("v0.200.1"), Some((0, 200, 1)));
        assert_eq!(version_in("codex-cli"), None);
        assert_eq!(version_in("codex 1.2"), None);
    }

    #[test]
    fn a_version_below_the_tested_one_is_refused_in_plain_words() {
        assert_eq!(judge(OLDEST), Ok(()));
        assert_eq!(judge((0, 200, 1)), Ok(()));
        assert_eq!(judge((1, 0, 0)), Ok(()));
        let why = judge((0, 163, 9)).unwrap_err();
        assert!(why.contains("0.164.0") && why.contains("0.163.9") && why.contains("Update Codex"), "{why}");
    }

    #[test]
    fn on_windows_only_a_name_with_a_runnable_ending_counts_and_the_first_folder_wins() {
        let d = scratch("detect-win");
        let (a, b) = (d.join("a"), d.join("b"));
        program(&a.join("codex"));
        program(&a.join("codex.ps1"));
        program(&b.join("codex.cmd"));
        program(&b.join("codex.exe"));
        let ends = endings(Some(".COM;.EXE;.BAT;.CMD;.VBS;.PS1"));
        assert_eq!(ends, [".com", ".exe", ".bat", ".cmd"]);
        assert_eq!(find_in("codex", [a.clone(), b.clone()], &ends), Some(b.join("codex.exe")));
        assert_eq!(find_in("codex", [a.clone()], &ends), None, "the bare script npm leaves is not a program there");
        assert_eq!(find_in("codex", [PathBuf::new(), b.clone()], &endings(Some(".CMD"))), Some(b.join("codex.cmd")));
    }

    #[cfg(unix)]
    #[test]
    fn on_unix_the_bare_name_counts_when_it_can_be_run() {
        let d = scratch("detect-unix");
        let (a, b) = (d.join("a"), d.join("b"));
        std::fs::create_dir_all(&a).unwrap();
        std::fs::write(a.join("codex"), b"").unwrap();
        program(&b.join("codex"));
        assert_eq!(find_in("codex", [a, b.clone()], &[]), Some(b.join("codex")));
    }

    #[test]
    fn the_program_behind_an_npm_shim_is_preferred_and_a_plain_install_is_left_as_it_is() {
        let d = scratch("detect-shim");
        let shim = d.join(if cfg!(windows) { "codex.cmd" } else { "codex" });
        program(&shim);
        assert_eq!(native_behind(&shim), None, "nothing behind it: the caller keeps the shim");
        let name = if cfg!(windows) { "codex.exe" } else { "codex" };
        let native = d.join("node_modules/@openai/codex-test-x64/vendor/some-triple/bin").join(name);
        program(&native);
        program(&d.join("node_modules/@openai/codex/bin/codex.js"));
        assert_eq!(native_behind(&shim), Some(std::fs::canonicalize(&native).unwrap()));
    }

    #[test]
    fn health_says_each_state_in_one_line_and_where_a_question_goes() {
        let ok = health_of(&Ok(Found { exe: "codex".into(), version: (0, 164, 0) }));
        assert_eq!((ok.state, ok.goes_to), (State::Ready, GOES_TO));
        assert!(ok.line.contains("0.164.0"));
        assert_eq!(health_of(&Err(Missing::NotInstalled)).state, State::NotInstalled);
        let old = health_of(&Err(Missing::TooOld(judge((0, 100, 0)).unwrap_err())));
        assert_eq!(old.state, State::TooOld);
        assert!(old.line.contains("0.100.0"));
        assert_eq!(health_of(&Err(Missing::Silent("it did not answer".into()))).state, State::Unknown);
    }

    #[test]
    fn the_app_server_is_started_with_one_constant_argument() {
        let l = Found { exe: "codex".into(), version: OLDEST }.launch();
        assert_eq!((l.args, l.env), (vec!["app-server".to_owned()], Vec::new()));
    }
}
