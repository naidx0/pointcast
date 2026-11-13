//! Linux and the rest: the computer tools are listed with a grant like anywhere, and each answers that it is not
//! supported here.
use super::{AppInfo, AppRef, Combo, Desktop, Limits, Node, Picture, Tree};

pub struct Unsupported;

const NO: &str = "Pointcast's computer tools are not supported on this system yet (macOS and Windows only).";

fn no<T>() -> Result<T, String> {
    Err(NO.into())
}

impl Desktop for Unsupported {
    fn front(&self) -> Result<AppRef, String> {
        no()
    }
    fn apps(&self) -> Result<Vec<AppInfo>, String> {
        no()
    }
    fn tree(&self, _: Limits) -> Result<Tree, String> {
        no()
    }
    fn at(&self, _: i32, _: i32) -> Result<Option<(AppRef, Node)>, String> {
        no()
    }
    fn focused(&self) -> Result<Option<(AppRef, Node)>, String> {
        no()
    }
    fn press(&self, _: &Tree, _: &Node) -> Result<(), String> {
        no()
    }
    fn set_value(&self, _: &Tree, _: &Node, _: &str) -> Result<(), String> {
        no()
    }
    fn click_at(&self, _: i32, _: i32, _: u32) -> Result<(), String> {
        no()
    }
    fn type_text(&self, _: &str) -> Result<(), String> {
        no()
    }
    fn key(&self, _: &Combo) -> Result<(), String> {
        no()
    }
    fn scroll(&self, _: i32, _: i32, _: i32, _: i32) -> Result<(), String> {
        no()
    }
    fn open_app(&self, _: &str) -> Result<(), String> {
        no()
    }
    fn focus(&self, _: &AppInfo, _: Option<&str>) -> Result<(), String> {
        no()
    }
    fn screen(&self) -> Result<Picture, String> {
        no()
    }
}
