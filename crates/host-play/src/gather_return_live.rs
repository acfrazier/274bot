//! Live proof that a Draynor small-net cooldown no longer turns into a
//! permanent `return-failed` block.
//!
//! The diagnosis cell teleported the account to the bank until Gatherer
//! observed a stall, then filled the pack so a real deposit/close/return
//! ran while both small-net placements were still excluded. Unfixed code
//! blocked with `return-failed` and stayed blocked past the deadline.
//! This cell uses the same trigger and requires a wait, a return, and a
//! later catch.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use api::interact;
use api::snapshot::{GameSnapshot, WorldTile};
use parking_lot::Mutex;
use script::native::{NativePhase, ScriptStatus, StatusValue};
use vault::{Profile, ProfileSettings};

use super::bank_core_live::{check_prerequisites, frame, live_profile, Cell, Prep, CAPTURE_GRACE};
use super::{run_with_template, SharedClientTemplate};

const NET: i32 = 303;
const SHRIMP: i32 = 317;
const ANCHOVY: i32 = 321;
const SPOTS: WorldTile = WorldTile {
    x: 3086,
    z: 3229,
    level: 0,
};
const BANK: WorldTile = WorldTile {
    x: 3092,
    z: 3243,
    level: 0,
};
const CELL_BOUND: Duration = Duration::from_secs(300);
const SEED: &[&str] = &[
    "setstat fishing 20",
    "setstat defence 99",
    "setstat hitpoints 99",
    "give net 1",
    "givebank net 1",
    "give raw_shrimp 26",
    "tele 0,48,50,14,29",
];

fn integer(status: &ScriptStatus, key: &str) -> i64 {
    status
        .fields
        .iter()
        .find(|field| field.key == key)
        .and_then(|field| match field.value {
            StatusValue::Integer(value) => Some(value),
            _ => None,
        })
        .unwrap_or(0)
}

fn text(status: &ScriptStatus, key: &str) -> String {
    status
        .fields
        .iter()
        .find(|field| field.key == key)
        .and_then(|field| match &field.value {
            StatusValue::Text(value) => Some(value.to_string()),
            _ => None,
        })
        .unwrap_or_default()
}

