//! Native chat and selected scroll/book continuation driver.
//! Acknowledgement follows page content as well as roots; completion waits on game ticks.

use super::reach;
use crate::dialogue_outcome::DialogueOutcome;
use crate::native::{ActionContext, ActionError, ActionHandle, NativeActions, NativeMachine};
use crate::shim::InteractReq;
use api::snapshot::{chat_page_fingerprint, WidgetRoot, WidgetView};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::Arc;
use std::task::Poll;

/// Quiet ticks after the chat is observed closed before a conversation counts
/// as over, unless the step authors `gap_ticks`. The engine closes chat with
/// the same `IfClose` at script end (`Player.ts:2223-2229`) and at a script's
/// own `if_close` before a `p_delay` and a later page, so the client cannot
/// tell them apart; script end is the normal case, and a Path step whose NPC
/// closes and reopens the chat authors its engine hole (`gap_ticks`).
pub const DIALOG_GAP_TICKS: u64 = 1;
pub const DIALOGUE_OPEN_MS: u64 = 8_000;
pub const DIALOGUE_APPROACH_MS: u64 = 20_000;
pub const DRIVE_STEPS: u32 = 120;
pub const PAGE_ACK_MS: u64 = 3_000;
/// Ticks before acting on a page that changed text on the same root (a same-
/// root choice page, a book or scroll forward) or offered no input: its text
/// can still be arriving in later frames of the server tick under the
/// client's 5-packets-per-frame read. A Continue or a new root needs none.
pub const PAGE_SETTLE_TICKS: u64 = 1;

/// The native dialogue acknowledgement shared with the v1 adapter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PageAcknowledgement {
    modal: i32,
    continue_visible: bool,
    fingerprint: u64,
}

impl PageAcknowledgement {
    pub(crate) fn capture(modal: i32, continue_visible: bool, fingerprint: u64) -> Self {
        Self {
            modal,
            continue_visible,
            fingerprint,
        }
    }

    pub(crate) fn acknowledged(self, modal: i32, continue_visible: bool, fingerprint: u64) -> bool {
        modal != self.modal
            || continue_visible != self.continue_visible
            || fingerprint != self.fingerprint
    }
}

impl Default for PageAcknowledgement {
    fn default() -> Self {
        Self::capture(-1, false, 0)
    }
}

#[derive(Clone)]
pub enum DialogueTarget {
    Npc {
        id: i32,
        name: Arc<str>,
    },
    /// Drive the chat opened by a previous host action. Never talk or walk.
    Continuation,
}

#[derive(Clone)]
pub struct LineRule {
    pub when_line: Arc<str>,
    pub choose: Arc<str>,
}

#[derive(Clone)]
pub struct DialogueOptions {
    pub prefer: Arc<[Arc<str>]>,
    pub choose: Option<i32>,
    pub line_rules: Arc<[LineRule]>,
    pub strict: bool,
    /// Chat continue clicks only: a Main document or modal ends the driver
    /// without a click, so it is left to its owner.
    pub chat_only: bool,
    /// Authored end gap in quiet ticks after the chat closes; `None` uses
    /// [`DIALOG_GAP_TICKS`].
    pub gap_ticks: Option<u16>,
}

impl Default for DialogueOptions {
    fn default() -> Self {
        static PREFER: std::sync::LazyLock<Arc<[Arc<str>]>> =
            std::sync::LazyLock::new(|| Arc::from([]));
        static RULES: std::sync::LazyLock<Arc<[LineRule]>> =
            std::sync::LazyLock::new(|| Arc::from([]));
        Self {
            prefer: Arc::clone(&PREFER),
            choose: None,
            line_rules: Arc::clone(&RULES),
            strict: false,
            chat_only: false,
            gap_ticks: None,
        }
    }
}

impl DialogueOptions {
    pub(crate) fn continue_only() -> Self {
        Self {
            strict: true,
            ..Self::default()
        }
    }

    /// Continue clicks on Chat pages only: a menu fails strict, and a Main
    /// scroll, book or modal ends the driver untouched. For recovery callers
    /// that own no conversation (the journal read's drain).
    pub(crate) fn chat_continue_only() -> Self {
        Self {
            strict: true,
            chat_only: true,
            ..Self::default()
        }
    }

    /// These authored answers narrowed to what a page's text proves: strict
    /// line rules and preferences, never the fixed `choose` index or the
    /// last-option fallback, and Chat clicks only (a Main document or modal
    /// ends the driver untouched). For a page this driver did not open.
    pub(crate) fn by_text_only(&self) -> Self {
        Self {
            choose: None,
            strict: true,
            chat_only: true,
            ..self.clone()
        }
    }

