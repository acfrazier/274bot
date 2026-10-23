//! Real runner-owned scratch-engine Grinch evidence capture.
//!
//! These ignored tests require the parent orchestration contract described in
//! `GRINCH-1020/design.md` section 6. They never start or stop an engine. Each
//! run records its typed trace and oracle receipt below `LIVE_EVIDENCE_DIR`.

#[path = "support/grinch_driver.rs"]
mod grinch_driver;
#[path = "support/grinch_oracle.rs"]
mod grinch_oracle;

use grinch_driver::Case;

fn capture(case: Case) {
    grinch_driver::capture_live(case).unwrap_or_else(|error| {
        panic!(
            "Grinch live evidence capture failed for {}: {error}",
            case.slug()
        )
    });
}

#[test]
#[ignore = "requires LIVE=1, an owned scratch engine, R289 assets, and a fresh evidence directory"]
fn grinch_a_tradeable_drops_live() {
    capture(Case::ATradeableDrops);
}

#[test]
#[ignore = "requires LIVE=1, an owned scratch engine, R289 assets, and a fresh evidence directory"]
fn grinch_a_green_mask_selection_live() {
    capture(Case::AGreenMaskSelection);
}

#[test]
#[ignore = "requires LIVE=1, an owned scratch engine, R289 assets, and a fresh evidence directory"]
fn grinch_a_negative_candidates_live() {
    capture(Case::ANegativeCandidates);
}

#[test]
#[ignore = "requires LIVE=1, an owned scratch engine, R289 assets, and a fresh evidence directory"]
fn grinch_a_private_untradeable_drops_live() {
    capture(Case::APrivateUntradeableDrops);
}

#[test]
#[ignore = "requires LIVE=1 and the running runner-owned scratch engine"]
fn grinch_b_pumpkin_respawns_live() {
    capture(Case::BPumpkinRespawns);
}

#[test]
#[ignore = "requires LIVE=1 and the running runner-owned scratch engine"]
fn grinch_b_contested_pumpkin_live() {
    capture(Case::BContestedPumpkin);
}

#[test]
#[ignore = "requires LIVE=1 and the running runner-owned scratch engine"]
fn grinch_b_scythe_duplicate_live() {
    capture(Case::BScytheDuplicate);
}

#[test]
#[ignore = "requires LIVE=1 and the running runner-owned scratch engine"]
fn grinch_b_bunny_ears_duplicate_live() {
    capture(Case::BBunnyEarsDuplicate);
}

#[test]
#[ignore = "requires LIVE=1 and the running runner-owned scratch engine"]
fn grinch_b_yoyo_repeat_live() {
    capture(Case::BYoYoRepeat);
}

#[test]
#[ignore = "requires LIVE=1, an owned scratch engine, R289 assets, and a fresh evidence directory"]
fn grinch_c_deposit_28_live() {
    capture(Case::CDeposit28);
}

#[test]
#[ignore = "requires LIVE=1, an owned scratch engine, R289 assets, and a fresh evidence directory"]
fn grinch_c_deposit_27_preserve_item_live() {
    capture(Case::CDeposit27PreserveItem);
}

#[test]
#[ignore = "requires LIVE=1, an owned scratch engine, R289 assets, and a fresh evidence directory"]
fn grinch_c_inventory_blocked_live() {
    capture(Case::CInventoryBlocked);
}

#[test]
#[ignore = "requires four captured A-variant scratch traces and verified process-tree stops"]
fn grinch_a_cell_oracle_live() {
    grinch_driver::replay_cell_a()
        .unwrap_or_else(|error| panic!("Grinch A replay failed: {error}"));
}

#[test]
#[ignore = "requires five captured scratch traces and verified full-process-tree stop evidence"]
fn grinch_b_cell_oracle_live() {
    grinch_driver::replay_cell_b()
        .unwrap_or_else(|error| panic!("Grinch B replay failed: {error}"));
}

#[test]
#[ignore = "requires three captured C-variant scratch traces and verified process-tree stops"]
fn grinch_c_cell_oracle_live() {
    grinch_driver::replay_cell_c()
        .unwrap_or_else(|error| panic!("Grinch C replay failed: {error}"));
}

