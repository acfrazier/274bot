//! Pure combat rules shared by native Combat, WalkGuard, and Hunt.
use super::frame::Frame;
use super::schedule::Schedule;
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
pub fn wanted_protect_hunt<'a>(
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
pub(crate) fn food_by_excluding(
    hp: i32,
    hp_max: i32,
    inventory: impl Iterator<Item = (i32, i32)> + Clone,
    tables: &CombatTables,
    gate: Option<i32>,
    protected_items: &[FoodReserve],
    eligible: impl Fn(&FoodFact) -> bool,
) -> Option<i32> {
    food_choice(
        hp,
        hp_max,
        inventory,
        tables,
        gate,
        protected_items,
        false,
        eligible,
    )
}

/// [`food_by_excluding`] with the waste rule: when `fit_only` is set, only a
/// food whose heal fits the missing HP (`hp + heal <= hp_max`) is a choice,
/// so a bite never overheals.
#[expect(
    clippy::too_many_arguments,
    reason = "the HP pair, the held rows, the tables, the gate, the reserves, the waste rule and the eligibility test stay explicit"
)]
fn food_choice(
    hp: i32,
    hp_max: i32,
    inventory: impl Iterator<Item = (i32, i32)> + Clone,
    tables: &CombatTables,
    gate: Option<i32>,
    protected_items: &[FoodReserve],
    fit_only: bool,
    eligible: impl Fn(&FoodFact) -> bool,
) -> Option<i32> {
    if hp <= 0 || hp >= hp_max {
        return None;
    }
    let mut fitting: Option<(i32, i32)> = None;
    let mut smallest: Option<(i32, i32)> = None;
    for (id, count) in inventory.clone() {
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
        let fits = hp.saturating_add(heal) <= hp_max;
        if fit_only && !fits {
            continue;
        }
        let replace_smallest = smallest.is_none_or(|(_, current)| heal < current);
        let replace_fitting = fits && fitting.is_none_or(|(_, current)| heal > current);
        if !replace_smallest && !replace_fitting {
            continue;
        }
        let reserved = FoodReserve::reserved(protected_items, id);
        if reserved > 0
            && inventory
                .clone()
                .filter(|(held_id, _)| *held_id == id)
                .fold(0i32, |sum, (_, count)| sum.saturating_add(count.max(0)))
                <= reserved
        {
            continue;
        }
        if replace_smallest {
            smallest = Some((id, heal));
        }
        if replace_fitting {
            fitting = Some((id, heal));
        }
    }
    fitting.or(smallest).map(|(id, _)| id)
}

/// The food a bite in `tier` takes: none outside a tier, the best fit in the
/// critical tier even when it overheals, and in the caution tier only a food
/// whose heal fits the missing HP. Every ordinary-food eater picks through
/// this, so no eater chooses a food by another rule.
#[expect(
    clippy::too_many_arguments,
    reason = "the tier, HP pair, held rows, tables, gate, reserves and eligibility test stay explicit"
)]
pub(crate) fn food_for_tier(
    tier: BiteTier,
    hp: i32,
    hp_max: i32,
    inventory: impl Iterator<Item = (i32, i32)> + Clone,
    tables: &CombatTables,
    gate: Option<i32>,
    protected_items: &[FoodReserve],
    eligible: impl Fn(&FoodFact) -> bool,
) -> Option<i32> {
    let fit_only = match tier {
        BiteTier::None => return None,
        BiteTier::Critical => false,
        BiteTier::Caution => true,
    };
    food_choice(
        hp,
        hp_max,
        inventory,
        tables,
        gate,
        protected_items,
        fit_only,
        eligible,
    )
}

/// An exact-ID minimum held count that survival food must leave untouched.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FoodReserve {
    pub id: i32,
    pub qty: u32,
}

impl FoodReserve {
    /// Repeated declarations impose the largest minimum, not a sum.
    pub fn reserved(reserves: &[Self], id: i32) -> i32 {
        let qty = reserves
            .iter()
            .filter(|reserve| reserve.id == id)
            .map(|reserve| reserve.qty)
            .max()
            .unwrap_or(0);
        i32::try_from(qty).unwrap_or(i32::MAX)
    }

