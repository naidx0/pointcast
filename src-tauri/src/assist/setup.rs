//! One-click setup for the AIs Pointcast talks to through their own apps: install, update and sign in, done for the
//! person instead of a command to copy. Install and update run the vendor's own package (npm, or Homebrew where the
//! app came from Homebrew) out of sight and wait for it. Sign-in is the vendor's own command, opened in a terminal
//! window of its own, because every one of them may ask the person something there (Claude Code asks for a pasted
//! code); the settings pane watches until the app says it is signed in. Pointcast never sees a password or a token.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use super::detect;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Job {
    Install,
    Update,
    SignIn,
}

impl Job {
    pub fn parse(s: &str) -> Option<Job> {
        match s {
            "install" => Some(Job::Install),
            "update" => Some(Job::Update),
            "sign_in" => Some(Job::SignIn),
            _ => None,
        }
    }
}

/// What a route's app is, for setting it up.
struct App {
    id: &'static str,
    name: &'static str,
    program: &'static str,
    npm: &'static str,
    /// The Homebrew formula or cask (`cask:` first), when Homebrew has it.
    brew: Option<&'static str>,
    /// The vendor's own sign-in, after the program. None: the app signs in on its first run.
    sign_in: Option<&'static [&'static str]>,
}

const APPS: &[App] = &[
    App { id: "codex", name: "Codex", program: "codex", npm: "@openai/codex", brew: Some("cask:codex"), sign_in: Some(&["login"]) },
    App { id: "claude-code", name: "Claude Code", program: "claude", npm: "@anthropic-ai/claude-code", brew: Some("cask:claude-code"), sign_in: Some(&["auth", "login"]) },
    App { id: "gemini-cli", name: "Gemini CLI", program: "gemini", npm: "@google/gemini-cli", brew: Some("gemini-cli"), sign_in: None },
    App { id: "copilot", name: "GitHub Copilot CLI", program: "copilot", npm: "@github/copilot", brew: None, sign_in: Some(&["login"]) },
    App { id: "opencode", name: "OpenCode", program: "opencode", npm: "opencode-ai", brew: None, sign_in: Some(&["auth", "login"]) },
];

fn app(id: &str) -> Result<&'static App, String> {
    APPS.iter().find(|a| a.id == id).ok_or_else(|| "Pointcast cannot set that AI up for you.".to_owned())
}

/// How long an install or update may take before Pointcast stops waiting.
const PATIENCE: Duration = Duration::from_secs(600);

/// A command to run: the program found on this computer and what it is told.
#[derive(Debug, Clone, PartialEq)]
pub struct Plan {
    pub program: PathBuf,
    pub args: Vec<String>,
}

impl Plan {
    /// The command as a person would type it, for the terminal window and for an error.
    pub fn typed(&self) -> String {
        let name = self.program.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        std::iter::once(name).chain(self.args.iter().cloned()).collect::<Vec<_>>().join(" ")
    }
}

/// Folders a package manager's node and npm may be in that a Dock-started app is not told: nvm's newest node.
fn more_dirs() -> Vec<PathBuf> {
    let mut all = Vec::new();
    if let Some(home) = dirs::home_dir() {
        let nvm = home.join(".nvm").join("versions").join("node");
        let mut found: Vec<PathBuf> = std::fs::read_dir(&nvm).map(|d| d.flatten().map(|e| e.path().join("bin")).collect()).unwrap_or_default();
        found.sort();
        all.extend(found.into_iter().rev());
    }
    if cfg!(windows) {
        if let Some(pf) = std::env::var_os("ProgramFiles") {
            all.push(PathBuf::from(pf).join("nodejs"));
        }
    }
    all
}

/// Where `name` is: PATH, the usual folders, then nvm's.
fn find(name: &str) -> Option<PathBuf> {
    detect::on_path(name).or_else(|| {
        let ends = if cfg!(windows) { detect::endings(std::env::var("PATHEXT").ok().as_deref()) } else { Vec::new() };
        detect::find_in(name, more_dirs(), &ends)
    })
}

/// Whether an installed program came from Homebrew: it resolves into Homebrew's own folders.
fn from_brew(exe: &Path) -> bool {
    let real = std::fs::canonicalize(exe).unwrap_or_else(|_| exe.to_owned());
    let s = real.to_string_lossy();
    s.contains("/Cellar/") || s.contains("/Caskroom/") || s.starts_with("/opt/homebrew/")
}

fn brew_plan(brew: &Path, what: &str, job: Job) -> Plan {
    let (cask, name) = what.strip_prefix("cask:").map_or((false, what), |n| (true, n));
    let mut args = vec![if job == Job::Update { "upgrade" } else { "install" }.to_owned()];
    if cask {
        args.push("--cask".to_owned());
    }
    args.push(name.to_owned());
    Plan { program: brew.to_owned(), args }
}

