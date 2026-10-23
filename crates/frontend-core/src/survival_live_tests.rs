//! Real-content proof for the saved Cure poison setting and the native Walk guard.
//!
//! The fixture uses a full 10/10 hitpoint row at poison onset. The guard cannot
//! eat at full HP while the profile is still `Never`; Engine A's first six-point
//! poison fire would leave 4 HP at the quiet-route eat line of 7, so the save,
//! mirror and cure must all arrive before the first fire. The 30-tick content
//! phase is justified by `host-play/src/slot_condition.rs:26-29`.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::task::Poll;
use std::time::{Duration, Instant, SystemTime};

use api::condition::{PoisonState, SessionMark};
use api::interact::{cheat, tele_args};
use api::snapshot::{GameSnapshot, WorldTile};
use host::Pump;
use host_play::{
    mint_live_entries, mint_live_names, run_with_template, InstancePermit, ProfileOptions,
    SharedClientTemplate, SlotArm,
};
use parking_lot::Mutex;
use scenario::{Proof, RunnerStatus, ScenarioRunner, Step, StepKind, Wait};
use script::native::walk::Walk;
use script::native::{
    ActionHandle, NativeTick, Script, ScriptFailure, ScriptFlow, WalkAllow, WalkEnd, WalkRequest,
};
use script::survival::CurePoison as ScriptCurePoison;
use serde_json::{json, Value};
use vault::{CurePoison as VaultCurePoison, Profile, ProfileSettings, RasterMode, Vault};

use crate::{
    CurePoison as DraftCurePoison, HeadlessSurface, OperatorCommand, OperatorSession, SurvivalEdit,
    Targets,
};

const EVIDENCE_ENV: &str = "LIVE_EVIDENCE_DIR";
const VAULT_PASSPHRASE: &str = "survival-live-pass-01";
const ANTIPOISON_ID: i32 = 179;
const LOBSTER_ID: i32 = 379;
const POISON_TICKS_TO_FIRST_FIRE: u32 = 30;
const FIRST_POISON_DAMAGE: u8 = 6;
const QUIET_BASE_EAT_LINE: u8 = 1;
const SETUP_DEADLINE: Duration = Duration::from_secs(240);
const CELL_DEADLINE: Duration = Duration::from_secs(20 * 60);
const CELL_TICK_LIMIT: u32 = 600;
const POLL_INTERVAL: Duration = Duration::from_millis(20);
const DRINK_LINE: &str = "You drink some of your antipoison potion.";

const WALK_ORIGIN: WorldTile = WorldTile {
    x: 3185,
    z: 3440,
    level: 0,
};
const WALK_TARGET: WorldTile = WorldTile {
    x: 3017,
    z: 3170,
    level: 0,
};

#[derive(Default)]
struct Trace {
    account: Option<String>,
    login_ticks: Vec<u32>,
    final_login_tick: Option<u32>,
    load_operation: Option<u64>,
    setup_passed: bool,
    script_run_retained: bool,
    script_start_error: Option<String>,
    native_never_frame: Option<u64>,
    native_never_tick: Option<u32>,
    native_when_needed_frame: Option<u64>,
    native_when_needed_tick: Option<u32>,
    arm_never_frame: Option<u64>,
    arm_when_needed_frame: Option<u64>,
    host_never_frame: Option<u64>,
    host_when_needed_frame: Option<u64>,
    host_policy_after_copy: bool,
    setting_operation: Option<u64>,
    setting_submit_tick: Option<u32>,
    setting_submit_frame: Option<u64>,
    setting_completed: bool,
    durable_policy: Option<String>,
    durable_frame: Option<u64>,
    durable_tick: Option<u32>,
    disk_policy: Option<String>,
    disk_error: Option<String>,
    walk_started_tick: Option<u32>,
    walk_started_frame: Option<u64>,
    walk_start_tile: Option<WorldTile>,
    walk_start_error: Option<String>,
    route_progress: u32,
    route_progress_at_onset: Option<u32>,
    seed_queued: bool,
    seed_tick: Option<u32>,
    seed_frame: Option<u64>,
    seed_sent: Option<bool>,
    seed_error: Option<String>,
    pre_seed_poison: Option<String>,
    seed_chat_sequence: Option<i32>,
    onset_tick: Option<u32>,
    onset_frame: Option<u64>,
    onset_since: Option<u16>,
    onset_p: Option<u8>,
    onset_hp: Option<u8>,
    onset_hp_max: Option<u8>,
    onset_food_count: Option<u8>,
    onset_dose_count: Option<u8>,
    onset_policy: Option<String>,
    onset_walk_pending: bool,
    first_mark_tick: Option<u32>,
    first_mark_damage: Option<u8>,
    raw_poison_marks: Vec<Value>,
    drink_line_tick: Option<u32>,
    dose_drop_tick: Option<u32>,
    immune_tick: Option<u32>,
    immune_until: Option<u16>,
    food_at_immune: Option<u8>,
    dose_at_immune: Option<u8>,
    food_drop_tick: Option<u32>,
    final_food_count: Option<i32>,
    death_tick: Option<u32>,
    walk_end: Option<String>,
    walk_arrived: bool,
    walk_receipt_tick: Option<u32>,
    walk_detail: Option<String>,
    events: Vec<Value>,
}

impl Trace {
    fn event(&mut self, kind: &str, frame: u64, tick: u32, detail: Value) {
        if self.events.len() < 512 {
            self.events.push(json!({
                "event": kind,
                "frame": frame,
                "tick": tick,
                "detail": detail,
            }));
        }
    }

