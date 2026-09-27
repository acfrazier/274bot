use super::*;

/// Compact proof that this run observed the exact HerbCleaner seed in a
/// loaded bank before the bank was closed for Start. Closed snapshots
/// intentionally do not retain bank contents.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct HerbCleanerStartPreparationReceipt {
    pub player: String,
    pub tick: u32,
    pub bank_generation: u64,
    pub guam_count: i32,
    pub marrentill_count: i32,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct HerbCleanerStartPreparation {
    loaded: Option<HerbCleanerStartPreparationReceipt>,
    closed_after_loaded: bool,
}

impl HerbCleanerStartPreparation {
    pub fn observe(&mut self, account: &str, observation: &Observation) {
        let expected = client::util::jstring::JString::to_screen_name(account);
        if !observation.ingame
            || observation.scene_state != 2
            || !observation
                .player
                .as_deref()
                .is_some_and(|player| player.eq_ignore_ascii_case(&expected))
        {
            *self = Self::default();
            return;
        }

        if observation.bank_open && observation.bank_loaded {
            let guam_count = observation.bank_item_id(UNIDENTIFIED_GUAM_ID);
            let marrentill_count = observation.bank_item_id(UNIDENTIFIED_MARENTILL_ID);
            self.loaded = (guam_count == 20 && marrentill_count == 0).then(|| {
                HerbCleanerStartPreparationReceipt {
                    player: observation.player.clone().expect("player checked above"),
                    tick: observation.tick,
                    bank_generation: observation.bank_generation,
                    guam_count,
                    marrentill_count,
                }
            });
            self.closed_after_loaded = false;
            return;
        }

        if observation.bank_open || observation.bank_loaded {
            *self = Self::default();
            return;
        }

        self.closed_after_loaded = self.loaded.as_ref().is_some_and(|loaded| {
            observation.bank_generation > loaded.bank_generation
                && observation.tick >= loaded.tick
                && observation.bank.is_empty()
                && observation.bank_ids.is_empty()
        });
    }

    pub fn receipt_for(
        &self,
        account: &str,
        baseline: &Observation,
    ) -> Option<HerbCleanerStartPreparationReceipt> {
        if !self.closed_after_loaded || baseline.bank_open || baseline.bank_loaded {
            return None;
        }
        let expected = client::util::jstring::JString::to_screen_name(account);
        let loaded = self.loaded.as_ref()?;
        (baseline.bank.is_empty()
            && baseline.bank_ids.is_empty()
            && baseline.bank_generation > loaded.bank_generation
            && baseline.tick >= loaded.tick
            && baseline
                .player
                .as_deref()
                .is_some_and(|player| player.eq_ignore_ascii_case(&expected))
            && loaded.player.eq_ignore_ascii_case(&expected))
        .then(|| loaded.clone())
    }
}
pub fn herblore_eggs_noted(observation: &Observation) -> bool {
    observation.item_id(NOTED_RED_SPIDERS_EGGS_ID) > 0
        || observation.bank_item_id(NOTED_RED_SPIDERS_EGGS_ID) > 0
}

pub fn herblore_newt_noted(observation: &Observation) -> bool {
    observation.item_id(NOTED_EYE_OF_NEWT_ID) > 0
        || observation.bank_item_id(NOTED_EYE_OF_NEWT_ID) > 0
}

pub fn herblore_eggs_baseline_ready(baseline: &Observation) -> bool {
    near(baseline.tile, EGG_FIELD, 8)
        && baseline.item_id(RED_SPIDERS_EGGS_ID) == 0
        && baseline.item_id(EYE_OF_NEWT_ID) == 0
        && !herblore_eggs_noted(baseline)
}

pub fn herblore_newt_baseline_ready(baseline: &Observation) -> bool {
    near(baseline.tile, BETTY_SHOP, 6)
        && baseline.item_id(EYE_OF_NEWT_ID) == 0
        && baseline.item_id(RED_SPIDERS_EGGS_ID) == 0
        && !herblore_newt_noted(baseline)
}
/// Identify a full pack, deposit script-created output, restock, then identify again.
#[derive(Debug, Clone, Default, Serialize)]
pub struct HerbCleanerCycle {
    pub first_pack: bool,
    pub deposited: Option<Observation>,
    pub withdrawn: Option<Observation>,
    pub cleaned_after_withdrawal: bool,
    pub filter_violated: bool,
}

