//! macOS: the Accessibility API (AXUIElement) for reading windows and pressing or setting elements, the window list
//! for the open apps, `open -a` to open one. Needs Pointcast to have Accessibility, as pasting does.
use super::{input, AppInfo, AppRef, Combo, Desktop, Limits, Node, Picture, Tree, Window, WALK_BUDGET};
use crate::element::{ax_role, tidy};
use core_foundation::array::{CFArray, CFArrayRef};
use core_foundation::base::{CFType, CFTypeRef, TCFType};
use core_foundation::boolean::CFBoolean;
use core_foundation::number::CFNumber;
use core_foundation::string::{CFString, CFStringRef};
use std::ffi::c_void;
use std::time::Instant;

type AxRef = *const c_void;

#[repr(C)]
#[derive(Default)]
struct Pair {
    a: f64,
    b: f64,
}

const POINT_TYPE: u32 = 1;
const SIZE_TYPE: u32 = 2;
/// How long one question to an app may take before it counts as no answer.
const TIMEOUT_S: f32 = 0.5;

#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn AXUIElementCreateSystemWide() -> AxRef;
    fn AXUIElementCreateApplication(pid: i32) -> AxRef;
    fn AXUIElementCopyAttributeValue(el: AxRef, attr: CFStringRef, out: *mut CFTypeRef) -> i32;
    fn AXUIElementSetAttributeValue(el: AxRef, attr: CFStringRef, value: CFTypeRef) -> i32;
    fn AXUIElementIsAttributeSettable(el: AxRef, attr: CFStringRef, out: *mut u8) -> i32;
    fn AXUIElementGetAttributeValueCount(el: AxRef, attr: CFStringRef, out: *mut isize) -> i32;
    fn AXUIElementCopyActionNames(el: AxRef, out: *mut CFArrayRef) -> i32;
    fn AXUIElementPerformAction(el: AxRef, action: CFStringRef) -> i32;
    fn AXUIElementCopyElementAtPosition(app: AxRef, x: f32, y: f32, out: *mut AxRef) -> i32;
    fn AXUIElementGetPid(el: AxRef, out: *mut i32) -> i32;
    fn AXUIElementSetMessagingTimeout(el: AxRef, seconds: f32) -> i32;
    fn AXValueGetValue(value: CFTypeRef, kind: u32, out: *mut c_void) -> u8;
}

pub struct Mac;

/// What an AX error code means, for the AI and the journal.
fn ax_error(code: i32, doing: &str) -> String {
    match code {
        -25211 => "Accessibility is off for Pointcast (System Settings › Privacy & Security › Accessibility).".into(),
        -25204 => format!("The app did not answer while {doing}."),
        -25205 | -25206 => format!("That element can't do that ({doing})."),
        -25202 => format!("That element is gone ({doing}): list the elements again."),
        _ => format!("macOS refused {doing} (AX error {code})."),
    }
}

fn access() -> Result<(), String> {
    if crate::permissions::can_paste() {
        Ok(())
    } else {
        Err(ax_error(-25211, ""))
    }
}

fn owned(r: CFTypeRef) -> Option<CFType> {
    (!r.is_null()).then(|| unsafe { CFType::wrap_under_create_rule(r) })
}

fn key(name: &str) -> CFString {
    CFString::new(name)
}

fn attr(el: &CFType, name: &str) -> Option<CFType> {
    let mut v: CFTypeRef = std::ptr::null();
    let err = unsafe { AXUIElementCopyAttributeValue(el.as_CFTypeRef(), key(name).as_concrete_TypeRef(), &mut v) };
    if err == 0 { owned(v) } else { None }
}

fn text(el: &CFType, name: &str) -> String {
    attr(el, name).and_then(|v| v.downcast_into::<CFString>()).map(|s| s.to_string()).unwrap_or_default()
}

fn pair(el: &CFType, name: &str, kind: u32) -> Option<(f64, f64)> {
    let v = attr(el, name)?;
    let mut p = Pair::default();
    let ok = unsafe { AXValueGetValue(v.as_CFTypeRef(), kind, &mut p as *mut Pair as *mut c_void) };
    (ok != 0).then_some((p.a, p.b))
}

fn elements(v: CFType) -> Vec<CFType> {
    let Some(arr) = v.downcast_into::<CFArray>() else { return vec![] };
    arr.iter().filter(|p| !p.is_null()).map(|p| unsafe { CFType::wrap_under_get_rule(*p) }).collect()
}

fn children(el: &CFType) -> Vec<CFType> {
    attr(el, "AXChildren").map(elements).unwrap_or_default()
}

