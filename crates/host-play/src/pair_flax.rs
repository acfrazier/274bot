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
    pub trade_interface_generation: u64,
    pub trade_modal_generation: u64,
    pub trade_inventory_generation: u64,
    pub trade_session_generation: u64,
    pub trade_my_offer_packet_generation: Option<u64>,
    pub trade_their_offer_packet_generation: Option<u64>,
    pub trade_my_offer_available: bool,
    pub trade_their_offer_available: bool,
    pub trade_my_offer_stopped: bool,
    pub trade_their_offer_stopped: bool,
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
            trade_interface_generation: trade.interface_generation,
            trade_modal_generation: trade.modal_generation,
            trade_inventory_generation: trade.inventory_generation,
            trade_session_generation: trade.session_generation,
            trade_my_offer_packet_generation: trade.my_offer_packet_generation,
            trade_their_offer_packet_generation: trade.their_offer_packet_generation,
            trade_my_offer_available: trade.my_offer_available,
            trade_their_offer_available: trade.their_offer_available,
            trade_my_offer_stopped: trade.my_offer_stopped,
            trade_their_offer_stopped: trade.their_offer_stopped,
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
    pub identity_unverified: bool,
    pub session_mismatch: bool,
    pub saw_offer_with_partner: bool,
    pub saw_confirm_with_partner: bool,
    pub saw_wrong_partner: bool,
    pub transferred_out: i32,
    pub transferred_in: i32,
    pub partner_transfer_events: u32,
    pub exchange_outcomes: Vec<i32>,
    pub exchange_incomes: Vec<i32>,
    pub exchange_stage: FlaxExchangeStage,
    pub local_offer_sequence: u64,
    pub active_episode_id: Option<u64>,
    pub episode_qty: i32,
    pub episode_offer_qty: i32,
    pub episode_offer_my_inventory_floor: Option<u64>,
    pub episode_offer_their_inventory_floor: Option<u64>,
    pub episode_offer_my_inventory_generation: Option<u64>,
    pub episode_offer_their_inventory_generation: Option<u64>,
    pub episode_offer_modal_generation: Option<u64>,
    pub episode_offer_tick: Option<u32>,
    pub episode_offer_accept_id: Option<i32>,
    pub episode_flax_after_offer: Option<i32>,
    pub confirm_gap_tick: Option<u32>,
    pub transfer_close_tick: Option<u32>,
    pub second_delivery_before_bank_return: bool,
    pub second_delivery_after_bank_return: bool,
    pub saw_bank_open_loaded: bool,
    pub saw_bank_at_seers: bool,
    pub physical_string_deposit_observed: bool,
    pub bank_deposit_unlinked_to_first_transfer: bool,
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
            identity_unverified: false,
            session_mismatch: false,
            saw_offer_with_partner: false,
            saw_confirm_with_partner: false,
            saw_wrong_partner: false,
            transferred_out: 0,
            transferred_in: 0,
            partner_transfer_events: 0,
            exchange_outcomes: Vec::new(),
            exchange_incomes: Vec::new(),
            exchange_stage: FlaxExchangeStage::Offer,
            local_offer_sequence: 0,
            active_episode_id: None,
            episode_qty: 0,
            episode_offer_qty: 0,
            episode_offer_my_inventory_floor: None,
            episode_offer_their_inventory_floor: None,
            episode_offer_my_inventory_generation: None,
            episode_offer_their_inventory_generation: None,
            episode_offer_modal_generation: None,
            episode_offer_tick: None,
            episode_offer_accept_id: None,
            episode_flax_after_offer: None,
            confirm_gap_tick: None,
            transfer_close_tick: None,
            second_delivery_before_bank_return: false,
            second_delivery_after_bank_return: false,
            saw_bank_open_loaded: false,
            saw_bank_at_seers: false,
            physical_string_deposit_observed: false,
            bank_deposit_unlinked_to_first_transfer: false,
            deposited_strings: false,
            returned_to_meet: false,
            string_from_script: 0,
            xp_from_script: 0,
            spin_events: 0,
        }
    }

    fn drop_episode(&mut self) {
        self.active_episode_id = None;
        self.episode_qty = 0;
        self.episode_offer_qty = 0;
        self.episode_offer_my_inventory_floor = None;
        self.episode_offer_their_inventory_floor = None;
        self.episode_offer_my_inventory_generation = None;
        self.episode_offer_their_inventory_generation = None;
        self.episode_offer_modal_generation = None;
        self.episode_offer_accept_id = None;
        self.episode_offer_tick = None;
        self.episode_flax_after_offer = None;
        self.confirm_gap_tick = None;
        self.transfer_close_tick = None;
        self.exchange_stage = FlaxExchangeStage::Offer;
    }

    fn bind_episode(
        &mut self,
        observation: &FlaxObservation,
        previous_flax: i32,
        previous_my_offer_packet_generation: Option<u64>,
        previous_their_offer_packet_generation: Option<u64>,
    ) {
        self.active_episode_id = Some(self.local_offer_sequence);
        self.episode_qty = previous_flax;
        self.episode_offer_qty = 0;
        self.episode_offer_my_inventory_floor = previous_my_offer_packet_generation;
        self.episode_offer_their_inventory_floor = previous_their_offer_packet_generation;
        self.episode_offer_my_inventory_generation = None;
        self.episode_offer_their_inventory_generation = None;
        self.episode_offer_modal_generation = Some(observation.trade_modal_generation);
        self.episode_offer_accept_id =
            (observation.trade_accept_id >= 0).then_some(observation.trade_accept_id);
        self.episode_offer_tick = Some(observation.tick);
        self.episode_flax_after_offer = None;
        self.confirm_gap_tick = None;
        self.transfer_close_tick = None;
        self.saw_offer_with_partner = true;
        self.exchange_stage = FlaxExchangeStage::Confirm;
    }

    fn capture_offer_contents(&mut self, observation: &FlaxObservation) -> bool {
        if !observation.trade_my_offer_available || !observation.trade_their_offer_available {
            return true;
        }
        let (quantity, other_quantity) = match self.role {
            FlaxRole::Runner => (observation.trade_mine_flax, observation.trade_theirs_flax),
            FlaxRole::Spinner => (observation.trade_theirs_flax, observation.trade_mine_flax),
        };
        if quantity <= 0 {
            return other_quantity == 0;
        }
        let (Some(my_generation), Some(their_generation), Some(offer_modal_generation)) = (
            observation.trade_my_offer_packet_generation,
            observation.trade_their_offer_packet_generation,
            self.episode_offer_modal_generation,
        ) else {
            return false;
        };
        let (contributing_generation, contributing_floor) = match self.role {
            FlaxRole::Runner => (my_generation, self.episode_offer_my_inventory_floor),
            FlaxRole::Spinner => (their_generation, self.episode_offer_their_inventory_floor),
        };
        // Empty container packets can precede modal-open. Only the side that
        // contributes items must advance after that floor; an unchanged empty
        // counteroffer contributes no delivery credit. Positive counteroffers
        // and stale contributing contents remain refused.
        if contributing_generation <= contributing_floor.unwrap_or(0)
            || my_generation < self.episode_offer_my_inventory_floor.unwrap_or(0)
            || their_generation < self.episode_offer_their_inventory_floor.unwrap_or(0)
            || observation.trade_modal_generation != offer_modal_generation
            || other_quantity != 0
        {
            return false;
        }
        if self.episode_offer_qty > 0 && self.episode_offer_qty != quantity {
            return false;
        }
        let pack_before_offer = match self.role {
            FlaxRole::Runner => observation.flax + quantity,
            FlaxRole::Spinner => observation.flax,
        };
        if pack_before_offer != self.episode_qty {
            return false;
        }
        self.episode_offer_qty = quantity;
        self.episode_offer_my_inventory_generation = Some(my_generation);
        self.episode_offer_their_inventory_generation = Some(their_generation);
        self.episode_flax_after_offer = Some(observation.flax);
        true
    }
    fn offer_packet_state_matches(
        captured_generation: u64,
        current_generation: Option<u64>,
        available: bool,
        stopped: bool,
    ) -> bool {
        if current_generation == Some(captured_generation) {
            return available && !stopped;
        }
        captured_generation.checked_add(1) == current_generation && stopped && !available
    }

    fn offer_packet_states_still_belong(&self, observation: &FlaxObservation) -> bool {
        let (Some(my_generation), Some(their_generation)) = (
            self.episode_offer_my_inventory_generation,
            self.episode_offer_their_inventory_generation,
        ) else {
            return false;
        };
        Self::offer_packet_state_matches(
            my_generation,
            observation.trade_my_offer_packet_generation,
            observation.trade_my_offer_available,
            observation.trade_my_offer_stopped,
        ) && Self::offer_packet_state_matches(
            their_generation,
            observation.trade_their_offer_packet_generation,
            observation.trade_their_offer_available,
            observation.trade_their_offer_stopped,
        )
    }

    fn gap_still_belongs_to_episode(&self, observation: &FlaxObservation) -> bool {
        let (Some(offer_modal_generation), Some(flax_after_offer)) = (
            self.episode_offer_modal_generation,
            self.episode_flax_after_offer,
        ) else {
            return false;
        };
        if self.episode_offer_qty <= 0
            || observation.flax != flax_after_offer
            || !self.offer_packet_states_still_belong(observation)
            || observation
                .trade_modal_generation
                .checked_sub(offer_modal_generation)
                .is_none_or(|delta| delta > 1)
        {
            return false;
        }
        self.confirm_gap_tick.is_none_or(|gap_tick| {
            observation.tick >= gap_tick && observation.tick.saturating_sub(gap_tick) <= 1
        })
    }

    fn confirm_belongs_to_episode(&self, observation: &FlaxObservation) -> bool {
        let (
            Some(offer_modal_generation),
            Some(offer_accept_id),
            Some(offer_tick),
            Some(flax_after_offer),
        ) = (
            self.episode_offer_modal_generation,
            self.episode_offer_accept_id,
            self.episode_offer_tick,
            self.episode_flax_after_offer,
        )
        else {
            return false;
        };
        self.episode_offer_qty > 0
            && observation.tick >= offer_tick
            && observation
                .trade_modal_generation
                .checked_sub(offer_modal_generation)
                .is_some_and(|delta| (1..=2).contains(&delta))
            && self.offer_packet_states_still_belong(observation)
            && observation.trade_accept_id >= 0
            && observation.trade_accept_id != offer_accept_id
            && observation.flax == flax_after_offer
            && self.confirm_gap_tick.is_none_or(|gap_tick| {
                observation.tick >= gap_tick && observation.tick.saturating_sub(gap_tick) <= 1
            })
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
        let identity_ok = observation
            .player
            .as_deref()
            .is_some_and(|player| account_identity_eq(player, &self.expected_player));
        if observation.player.is_none() {
            self.identity_unverified = true;
        } else if !identity_ok {
            self.mixed_identity = true;
        }
        let session_ok =
            observation.trade_session_generation == self.baseline.trade_session_generation;
        if !session_ok || !observation.ingame {
            self.session_mismatch = true;
        }
        let continuity_ok =
            identity_ok && session_ok && observation.ingame && observation.scene_state == 2;
        let (
            prev_flax,
            prev_string,
            prev_xp,
            prev_trade_modal_generation,
            prev_my_offer_packet_generation,
            prev_their_offer_packet_generation,
        ) = self.latest.as_ref().map_or(
            (
                self.baseline.flax,
                self.baseline.bow_string,
                self.baseline.crafting_xp,
                self.baseline.trade_modal_generation,
                self.baseline.trade_my_offer_packet_generation,
                self.baseline.trade_their_offer_packet_generation,
            ),
            |row| {
                (
                    row.flax,
                    row.bow_string,
                    row.crafting_xp,
                    row.trade_modal_generation,
                    row.trade_my_offer_packet_generation,
                    row.trade_their_offer_packet_generation,
                )
            },
        );
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
        if self.saw_wrong_partner || !continuity_ok {
            self.drop_episode();
        } else if offer_rising {
            // Every newly observed offer invalidates the previous local episode.
            self.drop_episode();
            self.local_offer_sequence = self.local_offer_sequence.saturating_add(1);
            if named_partner
                && observation.trade_modal_generation > prev_trade_modal_generation
                && observation.trade_accept_id >= 0
            {
                self.bind_episode(
                    &observation,
                    prev_flax,
                    prev_my_offer_packet_generation,
                    prev_their_offer_packet_generation,
                );
                if !self.capture_offer_contents(&observation) {
                    self.drop_episode();
                }
            }
        } else {
            match self.exchange_stage {
                FlaxExchangeStage::Offer => {}
                FlaxExchangeStage::Confirm => {
                    if Self::partnerless_confirm(&observation)
                        || (observation.trade_active() && !named_partner)
                    {
                        self.drop_episode();
                    } else if observation.trade_offer_open {
                        if !named_partner || !self.capture_offer_contents(&observation) {
                            self.drop_episode();
                        }
                    } else if observation.trade_confirm_open {
                        if named_partner && self.confirm_belongs_to_episode(&observation) {
                            self.saw_confirm_with_partner = true;
                            self.exchange_stage = FlaxExchangeStage::Transfer;
                        } else {
                            self.drop_episode();
                        }
                    } else if !self.gap_still_belongs_to_episode(&observation) {
                        self.drop_episode();
                    } else if self.confirm_gap_tick.is_none() {
                        self.confirm_gap_tick = Some(observation.tick);
                    }
                }
                FlaxExchangeStage::Transfer => {
                    if observation.trade_active() {
                        if !named_partner || observation.trade_offer_open {
                            self.drop_episode();
                        }
                    } else if self.transfer_close_tick.is_none() {
                        self.transfer_close_tick = Some(observation.tick);
                    }
                    if !observation.trade_active()
                        && self.exchange_stage == FlaxExchangeStage::Transfer
                    {
                        if self.transfer_close_tick != Some(observation.tick) {
                            self.drop_episode();
                        } else {
                            let at_meet = near(observation.tile, FLAX_MEET, 8);
                            let transfer_qty = match self.role {
                                FlaxRole::Runner => (self.episode_qty - observation.flax).max(0),
                                FlaxRole::Spinner => (observation.flax - self.episode_qty).max(0),
                            };
                            let completed = at_meet
                                && self.episode_offer_qty > 0
                                && transfer_qty == self.episode_offer_qty;
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
                                match self.role {
                                    FlaxRole::Runner => {
                                        self.transferred_out += transfer_qty;
                                        self.exchange_outcomes.push(transfer_qty);
                                    }
                                    FlaxRole::Spinner => {
                                        self.transferred_in += transfer_qty;
                                        self.exchange_incomes.push(transfer_qty);
                                    }
                                }
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
                if string_out > 0 {
                    self.physical_string_deposit_observed = true;
                    if self.partner_transfer_events == 0 {
                        self.bank_deposit_unlinked_to_first_transfer = true;
                    }
                }
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
            || prev.trade_modal_generation != observation.trade_modal_generation
            || prev.trade_my_offer_packet_generation != observation.trade_my_offer_packet_generation
            || prev.trade_their_offer_packet_generation
                != observation.trade_their_offer_packet_generation
            || prev.trade_my_offer_available != observation.trade_my_offer_available
            || prev.trade_their_offer_available != observation.trade_their_offer_available
            || prev.trade_my_offer_stopped != observation.trade_my_offer_stopped
            || prev.trade_their_offer_stopped != observation.trade_their_offer_stopped
            || prev.trade_mine_flax != observation.trade_mine_flax
            || prev.trade_theirs_flax != observation.trade_theirs_flax
            || stage_before != stage_after
    });
    if changed {
        eprintln!(
            "PAIR_TRADE_EDGE actor={actor} tick={} offer={} confirm={} partner={} accept_id={} modal_gen={} inv_gen={} my_gen={:?} their_gen={:?} my_available={} their_available={} my_stopped={} their_stopped={} offer_mine={} offer_theirs={} held_flax={} held_string={} stage={stage_before:?}->{stage_after:?}",
            observation.tick,
            observation.trade_offer_open,
            observation.trade_confirm_open,
            observation.trade_partner.as_deref().unwrap_or("<none>"),
            observation.trade_accept_id,
            observation.trade_modal_generation,
            observation.trade_inventory_generation,
            observation.trade_my_offer_packet_generation,
            observation.trade_their_offer_packet_generation,
            observation.trade_my_offer_available,
            observation.trade_their_offer_available,
            observation.trade_my_offer_stopped,
            observation.trade_their_offer_stopped,
            observation.trade_mine_flax,
            observation.trade_theirs_flax,
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
        if self.runner.exchange_outcomes.len() != self.spinner.exchange_incomes.len() {
            return Err(format!(
                "missing conservation: runner has {} bound deliveries, spinner has {}",
                self.runner.exchange_outcomes.len(),
                self.spinner.exchange_incomes.len()
            ));
        }
        for (index, (sent, received)) in self
            .runner
            .exchange_outcomes
            .iter()
            .zip(&self.spinner.exchange_incomes)
            .enumerate()
        {
            if sent != received {
                return Err(format!(
                    "missing conservation: exchange {} runner sent {sent} flax 1779, spinner received {received}",
                    index + 1
                ));
            }
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
        if self.spinner.bank_deposit_unlinked_to_first_transfer && !self.spinner.deposited_strings {
            return Err("bank observed but unlinked prior transfer: spinner physically deposited strings at Seers before a bilateral flax delivery was proven".into());
        }
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
        if !self.spinner.saw_bank_at_seers {
            return Err("no physical bank observation: spinner never opened a loaded bank at Seers after the first spin".into());
        }
        if !self.spinner.deposited_strings
            || !self.spinner.saw_bank_open_loaded
            || !self.spinner.physical_string_deposit_observed
        {
            return Err(
                "bank observed but no linked string deposit: spinner did not physically deposit spun strings at Seers after the first transfer"
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
        if self.runner.identity_unverified || self.spinner.identity_unverified {
            return Err(
                "stale identity: local player identity was absent during pair observations".into(),
            );
        }
        if self.runner.session_mismatch || self.spinner.session_mismatch {
            return Err(
                "stale session identity: pair observations crossed the prepared client session"
                    .into(),
            );
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
