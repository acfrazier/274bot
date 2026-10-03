//! Live M1-M6 melee proofs through the ordinary host-play observer.
//!
//! Run one ignored case at a time with `LIVE=1`, `BOT_ENGINE_DIR`, and the
//! selected local 289 navigation pack configured. The only HOME used is a
//! per-run temporary directory; receipts and terminal PNGs go to the assigned
//! evidence directory.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use api::game_data::SelectedGameData;
use api::interact::{Interactions, SendResult};
use api::quest_facts::QuestCatalog;
use api::selected::{ClientRevision, RunKey};
use api::snapshot::{GameSnapshot, ReadContext, WorldTile};
use host::{FrameBuf, Pump};
use scenario::{
    Proof, RunnerStatus, Scenario, ScenarioRunner, ScriptInjectValue, ScriptSettingInject, Step,
    StepKind, Wait,
};
use script::quester::compile::{compile_path, CompiledPath};
use script::quester::runner::Quester;
use serde_json::{json, Value};
use vault::{Profile, ProfileSettings};

use super::combat_proof::{self, CaptureRegistration, CombatCapture};
use super::{run_with_template, ProfileOptions, ScriptStartHandle};

const EVIDENCE_DIR: &str = "/Volumes/dev-scratch/274bot-evidence/COMBAT-S3A-3";
const IMP_START: WorldTile = WorldTile {
    x: 2632,
    z: 3222,
    level: 0,
};
const IMP_QUESTER_SETTINGS: &[ScriptSettingInject] = &[ScriptSettingInject {
    id: "quests",
    value: ScriptInjectValue::StrList(&["imp"]),
}];
const WARLORD_ANCHOR: WorldTile = WorldTile {
    x: 2457,
    z: 3302,
    level: 0,
};
// Outside every nearby static tree's one-tile hunt range. The fixture's first
// native walk enters3108/3346's one-tile hunt range after the exact HP8 Start.
const TREE_APPROACH: WorldTile = WorldTile {
    x: 3110,
    z: 3346,
    level: 0,
};
const TREE_SPAWNS: [WorldTile; 3] = [
    WorldTile {
        x: 3108,
        z: 3346,
        level: 0,
    },
    WorldTile {
        x: 3111,
        z: 3339,
        level: 0,
    },
    WorldTile {
        x: 3111,
        z: 3348,
        level: 0,
    },
];
const WALK_OUT: WorldTile = WorldTile {
    x: 3093,
    z: 3243,
    level: 0,
};
/// Fixture walk-out settle radius in `combat_unattackable.json`.
const WALK_OUT_RADIUS: i32 = 2;
const BRONZE_SCIMITAR_ID: i32 = 1321;
const RUNE_SCIMITAR_ID: i32 = 1333;
const LOBSTER_ID: i32 = 379;
const COOKED_KARAMBWAN_ID: i32 = 3144;
const PRAYER_POTION_4_ID: i32 = 2434;
const SUPER_ATTACK_4_ID: i32 = 2436;
const IMP_BEADS: [i32; 4] = [1470, 1472, 1474, 1476];

const M1_ITEMS: &[(&str, i32)] = &[("bronze_scimitar", 1)];
const M2_ITEMS: &[(&str, i32)] = &[
    ("rune_scimitar", 1),
    ("lobster", 6),
    ("4doseprayerrestore", 2),
    ("4dose2attack", 1),
];
const M3_ITEMS: &[(&str, i32)] = &[("rune_scimitar", 1), ("lobster", 20)];
const M4_ITEMS: &[(&str, i32)] = &[("lobster", 4)];
const M5_ITEMS: &[(&str, i32)] = &[("bronze_scimitar", 1)];
const M6_ITEMS: &[(&str, i32)] = &[
    ("rune_scimitar", 1),
    ("lobster", 6),
    ("tbwt_cooked_karambwan", 4),
];
// S3:583 permits re-staging exhausted food; 27 foods plus the weapon fit
// the 28-slot inventory. Keep the original seed for the abort observation.
const M6_RESERVE_ITEMS: &[(&str, i32)] = &[
    ("rune_scimitar", 1),
    ("lobster", 23),
    ("tbwt_cooked_karambwan", 4),
];

// Selected content drop tables/scripts/imp.rs2:10-19 has four mutually
// exclusive 5/128 buckets. Inclusion-exclusion gives 95% joint coverage at
// 110 kills, not the 76 kills needed for one specified colour. This is a
// probabilistic kill bound, never a guarantee about wall-clock acquisition.
// COMBAT-S3A-2 authorizes one natural experiment of up to 2.5 hours.
fn m1_coverage_budget() -> (u64, f64, u64) {
    let coverage = |n: i32| {
        1.0 - 4.0 * (123.0_f64 / 128.0).powi(n) + 6.0 * (118.0_f64 / 128.0).powi(n)
            - 4.0 * (113.0_f64 / 128.0).powi(n)
            + (108.0_f64 / 128.0).powi(n)
    };
    let kills = (1..).find(|n| coverage(*n) >= 0.95).unwrap() as u64;
    let observed_cadence = 3970.38 / 51.0;
    (kills, observed_cadence, 9_000)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Case {
    M1,
    M1HandIn,
    M2,
    M3,
    M4,
    M5,
    M6,
}

impl Case {
    fn key(self) -> &'static str {
        match self {
            Self::M1 => "M1",
            Self::M1HandIn => "M1-staged-hand-in",
            Self::M2 => "M2",
            Self::M3 => "M3",
            Self::M4 => "M4",
            Self::M5 => "M5",
            Self::M6 => "M6",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::M1 => "combat_m1_imp_production",
            Self::M1HandIn => "combat_m1_staged_hand_in",
            Self::M2 => "combat_m2_melee_upkeep",
            Self::M3 => "combat_m3_melee_food_only",
            Self::M4 => "combat_m4_unattackable_tree",
            Self::M5 => "combat_m5_random_interrupt_clear_prayers",
            Self::M6 => "combat_m6_melee_combo_eat",
        }
    }

    fn path_relative(self) -> &'static str {
        match self {
            Self::M1 | Self::M1HandIn | Self::M5 => "imp.json",
            Self::M2 => "fixtures/combat_melee_upkeep.json",
            Self::M3 | Self::M6 => "fixtures/combat_melee_food_only.json",
            Self::M4 => "fixtures/combat_unattackable.json",
        }
    }

    fn items(self) -> &'static [(&'static str, i32)] {
        match self {
            Self::M1 => M1_ITEMS,
            Self::M1HandIn => &[
                ("bronze_scimitar", 1),
                ("red_bead", 1),
                ("yellow_bead", 1),
                ("black_bead", 1),
                ("white_bead", 1),
            ],
            Self::M2 => M2_ITEMS,
            Self::M3 => M3_ITEMS,
            Self::M4 => M4_ITEMS,
            Self::M5 => M5_ITEMS,
            Self::M6 if std::env::var_os("BOT_COMBAT_M6_ORIGINAL_FOOD").is_some() => M6_ITEMS,
            Self::M6 => M6_RESERVE_ITEMS,
        }
    }

    fn timeout(self) -> Duration {
        match self {
            Self::M1 => Duration::from_secs(m1_coverage_budget().2),
            Self::M1HandIn => Duration::from_secs(1_200),
            Self::M5 => Duration::from_secs(1_200),
            Self::M2 | Self::M3 | Self::M6 => Duration::from_secs(900),
            Self::M4 => Duration::from_secs(300),
        }
    }

    fn stand(self, world: &nav::world::NavWorld) -> Result<WorldTile, String> {
        match self {
            Self::M1 | Self::M5 => Ok(IMP_START),
            // COMBAT-S3A-2 operator-authorized hand-in-only staging: the
            // CB46 seed cannot cross the default danger zones from Ardougne.
            // Start on the Wizard Tower ground floor, not at the farm.
            // wizard_mizgog.rs2:3-19,39-52 still runs both production talks.
            Self::M1HandIn => {
                let start = WorldTile {
                    x: 3102,
                    z: 3160,
                    level: 0,
                };
                world
                    .collision
                    .standable(start)
                    .then_some(start)
                    .ok_or_else(|| "Wizard Tower hand-in start is not standable".to_owned())
            }
            Self::M2 | Self::M3 | Self::M6 => standable_neighbor(world, WARLORD_ANCHOR),
            Self::M4 => world
                .collision
                .standable(TREE_APPROACH)
                .then_some(TREE_APPROACH)
                .ok_or_else(|| "safe Draynor Manor tree approach is not standable".to_owned()),
        }
    }

    fn inject_maze(self) -> bool {
        self == Self::M5
    }
}

struct ThrowawayHome {
    path: PathBuf,
    previous: Option<std::ffi::OsString>,
}

impl ThrowawayHome {
    fn enter(label: &str) -> Result<Self, String> {
        let id = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| format!("clock: {error}"))?
            .as_nanos();
        let path = PathBuf::from(EVIDENCE_DIR)
            .join("homes")
            .join(format!("{label}-{}-{id}", std::process::id()));
        std::fs::create_dir_all(&path)
            .map_err(|error| format!("create throwaway HOME {}: {error}", path.display()))?;
        let previous = std::env::var_os("HOME");
        std::env::set_var("HOME", &path);
        Ok(Self { path, previous })
    }
}

