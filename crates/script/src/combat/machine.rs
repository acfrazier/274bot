use super::arbiter::{self, PlanRow, RowKind, TickPlan};
use super::arm::{Arm, ArmObservation, ArmStep};
use super::frame::Frame;
use super::policy;
use super::prayer::{PrayerSweep, RaisedPrayers};
use super::request::*;
use super::schedule::{
    elapsed, reached, Interaction, OpKind, Schedule, EAT_OBSERVATION_WINDOW_TICKS,
};
use super::select;
use super::style::magic::{self, CastMode, Refusal};
use super::style::ranged;
use super::tables::{CombatTab, CombatTables, PotionKind, PrayerRole};
use super::threats::{StyleObs, ThreatSet};
use crate::interact::InteractReq;
use crate::native::{
    ActionContext, ActionError, NativeMachine, PvpGlobals, ReturnStallRecovery, WalkEnd,
    WalkRequest, QUEST_WALK_MAX_ATTEMPTS,
};
use crate::survival::cure::{self, CureDecision, CureInputs};
use api::prayer::PrayerObservation;
use api::snapshot::ItemView;
use std::num::NonZeroU64;
use std::sync::Arc;
use std::task::Poll;
use std::time::Duration;

#[path = "switch.rs"]
mod switch;
pub use switch::{RangedNeed, SwitchCause, SwitchState};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Prep,
    Engage,
    Fight,
    Escape,
    WindDown,
}
#[derive(Clone, Copy, Default)]
enum RangedReadinessWait {
    #[default]
    Inactive,
    CombatRoot {
        since: u16,
    },
    StyleEcho {
        since: u16,
    },
}
const PREP_READINESS_TICKS: u16 = 8;
/// Failed stand-approach walks in one fight before the step ends.
/// Each retry is a fresh route plan from the observed tile, which is
/// the recovery for a transient stall or a route that ended short.
/// Three failures return the last walk detail instead of re-walking
/// until the kill budget expires.
const MAX_STAND_WALK_FAILURES: u8 = QUEST_WALK_MAX_ATTEMPTS;
#[derive(Default)]
struct Counters {
    ticks: u16,
    swings: u16,
    casts: u16,
    damage: u16,
    food: u8,
    prayer: u8,
    boost: u8,
    antifire: u8,
    protected: u8,
    switches: u8,
    intruders: u8,
    restorations: u8,
    locked: u8,
    multi: u8,
    ammo: u8,
    pvp_blocked: u8,
}

/// Server-effect identity survives the host's rotating dispatch-receipt ring.
#[derive(Clone, Copy, Default)]
struct PendingRow {
    row: PlanRow,
    baseline: i16,
    age: u8,
}
struct MeleeState {
    anim_id: i32,
    anim_frame: i32,
    mode_fallback: Option<MeleeMode>,
}

/// The identity tag is kept separate so its padding is shared with Combat's
/// other small fields rather than retained beside every 32-bit identity.
#[derive(Default)]
enum EngagementKind {
    #[default]
    None,
    Npc,
    Player,
}

enum StyleState {
    Melee(MeleeState),
    Ranged(Box<RangedState>),
    Magic(Box<MagicState>),
}

struct RangedState {
    ammo_pick: i32,
    anim_update: Option<u32>,
    pickup_baseline: i32,
    sweep_attempts: u8,
    sweep_done: bool,
    readiness_wait: RangedReadinessWait,
}

/// One request has one attack style; idle runs do not carry other styles' state.
struct MagicState {
    arm: Option<Arm>,
    rejected: u32,
    spell_order: [u8; u8::MAX as usize],
    spell_order_len: u8,
    ordered_mask: u32,
    rune_base: i32,
    arm_since: u16,
    arm_wait: u8,
    selected: Option<u8>,
    last_launched: Option<u8>,
    cursor: u8,
    mode: Option<CastMode>,
    armed: bool,
    rune_seen: bool,
    /// Initial arm adoption is allowed once per fresh combat machine only.
    initial_arm_checked: bool,
}

impl Default for MagicState {
    fn default() -> Self {
        Self {
            arm: None,
            rejected: 0,
            spell_order: [0; u8::MAX as usize],
            spell_order_len: 0,
            ordered_mask: 0,
            rune_base: 0,
            arm_since: 0,
            arm_wait: 0,
            selected: None,
            last_launched: None,
            cursor: 0,
            mode: None,
            armed: false,
            rune_seen: false,
            initial_arm_checked: false,
        }
    }
}

impl MagicState {
    fn reset_for_engagement(&mut self) {
        self.arm = None;
        self.rejected = 0;
        self.rune_base = 0;
        self.arm_since = 0;
        self.arm_wait = 0;
        self.selected = None;
        self.last_launched = None;
        self.cursor = 0;
        self.armed = false;
        self.rune_seen = false;
        // Keep the initial observation one-shot across target changes in this machine.
    }
}
/// One shared planner and one host owner. Observation commits are independent
/// from admitted-operation commits; no-op ticks are real progress.
pub struct Combat {
    request: Arc<CombatRequest>,
    tables: Arc<CombatTables>,
    identity: i32,
    identity_kind: EngagementKind,
    deadline: Duration,
    pending_walk: Option<NonZeroU64>,
    threats: ThreatSet,
    schedule: Schedule,
    sweep: PrayerSweep,
    counters: Counters,
    style_state: StyleState,
    plan: TickPlan,
    // Receipt id for the emitted plan.
    plan_first_id: u64,
    pending: [PendingRow; 5],
    // Bits 0-4 stage dispatch slots.
    staged_pending: u8,
    // Three-tick deadlines are retired every observed tick, before byte wrap.
    prayer_ready: [u8; 15],
    prayer_ready_mask: u16,
    // Bit 15 marks readiness; the low 15 bits protect the complete user baseline.
    baseline_on: u16,
    raised_prayers: RaisedPrayers,
    eat_ready: u16,
    food_total: u16,

    // Fourteen wear slots: two failure bits and one Prep-origin bit each.
    // Bits 42–55 hold Prep-origin and skipped boost-family masks.
    prep_failures: u64,
    approach_recovery: ReturnStallRecovery,
    approach_walk_pending: bool,
    sequence: u64,
    /// The right-hand pick of each style with no kit, by `Style::index`
    /// (PVP-DESIGN-1026 §4.2); `-1` is "none".
    rhand_pick: [i32; 3],
    suspended_type: i32,
    chat_since: i32,
    /// The source that authorised STARTING a fight; `None` for self-defence.
    pvp_source: Option<select::PvpSource>,
    /// The tick until which an attack row is held after a single-way line.
    pvp_hold_until: u16,
    /// Out-of-scope refusal lines seen this engagement (bounded).
    oos_retries: u8,
    /// Consecutive "Someone else is already fighting your opponent." lines.
    single_way_lines: u8,
    /// The tick at which the open refusal window (`SW_WINDOW` in `flags2`)
    /// closes.
    single_way_until: u16,
    /// The engaged NPC's slot and its health (`g1`, 0-255) on the previous poll
    /// of an NPC fight; `SW_COMBAT` in `flags2` is its health-bar window on that
    /// poll. They show a fresh hit on it (`npc_hit_landed`).
    hit_index: u16,
    hit_health: u8,
    /// "The duel has not started yet!" lines seen this engagement.
    duel_wait_lines: u8,
    /// `PVP_*` bits; `flags` has all sixteen bits in use.
    flags2: u16,
    /// The executable style (PVP-DESIGN-1026 §4.1): the one whose `style_state`
    /// exists. `switch` names the pending one.
    active: Style,
    switch: SwitchState,
    /// Completed style switches and switches that ended `Failed`.
    switches: u8,
    switch_failed: u8,
    /// Styles whose last switch ended `Failed`, one bit per `Style::index`.
    failed_styles: u8,
    /// The engaged player's protection bits (`headicons & 0b111000`) on this
    /// and on the previous observed frame, and when the last switch completed.
    opp_protect: u8,
    opp_protect_prev: u8,
    last_switch_mask: u8,
    /// Consecutive observed ticks with the engaged player out of melee reach and
    /// no swing onset of ours (T4).
    reach_ticks: u8,
    engaged: Option<ActorRef>,
    suspended: Option<ActorRef>,
    end: Option<CombatEnd>,
    antifire_sip: u16,
    lost_since: u16,
    escape_since: u16,
    last_tick: u16,
    emitted_tick: u16,
    phase: Phase,
    flags: u16,
    stand_walk_failures: u8,
}
const OBSERVED: u16 = 1;
const EMITTED: u16 = 2;
const LOST: u16 = 4;
const ANTIFIRE: u16 = 8;
const PLAN_FIGHT: u16 = 16;
const RESTORE_COUNTED: u16 = 32;
const BOOST_WORTH: u16 = 64;
const SHIELD_OVERRIDE: u16 = 128;
const ENGAGEMENT_ATTACK: u16 = 256;
const FAILED_WEAPON: u16 = 512;
const FAILED_SHIELD: u16 = 1024;
const FAILED_ATTACK: u16 = 2048;
const TERMINAL_FOOD: u16 = 4096;
const PREP_RETALIATE: u16 = 8192;
const SKIP_RETALIATE: u16 = 16384;
const LEASH_WALKED: u16 = 32768;
/// An attack row is held (a single-way or duel-start line is being waited out).
const PVP_HOLD: u16 = 1;
/// A server kill line was seen for the engaged player.
const PVP_KILL_LINE: u16 = 2;
/// A swing onset of ours was observed this tick (consumed by the switch step).
const PVP_SWING: u16 = 4;
/// The engaged NPC's health-bar window was open on the previous poll.
const SW_COMBAT: u16 = 8;
/// A single-way refusal window is open (`single_way_until`).
const SW_WINDOW: u16 = 16;
/// Ticks an attack row stays held after a single-way line (`add(%lastcombat, 8)`,
/// `skill_combat/scripts/pvp/pvp_combat.rs2:102`).
const SINGLE_WAY_WINDOW: u16 = 8;
/// Out-of-scope refusal lines tolerated before `Aborted(Unattackable)`.
const OOS_RETRY: u8 = 3;
/// Windows in a row with a refusal and no landed hit that end the run:
/// "already fighting your opponent" lines for a player (each line counts),
/// engine windows (`SINGLE_WAY_WINDOW` ticks) with an "already under attack"
/// line for an NPC (one count per window).
const SINGLE_WAY_END: u8 = 3;
/// The refusal lines an attack on an NPC gets in single-way combat: the
/// revision-289 `player_in_combat_check` text (`player_combat.rs2:112,116`)
/// and the text with no mark that `player_in_combat_check2` sends
/// (`player_combat.rs2:137,141`) and the revision-377 `player_in_combat_check`
/// sends (its `player_combat.rs2:79,83`). The PvP line has a period and stays
/// in `pvp_line`.
const SINGLE_WAY_NPC_LINES: [&str; 2] = ["I'm already under attack!", "I'm already under attack"];
/// "The duel has not started yet!" lines that end the run.
const DUEL_WAIT_END: u8 = 3;
const POTION_PREP_SHIFT: u32 = 42;
const POTION_SKIP_SHIFT: u32 = 49;
// Linux and macOS measure 552 B, versus 520 B before PvP; Main allows 560 B.
// V1 +8: pvp_source 1, pvp_hold_until 2, oos_retries 1, single_way_lines 1,
// duel_wait_lines 1, flags2 2. V2 +24: rhand_pick 4 to 12 (+8), active 1,
// switch 8, and switches/switch_failed/failed_styles/opp_protect/
// opp_protect_prev/last_switch_mask/reach_ticks 1 each. Compiler field records:
// BATCH-029-INT/r2-pvp-v2-{linux,mac}-layout-1. The ceiling leaves 8 B.
const _: usize = 560 - std::mem::size_of::<Combat>();
const BASELINE_READY: u16 = 1 << api::prayer::PRAYER_COUNT;

