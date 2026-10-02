use super::owner::Owner;
use super::{ActionError, InteractionReceipt, WalkReceipt, WalkRequest};
use crate::native_bank::{BankPickReceipt, BankPickRequest};
use crate::shim::InteractReq;
use api::selected::RunKey;
use std::num::NonZeroU64;
use std::sync::Arc;
use std::time::{Duration, Instant};

#[derive(Default)]
pub(crate) struct Runtime {
    pub ledger: Option<Box<Ledger>>,
    pub budget: TickBudget,
    pub clock: ActiveClock,
    pub observed_walk_outcome_seq: u64,
}

impl Runtime {
    pub fn revoke(&mut self) {
        if let Some(ledger) = &mut self.ledger {
            ledger.revoke();
        }
    }
}
/// Typed host work retains an owner, never a frame borrow. The host must check
/// `live` again immediately before applying the work or advancing its follower.
pub struct HostAction {
    pub(crate) owner: Arc<Owner>,
    pub request_id: NonZeroU64,
    pub effect: HostEffect,
    pub observed_walk_outcome_seq: u64,
}

pub enum HostEffect {
    Interaction(InteractReq),
    Walk(WalkRequest),
    BankPick(BankPickRequest),
}

/// A host continuation retains this fence after consuming the request payload.
/// It grants no authority after handle drop, cancellation or session rollover.
#[derive(Clone)]
pub struct HostAuthority {
    owner: Arc<Owner>,
    request: NonZeroU64,
    walk: bool,
}

impl HostAuthority {
    pub fn live(&self) -> bool {
        if self.walk {
            self.owner.walk_live(self.request)
        } else {
            self.owner.interaction_live(self.request)
        }
    }

