//! Ignored live proof cells for Quester's shared Path action families.
//!
//! Each cell uses `common/quester_live.rs` for its host pump, retained-cache
//! copy, fresh local account, status log, and paired CPU PNG/JSON evidence.
//! Only the tested account is seeded before the native Quester Path starts.
//!
//! Run one cell at a time with Engine A already running and the cache source
//! kept immutable. The common runner documents all required environment.
//!
//! ```text
//! LIVE=1 BOT_CPU=1 BOT_LIVE_NAME_PREFIX=qn WORLD_GAME_PORT=44594 WORLD_HTTP_PORT=1080 WORLD_NAV_PACK=<nav-pack> WORLD_ENGINE_DIR=<engine-A-dir> RS2B0T=<catalog-root> BOT_CACHE_DIR=<owned-writable-cache-snapshot> LIVE_EVIDENCE_DIR=<evidence-root> cargo test -p host-play --test quester_families_live --features "live-harness test-support" live_quester_families_expected_combat -- --ignored --nocapture --test-threads=1
//! ```
#![cfg(all(feature = "live-harness", feature = "test-support"))]

#[path = "common/quester_live.rs"]
mod quester_live;

use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use api::game_data::SelectedGameData;
use api::quest_facts::QuestCatalog;
use api::selected::{ClientRevision, FactKey, RunKey};
use api::snapshot::{ActorKind, ActorTargetView, ChatLineView, GameSnapshot, WorldTile};
use quester_live::{Cell, Mode, ObserveFamily, StartFamily};
use scenario::quester::{quester_stage_with_seeds, FixtureLoadout, QuesterStage};
use scenario::{Proof, Step, StepKind, Wait};
use script::native::{NativePhase, ScriptStatus, StatusValue};
use script::quester::compile::compile_path;
use script::quester::path::{
    PathDocument, PredicateDocument, ProgressColourDocument, ProgressDocument,
    ProgressRuleDocument, QuestBankDocument, QuestLoadoutDocument, QuestRequirementDocument,
    QuestRequirementKindDocument, SequenceDocument, SequenceOrder, StepDocument,
};
use script::quester::runner::Quester;
use serde_json::{json, Value};

const COOK_PATH_JSON: &str = include_str!("../../script/paths/289/cook.json");
const CAPTAIN_TAUNT: &str = "Very well, if you're challenging me, let's get on with it!";
const BLACKCOG_SOURCE: &str = "live Clock Tower spawn observed at [2613,9639,0] before Start";
// desert_heat.rs2 drains water every 60 ticks in this iron kit. Twelve
// drinks cover the fixture's 300-second deadline without random thirst damage.
const DESERT_ITEMS: &[(&str, i32)] = &[("lobster", 6), ("water_skin4", 3)];
const DESERT_STAGE_REPLY: &str = "get desertrescue: 3";
const PRIEST_STAGE_REPLY: &str = "get priestperil: 1";
const COG_STAGE_REPLY: &str = "get cogquest: 1";
const PRIEST_DIALOGUE_COMPLETE: &str = "Okey-dokey, one dead dog coming up.";

const DEATH_IOU_ITEMS: &[(&str, i32)] = &[("death_iou", 1)];
const CLOCK_TOWER_ITEMS: &[(&str, i32)] = &[("bucket_water", 1)];
const MAIN_DOCUMENT_ITEMS: &[(&str, i32)] = &[("piratemessage", 1), ("the_shield_of_arrav", 1)];

const DESERT_STAND: WorldTile = WorldTile {
    x: 3270,
    z: 3029,
    level: 0,
};
const PRIEST_DOOR_STAND: WorldTile = WorldTile {
    x: 3406,
    z: 3488,
    level: 0,
};
const SAFE_STAND: WorldTile = WorldTile {
    x: 3209,
    z: 3215,
    level: 0,
};
const BLACKCOG_TILE: WorldTile = WorldTile {
    x: 2613,
    z: 9639,
    level: 0,
};
const BRONZE_COPPER_TILE: WorldTile = WorldTile {
    x: 2976,
    z: 3247,
    level: 0,
};
const BRONZE_TIN_TILE: WorldTile = WorldTile {
    x: 2984,
    z: 3237,
    level: 0,
};
const BRONZE_FURNACE_TILE: WorldTile = WorldTile {
    x: 2974,
    z: 3369,
    level: 0,
};
const BRONZE_ANVIL_TILE: WorldTile = WorldTile {
    x: 3188,
    z: 3425,
    level: 0,
};
const BRONZE_COPPER_SOURCE: &str = "content/maps/m46_50.jm2:6013 (copperrock1 at [2976,3247,0]); content/pack/loc.pack:2091; content/scripts/skill_mining/configs/mine.dbrow:23-34";
const BRONZE_TIN_SOURCE: &str = "content/maps/m46_50.jm2:6266 (tinrock1 at [2984,3237,0]); content/pack/loc.pack:2095; content/scripts/skill_mining/configs/mine.dbrow:36-47";
const BRONZE_FURNACE_SOURCE: &str = "content/maps/m46_52.jm2:7888 (furnace1 at [2975,3368,0]); content/scripts/skill_smithing/scripts/smelting/smelting.rs2:10-29,100-115; content/scripts/skill_smithing/configs/smelting/smelting.struct:1-15";
const BRONZE_ANVIL_SOURCE: &str = "content/maps/m49_53.jm2:7806 (anvil at [3188,3424,0]); content/pack/loc.pack:2784; content/scripts/skill_smithing/scripts/smithing/smithing.rs2:39-76,86-103";
const BRONZE_WIRE_MENU_SOURCE: &str = "content/scripts/skill_smithing/scripts/smithing/smithing.rs2:114-123; content/scripts/skill_smithing/configs/smithing/smithing.inv:30-35";
const BRONZE_WIRE_RECIPE_SOURCE: &str = "content/scripts/skill_smithing/configs/smithing/smithing.dbrow:1321-1330; content/scripts/skill_smithing/scripts/smithing/smithing.rs2:269-280";

fn bronze_wire_smithing_requirement() -> QuestRequirementDocument {
    QuestRequirementDocument {
        id: FactKey::new("smithing"),
        kind: QuestRequirementKindDocument::Skill {
            skill: "smithing".to_owned(),
            level: 4,
        },
        at: "Start".to_owned(),
        source: BRONZE_WIRE_RECIPE_SOURCE.to_owned(),
    }
}

fn shortbow_fletching_requirement() -> QuestRequirementDocument {
    QuestRequirementDocument {
        id: FactKey::new("fletching"),
        kind: QuestRequirementKindDocument::Skill {
            skill: "fletching".to_owned(),
            level: 5,
        },
        at: "Start".to_owned(),
        source: SHORTBOW_RECIPE_SOURCE.to_owned(),
    }
}

fn live_data() -> (Arc<SelectedGameData>, Arc<QuestCatalog>) {
    let selected = api::game_data::for_revision(ClientRevision::R289).expect("selected R289 data");
    let quests = Arc::new(
        QuestCatalog::from_identity(selected.quest_identity()).expect("selected quest catalog"),
    );
    (selected, quests)
}

fn quest_varp(selected: &SelectedGameData, quest: &str) -> (i32, String) {
    let row = selected
        .quest_identity()
        .expect("selected R289 quest identity")
        .rows
        .iter()
        .find(|row| row.id.eq_ignore_ascii_case(quest))
        .unwrap_or_else(|| panic!("selected quest identity does not include {quest:?}"));
    (row.varp_id, row.varp.clone())
}

fn item_id(selected: &SelectedGameData, alias: &str) -> i32 {
    selected
        .item_by_alias(alias)
        .unwrap_or_else(|| panic!("selected R289 items do not include {alias:?}"))
        .id
}

fn insert_target_quest_seed(
    fixture: &mut scenario::quester::QuesterFixture,
    varp: &str,
    stage: i32,
) {
    let command = format!("setvar {varp} {stage}");
    let relog = fixture
        .scenario
        .steps
        .iter()
        .position(|step| step.name == "relog after permanent quest stage and loadout seed")
        .expect("path-backed fixture has a post-seed relog");
    let step_command = command.clone();
    fixture.scenario.steps.insert(
        relog,
        Step {
            name: "stage target quest before relog",
            kind: StepKind::Perform {
                send: Box::new(move |client, _| {
                    api::interact::cheat(client, &step_command).is_sent()
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
        .expect("path-backed fixture records its teleport seed");
    fixture.seed_commands.insert(teleport, command);
}

fn make_cell(
    name: &'static str,
    (quest, display): (&'static str, &'static str),
    quest_stage: i32,
    items: &'static [(&'static str, i32)],
    stand: WorldTile,
    loadout: Option<FixtureLoadout<'_>>,
    selected: Arc<SelectedGameData>,
) -> (Cell, scenario::quester::QuesterSeed, Vec<String>) {
    make_cell_with_requirements(
        name,
        (quest, display, quest_stage),
        items,
        stand,
        loadout,
        selected,
        Vec::new(),
    )
}

fn make_cell_with_requirements(
    name: &'static str,
    (quest, display, quest_stage): (&'static str, &'static str, i32),
    items: &'static [(&'static str, i32)],
    stand: WorldTile,
    loadout: Option<FixtureLoadout<'_>>,
    selected: Arc<SelectedGameData>,
    requirements: Vec<QuestRequirementDocument>,
) -> (Cell, scenario::quester::QuesterSeed, Vec<String>) {
    // Cook is a Base40 profile-table key; the exact target quest state is
    // staged separately so every cell still begins at its authored target.
    let mut path = family_path(
        "cook",
        "Cook Base40 fixture root",
        "cook:0",
        Vec::new(),
        false,
    );
    path.quest
        .as_mut()
        .expect("released Cook quest header")
        .requirements = requirements;
    let identity = selected
        .quest_identity()
        .expect("selected R289 quest identity")
        .rows
        .iter()
        .find(|row| row.id == "cook")
        .expect("selected R289 Cook identity");
    let mut fixture = quester_stage_with_seeds(
        QuesterStage {
            name,
            quest_display: "Cook's Assistant",
            path: &path,
            identity,
            selected: &selected,
            stage: "cook:0",
            loadout,
            extra_items: items,
            stand,
        },
        &[(identity.varp.as_str(), 0)],
    )
    .unwrap_or_else(|error| panic!("build Cook-profile family fixture: {error}"));
    fixture.scenario.settings.deadline = Duration::from_secs(300);
    fixture
        .start_settings
        .insert("quests".into(), json!([quest]));
    if quest.eq_ignore_ascii_case("cook") {
        assert_eq!(quest_stage, 0, "Cook-profile path already seeds stage zero");
    } else {
        let (_, target_varp) = quest_varp(&selected, quest);
        insert_target_quest_seed(&mut fixture, &target_varp, quest_stage);
    }
    let seed = fixture.seed;
    let seed_commands = fixture.seed_commands;
    (
        Cell {
            quest,
            display,
            label: name.to_owned(),
            scenario: fixture.scenario,
            start_settings: fixture.start_settings,
            mode: Mode::Stage { expect: Vec::new() },
            observe_start: None,
        },
        seed,
        seed_commands,
    )
}

fn family_path(
    path_id: &str,
    display_name: &str,
    stage: &str,
    steps: Vec<StepDocument>,
    terminal: bool,
) -> PathDocument {
    // Keep a released R289 Path envelope and replace only its action sequence.
    let mut document: PathDocument =
        serde_json::from_str(COOK_PATH_JSON).expect("decode released Cook Path source");
    document.id = FactKey::new(path_id);
    document.display_name = display_name.to_owned();
    document.required.clear();
    document.tested_stats = None;
    let quest = document.quest.as_mut().expect("released Cook quest header");
    quest.members = false;
    quest.quest_points = 0;
    quest.requirements.clear();
    quest.items.clear();
    quest.acquire.clear();
    quest.bank = QuestBankDocument::Nearest(script::quester::path::NearestBankDocument::Nearest);
    quest.coin_float = 0;
    quest.loadouts.clear();
    quest.loadouts.insert(
        "family_melee".to_owned(),
        QuestLoadoutDocument {
            worn: [
                ("hat".to_owned(), "iron_full_helm".to_owned()),
                ("torso".to_owned(), "iron_platebody".to_owned()),
                ("legs".to_owned(), "iron_platelegs".to_owned()),
                ("lefthand".to_owned(), "iron_kiteshield".to_owned()),
                ("righthand".to_owned(), "iron_scimitar".to_owned()),
            ]
            .into_iter()
            .collect(),
            carry: Vec::new(),
        },
    );
    // Nature Spirit's Filliman dialogue needs the amulet worn
    // (`filliman.rs2:22-24`). A named loadout lets the fixture builder give
    // and wear it before Start through the normal kit path.
    quest.loadouts.insert(
        "filliman_wear".to_owned(),
        QuestLoadoutDocument {
            worn: [("front".to_owned(), "amulet_of_ghostspeak".to_owned())]
                .into_iter()
                .collect(),
            carry: Vec::new(),
        },
    );
    quest.areas.clear();
    quest.tools.clear();
    quest.owns_inventory = true;
    let stage_key = FactKey::new(stage);
    let role = document.roles.first_mut().expect("released Cook role");
    role.progress_binding = FactKey::new(&format!("journal:{path_id}"));
    role.progress = Some(ProgressDocument {
        colour: ProgressColourDocument {
            not_started: stage_key.clone(),
            in_progress: stage_key.clone(),
            complete: stage_key.clone(),
        },
        // Like the released Cook Path, use journal colour when no text rule is
        // needed. Every colour maps to this fixture's single authored stage.
        rules: Vec::new(),
        flags: Vec::new(),
        monotonic: false,
    });
    role.prelude.clear();
    role.sequences = vec![SequenceDocument {
        stage: FactKey::new(stage),
        required: Vec::new(),
        terminal,
        order: script::quester::path::SequenceOrder::Authored,
        recovery_entry: None,
        cross: Vec::new(),
        steps,
    }];
    document
}

#[test]
fn family_path_uses_valid_colour_fallback_instead_of_empty_text_rule() {
    let (selected, quests) = live_data();
    let compile = |document: &PathDocument| {
        let bytes = serde_json::to_vec(document).unwrap();
        let selected = Arc::clone(&selected);
        let quests = Arc::clone(&quests);
        api::selected::FamilyPreparation::run(move |cap| {
            compile_path(
                &bytes,
                &selected,
                &quests,
                cap,
                &script::local_state::OperatorLoadouts::absent(),
                None,
            )
        })
        .unwrap()
        .join()
        .unwrap()
    };
    let mut document = family_path(
        "cook",
        "Family progress regression",
        "cook:0",
        vec![step(
            "wait-in-combat",
            "wait",
            json!({"until": fact("in_combat", json!({})), "max_ticks": 10}),
            false,
            empty_any(),
            empty_all(),
        )],
        true,
    );
    assert!(compile(&document).is_ok());
    document.roles[0]
        .progress
        .as_mut()
        .unwrap()
        .rules
        .push(ProgressRuleDocument {
            stage: FactKey::new("cook:0"),
            all: Vec::new(),
            any: Vec::new(),
            not: Vec::new(),
            varp: Some(0),
        });
    let error = compile(&document)
        .err()
        .expect("empty text rule must stay invalid");
    assert_eq!(error.code.as_ref(), "invalid-progress-rule");
}

#[test]
fn family_fixture_keeps_the_explicit_cook_seed_without_progress_rules() {
    let (selected, _) = live_data();
    let (_, _, commands) = make_cell(
        "family-fixture-seed-regression",
        ("desertrescue", "The Tourist Trap"),
        3,
        DESERT_ITEMS,
        DESERT_STAND,
        None,
        selected,
    );
    assert!(commands
        .iter()
        .any(|command| command == "setvar cookquest 0"));
    assert!(commands
        .iter()
        .any(|command| command == "setvar desertrescue 3"));
    assert!(commands
        .iter()
        .any(|command| command == "give water_skin4 3"));
}

#[test]
fn computed_cog_progress_seeds_the_backing_varp() {
    let (selected, _) = live_data();
    let (_, _, commands) = make_cell(
        "computed-cog-seed-regression",
        ("cog", "Clock Tower"),
        1,
        CLOCK_TOWER_ITEMS,
        BLACKCOG_TILE,
        None,
        selected,
    );
    assert!(commands
        .iter()
        .any(|command| command == "setvar cogquest 1"));
    assert!(!commands
        .iter()
        .any(|command| command.contains("~get_cog_progress")));
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

fn empty_all() -> PredicateDocument {
    PredicateDocument::All(Vec::new())
}

fn family_start(
    document: PathDocument,
    selected: Arc<SelectedGameData>,
    quests: Arc<QuestCatalog>,
) -> StartFamily {
    Box::new(move |handle, account, banks| {
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
        .map_err(|error| format!("start inline family preparation: {error}"))?
        .join()
        .map_err(|_| "inline family preparation panicked".to_owned())?
        .map_err(|error| format!("compile inline family Path: {error:?}"))?;
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
            .map_err(|error| format!("start inline family Path: {error:?}"))
    })
}

fn tile(snapshot: &GameSnapshot) -> Option<WorldTile> {
    snapshot
        .tile()
        .map(|(x, z, level)| WorldTile { x, z, level })
}

fn within(here: WorldTile, target: WorldTile, radius: i32) -> bool {
    here.level == target.level
        && (here.x - target.x).abs() <= radius
        && (here.z - target.z).abs() <= radius
}

fn item_count(snapshot: &GameSnapshot, id: i32) -> i32 {
    snapshot
        .inventory()
        .iter()
        .filter(|item| item.def.id == id)
        .map(|item| item.count)
        .sum()
}

fn stat_base(snapshot: &GameSnapshot, name: &str) -> Option<i32> {
    snapshot
        .stats()
        .iter()
        .find(|stat| stat.name.eq_ignore_ascii_case(name))
        .map(|stat| stat.base)
}

fn varp_value(snapshot: &GameSnapshot, id: i32) -> Option<i32> {
    snapshot
        .varps()
        .iter()
        .find(|varp| varp.index == id)
        .map(|varp| varp.value)
}

fn quest_stage_witness<'a>(
    lines: &'a [ChatLineView],
    after: i32,
    expected: &str,
) -> Option<&'a ChatLineView> {
    lines.iter().find(|line| {
        line.sequence > after && line.type_ == 0 && line.username.is_none() && line.text == expected
    })
}

