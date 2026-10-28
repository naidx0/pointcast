//! Tiny local HTTP + JSON-RPC MCP server exposing the latest session.
//! Binds 127.0.0.1 only. Tools: `get_latest_session`.
use crate::prompt::SessionExport;
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::thread;

#[derive(Clone, Default)]
pub struct Latest {
    inner: Arc<Mutex<Option<SessionExport>>>,
}

impl Latest {
    pub fn set(&self, exp: SessionExport) {
        *self.inner.lock().unwrap() = Some(exp);
    }
    pub fn get(&self) -> Option<SessionExport> {
        self.inner.lock().unwrap().clone()
    }
}

pub fn spawn(bind: &str, latest: Latest) -> Result<SocketAddr, String> {
    let listener = TcpListener::bind(bind).map_err(|e| e.to_string())?;
    listener.set_nonblocking(false).ok();
    let addr = listener.local_addr().map_err(|e| e.to_string())?;
    thread::Builder::new()
        .name("pointcast-mcp".into())
        .spawn(move || {
            for stream in listener.incoming().flatten() {
                let latest = latest.clone();
                thread::spawn(move || {
                    let _ = handle(stream, &latest);
                });
            }
        })
        .map_err(|e| e.to_string())?;
    Ok(addr)
}

fn handle(mut stream: TcpStream, latest: &Latest) -> Result<(), String> {
    let mut buf = vec![0u8; 64 * 1024];
    let n = stream.read(&mut buf).map_err(|e| e.to_string())?;
    let req = String::from_utf8_lossy(&buf[..n]);
    let (method_line, body) = split_http(&req);
    let (code, ctype, body) = if method_line.starts_with("GET /v1/session")
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
    let resp = format!(
        "HTTP/1.1 {code} {}\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nAccess-Control-Allow-Origin: *\r\nConnection: close\r\n\r\n{body}",
        if code == 200 { "OK" } else if code == 404 { "Not Found" } else { "Error" },
        body.len()
    );
    stream.write_all(resp.as_bytes()).map_err(|e| e.to_string())?;
    Ok(())
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
        latest.set(assemble_export(&[Item::Speech {
            t_ms: 0,
            end_ms: 10,
            text: "hello there".into(),
        }]));
        let body = r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"get_latest_session"}}"#;
        let reply = mcp_rpc(body, &latest);
        assert!(reply.contains("hello there"), "{reply}");
    }
}
