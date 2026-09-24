//! Native boost-potion descriptors, planning and sip selection.
//!
//! The frozen `bot/api/combat/boostPotions` helper holds script-owned policy:
//! which dose forms exist, which flask the bank run draws, and whether a boost
//! is due another dose. That policy lives here. The v1 exports that walk the
//! caller's own objects (`plannedPotions(carry)`, `potionToSip(state)`) are one
//! native call each (`load::boost_potions_v8`), which reads the caller's rows
//! and calls its `levels`/`held` callbacks in the frozen order around the
//! table and [`boost_faded_by`] below. The v2 typed helpers use
//! [`planned_potions`] / [`potion_to_sip`] over marshalled values.

use std::convert::Infallible;

/// Dose counts, high to low, of every dose form the table lists.
const DOSE_COUNTS: [u32; 4] = [4, 3, 2, 1];

/// Index into [`DOSE_COUNTS`] of the dose form drawn when the loadout names none.
const DEFAULT_DOSE: usize = 1;

/// Flasks to carry per trip when the loadout names no dose for a potion.
pub const DEFAULT_WANT: i64 = 1;

/// One super-potion row: the skill the dose lifts, the paint label kept short
/// enough for a three-column row, and the dose-name stem.
pub struct BoostPotion {
    pub skill: &'static str,
    pub short: &'static str,
    stem: &'static str,
}

impl BoostPotion {
    /// Every dose form, high to low, so a part-used flask still counts as one in
    /// the pack.
    pub fn doses(&self) -> Vec<String> {
        (0..DOSE_COUNTS.len())
            .filter_map(|dose| self.dose_name(dose))
            .collect()
    }

    /// The dose form drawn when the loadout names none.
    pub fn flask(&self) -> String {
        self.dose_name(DEFAULT_DOSE).unwrap_or_default()
    }

    /// The canonical name of dose `dose` (0 is the four-dose flask).
    pub fn dose_name(&self, dose: usize) -> Option<String> {
        DOSE_COUNTS
            .get(dose)
            .map(|count| format!("{}({count})", self.stem))
    }

    /// Whether dose `dose` is the one `key` names, case-insensitively.
    ///
    /// `key` is the carried item the shim read at the frozen comparison's own
    /// right operand for this dose, already trimmed and lowercased.
    pub fn dose_matches(&self, dose: usize, key: &str) -> bool {
        self.dose_name(dose)
            .is_some_and(|name| name.to_lowercase() == key)
    }
}

pub const SUPER_ATTACK: BoostPotion = BoostPotion {
    skill: "attack",
    short: "Att",
    stem: "Super attack",
};

pub const SUPER_STRENGTH: BoostPotion = BoostPotion {
    skill: "strength",
    short: "Str",
    stem: "Super strength",
};

/// Checked in this order; attack wins a tick both could use.
pub const BOOST_POTIONS: [&BoostPotion; 2] = [&SUPER_ATTACK, &SUPER_STRENGTH];

/// What the last dose leaves behind.
pub const EMPTY_VIAL: &str = "Vial";

/// Share of the base level the boost may decay to before another dose is worth
/// its tick.
pub const BOOST_FLOOR: f64 = 0.1;

/// One operand read of the frozen `boostFaded` expression.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FadedOperand {
    Base,
    Effective,
    Floor,
}

/// Whether the boost has decayed back into the floor band, reading each
/// operand where the frozen expression does:
///
/// ```text
/// const boost = effective - base;
/// return base > 0 && boost >= 0 && boost <= floor * base;
/// ```
///
/// `read` converts one operand (`ToNumber`) per use, in that order —
/// effective, base, base, then floor and base only once the earlier terms
/// hold — so a caller-supplied operand is converted exactly as often as the
/// frozen code converts it. A boost sitting exactly on the threshold is due,
/// and a level drained *below* its base is not a decayed boost, because a
/// super potion restores none of it.
pub fn boost_faded_by<C: ?Sized, E>(
    cx: &mut C,
    mut read: impl FnMut(&mut C, FadedOperand) -> Result<f64, E>,
) -> Result<bool, E> {
    let boost = read(cx, FadedOperand::Effective)? - read(cx, FadedOperand::Base)?;
    Ok(read(cx, FadedOperand::Base)? > 0.0
        && boost >= 0.0
        && boost <= read(cx, FadedOperand::Floor)? * read(cx, FadedOperand::Base)?)
}

/// [`boost_faded_by`] over plain numbers.
pub fn boost_faded(base: f64, effective: f64, floor: f64) -> bool {
    let result = boost_faded_by(&mut (), |_, operand| {
        Ok::<_, Infallible>(match operand {
            FadedOperand::Base => base,
            FadedOperand::Effective => effective,
            FadedOperand::Floor => floor,
        })
    });
    match result {
        Ok(faded) => faded,
        Err(never) => match never {},
    }
}

