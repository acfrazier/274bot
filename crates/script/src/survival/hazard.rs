use serde::{Deserialize, Serialize};

/// The Hitpoints value used to evaluate a step's damage formula.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "path-schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum HazardStat {
    /// A constant formula that uses only `plus`.
    None,
    /// The current Hitpoints level.
    Current,
    /// The Hitpoints base level.
    Base,
}

/// A step's own damage source: `min(cap, floor(stat * mul / div) + plus)`.
/// Integer division truncates as the content's `calc` does. `of: None` is a
/// constant (`plus`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StepHazard {
    /// The stat input, if any.
    pub of: HazardStat,
    /// The formula multiplier.
    pub mul: u8,
    /// The formula divisor; compiled hazards require this to be at least 1.
    pub div: u8,
    /// The amount added after integer division.
    pub plus: i8,
    /// Optional upper bound applied after `plus`.
    pub cap: Option<u8>,
}

impl StepHazard {
    /// The most one event deals at current Hitpoints `hp` and base `hp_base`.
    /// The result is never below zero.
    pub fn max_damage(self, hp: i32, hp_base: i32) -> i32 {
        if self.div == 0 {
            // Invalid authored hazards are rejected by the compiler. If a
            // caller constructs one directly, fail closed rather than divide
            // by zero or under-price its damage.
            return i32::MAX;
        }

        let stat = match self.of {
            HazardStat::None => 0,
            HazardStat::Current => hp,
            HazardStat::Base => hp_base,
        };
        let scaled = i64::from(stat) * i64::from(self.mul) / i64::from(self.div);
        let damage = (scaled + i64::from(self.plus)).max(0);
        let damage = self.cap.map_or(damage, |cap| damage.min(i64::from(cap)));
        damage.min(i64::from(i32::MAX)) as i32
    }

