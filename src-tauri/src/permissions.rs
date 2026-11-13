//! Microphone, Screen Recording and Accessibility: what the OS says, how to ask, where to fix it.
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Access {
    /// Allowed, or the OS has no such gate.
    Granted,
    /// Never asked yet; asking shows the system prompt.
    Unknown,
    /// Refused; only System Settings can change it.
    Denied,
}

#[derive(Debug, Clone, Serialize)]
pub struct PermissionStatus {
    pub microphone: Access,
    pub screen: Access,
    /// Needed to paste into the focused app (macOS only).
    pub accessibility: Access,
    /// Screen Recording is switched on in System Settings, but macOS applies it only once Pointcast reopens.
    pub screen_pending: bool,
    /// Whether this OS gates the pane at all, so the UI can hide rows that never apply.
    pub gated: Gated,
    /// Off now, but granted to an earlier build: macOS likely kept the old version's switch (an ad hoc signature ties
    /// the grant to that build), so the switch shows on while it does not apply.
    pub stale: Stale,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct Stale {
    pub screen: bool,
    pub accessibility: bool,
}

/// What a permission's row should say.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Health {
    Fine,
    Off,
    /// Off, and last seen on under another build: macOS kept the old version's switch.
    StaleAfterUpdate,
}

/// The build a permission was seen granted under: the version plus the executable's size and mtime, which an update
/// changes even when the version stays.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, serde::Deserialize)]
pub struct Build {
    pub version: String,
    pub exe: String,
}

/// The build each permission was last seen granted under, kept in `permissions-seen.json` in the data folder.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, serde::Deserialize)]
pub struct Seen {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub screen: Option<Build>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accessibility: Option<Build>,
}

/// Granted reads Fine; off under the build it was last granted in (or never granted) reads Off; off after the build
/// changed reads StaleAfterUpdate.
pub fn health(prev: Option<&Build>, now: &Build, granted: bool) -> Health {
    match prev {
        _ if granted => Health::Fine,
        Some(p) if p != now => Health::StaleAfterUpdate,
        _ => Health::Off,
    }
}

impl Seen {
    /// Note the build for each permission granted now. True when the record changed.
    fn note(&mut self, now: &Build, screen: bool, accessibility: bool) -> bool {
        let was = self.clone();
        if screen {
            self.screen = Some(now.clone());
        }
        if accessibility {
            self.accessibility = Some(now.clone());
        }
        *self != was
    }

    fn forget(&mut self, kind: &str) {
        match kind {
            "screen" => self.screen = None,
            "accessibility" => self.accessibility = None,
            _ => {}
        }
    }
}

fn seen_path() -> std::path::PathBuf {
    crate::settings::data_dir().join("permissions-seen.json")
}

fn load_seen() -> Seen {
    std::fs::read_to_string(seen_path()).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

fn save_seen(s: &Seen) {
    if let Ok(t) = serde_json::to_string(s) {
        let _ = std::fs::create_dir_all(crate::settings::data_dir());
        let _ = std::fs::write(seen_path(), t);
    }
}

/// This process's build, read once: the running code does not change even if an update replaces the file.
fn this_build() -> Build {
    static EXE: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    let exe = EXE.get_or_init(|| {
        std::env::current_exe()
            .and_then(std::fs::metadata)
            .map(|m| {
                let t = m.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs());
                format!("{}-{t}", m.len())
            })
            .unwrap_or_default()
    });
    Build { version: env!("CARGO_PKG_VERSION").to_string(), exe: exe.clone() }
}

/// Record what is granted now and say which permissions read off only because macOS kept an older build's switch.
fn stale_now(mac: bool, screen: Access, accessibility: Access) -> Stale {
    if !mac {
        return Stale::default();
    }
    let now = this_build();
    let mut seen = load_seen();
    let (s, a) = (screen == Access::Granted, accessibility == Access::Granted);
    if seen.note(&now, s, a) {
        save_seen(&seen);
    }
    Stale {
        screen: health(seen.screen.as_ref(), &now, s) == Health::StaleAfterUpdate,
        accessibility: health(seen.accessibility.as_ref(), &now, a) == Health::StaleAfterUpdate,
    }
}

