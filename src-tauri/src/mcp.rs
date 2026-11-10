//! Tiny local HTTP + JSON-RPC MCP server (protocol 2025-06-18) handing coding agents Pointcast prompts.
//! Binds loopback only. Tools: `pointcast_latest`, `pointcast_get`, `pointcast_list`, `pointcast_shot`, and
//! `get_latest_session` (the same as `pointcast_latest`, for setups made before). A prompt comes back as its text,
//! then each picture's caption and the picture as an image block (downscaled, a JPEG when its PNG is large, within a
//! token and byte budget; the rest as file links), never as base64 inside text.
//!
//! The session holds screenshots, so only a local MCP client may read it: every request must
//! carry this install's token as `Authorization: Bearer <token>`. Anything a browser sends is
//! refused before the token is looked at: an `Origin` header (Pointcast's own windows never call
//! this server, so no origin is allowed) or a `Host` that is not this computer (DNS rebinding).
//! No CORS headers are ever sent, so preflights fail too.
use crate::prompt::SessionExport;
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

/// Where saved prompts are read from: History in the data folder, and the handoff folders agents are pointed at.
#[derive(Clone, Debug)]
pub struct Library {
    pub data_dir: PathBuf,
    pub handoff_root: PathBuf,
    /// Whether the text is tidied as the person's pastes are (Settings › Output).
    pub tidy: bool,
}

impl Library {
    /// The app's own: its data folder and the handoff folder its settings name (tests get temp folders).
    pub fn app() -> Self {
        let s = crate::settings::Settings::load(&crate::settings::settings_path());
        Library { data_dir: crate::settings::data_dir(), handoff_root: crate::prompt::handoff_root(&s), tidy: s.tidy }
    }
}

/// The prompt coding agents read, with the History entry it was saved as (none when History did not keep it).
#[derive(Clone, Default)]
pub struct Latest {
    inner: Arc<Mutex<Option<(Option<String>, SessionExport)>>>,
    /// Read from here instead of the app's own folders (tests).
    library: Option<Arc<Library>>,
}

impl Latest {
    /// Read History and handoff folders from `lib` instead of the app's own.
    pub fn with_library(mut self, lib: Library) -> Self {
        self.library = Some(Arc::new(lib));
        self
    }
    fn library(&self) -> Library {
        self.library.as_deref().cloned().unwrap_or_else(Library::app)
    }
    fn current(&self) -> Option<(Option<String>, SessionExport)> {
        self.inner.lock().unwrap().clone()
    }
    pub fn set(&self, id: Option<String>, exp: SessionExport) {
        *self.inner.lock().unwrap() = Some((id, exp));
    }
    pub fn get(&self) -> Option<SessionExport> {
        self.inner.lock().unwrap().as_ref().map(|(_, exp)| exp.clone())
    }
    /// History entry `id` now reads `exp` (a picture marked up or taken out): agents read that, if it is theirs.
    pub fn changed(&self, id: &str, exp: &SessionExport) {
        if let Some((Some(mine), served)) = self.inner.lock().unwrap().as_mut() {
            if mine == id {
                *served = exp.clone();
            }
        }
    }
    /// History lost `gone` (every entry when None): when it is the one agents read, they read nothing.
    pub fn forget(&self, gone: Option<&str>) {
        let mut g = self.inner.lock().unwrap();
        if gone.is_none() || g.as_ref().is_some_and(|(id, _)| id.as_deref() == gone) {
            *g = None;
        }
    }
}

/// Where this install's token lives: in the app's data folder, never in the repo or a log.
pub fn token_path() -> PathBuf {
    crate::settings::data_dir().join("mcp-token")
}

/// This install's token, made on first run. On macOS and Linux only this user can read the file;
/// on Windows it sits in the user's own local app data, which other accounts cannot open.
pub fn token(path: &Path) -> Result<String, String> {
    if let Some(t) = read_token(path) {
        return Ok(t);
    }
    let mut bytes = [0u8; 32];
    getrandom::getrandom(&mut bytes).map_err(|e| format!("no randomness for the MCP token: {e}"))?;
    let t: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    // A damaged file is replaced; a file another start wrote in between wins.
    let _ = std::fs::remove_file(path);
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut opts, 0o600);
    match opts.open(path) {
        Ok(mut f) => {
            f.write_all(t.as_bytes()).map_err(|e| e.to_string())?;
            Ok(t)
        }
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            read_token(path).ok_or_else(|| "the MCP token file is unreadable".into())
        }
        Err(e) => Err(e.to_string()),
    }
}

fn read_token(path: &Path) -> Option<String> {
    let t = std::fs::read_to_string(path).ok()?;
    let t = t.trim();
    (t.len() == 64 && t.bytes().all(|b| b.is_ascii_hexdigit())).then(|| t.to_string())
}

/// What a coding agent needs to connect (Claude Code `.mcp.json`, Cursor `mcp.json`).
pub fn client_config(bind: &str, token: &str) -> Value {
    json!({
        "mcpServers": {
            "pointcast": {
                "type": "http",
                "url": format!("http://{bind}/mcp"),
                "headers": { "Authorization": format!("Bearer {token}") }
            }
        }
    })
}

pub fn spawn(bind: &str, latest: Latest, token: String) -> Result<SocketAddr, String> {
    if token.len() < 16 {
        return Err("the MCP token is missing".into());
    }
    let addrs: Vec<SocketAddr> = bind.to_socket_addrs().map_err(|e| e.to_string())?.collect();
    if addrs.is_empty() || addrs.iter().any(|a| !a.ip().is_loopback()) {
        return Err(format!("{bind} is not this computer only; use 127.0.0.1"));
    }
    let listener = TcpListener::bind(&addrs[..]).map_err(|e| e.to_string())?;
    listener.set_nonblocking(false).ok();
    let addr = listener.local_addr().map_err(|e| e.to_string())?;
    let token: Arc<str> = token.into();
    let open = Arc::new(AtomicUsize::new(0));
    thread::Builder::new()
        .name("pointcast-mcp".into())
        .spawn(move || {
            for mut stream in listener.incoming().flatten() {
                if open.fetch_add(1, Ordering::SeqCst) >= MAX_OPEN {
                    open.fetch_sub(1, Ordering::SeqCst);
                    busy(&mut stream);
                    continue;
                }
                let slot = Slot(open.clone());
                let deadline = Instant::now() + DEADLINE;
                let latest = latest.clone();
                let token = token.clone();
                // When the OS refuses a thread, the connection and its slot are dropped instead.
                let _ = thread::Builder::new().name("pointcast-mcp-conn".into()).spawn(move || {
                    let _slot = slot;
                    let _ = handle(stream, &latest, &token, deadline);
                });
            }
        })
        .map_err(|e| e.to_string())?;
    Ok(addr)
}

const MAX_REQUEST: usize = 1024 * 1024;

/// One connection may take this long, from being accepted to the last byte of the answer, however
/// slowly the client sends or reads.
const DEADLINE: Duration = Duration::from_secs(5);

/// Connections served at once; more are refused until one finishes.
const MAX_OPEN: usize = 16;

/// A place among the connections being served, given back when dropped.
struct Slot(Arc<AtomicUsize>);

impl Drop for Slot {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Too many connections at once: say so, read what the client already sent (closing with it unread
/// would reset the connection before the answer is read) and close, all within a tenth of a second.
fn busy(stream: &mut TcpStream) {
    let deadline = Instant::now() + Duration::from_millis(100);
    let reply = b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
    let _ = write_by(stream, reply, deadline);
    let _ = stream.shutdown(Shutdown::Write);
    let mut sink = [0u8; 4096];
    while let Ok(wait) = left(deadline) {
        stream.set_read_timeout(Some(wait)).ok();
        if !matches!(stream.read(&mut sink), Ok(n) if n > 0) {
            break;
        }
    }
}

/// The time left before `deadline`, or an error once it has passed.
fn left(deadline: Instant) -> Result<Duration, String> {
    let d = deadline.saturating_duration_since(Instant::now());
    if d.is_zero() {
        Err("the client took too long".into())
    } else {
        Ok(d)
    }
}

fn write_by(stream: &mut TcpStream, mut bytes: &[u8], deadline: Instant) -> Result<(), String> {
    while !bytes.is_empty() {
        stream.set_write_timeout(Some(left(deadline)?)).ok();
        let n = stream.write(bytes).map_err(|e| e.to_string())?;
        if n == 0 {
            return Err("the connection closed".into());
        }
        bytes = &bytes[n..];
    }
    Ok(())
}

/// Read the head, then as much body as Content-Length says: the headers must be whole to check them.
fn read_request(stream: &mut TcpStream, deadline: Instant) -> Result<String, String> {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 16 * 1024];
    loop {
        stream.set_read_timeout(Some(left(deadline)?)).ok();
        let n = stream.read(&mut chunk).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
        if let Some(end) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            let head = String::from_utf8_lossy(&buf[..end]);
            let len = header(&head, "content-length")
                .and_then(|v| v.parse::<usize>().ok())
                .unwrap_or(0);
            if buf.len() >= end + 4 + len {
                break;
            }
        }
        if buf.len() > MAX_REQUEST {
            break;
        }
    }
    Ok(String::from_utf8_lossy(&buf).into_owned())
}

fn header<'a>(head: &'a str, name: &str) -> Option<&'a str> {
    head.lines().skip(1).find_map(|l| {
        let (k, v) = l.split_once(':')?;
        k.trim().eq_ignore_ascii_case(name).then(|| v.trim())
    })
}