fn settings() -> serde_json::Map<String, serde_json::Value> {
    let serde_json::Value::Object(bag) = serde_json::json!({
        "skill": "Fishing",
        "fishingMethod": "fishing.saltfish.op1",
        "location": "Custom",
        "customTile": {"x": SPOTS.x, "z": SPOTS.z, "level": SPOTS.level},
        "radius": 12,
        "disposition": "Bank",
        "bank": "Nearest",
        "deathPolicy": "Recover",
        "maxDeaths": 2,
        "allowTeleports": false,
        "allowWilderness": false,
    }) else {
        unreachable!("a JSON object literal");
    };
    bag
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

#[test]
#[ignore = "requires LIVE=1, GATHERER_NAV_PACK/GATHERER_ENGINE_DIR/GATHERER_CATALOG_ROOT/BOT_CACHE_DIR/CLIENT_UNPACK_DIR/LIVE_EVIDENCE_DIR and a local 289 engine"]
fn live_draynor_small_net_waits_out_exclusion_then_catches() {
    assert_eq!(std::env::var("LIVE").as_deref(), Ok("1"), "requires LIVE=1");
    let root = PathBuf::from(
        std::env::var_os("LIVE_EVIDENCE_DIR").expect("LIVE_EVIDENCE_DIR names the evidence root"),
    );
    let account = super::mint_live_names(1).pop().expect("one live account");
    let epoch = unix_now();
    let evidence_dir = root.join(format!("l-gather-return_{account}_utc-{epoch}Z"));
    let scratch = evidence_dir.join("scratch");
    std::fs::create_dir_all(&scratch).expect("create evidence scratch");
    let options = live_profile(&scratch).expect("live profile options");
    let profile = options
        .resolve(None)
        .and_then(|resolved| resolved.bind())
        .expect("bind the local 289 profile");
    assert_eq!(
        profile.client().game_host(),
        "127.0.0.1",
        "loopback engine only"
    );
    let bag = settings();
    let cell = Arc::new(Mutex::new(Cell::new(
        CELL_BOUND,
        SEED,
        |snapshot| {
            snapshot.tile() == Some((SPOTS.x, SPOTS.z, SPOTS.level))
                && snapshot.inventory().iter().any(|item| item.def.id == NET)
        },
        &[NET, SHRIMP, ANCHOVY],
        serde_json::json!({
            "request": "LEDGER-0202-GATHER",
            "settings": bag,
            "seed": SEED,
        }),
        |_| true,
        evidence_dir.clone(),
    )));
    let frame_cell = Arc::clone(&cell);
    let frame_account = account.clone();
    let fill_requested = Arc::new(Mutex::new(false));
    let frame_fill = Arc::clone(&fill_requested);
    let fill_sent = Arc::new(Mutex::new(false));
    let frame_fill_sent = Arc::clone(&fill_sent);
    let last_fault_tick = Mutex::new(0);
    let stall_requested = Arc::new(Mutex::new(false));
    let frame_stall = Arc::clone(&stall_requested);
    let snapshot = Mutex::new(GameSnapshot::new());
    let last_tick = Mutex::new(0);
    let template = SharedClientTemplate::load(Arc::clone(&profile)).expect("client template");
    let play = run_with_template(
        template,
        true,
        vec![Profile {
            username: account.clone(),
            password: account.clone().into(),
            uid: 274_279_230,
            settings: ProfileSettings::default(),
        }],
        |_| (None, None),
        move |client, username, fr| {
            if username != frame_account {
                return;
            }
            frame(client, &frame_cell, &frame_account);
            if client.ingame && client.scene_state == 2 {
                let mut snapshot = snapshot.lock();
                let mut last_tick = last_tick.lock();
                snapshot.rebuild(client);
                if snapshot.tick() != *last_tick {
                    *last_tick = snapshot.tick();
                }
                if !fr.hold && *frame_stall.lock() {
                    if !*frame_fill.lock() {
                        let mut last_fault_tick = last_fault_tick.lock();
                        if *last_fault_tick != snapshot.tick() {
                            *last_fault_tick = snapshot.tick();
                            let _ =
                                interact::cheat(client, &interact::tele_args(0, BANK.x, BANK.z));
                        }
                    } else if !*frame_fill_sent.lock() {
                        let sent = interact::cheat(client, "give raw_shrimp 27").is_sent();
                        if sent {
                            *frame_fill_sent.lock() = true;
                        }
                    }
                }
            }
        },
    )
    .unwrap_or_else(|error| {
        panic!("HARNESS PREREQUISITE FAILURE (before the cell deadline): starting login: {error}")
    });
    let preparation_started = cell.lock().preparation_started;
    let mut login_ready = false;
    let mut started = false;
    let mut last_status = String::new();
    let mut receipts = Vec::new();
    let mut saw_exclusion_wait = false;
    let mut yielded_at_exclusion = None;
    let deadline = Instant::now() + Duration::from_secs(480);
    let outcome = loop {
        check_prerequisites(
            &play,
            &account,
            preparation_started,
            &mut login_ready,
            &cell,
        );
        assert!(
            Instant::now() < deadline,
            "Draynor return cell deadline; last={last_status}"
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
                let event = text(&status, "last_event");
                let receipt = serde_json::json!({
                    "phase": format!("{:?}", status.phase),
                    "event": event,
                    "method": text(&status, "method"),
                    "area": text(&status, "area"),
                    "target": text(&status, "target"),
                    "trips": integer(&status, "trips"),
                    "yielded": integer(&status, "yielded"),
                    "failure": status.failure.as_ref().map(|failure| {
                        (failure.code.to_string(), failure.message.to_string())
                    }),
                });
                let value = receipt.to_string();
                if value != last_status {
                    receipts.push(receipt);
                    last_status = value;
                }
                if event.contains("waiting for excluded resource until") {
                    saw_exclusion_wait = true;
                    if yielded_at_exclusion.is_none() {
                        yielded_at_exclusion = Some(integer(&status, "yielded"));
                    }
                }
                if status
                    .failure
                    .as_ref()
                    .is_some_and(|failure| failure.code.as_ref() == "return-failed")
                {
                    play.script_stop(&account);
                    panic!(
                        "temporary exclusion became return-failed: {:?}",
                        status.failure
                    );
                }
                if integer(&status, "trips") >= 1 && integer(&status, "yielded") >= 2 {
                    *stall_requested.lock() = true;
                }
                if *stall_requested.lock() && status.phase == NativePhase::Waiting {
                    *fill_requested.lock() = true;
                }
                if saw_exclusion_wait {
                    let yielded = integer(&status, "yielded");
                    let trips = integer(&status, "trips");
                    if event.contains("gathering")
                        && yielded_at_exclusion.is_some_and(|before| yielded > before)
                    {
                        break serde_json::json!({
                            "observed": "returned-and-caught",
                            "saw_exclusion_wait": true,
                            "yielded": yielded,
                            "trips": trips,
                            "event": event,
                        });
                    }
                }
            }
        }
        if cell
            .lock()
            .started
            .is_some_and(|started| started.elapsed() >= CELL_BOUND + CAPTURE_GRACE)
        {
            panic!("cell bound elapsed without a post-exclusion catch; last={last_status}");
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    play.script_stop(&account);
    std::fs::write(
        evidence_dir.join("receipt.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "request": "LEDGER-0202-GATHER",
            "account": account,
            "outcome": outcome.clone(),
            "saw_exclusion_wait": saw_exclusion_wait,
            "statuses": receipts,
            "settings": bag,
            "seed": SEED,
        }))
        .expect("receipt json"),
    )
    .expect("write receipt");
    assert_eq!(
        outcome.get("observed").and_then(|value| value.as_str()),
        Some("returned-and-caught"),
        "the fixed return must catch again after the exclusion wait: {outcome}"
    );
}
