//! A vendor's command-line tool run as a helper: started with constant arguments and no console window, spoken to
//! over its stdio, and ended with Pointcast. The pattern is the speech helper's (`voice/whisper_server.rs`): a job
//! object on Windows, its own process group and the parent's death signal on Unix, so the whole tree goes (an npm
//! shim is three processes deep). One thing differs. The speech helper's path is Pointcast's alone; `codex` is also
//! the person's own session, so a helper left by a crash is found by its pid AND the time that process started,
//! and is ended only when both match.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// How much of what the helper last wrote to stderr is kept, so a dead one can say why.
pub const TAIL: usize = 4096;

/// How long a helper gets to leave by itself once its stdin is closed.
pub const CLOSE_GRACE: Duration = Duration::from_secs(2);

/// What to run. The arguments are constants: nothing the person said ever rides on a command line.
#[derive(Debug, Clone, PartialEq)]
pub struct Launch {
    pub exe: PathBuf,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
}

/// The end of the helper's stderr.
#[derive(Default)]
pub struct Tail {
    bytes: Mutex<Vec<u8>>,
    done: AtomicBool,
}

impl Tail {
    fn push(&self, more: &[u8]) {
        let mut b = self.bytes.lock().unwrap();
        b.extend_from_slice(more);
        if b.len() > TAIL {
            let cut = b.len() - TAIL;
            b.drain(..cut);
        }
    }

    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.bytes.lock().unwrap()).trim().to_owned()
    }

    /// The text once stderr has closed, or what there is after `within`.
    pub fn settled(&self, within: Duration) -> String {
        let t = Instant::now();
        while !self.done.load(Ordering::SeqCst) && t.elapsed() < within {
            std::thread::sleep(Duration::from_millis(10));
        }
        self.text()
    }
}

/// A running helper. Dropping it ends the helper and everything the helper started.
pub struct Proc {
    child: Option<Child>,
    #[cfg(windows)]
    job: Option<isize>,
    pid: u32,
    tail: Arc<Tail>,
    pidfile: Option<PathBuf>,
}

impl Proc {
    /// Start the helper in `cwd` and hand back its stdin and stdout. With a `pidfile`, record it there for the
    /// next run to find.
    pub fn start(l: &Launch, cwd: &Path, pidfile: Option<&Path>) -> std::io::Result<(Proc, ChildStdin, ChildStdout)> {
        let mut cmd = Command::new(&l.exe);
        cmd.args(&l.args).current_dir(cwd).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
        for (k, v) in &l.env {
            cmd.env(k, v);
        }
        let mut child = spawn(cmd)?;
        let pid = child.id();
        #[cfg(windows)]
        let job = job_for(&child);
        let tail = Arc::new(Tail::default());
        match child.stderr.take() {
            Some(mut err) => {
                let t = tail.clone();
                std::thread::spawn(move || {
                    let mut buf = [0u8; 1024];
                    while let Ok(n) = err.read(&mut buf) {
                        if n == 0 {
                            break;
                        }
                        t.push(&buf[..n]);
                    }
                    t.done.store(true, Ordering::SeqCst);
                });
            }
            None => tail.done.store(true, Ordering::SeqCst),
        }
        let pipes = (child.stdin.take(), child.stdout.take());
        let mut p = Proc {
            child: Some(child),
            #[cfg(windows)]
            job,
            pid,
            tail,
            pidfile: None,
        };
        let (Some(stdin), Some(stdout)) = pipes else {
            p.end();
            return Err(std::io::Error::other("the helper has no stdio"));
        };
        if let (Some(f), Some(started)) = (pidfile, start_time(pid)) {
            if let Some(dir) = f.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            if std::fs::write(f, format!("{pid}\n{started}\n{}\n", std::process::id())).is_ok() {
                p.pidfile = Some(f.to_owned());
            }
        }
        Ok((p, stdin, stdout))
    }

    pub fn pid(&self) -> u32 {
        self.pid
    }

    pub fn tail(&self) -> Arc<Tail> {
        self.tail.clone()
    }

    pub fn exited(&mut self) -> bool {
        self.child.as_mut().is_none_or(|c| !matches!(c.try_wait(), Ok(None)))
    }

    /// For a helper whose stdin the caller has closed: give it `grace` to leave, then end it and its tree.
    pub fn end_within(&mut self, grace: Duration) {
        let t = Instant::now();
        while !self.exited() && t.elapsed() < grace {
            std::thread::sleep(Duration::from_millis(20));
        }
        self.end();
    }