fn insert_quest_stage_query(
    cell: &mut Cell,
    command: &'static str,
    expected: &'static str,
    ground: Option<(i32, WorldTile)>,
) -> Arc<AtomicI32> {
    // Permanent server-only quest variables do not survive in the client
    // cache. Query the retained server value after relog, before native Start.
    let query_baseline = Arc::new(AtomicI32::new(i32::MAX));
    let send_baseline = Arc::clone(&query_baseline);
    let send = Box::new(move |client: &mut _, snapshot: &GameSnapshot| {
        // Repeat only observes the real spawn. It sends the query once.
        if send_baseline.load(Ordering::Relaxed) != i32::MAX {
            return true;
        }
        if ground.is_some_and(|(id, tile)| {
            !snapshot
                .ground_items()
                .iter()
                .any(|item| item.def.id == id && item.tile == tile)
        }) {
            return true;
        }
        send_baseline.store(
            snapshot
                .chat_lines()
                .iter()
                .map(|line| line.sequence)
                .max()
                .unwrap_or(0),
            Ordering::Relaxed,
        );
        api::interact::cheat(client, command).is_sent()
    });
    let kind = if ground.is_some() {
        StepKind::Repeat { send }
    } else {
        StepKind::Perform { send }
    };
    let start_step = cell
        .scenario
        .steps
        .iter()
        .position(|step| matches!(step.kind, StepKind::StartScript))
        .expect("family fixture has a native Start");
    cell.scenario.steps.insert(
        start_step,
        Step {
            name: "read retained quest stage before Start",
            kind,
            wait: Wait {
                arm: Proof::FreshChat { needle: expected },
                // Static objects respawn after 100 ticks (ObjType.ts:182).
                // Allow that interval plus the normal 100-tick query budget.
                budget_ticks: if ground.is_some() { 200 } else { 100 },
            },
        },
    );
    query_baseline
}

#[test]
fn quest_stage_oracle_requires_fresh_exact_server_reply() {
    for expected in [DESERT_STAGE_REPLY, PRIEST_STAGE_REPLY, COG_STAGE_REPLY] {
        let reply = ChatLineView {
            type_: 0,
            username: None,
            text: expected.to_owned(),
            sequence: 11,
        };
        assert!(quest_stage_witness(std::slice::from_ref(&reply), 10, expected).is_some());
        assert!(quest_stage_witness(std::slice::from_ref(&reply), 11, expected).is_none());
        assert!(quest_stage_witness(std::slice::from_ref(&reply), 12, expected).is_none());
        assert!(quest_stage_witness(&[], 10, expected).is_none());
        for text in [
            "get desertrescue: 0",
            "get desertrescue: 30",
            "get desertrescue_map_mechanisms: 3",
            "get priestperil: 0",
            "get priestperil: 10",
            "set desertrescue: to 3",
            "Please finish what you are doing first.",
        ] {
            let wrong = ChatLineView {
                text: text.to_owned(),
                ..reply.clone()
            };
            assert!(
                quest_stage_witness(&[wrong], 10, expected).is_none(),
                "{text}"
            );
        }
        let public = ChatLineView {
            type_: 2,
            ..reply.clone()
        };
        assert!(quest_stage_witness(&[public], 10, expected).is_none());
        let named = ChatLineView {
            username: Some("another player".to_owned()),
            ..reply
        };
        assert!(quest_stage_witness(&[named], 10, expected).is_none());
    }
}

fn require_item(
    snapshot: &GameSnapshot,
    id: i32,
    expected: i32,
    alias: &str,
) -> Result<(), String> {
    let actual = item_count(snapshot, id);
    if actual != expected {
        return Err(format!(
            "fixture expected {expected} {alias}, observed {actual}"
        ));
    }
    Ok(())
}

fn inventory_receipt(snapshot: &GameSnapshot) -> Vec<Value> {
    snapshot
        .inventory()
        .iter()
        .map(|item| json!({"id": item.def.id, "name": &item.def.name, "count": item.count}))
        .collect()
}

fn dialogue_texts_after(snapshot: &GameSnapshot, after: i32) -> Vec<String> {
    let mut texts: Vec<_> = snapshot
        .chat_options()
        .iter()
        .map(|option| option.text.clone())
        .chain(
            snapshot
                .chat_lines()
                .iter()
                .filter(|line| line.sequence > after)
                .map(|line| line.text.clone()),
        )
        .collect();
    // chat.rs2 splits one speech across adjacent chat widgets. Preserve the
    // rendered page as well as individual choices and system messages.
    let mut page = String::new();
    for text in snapshot
        .widgets()
        .iter()
        .filter(|widget| widget.root == api::snapshot::WidgetRoot::Chat)
        .filter_map(|widget| widget.text.as_deref())
    {
        if !page.is_empty() {
            page.push(' ');
        }
        page.push_str(text);
    }
    if !page.is_empty() {
        texts.push(page);
    }
    texts
}

fn remember_dialogue_texts(snapshot: &GameSnapshot, seen: &mut Vec<String>) {
    remember_dialogue_texts_after(snapshot, i32::MIN, seen);
}

fn remember_dialogue_texts_after(snapshot: &GameSnapshot, after: i32, seen: &mut Vec<String>) {
    for text in dialogue_texts_after(snapshot, after) {
        if !seen.iter().any(|old| old.eq_ignore_ascii_case(&text)) {
            seen.push(text);
        }
    }
}

fn saw_text(seen: &[String], expected: &str) -> bool {
    seen.iter()
        .any(|text| text.eq_ignore_ascii_case(expected) || text.contains(expected))
}

#[test]
fn dialogue_oracle_requires_the_complete_rendered_message() {
    let page = [
        "Mercenary Captain",
        "Very well, if you're challenging me,",
        "let's get on with it!",
        "Click here to continue",
    ]
    .join(" ");
    assert!(saw_text(&[page], CAPTAIN_TAUNT));
    assert!(!saw_text(
        &["Very well, if you're challenging me,".to_owned()],
        CAPTAIN_TAUNT,
    ));
}

fn priest_dialogue_witness(seen: &[String]) -> bool {
    [
        "Roald sent me to check on Drezel.",
        "Sure.",
        PRIEST_DIALOGUE_COMPLETE,
    ]
    .iter()
    .all(|text| saw_text(seen, text))
}