/// Pointcast's bundle id (tauri.conf.json `identifier`); the only one a reset may name.
pub const BUNDLE_ID: &str = "dev.pointcast.app";

/// The `tccutil` arguments that forget one permission for Pointcast alone: never `All`, never another app.
pub fn reset_argv(kind: &str) -> Option<[&'static str; 3]> {
    let service = match kind {
        "screen" => "ScreenCapture",
        "accessibility" => "Accessibility",
        _ => return None,
    };
    Some(["reset", service, BUNDLE_ID])
}

/// Forget the old build's switch for one permission, then ask again (the prompt, or the switch in System Settings).
pub fn reset_and_ask(kind: &str) -> Result<(), String> {
    if !cfg!(target_os = "macos") {
        return Err("only macOS keeps an old version's switch".into());
    }
    let argv = reset_argv(kind).ok_or("nothing to reset for this")?;
    let ok = std::process::Command::new("/usr/bin/tccutil").args(argv).status().map_err(|e| e.to_string())?.success();
    if !ok {
        return Err("macOS did not reset it".into());
    }
    let mut seen = load_seen();
    seen.forget(kind);
    save_seen(&seen);
    if kind == "screen" {
        // The reset makes macOS show its prompt again, so asking must not go straight to System Settings.
        let _ = std::fs::remove_file(crate::settings::data_dir().join("screen-asked"));
    }
    request(kind);
    Ok(())
}

#[derive(Debug, Clone, Serialize)]
pub struct Gated {
    pub microphone: bool,
    pub screen: bool,
    pub accessibility: bool,
}

pub fn probe() -> PermissionStatus {
    let screen = screen();
    let accessibility = accessibility();
    PermissionStatus {
        microphone: microphone(),
        screen,
        accessibility,
        stale: stale_now(cfg!(target_os = "macos"), screen, accessibility),
        screen_pending: pending(cfg!(target_os = "macos"), screen, crate::capture::sees_other_titles),
        gated: Gated {
            microphone: true,
            screen: cfg!(target_os = "macos"),
            accessibility: cfg!(target_os = "macos"),
        },
    }
}

/// Whether the microphones may be listed now. On macOS listing them counts as using the microphone and brings up
/// its permission prompt, so it waits until access is granted (asked for on the setup assistant's access step).
pub fn may_list_microphones() -> bool {
    // Off macOS the status is never read here: on Windows reading it lists the microphones, which would ask again.
    !cfg!(target_os = "macos") || lists_microphones(true, microphone())
}

/// A Screen Recording grant macOS has not applied yet: it says no, yet other apps' window titles show.
fn pending(mac: bool, screen: Access, sees_titles: impl FnOnce() -> bool) -> bool {
    mac && screen != Access::Granted && sees_titles()
}

fn lists_microphones(mac: bool, mic: Access) -> bool {
    !mac || mic == Access::Granted
}

/// Whether a synthetic Cmd/Ctrl+V will reach other apps (macOS needs Accessibility for it).
pub fn can_paste() -> bool {
    accessibility() == Access::Granted
}

/// Whether a capture shows other apps' windows: without Screen Recording macOS hands back only
/// the wallpaper and the menu bar.
pub fn can_capture() -> bool {
    screen() == Access::Granted
}

/// Settings page for one permission (`microphone`, `screen`, `accessibility`).
pub fn settings_url(kind: &str) -> Option<&'static str> {
    if cfg!(target_os = "macos") {
        return Some(match kind {
            "microphone" => "x-apple.systempreferences:com.apple.preference.security?Privacy_Microphone",
            "screen" => "x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture",
            "accessibility" => "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility",
            _ => return None,
        });
    }
    if cfg!(windows) && kind == "microphone" {
        return Some("ms-settings:privacy-microphone");
    }
    None
}

