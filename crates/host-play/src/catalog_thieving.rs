use super::*;
pub fn stall_food(observation: &Observation) -> i32 {
    observation.item_id(CAKE_ID)
        + observation.item_id(BREAD_ID)
        + observation.item_id(CHOCOLATE_SLICE_ID)
}

pub fn bank_stall_food(observation: &Observation) -> i32 {
    observation.bank_item_id(CAKE_ID)
        + observation.bank_item_id(BREAD_ID)
        + observation.bank_item_id(CHOCOLATE_SLICE_ID)
}

pub fn noted_stall_food(observation: &Observation) -> i32 {
    observation.item_id(NOTED_CAKE_ID)
        + observation.item_id(NOTED_BREAD_ID)
        + observation.item_id(NOTED_CHOCOLATE_SLICE_ID)
        + observation.bank_item_id(NOTED_CAKE_ID)
        + observation.bank_item_id(NOTED_BREAD_ID)
        + observation.bank_item_id(NOTED_CHOCOLATE_SLICE_ID)
}

pub fn ardy_thiever_baseline_ready(baseline: &Observation, thieving: i32) -> bool {
    near(baseline.tile, ARDY_THIEVER_STAND, 8)
        && baseline.level("thieving") >= thieving
        && baseline.item_id(COINS_ID) == 0
        && baseline.item_id(CAKE_ID) == 0
}

/// Baker's stall Flee cell prep plus combat stats and a weapon so FightBack can kill.
pub fn ardy_cakes_fight_baseline_ready(baseline: &Observation) -> bool {
    near(baseline.tile, ARDY_CAKES_STAND, 6)
        && baseline.level("thieving") >= 5
        && baseline.level("attack") >= COMBAT_ATTACK_LEVEL
        && baseline.level("strength") >= COMBAT_ATTACK_LEVEL
        && baseline.level("hitpoints") >= COMBAT_ATTACK_LEVEL
        && baseline.item_id(KNIFE_ID) == ARDY_CAKES_BALLAST_KNIVES
        && held_id(baseline, ADAMANT_SCIMITAR_ID) >= 1
        && stall_food(baseline) == 0
        && baseline.item_id(CHOCOLATE_CAKE_ID) == 0
        && noted_stall_food(baseline) == 0
}

/// Guard pickpocket Fight cell: same empty-pack thieving prep plus combat kit.
pub fn ardy_thiever_fight_baseline_ready(baseline: &Observation) -> bool {
    ardy_thiever_baseline_ready(baseline, 40)
        && baseline.level("attack") >= COMBAT_ATTACK_LEVEL
        && baseline.level("strength") >= COMBAT_ATTACK_LEVEL
        && baseline.level("hitpoints") >= COMBAT_ATTACK_LEVEL
        && held_id(baseline, ADAMANT_SCIMITAR_ID) >= 1
}
/// Start with exact 22-Knife ballast, steal six cake/bread/chocolate slices,
/// deposit both product and ballast in a fresh bank, return to STAND, steal
/// again. Chocolate cake 1897 and noted stall food fail.
#[derive(Debug, Clone, Default, Serialize)]
pub struct ArdyCakesCycle {
    pub stolen: Option<Observation>,
    pub deposited: Option<Observation>,
    pub returned: bool,
    pub further: bool,
    pub wrong_product: bool,
    pub noted: bool,
}

impl ArdyCakesCycle {
    pub fn observe(&mut self, baseline: &Observation, now: &Observation) {
        self.wrong_product |=
            now.item_id(CHOCOLATE_CAKE_ID) > 0 || now.bank_item_id(CHOCOLATE_CAKE_ID) > 0;
        self.noted |= noted_stall_food(now) > 0;
        if self.stolen.is_none()
            && stall_food(now) >= 1
            && stall_food(baseline) == 0
            && now.skill_xp("thieving") > baseline.skill_xp("thieving")
            && now.item_id(CHOCOLATE_CAKE_ID) == 0
            && noted_stall_food(now) == 0
        {
            self.stolen = Some(now.clone());
        }
        if self.stolen.is_some()
            && self.deposited.is_none()
            && now.bank_open
            && now.bank_loaded
            && now.bank_generation > baseline.bank_generation
            && stall_food(now) == 0
            && bank_stall_food(now) >= 1
            && now.item_id(KNIFE_ID) == 0
            && now.bank_item_id(KNIFE_ID)
                >= baseline.bank_item_id(KNIFE_ID) + ARDY_CAKES_BALLAST_KNIVES
        {
            self.deposited = Some(now.clone());
        }
        if let Some(deposited) = &self.deposited {
            self.returned |= !now.bank_open
                && !now.bank_loaded
                && now.bank_generation > deposited.bank_generation
                && near(now.tile, ARDY_CAKES_STAND, 6);
        }
        if self.returned {
            self.further |= !now.bank_open && stall_food(now) >= 1;
        }
    }

