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
const EXTRA_ITEMS: [(&str, i32); 1] = [("knife", 1)];
const STAND: WorldTile = WorldTile {
    x: 3145,
    z: 3202,
    level: 0,
};

fn base40_seed_steps() -> Vec<scenario::Step> {
    let mut steps = Vec::with_capacity(BASE40_STATS.len() + 2);
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
    for &(skill, level) in &BASE40_STATS {
        let name = Skill::names[usize::from(skill)];
        steps.push(scenario::Step {
            name: "seed explicit Base40 stat",
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
    let selected = api::game_data::for_revision(api::selected::ClientRevision::R289)?;
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
    let mut stage_values = progress
        .rules
        .iter()
        .filter(|rule| rule.stage.0.as_ref() == STAGE)
        .filter_map(|rule| rule.varp);
    let stage_value = stage_values
        .next()
        .ok_or("Zanaris stage has no authored varp hint")?;
    if stage_values.any(|value| value != stage_value) {
        return Err("Zanaris stage has contradictory varp hints".into());
    }
    let knife_id = selected
        .item_by_alias("knife")
        .map(|item| item.id)
        .ok_or("selected R289 data has no knife item")?;

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
    // Keep Zanaris out of the qualified-quest profile table and seed Base40 here.
    let observe_start = Box::new(move |snapshot: &GameSnapshot| {
        if !snapshot.ingame() || snapshot.scene_state() != 2 {
            return Err("Base40 seed was not observed in an active game".into());
        }
        let mut observed_stats = Vec::with_capacity(BASE40_STATS.len());
        for (skill, level) in BASE40_STATS {
            let stat = snapshot
                .stats()
                .iter()
                .find(|stat| stat.index == i32::from(skill))
                .ok_or_else(|| format!("Base40 stat {skill} is unobserved"))?;
            if stat.base != level || stat.effective != level {
                return Err(format!(
                    "Base40 stat {skill} expected {level}, observed base {} effective {}",
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
            return Err(format!("Base40 fixture knife expected 1, observed {knife_count}"));
        }
        Ok(json!({
            "profile": "Base40",
            "stats": observed_stats,
            "knife_id": knife_id,
            "knife_count": knife_count,
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
