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
