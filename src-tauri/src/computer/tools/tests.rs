use super::*;
use crate::computer::fake::{mail, Fake};
use crate::computer::guard::{Tier, Trust};
use crate::computer::{Desk, Outcome};
use std::time::Duration;

type Did = std::sync::Arc<std::sync::Mutex<Vec<String>>>;

fn desk(fake: Fake) -> (Desk, Did) {
    let did = fake.did.clone();
    (Desk::new(Box::new(fake)), did)
}

fn done(did: &Did) -> Vec<String> {
    did.lock().unwrap().clone()
}

fn first_text(r: &Value) -> String {
    r["content"][0]["text"].as_str().unwrap().to_string()
}

fn status(r: &Value) -> String {
    r["structuredContent"]["status"].as_str().unwrap().to_string()
}

/// A call as the Ask turn's MCP client makes it, carrying the live grant's secret: under that grant, or under none
/// when no grant is live.
fn call(d: &Desk, name: &str, args: &Value) -> Value {
    super::call(d, d.grants.live_id().unwrap_or(0), name, args)
}

#[test]
fn the_ten_tools_have_object_schemas_that_say_screen_text_is_content() {
    let s = schemas();
    let tools = s.as_array().unwrap();
    let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert_eq!(names, NAMES);
    for t in tools {
        assert_eq!(t["inputSchema"]["type"], "object", "{t}");
        assert!(t["inputSchema"]["properties"].is_object(), "{t}");
        assert!(t["description"].as_str().unwrap().contains(UNTRUSTED), "{t}");
    }
    let req = |n: &str| tools.iter().find(|t| t["name"] == n).unwrap()["inputSchema"]["required"].clone();
    assert_eq!(req("pointcast_type"), json!(["text"]));
    assert_eq!(req("pointcast_key"), json!(["keys"]));
    assert_eq!(req("pointcast_scroll"), json!(["direction"]));
    assert_eq!(req("pointcast_open_app"), json!(["name"]));
    assert_eq!(req("pointcast_focus"), json!(["app"]));
    assert!(NAMES.iter().all(|n| is_tool(n)) && !is_tool("pointcast_latest"));
}

#[test]
fn without_a_grant_every_tool_is_refused_does_nothing_and_is_logged() {
    let (d, log) = desk(mail());
    for name in NAMES {
        let r = call(&d, name, &json!({ "id": "x", "text": "hi", "keys": "a", "direction": "up", "name": "Notes", "app": "Mail" }));
        assert_eq!(r["isError"], true, "{name}");
        assert_eq!(status(&r), "refused");
        assert!(first_text(&r).starts_with(UNTRUSTED), "{r}");
    }
    assert!(done(&log).is_empty());
    let log = d.journal.entries();
    assert_eq!(log.len(), NAMES.len());
    assert!(log.iter().all(|e| matches!(e.outcome, Outcome::Refused(_))));
    // A grant that ran out is no grant.
    let g = d.grants.grant(Duration::from_millis(20));
    std::thread::sleep(Duration::from_millis(40));
    assert_eq!(status(&call(&d, "pointcast_apps", &json!({}))), "refused");
    drop(g);
}

#[test]
fn every_result_starts_with_the_untrusted_line_and_is_logged() {
    let (d, _log) = desk(mail());
    d.set_trust(Trust::with([("Mail".to_string(), Tier::Full)]));
    let _g = d.grants.grant(Duration::from_secs(60));
    let calls = [
        ("pointcast_screen", json!({})),
        ("pointcast_elements", json!({})),
        ("pointcast_apps", json!({})),
        ("pointcast_find", json!({ "name": "send" })),
        ("pointcast_find", json!({})),
        ("pointcast_click", json!({ "id": "nope" })),
        ("pointcast_type", json!({ "text": "hi" })),
        ("pointcast_key", json!({ "keys": "cmd+s" })),
        ("pointcast_key", json!({ "keys": "hyper+s" })),
        ("pointcast_scroll", json!({ "direction": "down" })),
        ("pointcast_open_app", json!({ "name": "Notes" })),
        ("pointcast_focus", json!({ "app": "safari" })),
    ];
    for (name, args) in &calls {
        let r = call(&d, name, args);
        assert!(first_text(&r).starts_with(&format!("{UNTRUSTED}\n\n")), "{name}: {r}");
    }
    assert_eq!(d.journal.entries().len(), calls.len());
}

