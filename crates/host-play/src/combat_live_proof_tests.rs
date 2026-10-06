//! Live M1-M6 melee and G1-G2 magic proofs through the ordinary host-play observer.
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
use api::interact::{ActionSpec, Interactions, OpTarget, SendResult};
use api::quest_facts::QuestCatalog;
use api::selected::{ClientRevision, RunKey};
use api::snapshot::{ActorKind, GameSnapshot, WorldTile};
use host::{FrameBuf, Pump};
use scenario::{
    Proof, RunnerStatus, Scenario, ScenarioRunner, ScriptInjectValue, ScriptSettingInject, Step,
    StepKind, Wait,
};
use script::combat::threats::targets_local_with_attack_evidence;
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
const WARLORD_WANDER_RANGE: i32 = 9;
const RANGED_STAGE_DISTANCE: i32 = 9;
const R2_STAGE_DISTANCE: i32 = 1;
const R2_START_DISTANCE_MAX: i32 = R2_STAGE_DISTANCE + 1;
const RANGED_START_RESAMPLE_TICKS: u32 = 3;
const RANGED_START_DISTANCE_MIN: i32 = RANGED_STAGE_DISTANCE - 1;
const RANGED_START_DISTANCE_MAX: i32 = RANGED_STAGE_DISTANCE + 1;
const WARLORD_RESPAWN_TICKS: u32 = 300;
const WARLORD_RESPAWN_MARGIN_TICKS: u32 = 50;
const RANGED_NATURAL_WARLORD_WAIT_TICKS: u32 = WARLORD_RESPAWN_TICKS + WARLORD_RESPAWN_MARGIN_TICKS;
const RANGED_ABORT_SURVIVAL_TICKS: i64 = 12;
// Selected R289 data: Protect from Melee requires base Prayer 43.
const RANGED_ABORT_MIN_PROTECT_LEVEL: i64 = 43;
const RANGED_ABORTED_COMBAT_STEP_REASON: &str = "combat aborted; caller must handle the failure";
const RANGED_ABORT_WALK_FAILED_REASON: &str = "combat abort walk did not reach a safe tile";
const RANGED_ABORT_HOLD_SAFE_REASON: &str =
    "combat abort guarded hold ended because the attacker disengaged or HP cleared the guard food line";
const RANGED_ABORT_HOLD_BOUND_REASON: &str = "combat abort guarded hold reached its 12-tick bound";
const RANGED_ABORT_HOLD_UNAVAILABLE_REASON: &str =
    "combat abort safety hold could not start its food guard";

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
const STAFF_OF_FIRE_ID: i32 = 1387;
const CHAOS_RUNE_ID: i32 = 562;
const AIR_RUNE_ID: i32 = 556;
const MIND_RUNE_ID: i32 = 558;
const MAGIC_AIR_RUNES: i32 = 150;
const MAGIC_CHAOS_RUNES: i32 = 12;
const MAGIC_MIND_RUNES: i32 = 90;
const MAGIC_PRAYER_RESTORES: i32 = 2;
const FIRE_BOLT_WIDGET: i64 = 1169;
const FIRE_STRIKE_WIDGET: i64 = 1158;
const AUTO_CHOOSER_COMPONENT: i64 = 353;
const AUTO_TOGGLE_COMPONENT: i64 = 349;
const AUTO_FIRE_BOLT_COMPONENT: i64 = 1837;
const AUTO_FIRE_STRIKE_COMPONENT: i64 = 1833;
const COMBAT_TAB_ROOT: i64 = 328;
const SPELL_PANEL_ROOT: i64 = 1829;
const FAILED_SPELL_SPLASH: i64 = 85;

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
    ("lobster", 6),
];
const R2_ITEMS: &[(&str, i32)] = &[
    ("maple_shortbow", 1),
    ("bolt", 50),
    ("4doseprayerrestore", 1),
    ("lobster", 6),
];
const R3_ITEMS: &[(&str, i32)] = &[
    ("bronze_dart", 200),
    ("4doseprayerrestore", 1),
    ("lobster", 6),
];
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
    MageAuto,
    MageManualFallback,
    MageManualNoFallback,
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
            Self::MageAuto => "cm-G1",
            Self::MageManualFallback => "cm-G2-fallback",
            Self::MageManualNoFallback => "cm-G2-no-fallback",
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
            Self::MageAuto => "combat_s3c_g1_magic_autocast",
            Self::MageManualFallback => "combat_s3c_g2_magic_manual_fallback",
            Self::MageManualNoFallback => "combat_s3c_g2_magic_manual_no_fallback",
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
            Self::MageAuto => "fixtures/combat_magic_autocast.json",
            Self::MageManualFallback => "fixtures/combat_magic_manual_fallback.json",
            Self::MageManualNoFallback => "fixtures/combat_magic_manual_no_fallback.json",
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
            Self::MageAuto | Self::MageManualFallback | Self::MageManualNoFallback => &[],
        }
    }

    fn timeout(self) -> Duration {
        match self {
            Self::M1 => Duration::from_secs(m1_coverage_budget().2),
            Self::M1HandIn | Self::M5 | Self::M5Stop => Duration::from_secs(1_200),
            Self::M2 | Self::M3 | Self::M6 | Self::R1 | Self::R3 => Duration::from_secs(900),
            Self::M4 | Self::R2 => Duration::from_secs(300),
            Self::MageAuto | Self::MageManualFallback | Self::MageManualNoFallback => {
                Duration::from_secs(900)
            }
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
            Self::MageAuto | Self::MageManualFallback | Self::MageManualNoFallback => {
                let (east, north) = match self {
                    Self::MageAuto => (0, 0),
                    Self::MageManualFallback => (32, 0),
                    Self::MageManualNoFallback => (0, 32),
                    _ => unreachable!(),
                };
                // Stage away from the natural Warlord; each fixed cell also
                // stays outside the preceding staged actor's tether.
                magic_spawn_stand(
                    world,
                    WorldTile {
                        x: IMP_START.x + east,
                        z: IMP_START.z + north,
                        ..IMP_START
                    },
                )
            }
            Self::M4 => world
                .collision
                .standable(TREE_APPROACH)
                .then_some(TREE_APPROACH)
                .ok_or_else(|| "safe Draynor Manor tree approach is not standable".to_owned()),
            Self::R1 | Self::R2 | Self::R3 => ranged_placement(self, world).map(|p| p.spawn),
        }
    }
    fn is_magic(self) -> bool {
        matches!(
            self,
            Self::MageAuto | Self::MageManualFallback | Self::MageManualNoFallback
        )
    }

    fn is_manual_magic(self) -> bool {
        matches!(self, Self::MageManualFallback | Self::MageManualNoFallback)
    }

    fn is_magic_fallback(self) -> bool {
        self == Self::MageManualFallback
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
    ranged_start_first_missing_tick: Option<u32>,
    magic_tele_tile: Option<WorldTile>,
}

