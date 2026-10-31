//! Native adapter for the shared walk correlation and arrival core.
use super::walk_wait::{HostOutcome, Observation, WalkKey, WalkSlot};
use super::{
    ActionContext, ActionError, HoldResume, NativeMachine, WalkEnd, WalkReceipt, WalkRequest,
};
use api::snapshot::SnapshotView;
use nav_planner::contract::ArrivalKind;
use std::num::NonZeroU64;
use std::task::Poll;
use std::time::Duration;

/// Eligible time a native walk may stay unsettled before it ends `Failed`
/// and revokes its host follow. The host owes every walk a terminal receipt;
/// this is the machine's backstop, measured on the active clock so pause,
/// hold, not-ready and Blocked intervals do not spend it.
pub const WALK_DEADLINE: Duration = Duration::from_secs(10 * 60);

pub struct Walk {
    // Retain only fields missing from the correlation core, which already
    // owns the target, radius and request ID ([`WalkSlot::request`]).
    replay: Box<WalkReplay>,
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
    evidence: Option<std::sync::Arc<dyn api::quest_progress::EvidenceProvider>>,
    cross: Box<[std::sync::Arc<str>]>,
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
            protect: request.protect,
            food_guard: request.food_guard,
            allow: request.allow,
        })
    }

    /// The request again, to the correlation core's target and radius.
    fn request(&self, key: WalkKey) -> WalkRequest {
        WalkRequest {
            target: key.tile,
            radius: u16::try_from(key.radius).expect("walk radius originated as u16"),
            loc_id: self.loc_id,
            arrival: self.arrival,
            required_after: self.required_after,
            candidate_goals: self.candidate_goals.clone(),
            options: self.options,
            evidence: self.evidence.clone(),
            cross: self.cross.clone(),
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

impl Walk {
    /// The host walk request this machine began. Dropping the machine does
    /// not revoke the host follow, so a caller that abandons a walk passes
    /// this to [`ActionContext::cancel_request`].
    pub(crate) fn request_id(&self) -> u64 {
        self.request_id
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
        let request_id = cx.walk(self.replay.request(key))?;
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
        if cx.active_now() >= self.deadline {
            cx.cancel_request(request_id);
            return Poll::Ready(Ok(WalkReceipt {
                goal: None,
                request_id,
                evidence: cx.evidence(),
                end: WalkEnd::Failed,
                blocked: None,
                detail: None,
                refusal: None,
                assessment: None,
                escape: None,
            }));
        }
        if !cx.evidence().meets(self.replay.required_after) {
            return Poll::Pending;
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
            }));
        }
        match receipt {
            Some(receipt) => Poll::Ready(Ok(receipt.clone())),
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

    /// The retained per-walk state (QUESTER-READ-KEEPS-STEP-R3, M1): 64 B
    /// inline and one boxed replay, which keeps everything of the request but
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
        assert_eq!(std::mem::size_of::<Walk>(), 64);
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
        assert!(request.protect);
        assert!(!request.allow.food);
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
}