/// Show the system prompt where the OS has one. The answer arrives later; probe again.
pub fn request(kind: &str) -> Access {
    match kind {
        "screen" => request_screen(),
        "accessibility" => request_accessibility(),
        _ => microphone(),
    }
}

fn microphone() -> Access {
    #[cfg(target_os = "macos")]
    {
        mac::microphone()
    }
    #[cfg(windows)]
    {
        win::microphone()
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        if crate::voice::audio::devices().is_empty() {
            Access::Denied
        } else {
            Access::Granted
        }
    }
}

fn screen() -> Access {
    #[cfg(target_os = "macos")]
    {
        mac::screen()
    }
    #[cfg(not(target_os = "macos"))]
    {
        Access::Granted
    }
}

pub fn request_screen() -> Access {
    #[cfg(target_os = "macos")]
    {
        mac::request_screen()
    }
    #[cfg(not(target_os = "macos"))]
    {
        Access::Granted
    }
}

fn accessibility() -> Access {
    #[cfg(target_os = "macos")]
    {
        mac::accessibility(false)
    }
    #[cfg(not(target_os = "macos"))]
    {
        Access::Granted
    }
}

fn request_accessibility() -> Access {
    #[cfg(target_os = "macos")]
    {
        mac::accessibility(true)
    }
    #[cfg(not(target_os = "macos"))]
    {
        Access::Granted
    }
}

#[cfg(target_os = "macos")]
mod mac {
    use super::Access;
    use core_foundation::base::TCFType;
    use core_foundation::boolean::CFBoolean;
    use core_foundation::dictionary::{CFDictionary, CFDictionaryRef};
    use core_foundation::string::{CFString, CFStringRef};
    use objc2::runtime::AnyObject;
    use objc2::{class, msg_send};

    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        fn CGRequestScreenCaptureAccess() -> bool;
        fn CGPreflightScreenCaptureAccess() -> bool;
    }

    #[link(name = "ApplicationServices", kind = "framework")]
    extern "C" {
        fn AXIsProcessTrusted() -> bool;
        fn AXIsProcessTrustedWithOptions(options: CFDictionaryRef) -> bool;
        static kAXTrustedCheckOptionPrompt: CFStringRef;
    }

    #[link(name = "AVFoundation", kind = "framework")]
    extern "C" {
        static AVMediaTypeAudio: *const AnyObject;
    }

    /// AVAuthorizationStatus: 0 not determined, 1 restricted, 2 denied, 3 authorized.
    pub fn microphone() -> Access {
        let status: isize =
            unsafe { msg_send![class!(AVCaptureDevice), authorizationStatusForMediaType: AVMediaTypeAudio] };
        match status {
            3 => Access::Granted,
            0 => Access::Unknown,
            _ => Access::Denied,
        }
    }

    /// Left once Pointcast has asked for Screen Recording, naming the version that asked: macOS shows its prompt only
    /// the first time.
    fn asked() -> std::path::PathBuf {
        crate::settings::data_dir().join("screen-asked")
    }

    pub fn screen() -> Access {
        let marker = std::fs::read_to_string(asked()).ok();
        let asked = super::asked_by(marker.as_deref(), env!("CARGO_PKG_VERSION"));
        super::screen_access(unsafe { CGPreflightScreenCaptureAccess() }, asked)
    }

    pub fn request_screen() -> Access {
        let marker = std::fs::read_to_string(asked()).ok();
        let asked_before = super::asked_by(marker.as_deref(), env!("CARGO_PKG_VERSION"));
        match super::screen_step(unsafe { CGPreflightScreenCaptureAccess() }, asked_before) {
            super::Step::Done => Access::Granted,
            super::Step::Settings => {
                // macOS has answered once and will not show its prompt again: the switch is the only way.
                if let Some(url) = super::settings_url("screen") {
                    let _ = std::process::Command::new("/usr/bin/open").arg(url).spawn();
                }
                Access::Denied
            }
            super::Step::Prompt => {
                let _ = std::fs::create_dir_all(crate::settings::data_dir());
                let _ = std::fs::write(asked(), env!("CARGO_PKG_VERSION"));
                if unsafe { CGRequestScreenCaptureAccess() } {
                    Access::Granted
                } else {
                    Access::Unknown
                }
            }
        }
    }

    /// With `prompt`, macOS adds Pointcast to the Accessibility list and shows its dialog.
    pub fn accessibility(prompt: bool) -> Access {
        let ok = unsafe {
            if prompt {
                let key = CFString::wrap_under_get_rule(kAXTrustedCheckOptionPrompt);
                let opts = CFDictionary::from_CFType_pairs(&[(
                    key.as_CFType(),
                    CFBoolean::true_value().as_CFType(),
                )]);
                AXIsProcessTrustedWithOptions(opts.as_concrete_TypeRef())
            } else {
                AXIsProcessTrusted()
            }
        };
        if ok {
            Access::Granted
        } else {
            Access::Unknown
        }
    }
}