/// The one command that installs or updates `id`, from what is on this computer: `found` is the app as installed now,
/// `npm` and `brew` the package managers found. Homebrew updates what Homebrew installed; npm does the rest.
pub fn plan_of(id: &str, job: Job, found: Option<&Path>, npm: Option<&Path>, brew: Option<&Path>) -> Result<Plan, String> {
    let a = app(id)?;
    if job == Job::SignIn {
        let exe = found.ok_or_else(|| format!("{} is not installed yet. Install it first.", a.name))?;
        let args = a.sign_in.unwrap_or(&[]).iter().map(|s| (*s).to_owned()).collect();
        return Ok(Plan { program: exe.to_owned(), args });
    }
    if let (Some(exe), Some(brew), Some(what)) = (found, brew, a.brew) {
        if from_brew(exe) {
            return Ok(brew_plan(brew, what, job));
        }
    }
    // Claude Code updates itself, however it was installed.
    if let (Job::Update, Some(exe), "claude-code") = (job, found, id) {
        return Ok(Plan { program: exe.to_owned(), args: vec!["update".to_owned()] });
    }
    if let Some(npm) = npm {
        return Ok(Plan { program: npm.to_owned(), args: vec!["install".to_owned(), "-g".to_owned(), format!("{}@latest", a.npm)] });
    }
    if let (Some(brew), Some(what)) = (brew, a.brew) {
        return Ok(brew_plan(brew, what, job));
    }
    Err(format!("Pointcast needs Node.js to {} {}. Get it from nodejs.org, then try again.", if job == Job::Update { "update" } else { "install" }, a.name))
}

/// `plan_of` with what is found on this computer now.
pub fn plan(id: &str, job: Job) -> Result<Plan, String> {
    let a = app(id)?;
    let (found, npm) = (find(a.program), find("npm"));
    let brew = if cfg!(windows) { None } else { find("brew") };
    plan_of(id, job, found.as_deref(), npm.as_deref(), brew.as_deref())
}

/// PATH for a child: the folder of the program itself (npm needs its node beside it), then PATH, then the usual
/// folders, so a Dock-started app runs the tools as a terminal would.
fn path_for(program: &Path) -> std::ffi::OsString {
    let mut dirs: Vec<PathBuf> = program.parent().map(Path::to_owned).into_iter().collect();
    dirs.extend(std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()));
    if cfg!(unix) {
        dirs.extend(["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin", "/bin"].map(PathBuf::from));
    }
    dirs.extend(more_dirs());
    std::env::join_paths(dirs).unwrap_or_default()
}

/// Run an install or update out of sight and wait for it. An error carries the last lines it printed.
pub fn run(plan: &Plan) -> Result<(), String> {
    let mut cmd = Command::new(&plan.program);
    cmd.args(&plan.args).env("PATH", path_for(&plan.program)).env("HOMEBREW_NO_AUTO_UPDATE", "1").env("NONINTERACTIVE", "1");
    cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    let mut child = cmd.spawn().map_err(|e| format!("{} did not start: {e}", plan.typed()))?;
    // Both pipes are drained on threads so a chatty installer never blocks on a full pipe.
    let drain = |r: Option<Box<dyn Read + Send>>| {
        std::thread::spawn(move || {
            let mut s = String::new();
            if let Some(mut r) = r {
                let _ = r.read_to_string(&mut s);
            }
            s
        })
    };
    let out = drain(child.stdout.take().map(|r| Box::new(r) as Box<dyn Read + Send>));
    let err = drain(child.stderr.take().map(|r| Box::new(r) as Box<dyn Read + Send>));
    let t = Instant::now();
    let status = loop {
        if let Some(s) = child.try_wait().map_err(|e| e.to_string())? {
            break s;
        }
        if t.elapsed() > PATIENCE {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!("{} took more than 10 minutes, so Pointcast stopped it.", plan.typed()));
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    if status.success() {
        return Ok(());
    }
    let said = format!("{}\n{}", out.join().unwrap_or_default(), err.join().unwrap_or_default());
    Err(failed(&plan.typed(), &said))
}

/// The error for a failed install: the command and its last meaningful line.
pub fn failed(typed: &str, said: &str) -> String {
    let last = said.lines().map(str::trim).filter(|l| !l.is_empty()).filter(|l| !l.starts_with("npm notice")).last().unwrap_or("");
    if last.contains("EACCES") {
        return format!("{typed} needs permission to write where Node.js keeps its apps. Run it in a terminal with sudo, then Refresh.");
    }
    if last.is_empty() {
        format!("{typed} did not finish.")
    } else {
        format!("{typed} did not finish: {last}")
    }
}

/// One shell word, quoted for sh.
fn sh_quote(s: &str) -> String {
    if !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || "/._-:@+=".contains(c)) {
        s.to_owned()
    } else {
        format!("'{}'", s.replace('\'', "'\\''"))
    }
}

