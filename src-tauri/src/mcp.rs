//! Tiny local HTTP + JSON-RPC MCP server exposing the latest session.
//! Binds loopback only. Tools: `get_latest_session`.
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

/// The prompt coding agents read, with the History entry it was saved as (none when History did not keep it).
#[derive(Clone, Default)]
pub struct Latest {
    inner: Arc<Mutex<Option<(Option<String>, SessionExport)>>>,
}

impl Latest {
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
            "protocolVersion": "2025-03-26",
            "capabilities": { "tools": {} },
            "serverInfo": { "name": "pointcast", "version": "0.1.0" }
        }),
        "tools/list" | "list_tools" => json!({
            "tools": [{
                "name": "get_latest_session",
                "description": "Return the most recent Pointcast session: transcript with [image n] markers, markdown, HTML, and inline PNG data URLs.",
                "inputSchema": { "type": "object", "properties": {} }
            }]
        }),
        "tools/call" | "call_tool" => {
            let name = v
                .pointer("/params/name")
                .and_then(|x| x.as_str())
                .unwrap_or("");
            if name == "get_latest_session" {
                match latest.get() {
                    Some(exp) => json!({
                        "content": [{ "type": "text", "text": serde_json::to_string_pretty(&exp).unwrap_or_default() }]
                    }),
                    None => json!({
                        "content": [{ "type": "text", "text": "No Pointcast session yet." }],
                        "isError": true
                    }),
                }
            } else {
                json!({ "content": [{ "type": "text", "text": "unknown tool" }], "isError": true })
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prompt::{assemble_export, Item};

    #[test]
    fn tools_list_names_get_latest_session() {
        let latest = Latest::default();
        let body = r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#;
        let reply = mcp_rpc(body, &latest);
        assert!(reply.contains("get_latest_session"), "{reply}");
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