    /// Food credit and live consumption use the same unreserved held count.
    pub fn spendable(reserves: &[Self], id: i32, held: i32) -> i32 {
        held.saturating_sub(Self::reserved(reserves, id)).max(0)
    }
}

/// Food chosen on one HP observation. Ordinary food follows the next-eat
/// clock; combo food is eligible only when it crosses its message-delay gate.
/// `basis` is what the bite was chosen on (the critical floor and the danger);
/// the survival log line records it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct EatChoice {
    pub ordinary: Option<i32>,
    pub combo: Option<i32>,
    pub basis: crate::survival::eat::Basis,
}
/// The scalar state used to choose food for one observation.
pub(crate) struct EatInputs {
    pub danger: Option<i32>,
    /// The most damage that can land before this bite takes effect
    /// ([`hits_before_bite`]); `None` when the danger is unknown.
    pub burst: Option<i32>,
    pub poison_fire: i32,
    pub hp: i32,
    pub hp_max: i32,
    pub tick: u16,
    pub eat_floor: i32,
}
/// Ticks between sending a bite and its heal: the route model's own `LAG`.
pub(crate) const BITE_DELAY: i32 = super::risk::consts::LAG;

/// How many hits one attacker with `rate` ticks between attacks can land
/// before a bite sent now takes effect. Attacks are `rate` ticks apart, so a
/// window of [`BITE_DELAY`] ticks holds at most `ceil(BITE_DELAY / rate)` of
/// them (an unknown rate of zero counts as an attack every tick).
#[inline]
pub(crate) fn hits_before_bite(rate: u8) -> i32 {
    (BITE_DELAY + i32::from(rate.max(1)) - 1) / i32::from(rate.max(1))
}

/// The critical floor: the HP at or below which the damage that can land
/// before a bite takes effect (`burst`) can kill, so the best-fitting food is
/// eaten even when it overheals. `burst` is every attacker's maximum hit once
/// per attack it can make inside [`BITE_DELAY`]; the real eater takes it from
/// the live threats and the route model from the plan's rows, both through
/// [`hits_before_bite`]. It is never below `danger`. With an unknown danger
/// the floor is a quarter of the maximum, rounded up. The next poison fire
/// is added to both.
pub(crate) fn critical_floor(
    danger: Option<i32>,
    burst: Option<i32>,
    hp_max: i32,
    poison_fire: i32,
) -> i32 {
    match danger {
        Some(danger) => burst
            .unwrap_or(danger)
            .max(danger)
            .saturating_add(1)
            .saturating_add(poison_fire),
        None => ((hp_max.max(0) + 3) / 4).saturating_add(poison_fire),
    }
}
/// The test entry to [`eat_choice_excluding`]: every attacker lands one hit
/// inside the bite delay (`burst` is the danger), no protected items.
#[cfg(test)]
#[expect(
    clippy::too_many_arguments,
    reason = "the frame, tables and schedule borrows plus the danger, poison fire, HP pair, tick and eat-floor scalars stay explicit"
)]
pub(crate) fn eat_choice(
    frame: &Frame<'_>,
    tables: &CombatTables,
    schedule: &Schedule,
    danger: Option<i32>,
    poison_fire: i32,
    hp: i32,
    hp_max: i32,
    tick: u16,
    eat_floor: i32,
) -> Option<EatChoice> {
    eat_choice_excluding(
        frame,
        tables,
        schedule,
        EatInputs {
            danger,
            burst: danger,
            poison_fire,
            hp,
            hp_max,
            tick,
            eat_floor,
        },
        &[],
    )
}
/// Which tier a bite is in at one HP observation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BiteTier {
    /// No bite: full HP, dead, above the eat line, or a known-zero danger.
    None,
    /// At or below the critical floor: the best-fitting food is eaten even
    /// when it overheals.
    Critical,
    /// Above the critical floor and up to the eat line: only a food whose
    /// heal fits the missing HP is eaten.
    Caution,
}

