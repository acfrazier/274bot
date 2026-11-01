//! Native adapter for the shared walk correlation and arrival core.
use super::walk_wait::{HostOutcome, Observation, WalkKey, WalkSlot};
use super::{
    ActionContext, ActionError, HoldResume, NativeMachine, PrereqAllow, WalkEnd, WalkReceipt,
    WalkRequest,
};
use api::selected::QuestGate;
use api::snapshot::SnapshotView;
use nav_planner::contract::ArrivalKind;
use std::num::NonZeroU64;
use std::sync::Arc;
use std::task::Poll;
use std::time::Duration;

/// Eligible time a native walk may stay unsettled without a host terminal
/// receipt before it ends `Failed` and revokes its host follow. The backstop
/// uses the active clock, so pause, hold, not-ready and Blocked intervals do
/// not spend it.
pub const WALK_DEADLINE: Duration = Duration::from_secs(10 * 60);
/// Login's `initalltabs` opens the quest tab and calls `update_questlist`.
/// Give unresolved gates one later tick for quest-list colour updates to
/// reach the snapshot before reporting missing evidence.
const QUEST_LIST_STATUS_WAIT_TICKS: u64 = 1;

pub struct Walk {
    // Retain only fields missing from the correlation core, which already
    // owns the target, radius and request ID ([`WalkSlot::request`]).
    replay: Box<WalkReplay>,
    quest_list_wait_since: Option<u64>,
    quest_list_retry_used: bool,
    deadline: Duration,
    wait: WalkSlot,
}

/// What the walk needs besides the correlation core's target, radius and
/// request ID: the loc identity, arrival mode, candidate goals and required
/// evidence it reads, and the rest of the request, to send it again if the
/// host drops it unsent in a guardian hold ([`NativeMachine::after_hold`]).
/// Boxed: one allocation per live walk, which a resend reuses.
struct WalkReplay {
    required_after: api::quest_progress::EvidenceStamp,
    loc_id: Option<i32>,
    arrival: ArrivalKind,
    candidate_goals: Option<super::WalkGoals>,
    options: super::WalkOptions,
    evidence: Option<Arc<dyn api::quest_progress::EvidenceProvider>>,
    cross: Box<[Arc<str>]>,
    avoid: Box<[Arc<str>]>,
    protect: bool,
    food_guard: bool,
    allow: super::WalkAllow,
}

impl WalkReplay {
    /// Keep everything of `request` but its target and radius.
    fn of(request: &WalkRequest) -> Box<Self> {
        Box::new(Self {
            required_after: request.required_after,
            loc_id: request.loc_id,
            arrival: request.arrival,
            candidate_goals: request.candidate_goals.clone(),
            options: request.options,
            evidence: request.evidence.clone(),
            cross: request.cross.clone(),
            avoid: request.avoid.clone(),
            protect: request.protect,
            food_guard: request.food_guard,
            allow: request.allow,
        })
    }

    /// The request again, to the correlation core's target and radius.
    fn request(
        &self,
        key: WalkKey,
        required_after: api::quest_progress::EvidenceStamp,
    ) -> WalkRequest {
        WalkRequest {
            target: key.tile,
            radius: u16::try_from(key.radius).expect("walk radius originated as u16"),
            loc_id: self.loc_id,
            arrival: self.arrival,
            required_after,
            candidate_goals: self.candidate_goals.clone(),
            options: self.options,
            evidence: self.evidence.clone(),
            cross: self.cross.clone(),
            avoid: self.avoid.clone(),
            protect: self.protect,
            food_guard: self.food_guard,
            allow: self.allow,
        }
    }
}

fn walk_key(request: &WalkRequest) -> WalkKey {
    WalkKey {
        tile: request.target,
        radius: i32::from(request.radius),
        allow_teleports: request.options.allow_teleports.explicit(),
    }
}

struct Frame<'a> {
    snapshot: SnapshotView<'a>,
    outcome: HostOutcome,
    loc_id: Option<i32>,
    arrival: ArrivalKind,
    candidate_goals: bool,
    goal: Option<api::WorldTile>,
}

fn quest_list_status_observed(
    gates: &[QuestGate],
    quests: Option<&api::quest_facts::QuestCatalog>,
    snapshot: SnapshotView<'_>,
) -> bool {
    let mut has_complete_gate = false;
    for gate in gates {
        if let QuestGate::Complete(quest) = gate {
            has_complete_gate = true;
            if !quests
                .and_then(|quests| {
                    api::quest_progress::quest_list_status(quests, &quest.0, snapshot)
                })
                .is_some_and(|status| status.value != api::snapshot::QuestListStatus::Unknown)
            {
                return false;
            }
        }
    }
    if has_complete_gate {
        return true;
    }
    snapshot.quest_statuses().is_some_and(|rows| {
        rows.value
            .iter()
            .any(|row| row.status() != api::snapshot::QuestListStatus::Unknown)
    })
}

