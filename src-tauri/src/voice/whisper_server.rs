//! Whisper on the graphics card: whisper.cpp's `whisper-server`, run as a helper process on a loopback port and sent
//! one piece of a take at a time. It dies with Pointcast (a job object on Windows, a process group and the parent's
//! death signal on Linux), and a helper left by a crash is ended at the next start, found by its pid file and its path.
use std::collections::VecDeque;
use std::io::{BufRead, Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// The helper's executable in its folder.
pub const EXE: &str = if cfg!(windows) { "whisper-server.exe" } else { "whisper-server" };

/// Why a request came back without words.
#[derive(Debug, Clone, PartialEq)]
pub enum Fail {
    /// Cancelled while the helper worked: the connection was closed, which stops its decoding.
    Aborted,
    /// The helper did not answer or answered nonsense.
    Broken(String),
}

/// What the helper runs on, from what it said on stderr while loading: "gpu", "cpu" or "unknown".
pub fn backend_of<'a>(lines: impl IntoIterator<Item = &'a str>) -> &'static str {
    let mut seen = "unknown";
    for l in lines {
        if l.contains("whisper_backend_init_gpu: using") {
            seen = "gpu";
        } else if l.contains("whisper_backend_init_gpu: no GPU found") {
            seen = "cpu";
        }
    }
    seen
}

/// Whether a stderr line can name the device: the lines `device_of` reads.
fn about_device(l: &str) -> bool {
    ["ggml_vulkan: ", "Device ", "GPU name:", "found device: ", "whisper_backend_init_gpu: using "].iter().any(|k| l.contains(k))
}

/// The graphics card the helper chose and how it drives it, as "NVIDIA GeForce RTX 2060 SUPER via Vulkan", from what
/// it said while loading: the backend it uses (`using Vulkan0 backend`) and the name its library gave that device
/// (Vulkan and CUDA list every card by number, Metal names its one). None before it chose one or with none.
pub fn device_of<'a>(lines: impl IntoIterator<Item = &'a str>) -> Option<String> {
    let mut names: Vec<(String, String)> = vec![];
    let mut using = None;
    for l in lines {
        if let Some((i, rest)) = l.split("ggml_vulkan: ").nth(1).and_then(|r| r.split_once(" = ")) {
            if let Ok(i) = i.trim().parse::<u32>() {
                let name = rest.split(" | ").next().unwrap_or(rest);
                names.push((format!("Vulkan{i}"), name.rsplit_once(" (").map_or(name, |(n, _)| n).trim().into()));
            }
        } else if let Some((i, rest)) = l.trim_start().strip_prefix("Device ").and_then(|r| r.split_once(": ")) {
            if let Ok(i) = i.trim().parse::<u32>() {
                names.push((format!("CUDA{i}"), rest.split(", compute").next().unwrap_or(rest).trim().into()));
            }
        } else if let Some(r) = l.split("GPU name:").nth(1).or_else(|| l.split("found device: ").nth(1)) {
            names.push(("Metal".into(), r.trim().into()));
        }
        if let Some(r) = l.split("whisper_backend_init_gpu: using ").nth(1) {
            using = Some(r.trim().trim_end_matches("backend").trim().to_string());
        }
    }
    let dev = using?;
    let api = match dev.trim_end_matches(|c: char| c.is_ascii_digit()) {
        "MTL" => "Metal",
        a => a,
    };
    let name = names.iter().find(|(k, _)| *k == dev).or_else(|| names.iter().find(|(k, _)| k.starts_with(api))).map(|(_, n)| n);
    Some(match name {
        Some(n) if !n.is_empty() => format!("{n} via {api}"),
        _ => api.to_string(),
    })
}

/// Words whisper.cpp gives for what is not speech, in brackets or parentheses.
const NOT_SPEECH: &[&str] = &[
    "blank_audio", "blank audio", "silence", "music", "applause", "laughter", "laughs", "laughing", "noise", "inaudible",
    "static", "beep", "sigh", "sighs", "coughs", "cough", "breathing", "wind", "background noise", "no speech", "sound",
    "pause", "clears throat", "typing", "clicking", "keyboard clicking",
];

