//! Borrowed combat selection, danger lines and fact-derived combat formulas.
use super::frame::Frame;
use super::request::{ActorKind, ActorRef, CombatRequest, Pick, PvpRefusal, Style, Target};
use super::tables::{CombatTables, PrayerRole, StyleMask};
use super::threats::{StyleObs, Threat, ThreatSet};
use crate::native::{PvpGlobals, WildernessLevels};
use api::game_data::{DragonfireKind, NpcAttackKind, NpcNameRow, PrayerFact};
use api::gather_methods::SceneRegionInput;
use api::snapshot::{NpcView, PlayerView, WorldTile};

/// Which source authorised STARTING a fight with a player. Reported on the
/// combat report, never re-derived (PVP-DESIGN-1026 §2.8).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum PvpSource {
    /// The request field (`pvp: true`) alone.
    Request,
    /// The global operator setting (`pvp.start_fights`) alone.
    Global,
    /// Both sources.
    Both,
}

impl PvpSource {
    /// The word scripts read (`pvpSource` in the `api.combat.fight` result).
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Request => "request",
            Self::Global => "global",
            Self::Both => "both",
        }
    }

    /// The wire code of the retained outcome page (0 is no source).
    pub const fn code(self) -> u8 {
        match self {
            Self::Request => 1,
            Self::Global => 2,
            Self::Both => 3,
        }
    }

    /// The source a wire code names; `None` for 0 and unknown codes.
    pub const fn from_code(code: u8) -> Option<Self> {
        match code {
            1 => Some(Self::Request),
            2 => Some(Self::Global),
            3 => Some(Self::Both),
            _ => None,
        }
    }
}

/// The ONE authorization predicate. `Combat::begin` calls it exactly once, and
/// only when the request target is [`Target::Player`]. No other route calls it:
/// self-defence needs no opt-in (operator Q1, PVP-DESIGN-1026 §2.8).
pub const fn pvp_initiation_allowed(
    request: &CombatRequest,
    globals: PvpGlobals,
) -> Option<PvpSource> {
    match (request.pvp, globals.start_fights) {
        (true, true) => Some(PvpSource::Both),
        (true, false) => Some(PvpSource::Request),
        (false, true) => Some(PvpSource::Global),
        (false, false) => None,
    }
}

/// The attack label the content gave a player row: "Attack" (the Wilderness) or
/// "Fight" (a started duel). `None` means the row is a threat, never a target.
pub fn attack_label(actions: &[Option<String>]) -> Option<&'static str> {
    actions
        .iter()
        .flatten()
        .find_map(|action| match action.as_str() {
            "Attack" => Some("Attack"),
            "Fight" => Some("Fight"),
            _ => None,
        })
}

/// The content's own zone checks (`pvp_is_attackable`,
/// `skill_combat/scripts/pvp/pvp_combat.rs2:50-74`, `pvp_level_check` `:32-48`)
/// as the machine can evaluate them from one frame. Inside a duel pen the
/// arena rules replace the Wilderness rules. The single-way and duel-state
/// checks are not observable and are classified from server lines instead.
pub fn zone_gate(
    here: WorldTile,
    them: WorldTile,
    my_combat_level: i32,
    their_combat_level: i32,
    tables: &CombatTables,
    wilderness: Option<&dyn WildernessLevels>,
) -> Result<(), PvpRefusal> {
    if let Some(zones) = tables.selected().duel_zones() {
        if here.level == 0 {
            let inside = |x1: i32, z1: i32, x2: i32, z2: i32| {
                here.x >= x1 && here.x <= x2 && here.z >= z1 && here.z <= z2
            };
            if zones
                .pens
                .iter()
                .any(|pen| inside(pen.x1, pen.z1, pen.x2, pen.z2))
            {
                return Ok(());
            }
            let area = zones.challenge;
            if inside(area.x1, area.z1, area.x2, area.z2) {
                return Err(PvpRefusal::ChallengeArea);
            }
        }
    }
    let level = |tile: WorldTile| wilderness.map_or(0, |levels| levels.level(tile));
    let ours = level(here);
    if ours < 1 {
        return Err(PvpRefusal::OutsideWilderness);
    }
    let theirs = level(them);
    if theirs < 1 {
        return Err(PvpRefusal::OpponentOutside);
    }
    let wilderness_level = ours.min(theirs);
    let difference = my_combat_level.abs_diff(their_combat_level);
    if u32::try_from(wilderness_level).is_ok_and(|level| level < difference) {
        return Err(PvpRefusal::LevelRange {
            wilderness: u8::try_from(wilderness_level).unwrap_or(u8::MAX),
            difference: u8::try_from(difference).unwrap_or(u8::MAX),
        });
    }
    Ok(())
}

