//! Pure combat rules shared by native Combat, WalkGuard, and Hunt.
use super::frame::Frame;
use super::select;
use super::tables::{CombatTables, FoodFact, StyleWhere};
use super::threats::{StyleObs, Threat, ThreatSet};
use api::game_data::PrayerFact;
use api::snapshot::{ActorKind, ActorTargetView};

#[inline]
pub fn prayer_restore(base: i32) -> i32 {
    7 + base / 4
}

#[inline]
pub fn prayer_sip_floor(base: i32) -> i32 {
    (base - prayer_restore(base)).max(3)
}

#[inline]
pub fn prayer_sip_due(points: i32, base: i32) -> bool {
    base > 0 && points <= prayer_sip_floor(base)
}

/// Select protection from a unanimous classified incoming volley first
/// (agreement is by protection varp, so Magic and Dragonfire agree), then
/// from the compact saved-damage threat score.
pub fn wanted_protect<'a>(
    threats: &ThreatSet,
    frame: &Frame<'_>,
    tables: &'a CombatTables,
    tick: u16,
    shield: bool,
    antifire: bool,
) -> Option<&'a PrayerFact> {
    wanted_protect_with(
        threats,
        frame.me(),
        frame
            .projectiles
            .iter()
            .map(|projectile| (projectile.spotanim, projectile.target)),
        tables,
        tick,
        shield,
        antifire,
    )
}

/// Hunt's isolate keeps just the fields the protect picker uses. This adapter
/// feeds the same native rule without constructing or copying a GameSnapshot.
#[cfg(feature = "load")]
pub(crate) fn wanted_protect_hunt<'a>(
    threats: &ThreatSet,
    local_player_index: usize,
    projectiles: impl Iterator<Item = (i32, Option<usize>)>,
    tables: &'a CombatTables,
    tick: u16,
    shield: bool,
    antifire: bool,
) -> Option<&'a PrayerFact> {
    wanted_protect_with(
        threats,
        local_player_index,
        projectiles.map(|(spotanim, target)| {
            (
                spotanim,
                target.map(|index| ActorTargetView {
                    kind: ActorKind::Player,
                    index,
                }),
            )
        }),
        tables,
        tick,
        shield,
        antifire,
    )
}

fn wanted_protect_with<'a>(
    threats: &ThreatSet,
    local_player_index: usize,
    projectiles: impl Iterator<Item = (i32, Option<ActorTargetView>)>,
    tables: &'a CombatTables,
    tick: u16,
    shield: bool,
    antifire: bool,
) -> Option<&'a PrayerFact> {
    let mut incoming: Option<&PrayerFact> = None;
    let mut disagreement = false;
    for (spotanim, target) in projectiles {
        if !target.is_some_and(|target| {
            target.kind == ActorKind::Player && target.index == local_player_index
        }) {
            continue;
        }
        let Some(style) = tables
            .style_spotanim(spotanim)
            .filter(|row| row.where_ == StyleWhere::Projectile)
            .map(|row| select::style_from_mask(row.style))
            .filter(|style| *style != StyleObs::Unknown)
        else {
            continue;
        };
        let Some(fact) = select::protect_fact(tables, style) else {
            continue;
        };
        match incoming {
            Some(previous) if previous.varp != fact.varp => {
                disagreement = true;
                break;
            }
            Some(_) => {}
            None => incoming = Some(fact),
        }
    }
    if !disagreement && incoming.is_some() {
        return incoming;
    }
    let protect_styles = [StyleObs::Melee, StyleObs::Ranged, StyleObs::Magic];
    let mut scores = [0i64; 3];
    let mut unknown = [0u8; 3];
    let mut newest = [u16::MAX; 3];
    let mut any_threat = false;
    let mut largest_known = 0i64;

    for threat in threats.iter(tick) {
        any_threat = true;
        let npc_row = (threat.actor.kind == super::request::ActorKind::Npc)
            .then(|| tables.npc(threat.ident))
            .flatten();
        if npc_row.is_some_and(|row| row.counter_protect) {
            let mut best_style = StyleObs::Magic;
            let mut best_mag = i32::MIN;
            for style in [StyleObs::Magic, StyleObs::Ranged, StyleObs::Melee] {
                let Some(magnitude) =
                    select::residual(threat, style, None, tables, shield, antifire)
                else {
                    continue;
                };
                if magnitude > best_mag {
                    best_mag = magnitude;
                    best_style = style;
                }
            }
            if best_mag > i32::MIN {
                score_style(
                    threat,
                    best_style,
                    1,
                    tick,
                    tables,
                    shield,
                    antifire,
                    &protect_styles,
                    &mut scores,
                    &mut unknown,
                    &mut newest,
                    &mut largest_known,
                );
            }
            continue;
        }
        let mixed = npc_row.is_some_and(|row| {
            row.dragonfire.is_some()
                || row.attack_kind == Some(api::game_data::NpcAttackKind::Mixed)
        });
        for style in [
            StyleObs::Melee,
            StyleObs::Ranged,
            StyleObs::Magic,
            StyleObs::Dragonfire,
        ] {
            let count = if mixed {
                let events = threat.recent_count(style);
                if events > 0 {
                    events
                } else if threat.history_len() == 0 && style == threat.style {
                    1
                } else {
                    0
                }
            } else if style == effective_style(threat) {
                1
            } else {
                0
            };
            if count == 0 {
                continue;
            }
            score_style(
                threat,
                style,
                count,
                tick,
                tables,
                shield,
                antifire,
                &protect_styles,
                &mut scores,
                &mut unknown,
                &mut newest,
                &mut largest_known,
            );
        }
    }
    if !any_threat {
        return None;
    }

    let proxy = largest_known.max(1);
    let best = (0..3).max_by_key(|index| {
        let score = scores[*index].saturating_add(i64::from(unknown[*index]).saturating_mul(proxy));
        (score, u16::MAX.wrapping_sub(newest[*index]))
    })?;
    let style = protect_styles[best];
    (scores[best] > 0 || unknown[best] > 0)
        .then(|| select::protect_fact(tables, style))
        .flatten()
}