impl Drop for ThrowawayHome {
    fn drop(&mut self) {
        match &self.previous {
            Some(previous) => std::env::set_var("HOME", previous),
            None => std::env::remove_var("HOME"),
        }
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

struct CpuRendererEnv(Option<std::ffi::OsString>);

impl CpuRendererEnv {
    fn enable() -> Self {
        let previous = std::env::var_os("BOT_CPU");
        std::env::set_var("BOT_CPU", "1");
        Self(previous)
    }
}

impl Drop for CpuRendererEnv {
    fn drop(&mut self) {
        match &self.0 {
            Some(previous) => std::env::set_var("BOT_CPU", previous),
            None => std::env::remove_var("BOT_CPU"),
        }
    }
}

struct LiveState {
    case: Case,
    account: String,
    runner: ScenarioRunner,
    snapshot: GameSnapshot,
    pump: Pump,
    start_context: Option<(ScriptStartHandle, Arc<api::named_banks::NamedBankFacts>)>,
    selected: Arc<SelectedGameData>,
    quests: Arc<QuestCatalog>,
    path: Arc<CompiledPath>,
    capture: Arc<Mutex<CombatCapture>>,
    started: bool,
}

impl LiveState {
    fn frame(&mut self, client: &mut client::client::Client, hold: bool) {
        let drain = self.pump.drain_client(client);
        host::publish_snapshot(&mut self.snapshot, client, drain);

        if self.runner.on_start_script() && !self.started {
            combat_proof::record_start_baseline(&self.account, &self.snapshot);
            if let Some(reason) = start_preflight(
                self.case,
                &combat_proof::snapshot_facts(&self.snapshot, None),
            ) {
                combat_proof::mark_invalid(&self.account, reason);
                return;
            }
            let Some((handle, banks)) = self.start_context.as_ref() else {
                combat_proof::mark_invalid(
                    &self.account,
                    "StartScript reached before the native preparation context was installed",
                );
                return;
            };
            let machine = Quester::new(
                RunKey {
                    slot: 0,
                    run: 0,
                    session: 0,
                },
                Arc::clone(&self.path),
                Arc::clone(&self.selected),
                Arc::clone(&self.quests),
                Arc::clone(banks),
            );
            match handle.start_test_script(
                &self.account,
                Box::new(machine),
                Some(Arc::clone(&self.selected)),
            ) {
                Ok(_) => {
                    self.started = true;
                    self.capture
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .started = true;
                    self.runner.observe_script_running();
                }
                Err(error) => {
                    combat_proof::mark_invalid(
                        &self.account,
                        format!("install compiled Quester failed: {error}"),
                    );
                    return;
                }
            }
        }

        // Startup ClearPrayers consumes the seed. Imp Path does not keep
        // Protect through Engage, so the Maze hold freezes the live Attack
        // owner and this staging click restores Protect before injection.
        // A real IF_BUTTON on a later observed host tick, never a cheat or
        // a second producer on the Attack plan's exclusive tick.
        if self.case == Case::M5 && self.started {
            let protect_on = combat_proof::protect_from_melee_active(&self.snapshot);
            let host_tick =
                combat_proof::host_tick_for_snapshot(&self.account, self.snapshot.tick());
            let (pending, already, attack_tick) = {
                let capture = self.capture.lock().unwrap_or_else(|e| e.into_inner());
                (
                    capture.maze_pending && !capture.maze_injected,
                    capture.maze_prayer_restaged,
                    capture
                        .actions
                        .iter()
                        .rev()
                        .find(|action| is_npc_attack(action))
                        .and_then(|action| action["tick"].as_u64()),
                )
            };
            if pending
                && !protect_on
                && !already
                && host_tick
                    .zip(attack_tick)
                    .is_some_and(|(now, attack)| now > attack)
            {
                let mut staged = false;
                if let Some(component_id) = self.selected.prayers().iter().find_map(|prayer| {
                    prayer
                        .name
                        .eq_ignore_ascii_case("Protect from Melee")
                        .then_some(prayer.button_com)
                }) {
                    if let Some(widget) = ReadContext::new(&self.snapshot).component(component_id) {
                        use api::interact::Driver;
                        let checkpoint = client.packet_checkpoint();
                        if matches!(
                            Interactions::new(&self.snapshot, client).if_button(widget),
                            SendResult::Sent { .. }
                        ) {
                            combat_proof::record_other_request(
                                &self.account,
                                host_tick,
                                &self.snapshot,
                                "staging",
                                "if-button Protect from Melee",
                            );
                            let mut opcodes = Vec::new();
                            let decoded = checkpoint.is_some_and(|checkpoint| {
                                client
                                    .trace_packets(*checkpoint, &mut |opcode| opcodes.push(opcode))
                            });
                            let mut capture =
                                self.capture.lock().unwrap_or_else(|e| e.into_inner());
                            if let Some(action) = capture.actions.last_mut() {
                                action["accepted"] = json!(true);
                                action["wire_decoded"] = json!(decoded);
                                action["wire_opcodes"] = json!(opcodes);
                            }
                            staged = true;
                        }
                    }
                }
                // No cheat fallback in the measured window (R4 §5 common
                // fixture contract). A missing widget cannot manufacture proof.
                if staged {
                    self.capture
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .maze_prayer_restaged = true;
                }
            }
        }

        if matches!(
            self.runner.status(),
            RunnerStatus::Passed | RunnerStatus::Failed(_)
        ) {
            return;
        }
        self.runner.tick_with_hold(client, hold);
    }
}

struct EvidenceWriter {
    case: Case,
    account: String,
    path: PathBuf,
    capture: Arc<Mutex<CombatCapture>>,
    frame: Arc<FrameBuf>,
    scenario_status: String,
    outcome: String,
    error: Option<String>,
    flushed: bool,
}

impl EvidenceWriter {
    fn flush(&mut self) {
        if self.flushed {
            return;
        }
        self.flushed = true;
        let capture = self.capture.lock().unwrap_or_else(|e| e.into_inner());
        let m3_timing = if self.case == Case::M3 {
            m3_timing_receipt(&capture)
        } else {
            Value::Null
        };
        let m6_combo = if self.case == Case::M6 {
            m6_combo_receipt(&capture)
        } else {
            Value::Null
        };
        let m4_eat = if self.case == Case::M4 {
            report_with_end(&capture, "Aborted(Unattackable)").map_or(Value::Null, |report| {
                m4_conditional_eat_receipt(&capture, report)
            })
        } else {
            Value::Null
        };
        let m5_clear = if self.case == Case::M5 {
            capture
                .random_events
                .iter()
                .find(|event| event["kind"] == "Maze")
                .is_some_and(|event| clear_prayers_before_next_operation(&capture, event))
        } else {
            false
        };
        let m1_deadline = if self.case == Case::M1 {
            let (kills, cadence, deadline) = m1_coverage_budget();
            json!({
                "content_source": "drop tables/scripts/imp.rs2",
                "each_colour_probability": "5/128 (mutually exclusive)",
                "joint_coverage": 0.950827,
                "kill_formula": "min n: 1-4*(123/128)^n+6*(118/128)^n-4*(113/128)^n+(108/128)^n >= .95",
                "required_kills": kills,
                "reference_receipt": "M1-cb1rquyywv_0-receipt.json",
                "reference_corpse_episodes": 51,
                "reference_distinct_killed": 49,
                "seconds_per_corpse_episode": cadence,
                "deadline_basis": "COMBAT-S3A-2 authorized 2.5h wall experiment; no guarantee of 110 kills or acquisition",
                "deadline_seconds": deadline,
            })
        } else {
            Value::Null
        };
        let mut receipt = json!({
            "proof": "COMBAT-S3A-3",
            "case": self.case.key(),
            "scenario": self.case.label(),
            "outcome": self.outcome,
            "error": self.error,
            "scenario_status": self.scenario_status,
            "account": self.account,
            "path": self.case.path_relative(),
            "started": capture.started,
            "invalid_reason": capture.invalid_reason,
            "start_baseline": capture.start_baseline,
            "statuses": capture.statuses,
            "actions": capture.actions,
            "random_events": capture.random_events,
            "observations": capture.observations,
            "prayer_facts": capture.prayer_facts,
            "frames": capture.frames,
            "maze_attack_owner_live_before": capture.maze_owner_live_before,
            "maze_attack_owner_live_after": capture.maze_owner_live_after,
            "m3_timing": m3_timing,
            "m6_combo": m6_combo,
            "warlord_first_open_hitbar": capture.frames.iter().find_map(|frame| {
                frame["nearby_npcs"].as_array()?.iter().find(|npc| {
                    npc["type"] == 477 && npc["total_health"].as_i64().is_some_and(|hp| hp > 0)
                }).map(|npc| json!({
                    "tick": frame["tick"], "health": npc["health"],
                    "total_health": npc["total_health"], "index": npc["index"],
                }))
            }),
            "m4_conditional_eat": m4_eat,
            "m5_clear_prayers_before_next_operation": m5_clear,
            "m1_content_deadline": m1_deadline,
            "combat_outcomes": combat_outcomes(&capture),
            "imp_corpse_classification": imp_corpse_classification(&capture),
        });
        drop(capture);

        let pixels = self.frame.snapshot();
        let png_path = if pixels.len() == (765 * 503) as usize {
            let mut rgba = Vec::with_capacity(pixels.len() * 4);
            for pixel in &pixels {
                rgba.push(((pixel >> 16) & 0xff) as u8);
                rgba.push(((pixel >> 8) & 0xff) as u8);
                rgba.push((pixel & 0xff) as u8);
                rgba.push(0xff);
            }
            let sidecar = serde_json::to_string_pretty(&receipt).unwrap_or_default();
            scenario::shot::write_shot(
                &self.path,
                &format!("{}-{}-terminal", self.case.key(), self.account),
                &rgba,
                765,
                503,
                &sidecar,
            )
            .ok()
        } else {
            None
        };
        receipt["terminal_png"] = png_path
            .as_ref()
            .map(|path| json!(path.display().to_string()))
            .unwrap_or(Value::Null);
        receipt["terminal_frame_pixels"] = json!(pixels.len());
        let receipt_path =
            self.path
                .join(format!("{}-{}-receipt.json", self.case.key(), self.account));
        match serde_json::to_vec_pretty(&receipt)
            .map_err(|error| error.to_string())
            .and_then(|bytes| {
                std::fs::write(&receipt_path, bytes).map_err(|error| error.to_string())
            }) {
            Ok(()) => eprintln!("combat proof receipt: {}", receipt_path.display()),
            Err(error) => eprintln!("combat proof receipt write failed: {error}"),
        }
    }
}

impl Drop for EvidenceWriter {
    fn drop(&mut self) {
        self.flush();
    }
}

fn profile_options(home: &Path) -> Result<ProfileOptions, String> {
    let engine_dir = std::env::var_os("BOT_ENGINE_DIR")
        .map(PathBuf::from)
        .ok_or_else(|| "combat live proof requires BOT_ENGINE_DIR".to_owned())?;
    let nav_pack = std::env::var_os("BOT_NAV_PACK")
        .map(PathBuf::from)
        .or_else(|| {
            let candidate = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../target/debug/nav/289/274bot.navpack");
            candidate.exists().then_some(candidate)
        });
    let port = std::env::var("BOT_GAME_PORT")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(45_594);
    let http_port = std::env::var("BOT_HTTP_PORT")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(2_080);
    // Runtime preparation may publish OnDemand entries. Copy the retained
    // complete snapshot into this run's writable HOME, never symlink it.
    let source = std::env::var_os("BOT_COMBAT_CACHE_SNAPSHOT")
        .map(PathBuf::from)
        .ok_or_else(|| "combat live proof requires BOT_COMBAT_CACHE_SNAPSHOT".to_owned())?;
    let version = source
        .file_name()
        .ok_or_else(|| "cache snapshot has no version directory".to_owned())?;
    let cache = home.join("unpack").join(version);
    std::fs::create_dir_all(&cache).map_err(|error| format!("create cache copy: {error}"))?;
    for entry in
        std::fs::read_dir(&source).map_err(|error| format!("read retained cache: {error}"))?
    {
        let entry = entry.map_err(|error| format!("read retained cache entry: {error}"))?;
        if !entry
            .file_type()
            .map_err(|error| format!("cache entry type: {error}"))?
            .is_file()
        {
            return Err(format!(
                "retained cache entry is not a regular file: {}",
                entry.path().display()
            ));
        }
        std::fs::copy(entry.path(), cache.join(entry.file_name()))
            .map_err(|error| format!("copy retained cache: {error}"))?;
    }
    Ok(ProfileOptions {
        profile: Some("local-289".into()),
        revision: Some("289".into()),
        host: Some("127.0.0.1".into()),
        asset_host: Some("127.0.0.1".into()),
        port: Some(port),
        http_port: Some(http_port),
        vault_path: Some(home.join("vault")),
        cache_dir: Some(cache),
        unpack_dir: Some(home.join("unpack")),
        nav_pack,
        engine_dir: Some(engine_dir),
        ..ProfileOptions::default()
    })
}

fn scenario_for(case: Case, stand: WorldTile, capture: Arc<Mutex<CombatCapture>>) -> Scenario {
    let mut scenario =
        scenario::quester_stage(case.label(), "Imp Catcher", "imp", 0, case.items(), stand);
    scenario.settings.script_settings_inject = Some(IMP_QUESTER_SETTINGS);
    let stand_index = scenario
        .steps
        .iter()
        .position(|step| step.name == "stand at the quest start")
        .expect("Quester stage has a stand step");
    // Quest colour needs the relog, but a dev-engine relog can reset position.
    // Stage the exact combat stats and destination in the completed session.
    let relog_index = scenario
        .steps
        .iter()
        .rposition(|step| matches!(step.kind, StepKind::Relog))
        .expect("Quester stage has a final quest-colour relog step");
    let relog = scenario.steps.remove(relog_index);
    scenario.steps.insert(stand_index, relog);
    let preparation = preparation_steps(case);
    scenario
        .steps
        .splice(stand_index + 1..stand_index + 1, preparation);
    let start_index = scenario
        .steps
        .iter()
        .position(|step| matches!(step.kind, StepKind::StartScript))
        .expect("Quester stage has StartScript");
    scenario.steps.truncate(start_index + 1);
    let ready_capture = Arc::clone(&capture);
    scenario.steps.push(Step {
        name: "wait for the bounded combat proof",
        kind: StepKind::Await {
            evidence: "combat proof criteria",
            ready: Box::new(move |_| {
                let capture = ready_capture.lock().unwrap_or_else(|e| e.into_inner());
                capture.invalid_reason.is_some() || case_ready(case, &capture)
            }),
        },
        wait: Wait {
            arm: Proof::Stat { id: 0, min: 1 },
            // Await counts dirty snapshots, not 600ms engine ticks. Only the
            // monotonic wall deadline in run_case bounds this measured wait.
            budget_ticks: u32::MAX,
        },
    });
    scenario.proof = Proof::Stat { id: 0, min: 1 };
    scenario.settings.deadline = case.timeout();
    scenario.settings.nav.engine_speed_ms = None;
    scenario
}

fn preparation_steps(case: Case) -> Vec<Step> {
    let stats: &[(&'static str, i32, i32)] = match case {
        Case::M1 | Case::M1HandIn => &[
            ("attack", 0, 40),
            ("strength", 2, 40),
            ("defence", 1, 40),
            ("hitpoints", 3, 40),
            ("prayer", 5, 1),
        ],
        Case::M2 => &[
            ("attack", 0, 60),
            ("strength", 2, 60),
            ("defence", 1, 40),
            ("hitpoints", 3, 40),
            ("prayer", 5, 43),
        ],
        Case::M3 | Case::M6 => &[
            ("attack", 0, 60),
            ("strength", 2, 60),
            ("defence", 1, 40),
            ("hitpoints", 3, 40),
            ("prayer", 5, 1),
        ],
        Case::M4 => &[("hitpoints", 3, 30), ("prayer", 5, 1)],
        Case::M5 => &[
            ("attack", 0, 40),
            ("strength", 2, 40),
            ("defence", 1, 40),
            ("hitpoints", 3, 40),
            ("prayer", 5, 43),
        ],
    };
    let mut steps = stats
        .iter()
        .map(|(skill, index, level)| {
            cheat_step(
                "seed combat stat before Start",
                format!("setstat {skill} {level}"),
                Proof::Stat {
                    id: *index,
                    min: *level,
                },
            )
        })
        .collect::<Vec<_>>();
    match case {
        Case::M1 | Case::M1HandIn => steps.push(wear_step(BRONZE_SCIMITAR_ID)),
        Case::M2 => {
            steps.push(cheat_step(
                "drain Prayer to the upkeep floor before Start",
                "~stat_drain prayer 17 0".to_owned(),
                Proof::Stat { id: 5, min: 17 },
            ));
        }
        Case::M3 => steps.push(wear_step(RUNE_SCIMITAR_ID)),
        Case::M4 => steps.push(cheat_step(
            "drain Hitpoints to eight before hostile-area teleport",
            "~stat_drain hitpoints 22 0".to_owned(),
            Proof::Stat { id: 3, min: 8 },
        )),
        Case::M5 => {
            steps.push(wear_step(BRONZE_SCIMITAR_ID));
            steps.push(cheat_step(
                "seed Protect from Melee before Start",
                // R289 setvar resolves debug names; varp.pack maps97 to prayer14.
                "setvar prayer14 1".to_owned(),
                Proof::VarpExact { id: 97, value: 1 },
            ));
        }
        Case::M6 => steps.push(cheat_step(
            "drain Hitpoints to sixteen before hostile-area teleport",
            "~stat_drain hitpoints 24 0".to_owned(),
            Proof::Stat { id: 3, min: 16 },
        )),
    }
    steps
}

fn cheat_step(name: &'static str, command: String, arm: Proof) -> Step {
    Step {
        name,
        kind: StepKind::Perform {
            send: Box::new(move |client, _| {
                matches!(
                    api::interact::cheat(client, &command),
                    client::CheatSend::Sent
                )
            }),
        },
        wait: Wait {
            arm,
            budget_ticks: 120,
        },
    }
}

fn wear_step(id: i32) -> Step {
    Step {
        name: "wear and observe melee weapon before Start",
        kind: StepKind::Perform {
            send: Box::new(move |client, snapshot| {
                matches!(
                    Interactions::new(snapshot, client).wear(id),
                    SendResult::Sent { .. }
                )
            }),
        },
        wait: Wait {
            arm: Proof::EquipmentId { id },
            budget_ticks: 120,
        },
    }
}

fn standable_neighbor(
    world: &nav::world::NavWorld,
    anchor: WorldTile,
) -> Result<WorldTile, String> {
    const OFFSETS: [(i32, i32); 8] = [
        (-1, 0),
        (1, 0),
        (0, -1),
        (0, 1),
        (-1, -1),
        (-1, 1),
        (1, -1),
        (1, 1),
    ];
    OFFSETS
        .into_iter()
        .map(|(dx, dz)| WorldTile {
            x: anchor.x + dx,
            z: anchor.z + dz,
            level: anchor.level,
        })
        .find(|tile| world.collision.standable(*tile))
        .ok_or_else(|| format!("no standable neighbor around {anchor:?}"))
}

fn start_preflight(case: Case, baseline: &Value) -> Option<String> {
    if baseline["ingame"] != json!(true) || baseline["scene_state"] != json!(2) {
        return Some("Start baseline is not an attached in-game scene".to_owned());
    }
    match case {
        Case::M1 | Case::M1HandIn => {
            if stat_pair(baseline, "attack") != Some((40, 40))
                || stat_pair(baseline, "strength") != Some((40, 40))
                || stat_pair(baseline, "defence") != Some((40, 40))
                || stat_pair(baseline, "hitpoints") != Some((40, 40))
                || item_count(baseline, BRONZE_SCIMITAR_ID) != 0
                || equipment_count(baseline, BRONZE_SCIMITAR_ID) != 1
            {
                return Some(
                    "M1 did not reach the exact Base40, worn bronze-scimitar baseline".into(),
                );
            }
            if item_count(baseline, LOBSTER_ID) != 0
                || item_count(baseline, PRAYER_POTION_4_ID) != 0
                || item_count(baseline, SUPER_ATTACK_4_ID) != 0
            {
                return Some("M1 Start baseline contains a consumable".into());
            }
            if case == Case::M1HandIn && !IMP_BEADS.iter().all(|id| item_count(baseline, *id) == 1)
            {
                return Some("staged hand-in requires all four seed beads".into());
            }
        }
        Case::M6 if item_count(baseline, COOKED_KARAMBWAN_ID) == 0 => {
            return Some("M6 NOT_STAGED: give tbwt_cooked_karambwan yielded no item".into());
        }
        Case::M2 | Case::M3 | Case::M6 => {
            let expected_prayer = if case == Case::M2 { 43 } else { 1 };
            let expected_hp = if case == Case::M6 { 16 } else { 40 };
            if stat_pair(baseline, "attack") != Some((60, 60))
                || stat_pair(baseline, "strength") != Some((60, 60))
                || stat_pair(baseline, "defence") != Some((40, 40))
                || stat_pair(baseline, "hitpoints") != Some((40, expected_hp))
                || stat_base(baseline, "prayer") != Some(expected_prayer)
                || (matches!(case, Case::M2 | Case::M6)
                    && (item_count(baseline, RUNE_SCIMITAR_ID) != 1
                        || equipment_count(baseline, RUNE_SCIMITAR_ID) != 0))
                || (case == Case::M3
                    && (item_count(baseline, RUNE_SCIMITAR_ID) != 0
                        || equipment_count(baseline, RUNE_SCIMITAR_ID) != 1))
            {
                return Some(format!(
                    "{} did not reach its exact Warlord melee baseline",
                    case.key()
                ));
            }
            let warlords = named_npcs(baseline, "khazard warlord");
            if warlords.len() != 1
                || warlords[0]["distance"]
                    .as_i64()
                    .is_none_or(|distance| distance > 9)
            {
                return Some(format!(
                    "{} natural-area interference: expected one natural, roaming Khazard Warlord within nine tiles of the stand near {WARLORD_ANCHOR:?}, observed {warlords:?}",
                    case.key()
                ));
            }
            if let Some(threats) = local_threats(baseline).filter(|rows| rows.len() > 1) {
                return Some(format!(
                    "{} natural-area interference: multiple max-9 aggressors at Start: {threats:?}",
                    case.key()
                ));
            }
            if case == Case::M2 {
                if stat_effective(baseline, "prayer") != Some(26)
                    || item_count(baseline, LOBSTER_ID) != 6
                    || item_count(baseline, PRAYER_POTION_4_ID) != 2
                    || item_count(baseline, SUPER_ATTACK_4_ID) != 1
                {
                    return Some("M2 did not seed Prayer26, lobster×6, prayer restore×2, and Super attack(4)×1".into());
                }
            } else if stat_effective(baseline, "prayer") != Some(1)
                // Both explicitly recorded seeds are replayable; no other
                // quantity is admitted. S3:583 authorizes the reserve seed.
                || if case == Case::M6 {
                    !matches!(item_count(baseline, LOBSTER_ID), 6 | 23)
                } else {
                    item_count(baseline, LOBSTER_ID) != 20
                }
                || (case == Case::M6 && item_count(baseline, COOKED_KARAMBWAN_ID) != 4)
                || item_count(baseline, PRAYER_POTION_4_ID) != 0
                || item_count(baseline, SUPER_ATTACK_4_ID) != 0
            {
                return Some(format!(
                    "{} did not seed its exact food quantities and Prayer1 with no potion",
                    case.key()
                ));
            }
        }
        Case::M4 => {
            if stat_base(baseline, "hitpoints") != Some(30)
                || stat_effective(baseline, "hitpoints") != Some(8)
                || stat_base(baseline, "prayer") != Some(1)
                || item_count(baseline, LOBSTER_ID) != 4
            {
                return Some("M4 did not reach HP30/current8, Prayer1, lobster×4".into());
            }
            if !baseline["nearby_npcs"].as_array().is_some_and(|npcs| {
                npcs.iter().any(|npc| {
                    npc["type"] == json!(152)
                        && TREE_SPAWNS.iter().any(|tile| tile_is(&npc["tile"], *tile))
                })
            }) {
                return Some(
                    "M4 static Draynor Manor nasty_tree was not present at its source-backed spawn"
                        .into(),
                );
            }
        }
        Case::M5 => {
            if stat_base(baseline, "prayer") != Some(43)
                || baseline["prayer_varps"]
                    .as_array()
                    .and_then(|rows| rows.iter().find(|row| row["index"] == json!(97)))
                    .and_then(|row| row["value"].as_i64())
                    != Some(1)
            {
                return Some("M5 did not observe Protect from Melee active before Start".into());
            }
        }
    }
    None
}

fn case_ready(case: Case, capture: &CombatCapture) -> bool {
    if capture.invalid_reason.is_some() || capture.start_baseline.is_none() {
        return false;
    }
    if matches!(case, Case::M2 | Case::M3 | Case::M6) && has_multiple_local_threats(capture) {
        return false;
    }
    match case {
        Case::M1 => m1_ready(capture),
        Case::M1HandIn => m1_hand_in_ready(capture),
        Case::M2 => m2_ready(capture),
        Case::M3 => m3_ready(capture),
        Case::M4 => m4_ready(capture),
        Case::M5 => m5_ready(capture),
        Case::M6 => m6_ready(capture),
    }
}

// This proves only the production hand-in, never natural bead acquisition.
// wizard_mizgog.rs2:3-19,39-52 requires quest start then a second talk.
fn m1_hand_in_ready(capture: &CombatCapture) -> bool {
    // wizard_mizgog.rs2:1-19,39-52: starting the quest and handing in
    // beads require separate talks, with dialogue between and after them.
    let mut talks = capture.actions.iter().enumerate().filter(|(_, action)| {
        action["kind"] == "interaction"
            && action["request"]["op"] == "npc"
            && action["request"]["name"] == "Wizard Mizgog"
            && action["request"]["action"] == "Talk-to"
            && action_wire_valid(action)
    });
    let dialogue = talks
        .next()
        .zip(talks.next())
        .is_some_and(|((first, _), (second, _))| {
            let start = &capture.actions[first + 1..second];
            let hand_in = &capture.actions[second + 1..];
            start
                .iter()
                .any(|row| row["request"]["debug"] == "ContinueDialog")
                && start
                    .iter()
                    .any(|row| row["request"]["debug"] == "Answer { option: 1 }")
                && hand_in
                    .iter()
                    .any(|row| row["request"]["debug"] == "ContinueDialog")
        });
    dialogue
        && native_interactions_wire_valid(capture)
        && capture
            .start_baseline
            .as_ref()
            .is_some_and(|baseline| IMP_BEADS.iter().all(|id| item_count(baseline, *id) == 1))
        && capture
            .statuses
            .iter()
            .any(|status| status["fields"]["stage"] == json!("imp:2"))
        && capture
            .frames
            .last()
            .is_some_and(|frame| IMP_BEADS.iter().all(|id| item_count(frame, *id) == 0))
        && !capture
            .actions
            .iter()
            .any(|action| action["kind"] == json!("other-request"))
}

fn capture_has_death(capture: &CombatCapture) -> bool {
    capture.statuses.iter().any(|status| {
        status["fields"]["combat_end"] == json!("Died")
            || integer(status, "deaths").is_some_and(|count| count > 0)
    }) || capture.start_baseline.as_ref().is_some_and(|baseline| {
        let start = baseline["snapshot_tick"]
            .as_u64()
            .or_else(|| baseline["tick"].as_u64());
        capture.frames.iter().any(|frame| {
            // A parked owner need not publish another status after its abort.
            // Known zero HP after Start is still a failed live combat cell;
            // do not hide it behind a later interference or timeout verdict.
            frame["ingame"] == json!(true)
                && frame["snapshot_tick"]
                    .as_u64()
                    .zip(start)
                    .is_some_and(|(tick, start)| tick >= start)
                && matches!(stat_pair(frame, "hitpoints"), Some((base, 0)) if base > 0)
        })
    })
}

fn case_invalid_reason(case: Case, capture: &CombatCapture) -> Option<String> {
    if matches!(case, Case::M3 | Case::M6)
        && report_with_end(capture, "Aborted(Unprotected(NoFood))").is_some()
    {
        return Some(format!(
            "{} exhausted its staged food (NoFood); not a kill proof",
            case.key()
        ));
    }
    if case == Case::M3
        && report_with_end(capture, "Killed").is_some()
        && m3_no_eligible_eat(capture)
    {
        return Some(
            "M3 INVALID: no Eat had a known prior attack deadline with D+3 <= its Eat tick".into(),
        );
    }
    if case == Case::M6
        && report_with_end(capture, "Killed").is_some()
        && m6_combo_receipt(capture)["first_eligible_decision"].is_null()
    {
        return Some(
            "M6 INVALID: no Fight eat decision had HP in 8..=16 after the first warlord onset"
                .into(),
        );
    }
    None
}

fn m1_ready(capture: &CombatCapture) -> bool {
    let Some(report) = report_with_end(capture, "Killed") else {
        return false;
    };
    let plans = batch_plans(capture);
    integer(report, "combat_target_gone_restarts").is_some_and(|count| count >= 1)
        && integer(report, "combat_prayer_doses") == Some(0)
        && integer(report, "combat_multi_op_plans") == Some(0)
        && report_multi_op_count_matches(capture, report)
        && every_killed_report_has_corpse(capture)
        && every_imp_corpse_has_outcome(capture)
        && capture
            .frames
            .iter()
            .any(|frame| IMP_BEADS.iter().all(|id| item_count(frame, *id) >= 1))
        && capture
            .statuses
            .iter()
            .any(|status| matches!(status["fields"]["stage"].as_str(), Some("imp:1" | "imp:2")))
        && !capture.actions.iter().any(is_eat_or_drink)
        && !has_prayer_clicks(capture)
        && single_action_per_native_tick(capture)
        && plans.iter().all(|plan| plan.rows.len() == 1)
        && native_interactions_wire_valid(capture)
        && batch_plan_contract(capture, &plans)
        && no_attack_after_report(capture, report)
        && !capture
            .statuses
            .iter()
            .any(|status| status["fields"]["combat_end"] == json!("Died"))
        && has_real_attack_packet(capture)
}

fn m2_ready(capture: &CombatCapture) -> bool {
    let Some(report) = report_with_end(capture, "Killed") else {
        return false;
    };
    let Some(prayer_doses) = integer(report, "combat_prayer_doses") else {
        return false;
    };
    let Some(boost_doses) = integer(report, "combat_boost_doses") else {
        return false;
    };
    let locked_ticks = integer(report, "combat_locked_ticks").unwrap_or(-1);
    let protected_hits = integer(report, "combat_hits_while_protected").unwrap_or(-1);
    let plans = batch_plans(capture);
    prayer_doses > 0
        && boost_doses > 0
        && locked_ticks == 2 * (prayer_doses + boost_doses)
        && protected_hits == 0
        && every_killed_report_has_corpse(capture)
        && first_prep_plan_ok(capture)
        && all_drinks_respect_lock(capture)
        && all_prayer_drinks_at_floor(capture, 26)
        && m2_first_floor_sip_timing_ok(capture)
        && protection_timing_ok(capture)
        && m2_restoration_runs_ok(capture, report)
        && prayer_off_plan_after_corpse(capture, report)
        && no_eat_at_zero_danger(capture, report)
        && m2_offensive_plan_ok(capture)
        && integer(report, "combat_multi_op_plans").is_some_and(|count| count >= 2)
        && report_multi_op_count_matches(capture, report)
        && plans.iter().any(|plan| {
            plan.rows.len() >= 2
                && plan_event_count(plan).is_some_and(|count| (4..=5).contains(&count))
        })
        && native_interactions_wire_valid(capture)
        && batch_plan_contract(capture, &plans)
        && no_attack_after_report(capture, report)
        && !capture
            .statuses
            .iter()
            .any(|status| status["fields"]["combat_end"] == json!("Died"))
        && has_real_attack_packet(capture)
}

fn m3_ready(capture: &CombatCapture) -> bool {
    let Some(report) = report_with_end(capture, "Killed") else {
        return false;
    };
    let Some(food) = integer(report, "combat_food") else {
        return false;
    };
    let plans = batch_plans(capture);
    let eats = capture
        .actions
        .iter()
        .filter(|action| is_eat(action))
        .collect::<Vec<_>>();
    integer(report, "combat_prayer_doses") == Some(0)
        && integer(report, "combat_boost_doses") == Some(0)
        && food >= 1
        && eats.len() as i64 == food
        && integer(report, "combat_restorations") == Some(food)
        && integer(report, "combat_multi_op_plans") == Some(food)
        && report_multi_op_count_matches(capture, report)
        && !has_prayer_clicks(capture)
        && !capture.actions.iter().any(is_drink)
        && m3_eat_plans_ok(capture)
        && m3_eat_timing_ok(capture)
        && every_killed_report_has_corpse(capture)
        && native_interactions_wire_valid(capture)
        && batch_plan_contract(capture, &plans)
        && no_attack_after_report(capture, report)
        && !capture
            .statuses
            .iter()
            .any(|status| status["fields"]["combat_end"] == json!("Died"))
        && has_real_attack_packet(capture)
}

fn m6_combo_receipt(capture: &CombatCapture) -> Value {
    // Base design §2.1: facing us with an open hit bar is live by fact,
    // even when the visible sequence is his defend animation.
    let onset = first_warlord_live_fact(capture);
    let plans = batch_plans(capture);
    let mut first_eligible = None;
    let mut combos = 0;
    let mut food_rows = 0;
    let eats = plans
        .iter()
        .filter(|plan| plan.rows.iter().any(|row| is_eat(row)))
        .map(|plan| {
            let first = plan.rows[0];
            let hp = stat_effective(&first["snapshot"], "hitpoints");
            let after_onset = onset.is_some_and(|onset| plan.tick >= onset);
            let eligible = after_onset
                && hp.is_some_and(|hp| (8..=16).contains(&hp))
                && item_count(&first["snapshot"], LOBSTER_ID) > 0
                && item_count(&first["snapshot"], COOKED_KARAMBWAN_ID) > 0;
            if eligible && first_eligible.is_none() {
                first_eligible = Some(plan.tick);
            }
            let combo = plan.rows.len() == 2 && is_eat(first) && is_eat(plan.rows[1]);
            food_rows += plan.rows.iter().filter(|row| is_eat(row)).count();
            combos += usize::from(combo);
            let ordered = held_item_id(first) == Some(i64::from(LOBSTER_ID))
                && plan.rows.len() == 2
                && if combo {
                    held_item_id(plan.rows[1]) == Some(i64::from(COOKED_KARAMBWAN_ID))
                } else {
                    is_npc_attack(plan.rows[1])
                };
            let next_output = capture
                .frames
                .iter()
                .rfind(|frame| frame["tick"].as_i64() == Some(plan.tick + 1));
            let combo_output = next_output.is_some_and(|frame| {
                stat_effective(frame, "hitpoints") == Some(40)
                    && item_count(frame, LOBSTER_ID)
                        == item_count(&first["snapshot"], LOBSTER_ID) - 1
                    && item_count(frame, COOKED_KARAMBWAN_ID)
                        == item_count(&first["snapshot"], COOKED_KARAMBWAN_ID) - 1
            });
            let silent_lock = !(plan.tick + 1..=plan.tick + 3).any(|tick| {
                capture
                    .actions
                    .iter()
                    .any(|action| action["tick"].as_i64() == Some(tick))
            });
            let restored = batch_plan_at(&plans, plan.tick + 4)
                .is_some_and(|resume| resume.rows.last().is_some_and(|row| is_npc_attack(row)));
            let row_valid = after_onset
                && hp.is_some_and(|hp| hp <= 19)
                && ordered
                && plan.rows.iter().all(|row| action_wire_valid(row))
                && (combo == eligible)
                && (!combo || (combo_output && silent_lock && restored));
            json!({
                "tick": plan.tick,
                "decision_hp": hp,
                "after_first_warlord_live_fact": after_onset,
                "eligible_combo": eligible,
                "combo": combo,
                "ordered_plan": ordered,
                "next_output_hp": next_output.and_then(|frame| stat_effective(frame, "hitpoints")),
                "both_counts_decremented_and_hp_capped": combo_output,
                "three_locked_ticks_silent": silent_lock,
                "attack_at_first_unlocked_tick": restored,
                "valid": row_valid,
            })
        })
        .collect::<Vec<_>>();
    json!({
        "first_warlord_live_fact": onset,
        "first_warlord_attack_onset": first_warlord_attack_onset(capture),
        "first_eligible_decision": first_eligible,
        "combo_count": combos,
        "food_rows": food_rows,
        "restoration_runs": eats.len(),
        "eats": eats,
    })
}

fn m6_ready(capture: &CombatCapture) -> bool {
    let Some(report) = report_with_end(capture, "Killed") else {
        return false;
    };
    let receipt = m6_combo_receipt(capture);
    let Some(combos) = receipt["combo_count"].as_i64().filter(|count| *count > 0) else {
        return false;
    };
    let Some(eats) = receipt["eats"].as_array() else {
        return false;
    };
    let plans = batch_plans(capture);
    let first_attack = first_attack_action(capture).and_then(|attack| attack["sequence"].as_u64());
    let prep_wear = first_attack.is_some_and(|attack_sequence| {
        plans.iter().any(|plan| {
            plan.rows.iter().any(|row| {
                row["sequence"]
                    .as_u64()
                    .is_some_and(|sequence| sequence < attack_sequence)
                    && row["request"]["op"] == json!("wear")
                    && action_wire_valid(row)
            })
        })
    });
    prep_wear
        && !receipt["first_eligible_decision"].is_null()
        && eats.iter().all(|eat| eat["valid"] == json!(true))
        && integer(report, "combat_food") == receipt["food_rows"].as_i64()
        && integer(report, "combat_restorations") == receipt["restoration_runs"].as_i64()
        && report_multi_op_count_matches(capture, report)
        && integer(report, "combat_locked_ticks") == Some(3 * combos)
        && integer(report, "combat_prayer_doses") == Some(0)
        && integer(report, "combat_boost_doses") == Some(0)
        && !capture.actions.iter().any(is_drink)
        && !has_prayer_clicks(capture)
        && every_killed_report_has_corpse(capture)
        && native_interactions_wire_valid(capture)
        && batch_plan_contract(capture, &plans)
        && no_attack_after_report(capture, report)
        && !capture
            .statuses
            .iter()
            .any(|status| status["fields"]["combat_end"] == json!("Died"))
        && has_real_attack_packet(capture)
}

fn m4_ready(capture: &CombatCapture) -> bool {
    let Some(report) = report_with_end(capture, "Aborted(Unattackable)") else {
        return false;
    };
    let Some(hit_tick) = m4_first_tree_hit_tick(capture) else {
        return false;
    };
    let Some(end_tick) = integer(report, "combat_evidence_tick") else {
        return false;
    };
    let plans = batch_plans(capture);
    let start_hp = capture
        .start_baseline
        .as_ref()
        .and_then(|frame| stat_effective(frame, "hitpoints"));
    let no_tree_attack = !capture.actions.iter().any(is_npc_attack_request);
    start_hp == Some(8)
        && capture
            .frames
            .iter()
            .any(|frame| stat_effective(frame, "hitpoints").is_some_and(|hp| hp < 8))
        && end_tick >= hit_tick
        && end_tick - hit_tick <= 10
        && no_tree_attack
        && m4_conditional_eat_ok(capture, report)
        && caller_walked_out(capture)
        && final_tile(capture).is_some_and(arrived_at_walk_out)
        && single_action_per_native_tick(capture)
        && native_interactions_wire_valid(capture)
        && batch_plan_contract(capture, &plans)
}

fn m5_ready(capture: &CombatCapture) -> bool {
    // M5 is interrupt hygiene, not a terminal Imp Catcher stop. After
    // ClearPrayers the Path may bank or attack again; those later ops must
    // not un-prove the random-event prefix.
    if !capture.maze_injected
        || capture.maze_owner_live_before != Some(true)
        || capture.maze_owner_live_after != Some(false)
    {
        return false;
    }
    let Some(injection) = capture
        .random_events
        .iter()
        .find(|event| event["kind"] == "Maze")
    else {
        return false;
    };
    m5_owner_preempted(capture, injection)
        && clear_prayers_before_next_operation(capture, injection)
        && m5_prefix_contract(capture, injection)
        && has_real_attack_packet(capture)
}

fn m5_prefix_contract(capture: &CombatCapture, injection: &Value) -> bool {
    let Some(injected) = injection["action_sequence_at_injection"].as_u64() else {
        return false;
    };
    let Some(next) = capture.actions.iter().find(|action| {
        action["sequence"]
            .as_u64()
            .is_some_and(|seq| seq > injected)
            && action["request"]["op"] != json!("if-button")
    }) else {
        return false;
    };
    let Some(end) = next["sequence"].as_u64() else {
        return false;
    };
    let mut prefix = CombatCapture::default();
    prefix.actions = capture
        .actions
        .iter()
        .filter(|action| action["sequence"].as_u64().is_some_and(|seq| seq <= end))
        .cloned()
        .collect();
    prefix.observations = capture.observations.clone();
    // Unknown-clock old staging cannot establish exclusivity.
    prefix.actions.iter().all(|action| {
        action["kind"] != json!("other-request")
            || (action["origin"] == json!("staging")
                && action["request"] == json!("if-button Protect from Melee")
                && action["host_tick"].as_u64().is_some()
                && accepted(action)
                && action["wire_decoded"] == json!(true)
                && wire_opcodes(action)
                    == Some(vec![i64::from(client::io::ClientProt289::IF_BUTTON.id)])
                && action["host_tick"] == action["tick"])
    }) && native_interactions_wire_valid(&prefix)
        && prefix
            .actions
            .iter()
            .filter(|action| is_npc_attack(action))
            .all(|action| action["batch"].as_u64().is_some_and(|batch| batch > 0))
        && batch_plan_contract(&prefix, &batch_plans(&prefix))
}

fn report_with_end<'a>(capture: &'a CombatCapture, end: &str) -> Option<&'a Value> {
    capture.statuses.iter().rev().find(|status| {
        status["fields"]["combat_end"]
            .as_str()
            .is_some_and(|value| value == end)
    })
}

fn has_corpse(capture: &CombatCapture, report: &Value) -> bool {
    let Some(index) = integer(report, "combat_engaged_index") else {
        return false;
    };
    let Some(evidence_tick) = integer(report, "combat_evidence_tick") else {
        return false;
    };
    let engaged_type = integer(report, "combat_engaged_npc_type");
    capture.frames.iter().any(|frame| {
        frame["tick"]
            .as_i64()
            .is_some_and(|tick| tick == evidence_tick)
            && frame["nearby_npcs"].as_array().is_some_and(|npcs| {
                npcs.iter().any(|npc| {
                    npc["index"].as_i64() == Some(index)
                        && npc["health"] == json!(0)
                        && npc["total_health"].as_i64().is_some_and(|total| total > 0)
                        && engaged_type.is_none_or(|kind| npc["type"].as_i64() == Some(kind))
                })
            })
    })
}

fn every_killed_report_has_corpse(capture: &CombatCapture) -> bool {
    let killed = capture
        .statuses
        .iter()
        .filter(|status| status["fields"]["combat_end"] == json!("Killed"))
        .collect::<Vec<_>>();
    !killed.is_empty() && killed.iter().all(|report| has_corpse(capture, report))
}

fn combat_outcomes(capture: &CombatCapture) -> Vec<&Value> {
    let mut seen = std::collections::HashSet::new();
    capture
        .statuses
        .iter()
        .map(|status| &status["fields"])
        .filter(|fields| fields["combat_end"].is_string())
        .filter(|fields| {
            seen.insert((
                fields["combat_evidence_tick"].as_i64(),
                fields["combat_evidence_sequence"].as_i64(),
                fields["combat_engaged_index"].as_i64(),
                fields["combat_end"].as_str(),
            ))
        })
        .collect()
}

// COMBAT-S3A-2 operator decision: Budget stays Budget. A same-target corpse
// may be an in-flight hit, not a Killed outcome and not an orphan. Bronze
// scimitar attackrate=4 (combat.param:76-79, scimitars.obj:1-28); melee
// npc_queue(2, damage, 0) (player_melee.rs2:47) runs next NPC phase because
// World.ts:367-369 processes NPCs before players (Npc.ts:561-575): 4 + 1.
const M1_POST_BUDGET_HIT_WINDOW: i64 = 5;

fn imp_corpse_classification(capture: &CombatCapture) -> Value {
    let outcomes = combat_outcomes(capture);
    let mut active = std::collections::HashMap::new();
    let mut corpses = Vec::new();
    for frame in &capture.frames {
        let Some(tick) = frame["tick"].as_i64() else {
            return json!({"malformed": true, "episodes": []});
        };
        let Some(npcs) = frame["nearby_npcs"].as_array() else {
            return json!({"malformed": true, "episodes": []});
        };
        let dead = npcs
            .iter()
            .filter(|npc| {
                npc["type"] == json!(708)
                    && npc["health"] == json!(0)
                    && npc["total_health"].as_i64().is_some_and(|hp| hp > 0)
            })
            .filter_map(|npc| npc["index"].as_i64())
            .collect::<std::collections::HashSet<_>>();
        active.retain(|index, (start, end)| {
            if dead.contains(index) {
                *end = tick;
                true
            } else {
                corpses.push((*index, *start, *end));
                false
            }
        });
        for index in dead {
            active.entry(index).or_insert((tick, tick));
        }
    }
    corpses.extend(
        active
            .into_iter()
            .map(|(index, (start, end))| (index, start, end)),
    );
    corpses.sort_unstable_by_key(|(index, start, _)| (*start, *index));
    let episodes = corpses
        .into_iter()
        .map(|(index, start, end)| {
            let same_target = |fields: &&Value| {
                fields["combat_engaged_index"].as_i64() == Some(index)
                    && fields["combat_engaged_npc_type"] == json!(708)
            };
            let killed = outcomes.iter().copied().filter(same_target).any(|fields| {
                fields["combat_end"] == json!("Killed")
                    && fields["combat_evidence_tick"]
                        .as_i64()
                        .is_some_and(|tick| (start..=end).contains(&tick))
            });
            let budget_tick = outcomes
                .iter()
                .copied()
                .filter(same_target)
                .filter_map(|fields| {
                    (fields["combat_end"] == json!("Budget"))
                        .then(|| fields["combat_evidence_tick"].as_i64())
                        .flatten()
                        .filter(|tick| start > *tick && start <= *tick + M1_POST_BUDGET_HIT_WINDOW)
                })
                .max();
            let class = if killed {
                "killed"
            } else if budget_tick.is_some() {
                "post_budget_kill"
            } else {
                "orphan"
            };
            json!({"index": index, "npc_type": 708, "first_corpse_tick": start,
            "last_corpse_tick": end, "class": class, "budget_tick": budget_tick})
        })
        .collect::<Vec<_>>();
    json!({
        "malformed": false,
        "post_budget_window_ticks": M1_POST_BUDGET_HIT_WINDOW,
        "killed": episodes.iter().filter(|row| row["class"] == json!("killed")).count(),
        "post_budget_kill": episodes.iter().filter(|row| row["class"] == json!("post_budget_kill")).count(),
        "orphan": episodes.iter().filter(|row| row["class"] == json!("orphan")).count(),
        "episodes": episodes,
    })
}

fn every_imp_corpse_has_outcome(capture: &CombatCapture) -> bool {
    let classification = imp_corpse_classification(capture);
    classification["malformed"] == json!(false)
        && classification["episodes"]
            .as_array()
            .is_some_and(|rows| !rows.is_empty())
        && classification["orphan"] == json!(0)
}

fn has_real_attack_packet(capture: &CombatCapture) -> bool {
    capture
        .actions
        .iter()
        .any(|action| is_npc_attack(action) && action_wire_valid(action))
}

struct BatchPlan<'a> {
    tick: i64,
    batch: u64,
    rows: Vec<&'a Value>,
}

