use super::*;
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AirRole {
    Master,
    Runner,
}
pub fn air_operation_gates() -> &'static [OperationGate] {
    &[
        OperationGate {
            source: "NatureCrafter.ts DriveTrade/DeliverEssence/AcceptRunner",
            call: "Trade.request(playerName)",
            host_shape: "{ op: 'player', name, action: 'Trade' }",
            kind: GateKind::Mapped,
            owner: "this fixture observes; runtime already maps player Trade",
        },
        OperationGate {
            source: "NatureCrafter.ts DriveTrade",
            call: "Trade.offerAll('Rune essence', i => i.id === 1436) / Trade.offer(name, n, filter)",
            host_shape: "shim Trade.offer(name) / offerAll(name) ignore qty and id filter; press trade_side name rows",
            kind: GateKind::ArityLimited,
            owner: "Air shortRouteWithdraw caps at TRADE_CAP 25 so offerAll name path is used; qty/filter arity is not implemented here",
        },
        OperationGate {
            source: "NatureCrafter.ts HandleOpenTrade/DriveTrade",
            call: "Trade.accept() / Trade.decline()",
            host_shape: "if-button trade_accept_id / trade_decline_id; throws notImpl when id < 0",
            kind: GateKind::Mapped,
            owner: "this fixture requires both offer and confirm accepts with posted ids",
        },
        OperationGate {
            source: "NatureCrafter.ts BankRestock",
            call: "Bank.openBooth(runnerBank Tile(3013,3355,0), 'Bank booth', 'Use-quickly')",
            host_shape: "walk-near stand then open-booth exact name/op; not BANK_LOCATIONS.find",
            kind: GateKind::Mapped,
            owner: "named-bank aliases t_bced5c76 are unused by this Air tile path",
        },
        OperationGate {
            source: "NatureCrafter.ts BankRestock",
            call: "Bank.withdrawX('Rune essence', want) after Bank.loaded()",
            host_shape: "existing Bank.withdrawX + count-dialog; Air setNoteMode(false)",
            kind: GateKind::Mapped,
            owner: "this fixture",
        },
        OperationGate {
            source: "NatureCrafter.ts enterAltar / CraftNatures",
            call: "talisman.useOn(Mysterious ruins) / Altar.interact('Craft-rune')",
            host_shape: "existing item-on-loc and loc interact",
            kind: GateKind::Mapped,
            owner: "this fixture",
        },
        OperationGate {
            source: "NatureCrafter.ts UnNoteEssence / openUnnoteShop",
            call: "Shop.sell/buy + ChatDialog + Jiminua Talk-to",
            host_shape: "Shop family",
            kind: GateKind::UnusedByCase,
            owner: "shop t_1591d140; Air unnote=null — Nature island/unnoting/boat not accepted",
        },
        OperationGate {
            source: "NatureCrafter.ts walkTo",
            call: "Traversal.walkResilient(ruins|runnerBank)",
            host_shape: "existing walk-near / resilient walk; not Game.teleport",
            kind: GateKind::Mapped,
            owner: "teleport t_1bf9a22e unused; prep tele is pre-Start seed only",
        },
        OperationGate {
            source: "NatureRunnerLogic.ts RUNES['Air runes']",
            call: "BANK_LOCATIONS named Falador East",
            host_shape: "host content.named_banks",
            kind: GateKind::UnusedByCase,
            owner: "named-bank t_bced5c76; Air uses hardcoded Tile(3013,3355,0)",
        },
    ]
}
pub fn bank_seed_acknowledged(observation: &AirObservation, min_count: i32) -> bool {
    observation.ingame
        && observation.scene_state == 2
        && observation.bank_open
        && observation.bank_loaded
        && observation.bank_essence_unnoted >= min_count
        && near(observation.tile, FALADOR_EAST, 8)
}

