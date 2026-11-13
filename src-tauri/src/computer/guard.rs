//! What the AI may do where, decided before anything happens. Pure: no OS calls, so every rule is unit-tested.
//!
//! Three questions, in this order. Is the target on the never-list (a password field, an OS password prompt, an
//! elevated app, Pointcast itself)? Does the app's trust tier allow this kind of action? Does the target read as
//! something that can't be undone (send, pay, delete...)? The first refuses, the second refuses, the third pauses
//! for the person.
use std::collections::HashMap;

/// How far the AI may go in one app: see it, click and scroll in it, or type and press keys in it too.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Tier {
    View,
    Click,
    Full,
}

impl Tier {
    pub fn word(self) -> &'static str {
        match self {
            Tier::View => "view",
            Tier::Click => "click",
            Tier::Full => "full",
        }
    }
}

/// The kind of thing a tool is about to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Read the screen or the accessibility tree.
    See,
    /// Bring an app or window to the front, or open one.
    Focus,
    Click,
    Scroll,
    Type,
    Key,
}

impl Kind {
    /// The lowest tier that allows it.
    pub fn needs(self) -> Tier {
        match self {
            Kind::See | Kind::Focus => Tier::View,
            Kind::Click | Kind::Scroll => Tier::Click,
            Kind::Type | Kind::Key => Tier::Full,
        }
    }

    fn verb(self) -> &'static str {
        match self {
            Kind::See => "read",
            Kind::Focus => "bring forward",
            Kind::Click => "click in",
            Kind::Scroll => "scroll in",
            Kind::Type => "type in",
            Kind::Key => "press keys in",
        }
    }
}

/// What an action lands on, as the accessibility tree describes it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Target {
    /// The app's name as the OS gives it ("TextEdit", "chrome.exe").
    pub app: String,
    /// The title of the window it is in.
    pub window: String,
    /// The OS's own role and subrole ("AXButton", "AXSecureTextField"; a UIA control type).
    pub raw_role: String,
    pub subrole: String,
    /// The plain word for it ("button", "text field").
    pub role: String,
    pub name: String,
    /// A password field: AXSecureTextField on macOS, IsPassword in UI Automation.
    pub secure: bool,
    /// One of Pointcast's own windows.
    pub own: bool,
    /// A Windows app running as administrator, which a normal app may not drive.
    pub elevated: bool,
}

/// What the guard says about one action.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    Allow,
    /// Never, with the reason the AI is told.
    Refuse(String),
    /// Only after the person says so, with what is about to happen.
    NeedsConfirm(String),
}

/// Apps that start at view. Matched on the app's name, any case, without ".exe" or ".app", from its start.
pub const BROWSERS: &[&str] = &[
    "safari", "google chrome", "chrome", "chromium", "firefox", "microsoft edge", "msedge", "arc", "brave browser",
    "brave", "opera", "vivaldi", "orion", "zen", "duckduckgo", "tor browser", "iexplore", "internet explorer",
];
pub const TERMINALS: &[&str] = &[
    "terminal", "iterm", "warp", "alacritty", "kitty", "wezterm", "hyper", "ghostty", "tabby", "windowsterminal",
    "windows terminal", "cmd", "command prompt", "powershell", "pwsh", "conhost", "windows powershell",
];
pub const SYSTEM_SETTINGS: &[&str] =
    &["system settings", "system preferences", "control panel", "systemsettings", "settings", "registry editor", "regedit"];
pub const PASSWORD_MANAGERS: &[&str] = &[
    "1password", "bitwarden", "keepass", "dashlane", "lastpass", "keeper", "nordpass", "enpass", "roboform",
    "proton pass", "keychain access", "passwords", "credential manager",
];
/// The OS's own password and consent prompts: never driven, whatever the tier.
pub const PASSWORD_PROMPTS: &[&str] = &[
    "securityagent", "coreautha", "coreauthd", "authorizationhost", "loginwindow", "consent", "credentialuibroker",
    "logonui", "windows security", "credential dialog xaml host", "userauthenticationagent",
];

/// Words that mark a target as something that can't be undone. Whole words of its name or role, any case.
pub const IRREVERSIBLE: &[&str] = &[
    "send", "submit", "buy", "pay", "purchase", "order", "checkout", "delete", "remove", "erase", "trash", "discard",
    "destroy", "wipe", "post", "publish", "share", "accept", "agree", "confirm", "approve", "transfer", "withdraw",
    "deposit", "donate", "subscribe", "unsubscribe", "install", "uninstall", "sign", "tweet",
];
/// Phrases that do the same when the words alone would not.
const IRREVERSIBLE_PHRASES: &[&str] = &["check out", "move to bin", "empty bin", "place order", "log out", "sign out"];
/// A plain yes on a button means whatever its window asks: then the window's title is read for the words above.
const AFFIRMATIVE: &[&str] = &["ok", "okay", "yes", "continue", "allow", "done", "apply", "proceed"];