/// The ONE candidate predicate: the attack label and the zone gate. It has no
/// opt-in input (operator Q1): it is the same for a fight we start and for a
/// fight we only answer. Returns the label the attack row must carry.
pub fn player_candidate(
    row: &PlayerView,
    frame: &Frame<'_>,
    tables: &CombatTables,
) -> Result<&'static str, PvpRefusal> {
    let label = attack_label(&row.actor.actions).ok_or(PvpRefusal::NoAttackOp)?;
    zone_gate(
        frame.here,
        row.network,
        frame.local.player.combat_level,
        row.combat_level,
        tables,
        frame.wilderness,
    )?;
    Ok(label)
}

/// The refusal `begin` reports for a request that starts a fight with `name`.
/// Where WE stand comes first: outside every PvP zone the reason is the zone
/// (a real player row there carries no attack op at all, and "no attack op"
/// would hide why). Then the candidate check on the named row when it is on
/// this frame; a row not yet seen is no refusal (the machine searches for it).
pub fn begin_refusal(name: &str, frame: &Frame<'_>, tables: &CombatTables) -> Option<PvpRefusal> {
    let level = frame.local.player.combat_level;
    let here = zone_gate(
        frame.here,
        frame.here,
        level,
        level,
        tables,
        frame.wilderness,
    )
    .err();
    if here.is_some() {
        return here;
    }
    frame
        .players
        .iter()
        .find(|row| row.actor.name.as_deref() == Some(name))
        .and_then(|row| player_candidate(row, frame, tables).err())
}

/// The eleven kill statements the server sends the killer
/// (`skill_combat/scripts/pvp/pvp_combat.rs2:201-212`), each as the text
/// before and after `<displayname>`.
const KILL_LINES: [(&str, &str); 11] = [
    ("You have defeated ", "."),
    ("", " was no match for you."),
    ("With a crushing blow you finish ", "."),
    ("Can anyone defeat you? Certainly not ", "."),
    ("", " regrets the day they met you in combat."),
    ("It's all over for ", "."),
    ("", " didn't stand a chance against you."),
    ("A humiliating defeat for ", "."),
    ("", " falls before your might."),
    ("", " has won a free ticket to Lumbridge."),
    ("You were clearly a better fighter than ", "."),
];

