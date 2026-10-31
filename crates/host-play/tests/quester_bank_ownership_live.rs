//! Focused Engine-Q proofs of Quester bank-close ownership.
//!
//! `quester_bank_duel_chest_owns_and_closes` exercises the selected Duel Arena
//! chest from its naturally closed state: the closed lid's `Open` changes the
//! loc but does not open a bank session; the open chest's `Bank` does, and the
//! Quester-owned provision trip closes that session. The shared chest is never
//! reset. The fixture waits for its content-defined 300-tick natural reset
//! before Start, then refuses to start if the closed lid remains unavailable.
//! `quester_bank_shilo_teller_owns_and_closes` proves the same owned close
//! through Shilo's declared NPC banker, with only this account's prerequisite
//! quest completed before Start.
//!
//!
//! `quester_bank_preserves_player_open_gundai_bank` opens Gundai's real teller
//! using observed NPC/chat choices before Quester Start, then proves Quester
//! withdraws the account's law rune without closing the pre-existing session.
//!
//! Run one selector at a time with the fixed 300 ms Engine Q and the normal
//! common Quester live environment; never set `QUESTER_TICK_MS` or send
//! `::speed`:
//!
//! ```text
//! LIVE=1 BOT_CPU=1 BOT_LIVE_NAME_PREFIX=qn WORLD_GAME_PORT=44694 WORLD_HTTP_PORT=1180 WORLD_NAV_PACK=<nav-pack> WORLD_ENGINE_DIR=<engine-Q-dir> RS2B0T=<catalog-root> BOT_CACHE_DIR=<owned-writable-cache-snapshot> LIVE_EVIDENCE_DIR=<evidence-root> cargo test -p host-play --test quester_bank_ownership_live --features "live-harness test-support live-probe" quester_bank_duel_chest_owns_and_closes -- --ignored --nocapture --test-threads=1
//! ```
//!
//! ```text
//! LIVE=1 BOT_CPU=1 BOT_LIVE_NAME_PREFIX=qn WORLD_GAME_PORT=44694 WORLD_HTTP_PORT=1180 WORLD_NAV_PACK=<nav-pack> WORLD_ENGINE_DIR=<engine-Q-dir> RS2B0T=<catalog-root> BOT_CACHE_DIR=<owned-writable-cache-snapshot> LIVE_EVIDENCE_DIR=<evidence-root> cargo test -p host-play --test quester_bank_ownership_live --features "live-harness test-support live-probe" quester_bank_preserves_player_open_gundai_bank -- --ignored --nocapture --test-threads=1
//! ```
//!
//! Source distinctions used here: `content/scripts/areas/area_mage_arena/scripts/gundai.rs2:1-11` opens the teller through its bank choice; `content/scripts/minigames/game_duelarena/configs/duelarena.loc:70-83` declares `Open` on the closed lid and `Bank` on the open chest; `content/scripts/minigames/game_duelarena/scripts/misc_locs.rs2:6-13` invokes `open_chest` only for the lid; `content/scripts/general_use/scripts/chests.rs2:63-69` changes the chest loc without opening a bank. The selected catalog is `crates/api/data/game-data/bank-catalog.rs:20-22`.
//!
//! `crates/host-play/src/bank_npc_live.rs` is a real native teller proof, but
//! its explicit native `Close` is not treated as Quester lease-close proof.
#![cfg(all(
    feature = "live-harness",
    feature = "test-support",
    feature = "live-probe"
))]

#[path = "common/quester_live.rs"]
mod quester_live;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use api::game_data::SelectedGameData;
use api::named_banks::{BankDefinition, BankOperation, NamedBankFacts, BANK_CATALOG};
use api::quest_facts::QuestCatalog;
use api::selected::{ClientRevision, FactKey, RunKey};
use api::snapshot::{GameSnapshot, LocView, WorldTile};
use quester_live::{Cell, Mode, ObserveFamily, StartFamily};
use scenario::quester::{quester_stage_with_seeds, QuesterStage};
use scenario::{Proof, Step, StepKind, Wait};
use script::native::{ScriptStatus, StatusValue};
use script::quester::compile::compile_path;
use script::quester::path::{
    PathDocument, PredicateDocument, ProgressColourDocument, ProgressDocument,
    ProgressRuleDocument, QuestBankDocument, QuestItemDocument, QuestItemKindDocument,
    SequenceDocument, SequenceOrder, StepDocument,
};
use script::quester::runner::Quester;
use serde_json::{json, Value};

const COOK_PATH_JSON: &str = include_str!("../../script/paths/289/cook.json");
const LAW_RUNE_ALIAS: &str = "law_rune";
const COOK_STAGE: &str = "cook:0";
const COOK_VARP: &str = "cookquest";
const COOK_DISPLAY: &str = "Cook's Assistant";
const GUNDAI_BANK_NAME: &str = "Mage Arena";
const GUNDAI_BANK_TILE: WorldTile = WorldTile {
    x: 2542,
    z: 4714,
    level: 0,
};
const GUNDAI_STAND: WorldTile = WorldTile {
    x: 2535,
    z: 4713,
    level: 0,
};
const DUEL_BANK_STAND: WorldTile = WorldTile {
    x: 3382,
    z: 3269,
    level: 0,
};
const SHILO_BANK_STAND: WorldTile = WorldTile {
    x: 2852,
    z: 2954,
    level: 0,
};
const GUNDAI_CHOICE_NEEDLE: &str = "I'd like to access my bank account";
const GUNDAI_CHOICE_TEXT: &str = "Cool, I'd like to access my bank account please.";
const BANK_SEED_COMMAND: &str = "givebank law_rune 1";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct SessionSample {
    tick: u32,
    open: bool,
    generation: u64,
}