/// The eat line: `2 * danger + 1` plus the next poison fire (half the maximum
/// when the danger is unknown), raised to the caller's floor and to the
/// critical floor.
#[inline]
pub(crate) fn eat_line(
    danger: Option<i32>,
    burst: Option<i32>,
    hp_max: i32,
    poison_fire: i32,
    eat_floor: i32,
) -> i32 {
    select::lines(danger, hp_max)
        .with_poison(poison_fire)
        .eat
        .max(eat_floor)
        .max(critical_floor(danger, burst, hp_max, poison_fire))
}

/// The tier of one observation. This is the whole bite-or-not rule that
/// [`eat_choice_excluding`] (the walk guard, the combat machine, the survival
/// step), the route model's replay and the supply planner
/// (`risk::guarded_bound`) all use, so they cannot drift. Only the choice of
/// which food fills the tier stays with the callers that hold several foods.
pub(crate) fn bite_tier(
    hp: i32,
    hp_max: i32,
    danger: Option<i32>,
    burst: Option<i32>,
    poison_fire: i32,
    eat_floor: i32,
) -> BiteTier {
    if hp <= 0
        || hp >= hp_max
        || (danger == Some(0) && poison_fire == 0 && eat_floor < select::lines(danger, hp_max).eat)
        || hp > eat_line(danger, burst, hp_max, poison_fire, eat_floor)
    {
        BiteTier::None
    } else if hp <= critical_floor(danger, burst, hp_max, poison_fire) {
        BiteTier::Critical
    } else {
        BiteTier::Caution
    }
}

