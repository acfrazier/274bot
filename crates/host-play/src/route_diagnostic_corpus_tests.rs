use super::super::{script_observe, script_slot_or_insert, ScriptWall};
use super::*;
use api::interact::Driver;
use api::selected::{FactKey, QuestGate};
use api::snapshot::WorldTile;
use api::work::{MonotonicClock, SharedCancellation, SliceBudget, SystemClock};
use nav_planner::grid::StepGrid;
use nav_planner::quest_gates::{QuestFamilyId, QuestGates};
use nav_planner::router::FindOptions;
use nav_planner::world::NavWorld;
use nav_planner::world_state::WorldState;
use nav_planner::zones::{ZoneExempt, ZoneKey};
use script::native::walk::Walk;
use script::native::{ActionHandle, HostAuthority, NativeTick, Script, ScriptFailure, ScriptFlow};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::num::{NonZeroU16, NonZeroU32};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

// The comparator retains the former per-query 4M-node budgets. Keep the same
// 64-unit cooperative slices while allowing those finite queries to finish.
const MAX_DIAGNOSTIC_SLICES: usize = 25_000_000;
const CORPUS_CASE_COUNT: usize = 10;
const TEST_CARRY_ID: i32 = 8003;
const TEST_FORBIDDEN_ID: i32 = 8001;

#[derive(Clone)]
struct DiagnosticCase {
    name: &'static str,
    provenance: &'static str,
    world_kind: &'static str,
    world: Arc<NavWorld>,
    from: WorldTile,
    to: WorldTile,
    radius: i32,
    arrival: ArrivalKind,
    candidate_goals: Option<Vec<WorldTile>>,
    opts: FindOptions,
    walk_options: script::native::WalkOptions,
    state_name: &'static str,
    state: WorldState,
    expected_unbounded_native_end: Option<&'static str>,
}

impl DiagnosticCase {
    fn walk_goals(&self) -> Option<script::native::WalkGoals> {
        self.candidate_goals.as_ref().map(|goals| {
            script::native::WalkGoals::new(goals.clone())
                .expect("diagnostic corpus goals are valid")
        })
    }

    fn request(&self) -> Box<ScriptRouteRequest> {
        Box::new(ScriptRouteRequest {
            candidate_goals: self.walk_goals(),
            admission: crate::admission::Admission::manual(
                self.opts,
                Default::default(),
                1,
                crate::WalkGlobals::default(),
            ),
            generation: 1,
            request_id: 1,
            world: Arc::clone(&self.world),
            from: self.from,
            to: self.to,
            radius: self.radius,
            loc_id: None,
            arrival: self.arrival,
            opts: self.opts,
            state: Some(Arc::new(self.state.clone())),
            bank_origin: api::bank_memory::Origin::Unknown,
            bank: Vec::new(),
            live_geometry: None,
            exclusions: None,
            compat_tile_intent: false,
            prereq_catalog: None,
            completion: Default::default(),
        })
    }

    fn describe(&self, targets: &[WorldTile]) -> Value {
        json!({
            "case": self.name,
            "provenance": self.provenance,
            "world_kind": self.world_kind,
            "from": tile_json(self.from),
            "requested_to": tile_json(self.to),
            "radius": self.radius,
            "arrival": format!("{:?}", self.arrival),
            "candidate_goals": self.candidate_goals.as_ref().map(|goals| {
                goals.iter().copied().map(tile_json).collect::<Vec<_>>()
            }),
            "diagnostic_targets": targets.iter().copied().map(tile_json).collect::<Vec<_>>(),
            "diagnostic_target_count": targets.len(),
            "find_options": format!("{:?}", self.opts),
            "native_walk_options": format!("{:?}", self.walk_options),
            "state_name": self.state_name,
            "state": state_json(&self.state),
            "route_selection_executed": false,
            "diagnostics_entry": "prepared RouteJob targets, then called begin_diagnostics()",
            "expected_unbounded_native_end": self.expected_unbounded_native_end,
        })
    }
}

struct NativeWalkScript {
    target: WorldTile,
    radius: u16,
    arrival: ArrivalKind,
    candidate_goals: Option<script::native::WalkGoals>,
    options: script::native::WalkOptions,
    handle: Option<ActionHandle<Walk>>,
}

impl Script for NativeWalkScript {
    fn configure(
        &mut self,
        _next: Arc<script::native::PreparedConfig>,
    ) -> Result<script::native::SettingsApply, script::native::ConfigError> {
        Ok(script::native::SettingsApply::Applied)
    }

    fn tick(&mut self, tick: &mut NativeTick<'_>) -> Result<ScriptFlow, ScriptFailure> {
        if self.handle.is_none() {
            let request = script::native::WalkRequest {
                target: self.target,
                candidate_goals: self.candidate_goals.clone(),
                radius: self.radius,
                arrival: self.arrival,
                loc_id: None,
                options: self.options,
                required_after: tick.cx.evidence(),
                evidence: None,
                cross: Box::default(),
                avoid: Box::default(),
                protect: false,
                food_guard: false,
                cure_guard: false,
                allow: script::native::WalkAllow::default(),
            };
            self.handle = Some(
                tick.actions
                    .begin::<Walk>(request, &mut tick.cx)
                    .expect("test script begins a real native Walk action"),
            );
        }
        Ok(ScriptFlow::Continue)
    }
}

