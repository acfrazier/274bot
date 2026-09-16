//! Per-slot native input identity and eligibility.
//!
//! Shared between `SlotScript` (publisher / revoker) and host `SlotInput`
//! (consumer). Contains only the current identity, eligibility, and the
//! mutex that linearizes revoke against consume. It is not mouse policy
//! and not a global registry.
//!
//! Consume must hold [`NativeInputAuthority::lock`] across validation,
//! shell application, and `latch_click`, then drop it before `mainloop`
//! or an isolate join. If the consumer also locks a mouse queue, this
//! mutex is first.

use std::sync::{Arc, Mutex, MutexGuard};

/// Snapshot of the live identity and whether script input may consume.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NativeInputPermit {
    identity: u64,
    eligible: bool,
}

impl NativeInputPermit {
    pub fn identity(&self) -> u64 {
        self.identity
    }

    pub fn eligible(&self) -> bool {
        self.eligible
    }
}

/// Slot-local authority for native script input.
pub struct NativeInputAuthority {
    inner: Mutex<NativeInputPermit>,
}

impl NativeInputAuthority {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            inner: Mutex::new(NativeInputPermit {
                identity: 0,
                eligible: false,
            }),
        })
    }

    /// Consume-side guard. Hold only across apply + latch.
    pub fn lock(&self) -> MutexGuard<'_, NativeInputPermit> {
        self.inner.lock().expect("native input authority")
    }

    /// After a live instance exists: new identity, eligible.
    pub fn publish_live(&self) -> u64 {
        let mut g = self.lock();
        g.identity = g.identity.wrapping_add(1);
        g.eligible = true;
        g.identity
    }

    /// Pause / Stop / replacement / watchdog / logout / error.
    /// Bumps identity so Resume cannot replay queued events.
    pub fn revoke(&self) {
        let mut g = self.lock();
        g.eligible = false;
        g.identity = g.identity.wrapping_add(1);
    }

    /// Resume after Pause: current (already bumped) identity becomes eligible.
    pub fn resume(&self) {
        let mut g = self.lock();
        g.eligible = true;
    }

    /// Observe / watchdog sync. Going live does not bump; leaving live
    /// revokes once so queued events cannot replay.
    pub fn sync_live(&self, live: bool) {
        let mut g = self.lock();
        if live {
            g.eligible = true;
        } else if g.eligible {
            g.eligible = false;
            g.identity = g.identity.wrapping_add(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::NativeInputAuthority;

    #[test]
    fn publish_live_enables_a_fresh_identity() {
        let a = NativeInputAuthority::new();
        assert!(!a.lock().eligible());
        assert_eq!(a.lock().identity(), 0);
        let id = a.publish_live();
        assert!(id > 0);
        let p = a.lock();
        assert!(p.eligible());
        assert_eq!(p.identity(), id);
    }

    #[test]
    fn revoke_disables_and_bumps_so_resume_cannot_replay() {
        let a = NativeInputAuthority::new();
        let old = a.publish_live();
        a.revoke();
        {
            let p = a.lock();
            assert!(!p.eligible());
            assert_ne!(p.identity(), old);
        }
        a.resume();
        let p = a.lock();
        assert!(p.eligible());
        assert_ne!(p.identity(), old);
    }

    #[test]
    fn sync_live_false_bumps_once() {
        let a = NativeInputAuthority::new();
        let id = a.publish_live();
        a.sync_live(false);
        let after = a.lock().identity();
        assert_ne!(after, id);
        a.sync_live(false);
        assert_eq!(a.lock().identity(), after);
        a.sync_live(true);
        assert!(a.lock().eligible());
        assert_eq!(a.lock().identity(), after);
    }
}