    fn receipt(&self, failure: Option<&str>) -> Value {
        let projected = self.projected_first_mark_bite();
        json!({
            "scenario": "L-GUARD-CURE",
            "account": self.account,
            "outcome": if failure.is_none() { "PASS" } else { "FAIL" },
            "map_members": true,
            "failure": failure,
            "ticks": {
                "login_transitions": self.login_ticks,
                "final_login": self.final_login_tick,
                "seed": self.seed_tick,
                "seed_frame": self.seed_frame,
                "onset": self.onset_tick,
                "onset_since": self.onset_since,
                "first_mark": self.first_mark_tick,
                "drink_line": self.drink_line_tick,
                "dose_drop": self.dose_drop_tick,
                "immune": self.immune_tick,
                "death": self.death_tick,
                "walk_started": self.walk_started_tick,
                "walk_receipt": self.walk_receipt_tick,
                "setting_submit": self.setting_submit_tick,
                "setting_durable": self.durable_tick,
            },
            "fixture": {
                "walk_origin": tile_json(WALK_ORIGIN),
                "walk_target": tile_json(WALK_TARGET),
                "route_progress_at_onset": self.route_progress_at_onset,
                "route_progress_total": self.route_progress,
                "food_guard": true,
                "allow_food": true,
                "onset_hp": self.onset_hp,
                "onset_hp_max": self.onset_hp_max,
                "poison_p": self.onset_p,
                "food_at_onset": self.onset_food_count,
                "dose_at_onset": self.onset_dose_count,
                "pre_seed_poison": self.pre_seed_poison,
                "seed_sent": self.seed_sent,
                "seed_error": self.seed_error,
                "seed_queued": self.seed_queued,
                "seed_chat_sequence": self.seed_chat_sequence,
            },
            "counterfactual_first_mark": {
                "first_mark_damage": self.first_mark_damage,
                "damage": FIRST_POISON_DAMAGE,
                "hp_after": self.onset_hp.map(|hp| hp.saturating_sub(FIRST_POISON_DAMAGE)),
                "quiet_eat_line": QUIET_BASE_EAT_LINE + FIRST_POISON_DAMAGE,
                "food_would_be_due": projected,
                "source": "quiet base line 2*danger+1 at danger 0, plus poison fire; policy.rs and select.rs",
            },
            "mirror": {
                "load_operation": self.load_operation,
                "native_never_frame": self.native_never_frame,
                "native_never_tick": self.native_never_tick,
                "arm_never_frame": self.arm_never_frame,
                "host_never_frame": self.host_never_frame,
                "operation": self.setting_operation,
                "submit_frame": self.setting_submit_frame,
                "durable_frame": self.durable_frame,
                "setting_completed": self.setting_completed,
                "durable_policy": self.durable_policy,
                "disk_policy": self.disk_policy,
                "disk_error": self.disk_error,
                "arm_when_needed_frame": self.arm_when_needed_frame,
                "host_when_needed_frame": self.host_when_needed_frame,
                "host_policy_after_copy": self.host_policy_after_copy,
                "native_when_needed_tick": self.native_when_needed_tick,
                "native_when_needed_frame": self.native_when_needed_frame,
                "onset_policy": self.onset_policy,
                "walk_pending_at_onset": self.onset_walk_pending,
            },
            "cure": {
                "drink_line": self.drink_line_tick,
                "dose_drop": self.dose_drop_tick,
                "immune": self.immune_tick,
                "immune_until": self.immune_until,
                "food_at_immune": self.food_at_immune,
                "dose_at_immune": self.dose_at_immune,
                "food_drop": self.food_drop_tick,
                "food_at_finish": self.final_food_count,
                "raw_poison_marks": self.raw_poison_marks,
                "post_cure_bite": "not due on the quiet route after poison is cleared; no bite is required",
            },
            "walk": {
                "started_frame": self.walk_started_frame,
                "start_tile": self.walk_start_tile.map(tile_json),
                "start_error": self.walk_start_error,
                "end": self.walk_end,
                "arrived": self.walk_arrived,
                "receipt_tick": self.walk_receipt_tick,
                "detail": self.walk_detail,
            },
            "script": {
                "run_retained": self.script_run_retained,
                "start_error": self.script_start_error,
                "setup_passed": self.setup_passed,
            },
            "events": self.events,
        })
    }

    fn projected_first_mark_bite(&self) -> Option<bool> {
        let hp = self.onset_hp?;
        let hp_max = self.onset_hp_max?;
        let food = self.onset_food_count?;
        let after_fire = hp.saturating_sub(FIRST_POISON_DAMAGE);
        let eat_line = QUIET_BASE_EAT_LINE.saturating_add(FIRST_POISON_DAMAGE);
        Some(food > 0 && after_fire > 0 && after_fire < hp_max && after_fire <= eat_line)
    }
}

struct LiveState {
    runner: ScenarioRunner,
    snapshot: GameSnapshot,
    pump: Pump,
    frame_count: u64,
    was_ingame: bool,
    last_tile: Option<WorldTile>,
    last_hitmark_cycles: [i32; 4],
    arm: Option<Arc<SlotArm>>,
    action: Option<FixtureAction>,
    previous_food_count: Option<i32>,
    previous_dose_count: Option<i32>,
}

#[derive(Clone, Copy)]
enum FixtureAction {
    SeedPoison,
}

struct GuardWalkScript {
    state: Arc<Mutex<LiveState>>,
    trace: Arc<Mutex<Trace>>,
    begun: bool,
    handle: Option<ActionHandle<Walk>>,
}