#[derive(Default)]
struct DiagnosticDriver;

impl Driver for DiagnosticDriver {
    fn set_menu(&mut self, _slot: i32, _action: i32, _a: i32, _b: i32, _c: i32) {}

    fn do_action(&mut self, _slot: i32) -> bool {
        false
    }

    fn try_move(
        &mut self,
        _src_x: i32,
        _src_z: i32,
        _dx: i32,
        _dz: i32,
        _try_nearest: bool,
        _loc_width: i32,
        _loc_length: i32,
        _loc_angle: i32,
        _loc_shape: i32,
        _forceapproach: i32,
        _type: i32,
    ) -> bool {
        false
    }

    fn local_route(&self) -> Option<(i32, i32)> {
        None
    }

    fn build_base(&self) -> (i32, i32) {
        (0, 0)
    }

    fn loc_typecode(&self, _scene_x: i32, _scene_z: i32) -> Option<i32> {
        None
    }

    fn send_packet(&mut self, _packet: &api::prot::Send) -> Result<(), api::prot::PacketError> {
        Ok(())
    }

    fn login(&mut self, _username: &str, _password: &str, _reconnect: bool) -> bool {
        false
    }
}

/// Keep the actual native walk action alive for as long as RouteJob uses its authority.
struct LiveNativeWalk {
    authority: HostAuthority,
    _scripts: ScriptWall,
    _navs: Arc<Mutex<HashMap<String, NavBot>>>,
}

fn live_native_walk(case: &DiagnosticCase, serial: u64) -> LiveNativeWalk {
    let name = format!("route-diagnostic-{serial}");
    let scripts: ScriptWall = Arc::new(Mutex::new(HashMap::new()));
    let slot = script_slot_or_insert(&scripts, &name);
    slot.lock()
        .expect("test script slot mutex is healthy")
        .start_test_script(
            Box::new(NativeWalkScript {
                target: case.to,
                radius: u16::try_from(case.radius).expect("native walk radius is nonnegative"),
                arrival: case.arrival,
                candidate_goals: case.walk_goals(),
                options: case.walk_options,
                handle: None,
            }),
            None,
        )
        .expect("native walk test script starts");
    let work_scope = slot
        .lock()
        .expect("test script slot mutex is healthy")
        .work_scope();
    let navs = Arc::new(Mutex::new(HashMap::from([(
        name.clone(),
        NavBot {
            work_scope,
            ..NavBot::default()
        },
    )])));
    let cheats = Arc::new(Mutex::new(HashMap::new()));
    let world = Some(Arc::clone(&case.world));
    let snapshot = GameSnapshot::new();
    let mut driver = DiagnosticDriver;

    let _ = script_observe(
        &mut driver,
        &name,
        true,
        true,
        1,
        Some((case.from.x, case.from.z, case.from.level)),
        None,
        Some(case.state.clone()),
        Some(&snapshot),
        None,
        &scripts,
        &cheats,
        &navs,
        &world,
        false,
        false,
    );

    let authority = navs
        .lock()
        .expect("test navigation mutex is healthy")
        .get(&name)
        .and_then(|bot| bot.native_walk.as_ref())
        .cloned()
        .expect("script_observe queued the real native walk authority");
    assert!(
        authority.live(),
        "native Walk action remains the authority owner"
    );

    LiveNativeWalk {
        authority,
        _scripts: scripts,
        _navs: navs,
    }
}

fn stage_identity(index: u64) -> WorkIdentity {
    WorkIdentity::new(
        RequestId::from_raw(RequestId::HOST_FIRST + index).expect("test route identity is valid"),
        api::identity::ProcessIncarnation::from_start_nonce(
            std::num::NonZeroU64::new(1).expect("test incarnation is nonzero"),
        ),
        Scope::Slot(
            api::identity::SlotKey::from_incarnation(1).expect("test slot incarnation is valid"),
        ),
        None,
    )
    .expect("test route identity is valid")
}

struct DiagnosticObservation {
    native_end: Option<String>,
    detail: Option<String>,
    missing_carry: Vec<(i32, i32)>,
    blocked_zone_ids: Vec<Value>,
    diagnostic_settled: usize,
    elapsed_ms: u128,
    cpu_time_us: Option<u64>,
    cutoff: bool,
    cutoff_stage: Option<String>,
    cutoff_search_purpose: Option<String>,
}

#[cfg(target_os = "linux")]
fn thread_cpu_time_us() -> Option<u64> {
    // SAFETY: Linux rusage contains integer fields for which zero is valid.
    let mut usage: libc::rusage = unsafe { std::mem::zeroed() };
    // SAFETY: `usage` is writable for getrusage, and RUSAGE_THREAD selects this test thread.
    if unsafe { libc::getrusage(libc::RUSAGE_THREAD, &mut usage) } != 0 {
        return None;
    }
    let timeval_us = |time: libc::timeval| {
        time.tv_sec
            .max(0)
            .saturating_mul(1_000_000)
            .saturating_add(time.tv_usec.max(0)) as u64
    };
    Some(timeval_us(usage.ru_utime).saturating_add(timeval_us(usage.ru_stime)))
}

