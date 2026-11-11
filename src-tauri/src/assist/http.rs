//! The line to a service that answers over HTTP: an OpenAI-compatible API on the network, or a model server on
//! this computer. A question is one request on a thread of its own, and its answer is read as server-sent events
//! while it arrives. A key rides in the `Authorization` header (`x-api-key` for Anthropic) and nowhere else: this is the only file that puts a
//! `Secret` on the wire, and what a server says back is shown with the key taken out.

use super::vault::Secret;
use serde_json::Value;
use std::io::{BufRead, BufReader, Read};
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub const CONNECT_WITHIN: Duration = Duration::from_secs(10);

/// How long an answer may say nothing before the line counts as lost. An answer that keeps coming has no limit.
pub const QUIET: Duration = Duration::from_secs(20);

/// How much of what a server said about a refusal is kept, in characters.
const SAID: usize = 300;

/// The most that is read of an answer that is not a stream.
const WHOLE: u64 = 8 * 1024 * 1024;

/// Why a request got no answer, or why its answer ended.
#[derive(Debug, Clone, PartialEq)]
pub enum Trouble {
    Stopped,
    /// Nobody answered: refused, no route, no name, no secure line, or silence for too long.
    Offline,
    /// The server answered no. `resets` is when it will take a question again (Unix seconds), `said` its own
    /// words in one line.
    Status { code: u16, resets: Option<i64>, said: String },
    Broken(String),
}

/// One request: a POST of `body` as JSON, or a GET without one. `within` is the wait for the answer to start and
/// for each further piece of it.
pub struct Ask {
    pub url: String,
    pub key: Option<Arc<Secret>>,
    pub body: Option<Value>,
    pub within: Duration,
}

/// What a stream tells its sink: each event's data as it arrives, then how it ended, once.
pub enum Piece {
    Data(String),
    End(Result<(), Trouble>),
}

/// Takes the pieces of one stream; `false` after a piece of data means no more is wanted.
pub type Sink = Box<dyn FnMut(Piece) -> bool + Send>;

/// Ends one stream from any thread. The sink is told `Stopped` at once and nothing after it; the connection is
/// let go when the reading thread next wakes (the next bytes, or the quiet time).
#[derive(Clone)]
pub struct Stopper(Arc<Mutex<Option<Sink>>>);

impl Stopper {
    pub fn stop(&self) {
        self.end(Err(Trouble::Stopped));
    }

    fn end(&self, how: Result<(), Trouble>) {
        let sink = self.0.lock().unwrap().take();
        if let Some(mut on) = sink {
            on(Piece::End(how));
        }
    }

    /// Hand on one event; `false` when the stream has ended or the sink wants no more.
    fn data(&self, d: String) -> bool {
        match self.0.lock().unwrap().as_mut() {
            Some(on) => on(Piece::Data(d)),
            None => false,
        }
    }
}

/// How a service is spoken to: OpenAI's chat API, with the key as a bearer, or Anthropic's Messages API, with the key
/// in `x-api-key` and the version of the API named.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Wire {
    OpenAi,
    Anthropic,
}

/// The version of Anthropic's Messages API the requests are written for.
pub const ANTHROPIC_VERSION: &str = "2023-06-01";

/// The connections to one service, kept between questions so the next one does not start from nothing.
#[derive(Clone)]
pub struct Line {
    client: reqwest::blocking::Client,
    wire: Wire,
}

