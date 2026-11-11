//! Agent setup in one click: Pointcast writes its MCP server entry into a coding agent's own user settings (Claude Code,
//! Cursor, Codex) and takes it out again, touching nothing else in the file.
//!
//! - Claude Code: `~/.claude.json`, top-level `mcpServers.pointcast` = `{ "type": "http", "url", "headers" }` (what
//!   `claude mcp add --transport http --scope user` writes). The pictures folder goes in `~/.claude/settings.json`
//!   under `permissions.additionalDirectories`.
//! - Cursor: `~/.cursor/mcp.json`, `mcpServers.pointcast` = `{ "url", "headers" }`.
//! - Codex: `~/.codex/config.toml`, `[mcp_servers.pointcast]` with `url` and `http_headers` (streamable HTTP, no bridge).
//!
//! JSON files are edited as text: the one member is replaced, inserted or cut, so every other byte (key order,
//! indentation, line endings) stays as it was. TOML goes through `toml_edit`, which keeps comments and layout. Every
//! write goes to a temporary file that is then renamed over the old one, and the first write keeps a
//! `.pointcast-backup` copy. The home folder is always a parameter, so tests never see the real one.

use serde::Serialize;
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};

/// The server's name in every agent's settings.
pub const NAME: &str = "pointcast";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Agent {
    Claude,
    Cursor,
    Codex,
}

pub const ALL: [Agent; 3] = [Agent::Claude, Agent::Cursor, Agent::Codex];

impl Agent {
    pub fn parse(s: &str) -> Result<Agent, String> {
        match s {
            "claude" => Ok(Agent::Claude),
            "cursor" => Ok(Agent::Cursor),
            "codex" => Ok(Agent::Codex),
            _ => Err("Unknown agent.".into()),
        }
    }

    pub fn id(self) -> &'static str {
        match self {
            Agent::Claude => "claude",
            Agent::Cursor => "cursor",
            Agent::Codex => "codex",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Agent::Claude => "Claude Code",
            Agent::Cursor => "Cursor",
            Agent::Codex => "Codex",
        }
    }

    /// The agent's own folder in the home folder: there once the agent has run on this computer.
    fn folder(self, home: &Path) -> PathBuf {
        home.join(match self {
            Agent::Claude => ".claude",
            Agent::Cursor => ".cursor",
            Agent::Codex => ".codex",
        })
    }

    /// The user-scope file the server entry goes in.
    pub fn config(self, home: &Path) -> PathBuf {
        match self {
            Agent::Claude => home.join(".claude.json"),
            Agent::Cursor => home.join(".cursor").join("mcp.json"),
            Agent::Codex => home.join(".codex").join("config.toml"),
        }
    }

    /// The file as people know it, for messages.
    pub fn shown(self) -> &'static str {
        match self {
            Agent::Claude => "~/.claude.json",
            Agent::Cursor => "~/.cursor/mcp.json",
            Agent::Codex => "~/.codex/config.toml",
        }
    }

    /// Installed means the agent has made its folder (Claude Code may have only `~/.claude.json`). Pointcast never makes
    /// an agent's folder itself.
    pub fn installed(self, home: &Path) -> bool {
        self.folder(home).is_dir() || (self == Agent::Claude && self.config(home).is_file())
    }

    pub fn not_found(self) -> String {
        format!("{} was not found on this computer.", self.name())
    }
}

/// What an agent's settings hold for Pointcast.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Entry {
    /// No `pointcast` entry.
    Missing,
    /// This address and this computer's key.
    Current,
    /// A `pointcast` entry with another address or an old key.
    Other,
}

/// The entry each agent gets.
fn json_entry(agent: Agent, url: &str, token: &str) -> Value {
    let headers = json!({ "Authorization": format!("Bearer {token}") });
    match agent {
        Agent::Claude => json!({ "type": "http", "url": url, "headers": headers }),
        _ => json!({ "url": url, "headers": headers }),
    }
}

/// What "Copy command" puts on the clipboard: something to run or paste into the agent's user settings.
pub fn copy_text(agent: Agent, url: &str, token: &str) -> String {
    match agent {
        Agent::Claude => format!(
            "claude mcp add --transport http --scope user {NAME} {url} --header {q}Authorization: Bearer {token}{q}",
            q = '"'
        ),
        Agent::Cursor => format!(
            "{q}{NAME}{q}: {}",
            serde_json::to_string_pretty(&json_entry(agent, url, token)).unwrap_or_default(),
            q = '"'
        ),
        Agent::Codex => toml_set("", url, token).unwrap_or_default().trim_start().to_string(),
    }
}

fn read(path: &Path, shown: &str) -> Result<Option<String>, String> {
    match fs::read(path) {
        Ok(bytes) => String::from_utf8(bytes)
            .map(Some)
            .map_err(|_| format!("{shown} is not text, so Pointcast left it unchanged.")),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("Pointcast could not read {shown}: {}.", plain_io(&e))),
    }
}

fn plain_io(e: &std::io::Error) -> String {
    match e.kind() {
        std::io::ErrorKind::PermissionDenied => "permission denied".into(),
        _ => e.to_string(),
    }
}

fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut s = path.as_os_str().to_owned();
    s.push(suffix);
    PathBuf::from(s)
}

/// Replace `path` with `text` all at once (a temporary file renamed over it). The first change keeps the old file as
/// `<file>.pointcast-backup`. A read-only file is left alone.
fn write(path: &Path, shown: &str, text: &str) -> Result<(), String> {
    let old = fs::metadata(path).ok();
    if old.as_ref().is_some_and(|m| m.permissions().readonly()) {
        return Err(format!("{shown} is read-only, so Pointcast left it unchanged."));
    }
    let fail = |e: std::io::Error| format!("Pointcast could not write {shown}: {}. Nothing was changed.", plain_io(&e));
    if old.is_some() {
        let backup = with_suffix(path, ".pointcast-backup");
        if !backup.exists() {
            fs::copy(path, &backup).map_err(fail)?;
        }
    }
    let tmp = with_suffix(path, ".pointcast-tmp");
    let done = (|| {
        fs::write(&tmp, text.as_bytes())?;
        match &old {
            Some(m) => fs::set_permissions(&tmp, m.permissions())?,
            // A new file holds this computer's key: only this user reads it.
            None => {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    fs::set_permissions(&tmp, fs::Permissions::from_mode(0o600))?;
                }
            }
        }
        fs::rename(&tmp, path)
    })();
    if let Err(e) = done {
        let _ = fs::remove_file(&tmp);
        return Err(fail(e));
    }
    Ok(())
}

/// Write Pointcast's entry into the agent's user settings, replacing an older one. Everything else stays.
pub fn set_up(agent: Agent, home: &Path, url: &str, token: &str) -> Result<(), String> {
    if !agent.installed(home) {
        return Err(agent.not_found());
    }
    let path = agent.config(home);
    let shown = agent.shown();
    let old = read(&path, shown)?.unwrap_or_default();
    let new = match agent {
        Agent::Codex => toml_set(&old, url, token),
        _ => json_set(&old, "mcpServers", NAME, &json_entry(agent, url, token)),
    }
    .map_err(|e| format!("{shown} {e}, so Pointcast left it unchanged."))?;
    if new == old {
        return Ok(());
    }
    write(&path, shown, &new)
}

/// Take Pointcast's entry out of the agent's settings, and only that. False when there was none.
pub fn remove(agent: Agent, home: &Path) -> Result<bool, String> {
    let path = agent.config(home);
    let shown = agent.shown();
    let Some(old) = read(&path, shown)? else {
        return Ok(false);
    };
    let new = match agent {
        Agent::Codex => toml_remove(&old),
        _ => json_remove(&old, "mcpServers", NAME),
    }
    .map_err(|e| format!("{shown} {e}, so Pointcast left it unchanged."))?;
    match new {
        Some(new) => write(&path, shown, &new).map(|_| true),
        None => Ok(false),
    }
}

/// What the agent's settings hold for Pointcast now.
pub fn entry(agent: Agent, home: &Path, url: &str, token: &str) -> Result<Entry, String> {
    let shown = agent.shown();
    let Some(text) = read(&agent.config(home), shown)? else {
        return Ok(Entry::Missing);
    };
    let auth = format!("Bearer {token}");
    let (got_url, got_auth, typed) = match agent {
        Agent::Codex => {
            let doc: toml_edit::DocumentMut = text.parse().map_err(|_| format!("{shown} is not valid TOML."))?;
            let Some(e) = doc.get("mcp_servers").and_then(|s| s.get(NAME)) else {
                return Ok(Entry::Missing);
            };
            let header = e.get("http_headers").and_then(|h| h.get("Authorization")).and_then(|v| v.as_str());
            (e.get("url").and_then(|v| v.as_str()).map(String::from), header.map(String::from), true)
        }
        _ => {
            if text.trim().is_empty() {
                return Ok(Entry::Missing);
            }
            let v: Value = serde_json::from_str(&text).map_err(|_| format!("{shown} is not plain JSON."))?;
            let Some(e) = v.get("mcpServers").and_then(|s| s.get(NAME)) else {
                return Ok(Entry::Missing);
            };
            let typed = agent != Agent::Claude || matches!(e["type"].as_str(), Some("http" | "streamable-http"));
            (e["url"].as_str().map(String::from), e["headers"]["Authorization"].as_str().map(String::from), typed)
        }
    };
    Ok(if got_url.as_deref() == Some(url) && got_auth.as_deref() == Some(auth.as_str()) && typed {
        Entry::Current
    } else {
        Entry::Other
    })
}

/// One agent as Settings shows it. Never holds the key.
#[derive(Debug, Serialize)]
pub struct View {
    pub id: &'static str,
    pub name: &'static str,
    pub path: &'static str,
    pub found: bool,
    pub entry: Entry,
    pub error: Option<String>,
}

