//! Shared paired full-cycle witness used by the existing paired harness and
//! the opt-in headed pair-watch bridge.
//!
//! This is proof infrastructure, not a second gameplay engine. Duel types stay
//! here so the existing harness keeps one consumer; headed 157 cells are Air,
//! Mule, and Flax only.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use api::game_data::{DuelControls, SelectedGameData};
use api::snapshot::{GameSnapshot, ItemView, WidgetView};
use client::util::JString;
use serde::Serialize;
use serde_json::{json, Map, Value};

pub const CATALOG_COMMIT_A: &str = "100adccc037d9f6898080e1cad58fcfc43364775";
pub const CATALOG_COMMIT_B: &str = "8e7d965be2071d6ec65c3265e12af797082d720a";

pub const NATURECRAFTER: &str = "NatureCrafter";
pub const MULECRAFTER: &str = "MuleCrafter";
pub const FLAXRUNNER: &str = "FlaxRunner";
pub const DUEL_ARENA: &str = "Duel Arena Combat Trainer";
pub const NATURECRAFTER_SHA256: &str =
    "025ac395b25d64ef818cc0321478f0a2c84a051b79f99decbbfec5a9a2f0812a";
pub const MULECRAFTER_SHA256: &str =
    "bf745db4c0a3df22406b49c8a8b716b10e0594302853d80ca6f38862d05dc1f9";
pub const FLAXRUNNER_SHA256: &str =
    "6edae2ae773b73b907b5a3d4c020052075f7b32f9c2872ca78246eead9dfff34";
pub const DUEL_ARENA_SHA256: &str =
    "5656dabb30a47aac590fa1afadba19e689dd792d70da8dc4851e18d62e52d090";
pub const NATURE_RUNNER_LOGIC_SHA256: &str =
    "7a81b75a4cc4fde41d12565f5fe999de7931d88f2529da6d2d5e6a31f82d82f0";
pub const MULECRAFTER_LOGIC_SHA256: &str =
    "d9cc408c1857a02332e3338e1ae1e956dea51c7c191d4a66af81be6e5108251a";
pub const FLAXRUNNER_LOGIC_SHA256: &str =
    "05a07311383d8ab8121b45e0dcf56a3eefef27d94d8c23565d80fec2462cc32e";
pub const DUEL_ARENA_LOGIC_SHA256: &str =
    "325ce631a7a3f246ab0bc51e9b09945aaa018d7c8971b334994384f62cd1d8f2";
pub const DUEL_INTERFACE_SHA256: &str =
    "658e20f50117f0d23b8524e0ca389399d6fd52de2de1585313081324434634f9";

pub const ESSENCE_UNNOTED_ID: i32 = 1436;
pub const ESSENCE_NOTED_ID: i32 = 1437;
pub const AIR_RUNE_ID: i32 = 556;
pub const AIR_TALISMAN_ID: i32 = 1438;
pub const TRADE_CAP: i32 = 25;
pub const MULE_TRADE_CAP: i32 = 27;
pub const BANK_SEED_ESSENCE: i32 = 200;
pub const TEMPLE_Z: i32 = 4000;
pub const AIR_RUINS: (i32, i32, i32) = (2983, 3288, 0);
pub const FALADOR_EAST: (i32, i32, i32) = (3013, 3355, 0);
pub const FLAX_ID: i32 = 1779;
pub const BOW_STRING_ID: i32 = 1777;
pub const FLAX_MIN_CAPACITY: i32 = 24;
pub const FLAX_FIELD: (i32, i32, i32) = (2741, 3444, 0);
pub const FLAX_MEET: (i32, i32, i32) = (2719, 3471, 0);
pub const FLAX_BANK: (i32, i32, i32) = (2725, 3493, 0);
pub const FLAX_WHEEL: (i32, i32, i32) = (2711, 3471, 1);
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
    Mule,
    Flax,
    Duel,
}

impl PairCase {
    pub fn headed_cells() -> [Self; 3] {
        [Self::Air, Self::Mule, Self::Flax]
    }

    pub fn parse(name: &str) -> Result<Self, String> {
        match name {
            "nature_crafter_air" | "air_pair" => Ok(Self::Air),
            "mule_crafter_air" | "mule_pair" => Ok(Self::Mule),
            "flax_runner" | "flax_pair" => Ok(Self::Flax),
            other => Err(format!(
                "pair core watch does not accept {other:?}; headed cells are nature_crafter_air, mule_crafter_air, flax_runner"
            )),
        }
    }

