//! Native chat-dialogue driver extracted from the isolate `dialog` family.
//! Constants and sequencing match `crates/script/src/dialog.rs`.

use super::reach;
use crate::native::{ActionContext, ActionError, ActionHandle, NativeActions, NativeMachine};
use crate::shim::InteractReq;
use std::sync::Arc;
use std::task::Poll;

pub const DIALOG_GAP_MS: u64 = 1_500;
pub const DIALOGUE_OPEN_MS: u64 = 8_000;
pub const DIALOGUE_APPROACH_MS: u64 = 20_000;
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
    Approach,
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
    walk_request_id: u64,
}

impl NativeMachine for Dialogue {
    type Args = DialogueArgs;
    type Output = bool;

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
            walk_request_id: 0,
        };
        dialogue.open(cx)?;
        Ok(dialogue)
    }

    fn poll(&mut self, cx: &mut ActionContext<'_>) -> Poll<Result<Self::Output, ActionError>> {
        let now = cx.active_now().as_millis() as u64;
        let Some(obs) = observe(cx) else {
            return Poll::Pending;
        };
        match self.phase {
            Phase::Approach => {
                if let Some(target) = nearest_talk(cx, self.args.id) {
                    if target.distance <= 2 {
                        self.npc_index = target.index;
                        self.npc_action = Arc::from(target.action);
                        cx.cancel_request(self.walk_request_id);
                        self.walk_request_id = 0;
                        return match self.open(cx) {
                            Ok(()) => Poll::Pending,
                            Err(error) => Poll::Ready(Err(error)),
                        };
                    }
                }
                if now >= self.deadline_ms
                    || (self.walk_request_id != 0
                        && cx.walk_receipt(self.walk_request_id).is_some())
                {
                    cx.cancel_request(self.walk_request_id);
                    self.walk_request_id = 0;
                    return Poll::Ready(Ok(false));
                }
                Poll::Pending
            }
            Phase::Open => {
                if obs.ready {
                    self.phase = Phase::Drive;
                    return self.drive(cx, &obs);
                }
                if now >= self.deadline_ms {
                    return Poll::Ready(Ok(false));
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
                    return Poll::Ready(Ok(false));
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
                    return Poll::Ready(Ok(false));
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
                } else if now >= self.deadline_ms {
                    Poll::Ready(Ok(!obs.open))
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
            if let Some(target) = nearest_talk(cx, self.args.id) {
                if target.distance > 2 {
                    self.walk_request_id =
                        cx.walk(reach::walk_request(target.tile, 1, cx.evidence()))?;
                    self.phase = Phase::Approach;
                    self.deadline_ms = cx.active_now().as_millis() as u64 + DIALOGUE_APPROACH_MS;
                    return Ok(());
                }
                self.npc_index = target.index;
                self.npc_action = Arc::from(target.action);
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
    ) -> Poll<Result<bool, ActionError>> {
        if self.steps >= DRIVE_STEPS {
            return Poll::Ready(Ok(!obs.open));
        }
        if !obs.ready {
            self.phase = Phase::WaitGap;
            self.deadline_ms = cx.active_now().as_millis() as u64 + DIALOG_GAP_MS;
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
    })
}

struct TalkTarget<'a> {
    index: i32,
    action: &'a str,
    tile: api::WorldTile,
    distance: i32,
}

fn nearest_talk<'a>(cx: &'a ActionContext<'_>, wanted: i32) -> Option<TalkTarget<'a>> {
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
            Some(TalkTarget {
                index: npc.index as i32,
                action,
                tile: npc.tile,
                distance: npc.distance,
            })
        })
        .min_by_key(|target| target.distance)
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
) -> Poll<Result<bool, ActionError>> {
    actions.poll(handle, cx)
}
