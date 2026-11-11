//! Calls over a pair of byte streams, one JSON object per line: JSON-RPC shapes without the `jsonrpc` field, which
//! is what `codex app-server` speaks on its stdio. It knows nothing of a child process, so the protocol is tested
//! over pipes inside one process. A line that is not a JSON object (a shim's notice, say) is skipped and counted.
//! `Client::rpc2` speaks strict JSON-RPC 2.0 instead, every message with `"jsonrpc": "2.0"`, as the Agent Client
//! Protocol wants (`acp`).

use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Debug, Clone, PartialEq)]
pub struct RpcError {
    pub code: i64,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum CallError {
    Rpc(RpcError),
    /// The other side is gone.
    Closed,
    /// No answer in the time allowed.
    Slow,
}

/// What the other side sent, in the order it was sent.
#[derive(Debug, Clone, PartialEq)]
pub enum Incoming {
    Notification { method: String, params: Value },
    /// A request of theirs, which wants an answer under its id.
    Request { id: Value, method: String, params: Value },
    /// The answer to a request sent with `post`.
    Reply { id: u64, result: Result<Value, RpcError> },
    Closed,
}

type Answer = Result<Value, RpcError>;

struct Inner {
    out: Mutex<Option<Box<dyn Write + Send>>>,
    next: AtomicU64,
    waiting: Mutex<HashMap<u64, Sender<Answer>>>,
    closed: AtomicBool,
    noise: AtomicU64,
    /// Every message carries `"jsonrpc": "2.0"`.
    rpc2: bool,
}

#[derive(Clone)]
pub struct Client {
    inner: Arc<Inner>,
}

impl Client {
    /// Speak over `reader` and `writer`. Everything that is not the answer to a `call` arrives on the receiver.
    pub fn over<R: Read + Send + 'static, W: Write + Send + 'static>(reader: R, writer: W) -> (Client, Receiver<Incoming>) {
        Self::speaking(reader, writer, false)
    }

    /// As `over`, in strict JSON-RPC 2.0.
    pub fn rpc2<R: Read + Send + 'static, W: Write + Send + 'static>(reader: R, writer: W) -> (Client, Receiver<Incoming>) {
        Self::speaking(reader, writer, true)
    }

    fn speaking<R: Read + Send + 'static, W: Write + Send + 'static>(reader: R, writer: W, rpc2: bool) -> (Client, Receiver<Incoming>) {
        let inner = Arc::new(Inner {
            out: Mutex::new(Some(Box::new(writer))),
            next: AtomicU64::new(1),
            waiting: Mutex::default(),
            closed: AtomicBool::new(false),
            noise: AtomicU64::new(0),
            rpc2,
        });
        let (tx, rx) = channel();
        let theirs = inner.clone();
        std::thread::spawn(move || {
            for line in BufReader::new(reader).split(b'\n') {
                let Ok(line) = line else { break };
                match sort(&line) {
                    Some(Incoming::Reply { id, result }) => {
                        let waiter = theirs.waiting.lock().unwrap().remove(&id);
                        match waiter {
                            Some(w) => {
                                let _ = w.send(result);
                            }
                            None => {
                                let _ = tx.send(Incoming::Reply { id, result });
                            }
                        }
                    }
                    Some(m) => {
                        let _ = tx.send(m);
                    }
                    None => {
                        theirs.noise.fetch_add(1, Ordering::SeqCst);
                    }
                }
            }
            theirs.closed.store(true, Ordering::SeqCst);
            theirs.waiting.lock().unwrap().clear();
            let _ = tx.send(Incoming::Closed);
        });
        (Client { inner }, rx)
    }

    /// An id for a request to come, so its sender can be ready for the answer before the request leaves.
    pub fn reserve(&self) -> u64 {
        self.inner.next.fetch_add(1, Ordering::SeqCst)
    }

    /// Send a request and wait for its answer.
    pub fn call(&self, method: &str, params: Value, within: Duration) -> Result<Value, CallError> {
        let id = self.reserve();
        let (tx, rx) = channel();
        self.inner.waiting.lock().unwrap().insert(id, tx);
        if self.post_as(id, method, params).is_err() || self.inner.closed.load(Ordering::SeqCst) {
            self.inner.waiting.lock().unwrap().remove(&id);
            return Err(CallError::Closed);
        }
        match rx.recv_timeout(within) {
            Ok(Ok(v)) => Ok(v),
            Ok(Err(e)) => Err(CallError::Rpc(e)),
            Err(RecvTimeoutError::Timeout) => {
                self.inner.waiting.lock().unwrap().remove(&id);
                Err(CallError::Slow)
            }
            Err(RecvTimeoutError::Disconnected) => Err(CallError::Closed),
        }
    }