pub fn view(agent: Agent, home: &Path, url: &str, token: &str) -> View {
    let found = agent.installed(home);
    let (entry, error) = match entry(agent, home, url, token) {
        Ok(e) => (e, None),
        Err(e) => (Entry::Missing, Some(e)),
    };
    View { id: agent.id(), name: agent.name(), path: agent.shown(), found, entry, error }
}

// Claude Code's pictures folder: `permissions.additionalDirectories` in `~/.claude/settings.json`.

const CLAUDE_SETTINGS: &str = "~/.claude/settings.json";

fn claude_settings(home: &Path) -> PathBuf {
    home.join(".claude").join("settings.json")
}

fn same_dir(a: &str, b: &str) -> bool {
    let norm = |s: &str| {
        let s = s.replace(std::path::MAIN_SEPARATOR, "/");
        let s = s.trim_end_matches('/').to_string();
        if cfg!(windows) {
            s.to_lowercase()
        } else {
            s
        }
    };
    norm(a) == norm(b)
}

/// Whether Claude Code may read `dir` without asking. False when its settings cannot be read.
pub fn folder_allowed(home: &Path, dir: &str) -> bool {
    let Ok(Some(text)) = read(&claude_settings(home), CLAUDE_SETTINGS) else {
        return false;
    };
    let Ok(v) = serde_json::from_str::<Value>(&text) else {
        return false;
    };
    v.pointer("/permissions/additionalDirectories")
        .and_then(Value::as_array)
        .is_some_and(|a| a.iter().filter_map(Value::as_str).any(|d| same_dir(d, dir)))
}

/// Add `dir` to (or take it out of) Claude Code's additional directories. Needs `~/.claude`, which is never made here.
pub fn allow_folder(home: &Path, dir: &str, on: bool) -> Result<(), String> {
    if !home.join(".claude").is_dir() {
        return Err(Agent::Claude.not_found());
    }
    let path = claude_settings(home);
    let old = read(&path, CLAUDE_SETTINGS)?.unwrap_or_default();
    match json_allow(&old, dir, on).map_err(|e| format!("{CLAUDE_SETTINGS} {e}, so Pointcast left it unchanged."))? {
        Some(new) => write(&path, CLAUDE_SETTINGS, &new),
        None => Ok(()),
    }
}

// JSON as text: members are found by a small scanner over a file serde_json has already accepted.

mod jtext {
    pub struct Member {
        pub key: String,
        /// Where the key's opening quote is.
        pub start: usize,
        /// The value, end exclusive.
        pub value: (usize, usize),
    }

    pub struct Obj {
        pub open: usize,
        pub close: usize,
        pub members: Vec<Member>,
    }

    struct P<'a> {
        b: &'a [u8],
        i: usize,
    }

    impl P<'_> {
        fn at(&self) -> Option<u8> {
            self.b.get(self.i).copied()
        }

        fn ws(&mut self) {
            while matches!(self.at(), Some(b' ' | b'\t' | b'\r' | b'\n')) {
                self.i += 1;
            }
        }

        fn string(&mut self) -> Option<String> {
            let start = self.i;
            if self.at()? != b'"' {
                return None;
            }
            self.i += 1;
            loop {
                match self.at()? {
                    b'\\' => self.i += 2,
                    b'"' => {
                        self.i += 1;
                        break;
                    }
                    _ => self.i += 1,
                }
            }
            serde_json::from_slice(&self.b[start..self.i]).ok()
        }

        fn value(&mut self) -> Option<(usize, usize)> {
            self.ws();
            let start = self.i;
            match self.at()? {
                b'{' => {
                    self.object()?;
                }
                b'[' => {
                    self.i += 1;
                    self.ws();
                    if self.at()? == b']' {
                        self.i += 1;
                    } else {
                        loop {
                            self.value()?;
                            self.ws();
                            match self.at()? {
                                b',' => self.i += 1,
                                b']' => {
                                    self.i += 1;
                                    break;
                                }
                                _ => return None,
                            }
                        }
                    }
                }
                b'"' => {
                    self.string()?;
                }
                _ => {
                    while !matches!(self.at(), None | Some(b',' | b'}' | b']' | b' ' | b'\t' | b'\r' | b'\n')) {
                        self.i += 1;
                    }
                    if self.i == start {
                        return None;
                    }
                }
            }
            Some((start, self.i))
        }

        fn object(&mut self) -> Option<Obj> {
            let open = self.i;
            if self.at()? != b'{' {
                return None;
            }
            self.i += 1;
            let mut members = Vec::new();
            self.ws();
            if self.at()? == b'}' {
                self.i += 1;
                return Some(Obj { open, close: self.i - 1, members });
            }
            loop {
                self.ws();
                let start = self.i;
                let key = self.string()?;
                self.ws();
                if self.at()? != b':' {
                    return None;
                }
                self.i += 1;
                let value = self.value()?;
                members.push(Member { key, start, value });
                self.ws();
                match self.at()? {
                    b',' => self.i += 1,
                    b'}' => {
                        self.i += 1;
                        return Some(Obj { open, close: self.i - 1, members });
                    }
                    _ => return None,
                }
            }
        }
    }

    pub fn object_at(text: &str, at: usize) -> Option<Obj> {
        let mut p = P { b: text.as_bytes(), i: at };
        p.ws();
        p.object()
    }
}

