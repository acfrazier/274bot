//! Generation-scoped transport revocation and physical retirement.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use super::park::Park;

#[derive(Debug)]
struct CancelState {
    generation: u64,
    fired: AtomicBool,
    parent: Option<Arc<CancelState>>,
    park: Arc<Park>,
}

/// The source of exact-generation revocation for one slot lifetime.
#[derive(Debug)]
pub struct CancelSource {
    state: Arc<CancelState>,
    next_generation: AtomicU64,
}

impl CancelSource {
    /// Create an independent slot lifetime with one shared park.
    pub fn new(park: Arc<Park>) -> Self {
        Self {
            state: Arc::new(CancelState {
                generation: 0,
                fired: AtomicBool::new(false),
                parent: None,
                park,
            }),
            next_generation: AtomicU64::new(1),
        }
    }

    /// Issue one attempt token; an exhausted generation cannot be reused.
    pub fn child(&self) -> Option<CancelToken> {
        let generation = self
            .next_generation
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
                current.checked_add(1)
            })
            .ok()?;
        Some(CancelToken(Arc::new(CancelState {
            generation,
            fired: AtomicBool::new(false),
            parent: Some(Arc::clone(&self.state)),
            park: Arc::clone(&self.state.park),
        })))
    }

    /// Revoke all children of this source and wake its slot after publication.
    pub fn cancel(&self) {
        self.state.park.publish_revocation(&self.state.fired);
    }
}

/// Exact-generation transport revocation, independent of a socket clone.
#[derive(Debug, Clone)]
pub struct CancelToken(Arc<CancelState>);

impl CancelToken {
    /// The generation assigned when this attempt was admitted.
    pub fn generation(&self) -> u64 {
        self.0.generation
    }
    /// Compare physical cancellation cells, not reused generation numbers.
    pub fn same_attempt(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }

    /// Revoke this attempt once and wake its slot after publication.
    pub fn cancel(&self) {
        self.0.park.publish_revocation(&self.0.fired);
    }

    /// Observe own revocation or the slot-lifetime parent's revocation.
    pub fn is_cancelled(&self) -> bool {
        self.0.fired.load(Ordering::Acquire)
            || self
                .0
                .parent
                .as_ref()
                .is_some_and(|parent| parent.fired.load(Ordering::Acquire))
    }

    /// Borrow the attempt's park without constructing a new wake channel.
    pub fn park(&self) -> &Arc<Park> {
        &self.0.park
    }
}

/// Physical release is distinct from queue-admission settlement and sealing.
#[derive(Debug)]
pub struct AttemptRetire {
    generation: u64,
    acknowledged: AtomicBool,
    #[cfg(any(test, feature = "test-support"))]
    acknowledgements: AtomicU64,
}

impl AttemptRetire {
    /// Create one retirement cell for one immutable attempt generation.
    pub fn new(generation: u64) -> Self {
        Self {
            generation,
            acknowledged: AtomicBool::new(false),
            #[cfg(any(test, feature = "test-support"))]
            acknowledgements: AtomicU64::new(0),
        }
    }

    /// The exact generation bound to this physical resource.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Acknowledge after actual close/drop or after committed detachment.
    /// Returns `true` only for the unique first acknowledgement.
    pub fn ack(&self) -> bool {
        let first = !self.acknowledged.swap(true, Ordering::AcqRel);
        #[cfg(any(test, feature = "test-support"))]
        if first {
            self.acknowledgements.fetch_add(1, Ordering::Relaxed);
        }
        first
    }

    /// Whether physical custody has been released or committed to a session.
    pub fn acknowledged(&self) -> bool {
        self.acknowledged.load(Ordering::Acquire)
    }

    /// Count unique acknowledgements for scheduled owner-boundary tests.
    #[cfg(any(test, feature = "test-support"))]
    pub fn ack_count(&self) -> u64 {
        self.acknowledgements.load(Ordering::Relaxed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_generation_and_parent_revocation_are_distinct() {
        let source = CancelSource::new(Arc::new(Park::new()));
        let old = source.child().unwrap();
        let next = source.child().unwrap();
        assert_ne!(old.generation(), next.generation());
        old.cancel();
        assert!(old.is_cancelled());
        assert!(!next.is_cancelled());
        source.cancel();
        assert!(next.is_cancelled());
    }

    #[test]
    fn retirement_cell_does_not_release_later_same_slot_attempt() {
        let old = AttemptRetire::new(5);
        let next = AttemptRetire::new(6);
        assert!(old.ack());
        assert!(!old.ack());
        assert_eq!(old.ack_count(), 1);
        assert!(!next.acknowledged());
        assert!(next.ack());
        assert_eq!(next.ack_count(), 1);
    }
}
