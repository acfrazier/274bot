use super::*;
pub fn coal_trucks_baseline_ready(baseline: &Observation) -> bool {
    near(baseline.tile, COAL_MINE, 8)
        && baseline.level("mining") >= 60
        && baseline.effective_level("mining") >= 60
        && baseline.item_id(catalog_item_id("rune_pickaxe")) == 1
        && baseline.item_id(catalog_item_id("knife")) == COAL_BALLAST_KNIVES
        && baseline.item_ids.values().copied().sum::<i32>() == 27
        && baseline.item_id(catalog_item_id("coal")) == 0
        && baseline.item_id(catalog_item_id("cert_coal")) == 0
        && baseline.bank_item_id(catalog_item_id("coal")) == 0
}
/// Mine coal 453 with Mining XP, deposit to the mine truck (pack empty at
/// the truck, bank closed, bank coal unchanged), then mine again. A Seers
/// bank deposit is not the truck proof.
#[derive(Debug, Clone, Default, Serialize)]
pub struct CoalTrucksCycle {
    pub mined: Option<Observation>,
    pub trucked: Option<Observation>,
    pub further: bool,
    pub noted: bool,
    pub banked: bool,
    pub fixture_changed: bool,
}

impl CoalTrucksCycle {
    pub fn observe(&mut self, baseline: &Observation, now: &Observation) {
        self.noted |= now.item_id(catalog_item_id("cert_coal")) > 0
            || now.bank_item_id(catalog_item_id("cert_coal")) > 0;
        self.banked |= now.bank_item_id(catalog_item_id("coal"))
            > baseline.bank_item_id(catalog_item_id("coal"));
        self.fixture_changed |= now.item_id(catalog_item_id("rune_pickaxe")) != 1
            || now.item_id(catalog_item_id("knife")) != COAL_BALLAST_KNIVES;
        if self.mined.is_none()
            && now.item_id(catalog_item_id("coal")) >= 1
            && baseline.item_id(catalog_item_id("coal")) == 0
            && now.skill_xp("mining") > baseline.skill_xp("mining")
            && now.item_id(catalog_item_id("cert_coal")) == 0
        {
            self.mined = Some(now.clone());
        }
        if self.mined.is_some()
            && self.trucked.is_none()
            && now.item_id(catalog_item_id("coal")) == 0
            && !now.bank_open
            && now.bank_item_id(catalog_item_id("coal"))
                == baseline.bank_item_id(catalog_item_id("coal"))
            && near(now.tile, COAL_MINE_TRUCK_STAND, 4)
            && !near(now.tile, SEERS_BANK, 8)
        {
            self.trucked = Some(now.clone());
        }
        if self.trucked.is_some() {
            self.further |= !now.bank_open
                && now.item_id(catalog_item_id("coal")) >= 1
                && now.skill_xp("mining") > baseline.skill_xp("mining");
        }
    }

    pub fn qualified(&self) -> bool {
        self.further
            && self.mined.is_some()
            && self.trucked.is_some()
            && !self.noted
            && !self.banked
            && !self.fixture_changed
    }
}
