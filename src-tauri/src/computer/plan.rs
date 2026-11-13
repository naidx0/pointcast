//! A plan of several steps, shown before anything acts (`pointcast_plan`), with its steps ticked as they end
//! (`pointcast_plan_step`).
//!
//! For work that takes more than one action the AI first says what it will do and in which apps. The Ask panel shows
//! that as a checklist with **Start** and **Cancel**, and the call waits. Start approves the plan: from then on the act
//! tools run in the plan's apps only, and the tiers and the confirm cards still apply on top. Cancel, or no answer in
//! time, refuses every computer tool for the rest of the turn. While a plan waits, nothing acts. A turn with no plan
//! acts as before: a single obvious action needs none.
//!
//! The AI ticks the steps itself, by number: Pointcast does not guess from the journal which free-text step an action
//! belonged to. Like a card, a plan belongs to the grant whose call made it and counts only while that grant is the
//! live one. Start is given here only, by `answer`, which only the panel's command calls: no MCP request reaches it.
use super::grant::Grants;
use super::guard::{self, Kind, Target};
use super::{Desk, Outcome};
use serde::Serialize;
use serde_json::{json, Value};
use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};

pub const MAX_STEPS: usize = 12;
pub const MAX_APPS: usize = 6;
/// A step's or a note's words are cut past this many characters.
const KEEP: usize = 200;

/// Where a step stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Mark {
    Pending,
    Done,
    Skipped,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Step {
    pub text: String,
    pub status: Mark,
    pub note: Option<String>,
}

/// Where the plan stands: waiting for Start, started, or cancelled (by the person, or by no answer in time).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum State {
    Proposed,
    Started,
    Cancelled,
}

/// What the panel shows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Plan {
    pub id: u64,
    pub steps: Vec<Step>,
    pub apps: Vec<String>,
    pub state: State,
}

struct Held {
    plan: Plan,
    grant: u64,
}

#[derive(Default)]
struct Inner {
    next: u64,
    held: Option<Held>,
}

/// How the wait for Start ended.
#[derive(Debug, PartialEq, Eq)]
pub enum Waited {
    Started,
    Cancelled,
    TimedOut,
    /// The turn ended, or a newer plan took its place.
    Ended,
}

/// What the AI is told once the person cancelled, for every tool it calls after.
pub const CANCELLED: &str = "The person cancelled the plan: nothing more is done this turn. Stop and say so.";

pub struct Plans {
    inner: Mutex<Inner>,
    cv: Condvar,
    grants: Grants,
}

/// The words of an app's name, lowered, without ".app" or ".exe".
fn words(app: &str) -> Vec<String> {
    guard::app_key(app).split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).map(String::from).collect()
}

fn holds_run(hay: &[String], needle: &[String]) -> bool {
    !needle.is_empty() && needle.len() <= hay.len() && hay.windows(needle.len()).any(|w| w == needle)
}

/// Whether `app` is one of the plan's `apps`: the same name, or one name's words a run of the other's ("Chrome" and
/// "Google Chrome"). Whole words only, so a plan for "e" opens nothing.
pub fn planned(apps: &[String], app: &str) -> bool {
    let have = words(app);
    apps.iter().any(|p| {
        let want = words(p);
        holds_run(&have, &want) || holds_run(&want, &have)
    })
}