    pub fn qualified(&self) -> bool {
        self.further
            && self.stolen.is_some()
            && self.deposited.is_some()
            && !self.wrong_product
            && !self.noted
    }
}

/// `guardResponse=Fight`: stall steal then FightBack kill. Landing on the Flee
/// kite tile fails — that is the other branch.
#[derive(Debug, Clone, Default, Serialize)]
pub struct ArdyCakesFightCycle {
    pub stolen: Option<Observation>,
    pub killed: Option<Observation>,
    pub fled: bool,
    pub wrong_product: bool,
    pub noted: bool,
    pub engaged_guard: Option<usize>,
    pub style_xp: bool,
}

impl ArdyCakesFightCycle {
    pub fn observe(&mut self, baseline: &Observation, now: &Observation) {
        self.wrong_product |=
            now.item_id(CHOCOLATE_CAKE_ID) > 0 || now.bank_item_id(CHOCOLATE_CAKE_ID) > 0;
        self.noted |= noted_stall_food(now) > 0;
        self.style_xp |= now.skill_xp("strength") > baseline.skill_xp("strength")
            || now.skill_xp("attack") > baseline.skill_xp("attack");
        if self.stolen.is_none()
            && stall_food(now) >= 1
            && stall_food(baseline) == 0
            && now.skill_xp("thieving") > baseline.skill_xp("thieving")
            && now.item_id(CHOCOLATE_CAKE_ID) == 0
            && noted_stall_food(now) == 0
        {
            self.stolen = Some(now.clone());
        }
        if self.stolen.is_some() && self.killed.is_none() {
            self.fled |= near(now.tile, ARDY_FLEE_TILE, 2);
        }
        for npc in &now.npc_facts {
            let Some(name) = npc.name.as_deref().map(str::trim) else {
                continue;
            };
            if name != "Guard" {
                continue;
            }
            let selected = npc.targeting_local
                || (now.local_target_npc == Some(npc.index) && now.local_in_combat);
            if selected {
                self.engaged_guard = Some(npc.index);
            }
            if self.stolen.is_some()
                && self.killed.is_none()
                && self.engaged_guard == Some(npc.index)
                && npc.total_health > 0
                && npc.health == 0
                && self.style_xp
            {
                self.killed = Some(now.clone());
            }
        }
        // Despawn after engagement also counts once style XP landed.
        if self.stolen.is_some()
            && self.killed.is_none()
            && self.style_xp
            && self
                .engaged_guard
                .is_some_and(|index| !now.npc_facts.iter().any(|npc| npc.index == index))
        {
            self.killed = Some(now.clone());
        }
    }

    pub fn qualified(&self) -> bool {
        self.stolen.is_some()
            && self.killed.is_some()
            && self.style_xp
            && !self.fled
            && !self.wrong_product
            && !self.noted
    }
}

/// Pickpocket coins with Thieving XP, deposit coins at loot-count 1, return
/// to the market stand, pickpocket again. Stall food without coins cannot
/// qualify. Fight stays pending.
#[derive(Debug, Clone, Default, Serialize)]
pub struct ArdyThieverCycle {
    pub pickpocketed: Option<Observation>,
    pub deposited: Option<Observation>,
    pub returned: bool,
    pub further: bool,
}

impl ArdyThieverCycle {
    pub fn observe(&mut self, baseline: &Observation, now: &Observation) {
        if self.pickpocketed.is_none()
            && now.item_id(COINS_ID) >= 1
            && baseline.item_id(COINS_ID) == 0
            && now.skill_xp("thieving") > baseline.skill_xp("thieving")
        {
            self.pickpocketed = Some(now.clone());
        }
        if self.pickpocketed.is_some()
            && self.deposited.is_none()
            && now.bank_open
            && now.bank_loaded
            && now.bank_generation > baseline.bank_generation
            && now.item_id(COINS_ID) == 0
            && now.bank_item_id(COINS_ID) >= 1
        {
            self.deposited = Some(now.clone());
        }
        if let Some(deposited) = &self.deposited {
            self.returned |= !now.bank_open
                && !now.bank_loaded
                && now.bank_generation > deposited.bank_generation
                && near(now.tile, ARDY_THIEVER_STAND, 6);
        }
        if self.returned {
            self.further |= !now.bank_open && now.item_id(COINS_ID) >= 1;
        }
    }

