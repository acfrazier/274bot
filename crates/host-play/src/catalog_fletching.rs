use super::*;

/// A first product or an empty-stock stop cannot qualify the bank loop.
#[derive(Debug, Clone, Default, Serialize)]
pub struct BankFletcherCycle {
    pub first_pack_created: bool,
    pub deposited: Option<Observation>,
    pub withdrawn: Option<Observation>,
    pub crafted_after_withdrawal: bool,
}

impl BankFletcherCycle {
    pub fn observe(&mut self, baseline: &Observation, now: &Observation) {
        self.first_pack_created |= now.item("Willow logs") == 0
            && now.item("Willow shortbow") == 27
            && now.skill_xp("fletching") > baseline.skill_xp("fletching");
        if self.first_pack_created
            && self.deposited.is_none()
            && now.bank_open
            && now.bank_loaded
            && now.bank_generation > baseline.bank_generation
            && now.item("Knife") == 1
            && now.item("Willow logs") == 0
            && now.item("Willow shortbow") == 0
            && now.bank_item("Willow shortbow") == 27
            && now.bank_item("Willow logs") == 54
        {
            self.deposited = Some(now.clone());
        }
        if let Some(deposited) = &self.deposited {
            if self.withdrawn.is_none()
                && now.bank_open
                && now.bank_loaded
                && now.bank_generation == deposited.bank_generation
                && now.item("Knife") == 1
                && now.item("Willow logs") == 27
                && now.item("Willow shortbow") == 0
                && now.bank_item("Willow logs") == 27
                && now.bank_item("Willow shortbow") == 27
            {
                self.withdrawn = Some(now.clone());
            }
        }
        if let Some(withdrawn) = &self.withdrawn {
            self.crafted_after_withdrawal |= !now.bank_open
                && !now.bank_loaded
                && now.item("Willow logs") < withdrawn.item("Willow logs")
                && now.item("Willow shortbow") > 0
                && now.skill_xp("fletching") > withdrawn.skill_xp("fletching");
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct BankFletcherOptionSpec {
    pub primary: i32,
    pub secondary: Option<i32>,
    pub product: i32,
    pub first_product_count: i32,
}

pub fn bank_fletcher_option_spec(case: CoreCase) -> Option<BankFletcherOptionSpec> {
    match case {
        CoreCase::BankFletcherShafts => Some(BankFletcherOptionSpec {
            primary: LOGS_ID,
            secondary: None,
            product: ARROW_SHAFT_ID,
            first_product_count: 405,
        }),
        CoreCase::BankFletcherHeadless => Some(BankFletcherOptionSpec {
            primary: FEATHER_ID,
            secondary: Some(ARROW_SHAFT_ID),
            product: HEADLESS_ARROW_ID,
            first_product_count: 30,
        }),
        _ => None,
    }
}

/// Exact option output, fresh-bank deposit/restock, closed return, and further
/// production. A seeded product or a first batch alone cannot qualify.
#[derive(Debug, Clone, Default, Serialize)]
pub struct BankFletcherOptionCycle {
    pub first: Option<Observation>,
    pub deposited: Option<Observation>,
    pub restocked: Option<Observation>,
    pub further: bool,
}

impl BankFletcherOptionCycle {
    pub fn observe(
        &mut self,
        spec: BankFletcherOptionSpec,
        baseline: &Observation,
        now: &Observation,
    ) {
        if self.first.is_none()
            && now.item_id(spec.product) >= spec.first_product_count
            && now.item_id(spec.primary) < baseline.item_id(spec.primary)
            && spec
                .secondary
                .is_none_or(|id| now.item_id(id) < baseline.item_id(id))
            && now.skill_xp("fletching") > baseline.skill_xp("fletching")
        {
            self.first = Some(now.clone());
        }
        if self.first.is_some()
            && self.deposited.is_none()
            && now.bank_open
            && now.bank_loaded
            && now.bank_generation > baseline.bank_generation
            && now.item_id(spec.product) == 0
            && now.bank_item_id(spec.product) >= spec.first_product_count
        {
            self.deposited = Some(now.clone());
        }
        if let Some(deposited) = &self.deposited {
            if self.restocked.is_none()
                && now.bank_open
                && now.bank_loaded
                && now.bank_generation == deposited.bank_generation
                && now.item_id(spec.primary) > 0
                && now.bank_item_id(spec.primary) < deposited.bank_item_id(spec.primary)
                && spec.secondary.is_none_or(|id| {
                    now.item_id(id) > 0 && now.bank_item_id(id) < deposited.bank_item_id(id)
                })
            {
                self.restocked = Some(now.clone());
            }
        }
        if let Some(restocked) = &self.restocked {
            self.further |= !now.bank_open
                && !now.bank_loaded
                && now.item_id(spec.product) >= 1
                && now.item_id(spec.primary) < restocked.item_id(spec.primary)
                && spec
                    .secondary
                    .is_none_or(|id| now.item_id(id) < restocked.item_id(id))
                && now.skill_xp("fletching") > restocked.skill_xp("fletching");
        }
    }

    pub fn qualified(&self) -> bool {
        self.further && self.first.is_some() && self.deposited.is_some() && self.restocked.is_some()
    }
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct BankFletcherStringCycle {
    pub initial_pairs_strung: bool,
    pub deposited: Option<Observation>,
    pub withdrawn: Option<Observation>,
    pub strung_after_withdrawal: bool,
}

impl BankFletcherStringCycle {
    pub fn observe(&mut self, baseline: &Observation, now: &Observation) {
        self.initial_pairs_strung |= now.item_id(60) == 0
            && now.item_id(1777) == 0
            && now.item_id(849) == 2
            && now.skill_xp("fletching") - baseline.skill_xp("fletching") >= 66;
        if self.initial_pairs_strung
            && self.deposited.is_none()
            && now.bank_open
            && now.bank_loaded
            && now.bank_generation > baseline.bank_generation
            && now.item_id(849) == 0
            && now.bank_item_id(849) == 2
            && now.bank_item_id(60) == 28
            && now.bank_item_id(1777) == 28
        {
            self.deposited = Some(now.clone());
        }
        if let Some(deposited) = &self.deposited {
            if self.withdrawn.is_none()
                && now.bank_open
                && now.bank_loaded
                && now.bank_generation == deposited.bank_generation
                && now.item_id(60) == 14
                && now.item_id(1777) == 14
                && now.bank_item_id(60) == 14
                && now.bank_item_id(1777) == 14
                && now.bank_item_id(849) == 2
            {
                self.withdrawn = Some(now.clone());
            }
        }
        if let Some(withdrawn) = &self.withdrawn {
            self.strung_after_withdrawal |= !now.bank_open
                && !now.bank_loaded
                && now.item_id(60) < withdrawn.item_id(60)
                && now.item_id(1777) < withdrawn.item_id(1777)
                && now.item_id(849) > 0
                && now.skill_xp("fletching") > withdrawn.skill_xp("fletching");
        }
    }
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct BankFletcherCutStringCycle {
    pub cut_pair_created: bool,
    pub deposited: Option<Observation>,
    pub withdrawn: Option<Observation>,
    pub string_pair_created: bool,
}

impl BankFletcherCutStringCycle {
    pub fn observe(&mut self, baseline: &Observation, now: &Observation) {
        self.cut_pair_created |= now.item_id(1519) == 0
            && now.item_id(60) == 2
            && now.item_id(849) == 0
            && now.skill_xp("fletching") - baseline.skill_xp("fletching") >= 66;
        if self.cut_pair_created
            && self.deposited.is_none()
            && now.bank_open
            && now.bank_loaded
            && now.bank_generation > baseline.bank_generation
            && now.item_id(60) == 0
            && now.bank_item_id(60) == 2
            && now.bank_item_id(1777) == 28
            && now.bank_item_id(849) == 0
        {
            self.deposited = Some(now.clone());
        }
        if let Some(deposited) = &self.deposited {
            if self.withdrawn.is_none()
                && now.bank_open
                && now.bank_loaded
                && now.bank_generation == deposited.bank_generation
                && now.item("Knife") == 0
                && now.bank_item("Knife") >= 1
                && now.item_id(60) == 2
                && now.item_id(1777) == 14
                && now.bank_item_id(60) == 0
                && now.bank_item_id(1777) == 14
            {
                self.withdrawn = Some(now.clone());
            }
        }
        if let Some(withdrawn) = &self.withdrawn {
            self.string_pair_created |= !now.bank_open
                && !now.bank_loaded
                && now.item_id(60) == 0
                && now.item_id(849) == 2
                && now.item_id(1777) == withdrawn.item_id(1777) - 2
                && now.skill_xp("fletching") > withdrawn.skill_xp("fletching");
        }
    }
}

/// Two observed dart actions: exact product id, both inputs down, XP, no wrong tier.
#[derive(Debug, Clone, Default, Serialize)]
pub struct DartFletcherCycle {
    pub first: Option<Observation>,
    pub further: bool,
    pub wrong_tier: bool,
}

impl DartFletcherCycle {
    pub fn observe(
        &mut self,
        tips: i32,
        feathers: i32,
        product: i32,
        wrong: i32,
        baseline: &Observation,
        now: &Observation,
    ) {
        self.wrong_tier |= now.item_id(wrong) > 0;
        if self.first.is_none()
            && now.item_id(product) >= 10
            && now.item_id(tips) < baseline.item_id(tips)
            && now.item_id(feathers) < baseline.item_id(feathers)
            && now.skill_xp("fletching") > baseline.skill_xp("fletching")
            && now.item_id(wrong) == 0
        {
            self.first = Some(now.clone());
        }
        if let Some(first) = &self.first {
            self.further |= now.item_id(product) > first.item_id(product)
                && now.item_id(tips) < first.item_id(tips)
                && now.item_id(feathers) < first.item_id(feathers)
                && now.skill_xp("fletching") > first.skill_xp("fletching")
                && now.item_id(wrong) == 0;
        }
    }

    pub fn qualified(&self) -> bool {
        self.further && !self.wrong_tier
    }
}