#[test]
fn priest_dialogue_oracle_requires_the_post_stage_assignment_line() {
    let complete = [
        "Roald sent me to check on Drezel.".to_owned(),
        "Sure.".to_owned(),
        PRIEST_DIALOGUE_COMPLETE.to_owned(),
    ];
    assert!(priest_dialogue_witness(&complete));
    for missing in 0..complete.len() {
        let incomplete = complete
            .iter()
            .enumerate()
            .filter(|(index, _)| *index != missing)
            .map(|(_, text)| text.clone())
            .collect::<Vec<_>>();
        assert!(!priest_dialogue_witness(&incomplete));
    }
    assert!(!priest_dialogue_witness(&[
        "Roald sent me to check on Drezel.".to_owned(),
        "Sure.".to_owned(),
        "Nope. Something about all this is very suspicious...".to_owned(),
    ]));
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

fn status_integer(status: Option<&ScriptStatus>, key: &str) -> Option<i64> {
    status?
        .fields
        .iter()
        .find(|field| field.key == key)
        .and_then(|field| match &field.value {
            StatusValue::Integer(value) => Some(*value),
            _ => None,
        })
}

fn completed_lifecycle(lifecycle: Option<&script::ScriptLifecycleReceipt>) -> bool {
    lifecycle.is_some_and(|receipt| receipt.state == script::ScriptTerminalState::Completed)
}

#[test]
fn completion_oracle_requires_a_retained_completed_lifecycle() {
    use script::{ScriptLifecycleReceipt, ScriptTerminalState};

    assert!(!completed_lifecycle(None));
    for state in [
        ScriptTerminalState::Completed,
        ScriptTerminalState::Stopped,
        ScriptTerminalState::Failed,
        ScriptTerminalState::Cancelled,
    ] {
        let receipt = ScriptLifecycleReceipt {
            runtime_generation: 1,
            state,
            tick: 35,
            reason: "oracle regression".to_owned(),
        };
        assert_eq!(
            completed_lifecycle(Some(&receipt)),
            state == ScriptTerminalState::Completed,
        );
    }
}

fn native_status_receipt(status: Option<&ScriptStatus>) -> Value {
    match status {
        Some(status) => json!({
            "phase": format!("{:?}", status.phase),
            "step_id": status_text(status, "step_id"),
            "failure": status.failure.as_ref().map(|failure| format!("{failure:?}")),
        }),
        None => Value::Null,
    }
}

fn common_observe_start(
    snapshot: &GameSnapshot,
    expected_tile: WorldTile,
) -> Result<WorldTile, String> {
    let here = tile(snapshot).ok_or("pre-Start snapshot has no local tile")?;
    if !within(here, expected_tile, 2) {
        return Err(format!(
            "fixture stood at {here:?}, expected near {expected_tile:?}"
        ));
    }
    Ok(here)
}

#[test]
#[ignore = "requires LIVE=1 and the already-running shared local Engine A"]
fn live_quester_families_expected_combat() {
    let (selected, quests) = live_data();
    let captain_id = selected
        .npc_by_config("desertminingcaptain")
        .expect("selected R289 desert mining captain config")
        .id;
    let (quest_varp_id, _) = quest_varp(&selected, "desertrescue");
    let (mut cell, seed, seed_commands) = make_cell(
        "live-quester-families-expected-combat",
        ("desertrescue", "The Tourist Trap"),
        3,
        DESERT_ITEMS,
        DESERT_STAND,
        Some(FixtureLoadout::Path("family_melee")),
        Arc::clone(&selected),
    );
    let query_baseline =
        insert_quest_stage_query(&mut cell, "getvar desertrescue", DESERT_STAGE_REPLY, None);
    let start_seed = seed;
    let start_selected = Arc::clone(&selected);
    cell.observe_start = Some(Box::new(move |snapshot| {
        let seed_receipt = start_seed.observe_start(snapshot)?;
        let here = common_observe_start(snapshot, DESERT_STAND)?;
        let stage_witness = quest_stage_witness(
            snapshot.chat_lines(),
            query_baseline.load(Ordering::Relaxed),
            DESERT_STAGE_REPLY,
        )
        .ok_or("Tourist Trap fixture needs a fresh server reply: get desertrescue: 3")?;
        let local = snapshot
            .local_player()
            .ok_or("pre-Start combat snapshot has no local player")?;
        if local.player.actor.in_combat {
            return Err("combat fixture began in combat before the talk step".into());
        }
        let captain_target_id = usize::try_from(captain_id)
            .map_err(|error| format!("invalid captain type id: {error}"))?;
        Ok(json!({
            "fixture": "desertrescue:3",
            "profile_seed": seed_receipt,
            "seed_commands": seed_commands,
            "tile": here,
            "quest_varp": [quest_varp_id, 3],
            "quest_varp_source": "post-relog getvar desertrescue",
            "quest_varp_witness": stage_witness,
            "client_varp_diagnostic": varp_value(snapshot, quest_varp_id),
            "local_in_combat_before_talk": local.player.actor.in_combat,
            "captain_type_id": captain_target_id,
            "stats": [
                ["attack", stat_base(snapshot, "attack")],
                ["strength", stat_base(snapshot, "strength")],
                ["defence", stat_base(snapshot, "defence")],
                ["hitpoints", stat_base(snapshot, "hitpoints")],
                ["prayer", stat_base(snapshot, "prayer")],
            ],
            "inventory": inventory_receipt(snapshot),
            "selected_revision": start_selected.revision().to_string(),
        }))
    }));

    let in_combat = fact("in_combat", json!({}));
    let document = family_path(
        "cook",
        "Expected combat family fixture",
        "cook:0",
        vec![
            step(
                "captain-handoff",
                "talk",
                json!({
                    "npc": "desertminingcaptain",
                    "prefer": [
                        "Wow! A real captain!",
                        "I'd love to work for a tough guy like you!",
                        "Can't I do something for a strong Captain like you?",
                        "Sorry Sir, I don't think I can do that.",
                        "It's a funny captain who can't fight his own battles!"
                    ],
                    "expect_combat": {"npc": "desertminingcaptain"}
                }),
                false,
                in_combat.clone(),
                in_combat.clone(),
            ),
            step(
                "safe-combat-wait",
                "wait",
                json!({"until": in_combat, "max_ticks": 10_000}),
                false,
                empty_any(),
                in_combat,
            ),
        ],
        false,
    );
    let start = family_start(document, Arc::clone(&selected), Arc::clone(&quests));
    let mut dialogue_seen = Vec::new();
    let mut saw_wait_step = false;
    let observe: ObserveFamily = Box::new(move |snapshot, status, _lifecycle| {
        if let Some(status) = status {
            saw_wait_step |= status_text(status, "step_id") == Some("safe-combat-wait");
        }
        remember_dialogue_texts(snapshot, &mut dialogue_seen);
        if !saw_wait_step || !saw_text(&dialogue_seen, CAPTAIN_TAUNT) {
            return Ok(None);
        }
        let Some(local) = snapshot.local_player() else {
            return Ok(None);
        };
        let local_index = local.player.index;
        let Some(target) = local.player.actor.target else {
            return Ok(None);
        };
        if !local.player.actor.in_combat || target.kind != ActorKind::Npc {
            return Ok(None);
        }
        let Some(captain) = snapshot.npcs().iter().find(|npc| {
            npc.r#type == Some(captain_id as usize)
                && npc.index == target.index
                && npc.target
                    == Some(ActorTargetView {
                        kind: ActorKind::Player,
                        index: local_index,
                    })
        }) else {
            return Ok(None);
        };
        Ok(Some(json!({
            "handoff_witness": "talk step completed; exact captain targets the local player",
            "talk_step_completed": saw_wait_step,
            "target_config": "desertminingcaptain",
            "npc_type_id": captain_id,
            "npc_index": captain.index,
            "local_player_index": local_index,
            "local_in_combat": local.player.actor.in_combat,
            "local_target": target,
            "captain_target": captain.target,
            "captain_in_combat_diagnostic": captain.in_combat,
            "captain_health_diagnostic": captain.health,
            "taunt_followup": CAPTAIN_TAUNT,
            "dialogue_text_seen": &dialogue_seen,
            "native_status": native_status_receipt(status),
        })))
    });
    quester_live::run_family(cell, start, observe)
        .unwrap_or_else(|error| panic!("expected combat family live proof failed: {error}"));
}

#[test]
#[ignore = "requires LIVE=1 and the already-running shared local Engine A"]
fn live_quester_families_loc_dialogue() {
    let (selected, quests) = live_data();
    let (quest_varp_id, _) = quest_varp(&selected, "priestperil");
    let door_id = selected
        .loc_by_config("priestperiltempledoorl")
        .expect("selected R289 Priest in Peril temple door config")
        .id;
    let (mut cell, seed, seed_commands) = make_cell(
        "live-quester-families-loc-dialogue",
        ("priestperil", "Priest in Peril"),
        1,
        &[],
        PRIEST_DOOR_STAND,
        None,
        Arc::clone(&selected),
    );
    let query_baseline =
        insert_quest_stage_query(&mut cell, "getvar priestperil", PRIEST_STAGE_REPLY, None);
    let start_seed = seed;
    cell.observe_start = Some(Box::new(move |snapshot| {
        let seed_receipt = start_seed.observe_start(snapshot)?;
        let here = common_observe_start(snapshot, PRIEST_DOOR_STAND)?;
        let stage_witness = quest_stage_witness(
            snapshot.chat_lines(),
            query_baseline.load(Ordering::Relaxed),
            PRIEST_STAGE_REPLY,
        )
        .ok_or("Priest in Peril fixture needs a fresh server reply: get priestperil: 1")?;
        let door = snapshot
            .locs()
            .iter()
            .find(|loc| loc.id == door_id)
            .ok_or("pre-Start snapshot did not observe priestperiltempledoorl")?;
        if door.distance > 2
            || !door
                .actions
                .iter()
                .flatten()
                .any(|action| action.eq_ignore_ascii_case("Knock-at"))
        {
            return Err(format!(
                "priestperiltempledoorl was not a nearby Knock-at target: {door:?}"
            ));
        }
        Ok(json!({
            "fixture": "priestperil:1",
            "profile_seed": seed_receipt,
            "seed_commands": seed_commands,
            "tile": here,
            "quest_varp": [quest_varp_id, 1],
            "quest_varp_source": "post-relog getvar priestperil",
            "quest_varp_witness": stage_witness,
            "client_varp_diagnostic": varp_value(snapshot, quest_varp_id),
            "loc": {"config": "priestperiltempledoorl", "id": door.id, "name": &door.name,
                    "tile": door.tile, "distance": door.distance, "actions": &door.actions},
        }))
    }));
    let document = family_path(
        "cook",
        "Priest in Peril loc dialogue family fixture",
        "cook:0",
        vec![step(
            "knock-temple-door",
            "interact",
            json!({
                "target": {"loc": "priestperiltempledoorl"},
                "op": "Knock-at",
                "anchor": {
                    "tile": [3406, 3488, 0],
                    "source": "R289 Priest in Peril door witness at [3406,3488,0]"
                },
                "radius": 2,
                "wait_if_missing": true,
                "dialogue": {
                    "prefer": ["Roald sent me to check on Drezel.", "Sure."]
                }
            }),
            false,
            empty_any(),
            empty_all(),
        )],
        true,
    );
    let start = family_start(document, Arc::clone(&selected), Arc::clone(&quests));
    let mut dialogue_seen = Vec::new();
    let observe: ObserveFamily = Box::new(move |snapshot, status, _lifecycle| {
        remember_dialogue_texts(snapshot, &mut dialogue_seen);
        // temple_doors.rs2:79 assigns stage 2 before this branch's final line.
        // The server never transmits priestperil, so its client cache stays 0.
        if !priest_dialogue_witness(&dialogue_seen) || snapshot.modals().chat != -1 {
            return Ok(None);
        }
        Ok(Some(json!({
            "family_step": "interact",
            "target_config": "priestperiltempledoorl",
            "operation": "Knock-at",
            "preferred_dialogue_choices_observed": [
                "Roald sent me to check on Drezel.", "Sure."
            ],
            "dialogue_text_seen": &dialogue_seen,
            "quest_stage": "priestperil_agree_to_kill_dog",
            "quest_stage_source": "temple_doors.rs2:77-82",
            "quest_stage_witness": PRIEST_DIALOGUE_COMPLETE,
            "client_varp_diagnostic": [quest_varp_id, varp_value(snapshot, quest_varp_id)],
            "native_status": native_status_receipt(status),
        })))
    });
    quester_live::run_family(cell, start, observe)
        .unwrap_or_else(|error| panic!("loc dialogue family live proof failed: {error}"));
}

#[test]
#[ignore = "requires LIVE=1 and the already-running shared local Engine A"]
fn live_quester_families_held() {
    let (selected, quests) = live_data();
    let iou_id = item_id(&selected, "death_iou");
    let combination_id = item_id(&selected, "death_combination");
    let (mut cell, seed, seed_commands) = make_cell(
        "live-quester-families-held-death-iou",
        ("cook", "Cook's Assistant"),
        0,
        DEATH_IOU_ITEMS,
        SAFE_STAND,
        None,
        Arc::clone(&selected),
    );
    let start_seed = seed;
    cell.observe_start = Some(Box::new(move |snapshot| {
        let seed_receipt = start_seed.observe_start(snapshot)?;
        let here = common_observe_start(snapshot, SAFE_STAND)?;
        require_item(snapshot, iou_id, 1, "death_iou")?;
        require_item(snapshot, combination_id, 0, "death_combination")?;
        if varp_value(snapshot, script::combat::OPTION_NODEF) != Some(0) {
            return Err(
                "held fixture requires observed auto-retaliate enabled before Start".into(),
            );
        }
        Ok(json!({
            "fixture": "held Death IOU read",
            "profile_seed": seed_receipt,
            "seed_commands": seed_commands,
            "tile": here,
            "held_target": {"config": "death_iou", "id": iou_id, "count": item_count(snapshot, iou_id)},
            "result_before": {"config": "death_combination", "id": combination_id,
                              "count": item_count(snapshot, combination_id)},
            "inventory": inventory_receipt(snapshot),
        }))
    }));
    let document = family_path(
        "cook",
        "Death IOU held read family fixture",
        "cook:0",
        vec![
            step(
                "disable-auto-retaliate",
                "setting",
                json!({"retaliate": false}),
                false,
                PredicateDocument::Any(vec![
                    fact("has_item", json!({"obj": "death_combination"})),
                    fact("retaliate", json!({"retaliate": false})),
                ]),
                fact("retaliate", json!({"retaliate": false})),
            ),
            step(
                "read-death-iou",
                "interact",
                json!({
                    "target": {"held": "death_iou"},
                    "op": "Read",
                    "dialogue": "continue"
                }),
                false,
                fact("has_item", json!({"obj": "death_combination"})),
                fact("has_item", json!({"obj": "death_combination"})),
            ),
            step(
                "enable-auto-retaliate",
                "setting",
                json!({"retaliate": true}),
                false,
                fact("retaliate", json!({"retaliate": true})),
                fact("retaliate", json!({"retaliate": true})),
            ),
        ],
        true,
    );
    let start = family_start(document, Arc::clone(&selected), Arc::clone(&quests));
    let mut saw_retaliate_off = false;
    let observe: ObserveFamily = Box::new(move |snapshot, status, lifecycle| {
        let retaliate = varp_value(snapshot, script::combat::OPTION_NODEF);
        saw_retaliate_off |= retaliate == Some(1);
        let source_count = item_count(snapshot, iou_id);
        let result_count = item_count(snapshot, combination_id);
        let path_completed = completed_lifecycle(lifecycle);
        if !path_completed
            || source_count != 0
            || result_count != 1
            || !saw_retaliate_off
            || retaliate != Some(0)
            || snapshot.modals().main != -1
            || snapshot.modals().chat != -1
        {
            return Ok(None);
        }
        Ok(Some(json!({
            "family_step": "interact",
            "target": {"held": "death_iou", "id": iou_id},
            "operation": "Read",
            "result": {"config": "death_combination", "id": combination_id,
                       "count": result_count},
            "source_count_after": source_count,
            "setting": {"observed_off": saw_retaliate_off, "option_nodef_after": retaliate},
            "main_modal_after": snapshot.modals().main,
            "native_status": native_status_receipt(status),
            "lifecycle": lifecycle,
        })))
    });
    quester_live::run_family(cell, start, observe)
        .unwrap_or_else(|error| panic!("held item family live proof failed: {error}"));
}

#[test]
#[ignore = "requires LIVE=1 and the already-running shared local Engine A"]
fn live_quester_families_ground_use_on() {
    let (selected, quests) = live_data();
    let water_id = item_id(&selected, "bucket_water");
    let empty_bucket_id = item_id(&selected, "bucket_empty");
    let cog_id = item_id(&selected, "blackcog");
    let (_, cog_varp) = quest_varp(&selected, "cog");
    assert_eq!(cog_varp, "cogquest", "selected Clock Tower progress varp");
    let (mut cell, seed, seed_commands) = make_cell(
        "live-quester-families-ground-use-on",
        ("cog", "Clock Tower"),
        1,
        CLOCK_TOWER_ITEMS,
        BLACKCOG_TILE,
        None,
        Arc::clone(&selected),
    );
    let query_baseline = insert_quest_stage_query(
        &mut cell,
        "getvar cogquest",
        COG_STAGE_REPLY,
        Some((cog_id, BLACKCOG_TILE)),
    );
    let start_seed = seed;
    let observe_selected = Arc::clone(&selected);
    cell.observe_start = Some(Box::new(move |snapshot| {
        let seed_receipt = start_seed.observe_start(snapshot)?;
        let here = common_observe_start(snapshot, BLACKCOG_TILE)?;
        let stage_witness = quest_stage_witness(
            snapshot.chat_lines(),
            query_baseline.load(Ordering::Relaxed),
            COG_STAGE_REPLY,
        )
        .ok_or("Clock Tower fixture needs a fresh server reply: get cogquest: 1")?;
        require_item(snapshot, water_id, 1, "bucket_water")?;
        require_item(snapshot, empty_bucket_id, 0, "bucket_empty")?;
        require_item(snapshot, cog_id, 0, "blackcog")?;
        // Selected R289 exports blackcog's exact item identity but no
        // source-map spawn row. Fail closed unless the real pre-Start scene
        // snapshot proves that exact item at the authored tile and exposes Take.
        let ground = snapshot
            .ground_items()
            .iter()
            .find(|item| item.def.id == cog_id && item.tile == BLACKCOG_TILE)
            .ok_or("pre-Start live scene lacks blackcog at [2613,9639,0]")?;
        if !ground
            .actions
            .iter()
            .flatten()
            .any(|action| action.eq_ignore_ascii_case("Take"))
        {
            return Err(format!(
                "blackcog at {:?} did not expose Take: {:?}",
                ground.tile, ground.actions
            ));
        }
        Ok(json!({
            "fixture": "cog:1",
            "profile_seed": seed_receipt,
            "seed_commands": seed_commands,
            "tile": here,
            "quest_progress": 1,
            "quest_varp_config": "cogquest",
            "quest_varp_source": "post-relog getvar cogquest",
            "quest_varp_witness": stage_witness,
            "bucket_water": {"id": water_id, "count": item_count(snapshot, water_id)},
            "target": {"config": "blackcog", "id": ground.def.id, "count": ground.count,
                       "tile": ground.tile, "distance": ground.distance, "actions": &ground.actions},
            "target_source": BLACKCOG_SOURCE,
            "selected_revision": observe_selected.revision().to_string(),
        }))
    }));
    let document = family_path(
        "cook",
        "Clock Tower ground use-on family fixture",
        "cook:0",
        vec![
            step(
                "use-water-on-blackcog",
                "use_on",
                json!({
                    "item": "bucket_water",
                    "target": {
                        "ground": "blackcog",
                        "tile": [2613, 9639, 0],
                        "source": BLACKCOG_SOURCE
                    },
                    "dialogue": "continue"
                }),
                false,
                fact("has_item", json!({"obj": "bucket_empty"})),
                fact("has_item", json!({"obj": "bucket_empty"})),
            ),
            step(
                "take-blackcog",
                "interact",
                json!({
                    "target": {"ground": "blackcog"},
                    "op": "Take",
                    "anchor": {
                        "tile": [2613, 9639, 0],
                        "source": BLACKCOG_SOURCE
                    },
                    "radius": 2,
                    "wait_if_missing": true
                }),
                false,
                fact("has_item", json!({"obj": "blackcog"})),
                fact("has_item", json!({"obj": "blackcog"})),
            ),
        ],
        true,
    );
    let start = family_start(document, Arc::clone(&selected), Arc::clone(&quests));
    let mut saw_take_step = false;
    let observe: ObserveFamily = Box::new(move |snapshot, status, _lifecycle| {
        if let Some(status) = status {
            saw_take_step |= status_text(status, "step_id") == Some("take-blackcog");
        }
        let held_cog = item_count(snapshot, cog_id);
        if !saw_take_step || held_cog != 1 {
            return Ok(None);
        }
        Ok(Some(json!({
            "family_steps": ["use-water-on-blackcog", "take-blackcog"],
            "use_on": {
                "item": "bucket_water",
                "target": {"ground": "blackcog", "id": cog_id,
                           "tile": BLACKCOG_TILE, "source": BLACKCOG_SOURCE},
                "dialogue": "continue"
            },
            "take_result": {"config": "blackcog", "id": cog_id, "count": held_cog},
            "bucket_water_after": item_count(snapshot, water_id),
            "native_status": native_status_receipt(status),
        })))
    });
    quester_live::run_family(cell, start, observe)
        .unwrap_or_else(|error| panic!("ground use-on family live proof failed: {error}"));
}

#[test]
#[ignore = "requires LIVE=1 and the already-running shared local Engine A"]
fn live_quester_families_main_documents() {
    let (selected, quests) = live_data();
    let ui = *selected
        .dialogue_ui()
        .expect("selected main-document controls");
    // Ordinary quest sequences reselect from observable skip predicates.
    // These informational reads do not change inventory or quest state, so
    // prove each family action in its own cell instead of inventing a cursor.
    for (label, alias, root, fragments) in [
        (
            "live-quester-families-main-scroll",
            "piratemessage",
            ui.scroll_root,
            &["Visit the city of the White Knights"][..],
        ),
        (
            "live-quester-families-main-book",
            "the_shield_of_arrav",
            ui.book_root,
            &["Arrav is probably", "year 143", "This tactic did not work"][..],
        ),
    ] {
        let id = item_id(&selected, alias);
        let (mut cell, seed, seed_commands) = make_cell(
            label,
            ("cook", "Cook's Assistant"),
            0,
            MAIN_DOCUMENT_ITEMS,
            SAFE_STAND,
            None,
            Arc::clone(&selected),
        );
        cell.observe_start = Some(Box::new(move |snapshot| {
            let seed_receipt = seed.observe_start(snapshot)?;
            let here = common_observe_start(snapshot, SAFE_STAND)?;
            require_item(snapshot, id, 1, alias)?;
            if snapshot.modals().main != -1 || snapshot.modals().chat != -1 {
                return Err(
                    "main-document fixture requires both modals closed before Start".into(),
                );
            }
            Ok(json!({
                "fixture": alias,
                "profile_seed": seed_receipt,
                "seed_commands": seed_commands,
                "tile": here,
                "inventory": inventory_receipt(snapshot),
            }))
        }));
        let document = family_path(
            "cook",
            "Selected main-document continuation fixture",
            "cook:0",
            vec![step(
                "read-document",
                "interact",
                json!({
                    "target": {"held": alias},
                    "op": "Read",
                    "dialogue": "continue"
                }),
                false,
                empty_any(),
                empty_all(),
            )],
            false,
        );
        let start = family_start(document, Arc::clone(&selected), Arc::clone(&quests));
        let mut pages_seen = vec![false; fragments.len()];
        let mut saw_root = false;
        let observe: ObserveFamily = Box::new(move |snapshot, status, _lifecycle| {
            let main = snapshot.modals().main;
            if main == root {
                saw_root = true;
                for widget in snapshot.widgets().iter().filter(|widget| {
                    widget.root == api::snapshot::WidgetRoot::Main
                        && widget.root_component_id == root
                        && !widget.hidden
                }) {
                    if let Some(text) = &widget.text {
                        for (seen, fragment) in pages_seen.iter_mut().zip(fragments) {
                            *seen |= text.contains(fragment);
                        }
                    }
                }
            }
            if !saw_root
                || !pages_seen.iter().all(|seen| *seen)
                || main != -1
                || snapshot.modals().chat != -1
                || item_count(snapshot, id) != 1
                || !status.is_some_and(|status| {
                    status.failure.is_none()
                        && status_text(status, "step_id") == Some("read-document")
                })
            {
                return Ok(None);
            }
            Ok(Some(json!({
                "family_step": "read-document",
                "target": {"held": alias, "id": id},
                "source": [
                    "quests/quest_hunt/scripts/pirate_message.rs2:23-27",
                    "quests/quest_blackarmgang/scripts/arrav_book.rs2:2-18",
                    "general/scripts/book.rs2:31-56"
                ],
                "main_root_observed": root,
                "page_fragments": fragments,
                "pages_observed": pages_seen,
                "main_modal_after": main,
                "chat_modal_after": snapshot.modals().chat,
                "native_status": native_status_receipt(status),
            })))
        });
        quester_live::run_family(cell, start, observe).unwrap_or_else(|error| {
            panic!("{alias} main-document family live proof failed: {error}")
        });
    }
}

/// Filliman appears at `0_53_52_48_8` when the grotto door is entered
/// (`quest_druidspirit.rs2:93-101`), one tile from the door placed at
/// `[3440,3337,0]`. Standing on his tile is safe: the door script opens his
/// dialogue whether or not `npc_add` accepts an occupied tile.
const NATURE_SPIRIT_STAND: WorldTile = WorldTile {
    x: 3440,
    z: 3336,
    level: 0,
};
const DRUIDSPIRIT_STAGE_REPLY: &str = "get druidspirit: 10";
const FILLIMAN_STEP: &str = "answer-filliman-once";

#[test]
#[ignore = "requires LIVE=1 and the already-running shared local Engine L"]
fn live_dialogue_answer_once_filliman_repeated_menu() {
    let (selected, quests) = live_data();
    let (quest_varp_id, quest_varp_alias) = quest_varp(&selected, "druidspirit");
    assert_eq!(quest_varp_alias, "druidspirit");
    let door_id = selected
        .loc_by_config("grotto_door_druidicspirit")
        .expect("selected R289 grotto door config")
        .id;
    let (mut cell, seed, seed_commands) = make_cell(
        "live-dialogue-answer-once-filliman",
        ("druidspirit", "Nature Spirit"),
        10,
        &[],
        NATURE_SPIRIT_STAND,
        Some(FixtureLoadout::Path("filliman_wear")),
        Arc::clone(&selected),
    );
    let query_baseline = insert_quest_stage_query(
        &mut cell,
        "getvar druidspirit",
        DRUIDSPIRIT_STAGE_REPLY,
        None,
    );
    // After the conversation closes, read the server-side stage again. The
    // quest varp is not transmitted to the client, so a `getvar` reply is
    // the only direct proof of `spoken_filliman`. This is a read-only
    // per-account diagnostic, not a seed.
    let saw_open = Arc::new(AtomicBool::new(false));
    let saw_chat = Arc::clone(&saw_open);
    let await_closed = Step {
        name: "wait for Filliman's chat to close",
        kind: StepKind::Await {
            evidence: "the Filliman conversation chat was open and then closed",
            ready: Box::new(move |snapshot| {
                let open = snapshot.modals().chat != -1;
                if open {
                    saw_chat.store(true, Ordering::Relaxed);
                }
                saw_chat.load(Ordering::Relaxed) && !open
            }),
            summary: None,
        },
        wait: Wait {
            arm: Proof::SideTabAvailable { index: 3 },
            budget_ticks: 200,
        },
    };
    let read_stage = Step {
        name: "read the stage after Filliman's conversation",
        kind: StepKind::Perform {
            send: Box::new(|client: &mut _, _: &GameSnapshot| {
                api::interact::cheat(client, "getvar druidspirit").is_sent()
            }),
        },
        wait: Wait {
            arm: Proof::FreshChat {
                needle: "get druidspirit: 20",
            },
            budget_ticks: 100,
        },
    };
    let completion_proof = cell
        .scenario
        .steps
        .len()
        .checked_sub(1)
        .expect("family fixture has a completion proof step");
    cell.scenario.steps.splice(
        completion_proof..completion_proof,
        [await_closed, read_stage],
    );
    let start_seed = seed;
    cell.observe_start = Some(Box::new(move |snapshot| {
        let seed_receipt = start_seed.observe_start(snapshot)?;
        let here = common_observe_start(snapshot, NATURE_SPIRIT_STAND)?;
        let stage_witness = quest_stage_witness(
            snapshot.chat_lines(),
            query_baseline.load(Ordering::Relaxed),
            DRUIDSPIRIT_STAGE_REPLY,
        )
        .ok_or("Nature Spirit fixture needs a fresh server reply: get druidspirit: 10")?;
        let door = snapshot
            .locs()
            .iter()
            .find(|loc| loc.id == door_id)
            .ok_or("pre-Start snapshot did not observe grotto_door_druidicspirit")?;
        if door.distance > 2
            || !door
                .actions
                .iter()
                .flatten()
                .any(|action| action.eq_ignore_ascii_case("Enter"))
        {
            return Err(format!(
                "grotto_door_druidicspirit was not a nearby Enter target: {door:?}"
            ));
        }
        Ok(json!({
            "fixture": "druidspirit:10",
            "profile_seed": seed_receipt,
            "seed_commands": seed_commands,
            "tile": here,
            "quest_varp": [quest_varp_id, 10],
            "quest_varp_alias": quest_varp_alias,
            "quest_varp_source": "post-relog getvar druidspirit",
            "quest_varp_witness": stage_witness,
            "loc": {"config": "grotto_door_druidicspirit", "id": door.id, "name": &door.name,
                    "tile": door.tile, "distance": door.distance, "actions": &door.actions},
        }))
    }));
    let mut document = family_path(
        "cook",
        "Nature Spirit answer-once family fixture",
        "cook:0",
        vec![step(
            FILLIMAN_STEP,
            "interact",
            json!({
                "target": {"loc": "grotto_door_druidicspirit"},
                "op": "Enter",
                "anchor": {
                    "tile": [3440, 3336, 0],
                    "source": "289 maps/m53_52.jm2:5135 places the door at [3440,3337,0]; quest_druidspirit.rs2:93-101 spawns Filliman at [3440,3336,0] and opens filliman.rs2:1-16"
                },
                "radius": 2,
                "wait_if_missing": true,
                "dialogue": {
                    "prefer": ["How long have you been a ghost?", "Ok, thanks."],
                    "answer_once": true,
                    "strict": true
                }
            }),
            true,
            empty_any(),
            empty_all(),
        )],
        true,
    );
    // The one step's completion has no account-local evidence to author as
    // skip_if; ordered order advances its cursor at settle, so the terminal
    // sequence finishes instead of replaying the door.
    document.roles[0].sequences[0].order = SequenceOrder::Ordered;
    let start = family_start(document, Arc::clone(&selected), Arc::clone(&quests));
    let mut menu_openings = 0;
    let mut first_menu_pages = None;
    let mut step_seen = false;
    let mut in_menu = false;
    let observe: ObserveFamily = Box::new(move |snapshot, status, lifecycle| {
        if status.is_some_and(|status| {
            status.phase == NativePhase::Working
                && status_text(status, "step_id") == Some(FILLIMAN_STEP)
        }) {
            step_seen = true;
        }
        // Count a menu opening once, not each poll of the same page. Text
        // frames may still arrive after the option rows; the rows identify
        // the menu. Freeze at the first close.
        let closed = snapshot.modals().chat == -1;
        if first_menu_pages.is_none() {
            let menu = snapshot
                .chat_options()
                .iter()
                .any(|option| option.text.eq_ignore_ascii_case("Ok, thanks."));
            if menu && !in_menu {
                menu_openings += 1;
            }
            in_menu = menu;
            if menu_openings == 2 && closed {
                first_menu_pages = Some(menu_openings);
            }
        }
        let Some(stage_witness) = snapshot.chat_lines().iter().find(|line| {
            line.type_ == 0 && line.username.is_none() && line.text == "get druidspirit: 20"
        }) else {
            return Ok(None);
        };
        if !step_seen || !completed_lifecycle(lifecycle) {
            return Ok(None);
        }
        let Some(menus) = first_menu_pages else {
            return Err(format!(
                "answer_once dialogue closed after {menu_openings} menu openings, not 2"
            ));
        };
        if snapshot.modals().chat != -1 {
            return Ok(None);
        }
        Ok(Some(json!({
            "family_step": "interact",
            "target_config": "grotto_door_druidicspirit",
            "operation": "Enter",
            "answer_once": true,
            "prefer": ["How long have you been a ghost?", "Ok, thanks."],
            "menu_pages": menus,
            "page_cap": script::quester::families::dialogue::DRIVE_STEPS,
            "chat_modal_after": snapshot.modals().chat,
            "quest_varp_id": quest_varp_id,
            "quest_varp_after": 20,
            "quest_varp_source": "post-dialogue getvar druidspirit",
            "quest_varp_witness": stage_witness,
            "quest_stage_after": "druidspirit:20",
            "quest_stage_source": "filliman.rs2:255-261 raises %druidspirit to spoken_filliman before reopening the menu; :233-234 sends Ok, thanks.",
            "native_status": native_status_receipt(status),
        })))
    });
    let receipt = quester_live::run_family(cell, start, observe)
        .unwrap_or_else(|error| panic!("answer_once Filliman family live proof failed: {error}"));
    assert_eq!(receipt["family_receipt"]["menu_pages"], 2);
    assert_eq!(receipt["family_receipt"]["quest_varp_after"], 20);
    assert_eq!(receipt["family_receipt"]["chat_modal_after"], -1);
}

const ENTRANA_ZOMBIE_STAND: WorldTile = WorldTile {
    x: 2842,
    z: 9765,
    level: 0,
};
const JAILER_STAND: WorldTile = WorldTile {
    x: 2932,
    z: 9693,
    level: 0,
};
const JAIL_KEY_TILE: WorldTile = WorldTile {
    x: 2933,
    z: 9693,
    level: 0,
};
const FLETCHING_STAND: WorldTile = WorldTile {
    x: 2612,
    z: 3251,
    level: 0,
};
const NORMAL_TREE_TILE: WorldTile = WorldTile {
    x: 2613,
    z: 3252,
    level: 0,
};
// zombie.rs2:102-110 gives 50/128 per kill. Missing 24 kills is (78/128)^24.
const ENTRANA_DROP_MAX_KILLS: u32 = 24;
const ENTRANA_DROP_STOP: &str = "bronze-axe-not-found-after-24-kills";
const JAIL_KEY_LIFETIME_TICKS: u32 = 100;
const ENTRANA_ZOMBIE_SOURCE: &str =
    "content/maps/m44_152.jm2:8477-8481; content/scripts/_unpack/225/all.npc:1877-1894; content/scripts/drop tables/scripts/zombie.rs2:102-110";
const JAIL_KEY_SOURCE: &str =
    "content/maps/m45_151.jm2:8528; content/scripts/drop tables/scripts/jailer.rs2:1-10";
const JAILER_COMBAT_SOURCE: &str =
    "content/scripts/areas/area_taverly/dungeon/configs/taverly_dungeon.npc:30-63";
const NORMAL_TREE_SOURCE: &str =
    "content/maps/m40_50.jm2:5891; content/pack/loc.pack:1277; content/scripts/skill_woodcutting/configs/trees.dbrow:1-16";
const SHORTBOW_MENU_SOURCE: &str = "content/scripts/skill_fletching/scripts/cut_logs.rs2:18-47";
const SHORTBOW_RECIPE_SOURCE: &str =
    "content/scripts/skill_fletching/configs/cut_logs/cut_logs.dbrow:1-7";

fn entrana_drop_outcome(
    completed_without_stop: bool,
    named_stop: bool,
    kills: i64,
    axes: i32,
) -> Result<&'static str, String> {
    let budget = i64::from(ENTRANA_DROP_MAX_KILLS);
    if completed_without_stop && !named_stop && (1..=budget).contains(&kills) && axes == 1 {
        Ok("drop_acquired")
    } else if !completed_without_stop && named_stop && kills == budget && axes == 0 {
        Ok("kill_budget_spent")
    } else {
        Err(format!(
            "invalid until_drop outcome: completed_without_stop={completed_without_stop}, \
             named_stop={named_stop}, confirmed_kills={kills}/{budget}, axes={axes}"
        ))
    }
}

