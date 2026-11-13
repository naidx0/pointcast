//! Windows: UI Automation for reading windows and invoking or setting elements, the top-level window list for the
//! open apps, ShellExecute to open one. Apps running as administrator are marked, and the guard refuses them.
//! Written against windows 0.58 and compiled only under cfg(windows): not yet run on a Windows PC.
use super::{input, AppInfo, AppRef, Combo, Desktop, Limits, Node, Picture, Tree, Window, WALK_BUDGET};
use crate::element::{tidy, uia_role};
use std::time::Instant;
use windows::core::{w, Interface, BSTR, HSTRING, PCWSTR, PWSTR};
use windows::Win32::Foundation::{CloseHandle, BOOL, HANDLE, HWND, LPARAM, POINT, RECT};
use windows::Win32::Security::{GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY};
use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED};
use windows::Win32::System::Threading::{OpenProcess, OpenProcessToken, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION};
use windows::Win32::UI::Accessibility::{
    CUIAutomation, CUIAutomation8, IUIAutomation, IUIAutomation2, IUIAutomationElement, IUIAutomationExpandCollapsePattern,
    IUIAutomationInvokePattern, IUIAutomationSelectionItemPattern, IUIAutomationTogglePattern, IUIAutomationTreeWalker,
    IUIAutomationValuePattern, UIA_ExpandCollapsePatternId, UIA_InvokePatternId, UIA_SelectionItemPatternId, UIA_TogglePatternId,
    UIA_ValuePatternId,
};
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetForegroundWindow, GetWindow, GetWindowRect, GetWindowTextW, GetWindowThreadProcessId, IsIconic, IsWindowVisible,
    SetForegroundWindow, ShowWindow, GW_OWNER, SW_RESTORE, SW_SHOWNORMAL,
};

pub struct Win;

/// COM is set up and torn down on the calling thread for this one call, as element.rs does.
fn com<T>(f: impl FnOnce(&IUIAutomation) -> Result<T, String>) -> Result<T, String> {
    unsafe {
        let ours = CoInitializeEx(None, COINIT_MULTITHREADED).is_ok();
        let r = (|| {
            let ui: IUIAutomation = CoCreateInstance(&CUIAutomation8, None, CLSCTX_INPROC_SERVER)
                .or_else(|_| CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER))
                .map_err(|e| format!("UI Automation is not available: {e}"))?;
            if let Ok(two) = ui.cast::<IUIAutomation2>() {
                let _ = two.SetConnectionTimeout(500);
                let _ = two.SetTransactionTimeout(500);
            }
            f(&ui)
        })();
        if ours {
            CoUninitialize();
        }
        r
    }
}

fn pid_of(h: HWND) -> u32 {
    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(h, Some(&mut pid)) };
    pid
}

/// The process's file name ("notepad.exe"), and whether it runs elevated; one that can't be asked counts as elevated.
fn process(pid: u32) -> (String, bool) {
    if pid == std::process::id() {
        return ("Pointcast".into(), false);
    }
    unsafe {
        let Ok(h) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, BOOL::from(false), pid) else { return (String::new(), true) };
        let mut buf = [0u16; 1024];
        let mut len = buf.len() as u32;
        let name = if QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len).is_ok() {
            let path = String::from_utf16_lossy(&buf[..len as usize]);
            path.rsplit(['\\', '/']).next().unwrap_or_default().to_string()
        } else {
            String::new()
        };
        let mut token = HANDLE::default();
        let mut elevated = true;
        if OpenProcessToken(h, TOKEN_QUERY, &mut token).is_ok() {
            let mut e = TOKEN_ELEVATION::default();
            let mut got = 0u32;
            let size = std::mem::size_of::<TOKEN_ELEVATION>() as u32;
            if GetTokenInformation(token, TokenElevation, Some(&mut e as *mut TOKEN_ELEVATION as *mut core::ffi::c_void), size, &mut got).is_ok() {
                elevated = e.TokenIsElevated != 0;
            }
            let _ = CloseHandle(token);
        }
        let _ = CloseHandle(h);
        (name, elevated)
    }
}

fn app_ref(pid: u32) -> AppRef {
    let (name, elevated) = process(pid);
    AppRef { name, pid, elevated }
}

fn title(h: HWND) -> String {
    let mut buf = [0u16; 512];
    let n = unsafe { GetWindowTextW(h, &mut buf) };
    String::from_utf16_lossy(&buf[..n.max(0) as usize])
}

/// Visible, titled, unowned top-level windows, front to back.
fn top_windows() -> Vec<HWND> {
    unsafe extern "system" fn each(h: HWND, out: LPARAM) -> BOOL {
        let list = &mut *(out.0 as *mut Vec<HWND>);
        let owned = GetWindow(h, GW_OWNER).is_ok_and(|o| !o.is_invalid());
        if IsWindowVisible(h).as_bool() && !owned && !title(h).trim().is_empty() {
            list.push(h);
        }
        BOOL(1)
    }
    let mut list: Vec<HWND> = vec![];
    unsafe {
        let _ = EnumWindows(Some(each), LPARAM(&mut list as *mut Vec<HWND> as isize));
    }
    list
}

