use super::*;
pub fn cook_wrong_or_burnt(observation: &Observation, wrong: i32) -> bool {
    observation.item_id(wrong) > 0
        || observation.bank_item_id(wrong) > 0
        || observation.item_id(catalog_item_id("burntfish1")) > 0
        || observation.bank_item_id(catalog_item_id("burntfish1")) > 0
        || observation.item_id(catalog_item_id("burntfish2")) > 0
        || observation.bank_item_id(catalog_item_id("burntfish2")) > 0
        || observation.item_id(catalog_item_id("burnt_lobster")) > 0
        || observation.bank_item_id(catalog_item_id("burnt_lobster")) > 0
}

pub fn cook_noted(observation: &Observation, noted_raw: i32, noted_product: i32) -> bool {
    observation.item_id(noted_raw) > 0
        || observation.bank_item_id(noted_raw) > 0
        || observation.item_id(noted_product) > 0
        || observation.bank_item_id(noted_product) > 0
}

pub fn cook_bot_baseline_ready(
    baseline: &Observation,
    raw: i32,
    product: i32,
    wrong: i32,
    noted_raw: i32,
    noted_product: i32,
) -> bool {
    near(baseline.tile, CATHERBY_BANK, 6)
        && baseline.level("cooking") >= COOKING_FIXTURE_LEVEL
        && baseline.item_id(raw) == 0
        && baseline.item_id(product) == 0
        && !cook_wrong_or_burnt(baseline, wrong)
        && !cook_noted(baseline, noted_raw, noted_product)
}

pub fn smelter_noted(
    observation: &Observation,
    noted_primary: i32,
    noted_secondary: i32,
    noted_product: i32,
) -> bool {
    observation.item_id(noted_primary) > 0
        || observation.bank_item_id(noted_primary) > 0
        || observation.item_id(noted_secondary) > 0
        || observation.bank_item_id(noted_secondary) > 0
        || observation.item_id(noted_product) > 0
        || observation.bank_item_id(noted_product) > 0
}

pub fn smelter_bot_baseline_ready(
    baseline: &Observation,
    primary: i32,
    secondary: i32,
    product: i32,
    wrong: i32,
    smithing: i32,
) -> bool {
    near(baseline.tile, AL_KHARID_BANK, 6)
        && baseline.level("smithing") >= smithing
        && baseline.item_id(primary) == 0
        && baseline.item_id(secondary) == 0
        && baseline.item_id(product) == 0
        && baseline.item_id(wrong) == 0
        && baseline.item_id(catalog_item_id("iron_bar")) == 0
        && !smelter_noted(baseline, primary + 1, secondary + 1, product + 1)
}

pub fn flax_spinner_noted(observation: &Observation) -> bool {
    observation.item_id(catalog_item_id("cert_flax")) > 0
        || observation.bank_item_id(catalog_item_id("cert_flax")) > 0
        || observation.item_id(catalog_item_id("cert_bow_string")) > 0
        || observation.bank_item_id(catalog_item_id("cert_bow_string")) > 0
}

pub fn flax_spinner_baseline_ready(baseline: &Observation) -> bool {
    near(baseline.tile, FLAX_SPINNER_BANK, 8)
        && baseline.level("crafting") >= 1
        && baseline.item_id(catalog_item_id("flax")) == 0
        && baseline.item_id(catalog_item_id("bow_string")) == 0
        && baseline.item_id(catalog_item_id("ball_of_wool")) == 0
        && !flax_spinner_noted(baseline)
}
/// Produce exact unnoted output with relevant XP after Start, deposit into a
/// fresh loaded bank, restock the raw input, close, return to the station,
/// and produce again. Seeded/name-only/noted/wrong products fail.
#[derive(Debug, Clone, Default, Serialize)]
pub struct StationProductionCycle {
    pub withdrawn: Option<Observation>,
    pub produced: Option<Observation>,
    pub deposited: Option<Observation>,
    pub restocked: Option<Observation>,
    pub returned: bool,
    pub further: bool,
    pub wrong_product: bool,
    pub noted: bool,
}

pub struct StationProductionSpec {
    pub product: i32,
    pub input: i32,
    pub extra_input: Option<i32>,
    pub wrong: i32,
    pub extra_wrong: Option<i32>,
    pub skill: &'static str,
    pub station: (i32, i32, i32),
    pub station_radius: i32,
    pub noted: bool,
}

