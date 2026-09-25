use super::*;
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MuleRole {
    Crafter,
    Mule,
}
pub fn mule_operation_gates() -> &'static [OperationGate] {
    &[
        OperationGate {
            source: "MuleCrafter.ts MuleTradeWithCrafter/CrafterRequestTrade",
            call: "Trade.request(playerName)",
            host_shape: "{ op: 'player', name, action: 'Trade' }",
            kind: GateKind::Mapped,
            owner: "this fixture observes; runtime already maps player Trade",
        },
        OperationGate {
            source: "MuleCrafter.ts MuleTradeExecute",
            call: "Trade.offerAll('Rune essence', i => i.id === 1436)",
            host_shape: "shim Trade.offerAll(name) ignores id filter; press trade_side name rows",
            kind: GateKind::ArityLimited,
            owner: "Mule TRADE_CAP 27 uses name-only offerAll of unnoted 1436; seed unnoted-only so ignored id-filter cannot offer 1437",
        },
        OperationGate {
            source: "MuleCrafter.ts CrafterTradeAtRuins",
            call: "Trade.offerAll(non-talisman names) when classifyMuleState sees essence",
            host_shape: "name-only offerAll of crafted Air rune after mule essence is on the window",
            kind: GateKind::Mapped,
            owner: "this fixture; crafter keeps Air talisman 1438",
        },
        OperationGate {
            source: "MuleCrafter.ts MuleTradeExecute/CrafterTradeAtRuins",
            call: "Trade.accept() / Trade.decline()",
            host_shape: "if-button trade_accept_id / trade_decline_id; throws notImpl when id < 0",
            kind: GateKind::Mapped,
            owner: "this fixture requires both offer and confirm accepts with posted ids",
        },
        OperationGate {
            source: "MuleCrafter.ts MuleGoBank",
            call: "Bank.openBooth(bankTile('Falador East'), 'Bank booth', 'Use-quickly')",
            host_shape: "named-bank already posted; walk-near stand then open-booth exact name/op",
            kind: GateKind::Mapped,
            owner: "this fixture; mule deposits received Air 556 then withdraws a new unnoted 1436 load",
        },
        OperationGate {
            source: "MuleCrafter.ts MuleGoBank",
            call: "Bank.deposit('Air rune', 'Deposit-All') then Bank.withdrawX('Rune essence', 27)",
            host_shape: "existing Bank.deposit / withdrawX + count-dialog",
            kind: GateKind::Mapped,
            owner: "this fixture; Mule first 27 and Crafter raw bootstrap 27 are distinct seeds, not restock or produced runes",
        },
        OperationGate {
            source: "MuleCrafter.ts EnterAltar / CraftRunes",
            call: "talisman.useOn(Mysterious ruins) / Altar.interact('Craft-rune')",
            host_shape: "existing item-on-loc and loc interact",
            kind: GateKind::Mapped,
            owner: "this fixture; RC XP and Air 556 must come from script craft",
        },
        OperationGate {
            source: "MuleCrafter.ts essCount",
            call: "reader.inventory() filtered by id 1436",
            host_shape: "posted inventory reader",
            kind: GateKind::Mapped,
            owner: "solo Air PASS already used this reader; pair still needs Trade",
        },
        OperationGate {
            source: "MuleCrafter.ts bankFill=false / non-Air runes / Nature",
            call: "mules bring all essence; Mind/Nature tiles",
            host_shape: "n/a",
            kind: GateKind::UnusedByCase,
            owner: "genuine extra gameplay after Air pair; NatureCrafter owns ship/Jiminua",
        },
        OperationGate {
            source: "MuleCrafter.ts walkTo",
            call: "Traversal.walkResilient(ruins|Falador East)",
            host_shape: "existing walk-near / resilient walk; not Game.teleport",
            kind: GateKind::Mapped,
            owner: "teleport unused; prep tele is pre-Start seed only",
        },
    ]
}
pub fn mule_mode_requires_partner(role: MuleRole, partner: &str) -> Result<(), String> {
    if role == MuleRole::Mule && partner.trim().is_empty() {
        return Err(
            "fixture miss: Mule mode throws without a crafter name; empty mule partner is not a LIVE cell"
                .into(),
        );
    }
    Ok(())
}

