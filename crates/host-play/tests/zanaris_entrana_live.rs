#![cfg(all(feature = "live-harness", feature = "test-support"))]

#[path = "common/quester_live.rs"]
mod quester_live;

use api::snapshot::{GameSnapshot, WorldTile};
use client::client::skill::Skill;
use quester_live::{Cell, Mode};
use script::quester::path::PathKind;
use script::quester::registry;
use serde_json::{json, Map};
use std::path::PathBuf;

const QUEST: &str = "zanaris";
const DISPLAY: &str = "Lost City";
const STAGE: &str = "zanaris:1";
const NEXT_STAGE: &str = "zanaris:2";
// R289 content defines `^zanaris_started = 1` in `scripts/quests/quest_zanaris/configs/quest_zanaris.constant`.
const STAGE_VALUE: i32 = 1;
const BASE40_LEVEL: i32 = scenario::quester::TestProfile::Base40.combat_level() as i32;
const BASE40_STATS: [(u8, i32); 7] = [
    (0, BASE40_LEVEL),
    (1, BASE40_LEVEL),
    (2, BASE40_LEVEL),
    (3, BASE40_LEVEL),
    (4, BASE40_LEVEL),
    (5, 43),
    (6, BASE40_LEVEL),
];
// R289 content checks Woodcutting 36 at `scripts/quests/quest_zanaris/scripts/quest_zanaris.rs2:28-40`.
// It checks Crafting 31 at the same file's lines 77-87.
const ZANARIS_CONTENT_STATS: [(u8, i32); 2] = [
    (8, 36),  // `Skill::names[8]` is Woodcutting.
    (12, 31), // `Skill::names[12]` is Crafting.
];
// The R289 Path reserves 16 coins for `buy-axe-for-shamus` (`zanaris.json:42,196-200`).
// `axes.obj:35-38` sets the axe cost to 16, and `lumbridge.inv:1-10` stocks it.
const ZANARIS_COIN_FLOAT: i32 = 16;
const EXTRA_ITEMS: [(&str, i32); 2] = [("knife", 1), ("coins", ZANARIS_COIN_FLOAT)];
const STAND: WorldTile = WorldTile {
    x: 3145,
    z: 3202,
    level: 0,
};

fn base40_seed_steps() -> Vec<scenario::Step> {
    let mut steps = Vec::with_capacity(BASE40_STATS.len() + ZANARIS_CONTENT_STATS.len() + 2);
    steps.push(scenario::Step {
        name: "reset stats before explicit Base40 seed",
        kind: scenario::StepKind::Perform {
            send: Box::new(|client, _| api::interact::cheat(client, "minme").is_sent()),
        },
        wait: scenario::Wait {
            arm: scenario::Proof::StatAtMost { id: 1, max: 1 },
            budget_ticks: 80,
        },
    });
    steps.push(scenario::Step {
        name: "clear worn equipment before Base40 seed",
        kind: scenario::StepKind::Perform {
            send: Box::new(|client, _| api::interact::cheat(client, "~clearinv worn").is_sent()),
        },
        wait: scenario::Wait {
            arm: scenario::Proof::SideTabAvailable { index: 3 },
            budget_ticks: 80,
        },
    });
    for &(skill, level) in BASE40_STATS.iter().chain(ZANARIS_CONTENT_STATS.iter()) {
        let name = Skill::names[usize::from(skill)];
        steps.push(scenario::Step {
            name: "seed explicit account stat",
            kind: scenario::StepKind::Perform {
                send: Box::new(move |client, _| {
                    api::interact::cheat(client, &format!("setstat {name} {level}")).is_sent()
                }),
            },
            wait: scenario::Wait {
                arm: scenario::Proof::Stat {
                    id: i32::from(skill),
                    min: level,
                },
                budget_ticks: 80,
            },
        });
    }
    steps
}