/// Three real bank returns through the operator's region boundary. The fixture
/// gives only this account pumpkins and drops one through its inventory action.
/// It never creates a loc, NPC, or server spawn.
#[test]
#[ignore = "requires LIVE=1, Engine A, selected assets, and LIVE_EVIDENCE_DIR"]
fn grinch_three_bank_returns_cross_region_live() {
    return_boundary::capture();
}

mod return_boundary {
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    use api::interact::{ActionSpec, Interactions, OpTarget};
    use api::snapshot::GameSnapshot;
    use script::native::{ScriptStatus, StatusValue};
    use serde_json::{json, Value};

    const ANCHOR: (i32, i32, i32) = (2550, 3348, 0);
    const PUMPKIN: i32 = 1959;
    const DEADLINE: Duration = Duration::from_secs(1200);

    static LOGS: Mutex<Vec<Value>> = Mutex::new(Vec::new());
    static LOG_SINK: LogSink = LogSink;
    struct LogSink;

    impl api::hostlog::Sink for LogSink {
        fn record(&self, record: &api::hostlog::Record<'_>) {
            LOGS.lock().unwrap().push(json!({
                "slot": record.slot, "tick": record.tick,
                "source": format!("{:?}", record.source),
                "level": format!("{:?}", record.level), "message": record.message,
            }));
        }
    }

    enum Prep {
        Session,
        Reseed,
        Confirm(scenario::tutorial::PostRelogTutorial),
        Seed(usize),
        Drop,
        ObserveDrop,
        Ready,
    }

    struct Capture {
        snapshot: GameSnapshot,
        relog: scenario::ScenarioRunner,
        prep: Prep,
        last_tick: Option<u32>,
        last_scene: Option<i32>,
        frames: Vec<Value>,
        statuses: Vec<Value>,
        seeds: Vec<Value>,
        returning: bool,
        banked: i64,
        picked: i64,
        refills: i64,
        drop_pending: bool,
        failure: Option<String>,
    }

    fn integer(status: &ScriptStatus, key: &str) -> i64 {
        status
            .fields
            .iter()
            .find_map(|field| match field.value {
                StatusValue::Integer(value) if field.key == key => Some(value),
                _ => None,
            })
            .unwrap_or(0)
    }