fn rect_of(r: RECT) -> (i32, i32, i32, i32) {
    (r.left, r.top, r.right - r.left, r.bottom - r.top)
}

fn has<T: Interface>(el: &IUIAutomationElement, id: windows::Win32::UI::Accessibility::UIA_PATTERN_ID) -> Option<T> {
    unsafe { el.GetCurrentPatternAs::<T>(id).ok() }
}

fn read(el: &IUIAutomationElement, path: Vec<usize>) -> Node {
    unsafe {
        let ct = el.CurrentControlType().map_or(0, |t| t.0);
        let aria = el.CurrentAriaRole().map(|b| b.to_string()).unwrap_or_default();
        let secure = el.CurrentIsPassword().is_ok_and(|b| b.as_bool());
        let value_pattern = has::<IUIAutomationValuePattern>(el, UIA_ValuePatternId);
        let mut actions = vec![];
        if has::<IUIAutomationInvokePattern>(el, UIA_InvokePatternId).is_some() || has::<IUIAutomationTogglePattern>(el, UIA_TogglePatternId).is_some() {
            actions.push("press".to_string());
        }
        if has::<IUIAutomationSelectionItemPattern>(el, UIA_SelectionItemPatternId).is_some() {
            actions.push("select".into());
        }
        if has::<IUIAutomationExpandCollapsePattern>(el, UIA_ExpandCollapsePatternId).is_some() {
            actions.push("expand".into());
        }
        let writable = value_pattern.as_ref().is_some_and(|v| v.CurrentIsReadOnly().is_ok_and(|r| !r.as_bool()));
        if writable && !secure {
            actions.push("set value".into());
        }
        let value = if secure { None } else { value_pattern.and_then(|v| v.CurrentValue().ok()).map(|b| b.to_string()) };
        Node {
            depth: path.len(),
            path,
            raw_role: ct.to_string(),
            subrole: aria.clone(),
            role: if secure { "password field".into() } else { uia_role(ct, &aria).into() },
            name: tidy(&el.CurrentName().map(|b| b.to_string()).unwrap_or_default()),
            value,
            secure,
            rect: el.CurrentBoundingRectangle().map(rect_of).unwrap_or_default(),
            actions,
        }
    }
}

fn kids(walker: &IUIAutomationTreeWalker, el: &IUIAutomationElement) -> Vec<IUIAutomationElement> {
    let mut out = vec![];
    let mut next = unsafe { walker.GetFirstChildElement(el) }.ok();
    while let Some(c) = next {
        next = unsafe { walker.GetNextSiblingElement(&c) }.ok();
        out.push(c);
        if out.len() >= 5_000 {
            break;
        }
    }
    out
}

fn front_window(ui: &IUIAutomation) -> Result<(AppRef, HWND, IUIAutomationElement), String> {
    let h = unsafe { GetForegroundWindow() };
    if h.is_invalid() {
        return Err("No window is in front.".into());
    }
    let el = unsafe { ui.ElementFromHandle(h) }.map_err(|e| format!("UI Automation could not read the front window: {e}"))?;
    Ok((app_ref(pid_of(h)), h, el))
}

/// The element at `path` under the front window, checked to be the one listed: same control type, same name.
fn resolve(ui: &IUIAutomation, tree: &Tree, node: &Node) -> Result<IUIAutomationElement, String> {
    let (app, _, mut el) = front_window(ui)?;
    if app.pid != tree.app.pid {
        return Err("Another window is in front now: list the elements again.".into());
    }
    let walker = unsafe { ui.ControlViewWalker() }.map_err(|e| e.to_string())?;
    for &i in &node.path {
        el = kids(&walker, &el).into_iter().nth(i).ok_or("The window changed: list the elements again.")?;
    }
    let now = read(&el, node.path.clone());
    if now.raw_role != node.raw_role || now.name != node.name {
        return Err("The window changed: list the elements again.".into());
    }
    Ok(el)
}

fn with_app(el: IUIAutomationElement) -> Option<(AppRef, Node)> {
    let pid = unsafe { el.CurrentProcessId() }.ok()? as u32;
    Some((app_ref(pid), read(&el, vec![])))
}

impl Desktop for Win {
    fn front(&self) -> Result<AppRef, String> {
        let h = unsafe { GetForegroundWindow() };
        if h.is_invalid() {
            return Err("No window is in front.".into());
        }
        Ok(app_ref(pid_of(h)))
    }