impl Script for GuardWalkScript {
    fn tick(&mut self, tick: &mut NativeTick<'_>) -> Result<ScriptFlow, ScriptFailure> {
        let condition = tick.cx.condition().copied();
        let policy = tick.cx.cure_poison();
        let (state_frame, snapshot_tick) = {
            let state = self.state.lock();
            (state.frame_count, state.snapshot.tick())
        };

        {
            let mut trace = self.trace.lock();
            if policy == ScriptCurePoison::Never && trace.native_never_frame.is_none() {
                trace.native_never_frame = Some(state_frame);
                trace.native_never_tick = Some(snapshot_tick);
                trace.event(
                    "native-policy-never-observed",
                    state_frame,
                    snapshot_tick,
                    json!({ "policy": "never" }),
                );
            }
            if policy == ScriptCurePoison::WhenNeeded && trace.native_when_needed_frame.is_none() {
                trace.native_when_needed_frame = Some(state_frame);
                trace.native_when_needed_tick = Some(snapshot_tick);
                trace.event(
                    "native-policy-mirrored",
                    state_frame,
                    snapshot_tick,
                    json!({ "policy": "when_needed" }),
                );
            }
            if let Some(condition) = condition {
                if trace.seed_tick.is_none()
                    && matches!(
                        condition.poison,
                        PoisonState::Onset { .. }
                            | PoisonState::Poisoned { .. }
                            | PoisonState::Immune { .. }
                    )
                    && trace.pre_seed_poison.is_none()
                {
                    trace.pre_seed_poison = Some(format!("{:?}", condition.poison));
                }
                match condition.poison {
                    PoisonState::Onset { since, q, .. } => {
                        if trace.onset_tick.is_none() {
                            trace.onset_tick = Some(condition.tick);
                            trace.onset_frame = Some(state_frame);
                            trace.onset_since = Some(since);
                            trace.onset_hp = Some(condition.hp);
                            trace.onset_hp_max = Some(condition.hp_max);
                            trace.onset_food_count = Some(condition.food.ordinary_count);
                            trace.onset_dose_count = Some(condition.cures.antipoison);
                            trace.onset_policy = Some(script_policy_name(policy).to_owned());
                            trace.onset_walk_pending = self.handle.is_some();
                            trace.route_progress_at_onset = Some(trace.route_progress);
                            let route_progress = trace.route_progress;
                            trace.event(
                                "poison-onset",
                                state_frame,
                                condition.tick,
                                json!({
                                    "since": since,
                                    "p": q,
                                    "hp": condition.hp,
                                    "hp_max": condition.hp_max,
                                    "food": condition.food.ordinary_count,
                                    "antipoison": condition.cures.antipoison,
                                    "policy": script_policy_name(policy),
                                    "walk_pending": self.handle.is_some(),
                                    "route_progress": route_progress,
                                }),
                            );
                        }
                        if q.is_some() {
                            trace.onset_p = q;
                        }
                    }
                    PoisonState::Poisoned { per_tick, .. } => {
                        if trace.first_mark_tick.is_none() {
                            trace.first_mark_tick = Some(condition.tick);
                            trace.first_mark_damage = Some(per_tick);
                            trace.event(
                                "condition-poison-mark",
                                state_frame,
                                condition.tick,
                                json!({ "per_tick": per_tick }),
                            );
                        }
                    }
                    PoisonState::Immune { until_lower } => {
                        if trace.immune_tick.is_none() {
                            trace.immune_tick = Some(condition.tick);
                            trace.immune_until = Some(until_lower);
                            trace.food_at_immune = Some(condition.food.ordinary_count);
                            trace.dose_at_immune = Some(condition.cures.antipoison);
                            trace.event(
                                "poison-immune",
                                state_frame,
                                condition.tick,
                                json!({
                                    "until_lower": until_lower,
                                    "food": condition.food.ordinary_count,
                                    "antipoison": condition.cures.antipoison,
                                }),
                            );
                        }
                    }
                    PoisonState::Unknown { .. } | PoisonState::Clear => {}
                }
                if condition.session == SessionMark::Dead
                    || (condition.hp_known && condition.hp == 0)
                {
                    if trace.death_tick.is_none() {
                        trace.death_tick = Some(condition.tick);
                        trace.event(
                            "death",
                            state_frame,
                            condition.tick,
                            json!({ "hp": condition.hp, "session": format!("{:?}", condition.session) }),
                        );
                    }
                }
            }
        }

        if !self.begun {
            self.begun = true;
            let start_tile = tick
                .cx
                .snapshot()
                .local_player()
                .map(|player| player.value.player.network);
            let request = walk_request(tick.cx.evidence());
            match tick.actions.begin::<Walk>(request, &mut tick.cx) {
                Ok(handle) => {
                    self.handle = Some(handle);
                    let mut trace = self.trace.lock();
                    trace.walk_started_tick = Some(snapshot_tick);
                    trace.walk_started_frame = Some(state_frame);
                    trace.walk_start_tile = start_tile;
                    trace.event(
                        "native-walk-begun",
                        state_frame,
                        snapshot_tick,
                        json!({
                            "from": start_tile.map(tile_json),
                            "to": tile_json(WALK_TARGET),
                            "food_guard": true,
                            "allow_food": true,
                        }),
                    );
                }
                Err(error) => {
                    self.trace.lock().walk_start_error = Some(format!("{error:?}"));
                    return Ok(ScriptFlow::Complete);
                }
            }
        }

        if let Some(handle) = &self.handle {
            if let Poll::Ready(result) = tick.actions.poll(handle, &mut tick.cx) {
                let mut trace = self.trace.lock();
                match result {
                    Ok(receipt) => {
                        trace.walk_arrived = receipt.end == WalkEnd::Arrived;
                        trace.walk_end = Some(format!("{:?}", receipt.end));
                        trace.walk_receipt_tick = Some(snapshot_tick);
                        trace.walk_detail = receipt.detail.as_deref().map(str::to_owned);
                        trace.event(
                            "native-walk-receipt",
                            state_frame,
                            snapshot_tick,
                            json!({
                                "end": format!("{:?}", receipt.end),
                                "request_id": receipt.request_id,
                                "detail": receipt.detail.as_deref(),
                            }),
                        );
                    }
                    Err(error) => {
                        trace.walk_end = Some("ActionError".to_owned());
                        trace.walk_detail = Some(format!("{error:?}"));
                        trace.event(
                            "native-walk-error",
                            state_frame,
                            snapshot_tick,
                            json!({ "error": format!("{error:?}") }),
                        );
                    }
                }
                self.handle = None;
                return Ok(ScriptFlow::Complete);
            }
        }
        Ok(ScriptFlow::Continue)
    }
}

fn walk_request(required_after: api::quest_progress::EvidenceStamp) -> WalkRequest {
    WalkRequest {
        target: WALK_TARGET,
        candidate_goals: None,
        loc_id: None,
        radius: 12,
        arrival: nav::arrival::ArrivalKind::Reach,
        options: script::native::WalkOptions::default(),
        required_after,
        evidence: None,
        cross: Box::default(),
        protect: false,
        food_guard: true,
        allow: WalkAllow {
            prayer: true,
            food: true,
            escape: true,
        },
    }
}

#[test]
#[ignore = "requires LIVE=1, isohome, Engine A, BOT_LIVE_NAME_PREFIX, BOT_CACHE_DIR, ENGINE_DIR, NAV_PACK, and LIVE_EVIDENCE_DIR"]
fn live_frontend_setting_mirrors_into_native_walk_guard() {
    assert_eq!(std::env::var("LIVE").as_deref(), Ok("1"));
    assert_eq!(std::env::var("BOT_CPU").as_deref(), Ok("1"));

    let _home = script::IsolatedEnv::enter("survival-guard-cure-live");
    let evidence_root = required_path(EVIDENCE_ENV).expect("LIVE_EVIDENCE_DIR");
    std::fs::create_dir_all(&evidence_root).expect("create evidence directory");
    let account = mint_live_names(1)
        .into_iter()
        .next()
        .expect("BOT_LIVE_NAME_PREFIX must mint one account");
    let evidence = evidence_root.join(format!(
        "l-guard-cure_{}_{}",
        account,
        scenario::shot::stamp_utc(SystemTime::now())
    ));
    std::fs::create_dir_all(&evidence).expect("create cell evidence directory");
    let vault_path = evidence.join("vault.bin");
    let trace = Arc::new(Mutex::new(Trace {
        account: Some(account.clone()),
        ..Trace::default()
    }));

    let run = run_guard_cure_live(&account, &vault_path, Arc::clone(&trace));
    let disk_result = if vault_path.is_file() {
        match Vault::unlock(&vault_path, VAULT_PASSPHRASE) {
            Ok(vault) => {
                let policy = vault
                    .get(&account)
                    .map(|profile| vault_policy_name(profile.settings.survival.cure_poison));
                trace.lock().disk_policy = policy.map(str::to_owned);
                if policy == Some("when_needed") {
                    Ok(())
                } else {
                    Err(format!(
                        "vault disk policy is {policy:?}, expected when_needed"
                    ))
                }
            }
            Err(error) => {
                trace.lock().disk_error = Some(error.to_string());
                Err(format!("could not reopen saved vault: {error}"))
            }
        }
    } else {
        Err("the test did not create its evidence vault".to_owned())
    };
    let oracle_result = verify_guard_cure(&trace.lock());
    let failure = run
        .as_ref()
        .err()
        .or_else(|| disk_result.as_ref().err())
        .or_else(|| oracle_result.as_ref().err())
        .map(String::as_str);
    let receipt = trace.lock().receipt(failure);
    std::fs::write(
        evidence.join("receipt.json"),
        serde_json::to_vec_pretty(&receipt).expect("serialize receipt"),
    )
    .expect("write receipt");

    assert!(
        run.is_ok() && disk_result.is_ok() && oracle_result.is_ok(),
        "L-GUARD-CURE failed; receipt: {}\nrun: {:?}\ndisk: {:?}\noracle: {:?}",
        evidence.join("receipt.json").display(),
        run.err(),
        disk_result.err(),
        oracle_result.err(),
    );
}