#[cfg(not(target_os = "linux"))]
fn thread_cpu_time_us() -> Option<u64> {
    None
}

impl DiagnosticObservation {
    fn reason_signature(&self) -> Value {
        json!({
            "native_end": self.native_end,
            "detail": self.detail,
            "missing_carry": self.missing_carry.iter().map(|(id, count)| {
                json!({"id": id, "count": count})
            }).collect::<Vec<_>>(),
            "blocked_zone_ids": self.blocked_zone_ids,
        })
    }

    fn to_json(&self) -> Value {
        json!({
            "native_end": self.native_end,
            "detail": self.detail,
            "missing_carry": self.missing_carry.iter().map(|(id, count)| {
                json!({"id": id, "count": count})
            }).collect::<Vec<_>>(),
            "blocked_zone_ids": self.blocked_zone_ids,
            "diagnostic_settled": self.diagnostic_settled,
            "elapsed_ms": self.elapsed_ms,
            "cpu_time_us": self.cpu_time_us,
            "cutoff": self.cutoff,
            "cutoff_stage": self.cutoff_stage,
            "cutoff_search_purpose": self.cutoff_search_purpose,
            "reason_signature": self.reason_signature(),
        })
    }
}

fn run_diagnostics(
    case: &DiagnosticCase,
    authority: Option<HostAuthority>,
    diagnostic_budget: usize,
    identity_index: u64,
) -> DiagnosticObservation {
    let request = case.request();
    let targets = request.targets();
    assert!(
        !targets.is_empty(),
        "{} must contribute its complete nonempty diagnostic target set",
        case.name
    );
    let mut job = Box::new(
        RouteJob::prepare(case.name, request, RouteDemandKind::Walk { authority })
            .unwrap_or_else(|error| panic!("{} job preparation failed: {error}", case.name)),
    );
    job.identity = Some(stage_identity(identity_index));
    job.targets = targets;
    job.diagnostic_budget = diagnostic_budget;
    job.begin_diagnostics();
    let started = Instant::now();
    let cpu_started_us = thread_cpu_time_us();

    let clock = SystemClock;
    let cancellation = SharedCancellation::new();
    let units = NonZeroU32::new(64).expect("cooperative slice is nonzero");
    let (output, diagnostic_settled, search_purpose) = {
        let mut completed = None;
        for slice in 0..MAX_DIAGNOSTIC_SLICES {
            let budget = SliceBudget::new(
                units,
                api::identity::Deadline::after(clock.now(), Duration::from_millis(10)),
            );
            let mut context = StepContext::<nav_planner::contract::RoutePhase>::new(
                &clock,
                budget,
                &cancellation,
            );
            match job.step(&mut context) {
                StepOutcome::Yielded => {}
                StepOutcome::Completed(output) => {
                    completed = Some(output);
                    break;
                }
                StepOutcome::Failed(error) => {
                    panic!("{} RouteJob diagnostics failed: {error}", case.name)
                }
            }
            assert!(
                slice + 1 < MAX_DIAGNOSTIC_SLICES,
                "{} exceeded the finite {}-slice harness backstop",
                case.name,
                MAX_DIAGNOSTIC_SLICES
            );
        }
        let output = completed.unwrap_or_else(|| {
            panic!(
                "{} RouteJob diagnostics did not complete within {} slices",
                case.name, MAX_DIAGNOSTIC_SLICES
            )
        });
        (output, job.diagnostic_settled, job.search_purpose)
    };
    let elapsed_ms = started.elapsed().as_millis();
    let cpu_time_us = cpu_started_us
        .zip(thread_cpu_time_us())
        .map(|(started, finished)| finished.saturating_sub(started));
    let cutoff_detail = "no route found; the route check stopped after";
    let RouteJobOutput {
        outcome,
        native_failure,
        detail,
        missing,
        blocked,
        ..
    } = output;
    assert!(
        matches!(outcome, RouteJobOutcome::NoPath),
        "{} diagnostic budget must never change the route outcome",
        case.name
    );
    let cutoff = detail.as_deref().is_some_and(|text| {
        let diagnosis = text
            .strip_prefix("search_result=")
            .and_then(|rest| rest.split_once("; "))
            .map_or(text, |(_, diagnosis)| diagnosis);
        diagnosis.starts_with(cutoff_detail)
    });
    let mut blocked_zone_ids: Vec<_> = blocked.iter().copied().map(zone_key_json).collect();
    blocked_zone_ids.sort_by_key(zone_json_sort_key);
    DiagnosticObservation {
        native_end: native_failure.map(|end| format!("{end:?}")),
        detail,
        missing_carry: {
            let mut items: Vec<_> = missing.iter().map(|item| (item.id, item.count)).collect();
            items.sort_unstable();
            items
        },
        blocked_zone_ids,
        diagnostic_settled,
        elapsed_ms,
        cpu_time_us,
        cutoff,
        cutoff_stage: cutoff.then(|| "Search -> Diagnostics".to_owned()),
        cutoff_search_purpose: cutoff.then(|| format!("{search_purpose:?}")),
    }
}

