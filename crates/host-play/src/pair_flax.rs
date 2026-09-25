use super::*;
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FlaxRole {
    Runner,
    Spinner,
}
pub fn flax_operation_gates() -> &'static [OperationGate] {
    &[
        OperationGate {
            source: "FlaxRunner.ts Runner/Spinner handoff",
            call: "Trade.request(playerName)",
            host_shape: "{ op: 'player', name, action: 'Trade' }",
            kind: GateKind::Mapped,
            owner: "this fixture observes; runtime already maps player Trade",
        },
        OperationGate {
            source: "FlaxRunner.ts driveActivePartnerTrade",
            call: "driveActivePartnerTrade({role, productNamesToOffer:['Flax']})",
            host_shape:
                "host-owned driver over Trade.active/offer/accept/decline; JS projects callbacks",
            kind: GateKind::Mapped,
            owner: "native 156; harness must not click Trade",
        },
        OperationGate {
            source: "FlaxRunner.ts Spinner conversion",
            call: "ChatDialog.makeX('Flax', flaxCount())",
            host_shape: "posted Make-X qty -1; do not guess comId",
            kind: GateKind::Mapped,
            owner: "Make-X t_4b04cb5f; full cycle LIVE is root-owned",
        },
        OperationGate {
            source: "FlaxRunner.ts Spinner bank",
            call: "Bank.depositInventory at Seers stand (2725,3493,0)",
            host_shape: "existing Bank.depositInventory",
            kind: GateKind::Mapped,
            owner: "this fixture; first flax pack is pick, not restock",
        },
        OperationGate {
            source: "FlaxRunner.ts Runner pick",
            call: "pick flax 1779 at (2741,3444,0)",
            host_shape: "existing loc pick; empty pack at Start",
            kind: GateKind::Mapped,
            owner: "this fixture; seeded 1779/1777 cannot qualify",
        },
        OperationGate {
            source: "FlaxRunner.ts towardDest / isOpenableObstacle / FlaxAIO",
            call: "unused stubs / one-actor pick+spin",
            host_shape: "n/a",
            kind: GateKind::UnusedByCase,
            owner: "ancillary stubs stay stubs; FlaxAIO is a different card",
        },
    ]
}
pub fn flax_prepared_current(
    role: FlaxRole,
    expected_player: &str,
    observation: &FlaxObservation,
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
    if observation.bow_string > 0 {
        return Err("Start baseline already has bow string 1777".into());
    }
    if observation.flax > 0 {
        return Err(
            "Start baseline already holds flax 1779; first pack must be picked after Start".into(),
        );
    }
    if observation.trade_active() {
        return Err("Start baseline already has a trade window".into());
    }
    if observation.bank_open {
        return Err("Start baseline still has an open bank".into());
    }
    match role {
        FlaxRole::Runner => {
            if !near(observation.tile, FLAX_FIELD, 8) {
                return Err(format!(
                    "runner baseline is not at the flax field: {:?}",
                    observation.tile
                ));
            }
        }
        FlaxRole::Spinner => {
            if observation.crafting < 10 {
                return Err("spinner baseline Crafting must be at least 10".into());
            }
            if !near(observation.tile, FLAX_MEET, 8) && !near(observation.tile, FLAX_WHEEL, 8) {
                return Err(format!(
                    "spinner baseline is not at the meet/wheel house: {:?}",
                    observation.tile
                ));
            }
        }
    }
    Ok(())
}
pub fn flax_settings(
    schema: &[script::SettingDef],
    role: FlaxRole,
    partner: &str,
) -> Map<String, Value> {
    let mut bag = Map::new();
    bag.insert(
        "mode".into(),
        json!(match role {
            FlaxRole::Runner => "Runner",
            FlaxRole::Spinner => "Spinner",
        }),
    );
    bag.insert("partner".into(), json!(partner));
    bag.insert("minFlaxCapacity".into(), json!(FLAX_MIN_CAPACITY));
    script::merge_bag(schema, &bag, None)
}
#[derive(Debug, Clone, Default, Serialize)]
pub struct FlaxObservation {
    pub ingame: bool,
    pub scene_state: i32,
    pub inventory_tab_available: bool,
    pub player: Option<String>,
    pub tile: Option<(i32, i32, i32)>,
    pub tick: u32,
    pub crafting: i32,
    pub crafting_xp: i32,
    pub flax: i32,
    pub bow_string: i32,
    pub bank_open: bool,
    pub bank_loaded: bool,
    pub bank_session_generation: u64,
    pub bank_string: i32,
    pub trade_offer_open: bool,
    pub trade_confirm_open: bool,
    pub trade_partner: Option<String>,
    pub trade_accept_id: i32,
    pub trade_mine_flax: i32,
    pub trade_theirs_flax: i32,
}