fn settable(el: &CFType, name: &str) -> bool {
    let mut b = 0u8;
    let err = unsafe { AXUIElementIsAttributeSettable(el.as_CFTypeRef(), key(name).as_concrete_TypeRef(), &mut b) };
    err == 0 && b != 0
}

fn pid_of(el: &CFType) -> Option<u32> {
    let mut pid = 0i32;
    let err = unsafe { AXUIElementGetPid(el.as_CFTypeRef(), &mut pid) };
    (err == 0 && pid > 0).then_some(pid as u32)
}

fn app_element(pid: u32) -> Option<CFType> {
    let app = owned(unsafe { AXUIElementCreateApplication(pid as i32) })?;
    unsafe { AXUIElementSetMessagingTimeout(app.as_CFTypeRef(), TIMEOUT_S) };
    Some(app)
}

fn app_ref(pid: u32) -> AppRef {
    let name = app_element(pid).map(|a| text(&a, "AXTitle")).unwrap_or_default();
    AppRef { name, pid, elevated: false }
}

fn system() -> Result<CFType, String> {
    let wide = owned(unsafe { AXUIElementCreateSystemWide() }).ok_or("no system-wide accessibility element")?;
    unsafe { AXUIElementSetMessagingTimeout(wide.as_CFTypeRef(), TIMEOUT_S) };
    Ok(wide)
}

/// Plain words for what an element can do.
fn action_word(ax: &str) -> Option<String> {
    Some(match ax {
        "AXPress" => "press".into(),
        "AXIncrement" => "increment".into(),
        "AXDecrement" => "decrement".into(),
        "AXConfirm" => "confirm".into(),
        "AXCancel" => "cancel".into(),
        "AXShowMenu" => "menu".into(),
        "AXPick" => "pick".into(),
        "AXRaise" => "raise".into(),
        "AXScrollToVisible" | "AXShowAlternateUI" | "AXShowDefaultUI" => return None,
        other => other.trim_start_matches("AX").to_lowercase(),
    })
}

fn value_text(el: &CFType) -> Option<String> {
    let v = attr(el, "AXValue")?;
    if let Some(s) = v.clone().downcast_into::<CFString>() {
        return Some(s.to_string());
    }
    if let Some(b) = v.clone().downcast_into::<CFBoolean>() {
        return Some(bool::from(b).to_string());
    }
    v.downcast_into::<CFNumber>().and_then(|n| n.to_f64()).map(|f| if f.fract() == 0.0 { format!("{}", f as i64) } else { format!("{f}") })
}

fn read(el: &CFType, path: Vec<usize>) -> Node {
    let raw_role = text(el, "AXRole");
    let subrole = text(el, "AXSubrole");
    let secure = raw_role == "AXSecureTextField" || subrole == "AXSecureTextField";
    let name = ["AXTitle", "AXDescription", "AXPlaceholderValue"]
        .iter()
        .map(|a| text(el, a))
        .find(|s| !s.trim().is_empty())
        .unwrap_or_default();
    let (x, y) = pair(el, "AXPosition", POINT_TYPE).unwrap_or_default();
    let (w, h) = pair(el, "AXSize", SIZE_TYPE).unwrap_or_default();
    let mut actions: Vec<String> = {
        let mut names: CFArrayRef = std::ptr::null();
        let err = unsafe { AXUIElementCopyActionNames(el.as_CFTypeRef(), &mut names) };
        let list = if err == 0 { owned(names as CFTypeRef) } else { None };
        list.map(elements)
            .unwrap_or_default()
            .into_iter()
            .filter_map(|a| a.downcast_into::<CFString>())
            .filter_map(|s| action_word(&s.to_string()))
            .collect()
    };
    if !secure && settable(el, "AXValue") {
        actions.push("set value".into());
    }
    Node {
        depth: path.len(),
        path,
        role: if secure { "password field".into() } else { ax_role(&raw_role).into() },
        raw_role,
        subrole,
        name: tidy(&name),
        value: if secure { None } else { value_text(el) },
        secure,
        rect: (x.round() as i32, y.round() as i32, w.round() as i32, h.round() as i32),
        actions,
    }
}

/// The front app and its focused window (else its main window, else its first).
fn front_window() -> Result<(AppRef, CFType), String> {
    access()?;
    let wide = system()?;
    let app = attr(&wide, "AXFocusedApplication").ok_or("No app is in front.")?;
    let pid = pid_of(&app).ok_or("The front app has no process id.")?;
    unsafe { AXUIElementSetMessagingTimeout(app.as_CFTypeRef(), TIMEOUT_S) };
    let win = window_of(&app).ok_or_else(|| format!("{} has no window open.", text(&app, "AXTitle")))?;
    Ok((AppRef { name: text(&app, "AXTitle"), pid, elevated: false }, win))
}