    fn select(&self, obs: &ChatObs<'_>) -> Option<i32> {
        if let Some(rule) = self.line_rules.iter().find(|rule| {
            obs.texts
                .iter()
                .any(|text| text_matches(text, &rule.when_line))
        }) {
            match matching_option(obs.options, &rule.choose, self.strict) {
                Ok(Some(index)) => return Some((index + 1) as i32),
                Err(()) => return None,
                Ok(None) if self.strict => return None,
                Ok(None) => {}
            }
        }
        if let Some(option) = self.choose {
            return (option > 0 && option as usize <= obs.options.len()).then_some(option);
        }
        if self.strict {
            for fragment in self.prefer.iter() {
                match matching_option(obs.options, fragment, true) {
                    Ok(Some(index)) => return Some((index + 1) as i32),
                    Err(()) => return None,
                    Ok(None) => {}
                }
            }
            None
        } else {
            Some(
                pick_preferred(obs.options, &self.prefer)
                    .map_or(obs.options.len() as i32, |index| (index + 1) as i32),
            )
        }
    }

    /// The one option these answers pick on a menu from its text alone, for
    /// a menu this driver did not open: every line rule whose line is on the
    /// page and every preference that matches an option must match exactly
    /// one option, and the same one. The fixed `choose` index and the
    /// last-option fallback never count.
    fn unique_text_answer(
        &self,
        texts: &[String],
        options: &[api::snapshot::ChatOptionView],
    ) -> Option<i32> {
        let rules = self
            .line_rules
            .iter()
            .filter(|rule| texts.iter().any(|text| text_matches(text, &rule.when_line)))
            .map(|rule| &rule.choose);
        let mut picked = None;
        for fragment in rules.chain(self.prefer.iter()) {
            match matching_option(options, fragment, true) {
                Ok(None) => {}
                Ok(Some(index)) if picked.is_none_or(|picked| picked == index) => {
                    picked = Some(index);
                }
                Ok(Some(_)) | Err(()) => return None,
            }
        }
        picked.map(|index| (index + 1) as i32)
    }
}