fn run_guard_cure_live(
    account: &str,
    vault_path: &Path,
    trace: Arc<Mutex<Trace>>,
) -> Result<(), String> {
    let nav_pack = required_path("NAV_PACK")?;
    let nav_flags = std::env::var_os("NAV_FLAGS")
        .map(PathBuf::from)
        .unwrap_or_else(|| nav_pack.with_extension("navflags"));
    let profile = ProfileOptions {
        profile: Some("local-289".into()),
        revision: Some("289".into()),
        host: Some("127.0.0.1".into()),
        asset_host: Some("127.0.0.1".into()),
        port: Some(44594),
        http_port: Some(1080),
        engine_dir: Some(required_path("ENGINE_DIR")?),
        cache_dir: Some(required_path("BOT_CACHE_DIR")?),
        unpack_dir: Some(vault_path.with_extension("runtime-cache")),
        nav_pack: Some(nav_pack),
        nav_flags: Some(nav_flags),
        catalog_root: script::rs2b0t_root(),
        vault_path: Some(vault_path.to_path_buf()),
        ..ProfileOptions::default()
    }
    .resolve(None)
    .map_err(|error| format!("resolve local Engine A profile: {error}"))?
    .bind_runtime()
    .map_err(|error| format!("bind local Engine A profile: {error}"))?;
    if !profile.map_members() {
        return Err("fixture requires Engine A's members world".into());
    }

    let mut vault = Vault::create(vault_path, VAULT_PASSPHRASE)
        .map_err(|error| format!("create evidence vault: {error}"))?;
    let mut settings = ProfileSettings {
        auto_login: true,
        raster: RasterMode::Cpu,
        ..ProfileSettings::default()
    };
    settings.survival.cure_poison = VaultCurePoison::Never;
    let password = mint_live_entries(&[account.to_owned()])
        .into_iter()
        .next()
        .map(|(_, password)| password)
        .ok_or("could not mint account password")?;
    vault
        .upsert(Profile {
            username: account.to_owned(),
            password: password.into(),
            uid: 274_279_111,
            settings,
        })
        .map_err(|error| format!("store initial Never profile: {error}"))?;

    let template = SharedClientTemplate::load(Arc::clone(&profile))
        .map_err(|error| format!("load Engine A template: {error}"))?;
    let mut prep = scenario::get("gatherer").ok_or("missing Gatherer tutorial/relog scenario")?;
    let seed_end = prep
        .steps
        .iter()
        .position(|step| step.name == "prepare Woodcutting 1 and a real bronze axe before Start")
        .ok_or("Gatherer tutorial/relog boundary is missing")?;
    prep.steps.truncate(seed_end);
    prep.steps.push(Step {
        name: "seed 10 hitpoints, one antipoison and one lobster at the real Walk origin",
        kind: StepKind::Perform {
            send: Box::new(|client, _| {
                cheat(client, "setstat hitpoints 10").is_sent()
                    && cheat(client, "give 1doseantipoison 1").is_sent()
                    && cheat(client, "give lobster 1").is_sent()
                    && cheat(client, &tele_args(0, WALK_ORIGIN.x, WALK_ORIGIN.z)).is_sent()
            }),
        },
        wait: Wait {
            arm: Proof::ItemId {
                id: ANTIPOISON_ID,
                count: 1,
            },
            budget_ticks: 120,
        },
    });
    prep.steps.push(Step {
        name: "verify the held lobster for the emergency-eat counterfactual",
        kind: StepKind::Perform {
            send: Box::new(|_, _| true),
        },
        wait: Wait {
            arm: Proof::ItemId {
                id: LOBSTER_ID,
                count: 1,
            },
            budget_ticks: 120,
        },
    });
    prep.steps.push(Step {
        name: "verify 10 hitpoints before the route starts",
        kind: StepKind::Perform {
            send: Box::new(|_, _| true),
        },
        wait: Wait {
            arm: Proof::Stat { id: 3, min: 10 },
            budget_ticks: 120,
        },
    });
    prep.steps.push(Step {
        name: "verify the real route origin before Start",
        kind: StepKind::Perform {
            send: Box::new(|_, _| true),
        },
        wait: Wait {
            arm: Proof::Arrived {
                x: WALK_ORIGIN.x,
                z: WALK_ORIGIN.z,
                level: WALK_ORIGIN.level,
            },
            budget_ticks: 120,
        },
    });
    prep.proof = Proof::Arrived {
        x: WALK_ORIGIN.x,
        z: WALK_ORIGIN.z,
        level: WALK_ORIGIN.level,
    };
    prep.settings.start_script = None;
    let mut runner = ScenarioRunner::with_world(prep, template.world());
    runner.set_map_members(profile.map_members());
    runner.set_live_names(&[account.to_owned()]);
    runner.set_shot_sink(Box::new(|_, _| {}));

    let state = Arc::new(Mutex::new(LiveState {
        runner,
        snapshot: GameSnapshot::new(),
        pump: Pump::new(),
        frame_count: 0,
        was_ingame: false,
        last_tile: None,
        last_hitmark_cycles: [-1; 4],
        arm: None,
        action: None,
        previous_food_count: None,
        previous_dose_count: None,
    }));
    let frame_state = Arc::clone(&state);
    let frame_trace = Arc::clone(&trace);
    let frame_account = account.to_owned();
    let mut play = run_with_template(
        template,
        true,
        vec![],
        |_| (None, None),
        move |client, name, frame| {
            if name != frame_account {
                return;
            }
            let mut state = frame_state.lock();
            let drain = state.pump.drain_client(client);
            host::publish_snapshot(&mut state.snapshot, client, drain);
            state.frame_count = state.frame_count.saturating_add(1);
            let frame_number = state.frame_count;
            let tick = state.snapshot.tick();
            let ingame = client.ingame;
            let food_count = item_count(&state.snapshot, LOBSTER_ID);
            let dose_count = item_count(&state.snapshot, ANTIPOISON_ID);
            let current_tile = state
                .snapshot
                .local_player()
                .map(|player| player.player.network);

            {
                let mut trace = frame_trace.lock();
                if ingame && !state.was_ingame {
                    trace.login_ticks.push(tick);
                    trace.final_login_tick = Some(tick);
                    let login_count = trace.login_ticks.len();
                    trace.event(
                        "login-ready",
                        frame_number,
                        tick,
                        json!({ "login_count": login_count }),
                    );
                }
                if let (Some(previous), Some(tile)) = (state.previous_food_count, Some(food_count))
                {
                    if tile < previous && trace.food_drop_tick.is_none() {
                        trace.food_drop_tick = Some(tick);
                        trace.event(
                            "food-count-decreased",
                            frame_number,
                            tick,
                            json!({ "before": previous, "after": tile }),
                        );
                    }
                }
                if let Some(previous) = state.previous_dose_count {
                    if dose_count < previous && trace.dose_drop_tick.is_none() {
                        trace.dose_drop_tick = Some(tick);
                        trace.event(
                            "antipoison-dose-dropped",
                            frame_number,
                            tick,
                            json!({ "before": previous, "after": dose_count }),
                        );
                    }
                }
                let fresh_drink_line = state.snapshot.chat_lines().iter().any(|line| {
                    line.text == DRINK_LINE
                        && trace
                            .seed_chat_sequence
                            .map_or(true, |sequence| line.sequence > sequence)
                });
                if trace.seed_tick.is_some() && trace.drink_line_tick.is_none() && fresh_drink_line
                {
                    trace.drink_line_tick = Some(tick);
                    trace.event(
                        "antipoison-drink-line",
                        frame_number,
                        tick,
                        json!({ "line": DRINK_LINE }),
                    );
                }
                if trace.walk_started_frame.is_some() {
                    if let Some(tile) = current_tile {
                        if Some(tile) != trace.walk_start_tile && Some(tile) != state.last_tile {
                            trace.route_progress = trace.route_progress.saturating_add(1);
                            if trace.route_progress == 1 {
                                trace.event(
                                    "walk-progress",
                                    frame_number,
                                    tick,
                                    json!({ "tile": tile_json(tile) }),
                                );
                            }
                        }
                    }
                }
                if trace.seed_tick.is_some() {
                    if let Some(marks) = state.snapshot.hitmarks().copied() {
                        for (index, mark) in marks.marks.iter().enumerate() {
                            if mark.kind != 2
                                || mark.cycle <= marks.loop_cycle
                                || state.last_hitmark_cycles[index] == mark.cycle
                            {
                                continue;
                            }
                            state.last_hitmark_cycles[index] = mark.cycle;
                            if trace.first_mark_tick.is_none() {
                                trace.first_mark_tick = Some(tick);
                                trace.first_mark_damage = u8::try_from(mark.value).ok();
                            }
                            if trace.raw_poison_marks.len() < 16 {
                                trace.raw_poison_marks.push(json!({
                                    "tick": tick,
                                    "kind": mark.kind,
                                    "value": mark.value,
                                    "cycle": mark.cycle,
                                    "loop_cycle": marks.loop_cycle,
                                }));
                            }
                            trace.event(
                                "raw-poison-hitmark",
                                frame_number,
                                tick,
                                json!({ "value": mark.value, "cycle": mark.cycle }),
                            );
                        }
                    }
                }
                if let Some(arm) = state.arm.as_ref() {
                    let policy = arm.cure_poison();
                    if policy == ScriptCurePoison::Never && trace.arm_never_frame.is_none() {
                        trace.arm_never_frame = Some(frame_number);
                    }
                    if policy == ScriptCurePoison::WhenNeeded
                        && trace.arm_when_needed_frame.is_none()
                    {
                        trace.arm_when_needed_frame = Some(frame_number);
                        trace.event(
                            "slot-arm-policy-mirrored",
                            frame_number,
                            tick,
                            json!({ "policy": "when_needed" }),
                        );
                    }
                }
                trace.final_food_count = Some(food_count);
                state.was_ingame = ingame;
                state.last_tile = current_tile;
                state.previous_food_count = Some(food_count);
                state.previous_dose_count = Some(dose_count);
            }

            if let Some(action) = state.action.take() {
                match action {
                    FixtureAction::SeedPoison => {
                        let sent = cheat(client, "~poison 27") == client::CheatSend::Sent;
                        let mut trace = frame_trace.lock();
                        trace.seed_tick = sent.then_some(tick);
                        trace.seed_frame = sent.then_some(frame_number);
                        trace.seed_sent = Some(sent);
                        if !sent {
                            trace.seed_error = Some("Engine A refused ~poison 27".to_owned());
                        }
                        trace.event(
                            "poison-seed",
                            frame_number,
                            tick,
                            json!({ "sent": sent, "command": "~poison 27" }),
                        );
                    }
                }
            }

            if !matches!(
                state.runner.status(),
                RunnerStatus::Passed | RunnerStatus::Failed(_)
            ) {
                state.runner.tick_with_hold(client, frame.hold);
            }
        },
    )
    .map_err(|error| format!("start Engine A Play: {error}"))?;
    state.lock().runner.set_obj_names(play.obj_names());
    let post_frame_state = Arc::clone(&state);
    let post_frame_trace = Arc::clone(&trace);
    let post_frame_account = account.to_owned();
    let host_policy_probe = play.script_nav_paint();
    play.set_post_host_frame_observer(move |_, name| {
        if name != post_frame_account {
            return;
        }
        let state = post_frame_state.lock();
        let frame = state.frame_count;
        let tick = state.snapshot.tick();
        let policy = host_policy_probe.native_walk_cure_poison_for_test(name);
        let mut trace = post_frame_trace.lock();
        if policy == Some(ScriptCurePoison::Never) && trace.host_never_frame.is_none() {
            trace.host_never_frame = Some(frame);
            trace.event(
                "host-guard-policy-never-observed",
                frame,
                tick,
                json!({ "policy": "never", "after_copy": true }),
            );
        }
        if policy == Some(ScriptCurePoison::WhenNeeded) && trace.host_when_needed_frame.is_none() {
            trace.host_when_needed_frame = Some(frame);
            trace.host_policy_after_copy = true;
            trace.event(
                "host-guard-policy-mirrored",
                frame,
                tick,
                json!({ "policy": "when_needed", "after_copy": true }),
            );
        }
    });

    let mut session: OperatorSession<()> = OperatorSession::new(InstancePermit::SkipLock);
    session.declare_headless_render();
    session.start_for_test(vault, play);
    let mut surface = HeadlessSurface::new();
    let load = session
        .submit(
            OperatorCommand::Load,
            Targets::Profile(account),
            &mut surface,
        )
        .map_err(|error| format!("load minted account through OperatorSession: {error:?}"))?;
    let load_operation = load.operation().ok_or("Load did not create an operation")?;
    trace.lock().load_operation = Some(load_operation.0);

    let deadline = Instant::now() + CELL_DEADLINE;
    let setup_deadline = Instant::now() + SETUP_DEADLINE;
    let mut script_started = false;
    let mut poison_queued = false;
    let mut setting_operation = None;
    loop {
        session.poll();
        if Instant::now() >= deadline {
            return Err("live cell wall-clock deadline expired".into());
        }

        let setup_status = state.lock().runner.status();
        if let RunnerStatus::Failed(reason) = &setup_status {
            return Err(format!(
                "Gatherer tutorial/relog preparation failed: {reason}"
            ));
        }
        let setup_passed = matches!(&setup_status, RunnerStatus::Passed);
        let load_complete = operation_completed(&session, load_operation, account)?;

        if setup_passed {
            trace.lock().setup_passed = true;
        }
        if setup_passed && load_complete && !script_started {
            let arm = session
                .play_for_test()
                .and_then(|play| play.arm(account))
                .ok_or("Play did not expose the loaded account's SlotArm")?;
            state.lock().arm = Some(arm);
            let start = {
                let play = session
                    .play_for_test()
                    .ok_or("OperatorSession lost its Play before native script start")?;
                play.script_start_handle().start_test_script(
                    account,
                    Box::new(GuardWalkScript {
                        state: Arc::clone(&state),
                        trace: Arc::clone(&trace),
                        begun: false,
                        handle: None,
                    }),
                    None,
                )
            };
            let start = match start {
                Ok(start) => start,
                Err(error) => {
                    let message = error.to_string();
                    trace.lock().script_start_error = Some(message.clone());
                    return Err(format!("start native Walk script: {message}"));
                }
            };
            let play = session
                .play_for_test()
                .ok_or("OperatorSession lost its Play after native script start")?;
            let retained = play.script_native_run(account) == Some(start);
            play.wake(account);
            trace.lock().script_run_retained = retained;
            if !retained {
                return Err("Play did not retain the started native run key".into());
            }
            script_started = true;
        }

        {
            let mut state = state.lock();
            let mut trace = trace.lock();
            if let Some(error) = trace.script_start_error.as_deref() {
                return Err(format!("native script failed to start: {error}"));
            }
            if let Some(error) = trace.walk_start_error.as_deref() {
                return Err(format!("native Walk begin failed: {error}"));
            }
            if trace
                .walk_end
                .as_deref()
                .is_some_and(|end| end != "Arrived")
            {
                return Err(format!(
                    "native Walk ended early: {:?} ({:?})",
                    trace.walk_end, trace.walk_detail
                ));
            }
            if !poison_queued && trace.walk_arrived {
                return Err("native Walk arrived before the poison fixture was seeded".into());
            }
            if let Some(error) = trace.seed_error.as_deref() {
                return Err(error.to_owned());
            }
            if trace.death_tick.is_some() {
                return Err(format!("account died at tick {:?}", trace.death_tick));
            }
            if trace.pre_seed_poison.is_some() && trace.seed_tick.is_none() {
                return Err(format!(
                    "account had poison before the cell seed: {:?}",
                    trace.pre_seed_poison
                ));
            }
            if trace
                .walk_started_tick
                .is_some_and(|start| state.snapshot.tick().saturating_sub(start) > CELL_TICK_LIMIT)
            {
                return Err("native Walk exceeded 600 observed game ticks".into());
            }
            if !poison_queued
                && script_started
                && trace.walk_started_tick.is_some()
                && trace.walk_end.is_none()
                && trace.route_progress > 0
                && trace
                    .final_login_tick
                    .is_some_and(|login| state.snapshot.tick().saturating_sub(login) >= 31)
                && trace.seed_tick.is_none()
                && trace.first_mark_tick.is_none()
            {
                let tick = state.snapshot.tick();
                state.action = Some(FixtureAction::SeedPoison);
                trace.seed_chat_sequence = state
                    .snapshot
                    .chat_lines()
                    .iter()
                    .map(|line| line.sequence)
                    .max();
                trace.seed_queued = true;
                trace.event(
                    "poison-seed-queued",
                    state.frame_count,
                    tick,
                    json!({ "command": "~poison 27" }),
                );
                poison_queued = true;
            }
        }

        if setting_operation.is_none() {
            let (onset, onset_frame, current_frame, current_tick, walk_active) = {
                let state = state.lock();
                let trace = trace.lock();
                (
                    trace.onset_tick,
                    trace.onset_frame,
                    state.frame_count,
                    state.snapshot.tick(),
                    trace.walk_started_tick.is_some() && trace.walk_end.is_none(),
                )
            };
            if let Some(onset_tick) = onset {
                if !walk_active {
                    return Err("poison onset was not observed during an active native Walk".into());
                }
                session.stage_survival(SurvivalEdit::cure_poison(DraftCurePoison::WhenNeeded));
                let submitted = session
                    .submit(
                        OperatorCommand::SetSurvival,
                        Targets::Profile(account),
                        &mut surface,
                    )
                    .map_err(|error| format!("submit SetSurvival: {error:?}"))?;
                let operation = submitted
                    .operation()
                    .ok_or("SetSurvival did not create a save operation")?;
                {
                    let mut trace = trace.lock();
                    trace.setting_operation = Some(operation.0);
                    trace.setting_submit_tick = Some(current_tick);
                    trace.setting_submit_frame = Some(current_frame);
                    trace.event(
                        "operator-set-survival-submitted",
                        current_frame,
                        current_tick,
                        json!({
                            "operation": operation.0,
                            "policy": "when_needed",
                            "onset_tick": onset_tick,
                            "onset_frame": onset_frame,
                        }),
                    );
                }
                setting_operation = Some(operation);
            }
        }

        if let Some(operation) = setting_operation {
            if operation_completed(&session, operation, account)? {
                let (frame, tick) = {
                    let state = state.lock();
                    (state.frame_count, state.snapshot.tick())
                };
                let durable_policy = session
                    .durable_profile(account)
                    .map(|profile| vault_policy_name(profile.settings.survival.cure_poison));
                if durable_policy != Some("when_needed") {
                    return Err(format!("durable profile policy is {durable_policy:?}"));
                }
                let arm_policy = session
                    .play_for_test()
                    .and_then(|play| play.arm(account))
                    .map(|arm| arm.cure_poison());
                if arm_policy != Some(ScriptCurePoison::WhenNeeded) {
                    return Err(format!(
                        "SlotArm policy after durable save is {arm_policy:?}"
                    ));
                }
                let mut trace = trace.lock();
                if !trace.setting_completed {
                    trace.setting_completed = true;
                    trace.durable_policy = durable_policy.map(str::to_owned);
                    trace.durable_frame = Some(frame);
                    trace.durable_tick = Some(tick);
                    trace.event(
                        "operator-setting-durable-and-arm-mirrored",
                        frame,
                        tick,
                        json!({
                            "operation": operation.0,
                            "durable_policy": durable_policy,
                            "arm_policy": "when_needed",
                        }),
                    );
                }
            }
        }

        let done = {
            let trace = trace.lock();
            trace.setting_completed
                && trace.immune_tick.is_some()
                && trace.walk_arrived
                && trace.walk_end.as_deref() == Some("Arrived")
                && trace.arm_when_needed_frame.is_some()
                && trace.native_when_needed_frame.is_some()
                && trace.host_when_needed_frame.is_some()
        };
        if done {
            break;
        }
        if !poison_queued && Instant::now() >= setup_deadline {
            return Err(
                "tutorial/relog, route progress, and account seed exceeded setup deadline".into(),
            );
        }
        std::thread::sleep(POLL_INTERVAL);
    }

    drop(session);
    Ok(())
}

