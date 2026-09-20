//! RangingGuild family observers. Qualifies post-Start work only; seeded
//! tickets or arrows cannot satisfy a spend or payout.

use serde::Serialize;

use super::{near, Observation, COINS_ID, MAGIC_SHORTBOW_ID, RUNE_ARROW_ID};

/// Selected 289 `obj.pack` / `289.json` `archery_ticket`.
pub const ARCHERY_TICKET_ID: i32 = 1464;
/// Packed `varp.pack`: `156=targetcount`.
pub const VARP_TARGET_COUNT: i32 = 156;
/// Packed `varp.pack`: `157=targetscore`.
pub const VARP_TARGET_SCORE: i32 = 157;
/// Packed `varp.pack`: `158=targethit`.
pub const VARP_TARGET_HIT: i32 = 158;
/// `RangingGuildLogic` / `competition_judge.rs2`.
pub const ENTRY_FEE: i32 = 200;
/// Frozen live / Logic / `tickets_shop:com_90`.
pub const TICKETS_PER_TRADE: i32 = 2000;
pub const RUNE_ARROWS_PER_TRADE: i32 = 50;
/// Frozen live `RANGED = 70`. Door needs 40; Magic shortbow needs 50.
pub const RANGED_LIVE: i32 = 70;
/// Logic `STAND` / live `--phase round` seat.
pub const RANGING_GUILD_STAND: (i32, i32, i32) = (2672, 3419, 0);
/// Logic `MERCHANT_STAND` / live `--phase redeem` seat.
pub const RANGING_GUILD_MERCHANT_STAND: (i32, i32, i32) = (2659, 3430, 0);
/// Logic `SEERS_BANK` / live `--phase full` booth stand. Live `teleTo` r6.
pub const RANGING_GUILD_SEERS_BANK: (i32, i32, i32) = (2725, 3491, 0);
/// Live `--phase full` `teleTo(SEERS_BANK, 6)` and Fire Camelot Seers radius.
pub const SEERS_BANK_RADIUS: i32 = 6;
/// Live `--phase full` seeds `TICKETS_PER_TRADE - 1` so redeem is not first.
pub const SEED_KEEP_TICKETS: i32 = TICKETS_PER_TRADE - 1;
/// Live full `coinsPerTrip` / two-round funding. Bank withdraw-X target.
pub const COINS_PER_TRIP: i32 = ENTRY_FEE * 2;

/// Ordered fee, shot, payout reset, ticket gain, then a further enter.
#[derive(Debug, Clone, Default, Serialize)]
pub struct RangingGuildRoundCycle {
    pub paid: bool,
    pub started: bool,
    pub progressed: bool,
    pub reset: bool,
    pub paid_out: bool,
    pub further: bool,
}

impl RangingGuildRoundCycle {
    pub fn observe(&mut self, baseline: &Observation, now: &Observation) {
        let coins_spent = baseline
            .item_id(COINS_ID)
            .saturating_sub(now.item_id(COINS_ID));
        let count = now.varp(VARP_TARGET_COUNT);
        let score = now.varp(VARP_TARGET_SCORE);
        self.paid |= coins_spent >= ENTRY_FEE && count >= 1;
        if !self.paid {
            return;
        }
        self.started |= count >= 1;
        if !self.started {
            return;
        }
        self.progressed |= count >= 2
            || score > baseline.varp(VARP_TARGET_SCORE)
            || now.skill_xp("ranged") > baseline.skill_xp("ranged");
        if !self.progressed {
            return;
        }
        self.reset |= count == 0;
        if !self.reset {
            return;
        }
        self.paid_out |= now.item_id(ARCHERY_TICKET_ID) > baseline.item_id(ARCHERY_TICKET_ID);
        if !self.paid_out {
            return;
        }
        self.further |= coins_spent >= ENTRY_FEE * 2 && count >= 1;
    }

    pub fn qualified(&self) -> bool {
        self.further
    }
}

/// Seeded-ticket spend plus a resulting rune-arrow gain. Not an earned round.
#[derive(Debug, Clone, Default, Serialize)]
pub struct RangingGuildRedeemCycle {
    pub spent: bool,
    pub received: bool,
}

impl RangingGuildRedeemCycle {
    pub fn observe(&mut self, baseline: &Observation, now: &Observation) {
        let tickets_spent = baseline
            .item_id(ARCHERY_TICKET_ID)
            .saturating_sub(now.item_id(ARCHERY_TICKET_ID));
        let arrows_gained = now
            .item_id(RUNE_ARROW_ID)
            .saturating_sub(baseline.item_id(RUNE_ARROW_ID));
        self.spent |= tickets_spent >= TICKETS_PER_TRADE;
        self.received |= arrows_gained >= RUNE_ARROWS_PER_TRADE;
    }

    pub fn qualified(&self) -> bool {
        self.spent && self.received
    }
}

