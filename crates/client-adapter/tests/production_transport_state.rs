//! Production-path tests for Park writer coordination and handoff release.
//!
//! These tests call the adapter's real state machines. The bounded models in
//! the fork are supplementary and do not supply this production evidence.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
#[cfg(feature = "test-support")]
use std::sync::mpsc;
use std::sync::Arc;
#[cfg(feature = "test-support")]
use std::thread;
use std::time::{Duration, Instant};

use client_adapter::cancel::{AttemptRetire, CancelSource};
use client_adapter::control::BindingId;
use client_adapter::handoff::{
    HandoffCell, HandoffIdentity, HandoffKey, HandoffPhase, QueueReleaseProof, RevocationReason,
    TakeReturnError, WorkerFailure, WorkerFinish,
};
use client_adapter::park::Park;
#[cfg(feature = "test-support")]
use client_adapter::park::{ParkOutcome, WriterBinding};
use mio::{Poll, Token, Waker};

#[cfg(feature = "test-support")]
#[test]
fn g1_m_production_park_writer_marker_wakes_waiter() {
    let park = Arc::new(Park::new());
    let writer = WriterBinding {
        connection: 7,
        binding: 11,
    };
    let space_available = Arc::new(AtomicBool::new(false));
    let (ready_checked, observed_ready_check) = mpsc::sync_channel(1);

    let worker_park = Arc::clone(&park);
    let worker_space = Arc::clone(&space_available);
    let waiter = thread::spawn(move || {
        worker_park.wait(
            Instant::now() + Duration::from_secs(5),
            Instant::now,
            || false,
            || false,
            || false,
            || {
                let ready = worker_space.load(Ordering::Acquire);
                if !ready {
                    // The writer marker is armed, and this check still holds K2.
                    // The publisher must wait for the condvar's atomic K2 release.
                    let _ = ready_checked.try_send(());
                }
                ready.then_some(())
            },
            false,
            Some(writer),
        )
    });

    observed_ready_check.recv().unwrap();

    space_available.store(true, Ordering::Release);
    park.publish_freed_space(writer);
    let epoch = park.epoch();
    let outcome = waiter.join().unwrap();

    assert_eq!(epoch, 1, "an armed writer advances Park's K2 epoch");
    assert_eq!(outcome, ParkOutcome::Ready(()));
}

struct ReleaseObservation {
    retirement: Arc<AttemptRetire>,
    drops: AtomicUsize,
    acknowledged_during_drop: AtomicBool,
}

struct ObservedResource(Arc<ReleaseObservation>);

impl Drop for ObservedResource {
    fn drop(&mut self) {
        self.0
            .acknowledged_during_drop
            .store(self.0.retirement.acknowledged(), Ordering::SeqCst);
        self.0.drops.fetch_add(1, Ordering::SeqCst);
    }
}

#[test]
fn g2_i_production_handoff_publishes_proof_before_physical_release() {
    let poll = Poll::new().unwrap();
    let waker = Arc::new(Waker::new(poll.registry(), Token(0)).unwrap());
    let handoff = HandoffCell::new(waker);
    let park = Arc::new(Park::new());
    let cancel_source = CancelSource::new(Arc::clone(&park));
    let token = cancel_source.child().unwrap();
    let binding = BindingId {
        incarnation: 1,
        connection: 2,
        binding: 3,
        attempt: 4,
    };
    let key = HandoffKey {
        identity: HandoffIdentity {
            generation: token.generation(),
            binding,
        },
        transfer_seq: handoff.next_transfer_seq().unwrap(),
    };
    let retirement = Arc::new(AttemptRetire::new(token.generation()));
    let proof = Arc::new(QueueReleaseProof::new(binding, key.transfer_seq));
    let observation = Arc::new(ReleaseObservation {
        retirement: Arc::clone(&retirement),
        drops: AtomicUsize::new(0),
        acknowledged_during_drop: AtomicBool::new(false),
    });
    let submitted_proof = handoff
        .submit(
            key,
            Arc::clone(&proof),
            token,
            Arc::clone(&retirement),
            Instant::now() + Duration::from_secs(5),
            ObservedResource(Arc::clone(&observation)),
        )
        .unwrap();

    assert!(Arc::ptr_eq(&proof, &submitted_proof));
    let work = handoff.worker_claim().unwrap();
    assert_eq!(handoff.phase(), HandoffPhase::Running);
    let revoked_proof = handoff
        .revoke(key, RevocationReason::Deadline)
        .unwrap()
        .unwrap();

    assert!(Arc::ptr_eq(&proof, &revoked_proof));
    assert_eq!(proof.identity(), binding);
    assert_eq!(proof.transfer_seq(), key.transfer_seq);
    assert!(proof.snapshot().is_some());
    assert!(!retirement.acknowledged());
    assert!(!handoff.can_admit());
    assert_eq!(observation.drops.load(Ordering::SeqCst), 0);

    assert_eq!(
        handoff
            .worker_failure(work, WorkerFailure::Handshake)
            .unwrap(),
        WorkerFinish::Revoked
    );
    assert_eq!(observation.drops.load(Ordering::SeqCst), 1);
    assert!(
        !observation.acknowledged_during_drop.load(Ordering::SeqCst),
        "physical acknowledgement must follow the resource destructor"
    );
    assert!(retirement.acknowledged());
    assert!(handoff.can_admit());
    assert_eq!(handoff.phase(), HandoffPhase::Released);
    assert!(matches!(
        handoff.take_return(key),
        Err(TakeReturnError::Revoked(_))
    ));
}
