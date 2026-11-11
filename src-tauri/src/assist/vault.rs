//! The keys a person pastes. They live only in the system's own store for secrets (Credential Manager on Windows,
//! the login password store on a Mac), one item per provider named `Pointcast assistant/<provider id>`. A key is
//! never in `settings.json`, a file of Pointcast's, a log line, an error or an event. Linux has no store here yet:
//! a key is refused in words, and a model on this computer needs none.

/// What every item's name starts with.
pub const SERVICE: &str = "Pointcast assistant";

/// A key in memory. It cannot be printed, compared or written out, and `http` alone reads it, to put it in a
/// request's header. Its bytes are wiped when it is let go.
pub struct Secret(String);

impl Secret {
    pub fn new(key: impl Into<String>) -> Secret {
        Secret(key.into())
    }

    pub fn is_empty(&self) -> bool {
        self.0.trim().is_empty()
    }

    pub(super) fn expose(&self) -> &str {
        &self.0
    }
}

impl Drop for Secret {
    fn drop(&mut self) {
        // Zeros are still text, so the string stays whole while it is wiped.
        for b in unsafe { self.0.as_bytes_mut() } {
            unsafe { std::ptr::write_volatile(b, 0) };
        }
    }
}

fn item(id: &str) -> String {
    format!("{SERVICE}/{id}")
}

/// Keep `key` for the provider `id`, in place of any kept before.
pub fn set(id: &str, key: Secret) -> Result<(), String> {
    if key.is_empty() {
        return Err("There is no key to keep.".into());
    }
    store::set(&item(id), key.expose().trim().as_bytes())
}

pub fn get(id: &str) -> Option<Secret> {
    store::get(&item(id)).map(Secret)
}

/// Forget the key of `id`. One that was never kept is already forgotten.
pub fn delete(id: &str) -> Result<(), String> {
    store::delete(&item(id))
}

pub fn has(id: &str) -> bool {
    get(id).is_some()
}

/// The system's store through the keyring project's crates: a generic credential in Credential Manager kept on
/// this computer only (it does not roam), or a generic password in the login keychain. The secret is the key's own
/// bytes, so an item kept before the swap still reads. A keyring error's text never holds the secret.
#[cfg(any(windows, target_os = "macos"))]
mod store {
    use keyring_core::api::CredentialStoreApi;
    use keyring_core::{Entry, Error};

    const ACCOUNT: &str = "Pointcast";

    #[cfg(windows)]
    const OS: &str = "Windows";
    #[cfg(target_os = "macos")]
    const OS: &str = "macOS";

    #[cfg(windows)]
    fn entry(item: &str) -> Result<Entry, Error> {
        let named = std::collections::HashMap::from([("target", item), ("persistence", "Local")]);
        windows_native_keyring_store::Store::new()?.build(item, ACCOUNT, Some(&named))
    }

    #[cfg(target_os = "macos")]
    fn entry(item: &str) -> Result<Entry, Error> {
        apple_native_keyring_store::keychain::Store::new()?.build(item, ACCOUNT, None)
    }

    pub fn set(item: &str, key: &[u8]) -> Result<(), String> {
        entry(item).and_then(|e| e.set_secret(key)).map_err(|e| format!("{OS} did not keep the key: {e}"))
    }

    pub fn get(item: &str) -> Option<String> {
        String::from_utf8(entry(item).ok()?.get_secret().ok()?).ok().filter(|k| !k.is_empty())
    }

    pub fn delete(item: &str) -> Result<(), String> {
        match entry(item).and_then(|e| e.delete_credential()) {
            Ok(()) | Err(Error::NoEntry) => Ok(()),
            Err(e) => Err(format!("{OS} did not forget the key: {e}")),
        }
    }

    #[cfg(all(test, windows))]
    pub fn named_and_local(item: &str) -> bool {
        let a = entry(item).and_then(|e| e.get_attributes()).unwrap_or_default();
        a.get("target_name").map(String::as_str) == Some(item) && a.get("persistence").map(String::as_str) == Some("Local")
    }
}