fn zone_key_json(key: ZoneKey) -> Value {
    match key {
        ZoneKey::Zone(index) => json!({"kind": "zone", "index": index}),
        ZoneKey::Group(index) => json!({"kind": "group", "index": index}),
    }
}

fn zone_json_sort_key(value: &Value) -> (u8, u16) {
    let kind = match value.get("kind").and_then(Value::as_str) {
        Some("zone") => 0,
        Some("group") => 1,
        _ => u8::MAX,
    };
    let index = value
        .get("index")
        .and_then(Value::as_u64)
        .and_then(|value| u16::try_from(value).ok())
        .unwrap_or(u16::MAX);
    (kind, index)
}

fn tile_json(tile: WorldTile) -> Value {
    json!([tile.x, tile.z, tile.level])
}

fn state_json(state: &WorldState) -> Value {
    let mut inventory: Vec<_> = state.inv.iter().map(|(&id, &count)| (id, count)).collect();
    inventory.sort_unstable();
    let mut worn: Vec<_> = state.worn.iter().copied().collect();
    worn.sort_unstable();
    let mut stats: Vec<_> = state
        .stats
        .iter()
        .map(|(&id, &level)| (id, level))
        .collect();
    stats.sort_unstable();
    let mut varps: Vec<_> = state
        .varps
        .iter()
        .map(|(&id, &value)| (id, value))
        .collect();
    varps.sort_unstable();
    let mut quests: Vec<_> = state.quests.iter().cloned().collect();
    quests.sort_unstable();
    json!({
        "inventory": inventory,
        "inventory_ready": state.inventory_ready,
        "worn": worn,
        "equipment_ready": state.equipment_ready,
        "allow_fetchable_worn": state.allow_fetchable_worn,
        "stats": stats,
        "combat_level": state.combat_level,
        "varps": varps,
        "quests": quests,
        "map_members": state.map_members,
        "quest_evidence_present": state.quest_evidence.is_some(),
    })
}

fn rich_289_state() -> WorldState {
    WorldState {
        map_members: true,
        combat_level: Some(126),
        quests: HashSet::from([
            "Prince Ali Rescue".into(),
            "Rune Mysteries".into(),
            "Lost City".into(),
            "Shilo Village".into(),
            "Tree Gnome Village".into(),
            "The Grand Tree".into(),
        ]),
        inv: HashMap::from([
            (995, 10_000),
            (554, 1_000),
            (555, 1_000),
            (556, 1_000),
            (557, 1_000),
            (558, 1_000),
            (561, 1_000),
            (563, 1_000),
        ]),
        stats: (0..21).map(|skill| (skill, 99)).collect(),
        ..WorldState::empty()
    }
}

fn generic_no_path_world() -> Arc<NavWorld> {
    let origin = WorldTile {
        x: 0,
        z: 0,
        level: 0,
    };
    let mut blocked = vec![0u64; (3usize * 4).div_ceil(64)];
    blocked[0] |= 1u64 << 1;
    Arc::new(NavWorld::from_parts(
        nav_planner::collision::WorldCollision {
            origin,
            width: 3,
            height: 1,
            walk: vec![0; 3 * 4],
            blocked,
            flags: None,
        },
        nav_planner::transport::TransportGraph::default(),
        Vec::new(),
    ))
}

fn corridor_world(configure: impl FnOnce(&mut NavWorld)) -> Arc<NavWorld> {
    let mut world = NavWorld::from_grid(&StepGrid::fixture_door_corridor());
    configure(&mut world);
    Arc::new(world)
}

fn synthetic_case(
    name: &'static str,
    provenance: &'static str,
    world: Arc<NavWorld>,
    endpoints: (WorldTile, WorldTile),
    state_name: &'static str,
    state: WorldState,
    expected_unbounded_native_end: &'static str,
) -> DiagnosticCase {
    let (from, to) = endpoints;
    DiagnosticCase {
        name,
        provenance,
        world_kind: "synthetic_route_fixture",
        world,
        from,
        to,
        radius: 0,
        arrival: ArrivalKind::Reach,
        candidate_goals: None,
        opts: FindOptions::default(),
        walk_options: script::native::WalkOptions {
            allow_danger_zones: script::native::WalkBit::Allow,
            ..script::native::WalkOptions::default()
        },
        state_name,
        state,
        expected_unbounded_native_end: Some(expected_unbounded_native_end),
    }
}

