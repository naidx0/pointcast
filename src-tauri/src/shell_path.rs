//! PATH as a terminal sees it. An app opened from the macOS Dock or Finder is given only /usr/bin:/bin:/usr/sbin:/sbin,
//! so every AI app installed with npm (a Node script that starts `node`) fails to start, and Homebrew's apps are not
//! found. At launch Pointcast asks the person's own login shell for its PATH, once, and adds the usual folders after
//! it, so the AI apps run the same as they do in Terminal. Windows gives apps the full PATH already.

use std::path::PathBuf;

/// What the shell printed around its PATH, so a greeting or a prompt from the person's shell files is not mistaken
/// for a folder.
const MARK: &str = "__POINTCAST_PATH__";

/// The PATH between the marks in what the shell printed.
pub fn between_marks(said: &str) -> Option<String> {
    let start = said.find(MARK)? + MARK.len();
    let end = said[start..].find(MARK)? + start;
    Some(said[start..end].to_owned()).filter(|p| !p.trim().is_empty())
}

/// The folders AI apps are usually in, newest Node first: Homebrew's, npm's own prefix in the home folder, Volta,
/// Bun and nvm's newest Node.
pub fn usual() -> Vec<PathBuf> {
    let mut all: Vec<PathBuf> = ["/opt/homebrew/bin", "/opt/homebrew/sbin", "/usr/local/bin"].map(PathBuf::from).to_vec();
    if let Some(home) = dirs::home_dir() {
        all.extend([".local/bin", ".npm-global/bin", ".volta/bin", ".bun/bin", ".cargo/bin"].map(|d| home.join(d)));
        let nvm = home.join(".nvm/versions/node");
        let mut nodes: Vec<PathBuf> = std::fs::read_dir(&nvm).map(|d| d.flatten().map(|e| e.path().join("bin")).collect()).unwrap_or_default();
        nodes.sort();
        all.extend(nodes.into_iter().rev());
    }
    all
}

/// `first`, then `then`, each folder once, in order.
pub fn merged(first: &str, then: &str, more: &[PathBuf]) -> std::ffi::OsString {
    let mut seen = std::collections::HashSet::new();
    let all = std::env::split_paths(first).chain(std::env::split_paths(then)).chain(more.iter().cloned());
    let kept: Vec<PathBuf> = all.filter(|p| !p.as_os_str().is_empty() && seen.insert(p.clone())).collect();
    std::env::join_paths(kept).unwrap_or_default()
}

/// The login shell's PATH, within 4 seconds; none when it did not say.
#[cfg(unix)]
fn from_shell() -> Option<String> {
    use std::io::Read;
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};
    let shell = std::env::var("SHELL").ok().filter(|s| !s.is_empty()).unwrap_or_else(|| "/bin/zsh".to_owned());
    let mut child = Command::new(shell)
        .args(["-ilc", &format!("printf '%s' \"{MARK}${{PATH}}{MARK}\"")])
        .env("TERM", "dumb")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let t = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if t.elapsed() < Duration::from_secs(4) => std::thread::sleep(Duration::from_millis(20)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
    let mut said = String::new();
    child.stdout.take()?.read_to_string(&mut said).ok()?;
    between_marks(&said)
}

/// Widen this process's PATH before anything starts a thread or a child. Unix only.
pub fn widen() {
    #[cfg(unix)]
    {
        let now = std::env::var("PATH").unwrap_or_default();
        let shell = from_shell().unwrap_or_default();
        let path = merged(&shell, &now, &usual());
        if !path.is_empty() {
            std::env::set_var("PATH", path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_what_is_between_the_marks_is_the_path() {
        let said = format!("Welcome back!\n{MARK}/a/bin:/b/bin{MARK}\nbye");
        assert_eq!(between_marks(&said).as_deref(), Some("/a/bin:/b/bin"));
        assert_eq!(between_marks("no marks at all"), None);
        assert_eq!(between_marks(&format!("{MARK}{MARK}")), None, "an empty PATH is no answer");
    }

    #[test]
    fn the_shells_folders_come_first_and_each_folder_once() {
        if cfg!(windows) {
            return;
        }
        let p = merged("/s/bin:/usr/bin", "/usr/bin:/bin", &[PathBuf::from("/opt/homebrew/bin"), PathBuf::from("/s/bin")]);
        assert_eq!(p.to_string_lossy(), "/s/bin:/usr/bin:/bin:/opt/homebrew/bin");
    }

    #[test]
    fn the_usual_folders_include_homebrew_and_the_home_folders_npm() {
        let u = usual();
        assert!(u.contains(&PathBuf::from("/opt/homebrew/bin")));
        if let Some(home) = dirs::home_dir() {
            assert!(u.contains(&home.join(".local/bin")));
        }
    }

    #[cfg(unix)]
    #[test]
    fn the_login_shell_says_its_path() {
        // The shell on the machine running the tests: it may be missing in a bare container, where none is fine.
        if std::path::Path::new(&std::env::var("SHELL").unwrap_or_default()).exists() {
            let p = from_shell().expect("the login shell's PATH");
            assert!(p.contains("/usr/bin") || p.contains("/bin"), "{p}");
        }
    }
}

#[cfg(all(test, unix))]
mod on_this_computer {
    /// Under a Dock-like PATH, after `widen`, an npm-installed AI app starts. Run by hand: `env -i HOME=$HOME
    /// PATH=/usr/bin:/bin SHELL=$SHELL cargo test --lib dock_started -- --ignored`.
    #[test]
    #[ignore]
    fn dock_started_apps_run_after_widening() {
        let before = std::env::var("PATH").unwrap_or_default();
        super::widen();
        let after = std::env::var("PATH").unwrap_or_default();
        println!("before: {before}\nafter has node: {}", after.split(':').any(|d| std::path::Path::new(d).join("node").exists()));
        for name in ["gemini", "copilot", "claude", "codex"] {
            let exe = crate::assist::detect::on_path(name);
            let v = exe.as_deref().map(crate::assist::detect::version_of);
            println!("{name}: {v:?}");
            if let Some(v) = v {
                assert!(v.is_ok(), "{name} did not start: {v:?}");
            }
        }
    }
}