#[test]
fn until_drop_oracle_accepts_both_bounded_outcomes_and_rejects_other_ends() {
    let budget = i64::from(ENTRANA_DROP_MAX_KILLS);
    assert_eq!(
        entrana_drop_outcome(true, false, 1, 1).unwrap(),
        "drop_acquired"
    );
    assert_eq!(
        entrana_drop_outcome(true, false, budget, 1).unwrap(),
        "drop_acquired"
    );
    assert_eq!(
        entrana_drop_outcome(false, true, budget, 0).unwrap(),
        "kill_budget_spent"
    );
    for (completed, stopped, kills, axes) in [
        (true, true, budget, 1),
        (true, false, 0, 1),
        (true, false, budget + 1, 1),
        (true, false, 1, 0),
        (false, true, budget - 1, 0),
        (false, true, budget, 1),
        (false, false, budget, 0),
    ] {
        assert!(entrana_drop_outcome(completed, stopped, kills, axes).is_err());
    }
}

#[test]
#[ignore = "requires LIVE=1 and the already-running local Engine Q"]
fn live_quester_families_until_drop_on_entrana() {
    let (selected, quests) = live_data();
    let zombie_id = selected
        .npc_by_config("zombie_entranan")
        .expect("selected R289 Entrana zombie config")
        .id;
    let zombie_type = usize::try_from(zombie_id).expect("selected zombie type id is nonnegative");
    let axe_id = item_id(&selected, "bronze_axe");
    let (mut cell, seed, seed_commands) = make_cell(
        "live-quester-families-until-drop-entrana",
        ("cook", "Cook's Assistant"),
        0,
        &[],
        ENTRANA_ZOMBIE_STAND,
        Some(FixtureLoadout::Path("family_melee")),
        Arc::clone(&selected),
    );
    cell.scenario.settings.deadline = Duration::from_secs(600);
    cell.mode = Mode::ExpectedNamedStop {
        reason: ENTRANA_DROP_STOP.into(),
    };
    let start_seed = seed;
    let start_selected = Arc::clone(&selected);
    cell.observe_start = Some(Box::new(move |snapshot| {
        let seed_receipt = start_seed.observe_start(snapshot)?;
        let here = common_observe_start(snapshot, ENTRANA_ZOMBIE_STAND)?;
        require_item(snapshot, axe_id, 0, "bronze_axe")?;
        if seed_commands
            .iter()
            .any(|command| command.starts_with("give bronze_axe "))
        {
            return Err("chance-only fixture seeded the target bronze axe".into());
        }
        if snapshot
            .ground_items()
            .iter()
            .any(|item| item.def.id == axe_id && item.distance <= 12)
        {
            return Err("a bronze axe was already on the ground at the fight site".into());
        }
        let visible_zombie = snapshot
            .npcs()
            .iter()
            .find(|npc| npc.r#type == Some(zombie_type))
            .ok_or("no zombie_entranan spawn was visible before Start")?;
        let local = snapshot
            .local_player()
            .ok_or("pre-Start Entrana snapshot has no local player")?;
        Ok(json!({
            "fixture": "cook:0 Base40",
            "profile_seed": seed_receipt,
            "seed_commands": &seed_commands,
            "tile": here,
            "npc_config": "zombie_entranan",
            "npc_type_id": zombie_id,
            "visible_spawn_index": visible_zombie.index,
            "drop": "bronze_axe",
            "drop_chance": [50, 128],
            "drop_source": ENTRANA_ZOMBIE_SOURCE,
            "local_in_combat_before_start": local.player.actor.in_combat,
            "inventory_before_start": inventory_receipt(snapshot),
            "selected_revision": start_selected.revision().to_string(),
        }))
    }));

    let mut document = family_path(
        "cook",
        "Entrana zombie chance-only drop family fixture",
        "cook:0",
        vec![step(
            "until-drop-bronze-axe",
            "combat",
            json!({
                "target": {
                    "npc": "zombie_entranan",
                    "not_targeting_others": true,
                    "pick": "nearest"
                },
                "stand": {
                    "tile": [ENTRANA_ZOMBIE_STAND.x, ENTRANA_ZOMBIE_STAND.z, 0],
                    "source": ENTRANA_ZOMBIE_SOURCE
                },
                "lost_radius": 12,
                "kill_budget_ticks": 600,
                "tactic": {
                    "kind": "open",
                    "style": "melee",
                    "engage_radius": 10,
                    "auto_retaliate": true
                },
                "until_drop": {
                    "obj": "bronze_axe",
                    "max_kills": ENTRANA_DROP_MAX_KILLS,
                    "stop": ENTRANA_DROP_STOP
                }
            }),
            false,
            empty_any(),
            fact("has_item", json!({"obj": "bronze_axe"})),
        )],
        true,
    );
    // This proof consumes its steps once while the fixture journal stays cook:0.
    document.roles[0].sequences[0].order = SequenceOrder::Ordered;
    document.roles[0].sequences[0].steps[0].comment = Some(format!(
        "{ENTRANA_ZOMBIE_SOURCE}. The step must loot the chance-only drop itself."
    ));
    let start = family_start(document, Arc::clone(&selected), Arc::clone(&quests));
    let mut combat_step_seen = false;
    let mut confirmed_kills = None;
    let observe: ObserveFamily = Box::new(move |snapshot, status, lifecycle| {
        if let Some(status) = status {
            combat_step_seen |= status.phase == NativePhase::Working
                && status_text(status, "step_id") == Some("until-drop-bronze-axe");
            if let Some(kills) = status_integer(Some(status), "combat_kills") {
                confirmed_kills = Some(confirmed_kills.unwrap_or(0).max(kills));
            }
        }
        let Some(lifecycle) = lifecycle else {
            return Ok(None);
        };
        let kills = confirmed_kills.ok_or("until_drop exposed no combat_kills status")?;
        if !combat_step_seen {
            return Err("until_drop terminal receipt never observed its combat step".into());
        }
        let completed_without_stop = completed_lifecycle(Some(lifecycle))
            && status.is_some_and(|status| {
                status.phase != NativePhase::Blocked
                    && status.failure.is_none()
                    && status_text(status, "block_reason").is_none()
            });
        let named_stop = quester_live::has_named_stop(status, ENTRANA_DROP_STOP)
            && lifecycle.state == script::ScriptTerminalState::Failed
            && lifecycle.reason == ENTRANA_DROP_STOP;
        let axes = item_count(snapshot, axe_id);
        let outcome = entrana_drop_outcome(completed_without_stop, named_stop, kills, axes)?;
        Ok(Some(json!({
            "family_step": "combat",
            "step_id": "until-drop-bronze-axe",
            "npc_config": "zombie_entranan",
            "npc_type_id": zombie_id,
            "drop": "bronze_axe",
            "drop_chance": [50, 128],
            "max_kills": ENTRANA_DROP_MAX_KILLS,
            "configured_stop": ENTRANA_DROP_STOP,
            "observed_stop": named_stop.then_some(ENTRANA_DROP_STOP),
            "outcome": outcome,
            "axes_held": axes,
            "combat_step_seen": combat_step_seen,
            "confirmed_kills": kills,
            "target_item_seeded": false,
            "inventory_after": inventory_receipt(snapshot),
            "source": ENTRANA_ZOMBIE_SOURCE,
            "native_status": native_status_receipt(status),
        })))
    });
    let receipt = quester_live::run_family(cell, start, observe)
        .unwrap_or_else(|error| panic!("Entrana until_drop live proof failed: {error}"));
    assert!(
        receipt["family_receipt"]["confirmed_kills"]
            .as_i64()
            .unwrap()
            > 0
    );
    assert_eq!(receipt["family_receipt"]["target_item_seeded"], false);
}

fn timed_jailer_cell(
    selected: Arc<SelectedGameData>,
) -> (Cell, scenario::quester::QuesterSeed, Vec<String>) {
    // These levels prepare only the fixture account against the real Jailer.
    // They are not Scorpion Catcher quest requirements.
    let requirements = ["attack", "defence", "strength", "hitpoints"]
        .into_iter()
        .map(|skill| QuestRequirementDocument {
            id: FactKey::new(skill),
            kind: QuestRequirementKindDocument::Skill {
                skill: skill.to_owned(),
                level: 99,
            },
            at: "Start".to_owned(),
            source: JAILER_COMBAT_SOURCE.to_owned(),
        })
        .collect();
    make_cell_with_requirements(
        "live-quester-families-timed-jail-key-take",
        ("cook", "Cook's Assistant", 0),
        &[("lobster", 20)],
        JAILER_STAND,
        Some(FixtureLoadout::Path("family_melee")),
        selected,
        requirements,
    )
}

#[test]
fn timed_jailer_fixture_seeds_owned_food_and_combat_levels() {
    let (selected, _) = live_data();
    let (_, _, commands) = timed_jailer_cell(selected);
    assert!(commands.iter().any(|command| command == "give lobster 20"));
    for skill in ["attack", "defence", "strength", "hitpoints"] {
        assert!(
            commands
                .iter()
                .any(|command| command == &format!("setstat {skill} 99")),
            "the timed-pickup setup must prepare {skill} before the real Jailer fight"
        );
    }
    assert!(!commands
        .iter()
        .any(|command| command.starts_with("give jail_key ")));
}

#[test]
#[ignore = "requires LIVE=1 and the already-running local Engine Q"]
fn live_quester_families_timed_jail_key_take() {
    let (selected, quests) = live_data();
    let jailer_id = selected
        .npc_by_config("jailer")
        .expect("selected R289 Jailer config")
        .id;
    let jailer_type = usize::try_from(jailer_id).expect("selected Jailer type id is nonnegative");
    let key_id = item_id(&selected, "jail_key");
    let food_id = item_id(&selected, "lobster");
    let (mut cell, seed, seed_commands) = timed_jailer_cell(Arc::clone(&selected));
    let start_seed = seed;
    let start_selected = Arc::clone(&selected);
    cell.observe_start = Some(Box::new(move |snapshot| {
        let seed_receipt = start_seed.observe_start(snapshot)?;
        let here = common_observe_start(snapshot, JAILER_STAND)?;
        require_item(snapshot, key_id, 0, "jail_key")?;
        require_item(snapshot, food_id, 20, "lobster")?;
        for skill in ["attack", "defence", "strength", "hitpoints"] {
            if stat_base(snapshot, skill) != Some(99) {
                return Err(format!(
                    "timed-ground fixture did not prepare {skill} 99: {:?}",
                    stat_base(snapshot, skill)
                ));
            }
        }
        if seed_commands
            .iter()
            .any(|command| command.starts_with("give jail_key "))
        {
            return Err("timed-ground fixture seeded the Jailer key".into());
        }
        if snapshot
            .ground_items()
            .iter()
            .any(|item| item.def.id == key_id && item.tile == JAIL_KEY_TILE)
        {
            return Err("a Jailer key was already on its content drop tile".into());
        }
        let visible_jailer = snapshot
            .npcs()
            .iter()
            .find(|npc| npc.r#type == Some(jailer_type))
            .ok_or("the content Jailer was not visible before Start")?;
        Ok(json!({
            "fixture": "cook:0 Base40 with owned Jailer combat preparation",
            "profile_seed": seed_receipt,
            "seed_commands": &seed_commands,
            "tile": here,
            "npc_config": "jailer",
            "npc_type_id": jailer_id,
            "visible_spawn_index": visible_jailer.index,
            "key": "jail_key",
            "key_lifetime_ticks": JAIL_KEY_LIFETIME_TICKS,
            "drop_source": JAIL_KEY_SOURCE,
            "combat_setup_source": JAILER_COMBAT_SOURCE,
            "combat_setup_stats": [
                ["attack", stat_base(snapshot, "attack")],
                ["defence", stat_base(snapshot, "defence")],
                ["strength", stat_base(snapshot, "strength")],
                ["hitpoints", stat_base(snapshot, "hitpoints")],
            ],
            "food_setup": {"obj": "lobster", "qty": 20},
            "inventory_before_start": inventory_receipt(snapshot),
            "selected_revision": start_selected.revision().to_string(),
        }))
    }));

    let mut document = family_path(
        "cook",
        "Jailer timed ground-item family fixture",
        "cook:0",
        vec![
            step(
                "kill-jailer-for-key",
                "combat",
                json!({
                    "target": {
                        "npc": "jailer",
                        "not_targeting_others": true,
                        "pick": "nearest"
                    },
                    "stand": {
                        "tile": [JAILER_STAND.x, JAILER_STAND.z, 0],
                        "source": JAIL_KEY_SOURCE
                    },
                    "lost_radius": 12,
                    "kill_budget_ticks": 600,
                    "tactic": {
                        "kind": "open",
                        "style": "melee",
                        "engage_radius": 6,
                        "auto_retaliate": true
                    }
                }),
                false,
                empty_any(),
                empty_all(),
            ),
            step(
                "take-timed-jail-key",
                "interact",
                json!({
                    "target": {"ground": "jail_key"},
                    "op": "Take",
                    "anchor": {
                        "tile": [JAIL_KEY_TILE.x, JAIL_KEY_TILE.z, 0],
                        "source": JAIL_KEY_SOURCE
                    },
                    "radius": 2,
                    "max_ticks": JAIL_KEY_LIFETIME_TICKS
                }),
                false,
                empty_any(),
                fact("has_item", json!({"obj": "jail_key"})),
            ),
        ],
        true,
    );
    document.roles[0].sequences[0].order = SequenceOrder::Ordered;
    document.roles[0].sequences[0].steps[0].comment = Some(format!(
        "{JAIL_KEY_SOURCE}. The combat step deliberately does not loot."
    ));
    document.roles[0].sequences[0].steps[1].comment = Some(format!(
        "{JAIL_KEY_SOURCE}. Take the content drop within its observed 100-tick lifetime."
    ));
    let start = family_start(document, Arc::clone(&selected), Arc::clone(&quests));
    let mut combat_step_seen = false;
    let mut take_step_seen = false;
    let mut ground_item_seen_during_take = false;
    let mut confirmed_kills = None;
    let mut ground_waited_ticks = None;
    let mut ground_held = None;
    let observe: ObserveFamily = Box::new(move |snapshot, status, lifecycle| {
        if let Some(status) = status {
            let step_id = status_text(status, "step_id");
            combat_step_seen |= step_id == Some("kill-jailer-for-key");
            take_step_seen |= step_id == Some("take-timed-jail-key");
            ground_item_seen_during_take |= step_id == Some("take-timed-jail-key")
                && snapshot
                    .ground_items()
                    .iter()
                    .any(|item| item.def.id == key_id && item.tile == JAIL_KEY_TILE);
            if let Some(kills) = status_integer(Some(status), "combat_kills") {
                confirmed_kills = Some(confirmed_kills.unwrap_or(0).max(kills));
            }
            if let Some(ticks) = status_integer(Some(status), "ground_waited_ticks") {
                ground_waited_ticks = Some(ticks);
            }
            if let Some(held) = status_integer(Some(status), "ground_held") {
                ground_held = Some(held);
            }
        }
        if lifecycle.is_none() {
            return Ok(None);
        }
        if !completed_lifecycle(lifecycle) {
            return Err(format!(
                "timed Jailer key take did not complete: kills={confirmed_kills:?}, \
                 ground_waited_ticks={ground_waited_ticks:?}, ground_held={ground_held:?}, \
                 status={:?}",
                native_status_receipt(status)
            ));
        }
        if !combat_step_seen || !take_step_seen {
            return Err(format!(
                "timed-ground sequence skipped an authored step: combat_seen={combat_step_seen}, \
                 take_seen={take_step_seen}"
            ));
        }
        let kills = confirmed_kills.ok_or("Jailer fight exposed no combat_kills status")?;
        if kills <= 0 {
            return Err(format!("Jailer take had no confirmed kill: {kills}"));
        }
        let waited = ground_waited_ticks
            .ok_or("successful timed take exposed no ground_waited_ticks status")?;
        if !(0..=i64::from(JAIL_KEY_LIFETIME_TICKS)).contains(&waited) {
            return Err(format!(
                "timed take waited {waited} ticks beyond the content 100-tick lifetime"
            ));
        }
        let held = ground_held.ok_or("successful timed take exposed no ground_held status")?;
        if held < 1 {
            return Err(format!("timed take reported only {held} held keys"));
        }
        require_item(snapshot, key_id, 1, "jail_key")?;
        Ok(Some(json!({
            "family_steps": ["combat", "interact"],
            "jailer_config": "jailer",
            "jailer_type_id": jailer_id,
            "confirmed_kills": kills,
            "ground_target": "jail_key",
            "operation": "Take",
            "max_ticks": JAIL_KEY_LIFETIME_TICKS,
            "key_lifetime_ticks": JAIL_KEY_LIFETIME_TICKS,
            "ground_waited_ticks": waited,
            "ground_held": held,
            "ground_item_seen_during_take": ground_item_seen_during_take,
            "interact_args_excluded": ["until", "dialogue", "settle_ms", "wait_until"],
            "jail_key_count_after": item_count(snapshot, key_id),
            "drop_source": JAIL_KEY_SOURCE,
            "native_status": native_status_receipt(status),
        })))
    });
    let receipt = quester_live::run_family(cell, start, observe)
        .unwrap_or_else(|error| panic!("timed Jailer key live proof failed: {error}"));
    assert!(
        receipt["family_receipt"]["ground_waited_ticks"]
            .as_i64()
            .unwrap()
            <= i64::from(JAIL_KEY_LIFETIME_TICKS)
    );
    assert!(receipt["family_receipt"]["ground_held"].as_i64().unwrap() >= 1);
}

#[test]
#[ignore = "requires LIVE=1 and the already-running local Engine Q"]
fn live_quester_families_make_from_open_fletching_menu() {
    let (selected, quests) = live_data();
    let axe_id = item_id(&selected, "bronze_axe");
    let knife_id = item_id(&selected, "knife");
    let logs_id = item_id(&selected, "logs");
    let product_id = item_id(&selected, "unstrung_shortbow");
    let (mut cell, seed, seed_commands) = make_cell_with_requirements(
        "live-quester-families-open-fletching-menu",
        ("cook", "Cook's Assistant", 0),
        &[("bronze_axe", 1), ("knife", 1)],
        FLETCHING_STAND,
        None,
        Arc::clone(&selected),
        vec![shortbow_fletching_requirement()],
    );
    let start_seed = seed;
    let start_selected = Arc::clone(&selected);
    cell.observe_start = Some(Box::new(move |snapshot| {
        let seed_receipt = start_seed.observe_start(snapshot)?;
        let here = common_observe_start(snapshot, FLETCHING_STAND)?;
        require_item(snapshot, axe_id, 1, "bronze_axe")?;
        require_item(snapshot, knife_id, 1, "knife")?;
        require_item(snapshot, logs_id, 0, "logs")?;
        require_item(snapshot, product_id, 0, "unstrung_shortbow")?;
        if seed_commands.iter().any(|command| {
            command.starts_with("give logs ") || command.starts_with("give unstrung_shortbow ")
        }) {
            return Err("fletching fixture seeded a content-acquired material or product".into());
        }
        if snapshot.modals().main != -1 {
            return Err(format!(
                "fletching fixture began with a modal already open: {}",
                snapshot.modals().main
            ));
        }
        Ok(json!({
            "fixture": "cook:0 Base40",
            "profile_seed": seed_receipt,
            "seed_commands": &seed_commands,
            "tile": here,
            "tools_seeded": ["bronze_axe", "knife"],
            "logs_seeded": false,
            "product_seeded": false,
            "main_modal_before_start": snapshot.modals().main,
            "stats": [
                ["woodcutting", stat_base(snapshot, "woodcutting")],
                ["fletching", stat_base(snapshot, "fletching")],
            ],
            "selected_revision": start_selected.revision().to_string(),
        }))
    }));

    let mut document = family_path(
        "cook",
        "Fletching open-menu Make family fixture",
        "cook:0",
        vec![
            step(
                "gather-normal-log",
                "gather",
                json!({
                    "skill": "woodcutting",
                    "resource": "normal",
                    "until": {"obj": "logs", "qty": 1},
                    "anchor": {
                        "tile": [NORMAL_TREE_TILE.x, NORMAL_TREE_TILE.z, 0],
                        "source": NORMAL_TREE_SOURCE
                    },
                    "radius": 2
                }),
                false,
                empty_any(),
                fact("has_item", json!({"obj": "logs"})),
            ),
            step(
                "open-fletching-menu",
                "use_on",
                json!({
                    "item": "knife",
                    "target": {"item": "logs"}
                }),
                false,
                empty_any(),
                // skill_multi3 opens chat products, not the modal_open main panel.
                empty_all(),
            ),
            step(
                "make-unstrung-shortbow",
                "make",
                json!({
                    "product": "unstrung_shortbow",
                    "menu": {
                        "obj": "shortbow",
                        "source": SHORTBOW_MENU_SOURCE
                    },
                    "qty": 1,
                    "make_x": false
                }),
                false,
                empty_any(),
                fact("has_item", json!({"obj": "unstrung_shortbow"})),
            ),
        ],
        true,
    );
    document.roles[0].sequences[0].order = SequenceOrder::Ordered;
    document
        .quest
        .as_mut()
        .expect("Cook quest header")
        .requirements = vec![shortbow_fletching_requirement()];
    document.roles[0].sequences[0].steps[0].comment = Some(format!(
        "{NORMAL_TREE_SOURCE}; content/scripts/skill_woodcutting/scripts/woodcut.rs2:1-14,91-148 yields logs."
    ));
    document.roles[0].sequences[0].steps[1].comment = Some(format!(
        "{SHORTBOW_MENU_SOURCE} opens the fletching production menu after knife -> logs."
    ));
    document.roles[0].sequences[0].steps[2].comment = Some(format!(
        "{SHORTBOW_MENU_SOURCE}; {SHORTBOW_RECIPE_SOURCE} selects the shortbow menu row for one unstrung_shortbow."
    ));
    let start = family_start(document, Arc::clone(&selected), Arc::clone(&quests));
    let mut gather_step_seen = false;
    let mut use_on_step_seen = false;
    let mut menu_open_after_use_on = false;
    let mut make_step_seen = false;
    let observe: ObserveFamily = Box::new(move |snapshot, status, lifecycle| {
        if let Some(status) = status {
            let step_id = status_text(status, "step_id");
            gather_step_seen |= step_id == Some("gather-normal-log");
            use_on_step_seen |= step_id == Some("open-fletching-menu");
            menu_open_after_use_on |= use_on_step_seen && !snapshot.make_products().is_empty();
            make_step_seen |= step_id == Some("make-unstrung-shortbow");
        }
        if lifecycle.is_none() {
            return Ok(None);
        }
        if !completed_lifecycle(lifecycle) {
            return Err(format!(
                "gather/use_on/menu-only Make path did not complete: \
                 gather={gather_step_seen}, use_on={use_on_step_seen}, \
                 modal_open={menu_open_after_use_on}, make={make_step_seen}; status={:?}",
                native_status_receipt(status)
            ));
        }
        if !gather_step_seen || !use_on_step_seen || !menu_open_after_use_on || !make_step_seen {
            return Err(format!(
                "open-menu production evidence is incomplete: gather={gather_step_seen}, \
                 use_on={use_on_step_seen}, modal_open={menu_open_after_use_on}, \
                 make={make_step_seen}"
            ));
        }
        require_item(snapshot, logs_id, 0, "logs")?;
        require_item(snapshot, product_id, 1, "unstrung_shortbow")?;
        Ok(Some(json!({
            "family_steps": ["gather", "use_on", "make"],
            "resource": "normal",
            "tree_tile": NORMAL_TREE_TILE,
            "tree_source": NORMAL_TREE_SOURCE,
            "gather_step_seen": gather_step_seen,
            "use_on_step_seen": use_on_step_seen,
            "modal_open_after_use_on": menu_open_after_use_on,
            "make_step_seen": make_step_seen,
            "make": {
                "product": "unstrung_shortbow",
                "menu": {"obj": "shortbow", "source": SHORTBOW_MENU_SOURCE},
                "qty": 1,
                "make_x": false,
                "loc": null,
                "anchor": null
            },
            "recipe_source": SHORTBOW_RECIPE_SOURCE,
            "logs_after": item_count(snapshot, logs_id),
            "shortbow_count_after": item_count(snapshot, product_id),
            "inventory_after": inventory_receipt(snapshot),
            "native_status": native_status_receipt(status),
        })))
    });
    let receipt = quester_live::run_family(cell, start, observe)
        .unwrap_or_else(|error| panic!("open-menu Make live proof failed: {error}"));
    assert_eq!(receipt["family_receipt"]["logs_after"], 0);
    assert_eq!(receipt["family_receipt"]["shortbow_count_after"], 1);
    assert_eq!(receipt["family_receipt"]["make"]["loc"], Value::Null);
    assert_eq!(receipt["family_receipt"]["make"]["anchor"], Value::Null);
}
#[test]
#[ignore = "requires LIVE=1 and the already-running local Engine Q"]
fn live_quester_families_bronze_bar_on_anvil_makes_wire() {
    let (selected, quests) = live_data();
    let bronze_pickaxe_id = item_id(&selected, "bronze_pickaxe");
    let hammer_id = item_id(&selected, "hammer");
    let copper_ore_id = item_id(&selected, "copper_ore");
    let tin_ore_id = item_id(&selected, "tin_ore");
    let bronze_bar_id = item_id(&selected, "bronze_bar");
    let bronze_wire_id = item_id(&selected, "bronzecraftwire");
    let (mut cell, seed, seed_commands) = make_cell_with_requirements(
        "live-quester-families-bronze-wire-on-anvil",
        ("cook", "Cook's Assistant", 0),
        &[("bronze_pickaxe", 1), ("hammer", 1)],
        BRONZE_COPPER_TILE,
        None,
        Arc::clone(&selected),
        vec![bronze_wire_smithing_requirement()],
    );
    cell.scenario.settings.deadline = Duration::from_secs(600);
    let start_seed = seed;
    let start_selected = Arc::clone(&selected);
    cell.observe_start = Some(Box::new(move |snapshot| {
        let seed_receipt = start_seed.observe_start(snapshot)?;
        let here = common_observe_start(snapshot, BRONZE_COPPER_TILE)?;
        require_item(snapshot, bronze_pickaxe_id, 1, "bronze_pickaxe")?;
        require_item(snapshot, hammer_id, 1, "hammer")?;
        require_item(snapshot, copper_ore_id, 0, "copper_ore")?;
        require_item(snapshot, tin_ore_id, 0, "tin_ore")?;
        require_item(snapshot, bronze_bar_id, 0, "bronze_bar")?;
        require_item(snapshot, bronze_wire_id, 0, "bronzecraftwire")?;
        if seed_commands.iter().any(|command| {
            command.starts_with("give copper_ore ")
                || command.starts_with("give tin_ore ")
                || command.starts_with("give bronze_bar ")
                || command.starts_with("give bronzecraftwire ")
        }) {
            return Err("smithing fixture seeded an ore, bar, or wire".into());
        }
        if snapshot.modals().main != -1 {
            return Err(format!(
                "bronze-wire fixture began with a modal already open: {}",
                snapshot.modals().main
            ));
        }
        if stat_base(snapshot, "smithing") != Some(4) {
            return Err(format!(
                "bronze-wire fixture did not seed minimum Smithing 4: {:?}",
                stat_base(snapshot, "smithing")
            ));
        }
        Ok(json!({
            "fixture": "cook:0 Base40",
            "profile_seed": seed_receipt,
            "seed_commands": &seed_commands,
            "tile": here,
            "tools_seeded": ["bronze_pickaxe", "hammer"],
            "smithing_seeded": 4,
            "copper_ore_seeded": false,
            "tin_ore_seeded": false,
            "bronze_bar_seeded": false,
            "bronzecraftwire_seeded": false,
            "main_modal_before_start": snapshot.modals().main,
            "selected_revision": start_selected.revision().to_string(),
        }))
    }));

    let mut document = family_path(
        "cook",
        "Bronze bar on anvil MainMake family fixture",
        "cook:0",
        vec![
            step(
                "gather-copper-ore",
                "gather",
                json!({
                    "skill": "mining",
                    "resource": "copper",
                    "until": {"obj": "copper_ore", "qty": 1},
                    "anchor": {
                        "tile": [
                            BRONZE_COPPER_TILE.x,
                            BRONZE_COPPER_TILE.z,
                            BRONZE_COPPER_TILE.level
                        ],
                        "source": BRONZE_COPPER_SOURCE
                    },
                    "radius": 3
                }),
                false,
                empty_any(),
                fact("has_item", json!({"obj": "copper_ore"})),
            ),
            step(
                "gather-tin-ore",
                "gather",
                json!({
                    "skill": "mining",
                    "resource": "tin",
                    "until": {"obj": "tin_ore", "qty": 1},
                    "anchor": {
                        "tile": [
                            BRONZE_TIN_TILE.x,
                            BRONZE_TIN_TILE.z,
                            BRONZE_TIN_TILE.level
                        ],
                        "source": BRONZE_TIN_SOURCE
                    },
                    "radius": 3
                }),
                false,
                empty_any(),
                fact("has_item", json!({"obj": "tin_ore"})),
            ),
            step(
                "smelt-bronze-bar",
                "use_on",
                json!({
                    "item": "copper_ore",
                    "target": {"loc": "furnace1"},
                    "anchor": {
                        "tile": [
                            BRONZE_FURNACE_TILE.x,
                            BRONZE_FURNACE_TILE.z,
                            BRONZE_FURNACE_TILE.level
                        ],
                        "source": BRONZE_FURNACE_SOURCE
                    },
                    "radius": 3,
                    "product": "bronze_bar"
                }),
                false,
                empty_any(),
                fact("has_item", json!({"obj": "bronze_bar"})),
            ),
            step(
                "use-bronze-bar-on-anvil",
                "use_on",
                json!({
                    "item": "bronze_bar",
                    "target": {"loc": "anvil"},
                    "anchor": {
                        "tile": [
                            BRONZE_ANVIL_TILE.x,
                            BRONZE_ANVIL_TILE.z,
                            BRONZE_ANVIL_TILE.level
                        ],
                        "source": BRONZE_ANVIL_SOURCE
                    },
                    "radius": 3
                }),
                false,
                empty_any(),
                fact("modal_open", json!({})),
            ),
            step(
                "make-bronze-wire",
                "make",
                json!({
                    "product": "bronzecraftwire",
                    "menu": {
                        "obj": "bronzecraftwire",
                        "source": BRONZE_WIRE_MENU_SOURCE
                    },
                    "qty": 1,
                    "make_x": false
                }),
                false,
                empty_any(),
                fact("has_item", json!({"obj": "bronzecraftwire"})),
            ),
        ],
        true,
    );
    document.roles[0].sequences[0].order = SequenceOrder::Ordered;
    document
        .quest
        .as_mut()
        .expect("Cook quest header")
        .requirements = vec![bronze_wire_smithing_requirement()];
    let steps = &mut document.roles[0].sequences[0].steps;
    steps[0].comment = Some(format!(
        "{BRONZE_COPPER_SOURCE}; Mining requires level 1 and uses the content mining recipe."
    ));
    steps[1].comment = Some(format!(
        "{BRONZE_TIN_SOURCE}; Mining requires level 1 and supplies the second bronze ore."
    ));
    steps[2].comment = Some(format!(
        "{BRONZE_FURNACE_SOURCE}; smelting consumes one copper ore and one tin ore for one bronze bar."
    ));
    steps[3].comment = Some(format!(
        "{BRONZE_ANVIL_SOURCE}; {BRONZE_WIRE_MENU_SOURCE} opens the main Make inventory after the bar is used on the anvil."
    ));
    steps[4].comment = Some(format!(
        "{BRONZE_WIRE_MENU_SOURCE}; {BRONZE_WIRE_RECIPE_SOURCE}; one named Make quantity consumes one bronze bar and returns one bronze wire after the content delay."
    ));
    let start = family_start(document, Arc::clone(&selected), Arc::clone(&quests));
    let mut copper_gather_seen = false;
    let mut tin_gather_seen = false;
    let mut smelt_step_seen = false;
    let mut anvil_step_seen = false;
    let mut anvil_menu_open_after_use_on = false;
    let mut main_make_wire_row_seen = false;
    let mut make_step_seen = false;
    let observe: ObserveFamily = Box::new(move |snapshot, status, lifecycle| {
        let step_id = status.and_then(|status| status_text(status, "step_id"));
        copper_gather_seen |= step_id == Some("gather-copper-ore");
        tin_gather_seen |= step_id == Some("gather-tin-ore");
        smelt_step_seen |= step_id == Some("smelt-bronze-bar");
        anvil_step_seen |= step_id == Some("use-bronze-bar-on-anvil");
        make_step_seen |= step_id == Some("make-bronze-wire");
        let wire_menu_row_visible = snapshot
            .main_make()
            .iter()
            .any(|item| item.def.id == bronze_wire_id);
        main_make_wire_row_seen |= wire_menu_row_visible;
        anvil_menu_open_after_use_on |= step_id == Some("use-bronze-bar-on-anvil")
            && snapshot.modals().main != -1
            && wire_menu_row_visible;
        if lifecycle.is_none() {
            return Ok(None);
        }
        if !completed_lifecycle(lifecycle) {
            return Err(format!(
                "content-mined bronze-wire MainMake path did not complete: \
                 copper={copper_gather_seen}, tin={tin_gather_seen}, \
                 smelt={smelt_step_seen}, anvil={anvil_step_seen}, \
                 main_make_row={main_make_wire_row_seen}, \
                 make={make_step_seen}; status={:?}",
                native_status_receipt(status)
            ));
        }
        if !copper_gather_seen
            || !tin_gather_seen
            || !smelt_step_seen
            || !anvil_step_seen
            || !anvil_menu_open_after_use_on
            || !main_make_wire_row_seen
            || !make_step_seen
        {
            return Err(format!(
                "bronze-wire MainMake evidence is incomplete: copper={copper_gather_seen}, \
                 tin={tin_gather_seen}, smelt={smelt_step_seen}, anvil={anvil_step_seen}, \
                 anvil_menu={anvil_menu_open_after_use_on}, \
                 main_make_row={main_make_wire_row_seen}, make={make_step_seen}"
            ));
        }
        require_item(snapshot, copper_ore_id, 0, "copper_ore")?;
        require_item(snapshot, tin_ore_id, 0, "tin_ore")?;
        require_item(snapshot, bronze_bar_id, 0, "bronze_bar")?;
        require_item(snapshot, bronze_wire_id, 1, "bronzecraftwire")?;
        Ok(Some(json!({
            "family_steps": ["gather", "gather", "use_on", "use_on", "make"],
            "copper_tile": BRONZE_COPPER_TILE,
            "copper_source": BRONZE_COPPER_SOURCE,
            "tin_tile": BRONZE_TIN_TILE,
            "tin_source": BRONZE_TIN_SOURCE,
            "furnace_tile": BRONZE_FURNACE_TILE,
            "furnace_source": BRONZE_FURNACE_SOURCE,
            "anvil_tile": BRONZE_ANVIL_TILE,
            "anvil_source": BRONZE_ANVIL_SOURCE,
            "copper_gather_step_seen": copper_gather_seen,
            "tin_gather_step_seen": tin_gather_seen,
            "smelt_step_seen": smelt_step_seen,
            "anvil_step_seen": anvil_step_seen,
            "anvil_menu_open_after_use_on": anvil_menu_open_after_use_on,
            "main_make_wire_row_seen": main_make_wire_row_seen,
            "make_step_seen": make_step_seen,
            "make": {
                "product": "bronzecraftwire",
                "menu": {
                    "obj": "bronzecraftwire",
                    "source": BRONZE_WIRE_MENU_SOURCE
                },
                "qty": 1,
                "make_x": false,
                "loc": null,
                "anchor": null
            },
            "recipe_source": BRONZE_WIRE_RECIPE_SOURCE,
            "copper_ore_after": item_count(snapshot, copper_ore_id),
            "tin_ore_after": item_count(snapshot, tin_ore_id),
            "bronze_bar_after": item_count(snapshot, bronze_bar_id),
            "bronzecraftwire_after": item_count(snapshot, bronze_wire_id),
            "inventory_after": inventory_receipt(snapshot),
            "native_status": native_status_receipt(status),
        })))
    });
    let receipt = quester_live::run_family(cell, start, observe)
        .unwrap_or_else(|error| panic!("bronze-wire MainMake live proof failed: {error}"));
    assert_eq!(receipt["family_receipt"]["copper_ore_after"], 0);
    assert_eq!(receipt["family_receipt"]["tin_ore_after"], 0);
    assert_eq!(receipt["family_receipt"]["bronze_bar_after"], 0);
    assert_eq!(receipt["family_receipt"]["bronzecraftwire_after"], 1);
    assert_eq!(receipt["family_receipt"]["make"]["qty"], 1);
    assert_eq!(receipt["family_receipt"]["make"]["loc"], Value::Null);
    assert_eq!(receipt["family_receipt"]["make"]["anchor"], Value::Null);
    assert_eq!(
        receipt["family_receipt"]["anvil_menu_open_after_use_on"],
        true
    );
    assert_eq!(receipt["family_receipt"]["main_make_wire_row_seen"], true);
}

const PREREQ_WEST: WorldTile = WorldTile {
    x: 3408,
    z: 3489,
    level: 0,
};
const PREREQ_EAST: WorldTile = WorldTile {
    x: 3423,
    z: 3485,
    level: 0,
};
const PREREQ_SWAMP: WorldTile = WorldTile {
    x: 3443,
    z: 3456,
    level: 0,
};
const HOLY_PASS: &str = "You pass through the holy barrier.";
const MORT_IN: &str = "You walk into the gloomy atmosphere of Mort Myre.";
const MORT_OUT: &str = "You skip gladly out of murky Mort Myre.";
const HOLY_ADVICE: &str = "Okay, I will keep it equipped whenever I fight werewolves.";
const DAGGER_RECLAIM: &str = "Luckily for you it washed up";
const DAGGER_RECLAIM_OPENER: &str = "I've lost my wolfbane dagger.";
const MORT_FOOD: &str = "The cleric hands you some food.";

struct PrereqLiveCase {
    selector: &'static str,
    priest: i32,
    druid: i32,
    bank_dagger: bool,
    pies: i32,
    stand: WorldTile,
    goal: WorldTile,
    holy: bool,
    mort: Option<bool>,
    advice: bool,
    reclaim: bool,
    start: bool,
}

fn insert_prereq_account_seed(cell: &mut Cell, command: String) {
    let relog = cell
        .scenario
        .steps
        .iter()
        .position(|step| step.name == "relog after permanent quest stage and loadout seed")
        .expect("account seed has its normal post-seed relog");
    cell.scenario.steps.insert(
        relog,
        Step {
            name: "seed prerequisite account state before relog",
            kind: StepKind::Perform {
                send: Box::new(move |client, _| api::interact::cheat(client, &command).is_sent()),
            },
            wait: Wait {
                arm: Proof::SideTabAvailable { index: 3 },
                budget_ticks: 100,
            },
        },
    );
}

fn run_quester_prereq_cell(case: PrereqLiveCase) {
    run_quester_prereq_case(case, None);
}

fn prerequisite_path(goal: WorldTile, recipe: bool) -> PathDocument {
    let mut document = family_path(
        "cook",
        "Native prerequisite-aware Quester Path",
        "cook:0",
        vec![step(
            "prerequisite-walk",
            if recipe { "acquire" } else { "walk" },
            if recipe {
                json!({"recipe": "open:morytania"})
            } else {
                json!({
                    "tile": [goal.x, goal.z, goal.level],
                    "source": "C maps/m53_54.jm2:5089,5096; scripts/quests/quest_druidspirit/scripts/quest_druidspirit.rs2:9-23; fixture destination beyond the generated crossing"
                })
            },
            false,
            empty_any(),
            empty_all(),
        )],
        true,
    );
    // This fixture owns one walk, not an authored rule that repeats until a
    // changing skip predicate excludes it after the fresh pass has settled.
    document.roles[0].sequences[0].order = script::quester::path::SequenceOrder::Ordered;
    if recipe {
        let mut released: PathDocument =
            serde_json::from_str(include_str!("../../script/paths/289/priestperil.json"))
                .expect("released Priest in Peril Path");
        let steps = released
            .quest
            .as_mut()
            .expect("Priest in Peril quest header")
            .acquire
            .remove("open:morytania")
            .expect("released Morytania recipe");
        document
            .quest
            .as_mut()
            .expect("live Path quest header")
            .acquire
            .insert("open:morytania".into(), steps);
    }
    document
}

#[test]
fn prerequisite_live_paths_compile_with_provenance_and_one_pass_order() {
    let (selected, quests) = live_data();
    for (goal, recipe) in [
        (PREREQ_EAST, true),
        (PREREQ_SWAMP, false),
        (
            WorldTile {
                x: 3443,
                z: 3458,
                level: 0,
            },
            false,
        ),
    ] {
        let document = prerequisite_path(goal, recipe);
        assert_eq!(
            document.roles[0].sequences[0].order,
            script::quester::path::SequenceOrder::Ordered
        );
        let bytes = serde_json::to_vec(&document).unwrap();
        let selected = Arc::clone(&selected);
        let quests = Arc::clone(&quests);
        let compiled = api::selected::FamilyPreparation::run(move |worker| {
            compile_path(
                &bytes,
                &selected,
                &quests,
                worker,
                &script::local_state::OperatorLoadouts::absent(),
            )
        })
        .unwrap()
        .join()
        .unwrap();
        assert!(
            compiled.is_ok(),
            "goal={goal:?} recipe={recipe}: {:?}",
            compiled.err()
        );
    }
}

struct PrereqFailureOracle {
    expected: &'static str,
    messages: Vec<String>,
    wolfbane: i32,
    meat_pie: i32,
    apple_pie: i32,
    pies: i32,
    stand: WorldTile,
    start_inventory: Arc<Mutex<Option<Vec<Value>>>>,
}

impl PrereqFailureOracle {
    fn new(selected: Arc<SelectedGameData>, expected: &'static str, case: &PrereqLiveCase) -> Self {
        use script::native::{PrereqMissing, PrereqRefusal, PrereqStepIdentity, PrereqSwitch};
        let wolfbane = item_id(&selected, "dagger_wolfbane");
        let meat_pie = item_id(&selected, "meat_pie");
        let apple_pie = item_id(&selected, "apple_pie");
        let catalog = api::selected::FamilyPreparation::run(move |worker| {
            selected.prepare_prereq_steps(worker)
        })
        .unwrap()
        .join()
        .unwrap()
        .unwrap();
        let id = match expected {
            "FreeSlots" => "priestperil:drezel-advice",
            "quest_start is not allowed" => "druidspirit:drezel-start",
            other => panic!("unsupported negative prerequisite oracle: {other}"),
        };
        let row = catalog
            .iter()
            .find_map(|(_, row)| (row.id.as_ref() == id).then_some(row))
            .expect("negative fixture has an authored prerequisite row");
        let messages = row
            .crossings
            .iter()
            .map(|crossing| {
                let step = PrereqStepIdentity {
                    id: Arc::clone(&row.id),
                    source: Arc::clone(&crossing.source.0),
                };
                match expected {
                    "FreeSlots" => PrereqRefusal::PreconditionMissing {
                        step,
                        missing: PrereqMissing::FreeSlots { required: 1 },
                    },
                    _ => PrereqRefusal::NotAllowed {
                        step,
                        switch: PrereqSwitch::QuestStart,
                    },
                }
                .to_string()
            })
            .collect();
        Self {
            expected,
            messages,
            wolfbane,
            meat_pie,
            apple_pie,
            pies: case.pies,
            stand: case.stand,
            start_inventory: Arc::new(Mutex::new(None)),
        }
    }

    fn observe(
        &self,
        snapshot: &GameSnapshot,
        status: Option<&ScriptStatus>,
        lifecycle: Option<&script::ScriptLifecycleReceipt>,
        dialogue_seen: &[String],
    ) -> Result<Option<Value>, String> {
        let Some(native) = status else {
            return Ok(None);
        };
        if native.phase != script::native::NativePhase::Blocked {
            if native.failure.is_some() {
                return Err(format!(
                    "negative prerequisite has failure outside Blocked: {native:?}"
                ));
            }
            return Ok(None);
        }
        let failure = native
            .failure
            .as_ref()
            .ok_or("blocked prerequisite cell has no native failure")?;
        if failure.code.as_ref() != "parked"
            || !self
                .messages
                .iter()
                .any(|message| message == failure.message.as_ref())
        {
            return Err(format!(
                "expected exact prerequisite refusal {:?}, got {failure:?}",
                self.messages
            ));
        }
        if snapshot.modals().chat != -1 || snapshot.modals().main != -1 {
            return Err("blocked prerequisite left an open modal".into());
        }
        let inventory_after = inventory_receipt(snapshot);
        let inventory_before = self
            .start_inventory
            .lock()
            .map_err(|_| "start inventory poisoned")?;
        if inventory_before.as_ref() != Some(&inventory_after) {
            return Err(format!("denied prerequisite changed inventory: before={inventory_before:?}, after={inventory_after:?}"));
        }
        require_item(snapshot, self.wolfbane, 0, "dagger_wolfbane")?;
        require_item(snapshot, self.meat_pie, self.pies, "meat_pie")?;
        require_item(snapshot, self.apple_pie, 0, "apple_pie")?;
        if snapshot
            .ground_items()
            .iter()
            .any(|item| item.def.id == self.wolfbane)
        {
            return Err("failed reclaim dropped Wolfbane on the floor".into());
        }
        if saw_text(dialogue_seen, HOLY_ADVICE) || saw_text(dialogue_seen, MORT_FOOD) {
            return Err(format!(
                "denied prerequisite performed a repair: {dialogue_seen:?}"
            ));
        }
        if self.expected == "FreeSlots" && !saw_text(dialogue_seen, DAGGER_RECLAIM_OPENER) {
            return Err("full-pack refusal did not reach the real reclaim opener".into());
        }
        let here = tile(snapshot);
        if self.expected == "quest_start is not allowed"
            && (here != Some(self.stand) || !dialogue_seen.is_empty())
        {
            return Err(format!(
                "request-level Forbid began a repair: here={here:?}, pages={dialogue_seen:?}"
            ));
        }
        Ok(Some(json!({"family_step": "prerequisite-walk",
            "owner": "native Quester Path", "expected_failure": self.expected,
            "expected_native_failures": self.messages,
            "request_quest_start": if self.expected == "quest_start is not allowed" { Some(false) } else { None },
            "final_tile": here, "dialogue_pages": dialogue_seen,
            "inventory_after": inventory_receipt(snapshot),
            "native_status": native_status_receipt(status), "lifecycle": lifecycle})))
    }
}

#[cfg(test)]
mod prereq_failure_oracle_tests {
    use super::*;
    use api::snapshot::{ItemActionFamily, ItemContainer, ItemView};
    use script::native::{NativePhase, ScriptFailure, StatusField};

    fn fixture() -> (PrereqFailureOracle, GameSnapshot, ScriptStatus, Vec<String>) {
        fixture_for("FreeSlots", 28)
    }

    fn fixture_for(
        expected: &'static str,
        pies: i32,
    ) -> (PrereqFailureOracle, GameSnapshot, ScriptStatus, Vec<String>) {
        let (selected, _) = live_data();
        let case = PrereqLiveCase {
            selector: "negative-oracle-unit",
            priest: 60,
            druid: 0,
            bank_dagger: false,
            pies,
            stand: PREREQ_WEST,
            goal: PREREQ_EAST,
            holy: true,
            mort: None,
            advice: false,
            reclaim: false,
            start: false,
        };
        let oracle = PrereqFailureOracle::new(selected, expected, &case);
        let mut snapshot = GameSnapshot::new();
        snapshot.seed_tile(PREREQ_WEST);
        snapshot.seed_inventory(
            (0..pies)
                .map(|slot| ItemView {
                    def: api::ItemDefView {
                        id: oracle.meat_pie,
                        name: None,
                        stackable: false,
                        members: false,
                        base_value: 0,
                        noted: false,
                        certificate_link: -1,
                        certificate_template: -1,
                    },
                    container: ItemContainer::Inventory,
                    action_family: ItemActionFamily::Held,
                    slot,
                    count: 1,
                    actions: vec![],
                    component_id: -1,
                })
                .collect(),
            28,
        );
        *oracle.start_inventory.lock().unwrap() = Some(inventory_receipt(&snapshot));
        let status = ScriptStatus {
            run: RunKey {
                slot: 1,
                run: 1,
                session: 1,
            },
            card: script::CompiledId("Quester"),
            phase: NativePhase::Blocked,
            active_settings: 1,
            pending_settings: None,
            failure: Some(ScriptFailure {
                code: "parked".into(),
                message: oracle.messages[0].as_str().into(),
            }),
            fields: Arc::from([]),
        };
        let pages = if expected == "FreeSlots" {
            vec![DAGGER_RECLAIM_OPENER.into()]
        } else {
            vec![]
        };
        (oracle, snapshot, status, pages)
    }

    #[test]
    fn prereq_negative_oracle_ignores_pre_start_chat() {
        let (oracle, mut snapshot, status, _) = fixture_for("quest_start is not allowed", 0);
        snapshot.seed_chat_lines(vec![ChatLineView {
            type_: 0,
            username: None,
            text: "get druidspirit: 0".into(),
            sequence: 10,
        }]);
        let pages = dialogue_texts_after(&snapshot, 10);
        assert!(oracle
            .observe(&snapshot, Some(&status), None, &pages)
            .unwrap()
            .is_some());
    }

    #[test]
    fn prereq_negative_oracle_still_rejects_fresh_chat() {
        let (oracle, mut snapshot, status, _) = fixture_for("quest_start is not allowed", 0);
        snapshot.seed_chat_lines(vec![ChatLineView {
            type_: 0,
            username: None,
            text: HOLY_ADVICE.into(),
            sequence: 11,
        }]);
        let pages = dialogue_texts_after(&snapshot, 10);
        assert!(oracle
            .observe(&snapshot, Some(&status), None, &pages)
            .is_err());
    }

    #[test]
    fn prereq_negative_oracle_accepts_exact_native_blocked_refusal() {
        let (oracle, snapshot, status, pages) = fixture();
        assert!(oracle
            .observe(&snapshot, Some(&status), None, &pages)
            .unwrap()
            .is_some());
    }

    #[test]
    fn prereq_negative_oracle_rejects_reclaim_effect_without_opener() {
        let (oracle, snapshot, status, _) = fixture();
        assert!(oracle
            .observe(&snapshot, Some(&status), None, &[DAGGER_RECLAIM.into()])
            .is_err());
    }

    #[test]
    fn prereq_negative_oracle_rejects_different_failure() {
        let (oracle, snapshot, mut status, pages) = fixture();
        status.failure.as_mut().unwrap().message =
            format!("{} with a different cause", oracle.messages[0]).into();
        assert!(oracle
            .observe(&snapshot, Some(&status), None, &pages)
            .is_err());
    }

    #[test]
    fn prereq_negative_oracle_rejects_open_modal() {
        for main in [false, true] {
            let (oracle, mut snapshot, status, pages) = fixture();
            if main {
                snapshot.seed_main_modal(1, vec![]);
            } else {
                snapshot.seed_chat_modal(1, vec![]);
            }
            assert!(oracle
                .observe(&snapshot, Some(&status), None, &pages)
                .is_err());
        }
    }

    #[test]
    fn prereq_negative_oracle_rejects_changed_inventory() {
        let (oracle, mut snapshot, status, pages) = fixture();
        snapshot.seed_inventory(vec![], 28);
        assert!(oracle
            .observe(&snapshot, Some(&status), None, &pages)
            .is_err());
    }

    #[test]
    fn prereq_negative_oracle_rejects_repair_attempt() {
        for repair in [HOLY_ADVICE, MORT_FOOD] {
            let (oracle, snapshot, status, mut pages) = fixture();
            pages.push(repair.into());
            assert!(oracle
                .observe(&snapshot, Some(&status), None, &pages)
                .is_err());
        }
    }

    #[test]
    fn prereq_negative_oracle_rejects_working_with_block_reason() {
        let (oracle, snapshot, mut status, pages) = fixture();
        status.phase = NativePhase::Working;
        status.fields = Arc::from([StatusField {
            key: "block_reason",
            label: "Block",
            value: StatusValue::Text(status.failure.as_ref().unwrap().message.clone()),
        }]);
        assert!(oracle
            .observe(&snapshot, Some(&status), None, &pages)
            .is_err());
    }
}

fn run_quester_prereq_case(case: PrereqLiveCase, expected_failure: Option<&'static str>) {
    let (selected, quests) = live_data();
    let wolfbane = item_id(&selected, "dagger_wolfbane");
    let meat_pie = item_id(&selected, "meat_pie");
    let apple_pie = item_id(&selected, "apple_pie");
    let failure_oracle = expected_failure
        .map(|expected| PrereqFailureOracle::new(Arc::clone(&selected), expected, &case));
    let failure_inventory = failure_oracle
        .as_ref()
        .map(|oracle| Arc::clone(&oracle.start_inventory));
    let items: &'static [(&'static str, i32)] = match case.pies {
        0 => &[],
        27 => &[("meat_pie", 27)],
        28 => &[("meat_pie", 28)],
        other => panic!("unsupported exact full-pack fixture: {other}"),
    };
    let (mut cell, seed, mut seed_commands) = make_cell(
        case.selector,
        ("priestperil", "Priest in Peril prerequisite walk"),
        case.priest,
        items,
        case.stand,
        None,
        Arc::clone(&selected),
    );
    let druid_seed = format!("setvar druidspirit {}", case.druid);
    insert_prereq_account_seed(&mut cell, druid_seed.clone());
    seed_commands.push(druid_seed);
    if case.bank_dagger {
        // ClientCheatHandler.ts:355-373 seeds only this account's bank without
        // opening a bank interface or giving the client a known bank snapshot.
        let command = "givebank dagger_wolfbane 1".to_owned();
        insert_prereq_account_seed(&mut cell, command.clone());
        seed_commands.push(command);
    }
    let priest_reply = match case.priest {
        60 => "get priestperil: 60",
        61 => "get priestperil: 61",
        other => panic!("unsupported prerequisite priest stage: {other}"),
    };
    let druid_reply = match case.druid {
        0 => "get druidspirit: 0",
        5 => "get druidspirit: 5",
        10 => "get druidspirit: 10",
        35 => "get druidspirit: 35",
        other => panic!("unsupported prerequisite druid stage: {other}"),
    };
    let priest_query =
        insert_quest_stage_query(&mut cell, "getvar priestperil", priest_reply, None);
    let druid_query = insert_quest_stage_query(&mut cell, "getvar druidspirit", druid_reply, None);
    let start_sequence = Arc::new(AtomicI32::new(i32::MAX));
    let observed_sequence = Arc::clone(&start_sequence);
    let stand = case.stand;
    let pies = case.pies;
    cell.observe_start = Some(Box::new(move |snapshot| {
        let seed_receipt = seed.observe_start(snapshot)?;
        let fixture_receipt = common_observe_start(snapshot, stand)?;
        for (baseline, expected) in [
            (priest_query.load(Ordering::Relaxed), priest_reply),
            (druid_query.load(Ordering::Relaxed), druid_reply),
        ] {
            if quest_stage_witness(snapshot.chat_lines(), baseline, expected).is_none() {
                return Err(format!(
                    "prerequisite fixture lacks fresh server witness {expected:?}"
                ));
            }
        }
        require_item(snapshot, wolfbane, 0, "dagger_wolfbane")?;
        require_item(snapshot, meat_pie, pies, "meat_pie")?;
        require_item(snapshot, apple_pie, 0, "apple_pie")?;
        if let Some(inventory) = failure_inventory.as_ref() {
            *inventory.lock().map_err(|_| "start inventory poisoned")? =
                Some(inventory_receipt(snapshot));
        }
        observed_sequence.store(
            snapshot
                .chat_lines()
                .iter()
                .map(|line| line.sequence)
                .max()
                .unwrap_or(0),
            Ordering::Relaxed,
        );
        Ok(json!({"fixture": fixture_receipt, "seed": seed_receipt,
            "account_seed_commands": seed_commands,
            "server_stage_witnesses": [priest_reply, druid_reply],
            "bank_memory": "unknown; no bank interface was opened"}))
    }));
    let recipe = case.holy && case.mort.is_none();
    let mut document = prerequisite_path(case.goal, recipe);
    if expected_failure == Some("quest_start is not allowed") {
        document.prereq.quest_start = Some(false);
    }
    let start = family_start(document, Arc::clone(&selected), quests);
    let mut dialogue_seen = Vec::new();
    let mut holy_landing = false;
    let mut mort_landing = false;
    let mut pass_receipts = Vec::new();
    let selector = case.selector;
    let observe: ObserveFamily = Box::new(move |snapshot, status, lifecycle| {
        let baseline = start_sequence.load(Ordering::Relaxed);
        if failure_oracle.is_some() {
            remember_dialogue_texts_after(snapshot, baseline, &mut dialogue_seen);
        } else {
            remember_dialogue_texts(snapshot, &mut dialogue_seen);
        }
        let here = tile(snapshot);
        if let Some(oracle) = failure_oracle.as_ref() {
            return oracle.observe(snapshot, status, lifecycle, &dialogue_seen);
        }
        for (message, required, landings, seen) in [
            (
                HOLY_PASS,
                case.holy,
                [PREREQ_EAST, PREREQ_EAST],
                &mut holy_landing,
            ),
            (
                if case.mort == Some(false) {
                    MORT_OUT
                } else {
                    MORT_IN
                },
                case.mort.is_some(),
                if case.mort == Some(false) {
                    [
                        WorldTile {
                            x: 3443,
                            z: 3458,
                            level: 0,
                        },
                        WorldTile {
                            x: 3444,
                            z: 3458,
                            level: 0,
                        },
                    ]
                } else {
                    [
                        WorldTile {
                            x: 3443,
                            z: 3457,
                            level: 0,
                        },
                        WorldTile {
                            x: 3444,
                            z: 3457,
                            level: 0,
                        },
                    ]
                },
                &mut mort_landing,
            ),
        ] {
            if !required || *seen || !here.is_some_and(|here| landings.contains(&here)) {
                continue;
            }
            if let Some(line) = quest_stage_witness(snapshot.chat_lines(), baseline, message) {
                *seen = true;
                pass_receipts.push(json!({"message": message, "sequence": line.sequence,
                    "landing": here, "tick": snapshot.tick()}));
            }
        }
        if !completed_lifecycle(lifecycle)
            || !here.is_some_and(|here| within(here, case.goal, 1))
            || (case.holy && !holy_landing)
            || (case.mort.is_some() && !mort_landing)
            || snapshot.modals().chat != -1
            || snapshot.modals().main != -1
        {
            return Ok(None);
        }
        for (expected, fragment) in [
            (case.advice, HOLY_ADVICE),
            (case.reclaim, DAGGER_RECLAIM),
            (case.start, MORT_FOOD),
        ] {
            if saw_text(&dialogue_seen, fragment) != expected {
                return Err(format!("prerequisite branch {fragment:?}: expected observed={expected}; pages={dialogue_seen:?}"));
            }
        }
        require_item(
            snapshot,
            wolfbane,
            i32::from(case.reclaim),
            "dagger_wolfbane",
        )?;
        require_item(
            snapshot,
            meat_pie,
            case.pies + if case.start { 3 } else { 0 },
            "meat_pie",
        )?;
        require_item(
            snapshot,
            apple_pie,
            if case.start { 3 } else { 0 },
            "apple_pie",
        )?;
        Ok(Some(
            json!({"family_step": "prerequisite-walk", "owner": "native Quester Path",
            "released_recipe": if recipe { Some("open:morytania") } else { None },
            "goal": case.goal, "final_tile": here, "fresh_exact_landing_passes": pass_receipts,
            "dialogue_pages": dialogue_seen, "inventory_after": inventory_receipt(snapshot),
            "native_status": native_status_receipt(status), "lifecycle": lifecycle}),
        ))
    });
    let result = if expected_failure.is_some() {
        quester_live::run_family_expected_blocked(cell, start, observe)
    } else {
        quester_live::run_family(cell, start, observe)
    };
    result.unwrap_or_else(|error| panic!("{selector} native prerequisite proof failed: {error}"));
    eprintln!("prereq cell outcome=completed test={selector}");
}

