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
fn the_tools_have_object_schemas_that_say_screen_text_is_content() {
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
    assert_eq!(req("pointcast_files"), json!(["action", "path"]));
    assert!(NAMES.iter().all(|n| is_tool(n)) && !is_tool("pointcast_latest"));
}

#[test]
fn the_plan_tools_take_steps_and_apps_then_a_step_number_and_how_it_ended() {
    let s = schemas();
    let tool = |n: &str| s.as_array().unwrap().iter().find(|t| t["name"] == n).cloned().unwrap_or_else(|| panic!("{n} is served"));
    let plan = tool("pointcast_plan");
    let p = &plan["inputSchema"];
    assert_eq!(p["required"], json!(["steps", "apps"]));
    for key in ["steps", "apps"] {
        assert_eq!((p["properties"][key]["type"].as_str(), p["properties"][key]["items"]["type"].as_str()), (Some("array"), Some("string")), "{key}");
        assert_eq!(p["properties"][key]["minItems"], 1, "{key}");
    }
    assert_eq!(p["properties"]["steps"]["maxItems"], crate::computer::plan::MAX_STEPS);
    assert_eq!(p["properties"]["apps"]["maxItems"], crate::computer::plan::MAX_APPS);
    let d = plan["description"].as_str().unwrap();
    assert!(d.contains("Start") && d.contains("more than one action") && d.contains("needs no plan"), "{d}");
    let step = tool("pointcast_plan_step");
    let p = &step["inputSchema"];
    assert_eq!(p["required"], json!(["index", "status"]));
    assert_eq!((p["properties"]["index"]["type"].as_str(), p["properties"]["index"]["minimum"].as_u64()), (Some("integer"), Some(1)), "numbered from 1");
    assert_eq!(p["properties"]["status"]["enum"], json!(["done", "skipped", "failed"]));
    assert_eq!(p["properties"]["note"]["type"], "string");
    assert!(NAMES.ends_with(&["pointcast_plan", "pointcast_plan_step"]));
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

mod plans {
    use super::*;
    use crate::computer::plan::{Mark, Plan, State, CANCELLED};
    use crate::computer::{Answer, Hooks};

    fn leaked(fake: Fake) -> (&'static Desk, Did) {
        let did = fake.did.clone();
        (Box::leak(Box::new(Desk::new(Box::new(fake)))), did)
    }

    /// The plan the panel shows once it is there in `state`.
    fn shown(d: &Desk, state: State) -> Plan {
        let t = std::time::Instant::now();
        loop {
            if let Some(p) = d.status().plan.filter(|p| p.state == state) {
                return p;
            }
            assert!(t.elapsed() < Duration::from_secs(10), "no {state:?} plan showed");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    /// The AI's `pointcast_plan` call, on its own thread: it waits for the person.
    fn propose(d: &'static Desk, steps: &[&str], apps: &[&str]) -> std::thread::JoinHandle<Value> {
        let args = json!({ "steps": steps, "apps": apps });
        std::thread::spawn(move || call(d, "pointcast_plan", &args))
    }

    /// A plan for `apps`, shown and started by the person.
    fn started(d: &'static Desk, apps: &[&str]) -> Value {
        let waiting = propose(d, &["Open it", "Write the list", "Select all"], apps);
        let p = shown(d, State::Proposed);
        d.start_plan(p.id, true).unwrap();
        waiting.join().unwrap()
    }

    #[test]
    fn nothing_acts_before_start_and_after_it_only_in_the_planned_apps() {
        let (d, log) = leaked(mail());
        d.ask_for(Duration::from_secs(10));
        let _g = d.grants.grant(Duration::from_secs(60));
        let waiting = propose(d, &["Make the text bold", "Write the subject"], &["Mail"]);
        let p = shown(d, State::Proposed);
        assert_eq!((p.apps.clone(), p.steps.len()), (vec!["Mail".to_string()], 2));
        assert!(p.steps.iter().all(|s| s.status == Mark::Pending && s.note.is_none()));
        // Before Start: looking goes on, acting is refused, in the plan's apps or not.
        assert_eq!(status(&call(d, "pointcast_elements", &json!({}))), "done");
        for (name, args) in [
            ("pointcast_click", json!({ "id": id_of(d, "Bold") })),
            ("pointcast_type", json!({ "id": id_of(d, "Subject"), "text": "hi" })),
            ("pointcast_key", json!({ "keys": "cmd+b" })),
            ("pointcast_scroll", json!({ "direction": "down" })),
            ("pointcast_open_app", json!({ "name": "Mail" })),
            ("pointcast_focus", json!({ "app": "Mail" })),
        ] {
            let r = call(d, name, &args);
            assert_eq!(status(&r), "refused", "{name}: {r}");
            assert!(first_text(&r).contains("waits for the person to press Start"), "{r}");
        }
        assert!(d.confirms.card().is_none(), "a refusal, not a card");
        assert!(done(&log).is_empty(), "nothing acted before Start");
        // Start: the waiting call hears it, with the steps numbered.
        d.start_plan(p.id, true).unwrap();
        let r = waiting.join().unwrap();
        assert_eq!((status(&r), r["isError"].clone()), ("started".to_string(), json!(false)), "{r}");
        let text = first_text(&r);
        assert!(text.contains("1. Make the text bold\n2. Write the subject") && text.contains("Mail only") && text.contains("pointcast_plan_step"), "{text}");
        assert!(d.start_plan(p.id, true).is_err(), "started once");
        // In the plan's app it acts; anywhere else it is refused, with the reason.
        assert_eq!(status(&call(d, "pointcast_click", &json!({ "id": id_of(d, "Bold") }))), "done");
        assert_eq!(status(&call(d, "pointcast_open_app", &json!({ "name": "mail.app" }))), "done");
        let r = call(d, "pointcast_open_app", &json!({ "name": "Notes" }));
        assert_eq!(status(&r), "refused");
        assert!(first_text(&r).contains("Notes is not in the plan the person started, which acts in Mail only"), "{r}");
        assert_eq!(done(&log), ["press Bold", "open mail.app"]);
        // The tiers and the cards still apply on top: Mail is at click, so typing asks to allow it.
        let subject = id_of(d, "Subject");
        let typing = std::thread::spawn(move || call(d, "pointcast_type", &json!({ "id": subject, "text": "hi" })));
        let t = std::time::Instant::now();
        let c = loop {
            if let Some(c) = d.confirms.card() {
                break c;
            }
            assert!(t.elapsed() < Duration::from_secs(10), "no card");
            std::thread::sleep(Duration::from_millis(5));
        };
        assert_eq!(c.raise, Some("full"));
        d.answer(c.id, Answer::Cancel).unwrap();
        assert_eq!(status(&typing.join().unwrap()), "declined");
        assert_eq!(done(&log).len(), 2);
    }

    #[test]
    fn a_plan_started_for_another_app_refuses_acting_in_the_front_one() {
        let (d, log) = leaked(mail());
        d.set_trust(Trust::with([("Mail".to_string(), Tier::Full)]));
        d.ask_for(Duration::from_secs(10));
        let _g = d.grants.grant(Duration::from_secs(60));
        assert_eq!(status(&started(d, &["TextEdit"])), "started");
        let r = call(d, "pointcast_click", &json!({ "id": id_of(d, "Bold") }));
        assert!(status(&r) == "refused" && first_text(&r).contains("Mail is not in the plan"), "{r}");
        let r = call(d, "pointcast_click", &json!({ "x": 5, "y": 5 }));
        assert!(status(&r) == "refused" && first_text(&r).contains("acts in TextEdit only"), "a point in Mail too: {r}");
        assert!(done(&log).is_empty());
    }

    #[test]
    fn with_no_plan_a_single_action_goes_as_before_and_a_plan_names_no_never_app() {
        let (d, log) = leaked(mail());
        let _g = d.grants.grant(Duration::from_secs(60));
        assert_eq!(status(&call(d, "pointcast_open_app", &json!({ "name": "Notes" }))), "done", "no plan asked for, none required");
        assert_eq!(done(&log), ["open Notes"]);
        for (args, why) in [
            (json!({ "steps": [], "apps": ["Notes"] }), "1 to 12 steps"),
            (json!({ "steps": vec!["a"; 13], "apps": ["Notes"] }), "1 to 12 steps"),
            (json!({ "steps": ["Open it"] }), "1 to 6 apps"),
            (json!({ "steps": ["Open it"], "apps": ["/Applications/Notes.app"] }), "not a path"),
            (json!({ "steps": ["Open it"], "apps": ["Pointcast"] }), "off limits"),
            (json!({ "steps": ["Open it"], "apps": ["SecurityAgent"] }), "password or consent prompt"),
        ] {
            let r = call(d, "pointcast_plan", &args);
            assert!(status(&r) == "refused" && first_text(&r).contains(why), "{args}: {r}");
        }
        assert!(d.status().plan.is_none(), "a refused plan is never shown");
        // Nobody to press Start (no panel): nothing acts after the plan.
        let r = call(d, "pointcast_plan", &json!({ "steps": ["Open it", "Type"], "apps": ["Notes"] }));
        assert!(status(&r) == "refused" && first_text(&r).contains("Nobody can press Start"), "{r}");
        assert_eq!(status(&call(d, "pointcast_open_app", &json!({ "name": "Notes" }))), "refused");
        assert_eq!(done(&log), ["open Notes"]);
    }

    #[test]
    fn cancel_or_no_answer_refuses_every_tool_for_the_rest_of_the_turn() {
        let (d, log) = leaked(mail());
        d.set_trust(Trust::with([("Mail".to_string(), Tier::Full)]));
        d.ask_for(Duration::from_secs(10));
        let _g = d.grants.grant(Duration::from_secs(60));
        let bold = id_of(d, "Bold");
        let waiting = propose(d, &["Bold it"], &["Mail"]);
        let p = shown(d, State::Proposed);
        d.start_plan(p.id, false).unwrap();
        let r = waiting.join().unwrap();
        assert_eq!((status(&r), r["isError"].clone()), ("declined".to_string(), json!(false)), "{r}");
        assert!(first_text(&r).contains("The person cancelled the plan"), "{r}");
        assert_eq!(d.status().plan.map(|p| p.state), Some(State::Cancelled));
        assert!(d.start_plan(p.id, true).is_err(), "no Start after Cancel");
        for name in NAMES {
            let r = call(d, name, &json!({ "id": bold, "text": "hi", "keys": "a", "direction": "up", "name": "Mail", "app": "Mail", "steps": ["x"], "apps": ["Mail"], "index": 1, "status": "done" }));
            assert!(status(&r) == "refused" && first_text(&r).contains(CANCELLED), "{name}: {r}");
        }
        assert!(done(&log).is_empty());
        // No answer in time counts the same.
        let (d, log) = leaked(mail());
        d.set_trust(Trust::with([("Mail".to_string(), Tier::Full)]));
        d.ask_for(Duration::from_millis(60));
        let _g = d.grants.grant(Duration::from_secs(60));
        let bold = id_of(d, "Bold");
        let r = call(d, "pointcast_plan", &json!({ "steps": ["Bold it"], "apps": ["Mail"] }));
        assert!(status(&r) == "refused" && first_text(&r).contains("did not press Start"), "{r}");
        assert_eq!(status(&call(d, "pointcast_click", &json!({ "id": bold }))), "refused");
        assert!(done(&log).is_empty());
    }

    #[test]
    fn a_new_turn_drops_the_plan_with_the_grant() {
        let (d, log) = leaked(mail());
        d.set_trust(Trust::with([("Mail".to_string(), Tier::Full)]));
        d.ask_for(Duration::from_secs(10));
        // Turn A's plan waits; turn B starts: A's call ends at once and its plan is gone.
        let a = d.begin(Duration::from_secs(60));
        let waiting = propose(d, &["Bold it"], &["Mail"]);
        let p = shown(d, State::Proposed);
        let t = std::time::Instant::now();
        let b = d.begin(Duration::from_secs(60));
        let r = waiting.join().unwrap();
        assert!(t.elapsed() < Duration::from_secs(2) && status(&r) == "refused" && first_text(&r).contains("turn ended"), "{r}");
        assert!(d.status().plan.is_none(), "the panel shows no plan of an earlier turn");
        assert!(d.start_plan(p.id, true).is_err(), "nor takes a Start for it");
        // B has no plan of its own: it acts as before. A letting go late leaves B alone.
        drop(a);
        assert_eq!(status(&call(d, "pointcast_click", &json!({ "id": id_of(d, "Bold") }))), "done");
        // B's plan, cancelled, refuses everything in B; the next turn C starts clean.
        let waiting = propose(d, &["Bold it"], &["Mail"]);
        d.start_plan(shown(d, State::Proposed).id, false).unwrap();
        waiting.join().unwrap();
        assert_eq!(status(&call(d, "pointcast_apps", &json!({}))), "refused");
        drop(b);
        assert!(d.status().plan.is_none(), "it goes with B's grant");
        let c = d.begin(Duration::from_secs(60));
        assert!(d.status().plan.is_none());
        assert_eq!(status(&call(d, "pointcast_click", &json!({ "id": id_of(d, "Bold") }))), "done");
        // A started plan goes with the stop key too.
        assert_eq!(status(&started(d, &["Mail"])), "started");
        d.halt();
        assert!(d.status().plan.is_none());
        drop(c);
        assert_eq!(done(&log), ["press Bold", "press Bold"]);
    }

    #[test]
    fn step_ticks_reach_the_panel_state() {
        let (d, _log) = leaked(mail());
        let seen: &'static std::sync::Mutex<Vec<crate::computer::Status>> = Box::leak(Box::default());
        d.set_hooks(Hooks { status: Box::new(move |s| seen.lock().unwrap().push(s.clone())), stop: Box::new(|| ()) });
        let _g = d.grants.grant(Duration::from_secs(60));
        let r = call(d, "pointcast_plan_step", &json!({ "index": 1, "status": "done" }));
        assert!(status(&r) == "refused" && first_text(&r).contains("No started plan"), "{r}");
        assert_eq!(status(&started(d, &["TextEdit"])), "started");
        let last = || seen.lock().unwrap().last().cloned().and_then(|s| s.plan).expect("the panel heard a plan");
        assert_eq!(last().state, State::Started);
        let r = call(d, "pointcast_plan_step", &json!({ "index": 1, "status": "done", "note": "  TextEdit is open " }));
        assert!(status(&r) == "done" && first_text(&r).contains("Step 1 of 3 is ticked done."), "{r}");
        assert_eq!((last().steps[0].status, last().steps[0].note.as_deref()), (Mark::Done, Some("TextEdit is open")));
        call(d, "pointcast_plan_step", &json!({ "index": 2, "status": "skipped" }));
        let r = call(d, "pointcast_plan_step", &json!({ "index": 3, "status": "failed", "note": "no text to select" }));
        assert!(first_text(&r).contains("Stop and ask the person"), "{r}");
        // What the panel gets, as it reads it.
        let panel = serde_json::to_value(seen.lock().unwrap().last().unwrap()).unwrap();
        assert_eq!(panel["plan"]["state"], "started");
        assert_eq!(panel["plan"]["apps"], json!(["TextEdit"]));
        let marks: Vec<_> = panel["plan"]["steps"].as_array().unwrap().iter().map(|s| s["status"].as_str().unwrap().to_string()).collect();
        assert_eq!(marks, ["done", "skipped", "failed"]);
        assert_eq!(panel["plan"]["steps"][2]["note"], "no text to select");
        // A step ticked again keeps the last word; numbers outside the plan and unknown words are refused.
        let r = call(d, "pointcast_plan_step", &json!({ "index": 3, "status": "done" }));
        assert!(first_text(&r).contains("Every step is ticked"), "{r}");
        assert_eq!(last().steps[2].status, Mark::Done);
        for args in [json!({ "index": 0, "status": "done" }), json!({ "index": 4, "status": "done" }), json!({ "index": 1, "status": "maybe" })] {
            assert_eq!(status(&call(d, "pointcast_plan_step", &args)), "refused", "{args}");
        }
        let ticks: Vec<_> = d.journal.entries().into_iter().filter(|e| e.tool == "pointcast_plan_step" && matches!(e.outcome, Outcome::Done(_))).collect();
        assert_eq!(ticks.len(), 4, "each tick is in the journal");
        assert!(ticks.iter().all(|e| e.line().is_none()), "and none is listed under the answer as an action");
    }
}

/// Mail has the keyboard focus as Accessibility tells it, but Claude is really in front, as when macOS ignored a
/// request to bring Mail forward. Mail is trusted with keys; Claude is not.
fn mail_behind(stuck: bool) -> (Desk, Did) {
    let f = mail();
    *f.frontmost.lock().unwrap() = Some(crate::computer::fake::app("Claude", 5150));
    let (d, log) = desk(Fake { stuck, ..f });
    d.set_trust(Trust::with([("Mail".to_string(), Tier::Full)]));
    (d, log)
}

#[test]
fn keys_and_typing_go_nowhere_while_the_cleared_app_is_not_really_in_front() {
    let (d, log) = mail_behind(true);
    let _g = d.grants.grant(Duration::from_secs(60));
    let r = call(&d, "pointcast_key", &json!({ "keys": "cmd+n" }));
    assert_eq!(status(&r), "refused", "{r}");
    assert!(first_text(&r).contains("Mail isn't in front, so nothing was typed."), "{r}");
    let r = call(&d, "pointcast_type", &json!({ "text": "hello" }));
    assert_eq!(status(&r), "refused", "{r}");
    assert!(first_text(&r).contains("Mail isn't in front, so nothing was typed."), "{r}");
    // By id, typed synthetically: refused too, before the click that would focus the field.
    let mut f = mail();
    f.tree.nodes[2].actions.clear();
    *f.frontmost.lock().unwrap() = Some(crate::computer::fake::app("Claude", 5150));
    let (d2, log2) = desk(f);
    d2.set_trust(Trust::with([("Mail".to_string(), Tier::Full)]));
    let _g2 = d2.grants.grant(Duration::from_secs(60));
    let r = call(&d2, "pointcast_type", &json!({ "id": id_of(&d2, "Subject"), "text": "hi" }));
    assert_eq!(status(&r), "refused", "{r}");
    // Setting a value by accessibility lands on that element, wherever the front is.
    let body = id_of(&d2, "AXTextArea");
    assert_eq!(status(&call(&d2, "pointcast_type", &json!({ "id": body, "text": "hi" }))), "done");
    assert_eq!(done(&log), Vec::<String>::new(), "nothing sent");
    assert_eq!(done(&log2), ["set =hi"], "only the value set");
    let refused = d.journal.entries().iter().filter(|e| matches!(&e.outcome, Outcome::Refused(w) if w.contains("isn't in front"))).count();
    assert_eq!(refused, 2, "each refusal is logged");
}

#[test]
fn a_save_panel_on_the_front_apps_window_counts_as_that_app() {
    let mut f = mail();
    let panel = crate::computer::fake::app("Open and Save Panel Service", 6060);
    f.focused = Some((panel.clone(), crate::computer::fake::node(&[0], "AXTextField", "text field", "Save As:", &["set value"])));
    f.panels = vec![(6060, 4242)];
    let (d, log) = desk(f);
    d.set_trust(Trust::with([("Mail".to_string(), Tier::Full)]));
    let _g = d.grants.grant(Duration::from_secs(60));
    assert_eq!(status(&call(&d, "pointcast_type", &json!({ "text": "notes" }))), "done");
    assert_eq!(status(&call(&d, "pointcast_key", &json!({ "keys": "cmd+a" }))), "done");
    assert_eq!(done(&log), ["type notes", "key [Cmd]+a"]);
    // The same panel on another app's window, or no panel known: refused.
    let mut f = mail();
    f.focused = Some((panel, crate::computer::fake::node(&[0], "AXTextField", "text field", "Save As:", &["set value"])));
    f.panels = vec![(6060, 77)];
    let (d, log) = desk(f);
    d.set_trust(Trust::with([("Mail".to_string(), Tier::Full)]));
    let _g = d.grants.grant(Duration::from_secs(60));
    assert_eq!(status(&call(&d, "pointcast_type", &json!({ "text": "notes" }))), "refused");
    assert_eq!(done(&log), Vec::<String>::new());
}

#[test]
fn focus_and_open_say_failed_when_the_app_never_comes_to_the_front() {
    let (d, log) = mail_behind(true);
    let _g = d.grants.grant(Duration::from_secs(60));
    let r = call(&d, "pointcast_focus", &json!({ "app": "Mail" }));
    assert_eq!(status(&r), "failed", "{r}");
    assert!(first_text(&r).contains("Mail didn't come to the front."), "{r}");
    let r = call(&d, "pointcast_open_app", &json!({ "name": "Mail" }));
    assert_eq!(status(&r), "failed", "{r}");
    assert!(first_text(&r).contains("Mail didn't come to the front."), "{r}");
    assert_eq!(done(&log), ["focus Mail None", "open Mail", "focus Mail None"], "asked, never claimed");
}

#[test]
fn focus_says_done_when_the_app_really_comes_forward_and_keys_then_go() {
    let (d, log) = mail_behind(false);
    let _g = d.grants.grant(Duration::from_secs(60));
    assert_eq!(status(&call(&d, "pointcast_key", &json!({ "keys": "cmd+n" }))), "refused");
    let r = call(&d, "pointcast_focus", &json!({ "app": "Mail" }));
    assert_eq!(status(&r), "done", "{r}");
    assert_eq!(status(&call(&d, "pointcast_key", &json!({ "keys": "cmd+n" }))), "done");
    assert_eq!(done(&log), ["focus Mail None", "key [Cmd]+n"]);
}

#[test]
fn file_paths_resolve_only_inside_home_and_away_from_kept_places() {
    let home = std::env::temp_dir().join(format!("pc-files-home-{}", std::process::id()));
    let data = home.join("appdata");
    std::fs::create_dir_all(data.join("sub")).unwrap();
    std::fs::create_dir_all(home.join("dev/repo/.git")).unwrap();
    let ok = |raw: &str| under_home(raw, &home, &data).map(|p| p.canonicalize().unwrap_or(p));
    let want = home.canonicalize().unwrap();
    assert_eq!(ok("~").unwrap(), want, "tilde is home");
    assert_eq!(ok(&format!("{}/dev/repo", home.display())).unwrap(), want.join("dev/repo"));
    assert_eq!(ok(&format!("~/dev/../dev", )).unwrap(), want.join("dev"), "a dot-dot inside home stays inside home");
    for raw in [
        "relative/path",
        "/etc/hosts",
        "/usr/bin",
        &format!("{}/.ssh/id_rsa", home.display()),
        &format!("{}/.config/gh/hosts.yml", home.display()),
        &format!("{}/.env.local", home.display()),
        &format!("{}/dev/repo/.git/HEAD", home.display()),
        &format!("{}/appdata/sub", home.display()),
        &format!("{}/dev/../..", home.display()),
    ] {
        assert!(under_home(raw, &home, &data).is_err(), "{raw} must be refused");
    }
    // A path not there yet still resolves for mkdir: the existing parent is what is checked.
    assert!(under_home(&format!("{}/dev/new-folder", home.display()), &home, &data).is_ok());
    std::fs::remove_dir_all(&home).ok();
}

#[test]
fn file_ops_list_move_and_mkdir_work_on_the_real_filesystem() {
    let root = std::env::temp_dir().join(format!("pc-files-ops-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let nested = root.join("a/b");
    let r = files_op("mkdir", &nested, None, 10).unwrap();
    assert!(matches!(r, End::Done { .. }) && nested.is_dir());
    let f = root.join("a/f.txt");
    std::fs::write(&f, "hi").unwrap();
    let End::Done { text, data, .. } = files_op("list", &root.join("a"), None, 10).unwrap() else { panic!() };
    assert!(text.contains("b/") && text.contains("f.txt"), "{text}");
    assert_eq!(data["count"], 2);
    // Move to a new name, then into a folder that exists.
    let moved = root.join("a/moved.txt");
    assert!(files_op("move", &f, Some(&moved), 10).is_ok() && moved.exists() && !f.exists());
    assert!(files_op("move", &moved, Some(&nested), 10).is_ok() && nested.join("moved.txt").exists());
    // Honest failures.
    for (action, p, to) in [
        ("list", root.join("missing").as_path(), None),
        ("move", nested.as_path(), Some(nested.join("inside").as_path())),
        ("move", nested.join("moved.txt").as_path(), Some(nested.join("moved.txt").as_path())),
        ("mkdir", nested.join("moved.txt").as_path(), None),
        ("wonder", root.as_path(), None),
    ] {
        let p = p.to_path_buf();
        assert!(files_op(action, &p, to.as_deref(), 10).is_err(), "{action} {p:?}");
    }
    // A real trashing: the file leaves the folder.
    let gone = root.join("bye.txt");
    std::fs::write(&gone, "x").unwrap();
    assert!(files_op("trash", &gone, None, 10).is_ok() && !gone.exists());
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn file_acts_go_through_the_guard_and_trashing_waits_for_the_person() {
    let (d, _log) = desk(mail());
    let _g = d.grants.grant(Duration::from_secs(60));
    d.set_trust(Trust::with([("Finder".to_string(), Tier::Full)]));
    // Outside home is refused before the OS is asked, whatever the tier.
    let r = call(&d, "pointcast_files", &json!({ "action": "list", "path": "/etc" }));
    assert_eq!(status(&r), "refused");
    // Listing home itself is a look, allowed at any tier.
    let r = call(&d, "pointcast_files", &json!({ "action": "list", "path": "~" }));
    assert_eq!(status(&r), "done", "{r}");
    // "Move x to the Trash" reads as can't-be-undone: with no one to ask in tests, it waits.
    let r = call(&d, "pointcast_files", &json!({ "action": "trash", "path": "~/pointcast-test-missing-file" }));
    assert_eq!(status(&r), "needs_confirm", "{r}");
    // A view-tier Finder may be listed but not moved in.
    d.set_trust(Trust::with([("Finder".to_string(), Tier::View)]));
    assert_eq!(status(&call(&d, "pointcast_files", &json!({ "action": "move", "path": "~/x", "to": "~/y" }))), "refused");
    assert_eq!(status(&call(&d, "pointcast_files", &json!({ "action": "list", "path": "~" }))), "done");
}

#[test]
fn a_browser_is_known_or_refused_and_the_script_carries_no_code() {
    use super::{browser_kind, browser_script};
    assert_eq!(browser_kind("Safari"), Some(false));
    assert_eq!(browser_kind("Google Chrome"), Some(true));
    assert_eq!(browser_kind("Arc"), Some(true));
    assert_eq!(browser_kind("Firefox"), None);
    assert_eq!(browser_kind("Finder"), None);
    // The front-tab script names the app, asks for one tab's title and address.
    let s = browser_script("Safari", false, false);
    assert!(s.contains("Application(\"Safari\")") && s.contains("currentTab") && s.contains("t.url()"), "{s}");
    // The Chrome family asks its own way, and the tabs script lists every window's tabs, capped.
    let s = browser_script("Google Chrome", true, true);
    assert!(s.contains("t.title()") && s.contains("slice(0, 50)"), "{s}");
    // An app name that is itself JavaScript can carry nothing: it lands as a quoted string.
    let s = browser_script("\"; evil()", false, false);
    assert!(s.contains("Application(\"\\\"; evil()\")"), "{s}");
}