    pub fn run(&self) -> RunKey {
        self.owner.run
    }
    pub fn action_id(&self) -> NonZeroU64 {
        self.owner.id
    }
    pub fn request_id(&self) -> NonZeroU64 {
        self.request
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QuietReadOwner {
    pub run: RunKey,
    pub action_id: NonZeroU64,
    pub request_id: NonZeroU64,
}

impl HostAction {
    pub fn live(&self) -> bool {
        match &self.effect {
            HostEffect::Interaction(_) | HostEffect::BankPick(_) => {
                self.owner.interaction_live(self.request_id)
            }
            HostEffect::Walk(_) => self.owner.walk_live(self.request_id),
        }
    }
    pub fn run(&self) -> RunKey {
        self.owner.run
    }
    pub fn action_id(&self) -> NonZeroU64 {
        self.owner.id
    }
    pub fn authority(&self) -> HostAuthority {
        HostAuthority {
            owner: Arc::clone(&self.owner),
            request: self.request_id,
            walk: matches!(self.effect, HostEffect::Walk(_)),
        }
    }
}

/// Allocated only on first native admission. There is one foreground owner,
/// one outstanding walk and a bounded outbox, not a second navigation runtime.
pub(crate) struct Ledger {
    pub owner: Option<Arc<Owner>>,
    next_id: u64,
    pub outbox: Vec<HostAction>,
    pub walk: Option<WalkReceipt>,
    pub interaction: Option<InteractionReceipt>,
    pub interaction_request: Option<NonZeroU64>,
    pub bank_pick: Option<BankPickReceipt>,
    pub bank_pick_request: Option<NonZeroU64>,
    pub disposal_receipts: [Option<InteractionReceipt>; 5],
    next_disposal_receipt: usize,
    pub quiet_since: Option<(NonZeroU64, NonZeroU64, Instant)>,
}

impl Default for Ledger {
    fn default() -> Self {
        Self {
            owner: None,
            next_id: 1,
            outbox: Vec::new(),
            walk: None,
            interaction: None,
            interaction_request: None,
            bank_pick: None,
            bank_pick_request: None,
            disposal_receipts: std::array::from_fn(|_| None),
            next_disposal_receipt: 0,
            quiet_since: None,
        }
    }
}

impl Ledger {
    pub fn next_id(&mut self) -> Result<NonZeroU64, ActionError> {
        let id = NonZeroU64::new(self.next_id).ok_or(ActionError::Cancelled)?;
        self.next_id = self.next_id.checked_add(1).unwrap_or(0);
        Ok(id)
    }

    pub fn quiet_read(&mut self, now: Instant) -> Option<QuietReadOwner> {
        let (request, lease, since) = self.quiet_since?;
        let owner = self.owner.as_ref()?;
        // Independent of the active action clock: a paused or stranded
        // reader must not keep hiding a modal indefinitely.
        if now.saturating_duration_since(since) >= Duration::from_secs(10) {
            owner.release_quiet(lease);
        }
        if !owner.quiet(lease) {
            self.quiet_since = None;
            return None;
        }
        Some(QuietReadOwner {
            run: owner.run,
            action_id: owner.id,
            request_id: request,
        })
    }

    pub fn revoke(&mut self) {
        if let Some(owner) = self.owner.take() {
            owner.revoke();
        }
        self.outbox.clear();
        self.walk = None;
        self.interaction = None;
        self.interaction_request = None;
        self.bank_pick = None;
        self.bank_pick_request = None;
        self.disposal_receipts.fill(None);
        self.next_disposal_receipt = 0;
        self.quiet_since = None;
    }

    pub fn complete_interaction(&mut self, authority: &HostAuthority, receipt: InteractionReceipt) {
        if !authority.live()
            || authority.request_id().get() != receipt.request_id
            || authority.run() != receipt.evidence.run
            || !self.owner.as_ref().is_some_and(|owner| {
                owner.run == authority.run() && owner.id == authority.action_id() && owner.live()
            })
        {
            return;
        }
        let owner = self.owner.as_ref().expect("owner checked");
        if owner.disposal_live(authority.request_id()) {
            self.disposal_receipts[self.next_disposal_receipt] = Some(receipt);
            self.next_disposal_receipt =
                (self.next_disposal_receipt + 1) % self.disposal_receipts.len();
            owner.cancel_interaction(authority.request_id());
        } else if self.interaction_request == Some(authority.request_id())
            && self.interaction.is_none()
        {
            self.interaction = Some(receipt);
        }
    }

    pub fn complete_bank_pick(&mut self, authority: &HostAuthority, receipt: BankPickReceipt) {
        let request = authority.request_id();
        if !authority.live()
            || request.get() != receipt.request_id
            || authority.run() != receipt.evidence.run
            || self.bank_pick_request != Some(request)
            || self.bank_pick.is_some()
            || !self.owner.as_ref().is_some_and(|owner| {
                owner.run == authority.run() && owner.id == authority.action_id() && owner.live()
            })
        {
            return;
        }
        self.bank_pick = Some(receipt);
    }
}

impl Drop for Ledger {
    fn drop(&mut self) {
        self.revoke();
    }
}

/// The observed tick, not the caller's polling rate, replenishes the allowance.
/// This value is slot-owned even when there is no native action ledger.
#[derive(Default)]
pub(crate) struct TickBudget {
    tick: Option<u64>,
    transitions: u8,
    events: u8,
}

impl TickBudget {
    pub fn observe(&mut self, tick: u64) {
        if self.tick != Some(tick) {
            self.tick = Some(tick);
            self.transitions = 0;
            self.events = 0;
        }
    }

    pub fn transition(&mut self) -> bool {
        if self.transitions == 32 {
            return false;
        }
        self.transitions += 1;
        true
    }

    pub fn event(&mut self, disposal: bool) -> bool {
        if self.events >= if disposal { 5 } else { 1 } {
            return false;
        }
        self.events += 1;
        true
    }
}

#[derive(Default)]
pub(crate) struct ActiveClock {
    elapsed: Duration,
    eligible_since: Option<Instant>,
}

impl ActiveClock {
    pub fn observe(&mut self, now: Instant, eligible: bool) {
        match (self.eligible_since, eligible) {
            (Some(since), false) => {
                self.elapsed += now.saturating_duration_since(since);
                self.eligible_since = None;
            }
            (None, true) => self.eligible_since = Some(now),
            _ => {}
        }
    }

    pub fn now(&self, now: Instant) -> Duration {
        self.elapsed
            + self
                .eligible_since
                .map_or(Duration::ZERO, |since| now.saturating_duration_since(since))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native::QuietReadLease;

    #[test]
    fn interaction_receipts_reject_old_requests_sessions_and_revoked_owners() {
        let run = RunKey {
            slot: 1,
            run: 2,
            session: 3,
        };
        let mut ledger = Ledger::default();
        let owner = Owner::new(run, ledger.next_id().unwrap());
        ledger.owner = Some(Arc::clone(&owner));
        let old = HostAuthority {
            owner: Arc::clone(&owner),
            request: ledger.next_id().unwrap(),
            walk: false,
        };
        let current = HostAuthority {
            owner: Arc::clone(&owner),
            request: ledger.next_id().unwrap(),
            walk: false,
        };
        ledger.interaction_request = Some(current.request);
        owner.set_interaction(current.request);
        let receipt = InteractionReceipt {
            request_id: current.request.get(),
            evidence: api::quest_progress::EvidenceStamp {
                run,
                tick: 4,
                sequence: 4,
            },
            accepted: true,
            chat_since: 0,
        };
        ledger.complete_interaction(
            &old,
            InteractionReceipt {
                request_id: old.request.get(),
                ..receipt
            },
        );
        assert!(ledger.interaction.is_none());
        ledger.complete_interaction(
            &current,
            InteractionReceipt {
                evidence: api::quest_progress::EvidenceStamp {
                    run: run.next_session().unwrap(),
                    ..receipt.evidence
                },
                ..receipt
            },
        );
        assert!(ledger.interaction.is_none());
        ledger.complete_interaction(&current, receipt);
        assert_eq!(ledger.interaction, Some(receipt));
        ledger.complete_interaction(
            &current,
            InteractionReceipt {
                accepted: false,
                ..receipt
            },
        );
        assert_eq!(
            ledger.interaction,
            Some(receipt),
            "a duplicate cannot replace settlement"
        );
        ledger.revoke();
        ledger.complete_interaction(&current, receipt);
        assert!(
            ledger.interaction.is_none(),
            "late receipt revived cancelled work"
        );
    }

    #[test]
    fn bank_pick_receipts_require_the_live_matching_owner_run_and_request() {
        let run = RunKey {
            slot: 1,
            run: 2,
            session: 3,
        };
        let mut ledger = Ledger::default();
        let owner = Owner::new(run, ledger.next_id().unwrap());
        ledger.owner = Some(Arc::clone(&owner));
        let old = HostAuthority {
            owner: Arc::clone(&owner),
            request: ledger.next_id().unwrap(),
            walk: false,
        };
        let current = HostAuthority {
            owner: Arc::clone(&owner),
            request: ledger.next_id().unwrap(),
            walk: false,
        };
        ledger.bank_pick_request = Some(current.request);
        owner.set_interaction(current.request);
        let receipt = BankPickReceipt {
            request_id: current.request.get(),
            evidence: api::quest_progress::EvidenceStamp {
                run,
                tick: 4,
                sequence: 4,
            },
            selected: crate::native_bank::SelectedBank {
                bank_index: 7,
                access_tile: api::snapshot::WorldTile {
                    x: 100,
                    z: 200,
                    level: 0,
                },
                kind: crate::native_bank::PickKind::Reachable,
                access: None,
            },
        };
        ledger.complete_bank_pick(
            &old,
            BankPickReceipt {
                request_id: old.request.get(),
                ..receipt.clone()
            },
        );
        assert!(ledger.bank_pick.is_none());
        ledger.complete_bank_pick(
            &current,
            BankPickReceipt {
                evidence: api::quest_progress::EvidenceStamp {
                    run: run.next_session().unwrap(),
                    ..receipt.evidence
                },
                ..receipt.clone()
            },
        );
        assert!(ledger.bank_pick.is_none());
        ledger.complete_bank_pick(&current, receipt.clone());
        assert_eq!(ledger.bank_pick, Some(receipt.clone()));
        ledger.complete_bank_pick(
            &current,
            BankPickReceipt {
                selected: crate::native_bank::SelectedBank {
                    bank_index: 8,
                    ..receipt.selected.clone()
                },
                ..receipt.clone()
            },
        );
        assert_eq!(
            ledger.bank_pick,
            Some(receipt.clone()),
            "a duplicate cannot replace the selected stand"
        );
        ledger.revoke();
        ledger.complete_bank_pick(&current, receipt);
        assert!(
            ledger.bank_pick.is_none(),
            "a late pick revived cancelled work"
        );
    }

    #[test]
    fn expired_guard_cannot_clear_a_replacement_for_the_same_request() {
        let run = RunKey {
            slot: 1,
            run: 2,
            session: 3,
        };
        let mut ledger = Ledger::default();
        let owner = Owner::new(run, ledger.next_id().unwrap());
        ledger.owner = Some(Arc::clone(&owner));
        let request = owner.id;
        let first = ledger.next_id().unwrap();
        let now = Instant::now();
        assert!(owner.acquire_quiet(first));
        ledger.quiet_since = Some((request, first, now));
        let old_guard = QuietReadLease {
            owner: Arc::clone(&owner),
            lease_id: first,
        };
        assert_eq!(ledger.quiet_read(now).unwrap().request_id, request);
        // No active-clock advance or machine poll is required for safety expiry.
        assert!(ledger.quiet_read(now + Duration::from_secs(10)).is_none());
        let replacement = ledger.next_id().unwrap();
        assert!(owner.acquire_quiet(replacement));
        ledger.quiet_since = Some((request, replacement, now + Duration::from_secs(10)));
        drop(old_guard);
        assert_eq!(
            ledger
                .quiet_read(now + Duration::from_secs(11))
                .unwrap()
                .request_id,
            request
        );
        ledger.revoke();
        assert!(ledger.quiet_read(now + Duration::from_secs(11)).is_none());
    }

    #[test]
    fn same_tick_repoll_does_not_replenish_shared_budget() {
        let mut budget = TickBudget::default();
        budget.observe(7);
        assert!(budget.event(false));
        for _ in 0..32 {
            budget.observe(7);
            assert!(!budget.event(false));
            assert!(budget.transition());
        }
        assert!(!budget.transition());
        for _ in 0..4 {
            assert!(budget.event(true));
        }
        assert!(!budget.event(true));
        budget.observe(8);
        assert!(budget.event(false));
        assert!(budget.transition());
    }

    #[test]
    fn frozen_intervals_do_not_spend_active_deadlines() {
        let start = Instant::now();
        let mut clock = ActiveClock::default();
        clock.observe(start, true);
        clock.observe(start + Duration::from_secs(2), false);
        assert_eq!(
            clock.now(start + Duration::from_secs(60)),
            Duration::from_secs(2)
        );
        clock.observe(start + Duration::from_secs(60), false);
        clock.observe(start + Duration::from_secs(120), true);
        assert_eq!(
            clock.now(start + Duration::from_secs(123)),
            Duration::from_secs(5)
        );
        clock.observe(start + Duration::from_secs(123), false);
        assert_eq!(
            clock.now(start + Duration::from_secs(180)),
            Duration::from_secs(5)
        );
    }
}