impl HerbCleanerCycle {
    pub fn observe(&mut self, named: bool, baseline: &Observation, now: &Observation) {
        if named
            && (now.item_id(UNIDENTIFIED_MARENTILL_ID) > 0
                || now.bank_item_id(UNIDENTIFIED_MARENTILL_ID) < 4
                    && now.bank_open
                    && now.bank_loaded)
        {
            self.filter_violated = true;
        }
        self.first_pack |= now.item_id(GUAM_LEAF_ID) >= 28
            && now.item_id(UNIDENTIFIED_GUAM_ID) == 0
            && now.skill_xp("herblore") > baseline.skill_xp("herblore")
            && baseline.item_id(GUAM_LEAF_ID) == 0;
        if self.first_pack
            && self.deposited.is_none()
            && now.bank_open
            && now.bank_loaded
            && now.bank_generation > baseline.bank_generation
            && now.item_id(GUAM_LEAF_ID) == 0
            && now.bank_item_id(GUAM_LEAF_ID) == 28
        {
            self.deposited = Some(now.clone());
        }
        if let Some(deposited) = &self.deposited {
            if self.withdrawn.is_none()
                && now.bank_open
                && now.bank_loaded
                && now.bank_generation == deposited.bank_generation
                && now.item_id(UNIDENTIFIED_GUAM_ID) >= 1
                && now.bank_item_id(UNIDENTIFIED_GUAM_ID)
                    < deposited.bank_item_id(UNIDENTIFIED_GUAM_ID)
                && (!named || now.bank_item_id(UNIDENTIFIED_MARENTILL_ID) == 4)
            {
                self.withdrawn = Some(now.clone());
            }
        }
        if let Some(withdrawn) = &self.withdrawn {
            self.cleaned_after_withdrawal |= !now.bank_open
                && !now.bank_loaded
                && now.item_id(GUAM_LEAF_ID) > 0
                && now.item_id(UNIDENTIFIED_GUAM_ID) < withdrawn.item_id(UNIDENTIFIED_GUAM_ID)
                && now.skill_xp("herblore") > withdrawn.skill_xp("herblore");
        }
    }

    pub fn qualified(&self) -> bool {
        self.cleaned_after_withdrawal && !self.filter_violated
    }
}

pub const HERB_CLEANER_EMPTY_STOP_REASON: &str = "every selected herb is empty in the bank";
pub const HERB_CLEANER_BANK_TRIP_STOP_REASON: &str = "bank has no eligible herbs";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HerbCleanerEmptyStopKind {
    LoopExhausted,
    BankTripNoEligibleHerbs,
}

impl HerbCleanerEmptyStopKind {
    fn from_reason(reason: &str) -> Option<Self> {
        match reason {
            HERB_CLEANER_EMPTY_STOP_REASON => Some(Self::LoopExhausted),
            HERB_CLEANER_BANK_TRIP_STOP_REASON => Some(Self::BankTripNoEligibleHerbs),
            _ => None,
        }
    }
}

/// Frozen 20-guam/two-selected-herb terminal fixture. Work, a fresh loaded
/// empty bank and the script's own Stop receipt must occur in that order.
#[derive(Debug, Clone, Default, Serialize)]
pub struct HerbCleanerEmptyCycle {
    pub cleaned: bool,
    pub exhausted_bank: Option<Observation>,
    pub stop_kind: Option<HerbCleanerEmptyStopKind>,
    pub stopped: Option<script::ScriptLifecycleReceipt>,
}

impl HerbCleanerEmptyCycle {
    pub fn observe(&mut self, baseline: &Observation, now: &Observation) {
        self.cleaned |= baseline.item_id(GUAM_LEAF_ID) == 0
            && now.item_id(GUAM_LEAF_ID) > 0
            && now.item_id(UNIDENTIFIED_GUAM_ID) == 0
            && now.skill_xp("herblore") > baseline.skill_xp("herblore");
        if self.cleaned
            && self.exhausted_bank.is_none()
            && now.bank_open
            && now.bank_loaded
            && now.bank_generation > baseline.bank_generation
            && now.bank_item_id(UNIDENTIFIED_GUAM_ID) == 0
            && now.bank_item_id(UNIDENTIFIED_MARENTILL_ID) == 0
        {
            self.exhausted_bank = Some(now.clone());
        }
    }

    pub fn observe_script_lifecycle(&mut self, receipt: script::ScriptLifecycleReceipt) {
        let stop_kind = HerbCleanerEmptyStopKind::from_reason(&receipt.reason);
        if self.exhausted_bank.is_some()
            && receipt.runtime_generation > 0
            && receipt.state == script::ScriptTerminalState::Stopped
            && stop_kind.is_some()
        {
            self.stop_kind = stop_kind;
            self.stopped = Some(receipt);
        }
    }

