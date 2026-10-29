//! The control under the pointer, from the OS accessibility tree (UI Automation on Windows, Accessibility on macOS):
//! a plain word for what it is, its name and its bounds, so a capture can outline it and the prompt can name it.
use crate::capture::Mapping;
use crate::pointer::Point;
use crate::privacy::Rect;
use serde::{Deserialize, Serialize};

/// What the OS reports at a point: `rect` is `(x, y, w, h)` in screen units.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Element {
    pub role: String,
    pub name: String,
    pub rect: (i32, i32, i32, i32),
}

/// An element as a capture keeps it, its rect in picture pixels.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ElementMark {
    pub role: String,
    pub name: String,
    pub rect: Bounds,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Bounds {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Bounds {
    pub fn rect(self) -> Rect {
        Rect { x: self.x, y: self.y, w: self.w, h: self.h }
    }
}

/// An element covering more of the monitor than this is the page or the window, not a control.
pub const MAX_SHARE: f64 = 0.6;
/// How long a capture waits for the OS to answer.
pub const WAIT_MS: u64 = 500;
/// Names longer than this are cut when kept.
const NAME_KEEP: usize = 200;

/// Never what someone points at.
const WHOLE: &[&str] = &["window", "app"];
/// Holders of other things: worth outlining only when they have a name.
const HOLDERS: &[&str] = &["pane", "group", "control", "document", "text"];

/// A plain word for a UI Automation control type id; `aria` is the element's ARIA role, which names web headings.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn uia_role(id: i32, aria: &str) -> &'static str {
    match id {
        50020 | 50025 | 50026 if aria.eq_ignore_ascii_case("heading") => "heading",
        50000 | 50031 => "button",
        50001 => "calendar",
        50002 => "checkbox",
        50003 => "dropdown",
        50004 => "text field",
        50005 => "link",
        50006 => "image",
        50007 => "list item",
        50008 => "list",
        50009 => "menu",
        50010 => "menu bar",
        50011 => "menu item",
        50012 => "progress bar",
        50013 => "radio button",
        50014 => "scroll bar",
        50015 => "slider",
        50016 => "spinner",
        50017 => "status bar",
        50018 => "tab bar",
        50019 => "tab",
        50020 => "text",
        50021 | 50040 => "toolbar",
        50022 => "tooltip",
        50023 => "tree",
        50024 => "tree item",
        50026 => "group",
        50027 => "handle",
        50028 | 50036 => "table",
        50029 => "row",
        50030 => "document",
        50032 => "window",
        50033 => "pane",
        50034 => "header",
        50035 => "column header",
        50037 => "title bar",
        50038 => "separator",
        _ => "control",
    }
}

/// A plain word for a macOS Accessibility role.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn ax_role(role: &str) -> &'static str {
    match role {
        "AXButton" => "button",
        "AXLink" => "link",
        "AXTextField" | "AXTextArea" | "AXSearchField" => "text field",
        "AXComboBox" | "AXPopUpButton" | "AXMenuButton" => "dropdown",
        "AXCheckBox" => "checkbox",
        "AXRadioButton" => "radio button",
        "AXMenuItem" | "AXMenuBarItem" => "menu item",
        "AXMenu" => "menu",
        "AXMenuBar" => "menu bar",
        "AXTabGroup" => "tab bar",
        "AXImage" => "image",
        "AXHeading" => "heading",
        "AXStaticText" => "text",
        "AXSlider" => "slider",
        "AXIncrementor" => "spinner",
        "AXProgressIndicator" | "AXBusyIndicator" => "progress bar",
        "AXList" => "list",
        "AXOutline" => "tree",
        "AXTable" | "AXGrid" => "table",
        "AXRow" => "row",
        "AXCell" => "cell",
        "AXColumn" => "column",
        "AXToolbar" => "toolbar",
        "AXScrollBar" => "scroll bar",
        "AXDisclosureTriangle" => "disclosure triangle",
        "AXSplitter" => "separator",
        "AXValueIndicator" => "handle",
        "AXColorWell" => "color well",
        "AXDateField" => "date field",
        "AXHelpTag" => "tooltip",
        "AXWindow" | "AXSheet" | "AXDrawer" => "window",
        "AXApplication" | "AXSystemWide" => "app",
        "AXGroup" | "AXRadioGroup" | "AXSplitGroup" | "AXLayoutArea" | "AXLayoutItem" => "group",
        "AXScrollArea" => "pane",
        "AXWebArea" => "document",
        _ => "control",
    }
}