fn batch_plans(capture: &CombatCapture) -> Vec<BatchPlan<'_>> {
    let mut plans: Vec<BatchPlan<'_>> = Vec::new();
    for action in &capture.actions {
        let Some(batch) = action["batch"].as_u64().filter(|batch| *batch != 0) else {
            continue;
        };
        let Some(tick) = action["tick"].as_i64() else {
            continue;
        };
        if let Some(plan) = plans
            .iter_mut()
            .find(|plan| plan.batch == batch && plan.tick == tick)
        {
            plan.rows.push(action);
        } else {
            plans.push(BatchPlan {
                tick,
                batch,
                rows: vec![action],
            });
        }
    }
    plans
}

fn plan_rows<'a>(capture: &'a CombatCapture, action: &Value) -> Vec<&'a Value> {
    let Some(batch) = action["batch"].as_u64().filter(|batch| *batch != 0) else {
        return Vec::new();
    };
    let Some(tick) = action["tick"].as_i64() else {
        return Vec::new();
    };
    capture
        .actions
        .iter()
        .filter(|row| row["batch"] == json!(batch) && row["tick"] == json!(tick))
        .collect()
}

fn is_npc_attack(action: &Value) -> bool {
    action["kind"] == json!("interaction")
        && action["request"]["op"] == json!("npc")
        && action["request"]["action"]
            .as_str()
            .is_some_and(|verb| verb.eq_ignore_ascii_case("attack"))
}

