use super::*;

/// Noted-id withdrawal then Nature-rune consumption. Display name
/// "Adamant scimitar" is shared by unnoted 1331 and certificate 1332.
#[derive(Debug, Clone, Default, Serialize)]
pub struct AlcherGeneratedCustomCycle {
    pub withdrawn: Option<Observation>,
    pub consumed: bool,
    /// Selected spell a spelled row demanded (`None` on the High/default rows).
    pub spell: Option<AlcherSpell>,
    /// Client-scale Magic XP one cast of that spell credits.
    pub magic_xp_per_cast: i32,
    /// Coins one cast pays for the selected item at that spell's rate.
    pub coins_per_cast: i32,
    /// Casts the noted-stack, rune and XP deltas agreed on.
    pub casts: i32,
    /// Fire staff worn on the baseline frame (`None`: no staff before Start).
    pub baseline_staff: Option<i32>,
    /// Fire staff worn on the frame that proved the cast.
    pub cast_staff: Option<i32>,
    /// Fire staff a spelled row required; `None` when the row claims none.
    pub required_staff: Option<i32>,
    /// A fire staff other than the required one was worn after Start.
    pub wrong_staff: bool,
}

/// The Alcher's selected spell: Low needs 21 Magic and pays 40% of shop cost,
/// High needs 55 and pays 60%.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AlcherSpell {
    Low,
    High,
}

/// One selected spell's per-cast expectations for the selected item.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AlcherSpellExpectation {
    pub spell: AlcherSpell,
    /// Client-scale Magic XP one cast credits (Low 31 / High 65).
    pub magic_xp: i32,
    /// Coins one cast pays: floor(canonical item cost * the spell's rate).
    pub coins_per_cast: i32,
    /// Fire staff the fixture's gear step must actually wear.
    pub staff: i32,
}

/// Low Level Alchemy on noted rune chainbodies under a Staff of fire (#744).
pub const ALCHER_LOW_EXPECTATION: AlcherSpellExpectation = AlcherSpellExpectation {
    spell: AlcherSpell::Low,
    magic_xp: LOW_ALCH_MAGIC_XP,
    coins_per_cast: RUNE_CHAINBODY_LOW_ALCH_COINS,
    staff: STAFF_OF_FIRE_ID,
};

/// High Level Alchemy on noted rune chainbodies under a Fire battlestaff.
pub const ALCHER_FIRE_BATTLESTAFF_EXPECTATION: AlcherSpellExpectation = AlcherSpellExpectation {
    spell: AlcherSpell::High,
    magic_xp: HIGH_ALCH_MAGIC_XP,
    coins_per_cast: RUNE_CHAINBODY_HIGH_ALCH_COINS,
    staff: FIRE_BATTLESTAFF_ID,
};

/// The fire staff worn right now, if any.
fn worn_fire_staff(observation: &Observation) -> Option<i32> {
    ALCHER_FIRE_STAFFS
        .iter()
        .copied()
        .find(|id| observation.equipment_id(*id) > 0)
}

impl AlcherGeneratedCustomCycle {
    pub fn observe(&mut self, baseline: &Observation, now: &Observation) {
        self.observe_target(
            baseline,
            now,
            ADAMANT_SCIMITAR_ID,
            CERT_ADAMANT_SCIMITAR_ID,
            ADAMANT_SCIMITAR_ALCH_COINS,
        );
    }

    /// A fresh bank generation handed over the noted stack, with no unnoted copy
    /// in the pack and at least one cast's fuel.
    fn withdrawal_ready(
        &self,
        baseline: &Observation,
        now: &Observation,
        unnoted: i32,
        noted: i32,
    ) -> bool {
        self.withdrawn.is_none()
            && now.bank_generation > baseline.bank_generation
            && now.item_id(noted) >= 1
            && now.item_id(unnoted) == 0
            && now.item_id(noted) > baseline.item_id(noted)
            && now.item_id(NATURE_RUNE_ID) >= 1
    }

    /// The High/default rows keep their own name-based guard on top of the
    /// shared withdrawal, exactly as before.
    pub fn observe_target(
        &mut self,
        baseline: &Observation,
        now: &Observation,
        unnoted: i32,
        noted: i32,
        alch_coins: i32,
    ) {
        if self.withdrawal_ready(baseline, now, unnoted, noted) && now.item("Rune chainbody") == 0 {
            self.withdrawn = Some(now.clone());
        }
        if let Some(withdrawn) = &self.withdrawn {
            self.consumed |= !now.bank_open
                && !now.bank_loaded
                && now.item_id(noted) < withdrawn.item_id(noted)
                && now.item_id(NATURE_RUNE_ID) < withdrawn.item_id(NATURE_RUNE_ID)
                && now.item_id(COINS_ID) - baseline.item_id(COINS_ID) == alch_coins
                && now.skill_xp("magic") - baseline.skill_xp("magic") >= HIGH_ALCH_MAGIC_XP
                && now.item("Rune chainbody") == 0;
        }
    }