fn synthetic_cases() -> Vec<DiagnosticCase> {
    let from = WorldTile {
        x: 1,
        z: 0,
        level: 0,
    };
    let to = WorldTile {
        x: 3,
        z: 0,
        level: 0,
    };
    let family = QuestFamilyId::new(
        [0x5a; 32],
        NonZeroU16::new(1).expect("quest evidence schema is nonzero"),
    );
    let quest_world = corridor_world(|world| {
        world.graph.quest_family = Some(family);
        world.graph.edges[0].quest_gates = Some(
            QuestGates::new(
                family,
                [QuestGate::Complete(FactKey::new("quest:route-diagnostic"))],
            )
            .expect("quest diagnostic gate is valid"),
        );
    });
    let carry_world = corridor_world(|world| {
        world.graph.edges[0].item_req = vec![(TEST_CARRY_ID, 1)];
    });
    let entrance_world = corridor_world(|world| {
        world.graph.edges[0].forbidden_held = vec![TEST_FORBIDDEN_ID];
    });
    let entrance_state = WorldState {
        inventory_ready: true,
        equipment_ready: true,
        inv: HashMap::from([(TEST_FORBIDDEN_ID, 1)]),
        ..WorldState::empty()
    };

    vec![
        DiagnosticCase {
            name: "generic_no_path",
            provenance: "deterministic disconnected collision fixture",
            world_kind: "synthetic_route_fixture",
            world: generic_no_path_world(),
            from: WorldTile {
                x: 0,
                z: 0,
                level: 0,
            },
            to: WorldTile {
                x: 2,
                z: 0,
                level: 0,
            },
            radius: 0,
            arrival: ArrivalKind::Reach,
            candidate_goals: None,
            opts: FindOptions::default(),
            walk_options: script::native::WalkOptions {
                allow_danger_zones: script::native::WalkBit::Allow,
                ..script::native::WalkOptions::default()
            },
            state_name: "empty",
            state: WorldState::empty(),
            expected_unbounded_native_end: Some("NoPath"),
        },
        synthetic_case(
            "unknown_quest_evidence",
            "synthetic one-door corridor with an unevidenced Complete quest gate",
            quest_world,
            (from, to),
            "empty_without_quest_evidence",
            WorldState::empty(),
            "NeedsEvidence",
        ),
        synthetic_case(
            "missing_carry_requirement",
            "synthetic one-door corridor with item_req and observed empty inventory",
            carry_world,
            (from, to),
            "observed_empty_inventory",
            WorldState {
                inventory_ready: true,
                equipment_ready: true,
                ..WorldState::empty()
            },
            "NoPath",
        ),
        synthetic_case(
            "entrance_requirement",
            "synthetic one-door corridor with forbidden_held item",
            entrance_world,
            (from, to),
            "observed_forbidden_held_item",
            entrance_state,
            "Blocked",
        ),
    ]
}

fn real_cases(world: Arc<NavWorld>) -> Vec<DiagnosticCase> {
    let tile = |x, z| WorldTile { x, z, level: 0 };
    let legacy = FindOptions {
        zones: ZoneExempt::all(),
        ..FindOptions::default()
    };
    let allow_legacy_zones = script::native::WalkOptions {
        allow_danger_zones: script::native::WalkBit::Allow,
        ..script::native::WalkOptions::default()
    };
    let scorpion_options = script::native::WalkOptions {
        allow_teleports: script::native::WalkBit::Allow,
        allow_wilderness: script::native::WalkBit::Allow,
        allow_danger_zones: script::native::WalkBit::Allow,
        ..script::native::WalkOptions::default()
    };

    let mut cases = vec![
        DiagnosticCase {
            name: "scorpion_no_key_radius5",
            provenance: "real_scorpcatcher_no_key_route_job regression",
            world_kind: "r289_navpack",
            world: Arc::clone(&world),
            from: WorldTile {
                x: 2701,
                z: 3475,
                level: 0,
            },
            to: WorldTile {
                x: 2875,
                z: 9799,
                level: 0,
            },
            radius: 5,
            arrival: ArrivalKind::Reach,
            candidate_goals: None,
            opts: FindOptions {
                allow_teleports: true,
                allow_wilderness: true,
                zones: ZoneExempt::all(),
                ..FindOptions::default()
            },
            walk_options: scorpion_options,
            state_name: "no_key_empty_state",
            state: WorldState::empty(),
            expected_unbounded_native_end: None,
        },
        DiagnosticCase {
            name: "karamja_exit_without_coins",
            provenance:
                "R289 sailors.rs2:13-30 charges 30 coins before sailing from Karamja to Port Sarim",
            world_kind: "r289_navpack",
            world: Arc::clone(&world),
            from: tile(2954, 3147),
            to: tile(3028, 3217),
            radius: 0,
            arrival: ArrivalKind::Reach,
            candidate_goals: None,
            opts: legacy,
            walk_options: allow_legacy_zones,
            state_name: "members_empty_inventory",
            state: WorldState {
                map_members: true,
                inventory_ready: true,
                equipment_ready: true,
                ..WorldState::empty()
            },
            expected_unbounded_native_end: None,
        },
        DiagnosticCase {
            name: "desert_walk_without_shantay_pass",
            provenance:
                "R289 shantay_pass.rs2:76-97 requires and consumes one pass entering the desert",
            world_kind: "r289_navpack",
            world: Arc::clone(&world),
            from: tile(3304, 3123),
            to: tile(3304, 3115),
            radius: 0,
            arrival: ArrivalKind::Reach,
            candidate_goals: None,
            opts: legacy,
            walk_options: allow_legacy_zones,
            state_name: "members_empty_inventory",
            state: WorldState {
                map_members: true,
                inventory_ready: true,
                equipment_ready: true,
                ..WorldState::empty()
            },
            expected_unbounded_native_end: None,
        },
        DiagnosticCase {
            name: "unreachable_fail_closed",
            provenance: "17-case per-goal parity corpus: unreachable_fail_closed",
            world_kind: "r289_navpack",
            world: Arc::clone(&world),
            from: tile(3222, 3218),
            to: tile(2817, 3443),
            radius: 0,
            arrival: ArrivalKind::Area,
            candidate_goals: Some(vec![tile(2817, 3443)]),
            opts: legacy,
            walk_options: allow_legacy_zones,
            state_name: "empty",
            state: WorldState::empty(),
            expected_unbounded_native_end: None,
        },
        DiagnosticCase {
            name: "lumbridge_rellekka_refused_zones_l126",
            provenance:
                "17-case per-goal parity corpus; nav/examples/perf_corpus.rs zone refusal witness",
            world_kind: "r289_navpack",
            world,
            from: tile(3222, 3218),
            to: tile(2664, 3664),
            radius: 0,
            arrival: ArrivalKind::Area,
            candidate_goals: Some(vec![tile(2664, 3664)]),
            opts: FindOptions::default(),
            walk_options: script::native::WalkOptions::default(),
            state_name: "rich_289_combat126",
            state: rich_289_state(),
            expected_unbounded_native_end: Some("Refused"),
        },
    ];
    let mut live_scorpion = cases[0].clone();
    live_scorpion.name = "scorpion_no_key_live_spawn";
    live_scorpion.provenance =
        "Engine L live no-route receipt starts at the selected account's spawn";
    live_scorpion.from = tile(3094, 3106);
    cases.push(live_scorpion);
    cases
}