/// Whether `line` is a server kill statement naming `name`.
pub fn is_kill_line(line: &str, name: &str) -> bool {
    KILL_LINES.iter().any(|(head, tail)| {
        line.strip_prefix(head)
            .and_then(|rest| rest.strip_suffix(tail))
            .is_some_and(|middle| middle == name)
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Lines {
    pub emergency: i32,
    pub eat: i32,
    /// The strict `hp > drink_gate` survival threshold for a three-tick potion lock.
    pub drink_gate: i32,
}

impl Lines {
    /// Raise every line by one poison fire's damage. One fire can land on
    /// any accessible tick, so a bot at a raised line can fall to the base
    /// line before its next decision.
    pub fn with_poison(self, poison_fire: i32) -> Lines {
        Lines {
            emergency: self.emergency.saturating_add(poison_fire),
            eat: self.eat.saturating_add(poison_fire),
            drink_gate: self.drink_gate.saturating_add(poison_fire),
        }
    }
}

/// Derived food/emergency/drink thresholds. Unknown danger uses the same
/// conservative half-max threshold for all three, never a fabricated zero.
pub fn lines(danger: Option<i32>, hp_max: i32) -> Lines {
    let half = hp_max.max(0).saturating_add(1) / 2;
    match danger {
        Some(danger) => {
            let danger = danger.max(0);
            Lines {
                emergency: danger.saturating_add(1),
                eat: danger.saturating_mul(2).saturating_add(1),
                drink_gate: danger.saturating_mul(3).saturating_add(1),
            }
        }
        None => Lines {
            emergency: half,
            eat: half,
            drink_gate: half,
        },
    }
}

/// A durable NPC's hitpoints earn melee offensive prayers/boosts.
fn durable_npc(npc_type: i32, tables: &CombatTables) -> bool {
    tables.npc(npc_type).is_some_and(|row| row.hitpoints >= 40)
}

/// A request earns melee offensive prayers/boosts when it engages a player, a
/// sufficiently durable NPC, or the Path explicitly packed a relevant boost.
pub fn offensives_worth(
    engaged: Option<ActorRef>,
    npc_type: i32,
    tables: &CombatTables,
    carries_boost: bool,
) -> bool {
    carries_boost
        || engaged.is_some_and(|actor| actor.kind == ActorKind::Player)
        || durable_npc(npc_type, tables)
}

/// Worth resolved from the request's own NPC target before any actor is in
/// range. Choosing a durable type is a decision to boost, so the opening Prep
/// batch can wear and drink together even while the target is out of reach.
pub fn requested_worth(request: &CombatRequest, tables: &CombatTables) -> bool {
    match &request.target {
        Target::Npc { types, .. } => types.iter().any(|ty| durable_npc(*ty, tables)),
        _ => false,
    }
}

/// G4b's FNV-1a identity hash, computed directly over a borrowed player name.
pub mod ident {
    #[inline]
    pub const fn fnv1a(name: &str) -> i32 {
        let bytes = name.as_bytes();
        let mut hash = 0x811c_9dc5_u32;
        let mut index = 0;
        while index < bytes.len() {
            hash ^= bytes[index] as u32;
            hash = hash.wrapping_mul(0x0100_0193);
            index += 1;
        }
        hash as i32
    }
}

/// Generated, fact-driven maximum hit for an NPC's main attack event.
pub mod facts {
    use super::{DragonfireKind, NpcAttackKind, NpcNameRow, StyleObs};

    /// The maximum of one attack event, including a forced multi-roll total.
    /// Missing inputs stay unknown instead of becoming an invented zero.
    pub fn npc_max_hit(row: &NpcNameRow) -> Option<u8> {
        if let Some(forced) = row.forced_max_hit {
            return to_hit(forced);
        }
        if row.bespoke {
            return None;
        }
        let main = match row.attack_kind {
            Some(NpcAttackKind::Ranged) => stat_max(row.ranged, row.rangebonus),
            Some(NpcAttackKind::Magic) => None,
            Some(NpcAttackKind::Mixed) | None => stat_max(row.strength, row.strengthbonus),
        }?;
        u8::try_from(main).ok()
    }

    /// Maximum hit for a live animation/spot-animation style. `forced_max_hit`
    /// is the total of that one fact-described special event, not an extra roll.
    pub fn npc_style_max_hit(row: &NpcNameRow, style: StyleObs) -> Option<i32> {
        if let Some(forced) = row.forced_max_hit {
            return (forced >= 0).then_some(forced);
        }
        if row.bespoke {
            return None;
        }
        match style {
            StyleObs::Melee => stat_max(row.strength, row.strengthbonus),
            StyleObs::Ranged => stat_max(row.ranged, row.rangebonus),
            StyleObs::Magic | StyleObs::Unknown | StyleObs::Dragonfire => None,
        }
    }

    /// Unreduced dragon breath maximum by content-derived dragon kind.
    pub fn dragonfire_max(kind: DragonfireKind) -> Option<i32> {
        match kind {
            DragonfireKind::Elvarg => Some(70),
            DragonfireKind::Chromatic | DragonfireKind::Metal => Some(50),
            DragonfireKind::Other => None,
        }
    }

    /// Dragon breath after the shield, antifire and Protect from Magic branches
    /// of the three observed scripts. The King Black Dragon stays Unknown.
    pub fn dragonfire_residual(
        kind: DragonfireKind,
        shield: bool,
        antifire: bool,
        protect_magic: bool,
    ) -> Option<i32> {
        let value = match kind {
            DragonfireKind::Elvarg => {
                (if shield { 10 } else { 70 })
                    - if shield {
                        3 * i32::from(antifire)
                    } else {
                        15 * i32::from(antifire)
                    }
                    - if shield {
                        3 * i32::from(protect_magic)
                    } else {
                        15 * i32::from(protect_magic)
                    }
            }
            DragonfireKind::Chromatic => {
                if shield {
                    if antifire {
                        0
                    } else {
                        5
                    }
                } else if antifire && protect_magic {
                    0
                } else if antifire {
                    35
                } else if protect_magic {
                    10
                } else {
                    50
                }
            }
            DragonfireKind::Metal => {
                if shield {
                    if antifire {
                        0
                    } else {
                        5
                    }
                } else if antifire {
                    35
                } else {
                    50
                }
            }
            DragonfireKind::Other => return None,
        };
        Some(value.max(0))
    }

    /// Player hit 9 under a matching protect implies at least max hit 15.
    #[inline]
    pub fn player_max_hit_floor(hit: i32, matching_protect: bool) -> Option<u8> {
        if hit < 0 {
            return None;
        }
        let maximum = if matching_protect {
            hit.saturating_mul(5).saturating_add(2) / 3
        } else {
            hit
        };
        u8::try_from(maximum).ok()
    }

    pub(super) fn stat_max(stat: Option<i32>, bonus: Option<i32>) -> Option<i32> {
        let stat = i64::from(stat?);
        let bonus = i64::from(bonus?);
        if stat < 0 {
            return None;
        }
        let hit = ((stat + 9) * (bonus + 64) + 320) / 640;
        i32::try_from(hit).ok().map(|value| value.max(0))
    }

    fn to_hit(value: i32) -> Option<u8> {
        if value < 0 {
            return None;
        }
        u8::try_from(value).ok()
    }
}

/// Return true only for the observed auto-retaliate-on encoding. Missing or
/// non-boolean varp values remain unknown for G4b's fail-closed decision.
pub fn retaliate(frame: &Frame<'_>) -> Option<bool> {
    frame
        .varps
        .iter()
        .find(|row| row.index == super::OPTION_NODEF)
        .and_then(|row| match row.value {
            0 => Some(true),
            1 => Some(false),
            _ => None,
        })
}

/// Existing HuntFighter rule re-expressed over the borrowed snapshot actor.
pub fn taken_by_another(npc: &NpcView, engaged: Option<ActorRef>, local_slot: usize) -> bool {
    if engaged
        .is_some_and(|actor| actor.kind == ActorKind::Npc && usize::from(actor.index) == npc.index)
    {
        return false;
    }
    let targets_me = npc
        .target
        .is_some_and(|target| target.kind == ActorKind::Player && target.index == local_slot);
    let targets_another = npc
        .target
        .is_some_and(|target| target.kind == ActorKind::Player && target.index != local_slot);
    targets_another || (npc.in_combat && !targets_me)
}

/// Literal action eligibility. A facing actor without an attack option is
/// still a threat, but it is never selected or emitted as a combat target. An
/// npc needs "Attack"; a player needs the label the content gave it ("Attack"
/// in the Wilderness, "Fight" in a duel). The zone gate is the candidate
/// predicate's ([`player_candidate`]), not this one's.
pub fn actor_attackable(frame: &Frame<'_>, actor: ActorRef) -> bool {
    match actor.kind {
        ActorKind::Npc => frame
            .npcs
            .iter()
            .find(|row| row.index == usize::from(actor.index))
            .is_some_and(|row| has_attack(&row.actions)),
        ActorKind::Player => frame
            .players
            .iter()
            .find(|row| row.index == usize::from(actor.index))
            .is_some_and(|row| attack_label(&row.actor.actions).is_some()),
    }
}

fn has_attack(actions: &[Option<String>]) -> bool {
    actions
        .iter()
        .any(|action| action.as_deref() == Some("Attack"))
}

/// True when the local player's network tile is within the stand's walk radius.
pub fn stand_reached(stand: api::WorldTile, local: &api::snapshot::LocalPlayerView) -> bool {
    tile_distance(local.player.network, stand) <= 1
}

/// True while an authored stand is still to be reached. A fight with a stand
/// picks no target until the network tile is within one tile of it.
pub fn stand_pending(request: &CombatRequest, frame: &Frame<'_>) -> bool {
    request
        .stand
        .is_some_and(|stand| !stand_reached(stand, frame.local))
}

/// Walking steps from the player's tile to a wall-valid stand beside
/// `npc`, from the posted live reach flood. `None` when no stand beside
/// the NPC is reachable in that flood.
fn npc_route_steps(view: &api::query::ReachQueryView, npc: &NpcView) -> Option<u32> {
    crate::reach_route::route_steps(
        view,
        crate::reach_route::Footprint {
            tile: npc.network,
            width: npc.size.max(1),
            length: npc.size.max(1),
        },
    )
    .map(|(steps, _)| steps)
}

/// Pick an attackable target without allocating or weakening actor identity.
/// The parent machine retains a selected actor/type while it remains valid.
/// Reach filtering follows the executable style, not the authored opening style.
pub fn pick_target(
    request: &CombatRequest,
    executable_style: Style,
    frame: &Frame<'_>,
    threats: &ThreatSet,
    _tables: &CombatTables,
    seed: u64,
) -> Option<ActorRef> {
    match &request.target {
        Target::Npc {
            types,
            pick,
            not_targeting_others,
        } => pick_npc(
            request,
            executable_style,
            frame,
            seed,
            NpcPick {
                types,
                pick,
                not_targeting_others: *not_targeting_others,
                attackers_only: false,
                except: None,
            },
        ),
        Target::Attacker { npcs, players } => threats
            .iter(frame.tick)
            .filter(|threat| match threat.actor.kind {
                ActorKind::Npc => *npcs,
                ActorKind::Player => *players,
            })
            .filter(|threat| {
                actor_in_search_bounds(frame, threat.actor, request.search_bounds.as_deref())
            })
            .filter(|threat| actor_attackable(frame, threat.actor))
            .max_by_key(|threat| {
                let magnitude = threat.max_hit_est().unwrap_or(u8::MAX);
                (
                    magnitude,
                    u16::MAX.wrapping_sub(frame.tick.wrapping_sub(threat.last_seen)),
                )
            })
            .map(|threat| threat.actor),
        Target::Player { name } => frame
            .players
            .iter()
            .find(|player| {
                player.actor.name.as_deref() == Some(name.as_ref())
                    && within_search_bounds(player.network, request.search_bounds.as_deref())
                    && player_actor(player).is_some_and(|actor| actor_attackable(frame, actor))
            })
            .and_then(player_actor),
    }
}

/// What [`pick_npc`] chooses among.
#[derive(Clone, Copy)]
struct NpcPick<'a> {
    types: &'a [i32],
    pick: &'a Pick,
    not_targeting_others: bool,
    /// Keep only the rows whose target is the local player: the single-way
    /// retarget ([`single_way_attacker`]) chooses among the NPCs that are
    /// hitting the bot.
    attackers_only: bool,
    /// With `attackers_only`: the actor the server just refused, which the
    /// retarget never picks again.
    except: Option<ActorRef>,
}

/// The authored-NPC pick of [`pick_target`].
fn pick_npc(
    request: &CombatRequest,
    executable_style: Style,
    frame: &Frame<'_>,
    seed: u64,
    filter: NpcPick<'_>,
) -> Option<ActorRef> {
    let NpcPick {
        types,
        pick,
        not_targeting_others,
        attackers_only,
        except,
    } = filter;
    // Walk to the stand first: a target near the stand is not
    // engageable from wherever the player happens to be, and a
    // melee click from outside its walls or fence never lands. The stand
    // gates starting an engagement: an NPC that already hits the bot needs
    // neither the stand nor a route, because it reached the bot.
    if !attackers_only && stand_pending(request, frame) {
        return None;
    }
    let origin = request.stand.unwrap_or(frame.here);
    // Melee uses the posted reach flood when it starts at the
    // network tile. A stale posted flood gives no pick this poll.
    // Only a missing flood keeps straight-line picks.
    let reach = if executable_style == Style::Melee && !attackers_only {
        match frame.reach {
            Some(view) => Some(crate::reach_route::live(
                Some(view),
                Some(frame.local.player.network),
            )?),
            None => None,
        }
    } else {
        None
    };
    // In single-way combat an attack on any NPC but the one that last hit the
    // bot is refused for 8 ticks (`player_combat.rs2:115`), so the nearest
    // pick takes an NPC that is already in combat with the bot first. Another
    // pick order is the request's own, and a multiway map has no such rule.
    let attacker_first =
        !attackers_only && matches!(pick, Pick::Nearest) && !frame.world.multi_combat;
    let mut best: Option<(ActorRef, u64, u64)> = None;
    let mut random_state = seed | 1;
    let mut seen = 0u64;
    let candidates = frame.npcs.iter().filter_map(|npc| {
        if corpse(npc)
            || !npc.r#type.is_some_and(|id| {
                types
                    .iter()
                    .any(|wanted| usize::try_from(*wanted).ok() == Some(id))
            })
            || !within_search_bounds(npc.network, request.search_bounds.as_deref())
            || tile_distance(npc.network, origin) > u32::from(request.engage_radius)
        {
            return None;
        }
        if attackers_only
            && (!targets_player(npc, frame.me())
                || except.is_some_and(|skip| npc_actor(npc) == Some(skip)))
        {
            return None;
        }
        let route_steps = reach.and_then(|view| npc_route_steps(view, npc));
        if reach.is_some() && route_steps.is_none() {
            return None;
        }
        if not_targeting_others && taken_by_another(npc, None, frame.me()) {
            return None;
        }
        let actor = npc_actor(npc)?;
        actor_attackable(frame, actor).then_some((npc, actor, route_steps))
    });
    for (npc, actor, route_steps) in candidates {
        seen = seen.saturating_add(1);
        let distance = if attackers_only {
            // The NPC that is in combat with the bot comes first, then the
            // nearest to the bot.
            (u64::from(!npc.in_combat) << 32) | u64::from(tile_distance(npc.network, frame.here))
        } else {
            let near = route_steps
                .map_or_else(|| u64::from(tile_distance(npc.network, origin)), u64::from);
            if attacker_first {
                (u64::from(!(npc.in_combat && targets_player(npc, frame.me()))) << 32) | near
            } else {
                near
            }
        };
        let health = health_rank(npc);
        let replace = match (pick, best) {
            (Pick::Nearest, Some((_, best_distance, _))) => distance < best_distance,
            (Pick::LowestHealth, Some((_, _, best_health))) => health < best_health,
            (Pick::Random, _) => {
                random_state ^= random_state << 13;
                random_state ^= random_state >> 7;
                random_state ^= random_state << 17;
                random_state.is_multiple_of(seen)
            }
            (_, None) => true,
        };
        if replace || best.is_none() {
            best = Some((actor, distance, health));
        }
    }
    best.map(|(actor, _, _)| actor)
}