/// Whether a single ordinary food of `heal` is eaten at `hp`: in the critical
/// tier always, in the caution tier only when the heal fits the missing HP.
pub(crate) fn eats_food(
    hp: i32,
    hp_max: i32,
    heal: i32,
    danger: Option<i32>,
    burst: Option<i32>,
    poison_fire: i32,
    eat_floor: i32,
) -> bool {
    match bite_tier(hp, hp_max, danger, burst, poison_fire, eat_floor) {
        BiteTier::None => false,
        BiteTier::Critical => true,
        BiteTier::Caution => hp.saturating_add(heal) <= hp_max,
    }
}
/// Shared food selection for Combat, WalkGuard and runner survival checks.
/// The caller owns admission and pending-operation policy; this function only
/// evaluates the current observation. `poison_fire` is the next poison
/// fire's damage. `eat_floor` is a caller-specific lower bound on the bite
/// line. A bot at full HP never bites.
///
/// Two tiers. **Critical**: at or below [`critical_floor`] (the damage that can
/// land before the bite takes effect, plus 1, plus the next poison fire; with
/// an unknown danger, a quarter of the maximum rounded up, plus the fire) the
/// best-fitting food is eaten even when it overheals. **Caution**: above that
/// and up to the eat line (`2 * danger + 1`, half the maximum when the danger
/// is unknown, or the caller's floor) only a food whose heal fits the missing
/// HP is chosen, so no bite is wasted; when none fits nothing is eaten (the
/// damage that can land before the bite cannot kill). A combo bite keeps
/// its own lock-survival gate in both tiers: its purpose is to carry the bot
/// through the input lock, so it may overheal.
pub(crate) fn eat_choice_excluding(
    frame: &Frame<'_>,
    tables: &CombatTables,
    schedule: &Schedule,
    inputs: EatInputs,
    protected_items: &[FoodReserve],
) -> Option<EatChoice> {
    let EatInputs {
        danger,
        burst,
        poison_fire,
        hp,
        hp_max,
        tick,
        eat_floor,
    } = inputs;
    let lines = select::lines(danger, hp_max).with_poison(poison_fire);
    let critical = critical_floor(danger, burst, hp_max, poison_fire);
    let tier = bite_tier(hp, hp_max, danger, burst, poison_fire, eat_floor);
    if tier == BiteTier::None {
        return None;
    }
    let basis = crate::survival::eat::Basis::new(critical, danger);
    let ordinary = food_for_tier(
        tier,
        hp,
        hp_max,
        frame.inventory.iter().map(|row| (row.def.id, row.count)),
        tables,
        None,
        protected_items,
        |food| food.eat_delay_arg.is_some() && schedule.food_ready(food, tick),
    );
    let heal = ordinary
        .and_then(|id| tables.food(id))
        .map_or(0, |food| food.heal);
    if ordinary.is_some() && hp.saturating_add(heal).min(hp_max) > lines.drink_gate {
        return Some(EatChoice {
            ordinary,
            combo: None,
            basis,
        });
    }
    let combo = food_by_excluding(
        hp,
        hp_max,
        frame.inventory.iter().map(|row| (row.def.id, row.count)),
        tables,
        None,
        protected_items,
        |food| {
            let Some(delay) = food.message_delay else {
                return false;
            };
            let gate = danger.map_or((hp_max + 1) / 2, |danger| {
                danger.saturating_mul(delay + 2).saturating_add(1)
            });
            food.eat_delay_arg.is_none()
                && hp
                    .saturating_add(heal)
                    .saturating_add(food.heal)
                    .min(hp_max)
                    > gate
        },
    );
    (ordinary.is_some() || combo.is_some()).then_some(EatChoice {
        ordinary,
        combo,
        basis,
    })
}
/// The cases the real eater (`WalkGuard`, in `guard.rs`) and the route model
/// (`risk::replay`, in `risk/tests.rs`) must both decide the same way. Each
/// attacker is a content NPC id (revision 289, melee, one hit per attack):
/// 1560 Ice Troll (max hit 25, rate 4), 64 Ice spider (6, rate 4). The food
/// is five lobsters (heal 12). `bite` is whether a bite is chosen.
#[cfg(test)]
pub(crate) mod shared_cases {
    pub(crate) struct Case {
        pub label: &'static str,
        pub attackers: &'static [i32],
        pub hp: i32,
        pub hp_max: i32,
        pub bite: bool,
    }
    const TROLL: i32 = 1560;
    const SPIDER: i32 = 64;
    pub(crate) const CASES: &[Case] = &[
        // One strong attacker: danger 25, critical floor 26. The 12-point
        // lobster overheals a 20/30 account, and it is eaten.
        Case {
            label: "single strong attacker is critical",
            attackers: &[TROLL],
            hp: 20,
            hp_max: 30,
            bite: true,
        },
        // Two weak attackers: danger 12, critical floor 13. At 20/30 the
        // next volley cannot kill; the lobster does not fit; no bite.
        Case {
            label: "two weak attackers are not critical",
            attackers: &[SPIDER, SPIDER],
            hp: 20,
            hp_max: 30,
            bite: false,
        },
        // A 20-point account: at 13 the lobster (25) overheals, and the
        // critical floor eats it; at 14 it is caution and no lobster fits.
        Case {
            label: "two weak attackers at their critical floor",
            attackers: &[SPIDER, SPIDER],
            hp: 13,
            hp_max: 20,
            bite: true,
        },
        Case {
            label: "two weak attackers one above their critical floor",
            attackers: &[SPIDER, SPIDER],
            hp: 14,
            hp_max: 20,
            bite: false,
        },
        // Caution tier: the heal fits exactly at 18/30 and not at 19/30.
        Case {
            label: "caution tier eats a lobster that just fits",
            attackers: &[SPIDER, SPIDER],
            hp: 18,
            hp_max: 30,
            bite: true,
        },
        Case {
            label: "caution tier skips a lobster that just overheals",
            attackers: &[SPIDER, SPIDER],
            hp: 19,
            hp_max: 30,
            bite: false,
        },
        Case {
            label: "single strong attacker at its critical floor",
            attackers: &[TROLL],
            hp: 26,
            hp_max: 30,
            bite: true,
        },
        Case {
            label: "single strong attacker one above its critical floor",
            attackers: &[TROLL],
            hp: 27,
            hp_max: 30,
            bite: false,
        },
        // Danger 31 (25 + 6), critical floor 32.
        Case {
            label: "strong plus weak at the summed critical floor",
            attackers: &[TROLL, SPIDER],
            hp: 32,
            hp_max: 40,
            bite: true,
        },
        Case {
            label: "strong plus weak one above the summed critical floor",
            attackers: &[TROLL, SPIDER],
            hp: 33,
            hp_max: 40,
            bite: false,
        },
        // Caution tier, the heal fits: eaten without waste.
        Case {
            label: "caution tier eats a lobster that fits",
            attackers: &[SPIDER, SPIDER],
            hp: 20,
            hp_max: 40,
            bite: true,
        },
        Case {
            label: "full HP never bites",
            attackers: &[TROLL],
            hp: 30,
            hp_max: 30,
            bite: false,
        },
    ];
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hits_before_bite_counts_attacks_inside_the_bite_delay() {
        // The bite delay is two ticks: attacks `rate` ticks apart put at most
        // ceil(2 / rate) of them in the window.
        assert_eq!(BITE_DELAY, 2);
        assert_eq!(hits_before_bite(0), 2);
        assert_eq!(hits_before_bite(1), 2);
        assert_eq!(hits_before_bite(2), 1);
        assert_eq!(hits_before_bite(4), 1);
        assert_eq!(hits_before_bite(u8::MAX), 1);
    }