#[cfg(windows)]
mod win {
    use super::Access;
    use std::os::windows::process::CommandExt;

    const STORE: &str =
        r"HKCU\Software\Microsoft\Windows\CurrentVersion\CapabilityAccessManager\ConsentStore\microphone";
    const DEVICE: &str =
        r"HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\CapabilityAccessManager\ConsentStore\microphone";
    const POLICY: &str = r"HKLM\SOFTWARE\Policies\Microsoft\Windows\AppPrivacy";
    const NO_WINDOW: u32 = 0x0800_0000;

    fn value(key: &str, name: &str) -> Option<String> {
        let out = std::process::Command::new("reg")
            .args(["query", key, "/v", name])
            .creation_flags(NO_WINDOW)
            .output()
            .ok()?;
        let text = String::from_utf8_lossy(&out.stdout).into_owned();
        text.lines()
            .find(|l| l.split_whitespace().next() == Some(name))
            .and_then(|l| l.split_whitespace().last())
            .map(str::to_string)
    }

    /// Windows: the privacy switch for all apps, the one for desktop apps, the one for the whole
    /// device, and an administrator's policy. Read again at most every few seconds: the setup
    /// assistant asks often, and each read starts `reg`.
    pub fn microphone() -> Access {
        type Switches = [Option<String>; 4];
        static READ: std::sync::Mutex<Option<(std::time::Instant, Switches)>> = std::sync::Mutex::new(None);
        let [all, desktop, device, policy] = super::cached(&READ, std::time::Duration::from_secs(5), || {
            [
                value(STORE, "Value"),
                value(&format!(r"{STORE}\NonPackaged"), "Value"),
                value(DEVICE, "Value"),
                value(POLICY, "LetAppsAccessMicrophone"),
            ]
        });
        super::windows_mic(all.as_deref(), desktop.as_deref(), device.as_deref(), policy.as_deref())
    }
}

/// Screen Recording: macOS says only yes or no, and shows its prompt once ever; after that only
/// System Settings can turn it on, so a no after asking is a refusal.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn screen_access(allowed: bool, asked: bool) -> Access {
    match (allowed, asked) {
        (true, _) => Access::Granted,
        (false, false) => Access::Unknown,
        (false, true) => Access::Denied,
    }
}

/// What asking for Screen Recording does.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
#[derive(Debug, PartialEq)]
enum Step {
    /// Already allowed.
    Done,
    /// Never asked: macOS shows its prompt.
    Prompt,
    /// Asked before: macOS shows nothing a second time, so Allow opens the switch in System Settings instead of
    /// doing nothing.
    Settings,
}

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn screen_step(allowed: bool, asked: bool) -> Step {
    match (allowed, asked) {
        (true, _) => Step::Done,
        (false, false) => Step::Prompt,
        (false, true) => Step::Settings,
    }
}

/// The first release signed with Pointcast's certificate (docs/SIGNING.md). From it on an update keeps what macOS
/// was allowed and what it was asked: install.sh resets the grants only when the signer changes, and then it deletes
/// the marker too.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
const FIRST_SIGNED: (u64, u64, u64) = (0, 4, 8);