fn is_npc_attack_request(action: &Value) -> bool {
    action["request"]["op"] == json!("npc")
        && action["request"]["action"]
            .as_str()
            .is_some_and(|verb| verb.eq_ignore_ascii_case("attack"))
}

fn is_if_button(action: &Value) -> bool {
    action["kind"] == json!("interaction") && action["request"]["op"] == json!("if-button")
}

fn is_fight_clearing_action(action: &Value) -> bool {
    is_if_button(action)
        || is_eat_or_drink(action)
        || action["request"]["op"] == json!("wear")
        || (action["request"]["op"] == json!("set-retaliate")
            && action["request"]["on"] == json!(true))
}

fn wire_opcodes(action: &Value) -> Option<Vec<i64>> {
    action["wire_opcodes"]
        .as_array()?
        .iter()
        .map(Value::as_i64)
        .collect()
}

fn opheld_opcode(index: usize) -> Option<i64> {
    use client::io::ClientProt289;
    let opcode = match index {
        0 => ClientProt289::OPHELD1,
        1 => ClientProt289::OPHELD2,
        2 => ClientProt289::OPHELD3,
        3 => ClientProt289::OPHELD4,
        4 => ClientProt289::OPHELD5,
        _ => return None,
    };
    Some(i64::from(opcode.id))
}

fn held_opcode(action: &Value) -> Option<i64> {
    let request = &action["request"];
    let is_held = request["op"] == json!("held");
    if !is_held && request["op"] != json!("wear") {
        return None;
    }
    let requested_action = if is_held {
        request["action"].as_str()?
    } else {
        "wield"
    };
    let requested_slot = if is_held {
        request["slot"].as_i64()
    } else {
        None
    };
    let inventory = action["snapshot"]["inventory"].as_array()?;
    let item = inventory.iter().find(|item| {
        requested_slot.is_none_or(|slot| item["slot"] == json!(slot))
            && item["actions"].as_array().is_some_and(|options| {
                options.iter().any(|option| {
                    option.as_str().is_some_and(|label| {
                        if is_held {
                            label.eq_ignore_ascii_case(requested_action)
                        } else {
                            label.eq_ignore_ascii_case("wield")
                                || label.eq_ignore_ascii_case("wear")
                        }
                    })
                })
            })
    })?;
    let options = item["actions"].as_array()?;
    let index = options.iter().position(|option| {
        option.as_str().is_some_and(|label| {
            if is_held {
                label.eq_ignore_ascii_case(requested_action)
            } else {
                label.eq_ignore_ascii_case("wield") || label.eq_ignore_ascii_case("wear")
            }
        })
    })?;
    opheld_opcode(index)
}

// Legacy receipts predate structured Obj/OpenStand capture. Parse the complete
// emitted Debug grammar, not a substring (unknown debug requests fail closed).
fn legacy_caller_opcode(request: &Value) -> Option<i64> {
    use client::io::ClientProt289;
    let debug = request["debug"].as_str()?;
    let (mut rest, bank) = if let Some(rest) = debug.strip_prefix("OpenStand { ") {
        (rest, true)
    } else {
        (debug.strip_prefix("Obj { ")?, false)
    };
    for (field, maximum) in [("x: ", 16383), ("z: ", 16383), ("level: ", 3)] {
        let (number, after) = rest.strip_prefix(field)?.split_once(", ")?;
        let coordinate = number.parse::<i32>().ok()?;
        if !(0..=maximum).contains(&coordinate) {
            return None;
        }
        rest = after;
    }
    if bank {
        (rest == "kind: \"booth\", name: Some(\"Bank booth\"), stand_op: Some(2), choose: None }")
            .then_some(i64::from(ClientProt289::OPLOC2.id))
    } else {
        ["Red bead", "Yellow bead", "Black bead", "White bead"]
            .iter()
            .any(|name| rest == format!("name: Some(\"{name}\"), action: \"Take\" }}"))
            .then_some(i64::from(ClientProt289::OPOBJ3.id))
    }
}

fn action_wire_valid(action: &Value) -> bool {
    use client::io::{ClientProt, ClientProt289};
    if !accepted(action) || action["wire_decoded"] != json!(true) {
        return false;
    }
    let Some(actual) = wire_opcodes(action) else {
        return false;
    };
    let opcode = |prot: ClientProt| i64::from(prot.id);
    let targeted = |wire: &[i64], expected| {
        wire == [expected] || wire == [opcode(ClientProt289::MOVE_OPCLICK), expected]
    };
    // S3:355 checks declared user events. The client OP_LOC2 arm
    // (client.rs:3895-3902) can prepend this exact anti-cheat packet.
    // Do not strip arbitrary packets or accept this prefix on other ops.
    let bank_wire = || {
        let expected = opcode(ClientProt289::OPLOC2);
        targeted(&actual, expected)
            || (actual.first() == Some(&opcode(ClientProt289::ANTICHEAT_OPLOGIC1))
                && targeted(&actual[1..], expected))
    };
    match action["request"]["op"].as_str() {
        Some("npc") if is_npc_attack(action) => {
            let attack = opcode(ClientProt289::OPNPC2);
            actual == [attack] || actual == [opcode(ClientProt289::MOVE_OPCLICK), attack]
        }
        Some("npc") if action["request"]["action"] == json!("Talk-to") => {
            // wizard_mizgog.rs2:1-19,39-52 uses opnpc1 and dialogue.
            // Resolve the literal action slot from the captured actor instead
            // of treating every Talk-to (or arbitrary NPC op) as OPNPC1.
            action["request"]["index"].as_i64().is_some_and(|index| {
                action["snapshot"]["nearby_npcs"]
                    .as_array()
                    .and_then(|rows| {
                        rows.iter().find(|row| {
                            row["index"].as_i64() == Some(index)
                                && row["name"] == action["request"]["name"]
                        })
                    })
                    .and_then(|row| row["actions"].as_array())
                    .and_then(|actions| actions.iter().position(|name| name == "Talk-to"))
                    .and_then(|slot| {
                        [
                            ClientProt289::OPNPC1,
                            ClientProt289::OPNPC2,
                            ClientProt289::OPNPC3,
                            ClientProt289::OPNPC4,
                            ClientProt289::OPNPC5,
                        ]
                        .get(slot)
                        .copied()
                    })
                    .is_some_and(|expected| targeted(&actual, opcode(expected)))
            })
        }
        Some("held" | "wear") => held_opcode(action).is_some_and(|expected| actual == [expected]),
        Some("if-button" | "set-retaliate") => actual == [opcode(ClientProt289::IF_BUTTON)],
        Some("obj") if action["request"]["action"] == json!("Take") => {
            let expected = opcode(ClientProt289::OPOBJ3);
            actual == [expected] || actual == [opcode(ClientProt289::MOVE_OPCLICK), expected]
        }
        Some("open-stand")
            if action["request"]["kind"] == json!("booth")
                && action["request"]["stand_op"] == json!(2) =>
        {
            bank_wire()
        }
        None => {
            let debug = action["request"]["debug"].as_str().unwrap_or_default();
            // api/interact.rs:636-648,771-798: exact accepted dialogue
            // operations; unknown Debug rows still fail closed.
            if debug == "ContinueDialog" {
                actual == [opcode(ClientProt289::RESUME_PAUSEBUTTON)]
            } else if debug
                .strip_prefix("Answer { option: ")
                .and_then(|value| value.strip_suffix(" }"))
                .and_then(|value| value.parse::<i32>().ok())
                .is_some_and(|option| option > 0)
            {
                actual == [opcode(ClientProt289::IF_BUTTON)]
            } else {
                legacy_caller_opcode(&action["request"]).is_some_and(|expected| {
                    if expected == opcode(ClientProt289::OPLOC2) {
                        bank_wire()
                    } else {
                        targeted(&actual, expected)
                    }
                })
            }
        }
        _ => false,
    }
}

fn native_interactions_wire_valid(capture: &CombatCapture) -> bool {
    capture
        .actions
        .iter()
        .filter(|action| action["kind"] == json!("interaction"))
        .all(action_wire_valid)
}

fn plan_event_count(plan: &BatchPlan<'_>) -> Option<usize> {
    plan.rows.iter().try_fold(0usize, |count, action| {
        Some(count + wire_opcodes(action)?.len())
    })
}

