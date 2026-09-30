//! Native chat-dialogue driver extracted from the isolate `dialog` family.
//! Page sequencing matches the isolate driver; completion waits on game ticks.

use crate::dialogue_outcome::DialogueOutcome;
use crate::native::{ActionContext, ActionError, ActionHandle, NativeActions, NativeMachine};
use crate::shim::InteractReq;
use std::sync::Arc;
use std::task::Poll;

pub const DIALOG_GAP_TICKS: u64 = 4;
pub const DIALOGUE_OPEN_MS: u64 = 8_000;
pub const DRIVE_STEPS: u32 = 120;
pub const PAGE_ACK_MS: u64 = 3_000;
pub const CONTINUE_TICKS: u64 = 1;
pub const CHOICE_TICKS: u64 = 2;

#[derive(Clone)]
pub struct DialogueArgs {
    pub id: i32,
    pub npc: Arc<str>,
    pub prefer: Arc<[Arc<str>]>,
    pub choose: Option<i32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Open,
    Drive,
    WaitContinueAck,
    WaitContinueTick,
    WaitChoiceAck,
    WaitChoiceTicks,
    WaitGap,
}

pub struct Dialogue {
    args: DialogueArgs,
    phase: Phase,
    steps: u32,
    due_tick: u64,
    ack_modal: i32,
    npc_index: i32,
    npc_action: Arc<str>,
    deadline_ms: u64,
}

impl NativeMachine for Dialogue {
    type Args = DialogueArgs;
    type Output = DialogueOutcome;

    fn begin(args: Self::Args, cx: &mut ActionContext<'_>) -> Result<Self, ActionError> {
        let mut dialogue = Self {
            args,
            phase: Phase::Open,
            steps: 0,
            due_tick: 0,
            ack_modal: -1,
            npc_index: -1,
            npc_action: Arc::from("Talk-to"),
            deadline_ms: cx.active_now().as_millis() as u64 + DIALOGUE_OPEN_MS,
        };
        dialogue.open(cx)?;
        Ok(dialogue)
    }

    fn poll(&mut self, cx: &mut ActionContext<'_>) -> Poll<Result<Self::Output, ActionError>> {
        let now = cx.active_now().as_millis() as u64;
        let Some(obs) = observe(cx) else {
            return Poll::Pending;
        };
        // NPC melee closes interfaces even when it deals zero damage. A
        // witnessed combat close is not the quiet end of a conversation.
        if let Some(outcome) = DialogueOutcome::combat_interruption(obs.open, obs.in_combat) {
            return Poll::Ready(Ok(outcome));
        }
        match self.phase {
            Phase::Open => {
                if obs.ready {
                    self.phase = Phase::Drive;
                    return self.drive(cx, &obs);
                }
                if now >= self.deadline_ms {
                    return Poll::Ready(Ok(DialogueOutcome::Failed));
                }
                Poll::Pending
            }
            Phase::WaitContinueAck => {
                if obs.modal != self.ack_modal || !obs.r#continue {
                    self.phase = Phase::WaitContinueTick;
                    self.due_tick = obs.tick.saturating_add(CONTINUE_TICKS);
                    return Poll::Pending;
                }
                if now >= self.deadline_ms {
                    return Poll::Ready(Ok(DialogueOutcome::Failed));
                }
                Poll::Pending
            }
            Phase::WaitChoiceAck => {
                if obs.modal != self.ack_modal || obs.r#continue {
                    self.phase = Phase::WaitChoiceTicks;
                    self.due_tick = obs.tick.saturating_add(CHOICE_TICKS);
                    return Poll::Pending;
                }
                if now >= self.deadline_ms {
                    return Poll::Ready(Ok(DialogueOutcome::Failed));
                }
                Poll::Pending
            }
            Phase::WaitContinueTick | Phase::WaitChoiceTicks => {
                if obs.tick >= self.due_tick {
                    self.phase = Phase::Drive;
                    self.drive(cx, &obs)
                } else {
                    Poll::Pending
                }
            }
            Phase::WaitGap => {
                if obs.ready {
                    self.phase = Phase::Drive;
                    self.drive(cx, &obs)
                } else if obs.tick >= self.due_tick {
                    Poll::Ready(Ok(if obs.open {
                        DialogueOutcome::Failed
                    } else {
                        DialogueOutcome::Completed
                    }))
                } else {
                    Poll::Pending
                }
            }
            Phase::Drive => self.drive(cx, &obs),
        }
    }

    fn cancel(&mut self) {}
}