/// True when the NPC row's target is the local player.
fn targets_player(npc: &NpcView, local_slot: usize) -> bool {
    npc.target
        .is_some_and(|target| target.kind == ActorKind::Player && target.index == local_slot)
}

/// The NPC that holds the bot in single-way combat: an attackable NPC the
/// request may fight whose target is the local player, other than `refused`
/// (the engaged actor whose attack the server just refused). In single-way
/// combat the attack is refused unless its target is the NPC that last
/// attacked the player (`player_combat.rs2:115`, `npc_combat.rs2:367-370`),
/// so after a refusal this is the target the content allows. The pick keeps
/// the authored NPC filters (types, search bounds, engage radius) and takes
/// the NPC in combat with the bot first, then the nearest. `None` when no
/// such NPC is observed.
pub fn single_way_attacker(
    request: &CombatRequest,
    executable_style: Style,
    frame: &Frame<'_>,
    threats: &ThreatSet,
    refused: Option<ActorRef>,
) -> Option<ActorRef> {
    match &request.target {
        Target::Npc { types, .. } => pick_npc(
            request,
            executable_style,
            frame,
            0,
            NpcPick {
                types,
                pick: &Pick::Nearest,
                not_targeting_others: false,
                attackers_only: true,
                except: refused,
            },
        ),
        Target::Attacker { npcs: true, .. } => threats
            .iter(frame.tick)
            .filter(|threat| threat.actor.kind == ActorKind::Npc)
            .filter(|threat| Some(threat.actor) != refused)
            .filter(|threat| {
                frame
                    .npcs
                    .iter()
                    .find(|row| row.index == usize::from(threat.actor.index))
                    .is_some_and(|row| targets_player(row, frame.me()))
            })
            .filter(|threat| {
                actor_in_search_bounds(frame, threat.actor, request.search_bounds.as_deref())
            })
            .filter(|threat| actor_attackable(frame, threat.actor))
            .max_by_key(|threat| {
                (
                    threat.max_hit_est().unwrap_or(u8::MAX),
                    u16::MAX.wrapping_sub(frame.tick.wrapping_sub(threat.last_seen)),
                )
            })
            .map(|threat| threat.actor),
        Target::Attacker { npcs: false, .. } | Target::Player { .. } => None,
    }
}
#[inline]
fn actor_in_search_bounds(
    frame: &Frame<'_>,
    actor: ActorRef,
    bounds: Option<&[SceneRegionInput]>,
) -> bool {
    match actor.kind {
        ActorKind::Npc => frame
            .npcs
            .iter()
            .find(|row| row.index == usize::from(actor.index))
            .is_some_and(|row| within_search_bounds(row.network, bounds)),
        ActorKind::Player => frame
            .players
            .iter()
            .find(|row| row.index == usize::from(actor.index))
            .is_some_and(|row| within_search_bounds(row.network, bounds)),
    }
}
#[inline]
fn npc_actor(npc: &NpcView) -> Option<ActorRef> {
    Some(ActorRef {
        kind: ActorKind::Npc,
        index: u16::try_from(npc.index).ok()?,
    })
}