fn operation_completed(
    session: &OperatorSession<()>,
    operation: crate::OperationId,
    account: &str,
) -> Result<bool, String> {
    let report = session
        .operation(operation)
        .ok_or_else(|| format!("operation {} is missing", operation.0))?;
    let outcome = report
        .outcome(account)
        .ok_or_else(|| format!("operation {} has no row for {account}", operation.0))?;
    if outcome.is_failed() || outcome.is_cancelled() || outcome.is_skipped() {
        return Err(format!(
            "operation {} did not complete: {}",
            operation.0,
            outcome.reason().unwrap_or("cancelled or skipped")
        ));
    }
    Ok(outcome.is_completed())
}

fn verify_guard_cure(trace: &Trace) -> Result<(), String> {
    if trace.account.is_none()
        || trace.load_operation.is_none()
        || !trace.setup_passed
        || !trace.script_run_retained
    {
        return Err(
            "live OperatorSession load, scenario setup, or native script start was not observed"
                .into(),
        );
    }
    if trace.login_ticks.len() < 2 {
        return Err(format!(
            "tutorial/relog setup produced {} ready transitions, expected at least two",
            trace.login_ticks.len()
        ));
    }
    let login = trace.final_login_tick.ok_or("missing final login tick")?;
    if trace.login_ticks.last().copied() != Some(login) {
        return Err("final login tick does not match the last ready transition".into());
    }
    let seed = trace.seed_tick.ok_or("missing poison seed tick")?;
    let onset = trace.onset_tick.ok_or("missing poison onset")?;
    if trace.seed_sent != Some(true) {
        return Err(format!("poison seed was not sent: {:?}", trace.seed_sent));
    }
    if seed.saturating_sub(login) < 31 {
        return Err(format!(
            "poison seed was only {} ticks after login",
            seed.saturating_sub(login)
        ));
    }
    if onset < seed {
        return Err(format!(
            "poison onset tick {onset} precedes seed tick {seed}"
        ));
    }
    if trace.pre_seed_poison.is_some() {
        return Err(format!(
            "poison was already present before seed: {:?}",
            trace.pre_seed_poison
        ));
    }
    if trace.onset_p != Some(11) {
        return Err(format!(
            "onset poison value was {:?}, expected 11",
            trace.onset_p
        ));
    }
    if trace.onset_hp != Some(10) || trace.onset_hp_max != Some(10) {
        return Err(format!(
            "onset HP was {:?}/{:?}, expected full 10/10",
            trace.onset_hp, trace.onset_hp_max
        ));
    }
    if trace.walk_start_tile != Some(WALK_ORIGIN) {
        return Err(format!(
            "native Walk started at {:?}, expected {:?}",
            trace.walk_start_tile, WALK_ORIGIN
        ));
    }
    if trace.onset_food_count != Some(1) || trace.onset_dose_count != Some(1) {
        return Err(format!(
            "onset held food/dose was {:?}/{:?}, expected 1/1",
            trace.onset_food_count, trace.onset_dose_count
        ));
    }
    if trace.projected_first_mark_bite() != Some(true) {
        return Err("fixture did not make the first-mark food bite feasible".into());
    }
    if trace.onset_policy.as_deref() != Some("never") {
        return Err(format!(
            "native policy at onset was {:?}, expected never",
            trace.onset_policy
        ));
    }
    if !trace.onset_walk_pending || trace.route_progress_at_onset.unwrap_or(0) == 0 {
        return Err("poison onset was not observed during a progressing native Walk".into());
    }
    if trace.setting_operation.is_none() || !trace.setting_completed {
        return Err("SetSurvival save operation did not complete".into());
    }
    if trace.durable_policy.as_deref() != Some("when_needed") {
        return Err(format!("durable policy was {:?}", trace.durable_policy));
    }
    let disk_policy = trace
        .disk_policy
        .as_deref()
        .ok_or("missing reopened vault policy")?;
    if disk_policy != "when_needed" {
        return Err(format!("reopened vault policy was {disk_policy}"));
    }
    let onset_frame = trace.onset_frame.ok_or("missing onset frame")?;
    let native_never_frame = trace
        .native_never_frame
        .ok_or("NativeTick never observed Never")?;
    let arm_never_frame = trace
        .arm_never_frame
        .ok_or("SlotArm never observed Never")?;
    let host_never_frame = trace
        .host_never_frame
        .ok_or("host guard never observed Never")?;
    if native_never_frame > onset_frame
        || arm_never_frame > onset_frame
        || host_never_frame > onset_frame
    {
        return Err(format!(
            "Never baseline was not observed by onset: native={native_never_frame}, arm={arm_never_frame}, onset={onset_frame}"
        ));
    }
    let submitted_frame = trace
        .setting_submit_frame
        .ok_or("missing SetSurvival frame")?;
    if submitted_frame.abs_diff(onset_frame) > 1 {
        return Err(format!(
            "SetSurvival was submitted {} frames from Onset",
            submitted_frame.abs_diff(onset_frame)
        ));
    }
    let durable_frame = trace.durable_frame.ok_or("missing durable save frame")?;
    let arm_frame = trace
        .arm_when_needed_frame
        .ok_or("SlotArm never mirrored WhenNeeded")?;
    let native_frame = trace
        .native_when_needed_frame
        .ok_or("NativeTick never observed WhenNeeded")?;
    let host_frame = trace
        .host_when_needed_frame
        .ok_or("host guard never mirrored WhenNeeded")?;
    if !trace.host_policy_after_copy {
        return Err("host policy was read before the per-frame copy".into());
    }
    if durable_frame.abs_diff(arm_frame) > 1 || durable_frame.abs_diff(host_frame) > 1 {
        return Err(format!(
            "running policy missed one-frame mirror: durable={durable_frame}, arm={arm_frame}, host={host_frame}, native_poll={native_frame}"
        ));
    }
    if trace.drink_line_tick.is_none()
        || trace.dose_drop_tick.is_none()
        || trace.immune_tick.is_none()
    {
        return Err(format!(
            "Drink proof incomplete: line={:?}, dose_drop={:?}, immune={:?}",
            trace.drink_line_tick, trace.dose_drop_tick, trace.immune_tick
        ));
    }
    let drink_tick = trace.drink_line_tick.expect("checked above");
    let dose_tick = trace.dose_drop_tick.expect("checked above");
    let immune_tick = trace.immune_tick.expect("checked above");
    if immune_tick.saturating_sub(onset) > 10 {
        return Err("cure proof arrived more than ten observed ticks after onset".into());
    }
    if drink_tick > immune_tick || dose_tick > immune_tick {
        return Err("drink line and dose drop did not precede the Immune condition".into());
    }
    if let Some(mark_tick) = trace.first_mark_tick {
        return Err(format!(
            "poison mark at {mark_tick} followed a fresh-timer seed despite the cure"
        ));
    }
    if immune_tick.saturating_sub(onset) >= POISON_TICKS_TO_FIRST_FIRE {
        return Err(format!(
            "Immune at {immune_tick} was not observed before the first-fire boundary after onset {onset}"
        ));
    }
    if trace.food_drop_tick.is_some() {
        return Err(format!("food was eaten at tick {:?}", trace.food_drop_tick));
    }
    if trace.food_at_immune != Some(1) || trace.dose_at_immune != Some(0) {
        return Err(format!(
            "held food/dose at Immune was {:?}/{:?}, expected 1/0",
            trace.food_at_immune, trace.dose_at_immune
        ));
    }
    if trace.final_food_count != Some(1) {
        return Err(format!(
            "final held food count was {:?}",
            trace.final_food_count
        ));
    }
    if trace.death_tick.is_some() {
        return Err(format!("death observed at tick {:?}", trace.death_tick));
    }
    if !trace.walk_arrived || trace.walk_end.as_deref() != Some("Arrived") {
        return Err(format!("real Walk did not arrive: {:?}", trace.walk_end));
    }
    Ok(())
}

