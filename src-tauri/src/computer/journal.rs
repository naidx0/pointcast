//! Every computer tool call, kept in memory so the turn can be read back: when, in which app, which tool, on what,
//! and how it ended. The Ask turn takes the entries and writes them into its session.
use serde::Serialize;
use std::sync::{Arc, Mutex};

/// Past this many entries the oldest go, so a journal nobody takes can't grow without end.
pub const KEEP: usize = 2_000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Entry {
    /// Milliseconds since the Unix epoch.
    pub at: u64,
    pub app: String,
    pub tool: String,
    /// What it acted on: an element's role and name, a point, the keys or the app.
    pub target: String,
    pub outcome: Outcome,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "status", content = "detail", rename_all = "snake_case")]
pub enum Outcome {
    /// It happened; how (by accessibility action or synthetic input).
    Done(String),
    /// The guard or the grant said no.
    Refused(String),
    /// It waits for the person.
    NeedsConfirm(String),
    /// It was tried and the OS said no.
    Failed(String),
}

#[derive(Clone, Default)]
pub struct Journal(Arc<Mutex<Vec<Entry>>>);

impl Journal {
    pub fn push(&self, app: &str, tool: &str, target: &str, outcome: Outcome) {
        let at = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64);
        let mut v = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if v.len() >= KEEP {
            v.remove(0);
        }
        v.push(Entry { at, app: app.into(), tool: tool.into(), target: target.into(), outcome });
    }

    /// A copy of what is kept.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn entries(&self) -> Vec<Entry> {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// Everything kept, leaving it empty: what a turn writes into its session when it ends.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn take(&self) -> Vec<Entry> {
        std::mem::take(&mut *self.0.lock().unwrap_or_else(|e| e.into_inner()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entries_are_kept_in_order_taken_once_and_capped() {
        let j = Journal::default();
        j.push("TextEdit", "pointcast_click", "button \"Bold\"", Outcome::Done("press".into()));
        j.push("Mail", "pointcast_click", "button \"Send\"", Outcome::NeedsConfirm("asks first".into()));
        let got = j.entries();
        assert_eq!(got.len(), 2);
        assert_eq!((got[0].app.as_str(), got[1].tool.as_str()), ("TextEdit", "pointcast_click"));
        assert!(got[0].at > 1_700_000_000_000 && got[0].at <= got[1].at);
        let json = serde_json::to_value(&got[1]).unwrap();
        assert_eq!(json["outcome"], serde_json::json!({ "status": "needs_confirm", "detail": "asks first" }));
        assert_eq!(j.take().len(), 2);
        assert!(j.take().is_empty());
        for i in 0..KEEP + 5 {
            j.push("A", "t", &i.to_string(), Outcome::Done(String::new()));
        }
        let all = j.entries();
        assert_eq!((all.len(), all[0].target.as_str()), (KEEP, "5"));
    }
}