fn batch_plan_contract(capture: &CombatCapture, plans: &[BatchPlan<'_>]) -> bool {
    plans.iter().all(|plan| {
        let Some(first) = plan.rows.first() else {
            return false;
        };
        let Some(first_request) = first["request_id"].as_u64() else {
            return false;
        };
        let same_tick_actions = capture
            .actions
            .iter()
            .filter(|action| action["tick"] == json!(plan.tick))
            .collect::<Vec<_>>();
        let same_tick_plans = plans.iter().filter(|other| other.tick == plan.tick).count();
        let consecutive_ids = plan.rows.iter().enumerate().all(|(offset, row)| {
            row["batch"] == json!(plan.batch)
                && row["request_id"].as_u64() == Some(first_request + offset as u64)
        });
        first_request == plan.batch
            && same_tick_plans == 1
            && same_tick_actions.len() == plan.rows.len()
            && same_tick_actions.iter().all(|action| {
                action["kind"] == json!("interaction") && action["batch"] == json!(plan.batch)
            })
            && capture.observations.iter().any(|observation| {
                observation["tick"] == json!(plan.tick) && observation["exclusive"] == json!(true)
            })
            && consecutive_ids
            && plan.rows.iter().all(|action| action_wire_valid(action))
            && plan_event_count(plan).is_some_and(|events| (1..=5).contains(&events))
    })
}

fn single_action_per_native_tick(capture: &CombatCapture) -> bool {
    let mut all_requests = std::collections::HashMap::<i64, usize>::new();
    let mut native_interactions = std::collections::HashMap::<i64, usize>::new();
    for action in &capture.actions {
        let Some(tick) = action["tick"].as_i64() else {
            return false;
        };
        *all_requests.entry(tick).or_default() += 1;
        if action["kind"] == json!("interaction") {
            *native_interactions.entry(tick).or_default() += 1;
        }
    }
    native_interactions
        .iter()
        .all(|(tick, count)| *count == 1 && all_requests.get(tick) == Some(count))
}

fn report_multi_op_count_matches(capture: &CombatCapture, report: &Value) -> bool {
    let observed = batch_plans(capture)
        .iter()
        .filter(|plan| plan.rows.len() >= 2)
        .count() as i64;
    integer(report, "combat_multi_op_plans") == Some(observed)
}

fn prayer_fact<'a>(capture: &'a CombatCapture, name: &str) -> Option<&'a Value> {
    capture.prayer_facts.iter().find(|prayer| {
        prayer["name"]
            .as_str()
            .is_some_and(|value| value.eq_ignore_ascii_case(name))
    })
}

fn prayer_component(capture: &CombatCapture, name: &str) -> Option<i64> {
    prayer_fact(capture, name)?["button_com"].as_i64()
}

fn prayer_varp_for_name(capture: &CombatCapture, name: &str) -> Option<i64> {
    prayer_fact(capture, name)?["varp"].as_i64()
}

fn prayer_action(action: &Value, component: i64) -> bool {
    is_if_button(action) && action["request"]["component_id"] == json!(component)
}

fn has_prayer_clicks(capture: &CombatCapture) -> bool {
    capture.actions.iter().any(|action| {
        action["request"]["op"] == json!("if-button")
            && capture
                .prayer_facts
                .iter()
                .any(|fact| action["request"]["component_id"] == fact["button_com"])
    })
}

fn prayer_echo_after(capture: &CombatCapture, varp: i64, action: &Value) -> bool {
    let Some(action_tick) = action["tick"].as_i64() else {
        return false;
    };
    capture.frames.iter().any(|frame| {
        frame["tick"]
            .as_i64()
            .is_some_and(|tick| tick > action_tick)
            && prayer_varp(frame, varp) == Some(1)
    })
}

fn held_item_id(action: &Value) -> Option<i64> {
    let slot = action["request"]["slot"].as_i64()?;
    action["snapshot"]["inventory"]
        .as_array()?
        .iter()
        .find(|item| item["slot"] == json!(slot))?["id"]
        .as_i64()
}

fn is_prayer_drink(action: &Value) -> bool {
    is_drink(action)
        && (held_item_id(action) == Some(PRAYER_POTION_4_ID as i64)
            || action["request"]["name"]
                .as_str()
                .is_some_and(|name| name.to_ascii_lowercase().contains("prayer")))
}

fn batch_plan_at<'a>(plans: &'a [BatchPlan<'a>], tick: i64) -> Option<&'a BatchPlan<'a>> {
    let mut matching = plans.iter().filter(|plan| plan.tick == tick);
    let plan = matching.next()?;
    matching.next().is_none().then_some(plan)
}

fn last_is_attack_or_drink(plan: &BatchPlan<'_>) -> bool {
    plan.rows
        .last()
        .is_some_and(|action| is_npc_attack(action) || is_drink(action))
}

fn all_drinks_respect_lock(capture: &CombatCapture) -> bool {
    let plans = batch_plans(capture);
    let drinks = capture
        .actions
        .iter()
        .filter(|action| is_drink(action))
        .collect::<Vec<_>>();
    !drinks.is_empty()
        && drinks.iter().all(|drink| {
            let Some(tick) = drink["tick"].as_i64() else {
                return false;
            };
            let rows = plan_rows(capture, drink);
            rows.last().is_some_and(|last| std::ptr::eq(*last, *drink))
                && !capture.actions.iter().any(|action| {
                    action["tick"] == json!(tick + 1) || action["tick"] == json!(tick + 2)
                })
                && batch_plan_at(&plans, tick + 3).is_some_and(last_is_attack_or_drink)
        })
}

fn all_prayer_drinks_at_floor(capture: &CombatCapture, floor: i64) -> bool {
    let prayer_drinks = capture
        .actions
        .iter()
        .filter(|action| is_prayer_drink(action))
        .collect::<Vec<_>>();
    !prayer_drinks.is_empty()
        && prayer_drinks.iter().all(|action| {
            stat_effective(&action["snapshot"], "prayer").is_some_and(|points| points <= floor)
                && action_wire_valid(action)
                && plan_rows(capture, action)
                    .last()
                    .is_some_and(|last| std::ptr::eq(*last, *action))
        })
}

fn first_attack_action(capture: &CombatCapture) -> Option<&Value> {
    capture.actions.iter().find(|action| is_npc_attack(action))
}

fn local_attack_onsets(capture: &CombatCapture, engaged_index: i64) -> Vec<i64> {
    // Independent of CombatTables/style_seq: selected scimitars.obj (bronze and
    // rune) names human_sword_stab/slash; selected seq.pack resolves386/390.
    // Defend388 and eat829 are not attacks. Design§3.3(b) separates our
    // already-observed output from the next input phase.
    let mut previous_animation = None;
    let mut previous_frame = None;
    let mut onsets = Vec::new();
    for frame in &capture.frames {
        let local = &frame["local_player"];
        let animation = local["animation"].as_i64();
        let animation_frame = local["animation_frame"].as_i64();
        let targets_engaged = local["target"]["kind"] == json!("Npc")
            && local["target"]["index"] == json!(engaged_index);
        let scimitar_equipped = frame["equipment"].as_array().is_some_and(|equipment| {
            equipment
                .iter()
                .any(|item| matches!(item["id"].as_i64(), Some(1321 | 1333)))
        });
        let restarted = animation_frame
            .zip(previous_frame)
            .is_some_and(|(current, previous)| current < previous);
        if targets_engaged
            && scimitar_equipped
            && matches!(animation, Some(386 | 390))
            && (animation != previous_animation || restarted)
        {
            if let Some(tick) = frame["tick"].as_i64() {
                onsets.push(tick);
            }
        }
        previous_animation = animation;
        previous_frame = animation_frame;
    }
    onsets
}

fn first_warlord_live_fact(capture: &CombatCapture) -> Option<i64> {
    capture.frames.iter().find_map(|frame| {
        let self_slot = frame["self_slot"].as_i64()?;
        frame["nearby_npcs"]
            .as_array()?
            .iter()
            .any(|npc| {
                npc["type"] == json!(477)
                    && npc["in_combat"] == json!(true)
                    && npc["target"]["kind"] == json!("Player")
                    && npc["target"]["index"] == json!(self_slot)
            })
            .then(|| frame["tick"].as_i64())
            .flatten()
    })
}

fn first_warlord_attack_onset(capture: &CombatCapture) -> Option<i64> {
    let mut previous_animations = std::collections::HashMap::<i64, Option<i64>>::new();
    for frame in &capture.frames {
        let self_slot = frame["self_slot"].as_i64()?;
        let Some(npcs) = frame["nearby_npcs"].as_array() else {
            continue;
        };
        for npc in npcs {
            if !npc["name"]
                .as_str()
                .is_some_and(|name| name.eq_ignore_ascii_case("khazard warlord"))
            {
                continue;
            }
            let Some(index) = npc["index"].as_i64() else {
                continue;
            };
            let animation = npc["animation"].as_i64();
            let previous = previous_animations.insert(index, animation).flatten();
            if npc["in_combat"] == json!(true)
                && npc["target"]["kind"] == json!("Player")
                && npc["target"]["index"] == json!(self_slot)
                // gnome_battlefield.npc attack_anim=human_blunt_pound (401);
                // human_blunt_def (404) is not an attack onset.
                && animation == Some(401)
                && animation != previous
            {
                return frame["tick"].as_i64();
            }
        }
    }
    None
}

fn first_prayer_action<'a>(capture: &'a CombatCapture, name: &str) -> Option<&'a Value> {
    let component = prayer_component(capture, name)?;
    capture
        .actions
        .iter()
        .find(|action| prayer_action(action, component))
}

fn protect_plan_ends_with_terminal(capture: &CombatCapture) -> bool {
    let Some(component) = prayer_component(capture, "Protect from Melee") else {
        return false;
    };
    let Some(varp) = prayer_varp_for_name(capture, "Protect from Melee") else {
        return false;
    };
    capture
        .actions
        .iter()
        .filter(|action| prayer_action(action, component))
        .all(|action| match prayer_varp(&action["snapshot"], varp) {
            Some(1) => true, // WindDown deactivation owes no restoring attack.
            Some(0) => plan_rows(capture, action)
                .last()
                .is_some_and(|last| is_npc_attack(last) || is_drink(last)),
            _ => false,
        })
}

fn protection_timing_ok(capture: &CombatCapture) -> bool {
    let Some(onset_tick) = first_warlord_attack_onset(capture) else {
        return false;
    };
    let Some(protect_action) = first_prayer_action(capture, "Protect from Melee") else {
        return false;
    };
    let Some(protect_tick) = protect_action["tick"].as_i64() else {
        return false;
    };
    let Some(protect_varp) = prayer_varp_for_name(capture, "Protect from Melee") else {
        return false;
    };
    accepted(protect_action)
        && protect_tick <= onset_tick + 2
        && capture.frames.iter().any(|frame| {
            frame["tick"] == json!(onset_tick + 2) && prayer_varp(frame, protect_varp) == Some(1)
        })
        && protect_plan_ends_with_terminal(capture)
        && prayer_echo_after(capture, protect_varp, protect_action)
}

fn prayer_off_plan_after_corpse(capture: &CombatCapture, report: &Value) -> bool {
    let Some(index) = integer(report, "combat_engaged_index") else {
        return false;
    };
    let Some(corpse) = capture.frames.iter().find(|frame| {
        frame["nearby_npcs"].as_array().is_some_and(|npcs| {
            npcs.iter().any(|npc| {
                npc["index"].as_i64() == Some(index)
                    && npc["health"] == json!(0)
                    && npc["total_health"].as_i64().is_some_and(|total| total > 0)
            })
        })
    }) else {
        return false;
    };
    let Some(corpse_tick) = corpse["tick"].as_i64() else {
        return false;
    };
    let active_components = capture
        .prayer_facts
        .iter()
        .filter(|fact| {
            fact["varp"]
                .as_i64()
                .is_some_and(|varp| prayer_varp(corpse, varp) == Some(1))
        })
        .filter_map(|fact| fact["button_com"].as_i64())
        .collect::<Vec<_>>();
    if active_components.is_empty() {
        return all_prayer_bits_off(corpse);
    }
    let lock_end = capture
        .actions
        .iter()
        .filter(|action| is_drink(action))
        .filter_map(|action| action["tick"].as_i64())
        .map(|tick| tick + 3)
        .filter(|end| *end > corpse_tick)
        .max()
        .unwrap_or(corpse_tick);
    let plans = batch_plans(capture);
    let off_plan = plans.iter().find(|plan| {
        (lock_end..=lock_end + 2).contains(&plan.tick)
            && active_components.iter().all(|component| {
                plan.rows
                    .iter()
                    .any(|action| prayer_action(action, *component) && accepted(action))
            })
    });
    off_plan.is_some_and(|plan| {
        capture.frames.iter().any(|frame| {
            frame["tick"]
                .as_i64()
                .is_some_and(|tick| tick > plan.tick && (lock_end..=lock_end + 2).contains(&tick))
                && all_prayer_bits_off(frame)
        })
    })
}

fn no_eat_at_zero_danger(capture: &CombatCapture, report: &Value) -> bool {
    integer(report, "combat_food") == Some(0)
        && !capture.actions.iter().any(is_eat)
        && capture
            .actions
            .iter()
            .filter(|action| is_if_button(action))
            .all(|action| {
                let component = action["request"]["component_id"].as_i64();
                let protects = prayer_component(capture, "Protect from Melee");
                if component != protects {
                    return true;
                }
                plan_rows(capture, action).iter().all(|row| !is_eat(row))
            })
}

fn first_prep_plan_ok(capture: &CombatCapture) -> bool {
    let plans = batch_plans(capture);
    let Some(first_attack) = first_attack_action(capture) else {
        return false;
    };
    let Some(first_attack_tick) = first_attack["tick"].as_i64() else {
        return false;
    };
    let Some(prep) = plans.iter().find(|plan| {
        plan.rows
            .iter()
            .any(|action| action["request"]["op"] == json!("wear"))
            && plan.tick < first_attack_tick
    }) else {
        return false;
    };
    let Some(drink) = prep.rows.get(1).copied() else {
        return false;
    };
    prep.rows.len() == 2
        && prep.rows[0]["request"]["op"] == json!("wear")
        && is_drink(drink)
        && held_item_id(drink) == Some(SUPER_ATTACK_4_ID as i64)
        && action_wire_valid(prep.rows[0])
        && action_wire_valid(drink)
        && drink["sequence"]
            .as_u64()
            .zip(prep.rows.last().and_then(|row| row["sequence"].as_u64()))
            .is_some_and(|(drink_sequence, last_sequence)| drink_sequence == last_sequence)
        && first_attack_tick >= prep.tick + 3
}

fn m2_restoration_runs_ok(capture: &CombatCapture, report: &Value) -> bool {
    let Some(first_attack) = first_attack_action(capture) else {
        return false;
    };
    let Some(first_attack_sequence) = first_attack["sequence"].as_u64() else {
        return false;
    };
    let Some(engaged_index) = integer(report, "combat_engaged_index") else {
        return false;
    };
    let Some(corpse_tick) = capture.frames.iter().find_map(|frame| {
        frame["nearby_npcs"].as_array().and_then(|npcs| {
            npcs.iter()
                .any(|npc| {
                    npc["index"].as_i64() == Some(engaged_index)
                        && npc["health"] == json!(0)
                        && npc["total_health"].as_i64().is_some_and(|total| total > 0)
                })
                .then(|| frame["tick"].as_i64())
                .flatten()
        })
    }) else {
        return false;
    };
    let plans = batch_plans(capture);
    let mut expected_runs = 0i64;
    let mut restore_owed = false;
    for plan in &plans {
        let first_sequence = plan.rows.first().and_then(|row| row["sequence"].as_u64());
        if first_sequence.is_none_or(|sequence| sequence <= first_attack_sequence)
            || plan.tick >= corpse_tick
        {
            continue;
        }
        let clearing = plan
            .rows
            .iter()
            .any(|action| is_fight_clearing_action(action));
        let attacks = plan
            .rows
            .iter()
            .filter(|action| is_npc_attack(action))
            .collect::<Vec<_>>();
        if clearing {
            if !restore_owed {
                expected_runs += 1;
            }
            restore_owed = true;
            if !attacks.is_empty() {
                if attacks.len() != 1 || !plan.rows.last().is_some_and(|last| is_npc_attack(last)) {
                    return false;
                }
                restore_owed = false;
            } else if plan.rows.last().is_some_and(|last| is_drink(last)) {
                if corpse_tick > plan.tick + 3 {
                    let Some(next) = batch_plan_at(&plans, plan.tick + 3) else {
                        return false;
                    };
                    if !last_is_attack_or_drink(next) {
                        return false;
                    }
                }
            } else {
                return false;
            }
        } else {
            for attack in attacks {
                if !restore_owed && !attack_is_mismatch_or_stale(capture, attack, engaged_index) {
                    return false;
                }
                restore_owed = false;
            }
        }
    }
    integer(report, "combat_restorations") == Some(expected_runs)
}