/// Whether Pointcast has asked for Screen Recording since macOS last forgot it (`marker` names the version that
/// asked). A version signed ad hoc, before the certificate, was a stranger to macOS after every update, so its
/// asking does not count; one signed with the certificate kept the answer, so a quiet update does not make the
/// assistant offer a prompt that macOS will not show again.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn asked_by(marker: Option<&str>, version: &str) -> bool {
    let Some(m) = marker.map(str::trim).filter(|m| !m.is_empty()) else {
        return false;
    };
    m == version || numbers(m).is_some_and(|v| v >= FIRST_SIGNED)
}

/// "1.2.3" as numbers.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn numbers(v: &str) -> Option<(u64, u64, u64)> {
    let mut n = v.trim_start_matches('v').split(['-', '+']).next()?.split('.').map(|p| p.parse::<u64>().ok());
    Some((n.next()??, n.next().unwrap_or(Some(0))?, n.next().unwrap_or(Some(0))?))
}

/// What `read` gave, read again only once `ttl` has passed since the read kept in `last`.
#[cfg_attr(not(windows), allow(dead_code))]
fn cached<T: Clone>(last: &std::sync::Mutex<Option<(std::time::Instant, T)>>, ttl: std::time::Duration, read: impl FnOnce() -> T) -> T {
    let mut last = last.lock().unwrap();
    if let Some((_, v)) = last.as_ref().filter(|(at, _)| at.elapsed() < ttl) {
        return v.clone();
    }
    let v = read();
    *last = Some((std::time::Instant::now(), v.clone()));
    v
}

