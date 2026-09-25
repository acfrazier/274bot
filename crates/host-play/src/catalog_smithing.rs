use super::*;
/// Anvil main-panel production (not chat make), then deposit except hammer,
/// restock bars, and further smithing.
#[derive(Debug, Clone, Default, Serialize)]
pub struct SmithingBotCycle {
    pub panel: Option<Observation>,
    pub produced: Option<Observation>,
    pub deposited: Option<Observation>,
    pub restocked: Option<Observation>,
    pub returned: bool,
    pub further: bool,
    pub wrong_product: bool,
}

pub struct SmithingBotSpec {
    pub product: i32,
    pub wrong: i32,
    pub bars_per: i32,
}

pub fn smithing_bot_spec(case: CoreCase) -> Option<SmithingBotSpec> {
    match case {
        CoreCase::SmithingBot => Some(SmithingBotSpec {
            product: BRONZE_DAGGER_ID,
            wrong: BRONZE_PLATEBODY_ID,
            bars_per: 1,
        }),
        CoreCase::SmithingBotPlatebody => Some(SmithingBotSpec {
            product: BRONZE_PLATEBODY_ID,
            wrong: BRONZE_DAGGER_ID,
            bars_per: 5,
        }),
        _ => None,
    }
}

impl SmithingBotCycle {
    pub fn observe(&mut self, spec: SmithingBotSpec, baseline: &Observation, now: &Observation) {
        let SmithingBotSpec {
            product,
            wrong,
            bars_per,
        } = spec;
        self.wrong_product |= now.item_id(wrong) > 0 || now.bank_item_id(wrong) > 0;
        if self.panel.is_none()
            && now.has_main_make(product)
            && near(now.tile, VARROCK_ANVIL, 8)
            && now.item_id(BRONZE_BAR_ID) >= bars_per
            && now.item_id(product) == 0
            && now.item_id(HAMMER_ID) >= 1
        {
            self.panel = Some(now.clone());
        }
        if let Some(panel) = &self.panel {
            if self.produced.is_none()
                && near(now.tile, VARROCK_ANVIL, 8)
                && now.item_id(product) >= 1
                && now.item_id(BRONZE_BAR_ID) <= panel.item_id(BRONZE_BAR_ID) - bars_per
                && now.skill_xp("smithing") > baseline.skill_xp("smithing")
                && now.item_id(wrong) == 0
            {
                self.produced = Some(now.clone());
            }
        }
        if self.produced.is_some()
            && self.deposited.is_none()
            && now.bank_open
            && now.bank_loaded
            && now.bank_generation > baseline.bank_generation
            && now.item_id(product) == 0
            && now.bank_item_id(product) >= 1
            && now.item_id(HAMMER_ID) >= 1
        {
            self.deposited = Some(now.clone());
        }
        if let Some(deposited) = &self.deposited {
            if self.restocked.is_none()
                && now.bank_open
                && now.bank_loaded
                && now.bank_generation == deposited.bank_generation
                && now.item_id(BRONZE_BAR_ID) >= bars_per
                && now.bank_item_id(BRONZE_BAR_ID) < deposited.bank_item_id(BRONZE_BAR_ID)
            {
                self.restocked = Some(now.clone());
            }
        }
        if let Some(deposited) = &self.deposited {
            self.returned |= self.restocked.is_some()
                && !now.bank_open
                && !now.bank_loaded
                && now.bank_generation > deposited.bank_generation
                && near(now.tile, VARROCK_ANVIL, 8);
        }
        if let Some(produced) = &self.produced {
            self.further |= self.returned
                && !now.bank_open
                && now.item_id(product) >= 1
                && now.skill_xp("smithing") > produced.skill_xp("smithing")
                && now.item_id(wrong) == 0;
        }
    }

    pub fn qualified(&self) -> bool {
        self.further
            && self.panel.is_some()
            && self.produced.is_some()
            && self.deposited.is_some()
            && self.restocked.is_some()
            && !self.wrong_product
    }
}