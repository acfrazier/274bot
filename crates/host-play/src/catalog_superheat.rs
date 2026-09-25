use super::*;
pub fn superheater_baseline_ready(
    baseline: &Observation,
    bar: i32,
    primary: i32,
    secondary: i32,
    staff: i32,
    smithing: i32,
) -> bool {
    near(baseline.tile, (3185, 3440, 0), 6)
        && baseline.level("magic") >= 43
        && baseline.level("smithing") >= smithing
        && baseline.item_id(bar) == 0
        && baseline.item_id(primary) == 0
        && baseline.item_id(secondary) == 0
        && baseline.item_id(NATURE_RUNE_ID) == 0
        && baseline.item_id(staff) == 0
        && baseline.item_id(IRON_BAR_ID) == 0
        && baseline.equipment_id(staff) == 0
}
/// Superheat a trip, deposit bars except natures, restock ores, then smelt again.
#[derive(Debug, Clone, Default, Serialize)]
pub struct SuperheaterCycle {
    pub first_bars: bool,
    pub deposited: Option<Observation>,
    pub withdrawn: Option<Observation>,
    pub produced_after_withdrawal: bool,
    pub wrong_product: bool,
    pub wrong_staff: bool,
}

pub struct SuperheaterSpec {
    pub bar: i32,
    pub primary: i32,
    pub secondary: i32,
    pub staff: i32,
    pub steel: bool,
}

impl SuperheaterCycle {
    pub fn observe(&mut self, spec: SuperheaterSpec, baseline: &Observation, now: &Observation) {
        let SuperheaterSpec {
            bar,
            primary,
            secondary,
            staff,
            steel,
        } = spec;
        let wrong_bars = [IRON_BAR_ID, BRONZE_BAR_ID, STEEL_BAR_ID, SILVER_BAR_ID]
            .into_iter()
            .filter(|id| *id != bar)
            .any(|id| now.item_id(id) > 0 || now.bank_item_id(id) > 0);
        self.wrong_product |= wrong_bars;
        if now.equipment_id(STAFF_OF_FIRE_ID) > 0 && staff != STAFF_OF_FIRE_ID {
            self.wrong_staff = true;
        }
        if now.equipment_id(FIRE_BATTLESTAFF_ID) > 0 && staff != FIRE_BATTLESTAFF_ID {
            self.wrong_staff = true;
        }
        self.first_bars |= now.item_id(bar) >= 1
            && now.item_id(NATURE_RUNE_ID) >= 1
            && now.item_id(staff) == 0
            && now.equipment_id(staff) >= 1
            && now.skill_xp("magic") > baseline.skill_xp("magic")
            && now.skill_xp("smithing") > baseline.skill_xp("smithing")
            && baseline.item_id(bar) == 0
            && !wrong_bars;
        if self.first_bars
            && self.deposited.is_none()
            && now.bank_open
            && now.bank_loaded
            && now.bank_generation > baseline.bank_generation
            && now.item_id(bar) == 0
            && now.bank_item_id(bar) >= 1
            && now.item_id(NATURE_RUNE_ID) >= 1
            && now.item_id(primary) == 0
            && now.item_id(secondary) == 0
        {
            self.deposited = Some(now.clone());
        }
        if let Some(deposited) = &self.deposited {
            if self.withdrawn.is_none()
                && now.bank_open
                && now.bank_loaded
                && now.bank_generation == deposited.bank_generation
                && now.item_id(primary) >= 1
                && now.item_id(secondary) >= 1
                && now.bank_item_id(primary) < deposited.bank_item_id(primary)
                && now.bank_item_id(secondary) < deposited.bank_item_id(secondary)
                && now.item_id(NATURE_RUNE_ID) >= 1
                && if steel {
                    now.item_id(secondary) == 2 * now.item_id(primary)
                } else {
                    now.item_id(primary) == now.item_id(secondary)
                }
            {
                self.withdrawn = Some(now.clone());
            }
        }
        if let Some(withdrawn) = &self.withdrawn {
            self.produced_after_withdrawal |= !now.bank_open
                && !now.bank_loaded
                && now.item_id(bar) >= 1
                && now.item_id(primary) < withdrawn.item_id(primary)
                && now.item_id(secondary) < withdrawn.item_id(secondary)
                && now.item_id(NATURE_RUNE_ID) < withdrawn.item_id(NATURE_RUNE_ID)
                && now.skill_xp("magic") > withdrawn.skill_xp("magic")
                && now.skill_xp("smithing") > withdrawn.skill_xp("smithing")
                && now.equipment_id(staff) >= 1;
        }
    }

    pub fn qualified(&self) -> bool {
        self.produced_after_withdrawal && !self.wrong_product && !self.wrong_staff
    }
}