fn attack_is_mismatch_or_stale(
    capture: &CombatCapture,
    action: &Value,
    engaged_index: i64,
) -> bool {
    let local_target = &action["snapshot"]["local_player"]["target"];
    let mismatch =
        local_target["kind"] != json!("Npc") || local_target["index"] != json!(engaged_index);
    if mismatch {
        return true;
    }
    let Some(tick) = action["tick"].as_i64() else {
        return false;
    };
    let onsets = local_attack_onsets(capture, engaged_index);
    onsets
        .into_iter()
        .filter(|onset| *onset <= tick)
        .max()
        .is_some_and(|last_onset| tick - last_onset >= 5)
        || capture
            .actions
            .iter()
            .filter(|candidate| is_npc_attack(candidate))
            .filter_map(|candidate| candidate["tick"].as_i64())
            .filter(|previous| *previous < tick)
            .max()
            .is_some_and(|previous_attack| tick - previous_attack >= 5)
}

fn m2_offensive_plan_ok(capture: &CombatCapture) -> bool {
    let Some(strength_button) = prayer_component(capture, "Ultimate Strength") else {
        return false;
    };
    let Some(reflexes_button) = prayer_component(capture, "Incredible Reflexes") else {
        return false;
    };
    let Some(protect_button) = prayer_component(capture, "Protect from Melee") else {
        return false;
    };
    let Some(strength_varp) = prayer_varp_for_name(capture, "Ultimate Strength") else {
        return false;
    };
    let Some(reflexes_varp) = prayer_varp_for_name(capture, "Incredible Reflexes") else {
        return false;
    };
    let Some(floor_sip) = capture
        .actions
        .iter()
        .find(|action| is_prayer_drink(action))
    else {
        return false;
    };
    let Some(floor_sip_tick) = floor_sip["tick"].as_i64() else {
        return false;
    };
    let plans = batch_plans(capture);
    let Some(plan) = batch_plan_at(&plans, floor_sip_tick + 3) else {
        return false;
    };
    let strength = plan
        .rows
        .iter()
        .find(|action| prayer_action(action, strength_button));
    let reflexes = plan
        .rows
        .iter()
        .find(|action| prayer_action(action, reflexes_button));
    let (Some(strength), Some(reflexes)) = (strength, reflexes) else {
        return false;
    };
    let last_is_attack = plan.rows.last().is_some_and(|action| is_npc_attack(action));
    accepted(strength)
        && accepted(reflexes)
        && strength["sequence"]
            .as_u64()
            .zip(reflexes["sequence"].as_u64())
            .is_some_and(|(strength, reflexes)| strength < reflexes)
        && prayer_echo_after(capture, strength_varp, strength)
        && prayer_echo_after(capture, reflexes_varp, reflexes)
        && last_is_attack
        && !plan
            .rows
            .iter()
            .any(|action| is_eat(action) || prayer_action(action, protect_button))
        && plan_event_count(plan).is_some_and(|events| (4..=5).contains(&events))
}

fn m2_first_floor_sip_timing_ok(capture: &CombatCapture) -> bool {
    let Some(protect) = first_prayer_action(capture, "Protect from Melee") else {
        return false;
    };
    let Some(sip) = capture
        .actions
        .iter()
        .find(|action| is_prayer_drink(action))
    else {
        return false;
    };
    let Some(protect_tick) = protect["tick"].as_i64() else {
        return false;
    };
    let Some(sip_tick) = sip["tick"].as_i64() else {
        return false;
    };
    let protect_rows = plan_rows(capture, protect);
    let sip_rows = plan_rows(capture, sip);
    let same_plan = !protect_rows.is_empty()
        && protect["batch"] == sip["batch"]
        && protect["tick"] == sip["tick"]
        && sip_rows.last().is_some_and(|last| std::ptr::eq(*last, sip));
    if same_plan {
        return accepted(protect) && action_wire_valid(sip);
    }
    let Some(protect_varp) = prayer_varp_for_name(capture, "Protect from Melee") else {
        return false;
    };
    let Some(echo_tick) = capture
        .frames
        .iter()
        .filter(|frame| {
            frame["tick"]
                .as_i64()
                .is_some_and(|tick| tick > protect_tick)
                && prayer_varp(frame, protect_varp) == Some(1)
        })
        .filter_map(|frame| frame["tick"].as_i64())
        .min()
    else {
        return false;
    };
    let plans = batch_plans(capture);
    let first_plan_after_echo = plans
        .iter()
        .filter(|plan| plan.tick > echo_tick)
        .min_by_key(|plan| plan.tick);
    accepted(protect)
        && sip_tick >= echo_tick
        && sip_tick <= echo_tick + 3
        && first_plan_after_echo
            .is_some_and(|plan| plan.tick == sip_tick && plan.batch == sip["batch"])
        && sip_rows.last().is_some_and(|last| std::ptr::eq(*last, sip))
        && action_wire_valid(sip)
}

fn m3_eat_plans_ok(capture: &CombatCapture) -> bool {
    capture
        .actions
        .iter()
        .filter(|action| is_eat(action))
        .all(|eat| {
            let Some(held) = held_opcode(eat) else {
                return false;
            };
            let plan = plan_rows(capture, eat);
            let Some(attack) = plan.get(1).copied() else {
                return false;
            };
            let Some(tick) = eat["tick"].as_i64() else {
                return false;
            };
            let packets = plan
                .iter()
                .filter_map(|action| wire_opcodes(action))
                .flatten()
                .collect::<Vec<_>>();
            plan.len() == 2
                && is_eat(plan[0])
                && std::ptr::eq(plan[0], eat)
                && is_npc_attack(attack)
                && held_item_id(eat) == Some(LOBSTER_ID as i64)
                && stat_effective(&eat["snapshot"], "hitpoints")
                    .is_some_and(|hitpoints| hitpoints <= 19)
                && action_wire_valid(eat)
                && accepted(attack)
                && packets
                    == [
                        held,
                        i64::from(client::io::ClientProt289::MOVE_OPCLICK.id),
                        i64::from(client::io::ClientProt289::OPNPC2.id),
                    ]
                && !capture.actions.iter().any(|action| {
                    is_eat(action)
                        && action["tick"]
                            .as_i64()
                            .is_some_and(|other_tick| (tick + 1..=tick + 2).contains(&other_tick))
                })
        })
}

fn m3_timing_receipt(capture: &CombatCapture) -> Value {
    let report = report_with_end(capture, "Killed");
    let engaged_index = report.and_then(|report| integer(report, "combat_engaged_index"));
    let onsets = engaged_index
        .map(|index| local_attack_onsets(capture, index))
        .unwrap_or_default();
    let eat_ticks = capture
        .actions
        .iter()
        .filter(|action| is_eat(action))
        .filter_map(|eat| eat["tick"].as_i64())
        .collect::<Vec<_>>();
    let eats = capture
        .actions
        .iter()
        .filter(|action| is_eat(action))
        .map(|eat| {
            let tick = eat["tick"].as_i64();
            // Design§3.3(b),§3.4 and §5.1 M3 P4, independently from content:
            // player_melee.rs2:52-56 writes map_clock+4; consume.rs2:129-130
            // adds3 to that existing clock on every ordinary Eat. A swing
            // observed at E belongs to the prior output, before Eat input E.
            let previous_onset =
                tick.and_then(|tick| onsets.iter().copied().filter(|onset| *onset <= tick).max());
            let old_deadline = previous_onset.zip(tick).map(|(onset, tick)| {
                onset
                    + 4
                    + 3 * eat_ticks
                        .iter()
                        .filter(|eat| onset <= **eat && **eat < tick)
                        .count() as i64
            });
            let mut expected_onset = tick
                .zip(old_deadline)
                .map(|(eat_tick, deadline)| (eat_tick + 1).max(deadline + 3));
            let eligible = tick
                .zip(old_deadline)
                .is_some_and(|(eat_tick, deadline)| deadline + 3 <= eat_tick + 1);
            let observed_onset =
                tick.and_then(|tick| onsets.iter().copied().filter(|onset| *onset > tick).min());
            let mut intervening_eats = 0;
            if let Some(tick) = tick {
                for later_eat in eat_ticks.iter().copied().filter(|eat| *eat > tick) {
                    if observed_onset.is_some_and(|onset| later_eat >= onset) {
                        break;
                    }
                    // Input at the expected observation tick cannot undo a
                    // swing the preceding output was already obliged to show.
                    if let Some(expected) = expected_onset.filter(|expected| later_eat < *expected)
                    {
                        expected_onset = Some(expected + 3);
                        intervening_eats += 1;
                    } else {
                        break;
                    }
                }
            }
            json!({
                "eat_sequence": eat["sequence"],
                "eat_tick": tick,
                "previous_swing_onset": previous_onset,
                "old_attack_deadline": old_deadline,
                "intervening_eats_before_next_swing": intervening_eats,
                "deadline_plus_three_was_due_at_eat": eligible,
                "expected_next_swing_onset": expected_onset,
                "observed_next_swing_onset": observed_onset,
                "exact": expected_onset.is_some() && expected_onset == observed_onset,
            })
        })
        .collect::<Vec<_>>();
    json!({
        "swing_rate_ticks": 4,
        "observation_delivery_offset_ticks": 1,
        "attack_sequences": [386, 390],
        "authority": "R4 design§3.3(b),§3.4,§5.1 M3 P4; selected scimitars.obj,seq.pack,player_melee.rs2:52-56,consume.rs2:129-130",
        "eligible_eat_count": eats.iter().filter(|eat| eat["deadline_plus_three_was_due_at_eat"] == json!(true)).count(),
        "eats": eats,
    })
}

fn m3_eat_timing_ok(capture: &CombatCapture) -> bool {
    let receipt = m3_timing_receipt(capture);
    receipt["eats"].as_array().is_some_and(|eats| {
        !eats.is_empty()
            && eats
                .iter()
                .any(|eat| eat["deadline_plus_three_was_due_at_eat"] == json!(true))
            && eats.iter().all(|eat| eat["exact"] == json!(true))
    })
}

fn m3_no_eligible_eat(capture: &CombatCapture) -> bool {
    m3_timing_receipt(capture)["eligible_eat_count"] == json!(0)
}

fn m4_first_tree_hit_tick(capture: &CombatCapture) -> Option<i64> {
    capture.frames.iter().find_map(|frame| {
        let self_slot = frame["self_slot"].as_i64()?;
        let tree_targets_player = frame["nearby_npcs"].as_array()?.iter().any(|npc| {
            npc["type"] == json!(152)
                && npc["animation"] == json!(73)
                && npc["target"]["kind"] == json!("Player")
                && npc["target"]["index"] == json!(self_slot)
        });
        tree_targets_player
            .then(|| frame["tick"].as_i64())
            .flatten()
    })
}

fn m4_conditional_eat_receipt(capture: &CombatCapture, report: &Value) -> Value {
    let ready_tick = integer(report, "combat_evidence_tick");
    // The measured window begins at native Start, not login. An unloaded
    // pre-Start stat row (base HP 0) is not the cell's conditional eat input.
    let start_tick = capture.start_baseline.as_ref().and_then(|baseline| {
        baseline["snapshot_tick"]
            .as_u64()
            .or_else(|| baseline["tick"].as_u64())
    });
    let low_hp_frames = capture
        .frames
        .iter()
        .filter(|frame| {
            frame["ingame"] == true
                && matches!(stat_pair(frame, "hitpoints"), Some((base, hp)) if base > 0 && hp <= 7)
                && frame["snapshot_tick"]
                    .as_u64()
                    .zip(start_tick)
                    .is_some_and(|(tick, start)| tick >= start)
                && ready_tick
                    .is_some_and(|end| frame["tick"].as_i64().is_some_and(|tick| tick < end))
        })
        .collect::<Vec<_>>();
    let first_low_hp_tick = low_hp_frames
        .iter()
        .filter_map(|frame| frame["tick"].as_i64())
        .min();
    let eat_rows = capture
        .actions
        .iter()
        .filter(|action| is_eat(action))
        .filter(|action| {
            first_low_hp_tick.is_some_and(|low| {
                action["tick"]
                    .as_i64()
                    .is_some_and(|tick| tick >= low && ready_tick.is_some_and(|end| tick < end))
            })
        })
        .collect::<Vec<_>>();
    json!({
        "hp_le_7_observed_before_ready": first_low_hp_tick.is_some(),
        "first_low_hp_tick": first_low_hp_tick,
        "ready_tick": ready_tick,
        "eat_requested_before_ready": !eat_rows.is_empty(),
        "eat_accepted_before_ready": eat_rows.iter().any(|action| accepted(action)),
        "eat_admitted_before_ready": eat_rows.iter().any(|action| action_wire_valid(action)),
        "eat_sequences": eat_rows.iter().map(|action| action["sequence"].clone()).collect::<Vec<_>>(),
    })
}

fn m4_conditional_eat_ok(capture: &CombatCapture, report: &Value) -> bool {
    let receipt = m4_conditional_eat_receipt(capture, report);
    receipt["hp_le_7_observed_before_ready"] != json!(true)
        || (receipt["eat_requested_before_ready"] == json!(true)
            && receipt["eat_admitted_before_ready"] == json!(true))
}

fn m5_owner_preempted(capture: &CombatCapture, injection: &Value) -> bool {
    let owner = &injection["active_combat_owner"];
    injection["active_combat_owner_live_before"] == json!(true)
        && injection["active_combat_owner_live_after"] == json!(false)
        && injection["active_combat_owner_liveness_basis"]
            == json!("native action owner revocation bit")
        && injection["delivery"] == json!("Play.observe -> PlaySlotScript.on_random")
        && owner["action_id"].as_u64().is_some_and(|id| id > 0)
        && capture.actions.iter().any(|action| {
            is_npc_attack(action)
                && accepted(action)
                && action["run"] == owner["run"]
                && action["action_id"] == owner["action_id"]
                && action["request_id"] == owner["request_id"]
                && action["sequence"]
                    .as_u64()
                    .zip(injection["action_sequence_at_injection"].as_u64())
                    .is_some_and(|(attack, injection)| attack <= injection)
        })
}

fn clear_prayers_before_next_operation(capture: &CombatCapture, injection: &Value) -> bool {
    if injection["hold"] != json!(true) {
        return false;
    }
    let Some(sequence) = injection["action_sequence_at_injection"].as_u64() else {
        return false;
    };
    let Some(active_varps) = injection["prayer_varps_at_injection"].as_array() else {
        return false;
    };
    let expected_components = capture
        .prayer_facts
        .iter()
        .filter(|fact| {
            let Some(varp) = fact["varp"].as_i64() else {
                return false;
            };
            active_varps
                .iter()
                .any(|row| row["index"] == json!(varp) && row["value"] == json!(1))
        })
        .filter_map(|fact| fact["button_com"].as_i64())
        .collect::<Vec<_>>();
    if expected_components.is_empty() {
        return false;
    }
    let after = capture
        .actions
        .iter()
        .filter(|action| {
            action["sequence"]
                .as_u64()
                .is_some_and(|seq| seq > sequence)
        })
        .collect::<Vec<_>>();
    let Some(next_operation_index) = after.iter().position(|action| {
        action["kind"] != json!("interaction") || action["request"]["op"] != json!("if-button")
    }) else {
        return false;
    };
    let cleanup = &after[..next_operation_index];
    let next_operation = after[next_operation_index];
    cleanup.len() == expected_components.len()
        && expected_components.iter().all(|component| {
            cleanup
                .iter()
                .filter(|action| prayer_action(action, *component) && action_wire_valid(action))
                .count()
                == 1
        })
        && all_prayer_bits_off(&next_operation["snapshot"])
}

fn no_attack_after_report(capture: &CombatCapture, report: &Value) -> bool {
    let Some(end_tick) = integer(report, "combat_evidence_tick") else {
        return false;
    };
    !capture.actions.iter().any(|action| {
        action["tick"].as_i64().is_some_and(|tick| tick > end_tick) && is_npc_attack_request(action)
    })
}

fn is_eat_or_drink(action: &Value) -> bool {
    is_eat(action) || is_drink(action)
}

fn is_eat(action: &Value) -> bool {
    action["request"]["op"] == json!("held")
        && action["request"]["action"]
            .as_str()
            .is_some_and(|verb| verb.eq_ignore_ascii_case("eat"))
}

fn is_drink(action: &Value) -> bool {
    action["request"]["op"] == json!("held")
        && action["request"]["action"]
            .as_str()
            .is_some_and(|verb| verb.eq_ignore_ascii_case("drink"))
}

fn accepted(action: &Value) -> bool {
    action["accepted"] == json!(true)
}

fn caller_walked_out(capture: &CombatCapture) -> bool {
    capture.actions.iter().any(|action| {
        action["kind"] == json!("walk")
            && action["request"]["target"]["x"] == json!(WALK_OUT.x)
            && action["request"]["target"]["z"] == json!(WALK_OUT.z)
            && action["request"]["target"]["level"] == json!(WALK_OUT.level)
    })
}

