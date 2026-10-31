//! Native chat and selected scroll/book continuation driver.
//! A Continue is acknowledged by its accepted receipt on a later observed tick or by a page
//! change; completion and the Continue bound wait on game ticks.

use super::reach;
use crate::dialogue_outcome::DialogueOutcome;
use crate::native::{
    ActionContext, ActionError, ActionHandle, InteractionDispatch, NativeActions, NativeMachine,
};
use crate::shim::InteractReq;
use api::snapshot::{chat_page_fingerprint, WidgetRoot, WidgetView};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::Arc;
use std::task::Poll;

/// Quiet ticks after the chat is observed closed before a conversation counts
/// as over, unless the step authors `gap_ticks`. The engine closes chat with
/// the same `IfClose` at script end (`Player.ts:2223-2229`) and at a script's
/// own `if_close` before a `p_delay` and a later page, so the client cannot
/// know whether a conversation is finished until its close gap settles. A
/// later script that deliberately closes and reopens the chat authors its
/// engine hole (`gap_ticks`).
pub const DIALOG_GAP_TICKS: u64 = 1;
/// Ticks from the engine's `opnpc1` trigger to the first chat-page write, for
/// the largest pre-page delay in the 797 scanned `[opnpc1,...]` handlers:
/// `p_delay(2)` in Captain Siad after a `mes`
/// (`quest_desertrescue/scripts/captain_siad.rs2:2-3`) and in the seven
/// Underground Pass slaves (`quest_upass/scripts/upass_slave.rs2:1-56`).
/// `P_DELAY` resumes the script at `currentTick + 1 + 2`
/// (`PlayerOps.ts:376-379`, `World.ts:692`).
pub const DIALOGUE_PAGE_DELAY_TICKS: u64 = 3;
/// One more observed tick after the page's own cycle. The engine writes
/// `IF_OPENCHAT` in `encodeOut`, after `PLAYER_INFO` (`World.ts:1110-1123`),
/// and the client reads at most five packets per frame (`client.rs:12798-12803`),
/// so the first poll that sees the page's tick can still miss the page.
pub const DIALOGUE_OBSERVE_TICKS: u64 = 1;
/// Observed ticks from the engine's trigger to the first-page deadline. The
/// trigger is the later of the engine's read of the accepted click and the
/// observed arrival beside the NPC. The host stamps the click receipt with the
/// click's own tick (`script_observe.rs:1649-1657`) and the engine reads the
/// packet one cycle later (`World.ts:355` before `World.ts:378`). The poll
/// checks for a page before its deadline, so a page on the deadline tick is
/// still observed. NPC-script messages are not failures: the first one
/// re-anchors the deadline (`Dialogue::observe_talk_message`).
///
/// This budget starts only when the host accepts the Talk-to. Before that
/// the open wait follows the host's dispatch lifecycle, not a tick count
/// ([`crate::native::InteractionDispatch`]): a request the host still holds
/// waits, a request the host dropped unsent is sent again, and a request the
/// host took and refused ends the wait at once.
pub const DIALOGUE_OPEN_TICKS: u64 = DIALOGUE_PAGE_DELAY_TICKS + DIALOGUE_OBSERVE_TICKS;
/// An NPC's legal wander square contains `(2 * wanderrange + 1)^2` tiles.
/// Set the replan cap to the number of non-home tiles in this square. The
/// engine chooses destinations within that square (`Npc.ts:705-728`) and has
/// a 1/8 chance to choose a destination each tick (`Npc.ts:720-726`). The cap
/// remains finite even when the NPC revisits a tile. A replan is spent only
/// when the NPC's observed tile changed since the last plan.
/// Brundt's `wanderrange=2` is sourced at `viking.npc:769-803`.
/// Walk one tile per observed engine cycle, conservatively; running advances
/// two tiles per cycle (`PathingEntity.ts:135-149`). Add two slack ticks: one
/// for the engine to read the input after `processClientsIn` and one for the
/// next `PLAYER_INFO` position observation (`World.ts:350-355`,
/// `NetworkPlayer.ts:171-177`, `host/src/slot.rs:22-25`).
pub(super) const DIALOGUE_APPROACH_SLACK_TICKS: u32 = 2;
/// Observed ticks to wait for the router's answer to one route assessment.
/// The host publishes the answer on the first observation after its route
/// job ends (`script_observe.rs:2116-2119`). A route cleanup drops a pending
/// advisory without an answer (`script_nav.rs:7109-7126`). In the live clean
/// Cook's Assistant run `d3c1` (QUESTER-DIALOGUE-OPEN r3), no walk and no
/// Talk-to followed the Talk pre-walk for 2,000 ticks, until the
/// watchdog restarted the script. [INFERENCE] The Dialogue waited for an
/// answer that never came. The cause is not proven. No content signal exists
/// for host compute time, so this bound only tells a lost request from a slow
/// one. An unanswered request uses the shared three-attempt approach budget.
const DIALOGUE_ASSESS_TICKS: u64 = 8;
pub const DRIVE_STEPS: u32 = 120;
/// Wall-clock bound that the Choice and Main-document acknowledgements still
/// use. A Continue's acknowledgement counts observed ticks
/// ([`PAGE_ACK_TICKS`]).
pub const PAGE_ACK_MS: u64 = 3_000;
/// Observed ticks from a Continue request to the host's dispatch receipt. The
/// host drains the queued interaction in an observation pass
/// (`script_observe.rs:1410`) and stamps the receipt with that pass's tick
/// (`script_observe.rs:1649-1657`). One tick covers a drain that waits for the
/// next observation.
const CONTINUE_DISPATCH_TICKS: u64 = 1;
/// The engine reads the Continue in the cycle after the receipt's tick
/// (`World.ts:355`, before `World.ts:378`). It resumes the paused script in
/// that handler (`ResumePauseButtonHandler.ts:7-13`), and the script's next
/// `p_pausebutton` page is written in the same cycle.
const CONTINUE_ENGINE_TICKS: u64 = 1;
/// Cycles from the Continue to the next page when content waits between two
/// pages. A scan of every `.rs2` script under `content/scripts` found 19,290
/// page calls (`~chatnpc`, `~chatplayer`, `~mesbox`, `~objbox`,
/// `~doubleobjbox`, `~doublemesbox`, `~confirmbox`, `p_pausebutton`). Only 34
/// are followed by a `p_delay` within two statements. The largest is
/// `p_delay(6)` (`quest_grandtree/scripts/king_narnode.rs2:55`). `P_DELAY`
/// resumes the script at `currentTick + 1 + n` (`PlayerOps.ts:376-379`,
/// `World.ts:692`), so the next page is written `6 + 1` cycles later.
const CONTINUE_PAGE_DELAY_TICKS: u64 = 6 + 1;
/// Observed ticks that one page gets to be acknowledged, counted from the tick
/// of the first Continue on it: dispatch, the engine's read cycle, the longest
/// content delay before the next page, and one observed tick for the page's
/// packets ([`DIALOGUE_OBSERVE_TICKS`]). A Continue whose accepted receipt
/// has matured on an unchanged page is acknowledged
/// ([`Dialogue::continue_receipt_acknowledged`]) and pressed again, but that
/// repeat keeps the first press's bound, so a page that never changes ends the
/// wait after this many ticks however many receipts arrive. A request that no
/// accepted receipt reaches ends it the same way. At the 300 ms tick of
/// Engine Q the bound is 3,000 ms, the value of the wall-clock bound that it
/// replaces.
pub const PAGE_ACK_TICKS: u64 = CONTINUE_DISPATCH_TICKS
    + CONTINUE_ENGINE_TICKS
    + CONTINUE_PAGE_DELAY_TICKS
    + DIALOGUE_OBSERVE_TICKS;