    /// End the helper and everything it started, now.
    pub fn end(&mut self) {
        if let Some(mut c) = self.child.take() {
            // The group first: it is named by the helper's pid, which stays ours until the wait below.
            #[cfg(unix)]
            unsafe {
                libc::kill(-(self.pid as i32), libc::SIGKILL);
            }
            #[cfg(windows)]
            if let Some(j) = self.job {
                use windows::Win32::Foundation::HANDLE;
                use windows::Win32::System::JobObjects::TerminateJobObject;
                unsafe {
                    let _ = TerminateJobObject(HANDLE(j as _), 1);
                }
            }
            let _ = c.kill();
            let _ = c.wait();
        }
        #[cfg(windows)]
        if let Some(j) = self.job.take() {
            use windows::Win32::Foundation::{CloseHandle, HANDLE};
            unsafe {
                let _ = CloseHandle(HANDLE(j as _));
            }
        }
        if let Some(p) = self.pidfile.take() {
            let _ = std::fs::remove_file(p);
        }
    }
}

impl Drop for Proc {
    fn drop(&mut self) {
        self.end();
    }
}

/// Start `cmd` so the helper ends with Pointcast: no console window on Windows (the job is made by `job_for`); its
/// own process group on Unix, and on Linux the parent's death signal (from a thread that lives as long as
/// Pointcast, since the signal follows the thread that started it).
fn spawn(mut cmd: Command) -> std::io::Result<Child> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
        cmd.spawn()
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
        #[cfg(target_os = "linux")]
        {
            unsafe {
                cmd.pre_exec(|| {
                    libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL);
                    Ok(())
                });
            }
            type Job = (Command, std::sync::mpsc::Sender<std::io::Result<Child>>);
            static SPAWNER: std::sync::OnceLock<Mutex<std::sync::mpsc::Sender<Job>>> = std::sync::OnceLock::new();
            let tx = SPAWNER.get_or_init(|| {
                let (tx, rx) = std::sync::mpsc::channel::<Job>();
                std::thread::spawn(move || {
                    for (mut c, back) in rx {
                        let _ = back.send(c.spawn());
                    }
                });
                Mutex::new(tx)
            });
            let (back, answer) = std::sync::mpsc::channel();
            tx.lock().unwrap().send((cmd, back)).map_err(|_| std::io::Error::other("no spawner"))?;
            answer.recv().map_err(|_| std::io::Error::other("no spawner"))?
        }
        #[cfg(not(target_os = "linux"))]
        cmd.spawn()
    }
}

/// A job object that ends the helper's whole tree when Pointcast's handle to it closes, crash included.
#[cfg(windows)]
fn job_for(child: &Child) -> Option<isize> {
    use std::os::windows::io::AsRawHandle;
    use windows::Win32::Foundation::{CloseHandle, HANDLE};
    use windows::Win32::System::JobObjects::*;
    unsafe {
        let job = CreateJobObjectW(None, None).ok()?;
        let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        let set = SetInformationJobObject(
            job,
            JobObjectExtendedLimitInformation,
            &info as *const _ as *const std::ffi::c_void,
            std::mem::size_of_val(&info) as u32,
        );
        if set.is_err() || AssignProcessToJobObject(job, HANDLE(child.as_raw_handle() as _)).is_err() {
            let _ = CloseHandle(job);
            return None;
        }
        Some(job.0 as isize)
    }
}

/// When the running process `pid` started, in the system's own units, or `None` if no such process runs. Two
/// processes that share a pid over time never share this.
#[cfg(windows)]
pub fn start_time(pid: u32) -> Option<u64> {
    use windows::Win32::Foundation::{CloseHandle, FILETIME};
    use windows::Win32::System::Threading::*;
    const STILL_ACTIVE: u32 = 259;
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        // A handle opens on a process that has exited, so ask whether it still runs.
        let mut code = 0u32;
        let runs = GetExitCodeProcess(h, &mut code).is_ok() && code == STILL_ACTIVE;
        let (mut made, mut gone, mut kernel, mut user) = (FILETIME::default(), FILETIME::default(), FILETIME::default(), FILETIME::default());
        let timed = GetProcessTimes(h, &mut made, &mut gone, &mut kernel, &mut user).is_ok();
        let _ = CloseHandle(h);
        (runs && timed).then(|| ((made.dwHighDateTime as u64) << 32) | made.dwLowDateTime as u64)
    }
}

