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

    pub fn screen() -> Access {
        if unsafe { CGPreflightScreenCaptureAccess() } {
            Access::Granted
        } else {
            Access::Unknown
        }
    }

    pub fn request_screen() -> Access {
        if unsafe { CGPreflightScreenCaptureAccess() || CGRequestScreenCaptureAccess() } {
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
    const NO_WINDOW: u32 = 0x0800_0000;

    fn value(key: &str) -> Option<String> {
        let out = std::process::Command::new("reg")
            .args(["query", key, "/v", "Value"])
            .creation_flags(NO_WINDOW)
            .output()
            .ok()?;
        let text = String::from_utf8_lossy(&out.stdout).into_owned();
        text.lines()
            .find(|l| l.trim_start().starts_with("Value"))
            .and_then(|l| l.split_whitespace().last())
            .map(str::to_string)
    }

    /// Windows: the privacy switch for all apps, then the one for desktop apps.
    pub fn microphone() -> Access {
        let all = value(STORE);
        let desktop = value(&format!(r"{STORE}\NonPackaged"));
        super::windows_mic(all.as_deref(), desktop.as_deref())
    }
}

/// Either switch at "Deny" blocks a desktop app's microphone.
#[cfg_attr(not(windows), allow(dead_code))]
fn windows_mic(all: Option<&str>, desktop: Option<&str>) -> Access {
    if all == Some("Deny") || desktop == Some("Deny") {
        Access::Denied
    } else if crate::voice::audio::devices().is_empty() {
        Access::Unknown
    } else {
        Access::Granted
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_windows_deny_on_either_switch_blocks_the_microphone() {
        assert_eq!(windows_mic(Some("Deny"), Some("Allow")), Access::Denied);
        assert_eq!(windows_mic(Some("Allow"), Some("Deny")), Access::Denied);
        assert_eq!(windows_mic(None, Some("Deny")), Access::Denied);
        assert_ne!(windows_mic(Some("Allow"), Some("Allow")), Access::Denied);
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
