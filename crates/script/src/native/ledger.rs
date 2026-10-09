use super::owner::Owner;
use super::{ActionError, AssessReceipt, InteractionReceipt, WalkEvent, WalkReceipt, WalkRequest};
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
/// Immutable evidence carried with a native deposit click until host dispatch.
#[derive(Debug, Clone)]
pub struct BankDepositTrace {
    pub generation: u64,
    pub transfer: u8,
    pub click_tick: u64,
    pub sequence: u64,
    pub id: i32,
    pub slot: i32,
    pub component: i32,
    pub option: Arc<str>,
    pub operation: i32,
    pub main_before: i32,
    pub side_before: Option<i32>,
    pub bank_before: Option<i32>,
    pub main_revision: Option<u64>,
    pub side_revision: Option<u64>,
}

pub struct HostAction {
    pub(crate) owner: Arc<Owner>,
    pub request_id: NonZeroU64,
    pub batch: u64,
    pub effect: HostEffect,
    pub observed_walk_outcome_seq: u64,
    pub bank_deposit: Option<Arc<BankDepositTrace>>,
}

pub enum HostEffect {
    Interaction(InteractReq),
    Walk(WalkRequest),
    BankPick(BankPickRequest),
    AssessWalk(WalkRequest),
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

    /// Observe the action owner's lifetime independently of this request.
    /// This is not dispatch authority: request continuations must use `live`.
    pub fn owner_live(&self) -> bool {
        self.owner.live()
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
            HostEffect::Interaction(_) | HostEffect::BankPick(_) | HostEffect::AssessWalk(_) => {
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
    pub walk_events: Vec<WalkEvent>,
    pub interaction: Option<InteractionReceipt>,
    pub interaction_request: Option<NonZeroU64>,
    pub bank_pick: Option<BankPickReceipt>,
    pub bank_pick_request: Option<NonZeroU64>,
    pub assess_owner: Option<Arc<Owner>>,
    pub assess_receipt: Option<AssessReceipt>,
    pub assess_request: Option<NonZeroU64>,
    pub batch_receipts: [Option<InteractionReceipt>; 5],
    next_batch_receipt: usize,
    pub quiet_since: Option<(NonZeroU64, NonZeroU64, Instant)>,
    /// The open bank session (login run, snapshot session generation) known
    /// to be in Item withdraw mode: a native machine opened it (the server
    /// resets `%bankcert` on open) or pressed Item in it. It outlives one
    /// machine so a later verb in the same session skips the press.
    pub bank_item_session: Option<(RunKey, u64)>,
}

impl Default for Ledger {
    fn default() -> Self {
        Self {
            owner: None,
            next_id: 1,
            outbox: Vec::with_capacity(6),
            walk: None,
            walk_events: Vec::new(),
            interaction: None,
            interaction_request: None,
            bank_pick: None,
            bank_pick_request: None,
            assess_owner: None,
            assess_receipt: None,
            assess_request: None,
            batch_receipts: std::array::from_fn(|_| None),
            next_batch_receipt: 0,
            quiet_since: None,
            bank_item_session: None,
        }
    }
}

impl Ledger {
    pub fn next_id(&mut self) -> Result<NonZeroU64, ActionError> {
        let id = NonZeroU64::new(self.next_id).ok_or(ActionError::Cancelled)?;
        self.next_id = self.next_id.checked_add(1).unwrap_or(0);
        Ok(id)
    }
    /// Check that `count` consecutive nonzero ids can be consumed without
    /// changing the cursor. Batches use this before reserving owner slots.
    pub fn id_range(&self, count: usize) -> Result<NonZeroU64, ActionError> {
        let first = NonZeroU64::new(self.next_id).ok_or(ActionError::Cancelled)?;
        let last_offset = u64::try_from(count.checked_sub(1).ok_or(ActionError::Cancelled)?)
            .map_err(|_| ActionError::Cancelled)?;
        first
            .get()
            .checked_add(last_offset)
            .ok_or(ActionError::Cancelled)?;
        Ok(first)
    }

    /// Commit a previously checked consecutive id range.
    pub fn commit_id_range(&mut self, first: NonZeroU64, count: usize) {
        debug_assert_eq!(self.next_id, first.get());
        self.next_id = first.get().checked_add(count as u64).unwrap_or(0);
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

    pub fn revoke_foreground(&mut self) {
        if let Some(owner) = self.owner.take() {
            owner.revoke();
        }
        self.outbox
            .retain(|action| matches!(action.effect, HostEffect::AssessWalk(_)));
        self.walk = None;
        self.walk_events.clear();
        self.interaction = None;
        self.interaction_request = None;
        self.bank_pick = None;
        self.bank_pick_request = None;
        self.batch_receipts.fill(None);
        self.next_batch_receipt = 0;
        self.quiet_since = None;
    }

    pub fn revoke(&mut self) {
        self.revoke_foreground();
        if let Some(owner) = self.assess_owner.take() {
            owner.revoke();
        }
        self.outbox.clear();
        self.assess_receipt = None;
        self.assess_request = None;
        self.bank_item_session = None;
    }

    pub fn complete_assess_walk(&mut self, authority: &HostAuthority, receipt: AssessReceipt) {
        let request = authority.request_id();
        if !authority.live()
            || request.get() != receipt.request_id
            || authority.run() != receipt.evidence.run
            || self.assess_request != Some(request)
            || self.assess_receipt.is_some()
            || !self.assess_owner.as_ref().is_some_and(|owner| {
                owner.run == authority.run() && owner.id == authority.action_id() && owner.live()
            })
        {
            return;
        }
        self.assess_receipt = Some(receipt);
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
        if owner.batch_live(authority.request_id()) {
            owner.record_batch_receipt(authority.request_id(), receipt.accepted);
            self.batch_receipts[self.next_batch_receipt] = Some(receipt);
            self.next_batch_receipt = (self.next_batch_receipt + 1) % self.batch_receipts.len();
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
    pub(super) events: u8,
    scene: SceneFence,
}

const SCENE_KEY_BITS: u32 = 25;
const SCENE_KEY_MASK: u32 = (1 << SCENE_KEY_BITS) - 1;
const SCENE_SEEN_SHIFT: u32 = SCENE_KEY_BITS;
const SCENE_SEEN_MASK: u32 = 0b11 << SCENE_SEEN_SHIFT;
const SCENE_ENTERED_BIT: u32 = 1 << 27;

/// What the last observed frame showed of the player's scene.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
#[repr(u8)]
enum Seen {
    /// The slot has observed no frame: a run that starts in a scene has not
    /// just entered it.
    #[default]
    Nothing = 0,
    /// Out of game, or the scene was not built.
    Outside = 1,
    /// In a particular observed scene key.
    Scene = 2,
}

/// The engine tracks the zones around the player per level inside the build
/// area (`BuildArea.ts:31-55`): a level change or a rebuilt build area (a
/// new origin) makes every zone around the player newly tracked, and
/// `NetworkPlayer.ts:294-315` then resets each one and resends its loc
/// changes. The scene key uses 25 bits: a known-origin bit, origin zone x and
/// z (11 bits each), and the level (2 bits). Its spare high bits hold the
/// shared scene-entry latch.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct SceneKey(u32);

impl SceneKey {
    fn observe(snapshot: Option<&api::snapshot::GameSnapshot>) -> Option<Self> {
        let snapshot = snapshot?;
        if !snapshot.ingame() || snapshot.scene_state() != 2 {
            return None;
        }
        Some(Self::from_parts(
            snapshot.local_player()?.player.network.level,
            snapshot.base(),
        ))
    }

    pub(crate) fn from_parts(level: i32, origin: Option<(i32, i32)>) -> Self {
        let level = (level as u32) & 0b11;
        let origin = origin.map_or(0, |(x, z)| {
            (1 << 22) | ((((x as u32) >> 3) & 0x7ff) << 11) | (((z as u32) >> 3) & 0x7ff)
        });
        Self((origin << 2) | level)
    }

    fn seen(self) -> Seen {
        match (self.0 & SCENE_SEEN_MASK) >> SCENE_SEEN_SHIFT {
            0 => Seen::Nothing,
            1 => Seen::Outside,
            2 => Seen::Scene,
            _ => unreachable!("SceneFence only stores the three Seen states"),
        }
    }

    fn scene(self) -> Self {
        Self(self.0 & SCENE_KEY_MASK)
    }
}

/// Shared native/compat latch packed into SceneKey's spare high bits, so
/// evidence wakes cannot clear scene entry without growing TickBudget.
#[derive(Default)]
pub(crate) struct SceneFence(SceneKey);

impl SceneFence {
    pub(crate) fn next_tick(&mut self) {
        self.0 .0 &= !SCENE_ENTERED_BIT;
    }

    pub(crate) fn observe(&mut self, scene: Option<SceneKey>) {
        let (seen, scene) = match scene {
            Some(scene) => (Seen::Scene, scene),
            None => (Seen::Outside, SceneKey::default()),
        };
        let previous_seen = self.0.seen();
        let previous_scene = self.0.scene();
        let mut entered = self.0 .0 & SCENE_ENTERED_BIT;
        if previous_seen != Seen::Nothing && (previous_seen, previous_scene) != (seen, scene) {
            entered = SCENE_ENTERED_BIT;
        }
        self.0 =
            SceneKey((scene.0 & SCENE_KEY_MASK) | ((seen as u32) << SCENE_SEEN_SHIFT) | entered);
    }

    pub(crate) fn entered_scene(&self) -> bool {
        self.0 .0 & SCENE_ENTERED_BIT != 0
    }
}

impl TickBudget {
    pub fn observe(&mut self, tick: u64) {
        if self.tick != Some(tick) {
            self.tick = Some(tick);
            self.transitions = 0;
            self.events = 0;
            self.scene.next_tick();
        }
    }

    /// One host frame: replenish on a new tick, and mark the tick that first
    /// shows the player on a new level or in a rebuilt build area.
    pub fn observe_frame(&mut self, tick: u64, snapshot: Option<&api::snapshot::GameSnapshot>) {
        self.observe(tick);
        self.scene.observe(SceneKey::observe(snapshot));
    }

    /// This observed tick is the first to show the player on a new level or
    /// in a rebuilt build area. The engine writes the PLAYER_INFO that moves
    /// the player before the resets and loc changes of the zones it now
    /// tracks (`World.ts:1108-1114`, `Zone.ts:157-189`), and the client reads
    /// at most five packets a frame (`client.rs:12497-12501`), so this tick's
    /// locs can still be the old ones. The next observed tick's PLAYER_INFO
    /// arrives after all of them.
    pub fn entered_scene(&self) -> bool {
        self.scene.entered_scene()
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
    pub fn batch(&mut self) -> bool {
        if self.events != 0 {
            return false;
        }
        self.events = 5;
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
    fn id_ranges_check_the_last_id_without_consuming_the_cursor() {
        let mut ledger = Ledger::default();
        ledger.next_id = u64::MAX - 1;
        let first = ledger.id_range(2).unwrap();
        assert_eq!(first.get(), u64::MAX - 1);
        assert_eq!(ledger.id_range(3), Err(ActionError::Cancelled));
        assert_eq!(ledger.id_range(0), Err(ActionError::Cancelled));
        assert_eq!(ledger.next_id, u64::MAX - 1);

        ledger.commit_id_range(first, 2);
        assert_eq!(ledger.next_id, 0, "the last nonzero id may be consumed");
        assert_eq!(ledger.id_range(1), Err(ActionError::Cancelled));
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
    fn batch_saturates_shared_events_until_the_next_observed_tick() {
        let mut budget = TickBudget::default();
        budget.observe(7);
        assert!(budget.batch());
        assert_eq!(budget.events, 5);
        assert!(!budget.batch());
        assert!(!budget.event(false));
        assert!(!budget.event(true));

        budget.observe(7);
        assert_eq!(
            budget.events, 5,
            "same-tick re-observation cannot refill it"
        );
        assert!(!budget.batch());
        budget.observe(8);
        assert!(budget.batch(), "the next observed tick starts uncharged");
        assert_eq!(budget.events, 5);
    }
    #[test]
    fn scene_fence_uses_four_bytes_and_tick_budget_stays_compact() {
        assert_eq!(std::mem::size_of::<SceneFence>(), 4);
        assert_eq!(std::mem::size_of::<TickBudget>(), 24);
    }

    fn scene_frame(level: i32, x: i32, z: i32, origin: (i32, i32)) -> api::snapshot::GameSnapshot {
        let mut snapshot = api::snapshot::GameSnapshot::new();
        snapshot.seed_ingame(2);
        snapshot.seed_world(api::snapshot::WorldStateView {
            map_base_x: origin.0,
            map_base_z: origin.1,
            ..Default::default()
        });
        snapshot.seed_local_player(crate::quester::families::tests::local_player(
            api::WorldTile { x, z, level },
        ));
        snapshot
    }

    #[test]
    fn only_a_level_change_or_rebuilt_build_area_enters_a_scene_for_its_tick() {
        let origin = (3112, 3256);
        let mut budget = TickBudget::default();
        budget.observe_frame(1, Some(&scene_frame(1, 3165, 3307, origin)));
        assert!(
            !budget.entered_scene(),
            "a run that starts in a scene has not just entered it"
        );
        budget.observe_frame(2, Some(&scene_frame(1, 3165, 3307, origin)));
        assert!(!budget.entered_scene());
        // Walking inside the build area keeps the tracked zones.
        budget.observe_frame(3, Some(&scene_frame(1, 3175, 3311, origin)));
        assert!(!budget.entered_scene());

        // Down the ladder: the new level's loc state follows PLAYER_INFO.
        budget.observe_frame(4, Some(&scene_frame(0, 3175, 3311, origin)));
        assert!(budget.entered_scene());
        budget.observe_frame(4, Some(&scene_frame(0, 3175, 3311, origin)));
        assert!(
            budget.entered_scene(),
            "a same-tick re-observation keeps the entry"
        );
        budget.observe_frame(5, Some(&scene_frame(0, 3175, 3311, origin)));
        assert!(!budget.entered_scene(), "the next tick holds the new locs");

        // A rebuilt build area resets every tracked zone.
        budget.observe_frame(6, Some(&scene_frame(0, 3184, 3311, (3120, 3256))));
        assert!(budget.entered_scene());
        budget.observe_frame(7, None);
        assert!(budget.entered_scene(), "leaving the scene is a change");
        budget.observe_frame(8, None);
        assert!(!budget.entered_scene());
        // A relog's first in-game frame enters its scene.
        budget.observe_frame(9, Some(&scene_frame(0, 3184, 3311, (3120, 3256))));
        assert!(budget.entered_scene());
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
