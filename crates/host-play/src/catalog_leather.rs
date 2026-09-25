use super::*;
/// Soft-leather gloves wait the selected leather interface; hard-leather
/// body is the no-modal `flow=single` burst.
#[derive(Debug, Clone, Default, Serialize)]
pub struct LeatherCrafterCycle {
    pub interface: Option<Observation>,
    pub withdrawn: Option<Observation>,
    pub produced: Option<Observation>,
    pub deposited: Option<Observation>,
    pub restocked: Option<Observation>,
    pub returned: bool,
    pub further: bool,
    pub used_interface: bool,
    pub wrong_product: bool,
}

pub struct LeatherCrafterSpec {
    pub product: i32,
    pub input: i32,
    pub wrong: i32,
    pub require_interface: bool,
}

pub fn leather_crafter_spec(case: CoreCase) -> Option<LeatherCrafterSpec> {
    match case {
        CoreCase::LeatherCrafter => Some(LeatherCrafterSpec {
            product: LEATHER_GLOVES_ID,
            input: SOFT_LEATHER_ID,
            wrong: HARDLEATHER_BODY_ID,
            require_interface: true,
        }),
        CoreCase::LeatherCrafterHardBody => Some(LeatherCrafterSpec {
            product: HARDLEATHER_BODY_ID,
            input: HARD_LEATHER_ID,
            wrong: LEATHER_GLOVES_ID,
            require_interface: false,
        }),
        _ => None,
    }
}

impl LeatherCrafterCycle {
    pub fn observe(&mut self, spec: LeatherCrafterSpec, baseline: &Observation, now: &Observation) {
        let LeatherCrafterSpec {
            product,
            input,
            wrong,
            require_interface,
        } = spec;
        self.wrong_product |= now.item_id(wrong) > 0 || now.bank_item_id(wrong) > 0;
        if now.main_modal == LEATHER_IF && now.has_widget(LEATHER_GLOVES_MAKE10) {
            self.used_interface = true;
            if self.interface.is_none()
                && now.item_id(input) >= 1
                && now.item_id(product) == 0
                && now.item_id(NEEDLE_ID) >= 1
            {
                self.interface = Some(now.clone());
            }
        }
        if self.withdrawn.is_none()
            && now.item_id(input) >= 1
            && now.item_id(product) == 0
            && now.item_id(NEEDLE_ID) >= 1
            && baseline.item_id(input) == 0
        {
            self.withdrawn = Some(now.clone());
        }
        let prior = if require_interface {
            self.interface.as_ref()
        } else {
            self.withdrawn.as_ref().filter(|_| !self.used_interface)
        };
        if let Some(prior) = prior {
            if self.produced.is_none()
                && now.item_id(product) >= 1
                && now.item_id(input) < prior.item_id(input)
                && now.skill_xp("crafting") > baseline.skill_xp("crafting")
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
            && now.item_id(NEEDLE_ID) >= 1
        {
            self.deposited = Some(now.clone());
        }
        if let Some(deposited) = &self.deposited {
            if self.restocked.is_none()
                && now.bank_open
                && now.bank_loaded
                && now.bank_generation == deposited.bank_generation
                && now.item_id(input) >= 1
                && now.bank_item_id(input) < deposited.bank_item_id(input)
            {
                self.restocked = Some(now.clone());
            }
        }
        if let Some(deposited) = &self.deposited {
            self.returned |= self.restocked.is_some()
                && !now.bank_open
                && !now.bank_loaded
                && now.bank_generation > deposited.bank_generation;
        }
        if let Some(produced) = &self.produced {
            self.further |= self.returned
                && !now.bank_open
                && now.item_id(product) >= 1
                && now.skill_xp("crafting") > produced.skill_xp("crafting")
                && now.item_id(wrong) == 0;
        }
    }

    pub fn qualified(&self) -> bool {
        self.further
            && self.produced.is_some()
            && self.deposited.is_some()
            && self.restocked.is_some()
            && !self.wrong_product
    }
}
