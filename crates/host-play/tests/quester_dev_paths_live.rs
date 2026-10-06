//! Folder override smoke through the normal compiled-card Start and shared headless runner.
#![cfg(all(feature = "live-harness", feature = "test-support"))]

#[path = "common/quester_live.rs"]
mod quester_live;

use api::selected::{ClientRevision, FamilyPreparation};
use quester_live::{Cell, Mode};
use script::native::{ScriptStatus, StatusValue};
use script::quester::registry::{self, FolderSource};
use serde_json::json;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

const COMMENT: &str = "Folder Path live-edit witness: accept Cook's request";
const RELOADED_COMMENT: &str = "Folder Path Reload witness: edited without restart";

struct StartLogSink(Mutex<Vec<String>>);
impl api::hostlog::Sink for StartLogSink {
    fn record(&self, record: &api::hostlog::Record<'_>) {
        if record.message.starts_with("Quester Start: Path cook ") {
            self.0.lock().unwrap().push(format!(
                "[{}] {}",
                record.slot.unwrap_or("-"),
                record.message
            ));
        }
    }
}
static START_LOG: StartLogSink = StartLogSink(Mutex::new(Vec::new()));

fn text<'a>(status: &'a ScriptStatus, key: &str) -> Option<&'a str> {
    status.fields.iter().find_map(|field| match &field.value {
        StatusValue::Text(value) if field.key == key => Some(value.as_ref()),
        _ => None,
    })
}

#[test]
#[ignore = "requires LIVE=1, shared local 289 engine, CPU captures and isolated HOME"]
fn live_quester_folder_override_start() {
    assert_eq!(std::env::var("LIVE").as_deref(), Ok("1"));
    assert!(api::hostlog::install_sink(&START_LOG));
    let selected = FamilyPreparation::run(|_| {
        api::game_data::for_revision(ClientRevision::R289).expect("selected 289 data")
    })
    .expect("selected data worker")
    .join()
    .expect("selected data");
    let evidence = PathBuf::from(std::env::var_os("LIVE_EVIDENCE_DIR").expect("LIVE_EVIDENCE_DIR"));
    let folder = evidence.join("folder-source").join("289");
    std::fs::create_dir_all(&folder).unwrap();
    let mut document: serde_json::Value =
        serde_json::from_slice(script::quester::compile::cook_bytes()).unwrap();
    document["roles"][0]["sequences"][0]["steps"][0]["comment"] = json!(COMMENT);
    let bytes = serde_json::to_vec_pretty(&document).unwrap();
    let digest = registry::digest_text(&script::quester::compile::digest_bytes(&bytes));
    let start_log =
        format!("Quester Start: Path cook source=folder digest={digest} step_comment={COMMENT:?}");
    std::fs::write(folder.join("cook.json"), bytes).unwrap();
    let before = registry::source();
    registry::set_source(FolderSource {
        enabled: true,
        folder: folder.clone(),
    });

    let mut scenario = scenario::quester_stage(
        "qd-folder-path-start",
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
    scenario.settings.deadline = Duration::from_secs(180);
    let settings = serde_json::from_value(json!({"quests":["cook"]})).unwrap();
    let result = quester_live::run_family(
        Cell {
            quest: "cook",
            display: "Cook's Assistant",
            label: "folder-path-start".into(),
            scenario,
            start_settings: settings,
            mode: Mode::Stage {
                expect: vec!["cook:0".into()],
            },
            observe_start: Some(Box::new(|snapshot| {
                if !snapshot.ingame() || snapshot.scene_state() != 2 {
                    return Err("fixture must be ingame with scene_state=2 before Start".into());
                }
                Ok(json!({"ingame":true,"fixture":"Cook stage zero, no quest item seeds"}))
            })),
        },
        Box::new(|handle, account, _| {
            handle.start_compiled(
                account,
                script::CompiledId("Quester"),
                serde_json::from_value(json!({"quests":["cook"]})).unwrap(),
            )
        }),
        Box::new(move |_, status, _lifecycle| {
            let Some(status) = status else {
                return Ok(None);
            };
            if text(status, "path_source") == Some("folder")
                && text(status, "step_comment") == Some(COMMENT)
            {
                document["roles"][0]["sequences"][0]["steps"][0]["comment"] =
                    json!(RELOADED_COMMENT);
                let edited =
                    serde_json::to_vec_pretty(&document).map_err(|error| error.to_string())?;
                let reloaded_digest =
                    registry::digest_text(&script::quester::compile::digest_bytes(&edited));
                std::fs::write(folder.join("cook.json"), edited)
                    .map_err(|error| error.to_string())?;
                let reload_data = Arc::clone(&selected);
                let reloaded =
                    FamilyPreparation::run(move |worker| registry::reload(&reload_data, worker))
                        .map_err(|error| format!("{error:?}"))?
                        .join()
                        .map_err(|error| format!("{error:?}"))?
                        .map_err(|error| format!("{error:?}"))?;
                let bytes = reloaded.bytes("cook").ok_or("Reload lost Cook's Path")?;
                let loaded: serde_json::Value =
                    serde_json::from_slice(bytes.as_ref()).map_err(|error| error.to_string())?;
                if loaded["roles"][0]["sequences"][0]["steps"][0]["comment"] != RELOADED_COMMENT
                    || reloaded_digest == digest
                {
                    return Err("Reload did not publish the edited folder document".into());
                }
                return Ok(Some(json!({
                    "path_source":"folder", "edited_comment":COMMENT, "sha256":digest,
                    "folder":folder, "reload_comment":RELOADED_COMMENT,
                    "reload_sha256":reloaded_digest, "reload_published":true,
                    "active_comment_retained":text(status, "step_comment"),
                })));
            }
            Ok(None)
        }),
    );
    registry::set_source(before);
    let receipt =
        result.expect("folder override must Start, publish its comment and reload an edit");
    assert!(!receipt["captures"].as_array().unwrap().is_empty());
    let logs = START_LOG.0.lock().unwrap().join("\n");
    std::fs::write(evidence.join("script.log"), &logs).expect("retain real host-log sink lines");
    assert!(logs.contains(&start_log), "missing {start_log} in {logs}");
}