impl Plans {
    pub(crate) fn new(grants: Grants) -> Plans {
        Plans { inner: Mutex::default(), cv: Condvar::new(), grants }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Show a plan under `grant`, waiting for Start. One already there, started or not, goes: a new plan needs a new
    /// Start. Its id.
    pub(crate) fn propose(&self, grant: u64, steps: Vec<String>, apps: Vec<String>) -> u64 {
        let mut g = self.lock();
        g.next += 1;
        let id = g.next;
        let steps = steps.into_iter().map(|text| Step { text, status: Mark::Pending, note: None }).collect();
        g.held = Some(Held { plan: Plan { id, steps, apps, state: State::Proposed }, grant });
        drop(g);
        self.cv.notify_all();
        id
    }

    /// The plan the panel shows: never one whose grant is over.
    pub fn plan(&self) -> Option<Plan> {
        self.lock().held.as_ref().filter(|h| self.grants.holds(h.grant)).map(|h| h.plan.clone())
    }

    /// The person's Start (`true`) or Cancel on plan `id`, while it waits.
    pub fn answer(&self, id: u64, start: bool) -> Result<(), String> {
        let mut g = self.lock();
        let waiting = |h: &&mut Held| h.plan.id == id && h.plan.state == State::Proposed && self.grants.holds(h.grant);
        let Some(h) = g.held.as_mut().filter(waiting) else {
            return Err("That plan is no longer waiting.".into());
        };
        h.plan.state = if start { State::Started } else { State::Cancelled };
        drop(g);
        self.cv.notify_all();
        Ok(())
    }

    /// Wait for the answer to plan `id`, while its grant is the live one and for at most `within`. No answer in time
    /// counts as Cancel.
    pub(crate) fn wait(&self, id: u64, within: Duration) -> Waited {
        let until = Instant::now() + within;
        let mut g = self.lock();
        loop {
            let Some((grant, state)) = g.held.as_ref().filter(|h| h.plan.id == id).map(|h| (h.grant, h.plan.state)) else {
                return Waited::Ended;
            };
            if !self.grants.holds(grant) {
                g.held = None;
                return Waited::Ended;
            }
            match state {
                State::Started => return Waited::Started,
                State::Cancelled => return Waited::Cancelled,
                State::Proposed => {}
            }
            let now = Instant::now();
            if now >= until {
                if let Some(h) = g.held.as_mut() {
                    h.plan.state = State::Cancelled;
                }
                return Waited::TimedOut;
            }
            g = self.cv.wait_timeout(g, (until - now).min(Duration::from_millis(50))).unwrap_or_else(|e| e.into_inner()).0;
        }
    }

    /// Mark step `index` (1 for the first) of the started plan under `grant`. The plan as it is now.
    pub(crate) fn tick(&self, grant: u64, index: usize, mark: Mark, note: Option<String>) -> Result<Plan, String> {
        let mut g = self.lock();
        let Some(h) = g.held.as_mut().filter(|h| h.grant == grant && h.plan.state == State::Started && self.grants.holds(grant)) else {
            return Err("No started plan to tick: call pointcast_plan first, and wait for the person's Start.".into());
        };
        let n = h.plan.steps.len();
        let step = index.checked_sub(1).and_then(|i| h.plan.steps.get_mut(i)).ok_or_else(|| format!("The plan has {n} step(s): give a number from 1 to {n}."))?;
        step.status = mark;
        step.note = note;
        Ok(h.plan.clone())
    }

    /// Whether the plan under `grant` lets `kind` happen in `app`. No plan: yes, as before plans. A plan waiting for
    /// Start: looking only. A started plan: acting in its apps only. A cancelled one: nothing.
    pub(crate) fn allows(&self, grant: u64, kind: Kind, app: &str) -> Result<(), String> {
        let g = self.lock();
        let Some(h) = g.held.as_ref().filter(|h| h.grant == grant && self.grants.holds(grant)) else { return Ok(()) };
        match h.plan.state {
            State::Cancelled => Err(CANCELLED.into()),
            _ if kind == Kind::See => Ok(()),
            State::Proposed => Err("The plan waits for the person to press Start: nothing acts until they do.".into()),
            State::Started if planned(&h.plan.apps, app) => Ok(()),
            State::Started => Err(format!(
                "{} is not in the plan the person started, which acts in {} only. Stop and ask the person before acting anywhere else.",
                if app.is_empty() { "That app" } else { app },
                h.plan.apps.join(", ")
            )),
        }
    }

    /// Why every tool is refused under `grant` now: the person cancelled its plan.
    pub(crate) fn stopped(&self, grant: u64) -> Option<&'static str> {
        let g = self.lock();
        g.held.as_ref().filter(|h| h.grant == grant && h.plan.state == State::Cancelled).map(|_| CANCELLED)
    }

    /// A grant ended or another took its place: a plan of a grant that is not the live one goes now.
    pub(crate) fn sweep(&self) {
        let mut g = self.lock();
        if g.held.as_ref().is_some_and(|h| !self.grants.holds(h.grant)) {
            g.held = None;
        }
        drop(g);
        self.cv.notify_all();
    }

    /// The stop key: the plan goes, and a call waiting on it ends.
    pub(crate) fn end(&self) {
        self.lock().held = None;
        self.cv.notify_all();
    }
}

/// The text of a list argument: trimmed, empty ones left out, each cut at `KEEP`.
fn lines(args: &Value, key: &str) -> Vec<String> {
    args.get(key)
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).map(|s| s.chars().take(KEEP).collect()).collect())
        .unwrap_or_default()
}

fn numbered(plan: &Plan) -> String {
    plan.steps.iter().enumerate().map(|(i, s)| format!("{}. {}", i + 1, s.text)).collect::<Vec<_>>().join("\n")
}

