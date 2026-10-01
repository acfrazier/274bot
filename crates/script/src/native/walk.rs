//! Native adapter for the shared walk correlation and arrival core.
use super::walk_wait::{HostOutcome, Observation, WalkKey, WalkSlot};
use super::{ActionContext, ActionError, NativeMachine, WalkEnd, WalkReceipt, WalkRequest};
use api::quest_progress::EvidenceStamp;
use api::snapshot::SnapshotView;
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
    request_id: u64,
    required_after: EvidenceStamp,
    deadline: Duration,
    wait: WalkSlot,
}

struct Frame<'a> {
    snapshot: SnapshotView<'a>,
    outcome: HostOutcome,
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
        self.snapshot.walk_arrived(here.value, key.tile, key.radius)
    }
}

impl NativeMachine for Walk {
    type Args = WalkRequest;
    type Output = WalkReceipt;

    fn begin(request: Self::Args, cx: &mut ActionContext<'_>) -> Result<Self, ActionError> {
        let key = WalkKey {
            tile: request.target,
            radius: i32::from(request.radius),
            allow_teleports: request.options.allow_teleports,
        };
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
            request_id,
            required_after,
            deadline: cx.active_now().saturating_add(WALK_DEADLINE),
            wait,
        })
    }

    fn poll(&mut self, cx: &mut ActionContext<'_>) -> Poll<Result<Self::Output, ActionError>> {
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
