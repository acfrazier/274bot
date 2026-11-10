use std::collections::HashMap;
use std::path::Path;
use std::time::Instant;

use api::snapshot::WorldTile;
use client::dash3d::LocShape;
use nav_planner::collision::WorldCollision;
use nav_planner::transport::{DoorDir, TransportEdge, TransportKind};
use rs2vm::machine::{
    Cond, CondClass, DeferKind, Effect, End, Entry, Expr, LocKind, Machine, PathResult, PlayerPos,
    Pred, Run, Value, View,
};
use rs2vm::profile::EngineProfile;
use rs2vm::program::Program;
use rs2vm::world::{LocObj, PlacedId};
use rs2vm::{Coord, LocTypeId, Site};
use serde::Serialize;

use super::index::Placement;
use super::nav_world::NavWorld;
use super::stage_doors::{gated_edge, worn_slot_facts, Needs};
use super::{
    dir_delta, door_dir, edge_ticks, extra_ticks, in_world_box, opposite, straight_door_landing,
};

#[derive(Serialize)]
pub(super) struct NavShadowReport {
    schema: u8,
    revision: u16,
    world: WorldSummary,
    machine_entries: usize,
    stage_elapsed_ns: u128,
    entries: Vec<EntryReport>,
    diffs: Vec<EdgeDiff>,
    limits: LimitSummary,
}

#[derive(Serialize)]
struct WorldSummary {
    mapsquares: usize,
    zones: usize,
    active_placements: usize,
}

#[derive(Serialize)]
struct LimitSummary {
    paths: BoundSummary,
    instructions: BoundSummary,
    bytes: BoundSummary,
}

#[derive(Default, Serialize)]
struct BoundSummary {
    maximum: u64,
    percentile_99: u64,
    ten_largest: Vec<EntryMeasurement>,
    recommended_limit: u64,
}

#[derive(Clone, Serialize)]
struct EntryMeasurement {
    value: u64,
    loc_type: i32,
    placed_id: u32,
    handler_script: Option<u32>,
    handler_name: Option<String>,
    handler_source: Option<String>,
    role: String,
    stand: Option<TileSummary>,
    bound_reached: bool,
}

#[derive(Serialize)]
struct EntryReport {
    loc_type: i32,
    placed_id: u32,
    handler_script: Option<u32>,
    handler_name: Option<String>,
    handler_source: Option<String>,
    role: String,
    placement: Option<TileSummary>,
    stand: Option<TileSummary>,
    stage_owned: bool,
    legacy_handler: String,
    execution: ExecutionReport,
}

#[derive(Serialize)]
struct ExecutionReport {
    status: String,
    elapsed_ns: u128,
    paths: u64,
    instructions: u64,
    bytes: u64,
    transfer_audit: Option<TransferSummary>,
    transfer_assertions: Option<TransferAssertions>,
    path_results: Vec<PathSummary>,
    candidates: Vec<CandidateSummary>,
}

#[derive(Serialize)]
struct TransferSummary {
    cancellations: u64,
    automatic_closes: u64,
    suspensions: u64,
    loop_summaries: u64,
    loop_allowance_before_exit: u64,
    unsupported_prefixes: u64,
}

#[derive(Serialize)]
struct TransferAssertions {
    unsupported_prefix_count_matches: bool,
    each_input_pause_was_cancelled: bool,
    each_loop_summary_has_pre_exit_allowance: bool,
    automatic_closes_do_not_exceed_suspensions: bool,
    all_hold: bool,
}

#[derive(Serialize)]
struct PathSummary {
    index: usize,
    end: String,
    at: SiteSummary,
    chain: Vec<SiteSummary>,
    conditions: Vec<ConditionSummary>,
    effects: Vec<EffectSummary>,
    player: String,
    final_tile: Option<TileSummary>,
    ticks: [u32; 2],
    ops: [u32; 2],
    world_reads_after_suspend: u32,
    modal: String,
    transfer_audit: TransferSummary,
    ints: Vec<String>,
    strings: Vec<String>,
}

#[derive(Clone, Serialize)]
struct SiteSummary {
    script: u32,
    script_name: Option<String>,
    pc: u32,
    source: String,
}
#[derive(Serialize)]
struct ConditionSummary {
    predicate: String,
    truth: bool,
    class: String,
    site: SiteSummary,
    choice: u32,
}

#[derive(Serialize)]
struct EffectSummary {
    effect: String,
    site: SiteSummary,
    ticks: [u32; 2],
}

#[derive(Serialize)]
struct CandidateSummary {
    path: usize,
    landing: Option<TileSummary>,
    covered: bool,
    selected: bool,
    edge: Option<EdgeSummary>,
    rejection: Option<String>,
    blocked_by: Vec<RivalSummary>,
    last_move: Option<String>,
    world_reads_after_suspend: u32,
    members_allocation: bool,
    members_sources: Vec<String>,
}

#[derive(Clone, Serialize)]
struct RivalSummary {
    path: usize,
    end: String,
    site: SiteSummary,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
struct TileSummary {
    x: i32,
    z: i32,
    level: i32,
}

#[derive(PartialEq, Eq, Serialize)]
struct EdgeSummary {
    kind: String,
    at: TileSummary,
    to: TileSummary,
    takeoff: Option<TileSummary>,
    player_delta: Option<TileSummary>,
    loc_id: i32,
    option: i32,
    ticks: i32,
    direction: Option<String>,
    open_loc_id: Option<i32>,
    skill_req: Vec<(i32, i32)>,
    item_req: Vec<(i32, i32)>,
    consumed_req: Vec<(i32, i32)>,
    item_returns: Vec<(i32, i32)>,
    quest_req: Vec<String>,
    varp_req: Vec<(i32, i32)>,
    worn_req: Vec<i32>,
    worn_all_req: Vec<i32>,
    forbidden_held: Vec<i32>,
    forbidden_worn: Vec<i32>,
    members_req: bool,
    wildy_cap: Option<i32>,
    quest_gates: Option<String>,
}

#[derive(Serialize)]
struct EdgeDiff {
    status: String,
    baseline: Option<EdgeSummary>,
    candidate: Option<EdgeSummary>,
    changed_fields: Vec<String>,
    loc_type: i32,
    placed_id: u32,
    stand: Option<TileSummary>,
    handler_source: Option<String>,
    last_move: Option<String>,
    blocked_by: Vec<RivalSummary>,
    world_reads_after_suspend: u32,
    members_allocation: bool,
    members_sources: Vec<String>,
}

#[derive(Default)]
struct RunPeaks {
    paths: Vec<EntryMeasurement>,
    instructions: Vec<EntryMeasurement>,
    bytes: Vec<EntryMeasurement>,
}

/// Real program, ordered base-world adapter, and non-mutating stage comparison.
pub(crate) struct NavShadow {
    program: Program,
    world: NavWorld,
    profile: EngineProfile,
    worn_slots: HashMap<i32, i32>,
    report: NavShadowReport,
    stage_started: Option<Instant>,
    peaks: RunPeaks,
}

impl NavShadow {
    pub(super) fn new(
        program: Program,
        world: NavWorld,
        profile: EngineProfile,
        revision: u16,
        content_root: &Path,
    ) -> Self {
        let objects = super::obj_ids_by_name(content_root);
        let constants = super::script_constants(content_root);
        let worn_slots = worn_slot_facts(content_root, &objects, &constants);
        let report = NavShadowReport {
            schema: 1,
            revision,
            world: WorldSummary {
                mapsquares: world.mapsquare_count(),
                zones: world.zone_count(),
                active_placements: world.placed_count(),
            },
            machine_entries: 0,
            stage_elapsed_ns: 0,
            entries: Vec::new(),
            diffs: Vec::new(),
            limits: LimitSummary {
                paths: BoundSummary::default(),
                instructions: BoundSummary::default(),
                bytes: BoundSummary::default(),
            },
        };
        Self {
            program,
            world,
            profile,
            worn_slots,
            report,
            stage_started: None,
            peaks: RunPeaks::default(),
        }
    }

    pub(super) fn begin_stage(&mut self) {
        self.stage_started = Some(Instant::now());
    }