#[test]
#[ignore = "requires LIVE=1 and the already-running shared local Engine Q"]
fn live_quester_prereq_holy_barrier() {
    run_quester_prereq_cell(PrereqLiveCase {
        selector: "live_quester_prereq_holy_barrier",
        priest: 60,
        druid: 0,
        bank_dagger: true,
        pies: 0,
        stand: PREREQ_WEST,
        goal: PREREQ_EAST,
        holy: true,
        mort: None,
        advice: true,
        reclaim: false,
        start: false,
    });
}

#[test]
#[ignore = "requires LIVE=1 and the already-running shared local Engine Q"]
fn live_quester_prereq_reclaim_then_advice() {
    run_quester_prereq_cell(PrereqLiveCase {
        selector: "live_quester_prereq_reclaim_then_advice",
        priest: 60,
        druid: 0,
        bank_dagger: false,
        pies: 27,
        stand: PREREQ_WEST,
        goal: PREREQ_EAST,
        holy: true,
        mort: None,
        advice: true,
        reclaim: true,
        start: false,
    });
}

#[test]
#[ignore = "requires LIVE=1 and the already-running shared local Engine Q"]
fn live_quester_prereq_bank_unknown_full_pack() {
    run_quester_prereq_cell(PrereqLiveCase {
        selector: "live_quester_prereq_bank_unknown_full_pack",
        priest: 60,
        druid: 0,
        bank_dagger: true,
        pies: 28,
        stand: PREREQ_WEST,
        goal: PREREQ_EAST,
        holy: true,
        mort: None,
        advice: true,
        reclaim: false,
        start: false,
    });
}