/// The line typed into the terminal window: PATH as Pointcast found it, then the command itself.
pub fn terminal_line(plan: &Plan) -> String {
    let path = path_for(&plan.program).to_string_lossy().into_owned();
    let cmd = std::iter::once(plan.program.to_string_lossy().into_owned()).chain(plan.args.iter().cloned()).map(|s| sh_quote(&s)).collect::<Vec<_>>().join(" ");
    format!("clear; export PATH={}; {cmd}", sh_quote(&path))
}

/// Open the vendor's sign-in in a terminal window of its own, where it can ask what it needs.
pub fn open_in_terminal(plan: &Plan) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        let line = terminal_line(plan).replace('\\', "\\\\").replace('"', "\\\"");
        let script = format!("tell application \"Terminal\"\nactivate\ndo script \"{line}\"\nend tell");
        let ok = Command::new("osascript").args(["-e", &script]).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).status().map_err(|e| e.to_string())?;
        return if ok.success() { Ok(()) } else { Err("Terminal did not open. Allow Pointcast to control Terminal in System Settings, Privacy & Security, Automation.".to_owned()) };
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let mut cmd = Command::new("cmd");
        // `start` gives the sign-in a console window of its own, which stays open with /k.
        cmd.raw_arg(format!("/c start \"Pointcast sign-in\" cmd /k \"\"{}\" {}\"", plan.program.display(), plan.args.join(" ")));
        cmd.creation_flags(0x0800_0000);
        return cmd.status().map_err(|e| e.to_string()).and_then(|s| if s.success() { Ok(()) } else { Err("A terminal window did not open.".to_owned()) });
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let line = terminal_line(plan);
        for term in ["x-terminal-emulator", "gnome-terminal", "konsole", "xterm"] {
            let args: Vec<&str> = if term == "gnome-terminal" { vec!["--", "sh", "-c", &line] } else { vec!["-e", "sh", "-c", &line] };
            if Command::new(term).args(&args).spawn().is_ok() {
                return Ok(());
            }
        }
        Err(format!("No terminal was found. Run {} in one, then Refresh.", plan.typed()))
    }
}

/// Do `job` for `id`: install and update wait for the package manager; sign-in opens its window and returns.
pub fn setup(id: &str, job: Job) -> Result<(), String> {
    let p = plan(id, job)?;
    match job {
        Job::SignIn => open_in_terminal(&p),
        Job::Install | Job::Update => run(&p).or_else(|e| match user_prefix(&p, &e) {
            // npm's own folder belongs to the system: the app goes to the person's own folder instead, where Pointcast
            // looks for it, and no password is asked.
            Some(mine) => run(&mine),
            None => Err(e),
        }),
    }
}

