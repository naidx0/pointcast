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
    /// Whether this OS gates the pane at all, so the UI can hide rows that never apply.
    pub gated: Gated,
}

#[derive(Debug, Clone, Serialize)]
pub struct Gated {
    pub microphone: bool,
    pub screen: bool,
    pub accessibility: bool,
}

pub fn probe() -> PermissionStatus {
    PermissionStatus {
        microphone: microphone(),
        screen: screen(),
        accessibility: accessibility(),
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
    lists_microphones(cfg!(target_os = "macos"), microphone())
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
        if unsafe { CGPreflightScreenCaptureAccess() } {
            return Access::Granted;
        }
        let _ = std::fs::create_dir_all(crate::settings::data_dir());
        let _ = std::fs::write(asked(), env!("CARGO_PKG_VERSION"));
        if unsafe { CGRequestScreenCaptureAccess() } {
            Access::Granted
        } else {
            Access::Unknown
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

/// Whether this version has asked for Screen Recording (`marker` names the version that asked). An update clears the
/// grant with `tccutil reset`, which also lets macOS prompt again, so an earlier version's asking does not count.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn asked_by(marker: Option<&str>, version: &str) -> bool {
    marker.is_some_and(|m| m.trim() == version)
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
    fn after_an_update_screen_recording_is_asked_for_again() {
        assert!(asked_by(Some("0.4.0"), "0.4.0"));
        assert!(!asked_by(None, "0.4.0"), "never asked");
        assert!(!asked_by(Some(""), "0.4.0"), "the empty marker an earlier version left");
        assert!(!asked_by(Some("0.3.9"), "0.4.0"));
        // Updated and reset by tccutil: the assistant offers the prompt again, not only System Settings.
        assert_eq!(screen_access(false, asked_by(Some("0.3.9"), "0.4.0")), Access::Unknown);
        assert_eq!(screen_access(false, asked_by(Some("0.4.0"), "0.4.0")), Access::Denied);
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
