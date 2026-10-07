//! `members_world` predicate against the real engine: a folder override of
//! Cook's Assistant whose first (`start`) step is guarded by the predicate.
//! The expected value is the engine's own declaration
//! (`$WORLD_ENGINE_DIR/data/config/world.json` `node.members`) and, as a
//! server-side cross-check independent of the host profile, the account
//! membership the server sent at login (`UPDATE_PID`, `world().members`):
//! a members world only admits members accounts.
//!
//! Positive cell: `skip_if = members_world`. A members world must log
//! `step start skipped: members_world({}) evaluated true`; a free-to-play
//! world must select the step instead. Negative cell: `skip_if =
//! {"Not": members_world}` with the opposite outcome.
//!
//! Ignored unless LIVE=1; environment and evidence layout in
//! `common/quester_live.rs`.
#![cfg(all(feature = "live-harness", feature = "test-support"))]

#[path = "common/quester_live.rs"]
mod quester_live;

use api::selected::{ClientRevision, FamilyPreparation};
use quester_live::{Cell, Mode};
use script::native::{ScriptStatus, StatusValue};
use script::quester::registry::{self, FolderSource};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

struct SkipLog(Mutex<Vec<String>>);
impl api::hostlog::Sink for SkipLog {
    fn record(&self, record: &api::hostlog::Record<'_>) {
        if record.message.starts_with("quester cook:") && record.message.contains(" skipped: ") {
            self.0.lock().unwrap().push(record.message.to_string());
        }
    }
}
static SKIP_LOG: SkipLog = SkipLog(Mutex::new(Vec::new()));

fn text<'a>(status: &'a ScriptStatus, key: &str) -> Option<&'a str> {
    status.fields.iter().find_map(|field| match &field.value {
        StatusValue::Text(value) if field.key == key => Some(value.as_ref()),
        _ => None,
    })
}

fn engine_members() -> bool {
    let engine = PathBuf::from(std::env::var_os("WORLD_ENGINE_DIR").expect("WORLD_ENGINE_DIR"));
    let world: Value =
        serde_json::from_slice(&std::fs::read(engine.join("data/config/world.json")).unwrap())
            .unwrap();
    world["node"]["members"]
        .as_bool()
        .expect("engine world.json node.members")
}

fn cell(label: &str, guard: Value, expect_skip: bool) {
    assert_eq!(std::env::var("LIVE").as_deref(), Ok("1"));
    static INSTALL: std::sync::Once = std::sync::Once::new();
    INSTALL.call_once(|| assert!(api::hostlog::install_sink(&SKIP_LOG)));
    SKIP_LOG.0.lock().unwrap().clear();
    let selected = FamilyPreparation::run(|_| {
        api::game_data::for_revision(ClientRevision::R289).expect("selected 289 data")
    })
    .expect("selected data worker")
    .join()
    .expect("selected data");
    drop(selected);
    let evidence = PathBuf::from(std::env::var_os("LIVE_EVIDENCE_DIR").expect("LIVE_EVIDENCE_DIR"));
    let folder = evidence.join(format!("folder-{label}")).join("289");
    std::fs::create_dir_all(&folder).unwrap();
    let mut document: Value =
        serde_json::from_slice(script::quester::compile::cook_bytes()).unwrap();
    let step = &mut document["roles"][0]["sequences"][0]["steps"][0];
    assert_eq!(step["id"], "start");
    step["skip_if"] = guard.clone();
    std::fs::write(
        folder.join("cook.json"),
        serde_json::to_vec_pretty(&document).unwrap(),
    )
    .unwrap();
    let before = registry::source();
    registry::set_source(FolderSource {
        enabled: true,
        folder: folder.clone(),
    });

    let engine_says_members = engine_members();
    let skip_expected = expect_skip == engine_says_members;
    let mut scenario = scenario::quester_stage(
        "qmw-members-world",
        "Cook's Assistant",
        "cookquest",
        0,
        &[],
        api::WorldTile {
            x: 3209,
            z: 3215,
            level: 0,
        },
    );
    scenario.settings.deadline = Duration::from_secs(120);
    let label_owned = label.to_owned();
    let mut dwell = 0u32;
    let result = quester_live::run_family(
        Cell {
            quest: "cook",
            display: "Cook's Assistant",
            label: label.to_owned(),
            scenario,
            start_settings: serde_json::from_value(json!({"quests":["cook"]})).unwrap(),
            mode: Mode::Stage {
                expect: vec!["cook:0".into()],
            },
            observe_start: None,
        },
        Box::new(|handle, account, _| {
            handle.start_compiled(
                account,
                script::CompiledId("Quester"),
                serde_json::from_value(json!({"quests":["cook"]})).unwrap(),
            )
        }),
        Box::new(move |snapshot, status, _| {
            let Some(status) = status else {
                return Ok(None);
            };
            let skips = SKIP_LOG.0.lock().unwrap().clone();
            let skipped = skips
                .iter()
                .any(|line| line.contains("step start skipped: "));
            let selected_start = text(status, "step_id") == Some("start");
            // A skipped step stays the reported step id, so the skip line,
            // not the step id, decides the positive cell. The negative cell
            // must see the step running and then keep seeing no skip line
            // for a dwell of observations.
            let done = if skip_expected {
                skipped
            } else if selected_start && !skipped {
                dwell += 1;
                dwell >= 50
            } else {
                false
            };
            if skipped && !skip_expected {
                return Err(format!(
                    "unexpected outcome: skipped={skipped} selected_start={selected_start}"
                ));
            }
            if !done {
                return Ok(None);
            }
            if !snapshot.ingame() {
                return Err("evidence arrived while not in game".into());
            }
            let receipt = json!({
                "label": label_owned,
                "guard": guard,
                "engine_world_json_node_members": engine_says_members,
                "server_account_members_flag_update_pid": snapshot.world().members,
                "skip_expected": skip_expected,
                "skipped_logged": skipped,
                "step_id": text(status, "step_id"),
                "skip_lines": skips,
            });
            Ok(Some(receipt))
        }),
    );
    registry::set_source(before);
    let receipt = result.expect("members_world cell must observe the expected outcome");
    // A members world only admits a members account; a free-to-play world
    // may still send either flag, so only the members direction is asserted.
    if engine_says_members {
        assert_eq!(
            receipt["family_receipt"]["server_account_members_flag_update_pid"],
            json!(true),
            "{receipt}"
        );
    }
    std::fs::write(
        evidence.join(format!("members-world-{label}.json")),
        serde_json::to_vec_pretty(&receipt).unwrap(),
    )
    .unwrap();
}

#[test]
#[ignore = "requires LIVE=1, shared local 289 engine, CPU captures and isolated HOME"]
fn members_world_true_skips_on_a_members_engine() {
    cell(
        "positive",
        json!({"Fact":{"kind":"members_world","version":1,"args":{}}}),
        true,
    );
}

#[test]
#[ignore = "requires LIVE=1, shared local 289 engine, CPU captures and isolated HOME"]
fn not_members_world_selects_on_a_members_engine() {
    cell(
        "negative",
        json!({"Not":{"Fact":{"kind":"members_world","version":1,"args":{}}}}),
        false,
    );
}