impl NativeMachine for Combat {
    type Args = (Arc<CombatRequest>, Arc<CombatTables>, PvpGlobals);
    type Output = CombatReport;
    fn begin(
        (request, tables, globals): Self::Args,
        cx: &mut ActionContext<'_>,
    ) -> Result<Self, ActionError> {
        if !matches!(request.style, Style::Melee | Style::Ranged | Style::Mage) {
            return Err(unavailable("style slice"));
        }
        if request.prayer_mode != PrayerMode::Hold {
            return Err(unavailable("flick slice"));
        }
        let pvp_source = match &request.target {
            Target::Player { .. } => Some(
                select::pvp_initiation_allowed(&request, globals).ok_or_else(|| {
                    api::host_log!(
                        api::hostlog::Category::ScriptLifecycle,
                        api::hostlog::Level::Info,
                        "combat: fight with a player refused ({}): request.pvp = {}, pvp.start_fights = {} (off, or not read yet)",
                        PvpRefusal::NotOptedIn,
                        request.pvp,
                        globals.start_fights
                    );
                    ActionError::PvpRefused(PvpRefusal::NotOptedIn)
                })?,
            ),
            Target::Npc { .. } | Target::Attacker { .. } => None,
        };
        if request.tactic != Tactic::Open || matches!(request.fallback, Fallback::FaceTank { .. }) {
            return Err(unavailable("tactic slice"));
        }
        let expected_revision = cx.revision().as_i32();
        if tables.selected().revision() != expected_revision {
            return Err(unavailable("combat data revision"));
        }
        if let Target::Npc { types, .. } = &request.target {
            if types.is_empty() || types.iter().any(|id| tables.npc(*id).is_none()) {
                return Err(unavailable("combat npc facts"));
            }
            if !types
                .iter()
                .any(|id| tables.npc_fact(*id).is_some_and(|row| row.attackable))
            {
                return Err(unavailable("unattackable npc target"));
            }
        }
        // The first legal declared style: the opening style is settled below,
        // once the machine can read its own capabilities from the begin frame.
        let provisional = switch::opening_style(&request, |_| true)
            .ok_or_else(|| unavailable("combat style capability"))?;
        if (provisional == Style::Mage || request.kits.mage.is_some())
            && (tables.selected().spells().is_empty() || tables.selected().spells().len() > 32)
        {
            return Err(unavailable("combat spell facts"));
        }
        let begin_frame =
            Frame::borrow(cx.snapshot()).map(|frame| frame.with_wilderness(cx.wilderness()));
        if let (Target::Player { name }, Some(frame)) = (&request.target, begin_frame.as_ref()) {
            if let Some(refusal) = select::begin_refusal(name, frame, &tables) {
                return Err(ActionError::PvpRefused(refusal));
            }
        }
        let ticks = if request.budget_ticks == 0 {
            1500
        } else {
            request.budget_ticks
        };
        let local_update = cx
            .snapshot()
            .local_player()
            .and_then(|local| local.value.animation_update)
            .map(|update| update.serial);
        if request.kits.mage.is_some() {
            // A bad spell order fails here, not at a later switch.
            Self::validate_spell_order(&request, &tables)?;
        }
        // An inline placeholder (no allocation) until the opening style is known.
        let style_state = Self::new_style_state(&request, &tables, Style::Melee, None, -1)?;
        let mut machine = Self {
            request,
            tables,
            identity: 0,
            identity_kind: EngagementKind::None,
            deadline: cx
                .active_now()
                .saturating_add(Duration::from_millis(u64::from(ticks) * 600)),
            pending_walk: None,
            threats: ThreatSet::default(),
            schedule: Schedule::default(),
            sweep: PrayerSweep::new(),
            counters: Counters::default(),
            style_state,
            sequence: 0,
            rhand_pick: [-1; 3],
            suspended_type: -1,
            chat_since: -1,
            pvp_source,
            pvp_hold_until: 0,
            oos_retries: 0,
            single_way_lines: 0,
            single_way_until: 0,
            hit_index: u16::MAX,
            hit_health: 0,
            duel_wait_lines: 0,
            flags2: 0,
            active: provisional,
            switch: SwitchState::Idle,
            switches: 0,
            switch_failed: 0,
            failed_styles: 0,
            opp_protect: 0,
            opp_protect_prev: 0,
            last_switch_mask: 0,
            reach_ticks: 0,
            engaged: None,
            suspended: None,
            end: None,
            antifire_sip: 0,
            lost_since: 0,
            escape_since: 0,
            last_tick: 0,
            emitted_tick: 0,
            plan: TickPlan::default(),
            plan_first_id: 0,
            pending: [PendingRow::default(); 5],
            staged_pending: 0,
            prayer_ready: [0; 15],
            prayer_ready_mask: 0,
            baseline_on: 0,
            raised_prayers: RaisedPrayers::default(),
            eat_ready: 0,
            food_total: 0,

            prep_failures: 0,
            approach_recovery: ReturnStallRecovery::with_max_attempts(QUEST_WALK_MAX_ATTEMPTS),
            approach_walk_pending: false,
            phase: Phase::Prep,
            flags: 0,
            stand_walk_failures: 0,
        };
        if let Some(frame) = begin_frame.as_ref() {
            if let Some(open) =
                switch::opening_style(&machine.request, |style| machine.style_usable(style, frame))
            {
                machine.active = open;
            }
        }
        machine.style_state = Self::new_style_state(
            &machine.request,
            &machine.tables,
            machine.active,
            local_update,
            -1,
        )?;

        // Request facts can warrant a boost before the first complete actor
        // frame is available to resolve engagement worth.
        if machine.request_worth() {
            machine.flags |= BOOST_WORTH;
        }
        if let Some(lines) = cx.snapshot().chat_lines(-1) {
            machine.chat_since = lines
                .value
                .iter()
                .map(|row| row.sequence)
                .max()
                .unwrap_or(-1);
        }
        if let Some(frame) = begin_frame {
            machine.capture_prayer_baseline(&prayer_observation(&frame));
            let picked = select::pick_target(
                &machine.request,
                machine.active,
                &frame,
                &machine.threats,
                &machine.tables,
                cx.run().slot ^ cx.run().run,
            );
            if let Some(actor) = picked {
                machine.engage(actor, &frame);
            }
            machine.pick_weapons(&frame);
            if machine.active == Style::Mage {
                machine.resolve_magic_mode(&frame);
            }
            machine.resolve_worth(&frame);
        }
        Ok(machine)
    }
    fn poll(&mut self, cx: &mut ActionContext<'_>) -> Poll<Result<CombatReport, ActionError>> {
        if self.pending_walk.is_some_and(|id| {
            cx.walk_receipt(id.get())
                .is_some_and(|receipt| receipt.end == WalkEnd::UserInput)
        }) {
            return Poll::Ready(Err(ActionError::UserInput));
        }
        let snapshot = cx.snapshot();
        let Some(frame) =
            Frame::borrow(snapshot).map(|frame| frame.with_wilderness(cx.wilderness()))
        else {
            return Poll::Pending;
        };
        let Some(chat) = snapshot.chat_lines(self.chat_since) else {
            return Poll::Pending;
        };
        let evidence = cx.evidence();
        if self.flags & OBSERVED != 0
            && (self.sequence == evidence.sequence || self.last_tick == evidence.tick as u16)
        {
            return Poll::Pending;
        }
        // Dispatch happened against the preceding poll's snapshot. Do not
        // overwrite its plan until every receipt is available.
        if !self.receipts(cx) {
            return Poll::Pending;
        }
        // begin may have no frame; choose the weapon on the first complete
        // observation so Prep knows the wear request before it can drink.
        if self.flags & OBSERVED == 0 && self.rhand_pick[self.active.index()] < 0 {
            self.pick_weapons(&frame);
        }
        self.capture_prayer_baseline(&prayer_observation(&frame));
        let tick = evidence.tick as u16;
        if self.flags2 & PVP_HOLD != 0 && reached(tick, self.pvp_hold_until) {
            self.flags2 &= !PVP_HOLD;
        }
        self.advance_prayer_ready(tick);
        let gap = self.flags & OBSERVED == 0 || elapsed(tick, self.last_tick) > 1;
        self.sequence = evidence.sequence;
        self.last_tick = tick;
        if gap {
            self.advance_eat_ready(tick);
            self.schedule.interaction = Interaction::Unknown;
            self.schedule.cycle.known = false;
        }
        self.flags |= OBSERVED;
        let events = self.threats.observe(&frame, &self.tables, tick);
        self.counters.damage = self.counters.damage.saturating_add(events.damage);
        self.counters.protected = self
            .counters
            .protected
            .saturating_add(events.positive_protected);
        self.counters.ticks = self.counters.ticks.saturating_add(1);
        let stand_failures_before_receipt = self.stand_walk_failures;
        if self.phase != Phase::Escape {
            if let Some(receipt) = self.pending_walk.and_then(|id| cx.walk_receipt(id.get())) {
                let approach_walk = self.approach_walk_pending();
                let stand_walk = approach_walk
                    && self.engaged.is_none()
                    && matches!(self.phase, Phase::Prep | Phase::Engage)
                    && self.request.stand.is_some();
                let stand_arrived = self
                    .request
                    .stand
                    .is_some_and(|stand| select::stand_reached(stand, frame.local));
                self.pending_walk = None;
                self.set_approach_walk_pending(false);
                if approach_walk {
                    // Route end can precede the rendered tile update. Accept
                    // the network tile, but never turn a refusal into arrival.
                    let retryable = crate::native::WalkRetryState::retryable_receipt(receipt);
                    let arrived = matches!(&receipt.end, WalkEnd::Arrived)
                        || (matches!(&receipt.end, WalkEnd::RouteEnded)
                            && retryable
                            && stand_arrived);
                    if arrived {
                        self.reset_approach_recovery();
                        if stand_walk || stand_arrived {
                            self.stand_walk_failures = 0;
                        }
                    } else {
                        if stand_walk && retryable {
                            self.stand_walk_failures = self.stand_walk_failures.saturating_add(1);
                        }
                        let stand_exhausted =
                            stand_walk && self.stand_walk_failures >= MAX_STAND_WALK_FAILURES;
                        if !stand_exhausted && self.observe_approach_failure(receipt, evidence.tick)
                        {
                            // The observation guard above prevents another
                            // plan until a later game tick. Keep the count so
                            // a kill-budget end cannot buy more stand walks.
                            return Poll::Pending;
                        }
                        let error = match &receipt.end {
                            WalkEnd::NeedsEvidence(gates) => {
                                ActionError::NeedsEvidence(Arc::clone(gates))
                            }
                            WalkEnd::Cancelled => ActionError::Cancelled,
                            WalkEnd::UserInput => ActionError::UserInput,
                            _ => ActionError::Blocked(receipt.failure_detail_or_fallback()),
                        };
                        return Poll::Ready(Err(error));
                    }
                } else if stand_arrived {
                    self.stand_walk_failures = 0;
                }
            }
        }
        self.observe_magic(&frame, tick);
        self.settle(&frame, tick);
        let died = chat
            .value
            .iter()
            .any(|row| row.text.contains("Oh dear, you are dead"));
        if let Some(sequence) = chat.value.iter().map(|row| row.sequence).max() {
            self.chat_since = sequence;
        }
        if self.phase != Phase::WindDown {
            if died || arbiter::stat(&frame, 3).0 <= 0 {
                self.finish(CombatEnd::Died, tick);
            } else {
                self.observe_target(&frame, tick, false);
                if self.active == Style::Mage {
                    for line in chat.value.iter() {
                        self.magic_refusal(&line.text);
                    }
                }
            }
            if self.phase != Phase::WindDown && cx.active_now() >= self.deadline {
                self.finish(CombatEnd::Budget, tick);
            }
            if self.phase != Phase::WindDown
                && self.request.until_ticks != 0
                && self.counters.ticks >= self.request.until_ticks
            {
                self.finish(CombatEnd::Budget, tick);
            }
            if self.phase != Phase::WindDown
                && matches!(self.request.target, Target::Attacker { .. })
                && select::pick_target(
                    &self.request,
                    self.active,
                    &frame,
                    &self.threats,
                    &self.tables,
                    evidence.tick,
                )
                .is_none()
                && select::has_unattackable_allowed(&self.request, &frame, &self.threats)
            {
                self.finish(CombatEnd::Aborted(AbortReason::Unattackable), tick);
            }
            if self.phase != Phase::WindDown {
                let failure = if self.flags & FAILED_WEAPON != 0 {
                    Some(AbortReason::PrepFailed(weapon_prep_item(self.active)))
                } else if self.flags & FAILED_SHIELD != 0 {
                    Some(AbortReason::PrepFailed(PrepItem::Shield))
                } else if self.flags & FAILED_ATTACK != 0 {
                    Some(AbortReason::Unresponsive)
                } else {
                    None
                };
                if let Some(reason) = failure {
                    self.finish(CombatEnd::Aborted(reason), tick);
                }
            }
            // The request-wide deadline is checked above; this narrower wait
            // consumes only fresh observations, even while style work is locked.
            if self.phase == Phase::Prep && self.active == Style::Ranged {
                if let Some(reason) = self.ranged_readiness_abort(&frame, None) {
                    self.finish(CombatEnd::Aborted(AbortReason::PrepReadiness(reason)), tick);
                }
            }
        }
        if self.phase != Phase::WindDown {
            self.switch_observe(&frame, tick);
        }
        if self.player_ident().is_some() {
            if self.engaged.is_some_and(|actor| {
                frame
                    .in_combat
                    .target
                    .is_some_and(|target| actor.matches(target))
            }) {
                self.single_way_lines = 0;
            }
            for line in chat.value.iter() {
                self.pvp_line(&frame, tick, &line.text);
            }
        } else if self
            .engaged
            .is_some_and(|actor| actor.kind == ActorKind::Npc)
        {
            // The run of refusals ends only on a confirmed attack: a hit that
            // landed on the engaged NPC. The player's target is not that proof;
            // a refused click leaves the target on the refused NPC.
            if self.npc_hit_landed(&frame) {
                self.single_way_lines = 0;
                self.flags2 &= !SW_WINDOW;
            }
            self.single_way_window_close(tick);
            if chat
                .value
                .iter()
                .any(|line| SINGLE_WAY_NPC_LINES.contains(&line.text.as_str()))
            {
                self.npc_single_way_refusal(&frame, tick);
            }
        } else {
            self.hit_index = u16::MAX;
        }
        if self.phase == Phase::Escape {
            if let Fallback::Retreat { tile } = self.request.fallback {
                if distance(frame.here, tile) <= 1 {
                    self.finish(CombatEnd::Aborted(AbortReason::Retreated), tick);
                }
            }
        }
        // Onsets/settles/ends above commit even when this tick must be silent.
        if self.schedule.locked(tick) {
            let danger = self.threats.danger(
                &frame,
                &self.tables,
                tick,
                self.shield(&frame),
                self.antifire(tick),
            );
            let poison_fire = crate::survival::math::poison_fire(
                cx.condition(),
                &self.threats,
                &self.tables,
                tick,
            );
            let lines = select::lines(danger, arbiter::stat(&frame, 3).1).with_poison(poison_fire);
            let food_available = self.emergency_food_available(&frame, danger);
            self.safety_end(&frame, tick, danger, lines.emergency, food_available);
            self.counters.locked = self.counters.locked.saturating_add(1);
            return Poll::Pending;
        }
        self.schedule.unlock_observed(tick);
        self.plan = TickPlan::default();
        match self.plan_tick(&frame, tick, cx) {
            Ok(plan) => {
                self.plan = plan;
                if plan.len == 0 {
                    if self.phase == Phase::WindDown
                        && self.baseline_on & BASELINE_READY != 0
                        && self.sweep.pending_mask() == 0
                        && !self.pending_row(RowKind::Pickup)
                        && (self.active != Style::Ranged || self.ranged().sweep_done)
                        && !(self.flags & TERMINAL_FOOD != 0 && self.pending_row(RowKind::Eat))
                        && self
                            .sweep
                            .candidates(self.tables.selected(), &prayer_observation(&frame))
                            .find(|click| self.raised_prayers.contains(click.varp))
                            .is_none()
                    {
                        if self.sweep.report().timed_out != 0 {
                            return Poll::Ready(Err(ActionError::Failed(
                                "prayer cleanup did not settle".into(),
                            )));
                        }
                        if self.end == Some(CombatEnd::TargetGone)
                            && self.flags & LOST != 0
                            && self.engaged.is_some()
                            && self.flags & LEASH_WALKED == 0
                        {
                            if let Some(destination) = self.leash_cancel_tile(&frame) {
                                std::task::ready!(crate::native::defer_budget(self.walk(
                                    destination,
                                    tick,
                                    cx,
                                    false,
                                )))?;
                                self.flags |= LEASH_WALKED;
                                return Poll::Pending;
                            }
                        }
                        // A completed handle drops this machine before
                        // the quest step can inspect it. If this poll
                        // changed the stand count, keep the machine alive
                        // for one more poll so the step mirrors the exact
                        // count before it seeds the next attempt.
                        if self.stand_walk_failures != stand_failures_before_receipt {
                            return Poll::Pending;
                        }
                        return Poll::Ready(Ok(self.report(evidence)));
                    }
                    return Poll::Pending;
                }
                match self.emit(&frame, tick, cx) {
                    Ok(())
                    | Err(ActionError::BudgetExhausted)
                    | Err(ActionError::Unavailable(_)) => Poll::Pending,
                    Err(error) => Poll::Ready(Err(error)),
                }
            }
            Err(error) => crate::native::defer_budget(Err(error)),
        }
    }
    fn cancel(&mut self) {
        self.schedule.terminal();
        self.pending_walk = None;
    }
}

