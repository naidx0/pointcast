//! Mic and screen-recording permission probes. On macOS, request the TCC prompts.
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct PermissionStatus {
    pub microphone: bool,
    pub screen: bool,
    pub note: String,
}

pub fn probe() -> PermissionStatus {
    let microphone = !crate::voice::audio::devices().is_empty();
    let screen = probe_screen();
    let note = if cfg!(target_os = "macos") {
        "macOS: System Settings → Privacy & Security → Screen Recording and Microphone. Pointcast asks on first capture / first listen."
            .into()
    } else if cfg!(windows) {
        "Windows: allow microphone access in Settings → Privacy. Screen capture uses the Desktop Duplication / GDI path via xcap."
            .into()
    } else {
        "Linux: a working X11/Wayland session and PulseAudio/PipeWire for the mic. Wayland portals may prompt on first capture."
            .into()
    };
    PermissionStatus {
        microphone,
        screen,
        note,
    }
}

fn probe_screen() -> bool {
    xcap::Monitor::all().map(|m| !m.is_empty()).unwrap_or(false)
}

/// Ask the OS for screen recording (macOS 10.15+). No-op elsewhere.
pub fn request_screen() -> bool {
    #[cfg(target_os = "macos")]
    {
        #[link(name = "CoreGraphics", kind = "framework")]
        extern "C" {
            fn CGRequestScreenCaptureAccess() -> u8;
            fn CGPreflightScreenCaptureAccess() -> u8;
        }
        unsafe {
            if CGPreflightScreenCaptureAccess() != 0 {
                return true;
            }
            CGRequestScreenCaptureAccess() != 0
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        probe_screen()
    }
}