#[derive(Clone)]
pub struct DialogueArgs {
    pub target: DialogueTarget,
    pub options: DialogueOptions,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Surface {
    Unselected,
    Chat,
    Main,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ChatTransitionAck {
    Accepted,
    Pending,
    Failed,
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
    WaitMainForwardAck,
    WaitMainForwardTick,
    WaitMainCloseAck,
    WaitMainCloseTick,
    WaitGap,
    Failed,
}

pub struct Dialogue {
    args: DialogueArgs,
    phase: Phase,
    surface: Surface,
    main_ui: Option<api::game_data::DialogueUiIds>,
    steps: u32,
    due_tick: u64,
    gap_inventory: Option<u64>,
    gap_position: Option<api::WorldTile>,
    gap_chat_mark: i32,
    gap_rearms_left: u64,
    gap_rearm_tick: Option<u64>,
    ack_page: PageAcknowledgement,
    npc_index: i32,
    npc_action: Arc<str>,
    deadline_ms: u64,
    walk_request_id: u64,
    talk_request_id: Option<u64>,
    talk_baseline: Option<PageAcknowledgement>,
    chat_advanced: bool,
    chat_page_owned: bool,
    chat_advance_request_id: Option<u64>,
}

impl NativeMachine for Dialogue {
    type Args = DialogueArgs;
    type Output = DialogueOutcome;

    fn begin(args: Self::Args, cx: &mut ActionContext<'_>) -> Result<Self, ActionError> {
        let mut dialogue = Self {
            args,
            phase: Phase::Open,
            main_ui: selected_dialogue_ui(cx),
            steps: 0,
            due_tick: 0,
            gap_inventory: None,
            gap_position: None,
            gap_chat_mark: reach::last_chat_seq(cx),
            // Each held unit can be transferred once. Allow four additional
            // observed changes for batched rewards or server-driven movement,
            // but never let unrelated activity keep closed dialogue alive.
            gap_rearms_left: cx
                .snapshot()
                .inventory()
                .map_or(0, |inventory| {
                    inventory.value.iter().fold(0u64, |count, item| {
                        count.saturating_add(item.count.max(0) as u64)
                    })
                })
                .saturating_add(4)
                .min(DRIVE_STEPS as u64),
            gap_rearm_tick: None,
            ack_page: PageAcknowledgement::default(),
            npc_index: -1,
            npc_action: Arc::from("Talk-to"),
            deadline_ms: cx.active_now().as_millis() as u64 + DIALOGUE_OPEN_MS,
            walk_request_id: 0,
            surface: Surface::Unselected,
            talk_request_id: None,
            talk_baseline: None,
            chat_advanced: false,
            chat_page_owned: false,
            chat_advance_request_id: None,
        };
        dialogue.open(cx)?;
        Ok(dialogue)
    }

    fn poll(&mut self, cx: &mut ActionContext<'_>) -> Poll<Result<Self::Output, ActionError>> {
        let now = cx.active_now().as_millis() as u64;
        if self.phase == Phase::Failed {
            return Poll::Ready(Ok(DialogueOutcome::Failed));
        }
        let Some(obs) = observe(cx) else {
            return Poll::Pending;
        };
        let Some(main) = observe_main(cx, self.main_ui) else {
            return Poll::Pending;
        };
        if !self.chat_page_owned && self.owned_chat_page(cx).is_some() {
            self.chat_page_owned = true;
        }
        let chat_active = obs.ready;
        let main_active = main.open();
        match self.surface {
            Surface::Main => {
                if chat_active {
                    // A level-up page can open on the chat surface after our
                    // own Main close (Cook's hand-in: quest scroll, our
                    // `CloseModal`, then "Congratulations, you just advanced
                    // a Cooking level"). Once the owned modal is observed
                    // closed, adopt the page as a continuation and drain it
                    // with the normal chat driver instead of failing. A chat
                    // page while Main is still open stays `Failed`.
                    let own_close = matches!(
                        self.phase,
                        Phase::WaitMainCloseAck | Phase::WaitMainCloseTick
                    );
                    if own_close && main.root == -1 {
                        let phase_before_drive = self.phase;
                        let result = self.drive(cx, &obs);
                        if self.phase != phase_before_drive {
                            self.surface = Surface::Chat;
                        }
                        return result;
                    } else {
                        return Poll::Ready(Ok(DialogueOutcome::Failed));
                    }
                } else {
                    return self.poll_main(cx, &main, now);
                }
            }
            Surface::Chat if main_active => {
                if !chat_active {
                    // A chat-only caller never adopts what the page opened.
                    if self.args.options.chat_only {
                        return Poll::Ready(Ok(DialogueOutcome::Completed));
                    }
                    // A selected document continues on the Main surface.
                    // Any other Main modal can only be new here (one open at
                    // begin fails), so once our own accepted advance opened
                    // it (`set_sail`'s `if_openmain(ship_journey)` after the
                    // boat or customs answer), the conversation is over: the
                    // step's settle judges what the modal did.
                    let document = matches!(main.kind, MainKind::Scroll | MainKind::Book);
                    if document || self.chat_advance_request_id.is_some() {
                        match self.chat_transition_acknowledgement(cx) {
                            ChatTransitionAck::Accepted if document => {
                                let previous_phase = self.phase;
                                self.phase = Phase::Open;
                                let result = self.poll_main(cx, &main, now);
                                if self.phase == Phase::Open && result.is_pending() {
                                    self.phase = previous_phase;
                                } else {
                                    self.surface = Surface::Main;
                                }
                                return result;
                            }
                            ChatTransitionAck::Accepted => {
                                return Poll::Ready(Ok(DialogueOutcome::Completed));
                            }
                            ChatTransitionAck::Pending if now < self.deadline_ms => {
                                return Poll::Pending;
                            }
                            ChatTransitionAck::Pending | ChatTransitionAck::Failed => {}
                        }
                    }
                }
                return Poll::Ready(Ok(DialogueOutcome::Failed));
            }
            Surface::Chat => {}
            Surface::Unselected => {
                if chat_active && main_active {
                    return Poll::Ready(Ok(DialogueOutcome::Failed));
                }
                if main_active {
                    if matches!(&self.args.target, DialogueTarget::Continuation)
                        && !self.args.options.chat_only
                    {
                        self.surface = Surface::Main;
                        return self.poll_main(cx, &main, now);
                    }
                    return Poll::Ready(Ok(DialogueOutcome::Failed));
                }
                if chat_active {
                    self.surface = Surface::Chat;
                }
            }
        }
        // NPC melee closes interfaces even when it deals zero damage. A
        // witnessed combat close is not the quiet end of a conversation.
        // A visible Continue component is an active page even without a root.
        if let Some(outcome) = DialogueOutcome::combat_interruption(obs.ready, obs.in_combat) {
            return Poll::Ready(Ok(outcome));
        }
        match self.phase {
            Phase::Approach => {
                if let Some(target) = self.npc_id().and_then(|id| nearest_talk(cx, id)) {
                    if target.distance <= 2 && talk_reachable(cx, target.tile) {
                        std::task::ready!(crate::native::defer_budget(self.open(cx)))?;
                        cx.cancel_request(self.walk_request_id);
                        self.walk_request_id = 0;
                        return Poll::Pending;
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
                    return self.drive(cx, &obs);
                }
                if now >= self.deadline_ms {
                    static REASON: std::sync::LazyLock<Arc<str>> =
                        std::sync::LazyLock::new(|| Arc::from("expected dialogue did not open"));
                    return Poll::Ready(Err(ActionError::Failed(Arc::clone(&REASON))));
                }
                Poll::Pending
            }
            Phase::WaitContinueAck => {
                if self
                    .ack_page
                    .acknowledged(obs.modal, obs.r#continue, obs.page_fingerprint())
                {
                    // The engine resumes the paused script in the ack packet's
                    // own cycle (`ResumePauseButtonHandler.ts:7-13`) and writes
                    // the next `~chatnpc`/`~chatplayer` page in that execution
                    // (`chat.rs2:271-321`): no tick is owed before driving it.
                    // A Continue does not read the page's text, so a page still
                    // mid-update under the client's 5-packets-per-frame read
                    // is safe to press. A closed chat starts the end gap now.
                    self.phase = Phase::Drive;
                    return self.drive(cx, &obs);
                }
                if now >= self.deadline_ms {
                    return Poll::Ready(Ok(DialogueOutcome::Failed));
                }
                Poll::Pending
            }
            Phase::WaitChoiceAck => {
                if self
                    .ack_page
                    .acknowledged(obs.modal, obs.r#continue, obs.page_fingerprint())
                {
                    // The choice resumes the same paused script in the ack
                    // packet's cycle (`IfButtonHandler.ts:26-29`). A new chat
                    // root means the page's `if_openchat` has been applied, and
                    // every `p_choiceN`/`~chat*` proc writes its texts before it
                    // (`chat.rs2:1-14,271-321`); a closed chat starts the end
                    // gap: drive now. A same-root text change may still be
                    // missing option rows under the client's 5-packets-per-
                    // frame read, so it waits one tick before answering.
                    if !obs.ready || obs.modal != self.ack_page.modal {
                        self.phase = Phase::Drive;
                        return self.drive(cx, &obs);
                    }
                    self.phase = Phase::WaitChoiceTicks;
                    self.due_tick = obs.tick.saturating_add(PAGE_SETTLE_TICKS);
                    return Poll::Pending;
                }
                if now >= self.deadline_ms {
                    return Poll::Ready(Ok(DialogueOutcome::Failed));
                }
                Poll::Pending
            }
            Phase::WaitContinueTick | Phase::WaitChoiceTicks => {
                if obs.tick >= self.due_tick {
                    self.drive(cx, &obs)
                } else {
                    Poll::Pending
                }
            }
            Phase::WaitGap => {
                if obs.ready {
                    self.drive(cx, &obs)
                } else {
                    let inventory = inventory_fingerprint(cx);
                    let position = cx.snapshot().here().map(|here| here.value);
                    let moved = position
                        .zip(self.gap_position)
                        .is_some_and(|(here, before)| here != before);
                    // Scripted `mes` output (Oddenstein's hand-in `mes` /
                    // `p_delay` run) arrives as game chat messages: `type_ ==
                    // 0` with no username, the same filter as
                    // `reach::saw_game_message` / `reach::last_chat_seq`. A
                    // new one re-arms the gap on the same finite budget.
                    let chat_mark = reach::last_chat_seq(cx);
                    let chat_new =
                        cx.snapshot()
                            .chat_lines(self.gap_chat_mark)
                            .is_some_and(|lines| {
                                lines
                                    .value
                                    .iter()
                                    .any(|line| line.username.is_none() && line.type_ == 0)
                            });
                    let evidence_changed = inventory != self.gap_inventory || moved || chat_new;
                    if evidence_changed {
                        // Process every wake so later changes use the latest
                        // baseline, but charge only once per observed tick.
                        self.gap_inventory = inventory;
                        self.gap_position = position;
                        self.gap_chat_mark = chat_mark;
                        if self.gap_rearms_left > 0 && self.gap_rearm_tick != Some(obs.tick) {
                            self.gap_rearms_left -= 1;
                            self.gap_rearm_tick = Some(obs.tick);
                            self.due_tick = obs.tick.saturating_add(self.gap_ticks());
                        }
                    }
                    if obs.tick >= self.due_tick {
                        let animation_active = cx
                            .snapshot()
                            .local_player()
                            .is_some_and(|player| player.value.player.actor.animation >= 0);
                        if animation_active && self.gap_rearms_left > 0 {
                            if self.gap_rearm_tick == Some(obs.tick) {
                                return Poll::Pending;
                            }
                            // Scripted work can close chat before its final page.
                            // Share the evidence-tick limit with changed evidence.
                            self.gap_rearms_left -= 1;
                            self.gap_rearm_tick = Some(obs.tick);
                            self.due_tick = obs.tick.saturating_add(self.gap_ticks());
                            return Poll::Pending;
                        }
                        Poll::Ready(Ok(DialogueOutcome::Completed))
                    } else {
                        Poll::Pending
                    }
                }
            }
            Phase::Drive => self.drive(cx, &obs),
            Phase::WaitMainForwardAck
            | Phase::WaitMainForwardTick
            | Phase::WaitMainCloseAck
            | Phase::WaitMainCloseTick
            | Phase::Failed => Poll::Ready(Ok(DialogueOutcome::Failed)),
        }
    }

    fn cancel(&mut self) {}
}

impl Dialogue {
    /// Quiet ticks this conversation waits after the chat closes, and after
    /// each re-arm signal.
    fn gap_ticks(&self) -> u64 {
        self.args
            .options
            .gap_ticks
            .map_or(DIALOG_GAP_TICKS, u64::from)
    }

    fn npc_id(&self) -> Option<i32> {
        match &self.args.target {
            DialogueTarget::Npc { id, .. } => Some(*id),
            DialogueTarget::Continuation => None,
        }
    }

    /// The NPC index this driver's own Talk-to opened a conversation with,
    /// while it is mid-conversation on the chat surface: a page that Talk-to
    /// opened has been seen or answered, and the driver is driving it or
    /// waiting on its next page. `None` before the Talk-to's page, for an
    /// adopted page (no Talk-to), on a Main document, in the closing gap and
    /// after the end.
    pub(crate) fn conversation_npc(&self) -> Option<i32> {
        let driving = matches!(
            self.phase,
            Phase::Drive
                | Phase::WaitContinueAck
                | Phase::WaitContinueTick
                | Phase::WaitChoiceAck
                | Phase::WaitChoiceTicks
        );
        (self.npc_index >= 0
            && self.npc_id().is_some()
            && self.surface == Surface::Chat
            && driving
            && (self.chat_page_owned || self.chat_advanced))
            .then_some(self.npc_index)
    }
    fn chat_transition_acknowledgement(&self, cx: &ActionContext<'_>) -> ChatTransitionAck {
        let Some(request_id) = self.chat_advance_request_id else {
            return if self.chat_page_owned {
                ChatTransitionAck::Accepted
            } else {
                ChatTransitionAck::Failed
            };
        };
        let Some(receipt) = cx.interaction_receipt(request_id) else {
            return ChatTransitionAck::Pending;
        };
        let evidence = cx.evidence();
        if !receipt.accepted || receipt.evidence.run != evidence.run {
            ChatTransitionAck::Failed
        } else if evidence.tick <= receipt.evidence.tick {
            ChatTransitionAck::Pending
        } else {
            ChatTransitionAck::Accepted
        }
    }

    fn open(&mut self, cx: &mut ActionContext<'_>) -> Result<(), ActionError> {
        let chat = observe(cx);
        let main = observe_main(cx, self.main_ui);
        if let Some(obs) = chat.as_ref() {
            if obs.ready {
                // No Talk-to means this page belongs to an earlier action.
                // Adopt it only with authored answers, never the last-option fallback.
                if self.npc_id().is_some() {
                    self.args.options.strict = true;
                }
                if main.as_ref().is_some_and(MainObs::open) {
                    self.phase = Phase::Failed;
                } else {
                    self.surface = Surface::Chat;
                    self.phase = Phase::Drive;
                }
                return Ok(());
            }
        }
        if main.as_ref().is_some_and(MainObs::open) {
            if matches!(&self.args.target, DialogueTarget::Continuation) {
                self.surface = Surface::Main;
                self.phase = Phase::Drive;
            } else {
                self.phase = Phase::Failed;
            }
            return Ok(());
        }
        if let Some(target) = self.npc_id().and_then(|id| nearest_talk(cx, id)) {
            let reachable = talk_reachable(cx, target.tile);
            if target.distance > 2 || !reachable {
                // A nearby NPC across a closed wall/door is not ready to
                // talk. Route to its side instead of accepting a blocked
                // adjacent tile and cancelling before the door traversal.
                self.walk_request_id = cx.walk(reach::walk_request(
                    target.tile,
                    if reachable { 1 } else { 0 },
                    None,
                    cx.evidence(),
                ))?;
                self.phase = Phase::Approach;
                self.deadline_ms = cx.active_now().as_millis() as u64 + DIALOGUE_APPROACH_MS;
                return Ok(());
            }
            self.npc_index = target.index;
            self.npc_action = Arc::from(target.action);
        }
        let DialogueTarget::Npc { name, .. } = &self.args.target else {
            self.surface = Surface::Unselected;
            self.phase = Phase::Open;
            self.deadline_ms = cx.active_now().as_millis() as u64 + DIALOGUE_OPEN_MS;
            return Ok(());
        };
        let talk_baseline = chat.as_ref().map(|obs| {
            PageAcknowledgement::capture(obs.modal, obs.r#continue, obs.page_fingerprint())
        });
        let talk_request_id = cx.emit(InteractReq::Npc {
            name: name.to_string(),
            action: self.npc_action.to_string(),
            index: (self.npc_index >= 0).then_some(self.npc_index),
        })?;
        self.talk_baseline = talk_baseline;
        self.talk_request_id = Some(talk_request_id);
        self.surface = Surface::Chat;
        self.phase = Phase::Open;
        self.deadline_ms = cx.active_now().as_millis() as u64 + DIALOGUE_OPEN_MS;
        Ok(())
    }

    /// The first page opened by this dialogue's accepted NPC Talk request.
    /// This view is borrowed from the current snapshot and never exposes
    /// pre-existing pages or pages after this driver has advanced once.
    pub fn owned_chat_page<'a>(&self, cx: &ActionContext<'a>) -> Option<OwnedChatPage<'a>> {
        if self.chat_advanced || !matches!(&self.args.target, DialogueTarget::Npc { .. }) {
            return None;
        }
        let request_id = self.talk_request_id?;
        let baseline = self.talk_baseline?;
        let receipt = cx.interaction_receipt(request_id)?;
        let evidence = cx.evidence();
        if !receipt.accepted
            || receipt.evidence.run != evidence.run
            || evidence.tick <= receipt.evidence.tick
        {
            return None;
        }
        let obs = observe(cx)?;
        if !obs.open
            || !obs.ready
            || !baseline.acknowledged(obs.modal, obs.r#continue, obs.page_fingerprint())
        {
            return None;
        }
        Some(OwnedChatPage {
            root: obs.modal,
            continue_component_id: obs.continue_component_id,
            texts: obs.texts,
            options: obs.options,
            fingerprint: obs.page_fingerprint(),
            tick: obs.tick,
        })
    }

    fn poll_main(
        &mut self,
        cx: &mut ActionContext<'_>,
        obs: &MainObs<'_>,
        now: u64,
    ) -> Poll<Result<DialogueOutcome, ActionError>> {
        if let Some(outcome) = DialogueOutcome::combat_interruption(obs.open(), obs.in_combat) {
            return Poll::Ready(Ok(outcome));
        }
        match self.phase {
            Phase::Open => {
                if obs.open() {
                    self.drive_main(cx, obs, now)
                } else if now >= self.deadline_ms {
                    Poll::Ready(Ok(DialogueOutcome::Failed))
                } else {
                    Poll::Pending
                }
            }
            Phase::Drive => self.drive_main(cx, obs, now),
            Phase::WaitMainForwardAck => {
                if obs.root != self.ack_page.modal || obs.kind != MainKind::Book {
                    return Poll::Ready(Ok(DialogueOutcome::Failed));
                }
                let Some(forward_visible) = obs.book_forward_visible() else {
                    return Poll::Ready(Ok(DialogueOutcome::Failed));
                };
                if self
                    .ack_page
                    .acknowledged(obs.root, forward_visible, obs.page_fingerprint())
                {
                    self.phase = Phase::WaitMainForwardTick;
                    self.due_tick = obs.tick.saturating_add(PAGE_SETTLE_TICKS);
                    Poll::Pending
                } else if now >= self.deadline_ms {
                    Poll::Ready(Ok(DialogueOutcome::Failed))
                } else {
                    Poll::Pending
                }
            }
            Phase::WaitMainForwardTick => {
                if obs.root != self.ack_page.modal || obs.kind != MainKind::Book {
                    return Poll::Ready(Ok(DialogueOutcome::Failed));
                }
                if obs.tick >= self.due_tick {
                    self.drive_main(cx, obs, now)
                } else {
                    Poll::Pending
                }
            }
            Phase::WaitMainCloseAck => {
                if obs.root == -1
                    && self
                        .ack_page
                        .acknowledged(obs.root, false, obs.page_fingerprint())
                {
                    self.phase = Phase::WaitMainCloseTick;
                    self.due_tick = obs.tick.saturating_add(PAGE_SETTLE_TICKS);
                    Poll::Pending
                } else if obs.root != self.ack_page.modal || now >= self.deadline_ms {
                    Poll::Ready(Ok(DialogueOutcome::Failed))
                } else {
                    Poll::Pending
                }
            }
            Phase::WaitMainCloseTick => {
                if obs.root != -1 {
                    return Poll::Ready(Ok(DialogueOutcome::Failed));
                }
                if obs.tick >= self.due_tick {
                    Poll::Ready(Ok(DialogueOutcome::Completed))
                } else {
                    Poll::Pending
                }
            }
            Phase::Approach
            | Phase::WaitContinueAck
            | Phase::WaitContinueTick
            | Phase::WaitChoiceAck
            | Phase::WaitChoiceTicks
            | Phase::WaitGap
            | Phase::Failed => Poll::Ready(Ok(DialogueOutcome::Failed)),
        }
    }

    fn drive_main(
        &mut self,
        cx: &mut ActionContext<'_>,
        obs: &MainObs<'_>,
        now: u64,
    ) -> Poll<Result<DialogueOutcome, ActionError>> {
        if !obs.open() || self.steps >= DRIVE_STEPS {
            return Poll::Ready(Ok(DialogueOutcome::Failed));
        }
        match obs.kind {
            MainKind::Scroll => {
                let ack_page =
                    PageAcknowledgement::capture(obs.root, false, obs.page_fingerprint());
                std::task::ready!(crate::native::defer_budget(
                    cx.emit(InteractReq::CloseModal)
                ))?;
                self.steps += 1;
                self.ack_page = ack_page;
                self.phase = Phase::WaitMainCloseAck;
                self.deadline_ms = now.saturating_add(PAGE_ACK_MS);
                Poll::Pending
            }
            MainKind::Book => {
                let Some(forward_visible) = obs.book_forward_visible() else {
                    return Poll::Ready(Ok(DialogueOutcome::Failed));
                };
                let Some(ids) = obs.ids else {
                    return Poll::Ready(Ok(DialogueOutcome::Failed));
                };
                let (component_id, phase) = if forward_visible {
                    let Some(widget) = obs.server_button(ids.book_forward) else {
                        return Poll::Ready(Ok(DialogueOutcome::Failed));
                    };
                    (widget.component_id, Phase::WaitMainForwardAck)
                } else {
                    let Some(widget) = obs.server_button(ids.book_close) else {
                        return Poll::Ready(Ok(DialogueOutcome::Failed));
                    };
                    (widget.component_id, Phase::WaitMainCloseAck)
                };
                let ack_page =
                    PageAcknowledgement::capture(obs.root, forward_visible, obs.page_fingerprint());
                std::task::ready!(crate::native::defer_budget(
                    cx.emit(InteractReq::IfButton { component_id })
                ))?;
                self.steps += 1;
                self.ack_page = ack_page;
                self.phase = phase;
                self.deadline_ms = now.saturating_add(PAGE_ACK_MS);
                Poll::Pending
            }
            MainKind::Closed | MainKind::Unsupported => Poll::Ready(Ok(DialogueOutcome::Failed)),
        }
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
            self.gap_position = cx.snapshot().here().map(|here| here.value);
            self.gap_chat_mark = reach::last_chat_seq(cx);
            self.due_tick = obs.tick.saturating_add(self.gap_ticks());
            return Poll::Pending;
        }
        if obs.r#continue {
            let ack_page =
                PageAcknowledgement::capture(obs.modal, obs.r#continue, obs.page_fingerprint());
            let request_id = std::task::ready!(crate::native::defer_budget(
                cx.emit(InteractReq::ContinueDialog { component_id: None })
            ))?;
            self.steps += 1;
            self.ack_page = ack_page;
            self.phase = Phase::WaitContinueAck;
            self.deadline_ms = cx.active_now().as_millis() as u64 + PAGE_ACK_MS;
            self.chat_advanced = true;
            self.chat_advance_request_id = Some(request_id);
            return Poll::Pending;
        }
        if !obs.options.is_empty() {
            let Some(option) = self.args.options.select(obs) else {
                if self.args.options.strict
                    && self.args.options.prefer.is_empty()
                    && self.args.options.choose.is_none()
                    && self.args.options.line_rules.is_empty()
                {
                    static REASON: std::sync::LazyLock<Arc<str>> = std::sync::LazyLock::new(|| {
                        Arc::from("dialogue menu requires an explicit answer rule")
                    });
                    return Poll::Ready(Err(ActionError::Failed(Arc::clone(&REASON))));
                }
                return Poll::Ready(Ok(DialogueOutcome::Failed));
            };
            let ack_page =
                PageAcknowledgement::capture(obs.modal, obs.r#continue, obs.page_fingerprint());
            let request_id = std::task::ready!(crate::native::defer_budget(
                cx.emit(InteractReq::Answer { option })
            ))?;
            self.steps += 1;
            self.ack_page = ack_page;
            self.phase = Phase::WaitChoiceAck;
            self.deadline_ms = cx.active_now().as_millis() as u64 + PAGE_ACK_MS;
            self.chat_advanced = true;
            self.chat_advance_request_id = Some(request_id);
            return Poll::Pending;
        }
        self.steps += 1;
        self.phase = Phase::WaitContinueTick;
        self.due_tick = obs.tick.saturating_add(PAGE_SETTLE_TICKS);
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

#[derive(Debug, Clone, Copy)]
pub struct OwnedChatPage<'a> {
    pub root: i32,
    pub continue_component_id: i32,
    pub texts: &'a [String],
    pub options: &'a [api::snapshot::ChatOptionView],
    pub fingerprint: u64,
    pub tick: u64,
}

struct ChatObs<'a> {
    open: bool,
    ready: bool,
    r#continue: bool,
    continue_component_id: i32,
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
        chat_page_fingerprint(
            self.texts,
            self.options
                .iter()
                .map(|option| (option.component_id, option.text.as_str())),
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MainKind {
    Closed,
    Scroll,
    Book,
    Unsupported,
}

struct MainObs<'a> {
    root: i32,
    kind: MainKind,
    ids: Option<api::game_data::DialogueUiIds>,
    widgets: Option<&'a [WidgetView]>,
    texts: &'a [String],
    tick: u64,
    in_combat: bool,
}

impl MainObs<'_> {
    fn open(&self) -> bool {
        self.root != -1
    }

    fn component(&self, component_id: i32) -> Option<&WidgetView> {
        let mut matches = self.widgets?.iter().filter(|widget| {
            widget.root == WidgetRoot::Main
                && widget.root_component_id == self.root
                && widget.component_id == component_id
        });
        let component = matches.next()?;
        matches.next().is_none().then_some(component)
    }

    fn server_button(&self, component_id: i32) -> Option<&WidgetView> {
        self.component(component_id)
            .filter(|widget| !widget.hidden && widget.client_code <= 0)
    }

    fn book_forward_visible(&self) -> Option<bool> {
        let marker_id = self.ids?.book_forward_marker;
        self.component(marker_id).map(|marker| !marker.hidden)
    }

    fn page_fingerprint(&self) -> u64 {
        let mut hash = DefaultHasher::new();
        self.root.hash(&mut hash);
        self.texts.hash(&mut hash);
        if let Some(widgets) = self.widgets {
            for widget in widgets.iter().filter(|widget| {
                widget.root == WidgetRoot::Main && widget.root_component_id == self.root
            }) {
                widget.component_id.hash(&mut hash);
                widget.parent_id.hash(&mut hash);
                widget.type_.hash(&mut hash);
                widget.button_type.hash(&mut hash);
                widget.client_code.hash(&mut hash);
                widget.hidden.hash(&mut hash);
                widget.text.hash(&mut hash);
                widget.alternate_text.hash(&mut hash);
                widget.button_text.hash(&mut hash);
                widget.actions.hash(&mut hash);
            }
        }
        hash.finish()
    }
}

fn selected_dialogue_ui(cx: &ActionContext<'_>) -> Option<api::game_data::DialogueUiIds> {
    let data = api::game_data::for_revision(cx.pin().revision).ok()?;
    let selected_pin = data.selected_pin().ok()?;
    if selected_pin.as_ref() != cx.pin() {
        return None;
    }
    data.dialogue_ui().copied()
}

pub(super) fn chat_page_open(root: i32, continue_component_id: i32) -> bool {
    root != -1 || continue_component_id >= 0
}

/// Operation adoption covers chat and selected documents, not unrelated interfaces.
pub(super) fn page_open(cx: &ActionContext<'_>) -> bool {
    cx.snapshot()
        .chat_modal()
        .is_some_and(|chat| chat_page_open(chat.value.root, chat.value.continue_component_id))
        || observe_main(cx, selected_dialogue_ui(cx))
            .is_some_and(|main| matches!(main.kind, MainKind::Scroll | MainKind::Book))
}

/// The option a held conversation's step answers on the open chat menu
/// (`quester::conversation`), or `None`: a Main modal, a continue page, or a
/// menu its text answers do not single out
/// ([`DialogueOptions::unique_text_answer`]).
pub(crate) fn held_menu_answer(cx: &ActionContext<'_>, options: &DialogueOptions) -> Option<i32> {
    let obs = observe(cx)?;
    if !obs.open || obs.r#continue || obs.options.is_empty() {
        return None;
    }
    if observe_main(cx, None)?.open() {
        return None;
    }
    options.unique_text_answer(obs.texts, obs.options)
}

fn observe<'a>(cx: &ActionContext<'a>) -> Option<ChatObs<'a>> {
    let chat = cx.snapshot.chat_modal()?;
    let modal = chat.value.root;
    let continue_component_id = chat.value.continue_component_id;
    let r#continue = continue_component_id >= 0;
    let open = modal != -1;
    Some(ChatObs {
        open,
        ready: chat_page_open(modal, continue_component_id),
        r#continue,
        continue_component_id,
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

fn observe_main<'a>(
    cx: &ActionContext<'a>,
    selected_ui: Option<api::game_data::DialogueUiIds>,
) -> Option<MainObs<'a>> {
    let main = cx.snapshot.main_modal()?;
    let root = main.value.root;
    let ids = (root != -1).then_some(selected_ui).flatten();
    let kind = if root == -1 {
        MainKind::Closed
    } else if let Some(ids) = ids {
        if root == ids.scroll_root || root == ids.quest_scroll_root {
            MainKind::Scroll
        } else if root == ids.book_root {
            MainKind::Book
        } else {
            MainKind::Unsupported
        }
    } else {
        MainKind::Unsupported
    };
    Some(MainObs {
        root,
        kind,
        ids,
        widgets: cx.snapshot.widgets().map(|widgets| widgets.value),
        texts: cx.snapshot.main_modal_texts()?.value,
        tick: cx.evidence().tick,
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
    prefer
        .iter()
        .find_map(|fragment| matching_option(options, fragment, false).ok().flatten())
}

fn text_matches(text: &str, fragment: &str) -> bool {
    !text.is_empty()
        && (fragment.is_empty()
            || text
                .as_bytes()
                .windows(fragment.len())
                .any(|part| part.eq_ignore_ascii_case(fragment.as_bytes())))
}

fn matching_option(
    options: &[api::snapshot::ChatOptionView],
    fragment: &str,
    strict: bool,
) -> Result<Option<usize>, ()> {
    let mut matches = options
        .iter()
        .enumerate()
        .filter(|(_, option)| text_matches(&option.text, fragment));
    let first = matches.next().map(|(index, _)| index);
    if strict && matches.next().is_some() {
        Err(())
    } else {
        Ok(first)
    }
}

pub fn poll_dialogue(
    actions: &mut NativeActions,
    handle: &ActionHandle<Dialogue>,
    cx: &mut ActionContext<'_>,
) -> Poll<Result<DialogueOutcome, ActionError>> {
    actions.poll(handle, cx)
}