#[inline]
fn player_actor(player: &api::snapshot::PlayerView) -> Option<ActorRef> {
    Some(ActorRef {
        kind: ActorKind::Player,
        index: u16::try_from(player.index).ok()?,
    })
}

/// `Target::Attacker` remains a threat-based request; this helper distinguishes
/// a live aggressor that cannot be attacked from an observed empty target set.
pub fn has_unattackable_allowed(
    request: &CombatRequest,
    frame: &Frame<'_>,
    threats: &ThreatSet,
) -> bool {
    let Target::Attacker { npcs, players } = &request.target else {
        return false;
    };
    threats.iter(frame.tick).any(|threat| {
        let allowed = match threat.actor.kind {
            ActorKind::Npc => *npcs,
            ActorKind::Player => *players,
        };
        allowed
            && actor_in_search_bounds(frame, threat.actor, request.search_bounds.as_deref())
            && !actor_attackable(frame, threat.actor)
    })
}

/// Candidate protect prayer for an attack style. Prayer tiers are generated
/// in level order: Magic, Missiles, Melee.
pub(crate) fn protect_fact(tables: &CombatTables, style: StyleObs) -> Option<&PrayerFact> {
    let tier = match style {
        StyleObs::Magic | StyleObs::Dragonfire => 0,
        StyleObs::Ranged => 1,
        StyleObs::Melee | StyleObs::Unknown => 2,
    };
    tables.prayer(PrayerRole::Protect, tier)
}
#[inline]
pub(crate) fn style_from_mask(mask: StyleMask) -> StyleObs {
    match mask.bits() {
        1 => StyleObs::Melee,
        2 => StyleObs::Ranged,
        4 => StyleObs::Magic,
        8 => StyleObs::Dragonfire,
        _ => StyleObs::Unknown,
    }
}