fn arrived_at_walk_out(tile: WorldTile) -> bool {
    tile.level == WALK_OUT.level
        && (tile.x - WALK_OUT.x).abs().max((tile.z - WALK_OUT.z).abs()) <= WALK_OUT_RADIUS
}

fn final_tile(capture: &CombatCapture) -> Option<WorldTile> {
    let tile = capture.frames.last()?.get("tile")?;
    Some(WorldTile {
        x: tile.get(0)?.as_i64()? as i32,
        z: tile.get(1)?.as_i64()? as i32,
        level: tile.get(2)?.as_i64()? as i32,
    })
}

fn all_prayer_bits_off(frame: &Value) -> bool {
    frame["prayer_varps"]
        .as_array()
        .is_some_and(|varps| varps.len() == 15 && varps.iter().all(|row| row["value"] == json!(0)))
}

fn prayer_varp(frame: &Value, id: i64) -> Option<i64> {
    frame["prayer_varps"]
        .as_array()?
        .iter()
        .find(|row| row["index"] == json!(id))?["value"]
        .as_i64()
}

fn stat_pair(frame: &Value, name: &str) -> Option<(i64, i64)> {
    let row = stat_row(frame, name)?;
    Some((row["base"].as_i64()?, row["effective"].as_i64()?))
}

fn stat_base(frame: &Value, name: &str) -> Option<i64> {
    stat_row(frame, name)?["base"].as_i64()
}

fn stat_effective(frame: &Value, name: &str) -> Option<i64> {
    stat_row(frame, name)?["effective"].as_i64()
}

fn stat_row<'a>(frame: &'a Value, name: &str) -> Option<&'a Value> {
    frame["stats"].as_array()?.iter().find(|row| {
        row["name"]
            .as_str()
            .is_some_and(|value| value.eq_ignore_ascii_case(name))
    })
}

fn item_count(frame: &Value, id: i32) -> i64 {
    frame["inventory"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|item| item["id"] == json!(id))
        .filter_map(|item| item["count"].as_i64())
        .sum()
}

fn equipment_count(frame: &Value, id: i32) -> i64 {
    frame["equipment"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|item| item["id"] == json!(id))
        .filter_map(|item| item["count"].as_i64())
        .sum()
}

fn integer(value: &Value, key: &str) -> Option<i64> {
    value["fields"][key].as_i64()
}

fn named_npcs<'a>(frame: &'a Value, name: &str) -> Vec<&'a Value> {
    frame["nearby_npcs"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|npc| {
            npc["name"]
                .as_str()
                .is_some_and(|value| value.eq_ignore_ascii_case(name))
        })
        .collect()
}

fn local_threats(frame: &Value) -> Option<Vec<Value>> {
    let self_slot = frame["self_slot"].as_i64()?;
    let threats = frame["nearby_npcs"]
        .as_array()?
        .iter()
        .filter(|npc| {
            npc["target"]["kind"] == json!("Player")
                && npc["target"]["index"] == json!(self_slot as usize)
        })
        .cloned()
        .collect();
    Some(threats)
}

fn has_multiple_local_threats(capture: &CombatCapture) -> bool {
    capture
        .frames
        .iter()
        .any(|frame| local_threats(frame).is_some_and(|threats| threats.len() > 1))
}

fn tile_is(value: &Value, tile: WorldTile) -> bool {
    value["x"] == json!(tile.x)
        && value["z"] == json!(tile.z)
        && value["level"] == json!(tile.level)
}

fn run_case(case: Case) {
    assert_eq!(
        std::env::var("LIVE").as_deref(),
        Ok("1"),
        "{} requires LIVE=1",
        case.key()
    );
    let _cpu = CpuRendererEnv::enable();
    std::fs::create_dir_all(EVIDENCE_DIR).expect("create assigned combat evidence directory");
    let home = ThrowawayHome::enter(case.key()).expect("create isolated HOME");
    let options = match profile_options(&home.path) {
        Ok(options) => options,
        Err(error) => panic!("{} live prerequisites: {error}", case.key()),
    };
    let template = options
        .resolve(None)
        .expect("resolve local-289 combat profile")
        .prepare_template()
        .expect("prepare isolated local-289 template");
    let selected = api::game_data::for_revision(ClientRevision::R289).expect("selected R289 data");
    let quests = Arc::new(
        QuestCatalog::from_identity(selected.quest_identity()).expect("selected quest catalog"),
    );
    let stand = case
        .stand(
            template
                .world()
                .as_deref()
                .expect("selected navigation world"),
        )
        .expect("choose collision-backed combat fixture stand");
    let path_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../script/paths/289");
    let source_path = path_root.join(case.path_relative());
    let source = std::fs::read(&source_path)
        .unwrap_or_else(|error| panic!("read {}: {error}", source_path.display()));
    let path = compile_path(&source, &selected, &quests)
        .unwrap_or_else(|error| panic!("compile {}: {error:?}", source_path.display()));

    let names = super::mint_live_names(1);
    let account = names.first().expect("mint combat account").clone();
    let password = super::mint_live_entries(&names)
        .into_iter()
        .find(|(name, _)| name == &account)
        .map(|(_, password)| password)
        .expect("mint local combat password");
    let capture = Arc::new(Mutex::new(CombatCapture::default()));
    {
        let mut proof = capture.lock().unwrap_or_else(|e| e.into_inner());
        proof.inject_maze_after_imp_attack = case.inject_maze();
        proof.prayer_facts = selected
            .prayers()
            .iter()
            .map(|prayer| {
                json!({
                    "name": prayer.name,
                    "varp": prayer.varp,
                    "button_com": prayer.button_com,
                })
            })
            .collect();
    }
    let _registration = CaptureRegistration::install(&account, Arc::clone(&capture));
    let mut writer = EvidenceWriter {
        case,
        account: account.clone(),
        path: PathBuf::from(EVIDENCE_DIR),
        capture: Arc::clone(&capture),
        frame: FrameBuf::new(),
        scenario_status: "Preparing".to_owned(),
        outcome: "RUNNING".to_owned(),
        error: None,
        flushed: false,
    };

    let scenario = scenario_for(case, stand, Arc::clone(&capture));
    let mut runner = ScenarioRunner::with_world(scenario, template.world());
    runner.set_map_members(true);
    runner.set_live_names(&names);
    runner.set_shot_sink(Box::new(|_, _| {}));
    let state = Arc::new(Mutex::new(LiveState {
        case,
        account: account.clone(),
        runner,
        snapshot: GameSnapshot::new(),
        pump: Pump::new(),
        start_context: None,
        selected: Arc::clone(&selected),
        quests,
        path,
        capture: Arc::clone(&capture),
        started: false,
    }));
    let frame_state = Arc::clone(&state);
    let frame_buffer = Arc::clone(&writer.frame);
    let frame_account = account.clone();
    let profile = Profile {
        username: account.clone(),
        password: password.into(),
        uid: (SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("combat proof clock")
            .as_millis()
            % i32::MAX as u128) as i32,
        settings: ProfileSettings::default(),
    };
    let mut play = run_with_template(
        Arc::clone(&template),
        true,
        vec![profile],
        move |_| (None, Some(Arc::clone(&frame_buffer))),
        move |client, username, hold| {
            if username == frame_account {
                client.set_draw(true);
                frame_state
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .frame(client, hold.hold);
            }
        },
    )
    .expect("start local combat Play");
    let obj_names = play.obj_names();
    {
        let mut state = state.lock().unwrap_or_else(|e| e.into_inner());
        state.runner.set_obj_names(obj_names);
        state.start_context = Some((play.script_start_handle(), play.named_banks()));
    }
    play.focus(&account);

    let deadline = Instant::now() + case.timeout();
    let mut terminal_error = None;
    let mut abort_observation_end = None;
    loop {
        if let Some(status) = play.script_native_status(&account) {
            combat_proof::record_status(&account, &status);
        }
        let capture_snapshot = capture.lock().unwrap_or_else(|e| e.into_inner());
        if capture_has_death(&capture_snapshot) {
            terminal_error = Some("death observed during the measured window".into());
            writer.outcome = "FAIL".to_owned();
            break;
        }
        if let Some(reason) = capture_snapshot
            .invalid_reason
            .clone()
            .filter(|_| abort_observation_end.is_none())
        {
            terminal_error = Some(reason);
            writer.outcome = "INVALID".to_owned();
            break;
        }
        if matches!(case, Case::M2 | Case::M3 | Case::M6)
            && has_multiple_local_threats(&capture_snapshot)
        {
            let reason = format!(
                "{} natural-area interference: multiple nearby/facing NPC threats",
                case.key()
            );
            drop(capture_snapshot);
            combat_proof::mark_invalid(&account, reason.clone());
            terminal_error = Some(reason);
            writer.outcome = "INVALID".to_owned();
            break;
        }
        if let Some(reason) = case_invalid_reason(case, &capture_snapshot) {
            let tick = capture_snapshot
                .frames
                .last()
                .and_then(|frame| frame["tick"].as_i64());
            let end = abort_observation_end.get_or_insert_with(|| tick.unwrap_or(0) + 30);
            let observed = tick.is_some_and(|tick| tick >= *end);
            drop(capture_snapshot);
            combat_proof::mark_invalid(&account, reason.clone());
            terminal_error = Some(reason);
            writer.outcome = "INVALID".to_owned();
            if observed || Instant::now() >= deadline {
                break;
            }
            thread::sleep(Duration::from_millis(50));
            continue;
        }
        drop(capture_snapshot);
        let state = state.lock().unwrap_or_else(|e| e.into_inner());
        match state.runner.status() {
            RunnerStatus::Passed => {
                writer.scenario_status = "Passed".to_owned();
                break;
            }
            RunnerStatus::Failed(error) => {
                writer.scenario_status = format!("Failed({error})");
                terminal_error = Some(format!("ScenarioRunner failed: {error}"));
                writer.outcome = "FAIL".to_owned();
                break;
            }
            _ => {}
        }
        if Instant::now() >= deadline {
            writer.scenario_status = format!("TimedOut({:?})", state.runner.status());
            terminal_error = Some(format!(
                "{} live proof exceeded {:?}",
                case.key(),
                case.timeout()
            ));
            writer.outcome = "FAIL".to_owned();
            break;
        }
        drop(state);
        thread::sleep(Duration::from_millis(50));
    }

    let capture_snapshot = capture.lock().unwrap_or_else(|e| e.into_inner());
    if capture_has_death(&capture_snapshot) {
        writer.outcome = "FAIL".to_owned();
        writer.error = Some("death observed during the measured window".into());
    } else if capture_snapshot.invalid_reason.is_some() {
        writer.outcome = if case == Case::M6
            && capture_snapshot
                .invalid_reason
                .as_deref()
                .is_some_and(|reason| reason.starts_with("M6 NOT_STAGED:"))
        {
            "NOT_STAGED"
        } else {
            "INVALID"
        }
        .to_owned();
        writer.error = terminal_error.or_else(|| capture_snapshot.invalid_reason.clone());
    } else if writer.outcome == "RUNNING" && case_ready(case, &capture_snapshot) {
        writer.outcome = "PASS".to_owned();
    } else if writer.outcome == "RUNNING" {
        writer.outcome = "FAIL".to_owned();
        writer.error =
            terminal_error.or_else(|| Some(format!("{} acceptance criteria not met", case.key())));
    } else {
        writer.error = terminal_error;
    }
    drop(capture_snapshot);
    drop(play);
    writer.flush();

    let started = capture.lock().unwrap_or_else(|e| e.into_inner()).started;
    if case == Case::M2 && started {
        let proof = capture.lock().unwrap_or_else(|e| e.into_inner());
        let baseline = proof
            .start_baseline
            .as_ref()
            .expect("native Start baseline");
        let first = proof
            .actions
            .iter()
            .find(|action| action["kind"] == "interaction")
            .expect("M2 native Prep must dispatch after the completed relog");
        assert_eq!(
            first["snapshot"]["self_slot"], baseline["self_slot"],
            "native Start must not precede the relog's session replacement"
        );
        assert_eq!(
            first["snapshot"]["tile"], baseline["tile"],
            "the first native Prep must retain the relogged seed's location"
        );
    }

    match writer.outcome.as_str() {
        "PASS" | "INVALID" | "NOT_STAGED" => {}
        other => panic!("{} live proof {other}: {:?}", case.key(), writer.error),
    }
}

#[test]
#[ignore = "requires LIVE=1 and the isolated local R289 engine"]
fn live_combat_m1_production_imp_four_beads() {
    run_case(Case::M1);
}

#[test]
#[ignore = "requires LIVE=1 and the isolated local R289 engine"]
fn live_combat_m1_staged_hand_in() {
    run_case(Case::M1HandIn);
}

#[test]
#[ignore = "requires LIVE=1 and the isolated local R289 engine"]
fn live_combat_m2_natural_warlord_upkeep() {
    run_case(Case::M2);
}

#[test]
#[ignore = "requires LIVE=1 and the isolated local R289 engine"]
fn live_combat_m3_natural_warlord_food_only() {
    run_case(Case::M3);
}

#[test]
#[ignore = "requires LIVE=1 and the isolated local R289 engine"]
fn live_combat_m4_static_nasty_tree_unattackable() {
    run_case(Case::M4);
}

#[test]
#[ignore = "requires LIVE=1 and the isolated local R289 engine"]
fn live_combat_m5_random_interrupt_clears_prayers() {
    run_case(Case::M5);
}

#[test]
#[ignore = "requires LIVE=1 and the isolated local R289 engine with cooked karambwan"]
fn live_combat_m6_natural_warlord_combo_eat() {
    run_case(Case::M6);
}

#[test]
fn corpse_identity_and_terminal_prayer_off_are_checked_in_their_own_shapes() {
    let mut capture = CombatCapture::default();
    capture.statuses.push(json!({"fields": {
        "combat_end": "Killed",
        "combat_engaged_index": 7,
        "combat_engaged_npc_type": 477,
        "combat_evidence_tick": 12,
    }}));
    capture.frames.push(json!({
        "tick": 12,
        "nearby_npcs": [{"index": 7, "type": 477, "health": 0, "total_health": 170}],
    }));
    assert!(every_killed_report_has_corpse(&capture));
    capture.frames[0]["nearby_npcs"][0]["index"] = json!(8);
    assert!(!every_killed_report_has_corpse(&capture));
    capture.frames[0]["nearby_npcs"][0]["index"] = json!(7);
    capture.frames[0]["nearby_npcs"][0]["type"] = json!(478);
    assert!(!every_killed_report_has_corpse(&capture));

    capture.prayer_facts.push(json!({
        "name": "Protect from Melee", "button_com": 5623, "varp": 97,
    }));
    capture.actions = vec![
        json!({"kind": "interaction", "tick": 10, "batch": 1, "request": {"op": "if-button", "component_id": 5623},
            "snapshot": {"prayer_varps": [{"index": 97, "value": 0}]}}),
        json!({"kind": "interaction", "tick": 10, "batch": 1, "request": {"op": "npc", "action": "Attack"}}),
        json!({"kind": "interaction", "tick": 11, "batch": 3, "request": {"op": "if-button", "component_id": 5623},
            "snapshot": {"prayer_varps": [{"index": 97, "value": 1}]}}),
    ];
    assert!(protect_plan_ends_with_terminal(&capture));
    capture.actions.remove(1);
    assert!(!protect_plan_ends_with_terminal(&capture));
    capture.actions.remove(0);
    assert!(protect_plan_ends_with_terminal(&capture));
    capture.actions[0]["snapshot"]["prayer_varps"][0]["value"] = Value::Null;
    assert!(!protect_plan_ends_with_terminal(&capture));
}

#[test]
fn static_tree_identity_uses_selected_type_not_debug_name() {
    let mut baseline = json!({
        "ingame": true, "scene_state": 2, "self_slot": 1,
        "stats": [
            {"name": "hitpoints", "base": 30, "effective": 8},
            {"name": "prayer", "base": 1, "effective": 1}
        ],
        "inventory": [{"id": LOBSTER_ID, "count": 4}],
        "nearby_npcs": [{
            "index": 4328, "type": 152, "name": "Tree",
            "tile": {"x": 3108, "z": 3346, "level": 0},
            "in_combat": false, "animation": 73,
            "target": {"kind": "Player", "index": 1}
        }]
    });
    assert_eq!(start_preflight(Case::M4, &baseline), None);
    baseline["stats"][0]["effective"] = json!(7);
    baseline["tick"] = json!(20);
    let mut capture = CombatCapture::default();
    capture.frames.push(baseline.clone());
    assert_eq!(m4_first_tree_hit_tick(&capture), Some(20));
    capture.frames[0]["stats"][0]["effective"] = json!(8);
    assert_eq!(
        m4_first_tree_hit_tick(&capture),
        Some(20),
        "tree attack animation 73 targeting the player is the hit onset, including before HP drops"
    );
    baseline["stats"][0]["effective"] = json!(8);
    baseline["nearby_npcs"][0]["type"] = json!(1226);
    assert!(start_preflight(Case::M4, &baseline).is_some());
    capture.frames[0]["nearby_npcs"][0]["type"] = json!(1226);
    assert_eq!(m4_first_tree_hit_tick(&capture), None);
}

