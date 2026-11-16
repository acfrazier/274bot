//! Random-event data types shared across the crate boundary: `host`
//! detects and guards random events, `script` answers the `on_random`
//! knock, and `host-play`/`panel`/`tui` bind the status row. The
//! detect/act machine stays in `host`; this module only carries the
//! cross-crate contracts (guardian spec `2026-09-01-random-event-guardian-design.md`).

/// The kind of random event one detection found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RandomKind {
    Dialog,
    Pick,
    Evade,
    Maze,
    Mime,
    Box,
    Lamp,
    Hazard,
    LostTool,
    LostGear,
}

/// One detected random event on the current snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetectedRandom {
    pub kind: RandomKind,
    pub name: String,
    pub ours: bool,
    pub npc_index: Option<usize>,
}

/// Who handles a detected random event. `Host` (the default) lets the
/// host guardian act and hold; `Handle` means the running script owns the
/// event — ticks and follow keep running and the host does not act.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RandomClaim {
    Host,
    Handle,
}

/// The Maze random's map square (`[mapzone,0_45_71]`,
/// `[random_event_maze_zones]`), as `(x >> 6, z >> 6)` at level 0.
pub const MAZE_SQUARE: (i32, i32) = (45, 71);
/// The Mime random's stage square (`[mapzone,0_31_74]`), level 0.
pub const MIME_SQUARE: (i32, i32) = (31, 74);

/// The random event whose content map square traps a player standing on
/// world tile `(x, z, level)`: the Maze or the Mime stage. Only the event
/// itself leads off these squares, so anything but its solver is stuck
/// there.
pub fn trapped_area(x: i32, z: i32, level: i32) -> Option<RandomKind> {
    if level != 0 {
        return None;
    }
    match (x >> 6, z >> 6) {
        MAZE_SQUARE => Some(RandomKind::Maze),
        MIME_SQUARE => Some(RandomKind::Mime),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Guardian Fight (V11b). The neutral contract between the host Guardian, the
// host-play adapter and the script child: it names no Combat, script, host or
// nav type (design V11D 4.1). The Guardian offers one hostile event, the child
// fights it with the shared Combat core, and every result comes back as a
// typed value, never as text.

/// The six hostile macro events a saved Fight applies to. Each spawn attacks
/// at once and can be damaged (operator approval CP5; content
/// `macro_event_*.rs2` spawn scripts).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FightEvent {
    RiverTroll,
    RockGolem,
    Zombie,
    Shade,
    Watchman,
    TreeSpirit,
}

impl FightEvent {
    /// Every event a saved Fight applies to, in display order.
    pub const ALL: [FightEvent; 6] = [
        Self::RiverTroll,
        Self::RockGolem,
        Self::Zombie,
        Self::Shade,
        Self::Watchman,
        Self::TreeSpirit,
    ];

    /// The saved policy key: the event NPC's lowercase name.
    pub const fn key(self) -> &'static str {
        match self {
            Self::RiverTroll => "river troll",
            Self::RockGolem => "rock golem",
            Self::Zombie => "zombie",
            Self::Shade => "shade",
            Self::Watchman => "watchman",
            Self::TreeSpirit => "tree spirit",
        }
    }

    /// The event the macro NPC cache type belongs to (`content/pack/npc.pack`
    /// 391-396 river troll, 413-418 golem, 419-424 zombie, 425-430 shade,
    /// 431-436 watchman, 438-443 dryad). A world namesake such as the
    /// unarmed zombie 73 or the Lost City tree spirit is not an event.
    pub const fn of_npc_type(npc_type: usize) -> Option<Self> {
        match npc_type {
            391..=396 => Some(Self::RiverTroll),
            413..=418 => Some(Self::RockGolem),
            419..=424 => Some(Self::Zombie),
            425..=430 => Some(Self::Shade),
            431..=436 => Some(Self::Watchman),
            438..=443 => Some(Self::TreeSpirit),
            _ => None,
        }
    }
}

/// Why a saved Fight does not apply to an event that has a policy row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FightExclusion {
    /// Player damage against the Swarm is capped at 0 (`antimacro.npc:141`).
    Swarm,
    /// The Strange plant, growing or angry, stays out of Fight (OA:11).
    StrangePlant,
}

/// What a saved Fight means for one policy event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FightEligibility {
    /// The event is fought.
    Applies(FightEvent),
    /// The event keeps its normal handling under a saved Fight.
    Excluded(FightExclusion),
}

/// Whether a saved Fight applies to the policy event `key` (one of the eight
/// saved keys, case-insensitive). `None` is a key with no policy row. Runtime
/// admission and both frontends explain the saved choice through this one
/// function.
pub fn fight_eligibility(key: &str) -> Option<FightEligibility> {
    let key = key.trim();
    if let Some(event) = FightEvent::ALL
        .into_iter()
        .find(|event| event.key().eq_ignore_ascii_case(key))
    {
        return Some(FightEligibility::Applies(event));
    }
    if key.eq_ignore_ascii_case("swarm") {
        return Some(FightEligibility::Excluded(FightExclusion::Swarm));
    }
    if key.eq_ignore_ascii_case("strange plant") {
        return Some(FightEligibility::Excluded(FightExclusion::StrangePlant));
    }
    None
}

/// The runtime admission: the detected event's saved key and the actor's
/// content type must name the same fightable event. A name alone never
/// admits an actor (an unarmed zombie shares the name of the event zombie).
pub fn fight_admission(key: &str, npc_type: Option<usize>) -> Option<FightEvent> {
    let FightEligibility::Applies(event) = fight_eligibility(key)? else {
        return None;
    };
    (FightEvent::of_npc_type(npc_type?) == Some(event)).then_some(event)
}