pub(crate) fn active_protect(frame: &Frame<'_>, tables: &CombatTables) -> Option<StyleObs> {
    [StyleObs::Melee, StyleObs::Ranged, StyleObs::Magic]
        .into_iter()
        .find(|style| protect_fact(tables, *style).is_some_and(|row| prayer_on(frame, row.varp)))
}

#[inline]
pub(crate) fn prayer_on(frame: &Frame<'_>, varp: i32) -> bool {
    varp.checked_sub(83)
        .and_then(|index| usize::try_from(index).ok())
        .and_then(|index| frame.prayers.get(index))
        .copied()
        .unwrap_or(false)
}

/// Maximum incoming damage of one threat under its current/selected style.
pub(crate) fn residual(
    threat: &Threat,
    style: StyleObs,
    protect: Option<StyleObs>,
    tables: &CombatTables,
    shield: bool,
    antifire: bool,
) -> Option<i32> {
    match threat.actor.kind {
        ActorKind::Player => {
            let maximum = i32::from(threat.max_hit_est()?);
            if protect == Some(style) {
                Some(maximum.saturating_mul(6) / 10)
            } else {
                Some(maximum)
            }
        }
        ActorKind::Npc => {
            let row = tables.npc(threat.ident)?;
            if style == StyleObs::Dragonfire {
                return row.dragonfire.and_then(|kind| {
                    facts::dragonfire_residual(
                        kind,
                        shield,
                        antifire,
                        protect == Some(StyleObs::Magic),
                    )
                });
            }
            let maximum = facts::npc_style_max_hit(row, style)?;
            if protect == Some(style) {
                Some(0)
            } else {
                Some(maximum)
            }
        }
    }
}