/// The app's name lowered, without ".exe" or ".app" and the padding round it.
pub fn app_key(app: &str) -> String {
    let a = app.trim().to_lowercase();
    let a = a.strip_suffix(".exe").or_else(|| a.strip_suffix(".app")).unwrap_or(&a);
    a.trim().to_string()
}

/// Whether `app` is one of `list`: equal to an entry, or starting with one followed by a space or a digit
/// ("iTerm2", "Firefox Developer Edition"). "Arc" does not take in "Archive Utility".
fn listed(app: &str, list: &[&str]) -> bool {
    let a = app_key(app);
    list.iter().any(|e| {
        a == *e || a.strip_prefix(e).is_some_and(|rest| rest.starts_with(|c: char| c == ' ' || c.is_ascii_digit() || c == 'x'))
    })
}

/// Whether the app is an OS password or consent prompt.
pub fn password_prompt(app: &str) -> bool {
    listed(app, PASSWORD_PROMPTS)
}

/// The tier each app gets, from its kind and the person's own choices.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Trust {
    /// The person's tier for an app, keyed by `app_key`. Wins over the default, up or down.
    overrides: HashMap<String, Tier>,
}

impl Trust {
    /// Tiers the person chose, by app name.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn with(overrides: impl IntoIterator<Item = (String, Tier)>) -> Trust {
        Trust { overrides: overrides.into_iter().map(|(a, t)| (app_key(&a), t)).collect() }
    }

    /// Where an app starts before the person changes it.
    pub fn default_tier(app: &str) -> Tier {
        let view = [BROWSERS, TERMINALS, SYSTEM_SETTINGS, PASSWORD_MANAGERS];
        if view.iter().any(|list| listed(app, list)) {
            Tier::View
        } else {
            Tier::Click
        }
    }

    pub fn tier(&self, app: &str) -> Tier {
        self.overrides.get(&app_key(app)).copied().unwrap_or_else(|| Self::default_tier(app))
    }
}

/// The lowercase words of `s`, split on anything that is not a letter or a digit.
fn words(s: &str) -> Vec<String> {
    s.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).map(str::to_lowercase).collect()
}

/// The irreversible word or phrase `s` reads as, if any.
pub fn irreversible(s: &str) -> Option<&'static str> {
    let ws = words(s);
    let joined = format!(" {} ", ws.join(" "));
    IRREVERSIBLE
        .iter()
        .find(|w| ws.iter().any(|x| x == *w))
        .or_else(|| IRREVERSIBLE_PHRASES.iter().find(|p| joined.contains(&format!(" {p} "))))
        .copied()
}

/// What makes this target irreversible: its name or role, or a plain yes in a window whose title is.
fn reads_irreversible(t: &Target) -> Option<String> {
    if let Some(w) = irreversible(&t.name).or_else(|| irreversible(&t.role)) {
        return Some(w.to_string());
    }
    let yes = words(&t.name).join(" ");
    if AFFIRMATIVE.contains(&yes.as_str()) {
        return irreversible(&t.window).map(String::from);
    }
    None
}

/// How a target reads in a message: `button "Send"`.
pub fn describe(t: &Target) -> String {
    let role = if t.role.is_empty() { "element" } else { t.role.as_str() };
    if t.name.is_empty() {
        format!("{role} in {}", t.app)
    } else {
        format!("{role} \"{}\" in {}", t.name, t.app)
    }
}

