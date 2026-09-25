use super::*;
#[derive(Debug, Clone, Default, Serialize)]
pub struct AirObservation {
    pub ingame: bool,
    pub scene_state: i32,
    pub inventory_tab_available: bool,
    pub player: Option<String>,
    pub tile: Option<(i32, i32, i32)>,
    pub tick: u32,
    pub runecraft: i32,
    pub runecraft_xp: i32,
    pub essence_unnoted: i32,
    pub essence_noted: i32,
    pub air_runes: i32,
    pub air_talisman: i32,
    pub bank_open: bool,
    pub bank_loaded: bool,
    pub bank_session_generation: u64,
    pub bank_essence_unnoted: i32,
    pub trade_offer_open: bool,
    pub trade_confirm_open: bool,
    pub trade_partner: Option<String>,
    pub trade_accept_id: i32,
    pub trade_mine_essence: i32,
    pub trade_theirs_essence: i32,
    pub in_temple: bool,
}

impl AirObservation {
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
            runecraft: stat(snapshot, "runecraft").map(|row| row.base).unwrap_or(0),
            runecraft_xp: stat(snapshot, "runecraft").map(|row| row.xp).unwrap_or(0),
            essence_unnoted: count_id(snapshot.inventory(), ESSENCE_UNNOTED_ID),
            essence_noted: count_id(snapshot.inventory(), ESSENCE_NOTED_ID),
            air_runes: count_id(snapshot.inventory(), AIR_RUNE_ID),
            air_talisman: count_id(snapshot.inventory(), AIR_TALISMAN_ID),
            bank_open: snapshot.bank_component_id() >= 0,
            bank_loaded: snapshot.bank_loaded(),
            bank_session_generation: snapshot.bank_session_generation(),
            bank_essence_unnoted: count_id(snapshot.bank(), ESSENCE_UNNOTED_ID),
            trade_offer_open: trade.offer_open,
            trade_confirm_open: trade.confirm_open,
            trade_partner: trade.partner.clone(),
            trade_accept_id: trade.accept_component_id,
            trade_mine_essence: count_id(&trade.my_offer, ESSENCE_UNNOTED_ID),
            trade_theirs_essence: count_id(&trade.their_offer, ESSENCE_UNNOTED_ID),
            in_temple: in_temple(tile),
        }
    }

    pub fn trade_active(&self) -> bool {
        self.trade_offer_open || self.trade_confirm_open
    }
}
#[allow(clippy::too_many_arguments)] // runecraft observe packs prev counts and partner transfer fields
pub(super) fn observe_runecraft_work(
    observation: &AirObservation,
    prev_ess: i32,
    prev_air: i32,
    prev_xp: i32,
    expire: bool,
    input_from_exchange: &mut i32,
    consumed: &mut i32,
    xp_at_consume: &mut i32,
    air_at_consume: &mut i32,
    from_exchange: &mut bool,
    craft_events: &mut u32,
    post_exchange_craft_events: &mut u32,
    partner_transfer_events: u32,
) {
    // Keep unconsumed receipt credit through ingame LoadingScene while
    // carrying received essence. Never complete a partly observed craft
    // across temple leave or scene_state != 2, and never treat loading
    // inventory/XP as consume or product.
    if expire || (*consumed > 0 && !observation.in_temple) {
        *input_from_exchange = 0;
        *consumed = 0;
        *xp_at_consume = 0;
        *air_at_consume = 0;
        *from_exchange = false;
        return;
    }
    if observation.scene_state != 2 {
        *consumed = 0;
        *xp_at_consume = 0;
        *air_at_consume = 0;
        *from_exchange = false;
        return;
    }
    let ess_drop = (prev_ess - observation.essence_unnoted).max(0);
    if observation.in_temple
        && !observation.trade_active()
        && !observation.bank_open
        && ess_drop > 0
    {
        if *consumed == 0 {
            *xp_at_consume = prev_xp;
            *air_at_consume = prev_air;
        }
        if *input_from_exchange > 0 {
            let take = ess_drop.min(*input_from_exchange);
            *input_from_exchange -= take;
            *from_exchange = true;
        }
        *consumed = consumed.saturating_add(ess_drop);
    }
    if *consumed > 0
        && observation.runecraft_xp > *xp_at_consume
        && observation.air_runes > *air_at_consume
    {
        *craft_events = craft_events.saturating_add(1);
        if *from_exchange && *post_exchange_craft_events < partner_transfer_events {
            *post_exchange_craft_events = post_exchange_craft_events.saturating_add(1);
        }
        *consumed = 0;
        *xp_at_consume = 0;
        *air_at_consume = 0;
        *from_exchange = false;
    }
}