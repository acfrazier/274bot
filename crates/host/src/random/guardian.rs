use super::*;

use api::hostlog::{Category, Level};
use api::random::{
    FightActor, FightCancel, FightEscape, FightEvent, FightOffer, FightOutcome, FightPhase,
    FightReason, FightToken, RandomResponse, ResponseReason, ResponseState,
};
mod fight;
pub use fight::{EscapeIntent, SentEscape};
use fight::{FightDesk, Response};
/// `map_findsquare(coord, 1, 7, ...)` finds an eligible ring tile, or returns
/// `coord` when no ring tile qualifies. Recovery checks distance 0 through 7
/// from both observed event-origin candidates (`ServerOps.ts:298-335`).
const LOST_TOOL_RADIUS: i32 = 7;
/// The content event can run before or after player movement. Keep the last
/// pre-event tile and the tile from the first event observation as origins.
/// Content adds the head at E+1 or E+2: `p_delay` receives 0 or 1 for the
/// 1–7-tile projectile range. The client can read only five packets per frame,
/// and OBJ_ADD follows PLAYER_INFO. Add one delivery tick to the two content
/// ticks: 2 + 1 = 3 (`macro_event_lost_axe.rs2:18,25-27`;
/// `macro_event_lost_pickaxe.rs2:20,25-29`; `projectile.rs2:31-35`;
/// `PlayerOps.ts:376-379`; `Zone.ts:208-219`;
/// `client/src/client/client.rs:12744`).
const LOST_TOOL_SPAWN_GRACE_TICKS: u32 = 3;
/// The lost-tool scripts pass `lootdrop_duration = 200` to `obj_add`; each
/// `Obj.turn` decrements its lifecycle (`lootdrop.constant:1`;
/// `macro_event_lost_axe.rs2:27`; `macro_event_lost_pickaxe.rs2:29`;
/// `World.ts:1485-1494`; `Obj.ts:27-43`).
const LOST_TOOL_GROUND_LIFETIME_TICKS: u32 = 200;
/// Hostile cleanup runs every 20 NPC ticks. NPC processing precedes player
/// movement, and the angry plant's death script delays deletion one tick:
/// allow those two phases when an escaped NPC is outside the client view
/// (`antimacro.npc:130,1009`; `World.ts:340-408`;
/// `macro_event_triffid.rs2:51-55`).
const EVADE_CLEANUP_TICKS: u8 = 22;
/// Final failure budget: the normal hostile spawn's 1000-tick lifetime plus
/// two observation ticks (`macro_events.rs2:36`; `macro_event_dryad.rs2:10`).
/// Missing/disengaged actors can need this long. Engaged actors inside chase
/// range have a shorter no-distance-progress budget below. Neither rule proves
/// despawn: the angry plant extends its lifetime (`macro_event_triffid.rs2:35`).
const EVADE_GIVE_UP_TICKS: u16 = 1_002;
/// A thieving stun blocks every walk at the server. The fail script applies
/// the stun for the pickpocket row's `stun_ticks` (Man 8:
/// `pickpocket.dbrow:10`; the largest row is 10: `pickpocket.dbrow:145`) and
/// arms the `stunned` walktrigger (`thieving.rs2:48-51`). Each movement
/// attempt inside that window receives the second message and walks the
/// player onto its own tile (`movecheck.rs2:1-6`). A flee tick the server
/// blocked this way is not distance-progress evidence.
const STUNNED_MARKERS: &[&str] = &["You've been stunned!", "You're stunned!"];
/// NPC packets use the preferred 15-tile radius (`info.ts:310`, `build.ts:57`).
const NPC_VIEW_RADIUS: i32 = 15;
/// Open has one `p_delay(0)` before its modal. Answers close the modal and
/// consume/replicate a box without a delay. Allow the input-processing tick,
/// that one delayed tick, and one packet-delivery tick, counted only through
/// PLAYER_INFO (`macro_event_strange_box.rs2:231-275,298-304`;
/// `PlayerOps.ts:376-379`; client packet drain: `client.rs:12744`).
const BOX_OUTCOME_TICKS: u32 = 3;
/// Chosen bound: one attempt plus two retries tolerates transient missing
/// outcomes without holding the script indefinitely. Each attempt waits for
/// Open's modal or the answer's inventory/server-message result within
/// BOX_OUTCOME_TICKS. Content does not prescribe the number of retries.
const BOX_MAX_FAILURES: u8 = 3;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum BoxPhase {
    #[default]
    Ready,
    Opening {
        tick: u32,
    },
    Answering {
        tick: u32,
        count: i32,
        sequence: i32,
    },
    Retry,
    Stalled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BoxFailure {
    OpenTimeout,
    AnswerTimeout,
    WrongAnswer,
    MissingModels,
    UnknownQuestion,
    OpenRefused(api::interact::SendReason),
    AnswerRefused,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct BoxSolve {
    phase: BoxPhase,
    failures: u8,
    last_failure: Option<BoxFailure>,
}

/// The chrome contract both views bind (guardian spec `RandomStatus`):
/// published every tick on the slot status row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RandomStatus {
    /// The detected event; `None` when the snapshot shows nothing.
    pub kind: Option<RandomKind>,
    pub name: Option<String>,
    /// An owned event pending handling. Detect-only events still publish
    /// kind/name, but cannot freeze scripts through their pending signal.
    pub ours: bool,
    /// A dialog handle is in flight (the host is talking it through).
    pub handling: bool,
    /// The slot must skip script tick / follow (host-play enforces the
    /// freeze). A dialog handle in flight or a trapped kind holds, while
    /// the claim is Host and the toggle is on.
    pub hold: bool,
    pub toggle: bool,
    pub claim: RandomClaim,
    /// The shown NPC slot is in the 45 s wrong-talk bin.
    pub cooldown: bool,
    /// What the Guardian is doing about the event, in typed form: the one row
    /// both frontends render (design V11D 7.1). `None` while nothing is
    /// detected or the event has no response worth showing.
    pub response: Option<RandomResponse>,
    /// The fight the Guardian waits on, for the slot's dispatch gate: only
    /// gameplay carrying this token may pass the Guardian hold.
    pub fight: Option<FightToken>,
}

impl Default for RandomStatus {
    fn default() -> Self {
        Self {
            kind: None,
            name: None,
            ours: false,
            handling: false,
            hold: false,
            toggle: false,
            claim: RandomClaim::Host,
            cooldown: false,
            response: None,
            fight: None,
        }
    }
}
/// Per-frame inputs that change independently of a slot's static profile.
pub struct GuardianLiveContext<'a> {
    /// Live per-event choices; the profile arm updates this map.
    pub event_policies: &'a BTreeMap<String, RandomEventPolicy>,
    /// Stats from the bound client revision; missing data must not guess.
    pub game_data: Option<&'a SelectedGameData>,
}
/// One explicit ownership-probe machine. Identity-scoped rejected actors are
/// retained separately so they cannot shadow a different probe candidate.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) enum PlantProbe {
    #[default]
    Idle,
    AwaitingResponse {
        actor: PlantActor,
        deadline_ms: u64,
    },
    Authenticated {
        actor: PlantActor,
        retry_at_ms: u64,
        deadline_ms: u64,
        continues: u32,
    },
}

impl PlantProbe {
    fn actor(&self) -> Option<&PlantActor> {
        match self {
            Self::Idle => None,
            Self::AwaitingResponse { actor, .. } | Self::Authenticated { actor, .. } => Some(actor),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct EvadeSighting {
    index: usize,
    npc_type: Option<usize>,
    tile: (i32, i32),
}

/// The detector's history of one hostile macro-event actor: its identity,
/// the content facts of its chase, its spawn anchor and its escape clock. It
/// outlives an episode (a retained actor keeps its deadline through an
/// overlapping trap) and holds no command authority: the flee in flight is the
/// active event's [`EvadeRun`].
///
/// The escape clock is the content's, not the host's: the engaged actor keeps
/// its target and attacks a player who makes no outward progress from its
/// spawn anchor, whoever moves the player (`Npc.ts:77-85,661-670`), and the
/// hostile lifetime is `macro_events.rs2:36`. It runs on every fresh
/// observation whoever owns the event and whatever the toggle says.
#[derive(Debug, Clone, PartialEq, Eq)]
struct EvadeTrack {
    index: usize,
    npc_type: Option<usize>,
    name: String,
    /// Selected content's NPC `maxrange`, not a guessed flee ring.
    maxrange: Option<i32>,
    /// Clear both chase (`maxrange+1`) and wandering cleanup (`range>3`).
    flee_distance: i32,
    /// Earliest scene sighting, before the delayed spawn acquires its target.
    /// If observation starts late, ongoing targeting overrides this estimate.
    start_tile: (i32, i32),
    /// Latest actor position distinguishes view-edge removal from despawn.
    last_tile: (i32, i32),
    /// Outside-range targeted ticks; one NPC phase can trail player movement.
    outside_ticks: u8,
    /// Consecutive stationary ticks outside chase range with no actor in view.
    missing_ticks: u8,
    /// Fresh observations since the sighting; never reset by view flicker.
    elapsed_ticks: u16,
    /// Greatest observed distance from the spawn anchor.
    furthest_distance: i32,
    /// Consecutive fresh ticks without a new maximum while engaged in range.
    no_progress_ticks: u16,
    /// Detect-only after the give-up limit, until this actor is removed: the
    /// released actor's suppression.
    gave_up: bool,
    /// A flee walk was accepted: a later removal beyond view is not despawn
    /// evidence.
    has_fled: bool,
    policy_logged: bool,
}

/// The flee in flight. It lives in the active event and ends with it; the next
/// hostile actor inherits it with the act.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct EvadeRun {
    /// The tracked actor (client NPC slot, cache type) the walks below flee
    /// from. A successor actor keeps `flee_from` and starts its own walks.
    actor: Option<(usize, Option<usize>)>,
    /// The tile the flee started from: the walk-back target once the threat
    /// despawns (rs2b0t).
    flee_from: Option<(i32, i32)>,
    flee_target: Option<(i32, i32)>,
    fallback_distance: Option<i32>,
    fallback_exhausted: bool,
    /// How the Guardian answers the event: the escape solver, or the fight a
    /// child runs (design V11D 3.2).
    response: Response,
    /// The escape follows a fight that handed over, and its counters started
    /// at that handoff.
    from_fight: bool,
    /// The child whose upkeep guard serves the walk out: staged walks go
    /// through it so no click can race an admitted input lock.
    escape: Option<FightToken>,
    /// An escape walk is staged and has not come back from the observation.
    staged: bool,
    /// An admitted input lock blocks every click before this tick (low 16
    /// bits, wrap-aware).
    lock_until: Option<u16>,
    /// Why the fight handed over, for the status row.
    why: Option<ResponseReason>,
}

/// One lost-tool recovery in flight: the content window and target, the
/// worn-handle evidence for the re-wield, and the refit's result token. It
/// lives in the active event; the stall latch that outlives it is
/// `Guardian::tool_stalled`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct LostToolRun {
    recovery: Option<LostToolRecovery>,
    /// The handle was worn, so the reattached tool is re-wielded.
    was_worn: bool,
    /// The handle's base tool name (the handle name minus the "handle"
    /// suffix), the re-wield target after reattach.
    handle_base: Option<String>,
    /// The chat sequence before the most recent sent refit use: only a
    /// rejection after it is this refit's result.
    refit_after_sequence: Option<i32>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct LostToolItem {
    id: i32,
    count: i32,
    kind: bool,
    head_id: Option<i32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct LostToolGroundHead {
    id: i32,
    tile: (i32, i32, i32),
    count: i32,
    kind: bool,
}

#[derive(Debug, Default)]
struct LostToolBaseline {
    tick: Option<u32>,
    tile: Option<(i32, i32, i32)>,
    tools: Vec<LostToolItem>,
    ground_heads: Vec<LostToolGroundHead>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct LostToolOnset {
    tick: u32,
    kind: bool,
    pre_event_tile: Option<(i32, i32, i32)>,
    observed_tile: Option<(i32, i32, i32)>,
    expected_head_id: Option<i32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct LostToolTarget {
    id: i32,
    tile: (i32, i32, i32),
    spawn_tick: u32,
    missing_since_tick: Option<u32>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
struct LostToolRecovery {
    event_tick: u32,
    kind: bool,
    expected_head_id: Option<i32>,
    origin_tiles: [Option<(i32, i32, i32)>; 2],
    target: Option<LostToolTarget>,
}
/// Who handles the current event. A claim is held by exactly one owner: the
/// host Guardian, or the running script. The native Guardian solver and its
/// children are host work, never a third owner. A script claim names the
/// runtime that made it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Owner {
    Host,
    Script { runtime: Option<ClaimRuntime> },
}

/// A Talk-to went out and the dialog may still be open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DialogHandle {
    /// The NPC slot the in-flight dialog targets (the cooldown key). Client
    /// NPC slots are below `u16::MAX`.
    pub(crate) index: u16,
    /// Continues sent for the in-flight dialog ([`MAX_CONTINUES`] cap).
    pub(crate) continues: u8,
}

/// The identity of one observed episode: the slot session, the detected
/// family and a generation that every new signature (a new event, a replaced
/// event, or none) advances. The host replaces the Guardian at every session
/// boundary and numbers the sessions, so a key from an earlier session never
/// equals one of this session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct EpisodeKey {
    pub(crate) session: u32,
    /// The detected family at the signature edge; `None` before any event.
    pub(crate) kind: Option<RandomKind>,
    pub(crate) generation: u32,
}

/// The Maze visit's bounded solver state. It ends with its episode.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct MazeVisit {
    /// The active solve, `None` while trapped without a route.
    solve: Option<maze::MazeSolve>,
    /// Solver ticks spent on this visit ([`maze::SOLVE_TICKS`] cap).
    ticks: u32,
    /// The solver gave this visit up with the player still on the square: no
    /// act, no hold and no `ours` until the player leaves it, so the slot's
    /// native run sees the trap and blocks instead of staying frozen.
    stalled: bool,
}

/// The Mime stage's watch and answer state. It ends with its episode.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct MimeStage {
    /// The last emote anim seq the mime NPC showed.
    last_seen: Option<i32>,
    /// The emote button went out for the open chat: no repeat press until the
    /// chat closes.
    answered: bool,
}

/// One lamp redemption in flight (Rub, skill, Confirm, award). Its attempt
/// history, which outlives the episode, is [`LampHistory`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct LampFlow {
    /// Rub went out: do not Rub again until the skill IF closes.
    rubbed: bool,
    /// The skill button went out: the next tick on the IF presses confirm
    /// (2831).
    skill_sent: bool,
    /// Confirm went out: the redemption is in flight (the server closes the
    /// IF, consumes the lamp, advances the skill and opens the award
    /// dialogue).
    confirmed: bool,
    /// The configured skill's (xp, base) as the Confirm tick's snapshot read
    /// them: the reward is witnessed by the advance past this baseline, never
    /// by a later stale read.
    reward: Option<(i32, i32)>,
    /// Award-dialogue continues sent since Confirm ([`MAX_LAMP_DIALOGUE`]).
    dialog: u32,
    /// Ticks the current phase has waited without progress
    /// ([`MAX_LAMP_WAIT`] cap).
    wait: u32,
    /// The lamp count when Confirm went out. One unstacked lamp leaves the
    /// pack per redemption, so the consume is a drop below this baseline.
    confirm_count: Option<i32>,
}

/// The bounded phase, pending work and outcome evidence of the family that
/// works the event (design 3.2). It lives and dies with the active event.
/// State that has to outlive an episode is not here: see the history cells of
/// [`Guardian`], none of which can emit an action or publish a hold.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) enum Family {
    #[default]
    None,
    Box(BoxSolve),
    Maze(MazeVisit),
    Mime(MimeStage),
    Lamp(LampFlow),
    /// The flee in flight.
    Evade(EvadeRun),
    /// The lost-tool recovery in flight.
    LostTool(LostToolRun),
    /// Strange Plant's bounded server-authenticated ownership probe.
    Pick(PlantProbe),
}