/// Parse as strict JSON first: comments or trailing commas are refused rather than guessed around.
fn root(text: &str) -> Result<jtext::Obj, String> {
    let v: Value = serde_json::from_str(text).map_err(|_| "is not plain JSON (it may have comments)".to_string())?;
    if !v.is_object() {
        return Err("is not a JSON object".into());
    }
    jtext::object_at(text, 0).ok_or_else(|| "could not be read".into())
}

fn newline(text: &str) -> &'static str {
    if text.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    }
}

/// The file's indentation step: the leading whitespace of its first indented line (two spaces when none).
fn unit(text: &str) -> String {
    text.lines()
        .skip(1)
        .map(|l| &l[..l.len() - l.trim_start_matches([' ', '\t']).len()])
        .find(|w| !w.is_empty())
        .unwrap_or("  ")
        .to_string()
}

/// The leading whitespace of the line holding `pos`.
fn line_indent(text: &str, pos: usize) -> String {
    let start = text[..pos].rfind('\n').map_or(0, |n| n + 1);
    let line = &text[start..];
    line[..line.len() - line.trim_start_matches([' ', '\t']).len()].to_string()
}

/// `v` laid out the way the file is: compact when the file is on one line, else pretty with the file's step, each new
/// line starting at `base`.
fn render(text: &str, v: &Value, base: &str) -> String {
    if !text.contains('\n') {
        return v.to_string();
    }
    let step = unit(text);
    let mut buf = Vec::new();
    let mut ser = serde_json::Serializer::with_formatter(&mut buf, serde_json::ser::PrettyFormatter::with_indent(step.as_bytes()));
    let _ = serde::Serialize::serialize(v, &mut ser);
    String::from_utf8(buf).unwrap_or_default().replace('\n', &format!("{}{base}", newline(text)))
}

fn quoted(key: &str) -> String {
    serde_json::to_string(key).unwrap_or_default()
}

/// Replace `key`'s value in `obj`, or add the member at the end.
fn set_member(text: &str, obj: &jtext::Obj, key: &str, v: &Value) -> String {
    let compact = !text.contains('\n');
    let nl = newline(text);
    if let Some(m) = obj.members.iter().rev().find(|m| m.key == key) {
        let base = line_indent(text, m.start);
        return format!("{}{}{}", &text[..m.value.0], render(text, v, &base), &text[m.value.1..]);
    }
    let (at, insert) = match obj.members.last() {
        Some(last) => {
            let base = line_indent(text, last.start);
            let body = format!("{}: {}", quoted(key), render(text, v, &base));
            let insert = if compact { format!(",{}:{}", quoted(key), v) } else { format!(",{nl}{base}{body}") };
            (last.value.1, insert)
        }
        None => {
            let parent = line_indent(text, obj.open);
            let base = format!("{parent}{}", unit(text));
            let insert = if compact {
                format!("{}:{}", quoted(key), v)
            } else {
                format!("{nl}{base}{}: {}{nl}{parent}", quoted(key), render(text, v, &base))
            };
            // Whatever whitespace sat between the braces goes.
            return format!("{}{insert}{}", &text[..=obj.open], &text[obj.close..]);
        }
    };
    format!("{}{insert}{}", &text[..at], &text[at..])
}

/// Cut `key` out of `obj` with its comma. None when it is not there.
fn remove_member(text: &str, obj: &jtext::Obj, key: &str) -> Option<String> {
    let i = obj.members.iter().rposition(|m| m.key == key)?;
    let m = &obj.members[i];
    let (from, to) = if i > 0 {
        (obj.members[i - 1].value.1, m.value.1)
    } else if let Some(next) = obj.members.get(1) {
        (m.start, next.start)
    } else {
        (obj.open + 1, obj.close)
    };
    Some(format!("{}{}", &text[..from], &text[to..]))
}

fn child(text: &str, obj: &jtext::Obj, key: &str) -> Result<Option<jtext::Obj>, String> {
    let Some(m) = obj.members.iter().rev().find(|m| m.key == key) else {
        return Ok(None);
    };
    let o = jtext::object_at(text, m.value.0).ok_or_else(|| format!("has a {key} that is not an object"))?;
    Ok(Some(o))
}

fn fresh(v: Value) -> String {
    format!("{}\n", serde_json::to_string_pretty(&v).unwrap_or_default())
}

/// `outer.key = v` in a JSON file's text.
fn json_set(text: &str, outer: &str, key: &str, v: &Value) -> Result<String, String> {
    if text.trim().is_empty() {
        return Ok(fresh(json!({ outer: { key: v } })));
    }
    let root = root(text)?;
    Ok(match child(text, &root, outer)? {
        Some(o) => set_member(text, &o, key, v),
        None => set_member(text, &root, outer, &json!({ key: v })),
    })
}