impl Observation for Frame<'_> {
    fn outcome(&self) -> HostOutcome {
        self.outcome
    }

    fn cancelled(&self) -> bool {
        false // The action owner fence rejects revoked polls before entry.
    }

    fn arrived(&self, key: WalkKey) -> bool {
        let key = if self.candidate_goals {
            let Some(tile) = self.goal else {
                return false;
            };
            WalkKey {
                tile,
                radius: 0,
                ..key
            }
        } else {
            key
        };
        let Some(player) = self.snapshot.local_player() else {
            return false;
        };
        // Arrival is a server-coordinate fact, not render interpolation.
        let here = player.value.player.network;
        match (self.arrival, self.loc_id) {
            (ArrivalKind::Area, None) => {
                api::query::arrived_in_area(here, key.tile, key.radius, |tile| {
                    self.snapshot
                        .reach()
                        .is_some_and(|reach| reach.value.walkable(tile))
                })
            }
            (ArrivalKind::Area, Some(_)) => false,
            (ArrivalKind::Reach, Some(id)) => self
                .snapshot
                .walk_loc_arrived(here, key.tile, key.radius, id),
            (ArrivalKind::Reach, None) => self.snapshot.walk_arrived(here, key.tile, key.radius),
        }
    }
    fn moving(&self) -> Option<bool> {
        self.snapshot
            .local_player()
            .map(|player| player.value.player.actor.moving)
    }
}
fn deadline_failure_receipt(
    request_id: u64,
    evidence: api::quest_progress::EvidenceStamp,
) -> WalkReceipt {
    WalkReceipt {
        goal: None,
        request_id,
        evidence,
        end: WalkEnd::Failed,
        blocked: None,
        detail: Some(std::sync::Arc::from(
            "walk receipt deadline expired before the host reported a terminal result",
        )),
        refusal: None,
        assessment: None,
        escape: None,
        prereq_allow: None,
    }
}

impl Walk {
    /// The current host walk request. Dropping the machine does not revoke
    /// the host follow, so an owner that abandons it passes this ID to
    /// [`ActionContext::cancel_request`] before it cancels the machine.
    pub(crate) fn request_id(&self) -> u64 {
        self.wait
            .request()
            .expect("walk correlation remains active")
            .0
    }

    /// Read the effective grants from this walk's fresh terminal receipt.
    pub(crate) fn prereq_allow(&self, cx: &ActionContext<'_>) -> Option<PrereqAllow> {
        cx.walk_receipt(self.request_id())
            .filter(|receipt| receipt.evidence.meets(self.replay.required_after))
            .and_then(|receipt| receipt.prereq_allow)
    }

    fn replan_after_quest_list_wait(
        &mut self,
        cx: &mut ActionContext<'_>,
    ) -> Result<(), ActionError> {
        let required_after = cx.evidence();
        let (_, key) = self
            .wait
            .request()
            .expect("walk correlation remains active");
        let request_id = cx.walk(self.replay.request(key, required_after))?;
        self.replay.required_after = required_after;
        self.quest_list_wait_since = None;
        self.quest_list_retry_used = true;
        self.wait.begin(
            NonZeroU64::new(request_id).expect("host request is nonzero"),
            key,
            HostOutcome::empty(),
        );
        Ok(())
    }
}

impl NativeMachine for Walk {
    type Args = WalkRequest;
    type Output = WalkReceipt;

    fn begin(request: Self::Args, cx: &mut ActionContext<'_>) -> Result<Self, ActionError> {
        let key = walk_key(&request);
        let replay = WalkReplay::of(&request);
        let request_id = cx.walk(request)?;
        let mut wait = WalkSlot::new();
        wait.begin(
            NonZeroU64::new(request_id).expect("host request is nonzero"),
            key,
            HostOutcome::empty(),
        );
        Ok(Self {
            replay,
            quest_list_wait_since: None,
            quest_list_retry_used: false,
            deadline: cx.active_now().saturating_add(WALK_DEADLINE),
            wait,
        })
    }

    const RESUMES_AFTER_HOLD: bool = true;

    /// The host follows an armed walk through a guardian hold: it freezes the
    /// route and carries on from where the player stands afterwards. Only a
    /// walk the host dropped unsent in the hold has no follow and no receipt
    /// to come, so this sends that one again, once, unless the player already
    /// stands at its goal.
    fn after_hold(&mut self, cx: &mut ActionContext<'_>) -> Result<HoldResume, ActionError> {
        let (request_id, key) = self
            .wait
            .request()
            .expect("walk correlation remains active");
        if cx.walk_receipt(request_id).is_some() {
            return Ok(HoldResume::Kept("the walk already has its receipt"));
        }
        if !cx.walk_unsent(request_id) {
            return Ok(HoldResume::Kept(
                "the host armed the walk and still follows it",
            ));
        }
        let frame = Frame {
            snapshot: cx.snapshot(),
            outcome: HostOutcome::empty(),
            loc_id: self.replay.loc_id,
            arrival: self.replay.arrival,
            candidate_goals: self.replay.candidate_goals.is_some(),
            goal: None,
        };
        if !frame.candidate_goals && frame.arrived(key) {
            return Ok(HoldResume::Kept("the player already stands at the goal"));
        }
        let request_id = cx.walk(self.replay.request(key, self.replay.required_after))?;
        self.wait.begin(
            NonZeroU64::new(request_id).expect("host request is nonzero"),
            key,
            HostOutcome::empty(),
        );
        Ok(HoldResume::Reissued("walk request"))
    }