fn corpse(npc: &NpcView) -> bool {
    npc.total_health > 0 && npc.health == 0
}

fn tile_distance(a: api::WorldTile, b: api::WorldTile) -> u32 {
    if a.level != b.level {
        return u32::MAX;
    }
    a.x.abs_diff(b.x).max(a.z.abs_diff(b.z))
}
#[inline]
fn within_search_bounds(tile: api::WorldTile, bounds: Option<&[SceneRegionInput]>) -> bool {
    bounds.is_none_or(|regions| {
        regions.iter().any(|region| {
            region.level == tile.level
                && (region.min_x..=region.max_x).contains(&tile.x)
                && (region.min_z..=region.max_z).contains(&tile.z)
        })
    })
}

fn health_rank(npc: &NpcView) -> u64 {
    if npc.total_health > 0 && npc.health >= 0 {
        u64::try_from(npc.health).unwrap_or(u64::MAX) * (u64::from(u32::MAX) + 1)
            / u64::try_from(npc.total_health).unwrap_or(1).max(1)
    } else {
        u64::MAX
    }
}

#[cfg(test)]
mod tests {
    use super::facts::{dragonfire_residual, npc_max_hit, player_max_hit_floor};
    use super::*;

    fn npc_fact(
        strength: Option<i32>,
        strengthbonus: Option<i32>,
        attack_kind: Option<NpcAttackKind>,
        forced_max_hit: Option<i32>,
        dragonfire: Option<DragonfireKind>,
    ) -> NpcNameRow {
        NpcNameRow {
            id: 1,
            config: String::new(),
            display: None,
            ops: Vec::new(),
            size: 1,
            wanderrange: 0,
            maxrange: 0,
            attackrange: 0,
            huntrange: 0,
            vislevel: 0,
            headicon: None,
            hitpoints: 0,
            max_dealt: Some(1000),
            damagetype: None,
            dragonfire,
            attack: 1,
            defence: 1,
            magic: 1,
            attackbonus: 0,
            rangeattack: 0,
            magicattack: 0,
            stabdefence: 0,
            slashdefence: 0,
            crushdefence: 0,
            magicdefence: 0,
            rangedefence: 0,
            strength,
            ranged: Some(1),
            strengthbonus,
            rangebonus: Some(0),
            undead: Some(0),
            ap_attack: false,
            attack_kind,
            forced_max_hit,
            attackrate: Some(4),
            bespoke: false,
            counter_protect: false,
            poison_severity: None,
            forcemulti: false,
            combat_transform: None,
        }
    }