/// Cut `outer.key` from a JSON file's text. None when it is not there.
fn json_remove(text: &str, outer: &str, key: &str) -> Result<Option<String>, String> {
    if text.trim().is_empty() {
        return Ok(None);
    }
    let root = root(text)?;
    Ok(match child(text, &root, outer)? {
        Some(o) => remove_member(text, &o, key),
        None => None,
    })
}

/// `permissions.additionalDirectories` with `dir` in it (on) or out of it (off). None when nothing changes.
fn json_allow(text: &str, dir: &str, on: bool) -> Result<Option<String>, String> {
    let v: Value = if text.trim().is_empty() {
        json!({})
    } else {
        serde_json::from_str(text).map_err(|_| "is not plain JSON (it may have comments)".to_string())?
    };
    let list: Vec<Value> = match v.pointer("/permissions/additionalDirectories") {
        None => Vec::new(),
        Some(Value::Array(a)) => a.clone(),
        Some(_) => return Err("has an additionalDirectories that is not a list".into()),
    };
    let has = list.iter().filter_map(Value::as_str).any(|d| same_dir(d, dir));
    if has == on {
        return Ok(None);
    }
    let list: Vec<Value> = if on {
        list.into_iter().chain([Value::String(dir.into())]).collect()
    } else {
        list.into_iter().filter(|d| !d.as_str().is_some_and(|d| same_dir(d, dir))).collect()
    };
    if text.trim().is_empty() {
        return Ok(Some(fresh(json!({ "permissions": { "additionalDirectories": list } }))));
    }
    let root = root(text)?;
    Ok(Some(match child(text, &root, "permissions")? {
        Some(o) => set_member(text, &o, "additionalDirectories", &Value::Array(list)),
        None => set_member(text, &root, "permissions", &json!({ "additionalDirectories": list })),
    }))
}

// Codex's TOML, through toml_edit so comments and layout stay.

fn toml_set(text: &str, url: &str, token: &str) -> Result<String, String> {
    use toml_edit::{value, DocumentMut, InlineTable, Item, Table};
    let mut doc: DocumentMut = text.parse().map_err(|_| "is not valid TOML".to_string())?;
    if !doc.contains_key("mcp_servers") {
        let mut t = Table::new();
        t.set_implicit(true);
        doc.insert("mcp_servers", Item::Table(t));
    }
    let Some(servers) = doc["mcp_servers"].as_table_mut() else {
        return Err("has an mcp_servers that is not a table".into());
    };
    let mut headers = InlineTable::new();
    headers.insert("Authorization", format!("Bearer {token}").into());
    match servers.get_mut(NAME).and_then(Item::as_table_mut) {
        // Replaced in place, so the table keeps its spot and the comment above it.
        Some(t) => {
            t.clear();
            t.insert("url", value(url));
            t.insert("http_headers", value(headers));
        }
        None => {
            let mut t = Table::new();
            t.insert("url", value(url));
            t.insert("http_headers", value(headers));
            servers.insert(NAME, Item::Table(t));
        }
    }
    Ok(doc.to_string())
}