/// The words of a piece without whisper.cpp's marks for what is not speech: every `[...]`, and `(...)` or `*...*`
/// around a sound, as `[BLANK_AUDIO]`, `[Music]`, `(silence)`.
pub fn clean(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(i) = rest.find(['[', '(', '*']) {
        let open = rest.as_bytes()[i];
        let close = match open {
            b'[' => ']',
            b'(' => ')',
            _ => '*',
        };
        let Some(j) = rest[i + 1..].find(close).map(|j| i + 1 + j) else { break };
        let inner = rest[i + 1..j].trim().trim_matches(['-', ' ', '.']).to_lowercase();
        let drop = open == b'[' || NOT_SPEECH.contains(&inner.as_str());
        out.push_str(&rest[..i]);
        if !drop {
            out.push_str(&rest[i..=j]);
        }
        rest = &rest[j + 1..];
    }
    out.push_str(rest);
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// 16 kHz mono samples as a 16-bit PCM WAV file in memory.
pub fn wav(samples: &[f32]) -> Vec<u8> {
    let data = samples.len() as u32 * 2;
    let mut w = Vec::with_capacity(44 + data as usize);
    w.extend_from_slice(b"RIFF");
    w.extend_from_slice(&(36 + data).to_le_bytes());
    w.extend_from_slice(b"WAVEfmt ");
    w.extend_from_slice(&16u32.to_le_bytes());
    w.extend_from_slice(&1u16.to_le_bytes());
    w.extend_from_slice(&1u16.to_le_bytes());
    w.extend_from_slice(&16_000u32.to_le_bytes());
    w.extend_from_slice(&32_000u32.to_le_bytes());
    w.extend_from_slice(&2u16.to_le_bytes());
    w.extend_from_slice(&16u16.to_le_bytes());
    w.extend_from_slice(b"data");
    w.extend_from_slice(&data.to_le_bytes());
    for s in samples {
        w.extend_from_slice(&((s.clamp(-1.0, 1.0) * 32767.0).round() as i16).to_le_bytes());
    }
    w
}

/// A multipart form with `fields` and the piece as `file`.
fn form(boundary: &str, fields: &[(&str, &str)], wav: &[u8]) -> Vec<u8> {
    let mut b = vec![];
    for (k, v) in fields {
        b.extend_from_slice(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"{k}\"\r\n\r\n{v}\r\n").as_bytes());
    }
    b.extend_from_slice(
        format!("--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"piece.wav\"\r\nContent-Type: audio/wav\r\n\r\n").as_bytes(),
    );
    b.extend_from_slice(wav);
    b.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    b
}

/// How long a piece of `ms` may take before the helper counts as hung: generous, as on a processor it runs slower
/// than real time.
pub fn piece_timeout(ms: u64) -> Duration {
    Duration::from_millis(60_000 + ms * 8)
}

/// One HTTP/1.1 request to the helper on `port`, read until it answers. Closing the connection is how a cancel
/// reaches it: `abort` moving on from `gen` closes it at once.
fn request(port: u16, head: &str, body: &[u8], deadline: Instant, abort: &AtomicU64, gen: u64) -> Result<(u16, Vec<u8>), Fail> {
    let broken = |e: &dyn std::fmt::Display| Fail::Broken(format!("the GPU engine stopped answering ({e})"));
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    let mut s = TcpStream::connect_timeout(&addr, Duration::from_secs(5)).map_err(|e| broken(&e))?;
    let _ = s.set_nodelay(true);
    s.set_write_timeout(Some(Duration::from_secs(30))).map_err(|e| broken(&e))?;
    s.write_all(format!("{head}Host: 127.0.0.1:{port}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()).as_bytes())
        .and_then(|_| s.write_all(body))
        .map_err(|e| broken(&e))?;
    s.set_read_timeout(Some(Duration::from_millis(50))).map_err(|e| broken(&e))?;
    let mut got = vec![];
    let mut buf = [0u8; 16 * 1024];
    loop {
        if abort.load(Ordering::SeqCst) != gen {
            let _ = s.shutdown(std::net::Shutdown::Both);
            return Err(Fail::Aborted);
        }
        if Instant::now() > deadline {
            let _ = s.shutdown(std::net::Shutdown::Both);
            return Err(Fail::Broken("the GPU engine took too long".into()));
        }
        match s.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                got.extend_from_slice(&buf[..n]);
                if answered(&got).is_some() {
                    break;
                }
            }
            Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {}
            Err(e) => return Err(broken(&e)),
        }
    }
    answered(&got).ok_or_else(|| broken(&"no answer"))
}

/// The status and body of a whole HTTP response in `got`, or None while it is still arriving.
fn answered(got: &[u8]) -> Option<(u16, Vec<u8>)> {
    let end = got.windows(4).position(|w| w == b"\r\n\r\n")?;
    let head = String::from_utf8_lossy(&got[..end]);
    let status = head.split_whitespace().nth(1)?.parse().ok()?;
    let len = head.lines().find_map(|l| {
        let (k, v) = l.split_once(':')?;
        k.trim().eq_ignore_ascii_case("content-length").then(|| v.trim().parse::<usize>().ok())?
    })?;
    let body = &got[end + 4..];
    (body.len() >= len).then(|| (status, body[..len].to_vec()))
}