pub fn bank_ack_target_absence(
    tile: Option<(i32, i32, i32)>,
    booth_present: bool,
) -> Option<String> {
    if !near(tile, FALADOR_EAST, 8) {
        Some(format!(
            "no Falador East booth in loaded scene at stand {FALADOR_EAST:?}; actor tile {tile:?}"
        ))
    } else if !booth_present {
        Some(format!(
            "no Falador East Use-quickly booth in loaded scene at stand {FALADOR_EAST:?}"
        ))
    } else {
        None
    }
}
pub fn air_prepared_current(
    role: AirRole,
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
        return Err("Start baseline has noted essence 1437; Air does not accept noting".into());
    }
    if observation.trade_active() {
        return Err("Start baseline already has a trade window".into());
    }
    if observation.bank_open {
        return Err("Start baseline still has an open bank".into());
    }
    match role {
        AirRole::Master => {
            if observation.air_talisman <= 0 {
                return Err("master baseline has no Air talisman".into());
            }
            if observation.essence_unnoted > 0 {
                return Err("master baseline already holds unnoted essence".into());
            }
        }
        AirRole::Runner => {
            if observation.essence_unnoted < TRADE_CAP {
                return Err("runner baseline has no seeded unnoted 1436 first load of 25".into());
            }
        }
    }
    Ok(())
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum AirClaim {
    FirstTransferCraft,
    BankReturnSecondCycle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum AirExchangeStage {
    Offer,
    Confirm,
    Transfer,
}

#[derive(Debug, Clone, Serialize)]
pub struct AirSlotRecord {
    pub role: AirRole,
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
    pub transferred_out: i32,
    pub transferred_in: i32,
    pub partner_transfer_events: u32,
    pub post_transfer_craft_events: u32,
    pub exchange_stage: AirExchangeStage,
    pub episode_qty: i32,
    pub confirm_gap_tick: Option<u32>,
    pub transfer_close_tick: Option<u32>,
    pub second_transfer_before_bank_return: bool,
    pub second_transfer_after_bank_return: bool,
    pub saw_bank_open_loaded: bool,
    pub saw_bank_at_falador: bool,
    pub restock_withdraw: bool,
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

impl AirSlotRecord {
    pub fn new(
        role: AirRole,
        account: String,
        expected_player: String,
        partner: String,
        settings: Map<String, Value>,
        baseline: AirObservation,
    ) -> Self {
        Self {
            peak_essence: baseline.essence_unnoted,
            min_essence_after_start: baseline.essence_unnoted,
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
            partner_transfer_events: 0,
            post_transfer_craft_events: 0,
            exchange_stage: AirExchangeStage::Offer,
            episode_qty: 0,
            confirm_gap_tick: None,
            transfer_close_tick: None,
            second_transfer_before_bank_return: false,
            second_transfer_after_bank_return: false,
            saw_bank_open_loaded: false,
            saw_bank_at_falador: false,
            restock_withdraw: false,
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
        self.confirm_gap_tick = None;
        self.transfer_close_tick = None;
        self.exchange_stage = AirExchangeStage::Offer;
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
        self.confirm_gap_tick = None;
        self.transfer_close_tick = None;
        self.saw_offer_with_partner = true;
        self.exchange_stage = AirExchangeStage::Confirm;
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
            expire_runecraft_work(self);
            if named_partner {
                self.bind_episode(&observation);
            }
        } else {
            match self.exchange_stage {
                AirExchangeStage::Offer => {}
                AirExchangeStage::Confirm => {
                    if Self::partnerless_confirm(&observation) {
                        self.drop_episode();
                    } else if observation.trade_confirm_open && named_partner {
                        match self.confirm_gap_tick {
                            None => {
                                self.saw_confirm_with_partner = true;
                                self.exchange_stage = AirExchangeStage::Transfer;
                            }
                            Some(tick) if tick == observation.tick => {
                                self.saw_confirm_with_partner = true;
                                self.exchange_stage = AirExchangeStage::Transfer;
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
                AirExchangeStage::Transfer => {
                    if !observation.trade_active() {
                        if self.transfer_close_tick.is_none() {
                            self.transfer_close_tick = Some(observation.tick);
                        }
                        if self.transfer_close_tick != Some(observation.tick) {
                            self.drop_episode();
                        } else {
                            let at_ruins = near(observation.tile, AIR_RUINS, 8);
                            let essence_out =
                                (self.episode_qty - observation.essence_unnoted).max(0);
                            let essence_in =
                                (observation.essence_unnoted - self.episode_qty).max(0);
                            let completed = at_ruins
                                && match self.role {
                                    AirRole::Runner => essence_out > 0,
                                    AirRole::Master => essence_in > 0,
                                };
                            if completed {
                                if self.role == AirRole::Runner && self.partner_transfer_events == 1
                                {
                                    if self.returned_to_ruins {
                                        self.second_transfer_after_bank_return = true;
                                    } else {
                                        self.second_transfer_before_bank_return = true;
                                    }
                                }
                                self.transferred_out += essence_out;
                                self.transferred_in += essence_in;
                                self.partner_transfer_events =
                                    self.partner_transfer_events.saturating_add(1);
                                if self.role == AirRole::Master {
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
        debug_trade_edge_air(
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
        if self.role == AirRole::Master {
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
                &mut self.post_transfer_craft_events,
                self.partner_transfer_events,
            );
        } else if expire_craft {
            expire_runecraft_work(self);
        }
        self.peak_essence = self.peak_essence.max(observation.essence_unnoted);
        self.min_essence_after_start = self
            .min_essence_after_start
            .min(observation.essence_unnoted);
        if observation.bank_open && observation.bank_loaded {
            self.saw_bank_open_loaded = true;
            if near(observation.tile, FALADOR_EAST, 8) {
                self.saw_bank_at_falador = true;
            }
        }
        if self.transferred_out > 0
            && observation.bank_open
            && observation.bank_loaded
            && near(observation.tile, FALADOR_EAST, 8)
            && observation.essence_unnoted > 0
            && observation.essence_unnoted > self.min_essence_after_start
        {
            self.restock_withdraw = true;
        }
        if self.restock_withdraw && near(observation.tile, AIR_RUINS, 8) {
            self.returned_to_ruins = true;
        }
        self.air_from_script = (observation.air_runes - self.baseline.air_runes).max(0);
        self.xp_from_script = (observation.runecraft_xp - self.baseline.runecraft_xp).max(0);
        self.latest = Some(observation);
    }
}

fn expire_runecraft_work(slot: &mut AirSlotRecord) {
    slot.craft_input_from_exchange = 0;
    slot.craft_consumed = 0;
    slot.craft_xp_at_consume = 0;
    slot.craft_air_at_consume = 0;
    slot.craft_from_exchange = false;
}
fn debug_trade_edge_air(
    actor: &str,
    previous: &Option<AirObservation>,
    observation: &AirObservation,
    stage_before: AirExchangeStage,
    stage_after: AirExchangeStage,
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
#[derive(Debug, Clone, Serialize)]
pub struct AirPairWitness {
    pub master: AirSlotRecord,
    pub runner: AirSlotRecord,
}

impl AirPairWitness {
    pub fn qualify_supported(&self) -> Result<AirClaim, String> {
        self.qualify_common()?;
        if !self.master.saw_offer_with_partner || !self.runner.saw_offer_with_partner {
            return Err("one-sided confirmation: both actors never observed the offer phase with the partner".into());
        }
        if !self.master.saw_confirm_with_partner || !self.runner.saw_confirm_with_partner {
            return Err("one-sided confirmation: both actors never observed the confirm phase with the partner".into());
        }
        if self.master.partner_transfer_events == 0 || self.runner.partner_transfer_events == 0 {
            return Err(
                "missing conservation: no inventory transfer immediately followed a confirmed counterpart trade"
                    .into(),
            );
        }
        if self.runner.transferred_out <= 0 {
            return Err(
                "missing conservation: runner unnoted essence 1436 did not leave the pack".into(),
            );
        }
        if self.master.transferred_in <= 0 {
            return Err("missing conservation: master did not receive unnoted 1436".into());
        }
        if self.master.transferred_in != self.runner.transferred_out {
            return Err(format!(
                "missing conservation: runner sent {} unnoted 1436, master received {}",
                self.runner.transferred_out, self.master.transferred_in
            ));
        }
        if self.master.air_from_script <= 0 {
            return Err(
                "seed-only inventory/XP: master Air 556 did not increase after Start".into(),
            );
        }
        if self.master.xp_from_script <= 0 {
            return Err(
                "seed-only inventory/XP: master Runecraft XP did not increase after Start".into(),
            );
        }
        if self.master.post_transfer_craft_events == 0 {
            return Err(
                "no fresh post-exchange work: seeded raw input/first craft cannot qualify the exchange"
                    .into(),
            );
        }
        if self.master.baseline.air_runes > 0 {
            return Err("seed-only inventory/XP: master baseline already held Air 556".into());
        }
        if let Some(latest) = self.master.latest.as_ref() {
            if latest.trade_active() && latest.air_runes > self.master.baseline.air_runes {
                return Err(
                    "stale trade/duel state: master still has an open trade after claimed craft"
                        .into(),
                );
            }
        }
        Ok(AirClaim::FirstTransferCraft)
    }

    pub fn qualify_full_cycle(&self) -> Result<AirClaim, String> {
        self.qualify_supported()?;
        if !self.runner.saw_bank_open_loaded || !self.runner.saw_bank_at_falador {
            return Err("no actual bank restock: runner never opened a loaded Falador East bank after the first transfer".into());
        }
        if !self.runner.restock_withdraw {
            return Err("no actual bank restock: runner pack 1436 did not refill from the bank after delivering the seed load".into());
        }
        if !self.runner.returned_to_ruins {
            return Err("no further work for full-cycle claims: runner did not return to the Air ruins after restock".into());
        }
        if self.runner.second_transfer_before_bank_return {
            return Err("no further work for full-cycle claims: second transfer occurred before the runner bank restock/return".into());
        }
        if self.master.partner_transfer_events < 2
            || self.runner.partner_transfer_events < 2
            || !self.runner.second_transfer_after_bank_return
        {
            return Err("no further work for full-cycle claims: no second transfer after a fresh counterpart offer/confirm and runner bank return".into());
        }
        if self.runner.transferred_out <= self.runner.baseline.essence_unnoted {
            return Err("no further work for full-cycle claims: only the seeded first load left the runner; no second transfer".into());
        }
        if self.master.post_transfer_craft_events < 2 {
            return Err(
                "no further work for full-cycle claims: no second master craft after restock"
                    .into(),
            );
        }
        Ok(AirClaim::BankReturnSecondCycle)
    }

    fn qualify_common(&self) -> Result<(), String> {
        if account_identity_eq(&self.master.account, &self.runner.account) {
            return Err("mixed identities: master and runner share one account".into());
        }
        if self.master.mixed_identity || self.runner.mixed_identity {
            return Err("mixed identities: published player did not match the minted slot".into());
        }
        if self.master.post_start == 0 || self.runner.post_start == 0 {
            return Err("queued-only success: no post-Start observations".into());
        }
        if self.master.saw_wrong_partner || self.runner.saw_wrong_partner {
            return Err("wrong partner: trade header was not the minted counterpart".into());
        }
        if !account_identity_eq(&self.master.partner, &self.runner.expected_player)
            || !account_identity_eq(&self.runner.partner, &self.master.expected_player)
        {
            return Err("wrong partner: configured partner is not the minted counterpart".into());
        }
        if self.master.baseline.essence_noted > 0 || self.runner.baseline.essence_noted > 0 {
            return Err("seed-only inventory/XP: noted essence 1437 is not accepted on Air".into());
        }
        Ok(())
    }
}
pub fn air_settings(
    schema: &[script::SettingDef],
    role: AirRole,
    partner: &str,
) -> Map<String, Value> {
    let mut bag = Map::new();
    bag.insert("rune".into(), json!("Air runes"));
    bag.insert(
        "mode".into(),
        json!(match role {
            AirRole::Master => "Master",
            AirRole::Runner => "Runner",
        }),
    );
    bag.insert("partner".into(), json!(partner));
    script::merge_bag(schema, &bag, None)
}