/// The verdict on doing `kind` to `t` with the person's `trust`. `commits` says the action itself sends or submits
/// whatever its target (Return, Cmd+Delete).
pub fn check(trust: &Trust, kind: Kind, t: &Target, commits: bool) -> Verdict {
    if t.own || app_key(&t.app) == "pointcast" {
        return Verdict::Refuse("Pointcast's own windows are off limits.".into());
    }
    if password_prompt(&t.app) {
        return Verdict::Refuse(format!("{} is a password or consent prompt: the person answers it themselves.", t.app));
    }
    if t.elevated {
        return Verdict::Refuse(format!("{} runs as administrator: Pointcast does not drive it.", t.app));
    }
    let secure = t.secure || t.raw_role == "AXSecureTextField" || t.subrole == "AXSecureTextField";
    if secure && kind != Kind::See && kind != Kind::Focus {
        return Verdict::Refuse("That is a password field: the person types passwords themselves.".into());
    }
    let tier = trust.tier(&t.app);
    if tier < kind.needs() {
        return Verdict::Refuse(format!(
            "{} is at \"{}\" in Pointcast's trust settings, which does not let the AI {} it. The person can raise it.",
            t.app,
            tier.word(),
            kind.verb()
        ));
    }
    if matches!(kind, Kind::Click | Kind::Type | Kind::Key) {
        if let Some(w) = reads_irreversible(t) {
            return Verdict::NeedsConfirm(format!("This would {} {} (it reads as \"{w}\").", kind.verb(), describe(t)));
        }
        if commits {
            return Verdict::NeedsConfirm(format!("This key can send or submit what is in {}.", describe(t)));
        }
    }
    Verdict::Allow
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(app: &str, role: &str, name: &str) -> Target {
        Target { app: app.into(), role: role.into(), name: name.into(), ..Default::default() }
    }

    fn refused(v: &Verdict) -> bool {
        matches!(v, Verdict::Refuse(_))
    }

    #[test]
    fn browsers_terminals_settings_and_password_managers_start_at_view_and_the_rest_at_click() {
        for app in [
            "Safari", "Google Chrome", "Google Chrome Canary", "chrome.exe", "Firefox", "Firefox Developer Edition",
            "Microsoft Edge", "msedge.exe", "Arc", "Brave Browser", "Terminal", "iTerm2", "Warp", "WindowsTerminal.exe",
            "cmd.exe", "powershell.exe", "System Settings", "System Preferences", "Control Panel", "SystemSettings.exe",
            "1Password 7", "Bitwarden", "KeePassXC", "Keychain Access", "Passwords",
        ] {
            assert_eq!(Trust::default_tier(app), Tier::View, "{app}");
        }
        for app in ["TextEdit", "Notes", "notepad.exe", "Finder", "Archive Utility", "Mail", "Slack", "Calculator", "Terminology Notes"] {
            assert_eq!(Trust::default_tier(app), Tier::Click, "{app}");
        }
    }

    #[test]
    fn the_persons_choice_wins_over_the_default_up_or_down() {
        let trust = Trust::with([("Notes".to_string(), Tier::Full), ("Safari".into(), Tier::Click), ("Mail.app".into(), Tier::View)]);
        assert_eq!(trust.tier("notes"), Tier::Full);
        assert_eq!(trust.tier("Safari"), Tier::Click);
        assert_eq!(trust.tier("Mail"), Tier::View);
        assert_eq!(trust.tier("TextEdit"), Tier::Click, "everything else keeps its default");
        assert_eq!(trust.tier("Terminal"), Tier::View);
    }

    #[test]
    fn each_tier_allows_exactly_its_kinds() {
        let all = [Kind::See, Kind::Focus, Kind::Click, Kind::Scroll, Kind::Type, Kind::Key];
        for (tier, allowed) in [
            (Tier::View, &[Kind::See, Kind::Focus][..]),
            (Tier::Click, &[Kind::See, Kind::Focus, Kind::Click, Kind::Scroll][..]),
            (Tier::Full, &all[..]),
        ] {
            let trust = Trust::with([("Notes".to_string(), tier)]);
            for kind in all {
                let v = check(&trust, kind, &target("Notes", "text field", "Body"), false);
                assert_eq!(v == Verdict::Allow, allowed.contains(&kind), "{tier:?} {kind:?}: {v:?}");
                if !allowed.contains(&kind) {
                    let Verdict::Refuse(why) = v else { panic!() };
                    assert!(why.contains(&format!("\"{}\"", tier.word())) && why.contains("Notes"), "{why}");
                }
            }
        }
    }

    #[test]
    fn the_never_list_refuses_whatever_the_tier() {
        let full = Trust::with(
            ["TextEdit", "SecurityAgent", "consent.exe", "CredentialUIBroker.exe", "Pointcast", "regedit.exe"].map(|a| (a.to_string(), Tier::Full)),
        );
        let secure_mac = Target { subrole: "AXSecureTextField".into(), raw_role: "AXTextField".into(), ..target("TextEdit", "text field", "Password") };
        let secure_raw = Target { raw_role: "AXSecureTextField".into(), ..target("TextEdit", "text field", "") };
        let secure_uia = Target { secure: true, ..target("TextEdit", "text field", "PIN") };
        for t in [&secure_mac, &secure_raw, &secure_uia] {
            for kind in [Kind::Click, Kind::Type, Kind::Key, Kind::Scroll] {
                assert!(refused(&check(&full, kind, t, false)), "{t:?} {kind:?}");
            }
            assert_eq!(check(&full, Kind::See, t, false), Verdict::Allow, "a password field is listed, never its value");
        }
        for app in ["SecurityAgent", "coreautha", "consent.exe", "CredentialUIBroker.exe", "LogonUI.exe", "Windows Security"] {
            for kind in [Kind::See, Kind::Focus, Kind::Click, Kind::Type] {
                assert!(refused(&check(&full, kind, &target(app, "button", "Allow"), false)), "{app} {kind:?}");
            }
        }
        let own = Target { own: true, ..target("TextEdit", "button", "Ask") };
        assert!(refused(&check(&full, Kind::See, &own, false)), "a window of this process");
        assert!(refused(&check(&full, Kind::Click, &target("Pointcast", "button", "Ask"), false)), "by name too");
        let admin = Target { elevated: true, ..target("regedit.exe", "button", "Open") };
        assert!(refused(&check(&full, Kind::Click, &admin, false)));
    }

    #[test]
    fn the_irreversible_words_are_this_set() {
        let mut set: Vec<&str> = IRREVERSIBLE.to_vec();
        set.sort();
        assert_eq!(
            set,
            [
                "accept", "agree", "approve", "buy", "checkout", "confirm", "delete", "deposit", "destroy", "discard", "donate",
                "erase", "install", "order", "pay", "post", "publish", "purchase", "remove", "send", "share",
                "sign", "submit", "subscribe", "transfer", "trash", "tweet", "uninstall", "unsubscribe", "wipe", "withdraw",
            ]
        );
        for w in ["send", "submit", "buy", "pay", "purchase", "order", "delete", "remove", "post", "publish", "accept", "confirm", "transfer"] {
            assert!(IRREVERSIBLE.contains(&w), "the brief names {w}");
        }
    }

    #[test]
    fn a_target_that_reads_irreversible_waits_for_the_person() {
        let full = Trust::with([("Mail".to_string(), Tier::Full)]);
        for name in ["Send", "Send Message", "Place Order", "Pay now", "Delete", "Move to Bin", "Empty Bin", "Publish post", "I agree", "Check out", "Buy with Apple Pay", "CONFIRM TRANSFER"] {
            for kind in [Kind::Click, Kind::Type, Kind::Key] {
                let v = check(&full, kind, &target("Mail", "button", name), false);
                let Verdict::NeedsConfirm(what) = &v else { panic!("{name} {kind:?}: {v:?}") };
                assert!(what.contains(name), "{what}");
            }
            assert_eq!(check(&full, Kind::Scroll, &target("Mail", "button", name), false), Verdict::Allow, "scrolling past it is fine");
            assert_eq!(check(&full, Kind::See, &target("Mail", "button", name), false), Verdict::Allow);
        }
        for name in ["Sender", "Ordered list", "Reorder", "Posted by", "Paste", "Save", "Bold", "New Document", "Sending options", ""] {
            assert_eq!(check(&full, Kind::Click, &target("Mail", "button", name), false), Verdict::Allow, "{name}");
        }
        // A plain yes reads its window's title.
        let ok = Target { window: "Delete 3 items?".into(), ..target("Mail", "button", "OK") };
        assert!(matches!(check(&full, Kind::Click, &ok, false), Verdict::NeedsConfirm(_)));
        let ok = Target { window: "Font".into(), ..target("Mail", "button", "OK") };
        assert_eq!(check(&full, Kind::Click, &ok, false), Verdict::Allow);
        // A key that sends whatever it lands on.
        assert!(matches!(check(&full, Kind::Key, &target("Mail", "text field", "Body"), true), Verdict::NeedsConfirm(_)));
        // The tier is asked first: a view app is refused, not offered.
        assert!(refused(&check(&Trust::default(), Kind::Click, &target("Safari", "button", "Buy"), false)));
    }

    #[test]
    fn app_names_are_compared_without_case_or_suffix() {
        assert_eq!(app_key("  Notepad.EXE "), "notepad");
        assert_eq!(app_key("TextEdit.app"), "textedit");
        assert!(password_prompt("SecurityAgent") && password_prompt("consent.exe") && !password_prompt("Consentio Notes"));
    }
}