    fn poll(&mut self, cx: &mut ActionContext<'_>) -> Poll<Result<Self::Output, ActionError>> {
        let (request_id, key) = self
            .wait
            .request()
            .expect("walk correlation remains active");
        if let Some(receipt) = cx
            .walk_receipt(request_id)
            .filter(|receipt| receipt.end == WalkEnd::UserInput)
        {
            return Poll::Ready(Ok(receipt.clone()));
        }
        if !cx.evidence().meets(self.replay.required_after) {
            if cx.active_now() >= self.deadline {
                cx.cancel_request(request_id);
                return Poll::Ready(Ok(deadline_failure_receipt(request_id, cx.evidence())));
            }
            return Poll::Pending;
        }

        let needs_quest_evidence = cx.walk_receipt(request_id).is_some_and(|receipt| {
            receipt.evidence.meets(self.replay.required_after)
                && matches!(&receipt.end, WalkEnd::NeedsEvidence(_))
        });
        if needs_quest_evidence {
            if cx.active_now() >= self.deadline {
                cx.cancel_request(request_id);
                return Poll::Ready(Ok(deadline_failure_receipt(request_id, cx.evidence())));
            }
            let receipt = cx
                .walk_receipt(request_id)
                .expect("quest-evidence receipt remains correlated")
                .clone();
            if self.quest_list_retry_used {
                let mut refused = receipt;
                refused.detail = Some(Arc::from(
                    "fresh route still needs quest evidence after one quest-list-triggered retry",
                ));
                return Poll::Ready(Ok(refused));
            }
            let WalkEnd::NeedsEvidence(gates) = &receipt.end else {
                unreachable!("quest-evidence receipt was checked above");
            };
            let first_observation = self.quest_list_wait_since.is_none();
            let tick = cx.evidence().tick;
            let started = self.quest_list_wait_since.get_or_insert(tick);
            let snapshot = cx.snapshot();
            let quest_rows = snapshot.quest_statuses().map(|statuses| statuses.value);
            // The catalog is prepared off-pump. This access only borrows its
            // cached rows and uses the provider's canonical ID-to-display match.
            let quests = api::game_data::for_revision(cx.pin().revision)
                .ok()
                .filter(|data| {
                    data.selected_pin()
                        .is_ok_and(|pin| pin.as_ref() == cx.pin())
                })
                .and_then(|data| data.try_quests());
            let gate_status_observed =
                quest_list_status_observed(gates, quests.as_deref(), snapshot);
            if first_observation || tick != *started {
                api::host_log!(
                    api::hostlog::Category::NavTrace,
                    api::hostlog::Level::Debug,
                    "walk-quest-list-observation request={} tick={} gates={gates:?} gate_status_observed={} rows={quest_rows:?}",
                    request_id,
                    tick,
                    gate_status_observed,
                );
            }
            if gate_status_observed {
                if let Err(error) = self.replan_after_quest_list_wait(cx) {
                    return Poll::Ready(Err(error));
                }
                return Poll::Pending;
            }
            if tick.saturating_sub(*started) < QUEST_LIST_STATUS_WAIT_TICKS {
                return Poll::Pending;
            }
            let mut refused = receipt;
            refused.detail = Some(Arc::from(
                "no usable quest-list status was observed after one game tick",
            ));
            return Poll::Ready(Ok(refused));
        }
        let receipt = cx
            .walk_receipt(request_id)
            .filter(|receipt| receipt.evidence.meets(self.replay.required_after));
        let outcome = receipt.map_or_else(HostOutcome::empty, |receipt| HostOutcome {
            seq: receipt.evidence.sequence,
            generation: receipt.evidence.run.run,
            request_id: receipt.request_id,
            failed: !matches!(
                receipt.end,
                WalkEnd::Arrived | WalkEnd::RouteEnded | WalkEnd::Blocked
            ),
            blocked: receipt.end == WalkEnd::Blocked,
            key,
        });
        let frame = Frame {
            snapshot: cx.snapshot(),
            outcome,
            loc_id: self.replay.loc_id,
            arrival: self.replay.arrival,
            candidate_goals: self.replay.candidate_goals.is_some(),
            goal: receipt.and_then(|receipt| receipt.goal),
        };
        if !self.wait.poll(request_id, &frame) {
            if cx.active_now() >= self.deadline {
                cx.cancel_request(request_id);
                return Poll::Ready(Ok(deadline_failure_receipt(request_id, cx.evidence())));
            }
            return Poll::Pending;
        }
        if let Some(receipt) = receipt {
            if !matches!(receipt.end, WalkEnd::Arrived | WalkEnd::RouteEnded) {
                return Poll::Ready(Ok(receipt.clone()));
            }
        }
        // An arrival on a new level or in a rebuilt build area is reported on
        // the next tick: this tick's locs can still be the old ones, and the
        // owner's next click would choose from them.
        if cx.entered_scene() {
            if cx.active_now() >= self.deadline {
                cx.cancel_request(request_id);
                return Poll::Ready(Ok(deadline_failure_receipt(request_id, cx.evidence())));
            }
            return Poll::Pending;
        }

        if frame.arrived(key) {
            let detail = receipt.and_then(|receipt| receipt.detail.clone());
            return Poll::Ready(Ok(WalkReceipt {
                goal: frame.goal,
                request_id,
                evidence: cx.evidence(),
                end: WalkEnd::Arrived,
                blocked: None,
                detail,
                refusal: None,
                assessment: None,
                escape: None,
                prereq_allow: receipt.and_then(|receipt| receipt.prereq_allow),
            }));
        }
        match receipt {
            Some(receipt) => Poll::Ready(Ok(receipt.clone())),
            None if cx.active_now() >= self.deadline => {
                cx.cancel_request(request_id);
                Poll::Ready(Ok(deadline_failure_receipt(request_id, cx.evidence())))
            }
            None => Poll::Pending,
        }
    }