fn window_of(app: &CFType) -> Option<CFType> {
    attr(app, "AXFocusedWindow")
        .or_else(|| attr(app, "AXMainWindow"))
        .or_else(|| attr(app, "AXWindows").map(elements).and_then(|w| w.into_iter().next()))
}

/// The element at `path` under the window, checked to be the one listed: same role, same name.
fn resolve(tree: &Tree, node: &Node) -> Result<CFType, String> {
    access()?;
    let app = app_element(tree.app.pid).ok_or("The app is gone.")?;
    let mut el = window_of(&app).ok_or("The window is gone: list the elements again.")?;
    for &i in &node.path {
        el = children(&el).into_iter().nth(i).ok_or("The window changed: list the elements again.")?;
    }
    let now = read(&el, node.path.clone());
    if now.raw_role != node.raw_role || now.name != node.name {
        return Err("The window changed: list the elements again.".into());
    }
    Ok(el)
}

impl Desktop for Mac {
    fn front(&self) -> Result<AppRef, String> {
        access()?;
        let app = attr(&system()?, "AXFocusedApplication").ok_or("No app is in front.")?;
        let pid = pid_of(&app).ok_or("The front app has no process id.")?;
        Ok(AppRef { name: text(&app, "AXTitle"), pid, elevated: false })
    }

    fn apps(&self) -> Result<Vec<AppInfo>, String> {
        let front = self.front().ok().map(|a| a.pid);
        let mut out: Vec<AppInfo> = vec![];
        for (pid, owner, w) in windows() {
            match out.iter_mut().find(|a| a.app.pid == pid) {
                Some(a) => a.windows.push(w),
                None => out.push(AppInfo { app: AppRef { name: owner, pid, elevated: false }, front: Some(pid) == front, windows: vec![w] }),
            }
        }
        // The front app first, then front to back.
        out.sort_by_key(|a| !a.front);
        Ok(out)
    }

    fn tree(&self, limits: Limits) -> Result<Tree, String> {
        let (app, win) = front_window()?;
        let started = Instant::now();
        let mut nodes = vec![];
        let mut cut = false;
        let mut stack = vec![(win.clone(), Vec::<usize>::new())];
        while let Some((el, path)) = stack.pop() {
            if nodes.len() >= limits.visit || started.elapsed() > WALK_BUDGET {
                cut = true;
                break;
            }
            if path.len() < limits.depth {
                for (i, k) in children(&el).into_iter().enumerate().rev() {
                    let mut p = path.clone();
                    p.push(i);
                    stack.push((k, p));
                }
            } else {
                let mut n = 0isize;
                let err = unsafe { AXUIElementGetAttributeValueCount(el.as_CFTypeRef(), key("AXChildren").as_concrete_TypeRef(), &mut n) };
                cut |= err == 0 && n > 0;
            }
            nodes.push(read(&el, path));
        }
        Ok(Tree { window: text(&win, "AXTitle"), app, nodes, cut })
    }

    fn at(&self, x: i32, y: i32) -> Result<Option<(AppRef, Node)>, String> {
        access()?;
        let mut hit: AxRef = std::ptr::null();
        let err = unsafe { AXUIElementCopyElementAtPosition(system()?.as_CFTypeRef(), x as f32, y as f32, &mut hit) };
        let Some(el) = (if err == 0 { owned(hit) } else { None }) else { return Ok(None) };
        Ok(pid_of(&el).map(|pid| (app_ref(pid), read(&el, vec![]))))
    }

    fn focused(&self) -> Result<Option<(AppRef, Node)>, String> {
        access()?;
        let Some(el) = attr(&system()?, "AXFocusedUIElement") else { return Ok(None) };
        Ok(pid_of(&el).map(|pid| (app_ref(pid), read(&el, vec![]))))
    }

    fn press(&self, tree: &Tree, node: &Node) -> Result<(), String> {
        let el = resolve(tree, node)?;
        let err = unsafe { AXUIElementPerformAction(el.as_CFTypeRef(), key("AXPress").as_concrete_TypeRef()) };
        if err == 0 { Ok(()) } else { Err(ax_error(err, "pressing it")) }
    }

    fn set_value(&self, tree: &Tree, node: &Node, text: &str) -> Result<(), String> {
        let el = resolve(tree, node)?;
        let v = CFString::new(text);
        let err = unsafe { AXUIElementSetAttributeValue(el.as_CFTypeRef(), key("AXValue").as_concrete_TypeRef(), v.as_CFTypeRef()) };
        if err == 0 { Ok(()) } else { Err(ax_error(err, "setting its value")) }
    }