/// The helper as launched, kept to start it again.
#[derive(Debug, Clone)]
pub struct Launch {
    /// The helper's folder, holding `EXE` and its libraries.
    pub dir: PathBuf,
    pub model: PathBuf,
    /// Where the pid file of a running helper goes, so one left by a crash can be ended.
    pub state: PathBuf,
    /// The libraries the helper's folder may hold, by file name: its pinned files. ggml loads every backend it finds
    /// in the program's folder and the working directory, so any other one there stops the start. Empty: unchecked
    /// (a development helper).
    pub allowed: Vec<String>,
}

/// Whether ggml or the system loader could load `name` from the helper's folder.
fn library(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    n.starts_with("ggml") || n.starts_with("libggml") || n.ends_with(".dll") || n.ends_with(".so") || n.contains(".so.") || n.ends_with(".dylib")
}

/// The libraries in `dir` that are not `allowed`: what a start refuses to run beside.
pub fn strays(dir: &Path, allowed: &[String]) -> Vec<String> {
    let names = std::fs::read_dir(dir).into_iter().flatten().flatten().filter_map(|e| e.file_name().into_string().ok());
    let mut out: Vec<String> = names.filter(|n| library(n) && !allowed.iter().any(|a| a == n)).collect();
    out.sort();
    out
}

/// A random path segment of 24 hex digits (96 bits), so nothing that finds the port can reach the helper.
fn secret(prefix: &str) -> Result<String, String> {
    let mut key = [0u8; 12];
    getrandom::getrandom(&mut key).map_err(|e| e.to_string())?;
    Ok(format!("/{prefix}{}", key.iter().map(|b| format!("{b:02x}")).collect::<String>()))
}

/// A running helper, or (in tests) any server on a loopback port speaking its protocol.
pub struct Server {
    child: Option<Child>,
    #[cfg(windows)]
    job: Option<isize>,
    pub port: u16,
    path: String,
    /// Where it answers whether the model is loaded.
    health: String,
    /// The last lines the helper wrote to stderr.
    log: Arc<Mutex<VecDeque<String>>>,
    /// What it said it runs on while loading.
    seen: Arc<Mutex<&'static str>>,
    /// The lines it said about its devices while loading, which name the graphics card it chose.
    devices: Arc<Mutex<Vec<String>>>,
    /// Moved on by a cancel: the request under way closes its connection.
    pub abort: Arc<AtomicU64>,
    pidfile: Option<PathBuf>,
    pub launch: Option<Launch>,
}

/// Said when the helper did not get ready in time.
pub const SLOW_START: &str = "Your graphics card took more than 3 minutes to get ready for speech, so Pointcast stopped waiting. Try again; it is quicker after the first time.";

/// Lines of the helper's stderr kept for an error message.
const LOG_LINES: usize = 20;

impl Server {
    /// A server already listening on `port` at `path`: the stand-in tests use.
    pub fn at(port: u16, path: &str) -> Server {
        Server {
            child: None,
            #[cfg(windows)]
            job: None,
            port,
            path: path.into(),
            health: "/health".into(),
            log: Arc::default(),
            seen: Arc::new(Mutex::new("unknown")),
            devices: Arc::default(),
            abort: Arc::default(),
            pidfile: None,
            launch: None,
        }
    }

