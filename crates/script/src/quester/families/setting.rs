//! Non-combat option changes and their observed-state predicates.

use super::super::compile::{
    CompileContext, CompileError, PredicateContext, PredicatePlan, StepContext, StepOutcome,
    StepPlan, StepRun,
};
use crate::native::{ActionContext, ActionError, ActionHandle, NativeActions, NativeMachine};
use crate::shim::InteractReq;
use api::quest_progress::EvidenceStamp;
use api::selected::Truth;
use serde::Deserialize;
use std::sync::Arc;
use std::task::Poll;
use std::time::Duration;

/// Required non-combat auto-retaliate state.
#[derive(Deserialize)]
#[cfg_attr(feature = "path-schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub(super) struct SettingArgs {
    /// The required auto-retaliate state. True enables auto-retaliate.
    retaliate: bool,
}

pub(super) fn compile_setting(
    args: SettingArgs,
    _cx: &CompileContext<'_>,
) -> Result<Arc<dyn StepPlan>, CompileError> {
    Ok(Arc::new(SettingPlan {
        retaliate: args.retaliate,
    }))
}

pub(super) fn compile_predicate(
    args: SettingArgs,
    _cx: &CompileContext<'_>,
) -> Result<Arc<dyn PredicatePlan>, CompileError> {
    Ok(Arc::new(SettingPredicate {
        retaliate: args.retaliate,
    }))
}

fn observed_retaliate(cx: &ActionContext<'_>) -> Option<bool> {
    let row = cx
        .snapshot()
        .varps()?
        .value
        .iter()
        .find(|row| row.index == crate::combat::OPTION_NODEF)?;
    match row.value {
        0 => Some(true),
        1 => Some(false),
        _ => None,
    }
}

struct SettingPredicate {
    retaliate: bool,
}
impl PredicatePlan for SettingPredicate {
    fn evaluate(&self, cx: &PredicateContext<'_, '_>) -> Truth {
        match observed_retaliate(cx.cx) {
            Some(value) if value == self.retaliate => Truth::True,
            Some(_) => Truth::False,
            None => Truth::Unknown,
        }
    }
}

struct SettingPlan {
    retaliate: bool,
}
impl StepPlan for SettingPlan {
    fn begin(&self, cx: &mut StepContext<'_, '_>) -> Result<Box<dyn StepRun>, ActionError> {
        Ok(Box::new(SettingRun {
            action: Some(
                cx.tick
                    .actions
                    .begin::<SetRetaliate>(self.retaliate, &mut cx.tick.cx)?,
            ),
        }))
    }
}

struct SettingRun {
    action: Option<ActionHandle<SetRetaliate>>,
}
impl StepRun for SettingRun {
    fn poll(&mut self, cx: &mut StepContext<'_, '_>) -> Poll<Result<StepOutcome, ActionError>> {
        let Some(handle) = &self.action else {
            return Poll::Ready(Err(ActionError::Cancelled));
        };
        cx.tick.actions.poll(handle, &mut cx.tick.cx).map(|result| {
            result.map(|evidence| StepOutcome {
                progress: None,
                evidence,
                receipt: None,
            })
        })
    }
    fn cancel(&mut self, actions: &mut NativeActions) {
        if let Some(handle) = self.action.take() {
            actions.cancel(handle);
        }
    }
}

struct SetRetaliate {
    retaliate: bool,
    request: Option<u64>,
    deadline: Duration,
}
impl NativeMachine for SetRetaliate {
    type Args = bool;
    type Output = EvidenceStamp;

    fn begin(retaliate: bool, cx: &mut ActionContext<'_>) -> Result<Self, ActionError> {
        let request = if observed_retaliate(cx) == Some(retaliate) {
            None
        } else {
            Some(cx.emit(InteractReq::SetRetaliate { on: retaliate })?)
        };
        Ok(Self {
            retaliate,
            request,
            deadline: cx.active_now() + Duration::from_secs(8),
        })
    }

    fn poll(&mut self, cx: &mut ActionContext<'_>) -> Poll<Result<EvidenceStamp, ActionError>> {
        if cx.active_now() >= self.deadline {
            return Poll::Ready(Err(ActionError::Failed(Arc::from(
                "auto-retaliate setting timeout",
            ))));
        }
        if let Some(request) = self.request {
            match cx.interaction_receipt(request) {
                None => return Poll::Pending,
                Some(receipt) if !receipt.accepted => {
                    return Poll::Ready(Err(ActionError::Failed(Arc::from(
                        "auto-retaliate setting rejected",
                    ))));
                }
                Some(_) => self.request = None,
            }
        }
        if observed_retaliate(cx) == Some(self.retaliate) {
            Poll::Ready(Ok(cx.evidence()))
        } else {
            Poll::Pending
        }
    }