/// Any switch at "Deny" blocks a desktop app's microphone, and so does a policy that forces it off
/// (LetAppsAccessMicrophone 2, a DWORD `reg` prints as 0x2).
#[cfg_attr(not(windows), allow(dead_code))]
fn windows_mic(all: Option<&str>, desktop: Option<&str>, device: Option<&str>, policy: Option<&str>) -> Access {
    if [all, desktop, device].contains(&Some("Deny")) || policy == Some("0x2") {
        Access::Denied
    } else if crate::voice::audio::devices().is_empty() {
        Access::Unknown
    } else {
        Access::Granted
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn on_macos_microphones_are_listed_only_once_access_is_granted() {
        use super::{lists_microphones, Access};
        assert!(!lists_microphones(true, Access::Unknown), "listing would bring up the prompt before setup explains it");
        assert!(!lists_microphones(true, Access::Denied));
        assert!(lists_microphones(true, Access::Granted));
        assert!(lists_microphones(false, Access::Unknown), "Windows and Linux list them freely");
    }

    use super::*;

    #[test]
    fn screen_recording_switched_on_but_not_applied_is_pending_until_pointcast_reopens() {
        assert!(pending(true, Access::Denied, || true));
        assert!(pending(true, Access::Unknown, || true));
        assert!(!pending(true, Access::Denied, || false), "still off");
        assert!(!pending(true, Access::Granted, || panic!("not read once granted")));
        assert!(!pending(false, Access::Granted, || panic!("not read off macOS")));
    }

    #[test]
    fn a_windows_deny_on_either_switch_blocks_the_microphone() {
        assert_eq!(windows_mic(Some("Deny"), Some("Allow"), None, None), Access::Denied);
        assert_eq!(windows_mic(Some("Allow"), Some("Deny"), None, None), Access::Denied);
        assert_eq!(windows_mic(None, Some("Deny"), None, None), Access::Denied);
        assert_ne!(windows_mic(Some("Allow"), Some("Allow"), None, None), Access::Denied);
    }

    #[test]
    fn the_device_switch_and_the_policy_block_the_microphone_whatever_the_user_allowed() {
        let allow = (Some("Allow"), Some("Allow"));
        assert_eq!(windows_mic(allow.0, allow.1, Some("Deny"), None), Access::Denied, "microphone access for this device is off");
        // LetAppsAccessMicrophone: 0 the user decides, 1 force allow, 2 force deny.
        assert_eq!(windows_mic(allow.0, allow.1, Some("Allow"), Some("0x2")), Access::Denied);
        assert_ne!(windows_mic(allow.0, allow.1, Some("Allow"), Some("0x0")), Access::Denied);
        assert_eq!(windows_mic(Some("Deny"), allow.1, None, Some("0x1")), Access::Denied, "Deny wins over a force allow");
        assert_ne!(windows_mic(allow.0, allow.1, None, Some("0x1")), Access::Denied);
    }

    #[test]
    fn screen_recording_refused_after_asking_reads_as_denied() {
        assert_eq!(screen_access(true, false), Access::Granted);
        assert_eq!(screen_access(true, true), Access::Granted);
        assert_eq!(screen_access(false, false), Access::Unknown, "never asked: asking shows the prompt");
        assert_eq!(screen_access(false, true), Access::Denied, "asked before: only System Settings helps");
        if !cfg!(target_os = "macos") {
            assert!(can_capture(), "only macOS gates captures");
        }
    }

    #[test]
    fn after_an_update_from_an_ad_hoc_build_screen_recording_is_asked_for_again() {
        assert!(asked_by(Some("0.4.0"), "0.4.0"));
        assert!(!asked_by(None, "0.4.0"), "never asked");
        assert!(!asked_by(Some(""), "0.4.9"), "the empty marker an earlier version left");
        assert!(!asked_by(Some("0.3.9"), "0.4.0"));
        assert!(!asked_by(Some("0.4.7"), "0.4.9"), "the last ad hoc build: moving to the certificate reset the grant");
        // Updated and reset by tccutil: the assistant offers the prompt again, not only System Settings.
        assert_eq!(screen_access(false, asked_by(Some("0.3.9"), "0.4.0")), Access::Unknown);
        assert_eq!(screen_access(false, asked_by(Some("0.4.0"), "0.4.0")), Access::Denied);
    }

    #[test]
    fn allow_after_macos_has_answered_opens_the_switch_instead_of_doing_nothing() {
        assert_eq!(screen_step(true, false), Step::Done);
        assert_eq!(screen_step(true, true), Step::Done);
        assert_eq!(screen_step(false, false), Step::Prompt, "the first time macOS shows its prompt");
        assert_eq!(screen_step(false, true), Step::Settings, "after that it shows nothing");
        // The row reads the same as the step: a Denied row's Allow goes to System Settings.
        for allowed in [true, false] {
            for asked in [true, false] {
                let row = screen_access(allowed, asked);
                let step = screen_step(allowed, asked);
                assert_eq!(row == Access::Granted, step == Step::Done);
                assert_eq!(row == Access::Denied, step == Step::Settings);
            }
        }
    }

    #[test]
    fn a_quiet_update_keeps_what_was_asked() {
        // Both signed with the certificate: macOS kept the answer and will not show its prompt again, so the
        // assistant must not offer one that does nothing.
        for asked in ["0.4.8", " 0.4.8 ", "0.5.0", "1.0"] {
            assert!(asked_by(Some(asked), "0.5.1"), "{asked:?}");
            assert_eq!(screen_access(false, asked_by(Some(asked), "0.5.1")), Access::Denied, "{asked:?}");
        }
        assert!(!asked_by(Some("nonsense"), "0.5.1"));
        assert_eq!(numbers("v0.4.8-beta.1"), Some((0, 4, 8)));
        assert_eq!(numbers("1"), Some((1, 0, 0)));
        assert_eq!(numbers("x.1"), None);
    }

    #[test]
    fn a_read_is_kept_for_a_few_seconds_then_read_again() {
        let last = std::sync::Mutex::new(None);
        let reads = std::cell::Cell::new(0);
        let read = || {
            reads.set(reads.get() + 1);
            reads.get()
        };
        let long = std::time::Duration::from_secs(60);
        assert_eq!((cached(&last, long, read), cached(&last, long, read)), (1, 1), "the second ask reads nothing");
        assert_eq!(cached(&last, std::time::Duration::ZERO, read), 2, "once the time is up it reads again");
        assert_eq!(cached(&last, long, read), 2);
    }

    fn build(version: &str, exe: &str) -> Build {
        Build { version: version.into(), exe: exe.into() }
    }

    #[test]
    fn a_permission_off_after_an_update_reads_as_a_stale_grant() {
        let now = build("0.5.1", "100-2");
        assert_eq!(health(None, &now, true), Health::Fine);
        assert_eq!(health(Some(&build("0.5.0", "90-1")), &now, true), Health::Fine);
        assert_eq!(health(None, &now, false), Health::Off, "never seen on");
        assert_eq!(health(Some(&now), &now, false), Health::Off, "turned off under this very build");
        assert_eq!(health(Some(&build("0.5.0", "90-1")), &now, false), Health::StaleAfterUpdate);
        assert_eq!(health(Some(&build("0.5.1", "90-1")), &now, false), Health::StaleAfterUpdate, "a rebuild of the same version");
    }

    #[test]
    fn the_record_moves_only_with_a_grant_and_a_reset_forgets_it() {
        let (old, now) = (build("0.5.0", "1"), build("0.5.1", "2"));
        let mut s = Seen { screen: Some(old.clone()), accessibility: Some(old.clone()) };
        assert!(!s.note(&now, false, false), "an off permission keeps the build it was on in");
        assert_eq!(s.screen, Some(old.clone()));
        assert!(s.note(&now, false, true));
        assert_eq!((s.screen.as_ref(), s.accessibility.as_ref()), (Some(&old), Some(&now)));
        assert!(!s.note(&now, false, true), "nothing new to write");
        s.forget("screen");
        assert_eq!(health(s.screen.as_ref(), &now, false), Health::Off, "after a reset it reads plainly off");
        let back: Seen = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!(back, s);
        assert_eq!(serde_json::from_str::<Seen>("{}").unwrap(), Seen::default());
    }

    #[test]
    fn stale_is_never_reported_off_macos() {
        assert_eq!(stale_now(false, Access::Denied, Access::Unknown), Stale::default());
    }

    #[test]
    fn a_reset_names_one_service_and_pointcast_alone() {
        assert_eq!(reset_argv("screen"), Some(["reset", "ScreenCapture", "dev.pointcast.app"]));
        assert_eq!(reset_argv("accessibility"), Some(["reset", "Accessibility", "dev.pointcast.app"]));
        for kind in ["microphone", "All", "all", "", "screen ", "ScreenCapture", "Accessibility", "accessibility;All"] {
            assert_eq!(reset_argv(kind), None, "{kind:?}");
        }
        let conf: serde_json::Value = serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        assert_eq!(conf["identifier"], BUNDLE_ID, "the reset names this app's own bundle id");
    }

    #[test]
    fn no_reset_can_reach_another_service_or_app() {
        // Every string a caller could pass: the argv is one of two, whatever comes in.
        let allowed = [["reset", "ScreenCapture", BUNDLE_ID], ["reset", "Accessibility", BUNDLE_ID]];
        let tries = ["screen", "accessibility", "microphone", "All", "camera", "com.apple.Terminal", "-h", "reset"];
        for t in tries.iter().flat_map(|a| [a.to_string(), a.to_uppercase(), format!("{a} All")]) {
            if let Some(argv) = reset_argv(&t) {
                assert!(allowed.contains(&argv), "{t:?} gave {argv:?}");
                assert!(!argv.contains(&"All"));
            }
        }
        if !cfg!(target_os = "macos") {
            assert!(reset_and_ask("screen").is_err(), "no reset off macOS");
        }
        assert!(reset_and_ask("microphone").is_err(), "nothing runs for a service with no reset");
    }

    #[test]
    fn every_gated_permission_has_a_settings_page() {
        let g = probe().gated;
        for (kind, on) in [("microphone", g.microphone), ("screen", g.screen), ("accessibility", g.accessibility)] {
            if on && (cfg!(target_os = "macos") || cfg!(windows)) {
                assert!(settings_url(kind).is_some(), "{kind}");
            }
        }
        assert_eq!(settings_url("nope"), None);
    }
}
