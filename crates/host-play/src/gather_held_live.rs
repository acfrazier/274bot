//! Detect-only random events must not freeze Gatherer behind a held box.
//! Uses only account cheats and Catherby's real bank and fishing content.
//! Boxes cannot be banked (`bank.rs2:135-138`), so this seed must reach
//! the bank and report its deposit failure, not silently wait for Guardian.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use api::snapshot::GameSnapshot;
use parking_lot::Mutex;
use script::native::{NativePhase, ScriptStatus, StatusValue};
use vault::{Profile, ProfileSettings};

use super::bank_core_live::{
    check_prerequisites, count, frame, live_profile, Cell, Prep, CAPTURE_GRACE,
};
use super::{run_with_template, SharedClientTemplate};

const HARPOON: i32 = 311;
const BOX: i32 = 3062;
const SHARK: i32 = 383;
const BOUND: Duration = Duration::from_secs(90);
const SEED: &[&str] = &[
    "setstat fishing 99",
    "setstat defence 99",
    "setstat hitpoints 99",
    "give harpoon 1",
    "give macro_cube 28",
    "give raw_shark 26",
    "tele 0,44,53,24,44",
];

fn text(status: &ScriptStatus, key: &str) -> String {
    status
        .fields
        .iter()
        .find_map(|field| {
            if field.key == key {
                if let StatusValue::Text(value) = &field.value {
                    return Some(value.to_string());
                }
            }
            None
        })
        .unwrap_or_default()
}

fn settings() -> serde_json::Map<String, serde_json::Value> {
    serde_json::json!({
        "skill": "Fishing", "fishingMethod": "fishing.memberfish.op3",
        "location": "Site", "site": "fishing.catherby", "radius": 32,
        "disposition": "Bank", "bank": "Nearest",
        "allowTeleports": false, "allowWilderness": false,
    })
    .as_object()
    .expect("settings object")
    .clone()
}

#[derive(Default)]
struct Observations {
    snapshot: GameSnapshot,
    tick: Option<u32>,
    frames: Vec<serde_json::Value>,
    bank_opened: bool,
    sharks_banked: bool,
    box_changed: bool,
    harpoon_lost: bool,
}

fn correct_outcome(
    observations: &Observations,
    phase: NativePhase,
    failure: Option<(&str, &str)>,
) -> bool {
    observations.bank_opened
        && !observations.box_changed
        && !observations.harpoon_lost
        && phase == NativePhase::Blocked
        && failure.is_some_and(|(code, message)| {
            code == "bank-deposit-failed" && message.contains("bank transfer did not settle")
        })
}

#[test]
fn held_box_oracle_requires_real_bank_progress_and_a_visible_failure() {
    let mut observations = Observations {
        bank_opened: true,
        ..Observations::default()
    };
    let failure = Some(("bank-deposit-failed", "bank transfer did not settle"));
    assert!(correct_outcome(
        &observations,
        NativePhase::Blocked,
        failure
    ));
    assert!(!correct_outcome(&observations, NativePhase::Waiting, None));
    assert!(!correct_outcome(&observations, NativePhase::Blocked, None));
    assert!(!correct_outcome(
        &observations,
        NativePhase::Blocked,
        Some(("unrelated", "bank transfer did not settle"))
    ));
    assert!(!correct_outcome(
        &observations,
        NativePhase::Blocked,
        Some(("bank-deposit-failed", ""))
    ));
    observations.bank_opened = false;
    assert!(!correct_outcome(
        &observations,
        NativePhase::Blocked,
        failure
    ));
    observations.bank_opened = true;
    observations.box_changed = true;
    assert!(!correct_outcome(
        &observations,
        NativePhase::Blocked,
        failure
    ));
    observations.box_changed = false;
    observations.harpoon_lost = true;
    assert!(!correct_outcome(
        &observations,
        NativePhase::Blocked,
        failure
    ));
}