/// A Host header, when present, must name this computer.
fn local_host(head: &str) -> bool {
    let Some(h) = header(head, "host") else {
        return true;
    };
    let name = match h.strip_prefix('[') {
        Some(v6) => v6.split(']').next().unwrap_or(""),
        None => h.rsplit_once(':').map_or(h, |(n, _)| n),
    };
    matches!(
        name.to_ascii_lowercase().as_str(),
        "127.0.0.1" | "localhost" | "::1"
    )
}

fn authorized(head: &str, token: &str) -> bool {
    let Some((scheme, got)) = header(head, "authorization").and_then(|v| v.split_once(' ')) else {
        return false;
    };
    let (a, b) = (got.trim().as_bytes(), token.as_bytes());
    // Compare every byte, so the time taken does not say how much of a guess was right.
    scheme.eq_ignore_ascii_case("bearer")
        && a.len() == b.len()
        && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn handle(mut stream: TcpStream, latest: &Latest, token: &str, deadline: Instant) -> Result<(), String> {
    let req = read_request(&mut stream, deadline)?;
    let head = req.split("\r\n\r\n").next().unwrap_or("");
    let (method_line, body) = split_http(&req);
    let (code, ctype, body) = if header(head, "origin").is_some() || !local_host(head) {
        (403, "text/plain", "web pages cannot read Pointcast".to_string())
    } else if !authorized(head, token) {
        (
            401,
            "text/plain",
            "missing or wrong token: copy the MCP setup again from Pointcast Settings".to_string(),
        )
    } else if method_line.starts_with("GET /v1/session")
        || method_line.starts_with("GET /session")
        || method_line.starts_with("GET / ")
    {
        match latest.get() {
            Some(exp) => (
                200,
                "application/json",
                serde_json::to_string_pretty(&exp).unwrap_or_else(|_| "{}".into()),
            ),
            None => (
                404,
                "application/json",
                "{\"error\":\"no session yet\"}".into(),
            ),
        }
    } else if method_line.starts_with("POST /mcp") || method_line.starts_with("POST /") {
        let reply = mcp_rpc(body, latest);
        (200, "application/json", reply)
    } else if method_line.starts_with("GET /health") {
        (200, "text/plain", "ok".into())
    } else {
        (404, "text/plain", "not found".into())
    };
    let reason = match code {
        200 => "OK",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        _ => "Error",
    };
    let challenge = if code == 401 {
        "WWW-Authenticate: Bearer realm=\"pointcast\"\r\n"
    } else {
        ""
    };
    let resp = format!(
        "HTTP/1.1 {code} {reason}\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\n{challenge}Connection: close\r\n\r\n{body}",
        body.len()
    );
    write_by(&mut stream, resp.as_bytes(), deadline)
}

fn split_http(req: &str) -> (&str, &str) {
    let method_line = req.lines().next().unwrap_or("");
    let body = req.split("\r\n\r\n").nth(1).unwrap_or("");
    (method_line, body)
}

fn mcp_rpc(body: &str, latest: &Latest) -> String {
    let v: Value = serde_json::from_str(body).unwrap_or(json!({}));
    let id = v.get("id").cloned().unwrap_or(Value::Null);
    let method = v.get("method").and_then(|m| m.as_str()).unwrap_or("");
    let result = match method {
        "initialize" => json!({
            "protocolVersion": protocol_for(v.pointer("/params/protocolVersion").and_then(|x| x.as_str())),
            "capabilities": { "tools": {} },
            "serverInfo": { "name": "pointcast", "version": "0.1.0" }
        }),
        "tools/list" | "list_tools" => json!({ "tools": tools() }),
        "tools/call" | "call_tool" => {
            let name = v
                .pointer("/params/name")
                .and_then(|x| x.as_str())
                .unwrap_or("");
            let args = v.pointer("/params/arguments").cloned().unwrap_or(json!({}));
            // A tool that cannot do what was asked answers with an error result the model reads, never a panic.
            match call_tool(name, &args, latest) {
                Ok(r) => r,
                Err(e) => json!({ "content": [{ "type": "text", "text": e }], "isError": true }),
            }
        }
        "notifications/initialized" => return "{}".into(),
        _ => {
            return serde_json::to_string(&json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": { "code": -32601, "message": "method not found" }
            }))
            .unwrap();
        }
    };
    serde_json::to_string(&json!({ "jsonrpc": "2.0", "id": id, "result": result })).unwrap()
}

/// The protocol this server speaks, and the one it gave every client before (still given to a client that asks for
/// an older version).
const PROTOCOL: &str = "2025-06-18";
const OLD_PROTOCOL: &str = "2025-03-26";

/// Inline pictures are downscaled so their longer edge is at most this many pixels.
pub const MAX_EDGE: u32 = 1568;
/// Images inlined in one result, in estimated tokens (width * height / 750 after downscaling); the rest are links.
pub const IMAGE_BUDGET: u64 = 18_000;
/// Inline images in one result, in bytes of base64; the rest are links. The first picture within the token budget is
/// inlined whatever its size, so a result shows at least one.
pub const INLINE_BYTES: usize = 1_500_000;
/// An inline picture whose PNG is larger than this is sent as a JPEG of `JPEG_QUALITY` (when that is smaller).
const JPEG_OVER: usize = 256 * 1024;
const JPEG_QUALITY: u8 = 82;
/// The prompt text in a result is cut past this many bytes (clients set long text results aside as files).
const TEXT_MAX: usize = 16_000;

/// The protocol version to answer an `initialize` with.
fn protocol_for(asked: Option<&str>) -> &'static str {
    match asked {
        Some(v) if v < PROTOCOL => OLD_PROTOCOL,
        _ => PROTOCOL,
    }
}

fn tools() -> Value {
    let images = json!({
        "type": "string",
        "enum": ["inline", "links", "none"],
        "default": "inline",
        "description": "inline: each picture as an image (downscaled to 1568 px, JPEG when large; about 18,000 tokens and 1.5 MB in all; the rest as file links). links: file links only. none: text only."
    });
    let copy_to = json!({
        "type": "string",
        "description": "An absolute path to an existing folder, usually your project. The pictures and prompt.md are copied into <copy_to>/.pointcast/<stamp>/ and the result names those files. Use it when you can only read files inside your project."
    });
    let id = json!({ "type": "string", "description": "A prompt id from pointcast_list, or \"latest\"." });
    let latest = json!({
        "type": "object",
        "properties": { "images": images, "copy_to": copy_to }
    });
    let latest_text = "The prompt the person last dictated with Pointcast: their words with [image n] markers, then each screenshot as an image with its caption. Structured content lists every picture's file.";
    json!([
        { "name": "pointcast_latest", "description": latest_text, "inputSchema": latest },
        {
            "name": "pointcast_get",
            "description": "One prompt from Pointcast History by id (see pointcast_list): words with [image n] markers, then each screenshot as an image with its caption.",
            "inputSchema": {
                "type": "object",
                "properties": { "id": id, "images": images, "copy_to": copy_to },
                "required": ["id"]
            }
        },
        {
            "name": "pointcast_list",
            "description": "The newest prompts in Pointcast History: id, when, how long, the first words and how many pictures.",
            "inputSchema": {
                "type": "object",
                "properties": { "limit": { "type": "integer", "minimum": 1, "maximum": 100, "default": 10 } }
            }
        },
        {
            "name": "pointcast_shot",
            "description": "One picture of a prompt at full size, as [image n] numbers it.",
            "inputSchema": {
                "type": "object",
                "properties": { "id": id, "n": { "type": "integer", "minimum": 1 } },
                "required": ["id", "n"]
            }
        },
        {
            "name": "get_latest_session",
            "description": format!("Same as pointcast_latest (kept for existing setups). {latest_text}"),
            "inputSchema": latest
        }
    ])
}

/// One picture of a prompt, in prompt order.
struct Pic {
    n: u32,
    kind: crate::prompt::CaptureKind,
    width: u32,
    height: u32,
    labels: Vec<crate::prompt::Label>,
    pointing_at: Option<String>,
    /// Its file, with forward slashes, when it has one.
    file: Option<String>,
    png: Vec<u8>,
}

/// A prompt as the tools hand it out.
struct Prompt {
    /// Whether its text is tidied (the library's setting when it was read).
    tidy: bool,
    /// The History entry; none for a prompt History did not keep.
    id: Option<String>,
    created: Option<String>,
    /// A folder name for copies: `YYYY-MM-DD_HHMM`.
    stamp: String,
    /// The session, when History has it (the text is then made again for copied files).
    items: Option<Vec<crate::prompt::Item>>,
    plain: String,
    pics: Vec<Pic>,
}

fn local_time(ms: u64) -> chrono::DateTime<chrono::Local> {
    use chrono::TimeZone;
    chrono::Local.timestamp_millis_opt(ms as i64).single().unwrap_or_else(chrono::Local::now)
}

fn slashes(p: &Path) -> String {
    p.to_string_lossy().replace(std::path::MAIN_SEPARATOR, "/")
}

/// `button "Save"`, as the prompt names the control pointed at.
fn element_name(e: &crate::element::ElementMark) -> Option<String> {
    let name = e.name.split_whitespace().collect::<Vec<_>>().join(" ").replace('"', "'");
    let name: String = name.chars().take(60).collect();
    let role = e.role.trim();
    let s = match (role.is_empty(), name.is_empty()) {
        (_, true) => role.to_string(),
        (true, false) => format!("\"{name}\""),
        (false, false) => format!("{role} \"{name}\""),
    };
    (!s.is_empty()).then_some(s)
}