    /// Start the helper on a free port with a path no other program knows, and wait until it answers.
    pub fn start(l: &Launch, ready_within: Duration) -> Result<Server, String> {
        let exe = l.dir.join(EXE);
        let pidfile = l.state.join("whisper-helper.pid");
        reap_stale(&pidfile, &exe);
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .and_then(|s| s.local_addr())
            .map_err(|e| e.to_string())?
            .port();
        if !l.allowed.is_empty() {
            let extra = strays(&l.dir, &l.allowed);
            if !extra.is_empty() {
                return Err(format!("The GPU engine's folder holds files it did not come with ({}); set it up again in Settings › Speech.", extra.join(", ")));
            }
        }
        // Every route under a secret prefix: upstream answers POST /load (any file path) and allows any web page.
        let (prefix, inference) = (secret("")?, secret("i-")?);
        let mut cmd = Command::new(&exe);
        cmd.arg("-m")
            .arg(&l.model)
            .args(["--host", "127.0.0.1", "--port", &port.to_string(), "--request-path", &prefix, "--inference-path", &inference, "-t", "4"])
            .current_dir(&l.dir)
            .env_remove("GGML_BACKEND_PATH");
        // A development build can make the helper find no graphics card, as on a computer without one.
        if cfg!(debug_assertions) && std::env::var_os("POINTCAST_HELPER_NO_GPU").is_some() {
            cmd.arg("-ng");
        }
        cmd
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        let mut child = spawn(cmd).map_err(|e| format!("The GPU engine did not start: {e}"))?;
        let mut s = Server::at(port, &format!("{prefix}{inference}"));
        s.health = format!("{prefix}/health");
        #[cfg(windows)]
        {
            s.job = job_for(&child);
        }
        if let Some(err) = child.stderr.take() {
            let (log, seen, devices) = (s.log.clone(), s.seen.clone(), s.devices.clone());
            std::thread::spawn(move || {
                for line in std::io::BufReader::new(err).lines().map_while(Result::ok) {
                    match backend_of([line.as_str()]) {
                        "unknown" => {}
                        b => *seen.lock().unwrap() = b,
                    }
                    if about_device(&line) {
                        devices.lock().unwrap().push(line.clone());
                    }
                    let mut g = log.lock().unwrap();
                    if g.len() >= LOG_LINES {
                        g.pop_front();
                    }
                    g.push_back(line);
                }
            });
        }
        let _ = std::fs::create_dir_all(&l.state);
        let _ = std::fs::write(&pidfile, format!("{}\n{}\n", child.id(), exe.display()));
        s.pidfile = Some(pidfile);
        s.child = Some(child);
        s.launch = Some(l.clone());
        let t = Instant::now();
        while !s.answers() {
            if let Some(code) = s.child.as_mut().and_then(|c| c.try_wait().ok().flatten()) {
                return Err(format!("The GPU engine stopped while loading ({code}). {}", s.last_words()));
            }
            if t.elapsed() > ready_within {
                return Err(SLOW_START.into());
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        Ok(s)
    }

    /// Whether it says its model is loaded (the port opens only then).
    pub fn answers(&self) -> bool {
        let never = AtomicU64::new(0);
        let head = format!("GET {} HTTP/1.1\r\n", self.health);
        matches!(request(self.port, &head, &[], Instant::now() + Duration::from_secs(2), &never, 0), Ok((200, _)))
    }

    /// The helper's process id, while it runs.
    pub fn pid(&self) -> Option<u32> {
        self.child.as_ref().map(Child::id)
    }

    /// The last lines it wrote to stderr.
    pub fn lines(&self) -> Vec<String> {
        self.log.lock().unwrap().iter().cloned().collect()
    }

    /// Whether the helper process has ended (always false for a stand-in).
    pub fn exited(&mut self) -> bool {
        self.child.as_mut().is_some_and(|c| c.try_wait().ok().flatten().is_some())
    }

    /// What it runs on: "gpu", "cpu" or "unknown".
    pub fn backend(&self) -> &'static str {
        *self.seen.lock().unwrap()
    }

    /// The graphics card it runs on, as `device_of` names it.
    pub fn device(&self) -> Option<String> {
        device_of(self.devices.lock().unwrap().iter().map(String::as_str))
    }

    /// The helper's last few lines, for an error message.
    fn last_words(&self) -> String {
        let g = self.log.lock().unwrap();
        g.iter().rev().take(3).rev().cloned().collect::<Vec<_>>().join(" / ")
    }

    /// The words of one piece: beam search of 5 at temperature 0, no prompt, in `language` ("auto" detects it).
    pub fn transcribe(&self, samples: &[f32], language: &str, timeout: Duration) -> Result<String, Fail> {
        let gen = self.abort.load(Ordering::SeqCst);
        let mut key = [0u8; 8];
        let _ = getrandom::getrandom(&mut key);
        let boundary = format!("pointcast{}", key.iter().map(|b| format!("{b:02x}")).collect::<String>());
        let fields = [("response_format", "json"), ("language", language), ("beam_size", "5"), ("temperature", "0.0")];
        let body = form(&boundary, &fields, &wav(samples));
        let head = format!("POST {} HTTP/1.1\r\nContent-Type: multipart/form-data; boundary={boundary}\r\n", self.path);
        let (status, body) = request(self.port, &head, &body, Instant::now() + timeout, &self.abort, gen)?;
        if status != 200 {
            return Err(Fail::Broken(format!("the GPU engine answered {status}")));
        }
        let v: serde_json::Value =
            serde_json::from_slice(&body).map_err(|_| Fail::Broken("the GPU engine's answer was not readable".into()))?;
        let text = v.get("text").and_then(|t| t.as_str()).ok_or_else(|| Fail::Broken("the GPU engine gave no text".into()))?;
        Ok(clean(text))
    }

    /// Stop the helper, which frees the graphics card's memory.
    pub fn kill(&mut self) {
        if let Some(mut c) = self.child.take() {
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

impl Drop for Server {
    fn drop(&mut self) {
        self.kill();
    }
}

/// Start `cmd` so the helper ends with Pointcast: no console window and a job that kills it when Pointcast's handle
/// closes on Windows; its own process group on Unix, and on Linux the parent's death signal (from a thread that
/// lives as long as Pointcast, since the signal follows the thread that started it).
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

/// A job object that ends the helper when Pointcast's handle to it closes, crash included.
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

/// End a helper an earlier run left behind: the pid in `pidfile`, only if that process is `exe`.
pub fn reap_stale(pidfile: &Path, exe: &Path) {
    let Ok(s) = std::fs::read_to_string(pidfile) else { return };
    let _ = std::fs::remove_file(pidfile);
    let Some(pid) = s.lines().next().and_then(|l| l.trim().parse::<u32>().ok()) else { return };
    if pid == std::process::id() {
        return;
    }
    kill_if(pid, exe);
}

#[cfg(windows)]
fn kill_if(pid: u32, exe: &Path) {
    use windows::core::PWSTR;
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Threading::*;
    unsafe {
        let Ok(h) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_TERMINATE, false, pid) else { return };
        let mut buf = [0u16; 1024];
        let mut len = buf.len() as u32;
        if QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len).is_ok() {
            let name = PathBuf::from(String::from_utf16_lossy(&buf[..len as usize]));
            if same_file(&name, exe) {
                let _ = TerminateProcess(h, 1);
            }
        }
        let _ = CloseHandle(h);
    }
}

#[cfg(unix)]
fn kill_if(pid: u32, exe: &Path) {
    #[cfg(target_os = "linux")]
    let name = std::fs::read_link(format!("/proc/{pid}/exe")).ok();
    #[cfg(target_os = "macos")]
    let name = {
        let mut buf = vec![0u8; 4096];
        let n = unsafe { libc::proc_pidpath(pid as i32, buf.as_mut_ptr() as *mut _, buf.len() as u32) };
        (n > 0).then(|| PathBuf::from(String::from_utf8_lossy(&buf[..n as usize]).into_owned()))
    };
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    let name: Option<PathBuf> = None;
    if name.is_some_and(|n| same_file(&n, exe)) {
        unsafe {
            libc::kill(-(pid as i32), libc::SIGKILL);
            libc::kill(pid as i32, libc::SIGKILL);
        }
    }
}

/// Whether two paths name the same file, compared as the OS resolves them.
fn same_file(a: &Path, b: &Path) -> bool {
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(x), Ok(y)) => x == y,
        _ => false,
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use std::io::BufReader;

    /// What the stand-in server does with each request to the inference path.
    #[derive(Clone)]
    pub enum Answer {
        /// 200 with this body.
        Json(String),
        /// Read the request, then wait this long before answering with `{"text":"late"}`; a closed connection is seen.
        Slow(Duration),
        /// Close the connection without a word, as a helper that died.
        Die,
    }

    /// A loopback stand-in for whisper-server: answers GET / and posts to `path` from `answers` in turn (the last one
    /// repeats), records each post's form fields, and notes when a client closed a connection it had not answered.
    pub struct Fake {
        pub port: u16,
        pub posts: Arc<Mutex<Vec<String>>>,
        pub hung_up: Arc<std::sync::atomic::AtomicBool>,
    }

    pub fn fake(path: &str, answers: Vec<Answer>) -> Fake {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        let (posts, hung_up) = (Arc::new(Mutex::new(vec![])), Arc::new(std::sync::atomic::AtomicBool::new(false)));
        let (seen, hang, path) = (posts.clone(), hung_up.clone(), path.to_string());
        std::thread::spawn(move || {
            let mut n = 0;
            for conn in l.incoming() {
                let Ok(mut c) = conn else { continue };
                let mut r = BufReader::new(c.try_clone().unwrap());
                let mut first = String::new();
                let _ = r.read_line(&mut first);
                let mut len = 0usize;
                loop {
                    let mut line = String::new();
                    if r.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                        break;
                    }
                    if let Some(v) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                        len = v.trim().parse().unwrap_or(0);
                    }
                }
                let mut body = vec![0u8; len];
                let _ = r.read_exact(&mut body);
                if first.starts_with("GET /health ") {
                    let _ = c.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok");
                    continue;
                }
                if !first.starts_with(&format!("POST {path} ")) {
                    let _ = c.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
                    continue;
                }
                seen.lock().unwrap().push(String::from_utf8_lossy(&body[..body.len().min(2_000)]).into_owned());
                let a = answers[n.min(answers.len() - 1)].clone();
                n += 1;
                let reply = |c: &mut TcpStream, json: &str| {
                    let _ = write!(c, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{json}", json.len());
                };
                match a {
                    Answer::Json(j) => reply(&mut c, &j),
                    Answer::Die => drop(c),
                    Answer::Slow(d) => {
                        let hang = hang.clone();
                        std::thread::spawn(move || {
                            let t = Instant::now();
                            c.set_read_timeout(Some(Duration::from_millis(20))).unwrap();
                            let mut b = [0u8; 1];
                            while t.elapsed() < d {
                                match c.read(&mut b) {
                                    Ok(0) => {
                                        hang.store(true, Ordering::SeqCst);
                                        return;
                                    }
                                    Err(e) if !matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {
                                        hang.store(true, Ordering::SeqCst);
                                        return;
                                    }
                                    _ => {}
                                }
                            }
                            reply(&mut c, r#"{"text":"late"}"#);
                        });
                    }
                }
            }
        });
        Fake { port, posts, hung_up }
    }

    const PATH: &str = "/i-test";

    /// One bodyless request as `head` (its request line), for what a route answers.
    pub fn raw(port: u16, head: &str, abort: &AtomicU64) -> Result<(u16, Vec<u8>), Fail> {
        request(port, head, &[], Instant::now() + Duration::from_secs(10), abort, abort.load(Ordering::SeqCst))
    }

    #[test]
    fn only_the_secret_paths_answer() {
        let f = fake(PATH, vec![Answer::Json(r#"{"text":"x"}"#.into())]);
        let never = AtomicU64::new(0);
        assert_eq!(raw(f.port, "POST /inference HTTP/1.1\r\n", &never).map(|r| r.0), Ok(404));
        assert_eq!(raw(f.port, "GET /health HTTP/1.1\r\n", &never).map(|r| r.0), Ok(200));
        let mut s = Server::at(f.port, PATH);
        assert!(s.answers());
        s.health = "/elsewhere/health".into();
        assert!(!s.answers(), "a 404 is not ready");
    }

    #[test]
    fn a_piece_is_posted_as_a_wav_with_beam_5_at_temperature_0_and_its_words_come_back() {
        let f = fake(PATH, vec![Answer::Json(r#"{"text":" Click the blue button.\n"}"#.into())]);
        let s = Server::at(f.port, PATH);
        assert!(s.answers());
        let text = s.transcribe(&vec![0.1; 16_000], "auto", Duration::from_secs(5)).unwrap();
        assert_eq!(text, "Click the blue button.");
        let post = f.posts.lock().unwrap()[0].clone();
        for (k, v) in [("beam_size", "5"), ("temperature", "0.0"), ("language", "auto"), ("response_format", "json")] {
            assert!(post.contains(&format!("name=\"{k}\"\r\n\r\n{v}\r\n")), "{k} in {post}");
        }
        assert!(!post.contains("name=\"prompt\""), "no prompt");
        assert!(post.contains("Content-Type: audio/wav\r\n\r\nRIFF"), "{post}");
    }

    #[test]
    fn whisper_marks_for_what_is_not_speech_read_as_nothing() {
        for t in [" [BLANK_AUDIO]\n", "[Music]", " (silence) ", "[ Silence ]", "*music*", "(Music) [BLANK_AUDIO]", "  "] {
            assert_eq!(clean(t), "", "{t:?}");
        }
        assert_eq!(clean(" Open the (settings) page [BLANK_AUDIO]"), "Open the (settings) page");
        assert_eq!(clean("Done. (laughs) Next"), "Done. Next");
        assert_eq!(clean("a [unclosed"), "a [unclosed");
        let f = fake(PATH, vec![Answer::Json(r#"{"text":" [BLANK_AUDIO]\n"}"#.into())]);
        assert_eq!(Server::at(f.port, PATH).transcribe(&[0.0; 8_000], "auto", Duration::from_secs(5)), Ok(String::new()));
    }

    #[test]
    fn a_malformed_or_missing_answer_is_broken_not_words() {
        let f = fake(PATH, vec![Answer::Json("not json".into()), Answer::Json(r#"{"error":"x"}"#.into()), Answer::Die]);
        let s = Server::at(f.port, PATH);
        for _ in 0..3 {
            assert!(matches!(s.transcribe(&[0.0; 1_600], "auto", Duration::from_secs(5)), Err(Fail::Broken(_))));
        }
        let nobody = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        assert!(matches!(Server::at(nobody, PATH).transcribe(&[0.0; 1_600], "auto", Duration::from_secs(5)), Err(Fail::Broken(_))));
        assert!(!Server::at(nobody, PATH).answers());
    }

    #[test]
    fn a_cancel_closes_the_request_under_way_at_once() {
        let f = fake(PATH, vec![Answer::Slow(Duration::from_secs(10))]);
        let s = Server::at(f.port, PATH);
        let abort = s.abort.clone();
        let t = Instant::now();
        let canceller = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(300));
            abort.fetch_add(1, Ordering::SeqCst);
        });
        assert_eq!(s.transcribe(&vec![0.1; 32_000], "auto", Duration::from_secs(30)), Err(Fail::Aborted));
        canceller.join().unwrap();
        assert!(t.elapsed() < Duration::from_secs(3), "{:?}", t.elapsed());
        let seen = Instant::now();
        while !f.hung_up.load(Ordering::SeqCst) {
            assert!(seen.elapsed() < Duration::from_secs(3), "the helper never saw the connection close");
            std::thread::sleep(Duration::from_millis(10));
        }
        // A cancel before the request began is not this request's.
        let f = fake(PATH, vec![Answer::Json(r#"{"text":"fine"}"#.into())]);
        let s = Server::at(f.port, PATH);
        s.abort.fetch_add(1, Ordering::SeqCst);
        assert_eq!(s.transcribe(&[0.0; 1_600], "auto", Duration::from_secs(5)), Ok("fine".into()));
    }

    #[test]
    fn a_piece_that_takes_too_long_is_broken() {
        let f = fake(PATH, vec![Answer::Slow(Duration::from_secs(10))]);
        let t = Instant::now();
        let r = Server::at(f.port, PATH).transcribe(&[0.0; 1_600], "auto", Duration::from_millis(400));
        assert!(matches!(r, Err(Fail::Broken(_))) && t.elapsed() < Duration::from_secs(3), "{r:?}");
        assert!(piece_timeout(25_000) >= Duration::from_secs(200));
    }

    #[test]
    fn the_backend_is_read_from_what_the_helper_said_while_loading() {
        let gpu = ["whisper_backend_init_gpu: device 0: Vulkan0 (type: 1)", "whisper_backend_init_gpu: using Vulkan0 backend"];
        assert_eq!(backend_of(gpu), "gpu");
        assert_eq!(backend_of(["whisper_backend_init_gpu: no GPU found"]), "cpu");
        assert_eq!(backend_of(["whisper_model_load: loading model"]), "unknown");
    }

    #[test]
    fn the_device_is_the_card_the_helper_chose_named_as_its_library_named_it() {
        let vulkan = [
            "ggml_vulkan: Found 2 Vulkan devices:",
            "ggml_vulkan: 0 = Intel(R) UHD Graphics 630 (Intel Corporation) | uma: 1 | fp16: 1 | warp size: 32",
            "ggml_vulkan: 1 = NVIDIA GeForce RTX 2060 SUPER (NVIDIA) | uma: 0 | fp16: 1 | bf16: 1 | warp size: 32 | matrix cores: NV_coopmat2",
            "whisper_backend_init_gpu: device 1: Vulkan1 (type: 1)",
            "whisper_backend_init_gpu: using Vulkan1 backend",
        ];
        assert_eq!(device_of(vulkan).as_deref(), Some("NVIDIA GeForce RTX 2060 SUPER via Vulkan"));
        let cuda = ["ggml_cuda_init: found 1 CUDA devices:", "  Device 0: NVIDIA GeForce RTX 3080, compute capability 8.6, VMM: yes", "whisper_backend_init_gpu: using CUDA0 backend"];
        assert_eq!(device_of(cuda).as_deref(), Some("NVIDIA GeForce RTX 3080 via CUDA"));
        let metal = ["ggml_metal_device_init: GPU name:   Apple M2 Pro", "whisper_backend_init_gpu: using MTL0 backend"];
        assert_eq!(device_of(metal).as_deref(), Some("Apple M2 Pro via Metal"));
        assert_eq!(device_of(["whisper_backend_init_gpu: using Metal backend"]).as_deref(), Some("Metal"));
        // Listed but not chosen, or no graphics card at all: none.
        assert_eq!(device_of(vulkan[..3].iter().copied()), None);
        assert_eq!(device_of(["whisper_backend_init_gpu: no GPU found"]), None);
        assert!(vulkan.iter().all(|l| about_device(l) || l.contains("device 1")) && !about_device("whisper_model_load: n_mels = 128"));
    }

    #[test]
    fn a_helper_folder_with_a_library_it_did_not_come_with_is_refused() {
        let d = std::env::temp_dir().join(format!("pc-strays-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        for f in ["whisper-server.exe", "ggml-base.dll", "LICENSE.txt", "libggml-vulkan.so"] {
            std::fs::write(d.join(f), b"x").unwrap();
        }
        let allowed: Vec<String> = ["whisper-server.exe", "ggml-base.dll", "libggml-vulkan.so"].map(String::from).to_vec();
        assert!(strays(&d, &allowed).is_empty());
        for f in ["ggml-cuda.dll", "libggml-hip.so", "vulkan-1.dll", "libfoo.so.1", "ggml-metal.dylib"] {
            std::fs::write(d.join(f), b"x").unwrap();
        }
        assert_eq!(strays(&d, &allowed), ["ggml-cuda.dll", "ggml-metal.dylib", "libfoo.so.1", "libggml-hip.so", "vulkan-1.dll"]);
        let l = Launch { dir: d.clone(), model: d.join("m.bin"), state: d.join("state"), allowed };
        let e = Server::start(&l, Duration::from_secs(1)).err().unwrap();
        assert!(e.contains("ggml-cuda.dll") && e.contains("did not come with"), "{e}");
        assert!(!d.join("state").join("whisper-helper.pid").exists(), "nothing was started");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn the_secret_paths_are_96_bits_and_differ() {
        let (a, b) = (secret("").unwrap(), secret("i-").unwrap());
        assert_eq!((a.len(), b.len()), (25, 27));
        assert!(a[1..].bytes().all(|c| c.is_ascii_hexdigit()) && b.starts_with("/i-") && a[1..] != b[3..]);
    }

    #[test]
    fn a_wav_is_16_bit_mono_at_16_khz() {
        let w = wav(&[0.0, 1.0, -1.0, 2.0]);
        assert_eq!((&w[..4], &w[8..16], w.len()), (&b"RIFF"[..], &b"WAVEfmt "[..], 44 + 8));
        let (rate, ch, pcm) = crate::voice::audio::read_wav(&w).unwrap();
        assert_eq!((rate, ch, pcm.len()), (16_000, 1, 4));
        assert_eq!(i16::from_le_bytes([w[46], w[47]]), 32767);
        assert_eq!(i16::from_le_bytes([w[50], w[51]]), 32767, "clipped, not wrapped");
    }

    #[test]
    fn a_stale_pid_file_ends_only_our_helper() {
        let d = std::env::temp_dir().join(format!("pc-reap-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        // A process that is not the helper, under a pid file naming the helper's path, lives on.
        let mut other = if cfg!(windows) {
            Command::new("ping").args(["-n", "30", "127.0.0.1"]).stdout(Stdio::null()).spawn().unwrap()
        } else {
            Command::new("sleep").arg("30").spawn().unwrap()
        };
        let pidfile = d.join("whisper-helper.pid");
        std::fs::write(&pidfile, format!("{}\n", other.id())).unwrap();
        reap_stale(&pidfile, &d.join(EXE));
        std::thread::sleep(Duration::from_millis(200));
        assert!(other.try_wait().unwrap().is_none(), "never by pid alone");
        assert!(!pidfile.exists());
        // Named by its own path, it is ended.
        let exe = std::env::current_exe().unwrap();
        let _ = other.kill();
        let _ = other.wait();
        let mut ours = Command::new(&exe)
            .args(["--exact", "voice::whisper_server::tests::sleeper", "--ignored"])
            .env("POINTCAST_SLEEPER", "1")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        std::thread::sleep(Duration::from_millis(300));
        std::fs::write(&pidfile, format!("{}\n", ours.id())).unwrap();
        reap_stale(&pidfile, &exe);
        let t = Instant::now();
        while ours.try_wait().unwrap().is_none() {
            assert!(t.elapsed() < Duration::from_secs(5), "the helper left by a crash is ended");
            std::thread::sleep(Duration::from_millis(20));
        }
        let _ = std::fs::remove_dir_all(&d);
    }

    /// Stands in for a helper left running: sleeps. Run only by `a_stale_pid_file_ends_only_our_helper`.
    #[test]
    #[ignore]
    fn sleeper() {
        if std::env::var_os("POINTCAST_SLEEPER").is_some() {
            std::thread::sleep(Duration::from_secs(30));
        }
    }
}