impl Line {
    /// `direct` goes straight to the address, past any proxy: the way to a server on this computer.
    pub fn new(direct: bool) -> Option<Line> {
        let mut b = reqwest::blocking::Client::builder()
            .user_agent(concat!("Pointcast/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(CONNECT_WITHIN)
            // The wait for an answer to start, then for each read of it: never a limit on the whole answer.
            .timeout(QUIET);
        if direct {
            b = b.no_proxy();
        }
        b.build().ok().map(|client| Line { client, wire: Wire::OpenAi })
    }

    /// The same line, speaking `wire`.
    pub fn speaking(self, wire: Wire) -> Line {
        Line { wire, ..self }
    }

    fn send(&self, ask: &Ask) -> Result<reqwest::blocking::Response, Trouble> {
        let mut req = match &ask.body {
            Some(body) => self.client.post(&ask.url).header("Content-Type", "application/json").body(body.to_string()),
            None => self.client.get(&ask.url),
        };
        if let Some(key) = &ask.key {
            req = match self.wire {
                Wire::OpenAi => req.bearer_auth(key.expose()),
                Wire::Anthropic => req.header("x-api-key", key.expose()),
            };
        }
        if self.wire == Wire::Anthropic {
            req = req.header("anthropic-version", ANTHROPIC_VERSION);
        }
        let resp = req.timeout(ask.within).send().map_err(|e| {
            if e.is_builder() {
                Trouble::Broken("That address is not one Pointcast can ask.".into())
            } else {
                Trouble::Offline
            }
        })?;
        if resp.status().is_success() {
            return Ok(resp);
        }
        let code = resp.status().as_u16();
        let headers = resp.headers().clone();
        let header = |name: &str| headers.get(name).and_then(|v| v.to_str().ok()).map(str::to_owned);
        let resets = resets(header, chrono::Utc::now().timestamp());
        Err(Trouble::Status { code, resets, said: said(&text(resp, 64 * 1024), ask.key.as_deref()) })
    }

    /// The whole of a short answer.
    pub fn whole(&self, ask: &Ask) -> Result<String, Trouble> {
        Ok(text(self.send(ask)?, WHOLE))
    }

    /// Send `ask` on a thread of its own and hand the answer's events to `on` as they arrive. The stream ends
    /// with exactly one `End`, from here or from the stopper.
    pub fn stream(&self, ask: Ask, on: Sink) -> Stopper {
        let stopper = Stopper(Arc::new(Mutex::new(Some(on))));
        let (line, theirs) = (self.clone(), stopper.clone());
        std::thread::spawn(move || {
            let how = line.read(&ask, &theirs);
            theirs.end(how);
        });
        stopper
    }

    fn read(&self, ask: &Ask, to: &Stopper) -> Result<(), Trouble> {
        let mut lines = BufReader::new(self.send(ask)?);
        let (mut line, mut data) = (Vec::new(), String::new());
        loop {
            line.clear();
            match lines.read_until(b'\n', &mut line) {
                Ok(0) => break,
                Ok(_) => {}
                // The line went quiet or broke in the middle of the answer.
                Err(_) => return Err(Trouble::Offline),
            }
            let text = String::from_utf8_lossy(&line);
            let text = text.trim_end_matches(['\r', '\n']);
            if text.is_empty() {
                // An empty line ends an event. A stream that was stopped is let go here.
                if !data.is_empty() && !to.data(std::mem::take(&mut data)) {
                    return Ok(());
                }
            } else if let Some(d) = text.strip_prefix("data:") {
                if !data.is_empty() {
                    data.push('\n');
                }
                data.push_str(d.strip_prefix(' ').unwrap_or(d));
            }
        }
        if !data.is_empty() {
            to.data(data);
        }
        Ok(())
    }
}

fn text(resp: reqwest::blocking::Response, most: u64) -> String {
    let mut bytes = Vec::new();
    let _ = resp.take(most).read_to_end(&mut bytes);
    String::from_utf8_lossy(&bytes).into_owned()
}

/// What a server said about a refusal, in one line, without the key even if the server repeated it.
fn said(body: &str, key: Option<&Secret>) -> String {
    let v: Value = serde_json::from_str(body).unwrap_or(Value::Null);
    let places = [&v["error"]["message"], &v["error"], &v["message"], &v["detail"], &v[0]["error"]["message"]];
    let found = places.into_iter().find_map(|m| m.as_str()).unwrap_or(body);
    let mut line = found.split_whitespace().collect::<Vec<_>>().join(" ");
    if let Some(k) = key.map(Secret::expose).filter(|k| !k.is_empty()) {
        line = line.replace(k, "[key]");
    }
    if line.chars().count() > SAID {
        line = line.chars().take(SAID).collect::<String>() + "...";
    }
    line
}

/// A wait as services write it: seconds as a number, or a span such as `6m0s`, `1h2m3s` or `20ms`.
fn span(s: &str) -> Option<f64> {
    let (mut total, mut number, mut any) = (0.0, String::new(), false);
    let mut chars = s.trim().chars().peekable();
    while let Some(c) = chars.next() {
        if c.is_ascii_digit() || c == '.' {
            number.push(c);
            continue;
        }
        let n: f64 = number.parse().ok()?;
        number.clear();
        let unit = match c {
            'h' => 3600.0,
            'm' if chars.peek() == Some(&'s') => {
                chars.next();
                0.001
            }
            'm' => 60.0,
            's' => 1.0,
            _ => return None,
        };
        total += n * unit;
        any = true;
    }
    if !number.is_empty() {
        total += number.parse::<f64>().ok()?;
        any = true;
    }
    any.then_some(total)
}

/// When a service that said "too many" takes a question again (Unix seconds), from whichever header names it.
pub fn resets(header: impl Fn(&str) -> Option<String>, now: i64) -> Option<i64> {
    if let Some(wait) = header("retry-after").and_then(|v| span(&v)) {
        return Some(now + wait.ceil() as i64);
    }
    if let Some(n) = header("x-ratelimit-reset").and_then(|v| v.trim().parse::<f64>().ok()) {
        // A moment in milliseconds or in seconds, or a wait in seconds.
        return Some(if n > 1e11 {
            (n / 1000.0) as i64
        } else if n > 1e9 {
            n as i64
        } else {
            now + n.ceil() as i64
        });
    }
    // The limit that was hit is not named, so the later of the two is when both are clear.
    let both = ["x-ratelimit-reset-requests", "x-ratelimit-reset-tokens"];
    both.iter().filter_map(|h| span(&header(h)?)).map(|wait| now + wait.ceil() as i64).max()
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use serde_json::json;
    use std::io::Write;
    use std::net::{TcpListener, TcpStream};
    use std::sync::mpsc::channel;
    use std::time::Instant;

    pub const KEY: &str = "test-key-not-real";

    /// One request as the stand-in server read it.
    #[derive(Debug, Clone, Default)]
    pub struct Asked {
        pub method: String,
        pub path: String,
        pub head: String,
        pub body: String,
    }

    pub enum Reply {
        /// A status, further header lines, and a body.
        Whole(u16, &'static str, String),
        /// The pieces of an event stream, each after a wait in milliseconds.
        Stream(Vec<(u64, String)>),
    }

    pub struct Stub {
        pub base: String,
        pub seen: Arc<Mutex<Vec<Asked>>>,
    }

    impl Stub {
        pub fn paths(&self) -> Vec<String> {
            self.seen.lock().unwrap().iter().map(|a| a.path.clone()).collect()
        }

        pub fn bodies(&self) -> Vec<Value> {
            self.seen.lock().unwrap().iter().map(|a| serde_json::from_str(&a.body).unwrap_or(Value::Null)).collect()
        }
    }

    fn request(s: &mut TcpStream) -> Option<Asked> {
        let (mut all, mut buf) = (Vec::new(), [0u8; 4096]);
        let split = loop {
            if let Some(i) = all.windows(4).position(|w| w == b"\r\n\r\n") {
                break i + 4;
            }
            let n = s.read(&mut buf).ok()?;
            if n == 0 {
                return None;
            }
            all.extend_from_slice(&buf[..n]);
        };
        let head = String::from_utf8_lossy(&all[..split]).into_owned();
        let length = |l: &str| l.to_lowercase().strip_prefix("content-length:").and_then(|v| v.trim().parse::<usize>().ok());
        let length = head.lines().find_map(length).unwrap_or(0);
        while all.len() < split + length {
            let n = s.read(&mut buf).ok()?;
            if n == 0 {
                break;
            }
            all.extend_from_slice(&buf[..n]);
        }
        let mut first = head.lines().next().unwrap_or("").split(' ');
        let (method, path) = (first.next()?.to_owned(), first.next()?.to_owned());
        Some(Asked { method, path, body: String::from_utf8_lossy(&all[split..]).into_owned(), head })
    }

    /// A server on a loopback port of this test's own: it answers each request as `answer` says and remembers
    /// what it was asked. Its base ends in `/v1`, as the real ones do.
    pub fn stub(answer: impl Fn(&Asked) -> Reply + Send + Sync + 'static) -> Stub {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}/v1", l.local_addr().unwrap());
        let seen: Arc<Mutex<Vec<Asked>>> = Arc::default();
        let (log, answer) = (seen.clone(), Arc::new(answer));
        std::thread::spawn(move || {
            for s in l.incoming() {
                let Ok(mut s) = s else { break };
                let (log, answer) = (log.clone(), answer.clone());
                std::thread::spawn(move || {
                    let Some(asked) = request(&mut s) else { return };
                    log.lock().unwrap().push(asked.clone());
                    match (*answer)(&asked) {
                        Reply::Whole(status, headers, body) => {
                            let length = body.len();
                            let _ = write!(
                                s,
                                "HTTP/1.1 {status} X\r\n{headers}Content-Type: application/json\r\nContent-Length: {length}\r\nConnection: close\r\n\r\n{body}"
                            );
                        }
                        Reply::Stream(pieces) => {
                            let _ = write!(s, "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n");
                            let _ = s.flush();
                            for (wait, piece) in pieces {
                                std::thread::sleep(Duration::from_millis(wait));
                                if s.write_all(piece.as_bytes()).and_then(|_| s.flush()).is_err() {
                                    return;
                                }
                            }
                        }
                    }
                });
            }
        });
        Stub { base, seen }
    }

    /// One event of a stream, as it is written on the wire.
    pub fn event(data: &str) -> String {
        format!("data: {data}\n\n")
    }

    /// `datas` as a stream of events a moment apart.
    pub fn events(datas: &[&str]) -> Reply {
        Reply::Stream(datas.iter().map(|d| (2, event(d))).collect())
    }

    /// An address on this computer where nobody listens.
    pub fn nobody() -> String {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        format!("http://{}/v1", l.local_addr().unwrap())
    }

    fn ask(base: &str, body: Option<Value>, within: Duration) -> Ask {
        Ask { url: format!("{base}/chat/completions"), key: Some(Arc::new(Secret::new(KEY))), body, within }
    }

    fn line() -> Line {
        Line::new(true).unwrap()
    }

    /// Everything a stream told its sink, up to its end.
    fn told(ask: Ask) -> (Vec<String>, Result<(), Trouble>) {
        let (tx, rx) = channel();
        line().stream(ask, Box::new(move |p| tx.send(p).is_ok()));
        let mut data = Vec::new();
        loop {
            match rx.recv_timeout(Duration::from_secs(20)).expect("the stream ends") {
                Piece::Data(d) => data.push(d),
                Piece::End(how) => return (data, how),
            }
        }
    }

    #[test]
    fn a_stream_is_read_event_by_event_however_it_is_cut_and_the_key_rides_only_in_the_header() {
        let pieces = [": a comment\n\n", "data: one\n\n", "data: tw", "o\r\n\r\ndata: three"];
        let s = stub(move |_| Reply::Stream(pieces.iter().map(|p| (2, p.to_string())).collect()));
        let body = json!({ "stream": true, "words": "hello" });
        let (data, how) = told(ask(&s.base, Some(body.clone()), QUIET));
        assert_eq!((data, how), (vec!["one".to_owned(), "two".into(), "three".into()], Ok(())));
        let seen = s.seen.lock().unwrap();
        assert_eq!((seen[0].method.as_str(), seen[0].path.as_str()), ("POST", "/v1/chat/completions"));
        assert!(seen[0].head.to_lowercase().contains(&format!("authorization: bearer {KEY}")), "{}", seen[0].head);
        assert_eq!(serde_json::from_str::<Value>(&seen[0].body).unwrap(), body);
        assert!(!seen[0].body.contains(KEY) && !seen[0].path.contains(KEY));
    }

    #[test]
    fn a_stop_ends_the_stream_at_once_and_what_comes_later_is_dropped() {
        let s = stub(|_| Reply::Stream(vec![(1, event("first")), (400, event("late")), (3_000, event("never"))]));
        let (tx, rx) = channel();
        let stopper = line().stream(ask(&s.base, Some(json!({})), QUIET), Box::new(move |p| tx.send(p).is_ok()));
        assert!(matches!(rx.recv_timeout(Duration::from_secs(10)), Ok(Piece::Data(d)) if d == "first"));
        let t = Instant::now();
        std::thread::spawn(move || stopper.stop()).join().unwrap();
        assert!(matches!(rx.recv_timeout(Duration::from_secs(5)), Ok(Piece::End(Err(Trouble::Stopped)))));
        assert!(t.elapsed() < Duration::from_millis(200), "stopped in {:?}", t.elapsed());
        assert!(rx.recv_timeout(Duration::from_millis(900)).is_err(), "nothing follows the end");
    }

    #[test]
    fn an_answer_that_goes_quiet_for_the_wait_is_offline_and_so_is_an_address_nobody_listens_at() {
        let s = stub(|_| Reply::Stream(vec![(1, event("first")), (3_000, event("late"))]));
        let (data, how) = told(ask(&s.base, Some(json!({})), Duration::from_millis(400)));
        assert_eq!((data, how), (vec!["first".to_owned()], Err(Trouble::Offline)));
        assert_eq!(line().whole(&ask(&nobody(), None, QUIET)).err(), Some(Trouble::Offline));
    }

    #[test]
    fn a_refusal_carries_its_status_its_reset_and_one_line_of_what_was_said_without_the_key() {
        // A server that repeats the whole request head, key included, in its error.
        let s = stub(|a| Reply::Whole(429, "Retry-After: 30\r\n", json!({ "error": { "message": format!("No more today.\n  {}", a.head) } }).to_string()));
        let now = chrono::Utc::now().timestamp();
        let Err(Trouble::Status { code, resets, said }) = line().whole(&ask(&s.base, None, QUIET)) else { panic!("a refusal") };
        assert_eq!(code, 429);
        assert!((now + 29..=now + 35).contains(&resets.unwrap()), "{resets:?}");
        assert!(said.starts_with("No more today. GET /v1/chat/completions"), "{said}");
        assert!(!said.contains(KEY) && said.contains("[key]") && !said.contains('\n'), "{said}");
        assert!(said.chars().count() <= SAID + 3);
    }

    #[test]
    fn the_reset_is_read_from_whichever_header_names_it() {
        let with = |pairs: &'static [(&'static str, &'static str)]| {
            resets(move |name| pairs.iter().find(|(n, _)| *n == name).map(|(_, v)| v.to_string()), 1_000)
        };
        assert_eq!(with(&[("retry-after", "30")]), Some(1_030));
        assert_eq!(with(&[("retry-after", "0.2"), ("x-ratelimit-reset-requests", "1h")]), Some(1_001));
        assert_eq!(with(&[("x-ratelimit-reset-requests", "6m0s"), ("x-ratelimit-reset-tokens", "20ms")]), Some(1_360));
        assert_eq!(with(&[("x-ratelimit-reset", "1700000000000")]), Some(1_700_000_000));
        assert_eq!(with(&[("x-ratelimit-reset", "1700000000")]), Some(1_700_000_000));
        assert_eq!(with(&[("x-ratelimit-reset", "45")]), Some(1_045));
        assert_eq!(with(&[("retry-after", "soon")]), None);
        assert_eq!(with(&[]), None);
        assert_eq!((span("1h2m3s"), span("1.5s"), span("x")), (Some(3723.0), Some(1.5), None));
    }
}