impl Dialogue {
    fn open(&mut self, cx: &mut ActionContext<'_>) -> Result<(), ActionError> {
        if let Some(obs) = observe(cx) {
            if obs.ready {
                self.phase = Phase::Drive;
                return Ok(());
            }
            if let Some((index, action)) = nearest_talk(cx, self.args.id) {
                self.npc_index = index;
                self.npc_action = action;
            }
        }
        cx.emit(InteractReq::Npc {
            name: self.args.npc.to_string(),
            action: self.npc_action.to_string(),
            index: (self.npc_index >= 0).then_some(self.npc_index),
        })?;
        self.phase = Phase::Open;
        Ok(())
    }

    fn drive(
        &mut self,
        cx: &mut ActionContext<'_>,
        obs: &ChatObs,
    ) -> Poll<Result<DialogueOutcome, ActionError>> {
        if self.steps >= DRIVE_STEPS {
            return Poll::Ready(Ok(if obs.open {
                DialogueOutcome::Failed
            } else {
                DialogueOutcome::Completed
            }));
        }
        if !obs.ready {
            self.phase = Phase::WaitGap;
            self.due_tick = obs.tick.saturating_add(DIALOG_GAP_TICKS);
            return Poll::Pending;
        }
        if obs.r#continue {
            self.steps += 1;
            self.ack_modal = obs.modal;
            self.phase = Phase::WaitContinueAck;
            self.deadline_ms = cx.active_now().as_millis() as u64 + PAGE_ACK_MS;
            return match cx.emit(InteractReq::ContinueDialog) {
                Ok(_) => Poll::Pending,
                Err(error) => Poll::Ready(Err(error)),
            };
        }
        if !obs.options.is_empty() {
            let option = self.args.choose.unwrap_or_else(|| {
                pick_preferred(obs.options, &self.args.prefer)
                    .map(|index| (index + 1) as i32)
                    .unwrap_or(obs.options.len() as i32)
            });
            self.steps += 1;
            self.ack_modal = obs.modal;
            self.phase = Phase::WaitChoiceAck;
            self.deadline_ms = cx.active_now().as_millis() as u64 + PAGE_ACK_MS;
            return match cx.emit(InteractReq::Answer { option }) {
                Ok(_) => Poll::Pending,
                Err(error) => Poll::Ready(Err(error)),
            };
        }
        self.steps += 1;
        self.phase = Phase::WaitContinueTick;
        self.due_tick = obs.tick.saturating_add(CONTINUE_TICKS);
        Poll::Pending
    }
}

struct ChatObs<'a> {
    open: bool,
    ready: bool,
    r#continue: bool,
    modal: i32,
    tick: u64,
    options: &'a [api::snapshot::ChatOptionView],
    in_combat: bool,
}

fn observe<'a>(cx: &ActionContext<'a>) -> Option<ChatObs<'a>> {
    let chat = cx.snapshot.chat_modal()?;
    let modal = chat.value.root;
    let r#continue = chat.value.continue_component_id >= 0;
    let open = modal != -1;
    Some(ChatObs {
        open,
        ready: open || r#continue,
        r#continue,
        modal,
        tick: cx.evidence().tick,
        options: chat.value.options,
        in_combat: cx
            .snapshot()
            .in_combat()
            .is_some_and(|combat| combat.value.in_combat),
    })
}

fn nearest_talk(cx: &ActionContext<'_>, wanted: i32) -> Option<(i32, Arc<str>)> {
    let npcs = cx.snapshot().npcs()?.value;
    npcs.iter()
        .filter_map(|npc| {
            if npc.r#type != Some(wanted as usize) {
                return None;
            }
            let action = npc
                .actions
                .iter()
                .flatten()
                .find(|action| action.len() >= 4 && action[..4].eq_ignore_ascii_case("talk"))?;
            Some((
                npc.distance,
                npc.index as i32,
                Arc::<str>::from(action.as_str()),
            ))
        })
        .min_by_key(|(distance, _, _)| *distance)
        .map(|(_, index, action)| (index, action))
}

pub fn pick_preferred(
    options: &[api::snapshot::ChatOptionView],
    prefer: &[Arc<str>],
) -> Option<usize> {
    prefer.iter().find_map(|fragment| {
        options.iter().position(|option| {
            !option.text.is_empty()
                && (fragment.is_empty()
                    || option
                        .text
                        .as_bytes()
                        .windows(fragment.len())
                        .any(|part| part.eq_ignore_ascii_case(fragment.as_bytes())))
        })
    })
}

pub fn poll_dialogue(
    actions: &mut NativeActions,
    handle: &ActionHandle<Dialogue>,
    cx: &mut ActionContext<'_>,
) -> Poll<Result<DialogueOutcome, ActionError>> {
    actions.poll(handle, cx)
}
