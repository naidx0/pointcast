//! The grant: the computer tools exist only while an Ask turn holds one. The turn makes it when it starts and drops it
//! when it ends; it also runs out by itself, after at most ten minutes, so a turn that never ends can't keep it.
//!
//! Each grant has a secret of its own, 32 bytes of the OS's randomness. The turn's MCP client carries it as its key,
//! and only a request carrying the live grant's secret reaches the computer tools (`mcp.rs`): this install's general
//! token, which coding agents hold, never does. A new grant's secret is new, so an earlier turn's key opens nothing.
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// The longest a grant lives, whatever the turn asked for.
pub const MAX_GRANT: Duration = Duration::from_secs(600);

static NEXT: AtomicU64 = AtomicU64::new(1);

/// Where the live grant is kept: one for the app (`computer::desk()`), a fresh one per test.
#[derive(Clone, Default)]
pub struct Grants(Arc<Mutex<Option<Live>>>);

#[derive(Clone, Copy)]
struct Live {
    id: u64,
    until: Instant,
    /// None when the OS gave no randomness: then no request can match it.
    secret: Option<[u8; 32]>,
}

/// A live grant. Dropping it ends it.
#[must_use = "the grant ends as soon as it is dropped"]
pub struct Grant {
    grants: Grants,
    id: u64,
    secret: Option<[u8; 32]>,
}

/// 32 bytes from the OS's randomness, or none: never a guessable stand-in.
fn mint() -> Option<[u8; 32]> {
    let mut b = [0u8; 32];
    getrandom::getrandom(&mut b).ok().map(|_| b)
}

fn hex(b: &[u8; 32]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// Whether `a` and `b` are the same bytes. Every byte is compared, so the time taken does not say how much of a guess
/// was right.
pub fn same(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

impl Grants {
    fn lock(&self) -> std::sync::MutexGuard<'_, Option<Live>> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// A grant for `ttl` (at most `MAX_GRANT`), with a secret of its own. It replaces any earlier one: a new turn takes
    /// over, and the earlier turn's secret opens nothing from now on.
    pub fn grant(&self, ttl: Duration) -> Grant {
        let id = NEXT.fetch_add(1, Ordering::Relaxed);
        let until = Instant::now() + ttl.min(MAX_GRANT);
        let secret = mint();
        *self.lock() = Some(Live { id, until, secret });
        Grant { grants: self.clone(), id, secret }
    }

    /// End the live grant now, whoever holds it: the stop key.
    pub fn end(&self) {
        *self.lock() = None;
    }

    /// Whether a grant is held and has not run out.
    pub fn live(&self) -> bool {
        self.lock().is_some_and(|l| Instant::now() < l.until)
    }

    /// Whether grant `id` is the live one and has not run out.
    pub fn holds(&self, id: u64) -> bool {
        self.lock().is_some_and(|l| l.id == id && Instant::now() < l.until)
    }

    /// The live grant whose secret `presented` is, if any. Compared in constant time.
    pub fn by_secret(&self, presented: &[u8]) -> Option<u64> {
        let l = (*self.lock())?;
        let secret = l.secret?;
        (Instant::now() < l.until && same(presented, hex(&secret).as_bytes())).then_some(l.id)
    }

    /// The live grant's id, if one is held and has not run out.
    #[cfg(test)]
    pub fn live_id(&self) -> Option<u64> {
        self.lock().filter(|l| Instant::now() < l.until).map(|l| l.id)
    }
}

impl Grant {
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn id(&self) -> u64 {
        self.id
    }

    /// The key a request carries to reach the computer tools under this grant, as 64 hex digits. None when the OS gave
    /// no randomness, and then nothing reaches them.
    pub fn secret(&self) -> Option<String> {
        self.secret.as_ref().map(hex)
    }

    /// Whether this grant is still the live one.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn live(&self) -> bool {
        self.grants.holds(self.id)
    }
}

impl Drop for Grant {
    fn drop(&mut self) {
        let mut g = self.grants.lock();
        if g.is_some_and(|l| l.id == self.id) {
            *g = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_grant_lives_until_dropped_or_run_out_and_never_past_ten_minutes() {
        let grants = Grants::default();
        assert!(!grants.live());
        let g = grants.grant(Duration::from_secs(60));
        assert!(grants.live() && g.live());
        drop(g);
        assert!(!grants.live(), "dropped at the end of the turn");

        let short = grants.grant(Duration::from_millis(30));
        assert!(grants.live());
        std::thread::sleep(Duration::from_millis(60));
        assert!(!grants.live() && !short.live(), "ran out by itself");
        drop(short);

        let long = grants.grant(Duration::from_secs(24 * 3600));
        let until = grants.0.lock().unwrap().unwrap().until;
        assert!(until <= Instant::now() + MAX_GRANT, "capped at ten minutes");
        drop(long);
    }

    #[test]
    fn a_new_turn_takes_over_and_the_old_grant_dropping_late_does_not_end_it() {
        let grants = Grants::default();
        let first = grants.grant(Duration::from_secs(60));
        let second = grants.grant(Duration::from_secs(60));
        assert!(!first.live() && second.live());
        drop(first);
        assert!(grants.live(), "the second turn still holds it");
        drop(second);
        assert!(!grants.live());
    }

    #[test]
    fn each_grant_has_a_secret_of_its_own_and_only_the_live_ones_opens_anything() {
        let grants = Grants::default();
        let a = grants.grant(Duration::from_secs(60));
        let key_a = a.secret().expect("the OS gave randomness");
        assert!(key_a.len() == 64 && key_a.bytes().all(|c| c.is_ascii_hexdigit()), "32 bytes, as hex");
        assert_eq!(grants.by_secret(key_a.as_bytes()), Some(a.id()));
        for wrong in [key_a[..63].to_string(), format!("{key_a}0"), String::new(), key_a.to_uppercase()] {
            assert!(grants.by_secret(wrong.as_bytes()).is_none(), "a near miss opens nothing");
        }
        // A new turn: a new secret, and the old one opens nothing, even before the old grant is dropped.
        let b = grants.grant(Duration::from_secs(60));
        let key_b = b.secret().unwrap();
        assert!(key_a != key_b, "a new grant's secret is new");
        assert!(grants.by_secret(key_a.as_bytes()).is_none());
        assert_eq!(grants.by_secret(key_b.as_bytes()), Some(b.id()));
        assert!(grants.holds(b.id()) && !grants.holds(a.id()));
        drop(a);
        assert_eq!(grants.by_secret(key_b.as_bytes()), Some(b.id()), "the old grant dropping late leaves the new key");
        // Ended by the stop key, or run out: nothing opens.
        grants.end();
        assert!(grants.by_secret(key_b.as_bytes()).is_none() && !grants.holds(b.id()));
        let short = grants.grant(Duration::from_millis(20));
        let key = short.secret().unwrap();
        std::thread::sleep(Duration::from_millis(40));
        assert!(grants.by_secret(key.as_bytes()).is_none() && !grants.holds(short.id()));
    }

    #[test]
    fn same_compares_whole_byte_strings() {
        assert!(same(b"abc", b"abc") && same(b"", b""));
        assert!(!same(b"abc", b"abd") && !same(b"abc", b"ab") && !same(b"ab", b"abc"));
    }
}