impl SessionSample {
    fn from_snapshot(snapshot: &GameSnapshot) -> Self {
        Self {
            tick: snapshot.tick(),
            open: snapshot.bank_component_id() >= 0,
            generation: snapshot.bank_session_generation(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct BankStockSample {
    tick: u32,
    generation: u64,
    law_runes: i32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ChestState {
    Closed { id: i32, tile: WorldTile },
    Open { id: i32, tile: WorldTile },
    Missing,
    Ambiguous,
}

#[derive(Default)]
struct BankTrace {
    start_session: Option<SessionSample>,
    sessions: Vec<SessionSample>,
    bank_stock: Vec<BankStockSample>,
    inventory_law_runes: Option<i32>,
    start_inventory_law_runes: Option<i32>,
    start_bank_loaded: bool,
    start_bank_law_runes: Option<i32>,
    start_chest: Option<ChestState>,
    saw_open_lid_without_bank: bool,
    provision_phases: Vec<String>,
    status_failure: Option<String>,
    gundai_talk_sent: bool,
    gundai_choice_sent: bool,
    gundai_choice_text: Option<String>,
    start_teller_npc: Option<usize>,
}

#[derive(Clone, Copy)]
enum StartGuard {
    DuelClosedChest,
    ShiloTeller,
    PlayerOpenGundai,
}

struct BankCell {
    cell: Cell,
    start: StartFamily,
    trace: Arc<Mutex<BankTrace>>,
    law_rune_id: i32,
}

fn live_data() -> (Arc<SelectedGameData>, Arc<QuestCatalog>) {
    let selected =
        api::game_data::for_revision(ClientRevision::R289).expect("selected R289 game data");
    let quests = Arc::new(
        QuestCatalog::from_identity(selected.quest_identity()).expect("selected quest catalog"),
    );
    (selected, quests)
}

fn law_rune_id(selected: &SelectedGameData) -> i32 {
    selected
        .item_by_alias(LAW_RUNE_ALIAS)
        .unwrap_or_else(|| panic!("selected R289 data has no {LAW_RUNE_ALIAS} item"))
        .id
}

fn count_item(items: &[api::snapshot::ItemView], id: i32) -> i32 {
    items
        .iter()
        .filter(|item| item.def.id == id)
        .map(|item| item.count)
        .sum()
}

fn status_text<'a>(status: &'a ScriptStatus, key: &str) -> Option<&'a str> {
    status
        .fields
        .iter()
        .find(|field| field.key == key)
        .and_then(|field| match &field.value {
            StatusValue::Text(value) => Some(value.as_ref()),
            _ => None,
        })
}

fn loc_has_action(loc: &LocView, expected: &str) -> bool {
    loc.actions
        .iter()
        .flatten()
        .any(|action| action == expected)
}

fn near_bank_tile(loc: &LocView, bank: WorldTile) -> bool {
    loc.tile.level == bank.level
        && (loc.tile.x - bank.x).abs() <= 5
        && (loc.tile.z - bank.z).abs() <= 5
}

fn duel_chest_state(snapshot: &GameSnapshot) -> ChestState {
    let mut closed = snapshot.locs().iter().filter(|loc| {
        loc.name.as_deref() == Some("Closed chest")
            && loc_has_action(loc, "Open")
            && near_bank_tile(loc, DUEL_BANK_STAND)
    });
    let mut open = snapshot.locs().iter().filter(|loc| {
        loc.name.as_deref() == Some("Open chest")
            && loc_has_action(loc, "Bank")
            && near_bank_tile(loc, DUEL_BANK_STAND)
    });
    match (closed.next(), open.next(), closed.next(), open.next()) {
        (Some(loc), None, None, None) => ChestState::Closed {
            id: loc.id,
            tile: loc.tile,
        },
        (None, Some(loc), None, None) => ChestState::Open {
            id: loc.id,
            tile: loc.tile,
        },
        (None, None, None, None) => ChestState::Missing,
        _ => ChestState::Ambiguous,
    }
}

fn closed_lid_open_action(snapshot: &GameSnapshot, at: WorldTile) -> bool {
    snapshot.locs().iter().any(|loc| {
        loc.name.as_deref() == Some("Open chest") && loc.tile == at && loc_has_action(loc, "Bank")
    })
}

fn record_session(trace: &mut BankTrace, sample: SessionSample) {
    if trace.sessions.last().copied() != Some(sample) {
        trace.sessions.push(sample);
    }
}

fn record_bank_snapshot(trace: &mut BankTrace, snapshot: &GameSnapshot, law_id: i32) {
    let session = SessionSample::from_snapshot(snapshot);
    record_session(trace, session);
    trace.inventory_law_runes = Some(count_item(snapshot.inventory(), law_id));
    if session.open && snapshot.bank_loaded() {
        let sample = BankStockSample {
            tick: snapshot.tick(),
            generation: session.generation,
            law_runes: count_item(snapshot.bank(), law_id),
        };
        if trace.bank_stock.last().copied() != Some(sample) {
            trace.bank_stock.push(sample);
        }
    }
    if trace
        .start_chest
        .is_some_and(|state| matches!(state, ChestState::Closed { .. }))
        && matches!(duel_chest_state(snapshot), ChestState::Open { .. })
        && !session.open
    {
        let ChestState::Closed { tile, .. } = trace.start_chest.expect("checked closed chest")
        else {
            unreachable!("closed chest state changed while checking its tile")
        };
        trace.saw_open_lid_without_bank |= closed_lid_open_action(snapshot, tile);
    }
}

fn record_status(trace: &mut BankTrace, status: Option<&ScriptStatus>) {
    let Some(status) = status else {
        return;
    };
    if let Some(phase) = status_text(status, "provision") {
        if trace.provision_phases.last().map(String::as_str) != Some(phase) {
            trace.provision_phases.push(phase.to_owned());
        }
    }
    if let Some(failure) = status.failure.as_ref() {
        trace
            .status_failure
            .get_or_insert_with(|| format!("{failure:?}"));
    }
}

fn saw_phase(trace: &BankTrace, expected: &str) -> bool {
    trace.provision_phases.iter().any(|phase| phase == expected)
}

fn stock_transfer_seen(trace: &BankTrace, generation: u64) -> bool {
    trace
        .bank_stock
        .iter()
        .enumerate()
        .filter(|(_, sample)| sample.generation == generation && sample.law_runes == 1)
        .any(|(index, _)| {
            trace.bank_stock[index + 1..]
                .iter()
                .any(|sample| sample.generation == generation && sample.law_runes == 0)
        })
}

fn owned_open_then_close(trace: &BankTrace) -> Option<(u64, u64)> {
    let start = trace.start_session?;
    if start.open {
        return None;
    }
    for (open_index, opened) in trace.sessions.iter().enumerate().skip(1) {
        if !opened.open || opened.generation == start.generation {
            continue;
        }
        if let Some(closed) = trace
            .sessions
            .iter()
            .skip(open_index + 1)
            .find(|sample| !sample.open && sample.generation != opened.generation)
        {
            return Some((opened.generation, closed.generation));
        }
    }
    None
}

fn completed(lifecycle: Option<&script::ScriptLifecycleReceipt>) -> bool {
    lifecycle.is_some_and(|receipt| receipt.state == script::ScriptTerminalState::Completed)
}

fn own_close_predicate(trace: &BankTrace, lifecycle_completed: bool) -> bool {
    trace.saw_open_lid_without_bank && owned_transfer_predicate(trace, lifecycle_completed)
}

fn teller_close_predicate(trace: &BankTrace, lifecycle_completed: bool) -> bool {
    trace.start_teller_npc.is_some() && owned_transfer_predicate(trace, lifecycle_completed)
}

fn owned_transfer_predicate(trace: &BankTrace, lifecycle_completed: bool) -> bool {
    let Some((open_generation, close_generation)) = owned_open_then_close(trace) else {
        return false;
    };
    lifecycle_completed
        && saw_phase(trace, "Scanning")
        && saw_phase(trace, "Withdrawing")
        && saw_phase(trace, "Closing")
        && stock_transfer_seen(trace, open_generation)
        && close_generation != open_generation
        && trace.start_inventory_law_runes == Some(0)
        && trace.inventory_law_runes == Some(1)
        && trace.status_failure.is_none()
}

fn player_open_predicate(trace: &BankTrace, lifecycle_completed: bool) -> bool {
    let Some(start) = trace.start_session else {
        return false;
    };
    if !start.open {
        return false;
    }
    let remained_open = trace.sessions.len() > 1
        && trace
            .sessions
            .iter()
            .skip(1)
            .all(|sample| sample.open && sample.generation == start.generation);
    lifecycle_completed
        && trace.gundai_talk_sent
        && trace.gundai_choice_sent
        && trace.gundai_choice_text.as_deref() == Some(GUNDAI_CHOICE_TEXT)
        && trace.start_bank_loaded
        && trace.start_bank_law_runes == Some(1)
        && trace.start_inventory_law_runes == Some(0)
        && remained_open
        && saw_phase(trace, "Withdrawing")
        && !saw_phase(trace, "Closing")
        && stock_transfer_seen(trace, start.generation)
        && trace.inventory_law_runes == Some(1)
        && trace.status_failure.is_none()
}

fn trace_receipt(trace: &BankTrace) -> Value {
    let sessions: Vec<_> = trace
        .sessions
        .iter()
        .map(|sample| {
            json!({
                "tick": sample.tick,
                "open": sample.open,
                "generation": sample.generation,
            })
        })
        .collect();
    let bank_stock: Vec<_> = trace
        .bank_stock
        .iter()
        .map(|sample| {
            json!({
                "tick": sample.tick,
                "generation": sample.generation,
                "law_runes": sample.law_runes,
            })
        })
        .collect();
    json!({
        "start_session": trace.start_session.map(|sample| json!({
            "tick": sample.tick,
            "open": sample.open,
            "generation": sample.generation,
        })),
        "bank_sessions": sessions,
        "bank_stock": bank_stock,
        "inventory_law_runes": trace.inventory_law_runes,
        "start_inventory_law_runes": trace.start_inventory_law_runes,
        "start_bank_loaded": trace.start_bank_loaded,
        "start_bank_law_runes": trace.start_bank_law_runes,
        "start_chest": trace.start_chest.map(|state| format!("{state:?}")),
        "saw_open_lid_without_bank": trace.saw_open_lid_without_bank,
        "provision_phases": &trace.provision_phases,
        "status_failure": &trace.status_failure,
        "gundai_talk_sent": trace.gundai_talk_sent,
        "gundai_choice_sent": trace.gundai_choice_sent,
        "gundai_choice_text": &trace.gundai_choice_text,
        "start_teller_npc": trace.start_teller_npc,
    })
}

fn insert_account_seed(
    fixture: &mut scenario::quester::QuesterFixture,
    name: &'static str,
    command: String,
) {
    let relog = fixture
        .scenario
        .steps
        .iter()
        .position(|step| step.name == "relog after permanent quest stage and loadout seed")
        .expect("stage fixture has its final seed relog");
    let sent_command = command.clone();
    fixture.scenario.steps.insert(
        relog,
        Step {
            name,
            kind: StepKind::Perform {
                send: Box::new(move |client, _| {
                    api::interact::cheat(client, &sent_command).is_sent()
                }),
            },
            wait: Wait {
                arm: Proof::SideTabAvailable { index: 3 },
                budget_ticks: 100,
            },
        },
    );
    let teleport = fixture
        .seed_commands
        .len()
        .checked_sub(1)
        .expect("stage fixture records its stand teleport");
    fixture.seed_commands.insert(teleport, command);
}

fn step(
    id: &str,
    kind: &str,
    args: Value,
    advances: bool,
    skip_if: PredicateDocument,
    settle: PredicateDocument,
) -> StepDocument {
    StepDocument {
        id: FactKey::new(id),
        kind: kind.to_owned(),
        version: 1,
        args,
        comment: None,
        advances: Some(advances),
        max_attempts: None,
        stop: None,
        skip_if,
        settle,
    }
}

fn fact(kind: &str, args: Value) -> PredicateDocument {
    PredicateDocument::Fact {
        kind: kind.to_owned(),
        version: 1,
        args,
    }
}

fn empty_any() -> PredicateDocument {
    PredicateDocument::Any(Vec::new())
}

fn bank_path(bank: WorldTile, bank_required: bool) -> PathDocument {
    let mut document: PathDocument =
        serde_json::from_str(COOK_PATH_JSON).expect("decode released Cook Path envelope");
    document.id = FactKey::new("cook");
    document.display_name = "Quester bank ownership probe".to_owned();
    document.required.clear();
    document.tested_stats = None;
    let quest = document.quest.as_mut().expect("released Cook quest header");
    quest.members = false;
    quest.quest_points = 0;
    quest.requirements.clear();
    quest.items = vec![QuestItemDocument {
        obj: LAW_RUNE_ALIAS.to_owned(),
        qty: 1,
        kind: QuestItemKindDocument::MustHave,
        acquire: None,
        from_stage: None,
    }];
    quest.acquire.clear();
    quest.bank = QuestBankDocument::Tile {
        tile: [bank.x, bank.z, bank.level],
        source: match bank {
            DUEL_BANK_STAND => {
                "content/scripts/minigames/game_duelarena/configs/duelarena.loc:70-83".to_owned()
            }
            SHILO_BANK_STAND => {
                "content/scripts/interface_bank/configs/banker.npc:137-144; content/scripts/interface_bank/scripts/banker.rs2:5".to_owned()
            }
            GUNDAI_BANK_TILE => {
                "content/scripts/areas/area_mage_arena/scripts/gundai.rs2:1-11".to_owned()
            }
            _ => "api/data/game-data/bank-catalog.rs:20-22".to_owned(),
        },
        required: bank_required,
    };
    quest.coin_float = 0;
    quest.loadouts.clear();
    quest.areas.clear();
    quest.tools.clear();
    quest.owns_inventory = false;

    let have_law_rune = fact("has_item", json!({"obj": LAW_RUNE_ALIAS}));
    let role = document.roles.first_mut().expect("released Cook role");
    role.progress_binding = FactKey::new("journal:cook");
    role.progress = Some(ProgressDocument {
        colour: ProgressColourDocument {
            not_started: FactKey::new(COOK_STAGE),
            in_progress: FactKey::new(COOK_STAGE),
            complete: FactKey::new(COOK_STAGE),
        },
        rules: Vec::<ProgressRuleDocument>::new(),
        flags: Vec::new(),
        monotonic: false,
    });
    role.prelude.clear();
    role.sequences = vec![SequenceDocument {
        stage: FactKey::new(COOK_STAGE),
        required: Vec::new(),
        terminal: true,
        order: SequenceOrder::Authored,
        recovery_entry: None,
        cross: Vec::new(),
        steps: vec![step(
            "wait-for-provisioned-law-rune",
            "wait",
            json!({"until": have_law_rune, "max_ticks": 800}),
            false,
            empty_any(),
            fact("has_item", json!({"obj": LAW_RUNE_ALIAS})),
        )],
    }];
    document
}

fn selected_gundai_metadata(selected: &SelectedGameData) -> (i32, &'static BankDefinition) {
    let npc = selected
        .npc_by_config("magearena_banker")
        .expect("selected R289 Gundai NPC config");
    let definition = BANK_CATALOG
        .iter()
        .find(|row| row.name == GUNDAI_BANK_NAME)
        .expect("selected Mage Arena bank definition");
    let teller = definition.npc.expect("Mage Arena declares its teller");
    assert_eq!(teller.name, "Gundai");
    assert_eq!(teller.op, "Talk-to");
    assert_eq!(definition.choose, Some(GUNDAI_CHOICE_NEEDLE));
    assert!(GUNDAI_CHOICE_TEXT.contains(GUNDAI_CHOICE_NEEDLE));
    (npc.id, definition)
}

fn manual_continue_page(name: &'static str, arm: Proof) -> Step {
    Step {
        name,
        kind: StepKind::Perform {
            send: Box::new(|client, snapshot| {
                api::interact::Interactions::new(snapshot, client)
                    .continue_dialog(None)
                    .is_sent()
            }),
        },
        wait: Wait {
            arm,
            budget_ticks: 200,
        },
    }
}

fn insert_player_open_gundai_steps(
    scenario: &mut scenario::Scenario,
    selected: &SelectedGameData,
    trace: Arc<Mutex<BankTrace>>,
    law_id: i32,
) {
    let (gundai_id, definition) = selected_gundai_metadata(selected);
    let teller: BankOperation = definition.npc.expect("validated Gundai operation");
    let choice_needle = definition
        .choose
        .expect("validated Gundai bank-choice text")
        .to_owned();
    let talk_trace = Arc::clone(&trace);
    let choice_trace = Arc::clone(&trace);
    let talk = Step {
        name: "player opens Gundai's selected teller dialogue before Quester Start",
        kind: StepKind::Perform {
            send: Box::new(move |client, snapshot| {
                let Some(npc) = snapshot.npcs().iter().find(|npc| {
                    npc.r#type == usize::try_from(gundai_id).ok()
                        && npc.name.as_deref() == Some(teller.name)
                }) else {
                    return false;
                };
                let mut interactions = api::interact::Interactions::new(snapshot, client);
                let sent = matches!(
                    interactions.interact(
                        api::interact::OpTarget::Npc(npc),
                        api::interact::ActionSpec::Label(teller.op.to_owned()),
                    ),
                    api::interact::SendResult::Sent { .. }
                );
                if sent {
                    talk_trace
                        .lock()
                        .expect("bank witness mutex")
                        .gundai_talk_sent = true;
                }
                sent
            }),
        },
        wait: Wait {
            arm: Proof::ActiveContinue,
            budget_ticks: 200,
        },
    };
    // gundai.rs2:2-5 sends one player page, one NPC page, then the choices.
    let continue_player =
        manual_continue_page("player continues Gundai's greeting", Proof::ActiveContinue);
    let await_banker_page = Step {
        name: "observe Gundai's banker reply before its continuation",
        kind: StepKind::Await {
            evidence: "the current Gundai page says she is a banker",
            ready: Box::new(|snapshot| {
                snapshot
                    .chat_modal_texts()
                    .iter()
                    .any(|text| text.contains("I'm a banker"))
            }),
            summary: None,
        },
        wait: Wait {
            arm: Proof::ActiveContinue,
            budget_ticks: 200,
        },
    };
    let continue_banker =
        manual_continue_page("player continues Gundai's banker reply", Proof::ChatChoice);
    let choose = Step {
        name: "player selects Gundai's content-backed bank access choice",
        kind: StepKind::Perform {
            send: Box::new(move |client, snapshot| {
                let Some((index, option)) = snapshot
                    .chat_options()
                    .iter()
                    .enumerate()
                    .find(|(_, option)| option.text.contains(&choice_needle))
                else {
                    return false;
                };
                let Ok(choice) = i32::try_from(index + 1) else {
                    return false;
                };
                let text = option.text.clone();
                let mut interactions = api::interact::Interactions::new(snapshot, client);
                let sent = matches!(
                    interactions.answer_choice(choice),
                    api::interact::SendResult::Sent { .. }
                );
                if sent {
                    let mut witness = choice_trace.lock().expect("bank witness mutex");
                    witness.gundai_choice_sent = true;
                    witness.gundai_choice_text = Some(text);
                }
                sent
            }),
        },
        wait: Wait {
            arm: Proof::BankOpen,
            budget_ticks: 200,
        },
    };
    let await_bank = Step {
        name: "observe Gundai's loaded account bank before Quester Start",
        kind: StepKind::Await {
            evidence: "Gundai bank open with the seeded law rune and empty inventory",
            ready: Box::new(move |snapshot| {
                let open = snapshot.bank_component_id() >= 0;
                let loaded = snapshot.bank_loaded();
                let law_runes = count_item(snapshot.bank(), law_id);
                let inventory_law = count_item(snapshot.inventory(), law_id);
                open && loaded && law_runes == 1 && inventory_law == 0
            }),
            summary: Some(Box::new(|| {
                "waiting for observed Gundai bank session generation and law-rune row".to_owned()
            })),
        },
        wait: Wait {
            arm: Proof::BankOpen,
            budget_ticks: 200,
        },
    };
    let start_index = scenario
        .steps
        .iter()
        .position(|step| matches!(step.kind, StepKind::StartScript))
        .expect("stage fixture has a native Start step");
    scenario.steps.splice(
        start_index..start_index,
        [
            talk,
            continue_player,
            await_banker_page,
            continue_banker,
            choose,
            await_bank,
        ],
    );
}

fn family_start(
    document: PathDocument,
    selected: Arc<SelectedGameData>,
    quests: Arc<QuestCatalog>,
    trace: Arc<Mutex<BankTrace>>,
    guard: StartGuard,
) -> StartFamily {
    Box::new(move |handle, account, banks: &Arc<NamedBankFacts>| {
        {
            let witness = trace.lock().map_err(|_| "bank witness mutex poisoned")?;
            match guard {
                StartGuard::DuelClosedChest => {
                    if !matches!(witness.start_chest, Some(ChestState::Closed { .. })) {
                        return Err(format!(
                            "Duel Arena shared chest was not closed at pre-Start: {:?}; classified without resetting world or dispatching a Quester action",
                            witness.start_chest
                        ));
                    }
                    if !witness.start_session.is_some_and(|sample| !sample.open) {
                        return Err(
                            "Duel Arena fixture had no observed closed bank generation at pre-Start (or a player bank was open)".into(),
                        );
                    }
                }
                StartGuard::ShiloTeller => {
                    if witness.start_teller_npc.is_none()
                        || !witness.start_session.is_some_and(|sample| !sample.open)
                    {
                        return Err(
                            "Shilo fixture needs its real banker and an observed closed bank before Start"
                                .into(),
                        );
                    }
                }
                StartGuard::PlayerOpenGundai => {
                    let valid = witness.start_session.is_some_and(|sample| sample.open)
                        && witness.start_bank_loaded
                        && witness.start_bank_law_runes == Some(1)
                        && witness.start_inventory_law_runes == Some(0)
                        && witness.gundai_talk_sent
                        && witness.gundai_choice_sent;
                    if !valid {
                        return Err(format!(
                            "Gundai player-open fixture did not establish a loaded bank before Start: {}",
                            trace_receipt(&witness)
                        ));
                    }
                }
            }
        }
        let bytes = serde_json::to_vec(&document).map_err(|error| error.to_string())?;
        let path = api::selected::FamilyPreparation::run({
            let selected = Arc::clone(&selected);
            let quests = Arc::clone(&quests);
            move |cap| {
                compile_path(
                    &bytes,
                    &selected,
                    &quests,
                    cap,
                    &script::local_state::OperatorLoadouts::absent(),
                    None,
                )
            }
        })
        .map_err(|error| format!("start inline bank family preparation: {error}"))?
        .join()
        .map_err(|_| "inline bank family preparation panicked".to_owned())?
        .map_err(|error| format!("compile inline bank Path: {error:?}"))?;
        let script = Quester::new(
            RunKey {
                slot: 0,
                run: 0,
                session: 0,
            },
            path,
            Arc::clone(&selected),
            Arc::clone(&quests),
            Arc::clone(banks),
        );
        handle
            .start_test_script(account, Box::new(script), Some(Arc::clone(&selected)))
            .map(|_| ())
            .map_err(|error| format!("start inline bank Path: {error:?}"))
    })
}

fn build_bank_cell(
    name: &'static str,
    bank_tile: WorldTile,
    bank_required: bool,
    stand: WorldTile,
    guard: StartGuard,
    selected: Arc<SelectedGameData>,
    quests: Arc<QuestCatalog>,
) -> BankCell {
    let law_id = law_rune_id(&selected);
    let document = bank_path(bank_tile, bank_required);
    let identity = selected
        .quest_identity()
        .expect("selected R289 quest identity")
        .rows
        .iter()
        .find(|row| row.id == "cook")
        .expect("selected Cook identity");
    let mut fixture = quester_stage_with_seeds(
        QuesterStage {
            name,
            quest_display: COOK_DISPLAY,
            path: &document,
            identity,
            selected: &selected,
            stage: COOK_STAGE,
            loadout: None,
            extra_items: &[],
            stand,
        },
        &[(COOK_VARP, 0)],
    )
    .unwrap_or_else(|error| panic!("build bank ownership fixture: {error}"));
    fixture.scenario.settings.deadline = Duration::from_secs(300);
    insert_account_seed(
        &mut fixture,
        "seed only this account's bank before the final relog",
        BANK_SEED_COMMAND.to_owned(),
    );
    let teller_type = if matches!(guard, StartGuard::ShiloTeller) {
        let prerequisite = selected
            .quest_identity()
            .and_then(|identity| {
                identity
                    .rows
                    .iter()
                    .find(|row| row.display == "Shilo Village")
            })
            .expect("selected Shilo Village prerequisite");
        insert_account_seed(
            &mut fixture,
            "seed this account's Shilo Village bank access before relog",
            format!("setvar {} {}", prerequisite.varp, prerequisite.complete),
        );
        Some(
            selected
                .npc_by_config("shilobanker")
                .expect("selected Shilo banker type")
                .id,
        )
    } else {
        None
    };
    if matches!(guard, StartGuard::DuelClosedChest) {
        let start_index = fixture
            .scenario
            .steps
            .iter()
            .position(|step| matches!(step.kind, StepKind::StartScript))
            .expect("stage fixture has a native Start step");
        fixture.scenario.steps.insert(
            start_index,
            Step {
                name: "wait for the Duel Arena chest's natural closed state",
                kind: StepKind::Await {
                    evidence: "the real Duel Arena chest is closed before Quester Start",
                    ready: Box::new(|snapshot| {
                        matches!(duel_chest_state(snapshot), ChestState::Closed { .. })
                    }),
                    summary: Some(Box::new(|| {
                        "waiting for the natural 300-tick chest reset; chests.rs2:69".to_owned()
                    })),
                },
                wait: Wait {
                    arm: Proof::BankClosed,
                    budget_ticks: 300,
                },
            },
        );
    }

    let trace = Arc::new(Mutex::new(BankTrace::default()));
    if matches!(guard, StartGuard::PlayerOpenGundai) {
        insert_player_open_gundai_steps(
            &mut fixture.scenario,
            &selected,
            Arc::clone(&trace),
            law_id,
        );
    }

    let seed = fixture.seed;
    let observe_trace = Arc::clone(&trace);
    let observe_start: quester_live::ObserveStart = Box::new(move |snapshot| {
        let seed_receipt = seed.observe_start(snapshot)?;
        let sample = SessionSample::from_snapshot(snapshot);
        let inventory_law = count_item(snapshot.inventory(), law_id);
        let bank_loaded = snapshot.bank_loaded();
        let bank_law = bank_loaded.then(|| count_item(snapshot.bank(), law_id));
        let chest =
            matches!(guard, StartGuard::DuelClosedChest).then(|| duel_chest_state(snapshot));
        let mut witness = observe_trace
            .lock()
            .map_err(|_| "bank witness mutex poisoned")?;
        witness.start_session = Some(sample);
        witness.start_inventory_law_runes = Some(inventory_law);
        witness.start_bank_loaded = bank_loaded;
        witness.start_bank_law_runes = bank_law;
        witness.start_chest = chest;
        witness.start_teller_npc = teller_type.and_then(|id| {
            snapshot
                .npcs()
                .iter()
                .find(|npc| npc.r#type == usize::try_from(id).ok())
                .map(|npc| npc.index)
        });
        record_bank_snapshot(&mut witness, snapshot, law_id);
        Ok(json!({
            "fixture": seed_receipt,
            "bank_session": {"tick": sample.tick, "open": sample.open, "generation": sample.generation},
            "bank_loaded": bank_loaded,
            "bank_law_runes": bank_law,
            "inventory_law_runes": inventory_law,
            "duel_chest_state": chest.map(|state| format!("{state:?}")),
            "start_teller_npc": witness.start_teller_npc,
            "gundai_talk_sent": witness.gundai_talk_sent,
            "gundai_choice_sent": witness.gundai_choice_sent,
            "gundai_choice_text": witness.gundai_choice_text,
            "classification": if matches!(chest, Some(ChestState::Open { .. })) {
                "shared chest already open; no reset attempted"
            } else if matches!(chest, Some(ChestState::Closed { .. })) {
                "closed lid observed before Start"
            } else if matches!(guard, StartGuard::PlayerOpenGundai) {
                "player-opened Gundai bank observed before Start"
            } else if matches!(guard, StartGuard::ShiloTeller) {
                "Shilo teller access checked before Start"
            } else {
                "Duel Arena chest missing or ambiguous; no reset attempted"
            }
        }))
    });
    let start_guard = guard;
    let start = family_start(
        document,
        Arc::clone(&selected),
        Arc::clone(&quests),
        Arc::clone(&trace),
        start_guard,
    );
    BankCell {
        cell: Cell {
            quest: "cook",
            display: COOK_DISPLAY,
            label: name.to_owned(),
            scenario: fixture.scenario,
            start_settings: fixture.start_settings,
            mode: Mode::Stage { expect: Vec::new() },
            observe_start: Some(observe_start),
        },
        start,
        trace,
        law_rune_id: law_id,
    }
}

fn observe_own_close(cell: BankCell, guard: StartGuard) -> Value {
    let BankCell {
        cell,
        start,
        trace,
        law_rune_id,
    } = cell;
    let observe_trace = Arc::clone(&trace);
    let observe: ObserveFamily = Box::new(move |snapshot, status, lifecycle| {
        let mut witness = observe_trace
            .lock()
            .map_err(|_| "bank witness mutex poisoned")?;
        record_bank_snapshot(&mut witness, snapshot, law_rune_id);
        record_status(&mut witness, status);
        if lifecycle.is_some_and(|receipt| receipt.state != script::ScriptTerminalState::Completed)
        {
            return Err(format!(
                "Quester bank owner lifecycle was not Completed: lifecycle={lifecycle:?}; evidence={}",
                trace_receipt(&witness)
            ));
        }
        if completed(lifecycle) {
            let proved = match guard {
                StartGuard::DuelClosedChest => own_close_predicate(&witness, true),
                StartGuard::ShiloTeller => teller_close_predicate(&witness, true),
                StartGuard::PlayerOpenGundai => false,
            };
            if proved {
                let (open_generation, close_generation) = owned_open_then_close(&witness)
                    .expect("successful close predicate has bank generations");
                return Ok(Some(json!({
                    "proof_predicate": match guard {
                        StartGuard::ShiloTeller => "Quester opened through the real Shilo banker, withdrew law_rune, closed its own session, and retained Completed lifecycle",
                        _ => "Quester opened a new bank session, scanned and withdrew law_rune, observed the closed-lid Open without a bank session, closed its own session in a later generation, and retained Completed lifecycle",
                    },
                    "open_generation": open_generation,
                    "close_generation": close_generation,
                    "evidence": trace_receipt(&witness),
                    "lifecycle": lifecycle,
                })));
            }
            return Err(format!(
                "Completed is insufficient: missing Quester-owned open/close, access, stock, item, or Closing-phase evidence; evidence={}",
                trace_receipt(&witness)
            ));
        }
        Ok(None)
    });
    quester_live::run_family(cell, start, observe)
        .unwrap_or_else(|error| panic!("Quester owned-bank proof failed: {error}"))
}

fn observe_player_open(cell: BankCell) -> Value {
    let BankCell {
        cell,
        start,
        trace,
        law_rune_id,
    } = cell;
    let observe_trace = Arc::clone(&trace);
    let observe: ObserveFamily = Box::new(move |snapshot, status, lifecycle| {
        let mut witness = observe_trace
            .lock()
            .map_err(|_| "bank witness mutex poisoned")?;
        record_bank_snapshot(&mut witness, snapshot, law_rune_id);
        record_status(&mut witness, status);
        if lifecycle.is_some_and(|receipt| receipt.state != script::ScriptTerminalState::Completed)
        {
            return Err(format!(
                "Quester player-open bank lifecycle was not Completed: lifecycle={lifecycle:?}; evidence={}",
                trace_receipt(&witness)
            ));
        }
        if completed(lifecycle) {
            if player_open_predicate(&witness, true) {
                return Ok(Some(json!({
                    "proof_predicate": "a real Gundai bank opened by the player before Start remained open in the same generation while Quester withdrew law_rune; no Quester Closing phase; retained Completed lifecycle",
                    "evidence": trace_receipt(&witness),
                    "lifecycle": lifecycle,
                })));
            }
            return Err(format!(
                "Completed is insufficient: the player-open Gundai session was not preserved with exact-generation and item evidence; evidence={}",
                trace_receipt(&witness)
            ));
        }
        Ok(None)
    });
    quester_live::run_family(cell, start, observe)
        .unwrap_or_else(|error| panic!("Gundai player-open Quester proof failed: {error}"))
}

#[test]
#[ignore = "requires LIVE=1, fixed-300ms Engine Q, and the real Shilo Village banker"]
fn quester_bank_shilo_teller_owns_and_closes() {
    let (selected, quests) = live_data();
    let cell = build_bank_cell(
        "live-quester-bank-shilo-teller-owns-and-closes",
        SHILO_BANK_STAND,
        true,
        SHILO_BANK_STAND,
        StartGuard::ShiloTeller,
        selected,
        quests,
    );
    let receipt = observe_own_close(cell, StartGuard::ShiloTeller);
    assert_eq!(
        receipt["family_receipt"]["proof_predicate"],
        "Quester opened through the real Shilo banker, withdrew law_rune, closed its own session, and retained Completed lifecycle",
        "{receipt}"
    );
}

#[test]
#[ignore = "requires LIVE=1, fixed-300ms Engine Q, and the shared Duel Arena chest to be naturally closed before Start"]
fn quester_bank_duel_chest_owns_and_closes() {
    let (selected, quests) = live_data();
    let cell = build_bank_cell(
        "live-quester-bank-duel-chest-owns-and-closes",
        DUEL_BANK_STAND,
        true,
        DUEL_BANK_STAND,
        StartGuard::DuelClosedChest,
        selected,
        quests,
    );
    let receipt = observe_own_close(cell, StartGuard::DuelClosedChest);
    assert_eq!(
        receipt["family_receipt"]["proof_predicate"],
        "Quester opened a new bank session, scanned and withdrew law_rune, observed the closed-lid Open without a bank session, closed its own session in a later generation, and retained Completed lifecycle",
        "{receipt}"
    );
}

#[test]
#[ignore = "requires LIVE=1, fixed-300ms Engine Q, and a fresh account at Gundai's Mage Arena teller"]
fn quester_bank_preserves_player_open_gundai_bank() {
    let (selected, quests) = live_data();
    let cell = build_bank_cell(
        "live-quester-bank-preserves-player-open-gundai",
        GUNDAI_BANK_TILE,
        false,
        GUNDAI_STAND,
        StartGuard::PlayerOpenGundai,
        selected,
        quests,
    );
    let receipt = observe_player_open(cell);
    assert_eq!(
        receipt["family_receipt"]["proof_predicate"],
        "a real Gundai bank opened by the player before Start remained open in the same generation while Quester withdrew law_rune; no Quester Closing phase; retained Completed lifecycle",
        "{receipt}"
    );
}

#[cfg(test)]
mod oracle_tests {
    use super::*;

    fn base_owned_trace() -> BankTrace {
        BankTrace {
            start_session: Some(SessionSample {
                tick: 10,
                open: false,
                generation: 4,
            }),
            sessions: vec![
                SessionSample {
                    tick: 10,
                    open: false,
                    generation: 4,
                },
                SessionSample {
                    tick: 11,
                    open: true,
                    generation: 5,
                },
                SessionSample {
                    tick: 12,
                    open: false,
                    generation: 6,
                },
            ],
            bank_stock: vec![
                BankStockSample {
                    tick: 11,
                    generation: 5,
                    law_runes: 1,
                },
                BankStockSample {
                    tick: 12,
                    generation: 5,
                    law_runes: 0,
                },
            ],
            inventory_law_runes: Some(1),
            start_inventory_law_runes: Some(0),
            start_chest: Some(ChestState::Closed {
                id: 100,
                tile: DUEL_BANK_STAND,
            }),
            saw_open_lid_without_bank: true,
            provision_phases: vec![
                "Scanning".to_owned(),
                "Withdrawing".to_owned(),
                "Closing".to_owned(),
            ],
            ..BankTrace::default()
        }
    }

    #[test]
    fn own_close_oracle_rejects_player_open_explicit_close_and_false_completion() {
        let good = base_owned_trace();
        assert!(own_close_predicate(&good, true));

        let mut player_open = good;
        player_open.start_session = Some(SessionSample {
            tick: 10,
            open: true,
            generation: 4,
        });
        assert!(!own_close_predicate(&player_open, true));

        let mut no_provision_closer = base_owned_trace();
        no_provision_closer
            .provision_phases
            .retain(|phase| phase != "Closing");
        assert!(!own_close_predicate(&no_provision_closer, true));

        let no_lifecycle = base_owned_trace();
        assert!(!own_close_predicate(&no_lifecycle, false));

        let mut no_lid_evidence = base_owned_trace();
        no_lid_evidence.saw_open_lid_without_bank = false;
        assert!(!own_close_predicate(&no_lid_evidence, true));
    }

    #[test]
    fn teller_oracle_requires_real_teller_and_ordered_stock_transfer() {
        let mut teller = base_owned_trace();
        teller.saw_open_lid_without_bank = false;
        assert!(!teller_close_predicate(&teller, true));
        teller.start_teller_npc = Some(7);
        assert!(teller_close_predicate(&teller, true));
        assert!(!own_close_predicate(&teller, true));
        assert!(!teller_close_predicate(&teller, false));
        teller.bank_stock.reverse();
        assert!(!teller_close_predicate(&teller, true));
        teller.bank_stock.reverse();
        teller.start_inventory_law_runes = Some(1);
        assert!(!teller_close_predicate(&teller, true));
    }

    #[test]
    fn player_open_oracle_rejects_generation_change_close_and_false_completion() {
        let mut good = BankTrace {
            start_session: Some(SessionSample {
                tick: 20,
                open: true,
                generation: 9,
            }),
            sessions: vec![
                SessionSample {
                    tick: 20,
                    open: true,
                    generation: 9,
                },
                SessionSample {
                    tick: 21,
                    open: true,
                    generation: 9,
                },
            ],
            bank_stock: vec![
                BankStockSample {
                    tick: 20,
                    generation: 9,
                    law_runes: 1,
                },
                BankStockSample {
                    tick: 21,
                    generation: 9,
                    law_runes: 0,
                },
            ],
            inventory_law_runes: Some(1),
            start_inventory_law_runes: Some(0),
            start_bank_loaded: true,
            start_bank_law_runes: Some(1),
            provision_phases: vec!["Withdrawing".to_owned()],
            gundai_talk_sent: true,
            gundai_choice_sent: true,
            gundai_choice_text: Some(GUNDAI_CHOICE_TEXT.to_owned()),
            ..BankTrace::default()
        };
        assert!(player_open_predicate(&good, true));
        good.bank_stock.reverse();
        assert!(!player_open_predicate(&good, true));
        good.bank_stock.reverse();

        good.sessions.push(SessionSample {
            tick: 22,
            open: true,
            generation: 10,
        });
        assert!(!player_open_predicate(&good, true));
        good.sessions.last_mut().unwrap().open = false;
        assert!(!player_open_predicate(&good, true));

        let mut no_start = BankTrace::default();
        no_start.sessions.push(SessionSample {
            tick: 20,
            open: true,
            generation: 9,
        });
        assert!(!player_open_predicate(&no_start, true));

        let mut no_lifecycle = BankTrace {
            start_session: Some(SessionSample {
                tick: 20,
                open: true,
                generation: 9,
            }),
            sessions: vec![SessionSample {
                tick: 20,
                open: true,
                generation: 9,
            }],
            gundai_talk_sent: true,
            gundai_choice_sent: true,
            ..BankTrace::default()
        };
        assert!(!player_open_predicate(&no_lifecycle, false));

        no_lifecycle.status_failure = Some("Failed".to_owned());
        assert!(!player_open_predicate(&no_lifecycle, true));
    }
}