/// A name on one line, without padding, at most `NAME_KEEP` characters.
pub fn tidy(name: &str) -> String {
    name.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(NAME_KEEP).collect()
}

/// Whether `e` is worth outlining for a pointer at `at` on a `monitor` (width, height) in screen units: not the window or
/// the desktop, not most of the monitor, not empty, not an unnamed holder, and under the pointer.
pub fn keep(e: &Element, at: (i32, i32), monitor: (f64, f64)) -> bool {
    let (x, y, w, h) = e.rect;
    let role = e.role.as_str();
    w > 0
        && h > 0
        && !WHOLE.contains(&role)
        && !(HOLDERS.contains(&role) && e.name.is_empty())
        && w as f64 * h as f64 <= MAX_SHARE * monitor.0 * monitor.1
        && at.0 >= x
        && at.1 >= y
        && at.0 - x < w
        && at.1 - y < h
}

/// The element in a picture, its rect mapped and clipped like the pointer; `None` when it is not worth outlining or
/// not on the picture. `at` is the screen point it was asked for.
pub fn mark(e: &Element, at: Point, map: &Mapping) -> Option<ElementMark> {
    if map.scale <= 0.0 {
        return None;
    }
    let monitor = (map.frame.0 as f64 / map.scale, map.frame.1 as f64 / map.scale);
    if !keep(e, (at.x, at.y), monitor) {
        return None;
    }
    let (x, y, w, h) = e.rect;
    let r = map.place_rect(Rect { x, y, w, h })?;
    Some(ElementMark {
        role: e.role.clone(),
        name: e.name.clone(),
        rect: Bounds { x: r.x, y: r.y, w: r.w, h: r.h },
    })
}

/// The element under a screen point, asked on a short-lived thread so a slow or hung app costs a capture at most
/// `WAIT_MS`. On a Mac only when Pointcast has Accessibility.
#[cfg(any(windows, target_os = "macos"))]
pub fn at(x: i32, y: i32) -> Option<Element> {
    #[cfg(target_os = "macos")]
    if !crate::permissions::can_paste() {
        return None;
    }
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(os::query(x, y));
    });
    rx.recv_timeout(std::time::Duration::from_millis(WAIT_MS)).ok().flatten()
}

#[cfg(not(any(windows, target_os = "macos")))]
pub fn at(_x: i32, _y: i32) -> Option<Element> {
    None
}

#[cfg(windows)]
mod os {
    use super::{tidy, uia_role, Element};
    use windows::core::Interface;
    use windows::Win32::Foundation::POINT;
    use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED};
    use windows::Win32::UI::Accessibility::{CUIAutomation, CUIAutomation8, IUIAutomation, IUIAutomation2};

    /// COM is set up and torn down on the calling thread for this one question.
    pub fn query(x: i32, y: i32) -> Option<Element> {
        unsafe {
            let ours = CoInitializeEx(None, COINIT_MULTITHREADED).is_ok();
            let found = ask(x, y);
            if ours {
                CoUninitialize();
            }
            found
        }
    }

    unsafe fn ask(x: i32, y: i32) -> Option<Element> {
        let ui: IUIAutomation = CoCreateInstance(&CUIAutomation8, None, CLSCTX_INPROC_SERVER)
            .or_else(|_| CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER))
            .ok()?;
        if let Ok(two) = ui.cast::<IUIAutomation2>() {
            let _ = two.SetConnectionTimeout(400);
            let _ = two.SetTransactionTimeout(400);
        }
        let el = ui.ElementFromPoint(POINT { x, y }).ok()?;
        let r = el.CurrentBoundingRectangle().ok()?;
        let aria = el.CurrentAriaRole().map(|b| b.to_string()).unwrap_or_default();
        Some(Element {
            role: uia_role(el.CurrentControlType().map_or(0, |t| t.0), &aria).into(),
            name: tidy(&el.CurrentName().map(|b| b.to_string()).unwrap_or_default()),
            rect: (r.left, r.top, r.right - r.left, r.bottom - r.top),
        })
    }
}