    /// The least Hitpoints value in `from..=hp_base` whose quiet, poison-aware
    /// eat line is clear. Returns `None` when no such value exists.
    pub fn release_hitpoints(self, from: i32, hp_base: i32, poison: i32) -> Option<i32> {
        if from > hp_base {
            return None;
        }

        (from..=hp_base).find(|&hp| {
            let eat_line = 2 * i64::from(self.max_damage(hp, hp_base)) + 1 + i64::from(poison);
            i64::from(hp) > eat_line
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{HazardStat, StepHazard};

    const INVENTORY: [InventoryRow; 26] = [
        InventoryRow::declared(
            "quest_mcannon/scripts/locs/railings.rs2:87-91",
            InventoryClass::StepHazard,
            (HazardStat::Base, 1, 10, 0, Some(3)),
            (3, 3),
        ),
        InventoryRow::declared(
            "quest_desertrescue/scripts/quest_desertrescue.rs2:256",
            InventoryClass::StepHazard,
            (HazardStat::None, 0, 1, 7, None),
            (7, 7),
        ),
        InventoryRow::declared(
            "quest_desertrescue/scripts/quest_desertrescue.rs2:271",
            InventoryClass::StepHazard,
            (HazardStat::None, 0, 1, 7, None),
            (7, 7),
        ),
        InventoryRow::declared(
            "quest_desertrescue/scripts/quest_desertrescue.rs2:823",
            InventoryClass::StepHazard,
            (HazardStat::None, 0, 1, 2, None),
            (2, 2),
        ),
        InventoryRow::uncovered(
            "quest_desertrescue/scripts/camp_guard.rs2:38",
            InventoryClass::NpcInitiated,
        ),
        InventoryRow::uncovered(
            "quest_desertrescue/scripts/camp_guard.rs2:111",
            InventoryClass::NpcInitiated,
        ),
        InventoryRow::uncovered(
            "quest_desertrescue/scripts/mercenary_captain.rs2:141",
            InventoryClass::NpcInitiated,
        ),
        InventoryRow::declared(
            "quest_upass/scripts/upass_obstacles.rs2:26",
            InventoryClass::Expressible,
            (HazardStat::None, 0, 1, 3, None),
            (3, 3),
        ),
        InventoryRow::declared(
            "quest_upass/scripts/upass_obstacles.rs2:47",
            InventoryClass::Expressible,
            (HazardStat::Current, 15, 100, 0, None),
            (6, 1),
        ),
        InventoryRow::declared(
            "quest_upass/scripts/upass_obstacles.rs2:118",
            InventoryClass::Expressible,
            (HazardStat::Current, 15, 100, 1, None),
            (7, 2),
        ),
        InventoryRow::declared(
            "quest_upass/scripts/upass_obstacles.rs2:166",
            InventoryClass::Expressible,
            (HazardStat::Current, 10, 100, 1, None),
            (5, 2),
        ),
        InventoryRow::uncovered(
            "quest_upass/scripts/upass_obstacles.rs2:182",
            InventoryClass::ZoneTimer,
        ),
        InventoryRow::uncovered(
            "quest_upass/scripts/upass_obstacles.rs2:186",
            InventoryClass::ZoneTimer,
        ),
        InventoryRow::declared(
            "quest_upass/scripts/upass_obstacles.rs2:279",
            InventoryClass::Expressible,
            (HazardStat::None, 0, 1, 5, None),
            (5, 5),
        ),
        InventoryRow::declared(
            "quest_upass/scripts/upass_obstacles.rs2:324",
            InventoryClass::Expressible,
            (HazardStat::None, 0, 1, 5, None),
            (5, 5),
        ),
        InventoryRow::declared(
            "quest_upass/scripts/upass_obstacles.rs2:380",
            InventoryClass::Expressible,
            (HazardStat::Current, 25, 100, 4, None),
            (14, 6),
        ),
        InventoryRow::declared(
            "quest_upass/scripts/upass_grid.rs2:99",
            InventoryClass::Expressible,
            (HazardStat::None, 0, 1, 15, None),
            (15, 15),
        ),
        InventoryRow::declared(
            "quest_upass/scripts/quest_upass.rs2:137",
            InventoryClass::Expressible,
            (HazardStat::Current, 1, 1, -1, Some(10)),
            (10, 9),
        ),
        InventoryRow::declared(
            "quest_upass/scripts/quest_upass.rs2:145",
            InventoryClass::Expressible,
            (HazardStat::None, 0, 1, 3, None),
            (3, 3),
        ),
        InventoryRow::declared(
            "quest_upass/scripts/quest_upass.rs2:195",
            InventoryClass::Expressible,
            (HazardStat::Current, 20, 100, 0, Some(5)),
            (5, 2),
        ),
        InventoryRow::declared(
            "quest_upass/scripts/quest_upass.rs2:444",
            InventoryClass::Expressible,
            (HazardStat::None, 0, 1, 10, None),
            (10, 10),
        ),
        InventoryRow::declared(
            "quest_upass/scripts/quest_upass.rs2:522",
            InventoryClass::Expressible,
            (HazardStat::None, 0, 1, 10, None),
            (10, 10),
        ),
        InventoryRow::declared(
            "quest_upass/scripts/kamen.rs2:26",
            InventoryClass::RandomBound,
            (HazardStat::None, 0, 1, 5, None),
            (5, 5),
        ),
        InventoryRow::declared(
            "quest_upass/scripts/kardia.rs2:13",
            InventoryClass::Expressible,
            (HazardStat::Current, 25, 100, 1, None),
            (11, 3),
        ),
        InventoryRow::uncovered(
            "quest_upass/scripts/lord_iban.rs2:15-36",
            InventoryClass::MultiHit,
        ),
        InventoryRow::declared(
            "areas/area_yanille/scripts/agility_dungeon.rs2:140",
            InventoryClass::Expressible,
            (HazardStat::Base, 15, 100, 1, None),
            (7, 7),
        ),
    ];

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum InventoryClass {
        StepHazard,
        Expressible,
        RandomBound,
        NpcInitiated,
        ZoneTimer,
        MultiHit,
    }

    #[derive(Clone, Copy)]
    struct InventoryRow {
        source: &'static str,
        class: InventoryClass,
        hazard: Option<StepHazard>,
        h40: Option<i32>,
        h10: Option<i32>,
    }

    impl InventoryRow {
        const fn declared(
            source: &'static str,
            class: InventoryClass,
            (of, mul, div, plus, cap): (HazardStat, u8, u8, i8, Option<u8>),
            (h40, h10): (i32, i32),
        ) -> Self {
            Self {
                source,
                class,
                hazard: Some(StepHazard {
                    of,
                    mul,
                    div,
                    plus,
                    cap,
                }),
                h40: Some(h40),
                h10: Some(h10),
            }
        }

        const fn uncovered(source: &'static str, class: InventoryClass) -> Self {
            Self {
                source,
                class,
                hazard: None,
                h40: None,
                h10: None,
            }
        }
    }

    #[test]
    fn hazard_max_damage_matches_the_content_formulas() {
        for row in INVENTORY.iter().filter(|row| row.hazard.is_some()) {
            let hazard = row.hazard.expect("declared inventory row has a formula");
            assert_eq!(
                hazard.max_damage(40, 40),
                row.h40.unwrap(),
                "{}",
                row.source
            );
            assert_eq!(
                hazard.max_damage(10, 40),
                row.h10.unwrap(),
                "{}",
                row.source
            );
        }

        let after_plus_cap = StepHazard {
            of: HazardStat::Current,
            mul: 15,
            div: 100,
            plus: 1,
            cap: Some(5),
        };
        assert_eq!(after_plus_cap.max_damage(40, 40), 5);
        assert_eq!(
            StepHazard {
                of: HazardStat::Base,
                mul: 1,
                div: 10,
                plus: 0,
                cap: Some(3),
            }
            .max_damage(20, 20),
            2
        );
        assert_eq!(
            StepHazard {
                of: HazardStat::Current,
                mul: 15,
                div: 100,
                plus: 1,
                cap: None,
            }
            .max_damage(1, 40),
            1
        );
        let negative = StepHazard {
            of: HazardStat::Current,
            mul: 1,
            div: 1,
            plus: -2,
            cap: Some(4),
        };
        assert_eq!(negative.max_damage(3, 40), 1);
        assert_eq!(negative.max_damage(1, 40), 0);
    }

    #[test]
    fn hazard_inventory_rows_are_expressible_or_declared_fixed() {
        assert_eq!(INVENTORY.len(), 26);
        for row in INVENTORY {
            match row.class {
                InventoryClass::StepHazard | InventoryClass::Expressible => {
                    let hazard = row.hazard.expect("expressible damage has a declaration");
                    assert_eq!(
                        hazard.max_damage(40, 40),
                        row.h40.unwrap(),
                        "{}",
                        row.source
                    );
                    assert_eq!(
                        hazard.max_damage(10, 40),
                        row.h10.unwrap(),
                        "{}",
                        row.source
                    );
                }
                InventoryClass::RandomBound => {
                    let hazard = row.hazard.expect("random bound has a declaration");
                    assert_eq!(
                        hazard.max_damage(40, 40),
                        row.h40.unwrap(),
                        "{}",
                        row.source
                    );
                    assert_eq!(
                        hazard.max_damage(10, 40),
                        row.h10.unwrap(),
                        "{}",
                        row.source
                    );
                    assert!(hazard.max_damage(40, 40) >= 5, "{}", row.source);
                }
                InventoryClass::NpcInitiated
                | InventoryClass::ZoneTimer
                | InventoryClass::MultiHit => {
                    assert!(row.hazard.is_none(), "{}", row.source);
                    assert!(row.h40.is_none(), "{}", row.source);
                    assert!(row.h10.is_none(), "{}", row.source);
                }
            }
        }
    }

    #[test]
    fn admitted_hazard_emergency_line_is_never_below_half() {
        for hp_max in 10..=99 {
            for hazard in 0..hp_max {
                if 2 * hazard + 1 < hp_max {
                    assert!(
                        hazard < (hp_max + 1) / 2,
                        "hazard {hazard} at max Hitpoints {hp_max}"
                    );
                }
            }
        }
    }

    #[test]
    fn food_park_release_uses_the_line_of_the_current_hitpoints() {
        assert_eq!(
            StepHazard {
                of: HazardStat::Current,
                mul: 25,
                div: 100,
                plus: 4,
                cap: None,
            }
            .release_hitpoints(10, 40, 0),
            Some(18)
        );
        let railing = StepHazard {
            of: HazardStat::Base,
            mul: 1,
            div: 10,
            plus: 0,
            cap: Some(3),
        };
        assert_eq!(railing.release_hitpoints(7, 40, 0), Some(8));
        assert_eq!(railing.release_hitpoints(4, 40, 0), Some(8));
        assert_eq!(
            StepHazard {
                of: HazardStat::None,
                mul: 0,
                div: 1,
                plus: 7,
                cap: None,
            }
            .release_hitpoints(7, 40, 0),
            Some(16)
        );
        assert_eq!(
            StepHazard {
                of: HazardStat::None,
                mul: 0,
                div: 1,
                plus: 15,
                cap: None,
            }
            .release_hitpoints(1, 10, 0),
            None
        );
    }

    #[test]
    fn zero_divisor_fails_closed() {
        assert_eq!(
            StepHazard {
                of: HazardStat::None,
                mul: 0,
                div: 0,
                plus: 1,
                cap: None,
            }
            .max_damage(10, 10),
            i32::MAX
        );
    }
}