    #[test]
    fn hit_estimates_use_fact_defaults_and_forced_event_totals() {
        let imp = npc_fact(Some(1), Some(-37), None, None, None);
        let warlord = npc_fact(Some(78), Some(0), None, None, None);
        let elvarg = npc_fact(Some(78), Some(0), None, None, Some(DragonfireKind::Elvarg));
        let dagannoth = npc_fact(None, None, Some(NpcAttackKind::Ranged), Some(24), None);
        let mut bespoke = npc_fact(Some(78), Some(0), None, None, None);
        bespoke.bespoke = true;
        assert_eq!(npc_max_hit(&imp), Some(0));
        assert_eq!(npc_max_hit(&warlord), Some(9));
        assert_eq!(npc_max_hit(&elvarg), Some(9));
        assert_eq!(npc_max_hit(&dagannoth), Some(24));
        assert_eq!(
            facts::npc_style_max_hit(&dagannoth, StyleObs::Ranged),
            Some(24)
        );
        assert_eq!(npc_max_hit(&bespoke), None);
        assert_eq!(facts::npc_style_max_hit(&bespoke, StyleObs::Melee), None);
        assert_eq!(
            facts::npc_max_hit(&npc_fact(
                None,
                None,
                Some(NpcAttackKind::Magic),
                None,
                None
            )),
            None
        );
    }

    #[test]
    fn player_hit_floors_and_fnv_identity_are_stable() {
        assert_eq!(player_max_hit_floor(9, true), Some(15));
        assert_eq!(player_max_hit_floor(9, false), Some(9));
        assert_eq!(player_max_hit_floor(-1, false), None);
        assert_eq!(ident::fnv1a("hello"), 1_335_831_723);
    }

    #[test]
    fn dragon_kinds_follow_their_distinct_protection_tables() {
        assert_eq!(
            dragonfire_residual(DragonfireKind::Elvarg, false, false, false),
            Some(70)
        );
        assert_eq!(
            dragonfire_residual(DragonfireKind::Elvarg, true, false, false),
            Some(10)
        );
        assert_eq!(
            dragonfire_residual(DragonfireKind::Elvarg, true, true, true),
            Some(4)
        );
        assert_eq!(
            dragonfire_residual(DragonfireKind::Elvarg, true, false, true),
            Some(7)
        );
        assert_eq!(
            dragonfire_residual(DragonfireKind::Chromatic, false, true, false),
            Some(35)
        );
        assert_eq!(
            dragonfire_residual(DragonfireKind::Chromatic, false, false, true),
            Some(10)
        );
        assert_eq!(
            dragonfire_residual(DragonfireKind::Chromatic, false, true, true),
            Some(0)
        );
        assert_eq!(
            dragonfire_residual(DragonfireKind::Chromatic, true, false, true),
            Some(5)
        );
        assert_eq!(
            dragonfire_residual(DragonfireKind::Chromatic, true, true, false),
            Some(0)
        );
        assert_eq!(
            dragonfire_residual(DragonfireKind::Metal, false, true, true),
            Some(35)
        );
        assert_eq!(
            dragonfire_residual(DragonfireKind::Metal, false, false, true),
            Some(50)
        );
        assert_eq!(
            dragonfire_residual(DragonfireKind::Other, false, false, true),
            None
        );
    }

    #[test]
    fn derived_lines_match_known_and_unknown_danger() {
        assert_eq!(
            lines(Some(9), 40),
            Lines {
                emergency: 10,
                eat: 19,
                drink_gate: 28
            }
        );
        assert_eq!(
            lines(None, 40),
            Lines {
                emergency: 20,
                eat: 20,
                drink_gate: 20
            }
        );
        assert_eq!(lines(Some(0), 40).eat, 1);
    }
    #[test]
    fn acquisition_bounds_are_inclusive_level_aware_unions() {
        let regions = [
            SceneRegionInput {
                min_x: 10,
                min_z: 20,
                max_x: 12,
                max_z: 22,
                level: 0,
            },
            SceneRegionInput {
                min_x: 20,
                min_z: 30,
                max_x: 21,
                max_z: 31,
                level: 1,
            },
        ];
        assert!(within_search_bounds(
            api::WorldTile {
                x: 10,
                z: 20,
                level: 0
            },
            Some(&regions)
        ));
        assert!(within_search_bounds(
            api::WorldTile {
                x: 12,
                z: 22,
                level: 0
            },
            Some(&regions)
        ));
        assert!(within_search_bounds(
            api::WorldTile {
                x: 20,
                z: 30,
                level: 1
            },
            Some(&regions)
        ));
        assert!(!within_search_bounds(
            api::WorldTile {
                x: 13,
                z: 22,
                level: 0
            },
            Some(&regions)
        ));
        assert!(!within_search_bounds(
            api::WorldTile {
                x: 15,
                z: 25,
                level: 0
            },
            Some(&regions)
        ));
        assert!(!within_search_bounds(
            api::WorldTile {
                x: 10,
                z: 20,
                level: 1
            },
            Some(&regions[..1])
        ));
        assert!(!within_search_bounds(
            api::WorldTile {
                x: 10,
                z: 20,
                level: 0
            },
            Some(&[])
        ));
        assert!(within_search_bounds(
            api::WorldTile {
                x: 500,
                z: -500,
                level: 3
            },
            None
        ));
    }
}