impl Combat {
    pub fn input_lock(&self) -> Option<u16> {
        let schedule = self.projected_schedule();
        schedule
            .locked(self.last_tick)
            .then_some(schedule.input_lock)
    }
    pub fn interaction(&self) -> Interaction {
        self.projected_schedule().interaction
    }
    pub fn cycle(&self) -> super::schedule::Cycle {
        self.projected_schedule().cycle
    }
    pub fn eat_ready(&self) -> u16 {
        self.eat_ready
    }
    pub fn engaged(&self) -> Option<ActorRef> {
        self.engaged
    }
    /// Stand-approach walk failures this attempt inherited and spent. The quest
    /// step mirrors this into the run that outlives one attempt.
    pub(crate) fn stand_walk_failures(&self) -> u8 {
        self.stand_walk_failures
    }
    /// Continue a quest step's spent stand approaches: the step owns the count,
    /// so the attempt it starts keeps counting toward the same bound. Every
    /// other caller begins at zero.
    pub(crate) fn seed_stand_walk_failures(&mut self, failures: u8) {
        self.stand_walk_failures = failures;
    }
    /// Observed resource consumption counts splashes as casts, never as hits.
    pub fn casts(&self) -> u16 {
        self.counters.casts
    }
    pub fn selected_spell(&self) -> Option<u8> {
        match &self.style_state {
            StyleState::Magic(state) => state.selected,
            _ => None,
        }
    }
    pub fn last_launched_spell(&self) -> Option<u8> {
        match &self.style_state {
            StyleState::Magic(state) => state.last_launched,
            _ => None,
        }
    }
    fn engaged_type(&self) -> i32 {
        match self.identity_kind {
            EngagementKind::Npc => self.identity,
            _ => -1,
        }
    }
    fn player_ident(&self) -> Option<i32> {
        match self.identity_kind {
            EngagementKind::Player => Some(self.identity),
            _ => None,
        }
    }
    fn magic(&self) -> &MagicState {
        let StyleState::Magic(state) = &self.style_state else {
            unreachable!("magic mechanics require the magic style");
        };
        state
    }
    fn magic_mut(&mut self) -> &mut MagicState {
        let StyleState::Magic(state) = &mut self.style_state else {
            unreachable!("magic mechanics require the magic style");
        };
        state
    }
    fn finish(&mut self, end: CombatEnd, _tick: u16) {
        if self.end.is_some() {
            return;
        }
        self.end = Some(end);
        self.phase = Phase::WindDown;
        self.schedule.terminal();
        self.sweep = PrayerSweep::new();
    }
    fn report(&self, evidence: api::quest_progress::EvidenceStamp) -> CombatReport {
        CombatReport {
            end: self.end.expect("terminal phase has a latched end"),
            evidence,
            engaged: self.engaged,
            engaged_npc_type: self.engaged_type(),
            ticks: self.counters.ticks,
            swings: self.counters.swings,
            casts: self.counters.casts,
            damage_taken: self.counters.damage,
            food: self.counters.food,
            prayer_doses: self.counters.prayer,
            boost_doses: self.counters.boost,
            antifire_doses: self.counters.antifire,
            hits_while_protected: self.counters.protected,
            protect_switches: self.counters.switches,
            intruders: self.counters.intruders,
            ammo_pickups: self.counters.ammo,
            restorations: self.counters.restorations,
            locked_ticks: self.counters.locked,
            multi_op_plans: self.counters.multi,
            melee_mode_fallback: match &self.style_state {
                StyleState::Melee(state) => state.mode_fallback,
                _ => None,
            },
            flick_resets: 0,
            flick_misses: 0,
            flick_fallback: false,
            pvp_blocked: self.counters.pvp_blocked,
            pvp_source: self.pvp_source,
            kill_line_seen: self.flags2 & PVP_KILL_LINE != 0,
            switches: self.switches,
            switch_failed: self.switch_failed,
            active_style: self.active,
        }
    }
    fn clear_pending_cast(&mut self) {
        for pending in &mut self.pending {
            if pending.row.kind == RowKind::Cast {
                *pending = PendingRow::default();
            }
        }
        self.schedule.settle(OpKind::Cast);
    }
    fn deselect(&mut self, reject: bool) {
        if reject {
            if let Some(index) = self.magic().selected {
                self.magic_mut().rejected |= 1 << index;
            }
        }
        {
            let state = self.magic_mut();
            state.selected = None;
            state.armed = false;
            state.arm = None;
            state.rune_seen = false;
        }
        self.clear_pending_cast();
    }
    /// The engaged player's row on this frame, matched by slot and name hash.
    fn engaged_player<'f>(&self, frame: &Frame<'f>) -> Option<&'f api::snapshot::PlayerView> {
        let actor = self
            .engaged
            .filter(|actor| actor.kind == ActorKind::Player)?;
        frame.players.iter().find(|row| {
            row.index == usize::from(actor.index)
                && self.player_ident().is_some()
                && row.actor.name.as_deref().map(select::ident::fnv1a) == self.player_ident()
        })
    }
    fn pvp_abort(&mut self, reason: PvpRefusal, tick: u16) {
        self.finish(CombatEnd::Aborted(AbortReason::PvpRefused(reason)), tick);
    }
    /// The level-range refusal with the values the machine computes itself.
    fn level_refusal(&self, frame: &Frame<'_>) -> PvpRefusal {
        let (wilderness, difference) = self.engaged_player(frame).map_or((0, 0), |row| {
            let level = |tile| frame.wilderness.map_or(0, |rules| rules.level(tile));
            (
                level(frame.here).min(level(row.actor.tile)),
                frame.local.player.combat_level.abs_diff(row.combat_level),
            )
        });
        PvpRefusal::LevelRange {
            wilderness: u8::try_from(wilderness).unwrap_or(u8::MAX),
            difference: u8::try_from(difference).unwrap_or(u8::MAX),
        }
    }
    /// One server line while a player is engaged (PVP-DESIGN-1026 §2.4): every
    /// `mes` of `pvp_combat.rs2` and `duel_arena.rs2` an attack can produce has
    /// a reaction here, matched byte for byte, never by substring.
    fn pvp_line(&mut self, frame: &Frame<'_>, tick: u16, line: &str) {
        match line {
            "You can't attack players who aren't in the wilderness." => {
                self.pvp_abort(PvpRefusal::OpponentOutside, tick);
            }
            "Your level difference is too great!"
            | "You need to move deeper into the wilderness." => {
                let refusal = self.level_refusal(frame);
                self.pvp_abort(refusal, tick);
            }
            "Saradomin won't let you attack your allies!"
            | "Zamorak wants you to kill your enemies!"
            | "Kolodion allows only magical combat within the arena." => {
                self.oos_retries = self.oos_retries.saturating_add(1);
                if self.oos_retries >= OOS_RETRY {
                    self.finish(CombatEnd::Aborted(AbortReason::Unattackable), tick);
                }
            }
            "I'm already under attack." => self.hold_attack(tick, SINGLE_WAY_WINDOW),
            "Someone else is already fighting your opponent." => {
                self.hold_attack(tick, SINGLE_WAY_WINDOW);
                self.single_way_lines = self.single_way_lines.saturating_add(1);
                if self.single_way_lines >= SINGLE_WAY_END {
                    self.finish(CombatEnd::TargetGone, tick);
                }
            }
            "You cannot attack players in the challenge area." => {
                self.pvp_abort(PvpRefusal::ChallengeArea, tick);
            }
            "You can only attack your opponent!" => {
                self.pvp_abort(PvpRefusal::NotOpponent, tick);
            }
            "The duel has not started yet!" => {
                self.duel_wait_lines = self.duel_wait_lines.saturating_add(1);
                if self.duel_wait_lines >= DUEL_WAIT_END {
                    self.pvp_abort(PvpRefusal::DuelNotStarted, tick);
                } else {
                    self.hold_attack(tick, 1);
                }
            }
            other => {
                if self
                    .engaged_player(frame)
                    .and_then(|row| row.actor.name.as_deref())
                    .is_some_and(|name| select::is_kill_line(other, name))
                {
                    self.flags2 |= PVP_KILL_LINE;
                }
            }
        }
    }
    /// Keep the attack row out of the plans for `ticks` ticks from `tick`.
    fn hold_attack(&mut self, tick: u16, ticks: u16) {
        self.pvp_hold_until = tick.wrapping_add(ticks);
        self.flags2 |= PVP_HOLD;
    }
    /// A hit landed on the engaged NPC since the previous poll, while the bot
    /// targets it. The client applies a hit (including a 0) to an NPC with
    /// `combat_cycle = loop_cycle + 400` and the new `health` bytes
    /// (`vendor/fr-client-rust/crates/client/src/client/client.rs:10300-10303`,
    /// `:10371-10374`), so the NPC's health-bar window opening (`in_combat`,
    /// `api/src/snapshot/decode.rs:1509`) or its health dropping is that hit.
    /// A hit while the window is already open and the health is unchanged (a
    /// miss) shows neither; the next real hit does.
    fn npc_hit_landed(&mut self, frame: &Frame<'_>) -> bool {
        let Some(actor) = self.engaged.filter(|actor| actor.kind == ActorKind::Npc) else {
            self.hit_index = u16::MAX;
            return false;
        };
        let Some(row) = frame
            .npcs
            .iter()
            .find(|row| row.index == usize::from(actor.index))
        else {
            self.hit_index = u16::MAX;
            return false;
        };
        let health = u8::try_from(row.health.max(0)).unwrap_or(u8::MAX);
        let seen = self.hit_index == actor.index;
        let was_open = self.flags2 & SW_COMBAT != 0;
        let was_health = self.hit_health;
        self.hit_index = actor.index;
        self.hit_health = health;
        if row.in_combat {
            self.flags2 |= SW_COMBAT;
        } else {
            self.flags2 &= !SW_COMBAT;
        }
        seen && frame
            .in_combat
            .target
            .is_some_and(|target| actor.matches(target))
            && ((row.in_combat && !was_open) || health < was_health)
    }
    /// An attack on an NPC was refused by single-way combat. The content lets
    /// the bot attack only the NPC that last attacked it while that attack is
    /// less than 8 ticks old (`player_combat.rs2:115`; `npc_combat.rs2:367-370`
    /// sets it on every NPC attack), and that NPC changes from tick to tick
    /// when several NPCs hit the bot. So the reaction follows the content:
    /// attack an NPC that is hitting the bot and that the request may fight,
    /// or hold the attack for the window and send it again.
    ///
    /// The budget measures the engine's window, not raw refusals. The first
    /// refusal opens a window of `SINGLE_WAY_WINDOW` observed ticks and counts
    /// once; refusals inside it retarget but do not count. The fight ends
    /// (`single_way_window_close`) when the third window with a refusal closes
    /// and no hit landed on the engaged NPC in between. Refusals that arrive
    /// while a hold is running belong to the attack the hold already answers.
    fn npc_single_way_refusal(&mut self, frame: &Frame<'_>, tick: u16) {
        // Only a fight that sends attacks reacts: a refusal read after the
        // fight ended or while it prepares belongs to no attack of this run.
        if !matches!(self.phase, Phase::Engage | Phase::Fight) || self.flags2 & PVP_HOLD != 0 {
            return;
        }
        if self.flags2 & SW_WINDOW == 0 {
            self.single_way_lines = self.single_way_lines.saturating_add(1);
            self.single_way_until = tick.wrapping_add(SINGLE_WAY_WINDOW);
            self.flags2 |= SW_WINDOW;
        }
        let attacker = select::single_way_attacker(
            &self.request,
            self.switch.pending().unwrap_or(self.active),
            frame,
            &self.threats,
            self.engaged,
        );
        if let Some(actor) = attacker {
            self.engage(actor, frame);
            // The refused click began no attack cycle, and the new target is a
            // fresh engagement: its attack goes out now under the Engage rules.
            self.schedule.refused(OpKind::Attack);
            self.phase = Phase::Engage;
            self.flags &= !ENGAGEMENT_ATTACK;
        } else {
            // Hold to the end of the window, never past it.
            self.pvp_hold_until = self.single_way_until;
            self.flags2 |= PVP_HOLD;
        }
    }
    /// Close the refusal window once its ticks have passed. The fight ends as
    /// `SingleWayBlocked` when this was the third window in a row with a
    /// refusal and no landed hit: about three windows (24 ticks) without
    /// progress. A landed hit clears the window and the count first.
    fn single_way_window_close(&mut self, tick: u16) {
        if self.flags2 & SW_WINDOW == 0 || !reached(tick, self.single_way_until) {
            return;
        }
        self.flags2 &= !SW_WINDOW;
        if self.single_way_lines >= SINGLE_WAY_END {
            self.finish(CombatEnd::Aborted(AbortReason::SingleWayBlocked), tick);
        }
    }
    fn magic_refusal(&mut self, line: &str) {
        let Some(index) = self.magic().selected else {
            return;
        };
        let spell = &self.tables.selected().spells()[usize::from(index)];
        match magic::refusal(line, spell) {
            Some(Refusal::Spell) => self.deselect(true),
            Some(Refusal::Runes) => self.deselect(false),
            None => {}
        }
    }
    fn resolve_magic_mode(&mut self, frame: &Frame<'_>) {
        if self.magic().mode.is_none() {
            let rhand = self.active_desired(3, frame).or_else(|| {
                frame
                    .equipment
                    .iter()
                    .find(|row| row.slot == 3)
                    .map(|row| row.def.id)
            });
            self.magic_mut().mode = Some(magic::resolve(&self.request, &self.tables, rhand));
        }
    }
    fn observe_magic(&mut self, frame: &Frame<'_>, tick: u16) {
        if self.active != Style::Mage {
            return;
        }
        let Some(index) = self.magic().selected else {
            return;
        };
        let spell = &self.tables.selected().spells()[usize::from(index)];
        let runes = magic::rune_count(spell, frame, &self.tables);
        let spent = runes
            .is_some_and(|(count, _)| self.magic().rune_seen && count < self.magic().rune_base);
        // Rune consumption is the cast acknowledgement even when the server
        // sends a failedspell_impact without a target damage mask.
        let accepted_cast = if self.magic().mode == Some(CastMode::Manual) {
            self.pending_id(RowKind::Cast, i32::from(index))
        } else {
            self.magic().armed && self.flags & ENGAGEMENT_ATTACK != 0
        };
        if spent && accepted_cast {
            self.magic_mut().last_launched = Some(index);
            self.counters.casts = self.counters.casts.saturating_add(1);
            self.flags2 |= PVP_SWING;
            if self.magic().mode == Some(CastMode::Autocast) {
                self.schedule.observe_swing(tick, magic::CAST_TICKS as u8);
            } else {
                self.schedule.interaction = Interaction::Installed;
            }
            if self.magic().mode == Some(CastMode::Manual) {
                self.magic_mut().cursor = self.magic().cursor.wrapping_add(1);
                self.clear_pending_cast();
            }
        }
        self.magic_mut().rune_base = runes.map_or(0, |(count, _)| count);
        self.magic_mut().rune_seen = runes.is_some();
    }
    fn choose_spell(&mut self, frame: &Frame<'_>) -> bool {
        let mode = self.magic().mode.expect("magic mode resolved");
        let rejected = self.magic().rejected;
        let selected = if self.request.spells.is_some() {
            let order_len = usize::from(self.magic().spell_order_len);
            let cursor = usize::from(self.magic().cursor);
            let mut selected = None;
            for offset in 0..order_len {
                let order_cursor = (cursor + offset) % order_len;
                let index = self.magic().spell_order[order_cursor];
                let spell = &self.tables.selected().spells()[usize::from(index)];
                if rejected & (1 << index) == 0
                    && magic::castable(spell, frame, &self.tables, self.engaged)
                {
                    self.magic_mut().cursor = order_cursor as u8;
                    selected = Some(index);
                    break;
                }
            }
            selected.or_else(|| {
                self.request
                    .fallback_spells
                    .then(|| {
                        magic::strongest(
                            frame,
                            &self.tables,
                            self.engaged,
                            CastMode::Manual,
                            rejected,
                        )
                    })
                    .flatten()
            })
        } else {
            magic::strongest(frame, &self.tables, self.engaged, mode, rejected)
        };
        if selected != self.magic().selected {
            self.deselect(false);
            self.magic_mut().selected = selected;
            if let Some(index) = selected {
                let spell = &self.tables.selected().spells()[usize::from(index)];
                let runes = magic::rune_count(spell, frame, &self.tables);
                self.magic_mut().rune_base = runes.map_or(0, |(count, _)| count);
                self.magic_mut().rune_seen = runes.is_some();
            }
        }
        selected.is_some()
    }
    /// Returns true while magic preparation owns this tick's style work.
    fn magic_prepare(
        &mut self,
        plan: &mut TickPlan,
        frame: &Frame<'_>,
        tick: u16,
        cx: &ActionContext<'_>,
    ) -> bool {
        if self.pending_row(RowKind::Wear)
            || plan.contains(RowKind::Wear)
            || self.pending_row(RowKind::Arm)
        {
            return true;
        }
        self.resolve_magic_mode(frame);
        if self.pending_row(RowKind::Cast) {
            return self.magic().mode == Some(CastMode::Manual);
        }
        if !self.choose_spell(frame) {
            let ordered_mask = self.magic().ordered_mask;
            let rejected = self.magic().rejected;
            let mode = self.magic().mode.expect("magic mode resolved");
            let rune_exhausted =
                self.tables
                    .selected()
                    .spells()
                    .iter()
                    .enumerate()
                    .any(|(index, spell)| {
                        let ordered = self.request.spells.is_none()
                            || self.request.fallback_spells
                            || ordered_mask & (1u32 << index) != 0;
                        ordered
                            && rejected & (1u32 << index) == 0
                            && (mode == CastMode::Manual || spell.autocast_selectable)
                            && magic::eligible(spell, frame, &self.tables, self.engaged)
                    });
            if rune_exhausted && self.exhaustion_deferred(frame) {
                // A settle holds the exhaustion for its first `Idle` tick,
                // where T1 switches (design §4.4).
                return true;
            }
            let reason = if rune_exhausted {
                AbortReason::Unprotected(Unprotected::NoRunes)
            } else if mode == CastMode::Manual {
                AbortReason::Unresponsive
            } else {
                AbortReason::PrepFailed(PrepItem::Arm)
            };
            self.finish(CombatEnd::Aborted(reason), tick);
            return true;
        }
        if self.magic().mode == Some(CastMode::Manual) {
            return false;
        }
        let Some(controls) = self.tables.selected().autocast_controls() else {
            self.finish(
                CombatEnd::Aborted(AbortReason::PrepFailed(PrepItem::Arm)),
                tick,
            );
            return true;
        };
        let Some(side) = cx.snapshot().active_side_tab() else {
            return true;
        };
        let Some(root) = frame.combat_tab else {
            return true;
        };
        let Some(value) = frame
            .varps
            .iter()
            .find(|row| row.index == controls.magic_varp)
        else {
            return true;
        };
        // Borrow only the style field: arming also reads the shared tables.
        let StyleState::Magic(state) = &mut self.style_state else {
            unreachable!("magic preparation requires magic state")
        };
        if !state.initial_arm_checked {
            state.initial_arm_checked = true;
            let spell = &self.tables.selected().spells()
                [usize::from(state.selected.expect("selected spell"))];
            if value.value == controls.armed_value
                && magic::observed_autocast_spell_matches(
                    cx.snapshot(),
                    controls,
                    side.value,
                    root,
                    spell,
                )
            {
                state.arm = None;
                state.arm_since = 0;
                state.arm_wait = 0;
                state.armed = true;
                return false;
            }
        }
        if state.armed && value.value == controls.armed_value {
            return false;
        }
        state.armed = false;
        if plan.len != 0 {
            return true;
        }
        if state.arm.is_none() {
            let spell = &self.tables.selected().spells()
                [usize::from(state.selected.expect("selected spell"))];
            let component = self.tables.selected().spell_button_com(&spell.name);
            state.arm = Some(Arm::new(component));
            state.arm_wait = 0;
        }
        let obs = ArmObservation {
            ingame: true,
            active_side_tab: side.value,
            combat_tab_root: root,
            magic_varp_value: value.value,
        };
        let timeout =
            state.arm_wait != 0 && elapsed(tick, state.arm_since) >= u16::from(state.arm_wait);
        let step = state
            .arm
            .as_mut()
            .expect("active arm")
            .poll(obs, controls, timeout);
        match step {
            ArmStep::Emit(request, ms) => {
                let wait = ms.div_ceil(600) as u8;
                let row = match request {
                    InteractReq::SideTab { .. } => {
                        PlanRow::new(RowKind::Arm, 0, wait | arbiter::ARM_SIDE_TAB_FLAG)
                    }
                    InteractReq::IfButton { component_id } => {
                        PlanRow::new(RowKind::Arm, component_id, wait)
                    }
                    _ => unreachable!("shared arm emits controls only"),
                };
                self.push(plan, row);
                true
            }
            ArmStep::Wait => true,
            ArmStep::Done(Ok(())) => {
                state.arm = None;
                state.armed = true;
                false
            }
            ArmStep::Done(Err(failure)) => {
                let item = if matches!(failure, super::arm::ArmFailure::StaffMissing) {
                    PrepItem::Staff
                } else {
                    PrepItem::Arm
                };
                self.finish(CombatEnd::Aborted(AbortReason::PrepFailed(item)), tick);
                true
            }
        }
    }
    /// A manual cast is an attack too: it waits out the PvP hold (a single-way
    /// or duel-start line) exactly as an attack row does.
    fn manual_cast(&self, plan: &mut TickPlan, _frame: &Frame<'_>, tick: u16) {
        if !plan.closed(&self.tables)
            && self.engaged.is_some()
            && self.flags2 & PVP_HOLD == 0
            && !self.pending_row(RowKind::Cast)
            && self.schedule.ready(OpKind::Cast, tick)
            && (!self.schedule.cycle.known || reached(tick, self.schedule.cycle.deadline))
        {
            if let Some(index) = self.magic().selected {
                self.push(plan, PlanRow::new(RowKind::Cast, i32::from(index), 0));
            }
        }
    }
    fn rate(&self, frame: &Frame<'_>) -> u8 {
        if self.active == Style::Mage {
            return magic::CAST_TICKS as u8;
        }
        let base = frame
            .equipment
            .iter()
            .find(|row| row.slot == 3)
            .and_then(|row| self.tables.weapon_style(row.def.id))
            .map_or(4, |row| row.attackrate.max(1));
        self.style_rate(self.active, base)
    }
    /// The attack interval of a weapon with `base` rate when used for `style`.
    fn style_rate(&self, style: Style, base: u8) -> u8 {
        if style == Style::Ranged {
            ranged::rate(base, self.request.ranged_style)
        } else {
            base.max(1)
        }
    }
    /// Engage `actor`. A player is engaged only when the one candidate check
    /// (`select::player_candidate`: the attack label and the zone gate) passes
    /// on this frame; otherwise this is a no-op that counts `pvp_blocked`
    /// and leaves the row a threat. It never reads the opt-in.
    fn engage(&mut self, actor: ActorRef, frame: &Frame<'_>) -> bool {
        if actor.kind == ActorKind::Player {
            let Some(row) = frame
                .players
                .iter()
                .find(|row| row.index == usize::from(actor.index))
            else {
                return false;
            };
            if select::player_candidate(row, frame, &self.tables).is_err() {
                self.counters.pvp_blocked = self.counters.pvp_blocked.saturating_add(1);
                return false;
            }
        }
        if self.engaged != Some(actor) {
            self.reset_target_attack();
        }
        self.engaged = Some(actor);
        self.flags &= !LOST;
        let identity = match actor.kind {
            ActorKind::Npc => frame
                .npcs
                .iter()
                .find(|row| row.index == usize::from(actor.index))
                .and_then(|row| row.r#type)
                .map(|id| (EngagementKind::Npc, id as i32)),
            ActorKind::Player => frame
                .players
                .iter()
                .find(|row| row.index == usize::from(actor.index))
                .and_then(|row| row.actor.name.as_deref())
                .map(|name| (EngagementKind::Player, select::ident::fnv1a(name))),
        };
        (self.identity_kind, self.identity) = identity.unwrap_or((EngagementKind::None, 0));
        self.schedule.interaction = Interaction::Unknown;
        self.resolve_worth(frame);
        true
    }
    fn reset_target_attack(&mut self) {
        if let StyleState::Magic(state) = &mut self.style_state {
            state.reset_for_engagement();
        }
        for pending in &mut self.pending {
            if pending.row.kind == RowKind::Attack {
                *pending = PendingRow::default();
            }
        }
        self.schedule.settle(OpKind::Attack);
    }

    fn reselect_npc_target(&mut self, frame: &Frame<'_>, tick: u16) {
        // Only authored NPC selection depends on the executable style's reach.
        // Keep player engagement, self-defence and suspended-target policy.
        if matches!(self.request.target, Target::Npc { .. })
            && self
                .engaged
                .is_none_or(|actor| actor.kind == ActorKind::Npc)
        {
            self.observe_target(frame, tick, true);
        }
    }

    fn carries_boost(&self) -> bool {
        self.request.kits.get(self.active).is_some_and(|kit| {
            kit.carry.iter().any(|(id, _)| {
                self.boost_kinds().iter().any(|kind| {
                    self.tables.potion(*kind).is_some_and(|family| {
                        family.doses.iter().flatten().any(|dose| dose.id == *id)
                    })
                })
            })
        })
    }
    fn request_worth(&self) -> bool {
        self.carries_boost() || select::requested_worth(&self.request, &self.tables)
    }
    fn resolve_worth(&mut self, frame: &Frame<'_>) {
        let carries_boost = self.carries_boost();

        let worth = select::offensives_worth(
            self.engaged,
            self.engaged_type(),
            &self.tables,
            carries_boost,
        ) || (self.engaged.is_none()
            && select::requested_worth(&self.request, &self.tables));
        if worth {
            // A target picked after Prep handed off still earns its opening
            // drink before Engage can issue the first attack.
            if self.flags & BOOST_WORTH == 0 && self.phase == Phase::Engage {
                self.phase = Phase::Prep;
            }
            self.flags |= BOOST_WORTH;
        } else {
            self.flags &= !BOOST_WORTH;
        }
        if self
            .tables
            .npc(self.engaged_type())
            .is_some_and(|row| row.dragonfire.is_some())
        {
            self.flags |= SHIELD_OVERRIDE;
        } else {
            self.flags &= !SHIELD_OVERRIDE;
        }
        if self.rhand_pick[self.active.index()] < 0 {
            self.pick_weapons(frame);
        }
    }
    /// Resolve the right-hand pick of every style that has no kit
    /// (PVP-DESIGN-1026 §4.2). A style with a kit wears the kit's rows only.
    fn pick_weapons(&mut self, frame: &Frame<'_>) {
        for style in Style::ALL {
            if self.request.kits.get(style).is_none() {
                self.rhand_pick[style.index()] = self.pick_weapon(style, frame);
            }
        }
    }
    fn pick_weapon(&self, style: Style, frame: &Frame<'_>) -> i32 {
        match style {
            Style::Mage => self.best_staff(frame),
            Style::Ranged => self.best_ranged_weapon(frame),
            Style::Melee => switch::best_melee_weapon(&self.tables, frame),
        }
    }
    fn observe_target(&mut self, frame: &Frame<'_>, tick: u16, reselect: bool) {
        let seed = u64::from(tick) ^ self.sequence;
        if self.engaged.is_none() || reselect {
            if let Some(actor) = select::pick_target(
                &self.request,
                self.switch.pending().unwrap_or(self.active),
                frame,
                &self.threats,
                &self.tables,
                seed,
            ) {
                if self.engaged != Some(actor) {
                    self.engage(actor, frame);
                }
            } else if reselect && self.engaged.take().is_some() {
                self.reset_target_attack();
                self.identity_kind = EngagementKind::None;
                self.identity = 0;
                self.flags &= !LOST;
            }
        }
        let mut present = false;
        let mut killed = false;
        let mut refused = None;
        if let Some(actor) = self.engaged {
            match actor.kind {
                ActorKind::Npc => {
                    if let Some(row) = frame
                        .npcs
                        .iter()
                        .find(|row| row.index == usize::from(actor.index))
                    {
                        let ty = row.r#type.map_or(-1, |id| id as i32);
                        let same = ty == self.engaged_type();
                        let listed = match &self.request.target {
                            Target::Npc { types, .. } => types.contains(&ty),
                            _ => same,
                        };
                        present = listed
                            && row
                                .actions
                                .iter()
                                .flatten()
                                .any(|action| action == "Attack")
                            && distance(row.tile, self.request.stand.unwrap_or(frame.here))
                                <= i32::from(self.request.lost_radius);
                        killed = same && row.total_health > 0 && row.health == 0;
                        if present && !same {
                            self.identity_kind = EngagementKind::Npc;
                            self.identity = ty;
                            self.schedule.interaction = Interaction::Unknown;
                            self.resolve_worth(frame);
                        }
                    }
                }
                ActorKind::Player => {
                    if let Some(row) = frame.players.iter().find(|row| {
                        row.index == usize::from(actor.index)
                            && self.player_ident().is_some()
                            && row.actor.name.as_deref().map(select::ident::fnv1a)
                                == self.player_ident()
                    }) {
                        present =
                            distance(row.actor.tile, self.request.stand.unwrap_or(frame.here))
                                <= i32::from(self.request.lost_radius);
                        killed = row.actor.total_health > 0 && row.actor.health == 0;
                        if present && !killed {
                            refused = select::player_candidate(row, frame, &self.tables).err();
                        }
                    }
                }
            }
        }
        if killed {
            if self.resume_suspended(frame) {
                return;
            }
            self.finish(CombatEnd::Killed, tick);
            return;
        }
        if let Some(reason) = refused {
            if !self.resume_suspended(frame) {
                self.finish(CombatEnd::Aborted(AbortReason::PvpRefused(reason)), tick);
            }
            return;
        }
        if self.engaged.is_some() && !present {
            if self.flags & LOST == 0 {
                self.lost_since = tick;
                self.flags |= LOST;
            } else if elapsed(tick, self.lost_since) >= 3 && !self.resume_suspended(frame) {
                self.finish(CombatEnd::TargetGone, tick);
            }
            return;
        }
        if present {
            self.flags &= !(LOST | LEASH_WALKED);
        }
        if self.engaged.is_none() {
            if self.phase != Phase::Escape && select::stand_pending(&self.request, frame) {
                // Walking to the stand is not lost-target time. Leave the
                // clock unarmed until arrival, even if polls skip ticks.
                // Only failed walks spend the step's approach counter.
                self.flags &= !LOST;
            } else {
                self.stand_walk_failures = 0;
                if self.flags & LOST == 0 {
                    self.lost_since = tick;
                    self.flags |= LOST;
                } else if elapsed(tick, self.lost_since) >= 15 {
                    self.finish(CombatEnd::NoTarget, tick);
                }
            }
        }
        if self.phase == Phase::Engage
            && self.flags & ENGAGEMENT_ATTACK != 0
            && self.engaged.is_some_and(|actor| {
                frame
                    .in_combat
                    .target
                    .is_some_and(|target| actor.matches(target))
                    || self.threats.iter(tick).any(|threat| threat.actor == actor)
            })
        {
            self.phase = Phase::Fight;
        }
        if matches!(self.request.target, Target::Npc { .. })
            && self.request.intruder.player == IntruderRule::FightBack
            && self
                .engaged
                .is_some_and(|actor| actor.kind == ActorKind::Npc)
        {
            let player = self
                .threats
                .iter(tick)
                .find(|threat| threat.actor.kind == ActorKind::Player)
                .map(|threat| threat.actor);
            if let Some(actor) = player {
                let (was, was_type) = (self.engaged, self.engaged_type());
                if self.engage(actor, frame) {
                    self.suspended = was;
                    self.suspended_type = was_type;
                    self.counters.intruders = self.counters.intruders.saturating_add(1);
                    self.phase = Phase::Fight;
                }
            }
        }
    }
    fn resume_suspended(&mut self, frame: &Frame<'_>) -> bool {
        let Some(actor) = self.suspended.take() else {
            return false;
        };
        if frame.npcs.iter().any(|row| {
            row.index == usize::from(actor.index)
                && row.r#type == usize::try_from(self.suspended_type).ok()
                && !(row.total_health > 0 && row.health == 0)
                && row
                    .actions
                    .iter()
                    .flatten()
                    .any(|action| action == "Attack")
        }) {
            self.engage(actor, frame);
            self.phase = Phase::Fight;
            true
        } else {
            false
        }
    }
    fn advance_eat_ready(&mut self, tick: u16) {
        let ready = tick.wrapping_add(2);
        if self.flags & OBSERVED == 0 || reached(ready, self.eat_ready) {
            self.eat_ready = ready;
        }
    }
    fn pending_row(&self, kind: RowKind) -> bool {
        self.pending.iter().any(|pending| pending.row.kind == kind)
    }
    fn pending_wear(&self, slot: u8) -> bool {
        self.pending
            .iter()
            .any(|pending| pending.row.kind == RowKind::Wear && pending.row.aux == slot)
    }
    fn pending_id(&self, kind: RowKind, id: i32) -> bool {
        self.pending
            .iter()
            .any(|pending| pending.row.kind == kind && pending.row.id == id)
    }
    fn effect(&self, row: PlanRow) -> super::schedule::InputEffect<'_> {
        match row.kind {
            RowKind::Eat => super::schedule::InputEffect::Food(
                self.tables.food(row.id).expect("planned food fact"),
            ),
            RowKind::CureFruit => super::schedule::InputEffect::CureFruit,
            RowKind::Attack if self.active == Style::Ranged => {
                super::schedule::InputEffect::RangedAttack
            }
            RowKind::Prayer if row.aux != 0 => super::schedule::InputEffect::PrayerOn,
            RowKind::Wear
                if self
                    .tables
                    .selected()
                    .item_by_alias("elemental_shield")
                    .is_some_and(|item| item.id == row.id) =>
            {
                super::schedule::InputEffect::ElementalShield
            }
            _ => super::schedule::InputEffect::Standard,
        }
    }
    fn projected_schedule(&self) -> Schedule {
        let mut schedule = self.schedule;
        if self.plan_first_id != 0 {
            for row in self.plan.iter() {
                if row.kind == RowKind::Arm && row.aux & arbiter::ARM_SIDE_TAB_FLAG != 0 {
                    continue;
                }
                schedule.admitted(
                    op_kind(row.kind),
                    self.emitted_tick,
                    self.plan_rate(),
                    self.flags & PLAN_FIGHT != 0,
                    self.effect(row),
                );
                if row.kind == RowKind::Attack
                    && schedule.clear_valid
                    && schedule.last_clear == self.emitted_tick
                {
                    schedule.restore_due = self.emitted_tick;
                }
            }
        }
        schedule
    }
    fn plan_rate(&self) -> u8 {
        self.plan
            .iter()
            .find(|row| row.kind == RowKind::Attack)
            .map_or(4, |row| row.aux)
    }
    fn prayer_varp(&self, button: i32) -> Option<i32> {
        self.tables
            .selected()
            .prayers()
            .iter()
            .find(|row| row.button_com == button)
            .map(|row| row.varp)
    }
    fn receipts(&mut self, cx: &ActionContext<'_>) -> bool {
        if self.plan_first_id == 0 {
            return true;
        }
        let mut accepted = 0;
        for index in 0..u64::from(self.plan.len) {
            let Some(receipt) = cx.interaction_receipt(self.plan_first_id + index) else {
                return false;
            };
            if receipt.accepted && accepted == index {
                accepted += 1;
            }
        }
        let fight = self.flags & PLAN_FIGHT != 0;
        let rate = self.plan_rate();
        let mut stage = self.staged_pending;
        for index in 0..usize::from(self.plan.len) {
            let slot = stage.trailing_zeros() as usize;
            stage &= !(1 << slot);
            let row = self.plan.rows[index];
            if row.kind == RowKind::Arm {
                self.pending[slot] = PendingRow::default();
                if index < accepted as usize {
                    if let Some(arm) = &mut self.magic_mut().arm {
                        arm.emitted();
                        self.magic_mut().arm_since = self.emitted_tick;
                        self.magic_mut().arm_wait = row.aux & arbiter::ARM_WAIT_MASK;
                    }
                    if row.aux & arbiter::ARM_SIDE_TAB_FLAG == 0 {
                        self.schedule.clear(self.emitted_tick, 1, fight);
                    }
                }
                continue;
            }
            if index >= accepted as usize {
                self.pending[slot] = PendingRow::default();
                continue;
            }
            if row.kind != RowKind::Attack {
                // A newly accepted clearing input cancels any previous
                // installed/pending interaction, but not this plan's terminal.
                for (slot, pending) in self.pending.iter_mut().enumerate() {
                    if self.staged_pending & (1 << slot) == 0 && pending.row.kind == RowKind::Attack
                    {
                        *pending = PendingRow::default();
                    }
                }
            }
            if fight
                && !matches!(row.kind, RowKind::Attack | RowKind::Cast)
                && self.flags & RESTORE_COUNTED == 0
            {
                self.counters.restorations = self.counters.restorations.saturating_add(1);
                self.flags |= RESTORE_COUNTED;
            }
            let effect = match row.kind {
                RowKind::Eat => super::schedule::InputEffect::Food(
                    self.tables.food(row.id).expect("planned food fact"),
                ),
                RowKind::CureFruit => super::schedule::InputEffect::CureFruit,
                RowKind::Attack if self.active == Style::Ranged => {
                    super::schedule::InputEffect::RangedAttack
                }
                RowKind::Prayer if row.aux != 0 => super::schedule::InputEffect::PrayerOn,
                RowKind::Wear
                    if self
                        .tables
                        .selected()
                        .item_by_alias("elemental_shield")
                        .is_some_and(|item| item.id == row.id) =>
                {
                    super::schedule::InputEffect::ElementalShield
                }
                _ => super::schedule::InputEffect::Standard,
            };
            self.schedule
                .admitted(op_kind(row.kind), self.emitted_tick, rate, fight, effect);
            match row.kind {
                RowKind::Prayer => {
                    let varp = self.prayer_varp(row.id).expect("planned prayer component");
                    let displaced = self.prayer_displaced(varp, row.aux != 0);
                    self.baseline_on &= !(displaced | prayer_bit(varp));
                    self.raised_prayers.accepted(varp, row.aux != 0, displaced);
                    if row.aux != 0 {
                        let bit = (varp - api::prayer::PRAYER_VARP0) as usize;
                        self.prayer_ready[bit] = self.emitted_tick.wrapping_add(3) as u8;
                        self.prayer_ready_mask |= 1 << bit;
                        if (0..3)
                            .filter_map(|tier| self.tables.prayer(PrayerRole::Protect, tier))
                            .any(|fact| fact.varp == varp)
                        {
                            self.counters.switches = self.counters.switches.saturating_add(1);
                        }
                    } else if self.phase == Phase::WindDown {
                        if let Some((index, fact)) = self
                            .tables
                            .selected()
                            .prayers()
                            .iter()
                            .enumerate()
                            .find(|(_, fact)| fact.varp == varp)
                        {
                            self.sweep.accepted(super::prayer::SweepClick {
                                index,
                                varp,
                                button_com: fact.button_com,
                            });
                        }
                    }
                }
                RowKind::Wear if !fight => self.prep_failures |= 4 << (u32::from(row.aux) * 3),
                RowKind::Retaliate if !fight => self.flags |= PREP_RETALIATE,
                RowKind::Drink => {
                    if !fight && row.aux <= PotionKind::Antifire as u8 {
                        self.prep_failures |= 1 << (POTION_PREP_SHIFT + u32::from(row.aux));
                    }
                    if potion_kind(row.aux) == PotionKind::Antifire {
                        self.antifire_sip = self.emitted_tick;
                    }
                }
                RowKind::Style => {
                    if let StyleState::Melee(state) = &mut self.style_state {
                        if let Some(mode) = MeleeMode::from_code((row.aux >> 2) & 3) {
                            state.mode_fallback =
                                (Some(mode) != self.request.melee_mode).then_some(mode);
                        }
                    }
                }
                RowKind::Cast => {
                    self.flags |= ENGAGEMENT_ATTACK;
                    self.flags &= !RESTORE_COUNTED;
                }
                RowKind::Attack => {
                    self.flags |= ENGAGEMENT_ATTACK;
                    self.flags &= !RESTORE_COUNTED;
                    if self.schedule.clear_valid && self.schedule.last_clear == self.emitted_tick {
                        self.schedule.restore_due = self.emitted_tick;
                    }
                }
                _ => {}
            }
        }
        self.plan_first_id = 0;
        self.staged_pending = 0;
        true
    }
    fn settle(&mut self, frame: &Frame<'_>, tick: u16) {
        let total = frame
            .inventory
            .iter()
            .filter(|item| {
                self.tables
                    .food(item.def.id)
                    .is_some_and(|food| food.eat_delay_arg.is_some())
            })
            .fold(0u16, |total, item| {
                total.saturating_add(item.count.clamp(0, i32::from(u16::MAX)) as u16)
            });
        if total < self.food_total {
            self.advance_eat_ready(tick);
        }
        self.food_total = total;
        let timed_out = self
            .pending
            .iter()
            .filter(|pending| {
                pending.row.kind == RowKind::Prayer && pending.row.aux == 0 && pending.age >= 3
            })
            .filter_map(|pending| self.prayer_varp(pending.row.id))
            .fold(0u32, |mask, varp| {
                mask | (1 << (varp - api::prayer::PRAYER_VARP0))
            });
        let observation = prayer_observation(frame);
        self.sweep.observe(&observation, timed_out);
        for row in self.tables.selected().prayers() {
            if observation.is_off(row.varp) && !self.pending_id(RowKind::Prayer, row.button_com) {
                self.raised_prayers.accepted(row.varp, false, 0);
            }
        }
        let onset = match &mut self.style_state {
            StyleState::Ranged(state) => super::style::ranged::onset(
                frame.local,
                self.engaged,
                &self.tables,
                &mut state.anim_update,
            ),
            StyleState::Melee(state) => super::style::melee::onset(
                &frame.local.player.actor,
                self.engaged,
                &self.tables,
                &mut state.anim_id,
                &mut state.anim_frame,
            ),
            StyleState::Magic(_) => false,
        };
        if onset
            && (!self.schedule.clear_valid
                || (reached(tick, self.schedule.last_clear) && tick != self.schedule.last_clear))
        {
            self.counters.swings = self.counters.swings.saturating_add(1);
            self.flags2 |= PVP_SWING;
            self.schedule.observe_swing(tick, self.rate(frame));
        }
        for slot in 0..self.pending.len() {
            let pending = self.pending[slot];
            let row = pending.row;
            if row.kind == RowKind::Empty {
                continue;
            }
            let kind = op_kind(row.kind);
            let settled = match row.kind {
                RowKind::Eat => arbiter::count(frame, row.id) < i32::from(pending.baseline),
                RowKind::CureFruit => arbiter::count(frame, row.id) < i32::from(pending.baseline),
                RowKind::Drink => {
                    arbiter::doses(frame, &self.tables, potion_kind(row.aux)) < pending.baseline
                }
                RowKind::Prayer => self.prayer_varp(row.id).is_some_and(|varp| {
                    frame
                        .varps
                        .iter()
                        .any(|value| value.index == varp && value.value == i32::from(row.aux != 0))
                }),
                RowKind::Wear => frame
                    .equipment
                    .iter()
                    .any(|item| item.slot == i32::from(row.aux) && item.def.id == row.id),
                RowKind::Style => self.style_varp().is_some_and(|varp| {
                    frame
                        .varps
                        .iter()
                        .any(|value| value.index == varp && value.value == i32::from(row.aux & 3))
                }),
                RowKind::Retaliate => frame.varps.iter().any(|value| {
                    value.index == super::OPTION_NODEF && value.value == i32::from(row.aux == 0)
                }),
                RowKind::Attack => {
                    self.engaged
                        .is_some_and(|actor| actor_token(actor) == row.id)
                        && (self.schedule.interaction == Interaction::Installed
                            || self.engaged.is_some_and(|actor| {
                                frame
                                    .in_combat
                                    .target
                                    .is_some_and(|target| actor.matches(target))
                            }))
                        && (!self.schedule.clear_valid
                            || (reached(tick, self.schedule.last_clear)
                                && tick != self.schedule.last_clear))
                }
                RowKind::Pickup => self.ammo_count(frame) > self.ranged().pickup_baseline,
                RowKind::Cast | RowKind::Arm | RowKind::Empty => false,
            };
            if settled {
                self.pending[slot] = PendingRow::default();
                match row.kind {
                    RowKind::Eat => self.counters.food = self.counters.food.saturating_add(1),
                    RowKind::Drink => match potion_kind(row.aux) {
                        PotionKind::Prayer => {
                            self.counters.prayer = self.counters.prayer.saturating_add(1)
                        }
                        PotionKind::Antifire => {
                            self.counters.antifire = self.counters.antifire.saturating_add(1);
                            self.flags |= ANTIFIRE;
                        }
                        PotionKind::Antipoison | PotionKind::SuperAntipoison => {}
                        _ => self.counters.boost = self.counters.boost.saturating_add(1),
                    },
                    RowKind::Pickup => self.counters.ammo = self.counters.ammo.saturating_add(1),
                    RowKind::Attack => self.schedule.interaction = Interaction::Installed,
                    RowKind::Wear => self.prep_failures &= !(7 << (u32::from(row.aux) * 3)),
                    RowKind::Retaliate => self.flags &= !PREP_RETALIATE,
                    RowKind::Cast => self.schedule.interaction = Interaction::Installed,
                    _ => {}
                }
                if !self.pending_row(row.kind) {
                    self.schedule.settle(kind);
                }
            } else {
                self.pending[slot].age = pending.age.saturating_add(1);
                let window = match row.kind {
                    RowKind::Eat => EAT_OBSERVATION_WINDOW_TICKS,
                    RowKind::Drink
                    | RowKind::CureFruit
                    | RowKind::Wear
                    | RowKind::Retaliate
                    | RowKind::Style
                    | RowKind::Pickup => 4,
                    RowKind::Prayer | RowKind::Attack => 8,
                    RowKind::Cast => 10,
                    RowKind::Arm => unreachable!("arm receipts settle independently"),
                    RowKind::Empty => unreachable!("pending row"),
                };
                if self.pending[slot].age < window {
                    continue;
                }
                self.pending[slot] = PendingRow::default();
                if row.kind == RowKind::Prayer && row.aux != 0 {
                    if let Some(varp) = self.prayer_varp(row.id) {
                        if observation.is_off(varp) {
                            self.raised_prayers.accepted(varp, false, 0);
                        }
                    }
                }
                if !self.pending_row(row.kind) {
                    self.schedule.timeout(kind);
                }
                let failures = self.schedule.unsettled[kind.index()];
                match row.kind {
                    RowKind::Cast if failures >= 2 => self.deselect(true),
                    RowKind::Cast => {}
                    RowKind::Wear => {
                        let shift = u32::from(row.aux) * 3;
                        let count = (((self.prep_failures >> shift) & 3) + 1).min(3);
                        self.prep_failures =
                            (self.prep_failures & !(3 << shift)) | (count << shift);
                        if self.switch.pending().is_some() {
                            // A refused wear of a pending kit is the switch's to
                            // judge (`Failed`), never the engagement's.
                        } else if count >= 2 && row.aux == 3 {
                            self.flags |= FAILED_WEAPON;
                        } else if count >= 2 && row.aux == 5 && self.flags & SHIELD_OVERRIDE != 0 {
                            self.flags |= FAILED_SHIELD;
                        } else if count >= 3 && !self.wear_skipped(row.aux) {
                            self.flags |= FAILED_ATTACK;
                        }
                    }
                    RowKind::Drink
                        if failures >= 2
                            && (1..=5).contains(&row.aux)
                            && self.prep_failures
                                & (1 << (POTION_PREP_SHIFT + u32::from(row.aux)))
                                != 0 =>
                    {
                        self.prep_failures |= 1 << (POTION_SKIP_SHIFT + u32::from(row.aux));
                    }
                    RowKind::Retaliate if failures >= 2 && self.flags & PREP_RETALIATE != 0 => {
                        self.flags |= SKIP_RETALIATE
                    }
                    RowKind::Pickup => {}
                    _ if failures >= 3 => self.flags |= FAILED_ATTACK,
                    _ => {}
                }
            }
        }
    }
    fn shield(&self, frame: &Frame<'_>) -> bool {
        self.tables
            .selected()
            .item_by_alias("antidragonbreathshield")
            .is_some_and(|item| {
                frame
                    .equipment
                    .iter()
                    .any(|row| row.slot == 5 && row.def.id == item.id)
            })
    }
    fn antifire(&self, tick: u16) -> bool {
        self.flags & ANTIFIRE != 0 && elapsed(tick, self.antifire_sip) < 600
    }
    fn safety_end(
        &mut self,
        frame: &Frame<'_>,
        tick: u16,
        danger: Option<i32>,
        emergency: i32,
        food_available: bool,
    ) {
        if self.phase == Phase::Escape || self.phase == Phase::WindDown {
            return;
        }
        let no_food =
            danger != Some(0) && arbiter::stat(frame, 3).0 <= emergency && !food_available;
        let no_fire = self.flags & SHIELD_OVERRIDE != 0
            && !self.shield(frame)
            && !self.antifire(tick)
            && (!self.request.allow.equipment
                || self
                    .tables
                    .selected()
                    .item_by_alias("antidragonbreathshield")
                    .is_none_or(|item| arbiter::count(frame, item.id) == 0))
            && (!self.request.allow.potions
                || arbiter::potion_id(frame, &self.tables, PotionKind::Antifire).is_none());
        if no_food || no_fire {
            let reason = AbortReason::Unprotected(if no_fire {
                Unprotected::Dragonfire
            } else {
                Unprotected::NoFood
            });
            match self.request.fallback {
                Fallback::Retreat { .. } => {
                    self.phase = Phase::Escape;
                    self.escape_since = tick;
                    self.schedule.terminal();
                }
                Fallback::Fight if !no_fire => {}
                _ => self.finish(CombatEnd::Aborted(reason), tick),
            }
        }
    }
    fn push(&self, plan: &mut TickPlan, row: PlanRow) -> bool {
        let free = self
            .pending
            .iter()
            .filter(|pending| pending.row.kind == RowKind::Empty)
            .count();
        let reserve =
            if matches!(self.phase, Phase::Engage | Phase::Fight) && !row.terminal(&self.tables) {
                2
            } else {
                0
            };
        // A clearing row also reserves server-effect storage for its Attack.
        if usize::from(plan.len) + 1 + usize::from(reserve != 0) > free {
            return false;
        }
        plan.push(row, reserve, &self.tables)
    }
    fn cure_row(
        &self,
        plan: &mut TickPlan,
        frame: &Frame<'_>,
        tick: u16,
        danger: Option<i32>,
        eat_floor: i32,
        cx: &ActionContext<'_>,
    ) -> bool {
        let decision = cure::cure_choice(&CureInputs {
            cond: cx.condition(),
            danger,
            exposure: crate::survival::math::exposure(&self.threats, &self.tables, tick),
            policy: cx.cure_poison(),
            route_refused_for_poison: false,
            inventory: frame.inventory,
            tables: &self.tables,
            schedule: &self.schedule,
            clue_reserve: false,
            tick,
            q_unknown: cx.poison_unknown_pulse(),
            eat_floor,
        });
        let row = match decision {
            CureDecision::Drink { item, kind, .. } => {
                PlanRow::new(RowKind::Drink, item, kind as u8)
            }
            CureDecision::EatFruit { item, .. } => PlanRow::new(RowKind::CureFruit, item, 0),
            CureDecision::None | CureDecision::Needed { .. } => return false,
        };
        let _ = self.push(plan, row);
        true
    }

    fn emergency_food_available(&self, frame: &Frame<'_>, danger: Option<i32>) -> bool {
        if !self.request.allow.food {
            return false;
        }
        if arbiter::food_by(frame, &self.tables, None, |food| {
            food.eat_delay_arg.is_some()
        })
        .is_some()
        {
            return true;
        }

        let (hp, max) = arbiter::stat(frame, 3);
        arbiter::food_by(frame, &self.tables, None, |food| {
            let Some(delay) = food.message_delay else {
                return false;
            };
            let gate = danger.map_or((max + 1) / 2, |danger| {
                danger.saturating_mul(delay + 2).saturating_add(1)
            });
            food.eat_delay_arg.is_none() && hp.saturating_add(food.heal).min(max) > gate
        })
        .is_some()
    }
    fn plain_food(&self, frame: &Frame<'_>, tick: u16, gate: Option<i32>) -> Option<i32> {
        if !self.request.allow.food || self.pending_row(RowKind::Eat) {
            return None;
        }
        arbiter::food_by(frame, &self.tables, gate, |food| {
            food.eat_delay_arg.is_some() && self.schedule.food_ready(food, tick)
        })
    }
    fn eat(
        &self,
        plan: &mut TickPlan,
        frame: &Frame<'_>,
        tick: u16,
        danger: Option<i32>,
        poison_fire: i32,
    ) {
        if plan.contains(RowKind::Eat)
            || self.pending_row(RowKind::Eat)
            || !self.request.allow.food
            || self.schedule.unsettled[OpKind::Eat.index()] >= 3
        {
            return;
        }
        let (hp, max) = arbiter::stat(frame, 3);
        let Some(choice) = policy::eat_choice(
            frame,
            &self.tables,
            &self.schedule,
            danger,
            poison_fire,
            hp,
            max,
            tick,
            0,
        ) else {
            return;
        };
        let ordinary = choice.ordinary;
        if let Some(id) = ordinary {
            if !self.push(plan, PlanRow::new(RowKind::Eat, id, 0)) {
                return;
            }
        }
        if ordinary.is_some() && !reached(tick, self.eat_ready) {
            return;
        }
        if let Some(id) = choice.combo {
            self.push(plan, PlanRow::new(RowKind::Eat, id, 0));
        }
    }
    fn drink_or_recover(
        &self,
        plan: &mut TickPlan,
        kind: PotionKind,
        lowers_danger: bool,
        frame: &Frame<'_>,
        tick: u16,
        gate: i32,
    ) {
        if plan.closed(&self.tables)
            || !self.request.allow.potions
            || (kind == PotionKind::Prayer && !self.request.allow.prayer)
            || !self.schedule.ready(OpKind::Drink, tick)
            || self.pending_row(RowKind::Drink)
            || self.prep_failures & (1 << (POTION_SKIP_SHIFT + kind as u32)) != 0
        {
            return;
        }
        let Some(id) = arbiter::potion_id(frame, &self.tables, kind) else {
            return;
        };
        let (hp, max) = arbiter::stat(frame, 3);
        let eat = plan.iter().find(|row| row.kind == RowKind::Eat);
        let projected = if reached(tick, self.eat_ready) {
            hp.saturating_add(
                eat.and_then(|row| self.tables.food(row.id))
                    .map_or(0, |food| food.heal),
            )
            .min(max)
        } else {
            hp
        };
        if projected > gate {
            self.push(plan, PlanRow::new(RowKind::Drink, id, kind as u8));
            return;
        }
        if !lowers_danger || eat.is_some() {
            return;
        }
        if let Some(food) = self.plain_food(frame, tick, Some(gate)) {
            if self.push(plan, PlanRow::new(RowKind::Eat, food, 0)) && reached(tick, self.eat_ready)
            {
                self.push(plan, PlanRow::new(RowKind::Drink, id, kind as u8));
            }
        }
    }
    fn advance_prayer_ready(&mut self, tick: u16) {
        if self.flags & OBSERVED != 0 && elapsed(tick, self.last_tick) >= 128 {
            self.prayer_ready_mask = 0;
            return;
        }
        let mut pending = self.prayer_ready_mask;
        while pending != 0 {
            let bit = pending.trailing_zeros() as usize;
            let mask = 1 << bit;
            pending &= !mask;
            if (tick as u8).wrapping_sub(self.prayer_ready[bit]) as i8 >= 0 {
                self.prayer_ready_mask &= !mask;
            }
        }
    }
    fn prayer_ready(&self, varp: i32, tick: u16) -> bool {
        let bit = (varp - api::prayer::PRAYER_VARP0) as usize;
        bit < 15
            && (self.prayer_ready_mask & (1 << bit) == 0
                || (tick as u8).wrapping_sub(self.prayer_ready[bit]) as i8 >= 0)
    }
    fn project_prayer(&self, mask: &mut u16, varp: i32, on: bool) {
        if on {
            for role in [
                PrayerRole::Protect,
                PrayerRole::Strength,
                PrayerRole::Attack,
            ] {
                if (0..3)
                    .filter_map(|tier| self.tables.prayer(role, tier))
                    .any(|row| row.varp == varp)
                {
                    for row in (0..3).filter_map(|tier| self.tables.prayer(role, tier)) {
                        *mask &= !(1 << (row.varp - api::prayer::PRAYER_VARP0));
                    }
                }
            }
            *mask |= 1 << (varp - api::prayer::PRAYER_VARP0);
        } else {
            *mask &= !(1 << (varp - api::prayer::PRAYER_VARP0));
        }
    }
    fn capture_prayer_baseline(&mut self, observation: &PrayerObservation) {
        if self.baseline_on & BASELINE_READY != 0 {
            return;
        }
        if (0..api::prayer::PRAYER_COUNT)
            .all(|index| observation.varp_observed(api::prayer::PRAYER_VARP0 + index as i32))
        {
            self.baseline_on = BASELINE_READY
                | (0..api::prayer::PRAYER_COUNT).fold(0, |mask, index| {
                    let varp = api::prayer::PRAYER_VARP0 + index as i32;
                    mask | if observation.is_on(varp) {
                        prayer_bit(varp)
                    } else {
                        0
                    }
                });
        }
    }
    fn prayer_displaced(&self, varp: i32, on: bool) -> u16 {
        if !on {
            return 0;
        }
        let mut projected = BASELINE_READY - 1;
        self.project_prayer(&mut projected, varp, true);
        (BASELINE_READY - 1) & !projected
    }
    pub(crate) fn prayer_plan_first_id(&self) -> u64 {
        self.plan_first_id
    }
    /// Include accepted dispatches that a cancellation interrupted before poll.
    /// Planned, queued, missing and refused receipts confer no ownership.
    pub(crate) fn prayer_cleanup(&self, accepted_prefix: usize) -> RaisedPrayers {
        let mut raised = self.raised_prayers;
        if self.plan_first_id != 0 {
            for row in self.plan.iter().take(accepted_prefix) {
                if row.kind == RowKind::Prayer {
                    let varp = self.prayer_varp(row.id).expect("planned prayer component");
                    raised.accepted(
                        varp,
                        row.aux != 0,
                        self.prayer_displaced(varp, row.aux != 0),
                    );
                }
            }
        }
        raised
    }
    fn offensive_prayer(
        &self,
        role: PrayerRole,
        base: i32,
        observation: &PrayerObservation,
    ) -> Option<&api::game_data::PrayerFact> {
        // A user's tier wins even when Combat can afford a stronger one.
        (0..3)
            .rev()
            .filter_map(|tier| self.tables.prayer(role, tier))
            .find(|row| {
                self.baseline_on & prayer_bit(row.varp) != 0
                    || (observation.is_on(row.varp) && !self.raised_prayers.contains(row.varp))
            })
            .or_else(|| {
                (0..3)
                    .rev()
                    .filter_map(|tier| self.tables.prayer(role, tier))
                    .find(|row| base >= row.level)
            })
    }
    fn plan(
        &mut self,
        frame: &Frame<'_>,
        tick: u16,
        cx: &mut ActionContext<'_>,
    ) -> Result<TickPlan, ActionError> {
        let mut plan = TickPlan::default();
        let (hp, hp_max) = arbiter::stat(frame, 3);
        let mut danger = self.threats.danger(
            frame,
            &self.tables,
            tick,
            self.shield(frame),
            self.antifire(tick),
        );
        let poison_fire =
            crate::survival::math::poison_fire(cx.condition(), &self.threats, &self.tables, tick);
        let mut lines = select::lines(danger, hp_max).with_poison(poison_fire);
        let food_available = self.emergency_food_available(frame, danger);
        self.safety_end(frame, tick, danger, lines.emergency, food_available);
        if self.cure_row(&mut plan, frame, tick, danger, lines.eat, cx) {
            return Ok(plan);
        }
        if self.phase == Phase::WindDown {
            let obs = prayer_observation(frame);
            if self.baseline_on & BASELINE_READY == 0 {
                return Ok(plan);
            }
            for click in self
                .sweep
                .candidates(self.tables.selected(), &obs)
                .filter(|click| self.raised_prayers.contains(click.varp))
            {
                if !self.pending_id(RowKind::Prayer, click.button_com) {
                    self.push(
                        &mut plan,
                        PlanRow::new(RowKind::Prayer, click.button_com, 0),
                    );
                }
            }
            if plan.len == 0
                && self.flags & TERMINAL_FOOD == 0
                && hp > 0
                && (danger != Some(0) || poison_fire > 0)
                && hp <= lines.eat
            {
                self.eat(&mut plan, frame, tick, danger, poison_fire);
            }
            if plan.len == 0 && !self.pending_row(RowKind::Prayer) {
                self.ranged_sweep(&mut plan, frame, tick, cx);
            }
            return Ok(plan);
        }
        if self.phase == Phase::Escape {
            let Fallback::Retreat { tile } = self.request.fallback else {
                unreachable!("escape requires retreat");
            };
            if self
                .pending_walk
                .and_then(|id| cx.walk_receipt(id.get()))
                .is_some()
                || elapsed(tick, self.escape_since) >= 60
            {
                self.finish(CombatEnd::Aborted(AbortReason::RetreatFailed), tick);
                return self.plan(frame, tick, cx);
            }
            if self.request.allow.retaliate_toggle && retaliate_on(frame) {
                if !self.pending_row(RowKind::Retaliate)
                    && self.schedule.ready(OpKind::Retaliate, tick)
                {
                    self.push(&mut plan, PlanRow::new(RowKind::Retaliate, 0, 0));
                }
                return Ok(plan);
            }
            if hp > 0 && (danger != Some(0) || poison_fire > 0) && hp <= lines.eat {
                self.eat(&mut plan, frame, tick, danger, poison_fire);
            }
            if plan.len == 0 && self.pending_walk.is_none() {
                self.walk(tile, tick, cx, false)?;
            }
            return Ok(plan);
        }
        // Emergency uses the original danger, before free reducers project it.
        if (danger != Some(0) || poison_fire > 0) && hp <= lines.emergency {
            self.eat(&mut plan, frame, tick, danger, poison_fire);
        }
        if plan.closed(&self.tables) {
            return Ok(plan);
        }
        let (points, base) = arbiter::stat(frame, 5);
        let prayer_allowed = self.baseline_on & BASELINE_READY != 0 && self.request.allow.prayer;
        let wanted = prayer_allowed
            .then(|| {
                policy::wanted_protect(
                    &self.threats,
                    frame,
                    &self.tables,
                    tick,
                    self.shield(frame),
                    self.antifire(tick),
                )
            })
            .flatten();
        let observation = prayer_observation(frame);
        let mut prayers = frame
            .varps
            .iter()
            .filter(|row| {
                row.value == 1
                    && (api::prayer::PRAYER_VARP0..api::prayer::PRAYER_VARP0 + 15)
                        .contains(&row.index)
            })
            .fold(0u16, |mask, row| {
                mask | (1 << (row.index - api::prayer::PRAYER_VARP0))
            });
        if let Some(protect) =
            wanted.filter(|row| base >= row.level && observation.is_off(row.varp))
        {
            if points == 0 {
                self.drink_or_recover(
                    &mut plan,
                    PotionKind::Prayer,
                    true,
                    frame,
                    tick,
                    lines.drink_gate,
                );
            }
            if points > 0
                && self.prayer_ready(protect.varp, tick)
                && !self.pending_id(RowKind::Prayer, protect.button_com)
                && self.push(
                    &mut plan,
                    PlanRow::new(RowKind::Prayer, protect.button_com, 1),
                )
            {
                self.project_prayer(&mut prayers, protect.varp, true);
                let style = (0..3)
                    .find(|tier| {
                        self.tables
                            .prayer(PrayerRole::Protect, *tier)
                            .is_some_and(|row| row.varp == protect.varp)
                    })
                    .map(|tier| match tier {
                        0 => StyleObs::Magic,
                        1 => StyleObs::Ranged,
                        _ => StyleObs::Melee,
                    });
                danger = self.threats.danger_with(
                    &self.tables,
                    tick,
                    self.shield(frame),
                    self.antifire(tick),
                    style,
                );
                lines = select::lines(danger, hp_max).with_poison(poison_fire);
            }
        }
        let wanted_varp = wanted.map(|row| row.varp);
        let protection_wanted = wanted_varp.is_some();
        if !plan.closed(&self.tables) && (danger != Some(0) || poison_fire > 0) && hp <= lines.eat {
            self.eat(&mut plan, frame, tick, danger, poison_fire);
        }
        if plan.closed(&self.tables) {
            return Ok(plan);
        }
        // A sticky target beyond lost_radius is dropped after a grace; interrupt
        // the server's queued chase before the terminal report (design-combat.md
        // §1.1, §2.6; combat-s3a-ReviewCombatFable.md F5).
        if self.flags & LOST != 0 && self.engaged.is_some() && self.phase != Phase::WindDown {
            if plan.len == 0 && self.flags & LEASH_WALKED == 0 {
                if let Some(destination) = self.leash_cancel_tile(frame) {
                    self.walk(destination, tick, cx, false)?;
                    self.flags |= LEASH_WALKED;
                    return Ok(plan);
                }
            }
            return Ok(plan);
        }
        if self.phase == Phase::Fight {
            self.switch_start(frame, tick);
        }
        if self.active == Style::Ranged {
            if !self.ranged_prepare(frame, tick) {
                return self.plan(frame, tick, cx);
            }
            if self.pending_wear(3) || self.pending_wear(13) {
                return Ok(plan);
            }
        }
        if self.phase == Phase::Prep {
            let weapon = self.desired(3, frame);
            let conflict = self.flags & SHIELD_OVERRIDE != 0
                && weapon.is_some_and(|id| {
                    self.tables
                        .selected()
                        .item_by_id(id)
                        .is_some_and(|item| item.is_two_handed())
                });
            let missing = self.request.allow.equipment
                && weapon.is_some_and(|id| {
                    arbiter::count(frame, id) == 0
                        && !frame
                            .equipment
                            .iter()
                            .any(|row| row.slot == 3 && row.def.id == id)
                });
            if conflict || missing {
                self.finish(
                    CombatEnd::Aborted(AbortReason::PrepFailed(if conflict {
                        PrepItem::Shield
                    } else {
                        weapon_prep_item(self.active)
                    })),
                    tick,
                );
                return self.plan(frame, tick, cx);
            }
            self.wear(&mut plan, frame, tick);
            self.style(&mut plan, frame, tick);
            // Ranged Prep waits for the exact tab and observed style echo.
            if self.active == Style::Ranged {
                if let Some(reason) = self.ranged_readiness_abort(frame, Some(&plan)) {
                    self.finish(CombatEnd::Aborted(AbortReason::PrepReadiness(reason)), tick);
                    return self.plan(frame, tick, cx);
                }
                if !self.ranged_ready(frame) {
                    return Ok(plan);
                }
            }
            if self.active == Style::Mage && self.magic_prepare(&mut plan, frame, tick, cx) {
                return Ok(plan);
            }
            self.retaliate(&mut plan, frame, tick);
            if self.flags & SHIELD_OVERRIDE != 0
                && (!self.antifire(tick) || elapsed(tick, self.antifire_sip) >= 590)
            {
                self.drink_or_recover(
                    &mut plan,
                    PotionKind::Antifire,
                    true,
                    frame,
                    tick,
                    lines.drink_gate,
                );
            }
            if self.flags & BOOST_WORTH != 0 {
                if let Some(kind) = self.boost(frame) {
                    self.drink_or_recover(&mut plan, kind, false, frame, tick, lines.drink_gate);
                }
            }
            if plan.closed(&self.tables)
                || self.pending_row(RowKind::Wear)
                || self.pending_row(RowKind::Drink)
            {
                return Ok(plan);
            }
            // Only completed or in-plan equipment permits engagement.
            let missing_wear = (0..14).any(|slot| {
                self.desired(slot, frame).is_some_and(|id| {
                    !frame
                        .equipment
                        .iter()
                        .any(|row| row.slot == i32::from(slot) && row.def.id == id)
                        && !self.wear_skipped(slot)
                        && arbiter::count(frame, id) > 0
                        && !plan
                            .iter()
                            .any(|row| row.kind == RowKind::Wear && row.aux == slot && row.id == id)
                })
            });
            if missing_wear && self.request.allow.equipment {
                return Ok(plan);
            }
            self.phase = Phase::Engage;
        }
        if self.phase == Phase::Engage {
            self.style(&mut plan, frame, tick);
            self.retaliate(&mut plan, frame, tick);
            if self.active == Style::Mage {
                if self.magic_prepare(&mut plan, frame, tick, cx) {
                    return Ok(plan);
                }
                if self.magic().mode == Some(CastMode::Manual) {
                    self.manual_cast(&mut plan, frame, tick);
                    return Ok(plan);
                }
            }
            if self.engaged.is_some()
                && self.flags2 & PVP_HOLD == 0
                && (plan.len != 0
                    || (!self.pending_row(RowKind::Attack)
                        && self.schedule.ready(OpKind::Attack, tick)))
            {
                let attack = self.attack_row(&plan, frame);
                self.push(&mut plan, attack);
            } else if plan.len == 0 && self.engaged.is_none() && self.pending_walk.is_none() {
                if let Some(tile) = self
                    .request
                    .stand
                    .filter(|tile| !select::stand_reached(*tile, frame.local))
                {
                    self.walk(tile, tick, cx, true)?;
                }
            }
            return Ok(plan);
        }
        if self.request.allow.prayer
            && (protection_wanted || prayers != 0)
            && policy::prayer_sip_due(points, base)
        {
            self.drink_or_recover(
                &mut plan,
                PotionKind::Prayer,
                true,
                frame,
                tick,
                lines.drink_gate,
            );
        }
        let cycle_room = !self.schedule.cycle.known
            || !reached(tick.wrapping_add(2), self.schedule.cycle.deadline);
        if cycle_room && self.flags & BOOST_WORTH != 0 && self.target_above_quarter(frame) {
            if let Some(kind) = self.boost(frame) {
                self.drink_or_recover(&mut plan, kind, false, frame, tick, lines.drink_gate);
            }
        }
        if cycle_room
            && self.flags & SHIELD_OVERRIDE != 0
            && (!self.antifire(tick) || elapsed(tick, self.antifire_sip) >= 590)
        {
            self.drink_or_recover(
                &mut plan,
                PotionKind::Antifire,
                true,
                frame,
                tick,
                lines.drink_gate,
            );
        }
        if plan.closed(&self.tables) {
            return Ok(plan);
        }
        let offense = self.active == Style::Melee
            && self.flags & BOOST_WORTH != 0
            && prayer_allowed
            && (frame.local.player.actor.in_combat || self.threats.iter(tick).next().is_some())
            && points > 0
            && (points > policy::prayer_sip_floor(base)
                || arbiter::potion_id(frame, &self.tables, PotionKind::Prayer).is_some());
        let offensives = if offense && self.active == Style::Melee {
            [
                self.offensive_prayer(PrayerRole::Strength, base, &observation),
                self.offensive_prayer(PrayerRole::Attack, base, &observation),
            ]
        } else {
            [None, None]
        };
        for row in offensives.into_iter().flatten() {
            if prayers & prayer_bit(row.varp) == 0
                && observation.is_off(row.varp)
                && self.prayer_ready(row.varp, tick)
                && !self.pending_id(RowKind::Prayer, row.button_com)
                && self.push(&mut plan, PlanRow::new(RowKind::Prayer, row.button_com, 1))
            {
                self.project_prayer(&mut prayers, row.varp, true);
            }
        }
        for row in self.tables.selected().prayers() {
            if prayers & (1 << (row.varp - api::prayer::PRAYER_VARP0)) == 0
                || !self.raised_prayers.contains(row.varp)
                || self.pending_id(RowKind::Prayer, row.button_com)
            {
                continue;
            }
            let keep = prayer_allowed
                && (wanted_varp == Some(row.varp)
                    || offensives
                        .iter()
                        .flatten()
                        .any(|best| best.varp == row.varp));
            if !keep && self.push(&mut plan, PlanRow::new(RowKind::Prayer, row.button_com, 0)) {
                self.project_prayer(&mut prayers, row.varp, false);
            }
        }
        self.wear(&mut plan, frame, tick);
        self.style(&mut plan, frame, tick);
        self.retaliate(&mut plan, frame, tick);
        if self.active == Style::Mage && self.switch.pending().is_none() {
            if self.magic_prepare(&mut plan, frame, tick, cx) {
                return Ok(plan);
            }
            if self.magic().mode == Some(CastMode::Manual) {
                self.manual_cast(&mut plan, frame, tick);
                return Ok(plan);
            }
        }
        let mismatch = self.engaged.is_some_and(|actor| {
            frame
                .in_combat
                .target
                .is_some_and(|target| !actor.matches(target))
        });
        if !plan.closed(&self.tables)
            && self.engaged.is_some()
            && self.flags2 & PVP_HOLD == 0
            && (plan.len != 0
                || (!self.pending_row(RowKind::Attack)
                    && (self.schedule.restore_ready(tick)
                        || mismatch
                        || self.schedule.stale_ready(tick, self.rate(frame)))))
        {
            let attack = self.attack_row(&plan, frame);
            self.push(&mut plan, attack);
        }
        Ok(plan)
    }
    fn attack_row(&self, plan: &TickPlan, frame: &Frame<'_>) -> PlanRow {
        let rate = plan
            .iter()
            .find(|row| row.kind == RowKind::Wear && row.aux == 3)
            .and_then(|row| self.tables.weapon_style(row.id))
            .map_or_else(
                || self.rate(frame),
                |weapon| {
                    self.style_rate(
                        self.switch.pending().unwrap_or(self.active),
                        weapon.attackrate,
                    )
                },
            );
        PlanRow::new(
            RowKind::Attack,
            actor_token(self.engaged.expect("planned engaged actor")),
            rate,
        )
    }
    fn boost_kinds(&self) -> &'static [PotionKind] {
        match self.active {
            Style::Ranged => &[PotionKind::Ranging, PotionKind::SuperDefence],
            Style::Mage => &[PotionKind::Magic, PotionKind::SuperDefence],
            Style::Melee => &[
                PotionKind::SuperAttack,
                PotionKind::SuperStrength,
                PotionKind::SuperDefence,
            ],
        }
    }
    fn boost(&self, frame: &Frame<'_>) -> Option<PotionKind> {
        if !self.request.allow.potions {
            return None;
        }
        let kinds = self.boost_kinds();
        kinds.iter().copied().find(|kind| {
            let Some(family) = self.tables.potion(*kind) else {
                return false;
            };
            let Some(stat) = family.stat else {
                return false;
            };
            let (effective, base) = arbiter::stat(frame, stat.index() as i32);
            let ceiling = base + family.constant + base * family.percent / 100;
            crate::boost_potions::boost_faded(
                f64::from(base),
                f64::from(effective),
                crate::boost_potions::BOOST_FLOOR,
            ) && effective < ceiling
                && arbiter::potion_id(frame, &self.tables, *kind).is_some()
        })
    }
    /// The item the wear repair wants in `slot`: the pending style's kit while a
    /// switch converges, otherwise the active style's (PVP-DESIGN-1026 §4.1).
    fn desired(&self, slot: u8, frame: &Frame<'_>) -> Option<i32> {
        self.desired_for(self.switch.pending().unwrap_or(self.active), slot, frame)
    }
    /// The active style's own kit row, for every helper that runs on `active`.
    fn active_desired(&self, slot: u8, frame: &Frame<'_>) -> Option<i32> {
        self.desired_for(self.active, slot, frame)
    }
    fn wear(&self, plan: &mut TickPlan, frame: &Frame<'_>, tick: u16) {
        if !self.request.allow.equipment || !self.schedule.ready(OpKind::Wear, tick) {
            return;
        }
        let converging = self.switch.pending().is_some();
        for slot in switch::wear_order(converging) {
            if self.wear_skipped(slot) {
                continue;
            }
            // A two-hander and a shield never share a plan: the shield waits
            // until the right hand is observed (PVP-DESIGN-1026 §4.4).
            if converging
                && slot == 5
                && self
                    .desired(3, frame)
                    .is_some_and(|id| !self.worn(frame, 3, id))
            {
                continue;
            }
            if let Some(id) = self.desired(slot, frame) {
                if !self.worn(frame, slot, id)
                    && arbiter::count(frame, id) > 0
                    && !self
                        .pending
                        .iter()
                        .any(|pending| pending.row.kind == RowKind::Wear && pending.row.aux == slot)
                {
                    self.push(plan, PlanRow::new(RowKind::Wear, id, slot));
                }
            }
        }
    }
    /// Whether `id` is worn in `slot` on this frame.
    fn worn(&self, frame: &Frame<'_>, slot: u8, id: i32) -> bool {
        frame
            .equipment
            .iter()
            .any(|row| row.slot == i32::from(slot) && row.def.id == id)
    }
    fn wear_skipped(&self, slot: u8) -> bool {
        let state = (self.prep_failures >> (u32::from(slot) * 3)) & 7;
        state & 4 != 0 && state & 3 >= 2
    }
    fn style_varp(&self) -> Option<i32> {
        if self.active == Style::Ranged {
            self.tables.selected().ranged_mode_varp()
        } else {
            self.tables.melee_mode_varp()
        }
    }

    fn ranged(&self) -> &RangedState {
        let StyleState::Ranged(state) = &self.style_state else {
            unreachable!("ranged mechanics require the ranged style");
        };
        state
    }

    fn ranged_mut(&mut self) -> &mut RangedState {
        let StyleState::Ranged(state) = &mut self.style_state else {
            unreachable!("ranged mechanics require the ranged style");
        };
        state
    }

    fn ammo_count(&self, frame: &Frame<'_>) -> i32 {
        frame
            .inventory
            .iter()
            .chain(frame.equipment)
            .filter(|row| row.def.id == self.ranged().ammo_pick)
            .fold(0i32, |count, row| count.saturating_add(row.count))
    }

    fn ranged_prepare(&mut self, frame: &Frame<'_>, tick: u16) -> bool {
        // A converging switch owns the style's exhaustion (the target style is
        // the answer to it) and a settle holds it for its first `Idle` tick
        // (T1 runs there), so nothing below may abort the engagement meanwhile.
        let weapon = self
            .weapon_for(Style::Ranged, frame)
            .and_then(|id| ranged::weapon(&self.tables, id));
        let prep = self.phase == Phase::Prep;
        let Some(weapon) = weapon else {
            if self.exhaustion_deferred(frame) {
                return true;
            }
            self.finish(
                CombatEnd::Aborted(if prep {
                    AbortReason::PrepFailed(PrepItem::Weapon)
                } else {
                    AbortReason::Unprotected(Unprotected::NoAmmo)
                }),
                tick,
            );
            return false;
        };
        let thrown = ranged::thrown(weapon);
        let slot = if thrown { 3 } else { 13 };
        if self.pending_wear(slot) {
            return true;
        }
        let ammo = match self.pending_ranged_need(Style::Ranged, frame) {
            Some(RangedNeed::Thrown { weapon }) => Some(weapon),
            Some(RangedNeed::Quiver { ammo, .. }) => Some(ammo),
            None => None,
        };
        let usable = ammo.is_some_and(|id| {
            ranged::accepts(&self.tables, weapon, id)
                && frame
                    .equipment
                    .iter()
                    .chain(
                        frame
                            .inventory
                            .iter()
                            .filter(|_| self.request.allow.equipment),
                    )
                    .any(|row| row.def.id == id && row.count > 0)
        });
        let shift = u32::from(slot) * 3;
        let failed = (self.prep_failures >> shift) & 3 >= 2;
        if !usable || failed {
            if self.exhaustion_deferred(frame) {
                return true;
            }
            let reason = if prep {
                AbortReason::PrepFailed(if thrown {
                    PrepItem::Weapon
                } else {
                    PrepItem::Ammo
                })
            } else {
                AbortReason::Unprotected(Unprotected::NoAmmo)
            };
            self.finish(CombatEnd::Aborted(reason), tick);
            return false;
        }
        self.ranged_mut().ammo_pick = ammo.expect("usable ammo");
        true
    }

    fn ranged_choice(&self, frame: &Frame<'_>) -> Option<&api::game_data::RangedModeFact> {
        let weapon = frame
            .equipment
            .iter()
            .find(|row| row.slot == 3 && row.count > 0)?;
        let tab = self.tables.weapon_style(weapon.def.id)?.tab?;
        if frame.combat_tab != self.tables.combat_tab_root(tab) {
            return None;
        }
        self.tables
            .selected()
            .ranged_modes()
            .iter()
            .find(|row| row.tab == tab as u8 && row.mode == self.request.ranged_style as u8)
    }

    fn ranged_readiness_abort(
        &mut self,
        frame: &Frame<'_>,
        plan: Option<&TickPlan>,
    ) -> Option<PrepReadiness> {
        if self.active != Style::Ranged {
            return None;
        }
        if self.phase != Phase::Prep {
            self.ranged_mut().readiness_wait = RangedReadinessWait::Inactive;
            return None;
        }
        let ammo_id = self.ranged().ammo_pick;
        let Some(weapon) = frame
            .equipment
            .iter()
            .find(|row| row.slot == 3 && row.count > 0)
        else {
            self.ranged_mut().readiness_wait = RangedReadinessWait::Inactive;
            return None;
        };
        if ammo_id < 0
            || self
                .active_desired(3, frame)
                .is_some_and(|id| id != weapon.def.id)
            || !frame.equipment.iter().any(|row| {
                row.def.id == ammo_id && row.count > 0 && (row.slot == 3 || row.slot == 13)
            })
            || self.pending_row(RowKind::Wear)
            || plan.is_some_and(|plan| plan.contains(RowKind::Wear))
        {
            self.ranged_mut().readiness_wait = RangedReadinessWait::Inactive;
            return None;
        }

        let expected_root = self
            .tables
            .weapon_style(weapon.def.id)
            .and_then(|style| style.tab)
            .and_then(|tab| self.tables.combat_tab_root(tab));
        let root_matches = expected_root.is_some() && frame.combat_tab == expected_root;
        if root_matches && self.ranged_ready(frame) {
            self.ranged_mut().readiness_wait = RangedReadinessWait::Inactive;
            return None;
        }

        // `counters.ticks` advances once per fresh combat observation; the
        // poll's evidence guard prevents same-tick re-polls aging this wait.
        let since = match (root_matches, self.ranged().readiness_wait) {
            (false, RangedReadinessWait::CombatRoot { since })
            | (true, RangedReadinessWait::StyleEcho { since }) => since,
            _ => self.counters.ticks,
        };
        self.ranged_mut().readiness_wait = if root_matches {
            RangedReadinessWait::StyleEcho { since }
        } else {
            RangedReadinessWait::CombatRoot { since }
        };
        if self.counters.ticks.saturating_sub(since) < PREP_READINESS_TICKS - 1 {
            return None;
        }

        self.ranged_mut().readiness_wait = RangedReadinessWait::Inactive;
        Some(if root_matches {
            PrepReadiness::StyleEchoMissing
        } else if frame.combat_tab.is_none() {
            PrepReadiness::CombatRootMissing
        } else {
            PrepReadiness::CombatRootWrong
        })
    }

    fn ranged_ready(&self, frame: &Frame<'_>) -> bool {
        if self.pending_row(RowKind::Wear)
            || self.active_desired(3, frame).is_some_and(|id| {
                !frame
                    .equipment
                    .iter()
                    .any(|row| row.slot == 3 && row.def.id == id && row.count > 0)
            })
            || !frame.equipment.iter().any(|row| {
                row.def.id == self.ranged().ammo_pick
                    && row.count > 0
                    && (row.slot == 3 || row.slot == 13)
            })
        {
            return false;
        }
        let Some(choice) = self.ranged_choice(frame) else {
            return false;
        };
        self.style_varp().is_some_and(|varp| {
            frame
                .varps
                .iter()
                .any(|row| row.index == varp && row.value == i32::from(choice.slot))
        }) && !self.pending_row(RowKind::Style)
    }

    fn ranged_style(&self, plan: &mut TickPlan, frame: &Frame<'_>, tick: u16) {
        if self.pending_row(RowKind::Wear)
            || self.pending_row(RowKind::Style)
            || plan.contains(RowKind::Style)
            || !self.schedule.ready(OpKind::Style, tick)
        {
            return;
        }
        let Some(choice) = self.ranged_choice(frame) else {
            return;
        };
        if plan.iter().any(|row| {
            row.kind == RowKind::Wear
                && row.aux == 3
                && self
                    .tables
                    .weapon_style(row.id)
                    .and_then(|fact| fact.tab)
                    .map(|tab| tab as u8)
                    != Some(choice.tab)
        }) {
            return;
        }
        if !self.style_varp().is_some_and(|varp| {
            frame
                .varps
                .iter()
                .any(|row| row.index == varp && row.value == i32::from(choice.slot))
        }) {
            self.push(
                plan,
                PlanRow::new(RowKind::Style, choice.button, choice.slot),
            );
        }
    }

    fn ranged_sweep(
        &mut self,
        plan: &mut TickPlan,
        frame: &Frame<'_>,
        tick: u16,
        cx: &ActionContext<'_>,
    ) {
        if self.active != Style::Ranged {
            return;
        }
        if self.end != Some(CombatEnd::Killed)
            || self.request.tactic != Tactic::Open
            || self.ranged().ammo_pick < 0
            || self.ranged().sweep_attempts >= 4
        {
            self.ranged_mut().sweep_done = true;
            return;
        }
        // Killed is latched from observed zero health, not disappearance. Ignore
        // its residual event row, but never ignore a respawn reusing the slot.
        let corpse = self.engaged.filter(|actor| {
            actor.kind == ActorKind::Npc
                && !frame.npcs.iter().any(|npc| {
                    npc.index == usize::from(actor.index)
                        && (npc.total_health <= 0 || npc.health > 0)
                })
        });
        let danger = self.threats.danger_except(
            &self.tables,
            tick,
            (self.shield(frame), self.antifire(tick)),
            select::active_protect(frame, &self.tables),
            corpse,
        );
        if danger != Some(0) {
            self.ranged_mut().sweep_done = true;
            return;
        }
        if self.pending_row(RowKind::Pickup) || !self.schedule.ready(OpKind::Pickup, tick) {
            return;
        }
        let snapshot = cx.snapshot();
        let (Some(ground), Some(reach)) = (snapshot.ground_items(), snapshot.reach()) else {
            return;
        };
        let next = ground
            .value
            .iter()
            .filter(|row| {
                row.def.id == self.ranged().ammo_pick
                    && row.count > 0
                    && distance(frame.here, row.tile) <= 2
                    && reach
                        .value
                        .can_reach(row.tile, &api::query::SceneReachOptions::default())
            })
            .min_by_key(|row| (distance(frame.here, row.tile), row.tile.x, row.tile.z));
        if let Some(item) = next {
            self.push(plan, PlanRow::new(RowKind::Pickup, pack_tile(item.tile), 0));
        } else {
            self.ranged_mut().sweep_done = true;
        }
    }

    fn style(&mut self, plan: &mut TickPlan, frame: &Frame<'_>, tick: u16) {
        // A converging switch emits no style row (PVP-DESIGN-1026 §4.4).
        if self.switch.pending().is_some() {
            return;
        }
        if self.active == Style::Ranged {
            self.ranged_style(plan, frame, tick);
            return;
        }
        let Some(wanted) = self.request.melee_mode else {
            return;
        };
        if plan.contains(RowKind::Style)
            || self.pending_row(RowKind::Style)
            || !self.schedule.ready(OpKind::Style, tick)
        {
            return;
        }
        let worn = if let Some(row) = frame.equipment.iter().find(|row| row.slot == 3) {
            let Some(style) = self.tables.weapon_style(row.def.id) else {
                return;
            };
            let Some(tab) = style.tab else {
                return;
            };
            tab
        } else {
            CombatTab::Unarmed
        };
        let Some(root) = self.tables.combat_tab_root(worn) else {
            return;
        };
        if frame.combat_tab != Some(root) {
            return;
        }
        if plan.iter().any(|row| {
            row.kind == RowKind::Wear
                && row.aux == 3
                && self
                    .tables
                    .weapon_style(row.id)
                    .is_none_or(|style| style.tab != Some(worn))
        }) {
            return;
        }
        // A preceding wear may still be waiting for its new exact tab.
        if self
            .pending
            .iter()
            .any(|pending| pending.row.kind == RowKind::Wear && pending.row.aux == 3)
        {
            return;
        }
        let observed = self
            .tables
            .melee_mode_varp()
            .and_then(|varp| frame.varps.iter().find(|row| row.index == varp))
            .and_then(|row| u8::try_from(row.value).ok());
        let Some(choice) = self.tables.melee_mode(worn, wanted, observed) else {
            return;
        };
        if observed != Some(choice.slot) {
            self.push(
                plan,
                PlanRow::new(
                    RowKind::Style,
                    choice.button,
                    choice.slot | ((choice.actual as u8) << 2),
                ),
            );
        } else {
            if let StyleState::Melee(state) = &mut self.style_state {
                state.mode_fallback = (choice.actual != wanted).then_some(choice.actual);
            }
        }
    }
    fn retaliate(&self, plan: &mut TickPlan, frame: &Frame<'_>, tick: u16) {
        let wanted = self.request.retaliate
            && !(self.active == Style::Mage
                && (self.request.spells.is_some() || self.magic().mode == Some(CastMode::Manual)));
        if !plan.contains(RowKind::Retaliate)
            && self.request.allow.retaliate_toggle
            && retaliate_on(frame) != wanted
            && self.flags & SKIP_RETALIATE == 0
            && self.schedule.ready(OpKind::Retaliate, tick)
            && !self.pending_row(RowKind::Retaliate)
        {
            self.push(plan, PlanRow::new(RowKind::Retaliate, 0, u8::from(wanted)));
        }
    }
    fn target_above_quarter(&self, frame: &Frame<'_>) -> bool {
        self.engaged.is_none_or(|actor| match actor.kind {
            ActorKind::Npc => frame
                .npcs
                .iter()
                .find(|row| row.index == usize::from(actor.index))
                .is_some_and(|row| {
                    row.total_health <= 0 || row.health.saturating_mul(4) > row.total_health
                }),
            ActorKind::Player => frame
                .players
                .iter()
                .find(|row| row.index == usize::from(actor.index))
                .is_some_and(|row| {
                    row.actor.total_health <= 0
                        || row.actor.health.saturating_mul(4) > row.actor.total_health
                }),
        })
    }
    fn leash_cancel_tile(&self, frame: &Frame<'_>) -> Option<api::WorldTile> {
        let stand = self.request.stand?;
        // The fight spot is read from the network tile, as every other
        // arrival check is: a rendered tile that lags behind it would send a
        // walk to a spot the player already reached, and that walk gives no
        // driver movement to cancel the chase.
        if !select::stand_reached(stand, frame.local) {
            return Some(stand);
        }
        let actor = self.engaged?;
        let target = match actor.kind {
            ActorKind::Npc => frame
                .npcs
                .iter()
                .find(|row| {
                    row.index == usize::from(actor.index)
                        && row.r#type.map_or(-1, |id| id as i32) == self.engaged_type()
                })
                .map(|row| row.tile),
            ActorKind::Player => frame
                .players
                .iter()
                .find(|row| {
                    row.index == usize::from(actor.index)
                        && self.player_ident().is_some()
                        && row.actor.name.as_deref().map(select::ident::fnv1a)
                            == self.player_ident()
                })
                .map(|row| row.actor.tile),
        }?;
        // A walk back to an already-reached stand produces no driver movement;
        // move just beyond its radius to cancel the live server chase instead.
        let away_x = (frame.here.x - target.x).signum();
        let away_z = (frame.here.z - target.z).signum();
        [
            (away_x * 2, away_z * 2),
            (away_x * 2, 0),
            (0, away_z * 2),
            (2, 0),
            (-2, 0),
            (0, 2),
            (0, -2),
        ]
        .into_iter()
        .find_map(|(dx, dz)| {
            if dx == 0 && dz == 0 {
                return None;
            }
            let x = frame.here.x + dx;
            let z = frame.here.z + dz;
            ((0..=16383).contains(&x) && (0..=16383).contains(&z)).then_some(api::WorldTile {
                x,
                z,
                level: frame.here.level,
            })
        })
        .or(Some(stand))
    }
    fn approach_walk_pending(&self) -> bool {
        self.approach_walk_pending
    }

    fn set_approach_walk_pending(&mut self, pending: bool) {
        self.approach_walk_pending = pending;
    }

    fn observe_approach_failure(
        &mut self,
        receipt: &crate::native::WalkReceipt,
        tick: u64,
    ) -> bool {
        let retry = self.approach_recovery.observe_receipt(receipt);
        if retry {
            let attempt = self.approach_recovery.retry_attempt_number();
            let detail = receipt.failure_detail();
            api::host_log!(
                api::hostlog::Category::NavTrace,
                api::hostlog::Level::Debug,
                "quester-approach-retry tick={tick} attempt={attempt}/{} end={:?} detail={detail:?}",
                QUEST_WALK_MAX_ATTEMPTS,
                receipt.end,
            );
        }
        retry
    }

    fn approach_action_submitted(&mut self) {
        self.approach_recovery.action_submitted();
        self.set_approach_walk_pending(true);
    }

    fn reset_approach_recovery(&mut self) {
        self.approach_recovery.reset();
        self.set_approach_walk_pending(false);
    }

    fn walk(
        &mut self,
        tile: api::WorldTile,
        tick: u16,
        cx: &mut ActionContext<'_>,
        approach: bool,
    ) -> Result<(), ActionError> {
        let id = cx.walk(WalkRequest {
            target: tile,
            candidate_goals: None,
            loc_id: None,
            radius: 1,
            arrival: nav_planner::contract::ArrivalKind::Reach,
            options: Default::default(),
            required_after: cx.evidence(),
            evidence: None,
            cross: self.request.cross.as_deref().unwrap_or_default().into(),
            protect: false,
            food_guard: false,
            cure_guard: false,
            allow: Default::default(),
            avoid: Box::default(),
        })?;
        self.pending_walk = NonZeroU64::new(id);
        if approach {
            self.approach_action_submitted();
        } else {
            self.reset_approach_recovery();
        }
        self.schedule.clear(tick, 1, false);
        self.flags |= EMITTED;
        self.emitted_tick = tick;
        Ok(())
    }
    fn emit(
        &mut self,
        frame: &Frame<'_>,
        tick: u16,
        cx: &mut ActionContext<'_>,
    ) -> Result<(), ActionError> {
        let mut rows: [Option<InteractReq>; 5] = Default::default();
        let mut baselines = [0i16; 5];
        for (index, row) in self.plan.iter().enumerate() {
            rows[index] = Some(match row.kind {
                RowKind::Eat => {
                    baselines[index] =
                        arbiter::count(frame, row.id).min(i32::from(i16::MAX)) as i16;
                    held(frame.inventory, row.id, "Eat")?
                }
                RowKind::CureFruit => {
                    baselines[index] =
                        arbiter::count(frame, row.id).min(i32::from(i16::MAX)) as i16;
                    held(frame.inventory, row.id, "Eat")?
                }
                RowKind::Drink => {
                    baselines[index] = arbiter::doses(frame, &self.tables, potion_kind(row.aux));
                    held(frame.inventory, row.id, "Drink")?
                }
                RowKind::Pickup => {
                    let tile = unpack_tile(row.id);
                    let item = cx
                        .snapshot()
                        .ground_items()
                        .and_then(|ground| {
                            ground.value.iter().find(|item| {
                                item.tile == tile && item.def.id == self.ranged().ammo_pick
                            })
                        })
                        .ok_or_else(|| unavailable("planned ammo disappeared"))?;
                    InteractReq::Obj {
                        x: tile.x,
                        z: tile.z,
                        level: tile.level,
                        id: Some(item.def.id),
                        name: item.def.name.to_owned(),
                        action: "Take".into(),
                    }
                }
                RowKind::Prayer | RowKind::Style => InteractReq::IfButton {
                    component_id: row.id,
                },
                RowKind::Wear => InteractReq::Wear {
                    name: item_name(frame.inventory, row.id)?.to_owned(),
                },
                RowKind::Retaliate => InteractReq::SetRetaliate { on: row.aux != 0 },
                RowKind::Arm if row.aux & arbiter::ARM_SIDE_TAB_FLAG != 0 => {
                    InteractReq::SideTab { tab: 0 }
                }
                RowKind::Arm => InteractReq::IfButton {
                    component_id: row.id,
                },
                RowKind::Cast => {
                    let actor = self
                        .engaged
                        .ok_or_else(|| unavailable("cast target disappeared"))?;
                    if !select::actor_attackable(frame, actor) {
                        return Err(unavailable("unattackable cast target"));
                    }
                    let (tile, name) = match actor.kind {
                        ActorKind::Npc => frame
                            .npcs
                            .iter()
                            .find(|npc| {
                                npc.index == usize::from(actor.index)
                                    && npc.r#type == usize::try_from(self.engaged_type()).ok()
                            })
                            .map(|npc| (npc.network, npc.name.as_deref())),
                        ActorKind::Player => frame
                            .players
                            .iter()
                            .find(|player| {
                                player.index == usize::from(actor.index)
                                    && player.actor.name.as_deref().map(select::ident::fnv1a)
                                        == self.player_ident()
                            })
                            .map(|player| (player.actor.tile, player.actor.name.as_deref())),
                    }
                    .ok_or_else(|| unavailable("cast target identity changed"))?;
                    let spell = &self.tables.selected().spells()[row.id as usize];
                    InteractReq::UseWidgetOn {
                        component_id: spell.component_id,
                        kind: if actor.kind == ActorKind::Npc {
                            "npc"
                        } else {
                            "player"
                        }
                        .into(),
                        target_name: name.map(str::to_owned),
                        x: tile.x,
                        z: tile.z,
                        level: tile.level,
                        index: Some(i32::from(actor.index)),
                        target_id: None,
                    }
                }
                RowKind::Attack => {
                    let actor = self
                        .engaged
                        .ok_or_else(|| unavailable("target disappeared"))?;
                    if actor_token(actor) != row.id {
                        return Err(unavailable("planned target changed"));
                    }
                    if !select::actor_attackable(frame, actor) {
                        return Err(unavailable("unattackable target"));
                    }
                    match actor.kind {
                        ActorKind::Npc => {
                            let npc = frame
                                .npcs
                                .iter()
                                .find(|row| {
                                    row.index == usize::from(actor.index)
                                        && row.r#type == usize::try_from(self.engaged_type()).ok()
                                })
                                .ok_or_else(|| unavailable("npc identity changed"))?;
                            InteractReq::Npc {
                                name: npc
                                    .name
                                    .as_deref()
                                    .ok_or_else(|| unavailable("npc name"))?
                                    .to_owned(),
                                action: "Attack".into(),
                                index: Some(i32::from(actor.index)),
                            }
                        }
                        ActorKind::Player => {
                            let row = frame
                                .players
                                .iter()
                                .find(|row| {
                                    row.index == usize::from(actor.index)
                                        && self.player_ident().is_some()
                                        && row.actor.name.as_deref().map(select::ident::fnv1a)
                                            == self.player_ident()
                                })
                                .ok_or_else(|| unavailable("player identity"))?;
                            let name = row
                                .actor
                                .name
                                .as_deref()
                                .ok_or_else(|| unavailable("player identity"))?;
                            // The verb is the label the content gave the row:
                            // "Attack" in the Wilderness, "Fight" in a duel.
                            let label = select::attack_label(&row.actor.actions)
                                .ok_or_else(|| unavailable("player attack label"))?;
                            InteractReq::Player {
                                name: name.to_owned(),
                                action: label.into(),
                            }
                        }
                    }
                }
                RowKind::Empty => unreachable!("compact combat plan"),
            });
        }
        let first = cx.emit_batch(rows)?;
        self.plan_first_id = first;
        self.emitted_tick = tick;
        self.flags |= EMITTED;
        if self.phase == Phase::Fight {
            self.flags |= PLAN_FIGHT;
        } else {
            self.flags &= !PLAN_FIGHT;
        }
        if self.plan.contains(RowKind::Pickup) {
            let baseline = self.ammo_count(frame);
            let state = self.ranged_mut();
            state.sweep_attempts += 1;
            state.pickup_baseline = baseline;
        }
        if self.phase == Phase::WindDown && self.plan.contains(RowKind::Eat) {
            self.flags |= TERMINAL_FOOD;
        }
        if self.plan.len >= 2 {
            self.counters.multi = self.counters.multi.saturating_add(1);
        }
        let mut index = 0;
        for (slot, pending) in self.pending.iter_mut().enumerate() {
            if index == usize::from(self.plan.len) {
                break;
            }
            if pending.row.kind == RowKind::Empty {
                *pending = PendingRow {
                    row: self.plan.rows[index],
                    baseline: baselines[index],
                    age: 0,
                };
                self.staged_pending |= 1 << slot;
                index += 1;
            }
        }
        debug_assert_eq!(index, usize::from(self.plan.len));
        Ok(())
    }
}
fn weapon_prep_item(style: Style) -> PrepItem {
    match style {
        Style::Mage => PrepItem::Staff,
        Style::Melee | Style::Ranged => PrepItem::Weapon,
    }
}
fn op_kind(kind: RowKind) -> OpKind {
    match kind {
        RowKind::Eat => OpKind::Eat,
        RowKind::Drink | RowKind::CureFruit => OpKind::Drink,
        RowKind::Prayer => OpKind::Prayer,
        RowKind::Wear => OpKind::Wear,
        RowKind::Style => OpKind::Style,
        RowKind::Retaliate => OpKind::Retaliate,
        RowKind::Attack => OpKind::Attack,
        RowKind::Pickup => OpKind::Pickup,
        RowKind::Cast => OpKind::Cast,
        // Autocast buttons share Style's IfButton cadence; the zero-wire
        // SideTab row is skipped in projected_schedule and settled separately.
        RowKind::Arm => OpKind::Style,
        RowKind::Empty => unreachable!("empty pending operation"),
    }
}
fn potion_kind(code: u8) -> PotionKind {
    match code {
        0 => PotionKind::Prayer,
        1 => PotionKind::SuperAttack,
        2 => PotionKind::SuperStrength,
        3 => PotionKind::SuperDefence,
        4 => PotionKind::Ranging,
        5 => PotionKind::Magic,
        6 => PotionKind::Antifire,
        7 => PotionKind::Antipoison,
        8 => PotionKind::SuperAntipoison,
        _ => unreachable!("planned potion kind"),
    }
}

