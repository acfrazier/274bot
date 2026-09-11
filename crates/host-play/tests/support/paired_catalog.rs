//! Catalog identity, observations, and paired witnesses for NatureCrafter Air
//! and Duel Arena Combat Trainer. Unique to this fixture; not a shared
//! scenario or isolation harness.

use std::path::{Component, Path, PathBuf};

use api::game_data::{DuelControls, SelectedGameData};
use api::snapshot::{GameSnapshot, ItemView, WidgetView};
use serde::Serialize;
use serde_json::{json, Map, Value};

pub const SUPPORT_MATRIX: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../docs/compat/support-matrix.json"
));
pub const CATALOG_COMMIT_A: &str = "100adccc037d9f6898080e1cad58fcfc43364775";
pub const CATALOG_COMMIT_B: &str = "8e7d965be2071d6ec65c3265e12af797082d720a";

pub const NATURECRAFTER: &str = "NatureCrafter";
pub const DUEL_ARENA: &str = "Duel Arena Combat Trainer";
pub const NATURECRAFTER_SHA256: &str =
    "025ac395b25d64ef818cc0321478f0a2c84a051b79f99decbbfec5a9a2f0812a";
pub const DUEL_ARENA_SHA256: &str =
    "5656dabb30a47aac590fa1afadba19e689dd792d70da8dc4851e18d62e52d090";
pub const NATURE_RUNNER_LOGIC_SHA256: &str =
    "7a81b75a4cc4fde41d12565f5fe999de7931d88f2529da6d2d5e6a31f82d82f0";
pub const DUEL_ARENA_LOGIC_SHA256: &str =
    "325ce631a7a3f246ab0bc51e9b09945aaa018d7c8971b334994384f62cd1d8f2";
pub const DUEL_INTERFACE_SHA256: &str =
    "658e20f50117f0d23b8524e0ca389399d6fd52de2de1585313081324434634f9";

pub const ESSENCE_UNNOTED_ID: i32 = 1436;
pub const ESSENCE_NOTED_ID: i32 = 1437;
pub const AIR_RUNE_ID: i32 = 556;
pub const AIR_TALISMAN_ID: i32 = 1438;
pub const TRADE_CAP: i32 = 25;
pub const BANK_SEED_ESSENCE: i32 = 200;
pub const TEMPLE_Z: i32 = 4000;
pub const AIR_RUINS: (i32, i32, i32) = (2983, 3288, 0);
pub const FALADOR_EAST: (i32, i32, i32) = (3013, 3355, 0);
pub const DUEL_CHALLENGE_ANCHOR: (i32, i32, i32) = (3368, 3274, 0);
pub const SCRIPT_GOLD_DEADLINE_SECS: u64 = 180;
pub const SCRIPT_GOLD_WATCH_TICKS: u32 = 150;
pub const PREP_DEADLINE_SECS: u64 = 180;

pub const DUEL_SELECT_MODAL: i32 = 6575;
pub const DUEL_CONFIRM_MODAL: i32 = 6412;
pub const DUEL_WIN_MODAL: i32 = 6733;
pub const DUEL_SELECT_ACCEPT: i32 = 6674;
pub const DUEL_CONFIRM_ACCEPT: i32 = 6520;
pub const DUEL_SELECT_PARTNER: i32 = 6671;
pub const DUEL_SELECT_STATUS: i32 = 6684;
pub const DUEL_CONFIRM_STATUS: i32 = 6571;

