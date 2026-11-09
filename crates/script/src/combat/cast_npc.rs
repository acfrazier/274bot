//! One selected content spell cast on one snapshot-identified NPC.
use std::sync::Arc;
use std::task::Poll;

use api::game_data::{SelectedGameData, SpellFact};
use api::snapshot::NpcView;

use crate::combat::CastNpcRequest;
use crate::combat::{frame::Frame, style::magic};
use crate::interact::InteractReq;
use crate::native::{ActionContext, ActionError, NativeMachine};

#[derive(Debug, Clone, Copy)]
struct AwaitingCast {
    request_id: u64,
    sent_tick: u64,
    sent_sequence: u64,
    chat_since: i32,
    rune_baseline: Option<(i32, i32)>,
    spot_animation_stamp: i32,
}

#[derive(Debug, Clone, Copy)]
enum Phase {
    Dispatch,
    Await(AwaitingCast),
}

/// Cast one selected spell on the NPC index provided by the compatibility API.
pub struct CastNpc {
    selected: Arc<SelectedGameData>,
    spell_index: usize,
    component_id: i32,
    npc_index: i32,
    npc_type: i32,
    npc_name: Option<String>,
    phase: Phase,
}

impl CastNpc {
    fn spell(&self) -> &SpellFact {
        &self.selected.spells()[self.spell_index]
    }

    fn target<'a>(&self, frame: &'a Frame<'_>) -> Option<&'a NpcView> {
        let index = usize::try_from(self.npc_index).ok()?;
        frame.npcs.iter().find(|npc| {
            npc.index == index
                && npc.r#type.and_then(|id| i32::try_from(id).ok()) == Some(self.npc_type)
                && npc.name == self.npc_name
        })
    }

    fn timed_out(sent_tick: u64, cx: &ActionContext<'_>) -> bool {
        cx.evidence().tick.saturating_sub(sent_tick) >= u64::from(magic::CAST_TICKS)
    }
}

impl NativeMachine for CastNpc {
    type Args = (CastNpcRequest, Arc<SelectedGameData>);
    type Output = bool;