    fn apps(&self) -> Result<Vec<AppInfo>, String> {
        let front = self.front().ok().map(|a| a.pid);
        let mut out: Vec<AppInfo> = vec![];
        for h in top_windows() {
            let pid = pid_of(h);
            let mut r = RECT::default();
            let _ = unsafe { GetWindowRect(h, &mut r) };
            let w = Window { title: title(h), rect: rect_of(r), minimized: unsafe { IsIconic(h) }.as_bool() };
            match out.iter_mut().find(|a| a.app.pid == pid) {
                Some(a) => a.windows.push(w),
                None => out.push(AppInfo { app: app_ref(pid), front: Some(pid) == front, windows: vec![w] }),
            }
        }
        out.sort_by_key(|a| !a.front);
        Ok(out)
    }

    fn tree(&self, limits: Limits) -> Result<Tree, String> {
        com(|ui| {
            let (app, h, win) = front_window(ui)?;
            let walker = unsafe { ui.ControlViewWalker() }.map_err(|e| e.to_string())?;
            let started = Instant::now();
            let mut nodes = vec![];
            let mut cut = false;
            let mut stack = vec![(win, Vec::<usize>::new())];
            while let Some((el, path)) = stack.pop() {
                if nodes.len() >= limits.visit || started.elapsed() > WALK_BUDGET {
                    cut = true;
                    break;
                }
                if path.len() < limits.depth {
                    for (i, k) in kids(&walker, &el).into_iter().enumerate().rev() {
                        let mut p = path.clone();
                        p.push(i);
                        stack.push((k, p));
                    }
                } else {
                    cut |= unsafe { walker.GetFirstChildElement(&el) }.is_ok();
                }
                nodes.push(read(&el, path));
            }
            Ok(Tree { app, window: title(h), nodes, cut })
        })
    }

    fn at(&self, x: i32, y: i32) -> Result<Option<(AppRef, Node)>, String> {
        com(|ui| Ok(unsafe { ui.ElementFromPoint(POINT { x, y }) }.ok().and_then(with_app)))
    }

    fn focused(&self) -> Result<Option<(AppRef, Node)>, String> {
        com(|ui| Ok(unsafe { ui.GetFocusedElement() }.ok().and_then(with_app)))
    }

    fn press(&self, tree: &Tree, node: &Node) -> Result<(), String> {
        com(|ui| {
            let el = resolve(ui, tree, node)?;
            unsafe {
                if let Some(inv) = has::<IUIAutomationInvokePattern>(&el, UIA_InvokePatternId) {
                    return inv.Invoke().map_err(|e| format!("Invoke failed: {e}"));
                }
                if let Some(t) = has::<IUIAutomationTogglePattern>(&el, UIA_TogglePatternId) {
                    return t.Toggle().map_err(|e| format!("Toggle failed: {e}"));
                }
            }
            Err("That element can't be pressed.".into())
        })
    }

    fn set_value(&self, tree: &Tree, node: &Node, text: &str) -> Result<(), String> {
        com(|ui| {
            let el = resolve(ui, tree, node)?;
            let vp = has::<IUIAutomationValuePattern>(&el, UIA_ValuePatternId).ok_or("That element has no value to set.")?;
            unsafe { vp.SetValue(&BSTR::from(text)) }.map_err(|e| format!("Setting the value failed: {e}"))
        })
    }

    fn click_at(&self, x: i32, y: i32, count: u32) -> Result<(), String> {
        input::click_at(x, y, count)
    }

    fn type_text(&self, text: &str) -> Result<(), String> {
        input::type_text(text)
    }

    fn key(&self, combo: &Combo) -> Result<(), String> {
        input::key(combo)
    }

    fn scroll(&self, x: i32, y: i32, dx: i32, dy: i32) -> Result<(), String> {
        input::scroll(x, y, dx, dy)
    }

    fn open_app(&self, name: &str) -> Result<(), String> {
        // ShellExecute finds an app by its registered name ("notepad", "winword"); no shell parses the name.
        let r = unsafe { ShellExecuteW(HWND::default(), w!("open"), &HSTRING::from(name), PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL) };
        if r.0 as isize > 32 {
            Ok(())
        } else {
            Err(format!("Windows could not open {name} (code {}).", r.0 as isize))
        }
    }

    fn focus(&self, app: &AppInfo, window: Option<&str>) -> Result<(), String> {
        let want = window.map(str::to_lowercase);
        let h = top_windows()
            .into_iter()
            .filter(|h| pid_of(*h) == app.app.pid)
            .find(|h| want.as_ref().is_none_or(|w| title(*h).to_lowercase().contains(w)))
            .ok_or_else(|| format!("{} has no window titled like that.", app.app.name))?;
        unsafe {
            if IsIconic(h).as_bool() {
                let _ = ShowWindow(h, SW_RESTORE);
            }
        }
        // UI Automation's focus brings a window forward where SetForegroundWindow alone is often refused.
        let _ = com(|ui| unsafe { ui.ElementFromHandle(h).and_then(|el| el.SetFocus()) }.map_err(|e| e.to_string()));
        unsafe {
            let _ = SetForegroundWindow(h);
        }
        Ok(())
    }

    fn screen(&self) -> Result<Picture, String> {
        input::screen()
    }
}