    pub fn qualified(&self) -> bool {
        self.further && self.pickpocketed.is_some() && self.deposited.is_some()
    }
}

/// `guardResponse=Fight` on ArdyThiever: a FightBack Guard kill plus the Flee
/// bank-cycle shape. The kill and the Flee-kite check count from Start, not
/// from the first coins: on 289 only a caught stall steal draws a Guard (a
/// failed pickpocket stuns but never starts combat), and ArdyThiever steals
/// from the stall only while its food is at `restockAtFood`, so the catch
/// comes from the opening restock. The deposit still needs both the coins
/// and the kill. The Flee kite tile fails this branch.
///
/// Coins count only as a pickpocket: a coin gain with a Thieving XP rise in
/// the same or the previous observation (the pickpocket script adds the
/// coins and the XP in one server tick). A dropped Guard's coins picked up by
/// `LootDrops` carry no Thieving XP, and neither does the stall steal that
/// raised it earlier.
#[derive(Debug, Clone, Default, Serialize)]
pub struct ArdyThieverFightCycle {
    pub pickpocketed: Option<Observation>,
    pub killed: Option<Observation>,
    pub deposited: Option<Observation>,
    pub returned: bool,
    pub further: bool,
    pub fled: bool,
    pub engaged_guard: Option<usize>,
    pub style_xp: bool,
    /// Coins and Thieving XP at the previous observation, and whether the
    /// XP rose on it.
    #[serde(skip)]
    last: Option<(i32, i32, bool)>,
}

impl ArdyThieverFightCycle {
    pub fn observe(&mut self, baseline: &Observation, now: &Observation) {
        self.style_xp |= now.skill_xp("strength") > baseline.skill_xp("strength")
            || now.skill_xp("attack") > baseline.skill_xp("attack");
        let (coins, thieving) = (now.item_id(COINS_ID), now.skill_xp("thieving"));
        let (last_coins, last_thieving, xp_rose_before) = self.last.unwrap_or((
            baseline.item_id(COINS_ID),
            baseline.skill_xp("thieving"),
            false,
        ));
        let xp_rose = thieving > last_thieving;
        let picked = coins > last_coins && (xp_rose || xp_rose_before);
        self.last = Some((coins, thieving, xp_rose));
        if self.pickpocketed.is_none() && picked && baseline.item_id(COINS_ID) == 0 {
            self.pickpocketed = Some(now.clone());
        }
        if self.killed.is_none() {
            self.fled |= near(now.tile, ARDY_FLEE_TILE, 2);
        }
        for npc in &now.npc_facts {
            let Some(name) = npc.name.as_deref().map(str::trim) else {
                continue;
            };
            if name != "Guard" {
                continue;
            }
            let selected = npc.targeting_local
                || (now.local_target_npc == Some(npc.index) && now.local_in_combat);
            if selected {
                self.engaged_guard = Some(npc.index);
            }
            if self.killed.is_none()
                && self.engaged_guard == Some(npc.index)
                && npc.total_health > 0
                && npc.health == 0
                && self.style_xp
            {
                self.killed = Some(now.clone());
            }
        }
        if self.killed.is_none()
            && self.style_xp
            && self
                .engaged_guard
                .is_some_and(|index| !now.npc_facts.iter().any(|npc| npc.index == index))
        {
            self.killed = Some(now.clone());
        }
        if self.pickpocketed.is_some()
            && self.killed.is_some()
            && self.deposited.is_none()
            && now.bank_open
            && now.bank_loaded
            && now.bank_generation > baseline.bank_generation
            && now.item_id(COINS_ID) == 0
            && now.bank_item_id(COINS_ID) >= 1
        {
            self.deposited = Some(now.clone());
        }
        if let Some(deposited) = &self.deposited {
            self.returned |= !now.bank_open
                && !now.bank_loaded
                && now.bank_generation > deposited.bank_generation
                && near(now.tile, ARDY_THIEVER_STAND, 6);
        }
        if self.returned {
            self.further |= !now.bank_open && picked;
        }
    }

    pub fn qualified(&self) -> bool {
        self.further
            && self.pickpocketed.is_some()
            && self.killed.is_some()
            && self.deposited.is_some()
            && self.style_xp
            && !self.fled
    }
}