    pub fn qualified(&self) -> bool {
        self.cleaned
            && self.exhausted_bank.is_some()
            && self.stop_kind.is_some()
            && self.stopped.is_some()
    }
}

/// Ground Take 223, deposit, closed return to the egg field, further Take.
/// Eye of newt 221 must not mix into this cell. Ground spawns are not given.
#[derive(Debug, Clone, Default, Serialize)]
pub struct HerbloreEggsCycle {
    pub taken: bool,
    pub deposited: Option<Observation>,
    pub returned: bool,
    pub further: bool,
    pub wrong_product: bool,
    pub noted: bool,
}

impl HerbloreEggsCycle {
    pub fn observe(&mut self, baseline: &Observation, now: &Observation) {
        self.wrong_product |=
            now.item_id(EYE_OF_NEWT_ID) > 0 || now.bank_item_id(EYE_OF_NEWT_ID) > 0;
        self.noted |= herblore_eggs_noted(now);
        self.taken |= now.item_id(RED_SPIDERS_EGGS_ID) >= 1
            && baseline.item_id(RED_SPIDERS_EGGS_ID) == 0
            && !self.wrong_product
            && !self.noted;
        if self.taken
            && self.deposited.is_none()
            && now.bank_open
            && now.bank_loaded
            && now.bank_generation > baseline.bank_generation
            && now.item_id(RED_SPIDERS_EGGS_ID) == 0
            && now.bank_item_id(RED_SPIDERS_EGGS_ID) >= 1
        {
            self.deposited = Some(now.clone());
        }
        if let Some(deposited) = &self.deposited {
            self.returned |= !now.bank_open
                && !now.bank_loaded
                && now.bank_generation > deposited.bank_generation
                && near(now.tile, EGG_FIELD, 14);
        }
        if self.returned {
            self.further |=
                !now.bank_open && now.item_id(RED_SPIDERS_EGGS_ID) >= 1 && !self.wrong_product;
        }
    }

    pub fn qualified(&self) -> bool {
        self.further && !self.wrong_product && !self.noted
    }
}

/// Betty shop purchase 221 with coins 995, deposit, return, further buy.
/// Eggs 223 must not mix into this cell.
#[derive(Debug, Clone, Default, Serialize)]
pub struct HerbloreNewtCycle {
    pub bought: bool,
    pub deposited: Option<Observation>,
    pub returned: bool,
    pub further: bool,
    pub coins_spent: bool,
    pub coins_peak: i32,
    pub wrong_product: bool,
    pub noted: bool,
}

impl HerbloreNewtCycle {
    pub fn observe(&mut self, baseline: &Observation, now: &Observation) {
        self.wrong_product |=
            now.item_id(RED_SPIDERS_EGGS_ID) > 0 || now.bank_item_id(RED_SPIDERS_EGGS_ID) > 0;
        self.noted |= herblore_newt_noted(now);
        let previous_coins_peak = self.coins_peak;
        self.coins_peak = previous_coins_peak.max(now.item_id(COINS_ID));
        self.bought |= now.item_id(EYE_OF_NEWT_ID) >= 1
            && baseline.item_id(EYE_OF_NEWT_ID) == 0
            && !self.wrong_product
            && !self.noted;
        if self.bought
            && previous_coins_peak > 0
            && now.item_id(EYE_OF_NEWT_ID) >= 1
            && now.item_id(COINS_ID) < previous_coins_peak
        {
            self.coins_spent = true;
        }
        if self.bought
            && self.deposited.is_none()
            && now.bank_open
            && now.bank_loaded
            && now.bank_generation > baseline.bank_generation
            && now.item_id(EYE_OF_NEWT_ID) == 0
            && now.bank_item_id(EYE_OF_NEWT_ID) >= 1
        {
            self.deposited = Some(now.clone());
        }
        if let Some(deposited) = &self.deposited {
            self.returned |= !now.bank_open
                && !now.bank_loaded
                && now.bank_generation > deposited.bank_generation
                && near(now.tile, BETTY_SHOP, 6);
        }
        if self.returned {
            self.further |=
                !now.bank_open && now.item_id(EYE_OF_NEWT_ID) >= 1 && !self.wrong_product;
        }
    }

    pub fn qualified(&self) -> bool {
        self.further && self.coins_spent && !self.wrong_product && !self.noted
    }
}