fn item_count(snapshot: &GameSnapshot, id: i32) -> i32 {
    snapshot
        .inventory()
        .iter()
        .filter(|item| item.def.id == id)
        .map(|item| item.count)
        .fold(0_i32, i32::saturating_add)
}

fn required_path(name: &str) -> Result<PathBuf, String> {
    let path = std::env::var_os(name)
        .map(PathBuf::from)
        .ok_or_else(|| format!("{name} is required"))?;
    if !path.is_absolute() {
        return Err(format!("{name} must be an absolute path"));
    }
    Ok(path)
}

fn script_policy_name(policy: ScriptCurePoison) -> &'static str {
    match policy {
        ScriptCurePoison::Never => "never",
        ScriptCurePoison::WhenNeeded => "when_needed",
        ScriptCurePoison::Always => "always",
    }
}

fn vault_policy_name(policy: VaultCurePoison) -> &'static str {
    match policy {
        VaultCurePoison::Never => "never",
        VaultCurePoison::WhenNeeded => "when_needed",
        VaultCurePoison::Always => "always",
    }
}

fn tile_json(tile: WorldTile) -> Value {
    json!({ "x": tile.x, "z": tile.z, "level": tile.level })
}

#[cfg(test)]
mod oracle_tests {
    use super::*;

    fn passing_trace() -> Trace {
        Trace {
            account: Some("live-test".into()),
            login_ticks: vec![90, 100],
            final_login_tick: Some(100),
            load_operation: Some(1),
            setup_passed: true,
            script_run_retained: true,
            native_never_frame: Some(10),
            native_never_tick: Some(131),
            native_when_needed_frame: Some(31),
            native_when_needed_tick: Some(135),
            arm_never_frame: Some(10),
            arm_when_needed_frame: Some(31),
            host_never_frame: Some(10),
            host_when_needed_frame: Some(31),
            host_policy_after_copy: true,
            setting_operation: Some(2),
            setting_submit_tick: Some(132),
            setting_submit_frame: Some(28),
            setting_completed: true,
            durable_policy: Some("when_needed".into()),
            durable_frame: Some(30),
            durable_tick: Some(134),
            disk_policy: Some("when_needed".into()),
            walk_started_tick: Some(131),
            walk_started_frame: Some(9),
            walk_start_tile: Some(WALK_ORIGIN),
            route_progress: 5,
            route_progress_at_onset: Some(4),
            seed_queued: true,
            seed_tick: Some(131),
            seed_frame: Some(10),
            seed_sent: Some(true),
            onset_tick: Some(132),
            onset_frame: Some(27),
            onset_since: Some(132),
            onset_p: Some(11),
            onset_hp: Some(10),
            onset_hp_max: Some(10),
            onset_food_count: Some(1),
            onset_dose_count: Some(1),
            onset_policy: Some("never".into()),
            onset_walk_pending: true,
            drink_line_tick: Some(136),
            dose_drop_tick: Some(136),
            immune_tick: Some(136),
            immune_until: Some(256),
            food_at_immune: Some(1),
            dose_at_immune: Some(0),
            final_food_count: Some(1),
            walk_end: Some("Arrived".into()),
            walk_arrived: true,
            walk_receipt_tick: Some(200),
            ..Trace::default()
        }
    }