    /// The selected-spell rows: the ordered withdrawal, then one frame where the
    /// noted stack, the Nature runes, the Magic XP and the coins agree on the
    /// same number of casts, with the selected fire staff actually worn after a
    /// baseline that wore no staff at all.
    pub fn observe_spelled(
        &mut self,
        expectation: AlcherSpellExpectation,
        baseline: &Observation,
        now: &Observation,
        unnoted: i32,
        noted: i32,
    ) {
        self.spell = Some(expectation.spell);
        self.magic_xp_per_cast = expectation.magic_xp;
        self.coins_per_cast = expectation.coins_per_cast;
        self.required_staff = Some(expectation.staff);
        if self.baseline_staff.is_none() {
            self.baseline_staff = worn_fire_staff(baseline);
        }
        self.wrong_staff |= worn_fire_staff(now).is_some_and(|worn| worn != expectation.staff);
        if self.withdrawal_ready(baseline, now, unnoted, noted) {
            self.withdrawn = Some(now.clone());
        }
        let withdrawn = match &self.withdrawn {
            Some(withdrawn) => withdrawn,
            None => return,
        };
        if now.bank_open || now.bank_loaded || self.consumed {
            return;
        }
        let casts = withdrawn.item_id(noted) - now.item_id(noted);
        if casts < 1 {
            return;
        }
        let coins = expectation.coins_per_cast * casts;
        let xp = expectation.magic_xp * casts;
        let consumed = withdrawn.item_id(NATURE_RUNE_ID) - now.item_id(NATURE_RUNE_ID) == casts
            && now.item_id(COINS_ID) - withdrawn.item_id(COINS_ID) == coins
            && now.skill_xp("magic") - withdrawn.skill_xp("magic") == xp
            && now.item_id(COINS_ID) - baseline.item_id(COINS_ID) == coins
            && now.skill_xp("magic") - baseline.skill_xp("magic") == xp
            && worn_fire_staff(now) == Some(expectation.staff);
        if consumed {
            self.casts = casts;
            self.cast_staff = worn_fire_staff(now);
            self.consumed = true;
        }
    }

    /// Qualification for a spelled row: the exact cast arithmetic proved, the
    /// demanded staff worn, no staff in the baseline and no other fire staff
    /// seen anywhere after Start.
    pub fn qualified_spell(&self) -> bool {
        self.consumed
            && self.casts >= 1
            && !self.wrong_staff
            && self.baseline_staff.is_none()
            && self.required_staff.is_some()
            && self.cast_staff == self.required_staff
    }
}

fn swarm_targeting_local(observation: &Observation) -> bool {
    observation.npc_facts.iter().any(|npc| {
        npc.name
            .as_deref()
            .is_some_and(|name| name.eq_ignore_ascii_case("Swarm"))
            && npc.targeting_local
    })
}

fn evade_hold(observation: &Observation) -> bool {
    observation.guardian.kind.as_deref() == Some("evade")
        && observation.guardian.hold
        && observation.guardian.ours
        && observation
            .guardian
            .name
            .as_deref()
            .is_some_and(|name| name.eq_ignore_ascii_case("Swarm"))
}

fn high_cast(from: &Observation, now: &Observation, noted: i32, coins_per_cast: i32) -> bool {
    if now.bank_open || now.bank_loaded {
        return false;
    }
    let casts = from.item_id(noted) - now.item_id(noted);
    if casts < 1 {
        return false;
    }
    from.item_id(NATURE_RUNE_ID) - now.item_id(NATURE_RUNE_ID) == casts
        && now.item_id(COINS_ID) - from.item_id(COINS_ID) == coins_per_cast * casts
        && now.skill_xp("magic") - from.skill_xp("magic") == HIGH_ALCH_MAGIC_XP * casts
        && worn_fire_staff(now) == Some(STAFF_OF_FIRE_ID)
}

fn rich_chainbody_exhausted(observation: &Observation) -> bool {
    observation.item_id(CERT_RUNE_CHAINBODY_ID) == 0 && observation.item_id(RUNE_CHAINBODY_ID) == 0
}

/// Ordered High-alch interruption: first cast, targeted Swarm + positive hit,
/// native guardian hold, actual flee under that hold, native hold release
/// independent of bank proximity, further rich cast from that release baseline
/// that exhausts remaining notes, bank return and loaded rich retirement near
/// Varrock West, then a complete poor restock (noted poor and Nature fuel)
/// before bank-closed consumption.
#[derive(Debug, Clone, Default, Serialize)]
pub struct AlcherSwarmDrainCycle {
    pub withdrawn: Option<Observation>,
    pub first_cast: Option<Observation>,
    pub swarm_hit: Option<Observation>,
    pub guardian_hold: Option<Observation>,
    pub fled: Option<Observation>,
    pub released: Option<Observation>,
    pub further_cast: Option<Observation>,
    pub rich_retired: Option<Observation>,
    pub poor_withdrawn: Option<Observation>,
    pub poor_consumed: bool,
    pub baseline_staff: Option<i32>,
    pub wrong_staff: bool,
}