/// Linux: the keyring project reaches the Secret Service only over D-Bus (a second async stack and a running
/// service a headless machine lacks), and the kernel keyring forgets at reboot. Neither is a quiet win yet.
#[cfg(not(any(windows, target_os = "macos")))]
mod store {
    pub const NOT_YET: &str = "Keys are not supported on Linux yet. A model on this computer needs no key.";

    pub fn set(_item: &str, _key: &[u8]) -> Result<(), String> {
        Err(NOT_YET.into())
    }

    pub fn get(_item: &str) -> Option<String> {
        None
    }

    pub fn delete(_item: &str) -> Result<(), String> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FAKE: &str = "test-key-not-real";

    #[test]
    fn an_item_is_named_for_pointcast_and_its_provider_and_an_empty_key_is_not_kept() {
        assert_eq!(item("openrouter"), "Pointcast assistant/openrouter");
        assert!(Secret::new("  ").is_empty() && !Secret::new(FAKE).is_empty());
        assert_eq!(set("pointcast-test-empty", Secret::new(" ")), Err("There is no key to keep.".to_owned()));
        assert!(!has("pointcast-test-empty"));
    }

    /// A throwaway item of this test's own in the real store, removed however the test ends.
    #[cfg(any(windows, target_os = "macos"))]
    struct Throwaway(String);

    #[cfg(any(windows, target_os = "macos"))]
    impl Drop for Throwaway {
        fn drop(&mut self) {
            let _ = delete(&self.0);
        }
    }

    #[cfg(any(windows, target_os = "macos"))]
    #[test]
    fn a_key_is_kept_in_the_systems_store_read_back_replaced_and_forgotten() {
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let t = Throwaway(format!("pointcast-test-{}-{nanos}", std::process::id()));
        assert!(!has(&t.0) && get(&t.0).is_none());
        assert_eq!(set(&t.0, Secret::new(format!(" {FAKE}\n"))), Ok(()));
        assert!(has(&t.0));
        #[cfg(windows)]
        assert!(store::named_and_local(&item(&t.0)), "one item named for Pointcast, kept on this computer only");
        assert_eq!(get(&t.0).unwrap().expose(), FAKE, "kept without the space and the line end of a paste");
        assert_eq!(set(&t.0, Secret::new("test-key-not-real-2")), Ok(()));
        assert_eq!(get(&t.0).unwrap().expose(), "test-key-not-real-2");
        assert_eq!(delete(&t.0), Ok(()));
        assert!(!has(&t.0));
        assert_eq!(delete(&t.0), Ok(()), "forgetting twice is not an error");
    }

    #[cfg(not(any(windows, target_os = "macos")))]
    #[test]
    fn on_linux_a_key_is_refused_in_words_and_none_is_ever_found() {
        let said = set("pointcast-test-linux", Secret::new(FAKE)).unwrap_err();
        assert!(said.contains("not supported on Linux yet"), "{said}");
        assert!(!has("pointcast-test-linux") && get("pointcast-test-linux").is_none());
        assert_eq!(delete("pointcast-test-linux"), Ok(()));
    }

    #[test]
    fn only_the_http_line_reads_a_key() {
        // Built from pieces, so this file's own test does not count as a reader.
        let reader = [".exp", "ose("].concat();
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src").join("assist");
        let mut read = 0;
        for e in std::fs::read_dir(&dir).unwrap().flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            let text = std::fs::read_to_string(e.path()).unwrap_or_default();
            read += 1;
            let allowed = name == "http.rs" || name == "vault.rs";
            assert!(allowed || !text.contains(&reader), "{name} reads a key");
            if name == "http.rs" {
                assert!(text.contains(&reader), "the scan finds the one reader");
            }
        }
        assert!(read >= 9, "the scan read the folder it meant to: {read}");
    }
}