#[allow(clippy::too_many_arguments)]
fn score_style(
    threat: &Threat,
    style: StyleObs,
    count: u8,
    tick: u16,
    tables: &CombatTables,
    shield: bool,
    antifire: bool,
    protect_styles: &[StyleObs; 3],
    scores: &mut [i64; 3],
    unknown: &mut [u8; 3],
    newest: &mut [u16; 3],
    largest_known: &mut i64,
) {
    let age = tick.wrapping_sub(threat.last_seen);
    let event_age = age.saturating_mul(8).saturating_add(u16::from(
        threat
            .recent_position(style)
            .unwrap_or(threat.history_len()),
    ));
    for (index, protect_style) in protect_styles.iter().copied().enumerate() {
        if !can_protect_style(style, protect_style) {
            continue;
        }
        let no_protect = select::residual(threat, style, None, tables, shield, antifire);
        let with_protect =
            select::residual(threat, style, Some(protect_style), tables, shield, antifire);
        match (no_protect, with_protect) {
            (Some(no), Some(with)) => {
                let saving = i64::from(no.saturating_sub(with).max(0)) * i64::from(count);
                scores[index] = scores[index].saturating_add(saving);
                *largest_known = (*largest_known).max(saving);
                if saving > 0 && event_age < newest[index] {
                    newest[index] = event_age;
                }
            }
            _ => {
                unknown[index] = unknown[index].saturating_add(count);
                if event_age < newest[index] {
                    newest[index] = event_age;
                }
            }
        }
    }
}

#[inline]
fn effective_style(threat: &Threat) -> StyleObs {
    if threat.style == StyleObs::Unknown {
        threat.fallback_style()
    } else {
        threat.style
    }
}

#[inline]
fn can_protect_style(attack: StyleObs, protect: StyleObs) -> bool {
    attack == protect || (attack == StyleObs::Dragonfire && protect == StyleObs::Magic)
}

/// Largest held heal fitting the HP deficit, otherwise the smallest eligible
/// heal. Recovery applies the cap, not the nominal heal.
pub(crate) fn food_by(
    hp: i32,
    hp_max: i32,
    inventory: impl Iterator<Item = (i32, i32)>,
    tables: &CombatTables,
    gate: Option<i32>,
    eligible: impl Fn(&FoodFact) -> bool,
) -> Option<i32> {
    let mut fitting: Option<(i32, i32)> = None;
    let mut smallest: Option<(i32, i32)> = None;
    for (id, count) in inventory {
        let Some(food) = tables.food(id) else {
            continue;
        };
        let heal = food.heal;
        if count <= 0
            || !eligible(food)
            || gate.is_some_and(|gate| hp.saturating_add(heal).min(hp_max) <= gate)
        {
            continue;
        }
        if smallest.is_none_or(|(_, current)| heal < current) {
            smallest = Some((id, heal));
        }
        if hp.saturating_add(heal) <= hp_max && fitting.is_none_or(|(_, current)| heal > current) {
            fitting = Some((id, heal));
        }
    }
    fitting.or(smallest).map(|(id, _)| id)
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prayer_sip_uses_the_native_c5_floor() {
        assert_eq!(prayer_restore(43), 17);
        assert_eq!(prayer_sip_floor(43), 26);
        assert!(prayer_sip_due(26, 43));
        assert!(!prayer_sip_due(27, 43));
        assert!(prayer_sip_due(17, 31));
        assert!(prayer_sip_due(9, 43));
        assert!(prayer_sip_due(8, 43));
    }
}
