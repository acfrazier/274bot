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

use api::game_data::{RangedAmmoFamily, RangedModeFact, SelectedGameData};
use api::interact::{Interactions, SendResult};
use api::quest_facts::QuestCatalog;
use api::selected::{ClientRevision, RunKey};
use api::snapshot::{ActorKind, GameSnapshot, WorldTile};
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

fn evidence_dir() -> Result<PathBuf, String> {
    std::env::var_os("LIVE_EVIDENCE_DIR")
        .map(PathBuf::from)
        .ok_or_else(|| "combat live proof requires LIVE_EVIDENCE_DIR".to_owned())
}
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
const R1_ITEMS: &[(&str, i32)] = &[
    ("maple_shortbow", 1),
    ("steel_arrow", 150),
    ("4doseprayerrestore", 1),
];
const R2_ITEMS: &[(&str, i32)] = &[
    ("maple_shortbow", 1),
    ("bolt", 50),
    ("4doseprayerrestore", 1),
];
const R3_ITEMS: &[(&str, i32)] = &[("bronze_dart", 200), ("4doseprayerrestore", 1)];
const WARLORD_NPC_ID: usize = 477;

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
    M5Stop,
    M6,
    R1,
    R2,
    R3,
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
            Self::M5Stop => "M5-Stop",
            Self::M6 => "M6",
            Self::R1 => "R1",
            Self::R2 => "R2",
            Self::R3 => "R3",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::M1 => "combat_m1_imp_production",
            Self::M1HandIn => "combat_m1_staged_hand_in",
            Self::M2 => "combat_m2_melee_upkeep",
            Self::M3 => "combat_m3_melee_food_only",
            Self::M4 => "combat_m4_unattackable_tree",
            Self::M5 => "combat_m5_raised_prayer_interrupt_hygiene",
            Self::M5Stop => "combat_stop_owned_prayer_cleanup",
            Self::M6 => "combat_m6_melee_combo_eat",
            Self::R1 => "combat_r1_ranged_rapid",
            Self::R2 => "combat_r2_ranged_wrong_ammo",
            Self::R3 => "combat_r3_ranged_thrown",
        }
    }

    fn is_ranged(self) -> bool {
        matches!(self, Self::R1 | Self::R2 | Self::R3)
    }

    fn path_relative(self) -> &'static str {
        match self {
            Self::M1 | Self::M1HandIn | Self::M5 | Self::M5Stop => "imp.json",
            Self::M2 | Self::R1 | Self::R2 | Self::R3 => "fixtures/combat_melee_upkeep.json",
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
            Self::M5 | Self::M5Stop => M5_ITEMS,
            Self::M6 if std::env::var_os("BOT_COMBAT_M6_ORIGINAL_FOOD").is_some() => M6_ITEMS,
            Self::M6 => M6_RESERVE_ITEMS,
            Self::R1 => R1_ITEMS,
            Self::R2 => R2_ITEMS,
            Self::R3 => R3_ITEMS,
        }
    }

    fn timeout(self) -> Duration {
        match self {
            Self::M1 => Duration::from_secs(m1_coverage_budget().2),
            Self::M1HandIn | Self::M5 | Self::M5Stop => Duration::from_secs(1_200),
            Self::M2 | Self::M3 | Self::M6 | Self::R1 | Self::R3 => Duration::from_secs(900),
            Self::M4 | Self::R2 => Duration::from_secs(300),
        }
    }

    fn stand(self, world: &nav::world::NavWorld) -> Result<WorldTile, String> {
        match self {
            Self::M1 | Self::M5 | Self::M5Stop => Ok(IMP_START),
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
            Self::R1 | Self::R2 | Self::R3 => ranged_placement(self, world).map(|p| p.spawn),
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
        let path = evidence_dir()?
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
    ranged_probe_signature: Option<String>,
}

impl LiveState {
    fn frame(&mut self, client: &mut client::client::Client, hold: bool) {
        let drain = self.pump.drain_client(client);
        host::publish_snapshot(&mut self.snapshot, client, drain);

        if self.runner.on_start_script() && !self.started {
            combat_proof::record_start_baseline(&self.account, &self.snapshot);
            let baseline = combat_proof::snapshot_facts(&self.snapshot, None);
            let preflight = if self.case.is_ranged() {
                let capture = self.capture.lock().unwrap_or_else(|e| e.into_inner());
                ranged_start_preflight(self.case, &baseline, &self.selected, &capture)
            } else {
                start_preflight(self.case, &baseline)
            };
            if let Some(reason) = preflight {
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
        if self.case.is_ranged() && self.started {
            record_ranged_probe(
                client,
                &self.snapshot,
                &self.selected,
                &self.capture,
                &mut self.ranged_probe_signature,
            );
        }
        if self.case == Case::M5Stop && self.started {
            let trigger = {
                let capture = self.capture.lock().unwrap_or_else(|e| e.into_inner());
                !capture
                    .random_events
                    .iter()
                    .any(|event| event["kind"] == "OperatorStop")
                    && stop_fight_raise(&capture).is_some()
                    && self.snapshot.ingame()
                    && self.snapshot.scene_state() == 2
                    && self
                        .snapshot
                        .varps()
                        .iter()
                        .any(|row| row.index == 83 && row.value == 1)
                    && self
                        .snapshot
                        .varps()
                        .iter()
                        .any(|row| row.index == 97 && row.value == 1)
            };
            if trigger {
                let facts = combat_proof::snapshot_facts(&self.snapshot, None);
                let result = self
                    .start_context
                    .as_ref()
                    .expect("started context")
                    .0
                    .stop(&self.account);
                self.capture
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .random_events
                    .push(json!({
                        "kind": "OperatorStop",
                        "snapshot": facts,
                        "accepted": result.is_ok(),
                    }));
                if let Err(error) = result {
                    combat_proof::mark_invalid(
                        &self.account,
                        format!("operator Stop failed: {error}"),
                    );
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
        let ranged = if self.case.is_ranged() {
            ranged_receipt(self.case, &capture)
        } else {
            Value::Null
        };
        let mut receipt = json!({
            "proof": if self.case == Case::M5Stop { "LIFECYCLE-FOLLOWUPS-1" } else { "COMBAT-S3A-3" },
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
            "ranged": ranged,
            "warlord_first_open_hitbar": capture.frames.iter().find_map(|frame| {
                frame["nearby_npcs"].as_array()?.iter().find(|npc| {
                    npc["type"] == 477 && npc["total_health"].as_i64().is_some_and(|hp| hp > 0)
                }).map(|npc| json!({
                    "tick": frame["tick"], "health": npc["health"],
                    "total_health": npc["total_health"], "index": npc["index"],
                }))
            }),
            "m4_conditional_eat": m4_eat,
            "m5_raised_protect_cleared_before_next_operation": m5_clear,
            "stop_cleared_only_combat_prayers": self.case == Case::M5Stop && stop_ready(&capture),
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

fn scenario_for(
    case: Case,
    stand: WorldTile,
    placement: Option<RangedPlacement>,
    capture: Arc<Mutex<CombatCapture>>,
) -> Scenario {
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
    // Ranged staging must observe arrival at the spawn before npcadd, then
    // finish at its five-tile firing stand without another stand teleport.
    let preparation_index = stand_index + if placement.is_some() { 2 } else { 1 };
    let preparation = preparation_steps(case, stand, placement, Arc::clone(&capture));
    scenario
        .steps
        .splice(preparation_index..preparation_index, preparation);
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

fn preparation_steps(
    case: Case,
    _stand: WorldTile,
    placement: Option<RangedPlacement>,
    capture: Arc<Mutex<CombatCapture>>,
) -> Vec<Step> {
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
        Case::M5 | Case::M5Stop => &[
            ("attack", 0, 40),
            ("strength", 2, 40),
            ("defence", 1, 40),
            ("hitpoints", 3, 40),
            ("prayer", 5, 43),
        ],
        Case::R1 | Case::R2 | Case::R3 => &[
            ("ranged", 4, 70),
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
        Case::M5 | Case::M5Stop => {
            steps.push(wear_step(BRONZE_SCIMITAR_ID));
            steps.push(cheat_step(
                "seed user Thick Skin before Start",
                // R289 selected data maps prayer0 to varp83.
                "setvar prayer0 1".to_owned(),
                Proof::VarpExact { id: 83, value: 1 },
            ));
        }
        Case::M6 => steps.push(cheat_step(
            "drain Hitpoints to sixteen before hostile-area teleport",
            "~stat_drain hitpoints 24 0".to_owned(),
            Proof::Stat { id: 3, min: 16 },
        )),
        Case::R1 | Case::R2 | Case::R3 => {
            let placement = placement.expect("ranged case has a collision-backed placement");
            steps.push(ranged_npcadd_step(placement.spawn, Arc::clone(&capture)));
            steps.push(ranged_spawn_observed_step(
                placement.spawn,
                Arc::clone(&capture),
            ));
            steps.push(ranged_tele_step(
                placement.spawn,
                placement.stand,
                Arc::clone(&capture),
            ));
            steps.push(ranged_tele_observed_step(Arc::clone(&capture)));
        }
    }
    steps
}

fn ranged_spawn_event(capture: &CombatCapture) -> Option<&Value> {
    capture
        .random_events
        .iter()
        .rev()
        .find(|event| event["kind"] == json!("RangedNpcAdd"))
}

fn ranged_spawn_event_mut(capture: &mut CombatCapture) -> Option<&mut Value> {
    capture
        .random_events
        .iter_mut()
        .rev()
        .find(|event| event["kind"] == json!("RangedNpcAdd"))
}

fn tile_value(tile: WorldTile) -> Value {
    json!({"x": tile.x, "z": tile.z, "level": tile.level})
}

fn value_tile(value: &Value) -> Option<WorldTile> {
    Some(WorldTile {
        x: i32::try_from(value["x"].as_i64()?).ok()?,
        z: i32::try_from(value["z"].as_i64()?).ok()?,
        level: i32::try_from(value["level"].as_i64()?).ok()?,
    })
}
#[derive(Clone, Copy)]
struct SelectedRangedFacts {
    weapon_id: i32,
    ammo_id: i32,
    weapon_family: RangedAmmoFamily,
    ammo_family: RangedAmmoFamily,
    rapid: RangedModeFact,
    tab_root_id: i32,
    mode_varp: i32,
}

fn ranged_aliases(case: Case) -> (&'static str, &'static str) {
    match case {
        Case::R1 => ("maple_shortbow", "steel_arrow"),
        Case::R2 => ("maple_shortbow", "bolt"),
        Case::R3 => ("bronze_dart", "bronze_dart"),
        _ => unreachable!("ranged aliases requested for a melee case"),
    }
}

fn selected_ranged_facts(
    case: Case,
    selected: &SelectedGameData,
) -> Result<SelectedRangedFacts, String> {
    let (weapon_alias, ammo_alias) = ranged_aliases(case);
    let weapon_id = selected
        .item_by_alias(weapon_alias)
        .map(|item| item.id)
        .ok_or_else(|| format!("selected content has no `{weapon_alias}` item"))?;
    let ammo_id = selected
        .item_by_alias(ammo_alias)
        .map(|item| item.id)
        .ok_or_else(|| format!("selected content has no `{ammo_alias}` item"))?;
    let weapon = selected
        .ranged_weapons()
        .iter()
        .find(|fact| fact.obj_id == weapon_id)
        .ok_or_else(|| format!("selected content has no ranged weapon fact for {weapon_alias}"))?;
    let ammo_family = if ammo_id == weapon_id
        && matches!(
            weapon.ammo_family,
            RangedAmmoFamily::Thrown | RangedAmmoFamily::Javelin
        ) {
        weapon.ammo_family
    } else {
        selected
            .ranged_ammo()
            .iter()
            .find(|fact| fact.obj_id == ammo_id)
            .map(|fact| fact.family)
            .ok_or_else(|| format!("selected content has no ranged ammo fact for {ammo_alias}"))?
    };
    let tab = selected
        .weapon_styles()
        .iter()
        .find(|fact| fact.obj_id == weapon_id)
        .and_then(|fact| fact.tab)
        .ok_or_else(|| format!("selected content has no combat tab for {weapon_alias}"))?;
    let tab_root_id = selected
        .combat_tabs()
        .iter()
        .find(|fact| fact.tab == tab)
        .map(|fact| fact.root_id)
        .ok_or_else(|| format!("selected content has no interface root for combat tab {tab}"))?;
    let rapid = selected
        .ranged_modes()
        .iter()
        .copied()
        .find(|fact| fact.tab == tab && fact.mode == 1)
        .ok_or_else(|| format!("selected content has no rapid mode for combat tab {tab}"))?;
    let mode_varp = selected
        .ranged_mode_varp()
        .ok_or_else(|| "selected content has no ranged mode varp".to_owned())?;
    Ok(SelectedRangedFacts {
        weapon_id,
        ammo_id,
        weapon_family: weapon.ammo_family,
        ammo_family,
        rapid,
        tab_root_id,
        mode_varp,
    })
}

fn ranged_selected_facts_event(case: Case, selected: &SelectedGameData) -> Value {
    match selected_ranged_facts(case, selected) {
        Ok(facts) => {
            let (weapon_alias, ammo_alias) = ranged_aliases(case);
            let weapon = selected
                .ranged_weapons()
                .iter()
                .find(|fact| fact.obj_id == facts.weapon_id)
                .expect("ranged fact lookup already succeeded");
            let ammo = selected
                .ranged_ammo()
                .iter()
                .find(|fact| fact.obj_id == facts.ammo_id);
            json!({
                "kind": "RangedSelectedFacts",
                "case": case.key(),
                "weapon_alias": weapon_alias,
                "weapon_obj_id": facts.weapon_id,
                "weapon_family": format!("{:?}", facts.weapon_family),
                "weapon_attackrange": weapon.attackrange,
                "weapon_levelrequire": weapon.levelrequire,
                "ammo_alias": ammo_alias,
                "ammo_obj_id": facts.ammo_id,
                "ammo_family": format!("{:?}", facts.ammo_family),
                "ammo_levelrequire": ammo.map_or(weapon.levelrequire, |fact| fact.levelrequire),
                "rapid": {
                    "tab": facts.rapid.tab,
                    "slot": facts.rapid.slot,
                    "mode": facts.rapid.mode,
                    "button": facts.rapid.button,
                },
                "tab_root_id": facts.tab_root_id,
                "mode_varp": facts.mode_varp,
            })
        }
        Err(error) => json!({
            "kind": "RangedSelectedFacts",
            "case": case.key(),
            "error": error,
        }),
    }
}

fn record_ranged_probe(
    client: &client::client::Client,
    snapshot: &GameSnapshot,
    selected: &SelectedGameData,
    capture: &Arc<Mutex<CombatCapture>>,
    last_signature: &mut Option<String>,
) {
    let tick = i64::from(snapshot.tick());
    let local_tile = snapshot
        .tile()
        .map(|(x, z, level)| WorldTile { x, z, level });
    let projectiles = snapshot
        .projectiles()
        .iter()
        .map(|projectile| {
            let target = projectile.target;
            let target_index = target
                .filter(|target| target.kind == ActorKind::Npc)
                .map(|target| target.index);
            let target_tile = target_index.and_then(|index| {
                snapshot
                    .npcs()
                    .iter()
                    .find(|npc| npc.index == index)
                    .map(|npc| (npc.network, npc.size.max(1)))
            });
            json!({
                "spotanim": projectile.spotanim,
                "level": projectile.level,
                "src": projectile.src,
                "t1": projectile.t1,
                "t2": projectile.t2,
                "target": target.map(|target| {
                    json!({"kind": format!("{:?}", target.kind), "index": target.index})
                }),
                "target_tile": target_tile.map(|(tile, _)| tile),
                "distance": local_tile.zip(target_tile).map(|(from, (to, size))| {
                    let closest = WorldTile {
                        x: from.x.clamp(to.x, to.x + size - 1),
                        z: from.z.clamp(to.z, to.z + size - 1),
                        level: to.level,
                    };
                    tile_distance(from, closest)
                }),
                "local_launch": local_tile.is_some_and(|tile| projectile.src == tile),
            })
        })
        .collect::<Vec<_>>();
    let npc_hitmarks = client
        .npc_ids
        .iter()
        .filter_map(|index| {
            let slot = usize::try_from(*index).ok()?;
            let npc = client.npc.get(slot)?.as_ref()?;
            (npc.r#type == Some(WARLORD_NPC_ID)).then(|| {
                let tile = snapshot
                    .npcs()
                    .iter()
                    .find(|view| view.index == slot)
                    .map(|view| view.tile);
                json!({
                    "index": slot,
                    "type": npc.r#type,
                    "tile": tile,
                    "damage_values": npc.entity.damage_values,
                    "damage_types": npc.entity.damage_types,
                    "damage_cycles": npc.entity.damage_cycles,
                })
            })
        })
        .collect::<Vec<_>>();
    let mode_varp = selected.ranged_mode_varp().and_then(|id| {
        snapshot
            .varps()
            .iter()
            .find(|row| row.index == id)
            .map(|row| row.value)
    });
    let active_side_tab = snapshot.active_side_tab();
    let combat_tab = snapshot.side_tabs().iter().find(|tab| tab.index == 0);
    let combat_root_id = combat_tab.map(|tab| tab.root_component_id);
    let combat_style_buttons = combat_tab
        .map(|tab| {
            api::query::widget_search::combat_style_labels(
                snapshot,
                tab.root_component_id,
                selected.ranged_mode_varp().unwrap_or(-1),
            )
            .into_iter()
            .map(|style| {
                json!({
                    "component_id": style.component_id,
                    "mode": style.mode,
                    "label": style.label,
                })
            })
            .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let signature = serde_json::to_string(&(
        local_tile,
        active_side_tab,
        combat_root_id,
        mode_varp,
        &combat_style_buttons,
        &projectiles,
        &npc_hitmarks,
    ))
    .unwrap_or_default();
    if last_signature.as_deref() == Some(&signature) {
        return;
    }
    *last_signature = Some(signature);
    capture
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .random_events
        .push(json!({
            "kind": "RangedProbe",
            "tick": tick,
            "snapshot_tick": snapshot.tick(),
            "loop_cycle": client.loop_cycle,
            "tile": local_tile,
            "active_side_tab": active_side_tab,
            "combat_root_id": combat_root_id,
            "combat_style_buttons": combat_style_buttons,
            "mode_varp": mode_varp,
            "projectiles": projectiles,
            "npc_hitmarks": npc_hitmarks,
        }));
}

fn tele_cheat_command(tile: WorldTile) -> String {
    format!(
        "tele {},{},{},{},{}",
        tile.level,
        tile.x >> 6,
        tile.z >> 6,
        tile.x & 63,
        tile.z & 63
    )
}

fn ranged_npcadd_step(spawn: WorldTile, capture: Arc<Mutex<CombatCapture>>) -> Step {
    Step {
        name: "spawn a local Khazard Warlord before Start",
        kind: StepKind::Perform {
            send: Box::new(move |client, snapshot| {
                let before_indices = snapshot
                    .npcs()
                    .iter()
                    .filter(|npc| npc.r#type == Some(WARLORD_NPC_ID))
                    .map(|npc| npc.index)
                    .collect::<Vec<_>>();
                let sent = matches!(
                    api::interact::cheat(client, "npcadd khazard_warlord"),
                    client::CheatSend::Sent
                );
                if sent {
                    capture
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .random_events
                        .push(json!({
                            "kind": "RangedNpcAdd",
                            "spawn_tile": tile_value(spawn),
                            "preexisting_indices": before_indices,
                            "spawned_index": null,
                            "stand_tile": null,
                            "tele_observed": false,
                        }));
                }
                sent
            }),
        },
        wait: Wait {
            // The following Await checks the unique spawned index in world
            // coordinates; NpcAt uses scene-local actor coordinates.
            arm: Proof::IngameScene2,
            budget_ticks: 120,
        },
    }
}

fn ranged_spawn_observed_step(spawn: WorldTile, capture: Arc<Mutex<CombatCapture>>) -> Step {
    let ready_capture = Arc::clone(&capture);
    Step {
        name: "observe the newly added Warlord before moving five tiles",
        kind: StepKind::Await {
            evidence: "new NPC index at the fixture spawn",
            ready: Box::new(move |snapshot| {
                let mut capture = ready_capture.lock().unwrap_or_else(|e| e.into_inner());
                let Some(event) = ranged_spawn_event_mut(&mut capture) else {
                    return false;
                };
                if event["spawned_index"].is_i64() {
                    return true;
                }
                let before = event["preexisting_indices"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_i64)
                    .collect::<std::collections::HashSet<_>>();
                let spawned = snapshot
                    .npcs()
                    .iter()
                    .filter(|npc| npc.r#type == Some(WARLORD_NPC_ID) && npc.tile == spawn)
                    .find(|npc| !before.contains(&(npc.index as i64)));
                if let Some(npc) = spawned {
                    event["spawned_index"] = json!(npc.index);
                    true
                } else {
                    false
                }
            }),
        },
        wait: Wait {
            arm: Proof::IngameScene2,
            budget_ticks: 120,
        },
    }
}

fn ranged_tele_step(
    spawn: WorldTile,
    stand: WorldTile,
    capture: Arc<Mutex<CombatCapture>>,
) -> Step {
    Step {
        name: "teleport exactly five tiles from the new Warlord",
        kind: StepKind::Perform {
            send: Box::new(move |client, snapshot| {
                let mut capture = capture.lock().unwrap_or_else(|e| e.into_inner());
                let Some(event) = ranged_spawn_event_mut(&mut capture) else {
                    return false;
                };
                let Some(spawned_index) = event["spawned_index"].as_i64() else {
                    return false;
                };
                if tile_distance(spawn, stand) != 5
                    || !snapshot.npcs().iter().any(|npc| {
                        npc.index as i64 == spawned_index && npc.r#type == Some(WARLORD_NPC_ID)
                    })
                {
                    return false;
                }
                let command = tele_cheat_command(stand);
                let sent = matches!(
                    api::interact::cheat(client, &command),
                    client::CheatSend::Sent
                );
                if sent {
                    event["stand_tile"] = tile_value(stand);
                    event["tele_command"] = json!(command);
                }
                sent
            }),
        },
        wait: Wait {
            arm: Proof::IngameScene2,
            budget_ticks: 120,
        },
    }
}

fn ranged_tele_observed_step(capture: Arc<Mutex<CombatCapture>>) -> Step {
    let ready_capture = Arc::clone(&capture);
    Step {
        name: "observe the exact five-tile ranged start before Start",
        kind: StepKind::Await {
            evidence: "teleport and NPC spawn baseline",
            ready: Box::new(move |snapshot| {
                let mut capture = ready_capture.lock().unwrap_or_else(|e| e.into_inner());
                let Some(event) = ranged_spawn_event_mut(&mut capture) else {
                    return false;
                };
                let Some(stand) = value_tile(&event["stand_tile"]) else {
                    return false;
                };
                let Some(spawned_index) = event["spawned_index"].as_i64() else {
                    return false;
                };
                let arrived = snapshot.tile() == Some((stand.x, stand.z, stand.level));
                let still_present = snapshot.npcs().iter().any(|npc| {
                    npc.index as i64 == spawned_index && npc.r#type == Some(WARLORD_NPC_ID)
                });
                if arrived && still_present {
                    event["tele_observed"] = json!(true);
                    true
                } else {
                    false
                }
            }),
        },
        wait: Wait {
            arm: Proof::IngameScene2,
            budget_ticks: 120,
        },
    }
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
#[derive(Clone)]
struct RangedPlacement {
    spawn: WorldTile,
    stand: WorldTile,
}

fn ranged_placement(case: Case, world: &nav::world::NavWorld) -> Result<RangedPlacement, String> {
    // Killing cells share a quiet parcel; keep the non-attacking ammo cell
    // separate because its staged actor survives the proof.
    let offset = match case {
        Case::R1 | Case::R3 => 32,
        Case::R2 => 64,
        _ => unreachable!("ranged placement requested for a melee case"),
    };
    // R2 must refuse before attacking, so only its spawn-to-stand corridor
    // needs clearing. Fighting cells additionally need the full wander margin.
    let (x_clear, z_clear) = if case == Case::R2 {
        (0..=6, 0..=1)
    } else {
        (-5..=11, -5..=6)
    };
    for dx in -8..=8 {
        for dz in -256..=256 {
            let spawn = WorldTile {
                x: IMP_START.x - offset + dx,
                z: IMP_START.z + dz,
                level: 0,
            };
            if x_clear.clone().all(|x| {
                z_clear.clone().all(|z| {
                    world.collision.standable(WorldTile {
                        x: spawn.x + x,
                        z: spawn.z + z,
                        level: 0,
                    })
                })
            }) {
                return Ok(RangedPlacement {
                    spawn,
                    stand: WorldTile {
                        x: spawn.x + 5,
                        ..spawn
                    },
                });
            }
        }
    }
    Err("no clear five-tile ranged staging corridor in the assigned parcel".to_owned())
}

fn tile_distance(a: WorldTile, b: WorldTile) -> i32 {
    if a.level != b.level {
        i32::MAX
    } else {
        (a.x - b.x).abs().max((a.z - b.z).abs())
    }
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
        Case::M5 | Case::M5Stop => {
            if stat_base(baseline, "prayer") != Some(43)
                || prayer_varp(baseline, 83) != Some(1)
                || prayer_varp(baseline, 97) != Some(0)
            {
                return Some(
                    "M5 did not observe user Thick Skin active and Protect from Melee off before Start"
                        .into(),
                );
            }
        }
        Case::R1 | Case::R2 | Case::R3 => {}
    }
    None
}
fn ranged_start_preflight(
    case: Case,
    baseline: &Value,
    selected: &SelectedGameData,
    capture: &CombatCapture,
) -> Option<String> {
    if baseline["ingame"] != json!(true) || baseline["scene_state"] != json!(2) {
        return Some("ranged Start baseline is not an attached in-game scene".to_owned());
    }
    let Some(spawn_event) = ranged_spawn_event(capture) else {
        return Some("ranged Start has no recorded npcadd fixture".to_owned());
    };
    let (Some(spawn), Some(stand), Some(spawned_index)) = (
        value_tile(&spawn_event["spawn_tile"]),
        value_tile(&spawn_event["stand_tile"]),
        spawn_event["spawned_index"].as_i64(),
    ) else {
        return Some("ranged Start fixture lacks its spawn identity or stand tile".to_owned());
    };
    let Some(player_tile) = value_tile(&baseline["local_player"]["tile"]) else {
        return Some("ranged Start baseline has no local world tile".to_owned());
    };
    if spawn_event["tele_observed"] != true
        || tile_distance(spawn, stand) != 5
        || player_tile != stand
        || !baseline["nearby_npcs"].as_array().is_some_and(|npcs| {
            npcs.iter().any(|npc| {
                npc["index"] == json!(spawned_index) && npc["type"] == json!(WARLORD_NPC_ID)
            })
        })
    {
        return Some(
            "ranged Start did not preserve the spawned Warlord identity and observed staging stand"
                .to_owned(),
        );
    }
    if stat_pair(baseline, "ranged") != Some((70, 70))
        || stat_pair(baseline, "defence") != Some((40, 40))
        || stat_pair(baseline, "hitpoints") != Some((40, 40))
        || stat_pair(baseline, "prayer") != Some((43, 43))
    {
        return Some(
            "ranged Start did not reach ranged70, defence40, hitpoints40, prayer43".to_owned(),
        );
    }
    let facts = match selected_ranged_facts(case, selected) {
        Ok(facts) => facts,
        Err(error) => return Some(error),
    };
    let (weapon_alias, ammo_alias) = ranged_aliases(case);
    let (expected_weapon_family, expected_ammo_family, expected_ammo_count) = match case {
        Case::R1 => (RangedAmmoFamily::Arrow, RangedAmmoFamily::Arrow, 150),
        Case::R2 => (RangedAmmoFamily::Arrow, RangedAmmoFamily::Bolt, 50),
        Case::R3 => (RangedAmmoFamily::Thrown, RangedAmmoFamily::Thrown, 200),
        _ => unreachable!("ranged preflight requested for a melee case"),
    };
    let expected_weapon_count = if facts.weapon_id == facts.ammo_id {
        expected_ammo_count
    } else {
        1
    };
    if facts.weapon_family != expected_weapon_family
        || facts.ammo_family != expected_ammo_family
        || item_count(baseline, facts.weapon_id) != expected_weapon_count
        || item_count(baseline, facts.ammo_id) != expected_ammo_count
        || item_count(baseline, PRAYER_POTION_4_ID) != 1
    {
        return Some(format!(
            "{} did not seed {}×{}, {}×{}, prayer restore×1 with source-backed ranged families",
            case.key(),
            weapon_alias,
            expected_weapon_count,
            ammo_alias,
            expected_ammo_count,
        ));
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
        Case::M5Stop => stop_ready(capture),
        Case::M6 => m6_ready(capture),
        Case::R1 | Case::R2 | Case::R3 => ranged_ready(case, capture),
    }
}
fn ranged_selected_event(capture: &CombatCapture) -> Option<&Value> {
    capture
        .random_events
        .iter()
        .find(|event| event["kind"] == json!("RangedSelectedFacts"))
}

fn ranged_probes(capture: &CombatCapture) -> impl Iterator<Item = &Value> {
    capture
        .random_events
        .iter()
        .filter(|event| event["kind"] == json!("RangedProbe"))
}

fn ranged_spawned_index(capture: &CombatCapture) -> Option<i64> {
    ranged_spawn_event(capture)?["spawned_index"].as_i64()
}

fn ranged_launches(capture: &CombatCapture) -> Vec<Value> {
    let Some(spawned_index) = ranged_spawned_index(capture) else {
        return Vec::new();
    };
    let mut launches: Vec<Value> = Vec::new();
    for probe in ranged_probes(capture) {
        let Some(projectiles) = probe["projectiles"].as_array() else {
            continue;
        };
        for projectile in projectiles {
            if projectile["local_launch"] != true
                || projectile["target"]["kind"] != json!("Npc")
                || projectile["target"]["index"].as_i64() != Some(spawned_index)
            {
                continue;
            }
            let (Some(t1), Some(t2), Some(distance), Some(launch_tick), Some(launch_cycle)) = (
                projectile["t1"].as_i64(),
                projectile["t2"].as_i64(),
                projectile["distance"].as_i64(),
                probe["tick"].as_i64(),
                probe["loop_cycle"].as_i64(),
            ) else {
                continue;
            };
            let spotanim = projectile["spotanim"].clone();
            if launches
                .iter()
                .any(|launch| launch["t1"] == json!(t1) && launch["spotanim"] == spotanim)
            {
                continue;
            }
            launches.push(json!({
                "spotanim": spotanim,
                "t1": t1,
                "t2": t2,
                "flight_cycles": t2 - t1,
                "distance": distance,
                "launch_tick": launch_tick,
                "snapshot_tick": probe["snapshot_tick"],
                "launch_cycle": launch_cycle,
            }));
        }
    }
    launches.sort_by_key(|launch| launch["launch_cycle"].as_i64().unwrap_or(i64::MAX));
    launches
}

fn ranged_hitmark_onsets(capture: &CombatCapture) -> Vec<Value> {
    let Some(spawned_index) = ranged_spawned_index(capture) else {
        return Vec::new();
    };
    let mut seen = std::collections::HashSet::new();
    let mut onsets = Vec::new();
    for probe in ranged_probes(capture) {
        let Some(npcs) = probe["npc_hitmarks"].as_array() else {
            continue;
        };
        for npc in npcs
            .iter()
            .filter(|npc| npc["index"].as_i64() == Some(spawned_index))
        {
            let (Some(values), Some(types), Some(cycles)) = (
                npc["damage_values"].as_array(),
                npc["damage_types"].as_array(),
                npc["damage_cycles"].as_array(),
            ) else {
                continue;
            };
            for (slot, ((value, kind), cycle)) in values.iter().zip(types).zip(cycles).enumerate() {
                let (Some(value), Some(kind), Some(cycle)) =
                    (value.as_i64(), kind.as_i64(), cycle.as_i64())
                else {
                    continue;
                };
                if cycle <= 0 || !seen.insert((slot, value, kind, cycle)) {
                    continue;
                }
                onsets.push(json!({
                    "tick": probe["tick"],
                    "loop_cycle": probe["loop_cycle"],
                    "slot": slot,
                    "value": value,
                    "type": kind,
                    "damage_cycle": cycle,
                }));
            }
        }
    }
    onsets
}

fn ranged_timing_receipt(case: Case, capture: &CombatCapture) -> Value {
    let launches = ranged_launches(capture);
    let impacts = ranged_hitmark_onsets(capture);
    let rate = if case == Case::R3 { 2 } else { 3 };
    let damage_delay = if case == Case::R3 { 32 } else { 46 };
    let flight_length = if case == Case::R3 { 0 } else { 5 };
    let mut cadence_rows = Vec::new();
    let mut cadence_ok = true;
    let mut cadence_samples = 0usize;
    for pair in launches.windows(2) {
        let previous_tick = pair[0]["launch_tick"].as_i64().unwrap_or(i64::MIN);
        let tick = pair[1]["launch_tick"].as_i64().unwrap_or(i64::MAX);
        let interval = tick - previous_tick;
        let interrupted = capture.actions.iter().any(|action| {
            action["snapshot_tick"].as_i64().is_some_and(|action_tick| {
                previous_tick < action_tick
                    && action_tick <= tick
                    && is_fight_clearing_action(action)
            })
        });
        if !interrupted {
            cadence_samples += 1;
            cadence_ok &= interval == rate;
        }
        cadence_rows.push(json!({
            "from_tick": previous_tick,
            "to_tick": tick,
            "interval": interval,
            "interrupted_by_clearing_action": interrupted,
            "valid": interrupted || interval == rate,
        }));
    }
    let flight_rows = launches
        .iter()
        .map(|launch| {
            let distance = launch["distance"].as_i64().unwrap_or(-1);
            let expected = i64::from(flight_length) + 5 * distance;
            json!({
                "distance": distance,
                "t1": launch["t1"],
                "t2": launch["t2"],
                "observed_cycles": launch["flight_cycles"],
                "expected_cycles": expected,
                "valid": launch["flight_cycles"].as_i64() == Some(expected),
            })
        })
        .collect::<Vec<_>>();
    let impact_rows = launches
        .iter()
        .enumerate()
        .map(|(index, launch)| {
            let distance = launch["distance"].as_i64().unwrap_or(-1);
            let expected = (i64::from(damage_delay) + 5 * distance + 30) / 30;
            let impact = impacts.get(index);
            let observed = impact.and_then(|impact| {
                Some(impact["tick"].as_i64()? - launch["launch_tick"].as_i64()?)
            });
            json!({
                "launch_tick": launch["launch_tick"],
                "distance": distance,
                "expected_impact_ticks": expected,
                "impact": impact,
                "observed_impact_ticks": observed,
                "valid": observed == Some(expected),
            })
        })
        .collect::<Vec<_>>();
    let distinct_distances = launches
        .iter()
        .filter_map(|launch| launch["distance"].as_i64())
        .collect::<std::collections::BTreeSet<_>>();
    let flight_valid =
        !launches.is_empty() && flight_rows.iter().all(|row| row["valid"] == json!(true));
    let impact_valid = !launches.is_empty()
        && impacts.len() == launches.len()
        && impact_rows.iter().all(|row| row["valid"] == json!(true));
    let cadence_valid = cadence_samples >= 2 && cadence_ok;
    let distance_valid = case != Case::R1
        || (distinct_distances.len() >= 2
            && distinct_distances.iter().any(|distance| *distance >= 3));
    json!({
        "expected_launch_rate_ticks": rate,
        "launches": launches,
        "hitmark_onsets": impacts,
        "cadence_intervals": cadence_rows,
        "cadence_uninterrupted_samples": cadence_samples,
        "cadence_valid": cadence_valid,
        "flights": flight_rows,
        "flight_valid": flight_valid,
        "impacts": impact_rows,
        "impact_valid": impact_valid,
        "distinct_distances": distinct_distances,
        "distance_valid": distance_valid,
    })
}

fn normalized_name(value: &str) -> String {
    value
        .to_ascii_lowercase()
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .collect()
}

fn ranged_style_timing(capture: &CombatCapture) -> Value {
    let selected = ranged_selected_event(capture);
    let weapon_wear = selected.and_then(|selected| {
        capture.actions.iter().find(|action| {
            action["request"]["op"] == "wear"
                && action["request"]["name"].as_str().is_some_and(|name| {
                    selected["weapon_alias"]
                        .as_str()
                        .is_some_and(|alias| normalized_name(name) == normalized_name(alias))
                })
        })
    });
    let style = selected.and_then(|selected| {
        capture.actions.iter().find(|action| {
            action["request"]["op"] == "if-button"
                && action["request"]["component_id"] == selected["rapid"]["button"]
        })
    });
    json!({
        "weapon_wear_snapshot_tick": weapon_wear.map(|action| &action["snapshot_tick"]),
        "style_snapshot_tick": style.map(|action| &action["snapshot_tick"]),
        "wear_to_style_ticks": weapon_wear.and_then(|wear| {
            style?["snapshot_tick"].as_i64()?.checked_sub(wear["snapshot_tick"].as_i64()?)
        }),
        "oracle": "first native poll observing equipped weapon and exact combat tab",
    })
}

fn ranged_style_prep_ok(case: Case, capture: &CombatCapture) -> bool {
    let Some(selected) = ranged_selected_event(capture) else {
        return false;
    };
    let Some(first_attack) = first_attack_action(capture) else {
        return false;
    };
    let Some(first_attack_sequence) = first_attack["sequence"].as_u64() else {
        return false;
    };
    let Some(first_attack_tick) = first_attack["snapshot_tick"].as_i64() else {
        return false;
    };
    let Some(button) = selected["rapid"]["button"].as_i64() else {
        return false;
    };
    if selected["rapid"]["mode"] != json!(1) {
        return false;
    }
    let Some(style_action) = capture.actions.iter().find(|action| {
        action["request"]["op"] == json!("if-button")
            && action["request"]["component_id"] == json!(button)
            && action["sequence"]
                .as_u64()
                .is_some_and(|sequence| sequence < first_attack_sequence)
            && action_wire_valid(action)
    }) else {
        return false;
    };
    let Some(style_tick) = style_action["snapshot_tick"].as_i64() else {
        return false;
    };
    let (weapon_alias, ammo_alias) = ranged_aliases(case);
    let wears = capture
        .actions
        .iter()
        .filter(|action| {
            action["request"]["op"] == json!("wear")
                && action["sequence"]
                    .as_u64()
                    .is_some_and(|sequence| sequence < first_attack_sequence)
                && action_wire_valid(action)
        })
        .collect::<Vec<_>>();
    let named_wear = |alias: &str| {
        let expected = normalized_name(alias);
        wears.iter().copied().find(|action| {
            action["request"]["name"]
                .as_str()
                .is_some_and(|name| normalized_name(name) == expected)
        })
    };
    let Some(weapon_wear) = named_wear(weapon_alias) else {
        return false;
    };
    if case == Case::R1 {
        let Some(ammo_wear) = named_wear(ammo_alias) else {
            return false;
        };
        if weapon_wear["batch"].as_u64().filter(|batch| *batch > 0)
            != ammo_wear["batch"].as_u64().filter(|batch| *batch > 0)
            || weapon_wear["tick"] != ammo_wear["tick"]
            || weapon_wear["sequence"].as_u64() >= ammo_wear["sequence"].as_u64()
            || ammo_wear["sequence"].as_u64() >= style_action["sequence"].as_u64()
        {
            return false;
        }
    } else if weapon_wear["sequence"].as_u64() >= style_action["sequence"].as_u64() {
        return false;
    }
    let wear_tick = weapon_wear["snapshot_tick"].as_i64().unwrap_or(i64::MIN);
    let tab_root_id = selected["tab_root_id"].as_i64().unwrap_or(-1);
    let first_eligible_poll = capture.observations.iter().find(|observation| {
        observation["native_tick_edge"] == true
            && observation["snapshot_tick"]
                .as_i64()
                .is_some_and(|tick| tick > wear_tick)
            && observation["combat_root_id"] == selected["tab_root_id"]
            && observation["equipment"]
                .as_array()
                .is_some_and(|items| items.contains(&selected["weapon_obj_id"]))
    });
    if first_eligible_poll.is_none_or(|poll| {
        poll["snapshot_tick"] != style_action["snapshot_tick"]
            || poll["host_tick"] != style_action["tick"]
    }) {
        return false;
    }
    let tab_observed = tab_root_id >= 0
        && ranged_probes(capture).any(|probe| {
            probe["combat_root_id"] == json!(tab_root_id)
                && probe["combat_style_buttons"]
                    .as_array()
                    .is_some_and(|buttons| {
                        buttons.iter().any(|style| {
                            style["component_id"] == json!(button) && style["mode"] == json!(1)
                        })
                    })
                && probe["tick"]
                    .as_i64()
                    .is_some_and(|tick| wear_tick <= tick && tick <= style_tick)
        });
    let mode_echoed = ranged_probes(capture).any(|probe| {
        probe["mode_varp"] == json!(1)
            && probe["tick"]
                .as_i64()
                .is_some_and(|tick| style_tick <= tick && tick <= first_attack_tick)
    });
    tab_observed && mode_echoed
}

fn is_take_of_alias(action: &Value, alias: &str) -> bool {
    let request = &action["request"];
    let take = request["action"]
        .as_str()
        .is_some_and(|action| action.eq_ignore_ascii_case("take"))
        || request["debug"]
            .as_str()
            .is_some_and(|debug| debug.to_ascii_lowercase().contains("take"));
    if !take {
        return false;
    }
    let expected = normalized_name(alias);
    request["name"]
        .as_str()
        .is_some_and(|name| normalized_name(name).contains(&expected))
        || request["debug"]
            .as_str()
            .is_some_and(|debug| normalized_name(debug).contains(&expected))
}

fn ranged_pickup_count(action: &Value, after: &Value, ammo_id: i32) -> Option<i64> {
    let request = &action["request"];
    let (x, z, level) = (
        request["x"].as_i64()?,
        request["z"].as_i64()?,
        request["level"].as_i64()?,
    );
    let stacks = |frame: &Value| -> Option<Vec<i64>> {
        let mut counts = Vec::new();
        for item in frame["ground_items"].as_array()? {
            if item["id"].as_i64() == Some(i64::from(ammo_id))
                && item["tile"]["x"].as_i64() == Some(x)
                && item["tile"]["z"].as_i64() == Some(z)
                && item["tile"]["level"].as_i64() == Some(level)
            {
                counts.push(item["count"].as_i64()?);
            }
        }
        counts.sort_unstable();
        Some(counts)
    };
    let before = stacks(&action["snapshot"])?;
    let after = stacks(after)?;
    // One Take removes one stack, not every same-ID stack on its tile.
    // Compare independent ground observations; do not infer units from inventory.
    if before.len() != after.len() + 1 {
        return None;
    }
    let removed = before
        .iter()
        .zip(&after)
        .position(|(before, after)| before != after)
        .unwrap_or(after.len());
    (before[removed] > 0 && before[removed + 1..] == after[removed..]).then_some(before[removed])
}

fn ranged_dart_count_steps(
    capture: &CombatCapture,
    ammo_id: i32,
    launches: &[Value],
) -> Vec<Value> {
    let ammo_count = |frame: &Value| item_count(frame, ammo_id) + equipment_count(frame, ammo_id);
    launches
        .iter()
        .map(|launch| {
            let snapshot_tick = launch["snapshot_tick"].as_i64();
            let before = snapshot_tick.and_then(|tick| {
                capture
                    .frames
                    .iter()
                    .rfind(|frame| {
                        frame["snapshot_tick"]
                            .as_i64()
                            .is_some_and(|frame_tick| frame_tick < tick)
                    })
                    .map(&ammo_count)
            });
            let after = snapshot_tick.and_then(|tick| {
                capture
                    .frames
                    .iter()
                    .rfind(|frame| frame["snapshot_tick"].as_i64() == Some(tick))
                    .or_else(|| {
                        capture.frames.iter().find(|frame| {
                            frame["snapshot_tick"]
                                .as_i64()
                                .is_some_and(|frame_tick| frame_tick > tick)
                        })
                    })
                    .map(&ammo_count)
            });
            let decrement = before.zip(after).map(|(before, after)| before - after);
            json!({
                "launch_tick": launch["launch_tick"],
                "snapshot_tick": snapshot_tick,
                "before": before,
                "after": after,
                "decrement": decrement,
                "valid": decrement == Some(1),
            })
        })
        .collect()
}

fn ranged_corpse_frame<'a>(capture: &'a CombatCapture, report: &Value) -> Option<&'a Value> {
    let index = integer(report, "combat_engaged_index")?;
    let kind = integer(report, "combat_engaged_npc_type")?;
    let start = capture.start_baseline.as_ref()?["tick"].as_i64()?;
    let end = integer(report, "combat_evidence_tick")?;
    capture.frames.iter().find(|frame| {
        frame["tick"]
            .as_i64()
            .is_some_and(|tick| (start..=end).contains(&tick))
            && frame["nearby_npcs"].as_array().is_some_and(|npcs| {
                npcs.iter().any(|npc| {
                    npc["index"].as_i64() == Some(index)
                        && npc["type"].as_i64() == Some(kind)
                        && npc["health"] == json!(0)
                        && npc["total_health"].as_i64().is_some_and(|total| total > 0)
                })
            })
    })
}

fn ranged_ammo_receipt(case: Case, capture: &CombatCapture, report: &Value) -> Value {
    let Some(selected) = ranged_selected_event(capture) else {
        return json!({"valid": false, "error": "selected ranged fact receipt missing"});
    };
    let Some(ammo_id) = selected["ammo_obj_id"].as_i64() else {
        return json!({"valid": false, "error": "selected ammo object id missing"});
    };
    let Some(start) = capture.start_baseline.as_ref() else {
        return json!({"valid": false, "error": "start baseline missing"});
    };
    let Some(final_frame) = capture.frames.last() else {
        return json!({"valid": false, "error": "terminal item frame missing"});
    };
    let Some(corpse_frame) = ranged_corpse_frame(capture, report) else {
        return json!({"valid": false, "error": "observed target corpse missing"});
    };
    let killed_tick = corpse_frame["tick"].as_i64().unwrap();
    let ammo_id = ammo_id as i32;
    let ammo_start = item_count(start, ammo_id) + equipment_count(start, ammo_id);
    let ammo_final = item_count(final_frame, ammo_id) + equipment_count(final_frame, ammo_id);
    let ammo_at_kill = item_count(corpse_frame, ammo_id) + equipment_count(corpse_frame, ammo_id);
    let (_, ammo_alias) = ranged_aliases(case);
    let launches = ranged_launches(capture);
    let dart_count_steps = if case == Case::R3 {
        ranged_dart_count_steps(capture, ammo_id, &launches)
    } else {
        Vec::new()
    };
    let dart_count_per_launch = case != Case::R3
        || (!dart_count_steps.is_empty()
            && dart_count_steps.len() == launches.len()
            && dart_count_steps
                .iter()
                .all(|step| step["valid"] == json!(true)));
    let pickups = capture
        .actions
        .iter()
        .filter(|action| is_take_of_alias(action, ammo_alias))
        .collect::<Vec<_>>();
    let pickup_after_kill = pickups.iter().all(|action| {
        action["tick"]
            .as_i64()
            .is_some_and(|tick| tick >= killed_tick)
    });
    let take_wire = [
        i64::from(client::io::ClientProt289::MOVE_OPCLICK.id),
        i64::from(client::io::ClientProt289::OPOBJ3.id),
    ];
    let pickup_counts = pickups
        .iter()
        .enumerate()
        .map(|(index, action)| {
            let after = pickups
                .get(index + 1)
                .map_or(final_frame, |next| &next["snapshot"]);
            ranged_pickup_count(action, after, ammo_id)
        })
        .collect::<Vec<_>>();
    let pickup_counts_valid = pickup_counts
        .iter()
        .all(|count| count.is_some_and(|count| count > 0));
    let swept_units = pickup_counts
        .iter()
        .try_fold(0_i64, |total, count| total.checked_add((*count)?));
    let pickup_plan_valid = pickups.iter().all(|action| {
        action["request"]["op"] == json!("obj")
            && action["request"]["action"] == json!("Take")
            && action["batch"].as_u64().is_some_and(|batch| batch > 0)
            && plan_rows(capture, action).len() == 1
            && action_wire_valid(action)
            && wire_opcodes(action).is_some_and(|wire| {
                wire.len() == take_wire.len()
                    && wire
                        .iter()
                        .zip(take_wire)
                        .all(|(actual, expected)| *actual == expected)
            })
    }) && pickup_counts_valid;
    let expected_final = swept_units.map(|swept| ammo_start - launches.len() as i64 + swept);
    let count_matches_launches = expected_final == Some(ammo_final);
    let pickup_limit = pickups.len() <= 4;
    // These cells must exercise the sweep, not pass vacuously with ground ammo.
    let sweep_observed = !pickups.is_empty()
        && swept_units.is_some_and(|units| units > 0)
        && ammo_final > ammo_at_kill;
    let valid = !launches.is_empty()
        && count_matches_launches
        && pickup_limit
        && sweep_observed
        && pickup_after_kill
        && pickup_plan_valid
        && dart_count_per_launch;
    json!({
        "ammo_obj_id": ammo_id,
        "ammo_alias": ammo_alias,
        "start_count": ammo_start,
        "launches": launches.len(),
        "pickup_actions": pickups,
        "pickup_count": pickups.len(),
        "pickup_limit_valid": pickup_limit,
        "sweep_observed": sweep_observed,
        "corpse_tick": killed_tick,
        "held_count_at_kill": ammo_at_kill,
        "pickup_stack_counts": pickup_counts,
        "swept_units": swept_units,
        "all_pickups_after_kill": pickup_after_kill,
        "one_row_pickup_plans": pickup_plan_valid,
        "final_count": ammo_final,
        "expected_final_count": expected_final,
        "count_matches_launches_and_sweeps": count_matches_launches,
        "dart_count_per_launch": dart_count_per_launch,
        "dart_count_steps": dart_count_steps,
        "valid": valid,
    })
}

fn ranged_wrong_ammo_ready(capture: &CombatCapture) -> bool {
    let Some(report) = report_with_end(capture, "Aborted(PrepFailed(Ammo))") else {
        return false;
    };
    // `combat_proof::status_value` stores `ScriptStatus.failure` with `Debug`.
    let blocked_reason = "ScriptFailure { code: \"parked\", message: \"combat aborted; caller must handle the failure\" }";
    let quester_blocked = capture.statuses.iter().any(|status| {
        status["phase"] == json!("Blocked") && status["failure"].as_str() == Some(blocked_reason)
    });
    let no_attack = !capture.actions.iter().any(is_npc_attack);
    let probes = ranged_probes(capture).collect::<Vec<_>>();
    let no_projectile = !probes.is_empty()
        && probes.iter().all(|probe| {
            probe["projectiles"]
                .as_array()
                .is_some_and(|projectiles| projectiles.is_empty())
        });
    report["fields"]["combat_end"] == json!("Aborted(PrepFailed(Ammo))")
        && quester_blocked
        && no_attack
        && no_projectile
}

fn ranged_ready(case: Case, capture: &CombatCapture) -> bool {
    if case == Case::R2 {
        return ranged_wrong_ammo_ready(capture);
    }
    let Some(report) = report_with_end(capture, "Killed") else {
        return false;
    };
    let Some(spawned_index) = ranged_spawned_index(capture) else {
        return false;
    };
    let timing = ranged_timing_receipt(case, capture);
    let ammo = ranged_ammo_receipt(case, capture, report);
    let attacks = capture
        .actions
        .iter()
        .filter(|action| is_npc_attack(action))
        .collect::<Vec<_>>();
    let attacks_target_spawn = !attacks.is_empty()
        && attacks.iter().all(|action| {
            action["request"]["name"]
                .as_str()
                .is_some_and(|name| name.eq_ignore_ascii_case("Khazard Warlord"))
                && action["request"]["index"].as_i64() == Some(spawned_index)
                && action_wire_valid(action)
        });
    let launches_match_report =
        integer(report, "combat_swings") == Some(ranged_launches(capture).len() as i64);
    let plans = batch_plans(capture);
    report["fields"]["combat_engaged_index"].as_i64() == Some(spawned_index)
        && report["fields"]["combat_engaged_npc_type"] == json!(WARLORD_NPC_ID)
        && ranged_corpse_frame(capture, report).is_some()
        && attacks_target_spawn
        && launches_match_report
        && ranged_style_prep_ok(case, capture)
        && timing["flight_valid"] == json!(true)
        && timing["cadence_valid"] == json!(true)
        && timing["impact_valid"] == json!(true)
        && timing["distance_valid"] == json!(true)
        && ammo["valid"] == json!(true)
        && protection_timing_ok(capture)
        && m2_restoration_runs_ok(capture, report)
        && report_multi_op_count_matches(capture, report)
        && native_interactions_wire_valid(capture)
        && batch_plan_contract(capture, &plans)
        && no_attack_after_report(capture, report)
        && !capture
            .statuses
            .iter()
            .any(|status| status["fields"]["combat_end"] == json!("Died"))
        && has_real_attack_packet(capture)
}
fn ranged_receipt(case: Case, capture: &CombatCapture) -> Value {
    let selected = ranged_selected_event(capture)
        .cloned()
        .unwrap_or(Value::Null);
    let fixture = ranged_spawn_event(capture).cloned().unwrap_or(Value::Null);
    if case == Case::R2 {
        let local_projectiles = ranged_probes(capture)
            .flat_map(|probe| probe["projectiles"].as_array().into_iter().flatten())
            .filter(|projectile| projectile["local_launch"] == true)
            .count();
        return json!({
            "selected_facts": selected,
            "spawn": fixture,
            "exact_ammo_abort": ranged_wrong_ammo_ready(capture),
            "attack_requests": capture.actions.iter().filter(|action| is_npc_attack(action)).count(),
            "local_projectiles": local_projectiles,
            "blocked_statuses": capture.statuses.iter().filter(|status| {
                status["phase"] == json!("Blocked")
            }).collect::<Vec<_>>(),
        });
    }
    let timing = ranged_timing_receipt(case, capture);
    let report = report_with_end(capture, "Killed");
    let ammo = report.map_or_else(
        || json!({"valid": false, "error": "Killed report missing"}),
        |report| ranged_ammo_receipt(case, capture, report),
    );
    let plans = batch_plans(capture);
    json!({
        "selected_facts": selected,
        "spawn": fixture,
        "outcome": report.map(|report| report["fields"]["combat_end"].clone()),
        "style_prep_valid": ranged_style_prep_ok(case, capture),
        "style_timing": ranged_style_timing(capture),
        "launch_timing": timing,
        "ammo_sweep": ammo,
        "protect_before_first_onset_plus_two": protection_timing_ok(capture),
        "restoration_runs": report.is_some_and(|report| m2_restoration_runs_ok(capture, report)),
        "batch_plan_contract": batch_plan_contract(capture, &plans),
        "corpse_for_kill": report.is_some_and(|report| ranged_corpse_frame(capture, report).is_some()),
        "ready": ranged_ready(case, capture),
    })
}

fn stop_fight_raise(capture: &CombatCapture) -> Option<&Value> {
    capture.actions.iter().find(|raise| {
        raise["request"]["op"] == "if-button"
            && raise["request"]["component_id"] == 5623
            && action_wire_valid(raise)
            && prayer_varp(&raise["snapshot"], 97) == Some(0)
            && capture.actions.iter().any(|attack| {
                is_npc_attack(attack)
                    && attack["request"]["name"] == "Imp"
                    && action_wire_valid(attack)
                    && attack["run"] == raise["run"]
                    && attack["action_id"] == raise["action_id"]
                    && attack["sequence"].as_u64() < raise["sequence"].as_u64()
            })
    })
}

fn stop_ready(capture: &CombatCapture) -> bool {
    let Some(stop) = capture
        .random_events
        .iter()
        .find(|event| event["kind"] == "OperatorStop")
    else {
        return false;
    };
    let Some(baseline) = capture.start_baseline.as_ref() else {
        return false;
    };
    stop["accepted"] == true
        && start_preflight(Case::M5Stop, baseline).is_none()
        && stop_fight_raise(capture).is_some()
        && prayer_varp(&stop["snapshot"], 83) == Some(1)
        && prayer_varp(&stop["snapshot"], 97) == Some(1)
        && capture.actions.iter().any(|action| {
            action["kind"] == "guard"
                && action["op"] == "if-button"
                && action["component_id"] == 5623
                && prayer_varp(&action["snapshot"], 83) == Some(1)
                && prayer_varp(&action["snapshot"], 97) == Some(1)
        })
        && capture.frames.last().is_some_and(|frame| {
            frame["ingame"] == true
                && frame["scene_state"] == 2
                && prayer_varp(frame, 83) == Some(1)
                && prayer_varp(frame, 97) == Some(0)
        })
        && !capture_has_death(capture)
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
    // Thick Skin is user-owned and survives the interrupt; only Combat-raised
    // Protect from Melee is cleaned up. Later Path operations do not unprove
    // this random-event prefix.
    let Some(baseline) = capture.start_baseline.as_ref() else {
        return false;
    };
    if start_preflight(Case::M5, baseline).is_some()
        || m5_selected_prayers(capture).is_none()
        || !capture.maze_injected
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
    // Every measured prayer change must come from a native interaction.
    prefix.actions.iter().all(|action| {
        action["kind"] != json!("other-request")
            && (action["request"]["op"] != json!("if-button")
                || action["kind"] == json!("interaction"))
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

fn m5_selected_prayers(capture: &CombatCapture) -> Option<(i64, i64, i64, i64)> {
    let skin_varp = prayer_varp_for_name(capture, "Thick Skin")?;
    let protect_varp = prayer_varp_for_name(capture, "Protect from Melee")?;
    let skin_component = prayer_component(capture, "Thick Skin")?;
    let protect_component = prayer_component(capture, "Protect from Melee")?;
    (skin_varp == 83
        && protect_varp == 97
        && skin_component > 0
        && protect_component > 0
        && skin_component != protect_component)
        .then_some((skin_varp, protect_varp, skin_component, protect_component))
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
    if let Some(selected) = ranged_selected_event(capture) {
        let Some(tick) = action["snapshot_tick"].as_i64() else {
            return false;
        };
        let stale_after = if selected["case"] == json!("R3") {
            3
        } else {
            4
        };
        let last_launch = ranged_launches(capture)
            .iter()
            .filter_map(|launch| launch["snapshot_tick"].as_i64())
            .filter(|launch| *launch < tick)
            .max();
        let last_attack = capture
            .actions
            .iter()
            .filter(|candidate| is_npc_attack(candidate))
            .filter_map(|candidate| candidate["snapshot_tick"].as_i64())
            .filter(|previous| *previous < tick)
            .max();
        return last_launch
            .or(last_attack)
            .is_some_and(|previous| tick - previous >= stale_after);
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
    let Some(injection_sequence) = injection["action_sequence_at_injection"].as_u64() else {
        return false;
    };
    if injection["active_combat_owner_live_before"] != json!(true)
        || injection["active_combat_owner_live_after"] != json!(false)
        || injection["active_combat_owner_liveness_basis"]
            != json!("native action owner revocation bit")
        || injection["delivery"] != json!("Play.observe -> PlaySlotScript.on_random")
        || !owner["run"].as_str().is_some_and(|run| !run.is_empty())
        || !owner["action_id"].as_u64().is_some_and(|id| id > 0)
        || !owner["request_id"].as_u64().is_some_and(|id| id > 0)
    {
        return false;
    }
    let Some((skin_varp, _, _, _)) = m5_selected_prayers(capture) else {
        return false;
    };
    let Some(attack) = capture.actions.iter().find(|action| {
        is_imp_attack(action)
            && action_wire_valid(action)
            && action["run"] == owner["run"]
            && action["action_id"] == owner["action_id"]
            && action["request_id"] == owner["request_id"]
            && action["sequence"]
                .as_u64()
                .is_some_and(|sequence| sequence <= injection_sequence)
    }) else {
        return false;
    };
    let Some(raise) = m5_protect_raise_action(capture, injection) else {
        return false;
    };
    attack["sequence"] != raise["sequence"]
        && prayer_varp(&attack["snapshot"], skin_varp) == Some(1)
}

fn is_imp_attack(action: &Value) -> bool {
    let Some(index) = action["request"]["index"].as_i64() else {
        return false;
    };
    is_npc_attack(action)
        && action["request"]["name"]
            .as_str()
            .is_some_and(|name| name.eq_ignore_ascii_case("imp"))
        && action["snapshot"]["nearby_npcs"]
            .as_array()
            .is_some_and(|npcs| {
                npcs.iter().any(|npc| {
                    npc["index"].as_i64() == Some(index)
                        && npc["name"]
                            .as_str()
                            .is_some_and(|name| name.eq_ignore_ascii_case("imp"))
                })
            })
}

fn m5_protect_raise_action<'a>(capture: &'a CombatCapture, injection: &Value) -> Option<&'a Value> {
    let owner = &injection["active_combat_owner"];
    let injection_sequence = injection["action_sequence_at_injection"].as_u64()?;
    let (skin_varp, protect_varp, _, protect_component) = m5_selected_prayers(capture)?;
    capture.actions.iter().find(|action| {
        prayer_action(action, protect_component)
            && action_wire_valid(action)
            && action["run"] == owner["run"]
            && action["action_id"] == owner["action_id"]
            && action["request_id"].as_u64().is_some_and(|id| id > 0)
            && action["sequence"]
                .as_u64()
                .is_some_and(|sequence| sequence <= injection_sequence)
            && prayer_varp(&action["snapshot"], skin_varp) == Some(1)
            && prayer_varp(&action["snapshot"], protect_varp) == Some(0)
    })
}

fn clear_prayers_before_next_operation(capture: &CombatCapture, injection: &Value) -> bool {
    if injection["hold"] != json!(true) {
        return false;
    }
    let Some(baseline) = capture.start_baseline.as_ref() else {
        return false;
    };
    let Some((skin_varp, protect_varp, _, protect_component)) = m5_selected_prayers(capture) else {
        return false;
    };
    if prayer_varp(baseline, skin_varp) != Some(1) || prayer_varp(baseline, protect_varp) != Some(0)
    {
        return false;
    }
    if !injection["prayer_varps_at_injection"].is_array()
        || prayer_varp_rows(&injection["prayer_varps_at_injection"], skin_varp) != Some(1)
        || prayer_varp_rows(&injection["prayer_varps_at_injection"], protect_varp) != Some(1)
        || m5_protect_raise_action(capture, injection).is_none()
    {
        return false;
    }
    let Some(injection_sequence) = injection["action_sequence_at_injection"].as_u64() else {
        return false;
    };
    let after = capture
        .actions
        .iter()
        .filter(|action| {
            action["sequence"]
                .as_u64()
                .is_some_and(|action_sequence| action_sequence > injection_sequence)
        })
        .collect::<Vec<_>>();
    let Some(next_operation_index) = after.iter().position(|action| {
        action["kind"] != json!("interaction") || action["request"]["op"] != json!("if-button")
    }) else {
        return false;
    };
    let cleanup = &after[..next_operation_index];
    let next_operation = after[next_operation_index];
    let Some(cleanup_action) = cleanup.first() else {
        return false;
    };
    let combat_owner = &injection["active_combat_owner"];
    cleanup.len() == 1
        && prayer_action(cleanup_action, protect_component)
        && action_wire_valid(cleanup_action)
        && cleanup_action["run"].as_str().is_some()
        && cleanup_action["action_id"]
            .as_u64()
            .is_some_and(|id| id > 0)
        && cleanup_action["request_id"]
            .as_u64()
            .is_some_and(|id| id > 0)
        && (cleanup_action["run"] != combat_owner["run"]
            || cleanup_action["action_id"] != combat_owner["action_id"])
        && prayer_varp(&cleanup_action["snapshot"], skin_varp) == Some(1)
        && prayer_varp(&cleanup_action["snapshot"], protect_varp) == Some(1)
        && prayer_varp(&next_operation["snapshot"], skin_varp) == Some(1)
        && prayer_varp(&next_operation["snapshot"], protect_varp) == Some(0)
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
    prayer_varp_rows(&frame["prayer_varps"], id)
}

fn prayer_varp_rows(prayer_varps: &Value, id: i64) -> Option<i64> {
    prayer_varps
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
    let evidence = match evidence_dir() {
        Ok(dir) => dir,
        Err(error) => panic!("{} live prerequisites: {error}", case.key()),
    };
    std::fs::create_dir_all(&evidence).expect("create assigned combat evidence directory");
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
    let world = template.world();
    let world_ref = world.as_deref().expect("selected navigation world");
    let placement = case.is_ranged().then(|| {
        ranged_placement(case, world_ref).expect("choose collision-backed ranged placement")
    });
    let stand = placement
        .as_ref()
        .map(|placement| placement.spawn)
        .or_else(|| case.stand(world_ref).ok())
        .expect("choose collision-backed combat fixture stand");
    let path_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../script/paths/289");
    let source_path = path_root.join(case.path_relative());
    let source = std::fs::read(&source_path)
        .unwrap_or_else(|error| panic!("read {}: {error}", source_path.display()));
    let compile_source = if let Some(placement) = placement.as_ref() {
        let mut fixture: Value =
            serde_json::from_slice(&source).expect("parse ranged combat fixture");
        let step = &mut fixture["roles"][0]["sequences"][0]["steps"][0];
        step["args"]["tactic"]["style"] = json!("ranged");
        step["args"]["stand"]["tile"] =
            json!([placement.stand.x, placement.stand.z, placement.stand.level]);
        step["args"]["area"]["box"] = json!([
            placement.spawn.x - 12,
            placement.spawn.z - 12,
            placement.spawn.x + 12,
            placement.spawn.z + 12,
            placement.spawn.level
        ]);
        serde_json::to_vec(&fixture).expect("serialize runtime ranged combat fixture")
    } else {
        source
    };
    let path = compile_path(&compile_source, &selected, &quests)
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
        if case.is_ranged() {
            proof
                .random_events
                .push(ranged_selected_facts_event(case, &selected));
        }
    }
    let _registration = CaptureRegistration::install(&account, Arc::clone(&capture));
    let mut writer = EvidenceWriter {
        case,
        account: account.clone(),
        path: evidence,
        capture: Arc::clone(&capture),
        frame: FrameBuf::new(),
        scenario_status: "Preparing".to_owned(),
        outcome: "RUNNING".to_owned(),
        error: None,
        flushed: false,
    };

    let scenario = scenario_for(case, stand, placement, Arc::clone(&capture));
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
        ranged_probe_signature: None,
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
        if matches!(case, Case::R1 | Case::R3)
            && report_with_end(&capture_snapshot, "Killed")
                .and_then(|report| integer(report, "combat_evidence_tick"))
                .is_some_and(|done_tick| {
                    capture_snapshot
                        .frames
                        .last()
                        .and_then(|frame| frame["tick"].as_i64())
                        .is_some_and(|tick| tick >= done_tick + 3)
                })
            && !ranged_ready(case, &capture_snapshot)
        {
            terminal_error = Some("completed ranged fight failed receipt predicates".to_owned());
            writer.outcome = "FAIL".to_owned();
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

    if case == Case::M5Stop {
        assert_eq!(
            writer.outcome, "PASS",
            "Stop live proof: {:?}",
            writer.error
        );
    }
    match writer.outcome.as_str() {
        "PASS" => {}
        "INVALID" | "NOT_STAGED" if !case.is_ranged() => {}
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
fn live_combat_m5_random_interrupt_clears_only_combat_raised_protect() {
    run_case(Case::M5);
}

#[test]
#[ignore = "requires LIVE=1 and the shared local R289 engine"]
fn live_lifecycle_followups_stop_clears_only_combat_raised_protect() {
    run_case(Case::M5Stop);
}

#[test]
#[ignore = "requires LIVE=1 and the isolated local R289 engine with cooked karambwan"]
fn live_combat_m6_natural_warlord_combo_eat() {
    run_case(Case::M6);
}

#[test]
#[ignore = "requires LIVE=1, BOT_ENGINE_DIR, the selected local R289 nav pack, and LIVE_EVIDENCE_DIR"]
fn live_combat_r1_ranged_rapid() {
    run_case(Case::R1);
}

#[test]
#[ignore = "requires LIVE=1, BOT_ENGINE_DIR, the selected local R289 nav pack, and LIVE_EVIDENCE_DIR"]
fn live_combat_r2_ranged_wrong_ammo() {
    run_case(Case::R2);
}

#[test]
#[ignore = "requires LIVE=1, BOT_ENGINE_DIR, the selected local R289 nav pack, and LIVE_EVIDENCE_DIR"]
fn live_combat_r3_ranged_thrown() {
    run_case(Case::R3);
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

fn m5_oracle_capture() -> CombatCapture {
    use client::io::ClientProt289;

    let prayer_varps = |skin_on, protect_on| {
        (83..=97)
            .map(|index| {
                json!({
                    "index": index,
                    "value": i32::from((index == 83 && skin_on) || (index == 97 && protect_on))
                })
            })
            .collect::<Vec<_>>()
    };
    let snapshot = |skin_on, protect_on| {
        json!({
            "prayer_varps": prayer_varps(skin_on, protect_on),
            "nearby_npcs": [{"index": 42, "name": "Imp", "type": 708}]
        })
    };
    let attack = json!({
        "kind": "interaction", "sequence": 1, "tick": 10, "host_tick": 10,
        "snapshot_tick": 10, "accepted": true, "wire_decoded": true,
        "wire_opcodes": [ClientProt289::MOVE_OPCLICK.id, ClientProt289::OPNPC2.id],
        "run": "run-1", "action_id": 7, "request_id": 9, "batch": 9,
        "request": {"op": "npc", "name": "Imp", "action": "Attack", "index": 42},
        "snapshot": snapshot(true, false)
    });
    let protect_raise = json!({
        "kind": "interaction", "sequence": 2, "tick": 11, "host_tick": 11,
        "snapshot_tick": 11, "accepted": true, "wire_decoded": true,
        "wire_opcodes": [ClientProt289::IF_BUTTON.id],
        "run": "run-1", "action_id": 7, "request_id": 10, "batch": 10,
        "request": {"op": "if-button", "component_id": 5623},
        "snapshot": snapshot(true, false)
    });
    let cleanup = json!({
        "kind": "interaction", "sequence": 3, "tick": 12, "host_tick": 12,
        "snapshot_tick": 12, "accepted": true, "wire_decoded": true,
        "wire_opcodes": [ClientProt289::IF_BUTTON.id],
        "run": "run-1", "action_id": 12, "request_id": 11, "batch": 11,
        "request": {"op": "if-button", "component_id": 5623},
        "snapshot": snapshot(true, true)
    });
    let next = json!({
        "kind": "walk", "sequence": 4, "tick": 14,
        "request": {"target": {"x": 2632, "z": 3222, "level": 0}, "radius": 2},
        "snapshot": snapshot(true, false)
    });
    let later_attack = json!({
        "kind": "interaction", "sequence": 5, "tick": 40, "accepted": true,
        "wire_decoded": true,
        "wire_opcodes": [ClientProt289::MOVE_OPCLICK.id, ClientProt289::OPNPC2.id],
        "request": {"op": "npc", "name": "Imp", "action": "Attack"}
    });
    let bank = json!({
        "kind": "interaction", "sequence": 6, "tick": 50, "accepted": true,
        "wire_decoded": true, "wire_opcodes": [195, 67, 45],
        "request": {"debug": "OpenStand"}
    });
    let injection = json!({
        "kind": "Maze", "hold": true, "action_sequence_at_injection": 2,
        "prayer_varps_at_injection": prayer_varps(true, true),
        "active_combat_owner": {"run": "run-1", "action_id": 7, "request_id": 9},
        "active_combat_owner_live_before": true,
        "active_combat_owner_live_after": false,
        "active_combat_owner_liveness_basis": "native action owner revocation bit",
        "delivery": "Play.observe -> PlaySlotScript.on_random"
    });

    let mut capture = CombatCapture::default();
    capture.start_baseline = Some(json!({
        "ingame": true,
        "scene_state": 2,
        "stats": [{"name": "prayer", "base": 43, "effective": 43}],
        "prayer_varps": prayer_varps(true, false)
    }));
    capture.maze_injected = true;
    capture.maze_owner_live_before = Some(true);
    capture.maze_owner_live_after = Some(false);
    capture.prayer_facts = vec![
        json!({"name": "Thick Skin", "varp": 83, "button_com": 5609}),
        json!({"name": "Protect from Melee", "varp": 97, "button_com": 5623}),
    ];
    capture.random_events.push(injection);
    capture.actions = vec![attack, protect_raise, cleanup, next, later_attack, bank];
    capture.observations = vec![
        json!({"tick": 10, "exclusive": true}),
        json!({"tick": 11, "exclusive": true}),
        json!({"tick": 12, "exclusive": true}),
    ];
    capture
}

#[test]
fn hygiene_cleanup_requires_user_skin_and_only_combat_raised_protect_off() {
    let mut capture = m5_oracle_capture();
    let injection = capture.random_events[0].clone();
    assert!(clear_prayers_before_next_operation(&capture, &injection));

    let cleanup = capture.actions[2].clone();
    capture.actions.remove(2);
    assert!(!clear_prayers_before_next_operation(&capture, &injection));
    capture.actions.insert(2, cleanup.clone());

    capture.actions[2]["accepted"] = json!(false);
    assert!(!clear_prayers_before_next_operation(&capture, &injection));
    capture.actions[2] = cleanup.clone();
    capture.actions[2]["request"]["component_id"] = json!(5609);
    assert!(!clear_prayers_before_next_operation(&capture, &injection));
    capture.actions[2] = cleanup.clone();
    capture.actions[2]["action_id"] = json!(7);
    assert!(!clear_prayers_before_next_operation(&capture, &injection));
    capture.actions[2] = cleanup.clone();
    capture.actions[2]["kind"] = json!("other-request");
    assert!(!clear_prayers_before_next_operation(&capture, &injection));
    capture.actions[2] = cleanup.clone();

    capture.start_baseline.as_mut().unwrap()["prayer_varps"][0]["value"] = json!(0);
    assert!(!clear_prayers_before_next_operation(&capture, &injection));
    capture.start_baseline.as_mut().unwrap()["prayer_varps"][0]["value"] = json!(1);

    let mut skin_off_at_injection = injection.clone();
    skin_off_at_injection["prayer_varps_at_injection"][0]["value"] = json!(0);
    assert!(!clear_prayers_before_next_operation(
        &capture,
        &skin_off_at_injection
    ));

    capture.actions[3]["snapshot"]["prayer_varps"][14]["value"] = json!(1);
    assert!(
        !clear_prayers_before_next_operation(&capture, &injection),
        "Protect from Melee must be off before the next non-prayer operation"
    );
    capture.actions[3]["snapshot"]["prayer_varps"][14]["value"] = json!(0);
    capture.actions[3]["snapshot"]["prayer_varps"][0]["value"] = json!(0);
    assert!(
        !clear_prayers_before_next_operation(&capture, &injection),
        "user Thick Skin must survive through the next operation"
    );
}

#[test]
fn hygiene_owner_oracle_requires_a_valid_protect_raise_from_the_imp_combat_owner() {
    let mut capture = m5_oracle_capture();
    let mut injection = capture.random_events[0].clone();
    assert!(m5_owner_preempted(&capture, &injection));

    let protect_raise = capture.actions[1].clone();
    capture.actions[1]["kind"] = json!("other-request");
    capture.actions[1]["request"] = json!("if-button Protect from Melee");
    assert!(!m5_owner_preempted(&capture, &injection));
    capture.actions[1] = protect_raise.clone();
    capture.actions[1]["accepted"] = json!(false);
    assert!(!m5_owner_preempted(&capture, &injection));
    capture.actions[1] = protect_raise.clone();
    capture.actions[1]["wire_decoded"] = json!(false);
    assert!(!m5_owner_preempted(&capture, &injection));
    capture.actions[1] = protect_raise.clone();
    capture.actions[1]["wire_opcodes"] = json!([0]);
    assert!(!m5_owner_preempted(&capture, &injection));
    capture.actions[1] = protect_raise.clone();
    capture.actions[1]["action_id"] = json!(8);
    assert!(!m5_owner_preempted(&capture, &injection));
    capture.actions[1] = protect_raise;

    injection["active_combat_owner_live_before"] = json!(false);
    assert!(!m5_owner_preempted(&capture, &injection));
    injection["active_combat_owner_live_before"] = json!(true);
    injection["active_combat_owner_live_after"] = json!(true);
    assert!(!m5_owner_preempted(&capture, &injection));
    injection["active_combat_owner_live_after"] = json!(false);
    injection["action_sequence_at_injection"] = json!(1);
    assert!(!m5_owner_preempted(&capture, &injection));
    injection["action_sequence_at_injection"] = json!(2);
    injection["active_combat_owner_liveness_basis"] = json!("request reservation");
    assert!(!m5_owner_preempted(&capture, &injection));
}

#[test]
fn m5_ready_accepts_raised_prayer_hygiene_while_the_path_continues() {
    let mut capture = m5_oracle_capture();
    assert!(
        m5_ready(&capture),
        "later Imp attacks and bank operations must not unprove raised-only hygiene"
    );

    let duplicate = capture.actions[0].clone();
    capture.actions.insert(1, duplicate);
    assert!(
        !m5_ready(&capture),
        "duplicate Attack breaks the admitted prefix"
    );
    capture.actions.remove(1);

    let protect_raise = capture.actions[1].clone();
    capture.actions[1]["kind"] = json!("other-request");
    capture.actions[1]["origin"] = json!("staging");
    capture.actions[1]["request"] = json!("if-button Protect from Melee");
    assert!(
        !m5_ready(&capture),
        "manual harness prayer staging cannot stand in for Combat's raise"
    );
    capture.actions[1] = protect_raise.clone();
    capture.actions[1]["accepted"] = json!(false);
    assert!(
        !m5_ready(&capture),
        "an unaccepted Combat raise is not owned"
    );
    capture.actions[1] = protect_raise.clone();
    capture.actions[1]["action_id"] = json!(8);
    assert!(
        !m5_ready(&capture),
        "an unrelated owner cannot raise the prayer"
    );
    capture.actions[1] = protect_raise;

    let cleanup = capture.actions[2].clone();
    capture.actions.remove(2);
    assert!(!m5_ready(&capture), "missing cleanup cannot pass");
    capture.actions.insert(2, cleanup);

    capture.random_events[0]["prayer_varps_at_injection"][14]["value"] = json!(0);
    assert!(
        !m5_ready(&capture),
        "Protect from Melee must be observed on at injection"
    );
    capture.random_events[0]["prayer_varps_at_injection"][14]["value"] = json!(1);
    capture.start_baseline.as_mut().unwrap()["prayer_varps"][0]["value"] = json!(0);
    assert!(!m5_ready(&capture), "off user Thick Skin cannot pass");
    capture.start_baseline.as_mut().unwrap()["prayer_varps"][0]["value"] = json!(1);

    capture.random_events[0]["prayer_varps_at_injection"][0]["value"] = json!(0);
    assert!(
        !m5_ready(&capture),
        "off user Thick Skin at injection cannot pass"
    );
    capture.random_events[0]["prayer_varps_at_injection"][0]["value"] = json!(1);

    capture.actions[3]["snapshot"]["prayer_varps"][14]["value"] = json!(1);
    assert!(
        !m5_ready(&capture),
        "still-on Protect from Melee cannot pass"
    );
}

#[test]
fn m5_start_requires_the_user_prayer_and_protect_off_baseline() {
    let mut capture = m5_oracle_capture();
    assert_eq!(
        start_preflight(Case::M5, capture.start_baseline.as_ref().unwrap()),
        None
    );
    capture.start_baseline.as_mut().unwrap()["prayer_varps"][0]["value"] = json!(0);
    assert!(start_preflight(Case::M5, capture.start_baseline.as_ref().unwrap()).is_some());
    capture.start_baseline.as_mut().unwrap()["prayer_varps"][0]["value"] = json!(1);
    capture.start_baseline.as_mut().unwrap()["prayer_varps"][14]["value"] = json!(1);
    assert!(start_preflight(Case::M5, capture.start_baseline.as_ref().unwrap()).is_some());
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
#[test]
fn ranged_darts_decrease_once_per_observed_projectile() {
    let mut capture = CombatCapture::default();
    capture.frames.extend([
        json!({"snapshot_tick": 10, "inventory": [{"id": 806, "count": 200}], "equipment": []}),
        json!({"snapshot_tick": 11, "inventory": [{"id": 806, "count": 199}], "equipment": []}),
        json!({"snapshot_tick": 12, "inventory": [{"id": 806, "count": 199}], "equipment": []}),
        json!({"snapshot_tick": 13, "inventory": [{"id": 806, "count": 198}], "equipment": []}),
    ]);
    let launches = vec![
        json!({"launch_tick": 11, "snapshot_tick": 11}),
        json!({"launch_tick": 13, "snapshot_tick": 13}),
    ];
    let steps = ranged_dart_count_steps(&capture, 806, &launches);
    assert_eq!(steps.len(), launches.len());
    assert!(steps.iter().all(|step| step["valid"] == json!(true)));

    capture.frames[3]["inventory"][0]["count"] = json!(199);
    let steps = ranged_dart_count_steps(&capture, 806, &launches);
    assert_eq!(steps[1]["decrement"], json!(0));
    assert_eq!(steps[1]["valid"], json!(false));
}

#[test]
fn ranged_ammo_sweep_counts_targeted_ground_stack_units() {
    let action = json!({
        "request": {"x": 2638, "z": 3224, "level": 0},
        "snapshot": {"ground_items": [
            {"id": 882, "count": 3, "tile": {"x": 2638, "z": 3224, "level": 0}},
            {"id": 883, "count": 7, "tile": {"x": 2638, "z": 3224, "level": 0}},
            {"id": 882, "count": 5, "tile": {"x": 2639, "z": 3224, "level": 0}},
        ]},
    });
    let after = json!({"ground_items": [
        {"id": 883, "count": 7, "tile": {"x": 2638, "z": 3224, "level": 0}},
        {"id": 882, "count": 5, "tile": {"x": 2639, "z": 3224, "level": 0}}
    ]});
    assert_eq!(ranged_pickup_count(&action, &after, 882), Some(3));
    assert_eq!(ranged_pickup_count(&action, &after, 883), None);
    assert_eq!(ranged_pickup_count(&action, &after, 884), None);
}

#[test]
fn ranged_ammo_sweep_does_not_double_count_same_tile_stacks() {
    let stack = |count| {
        json!({
            "id": 886, "count": count, "tile": {"x": 2600, "z": 3372, "level": 0}
        })
    };
    let mut action = json!({
        "request": {"x": 2600, "z": 3372, "level": 0},
        "snapshot": {"ground_items": [stack(24), stack(22)]}
    });
    let after = json!({"ground_items": [stack(22)]});
    assert_eq!(ranged_pickup_count(&action, &after, 886), Some(24));
    assert_eq!(ranged_pickup_count(&action, &action["snapshot"], 886), None);
    assert_eq!(
        ranged_pickup_count(&action, &json!({"ground_items": []}), 886),
        None
    );
    action["snapshot"] = after;
    assert_eq!(
        ranged_pickup_count(&action, &json!({"ground_items": []}), 886),
        Some(22)
    );
}

#[test]
fn ranged_sweep_requires_pickups_and_uses_the_observed_corpse_baseline() {
    let mut capture = CombatCapture::default();
    capture.start_baseline = Some(json!({"tick": 10, "inventory": [{"id": 882, "count": 20}]}));
    capture.random_events.push(json!({
        "kind": "RangedSelectedFacts", "ammo_obj_id": 882
    }));
    let corpse = json!({
        "tick": 12,
        "inventory": [{"id": 882, "count": 19}],
        "nearby_npcs": [{"index": 7, "type": 81, "health": 0, "total_health": 10}],
        "ground_items": [{"id": 882, "count": 1}]
    });
    capture.frames.extend([
        corpse.clone(),
        json!({
            "tick": 15, "inventory": [{"id": 882, "count": 19}], "nearby_npcs": []
        }),
    ]);
    let report = json!({"fields": {
        "combat_engaged_index": 7, "combat_engaged_npc_type": 81, "combat_evidence_tick": 15
    }});
    assert_eq!(ranged_corpse_frame(&capture, &report), Some(&corpse));
    for case in [Case::R1, Case::R3] {
        let receipt = ranged_ammo_receipt(case, &capture, &report);
        assert_eq!(receipt["corpse_tick"], json!(12));
        assert_eq!(receipt["held_count_at_kill"], json!(19));
        assert_eq!(receipt["pickup_count"], json!(0));
        assert_eq!(receipt["sweep_observed"], json!(false));
        assert_eq!(receipt["valid"], json!(false));
    }
    capture.frames[0]["nearby_npcs"][0]["type"] = json!(82);
    assert!(ranged_corpse_frame(&capture, &report).is_none());
}

#[test]
fn ranged_wrong_ammo_requires_a_blocked_abort_and_no_projectile() {
    let mut capture = CombatCapture::default();
    capture.statuses.push(json!({
        "phase": "Blocked",
        "failure": "ScriptFailure { code: \"parked\", message: \"combat aborted; caller must handle the failure\" }",
    }));
    capture.statuses.push(json!({
        "fields": {"combat_end": "Aborted(PrepFailed(Ammo))"},
    }));
    capture.random_events.push(json!({
        "kind": "RangedProbe",
        "projectiles": [],
    }));
    assert!(ranged_wrong_ammo_ready(&capture));
    capture.statuses[0]["failure"] =
        json!("\"extra: combat aborted; caller must handle the failure\"");
    assert!(!ranged_wrong_ammo_ready(&capture));
    capture.statuses[0]["failure"] = json!("ScriptFailure { code: \"parked\", message: \"combat aborted; caller must handle the failure\" }");
    assert!(ranged_wrong_ammo_ready(&capture));

    capture.actions.push(json!({
        "kind": "interaction",
        "request": {"op": "npc", "action": "Attack"},
    }));
    assert!(!ranged_wrong_ammo_ready(&capture));
    capture.actions.clear();
    capture.random_events[0]["projectiles"] = json!([{"local_launch": false}]);
    assert!(!ranged_wrong_ammo_ready(&capture));
}

#[test]
fn ranged_scenario_arrives_before_spawn_and_leaves_the_firing_stand_last() {
    let spawn = WorldTile {
        x: 2457,
        z: 3302,
        level: 0,
    };
    let stand = WorldTile {
        x: 2452,
        z: 3302,
        level: 0,
    };
    let scenario = scenario_for(
        Case::R2,
        spawn,
        Some(RangedPlacement { spawn, stand }),
        Arc::new(Mutex::new(CombatCapture::default())),
    );
    let position = |name| {
        scenario
            .steps
            .iter()
            .position(|step| step.name == name)
            .unwrap()
    };
    assert!(
        position("stand at the quest start")
            < position("spawn a local Khazard Warlord before Start")
    );
    assert!(
        position("spawn a local Khazard Warlord before Start")
            < position("teleport exactly five tiles from the new Warlord")
    );
    assert_eq!(ranged_aliases(Case::R2).1, "bolt");
}

#[test]
fn ranged_preflight_reads_the_real_local_player_tile_shape() {
    let selected = api::game_data::for_revision(ClientRevision::R289).unwrap();
    let facts = selected_ranged_facts(Case::R2, &selected).unwrap();
    let spawn = WorldTile {
        x: 2500,
        z: 3200,
        level: 0,
    };
    let stand = WorldTile { x: 2505, ..spawn };
    let mut capture = CombatCapture::default();
    capture.random_events.push(json!({
        "kind": "RangedNpcAdd", "spawn_tile": tile_value(spawn),
        "stand_tile": tile_value(stand), "spawned_index": 42, "tele_observed": true
    }));
    let mut baseline = json!({
        "ingame": true, "scene_state": 2,
        "tile": [stand.x, stand.z, stand.level],
        "local_player": {"tile": tile_value(stand)},
        "nearby_npcs": [{"index": 42, "type": WARLORD_NPC_ID,
            "tile": {"x": spawn.x + 1, "z": spawn.z, "level": 0}}],
        "stats": [
            {"name": "ranged", "base": 70, "effective": 70},
            {"name": "defence", "base": 40, "effective": 40},
            {"name": "hitpoints", "base": 40, "effective": 40},
            {"name": "prayer", "base": 43, "effective": 43}
        ],
        "inventory": [
            {"id": facts.weapon_id, "count": 1},
            {"id": facts.ammo_id, "count": 50},
            {"id": PRAYER_POTION_4_ID, "count": 1}
        ]
    });
    assert_eq!(
        ranged_start_preflight(Case::R2, &baseline, &selected, &capture),
        None
    );
    baseline["local_player"]["tile"] = Value::Null;
    assert!(ranged_start_preflight(Case::R2, &baseline, &selected, &capture).is_some());
}

#[test]
fn ranged_stale_attack_oracle_uses_snapshot_clock_and_selected_rate() {
    for (case, threshold) in [("R1", 4), ("R3", 3)] {
        let mut capture = CombatCapture::default();
        capture.random_events.push(json!({
            "kind": "RangedSelectedFacts", "case": case
        }));
        capture.actions.push(json!({
            "kind": "interaction", "request": {"op": "npc", "action": "Attack"},
            "tick": 1017, "snapshot_tick": 1000
        }));
        let mut action = json!({
            "tick": 1017 + threshold, "snapshot_tick": 1000 + threshold,
            "snapshot": {"local_player": {"target": {"kind": "Npc", "index": 42}}}
        });
        assert!(attack_is_mismatch_or_stale(&capture, &action, 42));
        action["snapshot_tick"] = json!(999 + threshold);
        assert!(!attack_is_mismatch_or_stale(&capture, &action, 42));
    }
}

#[test]
fn ranged_prep_requires_first_equipped_native_poll_not_render_frame() {
    use client::io::ClientProt289;
    let selected = api::game_data::for_revision(ClientRevision::R289).unwrap();
    let facts = selected_ranged_facts(Case::R3, &selected).unwrap();
    assert_eq!(facts.weapon_id, facts.ammo_id);
    let mut capture = CombatCapture::default();
    capture.random_events.extend([
        ranged_selected_facts_event(Case::R3, &selected),
        json!({"kind": "RangedProbe", "tick": 12, "mode_varp": 1,
            "combat_root_id": facts.tab_root_id,
            "combat_style_buttons": [{"component_id": facts.rapid.button, "mode": 1}]}),
    ]);
    capture.actions.extend([
        json!({"kind": "interaction", "sequence": 1, "snapshot_tick": 10, "tick": 30,
            "accepted": true, "wire_decoded": true, "wire_opcodes": [ClientProt289::OPHELD1.id],
            "request": {"op": "wear", "name": ranged_aliases(Case::R3).0},
            "snapshot": {"inventory": [{"actions": ["Wield"]}]}}),
        json!({"kind": "interaction", "sequence": 2, "snapshot_tick": 12, "tick": 32,
            "accepted": true, "wire_decoded": true, "wire_opcodes": [ClientProt289::IF_BUTTON.id],
            "request": {"op": "if-button", "component_id": facts.rapid.button}}),
        json!({"kind": "interaction", "sequence": 3, "snapshot_tick": 13, "tick": 33,
            "request": {"op": "npc", "action": "Attack"}}),
    ]);
    capture.observations.extend([
        json!({"native_tick_edge": true, "snapshot_tick": 11, "host_tick": 31,
            "equipment": [], "combat_root_id": facts.tab_root_id}),
        json!({"native_tick_edge": false, "snapshot_tick": 11, "host_tick": 31,
            "equipment": [facts.weapon_id], "combat_root_id": facts.tab_root_id}),
        json!({"native_tick_edge": true, "snapshot_tick": 12, "host_tick": 32,
            "equipment": [facts.weapon_id], "combat_root_id": facts.tab_root_id}),
    ]);
    assert!(ranged_style_prep_ok(Case::R3, &capture));
    assert_eq!(ranged_style_timing(&capture)["wear_to_style_ticks"], 2);
    capture.actions[1]["snapshot_tick"] = json!(11);
    capture.actions[1]["tick"] = json!(31);
    assert!(!ranged_style_prep_ok(Case::R3, &capture));
    capture.actions[1]["snapshot_tick"] = json!(13);
    capture.actions[1]["tick"] = json!(33);
    assert!(!ranged_style_prep_ok(Case::R3, &capture));
    capture.actions[1]["snapshot_tick"] = json!(12);
    capture.actions[1]["tick"] = json!(32);
    capture.observations[2]["combat_root_id"] = json!(-1);
    assert!(!ranged_style_prep_ok(Case::R3, &capture));
}