impl Family {
    /// The family gave up this episode: detect-only, no pending operation, no
    /// `ours` and no hold until the episode ends. A stalled Box ends when its
    /// event leaves or the server replicates it; a stalled Maze when the
    /// player leaves the square.
    fn released(&self) -> bool {
        match self {
            Self::Box(solve) => solve.phase == BoxPhase::Stalled,
            Self::Maze(visit) => visit.stalled,
            Self::None
            | Self::Mime(_)
            | Self::Lamp(_)
            | Self::Evade(_)
            | Self::LostTool(_)
            | Self::Pick(_) => false,
        }
    }

    /// Nothing worth keeping: the state of a family that has not started.
    fn is_idle(&self) -> bool {
        match self {
            Self::None => true,
            Self::Box(solve) => *solve == BoxSolve::default(),
            Self::Maze(visit) => *visit == MazeVisit::default(),
            Self::Mime(stage) => *stage == MimeStage::default(),
            Self::Lamp(flow) => *flow == LampFlow::default(),
            Self::Evade(run) => *run == EvadeRun::default(),
            Self::LostTool(run) => *run == LostToolRun::default(),
            Self::Pick(probe) => *probe == PlantProbe::Idle,
        }
    }
}

/// The one Guardian event owner (design 3.2). It carries the episode, the
/// owner, the host work that owner has in flight and the family's bounded
/// phase; `claim`, `handling`, `acting`, `ours` and `hold` are derived from it
/// and from the enablement inputs. There is no second copy of any of them.
/// `Guardian::active` is `None` exactly while the Host owns the event and has
/// nothing in flight, nothing released and nothing guarded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ActiveEvent {
    /// The episode the event was opened for.
    pub(crate) key: EpisodeKey,
    pub(crate) owner: Owner,
    /// A dialog handle is in flight (the host is talking it through).
    pub(crate) dialog: Option<DialogHandle>,
    /// A non-dialog act is in flight: an op or walk was sent for the current
    /// event and it has not resolved (evade/plant/hazard/lamp-rub/lost-gear/
    /// lost-tool, plus the out-of-range walk before a Talk-to / Pick). Holds
    /// the slot like the dialog handle. The value is the kind that act
    /// belongs to.
    pub(crate) solver: Option<RandomKind>,
    /// The Ent guard holds the slot this pass.
    pub(crate) guard: bool,
    /// The live Ent guard (latched slot, held ticks, stop sent). Only its
    /// expired-slot exclusion outlives it, in `Guardian::ent`.
    pub(crate) ent: Option<ent::EntLatch>,
    /// The family's bounded phase, progress and outcome evidence.
    pub(crate) family: Family,
}

impl ActiveEvent {
    /// A Host-owned shell with nothing in flight.
    fn new(key: EpisodeKey) -> Self {
        Self {
            key,
            owner: Owner::Host,
            dialog: None,
            solver: None,
            guard: false,
            ent: None,
            family: Family::None,
        }
    }

    /// The Host owns the event and has nothing to keep.
    fn is_spent(&self) -> bool {
        self.owner == Owner::Host
            && self.dialog.is_none()
            && self.solver.is_none()
            && !self.guard
            && self.ent.is_none()
            && self.family.is_idle()
    }
}

/// The enablement and readiness inputs of one publication besides the active
/// event: the toggle and what the detector reports. Suppression history
/// (a parked lamp, a stalled tool) is already part of `detected_ours`.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PublicationInputs {
    /// The random-event toggle.
    pub(crate) toggle: bool,
    /// The detector says the event is owned work (`DetectedRandom::ours`) and
    /// no suppression history says the host gave up on it.
    pub(crate) detected_ours: bool,
    /// The detected event is a trapped kind (Maze, Mime, Box).
    pub(crate) trapped: bool,
}

/// The ownership fields of the status row, and `acting`. They are computed
/// here and nowhere else, from the one active event and the inputs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Publication {
    pub(crate) claim: RandomClaim,
    pub(crate) handling: bool,
    pub(crate) acting: bool,
    pub(crate) ours: bool,
    pub(crate) hold: bool,
}

pub(crate) fn derive_publication(
    active: Option<&ActiveEvent>,
    inputs: PublicationInputs,
) -> Publication {
    let claim = match active.map(|active| active.owner) {
        Some(Owner::Script { .. }) => RandomClaim::Handle,
        Some(Owner::Host) | None => RandomClaim::Host,
    };
    let handling = active.is_some_and(|active| active.dialog.is_some());
    let acting = active.is_some_and(|active| active.solver.is_some());
    let guard = active.is_some_and(|active| active.guard);
    let released = active.is_some_and(|active| active.family.released());
    Publication {
        claim,
        handling,
        acting,
        // Detection alone must not latch pending work: both native
        // eligibility and EventSignal.pending use hold OR ours.
        ours: inputs.toggle && !released && inputs.detected_ours,
        hold: inputs.toggle
            && claim == RandomClaim::Host
            && !released
            && (handling || acting || guard || inputs.trapped),
    }
}

/// The lamp's attempt history. It outlives the episode: a Confirm is never
/// replayed, so the stall latch and the Rub budget survive another family's
/// event until the lamp leaves the pack or `lamp_auto` goes off. It holds no
/// command authority: it only suppresses `ours` and the next Rub.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct LampHistory {
    /// Rub attempts sent in the current redemption (re-Rubs after a missed
    /// skill IF, [`MAX_LAMP_RUBS`] cap; reset when a pause starts a new
    /// episode and when the redemption resolves).
    rubs: u32,
    /// Fresh ticks left in a pause before the next Rub. Inert while it counts
    /// down (no act, no hold, no `ours`), so the script runs between Rub
    /// attempts.
    park: u32,
    /// The redemption failed after Confirm (or the skill is unclickable) with
    /// the lamp still held: no act and no hold (the status row still detects
    /// it) until the lamp leaves the pack or `lamp_auto` is switched off.
    stalled: bool,
}

/// Per-slot random-event guardian state.
pub struct Guardian {
    /// The snapshot tick the last act ran on (one scan per game tick).
    pub last_tick: u64,
    /// NPC slot → wrong-talk cooldown expiry `now_ms`.
    pub cooldown: CooldownMap,
    /// Signature (kind+name) of the last detected event: the rising-edge
    /// key the `on_random` knock fires on once per new event.
    pub sig: Option<String>,
    /// The one owner of the current event and the host work in flight for it.
    /// Filled by the rising-edge knock (host-play's script); `None` is the
    /// Host with nothing in flight: no event, no script, or the script did
    /// not claim it.
    active: Option<ActiveEvent>,
    /// The episode of the last signature edge. A new generation starts at
    /// every new, replaced or vanished event.
    episode: EpisodeKey,
    /// The inputs of the last publication, for the conformance tests.
    #[cfg(test)]
    last_inputs: PublicationInputs,
    /// Newest chat sequence already scanned for wrong-talk lines, so a
    /// stale rejection line cannot re-bin a later NPC.
    chat_seen: i32,
    /// Detector history of the current hostile event NPC: identity, chase
    /// range, spawn anchor and the content's escape clock.
    evade: Option<EvadeTrack>,
    /// First scene positions survive the spawn's delayed targeting/movement.
    evade_starts: Vec<EvadeSighting>,
    /// The Ent guard's expired-slot exclusion. The live latch and the hold it
    /// asks for are `ActiveEvent::ent` and `ActiveEvent::guard`.
    ent: ent::EntGuard,
    /// Last pre-event tool inventory, ground heads, and player tile.
    tool_baseline: LostToolBaseline,
    /// First loss evidence from a removed tool or a handle update. It is
    /// kept while the handle stays held, so a recovery that begins late
    /// measures the content window from the loss and searches its origin.
    tool_onset: Option<LostToolOnset>,
    /// One bounded recovery per handle observation; failure releases the
    /// native Gatherer's missing-tool supply trip instead of holding forever.
    tool_stalled: bool,
    /// The lamp's attempt history (outlives the episode).
    lamp: LampHistory,
    /// Exact foreign/refused/timed-out plant identities still present in the
    /// scene. The probe in flight is `Family::Pick`.
    plant_ignored: Vec<PlantActor>,
    /// Per-slot fishing-gear held/lost history used to prove `LostGear`.
    gear_loss: GearLoss,
    /// Fight tokens minted this session.
    fight_serial: u32,
    /// The actor (client slot, cache type) whose Fight is spent in this
    /// episode: handed over, cancelled, won or refused. It never restarts.
    fight_spent: Option<(usize, Option<usize>)>,
    /// The current episode began under a saved Fight for a hostile event:
    /// Host ownership holds to its release, whatever the setting becomes.
    episode_fight: bool,
    /// The actor (client slot, cache type) of the Fight child the claim source
    /// holds, when no release has gone out for it: the latest offered token
    /// (`fight_serial`) names it. A pass that finds no Evade run holding it
    /// revokes it and spends that actor's Fight, so no child outlives the run
    /// and none restarts for the same actor.
    fight_open: Option<(usize, Option<usize>)>,
}

impl Default for Guardian {
    fn default() -> Self {
        Self::new()
    }
}
impl Guardian {
    pub fn new() -> Self {
        Self::in_session(0)
    }

    /// A Guardian for slot session `session` (the host numbers them).
    pub fn in_session(session: u32) -> Self {
        Self {
            last_tick: 0,
            cooldown: HashMap::new(),
            sig: None,
            active: None,
            episode: EpisodeKey {
                session,
                kind: None,
                generation: 0,
            },
            #[cfg(test)]
            last_inputs: PublicationInputs::default(),
            chat_seen: 0,
            evade: None,
            evade_starts: Vec::new(),
            ent: ent::EntGuard::new(),
            tool_baseline: LostToolBaseline::default(),
            tool_onset: None,
            tool_stalled: false,
            lamp: LampHistory::default(),
            plant_ignored: Vec::new(),
            gear_loss: GearLoss::new(),
            fight_serial: 0,
            fight_spent: None,
            episode_fight: false,
            fight_open: None,
        }
    }

    /// Who handles the current event: derived from the one active event.
    pub fn claim(&self) -> RandomClaim {
        match self.active.as_ref().map(|active| active.owner) {
            Some(Owner::Script { .. }) => RandomClaim::Handle,
            Some(Owner::Host) | None => RandomClaim::Host,
        }
    }

    /// A dialog handle is in flight (the host is talking it through).
    pub fn in_flight(&self) -> bool {
        self.active
            .as_ref()
            .is_some_and(|active| active.dialog.is_some())
    }

    /// A non-dialog act is in flight.
    pub fn acting(&self) -> bool {
        self.acting_kind().is_some()
    }

    /// The kind of the non-dialog act in flight.
    fn acting_kind(&self) -> Option<RandomKind> {
        self.active.as_ref().and_then(|active| active.solver)
    }

    fn dialog(&self) -> Option<DialogHandle> {
        self.active.as_ref().and_then(|active| active.dialog)
    }

    fn dialog_mut(&mut self) -> Option<&mut DialogHandle> {
        self.active
            .as_mut()
            .and_then(|active| active.dialog.as_mut())
    }

    /// The key of the episode being observed.
    fn key(&self) -> EpisodeKey {
        self.episode
    }

    /// The act of `kind` in flight continues for the current episode: the
    /// same family took over from a replaced event.
    fn adopt_episode(&mut self, kind: RandomKind) {
        let episode = self.episode;
        if let Some(active) = self
            .active
            .as_mut()
            .filter(|active| active.solver == Some(kind))
        {
            active.key = episode;
        }
    }

    /// Hand the current event to `claim`, made under script runtime
    /// `runtime`. Host work already in flight survives a change of owner.
    fn set_claim(&mut self, claim: RandomClaim, runtime: Option<ClaimRuntime>) {
        match claim {
            RandomClaim::Host => {
                if let Some(active) = self.active.as_mut() {
                    active.owner = Owner::Host;
                }
            }
            RandomClaim::Handle => self.active_mut().owner = Owner::Script { runtime },
        }
        self.settle();
    }

    /// The script runtime that made the current claim, if a script owns it.
    fn claim_runtime(&self) -> Option<Option<ClaimRuntime>> {
        match self.active.as_ref()?.owner {
            Owner::Script { runtime } => Some(runtime),
            Owner::Host => None,
        }
    }

    /// The Host has nothing to keep and owns the event: no active event.
    fn settle(&mut self) {
        if self.active.as_ref().is_some_and(ActiveEvent::is_spent) {
            self.active = None;
        }
    }

    fn active_mut(&mut self) -> &mut ActiveEvent {
        let key = self.key();
        self.active.get_or_insert_with(|| ActiveEvent::new(key))
    }

    /// A non-dialog act starts for `kind`.
    fn begin_solver(&mut self, kind: RandomKind) {
        self.active_mut().solver = Some(kind);
    }

    /// The non-dialog act ended or released.
    fn end_solver(&mut self) {
        if let Some(active) = self.active.as_mut() {
            active.solver = None;
        }
        self.settle();
    }

    /// A Talk-to went out for NPC slot `index`.
    fn begin_dialog(&mut self, index: usize) {
        self.active_mut().dialog = Some(DialogHandle {
            index: u16::try_from(index).unwrap_or(u16::MAX),
            continues: 0,
        });
    }

    /// Store the live Ent latch the guard left, and whether it holds the slot
    /// this pass.
    fn set_ent(&mut self, latch: Option<ent::EntLatch>, hold: bool) {
        if latch.is_some() || hold {
            let active = self.active_mut();
            active.ent = latch;
            active.guard = hold;
        } else if let Some(active) = self.active.as_mut() {
            active.ent = None;
            active.guard = false;
        }
        self.settle();
    }

    /// The family state, or the idle one while nothing is active.
    fn family(&self) -> &Family {
        self.active
            .as_ref()
            .map_or(&Family::None, |active| &active.family)
    }

    /// End the family state that belongs to `kind`, if it is the one active.
    fn end_family(&mut self, kind: RandomKind) {
        if let Some(active) = self.active.as_mut() {
            let matches = matches!(
                (&active.family, kind),
                (Family::Box(_), RandomKind::Box)
                    | (Family::Maze(_), RandomKind::Maze)
                    | (Family::Mime(_), RandomKind::Mime)
                    | (Family::Lamp(_), RandomKind::Lamp)
                    | (Family::Evade(_), RandomKind::Evade)
                    | (Family::LostTool(_), RandomKind::LostTool)
                    | (Family::Pick(_), RandomKind::Pick)
            );
            if matches {
                active.family = Family::None;
            }
        }
        self.settle();
    }

    /// End whatever family state is active (the event resolved).
    fn end_any_family(&mut self) {
        if let Some(active) = self.active.as_mut() {
            active.family = Family::None;
        }
        self.settle();
    }

    fn box_solve(&self) -> BoxSolve {
        match self.family() {
            Family::Box(solve) => *solve,
            _ => BoxSolve::default(),
        }
    }

    fn box_mut(&mut self) -> &mut BoxSolve {
        let family = &mut self.active_mut().family;
        if !matches!(family, Family::Box(_)) {
            *family = Family::Box(BoxSolve::default());
        }
        let Family::Box(solve) = family else {
            unreachable!("the family was just made a box")
        };
        solve
    }

    fn maze_mut(&mut self) -> &mut MazeVisit {
        let family = &mut self.active_mut().family;
        if !matches!(family, Family::Maze(_)) {
            *family = Family::Maze(MazeVisit::default());
        }
        let Family::Maze(visit) = family else {
            unreachable!("the family was just made a maze visit")
        };
        visit
    }

    fn mime_stage(&self) -> MimeStage {
        match self.family() {
            Family::Mime(stage) => *stage,
            _ => MimeStage::default(),
        }
    }

