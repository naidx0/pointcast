//! The person's say before the AI acts where it may not on its own: a card in the Ask panel while the tool call waits.
//!
//! Two kinds of card. An action that reads as irreversible (the guard's `NeedsConfirm`) asks for a yes: **Do it**
//! makes a one-time approval bound to exactly that action (its app, tool, target and arguments), good for one minute,
//! and the waiting call runs once more with it. An action the app's tier does not allow asks to raise it: **Allow in
//! <app>** raises the tier to what the action needs (click or full), which the caller keeps in the settings, and the
//! call runs again under the new tier, so an irreversible target still asks for its yes. **Cancel** tells the AI the
//! person declined. Approvals are made here only, by `answer`, which only the panel's command calls: no MCP request
//! reaches it, and `Approval` has no other constructor and cannot be copied.
use super::guard::Tier;
use serde::Serialize;
use serde_json::Value;
use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};

/// How long a card waits for the person, and how long a yes stays good.
pub const WITHIN: Duration = Duration::from_secs(60);

/// What the panel shows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Card {
    pub id: u64,
    pub app: String,
    pub tool: String,
    /// What is about to happen, in plain words.
    pub what: String,
    /// The tier "Allow in <app>" raises the app to; none on a card that asks for a yes.
    pub raise: Option<&'static str>,
}

/// The one action a card is about.
#[derive(Debug, Clone, PartialEq)]
pub struct Binding {
    pub app: String,
    pub tool: String,
    pub target: String,
    pub args: Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Answer {
    Do,
    Cancel,
    Allow,
}

/// One action the person said yes to, once. Only `Confirms::answer` makes one.
#[derive(Debug)]
pub struct Approval(u128);

#[cfg(test)]
impl Approval {
    /// The same token again, as a replay would present it.
    pub(crate) fn replayed(&self) -> Approval {
        Approval(self.0)
    }
}

/// How the wait on a card ended.
#[derive(Debug)]
pub enum Waited {
    Approved(Approval),
    Raised(Tier),
    Declined,
    TimedOut,
    /// The turn ended, or another card took its place.
    Ended,
}

#[derive(Debug)]
enum Decision {
    Do(Approval),
    Cancel,
    Allow(Tier),
}

struct Open {
    card: Card,
    binding: Binding,
    need: Option<Tier>,
    decision: Option<Decision>,
}

#[derive(Default)]
struct Inner {
    next: u64,
    open: Option<Open>,
    approvals: Vec<(u128, Binding, Instant)>,
}

#[derive(Default)]
pub struct Confirms {
    inner: Mutex<Inner>,
    cv: Condvar,
}

fn token() -> u128 {
    let mut b = [0u8; 16];
    if getrandom::getrandom(&mut b).is_err() {
        // No randomness: a token nobody can guess is still one nobody outside this process can send.
        let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
        b = (t ^ (std::process::id() as u128) << 64).to_le_bytes();
    }
    u128::from_le_bytes(b)
}

impl Confirms {
    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Show a card for `binding`; `need` is set when the app's tier is what stands in the way. A card already open
    /// ends (its call is told the turn moved on). Its id.
    pub(crate) fn open(&self, what: &str, need: Option<Tier>, binding: Binding) -> u64 {
        let mut g = self.lock();
        g.next += 1;
        let card = Card { id: g.next, app: binding.app.clone(), tool: binding.tool.clone(), what: what.into(), raise: need.map(Tier::word) };
        g.open = Some(Open { card, binding, need, decision: None });
        let id = g.next;
        drop(g);
        self.cv.notify_all();
        id
    }

    /// The card waiting for the person, if any.
    pub fn card(&self) -> Option<Card> {
        self.lock().open.as_ref().filter(|o| o.decision.is_none()).map(|o| o.card.clone())
    }

    /// The person's answer to card `id`. Do makes the approval for exactly that card's action, good for `WITHIN`;
    /// Allow gives back the app and the tier to keep.
    /// `raise` is told the app and its new tier before the waiting call hears the answer.
    pub fn answer(&self, id: u64, a: Answer, raise: &dyn Fn(&str, Tier)) -> Result<Option<(String, Tier)>, String> {
        let mut g = self.lock();
        let Some((need, binding)) = g.open.as_ref().filter(|o| o.card.id == id && o.decision.is_none()).map(|o| (o.need, o.binding.clone())) else {
            return Err("That question is no longer waiting.".into());
        };
        let (decision, raised) = match (a, need) {
            (Answer::Cancel, _) => (Decision::Cancel, None),
            (Answer::Allow, Some(t)) => (Decision::Allow(t), Some((binding.app.clone(), t))),
            (Answer::Allow, None) => return Err("This asks for a yes: Do it or Cancel.".into()),
            (Answer::Do, Some(_)) => return Err("This asks to allow the app: Allow or Cancel.".into()),
            (Answer::Do, None) => {
                let t = token();
                let now = Instant::now();
                g.approvals.retain(|(_, _, until)| *until > now);
                g.approvals.push((t, binding, now + WITHIN));
                (Decision::Do(Approval(t)), None)
            }
        };
        if let Some((app, t)) = &raised {
            raise(app, *t);
        }
        if let Some(o) = g.open.as_mut() {
            o.decision = Some(decision);
        }
        drop(g);
        self.cv.notify_all();
        Ok(raised)
    }

    /// Wait for the answer to card `id`, while `live` holds and for at most `within`. The card goes when it ends.
    pub(crate) fn wait(&self, id: u64, live: &dyn Fn() -> bool, within: Duration) -> Waited {
        let until = Instant::now() + within;
        let mut g = self.lock();
        loop {
            let ours = g.open.as_ref().is_some_and(|o| o.card.id == id);
            if !ours {
                return Waited::Ended;
            }
            if g.open.as_ref().is_some_and(|o| o.decision.is_some()) {
                let o = g.open.take().expect("checked");
                return match o.decision.expect("checked") {
                    Decision::Do(a) => Waited::Approved(a),
                    Decision::Cancel => Waited::Declined,
                    Decision::Allow(t) => Waited::Raised(t),
                };
            }
            if !live() {
                g.open = None;
                return Waited::Ended;
            }
            let now = Instant::now();
            if now >= until {
                g.open = None;
                return Waited::TimedOut;
            }
            g = self.cv.wait_timeout(g, (until - now).min(Duration::from_millis(50))).unwrap_or_else(|e| e.into_inner()).0;
        }
    }

    /// Spend `a` on the action `b`: true once, for exactly the action it was made for, within `WITHIN` of the yes.
    /// It is gone after this, whatever the answer.
    pub(crate) fn spend(&self, a: Approval, b: &Binding) -> bool {
        let mut g = self.lock();
        let Some(i) = g.approvals.iter().position(|(t, _, _)| *t == a.0) else { return false };
        let (_, bound, until) = g.approvals.remove(i);
        bound == *b && Instant::now() < until
    }

    /// The turn ended: the card goes, and its waiting call ends.
    pub(crate) fn end(&self) {
        self.lock().open = None;
        self.cv.notify_all();
    }

    #[cfg(test)]
    pub(crate) fn age(&self, by: Duration) {
        for (_, _, until) in self.lock().approvals.iter_mut() {
            *until -= by;
        }
    }
}