#[test]
#[ignore = "requires LIVE=1 and the already-running shared local Engine Q"]
fn live_quester_prereq_authoritative_pass() {
    run_quester_prereq_cell(PrereqLiveCase {
        selector: "live_quester_prereq_authoritative_pass",
        priest: 61,
        druid: 35,
        bank_dagger: false,
        pies: 28,
        stand: PREREQ_WEST,
        goal: PREREQ_EAST,
        holy: true,
        mort: None,
        advice: false,
        reclaim: false,
        start: false,
    });
}

#[test]
#[ignore = "requires LIVE=1 and the already-running shared local Engine Q"]
fn live_quester_prereq_mort_myre() {
    run_quester_prereq_cell(PrereqLiveCase {
        selector: "live_quester_prereq_mort_myre",
        priest: 61,
        druid: 0,
        bank_dagger: true,
        pies: 0,
        stand: PREREQ_EAST,
        goal: PREREQ_SWAMP,
        holy: false,
        mort: Some(true),
        advice: false,
        reclaim: false,
        start: true,
    });
}

#[test]
#[ignore = "requires LIVE=1 and the already-running shared local Engine Q"]
fn live_quester_prereq_both_gates() {
    run_quester_prereq_cell(PrereqLiveCase {
        selector: "live_quester_prereq_both_gates",
        priest: 60,
        druid: 0,
        bank_dagger: true,
        pies: 0,
        stand: PREREQ_WEST,
        goal: PREREQ_SWAMP,
        holy: true,
        mort: Some(true),
        advice: true,
        reclaim: false,
        start: true,
    });
}

