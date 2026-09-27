use super::*;

pub fn flax_aio_baseline_ready(baseline: &Observation) -> bool {
    near(baseline.tile, FLAX_FIELD, 6)
        && baseline.level("crafting") >= 1
        && baseline.item_id(FLAX_ID) == 0
        && baseline.item_id(BOW_STRING_ID) == 0
        && baseline.item_id(BALL_OF_WOOL_ID) == 0
        && !flax_spinner_noted(baseline)
}

pub fn flax_aio_pick_baseline_ready(baseline: &Observation) -> bool {
    near(baseline.tile, FLAX_FIELD, 6)
        && baseline.item_id(FLAX_ID) == 0
        && baseline.item_id(BOW_STRING_ID) == 0
        && !flax_spinner_noted(baseline)
}

pub fn flax_aio_spin_baseline_ready(baseline: &Observation) -> bool {
    near(baseline.tile, FLAX_AIO_BANK, 8)
        && baseline.level("crafting") >= 1
        && baseline.item_id(FLAX_ID) == 0
        && baseline.item_id(BOW_STRING_ID) == 0
        && baseline.item_id(BALL_OF_WOOL_ID) == 0
        && !flax_spinner_noted(baseline)
}
/// Full pack of exact flax 1779, Seers deposit, return, further pick.
#[derive(Debug, Clone, Default, Serialize)]
pub struct FlaxPickerCycle {
    pub first_pack: bool,
    pub deposited: Option<Observation>,
    pub returned: bool,
    pub further: bool,
}

impl FlaxPickerCycle {
    pub fn observe(&mut self, baseline: &Observation, now: &Observation) {
        self.first_pack |= now.item_id(FLAX_ID) >= 28
            && baseline.item_id(FLAX_ID) == 0
            && now.item_id(FLAX_ID) > baseline.item_id(FLAX_ID);
        if self.first_pack
            && self.deposited.is_none()
            && now.bank_open
            && now.bank_loaded
            && now.bank_generation > baseline.bank_generation
            && now.item_id(FLAX_ID) == 0
            && now.bank_item_id(FLAX_ID) >= 28
        {
            self.deposited = Some(now.clone());
        }
        if let Some(deposited) = &self.deposited {
            self.returned |= !now.bank_open
                && !now.bank_loaded
                // Closing the modal advances the bank session generation.
                && now.bank_generation > deposited.bank_generation
                && near(now.tile, FLAX_FIELD, 12);
        }
        if self.returned {
            self.further |= !now.bank_open && now.item_id(FLAX_ID) >= 1;
        }
    }

    pub fn qualified(&self) -> bool {
        self.further
    }
}

/// Pick 1779 at the field, convert at the wheel with Crafting XP, deposit 1777,
/// closed return to the field, further Pick. Wool or noted ids fail.
#[derive(Debug, Clone, Default, Serialize)]
pub struct FlaxAioCycle {
    pub picked: Option<Observation>,
    pub produced: Option<Observation>,
    pub deposited: Option<Observation>,
    pub returned: bool,
    pub further: bool,
    pub wrong_product: bool,
    pub noted: bool,
}

impl FlaxAioCycle {
    pub fn observe(&mut self, baseline: &Observation, now: &Observation) {
        self.wrong_product |=
            now.item_id(BALL_OF_WOOL_ID) > 0 || now.bank_item_id(BALL_OF_WOOL_ID) > 0;
        self.noted |= flax_spinner_noted(now);
        if self.picked.is_none()
            && now.item_id(FLAX_ID) >= 1
            && baseline.item_id(FLAX_ID) == 0
            && now.item_id(BOW_STRING_ID) == 0
            && !self.noted
            && !self.wrong_product
        {
            self.picked = Some(now.clone());
        }
        if let Some(picked) = &self.picked {
            if self.produced.is_none()
                && now.item_id(BOW_STRING_ID) >= 1
                && now.item_id(FLAX_ID) < picked.item_id(FLAX_ID)
                && now.skill_xp("crafting") > baseline.skill_xp("crafting")
                && near(now.tile, FLAX_SPINNER_WHEEL, 8)
                && now.item_id(BALL_OF_WOOL_ID) == 0
                && !self.noted
            {
                self.produced = Some(now.clone());
            }
        }
        if self.produced.is_some()
            && self.deposited.is_none()
            && now.bank_open
            && now.bank_loaded
            && now.bank_generation > baseline.bank_generation
            && now.item_id(BOW_STRING_ID) == 0
            && now.bank_item_id(BOW_STRING_ID) >= 1
        {
            self.deposited = Some(now.clone());
        }
        if let Some(deposited) = &self.deposited {
            self.returned |= !now.bank_open
                && !now.bank_loaded
                && now.bank_generation > deposited.bank_generation
                && near(now.tile, FLAX_FIELD, 12);
        }
        if self.returned {
            self.further |=
                !now.bank_open && now.item_id(FLAX_ID) >= 1 && now.item_id(BALL_OF_WOOL_ID) == 0;
        }
    }

    pub fn qualified(&self) -> bool {
        self.further
            && self.picked.is_some()
            && self.produced.is_some()
            && self.deposited.is_some()
            && !self.wrong_product
            && !self.noted
    }
}

/// Full pack of exact flax 1779, deposit, return, further pick. Bow string 1777
/// must never qualify this pick-only cell.
#[derive(Debug, Clone, Default, Serialize)]
pub struct FlaxAioPickCycle {
    pub first_pack: bool,
    pub deposited: Option<Observation>,
    pub returned: bool,
    pub further: bool,
    pub wrong_product: bool,
    pub noted: bool,
}

impl FlaxAioPickCycle {
    pub fn observe(&mut self, baseline: &Observation, now: &Observation) {
        self.wrong_product |= now.item_id(BOW_STRING_ID) > 0
            || now.bank_item_id(BOW_STRING_ID) > 0
            || now.item_id(BALL_OF_WOOL_ID) > 0
            || now.bank_item_id(BALL_OF_WOOL_ID) > 0;
        self.noted |= flax_spinner_noted(now);
        self.first_pack |= now.item_id(FLAX_ID) >= 28
            && baseline.item_id(FLAX_ID) == 0
            && now.item_id(FLAX_ID) > baseline.item_id(FLAX_ID)
            && !self.wrong_product;
        if self.first_pack
            && self.deposited.is_none()
            && now.bank_open
            && now.bank_loaded
            && now.bank_generation > baseline.bank_generation
            && now.item_id(FLAX_ID) == 0
            && now.bank_item_id(FLAX_ID) >= 28
        {
            self.deposited = Some(now.clone());
        }
        if let Some(deposited) = &self.deposited {
            self.returned |= !now.bank_open
                && !now.bank_loaded
                && now.bank_generation > deposited.bank_generation
                && near(now.tile, FLAX_FIELD, 12);
        }
        if self.returned {
            self.further |= !now.bank_open && now.item_id(FLAX_ID) >= 1 && !self.wrong_product;
        }
    }

    pub fn qualified(&self) -> bool {
        self.further && !self.wrong_product && !self.noted
    }
}