#[cfg(target_os = "linux")]
pub fn start_time(pid: u32) -> Option<u64> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    // After the name in brackets: the state, then the fields from the fourth on; the start time is the 22nd.
    let mut rest = stat[stat.rfind(')')? + 1..].split_whitespace();
    if rest.next()? == "Z" {
        return None;
    }
    rest.nth(18)?.parse().ok()
}

#[cfg(target_os = "macos")]
pub fn start_time(pid: u32) -> Option<u64> {
    let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
    let size = std::mem::size_of::<libc::proc_bsdinfo>() as libc::c_int;
    let n = unsafe { libc::proc_pidinfo(pid as libc::c_int, libc::PROC_PIDTBSDINFO, 0, &mut info as *mut _ as *mut libc::c_void, size) };
    // 5 is a zombie: ended, not yet collected.
    (n == size && info.pbi_status != 5).then(|| info.pbi_start_tvsec * 1_000_000 + info.pbi_start_tvusec)
}

#[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
pub fn start_time(_pid: u32) -> Option<u64> {
    None
}

/// End a helper an earlier run left behind. The pid file holds its pid, the time it started and the Pointcast that
/// started it; the process is ended only if one with that pid runs AND started at that time. A file written by
/// this run names a helper that is in use, and is left alone.
pub fn reap_stale(pidfile: &Path) {
    let Ok(s) = std::fs::read_to_string(pidfile) else { return };
    let mut n = s.lines().map(|l| l.trim().parse::<u64>().ok());
    let (pid, started, owner) = (n.next().flatten(), n.next().flatten(), n.next().flatten());
    if owner == Some(std::process::id() as u64) {
        return;
    }
    let _ = std::fs::remove_file(pidfile);
    let (Some(pid), Some(started)) = (pid.and_then(|p| u32::try_from(p).ok()), started) else { return };
    if pid <= 1 || pid == std::process::id() || start_time(pid) != Some(started) {
        return;
    }
    end_stray(pid);
}

#[cfg(windows)]
fn end_stray(pid: u32) {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Threading::*;
    unsafe {
        let Ok(h) = OpenProcess(PROCESS_TERMINATE, false, pid) else { return };
        let _ = TerminateProcess(h, 1);
        let _ = CloseHandle(h);
    }
}