impl FlaxObservation {
    pub fn from_snapshot(snapshot: &GameSnapshot) -> Self {
        let tile = snapshot.tile();
        let trade = snapshot.trade();
        Self {
            ingame: snapshot.ingame() && snapshot.attached(),
            scene_state: snapshot.scene_state(),
            inventory_tab_available: snapshot
                .side_tabs()
                .iter()
                .any(|tab| tab.index == 3 && tab.available),
            player: snapshot
                .local_player()
                .and_then(|local| local.player.actor.name.clone()),
            tile,
            tick: snapshot.tick(),
            crafting: stat(snapshot, "crafting").map(|row| row.base).unwrap_or(0),
            crafting_xp: stat(snapshot, "crafting").map(|row| row.xp).unwrap_or(0),
            flax: count_id(snapshot.inventory(), FLAX_ID),
            bow_string: count_id(snapshot.inventory(), BOW_STRING_ID),
            bank_open: snapshot.bank_component_id() >= 0,
            bank_loaded: snapshot.bank_loaded(),
            bank_session_generation: snapshot.bank_session_generation(),
            bank_string: count_id(snapshot.bank(), BOW_STRING_ID),
            trade_offer_open: trade.offer_open,
            trade_confirm_open: trade.confirm_open,
            trade_partner: trade.partner.clone(),
            trade_accept_id: trade.accept_component_id,
            trade_mine_flax: count_id(&trade.my_offer, FLAX_ID),
            trade_theirs_flax: count_id(&trade.their_offer, FLAX_ID),
        }
    }