    /// Send a request whose answer arrives as `Incoming::Reply`, in order with everything else.
    pub fn post(&self, method: &str, params: Value) -> std::io::Result<u64> {
        let id = self.reserve();
        self.post_as(id, method, params).map(|_| id)
    }

    pub fn post_as(&self, id: u64, method: &str, params: Value) -> std::io::Result<()> {
        let mut m = json!({ "method": method, "id": id });
        if !params.is_null() {
            m["params"] = params;
        }
        self.write(&m)
    }

    pub fn notify(&self, method: &str) -> std::io::Result<()> {
        self.write(&json!({ "method": method }))
    }

    pub fn notify_with(&self, method: &str, params: Value) -> std::io::Result<()> {
        self.write(&json!({ "method": method, "params": params }))
    }

    /// Answer a request of theirs.
    pub fn answer(&self, id: &Value, result: Value) -> std::io::Result<()> {
        self.write(&json!({ "id": id, "result": result }))
    }

    /// Refuse a request of theirs.
    pub fn refuse(&self, id: &Value, code: i64, message: &str) -> std::io::Result<()> {
        self.write(&json!({ "id": id, "error": { "code": code, "message": message } }))
    }

    /// Close our end, which a well-behaved helper takes as the end of its work.
    pub fn hang_up(&self) {
        self.inner.out.lock().unwrap().take();
    }

    /// Lines that were not a message.
    pub fn noise(&self) -> u64 {
        self.inner.noise.load(Ordering::SeqCst)
    }