    fn text<'a>(status: &'a ScriptStatus, key: &str) -> &'a str {
        status
            .fields
            .iter()
            .find_map(|field| match &field.value {
                StatusValue::Text(value) if field.key == key => Some(value.as_ref()),
                _ => None,
            })
            .unwrap_or("")
    }

    fn held(snapshot: &GameSnapshot) -> i32 {
        snapshot
            .inventory()
            .iter()
            .filter(|row| row.def.id == PUMPKIN)
            .map(|row| row.count)
            .sum()
    }

    impl Capture {
        fn frame(&mut self, client: &mut client::client::Client, hold: bool) {
            self.snapshot.rebuild(client);
            let tick = self.snapshot.tick();
            let scene = self.snapshot.scene_state();
            if self.last_tick != Some(tick) || self.last_scene != Some(scene) {
                self.frames.push(json!({
                    "tick": tick, "player_info": client.gens.player_info,
                    "scene": scene, "base": [client.map_build_base_x, client.map_build_base_z],
                    "server": self.snapshot.tile(),
                    "render": self.snapshot.local_player().map(|p| p.player.actor.tile),
                    "energy": self.snapshot.runenergy(), "held": held(&self.snapshot),
                    "bank": self.snapshot.bank().iter().filter(|row| row.def.id == PUMPKIN).map(|row| row.count).sum::<i32>(),
                    "returning": self.returning, "banked": self.banked, "picked": self.picked,
                }));
            }
            self.last_scene = Some(scene);
            if matches!(self.prep, Prep::Session) {
                self.relog.tick_with_hold(client, hold);
                match self.relog.status() {
                    scenario::RunnerStatus::Passed => self.prep = Prep::Reseed,
                    scenario::RunnerStatus::Failed(error) => self.failure = Some(error.to_string()),
                    _ => {}
                }
                return;
            }
            if hold || !client.ingame || scene != 2 || self.last_tick == Some(tick) {
                return;
            }
            self.last_tick = Some(tick);
            match &mut self.prep {
                Prep::Reseed => {
                    let confirm = scenario::tutorial::PostRelogTutorial::new(
                        scenario::tutorial::chat_baseline(&self.snapshot),
                    );
                    confirm.send_reseed(client);
                    self.prep = Prep::Confirm(confirm);
                }
                Prep::Confirm(confirm) => match confirm.check(&self.snapshot) {
                    Ok(true) => self.prep = Prep::Seed(0),
                    Ok(false) => {}
                    Err(error) => self.failure = Some(error),
                },
                Prep::Seed(index) => {
                    let commands = ["tele 0,39,52,54,20", "~clearinv", "give pumpkin 28"];
                    if let Some(command) = commands.get(*index) {
                        let sent = api::interact::cheat(client, command).is_sent();
                        self.seeds
                            .push(json!({"tick": tick, "command": command, "sent": sent}));
                        if sent {
                            *index += 1;
                        }
                    } else if self.snapshot.tile() == Some(ANCHOR) && held(&self.snapshot) == 28 {
                        self.prep = Prep::Drop;
                    }
                }
                Prep::Drop => {
                    if self.drop_one(client) {
                        self.prep = Prep::ObserveDrop;
                    }
                }
                Prep::ObserveDrop => {
                    if held(&self.snapshot) == 27
                        && self
                            .snapshot
                            .ground_items()
                            .iter()
                            .any(|row| row.def.id == PUMPKIN)
                    {
                        self.prep = Prep::Ready;
                    }
                }
                Prep::Ready => {
                    // Refill only during an already active return, before it
                    // reaches Start. The script remains running throughout.
                    if self.returning
                        && self.refills < 2
                        && self.banked >= (self.refills + 1) * 28
                        && held(&self.snapshot) == 0
                        && self.snapshot.tile().is_some_and(|(x, z, l)| {
                            l == 0 && (x - ANCHOR.0).abs().max((z - ANCHOR.1).abs()) <= 12
                        })
                    {
                        let sent = api::interact::cheat(client, "give pumpkin 28").is_sent();
                        self.seeds.push(
                            json!({"tick": tick, "command": "give pumpkin 28", "sent": sent}),
                        );
                        if sent {
                            self.refills += 1;
                            self.drop_pending = true;
                        }
                    } else if self.drop_pending
                        && held(&self.snapshot) == 28
                        && self.drop_one(client)
                    {
                        self.drop_pending = false;
                    }
                }
                Prep::Session => {}
            }
        }

        fn drop_one(&mut self, client: &mut client::client::Client) -> bool {
            let Some(item) = self
                .snapshot
                .inventory()
                .iter()
                .find(|row| row.def.id == PUMPKIN)
            else {
                return false;
            };
            let Some(label) = item
                .actions
                .iter()
                .flatten()
                .find(|label| label.eq_ignore_ascii_case("Drop"))
                .cloned()
            else {
                return false;
            };
            let sent = matches!(
                Interactions::new(&self.snapshot, client)
                    .interact(OpTarget::Item(item), ActionSpec::Label(label)),
                api::interact::SendResult::Sent { .. }
            );
            self.seeds.push(json!({"tick": self.snapshot.tick(), "action": "Drop", "tile": self.snapshot.tile(), "sent": sent}));
            sent
        }
    }

    pub(super) fn capture() {
        assert_eq!(std::env::var("LIVE").as_deref(), Ok("1"));
        api::hostlog::set_debug(true);
        assert!(api::hostlog::install_sink(&LOG_SINK));
        let root =
            PathBuf::from(std::env::var_os("LIVE_EVIDENCE_DIR").expect("evidence directory"));
        std::fs::create_dir_all(&root).unwrap();
        let account = host_play::mint_live_names(1).pop().unwrap();
        let path = |key| PathBuf::from(std::env::var_os(key).expect(key));
        let options = host_play::ProfileOptions {
            profile: Some("local-289".into()),
            revision: Some("289".into()),
            host: Some("127.0.0.1".into()),
            asset_host: Some("127.0.0.1".into()),
            port: Some(44594),
            http_port: Some(1080),
            nav_pack: Some(path("NAV_PACK")),
            engine_dir: Some(path("ENGINE_DIR")),
            catalog_root: Some(path("CATALOG_ROOT")),
            cache_dir: Some(path("BOT_CACHE_DIR")),
            unpack_dir: Some(path("CLIENT_UNPACK_DIR")),
            vault_path: Some(root.join("vault")),
            ..Default::default()
        };
        let profile = options
            .resolve(None)
            .and_then(|profile| profile.bind())
            .unwrap();
        let template = host_play::SharedClientTemplate::load(profile).unwrap();
        let state = Arc::new(Mutex::new(Capture {
            snapshot: GameSnapshot::new(),
            relog: super::grinch_driver::fresh_account_relogger(),
            prep: Prep::Session,
            last_tick: None,
            last_scene: None,
            frames: Vec::new(),
            statuses: Vec::new(),
            seeds: Vec::new(),
            returning: false,
            banked: 0,
            picked: 0,
            refills: 0,
            drop_pending: false,
            failure: None,
        }));
        let hook = Arc::clone(&state);
        let mut play = host_play::run_with_template(
            template,
            true,
            vec![vault::Profile {
                username: account.clone(),
                password: account.clone().into(),
                uid: 274_1023,
                settings: Default::default(),
            }],
            |_| (None, None),
            move |client, _, input| hook.lock().unwrap().frame(client, input.hold),
        )
        .unwrap();
        let deadline = Instant::now() + DEADLINE;
        let mut started = false;
        let mut run = None;
        let mut last_status = String::new();
        let mut arrived = false;
        while Instant::now() < deadline {
            let ready = matches!(state.lock().unwrap().prep, Prep::Ready);
            if ready && !started {
                play.script_start(
                    &account,
                    script::CompiledId("Grinch"),
                    serde_json::from_value(json!({"items":["1959"], "radius":12})).unwrap(),
                )
                .unwrap();
                started = true;
            }
            if let Some(status) = play.script_native_status(&account) {
                let mut state = state.lock().unwrap();
                let current_run = format!("{:?}", status.run);
                if let Some(run) = &run {
                    assert_eq!(run, &current_run, "the fixture must not restart Grinch");
                }
                run = Some(current_run);
                state.returning = text(&status, "status") == "returning to start";
                state.banked = integer(&status, "banked");
                state.picked = integer(&status, "picked");
                let current_status = format!("{status:?}");
                if current_status != last_status {
                    let tick = state.snapshot.tick();
                    state
                        .statuses
                        .push(json!({"tick":tick, "status":current_status}));
                    last_status = format!("{status:?}");
                }
                if let Some(failure) = &status.failure {
                    state.failure = Some(format!("{failure:?}"));
                }
                arrived = state.banked == 84
                    && state.picked == 3
                    && !state.returning
                    && state.snapshot.tile() == Some(ANCHOR)
                    && held(&state.snapshot) == 0;
            }
            if arrived || state.lock().unwrap().failure.is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        play.stop_slot(&account);
        let state = state.lock().unwrap();
        let rebuild_on_return = state
            .frames
            .iter()
            .any(|frame| frame["returning"] == true && frame["scene"] == 1);
        let crossed_boundary = state.frames.iter().any(|frame| {
            frame["returning"] == true && frame["server"][0].as_i64().is_some_and(|x| x >= 2560)
        }) && state.frames.iter().any(|frame| {
            frame["returning"] == true && frame["server"][0].as_i64().is_some_and(|x| x < 2560)
        });
        let passed = arrived && rebuild_on_return && crossed_boundary && state.failure.is_none();
        let receipt = json!({
            "request":"GRINCH-RETURN-STALL-1023", "passed":passed, "account":account,
            "engine":"127.0.0.1:44594", "deadline_seconds":DEADLINE.as_secs(),
            "anchor":ANCHOR, "run":run, "banked":state.banked, "picked":state.picked,
            "arrived":arrived, "rebuild_on_return":rebuild_on_return, "crossed_boundary":crossed_boundary,
            "failure":state.failure, "frames":state.frames, "statuses":state.statuses, "seeds":state.seeds,
            "logs": *LOGS.lock().unwrap(),
        });
        std::fs::write(
            root.join("receipt.json"),
            serde_json::to_vec_pretty(&receipt).unwrap(),
        )
        .unwrap();
        assert!(
            passed,
            "Grinch return proof failed: {}",
            root.join("receipt.json").display()
        );
    }
}