#[test]
fn m4_walk_out_oracle_reads_the_captured_walk_target_object() {
    let mut capture = CombatCapture::default();
    capture.actions.push(json!({
        "kind": "walk",
        "request": "Walk 3093,3243"
    }));
    assert!(
        !caller_walked_out(&capture),
        "debug-string walk requests are not the capture schema"
    );
    capture.actions.push(json!({
        "kind": "walk",
        "request": { "target": { "x": 3093, "z": 3243, "level": 0 }, "radius": 2 }
    }));
    assert!(caller_walked_out(&capture));
    assert!(arrived_at_walk_out(WorldTile {
        x: 3092,
        z: 3245,
        level: 0,
    }));
    assert!(arrived_at_walk_out(WALK_OUT));
    assert!(!arrived_at_walk_out(WorldTile {
        x: 3109,
        z: 3346,
        level: 0,
    }));
}

#[test]
fn food_oracle_discriminates_past_deadline_and_cumulative_extension() {
    let fixture = |onsets: &[i64], eats: &[i64]| {
        let mut capture = CombatCapture::default();
        capture.statuses.push(json!({"fields": {
            "combat_end": "Killed", "combat_engaged_index": 830
        }}));
        for tick in onsets {
            for (frame_tick, animation) in [(tick - 1, -1), (*tick, 390)] {
                capture.frames.push(json!({
                    "tick": frame_tick, "equipment": [{"id": RUNE_SCIMITAR_ID}],
                    "local_player": {
                        "animation": animation,
                        "target": {"kind": "Npc", "index": 830}
                    }
                }));
            }
        }
        for tick in eats {
            capture.actions.push(json!({
                "tick": tick, "request": {"op": "held", "action": "Eat"}
            }));
        }
        capture
    };
    // Design case28's product branch: OBS20 => D24, Eat32 => D27 (past),
    // same-plan Attack32 => next OBS33. A single-op Attack33 slips to OBS34.
    let eligible = fixture(&[20, 33], &[32]);
    assert!(m3_eat_timing_ok(&eligible));
    let delayed_attack = fixture(&[20, 34], &[32]);
    assert!(!m3_eat_timing_ok(&delayed_attack));
    // A mutant that ignores the food's clock extension swings at13, not16.
    let ignores_deadline = fixture(&[9, 13], &[11]);
    assert_eq!(
        m3_timing_receipt(&ignores_deadline)["eats"][0]["exact"],
        json!(false)
    );
    let cumulative = fixture(&[127, 137], &[129, 133]);
    let receipt = m3_timing_receipt(&cumulative);
    assert!(receipt["eats"]
        .as_array()
        .unwrap()
        .iter()
        .all(|eat| eat["exact"] == json!(true)));
    assert_eq!(receipt["eats"][1]["old_attack_deadline"], json!(134));
    // Moving the second input one tick late cannot retroactively suppress the
    // output that was due at134; the independent oracle must reject this trace.
    let late_eat = fixture(&[127, 137], &[129, 134]);
    assert_eq!(
        m3_timing_receipt(&late_eat)["eats"][0]["exact"],
        json!(false)
    );
}

#[test]
fn every_imp_corpse_episode_requires_its_own_exact_killed_outcome() {
    let mut capture = CombatCapture::default();
    for (tick, health) in [(10, 0), (11, 0), (12, 4), (20, 0)] {
        capture.frames.push(json!({
            "tick": tick,
            "nearby_npcs": [{
                "index": 123, "type": 708, "health": health, "total_health": 8
            }]
        }));
    }
    let outcome = |tick| {
        json!({"fields": {
            "combat_end": "Killed", "combat_evidence_tick": tick,
            "combat_evidence_sequence": tick,
            "combat_engaged_index": 123, "combat_engaged_npc_type": 708
        }})
    };
    capture.statuses.push(outcome(11));
    assert!(!every_imp_corpse_has_outcome(&capture));
    capture.statuses.push(outcome(20));
    assert!(every_imp_corpse_has_outcome(&capture));
    capture.statuses[1]["fields"]["combat_engaged_index"] = json!(124);
    assert!(!every_imp_corpse_has_outcome(&capture));
    capture.statuses[1]["fields"]["combat_engaged_index"] = json!(123);
    capture.statuses[1]["fields"]["combat_evidence_tick"] = json!(19);
    assert!(!every_imp_corpse_has_outcome(&capture));
    capture.statuses[1]["fields"]["combat_evidence_tick"] = json!(20);
    capture.statuses[1]["fields"]["combat_engaged_npc_type"] = json!(477);
    assert!(!every_imp_corpse_has_outcome(&capture));
}

#[test]
fn hygiene_clear_prefix_rejects_missing_clear_empty_prayers_and_other_clicks() {
    let varps = |active| {
        (83..=97)
            .map(|index| json!({"index": index, "value": i32::from(active && index == 97)}))
            .collect::<Vec<_>>()
    };
    let injection = json!({
        "hold": true, "action_sequence_at_injection": 1,
        "prayer_varps_at_injection": varps(true)
    });
    let clear = json!({
        "kind": "interaction", "sequence": 2,
        "request": {"op": "if-button", "component_id": 5623},
        "accepted": true, "wire_decoded": true,
        "wire_opcodes": [client::io::ClientProt289::IF_BUTTON.id],
    });
    let next = json!({
        "kind": "walk", "sequence": 3,
        "snapshot": {"prayer_varps": varps(false)}
    });
    let mut capture = CombatCapture::default();
    capture
        .prayer_facts
        .push(json!({"varp": 97, "button_com": 5623}));
    capture.actions = vec![clear.clone(), next.clone()];
    assert!(clear_prayers_before_next_operation(&capture, &injection));
    capture.actions[0]["request"]["component_id"] = json!(999);
    assert!(!clear_prayers_before_next_operation(&capture, &injection));
    capture.actions[0] = clear.clone();
    let mut empty = injection.clone();
    empty["prayer_varps_at_injection"] = json!(varps(false));
    assert!(!clear_prayers_before_next_operation(&capture, &empty));
    capture.actions = vec![next.clone()];
    assert!(!clear_prayers_before_next_operation(&capture, &injection));
    capture.actions = vec![next.clone(), clear.clone()];
    assert!(!clear_prayers_before_next_operation(&capture, &injection));
    capture.actions = vec![clear.clone(), next.clone()];
    capture.actions[1]["snapshot"]["prayer_varps"][14]["value"] = json!(1);
    assert!(!clear_prayers_before_next_operation(&capture, &injection));
    capture.actions = vec![clear.clone(), next];
    let mut unrelated = clear;
    unrelated["request"]["component_id"] = json!(999);
    capture.actions.insert(1, unrelated);
    assert!(
        !clear_prayers_before_next_operation(&capture, &injection),
        "an unrelated if-button is not part of ClearPrayers"
    );
}

#[test]
fn hygiene_owner_oracle_requires_the_same_live_then_revoked_combat_owner() {
    let mut capture = CombatCapture::default();
    capture.actions.push(json!({
        "kind": "interaction", "sequence": 1, "accepted": true,
        "run": "run-1", "action_id": 7, "request_id": 9,
        "request": {"op": "npc", "action": "Attack"}
    }));
    let mut injection = json!({
        "active_combat_owner": {"run": "run-1", "action_id": 7, "request_id": 9},
        "active_combat_owner_live_before": true,
        "active_combat_owner_live_after": false,
        "active_combat_owner_liveness_basis": "native action owner revocation bit",
        "delivery": "Play.observe -> PlaySlotScript.on_random",
        "action_sequence_at_injection": 1
    });
    assert!(m5_owner_preempted(&capture, &injection));
    injection["active_combat_owner_live_before"] = json!(false);
    assert!(!m5_owner_preempted(&capture, &injection));
    injection["active_combat_owner_live_before"] = json!(true);
    injection["active_combat_owner_live_after"] = json!(true);
    assert!(!m5_owner_preempted(&capture, &injection));
    injection["active_combat_owner_live_after"] = json!(false);
    injection["active_combat_owner"]["action_id"] = json!(8);
    assert!(!m5_owner_preempted(&capture, &injection));
    injection["active_combat_owner"]["action_id"] = json!(7);
    injection["active_combat_owner_liveness_basis"] = json!("request reservation");
    assert!(!m5_owner_preempted(&capture, &injection));
}

#[test]
fn m5_ready_accepts_interrupt_hygiene_while_the_path_continues() {
    use client::io::ClientProt289;
    let varps = |active| {
        (83..=97)
            .map(|index| json!({"index": index, "value": i32::from(active && index == 97)}))
            .collect::<Vec<_>>()
    };
    let attack_opcodes = json!([ClientProt289::MOVE_OPCLICK.id, ClientProt289::OPNPC2.id]);
    let attack = json!({
        "kind": "interaction", "sequence": 1, "tick": 10, "accepted": true,
        "wire_decoded": true, "wire_opcodes": attack_opcodes,
        "run": "run-1", "action_id": 7, "request_id": 9,
        "batch": 9,
        "request": {"op": "npc", "action": "Attack"}
    });
    let clear = json!({
        "kind": "interaction", "sequence": 2, "tick": 12, "accepted": true,
        "wire_decoded": true, "wire_opcodes": [ClientProt289::IF_BUTTON.id],
        "request": {"op": "if-button", "component_id": 5623},
    });
    let next = json!({
        "kind": "walk", "sequence": 3, "tick": 14,
        "snapshot": {"prayer_varps": varps(false)}
    });
    let later_attack = json!({
        "kind": "interaction", "sequence": 4, "tick": 40, "accepted": true,
        "wire_decoded": true, "wire_opcodes": attack_opcodes,
        "request": {"op": "npc", "action": "Attack"}
    });
    let bank = json!({
        "kind": "interaction", "sequence": 5, "tick": 50, "accepted": true,
        "wire_decoded": true, "wire_opcodes": [195, 67, 45],
        "request": {"debug": "OpenStand"}
    });
    let injection = json!({
        "kind": "Maze", "hold": true, "action_sequence_at_injection": 1,
        "prayer_varps_at_injection": varps(true),
        "active_combat_owner": {"run": "run-1", "action_id": 7, "request_id": 9},
        "active_combat_owner_live_before": true,
        "active_combat_owner_live_after": false,
        "active_combat_owner_liveness_basis": "native action owner revocation bit",
        "delivery": "Play.observe -> PlaySlotScript.on_random",
    });
    let mut capture = CombatCapture::default();
    capture.maze_injected = true;
    capture.maze_owner_live_before = Some(true);
    capture.maze_owner_live_after = Some(false);
    capture.prayer_facts = vec![json!({"varp": 97, "button_com": 5623})];
    capture.random_events = vec![injection.clone()];
    capture.actions = vec![attack, clear, next, later_attack, bank];
    capture.observations = vec![json!({"tick": 10, "exclusive": true})];
    assert!(
        m5_ready(&capture),
        "later Imp attacks and a bank OpenStand must not un-prove interrupt hygiene"
    );
    let duplicate = capture.actions[0].clone();
    capture.actions.insert(1, duplicate);
    assert!(
        !m5_ready(&capture),
        "duplicate Attack breaks the admitted prefix"
    );
    capture.actions.remove(1);
    capture.actions.insert(
        1,
        json!({
            "kind": "other-request", "origin": "staging", "sequence": 1,
            "tick": 11, "host_tick": 11, "request": "setvar prayer14 1"
        }),
    );
    assert!(
        !m5_ready(&capture),
        "a measured-window cheat cannot stage proof"
    );
    capture.actions[1]["request"] = json!("if-button Protect from Melee");
    capture.actions[1]["accepted"] = json!(true);
    capture.actions[1]["wire_decoded"] = json!(true);
    capture.actions[1]["wire_opcodes"] = json!([ClientProt289::IF_BUTTON.id]);
    assert!(
        m5_ready(&capture),
        "a captured real prayer click on its own later tick is admissible"
    );
    capture.actions[1]["tick"] = json!(10);
    capture.actions[1]["host_tick"] = json!(10);
    assert!(
        !m5_ready(&capture),
        "a staging click cannot share the exclusive Attack tick"
    );
    capture.actions.remove(1);
    capture.random_events[0]["prayer_varps_at_injection"] = json!(varps(false));
    assert!(
        !m5_ready(&capture),
        "an all-off injection snapshot is not a Protect-from-Melee interrupt"
    );
}

#[test]
fn legacy_caller_wire_rejects_unknown_debug_and_wrong_packets() {
    use client::io::ClientProt289;
    let mut action = json!({
        "accepted": true, "wire_decoded": true,
        "wire_opcodes": [ClientProt289::MOVE_OPCLICK.id, ClientProt289::OPOBJ3.id],
        "request": {"debug": "Obj { x: 2638, z: 3224, level: 0, name: Some(\"Red bead\"), action: \"Take\" }"}
    });
    assert!(action_wire_valid(&action));
    action["request"]["debug"] =
        json!("Obj { x: 2638, z: 3224, level: 0, name: Some(\"Red bead\"), action: \"Destroy\" }");
    assert!(!action_wire_valid(&action));
    action["request"]["debug"] = json!("OpenStand { x: 2656, z: 3283, level: 0, kind: \"booth\", name: Some(\"Bank booth\"), stand_op: Some(2), choose: None }");
    assert!(!action_wire_valid(&action));
    action["wire_opcodes"] = json!([ClientProt289::MOVE_OPCLICK.id, ClientProt289::OPLOC2.id]);
    assert!(action_wire_valid(&action));
    action["request"]["debug"] = json!("arbitrary OpenStand Take");
    assert!(!action_wire_valid(&action));
}

#[test]
fn warlord_live_fact_is_not_a_defend_animation_attack() {
    let mut capture = CombatCapture::default();
    capture.frames.push(json!({
        "tick": 43, "self_slot": 7, "nearby_npcs": [{
            "type": 477, "name": "Khazard Warlord", "index": 8,
            "in_combat": true, "animation": 404,
            "target": {"kind": "Player", "index": 7}
        }]
    }));
    assert_eq!(first_warlord_live_fact(&capture), Some(43));
    assert_eq!(first_warlord_attack_onset(&capture), None);
    capture.frames[0]["nearby_npcs"][0]["animation"] = json!(401);
    assert_eq!(first_warlord_attack_onset(&capture), Some(43));
    capture.frames[0]["nearby_npcs"][0]["target"]["index"] = json!(9);
    assert_eq!(first_warlord_live_fact(&capture), None);
}

#[path = "combat_receipt_replay_tests.rs"]
mod receipt_replay;

#[test]
fn post_budget_kill_is_separate_and_rejects_late_or_different_npc() {
    let mut capture = CombatCapture::default();
    capture.statuses.push(json!({"fields": {
        "combat_end": "Budget", "combat_evidence_tick": 20,
        "combat_evidence_sequence": 20, "combat_engaged_index": 123,
        "combat_engaged_npc_type": 708
    }}));
    capture.frames.push(json!({"tick": 25, "nearby_npcs": [{
        "index": 123, "type": 708, "health": 0, "total_health": 8
    }]}));
    let classified = imp_corpse_classification(&capture);
    assert_eq!(classified["post_budget_kill"], json!(1));
    assert_eq!(classified["killed"], json!(0));
    assert!(every_imp_corpse_has_outcome(&capture));
    capture.frames[0]["tick"] = json!(26);
    assert!(!every_imp_corpse_has_outcome(&capture));
    capture.frames[0]["tick"] = json!(25);
    capture.frames[0]["nearby_npcs"][0]["index"] = json!(124);
    assert!(!every_imp_corpse_has_outcome(&capture));
}

#[test]
fn parked_owner_zero_hp_is_not_hidden_by_missing_death_status() {
    let mut capture = CombatCapture::default();
    capture.start_baseline = Some(json!({"snapshot_tick": 20}));
    capture.frames.push(json!({
        "ingame": true, "snapshot_tick": 30,
        "stats": [{"name": "hitpoints", "base": 40, "effective": 0}]
    }));
    assert!(capture_has_death(&capture));
    capture.frames[0]["stats"][0]["effective"] = json!(1);
    assert!(!capture_has_death(&capture));
    capture.frames[0]["stats"][0]["effective"] = json!(0);
    capture.frames[0]["snapshot_tick"] = json!(19);
    assert!(!capture_has_death(&capture));
    capture.frames[0]["snapshot_tick"] = json!(30);
    capture.frames[0]["stats"][0]["base"] = json!(0);
    assert!(!capture_has_death(&capture));
}
