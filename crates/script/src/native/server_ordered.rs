//! One native reservation and one terminal completion per ordered request.

use api::interact::server_ordered::{ServerOrderedDispatch, ServerOrderedRequest};
use api::quest_progress::EvidenceStamp;

use super::ledger::{HostAction, HostEffect};
use super::{ActionContext, ActionError};
use crate::surface::ServerOrderedEnvelope;

/// Dispatch evidence retains the existing run/session envelope outside the receipt.
pub struct ServerOrderedCompletion {
    pub evidence: EvidenceStamp,
    pub dispatch: ServerOrderedDispatch,
}

impl ActionContext<'_> {
    /// Admit named native content, without an external-script declaration.
    /// The host revalidates the complete request against its drain snapshot.
    pub fn emit_server_ordered(
        &mut self,
        request: ServerOrderedRequest,
    ) -> Result<u64, ActionError> {
        request.validate_shape(self.pin.revision)?;
        self.emit_server_ordered_envelope(ServerOrderedEnvelope::content(request))
    }

    pub(crate) fn emit_server_ordered_envelope(
        &mut self,
        envelope: ServerOrderedEnvelope,
    ) -> Result<u64, ActionError> {
        let owner = self.owner()?;
        let ledger = self.ledger.as_ref().expect("owner checked");
        // Only this native runtime's emissions charge this observed tick.
        // Foreign packet observations never enter TickBudget.
        if self.budget.events != 0
            || ledger.outbox.len() == ledger.outbox.capacity()
            || owner.batch_free() == 0
            || ledger
                .server_ordered_request
                .is_some_and(|request| owner.batch_live(request))
        {
            return Err(ActionError::BudgetExhausted);
        }
        let request_id = ledger.id_range(1)?;
        if !owner.acquire_batch(request_id, 1) {
            return Err(ActionError::BudgetExhausted);
        }
        if !owner.live() {
            owner.cancel_interaction(request_id);
            return Err(ActionError::Cancelled);
        }
        if !self.budget.batch() {
            owner.cancel_interaction(request_id);
            return Err(ActionError::BudgetExhausted);
        }
        let ledger = self.ledger.as_mut().expect("owner checked");
        ledger.commit_id_range(request_id, 1);
        ledger.server_ordered = None;
        ledger.server_ordered_request = Some(request_id);
        ledger.outbox.push(HostAction {
            owner,
            request_id,
            batch: 0,
            effect: HostEffect::ServerOrdered(envelope),
            observed_walk_outcome_seq: self.observed_walk_outcome_seq,
            bank_deposit: None,
        });
        Ok(request_id.get())
    }

    /// Read the terminal dispatch for this owner. Server outcomes are separate.
    pub fn server_ordered_receipt(&self, request_id: u64) -> Option<&ServerOrderedDispatch> {
        let ledger = self.ledger.as_ref()?;
        let owner = ledger.owner.as_ref()?;
        let completion = ledger.server_ordered.as_deref()?;
        (owner.run == self.run()
            && owner.id.get() == self.action_id
            && completion.evidence.run == self.run()
            && completion.dispatch.request_id == request_id)
            .then_some(&completion.dispatch)
    }
}

#[cfg(test)]
mod tests {
    use super::super::ledger::{Ledger, TickBudget};
    use super::super::owner::Owner;
    use super::super::{RetainedMemory, SnapshotView, WalkCancelReason};
    use super::*;
    use api::interact::server_ordered::{CloseModalReason, ExpectedModalRoot, ServerOrderedError};
    use api::selected::{RunKey, SelectedPin};
    use std::num::NonZeroU64;
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    fn request() -> ServerOrderedRequest {
        ServerOrderedRequest::CloseModal {
            expected_root: ExpectedModalRoot::Bank,
            reason: CloseModalReason::TerminalBankExit,
        }
    }

    struct Frame {
        ledger: Option<Box<Ledger>>,
        budget: TickBudget,
        retained: RetainedMemory,
        pin: Arc<SelectedPin>,
        evidence: EvidenceStamp,
        owner: Arc<Owner>,
    }