/// History entry `id`. Each picture's file is its handoff copy while that still holds the same picture, else History's own.
fn from_library(lib: &Library, id: &str) -> Result<Prompt, String> {
    let store = crate::history::Store::new(&lib.data_dir);
    let items = store.items(id).map_err(|_| format!("No Pointcast prompt with id {id:?}; pointcast_list names them."))?;
    let meta = store.list().into_iter().find(|m| m.id == id);
    let (_, caps) = crate::prompt::compose(&items);
    let kept = store.prompt_files(id);
    let handoff = crate::prompt::find_handoff(&lib.handoff_root, id);
    let pics: Vec<Pic> = caps
        .into_iter()
        .enumerate()
        .map(|(j, c)| {
            let copy = handoff.as_ref().and_then(|(dir, m)| crate::prompt::handoff_copy(dir, m, &c));
            Pic {
                n: c.id,
                kind: c.kind,
                width: c.width,
                height: c.height,
                pointing_at: c.element.as_ref().and_then(element_name),
                labels: c.labels,
                file: copy.or_else(|| kept.get(j).cloned()),
                png: c.png,
            }
        })
        .collect();
    let files: Vec<String> = pics.iter().filter_map(|p| p.file.clone()).collect();
    let files = if files.len() == pics.len() { files } else { vec![] };
    let plain = crate::prompt::assemble_clipboard_files(&items, &files, lib.tidy).plain;
    let created_ms = meta.as_ref().map(|m| m.created_ms);
    let stamp = match &handoff {
        Some((dir, _)) => dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
        None => local_time(created_ms.unwrap_or(0)).format("%Y-%m-%d_%H%M").to_string(),
    };
    Ok(Prompt {
        tidy: lib.tidy,
        id: Some(id.to_string()),
        created: created_ms.map(|ms| local_time(ms).to_rfc3339_opts(chrono::SecondsFormat::Secs, false)),
        stamp,
        items: Some(items),
        plain,
        pics,
    })
}

/// The prompt as it was served, for one History did not keep (or cannot read now): its pictures from the export.
fn from_export(id: Option<String>, exp: SessionExport) -> Prompt {
    let pics = exp
        .images
        .into_iter()
        .map(|im| {
            let b64 = im.data_url.split_once(',').map_or("", |(_, d)| d);
            let png = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, b64).unwrap_or_default();
            // The picture list names the file: "[image n] `path`  caption".
            let head = format!("[image {}] `", im.n);
            let file = exp
                .plain
                .lines()
                .find_map(|l| l.strip_prefix(&head).and_then(|r| r.split_once('`')).map(|(f, _)| f.to_string()));
            Pic { n: im.n, kind: im.kind, width: im.width, height: im.height, labels: im.labels, pointing_at: None, file, png }
        })
        .collect();
    Prompt {
        tidy: false,
        id,
        created: None,
        stamp: chrono::Local::now().format("%Y-%m-%d_%H%M").to_string(),
        items: None,
        plain: exp.plain,
        pics,
    }
}

fn latest_prompt(latest: &Latest) -> Result<Prompt, String> {
    match latest.current() {
        None => Err("No Pointcast session yet.".into()),
        Some((Some(id), exp)) => Ok(from_library(&latest.library(), &id).unwrap_or_else(|_| from_export(Some(id), exp))),
        Some((None, exp)) => Ok(from_export(None, exp)),
    }
}

fn get_prompt(latest: &Latest, id: &str) -> Result<Prompt, String> {
    if id == "latest" {
        latest_prompt(latest)
    } else {
        from_library(&latest.library(), id)
    }
}

/// `region 933x73 · pointing at button "Save" · pins: 1=too far left`.
fn caption(p: &Pic) -> String {
    let mut bits = vec![format!("{} {}x{}", p.kind.as_str(), p.width, p.height)];
    bits.extend(p.pointing_at.iter().map(|e| format!("pointing at {e}")));
    let pins: Vec<String> = p
        .labels
        .iter()
        .map(|l| match l.text.trim() {
            "" => l.n.to_string(),
            t => format!("{}={t}", l.n),
        })
        .collect();
    if !pins.is_empty() {
        bits.push(format!("pins: {}", pins.join("; ")));
    }
    bits.join(" · ")
}

/// The size a picture is inlined at: its longer edge at most `MAX_EDGE`.
fn fit(w: u32, h: u32) -> (u32, u32) {
    let long = w.max(h);
    if long <= MAX_EDGE {
        return (w.max(1), h.max(1));
    }
    let k = MAX_EDGE as f64 / long as f64;
    let side = |v: u32| ((v as f64 * k).round() as u32).clamp(1, MAX_EDGE);
    (side(w), side(h))
}

fn image_tokens(w: u32, h: u32) -> u64 {
    (w as u64 * h as u64).div_ceil(750)
}

fn decoded(png: &[u8]) -> Result<image::RgbaImage, String> {
    let (rgba, w, h) = crate::image_ops::decode_rgba(png)?;
    image::RgbaImage::from_raw(w, h, rgba).ok_or_else(|| "a picture did not decode".to_string())
}

/// A picture as an image block's PNG: as it is when small enough, else downscaled to `fit` (with its pixels).
fn inline_png(png: &[u8]) -> Result<(Vec<u8>, Option<image::RgbaImage>), String> {
    let (w, h) = crate::image_ops::png_size(png).ok_or("a picture is not a PNG")?;
    let (tw, th) = fit(w, h);
    if (tw, th) == (w, h) {
        return Ok((png.to_vec(), None));
    }
    let small = image::imageops::resize(&decoded(png)?, tw, th, image::imageops::FilterType::Triangle);
    Ok((crate::image_ops::encode_png(small.as_raw(), tw, th)?, Some(small)))
}

/// A picture as an inline image block's bytes and type: the PNG of `inline_png`, or when that is larger than
/// `JPEG_OVER`, a JPEG of the same pixels if it is smaller still. The files on disk stay PNG.
fn inline_image(png: &[u8]) -> Result<(Vec<u8>, &'static str), String> {
    let (small, pixels) = inline_png(png)?;
    if small.len() <= JPEG_OVER {
        return Ok((small, "image/png"));
    }
    let pixels = match pixels {
        Some(p) => p,
        None => decoded(png)?,
    };
    let rgb = image::DynamicImage::ImageRgba8(pixels).to_rgb8();
    let mut jpeg = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, JPEG_QUALITY)
        .encode(rgb.as_raw(), rgb.width(), rgb.height(), image::ExtendedColorType::Rgb8)
        .map_err(|e| e.to_string())?;
    Ok(if jpeg.len() < small.len() { (jpeg, "image/jpeg") } else { (small, "image/png") })
}

fn image_block(bytes: &[u8], mime: &str) -> Value {
    json!({
        "type": "image",
        "data": base64::Engine::encode(&base64::engine::general_purpose::STANDARD, bytes),
        "mimeType": mime
    })
}

/// The length of `n` bytes in base64.
fn b64_len(n: usize) -> usize {
    n.div_ceil(3) * 4
}

