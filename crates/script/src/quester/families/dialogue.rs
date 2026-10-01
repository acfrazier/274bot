//! Native chat-dialogue driver extracted from the isolate `dialog` family.
//! Acknowledgement follows page content as well as roots; completion waits on game ticks.

use super::reach;
use crate::dialogue_outcome::DialogueOutcome;
use crate::native::{ActionContext, ActionError, ActionHandle, NativeActions, NativeMachine};
use crate::shim::InteractReq;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::Arc;
use std::task::Poll;

pub const DIALOG_GAP_TICKS: u64 = 4;
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
    gap_inventory: Option<u64>,
    gap_rearms_left: u64,
    ack_modal: i32,
    ack_page: u64,
    npc_index: i32,
    npc_action: Arc<str>,
    deadline_ms: u64,
    walk_request_id: u64,
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
            gap_inventory: None,
            // Each held unit can be transferred once. Allow four additional
            // observed changes for batched/reward updates, but never let
            // unrelated inventory activity keep a closed dialogue alive.
            gap_rearms_left: cx
                .snapshot()
                .inventory()
                .map_or(0, |inventory| {
                    inventory.value.iter().fold(0u64, |count, item| {
                        count.saturating_add(item.count.max(0) as u64)
                    })
                })
                .saturating_add(DIALOG_GAP_TICKS)
                .min(DRIVE_STEPS as u64),
            ack_modal: -1,
            ack_page: 0,
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
        // NPC melee closes interfaces even when it deals zero damage. A
        // witnessed combat close is not the quiet end of a conversation.
        if let Some(outcome) = DialogueOutcome::combat_interruption(obs.open, obs.in_combat) {
            return Poll::Ready(Ok(outcome));
        }
        match self.phase {
            Phase::Approach => {
                if let Some(target) = nearest_talk(cx, self.args.id) {
                    if target.distance <= 2 && talk_reachable(cx, target.tile) {
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
                    return Poll::Ready(Ok(DialogueOutcome::Failed));
                }
                Poll::Pending
            }
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
                if obs.modal != self.ack_modal
                    || !obs.r#continue
                    || obs.page_fingerprint() != self.ack_page
                {
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
                if obs.modal != self.ack_modal
                    || obs.r#continue
                    || obs.page_fingerprint() != self.ack_page
                {
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
                } else {
                    let inventory = inventory_fingerprint(cx);
                    if inventory != self.gap_inventory && self.gap_rearms_left > 0 {
                        // Bulk hand-ins close the chat while the server consumes
                        // items, then reopen it for the final page. These are
                        // active game ticks, not the conversation's quiet end.
                        self.gap_inventory = inventory;
                        self.gap_rearms_left -= 1;
                        self.due_tick = obs.tick.saturating_add(DIALOG_GAP_TICKS);
                    }
                    if obs.tick >= self.due_tick {
                        Poll::Ready(Ok(if obs.open {
                            DialogueOutcome::Failed
                        } else {
                            DialogueOutcome::Completed
                        }))
                    } else {
                        Poll::Pending
                    }
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
                let reachable = talk_reachable(cx, target.tile);
                if target.distance > 2 || !reachable {
                    // A nearby NPC across a closed wall/door is not ready to
                    // talk. Route to its side instead of accepting a blocked
                    // adjacent tile and cancelling before the door traversal.
                    self.walk_request_id = cx.walk(reach::walk_request(
                        target.tile,
                        if reachable { 1 } else { 0 },
                        cx.evidence(),
                    ))?;
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
        self.deadline_ms = cx.active_now().as_millis() as u64 + DIALOGUE_OPEN_MS;
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
            self.gap_inventory = inventory_fingerprint(cx);
            self.due_tick = obs.tick.saturating_add(DIALOG_GAP_TICKS);
            return Poll::Pending;
        }
        if obs.r#continue {
            self.steps += 1;
            self.ack_modal = obs.modal;
            self.ack_page = obs.page_fingerprint();
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
            self.ack_page = obs.page_fingerprint();
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

fn inventory_fingerprint(cx: &ActionContext<'_>) -> Option<u64> {
    cx.snapshot().inventory().map(|inventory| {
        let mut hash = DefaultHasher::new();
        for item in inventory.value {
            item.def.id.hash(&mut hash);
            item.slot.hash(&mut hash);
            item.count.hash(&mut hash);
        }
        hash.finish()
    })
}

struct ChatObs<'a> {
    open: bool,
    ready: bool,
    r#continue: bool,
    modal: i32,
    tick: u64,
    options: &'a [api::snapshot::ChatOptionView],
    texts: &'a [String],
    in_combat: bool,
}

impl ChatObs<'_> {
    // The server reuses a chat root and Continue component across successive
    // pages. A newer snapshot/tick alone is not an acknowledgement; the page
    // must change. Fingerprinting borrowed content avoids copying each page.
    fn page_fingerprint(&self) -> u64 {
        let mut hash = DefaultHasher::new();
        self.texts.hash(&mut hash);
        for option in self.options {
            option.component_id.hash(&mut hash);
            option.text.hash(&mut hash);
        }
        hash.finish()
    }
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
        texts: chat.value.texts,
        in_combat: cx
            .snapshot()
            .in_combat()
            .is_some_and(|combat| combat.value.in_combat),
    })
}

struct TalkTarget<'a> {
    index: i32,
    action: &'a str,
    tile: api::WorldTile,
    distance: i32,
}

fn talk_reachable(cx: &ActionContext<'_>, tile: api::WorldTile) -> bool {
    cx.snapshot().reach().is_none_or(|reach| {
        reach.value.can_reach(
            tile,
            &api::query::SceneReachOptions {
                max_steps: None,
                adjacent_ok: true,
            },
        )
    })
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
) -> Poll<Result<DialogueOutcome, ActionError>> {
    actions.poll(handle, cx)
}
