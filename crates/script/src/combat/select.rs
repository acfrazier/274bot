//! Borrowed combat selection, danger lines and fact-derived combat formulas.
use super::frame::Frame;
use super::request::{ActorKind, ActorRef, CombatRequest, Pick, Target};
use super::tables::{CombatTables, PrayerRole, StyleMask};
use super::threats::{StyleObs, Threat, ThreatSet};
use api::game_data::{DragonfireKind, NpcAttackKind, NpcNameRow, PrayerFact};
use api::gather_methods::SceneRegionInput;
use api::snapshot::NpcView;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Lines {
    pub emergency: i32,
    pub eat: i32,
    /// The strict `hp > drink_gate` survival threshold for a three-tick potion lock.
    pub drink_gate: i32,
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
        || tables.npc(npc_type).is_some_and(|row| row.hitpoints >= 40)
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

/// Literal action eligibility. A facing actor without an `Attack` option is
/// still a threat, but it is never selected or emitted as a combat target.
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
            .is_some_and(|row| has_attack(&row.actor.actions)),
    }
}

fn has_attack(actions: &[Option<String>]) -> bool {
    actions
        .iter()
        .any(|action| action.as_deref() == Some("Attack"))
}

/// Pick an attackable target without allocating or weakening actor identity.
/// The parent machine retains a selected actor/type while it remains valid.
pub fn pick_target(
    request: &CombatRequest,
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
        } => {
            let origin = request.stand.unwrap_or(frame.here);
            let mut best: Option<(&NpcView, u64, u64)> = None;
            let mut random_state = seed | 1;
            let mut seen = 0u64;
            for npc in frame.npcs.iter().filter(|npc| {
                !corpse(npc)
                    && npc.r#type.is_some_and(|id| {
                        types
                            .iter()
                            .any(|wanted| usize::try_from(*wanted).ok() == Some(id))
                    })
                    && within_search_bounds(npc.tile, request.search_bounds.as_deref())
                    && tile_distance(npc.tile, origin) <= u32::from(request.engage_radius)
                    && (!not_targeting_others || !taken_by_another(npc, None, frame.me()))
                    && npc_actor(npc).is_some_and(|actor| actor_attackable(frame, actor))
            }) {
                seen = seen.saturating_add(1);
                let distance = u64::from(tile_distance(npc.tile, origin));
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
                    best = Some((npc, distance, health));
                }
            }
            best.and_then(|(npc, _, _)| npc_actor(npc))
        }
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
                    && within_search_bounds(player.actor.tile, request.search_bounds.as_deref())
                    && player_actor(player).is_some_and(|actor| actor_attackable(frame, actor))
            })
            .and_then(player_actor),
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
            .is_some_and(|row| within_search_bounds(row.tile, bounds)),
        ActorKind::Player => frame
            .players
            .iter()
            .find(|row| row.index == usize::from(actor.index))
            .is_some_and(|row| within_search_bounds(row.actor.tile, bounds)),
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
            damagetype: None,
            dragonfire,
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