pub const EXPECTED_DUEL_CONTROLS: DuelControls = DuelControls {
    select_modal: DUEL_SELECT_MODAL,
    confirm_modal: DUEL_CONFIRM_MODAL,
    win_modal: DUEL_WIN_MODAL,
    select_accept: DUEL_SELECT_ACCEPT,
    confirm_accept: DUEL_CONFIRM_ACCEPT,
    select_partner: DUEL_SELECT_PARTNER,
    select_status: DUEL_SELECT_STATUS,
    confirm_status: DUEL_CONFIRM_STATUS,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PairCase {
    Air,
    Duel,
}

impl PairCase {
    pub fn card_name(self) -> &'static str {
        match self {
            Self::Air => NATURECRAFTER,
            Self::Duel => DUEL_ARENA,
        }
    }

    pub fn source_sha256(self) -> &'static str {
        match self {
            Self::Air => NATURECRAFTER_SHA256,
            Self::Duel => DUEL_ARENA_SHA256,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AirRole {
    Master,
    Runner,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GateKind {
    Mapped,
    ArityLimited,
    UnusedByCase,
    CatalogLiteralMatchesGenerated,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct OperationGate {
    pub source: &'static str,
    pub call: &'static str,
    pub host_shape: &'static str,
    pub kind: GateKind,
    pub owner: &'static str,
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

pub fn duel_operation_gates() -> &'static [OperationGate] {
    &[
        OperationGate {
            source: "DuelInterface.ts Duel.challenge/fight",
            call: "Input.interactPlayer(index, 1|2)",
            host_shape: "{ op: 'player', name, action } from posted player ops[op-1]; false if missing",
            kind: GateKind::Mapped,
            owner: "this fixture; queued Challenge without combat is not a duel",
        },
        OperationGate {
            source: "DuelInterface.ts Duel.accept/partner/waitingForOther",
            call: "actions.ifButton(6674|6520) / reader.ifText(6671|6684|6571) / reader.modals().main",
            host_shape: "generated duel controls + posted widget text; absent id is null, never stale IfType",
            kind: GateKind::CatalogLiteralMatchesGenerated,
            owner: "catalog still hardcodes IDs; both caches match 6575/6412/6733/6674/6520/6671/6684/6571",
        },
        OperationGate {
            source: "DuelArena.ts FightOpponent / observeFightState",
            call: "Game.inCombat / reader.selfChat 3/2/1/FIGHT / fightArenaAt pens",
            host_shape: "local actor in_combat + overhead; seeded modal is not combat",
            kind: GateKind::Mapped,
            owner: "this fixture",
        },
        OperationGate {
            source: "DuelArena.ts SetTrainingStyle",
            call: "Game.combatStyleResolution / Game.setCombatStyle / Game.combatMode",
            host_shape: "existing combat-style mapping; requires exact Attack+Strength (Defence only if target>1)",
            kind: GateKind::Mapped,
            owner: "this fixture seeds a 1-handed melee weapon",
        },
        OperationGate {
            source: "DuelArena.ts CenterLobby / seekOpponent",
            call: "DirectNavigator.walkTo",
            host_shape: "existing direct walk",
            kind: GateKind::Mapped,
            owner: "this fixture",
        },
        OperationGate {
            source: "DuelArena.ts",
            call: "Game.teleport / special / Shop",
            host_shape: "n/a",
            kind: GateKind::UnusedByCase,
            owner: "teleport t_1bf9a22e, special t_68de6f48, shop t_1591d140",
        },
    ]
}

pub fn duel_controls_match(controls: &DuelControls) -> bool {
    controls.select_modal == EXPECTED_DUEL_CONTROLS.select_modal
        && controls.confirm_modal == EXPECTED_DUEL_CONTROLS.confirm_modal
        && controls.win_modal == EXPECTED_DUEL_CONTROLS.win_modal
        && controls.select_accept == EXPECTED_DUEL_CONTROLS.select_accept
        && controls.confirm_accept == EXPECTED_DUEL_CONTROLS.confirm_accept
        && controls.select_partner == EXPECTED_DUEL_CONTROLS.select_partner
        && controls.select_status == EXPECTED_DUEL_CONTROLS.select_status
        && controls.confirm_status == EXPECTED_DUEL_CONTROLS.confirm_status
        && controls.available()
}

pub fn verify_generated_duel_controls(data: &SelectedGameData) -> Result<(), String> {
    let Some(controls) = data.duel_controls() else {
        return Err(format!(
            "selected game data revision {} has no available duel controls",
            data.revision()
        ));
    };
    if !duel_controls_match(controls) {
        return Err(format!(
            "generated duel controls mismatch on revision {}: {controls:?}",
            data.revision()
        ));
    }
    Ok(())
}

pub fn count_id(items: &[ItemView], id: i32) -> i32 {
    items
        .iter()
        .filter(|item| item.def.id == id && item.count > 0)
        .map(|item| item.count)
        .sum()
}

pub fn near(tile: Option<(i32, i32, i32)>, target: (i32, i32, i32), radius: i32) -> bool {
    tile.is_some_and(|tile| {
        tile.2 == target.2 && (tile.0 - target.0).abs().max((tile.1 - target.1).abs()) <= radius
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelogAdmission {
    WaitLogout,
    LoggedOut,
    WaitLogin,
    Ready,
}

pub fn relog_admission(
    saw_logout: bool,
    ingame: bool,
    scene_state: i32,
    inventory_tab_available: bool,
) -> RelogAdmission {
    if !saw_logout {
        if !ingame || scene_state != 2 {
            RelogAdmission::LoggedOut
        } else {
            RelogAdmission::WaitLogout
        }
    } else if ingame && scene_state == 2 && inventory_tab_available {
        RelogAdmission::Ready
    } else {
        RelogAdmission::WaitLogin
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartBarrier {
    Wait,
    StartBoth,
    RejectStartedWhileUnready,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StartBarrierInput {
    pub a_prepared: bool,
    pub b_prepared: bool,
    pub a_started: bool,
    pub b_started: bool,
    pub a_wait_ack: bool,
    pub b_wait_ack: bool,
    pub a_current_ok: bool,
    pub b_current_ok: bool,
}

pub fn shared_start_barrier(input: StartBarrierInput) -> StartBarrier {
    if input.a_started || input.b_started {
        if input.a_started && input.b_started {
            return StartBarrier::Wait;
        }
        return StartBarrier::RejectStartedWhileUnready;
    }
    if input.a_wait_ack || input.b_wait_ack {
        return StartBarrier::Wait;
    }
    if input.a_prepared && input.b_prepared && input.a_current_ok && input.b_current_ok {
        StartBarrier::StartBoth
    } else {
        StartBarrier::Wait
    }
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
    if !player.eq_ignore_ascii_case(expected_player) {
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

pub fn duel_prepared_current(
    expected_player: &str,
    observation: &DuelObservation,
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
    if !player.eq_ignore_ascii_case(expected_player) {
        return Err(format!(
            "Start baseline player {player:?} is not fresh account {expected_player:?}"
        ));
    }
    if !observation.in_challenge_area {
        return Err(format!(
            "Start baseline is not in the Duel Arena challenge area: {:?}",
            observation.tile
        ));
    }
    if observation.duel_active() || observation.duel_win_open {
        return Err("seeded modal is not a duel: baseline already has a duel interface".into());
    }
    if !observation.weapon_equipped {
        return Err("Start baseline has no 1-handed melee weapon equipped".into());
    }
    Ok(())
}

pub fn in_temple(tile: Option<(i32, i32, i32)>) -> bool {
    tile.is_some_and(|tile| tile.1 > TEMPLE_Z)
}

pub fn in_duel_challenge_area(tile: Option<(i32, i32, i32)>) -> bool {
    tile.is_some_and(|tile| {
        tile.2 == 0
            && tile.0 >= 3328
            && tile.0 <= 3393
            && tile.1 >= 3203
            && tile.1 <= 3325
            && fight_pen(Some(tile)).is_none()
    })
}

pub fn fight_pen(tile: Option<(i32, i32, i32)>) -> Option<(i32, i32, i32, i32)> {
    const PENS: [(i32, i32, i32, i32); 6] = [
        (3333, 3357, 3244, 3258),
        (3364, 3388, 3225, 3239),
        (3333, 3357, 3206, 3220),
        (3364, 3388, 3244, 3258),
        (3333, 3357, 3225, 3239),
        (3364, 3388, 3206, 3220),
    ];
    let tile = tile?;
    if tile.2 != 0 {
        return None;
    }
    PENS.iter().copied().find(|(min_x, max_x, min_z, max_z)| {
        tile.0 >= *min_x && tile.0 <= *max_x && tile.1 >= *min_z && tile.1 <= *max_z
    })
}

fn widget_text(widgets: &[WidgetView], component_id: i32) -> Option<String> {
    widgets.iter().find_map(|widget| {
        (widget.component_id == component_id)
            .then(|| widget.text.clone())
            .flatten()
            .filter(|text| !text.is_empty())
    })
}

fn parse_duel_partner_header(header: Option<&str>) -> Option<String> {
    let header = header?.trim();
    let stripped = header
        .strip_prefix("Dueling with:")
        .or_else(|| header.strip_prefix("dueling with:"))
        .unwrap_or(header)
        .trim()
        .trim_start_matches(':')
        .trim();
    if stripped.is_empty() {
        None
    } else {
        Some(stripped.to_string())
    }
}

#[derive(Debug, Clone, Serialize)]
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

#[derive(Debug, Clone, Serialize)]
pub struct DuelObservation {
    pub ingame: bool,
    pub scene_state: i32,
    pub inventory_tab_available: bool,
    pub player: Option<String>,
    pub tile: Option<(i32, i32, i32)>,
    pub tick: u32,
    pub attack_xp: i32,
    pub strength_xp: i32,
    pub defence_xp: i32,
    pub hitpoints_xp: i32,
    pub in_combat: bool,
    pub in_challenge_area: bool,
    pub in_fight_pen: bool,
    pub main_modal: i32,
    pub duel_offer_open: bool,
    pub duel_confirm_open: bool,
    pub duel_win_open: bool,
    pub duel_partner: Option<String>,
    pub waiting_for_other: bool,
    pub weapon_equipped: bool,
    pub peer_visible: bool,
}

impl DuelObservation {
    pub fn from_snapshot(snapshot: &GameSnapshot, peer: &str, weapon_id: i32) -> Self {
        let tile = snapshot.tile();
        let main = snapshot.modals().main;
        let partner = parse_duel_partner_header(
            widget_text(snapshot.widgets(), DUEL_SELECT_PARTNER).as_deref(),
        );
        let status = if main == DUEL_SELECT_MODAL {
            widget_text(snapshot.widgets(), DUEL_SELECT_STATUS)
        } else if main == DUEL_CONFIRM_MODAL {
            widget_text(snapshot.widgets(), DUEL_CONFIRM_STATUS)
        } else {
            None
        };
        let waiting = status.as_deref().is_some_and(|text| {
            text.trim()
                .to_ascii_lowercase()
                .starts_with("waiting for other")
        });
        let peer_visible = snapshot.players().iter().any(|player| {
            player
                .actor
                .name
                .as_deref()
                .is_some_and(|name| name.eq_ignore_ascii_case(peer))
        });
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
            attack_xp: stat(snapshot, "attack").map(|row| row.xp).unwrap_or(0),
            strength_xp: stat(snapshot, "strength").map(|row| row.xp).unwrap_or(0),
            defence_xp: stat(snapshot, "defence").map(|row| row.xp).unwrap_or(0),
            hitpoints_xp: stat(snapshot, "hitpoints").map(|row| row.xp).unwrap_or(0),
            in_combat: snapshot
                .local_player()
                .is_some_and(|local| local.player.actor.in_combat),
            in_challenge_area: in_duel_challenge_area(tile),
            in_fight_pen: fight_pen(tile).is_some(),
            main_modal: main,
            duel_offer_open: main == DUEL_SELECT_MODAL,
            duel_confirm_open: main == DUEL_CONFIRM_MODAL,
            duel_win_open: main == DUEL_WIN_MODAL,
            duel_partner: partner,
            waiting_for_other: waiting,
            weapon_equipped: count_id(snapshot.equipment(), weapon_id) > 0,
            peer_visible,
        }
    }

    pub fn melee_xp(&self) -> i32 {
        self.attack_xp + self.strength_xp + self.defence_xp
    }

    pub fn duel_active(&self) -> bool {
        self.duel_offer_open || self.duel_confirm_open
    }
}

fn stat<'a>(snapshot: &'a GameSnapshot, name: &str) -> Option<&'a api::snapshot::StatView> {
    snapshot
        .stats()
        .iter()
        .find(|row| row.name.eq_ignore_ascii_case(name))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum AirClaim {
    FirstTransferCraft,
    BankReturnSecondCycle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum DuelClaim {
    FirstCombat,
    ResetAndFurther,
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
    pub saw_bank_open_loaded: bool,
    pub saw_bank_at_falador: bool,
    pub restock_withdraw: bool,
    pub returned_to_ruins: bool,
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
            saw_bank_open_loaded: false,
            saw_bank_at_falador: false,
            restock_withdraw: false,
            returned_to_ruins: false,
            air_from_script: 0,
            xp_from_script: 0,
        }
    }

    pub fn observe(&mut self, observation: AirObservation) {
        self.post_start = self.post_start.saturating_add(1);
        if observation
            .player
            .as_deref()
            .is_some_and(|player| !player.eq_ignore_ascii_case(&self.expected_player))
        {
            self.mixed_identity = true;
        }
        if observation.trade_active() {
            if let Some(partner) = observation.trade_partner.as_deref() {
                if !partner.is_empty() && !partner.eq_ignore_ascii_case(&self.partner) {
                    self.saw_wrong_partner = true;
                }
                if partner.eq_ignore_ascii_case(&self.partner) {
                    if observation.trade_offer_open {
                        self.saw_offer_with_partner = true;
                    }
                    if observation.trade_confirm_open {
                        self.saw_confirm_with_partner = true;
                    }
                }
            } else if observation.trade_offer_open {
                self.saw_offer_with_partner = true;
            } else if observation.trade_confirm_open {
                self.saw_confirm_with_partner = true;
            }
        }
        let prev_ess = self
            .latest
            .as_ref()
            .map(|row| row.essence_unnoted)
            .unwrap_or(self.baseline.essence_unnoted);
        if observation.essence_unnoted < prev_ess {
            self.transferred_out += prev_ess - observation.essence_unnoted;
        }
        if observation.essence_unnoted > prev_ess {
            self.transferred_in += observation.essence_unnoted - prev_ess;
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
        if self.runner.transferred_out <= 0 {
            return Err(
                "missing conservation: runner unnoted essence 1436 did not leave the pack".into(),
            );
        }
        if self.master.transferred_in <= 0 && self.master.air_from_script <= 0 {
            return Err(
                "missing conservation: master did not receive unnoted 1436 and did not craft Air 556"
                    .into(),
            );
        }
        if self.master.transferred_in > 0
            && self.master.transferred_in != self.runner.transferred_out
            && self.master.air_from_script <= 0
        {
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
        if self.runner.transferred_out <= self.runner.baseline.essence_unnoted {
            return Err("no further work for full-cycle claims: only the seeded first load left the runner; no second transfer".into());
        }
        if self.master.air_from_script < 2 && self.master.xp_from_script <= 5 {
            return Err(
                "no further work for full-cycle claims: no second master craft after restock"
                    .into(),
            );
        }
        Ok(AirClaim::BankReturnSecondCycle)
    }

    fn qualify_common(&self) -> Result<(), String> {
        if self
            .master
            .account
            .eq_ignore_ascii_case(&self.runner.account)
        {
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
        if !self
            .master
            .partner
            .eq_ignore_ascii_case(&self.runner.expected_player)
            || !self
                .runner
                .partner
                .eq_ignore_ascii_case(&self.master.expected_player)
        {
            return Err("wrong partner: configured partner is not the minted counterpart".into());
        }
        if self.master.baseline.essence_noted > 0 || self.runner.baseline.essence_noted > 0 {
            return Err("seed-only inventory/XP: noted essence 1437 is not accepted on Air".into());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct DuelSlotRecord {
    pub account: String,
    pub expected_player: String,
    pub partner: String,
    pub settings: Map<String, Value>,
    pub baseline: DuelObservation,
    pub latest: Option<DuelObservation>,
    pub post_start: u32,
    pub mixed_identity: bool,
    pub saw_offer: bool,
    pub saw_confirm: bool,
    pub saw_wrong_partner: bool,
    pub saw_pen: bool,
    pub saw_combat: bool,
    pub saw_win_or_lobby_return: bool,
    pub melee_xp_from_script: i32,
    pub further_combat: bool,
}

impl DuelSlotRecord {
    pub fn new(
        account: String,
        expected_player: String,
        partner: String,
        settings: Map<String, Value>,
        baseline: DuelObservation,
    ) -> Self {
        Self {
            account,
            expected_player,
            partner,
            settings,
            baseline,
            latest: None,
            post_start: 0,
            mixed_identity: false,
            saw_offer: false,
            saw_confirm: false,
            saw_wrong_partner: false,
            saw_pen: false,
            saw_combat: false,
            saw_win_or_lobby_return: false,
            melee_xp_from_script: 0,
            further_combat: false,
        }
    }

    pub fn observe(&mut self, observation: DuelObservation) {
        self.post_start = self.post_start.saturating_add(1);
        if observation
            .player
            .as_deref()
            .is_some_and(|player| !player.eq_ignore_ascii_case(&self.expected_player))
        {
            self.mixed_identity = true;
        }
        if observation.duel_offer_open {
            self.saw_offer = true;
        }
        if observation.duel_confirm_open {
            self.saw_confirm = true;
        }
        if let Some(partner) = observation.duel_partner.as_deref() {
            if !partner.eq_ignore_ascii_case(&self.partner) {
                self.saw_wrong_partner = true;
            }
        }
        if observation.in_fight_pen {
            self.saw_pen = true;
        }
        if observation.in_combat && observation.in_fight_pen {
            if self.saw_combat && self.saw_win_or_lobby_return {
                self.further_combat = true;
            }
            self.saw_combat = true;
        }
        if self.saw_combat
            && (observation.duel_win_open || observation.in_challenge_area)
            && !observation.in_fight_pen
        {
            self.saw_win_or_lobby_return = true;
        }
        self.melee_xp_from_script = (observation.melee_xp() - self.baseline.melee_xp()).max(0);
        self.latest = Some(observation);
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct DuelPairWitness {
    pub a: DuelSlotRecord,
    pub b: DuelSlotRecord,
}

impl DuelPairWitness {
    pub fn qualify_supported(&self) -> Result<DuelClaim, String> {
        self.qualify_common()?;
        if self.a.baseline.duel_active() || self.b.baseline.duel_active() {
            return Err("seeded modal is not a duel: baseline already had a duel interface".into());
        }
        if !self.a.saw_offer || !self.b.saw_offer {
            return Err(
                "one-sided confirmation: both scripts never observed the select/offer duel modal"
                    .into(),
            );
        }
        if !self.a.saw_confirm || !self.b.saw_confirm {
            return Err(
                "one-sided confirmation: both scripts never observed the confirm duel modal".into(),
            );
        }
        if !self.a.saw_pen || !self.b.saw_pen {
            return Err("no real duel combat: neither actor entered a fight pen".into());
        }
        if !self.a.saw_combat || !self.b.saw_combat {
            return Err("no real duel combat: in-combat was never observed inside a pen".into());
        }
        if self.a.melee_xp_from_script <= 0 && self.b.melee_xp_from_script <= 0 {
            return Err(
                "seed-only inventory/XP: no Attack/Strength/Defence XP caused by hits".into(),
            );
        }
        Ok(DuelClaim::FirstCombat)
    }

    pub fn qualify_full_cycle(&self) -> Result<DuelClaim, String> {
        self.qualify_supported()?;
        if !self.a.saw_win_or_lobby_return && !self.b.saw_win_or_lobby_return {
            return Err(
                "no further work for full-cycle claims: duel did not end/reset to the lobby".into(),
            );
        }
        if !self.a.further_combat && !self.b.further_combat {
            return Err("no further work for full-cycle claims: no further script-caused challenge/combat after reset".into());
        }
        Ok(DuelClaim::ResetAndFurther)
    }

    fn qualify_common(&self) -> Result<(), String> {
        if self.a.account.eq_ignore_ascii_case(&self.b.account) {
            return Err("mixed identities: both duel slots share one account".into());
        }
        if self.a.mixed_identity || self.b.mixed_identity {
            return Err("mixed identities: published player did not match the minted slot".into());
        }
        if self.a.post_start == 0 || self.b.post_start == 0 {
            return Err("queued-only success: no post-Start observations".into());
        }
        if self.a.saw_wrong_partner || self.b.saw_wrong_partner {
            return Err("wrong partner: duel header was not the minted counterpart".into());
        }
        if !self.a.partner.eq_ignore_ascii_case(&self.b.expected_player)
            || !self.b.partner.eq_ignore_ascii_case(&self.a.expected_player)
        {
            return Err("wrong partner: configured partner is not the minted counterpart".into());
        }
        Ok(())
    }
}

#[derive(Debug, serde::Deserialize)]
struct SupportMatrix {
    catalogs: Vec<CatalogLedger>,
    rows: Vec<CardLedger>,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct CatalogLedger {
    pub commit: String,
    pub identity: String,
    pub read_only_path: String,
    pub registry_path: String,
    pub registry_sha256: String,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct CardLedger {
    pub display_name: String,
    pub catalog_commit: String,
    pub source_path: String,
    pub source_sha256: String,
    pub revision: u16,
}

pub struct PreparedCard {
    pub js: String,
    pub shape: script::LoadShape,
    pub siblings: Vec<(String, String)>,
    pub schema: Vec<script::SettingDef>,
    pub identity: Value,
}

fn support_matrix() -> Result<SupportMatrix, String> {
    serde_json::from_str(SUPPORT_MATRIX).map_err(|error| format!("support matrix: {error}"))
}

fn validate_commit(commit: &str) -> Result<(), String> {
    if commit.len() != 40
        || !commit
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(format!(
            "PAIRED_CATALOG_COMMIT must be the exact lowercase 40-hex frozen commit, got {commit:?}"
        ));
    }
    if commit != CATALOG_COMMIT_A && commit != CATALOG_COMMIT_B {
        return Err(format!(
            "PAIRED_CATALOG_COMMIT {commit} is not a frozen catalog"
        ));
    }
    Ok(())
}

pub fn catalog_ledger(commit: &str) -> Result<CatalogLedger, String> {
    let matrix = support_matrix()?;
    validate_commit(commit)?;
    let rows = matrix
        .catalogs
        .iter()
        .filter(|catalog| catalog.commit == commit)
        .cloned()
        .collect::<Vec<_>>();
    match rows.as_slice() {
        [catalog] => Ok(catalog.clone()),
        [] => Err(format!(
            "PAIRED_CATALOG_COMMIT {commit} is not frozen in support-matrix.json"
        )),
        _ => Err(format!(
            "PAIRED_CATALOG_COMMIT {commit} is duplicated in support-matrix.json"
        )),
    }
}

pub fn card_row(commit: &str, revision: u16, display_name: &str) -> Result<CardLedger, String> {
    let matrix = support_matrix()?;
    validate_commit(commit)?;
    let rows = matrix
        .rows
        .iter()
        .filter(|row| {
            row.catalog_commit == commit
                && row.revision == revision
                && row.display_name == display_name
        })
        .cloned()
        .collect::<Vec<_>>();
    match rows.as_slice() {
        [row] => Ok(row.clone()),
        [] => Err(format!(
            "support matrix has no {display_name} row for catalog {commit} revision {revision}"
        )),
        _ => Err(format!(
            "support matrix has duplicate {display_name} rows for catalog {commit} revision {revision}"
        )),
    }
}

fn safe_catalog_path(root: &Path, relative: &str) -> Result<PathBuf, String> {
    let relative = Path::new(relative);
    if relative.is_absolute()
        || relative.components().any(|part| {
            matches!(
                part,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err(format!("unsafe support-matrix catalog path {relative:?}"));
    }
    Ok(root.join(relative))
}

pub fn hash_file(path: &Path) -> Result<String, String> {
    let bytes = std::fs::read(path).map_err(|error| format!("read {}: {error}", path.display()))?;
    Ok(script::JsCache::origin_sha(&bytes))
}

pub fn verify_source_identity(root: &Path, row: &CardLedger) -> Result<PathBuf, String> {
    let path = safe_catalog_path(root, &row.source_path)?;
    let actual = hash_file(&path)?;
    if actual != row.source_sha256 {
        return Err(format!(
            "source SHA-256 mismatch for {}: expected {}, got {}",
            path.display(),
            row.source_sha256,
            actual
        ));
    }
    Ok(path)
}

pub fn verify_registry_identity(root: &Path, catalog: &CatalogLedger) -> Result<PathBuf, String> {
    let path = safe_catalog_path(root, &catalog.registry_path)?;
    let actual = hash_file(&path)?;
    if actual != catalog.registry_sha256 {
        return Err(format!(
            "registry SHA-256 mismatch for {}: expected {}, got {}",
            path.display(),
            catalog.registry_sha256,
            actual
        ));
    }
    Ok(path)
}

pub fn prepare_card(
    root: &Path,
    temp: &Path,
    row: &CardLedger,
    display_name: &str,
) -> Result<PreparedCard, String> {
    let source_path = verify_source_identity(root, row)?;
    let mut library =
        script::JsLibrary::with_cache(temp.join("js-scripts.json"), temp.join("js-cache"));
    let registered = library.register_rs2b0t(root, &temp.join("rs2b0t-path"))?;
    if registered == 0 {
        return Err("frozen catalog registered no script cards".into());
    }
    library.ensure_js(script::ScriptSource::Catalog, display_name)?;
    let card = library
        .get(script::ScriptSource::Catalog, display_name)
        .cloned()
        .ok_or_else(|| format!("catalog registry has no {display_name} card"))?;
    let canonical_source = source_path
        .canonicalize()
        .map_err(|error| format!("canonicalize {}: {error}", source_path.display()))?;
    let canonical_card = card
        .path
        .canonicalize()
        .map_err(|error| format!("canonicalize {}: {error}", card.path.display()))?;
    if canonical_card != canonical_source {
        return Err(format!(
            "registry path mismatch for {display_name}: ledger {}, loader {}",
            source_path.display(),
            card.path.display()
        ));
    }
    if card.sha256 != row.source_sha256 {
        return Err(format!(
            "loader source hash mismatch for {display_name}: ledger {}, loader {}",
            row.source_sha256, card.sha256
        ));
    }
    if let Some(import) = &card.unloadable {
        return Err(format!("{display_name} is unloadable: {import}"));
    }
    let siblings = script::resolve_sibling_modules(
        &card.path,
        &card.origin,
        library.cache(),
        script::CacheMeta {
            kind: card.kind,
            source: card.source,
            shape: None,
        },
    )?;
    let sibling_hashes = siblings
        .iter()
        .map(|(url, js)| {
            json!({
                "module": url,
                "compiled_sha256": script::JsCache::origin_sha(js.as_bytes()),
            })
        })
        .collect::<Vec<_>>();
    Ok(PreparedCard {
        identity: json!({
            "card": display_name,
            "source_path": card.path,
            "source_sha256": card.sha256,
            "compiled_sha256": script::JsCache::origin_sha(card.js.as_bytes()),
            "siblings": sibling_hashes,
        }),
        js: card.js,
        shape: card.shape,
        siblings,
        schema: card.settings_schema,
    })
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

pub fn duel_settings(schema: &[script::SettingDef]) -> Map<String, Value> {
    script::merge_bag(schema, &Map::new(), None)
}

pub fn frozen_card_hashes_match(case: PairCase) -> Result<(), String> {
    for commit in [CATALOG_COMMIT_A, CATALOG_COMMIT_B] {
        for revision in [274_u16, 289] {
            let row = card_row(commit, revision, case.card_name())?;
            if row.source_sha256 != case.source_sha256() {
                return Err(format!(
                    "{} hash {} on {commit} r{revision} is not the frozen {}",
                    case.card_name(),
                    row.source_sha256,
                    case.source_sha256()
                ));
            }
        }
    }
    Ok(())
}