    pub fn trade_active(&self) -> bool {
        self.trade_offer_open || self.trade_confirm_open
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum FlaxClaim {
    FirstFlaxTransfer,
    SpinBankSecondDelivery,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum FlaxExchangeStage {
    Offer,
    Confirm,
    Transfer,
}

#[derive(Debug, Clone, Serialize)]
pub struct FlaxSlotRecord {
    pub role: FlaxRole,
    pub account: String,
    pub expected_player: String,
    pub partner: String,
    pub settings: Map<String, Value>,
    pub baseline: FlaxObservation,
    pub latest: Option<FlaxObservation>,
    pub post_start: u32,
    pub mixed_identity: bool,
    pub saw_offer_with_partner: bool,
    pub saw_confirm_with_partner: bool,
    pub saw_wrong_partner: bool,
    pub transferred_out: i32,
    pub transferred_in: i32,
    pub partner_transfer_events: u32,
    pub exchange_stage: FlaxExchangeStage,
    pub episode_qty: i32,
    pub confirm_gap_tick: Option<u32>,
    pub transfer_close_tick: Option<u32>,
    pub second_delivery_before_bank_return: bool,
    pub second_delivery_after_bank_return: bool,
    pub saw_bank_open_loaded: bool,
    pub saw_bank_at_seers: bool,
    pub deposited_strings: bool,
    pub returned_to_meet: bool,
    pub string_from_script: i32,
    pub xp_from_script: i32,
    pub spin_events: u32,
}

impl FlaxSlotRecord {
    pub fn new(
        role: FlaxRole,
        account: String,
        expected_player: String,
        partner: String,
        settings: Map<String, Value>,
        baseline: FlaxObservation,
    ) -> Self {
        Self {
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
            exchange_stage: FlaxExchangeStage::Offer,
            episode_qty: 0,
            confirm_gap_tick: None,
            transfer_close_tick: None,
            second_delivery_before_bank_return: false,
            second_delivery_after_bank_return: false,
            saw_bank_open_loaded: false,
            saw_bank_at_seers: false,
            deposited_strings: false,
            returned_to_meet: false,
            string_from_script: 0,
            xp_from_script: 0,
            spin_events: 0,
        }
    }

    fn drop_episode(&mut self) {
        self.episode_qty = 0;
        self.confirm_gap_tick = None;
        self.transfer_close_tick = None;
        self.exchange_stage = FlaxExchangeStage::Offer;
    }

    fn bind_episode(&mut self, observation: &FlaxObservation) {
        let mine_stale = self.latest.as_ref().is_some_and(|prev| {
            !prev.trade_offer_open
                && prev.trade_mine_flax == observation.trade_mine_flax
                && observation.trade_mine_flax > 0
        });
        let authoritative_mine = observation.trade_offer_open && !mine_stale;
        self.episode_qty = observation.flax
            + if authoritative_mine {
                observation.trade_mine_flax
            } else {
                0
            };
        self.confirm_gap_tick = None;
        self.transfer_close_tick = None;
        self.saw_offer_with_partner = true;
        self.exchange_stage = FlaxExchangeStage::Confirm;
    }

    fn partnerless_confirm(observation: &FlaxObservation) -> bool {
        observation.trade_confirm_open
            && observation
                .trade_partner
                .as_deref()
                .is_none_or(|partner| partner.is_empty())
    }

    pub fn observe(&mut self, observation: FlaxObservation) {
        self.post_start = self.post_start.saturating_add(1);
        if observation
            .player
            .as_deref()
            .is_some_and(|player| !account_identity_eq(player, &self.expected_player))
        {
            self.mixed_identity = true;
        }
        let prev_flax = self
            .latest
            .as_ref()
            .map(|row| row.flax)
            .unwrap_or(self.baseline.flax);
        let prev_string = self
            .latest
            .as_ref()
            .map(|row| row.bow_string)
            .unwrap_or(self.baseline.bow_string);
        let prev_xp = self
            .latest
            .as_ref()
            .map(|row| row.crafting_xp)
            .unwrap_or(self.baseline.crafting_xp);
        let flax_out = (prev_flax - observation.flax).max(0);
        let string_out = (prev_string - observation.bow_string).max(0);
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
            if named_partner {
                self.bind_episode(&observation);
            }
        } else {
            match self.exchange_stage {
                FlaxExchangeStage::Offer => {}
                FlaxExchangeStage::Confirm => {
                    if Self::partnerless_confirm(&observation) {
                        self.drop_episode();
                    } else if observation.trade_confirm_open && named_partner {
                        match self.confirm_gap_tick {
                            None => {
                                self.saw_confirm_with_partner = true;
                                self.exchange_stage = FlaxExchangeStage::Transfer;
                            }
                            Some(tick) if tick == observation.tick => {
                                self.saw_confirm_with_partner = true;
                                self.exchange_stage = FlaxExchangeStage::Transfer;
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
                FlaxExchangeStage::Transfer => {
                    if !observation.trade_active() {
                        if self.transfer_close_tick.is_none() {
                            self.transfer_close_tick = Some(observation.tick);
                        }
                        if self.transfer_close_tick != Some(observation.tick) {
                            self.drop_episode();
                        } else {
                            let at_meet = near(observation.tile, FLAX_MEET, 8);
                            let flax_out = (self.episode_qty - observation.flax).max(0);
                            let flax_in = (observation.flax - self.episode_qty).max(0);
                            let completed = at_meet
                                && match self.role {
                                    FlaxRole::Runner => flax_out > 0,
                                    FlaxRole::Spinner => flax_in > 0,
                                };
                            if completed {
                                if self.role == FlaxRole::Spinner
                                    && self.partner_transfer_events == 1
                                {
                                    if self.returned_to_meet {
                                        self.second_delivery_after_bank_return = true;
                                    } else {
                                        self.second_delivery_before_bank_return = true;
                                    }
                                }
                                self.transferred_out += flax_out;
                                self.transferred_in += flax_in;
                                self.partner_transfer_events =
                                    self.partner_transfer_events.saturating_add(1);
                                self.drop_episode();
                            }
                        }
                    }
                }
            }
        }
        debug_trade_edge_flax(
            &self.account,
            &self.latest,
            &observation,
            stage_before,
            self.exchange_stage,
        );
        if self.role == FlaxRole::Spinner
            && flax_out > 0
            && observation.crafting_xp > prev_xp
            && observation.bow_string > prev_string
        {
            self.spin_events = self.spin_events.saturating_add(1);
        }
        if observation.bank_open && observation.bank_loaded {
            self.saw_bank_open_loaded = true;
            if near(observation.tile, FLAX_BANK, 8) {
                self.saw_bank_at_seers = true;
            }
        }
        if self.role == FlaxRole::Spinner
            && self.partner_transfer_events >= 1
            && self.spin_events >= 1
            && observation.bank_open
            && observation.bank_loaded
            && near(observation.tile, FLAX_BANK, 8)
            && string_out > 0
        {
            self.deposited_strings = true;
        }
        if self.role == FlaxRole::Spinner
            && self.deposited_strings
            && !observation.bank_open
            && !observation.trade_active()
            && near(observation.tile, FLAX_MEET, 8)
        {
            self.returned_to_meet = true;
        }
        self.string_from_script = (observation.bow_string - self.baseline.bow_string)
            .max(0)
            .max(self.string_from_script);
        self.xp_from_script = (observation.crafting_xp - self.baseline.crafting_xp).max(0);
        self.latest = Some(observation);
    }
}
fn debug_trade_edge_flax(
    actor: &str,
    previous: &Option<FlaxObservation>,
    observation: &FlaxObservation,
    stage_before: FlaxExchangeStage,
    stage_after: FlaxExchangeStage,
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
            "PAIR_TRADE_EDGE actor={actor} tick={} offer={} confirm={} partner={} accept_id={} held_flax={} held_string={} stage={stage_before:?}->{stage_after:?}",
            observation.tick,
            observation.trade_offer_open,
            observation.trade_confirm_open,
            observation.trade_partner.as_deref().unwrap_or("<none>"),
            observation.trade_accept_id,
            observation.flax,
            observation.bow_string,
        );
    }
}
#[derive(Debug, Clone, Serialize)]
pub struct FlaxPairWitness {
    pub runner: FlaxSlotRecord,
    pub spinner: FlaxSlotRecord,
}

impl FlaxPairWitness {
    pub fn qualify_supported(&self) -> Result<FlaxClaim, String> {
        self.qualify_common()?;
        if !self.runner.saw_offer_with_partner || !self.spinner.saw_offer_with_partner {
            return Err("one-sided confirmation: both actors never observed the offer phase with the partner".into());
        }
        if !self.runner.saw_confirm_with_partner || !self.spinner.saw_confirm_with_partner {
            return Err("one-sided confirmation: both actors never observed the confirm phase with the partner".into());
        }
        if self.runner.partner_transfer_events == 0 || self.spinner.partner_transfer_events == 0 {
            return Err(
                "missing conservation: no flax transfer immediately followed a confirmed counterpart trade"
                    .into(),
            );
        }
        if self.runner.transferred_out <= 0 {
            return Err("missing conservation: runner flax 1779 did not leave the pack".into());
        }
        if self.spinner.transferred_in <= 0 {
            return Err("missing conservation: spinner did not receive flax 1779".into());
        }
        if self.runner.transferred_out != self.spinner.transferred_in {
            return Err(format!(
                "missing conservation: runner sent {} flax 1779, spinner received {}",
                self.runner.transferred_out, self.spinner.transferred_in
            ));
        }
        Ok(FlaxClaim::FirstFlaxTransfer)
    }

    pub fn qualify_full_cycle(&self) -> Result<FlaxClaim, String> {
        self.qualify_supported()?;
        if self.spinner.second_delivery_before_bank_return {
            return Err("no further work for full-cycle claims: second delivery occurred before spinner bank deposit/return".into());
        }
        if self.spinner.string_from_script <= 0 || self.spinner.xp_from_script <= 0 {
            return Err(
                "seed-only inventory/XP: spinner bow string 1777 / Crafting XP did not increase after Start"
                    .into(),
            );
        }
        if self.spinner.spin_events == 0 {
            return Err(
                "no fresh post-transfer work: seeded strings cannot qualify Make-X conversion"
                    .into(),
            );
        }
        if !self.spinner.deposited_strings
            || !self.spinner.saw_bank_open_loaded
            || !self.spinner.saw_bank_at_seers
        {
            return Err(
                "no actual bank restock: spinner never deposited strings at Seers after the first spin"
                    .into(),
            );
        }
        if !self.spinner.returned_to_meet {
            return Err(
                "no further work for full-cycle claims: spinner did not return to the meet after deposit"
                    .into(),
            );
        }
        if self.runner.partner_transfer_events < 2
            || self.spinner.partner_transfer_events < 2
            || !self.spinner.second_delivery_after_bank_return
        {
            return Err("no further work for full-cycle claims: no second flax delivery after spinner bank return".into());
        }
        Ok(FlaxClaim::SpinBankSecondDelivery)
    }

    fn qualify_common(&self) -> Result<(), String> {
        if self.runner.role != FlaxRole::Runner || self.spinner.role != FlaxRole::Spinner {
            return Err("fixture miss: both Runner is not a Runner+Spinner pair".into());
        }
        if account_identity_eq(&self.runner.account, &self.spinner.account) {
            return Err("mixed identities: runner and spinner share one account".into());
        }
        if self.runner.mixed_identity || self.spinner.mixed_identity {
            return Err("mixed identities: published player did not match the minted slot".into());
        }
        if self.runner.post_start == 0 || self.spinner.post_start == 0 {
            return Err("queued-only success: no post-Start observations".into());
        }
        if self.runner.saw_wrong_partner || self.spinner.saw_wrong_partner {
            return Err("wrong partner: trade header was not the minted counterpart".into());
        }
        if !account_identity_eq(&self.runner.partner, &self.spinner.expected_player)
            || !account_identity_eq(&self.spinner.partner, &self.runner.expected_player)
        {
            return Err("wrong partner: configured partner is not the minted counterpart".into());
        }
        if self.runner.baseline.bow_string > 0 || self.spinner.baseline.bow_string > 0 {
            return Err("seed-only inventory/XP: baseline already held bow string 1777".into());
        }
        if self.runner.baseline.flax > 0 || self.spinner.baseline.flax > 0 {
            return Err("seed-only inventory/XP: baseline already held flax 1779".into());
        }
        Ok(())
    }
}