fn toml_remove(text: &str) -> Result<Option<String>, String> {
    let mut doc: toml_edit::DocumentMut = text.parse().map_err(|_| "is not valid TOML".to_string())?;
    let Some(servers) = doc.get_mut("mcp_servers").and_then(|s| s.as_table_like_mut()) else {
        return Ok(None);
    };
    if servers.remove(NAME).is_none() {
        return Ok(None);
    }
    Ok(Some(doc.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const URL: &str = "http://127.0.0.1:47821/mcp";
    const TOKEN: &str = "test-key-0000000000000000000000000000000000000000000000000000abcd";

    /// A home folder of its own for each test; never the real one.
    struct Home(PathBuf);

    impl Home {
        fn new(name: &str) -> Home {
            let p = std::env::temp_dir().join(format!("pc-agents-{name}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&p);
            fs::create_dir_all(&p).unwrap();
            Home(p)
        }
        fn with(self, dir: &str) -> Home {
            fs::create_dir_all(self.0.join(dir)).unwrap();
            self
        }
        fn put(&self, rel: &str, text: &str) {
            fs::write(self.0.join(rel), text).unwrap();
        }
        fn get(&self, rel: &str) -> String {
            fs::read_to_string(self.0.join(rel)).unwrap()
        }
    }

    impl Drop for Home {
        fn drop(&mut self) {
            // A read-only file would stop the clean-up.
            for rel in [".claude.json", ".cursor/mcp.json", ".codex/config.toml", ".claude/settings.json"] {
                let p = self.0.join(rel);
                if let Ok(m) = fs::metadata(&p) {
                    let mut perm = m.permissions();
                    #[allow(clippy::permissions_set_readonly_false)]
                    perm.set_readonly(false);
                    let _ = fs::set_permissions(&p, perm);
                }
            }
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    const CLAUDE_JSON: &str = "{\n  \"numStartups\": 12,\n  \"mcpServers\": {\n    \"github\": {\n      \"type\": \"http\",\n      \"url\": \"https://api.github.com/mcp\"\n    }\n  },\n  \"projects\": {\n    \"/x\": { \"mcpServers\": {} }\n  },\n  \"zeta\": [1, 2, {\"a\": \"}\"}]\n}\n";

    #[test]
    fn claude_code_gets_its_entry_and_every_other_byte_stays() {
        let h = Home::new("claude-keep").with(".claude");
        h.put(".claude.json", CLAUDE_JSON);
        set_up(Agent::Claude, &h.0, URL, TOKEN).unwrap();
        let after = h.get(".claude.json");
        let v: Value = serde_json::from_str(&after).unwrap();
        assert_eq!(v["mcpServers"]["pointcast"], json!({"type": "http", "url": URL, "headers": {"Authorization": format!("Bearer {TOKEN}")}}));
        assert_eq!(v["mcpServers"]["github"]["url"], "https://api.github.com/mcp");
        assert!(v["projects"]["/x"]["mcpServers"].get("pointcast").is_none(), "only the user scope");
        // Cut the new member out again and the file is the old file to the byte.
        let o = jtext::object_at(&after, after.find("\"mcpServers\": {").unwrap() + 14).unwrap();
        assert_eq!(remove_member(&after, &o, NAME).unwrap(), CLAUDE_JSON);
        assert!(after.contains("\n    },\n    \"pointcast\": {\n      \""), "indented like its neighbours: {after}");
        assert_eq!(entry(Agent::Claude, &h.0, URL, TOKEN).unwrap(), Entry::Current);
        assert_eq!(entry(Agent::Claude, &h.0, URL, "another-key").unwrap(), Entry::Other);
    }

    #[test]
    fn removing_takes_out_only_pointcast_and_gives_back_the_file() {
        let h = Home::new("claude-remove").with(".claude");
        h.put(".claude.json", CLAUDE_JSON);
        set_up(Agent::Claude, &h.0, URL, TOKEN).unwrap();
        assert!(remove(Agent::Claude, &h.0).unwrap());
        assert_eq!(h.get(".claude.json"), CLAUDE_JSON);
        assert!(!remove(Agent::Claude, &h.0).unwrap(), "nothing left to remove");
        assert_eq!(entry(Agent::Claude, &h.0, URL, TOKEN).unwrap(), Entry::Missing);
    }

    #[test]
    fn an_old_pointcast_entry_is_replaced_in_place() {
        let h = Home::new("cursor-replace").with(".cursor");
        let old = "{\r\n\t\"mcpServers\": {\r\n\t\t\"pointcast\": {\r\n\t\t\t\"url\": \"http://127.0.0.1:40000/mcp\",\r\n\t\t\t\"headers\": { \"Authorization\": \"Bearer old\" }\r\n\t\t},\r\n\t\t\"other\": { \"command\": \"x\" }\r\n\t}\r\n}";
        h.put(".cursor/mcp.json", old);
        assert_eq!(entry(Agent::Cursor, &h.0, URL, TOKEN).unwrap(), Entry::Other);
        set_up(Agent::Cursor, &h.0, URL, TOKEN).unwrap();
        let after = h.get(".cursor/mcp.json");
        assert_eq!(entry(Agent::Cursor, &h.0, URL, TOKEN).unwrap(), Entry::Current);
        assert!(!after.contains("40000") && !after.contains("Bearer old"), "{after}");
        assert!(after.find("pointcast").unwrap() < after.find("other").unwrap(), "kept its place: {after}");
        assert!(after.contains("\r\n\t\t\t\"url\""), "tabs and CRLF kept: {after:?}");
        assert!(!after.replace("\r\n", "").contains('\n'), "no bare LF: {after:?}");
        assert!(after.ends_with("}\r\n}"), "{after:?}");
        let v: Value = serde_json::from_str(&after).unwrap();
        assert_eq!(v["mcpServers"]["other"]["command"], "x");
        assert!(v["mcpServers"]["pointcast"].get("type").is_none(), "Cursor's remote entries have no type");
    }

    #[test]
    fn a_missing_file_is_made_when_the_agents_folder_is_there() {
        let h = Home::new("cursor-new").with(".cursor");
        set_up(Agent::Cursor, &h.0, URL, TOKEN).unwrap();
        let v: Value = serde_json::from_str(&h.get(".cursor/mcp.json")).unwrap();
        assert_eq!(v, json!({"mcpServers": {"pointcast": {"url": URL, "headers": {"Authorization": format!("Bearer {TOKEN}")}}}}));
        assert!(!h.0.join(".cursor/mcp.json.pointcast-backup").exists(), "nothing to back up");
        let h = Home::new("codex-new").with(".codex");
        set_up(Agent::Codex, &h.0, URL, TOKEN).unwrap();
        assert_eq!(
            h.get(".codex/config.toml"),
            format!("[mcp_servers.pointcast]\nurl = \"{URL}\"\nhttp_headers = {{ Authorization = \"Bearer {TOKEN}\" }}\n")
        );
        assert_eq!(entry(Agent::Codex, &h.0, URL, TOKEN).unwrap(), Entry::Current);
    }

    #[test]
    fn an_agent_that_is_not_installed_is_not_set_up_and_its_folder_is_not_made() {
        let h = Home::new("absent");
        for a in ALL {
            assert!(!a.installed(&h.0));
            assert_eq!(set_up(a, &h.0, URL, TOKEN).unwrap_err(), format!("{} was not found on this computer.", a.name()));
            assert!(!view(a, &h.0, URL, TOKEN).found);
        }
        assert_eq!(fs::read_dir(&h.0).unwrap().count(), 0, "nothing was made");
        assert!(allow_folder(&h.0, "/home/x/Pointcast", true).is_err());
        assert_eq!(fs::read_dir(&h.0).unwrap().count(), 0);
        // Claude Code with only its ~/.claude.json counts as installed.
        h.put(".claude.json", "{}");
        assert!(Agent::Claude.installed(&h.0));
    }

    const CODEX_TOML: &str = "# my Codex settings\nmodel = \"gpt-5\"\n\n[mcp_servers.docs]\n# the docs server\ncommand = \"npx\"\nargs = [\"-y\", \"docs-mcp\"]  # pinned\n\n[profiles.fast]\nmodel = \"small\"\n";

    #[test]
    fn codex_keeps_comments_and_other_servers_and_removal_gives_back_the_file() {
        let h = Home::new("codex-keep").with(".codex");
        h.put(".codex/config.toml", CODEX_TOML);
        set_up(Agent::Codex, &h.0, URL, TOKEN).unwrap();
        let after = h.get(".codex/config.toml");
        let docs = "# my Codex settings\nmodel = \"gpt-5\"\n\n[mcp_servers.docs]\n# the docs server\ncommand = \"npx\"\nargs = [\"-y\", \"docs-mcp\"]  # pinned\n";
        assert!(after.starts_with(docs), "everything before stays: {after}");
        assert!(after.ends_with("[profiles.fast]\nmodel = \"small\"\n"), "{after}");
        assert!(after.contains(&format!("[mcp_servers.pointcast]\nurl = \"{URL}\"\nhttp_headers = {{ Authorization = \"Bearer {TOKEN}\" }}")), "{after}");
        let doc: toml_edit::DocumentMut = after.parse().unwrap();
        assert_eq!(doc["mcp_servers"]["docs"]["command"].as_str(), Some("npx"));
        assert_eq!(doc["profiles"]["fast"]["model"].as_str(), Some("small"));
        assert!(remove(Agent::Codex, &h.0).unwrap());
        assert_eq!(h.get(".codex/config.toml").trim_end(), CODEX_TOML.trim_end());
    }

    #[test]
    fn codex_replaces_an_old_entry_where_it_stood() {
        let h = Home::new("codex-replace").with(".codex");
        let old = "[mcp_servers.pointcast]\n# old bridge\ncommand = \"npx\"\nargs = [\"mcp-remote\", \"http://127.0.0.1:40000/mcp\"]\n\n[mcp_servers.after]\ncommand = \"y\"\n";
        h.put(".codex/config.toml", old);
        assert_eq!(entry(Agent::Codex, &h.0, URL, TOKEN).unwrap(), Entry::Other);
        set_up(Agent::Codex, &h.0, URL, TOKEN).unwrap();
        let after = h.get(".codex/config.toml");
        assert!(!after.contains("mcp-remote") && !after.contains("40000"), "{after}");
        assert!(after.find("[mcp_servers.pointcast]").unwrap() < after.find("[mcp_servers.after]").unwrap(), "{after}");
        assert_eq!(entry(Agent::Codex, &h.0, URL, TOKEN).unwrap(), Entry::Current);
    }

    #[test]
    fn a_read_only_file_gives_a_plain_error_and_changes_nothing() {
        let h = Home::new("readonly").with(".claude");
        h.put(".claude.json", CLAUDE_JSON);
        let p = h.0.join(".claude.json");
        let mut perm = fs::metadata(&p).unwrap().permissions();
        perm.set_readonly(true);
        fs::set_permissions(&p, perm).unwrap();
        let e = set_up(Agent::Claude, &h.0, URL, TOKEN).unwrap_err();
        assert_eq!(e, "~/.claude.json is read-only, so Pointcast left it unchanged.");
        assert!(!e.contains(TOKEN));
        assert_eq!(h.get(".claude.json"), CLAUDE_JSON);
        let names: Vec<_> = fs::read_dir(&h.0).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(names.len(), 2, "no backup or temporary file: {names:?}");
    }

    #[test]
    fn json_with_comments_is_refused_not_guessed_around() {
        let h = Home::new("comments").with(".cursor");
        let text = "{\n  // mine\n  \"mcpServers\": {}\n}\n";
        h.put(".cursor/mcp.json", text);
        let e = set_up(Agent::Cursor, &h.0, URL, TOKEN).unwrap_err();
        assert!(e.starts_with("~/.cursor/mcp.json is not plain JSON") && e.ends_with("left it unchanged."), "{e}");
        assert_eq!(h.get(".cursor/mcp.json"), text);
    }

    #[test]
    fn the_backup_is_the_file_before_the_first_change_and_is_written_once() {
        let h = Home::new("backup").with(".claude");
        h.put(".claude.json", CLAUDE_JSON);
        set_up(Agent::Claude, &h.0, URL, TOKEN).unwrap();
        let backup = ".claude.json.pointcast-backup";
        assert_eq!(h.get(backup), CLAUDE_JSON);
        set_up(Agent::Claude, &h.0, URL, "a-newer-key-000000000000").unwrap();
        remove(Agent::Claude, &h.0).unwrap();
        assert_eq!(h.get(backup), CLAUDE_JSON, "later changes leave the first backup alone");
        assert!(!h.0.join(".claude.json.pointcast-tmp").exists());
    }

    #[test]
    fn empty_and_one_line_json_files_take_the_entry() {
        let h = Home::new("shapes").with(".cursor");
        h.put(".cursor/mcp.json", "");
        set_up(Agent::Cursor, &h.0, URL, TOKEN).unwrap();
        assert_eq!(entry(Agent::Cursor, &h.0, URL, TOKEN).unwrap(), Entry::Current);
        h.put(".cursor/mcp.json", r#"{"mcpServers":{"a":{"url":"x"}},"z":1}"#);
        set_up(Agent::Cursor, &h.0, URL, TOKEN).unwrap();
        let after = h.get(".cursor/mcp.json");
        assert!(!after.contains('\n'), "stays on one line: {after}");
        assert!(after.ends_with(r#"/mcp"}},"z":1}"#), "{after}");
        h.put(".cursor/mcp.json", "{\n  \"mcpServers\": {}\n}\n");
        set_up(Agent::Cursor, &h.0, URL, TOKEN).unwrap();
        assert!(h.get(".cursor/mcp.json").starts_with("{\n  \"mcpServers\": {\n    \"pointcast\": {\n      \"url\""), "{}", h.get(".cursor/mcp.json"));
        h.put(".cursor/mcp.json", "{\n  \"theme\": 1\n}\n");
        set_up(Agent::Cursor, &h.0, URL, TOKEN).unwrap();
        assert_eq!(entry(Agent::Cursor, &h.0, URL, TOKEN).unwrap(), Entry::Current);
        assert!(h.get(".cursor/mcp.json").starts_with("{\n  \"theme\": 1,\n  \"mcpServers\": {\n    \"pointcast\": {"));
        // Setting up twice changes nothing the second time.
        let once = h.get(".cursor/mcp.json");
        set_up(Agent::Cursor, &h.0, URL, TOKEN).unwrap();
        assert_eq!(h.get(".cursor/mcp.json"), once);
    }

    #[test]
    fn claude_code_may_open_the_pictures_folder_and_the_rest_of_its_settings_stay() {
        let h = Home::new("folder").with(".claude");
        let dir = "C:/Users/me/Pointcast";
        assert!(!folder_allowed(&h.0, dir));
        allow_folder(&h.0, dir, true).unwrap();
        assert!(folder_allowed(&h.0, dir));
        let v: Value = serde_json::from_str(&h.get(".claude/settings.json")).unwrap();
        assert_eq!(v, json!({"permissions": {"additionalDirectories": [dir]}}));
        let mine = "{\n  \"model\": \"opus\",\n  \"permissions\": {\n    \"allow\": [\"Bash(ls)\"],\n    \"additionalDirectories\": [\"/src\"]\n  }\n}\n";
        h.put(".claude/settings.json", mine);
        allow_folder(&h.0, dir, true).unwrap();
        let v: Value = serde_json::from_str(&h.get(".claude/settings.json")).unwrap();
        assert_eq!(v["permissions"]["additionalDirectories"], json!(["/src", dir]));
        assert_eq!(v["permissions"]["allow"], json!(["Bash(ls)"]));
        allow_folder(&h.0, &format!("{dir}/"), true).unwrap();
        let v: Value = serde_json::from_str(&h.get(".claude/settings.json")).unwrap();
        assert_eq!(v["permissions"]["additionalDirectories"].as_array().unwrap().len(), 2, "no second copy");
        allow_folder(&h.0, dir, false).unwrap();
        let v: Value = serde_json::from_str(&h.get(".claude/settings.json")).unwrap();
        assert_eq!(v["permissions"]["additionalDirectories"], json!(["/src"]));
        assert_eq!(v["model"], "opus");
    }

    #[test]
    fn the_copy_text_names_each_agents_own_format() {
        assert_eq!(
            copy_text(Agent::Claude, URL, TOKEN),
            format!("claude mcp add --transport http --scope user pointcast {URL} --header \"Authorization: Bearer {TOKEN}\"")
        );
        let cursor = copy_text(Agent::Cursor, URL, TOKEN);
        assert!(cursor.starts_with("\"pointcast\": {") && cursor.contains(URL) && !cursor.contains("mcpServers"), "{cursor}");
        let codex = copy_text(Agent::Codex, URL, TOKEN);
        assert!(codex.starts_with("[mcp_servers.pointcast]\nurl = ") && codex.contains("http_headers"), "{codex}");
        assert!(Agent::parse("vim").is_err());
    }
}
