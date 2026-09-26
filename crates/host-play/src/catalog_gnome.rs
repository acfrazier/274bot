use super::*;
pub fn gnome_wrong_bows(observation: &Observation) -> bool {
    observation.item_id(MAGIC_SHORTBOW_ID) > 0
        || observation.bank_item_id(MAGIC_SHORTBOW_ID) > 0
        || observation.item_id(MAGIC_LONGBOW_ID) > 0
        || observation.bank_item_id(MAGIC_LONGBOW_ID) > 0
}

pub fn gnome_noted(observation: &Observation) -> bool {
    observation.item_id(NOTED_MAGIC_LOGS_ID) > 0
        || observation.bank_item_id(NOTED_MAGIC_LOGS_ID) > 0
        || observation.item_id(NOTED_UNSTRUNG_MAGIC_SHORTBOW_ID) > 0
        || observation.bank_item_id(NOTED_UNSTRUNG_MAGIC_SHORTBOW_ID) > 0
        || observation.item_id(NOTED_UNSTRUNG_MAGIC_LONGBOW_ID) > 0
        || observation.bank_item_id(NOTED_UNSTRUNG_MAGIC_LONGBOW_ID) > 0
}

pub fn gnome_resource_tools(observation: &Observation) -> bool {
    held_id(observation, RUNE_AXE_ID) == 1 && observation.item_id(KNIFE_ID) == GNOME_BALLAST_KNIVES
}

pub fn gnome_chop_baseline_ready(baseline: &Observation) -> bool {
    near(baseline.tile, GNOME_SOUTH_BANK_MAGIC_STAND, 1)
        && baseline.magic_tree_ready
        && baseline.level("woodcutting") >= 75
        && gnome_resource_tools(baseline)
        && baseline.item_id(MAGIC_LOGS_ID) == 0
        && baseline.item_id(UNSTRUNG_MAGIC_SHORTBOW_ID) == 0
        && baseline.item_id(UNSTRUNG_MAGIC_LONGBOW_ID) == 0
        && !gnome_wrong_bows(baseline)
        && !gnome_noted(baseline)
}

pub fn gnome_fletch_baseline_ready(
    baseline: &Observation,
    fletching: i32,
    max: Option<i32>,
) -> bool {
    gnome_chop_baseline_ready(baseline)
        && baseline.level("fletching") >= fletching
        && max
            .map(|max| baseline.level("fletching") <= max)
            .unwrap_or(true)
}
/// Chop magic logs 1513 with Woodcutting XP, deposit upstairs, return to
/// ground, chop again. Strung bows and noted logs fail.
#[derive(Debug, Clone, Default, Serialize)]
pub struct GnomeChopCycle {
    pub chopped: Option<Observation>,
    pub deposited: Option<Observation>,
    pub returned: bool,
    pub further: bool,
    pub wrong_product: bool,
    pub noted: bool,
}

impl GnomeChopCycle {
    pub fn observe(&mut self, baseline: &Observation, now: &Observation) {
        self.wrong_product |= gnome_wrong_bows(now)
            || now.item_id(UNSTRUNG_MAGIC_SHORTBOW_ID) > 0
            || now.bank_item_id(UNSTRUNG_MAGIC_SHORTBOW_ID) > 0
            || now.item_id(UNSTRUNG_MAGIC_LONGBOW_ID) > 0
            || now.bank_item_id(UNSTRUNG_MAGIC_LONGBOW_ID) > 0;
        self.noted |= gnome_noted(now);
        if self.chopped.is_none()
            && now.item_id(MAGIC_LOGS_ID) >= 1
            && baseline.item_id(MAGIC_LOGS_ID) == 0
            && now.skill_xp("woodcutting") > baseline.skill_xp("woodcutting")
            && gnome_resource_tools(now)
            && !gnome_wrong_bows(now)
            && !gnome_noted(now)
        {
            self.chopped = Some(now.clone());
        }
        if self.chopped.is_some()
            && self.deposited.is_none()
            && now.bank_open
            && now.bank_loaded
            && now.bank_generation > baseline.bank_generation
            && now.item_id(MAGIC_LOGS_ID) == 0
            && now.bank_item_id(MAGIC_LOGS_ID) >= 1
            && near(now.tile, GNOME_BANK_STAND, 8)
            && gnome_resource_tools(now)
        {
            self.deposited = Some(now.clone());
        }
        if let Some(deposited) = &self.deposited {
            self.returned |= !now.bank_open
                && !now.bank_loaded
                && now.bank_generation > deposited.bank_generation
                && near(now.tile, GNOME_BANK_STAIR_SOUTH, 30)
                && gnome_resource_tools(now);
        }
        if self.returned {
            self.further |=
                !now.bank_open && now.item_id(MAGIC_LOGS_ID) >= 1 && gnome_resource_tools(now);
        }
    }