fn changed_reason_rows(rows: &[Value]) -> Vec<Value> {
    rows.iter()
        .filter_map(|row| {
            let bounded = row.get("bounded")?;
            let unbounded = row.get("unbounded")?;
            let bounded_signature = bounded.get("reason_signature")?;
            let unbounded_signature = unbounded.get("reason_signature")?;
            (bounded_signature != unbounded_signature).then(|| {
                json!({
                    "case": row.get("case"),
                    "bounded_reason": bounded_signature,
                    "unbounded_reason": unbounded_signature,
                })
            })
        })
        .collect()
}

fn worst_case(rows: &[Value], run: &str, metric: &str) -> Value {
    let mut worst: Option<(&Value, u64)> = None;
    for row in rows {
        let Some(observation) = row.get(run) else {
            continue;
        };
        let Some(value) = observation.get(metric).and_then(Value::as_u64) else {
            continue;
        };
        if worst.is_none_or(|(_, current)| value > current) {
            worst = Some((row, value));
        }
    }
    worst.map_or(
        Value::Null,
        |(row, value)| json!({"case": row.get("case"), "value": value}),
    )
}

fn write_report(
    path: &Path,
    navpack: &Path,
    expected_cases: usize,
    rows: &[Value],
    started: &Instant,
) {
    let complete = rows.len() == expected_cases
        && rows.iter().all(|row| {
            row.get("bounded").is_some_and(|value| !value.is_null())
                && row.get("unbounded").is_some_and(|value| !value.is_null())
        });
    let record = json!({
        "schema": "274bot-route-diagnostic-budget-comparison-v1",
        "complete": complete,
        "expected_cases": expected_cases,
        "completed_pairs": rows.iter().filter(|row| {
            row.get("bounded").is_some_and(|value| !value.is_null())
                && row.get("unbounded").is_some_and(|value| !value.is_null())
        }).count(),
        "navpack": navpack.display().to_string(),
        "route_selection_timed": false,
        "timing_boundary": "RouteJob preparation and target construction are excluded; timing starts immediately after begin_diagnostics() and ends when that RouteJob completes.",
        "aggregate_budget": DIAGNOSTIC_BUDGET,
        "unbounded_budget": usize::MAX,
        "slice_backstop": {
            "max_slices_per_job": MAX_DIAGNOSTIC_SLICES,
            "units_per_slice": 64,
            "deadline_per_slice_ms": 10,
            "product_timeout_success": false,
        },
        "worst_case": {
            "bounded_max_diagnostic_settled": worst_case(rows, "bounded", "diagnostic_settled"),
            "bounded_max_elapsed_ms": worst_case(rows, "bounded", "elapsed_ms"),
            "unbounded_max_diagnostic_settled": worst_case(rows, "unbounded", "diagnostic_settled"),
            "unbounded_max_elapsed_ms": worst_case(rows, "unbounded", "elapsed_ms"),
        },
        "changed_reasons": changed_reason_rows(rows),
        "elapsed_ms": started.elapsed().as_millis(),
        "rows": rows,
    });
    std::fs::write(
        path,
        serde_json::to_vec_pretty(&record).expect("diagnostic corpus report serializes"),
    )
    .unwrap_or_else(|error| panic!("cannot write {}: {error}", path.display()));
}

fn assert_expected_native_end(case: &DiagnosticCase, observation: &DiagnosticObservation) {
    if let Some(expected) = case.expected_unbounded_native_end {
        assert!(
            observation
                .native_end
                .as_deref()
                .is_some_and(|actual| actual.starts_with(expected)),
            "{} expected native end {expected}, observed {:?}",
            case.name,
            observation.native_end,
        );
    }
}