/// One offered Fight: the session that offered it and a serial that only
/// grows inside that session's Guardian. A result carries its token; a result
/// of another token is stale and changes nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FightToken {
    pub session: u32,
    pub serial: u32,
}

/// The exact event actor: its client NPC slot and its content type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FightActor {
    pub index: u16,
    pub npc_type: u16,
}

/// The escape the Guardian would walk if the fight cannot continue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FightEscape {
    /// The spawn anchor the safety radius counts from.
    pub anchor: (i32, i32),
    /// Chebyshev distance from the anchor that ends the pursuit:
    /// `max(maxrange + 2, wanderrange + 4)`.
    pub radius: i32,
    /// The Guardian's outward candidates, in the order it would try them: the
    /// escape estimate prices a walk to the nearest reachable one.
    pub ring: [(i32, i32); 8],
}

/// What the Guardian offers the script layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FightOffer {
    pub token: FightToken,
    pub event: FightEvent,
    pub actor: FightActor,
    pub escape: FightEscape,
    /// Fresh observations left in the episode's budget.
    pub budget_ticks: u16,
}

/// Why a prepared item blocks the fight.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FightPrep {
    Weapon,
    Shield,
    Ammo,
    Staff,
    Arm,
}

/// A readiness fact the Combat core could not observe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FightReadiness {
    CombatRootMissing,
    CombatRootWrong,
    StyleEchoMissing,
}

/// Why the fight hands over to Flee, or fails. Typed, so no behaviour hangs
/// on a log line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FightReason {
    /// The offered event is not one a saved Fight applies to.
    NotEligible,
    /// The slot has no script service that can host the child.
    NoService,
    /// The child could not be admitted (another owner, a held session).
    AdmissionFailed,
    /// The Hitpoints that remain cannot pay for the escape: at or below the
    /// Fight threshold with no unreserved food (design V11D 5.3).
    EscapeReserve,
    /// A content, inventory, threat or route fact is missing or stale.
    UnknownFacts,
    NoFood,
    Dragonfire,
    NoAmmo,
    NoRunes,
    NoTarget,
    Unattackable,
    SingleWayBlocked,
    Unresponsive,
    Preparation(FightPrep),
    Readiness(FightReadiness),
    /// Combat's own budget ended with episode time left.
    CombatBudget,
    /// The episode's own budget ended.
    EpisodeBudget,
    /// The Combat core refused the exact event actor.
    InvalidEventTarget,
    /// The core ended in a way an Open, Abort fight does not use.
    UnexpectedEnd,
    /// The adapter broke its own contract.
    Internal,
}

/// An explicit lifecycle command or setting change that revokes a Fight.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FightCancel {
    Paused,
    Stopped,
    Replaced,
    CardFailed,
    /// Slot stop, logout or session replacement.
    SessionEnded,
    ManualTakeover,
    /// The live policy changed from Fight to Flee.
    SettingChanged,
    /// Random-event handling went off.
    HandlingOff,
}

/// How the child's fight ended, or `Pending` while it runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FightOutcome {
    Pending,
    /// The exact actor died; cleanup is finished.
    Killed,
    /// The actor left with disappearance evidence; no kill is claimed.
    TargetGone,
    /// The fight cannot continue and the Guardian flees.
    FleeRequired(FightReason),
    Cancelled(FightCancel),
    /// The player died; the death system owns what follows.
    Died,
    Failed(FightReason),
}

impl FightOutcome {
    /// The fight has ended, one way or another.
    pub const fn is_terminal(self) -> bool {
        !matches!(self, Self::Pending)
    }
}

/// Where the child stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FightPhase {
    /// Offered; the escape estimate and admission are due by the next fresh
    /// observation.
    Offered,
    /// Combat is running against the exact actor.
    Fighting,
    /// The fight ended; admitted cleanup finishes before the hold lifts.
    Ending,
    /// Combat ended with Flee required; the child keeps one upkeep guard
    /// while the Guardian walks away.
    Escaping,
    /// Nothing is left to do.
    Done,
}

/// The child's state, as the Guardian reads it each tick.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FightRead {
    pub token: FightToken,
    pub phase: FightPhase,
    pub outcome: FightOutcome,
    /// The game tick (low 16 bits, wrap-aware) before which an admitted bite
    /// blocks every click. `None` while nothing is admitted.
    pub blocked_until: Option<u16>,
    /// The fresh observation the child last stepped on.
    pub observed_tick: u32,
}

/// What the Guardian shows the operator while it works an event (the
/// `RandomStatus` response row, design V11D 7.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResponseState {
    Fighting,
    Fleeing,
    Picking,
    /// A script claim owns the event.
    ScriptOwned,
    /// The episode ended without success; the reason says why.
    Stopped,
    /// Random-event handling is off.
    HandlingOff,
}

/// The short reason behind a response.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResponseReason {
    /// The event cannot be damaged (Swarm).
    Immune,
    /// Fight does not apply to this event (Strange plant).
    NotFought,
    /// The Hitpoints cannot pay for the escape.
    EscapeReserve,
    /// Facts the fight needs are missing.
    UnavailableFacts,
    /// The held setup cannot fight (no ammunition, no runes, no weapon).
    UnavailableSetup,
    /// A command or setting ended the fight.
    Cancelled,
    ExhaustedBudget,
    /// The fight ended; the attack target was lost or unsafe.
    FightEnded,
}

/// The typed response row of one status publication.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RandomResponse {
    pub state: ResponseState,
    pub reason: Option<ResponseReason>,
}