pub fn mule_prepared_current(
    role: MuleRole,
    expected_player: &str,
    observation: &AirObservation,
) -> Result<(), String> {
    if !observation.ingame || observation.scene_state != 2 {
        return Err(format!(
            "Start baseline is not attached ingame scene2: {observation:?}"
        ));
    }
    if !observation.inventory_tab_available {
        return Err("Start baseline inventory tab is not bound after relog".into());
    }
    let player = observation
        .player
        .as_deref()
        .ok_or_else(|| "Start baseline has no local player".to_string())?;
    if !account_identity_eq(player, expected_player) {
        return Err(format!(
            "Start baseline player {player:?} is not fresh account {expected_player:?}"
        ));
    }
    if !near(observation.tile, AIR_RUINS, 8) {
        return Err(format!(
            "Start baseline is not at Air ruins: {:?}",
            observation.tile
        ));
    }
    if observation.air_runes > 0 {
        return Err("Start baseline already has Air 556".into());
    }
    if observation.essence_noted > 0 {
        return Err(
            "Start baseline has noted essence 1437; Mule Air does not accept noting".into(),
        );
    }
    if observation.trade_active() {
        return Err("Start baseline already has a trade window".into());
    }
    if observation.bank_open {
        return Err("Start baseline still has an open bank".into());
    }
    match role {
        MuleRole::Crafter => {
            if observation.air_talisman <= 0 {
                return Err("crafter baseline has no Air talisman".into());
            }
            if observation.essence_unnoted != MULE_TRADE_CAP {
                return Err(
                    "crafter baseline must hold exactly one raw unnoted 1436 load of 27".into(),
                );
            }
        }
        MuleRole::Mule => {
            if observation.air_talisman > 0 {
                return Err("Air talisman is only on the crafter; mule baseline holds 1438".into());
            }
            if observation.essence_unnoted != MULE_TRADE_CAP {
                return Err(
                    "mule baseline must hold exactly one unnoted 1436 first load of 27".into(),
                );
            }
        }
    }
    Ok(())
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum MuleClaim {
    FirstExchangeCraft,
    MuleBankReturnSecondCycle,
}
fn debug_trade_edge_mule(
    actor: &str,
    previous: &Option<AirObservation>,
    observation: &AirObservation,
    stage_before: MuleExchangeStage,
    stage_after: MuleExchangeStage,
) {
    if std::env::var_os("BOT_DEBUG").is_none() {
        return;
    }
    let changed = previous.as_ref().is_none_or(|prev| {
        prev.trade_offer_open != observation.trade_offer_open
            || prev.trade_confirm_open != observation.trade_confirm_open
            || prev.trade_partner != observation.trade_partner
            || prev.trade_accept_id != observation.trade_accept_id
            || stage_before != stage_after
    });
    if changed {
        eprintln!(
            "PAIR_TRADE_EDGE actor={actor} tick={} offer={} confirm={} partner={} accept_id={} held_ess={} held_air={} stage={stage_before:?}->{stage_after:?}",
            observation.tick,
            observation.trade_offer_open,
            observation.trade_confirm_open,
            observation.trade_partner.as_deref().unwrap_or("<none>"),
            observation.trade_accept_id,
            observation.essence_unnoted,
            observation.air_runes,
        );
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum MuleExchangeStage {
    Offer,
    Confirm,
    Transfer,
}

#[derive(Debug, Clone, Serialize)]
pub struct MuleSlotRecord {
    pub role: MuleRole,
    pub account: String,
    pub expected_player: String,
    pub partner: String,
    pub settings: Map<String, Value>,
    pub baseline: AirObservation,
    pub latest: Option<AirObservation>,
    pub post_start: u32,
    pub mixed_identity: bool,
    pub saw_offer_with_partner: bool,
    pub saw_confirm_with_partner: bool,
    pub saw_wrong_partner: bool,
    pub peak_essence: i32,
    pub min_essence_after_start: i32,
    pub peak_air: i32,
    pub transferred_out: i32,
    pub transferred_in: i32,
    pub air_transferred_out: i32,
    pub air_transferred_in: i32,
    pub partner_transfer_events: u32,
    pub post_exchange_craft_events: u32,
    pub exchange_stage: MuleExchangeStage,
    pub episode_qty: i32,
    pub episode_qty_air: i32,
    pub confirm_gap_tick: Option<u32>,
    pub transfer_close_tick: Option<u32>,
    pub second_exchange_before_bank_return: bool,
    pub second_exchange_after_bank_return: bool,
    pub saw_bank_open_loaded: bool,
    pub saw_bank_at_falador: bool,
    pub restock_withdraw: bool,
    pub deposited_received_air: bool,
    pub returned_to_ruins: bool,
    pub craft_events: u32,
    pub craft_input_from_exchange: i32,
    pub craft_consumed: i32,
    pub craft_xp_at_consume: i32,
    pub craft_air_at_consume: i32,
    pub craft_from_exchange: bool,
    pub air_from_script: i32,
    pub xp_from_script: i32,
}

impl MuleSlotRecord {
    pub fn new(
        role: MuleRole,
        account: String,
        expected_player: String,
        partner: String,
        settings: Map<String, Value>,
        baseline: AirObservation,
    ) -> Self {
        Self {
            peak_essence: baseline.essence_unnoted,
            min_essence_after_start: baseline.essence_unnoted,
            peak_air: baseline.air_runes,
            role,
            account,
            expected_player,
            partner,
            settings,
            baseline,
            latest: None,
            post_start: 0,
            mixed_identity: false,
            saw_offer_with_partner: false,
            saw_confirm_with_partner: false,
            saw_wrong_partner: false,
            transferred_out: 0,
            transferred_in: 0,
            air_transferred_out: 0,
            air_transferred_in: 0,
            partner_transfer_events: 0,
            post_exchange_craft_events: 0,
            exchange_stage: MuleExchangeStage::Offer,
            episode_qty: 0,
            episode_qty_air: 0,
            confirm_gap_tick: None,
            transfer_close_tick: None,
            second_exchange_before_bank_return: false,
            second_exchange_after_bank_return: false,
            saw_bank_open_loaded: false,
            saw_bank_at_falador: false,
            restock_withdraw: false,
            deposited_received_air: false,
            returned_to_ruins: false,
            craft_events: 0,
            craft_input_from_exchange: 0,
            craft_consumed: 0,
            craft_xp_at_consume: 0,
            craft_air_at_consume: 0,
            craft_from_exchange: false,
            air_from_script: 0,
            xp_from_script: 0,
        }
    }

    fn drop_episode(&mut self) {
        self.episode_qty = 0;
        self.episode_qty_air = 0;
        self.confirm_gap_tick = None;
        self.transfer_close_tick = None;
        self.exchange_stage = MuleExchangeStage::Offer;
    }

    fn bind_episode(&mut self, observation: &AirObservation) {
        let mine_stale = self.latest.as_ref().is_some_and(|prev| {
            !prev.trade_offer_open
                && prev.trade_mine_essence == observation.trade_mine_essence
                && observation.trade_mine_essence > 0
        });
        let authoritative_mine = observation.trade_offer_open && !mine_stale;
        self.episode_qty = observation.essence_unnoted
            + if authoritative_mine {
                observation.trade_mine_essence
            } else {
                0
            };
        self.episode_qty_air = observation.air_runes;
        self.confirm_gap_tick = None;
        self.transfer_close_tick = None;
        self.saw_offer_with_partner = true;
        self.exchange_stage = MuleExchangeStage::Confirm;
    }

    fn partnerless_confirm(observation: &AirObservation) -> bool {
        observation.trade_confirm_open
            && observation
                .trade_partner
                .as_deref()
                .is_none_or(|partner| partner.is_empty())
    }

    pub fn observe(&mut self, observation: AirObservation) {
        self.post_start = self.post_start.saturating_add(1);
        if observation
            .player
            .as_deref()
            .is_some_and(|player| !account_identity_eq(player, &self.expected_player))
        {
            self.mixed_identity = true;
        }
        let prev_ess = self
            .latest
            .as_ref()
            .map(|row| row.essence_unnoted)
            .unwrap_or(self.baseline.essence_unnoted);
        let prev_air = self
            .latest
            .as_ref()
            .map(|row| row.air_runes)
            .unwrap_or(self.baseline.air_runes);
        let prev_xp = self
            .latest
            .as_ref()
            .map(|row| row.runecraft_xp)
            .unwrap_or(self.baseline.runecraft_xp);
        let at_ruins = near(observation.tile, AIR_RUINS, 8);
        let essence_in = (observation.essence_unnoted - prev_ess).max(0);
        let air_out = (prev_air - observation.air_runes).max(0);
        let named_partner = observation
            .trade_partner
            .as_deref()
            .is_some_and(|partner| account_identity_eq(partner, &self.partner));
        if observation.trade_active() {
            if let Some(partner) = observation.trade_partner.as_deref() {
                if !partner.is_empty() && !account_identity_eq(partner, &self.partner) {
                    self.saw_wrong_partner = true;
                }
            }
        }
        let offer_rising = observation.trade_offer_open
            && self
                .latest
                .as_ref()
                .is_none_or(|prev| !prev.trade_offer_open);
        let stage_before = self.exchange_stage;
        if self.saw_wrong_partner {
            self.drop_episode();
        } else if offer_rising {
            // A new offer ends the old episode even before its partner is known.
            self.drop_episode();
            self.craft_input_from_exchange = 0;
            self.craft_consumed = 0;
            self.craft_xp_at_consume = 0;
            self.craft_air_at_consume = 0;
            self.craft_from_exchange = false;
            if named_partner {
                self.bind_episode(&observation);
            }
        } else {
            match self.exchange_stage {
                MuleExchangeStage::Offer => {}
                MuleExchangeStage::Confirm => {
                    if Self::partnerless_confirm(&observation) {
                        self.drop_episode();
                    } else if observation.trade_confirm_open && named_partner {
                        match self.confirm_gap_tick {
                            None => {
                                self.saw_confirm_with_partner = true;
                                self.exchange_stage = MuleExchangeStage::Transfer;
                            }
                            Some(tick) if tick == observation.tick => {
                                self.saw_confirm_with_partner = true;
                                self.exchange_stage = MuleExchangeStage::Transfer;
                            }
                            Some(_) => self.drop_episode(),
                        }
                    } else if !observation.trade_active() {
                        match self.confirm_gap_tick {
                            None => self.confirm_gap_tick = Some(observation.tick),
                            Some(tick) if tick == observation.tick => {}
                            Some(_) => self.drop_episode(),
                        }
                    }
                }
                MuleExchangeStage::Transfer => {
                    if !observation.trade_active() {
                        if self.transfer_close_tick.is_none() {
                            self.transfer_close_tick = Some(observation.tick);
                        }
                        if self.transfer_close_tick != Some(observation.tick) {
                            self.drop_episode();
                        } else {
                            let essence_out =
                                (self.episode_qty - observation.essence_unnoted).max(0);
                            let essence_in =
                                (observation.essence_unnoted - self.episode_qty).max(0);
                            let air_out = (self.episode_qty_air - observation.air_runes).max(0);
                            let air_in = (observation.air_runes - self.episode_qty_air).max(0);
                            let completed = at_ruins
                                && match self.role {
                                    MuleRole::Crafter => essence_in > 0 && air_out > 0,
                                    MuleRole::Mule => essence_out > 0 && air_in > 0,
                                };
                            if completed {
                                if self.role == MuleRole::Mule && self.partner_transfer_events == 1
                                {
                                    if self.returned_to_ruins {
                                        self.second_exchange_after_bank_return = true;
                                    } else {
                                        self.second_exchange_before_bank_return = true;
                                    }
                                }
                                self.transferred_out += essence_out;
                                self.transferred_in += essence_in;
                                self.air_transferred_out += air_out;
                                self.air_transferred_in += air_in;
                                self.partner_transfer_events =
                                    self.partner_transfer_events.saturating_add(1);
                                if self.role == MuleRole::Crafter {
                                    self.craft_input_from_exchange =
                                        self.craft_input_from_exchange.saturating_add(essence_in);
                                }
                                self.drop_episode();
                            }
                        }
                    }
                }
            }
        }
        debug_trade_edge_mule(
            &self.account,
            &self.latest,
            &observation,
            stage_before,
            self.exchange_stage,
        );
        let expire_craft = !observation.ingame
            || self.mixed_identity
            || self.saw_wrong_partner
            || observation.bank_open;
        if self.role == MuleRole::Crafter {
            observe_runecraft_work(
                &observation,
                prev_ess,
                prev_air,
                prev_xp,
                expire_craft,
                &mut self.craft_input_from_exchange,
                &mut self.craft_consumed,
                &mut self.craft_xp_at_consume,
                &mut self.craft_air_at_consume,
                &mut self.craft_from_exchange,
                &mut self.craft_events,
                &mut self.post_exchange_craft_events,
                self.partner_transfer_events,
            );
        } else if expire_craft {
            self.craft_input_from_exchange = 0;
            self.craft_consumed = 0;
            self.craft_xp_at_consume = 0;
            self.craft_air_at_consume = 0;
            self.craft_from_exchange = false;
        }
        self.peak_essence = self.peak_essence.max(observation.essence_unnoted);
        self.peak_air = self.peak_air.max(observation.air_runes);
        self.min_essence_after_start = self
            .min_essence_after_start
            .min(observation.essence_unnoted);
        if observation.bank_open && observation.bank_loaded {
            self.saw_bank_open_loaded = true;
            if near(observation.tile, FALADOR_EAST, 8) {
                self.saw_bank_at_falador = true;
            }
        }
        if self.role == MuleRole::Mule
            && self.partner_transfer_events == 1
            && self.deposited_received_air
            && observation.bank_open
            && observation.bank_loaded
            && near(observation.tile, FALADOR_EAST, 8)
            && essence_in > 0
        {
            self.restock_withdraw = true;
        }
        if self.role == MuleRole::Mule
            && self.partner_transfer_events == 1
            && !self.deposited_received_air
            && observation.bank_open
            && observation.bank_loaded
            && near(observation.tile, FALADOR_EAST, 8)
            && air_out > 0
        {
            self.deposited_received_air = true;
        }
        if self.role == MuleRole::Mule
            && self.partner_transfer_events == 1
            && self.restock_withdraw
            && !observation.trade_active()
            && observation.essence_unnoted > 0
            && near(observation.tile, AIR_RUINS, 8)
        {
            self.returned_to_ruins = true;
        }
        self.air_from_script = self
            .air_from_script
            .max((observation.air_runes - self.baseline.air_runes).max(0));
        self.xp_from_script = (observation.runecraft_xp - self.baseline.runecraft_xp).max(0);
        self.latest = Some(observation);
    }
}
#[derive(Debug, Clone, Serialize)]
pub struct MulePairWitness {
    pub crafter: MuleSlotRecord,
    pub mule: MuleSlotRecord,
}

impl MulePairWitness {
    pub fn qualify_supported(&self) -> Result<MuleClaim, String> {
        self.qualify_common()?;
        if !self.crafter.saw_offer_with_partner || !self.mule.saw_offer_with_partner {
            return Err("one-sided confirmation: both actors never observed the offer phase with the partner".into());
        }
        if !self.crafter.saw_confirm_with_partner || !self.mule.saw_confirm_with_partner {
            return Err("one-sided confirmation: both actors never observed the confirm phase with the partner".into());
        }
        if self.crafter.partner_transfer_events == 0 || self.mule.partner_transfer_events == 0 {
            return Err(
                "missing conservation: no inventory transfer immediately followed a confirmed counterpart trade"
                    .into(),
            );
        }
        if self.mule.transferred_out <= 0 {
            return Err(
                "missing conservation: mule unnoted essence 1436 did not leave the pack".into(),
            );
        }
        if self.crafter.transferred_in <= 0 {
            return Err(
                "missing conservation: crafter did not receive the mule's unnoted 1436".into(),
            );
        }
        if self.crafter.transferred_in != self.mule.transferred_out {
            return Err(format!(
                "missing conservation: mule sent {} unnoted 1436, crafter received {}",
                self.mule.transferred_out, self.crafter.transferred_in
            ));
        }
        if self.crafter.air_transferred_out <= 0 || self.mule.air_transferred_in <= 0 {
            return Err(
                "missing conservation: crafted Air 556 did not leave the crafter and enter the mule"
                    .into(),
            );
        }
        if self.crafter.air_transferred_out != self.mule.air_transferred_in {
            return Err(format!(
                "missing conservation: crafter sent {} Air 556, mule received {}",
                self.crafter.air_transferred_out, self.mule.air_transferred_in
            ));
        }
        if self.crafter.air_from_script <= 0 {
            return Err(
                "seed-only inventory/XP: crafter Air 556 did not increase after Start".into(),
            );
        }
        if self.crafter.xp_from_script <= 0 {
            return Err(
                "seed-only inventory/XP: crafter Runecraft XP did not increase after Start".into(),
            );
        }
        if self.crafter.post_exchange_craft_events == 0 {
            return Err(
                "no fresh post-exchange work: seeded raw input/first craft cannot qualify the exchange"
                    .into(),
            );
        }
        if self.crafter.baseline.air_runes > 0 || self.mule.baseline.air_runes > 0 {
            return Err("seed-only inventory/XP: baseline already held Air 556".into());
        }
        if let Some(latest) = self.crafter.latest.as_ref() {
            if latest.trade_active() && latest.air_runes > self.crafter.baseline.air_runes {
                return Err(
                    "stale trade/duel state: crafter still has an open trade after claimed craft"
                        .into(),
                );
            }
        }
        Ok(MuleClaim::FirstExchangeCraft)
    }

    pub fn qualify_full_cycle(&self) -> Result<MuleClaim, String> {
        self.qualify_supported()?;
        if self.mule.second_exchange_before_bank_return {
            return Err("no further work for full-cycle claims: second transfer occurred before the mule bank deposit/restock/return".into());
        }
        let mule_held_script_air = self
            .mule
            .air_from_script
            .max((self.mule.peak_air - self.mule.baseline.air_runes).max(0));
        if mule_held_script_air <= 0 {
            return Err(
                "no mule bank deposit of received runes: mule never held script Air 556".into(),
            );
        }
        if !self.mule.deposited_received_air {
            return Err(
                "no mule bank deposit of received runes: mule did not deposit received 556 at Falador East"
                    .into(),
            );
        }
        if !self.mule.saw_bank_open_loaded || !self.mule.saw_bank_at_falador {
            return Err("no actual bank restock: mule never opened a loaded Falador East bank after the first exchange".into());
        }
        if !self.mule.restock_withdraw {
            return Err("no actual bank restock: mule pack 1436 did not refill from the bank after delivering the seed load".into());
        }
        if !self.mule.returned_to_ruins {
            return Err("no further work for full-cycle claims: mule did not return to the Air ruins after restock".into());
        }
        if self.crafter.partner_transfer_events < 2
            || self.mule.partner_transfer_events < 2
            || !self.mule.second_exchange_after_bank_return
        {
            return Err("no further work for full-cycle claims: no second transfer after a fresh counterpart offer/confirm and mule bank return".into());
        }
        if self.mule.transferred_out <= self.mule.baseline.essence_unnoted {
            return Err("no further work for full-cycle claims: only the seeded first load left the mule; no second transfer".into());
        }
        if self.crafter.transferred_in <= self.crafter.baseline.essence_unnoted {
            return Err(
                "no further work for full-cycle claims: crafter did not receive a second mule load"
                    .into(),
            );
        }
        if self.crafter.post_exchange_craft_events < 2 {
            return Err(
                "no further work for full-cycle claims: no fresh crafter craft after the second exchange"
                    .into(),
            );
        }
        Ok(MuleClaim::MuleBankReturnSecondCycle)
    }

    fn qualify_common(&self) -> Result<(), String> {
        mule_mode_requires_partner(self.mule.role, &self.mule.partner)?;
        if self.crafter.role != MuleRole::Crafter || self.mule.role != MuleRole::Mule {
            return Err("fixture miss: both Crafter is not a Crafter+Mule pair".into());
        }
        if account_identity_eq(&self.crafter.account, &self.mule.account) {
            return Err("mixed identities: crafter and mule share one account".into());
        }
        if self.crafter.mixed_identity || self.mule.mixed_identity {
            return Err("mixed identities: published player did not match the minted slot".into());
        }
        if self.crafter.post_start == 0 || self.mule.post_start == 0 {
            return Err("queued-only success: no post-Start observations".into());
        }
        if self.crafter.saw_wrong_partner || self.mule.saw_wrong_partner {
            return Err("wrong partner: trade header was not the minted counterpart".into());
        }
        if !account_identity_eq(&self.crafter.partner, &self.mule.expected_player)
            || !account_identity_eq(&self.mule.partner, &self.crafter.expected_player)
        {
            return Err("wrong partner: configured partner is not the minted counterpart".into());
        }
        if self.crafter.baseline.essence_noted > 0 || self.mule.baseline.essence_noted > 0 {
            return Err(
                "seed-only inventory/XP: noted essence 1437 is not accepted on Mule Air".into(),
            );
        }
        Ok(())
    }
}
pub fn mule_settings(
    schema: &[script::SettingDef],
    role: MuleRole,
    partner: &str,
) -> Map<String, Value> {
    let mut bag = Map::new();
    bag.insert("rune".into(), json!("Air rune"));
    bag.insert(
        "mode".into(),
        json!(match role {
            MuleRole::Crafter => "Crafter",
            MuleRole::Mule => "Mule",
        }),
    );
    bag.insert("partner".into(), json!(partner));
    bag.insert("bankFill".into(), json!(true));
    script::merge_bag(schema, &bag, None)
}
