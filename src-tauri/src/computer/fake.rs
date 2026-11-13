//! A desktop for tests: a fixed window, apps and focus, and a record of everything the tools asked it to do.
use super::*;
use std::sync::{Arc, Mutex};

#[derive(Default)]
pub struct Fake {
    pub tree: Tree,
    pub apps: Vec<AppInfo>,
    pub focused: Option<(AppRef, Node)>,
    pub at: Option<(AppRef, Node)>,
    /// The app really in front, where synthetic keys go; `None` while it is the tree's app.
    pub frontmost: Arc<Mutex<Option<AppRef>>>,
    /// Asked forward, nothing comes: as macOS 14 does with a request from the background.
    pub stuck: bool,
    /// Open and save panels run in their own process: (the panel's pid, the pid of the app whose window it is on).
    pub panels: Vec<(u32, u32)>,
    /// What was done, in order: "press Send", "set Body=hi", "click 10,20 x1", "type hi", "key cmd+s"...
    pub did: Arc<Mutex<Vec<String>>>,
}

impl Fake {
    fn log(&self, s: String) -> Result<(), String> {
        self.did.lock().unwrap().push(s);
        Ok(())
    }

    /// `app` is now really in front, unless the fake is stuck.
    fn comes_forward(&self, app: AppRef) {
        if !self.stuck {
            *self.frontmost.lock().unwrap() = Some(app);
        }
    }
}

pub fn app(name: &str, pid: u32) -> AppRef {
    AppRef { name: name.into(), pid, elevated: false }
}

pub fn node(path: &[usize], raw: &str, role: &str, name: &str, actions: &[&str]) -> Node {
    Node {
        path: path.to_vec(),
        depth: path.len(),
        raw_role: raw.into(),
        role: role.into(),
        name: name.into(),
        rect: (100 + 10 * path.len() as i32, 100, 80, 20),
        actions: actions.iter().map(|a| a.to_string()).collect(),
        ..Default::default()
    }
}

/// A mail window: a Send button, a subject field, a body, a password field and an unnamed group.
pub fn mail() -> Fake {
    let mut password = node(&[3], "AXTextField", "text field", "Password", &["set value"]);
    password.subrole = "AXSecureTextField".into();
    password.secure = true;
    let mut body = node(&[2], "AXTextArea", "text field", "", &["set value"]);
    body.value = Some("Hi Sam".into());
    let mail = app("Mail", 4242);
    Fake {
        tree: Tree {
            app: mail.clone(),
            window: "New Message".into(),
            nodes: vec![
                node(&[], "AXWindow", "window", "New Message", &["raise"]),
                node(&[0], "AXButton", "button", "Send", &["press"]),
                node(&[1], "AXTextField", "text field", "Subject", &["set value"]),
                body,
                password,
                node(&[4], "AXGroup", "group", "", &[]),
                node(&[5], "AXButton", "button", "Bold", &["press"]),
            ],
            cut: false,
        },
        apps: vec![
            AppInfo { app: mail.clone(), front: true, windows: vec![Window { title: "New Message".into(), ..Default::default() }] },
            AppInfo { app: app("Safari", 77), front: false, windows: vec![] },
            AppInfo { app: AppRef { name: "Pointcast".into(), pid: std::process::id(), elevated: false }, front: false, windows: vec![] },
        ],
        focused: Some((mail.clone(), node(&[1], "AXTextField", "text field", "Subject", &["set value"]))),
        at: Some((mail, node(&[], "AXButton", "button", "Bold", &["press"]))),
        did: Default::default(),
        ..Default::default()
    }
}

impl Desktop for Fake {
    fn front(&self) -> Result<AppRef, String> {
        Ok(self.tree.app.clone())
    }
    fn apps(&self) -> Result<Vec<AppInfo>, String> {
        Ok(self.apps.clone())
    }
    fn tree(&self, limits: Limits) -> Result<Tree, String> {
        let mut t = self.tree.clone();
        t.nodes.retain(|n| n.depth <= limits.depth);
        Ok(t)
    }
    fn at(&self, _x: i32, _y: i32) -> Result<Option<(AppRef, Node)>, String> {
        Ok(self.at.clone())
    }
    fn focused(&self) -> Result<Option<(AppRef, Node)>, String> {
        Ok(self.focused.clone())
    }
    fn press(&self, _tree: &Tree, node: &Node) -> Result<(), String> {
        self.log(format!("press {}", node.name))
    }
    fn set_value(&self, _tree: &Tree, node: &Node, text: &str) -> Result<(), String> {
        self.log(format!("set {}={text}", node.name))
    }
    fn click_at(&self, x: i32, y: i32, count: u32) -> Result<(), String> {
        self.log(format!("click {x},{y} x{count}"))
    }
    fn type_text(&self, text: &str) -> Result<(), String> {
        self.log(format!("type {text}"))
    }
    fn key(&self, combo: &Combo) -> Result<(), String> {
        self.log(format!("key {:?}+{}", combo.mods, combo.key))
    }
    fn scroll(&self, x: i32, y: i32, dx: i32, dy: i32) -> Result<(), String> {
        self.log(format!("scroll {x},{y} by {dx},{dy}"))
    }
    fn open_app(&self, name: &str) -> Result<(), String> {
        let key = super::guard::app_key(name);
        let running = self.apps.iter().find(|a| super::guard::app_key(&a.app.name) == key).map(|a| a.app.clone());
        self.comes_forward(running.unwrap_or_else(|| app(name, 9000)));
        self.log(format!("open {name}"))
    }
    fn focus(&self, app: &AppInfo, window: Option<&str>) -> Result<(), String> {
        self.comes_forward(app.app.clone());
        self.log(format!("focus {} {window:?}", app.app.name))
    }
    fn frontmost(&self) -> Result<AppRef, String> {
        Ok(self.frontmost.lock().unwrap().clone().unwrap_or_else(|| self.tree.app.clone()))
    }
    fn panel_for(&self, panel: &AppRef, host: &AppRef) -> bool {
        self.panels.contains(&(panel.pid, host.pid))
    }
    fn screen(&self) -> Result<Picture, String> {
        Ok(Picture { bytes: vec![0x89, b'P', b'N', b'G'], mime: "image/png", width: 200, height: 100, area: (0, 0, 100, 50) })
    }
}