/// One planned flask recommendation. `short` is table-owned for
/// [`planned_potions`] and optional on a caller-supplied sip plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PotionPlan {
    pub skill: String,
    pub short: Option<String>,
    pub flask: String,
    pub doses: Vec<String>,
    pub want: u32,
}

/// One carry row used by [`planned_potions`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedCarry {
    pub item: String,
    pub qty: u32,
}

/// One skill observation used by [`potion_to_sip`]. Missing rows stay absent.
#[derive(Debug, Clone, PartialEq)]
pub struct PotionLevel {
    pub skill: String,
    pub base: f64,
    pub effective: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PotionToSipError {
    InvalidArgs,
    MissingObservation,
}

/// Recommend the melee pair. First matching carry dose wins; otherwise flask `(3)`, `want: 1`.
///
/// This is a withdraw-form recommendation. It does not observe inventory or drink.
pub fn planned_potions(carry: &[PlannedCarry]) -> Vec<PotionPlan> {
    BOOST_POTIONS
        .iter()
        .map(|potion| {
            let matched = carry.iter().find_map(|row| {
                let key = row.item.trim().to_lowercase();
                (0..DOSE_COUNTS.len()).find_map(|dose| {
                    if potion.dose_matches(dose, &key) {
                        potion.dose_name(dose).map(|flask| (flask, row.qty))
                    } else {
                        None
                    }
                })
            });
            let (flask, want) = matched.unwrap_or_else(|| (potion.flask(), DEFAULT_WANT as u32));
            PotionPlan {
                skill: potion.skill.to_string(),
                short: Some(potion.short.to_string()),
                flask,
                doses: potion.doses(),
                want,
            }
        })
        .collect()
}

/// First held plan whose levels are faded. `held == 0` skips that plan.
///
/// A held plan with no matching level row is [`PotionToSipError::MissingObservation`].
/// An explicit `base == 0` row is a known observation and uses current `boost_faded`.
pub fn potion_to_sip(
    plans: &[PotionPlan],
    held: &[f64],
    levels: &[PotionLevel],
) -> Result<Option<PotionPlan>, PotionToSipError> {
    if held.len() != plans.len() {
        return Err(PotionToSipError::InvalidArgs);
    }
    for &count in held {
        if !count.is_finite() || count < 0.0 {
            return Err(PotionToSipError::InvalidArgs);
        }
    }
    let mut seen: Vec<&str> = Vec::new();
    for level in levels {
        if !level.base.is_finite() || !level.effective.is_finite() {
            return Err(PotionToSipError::InvalidArgs);
        }
        if seen
            .iter()
            .any(|skill| skill.eq_ignore_ascii_case(&level.skill))
        {
            return Err(PotionToSipError::InvalidArgs);
        }
        seen.push(level.skill.as_str());
    }
    if plans.is_empty() {
        return Ok(None);
    }
    for (plan, &count) in plans.iter().zip(held.iter()) {
        if count == 0.0 {
            continue;
        }
        let Some(level) = levels
            .iter()
            .find(|level| level.skill.eq_ignore_ascii_case(&plan.skill))
        else {
            return Err(PotionToSipError::MissingObservation);
        };
        if boost_faded(level.base, level.effective, BOOST_FLOOR) {
            return Ok(Some(plan.clone()));
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_table_is_the_frozen_descriptor_rows_in_order() {
        let rows: Vec<_> = BOOST_POTIONS
            .iter()
            .map(|potion| (potion.skill, potion.short, potion.flask(), potion.doses()))
            .collect();
        assert_eq!(
            rows,
            [
                (
                    "attack",
                    "Att",
                    "Super attack(3)".to_string(),
                    vec![
                        "Super attack(4)".to_string(),
                        "Super attack(3)".into(),
                        "Super attack(2)".into(),
                        "Super attack(1)".into()
                    ]
                ),
                (
                    "strength",
                    "Str",
                    "Super strength(3)".to_string(),
                    vec![
                        "Super strength(4)".to_string(),
                        "Super strength(3)".into(),
                        "Super strength(2)".into(),
                        "Super strength(1)".into()
                    ]
                ),
            ],
            "attack is checked before strength"
        );
        assert_eq!(BOOST_FLOOR, 0.1);
        assert_eq!(EMPTY_VIAL, "Vial");
    }

    #[test]
    fn boost_faded_is_the_floor_band_and_never_a_drained_level() {
        let faded = |base: f64, effective: f64| boost_faded(base, effective, BOOST_FLOOR);
        assert!(faded(70.0, 70.0), "an unboosted skill is due its first sip");
        assert!(faded(70.0, 77.0), "a boost exactly on the tenth is due");
        assert!(!faded(70.0, 78.0), "one level above the tenth is not");
        assert!(!faded(70.0, 85.0), "a fresh super potion is not faded");
        assert!(faded(1.0, 1.0), "a tenth of one rounds to nothing");
        assert!(!faded(1.0, 2.0));
        assert!(!faded(0.0, 0.0), "an unread skill never asks for a dose");
        assert!(!faded(70.0, 60.0), "a drained skill is not a decayed boost");
        assert!(boost_faded(70.0, 77.0, 0.2));
        assert!(!boost_faded(70.0, 77.0, 0.0));
        assert!(!boost_faded(f64::NAN, f64::NAN, BOOST_FLOOR));
        assert!(!boost_faded(70.0, f64::INFINITY, BOOST_FLOOR));
        assert!(
            boost_faded(70.0, f64::INFINITY, f64::INFINITY),
            "the frozen arithmetic keeps `Infinity <= Infinity * 70` due"
        );
    }

    #[test]
    fn boost_faded_reads_operands_in_the_frozen_order_and_short_circuits() {
        let reads = |base: f64, effective: f64| {
            let mut log = Vec::new();
            let faded = boost_faded_by(&mut log, |log, operand| {
                log.push(operand);
                Ok::<_, Infallible>(match operand {
                    FadedOperand::Base => base,
                    FadedOperand::Effective => effective,
                    FadedOperand::Floor => BOOST_FLOOR,
                })
            })
            .unwrap();
            (faded, log)
        };
        use FadedOperand::{Base, Effective, Floor};
        assert_eq!(reads(70.0, 70.0), (true, vec![Effective, Base, Base, Floor, Base]));
        assert_eq!(
            reads(0.0, 0.0),
            (false, vec![Effective, Base, Base]),
            "`base > 0` fails before the floor term is read"
        );
        assert_eq!(
            reads(70.0, 60.0),
            (false, vec![Effective, Base, Base]),
            "a drained boost fails before the floor term is read"
        );
    }

    #[test]
    fn a_dose_matches_only_its_lowercased_canonical_name() {
        assert!(SUPER_ATTACK.dose_matches(2, "super attack(2)"));
        assert!(!SUPER_ATTACK.dose_matches(2, "super attack(5)"));
        assert!(!SUPER_ATTACK.dose_matches(2, "Super attack(2)"));
        assert!(!SUPER_ATTACK.dose_matches(0, "super attack(2)"));
        assert!(!SUPER_ATTACK.dose_matches(4, "super attack(5)"));
        assert!(
            !SUPER_ATTACK.dose_matches(0, "super strength(4)"),
            "a dose form belongs to its own potion"
        );
    }

    #[test]
    fn planned_potions_value_matches_first_dose_then_fallback() {
        let defaults = planned_potions(&[]);
        assert_eq!(defaults[0].flask, "Super attack(3)");
        assert_eq!(defaults[0].want, 1);
        assert_eq!(defaults[0].short.as_deref(), Some("Att"));
        assert_eq!(defaults[1].flask, "Super strength(3)");
        let named = planned_potions(&[
            PlannedCarry {
                item: " Super Attack(4) ".into(),
                qty: 3,
            },
            PlannedCarry {
                item: "Lobster".into(),
                qty: 10,
            },
        ]);
        assert_eq!(named[0].flask, "Super attack(4)");
        assert_eq!(named[0].want, 3);
        assert_eq!(named[1].flask, "Super strength(3)");
        assert_eq!(named[1].want, 1);
    }

    #[test]
    fn potion_to_sip_does_not_treat_missing_levels_as_zero() {
        let plans = planned_potions(&[]);
        assert_eq!(potion_to_sip(&[], &[], &[]).unwrap(), None);
        assert_eq!(
            potion_to_sip(&plans, &[0.0, 0.0], &[]).unwrap(),
            None,
            "empty packs skip without needing levels"
        );
        assert_eq!(
            potion_to_sip(&plans, &[1.0, 1.0], &[]),
            Err(PotionToSipError::MissingObservation)
        );
        let later_due = potion_to_sip(
            &plans,
            &[1.0, 1.0],
            &[PotionLevel {
                skill: "strength".into(),
                base: 70.0,
                effective: 70.0,
            }],
        );
        assert_eq!(
            later_due,
            Err(PotionToSipError::MissingObservation),
            "a held unknown earlier plan is not skipped for a later due plan"
        );
        let explicit_zero = potion_to_sip(
            &plans,
            &[1.0, 1.0],
            &[
                PotionLevel {
                    skill: "Attack".into(),
                    base: 0.0,
                    effective: 0.0,
                },
                PotionLevel {
                    skill: "strength".into(),
                    base: 70.0,
                    effective: 70.0,
                },
            ],
        )
        .unwrap();
        assert_eq!(
            explicit_zero.as_ref().map(|plan| plan.skill.as_str()),
            Some("strength"),
            "present base 0 is a known false, not a missing observation"
        );
    }
}