    /// Run the procedure and scripted-door entries that have no legacy stage edge owner.
    pub(super) fn run_auxiliary_entries(
        &mut self,
        content_root: &Path,
        collision: &WorldCollision,
        observable: &super::ObservableGates,
    ) {
        self.run_climb_helper();
        let ids = super::loc_ids_by_name(content_root);
        for name in [
            "chefdoor",
            "crafting_guild_door",
            "loc_2025",
            "mournerstewfence",
        ] {
            let Some(loc_type) = ids
                .iter()
                .find_map(|(candidate, id)| candidate.eq_ignore_ascii_case(name).then_some(*id))
            else {
                continue;
            };
            let placements: Vec<Placement> = self
                .world
                .placements(loc_type)
                .iter()
                .map(|world_placement| world_placement.placement)
                .collect();
            for placement in placements {
                let at = WorldTile {
                    x: placement.x,
                    z: placement.z,
                    level: placement.level,
                };
                match placement.shape {
                    LocShape::WALL_STRAIGHT => {
                        let Some(angle_dir) = door_dir(placement.angle) else {
                            continue;
                        };
                        for direction in [angle_dir, opposite(angle_dir)] {
                            let stand = if direction == angle_dir {
                                at
                            } else {
                                let (dx, dz) = dir_delta(angle_dir);
                                WorldTile {
                                    x: at.x + dx,
                                    z: at.z + dz,
                                    level: at.level,
                                }
                            };
                            if !self.has_entry(loc_type, &placement, Some(stand)) {
                                self.run_candidate(
                                    loc_type,
                                    &placement,
                                    "wall",
                                    Some(direction),
                                    Some(stand),
                                    false,
                                    "machine-only scripted-door entry",
                                    &[],
                                    collision,
                                    observable,
                                );
                            }
                        }
                    }
                    LocShape::CENTREPIECE_STRAIGHT | LocShape::CENTREPIECE_DIAGONAL => {
                        if !self.has_entry(loc_type, &placement, None) {
                            self.run_candidate(
                                loc_type,
                                &placement,
                                "ladder",
                                None,
                                None,
                                false,
                                "machine-only scripted-door entry",
                                &[],
                                collision,
                                observable,
                            );
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    fn has_entry(&self, loc_type: i32, placement: &Placement, stand: Option<WorldTile>) -> bool {
        let Some(placed_id) = self
            .world
            .placements(loc_type)
            .iter()
            .find(|world_placement| world_placement.placement == *placement)
            .map(|world_placement| world_placement.placed_id.0)
        else {
            return false;
        };
        let stand = stand.map(tile_summary);
        self.report.entries.iter().any(|entry| {
            entry.loc_type == loc_type && entry.placed_id == placed_id && entry.stand == stand
        })
    }

    fn run_climb_helper(&mut self) {
        let script_id = self
            .program
            .scripts
            .values()
            .find(|script| script.name.eq_ignore_ascii_case("[proc,climb_ladder]"))
            .map(|script| script.id);
        let Some(script_id) = script_id else {
            self.report.entries.push(EntryReport {
                loc_type: -1,
                placed_id: 0,
                handler_script: None,
                handler_name: Some("climb_ladder".to_owned()),
                handler_source: None,
                role: "procedure".to_owned(),
                placement: None,
                stand: None,
                stage_owned: false,
                legacy_handler: "machine-only [proc,climb_ladder] entry".to_owned(),
                execution: empty_execution("script not found"),
            });
            return;
        };
        let Some(script) = self.program.script(script_id) else {
            self.report.entries.push(EntryReport {
                loc_type: -1,
                placed_id: 0,
                handler_script: Some(script_id.0),
                handler_name: Some("climb_ladder".to_owned()),
                handler_source: None,
                role: "procedure".to_owned(),
                placement: None,
                stand: None,
                stage_owned: false,
                legacy_handler: "machine-only [proc,climb_ladder] entry".to_owned(),
                execution: empty_execution("script id missing"),
            });
            return;
        };
        let handler_name = script.name.to_string();
        let handler_source = script.source.to_string();
        let mut entry_report = EntryReport {
            loc_type: -1,
            placed_id: 0,
            handler_script: Some(script_id.0),
            handler_name: Some(handler_name),
            handler_source: Some(handler_source),
            role: "procedure".to_owned(),
            placement: None,
            stand: None,
            stage_owned: false,
            legacy_handler: "machine-only [proc,climb_ladder] entry".to_owned(),
            execution: empty_execution("not run"),
        };
        self.report.machine_entries += 1;
        let started = Instant::now();
        let run = Machine::new(&self.program, self.profile, &self.world).run(
            Entry::new(script_id),
            View::NAV,
            rs2vm::machine::Limits::default(),
        );
        let elapsed_ns = started.elapsed().as_nanos();
        match run {
            Err(failure) => {
                entry_report.execution.status = format!("entry failure: {failure}");
                entry_report.execution.elapsed_ns = elapsed_ns;
                match failure.bound {
                    "paths" => {
                        entry_report.execution.paths = failure.count;
                        self.peaks.paths.push(entry_measurement(
                            &self.program,
                            -1,
                            0,
                            "procedure",
                            None,
                            Some(script_id),
                            failure.count,
                            true,
                        ));
                    }
                    "instructions" => {
                        entry_report.execution.instructions = failure.count;
                        self.peaks.instructions.push(entry_measurement(
                            &self.program,
                            -1,
                            0,
                            "procedure",
                            None,
                            Some(script_id),
                            failure.count,
                            true,
                        ));
                    }
                    "bytes" => {
                        entry_report.execution.bytes = failure.count;
                        self.peaks.bytes.push(entry_measurement(
                            &self.program,
                            -1,
                            0,
                            "procedure",
                            None,
                            Some(script_id),
                            failure.count,
                            true,
                        ));
                    }
                    _ => {}
                }
            }
            Ok(run) => {
                self.peaks.paths.push(entry_measurement(
                    &self.program,
                    -1,
                    0,
                    "procedure",
                    None,
                    Some(script_id),
                    run.stats.paths,
                    false,
                ));
                self.peaks.instructions.push(entry_measurement(
                    &self.program,
                    -1,
                    0,
                    "procedure",
                    None,
                    Some(script_id),
                    run.stats.instructions,
                    false,
                ));
                self.peaks.bytes.push(entry_measurement(
                    &self.program,
                    -1,
                    0,
                    "procedure",
                    None,
                    Some(script_id),
                    run.stats.bytes,
                    false,
                ));
                let transfer = transfer_summary(run.transfers);
                let assertions = transfer_assertions(&run);
                entry_report.execution = ExecutionReport {
                    status: if assertions.all_hold {
                        "completed".to_owned()
                    } else {
                        "transfer assertion failed".to_owned()
                    },
                    elapsed_ns,
                    paths: run.stats.paths,
                    instructions: run.stats.instructions,
                    bytes: run.stats.bytes,
                    transfer_audit: Some(transfer),
                    transfer_assertions: Some(assertions),
                    path_results: run
                        .paths
                        .iter()
                        .enumerate()
                        .map(|(index, path)| path_summary(index, path, &run, &self.program))
                        .collect(),
                    candidates: Vec::new(),
                };
            }
        }
        self.report.entries.push(entry_report);
    }

    /// Run one placed OPLOC1 entry and compare its projected candidates with the bake.
    pub(super) fn run_candidate(
        &mut self,
        loc_type: i32,
        placement: &Placement,
        role_name: &str,
        direction: Option<DoorDir>,
        stand: Option<WorldTile>,
        stage_owned: bool,
        legacy_handler: &str,
        baseline: &[TransportEdge],
        collision: &WorldCollision,
        observable: &super::ObservableGates,
    ) {
        let Some(placed_id) = self
            .world
            .placements(loc_type)
            .iter()
            .find(|world_placement| world_placement.placement == *placement)
            .map(|world_placement| world_placement.placed_id)
        else {
            return;
        };
        let script_id = self.program.trigger("OPLOC1").and_then(|trigger| {
            let category = self
                .program
                .packs
                .locs
                .get(&LocTypeId(loc_type))
                .and_then(|config| config.category);
            self.program.lookup(trigger, Some(loc_type), category)
        });
        let script = script_id.and_then(|id| self.program.script(id));
        let mut entry_report = EntryReport {
            loc_type,
            placed_id: placed_id.0,
            handler_script: script_id.map(|id| id.0),
            handler_name: script.map(|script| script.name.to_string()),
            handler_source: script.map(|script| script.source.to_string()),
            role: role_name.to_owned(),
            placement: Some(placement_summary(placement)),
            stand: stand.map(tile_summary),
            stage_owned,
            legacy_handler: legacy_handler.to_owned(),
            execution: empty_execution("missing OPLOC1 handler"),
        };
        let Some(script_id) = script_id else {
            self.report.entries.push(entry_report);
            return;
        };
        let Some(script_source) = entry_report.handler_source.as_deref() else {
            entry_report.execution.status = "lookup returned a missing script".to_owned();
            self.report.entries.push(entry_report);
            return;
        };
        let mut entry = Entry::new(script_id);
        entry.active_loc = Some(LocObj::Placed(placed_id));
        entry.stand = stand.map(|tile| Coord::new(tile.x, tile.level, tile.z));
        self.report.machine_entries += 1;
        let run_started = Instant::now();
        let machine = Machine::new(&self.program, self.profile, &self.world);
        let run = machine.run(entry, View::NAV, rs2vm::machine::Limits::default());
        let elapsed_ns = run_started.elapsed().as_nanos();
        match run {
            Err(failure) => {
                entry_report.execution.status = format!("entry failure: {failure}");
                entry_report.execution.elapsed_ns = elapsed_ns;
                let value = failure.count;
                match failure.bound {
                    "paths" => {
                        entry_report.execution.paths = value;
                        self.peaks.paths.push(entry_measurement(
                            &self.program,
                            loc_type,
                            placed_id.0,
                            role_name,
                            stand,
                            Some(script_id),
                            value,
                            true,
                        ));
                    }
                    "instructions" => {
                        entry_report.execution.instructions = value;
                        self.peaks.instructions.push(entry_measurement(
                            &self.program,
                            loc_type,
                            placed_id.0,
                            role_name,
                            stand,
                            Some(script_id),
                            value,
                            true,
                        ));
                    }
                    "bytes" => {
                        entry_report.execution.bytes = value;
                        self.peaks.bytes.push(entry_measurement(
                            &self.program,
                            loc_type,
                            placed_id.0,
                            role_name,
                            stand,
                            Some(script_id),
                            value,
                            true,
                        ));
                    }
                    _ => {}
                }
            }
            Ok(run) => {
                self.peaks.paths.push(entry_measurement(
                    &self.program,
                    loc_type,
                    placed_id.0,
                    role_name,
                    stand,
                    Some(script_id),
                    run.stats.paths,
                    false,
                ));
                self.peaks.instructions.push(entry_measurement(
                    &self.program,
                    loc_type,
                    placed_id.0,
                    role_name,
                    stand,
                    Some(script_id),
                    run.stats.instructions,
                    false,
                ));
                self.peaks.bytes.push(entry_measurement(
                    &self.program,
                    loc_type,
                    placed_id.0,
                    role_name,
                    stand,
                    Some(script_id),
                    run.stats.bytes,
                    false,
                ));
                let transfer = transfer_summary(run.transfers);
                let assertions = transfer_assertions(&run);
                let candidates = self.candidates(
                    &run,
                    loc_type,
                    placement,
                    placed_id,
                    role_name,
                    direction,
                    stand,
                    stage_owned,
                    baseline,
                    collision,
                    observable,
                    script_source,
                );
                entry_report.execution = ExecutionReport {
                    status: if assertions.all_hold {
                        "completed".to_owned()
                    } else {
                        "transfer assertion failed".to_owned()
                    },
                    elapsed_ns,
                    paths: run.stats.paths,
                    instructions: run.stats.instructions,
                    bytes: run.stats.bytes,
                    transfer_audit: Some(transfer),
                    transfer_assertions: Some(assertions),
                    path_results: run
                        .paths
                        .iter()
                        .enumerate()
                        .map(|(i, path)| path_summary(i, path, &run, &self.program))
                        .collect(),
                    candidates,
                };
            }
        }
        self.report.entries.push(entry_report);
    }

    fn candidates(
        &mut self,
        run: &Run,
        loc_type: i32,
        placement: &Placement,
        placed_id: PlacedId,
        role_name: &str,
        direction: Option<DoorDir>,
        stand: Option<WorldTile>,
        stage_owned: bool,
        baseline: &[TransportEdge],
        collision: &WorldCollision,
        observable: &super::ObservableGates,
        handler_source: &str,
    ) -> Vec<CandidateSummary> {
        let at = WorldTile {
            x: placement.x,
            z: placement.z,
            level: placement.level,
        };
        let angle_dir = door_dir(placement.angle);
        let expected_wall_landing = match (role_name, direction, angle_dir) {
            ("wall", Some(direction), Some(angle_dir)) => {
                straight_door_landing(at, angle_dir, direction, collision)
            }
            _ => None,
        };
        let ladder_ticks = extra_ticks("ladder_cellar").and_then(edge_ticks);
        let mut projected_edges = Vec::new();
        let mut results = Vec::with_capacity(run.paths.len());
        for (index, path) in run.paths.iter().enumerate() {
            let landing = candidate_landing(path, role_name, at, stand);
            let mut row = CandidateSummary {
                path: index,
                landing: landing.map(|(tile, _)| tile_summary(tile)),
                covered: false,
                selected: is_selected(run, path, &self.program, &self.worn_slots, observable),
                edge: None,
                rejection: None,
                blocked_by: Vec::new(),
                last_move: last_move_source(path, &self.program),
                world_reads_after_suspend: path.world_reads_after_suspend,
                members_allocation: false,
                members_sources: Vec::new(),
            };
            let Some((landing, delta)) = landing else {
                row.rejection = Some(landing_rejection(path));
                results.push(row);
                continue;
            };
            if !path_effects_admitted(path) {
                row.rejection = Some("movement path violates the effect rule".to_owned());
                results.push(row);
                continue;
            }
            if landing
                == stand.unwrap_or(WorldTile {
                    x: i32::MIN,
                    z: i32::MIN,
                    level: i32::MIN,
                })
            {
                row.rejection = Some("landing is the stand".to_owned());
                results.push(row);
                continue;
            }
            let expected = match role_name {
                "wall" => expected_wall_landing,
                "ladder" => in_world_box(&landing).then_some(landing),
                _ => None,
            };
            if expected != Some(landing) {
                row.rejection =
                    Some("landing does not satisfy the role's consumer rule".to_owned());
                results.push(row);
                continue;
            }
            let same = |rival: &PathResult| {
                matches!(rival.end, End::Finished | End::AwaitingInput)
                    && path_effects_admitted(rival)
                    && candidate_landing(rival, role_name, at, stand)
                        .is_some_and(|(other, _)| other == landing)
            };
            row.covered = run.covered(index, same);
            if !row.covered {
                let rivals = run.rivals(&path.conds);
                for (rival_index, rival) in run.paths.iter().enumerate() {
                    if rivals
                        .iter()
                        .any(|candidate| std::ptr::eq(*candidate, rival))
                        && !same(rival)
                    {
                        row.blocked_by.push(RivalSummary {
                            path: rival_index,
                            end: format!("{:?}", rival.end),
                            site: site_summary(rival.at, &self.program),
                        });
                    }
                }
                row.rejection =
                    Some("an overlapping rival does not preserve this landing".to_owned());
                results.push(row);
                continue;
            }
            if !row.selected {
                row.rejection =
                    Some("candidate is not selected by the legacy threshold rule".to_owned());
                results.push(row);
                continue;
            }
            let generalized = run.generalise(index, same, |condition| {
                condition_order(run, condition, &self.program, &self.worn_slots, observable)
            });
            let mut needs = Needs::default();
            let mut fields_complete = true;
            for condition in &generalized {
                match condition_needs(run, condition, &self.program, &self.worn_slots, observable) {
                    Some(mut condition_needs) => needs.merge(std::mem::take(&mut condition_needs)),
                    None => {
                        fields_complete = false;
                        row.rejection = Some(format!(
                            "condition has no consumer field: {:?} at {}",
                            condition.pred,
                            self.program.format_site(condition.site)
                        ));
                        break;
                    }
                }
            }
            if !fields_complete {
                results.push(row);
                continue;
            }
            let member_sources = members_sources(run, &generalized, &self.program);
            row.members_allocation = member_sources
                .iter()
                .any(|source| source.starts_with("allocation: "));
            row.members_sources = member_sources;
            let kind = if role_name == "wall" {
                TransportKind::Door
            } else {
                TransportKind::Ladder
            };
            let mut edge = gated_edge(kind, loc_type, at, landing, needs.clone());
            if let Some(direction) = direction {
                edge.dir = Some(direction);
                edge.open_loc_id = opened_type(path, LocObj::Placed(placed_id), loc_type);
            } else {
                let relative_safe =
                    delta.is_some() && needs == Needs::default() && relative_effects_safe(path);
                if relative_safe {
                    edge.player_delta = delta;
                }
                let Some(ticks) = ladder_ticks else {
                    row.rejection = Some("ladder price is unavailable".to_owned());
                    results.push(row);
                    continue;
                };
                edge.ticks = ticks;
            }
            let mut admitted_graph = nav_planner::transport::TransportGraph::default();
            let mut admission_audit = super::VarpGateAudit::default();
            if !observable.admit_edge(&mut admitted_graph, edge, &mut admission_audit) {
                row.rejection = Some("observable-gate consumer refused the edge".to_owned());
                results.push(row);
                continue;
            }
            let Some(edge) = admitted_graph.edges.pop() else {
                row.rejection = Some("consumer admitted no graph edge".to_owned());
                results.push(row);
                continue;
            };
            row.edge = Some(edge_summary(&edge));
            projected_edges.push((index, edge));
            results.push(row);
        }
        if stage_owned {
            self.compare_stand_edges(
                loc_type,
                placed_id,
                stand,
                direction,
                baseline,
                &projected_edges,
                &results,
                handler_source,
            );
        }
        results
    }

    fn compare_stand_edges(
        &mut self,
        loc_type: i32,
        placed_id: PlacedId,
        stand: Option<WorldTile>,
        direction: Option<DoorDir>,
        baseline: &[TransportEdge],
        candidates: &[(usize, TransportEdge)],
        results: &[CandidateSummary],
        handler_source: &str,
    ) {
        let mut relevant: Vec<&TransportEdge> = baseline
            .iter()
            .filter(|edge| {
                edge.loc_id == loc_type
                    && edge.option == 1
                    && edge.takeoff.is_none()
                    && edge.dir == direction
                    && matches!(edge.kind, TransportKind::Door | TransportKind::Ladder)
            })
            .collect();
        for (path, candidate) in candidates {
            let candidate_row = results.iter().find(|row| row.path == *path);
            let candidate_summary = edge_summary(candidate);
            let same = relevant
                .iter()
                .position(|edge| edge_identity_eq(edge, candidate));
            let baseline_edge = same.map(|index| relevant.remove(index));
            let baseline_summary = baseline_edge.map(edge_summary);
            let status = match baseline_summary.as_ref() {
                Some(summary) if summary == &candidate_summary => "unchanged",
                Some(_) => "changed",
                None => "added",
            };
            self.report.diffs.push(EdgeDiff {
                status: status.to_owned(),
                changed_fields: baseline_summary.as_ref().map_or_else(Vec::new, |summary| {
                    edge_changed_fields(summary, &candidate_summary)
                }),
                baseline: baseline_summary,
                candidate: Some(candidate_summary),
                loc_type,
                placed_id: placed_id.0,
                stand: stand.map(tile_summary),
                handler_source: Some(handler_source.to_owned()),
                last_move: candidate_row.and_then(|row| row.last_move.clone()),
                blocked_by: candidate_row.map_or_else(Vec::new, |row| row.blocked_by.clone()),
                world_reads_after_suspend: candidate_row
                    .map_or(0, |row| row.world_reads_after_suspend),
                members_allocation: candidate_row.is_some_and(|row| row.members_allocation),
                members_sources: candidate_row
                    .map_or_else(Vec::new, |row| row.members_sources.clone()),
            });
        }
        let mut blocked_by = Vec::new();
        for row in results {
            for rival in &row.blocked_by {
                if !blocked_by
                    .iter()
                    .any(|seen: &RivalSummary| seen.path == rival.path)
                {
                    blocked_by.push(rival.clone());
                }
            }
        }
        let world_reads_after_suspend = results
            .iter()
            .map(|row| row.world_reads_after_suspend)
            .max()
            .unwrap_or(0);
        let last_move = results
            .iter()
            .find(|row| !row.blocked_by.is_empty())
            .and_then(|row| row.last_move.clone());
        for edge in relevant {
            self.report.diffs.push(EdgeDiff {
                status: "removed".to_owned(),
                baseline: Some(edge_summary(edge)),
                candidate: None,
                changed_fields: Vec::new(),
                loc_type,
                placed_id: placed_id.0,
                stand: stand.map(tile_summary),
                handler_source: Some(handler_source.to_owned()),
                last_move: last_move.clone(),
                blocked_by: blocked_by.clone(),
                world_reads_after_suspend,
                members_allocation: false,
                members_sources: Vec::new(),
            });
        }
    }

    pub(super) fn finish(mut self) -> Result<String, serde_json::Error> {
        self.report.stage_elapsed_ns = self
            .stage_started
            .take()
            .map_or(0, |started| started.elapsed().as_nanos());
        self.report.limits = LimitSummary {
            paths: bound_summary(self.peaks.paths),
            instructions: bound_summary(self.peaks.instructions),
            bytes: bound_summary(self.peaks.bytes),
        };
        serde_json::to_string_pretty(&self.report)
    }

    #[cfg(test)]
    pub(super) fn world(&self) -> &NavWorld {
        &self.world
    }
}

fn empty_execution(status: &str) -> ExecutionReport {
    ExecutionReport {
        status: status.to_owned(),
        elapsed_ns: 0,
        paths: 0,
        instructions: 0,
        bytes: 0,
        transfer_audit: None,
        transfer_assertions: None,
        path_results: Vec::new(),
        candidates: Vec::new(),
    }
}

fn entry_measurement(
    program: &Program,
    loc_type: i32,
    placed_id: u32,
    role: &str,
    stand: Option<WorldTile>,
    script_id: Option<rs2vm::ScriptId>,
    value: u64,
    bound_reached: bool,
) -> EntryMeasurement {
    let script = script_id.and_then(|id| program.script(id));
    EntryMeasurement {
        value,
        loc_type,
        placed_id,
        handler_script: script_id.map(|id| id.0),
        handler_name: script.map(|script| script.name.to_string()),
        handler_source: script.map(|script| script.source.to_string()),
        role: role.to_owned(),
        stand: stand.map(tile_summary),
        bound_reached,
    }
}

fn transfer_summary(audit: rs2vm::machine::TransferAudit) -> TransferSummary {
    TransferSummary {
        cancellations: audit.cancellations,
        automatic_closes: audit.automatic_closes,
        suspensions: audit.suspensions,
        loop_summaries: audit.loop_summaries,
        loop_allowance_before_exit: audit.loop_allowance_before_exit,
        unsupported_prefixes: audit.unsupported_prefixes,
    }
}

fn transfer_assertions(run: &Run) -> TransferAssertions {
    let unsupported_prefix_count_matches = run.paths.iter().all(|path| {
        (path.transfers.unsupported_prefixes > 0)
            == matches!(path.end, End::Unsupported(_))
    });
    let each_input_pause_was_cancelled = run.paths.iter().all(|path| {
        (path.transfers.cancellations > 0) == matches!(path.end, End::AwaitingInput)
    });
    let each_loop_summary_has_pre_exit_allowance = run.paths.iter().all(|path| {
        path.transfers.loop_allowance_before_exit >= path.transfers.loop_summaries
    });
    let automatic_closes_do_not_exceed_suspensions = run.paths.iter().all(|path| {
        path.transfers.automatic_closes <= path.transfers.suspensions
    });
    TransferAssertions {
        unsupported_prefix_count_matches,
        each_input_pause_was_cancelled,
        each_loop_summary_has_pre_exit_allowance,
        automatic_closes_do_not_exceed_suspensions,
        all_hold: unsupported_prefix_count_matches
            && each_input_pause_was_cancelled
            && each_loop_summary_has_pre_exit_allowance
            && automatic_closes_do_not_exceed_suspensions,
    }
}

fn path_summary(index: usize, path: &PathResult, run: &Run, program: &Program) -> PathSummary {
    PathSummary {
        index,
        end: format!("{:?}", path.end),
        at: site_summary(path.at, program),
        chain: path
            .chain
            .iter()
            .copied()
            .map(|site| site_summary(site, program))
            .collect(),
        conditions: path
            .conds
            .iter()
            .map(|condition| ConditionSummary {
                predicate: predicate_summary(&condition.pred, run, program),
                truth: condition.truth,
                class: format!("{:?}", condition.class),
                site: site_summary(condition.site, program),
                choice: condition.choice,
            })
            .collect(),
        effects: path
            .effects
            .iter()
            .map(|effect| EffectSummary {
                effect: format!("{:?}", effect.effect),
                site: site_summary(effect.site, program),
                ticks: [effect.ticks.min, effect.ticks.max],
            })
            .collect(),
        player: format!("{:?}", path.player),
        final_tile: match path.player {
            PlayerPos::Tile(tile) => Some(coord_summary(tile)),
            _ => None,
        },
        ticks: [path.ticks.min, path.ticks.max],
        ops: [path.ops.min, path.ops.max],
        world_reads_after_suspend: path.world_reads_after_suspend,
        modal: format!("{:?}", path.modal),
        transfer_audit: transfer_summary(path.transfers),
        ints: path.ints.iter().map(|value| format!("{value:?}")).collect(),
        strings: path
            .strings
            .iter()
            .map(|value| format!("{value:?}"))
            .collect(),
    }
}

fn predicate_summary(pred: &Pred, run: &Run, program: &Program) -> String {
    match pred {
        Pred::Cmp { lhs, op, rhs } => format!(
            "{} {:?} {}",
            value_summary(*lhs, run, program, 0),
            op,
            value_summary(*rhs, run, program, 0)
        ),
        Pred::In { of, set } => format!("{} in {:?}", value_summary(*of, run, program, 0), set),
    }
}

fn value_summary(value: Value, run: &Run, program: &Program, depth: u8) -> String {
    match value {
        Value::Known(value) => format!("Known({value})"),
        Value::Sym(id) => expression_summary(id, run, program, depth),
        Value::Opaque(site) => format!("Opaque({})", program.format_site(site)),
    }
}

fn expression_summary(
    id: rs2vm::machine::ExprId,
    run: &Run,
    program: &Program,
    depth: u8,
) -> String {
    if depth >= 16 {
        return format!("ExprId({})", id.0);
    }
    let Some(expr) = run.arena.expr(id) else {
        return format!("ExprId({})", id.0);
    };
    match expr {
        Expr::Atom(atom) => run
            .arena
            .atom(*atom)
            .map(|atom| format!("{:?}", atom.kind))
            .unwrap_or_else(|| format!("AtomId({})", atom.0)),
        Expr::AddConst { of, c } => format!(
            "AddConst({}, {c})",
            expression_summary(*of, run, program, depth + 1)
        ),
        Expr::Bits { of, lo, hi } => format!(
            "Bits({}, {lo}..={hi})",
            expression_summary(*of, run, program, depth + 1)
        ),
        Expr::WithBits { of, lo, hi, to } => format!(
            "WithBits({}, {lo}..={hi}, {})",
            expression_summary(*of, run, program, depth + 1),
            value_summary(*to, run, program, depth + 1)
        ),
        Expr::CoordPlus { of, dx, dlevel, dz } => format!(
            "CoordPlus({}, {dx}, {dlevel}, {dz})",
            expression_summary(*of, run, program, depth + 1)
        ),
        Expr::CoordPart { of, axis } => format!(
            "CoordPart({}, {axis:?})",
            expression_summary(*of, run, program, depth + 1)
        ),
    }
}

fn site_summary(site: Site, program: &Program) -> SiteSummary {
    SiteSummary {
        script: site.script.0,
        script_name: program
            .script(site.script)
            .map(|script| script.name.to_string()),
        pc: site.pc,
        source: program.format_site(site),
    }
}

fn last_move_source(path: &PathResult, program: &Program) -> Option<String> {
    path.effects
        .iter()
        .rev()
        .find(|record| matches!(record.effect, Effect::Move { .. }))
        .map(|record| program.format_site(record.site))
}

fn landing_rejection(path: &PathResult) -> String {
    let movement = path
        .effects
        .iter()
        .rev()
        .find_map(|record| match record.effect {
            Effect::Move { to, .. } => Some(to),
            _ => None,
        });
    landing_rejection_reason(movement).to_owned()
}

fn landing_rejection_reason(movement: Option<PlayerPos>) -> &'static str {
    match movement {
        Some(PlayerPos::Unknown(_)) => {
            "unknown typed coordinate is not a usable navigation landing"
        }
        Some(PlayerPos::Near(_)) => {
            "Near landing has no supported outcome in the navigation consumer"
        }
        Some(_) => "no known landing from the last Move effect",
        None => "path has no Move effect",
    }
}

fn candidate_landing(
    path: &PathResult,
    role: &str,
    at: WorldTile,
    stand: Option<WorldTile>,
) -> Option<(WorldTile, Option<WorldTile>)> {
    if !matches!(path.end, End::Finished | End::AwaitingInput) {
        return None;
    }
    let movement = path.effects.iter().rev().find_map(|record| {
        if let Effect::Move { to, .. } = &record.effect {
            Some(*to)
        } else {
            None
        }
    })?;
    candidate_landing_from_position(role, movement, at, stand)
}

fn candidate_landing_from_position(
    role: &str,
    movement: PlayerPos,
    at: WorldTile,
    stand: Option<WorldTile>,
) -> Option<(WorldTile, Option<WorldTile>)> {
    match (role, movement) {
        ("wall", PlayerPos::Tile(coord)) => Some((tile_of(coord), None)),
        ("wall", PlayerPos::Rel { dx, dlevel, dz }) => {
            let stand = stand?;
            let coord = Coord::new(stand.x, stand.level, stand.z).offset(dx, dlevel, dz);
            Some((tile_of(coord), None))
        }
        ("ladder", PlayerPos::Tile(coord)) => Some((tile_of(coord), None)),
        (
            "ladder",
            PlayerPos::Rel {
                dx: 0,
                dlevel,
                dz: 0,
            },
        ) => {
            let delta = WorldTile {
                x: 0,
                z: 0,
                level: dlevel,
            };
            Some((offset_tile(at, delta), Some(delta)))
        }
        (
            "ladder",
            PlayerPos::Rel {
                dx: 0,
                dlevel: 0,
                dz,
            },
        ) if dz == nav_planner::transport::CELLAR_SHIFT
            || dz == -nav_planner::transport::CELLAR_SHIFT =>
        {
            let delta = WorldTile {
                x: 0,
                z: dz,
                level: 0,
            };
            Some((offset_tile(at, delta), Some(delta)))
        }
        _ => None,
    }
}

fn tile_of(coord: Coord) -> WorldTile {
    WorldTile {
        x: coord.x(),
        z: coord.z(),
        level: coord.level(),
    }
}

fn offset_tile(tile: WorldTile, delta: WorldTile) -> WorldTile {
    let coord = Coord::new(tile.x, tile.level, tile.z).offset(delta.x, delta.level, delta.z);
    tile_of(coord)
}

fn path_effects_admitted(path: &PathResult) -> bool {
    let Some(last_move) = path
        .effects
        .iter()
        .rposition(|record| matches!(record.effect, Effect::Move { .. }))
    else {
        return false;
    };
    path.effects.iter().enumerate().all(|(index, record)| {
        let after = index > last_move;
        match &record.effect {
            Effect::Move { .. }
            | Effect::NoMove { .. }
            | Effect::Delay { .. }
            | Effect::Loc { .. }
            | Effect::Message { .. }
            | Effect::Cosmetic { .. }
            | Effect::Npc { .. }
            | Effect::Obj { .. }
            | Effect::Modal { .. }
            | Effect::WeakQueueCleared => true,
            Effect::VarWrite { .. } => after,
            Effect::Defer { kind, .. } => after && *kind == DeferKind::Cancel,
            Effect::Inv { .. } => false,
        }
    })
}

fn relative_effects_safe(path: &PathResult) -> bool {
    path.effects.iter().all(|record| {
        matches!(
            record.effect,
            Effect::Move { .. }
                | Effect::Delay { .. }
                | Effect::Message { .. }
                | Effect::Cosmetic { .. }
        )
    })
}

fn condition_order(
    run: &Run,
    condition: &Cond,
    program: &Program,
    worn_slots: &HashMap<i32, i32>,
    observable: &super::ObservableGates,
) -> u32 {
    if condition_needs(run, condition, program, worn_slots, observable).is_none() {
        return 0;
    }
    let Some((value, _, _)) = comparison(condition) else {
        return 0;
    };
    let Some(atom) = direct_atom(run, value) else {
        return 0;
    };
    match atom.kind {
        rs2vm::machine::AtomKind::InvTotal { .. } | rs2vm::machine::AtomKind::InvSlot { .. } => 1,
        rs2vm::machine::AtomKind::Stat { .. } | rs2vm::machine::AtomKind::Members => 2,
        rs2vm::machine::AtomKind::Varp(_) => 3,
        _ => 0,
    }
}

fn condition_needs(
    run: &Run,
    condition: &Cond,
    program: &Program,
    worn_slots: &HashMap<i32, i32>,
    observable: &super::ObservableGates,
) -> Option<Needs> {
    let (value, operator, constant) = comparison(condition)?;
    let atom = direct_atom(run, value)?;
    if selection_truth(run, condition, program, worn_slots, observable)? != condition.truth {
        return None;
    }
    let effective = if condition.truth {
        operator
    } else {
        operator.negate()
    };
    let mut needs = Needs::default();
    match atom.kind {
        rs2vm::machine::AtomKind::Varp(id) => {
            if let Some(window) = observable.stage_window(i32::from(id.0)) {
                let window = stage_window_requirement(window, effective, constant)?;
                needs.quests.push(api::selected::QuestGate::Window(window));
                return Some(needs);
            }
            let varp = program.packs.varps.get(&id)?;
            if !varp.transmit {
                return None;
            }
            let minimum = lower_bound(effective, constant)?;
            if minimum > 0 {
                needs.varps.push((i32::from(id.0), minimum));
            }
            Some(needs)
        }
        rs2vm::machine::AtomKind::Stat { id, .. } => {
            let minimum = lower_bound(effective, constant)?;
            if minimum > 0 {
                needs.skills.push((id, minimum));
            }
            Some(needs)
        }
        rs2vm::machine::AtomKind::Members => {
            needs.members = true;
            Some(needs)
        }
        rs2vm::machine::AtomKind::InvTotal { inv, obj } => {
            let worn = inventory_is_worn(program, inv);
            let (minimum, forbidden) = inventory_requirement(effective, constant)?;
            if forbidden {
                if worn {
                    needs.forbidden_worn.push(obj);
                } else {
                    needs.forbidden_held.push(obj);
                }
            } else if minimum > 0 {
                if worn {
                    needs.worn.push((obj, minimum));
                } else {
                    needs.items.push((obj, minimum));
                }
            }
            Some(needs)
        }
        rs2vm::machine::AtomKind::InvSlot { inv, slot } => {
            if !inventory_is_worn(program, inv) {
                return None;
            }
            let object = constant;
            if object <= 0 || worn_slots.get(&object) != Some(&slot) {
                return None;
            }
            match (operator, condition.truth) {
                (rs2vm::machine::CmpOp::Eq, true) | (rs2vm::machine::CmpOp::Ne, false) => {
                    needs.worn.push((object, 1))
                }
                (rs2vm::machine::CmpOp::Eq, false) | (rs2vm::machine::CmpOp::Ne, true) => {
                    needs.forbidden_worn.push(object)
                }
                _ => return None,
            }
            Some(needs)
        }
        _ => None,
    }
}

fn comparison(condition: &Cond) -> Option<(Value, rs2vm::machine::CmpOp, i32)> {
    let Pred::Cmp { lhs, op, rhs } = &condition.pred else {
        return None;
    };
    match (*lhs, *rhs) {
        (Value::Sym(_), Value::Known(value)) => Some((*lhs, *op, value)),
        (Value::Known(value), Value::Sym(_)) => Some((*rhs, op.reverse(), value)),
        _ => None,
    }
}

fn direct_atom(run: &Run, value: Value) -> Option<&rs2vm::machine::Atom> {
    let Value::Sym(expression) = value else {
        return None;
    };
    let Expr::Atom(atom) = run.arena.expr(expression)? else {
        return None;
    };
    run.arena.atom(*atom)
}

fn command_at(program: &Program, site: Site) -> Option<&str> {
    let instruction = program.script(site.script)?.code.get(site.pc as usize)?;
    program.opcode(instruction.opcode)
}

fn members_sources(run: &Run, conditions: &[Cond], program: &Program) -> Vec<String> {
    let mut sources = Vec::new();
    for condition in conditions {
        let Some((value, _, _)) = comparison(condition) else {
            continue;
        };
        let Some(atom) = direct_atom(run, value) else {
            continue;
        };
        if atom.kind != rs2vm::machine::AtomKind::Members {
            continue;
        }
        let allocation = atom.site == Site::default();
        let site = if allocation {
            condition.site
        } else {
            atom.site
        };
        let command = command_at(program, site).unwrap_or("unknown command");
        let source_kind = if allocation { "allocation" } else { "script" };
        sources.push(format!(
            "{source_kind}: {command} at {}",
            program.format_site(site)
        ));
    }
    sources.sort();
    sources.dedup();
    sources
}

fn stage_window_requirement(
    window: &api::selected::StageWindow,
    operator: rs2vm::machine::CmpOp,
    value: i32,
) -> Option<api::selected::StageWindow> {
    let mut selected = window.clone();
    let minimum = selected.values.min?;
    let maximum = selected.values.max?;
    if operator == rs2vm::machine::CmpOp::Eq {
        if !(minimum..=maximum).contains(&value) {
            return None;
        }
        selected.values.min = Some(value);
        selected.values.max = Some(value);
        return Some(selected);
    }
    if operator == rs2vm::machine::CmpOp::Ne && (value < minimum || value > maximum) {
        return Some(selected);
    }
    let minimum = lower_bound(operator, value)?.max(minimum);
    if minimum > maximum {
        return None;
    }
    selected.values.min = Some(minimum);
    Some(selected)
}

fn lower_bound(operator: rs2vm::machine::CmpOp, value: i32) -> Option<i32> {
    use rs2vm::machine::CmpOp::{Ge, Gt, Ne};
    match operator {
        Ge => Some(value),
        Gt => value.checked_add(1),
        Ne if value < 0 => Some(0),
        Ne if value == 0 => Some(1),
        Ne => value.checked_add(1),
        _ => None,
    }
}

fn inventory_requirement(operator: rs2vm::machine::CmpOp, value: i32) -> Option<(i32, bool)> {
    use rs2vm::machine::CmpOp::{Eq, Ge, Gt, Le, Lt, Ne};
    match operator {
        Ge => Some((value.max(0), false)),
        Gt => value.checked_add(1).map(|minimum| (minimum.max(0), false)),
        Ne if value < 0 => Some((0, false)),
        Ne if value == 0 => Some((1, false)),
        Ne => value.checked_add(1).map(|minimum| (minimum, false)),
        Eq if value == 0 => Some((0, true)),
        Eq => None,
        Le if value == 0 => Some((0, true)),
        Lt if value == 1 => Some((0, true)),
        Lt | Le => None,
    }
}

fn inventory_is_worn(program: &Program, id: i32) -> bool {
    program
        .packs
        .invs
        .get(&id)
        .is_some_and(|name| name.eq_ignore_ascii_case("worn"))
}

fn selection_truth(
    run: &Run,
    condition: &Cond,
    program: &Program,
    worn_slots: &HashMap<i32, i32>,
    observable: &super::ObservableGates,
) -> Option<bool> {
    let (value, operator, constant) = comparison(condition)?;
    let atom = direct_atom(run, value)?;
    use rs2vm::machine::{AtomKind, CmpOp};
    match atom.kind {
        AtomKind::Varp(id)
            if observable.stage_window(i32::from(id.0)).is_some()
                && operator == CmpOp::Eq
                && constant != 0 =>
        {
            Some(true)
        }
        AtomKind::Varp(_) | AtomKind::Stat { .. } | AtomKind::InvTotal { .. } => match operator {
            CmpOp::Ge | CmpOp::Gt | CmpOp::Ne => Some(true),
            CmpOp::Le | CmpOp::Lt | CmpOp::Eq => Some(false),
        },
        AtomKind::Members if operator == CmpOp::Eq && constant == 1 => Some(true),
        AtomKind::Members if operator == CmpOp::Eq && constant == 0 => Some(false),
        AtomKind::InvSlot { inv, slot }
            if inventory_is_worn(program, inv)
                && operator == CmpOp::Eq
                && constant > 0
                && worn_slots.get(&constant) == Some(&slot) =>
        {
            Some(true)
        }
        _ => None,
    }
}

fn is_selected(
    run: &Run,
    path: &PathResult,
    program: &Program,
    worn_slots: &HashMap<i32, i32>,
    observable: &super::ObservableGates,
) -> bool {
    path.conds
        .iter()
        .filter(|condition| matches!(condition.class, CondClass::Requirement | CondClass::Hidden))
        .all(
            |condition| match selection_truth(run, condition, program, worn_slots, observable) {
                Some(required_truth) => condition.truth == required_truth,
                None => true,
            },
        )
}

fn opened_type(
    effects: &[rs2vm::machine::EffectRecord],
    target: LocObj,
    closed_id: i32,
) -> Option<i32> {
    let closed = effects.iter().enumerate().rev().find_map(|(index, record)| {
        match &record.effect {
            Effect::Loc {
                kind: LocKind::Del,
                obj,
                ty,
                tile,
                shape,
                angle,
                ..
            } if *obj == target && ty.0 == closed_id => Some((index, *tile, *shape, *angle)),
            _ => None,
        }
    });
    if let Some((delete_index, target_coord, shape, angle)) = closed {
        if let Some(open_coord) = open_leaf_coord(target_coord, shape, angle) {
            let mut opened_type = None;
            for record in &effects[delete_index + 1..] {
                if let Effect::Loc {
                    kind: LocKind::Add,
                    obj: LocObj::Created(_),
                    ty,
                    tile,
                    ..
                } = &record.effect
                {
                    if *tile != open_coord || ty.0 == closed_id {
                        continue;
                    }
                    match opened_type {
                        Some(previous) if previous != ty.0 => return None,
                        None => opened_type = Some(ty.0),
                        Some(_) => {}
                    }
                }
            }
            if opened_type.is_some() {
                return opened_type;
            }
        }
        return effects[delete_index + 1..]
            .iter()
            .rev()
            .find_map(|record| match &record.effect {
                Effect::Loc {
                    kind: LocKind::Change,
                    obj,
                    ty,
                    tile,
                    ..
                } if *obj == target && *tile == target_coord && ty.0 != closed_id => {
                    Some(ty.0)
                }
                _ => None,
            });
    }
    effects.iter().rev().find_map(|record| match &record.effect {
        Effect::Loc {
            kind: LocKind::Add | LocKind::Change,
            obj,
            ty,
            ..
        } if *obj == target && ty.0 != closed_id => Some(ty.0),
        _ => None,
    })
}

// `doubledoors.rs2:133-141` adds the inviswall at `loc_coord`, then the
// open leaf at `door_open(loc_angle, loc_shape)`; `door_procs.rs2:1-19`
// defines that offset. Each emitted loc effect retains its source site.
fn open_leaf_coord(tile: Coord, shape: u8, angle: u8) -> Option<Coord> {
    let angle = i32::from(angle);
    let direction = match i32::from(shape) {
        LocShape::WALL_STRAIGHT => door_dir(angle)?,
        LocShape::WALL_DIAGONAL => door_dir((angle + 1) & 3)?,
        _ => return None,
    };
    let (dx, dz) = dir_delta(direction);
    Some(tile.offset(dx, 0, dz))
}

fn edge_identity_eq(left: &TransportEdge, right: &TransportEdge) -> bool {
    left.loc_id == right.loc_id && left.at == right.at && left.to == right.to
}

fn edge_changed_fields(left: &EdgeSummary, right: &EdgeSummary) -> Vec<String> {
    let mut fields = Vec::new();
    macro_rules! changed {
        ($field:ident) => {
            if left.$field != right.$field {
                fields.push(stringify!($field).to_owned());
            }
        };
    }
    changed!(kind);
    changed!(at);
    changed!(to);
    changed!(takeoff);
    changed!(player_delta);
    changed!(loc_id);
    changed!(option);
    changed!(ticks);
    changed!(direction);
    changed!(open_loc_id);
    changed!(skill_req);
    changed!(item_req);
    changed!(consumed_req);
    changed!(item_returns);
    changed!(quest_req);
    changed!(varp_req);
    changed!(worn_req);
    changed!(worn_all_req);
    changed!(forbidden_held);
    changed!(forbidden_worn);
    changed!(members_req);
    changed!(wildy_cap);
    changed!(quest_gates);
    fields
}

fn edge_summary(edge: &TransportEdge) -> EdgeSummary {
    EdgeSummary {
        kind: format!("{:?}", edge.kind),
        at: tile_summary(edge.at),
        to: tile_summary(edge.to),
        takeoff: edge.takeoff.map(tile_summary),
        player_delta: edge.player_delta.map(tile_summary),
        loc_id: edge.loc_id,
        option: edge.option,
        ticks: edge.ticks,
        direction: edge.dir.map(|direction| format!("{direction:?}")),
        open_loc_id: edge.open_loc_id,
        skill_req: edge.skill_req.clone(),
        item_req: edge.item_req.clone(),
        consumed_req: edge.consumed_req.clone(),
        item_returns: edge.item_returns.clone(),
        quest_req: edge.quest_req.clone(),
        varp_req: edge.varp_req.clone(),
        worn_req: edge.worn_req.clone(),
        worn_all_req: edge.worn_all_req.clone(),
        forbidden_held: edge.forbidden_held.clone(),
        forbidden_worn: edge.forbidden_worn.clone(),
        members_req: edge.members_req,
        wildy_cap: edge.wildy_cap,
        quest_gates: edge.quest_gates.as_ref().map(|gates| format!("{gates:?}")),
    }
}

fn placement_summary(placement: &Placement) -> TileSummary {
    TileSummary {
        x: placement.x,
        z: placement.z,
        level: placement.level,
    }
}

fn coord_summary(coord: Coord) -> TileSummary {
    TileSummary {
        x: coord.x(),
        z: coord.z(),
        level: coord.level(),
    }
}

fn tile_summary(tile: WorldTile) -> TileSummary {
    TileSummary {
        x: tile.x,
        z: tile.z,
        level: tile.level,
    }
}

fn bound_summary(mut entries: Vec<EntryMeasurement>) -> BoundSummary {
    entries.sort_by_key(|entry| {
        (
            entry.value,
            entry.loc_type,
            entry.placed_id,
            entry.stand.map(|stand| (stand.x, stand.z, stand.level)),
        )
    });
    let maximum = entries.last().map_or(0, |entry| entry.value);
    let percentile_index = entries
        .len()
        .saturating_mul(99)
        .div_ceil(100)
        .saturating_sub(1);
    let percentile_99 = entries.get(percentile_index).map_or(0, |entry| entry.value);
    let ten_largest = entries.iter().rev().take(10).cloned().collect();
    let recommended_limit = maximum
        .saturating_mul(4)
        .checked_next_power_of_two()
        .unwrap_or(u64::MAX);
    BoundSummary {
        maximum,
        percentile_99,
        ten_largest,
        recommended_limit,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use api::game_data::{QuestIdentityRow, QuestRequirements, Transmission};
    use api::selected::{FactKey, InclusiveRange, QuestGate, StageWindow};
    use nav_planner::quest_gates::QuestGates;
    use rs2vm::machine::{
        Arena, Atom, AtomId, AtomKind, CmpOp, EffectRecord, ExprId, RunStats, TransferAudit,
    };
    use rs2vm::{ScriptId, Ticks, VarpId};

    fn test_tile(x: i32, z: i32, level: i32) -> WorldTile {
        WorldTile { x, z, level }
    }
    fn loc_effect(
        kind: LocKind,
        obj: LocObj,
        ty: i32,
        tile: Coord,
        shape: u8,
        angle: u8,
    ) -> EffectRecord {
        EffectRecord {
            effect: Effect::Loc {
                kind,
                obj,
                ty: LocTypeId(ty),
                tile,
                shape,
                angle,
                duration: 3,
            },
            site: Site::default(),
            ticks: Ticks::default(),
        }
    }

    #[test]
    fn bounds_keep_entry_identity_with_each_of_ten_largest_measurements() {
        let entries = (1..=12)
            .map(|value| EntryMeasurement {
                value,
                loc_type: value as i32,
                placed_id: value as u32 * 10,
                handler_script: Some(value as u32),
                handler_name: Some(format!("handler-{value}")),
                handler_source: Some(format!("scripts/handler_{value}.rs2")),
                role: "wall".to_owned(),
                stand: Some(tile_summary(test_tile(value as i32, 20, 0))),
                bound_reached: value == 12,
            })
            .collect();

        let summary = bound_summary(entries);

        assert_eq!(summary.maximum, 12);
        assert_eq!(summary.recommended_limit, 64);
        assert_eq!(summary.ten_largest.len(), 10);
        for (rank, entry) in summary.ten_largest.iter().enumerate() {
            let value = 12 - rank as u64;
            assert_eq!(entry.value, value);
            assert_eq!(entry.loc_type, value as i32);
            assert_eq!(entry.placed_id, value as u32 * 10);
            assert_eq!(entry.handler_script, Some(value as u32));
            let expected_name = format!("handler-{value}");
            assert_eq!(entry.handler_name.as_deref(), Some(expected_name.as_str()));
            let expected_source = format!("scripts/handler_{value}.rs2");
            assert_eq!(
                entry.handler_source.as_deref(),
                Some(expected_source.as_str())
            );
            assert_eq!(entry.role, "wall");
            assert_eq!(
                entry.stand,
                Some(tile_summary(test_tile(value as i32, 20, 0)))
            );
        }
    }

    #[test]
    fn edge_diff_names_every_changed_transport_field() {
        let left = TransportEdge {
            kind: TransportKind::Door,
            at: test_tile(10, 20, 0),
            to: test_tile(11, 20, 0),
            takeoff: None,
            player_delta: None,
            loc_id: 100,
            option: 0,
            ticks: 1,
            dir: None,
            open_loc_id: None,
            skill_req: Vec::new(),
            item_req: Vec::new(),
            consumed_req: Vec::new(),
            item_returns: Vec::new(),
            quest_req: Vec::new(),
            varp_req: Vec::new(),
            worn_req: Vec::new(),
            worn_all_req: Vec::new(),
            forbidden_held: Vec::new(),
            forbidden_worn: Vec::new(),
            members_req: false,
            wildy_cap: None,
            quest_gates: None,
        };
        let mut right = left.clone();
        right.kind = TransportKind::Ladder;
        right.at = test_tile(12, 20, 0);
        right.to = test_tile(13, 20, 0);
        right.takeoff = Some(test_tile(12, 21, 0));
        right.player_delta = Some(test_tile(0, 0, 1));
        right.loc_id = 101;
        right.option = 1;
        right.ticks = 2;
        right.dir = Some(DoorDir::N);
        right.open_loc_id = Some(102);
        right.skill_req = vec![(1, 2)];
        right.item_req = vec![(3, 4)];
        right.consumed_req = vec![(5, 6)];
        right.item_returns = vec![(7, 8)];
        right.quest_req = vec!["quest".to_owned()];
        right.varp_req = vec![(9, 10)];
        right.worn_req = vec![11];
        right.worn_all_req = vec![12];
        right.forbidden_held = vec![13];
        right.forbidden_worn = vec![14];
        right.members_req = true;
        right.wildy_cap = Some(15);
        right.quest_gates = Some(
            QuestGates::new([QuestGate::Complete(FactKey::new("quest"))])
                .expect("one valid quest gate"),
        );

        let changed = edge_changed_fields(&edge_summary(&left), &edge_summary(&right));
        let changed: Vec<&str> = changed.iter().map(String::as_str).collect();
        assert_eq!(
            changed,
            vec![
                "kind",
                "at",
                "to",
                "takeoff",
                "player_delta",
                "loc_id",
                "option",
                "ticks",
                "direction",
                "open_loc_id",
                "skill_req",
                "item_req",
                "consumed_req",
                "item_returns",
                "quest_req",
                "varp_req",
                "worn_req",
                "worn_all_req",
                "forbidden_held",
                "forbidden_worn",
                "members_req",
                "wildy_cap",
                "quest_gates",
            ]
        );
    }
    #[test]
    fn open_type_is_bound_to_the_entry_loc_and_its_geometry() {
        let left = LocObj::Placed(PlacedId(23_367));
        let right = LocObj::Placed(PlacedId(23_368));
        let left_coord = Coord::new(2_570, 0, 3_441);
        let right_coord = Coord::new(2_571, 0, 3_441);
        let unrelated_coord = Coord::new(2_600, 0, 3_441);
        let straight = LocShape::WALL_STRAIGHT as u8;
        let diagonal = LocShape::WALL_DIAGONAL as u8;
        let left_open = open_leaf_coord(left_coord, straight, 0).expect("straight door offset");
        let right_open = open_leaf_coord(right_coord, straight, 2).expect("straight door offset");
        assert_eq!(left_open, left_coord.offset(-1, 0, 0));
        assert_eq!(
            open_leaf_coord(left_coord, diagonal, 0),
            Some(left_coord.offset(0, 0, 1))
        );
        assert_eq!(open_leaf_coord(left_coord, u8::MAX, 0), None);

        let effects = vec![
            loc_effect(LocKind::Del, left, 89, left_coord, straight, 0),
            loc_effect(LocKind::Add, left, 83, left_coord, straight, 0),
            loc_effect(
                LocKind::Add,
                LocObj::Created(0),
                1_563,
                unrelated_coord,
                straight,
                3,
            ),
            loc_effect(LocKind::Del, right, 90, right_coord, straight, 2),
            loc_effect(LocKind::Add, right, 83, right_coord, straight, 2),
            loc_effect(LocKind::Add, LocObj::Created(1), 1_562, left_open, straight, 3),
            loc_effect(LocKind::Add, LocObj::Created(2), 1_563, right_open, straight, 1),
        ];

        assert_eq!(opened_type(&effects, left, 89), Some(1_562));
        assert_eq!(opened_type(&effects, right, 90), Some(1_563));

        let direct_change = vec![
            loc_effect(LocKind::Del, left, 89, left_coord, straight, 0),
            loc_effect(LocKind::Change, left, 1_562, left_coord, straight, 0),
        ];
        assert_eq!(opened_type(&direct_change, left, 89), Some(1_562));

        let ambiguous = vec![
            loc_effect(LocKind::Del, left, 89, left_coord, straight, 0),
            loc_effect(LocKind::Add, left, 83, left_coord, straight, 0),
            loc_effect(LocKind::Add, LocObj::Created(0), 1_562, left_open, straight, 3),
            loc_effect(LocKind::Add, LocObj::Created(1), 1_563, left_open, straight, 3),
        ];
        assert_eq!(opened_type(&ambiguous, left, 89), None);

        let unsupported = vec![
            loc_effect(LocKind::Del, left, 89, left_coord, u8::MAX, 0),
            loc_effect(LocKind::Add, left, 83, left_coord, u8::MAX, 0),
            loc_effect(LocKind::Add, LocObj::Created(0), 1_562, left_open, straight, 3),
        ];
        assert_eq!(opened_type(&unsupported, left, 89), None);
    }


    #[test]
    fn typed_unknown_and_near_positions_are_rejected_as_landings() {
        let at = test_tile(10, 20, 0);
        let site = Site {
            script: ScriptId(1),
            pc: 2,
        };
        let unknown = PlayerPos::Unknown(Value::Sym(ExprId(0)));
        let near = PlayerPos::Near(site);

        assert_eq!(
            candidate_landing_from_position("ladder", unknown, at, None),
            None
        );
        assert_eq!(
            candidate_landing_from_position("ladder", near, at, None),
            None
        );
        assert!(landing_rejection_reason(Some(unknown)).contains("unknown typed coordinate"));
        assert!(landing_rejection_reason(Some(near)).contains("Near"));
    }

    #[test]
    fn stage_window_gate_uses_raw_varp_selection_and_exact_window() {
        let facts = api::game_data::QuestIdentityFacts {
            rows: vec![QuestIdentityRow {
                id: "test_quest".to_owned(),
                component: "test".to_owned(),
                display: "Test Quest".to_owned(),
                varp: "test_progress".to_owned(),
                varp_id: 42,
                complete: 10,
                colour_authoritative: true,
                stage_domain: Some(InclusiveRange {
                    min: Some(0),
                    max: Some(20),
                }),
                transmission: Transmission::NotTransmitted,
                quest_points: 0,
                unknown_sides: Vec::new(),
                requirements: QuestRequirements {
                    qualification: "complete".to_owned(),
                    skills: Vec::new(),
                    items: Vec::new(),
                    empty_must_have: false,
                    unknown_as_satisfied: false,
                },
                members: false,
                kind: "quest".to_owned(),
                journal_title: None,
                journal_script: None,
            }],
            coverage: Vec::new(),
        };
        let source = super::super::QuestSource { facts: &facts };
        let observable = super::super::ObservableGates::from_content(
            Path::new("missing-nav-shadow-test-content"),
            Some(&source),
        );
        let site = Site {
            script: ScriptId(1),
            pc: 2,
        };
        let run = Run {
            paths: Vec::new(),
            arena: Arena {
                atoms: vec![Atom {
                    kind: AtomKind::Varp(VarpId(42)),
                    epoch: 0,
                    nth: 0,
                    site,
                    class: CondClass::Requirement,
                    bounds: (0, i32::MAX),
                }],
                exprs: vec![Expr::Atom(AtomId(0))],
                near: Vec::new(),
            },
            stats: RunStats::default(),
            transfers: TransferAudit::default(),
        };
        let condition = Cond {
            pred: Pred::Cmp {
                lhs: Value::Sym(ExprId(0)),
                op: CmpOp::Ge,
                rhs: Value::Known(10),
            },
            truth: true,
            class: CondClass::Requirement,
            site,
            choice: 1,
        };
        let needs_for = |condition: &Cond| {
            condition_needs(
                &run,
                condition,
                &Program::default(),
                &HashMap::new(),
                &observable,
            )
        };
        let needs = needs_for(&condition).expect("stage threshold is a representable gate");

        assert_eq!(
            needs.quests,
            vec![QuestGate::Window(StageWindow {
                quest: FactKey::new("test_quest"),
                signal: FactKey::new("test_progress"),
                values: InclusiveRange {
                    min: Some(10),
                    max: Some(20),
                },
            })]
        );
        let condition_for = |operator, constant, truth| Cond {
            pred: Pred::Cmp {
                lhs: Value::Sym(ExprId(0)),
                op: operator,
                rhs: Value::Known(constant),
            },
            truth,
            ..condition.clone()
        };
        let exact = needs_for(&condition_for(CmpOp::Eq, 10, true))
            .expect("selected exact stage is representable");
        assert_eq!(
            exact.quests,
            vec![QuestGate::Window(StageWindow {
                quest: FactKey::new("test_quest"),
                signal: FactKey::new("test_progress"),
                values: InclusiveRange {
                    min: Some(10),
                    max: Some(10),
                },
            })]
        );
        let after_zero = needs_for(&condition_for(CmpOp::Eq, 0, false))
            .expect("selected nonzero stage is representable");
        assert_eq!(
            after_zero.quests,
            vec![QuestGate::Window(StageWindow {
                quest: FactKey::new("test_quest"),
                signal: FactKey::new("test_progress"),
                values: InclusiveRange {
                    min: Some(1),
                    max: Some(20),
                },
            })]
        );
        assert!(needs_for(&condition_for(CmpOp::Eq, 0, true)).is_none());
        assert!(needs_for(&condition_for(CmpOp::Ge, 10, false)).is_none());
    }

    #[test]
    fn members_eq_zero_false_arm_requires_the_members_world() {
        let site = Site {
            script: ScriptId(1),
            pc: 2,
        };
        let run = Run {
            paths: Vec::new(),
            arena: Arena {
                atoms: vec![Atom {
                    kind: AtomKind::Members,
                    epoch: 0,
                    nth: 0,
                    site,
                    class: CondClass::Requirement,
                    bounds: (0, 1),
                }],
                exprs: vec![Expr::Atom(AtomId(0))],
                near: Vec::new(),
            },
            stats: RunStats::default(),
            transfers: TransferAudit::default(),
        };
        let program = Program::default();
        let worn_slots = HashMap::new();
        let observable = super::super::ObservableGates::from_content(
            Path::new("missing-nav-shadow-test-content"),
            None,
        );
        let condition_for = |constant, truth| Cond {
            pred: Pred::Cmp {
                lhs: Value::Sym(ExprId(0)),
                op: CmpOp::Eq,
                rhs: Value::Known(constant),
            },
            truth,
            class: CondClass::Requirement,
            site,
            choice: 1,
        };
        let needs_for = |condition: &Cond| {
            condition_needs(&run, condition, &program, &worn_slots, &observable)
        };

        assert!(needs_for(&condition_for(0, false))
            .expect("members account takes the false arm of members == 0")
            .members);
        assert!(needs_for(&condition_for(1, true))
            .expect("members account takes the true arm of members == 1")
            .members);
        assert!(needs_for(&condition_for(0, true)).is_none());
        assert!(needs_for(&condition_for(1, false)).is_none());
    }

    #[test]
    fn positive_not_equal_thresholds_use_the_next_value() {
        assert_eq!(lower_bound(CmpOp::Ne, 5), Some(6));
        assert_eq!(inventory_requirement(CmpOp::Ne, 5), Some((6, false)));
        assert_eq!(lower_bound(CmpOp::Ne, i32::MAX), None);
        assert_eq!(inventory_requirement(CmpOp::Ne, i32::MAX), None);
    }
}
#[cfg(test)]
mod content_tests;
