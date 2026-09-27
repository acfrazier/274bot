use super::*;
/// Tan at the Tanner widget, deposit produced leather, restock hides, tan again.
#[derive(Debug, Clone, Default, Serialize)]
pub struct TannerBotCycle {
    pub widget: Option<Observation>,
    pub tanned: Option<Observation>,
    pub deposited: Option<Observation>,
    pub withdrawn: Option<Observation>,
    pub returned: bool,
    pub further: bool,
    pub wrong_product: bool,
    pub shop: bool,
}

pub struct TannerBotSpec {
    pub product: i32,
    pub wrong_product: i32,
    pub tan_all: i32,
}

impl TannerBotCycle {
    pub fn observe(&mut self, spec: TannerBotSpec, baseline: &Observation, now: &Observation) {
        let TannerBotSpec {
            product,
            wrong_product,
            tan_all,
        } = spec;
        self.wrong_product |= now.item_id(wrong_product) > 0 || now.bank_item_id(wrong_product) > 0;
        self.shop |= now.main_modal == SHOPMAIN;
        if self.widget.is_none()
            && now.main_modal == TANNER_IF
            && now.has_widget(tan_all)
            && now.item_id(COW_HIDE_ID) >= 1
            && now.item_id(product) == 0
            && now.item_id(COINS_ID) >= 1
            && near(now.tile, TANNER_STAND, 4)
            && now.main_modal != SHOPMAIN
        {
            self.widget = Some(now.clone());
        }
        if let Some(widget) = &self.widget {
            if self.tanned.is_none()
                && near(now.tile, TANNER_STAND, 4)
                && now.item_id(product) >= 1
                && now.item_id(COW_HIDE_ID) == 0
                && now.item_id(COINS_ID) < widget.item_id(COINS_ID)
                && now.item_id(wrong_product) == 0
                && now.main_modal != SHOPMAIN
                && !near(now.tile, DOMMIK_STAND, 4)
            {
                self.tanned = Some(now.clone());
            }
        }
        if self.tanned.is_some()
            && self.deposited.is_none()
            && now.bank_open
            && now.bank_loaded
            && now.bank_generation > baseline.bank_generation
            && now.item_id(product) == 0
            && now.bank_item_id(product) >= 1
        {
            self.deposited = Some(now.clone());
        }
        if let Some(deposited) = &self.deposited {
            if self.withdrawn.is_none()
                && now.bank_open
                && now.bank_loaded
                && now.bank_generation == deposited.bank_generation
                && now.item_id(COW_HIDE_ID) >= 1
                && now.bank_item_id(COW_HIDE_ID) < deposited.bank_item_id(COW_HIDE_ID)
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
                && near(now.tile, TANNER_STAND, 4);
        }
        if self.returned {
            self.further |= !now.bank_open
                && now.item_id(product) >= 1
                && now.item_id(COW_HIDE_ID) == 0
                && now.item_id(wrong_product) == 0;
        }
    }

    pub fn qualified(&self) -> bool {
        self.further && !self.wrong_product && !self.shop && self.tanned.is_some()
    }
}