fn unavailable(reason: &'static str) -> ActionError {
    ActionError::Unavailable(reason.into())
}
fn prayer_bit(varp: i32) -> u16 {
    1 << (varp - api::prayer::PRAYER_VARP0)
}
fn retaliate_on(frame: &Frame<'_>) -> bool {
    frame
        .varps
        .iter()
        .any(|row| row.index == super::OPTION_NODEF && row.value == 0)
}
fn prayer_observation(frame: &Frame<'_>) -> PrayerObservation {
    let mut obs = PrayerObservation::empty();
    (obs.points, obs.max) = arbiter::stat(frame, 5);
    for row in frame.varps {
        obs.set_varp(row.index, row.value);
    }
    obs
}
fn item_name(items: &[ItemView], id: i32) -> Result<&str, ActionError> {
    items
        .iter()
        .find(|row| row.def.id == id)
        .and_then(|row| row.def.name.as_deref())
        .ok_or_else(|| unavailable("item name"))
}
fn held(items: &[ItemView], id: i32, action: &str) -> Result<InteractReq, ActionError> {
    let row = items
        .iter()
        .find(|row| row.def.id == id && row.count > 0)
        .ok_or_else(|| unavailable("held item"))?;
    Ok(InteractReq::Held {
        name: item_name(items, id)?.to_owned(),
        action: action.to_owned(),
        slot: Some(row.slot),
        target_item_id: None,
    })
}
fn distance(a: api::WorldTile, b: api::WorldTile) -> i32 {
    if a.level != b.level {
        i32::MAX
    } else {
        (a.x - b.x).abs().max((a.z - b.z).abs())
    }
}
fn pack_tile(tile: api::WorldTile) -> i32 {
    (tile.x & 0x3fff) | ((tile.z & 0x3fff) << 14) | ((tile.level & 3) << 28)
}
fn unpack_tile(packed: i32) -> api::WorldTile {
    api::WorldTile {
        x: packed & 0x3fff,
        z: (packed >> 14) & 0x3fff,
        level: (packed >> 28) & 3,
    }
}

#[cfg(test)]
#[path = "machine_tests.rs"]
mod tests;

fn actor_token(actor: ActorRef) -> i32 {
    i32::from(actor.index)
        | if actor.kind == ActorKind::Player {
            1 << 16
        } else {
            0
        }
}
