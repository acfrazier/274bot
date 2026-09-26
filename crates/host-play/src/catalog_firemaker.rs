use super::*;
pub fn fire_in_varrock_east_plot(observation: &Observation) -> bool {
    observation.loc_facts.iter().any(|loc| {
        loc.level == 0
            && loc.x >= FIRE_PLOT_VARROCK_EAST_X0
            && loc.x <= FIRE_PLOT_VARROCK_EAST_X1
            && loc.z >= FIRE_PLOT_VARROCK_EAST_Z0
            && loc.z <= FIRE_PLOT_VARROCK_EAST_Z1
            && loc
                .name
                .as_deref()
                .is_some_and(|name| name.trim().eq_ignore_ascii_case("fire"))
    })
}
/// Firemaking XP plus a Fire loc inside the posted Varrock East AABB, then
/// deposit except tinderbox, restock logs, and another light.
#[derive(Debug, Clone, Default, Serialize)]
pub struct FiremakerCycle {
    pub withdrawn: Option<Observation>,
    pub lit: Option<Observation>,
    pub deposited: Option<Observation>,
    pub restocked: Option<Observation>,
    pub returned: bool,
    pub further: bool,
    pub wrong_log: bool,
}

#[derive(Debug, Clone, Copy)]
pub struct FiremakerSpec {
    pub log: i32,
    pub wrong: i32,
}

pub fn firemaker_spec(case: CoreCase) -> Option<FiremakerSpec> {
    match case {
        CoreCase::Firemaker => Some(FiremakerSpec {
            log: LOGS_ID,
            wrong: OAK_LOGS_ID,
        }),
        CoreCase::FiremakerOak => Some(FiremakerSpec {
            log: OAK_LOGS_ID,
            wrong: LOGS_ID,
        }),
        _ => None,
    }
}

impl FiremakerCycle {
    pub fn observe(&mut self, spec: FiremakerSpec, baseline: &Observation, now: &Observation) {
        let FiremakerSpec { log, wrong } = spec;
        self.wrong_log |= now.item_id(wrong) > 0 || now.bank_item_id(wrong) > 0;
        if self.withdrawn.is_none()
            && now.item_id(log) >= 1
            && now.item_id(TINDERBOX_ID) >= 1
            && baseline.item_id(log) == 0
            && !fire_in_varrock_east_plot(now)
        {
            self.withdrawn = Some(now.clone());
        }
        if let Some(withdrawn) = &self.withdrawn {
            if self.lit.is_none()
                && now.skill_xp("firemaking") > baseline.skill_xp("firemaking")
                && now.item_id(log) < withdrawn.item_id(log)
                && fire_in_varrock_east_plot(now)
            {
                self.lit = Some(now.clone());
            }
        }
        if self.lit.is_some()
            && self.deposited.is_none()
            && now.bank_open
            && now.bank_loaded
            && now.bank_generation > baseline.bank_generation
            && now.item_id(log) == 0
            && now.item_id(TINDERBOX_ID) >= 1
        {
            self.deposited = Some(now.clone());
        }
        if let Some(deposited) = &self.deposited {
            if self.restocked.is_none()
                && now.bank_open
                && now.bank_loaded
                && now.bank_generation == deposited.bank_generation
                && now.item_id(log) >= 1
                && now.bank_item_id(log) < deposited.bank_item_id(log)
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
        if let (Some(_lit), Some(restocked)) = (&self.lit, &self.restocked) {
            self.further |= self.returned
                && !now.bank_open
                && now.tick > restocked.tick
                && now.item_id(log) < restocked.item_id(log)
                && now.skill_xp("firemaking") > restocked.skill_xp("firemaking")
                && fire_in_varrock_east_plot(now);
        }
    }

    pub fn qualified(&self) -> bool {
        self.further
            && self.withdrawn.is_some()
            && self.lit.is_some()
            && self.deposited.is_some()
            && self.restocked.is_some()
            && !self.wrong_log
    }
}