    #[test]
    fn oracle_accepts_durable_mirror_drink_and_arrived_walk() {
        assert_eq!(verify_guard_cure(&passing_trace()), Ok(()));
    }

    #[test]
    fn oracle_rejects_host_policy_read_before_the_copy() {
        let mut trace = passing_trace();
        trace.host_policy_after_copy = false;
        assert_eq!(
            verify_guard_cure(&trace),
            Err("host policy was read before the per-frame copy".into())
        );
    }

    #[test]
    fn oracle_rejects_wrong_start_tile_and_low_hitpoints() {
        let mut wrong_start = passing_trace();
        wrong_start.walk_start_tile = Some(WALK_TARGET);
        assert!(verify_guard_cure(&wrong_start).is_err());

        let mut low_hitpoints = passing_trace();
        low_hitpoints.onset_hp = Some(5);
        low_hitpoints.onset_hp_max = Some(60);
        assert!(verify_guard_cure(&low_hitpoints).is_err());
    }

    #[test]
    fn oracle_rejects_food_before_the_cure() {
        let mut trace = passing_trace();
        trace.food_drop_tick = Some(135);
        assert!(verify_guard_cure(&trace).is_err());
    }

    #[test]
    fn oracle_rejects_missing_drink_line_or_dose_drop() {
        let mut no_line = passing_trace();
        no_line.drink_line_tick = None;
        assert!(verify_guard_cure(&no_line).is_err());

        let mut no_dose_drop = passing_trace();
        no_dose_drop.dose_drop_tick = None;
        assert!(verify_guard_cure(&no_dose_drop).is_err());
    }