/// The same npm install into `~/.local`, when it failed only for permission on npm's own folder.
pub fn user_prefix(p: &Plan, error: &str) -> Option<Plan> {
    let npm = p.program.file_stem().is_some_and(|s| s == "npm");
    if !npm || !error.contains("needs permission") || cfg!(windows) {
        return None;
    }
    let home = dirs::home_dir()?;
    let mut args = p.args.clone();
    args.extend(["--prefix".to_owned(), home.join(".local").to_string_lossy().into_owned()]);
    Some(Plan { program: p.program.clone(), args })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> PathBuf {
        PathBuf::from(s)
    }

    #[test]
    fn an_app_from_homebrew_is_updated_by_homebrew() {
        let plan = plan_of("codex", Job::Update, Some(&p("/opt/homebrew/bin/codex")), Some(&p("/usr/local/bin/npm")), Some(&p("/opt/homebrew/bin/brew"))).unwrap();
        assert_eq!(plan.args, ["upgrade", "--cask", "codex"]);
        assert_eq!(plan.typed(), "brew upgrade --cask codex");
    }

    #[test]
    fn an_app_from_npm_is_updated_by_npm_to_the_latest() {
        let plan = plan_of("codex", Job::Update, Some(&p("/home/x/.local/bin/codex")), Some(&p("/home/x/.local/bin/npm")), None).unwrap();
        assert_eq!(plan.typed(), "npm install -g @openai/codex@latest");
    }

    #[test]
    fn claude_code_updates_itself() {
        let plan = plan_of("claude-code", Job::Update, Some(&p("/home/x/.local/bin/claude")), Some(&p("/usr/bin/npm")), None).unwrap();
        assert_eq!(plan.typed(), "claude update");
    }

    #[test]
    fn a_missing_app_installs_with_npm_first_and_homebrew_without_node() {
        assert_eq!(plan_of("gemini-cli", Job::Install, None, Some(&p("/usr/bin/npm")), Some(&p("/opt/homebrew/bin/brew"))).unwrap().typed(), "npm install -g @google/gemini-cli@latest");
        assert_eq!(plan_of("gemini-cli", Job::Install, None, None, Some(&p("/opt/homebrew/bin/brew"))).unwrap().typed(), "brew install gemini-cli");
        assert!(plan_of("opencode", Job::Install, None, None, Some(&p("/opt/homebrew/bin/brew"))).unwrap_err().contains("Node.js"));
    }

    #[test]
    fn sign_in_is_the_vendors_own_command() {
        assert_eq!(plan_of("claude-code", Job::SignIn, Some(&p("/x/claude")), None, None).unwrap().typed(), "claude auth login");
        assert_eq!(plan_of("copilot", Job::SignIn, Some(&p("/x/copilot")), None, None).unwrap().typed(), "copilot login");
        assert_eq!(plan_of("gemini-cli", Job::SignIn, Some(&p("/x/gemini")), None, None).unwrap().typed(), "gemini");
        assert!(plan_of("claude-code", Job::SignIn, None, None, None).is_err());
        assert!(plan_of("lmstudio", Job::Install, None, None, None).is_err());
    }

    #[test]
    fn the_terminal_line_quotes_what_needs_it() {
        let line = terminal_line(&Plan { program: p("/Users/a b/.local/bin/claude"), args: vec!["auth".into(), "login".into()] });
        assert!(line.ends_with("; '/Users/a b/.local/bin/claude' auth login"), "{line}");
        assert!(line.starts_with("clear; export PATH="));
    }

    #[test]
    fn a_failure_says_its_last_line_and_permission_is_named() {
        assert_eq!(failed("npm install -g x", "a\nnpm error 404 Not Found\nnpm notice x\n"), "npm install -g x did not finish: npm error 404 Not Found");
        assert!(failed("npm install -g x", "npm error code EACCES").contains("sudo"));
        assert_eq!(failed("brew upgrade codex", ""), "brew upgrade codex did not finish.");
    }

    #[test]
    fn npm_without_permission_installs_into_the_persons_own_folder() {
        let plan = plan_of("gemini-cli", Job::Install, None, Some(&p("/usr/local/bin/npm")), None).unwrap();
        let denied = failed(&plan.typed(), "npm error code EACCES");
        if cfg!(windows) {
            return;
        }
        let mine = user_prefix(&plan, &denied).expect("a retry into ~/.local");
        assert_eq!(&mine.args[..4], ["install", "-g", "@google/gemini-cli@latest", "--prefix"]);
        assert!(mine.args[4].ends_with("/.local"));
        assert!(user_prefix(&plan, "npm install did not finish: 404").is_none(), "only a permission failure retries");
        let brew = plan_of("codex", Job::Update, Some(&p("/opt/homebrew/bin/codex")), None, Some(&p("/opt/homebrew/bin/brew"))).unwrap();
        assert!(user_prefix(&brew, &denied).is_none(), "only npm retries");
    }

    #[test]
    fn a_failed_command_is_reported_with_its_words() {
        if cfg!(windows) {
            return;
        }
        let e = run(&Plan { program: p("/bin/sh"), args: vec!["-c".into(), "echo nope >&2; exit 3".into()] }).unwrap_err();
        assert!(e.ends_with("did not finish: nope"), "{e}");
        assert!(run(&Plan { program: p("/bin/sh"), args: vec!["-c".into(), "exit 0".into()] }).is_ok());
    }
}

#[cfg(test)]
mod on_this_computer {
    /// What each button would run on this computer. Prints only; run with --ignored --nocapture.
    #[test]
    #[ignore]
    fn print_plans() {
        for id in ["codex", "claude-code", "gemini-cli", "copilot", "opencode"] {
            for job in [super::Job::Install, super::Job::Update, super::Job::SignIn] {
                println!("{id} {job:?}: {:?}", super::plan(id, job).map(|p| (p.program, p.args)));
            }
        }
    }
}

#[cfg(test)]
mod really {
    /// Runs the Update button's own path for `POINTCAST_SETUP_ID`. Changes this computer; run by hand only.
    #[test]
    #[ignore]
    fn update_for_real() {
        let id = std::env::var("POINTCAST_SETUP_ID").expect("POINTCAST_SETUP_ID");
        println!("result: {:?}", super::setup(&id, super::Job::Update));
    }
}
