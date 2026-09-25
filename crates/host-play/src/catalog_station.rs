use super::*;
pub fn cook_wrong_or_burnt(observation: &Observation, wrong: i32) -> bool {
    observation.item_id(wrong) > 0
        || observation.bank_item_id(wrong) > 0
        || observation.item_id(BURNT_FISH_1_ID) > 0
        || observation.bank_item_id(BURNT_FISH_1_ID) > 0
        || observation.item_id(BURNT_FISH_2_ID) > 0
        || observation.bank_item_id(BURNT_FISH_2_ID) > 0
        || observation.item_id(BURNT_LOBSTER_ID) > 0
        || observation.bank_item_id(BURNT_LOBSTER_ID) > 0
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
        && baseline.item_id(IRON_BAR_ID) == 0
        && !smelter_noted(baseline, primary + 1, secondary + 1, product + 1)
}

pub fn flax_spinner_noted(observation: &Observation) -> bool {
    observation.item_id(NOTED_FLAX_ID) > 0
        || observation.bank_item_id(NOTED_FLAX_ID) > 0
        || observation.item_id(NOTED_BOW_STRING_ID) > 0
        || observation.bank_item_id(NOTED_BOW_STRING_ID) > 0
}

pub fn flax_spinner_baseline_ready(baseline: &Observation) -> bool {
    near(baseline.tile, FLAX_SPINNER_BANK, 8)
        && baseline.level("crafting") >= 1
        && baseline.item_id(FLAX_ID) == 0
        && baseline.item_id(BOW_STRING_ID) == 0
        && baseline.item_id(BALL_OF_WOOL_ID) == 0
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
            product: SALMON_ID,
            input: RAW_SALMON_ID,
            extra_input: None,
            wrong: LOBSTER_ID,
            extra_wrong: None,
            skill: "cooking",
            station: CATHERBY_RANGE_STAND,
            station_radius: 8,
            noted: cook_noted(observation, NOTED_RAW_SALMON_ID, NOTED_SALMON_ID)
                || cook_wrong_or_burnt(observation, LOBSTER_ID),
        }),
        CoreCase::CookBotLobster => Some(StationProductionSpec {
            product: LOBSTER_ID,
            input: RAW_LOBSTER_ID,
            extra_input: None,
            wrong: SALMON_ID,
            extra_wrong: None,
            skill: "cooking",
            station: CATHERBY_RANGE_STAND,
            station_radius: 8,
            noted: cook_noted(observation, NOTED_RAW_LOBSTER_ID, NOTED_LOBSTER_ID)
                || cook_wrong_or_burnt(observation, SALMON_ID),
        }),
        CoreCase::SmelterBot => Some(StationProductionSpec {
            product: BRONZE_BAR_ID,
            input: COPPER_ORE_ID,
            extra_input: Some(TIN_ORE_ID),
            wrong: STEEL_BAR_ID,
            extra_wrong: Some(IRON_BAR_ID),
            skill: "smithing",
            station: AL_KHARID_FURNACE,
            station_radius: 8,
            noted: smelter_noted(
                observation,
                NOTED_COPPER_ORE_ID,
                NOTED_TIN_ORE_ID,
                NOTED_BRONZE_BAR_ID,
            ),
        }),
        CoreCase::SmelterBotSteel => Some(StationProductionSpec {
            product: STEEL_BAR_ID,
            input: IRON_ORE_ID,
            extra_input: Some(COAL_ID),
            wrong: BRONZE_BAR_ID,
            extra_wrong: Some(IRON_BAR_ID),
            skill: "smithing",
            station: AL_KHARID_FURNACE,
            station_radius: 8,
            noted: smelter_noted(
                observation,
                NOTED_IRON_ORE_ID,
                NOTED_COAL_ID,
                NOTED_STEEL_BAR_ID,
            ),
        }),
        CoreCase::FlaxSpinner => Some(StationProductionSpec {
            product: BOW_STRING_ID,
            input: FLAX_ID,
            extra_input: None,
            wrong: BALL_OF_WOOL_ID,
            extra_wrong: None,
            skill: "crafting",
            station: FLAX_SPINNER_WHEEL,
            station_radius: 8,
            noted: flax_spinner_noted(observation),
        }),
        CoreCase::FlaxAioSpin => Some(StationProductionSpec {
            product: BOW_STRING_ID,
            input: FLAX_ID,
            extra_input: None,
            wrong: BALL_OF_WOOL_ID,
            extra_wrong: None,
            skill: "crafting",
            station: FLAX_SPINNER_WHEEL,
            station_radius: 8,
            noted: flax_spinner_noted(observation),
        }),
        _ => None,
    }
}