/// `file:///C:/Users/max/Pointcast/2026-11-09_1432/01.png`, with anything unusual percent-encoded.
fn file_uri(path: &str) -> String {
    let p = path.replace(std::path::MAIN_SEPARATOR, "/");
    let mut out = String::from("file://");
    if !p.starts_with('/') {
        out.push('/');
    }
    for b in p.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~/:".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// `s` cut at `max` bytes, on a character boundary, saying so.
fn cut(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{} [... cut here; the whole prompt is in prompt.md]", &s[..end])
}

#[derive(Clone, Copy, PartialEq)]
enum Images {
    Inline,
    Links,
    None,
}

fn images_arg(args: &Value) -> Result<Images, String> {
    match args.get("images").map(|v| v.as_str()) {
        None | Some(Some("inline")) => Ok(Images::Inline),
        Some(Some("links")) => Ok(Images::Links),
        Some(Some("none")) => Ok(Images::None),
        _ => Err("images must be \"inline\", \"links\" or \"none\".".into()),
    }
}

/// Copy a prompt's pictures and `prompt.md` into `<to>/.pointcast/<stamp>/` and point the prompt at the copies.
/// Writes nothing anywhere else: `to` must be an absolute path to an existing folder, and `.pointcast` in it a plain
/// folder (made when missing). Returns what was written.
fn copy_out(p: &mut Prompt, to: &str) -> Result<String, String> {
    let base = Path::new(to.trim());
    if to.trim().is_empty() || !base.is_absolute() {
        return Err(format!("copy_to must be an absolute folder path, such as your project folder; got {to:?}. Nothing was written."));
    }
    match std::fs::metadata(base) {
        Ok(m) if m.is_dir() => {}
        Ok(_) => return Err(format!("copy_to {to:?} is a file, not a folder. Nothing was written.")),
        Err(_) => return Err(format!("copy_to {to:?} does not exist. Nothing was written.")),
    }
    let root = base.join(".pointcast");
    match std::fs::symlink_metadata(&root) {
        Ok(m) if m.file_type().is_symlink() || !m.is_dir() => {
            return Err(format!("{} is not a plain folder. Nothing was written.", slashes(&root)))
        }
        Ok(_) => {}
        Err(_) => std::fs::create_dir(&root).map_err(|e| format!("could not make {}: {e}", slashes(&root)))?,
    }
    let mut dir = None;
    for k in 1..1000 {
        let d = root.join(if k == 1 { p.stamp.clone() } else { format!("{}-{k}", p.stamp) });
        match std::fs::create_dir(&d) {
            Ok(()) => {
                dir = Some(d);
                break;
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(format!("could not make a folder in {}: {e}", slashes(&root))),
        }
    }
    let dir = dir.ok_or("no free folder name in .pointcast")?;
    let width = if p.pics.len() >= 100 { 3 } else { 2 };
    let files: Vec<String> = p.pics.iter().map(|pic| format!("{}/{:0width$}.png", slashes(&dir), pic.n)).collect();
    let plain = match &p.items {
        Some(items) => crate::prompt::assemble_clipboard_files(items, &files, p.tidy).plain,
        None => {
            let mut t = p.plain.clone();
            let mut named = !p.pics.is_empty();
            for (pic, f) in p.pics.iter().zip(&files) {
                match &pic.file {
                    Some(old) if t.contains(old.as_str()) => t = t.replace(old.as_str(), f),
                    _ => named = false,
                }
            }
            if !named && !files.is_empty() {
                let rows: Vec<String> = p.pics.iter().zip(&files).map(|(pic, f)| format!("[image {}] `{f}`  {}", pic.n, caption(pic))).collect();
                t = format!("{t}\n\nPictures ({}):\n{}", rows.len(), rows.join("\n"));
            }
            t
        }
    };
    let write = || -> Result<(), String> {
        for (pic, f) in p.pics.iter().zip(&files) {
            std::fs::write(f, &pic.png).map_err(|e| e.to_string())?;
        }
        std::fs::write(dir.join("prompt.md"), format!("{plain}\n")).map_err(|e| e.to_string())
    };
    if let Err(e) = write() {
        let _ = std::fs::remove_dir_all(&dir);
        return Err(format!("could not copy into {}: {e}. Nothing was left there.", slashes(&dir)));
    }
    for (pic, f) in p.pics.iter_mut().zip(files) {
        pic.file = Some(f);
    }
    p.plain = plain;
    let n = p.pics.len();
    Ok(format!("Copied {n} picture{} and prompt.md to `{}/`.", if n == 1 { "" } else { "s" }, slashes(&dir)))
}

/// A prompt result: the prompt's text, then for each picture its caption and the picture (an image within the
/// budget, else a link to its file), with the manifest as structured content.
fn prompt_result(mut p: Prompt, images: Images, copy_to: Option<&str>) -> Result<Value, String> {
    let copied = match copy_to {
        Some(to) => Some(copy_out(&mut p, to)?),
        None => None,
    };
    let mut content = vec![];
    let mut notes = vec![];
    let mut spent = 0u64;
    let mut spent_bytes = 0usize;
    let mut linked = vec![];
    // Each picture's inline image size in bytes (before base64); 0 when it is not inlined.
    let mut inlined = vec![0usize; p.pics.len()];
    if images != Images::None {
        let mut room = images == Images::Inline;
        for (k, pic) in p.pics.iter().enumerate() {
            content.push(json!({ "type": "text", "text": format!("[image {}] {}", pic.n, caption(pic)) }));
            let (w, h) = crate::image_ops::png_size(&pic.png).unwrap_or((pic.width, pic.height));
            let (fw, fh) = fit(w, h);
            let t = image_tokens(fw, fh);
            if room && spent + t <= IMAGE_BUDGET {
                if let Ok((bytes, mime)) = inline_image(&pic.png) {
                    let b = b64_len(bytes.len());
                    if spent_bytes == 0 || spent_bytes + b <= INLINE_BYTES {
                        spent += t;
                        spent_bytes += b;
                        inlined[k] = bytes.len();
                        content.push(image_block(&bytes, mime));
                        continue;
                    }
                }
            }
            if images == Images::Inline {
                room = false;
            }
            linked.push(pic.n);
            if let Some(f) = &pic.file {
                let name = f.rsplit('/').next().unwrap_or(f);
                content.push(json!({
                    "type": "resource_link",
                    "uri": file_uri(f),
                    "name": name,
                    "description": format!("[image {}] {}", pic.n, caption(pic)),
                    "mimeType": "image/png"
                }));
            }
        }
    }
    let id = p.id.clone().unwrap_or_else(|| "latest".into());
    if !linked.is_empty() {
        let which: Vec<String> = linked.iter().map(|n| n.to_string()).collect();
        let why = if images == Images::Inline { " (past the image budget of this result)" } else { "" };
        notes.push(format!(
            "Pictures {} are links, not images{why}: open the file, or call pointcast_shot with id \"{id}\" and n to see one.",
            which.join(", ")
        ));
    }
    if images == Images::None && !p.pics.is_empty() {
        notes.push(format!("{} pictures not shown: call pointcast_shot with id \"{id}\" and n to see one.", p.pics.len()));
    }
    notes.extend(copied);
    let mut text = if p.plain.trim().is_empty() { "(no words)".to_string() } else { cut(&p.plain, TEXT_MAX) };
    for n in notes {
        text.push_str("\n\n");
        text.push_str(&n);
    }
    content.insert(0, json!({ "type": "text", "text": text }));
    let pictures: Vec<Value> = p
        .pics
        .iter()
        .zip(&inlined)
        .map(|(pic, inl)| {
            json!({
                "n": pic.n,
                "file": pic.file,
                "kind": pic.kind,
                "width": pic.width,
                "height": pic.height,
                "labels": pic.labels,
                "pointing_at": pic.pointing_at,
                "inline": *inl > 0,
                "inline_bytes": inl
            })
        })
        .collect();
    Ok(json!({
        "content": content,
        "structuredContent": { "id": p.id, "created": p.created, "pictures": pictures }
    }))
}

fn list_result(latest: &Latest, args: &Value) -> Result<Value, String> {
    let limit = match args.get("limit") {
        None | Some(Value::Null) => 10,
        Some(v) => v.as_u64().filter(|n| *n >= 1).ok_or("limit must be a whole number of at least 1.")?.min(100) as usize,
    };
    let store = crate::history::Store::new(&latest.library().data_dir);
    let rows: Vec<Value> = store
        .list()
        .into_iter()
        .take(limit)
        .map(|m| {
            json!({
                "id": m.id,
                "created": local_time(m.created_ms).to_rfc3339_opts(chrono::SecondsFormat::Secs, false),
                "duration_ms": m.duration_ms,
                "preview": m.preview.chars().take(80).collect::<String>(),
                "pictures": m.images
            })
        })
        .collect();
    let lines: Vec<String> = rows
        .iter()
        .map(|r| {
            let secs = r["duration_ms"].as_u64().unwrap_or(0) / 1000;
            format!(
                "{}  {}  {}:{:02}  {} pictures  {}",
                r["id"].as_str().unwrap_or(""),
                r["created"].as_str().unwrap_or(""),
                secs / 60,
                secs % 60,
                r["pictures"],
                r["preview"].as_str().unwrap_or("")
            )
        })
        .collect();
    let text = if lines.is_empty() { "Pointcast History is empty.".to_string() } else { lines.join("\n") };
    Ok(json!({
        "content": [{ "type": "text", "text": text }],
        "structuredContent": { "prompts": rows }
    }))
}

fn shot_result(latest: &Latest, args: &Value) -> Result<Value, String> {
    let id = args.get("id").and_then(Value::as_str).ok_or("pointcast_shot needs an id (or \"latest\") and n.")?;
    let n = args.get("n").and_then(Value::as_u64).ok_or("pointcast_shot needs n, the picture's number (1 for [image 1]).")?;
    let p = get_prompt(latest, id)?;
    let pic = p
        .pics
        .iter()
        .find(|pic| pic.n as u64 == n)
        .ok_or_else(|| format!("There is no [image {n}]: this prompt has {} picture(s).", p.pics.len()))?;
    Ok(json!({
        "content": [
            { "type": "text", "text": format!("[image {}] {}", pic.n, caption(pic)) },
            image_block(&pic.png, "image/png")
        ]
    }))
}

fn call_tool(name: &str, args: &Value, latest: &Latest) -> Result<Value, String> {
    let copy_to = |args: &Value| -> Result<Option<String>, String> {
        match args.get("copy_to") {
            None | Some(Value::Null) => Ok(None),
            Some(Value::String(s)) => Ok(Some(s.clone())),
            Some(_) => Err("copy_to must be a folder path.".into()),
        }
    };
    match name {
        "pointcast_latest" | "get_latest_session" => {
            let images = images_arg(args)?;
            let to = copy_to(args)?;
            prompt_result(latest_prompt(latest)?, images, to.as_deref())
        }
        "pointcast_get" => {
            let id = args.get("id").and_then(Value::as_str).ok_or("pointcast_get needs an id; pointcast_list names them.")?;
            let images = images_arg(args)?;
            let to = copy_to(args)?;
            prompt_result(get_prompt(latest, id)?, images, to.as_deref())
        }
        "pointcast_list" => list_result(latest, args),
        "pointcast_shot" => shot_result(latest, args),
        _ => Err("unknown tool".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prompt::{assemble_export, Item};

    #[test]
    fn tools_list_names_the_five_tools_with_schemas() {
        let latest = Latest::default();
        let body = r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#;
        let reply: Value = serde_json::from_str(&mcp_rpc(body, &latest)).unwrap();
        let tools = reply["result"]["tools"].as_array().unwrap();
        let mut names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
        names.sort();
        assert_eq!(names, ["get_latest_session", "pointcast_get", "pointcast_latest", "pointcast_list", "pointcast_shot"]);
        for t in tools {
            let s = &t["inputSchema"];
            assert_eq!(s["type"], "object", "{t}");
            assert!(s["properties"].is_object(), "{t}");
            assert!(!t["description"].as_str().unwrap().is_empty(), "{t}");
        }
        let schema = |n: &str| tools.iter().find(|t| t["name"] == n).unwrap()["inputSchema"].clone();
        assert_eq!(schema("pointcast_get")["required"], json!(["id"]));
        assert_eq!(schema("pointcast_shot")["required"], json!(["id", "n"]));
        assert_eq!(schema("pointcast_latest")["properties"]["images"]["enum"], json!(["inline", "links", "none"]));
        assert_eq!(schema("pointcast_latest"), schema("get_latest_session"));
        assert_eq!(schema("pointcast_list")["properties"]["limit"]["type"], "integer");
    }

    #[test]
    fn initialize_declares_2025_06_18_and_answers_older_clients_as_before() {
        let latest = Latest::default();
        let ask = |v: Option<&str>| -> String {
            let params = v.map_or(json!({}), |v| json!({ "protocolVersion": v }));
            let body = json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":params}).to_string();
            let r: Value = serde_json::from_str(&mcp_rpc(&body, &latest)).unwrap();
            r["result"]["protocolVersion"].as_str().unwrap().to_string()
        };
        assert_eq!(ask(Some("2025-06-18")), "2025-06-18");
        assert_eq!(ask(None), "2025-06-18");
        assert_eq!(ask(Some("2025-03-26")), "2025-03-26");
        assert_eq!(ask(Some("2024-11-05")), "2025-03-26");
    }

    // Tools over saved prompts: each test owns a temp folder for History and the handoff folders.
    const CREATED: u64 = 1_762_700_000_000;

    struct Lab {
        root: PathBuf,
        lib: Library,
    }

    impl Drop for Lab {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    fn lab(name: &str) -> Lab {
        let root = std::env::temp_dir().join(format!("pc-mcp-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let lib = Library { data_dir: root.join("data"), handoff_root: root.join("handoff"), tidy: false };
        Lab { root, lib }
    }

    /// A PNG of `w`x`h` pixels that compresses like a photo, not like a flat colour.
    fn noisy_png(w: u32, h: u32, seed: u32) -> Vec<u8> {
        let mut x = seed.wrapping_mul(2_654_435_761).max(1);
        let mut buf = Vec::with_capacity((w * h * 4) as usize);
        for _ in 0..w * h {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            buf.extend_from_slice(&[x as u8, (x >> 8) as u8, (x >> 16) as u8, 255]);
        }
        crate::image_ops::encode_png(&buf, w, h).unwrap()
    }

    fn pic(t: u64, png: &[u8], labels: Vec<crate::prompt::Label>) -> Item {
        let (width, height) = crate::image_ops::png_size(png).unwrap();
        Item::Image {
            t_ms: t,
            capture: crate::prompt::Capture {
                id: 0,
                kind: crate::prompt::CaptureKind::Region,
                t_ms: t,
                png: png.to_vec(),
                width,
                height,
                labels,
                pointer: None,
                trail: vec![],
                element: None,
                place: None,
                anchors: vec![],
                auto: false,
            },
        }
    }

    fn words(t: u64, end: u64, text: &str) -> Item {
        Item::Speech { t_ms: t, end_ms: end, text: text.into() }
    }

    /// Save `items` to History at `created` (and a handoff folder when `handoff`); agents read it as the latest.
    fn saved(lab: &Lab, items: &[Item], created: u64, handoff: bool) -> (Latest, String) {
        let store = crate::history::Store::new(&lab.lib.data_dir);
        let id = store.save(items, created, 5_000).unwrap().id;
        if handoff {
            crate::prompt::write_handoff(&lab.lib.handoff_root, &id, created, items, created, lab.lib.tidy).unwrap();
        }
        let latest = Latest::default().with_library(lab.lib.clone());
        latest.set(Some(id.clone()), store.export_with(&id, true, lab.lib.tidy).unwrap());
        (latest, id)
    }

    fn call(latest: &Latest, name: &str, args: Value) -> Value {
        let body = json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":name,"arguments":args}});
        let reply: Value = serde_json::from_str(&mcp_rpc(&body.to_string(), latest)).unwrap();
        assert!(reply.get("error").is_none(), "{reply}");
        reply["result"].clone()
    }

    fn kinds(r: &Value) -> Vec<String> {
        r["content"].as_array().unwrap().iter().map(|c| c["type"].as_str().unwrap().to_string()).collect()
    }

    fn texts(r: &Value) -> Vec<String> {
        r["content"].as_array().unwrap().iter().filter_map(|c| c["text"].as_str().map(String::from)).collect()
    }

    /// Each image block's type and bytes, decoded from base64.
    fn image_blocks(r: &Value) -> Vec<(String, Vec<u8>)> {
        r["content"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|c| c["type"] == "image")
            .map(|c| {
                let bytes = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, c["data"].as_str().unwrap()).unwrap();
                (c["mimeType"].as_str().unwrap().to_string(), bytes)
            })
            .collect()
    }

    /// The width and height of each image block, after decoding it as the type it says (PNG or JPEG).
    fn image_sizes(r: &Value) -> Vec<(u32, u32)> {
        image_blocks(r)
            .into_iter()
            .map(|(mime, bytes)| {
                let format = match mime.as_str() {
                    "image/png" => image::ImageFormat::Png,
                    "image/jpeg" => image::ImageFormat::Jpeg,
                    other => panic!("an image block of type {other}"),
                };
                let img = image::load_from_memory_with_format(&bytes, format).unwrap();
                (img.width(), img.height())
            })
            .collect()
    }

    /// A full screen as a screenshot looks: flat panels and soft gradients with a little sensor-like grain, so its
    /// PNG is large and a JPEG of it is not.
    fn screen_png(w: u32, h: u32, seed: u32) -> Vec<u8> {
        let mut x = seed.wrapping_mul(2_654_435_761).max(1);
        let mut buf = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h {
            for xx in 0..w {
                x ^= x << 13;
                x ^= x >> 17;
                x ^= x << 5;
                let grain = (x % 17) as i32 - 8;
                let panel = if (xx / 240 + y / 160) % 2 == 0 { 40 } else { 0 };
                let base = [(xx * 200 / w) as i32 + panel, (y * 200 / h) as i32, 160 + panel / 2];
                let px = |k: usize| (base[k] + grain).clamp(0, 255) as u8;
                buf.extend_from_slice(&[px(0), px(1), px(2), 255]);
            }
        }
        crate::image_ops::encode_png(&buf, w, h).unwrap()
    }

    fn base64_total(r: &Value) -> usize {
        r["content"].as_array().unwrap().iter().filter(|c| c["type"] == "image").map(|c| c["data"].as_str().unwrap().len()).sum()
    }

    /// No text block carries picture bytes or runs long enough for a client to set it aside as a file.
    fn assert_lean_text(r: &Value) {
        for t in texts(r) {
            assert!(!t.contains("base64") && !t.contains("data:image"), "picture bytes in a text block: {}", &t[..t.len().min(200)]);
            assert!(t.chars().count() <= 20_000, "a text block of {} characters", t.chars().count());
        }
    }

    fn uri_path(uri: &str) -> PathBuf {
        let p = uri.strip_prefix("file://").unwrap();
        let p = if p.as_bytes().get(2) == Some(&b':') { &p[1..] } else { p };
        let bytes = p.as_bytes();
        let mut out = Vec::new();
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] == b'%' {
                out.push(u8::from_str_radix(&p[i + 1..i + 3], 16).unwrap());
                i += 3;
            } else {
                out.push(bytes[i]);
                i += 1;
            }
        }
        PathBuf::from(String::from_utf8(out).unwrap())
    }

    #[test]
    fn a_prompt_with_two_pictures_returns_text_then_caption_and_image_for_each() {
        let lab = lab("two");
        let (a, b) = (noisy_png(40, 30, 1), noisy_png(50, 20, 2));
        let pin = vec![crate::prompt::Label { n: 1, text: "too far left".into() }];
        let items = [words(0, 4_000, "the save button sits here and the toast covers it"), pic(1_000, &a, pin), pic(3_000, &b, vec![])];
        let (latest, id) = saved(&lab, &items, CREATED, true);
        for name in ["pointcast_latest", "get_latest_session"] {
            let r = call(&latest, name, json!({}));
            assert_ne!(r["isError"], true, "{r}");
            assert_eq!(kinds(&r), ["text", "text", "image", "text", "image"], "{name}");
            assert_lean_text(&r);
            let t = texts(&r);
            assert!(t[0].contains("the save button") && t[0].contains("[image 1]") && t[0].contains("[image 2]"), "{}", t[0]);
            assert!(t[1].starts_with("[image 1] region 40x30"), "{}", t[1]);
            assert!(t[1].contains("pins: 1=too far left"), "{}", t[1]);
            assert!(t[2].starts_with("[image 2] region 50x20"), "{}", t[2]);
            assert_eq!(image_sizes(&r), [(40, 30), (50, 20)]);
            let m = &r["structuredContent"];
            assert_eq!(m["id"], id.as_str());
            assert!(m["created"].as_str().is_some_and(|c| c.starts_with("20")), "{m}");
            let pics = m["pictures"].as_array().unwrap();
            assert_eq!(pics.len(), 2);
            for (k, p) in pics.iter().enumerate() {
                assert_eq!(p["n"], k as u64 + 1);
                assert_eq!(p["kind"], "region");
                let file = p["file"].as_str().unwrap();
                assert!(file.starts_with(&lab.lib.handoff_root.to_string_lossy().replace(std::path::MAIN_SEPARATOR, "/")), "{file}");
                assert!(t[0].contains(file), "the prompt names {file}: {}", t[0]);
                assert!(Path::new(file).is_file(), "{file}");
            }
            assert_eq!((pics[0]["width"].as_u64(), pics[0]["height"].as_u64()), (Some(40), Some(30)));
            assert_eq!(pics[0]["labels"][0]["text"], "too far left");
        }
        // pointcast_get by id reads the same prompt.
        let r = call(&latest, "pointcast_get", json!({ "id": id }));
        let mut shown = r.clone();
        for c in shown["content"].as_array_mut().unwrap() {
            if let Some(d) = c.get_mut("data") {
                *d = json!(format!("{}...({} chars)", &d.as_str().unwrap()[..24], d.as_str().unwrap().len()));
            }
        }
        eprintln!("EXAMPLE: {}", serde_json::to_string_pretty(&shown).unwrap());
        assert_eq!(kinds(&r), ["text", "text", "image", "text", "image"]);
    }

    #[test]
    fn a_picture_past_1568_pixels_comes_back_downscaled_and_full_size_from_pointcast_shot() {
        let _alone = crate::test_support::alone();
        let lab = lab("big");
        let big = noisy_png(2400, 1000, 3);
        let (latest, id) = saved(&lab, &[words(0, 2_000, "look"), pic(500, &big, vec![])], CREATED, true);
        let r = call(&latest, "pointcast_latest", json!({}));
        assert_eq!(image_sizes(&r), [(1568, 653)]);
        assert_lean_text(&r);
        let r = call(&latest, "pointcast_shot", json!({ "id": id, "n": 1 }));
        assert_ne!(r["isError"], true, "{r}");
        assert_eq!(kinds(&r), ["text", "image"]);
        assert_eq!(image_sizes(&r), [(2400, 1000)]);
    }

    #[test]
    fn thirty_large_pictures_stay_within_the_image_budget_and_the_rest_are_links() {
        let _alone = crate::test_support::alone();
        let lab = lab("thirty");
        let png = noisy_png(1600, 1000, 4);
        let mut items = vec![words(0, 60_000, "a long walk through the app")];
        items.extend((0..30).map(|k| pic(1_000 + k * 1_000, &png, vec![])));
        let (latest, id) = saved(&lab, &items, CREATED, true);
        let r = call(&latest, "pointcast_latest", json!({}));
        assert_lean_text(&r);
        let sizes = image_sizes(&r);
        let tokens: u64 = sizes.iter().map(|(w, h)| (*w as u64 * *h as u64).div_ceil(750)).sum();
        assert!(!sizes.is_empty(), "some pictures are inlined");
        assert!(tokens <= 18_000, "{tokens} tokens of images");
        assert!(sizes.iter().all(|s| *s == (1568, 980)), "{sizes:?}");
        let content = r["content"].as_array().unwrap();
        let links: Vec<&Value> = content.iter().filter(|c| c["type"] == "resource_link").collect();
        assert_eq!(links.len() + sizes.len(), 30);
        for l in &links {
            let uri = l["uri"].as_str().unwrap();
            assert!(uri.starts_with("file:///"), "{uri}");
            assert!(uri_path(uri).is_file(), "{uri}");
            assert_eq!(l["mimeType"], "image/png");
        }
        // Every picture is introduced in order, inlined or linked.
        let captions: Vec<String> = texts(&r).into_iter().skip(1).collect();
        assert_eq!(captions.len(), 30);
        for (k, c) in captions.iter().enumerate() {
            assert!(c.starts_with(&format!("[image {}] ", k + 1)), "{c}");
        }
        let first = &texts(&r)[0];
        assert!(first.contains("pointcast_shot") && first.contains(&id), "{first}");
    }

    #[test]
    fn fifteen_full_screen_pictures_fit_one_result() {
        let _alone = crate::test_support::alone();
        let lab = lab("fifteen");
        let png = noisy_png(1920, 1080, 5);
        let mut items = vec![words(0, 30_000, "fifteen screens")];
        items.extend((0..15).map(|k| pic(1_000 + k * 1_000, &png, vec![])));
        let (latest, _) = saved(&lab, &items, CREATED, true);
        let r = call(&latest, "pointcast_latest", json!({}));
        assert_lean_text(&r);
        let sizes = image_sizes(&r);
        let tokens: u64 = sizes.iter().map(|(w, h)| (*w as u64 * *h as u64).div_ceil(750)).sum();
        let text_chars: usize = texts(&r).iter().map(|t| t.len()).sum();
        let bytes = r.to_string().len();
        assert!(!sizes.is_empty() && tokens <= 18_000, "{} images, {tokens} tokens", sizes.len());
        assert!(sizes.iter().all(|(w, h)| *w <= 1568 && *h <= 1568), "{sizes:?}");
        assert_eq!(r["structuredContent"]["pictures"].as_array().unwrap().len(), 15);
        let shown = r["content"].as_array().unwrap().iter().filter(|c| c["type"] == "image" || c["type"] == "resource_link").count();
        assert_eq!(shown, 15, "every picture is an image or a link");
        eprintln!(
            "FIFTEEN: {} inlined ({tokens} image tokens), {} links, {text_chars} text characters (~{} tokens), {bytes} bytes of JSON",
            sizes.len(),
            15 - sizes.len(),
            text_chars / 4
        );
    }

    #[test]
    fn a_large_picture_comes_back_as_a_jpeg_and_pointcast_shot_and_the_files_stay_png() {
        let _alone = crate::test_support::alone();
        let lab = lab("jpeg");
        let screen = screen_png(1920, 1080, 11);
        assert!(screen.len() > 1_000_000, "the fixture is a heavy PNG: {} bytes", screen.len());
        let (latest, id) = saved(&lab, &[words(0, 2_000, "this screen"), pic(500, &screen, vec![])], CREATED, true);
        let r = call(&latest, "pointcast_latest", json!({}));
        let blocks = image_blocks(&r);
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].0, "image/jpeg");
        assert!(blocks[0].1.len() < 500_000, "a JPEG of {} bytes", blocks[0].1.len());
        assert_eq!(image_sizes(&r), [(1568, 882)]);
        let pics = r["structuredContent"]["pictures"].as_array().unwrap();
        assert_eq!(pics[0]["inline_bytes"].as_u64(), Some(blocks[0].1.len() as u64));
        assert_eq!(pics[0]["inline"], true);
        let file = pics[0]["file"].as_str().unwrap();
        assert_eq!(std::fs::read(file).unwrap(), screen, "the file on disk is the PNG");
        // copy_to copies stay PNG.
        let project = lab.root.join("project");
        std::fs::create_dir_all(&project).unwrap();
        let r = call(&latest, "pointcast_latest", json!({ "copy_to": project.to_string_lossy() }));
        let copied = r["structuredContent"]["pictures"][0]["file"].as_str().unwrap().to_string();
        assert!(copied.ends_with(".png"), "{copied}");
        assert_eq!(std::fs::read(&copied).unwrap(), screen);
        // pointcast_shot: one picture at full size, PNG.
        let r = call(&latest, "pointcast_shot", json!({ "id": id, "n": 1 }));
        let blocks = image_blocks(&r);
        assert_eq!(blocks[0].0, "image/png");
        assert_eq!(blocks[0].1, screen);
    }

    #[test]
    fn a_small_picture_stays_png() {
        let lab = lab("small-png");
        let small = noisy_png(120, 80, 12);
        assert!(small.len() < 256 * 1024);
        let (latest, _) = saved(&lab, &[words(0, 500, "small"), pic(100, &small, vec![])], CREATED, true);
        let r = call(&latest, "pointcast_latest", json!({}));
        let blocks = image_blocks(&r);
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0], ("image/png".to_string(), small.clone()), "sent as it is");
        assert_eq!(r["structuredContent"]["pictures"][0]["inline_bytes"].as_u64(), Some(small.len() as u64));
    }

    #[test]
    fn two_full_screen_pictures_give_a_response_under_1_6_mb() {
        let _alone = crate::test_support::alone();
        let lab = lab("two-screens");
        let (a, b) = (screen_png(1920, 1080, 13), screen_png(1920, 1080, 14));
        let items = [words(0, 4_000, "two screens"), pic(1_000, &a, vec![]), pic(3_000, &b, vec![])];
        let (latest, _) = saved(&lab, &items, CREATED, true);
        let r = call(&latest, "pointcast_latest", json!({}));
        let bytes = r.to_string().len();
        assert_eq!(image_sizes(&r), [(1568, 882), (1568, 882)], "both inlined");
        // What the same result weighed with PNG image blocks.
        let png_b64: usize = [&a, &b].iter().map(|p| b64_len(inline_png(p).unwrap().0.len())).sum();
        let before = bytes - base64_total(&r) + png_b64;
        assert!(bytes < 1_600_000, "{bytes} bytes of JSON (with PNG blocks it was {before})");
        assert!(before > bytes, "PNG blocks: {before} bytes, JPEG: {bytes}");
        eprintln!("TWO SCREENS: {bytes} bytes of JSON; with PNG blocks {before}");
    }

    #[test]
    fn the_byte_budget_turns_the_overflow_into_links() {
        let _alone = crate::test_support::alone();
        let lab = lab("bytes");
        // Pure noise at 800x600: few tokens each (640), many bytes even as a JPEG.
        let mut items = vec![words(0, 9_000, "noise")];
        items.extend((0..6).map(|k| pic(1_000 + k * 1_000, &noisy_png(800, 600, 20 + k as u32), vec![])));
        let (latest, _) = saved(&lab, &items, CREATED, true);
        let r = call(&latest, "pointcast_latest", json!({}));
        let sizes = image_sizes(&r);
        let tokens: u64 = sizes.iter().map(|(w, h)| (*w as u64 * *h as u64).div_ceil(750)).sum();
        let links = r["content"].as_array().unwrap().iter().filter(|c| c["type"] == "resource_link").count();
        assert!(!sizes.is_empty() && links > 0, "{} images, {links} links", sizes.len());
        assert_eq!(sizes.len() + links, 6);
        assert!(6 * 640 < IMAGE_BUDGET && tokens < IMAGE_BUDGET, "the token budget did not bind");
        assert!(base64_total(&r) <= INLINE_BYTES, "{} bytes of base64", base64_total(&r));
        let pics = r["structuredContent"]["pictures"].as_array().unwrap();
        let inline: Vec<bool> = pics.iter().map(|p| p["inline_bytes"].as_u64().unwrap() > 0).collect();
        assert_eq!(inline.iter().filter(|i| **i).count(), sizes.len());
        assert!(inline.windows(2).all(|w| w[0] || !w[1]), "the first pictures inline, the rest links: {inline:?}");
        assert!(texts(&r)[0].contains("past the image budget"), "{}", texts(&r)[0]);
    }

    #[test]
    fn images_none_returns_only_text_and_links_returns_only_links() {
        let lab = lab("modes");
        let items = [words(0, 500, "two shots"), pic(1_000, &noisy_png(40, 30, 6), vec![]), pic(2_000, &noisy_png(30, 40, 7), vec![])];
        let (latest, _) = saved(&lab, &items, CREATED, true);
        let r = call(&latest, "pointcast_latest", json!({ "images": "none" }));
        assert_eq!(kinds(&r), ["text"]);
        assert!(texts(&r)[0].contains("two shots"));
        assert_eq!(r["structuredContent"]["pictures"].as_array().unwrap().len(), 2);
        let r = call(&latest, "pointcast_latest", json!({ "images": "links" }));
        assert_eq!(kinds(&r), ["text", "text", "resource_link", "text", "resource_link"]);
        assert_lean_text(&r);
        let r = call(&latest, "pointcast_latest", json!({ "images": "all of them" }));
        assert_eq!(r["isError"], true, "{r}");
    }

    #[test]
    fn pointcast_list_gives_the_newest_up_to_the_limit() {
        let lab = lab("list");
        let long = "word ".repeat(60);
        saved(&lab, &[words(0, 1_000, "the oldest one")], CREATED, false);
        saved(&lab, &[words(0, 1_000, &long), pic(500, &noisy_png(8, 8, 8), vec![])], CREATED + 60_000, false);
        let (latest, newest) = saved(&lab, &[words(0, 1_000, "the newest one")], CREATED + 120_000, false);
        let r = call(&latest, "pointcast_list", json!({ "limit": 2 }));
        assert_ne!(r["isError"], true, "{r}");
        let rows = r["structuredContent"]["prompts"].as_array().unwrap();
        assert_eq!(rows.len(), 2, "{r}");
        assert_eq!(rows[0]["id"], newest.as_str());
        assert_eq!(rows[1]["pictures"], 1);
        assert!(rows[1]["preview"].as_str().unwrap().chars().count() <= 80, "{}", rows[1]["preview"]);
        assert!(rows[0]["duration_ms"].is_u64() && rows[0]["created"].is_string(), "{}", rows[0]);
        assert!(texts(&r)[0].contains(&newest), "{}", texts(&r)[0]);
        assert!(!texts(&r)[0].contains("the oldest one"));
        let r = call(&latest, "pointcast_list", json!({}));
        assert_eq!(r["structuredContent"]["prompts"].as_array().unwrap().len(), 3);
        // The older entry is still there to read by id.
        let older = r["structuredContent"]["prompts"][1]["id"].as_str().unwrap().to_string();
        let r = call(&latest, "pointcast_get", json!({ "id": older, "images": "inline" }));
        assert_eq!(kinds(&r), ["text", "text", "image"]);
        assert_eq!(image_sizes(&r), [(8, 8)]);
    }

    #[test]
    fn pointcast_shot_out_of_range_or_unknown_is_an_error_result() {
        let lab = lab("shot");
        let items = [words(0, 4_000, "two"), pic(1_000, &noisy_png(20, 10, 9), vec![]), pic(2_000, &noisy_png(10, 20, 10), vec![])];
        let (latest, id) = saved(&lab, &items, CREATED, true);
        assert_eq!(image_sizes(&call(&latest, "pointcast_shot", json!({ "id": id, "n": 2 }))), [(10, 20)]);
        for args in [
            json!({ "id": id, "n": 3 }),
            json!({ "id": id, "n": 0 }),
            json!({ "id": id, "n": -1 }),
            json!({ "id": id }),
            json!({ "n": 1 }),
            json!({ "id": "../../etc", "n": 1 }),
            json!({ "id": "no-such-entry", "n": 1 }),
        ] {
            let r = call(&latest, "pointcast_shot", args.clone());
            assert_eq!(r["isError"], true, "{args}: {r}");
            assert_eq!(kinds(&r), ["text"], "{args}");
        }
        assert_eq!(call(&latest, "pointcast_get", json!({})).get("isError"), Some(&json!(true)));
        assert_eq!(call(&latest, "pointcast_get", json!({ "id": "nope" }))["isError"], true);
    }

    /// Every file and folder under `dir`, relative, with forward slashes.
    fn tree(dir: &Path) -> Vec<String> {
        let mut out = vec![];
        let mut stack = vec![dir.to_path_buf()];
        while let Some(d) = stack.pop() {
            for e in std::fs::read_dir(&d).unwrap().flatten() {
                let p = e.path();
                out.push(p.strip_prefix(dir).unwrap().to_string_lossy().replace(std::path::MAIN_SEPARATOR, "/"));
                if p.is_dir() {
                    stack.push(p);
                }
            }
        }
        out.sort();
        out
    }

    #[test]
    fn copy_to_writes_only_under_the_projects_pointcast_folder() {
        let lab = lab("copy");
        let items = [words(0, 500, "copy these"), pic(1_000, &noisy_png(40, 30, 11), vec![]), pic(2_000, &noisy_png(30, 40, 12), vec![])];
        let (latest, _) = saved(&lab, &items, CREATED, true);
        let project = lab.root.join("project");
        std::fs::create_dir_all(&project).unwrap();
        std::fs::write(project.join("main.rs"), "fn main() {}").unwrap();
        let a_file = project.join("main.rs").to_string_lossy().into_owned();
        let missing = project.join("nope").to_string_lossy().into_owned();
        for bad in ["relative/folder", "project", a_file.as_str(), missing.as_str(), ""] {
            let r = call(&latest, "pointcast_latest", json!({ "copy_to": bad }));
            assert_eq!(r["isError"], true, "{bad}: {r}");
        }
        assert_eq!(tree(&project), ["main.rs"], "a refused copy writes nothing");

        let r = call(&latest, "pointcast_latest", json!({ "copy_to": project.to_string_lossy() }));
        assert_ne!(r["isError"], true, "{r}");
        assert_eq!(kinds(&r), ["text", "text", "image", "text", "image"]);
        let written = tree(&project);
        assert!(written.iter().all(|p| p == "main.rs" || p.starts_with(".pointcast")), "{written:?}");
        let base = project.join(".pointcast").to_string_lossy().replace(std::path::MAIN_SEPARATOR, "/");
        let pics = r["structuredContent"]["pictures"].as_array().unwrap();
        let text = &texts(&r)[0];
        for p in pics {
            let f = p["file"].as_str().unwrap();
            assert!(f.starts_with(&format!("{base}/")), "{f}");
            assert!(Path::new(f).is_file(), "{f}");
            assert!(text.contains(f), "{text}");
        }
        let folder = Path::new(pics[0]["file"].as_str().unwrap()).parent().unwrap().to_path_buf();
        let mut inside = tree(&folder);
        inside.sort();
        assert_eq!(inside, ["01.png", "02.png", "prompt.md"]);
        let md = std::fs::read_to_string(folder.join("prompt.md")).unwrap();
        assert!(md.contains("copy these") && md.contains(pics[1]["file"].as_str().unwrap()), "{md}");
        assert!(text.contains("Copied 2 pictures and prompt.md"), "{text}");
        assert_eq!(std::fs::read(folder.join("02.png")).unwrap(), noisy_png(30, 40, 12));

        // A second copy gets its own folder; a `.pointcast` that is not a folder is refused.
        let r = call(&latest, "pointcast_latest", json!({ "copy_to": project.to_string_lossy(), "images": "none" }));
        assert_ne!(r["isError"], true, "{r}");
        assert_eq!(std::fs::read_dir(project.join(".pointcast")).unwrap().count(), 2);
        let other = lab.root.join("other");
        std::fs::create_dir_all(&other).unwrap();
        std::fs::write(other.join(".pointcast"), "mine").unwrap();
        let r = call(&latest, "pointcast_latest", json!({ "copy_to": other.to_string_lossy() }));
        assert_eq!(r["isError"], true, "{r}");
        assert_eq!(std::fs::read_to_string(other.join(".pointcast")).unwrap(), "mine");
    }

    #[test]
    fn tools_call_returns_the_stored_session() {
        let latest = Latest::default();
        latest.set(None, assemble_export(&[Item::Speech {
            t_ms: 0,
            end_ms: 10,
            text: "hello there".into(),
        }]));
        let body = r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"get_latest_session"}}"#;
        let reply = mcp_rpc(body, &latest);
        assert!(reply.contains("hello there"), "{reply}");
    }

    // Over the wire: a web page open in the browser must never read the session; a local MCP
    // client (no Origin, the install's token) must.
    const TOKEN: &str = "test-token-3f9a1c0d2b7e4a58";
    const SECRET: &str = "the private words on screen";
    const EVIL: &str = "https://evil.example";
    const CALL: &str =
        r#"{"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"get_latest_session"}}"#;

    fn start() -> SocketAddr {
        let latest = Latest::default();
        latest.set(None, assemble_export(&[Item::Speech {
            t_ms: 0,
            end_ms: 10,
            text: SECRET.into(),
        }]));
        spawn("127.0.0.1:0", latest, TOKEN.into()).unwrap()
    }

    /// Send one raw request; return the status code and the whole response.
    fn send(addr: SocketAddr, req: &str) -> (u16, String) {
        let mut s = TcpStream::connect(addr).unwrap();
        s.set_read_timeout(Some(std::time::Duration::from_secs(5))).unwrap();
        s.write_all(req.as_bytes()).unwrap();
        let mut out = String::new();
        let _ = s.read_to_string(&mut out);
        let code = out.split(' ').nth(1).and_then(|c| c.parse().ok()).unwrap_or(0);
        (code, out)
    }

    fn request(addr: SocketAddr, method_path: &str, headers: &str, body: &str) -> (u16, String) {
        send(
            addr,
            &format!(
                "{method_path} HTTP/1.1\r\nHost: {addr}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n{headers}\r\n{body}",
                body.len()
            ),
        )
    }

    fn bearer(token: &str) -> String {
        format!("Authorization: Bearer {token}\r\n")
    }

    fn allows_cors(resp: &str) -> bool {
        let head = resp.split("\r\n\r\n").next().unwrap_or("").to_ascii_lowercase();
        head.contains("access-control-allow-origin")
    }

    /// Every route the server answers, plus one it does not.
    const ROUTES: [&str; 7] = [
        "GET /v1/session",
        "GET /session",
        "GET /",
        "GET /health",
        "POST /mcp",
        "POST /",
        "GET /nope",
    ];

    #[test]
    fn a_web_page_is_refused_even_with_the_token() {
        let addr = start();
        for route in ROUTES {
            let headers = format!("Origin: {EVIL}\r\n{}", bearer(TOKEN));
            let (code, resp) = request(addr, route, &headers, CALL);
            assert_eq!(code, 403, "{route}: {resp}");
            assert!(!allows_cors(&resp), "{route} must not send CORS headers: {resp}");
            assert!(!resp.contains(SECRET), "{route} leaked the session: {resp}");
        }
    }

    #[test]
    fn a_preflight_from_another_origin_is_refused() {
        let addr = start();
        let headers = format!(
            "Origin: {EVIL}\r\nAccess-Control-Request-Method: POST\r\nAccess-Control-Request-Headers: authorization, content-type\r\n"
        );
        let (code, resp) = request(addr, "OPTIONS /mcp", &headers, "");
        assert_eq!(code, 403, "{resp}");
        assert!(!allows_cors(&resp), "{resp}");
    }

    #[test]
    fn a_request_without_the_token_is_refused() {
        let addr = start();
        for route in ROUTES {
            let (code, resp) = request(addr, route, "", CALL);
            assert_eq!(code, 401, "{route}: {resp}");
            assert!(!resp.contains(SECRET), "{route} leaked the session: {resp}");
        }
    }

    #[test]
    fn a_wrong_token_is_refused() {
        let addr = start();
        for token in ["", "nope", &TOKEN[..TOKEN.len() - 1], &format!("{TOKEN}x")] {
            let (code, resp) = request(addr, "POST /mcp", &bearer(token), CALL);
            assert_eq!(code, 401, "token {token:?}: {resp}");
            assert!(!resp.contains(SECRET), "{resp}");
        }
    }

    #[test]
    fn a_rebound_host_name_is_refused() {
        // DNS rebinding: a page on evil.example resolved to 127.0.0.1 sends its own host name.
        let addr = start();
        let req = format!(
            "GET /v1/session HTTP/1.1\r\nHost: evil.example:{}\r\n{}\r\n",
            addr.port(),
            bearer(TOKEN)
        );
        let (code, resp) = send(addr, &req);
        assert_eq!(code, 403, "{resp}");
        assert!(!resp.contains(SECRET), "{resp}");
    }

    #[test]
    fn a_cleared_session_answers_no_session_not_an_empty_prompt() {
        let latest = Latest::default();
        latest.set(None, assemble_export(&[Item::Speech {
            t_ms: 0,
            end_ms: 10,
            text: SECRET.into(),
        }]));
        let addr = spawn("127.0.0.1:0", latest.clone(), TOKEN.into()).unwrap();
        latest.forget(None);
        let (code, resp) = request(addr, "POST /mcp", &bearer(TOKEN), CALL);
        assert_eq!(code, 200, "{resp}");
        assert!(resp.contains("No Pointcast session yet."), "{resp}");
        assert!(resp.contains("isError"), "{resp}");
        assert!(!resp.contains(SECRET), "{resp}");
        let (code, resp) = request(addr, "GET /v1/session", &bearer(TOKEN), "");
        assert_eq!(code, 404, "{resp}");
        assert!(resp.contains("no session yet"), "{resp}");
        assert!(!resp.contains(SECRET), "{resp}");
    }

    #[test]
    fn a_local_client_with_the_token_reads_the_session() {
        let addr = start();
        let (code, resp) = request(addr, "POST /mcp", &bearer(TOKEN), CALL);
        assert_eq!(code, 200, "{resp}");
        assert!(resp.contains(SECRET), "{resp}");
        assert!(!allows_cors(&resp), "{resp}");
        let (code, resp) = request(addr, "GET /v1/session", &bearer(TOKEN), "");
        assert_eq!(code, 200, "{resp}");
        assert!(resp.contains(SECRET), "{resp}");
    }

    #[test]
    fn the_server_listens_on_loopback_only() {
        for bind in ["0.0.0.0:0", "[::]:0"] {
            assert!(spawn(bind, Latest::default(), TOKEN.into()).is_err(), "{bind} must be refused");
        }
    }

    #[test]
    fn the_token_is_made_once_and_kept() {
        let dir = std::env::temp_dir().join(format!("pc-mcp-token-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let (a, b) = (dir.join("a").join("mcp-token"), dir.join("b").join("mcp-token"));
        let first = token(&a).unwrap();
        assert_eq!(first.len(), 64, "{first}");
        assert!(first.bytes().all(|c| c.is_ascii_hexdigit()), "{first}");
        assert_eq!(token(&a).unwrap(), first, "a second start keeps the token");
        assert_eq!(std::fs::read_to_string(&a).unwrap(), first);
        assert_ne!(token(&b).unwrap(), first, "every install gets its own token");
        // A damaged file is replaced, not trusted.
        std::fs::write(&a, "short").unwrap();
        let fresh = token(&a).unwrap();
        assert_eq!(fresh.len(), 64, "{fresh}");
        assert_ne!(fresh, "short");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&a).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600, "only this user may read the token");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_client_that_dribbles_its_request_is_cut_off_at_the_deadline() {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = l.local_addr().unwrap();
        // A byte every 50 ms for 3 s: no single read waits long.
        let slow = thread::spawn(move || {
            let mut c = TcpStream::connect(addr).unwrap();
            for _ in 0..60 {
                if c.write_all(b"X").is_err() {
                    break;
                }
                thread::sleep(Duration::from_millis(50));
            }
        });
        let (mut s, _) = l.accept().unwrap();
        let t = Instant::now();
        let got = read_request(&mut s, Instant::now() + Duration::from_millis(300));
        assert!(got.is_err(), "{got:?}");
        assert!(t.elapsed() < Duration::from_secs(1), "{:?}", t.elapsed());
        drop(s);
        slow.join().unwrap();
    }

    #[test]
    fn connections_past_the_cap_are_refused_until_one_closes() {
        let addr = start();
        let idle: Vec<TcpStream> = (0..MAX_OPEN).map(|_| TcpStream::connect(addr).unwrap()).collect();
        let (code, resp) = request(addr, "GET /health", &bearer(TOKEN), "");
        assert_eq!(code, 503, "{resp}");
        assert!(!resp.contains(SECRET), "{resp}");
        drop(idle);
        let t = Instant::now();
        loop {
            let (code, resp) = request(addr, "GET /health", &bearer(TOKEN), "");
            if code == 200 {
                break;
            }
            assert!(t.elapsed() < Duration::from_secs(3), "still refused: {resp}");
            thread::sleep(Duration::from_millis(20));
        }
    }

    #[test]
    fn a_too_short_token_does_not_start_the_server() {
        assert!(spawn("127.0.0.1:0", Latest::default(), String::new()).is_err());
    }

    #[test]
    fn the_copied_setup_carries_the_token_and_reaches_the_server() {
        let addr = start();
        let cfg = client_config(&addr.to_string(), TOKEN);
        let server = &cfg["mcpServers"]["pointcast"];
        assert_eq!(server["type"], "http");
        let url = server["url"].as_str().unwrap();
        assert_eq!(url, format!("http://{addr}/mcp"));
        let auth = server["headers"]["Authorization"].as_str().unwrap();
        assert_eq!(auth, format!("Bearer {TOKEN}"));
        // Exactly what a client would send from that config.
        let (code, resp) = request(addr, "POST /mcp", &format!("Authorization: {auth}\r\n"), CALL);
        assert_eq!(code, 200, "{resp}");
        assert!(resp.contains(SECRET), "{resp}");
    }
}