    fn begin(
        (request, selected): Self::Args,
        cx: &mut ActionContext<'_>,
    ) -> Result<Self, ActionError> {
        if request.component_id < 0 || request.index < 0 {
            return Err(unavailable("invalid spell or NPC identity"));
        }
        let mut spell_index = None;
        for (index, spell) in selected.spells().iter().enumerate() {
            if spell.component_id == request.component_id {
                if spell_index.is_some() {
                    return Err(unavailable("ambiguous selected spell component"));
                }
                spell_index = Some(index);
            }
        }
        let Some(spell_index) = spell_index else {
            return Err(unavailable("selected spell component is unknown"));
        };
        let npc_index =
            usize::try_from(request.index).map_err(|_| unavailable("invalid NPC identity"))?;
        let snapshot = cx.snapshot();
        let Some(frame) = Frame::borrow(snapshot) else {
            return Err(unavailable("combat snapshot is incomplete"));
        };
        let Some(npc) = frame.npcs.iter().find(|npc| npc.index == npc_index) else {
            return Err(unavailable("NPC is absent"));
        };
        let Some(npc_type) = npc.r#type else {
            return Err(unavailable("NPC type is unavailable"));
        };
        let npc_type = i32::try_from(npc_type)
            .map_err(|_| unavailable("NPC type is outside the selected range"))?;
        let spell = &selected.spells()[spell_index];
        if !magic::castable_npc(spell, &frame, &selected, npc_type) {
            return Err(unavailable("spell is not castable on this NPC"));
        }
        Ok(Self {
            selected,
            spell_index,
            component_id: request.component_id,
            npc_index: request.index,
            npc_type,
            npc_name: npc.name.clone(),
            phase: Phase::Dispatch,
        })
    }

    fn poll(&mut self, cx: &mut ActionContext<'_>) -> Poll<Result<bool, ActionError>> {
        let snapshot = cx.snapshot();
        let Some(frame) = Frame::borrow(snapshot) else {
            return Poll::Pending;
        };
        let npc = self.target(&frame);
        let spell = self.spell();
        match self.phase {
            Phase::Dispatch => {
                let Some(npc) = npc else {
                    return Poll::Ready(Ok(false));
                };
                if !magic::castable_npc(spell, &frame, &self.selected, self.npc_type) {
                    return Poll::Ready(Ok(false));
                }
                let rune_baseline = magic::rune_count_npc(spell, &frame, &self.selected);
                let chat_since = snapshot
                    .chat_lines(-1)
                    .and_then(|chat| chat.value.iter().map(|line| line.sequence).max())
                    .unwrap_or(-1);
                let request = InteractReq::UseWidgetOn {
                    component_id: self.component_id,
                    kind: "npc".into(),
                    target_name: npc.name.clone(),
                    x: npc.network.x,
                    z: npc.network.z,
                    level: npc.network.level,
                    index: Some(self.npc_index),
                    target_id: Some(self.npc_type),
                };
                match cx.emit(request) {
                    Ok(request_id) => {
                        self.phase = Phase::Await(AwaitingCast {
                            request_id,
                            sent_tick: cx.evidence().tick,
                            sent_sequence: cx.evidence().sequence,
                            chat_since,
                            rune_baseline,
                            spot_animation_stamp: npc.spot_animation_stamp,
                        });
                        Poll::Pending
                    }
                    Err(ActionError::BudgetExhausted) => Poll::Pending,
                    Err(_) => Poll::Ready(Ok(false)),
                }
            }
            Phase::Await(mut awaiting) => {
                let receipt = cx.interaction_receipt(awaiting.request_id);
                let Some(receipt) = receipt else {
                    return if Self::timed_out(awaiting.sent_tick, cx) {
                        Poll::Ready(Ok(false))
                    } else {
                        Poll::Pending
                    };
                };
                if !receipt.accepted {
                    return Poll::Ready(Ok(false));
                }
                if receipt.evidence.sequence < awaiting.sent_sequence
                    || cx.evidence().sequence <= receipt.evidence.sequence
                {
                    return Poll::Pending;
                }
                awaiting.chat_since = awaiting.chat_since.max(receipt.chat_since);
                if let Some(chat) = snapshot.chat_lines(awaiting.chat_since) {
                    for line in chat.value.iter() {
                        if magic::refusal(&line.text, spell).is_some() {
                            return Poll::Ready(Ok(false));
                        }
                    }
                    if let Some(sequence) = chat.value.iter().map(|line| line.sequence).max() {
                        awaiting.chat_since = sequence;
                    }
                }
                if let Some(npc) = npc {
                    // `Game.castOnNpc` reports that the cast was sent
                    // (`Game.ts:177-185` returns `Input.castOnNpc`). A splash
                    // (the shared failed-spell impact) is a cast that missed:
                    // the runes are spent, so it is `true` like a hit.
                    let new_spot_animation =
                        npc.spot_animation_stamp != awaiting.spot_animation_stamp;
                    let landed = (spell.impact_spotanim >= 0
                        && npc.spot_animation == spell.impact_spotanim)
                        || self
                            .selected
                            .failed_spell_impact()
                            .is_some_and(|failed| npc.spot_animation == failed);
                    if new_spot_animation && landed {
                        return Poll::Ready(Ok(true));
                    }
                }
                if awaiting.rune_baseline.is_some_and(|(before, required)| {
                    magic::rune_count_npc(spell, &frame, &self.selected)
                        .is_some_and(|(after, _)| before.saturating_sub(after) >= required)
                }) {
                    return Poll::Ready(Ok(true));
                }
                if Self::timed_out(awaiting.sent_tick, cx) {
                    return Poll::Ready(Ok(false));
                }
                self.phase = Phase::Await(awaiting);
                Poll::Pending
            }
        }
    }
}

fn unavailable(reason: &'static str) -> ActionError {
    ActionError::Unavailable(reason.into())
}