    fn mime_mut(&mut self) -> &mut MimeStage {
        let family = &mut self.active_mut().family;
        if !matches!(family, Family::Mime(_)) {
            *family = Family::Mime(MimeStage::default());
        }
        let Family::Mime(stage) = family else {
            unreachable!("the family was just made a mime stage")
        };
        stage
    }

    fn lamp_flow(&self) -> LampFlow {
        match self.family() {
            Family::Lamp(flow) => *flow,
            _ => LampFlow::default(),
        }
    }

    fn lamp_flow_mut(&mut self) -> &mut LampFlow {
        let family = &mut self.active_mut().family;
        if !matches!(family, Family::Lamp(_)) {
            *family = Family::Lamp(LampFlow::default());
        }
        let Family::Lamp(flow) = family else {
            unreachable!("the family was just made a lamp flow")
        };
        flow
    }

    fn evade_run(&self) -> EvadeRun {
        match self.family() {
            Family::Evade(run) => *run,
            _ => EvadeRun::default(),
        }
    }

    fn evade_run_mut(&mut self) -> &mut EvadeRun {
        let family = &mut self.active_mut().family;
        if !matches!(family, Family::Evade(_)) {
            *family = Family::Evade(EvadeRun::default());
        }
        let Family::Evade(run) = family else {
            unreachable!("the family was just made an evade run")
        };
        run
    }

    fn tool_run(&self) -> Option<&LostToolRun> {
        match self.family() {
            Family::LostTool(run) => Some(run),
            _ => None,
        }
    }

    fn tool_run_mut(&mut self) -> &mut LostToolRun {
        let family = &mut self.active_mut().family;
        if !matches!(family, Family::LostTool(_)) {
            *family = Family::LostTool(LostToolRun::default());
        }
        let Family::LostTool(run) = family else {
            unreachable!("the family was just made a lost-tool run")
        };
        run
    }

    fn tool_recovery(&self) -> Option<&LostToolRecovery> {
        self.tool_run().and_then(|run| run.recovery.as_ref())
    }

    /// The plant probe in flight, `Idle` while none is.
    fn plant(&self) -> &PlantProbe {
        const IDLE: &PlantProbe = &PlantProbe::Idle;
        match self.family() {
            Family::Pick(probe) => probe,
            _ => IDLE,
        }
    }

    /// Replace the plant probe in flight. `Idle` ends a pick's family state.
    fn set_plant(&mut self, probe: PlantProbe) {
        if probe == PlantProbe::Idle {
            if matches!(self.family(), Family::Pick(_)) {
                self.end_family(RandomKind::Pick);
            }
            return;
        }
        self.active_mut().family = Family::Pick(probe);
    }

    fn plant_mut(&mut self) -> Option<&mut PlantProbe> {
        match self.active.as_mut().map(|active| &mut active.family) {
            Some(Family::Pick(probe)) => Some(probe),
            _ => None,
        }
    }

    /// One guardian pass per caller frame: detect + publish the status
    /// every frame, but act at most once per snapshot `tick` (the
    /// PLAYER_INFO game-tick edge). Talk-to runs only for a dialog event
    /// that is ours on an un-binned slot; an open dialog then continues
    /// via `continue_dialog` / `answer_choice` (first option), max
    /// [`MAX_CONTINUES`] — the continue is keyed to the in-flight handle,
    /// not to a fresh detect, so a despawned genie cannot stall the chat.
    /// A wrong-talk chat line bins that NPC slot for 45 s. Toggle off:
    /// never act, never hold, still detect+publish. `knock` is the
    /// rising-edge `on_random` arm: once per detected event (kind+name
    /// signature), when the caller supplies it. The script's claim normally
    /// gates act + hold (`Host` claims both; `Handle` leaves the script
    /// available); an event saved as Fight is the Host's for the whole
    /// episode, and a listed one is fought by a native child the claim
    /// source holds (see `fight.rs`).
    pub fn tick<D: Driver>(
        &mut self,
        driver: &mut D,
        snap: &GameSnapshot,
        settings: &ProfileSettings,
        now_ms: u64,
        knock: Option<&mut dyn RandomClaimSource>,
    ) -> RandomStatus {
        self.tick_with_live_policies(
            driver,
            snap,
            settings,
            &GuardianLiveContext {
                event_policies: &settings.random_event_policies,
                game_data: None,
            },
            now_ms,
            knock,
        )
    }

    /// Host-play supplies the live policy map and selected content through
    /// this context, without copying either into the static profile.
    pub fn tick_with_live_policies<D: Driver>(
        &mut self,
        driver: &mut D,
        snap: &GameSnapshot,
        settings: &ProfileSettings,
        context: &GuardianLiveContext<'_>,
        now_ms: u64,
        knock: Option<&mut dyn RandomClaimSource>,
    ) -> RandomStatus {
        let tick = snap.tick() as u64;
        let fresh = self.last_tick != tick;
        let active = snap.ingame() && snap.scene_state() == SCENE_READY;
        if active {
            self.refresh_ignored_plants(snap);
            self.revalidate_plant(snap);
        } else {
            self.clear_plant();
            self.plant_ignored.clear();
        }
        self.gear_loss.observe(snap, now_ms);
        if fresh {
            self.observe_evade_starts(snap, active);
        }
        let mut port = knock;
        self.fight_fold(snap, &mut port, settings, context, fresh && active);
        let mut ev = detect_ignoring_plants(
            snap,
            now_ms,
            &self.cooldown,
            &self.plant_ignored,
            Some(&self.gear_loss),
        );
        self.pin_plant_event(snap, &mut ev);
        self.refresh_evade(snap, &mut ev, context.game_data, fresh);
        if self.evade.is_none() {
            // The actor's removal is observed: its slot can host another.
            self.fight_spent = None;
        }

        if fresh && active {
            // Fresh chat only: a stale wrong-talk line must not re-bin a
            // later NPC the guardian talks to.
            let head = snap
                .chat_lines()
                .first()
                .map(|l| l.sequence)
                .unwrap_or(self.chat_seen);
            let wrong_talk = self.fresh_chat_has(snap, WRONG_TALK_MARKERS);
            let plant_growing = self.fresh_chat_has(snap, &[PLANT_GROWING_MARKER]);
            let plant_rejected = self.fresh_chat_has(snap, &[PLANT_REJECTION_MARKER]);
            let plant_before_chat = self.plant().clone();
            if let Some(handle) = self.dialog().filter(|_| wrong_talk) {
                self.cooldown
                    .insert(usize::from(handle.index), now_ms + WRONG_TALK_COOLDOWN_MS);
                self.clear_handle();
            } else if !matches!(self.plant(), PlantProbe::Idle) && plant_rejected {
                // Plant rejection is pinned to the complete actor identity;
                // a reused slot must not inherit this bin.
                self.ignore_current_plant();
            } else if matches!(self.plant(), PlantProbe::AwaitingResponse { .. }) && plant_growing {
                self.authenticate_plant(now_ms);
            } else if matches!(self.plant(), PlantProbe::Authenticated { .. }) && plant_growing {
                self.note_plant_growing(now_ms);
            } else if self.acting_kind() == Some(RandomKind::Pick) && plant_rejected {
                if let Some(index) = ev.as_ref().and_then(|e| e.npc_index) {
                    self.cooldown.insert(index, now_ms + WRONG_TALK_COOLDOWN_MS);
                }
                self.end_solver();
            }
            // The content adds one box and sends this line every 150 ticks,
            // but stops at 28 boxes (macro_event_strange_box.rs2:25-36,319).
            // Only that fresh server signal re-arms an exhausted budget.
            // Keep the last failure until an answer succeeds or the event ends.
            if self.box_solve().phase == BoxPhase::Stalled
                && Self::server_chat_has_after(snap, self.chat_seen, |text| {
                    text == "Oh dear, you have taken so long that the cube has replicated."
                })
            {
                let solve = self.box_mut();
                solve.failures = 0;
                solve.phase = BoxPhase::Retry;
            }
            self.chat_seen = head;
            // The dialog ended: the chat is closed and the NPC is gone.
            if self.in_flight() && self.dialog_done(snap) {
                self.clear_handle();
            }
            if *self.plant() != plant_before_chat {
                ev = detect_ignoring_plants(
                    snap,
                    now_ms,
                    &self.cooldown,
                    &self.plant_ignored,
                    Some(&self.gear_loss),
                );
            }
            self.pin_plant_event(snap, &mut ev);
            // A claim belongs to the script runtime that made it. A runtime
            // that stopped, failed or was replaced returns it within this
            // tick, and its successor is not called back for the event
            // (design 3.3).
            if let (Some(claimed), Some(source)) = (self.claim_runtime(), port.as_mut()) {
                if source.runtime() != claimed {
                    self.set_claim(RandomClaim::Host, None);
                }
            }
            // Compiled callbacks remain rising-edge only. Load membership
            // can change while this exact event persists, independently of
            // its solver state. No callback source means Host.
            let sig = ev.as_ref().map(|e| format!("{:?}:{}", e.kind, e.name));
            if sig != self.sig {
                self.sig = sig;
                self.episode = EpisodeKey {
                    session: self.episode.session,
                    kind: ev.as_ref().map(|event| event.kind),
                    generation: self.episode.generation.wrapping_add(1),
                };
                // A new event is a new claim, whoever held the last.
                if let Some(active) = self.active.as_mut() {
                    active.owner = Owner::Host;
                }
                // The episode's saved Fight, fixed when it began: a later change
                // of the setting never moves the claim or starts a fight.
                self.episode_fight = ev.as_ref().is_some_and(|event| {
                    event.kind == RandomKind::Evade
                        && Self::saved_policy(context, &event.name) == RandomEventPolicy::Fight
                });
                let (claim, runtime) = match (&ev, port.as_mut()) {
                    (Some(ev), Some(source))
                        if !(ev.kind == RandomKind::Lamp && !settings.lamp_auto) =>
                    {
                        let claim = source.on_random(ev);
                        let runtime = (claim == RandomClaim::Handle)
                            .then(|| source.runtime())
                            .flatten();
                        (claim, runtime)
                    }
                    _ => (RandomClaim::Host, None),
                };
                self.set_claim(claim, runtime);
            }
            let fight_host = self.episode_fight
                && ev
                    .as_ref()
                    .is_some_and(|event| event.kind == RandomKind::Evade);
            match (&ev, port.as_mut()) {
                (Some(ev), Some(source)) => {
                    if !(ev.kind == RandomKind::Lamp && !settings.lamp_auto) && !fight_host {
                        match source.current_claim(ev) {
                            Some(RandomClaim::Handle) => {
                                let runtime = source.runtime();
                                self.set_claim(RandomClaim::Handle, runtime);
                            }
                            Some(RandomClaim::Host) => self.set_claim(RandomClaim::Host, None),
                            None => {}
                        }
                    }
                }
                // No claim source means no script can own the event.
                (_, None) => self.set_claim(RandomClaim::Host, None),
                (None, Some(_)) => {}
            }
        }
        // A saved Fight holds the episode's Host ownership from its first
        // observation to its release: an eligible event is fought, an excluded
        // one (Swarm, Strange plant) keeps today's handling, and neither lets a
        // script claim take it (design V11D 4.3; operator answer CP5).
        if self.episode_fight
            && ev
                .as_ref()
                .is_some_and(|event| event.kind == RandomKind::Evade)
        {
            self.set_claim(RandomClaim::Host, None);
            if let Some(event) = ev.as_ref().filter(|event| {
                matches!(
                    api::random::fight_eligibility(&event.name),
                    Some(api::random::FightEligibility::Excluded(_)) | None
                )
            }) {
                if let Some(state) = self
                    .evade
                    .as_mut()
                    .filter(|state| Some(state.index) == event.npc_index)
                {
                    if !state.policy_logged {
                        api::host_log!(
                            Category::RandomEvent,
                            Level::Info,
                            "evade: Fight does not apply to {}; handling it as Flee",
                            event.name
                        );
                        state.policy_logged = true;
                    }
                }
            }
        }
        if fresh {
            self.observe_lost_tool_refit_rejection(snap);
            // The loss evidence stays while the handle is held: a recovery
            // that begins late (after a script claim, a toggle off or an Ent
            // guard) measures the content window from the loss and searches
            // the loss's origin tiles, not a later tile
            // (`macro_event_lost_axe.rs2:18,25-27`; `ServerOps.ts:298-335`).
            if !has_lost_tool(snap)
                && self.tool_onset.is_some_and(|onset| {
                    snap.tick().wrapping_sub(onset.tick) > LOST_TOOL_SPAWN_GRACE_TICKS
                })
            {
                self.tool_onset = None;
            }
            if active && self.tool_recovery().is_none() {
                self.observe_lost_tool_onset(snap);
            }
            if !ev
                .as_ref()
                .is_some_and(|event| event.kind == RandomKind::LostTool)
                && self.tool_onset.is_none()
            {
                self.observe_lost_tool_baseline(snap, context.game_data);
            }
        }
        if !has_lost_tool(snap) {
            // The handle is gone: the window and the refit token end. The
            // worn-handle evidence stays for the re-wield on resolve.
            if let Some(Family::LostTool(run)) =
                self.active.as_mut().map(|active| &mut active.family)
            {
                run.recovery = None;
                run.refit_after_sequence = None;
            }
            self.settle();
            self.tool_stalled = false;
        } else if !settings.random_events {
            self.tool_stalled = false;
        } else if self.tool_stalled
            && self.tool_recovery().is_some_and(|recovery| {
                recovery.target.is_some_and(|target| {
                    target.missing_since_tick.is_some()
                        && snap
                            .inventory()
                            .iter()
                            .any(|item| item.count > 0 && item.def.id == target.id)
                })
            })
        {
            self.tool_stalled = false;
            self.begin_solver(RandomKind::LostTool);
        }
        let inert_tool = self.tool_stalled
            && ev
                .as_ref()
                .is_some_and(|event| event.kind == RandomKind::LostTool);

        let on_box = ev
            .as_ref()
            .is_some_and(|event| event.kind == RandomKind::Box);
        if !on_box || !settings.random_events {
            self.end_family(RandomKind::Box);
        }
        if fresh && self.claim() != RandomClaim::Host {
            // No native action runs under a script claim. Shift the origin
            // by those observed ticks so neither wait spends Host's budget.
            let paused = (tick as u32).wrapping_sub(self.last_tick as u32);
            if let Some(Family::Box(solve)) = self.active.as_mut().map(|active| &mut active.family)
            {
                match &mut solve.phase {
                    BoxPhase::Opening { tick } | BoxPhase::Answering { tick, .. } => {
                        *tick = tick.wrapping_add(paused);
                    }
                    _ => {}
                }
            }
        }

        // A pause counts down once per game tick; the Rub resumes when it
        // reaches zero.
        if fresh {
            self.lamp.park = self.lamp.park.saturating_sub(1);
        }
        // A stalled or paused redemption unlocks on the two operator-visible
        // state changes: the lamp left the pack, or lamp auto went off (the
        // next auto-on redemption starts clean).
        if (self.lamp.stalled || self.lamp.park > 0) && (!lamp_held(snap) || !settings.lamp_auto) {
            self.clear_lamp();
        }
        // An inert lamp is detect-only: `lamp_auto` off, or a redemption
        // that gave up or is paused between Rubs. No act, no hold, no `ours`
        // — the script must not stay frozen behind a lamp the host is not
        // driving right now.
        let inert_lamp = ev.as_ref().is_some_and(|e| e.kind == RandomKind::Lamp)
            && (!settings.lamp_auto || self.lamp_inert());
        if inert_lamp {
            // Drop a previous auto-on latch the moment the operator
            // turns lamp auto off with the lamp still in inv, or the
            // redemption gave up with the lamp still held.
            self.end_solver();
        }
        // A Maze visit the host gave up is released like a stalled Box: the
        // square stays detected, but nothing holds the slot on a trap the
        // guardian is not walking out of. Both families end with their event.
        let on_maze = ev.as_ref().is_some_and(|e| e.kind == RandomKind::Maze);
        if !on_maze {
            self.end_family(RandomKind::Maze);
        }
        if !ev.as_ref().is_some_and(|e| e.kind == RandomKind::Mime) {
            self.end_family(RandomKind::Mime);
        }
        let released = self.family().released();

        let mut ent_latch = self.active.as_mut().and_then(|active| active.ent.take());
        let ent_hold = self.ent.tick(
            &mut ent_latch,
            driver,
            snap,
            context.game_data,
            active && settings.random_events && self.claim() == RandomClaim::Host,
            fresh,
        );
        self.set_ent(ent_latch, ent_hold);

        if fresh
            && active
            && settings.random_events
            && self.claim() == RandomClaim::Host
            && !inert_lamp
            && !released
            && !inert_tool
            && !ent_hold
        {
            let plant_before_act = self.plant().clone();
            let ignored_before_act = self.plant_ignored.len();
            self.act(
                driver,
                snap,
                ev.as_ref(),
                settings,
                &mut FightDesk {
                    context,
                    port: &mut port,
                },
                now_ms,
            );
            if *self.plant() != plant_before_act || self.plant_ignored.len() != ignored_before_act {
                ev = detect_ignoring_plants(
                    snap,
                    now_ms,
                    &self.cooldown,
                    &self.plant_ignored,
                    Some(&self.gear_loss),
                );
            }
        }
        // `step_pick` may have timed out or seen a refused send. Reflect
        // that release in this same status frame instead of publishing a
        // stale authenticated `ours`.
        self.pin_plant_event(snap, &mut ev);
        self.last_tick = tick;
        // A family that replaced the Evade run this pass left its child
        // without an owner: it is revoked before the status publishes.
        self.fight_close_orphan(&mut port);

        // `act` may have paused or stalled the redemption just above: a lamp
        // the host is not driving this tick is inert in the same tick, so the
        // script's `EventSignal.pending` (hold OR ours) clears immediately.
        let inert_lamp = inert_lamp
            || (self.lamp_inert() && ev.as_ref().is_some_and(|e| e.kind == RandomKind::Lamp));
        let inert_tool = inert_tool
            || (self.tool_stalled
                && ev
                    .as_ref()
                    .is_some_and(|event| event.kind == RandomKind::LostTool));
        let cooldown = ev
            .as_ref()
            .and_then(|e| e.npc_index)
            .is_some_and(|i| binned(i, now_ms, &self.cooldown));
        let inputs = PublicationInputs {
            toggle: settings.random_events,
            // A parked lamp and a stalled tool are suppression history: the
            // host gave up on them, so the event is detected but not ours.
            detected_ours: ev.as_ref().is_some_and(|e| e.ours) && !inert_lamp && !inert_tool,
            trapped: ev.as_ref().is_some_and(|e| is_trapped(e.kind)),
        };
        #[cfg(test)]
        {
            self.last_inputs = inputs;
        }
        let published = derive_publication(self.active.as_ref(), inputs);
        RandomStatus {
            kind: ev.as_ref().map(|e| e.kind),
            name: ev.as_ref().map(|e| e.name.clone()),
            ours: published.ours,
            handling: published.handling,
            hold: published.hold,
            toggle: settings.random_events,
            claim: published.claim,
            cooldown,
            response: self.derive_response(ev.as_ref(), settings.random_events),
            fight: self.fight_token(),
        }
    }