    pub fn scenario_name(self) -> &'static str {
        match self {
            Self::Air => "nature_crafter_air",
            Self::Mule => "mule_crafter_air",
            Self::Flax => "flax_runner",
            Self::Duel => "duel_arena",
        }
    }

    pub fn card_name(self) -> &'static str {
        match self {
            Self::Air => NATURECRAFTER,
            Self::Mule => MULECRAFTER,
            Self::Flax => FLAXRUNNER,
            Self::Duel => DUEL_ARENA,
        }
    }

    pub fn source_sha256(self) -> &'static str {
        match self {
            Self::Air => NATURECRAFTER_SHA256,
            Self::Mule => MULECRAFTER_SHA256,
            Self::Flax => FLAXRUNNER_SHA256,
            Self::Duel => DUEL_ARENA_SHA256,
        }
    }

    pub fn is_headed_cell(self) -> bool {
        matches!(self, Self::Air | Self::Mule | Self::Flax)
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
pub enum MuleRole {
    Crafter,
    Mule,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FlaxRole {
    Runner,
    Spinner,
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

fn account_identity_eq(left: &str, right: &str) -> bool {
    let left = left.trim();
    let right = right.trim();
    if left.is_empty() || right.is_empty() {
        return false;
    }
    JString::to_userhash(left) == JString::to_userhash(right)
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
            if observation.crafting < 1 {
                return Err("spinner baseline Crafting must be at least 1".into());
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
    if !account_identity_eq(player, expected_player) {
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
                .is_some_and(|name| account_identity_eq(name, peer))
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
pub enum MuleClaim {
    FirstExchangeCraft,
    MuleBankReturnSecondCycle,
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
            .is_some_and(|player| !account_identity_eq(player, &self.expected_player))
        {
            self.mixed_identity = true;
        }
        if observation.trade_active() {
            if let Some(partner) = observation.trade_partner.as_deref() {
                if !partner.is_empty() && !account_identity_eq(partner, &self.partner) {
                    self.saw_wrong_partner = true;
                }
                if account_identity_eq(partner, &self.partner) {
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
    pub second_exchange_before_bank_return: bool,
    pub second_exchange_after_bank_return: bool,
    pub saw_bank_open_loaded: bool,
    pub saw_bank_at_falador: bool,
    pub restock_withdraw: bool,
    pub deposited_received_air: bool,
    pub returned_to_ruins: bool,
    pub craft_events: u32,
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
            second_exchange_before_bank_return: false,
            second_exchange_after_bank_return: false,
            saw_bank_open_loaded: false,
            saw_bank_at_falador: false,
            restock_withdraw: false,
            deposited_received_air: false,
            returned_to_ruins: false,
            craft_events: 0,
            air_from_script: 0,
            xp_from_script: 0,
        }
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
        let essence_out = (prev_ess - observation.essence_unnoted).max(0);
        let essence_in = (observation.essence_unnoted - prev_ess).max(0);
        let air_out = (prev_air - observation.air_runes).max(0);
        let air_in = (observation.air_runes - prev_air).max(0);
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
        match self.exchange_stage {
            MuleExchangeStage::Offer => {
                if observation.trade_offer_open && named_partner {
                    self.saw_offer_with_partner = true;
                    self.exchange_stage = MuleExchangeStage::Confirm;
                }
            }
            MuleExchangeStage::Confirm => {
                if observation.trade_confirm_open && named_partner {
                    self.saw_confirm_with_partner = true;
                    self.exchange_stage = MuleExchangeStage::Transfer;
                } else if !observation.trade_active() {
                    self.exchange_stage = MuleExchangeStage::Offer;
                }
            }
            MuleExchangeStage::Transfer => {
                if !observation.trade_active() {
                    let completed = at_ruins
                        && match self.role {
                            MuleRole::Crafter => essence_in > 0 && air_out > 0,
                            MuleRole::Mule => essence_out > 0 && air_in > 0,
                        };
                    if completed {
                        if self.role == MuleRole::Mule && self.partner_transfer_events == 1 {
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
                    }
                    self.exchange_stage = MuleExchangeStage::Offer;
                }
            }
        }
        if observation.essence_unnoted < prev_ess && observation.runecraft_xp > prev_xp {
            self.craft_events = self.craft_events.saturating_add(1);
            if self.role == MuleRole::Crafter
                && self.post_exchange_craft_events < self.partner_transfer_events
            {
                self.post_exchange_craft_events = self.post_exchange_craft_events.saturating_add(1);
            }
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
            .is_some_and(|player| !account_identity_eq(player, &self.expected_player))
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
            if !account_identity_eq(partner, &self.partner) {
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
        if account_identity_eq(&self.a.account, &self.b.account) {
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
        if !account_identity_eq(&self.a.partner, &self.b.expected_player)
            || !account_identity_eq(&self.b.partner, &self.a.expected_player)
        {
            return Err("wrong partner: configured partner is not the minted counterpart".into());
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

pub fn duel_settings(schema: &[script::SettingDef]) -> Map<String, Value> {
    script::merge_bag(schema, &Map::new(), None)
}

pub fn pair_settings(
    case: PairCase,
    schema: &[script::SettingDef],
    slot_index: usize,
    _self_name: &str,
    partner: &str,
) -> Result<Map<String, Value>, String> {
    match (case, slot_index) {
        (PairCase::Air, 0) => Ok(air_settings(schema, AirRole::Master, partner)),
        (PairCase::Air, 1) => Ok(air_settings(schema, AirRole::Runner, partner)),
        (PairCase::Mule, 0) => Ok(mule_settings(schema, MuleRole::Crafter, partner)),
        (PairCase::Mule, 1) => Ok(mule_settings(schema, MuleRole::Mule, partner)),
        (PairCase::Flax, 0) => Ok(flax_settings(schema, FlaxRole::Runner, partner)),
        (PairCase::Flax, 1) => Ok(flax_settings(schema, FlaxRole::Spinner, partner)),
        (PairCase::Duel, _) => Ok(duel_settings(schema)),
        _ => Err(format!(
            "pair settings require two headed slots for {case:?}, got index {slot_index}"
        )),
    }
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
        let flax_in = (observation.flax - prev_flax).max(0);
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
        match self.exchange_stage {
            FlaxExchangeStage::Offer => {
                if observation.trade_offer_open && named_partner {
                    self.saw_offer_with_partner = true;
                    self.exchange_stage = FlaxExchangeStage::Confirm;
                }
            }
            FlaxExchangeStage::Confirm => {
                if observation.trade_confirm_open && named_partner {
                    self.saw_confirm_with_partner = true;
                    self.exchange_stage = FlaxExchangeStage::Transfer;
                } else if !observation.trade_active() {
                    self.exchange_stage = FlaxExchangeStage::Offer;
                }
            }
            FlaxExchangeStage::Transfer => {
                if !observation.trade_active() {
                    let at_meet = near(observation.tile, FLAX_MEET, 8);
                    let completed = at_meet
                        && match self.role {
                            FlaxRole::Runner => flax_out > 0,
                            FlaxRole::Spinner => flax_in > 0,
                        };
                    if completed {
                        if self.role == FlaxRole::Spinner && self.partner_transfer_events == 1 {
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
                    }
                    self.exchange_stage = FlaxExchangeStage::Offer;
                }
            }
        }
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PairWatchStatus {
    Disabled,
    Ready,
    Running,
    Qualified,
    Failed,
}

#[derive(Clone, Default)]
pub struct PairWatch {
    active: Arc<AtomicBool>,
    inner: Arc<Mutex<PairWatchState>>,
}

struct SlotReady {
    account: String,
    latest_air: Option<AirObservation>,
    latest_flax: Option<FlaxObservation>,
}

enum PairRuntimeWitness {
    Air(AirPairWitness),
    Mule(MulePairWitness),
    Flax(FlaxPairWitness),
}

#[derive(Default)]
enum PairWatchState {
    #[default]
    Disabled,
    Ready {
        case: PairCase,
        a: Box<SlotReady>,
        b: Box<SlotReady>,
    },
    Running {
        a_account: String,
        b_account: String,
        witness: Box<PairRuntimeWitness>,
    },
    Qualified {
        evidence: Arc<Value>,
    },
    Failed {
        error: String,
        evidence: Option<Value>,
    },
}

impl PairWatch {
    pub fn configure(
        &self,
        case: PairCase,
        account_a: impl Into<String>,
        account_b: impl Into<String>,
    ) {
        let a = account_a.into();
        let b = account_b.into();
        *self.inner.lock().unwrap() = PairWatchState::Ready {
            case,
            a: Box::new(SlotReady {
                account: a,
                latest_air: None,
                latest_flax: None,
            }),
            b: Box::new(SlotReady {
                account: b,
                latest_air: None,
                latest_flax: None,
            }),
        };
        self.active.store(true, Ordering::Release);
    }

    pub fn clear(&self) {
        self.active.store(false, Ordering::Release);
        *self.inner.lock().unwrap() = PairWatchState::Disabled;
    }

    pub fn configured(&self) -> bool {
        self.active.load(Ordering::Acquire)
    }

    pub fn barrier(&self) -> StartBarrier {
        let state = self.inner.lock().unwrap();
        match &*state {
            PairWatchState::Disabled => StartBarrier::Wait,
            PairWatchState::Ready { case, a, b } => {
                let a_ok = slot_prepared(*case, true, a);
                let b_ok = slot_prepared(*case, false, b);
                shared_start_barrier(StartBarrierInput {
                    a_prepared: a_ok,
                    b_prepared: b_ok,
                    a_started: false,
                    b_started: false,
                    a_wait_ack: false,
                    b_wait_ack: false,
                    a_current_ok: a_ok,
                    b_current_ok: b_ok,
                })
            }
            PairWatchState::Running { .. }
            | PairWatchState::Qualified { .. }
            | PairWatchState::Failed { .. } => StartBarrier::Wait,
        }
    }

    pub fn begin_shared_start(&self, account_a: &str, account_b: &str) -> Result<(), String> {
        let mut state = self.inner.lock().unwrap();
        let current = std::mem::take(&mut *state);
        match current {
            PairWatchState::Disabled => {
                *state = PairWatchState::Disabled;
                Ok(())
            }
            PairWatchState::Ready { case, a, b }
                if a.account == account_a && b.account == account_b =>
            {
                match freeze_pair(case, &a, &b) {
                    Ok(witness) => {
                        *state = PairWatchState::Running {
                            a_account: account_a.to_string(),
                            b_account: account_b.to_string(),
                            witness: Box::new(witness),
                        };
                        Ok(())
                    }
                    Err(error) => {
                        *state = PairWatchState::Ready { case, a, b };
                        Err(error)
                    }
                }
            }
            PairWatchState::Ready { case, a, b } => {
                let error = format!(
                    "pair core Start slots {account_a:?}/{account_b:?} are not configured accounts {:?}/{:?}",
                    a.account, b.account
                );
                *state = PairWatchState::Ready { case, a, b };
                Err(error)
            }
            other => {
                *state = other;
                Err("pair core Start was armed more than once".into())
            }
        }
    }

    pub fn fail_start(&self, error: impl Into<String>) {
        let mut state = self.inner.lock().unwrap();
        let current = std::mem::take(&mut *state);
        *state = match current {
            PairWatchState::Running { witness, .. } => PairWatchState::Failed {
                error: error.into(),
                evidence: Some(runtime_evidence(&witness)),
            },
            other => other,
        };
    }

    pub fn observe_air(&self, account: &str, observation: AirObservation, session_boundary: bool) {
        if !self.active.load(Ordering::Acquire) {
            return;
        }
        let mut state = self.inner.lock().unwrap();
        observe_air_locked(&mut state, account, observation, session_boundary);
    }

    pub fn observe_flax(
        &self,
        account: &str,
        observation: FlaxObservation,
        session_boundary: bool,
    ) {
        if !self.active.load(Ordering::Acquire) {
            return;
        }
        let mut state = self.inner.lock().unwrap();
        observe_flax_locked(&mut state, account, observation, session_boundary);
    }

    pub fn observe_snapshot(&self, account: &str, snapshot: &GameSnapshot, session_boundary: bool) {
        if !self.active.load(Ordering::Acquire) {
            return;
        }
        let mut state = self.inner.lock().unwrap();
        let case = match &*state {
            PairWatchState::Ready { case, a, b }
                if a.account == account || b.account == account =>
            {
                Some(*case)
            }
            PairWatchState::Running {
                a_account,
                b_account,
                ..
            } if a_account == account || b_account == account => None,
            _ => return,
        };
        match case {
            Some(PairCase::Flax) => {
                observe_flax_locked(
                    &mut state,
                    account,
                    FlaxObservation::from_snapshot(snapshot),
                    session_boundary,
                );
            }
            Some(PairCase::Air | PairCase::Mule) | None => {
                // Running witnesses already know the case via their variant.
                if matches!(
                    &*state,
                    PairWatchState::Running {
                        witness,
                        ..
                    } if matches!(**witness, PairRuntimeWitness::Flax(_))
                ) {
                    observe_flax_locked(
                        &mut state,
                        account,
                        FlaxObservation::from_snapshot(snapshot),
                        session_boundary,
                    );
                } else if !matches!(
                    &*state,
                    PairWatchState::Ready {
                        case: PairCase::Duel,
                        ..
                    }
                ) {
                    observe_air_locked(
                        &mut state,
                        account,
                        AirObservation::from_snapshot(snapshot),
                        session_boundary,
                    );
                }
            }
            Some(PairCase::Duel) => {}
        }
    }

    pub fn status(&self) -> PairWatchStatus {
        if !self.active.load(Ordering::Acquire) {
            return PairWatchStatus::Disabled;
        }
        let mut state = self.inner.lock().unwrap();
        let should_cache = match &*state {
            PairWatchState::Running { witness, .. } => runtime_full_cycle(witness).is_ok(),
            _ => false,
        };
        if should_cache {
            let current = std::mem::take(&mut *state);
            if let PairWatchState::Running { witness, .. } = current {
                *state = PairWatchState::Qualified {
                    evidence: Arc::new(runtime_evidence(&witness)),
                };
            }
        }
        match &*state {
            PairWatchState::Disabled => PairWatchStatus::Disabled,
            PairWatchState::Ready { .. } => PairWatchStatus::Ready,
            PairWatchState::Running { .. } => PairWatchStatus::Running,
            PairWatchState::Qualified { .. } => PairWatchStatus::Qualified,
            PairWatchState::Failed { .. } => PairWatchStatus::Failed,
        }
    }

    pub fn failure(&self) -> Option<String> {
        match &*self.inner.lock().unwrap() {
            PairWatchState::Failed { error, .. } => Some(error.clone()),
            _ => None,
        }
    }

    pub fn qualify(&self) -> Result<Arc<Value>, String> {
        let mut state = self.inner.lock().unwrap();
        let current = std::mem::take(&mut *state);
        match current {
            PairWatchState::Running {
                a_account,
                b_account,
                witness,
            } => match runtime_full_cycle(&witness) {
                Ok(()) => {
                    let evidence = Arc::new(runtime_evidence(&witness));
                    *state = PairWatchState::Qualified {
                        evidence: Arc::clone(&evidence),
                    };
                    Ok(evidence)
                }
                Err(error) => {
                    *state = PairWatchState::Running {
                        a_account,
                        b_account,
                        witness,
                    };
                    Err(error)
                }
            },
            PairWatchState::Qualified { evidence } => {
                let receipt = Arc::clone(&evidence);
                *state = PairWatchState::Qualified { evidence };
                Ok(receipt)
            }
            failed @ PairWatchState::Failed { .. } => {
                let error = match &failed {
                    PairWatchState::Failed { error, .. } => error.clone(),
                    _ => unreachable!(),
                };
                *state = failed;
                Err(error)
            }
            ready @ PairWatchState::Ready { .. } => {
                *state = ready;
                Err("pair core Start not observed".into())
            }
            PairWatchState::Disabled => {
                *state = PairWatchState::Disabled;
                Err("pair core watch disabled".into())
            }
        }
    }

    pub fn evidence(&self) -> Value {
        match &*self.inner.lock().unwrap() {
            PairWatchState::Disabled => json!({"phase": "disabled"}),
            PairWatchState::Ready { case, a, b } => json!({
                "phase": "ready",
                "case": case,
                "a": a.account,
                "b": b.account,
            }),
            PairWatchState::Running { witness, .. } => json!({
                "phase": "running",
                "witness": runtime_evidence(witness),
                "qualification": runtime_full_cycle(witness).err(),
            }),
            PairWatchState::Qualified { evidence } => json!({
                "phase": "qualified",
                "receipt": evidence,
            }),
            PairWatchState::Failed { error, evidence } => json!({
                "phase": "failed",
                "error": error,
                "witness": evidence,
            }),
        }
    }
}

fn slot_prepared(case: PairCase, first: bool, slot: &SlotReady) -> bool {
    match case {
        PairCase::Air => slot.latest_air.as_ref().is_some_and(|obs| {
            air_prepared_current(
                if first {
                    AirRole::Master
                } else {
                    AirRole::Runner
                },
                &slot.account,
                obs,
            )
            .is_ok()
        }),
        PairCase::Mule => slot.latest_air.as_ref().is_some_and(|obs| {
            mule_prepared_current(
                if first {
                    MuleRole::Crafter
                } else {
                    MuleRole::Mule
                },
                &slot.account,
                obs,
            )
            .is_ok()
        }),
        PairCase::Flax => slot.latest_flax.as_ref().is_some_and(|obs| {
            flax_prepared_current(
                if first {
                    FlaxRole::Runner
                } else {
                    FlaxRole::Spinner
                },
                &slot.account,
                obs,
            )
            .is_ok()
        }),
        PairCase::Duel => false,
    }
}

fn freeze_pair(case: PairCase, a: &SlotReady, b: &SlotReady) -> Result<PairRuntimeWitness, String> {
    match case {
        PairCase::Air => {
            let Some(a_obs) = a.latest_air.clone() else {
                return Err("pair core has no published pre-Start observation".into());
            };
            let Some(b_obs) = b.latest_air.clone() else {
                return Err("pair core has no published pre-Start observation".into());
            };
            air_prepared_current(AirRole::Master, &a.account, &a_obs)?;
            air_prepared_current(AirRole::Runner, &b.account, &b_obs)?;
            Ok(PairRuntimeWitness::Air(AirPairWitness {
                master: AirSlotRecord::new(
                    AirRole::Master,
                    a.account.clone(),
                    a.account.clone(),
                    b.account.clone(),
                    Map::new(),
                    a_obs,
                ),
                runner: AirSlotRecord::new(
                    AirRole::Runner,
                    b.account.clone(),
                    b.account.clone(),
                    a.account.clone(),
                    Map::new(),
                    b_obs,
                ),
            }))
        }
        PairCase::Mule => {
            let Some(a_obs) = a.latest_air.clone() else {
                return Err("pair core has no published pre-Start observation".into());
            };
            let Some(b_obs) = b.latest_air.clone() else {
                return Err("pair core has no published pre-Start observation".into());
            };
            mule_prepared_current(MuleRole::Crafter, &a.account, &a_obs)?;
            mule_prepared_current(MuleRole::Mule, &b.account, &b_obs)?;
            Ok(PairRuntimeWitness::Mule(MulePairWitness {
                crafter: MuleSlotRecord::new(
                    MuleRole::Crafter,
                    a.account.clone(),
                    a.account.clone(),
                    b.account.clone(),
                    Map::new(),
                    a_obs,
                ),
                mule: MuleSlotRecord::new(
                    MuleRole::Mule,
                    b.account.clone(),
                    b.account.clone(),
                    a.account.clone(),
                    Map::new(),
                    b_obs,
                ),
            }))
        }
        PairCase::Flax => {
            let Some(a_obs) = a.latest_flax.clone() else {
                return Err("pair core has no published pre-Start observation".into());
            };
            let Some(b_obs) = b.latest_flax.clone() else {
                return Err("pair core has no published pre-Start observation".into());
            };
            flax_prepared_current(FlaxRole::Runner, &a.account, &a_obs)?;
            flax_prepared_current(FlaxRole::Spinner, &b.account, &b_obs)?;
            Ok(PairRuntimeWitness::Flax(FlaxPairWitness {
                runner: FlaxSlotRecord::new(
                    FlaxRole::Runner,
                    a.account.clone(),
                    a.account.clone(),
                    b.account.clone(),
                    Map::new(),
                    a_obs,
                ),
                spinner: FlaxSlotRecord::new(
                    FlaxRole::Spinner,
                    b.account.clone(),
                    b.account.clone(),
                    a.account.clone(),
                    Map::new(),
                    b_obs,
                ),
            }))
        }
        PairCase::Duel => {
            Err("headed pair core does not accept Duel; Duel stays on the existing harness".into())
        }
    }
}

fn observe_air_locked(
    state: &mut PairWatchState,
    account: &str,
    observation: AirObservation,
    session_boundary: bool,
) {
    let current = std::mem::take(state);
    *state = match current {
        PairWatchState::Ready { case, mut a, mut b }
            if a.account == account || b.account == account =>
        {
            if session_boundary {
                if a.account == account {
                    a.latest_air = None;
                } else {
                    b.latest_air = None;
                }
            } else if a.account == account {
                a.latest_air = Some(observation);
            } else {
                b.latest_air = Some(observation);
            }
            PairWatchState::Ready { case, a, b }
        }
        PairWatchState::Running {
            a_account,
            b_account,
            mut witness,
        } if a_account == account || b_account == account => {
            if session_boundary {
                PairWatchState::Failed {
                    error: "pair core session boundary after Start".into(),
                    evidence: Some(runtime_evidence(&witness)),
                }
            } else {
                match &mut *witness {
                    PairRuntimeWitness::Air(pair) if a_account == account => {
                        pair.master.observe(observation);
                    }
                    PairRuntimeWitness::Air(pair) => pair.runner.observe(observation),
                    PairRuntimeWitness::Mule(pair) if a_account == account => {
                        pair.crafter.observe(observation);
                    }
                    PairRuntimeWitness::Mule(pair) => pair.mule.observe(observation),
                    PairRuntimeWitness::Flax(_) => {}
                }
                PairWatchState::Running {
                    a_account,
                    b_account,
                    witness,
                }
            }
        }
        other => other,
    };
}

fn observe_flax_locked(
    state: &mut PairWatchState,
    account: &str,
    observation: FlaxObservation,
    session_boundary: bool,
) {
    let current = std::mem::take(state);
    *state = match current {
        PairWatchState::Ready { case, mut a, mut b }
            if a.account == account || b.account == account =>
        {
            if session_boundary {
                if a.account == account {
                    a.latest_flax = None;
                } else {
                    b.latest_flax = None;
                }
            } else if a.account == account {
                a.latest_flax = Some(observation);
            } else {
                b.latest_flax = Some(observation);
            }
            PairWatchState::Ready { case, a, b }
        }
        PairWatchState::Running {
            a_account,
            b_account,
            mut witness,
        } if a_account == account || b_account == account => {
            if session_boundary {
                PairWatchState::Failed {
                    error: "pair core session boundary after Start".into(),
                    evidence: Some(runtime_evidence(&witness)),
                }
            } else {
                if let PairRuntimeWitness::Flax(pair) = &mut *witness {
                    if a_account == account {
                        pair.runner.observe(observation);
                    } else {
                        pair.spinner.observe(observation);
                    }
                }
                PairWatchState::Running {
                    a_account,
                    b_account,
                    witness,
                }
            }
        }
        other => other,
    };
}

fn runtime_full_cycle(witness: &PairRuntimeWitness) -> Result<(), String> {
    match witness {
        PairRuntimeWitness::Air(pair) => pair.qualify_full_cycle().map(|_| ()),
        PairRuntimeWitness::Mule(pair) => pair.qualify_full_cycle().map(|_| ()),
        PairRuntimeWitness::Flax(pair) => pair.qualify_full_cycle().map(|_| ()),
    }
}

fn runtime_evidence(witness: &PairRuntimeWitness) -> Value {
    match witness {
        PairRuntimeWitness::Air(pair) => json!({
            "case": PairCase::Air,
            "supported": pair.qualify_supported().ok(),
            "full": pair.qualify_full_cycle().ok(),
            "master": pair.master,
            "runner": pair.runner,
        }),
        PairRuntimeWitness::Mule(pair) => json!({
            "case": PairCase::Mule,
            "supported": pair.qualify_supported().ok(),
            "full": pair.qualify_full_cycle().ok(),
            "crafter": pair.crafter,
            "mule": pair.mule,
        }),
        PairRuntimeWitness::Flax(pair) => json!({
            "case": PairCase::Flax,
            "supported": pair.qualify_supported().ok(),
            "full": pair.qualify_full_cycle().ok(),
            "runner": pair.runner,
            "spinner": pair.spinner,
        }),
    }
}