    impl Frame {
        fn new() -> Self {
            let selected =
                api::game_data::for_revision(api::selected::ClientRevision::R289).unwrap();
            let evidence = EvidenceStamp {
                run: RunKey {
                    slot: 1,
                    run: 1,
                    session: 1,
                },
                tick: 1,
                sequence: 1,
            };
            let mut ledger = Box::<Ledger>::default();
            let owner = Owner::new(evidence.run, ledger.next_id().unwrap());
            ledger.owner = Some(Arc::clone(&owner));
            let mut budget = TickBudget::default();
            budget.observe(evidence.tick);
            Self {
                ledger: Some(ledger),
                budget,
                retained: RetainedMemory::default(),
                pin: selected.selected_pin().unwrap(),
                evidence,
                owner,
            }
        }

        fn with<R>(&mut self, f: impl FnOnce(&mut ActionContext<'_>) -> R) -> R {
            let mut cx = ActionContext {
                condition: None,
                poison_unknown_pulse: u8::MAX,
                evidence: self.evidence,
                pin: &self.pin,
                snapshot: SnapshotView::new(None, self.evidence),
                retained: &mut self.retained,
                action_id: self.owner.id.get(),
                observed_walk_outcome_seq: 0,
                observed_walk_cancel_reason: WalkCancelReason::None,
                active_now: Duration::ZERO,
                wall_now: Instant::now(),
                ledger: &mut self.ledger,
                pending_note_progress: None,
                budget: &mut self.budget,
                eligible: true,
            };
            f(&mut cx)
        }

        fn next_tick(&mut self) {
            self.evidence.tick += 1;
            self.evidence.sequence += 1;
            self.budget.observe(self.evidence.tick);
        }

        fn complete(&mut self, authority: &super::super::HostAuthority) {
            self.ledger.as_mut().unwrap().complete_server_ordered(
                authority,
                ServerOrderedCompletion {
                    evidence: self.evidence,
                    dispatch: ServerOrderedDispatch::refused(
                        authority.request_id().get(),
                        self.evidence.tick,
                        1,
                        0,
                        ServerOrderedError::Cancelled,
                    ),
                },
            );
        }
    }

    #[test]
    fn ordered_request_reserves_one_identity_slot_outbox_and_completion() {
        let mut frame = Frame::new();
        let next = frame.ledger.as_ref().unwrap().id_range(1).unwrap().get();
        let id = frame.with(|cx| cx.emit_server_ordered(request())).unwrap();
        assert_eq!(id, next);
        assert_eq!(frame.owner.batch_free(), 4);
        assert_eq!(frame.ledger.as_ref().unwrap().outbox.len(), 1);
        assert_eq!(
            frame.ledger.as_ref().unwrap().id_range(1).unwrap().get(),
            next + 1,
        );
        let action = frame.ledger.as_mut().unwrap().outbox.pop().unwrap();
        let authority = action.authority();
        assert!(action.live());
        frame.complete(&authority);
        assert_eq!(frame.owner.batch_free(), 5);
        assert!(!authority.live());
        let completion = frame
            .ledger
            .as_ref()
            .unwrap()
            .server_ordered
            .as_deref()
            .unwrap();
        assert_eq!(completion.dispatch.request_id, id);
        let retained = completion as *const ServerOrderedCompletion;
        frame.complete(&authority);
        assert_eq!(
            frame
                .ledger
                .as_ref()
                .unwrap()
                .server_ordered
                .as_deref()
                .unwrap() as *const ServerOrderedCompletion,
            retained,
            "a duplicate terminal callback must not allocate a replacement",
        );
        frame.with(|cx| {
            assert_eq!(cx.server_ordered_receipt(id).unwrap().request_id, id);
            assert!(cx.server_ordered_receipt(id + 1).is_none());
        });
    }