#[test]
#[ignore = "requires BOT_ROUTE_DIAGNOSTIC_NAVPACK and BOT_ROUTE_DIAGNOSTIC_OUT; real diagnostic budget comparison corpus"]
fn compares_bounded_and_unbounded_route_diagnostics_corpus() {
    let navpack = PathBuf::from(
        std::env::var_os("BOT_ROUTE_DIAGNOSTIC_NAVPACK")
            .expect("set BOT_ROUTE_DIAGNOSTIC_NAVPACK to the reviewed R289 NAV_PACK"),
    );
    let output = PathBuf::from(
        std::env::var_os("BOT_ROUTE_DIAGNOSTIC_OUT")
            .expect("set BOT_ROUTE_DIAGNOSTIC_OUT to the diagnostic comparison JSON path"),
    );
    assert!(navpack.is_absolute() && output.is_absolute());

    let loaded = NavWorld::load_pack(&navpack).unwrap_or_else(|error| {
        panic!(
            "cannot load diagnostic NAVPACK {}: {error:?}",
            navpack.display()
        )
    });
    loaded
        .bind_named_bank_facts(
            &api::game_data::for_revision(api::selected::ClientRevision::R289)
                .expect("R289 selected game data is available"),
        )
        .expect("R289 named bank facts bind once");
    let world = Arc::new(loaded);
    let mut cases = real_cases(world);
    cases.extend(synthetic_cases());
    assert_eq!(
        cases.len(),
        CORPUS_CASE_COUNT,
        "the checked-in diagnostic corpus must be complete, not sampled"
    );

    let started = Instant::now();
    let mut rows = Vec::with_capacity(CORPUS_CASE_COUNT);
    write_report(&output, &navpack, CORPUS_CASE_COUNT, &rows, &started);
    for (index, case) in cases.iter().enumerate() {
        assert!(
            case.world.collision.standable(case.from),
            "{} must start on a real standable tile, not the gate's blocked loc anchor",
            case.name,
        );
        let request = case.request();
        let targets = request.targets();
        assert!(
            !targets.is_empty(),
            "{} must contribute every goal to the diagnostics corpus",
            case.name
        );
        let row = json!({
            "case": case.name,
            "input": case.describe(&targets),
            "bounded": Value::Null,
            "unbounded": Value::Null,
        });
        rows.push(row);
        write_report(&output, &navpack, CORPUS_CASE_COUNT, &rows, &started);
        let native = (!matches!(
            case.name,
            "karamja_exit_without_coins" | "desert_walk_without_shantay_pass"
        ))
        .then(|| live_native_walk(case, index as u64));
        let authority = native.as_ref().map(|native| native.authority.clone());

        let unbounded = run_diagnostics(
            case,
            authority.clone(),
            usize::MAX,
            (index as u64).saturating_add(CORPUS_CASE_COUNT as u64),
        );
        rows.last_mut().expect("case row was appended")["unbounded"] = unbounded.to_json();
        write_report(&output, &navpack, CORPUS_CASE_COUNT, &rows, &started);
        assert_expected_native_end(case, &unbounded);
        let bounded = run_diagnostics(case, authority, DIAGNOSTIC_BUDGET, index as u64);
        rows.last_mut().expect("case row was appended")["bounded"] = bounded.to_json();
        write_report(&output, &navpack, CORPUS_CASE_COUNT, &rows, &started);
        match case.name {
            "scorpion_no_key_radius5" | "scorpion_no_key_live_spawn" => assert!(
                bounded.missing_carry.contains(&(1590, 1)),
                "real Scorpion route must diagnose the exact missing key row: {:?}",
                bounded.missing_carry,
            ),
            "karamja_exit_without_coins" => assert_eq!(
                bounded.missing_carry,
                vec![(995, 30)],
                "Karamja exit must preserve the single full-fare row required by boat recovery",
            ),
            "desert_walk_without_shantay_pass" => assert!(
                bounded.missing_carry.contains(&(1854, 1)),
                "desert entry must diagnose its exact Shantay pass row: {:?}",
                bounded.missing_carry,
            ),
            _ => {}
        }
        assert!(
            bounded.diagnostic_settled <= DIAGNOSTIC_BUDGET.saturating_add(64),
            "{} aggregate diagnostic count may overrun only by one cooperative slice",
            case.name,
        );
    }

    assert_eq!(
        rows.len(),
        CORPUS_CASE_COUNT,
        "the whole corpus must be executed"
    );
    assert!(
        rows.iter().all(|row| {
            row.get("bounded").is_some_and(|value| !value.is_null())
                && row.get("unbounded").is_some_and(|value| !value.is_null())
        }),
        "the diagnostic corpus must not report a partial comparison as complete"
    );
    write_report(&output, &navpack, CORPUS_CASE_COUNT, &rows, &started);
}