#[test]
fn elements_list_ids_never_a_password_and_leave_out_unnamed_holders() {
    let (d, _log) = desk(mail());
    let _g = d.grants.grant(Duration::from_secs(60));
    let r = call(&d, "pointcast_elements", &json!({}));
    assert_eq!(status(&r), "done", "{r}");
    let els = r["structuredContent"]["elements"].as_array().unwrap();
    let names: Vec<&str> = els.iter().map(|e| e["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["New Message", "Send", "Subject", "", "Password", "Bold"], "the unnamed group is left out");
    let pw = els.iter().find(|e| e["name"] == "Password").unwrap();
    assert!(pw["value"].is_null(), "{pw}");
    assert_eq!(els.iter().find(|e| e["raw_role"] == "AXTextArea").unwrap()["value"], "Hi Sam");
    let text = first_text(&r);
    assert!(text.contains("Mail \"New Message\": 6 element(s)") && text.contains("(password field)"), "{text}");
    let id = els[1]["id"].as_str().unwrap();
    assert!(text.contains(&format!("{id} button \"Send\" [press] @")), "{text}");
    // find by role or by part of the name
    let f = call(&d, "pointcast_find", &json!({ "role": "button" }));
    let found: Vec<&str> = f["structuredContent"]["elements"].as_array().unwrap().iter().map(|e| e["name"].as_str().unwrap()).collect();
    assert_eq!(found, ["Send", "Bold"]);
    let f = call(&d, "pointcast_find", &json!({ "name": "SUBJ", "role": "AXTextField" }));
    assert_eq!(f["structuredContent"]["elements"][0]["id"], els[2]["id"]);
    // limits
    let few = call(&d, "pointcast_elements", &json!({ "limit": 2 }));
    assert_eq!(few["structuredContent"]["elements"].as_array().unwrap().len(), 2);
    assert!(first_text(&few).contains("2 element(s) of 6"));
    let shallow = call(&d, "pointcast_elements", &json!({ "depth": 0 }));
    assert_eq!(shallow["structuredContent"]["elements"].as_array().unwrap().len(), 1);
}

fn id_of(d: &Desk, name: &str) -> String {
    let r = call(d, "pointcast_elements", &json!({}));
    let els = r["structuredContent"]["elements"].as_array().unwrap().clone();
    els.iter().find(|e| e["name"] == name || e["raw_role"] == name).unwrap()["id"].as_str().unwrap().to_string()
}

#[test]
fn a_click_presses_by_accessibility_and_send_waits_for_the_person() {
    let (d, log) = desk(mail());
    let _g = d.grants.grant(Duration::from_secs(60));
    let r = call(&d, "pointcast_click", &json!({ "id": id_of(&d, "Bold") }));
    assert_eq!(status(&r), "done", "{r}");
    let r = call(&d, "pointcast_click", &json!({ "id": id_of(&d, "Send") }));
    assert_eq!((status(&r), r["isError"].clone()), ("needs_confirm".to_string(), json!(false)));
    assert!(first_text(&r).contains("button \"Send\" in Mail"), "{r}");
    let r = call(&d, "pointcast_click", &json!({ "id": id_of(&d, "Bold"), "count": 2 }));
    assert_eq!(status(&r), "done");
    assert_eq!(done(&log), ["press Bold", "click 150,110 x2"], "a double click is synthetic at the centre");
    let last = d.journal.entries();
    let send = last.iter().find(|e| e.target == "button \"Send\"").unwrap();
    assert!(matches!(send.outcome, Outcome::NeedsConfirm(_)) && send.app == "Mail" && send.tool == "pointcast_click");
    // A stale id is an error, never a guess.
    assert_eq!(status(&call(&d, "pointcast_click", &json!({ "id": "zzzzzz" }))), "failed");
}

#[test]
fn a_click_at_a_point_asks_what_is_there_and_never_clicks_blind() {
    let mut f = mail();
    let (d0, log0) = desk(mail());
    let _g0 = d0.grants.grant(Duration::from_secs(60));
    let r = call(&d0, "pointcast_click", &json!({ "x": 150, "y": 110 }));
    assert_eq!(status(&r), "done", "{r}");
    assert_eq!(done(&log0), ["click 150,110 x1"]);
    f.at = None;
    let (d, log) = desk(f);
    let _g = d.grants.grant(Duration::from_secs(60));
    let r = call(&d, "pointcast_click", &json!({ "x": 5, "y": 5 }));
    assert_eq!(status(&r), "refused");
    assert!(done(&log).is_empty());
}

#[test]
fn typing_needs_full_never_reaches_a_password_and_return_waits() {
    let (d, log) = desk(mail());
    let _g = d.grants.grant(Duration::from_secs(60));
    let body = id_of(&d, "AXTextArea");
    let r = call(&d, "pointcast_type", &json!({ "id": body, "text": "hello" }));
    assert_eq!(status(&r), "refused", "Mail starts at click: {r}");
    d.set_trust(Trust::with([("Mail".to_string(), Tier::Full)]));
    assert_eq!(status(&call(&d, "pointcast_type", &json!({ "id": body, "text": "hello\nthere" }))), "done", "a value set sends nothing");
    assert_eq!(status(&call(&d, "pointcast_type", &json!({ "id": id_of(&d, "Password"), "text": "hunter2" }))), "refused");
    assert_eq!(status(&call(&d, "pointcast_type", &json!({ "text": "Lunch" }))), "done");
    assert_eq!(status(&call(&d, "pointcast_type", &json!({ "text": "Lunch\n" }))), "needs_confirm", "a typed Return can send");
    assert_eq!(status(&call(&d, "pointcast_type", &json!({ "text": "x".repeat(5001) }))), "failed");
    assert_eq!(done(&log), ["set =hello\nthere", "type Lunch"]);
}

#[test]
fn keys_parse_and_return_or_delete_combos_wait() {
    assert_eq!(parse_keys("Cmd+Shift+S").unwrap(), Combo { mods: vec![Modifier::Cmd, Modifier::Shift], key: "s".into() });
    assert_eq!(parse_keys("enter").unwrap().key, "return");
    assert_eq!(parse_keys("ctrl+alt+Esc").unwrap(), Combo { mods: vec![Modifier::Ctrl, Modifier::Alt], key: "escape".into() });
    assert_eq!(parse_keys("option+F12").unwrap().key, "f12");
    for bad in ["", "cmd+", "hyper+s", "cmd+ss", "f13"] {
        assert!(parse_keys(bad).is_err(), "{bad}");
    }
    for (keys, sends) in [("return", true), ("cmd+return", true), ("cmd+backspace", true), ("ctrl+delete", true), ("shift+delete", true), ("backspace", false), ("cmd+s", false), ("tab", false), ("escape", false)] {
        assert_eq!(commits(&parse_keys(keys).unwrap()), sends, "{keys}");
    }
    let (d, log) = desk(mail());
    let _g = d.grants.grant(Duration::from_secs(60));
    assert_eq!(status(&call(&d, "pointcast_key", &json!({ "keys": "cmd+s" }))), "refused", "keys need full");
    d.set_trust(Trust::with([("Mail".to_string(), Tier::Full)]));
    assert_eq!(status(&call(&d, "pointcast_key", &json!({ "keys": "cmd+s" }))), "done");
    assert_eq!(status(&call(&d, "pointcast_key", &json!({ "keys": "return" }))), "needs_confirm");
    assert_eq!(done(&log), ["key [Cmd]+s"]);
}

#[test]
fn scroll_open_and_focus_ask_the_guard_and_apps_leave_pointcast_out() {
    let (d, log) = desk(mail());
    let _g = d.grants.grant(Duration::from_secs(60));
    assert_eq!(status(&call(&d, "pointcast_scroll", &json!({ "direction": "down", "amount": 3 }))), "done");
    assert_eq!(status(&call(&d, "pointcast_scroll", &json!({ "direction": "sideways" }))), "failed");
    assert_eq!(status(&call(&d, "pointcast_open_app", &json!({ "name": "TextEdit" }))), "done");
    assert_eq!(status(&call(&d, "pointcast_open_app", &json!({ "name": "Pointcast" }))), "refused");
    assert_eq!(status(&call(&d, "pointcast_open_app", &json!({ "name": "SecurityAgent" }))), "refused");
    assert_eq!(status(&call(&d, "pointcast_open_app", &json!({ "name": "/bin/sh" }))), "failed");
    assert_eq!(status(&call(&d, "pointcast_focus", &json!({ "app": "safari" }))), "done", "bringing a view app forward is seeing it");
    assert_eq!(status(&call(&d, "pointcast_focus", &json!({ "app": "pointcast" }))), "refused");
    assert_eq!(status(&call(&d, "pointcast_focus", &json!({ "app": "Notes" }))), "failed");
    assert_eq!(done(&log), ["scroll 140,110 by 0,3", "open TextEdit", "focus Safari None"]);
    let r = call(&d, "pointcast_apps", &json!({}));
    let text = first_text(&r);
    assert!(text.contains("Mail [front] (click)") && text.contains("Safari (view)") && !text.contains("Pointcast\n"), "{text}");
    assert_eq!(r["structuredContent"]["apps"].as_array().unwrap().len(), 2);
}

#[test]
fn the_screen_comes_as_text_then_an_image_block() {
    let (d, _log) = desk(mail());
    let _g = d.grants.grant(Duration::from_secs(60));
    let r = call(&d, "pointcast_screen", &json!({}));
    let content = r["content"].as_array().unwrap();
    assert_eq!(content.len(), 2, "{r}");
    assert_eq!((content[1]["type"].as_str(), content[1]["mimeType"].as_str()), (Some("image"), Some("image/png")));
    assert!(first_text(&r).contains("(0 + px * 100 / 200, 0 + py * 50 / 100)"), "{r}");
    assert!(r["structuredContent"].get("image").is_none());
}

#[test]
fn no_screenshot_while_a_password_prompt_or_pointcast_is_in_front() {
    for front in ["SecurityAgent", "Pointcast"] {
        let mut fake = mail();
        fake.tree.app.name = front.into();
        let (d, _log) = desk(fake);
        let _g = d.grants.grant(Duration::from_secs(60));
        let r = call(&d, "pointcast_screen", &json!({}));
        assert_eq!(status(&r), "refused", "{front}: {r}");
        assert_eq!(r["content"].as_array().unwrap().len(), 1, "{front}: no picture goes out: {r}");
    }
}

mod cards {
    use super::*;
    use crate::computer::confirm::{Binding, Confirms, Waited};
    use crate::computer::{Answer, Grants, Hooks, WITHIN};

    fn leaked(fake: Fake) -> (&'static Desk, Did) {
        let did = fake.did.clone();
        (Box::leak(Box::new(Desk::new(Box::new(fake)))), did)
    }

    /// The card the call is waiting on, once it shows.
    fn card(d: &Desk) -> crate::computer::Card {
        let t = std::time::Instant::now();
        loop {
            if let Some(c) = d.confirms.card() {
                return c;
            }
            assert!(t.elapsed() < Duration::from_secs(10), "no card showed");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn under(grant: u64, target: &str) -> Binding {
        Binding { grant, app: "Mail".into(), tool: "pointcast_click".into(), target: target.into(), args: json!({ "id": "abc" }) }
    }

    /// A yes on a card for `b`: the approval it makes.
    fn yes(c: &Confirms, b: Binding) -> crate::computer::confirm::Approval {
        let id = c.open("This would click button \"Send\" in Mail.", None, b);
        assert_eq!(c.answer(id, Answer::Allow, &|_, _| ()), Err("This asks for a yes: Do it or Cancel.".into()), "no tier to raise on a yes card");
        assert_eq!(c.answer(id, Answer::Do, &|_, _| ()), Ok(None));
        assert!(c.answer(id, Answer::Do, &|_, _| ()).is_err(), "answered once");
        let Waited::Approved(a) = c.wait(id, Duration::from_secs(1)) else { panic!("approved") };
        assert!(c.card().is_none(), "the card goes once answered");
        a
    }

    #[test]
    fn an_approval_works_once_only_for_the_same_target_and_expires() {
        let grants = Grants::default();
        let g = grants.grant(Duration::from_secs(60));
        let c = Confirms::new(grants.clone());
        let binding = |target: &str| under(g.id(), target);
        let a = yes(&c, binding("button \"Send\""));
        let again = a.replayed();
        assert!(c.spend(a, &binding("button \"Send\"")), "the action it was made for");
        assert!(!c.spend(again, &binding("button \"Send\"")), "once only");
        let a = yes(&c, binding("button \"Send\""));
        assert!(!c.spend(a.replayed(), &binding("button \"Delete\"")), "another target");
        assert!(!c.spend(a, &binding("button \"Send\"")), "and spent by the try");
        let mut other_args = binding("button \"Send\"");
        other_args.args = json!({ "id": "abc", "count": 2 });
        let a = yes(&c, binding("button \"Send\""));
        assert!(!c.spend(a, &other_args), "other arguments");
        let a = yes(&c, binding("button \"Send\""));
        c.age(WITHIN);
        assert!(!c.spend(a, &binding("button \"Send\"")), "expired after a minute");
        // A card nobody answers, and one whose turn ends.
        let id = c.open("x", None, binding("button \"Send\""));
        assert!(matches!(c.wait(id, Duration::from_millis(30)), Waited::TimedOut));
        let id = c.open("x", None, binding("button \"Send\""));
        drop(g);
        assert!(matches!(c.wait(id, Duration::from_secs(5)), Waited::Ended));
        assert!(c.card().is_none());
    }

    #[test]
    fn an_approval_made_under_one_grant_opens_nothing_once_another_replaces_it() {
        let grants = Grants::default();
        let c = Confirms::new(grants.clone());
        let a = grants.grant(Duration::from_secs(60));
        let ok = yes(&c, under(a.id(), "button \"Send\""));
        // Turn B takes over before the yes is spent: it opens nothing, under A's binding or under B's.
        let b = grants.grant(Duration::from_secs(60));
        assert!(!c.spend(ok.replayed(), &under(a.id(), "button \"Send\"")), "A's grant is over");
        assert!(!c.spend(ok, &under(b.id(), "button \"Send\"")), "made under A, never B's");
        // The sweep a new turn makes drops A's approvals and A's card at once.
        let _kept = yes(&c, under(b.id(), "button \"Send\""));
        let a2 = grants.grant(Duration::from_secs(60));
        let _old = yes(&c, under(a2.id(), "button \"Send\""));
        let id = c.open("x", None, under(a2.id(), "button \"Send\""));
        let b2 = grants.grant(Duration::from_secs(60));
        assert!(c.card().is_none(), "a card of a grant that is over is never shown");
        assert_eq!(c.answer(id, Answer::Do, &|_, _| ()), Err("That question is no longer waiting.".into()), "nor answered");
        c.sweep();
        assert_eq!(c.approvals(), 0, "every approval of an earlier grant is gone");
        assert!(matches!(c.wait(id, Duration::from_secs(5)), Waited::Ended));
        drop((a, b, a2, b2));
    }

    #[test]
    fn a_new_turn_cancels_the_card_an_earlier_turn_is_waiting_on() {
        let (d, log) = leaked(mail());
        d.set_trust(Trust::with([("Mail".to_string(), Tier::Full)]));
        d.ask_for(Duration::from_secs(10));
        let first = d.begin(Duration::from_secs(60));
        let send = id_of(d, "Send");
        let waiting = std::thread::spawn(move || call(d, "pointcast_click", &json!({ "id": send })));
        let c = card(d);
        let t = std::time::Instant::now();
        let second = d.begin(Duration::from_secs(60));
        assert!(d.confirms.card().is_none(), "the card goes when the new turn starts");
        let r = waiting.join().unwrap();
        assert!(t.elapsed() < Duration::from_secs(2), "the waiting call ends at once");
        assert_eq!(status(&r), "refused", "{r}");
        assert!(d.answer(c.id, Answer::Do).is_err(), "a yes after the new turn started counts for nothing");
        // The earlier turn letting go late leaves the new turn's card alone.
        let send = id_of(d, "Send");
        let waiting = std::thread::spawn(move || call(d, "pointcast_click", &json!({ "id": send })));
        let c = card(d);
        drop(first);
        assert_eq!(d.confirms.card().map(|k| k.id), Some(c.id), "the new turn's card stays");
        d.answer(c.id, Answer::Cancel).unwrap();
        assert_eq!(status(&waiting.join().unwrap()), "declined");
        drop(second);
        assert!(done(&log).is_empty(), "Send was never pressed");
    }

    #[test]
    fn a_yes_given_under_one_grant_does_nothing_when_another_takes_over_before_the_call_resumes() {
        let (d, log) = leaked(mail());
        d.set_trust(Trust::with([("Mail".to_string(), Tier::Full)]));
        // Right after the person's yes wakes the waiting call (its notice, on its own thread), turn B takes over.
        let armed = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let taken: &'static std::sync::Mutex<Vec<crate::computer::Grant>> = Box::leak(Box::default());
        let trigger = armed.clone();
        d.set_hooks(Hooks {
            status: Box::new(move |s| {
                let on_call = std::thread::current().name() == Some("the-call");
                if on_call && s.card.is_none() && trigger.swap(false, std::sync::atomic::Ordering::SeqCst) {
                    taken.lock().unwrap().push(d.grants.grant(Duration::from_secs(60)));
                }
            }),
            stop: Box::new(|| ()),
        });
        d.ask_for(Duration::from_secs(10));
        let a = d.grants.grant(Duration::from_secs(60));
        let send = id_of(d, "Send");
        let waiting = std::thread::Builder::new().name("the-call".into()).spawn(move || call(d, "pointcast_click", &json!({ "id": send }))).unwrap();
        let c = card(d);
        armed.store(true, std::sync::atomic::Ordering::SeqCst);
        d.answer(c.id, Answer::Do).unwrap();
        let r = waiting.join().unwrap();
        assert!(!armed.load(std::sync::atomic::Ordering::SeqCst), "turn B took over between the yes and the resumed call");
        assert_eq!(status(&r), "refused", "{r}");
        assert!(first_text(&r).contains("The turn ended"), "{r}");
        assert!(done(&log).is_empty(), "Send was never pressed under turn B");
        assert!(!a.live() && d.granted());
        assert_eq!(d.confirms.approvals(), 0, "the yes was not kept for later");
    }

    #[test]
    fn do_it_lets_that_one_call_through_and_the_next_one_asks_again() {
        let (d, log) = leaked(mail());
        d.set_trust(Trust::with([("Mail".to_string(), Tier::Full)]));
        d.ask_for(Duration::from_secs(10));
        let _g = d.grants.grant(Duration::from_secs(60));
        let send = id_of(d, "Send");
        let waiting = std::thread::spawn(move || call(d, "pointcast_click", &json!({ "id": send })));
        let c = card(d);
        assert_eq!((c.app.as_str(), c.tool.as_str(), c.raise), ("Mail", "pointcast_click", None));
        assert!(c.what.contains("button \"Send\" in Mail"), "{}", c.what);
        assert!(done(&log).is_empty(), "nothing happens before the press");
        d.answer(c.id, Answer::Do).unwrap();
        let r = waiting.join().unwrap();
        assert_eq!(status(&r), "done", "{r}");
        assert_eq!(done(&log), ["press Send"]);
        assert!(!r.to_string().contains("approval"), "no token ever reaches the AI");
        let last = d.journal.entries();
        assert!(matches!(&last[last.len() - 2].outcome, Outcome::NeedsConfirm(_)));
        assert!(matches!(&last[last.len() - 1].outcome, Outcome::Done(how) if how.ends_with("after the person's yes")), "{last:?}");
        // The yes was for that call only: the same click again asks again, and unanswered does nothing.
        d.ask_for(Duration::from_millis(60));
        let send = id_of(d, "Send");
        let r = call(d, "pointcast_click", &json!({ "id": send }));
        assert_eq!(status(&r), "refused", "{r}");
        assert!(first_text(&r).contains("did not answer"), "{r}");
        assert_eq!(done(&log), ["press Send"]);
    }

    #[test]
    fn cancel_tells_the_ai_the_person_declined_and_a_stop_ends_the_wait() {
        let (d, log) = leaked(mail());
        d.set_trust(Trust::with([("Mail".to_string(), Tier::Full)]));
        d.ask_for(Duration::from_secs(10));
        let g = d.grants.grant(Duration::from_secs(60));
        let send = id_of(d, "Send");
        let waiting = std::thread::spawn(move || call(d, "pointcast_click", &json!({ "id": send })));
        d.answer(card(d).id, Answer::Cancel).unwrap();
        let r = waiting.join().unwrap();
        assert_eq!((status(&r), r["isError"].clone()), ("declined".to_string(), json!(false)), "{r}");
        assert!(first_text(&r).contains("The person declined"), "{r}");
        let send = id_of(d, "Send");
        let waiting = std::thread::spawn(move || call(d, "pointcast_click", &json!({ "id": send })));
        let c = card(d);
        let t = std::time::Instant::now();
        drop(g);
        let r = waiting.join().unwrap();
        assert!(t.elapsed() < Duration::from_secs(2) && status(&r) == "refused", "{r}");
        assert!(d.answer(c.id, Answer::Do).is_err(), "too late");
        assert!(done(&log).is_empty());
    }

    #[test]
    fn allow_in_the_app_raises_its_tier_to_what_the_action_needs_and_the_call_goes_on() {
        let (d, log) = leaked(mail());
        d.ask_for(Duration::from_secs(10));
        let _g = d.grants.grant(Duration::from_secs(60));
        let body = id_of(d, "AXTextArea");
        let waiting = std::thread::spawn(move || call(d, "pointcast_type", &json!({ "id": body, "text": "hello" })));
        let c = card(d);
        assert_eq!(c.raise, Some("full"), "Mail starts at click; typing needs full");
        assert!(d.answer(c.id, Answer::Do).is_err(), "a tier card is allowed or cancelled");
        assert_eq!(d.answer(c.id, Answer::Allow), Ok(Some(("Mail".to_string(), Tier::Full))), "what the settings keep");
        let r = waiting.join().unwrap();
        assert_eq!(status(&r), "done", "{r}");
        assert_eq!(d.trust().tier("Mail"), Tier::Full);
        assert_eq!(done(&log), ["set =hello"]);
        // A click-tier refusal raises to click only.
        let (d, _) = leaked(mail());
        d.set_trust(Trust::with([("Mail".to_string(), Tier::View)]));
        d.ask_for(Duration::from_secs(10));
        let _g = d.grants.grant(Duration::from_secs(60));
        let bold = id_of(d, "Bold");
        let waiting = std::thread::spawn(move || call(d, "pointcast_click", &json!({ "id": bold })));
        let c = card(d);
        assert_eq!(c.raise, Some("click"));
        assert_eq!(d.answer(c.id, Answer::Allow), Ok(Some(("Mail".to_string(), Tier::Click))));
        assert_eq!(status(&waiting.join().unwrap()), "done");
        // The never-list asks nobody.
        let r = call(d, "pointcast_open_app", &json!({ "name": "SecurityAgent" }));
        assert_eq!(status(&r), "refused");
        assert!(d.confirms.card().is_none());
    }
}