    #[test]
    fn ordered_cancellation_and_refusal_release_repeated_requests_without_leaks() {
        let mut frame = Frame::new();
        for round in 0..32 {
            let id = frame.with(|cx| cx.emit_server_ordered(request())).unwrap();
            let action = frame.ledger.as_mut().unwrap().outbox.pop().unwrap();
            let authority = action.authority();
            if round % 2 == 0 {
                frame.with(|cx| cx.cancel_request(id));
                assert!(!authority.live());
            }
            frame.complete(&authority);
            assert_eq!(frame.owner.batch_free(), 5);
            assert!(frame
                .ledger
                .as_ref()
                .unwrap()
                .server_ordered_request
                .is_none());
            frame.with(|cx| {
                let receipt = cx.server_ordered_receipt(id).unwrap();
                assert_eq!(receipt.dispatched_prefix, 0);
                assert_eq!(receipt.wire_events, 0);
                assert_eq!(receipt.terminal, Some(ServerOrderedError::Cancelled));
            });
            frame.next_tick();
        }
    }

    #[test]
    fn ordered_batch_allowance_does_not_replenish_on_extra_wakes_or_cancellation() {
        let mut frame = Frame::new();
        let id = frame.with(|cx| cx.emit_server_ordered(request())).unwrap();
        frame.with(|cx| cx.cancel_request(id));
        for _ in 0..3 {
            frame.budget.observe(frame.evidence.tick);
            assert_eq!(
                frame.with(|cx| cx.emit_server_ordered(request())),
                Err(ActionError::BudgetExhausted),
            );
        }
        frame.next_tick();
        assert!(frame.with(|cx| cx.emit_server_ordered(request())).is_ok());
    }

    #[test]
    fn ordered_admission_refuses_stale_held_cancelled_and_spent_native_frames() {
        let mut frame = Frame::new();
        frame.with(|cx| {
            cx.action_id += 1;
            assert_eq!(cx.emit_server_ordered(request()), Err(ActionError::Stale));
        });
        frame.with(|cx| {
            cx.eligible = false;
            assert_eq!(cx.emit_server_ordered(request()), Err(ActionError::Held));
        });
        assert!(frame
            .with(|cx| cx.emit(crate::shim::InteractReq::CloseModal))
            .is_ok());
        assert_eq!(
            frame.with(|cx| cx.emit_server_ordered(request())),
            Err(ActionError::BudgetExhausted),
        );
        frame.owner.revoke();
        assert_eq!(
            frame.with(|cx| cx.emit_server_ordered(request())),
            Err(ActionError::Cancelled),
        );
        assert!(frame
            .ledger
            .as_ref()
            .unwrap()
            .server_ordered_request
            .is_none());
    }

    #[test]
    fn ordered_pending_request_keeps_one_slot_across_ticks_until_terminal() {
        let mut frame = Frame::new();
        let id = frame.with(|cx| cx.emit_server_ordered(request())).unwrap();
        frame.next_tick();
        let next = frame.ledger.as_ref().unwrap().id_range(1).unwrap();
        assert_eq!(
            frame.with(|cx| cx.emit_server_ordered(request())),
            Err(ActionError::BudgetExhausted),
        );
        assert_eq!(frame.owner.batch_free(), 4);
        assert_eq!(frame.ledger.as_ref().unwrap().id_range(1).unwrap(), next);
        let action = frame.ledger.as_mut().unwrap().outbox.pop().unwrap();
        frame.complete(&action.authority());
        assert_eq!(frame.owner.batch_free(), 5);
        let later = frame.with(|cx| cx.emit_server_ordered(request())).unwrap();
        assert_eq!(later, id + 1);
    }

    #[test]
    fn ordered_old_run_completion_cannot_settle_a_replacement_owner() {
        let mut frame = Frame::new();
        frame.with(|cx| cx.emit_server_ordered(request())).unwrap();
        let action = frame.ledger.as_mut().unwrap().outbox.pop().unwrap();
        frame.ledger.as_mut().unwrap().revoke_foreground();
        frame.evidence.run.run += 1;
        frame.owner = Owner::new(frame.evidence.run, NonZeroU64::new(99).unwrap());
        frame.ledger.as_mut().unwrap().owner = Some(Arc::clone(&frame.owner));
        frame.next_tick();
        let replacement = frame.with(|cx| cx.emit_server_ordered(request())).unwrap();
        frame.complete(&action.authority());
        assert!(frame.ledger.as_ref().unwrap().server_ordered.is_none());
        assert_eq!(
            frame
                .ledger
                .as_ref()
                .unwrap()
                .server_ordered_request
                .unwrap()
                .get(),
            replacement,
        );
        assert_eq!(frame.owner.batch_free(), 4);
    }