impl AlcherSwarmDrainCycle {
    pub fn observe(&mut self, baseline: &Observation, now: &Observation) {
        if self.baseline_staff.is_none() {
            self.baseline_staff = worn_fire_staff(baseline);
        }
        self.wrong_staff |= worn_fire_staff(now).is_some_and(|worn| worn != STAFF_OF_FIRE_ID);
        if self.withdrawn.is_none()
            && now.bank_generation > baseline.bank_generation
            && now.item_id(CERT_RUNE_CHAINBODY_ID) >= 1
            && now.item_id(RUNE_CHAINBODY_ID) == 0
            && now.item_id(CERT_RUNE_CHAINBODY_ID) > baseline.item_id(CERT_RUNE_CHAINBODY_ID)
            && now.item_id(NATURE_RUNE_ID) >= 1
        {
            self.withdrawn = Some(now.clone());
        }
        if self.first_cast.is_none() {
            if let Some(withdrawn) = &self.withdrawn {
                if high_cast(
                    withdrawn,
                    now,
                    CERT_RUNE_CHAINBODY_ID,
                    RUNE_CHAINBODY_HIGH_ALCH_COINS,
                ) {
                    self.first_cast = Some(now.clone());
                }
            }
        }
        if self.first_cast.is_some()
            && self.swarm_hit.is_none()
            && swarm_targeting_local(now)
            && now.taking_damage
        {
            self.swarm_hit = Some(now.clone());
        }
        if self.swarm_hit.is_some() && self.guardian_hold.is_none() && evade_hold(now) {
            self.guardian_hold = Some(now.clone());
        }
        if let Some(held) = &self.guardian_hold {
            if self.fled.is_none()
                && evade_hold(now)
                && now.tile.is_some()
                && held.tile.is_some()
                && now.tile != held.tile
            {
                self.fled = Some(now.clone());
            }
        }
        if self.fled.is_some()
            && self.released.is_none()
            && !now.guardian.hold
            && now.guardian.kind.as_deref() != Some("evade")
        {
            self.released = Some(now.clone());
        }
        if self.further_cast.is_none() {
            if let Some(released) = &self.released {
                if high_cast(
                    released,
                    now,
                    CERT_RUNE_CHAINBODY_ID,
                    RUNE_CHAINBODY_HIGH_ALCH_COINS,
                ) && rich_chainbody_exhausted(now)
                {
                    self.further_cast = Some(now.clone());
                }
            }
        }
        if self
            .further_cast
            .as_ref()
            .is_some_and(rich_chainbody_exhausted)
            && self.rich_retired.is_none()
            && now.bank_open
            && now.bank_loaded
            && now.bank_generation > baseline.bank_generation
            && now.bank_item_id(RUNE_CHAINBODY_ID) == 0
            && now.bank_item_id(CERT_RUNE_CHAINBODY_ID) == 0
            && rich_chainbody_exhausted(now)
            && near(now.tile, VARROCK_WEST_BANK, 6)
        {
            self.rich_retired = Some(now.clone());
        }
        if self.rich_retired.is_some()
            && self.poor_withdrawn.is_none()
            && now.bank_generation > baseline.bank_generation
            && now.item_id(CERT_YEW_LONGBOW_ID) >= 1
            && now.item_id(YEW_LONGBOW_ID) == 0
            && now.item_id(CERT_YEW_LONGBOW_ID) > baseline.item_id(CERT_YEW_LONGBOW_ID)
            && now.item_id(NATURE_RUNE_ID) >= 1
        {
            self.poor_withdrawn = Some(now.clone());
        }
        if let Some(poor) = &self.poor_withdrawn {
            self.poor_consumed |= high_cast(poor, now, CERT_YEW_LONGBOW_ID, YEW_LONGBOW_ALCH_COINS);
        }
    }

    pub fn qualified(&self) -> bool {
        self.first_cast.is_some()
            && self.swarm_hit.is_some()
            && self.guardian_hold.is_some()
            && self.fled.is_some()
            && self.released.is_some()
            && self.further_cast.is_some()
            && self.rich_retired.is_some()
            && self.poor_consumed
            && !self.wrong_staff
            && self.baseline_staff.is_none()
    }

    /// Compact named-phase receipt for timeout/stop diagnosis. Does not
    /// serialize world snapshots or grow an observation history.
    pub fn phase_status(&self) -> Value {
        json!({
            "firstcast": self.first_cast.is_some(),
            "swarm_hit": self.swarm_hit.is_some(),
            "guardianhold": self.guardian_hold.is_some(),
            "fled": self.fled.is_some(),
            "released": self.released.is_some(),
            "further": self.further_cast.is_some(),
            "richretired": self.rich_retired.is_some(),
            "poor": self.poor_consumed,
        })
    }
}