#[cfg(target_os = "macos")]
mod os {
    use super::{ax_role, tidy, Element};
    use core_foundation::base::{CFType, CFTypeRef, TCFType};
    use core_foundation::string::{CFString, CFStringRef};
    use std::ffi::c_void;

    type AxRef = *const c_void;

    #[repr(C)]
    #[derive(Default)]
    struct Pair {
        a: f64,
        b: f64,
    }

    const POINT_TYPE: u32 = 1;
    const SIZE_TYPE: u32 = 2;

    #[link(name = "ApplicationServices", kind = "framework")]
    extern "C" {
        fn AXUIElementCreateSystemWide() -> AxRef;
        fn AXUIElementCopyElementAtPosition(app: AxRef, x: f32, y: f32, out: *mut AxRef) -> i32;
        fn AXUIElementCopyAttributeValue(el: AxRef, attr: CFStringRef, out: *mut CFTypeRef) -> i32;
        fn AXUIElementSetMessagingTimeout(el: AxRef, seconds: f32) -> i32;
        fn AXValueGetValue(value: CFTypeRef, kind: u32, out: *mut c_void) -> u8;
    }

    fn owned(r: CFTypeRef) -> Option<CFType> {
        (!r.is_null()).then(|| unsafe { CFType::wrap_under_create_rule(r) })
    }

    fn attr(el: &CFType, name: &'static str) -> Option<CFType> {
        let mut v: CFTypeRef = std::ptr::null();
        let key = CFString::from_static_string(name);
        let err = unsafe { AXUIElementCopyAttributeValue(el.as_CFTypeRef(), key.as_concrete_TypeRef(), &mut v) };
        if err == 0 { owned(v) } else { None }
    }

    fn text(el: &CFType, name: &'static str) -> String {
        attr(el, name).and_then(|v| v.downcast_into::<CFString>()).map(|s| s.to_string()).unwrap_or_default()
    }

    fn pair(el: &CFType, name: &'static str, kind: u32) -> Option<(f64, f64)> {
        let v = attr(el, name)?;
        let mut p = Pair::default();
        let ok = unsafe { AXValueGetValue(v.as_CFTypeRef(), kind, &mut p as *mut Pair as *mut c_void) };
        (ok != 0).then_some((p.a, p.b))
    }