/// Ticks before acting on a page that changed text on the same root (a same-
/// root choice page, a book or scroll forward) or offered no input: its text
/// can still be arriving in later frames of the server tick under the
/// client's 5-packets-per-frame read. A Continue or a new root needs none.
pub const PAGE_SETTLE_TICKS: u64 = 1;

/// The native dialogue acknowledgement shared with the v1 adapter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PageAcknowledgement {
    modal: i32,
    continue_visible: bool,
    fingerprint: u64,
}

impl PageAcknowledgement {
    pub fn capture(modal: i32, continue_visible: bool, fingerprint: u64) -> Self {
        Self {
            modal,
            continue_visible,
            fingerprint,
        }
    }

    pub fn acknowledged(self, modal: i32, continue_visible: bool, fingerprint: u64) -> bool {
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

/// `prefer` entries one conversation can remember when `answer_once` is set:
/// one bit per entry in the driver's fixed-size mask.
pub const ANSWER_ONCE_PREFER_MAX: usize = u32::BITS as usize;

#[derive(Clone)]
pub struct DialogueOptions {
    pub prefer: Arc<[Arc<str>]>,
    pub choose: Option<i32>,
    pub line_rules: Arc<[LineRule]>,
    pub strict: bool,
    /// Skip a `prefer` entry that already answered a page in this
    /// conversation, falling through to the next entry in order. A repeated
    /// menu can then be answered once and left. `line_rules` and `choose` are
    /// unaffected. Compilation caps `prefer` at [`ANSWER_ONCE_PREFER_MAX`].
    pub answer_once: bool,
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
            answer_once: false,
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

    /// Select the answer for one page. `used` is the conversation's mask of
    /// `prefer` entries that already answered an accepted page; a selection
    /// names the entry it used so the caller marks it only once the answer's
    /// receipt is accepted and the page advances.
    fn select(&self, obs: &ChatObs<'_>, used: u32) -> Option<Selection> {
        if let Some(rule) = self.line_rules.iter().find(|rule| {
            obs.texts
                .iter()
                .any(|text| text_matches(text, &rule.when_line))
        }) {
            match matching_option(obs.options, &rule.choose, self.strict) {
                Ok(Some(index)) => return Some(Selection::option(index)),
                Err(()) => return None,
                Ok(None) if self.strict => return None,
                Ok(None) => {}
            }
        }
        if let Some(option) = self.choose {
            return (option > 0 && option as usize <= obs.options.len())
                .then(|| Selection::option_fixed(option));
        }
        if self.strict {
            for (index, fragment) in self.prefer.iter().enumerate() {
                if self.answer_once && prefer_used(used, index) {
                    continue;
                }
                match matching_option(obs.options, fragment, true) {
                    Ok(Some(option)) => {
                        return Some(Selection {
                            option: (option + 1) as i32,
                            prefer: self.answer_once.then_some(index),
                        })
                    }
                    Err(()) => return None,
                    Ok(None) => {}
                }
            }
            None
        } else {
            match self
                .prefer
                .iter()
                .enumerate()
                .find_map(|(index, fragment)| {
                    if self.answer_once && prefer_used(used, index) {
                        return None;
                    }
                    matching_option(obs.options, fragment, false)
                        .ok()
                        .flatten()
                        .map(|option| (index, option))
                }) {
                Some((index, option)) => Some(Selection {
                    option: (option + 1) as i32,
                    prefer: self.answer_once.then_some(index),
                }),
                None => Some(Selection::option_fixed(obs.options.len() as i32)),
            }
        }
    }
}

/// One chosen answer and the `prefer` entry that chose it, if any.
#[derive(Debug, Clone, Copy)]
struct Selection {
    option: i32,
    prefer: Option<usize>,
}

impl Selection {
    fn option(index: usize) -> Self {
        Self {
            option: (index + 1) as i32,
            prefer: None,
        }
    }

    fn option_fixed(option: i32) -> Self {
        Self {
            option,
            prefer: None,
        }
    }
}

/// Whether `prefer[index]` already answered a page in this conversation.
/// An index outside the mask width is never used; compilation rejects it.
fn prefer_used(used: u32, index: usize) -> bool {
    used & 1u32.checked_shl(index as u32).unwrap_or(0) != 0
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
    /// Waiting for the router's route length before the walk starts, or for the
    /// tick on which a fresh plan follows a walk failure.
    Plan,
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
    /// `prefer` entries whose answer was accepted in this conversation, one
    /// bit each. Only read when `answer_once` is set; marked by
    /// [`Self::mark_answer_once`] once the answer's receipt is accepted and
    /// the page acknowledgement shows the page advanced.
    answers_used: u32,
    /// The `prefer` entry that chose the answer awaiting acknowledgement and
    /// the request id of that answer. Queueing the interaction does not
    /// accept it, so the entry stays unmarked until both the receipt is
    /// accepted and the page advances; a refused answer, a lost receipt or a
    /// page that did not advance leaves it unused.
    pending_prefer: Option<(usize, u64)>,
    due_tick: u64,
    gap_inventory: Option<u64>,
    gap_position: Option<api::WorldTile>,
    gap_chat_mark: i32,
    gap_rearms_left: u64,
    /// The last close-gap rearm tick.
    gap_rearm_tick: Option<u64>,
    /// The first tick the bot was observed beside the NPC after the Talk click.
    talk_arrival_tick: Option<u64>,
    /// The tick the NPC's first game message after the Talk click was observed.
    talk_message_tick: Option<u64>,
    ack_page: PageAcknowledgement,
    npc_index: i32,
    npc_action: Arc<str>,
    deadline_ms: u64,
    /// Deadline for the current approach route, route assessment or first-page
    /// wait.
    content_deadline_tick: u64,
    walk_request_id: u64,
    /// Shared attempt accounting. `due_tick` holds its later-tick retry.
    walk_recovery: crate::native::ReturnStallRecovery,
    /// Remaining routes to replan after the NPC wanders.
    approach_replans_left: u16,
    /// Planned walking ticks plus slack from Talk request to adjacent stand.
    talk_approach_budget_ticks: u32,
    /// An explicit refusal has started the one recovery approach.
    refusal_recovery_active: bool,
    /// NPC tile that the current route was planned to. A route that ends
    /// short replans only when the NPC has since moved.
    plan_tile: api::WorldTile,
    /// Arrival radius of the planned route.
    plan_radius: u16,
    /// NPC tile from the newest snapshot that showed the NPC. While the NPC
    /// is missing from the snapshot, the approach holds this tile.
    last_seen_tile: api::WorldTile,
    /// Compute-only route assessment awaited in `Phase::Plan`; 0 when none.
    assess_request_id: u64,
    talk_request_id: Option<u64>,
    talk_baseline: Option<PageAcknowledgement>,
    talk_chat_mark: Option<i32>,
    chat_advanced: bool,
    chat_page_owned: bool,
    chat_advance_request_id: Option<u64>,
}
// Keep retained per-action dialogue state within its reviewed size budget.
// 400 B measured on x86_64 Linux (no headroom left). The approach round 3 fix
// added the route plan (tile, radius, assessment request, retries) and a
// separate Talk arrival and NPC-message tick: 56 B over the 336 B of round 2.
// QUESTER-TALK-APPROACH-HOLD added the 12 B last seen NPC tile: 8 B over 392 B.
const _: () = assert!(std::mem::size_of::<Dialogue>() <= 400);

impl NativeMachine for Dialogue {
    type Args = DialogueArgs;
    type Output = DialogueOutcome;

    fn begin(args: Self::Args, cx: &mut ActionContext<'_>) -> Result<Self, ActionError> {
        let approach_replans_left = match &args.target {
            DialogueTarget::Npc { id, .. } => dialogue_approach_replan_budget(cx, *id),
            DialogueTarget::Continuation => 0,
        };
        let mut dialogue = Self {
            args,
            phase: Phase::Open,
            main_ui: selected_dialogue_ui(cx),
            steps: 0,
            answers_used: 0,
            pending_prefer: None,
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
            talk_arrival_tick: None,
            talk_message_tick: None,
            ack_page: PageAcknowledgement::default(),
            npc_index: -1,
            npc_action: Arc::from("Talk-to"),
            deadline_ms: 0,
            content_deadline_tick: 0,
            walk_request_id: 0,
            walk_recovery: crate::native::ReturnStallRecovery::with_max_attempts(
                crate::native::QUEST_WALK_MAX_ATTEMPTS,
            ),
            approach_replans_left,
            talk_approach_budget_ticks: 0,
            refusal_recovery_active: false,
            plan_tile: api::WorldTile {
                x: 0,
                z: 0,
                level: 0,
            },
            plan_radius: 0,
            last_seen_tile: api::WorldTile {
                x: 0,
                z: 0,
                level: 0,
            },
            assess_request_id: 0,
            surface: Surface::Unselected,
            talk_request_id: None,
            talk_baseline: None,
            talk_chat_mark: None,
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
                            ChatTransitionAck::Pending if obs.tick < self.content_deadline_tick => {
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
        // The local hit window also covers self-damage. Require visible or
        // slot-owned attacker evidence before treating a closed chat as combat.
        // A visible Continue component is an active page even without a root.
        if DialogueOutcome::combat_interrupted(obs.ready, obs.in_combat, obs.attacker_evidence) {
            return Poll::Ready(Err(ActionError::CombatInterrupted));
        }
        match self.phase {
            Phase::Approach | Phase::Plan => {
                let seen = self
                    .npc_id()
                    .and_then(|id| nearest_talk(cx, id))
                    .map(|target| approach_goal(cx, &target));
                // An NPC missing from a snapshot during an approach is
                // transient, for example while it wanders (`Npc.ts:705-728`)
                // or near a scene edge. The route keeps the last seen tile and
                // its own observed-tick budget, and a later snapshot
                // reacquires the NPC. A route that ends with the NPC still
                // unseen is a walk failure with the same bounded fresh plan.
                let goal = match seen {
                    Some(goal) => {
                        self.last_seen_tile = goal.tile;
                        goal
                    }
                    None => held_goal(cx, self.last_seen_tile),
                };
                let terminal_walk = (self.walk_request_id != 0)
                    .then(|| cx.walk_receipt(self.walk_request_id).cloned())
                    .flatten()
                    .filter(|receipt| {
                        !crate::native::WalkRetryState::retryable_receipt(receipt)
                            && receipt.end != crate::native::WalkEnd::Arrived
                    });
                if let Some(receipt) = terminal_walk {
                    self.cancel_route(cx);
                    return Poll::Ready(Err(receipt
                        .into_arrival()
                        .expect_err("non-arrival walk receipt must return an error")));
                }
                if goal.close {
                    self.walk_recovery.reset();
                    std::task::ready!(crate::native::defer_budget(self.open(cx)))?;
                    self.cancel_route(cx);
                    return Poll::Pending;
                }
                self.poll_route(cx, obs.tick, goal)
            }
            Phase::Open => {
                if obs.ready {
                    self.talk_chat_mark = None;
                    self.refusal_recovery_active = false;
                    return self.drive(cx, &obs);
                }
                // NPC content can send a game message before its first chat page.
                // Keep waiting for the page; only the engine's explicit reach
                // refusal takes the recovery branch below.
                if self
                    .talk_chat_mark
                    .is_some_and(|since| reach::saw_game_message(cx, since, reach::CANT_REACH))
                {
                    let Some(npc_id) = self.npc_id() else {
                        self.cancel_talk_request(cx);
                        return Poll::Ready(Err(dialogue_refusal_error()));
                    };
                    if self.refusal_recovery_active {
                        self.cancel_talk_request(cx);
                        return Poll::Ready(Err(dialogue_refusal_error()));
                    }
                    let goal = nearest_talk(cx, npc_id).map(|target| approach_goal(cx, &target));
                    let Some(goal) = goal else {
                        self.cancel_talk_request(cx);
                        return Poll::Ready(Err(dialogue_refusal_error()));
                    };
                    // The recovery walk gets the same route length as any
                    // approach: the flood's, or the router's when the flood
                    // cannot reach the NPC.
                    std::task::ready!(crate::native::defer_budget(self.start_route(cx, goal)))?;
                    self.cancel_talk_request(cx);
                    self.refusal_recovery_active = true;
                    self.walk_recovery.reset();
                    self.chat_advanced = false;
                    self.chat_page_owned = false;
                    self.chat_advance_request_id = None;
                    return Poll::Pending;
                }
                self.observe_talk_arrival(cx, obs.tick);
                self.observe_talk_message(cx, obs.tick);
                match self.update_talk_open_deadline(cx) {
                    // The host still holds the click: no pass that may
                    // dispatch has run since it was queued. The engine
                    // cannot trigger `opnpc1` from a click it never received,
                    // so the page budget does not run yet.
                    TalkDispatch::Queued => return Poll::Pending,
                    // A held tick, a guardian hold or a session reset dropped
                    // the click unsent: send it again ("a later retry
                    // re-queues what still matters", `script_observe.rs:1335-
                    // 1339`). Only a host hold or reset drops queued gameplay,
                    // and the script does not run while the host holds it.
                    TalkDispatch::Unsent => {
                        std::task::ready!(crate::native::defer_budget(self.open(cx)))?;
                        return Poll::Pending;
                    }
                    // The host took the click in a pass that could dispatch
                    // it and did not send it. Its receipt is settled
                    // (`Ledger::complete_interaction` keeps the first one), so
                    // no later acceptance can follow, and no page is open.
                    TalkDispatch::Refused(tick) => {
                        return Poll::Ready(Err(self.talk_refused_error(cx, tick)));
                    }
                    TalkDispatch::Accepted | TalkDispatch::NotRequested => {}
                }
                if obs.tick >= self.content_deadline_tick {
                    static REASON: std::sync::LazyLock<Arc<str>> =
                        std::sync::LazyLock::new(|| Arc::from("expected dialogue did not open"));
                    return Poll::Ready(Err(ActionError::Failed(Arc::clone(&REASON))));
                }
                Poll::Pending
            }
            Phase::WaitContinueAck => {
                // A Continue is acknowledged by its own accepted receipt on a
                // later observed tick, or by a changed page. The receipt is the
                // signal that works when the next page is identical to this
                // one (two `~mesbox` calls with the same text in a row,
                // `quest_arthur.rs2:15-16`): the server reuses the chat root
                // and Continue component, so such a page has the same
                // acknowledgement values as the one that was dismissed.
                // An unchanged page is pressed again only inside the tick
                // bound of its first press: once that bound has passed, a
                // matured receipt cannot renew it, however sparse the
                // observations were. The repeat stays in this phase until the
                // host admits it, so a repeat deferred for the tick's event
                // allowance meets this same check on the next poll.
                let page_changed =
                    self.ack_page
                        .acknowledged(obs.modal, obs.r#continue, obs.page_fingerprint());
                if page_changed {
                    // The engine resumes the paused script in the ack packet's
                    // own cycle (`ResumePauseButtonHandler.ts:7-13`) and writes
                    // the next `~chatnpc`/`~chatplayer` page in that execution
                    // (`chat.rs2:271-321`): no tick is owed before driving it.
                    // A Continue does not read the page's text, so a page still
                    // mid-update under the client's 5-packets-per-frame read
                    // is safe to press. A closed chat starts the end gap now.
                    // The changed page gets its own bound when it is pressed.
                    self.phase = Phase::Drive;
                    return self.drive(cx, &obs);
                }
                if obs.ready
                    && obs.r#continue
                    && obs.tick <= self.content_deadline_tick
                    && self.continue_receipt_acknowledged(cx)
                {
                    return self.press_repeat(cx, &obs);
                }
                if obs.tick >= self.content_deadline_tick {
                    return Poll::Ready(Ok(DialogueOutcome::Failed));
                }
                Poll::Pending
            }
            Phase::WaitChoiceAck => {
                if self
                    .ack_page
                    .acknowledged(obs.modal, obs.r#continue, obs.page_fingerprint())
                {
                    // The page advanced. Mark the `prefer` entry that chose
                    // this answer only when the answer's own receipt says it
                    // was accepted, before the follow-up `drive` can select
                    // the next page; a refused or lost receipt leaves the
                    // entry unused so the next menu picks it again.
                    self.mark_answer_once(cx);
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

    /// Whether this driver's own Continue request was accepted by the host and
    /// a later observed tick has arrived. An accepted Continue that the
    /// engine read is one dismissed page, whether or not the next page looks
    /// different. The engine reads the request in the cycle after the
    /// receipt's tick and writes the next page in that cycle, after its
    /// `PLAYER_INFO`; the client can need one more observed tick to apply it
    /// ([`DIALOGUE_OBSERVE_TICKS`]). An accept on the send tick, or on the
    /// tick after it, is therefore not yet proof that the next page is up.
    fn continue_receipt_acknowledged(&self, cx: &ActionContext<'_>) -> bool {
        let Some(request_id) = self.chat_advance_request_id else {
            return false;
        };
        let Some(receipt) = cx.interaction_receipt(request_id) else {
            return false;
        };
        let evidence = cx.evidence();
        receipt.accepted
            && receipt.evidence.run == evidence.run
            && evidence.tick
                >= receipt
                    .evidence
                    .tick
                    .saturating_add(CONTINUE_ENGINE_TICKS + DIALOGUE_OBSERVE_TICKS)
    }

    /// Mark the pending `prefer` entry now that the page has advanced, but
    /// only when the answer's own receipt is accepted. Queueing the answer is
    /// not acceptance: a refused answer and a lost receipt both leave the
    /// entry unused, so the next menu selects it again instead of falling
    /// through to a later entry.
    fn mark_answer_once(&mut self, cx: &ActionContext<'_>) {
        let Some((index, request_id)) = self.pending_prefer.take() else {
            return;
        };
        let accepted = cx
            .interaction_receipt(request_id)
            .is_some_and(|receipt| receipt.accepted);
        if accepted {
            self.answers_used |= 1u32.checked_shl(index as u32).unwrap_or(0);
        }
    }
    fn cancel_talk_request(&mut self, cx: &mut ActionContext<'_>) {
        if let Some(request_id) = self.talk_request_id.take() {
            cx.cancel_request(request_id);
        }
        self.talk_baseline = None;
        self.talk_chat_mark = None;
    }

    /// The named terminal failure of an approach whose retries are spent. A
    /// failure after a server refusal keeps the refusal text first. When the
    /// NPC is missing from the current snapshot, the text names the NPC and
    /// the tile where it was last seen.
    fn approach_error(&self, cx: &ActionContext<'_>, why: WalkFailure) -> ActionError {
        let text = match why {
            WalkFailure::NoRoute(detail) => format!("no route to the NPC: {detail}"),
            WalkFailure::Ended(detail) => {
                format!("the walk ended before the NPC was in reach: {detail}")
            }
            WalkFailure::Late => format!(
                "the walk did not reach the NPC by observed tick {}",
                self.content_deadline_tick
            ),
            WalkFailure::NoAnswer => format!(
                "the router did not answer the route assessment by observed tick {}",
                self.content_deadline_tick
            ),
            WalkFailure::NpcKeepsMoving => "the NPC kept moving through every replan".to_owned(),
        };
        let text = match &self.args.target {
            DialogueTarget::Npc { id, name } if nearest_talk(cx, *id).is_none() => {
                let tile = self.last_seen_tile;
                format!(
                    "{text}; {name} is not in view, last seen at ({}, {}, level {})",
                    tile.x, tile.z, tile.level
                )
            }
            _ => text,
        };
        let reason = if self.refusal_recovery_active {
            format!("{DIALOGUE_REFUSAL}; recovery walk failed: {text}")
        } else {
            format!("dialogue approach failed: {text}")
        };
        ActionError::Failed(Arc::from(reason))
    }

    /// Cancel the walk and the route assessment, whichever is in flight.
    fn cancel_route(&mut self, cx: &mut ActionContext<'_>) {
        cx.cancel_request(self.walk_request_id);
        self.walk_request_id = 0;
        cx.cancel_request(self.assess_request_id);
        self.assess_request_id = 0;
    }

    /// Record a started walk. Its deadline counts observed ticks from the
    /// tick on which the walk was requested.
    fn begin_walk(&mut self, cx: &ActionContext<'_>, request_id: u64, budget_ticks: u32) {
        self.walk_request_id = request_id;
        self.phase = Phase::Approach;
        self.content_deadline_tick = cx.evidence().tick.saturating_add(u64::from(budget_ticks));
    }

    /// Start a route to the NPC's observed tile. The live flood's route depth
    /// is the budget when it reaches the NPC. Otherwise the router assesses the
    /// same request first (`Phase::Plan`), and the walk's budget is the
    /// router's route length, which includes any door or wall detour.
    fn start_route(
        &mut self,
        cx: &mut ActionContext<'_>,
        goal: ApproachGoal,
    ) -> Result<(), ActionError> {
        let request = reach::walk_request(goal.tile, goal.radius, None, cx.evidence());
        let previous = self.walk_request_id;
        match goal.route_steps {
            Some(steps) => {
                let request_id = cx.walk(request)?;
                self.begin_walk(
                    cx,
                    request_id,
                    steps.saturating_add(DIALOGUE_APPROACH_SLACK_TICKS),
                );
            }
            None => {
                self.assess_request_id = cx.assess_walk(request)?;
                self.walk_request_id = 0;
                self.phase = Phase::Plan;
                self.content_deadline_tick =
                    cx.evidence().tick.saturating_add(DIALOGUE_ASSESS_TICKS);
            }
        }
        if previous != 0 && previous != self.walk_request_id {
            cx.cancel_request(previous);
        }
        self.plan_tile = goal.tile;
        self.plan_radius = goal.radius;
        self.last_seen_tile = goal.tile;
        self.talk_arrival_tick = None;
        self.talk_message_tick = None;
        Ok(())
    }

    /// Follow the route. A refusal stops at once. A retryable failure gets a
    /// fresh plan from the observed position on a later tick.
    fn poll_route(
        &mut self,
        cx: &mut ActionContext<'_>,
        tick: u64,
        goal: ApproachGoal,
    ) -> Poll<Result<DialogueOutcome, ActionError>> {
        if self.phase == Phase::Plan {
            return self.poll_plan(cx, tick, goal);
        }
        let receipt = (self.walk_request_id != 0)
            .then(|| cx.walk_receipt(self.walk_request_id).cloned())
            .flatten();
        if let Some(receipt) = receipt.as_ref() {
            if !crate::native::WalkRetryState::retryable_receipt(receipt)
                && receipt.end != crate::native::WalkEnd::Arrived
            {
                self.cancel_route(cx);
                return Poll::Ready(Err(receipt
                    .clone()
                    .into_arrival()
                    .expect_err("non-arrival walk receipt must return an error")));
            }
        }
        if receipt.is_none() && tick < self.content_deadline_tick {
            return Poll::Pending;
        }
        let why = receipt.as_ref().map_or(WalkFailure::Late, |receipt| {
            WalkFailure::Ended(
                receipt
                    .failure_detail()
                    .unwrap_or_else(|| Arc::from(format!("{:?}", receipt.end))),
            )
        });
        if goal.tile != self.plan_tile {
            if self.approach_replans_left == 0 {
                self.cancel_route(cx);
                return Poll::Ready(Err(self.approach_error(cx, WalkFailure::NpcKeepsMoving)));
            }
            std::task::ready!(crate::native::defer_budget(self.start_route(cx, goal)))?;
            self.approach_replans_left -= 1;
            self.walk_recovery.reset();
            return Poll::Pending;
        }
        self.walk_failed(cx, tick, why)
    }

    /// `Phase::Plan`: wait for the router's route length, then walk with it.
    /// Only the router's no-route refusal uses the bounded retry budget.
    fn poll_plan(
        &mut self,
        cx: &mut ActionContext<'_>,
        tick: u64,
        goal: ApproachGoal,
    ) -> Poll<Result<DialogueOutcome, ActionError>> {
        if self.assess_request_id == 0 {
            if tick < self.due_tick {
                return Poll::Pending;
            }
            std::task::ready!(crate::native::defer_budget(self.start_route(cx, goal)))?;
            self.walk_recovery.action_submitted();
            return Poll::Pending;
        }
        let Some(receipt) = cx.assess_receipt(self.assess_request_id) else {
            if tick >= self.content_deadline_tick {
                return self.walk_failed(cx, tick, WalkFailure::NoAnswer);
            }
            return Poll::Pending;
        };
        if let Some(refusal) = receipt.refusal {
            let detail = receipt
                .detail
                .clone()
                .unwrap_or_else(|| Arc::from(refusal.to_string()));
            if matches!(
                refusal,
                crate::native::WalkRefusal::NoRouteWithinBounds { .. }
            ) {
                return self.walk_failed(cx, tick, WalkFailure::NoRoute(detail));
            }
            self.cancel_route(cx);
            return Poll::Ready(Err(ActionError::Blocked(detail)));
        }
        let route_ticks = receipt.route_ticks;
        let request = reach::walk_request(self.plan_tile, self.plan_radius, None, cx.evidence());
        let request_id = std::task::ready!(crate::native::defer_budget(cx.walk(request)))?;
        cx.cancel_request(self.assess_request_id);
        self.assess_request_id = 0;
        self.begin_walk(
            cx,
            request_id,
            route_ticks.saturating_add(DIALOGUE_APPROACH_SLACK_TICKS),
        );
        Poll::Pending
    }

    /// A route failed with the NPC still or unseen. Spend a bounded fresh
    /// plan from the observed position, or report the last failure once spent.
    fn walk_failed(
        &mut self,
        cx: &mut ActionContext<'_>,
        tick: u64,
        why: WalkFailure,
    ) -> Poll<Result<DialogueOutcome, ActionError>> {
        let walk_request_id = self.walk_request_id;
        let assess_request_id = self.assess_request_id;
        let attempt = self.walk_recovery.retry_attempt_number().saturating_sub(1);
        let retry = self.walk_recovery.observe_retryable(true);
        let result = if retry { "retry" } else { "exhausted" };
        api::host_log!(
            api::hostlog::Category::NavTrace,
            api::hostlog::Level::Debug,
            "quester-dialogue-approach-{result} tick={tick} attempt={attempt}/{} remaining={} walk_request={walk_request_id} assess_request={assess_request_id} goal={:?} reason={why:?}",
            crate::native::QUEST_WALK_MAX_ATTEMPTS,
            crate::native::QUEST_WALK_MAX_ATTEMPTS.saturating_sub(attempt),
            self.plan_tile,
        );
        self.cancel_route(cx);
        if !retry {
            return Poll::Ready(Err(self.approach_error(cx, why)));
        }
        self.phase = Phase::Plan;
        self.due_tick = tick.saturating_add(1);
        Poll::Pending
    }

    fn observe_talk_arrival(&mut self, cx: &ActionContext<'_>, tick: u64) {
        if self.talk_arrival_tick.is_some() {
            return;
        }
        let Some(target) = self.npc_id().and_then(|id| nearest_talk(cx, id)) else {
            return;
        };
        if reach::npc_adjacent(cx, target.npc) {
            self.talk_arrival_tick = Some(tick);
        }
    }

    /// The NPC script's first game message after the click proves that the
    /// engine ran `opnpc1` (it runs only in range), and the page follows the
    /// message by the content's own delay (`p_delay`), so it anchors the wait.
    fn observe_talk_message(&mut self, cx: &ActionContext<'_>, tick: u64) {
        if self.talk_message_tick.is_some() {
            return;
        }
        if self
            .talk_chat_mark
            .is_some_and(|since| reach::saw_game_message(cx, since, ""))
        {
            self.talk_message_tick = Some(tick);
        }
    }

    /// What the host did with the Talk-to, and the first-page deadline once
    /// it accepted it. Until acceptance there is no deadline: the host's
    /// dispatch lifecycle settles the click ([`TalkDispatch`]). After
    /// acceptance the deadline is counted from the engine's own trigger. The
    /// host stamps the receipt with the click tick, so the engine reads the
    /// click one cycle later; when the bot walks the last tiles after the
    /// click, the trigger is the cycle of arrival. Until the bot is beside
    /// the NPC, the planned route plus slack bounds the wait.
    fn update_talk_open_deadline(&mut self, cx: &ActionContext<'_>) -> TalkDispatch {
        let Some(request_id) = self.talk_request_id else {
            return TalkDispatch::NotRequested;
        };
        let receipt = match cx.interaction_dispatch(request_id) {
            InteractionDispatch::Queued => return TalkDispatch::Queued,
            InteractionDispatch::Unsent => return TalkDispatch::Unsent,
            InteractionDispatch::Settled(receipt) if !receipt.accepted => {
                return TalkDispatch::Refused(receipt.evidence.tick);
            }
            InteractionDispatch::Settled(receipt) => receipt,
        };
        let click_tick = receipt.evidence.tick;
        let mut deadline = click_tick.saturating_add(u64::from(self.talk_approach_budget_ticks));
        if let Some(arrival_tick) = self.talk_arrival_tick {
            deadline = click_tick
                .saturating_add(1)
                .max(arrival_tick)
                .saturating_add(DIALOGUE_OPEN_TICKS);
        }
        if let Some(message_tick) = self.talk_message_tick {
            deadline = message_tick.saturating_add(DIALOGUE_OPEN_TICKS);
        }
        self.content_deadline_tick = deadline;
        TalkDispatch::Accepted
    }

    /// The host refused the Talk-to and no page is open. The error names the
    /// request, the NPC, the refusal's observed tick and the last game
    /// message since the click.
    fn talk_refused_error(&self, cx: &ActionContext<'_>, tick: u64) -> ActionError {
        let name = match &self.args.target {
            DialogueTarget::Npc { name, .. } => name.as_ref(),
            DialogueTarget::Continuation => "",
        };
        let last = self
            .talk_chat_mark
            .and_then(|since| last_game_message(cx, since))
            .map_or_else(String::new, |text| format!("; last game message: {text}"));
        ActionError::Failed(Arc::from(format!(
            "expected dialogue did not open: {} {name}: the host refused it on observed tick {tick}{last}",
            self.npc_action
        )))
    }

    /// The conversation's used-`prefer` mask, for the acceptance tests.
    #[cfg(test)]
    pub(super) fn answers_used(&self) -> u32 {
        self.answers_used
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
        let mut talk_approach_budget_ticks = DIALOGUE_APPROACH_SLACK_TICKS;
        let npc_id = self.npc_id();
        if let Some(target) = npc_id.and_then(|id| nearest_talk(cx, id)) {
            let goal = approach_goal(cx, &target);
            if !goal.close {
                return self.start_route(cx, goal);
            }
            self.npc_index = target.index;
            self.npc_action = Arc::from(target.action);
            // The engine walks the last tiles after the click. The flood's
            // route depth bounds them; with no flood posted, the observed
            // distance does, which is at most two tiles here.
            talk_approach_budget_ticks = goal
                .route_steps
                .unwrap_or_else(|| u32::try_from(goal.distance.max(0)).unwrap_or(u32::MAX))
                .saturating_add(DIALOGUE_APPROACH_SLACK_TICKS);
        }
        let DialogueTarget::Npc { name, .. } = &self.args.target else {
            self.surface = Surface::Unselected;
            self.phase = Phase::Open;
            self.content_deadline_tick = cx.evidence().tick.saturating_add(DIALOGUE_OPEN_TICKS);
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
        self.talk_chat_mark = Some(reach::last_chat_seq(cx));
        self.talk_approach_budget_ticks = talk_approach_budget_ticks;
        self.talk_arrival_tick = None;
        self.talk_message_tick = None;
        self.surface = Surface::Chat;
        // No deadline yet: the page budget starts when the host accepts the
        // click (`update_talk_open_deadline`).
        self.phase = Phase::Open;
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
        if DialogueOutcome::combat_interrupted(obs.open(), obs.in_combat, obs.attacker_evidence) {
            return Poll::Ready(Err(ActionError::CombatInterrupted));
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
            | Phase::Plan
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
            self.content_deadline_tick = obs.tick.saturating_add(PAGE_ACK_TICKS);
            self.ack_page = ack_page;
            self.phase = Phase::WaitContinueAck;
            self.chat_advanced = true;
            self.chat_advance_request_id = Some(request_id);
            return Poll::Pending;
        }
        if !obs.options.is_empty() {
            let Some(selection) = self.args.options.select(obs, self.answers_used) else {
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
            let request_id =
                std::task::ready!(crate::native::defer_budget(cx.emit(InteractReq::Answer {
                    option: selection.option
                })))?;
            self.pending_prefer = selection.prefer.map(|index| (index, request_id));
            self.steps += 1;
            self.ack_page = ack_page;
            self.phase = Phase::WaitChoiceAck;
            self.deadline_ms = cx.active_now().as_millis() as u64 + PAGE_ACK_MS;
            self.content_deadline_tick = obs.tick.saturating_add(PAGE_ACK_TICKS);
            self.chat_advanced = true;
            self.chat_advance_request_id = Some(request_id);
            return Poll::Pending;
        }
        self.steps += 1;
        self.phase = Phase::WaitContinueTick;
        self.due_tick = obs.tick.saturating_add(PAGE_SETTLE_TICKS);
        Poll::Pending
    }

    /// Press the same Continue page again because the previous press's
    /// accepted receipt matured while the page had not changed: the identical
    /// second `And wait...`, or a page that ignores the press. The repeat keeps
    /// the first press's tick bound and is not another page toward
    /// `DRIVE_STEPS`, so a page that never changes ends the wait however many
    /// receipts arrive. Nothing changes until the host admits the press
    /// (`defer_budget`): a deferred repeat stays in `WaitContinueAck`, and its
    /// next poll checks the first press's bound again.
    fn press_repeat(
        &mut self,
        cx: &mut ActionContext<'_>,
        obs: &ChatObs,
    ) -> Poll<Result<DialogueOutcome, ActionError>> {
        let ack_page =
            PageAcknowledgement::capture(obs.modal, obs.r#continue, obs.page_fingerprint());
        let request_id = std::task::ready!(crate::native::defer_budget(
            cx.emit(InteractReq::ContinueDialog { component_id: None })
        ))?;
        self.ack_page = ack_page;
        self.chat_advanced = true;
        self.chat_advance_request_id = Some(request_id);
        Poll::Pending
    }
}

/// What the host's dispatch lifecycle did with the open wait's Talk-to
/// ([`InteractionDispatch`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TalkDispatch {
    /// This dialogue sent no Talk-to (a continuation).
    NotRequested,
    /// The host still holds the click for a pass that may dispatch it.
    Queued,
    /// The host dropped the click without sending it.
    Unsent,
    /// The host took the click and refused it, on this observed tick.
    Refused(u64),
    Accepted,
}

/// The text of the newest server game message newer than chat `since`.
fn last_game_message(cx: &ActionContext<'_>, since: i32) -> Option<String> {
    cx.snapshot().chat_lines(since).and_then(|lines| {
        lines
            .value
            .iter()
            .filter(|line| line.username.is_none() && line.type_ == 0)
            .max_by_key(|line| line.sequence)
            .map(|line| line.text.to_string())
    })
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
    attacker_evidence: bool,
}
// 56 B measured on 0cd550dfa (+8 B headroom): borrowed poll observations.
const _: () = assert!(std::mem::size_of::<ChatObs<'static>>() <= 64);

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
    attacker_evidence: bool,
}
// 80 B measured on 0cd550dfa (+16 B headroom): borrowed Main-surface observations.
const _: () = assert!(std::mem::size_of::<MainObs<'static>>() <= 96);

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

/// Operation adoption covers dialogue chat and selected documents, not production menus.
pub(crate) fn page_open(cx: &ActionContext<'_>) -> bool {
    let snapshot = cx.snapshot();
    let production_menu = snapshot
        .make_products()
        .is_some_and(|products| !products.value.is_empty());
    (!production_menu
        && snapshot
            .chat_modal()
            .is_some_and(|chat| chat_page_open(chat.value.root, chat.value.continue_component_id)))
        || observe_main(cx, selected_dialogue_ui(cx))
            .is_some_and(|main| matches!(main.kind, MainKind::Scroll | MainKind::Book))
}

fn combat_attacker_evidence(cx: &ActionContext<'_>) -> bool {
    let snapshot = cx.snapshot();
    // The local combat target is slot-owned state, but only a face entity, not
    // a hit source, and it may be stale. An attacker outside the rebuilt scene
    // is not seen; visible actors targeting this local player are separate
    // evidence.
    let slot_target = snapshot
        .in_combat()
        .is_some_and(|combat| combat.value.target.is_some());
    let Some(local_index) = snapshot
        .local_player()
        .map(|player| player.value.player.index)
    else {
        return slot_target;
    };
    let local_target = api::snapshot::ActorTargetView {
        kind: api::snapshot::ActorKind::Player,
        index: local_index,
    };
    slot_target
        || snapshot.npcs().is_some_and(|npcs| {
            npcs.value
                .iter()
                .any(|npc| npc.target == Some(local_target))
        })
        || snapshot.players().is_some_and(|players| {
            players.value.iter().any(|player| {
                player.index != local_index && player.actor.target == Some(local_target)
            })
        })
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
        attacker_evidence: combat_attacker_evidence(cx),
    })
}

/// Whether `root` is a main document this revision treats as a scroll. A
/// revision without a chapter-page interface has `chapter_root: None`, and no
/// root can match it; the id is never guessed from another revision.
pub(super) fn is_scroll_root(root: i32, ids: api::game_data::DialogueUiIds) -> bool {
    root == ids.scroll_root || root == ids.quest_scroll_root || ids.chapter_root == Some(root)
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
        if is_scroll_root(root, ids) {
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
        attacker_evidence: combat_attacker_evidence(cx),
    })
}

struct TalkTarget<'a> {
    npc: &'a api::snapshot::NpcView,
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

fn dialogue_approach_replan_budget(cx: &ActionContext<'_>, npc_id: i32) -> u16 {
    let Ok(data) = api::game_data::for_revision(cx.pin().revision) else {
        return 0;
    };
    let Some(npc) = data.npc_name(npc_id) else {
        return 0;
    };
    let range = u64::try_from(npc.wanderrange.max(0)).unwrap_or(u64::MAX);
    let side = range.saturating_mul(2).saturating_add(1);
    u16::try_from(side.saturating_mul(side).saturating_sub(1)).unwrap_or(u16::MAX)
}

/// Why an approach route ended without the bot in reach of the NPC.
#[derive(Debug)]
enum WalkFailure {
    /// The router found no route within its configured bounds.
    NoRoute(Arc<str>),
    /// The walk's own receipt ended the route.
    Ended(Arc<str>),
    /// The route's observed-tick budget ran out.
    Late,
    /// The router did not answer a route assessment in time.
    NoAnswer,
    /// Every replan was spent on a moving NPC.
    NpcKeepsMoving,
}

/// What one approach decision reads from the NPC and the live reach flood.
#[derive(Clone, Copy)]
struct ApproachGoal {
    tile: api::WorldTile,
    radius: u16,
    /// Route depth from the live flood, when it reaches the NPC.
    route_steps: Option<u32>,
    distance: i32,
    /// Within two tiles and reachable: click now, the engine walks the rest.
    close: bool,
}

/// The approach goal while the NPC is missing from the snapshot: its last seen
/// tile. The router assesses any new route to it, and the bot never clicks an
/// NPC it cannot see, so the goal is never close.
fn held_goal(cx: &ActionContext<'_>, tile: api::WorldTile) -> ApproachGoal {
    ApproachGoal {
        tile,
        radius: u16::from(talk_reachable(cx, tile)),
        route_steps: None,
        distance: i32::MAX,
        close: false,
    }
}

fn approach_goal(cx: &ActionContext<'_>, target: &TalkTarget<'_>) -> ApproachGoal {
    let reachable = talk_reachable(cx, target.tile);
    ApproachGoal {
        tile: target.tile,
        radius: u16::from(reachable),
        route_steps: reach::npc_route_steps(cx, target.npc),
        distance: target.distance,
        close: target.distance <= 2 && reachable,
    }
}

const DIALOGUE_REFUSAL: &str = "server refused Talk-to: I can't reach that!";

fn dialogue_refusal_error() -> ActionError {
    static REASON: std::sync::LazyLock<Arc<str>> =
        std::sync::LazyLock::new(|| Arc::from(DIALOGUE_REFUSAL));
    ActionError::Failed(Arc::clone(&REASON))
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
                npc,
                index: npc.index as i32,
                action,
                tile: npc.tile,
                distance: npc.distance,
            })
        })
        .min_by_key(|target| target.distance)
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
