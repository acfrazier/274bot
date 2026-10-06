//! Native adapter for the shared walk correlation and arrival core.
use super::walk_wait::{HostOutcome, Observation, WalkKey, WalkSlot};
use super::{ActionContext, ActionError, NativeMachine, WalkEnd, WalkReceipt, WalkRequest};
use api::quest_progress::EvidenceStamp;
use api::snapshot::SnapshotView;
use nav::arrival::ArrivalKind;
use std::num::NonZeroU64;
use std::task::Poll;
use std::time::Duration;

/// Eligible time a native walk may stay unsettled before it ends `Failed`
/// and revokes its host follow. The host owes every walk a terminal receipt;
/// this is the machine's backstop, measured on the active clock so pause,
/// hold, not-ready and Blocked intervals do not spend it.
pub const WALK_DEADLINE: Duration = Duration::from_secs(10 * 60);

pub struct Walk {
    key: WalkKey,
    loc_id: Option<i32>,
    arrival: ArrivalKind,
    request_id: u64,
    required_after: EvidenceStamp,
    deadline: Duration,
    wait: WalkSlot,
}

struct Frame<'a> {
    snapshot: SnapshotView<'a>,
    outcome: HostOutcome,
    loc_id: Option<i32>,
    arrival: ArrivalKind,
}

impl Observation for Frame<'_> {
    fn outcome(&self) -> HostOutcome {
        self.outcome
    }

    fn cancelled(&self) -> bool {
        false // The action owner fence rejects revoked polls before entry.
    }

    fn arrived(&self, key: WalkKey) -> bool {
        let Some(here) = self.snapshot.here() else {
            return false;
        };
        match (self.arrival, self.loc_id) {
            (ArrivalKind::Area, None) => {
                nav::arrival::arrived_in_area(here.value, key.tile, key.radius, |tile| {
                    self.snapshot
                        .reach()
                        .is_some_and(|reach| reach.value.walkable(tile))
                })
            }
            (ArrivalKind::Area, Some(_)) => false,
            (ArrivalKind::Reach, Some(id)) => self
                .snapshot
                .walk_loc_arrived(here.value, key.tile, key.radius, id),
            (ArrivalKind::Reach, None) => {
                self.snapshot.walk_arrived(here.value, key.tile, key.radius)
            }
        }
    }
    fn moving(&self) -> Option<bool> {
        self.snapshot
            .local_player()
            .map(|player| player.value.player.actor.moving)
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
        let request_id = cx.walk(request)?;
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
            request_id,
            required_after,
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
                request_id: self.request_id,
                evidence: cx.evidence(),
                end: WalkEnd::Failed,
                blocked: None,
                detail: None,
            }));
        }
        if !cx.evidence().meets(self.required_after) {
            return Poll::Pending;
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
        };
        if !self.wait.poll(self.request_id, &frame) {
            return Poll::Pending;
        }
        if frame.arrived(self.key) {
            return Poll::Ready(Ok(WalkReceipt {
                request_id: self.request_id,
                evidence: cx.evidence(),
                end: WalkEnd::Arrived,
                blocked: None,
                detail: None,
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
}