    fn cancel(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native::{ledger, HostEffect, InteractionReceipt};
    use crate::quester::families::tests::with_tick;
    use api::snapshot::{GameSnapshot, VarpView};

    fn snapshot(value: Option<i32>) -> GameSnapshot {
        let mut snapshot = GameSnapshot::new();
        snapshot.seed_ingame(2);
        snapshot.seed_varps(
            value
                .map(|value| VarpView {
                    index: crate::combat::OPTION_NODEF,
                    value,
                })
                .into_iter()
                .collect(),
        );
        snapshot
    }

    #[test]
    fn setting_requires_explicit_known_boolean_args() {
        assert!(serde_json::from_value::<SettingArgs>(serde_json::json!({})).is_err());
        assert!(serde_json::from_value::<SettingArgs>(
            serde_json::json!({"retaliate": true, "unknown": false})
        )
        .is_err());
        assert!(
            serde_json::from_value::<SettingArgs>(serde_json::json!({"retaliate": "false"}))
                .is_err()
        );
    }

    #[test]
    fn already_desired_retaliate_state_needs_no_packet() {
        for (value, desired) in [(0, true), (1, false)] {
            let snapshot = snapshot(Some(value));
            let mut ledger = None;
            let handle = with_tick(&snapshot, &mut ledger, 1, |tick| {
                tick.actions
                    .begin::<SetRetaliate>(desired, &mut tick.cx)
                    .unwrap()
            });
            assert!(ledger.as_ref().unwrap().outbox.is_empty());
            assert!(matches!(
                with_tick(&snapshot, &mut ledger, 2, |tick| {
                    tick.actions.poll(&handle, &mut tick.cx)
                }),
                Poll::Ready(Ok(evidence)) if evidence.tick == 2
            ));
        }
    }

    #[test]
    fn setting_requires_dispatch_acceptance_and_observed_desired_state() {
        let mut snapshot = snapshot(Some(0));
        let mut ledger = None;
        let handle = with_tick(&snapshot, &mut ledger, 1, |tick| {
            tick.actions
                .begin::<SetRetaliate>(false, &mut tick.cx)
                .unwrap()
        });
        let action = &ledger.as_ref().unwrap().outbox[0];
        assert!(matches!(
            action.effect,
            HostEffect::Interaction(InteractReq::SetRetaliate { on: false })
        ));
        let authority = action.authority();
        snapshot.seed_varps(vec![VarpView {
            index: crate::combat::OPTION_NODEF,
            value: 1,
        }]);
        assert!(with_tick(&snapshot, &mut ledger, 2, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        })
        .is_pending());
        ledger.as_mut().unwrap().complete_interaction(
            &authority,
            InteractionReceipt {
                request_id: authority.request_id().get(),
                evidence: EvidenceStamp {
                    run: authority.run(),
                    tick: 2,
                    sequence: 2,
                },
                accepted: true,
                chat_since: 0,
            },
        );
        assert!(matches!(
            with_tick(&snapshot, &mut ledger, 3, |tick| {
                tick.actions.poll(&handle, &mut tick.cx)
            }),
            Poll::Ready(Ok(evidence)) if evidence.tick == 3
        ));
    }

    #[test]
    fn unknown_or_unchanged_setting_is_bounded_and_rejection_is_failure() {
        for accepted in [true, false] {
            let snapshot = snapshot(None);
            let mut ledger = None;
            let handle = with_tick(&snapshot, &mut ledger, 1, |tick| {
                tick.actions
                    .begin::<SetRetaliate>(false, &mut tick.cx)
                    .unwrap()
            });
            let authority = ledger.as_ref().unwrap().outbox[0].authority();
            ledger.as_mut().unwrap().complete_interaction(
                &authority,
                InteractionReceipt {
                    request_id: authority.request_id().get(),
                    evidence: EvidenceStamp {
                        run: authority.run(),
                        tick: 1,
                        sequence: 1,
                    },
                    accepted,
                    chat_since: 0,
                },
            );
            let poll = with_tick(
                &snapshot,
                &mut ledger,
                if accepted { 15 } else { 2 },
                |tick| tick.actions.poll(&handle, &mut tick.cx),
            );
            assert!(matches!(poll, Poll::Ready(Err(ActionError::Failed(_)))));
        }
    }

    #[test]
    fn retaliate_predicate_does_not_guess_unobserved_or_invalid_values() {
        for (value, expected) in [
            (None, Truth::Unknown),
            (Some(2), Truth::Unknown),
            (Some(0), Truth::False),
            (Some(1), Truth::True),
        ] {
            let snapshot = snapshot(value);
            let mut ledger: Option<Box<ledger::Ledger>> = None;
            with_tick(&snapshot, &mut ledger, 1, |tick| {
                let quests = api::quest_facts::QuestCatalog::empty();
                let context = PredicateContext {
                    cx: &tick.cx,
                    pairs: tick.pairs,
                    quests: &quests,
                    progress: &[],
                    required_after: tick.cx.evidence(),
                    chat_since: 0,
                    outcome: None,
                };
                assert_eq!(
                    SettingPredicate { retaliate: false }.evaluate(&context),
                    expected
                );
            });
        }
    }
}