    #[test]
    fn bite_tier_splits_critical_caution_and_none() {
        use BiteTier::{Caution, Critical, None as No};
        // Danger 6, burst 6: critical floor 7, eat line 13.
        let tier = |hp, hp_max, fire, floor| bite_tier(hp, hp_max, Some(6), Some(6), fire, floor);
        assert_eq!(tier(0, 40, 0, 0), No);
        assert_eq!(tier(1, 40, 0, 0), Critical);
        assert_eq!(tier(7, 40, 0, 0), Critical);
        assert_eq!(tier(8, 40, 0, 0), Caution);
        assert_eq!(tier(13, 40, 0, 0), Caution);
        assert_eq!(tier(14, 40, 0, 0), No);
        assert_eq!(tier(40, 40, 0, 0), No);
        // The caller's floor lifts the line only; the floor stays critical.
        assert_eq!(tier(20, 40, 0, 20), Caution);
        assert_eq!(tier(21, 40, 0, 20), No);
        // The poison fire lifts both.
        assert_eq!(tier(10, 40, 3, 0), Critical);
        assert_eq!(tier(16, 40, 3, 0), Caution);
        // Known-zero danger bites only for a charged fire or a caller floor.
        assert_eq!(bite_tier(1, 40, Some(0), Some(0), 0, 0), No);
        assert_eq!(bite_tier(1, 40, Some(0), Some(0), 2, 0), Critical);
        // Unknown danger: critical a quarter, line half.
        assert_eq!(bite_tier(10, 40, None, None, 0, 0), Critical);
        assert_eq!(bite_tier(11, 40, None, None, 0, 0), Caution);
        assert_eq!(bite_tier(20, 40, None, None, 0, 0), Caution);
        assert_eq!(bite_tier(21, 40, None, None, 0, 0), No);
    }

    #[test]
    fn eats_food_overheals_only_in_the_critical_tier() {
        // 20-point account, danger 6, burst 6, lobster 12.
        let eats = |hp| eats_food(hp, 20, 12, Some(6), Some(6), 0, 0);
        assert!(eats(7), "critical: overheals");
        assert!(eats(8), "caution: 8 + 12 fits");
        assert!(!eats(9), "caution: 9 + 12 overheals");
        assert!(!eats(13));
        assert!(!eats(14), "above the line");
    }

    #[test]
    fn critical_floor_is_the_burst_plus_one_plus_the_fire() {
        assert_eq!(critical_floor(Some(12), Some(12), 40, 0), 13);
        assert_eq!(critical_floor(Some(12), Some(24), 40, 0), 25);
        assert_eq!(critical_floor(Some(12), Some(24), 40, 6), 31);
        // A burst below the danger cannot lower the floor.
        assert_eq!(critical_floor(Some(12), Some(3), 40, 0), 13);
        assert_eq!(critical_floor(Some(12), None, 40, 0), 13);
        // Unknown danger: a quarter of the maximum, rounded up.
        assert_eq!(critical_floor(None, None, 40, 0), 10);
        assert_eq!(critical_floor(None, None, 41, 2), 13);
    }

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
