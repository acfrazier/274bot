use super::*;

/// Cut a chisel-kept pack, deposit except chisel, restock, then cut again.
#[derive(Debug, Clone, Default, Serialize)]
pub struct GemCutterCycle {
    pub first_pack: bool,
    pub deposited: Option<Observation>,
    pub withdrawn: Option<Observation>,
    pub cut_after_withdrawal: bool,
    pub filter_violated: bool,
}

impl GemCutterCycle {
    pub fn observe(&mut self, named: bool, baseline: &Observation, now: &Observation) {
        if now.item_id(catalog_item_id("crushed_gemstone")) > 0
            || now.bank_item_id(catalog_item_id("crushed_gemstone")) > 0
        {
            self.filter_violated = true;
        }
        if named
            && (now.item_id(catalog_item_id("uncut_opal")) > 0
                || (now.bank_open
                    && now.bank_loaded
                    && now.bank_item_id(catalog_item_id("uncut_opal")) < 4
                    && now.bank_generation > baseline.bank_generation))
        {
            self.filter_violated = true;
        }
        self.first_pack |= now.item_id(catalog_item_id("sapphire")) >= 27
            && now.item_id(catalog_item_id("uncut_sapphire")) == 0
            && now.item_id(catalog_item_id("chisel")) == 1
            && now.skill_xp("crafting") > baseline.skill_xp("crafting")
            && baseline.item_id(catalog_item_id("sapphire")) == 0;
        if self.first_pack
            && self.deposited.is_none()
            && now.bank_open
            && now.bank_loaded
            && now.bank_generation > baseline.bank_generation
            && now.item_id(catalog_item_id("sapphire")) == 0
            && now.item_id(catalog_item_id("chisel")) == 1
            && now.bank_item_id(catalog_item_id("sapphire")) == 27
        {
            self.deposited = Some(now.clone());
        }
        if let Some(deposited) = &self.deposited {
            if self.withdrawn.is_none()
                && now.bank_open
                && now.bank_loaded
                && now.bank_generation == deposited.bank_generation
                && now.item_id(catalog_item_id("uncut_sapphire")) >= 1
                && now.item_id(catalog_item_id("chisel")) == 1
                && now.bank_item_id(catalog_item_id("uncut_sapphire"))
                    < deposited.bank_item_id(catalog_item_id("uncut_sapphire"))
                && (!named || now.bank_item_id(catalog_item_id("uncut_opal")) == 4)
            {
                self.withdrawn = Some(now.clone());
            }
        }
        if let Some(withdrawn) = &self.withdrawn {
            self.cut_after_withdrawal |= !now.bank_open
                && !now.bank_loaded
                && now.item_id(catalog_item_id("sapphire")) > 0
                && now.item_id(catalog_item_id("chisel")) == 1
                && now.item_id(catalog_item_id("uncut_sapphire"))
                    < withdrawn.item_id(catalog_item_id("uncut_sapphire"))
                && now.skill_xp("crafting") > withdrawn.skill_xp("crafting");
        }
    }

    pub fn qualified(&self) -> bool {
        self.cut_after_withdrawal && !self.filter_violated
    }
}