#[test]
#[ignore = "requires LIVE=1 and the already-running shared local Engine Q"]
fn live_quester_prereq_mort_myre_outward() {
    run_quester_prereq_cell(PrereqLiveCase {
        selector: "live_quester_prereq_mort_myre_outward",
        priest: 61,
        druid: 10,
        bank_dagger: true,
        pies: 0,
        stand: PREREQ_SWAMP,
        goal: PREREQ_EAST,
        holy: false,
        mort: Some(false),
        advice: false,
        reclaim: false,
        start: false,
    });
}

#[test]
#[ignore = "requires LIVE=1 and the already-running shared local Engine Q"]
fn live_quester_prereq_reclaim_full_pack_refused() {
    run_quester_prereq_case(
        PrereqLiveCase {
            selector: "live_quester_prereq_reclaim_full_pack_refused",
            priest: 60,
            druid: 0,
            bank_dagger: false,
            pies: 28,
            stand: PREREQ_WEST,
            goal: PREREQ_EAST,
            holy: true,
            mort: None,
            advice: false,
            reclaim: false,
            start: false,
        },
        Some("FreeSlots"),
    );
}

#[test]
#[ignore = "requires LIVE=1 and the already-running shared local Engine Q"]
fn live_quester_prereq_authoritative_bank_unknown_full_pack() {
    run_quester_prereq_cell(PrereqLiveCase {
        selector: "live_quester_prereq_authoritative_bank_unknown_full_pack",
        priest: 61,
        druid: 35,
        bank_dagger: true,
        pies: 28,
        stand: PREREQ_WEST,
        goal: PREREQ_EAST,
        holy: true,
        mort: None,
        advice: false,
        reclaim: false,
        start: false,
    });
}

#[test]
#[ignore = "requires LIVE=1 and the already-running shared local Engine Q"]
fn live_quester_prereq_mort_myre_already_started() {
    run_quester_prereq_cell(PrereqLiveCase {
        selector: "live_quester_prereq_mort_myre_already_started",
        priest: 61,
        druid: 5,
        bank_dagger: true,
        pies: 0,
        stand: PREREQ_EAST,
        goal: PREREQ_SWAMP,
        holy: false,
        mort: Some(true),
        advice: false,
        reclaim: false,
        start: false,
    });
}

#[test]
#[ignore = "requires LIVE=1 and the already-running shared local Engine Q"]
fn live_quester_prereq_mort_myre_request_forbid() {
    run_quester_prereq_case(
        PrereqLiveCase {
            selector: "live_quester_prereq_mort_myre_request_forbid",
            priest: 61,
            druid: 0,
            bank_dagger: true,
            pies: 0,
            stand: PREREQ_EAST,
            goal: PREREQ_SWAMP,
            holy: false,
            mort: Some(true),
            advice: false,
            reclaim: false,
            start: false,
        },
        Some("quest_start is not allowed"),
    );
}