    #[test]
    fn ordered_failed_admission_does_not_consume_identity_or_capacity() {
        for failure in ["no-ledger", "no-owner", "full-outbox", "full-slots"] {
            let mut frame = Frame::new();
            let next = frame.ledger.as_ref().unwrap().id_range(1).unwrap();
            match failure {
                "no-ledger" => frame.ledger = None,
                "no-owner" => frame.ledger.as_mut().unwrap().owner = None,
                "full-outbox" => {
                    let ledger = frame.ledger.as_mut().unwrap();
                    for offset in 0..ledger.outbox.capacity() {
                        ledger.outbox.push(HostAction {
                            owner: Arc::clone(&frame.owner),
                            request_id: NonZeroU64::new(100 + offset as u64).unwrap(),
                            batch: 0,
                            effect: HostEffect::Interaction(crate::shim::InteractReq::CloseModal),
                            observed_walk_outcome_seq: 0,
                            bank_deposit: None,
                        });
                    }
                }
                "full-slots" => {
                    assert!(frame.owner.acquire_batch(NonZeroU64::new(100).unwrap(), 5))
                }
                _ => unreachable!(),
            }
            let free_before = frame.owner.batch_free();
            let expected = if matches!(failure, "no-ledger" | "no-owner") {
                ActionError::Stale
            } else {
                ActionError::BudgetExhausted
            };
            assert_eq!(
                frame.with(|cx| cx.emit_server_ordered(request())),
                Err(expected),
                "{failure}"
            );
            assert_eq!(frame.budget.events, 0, "{failure}");
            if let Some(ledger) = frame.ledger.as_ref() {
                assert_eq!(ledger.id_range(1).unwrap(), next, "{failure}");
                assert!(ledger.server_ordered_request.is_none(), "{failure}");
            }
            assert_eq!(frame.owner.batch_free(), free_before, "{failure}");
        }
    }

    #[test]
    fn ordered_completion_memory_is_bounded_and_default_native_has_no_packet_buffer() {
        assert_eq!(std::mem::size_of::<Owner>(), 112);
        assert_eq!(std::mem::size_of::<Option<NonZeroU64>>(), 8);
        assert_eq!(
            std::mem::size_of::<Option<Box<ServerOrderedCompletion>>>(),
            8
        );
        // One boxed 72 B terminal envelope per ledger, allocated only on first admission.
        assert_eq!(std::mem::size_of::<ServerOrderedCompletion>(), 72);
        // The 32 B dispatch receipt stays Copy on the stack: prefix results without heap growth.
        assert_eq!(std::mem::size_of::<ServerOrderedDispatch>(), 32);
        // The 912 B native ledger stays fixed-size with a bounded outbox: no per-bot growth.
        assert_eq!(std::mem::size_of::<Ledger>(), 912);
        let mut frame = Frame::new();
        assert!(frame.ledger.as_ref().unwrap().server_ordered.is_none());
        frame.with(|cx| cx.emit_server_ordered(request())).unwrap();
        let action = frame.ledger.as_mut().unwrap().outbox.pop().unwrap();
        let authority = action.authority();
        let completed = allocation_counter::measure(|| frame.complete(&authority));
        let duplicate = allocation_counter::measure(|| frame.complete(&authority));
        assert_eq!(completed.count_current, 1);
        assert_eq!(
            completed.bytes_current as usize,
            std::mem::size_of::<ServerOrderedCompletion>()
        );
        assert_eq!(duplicate.count_total, 0);
        assert_eq!(duplicate.bytes_current, 0);
        eprintln!(
            "S3 native memory: owner={} ledger={} completion={} receipt={}; default advanced heap=0; terminal retained heap={}",
            std::mem::size_of::<Owner>(), std::mem::size_of::<Ledger>(),
            std::mem::size_of::<ServerOrderedCompletion>(),
            std::mem::size_of::<ServerOrderedDispatch>(), completed.bytes_current,
        );
    }
}