    pub fn qualified(&self) -> bool {
        self.further
            && self.chopped.is_some()
            && self.deposited.is_some()
            && !self.wrong_product
            && !self.noted
    }
}

/// Chop logs, consume them into exact unstrung 72 or 70 with Fletching XP,
/// deposit upstairs, return, chop again. The other unstrung id and strung
/// 861/859 fail. Missing knife cannot qualify.
#[derive(Debug, Clone, Default, Serialize)]
pub struct GnomeFletchCycle {
    pub chopped: Option<Observation>,
    pub fletched: Option<Observation>,
    pub deposited: Option<Observation>,
    pub returned: bool,
    pub further: bool,
    pub wrong_product: bool,
    pub noted: bool,
}

impl GnomeFletchCycle {
    pub fn observe(&mut self, product: i32, other: i32, baseline: &Observation, now: &Observation) {
        self.wrong_product |=
            gnome_wrong_bows(now) || now.item_id(other) > 0 || now.bank_item_id(other) > 0;
        self.noted |= gnome_noted(now);
        if self.chopped.is_none()
            && now.item_id(MAGIC_LOGS_ID) >= 1
            && baseline.item_id(MAGIC_LOGS_ID) == 0
            && now.skill_xp("woodcutting") > baseline.skill_xp("woodcutting")
            && now.item_id(product) == 0
            && gnome_resource_tools(now)
            && !gnome_wrong_bows(now)
            && !gnome_noted(now)
        {
            self.chopped = Some(now.clone());
        }
        if self.chopped.is_some()
            && self.fletched.is_none()
            && now.item_id(product) >= 1
            && now.item_id(MAGIC_LOGS_ID)
                < self
                    .chopped
                    .as_ref()
                    .map(|row| row.item_id(MAGIC_LOGS_ID))
                    .unwrap_or(0)
            && now.skill_xp("fletching") > baseline.skill_xp("fletching")
            && now.item_id(other) == 0
            && gnome_resource_tools(now)
            && !gnome_wrong_bows(now)
            && !gnome_noted(now)
        {
            self.fletched = Some(now.clone());
        }
        if self.fletched.is_some()
            && self.deposited.is_none()
            && now.bank_open
            && now.bank_loaded
            && now.bank_generation > baseline.bank_generation
            && now.item_id(product) == 0
            && now.bank_item_id(product) >= 1
            && near(now.tile, GNOME_BANK_STAND, 8)
            && gnome_resource_tools(now)
        {
            self.deposited = Some(now.clone());
        }
        if let Some(deposited) = &self.deposited {
            self.returned |= !now.bank_open
                && !now.bank_loaded
                && now.bank_generation > deposited.bank_generation
                && near(now.tile, GNOME_BANK_STAIR_SOUTH, 30)
                && gnome_resource_tools(now);
        }
        if self.returned {
            self.further |=
                !now.bank_open && now.item_id(MAGIC_LOGS_ID) >= 1 && gnome_resource_tools(now);
        }
    }

    pub fn qualified(&self) -> bool {
        self.further
            && self.chopped.is_some()
            && self.fletched.is_some()
            && self.deposited.is_some()
            && !self.wrong_product
            && !self.noted
    }
}