#[test]
fn native_diagnostic_reasons_preserve_unknown_quest_carry_and_entrance_requirements() {
    for (index, case) in synthetic_cases().into_iter().skip(1).enumerate() {
        let native = live_native_walk(&case, 10_000 + index as u64);
        let observation = run_diagnostics(
            &case,
            Some(native.authority.clone()),
            DIAGNOSTIC_BUDGET,
            20_000 + index as u64,
        );
        assert_expected_native_end(&case, &observation);
        assert!(
            !observation.cutoff,
            "small deterministic {} fixture must finish within the normal aggregate budget",
            case.name
        );
        match case.name {
            "unknown_quest_evidence" => {
                assert!(
                    observation
                        .native_end
                        .as_deref()
                        .is_some_and(|end| end.starts_with("NeedsEvidence")),
                    "unknown quest must publish NeedsEvidence: {:?}",
                    observation.native_end,
                );
            }
            "missing_carry_requirement" => assert_eq!(
                observation.missing_carry,
                vec![(TEST_CARRY_ID, 1)],
                "diagnosis preserves the exact missing carry requirement",
            ),
            "entrance_requirement" => {
                assert_eq!(observation.native_end.as_deref(), Some("Blocked"));
                assert!(observation
                    .detail
                    .as_deref()
                    .is_some_and(|detail| detail.contains("item 8001 must not be carried")));
            }
            _ => unreachable!("only gated synthetic reason cases follow the generic case"),
        }
    }
}

#[test]
fn diagnostic_budget_cutoff_publishes_no_path_and_the_cutoff_detail() {
    let case = synthetic_cases()
        .into_iter()
        .next()
        .expect("synthetic generic no-path case exists");
    for budget in [0, 1] {
        let native = live_native_walk(&case, 30_000 + budget as u64);
        let observation = run_diagnostics(
            &case,
            Some(native.authority.clone()),
            budget,
            30_100 + budget as u64,
        );
        assert_eq!(observation.native_end.as_deref(), Some("NoPath"));
        assert!(
            observation.cutoff,
            "an exhausted allowance cuts off post-failure diagnosis"
        );
        let expected = format!(
            "no route found; the route check stopped after {} places before it found the cause",
            observation.diagnostic_settled
        );
        let diagnosis = observation
            .detail
            .as_deref()
            .and_then(|detail| detail.strip_prefix("search_result=NoPath; "));
        assert!(
            diagnosis.is_some(),
            "cutoff detail must start with the exact search_result=NoPath prefix"
        );
        assert!(diagnosis.is_some_and(|detail| detail.starts_with(&expected)));
        assert!(
            observation.diagnostic_settled <= budget + 64,
            "the visible count includes the in-flight slice without hiding the cap overrun"
        );
    }
}

#[cfg(target_os = "linux")]
#[test]
#[ignore = "Linux release measurement; one real-pack diagnosis in a fresh process"]
fn measures_one_bounded_route_diagnosis_rss() {
    let navpack = std::env::var_os("BOT_ROUTE_DIAGNOSTIC_NAVPACK")
        .expect("BOT_ROUTE_DIAGNOSTIC_NAVPACK is required");
    let world = Arc::new(NavWorld::load_pack(Path::new(&navpack)).expect("real v17 pack"));
    let name = std::env::var("BOT_ROUTE_DIAGNOSTIC_MEMORY_CASE")
        .expect("BOT_ROUTE_DIAGNOSTIC_MEMORY_CASE is required");
    let case = real_cases(world)
        .into_iter()
        .find(|case| case.name == name)
        .expect("the selected real corpus case exists");
    let native = (!matches!(
        case.name,
        "karamja_exit_without_coins" | "desert_walk_without_shantay_pass"
    ))
    .then(|| live_native_walk(&case, 40_000));
    let authority = native.as_ref().map(|native| native.authority.clone());
    let rss_kib = |field: &str| {
        std::fs::read_to_string("/proc/self/status")
            .expect("Linux process memory status")
            .lines()
            .find_map(|line| {
                line.strip_prefix(field)
                    .and_then(|value| value.split_whitespace().next())
                    .and_then(|value| value.parse::<u64>().ok())
            })
            .expect("the requested process memory field is present")
    };
    let before = rss_kib("VmRSS:");
    // Linux clear_refs value 5 resets only this process's RSS high-water mark.
    // This excludes the earlier pack-load peak, not live diagnosis memory.
    std::fs::write("/proc/self/clear_refs", b"5").expect("reset own RSS high-water mark");
    let observation = run_diagnostics(&case, authority, DIAGNOSTIC_BUDGET, 40_001);
    let peak = rss_kib("VmHWM:");
    let after = rss_kib("VmRSS:");
    let report = json!({
        "case": case.name,
        "profile": if cfg!(debug_assertions) { "debug" } else { "release" },
        "aggregate_budget": DIAGNOSTIC_BUDGET,
        "rss_before_kib": before,
        "peak_rss_kib": peak,
        "rss_after_kib": after,
        "peak_rss_increase_kib": peak.saturating_sub(before),
        "scope": "one diagnosis plus RouteJob preparation; real pack and native owner loaded before RSS reset; no strict route selection",
        "diagnosis": observation.to_json(),
    });
    let output = std::env::var_os("BOT_ROUTE_DIAGNOSTIC_MEMORY_OUT")
        .expect("BOT_ROUTE_DIAGNOSTIC_MEMORY_OUT is required");
    std::fs::write(output, serde_json::to_vec_pretty(&report).unwrap())
        .expect("write the one-diagnosis memory measurement");
    assert!(observation.diagnostic_settled <= DIAGNOSTIC_BUDGET.saturating_add(64));
}
