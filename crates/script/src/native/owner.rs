//! Revocation identities shared with queued host work. Revocation is independent
//! of a frame context and happens before invoking any machine cleanup.
use api::selected::RunKey;
use std::num::NonZeroU64;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

pub(crate) struct Owner {
    pub run: RunKey,
    pub id: NonZeroU64,
    revoked: AtomicBool,
    quiet: AtomicU64,
    walk: AtomicU64,
    interaction: AtomicU64,
    batch: [AtomicU64; 5],
}

impl Owner {
    pub fn new(run: RunKey, id: NonZeroU64) -> Arc<Self> {
        Arc::new(Self {
            run,
            id,
            revoked: AtomicBool::new(false),
            quiet: AtomicU64::new(0),
            walk: AtomicU64::new(0),
            interaction: AtomicU64::new(0),
            batch: std::array::from_fn(|_| AtomicU64::new(0)),
        })
    }

    pub fn live(&self) -> bool {
        !self.revoked.load(Ordering::Acquire)
    }

    pub fn revoke(&self) {
        self.revoked.store(true, Ordering::Release);
        self.quiet.store(0, Ordering::Release);
        self.walk.store(0, Ordering::Release);
        self.interaction.store(0, Ordering::Release);
        for slot in &self.batch {
            slot.store(0, Ordering::Release);
        }
    }

    pub fn acquire_quiet(&self, request: NonZeroU64) -> bool {
        self.live()
            && self
                .quiet
                .compare_exchange(0, request.get(), Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
    }

    pub fn release_quiet(&self, request: NonZeroU64) {
        let _ = self
            .quiet
            .compare_exchange(request.get(), 0, Ordering::AcqRel, Ordering::Acquire);
    }

    pub fn quiet(&self, request: NonZeroU64) -> bool {
        self.live() && self.quiet.load(Ordering::Acquire) == request.get()
    }

    pub fn set_walk(&self, request: NonZeroU64) {
        self.walk.store(request.get(), Ordering::Release);
    }

    pub fn cancel_walk(&self, request: NonZeroU64) {
        let _ = self
            .walk
            .compare_exchange(request.get(), 0, Ordering::AcqRel, Ordering::Acquire);
    }

    pub fn walk_live(&self, request: NonZeroU64) -> bool {
        self.live() && self.walk.load(Ordering::Acquire) == request.get()
    }

    pub fn set_interaction(&self, request: NonZeroU64) {
        self.interaction.store(request.get(), Ordering::Release);
    }

    pub fn cancel_interaction(&self, request: NonZeroU64) {
        let _ = self.interaction.compare_exchange(
            request.get(),
            0,
            Ordering::AcqRel,
            Ordering::Acquire,
        );
        for slot in &self.batch {
            let _ = slot.compare_exchange(request.get(), 0, Ordering::AcqRel, Ordering::Acquire);
        }
    }

    /// Reserve a consecutive set of ids in the shared five-slot authority.
    /// Any partial reservation is released if capacity or owner liveness races.
    pub fn acquire_batch(&self, first: NonZeroU64, count: usize) -> bool {
        if count == 0 || count > self.batch.len() || !self.live() {
            return false;
        }
        let mut acquired = 0;
        while acquired < count {
            let Some(request) = first
                .get()
                .checked_add(acquired as u64)
                .and_then(NonZeroU64::new)
            else {
                self.release_batch(first, acquired);
                return false;
            };
            let reserved = self.live()
                && self.batch.iter().any(|slot| {
                    slot.compare_exchange(0, request.get(), Ordering::AcqRel, Ordering::Acquire)
                        .is_ok()
                });
            if !reserved {
                self.release_batch(first, acquired);
                return false;
            }
            acquired += 1;
        }
        if !self.live() {
            self.release_batch(first, acquired);
            return false;
        }
        true
    }

    fn release_batch(&self, first: NonZeroU64, count: usize) {
        for offset in 0..count {
            let Some(request) = first
                .get()
                .checked_add(offset as u64)
                .and_then(NonZeroU64::new)
            else {
                break;
            };
            self.cancel_interaction(request);
        }
    }

    /// Number of free reservations shared by batches and slot-exact drops.
    pub fn batch_free(&self) -> usize {
        if !self.live() {
            return 0;
        }
        self.batch
            .iter()
            .filter(|slot| slot.load(Ordering::Acquire) == 0)
            .count()
    }

    pub fn batch_live(&self, request: NonZeroU64) -> bool {
        self.live()
            && self
                .batch
                .iter()
                .any(|slot| slot.load(Ordering::Acquire) == request.get())
    }

    pub fn interaction_live(&self, request: NonZeroU64) -> bool {
        self.live()
            && (self.interaction.load(Ordering::Acquire) == request.get()
                || self.batch_live(request))
    }
}

impl Drop for Owner {
    fn drop(&mut self) {
        self.revoke();
    }
}