pub fn ranging_guild_round_baseline_ready(baseline: &Observation) -> bool {
    super::near(baseline.tile, RANGING_GUILD_STAND, 2)
        && baseline.level("ranged") >= RANGED_LIVE
        && baseline.effective_level("ranged") >= RANGED_LIVE
        && baseline.item_id(MAGIC_SHORTBOW_ID) >= 1
        && baseline.item_id(COINS_ID) >= ENTRY_FEE * 2
        && baseline.item_id(ARCHERY_TICKET_ID) == 0
        && baseline.item_id(RUNE_ARROW_ID) == 0
        && baseline.varp(VARP_TARGET_COUNT) == 0
}

pub fn ranging_guild_redeem_baseline_ready(baseline: &Observation) -> bool {
    super::near(baseline.tile, RANGING_GUILD_MERCHANT_STAND, 3)
        && baseline.level("ranged") >= RANGED_LIVE
        && baseline.effective_level("ranged") >= RANGED_LIVE
        && baseline.item_id(MAGIC_SHORTBOW_ID) >= 1
        && baseline.item_id(ARCHERY_TICKET_ID) >= TICKETS_PER_TRADE
        && baseline.item_id(RUNE_ARROW_ID) == 0
}

/// Seers KEEP deposit, coin withdraw-X, close, STAND return, then a real fee.
/// Seeded tickets/arrows are setup, not earned gold.
#[derive(Debug, Clone, Default, Serialize)]
pub struct RangingGuildBankCycle {
    pub deposited: bool,
    pub coin_withdrawn: bool,
    pub closed: bool,
    pub returned: bool,
    pub further: bool,
    pub wrong_bank: bool,
    pub banked: Option<Observation>,
    pub coins_at_withdraw: i32,
}

impl RangingGuildBankCycle {
    fn keep_held(baseline: &Observation, now: &Observation) -> bool {
        now.item_id(ARCHERY_TICKET_ID) >= baseline.item_id(ARCHERY_TICKET_ID)
    }

    pub fn observe(&mut self, baseline: &Observation, now: &Observation) {
        let open = now.bank_open && now.bank_loaded;
        let at_seers = near(now.tile, RANGING_GUILD_SEERS_BANK, SEERS_BANK_RADIUS);
        if open && !at_seers {
            self.wrong_bank = true;
        }
        let deposited_now = open
            && at_seers
            && now.bank_generation > baseline.bank_generation
            && now.item_id(RUNE_ARROW_ID) == 0
            && now.bank_item_id(RUNE_ARROW_ID) > baseline.bank_item_id(RUNE_ARROW_ID)
            && Self::keep_held(baseline, now);
        if self.banked.is_none() && deposited_now {
            self.deposited = true;
            self.banked = Some(now.clone());
            if now.item_id(COINS_ID) > baseline.item_id(COINS_ID) {
                self.coin_withdrawn = true;
                self.coins_at_withdraw = now.item_id(COINS_ID);
            }
        }
        if let Some(banked) = &self.banked {
            if open && now.bank_generation == banked.bank_generation {
                let from_bank = now.bank_item_id(COINS_ID) < banked.bank_item_id(COINS_ID)
                    || now.item_id(COINS_ID) > banked.item_id(COINS_ID)
                    || banked.item_id(COINS_ID) > baseline.item_id(COINS_ID);
                if now.item_id(COINS_ID) > baseline.item_id(COINS_ID) && from_bank {
                    self.coin_withdrawn = true;
                    self.coins_at_withdraw = self.coins_at_withdraw.max(now.item_id(COINS_ID));
                }
            }
            self.closed |=
                !now.bank_open && !now.bank_loaded && now.bank_generation > banked.bank_generation;
        }
        if self.closed {
            self.returned |= !now.bank_open && near(now.tile, RANGING_GUILD_STAND, 2);
            if self.returned {
                self.further |= self.coin_withdrawn
                    && self.coins_at_withdraw >= ENTRY_FEE
                    && now.item_id(COINS_ID) + ENTRY_FEE <= self.coins_at_withdraw
                    && now.varp(VARP_TARGET_COUNT) >= 1;
            }
        }
    }

    pub fn qualified(&self) -> bool {
        self.deposited
            && self.coin_withdrawn
            && self.closed
            && self.returned
            && self.further
            && !self.wrong_bank
    }
}

pub fn ranging_guild_bank_baseline_ready(baseline: &Observation) -> bool {
    near(baseline.tile, RANGING_GUILD_SEERS_BANK, SEERS_BANK_RADIUS)
        && baseline.level("ranged") >= RANGED_LIVE
        && baseline.effective_level("ranged") >= RANGED_LIVE
        && baseline.item_id(ARCHERY_TICKET_ID) == SEED_KEEP_TICKETS
        && baseline.item_id(RUNE_ARROW_ID) == RUNE_ARROWS_PER_TRADE
        && baseline.item_id(COINS_ID) == 0
        && baseline.item_id(MAGIC_SHORTBOW_ID) == 0
        && baseline.equipment_id(MAGIC_SHORTBOW_ID) == 0
        && baseline.varp(VARP_TARGET_COUNT) == 0
}