/// `pointcast_plan` and `pointcast_plan_step` under grant `grant`, which the caller checked is the live one.
pub(crate) fn call(desk: &Desk, grant: u64, name: &str, args: &Value) -> Value {
    let result = super::tools::result;
    let refuse = |app: &str, target: &str, why: String| {
        desk.journal.push(app, name, target, Outcome::Refused(why.clone()));
        result(&format!("Not done. {why}"), json!({ "status": "refused", "reason": why }), true)
    };
    if name == "pointcast_plan_step" {
        let index = args.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
        let target = format!("step {index}");
        let mark = match args.get("status").and_then(Value::as_str) {
            Some("done") => Mark::Done,
            Some("skipped") => Mark::Skipped,
            Some("failed") => Mark::Failed,
            _ => return refuse("", &target, "pointcast_plan_step needs a status: done, skipped or failed.".into()),
        };
        let note = args.get("note").and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).map(|s| s.chars().take(KEEP).collect());
        let plan = match desk.plans.tick(grant, index, mark, note) {
            Ok(p) => p,
            Err(why) => return refuse("", &target, why),
        };
        desk.notify();
        let word = serde_json::to_value(mark).ok().and_then(|v| v.as_str().map(String::from)).unwrap_or_default();
        let left = plan.steps.iter().filter(|s| s.status == Mark::Pending).count();
        let mut text = format!("Step {index} of {} is ticked {word}.", plan.steps.len());
        if mark == Mark::Failed {
            text += " Stop and ask the person what to do.";
        } else if left == 0 {
            text += " Every step is ticked: say in a sentence or two what you did.";
        }
        desk.journal.push(&plan.apps.join(", "), name, &target, Outcome::Done(word.clone()));
        return result(&text, json!({ "status": "done", "index": index, "mark": word, "left": left }), false);
    }
    let (steps, apps) = (lines(args, "steps"), lines(args, "apps"));
    let target = format!("{} step(s)", steps.len());
    if steps.is_empty() || steps.len() > MAX_STEPS {
        return refuse("", &target, format!("A plan has 1 to {MAX_STEPS} steps, one short line each."));
    }
    if apps.is_empty() || apps.len() > MAX_APPS {
        return refuse("", &target, format!("A plan names the 1 to {MAX_APPS} apps it acts in."));
    }
    let mut keep: Vec<String> = vec![];
    for a in apps {
        if a.len() > 100 || a.starts_with('-') || a.contains(['/', '\\']) || a.chars().any(char::is_control) {
            return refuse("", &target, "Name each app by its name, not a path.".into());
        }
        if let Some(why) = guard::never(Kind::Focus, &Target { app: a.clone(), ..Default::default() }) {
            return refuse(&a, &target, why);
        }
        if !keep.iter().any(|k| guard::app_key(k) == guard::app_key(&a)) {
            keep.push(a);
        }
    }
    let in_apps = keep.join(", ");
    let id = desk.plans.propose(grant, steps, keep);
    desk.notify();
    let within = desk.asking();
    let waited = desk.plans.wait(id, within);
    desk.notify();
    let plan = desk.plans.plan().filter(|p| p.id == id);
    match (waited, plan) {
        (Waited::Started, Some(plan)) => {
            desk.journal.push(&in_apps, name, &target, Outcome::Done("started by the person".into()));
            let text = format!(
                "The person pressed Start. Go ahead one step at a time, acting in {in_apps} only. After each step call pointcast_plan_step with its number and done, skipped or failed. If anything unexpected happens (an action refused or failing, a window you did not expect, a need for an app not in the plan), stop and ask the person.\n\n{}",
                numbered(&plan)
            );
            result(&text, json!({ "status": "started", "plan": plan }), false)
        }
        (Waited::Cancelled, _) => {
            let why = "The person cancelled the plan: nothing was done, and nothing more is done this turn. Say so in a sentence.".to_string();
            desk.journal.push(&in_apps, name, &target, Outcome::Refused("The person declined the plan".into()));
            result(&why, json!({ "status": "declined", "reason": why }), false)
        }
        (Waited::TimedOut, _) if within.is_zero() => refuse(&in_apps, &target, "Nobody can press Start now, so nothing was done. Say what you planned.".into()),
        (Waited::TimedOut, _) => refuse(
            &in_apps,
            &target,
            format!("The person did not press Start within {} seconds, so nothing was done and nothing more is done this turn. Say what you planned.", within.as_secs()),
        ),
        _ => refuse(&in_apps, &target, "The turn ended before the person pressed Start: nothing was done.".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_app_is_in_the_plan_by_its_name_or_a_whole_word_run_of_it() {
        let apps = vec!["TextEdit".to_string(), "Google Chrome".into(), "Word".into()];
        for app in ["TextEdit", "textedit.app", "TEXTEDIT", "Google Chrome", "Chrome", "Microsoft Word", "word.exe"] {
            assert!(planned(&apps, app), "{app}");
        }
        for app in ["Notes", "Text", "Edit", "Chromium", "Wordle", "", "Mail"] {
            assert!(!planned(&apps, app), "{app}");
        }
        assert!(!planned(&["e".to_string()], "TextEdit"), "whole words only");
        assert!(!planned(&[], "TextEdit"));
    }
}