    fn click_at(&self, x: i32, y: i32, count: u32) -> Result<(), String> {
        access()?;
        input::click_at(x, y, count)
    }

    fn type_text(&self, text: &str) -> Result<(), String> {
        access()?;
        input::type_text(text)
    }

    fn key(&self, combo: &Combo) -> Result<(), String> {
        access()?;
        input::key(combo)
    }

    fn scroll(&self, x: i32, y: i32, dx: i32, dy: i32) -> Result<(), String> {
        access()?;
        input::scroll(x, y, dx, dy)
    }

    fn open_app(&self, name: &str) -> Result<(), String> {
        let out = std::process::Command::new("/usr/bin/open").arg("-a").arg(name).output().map_err(|e| e.to_string())?;
        if out.status.success() {
            Ok(())
        } else {
            Err(format!("macOS could not open {name}: {}", String::from_utf8_lossy(&out.stderr).trim()))
        }
    }

    fn focus(&self, app: &AppInfo, window: Option<&str>) -> Result<(), String> {
        access()?;
        let el = app_element(app.app.pid).ok_or("The app is gone.")?;
        let yes = CFBoolean::true_value();
        let err = unsafe { AXUIElementSetAttributeValue(el.as_CFTypeRef(), key("AXFrontmost").as_concrete_TypeRef(), yes.as_CFTypeRef()) };
        if err != 0 {
            return Err(ax_error(err, "bringing it forward"));
        }
        let Some(want) = window.map(str::to_lowercase) else { return Ok(()) };
        let win = attr(&el, "AXWindows")
            .map(elements)
            .unwrap_or_default()
            .into_iter()
            .find(|w| text(w, "AXTitle").to_lowercase().contains(&want))
            .ok_or_else(|| format!("{} has no window titled like \"{want}\".", app.app.name))?;
        unsafe {
            AXUIElementPerformAction(win.as_CFTypeRef(), key("AXRaise").as_concrete_TypeRef());
            AXUIElementSetAttributeValue(win.as_CFTypeRef(), key("AXMain").as_concrete_TypeRef(), yes.as_CFTypeRef());
        }
        Ok(())
    }

    fn screen(&self) -> Result<Picture, String> {
        input::screen()
    }
}

/// Every app window in the window list, front to back: its owner's pid and name, and the window. Normal windows only
/// (layer 0), minimised ones included.
fn windows() -> Vec<(u32, String, Window)> {
    use core_foundation::dictionary::CFDictionary;
    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        fn CGWindowListCopyWindowInfo(option: u32, relative_to: u32) -> CFArrayRef;
    }
    fn get(d: &CFDictionary, k: &str) -> Option<CFType> {
        let k = CFString::new(k);
        d.find(k.as_CFTypeRef()).map(|v| unsafe { CFType::wrap_under_get_rule(*v) })
    }
    let txt = |d: &CFDictionary, k: &str| get(d, k).and_then(|v| v.downcast::<CFString>()).map(|s| s.to_string());
    let num = |d: &CFDictionary, k: &str| get(d, k).and_then(|v| v.downcast::<CFNumber>()).and_then(|n| n.to_f64());
    // kCGWindowListOptionAll, excluding desktop elements.
    let raw = unsafe { CGWindowListCopyWindowInfo(16, 0) };
    if raw.is_null() {
        return vec![];
    }
    let list: CFArray = unsafe { CFArray::wrap_under_create_rule(raw) };
    list.iter()
        .filter_map(|item| {
            let d: CFDictionary = unsafe { CFDictionary::wrap_under_get_rule((*item).cast()) };
            if num(&d, "kCGWindowLayer") != Some(0.0) {
                return None;
            }
            let owner = txt(&d, "kCGWindowOwnerName")?;
            let pid = num(&d, "kCGWindowOwnerPID")? as u32;
            let bounds = get(&d, "kCGWindowBounds")?.downcast::<CFDictionary>()?;
            let at = |k: &str| num(&bounds, k).unwrap_or(0.0) as i32;
            let shown = get(&d, "kCGWindowIsOnscreen").and_then(|v| v.downcast::<CFBoolean>()).map(bool::from).unwrap_or(false);
            let title = txt(&d, "kCGWindowName").unwrap_or_default();
            // Off-screen windows without a title are an app's hidden helpers, not windows anyone sees.
            if !shown && title.is_empty() {
                return None;
            }
            Some((pid, owner, Window { title, rect: (at("X"), at("Y"), at("Width"), at("Height")), minimized: !shown }))
        })
        .collect()
}