    /// Whether a server game message newer than `after_sequence`
    /// matches `matches`. Every caller matches server text, never
    /// player text. The stun lines, the two plant lines, the
    /// wrong-talk rejection line, and the lost-tool refit rejection
    /// line are content `mes` messages. The snapshot row
    /// is `ChatLineView { type_, username, text, sequence }`
    /// (`crates/api/src/snapshot/views.rs:481-486`). A `mes`
    /// reaches the client as `MESSAGE_GAME` and is stored as type 0
    /// with an empty sender (`client.rs:6425`; the operation path
    /// stores the same form at `client.rs:5994`); the snapshot maps
    /// an empty sender to `None` (`decode.rs:362-363`). Public chat
    /// from another player is type 2 with a sender
    /// (`actor_289.rs:613`) and a private message is type 3 or 7
    /// (`client.rs:5899-5903`). Player text is not a server signal,
    /// so only type 0 with no sender counts. Chat rows are newest
    /// first with descending sequences (`snapshot.rs:1103-1106`),
    /// so the scan stops at the cutoff.
    fn server_chat_has_after(
        snap: &GameSnapshot,
        after_sequence: i32,
        matches: impl Fn(&str) -> bool,
    ) -> bool {
        snap.chat_lines()
            .iter()
            .take_while(|l| l.sequence > after_sequence)
            .filter(|l| l.type_ == 0 && l.username.is_none())
            .any(|l| matches(&l.text))
    }

    /// Whether a fresh server game message contains a marker. This
    /// runs before the tick's chat scan advances `chat_seen`, so
    /// every caller sees the same batch.
    fn fresh_chat_has(&self, snap: &GameSnapshot, markers: &[&str]) -> bool {
        Self::server_chat_has_after(snap, self.chat_seen, |text| {
            markers.iter().any(|marker| text.contains(marker))
        })
    }