impl LiveState {
    fn frame(&mut self, client: &mut client::client::Client, hold: bool) {
        let drain = self.pump.drain_client(client);
        host::publish_snapshot(&mut self.snapshot, client, drain);
        if self.case.is_magic() && self.started {
            self.record_magic_npc_events(client);
        }

        if self.runner.on_start_script() && !self.started {
            combat_proof::record_start_baseline(&self.account, &self.snapshot);
            let baseline = combat_proof::snapshot_facts(&self.snapshot, None);
            let magic_setup = self
                .capture
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .magic_setup
                .clone();
            let preflight = if self.case.is_ranged() {
                let retaliation_off = self
                    .snapshot
                    .varps()
                    .iter()
                    .find(|row| row.index == AUTO_RETALIATE_VARP)
                    .is_some_and(|row| row.value == 1);
                let mut capture = self.capture.lock().unwrap_or_else(|e| e.into_inner());
                let interference = ranged_spawned_index(&capture)
                    .and_then(|index| usize::try_from(index).ok())
                    .and_then(|index| {
                        ranged_snapshot_other_player(&self.snapshot, index)
                            .map(|player| (index, player))
                    });
                if let Some((npc_index, player_index)) = interference {
                    if let Some(event) = ranged_spawn_event_mut(&mut capture) {
                        event["interference"] = json!({"player_index": player_index});
                    }
                    Some(ranged_interference_reason(
                        self.case,
                        npc_index,
                        player_index,
                    ))
                } else if !retaliation_off {
                    Some("ranged Start has auto-retaliate enabled".to_owned())
                } else {
                    ranged_start_preflight(self.case, &baseline, &self.selected, &capture)
                }
            } else {
                start_preflight_at(
                    self.case,
                    &baseline,
                    self.magic_tele_tile,
                    magic_setup.as_ref(),
                )
            };
            if let Some(reason) = preflight {
                let should_resample = if self.case == Case::R1 {
                    let capture = self.capture.lock().unwrap_or_else(|e| e.into_inner());
                    ranged_start_resample_pending(
                        &baseline,
                        &capture,
                        self.snapshot.tick(),
                        &mut self.ranged_start_first_missing_tick,
                    )
                } else {
                    false
                };
                if should_resample {
                    return;
                }
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

    fn record_magic_npc_events(&self, client: &client::client::Client) {
        let mut capture = self
            .capture
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        for npc in self
            .snapshot
            .npcs()
            .iter()
            .filter(|npc| npc.r#type == Some(477))
        {
            let Some(raw) = client.npc.get(npc.index).and_then(Option::as_ref) else {
                continue;
            };
            let entity = &raw.entity;
            for slot in 0..entity.damage_cycles.len() {
                let cycle = entity.damage_cycles[slot];
                if cycle <= client.loop_cycle
                    || capture.magic_npc_events.iter().any(|event| {
                        event["kind"] == "hitmark"
                            && event["npc_index"] == json!(npc.index)
                            && event["slot"] == json!(slot)
                            && event["cycle"] == json!(cycle)
                    })
                {
                    continue;
                }
                capture.magic_npc_events.push(json!({
                    "kind": "hitmark", "npc_index": npc.index, "slot": slot,
                    "cycle": cycle, "damage": entity.damage_values[slot],
                    "damage_kind": entity.damage_types[slot],
                    "player_generation": client.gens.player,
                    "npc_generation": client.gens.npc,
                    "client_cycle": client.loop_cycle,
                    "health": entity.health, "total_health": entity.total_health,
                }));
            }
            if i64::from(entity.spotanim_id) == FAILED_SPELL_SPLASH
                && entity.spotanim_frame >= 0
                && entity.spotanim_last_cycle <= client.loop_cycle
                && !capture.magic_npc_events.iter().any(|event| {
                    event["kind"] == "splash"
                        && event["npc_index"] == json!(npc.index)
                        && event["spot_animation_stamp"] == json!(entity.spotanim_last_cycle)
                })
            {
                capture.magic_npc_events.push(json!({
                    "kind": "splash", "npc_index": npc.index,
                    "spot_animation": entity.spotanim_id,
                    "spot_animation_stamp": entity.spotanim_last_cycle,
                    "player_generation": client.gens.player,
                    "npc_generation": client.gens.npc,
                    "client_cycle": client.loop_cycle,
                    "health": entity.health, "total_health": entity.total_health,
                }));
            }
        }
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
        let magic = if self.case.is_magic() {
            magic_receipt(&capture, self.case)
        } else {
            Value::Null
        };
        let mut receipt = json!({
            "proof": if self.case == Case::M5Stop {
                "LIFECYCLE-FOLLOWUPS-1"
            } else if self.case.is_magic() {
                "COMBAT-S3C"
            } else {
                "COMBAT-S3A-3"
            },
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
            "magic_setup": capture.magic_setup,
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
            "magic": magic,
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
    // The runner supplies a per-run writable APFS clone. Never mutate the
    // retained snapshot or copy it again into the throwaway profile.
    let cache = std::env::var_os("BOT_CACHE_DIR")
        .map(PathBuf::from)
        .filter(|path| path.is_dir())
        .ok_or_else(|| "combat live proof requires a per-run BOT_CACHE_DIR clone".to_owned())?;
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
    // Set the quest stage and disable auto-retaliation before arriving within
    // the natural Warlord's range; its real dialogue starts the encounter.
    let pre_stand_steps = if case.is_ranged() {
        vec![
            cheat_step(
                "seed Tree Gnome Village at returned-first-orb stage",
                "setvar treequest 7".to_owned(),
                // treequest is server-only (scope=perm, no transmit). The
                // cheat acknowledgement proves seeding; real dialogue proves
                // that the seeded content stage permits this encounter.
                Proof::Chat {
                    needle: "set treequest: to 7",
                },
            ),
            ranged_retaliation_off_step(),
        ]
    } else {
        Vec::new()
    };
    let pre_stand_count = pre_stand_steps.len();
    scenario
        .steps
        .splice(stand_index + 1..stand_index + 1, pre_stand_steps);
    let preparation_index =
        stand_index + (if placement.is_some() { 2 } else { 1 }) + pre_stand_count;
    let preparation = preparation_steps(case, stand, placement, Arc::clone(&capture));
    scenario
        .steps
        .splice(preparation_index..preparation_index, preparation);
    if case.is_magic() {
        let stand_index = scenario
            .steps
            .iter()
            .position(|step| step.name == "stand at the quest start")
            .expect("Quester stage keeps its settled stand step");
        scenario.steps.insert(
            stand_index + 1,
            magic_spawn_teleport_step(stand, Arc::clone(&capture)),
        );
    }
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
        Case::MageAuto | Case::MageManualFallback | Case::MageManualNoFallback => &[
            ("magic", 6, 35),
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
            steps.push(ranged_natural_actor_observed_step(
                case,
                placement.spawn,
                Arc::clone(&capture),
            ));
            steps.push(ranged_warlord_talk_step(case, Arc::clone(&capture)));
            steps.push(ranged_dialogue_open_observed_step(
                case,
                Arc::clone(&capture),
            ));
            steps.push(ranged_dialogue_drain_step(case, Arc::clone(&capture)));
            steps.push(ranged_dialogue_settled_step(case, Arc::clone(&capture)));
            steps.push(ranged_tele_step(
                case,
                placement.stands,
                Arc::clone(&capture),
            ));
            steps.push(ranged_tele_observed_step(case, Arc::clone(&capture)));
        }
        Case::MageAuto | Case::MageManualFallback | Case::MageManualNoFallback => {
            for (alias, id, count) in [
                ("staff_of_fire", STAFF_OF_FIRE_ID, 1),
                ("chaosrune", CHAOS_RUNE_ID, MAGIC_CHAOS_RUNES),
                ("airrune", AIR_RUNE_ID, MAGIC_AIR_RUNES),
                ("mindrune", MIND_RUNE_ID, MAGIC_MIND_RUNES),
                (
                    "4doseprayerrestore",
                    PRAYER_POTION_4_ID,
                    MAGIC_PRAYER_RESTORES,
                ),
            ] {
                steps.push(cheat_step(
                    "seed exact mage equipment and runes before Start",
                    format!("give {alias} {count}"),
                    Proof::ItemId { id, count },
                ));
            }
            steps.push(wear_step(STAFF_OF_FIRE_ID));
        }
    }
    steps
}

fn ranged_spawn_event(capture: &CombatCapture) -> Option<&Value> {
    capture
        .random_events
        .iter()
        .rev()
        .find(|event| event["kind"] == json!("RangedNaturalDialogue"))
}

fn ranged_spawn_event_mut(capture: &mut CombatCapture) -> Option<&mut Value> {
    capture
        .random_events
        .iter_mut()
        .rev()
        .find(|event| event["kind"] == json!("RangedNaturalDialogue"))
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

const TREEQUEST_VARP: i32 = 111;
const TREEQUEST_RETURNED_FIRST_ORB: i32 = 7;
const AUTO_RETALIATE_VARP: i32 = 172;

fn ranged_retaliation_off_step() -> Step {
    Step {
        name: "disable auto retaliation before Warlord dialogue",
        kind: StepKind::Repeat {
            send: Box::new(|client, snapshot| {
                matches!(
                    Interactions::new(snapshot, client).set_retaliate(false),
                    SendResult::Sent { .. }
                )
            }),
        },
        wait: Wait {
            arm: Proof::VarpExact {
                id: AUTO_RETALIATE_VARP,
                value: 1,
            },
            budget_ticks: 120,
        },
    }
}

fn ranged_other_player_index(
    local_slot: usize,
    npc_index: usize,
    npc_target: Option<api::snapshot::ActorTargetView>,
    mut players: impl Iterator<Item = (usize, Option<api::snapshot::ActorTargetView>)>,
) -> Option<usize> {
    if let Some(target) =
        npc_target.filter(|target| target.kind == ActorKind::Player && target.index != local_slot)
    {
        return Some(target.index);
    }
    players.find_map(|(player_index, target)| {
        (player_index != local_slot
            && target
                .is_some_and(|target| target.kind == ActorKind::Npc && target.index == npc_index))
        .then_some(player_index)
    })
}

fn ranged_snapshot_other_player(snapshot: &GameSnapshot, npc_index: usize) -> Option<usize> {
    let npc_target = snapshot
        .npcs()
        .iter()
        .find(|npc| npc.index == npc_index && npc.r#type == Some(WARLORD_NPC_ID))
        .and_then(|npc| npc.target);
    ranged_other_player_index(
        snapshot.self_slot() as usize,
        npc_index,
        npc_target,
        snapshot
            .players()
            .iter()
            .map(|player| (player.index, player.actor.target)),
    )
}

fn ranged_interference_reason(case: Case, npc_index: usize, player_index: usize) -> String {
    format!(
        "{} natural-area interference: another player {player_index} is engaging Khazard Warlord {npc_index}",
        case.key()
    )
}

fn ranged_mark_interference(
    capture: &mut CombatCapture,
    snapshot: &GameSnapshot,
    case: Case,
    npc_index: usize,
) -> Option<usize> {
    let player_index = ranged_snapshot_other_player(snapshot, npc_index)?;
    if let Some(event) = ranged_spawn_event_mut(capture) {
        event["interference"] = json!({"player_index": player_index});
    }
    capture
        .invalid_reason
        .get_or_insert_with(|| ranged_interference_reason(case, npc_index, player_index));
    Some(player_index)
}

fn ranged_check_fixture_interference(
    capture: &mut CombatCapture,
    snapshot: &GameSnapshot,
    case: Case,
) -> bool {
    let Some(npc_index) =
        ranged_spawned_index(capture).and_then(|index| usize::try_from(index).ok())
    else {
        return false;
    };
    ranged_mark_interference(capture, snapshot, case, npc_index).is_some()
}

fn ranged_natural_actor_observed_step(
    case: Case,
    approach: WorldTile,
    capture: Arc<Mutex<CombatCapture>>,
) -> Step {
    let ready_capture = Arc::clone(&capture);
    Step {
        name: "wait up to 350 game ticks for the natural Khazard Warlord row at its map spawn",
        kind: StepKind::Await {
            evidence: "natural Khazard Warlord row within its nine-tile map spawn radius",
            ready: Box::new(move |snapshot| {
                let mut capture = ready_capture.lock().unwrap_or_else(|e| e.into_inner());
                if capture.invalid_reason.is_some() || ranged_spawn_event(&capture).is_some() {
                    return true;
                }
                let varp = |id| {
                    snapshot
                        .varps()
                        .iter()
                        .find(|row| row.index == id)
                        .map(|row| row.value)
                };
                if varp(AUTO_RETALIATE_VARP) != Some(1) {
                    return false;
                }
                // The map actor wanders up to nine network tiles from its
                // selected-content anchor; its anchor is not its live tile.
                let Some(npc) = snapshot.npcs().iter().find(|npc| {
                    npc.r#type == Some(WARLORD_NPC_ID)
                        && tile_distance(npc.network, WARLORD_ANCHOR) <= WARLORD_WANDER_RANGE
                }) else {
                    return false;
                };
                let other_player = ranged_snapshot_other_player(snapshot, npc.index);
                capture.random_events.push(json!({
                    "kind": "RangedNaturalDialogue",
                    "natural_npc": "khazard_warlord",
                    "npc_type": WARLORD_NPC_ID,
                    "npc_anchor": tile_value(WARLORD_ANCHOR),
                    "spawn_tile": tile_value(npc.network),
                    "approach_tile": tile_value(approach),
                    "stand_tile": null,
                    "spawned_index": npc.index,
                    "treequest_varp": TREEQUEST_VARP,
                    "treequest_value": TREEQUEST_RETURNED_FIRST_ORB,
                    "retaliate_varp": AUTO_RETALIATE_VARP,
                    "retaliate_off_before_talk": true,
                    "talk_sent": false,
                    "dialogue_opened": false,
                    "dialogue_settled": false,
                    "dialogue_text": [],
                    "tele_observed": false,
                    "interference": other_player.map(|player_index| json!({
                        "player_index": player_index,
                    })).unwrap_or(Value::Null),
                }));
                if let Some(player_index) = other_player {
                    capture.invalid_reason =
                        Some(ranged_interference_reason(case, npc.index, player_index));
                }
                true
            }),
        },
        wait: Wait {
            arm: Proof::NpcNameNear {
                name: "Khazard warlord",
                x: WARLORD_ANCHOR.x,
                z: WARLORD_ANCHOR.z,
                level: WARLORD_ANCHOR.level,
                radius: WARLORD_WANDER_RANGE,
            },
            budget_ticks: RANGED_NATURAL_WARLORD_WAIT_TICKS,
        },
    }
}

fn ranged_warlord_talk_step(case: Case, capture: Arc<Mutex<CombatCapture>>) -> Step {
    Step {
        name: "talk to the natural Khazard Warlord",
        kind: StepKind::Perform {
            send: Box::new(move |client, snapshot| {
                let mut capture = capture.lock().unwrap_or_else(|e| e.into_inner());
                if capture.invalid_reason.is_some() {
                    return true;
                }
                let Some(index) = ranged_spawn_event(&capture)
                    .and_then(|event| event["spawned_index"].as_i64())
                    .and_then(|index| usize::try_from(index).ok())
                else {
                    return false;
                };
                if ranged_mark_interference(&mut capture, snapshot, case, index).is_some() {
                    return true;
                }
                let Some(event) = ranged_spawn_event_mut(&mut capture) else {
                    return false;
                };
                if event["interference"].is_object() {
                    return true;
                }
                if event["retaliate_off_before_talk"] != true
                    || snapshot
                        .varps()
                        .iter()
                        .find(|row| row.index == AUTO_RETALIATE_VARP)
                        .map(|row| row.value)
                        != Some(1)
                    || snapshot.modals().chat != -1
                    || snapshot.chat_continue_component_id() != -1
                {
                    return false;
                }
                let Some(npc) = snapshot
                    .npcs()
                    .iter()
                    .find(|npc| npc.index == index && npc.r#type == Some(WARLORD_NPC_ID))
                else {
                    return false;
                };
                if !matches!(
                    Interactions::new(snapshot, client)
                        .interact(OpTarget::Npc(npc), ActionSpec::Label("Talk-to".into())),
                    SendResult::Sent { .. }
                ) {
                    return false;
                }
                event["talk_sent"] = json!(true);
                event["talk_tick"] = json!(snapshot.tick());
                true
            }),
        },
        wait: Wait {
            arm: Proof::IngameScene2,
            budget_ticks: 120,
        },
    }
}

fn ranged_dialogue_open_observed_step(case: Case, capture: Arc<Mutex<CombatCapture>>) -> Step {
    let ready_capture = Arc::clone(&capture);
    Step {
        name: "observe the opened Warlord dialogue before continuing",
        kind: StepKind::Await {
            evidence: "native Warlord chat after Talk-to",
            ready: Box::new(move |snapshot| {
                let mut capture = ready_capture.lock().unwrap_or_else(|e| e.into_inner());
                if capture.invalid_reason.is_some()
                    || ranged_check_fixture_interference(&mut capture, snapshot, case)
                {
                    return true;
                }
                let Some(event) = ranged_spawn_event_mut(&mut capture) else {
                    return false;
                };
                if event["interference"].is_object() {
                    return true;
                }
                if event["talk_sent"] != true {
                    return false;
                }
                let open = snapshot.chat_continue_component_id() != -1
                    || !snapshot.chat_options().is_empty();
                if open {
                    event["dialogue_opened"] = json!(true);
                    event["dialogue_open_tick"] = json!(snapshot.tick());
                    event["dialogue_text"] = json!(snapshot.chat_modal_texts());
                }
                open
            }),
        },
        wait: Wait {
            arm: Proof::IngameScene2,
            budget_ticks: 120,
        },
    }
}

fn ranged_dialogue_drain_step(case: Case, capture: Arc<Mutex<CombatCapture>>) -> Step {
    Step {
        name: "drain the real Warlord dialogue before Start",
        kind: StepKind::Repeat {
            send: Box::new(move |client, snapshot| {
                let mut capture = capture.lock().unwrap_or_else(|e| e.into_inner());
                if capture.invalid_reason.is_some()
                    || ranged_check_fixture_interference(&mut capture, snapshot, case)
                    || ranged_spawn_event(&capture)
                        .is_none_or(|event| event["dialogue_opened"] != true)
                {
                    return false;
                }
                let mut interactions = Interactions::new(snapshot, client);
                if snapshot.chat_continue_component_id() != -1 {
                    matches!(interactions.continue_dialog(None), SendResult::Sent { .. })
                } else if !snapshot.chat_options().is_empty() {
                    matches!(interactions.answer_choice(1), SendResult::Sent { .. })
                } else {
                    // A sent Continue latches the button to -1 until the
                    // server publishes the next page. Await that observation;
                    // NoActiveContinue still requires the modal to close.
                    true
                }
            }),
        },
        wait: Wait {
            arm: Proof::NoActiveContinue,
            budget_ticks: 120,
        },
    }
}

fn ranged_dialogue_settled_step(case: Case, capture: Arc<Mutex<CombatCapture>>) -> Step {
    let ready_capture = Arc::clone(&capture);
    Step {
        name: "observe the settled Warlord dialogue before ranged Start",
        kind: StepKind::Await {
            evidence: "two closed native-dialogue snapshots",
            ready: Box::new(move |snapshot| {
                let mut capture = ready_capture.lock().unwrap_or_else(|e| e.into_inner());
                if capture.invalid_reason.is_some()
                    || ranged_check_fixture_interference(&mut capture, snapshot, case)
                {
                    return true;
                }
                let Some(event) = ranged_spawn_event_mut(&mut capture) else {
                    return false;
                };
                if event["interference"].is_object() {
                    return true;
                }
                if event["dialogue_opened"] != true {
                    return false;
                }
                let closed = snapshot.modals().chat == -1
                    && snapshot.chat_continue_component_id() == -1
                    && snapshot.chat_options().is_empty();
                if !closed {
                    event["dialogue_closed_polls"] = json!(0);
                    event["dialogue_last_closed_tick"] = Value::Null;
                    return false;
                }
                let tick = i64::from(snapshot.tick());
                if event["dialogue_last_closed_tick"].as_i64() != Some(tick) {
                    let polls = event["dialogue_closed_polls"].as_i64().unwrap_or(0) + 1;
                    event["dialogue_closed_polls"] = json!(polls);
                    event["dialogue_last_closed_tick"] = json!(tick);
                    if polls >= 2 {
                        event["dialogue_settled"] = json!(true);
                        event["dialogue_settled_tick"] = json!(tick);
                    }
                }
                event["dialogue_settled"] == true
            }),
        },
        wait: Wait {
            arm: Proof::NoActiveContinue,
            budget_ticks: 120,
        },
    }
}

fn ranged_tele_observed_step(case: Case, capture: Arc<Mutex<CombatCapture>>) -> Step {
    let ready_capture = Arc::clone(&capture);
    Step {
        name: if case == Case::R2 {
            "observe the natural Warlord attacking at the R2 prep stand"
        } else {
            "observe the natural Warlord at the eight-to-ten-tile ranged stand"
        },
        kind: StepKind::Await {
            evidence: if case == Case::R2 {
                "settled dialogue, adjacent natural Warlord, and live attack evidence"
            } else {
                "settled dialogue, live Warlord identity, and eight-to-ten-tile network staging"
            },
            ready: Box::new(move |snapshot| {
                let mut capture = ready_capture.lock().unwrap_or_else(|e| e.into_inner());
                if capture.invalid_reason.is_some() {
                    return true;
                }
                let Some(event) = ranged_spawn_event_mut(&mut capture) else {
                    return false;
                };
                if event["interference"].is_object() {
                    return true;
                }
                let Some(stand) = value_tile(&event["stand_tile"]) else {
                    return false;
                };
                let Some(spawned_index) = event["spawned_index"]
                    .as_i64()
                    .and_then(|index| usize::try_from(index).ok())
                else {
                    return false;
                };
                let Some(npc) = snapshot
                    .npcs()
                    .iter()
                    .find(|npc| npc.index == spawned_index && npc.r#type == Some(WARLORD_NPC_ID))
                else {
                    return false;
                };
                if let Some(player_index) = ranged_snapshot_other_player(snapshot, spawned_index) {
                    event["interference"] = json!({"player_index": player_index});
                    return true;
                }
                let retaliation_off = snapshot
                    .varps()
                    .iter()
                    .find(|row| row.index == AUTO_RETALIATE_VARP)
                    .is_some_and(|row| row.value == 1);
                let arrived = snapshot.tile() == Some((stand.x, stand.z, stand.level));
                let network_distance = tile_distance(npc.network, stand);
                let live_attack_observed = case != Case::R2
                    || i64::try_from(spawned_index).ok().and_then(|index| {
                        ranged_live_attacker(&combat_proof::snapshot_facts(snapshot, None), index)
                    }) == Some(true);
                if arrived
                    && retaliation_off
                    && ranged_start_distance_valid(case, network_distance)
                    && live_attack_observed
                {
                    event["spawn_tile"] = tile_value(npc.network);
                    event["retaliate_off_at_stand"] = json!(true);
                    event["tele_observed"] = json!(true);
                    event["r2_live_attack_observed_at_start"] = json!(case == Case::R2);
                    event["r2_live_attack_observed_tick"] = if case == Case::R2 {
                        json!(snapshot.tick())
                    } else {
                        Value::Null
                    };
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
    case: Case,
    stands: Vec<WorldTile>,
    capture: Arc<Mutex<CombatCapture>>,
) -> Step {
    let stage_distance = ranged_stage_distance(case);
    let name = if case == Case::R2 {
        "teleport adjacent to the natural Warlord for the live-threat ammo abort"
    } else {
        "teleport nine tiles from the natural Warlord"
    };
    Step {
        name,
        kind: StepKind::Perform {
            send: Box::new(move |client, snapshot| {
                let mut capture = capture.lock().unwrap_or_else(|e| e.into_inner());
                if capture.invalid_reason.is_some() {
                    return true;
                }
                let Some(event) = ranged_spawn_event_mut(&mut capture) else {
                    return false;
                };
                if event["dialogue_settled"] != true
                    || snapshot
                        .varps()
                        .iter()
                        .find(|row| row.index == AUTO_RETALIATE_VARP)
                        .map(|row| row.value)
                        != Some(1)
                {
                    return false;
                }
                let Some(spawned_index) = event["spawned_index"]
                    .as_i64()
                    .and_then(|index| usize::try_from(index).ok())
                else {
                    return false;
                };
                let Some(npc) = snapshot
                    .npcs()
                    .iter()
                    .find(|npc| npc.index == spawned_index && npc.r#type == Some(WARLORD_NPC_ID))
                else {
                    return false;
                };
                if let Some(player_index) = ranged_snapshot_other_player(snapshot, spawned_index) {
                    event["interference"] = json!({"player_index": player_index});
                    return true;
                }
                let Some(stand) = stands
                    .iter()
                    .copied()
                    .find(|stand| tile_distance(npc.network, *stand) == stage_distance)
                else {
                    return false;
                };
                let command = tele_cheat_command(stand);
                let sent = matches!(
                    api::interact::cheat(client, &command),
                    client::CheatSend::Sent
                );
                if sent {
                    event["spawn_tile"] = tile_value(npc.network);
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

fn magic_spawn_teleport_step(spawn: WorldTile, capture: Arc<Mutex<CombatCapture>>) -> Step {
    let tele = WorldTile {
        x: spawn.x + 5,
        z: spawn.z,
        level: spawn.level,
    };
    Step {
        name: "spawn local Khazard Warlord and teleport five tiles east",
        kind: StepKind::Perform {
            send: Box::new(move |client, snapshot| {
                if snapshot.tile() != Some((spawn.x, spawn.z, spawn.level)) {
                    return false;
                }
                if !matches!(
                    api::interact::cheat(client, "npcadd khazard_warlord"),
                    client::CheatSend::Sent
                ) {
                    return false;
                }
                let command = api::interact::tele_args(tele.level, tele.x, tele.z);
                if !matches!(
                    api::interact::cheat(client, &command),
                    client::CheatSend::Sent
                ) {
                    return false;
                }
                capture
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .magic_setup = Some(json!({
                    "npcadd": "khazard_warlord",
                    "spawn_tile": spawn,
                    "tele_tile": tele,
                    "chebyshev_distance": 5,
                }));
                true
            }),
        },
        wait: Wait {
            arm: Proof::Arrived {
                x: tele.x,
                z: tele.z,
                level: tele.level,
            },
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
    stands: Vec<WorldTile>,
}

fn ranged_placement(case: Case, world: &nav::world::NavWorld) -> Result<RangedPlacement, String> {
    if !case.is_ranged() {
        return Err("ranged placement requested for a non-ranged case".to_owned());
    }
    let spawn = standable_neighbor(world, WARLORD_ANCHOR)?;
    // Cover the content's nine-tile wander plus the nine-tile firing offset.
    // Only the account moves; the natural NPC is never repositioned.
    let stand_radius = WARLORD_WANDER_RANGE + RANGED_STAGE_DISTANCE;
    let stands: Vec<_> = (-stand_radius..=stand_radius)
        .flat_map(|dx| {
            (-stand_radius..=stand_radius).map(move |dz| WorldTile {
                x: WARLORD_ANCHOR.x + dx,
                z: WARLORD_ANCHOR.z + dz,
                ..WARLORD_ANCHOR
            })
        })
        .filter(|tile| world.collision.standable(*tile))
        .collect();
    if stands.is_empty() {
        Err("no collision-backed ranged stand near the natural Khazard Warlord".to_owned())
    } else {
        Ok(RangedPlacement { spawn, stands })
    }
}
fn ranged_runtime_fixture(source: &[u8], placement: &RangedPlacement) -> Vec<u8> {
    let mut fixture: Value = serde_json::from_slice(source).expect("parse ranged combat fixture");
    let step = &mut fixture["roles"][0]["sequences"][0]["steps"][0];
    step["args"]["tactic"]["style"] = json!("ranged");
    step["args"]["tactic"]["engage_radius"] = json!(11);
    // Open combat starts from the collision-backed stand selected after
    // dialogue. A static pre-dialogue anchor would walk away from it.
    step["args"]
        .as_object_mut()
        .expect("combat args object")
        .remove("stand");
    step["args"]["area"]["box"] = json!([
        placement.spawn.x - 12,
        placement.spawn.z - 12,
        placement.spawn.x + 12,
        placement.spawn.z + 12,
        placement.spawn.level
    ]);
    serde_json::to_vec(&fixture).expect("serialize runtime ranged combat fixture")
}

fn tile_distance(a: WorldTile, b: WorldTile) -> i32 {
    if a.level != b.level {
        i32::MAX
    } else {
        (a.x - b.x).abs().max((a.z - b.z).abs())
    }
}

fn magic_spawn_stand(world: &nav::world::NavWorld, anchor: WorldTile) -> Result<WorldTile, String> {
    // A clear two-tile-wide walking corridor conservatively excludes scenery
    // and wall faces along the five-tile casting line, including the actor.
    for dz in -8..=8 {
        for dx in -8..=8 {
            let spawn = WorldTile {
                x: anchor.x + dx,
                z: anchor.z + dz,
                ..anchor
            };
            if (0..=5).all(|east| {
                (0..=1).all(|north| {
                    world.collision.walkable(WorldTile {
                        x: spawn.x + east,
                        z: spawn.z + north,
                        ..spawn
                    })
                })
            }) {
                return Ok(spawn);
            }
        }
    }
    Err(format!(
        "no clear five-tile magic staging corridor around {anchor:?}"
    ))
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
        Case::MageAuto | Case::MageManualFallback | Case::MageManualNoFallback => {
            if stat_pair(baseline, "magic") != Some((35, 35))
                || stat_pair(baseline, "defence") != Some((40, 40))
                || stat_pair(baseline, "hitpoints") != Some((40, 40))
                || stat_pair(baseline, "prayer") != Some((43, 43))
                || prayer_varp(baseline, 97) != Some(0)
                || item_count(baseline, CHAOS_RUNE_ID) != i64::from(MAGIC_CHAOS_RUNES)
                || item_count(baseline, AIR_RUNE_ID) != i64::from(MAGIC_AIR_RUNES)
                || item_count(baseline, MIND_RUNE_ID) != i64::from(MAGIC_MIND_RUNES)
                || item_count(baseline, PRAYER_POTION_4_ID) != i64::from(MAGIC_PRAYER_RESTORES)
                || item_count(baseline, STAFF_OF_FIRE_ID) != 0
                || equipment_count(baseline, STAFF_OF_FIRE_ID) != 1
            {
                return Some(format!(
                    "{} did not reach magic35/defence40/HP40/prayer43 with the exact staff, 12/150/90 runes, and two Prayer(4) restores",
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
                    "{} local setup expected one Khazard Warlord within nine tiles, observed {warlords:?}",
                    case.key()
                ));
            }
            if let Some(threats) = local_threats(baseline).filter(|rows| rows.len() > 1) {
                return Some(format!(
                    "{} local setup has multiple max-9 aggressors at Start: {threats:?}",
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
fn ranged_start_resample_pending(
    baseline: &Value,
    capture: &CombatCapture,
    current_tick: u32,
    first_missing_tick: &mut Option<u32>,
) -> bool {
    let Some(spawned_index) = ranged_spawned_index(capture) else {
        return false;
    };
    let actor_visible = baseline["nearby_npcs"].as_array().is_some_and(|npcs| {
        npcs.iter().any(|npc| {
            npc["index"].as_i64() == Some(spawned_index) && npc["type"] == json!(WARLORD_NPC_ID)
        })
    });
    if actor_visible {
        return false;
    }
    let first_tick = *first_missing_tick.get_or_insert(current_tick);
    current_tick.saturating_sub(first_tick) <= RANGED_START_RESAMPLE_TICKS
}

fn ranged_stage_distance(case: Case) -> i32 {
    if case == Case::R2 {
        R2_STAGE_DISTANCE
    } else {
        RANGED_STAGE_DISTANCE
    }
}

fn ranged_start_distance_valid(case: Case, distance: i32) -> bool {
    match case {
        Case::R2 => (R2_STAGE_DISTANCE..=R2_START_DISTANCE_MAX).contains(&distance),
        Case::R1 | Case::R3 => {
            (RANGED_START_DISTANCE_MIN..=RANGED_START_DISTANCE_MAX).contains(&distance)
        }
        _ => false,
    }
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
        return Some("ranged Start has no recorded natural Warlord dialogue fixture".to_owned());
    };
    if spawn_event["interference"].is_object() {
        return Some(format!(
            "{} natural-area interference before Start: {}",
            case.key(),
            spawn_event["interference"]
        ));
    }
    if spawn_event["natural_npc"] != json!("khazard_warlord")
        || spawn_event["npc_type"] != json!(WARLORD_NPC_ID)
        || !tile_is(&spawn_event["npc_anchor"], WARLORD_ANCHOR)
        || spawn_event["treequest_varp"] != json!(TREEQUEST_VARP)
        || spawn_event["treequest_value"] != json!(TREEQUEST_RETURNED_FIRST_ORB)
        || spawn_event["retaliate_varp"] != json!(AUTO_RETALIATE_VARP)
        || spawn_event["retaliate_off_before_talk"] != true
        || spawn_event["retaliate_off_at_stand"] != true
        || spawn_event["talk_sent"] != true
        || spawn_event["dialogue_opened"] != true
        || spawn_event["dialogue_settled"] != true
    {
        return Some(
            "ranged Start did not observe the returned-first-orb stage, auto-retaliate off, and a fully settled natural Warlord dialogue"
                .to_owned(),
        );
    }
    let (Some(spawn), Some(stand), Some(spawned_index)) = (
        value_tile(&spawn_event["spawn_tile"]),
        value_tile(&spawn_event["stand_tile"]),
        spawn_event["spawned_index"].as_i64(),
    ) else {
        return Some(
            "ranged Start fixture lacks its natural Warlord identity or stand tile".to_owned(),
        );
    };
    let Some(player_tile) = value_tile(&baseline["local_player"]["network"]) else {
        return Some("ranged Start baseline has no local network tile".to_owned());
    };
    let target = baseline["nearby_npcs"].as_array().and_then(|npcs| {
        npcs.iter().find(|npc| {
            npc["index"] == json!(spawned_index) && npc["type"] == json!(WARLORD_NPC_ID)
        })
    });
    let Some(target_tile) = target.and_then(|npc| value_tile(&npc["network"])) else {
        return Some("ranged Start baseline lost the natural Warlord network tile".to_owned());
    };
    let other_player_targets_warlord = target.is_some_and(|npc| {
        npc["target"]["kind"] == json!("Player")
            && npc["target"]["index"].as_i64() != baseline["self_slot"].as_i64()
    });
    if other_player_targets_warlord {
        return Some(format!(
            "{} natural-area interference: another player owns the Warlord target at Start",
            case.key()
        ));
    }
    if spawn_event["tele_observed"] != true
        || tile_distance(spawn, stand) != ranged_stage_distance(case)
        || !ranged_start_distance_valid(case, tile_distance(target_tile, player_tile))
        || player_tile != stand
    {
        return Some(if case == Case::R2 {
            "R2 Start did not preserve its natural Warlord at adjacent attack range".to_owned()
        } else {
            "ranged Start did not preserve the natural Warlord identity and observed eight-to-ten-tile network staging"
                .to_owned()
        });
    }
    if case == Case::R2 && ranged_warlord_live_attack(baseline, spawned_index) != Some(true) {
        return Some(
            "R2 Start did not observe the natural Warlord actively attacking before Prep"
                .to_owned(),
        );
    }
    if stat_pair(baseline, "ranged") != Some((70, 70))
        || stat_pair(baseline, "defence") != Some((40, 40))
        || !stat_pair(baseline, "hitpoints")
            .is_some_and(|(base, current)| base == 40 && (1..=40).contains(&current))
        || stat_pair(baseline, "prayer") != Some((43, 43))
    {
        return Some(
            "ranged Start did not reach ranged70, defence40, base hitpoints40 with nonzero current HP, prayer43".to_owned(),
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
        || item_count(baseline, LOBSTER_ID) != 6
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

fn start_preflight_at(
    case: Case,
    baseline: &Value,
    magic_tele_tile: Option<WorldTile>,
    magic_setup: Option<&Value>,
) -> Option<String> {
    if let Some(reason) = start_preflight(case, baseline) {
        return Some(reason);
    }
    if case.is_magic() {
        let Some(tele) = magic_tele_tile else {
            return Some("magic proof is missing its five-tile teleport destination".to_owned());
        };
        let Some(setup) = magic_setup else {
            return Some("magic proof is missing its local npcadd preparation receipt".to_owned());
        };
        let spawn = WorldTile {
            x: tele.x - 5,
            z: tele.z,
            level: tele.level,
        };
        if setup["npcadd"] != json!("khazard_warlord")
            || !tile_is(&setup["spawn_tile"], spawn)
            || !tile_is(&setup["tele_tile"], tele)
            || setup["chebyshev_distance"] != json!(5)
            || baseline["tile"] != json!([tele.x, tele.z, tele.level])
        {
            return Some("magic proof did not observe npcadd at the stand then a five-tile east teleport before Start".to_owned());
        }
    }
    None
}

fn case_ready(case: Case, capture: &CombatCapture) -> bool {
    if capture.invalid_reason.is_some() || capture.start_baseline.is_none() {
        return false;
    }
    if (matches!(case, Case::M2 | Case::M3 | Case::M6) || case.is_magic())
        && has_multiple_local_threats(capture)
    {
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
        Case::MageAuto => magic_ready(Case::MageAuto, capture),
        Case::MageManualFallback => magic_ready(Case::MageManualFallback, capture),
        Case::MageManualNoFallback => magic_ready(Case::MageManualNoFallback, capture),
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
    if before == after {
        let requested_take = request["op"] == json!("obj")
            && request["action"]
                .as_str()
                .is_some_and(|action| action.eq_ignore_ascii_case("take"))
            && request["name"]
                .as_str()
                .is_some_and(|name| !name.trim().is_empty());
        return requested_take.then_some(0);
    }
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
        .all(|count| count.is_some_and(|count| count >= 0));
    let swept_units = pickup_counts
        .iter()
        .try_fold(0_i64, |total, count| total.checked_add((*count)?));
    let pickup_plan_shape_valid = pickups.iter().all(|action| {
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
    });
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
        && pickup_counts_valid
        && pickup_plan_shape_valid
        && dart_count_per_launch;
    json!({
        "ammo_obj_id": ammo_id,
        "ammo_alias": ammo_alias,
        "start_count": ammo_start,
        "launches": launches.len(),
        "pickup_actions": pickups,
        "pickup_action_count": pickups.len(),
        "pickup_action_limit_valid": pickup_limit,
        "corpse_tick": killed_tick,
        "held_count_at_kill": ammo_at_kill,
        "pickup_stack_counts": pickup_counts,
        "sweep_observed": sweep_observed,
        "swept_units": swept_units,
        "all_pickups_after_kill": pickup_after_kill,
        "pickup_count_observations_valid": pickup_counts_valid,
        "pickup_plan_shape_valid": pickup_plan_shape_valid,
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
    let no_attack = !capture.actions.iter().any(is_npc_attack);
    let probes = ranged_probes(capture).collect::<Vec<_>>();
    let no_projectile = !probes.is_empty()
        && probes.iter().all(|probe| {
            probe["projectiles"]
                .as_array()
                .is_some_and(|projectiles| projectiles.is_empty())
        });
    report["fields"]["combat_end"] == json!("Aborted(PrepFailed(Ammo))")
        && no_attack
        && no_projectile
        && ranged_abort_safety_receipt(capture, report)["valid"] == json!(true)
}

fn ranged_abort_terminal_kind(capture: &CombatCapture) -> Option<&'static str> {
    let status = capture.statuses.last()?;
    if status["phase"] != json!("Blocked") {
        return None;
    }
    match status["failure"].as_str()? {
        failure if failure.contains(RANGED_ABORT_HOLD_SAFE_REASON) => Some("hold_safe"),
        failure if failure.contains(RANGED_ABORT_HOLD_BOUND_REASON) => Some("hold_bound"),
        failure if failure.contains(RANGED_ABORT_HOLD_UNAVAILABLE_REASON) => Some("unavailable"),
        failure if failure.contains(RANGED_ABORT_WALK_FAILED_REASON) => Some("walk_failed"),
        failure if failure.contains(RANGED_ABORTED_COMBAT_STEP_REASON) => Some("walk_completed"),
        _ => None,
    }
}

fn npc_live_attack_evidence(npc: &Value) -> Option<bool> {
    Some(targets_local_with_attack_evidence(
        npc["targets_local"].as_bool()?,
        npc["attack_animation"].as_bool()?,
        npc["attack_spot"].as_bool()?,
        npc["local_hitmark_recent"].as_bool()?,
    ))
}

fn ranged_warlord_live_attack(frame: &Value, spawned_index: i64) -> Option<bool> {
    let warlord_type = i64::try_from(WARLORD_NPC_ID).ok()?;
    let npcs = frame["nearby_npcs"].as_array()?;
    let warlord = npcs.iter().find(|npc| {
        npc["index"].as_i64() == Some(spawned_index) && npc["type"].as_i64() == Some(warlord_type)
    })?;
    npc_live_attack_evidence(warlord)
}

fn ranged_live_attacker(frame: &Value, spawned_index: i64) -> Option<bool> {
    let warlord_type = i64::try_from(WARLORD_NPC_ID).ok()?;
    let npcs = frame["nearby_npcs"].as_array()?;
    if !npcs.iter().any(|npc| {
        npc["index"].as_i64() == Some(spawned_index) && npc["type"].as_i64() == Some(warlord_type)
    }) {
        return None;
    }
    let mut live = false;
    for npc in npcs {
        live |= npc_live_attack_evidence(npc)?;
    }
    Some(live)
}

fn ranged_abort_terminal_tick(capture: &CombatCapture) -> Option<i64> {
    let status = capture.statuses.last()?;
    (status["phase"] == json!("Blocked"))
        .then(|| status["observed_tick"].as_i64())
        .flatten()
}

fn ranged_abort_blocked_after_horizon(
    capture: &CombatCapture,
    last_capture_tick: Option<i64>,
    horizon_end: Option<i64>,
) -> bool {
    capture
        .statuses
        .last()
        .is_some_and(|status| status["phase"] == json!("Blocked"))
        && last_capture_tick
            .zip(horizon_end)
            .is_some_and(|(last, end)| last >= end)
}

fn ranged_abort_walk_action(
    action: &Value,
    report_tick: i64,
    horizon_end: i64,
) -> Option<(i64, WorldTile, i32)> {
    if action["kind"] != json!("walk") {
        return None;
    }
    let tick = action["tick"].as_i64()?;
    if !(report_tick + 1..=horizon_end).contains(&tick) {
        return None;
    }
    let target = value_tile(&action["request"]["target"])?;
    let radius = i32::try_from(action["request"]["radius"].as_i64()?)
        .ok()
        .filter(|radius| *radius >= 0)?;
    Some((tick, target, radius))
}

fn ranged_abort_walk_observed(capture: &CombatCapture, report_tick: i64, horizon_end: i64) -> bool {
    capture
        .actions
        .iter()
        .any(|action| ranged_abort_walk_action(action, report_tick, horizon_end).is_some())
}

fn ranged_abort_route_arrived(capture: &CombatCapture, report_tick: i64, horizon_end: i64) -> bool {
    capture.actions.iter().any(|action| {
        let Some((walk_tick, target, radius)) =
            ranged_abort_walk_action(action, report_tick, horizon_end)
        else {
            return false;
        };
        let Some(start) = value_tile(&action["snapshot"]["local_player"]["network"]) else {
            return false;
        };
        tile_distance(start, target) > radius
            && capture.frames.iter().any(|frame| {
                frame["tick"]
                    .as_i64()
                    .is_some_and(|tick| (walk_tick..=horizon_end).contains(&tick))
                    && value_tile(&frame["local_player"]["network"])
                        .is_some_and(|tile| tile_distance(tile, target) <= radius)
            })
    })
}

fn ranged_abort_protect_action_at(capture: &CombatCapture, tick: i64, component: i64) -> bool {
    capture.actions.iter().any(|action| {
        action["kind"] == json!("guard")
            && action["op"] == json!("if-button")
            && action["component_id"].as_i64() == Some(component)
            && action["accepted"] == json!(true)
            && action["tick"].as_i64() == Some(tick)
    })
}

fn ranged_abort_food_guard_seen(capture: &CombatCapture, report_tick: i64, tick: i64) -> bool {
    capture.actions.iter().any(|action| {
        action["kind"] == json!("guard")
            && matches!(action["op"].as_str(), Some("eat" | "drink"))
            && action["accepted"] == json!(true)
            && action["tick"]
                .as_i64()
                .is_some_and(|action_tick| (report_tick + 1..=tick).contains(&action_tick))
    })
}

fn ranged_abort_safety_receipt(capture: &CombatCapture, report: &Value) -> Value {
    let Some(report_tick) = integer(report, "combat_evidence_tick") else {
        return json!({"valid": false, "error": "abort report tick missing"});
    };
    let Some(horizon_start) = report_tick.checked_add(1) else {
        return json!({"valid": false, "error": "abort report tick overflow"});
    };
    let Some(horizon_end) = report_tick.checked_add(RANGED_ABORT_SURVIVAL_TICKS) else {
        return json!({"valid": false, "error": "abort survival horizon overflow"});
    };
    let Some(spawned_index) = ranged_spawned_index(capture) else {
        return json!({"valid": false, "error": "natural Warlord identity missing"});
    };
    let Some(protect_component) = prayer_component(capture, "Protect from Melee") else {
        return json!({"valid": false, "error": "Protect from Melee component missing"});
    };
    let Some(protect_varp) = prayer_varp_for_name(capture, "Protect from Melee") else {
        return json!({"valid": false, "error": "Protect from Melee varp missing"});
    };
    let Some(report_frame) = capture
        .frames
        .iter()
        .rev()
        .find(|frame| frame["tick"].as_i64() == Some(report_tick))
    else {
        return json!({"valid": false, "error": "abort-frame observation missing"});
    };
    let Some(report_hp) = stat_effective(report_frame, "hitpoints") else {
        return json!({"valid": false, "error": "abort-frame hitpoints missing"});
    };

    let threat_at_abort = ranged_live_attacker(report_frame, spawned_index);
    let terminal_kind = ranged_abort_terminal_kind(capture);
    let hold_safe_terminal_tick = if terminal_kind == Some("hold_safe") {
        ranged_abort_terminal_tick(capture)
    } else {
        None
    };
    let hold_safe_terminal_in_horizon =
        hold_safe_terminal_tick.is_some_and(|tick| (horizon_start..=horizon_end).contains(&tick));
    let mut observed_ticks = 0;
    let mut complete = threat_at_abort.is_some()
        && (terminal_kind != Some("hold_safe") || hold_safe_terminal_in_horizon);
    let mut positive_hitpoints = true;
    let mut no_hitpoint_loss_under_threat = true;
    let mut guarded_while_threatened = true;
    let mut live_threat_seen = threat_at_abort == Some(true);
    let mut no_threat_at_horizon_end = true;
    let mut horizon_end_seen = false;
    let mut hold_safe_terminal_seen = false;
    let mut disengaged_from_hold_safe_terminal = true;
    let mut previous_hp = report_hp;
    for tick in horizon_start..=horizon_end {
        let mut found_tick = false;
        for frame in capture
            .frames
            .iter()
            .filter(|frame| frame["tick"].as_i64() == Some(tick))
        {
            found_tick = true;
            if frame["ingame"] != json!(true) {
                complete = false;
            }
            let Some(hitpoints) = stat_effective(frame, "hitpoints") else {
                positive_hitpoints = false;
                continue;
            };
            if hitpoints <= 0 {
                positive_hitpoints = false;
            }
            let Some(live_attacker) = ranged_live_attacker(frame, spawned_index) else {
                complete = false;
                continue;
            };
            if live_attacker && hitpoints < previous_hp {
                no_hitpoint_loss_under_threat = false;
            }
            if live_attacker {
                live_threat_seen = true;
                let Some((prayer_level, prayer_points)) = stat_pair(frame, "prayer") else {
                    guarded_while_threatened = false;
                    previous_hp = hitpoints;
                    continue;
                };
                let protected = prayer_varp(frame, protect_varp) == Some(1)
                    || ranged_abort_protect_action_at(capture, tick, protect_component);
                if prayer_level >= RANGED_ABORT_MIN_PROTECT_LEVEL && prayer_points > 0 {
                    guarded_while_threatened &= protected;
                } else {
                    guarded_while_threatened &=
                        protected || ranged_abort_food_guard_seen(capture, report_tick, tick);
                }
            }
            if terminal_kind == Some("hold_safe")
                && hold_safe_terminal_tick.is_some_and(|terminal_tick| tick >= terminal_tick)
            {
                if Some(tick) == hold_safe_terminal_tick {
                    hold_safe_terminal_seen = true;
                }
                disengaged_from_hold_safe_terminal &= !live_attacker;
            }
            if tick == horizon_end {
                horizon_end_seen = true;
                no_threat_at_horizon_end &= !live_attacker;
            }
            previous_hp = hitpoints;
        }
        if found_tick {
            observed_ticks += 1;
        } else {
            complete = false;
        }
    }
    let walk_observed = ranged_abort_walk_observed(capture, report_tick, horizon_end);
    let route_arrived = ranged_abort_route_arrived(capture, report_tick, horizon_end);
    let safe_escape = route_arrived && horizon_end_seen && no_threat_at_horizon_end;
    let terminal_valid = threat_at_abort == Some(true)
        && match terminal_kind {
            Some("hold_safe") => hold_safe_terminal_seen && disengaged_from_hold_safe_terminal,
            Some("hold_bound") => live_threat_seen,
            Some("walk_completed") => safe_escape,
            _ => false,
        };
    let route_valid = !live_threat_seen || walk_observed;
    let valid = complete
        && positive_hitpoints
        && no_hitpoint_loss_under_threat
        && guarded_while_threatened
        && route_valid
        && terminal_valid;
    let verified_no_threat =
        complete && horizon_end_seen && threat_at_abort == Some(false) && !live_threat_seen;
    let outcome = if verified_no_threat {
        "verified_no_threat"
    } else if threat_at_abort == Some(true) {
        "live_threat_at_abort"
    } else {
        "unverified_or_threat_after_abort"
    };
    json!({
        "spawned_index": spawned_index,
        "report_tick": report_tick,
        "horizon_start": horizon_start,
        "horizon_end": horizon_end,
        "ticks_observed": observed_ticks,
        "expected_ticks": RANGED_ABORT_SURVIVAL_TICKS,
        "positive_hitpoints": positive_hitpoints,
        "no_hitpoint_loss_under_live_threat": no_hitpoint_loss_under_threat,
        "threat_at_abort": threat_at_abort,
        "live_threat_seen": live_threat_seen,
        "guarded_while_live_threat": guarded_while_threatened,
        "hold_safe_terminal_tick": hold_safe_terminal_tick,
        "hold_safe_disengaged_through_horizon": hold_safe_terminal_seen
            && disengaged_from_hold_safe_terminal,
        "walk_after_abort": walk_observed,
        "route_arrived": route_arrived,
        "safe_escape": safe_escape,
        "terminal": terminal_kind.unwrap_or("missing_or_unaccepted"),
        "outcome": outcome,
        "verified_no_threat": verified_no_threat,
        "valid": valid,
    })
}

fn ranged_ready(case: Case, capture: &CombatCapture) -> bool {
    if !no_melee_offensive_prayers(capture) {
        return false;
    }
    if case == Case::R2 {
        return ranged_wrong_ammo_ready(capture);
    }
    if !magic_protection_timing_ok(capture) {
        return false;
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
        let abort_safety = report_with_end(capture, "Aborted(PrepFailed(Ammo))").map_or_else(
            || json!({"valid": false, "error": "wrong-ammo abort report missing"}),
            |report| ranged_abort_safety_receipt(capture, report),
        );
        let local_projectiles = ranged_probes(capture)
            .flat_map(|probe| probe["projectiles"].as_array().into_iter().flatten())
            .filter(|projectile| projectile["local_launch"] == true)
            .count();
        let protected_during_abort_horizon = abort_safety["threat_at_abort"] == json!(true)
            && abort_safety["guarded_while_live_threat"] == json!(true);
        return json!({
            "selected_facts": selected,
            "spawn": fixture,
            "exact_ammo_abort": ranged_wrong_ammo_ready(capture),
            "protect_timing": "evaluated_in_abort_guarded_horizon",
            "no_melee_offensive_prayers": no_melee_offensive_prayers(capture),
            "protection_valid": protected_during_abort_horizon,
            "abort_safety": abort_safety,
            "ready": ranged_ready(case, capture),
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
        "protect_timing": protection_timing_receipt(capture),
        "no_melee_offensive_prayers": no_melee_offensive_prayers(capture),
        "protection_valid": magic_protection_timing_ok(capture),
        "restoration_runs": report.is_some_and(|report| m2_restoration_runs_ok(capture, report)),
        "batch_plan_contract": batch_plan_contract(capture, &plans),
        "corpse_for_kill": report.is_some_and(|report| ranged_corpse_frame(capture, report).is_some()),
        "ready": ranged_ready(case, capture),
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum MageSpell {
    FireBolt,
    FireStrike,
}

impl MageSpell {
    fn name(self) -> &'static str {
        match self {
            Self::FireBolt => "Fire Bolt",
            Self::FireStrike => "Fire Strike",
        }
    }

    fn widget(self) -> i64 {
        match self {
            Self::FireBolt => FIRE_BOLT_WIDGET,
            Self::FireStrike => FIRE_STRIKE_WIDGET,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct RuneCast {
    tick: i64,
    spell: MageSpell,
}

struct RuneEvidence {
    casts: Vec<RuneCast>,
    coherent: bool,
}

fn inventory_item_count(facts: &Value, id: i32) -> Option<i64> {
    facts["inventory"]
        .as_array()?
        .iter()
        .try_fold(0i64, |total, item| {
            if item["id"].as_i64() == Some(i64::from(id)) {
                total.checked_add(item["count"].as_i64()?)
            } else {
                Some(total)
            }
        })
}

fn magic_rune_evidence(capture: &CombatCapture) -> RuneEvidence {
    let mut evidence = RuneEvidence {
        casts: Vec::new(),
        coherent: true,
    };
    let Some(start) = capture.start_baseline.as_ref() else {
        evidence.coherent = false;
        return evidence;
    };
    let Some(start_tick) = start["snapshot_tick"].as_i64() else {
        evidence.coherent = false;
        return evidence;
    };
    let mut previous = start;
    for frame in &capture.frames {
        let Some(snapshot_tick) = frame["snapshot_tick"].as_i64() else {
            evidence.coherent = false;
            continue;
        };
        if snapshot_tick <= start_tick {
            continue;
        }
        let (Some(before_chaos), Some(after_chaos)) = (
            inventory_item_count(previous, CHAOS_RUNE_ID),
            inventory_item_count(frame, CHAOS_RUNE_ID),
        ) else {
            evidence.coherent = false;
            previous = frame;
            continue;
        };
        let (Some(before_air), Some(after_air)) = (
            inventory_item_count(previous, AIR_RUNE_ID),
            inventory_item_count(frame, AIR_RUNE_ID),
        ) else {
            evidence.coherent = false;
            previous = frame;
            continue;
        };
        let (Some(before_mind), Some(after_mind)) = (
            inventory_item_count(previous, MIND_RUNE_ID),
            inventory_item_count(frame, MIND_RUNE_ID),
        ) else {
            evidence.coherent = false;
            previous = frame;
            continue;
        };
        let chaos_spent = before_chaos - after_chaos;
        let air_spent = before_air - after_air;
        let mind_spent = before_mind - after_mind;
        if chaos_spent < 0 || air_spent < 0 || mind_spent < 0 {
            evidence.coherent = false;
        } else if chaos_spent != 0 || air_spent != 0 || mind_spent != 0 {
            let spell = if chaos_spent == 1 && air_spent == 3 && mind_spent == 0 {
                Some(MageSpell::FireBolt)
            } else if chaos_spent == 0 && air_spent == 2 && mind_spent == 1 {
                Some(MageSpell::FireStrike)
            } else {
                None
            };
            match (spell, frame["tick"].as_i64()) {
                (Some(spell), Some(tick)) => evidence.casts.push(RuneCast { tick, spell }),
                _ => evidence.coherent = false,
            }
        }
        previous = frame;
    }
    evidence
}

fn rune_cast_count(casts: &[RuneCast], spell: MageSpell) -> usize {
    casts.iter().filter(|cast| cast.spell == spell).count()
}

fn rune_cast_cadence_ok(casts: &[RuneCast]) -> bool {
    casts.len() >= 5
        && casts
            .windows(2)
            .all(|pair| pair[1].tick - pair[0].tick >= 5)
}

fn twelve_bolts_then_strikes(casts: &[RuneCast], require_strike: bool) -> bool {
    casts.len() >= 12
        && casts[..12]
            .iter()
            .all(|cast| cast.spell == MageSpell::FireBolt)
        && casts[12..]
            .iter()
            .all(|cast| cast.spell == MageSpell::FireStrike)
        && (!require_strike || casts.len() > 12)
}

fn magic_input_distances(capture: &CombatCapture) -> Vec<i64> {
    capture
        .actions
        .iter()
        .filter(|action| is_npc_attack(action) || action["request"]["op"] == "use-widget-on")
        .filter_map(|action| {
            action["snapshot"]["nearby_npcs"]
                .as_array()?
                .iter()
                .find(|npc| {
                    npc["type"] == json!(477)
                        && npc["name"]
                            .as_str()
                            .is_some_and(|name| name.eq_ignore_ascii_case("khazard warlord"))
                })?["distance"]
                .as_i64()
        })
        .collect()
}

fn magic_cast_distances(capture: &CombatCapture, casts: &[RuneCast]) -> Vec<i64> {
    casts
        .iter()
        .filter_map(|cast| {
            let frame = capture
                .frames
                .iter()
                .filter(|frame| frame["tick"].as_i64() == Some(cast.tick))
                .max_by_key(|frame| frame["snapshot_tick"].as_i64().unwrap_or(i64::MIN))?;
            warlord_observation(frame)?["distance"].as_i64()
        })
        .collect()
}

fn magic_projectile_queues(capture: &CombatCapture, casts: &[RuneCast]) -> Vec<Value> {
    let selected = api::game_data::for_revision(ClientRevision::R289)
        .expect("selected magic projectile facts");
    casts
        .iter()
        .map(|cast| {
            let launch = capture
                .frames
                .iter()
                .filter(|frame| frame["tick"].as_i64() == Some(cast.tick))
                .find_map(|frame| {
                    let npc = warlord_observation(frame)?;
                    let here = frame["tile"].as_array()?;
                    let projectile = frame["projectiles"]
                        .as_array()?
                        .iter()
                        .filter(|projectile| {
                            projectile["target"]["kind"] == "Npc"
                                && projectile["target"]["index"] == npc["index"]
                                && projectile["src"]["x"] == here[0]
                                && projectile["src"]["z"] == here[1]
                                && projectile["src"]["level"] == here[2]
                                && selected.style_spotanims().iter().any(|fact| {
                                    Some(i64::from(fact.spotanim_id))
                                        == projectile["spotanim"].as_i64()
                                        && fact.style & 4 != 0
                                        && fact.location == "projectile"
                                })
                        })
                        .max_by_key(|projectile| projectile["t1"].as_i64())?;
                    Some((frame, npc, projectile))
                });
            let Some((frame, npc, projectile)) = launch else {
                return json!({"cast_tick": cast.tick, "spell": cast.spell.name(),
                "own_projectile_observed": false, "numeric_queue_matches": false,
                "splash_without_damage": false});
            };
            let generation = frame["player_generation"].as_u64();
            let flight = projectile["t1"]
                .as_i64()
                .zip(projectile["t2"].as_i64())
                .map(|(t1, t2)| t2 - t1);
            let distance = flight
                .filter(|flight| *flight >= -5 && (flight + 5) % 10 == 0)
                .map(|flight| (flight + 5) / 10);
            let expected_delay = flight.map(|flight| (51 + flight) / 30 + 1);
            let hit = capture
                .magic_npc_events
                .iter()
                .filter(|event| {
                    event["kind"] == "hitmark"
                        && event["npc_index"] == npc["index"]
                        && matches!(event["damage_kind"].as_i64(), Some(0 | 1))
                        && event["cycle"]
                            .as_i64()
                            .zip(event["client_cycle"].as_i64())
                            .is_some_and(|(expires, now)| expires > now)
                        && event["player_generation"]
                            .as_u64()
                            .zip(generation)
                            .is_some_and(|(hit, launch)| hit > launch && hit <= launch + 5)
                })
                .min_by_key(|event| event["player_generation"].as_u64());
            let actual_delay = hit.and_then(|event| {
                event["player_generation"]
                    .as_u64()
                    .zip(generation)
                    .and_then(|(hit, launch)| i64::try_from(hit - launch).ok())
            });
            let splash = capture.magic_npc_events.iter().find(|event| {
                event["kind"] == "splash"
                    && event["npc_index"] == npc["index"]
                    && event["spot_animation_stamp"] == projectile["t2"]
                    && event["client_cycle"]
                        .as_i64()
                        .zip(event["spot_animation_stamp"].as_i64())
                        .is_some_and(|(now, starts)| now >= starts)
                    && event["health"].as_i64().is_some()
                    && event["health"] == npc["health"]
            });
            json!({
                "cast_tick": cast.tick, "spell": cast.spell.name(),
                "own_projectile_observed": true, "target_index": npc["index"],
                "launch_player_generation": generation, "target_health_at_launch": npc["health"],
                "wire_projectile": projectile, "wire_flight_cycles": flight,
                "wire_distance": distance, "flight_matches_minus5_plus10d": distance.is_some(),
                "numeric_queue_formula": "floor((51 + (t2 - t1))/30) + 1",
                "expected_numeric_delay_ticks": expected_delay,
                "observed_numeric_delay_ticks": actual_delay,
                "fresh_numeric_mask": hit,
                "numeric_queue_matches": actual_delay.is_some() && actual_delay == expected_delay,
                "landed_splash": splash,
                "splash_without_damage": splash.is_some() && hit.is_none(),
            })
        })
        .collect()
}

fn magic_queue_contract(capture: &CombatCapture, casts: &[RuneCast]) -> bool {
    let queues = magic_projectile_queues(capture, casts);
    let distances = queues
        .iter()
        .filter(|queue| queue["numeric_queue_matches"] == true)
        .filter_map(|queue| queue["wire_distance"].as_i64())
        .collect::<std::collections::BTreeSet<_>>();
    !queues.is_empty()
        && queues.iter().all(|queue| {
            queue["own_projectile_observed"] == true
                && queue["flight_matches_minus5_plus10d"] == true
                && (queue["numeric_queue_matches"] == true
                    || queue["splash_without_damage"] == true)
        })
        && distances.len() >= 2
        && distances.iter().any(|distance| *distance >= 3)
}

struct MagicArmActions<'a> {
    side_tab: Option<&'a Value>,
    chooser: &'a Value,
    selection: &'a Value,
    toggle: &'a Value,
}

fn magic_if_button(
    capture: &CombatCapture,
    component: i64,
    after_tick: Option<i64>,
) -> Option<&Value> {
    capture.actions.iter().find(|action| {
        is_if_button(action)
            && action["request"]["component_id"] == json!(component)
            && after_tick.is_none_or(|tick| {
                action["tick"]
                    .as_i64()
                    .is_some_and(|action_tick| action_tick > tick)
            })
    })
}

fn magic_side_tab(capture: &CombatCapture, after_tick: Option<i64>) -> Option<&Value> {
    capture.actions.iter().find(|action| {
        action["kind"] == "interaction"
            && action["request"]["op"] == "side-tab"
            && action["request"]["tab"] == 0
            && after_tick.is_none_or(|tick| {
                action["tick"]
                    .as_i64()
                    .is_some_and(|action_tick| action_tick > tick)
            })
    })
}

fn action_precedes(first: &Value, second: &Value) -> bool {
    first["sequence"]
        .as_u64()
        .zip(second["sequence"].as_u64())
        .is_some_and(|(first, second)| first < second)
}

fn root_visible(frame: &Value, component: i64) -> bool {
    ["main", "side", "chat", "tutorial"]
        .iter()
        .any(|root| frame["roots"][*root].as_i64() == Some(component))
        || frame["roots"]["side_tabs"]
            .as_array()
            .is_some_and(|tabs| tabs.iter().any(|tab| tab["root"] == json!(component)))
}

fn settled_between(first: &Value, next: &Value, settled: impl Fn(&Value) -> bool) -> bool {
    let (Some(first_tick), Some(next_tick)) = (first["tick"].as_i64(), next["tick"].as_i64())
    else {
        return false;
    };
    first_tick < next_tick
        && action_precedes(first, next)
        && next["snapshot"]["tick"].as_i64() == Some(next_tick)
        && settled(&next["snapshot"])
}

fn autocast_arm_actions<'a>(
    capture: &'a CombatCapture,
    spell_component: i64,
    after_tick: Option<i64>,
    before_tick: i64,
    require_side_tab: bool,
) -> Option<MagicArmActions<'a>> {
    let side_tab = if require_side_tab {
        Some(magic_side_tab(capture, after_tick)?)
    } else {
        None
    };
    let chooser = magic_if_button(capture, AUTO_CHOOSER_COMPONENT, after_tick)?;
    let selection = magic_if_button(capture, spell_component, after_tick)?;
    let toggle = magic_if_button(capture, AUTO_TOGGLE_COMPONENT, after_tick)?;
    let next_action_tick = |action: &Value| action["tick"].as_i64();
    if !action_precedes(chooser, selection)
        || !action_precedes(selection, toggle)
        || !next_action_tick(toggle).is_some_and(|tick| tick < before_tick)
        || ![chooser, selection, toggle]
            .into_iter()
            .all(action_wire_valid)
        || !settled_between(chooser, selection, |frame| {
            root_visible(frame, SPELL_PANEL_ROOT)
        })
        || !settled_between(selection, toggle, |frame| {
            root_visible(frame, COMBAT_TAB_ROOT) && frame["magicvarp"] == json!(2)
        })
        || !capture
            .actions
            .iter()
            .filter(|action| is_npc_attack(action))
            .any(|action| {
                action["tick"].as_i64().is_some_and(|tick| {
                    next_action_tick(toggle)
                        .is_some_and(|toggle_tick| toggle_tick < tick && tick <= before_tick)
                }) && action_precedes(toggle, action)
                    && root_visible(&action["snapshot"], COMBAT_TAB_ROOT)
                    && action["snapshot"]["magicvarp"] == json!(3)
            })
    {
        return None;
    }
    if let Some(side_tab) = side_tab {
        if !action_precedes(side_tab, chooser)
            || !action_wire_valid(side_tab)
            || !settled_between(side_tab, chooser, |frame| {
                frame["active_side_tab"] == json!(0) && root_visible(frame, COMBAT_TAB_ROOT)
            })
        {
            return None;
        }
    }
    Some(MagicArmActions {
        side_tab,
        chooser,
        selection,
        toggle,
    })
}

fn warlord_observation(facts: &Value) -> Option<&Value> {
    facts["nearby_npcs"].as_array()?.iter().find(|npc| {
        npc["type"] == json!(477)
            && npc["name"]
                .as_str()
                .is_some_and(|name| name.eq_ignore_ascii_case("khazard warlord"))
    })
}

fn magic_splashes(capture: &CombatCapture, casts: &[RuneCast]) -> Vec<Value> {
    magic_projectile_queues(capture, casts)
        .into_iter()
        .filter(|queue| queue["splash_without_damage"] == true)
        .map(|queue| {
            json!({
                "target_index": queue["target_index"],
                "spot_animation": FAILED_SPELL_SPLASH,
                "spot_animation_stamp": queue["landed_splash"]["spot_animation_stamp"],
                "client_cycle_at_landing": queue["landed_splash"]["client_cycle"],
                "health_before": queue["target_health_at_launch"],
                "health_after": queue["landed_splash"]["health"],
                "rune_cast_tick": queue["cast_tick"],
                "rune_cast_spell": queue["spell"],
                "cast_counted_from_rune_consumption": true,
                "no_fresh_numeric_damage_mask": true,
            })
        })
        .collect()
}

fn observed_action_tick(action: &Value) -> Option<i64> {
    action["tick"].as_i64()
}

fn action_tick_cadence_ok(actions: &[&Value]) -> bool {
    actions.len() >= 5
        && actions.windows(2).all(|pair| {
            observed_action_tick(pair[0])
                .zip(observed_action_tick(pair[1]))
                .is_some_and(|(previous, current)| current - previous >= 5)
        })
}

fn manual_magic_cast_actions(capture: &CombatCapture) -> Vec<&Value> {
    capture
        .actions
        .iter()
        .filter(|action| action["request"]["op"] == "use-widget-on")
        .collect()
}

fn manual_magic_cast_contract(capture: &CombatCapture, casts: &[RuneCast]) -> bool {
    let actions = manual_magic_cast_actions(capture);
    actions.len() == casts.len()
        && actions.len() >= 5
        && actions.iter().zip(casts).all(|(action, cast)| {
            let expected_wire = [
                i64::from(client::io::ClientProt289::MOVE_OPCLICK.id),
                i64::from(client::io::ClientProt289::OPNPCT.id),
            ];
            action["kind"] == "interaction"
                && action["request"]["component_id"] == json!(cast.spell.widget())
                && action["request"]["kind"] == "npc"
                && action["request"]["target_name"]
                    .as_str()
                    .is_some_and(|name| name.eq_ignore_ascii_case("khazard warlord"))
                && action["request"]["index"].as_i64().is_some()
                && wire_opcodes(action).as_deref() == Some(expected_wire.as_slice())
                && action_wire_valid(action)
                && plan_rows(capture, action)
                    .last()
                    .is_some_and(|last| std::ptr::eq(*last, *action))
        })
        && action_tick_cadence_ok(&actions)
}

fn no_magic_autocast_controls(capture: &CombatCapture) -> bool {
    !capture.actions.iter().any(|action| {
        action["request"]["op"] == "side-tab"
            || (is_if_button(action)
                && [
                    AUTO_CHOOSER_COMPONENT,
                    AUTO_TOGGLE_COMPONENT,
                    AUTO_FIRE_BOLT_COMPONENT,
                    AUTO_FIRE_STRIKE_COMPONENT,
                ]
                .into_iter()
                .any(|component| action["request"]["component_id"] == json!(component)))
    })
}

fn magic_autocast_contract(capture: &CombatCapture, casts: &[RuneCast]) -> bool {
    let (Some(first_attack), Some(first_bolt), Some(last_bolt)) = (
        capture.actions.iter().find(|action| is_npc_attack(action)),
        casts.iter().find(|cast| cast.spell == MageSpell::FireBolt),
        casts.iter().rfind(|cast| cast.spell == MageSpell::FireBolt),
    ) else {
        return false;
    };
    let Some(first_attack_tick) = first_attack["tick"].as_i64() else {
        return false;
    };
    let Some(first_strike_tick) = casts
        .iter()
        .find(|cast| cast.spell == MageSpell::FireStrike)
        .map(|cast| cast.tick)
    else {
        return false;
    };
    let Some(baseline) = capture.start_baseline.as_ref() else {
        return false;
    };
    let Some(initial) = autocast_arm_actions(
        capture,
        AUTO_FIRE_BOLT_COMPONENT,
        None,
        first_attack_tick,
        true,
    ) else {
        return false;
    };
    let Some(rearm) = autocast_arm_actions(
        capture,
        AUTO_FIRE_STRIKE_COMPONENT,
        Some(last_bolt.tick),
        first_strike_tick,
        false,
    ) else {
        return false;
    };
    let count_button = |component| {
        capture
            .actions
            .iter()
            .filter(|action| {
                is_if_button(action) && action["request"]["component_id"] == json!(component)
            })
            .count()
    };
    let side_tabs = capture
        .actions
        .iter()
        .filter(|action| action["request"]["op"] == "side-tab")
        .collect::<Vec<_>>();
    let strike_toggle_tick = rearm.toggle["tick"].as_i64();
    first_attack["tick"]
        .as_i64()
        .is_some_and(|tick| tick > initial.toggle["tick"].as_i64().unwrap_or(i64::MAX))
        && first_attack_tick < first_bolt.tick
        && last_bolt.tick < rearm.chooser["tick"].as_i64().unwrap_or(i64::MIN)
        && strike_toggle_tick.is_some_and(|tick| tick < first_strike_tick)
        && baseline["magicvarp"] == json!(0)
        && side_tabs.len() == 1
        && side_tabs[0]["request"]["tab"] == json!(0)
        && count_button(AUTO_CHOOSER_COMPONENT) == 2
        && count_button(AUTO_TOGGLE_COMPONENT) == 2
        && count_button(AUTO_FIRE_BOLT_COMPONENT) == 1
        && count_button(AUTO_FIRE_STRIKE_COMPONENT) == 1
        && manual_magic_cast_actions(capture).is_empty()
        && capture
            .actions
            .iter()
            .filter(|action| is_npc_attack(action))
            .all(action_wire_valid)
        && initial.side_tab.is_some()
        && initial.selection["request"]["component_id"] == json!(AUTO_FIRE_BOLT_COMPONENT)
        && rearm.selection["request"]["component_id"] == json!(AUTO_FIRE_STRIKE_COMPONENT)
}

fn magic_input_after_report(capture: &CombatCapture, report: &Value) -> bool {
    let Some(report_tick) = integer(report, "combat_evidence_tick") else {
        return false;
    };
    !capture.actions.iter().any(|action| {
        (is_npc_attack(action) || action["request"]["op"] == "use-widget-on")
            && action["tick"]
                .as_i64()
                .is_some_and(|tick| tick > report_tick)
    })
}

fn magic_batch_contract(case: Case, capture: &CombatCapture) -> bool {
    let interactions = capture
        .actions
        .iter()
        .filter(|action| action["kind"] == "interaction")
        .collect::<Vec<_>>();
    let side_tabs = interactions
        .iter()
        .filter(|action| action["request"]["op"] == "side-tab")
        .collect::<Vec<_>>();
    let arm_components = [
        AUTO_CHOOSER_COMPONENT,
        AUTO_TOGGLE_COMPONENT,
        AUTO_FIRE_BOLT_COMPONENT,
        AUTO_FIRE_STRIKE_COMPONENT,
    ];
    let arm_inputs_are_solo = interactions.iter().all(|action| {
        let is_arm = action["request"]["op"] == "side-tab"
            || (is_if_button(action)
                && arm_components
                    .into_iter()
                    .any(|component| action["request"]["component_id"] == json!(component)));
        if !is_arm {
            return true;
        }
        let rows = plan_rows(capture, action);
        rows.len() == 1 && rows.last().is_some_and(|last| std::ptr::eq(*last, *action))
    });
    interactions.iter().all(|action| {
        action["batch"].as_u64().is_some_and(|batch| batch > 0) && action_wire_valid(action)
    }) && capture
        .actions
        .iter()
        .all(|action| action["kind"] != "other-request")
        && !interactions.is_empty()
        && batch_plan_contract(capture, &batch_plans(capture))
        && arm_inputs_are_solo
        && if case == Case::MageAuto {
            side_tabs.len() == 1
                && side_tabs[0]["request"]["tab"] == json!(0)
                && batch_plans(capture).iter().any(|plan| {
                    side_tabs[0]["batch"] == json!(plan.batch)
                        && plan.rows.len() == 1
                        && plan_event_count(plan) == Some(0)
                })
        } else {
            side_tabs.is_empty()
        }
}

fn magic_expected_end(case: Case) -> &'static str {
    if case == Case::MageManualNoFallback {
        "Aborted(Unprotected(NoRunes))"
    } else {
        "Killed"
    }
}

fn magic_ready(case: Case, capture: &CombatCapture) -> bool {
    let expected_end = magic_expected_end(case);
    let Some(report) = report_with_end(capture, expected_end) else {
        return false;
    };
    let rune_evidence = magic_rune_evidence(capture);
    let casts = &rune_evidence.casts;
    let rune_order_ok = rune_evidence.coherent
        && rune_cast_cadence_ok(casts)
        && twelve_bolts_then_strikes(casts, case != Case::MageManualNoFallback)
        && (case != Case::MageManualNoFallback
            || (casts.len() == 12 && rune_cast_count(casts, MageSpell::FireStrike) == 0));
    let cast_mode_ok = if case == Case::MageAuto {
        magic_autocast_contract(capture, casts)
    } else {
        !capture.actions.iter().any(is_npc_attack)
            && no_magic_autocast_controls(capture)
            && manual_magic_cast_contract(capture, casts)
    };
    let splashes = magic_splashes(capture, casts);
    let no_budget = !combat_outcomes(capture)
        .iter()
        .any(|fields| fields["combat_end"] == json!("Budget"));
    let killed = expected_end != "Killed"
        || (every_killed_report_has_corpse(capture)
            && prayer_off_plan_after_corpse(capture, report));
    rune_order_ok
        && rune_cast_count(casts, MageSpell::FireBolt) == 12
        && cast_mode_ok
        && (case != Case::MageAuto || !splashes.is_empty())
        && no_budget
        && killed
        && magic_protection_timing_ok(capture)
        && no_melee_offensive_prayers(capture)
        && (case != Case::MageAuto || magic_queue_contract(capture, casts))
        && magic_input_after_report(capture, report)
        && magic_batch_contract(case, capture)
        && native_interactions_wire_valid(capture)
        && (!case.is_manual_magic() || !capture.actions.iter().any(is_npc_attack))
        && (case != Case::MageAuto || has_real_attack_packet(capture))
        && !capture_has_death(capture)
}
fn magic_arm_receipt(capture: &CombatCapture, casts: &[RuneCast]) -> Value {
    let Some(first_attack) = capture.actions.iter().find(|action| is_npc_attack(action)) else {
        return Value::Null;
    };
    let Some(first_bolt) = casts.iter().find(|cast| cast.spell == MageSpell::FireBolt) else {
        return Value::Null;
    };
    let Some(last_bolt) = casts.iter().rfind(|cast| cast.spell == MageSpell::FireBolt) else {
        return Value::Null;
    };
    let Some(first_strike) = casts
        .iter()
        .find(|cast| cast.spell == MageSpell::FireStrike)
    else {
        return Value::Null;
    };
    let Some(first_attack_tick) = first_attack["tick"].as_i64() else {
        return Value::Null;
    };
    let Some(initial) = autocast_arm_actions(
        capture,
        AUTO_FIRE_BOLT_COMPONENT,
        None,
        first_attack_tick,
        true,
    ) else {
        return Value::Null;
    };
    let Some(rearm) = autocast_arm_actions(
        capture,
        AUTO_FIRE_STRIKE_COMPONENT,
        Some(last_bolt.tick),
        first_strike.tick,
        false,
    ) else {
        return Value::Null;
    };
    let arm = |side_tab: Option<&Value>,
               chooser: &Value,
               selection: &Value,
               toggle: &Value,
               spell: &str| {
        json!({
            "side_tab": side_tab,
            "chooser": chooser,
            "selection": selection,
            "toggle": toggle,
            "spell": spell,
            "settled_before_next_press": true,
        })
    };
    let initial_arm = arm(
        initial.side_tab,
        initial.chooser,
        initial.selection,
        initial.toggle,
        MageSpell::FireBolt.name(),
    );
    let rearm = arm(
        None,
        rearm.chooser,
        rearm.selection,
        rearm.toggle,
        MageSpell::FireStrike.name(),
    );
    json!({
        "initial": initial_arm,
        "initial_magicvarp": capture.start_baseline.as_ref().map(|frame| &frame["magicvarp"]),
        "rearm": rearm,
        "first_attack_tick": first_attack_tick,
        "first_bolt_tick": first_bolt.tick,
        "last_bolt_tick": last_bolt.tick,
        "first_strike_tick": first_strike.tick,
        "all_arm_inputs_are_single_row_exclusive_batches": magic_batch_contract(Case::MageAuto, capture),
    })
}

fn magic_receipt(capture: &CombatCapture, case: Case) -> Value {
    let evidence = magic_rune_evidence(capture);
    let casts = evidence
        .casts
        .iter()
        .map(|cast| {
            let (chaos, air, mind) = match cast.spell {
                MageSpell::FireBolt => (1, 3, 0),
                MageSpell::FireStrike => (0, 2, 1),
            };
            json!({
                "tick": cast.tick,
                "spell": cast.spell.name(),
                "runes_decremented": {"chaos": chaos, "air": air, "mind": mind},
                "counted_from_observed_rune_consumption": true,
            })
        })
        .collect::<Vec<_>>();
    let splashes = magic_splashes(capture, &evidence.casts);
    let input_distances = magic_input_distances(capture);
    let cast_distances = magic_cast_distances(capture, &evidence.casts);
    let queue_distances = if case == Case::MageAuto {
        &cast_distances
    } else {
        &input_distances
    };
    let distinct_queue_distances = queue_distances
        .iter()
        .copied()
        .filter(|distance| *distance >= 2)
        .collect::<std::collections::BTreeSet<_>>();
    let expected_end = magic_expected_end(case);
    let batch_events = batch_plans(capture)
        .iter()
        .filter_map(plan_event_count)
        .collect::<Vec<_>>();
    let manual_casts = manual_magic_cast_actions(capture)
        .into_iter()
        .map(|action| {
            json!({
                "tick": action["tick"],
                "request": action["request"],
                "wire_opcodes": action["wire_opcodes"],
                "warlord_distance": action["snapshot"]["nearby_npcs"]
                    .as_array()
                    .and_then(|npcs| npcs.iter().find(|npc| npc["type"] == json!(477)))
                    .and_then(|npc| npc["distance"].as_i64()),
            })
        })
        .collect::<Vec<_>>();
    let manual_casts_valid =
        !manual_casts.is_empty() && manual_magic_cast_contract(capture, &evidence.casts);
    let launch_distances_ok = distinct_queue_distances.len() >= 2
        && distinct_queue_distances
            .iter()
            .any(|distance| *distance >= 3);
    let observed_end_reports = combat_outcomes(capture);
    let budget_invalid = observed_end_reports
        .iter()
        .any(|fields| fields["combat_end"] == json!("Budget"));
    json!({
        "style": "Mage",
        "scenario": case.label(),
        "request": {
            "spells": if case == Case::MageAuto { Value::Null } else {
                json!(["fire_bolt"])
            },
            "fallback_spells": case.is_magic_fallback(),
        },
        "seed": {
            "magic": 35,
            "defence": 40,
            "hitpoints": 40,
            "prayer": 43,
            "staff": "staff_of_fire",
            "chaos_runes": 12,
            "air_runes": 150,
            "mind_runes": MAGIC_MIND_RUNES,
            "prayer_restores_4dose": MAGIC_PRAYER_RESTORES,
        },
        "seed_rationale": {
            "approved_resource_changes": [
                {"resource": "air_runes", "previous": 40, "current": MAGIC_AIR_RUNES},
                {"resource": "mind_runes", "previous": 30, "current": MAGIC_MIND_RUNES},
                {"resource": "prayer_restores_4dose", "previous": 0, "current": MAGIC_PRAYER_RESTORES}
            ],
            "scope": "same fixed changes in G1, G2 fallback=true, and G2 fallback=false; other stats, Chaos12, staff, and deadline unchanged",
            "previous_seed_maximum_damage": {
                "fire_bolt": {"casts": 12, "maximum_per_cast": 12, "total": 144},
                "fire_strike": {"available_air_after_bolts": 4, "casts": 2, "maximum_per_cast": 8, "total": 16},
                "combined": 160,
                "warlord_hitpoints": 170,
                "shortfall": 10,
                "conclusion": "airrune40 cannot kill the 170-HP Warlord even at maximum damage; measured 12/150/30 seed then exhausted prayer and still left HP24, so the approved reserve corrects stock and drain without policy or RNG changes",
            },
        },
        "local_npc_setup": &capture.magic_setup,
        "rune_evidence_coherent": evidence.coherent,
        "casts": casts,
        "fire_bolt_count": rune_cast_count(&evidence.casts, MageSpell::FireBolt),
        "fire_strike_count": rune_cast_count(&evidence.casts, MageSpell::FireStrike),
        "cast_cadence_at_least_5_ticks": rune_cast_cadence_ok(&evidence.casts),
        "cast_order": twelve_bolts_then_strikes(&evidence.casts, case != Case::MageManualNoFallback),
        "queue_input_distances": input_distances,
        "rune_cast_distances": cast_distances,
        "queue_evidence_distances": queue_distances,
        "distinct_queue_distances_at_least_2": &distinct_queue_distances,
        "observed_launch_distance_coverage": launch_distances_ok,
        "splash_animation_id": FAILED_SPELL_SPLASH,
        "splash_and_two_distance_numeric_queue_required": case == Case::MageAuto,
        "measured_projectile_queues": magic_projectile_queues(capture, &evidence.casts),
        "raw_npc_mask_and_landing_events": &capture.magic_npc_events,
        "splash_casts_with_unchanged_health": splashes,
        "protect_onset_tick": first_engaged_warlord_attack_onset(capture),
        "protect_timing": protection_timing_receipt(capture),
        "protect_restoring_terminal": protect_plan_ends_with_terminal(capture),
        "no_melee_offensive_prayers": no_melee_offensive_prayers(capture),
        "manual_casts": manual_casts,
        "manual_casts_use_only_widget_on_npc": manual_casts_valid,
        "no_attack_actions_in_manual_mode": !case.is_manual_magic()
            || !capture.actions.iter().any(is_npc_attack),
        "autocast_arm": if case == Case::MageAuto {
            magic_arm_receipt(capture, &evidence.casts)
        } else {
            Value::Null
        },
        "exclusive_batches_max_wire_events": batch_events.iter().max(),
        "exclusive_batches_all_within_5_wire_events": batch_events.iter().all(|events| *events <= 5),
        "expected_end": expected_end,
        "observed_end_reports": observed_end_reports,
        "budget_is_invalid": budget_invalid,
        "required_outcome_observed": report_with_end(capture, expected_end).is_some(),
        "proof_ready": magic_ready(case, capture),
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
            start.iter().any(|row| {
                chat_continue_debug(row["request"]["debug"].as_str().unwrap_or_default())
            }) && start
                .iter()
                .any(|row| row["request"]["debug"] == "Answer { option: 1 }")
                && hand_in.iter().any(|row| {
                    chat_continue_debug(row["request"]["debug"].as_str().unwrap_or_default())
                })
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
    if case.is_magic() || case.is_ranged() {
        if report_with_end(capture, "Budget").is_some() {
            return Some(format!(
                "{} INVALID: Combat ended on Budget rather than the required outcome",
                case.key()
            ));
        }
        let expected_end = if case == Case::R2 {
            "Aborted(PrepFailed(Ammo))"
        } else if case.is_ranged() {
            "Killed"
        } else {
            magic_expected_end(case)
        };
        if let Some(report) = report_with_end(capture, expected_end) {
            let report_tick = integer(report, "combat_evidence_tick");
            let last_tick = capture
                .frames
                .last()
                .and_then(|frame| frame["tick"].as_i64());
            // G2 has no incidental splash gate; G1 alone owns P5.
            if case == Case::MageAuto
                && report_tick
                    .zip(last_tick)
                    .is_some_and(|(report_tick, last_tick)| last_tick >= report_tick + 30)
                && magic_splashes(capture, &magic_rune_evidence(capture).casts).is_empty()
            {
                return Some(format!(
                    "{} INVALID: required fresh spot-animation-{FAILED_SPELL_SPLASH} splash with unchanged Warlord health was not observed; no reroll",
                    case.key()
                ));
            }
            // A missing segment or actor cannot establish a no-onset case.
            // Unlike observed unsafe behavior, incomplete evidence is INVALID,
            // never an implicit not-applicable timing result.
            if first_engaged_warlord_attack_onset(capture).is_none()
                && no_onset_evidence(capture) == NoOnsetEvidence::MissingData
            {
                return Some(format!(
                    "{} INVALID: incomplete engaged-NPC no-onset protection evidence",
                    case.key()
                ));
            }
        }
    }
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
    let begin_tick = integer(report, "combat_ticks")
        .filter(|ticks| *ticks >= 0)
        .map_or(evidence_tick, |ticks| evidence_tick.saturating_sub(ticks));
    // The first exact HP0 bar is the latched end; the latest bar still has to
    // remain that corpse. A later Ready status is bounded by WindDown's lock
    // end + 2, with one publication tick because the retained G2 publishes
    // Ready one tick after its final prayer-off observation.
    let latch_tick = capture.frames.iter().find_map(|frame| {
        let tick = frame["tick"].as_i64()?;
        if tick < begin_tick || tick > evidence_tick {
            return None;
        }
        frame["nearby_npcs"]
            .as_array()?
            .iter()
            .find(|npc| {
                npc["index"].as_i64() == Some(index)
                    && npc["health"] == json!(0)
                    && npc["total_health"].as_i64().is_some_and(|total| total > 0)
                    && engaged_type.is_none_or(|kind| npc["type"].as_i64() == Some(kind))
            })
            .map(|_| tick)
    });
    let Some(latch_tick) = latch_tick else {
        return false;
    };
    let latest_bar = capture
        .frames
        .iter()
        .rev()
        .filter(|frame| {
            frame["tick"]
                .as_i64()
                .is_some_and(|tick| begin_tick <= tick && tick <= evidence_tick)
        })
        .find_map(|frame| {
            frame["nearby_npcs"]
                .as_array()?
                .iter()
                .find(|npc| npc["index"].as_i64() == Some(index))
                .map(|npc| (frame, npc))
        });
    let Some((_, npc)) = latest_bar else {
        return false;
    };
    let report_deadline = winddown_lock_end(capture, latch_tick).saturating_add(3);
    evidence_tick <= report_deadline
        && npc["health"] == json!(0)
        && npc["total_health"].as_i64().is_some_and(|total| total > 0)
        && engaged_type.is_none_or(|kind| npc["type"].as_i64() == Some(kind))
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

fn chat_continue_debug(debug: &str) -> bool {
    // Frozen receipts used the unit variant before explicit pause targets existed.
    matches!(
        debug,
        "ContinueDialog" | "ContinueDialog { component_id: None }"
    )
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
        Some("side-tab") => actual.is_empty() && action["request"]["tab"].as_i64() == Some(0),
        Some("use-widget-on") => {
            let component_id = action["request"]["component_id"].as_i64();
            let target_index = action["request"]["index"].as_i64();
            let target_name = action["request"]["target_name"]
                .as_str()
                .is_some_and(|name| name.eq_ignore_ascii_case("khazard warlord"));
            component_id.is_some_and(|component_id| {
                [FIRE_BOLT_WIDGET, FIRE_STRIKE_WIDGET].contains(&component_id)
            }) && action["request"]["kind"] == json!("npc")
                && target_name
                && target_index.is_some_and(|index| {
                    action["snapshot"]["nearby_npcs"]
                        .as_array()
                        .is_some_and(|npcs| {
                            npcs.iter().any(|npc| {
                                npc["index"].as_i64() == Some(index)
                                    && npc["type"] == json!(477)
                                    && npc["name"].as_str().is_some_and(|name| {
                                        name.eq_ignore_ascii_case("khazard warlord")
                                    })
                            })
                        })
                })
                && targeted(&actual, opcode(ClientProt289::OPNPCT))
        }
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
            if chat_continue_debug(debug)
                || debug
                    .strip_prefix("ContinueDialog { component_id: Some(")
                    .and_then(|value| value.strip_suffix(") }"))
                    .and_then(|value| value.parse::<i32>().ok())
                    .is_some_and(|component_id| component_id >= 0)
            {
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
            && plan_event_count(plan).is_some_and(|events| {
                (1..=5).contains(&events)
                    || (events == 0
                        && plan.rows.len() == 1
                        && plan.rows[0]["request"]["op"] == json!("side-tab"))
            })
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
        let npcs = frame["nearby_npcs"].as_array()?;
        let mut live = false;
        for npc in npcs
            .iter()
            .filter(|npc| npc["type"] == json!(WARLORD_NPC_ID))
        {
            live |= npc_live_attack_evidence(npc)?;
        }
        live.then(|| frame["tick"].as_i64()).flatten()
    })
}

fn first_warlord_attack_onset(capture: &CombatCapture) -> Option<i64> {
    let mut previous_animations = std::collections::HashMap::<i64, Option<i64>>::new();
    for frame in &capture.frames {
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
            if npc["attack_animation"].as_bool()? && npc_live_attack_evidence(npc)?
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

fn protect_action_is_in_fight(capture: &CombatCapture, protect_index: usize) -> bool {
    let Some(protect_tick) = capture.actions[protect_index]["tick"].as_i64() else {
        return false;
    };
    capture.actions.iter().take(protect_index).any(|engage| {
        let request = &engage["request"];
        accepted(engage)
            && engage["tick"]
                .as_i64()
                .is_some_and(|engage_tick| engage_tick < protect_tick)
            && (is_npc_attack(engage)
                || (request["op"] == json!("use-widget-on") && request["kind"] == json!("npc")))
    })
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
        .enumerate()
        .filter(|(_, action)| prayer_action(action, component))
        .all(
            |(index, action)| match prayer_varp(&action["snapshot"], varp) {
                Some(1) => true, // WindDown deactivation owes no restoring attack.
                Some(0) if !protect_action_is_in_fight(capture, index) => true,
                // Only Fight rows create the restore obligation. Prep and Engage
                // may turn Protect on while wearing kit or opening the encounter.
                Some(0) => plan_rows(capture, action).last().is_some_and(|last| {
                    is_npc_attack(last)
                        || is_drink(last)
                        || (last["request"]["op"] == "use-widget-on"
                            && last["request"]["kind"] == "npc")
                }),
                _ => false,
            },
        )
}

// Magic and ranged cells share the same fail-closed N1 and protection oracles.
const MELEE_OFFENSIVE_PRAYER_COMPONENTS: [i64; 2] = [5619, 5620];
const MELEE_OFFENSIVE_PRAYER_VARPS: [i64; 2] = [93, 94];

// The report defines `[begin, report]` as `combat_evidence_tick - combat_ticks`
// through the evidence tick, inclusive; every tick in that interval is required.

#[derive(Clone, Copy)]
struct MagicEngagementWindow {
    begin_tick: i64,
    report_tick: i64,
    engaged_index: i64,
    engaged_type: i64,
}

fn magic_engagement_window(capture: &CombatCapture) -> Option<MagicEngagementWindow> {
    let (report, engaged_index) = capture.statuses.iter().rev().find_map(|status| {
        let end = status["fields"]["combat_end"].as_str()?;
        if !matches!(
            end,
            "Killed" | "Aborted(Unprotected(NoRunes))" | "Aborted(PrepFailed(Ammo))"
        ) {
            return None;
        }
        let index = if status["fields"]["combat_engaged_kind"] == json!("Npc")
            && integer(status, "combat_engaged_npc_type") == Some(477)
        {
            integer(status, "combat_engaged_index")?
        } else if end == "Aborted(PrepFailed(Ammo))"
            && ranged_selected_event(capture)?["case"] == "R2"
            && ranged_spawn_event(capture)?["npc_type"] == 477
        {
            // Wrong ammunition aborts before engagement. The natural fixture
            // still identifies the exact NPC whose complete window must prove
            // safety; absent frames/identity never become no-onset evidence.
            ranged_spawned_index(capture)?
        } else {
            return None;
        };
        Some((status, index))
    })?;
    let report_tick = integer(report, "combat_evidence_tick")?;
    let combat_ticks = integer(report, "combat_ticks").filter(|ticks| *ticks >= 0)?;
    Some(MagicEngagementWindow {
        begin_tick: report_tick.checked_sub(combat_ticks)?,
        report_tick,
        engaged_index,
        engaged_type: 477,
    })
}

fn engagement_frames_contiguous(capture: &CombatCapture, window: MagicEngagementWindow) -> bool {
    let Some(after_report) = window.report_tick.checked_add(1) else {
        return false;
    };
    let mut next_tick = window.begin_tick;
    let mut found = false;
    for frame in &capture.frames {
        let Some(tick) = frame["tick"].as_i64() else {
            return false;
        };
        if !(window.begin_tick..=window.report_tick).contains(&tick) {
            continue;
        }
        // The harness can capture multiple outputs during one host tick.
        // Keep every row for the evidence checks, but require no skipped tick.
        if found && tick == next_tick - 1 {
            continue;
        }
        if tick != next_tick {
            return false;
        }
        found = true;
        let Some(next) = next_tick.checked_add(1) else {
            return false;
        };
        next_tick = next;
    }
    found && next_tick == after_report
}

fn first_engaged_warlord_attack_onset(capture: &CombatCapture) -> Option<i64> {
    let window = magic_engagement_window(capture)?;
    let mut previous_animation = None;
    for frame in &capture.frames {
        let tick = frame["tick"].as_i64()?;
        if !(window.begin_tick..=window.report_tick).contains(&tick) {
            continue;
        }
        let npc = frame["nearby_npcs"].as_array()?.iter().find(|npc| {
            npc["index"].as_i64() == Some(window.engaged_index)
                && npc["type"].as_i64() == Some(window.engaged_type)
        })?;
        let animation = npc["animation"].as_i64();
        let previous = previous_animation;
        previous_animation = animation;
        if animation == Some(401)
            && animation != previous
            && npc["attack_animation"].as_bool()?
            && npc_live_attack_evidence(npc)?
        {
            return Some(tick);
        }
    }
    None
}

// Only complete, safe evidence can prove no onset. Missing rows or facts map to
// INVALID; adjacency, HP loss, or an attack animation targeting us refuses N/A.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum NoOnsetEvidence {
    Proven,
    MissingData,
    Unsafe,
}
fn network_footprint_distance(player: WorldTile, npc: WorldTile, size: i64) -> Option<i32> {
    if player.level != npc.level {
        return None;
    }
    let size = i32::try_from(size).ok()?.max(1);
    let max_x = npc.x.checked_add(size - 1)?;
    let max_z = npc.z.checked_add(size - 1)?;
    let closest_x = player.x.clamp(npc.x, max_x);
    let closest_z = player.z.clamp(npc.z, max_z);
    Some(
        (player.x - closest_x)
            .abs()
            .max((player.z - closest_z).abs()),
    )
}

fn no_onset_evidence(capture: &CombatCapture) -> NoOnsetEvidence {
    let Some(window) = magic_engagement_window(capture) else {
        return NoOnsetEvidence::MissingData;
    };
    if !engagement_frames_contiguous(capture, window) {
        return NoOnsetEvidence::MissingData;
    }

    let mut previous_hp = None;
    let mut found = false;
    for frame in &capture.frames {
        let Some(tick) = frame["tick"].as_i64() else {
            return NoOnsetEvidence::MissingData;
        };
        if !(window.begin_tick..=window.report_tick).contains(&tick) {
            continue;
        }
        let Some(npcs) = frame["nearby_npcs"].as_array() else {
            return NoOnsetEvidence::MissingData;
        };
        let Some(npc) = npcs.iter().find(|npc| {
            npc["index"].as_i64() == Some(window.engaged_index)
                && npc["type"].as_i64() == Some(window.engaged_type)
        }) else {
            return NoOnsetEvidence::MissingData;
        };
        let Some(player_tile) = value_tile(&frame["local_player"]["network"]) else {
            return NoOnsetEvidence::MissingData;
        };
        let Some(npc_tile) = value_tile(&npc["network"]) else {
            return NoOnsetEvidence::MissingData;
        };
        let Some(size) = npc["size"].as_i64() else {
            return NoOnsetEvidence::MissingData;
        };
        let Some(distance) = network_footprint_distance(player_tile, npc_tile, size) else {
            return NoOnsetEvidence::MissingData;
        };
        if distance <= 1 {
            return NoOnsetEvidence::Unsafe;
        }
        let Some(hitpoints) = stat_effective(frame, "hitpoints") else {
            return NoOnsetEvidence::MissingData;
        };
        if previous_hp.is_some_and(|previous| hitpoints < previous) {
            return NoOnsetEvidence::Unsafe;
        }
        previous_hp = Some(hitpoints);

        match npc_live_attack_evidence(npc) {
            Some(true) => return NoOnsetEvidence::Unsafe,
            Some(false) => {}
            None => return NoOnsetEvidence::MissingData,
        }
        found = true;
    }
    if found {
        NoOnsetEvidence::Proven
    } else {
        NoOnsetEvidence::MissingData
    }
}

fn protection_timing_receipt(capture: &CombatCapture) -> Value {
    if first_engaged_warlord_attack_onset(capture).is_some() {
        json!(protection_timing_ok(capture))
    } else if no_onset_evidence(capture) == NoOnsetEvidence::Proven {
        json!("not_applicable_no_onset")
    } else {
        json!(false)
    }
}

// P4 remains strict when an onset exists; positive no-onset is not proof.
// Only Fight-emitted Protect activations require a restoring terminal.

fn magic_protection_timing_ok(capture: &CombatCapture) -> bool {
    let timing = protection_timing_receipt(capture);
    (timing == json!(true) || timing == json!("not_applicable_no_onset"))
        && protect_plan_ends_with_terminal(capture)
}

// N1: only Protect from Melee clicks are allowed; 5619/5620 and other known
// prayer components are forbidden, and varps 93/94 stay zero while engaged.

fn no_melee_offensive_prayers(capture: &CombatCapture) -> bool {
    let Some(window) = magic_engagement_window(capture) else {
        return false;
    };
    let Some(protect_component) = prayer_component(capture, "Protect from Melee") else {
        return false;
    };
    if !engagement_frames_contiguous(capture, window)
        || capture.actions.iter().any(|action| {
            if !accepted(action) || action["request"]["op"] != json!("if-button") {
                return false;
            }
            let Some(component) = action["request"]["component_id"].as_i64() else {
                return true;
            };
            MELEE_OFFENSIVE_PRAYER_COMPONENTS.contains(&component)
                || (component != protect_component
                    && capture
                        .prayer_facts
                        .iter()
                        .any(|fact| fact["button_com"].as_i64() == Some(component)))
        })
    {
        return false;
    }
    let mut found = false;
    for frame in &capture.frames {
        let Some(tick) = frame["tick"].as_i64() else {
            return false;
        };
        if !(window.begin_tick..=window.report_tick).contains(&tick) {
            continue;
        }
        if MELEE_OFFENSIVE_PRAYER_VARPS
            .iter()
            .any(|varp| prayer_varp(frame, *varp) != Some(0))
        {
            return false;
        }
        found = true;
    }
    found
}

// S3 §5.1 G1 P4: Protect from Melee is on by the first onset + 2.
// Earlier activation is valid, but a request without its observed echo is not.
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

fn winddown_lock_end(capture: &CombatCapture, latch_tick: i64) -> i64 {
    capture
        .actions
        .iter()
        .filter_map(|action| {
            if !accepted(action) {
                return None;
            }
            let tick = action["tick"].as_i64()?;
            if tick > latch_tick {
                return None;
            }
            let end = if is_drink(action) {
                tick.saturating_add(3)
            } else if is_eat(action) && held_item_id(action) == Some(i64::from(COOKED_KARAMBWAN_ID))
            {
                tick.saturating_add(4)
            } else {
                return None;
            };
            (end > latch_tick).then_some(end)
        })
        .max()
        .unwrap_or(latch_tick)
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
    let lock_end = winddown_lock_end(capture, corpse_tick);
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
        ranged_runtime_fixture(&source, placement)
    } else {
        source
    };
    let path = script::quester::compile::prepare_for_test({
        let selected = Arc::clone(&selected);
        let quests = Arc::clone(&quests);
        move |cap| compile_path(&compile_source, &selected, &quests, cap)
    })
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
        magic_tele_tile: case.is_magic().then_some(WorldTile {
            x: stand.x + 5,
            z: stand.z,
            level: stand.level,
        }),
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
        ranged_start_first_missing_tick: None,
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
            combat_proof::record_status(&account, &status, None);
        }
        if case.is_ranged() {
            let target_index = {
                let proof = capture.lock().unwrap_or_else(|e| e.into_inner());
                ranged_spawned_index(&proof)
            }
            .and_then(|index| usize::try_from(index).ok());
            if let Some(target_index) = target_index {
                let other_player = {
                    let live = state.lock().unwrap_or_else(|e| e.into_inner());
                    ranged_snapshot_other_player(&live.snapshot, target_index)
                };
                if let Some(player_index) = other_player {
                    {
                        let mut proof = capture.lock().unwrap_or_else(|e| e.into_inner());
                        if let Some(event) = ranged_spawn_event_mut(&mut proof) {
                            event["interference"] = json!({"player_index": player_index});
                        }
                    }
                    let reason = ranged_interference_reason(case, target_index, player_index);
                    combat_proof::mark_invalid(&account, reason.clone());
                    terminal_error = Some(reason);
                    writer.outcome = "INVALID".to_owned();
                    break;
                }
            }
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
        let ranged_abort_horizon_end = if case == Case::R2 {
            report_with_end(&capture_snapshot, "Aborted(PrepFailed(Ammo))")
                .and_then(|report| integer(report, "combat_evidence_tick"))
                .and_then(|tick| tick.checked_add(RANGED_ABORT_SURVIVAL_TICKS))
        } else {
            None
        };
        let last_capture_tick = capture_snapshot
            .frames
            .last()
            .and_then(|frame| frame["tick"].as_i64());
        let blocked_after_r2_horizon = case == Case::R2
            && ranged_abort_blocked_after_horizon(
                &capture_snapshot,
                last_capture_tick,
                ranged_abort_horizon_end,
            );
        drop(capture_snapshot);
        if blocked_after_r2_horizon {
            writer.scenario_status = "Blocked after R2 post-abort horizon".to_owned();
            break;
        }
        let state = state.lock().unwrap_or_else(|e| e.into_inner());
        match state.runner.status() {
            RunnerStatus::Passed => {
                writer.scenario_status = "Passed".to_owned();
                break;
            }
            RunnerStatus::Failed(error) => {
                if !writer.scenario_status.starts_with("Failed(") {
                    writer.scenario_status = format!("Failed({error})");
                }
                terminal_error.get_or_insert_with(|| format!("ScenarioRunner failed: {error}"));
                if case == Case::R2 && ranged_abort_horizon_end.is_some() {
                    // The blocked Quester ends the scenario step, not the
                    // observer: keep sampling until the post-abort horizon.
                    if ranged_abort_horizon_end.is_some_and(|horizon_end| {
                        last_capture_tick.is_some_and(|tick| tick >= horizon_end)
                    }) {
                        break;
                    }
                    // Leave RUNNING so the final receipt, not Blocked alone,
                    // decides whether this expected abort is accepted.
                } else {
                    writer.outcome = "FAIL".to_owned();
                    break;
                }
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
#[ignore = "requires LIVE=1 and the shared local R289 engine"]
fn live_combat_s3c_g1_magic_autocast() {
    run_case(Case::MageAuto);
}

#[test]
#[ignore = "requires LIVE=1 and the shared local R289 engine"]
fn live_combat_s3c_g2_manual_spell_fallback() {
    run_case(Case::MageManualFallback);
}

#[test]
#[ignore = "requires LIVE=1 and the shared local R289 engine"]
fn live_combat_s3c_g2_manual_no_fallback_aborts_unprotected() {
    run_case(Case::MageManualNoFallback);
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
    assert!(
        protect_plan_ends_with_terminal(&capture),
        "without a Fight action, a Prep protect row has no restore obligation"
    );
    capture.actions.remove(0);
    assert!(protect_plan_ends_with_terminal(&capture));
    capture.actions[0]["snapshot"]["prayer_varps"][0]["value"] = Value::Null;
    assert!(!protect_plan_ends_with_terminal(&capture));
}

#[test]
fn preemptive_manual_protect_rejects_never_on_and_after_hit_activation() {
    let mut capture = CombatCapture::default();
    capture.prayer_facts.push(json!({
        "name": "Protect from Melee", "button_com": 5623, "varp": 97
    }));
    capture.actions = vec![
        json!({
            "kind": "interaction", "tick": 8, "batch": 1, "accepted": true,
            "request": {"op": "if-button", "component_id": 5623},
            "snapshot": {"prayer_varps": [{"index": 97, "value": 0}]}
        }),
        json!({
            "kind": "interaction", "tick": 8, "batch": 1, "accepted": true,
            "request": {"op": "use-widget-on", "kind": "npc", "index": 7}
        }),
    ];
    let frame = |tick, on, hp| {
        json!({
            "tick": tick, "self_slot": 1,
            "stats": [{"name": "hitpoints", "base": 40, "effective": hp}],
            "prayer_varps": [{"index": 97, "value": on}],
            "nearby_npcs": [{
                "name": "Khazard warlord", "index": 7, "animation": 401,
                "in_combat": false,
                "targets_local": true, "attack_animation": true,
                "attack_spot": false, "local_hitmark_recent": false,
                "target": {"kind": "Player", "index": 1}
            }]
        })
    };
    capture.frames = vec![frame(10, 1, 40), frame(12, 1, 40)];
    assert!(
        protection_timing_ok(&capture),
        "preemptive manual Cast is valid"
    );
    capture.frames = vec![frame(10, 0, 40), frame(12, 0, 31)];
    assert!(!protection_timing_ok(&capture), "protect never turned on");
    capture.actions[0]["tick"] = json!(13);
    capture.actions[1]["tick"] = json!(13);
    capture.frames.push(frame(13, 1, 31));
    assert!(
        !protection_timing_ok(&capture),
        "activation after the hit at12 is late"
    );
}

#[test]
fn latched_corpse_survives_winddown_but_rejects_every_missing_identity_case() {
    let report = json!({"fields": {
        "combat_end": "Killed", "combat_evidence_tick": 14, "combat_ticks": 10,
        "combat_engaged_index": 7, "combat_engaged_npc_type": 477
    }});
    let corpse = |tick, index, type_, hp| {
        json!({
            "tick": tick,
            "nearby_npcs": [{"index": index, "type": type_, "health": hp, "total_health": 170}]
        })
    };
    let mut capture = CombatCapture::default();
    capture.frames = vec![
        corpse(11, 7, 477, 0),
        json!({"tick": 14, "nearby_npcs": []}),
    ];
    assert!(
        has_corpse(&capture, &report),
        "latched corpse precedes cleanup completion"
    );
    capture.frames.insert(1, corpse(12, 7, 477, 1));
    assert!(!has_corpse(&capture, &report), "revived NPC");
    capture.frames = vec![corpse(11, 7, 478, 0)];
    assert!(!has_corpse(&capture, &report), "wrong NPC type");
    capture.frames = vec![corpse(11, 8, 477, 0)];
    assert!(!has_corpse(&capture, &report), "wrong NPC index");
    capture.frames = vec![corpse(3, 7, 477, 0)];
    assert!(!has_corpse(&capture, &report), "bar before engagement");
    capture.frames = vec![corpse(15, 7, 477, 0)];
    assert!(!has_corpse(&capture, &report), "bar after report");
    capture.frames = vec![corpse(11, 7, 477, 1)];
    assert!(!has_corpse(&capture, &report), "no HP0 bar");
}

#[test]
fn latched_corpse_report_is_bounded_by_winddown_and_publication() {
    let report = |evidence_tick, combat_ticks| {
        json!({"fields": {
            "combat_end": "Killed", "combat_evidence_tick": evidence_tick,
            "combat_ticks": combat_ticks, "combat_engaged_index": 7,
            "combat_engaged_npc_type": 477
        }})
    };
    let corpse = |tick| {
        json!({
            "tick": tick,
            "nearby_npcs": [{"index": 7, "type": 477, "health": 0, "total_health": 170}]
        })
    };
    let mut capture = CombatCapture::default();
    capture.frames = vec![
        corpse(261),
        corpse(263),
        json!({"tick": 264, "nearby_npcs": []}),
    ];
    capture.actions.push(json!({
        "kind": "interaction", "tick": 261, "batch": 1, "accepted": true,
        "request": {"op": "held", "action": "Drink"}
    }));
    assert!(
        has_corpse(&capture, &report(267, 215)),
        "allow one publication tick after lock_end + 2"
    );

    capture.actions.clear();
    capture.actions.push(json!({
        "kind": "interaction", "tick": 320, "batch": 2, "accepted": true,
        "request": {"op": "held", "action": "Drink"}
    }));
    assert!(
        !has_corpse(&capture, &report(324, 272)),
        "a future drink cannot extend WindDown for a late Killed report"
    );
}

#[test]
fn manual_protect_terminal_rejects_non_npc_widget_and_protect_alone() {
    let mut capture = CombatCapture::default();
    capture.prayer_facts.push(json!({
        "name": "Protect from Melee", "button_com": 5623, "varp": 97,
    }));
    capture.actions = vec![
        json!({
            "kind": "interaction", "tick": 7, "batch": 1, "accepted": true,
            "request": {"op": "npc", "name": "Khazard Warlord", "index": 7, "action": "Attack"}
        }),
        json!({
            "kind": "interaction", "tick": 8, "batch": 2, "accepted": true,
            "request": {"op": "if-button", "component_id": 5623},
            "snapshot": {
                "tick": 8, "self_slot": 1,
                "prayer_varps": [{"index": 97, "value": 0}],
                "nearby_npcs": [{
                    "index": 7, "type": 477, "in_combat": true,
                    "target": {"kind": "Player", "index": 1}
                }]
            }
        }),
        json!({
            "kind": "interaction", "tick": 8, "batch": 2,
            "request": {"op": "use-widget-on", "kind": "obj", "index": 9}
        }),
    ];
    assert!(
        !protect_plan_ends_with_terminal(&capture),
        "a manual Protect plus a non-NPC widget-on is not a restoring Cast"
    );
    capture.actions.pop();
    assert!(
        !protect_plan_ends_with_terminal(&capture),
        "a manual capture with Protect alone has no restoring terminal"
    );
}
#[test]
fn ranged_prep_protect_with_two_wears_is_legal_but_late_protect_is_not() {
    let mut prep = magic_protection_capture(50, 110, 62);
    prep.actions = vec![
        json!({
            "kind": "interaction", "sequence": 1, "tick": 62, "batch": 1,
            "request": {"op": "if-button", "component_id": 5623},
            "snapshot": {"prayer_varps": [{"index": 97, "value": 0}]}
        }),
        json!({
            "kind": "interaction", "sequence": 2, "tick": 62, "batch": 1,
            "request": {"op": "wear", "name": "Maple shortbow"}
        }),
        json!({
            "kind": "interaction", "sequence": 3, "tick": 62, "batch": 1,
            "request": {"op": "wear", "name": "Steel arrow"}
        }),
        json!({
            "kind": "interaction", "sequence": 4, "tick": 63, "batch": 2,
            "request": {"op": "npc", "name": "Khazard Warlord", "action": "Attack"}
        }),
    ];
    assert!(protect_plan_ends_with_terminal(&prep));
    assert!(
        magic_protection_timing_ok(&prep),
        "Prep [protect, wear, wear] has no Fight restore obligation"
    );

    let mut late = magic_protection_capture(50, 110, 62);
    late.actions = vec![
        json!({
            "kind": "interaction", "sequence": 1, "tick": 60, "batch": 1, "accepted": true,
            "request": {"op": "npc", "name": "Khazard Warlord", "index": 7, "action": "Attack"}
        }),
        json!({
            "kind": "interaction", "sequence": 2, "tick": 62, "batch": 2,
            "request": {"op": "if-button", "component_id": 5623},
            "snapshot": {"prayer_varps": [{"index": 97, "value": 0}]}
        }),
    ];
    set_magic_warlord_field(&mut late, 58, "animation", json!(401));
    for frame in &mut late.frames {
        frame["nearby_npcs"][0]["in_combat"] = json!(false);
    }
    assert!(!protect_plan_ends_with_terminal(&late));
    assert_eq!(protection_timing_receipt(&late), json!(false));
}

#[test]
fn no_onset_distance_uses_network_tiles_not_rendered_actor_tiles() {
    let mut capture = magic_protection_capture(50, 110, 62);
    for frame in &mut capture.frames {
        frame["nearby_npcs"][0]["tile"]["x"] = json!(101);
        frame["nearby_npcs"][0]["distance"] = json!(1);
        frame["nearby_npcs"][0]["network"]["x"] = json!(107);
    }
    assert_eq!(no_onset_evidence(&capture), NoOnsetEvidence::Proven);

    for frame in &mut capture.frames {
        frame["nearby_npcs"][0]["tile"]["x"] = json!(107);
        frame["nearby_npcs"][0]["distance"] = json!(7);
        frame["nearby_npcs"][0]["network"]["x"] = json!(101);
    }
    assert_eq!(no_onset_evidence(&capture), NoOnsetEvidence::Unsafe);
    let mut footprint = magic_protection_capture(50, 110, 62);
    for frame in &mut footprint.frames {
        frame["nearby_npcs"][0]["tile"]["x"] = json!(107);
        frame["nearby_npcs"][0]["distance"] = json!(7);
        frame["nearby_npcs"][0]["network"]["x"] = json!(98);
        frame["nearby_npcs"][0]["size"] = json!(3);
    }
    assert_eq!(no_onset_evidence(&footprint), NoOnsetEvidence::Unsafe);
    footprint.frames[0]["nearby_npcs"][0]
        .as_object_mut()
        .unwrap()
        .remove("size");
    assert_eq!(no_onset_evidence(&footprint), NoOnsetEvidence::MissingData);

    capture.frames[0]["nearby_npcs"][0]
        .as_object_mut()
        .unwrap()
        .remove("network");
    assert_eq!(no_onset_evidence(&capture), NoOnsetEvidence::MissingData);
    capture.frames[0]["nearby_npcs"][0]["network"] = json!({"x": 101, "z": 100, "level": 0});
    capture.frames[0]["local_player"]
        .as_object_mut()
        .unwrap()
        .remove("network");
    assert_eq!(no_onset_evidence(&capture), NoOnsetEvidence::MissingData);
}

fn magic_protection_capture(begin_tick: i64, report_tick: i64, protect_tick: i64) -> CombatCapture {
    let mut capture = CombatCapture::default();
    capture.statuses.push(json!({"fields": {
        "combat_end": "Killed",
        "combat_engaged": "True",
        "combat_engaged_kind": "Npc",
        "combat_engaged_index": 7,
        "combat_engaged_npc_type": 477,
        "combat_evidence_tick": report_tick,
        "combat_ticks": report_tick - begin_tick,
    }}));
    capture.prayer_facts = vec![
        json!({"name": "Protect from Melee", "button_com": 5623, "varp": 97}),
        json!({"name": "Attack", "button_com": 5619, "varp": 93}),
        json!({"name": "Strength", "button_com": 5620, "varp": 94}),
    ];
    capture.actions = vec![
        json!({
            "kind": "interaction", "tick": protect_tick, "batch": 1, "accepted": true,
            "request": {"op": "if-button", "component_id": 5623},
            "snapshot": {"prayer_varps": [{"index": 97, "value": 0}]}
        }),
        json!({
            "kind": "interaction", "tick": protect_tick, "batch": 1, "accepted": true,
            "request": {"op": "npc", "name": "Khazard Warlord", "action": "Attack"}
        }),
    ];
    capture.frames = (begin_tick..=report_tick)
        .map(|tick| {
            json!({
                "tick": tick,
                "ingame": true,
                "tile": [100, 100, 0],
                "local_player": {
                    "tile": {"x": 100, "z": 100, "level": 0},
                    "network": {"x": 100, "z": 100, "level": 0}
                },
                "self_slot": 1,
                "stats": [
                    {"name": "hitpoints", "base": 40, "effective": 40},
                    {"name": "prayer", "base": 40, "effective": 30}
                ],
                "prayer_varps": [
                    {"index": 93, "value": 0},
                    {"index": 94, "value": 0},
                    {"index": 97, "value": if tick > protect_tick { 1 } else { 0 }},
                ],
                "nearby_npcs": [{
                    "name": "Khazard Warlord",
                    "index": 7,
                    "type": 477,
                    "tile": {"x": 107, "z": 100, "level": 0},
                    "network": {"x": 107, "z": 100, "level": 0},
                    "size": 1,
                    "distance": 7,
                    "health": if tick == report_tick { 0 } else { 170 },
                    "total_health": 170,
                    "animation": -1,
                    "targets_local": true,
                    "attack_animation": false,
                    "attack_spot": false,
                    "local_hitmark_recent": false,
                    "in_combat": true,
                    "target": {"kind": "Player", "index": 1},
                }]
            })
        })
        .collect();
    capture
}

fn set_magic_warlord_field(capture: &mut CombatCapture, tick: i64, field: &str, value: Value) {
    let attack_animation = field == "animation" && value.as_i64() == Some(401);
    let frame = capture
        .frames
        .iter_mut()
        .find(|frame| frame["tick"] == json!(tick))
        .expect("magic proof test frame");
    frame["nearby_npcs"][0][field] = value;
    if field == "animation" {
        frame["nearby_npcs"][0]["attack_animation"] = json!(attack_animation);
    }
}

fn set_npc_attack_evidence(
    npc: &mut Value,
    targets_local: bool,
    attack_animation: bool,
    attack_spot: bool,
    local_hitmark_recent: bool,
) {
    npc["targets_local"] = json!(targets_local);
    npc["attack_animation"] = json!(attack_animation);
    npc["attack_spot"] = json!(attack_spot);
    npc["local_hitmark_recent"] = json!(local_hitmark_recent);
}

fn set_warlord_attack_evidence(capture: &mut CombatCapture, tick: i64, live: bool) {
    let frame = capture
        .frames
        .iter_mut()
        .find(|frame| frame["tick"] == json!(tick))
        .expect("magic proof test frame");
    let self_slot = frame["self_slot"].clone();
    let npc = &mut frame["nearby_npcs"][0];
    set_npc_attack_evidence(npc, live, false, live, false);
    npc["target"] = if live {
        json!({"kind": "Player", "index": self_slot})
    } else {
        Value::Null
    };
}

#[test]
fn magic_protection_readiness_accepts_proven_no_onset_after_killed_report() {
    let capture = magic_protection_capture(50, 110, 62);
    assert_eq!(
        protection_timing_receipt(&capture),
        json!("not_applicable_no_onset")
    );
    assert!(
        magic_protection_timing_ok(&capture),
        "safe no-onset timing is acceptable only with the protect plan's restoring terminal"
    );
    assert!(no_melee_offensive_prayers(&capture));
    assert_eq!(
        report_with_end(&capture, "Killed").unwrap()["fields"]["combat_ticks"],
        json!(60)
    );
    assert_eq!(case_invalid_reason(Case::MageAuto, &capture), None);
}

#[test]
fn repeated_tick_frames_preserve_all_no_onset_and_prayer_evidence() {
    let mut capture = magic_protection_capture(50, 110, 62);
    capture.frames.insert(21, capture.frames[20].clone());
    assert_eq!(
        protection_timing_receipt(&capture),
        json!("not_applicable_no_onset")
    );
    assert!(no_melee_offensive_prayers(&capture));
    capture.frames[21]["nearby_npcs"][0]["distance"] = json!(1);
    capture.frames[21]["nearby_npcs"][0]["network"]["x"] = json!(101);
    assert_eq!(protection_timing_receipt(&capture), json!(false));
    capture.frames[21]["nearby_npcs"][0]["distance"] = json!(7);
    capture.frames[21]["nearby_npcs"][0]["network"]["x"] = json!(107);
    capture.frames[21]["prayer_varps"][0]["value"] = json!(1);
    assert!(!no_melee_offensive_prayers(&capture));
    capture.frames.swap(21, 22);
    assert_eq!(protection_timing_receipt(&capture), json!(false));
}

#[test]
fn magic_protection_timing_unit_controls_a_b_g_and_i() {
    // (a) onset + 3 remains late.
    let mut late = magic_protection_capture(50, 110, 61);
    set_magic_warlord_field(&mut late, 58, "animation", json!(401));
    assert_eq!(protection_timing_receipt(&late), json!(false));

    // (b) a real onset with Protect never observed on remains a failure.
    let mut never_on = magic_protection_capture(50, 110, 62);
    set_magic_warlord_field(&mut never_on, 58, "animation", json!(401));
    for frame in &mut never_on.frames {
        frame["prayer_varps"][2]["value"] = json!(0);
    }
    assert_eq!(protection_timing_receipt(&never_on), json!(false));

    // (g) the same plan fails for an onset at 58, but passes if the first
    // onset moves to 100 after Protect is already observed on.
    let mut early_onset = magic_protection_capture(50, 110, 62);
    set_magic_warlord_field(&mut early_onset, 58, "animation", json!(401));
    assert_eq!(protection_timing_receipt(&early_onset), json!(false));
    let mut late_onset = magic_protection_capture(50, 110, 62);
    set_magic_warlord_field(&mut late_onset, 100, "animation", json!(401));
    assert_eq!(protection_timing_receipt(&late_onset), json!(true));
    assert!(magic_protection_timing_ok(&late_onset));

    // (i) a protect before Fight has no restore obligation; a later control
    // rejects a genuinely late protect after Attack.
    let mut no_terminal = magic_protection_capture(50, 110, 62);
    no_terminal.actions.pop();
    assert_eq!(
        protection_timing_receipt(&no_terminal),
        json!("not_applicable_no_onset")
    );
    assert!(magic_protection_timing_ok(&no_terminal));
}

#[test]
fn magic_no_onset_controls_c_through_f_require_positive_complete_evidence() {
    // (c) rewriting every 401 to -1 cannot open N/A when the engaged NPC is
    // at distance 1 on any frame.
    let mut missed_onset = magic_protection_capture(50, 110, 62);
    set_magic_warlord_field(&mut missed_onset, 70, "distance", json!(1));
    set_magic_warlord_field(
        &mut missed_onset,
        70,
        "network",
        json!({"x": 101, "z": 100, "level": 0}),
    );
    assert_eq!(protection_timing_receipt(&missed_onset), json!(false));

    // (d) both a missing interval frame and a missing exact actor are INVALID,
    // never not-applicable.
    let mut missing_frame = magic_protection_capture(50, 110, 62);
    missing_frame
        .frames
        .retain(|frame| frame["tick"] != json!(80));
    assert_eq!(protection_timing_receipt(&missing_frame), json!(false));
    assert!(case_invalid_reason(Case::MageAuto, &missing_frame)
        .is_some_and(|reason| reason.contains("INVALID")));
    let mut missing_actor = magic_protection_capture(50, 110, 62);
    missing_actor.frames[20]["nearby_npcs"] = json!([]);
    assert_eq!(protection_timing_receipt(&missing_actor), json!(false));
    assert!(case_invalid_reason(Case::MageAuto, &missing_actor)
        .is_some_and(|reason| reason.contains("INVALID")));

    // (e) any adjacent frame refuses N/A.
    let mut adjacent = magic_protection_capture(50, 110, 62);
    set_magic_warlord_field(&mut adjacent, 90, "distance", json!(1));
    set_magic_warlord_field(
        &mut adjacent,
        90,
        "network",
        json!({"x": 101, "z": 100, "level": 0}),
    );
    assert_eq!(protection_timing_receipt(&adjacent), json!(false));

    // (f) an observed HP drop fails the no-onset branch rather than becoming
    // an INVALID missing-data classification.
    let mut hp_drop = magic_protection_capture(50, 110, 62);
    hp_drop.frames[40]["stats"][0]["effective"] = json!(39);
    assert_eq!(protection_timing_receipt(&hp_drop), json!(false));
    assert!(case_invalid_reason(Case::MageAuto, &hp_drop).is_none());
}

#[test]
fn magic_no_melee_offensive_prayer_control_h_gates_every_magic_case() {
    let mut safe = magic_protection_capture(50, 110, 62);
    for case in [
        Case::MageAuto,
        Case::MageManualFallback,
        Case::MageManualNoFallback,
    ] {
        assert_eq!(
            magic_receipt(&safe, case)["no_melee_offensive_prayers"],
            json!(true)
        );
    }

    // (h) either offensive button or either Attack/Strength varp activates
    // the N1 gate for every current magic case.
    safe.actions.push(json!({
        "kind": "interaction", "tick": 63, "batch": 2, "accepted": true,
        "request": {"op": "if-button", "component_id": 5619}
    }));
    assert!(!no_melee_offensive_prayers(&safe));
    for case in [
        Case::MageAuto,
        Case::MageManualFallback,
        Case::MageManualNoFallback,
    ] {
        assert_eq!(
            magic_receipt(&safe, case)["no_melee_offensive_prayers"],
            json!(false)
        );
    }

    let mut varp_93 = magic_protection_capture(50, 110, 62);
    varp_93.frames[20]["prayer_varps"][0]["value"] = json!(1);
    assert!(!no_melee_offensive_prayers(&varp_93));
    let mut varp_94 = magic_protection_capture(50, 110, 62);
    varp_94.frames[20]["prayer_varps"][1]["value"] = json!(1);
    assert!(!no_melee_offensive_prayers(&varp_94));
}

#[test]
fn ranged_wrong_ammo_safety_requires_a_live_attacker_before_and_after_abort() {
    let mut capture = magic_protection_capture(50, 110, 62);
    for frame in &mut capture.frames {
        frame["stats"][1]["base"] = json!(43);
    }
    capture.actions.clear();
    capture.statuses[0]["fields"]["combat_end"] = json!("Aborted(PrepFailed(Ammo))");
    capture.statuses[0]["fields"]["combat_engaged_kind"] = Value::Null;
    capture.statuses[0]["fields"]["combat_engaged_index"] = Value::Null;
    capture.statuses[0]["fields"]["combat_engaged_npc_type"] = json!(-1);
    capture.statuses.push(json!({
        "phase": "Blocked",
        "failure": "ScriptFailure { code: \"parked\", message: \"combat aborted; caller must handle the failure\" }",
        "observed_tick": 110
    }));
    capture.random_events.extend([
        json!({"kind": "RangedSelectedFacts", "case": "R2"}),
        json!({"kind": "RangedNaturalDialogue", "spawned_index": 7, "npc_type": 477}),
        json!({"kind": "RangedProbe", "projectiles": []}),
    ]);
    capture.frames.last_mut().unwrap()["nearby_npcs"][0]["health"] = json!(170);
    set_warlord_attack_evidence(&mut capture, 110, true);
    for tick in 111..=122 {
        let mut frame = capture.frames.last().unwrap().clone();
        frame["tick"] = json!(tick);
        frame["ingame"] = json!(true);
        frame["nearby_npcs"][0]["health"] = json!(170);
        capture.frames.push(frame);
    }
    for frame in &mut capture.frames {
        if frame["tick"].as_i64().is_some_and(|tick| tick >= 110) {
            frame["nearby_npcs"][0]["in_combat"] = json!(false);
        }
    }
    assert!(
        !ranged_ready(Case::R2, &capture),
        "Blocked immediately after abort is not survival or safety ownership"
    );
    assert!(!ranged_abort_blocked_after_horizon(
        &capture,
        Some(121),
        Some(122)
    ));
    assert!(ranged_abort_blocked_after_horizon(
        &capture,
        Some(122),
        Some(122)
    ));

    capture
        .statuses
        .retain(|status| status["phase"] != json!("Blocked"));
    let walk_snapshot = capture
        .frames
        .iter()
        .find(|frame| frame["tick"] == json!(111))
        .unwrap()
        .clone();
    capture.actions.push(json!({
        "kind": "walk", "tick": 111,
        "request": {"target": {"x": 90, "z": 100, "level": 0}, "radius": 1},
        "snapshot": walk_snapshot
    }));
    let mut protect_snapshot = capture
        .frames
        .iter()
        .find(|frame| frame["tick"] == json!(112))
        .unwrap()
        .clone();
    protect_snapshot["prayer_varps"][2]["value"] = json!(0);
    capture.actions.push(json!({
        "kind": "guard", "tick": 112, "op": "if-button", "component_id": 5623,
        "accepted": true, "snapshot": protect_snapshot
    }));
    capture.statuses.push(json!({
        "phase": "Blocked",
        "failure": "ScriptFailure { code: \"parked\", message: \"combat abort guarded hold reached its 12-tick bound\" }",
        "observed_tick": 122
    }));
    for frame in capture
        .frames
        .iter_mut()
        .filter(|frame| frame["tick"] == json!(112))
    {
        frame["prayer_varps"][2]["value"] = json!(0);
    }
    for frame in capture
        .frames
        .iter_mut()
        .filter(|frame| frame["tick"].as_i64().is_some_and(|tick| tick > 112))
    {
        frame["prayer_varps"][2]["value"] = json!(1);
    }
    let hold_bound_safety = ranged_abort_safety_receipt(&capture, &capture.statuses[0]);
    assert_eq!(hold_bound_safety["terminal"], json!("hold_bound"));
    assert_eq!(hold_bound_safety["threat_at_abort"], json!(true));
    assert_eq!(hold_bound_safety["live_threat_seen"], json!(true));
    assert_eq!(hold_bound_safety["guarded_while_live_threat"], json!(true));
    set_warlord_attack_evidence(&mut capture, 110, false);
    let threat_only_after_abort = ranged_abort_safety_receipt(&capture, &capture.statuses[0]);
    assert_eq!(threat_only_after_abort["threat_at_abort"], json!(false));
    assert_eq!(threat_only_after_abort["live_threat_seen"], json!(true));
    assert_eq!(threat_only_after_abort["valid"], json!(false));
    assert!(!ranged_ready(Case::R2, &capture));
    set_warlord_attack_evidence(&mut capture, 110, true);
    assert!(
        ranged_ready(Case::R2, &capture),
        "the abort owner walks, protects while the Warlord targets us, and survives 12 ticks"
    );

    let live_threat_tick = capture
        .frames
        .iter()
        .position(|frame| frame["tick"] == json!(117))
        .unwrap();
    capture.frames[live_threat_tick]["stats"][0]["effective"] = json!(39);
    assert!(
        !ranged_ready(Case::R2, &capture),
        "declining HP under the live attacker fails the post-abort survival horizon"
    );
    capture.frames[live_threat_tick]["stats"][0]["effective"] = json!(40);
    let last_horizon_frame = capture.frames.pop().unwrap();
    assert!(
        !ranged_ready(Case::R2, &capture),
        "eleven observed ticks do not satisfy a twelve-tick survival horizon"
    );
    capture.frames.push(last_horizon_frame);
    assert!(ranged_ready(Case::R2, &capture));
    let observed_warlord = capture.frames[live_threat_tick]["nearby_npcs"][0].clone();
    capture.frames[live_threat_tick]["nearby_npcs"] = json!([]);
    assert_eq!(
        ranged_abort_safety_receipt(&capture, &capture.statuses[0])["valid"],
        json!(false),
        "an absent natural Warlord row is unknown, not evidence of no live threat"
    );
    capture.frames[live_threat_tick]["nearby_npcs"] = json!([observed_warlord]);
    capture.frames[live_threat_tick]["nearby_npcs"][0]
        .as_object_mut()
        .unwrap()
        .remove("targets_local");
    assert_eq!(
        ranged_abort_safety_receipt(&capture, &capture.statuses[0])["valid"],
        json!(false),
        "missing shared attack evidence is unknown, not evidence of no live threat"
    );
    capture.frames[live_threat_tick]["nearby_npcs"][0]["targets_local"] = json!(true);

    capture.statuses.push(json!({
        "phase": "Blocked",
        "failure": format!(
            "ScriptFailure {{ code: \"parked\", message: \"{}\" }}",
            RANGED_ABORT_HOLD_SAFE_REASON
        ),
        "observed_tick": 113
    }));
    for frame in capture
        .frames
        .iter_mut()
        .filter(|frame| frame["tick"].as_i64().is_some_and(|tick| tick > 112))
    {
        frame["nearby_npcs"][0]["target"] = Value::Null;
        frame["nearby_npcs"][0]["in_combat"] = json!(false);
        set_npc_attack_evidence(&mut frame["nearby_npcs"][0], false, false, false, false);
    }
    let hold_safe_safety = ranged_abort_safety_receipt(&capture, &capture.statuses[0]);
    assert_eq!(hold_safe_safety["terminal"], json!("hold_safe"));
    assert_eq!(hold_safe_safety["hold_safe_terminal_tick"], json!(113));
    assert_eq!(
        hold_safe_safety["hold_safe_disengaged_through_horizon"],
        json!(true)
    );
    assert!(
        ranged_ready(Case::R2, &capture),
        "a hold that ends after the threat disengages remains valid through the full survival horizon"
    );
    set_warlord_attack_evidence(&mut capture, 120, true);
    let reengaged_hold = ranged_abort_safety_receipt(&capture, &capture.statuses[0]);
    assert_eq!(
        reengaged_hold["hold_safe_disengaged_through_horizon"],
        json!(false),
        "hold_safe cannot pass when an attacker returns before horizon end"
    );
    assert!(!ranged_ready(Case::R2, &capture));
    set_warlord_attack_evidence(&mut capture, 120, false);
    capture.statuses.pop();

    capture.statuses.push(json!({
        "phase": "Blocked",
        "failure": "ScriptFailure { code: \"parked\", message: \"combat aborted; caller must handle the failure\" }",
        "observed_tick": 122
    }));
    for frame in capture
        .frames
        .iter_mut()
        .filter(|frame| frame["tick"].as_i64().is_some_and(|tick| tick >= 113))
    {
        frame["tile"] = json!([90, 100, 0]);
        frame["local_player"]["tile"] = json!({"x": 90, "z": 100, "level": 0});
        frame["local_player"]["network"] = json!({"x": 90, "z": 100, "level": 0});
        frame["nearby_npcs"][0]["distance"] = json!(10);
        frame["nearby_npcs"][0]["target"] = Value::Null;
        frame["nearby_npcs"][0]["in_combat"] = json!(false);
        set_npc_attack_evidence(&mut frame["nearby_npcs"][0], false, false, false, false);
    }
    assert_eq!(
        ranged_abort_safety_receipt(&capture, &capture.statuses[0])["terminal"],
        json!("walk_completed")
    );
    assert!(
        ranged_ready(Case::R2, &capture),
        "Blocked after a demonstrated safe escape remains a valid terminal"
    );
    capture.statuses.last_mut().unwrap()["failure"] = json!(format!(
        "ScriptFailure {{ code: \"parked\", message: \"{}\" }}",
        RANGED_ABORT_HOLD_SAFE_REASON
    ));
    assert_eq!(
        ranged_abort_safety_receipt(&capture, &capture.statuses[0])["terminal"],
        json!("hold_safe")
    );
    assert!(ranged_ready(Case::R2, &capture));

    capture.frames[20]["prayer_varps"][0]["value"] = json!(1);
    assert!(!ranged_ready(Case::R2, &capture));
    capture.frames[20]["prayer_varps"][0]["value"] = json!(0);
    capture.frames.last_mut().unwrap()["nearby_npcs"] = json!([]);
    assert!(!ranged_ready(Case::R2, &capture));
}

#[test]
fn ranged_verified_no_threat_is_named_but_cannot_pass_r2() {
    let mut capture = magic_protection_capture(50, 110, 62);
    capture.actions.clear();
    capture.statuses[0]["fields"]["combat_end"] = json!("Aborted(PrepFailed(Ammo))");
    capture.statuses[0]["fields"]["combat_engaged_kind"] = Value::Null;
    capture.statuses[0]["fields"]["combat_engaged_index"] = Value::Null;
    capture.statuses[0]["fields"]["combat_engaged_npc_type"] = json!(-1);
    capture.random_events.extend([
        json!({"kind": "RangedSelectedFacts", "case": "R2"}),
        json!({"kind": "RangedNaturalDialogue", "spawned_index": 7, "npc_type": 477}),
        json!({"kind": "RangedProbe", "projectiles": []}),
    ]);
    for frame in &mut capture.frames {
        frame["nearby_npcs"][0]["target"] = Value::Null;
        frame["nearby_npcs"][0]["in_combat"] = json!(false);
        set_npc_attack_evidence(&mut frame["nearby_npcs"][0], false, false, false, false);
    }
    for tick in 111..=122 {
        let mut frame = capture.frames.last().unwrap().clone();
        frame["tick"] = json!(tick);
        frame["ingame"] = json!(true);
        capture.frames.push(frame);
    }
    capture.statuses.push(json!({
        "phase": "Blocked",
        "failure": format!(
            "ScriptFailure {{ code: \"parked\", message: \"{}\" }}",
            RANGED_ABORT_WALK_FAILED_REASON
        ),
        "observed_tick": 122
    }));

    let safety = ranged_abort_safety_receipt(&capture, &capture.statuses[0]);
    assert_eq!(safety["outcome"], json!("verified_no_threat"));
    assert_eq!(safety["verified_no_threat"], json!(true));
    assert_eq!(safety["threat_at_abort"], json!(false));
    assert_eq!(safety["valid"], json!(false));
    assert!(
        !ranged_ready(Case::R2, &capture),
        "the named no-threat outcome cannot replace R2's live-threat-at-abort proof"
    );

    capture.frames[60]["nearby_npcs"] = json!([]);
    let incomplete = ranged_abort_safety_receipt(&capture, &capture.statuses[0]);
    assert_eq!(
        incomplete["outcome"],
        json!("unverified_or_threat_after_abort")
    );
    assert_eq!(incomplete["valid"], json!(false));
    assert!(
        !ranged_ready(Case::R2, &capture),
        "a missing actor observation is unknown, not verified no-threat"
    );
}

#[test]
fn ranged_wrong_ammo_uses_food_guard_when_melee_protect_is_unavailable() {
    let mut capture = magic_protection_capture(50, 110, 62);
    capture.actions.clear();
    capture.statuses[0]["fields"]["combat_end"] = json!("Aborted(PrepFailed(Ammo))");
    capture.statuses[0]["fields"]["combat_engaged_kind"] = Value::Null;
    capture.statuses[0]["fields"]["combat_engaged_index"] = Value::Null;
    capture.statuses[0]["fields"]["combat_engaged_npc_type"] = json!(-1);
    capture.random_events.extend([
        json!({"kind": "RangedSelectedFacts", "case": "R2"}),
        json!({"kind": "RangedNaturalDialogue", "spawned_index": 7, "npc_type": 477}),
        json!({"kind": "RangedProbe", "projectiles": []}),
    ]);
    for frame in &mut capture.frames {
        frame["stats"][1]["base"] = json!(42);
        frame["stats"][1]["effective"] = json!(0);
        frame["prayer_varps"][2]["value"] = json!(0);
    }
    capture.frames.last_mut().unwrap()["nearby_npcs"][0]["health"] = json!(170);
    set_warlord_attack_evidence(&mut capture, 110, true);
    for tick in 111..=122 {
        let mut frame = capture.frames.last().unwrap().clone();
        frame["tick"] = json!(tick);
        frame["ingame"] = json!(true);
        frame["nearby_npcs"][0]["health"] = json!(170);
        capture.frames.push(frame);
    }
    for frame in &mut capture.frames {
        if frame["tick"].as_i64().is_some_and(|tick| tick >= 110) {
            frame["nearby_npcs"][0]["in_combat"] = json!(false);
        }
    }
    let walk_snapshot = capture
        .frames
        .iter()
        .find(|frame| frame["tick"] == json!(111))
        .unwrap()
        .clone();
    capture.actions.push(json!({
        "kind": "walk", "tick": 111,
        "request": {"target": {"x": 90, "z": 100, "level": 0}, "radius": 1},
        "snapshot": walk_snapshot
    }));
    capture.actions.push(json!({
        "kind": "guard", "tick": 111, "op": "eat", "accepted": true
    }));
    capture.statuses.push(json!({
        "phase": "Blocked",
        "failure": format!(
            "ScriptFailure {{ code: \"parked\", message: \"{}\" }}",
            RANGED_ABORT_HOLD_BOUND_REASON
        )
    }));

    let safety = ranged_abort_safety_receipt(&capture, &capture.statuses[0]);
    assert_eq!(safety["terminal"], json!("hold_bound"));
    assert_eq!(safety["guarded_while_live_threat"], json!(true));
    assert_eq!(safety["ticks_observed"], json!(RANGED_ABORT_SURVIVAL_TICKS));
    assert!(
        ranged_ready(Case::R2, &capture),
        "food guard keeps the bot alive when prayer level is below Protect from Melee's requirement"
    );
}

#[test]
fn ranged_protection_controls_a_through_i_gate_ranged_kill_cells() {
    for case in [Case::R1, Case::R3] {
        let clean = || magic_protection_capture(50, 110, 62);
        let safe = clean();
        assert_eq!(
            ranged_receipt(case, &safe)["protect_timing"],
            "not_applicable_no_onset"
        );
        assert_eq!(
            ranged_receipt(case, &safe)["no_melee_offensive_prayers"],
            true
        );
        assert_eq!(ranged_receipt(case, &safe)["protection_valid"], true);

        for control in [
            "a",
            "b",
            "c",
            "d-frame",
            "d-actor",
            "e",
            "f",
            "g-",
            "h-button",
            "h-button-5620",
            "h-93",
            "h-94",
            "i",
        ] {
            let mut mutant = clean();
            match control {
                "a" | "g-" => set_magic_warlord_field(&mut mutant, 58, "animation", json!(401)),
                "b" => {
                    set_magic_warlord_field(&mut mutant, 100, "animation", json!(401));
                    for frame in &mut mutant.frames {
                        frame["prayer_varps"][2]["value"] = json!(0);
                    }
                }
                "c" | "e" => {
                    set_magic_warlord_field(&mut mutant, 70, "distance", json!(1));
                    set_magic_warlord_field(
                        &mut mutant,
                        70,
                        "network",
                        json!({"x": 101, "z": 100, "level": 0}),
                    );
                }
                "d-frame" => mutant.frames.retain(|frame| frame["tick"] != 80),
                "d-actor" => mutant.frames[20]["nearby_npcs"] = json!([]),
                "f" => mutant.frames[40]["stats"][0]["effective"] = json!(39),
                "h-button" | "h-button-5620" => mutant.actions.push(json!({
                    "kind": "interaction", "tick": 63, "accepted": true,
                    "request": {"op": "if-button", "component_id": if control == "h-button" { 5619 } else { 5620 }}
                })),
                "h-93" => mutant.frames[20]["prayer_varps"][0]["value"] = json!(1),
                "h-94" => mutant.frames[20]["prayer_varps"][1]["value"] = json!(1),
                "i" => {
                    // Exercise the Fight-only restore obligation, not a
                    // pre-Fight Protect activation.
                    mutant.actions[1]["tick"] = json!(61);
                    mutant.actions[1]["request"]["index"] = json!(7);
                    mutant.actions.swap(0, 1);
                }
                _ => unreachable!(),
            }
            let receipt = ranged_receipt(case, &mutant);
            let leaf = if control.starts_with("h-") {
                "no_melee_offensive_prayers"
            } else {
                "protection_valid"
            };
            assert_eq!(receipt[leaf], false, "{} control {control}", case.key());
            assert!(
                !ranged_ready(case, &mutant),
                "{} control {control}",
                case.key()
            );
            if control.starts_with("d-") {
                assert!(case_invalid_reason(case, &mutant).is_some());
            }
        }
        let mut onset = clean();
        set_magic_warlord_field(&mut onset, 100, "animation", json!(401));
        assert_eq!(ranged_receipt(case, &onset)["protect_timing"], true);
    }
}

#[test]
#[ignore = "offline baked-collision probe requires the selected engine, cache and nav pack"]
fn magic_no_fallback_corridor_uses_validated_baked_collision() {
    let home = ThrowawayHome::enter("cm-G2-no-fallback-collision").unwrap();
    let template = profile_options(&home.path)
        .unwrap()
        .resolve(None)
        .unwrap()
        .prepare_template()
        .unwrap();
    let world = template.world().unwrap();
    let stand = Case::MageManualNoFallback.stand(&world).unwrap();
    for east in 0..=5 {
        for north in 0..=1 {
            assert!(world.collision.walkable(WorldTile {
                x: stand.x + east,
                z: stand.z + north,
                ..stand
            }));
        }
    }
    println!(
        "G2-false anchor=(2632,3254,0), baked 2-wide/5-tile spawn corridor starts at {stand:?}"
    );
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
            "in_combat": false, "animation": 404,
            "targets_local": true, "attack_animation": false,
            "attack_spot": true, "local_hitmark_recent": false,
            "target": {"kind": "Player", "index": 7}
        }]
    }));
    assert_eq!(first_warlord_live_fact(&capture), Some(43));
    assert_eq!(first_warlord_attack_onset(&capture), None);
    capture.frames[0]["nearby_npcs"][0]["animation"] = json!(401);
    capture.frames[0]["nearby_npcs"][0]["attack_animation"] = json!(true);
    assert_eq!(first_warlord_attack_onset(&capture), Some(43));
    capture.frames[0]["nearby_npcs"][0]["target"]["index"] = json!(9);
    capture.frames[0]["nearby_npcs"][0]["targets_local"] = json!(false);
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
        "request": {
            "op": "obj", "x": 2638, "z": 3224, "level": 0,
            "name": "Arrow", "action": "Take"
        },
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
    assert_eq!(ranged_pickup_count(&action, &after, 883), Some(0));
    assert_eq!(ranged_pickup_count(&action, &after, 884), Some(0));
}

#[test]
fn ranged_ammo_sweep_does_not_double_count_same_tile_stacks() {
    let stack = |count| {
        json!({
            "id": 886, "count": count, "tile": {"x": 2600, "z": 3372, "level": 0}
        })
    };
    let mut action = json!({
        "request": {
            "op": "obj", "x": 2600, "z": 3372, "level": 0,
            "name": "Steel arrow", "action": "Take"
        },
        "snapshot": {"ground_items": [stack(24), stack(22)]}
    });
    let after = json!({"ground_items": [stack(22)]});
    assert_eq!(ranged_pickup_count(&action, &after, 886), Some(24));
    assert_eq!(
        ranged_pickup_count(&action, &action["snapshot"], 886),
        Some(0)
    );
    let mut invalid_take = action.clone();
    invalid_take["request"]["action"] = json!("Examine");
    assert_eq!(
        ranged_pickup_count(&invalid_take, &invalid_take["snapshot"], 886),
        None
    );
    invalid_take["request"]["op"] = json!("npc");
    invalid_take["request"]["action"] = json!("Take");
    assert_eq!(
        ranged_pickup_count(&invalid_take, &invalid_take["snapshot"], 886),
        None
    );
    invalid_take["request"]["op"] = json!("obj");
    invalid_take["request"]["name"] = Value::Null;
    assert_eq!(
        ranged_pickup_count(&invalid_take, &invalid_take["snapshot"], 886),
        None
    );
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
    capture.random_events.extend([
        json!({"kind": "RangedSelectedFacts", "ammo_obj_id": 882}),
        json!({"kind": "RangedNaturalDialogue", "spawned_index": 7, "npc_type": 81}),
        json!({
            "kind": "RangedProbe", "tick": 11, "snapshot_tick": 11, "loop_cycle": 1000,
            "projectiles": [{
                "spotanim": 10, "t1": 1000, "t2": 1010, "distance": 1,
                "local_launch": true, "target": {"kind": "Npc", "index": 7}
            }]
        }),
    ]);
    let corpse = json!({
        "tick": 12, "snapshot_tick": 11,
        "inventory": [{"id": 882, "count": 19}],
        "nearby_npcs": [{"index": 7, "type": 81, "health": 0, "total_health": 10}],
        "ground_items": [{"id": 882, "count": 1}]
    });
    capture.frames.extend([
        json!({"snapshot_tick": 10, "inventory": [{"id": 882, "count": 20}]}),
        corpse.clone(),
        json!({
            "tick": 15, "snapshot_tick": 15,
            "inventory": [{"id": 882, "count": 19}], "nearby_npcs": []
        }),
    ]);
    let report = json!({"fields": {
        "combat_engaged_index": 7, "combat_engaged_npc_type": 81, "combat_evidence_tick": 15
    }});
    assert_eq!(ranged_corpse_frame(&capture, &report), Some(&corpse));
    for case in [Case::R1, Case::R3] {
        let receipt = ranged_ammo_receipt(case, &capture, &report);
        assert_eq!(receipt["launches"], json!(1));
        assert_eq!(receipt["corpse_tick"], json!(12));
        assert_eq!(receipt["held_count_at_kill"], json!(19));
        assert_eq!(receipt["pickup_action_count"], json!(0));
        assert_eq!(receipt["swept_units"], json!(0));
        assert_eq!(receipt["sweep_observed"], json!(false));
        assert_eq!(receipt["pickup_action_limit_valid"], json!(true));
        assert_eq!(receipt["pickup_count_observations_valid"], json!(true));
        assert_eq!(receipt["pickup_plan_shape_valid"], json!(true));
        assert_eq!(receipt["final_count"], json!(19));
        assert_eq!(receipt["expected_final_count"], json!(19));
        assert_eq!(receipt["count_matches_launches_and_sweeps"], json!(true));
        assert_eq!(receipt["all_pickups_after_kill"], json!(true));
        assert_eq!(receipt["dart_count_per_launch"], json!(true));
        assert_eq!(receipt["valid"], json!(false));
    }
    capture.frames[1]["nearby_npcs"][0]["type"] = json!(82);
    assert!(ranged_corpse_frame(&capture, &report).is_none());
}
#[test]
fn ranged_sweep_sums_observed_gains_and_accepts_a_content_voided_take() {
    use client::io::ClientProt289;

    let stack = |count| {
        json!({
            "id": 886, "count": count,
            "tile": {"x": 2456, "z": 3302, "level": 0}
        })
    };
    let stacks = |counts: &[i64]| counts.iter().map(|count| stack(*count)).collect::<Vec<_>>();
    let take = |tick, batch, counts: &[i64]| {
        json!({
            "kind": "interaction", "tick": tick, "batch": batch, "accepted": true,
            "wire_decoded": true,
            "wire_opcodes": [
                i64::from(ClientProt289::MOVE_OPCLICK.id),
                i64::from(ClientProt289::OPOBJ3.id)
            ],
            "request": {
                "op": "obj", "x": 2456, "z": 3302, "level": 0,
                "name": "Steel arrow", "action": "Take"
            },
            "snapshot": {"ground_items": stacks(counts)}
        })
    };

    let mut capture = CombatCapture::default();
    capture.start_baseline = Some(json!({
        "tick": 10, "inventory": [{"id": 886, "count": 150}], "equipment": []
    }));
    capture.random_events.extend([
        json!({"kind": "RangedSelectedFacts", "case": "R1", "ammo_obj_id": 886}),
        json!({"kind": "RangedNaturalDialogue", "spawned_index": 7, "npc_type": 477}),
        json!({
            "kind": "RangedProbe", "tick": 11, "snapshot_tick": 11, "loop_cycle": 1000,
            "projectiles": [{
                "spotanim": 10, "t1": 1000, "t2": 1010, "distance": 1,
                "local_launch": true, "target": {"kind": "Npc", "index": 7}
            }]
        }),
    ]);
    let corpse = json!({
        "tick": 12,
        "inventory": [{"id": 886, "count": 149}],
        "equipment": [],
        "nearby_npcs": [{"index": 7, "type": 477, "health": 0, "total_health": 170}],
        "ground_items": [stack(28), stack(11)]
    });
    capture.frames.extend([
        corpse,
        json!({
            "tick": 20, "inventory": [{"id": 886, "count": 188}], "equipment": [],
            "nearby_npcs": [], "ground_items": []
        }),
    ]);
    capture.actions = vec![
        take(13, 1, &[28, 11]),
        take(14, 2, &[28, 11]),
        take(15, 3, &[11]),
        take(16, 4, &[]),
    ];
    let report = json!({
        "fields": {
            "combat_engaged_index": 7, "combat_engaged_npc_type": 477,
            "combat_evidence_tick": 20
        }
    });

    let receipt = ranged_ammo_receipt(Case::R1, &capture, &report);
    assert_eq!(receipt["pickup_stack_counts"], json!([0, 28, 11, 0]));
    assert_eq!(receipt["pickup_action_count"], json!(4));
    assert_eq!(receipt["pickup_action_limit_valid"], json!(true));
    assert_eq!(receipt["swept_units"], json!(39));
    assert_eq!(receipt["sweep_observed"], json!(true));
    assert_eq!(receipt["pickup_count_observations_valid"], json!(true));
    assert_eq!(receipt["pickup_plan_shape_valid"], json!(true));
    assert_eq!(receipt["count_matches_launches_and_sweeps"], json!(true));
    assert_eq!(receipt["valid"], json!(true));

    let mut duplicate_row = capture.actions[0].clone();
    duplicate_row["sequence"] = json!(99);
    capture.actions.insert(1, duplicate_row);
    let malformed_plan = ranged_ammo_receipt(Case::R1, &capture, &report);
    assert_eq!(
        malformed_plan["pickup_count_observations_valid"],
        json!(true)
    );
    assert_eq!(malformed_plan["pickup_plan_shape_valid"], json!(false));
    assert_eq!(malformed_plan["valid"], json!(false));

    capture.actions.remove(1);
    capture.frames.last_mut().unwrap()["ground_items"] = Value::Null;
    let missing_count = ranged_ammo_receipt(Case::R1, &capture, &report);
    assert_eq!(
        missing_count["pickup_count_observations_valid"],
        json!(false)
    );
    assert_eq!(missing_count["pickup_plan_shape_valid"], json!(true));
}

#[test]
fn ranged_wrong_ammo_requires_survival_and_no_projectile() {
    let mut capture = CombatCapture::default();
    capture.statuses.push(json!({
        "phase": "Blocked",
        "failure": "ScriptFailure { code: \"parked\", message: \"combat aborted; caller must handle the failure\" }",
    }));
    capture.statuses.push(json!({
        "fields": {
            "combat_end": "Aborted(PrepFailed(Ammo))",
            "combat_evidence_tick": 110
        },
    }));
    capture.random_events.push(json!({
        "kind": "RangedProbe",
        "projectiles": [],
    }));
    assert!(
        !ranged_wrong_ammo_ready(&capture),
        "an abort followed immediately by Blocked has no post-abort survival proof"
    );

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
fn ranged_scenario_requires_the_natural_warlord_dialogue_before_the_firing_stand() {
    let approach = WorldTile {
        x: WARLORD_ANCHOR.x - 1,
        ..WARLORD_ANCHOR
    };
    let stand = WorldTile {
        x: WARLORD_ANCHOR.x - R2_STAGE_DISTANCE,
        ..WARLORD_ANCHOR
    };
    let scenario = scenario_for(
        Case::R2,
        approach,
        Some(RangedPlacement {
            spawn: approach,
            stands: vec![stand],
        }),
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
        position("seed Tree Gnome Village at returned-first-orb stage")
            < position("disable auto retaliation before Warlord dialogue")
    );
    assert!(
        position("disable auto retaliation before Warlord dialogue")
            < position("stand at the quest start")
    );
    assert!(
        position("stand at the quest start")
            < position(
                "wait up to 350 game ticks for the natural Khazard Warlord row at its map spawn"
            )
    );
    assert!(
        position("wait up to 350 game ticks for the natural Khazard Warlord row at its map spawn")
            < position("talk to the natural Khazard Warlord")
    );
    assert!(
        position("talk to the natural Khazard Warlord")
            < position("observe the opened Warlord dialogue before continuing")
    );
    assert!(
        position("drain the real Warlord dialogue before Start")
            < position("observe the settled Warlord dialogue before ranged Start")
    );
    assert!(
        position("observe the settled Warlord dialogue before ranged Start")
            < position("teleport adjacent to the natural Warlord for the live-threat ammo abort")
    );
    let spawn_wait = scenario
        .steps
        .iter()
        .find(|step| step.name.contains("350 game ticks"))
        .unwrap();
    let StepKind::Await { evidence, .. } = &spawn_wait.kind else {
        panic!("natural Warlord deadline is an observation wait");
    };
    assert!(spawn_wait.name.contains("Khazard Warlord row"));
    assert!(evidence.contains("Khazard Warlord row"));
    let Proof::NpcNameNear {
        name,
        x,
        z,
        level,
        radius,
    } = spawn_wait.wait.arm
    else {
        panic!("the bounded wait names the missing natural Warlord row");
    };
    assert_eq!(name, "Khazard warlord");
    assert_eq!(
        (x, z, level, radius),
        (WARLORD_ANCHOR.x, WARLORD_ANCHOR.z, WARLORD_ANCHOR.level, 9)
    );
    assert_eq!(spawn_wait.wait.budget_ticks, 350);
    assert_eq!(scenario.settings.deadline, Duration::from_secs(300));
    let start = scenario
        .steps
        .iter()
        .position(|step| matches!(step.kind, StepKind::StartScript))
        .unwrap();
    assert!(position("observe the natural Warlord attacking at the R2 prep stand") < start);
    assert!(!scenario
        .steps
        .iter()
        .any(|step| step.name.to_ascii_lowercase().contains("npcadd")));
    assert_eq!(ranged_aliases(Case::R2).1, "bolt");
}

#[test]
fn ranged_other_player_engagement_is_natural_area_interference() {
    use api::snapshot::ActorTargetView;

    let npc_target = ActorTargetView {
        kind: ActorKind::Player,
        index: 12,
    };
    assert_eq!(
        ranged_other_player_index(7, 42, Some(npc_target), std::iter::empty()),
        Some(12)
    );
    let player_targets_warlord = ActorTargetView {
        kind: ActorKind::Npc,
        index: 42,
    };
    assert_eq!(
        ranged_other_player_index(7, 42, None, [(9, Some(player_targets_warlord))].into_iter(),),
        Some(9)
    );
    let local_targets_warlord = Some((7, Some(player_targets_warlord)));
    assert_eq!(
        ranged_other_player_index(7, 42, None, local_targets_warlord.into_iter(),),
        None
    );
}

#[test]
fn ranged_preflight_requires_a_settled_natural_dialogue_at_the_real_map_warlord() {
    let selected = api::game_data::for_revision(ClientRevision::R289).unwrap();
    let spawn = WARLORD_ANCHOR;
    for (case, accepted_distances, out_of_range_distance) in
        [(Case::R1, [8, 10], 7), (Case::R2, [1, 2], 3)]
    {
        let facts = selected_ranged_facts(case, &selected).unwrap();
        let approach = WorldTile {
            x: spawn.x - 1,
            ..spawn
        };
        let stand = WorldTile {
            x: spawn.x - ranged_stage_distance(case),
            ..spawn
        };
        let mut capture = CombatCapture::default();
        capture.random_events.push(json!({
            "kind": "RangedNaturalDialogue",
            "natural_npc": "khazard_warlord",
            "npc_type": WARLORD_NPC_ID,
            "npc_anchor": tile_value(WARLORD_ANCHOR),
            "spawn_tile": tile_value(spawn),
            "approach_tile": tile_value(approach),
            "stand_tile": tile_value(stand),
            "spawned_index": 42,
            "treequest_varp": TREEQUEST_VARP,
            "treequest_value": TREEQUEST_RETURNED_FIRST_ORB,
            "retaliate_varp": AUTO_RETALIATE_VARP,
            "retaliate_off_before_talk": true,
            "retaliate_off_at_stand": true,
            "talk_sent": true,
            "dialogue_opened": true,
            "dialogue_settled": true,
            "tele_observed": true,
            "interference": null
        }));
        let live_at_start = case == Case::R2;
        let mut baseline = json!({
            "ingame": true, "scene_state": 2, "self_slot": 7,
            "tile": [stand.x, stand.z, stand.level],
            "local_player": {
                "tile": tile_value(stand),
                "network": tile_value(stand)
            },
            "nearby_npcs": [{
                "index": 42, "type": WARLORD_NPC_ID,
                "tile": tile_value(spawn), "network": tile_value(spawn),
                "distance": ranged_stage_distance(case),
                "targets_local": live_at_start,
                "attack_animation": live_at_start,
                "attack_spot": false,
                "local_hitmark_recent": false,
                "in_combat": false,
                "target": if live_at_start {
                    json!({"kind": "Player", "index": 7})
                } else {
                    Value::Null
                }
            }],
            "stats": [
                {"name": "ranged", "base": 70, "effective": 70},
                {"name": "defence", "base": 40, "effective": 40},
                {"name": "hitpoints", "base": 40, "effective": 40},
                {"name": "prayer", "base": 43, "effective": 43}
            ],
            "inventory": [
                {"id": facts.weapon_id, "count": 1},
                {"id": facts.ammo_id, "count": if case == Case::R1 { 150 } else { 50 }},
                {"id": PRAYER_POTION_4_ID, "count": 1},
                {"id": LOBSTER_ID, "count": 6}
            ]
        });
        assert_eq!(
            ranged_start_preflight(case, &baseline, &selected, &capture),
            None
        );

        baseline["local_player"]["tile"] = tile_value(WorldTile {
            x: stand.x + 8,
            ..stand
        });
        baseline["nearby_npcs"][0]["tile"] = tile_value(WorldTile {
            x: stand.x + 7,
            ..stand
        });
        baseline["nearby_npcs"][0]["distance"] = json!(1);
        for distance in accepted_distances {
            baseline["nearby_npcs"][0]["network"] = tile_value(WorldTile {
                x: stand.x + distance,
                ..stand
            });
            assert_eq!(
                ranged_start_preflight(case, &baseline, &selected, &capture),
                None,
                "network distance is authoritative for {} staging",
                case.key()
            );
        }
        baseline["nearby_npcs"][0]["network"] = tile_value(WorldTile {
            x: stand.x + out_of_range_distance,
            ..stand
        });
        assert!(
            ranged_start_preflight(case, &baseline, &selected, &capture).is_some(),
            "{} rejects a Warlord outside its staged network range",
            case.key()
        );
        baseline["nearby_npcs"][0]["network"] = tile_value(spawn);
        baseline["local_player"]["tile"] = tile_value(stand);
        baseline["nearby_npcs"][0]["tile"] = tile_value(spawn);
        baseline["nearby_npcs"][0]["distance"] = json!(ranged_stage_distance(case));

        if case == Case::R2 {
            let mut unrelated_attacker = baseline.clone();
            unrelated_attacker["nearby_npcs"][0]["targets_local"] = json!(false);
            unrelated_attacker["nearby_npcs"][0]["attack_animation"] = json!(false);
            unrelated_attacker["nearby_npcs"][0]["target"] = Value::Null;
            unrelated_attacker["nearby_npcs"]
                .as_array_mut()
                .unwrap()
                .push(json!({
                    "index": 99, "type": 1,
                    "targets_local": true, "attack_animation": true,
                    "attack_spot": false, "local_hitmark_recent": false
                }));
            assert!(
                ranged_start_preflight(case, &unrelated_attacker, &selected, &capture).is_some(),
                "another NPC attacking cannot substitute for the natural Warlord"
            );
            baseline["nearby_npcs"][0]["attack_animation"] = json!(false);
            baseline["nearby_npcs"][0]["attack_spot"] = json!(true);
            assert_eq!(
                ranged_start_preflight(case, &baseline, &selected, &capture),
                None,
                "an observed attack spot independently proves a live attack"
            );
            baseline["nearby_npcs"][0]["attack_spot"] = json!(false);
            baseline["nearby_npcs"][0]["in_combat"] = json!(true);
            assert!(ranged_start_preflight(case, &baseline, &selected, &capture).is_some());
            baseline["nearby_npcs"][0]["in_combat"] = json!(false);
            baseline["nearby_npcs"][0]["attack_animation"] = json!(true);
            baseline["nearby_npcs"][0]
                .as_object_mut()
                .unwrap()
                .remove("local_hitmark_recent");
            assert!(ranged_start_preflight(case, &baseline, &selected, &capture).is_some());
            baseline["nearby_npcs"][0]["local_hitmark_recent"] = json!(false);
        }

        baseline["stats"][2]["effective"] = json!(31);
        assert_eq!(
            ranged_start_preflight(case, &baseline, &selected, &capture),
            None,
            "nonlethal preparation damage is allowed"
        );
        baseline["stats"][2]["effective"] = json!(0);
        assert!(ranged_start_preflight(case, &baseline, &selected, &capture).is_some());
        baseline["stats"][2]["effective"] = json!(40);
        capture.random_events[0]["dialogue_settled"] = json!(false);
        assert!(ranged_start_preflight(case, &baseline, &selected, &capture).is_some());
        capture.random_events[0]["dialogue_settled"] = json!(true);
        capture.random_events[0]["interference"] = json!({"player_index": 9});
        assert!(ranged_start_preflight(case, &baseline, &selected, &capture).is_some());
        capture.random_events[0]["interference"] = Value::Null;
        baseline["local_player"]["network"] = Value::Null;
        assert!(ranged_start_preflight(case, &baseline, &selected, &capture).is_some());

        if case == Case::R1 {
            let missing_actor = json!({"nearby_npcs": []});
            let mut first_missing_tick = None;
            assert!(ranged_start_resample_pending(
                &missing_actor,
                &capture,
                100,
                &mut first_missing_tick
            ));
            assert!(ranged_start_resample_pending(
                &missing_actor,
                &capture,
                100 + RANGED_START_RESAMPLE_TICKS,
                &mut first_missing_tick
            ));
            assert!(!ranged_start_resample_pending(
                &missing_actor,
                &capture,
                101 + RANGED_START_RESAMPLE_TICKS,
                &mut first_missing_tick
            ));
            assert!(!ranged_start_resample_pending(
                &baseline,
                &capture,
                101 + RANGED_START_RESAMPLE_TICKS,
                &mut first_missing_tick
            ));
        }
    }
}

#[test]
fn ranged_runtime_fixture_expands_acquisition_radius_for_staging() {
    let source = serde_json::to_vec(&json!({
        "roles": [{
            "sequences": [{
                "steps": [{
                    "args": {
                        "tactic": {"style": "melee", "engage_radius": 6},
                        "stand": {"x": 1, "z": 2, "level": 0},
                        "area": {"box": []}
                    }
                }]
            }]
        }]
    }))
    .unwrap();
    let placement = RangedPlacement {
        spawn: WorldTile {
            x: 100,
            z: 200,
            level: 0,
        },
        stands: Vec::new(),
    };
    let runtime = ranged_runtime_fixture(&source, &placement);
    let fixture: Value = serde_json::from_slice(&runtime).unwrap();
    let args = &fixture["roles"][0]["sequences"][0]["steps"][0]["args"];
    assert_eq!(args["tactic"]["style"], json!("ranged"));
    assert_eq!(args["tactic"]["engage_radius"], json!(11));
    assert!(args.get("stand").is_none());
    assert_eq!(args["area"]["box"], json!([88, 188, 112, 212, 0]));
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

#[test]
fn mage_rune_deltas_count_splashes_and_capture_two_cast_ranges() {
    let npc = |distance, spot_animation, spot_animation_stamp| {
        json!({
            "type": 477,
            "name": "Khazard Warlord",
            "index": 8,
            "health": 170,
            "total_health": 170,
            "distance": distance,
            "spot_animation": spot_animation,
            "spot_animation_stamp": spot_animation_stamp,
        })
    };
    let inventory = |chaos, air, mind| {
        json!([
            {"id": CHAOS_RUNE_ID, "count": chaos},
            {"id": AIR_RUNE_ID, "count": air},
            {"id": MIND_RUNE_ID, "count": mind},
        ])
    };
    let mut capture = CombatCapture::default();
    capture.start_baseline = Some(json!({
        "snapshot_tick": 10,
        "inventory": inventory(12, 150, MAGIC_MIND_RUNES),
        "nearby_npcs": [npc(5, 0, 0)],
    }));
    let mut chaos = 12;
    let mut air = 150;
    let mut mind = MAGIC_MIND_RUNES;
    let selected = api::game_data::for_revision(ClientRevision::R289).unwrap();
    let projectile_id = selected
        .style_spotanims()
        .iter()
        .find(|fact| fact.style == 4 && fact.location == "projectile")
        .unwrap()
        .spotanim_id;
    let projectile = |tick, distance| {
        json!({
            "spotanim": projectile_id, "src": {"x": 100, "z": 100, "level": 0},
            "target": {"kind": "Npc", "index": 8},
            "t1": tick * 30 + 51, "t2": tick * 30 + 46 + 10 * distance,
        })
    };
    let hitmark = |tick, distance| {
        let impact = tick + (46 + 10 * distance) / 30 + 1;
        json!({
            "kind": "hitmark", "npc_index": 8, "slot": 0,
            "player_generation": impact, "client_cycle": impact * 30,
            "cycle": impact * 30 + 70, "damage_kind": 0, "damage": 0,
            "health": 170,
        })
    };
    for bolt in 1..=12 {
        chaos -= 1;
        air -= 3;
        let tick = 10 + bolt * 5;
        let distance = if bolt < 6 { 5 } else { 2 };
        capture.frames.push(json!({
            "tick": tick,
            "snapshot_tick": tick,
            "player_generation": tick,
            "tile": [100, 100, 0],
            "projectiles": [projectile(tick, distance)],
            "inventory": inventory(chaos, air, mind),
            "nearby_npcs": [npc(distance, if bolt == 1 { FAILED_SPELL_SPLASH } else { 0 }, 1)],
        }));
        capture.magic_npc_events.push(if bolt == 1 {
            json!({
                "kind": "splash", "npc_index": 8, "spot_animation": FAILED_SPELL_SPLASH,
                "spot_animation_stamp": tick * 30 + 46 + 10 * distance,
                "client_cycle": tick * 30 + 46 + 10 * distance,
                "player_generation": tick + 3, "health": 170,
            })
        } else {
            hitmark(tick, distance)
        });
    }
    for strike in 1..=2 {
        air -= 2;
        mind -= 1;
        let tick = 70 + strike * 5;
        capture.frames.push(json!({
            "tick": tick,
            "snapshot_tick": tick,
            "player_generation": tick,
            "tile": [100, 100, 0],
            "projectiles": [projectile(tick, 2)],
            "inventory": inventory(chaos, air, mind),
            "nearby_npcs": [npc(2, 0, 1)],
        }));
        capture.magic_npc_events.push(hitmark(tick, 2));
    }

    let evidence = magic_rune_evidence(&capture);
    assert!(evidence.coherent);
    assert_eq!(rune_cast_count(&evidence.casts, MageSpell::FireBolt), 12);
    assert_eq!(rune_cast_count(&evidence.casts, MageSpell::FireStrike), 2);
    assert!(rune_cast_cadence_ok(&evidence.casts));
    assert!(twelve_bolts_then_strikes(&evidence.casts, true));
    let splashes = magic_splashes(&capture, &evidence.casts);
    assert_eq!(splashes.len(), 1);
    assert_eq!(splashes[0]["health_before"], json!(170));
    assert_eq!(splashes[0]["health_after"], json!(170));
    assert_eq!(
        splashes[0]["cast_counted_from_rune_consumption"],
        json!(true)
    );
    assert_eq!(
        magic_cast_distances(&capture, &evidence.casts).first(),
        Some(&5)
    );
    assert!(magic_queue_contract(&capture, &evidence.casts));
    let generation = capture.magic_npc_events[1]["player_generation"]
        .as_u64()
        .unwrap();
    capture.magic_npc_events[1]["player_generation"] = json!(generation + 1);
    assert!(!magic_queue_contract(&capture, &evidence.casts));
    capture.magic_npc_events[1]["player_generation"] = json!(generation);
    capture.frames[0]["projectiles"][0]["src"]["x"] = json!(101);
    assert!(!magic_queue_contract(&capture, &evidence.casts));
    capture.frames[0]["projectiles"][0]["src"]["x"] = json!(100);
    let landing = capture.magic_npc_events[0]["client_cycle"]
        .as_i64()
        .unwrap();
    capture.magic_npc_events[0]["client_cycle"] = json!(landing - 1);
    assert!(magic_splashes(&capture, &evidence.casts).is_empty());
    capture.magic_npc_events[0]["client_cycle"] = json!(landing);

    let receipt = magic_receipt(&capture, Case::MageAuto);
    assert_eq!(receipt["seed"]["air_runes"], json!(150));
    assert_eq!(receipt["seed"]["mind_runes"], json!(90));
    assert_eq!(receipt["seed"]["prayer_restores_4dose"], json!(2));
    assert_eq!(
        receipt["seed_rationale"]["previous_seed_maximum_damage"]["combined"],
        json!(160)
    );
    assert_eq!(
        receipt["seed_rationale"]["previous_seed_maximum_damage"]["warlord_hitpoints"],
        json!(170)
    );
}

#[test]
fn every_magic_cell_waits_for_magic_not_cooking_during_staging() {
    for case in [
        Case::MageAuto,
        Case::MageManualFallback,
        Case::MageManualNoFallback,
    ] {
        let steps = preparation_steps(
            case,
            IMP_START,
            None,
            Arc::new(Mutex::new(CombatCapture::default())),
        );
        assert!(matches!(&steps[0].wait.arm, Proof::Stat { id: 6, min: 35 }));
    }
}

#[test]
fn every_magic_spawn_follows_the_settled_stand_and_precedes_start() {
    for case in [
        Case::MageAuto,
        Case::MageManualFallback,
        Case::MageManualNoFallback,
    ] {
        let scenario = scenario_for(
            case,
            IMP_START,
            None,
            Arc::new(Mutex::new(CombatCapture::default())),
        );
        let stand = scenario
            .steps
            .iter()
            .position(|step| step.name == "stand at the quest start")
            .unwrap();
        let spawn = scenario
            .steps
            .iter()
            .position(|step| {
                step.name == "spawn local Khazard Warlord and teleport five tiles east"
            })
            .unwrap();
        let start = scenario
            .steps
            .iter()
            .position(|step| matches!(step.kind, StepKind::StartScript))
            .unwrap();
        assert!(stand < spawn && spawn < start);
    }
}

#[test]
fn magic_preflight_compares_array_baseline_with_object_setup_tiles() {
    let tele = WorldTile {
        x: IMP_START.x + 5,
        ..IMP_START
    };
    let mut baseline = json!({
        "ingame": true, "scene_state": 2,
        "tile": [tele.x, tele.z, tele.level],
        "stats": [
            {"name": "magic", "base": 35, "effective": 35},
            {"name": "defence", "base": 40, "effective": 40},
            {"name": "hitpoints", "base": 40, "effective": 40},
            {"name": "prayer", "base": 43, "effective": 43}
        ],
        "prayer_varps": [{"index": 97, "value": 0}],
        "inventory": [
            {"id": CHAOS_RUNE_ID, "count": MAGIC_CHAOS_RUNES},
            {"id": AIR_RUNE_ID, "count": MAGIC_AIR_RUNES},
            {"id": MIND_RUNE_ID, "count": MAGIC_MIND_RUNES},
            {"id": PRAYER_POTION_4_ID, "count": MAGIC_PRAYER_RESTORES}
        ],
        "equipment": [{"id": STAFF_OF_FIRE_ID, "count": 1}],
        "nearby_npcs": [{"name": "Khazard warlord", "distance": 5}]
    });
    let setup = json!({
        "npcadd": "khazard_warlord",
        "spawn_tile": IMP_START,
        "tele_tile": tele,
        "chebyshev_distance": 5
    });
    for case in [
        Case::MageAuto,
        Case::MageManualFallback,
        Case::MageManualNoFallback,
    ] {
        assert_eq!(
            start_preflight_at(case, &baseline, Some(tele), Some(&setup)),
            None
        );
    }
    baseline["tile"] = json!([tele.x + 1, tele.z, tele.level]);
    assert!(start_preflight_at(Case::MageAuto, &baseline, Some(tele), Some(&setup)).is_some());
}

#[test]
fn adjacent_arm_steps_use_the_observation_that_enabled_the_next_press() {
    let first = json!({"tick": 49, "sequence": 1});
    let mut next = json!({
        "tick": 50, "sequence": 2,
        "snapshot": {
            "tick": 50,
            "roots": {"side_tabs": [{"root": SPELL_PANEL_ROOT}]}
        }
    });
    assert!(settled_between(&first, &next, |frame| {
        root_visible(frame, SPELL_PANEL_ROOT)
    }));
    next["snapshot"]["tick"] = json!(49);
    assert!(!settled_between(&first, &next, |frame| {
        root_visible(frame, SPELL_PANEL_ROOT)
    }));
    next["snapshot"]["tick"] = json!(50);
    next["snapshot"]["roots"]["side_tabs"][0]["root"] = json!(COMBAT_TAB_ROOT);
    assert!(!settled_between(&first, &next, |frame| {
        root_visible(frame, SPELL_PANEL_ROOT)
    }));
}

#[test]
fn magic_side_tab_is_one_exclusive_zero_event_row() {
    let mut capture = CombatCapture::default();
    capture.actions.push(json!({
        "kind": "interaction",
        "tick": 10,
        "batch": 1,
        "request_id": 1,
        "accepted": true,
        "wire_decoded": true,
        "wire_opcodes": [],
        "request": {"op": "side-tab", "tab": 0},
    }));
    capture
        .observations
        .push(json!({"tick": 10, "exclusive": true}));
    assert_eq!(plan_event_count(&batch_plans(&capture)[0]), Some(0));
    assert!(magic_batch_contract(Case::MageAuto, &capture));
}

#[test]
fn manual_spell_packets_are_widget_on_npc_not_attack() {
    use client::io::ClientProt289;

    let mut action = json!({
        "kind": "interaction",
        "accepted": true,
        "wire_decoded": true,
        "wire_opcodes": [
            ClientProt289::MOVE_OPCLICK.id,
            ClientProt289::OPNPCT.id
        ],
        "request": {
            "op": "use-widget-on",
            "component_id": FIRE_BOLT_WIDGET,
            "kind": "npc",
            "target_name": "Khazard Warlord",
            "index": 8,
        },
        "snapshot": {
            "nearby_npcs": [{
                "type": 477, "name": "Khazard Warlord", "index": 8
            }]
        }
    });
    assert!(action_wire_valid(&action));
    action["wire_opcodes"] = json!([ClientProt289::OPNPC2.id]);
    assert!(!action_wire_valid(&action));
    action["wire_opcodes"] = json!([ClientProt289::MOVE_OPCLICK.id, ClientProt289::OPNPCT.id]);
    action["request"]["target_name"] = json!("Goblin");
    assert!(!action_wire_valid(&action));
}

#[test]
fn magic_budget_end_is_invalid_not_a_pass() {
    let mut capture = CombatCapture::default();
    capture
        .statuses
        .push(json!({"fields": {"combat_end": "Budget"}}));
    assert!(case_invalid_reason(Case::MageAuto, &capture)
        .is_some_and(|reason| reason.contains("Budget")));
    assert!(!magic_ready(Case::MageAuto, &capture));
}

#[test]
fn manual_magic_has_no_incidental_splash_or_distance_gate() {
    for case in [Case::MageManualFallback, Case::MageManualNoFallback] {
        let end = if case == Case::MageManualNoFallback {
            "Aborted(Unprotected(NoRunes))"
        } else {
            "Killed"
        };
        let mut capture = magic_protection_capture(1, 10, 2);
        capture.statuses[0]["fields"]["combat_end"] = json!(end);
        // The onset makes protection timing applicable; manual cells do not
        // need a splash or multiple launch distances, even at melee range.
        set_magic_warlord_field(&mut capture, 5, "animation", json!(401));
        set_magic_warlord_field(&mut capture, 5, "distance", json!(1));
        capture.frames.push(json!({"tick": 50}));
        assert!(case_invalid_reason(case, &capture).is_none());
        assert_eq!(
            magic_receipt(&capture, case)["splash_and_two_distance_numeric_queue_required"],
            false
        );
    }
}