    pub fn query(x: i32, y: i32) -> Option<Element> {
        let wide = owned(unsafe { AXUIElementCreateSystemWide() })?;
        unsafe { AXUIElementSetMessagingTimeout(wide.as_CFTypeRef(), 0.4) };
        let mut hit: AxRef = std::ptr::null();
        let err = unsafe { AXUIElementCopyElementAtPosition(wide.as_CFTypeRef(), x as f32, y as f32, &mut hit) };
        let el = if err == 0 { owned(hit)? } else { return None };
        let (px, py) = pair(&el, "AXPosition", POINT_TYPE)?;
        let (w, h) = pair(&el, "AXSize", SIZE_TYPE)?;
        let title = text(&el, "AXTitle");
        let name = if title.trim().is_empty() { text(&el, "AXDescription") } else { title };
        Some(Element {
            role: ax_role(&text(&el, "AXRole")).into(),
            name: tidy(&name),
            rect: (px.round() as i32, py.round() as i32, w.round() as i32, h.round() as i32),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn el(role: &str, name: &str, rect: (i32, i32, i32, i32)) -> Element {
        Element { role: role.into(), name: name.into(), rect }
    }
    fn screen(origin: (i32, i32), scale: f64, w: u32, h: u32) -> Mapping {
        Mapping { origin, scale, frame: (w, h), crop: (0, 0), size: (w, h) }
    }
    fn p(x: i32, y: i32) -> Point {
        Point { t_ms: 0, x, y }
    }
    fn b(x: i32, y: i32, w: i32, h: i32) -> Bounds {
        Bounds { x, y, w, h }
    }

    #[test]
    fn controls_get_short_plain_words_on_windows_and_mac() {
        let uia = [
            (50000, "button"),
            (50031, "button"),
            (50005, "link"),
            (50004, "text field"),
            (50002, "checkbox"),
            (50011, "menu item"),
            (50019, "tab"),
            (50007, "list item"),
            (50006, "image"),
            (50003, "dropdown"),
            (50032, "window"),
            (50033, "pane"),
            (50026, "group"),
            (50025, "control"),
            (12345, "control"),
        ];
        for (id, word) in uia {
            assert_eq!(uia_role(id, ""), word, "{id}");
        }
        assert_eq!(uia_role(50020, "heading"), "heading", "a web heading is text with an ARIA role");
        assert_eq!(uia_role(50020, ""), "text");
        assert_eq!(uia_role(50000, "heading"), "button", "only text and holders become headings");
        let ax = [
            ("AXButton", "button"),
            ("AXLink", "link"),
            ("AXTextField", "text field"),
            ("AXTextArea", "text field"),
            ("AXCheckBox", "checkbox"),
            ("AXMenuItem", "menu item"),
            ("AXPopUpButton", "dropdown"),
            ("AXImage", "image"),
            ("AXHeading", "heading"),
            ("AXWindow", "window"),
            ("AXGroup", "group"),
            ("AXSomethingNew", "control"),
            ("", "control"),
        ];
        for (role, word) in ax {
            assert_eq!(ax_role(role), word, "{role}");
        }
        assert_eq!(tidy("  Place\n\torder  "), "Place order");
        assert_eq!(tidy(&"x".repeat(500)).len(), NAME_KEEP);
    }

    #[test]
    fn only_a_control_under_the_pointer_is_worth_outlining() {
        let m = (1920.0, 1080.0);
        let at = (1040, 455);
        assert!(keep(&el("button", "Place order", (987, 435, 116, 41)), at, m));
        assert!(keep(&el("button", "", (987, 435, 116, 41)), at, m), "an unnamed button still shows where");
        assert!(keep(&el("group", "Shipping address", (900, 400, 400, 200)), at, m), "a named group says something");
        assert!(!keep(&el("window", "Checkout", (900, 400, 400, 200)), at, m), "the whole window");
        assert!(!keep(&el("pane", "Desktop 1", (0, 0, 1920, 1080)), at, m), "the desktop");
        assert!(!keep(&el("document", "Northwind", (0, 100, 1920, 700)), at, m), "more than 60% of the monitor");
        assert!(keep(&el("document", "Northwind", (94, 135, 1100, 560)), at, m), "under 60% is fine");
        assert!(!keep(&el("button", "Place order", (987, 435, 0, 41)), at, m), "zero size");
        assert!(!keep(&el("button", "Place order", (987, 435, 116, -4)), at, m));
        for holder in ["pane", "group", "control", "document", "text"] {
            assert!(!keep(&el(holder, "", (987, 435, 116, 41)), at, m), "unnamed {holder}");
        }
        assert!(!keep(&el("button", "Place order", (100, 100, 116, 41)), at, m), "not under the pointer");
        assert!(!keep(&el("button", "Place order", (987, 435, 53, 41)), at, m), "right edge is outside");
        // On a Retina Mac the monitor and the element are both in points.
        assert!(keep(&el("link", "Help", (700, 500, 60, 20)), (720, 510), (1512.0, 982.0)));
    }

    #[test]
    fn the_outline_lands_on_the_picture_at_any_scale_and_monitor() {
        let button = el("button", "Place order", (987, 435, 116, 41));
        let at = p(1040, 455);
        let one = mark(&button, at, &screen((0, 0), 1.0, 1920, 1080)).unwrap();
        assert_eq!(one, ElementMark { role: "button".into(), name: "Place order".into(), rect: b(987, 435, 116, 41) });
        // Retina: points to pixels.
        let link = el("link", "Help", (700, 500, 60, 20));
        let two = mark(&link, p(720, 510), &screen((0, 0), 2.0, 3024, 1964)).unwrap();
        assert_eq!(two.rect, b(1400, 1000, 120, 40));
        // A monitor left of the main one.
        let left = el("checkbox", "Remember me", (-1500, 300, 20, 20));
        let neg = mark(&left, p(-1490, 310), &screen((-1920, 0), 1.0, 1920, 1080)).unwrap();
        assert_eq!(neg.rect, b(420, 300, 20, 20));
        // A close-up cut at (900, 400) and enlarged 2x keeps the part on it.
        let zoom = Mapping { crop: (900, 400), size: (128, 128), ..screen((0, 0), 1.0, 1920, 1080) }.times(2);
        assert_eq!(mark(&button, at, &zoom).unwrap().rect, b(174, 70, 82, 82));
        // Rejected, or off this picture.
        assert_eq!(mark(&el("window", "Checkout", (987, 435, 116, 41)), at, &screen((0, 0), 1.0, 1920, 1080)), None);
        assert_eq!(mark(&left, p(-1490, 310), &screen((0, 0), 1.0, 1920, 1080)), None);
        assert_eq!(mark(&button, at, &Mapping::default()), None);
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "puts a small window on screen for a moment and asks UI Automation about it"]
    fn uia_names_a_button_on_a_window_this_test_made() {
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::{mpsc, Arc};
        use std::time::Duration;
        use windows::core::w;
        use windows::Win32::Foundation::{HINSTANCE, HWND, RECT};
        use windows::Win32::UI::WindowsAndMessaging::*;

        let done = Arc::new(AtomicBool::new(false));
        let stop = done.clone();
        let (tx, rx) = mpsc::channel();
        // The window's thread keeps answering messages while UI Automation asks it about the button.
        let ui = std::thread::spawn(move || unsafe {
            let _ = SetProcessDPIAware();
            let host = CreateWindowExW(
                WS_EX_TOPMOST | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW,
                w!("STATIC"),
                w!("Pointcast probe"),
                WS_POPUP | WS_VISIBLE | WS_BORDER,
                80,
                80,
                320,
                160,
                HWND::default(),
                HMENU::default(),
                HINSTANCE::default(),
                None,
            )
            .unwrap();
            let button = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("BUTTON"),
                w!("Place order"),
                WS_CHILD | WS_VISIBLE | WINDOW_STYLE(BS_PUSHBUTTON as u32),
                60,
                50,
                200,
                60,
                host,
                HMENU::default(),
                HINSTANCE::default(),
                None,
            )
            .unwrap();
            let mut r = RECT::default();
            GetWindowRect(button, &mut r).unwrap();
            tx.send(r).unwrap();
            let mut msg = MSG::default();
            while !stop.load(Ordering::SeqCst) {
                while PeekMessageW(&mut msg, HWND::default(), 0, 0, PM_REMOVE).as_bool() {
                    let _ = TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            let _ = DestroyWindow(button);
            let _ = DestroyWindow(host);
        });
        let r = rx.recv_timeout(Duration::from_secs(5)).unwrap();
        std::thread::sleep(Duration::from_millis(300));
        let (cx, cy) = ((r.left + r.right) / 2, (r.top + r.bottom) / 2);
        let got = at(cx, cy);
        // Beside the button, on the window that holds it: something else.
        let beside = at(r.left - 30, r.top - 20);
        done.store(true, Ordering::SeqCst);
        ui.join().unwrap();
        let got = got.expect("UI Automation named something at the button");
        assert_eq!((got.role.as_str(), got.name.as_str()), ("button", "Place order"), "{got:?}");
        assert_eq!(got.rect, (r.left, r.top, r.right - r.left, r.bottom - r.top));
        assert!(keep(&got, (cx, cy), (1920.0, 1080.0)));
        let beside = beside.expect("UI Automation named something beside the button");
        assert_ne!(beside.name, "Place order", "{beside:?}");
        assert_ne!(beside.rect, got.rect);
    }
}