    #[test]
    fn oracle_rejects_missing_immunity_or_late_cure() {
        let mut no_immune = passing_trace();
        no_immune.immune_tick = None;
        assert!(verify_guard_cure(&no_immune).is_err());

        let mut late = passing_trace();
        late.immune_tick = Some(163);
        late.drink_line_tick = Some(163);
        late.dose_drop_tick = Some(163);
        assert!(verify_guard_cure(&late).is_err());
    }

    #[test]
    fn oracle_rejects_missing_durable_one_frame_mirror() {
        let mut late_arm = passing_trace();
        late_arm.arm_when_needed_frame = Some(32);
        assert!(verify_guard_cure(&late_arm).is_err());

        let mut late_host = passing_trace();
        late_host.host_when_needed_frame = Some(32);
        assert!(verify_guard_cure(&late_host).is_err());
    }

    #[test]
    fn oracle_accepts_the_next_native_game_tick_after_the_host_frame_mirror() {
        let mut trace = passing_trace();
        trace.native_when_needed_frame = Some(57);
        assert!(verify_guard_cure(&trace).is_ok());
    }

    #[test]
    fn oracle_rejects_death_or_non_arrival() {
        let mut dead = passing_trace();
        dead.death_tick = Some(150);
        assert!(verify_guard_cure(&dead).is_err());

        let mut blocked = passing_trace();
        blocked.walk_end = Some("Blocked".into());
        blocked.walk_arrived = false;
        assert!(verify_guard_cure(&blocked).is_err());
    }

    #[test]
    fn oracle_rejects_poison_marks_after_a_fresh_timer_cure() {
        for tick in [135, 162] {
            let mut trace = passing_trace();
            trace.first_mark_tick = Some(tick);
            trace.first_mark_damage = Some(6);
            assert!(verify_guard_cure(&trace).is_err());
        }
    }
}