    /// Remember hostile spawn sightings even before they target this player.
    /// Dryads delay one tick between `npc_add` and `opplayer2`
    /// (`macro_event_dryad.rs2:10-18`); the engine's immutable chase anchor is
    /// constructor `startX/startZ`, not the later targeting tile
    /// (`Npc.ts:77-85,661-670`). A late sighting is only a route estimate:
    /// it can never overrule a still-targeting actor.
    fn observe_evade_starts(&mut self, snap: &GameSnapshot, active: bool) {
        if !active {
            self.evade_starts.clear();
            return;
        }
        self.evade_starts.retain(|seen| {
            snap.npcs()
                .iter()
                .any(|npc| npc.index == seen.index && npc.r#type == seen.npc_type)
        });
        for npc in snap.npcs().iter().filter(|npc| is_evade_npc(npc)) {
            if !self
                .evade_starts
                .iter()
                .any(|seen| seen.index == npc.index && seen.npc_type == npc.r#type)
            {
                self.evade_starts.push(EvadeSighting {
                    index: npc.index,
                    npc_type: npc.r#type,
                    tile: (npc.network.x, npc.network.z),
                });
            }
        }
    }

    /// Retain the exact macro-event actor through its cleanup interval.
    /// Missing beyond view keeps fleeing and counts stationary outside-chase
    /// ticks, even short of the full flee ring. Give up sooner if an engaged
    /// actor can still hit a bot that makes no outward distance progress.
    fn refresh_evade(
        &mut self,
        snap: &GameSnapshot,
        ev: &mut Option<DetectedRandom>,
        game_data: Option<&SelectedGameData>,
        fresh: bool,
    ) {
        // The fight owns the actor's ticks, and an admitted input lock blocks
        // the walk: neither is evidence of escape progress or of its failure
        // (design V11D 6.3), as a stunned tick is not.
        let counted = !(self.in_combat() || self.lock_blocked(snap.tick()));
        if let Some(mut state) = self.evade.take() {
            let actor = snap.npcs().iter().find(|npc| npc.index == state.index);
            let same_actor =
                actor.is_some_and(|npc| npc.r#type == state.npc_type && is_evade_npc(npc));
            let engaged = actor.is_some_and(|npc| {
                event_npc_targets_player(npc, snap.self_slot())
                    || (npc.target.is_none() && npc.in_combat)
            });
            let player = snap.tile().map(|(x, z, _)| (x, z));
            let outside = player.is_some_and(|player| {
                state
                    .maxrange
                    .is_some_and(|range| outside_chase_range(player, state.start_tile, range))
            });
            let moving = snap
                .local_player()
                .is_some_and(|local| local.player.actor.moving);
            // An NPC can take two movement steps between observations
            // (PathingEntity.ts:135-152). Only a removal comfortably inside
            // its latest view position is immediate despawn evidence.
            let beyond_view =
                player.is_some_and(|player| cheb(player, state.last_tile) > NPC_VIEW_RADIUS - 2);
            let other_event = ev.as_ref().is_some_and(|event| {
                event.kind != RandomKind::Evade
                    || event.npc_index != Some(state.index)
                    || !event.name.eq_ignore_ascii_case(&state.name)
            });
            let retain = if same_actor {
                state.last_tile = actor
                    .map(|npc| (npc.network.x, npc.network.z))
                    .expect("same actor is visible");
                state.missing_ticks = 0;
                if fresh {
                    state.outside_ticks = if outside && engaged {
                        state.outside_ticks.saturating_add(1)
                    } else {
                        0
                    };
                }
                true
            } else if actor.is_none() && state.has_fled && beyond_view {
                if fresh {
                    state.missing_ticks = if outside && !moving {
                        state.missing_ticks.saturating_add(1)
                    } else {
                        0
                    };
                }
                if state.missing_ticks >= EVADE_CLEANUP_TICKS {
                    if !other_event {
                        *ev = None;
                    }
                    return;
                }
                true
            } else {
                // Missing inside the view, or a changed slot, is observed removal.
                false
            };
            if retain {
                if fresh {
                    state.elapsed_ticks = state.elapsed_ticks.saturating_add(1);
                    if let Some(distance) = player
                        .map(|tile| cheb(tile, state.start_tile))
                        .filter(|_| counted)
                    {
                        if distance > state.furthest_distance {
                            state.furthest_distance = distance;
                            state.no_progress_ticks = 0;
                        } else if same_actor && engaged && !outside && state.maxrange.is_some() {
                            // A stunned tick neither increments nor resets:
                            // the server cancelled the walk, so the route
                            // had no chance to progress or to fail.
                            if !self.fresh_chat_has(snap, STUNNED_MARKERS) {
                                state.no_progress_ticks = state.no_progress_ticks.saturating_add(1);
                            }
                        } else {
                            state.no_progress_ticks = 0;
                        }
                    } else if counted {
                        state.no_progress_ticks = 0;
                    }
                }
                // An outward route advances 1-2 tiles per tick
                // (PathingEntity.ts:135-152). Allow every fallback ring one
                // fresh try plus two movement observations, but never count
                // accepted walks that leave the bot on its own tile as progress.
                let stalled =
                    i32::from(state.no_progress_ticks) >= state.flee_distance.saturating_add(2);
                if (stalled || state.elapsed_ticks >= EVADE_GIVE_UP_TICKS) && !state.gave_up {
                    state.gave_up = true;
                    if self.acting_kind() == Some(RandomKind::Evade) {
                        self.end_solver();
                        self.end_family(RandomKind::Evade);
                    }
                    api::host_log!(
                        Category::RandomEvent,
                        Level::Warn,
                        "evade: {} reached the escape attempt limit; returning control to the script",
                        state.name
                    );
                }
                // A retained former attacker must not replace a newly
                // detected box, trap or dialog. Keep its original deadline.
                if other_event {
                    if ev
                        .as_ref()
                        .is_some_and(|event| event.kind != RandomKind::Evade)
                    {
                        self.evade = Some(state);
                        return;
                    }
                } else {
                    *ev = Some(DetectedRandom {
                        kind: RandomKind::Evade,
                        name: state.name.clone(),
                        ours: state.maxrange.is_some() && !state.gave_up,
                        npc_index: Some(state.index),
                    });
                    self.evade = Some(state);
                    return;
                }
            }
        }

        let Some(event) = ev.as_ref().filter(|event| event.kind == RandomKind::Evade) else {
            return;
        };
        let Some(index) = event.npc_index else {
            return;
        };
        let Some(npc) = snap.npcs().iter().find(|npc| npc.index == index) else {
            return;
        };
        if !event_npc_targets_player(npc, snap.self_slot()) {
            return;
        }
        let content = npc
            .r#type
            .and_then(|id| i32::try_from(id).ok())
            .and_then(|id| game_data?.npc_name(id));
        let maxrange = content.map(|row| row.maxrange);
        // Wandering stays within wanderrange of startX/startZ (Npc.ts:705-728).
        // Hostile deletion needs NPC-player distance >3 (macro_events.rs2:168).
        let flee_distance = content
            .map(|row| {
                row.maxrange
                    .max(0)
                    .saturating_add(2)
                    .max(row.wanderrange.max(0).saturating_add(4))
            })
            .unwrap_or(0);
        let start_tile = self
            .evade_starts
            .iter()
            .find(|seen| seen.index == index && seen.npc_type == npc.r#type)
            .map(|seen| seen.tile)
            .unwrap_or((npc.network.x, npc.network.z));
        let state = EvadeTrack {
            index,
            npc_type: npc.r#type,
            name: event.name.clone(),
            maxrange,
            flee_distance,
            start_tile,
            last_tile: (npc.network.x, npc.network.z),
            outside_ticks: 0,
            missing_ticks: 0,
            elapsed_ticks: 0,
            furthest_distance: snap.tile().map_or(0, |(x, z, _)| cheb((x, z), start_tile)),
            no_progress_ticks: 0,
            gave_up: false,
            has_fled: false,
            policy_logged: false,
        };
        if state.maxrange.is_none() {
            api::host_log!(
                Category::RandomEvent,
                Level::Warn,
                "evade: no selected content range for {}; not fleeing",
                state.name
            );
            if let Some(event) = ev.as_mut() {
                event.ours = false;
            }
        }
        self.evade = Some(state);
    }

    fn clear_plant(&mut self) {
        self.set_plant(PlantProbe::Idle);
        if self.acting_kind() == Some(RandomKind::Pick) {
            self.end_solver();
        }
    }

    fn ignore_current_plant(&mut self) {
        if let Some(actor) = self.plant().actor().cloned() {
            self.ignore_plant(actor);
        }
        self.set_plant(PlantProbe::Idle);
        if self.acting_kind() == Some(RandomKind::Pick) {
            self.end_solver();
        }
    }

    fn ignore_plant(&mut self, actor: PlantActor) {
        if let Some(existing) = self
            .plant_ignored
            .iter_mut()
            .find(|existing| existing.slot == actor.slot)
        {
            *existing = actor;
        } else {
            self.plant_ignored.push(actor);
        }
    }

    /// Remove a bin only after its slot is observed empty. If a structurally
    /// different pickable actor appears in that slot without an observed gap,
    /// replace the bin so the ambiguous replacement inherits no auth.
    fn refresh_ignored_plants(&mut self, snap: &GameSnapshot) {
        self.plant_ignored.retain_mut(|ignored| {
            let Some(current) =
                npc_by_index(snap.npcs(), ignored.slot).and_then(PlantActor::from_npc)
            else {
                return false;
            };
            *ignored = current;
            true
        });
    }

    fn authenticate_plant(&mut self, now_ms: u64) {
        let PlantProbe::AwaitingResponse { actor, .. } = self.plant() else {
            return;
        };
        let actor = actor.clone();
        self.set_plant(PlantProbe::Authenticated {
            actor,
            retry_at_ms: now_ms.saturating_add(PLANT_RETRY_INTERVAL_MS),
            deadline_ms: now_ms.saturating_add(PLANT_AUTH_TIMEOUT_MS),
            continues: 0,
        });
    }

    fn note_plant_growing(&mut self, now_ms: u64) {
        if let Some(PlantProbe::Authenticated { retry_at_ms, .. }) = self.plant_mut() {
            *retry_at_ms = now_ms.saturating_add(PLANT_RETRY_INTERVAL_MS);
        }
    }

    /// Drop authentication as soon as any structural actor field changes.
    /// If a different pickable actor reused the slot, bind an ignore to the
    /// replacement so stale state can never turn into a retarget.
    fn revalidate_plant(&mut self, snap: &GameSnapshot) {
        let Some(expected) = self.plant().actor().cloned() else {
            return;
        };
        let current = npc_by_index(snap.npcs(), expected.slot).and_then(PlantActor::from_npc);
        if current.as_ref() == Some(&expected) {
            return;
        }

        if let Some(replacement) = current {
            self.ignore_plant(replacement);
        }
        self.set_plant(PlantProbe::Idle);
        if self.acting_kind() == Some(RandomKind::Pick) {
            self.end_solver();
        }
    }

    /// Keep an in-flight probe pinned to its actor even after the player
    /// moves away and publish server authentication as `ours`. Established
    /// non-plant random kinds still preempt it.
    fn pin_plant_event(&mut self, snap: &GameSnapshot, ev: &mut Option<DetectedRandom>) {
        match self.plant().clone() {
            PlantProbe::AwaitingResponse { actor, .. }
            | PlantProbe::Authenticated { actor, .. } => {
                if ev.as_ref().is_some_and(|e| e.kind != RandomKind::Pick) {
                    return;
                }
                let Some(npc) = npc_by_index(snap.npcs(), actor.slot) else {
                    *ev = None;
                    return;
                };
                let display_name = snap
                    .local_player()
                    .and_then(|lp| lp.player.actor.name.clone());
                let authenticated = matches!(self.plant(), PlantProbe::Authenticated { .. });
                *ev = Some(DetectedRandom {
                    kind: RandomKind::Pick,
                    name: PICK_NAME.to_string(),
                    ours: authenticated
                        || owned_pickable_plant(npc, snap.self_slot(), display_name.as_deref()),
                    npc_index: Some(actor.slot),
                });
            }
            PlantProbe::Idle => {}
        }
    }

    /// One send per game tick. The in-flight dialog continues on its own
    /// handle; everything else goes through the solver machine below.
    /// Refuses silently when the wire layer says no (not ingame, stale
    /// target, chat closed), so the machine is driven by the snapshot,
    /// not by error paths.
    fn act<D: Driver>(
        &mut self,
        driver: &mut D,
        snap: &GameSnapshot,
        ev: Option<&DetectedRandom>,
        settings: &ProfileSettings,
        desk: &mut FightDesk<'_, '_, '_>,
        now_ms: u64,
    ) {
        // The in-flight dialog continues on its own handle, independent
        // of a fresh detect: the genie can despawn while the chat is
        // still open, and detect would then return None.
        if let Some(handle) = self.dialog() {
            if u32::from(handle.continues) >= MAX_CONTINUES {
                self.clear_handle();
                return;
            }
            let mut ix = Interactions::new(snap, driver);
            let result = if snap.chat_options().is_empty() {
                ix.continue_dialog(None)
            } else {
                ix.answer_choice(1)
            };
            match result {
                SendResult::Sent { .. } => {
                    if let Some(handle) = self.dialog_mut() {
                        handle.continues = handle.continues.saturating_add(1);
                    }
                }
                // Spec: stop when continue refuses. Keep one tick after
                // Talk-to for chat to open; clear once chat is still
                // closed and a continue/answer has refused.
                SendResult::Refused { .. } => {
                    if !chat_is_open(snap) {
                        if handle.continues == 0 {
                            if let Some(handle) = self.dialog_mut() {
                                handle.continues = 1;
                            }
                        } else {
                            self.clear_handle();
                        }
                    }
                }
            }
            return;
        }
        self.act_solver(driver, snap, ev, settings, desk, now_ms);
    }

    /// The non-dialog act machine: drive the in-flight act to completion,
    /// or start one on a fresh event (the trapped kinds and the dialog
    /// handle never get here). `acting` latches the first send; the
    /// resolution rules are per kind — the walk-to-range kinds resolve
    /// when the event's signature changes (the NPC/plant/gear/tool left
    /// the scene), hazard resolves when the loc is no longer underfoot,
    /// lamp resolves when the lamp leaves the inventory.
    fn act_solver<D: Driver>(
        &mut self,
        driver: &mut D,
        snap: &GameSnapshot,
        ev: Option<&DetectedRandom>,
        settings: &ProfileSettings,
        desk: &mut FightDesk<'_, '_, '_>,
        now_ms: u64,
    ) {
        // A confirmed lamp redemption outlives its own detect: the server
        // consumes the lamp and opens the award dialogue in the same step,
        // so the drain must run while `detect` already shows nothing (or a
        // newly spawned event). It takes the slot, like the dialog handle,
        // until it completes or gives up.
        if self.acting_kind() == Some(RandomKind::Lamp) && self.lamp_flow().confirmed {
            self.step_lamp(driver, snap, settings);
            return;
        }
        let Some(ev) = ev else {
            if self.acting() {
                self.resolve(
                    driver,
                    snap,
                    self.acting_kind() == Some(RandomKind::Evade) && self.evade.is_none(),
                );
            }
            return;
        };
        if !ev.ours && ev.kind != RandomKind::Pick {
            if self.acting() {
                self.resolve(
                    driver,
                    snap,
                    self.acting_kind() == Some(RandomKind::Evade) && self.evade.is_none(),
                );
            }
            return;
        }
        if self.acting() {
            let same_evade = self.acting_kind() != Some(RandomKind::Evade)
                || self
                    .evade
                    .as_ref()
                    .is_some_and(|state| ev.npc_index == Some(state.index));
            if Some(ev.kind) != self.acting_kind() || !same_evade {
                self.resolve(
                    driver,
                    snap,
                    self.acting_kind() == Some(RandomKind::Evade) && self.evade.is_none(),
                );
            } else {
                // The act continues for the successor episode of its family:
                // an Evade hands its flee to the next hostile actor without a
                // release tick.
                self.adopt_episode(ev.kind);
                self.step_act(driver, snap, settings, desk, ev, now_ms);
            }
            return;
        }
        self.begin_solver(ev.kind);
        match ev.kind {
            RandomKind::Evade => {
                // The pre-flee tile: the walk-back target after despawn.
                *self.evade_run_mut() = EvadeRun {
                    flee_from: snap.tile().map(|(x, z, _)| (x, z)),
                    ..EvadeRun::default()
                };
            }
            RandomKind::LostTool => {
                self.begin_lost_tool(snap);
                // Remember whether the handle was worn so the reattached
                // tool is re-wielded.
                let was_worn = snap
                    .equipment()
                    .iter()
                    .any(|i| is_tool_handle(i.def.name.as_deref()));
                let handle_base = snap
                    .equipment()
                    .iter()
                    .chain(snap.inventory().iter())
                    .find(|i| is_tool_handle(i.def.name.as_deref()))
                    .and_then(|i| i.def.name.as_deref())
                    .map(|n| {
                        n.trim()
                            .trim_end_matches("handle")
                            .trim_end_matches("Handle")
                            .trim()
                            .to_lowercase()
                    });
                let run = self.tool_run_mut();
                run.was_worn = was_worn;
                run.handle_base = handle_base;
            }
            _ => {}
        }
        self.step_act(driver, snap, settings, desk, ev, now_ms);
    }

    /// One step of the in-flight (or freshly started) non-dialog act,
    /// per kind.
    fn step_act<D: Driver>(
        &mut self,
        driver: &mut D,
        snap: &GameSnapshot,
        settings: &ProfileSettings,
        desk: &mut FightDesk<'_, '_, '_>,
        ev: &DetectedRandom,
        now_ms: u64,
    ) {
        let Some(kind) = self.acting_kind() else {
            return;
        };
        match kind {
            RandomKind::Dialog => self.step_dialog(driver, snap, ev, now_ms),
            RandomKind::Pick => self.step_pick(driver, snap, ev, now_ms),
            RandomKind::Evade => self.step_evade(driver, snap, ev, settings, desk),
            RandomKind::Hazard => self.step_hazard(driver, snap),
            RandomKind::Lamp => self.step_lamp(driver, snap, settings),
            RandomKind::LostGear => self.step_lost_gear(driver, snap, ev),
            RandomKind::LostTool => self.step_lost_tool(driver, snap),
            RandomKind::Mime => self.step_mime(driver, snap),
            RandomKind::Box => self.step_box(driver, snap),
            RandomKind::Maze => self.step_maze(driver, snap),
        }
    }

    /// The event resolved (gone, changed kind, or no longer ours): clear
    /// the in-flight latch and finish the kind's tail work — the evade
    /// walk-back toward the pre-flee tile, and the lost-tool re-wield of
    /// a handle that was worn.
    fn resolve<D: Driver>(&mut self, driver: &mut D, snap: &GameSnapshot, walk_back: bool) {
        if self.acting_kind() == Some(RandomKind::Evade) && walk_back {
            if let Some((fx, fz)) = self.evade_run().flee_from {
                walk(driver, fx, fz);
            }
        }
        // Content handles omit the tier: "Axe handle" can restore "Rune axe".
        // Re-wield only a formerly worn tool; Gatherer also validates its gates.
        if self.acting_kind() == Some(RandomKind::LostTool) {
            if let Some(base) = self
                .tool_run()
                .filter(|run| run.was_worn)
                .and_then(|run| run.handle_base.as_deref())
                .and_then(|n| n.rsplit(' ').next())
            {
                if let Some(tool) = snap.inventory().iter().find(|i| {
                    i.def.name.as_deref().is_some_and(|name| {
                        name.trim()
                            .rsplit(' ')
                            .next()
                            .is_some_and(|last| last.eq_ignore_ascii_case(base))
                    })
                }) {
                    let mut ix = Interactions::new(snap, driver);
                    let _ = ix.wear(tool.def.id);
                }
            }
        }
        self.end_solver();
        self.end_any_family();
        self.clear_lamp();
    }

    /// Talk-to, gated on range: an NPC further than Chebyshev 1 gets a
    /// ground walk to its tile first (the same `try_move` the nav bot
    /// uses); the Talk-to fires once the walk closes in.
    fn step_dialog<D: Driver>(
        &mut self,
        driver: &mut D,
        snap: &GameSnapshot,
        ev: &DetectedRandom,
        now_ms: u64,
    ) {
        let Some(index) = ev.npc_index else {
            self.end_solver();
            return;
        };
        // A slot binned this pass (the wrong-talk above) or earlier is
        // not re-engaged; detect skips binned slots, this guards the
        // same-tick bin.
        if binned(index, now_ms, &self.cooldown) {
            self.end_solver();
            return;
        }
        // `ev.npc_index` is the client NPC slot (`NpcView.index`), not
        // the dense view-vec position, so the lookup must scan by slot.
        let Some(npc) = npc_by_index(snap.npcs(), index) else {
            self.end_solver();
            return;
        };
        let Some((px, pz, level)) = snap.tile() else {
            return;
        };
        let player = api::snapshot::WorldTile {
            x: px,
            z: pz,
            level,
        };
        if api::query::chebyshev_to(player, npc.network) > 1 {
            walk(driver, npc.network.x, npc.network.z);
            return;
        }
        let mut ix = Interactions::new(snap, driver);
        match ix.interact(OpTarget::Npc(npc), ActionSpec::Label("Talk-to".to_string())) {
            SendResult::Sent { .. } => {
                self.begin_dialog(index);
                self.end_solver();
            }
            SendResult::Refused { .. } => self.end_solver(),
        }
    }

    /// Drive either the established hard-owner behavior or the bounded
    /// adjacent server probe. No featureless actor is walked to; only the
    /// exact actor authenticated by the growing response may later be chased.
    fn step_pick<D: Driver>(
        &mut self,
        driver: &mut D,
        snap: &GameSnapshot,
        ev: &DetectedRandom,
        now_ms: u64,
    ) {
        let Some((px, pz, level)) = snap.tile() else {
            return;
        };
        let player = api::snapshot::WorldTile {
            x: px,
            z: pz,
            level,
        };
        let Some(index) = ev.npc_index else {
            self.end_solver();
            return;
        };
        let Some(npc) = npc_by_index(snap.npcs(), index) else {
            self.end_solver();
            return;
        };
        let display_name = snap
            .local_player()
            .and_then(|lp| lp.player.actor.name.clone());
        let Some(actor) = PlantActor::from_npc(npc) else {
            self.clear_plant();
            return;
        };
        let distance = api::query::chebyshev_to(player, npc.network);

        // Preserve the existing hard evidence path, including its allowed
        // walk to range. It does not need or inherit probe authentication.
        if owned_pickable_plant(npc, snap.self_slot(), display_name.as_deref()) {
            self.set_plant(PlantProbe::Idle);
            self.plant_ignored.retain(|ignored| ignored != &actor);
            if distance > 1 {
                walk(driver, npc.network.x, npc.network.z);
                return;
            }
            let mut ix = Interactions::new(snap, driver);
            if matches!(
                ix.interact(OpTarget::Npc(npc), ActionSpec::Label("Pick".to_string())),
                SendResult::Refused { .. }
            ) {
                self.end_solver();
            }
            return;
        }

        if !featureless_pickable_plant(npc) {
            self.ignore_current_plant();
            return;
        }

        match self.plant().clone() {
            PlantProbe::Idle => {
                if distance > 1 {
                    self.end_solver();
                    return;
                }
                let mut ix = Interactions::new(snap, driver);
                match ix.interact(OpTarget::Npc(npc), ActionSpec::Label("Pick".to_string())) {
                    SendResult::Sent { .. } => {
                        self.set_plant(PlantProbe::AwaitingResponse {
                            actor,
                            deadline_ms: now_ms.saturating_add(PLANT_PROBE_TIMEOUT_MS),
                        });
                    }
                    SendResult::Refused { .. } => {
                        self.ignore_plant(actor);
                        self.set_plant(PlantProbe::Idle);
                        self.end_solver();
                    }
                }
            }
            PlantProbe::AwaitingResponse {
                actor: expected,
                deadline_ms,
            } => {
                if actor != expected {
                    self.ignore_plant(actor);
                    self.set_plant(PlantProbe::Idle);
                    self.end_solver();
                } else if now_ms >= deadline_ms {
                    self.ignore_current_plant();
                }
            }
            PlantProbe::Authenticated {
                actor: expected,
                retry_at_ms,
                deadline_ms,
                continues,
            } => {
                if actor != expected {
                    self.ignore_plant(actor);
                    self.set_plant(PlantProbe::Idle);
                    self.end_solver();
                    return;
                }
                if now_ms >= deadline_ms || continues >= MAX_PLANT_CONTINUES {
                    self.ignore_current_plant();
                    return;
                }
                if chat_is_open(snap) {
                    let mut ix = Interactions::new(snap, driver);
                    match ix.continue_dialog(None) {
                        SendResult::Sent { .. } => {
                            if let Some(PlantProbe::Authenticated { continues, .. }) =
                                self.plant_mut()
                            {
                                *continues += 1;
                            }
                        }
                        SendResult::Refused { .. } => self.ignore_current_plant(),
                    }
                    return;
                }
                if now_ms < retry_at_ms {
                    return;
                }
                if distance > 1 {
                    if !walk(driver, npc.network.x, npc.network.z) {
                        self.ignore_current_plant();
                    } else if let Some(PlantProbe::Authenticated { retry_at_ms, .. }) =
                        self.plant_mut()
                    {
                        *retry_at_ms = now_ms.saturating_add(PLANT_RETRY_INTERVAL_MS);
                    }
                    return;
                }
                let mut ix = Interactions::new(snap, driver);
                match ix.interact(OpTarget::Npc(npc), ActionSpec::Label("Pick".to_string())) {
                    SendResult::Sent { .. } => {
                        if let Some(PlantProbe::Authenticated { retry_at_ms, .. }) =
                            self.plant_mut()
                        {
                            *retry_at_ms = now_ms.saturating_add(PLANT_RETRY_INTERVAL_MS);
                        }
                    }
                    SendResult::Refused { .. } => self.ignore_current_plant(),
                }
            }
        }
    }

    /// Clear `maxrange+1` targeting and every `wanderrange` tile by >3 for
    /// the content's cleanup timer. Late-anchor targeting feedback after
    /// the next NPC phase requires another outward step, not a finished latch.
    /// Blocked exact routes retain the existing nearest/nearer-ring fallback.
    fn step_evade<D: Driver>(
        &mut self,
        driver: &mut D,
        snap: &GameSnapshot,
        _ev: &DetectedRandom,
        settings: &ProfileSettings,
        desk: &mut FightDesk<'_, '_, '_>,
    ) {
        let Some((npc_type, facts)) = self.evade.as_ref().map(|state| {
            (
                state.npc_type,
                FleeFacts {
                    index: state.index,
                    maxrange: state.maxrange,
                    flee_distance: state.flee_distance,
                    start_tile: state.start_tile,
                    outside_ticks: state.outside_ticks,
                },
            )
        }) else {
            self.end_solver();
            return;
        };
        // The flee in flight belongs to this actor. A successor actor keeps
        // the walk-back tile and starts its own walks; a fight of the old
        // actor ends with it.
        let mut run = self.evade_run();
        if run.actor != Some((facts.index, npc_type)) {
            if let (Some(token), Some(source)) = (
                run.response.token().or(run.escape),
                desk.port.as_deref_mut(),
            ) {
                source.fight_revoke(token, FightCancel::Replaced);
                source.fight_release(token);
                self.fight_open = None;
            }
            run = EvadeRun {
                actor: Some((facts.index, npc_type)),
                flee_from: run.flee_from,
                ..EvadeRun::default()
            };
        }
        // The fight, or the wait for its kill's removal, holds the event: no
        // escape walk goes out.
        if run.response.in_combat() {
            *self.evade_run_mut() = run;
            return;
        }
        // Offer the Fight once, before this actor's first escape step.
        if !run.from_fight && run.escape.is_none() {
            *self.evade_run_mut() = run;
            if self.fight_begin(snap, desk.port, desk.context, settings) {
                return;
            }
            run = self.evade_run();
        }
        // The walk out of a fight that handed over goes through the child's
        // upkeep guard, which knows an admitted input lock.
        if run.escape.is_some() {
            *self.evade_run_mut() = run;
            if self.fight_stage_escape(desk.port, facts, run) {
                return;
            }
            run = self.evade_run();
        }
        // An admitted lock still blocks every click, raw or routed.
        if let Some(until) = run.lock_until {
            if fight::lock_blocks(snap.tick(), until) {
                *self.evade_run_mut() = run;
                return;
            }
            run.lock_until = None;
        }
        let fled = flee_step(driver, snap, &mut run, facts);
        *self.evade_run_mut() = run;
        if fled {
            if let Some(state) = self.evade.as_mut() {
                state.has_fled = true;
            }
        }
    }

    /// Step off a hazard underfoot: use the hazard's posted tile as the
    /// threat and the frozen four-tile step ring (`RandomEvents.ts:607-614`).
    /// Stop once no hazard loc is within Chebyshev 2; the loc may still
    /// linger elsewhere in the loaded scene.
    fn step_hazard<D: Driver>(&mut self, driver: &mut D, snap: &GameSnapshot) {
        let Some((px, pz, _)) = snap.tile() else {
            return;
        };
        let Some(hazard) = snap.locs().iter().find(|loc| {
            is_hazard_loc_name(loc.name.as_deref()) && cheb((px, pz), (loc.tile.x, loc.tile.z)) <= 2
        }) else {
            self.end_solver();
            return;
        };
        for (x, z) in hazard_flee_candidates((px, pz), (hazard.tile.x, hazard.tile.z)) {
            if walk(driver, x, z) {
                break;
            }
        }
    }

    /// Copy the mime's performance: watch the mime NPC's anim; when the
    /// emote chat (6543) opens, press the button for the last seen
    /// emote, once per chat-open (rs2b0t `performMimeStage`). The
    /// trapped hold is the square's; the act ends when the player leaves
    /// the stage.
    fn step_mime<D: Driver>(&mut self, driver: &mut D, snap: &GameSnapshot) {
        if !on_mime_square(snap) {
            self.end_solver();
            return;
        }
        // Watch the mime NPC (rs2b0t watches every frame).
        for npc in snap.npcs() {
            if npc
                .name
                .as_deref()
                .is_some_and(|n| n.eq_ignore_ascii_case("mime"))
                && mime_answer(npc.animation).is_some()
            {
                self.mime_mut().last_seen = Some(npc.animation);
            }
        }
        // Emote chat up: answer with the last seen emote, then wait for
        // the chat to close (a still-open chat must not re-press).
        let stage = self.mime_stage();
        if snap.modals().chat == MIME_IF_ROOT {
            if !stage.answered {
                if let Some(answer) = stage.last_seen.and_then(mime_answer) {
                    press(driver, MIME_IF_BUTTONS[answer]);
                    self.mime_mut().answered = true;
                }
            }
            return;
        }
        self.mime_mut().answered = false;
    }

    /// Solve boxes from their modal, inventory and server-message outcomes.
    /// Missing outcomes retry within observed game ticks, then release the
    /// slot with the last failure retained until the event leaves.
    fn step_box<D: Driver>(&mut self, driver: &mut D, snap: &GameSnapshot) {
        let count: i32 = snap
            .inventory()
            .iter()
            .filter(|item| item.def.id == STRANGE_BOX_OBJ)
            .map(|item| item.count.max(0))
            .sum();
        if count == 0 {
            self.end_family(RandomKind::Box);
            self.end_solver();
            return;
        }
        let tick = snap.tick();
        match self.box_solve().phase {
            BoxPhase::Stalled => {
                self.end_solver();
                return;
            }
            BoxPhase::Answering {
                tick: sent,
                count: before,
                sequence,
            } => {
                let succeeded = count < before
                    || Self::server_chat_has_after(snap, sequence, |text| {
                        text.contains(
                            "You have completed this cube, but there are more for you to do.",
                        )
                    });
                if succeeded {
                    *self.box_mut() = BoxSolve {
                        phase: BoxPhase::Retry,
                        ..BoxSolve::default()
                    };
                } else if Self::server_chat_has_after(snap, sequence, |text| {
                    text.contains("Oh dear, you have answered the question incorrectly")
                }) {
                    self.fail_box(BoxFailure::WrongAnswer);
                    return;
                } else if tick.wrapping_sub(sent) >= BOX_OUTCOME_TICKS {
                    self.fail_box(BoxFailure::AnswerTimeout);
                    return;
                } else {
                    return;
                }
            }
            BoxPhase::Opening { tick: sent } if snap.modals().main != CUBE_IF_ROOT => {
                if tick.wrapping_sub(sent) >= BOX_OUTCOME_TICKS {
                    self.fail_box(BoxFailure::OpenTimeout);
                }
                return;
            }
            _ => {}
        }
        if snap.modals().main == CUBE_IF_ROOT && self.box_solve().phase != BoxPhase::Retry {
            let started = match self.box_solve().phase {
                BoxPhase::Opening { tick } => tick,
                _ => {
                    self.box_mut().phase = BoxPhase::Opening { tick };
                    tick
                }
            };
            let ctx = ReadContext::new(snap);
            let question = ctx.component_text(CUBE_IF_QUESTION).unwrap_or("");
            let models = CUBE_IF_MODELS.map(|id| ctx.component_model_obj_id(id));
            let Some(answer) = solve_cube(question, models) else {
                if tick.wrapping_sub(started) >= BOX_OUTCOME_TICKS {
                    self.fail_box(if models.iter().any(Option::is_none) {
                        BoxFailure::MissingModels
                    } else {
                        BoxFailure::UnknownQuestion
                    });
                }
                return;
            };
            if press(driver, CUBE_IF_BUTTONS[answer]) {
                self.box_mut().phase = BoxPhase::Answering {
                    tick,
                    count,
                    sequence: snap.chat_lines().first().map_or(0, |line| line.sequence),
                };
            } else {
                self.fail_box(BoxFailure::AnswerRefused);
            }
            return;
        }
        let Some(held) = snap
            .inventory()
            .iter()
            .find(|item| item.def.id == STRANGE_BOX_OBJ)
        else {
            return;
        };
        let mut ix = Interactions::new(snap, driver);
        match ix.interact(OpTarget::Item(held), ActionSpec::Label("Open".to_string())) {
            SendResult::Sent { .. } => self.box_mut().phase = BoxPhase::Opening { tick },
            SendResult::Refused { reason, .. } => self.fail_box(BoxFailure::OpenRefused(reason)),
        }
    }

    fn fail_box(&mut self, failure: BoxFailure) {
        let solve = self.box_mut();
        solve.failures += 1;
        solve.last_failure = Some(failure);
        if solve.failures >= BOX_MAX_FAILURES {
            solve.phase = BoxPhase::Stalled;
            let (failures, last_failure) = (solve.failures, solve.last_failure);
            self.end_solver();
            api::host_log!(
                Category::RandomEvent,
                Level::Warn,
                "strange box: stopped after {failures} failed attempts; last failure: {last_failure:?}"
            );
        } else {
            solve.phase = BoxPhase::Retry;
        }
    }

    /// One maze solver step per tick (rs2b0t `solveMaze`): solve the
    /// route from the observed tile, then drive the door / shrine phase
    /// machine. No route → log and keep the trapped hold (never replay a
    /// different spawn's route). The hold lifts on its own once the
    /// player is no longer on the maze square; a visit still on it after
    /// [`maze::SOLVE_TICKS`] solver ticks is given up (inert, no hold).
    fn step_maze<D: Driver>(&mut self, driver: &mut D, snap: &GameSnapshot) {
        let Some((px, pz, level)) = snap.tile() else {
            self.end_solver();
            return;
        };
        if api::random::trapped_area(px, pz, level) != Some(RandomKind::Maze) {
            self.end_solver();
            return;
        }
        let visit = self.maze_mut();
        visit.ticks += 1;
        if visit.ticks > maze::SOLVE_TICKS {
            api::host_log!(
                Category::RandomEvent,
                Level::Warn,
                "maze: still inside after {} solver ticks at ({px},{pz}); giving the Maze up and releasing the slot",
                maze::SOLVE_TICKS
            );
            visit.solve = None;
            visit.stalled = true;
            self.end_solver();
            return;
        }
        if visit.solve.is_none() {
            let me = (px, pz);
            match maze::select_route(maze::graph(), me) {
                Some(doors) => {
                    api::host_log!(
                        Category::RandomEvent,
                        Level::Info,
                        "maze: spawn ({px},{pz}) -> {} doors, first ({},{})",
                        doors.len(),
                        doors[0].0,
                        doors[0].1
                    );
                    visit.solve = Some(maze::MazeSolve::new(doors));
                }
                None => {
                    api::host_log!(
                        Category::RandomEvent,
                        Level::Warn,
                        "maze: no route solvable from ({px},{pz}); the layout does not reach the shrine from here"
                    );
                }
            }
            return;
        }
        let keep = step_maze_phase(
            visit.solve.as_mut().expect("checked above"),
            driver,
            snap,
            (px, pz),
        );
        if !keep {
            api::host_log!(
                Category::RandomEvent,
                Level::Warn,
                "maze: pass gave up; restarting the route from ({px},{pz})"
            );
            self.maze_mut().solve = None;
        }
    }

    /// Lamp auto-use, driven to a real redemption: Rub (held op 1,
    /// `opheld1`), wait for the skill IF (2808), press the vault
    /// `lamp_skill` button, then Confirm (2831) — after which the server
    /// runs `xplamp_confirm`: it closes the IF, consumes ONE lamp,
    /// `stat_advance`s the chosen skill and opens the award `mesbox`. A
    /// redemption completes on the count drop below the Confirm-tick count,
    /// the observed reward and the drained award dialogue; the next lamp in
    /// the pack then gets its own Rub. A Rub whose skill IF never opens is
    /// paused and re-Rubbed from a fresh snapshot (a resolving script burial
    /// can shift the lamp out from under the Rub); after [`MAX_LAMP_RUBS`]
    /// misses the episode pauses for [`LAMP_STALL_COOLDOWN`] and starts over.
    /// A refused Confirm (lamp still held) or an unknown skill latches the
    /// lamp until it leaves the pack or `lamp_auto` goes off. Confirm is
    /// never replayed. `lamp_auto` off keeps the 0.1.2 behavior (detect, no
    /// op, no hold).
    fn step_lamp<D: Driver>(
        &mut self,
        driver: &mut D,
        snap: &GameSnapshot,
        settings: &ProfileSettings,
    ) {
        if !settings.lamp_auto {
            // Detect-only: tick() already skipped act + ours. Keep the
            // latch down if we still land here (live toggle mid-step).
            self.clear_lamp();
            self.end_solver();
            return;
        }
        let lamp_here = lamp_held(snap);
        if self.lamp_flow().confirmed {
            // Confirm is out: the redemption is in flight.
            self.step_lamp_redeem(driver, snap, settings);
            return;
        }
        if !lamp_here {
            // The lamp left the pack without a Confirm of ours (another
            // actor consumed it): nothing left to drive.
            self.clear_lamp();
            self.end_solver();
            return;
        }
        if snap.modals().main == LAMP_IF_ROOT {
            self.lamp_flow_mut().wait = 0;
            if !self.lamp_flow().skill_sent {
                let Some(btn) = lamp_skill_button(&settings.lamp_skill) else {
                    // An unknown `lamp_skill` cannot be clicked at all.
                    // No click (fail closed) — and no endless hold behind
                    // a lamp the host will not redeem.
                    api::host_log!(
                        Category::RandomEvent,
                        Level::Warn,
                        "lamp: unknown skill {:?}",
                        settings.lamp_skill
                    );
                    self.stall_lamp();
                    return;
                };
                press(driver, btn);
                self.lamp_flow_mut().skill_sent = true;
                return;
            }
            // The skill press has settled: confirm, and take the reward
            // baseline from *this* tick's snapshot.
            let reward = stat_by_name(snap, &settings.lamp_skill).map(|s| (s.xp, s.base));
            let flow = self.lamp_flow_mut();
            flow.reward = reward;
            flow.dialog = 0;
            flow.wait = 0;
            flow.confirmed = true;
            flow.confirm_count = Some(lamp_count(snap));
            press(driver, LAMP_IF_CONFIRM);
            return;
        }
        self.lamp_flow_mut().skill_sent = false;
        if self.lamp_flow().rubbed {
            // Waiting for the skill IF to open after the Rub.
            self.lamp_flow_mut().wait += 1;
            if self.lamp_flow().wait > MAX_LAMP_WAIT {
                if self.lamp.rubs < MAX_LAMP_RUBS {
                    // The Rub may have hit a stale slot (a script burial
                    // resolving after the snapshot it was built from
                    // compacts the pack and shifts the lamp, so the server
                    // ignores the op and no IF opens). Pause, then re-resolve
                    // and re-Rub from a fresh snapshot. Confirm is never
                    // replayed.
                    api::host_log!(
                        Category::RandomEvent,
                        Level::Info,
                        "lamp: the skill interface never opened; re-rubbing (rub {} of {})",
                        self.lamp.rubs + 1,
                        MAX_LAMP_RUBS,
                    );
                    self.pause_lamp(LAMP_RETRY_COOLDOWN);
                } else {
                    api::host_log!(
                        Category::RandomEvent,
                        Level::Warn,
                        "lamp: the skill interface never opened in {} rubs; pausing {} ticks",
                        MAX_LAMP_RUBS,
                        LAMP_STALL_COOLDOWN,
                    );
                    self.lamp.rubs = 0;
                    self.pause_lamp(LAMP_STALL_COOLDOWN);
                }
            }
            return;
        }
        let Some(lamp) = snap.inventory().iter().find(|i| i.def.id == LAMP_OBJ) else {
            self.end_solver();
            return;
        };
        let mut ix = Interactions::new(snap, driver);
        match ix.interact(OpTarget::Item(lamp), ActionSpec::Label("Rub".to_string())) {
            SendResult::Sent { .. } => {
                let flow = self.lamp_flow_mut();
                flow.rubbed = true;
                flow.wait = 0;
                self.lamp.rubs += 1;
            }
            SendResult::Refused { .. } => self.end_solver(),
        }
    }

    /// The post-Confirm half of [`Guardian::step_lamp`]. The redemption is
    /// complete when the pack count dropped below the Confirm-tick count
    /// (one lamp consumed), the chosen skill's stat advanced past the
    /// Confirm-tick baseline, and the award dialogue drained. The lamp
    /// leaving the pack alone is not completion, and a sibling lamp still in
    /// the pack does not block it: a completed redemption clears the per-lamp
    /// state so the next lamp is rubbed. Nothing here replays a confirmation
    /// or drains a dialogue the Confirm did not open.
    fn step_lamp_redeem<D: Driver>(
        &mut self,
        driver: &mut D,
        snap: &GameSnapshot,
        settings: &ProfileSettings,
    ) {
        let flow = self.lamp_flow();
        let consumed = flow
            .confirm_count
            .is_some_and(|before| lamp_count(snap) < before);
        let rewarded = reward_landed(snap, &settings.lamp_skill, flow.reward);
        let chat = chat_is_open(snap);
        if consumed {
            if rewarded && !chat {
                // Consumed, awarded and the award dialogue drained: the
                // redemption is real. Release the hold; the next lamp, if
                // any, starts from a fresh Rub.
                self.clear_lamp();
                self.end_solver();
                return;
            }
            if chat && flow.dialog < MAX_LAMP_DIALOGUE {
                let mut ix = Interactions::new(snap, driver);
                if matches!(ix.continue_dialog(None), SendResult::Sent { .. }) {
                    let flow = self.lamp_flow_mut();
                    flow.dialog += 1;
                    flow.wait = 0;
                    return;
                }
            }
        }
        self.lamp_flow_mut().wait += 1;
        if self.lamp_flow().wait <= MAX_LAMP_WAIT {
            return;
        }
        if consumed {
            // The lamp is gone; its reward or award dialogue never showed.
            // Nothing is left to protect, so release without latching: a
            // sibling lamp still in the pack is rubbed next.
            api::host_log!(
                Category::RandomEvent,
                Level::Warn,
                "lamp: the reward never completed; releasing the hold"
            );
            self.clear_lamp();
            self.end_solver();
        } else {
            // Confirm did not take: the lamp is still held and the IF stayed
            // open or the selection was refused. Latch without replaying.
            api::host_log!(
                Category::RandomEvent,
                Level::Warn,
                "lamp: redemption did not complete; giving up"
            );
            self.stall_lamp();
        }
    }

    /// Drop the lamp flow and its history (a resolved, consumed or abandoned
    /// lamp).
    fn clear_lamp(&mut self) {
        self.lamp = LampHistory::default();
        self.end_family(RandomKind::Lamp);
    }

    /// Pause the Rub flow for `ticks` fresh ticks: the lamp is inert while
    /// the pause counts down, so the script runs between attempts. The Rub
    /// budget `rubs` survives the pause; the caller resets it when a new
    /// episode starts.
    fn pause_lamp(&mut self, ticks: u32) {
        self.end_family(RandomKind::Lamp);
        self.lamp.park = ticks;
        self.end_solver();
    }

    /// A stalled redemption or a paused Rub: detected, but not acted on or
    /// held until it resolves.
    fn lamp_inert(&self) -> bool {
        self.lamp.stalled || self.lamp.park > 0
    }

    /// Give up on this lamp: release the hold, keep the latch that stops
    /// `tick` from acting/holding again until the lamp leaves the pack or
    /// `lamp_auto` goes off.
    fn stall_lamp(&mut self) {
        self.clear_lamp();
        self.lamp.stalled = true;
        self.end_solver();
    }

    /// Take the named lost fishing gear from the ground (Chebyshev ≤ 10,
    /// the client walks the take). A full pack drops one sacrificial
    /// item first so the Take lands.
    fn step_lost_gear<D: Driver>(
        &mut self,
        driver: &mut D,
        snap: &GameSnapshot,
        ev: &DetectedRandom,
    ) {
        let gear = ev.name.as_str();
        let Some(item) = snap
            .ground_items()
            .iter()
            .find(|g| g.distance <= LOST_GEAR_RADIUS && item_named(g.def.name.as_deref(), gear))
        else {
            self.end_solver();
            return;
        };
        if pack_full(snap) {
            if let Some(junk) = sacrificial_item(snap) {
                let mut ix = Interactions::new(snap, driver);
                let _ = ix.interact(OpTarget::Item(junk), ActionSpec::Label("Drop".to_string()));
                return;
            }
        }
        let mut ix = Interactions::new(snap, driver);
        match ix.interact(
            OpTarget::GroundItem(item),
            ActionSpec::Label("Take".to_string()),
        ) {
            SendResult::Sent { .. } => {}
            SendResult::Refused { .. } => self.end_solver(),
        }
    }

    /// Refit only from the exact head dropped by this event. Search checks
    /// distance 0 through 7 from the last pre-event and first-evidence tiles.
    /// Content waits use PLAYER_INFO ticks.
    fn step_lost_tool<D: Driver>(&mut self, driver: &mut D, snap: &GameSnapshot) {
        let handle_inv = snap
            .inventory()
            .iter()
            .find(|i| is_tool_handle(i.def.name.as_deref()));
        let handle_worn = snap
            .equipment()
            .iter()
            .find(|i| is_tool_handle(i.def.name.as_deref()));
        let Some(handle) = handle_inv.or(handle_worn) else {
            self.end_solver();
            return;
        };
        let Some(kind) = tool_part(handle.def.name.as_deref(), "handle") else {
            self.end_solver();
            return;
        };
        if self.tool_recovery().is_none() {
            self.begin_lost_tool(snap);
        }
        let Some((recovery_kind, event_tick, expected_head_id, origin_tiles, mut target)) =
            self.tool_recovery().map(|recovery| {
                (
                    recovery.kind,
                    recovery.event_tick,
                    recovery.expected_head_id,
                    recovery.origin_tiles,
                    recovery.target,
                )
            })
        else {
            self.end_solver();
            return;
        };
        if recovery_kind != kind {
            self.tool_stalled = true;
            self.end_solver();
            return;
        }
        let tick = snap.tick();
        let expected_head_in_pack = expected_head_id.is_some_and(|id| {
            snap.inventory()
                .iter()
                .any(|item| item.count > 0 && item.def.id == id)
        });
        if target.is_none() && !expected_head_in_pack {
            let candidate = snap
                .ground_items()
                .iter()
                .filter_map(|ground| {
                    let distance = lost_tool_landing_distance(
                        origin_tiles,
                        (ground.tile.x, ground.tile.z, ground.tile.level),
                    )?;
                    if tool_part(ground.def.name.as_deref(), "head") != Some(kind)
                        || expected_head_id.is_some_and(|id| id != ground.def.id)
                        || !self.lost_tool_ground_count_increased(
                            snap,
                            ground.def.id,
                            (ground.tile.x, ground.tile.z, ground.tile.level),
                            kind,
                        )
                    {
                        return None;
                    }
                    Some((ground, distance))
                })
                .min_by_key(|(ground, distance)| {
                    (
                        *distance,
                        ground.tile.level,
                        ground.tile.x,
                        ground.tile.z,
                        ground.def.id,
                    )
                })
                .map(|(ground, _)| LostToolTarget {
                    id: ground.def.id,
                    tile: (ground.tile.x, ground.tile.z, ground.tile.level),
                    spawn_tick: tick,
                    missing_since_tick: None,
                });
            if let Some(candidate) = candidate {
                target = Some(candidate);
                self.set_tool_target(candidate);
            }
        }

        let target_id = target.map(|target| target.id).or(expected_head_id);
        let head_inv = target_id.and_then(|id| {
            snap.inventory()
                .iter()
                .find(|item| item.count > 0 && item.def.id == id)
        });
        if target.is_none() && head_inv.is_none() {
            if pack_full(snap) && sacrificial_item(snap).is_none() {
                self.tool_stalled = true;
                self.end_solver();
                return;
            }
            if tick.wrapping_sub(event_tick) >= LOST_TOOL_SPAWN_GRACE_TICKS {
                self.tool_stalled = true;
                self.end_solver();
                return;
            }
            if let Some(handle) = handle_worn {
                if pack_full(snap) {
                    if let Some(junk) = sacrificial_item(snap) {
                        let mut ix = Interactions::new(snap, driver);
                        let _ = ix
                            .interact(OpTarget::Item(junk), ActionSpec::Label("Drop".to_string()));
                    }
                } else {
                    let mut ix = Interactions::new(snap, driver);
                    let _ = ix.interact(
                        OpTarget::Item(handle),
                        ActionSpec::Label("Remove".to_string()),
                    );
                }
            }
            return;
        }

        let mut head_ground = None;
        if let Some(mut target) = target {
            let target_ground = snap.ground_items().iter().find(|ground| {
                ground.def.id == target.id
                    && (ground.tile.x, ground.tile.z, ground.tile.level) == target.tile
            });
            head_ground = target_ground.filter(|ground| {
                lost_tool_landing_distance(
                    origin_tiles,
                    (ground.tile.x, ground.tile.z, ground.tile.level),
                )
                .is_some()
            });
            let target_missing_in_scene =
                target_ground.is_none() && lost_tool_tile_in_scene(snap, target.tile);
            let lifetime_elapsed =
                tick.wrapping_sub(target.spawn_tick) >= LOST_TOOL_GROUND_LIFETIME_TICKS;
            // PLAYER_INFO precedes zone and inventory updates
            // (`World.ts:1108-1116`). OBJ_DEL can therefore arrive before
            // the matching inventory update. Require a later observed
            // PLAYER_INFO before settling the loss.
            if head_inv.is_none() && (target_missing_in_scene || lifetime_elapsed) {
                if target
                    .missing_since_tick
                    .is_some_and(|missing| tick.wrapping_sub(missing) >= 1)
                {
                    self.tool_stalled = true;
                    self.end_solver();
                    return;
                }
                if target.missing_since_tick.is_none() {
                    target.missing_since_tick = Some(tick);
                    self.set_tool_target(target);
                }
                return;
            }
            if target.missing_since_tick.is_some() {
                target.missing_since_tick = None;
                self.set_tool_target(target);
            }
            if head_inv.is_none() && head_ground.is_none() {
                return;
            }
        }

        if (handle_worn.is_some() || head_inv.is_none()) && pack_full(snap) {
            if let Some(junk) = sacrificial_item(snap) {
                let mut ix = Interactions::new(snap, driver);
                let _ = ix.interact(OpTarget::Item(junk), ActionSpec::Label("Drop".to_string()));
            } else {
                self.tool_stalled = true;
                self.end_solver();
            }
            return;
        }
        let refit_cutoff = self.chat_seen;
        let mut ix = Interactions::new(snap, driver);
        let result = if handle_worn.is_some() {
            ix.interact(
                OpTarget::Item(handle),
                ActionSpec::Label("Remove".to_string()),
            )
        } else if let Some(head) = head_inv {
            ix.use_item_on(head, OpTarget::Item(handle))
        } else if let Some(head) = head_ground {
            ix.interact(
                OpTarget::GroundItem(head),
                ActionSpec::Label("Take".to_string()),
            )
        } else {
            return;
        };
        if handle_worn.is_none() && head_inv.is_some() && matches!(result, SendResult::Sent { .. })
        {
            self.tool_run_mut()
                .refit_after_sequence
                .get_or_insert(refit_cutoff);
        }
    }

    fn begin_lost_tool(&mut self, snap: &GameSnapshot) {
        let handle = snap
            .inventory()
            .iter()
            .chain(snap.equipment().iter())
            .find(|item| is_tool_handle(item.def.name.as_deref()));
        let Some(kind) = handle.and_then(|item| tool_part(item.def.name.as_deref(), "handle"))
        else {
            self.tool_run_mut().recovery = None;
            return;
        };
        let tick = snap.tick();
        let onset = self.tool_onset.filter(|onset| onset.kind == kind);
        let event_tick = onset.map_or(tick, |onset| onset.tick);
        let baseline_at_onset = self
            .tool_baseline
            .tick
            .is_some_and(|previous| previous.wrapping_add(1) == event_tick);
        let expected_head_id = onset.and_then(|onset| onset.expected_head_id).or_else(|| {
            (baseline_at_onset || onset.is_some())
                .then(|| self.removed_lost_tool_head(snap, kind))
                .flatten()
        });
        let origin_tiles = onset.map_or([self.tool_baseline.tile, snap.tile()], |onset| {
            [onset.pre_event_tile, onset.observed_tile]
        });
        if !baseline_at_onset {
            self.tool_baseline.tools.clear();
            // Keep the last non-event ground baseline. The current snapshot
            // can already contain the event's delayed head.
        }
        self.tool_onset = None;
        self.tool_run_mut().recovery = Some(LostToolRecovery {
            event_tick,
            kind,
            expected_head_id,
            origin_tiles,
            target: None,
        });
    }

    /// The recovery in flight now aims at `target`.
    fn set_tool_target(&mut self, target: LostToolTarget) {
        if let Some(recovery) = self.tool_run_mut().recovery.as_mut() {
            recovery.target = Some(target);
        }
    }

    /// Ordinary woodcutting and mining also send this text.
    /// See `woodcut.rs2:83` and `mining.rs2:71`.
    /// A removed tool or handle must confirm loss.
    fn observe_lost_tool_onset(&mut self, snap: &GameSnapshot) {
        if self.tool_onset.is_some() {
            return;
        }
        let removed = self.removed_lost_tool_item(snap);
        let handle_kind = snap
            .inventory()
            .iter()
            .chain(snap.equipment().iter())
            .find_map(|item| tool_part(item.def.name.as_deref(), "handle"));
        let Some(kind) = removed.map(|(kind, _)| kind).or(handle_kind) else {
            return;
        };
        self.tool_onset = Some(LostToolOnset {
            tick: snap.tick(),
            kind,
            pre_event_tile: self.tool_baseline.tile,
            observed_tile: snap.tile(),
            expected_head_id: removed
                .filter(|(removed_kind, _)| *removed_kind == kind)
                .and_then(|(_, head_id)| head_id),
        });
    }

    fn removed_lost_tool_item(&self, snap: &GameSnapshot) -> Option<(bool, Option<i32>)> {
        let mut removed = None;
        for tool in &self.tool_baseline.tools {
            let current_count = snap
                .inventory()
                .iter()
                .chain(snap.equipment().iter())
                .filter(|item| item.def.id == tool.id)
                .map(|item| item.count)
                .fold(0, i32::saturating_add);
            if tool.count > current_count {
                if removed.is_some() {
                    return None;
                }
                removed = Some((tool.kind, tool.head_id));
            }
        }
        removed
    }

    fn removed_lost_tool_head(&self, snap: &GameSnapshot, kind: bool) -> Option<i32> {
        self.removed_lost_tool_item(snap)
            .filter(|(removed_kind, _)| *removed_kind == kind)
            .and_then(|(_, head_id)| head_id)
    }

    fn observe_lost_tool_baseline(
        &mut self,
        snap: &GameSnapshot,
        game_data: Option<&SelectedGameData>,
    ) {
        let baseline = &mut self.tool_baseline;
        baseline.tick = Some(snap.tick());
        baseline.tile = snap.tile();
        if let Some(game_data) = game_data {
            for tool in &mut baseline.tools {
                tool.count = 0;
            }
            for item in snap.inventory().iter().chain(snap.equipment().iter()) {
                let Some(alias) = game_data
                    .item_by_id(item.def.id)
                    .and_then(|game_item| game_item.alias.as_deref())
                else {
                    continue;
                };
                let Some(kind) = lost_tool_kind(alias) else {
                    continue;
                };
                if let Some(existing) = baseline
                    .tools
                    .iter_mut()
                    .find(|existing| existing.id == item.def.id && existing.kind == kind)
                {
                    if existing.count == 0 {
                        existing.count = item.count;
                    } else {
                        existing.count = existing.count.saturating_add(item.count);
                    }
                } else {
                    baseline.tools.push(LostToolItem {
                        id: item.def.id,
                        count: item.count,
                        kind,
                        head_id: lost_tool_head_id(game_data, alias, kind),
                    });
                }
            }
            baseline.tools.retain(|tool| tool.count > 0);
        } else {
            baseline.tools.clear();
        }
        self.capture_lost_tool_ground_baseline(snap);
    }

    fn capture_lost_tool_ground_baseline(&mut self, snap: &GameSnapshot) {
        let baseline = &mut self.tool_baseline;
        baseline.tick = Some(snap.tick());
        baseline.ground_heads.clear();
        baseline
            .ground_heads
            .extend(snap.ground_items().iter().filter_map(|ground| {
                Some(LostToolGroundHead {
                    id: ground.def.id,
                    tile: (ground.tile.x, ground.tile.z, ground.tile.level),
                    count: ground.count,
                    kind: tool_part(ground.def.name.as_deref(), "head")?,
                })
            }));
    }

    fn lost_tool_ground_count_increased(
        &self,
        snap: &GameSnapshot,
        id: i32,
        tile: (i32, i32, i32),
        kind: bool,
    ) -> bool {
        let current: i64 = snap
            .ground_items()
            .iter()
            .filter(|ground| {
                ground.def.id == id && (ground.tile.x, ground.tile.z, ground.tile.level) == tile
            })
            .map(|ground| i64::from(ground.count))
            .sum();
        let previous: i64 = self
            .tool_baseline
            .ground_heads
            .iter()
            .filter(|ground| ground.id == id && ground.tile == tile && ground.kind == kind)
            .map(|ground| i64::from(ground.count))
            .sum();
        current > previous
    }

    /// Each handle accepts every head tier of its part kind. The default
    /// switch branch sends `dm_default`, "Nothing interesting happens."
    /// (`macro_event_lost_axe.rs2:41-45`; `macro_event_lost_pickaxe.rs2:42-46`;
    /// `displaymessage.enum:5`). Only the server line counts. A player
    /// line with the same text must not stop the repair.
    fn observe_lost_tool_refit_rejection(&mut self, snap: &GameSnapshot) {
        let Some(after_sequence) = self.tool_run().and_then(|run| run.refit_after_sequence) else {
            return;
        };
        let rejected = Self::server_chat_has_after(snap, after_sequence, |text| {
            text.trim() == "Nothing interesting happens."
        });
        if rejected {
            if let Some(Family::LostTool(run)) =
                self.active.as_mut().map(|active| &mut active.family)
            {
                run.refit_after_sequence = None;
            }
            self.tool_stalled = true;
            self.end_solver();
        }
    }

    /// The handle lifts when the chat is fully closed and the in-flight
    /// NPC has left the scene (the spec's "NPC gone and chat closed").
    /// Continue/answer refuse clears via [`Guardian::act`] when chat is
    /// closed (after a one-tick grace post Talk-to).
    fn dialog_done(&self, snap: &GameSnapshot) -> bool {
        let npc_here = self.dialog().is_some_and(|handle| {
            snap.npcs()
                .iter()
                .any(|v| v.index == usize::from(handle.index))
        });
        !chat_is_open(snap) && !npc_here
    }

    fn clear_handle(&mut self) {
        if let Some(active) = self.active.as_mut() {
            active.dialog = None;
        }
        self.settle();
    }
}

/// The tracked hostile actor's facts one flee decision reads.
#[derive(Debug, Clone, Copy)]
struct FleeFacts {
    index: usize,
    maxrange: Option<i32>,
    flee_distance: i32,
    start_tile: (i32, i32),
    outside_ticks: u8,
}

/// One flee decision of the flee in flight `run`. Returns whether a flee walk
/// was accepted.
fn flee_step<D: Driver + ?Sized>(
    driver: &mut D,
    snap: &GameSnapshot,
    run: &mut EvadeRun,
    facts: FleeFacts,
) -> bool {
    let FleeFacts {
        index,
        maxrange,
        flee_distance,
        start_tile,
        outside_ticks,
    } = facts;
    if run.fallback_exhausted {
        return false;
    }
    let Some((px, pz, level)) = snap.tile() else {
        return false;
    };
    let player = (px, pz);
    let player_tile = api::snapshot::WorldTile {
        x: px,
        z: pz,
        level,
    };
    let Some(maxrange) = maxrange else {
        return false;
    };
    let npc = npc_by_index(snap.npcs(), index);
    let engaged = npc.is_some_and(|npc| {
        event_npc_targets_player(npc, snap.self_slot()) || (npc.target.is_none() && npc.in_combat)
    });
    let reached_flee = cheb(player, start_tile) >= flee_distance;
    if engaged
        && outside_chase_range(player, start_tile, maxrange)
        && reached_flee
        && outside_ticks < 2
    {
        return false;
    }
    if !engaged
        && npc.map_or(reached_flee, |npc| {
            api::query::chebyshev_to(player_tile, npc.network) > 3
        })
    {
        return false;
    }
    let moving = snap
        .local_player()
        .is_some_and(|local| local.player.actor.moving);
    if run.flee_target.is_some() {
        if moving {
            return false;
        }
        run.flee_target = None;
        run.fallback_distance = None;
        run.fallback_exhausted = false;
    }
    let distance = flee_distance.max(cheb(player, start_tile).saturating_add(1));
    if run.fallback_distance.is_none() {
        for (x, z) in flee_candidates_away(start_tile, player, distance) {
            if walk(driver, x, z) {
                run.flee_target = Some((x, z));
                return true;
            }
        }
        run.fallback_distance = Some(distance);
    }
    let fallback_distance = run.fallback_distance.expect("fallback ring is set");
    for (x, z) in flee_candidates_away(start_tile, player, fallback_distance) {
        if walk_nearest(driver, x, z) {
            run.flee_target = Some((x, z));
            run.fallback_distance = None;
            return true;
        }
    }
    if fallback_distance > 1 {
        run.fallback_distance = Some(fallback_distance - 1);
    } else {
        run.fallback_distance = None;
        run.fallback_exhausted = true;
    }
    false
}
/// Whether the current scene is complete for the target tile. A missing
/// object is evidence of despawn only while its tile remains in the scene.
fn lost_tool_tile_in_scene(snap: &GameSnapshot, (x, z, level): (i32, i32, i32)) -> bool {
    let scene = snap.scene();
    if !scene.available || scene.level != level {
        return false;
    }
    let Some(local_x) = x.checked_sub(scene.base_x) else {
        return false;
    };
    let Some(local_z) = z.checked_sub(scene.base_z) else {
        return false;
    };
    local_x >= 0 && local_x < scene.width && local_z >= 0 && local_z < scene.height
}

/// Distance from an observed event-origin candidate to a content-eligible
/// landing tile. The player can move between those two snapshots.
fn lost_tool_landing_distance(
    origins: [Option<(i32, i32, i32)>; 2],
    tile: (i32, i32, i32),
) -> Option<i32> {
    origins
        .into_iter()
        .flatten()
        .filter_map(|(origin_x, origin_z, origin_level)| {
            if tile.2 != origin_level {
                return None;
            }
            let distance = cheb((origin_x, origin_z), (tile.0, tile.1));
            (0..=LOST_TOOL_RADIUS)
                .contains(&distance)
                .then_some(distance)
        })
        .min()
}

fn lost_tool_kind(alias: &str) -> Option<bool> {
    let alias = alias.strip_prefix("macro_").unwrap_or(alias);
    if alias
        .strip_suffix("_pickaxe")
        .is_some_and(|tier| !tier.is_empty())
    {
        Some(true)
    } else if alias
        .strip_suffix("_axe")
        .is_some_and(|tier| !tier.is_empty())
    {
        Some(false)
    } else {
        None
    }
}

fn lost_tool_head_id(game_data: &SelectedGameData, tool_alias: &str, kind: bool) -> Option<i32> {
    let tool_alias = tool_alias.strip_prefix("macro_").unwrap_or(tool_alias);
    let (tier, part) = if kind {
        (tool_alias.strip_suffix("_pickaxe")?, "_pickaxehead")
    } else {
        (tool_alias.strip_suffix("_axe")?, "_hatchethead")
    };
    game_data.items().iter().find_map(|item| {
        let alias = item.alias.as_deref()?.strip_prefix("macro_")?;
        (alias.strip_prefix(tier)? == part).then_some(item.id)
    })
}

/// One `try_move` toward `target`; false when the walk is stuck
/// ([`maze::WALK_LIMIT`] sends without a tile change — the door is
/// walled off).
fn maze_walk_step<D: Driver>(
    st: &mut maze::MazeSolve,
    driver: &mut D,
    me: (i32, i32),
    target: (i32, i32),
) -> bool {
    if st.walk_from != Some(me) {
        st.walk_from = Some(me);
        st.walk_sends = 0;
    }
    st.walk_sends += 1;
    if st.walk_sends > maze::WALK_LIMIT {
        return false;
    }
    walk_nearest(driver, target.0, target.1);
    true
}

/// `oploc` Open on a route door (ids 3628–3632 at that tile).
fn maze_send_open<D: Driver>(driver: &mut D, tile: (i32, i32)) {
    let loc_id = maze::graph()
        .door_id
        .get(&tile)
        .copied()
        .unwrap_or(maze::MAZE_DOOR_IDS[0]);
    op_loc(driver, tile.0, tile.1, loc_id);
}

/// `oploc` Touch on the shrine (loc 3634) and wait to leave the square.
fn maze_send_touch<D: Driver>(st: &mut maze::MazeSolve, driver: &mut D, pass: u32) {
    op_loc(
        driver,
        maze::MAZE_SHRINE.0,
        maze::MAZE_SHRINE.1,
        maze::MAZE_SHRINE_LOC,
    );
    st.phase = maze::MazePhase::TouchWait;
    st.touch_pass = pass;
    st.wait_ticks = 0;
}

/// One maze phase step (rs2b0t `solveMaze` loop body). Returns false
/// when the pass gives up and the route restarts.
fn step_maze_phase<D: Driver>(
    st: &mut maze::MazeSolve,
    driver: &mut D,
    snap: &GameSnapshot,
    me: (i32, i32),
) -> bool {
    // A mesbox/briefing chat is drained first; while it is up nothing
    // else happens. A chat during an in-flight open is the wrong-door
    // refusal mesbox (rs2b0t clears it, then continues the route).
    if chat_is_open(snap) {
        if matches!(
            st.phase,
            maze::MazePhase::OpenDoor { .. }
                | maze::MazePhase::OpenResync { .. }
                | maze::MazePhase::OpenShrine { .. }
        ) {
            st.refused = true;
        }
        if st.continues < maze::MESBOX_LIMIT {
            st.continues += 1;
            let mut ix = Interactions::new(snap, driver);
            let _ = ix.continue_dialog(None);
        }
        return true;
    }
    st.continues = 0;

    match st.phase {
        maze::MazePhase::WalkDoor => {
            let Some(door) = st.target() else {
                // A route ending at the chamber door already opened it.
                // Regenerated routes that missed it retain the fallback.
                st.phase = if st.doors.last() == Some(&maze::MAZE_SHRINE_DOOR) {
                    maze::MazePhase::Touch { pass: 0 }
                } else {
                    maze::MazePhase::ShrineDoor
                };
                st.touch_pass = 0;
                return true;
            };
            if cheb(me, door) <= 1 {
                maze_send_open(driver, door);
                st.phase = maze::MazePhase::OpenDoor { from: me };
                return true;
            }
            if !maze_walk_step(st, driver, me, door) {
                // Walled off: step back through the previous door.
                if st.next == 0 || st.resyncs >= maze::MAX_RESYNCS {
                    return false;
                }
                st.resyncs += 1;
                st.walk_from = None;
                st.walk_sends = 0;
                st.wait_ticks = 0;
                st.phase = maze::MazePhase::Resync;
            }
            true
        }
        maze::MazePhase::OpenDoor { from } => {
            if cheb(me, from) >= 2 || st.refused || st.wait_ticks >= maze::OPEN_WAIT {
                st.refused = false;
                st.next += 1;
                st.phase = maze::MazePhase::WalkDoor;
                st.walk_from = None;
                st.walk_sends = 0;
                st.wait_ticks = 0;
                return true;
            }
            st.wait_ticks += 1;
            true
        }
        maze::MazePhase::Resync => {
            let Some(prev_door) = st.target() else {
                return false;
            };
            if cheb(me, prev_door) <= 1 {
                maze_send_open(driver, prev_door);
                st.phase = maze::MazePhase::OpenResync { from: me };
                return true;
            }
            if !maze_walk_step(st, driver, me, prev_door) {
                // The previous door is walled off too: retry the route
                // door, which re-counts a resync (rs2b0t the same way).
                st.phase = maze::MazePhase::WalkDoor;
                st.walk_from = None;
                st.walk_sends = 0;
                st.wait_ticks = 0;
            }
            true
        }
        maze::MazePhase::OpenResync { from } => {
            if cheb(me, from) >= 2 || st.refused || st.wait_ticks >= maze::OPEN_WAIT {
                st.refused = false;
                // Back on the route: retry the walled-off door.
                st.phase = maze::MazePhase::WalkDoor;
                st.walk_from = None;
                st.walk_sends = 0;
                st.wait_ticks = 0;
                return true;
            }
            st.wait_ticks += 1;
            true
        }
        maze::MazePhase::ShrineDoor => {
            if cheb(me, maze::MAZE_SHRINE_DOOR) <= 1 {
                maze_send_open(driver, maze::MAZE_SHRINE_DOOR);
                st.phase = maze::MazePhase::OpenShrine { from: me };
                return true;
            }
            if !maze_walk_step(st, driver, me, maze::MAZE_SHRINE_DOOR) {
                // The chamber door is unreachable: give up this pass.
                return false;
            }
            true
        }
        maze::MazePhase::OpenShrine { from } => {
            if cheb(me, from) >= 2 || st.refused || st.wait_ticks >= maze::OPEN_WAIT {
                st.refused = false;
                st.phase = maze::MazePhase::Touch {
                    pass: st.touch_pass,
                };
                st.walk_from = None;
                st.walk_sends = 0;
                st.wait_ticks = 0;
                return true;
            }
            st.wait_ticks += 1;
            true
        }
        maze::MazePhase::Touch { pass } => {
            if pass >= maze::TOUCH_LIMIT {
                // Still inside after all passes: restart the route.
                return false;
            }
            // Pass 0 near the shrine (the post-door tile): touch now.
            if pass == 0 && cheb(me, maze::MAZE_SHRINE) <= 2 {
                maze_send_touch(st, driver, pass);
                return true;
            }
            let stand = maze::TOUCH_STANDS[pass as usize % maze::TOUCH_STANDS.len()];
            let onto = pass % 2 == 0;
            let reached = if onto {
                me == stand
            } else {
                cheb(me, stand) <= 1
            };
            if reached {
                maze_send_touch(st, driver, pass);
                return true;
            }
            if !maze_walk_step(st, driver, me, stand) {
                // A walled-off stand: the next pass.
                st.phase = maze::MazePhase::Touch { pass: pass + 1 };
                st.walk_from = None;
                st.walk_sends = 0;
                st.wait_ticks = 0;
            }
            true
        }
        maze::MazePhase::TouchWait => {
            st.wait_ticks += 1;
            if st.wait_ticks >= maze::TOUCH_WAIT {
                st.wait_ticks = 0;
                let pass = st.touch_pass;
                st.touch_pass = pass + 1;
                // rs2b0t re-opens the chamber door on odd passes.
                if pass % 2 == 1 {
                    st.phase = maze::MazePhase::ShrineDoor;
                } else {
                    st.phase = maze::MazePhase::Touch { pass: pass + 1 };
                }
            }
            true
        }
    }
}

/// Whether the NPC chat modal (continue button, choice buttons, or chat
/// root) is up — the same open check dialog_done / refuse-clear share.
fn chat_is_open(snap: &GameSnapshot) -> bool {
    snap.chat_continue_component_id() != -1
        || !snap.chat_options().is_empty()
        || snap.modals().chat != -1
}
#[cfg(test)]
#[path = "../random_tests.rs"]
mod tests;