    fn cancel(&mut self) {
        self.wait.reset();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::quester::families::tests::{local_player, post_user_input_walk_receipt, with_tick};
    use api::quest_progress::EvidenceStamp;
    use api::selected::RunKey;
    use api::snapshot::{GameSnapshot, WorldTile};

    /// The retained per-walk state: 88 B inline, including the quest-list
    /// wait state, and one 112 B boxed replay. The replay keeps everything but
    /// the target, radius and request id that the correlation core holds. A
    /// walk the host dropped unsent in a guardian hold is sent again from it,
    /// field for field.
    #[test]
    fn a_walk_keeps_one_boxed_replay_and_resends_the_same_request_from_it() {
        use crate::native::ledger::TickBudget;
        use crate::native::HostEffect;
        use crate::quester::families::tests::{host_drops_queued_work, with_tick_budget};
        use std::sync::Arc;
        println!(
            "Walk={} WalkReplay={} retained_bytes={}",
            std::mem::size_of::<Walk>(),
            std::mem::size_of::<WalkReplay>(),
            std::mem::size_of::<Walk>() + std::mem::size_of::<WalkReplay>(),
        );
        assert_eq!(std::mem::size_of::<Walk>(), 88);
        assert_eq!(std::mem::size_of::<WalkReplay>(), 112);

        let target = WorldTile {
            x: 3220,
            z: 3200,
            level: 0,
        };
        let mut snapshot = GameSnapshot::new();
        snapshot.seed_ingame(2);
        snapshot.seed_local_player(local_player(WorldTile { x: 3200, ..target }));
        let (mut ledger, mut budget) = (None, TickBudget::default());
        let handle = with_tick_budget(&snapshot, &mut ledger, &mut budget, 1, |tick| {
            let mut request =
                crate::quester::families::reach::walk_request(target, 3, None, tick.cx.evidence());
            request.loc_id = Some(1530);
            request.options.allow_wilderness = super::super::WalkBit::Allow;
            request.cross = Box::from([Arc::from("test-zone")]);
            request.avoid = Box::from([Arc::from("avoided-zone")]);
            request.protect = true;
            request.allow.food = false;
            tick.actions.begin::<Walk>(request, &mut tick.cx).unwrap()
        });
        let first = ledger.as_ref().unwrap().outbox[0].request_id.get();
        budget.hold_begin();
        assert_eq!(host_drops_queued_work(&mut ledger), 1);
        budget.hold_end();
        for tick in [9, 10] {
            assert!(
                with_tick_budget(&snapshot, &mut ledger, &mut budget, tick, |tick| {
                    tick.actions.poll(&handle, &mut tick.cx)
                })
                .is_pending()
            );
        }
        let outbox = &ledger.as_ref().unwrap().outbox;
        assert_eq!(outbox.len(), 1, "one walk is sent again");
        assert_ne!(outbox[0].request_id.get(), first);
        assert_eq!(
            handle.inspect(Walk::request_id),
            Some(outbox[0].request_id.get()),
            "a nested owner cancels the resumed request, not the original"
        );
        let HostEffect::Walk(request) = &outbox[0].effect else {
            panic!("the resend is a walk");
        };
        assert_eq!((request.target, request.radius), (target, 3));
        assert_eq!(request.loc_id, Some(1530));
        assert_eq!(
            request.required_after.tick, 1,
            "the original evidence bound"
        );
        assert_eq!(
            request.options.allow_wilderness,
            super::super::WalkBit::Allow
        );
        assert_eq!(request.cross.as_ref(), [Arc::<str>::from("test-zone")]);
        assert_eq!(request.avoid.as_ref(), [Arc::<str>::from("avoided-zone")]);
        assert!(request.protect);
        assert!(request.food_guard, "named consent retains its food guard");
        assert!(request.allow.food);
    }

    #[test]
    fn moving_radius_arrival_uses_network_origin_and_rejects_stale_flood() {
        let logical = WorldTile {
            x: 3200,
            z: 3200,
            level: 0,
        };
        let rendered = WorldTile { x: 3201, ..logical };
        let target = WorldTile { x: 3202, ..logical };
        let mut player = local_player(rendered);
        player.player.network = logical;
        player.player.actor.moving = true;
        let mut snapshot = GameSnapshot::new();
        snapshot.seed_ingame(2);
        snapshot.seed_local_player(player);
        snapshot.seed_tile(logical);
        let stamp = EvidenceStamp {
            run: RunKey {
                slot: 1,
                run: 1,
                session: 1,
            },
            tick: 1,
            sequence: 1,
        };
        let mut reach = api::query::ReachQueryView::unavailable();
        reach.available = true;
        reach.base_x = logical.x;
        reach.base_z = logical.z;
        reach.level = logical.level;
        reach.width = 3;
        reach.height = 1;
        reach.walkable = vec![7];
        reach.reachable = vec![7];
        reach.reachable_adj = vec![7];
        reach.exact_rank = vec![0, 1, 2];
        reach.adjacent_rank = vec![0, 0, 1];
        let key = WalkKey {
            tile: target,
            radius: 2,
            allow_teleports: Some(false),
        };
        let arrived = |snapshot: &GameSnapshot, reach: &api::query::ReachQueryView, key| {
            let view = SnapshotView::new(Some(snapshot), stamp).with_reach(Some(reach));
            assert_eq!(view.here().map(|here| here.value), Some(rendered));
            Frame {
                snapshot: view,
                outcome: HostOutcome::empty(),
                loc_id: None,
                arrival: ArrivalKind::Reach,
                candidate_goals: false,
                goal: None,
            }
            .arrived(key)
        };
        assert!(arrived(&snapshot, &reach, key));
        assert!(!arrived(
            &snapshot,
            &reach,
            WalkKey {
                tile: WorldTile { level: 1, ..target },
                ..key
            }
        ));
        reach.exact_rank[0] = 1;
        assert!(
            !arrived(&snapshot, &reach, key),
            "another flood origin cannot prove arrival"
        );
        reach.exact_rank[0] = 0;
        reach.available = false;
        assert!(!arrived(&snapshot, &reach, key));
    }
    #[test]
    fn area_arrival_uses_loaded_standability_without_reaching_the_centre() {
        let here = WorldTile {
            x: 2840,
            z: 3436,
            level: 0,
        };
        let centre = WorldTile {
            x: 2848,
            z: 3426,
            level: 0,
        };
        let mut snapshot = GameSnapshot::new();
        snapshot.seed_ingame(2);
        snapshot.seed_local_player(local_player(here));
        let mut reach = api::query::ReachQueryView::unavailable();
        reach.available = true;
        reach.base_x = here.x;
        reach.base_z = here.z;
        reach.level = here.level;
        reach.width = 1;
        reach.height = 1;
        reach.walkable = vec![1];
        let stamp = EvidenceStamp {
            run: RunKey {
                slot: 1,
                run: 1,
                session: 1,
            },
            tick: 1,
            sequence: 1,
        };
        let key = WalkKey {
            tile: centre,
            radius: 40,
            allow_teleports: Some(false),
        };
        let arrived = |arrival, loc_id, view: &api::query::ReachQueryView| {
            Frame {
                snapshot: SnapshotView::new(Some(&snapshot), stamp).with_reach(Some(view)),
                outcome: HostOutcome::empty(),
                loc_id,
                candidate_goals: false,
                goal: None,
                arrival,
            }
            .arrived(key)
        };
        assert!(arrived(ArrivalKind::Area, None, &reach));
        assert!(!arrived(ArrivalKind::Reach, None, &reach));
        assert!(!arrived(ArrivalKind::Area, Some(1), &reach));
        reach.walkable[0] = 0;
        assert!(!arrived(ArrivalKind::Area, None, &reach));
        reach.walkable[0] = 1;
        reach.available = false;
        assert!(!arrived(ArrivalKind::Area, None, &reach));
    }
    #[test]
    fn user_input_receipt_precedes_arrival_deadline_and_required_evidence() {
        let run = RunKey {
            slot: 1,
            run: 1,
            session: 1,
        };
        let target = WorldTile {
            x: 3200,
            z: 3200,
            level: 0,
        };
        let mut initial = GameSnapshot::new();
        initial.seed_ingame(2);
        let mut at_target = GameSnapshot::new();
        at_target.seed_ingame(2);
        at_target.seed_local_player(local_player(target));

        let mut ledger = None;
        let handle = with_tick(&initial, &mut ledger, 1, |tick| {
            tick.actions
                .begin::<Walk>(
                    crate::quester::families::reach::walk_request(
                        target,
                        1,
                        None,
                        EvidenceStamp {
                            run,
                            tick: 1002,
                            sequence: 1002,
                        },
                    ),
                    &mut tick.cx,
                )
                .unwrap()
        });
        let request_id = post_user_input_walk_receipt(&mut ledger, 1001);
        let result = with_tick(&at_target, &mut ledger, 1001, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        });

        let Poll::Ready(Ok(receipt)) = result else {
            panic!("correlated user input must finish the walk: {result:?}");
        };
        assert_eq!(receipt.request_id, request_id);
        assert_eq!(receipt.end, WalkEnd::UserInput);
        assert!(receipt.blocked.is_none());
        assert!(receipt.detail.is_none());
    }
    #[test]
    fn aborted_receipt_cannot_become_arrived_at_the_requested_destination() {
        let run = RunKey {
            slot: 1,
            run: 1,
            session: 1,
        };
        let target = WorldTile {
            x: 3200,
            z: 3200,
            level: 0,
        };
        let mut initial = GameSnapshot::new();
        initial.seed_ingame(2);
        let mut at_target = GameSnapshot::new();
        at_target.seed_ingame(2);
        at_target.seed_local_player(local_player(target));
        let required_after = EvidenceStamp {
            run,
            tick: 2,
            sequence: 2,
        };

        let mut ledger = None;
        let handle = with_tick(&initial, &mut ledger, 1, |tick| {
            tick.actions
                .begin::<Walk>(
                    crate::quester::families::reach::walk_request(target, 1, None, required_after),
                    &mut tick.cx,
                )
                .unwrap()
        });
        let request_id = post_user_input_walk_receipt(&mut ledger, 2);
        let receipt = ledger.as_mut().unwrap().walk.as_mut().unwrap();
        receipt.end = WalkEnd::Aborted;
        receipt.detail = Some(std::sync::Arc::from("no way out"));

        let result = with_tick(&at_target, &mut ledger, 2, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        });
        let Poll::Ready(Ok(receipt)) = result else {
            panic!("the escape receipt must finish the walk: {result:?}");
        };
        assert_eq!(receipt.request_id, request_id);
        assert_eq!(receipt.end, WalkEnd::Aborted);
        assert_eq!(receipt.failure_detail().as_deref(), Some("no way out"));
        assert!(matches!(
            receipt.into_arrival(),
            Err(ActionError::Blocked(detail)) if detail.as_ref() == "no way out"
        ));
    }
    fn complete_gate(quest: &str) -> api::selected::QuestGate {
        api::selected::QuestGate::Complete(api::selected::FactKey::new(quest))
    }

    fn post_needs_evidence_receipt(
        ledger: &mut Option<Box<crate::native::ledger::Ledger>>,
        tick: u64,
        quest: &str,
    ) -> u64 {
        let request_id = post_user_input_walk_receipt(ledger, tick);
        let receipt = ledger
            .as_mut()
            .expect("ledger remains present")
            .walk
            .as_mut()
            .expect("walk receipt exists");
        receipt.end = WalkEnd::NeedsEvidence(std::sync::Arc::from([complete_gate(quest)]));
        request_id
    }

    #[test]
    fn observed_not_started_quest_replans_to_an_arriving_route() {
        assert_observed_quest_replans("biohazard", "Biohazard");
    }

    #[test]
    fn observed_quest_display_name_replans_when_it_differs_from_the_id() {
        assert_observed_quest_replans("priestperil", "Priest in Peril");
    }

    #[test]
    #[ignore = "requires NAV_PACK naming a real quest-gated pack"]
    fn pack_completion_gates_resolve_observed_catalog_rows() {
        let path = std::env::var_os("NAV_PACK").expect("explicit real quest-gated pack");
        let world = nav_planner::world::NavWorld::load_pack(std::path::Path::new(&path))
            .expect("load real quest-gated pack");
        let quests = prepared_quests();
        let mut snapshot = GameSnapshot::new();
        snapshot.seed_ingame(2);
        snapshot.seed_quest_statuses(
            quests
                .iter()
                .map(|facts| api::snapshot::QuestStatusView {
                    component_id: 0,
                    name: facts.display.to_string(),
                    colour: 0xF80000,
                })
                .collect(),
            true,
        );
        let stamp = EvidenceStamp {
            run: RunKey {
                slot: 1,
                run: 1,
                session: 1,
            },
            tick: 1,
            sequence: 1,
        };
        let view = SnapshotView::new(Some(&snapshot), stamp);
        let ids: std::collections::BTreeSet<_> = world
            .graph
            .edges
            .iter()
            .filter_map(|edge| edge.quest_gates.as_ref())
            .flat_map(|gates| gates.gates())
            .filter_map(|gate| match gate {
                QuestGate::Complete(quest) => Some(&quest.0),
                QuestGate::Window(_) => None,
            })
            .collect();
        assert!(!ids.is_empty(), "the pack must contain completion gates");
        for id in &ids {
            let gate = complete_gate(id);
            let facts = quests.quest(id).expect("gate has a catalog identity");
            assert!(
                quest_list_status_observed(&[gate], Some(&quests), view),
                "completion gate {id} must resolve its catalog row"
            );
            println!("QUEST_ROW\t{id}\t{}\tobserved", facts.display);
        }
        println!("QUEST_ROW_COUNT={}", ids.len());
    }

    fn assert_observed_quest_replans(quest: &str, display: &str) {
        let _quests = prepared_quests();
        println!(
            "Walk={} WalkReplay={} retained_bytes={}",
            std::mem::size_of::<Walk>(),
            std::mem::size_of::<WalkReplay>(),
            std::mem::size_of::<Walk>() + std::mem::size_of::<WalkReplay>(),
        );
        let run = RunKey {
            slot: 1,
            run: 1,
            session: 1,
        };
        let target = WorldTile {
            x: 3200,
            z: 3200,
            level: 0,
        };
        let mut initial = GameSnapshot::new();
        initial.seed_ingame(2);
        let mut observed = GameSnapshot::new();
        observed.seed_ingame(2);
        observed.seed_quest_statuses(
            vec![api::snapshot::QuestStatusView {
                component_id: 42,
                name: display.to_owned(),
                colour: 0xF80000,
            }],
            true,
        );
        observed.seed_local_player(local_player(target));
        let mut ledger = None;
        let handle = with_tick(&initial, &mut ledger, 1, |tick| {
            let mut request = crate::quester::families::reach::walk_request(
                target,
                1,
                None,
                EvidenceStamp {
                    run,
                    tick: 1,
                    sequence: 1,
                },
            );
            request.options.allow_wilderness = super::super::WalkBit::Allow;
            request.cross = Box::from([Arc::from("test-zone")]);
            request.avoid = Box::from([Arc::from("avoided-zone")]);
            request.protect = true;
            request.allow.food = false;
            request.allow.escape = false;
            tick.actions.begin::<Walk>(request, &mut tick.cx).unwrap()
        });
        let original = post_needs_evidence_receipt(&mut ledger, 1, quest);

        assert!(matches!(
            with_tick(&initial, &mut ledger, 1, |tick| tick
                .actions
                .poll(&handle, &mut tick.cx)),
            Poll::Pending
        ));

        assert!(matches!(
            with_tick(&observed, &mut ledger, 2, |tick| tick
                .actions
                .poll(&handle, &mut tick.cx)),
            Poll::Pending
        ));

        let retried_request_id = {
            let queued = ledger
                .as_ref()
                .expect("ledger remains present")
                .outbox
                .last()
                .expect("fresh route is queued");
            assert_ne!(queued.request_id.get(), original);
            let crate::native::HostEffect::Walk(request) = &queued.effect else {
                panic!("fresh route must retain the walk request");
            };
            assert_eq!(request.required_after.tick, 2);
            assert_eq!(request.target, target);
            assert_eq!(request.radius, 1);
            assert_eq!(
                request.options.allow_wilderness,
                super::super::WalkBit::Allow
            );
            assert_eq!(request.cross.as_ref(), [Arc::<str>::from("test-zone")]);
            assert_eq!(request.avoid.as_ref(), [Arc::<str>::from("avoided-zone")]);
            assert!(request.protect);
            assert!(request.food_guard, "named consent retains its food guard");
            assert!(request.allow.food);
            assert!(!request.allow.escape);
            queued.request_id.get()
        };
        assert_eq!(handle.inspect(Walk::request_id), Some(retried_request_id));
        let retried = post_user_input_walk_receipt(&mut ledger, 2);
        assert_eq!(retried, retried_request_id);
        ledger
            .as_mut()
            .expect("ledger remains present")
            .walk
            .as_mut()
            .expect("fresh route receipt exists")
            .end = WalkEnd::Arrived;
        let Poll::Ready(Ok(receipt)) = with_tick(&observed, &mut ledger, 3, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        }) else {
            panic!("an observed not-started quest permits the refreshed route");
        };
        assert_eq!(receipt.request_id, retried);
        assert!(matches!(receipt.end, WalkEnd::Arrived));
        assert_eq!(
            ledger
                .as_ref()
                .unwrap()
                .outbox
                .iter()
                .filter(|entry| matches!(&entry.effect, crate::native::HostEffect::Walk(_)))
                .count(),
            2,
            "only one fresh route is queued for the observed status"
        );
    }

    fn prepared_quests() -> Arc<api::quest_facts::QuestCatalog> {
        api::selected::FamilyPreparation::run(|worker| {
            api::game_data::for_revision(api::selected::ClientRevision::R289)
                .unwrap()
                .prepare_quests(worker)
                .unwrap()
        })
        .unwrap()
        .join()
        .unwrap()
    }

    #[test]
    fn no_path_receipt_finishes_on_the_next_observation_with_host_detail() {
        let run = RunKey {
            slot: 1,
            run: 1,
            session: 1,
        };
        let target = WorldTile {
            x: 3200,
            z: 3200,
            level: 0,
        };
        let required_after = EvidenceStamp {
            run,
            tick: 1,
            sequence: 1,
        };
        let mut snapshot = GameSnapshot::new();
        snapshot.seed_ingame(2);

        let mut ledger = None;
        let handle = with_tick(&snapshot, &mut ledger, 1, |tick| {
            tick.actions
                .begin::<Walk>(
                    crate::quester::families::reach::walk_request(target, 1, None, required_after),
                    &mut tick.cx,
                )
                .unwrap()
        });
        let request_id = post_user_input_walk_receipt(&mut ledger, 2);
        let detail = "target is unreachable from this scene";
        {
            let receipt = ledger.as_mut().unwrap().walk.as_mut().unwrap();
            receipt.end = WalkEnd::NoPath;
            receipt.detail = Some(std::sync::Arc::from(detail));
        }

        let result = with_tick(&snapshot, &mut ledger, 2, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        });
        let Poll::Ready(Ok(receipt)) = result else {
            panic!("the next observation must deliver the no-path receipt: {result:?}");
        };
        assert_eq!(receipt.request_id, request_id);
        assert_eq!(receipt.end, WalkEnd::NoPath);
        assert_eq!(receipt.failure_detail().as_deref(), Some(detail));
        assert!(matches!(
            receipt.into_arrival(),
            Err(ActionError::Blocked(reason)) if reason.as_ref() == detail
        ));
    }

    #[test]
    fn host_no_path_receipt_takes_priority_over_the_deadline() {
        let run = RunKey {
            slot: 1,
            run: 1,
            session: 1,
        };
        let target = WorldTile {
            x: 3200,
            z: 3200,
            level: 0,
        };
        let required_after = EvidenceStamp {
            run,
            tick: 1,
            sequence: 1,
        };
        let mut snapshot = GameSnapshot::new();
        snapshot.seed_ingame(2);

        let mut ledger = None;
        let handle = with_tick(&snapshot, &mut ledger, 1, |tick| {
            tick.actions
                .begin::<Walk>(
                    crate::quester::families::reach::walk_request(target, 1, None, required_after),
                    &mut tick.cx,
                )
                .unwrap()
        });
        let request_id = post_user_input_walk_receipt(&mut ledger, 1001);
        let detail = "host proved that no route exists";
        {
            let receipt = ledger.as_mut().unwrap().walk.as_mut().unwrap();
            receipt.end = WalkEnd::NoPath;
            receipt.detail = Some(std::sync::Arc::from(detail));
        }

        let result = with_tick(&snapshot, &mut ledger, 1001, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        });
        let Poll::Ready(Ok(receipt)) = result else {
            panic!("a host terminal receipt must take priority at the deadline: {result:?}");
        };
        assert_eq!(receipt.request_id, request_id);
        assert_eq!(receipt.end, WalkEnd::NoPath);
        assert_eq!(receipt.failure_detail().as_deref(), Some(detail));
    }

    #[test]
    fn settled_wait_without_a_receipt_still_obeys_the_deadline() {
        let required_after = EvidenceStamp {
            run: RunKey {
                slot: 1,
                run: 1,
                session: 1,
            },
            tick: 1,
            sequence: 1,
        };
        let target = WorldTile {
            x: 3200,
            z: 3200,
            level: 0,
        };
        let mut snapshot = GameSnapshot::new();
        snapshot.seed_ingame(2);
        let mut ledger = None;
        let handle = with_tick(&snapshot, &mut ledger, 1, |tick| {
            tick.actions
                .begin::<Walk>(
                    crate::quester::families::reach::walk_request(target, 1, None, required_after),
                    &mut tick.cx,
                )
                .unwrap()
        });
        let mut arrived = GameSnapshot::new();
        arrived.seed_ingame(2);
        arrived.seed_local_player(local_player(target));
        {
            let mut retained = handle.machine.borrow_mut();
            let machine = retained.as_mut().expect("the walk is active");
            assert!(machine.wait.poll(
                machine.request_id(),
                &Frame {
                    snapshot: SnapshotView::new(Some(&arrived), required_after),
                    outcome: HostOutcome::empty(),
                    loc_id: None,
                    arrival: ArrivalKind::Reach,
                    candidate_goals: false,
                    goal: None,
                },
            ));
        }
        assert!(with_tick(&snapshot, &mut ledger, 2, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        })
        .is_pending());
        let result = with_tick(&snapshot, &mut ledger, 1001, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        });
        let Poll::Ready(Ok(receipt)) = result else {
            panic!("a settled wait without its receipt must expire: {result:?}");
        };
        assert_eq!(receipt.end, WalkEnd::Failed);
        assert_eq!(
            receipt.failure_detail().as_deref(),
            Some("walk receipt deadline expired before the host reported a terminal result")
        );
    }

    #[test]
    fn deadline_failure_reports_why_the_host_receipt_is_missing() {
        let run = RunKey {
            slot: 1,
            run: 1,
            session: 1,
        };
        let target = WorldTile {
            x: 3200,
            z: 3200,
            level: 0,
        };
        let required_after = EvidenceStamp {
            run,
            tick: 1,
            sequence: 1,
        };
        let mut snapshot = GameSnapshot::new();
        snapshot.seed_ingame(2);

        let mut ledger = None;
        let handle = with_tick(&snapshot, &mut ledger, 1, |tick| {
            tick.actions
                .begin::<Walk>(
                    crate::quester::families::reach::walk_request(target, 1, None, required_after),
                    &mut tick.cx,
                )
                .unwrap()
        });

        let result = with_tick(&snapshot, &mut ledger, 1001, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        });
        let Poll::Ready(Ok(receipt)) = result else {
            panic!("the deadline must end a walk without a host receipt: {result:?}");
        };
        assert_eq!(receipt.end, WalkEnd::Failed);
        assert_eq!(
            receipt.failure_detail().as_deref(),
            Some("walk receipt deadline expired before the host reported a terminal result")
        );
    }
}
