//! Native adapter for the shared walk correlation and arrival core.
use super::walk_wait::{HostOutcome, Observation, WalkKey, WalkSlot};
use super::{ActionContext, ActionError, NativeMachine, WalkEnd, WalkReceipt, WalkRequest};
use api::quest_progress::EvidenceStamp;
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
/// Login's `initalltabs` sends the quest tab and all row colours together.
/// One later observed game tick is enough for the client snapshot to catch up.
const QUEST_LIST_STATUS_WAIT_TICKS: u64 = 1;

pub struct Walk {
    key: WalkKey,
    loc_id: Option<i32>,
    arrival: ArrivalKind,
    // Retain only arrival mode. The host retains the goals and reports their tier.
    candidate_goals: bool,
    request: WalkRequest,
    request_id: u64,
    required_after: EvidenceStamp,
    quest_list_wait_since: Option<u64>,
    quest_list_retry_used: bool,
    deadline: Duration,
    wait: WalkSlot,
}

struct Frame<'a> {
    snapshot: SnapshotView<'a>,
    outcome: HostOutcome,
    loc_id: Option<i32>,
    arrival: ArrivalKind,
    candidate_goals: bool,
    goal: Option<api::WorldTile>,
}

fn quest_list_status_observed(snapshot: SnapshotView<'_>) -> bool {
    snapshot.quest_statuses().is_some_and(|statuses| {
        statuses.value.iter().any(|status| {
            status.status() != api::snapshot::QuestListStatus::Unknown
        })
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

impl Walk {
    fn replan_with_current_quest_list(
        &mut self,
        cx: &mut ActionContext<'_>,
    ) -> Result<(), ActionError> {
        let required_after = cx.evidence();
        let mut request = self.request.clone();
        request.required_after = required_after;
        let request_id = cx.walk(request)?;
        self.request.required_after = required_after;
        self.required_after = required_after;
        self.request_id = request_id;
        self.quest_list_wait_since = None;
        self.quest_list_retry_used = true;
        self.wait.begin(
            NonZeroU64::new(request_id).expect("host request is nonzero"),
            self.key,
            HostOutcome::empty(),
        );
        Ok(())
    }
}

impl NativeMachine for Walk {
    type Args = WalkRequest;
    type Output = WalkReceipt;

    fn begin(request: Self::Args, cx: &mut ActionContext<'_>) -> Result<Self, ActionError> {
        let key = WalkKey {
            tile: request.target,
            radius: i32::from(request.radius),
            allow_teleports: request.options.allow_teleports.explicit(),
        };
        let loc_id = request.loc_id;
        let arrival = request.arrival;
        let required_after = request.required_after;
        let candidate_goals = request.candidate_goals.is_some();
        let request_id = cx.walk(request.clone())?;
        let mut wait = WalkSlot::new();
        wait.begin(
            NonZeroU64::new(request_id).expect("host request is nonzero"),
            key,
            HostOutcome::empty(),
        );
        Ok(Self {
            key,
            loc_id,
            arrival,
            candidate_goals,
            request,
            request_id,
            required_after,
            quest_list_wait_since: None,
            quest_list_retry_used: false,
            deadline: cx.active_now().saturating_add(WALK_DEADLINE),
            wait,
        })
    }

    fn poll(&mut self, cx: &mut ActionContext<'_>) -> Poll<Result<Self::Output, ActionError>> {
        if let Some(receipt) = cx
            .walk_receipt(self.request_id)
            .filter(|receipt| receipt.end == WalkEnd::UserInput)
        {
            return Poll::Ready(Ok(receipt.clone()));
        }
        if cx.active_now() >= self.deadline {
            cx.cancel_request(self.request_id);
            return Poll::Ready(Ok(WalkReceipt {
                goal: None,
                request_id: self.request_id,
                evidence: cx.evidence(),
                end: WalkEnd::Failed,
                blocked: None,
                detail: None,
                refusal: None,
                assessment: None,
                escape: None,
            }));
        }
        if !cx.evidence().meets(self.required_after) {
            return Poll::Pending;
        }

        let needs_quest_evidence = cx
            .walk_receipt(self.request_id)
            .is_some_and(|receipt| {
                receipt.evidence.meets(self.required_after)
                    && matches!(&receipt.end, WalkEnd::NeedsEvidence(_))
            });
        if needs_quest_evidence {
            let receipt = cx
                .walk_receipt(self.request_id)
                .expect("quest-evidence receipt remains correlated")
                .clone();
            if self.quest_list_retry_used {
                return Poll::Ready(Ok(receipt));
            }
            let started = self
                .quest_list_wait_since
                .get_or_insert_with(|| cx.evidence().tick);
            if cx.evidence().tick.saturating_sub(*started) < QUEST_LIST_STATUS_WAIT_TICKS {
                return Poll::Pending;
            }
            if quest_list_status_observed(cx.snapshot()) {
                if let Err(error) = self.replan_with_current_quest_list(cx) {
                    return Poll::Ready(Err(error));
                }
                return Poll::Pending;
            }
            let mut refused = receipt;
            refused.detail = Some(std::sync::Arc::from(
                "quest-list status was not observed after one game tick",
            ));
            return Poll::Ready(Ok(refused));
        }
        let receipt = cx
            .walk_receipt(self.request_id)
            .filter(|receipt| receipt.evidence.meets(self.required_after));
        let outcome = receipt.map_or_else(HostOutcome::empty, |receipt| HostOutcome {
            seq: receipt.evidence.sequence,
            generation: receipt.evidence.run.run,
            request_id: receipt.request_id,
            failed: !matches!(
                receipt.end,
                WalkEnd::Arrived | WalkEnd::RouteEnded | WalkEnd::Blocked
            ),
            blocked: receipt.end == WalkEnd::Blocked,
            key: self.key,
        });
        let frame = Frame {
            snapshot: cx.snapshot(),
            outcome,
            loc_id: self.loc_id,
            arrival: self.arrival,
            candidate_goals: self.candidate_goals,
            goal: receipt.and_then(|receipt| receipt.goal),
        };
        if !self.wait.poll(self.request_id, &frame) {
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

        if frame.arrived(self.key) {
            let detail = receipt.and_then(|receipt| receipt.detail.clone());
            return Poll::Ready(Ok(WalkReceipt {
                goal: frame.goal,
                request_id: self.request_id,
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
    fn biohazard_complete_gate() -> api::selected::QuestGate {
        api::selected::QuestGate::Complete(api::selected::FactKey::new("biohazard"))
    }

    fn post_needs_evidence_receipt(
        ledger: &mut Option<Box<crate::native::ledger::Ledger>>,
        tick: u64,
    ) -> u64 {
        let request_id = post_user_input_walk_receipt(ledger, tick);
        let receipt = ledger
            .as_mut()
            .expect("ledger remains present")
            .walk
            .as_mut()
            .expect("walk receipt exists");
        receipt.end = WalkEnd::NeedsEvidence(std::sync::Arc::from([biohazard_complete_gate()]));
        request_id
    }

    #[test]
    fn unresolved_quest_evidence_waits_one_observed_game_tick() {
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
        let mut snapshot = GameSnapshot::new();
        snapshot.seed_ingame(2);
        let mut ledger = None;
        let handle = with_tick(&snapshot, &mut ledger, 1, |tick| {
            tick.actions
                .begin::<Walk>(
                    crate::quester::families::reach::walk_request(
                        target,
                        1,
                        None,
                        EvidenceStamp {
                            run,
                            tick: 1,
                            sequence: 1,
                        },
                    ),
                    &mut tick.cx,
                )
                .unwrap()
        });
        post_needs_evidence_receipt(&mut ledger, 1);

        for _ in 0..2 {
            assert!(matches!(
                with_tick(&snapshot, &mut ledger, 1, |tick| tick
                    .actions
                    .poll(&handle, &mut tick.cx)),
                Poll::Pending
            ));
        }

        let Poll::Ready(Ok(receipt)) =
            with_tick(&snapshot, &mut ledger, 2, |tick| tick
                .actions
                .poll(&handle, &mut tick.cx))
        else {
            panic!("missing journal status must refuse after one observed tick");
        };
        assert!(matches!(
            &receipt.end,
            WalkEnd::NeedsEvidence(gates) if gates.as_ref() == [biohazard_complete_gate()]
        ));
        assert!(
            receipt
                .failure_detail()
                .is_some_and(|detail| detail.contains("quest-list status was not observed")),
            "the bounded refusal names the missing journal observation"
        );
    }

    #[test]
    fn observed_quest_list_replans_with_a_fresh_route_request() {
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
                name: "Biohazard".to_owned(),
                colour: 0xF80000,
            }],
            true,
        );
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
                            tick: 1,
                            sequence: 1,
                        },
                    ),
                    &mut tick.cx,
                )
                .unwrap()
        });
        let original = post_needs_evidence_receipt(&mut ledger, 1);

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
    }
}