    fn write(&self, v: &Value) -> std::io::Result<()> {
        let mut v = v.clone();
        if self.inner.rpc2 {
            v["jsonrpc"] = json!("2.0");
        }
        let mut line = serde_json::to_vec(&v)?;
        line.push(b'\n');
        let mut out = self.inner.out.lock().unwrap();
        let Some(w) = out.as_mut() else { return Err(std::io::Error::new(std::io::ErrorKind::BrokenPipe, "closed")) };
        w.write_all(&line)?;
        w.flush()
    }
}

/// What one line is, or `None` for a line that is no message.
fn sort(line: &[u8]) -> Option<Incoming> {
    let Value::Object(mut m) = serde_json::from_slice::<Value>(line).ok()? else { return None };
    let method = m.get("method").and_then(Value::as_str).map(str::to_owned);
    let id = m.remove("id").filter(|i| !i.is_null());
    match (method, id) {
        (Some(method), Some(id)) => Some(Incoming::Request { id, method, params: m.remove("params").unwrap_or(Value::Null) }),
        (Some(method), None) => Some(Incoming::Notification { method, params: m.remove("params").unwrap_or(Value::Null) }),
        (None, Some(id)) => {
            let id = id.as_u64()?;
            let result = match (m.remove("error"), m.remove("result")) {
                (Some(e), _) => Err(RpcError {
                    code: e["code"].as_i64().unwrap_or(0),
                    message: e["message"].as_str().unwrap_or_default().to_owned(),
                }),
                (None, Some(r)) => Ok(r),
                (None, None) => return None,
            };
            Some(Incoming::Reply { id, result })
        }
        (None, None) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{PipeReader, PipeWriter};

    /// A client, and the other side's two ends of the streams.
    fn pair() -> (Client, Receiver<Incoming>, BufReader<PipeReader>, PipeWriter) {
        let (their_in, our_out) = std::io::pipe().unwrap();
        let (our_in, their_out) = std::io::pipe().unwrap();
        let (c, rx) = Client::over(our_in, our_out);
        (c, rx, BufReader::new(their_in), their_out)
    }

    fn read(from: &mut BufReader<PipeReader>) -> Value {
        let mut line = String::new();
        from.read_line(&mut line).unwrap();
        serde_json::from_str(&line).unwrap()
    }

    fn next(rx: &Receiver<Incoming>) -> Incoming {
        rx.recv_timeout(Duration::from_secs(10)).unwrap()
    }

    #[test]
    fn a_request_is_one_line_with_method_id_and_params_and_no_jsonrpc_field() {
        let (c, _rx, mut theirs, _w) = pair();
        let id = c.post("model/list", json!({ "limit": 1 })).unwrap();
        assert_eq!(read(&mut theirs), json!({ "method": "model/list", "id": id, "params": { "limit": 1 } }));
        c.post("account/rateLimits/read", Value::Null).unwrap();
        assert_eq!(read(&mut theirs), json!({ "method": "account/rateLimits/read", "id": id + 1 }));
        c.notify("initialized").unwrap();
        assert_eq!(read(&mut theirs), json!({ "method": "initialized" }));
    }

    #[test]
    fn answers_reach_the_call_that_asked_even_out_of_order() {
        let (c, _rx, mut theirs, mut w) = pair();
        let ask = |method: &'static str| {
            let c = c.clone();
            std::thread::spawn(move || c.call(method, Value::Null, Duration::from_secs(10)))
        };
        let (a, b) = (ask("first"), ask("second"));
        let (x, y) = (read(&mut theirs), read(&mut theirs));
        // Answered in the other order than they arrived, each with its own method as the result.
        for m in [&y, &x] {
            writeln!(w, "{}", json!({ "id": m["id"], "result": m["method"] })).unwrap();
        }
        assert_eq!(a.join().unwrap(), Ok(json!("first")));
        assert_eq!(b.join().unwrap(), Ok(json!("second")));
    }

    #[test]
    fn an_error_answer_fails_the_call_with_its_code_and_message() {
        let (c, _rx, mut theirs, mut w) = pair();
        let t = std::thread::spawn(move || c.call("thread/start", json!({}), Duration::from_secs(10)));
        let m = read(&mut theirs);
        writeln!(w, "{}", json!({ "id": m["id"], "error": { "code": -32602, "message": "no" } })).unwrap();
        assert_eq!(t.join().unwrap(), Err(CallError::Rpc(RpcError { code: -32602, message: "no".into() })));
    }

    #[test]
    fn notes_requests_and_posted_answers_arrive_in_the_order_sent_and_noise_is_counted() {
        let (c, rx, mut theirs, mut w) = pair();
        let id = c.post("turn/start", json!({})).unwrap();
        read(&mut theirs);
        writeln!(w, "{}", json!({ "method": "turn/completed", "params": { "n": 1 } })).unwrap();
        writeln!(w).unwrap();
        writeln!(w, "running 1 test").unwrap();
        writeln!(w, "{}", json!({ "method": "item/tool/call", "id": "abc", "params": {} })).unwrap();
        writeln!(w, "{}", json!({ "id": id, "result": { "ok": true } })).unwrap();
        writeln!(w, "{}", json!({ "method": "bare" })).unwrap();
        assert_eq!(next(&rx), Incoming::Notification { method: "turn/completed".into(), params: json!({ "n": 1 }) });
        assert_eq!(next(&rx), Incoming::Request { id: json!("abc"), method: "item/tool/call".into(), params: json!({}) });
        assert_eq!(next(&rx), Incoming::Reply { id, result: Ok(json!({ "ok": true })) });
        assert_eq!(next(&rx), Incoming::Notification { method: "bare".into(), params: Value::Null });
        assert_eq!(c.noise(), 2);
        c.answer(&json!("abc"), json!({ "success": false })).unwrap();
        assert_eq!(read(&mut theirs), json!({ "id": "abc", "result": { "success": false } }));
        c.refuse(&json!(7), -32601, "Method not found").unwrap();
        assert_eq!(read(&mut theirs), json!({ "id": 7, "error": { "code": -32601, "message": "Method not found" } }));
    }

    #[test]
    fn a_stream_that_closes_fails_the_call_under_way_and_says_closed() {
        let (c, rx, mut theirs, w) = pair();
        let caller = c.clone();
        let t = std::thread::spawn(move || caller.call("initialize", json!({}), Duration::from_secs(10)));
        read(&mut theirs);
        drop(w);
        assert_eq!(t.join().unwrap(), Err(CallError::Closed));
        assert_eq!(next(&rx), Incoming::Closed);
        assert_eq!(c.call("again", Value::Null, Duration::from_secs(10)), Err(CallError::Closed));
    }

    #[test]
    fn a_call_nobody_answers_is_slow_and_a_hung_up_line_takes_no_more() {
        let (c, _rx, mut theirs, _w) = pair();
        assert_eq!(c.call("initialize", json!({}), Duration::from_millis(50)), Err(CallError::Slow));
        c.hang_up();
        assert!(c.post("model/list", json!({})).is_err());
        // The other side reads the one request, then the end.
        read(&mut theirs);
        let mut rest = String::new();
        assert_eq!(theirs.read_line(&mut rest).unwrap(), 0);
    }
}