#[cfg(unix)]
fn end_stray(pid: u32) {
    unsafe {
        libc::kill(-(pid as i32), libc::SIGKILL);
        libc::kill(pid as i32, libc::SIGKILL);
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use crate::assist::tests::scratch;
    use std::io::BufRead;

    const KID: &str = "assist::child::tests::kid";

    /// This test program run again as a helper that behaves as `mode` says.
    pub fn helper(mode: &str) -> Launch {
        Launch {
            exe: std::env::current_exe().unwrap(),
            args: ["--exact", KID, "--ignored", "--nocapture"].map(String::from).to_vec(),
            env: vec![("POINTCAST_KID".into(), mode.into())],
        }
    }

    /// Whether `pid` is gone within five seconds.
    pub fn gone(pid: u32) -> bool {
        let t = Instant::now();
        while start_time(pid).is_some() {
            if t.elapsed() > Duration::from_secs(5) {
                return false;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        true
    }

    /// The helper the tests here start. Does nothing when run with the rest.
    #[test]
    #[ignore]
    fn kid() {
        let Ok(mode) = std::env::var("POINTCAST_KID") else { return };
        // On one test thread the harness leaves "test <name> ... " unfinished on stdout: end that line first.
        println!();
        match mode.as_str() {
            "tree" => {
                let l = helper("sleep");
                let g = Command::new(&l.exe)
                    .args(&l.args)
                    .env("POINTCAST_KID", "sleep")
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .spawn()
                    .unwrap();
                println!("grandchild {}", g.id());
                std::thread::sleep(Duration::from_secs(30));
            }
            "loud" => {
                eprint!("{}", "x".repeat(6000));
                eprintln!("last words");
                std::process::exit(3);
            }
            "polite" => {
                let mut all = Vec::new();
                let _ = std::io::stdin().read_to_end(&mut all);
                std::process::exit(0);
            }
            _ => std::thread::sleep(Duration::from_secs(30)),
        }
    }

    #[test]
    fn ending_the_helper_ends_what_the_helper_started() {
        let d = scratch("tree");
        let (mut p, _stdin, stdout) = Proc::start(&helper("tree"), &d, None).unwrap();
        let grandchild = std::io::BufReader::new(stdout)
            .lines()
            .map_while(Result::ok)
            .find_map(|l| l.strip_prefix("grandchild ").and_then(|n| n.trim().parse::<u32>().ok()))
            .expect("the helper names what it started");
        assert!(start_time(grandchild).is_some() && start_time(p.pid()).is_some());
        let pid = p.pid();
        p.end();
        assert!(gone(pid), "the helper is ended");
        assert!(gone(grandchild), "and so is what it started");
    }

    #[test]
    fn dropping_the_handle_ends_the_helper() {
        let d = scratch("drop");
        let (p, _stdin, _stdout) = Proc::start(&helper("sleep"), &d, None).unwrap();
        let pid = p.pid();
        assert!(start_time(pid).is_some());
        drop(p);
        assert!(gone(pid));
    }

    #[test]
    fn a_helper_that_leaves_when_stdin_closes_is_not_killed_and_one_that_stays_is_ended_after_the_wait() {
        let d = scratch("close");
        let (mut p, stdin, _stdout) = Proc::start(&helper("polite"), &d, None).unwrap();
        let pid = p.pid();
        drop(stdin);
        let t = Instant::now();
        p.end_within(Duration::from_secs(10));
        assert!(t.elapsed() < Duration::from_secs(8), "it left by itself, well inside the wait");
        assert!(gone(pid));

        let (mut p, stdin, _stdout) = Proc::start(&helper("sleep"), &d, None).unwrap();
        let pid = p.pid();
        drop(stdin);
        let t = Instant::now();
        p.end_within(Duration::from_millis(300));
        assert!(t.elapsed() >= Duration::from_millis(300), "it had its wait");
        assert!(gone(pid), "then it was ended");
    }

    #[test]
    fn the_stderr_tail_keeps_the_last_4_kb() {
        let d = scratch("tail");
        let (mut p, _stdin, _stdout) = Proc::start(&helper("loud"), &d, None).unwrap();
        let t = Instant::now();
        while !p.exited() {
            assert!(t.elapsed() < Duration::from_secs(20), "the helper ends by itself");
            std::thread::sleep(Duration::from_millis(20));
        }
        let said = p.tail().settled(Duration::from_secs(5));
        assert!(said.len() <= TAIL, "{} bytes kept", said.len());
        assert!(said.len() > TAIL - 100, "close to all of the 4 KB is kept: {}", said.len());
        assert!(said.starts_with('x') && said.ends_with("last words"), "the end is what is kept: {}", &said[said.len() - 20..]);
    }

    #[test]
    fn the_pid_file_holds_pid_start_time_and_owner_and_goes_with_the_helper() {
        let d = scratch("pidfile");
        let f = d.join("run").join("helper.pid");
        let (mut p, _stdin, _stdout) = Proc::start(&helper("sleep"), &d, Some(&f)).unwrap();
        let want = format!("{}\n{}\n{}\n", p.pid(), start_time(p.pid()).unwrap(), std::process::id());
        assert_eq!(std::fs::read_to_string(&f).unwrap(), want);
        p.end();
        assert!(!f.exists());
    }

    #[test]
    fn a_stray_is_ended_only_when_its_pid_and_its_start_time_both_match() {
        let d = scratch("stray");
        let f = d.join("helper.pid");
        // Another process under the recorded pid (the person's own session, say): the start time differs, so it
        // lives on, and the stale file is gone.
        let (mut other, _i, _o) = Proc::start(&helper("sleep"), &d, None).unwrap();
        let started = start_time(other.pid()).unwrap();
        std::fs::write(&f, format!("{}\n{}\n0\n", other.pid(), started + 1)).unwrap();
        reap_stale(&f);
        std::thread::sleep(Duration::from_millis(300));
        assert_eq!(start_time(other.pid()), Some(started), "never by pid alone");
        assert!(!f.exists());
        // The same process named with its own start time is the helper a crash left: ended.
        std::fs::write(&f, format!("{}\n{started}\n0\n", other.pid())).unwrap();
        reap_stale(&f);
        assert!(gone(other.pid()), "the stray is ended");
        assert!(!f.exists());
        other.end();
    }

    #[test]
    fn a_helper_this_run_started_is_not_a_stray() {
        let d = scratch("ours");
        let f = d.join("helper.pid");
        let (mut p, _i, _o) = Proc::start(&helper("sleep"), &d, Some(&f)).unwrap();
        reap_stale(&f);
        std::thread::sleep(Duration::from_millis(300));
        assert!(start_time(p.pid()).is_some(), "a conversation in use is left alone");
        assert!(f.exists());
        p.end();
    }
}