#[test]
#[ignore = "requires LIVE=1, own BOT_LIVE_NAME_PREFIX (1-4 bytes), Engine A and Gatherer nav/engine/catalog/cache/evidence paths"]
fn live_gatherer_disabled_events_strange_boxes_engine_a() {
    assert_eq!(std::env::var("LIVE").as_deref(), Ok("1"));
    api::hostlog::set_debug(true);
    let account = super::mint_live_names(1).pop().expect("one account");
    let epoch = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let root = PathBuf::from(std::env::var_os("LIVE_EVIDENCE_DIR").expect("evidence root"));
    let evidence_dir = root.join(format!("gatherer-held-{account}-{epoch}"));
    let scratch = evidence_dir.join("scratch");
    std::fs::create_dir_all(&scratch).unwrap();
    let profile = live_profile(&scratch)
        .unwrap()
        .resolve(None)
        .unwrap()
        .bind()
        .unwrap();
    assert_eq!(profile.client().game_host(), "127.0.0.1");
    assert_eq!(profile.client().game_port(), 44594, "Engine A only");
    let bag = settings();
    let cell = Arc::new(Mutex::new(Cell::new(
        BOUND,
        SEED,
        |snapshot| {
            snapshot.tile() == Some((2840, 3436, 0))
                && count(snapshot.inventory(), HARPOON) == 1
                && count(snapshot.inventory(), BOX) == 28
                && count(snapshot.inventory(), SHARK) == 26
        },
        &[HARPOON, BOX, SHARK],
        serde_json::json!({"request": "GATHERER-HELD-FOREVER", "seed": SEED, "settings": bag}),
        |_| true,
        evidence_dir.clone(),
    )));
    let observations = Arc::new(Mutex::new(Observations::default()));
    let frame_cell = Arc::clone(&cell);
    let frame_observations = Arc::clone(&observations);
    let frame_account = account.clone();
    let template = SharedClientTemplate::load(Arc::clone(&profile)).unwrap();
    let play = run_with_template(
        template,
        true,
        vec![Profile {
            username: account.clone(),
            password: account.clone().into(),
            uid: 274_279_231,
            settings: ProfileSettings {
                random_events: false,
                ..ProfileSettings::default()
            },
        }],
        |_| (None, None),
        move |client, username, fr| {
            if username != frame_account {
                return;
            }
            frame(client, &frame_cell, &frame_account);
            let Some(started) = frame_cell.lock().started else {
                return;
            };
            let mut obs = frame_observations.lock();
            obs.snapshot.rebuild(client);
            let tick = obs.snapshot.tick();
            if obs.tick == Some(tick) {
                return;
            }
            obs.tick = Some(tick);
            let bank_open = obs.snapshot.bank_component_id() >= 0;
            let boxes = count(obs.snapshot.inventory(), BOX);
            let harpoons = count(obs.snapshot.inventory(), HARPOON);
            let sharks = count(obs.snapshot.inventory(), SHARK);
            let bank_sharks = count(obs.snapshot.bank(), SHARK);
            let row = serde_json::json!({
                "tick": tick, "tile": obs.snapshot.tile(), "host_hold": fr.hold,
                "elapsed_ms": started.elapsed().as_millis(),
                "bank_open": bank_open, "boxes": boxes, "harpoons": harpoons,
                "sharks": sharks, "bank_sharks": bank_sharks,
            });
            obs.frames.push(row);
            obs.bank_opened |= bank_open;
            obs.sharks_banked |= bank_sharks >= 26;
            obs.box_changed |= boxes != 28;
            obs.harpoon_lost |= harpoons != 1;
        },
    )
    .unwrap();
    let preparation_started = cell.lock().preparation_started;
    let mut login_ready = false;
    let mut started = false;
    let mut statuses = Vec::new();
    let mut previous = serde_json::Value::Null;
    let passed = loop {
        check_prerequisites(
            &play,
            &account,
            preparation_started,
            &mut login_ready,
            &cell,
        );
        if !started
            && cell.lock().phase == Prep::Ready
            && cell.lock().last_action.elapsed() >= Duration::from_secs(1)
        {
            play.script_start(&account, script::CompiledId("Gatherer"), bag.clone())
                .unwrap();
            started = true;
        }
        if started {
            if let Some(status) = play.script_native_status(&account) {
                let row = serde_json::json!({
                    "phase": format!("{:?}", status.phase), "event": text(&status, "last_event"),
                    "failure": status.failure.as_ref().map(|f| (&f.code, &f.message)),
                });
                if row != previous {
                    statuses.push(row.clone());
                    previous = row;
                }
                if correct_outcome(
                    &observations.lock(),
                    status.phase,
                    status
                        .failure
                        .as_ref()
                        .map(|f| (f.code.as_ref(), f.message.as_ref())),
                ) {
                    break true;
                }
            }
        }
        if cell
            .lock()
            .started
            .is_some_and(|start| start.elapsed() >= BOUND)
        {
            break false;
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    play.script_stop(&account);
    {
        let mut cell = cell.lock();
        cell.trace.passed = passed;
        if !passed {
            cell.fail("disabled event prevented real bank progress before the deadline".into());
        }
        cell.terminal = true;
    }
    let capture_deadline = Instant::now() + CAPTURE_GRACE;
    while !cell.lock().capture_written && Instant::now() < capture_deadline {
        std::thread::sleep(Duration::from_millis(100));
    }
    let obs = observations.lock();
    std::fs::write(
        evidence_dir.join("receipt.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "request": "GATHERER-HELD-FOREVER", "account": account, "engine": "127.0.0.1:44594",
            "seed": SEED, "settings": bag, "random_events_enabled": false,
            "passed": passed, "bank_opened": obs.bank_opened, "sharks_banked": obs.sharks_banked,
            "box_changed": obs.box_changed, "harpoon_lost": obs.harpoon_lost,
            "statuses": statuses, "frames": obs.frames,
        }))
        .unwrap(),
    )
    .unwrap();
    assert!(cell.lock().capture_written, "final capture did not finish");
    assert!(cell.lock().capture_error.is_none(), "final capture failed");
    assert!(
        passed,
        "disabled event did not reach its visible bank failure; receipt={}",
        evidence_dir.display()
    );
}