impl StationProductionCycle {
    pub fn observe(
        &mut self,
        spec: StationProductionSpec,
        baseline: &Observation,
        now: &Observation,
    ) {
        let StationProductionSpec {
            product,
            input,
            extra_input,
            wrong,
            extra_wrong,
            skill,
            station,
            station_radius,
            noted,
        } = spec;
        self.wrong_product |= now.item_id(wrong) > 0
            || now.bank_item_id(wrong) > 0
            || extra_wrong.is_some_and(|id| now.item_id(id) > 0 || now.bank_item_id(id) > 0);
        self.noted |= noted;
        if self.withdrawn.is_none()
            && now.item_id(input) >= 1
            && now.item_id(product) == 0
            && baseline.item_id(input) == 0
            && extra_input.is_none_or(|id| now.item_id(id) >= 1)
            && now.item_id(wrong) == 0
            && extra_wrong.is_none_or(|id| now.item_id(id) == 0)
            && !noted
        {
            self.withdrawn = Some(now.clone());
        }
        if let Some(withdrawn) = &self.withdrawn {
            if self.produced.is_none()
                && now.item_id(product) >= 1
                && now.item_id(input) < withdrawn.item_id(input)
                && extra_input.is_none_or(|id| now.item_id(id) < withdrawn.item_id(id))
                && now.skill_xp(skill) > baseline.skill_xp(skill)
                && near(now.tile, station, station_radius)
                && now.item_id(wrong) == 0
                && extra_wrong.is_none_or(|id| now.item_id(id) == 0)
                && !noted
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
                && extra_input.is_none_or(|id| now.item_id(id) >= 1)
            {
                self.restocked = Some(now.clone());
            }
        }
        if let Some(deposited) = &self.deposited {
            self.returned |= self.restocked.is_some()
                && !now.bank_open
                && !now.bank_loaded
                && now.bank_generation > deposited.bank_generation
                && near(now.tile, station, station_radius);
        }
        if let Some(produced) = &self.produced {
            self.further |= self.returned
                && !now.bank_open
                && now.item_id(product) >= 1
                && now.item_id(wrong) == 0
                && extra_wrong.is_none_or(|id| now.item_id(id) == 0)
                && now.skill_xp(skill) > produced.skill_xp(skill);
        }
    }

    pub fn qualified(&self) -> bool {
        self.further
            && self.withdrawn.is_some()
            && self.produced.is_some()
            && self.deposited.is_some()
            && self.restocked.is_some()
            && !self.wrong_product
            && !self.noted
    }
}

pub fn station_production_spec(
    case: CoreCase,
    observation: &Observation,
) -> Option<StationProductionSpec> {
    match case {
        CoreCase::CookBot => Some(StationProductionSpec {
            product: catalog_item_id("salmon"),
            input: catalog_item_id("raw_salmon"),
            extra_input: None,
            wrong: catalog_item_id("lobster"),
            extra_wrong: None,
            skill: "cooking",
            station: CATHERBY_RANGE_STAND,
            station_radius: 8,
            noted: cook_noted(
                observation,
                catalog_item_id("cert_raw_salmon"),
                catalog_item_id("cert_salmon"),
            ) || cook_wrong_or_burnt(observation, catalog_item_id("lobster")),
        }),
        CoreCase::CookBotLobster => Some(StationProductionSpec {
            product: catalog_item_id("lobster"),
            input: catalog_item_id("raw_lobster"),
            extra_input: None,
            wrong: catalog_item_id("salmon"),
            extra_wrong: None,
            skill: "cooking",
            station: CATHERBY_RANGE_STAND,
            station_radius: 8,
            noted: cook_noted(
                observation,
                catalog_item_id("cert_raw_lobster"),
                catalog_item_id("cert_lobster"),
            ) || cook_wrong_or_burnt(observation, catalog_item_id("salmon")),
        }),
        CoreCase::SmelterBot => Some(StationProductionSpec {
            product: catalog_item_id("bronze_bar"),
            input: catalog_item_id("copper_ore"),
            extra_input: Some(catalog_item_id("tin_ore")),
            wrong: catalog_item_id("steel_bar"),
            extra_wrong: Some(catalog_item_id("iron_bar")),
            skill: "smithing",
            station: AL_KHARID_FURNACE,
            station_radius: 8,
            noted: smelter_noted(
                observation,
                catalog_item_id("cert_copper_ore"),
                catalog_item_id("cert_tin_ore"),
                catalog_item_id("cert_bronze_bar"),
            ),
        }),
        CoreCase::SmelterBotSteel => Some(StationProductionSpec {
            product: catalog_item_id("steel_bar"),
            input: catalog_item_id("iron_ore"),
            extra_input: Some(catalog_item_id("coal")),
            wrong: catalog_item_id("bronze_bar"),
            extra_wrong: Some(catalog_item_id("iron_bar")),
            skill: "smithing",
            station: AL_KHARID_FURNACE,
            station_radius: 8,
            noted: smelter_noted(
                observation,
                catalog_item_id("cert_iron_ore"),
                catalog_item_id("cert_coal"),
                catalog_item_id("cert_steel_bar"),
            ),
        }),
        CoreCase::FlaxSpinner => Some(StationProductionSpec {
            product: catalog_item_id("bow_string"),
            input: catalog_item_id("flax"),
            extra_input: None,
            wrong: catalog_item_id("ball_of_wool"),
            extra_wrong: None,
            skill: "crafting",
            station: FLAX_SPINNER_WHEEL,
            station_radius: 8,
            noted: flax_spinner_noted(observation),
        }),
        CoreCase::FlaxAioSpin => Some(StationProductionSpec {
            product: catalog_item_id("bow_string"),
            input: catalog_item_id("flax"),
            extra_input: None,
            wrong: catalog_item_id("ball_of_wool"),
            extra_wrong: None,
            skill: "crafting",
            station: FLAX_SPINNER_WHEEL,
            station_radius: 8,
            noted: flax_spinner_noted(observation),
        }),
        _ => None,
    }
}
