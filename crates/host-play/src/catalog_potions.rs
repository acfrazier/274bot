use super::*;
/// Fill at the fountain, deposit produced water vials, restock empties, fill again.
#[derive(Debug, Clone, Default, Serialize)]
pub struct VialFillerCycle {
    pub filled: Option<Observation>,
    pub deposited: Option<Observation>,
    pub withdrawn: Option<Observation>,
    pub returned: bool,
    pub further: bool,
}

impl VialFillerCycle {
    pub fn observe(&mut self, baseline: &Observation, now: &Observation) {
        if self.filled.is_none()
            && now.item_id(VIAL_OF_WATER_ID) >= 1
            && baseline.item_id(VIAL_OF_WATER_ID) == 0
            && near(now.tile, FALADOR_FOUNTAIN, 4)
        {
            self.filled = Some(now.clone());
        }
        if self.filled.is_some()
            && self.deposited.is_none()
            && now.bank_open
            && now.bank_loaded
            && now.bank_generation > baseline.bank_generation
            && now.item_id(VIAL_OF_WATER_ID) == 0
            && now.bank_item_id(VIAL_OF_WATER_ID) >= 1
        {
            self.deposited = Some(now.clone());
        }
        if let Some(deposited) = &self.deposited {
            if self.withdrawn.is_none()
                && now.bank_open
                && now.bank_loaded
                && now.bank_generation == deposited.bank_generation
                && now.item_id(EMPTY_VIAL_ID) >= 1
                && now.bank_item_id(EMPTY_VIAL_ID) < deposited.bank_item_id(EMPTY_VIAL_ID)
            {
                self.withdrawn = Some(now.clone());
            }
        }
        if let Some(deposited) = &self.deposited {
            self.returned |= self.withdrawn.is_some()
                && !now.bank_open
                && !now.bank_loaded
                // Closing the modal advances the bank session generation.
                && now.bank_generation > deposited.bank_generation
                && near(now.tile, FALADOR_FOUNTAIN, 4);
        }
        if self.returned {
            self.further |= !now.bank_open && now.item_id(VIAL_OF_WATER_ID) >= 1;
        }
    }

    pub fn qualified(&self) -> bool {
        self.further
    }
}

/// Staged herb+water to unfinished, secondary to finished, deposit, restock, further product.
#[derive(Debug, Clone, Default, Serialize)]
pub struct PotionMakerCycle {
    pub unfinished: Option<Observation>,
    pub finished: Option<Observation>,
    pub deposited: Option<Observation>,
    pub withdrawn: Option<Observation>,
    pub further: bool,
    pub wrong_product: bool,
    pub filter_violated: bool,
}

pub struct PotionMakerSpec {
    pub herb: i32,
    pub unf: i32,
    pub secondary: i32,
    pub finished: i32,
    pub wrong_unf: i32,
    pub wrong_finished: i32,
    pub named: bool,
}

impl PotionMakerCycle {
    pub fn observe(&mut self, spec: PotionMakerSpec, baseline: &Observation, now: &Observation) {
        let PotionMakerSpec {
            herb,
            unf,
            secondary,
            finished,
            wrong_unf,
            wrong_finished,
            named,
        } = spec;
        self.wrong_product |= now.item_id(wrong_unf) > 0
            || now.item_id(wrong_finished) > 0
            || now.bank_item_id(wrong_unf) > 0
            || now.bank_item_id(wrong_finished) > 0;
        if named
            && (now.item_id(GUAM_LEAF_ID) > 0
                || (now.bank_open
                    && now.bank_loaded
                    && now.bank_generation > baseline.bank_generation
                    && now.bank_item_id(GUAM_LEAF_ID) < 14))
        {
            self.filter_violated = true;
        }
        if self.unfinished.is_none()
            && now.item_id(unf) >= 1
            && now.item_id(herb) < 14
            && now.item_id(VIAL_OF_WATER_ID) < 14
            && now.item_id(finished) == 0
            && baseline.item_id(unf) == 0
            && baseline.item_id(finished) == 0
        {
            self.unfinished = Some(now.clone());
        }
        if let Some(unfinished) = &self.unfinished {
            if self.finished.is_none()
                && now.item_id(finished) >= 1
                && now.item_id(unf) < unfinished.item_id(unf)
                && now.item_id(secondary) < 14
                && now.skill_xp("herblore") > baseline.skill_xp("herblore")
                && !now.bank_open
            {
                self.finished = Some(now.clone());
            }
        }
        if self.finished.is_some()
            && self.deposited.is_none()
            && now.bank_open
            && now.bank_loaded
            && now.bank_generation > baseline.bank_generation
            && now.item_id(finished) == 0
            && now.bank_item_id(finished) >= 1
        {
            self.deposited = Some(now.clone());
        }
        if let Some(deposited) = &self.deposited {
            if self.withdrawn.is_none()
                && now.bank_open
                && now.bank_loaded
                && now.bank_generation == deposited.bank_generation
                && now.item_id(herb) >= 1
                && now.item_id(VIAL_OF_WATER_ID) >= 1
                && now.bank_item_id(herb) < deposited.bank_item_id(herb)
                && now.bank_item_id(VIAL_OF_WATER_ID) < deposited.bank_item_id(VIAL_OF_WATER_ID)
                && (!named || now.bank_item_id(GUAM_LEAF_ID) == 14)
            {
                self.withdrawn = Some(now.clone());
            }
        }
        if let Some(withdrawn) = &self.withdrawn {
            self.further |= now.item_id(unf) >= 1
                && now.item_id(herb) < withdrawn.item_id(herb)
                && now.item_id(VIAL_OF_WATER_ID) < withdrawn.item_id(VIAL_OF_WATER_ID);
        }
    }

    pub fn qualified(&self) -> bool {
        self.further && self.finished.is_some() && !self.wrong_product && !self.filter_violated
    }
}