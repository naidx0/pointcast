//! The grant: the computer tools exist only while an Ask turn holds one. The turn makes it when it starts and drops it
//! when it ends; it also runs out by itself, after at most ten minutes, so a turn that never ends can't keep it.
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
}

/// A live grant. Dropping it ends it.
#[must_use = "the grant ends as soon as it is dropped"]
pub struct Grant {
    grants: Grants,
    id: u64,
}

impl Grants {
    /// A grant for `ttl` (at most `MAX_GRANT`). It replaces any earlier one: a new turn takes over.
    pub fn grant(&self, ttl: Duration) -> Grant {
        let id = NEXT.fetch_add(1, Ordering::Relaxed);
        let until = Instant::now() + ttl.min(MAX_GRANT);
        *self.0.lock().unwrap_or_else(|e| e.into_inner()) = Some(Live { id, until });
        Grant { grants: self.clone(), id }
    }

    /// End the live grant now, whoever holds it: the stop key.
    pub fn end(&self) {
        *self.0.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }

    /// Whether a grant is held and has not run out.
    pub fn live(&self) -> bool {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).is_some_and(|l| Instant::now() < l.until)
    }
}

impl Grant {
    /// Whether this grant is still the live one.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn live(&self) -> bool {
        let g = self.grants.0.lock().unwrap_or_else(|e| e.into_inner());
        g.is_some_and(|l| l.id == self.id && Instant::now() < l.until)
    }
}

impl Drop for Grant {
    fn drop(&mut self) {
        let mut g = self.grants.0.lock().unwrap_or_else(|e| e.into_inner());
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
}
