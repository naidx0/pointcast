//! Password-manager and credential windows never appear in a screenshot.

/// Apps and window titles that never show in a screenshot (a substring, any case, of the app's name or the title).
pub const DEFAULT_HIDDEN: &[&str] = &[
    "1Password",
    "Bitwarden",
    "KeePass",
    "Dashlane",
    "LastPass",
    "Keeper",
    "NordPass",
    "Enpass",
    "RoboForm",
    "Proton Pass",
    "Credential Manager",
    "Windows Security",
    "CredentialUIBroker",
    "Keychain Access",
    "Secret Service",
];

/// A window's rectangle in the picture's pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

pub fn hides(app: &str, title: &str, list: &[String]) -> bool {
    let (a, t) = (app.to_lowercase(), title.to_lowercase());
    list.iter()
        .map(|s| s.trim().to_lowercase())
        .filter(|s| !s.is_empty())
        .any(|s| a.contains(&s) || t.contains(&s))
}

/// Paint `r` (clipped to the picture) one flat colour in an RGBA buffer `width` pixels wide.
pub fn mask(buf: &mut [u8], width: u32, height: u32, r: Rect) {
    let (w, h) = (width as i32, height as i32);
    let (x0, y0) = (r.x.clamp(0, w), r.y.clamp(0, h));
    let (x1, y1) = (
        r.x.saturating_add(r.w).clamp(0, w),
        r.y.saturating_add(r.h).clamp(0, h),
    );
    for y in y0..y1 {
        for x in x0..x1 {
            let i = ((y * w + x) * 4) as usize;
            if let Some(px) = buf.get_mut(i..i + 4) {
                px.copy_from_slice(&[24, 24, 27, 255]);
            }
        }
    }
}

pub fn default_hidden_list() -> Vec<String> {
    DEFAULT_HIDDEN.iter().map(|s| (*s).to_string()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hidden_windows_are_matched_by_app_or_title_and_painted_over() {
        let list = default_hidden_list();
        assert!(hides("1Password", "", &list));
        assert!(hides(
            "Credential Dialog Xaml Host",
            "Windows Security",
            &list
        ));
        assert!(hides("Google Chrome", "Bitwarden Web Vault", &list));
        assert!(hides("Keychain Access", "login", &list));
        assert!(!hides("Notepad", "notes.txt", &list));
        assert!(!hides("Notepad", "x", &["  ".into()]));

        let (w, h) = (4u32, 3u32);
        let mut buf = vec![200u8; (w * h * 4) as usize];
        mask(
            &mut buf,
            w,
            h,
            Rect {
                x: 2,
                y: 1,
                w: 9,
                h: 1,
            },
        );
        let px = |x: u32, y: u32| buf[((y * w + x) * 4) as usize];
        assert_eq!((px(2, 1), px(3, 1)), (24, 24));
        assert_eq!((px(1, 1), px(2, 0), px(2, 2)), (200, 200, 200));
        let before = buf.clone();
        mask(
            &mut buf,
            w,
            h,
            Rect {
                x: -50,
                y: -50,
                w: 10,
                h: 10,
            },
        );
        mask(
            &mut buf,
            w,
            h,
            Rect {
                x: 10,
                y: 0,
                w: i32::MAX,
                h: i32::MAX,
            },
        );
        assert_eq!(buf, before);
    }
}