fn zanaris_navigation_cell() -> Result<Cell, String> {
    let (path, _) = scenario::quester::load_quester_path_document(QUEST)?;
    if path.id.0.as_ref() != QUEST
        || path.kind != PathKind::Quest
        || path.partner.is_some()
        || path.roles.len() != 1
    {
        return Err("Zanaris navigation cell needs one single-account quest Path".into());
    }
    let selected = api::game_data::for_revision(api::selected::ClientRevision::R289)
        .map_err(|error| error.to_string())?;
    let identity = selected
        .quest_identity()
        .and_then(|table| table.rows.iter().find(|row| row.id == QUEST))
        .ok_or_else(|| "selected R289 data has no Zanaris quest identity".to_owned())?;
    if identity.display != DISPLAY || identity.varp != QUEST {
        return Err(format!(
            "Zanaris identity changed: display {:?}, varp {:?}",
            identity.display, identity.varp
        ));
    }
    let progress = path.roles[0]
        .progress
        .as_ref()
        .ok_or("Zanaris Path has no progress program")?;
    let mut stage_found = false;
    for rule in &progress.rules {
        if rule.stage.0.as_ref() != STAGE {
            continue;
        }
        stage_found = true;
        if let Some(value) = rule.varp {
            if value != STAGE_VALUE {
                return Err("Zanaris stage has contradictory varp hints".into());
            }
        }
    }
    if !stage_found {
        return Err("Zanaris Path has no stage 1 progress rule".into());
    }
    let stage_value = STAGE_VALUE;
    let knife_id = selected
        .item_by_alias("knife")
        .map(|item| item.id)
        .ok_or("selected R289 data has no knife item")?;
    let coins_id = selected
        .item_by_alias("coins")
        .map(|item| item.id)
        .ok_or("selected R289 data has no coins item")?;

    let mut scenario = scenario::quester_stage_with_watch(
        "zanaris-entrana-dungeon-entry",
        DISPLAY,
        QUEST,
        stage_value,
        &EXTRA_ITEMS,
        STAND,
        12_000,
    );
    scenario.settings.deadline = std::time::Duration::from_secs(3600);
    let seed_index = scenario
        .steps
        .iter()
        .position(|step| step.name == "reset quest stage")
        .ok_or("quest stage seed step is missing")?;
    scenario
        .steps
        .splice(seed_index..seed_index, base40_seed_steps());
    let start_settings = Map::from_iter([("quests".into(), json!([QUEST]))]);
    scenario.settings.script_settings_overrides = Some(start_settings.clone());

    // This cell proves one navigation leg, not Zanaris qualification.
    // Keep Zanaris out of the qualified-quest profile table.
    // Seed Base40 combat and the required R289 path skills locally.
    let observe_start = Box::new(move |snapshot: &GameSnapshot| {
        if !snapshot.ingame() || snapshot.scene_state() != 2 {
            return Err("account seed was not observed in an active game".into());
        }
        let mut observed_stats =
            Vec::with_capacity(BASE40_STATS.len() + ZANARIS_CONTENT_STATS.len());
        for &(skill, level) in BASE40_STATS.iter().chain(ZANARIS_CONTENT_STATS.iter()) {
            let stat = snapshot
                .stats()
                .iter()
                .find(|stat| stat.index == i32::from(skill))
                .ok_or_else(|| format!("seeded stat {skill} is unobserved"))?;
            if stat.base != level || stat.effective != level {
                return Err(format!(
                    "seeded stat {skill} expected {level}, observed base {} effective {}",
                    stat.base, stat.effective
                ));
            }
            observed_stats.push(json!({
                "skill": Skill::names[usize::from(skill)],
                "base": stat.base,
                "effective": stat.effective
            }));
        }
        if snapshot.tile() != Some((STAND.x, STAND.z, STAND.level)) {
            return Err(format!(
                "Base40 seed stand expected ({},{},{}), observed {:?}",
                STAND.x,
                STAND.z,
                STAND.level,
                snapshot.tile()
            ));
        }
        let knife_count: i32 = snapshot
            .inventory()
            .iter()
            .filter(|item| item.def.id == knife_id)
            .map(|item| item.count)
            .sum();
        if knife_count < 1 {
            return Err(format!(
                "Base40 fixture knife expected 1, observed {knife_count}"
            ));
        }
        let coin_count: i32 = snapshot
            .inventory()
            .iter()
            .filter(|item| item.def.id == coins_id)
            .map(|item| item.count)
            .sum();
        if coin_count != ZANARIS_COIN_FLOAT {
            return Err(format!(
                "Base40 fixture coins expected {}, observed {coin_count}",
                ZANARIS_COIN_FLOAT
            ));
        }
        Ok(json!({
            "profile": "Base40 combat + Zanaris content stats",
            "stats": observed_stats,
            "knife_id": knife_id,
            "knife_count": knife_count,
            "coins_id": coins_id,
            "coin_count": coin_count,
            "tile": {"x": STAND.x, "z": STAND.z, "level": STAND.level}
        }))
    });
    Ok(Cell {
        quest: QUEST,
        display: DISPLAY,
        label: "dungeon-entry".to_owned(),
        scenario,
        start_settings,
        mode: Mode::Stage {
            expect: vec![NEXT_STAGE.to_owned()],
        },
        observe_start: Some(observe_start),
    })
}

struct SourceGuard(Option<registry::FolderSource>);

impl Drop for SourceGuard {
    fn drop(&mut self) {
        if let Some(source) = self.0.take() {
            registry::set_source(source);
        }
    }
}

#[test]
#[ignore = "requires LIVE=1 and the selected local 289 engine; uses a content-backed Path"]
fn live_zanaris_entrana_dungeon_entry() {
    if !api::test_support::game_data_available() {
        return;
    }
    assert_eq!(std::env::var("LIVE").as_deref(), Ok("1"));
    let path_dir = PathBuf::from(std::env::var_os("QUESTER_PATH_DIR").expect("QUESTER_PATH_DIR"));
    let prior_source = registry::source();
    let _source_guard = SourceGuard(Some(prior_source));
    let source = scenario::quester::reload_quester_path_source("zanaris", Some(&path_dir)).unwrap();
    assert!(
        matches!(source, "folder" | "draft"),
        "expected a folder-backed Path, got {source}"
    );

    let cell = zanaris_navigation_cell().unwrap();
    let start_settings = cell.start_settings.clone();
    let receipt = quester_live::run_family(
        cell,
        Box::new(move |handle, account, _| {
            handle.start_compiled(
                account,
                script::CompiledId("Quester"),
                start_settings.clone(),
            )
        }),
        Box::new(|snapshot, _, _| {
            if let Some((x, z, level)) = snapshot.tile() {
                if (2816..=2879).contains(&x) && (9728..=9791).contains(&z) && level == 0 {
                    return Ok(Some(json!({
                        "event": "entrana-dungeon-entry",
                        "tile": {"x": x, "z": z, "level": level},
                        "stop_condition": "first observed tile inside the dungeon region"
                    })));
                }
            }
            Ok(None)
        }),
    )
    .expect("Lost City Path must reach the Entrana dungeon on the selected nav pack");
    println!("LOST-CITY-DUNGEON-ENTRY {receipt}");
}
