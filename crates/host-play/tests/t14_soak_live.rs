//! Shared dev/candidate local-WSS soak. The launcher supplies the effective
//! configuration, isolated HOME, local proxy and output directory. This test
//! starts ordinary production Play slots; it never launches the Engine or
//! proxy itself.

#[cfg(feature = "soak-observers")]
#[path = "support/t14_candidate_observers.rs"]
mod candidate_observers;

#[path = "support/t14_final_barrier.rs"]
mod final_barrier;

use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::{BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use api::game_data::{self, GatherSiteOption};
use api::interact::cheat;
use api::selected::ClientRevision;
use api::snapshot::{
    ActorKind, ActorTargetView, ChatLineView, GameSnapshot, HitmarksView, ProjectileView, WorldTile,
};
use host::Pump;
use host_play::{ProfileOptions, SharedClientTemplate};
use scenario::{Proof, RunnerStatus, Scenario, ScenarioRunner, Step, StepKind, Wait};
use script::native::death::{hitpoints_zero, is_death_line, DeathLatch, RESPAWN_SQUARE};
use script::native::{ScriptStatus, StatusValue};
use serde_json::{json, Map, Value};
use vault::{Profile, ProfileSettings};

const CONFIG_ENV: &str = "BOT_T14_SOAK_CONFIG";
const OUTPUT_ENV: &str = "BOT_T14_SOAK_OUTPUT";
const TICK_MS: u64 = 100;
const NORMAL_LOG_ID: i32 = 1511;
const WOODCUTTING_STAT_ID: i32 = 8;
const GATHERER_PREP_STEP: &str = "prepare Woodcutting 1 and a real bronze axe before Start";
const GATHERER_SOAK_PREP_STEP: &str =
    "prepare Woodcutting 99, a rune axe, lobster and spare supplies before Start";
const GATHERER_FOOD_PROOF_STEP: &str =
    "observe the seeded lobster in the account inventory before Start";
const GATHERER_SITE_STEP: &str = "stand at the live Draynor normal tree before Start";
const GATHERER_TREE_PROOF_STEP: &str = "confirm the selected-289 normal tree is live before Start";
const GATHERER_RELOG_STEP: &str = "relog successfully so the inventory tab binds before Start";
const SURVIVAL_FOOD_ALIAS: &str = "lobster";
const SURVIVAL_FOOD_NAME: &str = "Lobster";
const SURVIVAL_FOOD_ID: i32 = 379;
const SURVIVAL_FOOD_TARGET: i32 = 1;

// Five normal slots use distinct in-site Starts and a five-tree reachability
// witness; fifteen slots use oak. Nearby normal resources can still overlap.
// The offline audit covers workload routes and ordinary recovery only.
// Random-event movement is outside this proof.
const NORMAL_SLOTS: [usize; 5] = [0, 2, 4, 6, 8];
const NORMAL_STARTS: [(i32, i32); 5] = [
    (2946, 3399),
    (2955, 3399),
    (2952, 3417),
    (2955, 3407),
    (2958, 3415),
];
const NORMAL_RADIUS: i32 = 5;
const NORMAL_SITE_ID: &str = "woodcutting.falador_west.n";
const SECOND_TIER_SITE_ID: &str = "woodcutting.dwarven_mine";
const SECOND_TIER_START: (i32, i32) = (3007, 3411);
const SECOND_TIER_KEY: &str = "oak";
const SECOND_TIER_LOG_ID: i32 = 1521;
const SECOND_TIER_LEVEL: i32 = 15;
const SECOND_TIER_RADIUS: i32 = 12;
const SOAK_BANK: &str = "Falador West";
const SOAK_BANK_PREFIX: &str = "Falador West; Reachable;";

#[derive(Clone, Copy)]
struct SafeWalk {
    min_x: i32,
    max_x: i32,
    min_z: i32,
    max_z: i32,
}

impl SafeWalk {
    fn contains(self, tile: (i32, i32, i32)) -> bool {
        tile.2 == 0
            && (self.min_x..=self.max_x).contains(&tile.0)
            && (self.min_z..=self.max_z).contains(&tile.1)
    }
}

// Worksite census input, not a movement stop condition. Correct Guardian
// travel can leave this rectangle without entering an authored danger envelope.
const NORMAL_SAFE_WALK: &[SafeWalk] = &[
    SafeWalk {
        min_x: 2941,
        max_x: 2963,
        min_z: 3394,
        max_z: 3422,
    },
    SafeWalk {
        min_x: 2995,
        max_x: 3019,
        min_z: 3399,
        max_z: 3423,
    },
];

// Selected-region bounds leave room for the sampled Guardian travel.
// The danger-envelope guard remains active everywhere in this bound.
const OUTER_WALK_BOUNDS: &[SafeWalk] = &[SafeWalk {
    min_x: 2840,
    max_x: 3065,
    min_z: 3262,
    max_z: 3527,
}];

fn danger_envelopes() -> &'static [(WorldTile, i32)] {
    static ENVELOPES: LazyLock<Vec<(WorldTile, i32)>> = LazyLock::new(|| {
        let data = game_data::for_revision(ClientRevision::R289)
            .expect("the soak requires embedded revision-289 content");
        data.npc_placements()
            .expect("the soak requires authored NPC placements")
            .rows
            .iter()
            .filter_map(|placement| {
                let npc = data
                    .npc_name(placement.npc_id)
                    .expect("every authored NPC placement requires its definition");
                if npc.huntrange <= 0 {
                    return None;
                }
                let radius = npc.wanderrange + npc.huntrange + npc.size;
                let intersects = placement.plane == 0
                    && OUTER_WALK_BOUNDS.iter().any(|area| {
                        (area.min_x - radius..=area.max_x + radius).contains(&placement.x)
                            && (area.min_z - radius..=area.max_z + radius).contains(&placement.z)
                    });
                intersects.then_some((
                    WorldTile {
                        x: placement.x,
                        z: placement.z,
                        level: placement.plane,
                    },
                    radius,
                ))
            })
            .collect()
    });
    &ENVELOPES
}

fn safe_walk_contains(areas: &[SafeWalk], tile: (i32, i32, i32)) -> bool {
    areas.iter().any(|area| area.contains(tile))
}

fn worksite_exit(
    areas: &[SafeWalk],
    previous: Option<(i32, i32, i32)>,
    tile: (i32, i32, i32),
) -> bool {
    previous.is_some_and(|last| safe_walk_contains(areas, last)) && !safe_walk_contains(areas, tile)
}

fn work_route_escape(outer_bounds: &[SafeWalk], tile: (i32, i32, i32)) -> bool {
    danger_envelopes().iter().any(|(spawn, radius)| {
        tile.2 == spawn.level && (tile.0 - spawn.x).abs().max((tile.1 - spawn.z).abs()) <= *radius
    }) || !safe_walk_contains(outer_bounds, tile)
}

fn worksite_patch_is_safe((x, z, level): (i32, i32, i32), radius: i32) -> bool {
    (-radius..=radius).all(|dx| {
        (-radius..=radius).all(|dz| !work_route_escape(OUTER_WALK_BOUNDS, (x + dx, z + dz, level)))
    })
}

#[derive(Debug, Default, PartialEq, Eq)]
struct RandomRouteAttribution {
    observed_event: Option<host::RandomKind>,
    attributed_kind: Option<host::RandomKind>,
    event_origin: Option<(i32, i32, i32)>,
    excursion: bool,
    return_pending: bool,
    progress_fence_pending: bool,
    progress_baseline: Option<WorkProgress>,
}

#[derive(Debug, Default, PartialEq, Eq)]
struct WorkProgress {
    trips: Option<u64>,
    yielded: Option<u64>,
}

#[derive(Debug, Default, PartialEq, Eq)]
struct RandomRouteUpdate {
    event_started: Option<host::RandomKind>,
    status_ended: Option<host::RandomKind>,
    status_end_clear_reason: Option<&'static str>,
    excursion_started: Option<host::RandomKind>,
    reentered: Option<host::RandomKind>,
    reentry_clear_reason: Option<&'static str>,
    work_progress_resumed: Option<host::RandomKind>,
    attribution_cleared: bool,
    route_escape_exempt: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RandomRouteLog {
    event_name: &'static str,
    kind: host::RandomKind,
    attribution_cleared: bool,
    clear_reason: Option<&'static str>,
}

impl RandomRouteUpdate {
    fn log_events(&self) -> [Option<RandomRouteLog>; 5] {
        [
            self.event_started.map(|kind| RandomRouteLog {
                event_name: "random-event-start",
                kind,
                attribution_cleared: false,
                clear_reason: None,
            }),
            self.status_ended.map(|kind| RandomRouteLog {
                event_name: "random-event-route-status-ended",
                kind,
                attribution_cleared: self.status_end_clear_reason.is_some(),
                clear_reason: self.status_end_clear_reason,
            }),
            self.excursion_started.map(|kind| RandomRouteLog {
                event_name: "random-event-route-excursion",
                kind,
                attribution_cleared: false,
                clear_reason: None,
            }),
            self.reentered.map(|kind| RandomRouteLog {
                event_name: "random-event-route-reentry",
                kind,
                attribution_cleared: true,
                clear_reason: self.reentry_clear_reason,
            }),
            self.work_progress_resumed.map(|kind| RandomRouteLog {
                event_name: "random-event-route-work-progress-resumed",
                kind,
                attribution_cleared: true,
                clear_reason: Some("work-progress-resumed"),
            }),
        ]
    }
}

impl WorkProgress {
    fn advance(baseline: &mut Option<u64>, current: Option<u64>) -> bool {
        let previous = *baseline;
        let increased = previous
            .zip(current)
            .is_some_and(|(previous, current)| current > previous);
        if let Some(current) = current {
            *baseline = Some(previous.map_or(current, |previous| previous.max(current)));
        }
        increased
    }
}

impl RandomRouteAttribution {
    fn clear(&mut self) {
        self.attributed_kind = None;
        self.event_origin = None;
        self.excursion = false;
        self.return_pending = false;
        self.progress_fence_pending = false;
        self.progress_baseline = None;
    }

    fn observe(
        &mut self,
        random_event: Option<host::RandomKind>,
        tile: Option<(i32, i32, i32)>,
        route_escape: Option<bool>,
    ) -> RandomRouteUpdate {
        let previous_event = self.observed_event;
        let event_started = random_event.filter(|kind| previous_event != Some(*kind));
        let status_ended = previous_event.filter(|_| random_event.is_none());
        self.observed_event = random_event;
        if let Some(kind) = event_started.filter(|_| !self.excursion) {
            self.attributed_kind = Some(kind);
            self.event_origin = tile;
            self.return_pending = false;
            self.progress_fence_pending = false;
            self.progress_baseline = None;
        }

        let mut update = RandomRouteUpdate {
            event_started,
            status_ended,
            ..RandomRouteUpdate::default()
        };
        if let Some(kind) = status_ended {
            let moved_from_origin = kind == host::RandomKind::Evade
                && self
                    .event_origin
                    .is_some_and(|origin| tile.is_some_and(|tile| tile != origin));
            if moved_from_origin {
                // The native Evade resolver walks back to the pre-flee tile
                // after its event kind disappears; retain only that return.
                self.return_pending = true;
            } else if !self.excursion {
                self.clear();
                update.attribution_cleared = true;
                update.status_end_clear_reason = Some("event-ended-without-movement");
            }
            if self.attributed_kind.is_some() {
                self.progress_fence_pending = true;
                self.progress_baseline = None;
            }
        }

        match route_escape {
            Some(true) => {
                let event_is_attributed =
                    random_event == self.attributed_kind && random_event.is_some();
                let native_evade_return =
                    self.return_pending && self.attributed_kind == Some(host::RandomKind::Evade);
                if !self.excursion && (event_is_attributed || native_evade_return) {
                    self.excursion = true;
                    update.excursion_started = self.attributed_kind;
                }
                update.route_escape_exempt = self.excursion;
            }
            Some(false) => {
                if self.return_pending && tile == self.event_origin {
                    update.reentered = self.attributed_kind;
                    update.reentry_clear_reason = Some("evade-return-origin");
                    self.clear();
                    update.attribution_cleared = true;
                } else if !self.return_pending && self.excursion {
                    update.reentered = self.attributed_kind;
                    update.reentry_clear_reason = Some("safe-reentry");
                    self.clear();
                    update.attribution_cleared = true;
                }
            }
            None => {}
        }
        update
    }

    fn observe_work_progress(
        &mut self,
        sample: &StatusSample,
        route_escape: Option<bool>,
    ) -> RandomRouteUpdate {
        let mut update = RandomRouteUpdate::default();
        if self.attributed_kind.is_none()
            || self.observed_event.is_some()
            || (!self.excursion && !self.return_pending)
        {
            return update;
        }

        let progress = WorkProgress {
            trips: sample.trips,
            yielded: sample.yielded,
        };
        if self.progress_fence_pending {
            self.progress_fence_pending = false;
            self.progress_baseline = Some(progress);
            return update;
        }

        let Some(baseline) = self.progress_baseline.as_mut() else {
            self.progress_baseline = Some(progress);
            return update;
        };
        let yielded_increased = WorkProgress::advance(&mut baseline.yielded, progress.yielded);
        let trips_increased = WorkProgress::advance(&mut baseline.trips, progress.trips);
        if route_escape == Some(false) && (yielded_increased || trips_increased) {
            update.work_progress_resumed = self.attributed_kind;
            update.attribution_cleared = true;
            self.clear();
        }
        update
    }
}

fn route_oracle_failure(
    script_started: bool,
    route_escape: Option<bool>,
    update: &RandomRouteUpdate,
) -> bool {
    script_started && route_escape.is_some_and(|escaped| escaped && !update.route_escape_exempt)
}

fn status_progress(trips: u64, yielded: u64) -> StatusSample {
    StatusSample {
        trips: Some(trips),
        yielded: Some(yielded),
        ..StatusSample::default()
    }
}

#[test]
fn workload_route_violation_is_not_exempted_by_missing_event_or_hold() {
    let unsafe_tile = (NORMAL_STARTS[0].0, NORMAL_STARTS[0].1, 1);
    assert!(work_route_escape(OUTER_WALK_BOUNDS, unsafe_tile));

    let held = host_play::SlotFrameInput {
        hold: true,
        random_event: None,
        ..host_play::SlotFrameInput::default()
    };
    let mut attribution = RandomRouteAttribution::default();
    let update = attribution.observe(held.random_event, Some(unsafe_tile), Some(true));
    assert!(route_oracle_failure(true, Some(true), &update));
}

#[test]
fn owned_random_event_logs_before_the_same_frame_route_excursion() {
    let origin = (NORMAL_STARTS[0].0, NORMAL_STARTS[0].1, 0);
    let unsafe_tile = (origin.0, origin.1, 1);
    assert!(!work_route_escape(OUTER_WALK_BOUNDS, origin));
    assert!(work_route_escape(OUTER_WALK_BOUNDS, unsafe_tile));

    let mut same_frame_attribution = RandomRouteAttribution::default();
    let same_frame = same_frame_attribution.observe(
        Some(host::RandomKind::Evade),
        Some(unsafe_tile),
        Some(true),
    );
    let same_frame_logs = same_frame.log_events();
    assert_eq!(
        same_frame_logs[0].map(|event| event.event_name),
        Some("random-event-start")
    );
    assert_eq!(
        same_frame_logs[2].map(|event| event.event_name),
        Some("random-event-route-excursion")
    );
    assert!(!route_oracle_failure(true, Some(true), &same_frame));

    let mut attribution = RandomRouteAttribution::default();
    let start = attribution.observe(Some(host::RandomKind::Evade), Some(origin), Some(false));
    assert_eq!(
        start.log_events()[0].map(|event| event.event_name),
        Some("random-event-start")
    );
    let departure =
        attribution.observe(Some(host::RandomKind::Evade), Some(unsafe_tile), Some(true));
    assert_eq!(
        departure.log_events()[2].map(|event| event.event_name),
        Some("random-event-route-excursion")
    );
    assert!(departure.route_escape_exempt);
    assert!(!route_oracle_failure(true, Some(true), &departure));
}

#[test]
fn event_ended_before_evade_replan_keeps_attribution_until_return() {
    let origin = (NORMAL_STARTS[0].0, NORMAL_STARTS[0].1, 0);
    let moved = (origin.0 + 1, origin.1, 0);
    let unsafe_tile = (origin.0, origin.1, 1);
    assert!(!work_route_escape(OUTER_WALK_BOUNDS, origin));
    assert!(!work_route_escape(OUTER_WALK_BOUNDS, moved));
    assert!(work_route_escape(OUTER_WALK_BOUNDS, unsafe_tile));

    let mut attribution = RandomRouteAttribution::default();
    attribution.observe(Some(host::RandomKind::Evade), Some(origin), Some(false));
    attribution.observe(Some(host::RandomKind::Evade), Some(moved), Some(false));
    let ended = attribution.observe(None, Some(moved), Some(false));
    assert_eq!(ended.status_ended, Some(host::RandomKind::Evade));
    assert!(!ended.attribution_cleared);
    assert!(attribution.return_pending);

    let replan = attribution.observe(None, Some(unsafe_tile), Some(true));
    assert!(replan.route_escape_exempt);
    assert!(!route_oracle_failure(true, Some(true), &replan));
    assert_eq!(
        replan.log_events()[2].map(|event| event.event_name),
        Some("random-event-route-excursion")
    );

    // The return can cross back into the approved area before the native
    // walk reaches its exact pre-flee tile.
    let waypoint = attribution.observe(None, Some(moved), Some(false));
    assert!(!waypoint.attribution_cleared);
    assert!(attribution.return_pending);
    assert!(attribution.excursion);
    let stair_replan = attribution.observe(None, Some(unsafe_tile), Some(true));
    assert!(stair_replan.route_escape_exempt);
    assert!(!route_oracle_failure(true, Some(true), &stair_replan));

    let returned = attribution.observe(None, Some(origin), Some(false));
    assert_eq!(returned.reentered, Some(host::RandomKind::Evade));
    assert!(returned.attribution_cleared);
    let later_workload_departure = attribution.observe(None, Some(unsafe_tile), Some(true));
    assert!(route_oracle_failure(
        true,
        Some(true),
        &later_workload_departure
    ));
}

#[test]
fn event_ended_without_movement_does_not_exempt_a_later_departure() {
    let origin = (NORMAL_STARTS[0].0, NORMAL_STARTS[0].1, 0);
    let unsafe_tile = (origin.0, origin.1, 1);
    let mut attribution = RandomRouteAttribution::default();
    attribution.observe(Some(host::RandomKind::Evade), Some(origin), Some(false));
    let ended = attribution.observe(None, Some(origin), Some(false));
    assert_eq!(ended.status_ended, Some(host::RandomKind::Evade));
    assert!(ended.attribution_cleared);
    assert!(!attribution.return_pending);

    let later_departure = attribution.observe(None, Some(unsafe_tile), Some(true));
    assert!(route_oracle_failure(true, Some(true), &later_departure));
}

#[test]
fn resumed_work_progress_clears_evade_attribution_at_safe_work_tile() {
    let origin = (NORMAL_STARTS[0].0, NORMAL_STARTS[0].1, 0);
    let work_tile = (origin.0 + 1, origin.1, 0);
    let unsafe_tile = (origin.0, origin.1, 1);
    assert_ne!(work_tile, origin);
    assert!(!work_route_escape(OUTER_WALK_BOUNDS, work_tile));
    assert!(safe_walk_contains(NORMAL_SAFE_WALK, work_tile));
    assert!(work_route_escape(OUTER_WALK_BOUNDS, unsafe_tile));

    let mut attribution = RandomRouteAttribution::default();
    attribution.observe(Some(host::RandomKind::Evade), Some(origin), Some(false));
    let excursion =
        attribution.observe(Some(host::RandomKind::Evade), Some(unsafe_tile), Some(true));
    assert!(excursion.route_escape_exempt);
    let ended = attribution.observe(None, Some(unsafe_tile), Some(true));
    assert_eq!(ended.status_ended, Some(host::RandomKind::Evade));
    assert!(ended.route_escape_exempt);
    assert!(attribution.return_pending);

    let work_return = attribution.observe(None, Some(work_tile), Some(false));
    assert!(!work_return.attribution_cleared);
    assert!(attribution.return_pending);

    // This first post-event sample can include an increment observed from
    // before the event; it fences that value rather than proving resumption.
    let first_post_event = status_progress(0, 5);
    let fenced = attribution.observe_work_progress(&first_post_event, Some(false));
    assert!(!fenced.attribution_cleared);
    assert!(!attribution.progress_fence_pending);
    assert_eq!(
        attribution.progress_baseline,
        Some(WorkProgress {
            trips: Some(0),
            yielded: Some(5),
        })
    );

    let unchanged = attribution.observe_work_progress(&status_progress(0, 5), Some(false));
    assert!(!unchanged.attribution_cleared);
    assert!(attribution.return_pending);

    let resumed = attribution.observe_work_progress(&status_progress(0, 6), Some(false));
    assert!(resumed.attribution_cleared);
    let clear_event = resumed
        .log_events()
        .into_iter()
        .flatten()
        .find(|event| event.event_name == "random-event-route-work-progress-resumed")
        .expect("work progress clear event");
    assert!(clear_event.attribution_cleared);
    assert_eq!(clear_event.clear_reason, Some("work-progress-resumed"));

    let later_workload_departure = attribution.observe(None, Some(unsafe_tile), Some(true));
    assert!(route_oracle_failure(
        true,
        Some(true),
        &later_workload_departure
    ));
}

#[test]
fn worksite_census_and_bank_guard_use_distinct_bounds() {
    for (x, z) in NORMAL_STARTS {
        assert!(safe_walk_contains(NORMAL_SAFE_WALK, (x, z, 0)));
        assert!(worksite_patch_is_safe((x, z, 0), NORMAL_RADIUS));
    }
    assert!(!safe_walk_contains(NORMAL_SAFE_WALK, (2944, 3366, 0)));
    assert!(!work_route_escape(OUTER_WALK_BOUNDS, (2944, 3366, 0)));
    assert!(safe_walk_contains(
        NORMAL_SAFE_WALK,
        (SECOND_TIER_START.0, SECOND_TIER_START.1, 0)
    ));
    assert!(worksite_patch_is_safe(
        (SECOND_TIER_START.0, SECOND_TIER_START.1, 0),
        SECOND_TIER_RADIUS
    ));
    for dx in -SECOND_TIER_RADIUS..=SECOND_TIER_RADIUS {
        for dz in -SECOND_TIER_RADIUS..=SECOND_TIER_RADIUS {
            let tile = (SECOND_TIER_START.0 + dx, SECOND_TIER_START.1 + dz, 0);
            assert!(safe_walk_contains(NORMAL_SAFE_WALK, tile), "{tile:?}");
            assert!(!work_route_escape(OUTER_WALK_BOUNDS, tile), "{tile:?}");
        }
    }
    assert!(!safe_walk_contains(
        NORMAL_SAFE_WALK,
        (
            SECOND_TIER_START.0 + SECOND_TIER_RADIUS + 1,
            SECOND_TIER_START.1,
            0
        )
    ));
}

#[test]
fn worksite_patch_rejects_outer_boundary_and_danger_crossings() {
    let bounds = OUTER_WALK_BOUNDS[0];
    let (_, z) = NORMAL_STARTS[0];
    assert!(!worksite_patch_is_safe((bounds.max_x, z, 0), NORMAL_RADIUS));
    assert!(!worksite_patch_is_safe((bounds.min_x, z, 0), NORMAL_RADIUS));
    let (x, _) = NORMAL_STARTS[0];
    assert!(!worksite_patch_is_safe((x, bounds.max_z, 0), NORMAL_RADIUS));
    assert!(!worksite_patch_is_safe((x, bounds.min_z, 0), NORMAL_RADIUS));
    let mut crossings = 0;
    for (spawn, radius) in danger_envelopes() {
        let anchor = (spawn.x + radius + 1, spawn.z, spawn.level);
        if !work_route_escape(OUTER_WALK_BOUNDS, anchor) {
            assert!(!worksite_patch_is_safe(anchor, NORMAL_RADIUS));
            crossings += 1;
        }
    }
    assert!(
        crossings > 0,
        "content must supply a safe anchor beside danger"
    );
}

#[test]
fn worksite_departure_is_recorded_without_failing_the_route_guard() {
    let oak_rect = NORMAL_SAFE_WALK[1];
    let inside = (oak_rect.max_x, SECOND_TIER_START.1, 0);
    let outside = (oak_rect.max_x + 1, SECOND_TIER_START.1, 0);
    assert!(worksite_exit(NORMAL_SAFE_WALK, Some(inside), outside));
    assert!(!work_route_escape(OUTER_WALK_BOUNDS, outside));
    assert!(!worksite_exit(NORMAL_SAFE_WALK, Some(outside), inside));
    assert!(!worksite_exit(NORMAL_SAFE_WALK, None, outside));
}

#[test]
fn work_route_guard_rejects_every_danger_envelope_and_outer_boundary() {
    for &(spawn, radius) in danger_envelopes() {
        for dx in [-radius, 0, radius] {
            for dz in [-radius, 0, radius] {
                let tile = (spawn.x + dx, spawn.z + dz, spawn.level);
                assert!(work_route_escape(OUTER_WALK_BOUNDS, tile), "{tile:?}");
            }
        }
    }
    // Historical unsafe route tiles stay forbidden after a region change.
    assert!(work_route_escape(OUTER_WALK_BOUNDS, (2812, 3495, 0)));
    assert!(work_route_escape(OUTER_WALK_BOUNDS, (2813, 3477, 0)));
    let bounds = OUTER_WALK_BOUNDS[0];
    let (x, z) = NORMAL_STARTS[0];
    for tile in [
        (bounds.min_x - 1, z, 0),
        (bounds.max_x + 1, z, 0),
        (x, bounds.min_z - 1, 0),
        (x, bounds.max_z + 1, 0),
        (x, z, 1),
    ] {
        assert!(work_route_escape(OUTER_WALK_BOUNDS, tile), "{tile:?}");
    }
}

#[derive(Clone)]
struct BankSite {
    id: String,
    resource: &'static str,
    product_id: i32,
    level: i32,
    bank: &'static str,
    bank_prefix: &'static str,
    start: WorldTile,
    outer_bounds: &'static [SafeWalk],
}

struct RunConfig {
    qualification: bool,
    slots: usize,
    duration: Duration,
    warmup: Duration,
    sample: Duration,
    hash: Duration,
    name_prefix: String,
    profile_name: String,
    host: String,
    port: u16,
    asset_host: String,
    http_port: u16,
    ca_pem: PathBuf,
    cache_dir: PathBuf,
    engine_dir: PathBuf,
    nav_pack: PathBuf,
    nav_flags: PathBuf,
    fault_seed: u64,
    max_unexpected_exits: u64,
    max_payload_divergences: u64,
    connection_bytes: u64,
    rss_slope_baseline_slack_bytes_per_hour: u64,
    rss_final_baseline_extra_bytes_per_connection: u64,
    rss_final_baseline_extra_percent: u64,
    fixed_threads: u64,
    sites: [BankSite; 2],
    source_config: Option<String>,
}

struct Actor {
    account: String,
    site: BankSite,
    runner: ScenarioRunner,
    snapshot: GameSnapshot,
    pump: Pump,
    death_latch: DeathLatch,
    last_life_tick: Option<u32>,
    last_life_tile: Option<(i32, i32, i32)>,
    last_life_hitpoints: Option<(i32, i32)>,
    last_life_hitmarks: Option<HitmarksView>,
    last_life_death_watermark: Option<i32>,
    last_damage_sources: DamageSourceSnapshot,
    death_events_observed: u64,
    last_death_sequence: Option<i32>,
    scene2_since: Option<Instant>,
    scene2_unix_ms: Option<u128>,
    script_requested: bool,
    script_started_at: Option<Instant>,
    script_started_unix_ms: Option<u128>,
    script_start_tick: Option<u32>,
    script_start_budget_ticks: u32,
    script_running_observed: bool,
    last_tile: Option<(i32, i32, i32)>,
    random_route: RandomRouteAttribution,
    walk_tile_changes: u64,
    last_trips: u64,
    bank_cycles: u64,
    first_bank_cycle_elapsed_seconds: Option<u64>,
    first_bank_cycle_monotonic_ns: Option<u64>,
    last_status: Option<StatusSample>,
    connected_ever: bool,
    setup_relog_requested: bool,
    setup_relog_exit_observed: bool,
    setup_relog_returned: bool,
    last_stream_epoch: Option<u32>,
    exit_recorded: bool,
    warmup_qualified: bool,
    slot_ready_monotonic_ns: Option<u64>,
    scene2_monotonic_ns: Option<u64>,
    script_running_monotonic_ns: Option<u64>,
    task_status_monotonic_ns: Option<u64>,
    failure: Option<String>,
}

#[derive(Debug, Eq, PartialEq)]
enum ScriptStartObservation {
    Waiting,
    Running,
    Expired,
}

fn script_start_observation(
    start_tick: Option<u32>,
    observed_tick: u32,
    running: bool,
    budget_ticks: u32,
) -> ScriptStartObservation {
    let Some(start_tick) = start_tick else {
        return ScriptStartObservation::Waiting;
    };
    let elapsed = observed_tick.wrapping_sub(start_tick);
    if elapsed > budget_ticks || (!running && elapsed == budget_ticks) {
        ScriptStartObservation::Expired
    } else if running {
        ScriptStartObservation::Running
    } else {
        ScriptStartObservation::Waiting
    }
}

#[test]
fn script_start_budget_excludes_the_cohort_barrier() {
    let scenario = scenario::get("gatherer").expect("Gatherer scenario");
    let budget = scenario
        .steps
        .iter()
        .find(|step| matches!(step.kind, StepKind::StartScript))
        .expect("compiled Start step")
        .wait
        .budget_ticks;
    assert_eq!(budget, 50);
    for barrier_tick in [0, 50, 500, 10_000] {
        assert_eq!(
            script_start_observation(None, barrier_tick, false, budget),
            ScriptStartObservation::Waiting
        );
    }
    let actual_start = 10_000;
    assert_eq!(
        script_start_observation(Some(actual_start), actual_start + 49, false, budget),
        ScriptStartObservation::Waiting
    );
    for elapsed in [0, 49, 50] {
        assert_eq!(
            script_start_observation(Some(actual_start), actual_start + elapsed, true, budget),
            ScriptStartObservation::Running
        );
    }
}

#[test]
fn script_start_budget_expires_without_running_after_actual_start() {
    let budget = 50;
    let actual_start = 10_000;
    assert_eq!(
        script_start_observation(None, actual_start, false, budget),
        ScriptStartObservation::Waiting
    );
    assert_eq!(
        script_start_observation(Some(actual_start), actual_start + 49, false, budget),
        ScriptStartObservation::Waiting
    );
    assert_eq!(
        script_start_observation(Some(actual_start), actual_start + 50, false, budget),
        ScriptStartObservation::Expired
    );
    assert_eq!(
        script_start_observation(Some(actual_start), actual_start + 51, true, budget),
        ScriptStartObservation::Expired
    );
}

#[derive(Clone, Default)]
struct StatusSample {
    phase: String,
    trips: Option<u64>,
    deposited: Option<u64>,
    yielded: Option<u64>,
    deaths: Option<u64>,
    bank: Option<String>,
    target: Option<String>,
    excluded_targets: Option<String>,
    absent: Option<u64>,
    last_event: Option<String>,
    failure_code: Option<String>,
    failure_message: Option<String>,
}

#[derive(Clone)]
struct PhaseZero {
    monotonic_ns: u64,
    account: String,
    seed: u64,
    at_utc: Option<String>,
    elapsed_seconds: Option<f64>,
}

struct ProxyEventReader {
    path: PathBuf,
    offset: u64,
    pending: Vec<u8>,
    phase_zero: Option<PhaseZero>,
}

impl ProxyEventReader {
    fn new(path: PathBuf) -> Self {
        Self {
            path,
            offset: 0,
            pending: Vec::new(),
            phase_zero: None,
        }
    }

    fn poll(&mut self, expected_seed: u64) -> Result<Option<PhaseZero>, String> {
        let mut file = match File::open(&self.path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(self.phase_zero.clone());
            }
            Err(error) => return Err(format!("open proxy event log: {error}")),
        };
        file.seek(SeekFrom::Start(self.offset))
            .map_err(|error| format!("seek proxy event log: {error}"))?;
        let mut appended = Vec::new();
        file.read_to_end(&mut appended)
            .map_err(|error| format!("read proxy event log: {error}"))?;
        self.offset = self
            .offset
            .checked_add(appended.len() as u64)
            .ok_or("proxy event log offset overflow")?;
        self.pending.extend_from_slice(&appended);
        while let Some(end) = self.pending.iter().position(|byte| *byte == b'\n') {
            let line = self.pending.drain(..=end).take(end).collect::<Vec<_>>();
            if line.is_empty() {
                continue;
            }
            let event: Value = serde_json::from_slice(&line)
                .map_err(|error| format!("parse proxy event: {error}"))?;
            if event.get("type").and_then(Value::as_str) != Some("fault_phase_zero") {
                continue;
            }
            if self.phase_zero.is_some() {
                return Err("proxy emitted more than one fault_phase_zero event".into());
            }
            let seed = event
                .get("seed")
                .and_then(Value::as_u64)
                .ok_or("fault_phase_zero event lacks an unsigned seed")?;
            if seed != expected_seed {
                return Err("fault_phase_zero seed differs from the explicit soak config".into());
            }
            let monotonic_ns = event
                .get("phase_zero_monotonic_ns")
                .and_then(Value::as_u64)
                .ok_or("fault_phase_zero event lacks phase_zero_monotonic_ns")?;
            if event.get("trigger").and_then(Value::as_str)
                != Some("first_script_start_after_setup")
            {
                return Err("proxy phase zero was not caused by Play script Start".into());
            }
            self.phase_zero = Some(PhaseZero {
                monotonic_ns,
                account: event
                    .get("account")
                    .and_then(Value::as_str)
                    .ok_or("fault_phase_zero event lacks script-Start account")?
                    .to_owned(),
                seed,
                at_utc: event
                    .get("phase_zero_at_utc")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                elapsed_seconds: event
                    .get("phase_zero_elapsed_seconds")
                    .and_then(Value::as_f64),
            });
        }
        Ok(self.phase_zero.clone())
    }
}

struct EventWriter(Mutex<BufWriter<File>>);

impl EventWriter {
    fn create(output: &Path) -> Result<Self, String> {
        let path = output.join("events.jsonl");
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|error| format!("create {}: {error}", path.display()))?;
        Ok(Self(Mutex::new(BufWriter::new(file))))
    }

    fn write(&self, event: &Value) -> Result<(), String> {
        let mut output = self.0.lock().map_err(|_| "event writer poisoned")?;
        serde_json::to_writer(&mut *output, event)
            .map_err(|error| format!("serialize event: {error}"))?;
        output
            .write_all(b"\n")
            .map_err(|error| format!("write event: {error}"))?;
        output
            .flush()
            .map_err(|error| format!("flush event: {error}"))
    }
}

fn required<'a>(root: &'a Value, path: &str) -> Result<&'a Value, String> {
    let mut value = root;
    for key in path.split('.') {
        value = value
            .get(key)
            .ok_or_else(|| format!("soak config is missing {path}"))?;
    }
    Ok(value)
}

fn required_str(root: &Value, path: &str) -> Result<String, String> {
    required(root, path)?
        .as_str()
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .ok_or_else(|| format!("soak config {path} must be a non-empty string"))
}

fn required_u64(root: &Value, path: &str) -> Result<u64, String> {
    required(root, path)?
        .as_u64()
        .ok_or_else(|| format!("soak config {path} must be a non-negative integer"))
}

fn required_bool(root: &Value, path: &str) -> Result<bool, String> {
    required(root, path)?
        .as_bool()
        .ok_or_else(|| format!("soak config {path} must be a boolean"))
}

fn required_path(root: &Value, path: &str) -> Result<PathBuf, String> {
    let value = PathBuf::from(required_str(root, path)?);
    if !value.is_absolute() {
        return Err(format!(
            "soak config {path} must be absolute: {}",
            value.display()
        ));
    }
    Ok(value)
}

fn required_port(root: &Value, path: &str) -> Result<u16, String> {
    let value = required_u64(root, path)?;
    u16::try_from(value)
        .ok()
        .filter(|port| *port != 0)
        .ok_or_else(|| format!("soak config {path} must be an assigned TCP port"))
}

fn time_ms() -> Result<u128, String> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .map_err(|error| format!("system clock before UNIX epoch: {error}"))
}

#[cfg(unix)]
fn monotonic_now_ns() -> Result<u64, String> {
    let mut timestamp = std::mem::MaybeUninit::<libc::timespec>::uninit();
    // SAFETY: `timestamp` is aligned writable storage for `timespec`; a
    // successful clock_gettime call initializes it before the pointer is read.
    let result = unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, timestamp.as_mut_ptr()) };
    if result != 0 {
        return Err(format!(
            "read CLOCK_MONOTONIC: {}",
            std::io::Error::last_os_error()
        ));
    }
    // SAFETY: POSIX specifies that successful clock_gettime initializes the
    // complete timespec object pointed to by `timestamp`.
    let timestamp = unsafe { timestamp.assume_init() };
    let seconds =
        u64::try_from(timestamp.tv_sec).map_err(|_| "negative CLOCK_MONOTONIC seconds")?;
    let nanoseconds =
        u64::try_from(timestamp.tv_nsec).map_err(|_| "negative CLOCK_MONOTONIC nanoseconds")?;
    seconds
        .checked_mul(1_000_000_000)
        .and_then(|value| value.checked_add(nanoseconds))
        .ok_or_else(|| "CLOCK_MONOTONIC timestamp overflow".to_owned())
}

#[cfg(not(unix))]
fn monotonic_now_ns() -> Result<u64, String> {
    Err("the local soak requires a Unix CLOCK_MONOTONIC clock".into())
}

#[cfg(unix)]
#[test]
fn monotonic_clock_reports_ordered_timespecs() {
    let first = monotonic_now_ns().expect("read monotonic clock");
    let second = monotonic_now_ns().expect("read monotonic clock again");
    assert!(second >= first);
}

fn site_has_key(site: &GatherSiteOption, key: &str) -> bool {
    site.keys
        .iter()
        .any(|row| row.key.eq_ignore_ascii_case(key))
}

fn select_sites(root: &Value) -> Result<[BankSite; 2], String> {
    let selected = game_data::for_revision(ClientRevision::R289)
        .map_err(|error| format!("selected 289 game data: {error}"))?;
    let names = required(root, "workload.site_names")?
        .as_array()
        .filter(|names| names.len() == 2)
        .ok_or("workload.site_names must contain the two reviewed content sites")?;
    let select = |index: usize, expected: &str, resource: &'static str| {
        let name = names[index]
            .as_str()
            .filter(|name| *name == expected)
            .ok_or_else(|| format!("workload.site_names[{index}] must be {expected:?}"))?;
        let site = selected
            .gather_site("woodcutting", name)
            .ok_or_else(|| format!("selected 289 content lacks woodcutting site {name:?}"))?;
        make_site(site, resource)
    };
    Ok([
        select(0, NORMAL_SITE_ID, "normal")?,
        select(1, SECOND_TIER_SITE_ID, SECOND_TIER_KEY)?,
    ])
}

fn site_for_slot(index: usize, sites: &[BankSite; 2]) -> BankSite {
    if let Some(normal_index) = NORMAL_SLOTS.iter().position(|slot| *slot == index) {
        let mut site = sites[0].clone();
        let (x, z) = NORMAL_STARTS[normal_index];
        site.start = WorldTile { x, z, level: 0 };
        site
    } else {
        sites[1].clone()
    }
}

#[test]
fn cohort_keeps_five_lowest_normal_slots_and_moves_the_rest_to_oak() {
    let sites = select_sites(&json!({
        "workload": {"site_names": [NORMAL_SITE_ID, SECOND_TIER_SITE_ID]}
    }))
    .expect("selected content sites");
    assert_eq!(sites[0].id, "woodcutting.falador_west.n");
    assert_eq!(
        NORMAL_STARTS,
        [
            (2946, 3399),
            (2955, 3399),
            (2952, 3417),
            (2955, 3407),
            (2958, 3415),
        ]
    );
    assert_eq!(sites[1].id, "woodcutting.dwarven_mine");
    assert_eq!(
        (sites[1].start.x, sites[1].start.z, sites[1].start.level),
        (3007, 3411, 0)
    );
    assert_eq!(SECOND_TIER_RADIUS, 12);
    let mut counts = [0, 0];
    for slot in 0..20 {
        let site = site_for_slot(slot, &sites);
        if slot < 10 && slot % 2 == 0 {
            counts[0] += 1;
            assert_eq!(site.resource, "normal");
            assert_eq!((site.start.x, site.start.z), NORMAL_STARTS[slot / 2]);
        } else {
            counts[1] += 1;
            assert_eq!(site.resource, SECOND_TIER_KEY);
            assert_eq!((site.start.x, site.start.z), SECOND_TIER_START);
        }
        assert_eq!(site.start.level, 0);
        assert_eq!(site.bank, SOAK_BANK);
    }
    assert_eq!(counts, [5, 15]);
}

fn make_site(site: &GatherSiteOption, resource: &'static str) -> Result<BankSite, String> {
    if !site_has_key(site, resource) {
        return Err(format!(
            "selected content site {} lacks {resource}",
            site.id
        ));
    }
    let (anchor, product_id, level, radius) = if resource == "normal" {
        (NORMAL_STARTS[0], NORMAL_LOG_ID, 1, NORMAL_RADIUS)
    } else if resource == SECOND_TIER_KEY {
        (
            SECOND_TIER_START,
            SECOND_TIER_LOG_ID,
            SECOND_TIER_LEVEL,
            SECOND_TIER_RADIUS,
        )
    } else {
        return Err(format!("no reviewed live bank seed for {resource}"));
    };
    let start = WorldTile {
        x: anchor.0,
        z: anchor.1,
        level: 0,
    };
    let region = &site.region;
    // Normal anchors use the approved census union, not the authored Site box.
    // Oak starts inside their distinct Site and require a safe radius-12 patch.
    let allowed = if resource == "normal" {
        safe_walk_contains(NORMAL_SAFE_WALK, (start.x, start.z, start.level))
            && worksite_patch_is_safe((start.x, start.z, start.level), NORMAL_RADIUS)
    } else {
        start.x >= region.min_x
            && start.x <= region.max_x
            && start.z >= region.min_z
            && start.z <= region.max_z
            && start.level == region.level
            && worksite_patch_is_safe((start.x, start.z, start.level), radius)
    };
    if !allowed {
        return Err(format!(
            "live seed tile {start:?} is outside the approved fixture geometry for {}",
            site.id
        ));
    }
    Ok(BankSite {
        id: site.id.clone(),
        resource,
        product_id,
        level,
        bank: SOAK_BANK,
        bank_prefix: SOAK_BANK_PREFIX,
        start,
        outer_bounds: OUTER_WALK_BOUNDS,
    })
}

fn parse_config(path: &Path) -> Result<RunConfig, String> {
    let bytes =
        fs::read(path).map_err(|error| format!("read config {}: {error}", path.display()))?;
    let root: Value = serde_json::from_slice(&bytes)
        .map_err(|error| format!("parse config {}: {error}", path.display()))?;
    let qualification = required_bool(&root, "run.qualification")?;
    let slots = usize::try_from(required_u64(&root, "run.slots")?)
        .map_err(|_| "run.slots does not fit usize")?;
    if slots < 2 {
        return Err("run.slots must cover both required real bank sites".into());
    }
    let duration_seconds = required_u64(&root, "run.duration_seconds")?;
    let warmup_seconds = required_u64(&root, "run.warmup_min_seconds")?;
    let sample_seconds = required_u64(&root, "run.sample_seconds")?;
    let hash_seconds = required_u64(&root, "run.hash_seconds")?;
    if duration_seconds == 0 || sample_seconds == 0 || hash_seconds == 0 {
        return Err("run duration, sample interval and hash interval must be positive".into());
    }
    let name_prefix = required_str(&root, "run.name_prefix")?;
    if !(1..=4).contains(&name_prefix.len()) {
        return Err("run.name_prefix must be 1–4 bytes for host-play account minting".into());
    }
    if qualification
        && (slots != 20
            || duration_seconds != 7200
            || warmup_seconds < 900
            || sample_seconds != 10
            || hash_seconds != 60)
    {
        return Err("qualification config must specify 20 slots, 7200 seconds, >=900 seconds warmup, 10-second samples and 60-second hashes".into());
    }

    let max_unexpected_exits = required_u64(&root, "limits.max_unexpected_exits")?;
    let max_payload_divergences = required_u64(&root, "limits.max_payload_divergences")?;
    if qualification && (max_unexpected_exits != 0 || max_payload_divergences != 0) {
        return Err(
            "qualification limits must require zero unexpected exits and zero payload divergences"
                .into(),
        );
    }
    if required_str(&root, "profile.host")? != "127.0.0.1"
        || required_str(&root, "profile.asset_host")? != "127.0.0.1"
        || required_str(&root, "proxy.upstream_host")? != "127.0.0.1"
        || required_str(&root, "proxy.listen_host")? != "127.0.0.1"
    {
        return Err("live soak and debug seeding require exclusively loopback endpoints".into());
    }
    let source_config = root
        .get("run")
        .and_then(|run| run.get("source_config"))
        .and_then(Value::as_str)
        .map(ToOwned::to_owned);

    Ok(RunConfig {
        qualification,
        slots,
        duration: Duration::from_secs(duration_seconds),
        warmup: Duration::from_secs(warmup_seconds),
        sample: Duration::from_secs(sample_seconds),
        hash: Duration::from_secs(hash_seconds),
        name_prefix,
        profile_name: required_str(&root, "profile.name")?,
        host: required_str(&root, "profile.host")?,
        port: required_port(&root, "profile.port")?,
        asset_host: required_str(&root, "profile.asset_host")?,
        http_port: required_port(&root, "profile.http_port")?,
        ca_pem: required_path(&root, "profile.ca_pem")?,
        cache_dir: required_path(&root, "profile.cache_dir")?,
        engine_dir: required_path(&root, "profile.engine_dir")?,
        nav_pack: required_path(&root, "profile.nav_pack")?,
        fault_seed: required_u64(&root, "fault.seed")?,
        nav_flags: required_path(&root, "profile.nav_flags")?,
        connection_bytes: required_u64(&root, "limits.connection_bytes")?,
        rss_slope_baseline_slack_bytes_per_hour: required_u64(
            &root,
            "limits.rss_slope_baseline_slack_bytes_per_hour",
        )?,
        rss_final_baseline_extra_bytes_per_connection: required_u64(
            &root,
            "limits.rss_final_baseline_extra_bytes_per_connection",
        )?,
        rss_final_baseline_extra_percent: required_u64(
            &root,
            "limits.rss_final_baseline_extra_percent",
        )?,
        fixed_threads: required_u64(&root, "limits.fixed_threads")?,
        max_unexpected_exits,
        max_payload_divergences,
        sites: select_sites(&root)?,
        source_config,
    })
}

fn validate_isolated_tls(config: &RunConfig, output: &Path) -> Result<(), String> {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or("isolated HOME is required")?;
    if !home.is_absolute() {
        return Err("HOME must be an absolute isolated directory".into());
    }
    let home = fs::canonicalize(&home).map_err(|error| format!("canonicalize HOME: {error}"))?;
    let ca = fs::canonicalize(&config.ca_pem)
        .map_err(|error| format!("canonicalize throwaway CA: {error}"))?;
    if !ca.starts_with(&home) || !ca.is_file() {
        return Err("profile.ca_pem must be a file inside the isolated HOME".into());
    }
    let cert_file = std::env::var_os("SSL_CERT_FILE")
        .map(PathBuf::from)
        .ok_or("SSL_CERT_FILE must name the isolated throwaway CA")?;
    let cert_file = fs::canonicalize(cert_file)
        .map_err(|error| format!("canonicalize SSL_CERT_FILE: {error}"))?;
    if cert_file != ca {
        return Err("SSL_CERT_FILE must be exactly profile.ca_pem".into());
    }
    let cert_dir = std::env::var_os("SSL_CERT_DIR")
        .map(PathBuf::from)
        .ok_or("SSL_CERT_DIR must remain inside isolated HOME")?;
    let cert_dir = fs::canonicalize(cert_dir)
        .map_err(|error| format!("canonicalize SSL_CERT_DIR: {error}"))?;
    if !cert_dir.starts_with(&home) {
        return Err("SSL_CERT_DIR must remain inside isolated HOME".into());
    }
    let output = fs::canonicalize(output)
        .map_err(|error| format!("canonicalize output directory: {error}"))?;
    if output.starts_with(&home) {
        return Err("soak output must be outside the isolated HOME".into());
    }
    if !config.cache_dir.is_dir()
        || !config.engine_dir.is_dir()
        || !config.nav_pack.is_file()
        || !config.nav_flags.is_file()
    {
        return Err(
            "profile cache, Engine, navigation pack and navigation flags must exist".into(),
        );
    }
    Ok(())
}

fn load_profile(
    config: &RunConfig,
) -> Result<(Arc<host_play::ServerProfile>, Arc<SharedClientTemplate>), String> {
    let options = ProfileOptions {
        profile: Some(config.profile_name.clone()),
        revision: Some("289".into()),
        host: Some(config.host.clone()),
        port: Some(config.port),
        asset_host: Some(config.asset_host.clone()),
        http_port: Some(config.http_port),
        engine_dir: Some(config.engine_dir.clone()),
        cache_dir: Some(config.cache_dir.clone()),
        nav_pack: Some(config.nav_pack.clone()),
        nav_flags: Some(config.nav_flags.clone()),
        ..ProfileOptions::default()
    };
    let profile = options.resolve(None)?.bind_runtime()?;
    if profile.profile_class() != host_play::ProfileClass::Remote
        || profile.transport() != client::Transport::Wss
        || profile.revision() != ClientRevision::R289
        || profile.client().game_host() != config.host
        || profile.client().game_port() != config.port
        || profile.client().asset_host() != config.asset_host
        || profile.client().asset_port() != config.http_port
    {
        return Err("selected profile is not the configured local WSS 289 endpoint".into());
    }
    if profile.game_data().is_none() {
        return Err(format!(
            "selected WSS profile withheld the generated game data needed by Gatherer: {}",
            profile.game_data_status().detail(),
        ));
    }
    let template = SharedClientTemplate::load(Arc::clone(&profile))?;
    if template.world().is_none() {
        return Err("selected template has no navigation world".into());
    }
    Ok((profile, template))
}

fn scenario_for(site: &BankSite, duration: Duration) -> Result<Scenario, String> {
    let mut scenario = scenario::get("gatherer")
        .ok_or("the existing Gatherer live-harness scenario is unavailable")?;
    scenario.settings.deadline = duration;
    scenario.settings.nav.engine_speed_ms = None;
    scenario.settings.script_settings_inject = None;

    let skill_level = 99;
    let site_start = site.start;
    let prep = scenario
        .steps
        .iter_mut()
        .find(|step| step.name == GATHERER_PREP_STEP)
        .ok_or("Gatherer scenario has no existing account/tool seed step")?;
    prep.name = GATHERER_SOAK_PREP_STEP;
    let inventory_food_seed = format!("give {SURVIVAL_FOOD_ALIAS} {SURVIVAL_FOOD_TARGET}");
    let bank_food_seed = format!("givebank {SURVIVAL_FOOD_ALIAS} {SURVIVAL_FOOD_TARGET}");
    prep.kind = StepKind::Perform {
        send: Box::new(move |client, _| {
            cheat(client, &format!("setstat woodcutting {skill_level}")).is_sent()
                && cheat(client, "give rune_axe 1").is_sent()
                && cheat(client, "givebank rune_axe 64").is_sent()
                && cheat(client, &inventory_food_seed).is_sent()
                && cheat(client, &bank_food_seed).is_sent()
        }),
    };
    prep.wait.arm = Proof::ItemId { id: 1359, count: 1 };

    let tele = scenario
        .steps
        .iter_mut()
        .find(|step| step.name == GATHERER_SITE_STEP)
        .ok_or("Gatherer scenario has no existing live-site seed step")?;
    tele.name = "stand in the selected live woodcutting content site before Start";
    tele.kind = StepKind::Perform {
        send: Box::new(move |client, _| {
            let args = api::interact::tele_args(site_start.level, site_start.x, site_start.z);
            cheat(client, &args).is_sent()
        }),
    };
    tele.wait = Wait {
        arm: Proof::Arrived {
            x: site.start.x,
            z: site.start.z,
            level: site.start.level,
        },
        budget_ticks: 120,
    };
    let prep_index = scenario
        .steps
        .iter()
        .position(|step| step.name == GATHERER_SOAK_PREP_STEP)
        .ok_or("updated Gatherer preparation step disappeared")?;
    scenario.steps.insert(
        prep_index + 1,
        Step {
            name: GATHERER_FOOD_PROOF_STEP,
            kind: StepKind::Perform {
                send: Box::new(|_, _| true),
            },
            wait: Wait {
                arm: Proof::ItemId {
                    id: SURVIVAL_FOOD_ID,
                    count: SURVIVAL_FOOD_TARGET,
                },
                budget_ticks: 120,
            },
        },
    );
    scenario.steps.insert(
        prep_index + 2,
        Step {
            name: "observe the selected skill reached its real resource requirement",
            kind: StepKind::Perform {
                send: Box::new(|_, _| true),
            },
            wait: Wait {
                arm: Proof::Stat {
                    id: WOODCUTTING_STAT_ID,
                    min: skill_level,
                },
                budget_ticks: 120,
            },
        },
    );
    // The rune axe works from inventory. The inherited bronze-only equip step
    // would require an absent item; equipping rune would require an unrelated
    // Attack seed. Keep the inventory and skill proofs instead.
    scenario.steps.retain(|step| {
        step.name != GATHERER_TREE_PROOF_STEP
            && step.name != "equip the observed bronze axe before Start"
    });
    Ok(scenario)
}

#[test]
fn soak_seed_proves_rune_axe_and_lobster_without_inherited_bronze_equipment() {
    let sites = select_sites(&json!({
        "workload": {"site_names": [NORMAL_SITE_ID, SECOND_TIER_SITE_ID]}
    }))
    .expect("selected content sites");
    for site in sites {
        let scenario = scenario_for(&site, Duration::from_secs(7200)).expect("soak scenario");
        let prep = scenario
            .steps
            .iter()
            .find(|step| step.name == GATHERER_SOAK_PREP_STEP)
            .expect("soak seed step");
        assert!(matches!(
            prep.wait.arm,
            Proof::ItemId { id: 1359, count: 1 }
        ));
        assert_eq!(prep.wait.budget_ticks, 120);
        let food = scenario
            .steps
            .iter()
            .find(|step| step.name == GATHERER_FOOD_PROOF_STEP)
            .expect("soak food proof step");
        assert!(matches!(
            food.wait.arm,
            Proof::ItemId {
                id: SURVIVAL_FOOD_ID,
                count: SURVIVAL_FOOD_TARGET
            }
        ));
        assert_eq!(food.wait.budget_ticks, 120);
        assert!(!scenario.steps.iter().any(|step| {
            step.name == "equip the observed bronze axe before Start"
                || matches!(step.wait.arm, Proof::ItemId { id: 1351, .. })
                || matches!(step.wait.arm, Proof::EquipmentId { id: 1351 })
        }));
        assert!(scenario.steps.iter().any(|step| {
            matches!(
                step.wait.arm,
                Proof::Stat {
                    id: WOODCUTTING_STAT_ID,
                    min: 99
                }
            ) && step.wait.budget_ticks == 120
        }));
    }
}

fn bank_settings(site: &BankSite) -> Map<String, Value> {
    let mut settings = Map::new();
    settings.insert("skill".into(), json!("Woodcutting"));
    settings.insert("woodcuttingResources".into(), json!([site.resource]));
    settings.insert("targetPreference".into(), json!("Nearest"));
    settings.insert("location".into(), json!("Start"));
    settings.insert("site".into(), json!(site.id));
    settings.insert(
        "radius".into(),
        json!(if site.resource == "normal" {
            NORMAL_RADIUS
        } else {
            SECOND_TIER_RADIUS
        }),
    );
    settings.insert("disposition".into(), json!("Bank"));
    settings.insert("bank".into(), json!(site.bank));
    settings.insert("allowTeleports".into(), json!(false));
    settings.insert("allowWilderness".into(), json!(false));
    settings.insert("deathPolicy".into(), json!("Stop"));
    settings.insert("maxDeaths".into(), json!(2));
    settings.insert("food".into(), json!(SURVIVAL_FOOD_NAME));
    settings.insert("foodTarget".into(), json!(SURVIVAL_FOOD_TARGET));
    settings.insert("eatBelow".into(), json!(0));
    settings
}

#[test]
fn bank_settings_pin_both_tree_tiers_to_their_start_radius() {
    let sites = select_sites(&json!({
        "workload": {"site_names": [NORMAL_SITE_ID, SECOND_TIER_SITE_ID]}
    }))
    .expect("selected content sites");
    for (site, radius) in sites.iter().zip([NORMAL_RADIUS, SECOND_TIER_RADIUS]) {
        let settings = bank_settings(site);
        assert_eq!(settings["location"], json!("Start"));
        assert_eq!(settings["radius"], json!(radius));
        assert_eq!(settings["woodcuttingResources"], json!([site.resource]));
        assert_eq!(settings["bank"], json!(SOAK_BANK));
        assert_eq!(settings["disposition"], json!("Bank"));
        assert_eq!(settings["allowTeleports"], json!(false));
        assert_eq!(settings["allowWilderness"], json!(false));
        assert_eq!(settings["food"], json!(SURVIVAL_FOOD_NAME));
        assert_eq!(settings["foodTarget"], json!(SURVIVAL_FOOD_TARGET));
        assert_eq!(settings["eatBelow"], json!(0));
    }
}

fn write_pid_atomic(output: &Path) -> Result<(), String> {
    let temporary = output.join("pid.json.tmp");
    let destination = output.join("pid.json");
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(|error| format!("create {}: {error}", temporary.display()))?;
    let mut writer = BufWriter::new(file);
    serde_json::to_writer(&mut writer, &json!({"pid": std::process::id()}))
        .map_err(|error| format!("write pid receipt: {error}"))?;
    writer
        .write_all(b"\n")
        .map_err(|error| format!("write pid receipt newline: {error}"))?;
    writer
        .flush()
        .map_err(|error| format!("flush pid receipt: {error}"))?;
    writer
        .get_ref()
        .sync_all()
        .map_err(|error| format!("sync pid receipt: {error}"))?;
    fs::rename(&temporary, &destination)
        .map_err(|error| format!("publish {}: {error}", destination.display()))
}

fn write_soak_start_atomic(
    output: &Path,
    account: &str,
    seed: u64,
    monotonic_ns: u64,
    wall_ms: u128,
) -> Result<(), String> {
    let temporary = output.join("proxy").join("soak-start.json.tmp");
    let destination = output.join("proxy").join("soak-start.json");
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(|error| format!("create {}: {error}", temporary.display()))?;
    let mut writer = BufWriter::new(file);
    serde_json::to_writer(
        &mut writer,
        &json!({
            "schema_version": 1,
            "account": account,
            "seed": seed,
            "monotonic_ns": monotonic_ns,
            "wall_clock_unix_ms": wall_ms,
        }),
    )
    .map_err(|error| format!("write soak-start marker: {error}"))?;
    writer
        .write_all(b"\n")
        .map_err(|error| format!("write soak-start newline: {error}"))?;
    writer
        .flush()
        .map_err(|error| format!("flush soak-start marker: {error}"))?;
    writer
        .get_ref()
        .sync_all()
        .map_err(|error| format!("sync soak-start marker: {error}"))?;
    fs::rename(&temporary, &destination)
        .map_err(|error| format!("publish {}: {error}", destination.display()))
}

fn event_base(event: &str, actor: &Actor, started: Instant, wall_ms: u128) -> Value {
    json!({
        "schema_version": 1,
        "event": event,
        "account": actor.account,
        "site_id": actor.site.id,
        "resource": actor.site.resource,
        "worksite_start": [actor.site.start.x, actor.site.start.z, actor.site.start.level],
        "process_elapsed_seconds": started.elapsed().as_secs(),
        "event_monotonic_ns": monotonic_now_ns().ok(),
        "wall_clock_unix_ms": wall_ms,
        "game_tick": actor.snapshot.tick(),
        "scenario_game_ticks": actor.runner.total_ticks(),
        "script_start_game_tick": actor.script_start_tick,
        "script_start_budget_ticks": actor.script_start_budget_ticks,
    })
}

fn field_integer(status: &ScriptStatus, key: &str) -> Option<u64> {
    status.fields.iter().find_map(|field| {
        if field.key == key {
            match &field.value {
                StatusValue::Integer(value) if *value >= 0 => Some(*value as u64),
                _ => None,
            }
        } else {
            None
        }
    })
}

fn field_text(status: &ScriptStatus, key: &str) -> Option<String> {
    status.fields.iter().find_map(|field| {
        if field.key == key {
            match &field.value {
                StatusValue::Text(value) => Some(value.to_string()),
                _ => None,
            }
        } else {
            None
        }
    })
}

fn status_sample(status: &ScriptStatus) -> StatusSample {
    StatusSample {
        phase: format!("{:?}", status.phase),
        trips: field_integer(status, "trips"),
        deposited: field_integer(status, "deposited"),
        yielded: field_integer(status, "yielded"),
        deaths: field_integer(status, "deaths"),
        bank: field_text(status, "bank"),
        target: field_text(status, "target"),
        excluded_targets: field_text(status, "excluded_targets"),
        absent: field_integer(status, "absent"),
        last_event: field_text(status, "last_event"),
        failure_code: status
            .failure
            .as_ref()
            .map(|failure| failure.code.to_string()),
        failure_message: status
            .failure
            .as_ref()
            .map(|failure| failure.message.to_string()),
    }
}

fn status_json(sample: Option<&StatusSample>) -> Value {
    match sample {
        Some(sample) => json!({
            "phase": sample.phase,
            "trips": sample.trips,
            "deposited": sample.deposited,
            "yielded": sample.yielded,
            "deaths": sample.deaths,
            "bank": sample.bank,
            "target": sample.target,
            "excluded_targets": sample.excluded_targets,
            "absent": sample.absent,
            "last_event": sample.last_event,
            "failure_code": sample.failure_code,
            "failure_message": sample.failure_message,
        }),
        None => Value::Null,
    }
}

fn fail_actor(
    actor: &mut Actor,
    writer: &EventWriter,
    run_started: Instant,
    kind: &str,
    message: &str,
) {
    if actor.failure.is_some() {
        return;
    }
    actor.failure = Some(message.to_owned());
    if let Ok(wall_ms) = time_ms() {
        let mut event = event_base("failure", actor, run_started, wall_ms);
        event["kind"] = json!(kind);
        event["detail"] = json!(message);
        let _ = writer.write(&event);
    }
}

fn make_actor(
    account: String,
    site: BankSite,
    template: &SharedClientTemplate,
    run_duration: Duration,
) -> Result<Actor, String> {
    let mut scenario = scenario_for(&site, run_duration)?;
    scenario.seed.profiles = vec![("soak", "unused")];
    let script_start_budget_ticks = scenario
        .steps
        .iter()
        .find(|step| matches!(step.kind, StepKind::StartScript))
        .ok_or("Gatherer scenario has no compiled Start step")?
        .wait
        .budget_ticks;
    if script_start_budget_ticks != 50 {
        return Err("the soak requires the existing 50-game-tick compiled Start budget".into());
    }
    let world = template.world();
    if !world
        .as_ref()
        .is_some_and(|world| world.collision.standable(site.start))
    {
        return Err(format!(
            "woodcutting fixture Start is not standable: {:?}",
            site.start
        ));
    }
    let mut runner = ScenarioRunner::with_world(scenario, world);
    runner.set_deadline(run_duration);
    runner.set_live_names(std::slice::from_ref(&account));
    runner.set_map_members(false);
    Ok(Actor {
        account,
        site,
        runner,
        snapshot: GameSnapshot::new(),
        pump: Pump::new(),
        death_latch: DeathLatch::default(),
        last_life_tick: None,
        last_life_tile: None,
        last_life_hitpoints: None,
        last_life_hitmarks: None,
        last_life_death_watermark: None,
        last_damage_sources: DamageSourceSnapshot::default(),
        death_events_observed: 0,
        last_death_sequence: None,
        scene2_since: None,
        scene2_unix_ms: None,
        scene2_monotonic_ns: None,
        script_requested: false,
        script_started_at: None,
        script_started_unix_ms: None,
        script_start_tick: None,
        script_start_budget_ticks,
        script_running_observed: false,
        script_running_monotonic_ns: None,
        last_tile: None,
        random_route: RandomRouteAttribution::default(),
        walk_tile_changes: 0,
        last_trips: 0,
        bank_cycles: 0,
        first_bank_cycle_elapsed_seconds: None,
        first_bank_cycle_monotonic_ns: None,
        last_status: None,
        task_status_monotonic_ns: None,
        connected_ever: false,
        last_stream_epoch: None,
        exit_recorded: false,
        warmup_qualified: false,
        setup_relog_requested: false,
        setup_relog_exit_observed: false,
        setup_relog_returned: false,
        slot_ready_monotonic_ns: None,
        failure: None,
    })
}

fn record_scene2(actor: &mut Actor, writer: &EventWriter, run_started: Instant) {
    if actor.snapshot.ingame() && actor.snapshot.scene_state() == 2 {
        if actor.scene2_since.is_none() {
            let now = Instant::now();
            actor.scene2_since = Some(now);
            match monotonic_now_ns() {
                Ok(monotonic_ns) => actor.scene2_monotonic_ns = Some(monotonic_ns),
                Err(error) => {
                    actor.failure = Some(error);
                    return;
                }
            }
            match time_ms() {
                Ok(wall_ms) => {
                    actor.scene2_unix_ms = Some(wall_ms);
                    let mut event = event_base("scene2", actor, run_started, wall_ms);
                    event["tile"] = json!(actor.snapshot.tile());
                    event["scene2_monotonic_ns"] = json!(actor.scene2_monotonic_ns);
                    if let Err(error) = writer.write(&event) {
                        actor.failure = Some(error);
                    }
                }
                Err(error) => actor.failure = Some(error),
            }
        }
    } else {
        actor.scene2_since = None;
        actor.scene2_monotonic_ns = None;
    }
}

fn record_random_route_observation(
    actor: &mut Actor,
    writer: &EventWriter,
    run_started: Instant,
    tile: Option<(i32, i32, i32)>,
    observation: RandomRouteLog,
) {
    let wall_ms = match time_ms() {
        Ok(wall_ms) => wall_ms,
        Err(error) => {
            actor.failure = Some(error);
            return;
        }
    };
    let mut event = event_base(observation.event_name, actor, run_started, wall_ms);
    event["random_event_kind"] = json!(format!("{:?}", observation.kind));
    event["random_event_active"] = json!(actor.random_route.observed_event.is_some());
    event["tile"] = json!(tile);
    event["attribution_cleared"] = json!(observation.attribution_cleared);
    event["attribution_clear_reason"] = json!(observation.clear_reason);
    if let Err(error) = writer.write(&event) {
        actor.failure = Some(error);
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
struct DamageSourceSnapshot {
    local_player_index: Option<usize>,
    npc_targets_local: Vec<(usize, Option<usize>, WorldTile)>,
    player_targets_local: Vec<(usize, i32, WorldTile)>,
    projectiles_targeting_local: Vec<ProjectileView>,
}

impl DamageSourceSnapshot {
    fn matches(&self, snapshot: &GameSnapshot) -> bool {
        let local_player_index = snapshot.local_player().map(|local| local.player.index);
        if self.local_player_index != local_player_index {
            return false;
        }
        let Some(local_player_index) = local_player_index else {
            return self.npc_targets_local.is_empty()
                && self.player_targets_local.is_empty()
                && self.projectiles_targeting_local.is_empty();
        };
        let target = ActorTargetView {
            kind: ActorKind::Player,
            index: local_player_index,
        };
        let mut npcs = snapshot
            .npcs()
            .iter()
            .filter(|npc| npc.target == Some(target));
        for (index, npc_type, tile) in &self.npc_targets_local {
            let Some(npc) = npcs.next() else {
                return false;
            };
            if (npc.index, npc.r#type, npc.tile) != (*index, *npc_type, *tile) {
                return false;
            }
        }
        if npcs.next().is_some() {
            return false;
        }

        let mut players = snapshot
            .players()
            .iter()
            .filter(|player| player.actor.target == Some(target));
        for (index, combat_level, tile) in &self.player_targets_local {
            let Some(player) = players.next() else {
                return false;
            };
            if (player.index, player.combat_level, player.actor.tile)
                != (*index, *combat_level, *tile)
            {
                return false;
            }
        }
        if players.next().is_some() {
            return false;
        }

        let mut projectiles = snapshot
            .projectiles()
            .iter()
            .filter(|projectile| projectile.target == Some(target));
        for expected in &self.projectiles_targeting_local {
            if projectiles.next() != Some(expected) {
                return false;
            }
        }
        projectiles.next().is_none()
    }

    fn capture(snapshot: &GameSnapshot) -> Self {
        let local_player_index = snapshot.local_player().map(|local| local.player.index);
        let Some(local_player_index) = local_player_index else {
            return Self::default();
        };
        let target = ActorTargetView {
            kind: ActorKind::Player,
            index: local_player_index,
        };
        Self {
            local_player_index: Some(local_player_index),
            npc_targets_local: snapshot
                .npcs()
                .iter()
                .filter(|npc| npc.target == Some(target))
                .map(|npc| (npc.index, npc.r#type, npc.tile))
                .collect(),
            player_targets_local: snapshot
                .players()
                .iter()
                .filter(|player| player.actor.target == Some(target))
                .map(|player| (player.index, player.combat_level, player.actor.tile))
                .collect(),
            projectiles_targeting_local: snapshot
                .projectiles()
                .iter()
                .filter(|projectile| projectile.target == Some(target))
                .copied()
                .collect(),
        }
    }

    fn as_json(&self, snapshot: &GameSnapshot) -> Value {
        let local_player_index = self.local_player_index;
        let target = local_player_index.map(|index| ActorTargetView {
            kind: ActorKind::Player,
            index,
        });
        let npcs: Vec<Value> = target.map_or_else(Vec::new, |target| {
            snapshot
                .npcs()
                .iter()
                .filter(|npc| npc.target == Some(target))
                .map(|npc| {
                    json!({
                        "index": npc.index,
                        "npc_type_id": npc.r#type,
                        "name": npc.name,
                        "tile": npc.tile,
                        "in_combat": npc.in_combat,
                        "target": npc.target
                    })
                })
                .collect()
        });
        let players: Vec<Value> = target.map_or_else(Vec::new, |target| {
            snapshot
                .players()
                .iter()
                .filter(|player| player.actor.target == Some(target))
                .map(|player| {
                    json!({
                        "index": player.index,
                        "combat_level": player.combat_level,
                        "tile": player.actor.tile,
                        "in_combat": player.actor.in_combat,
                        "target": player.actor.target
                    })
                })
                .collect()
        });
        let projectiles: Vec<Value> = target.map_or_else(Vec::new, |target| {
            snapshot
                .projectiles()
                .iter()
                .filter(|projectile| projectile.target == Some(target))
                .map(|projectile| {
                    json!({
                        "spotanim": projectile.spotanim,
                        "source_tile": projectile.src,
                        "target": projectile.target,
                        "t1": projectile.t1,
                        "t2": projectile.t2
                    })
                })
                .collect()
        });
        json!({
            "attribution": "observed target relations, not confirmed damage causation",
            "local_player_index": local_player_index,
            "npc_targets_local": npcs,
            "player_targets_local": players,
            "projectiles_targeting_local": projectiles
        })
    }
}

fn record_life_frame(actor: &mut Actor, writer: &EventWriter, run_started: Instant) {
    let tick = actor.snapshot.tick();
    let tile = actor.snapshot.tile();
    let hitpoints = actor
        .snapshot
        .stats()
        .iter()
        .find(|stat| stat.name == "hitpoints")
        .map(|stat| (stat.effective, stat.base));
    let hitmarks = actor.snapshot.hitmarks().copied();
    let chat_lines: &[ChatLineView] = actor.snapshot.chat_lines();
    let previous_watermark = actor.death_latch.watermark();
    let death_observed = actor.death_latch.observe_lines(|| {
        chat_lines
            .iter()
            .map(|line| (line.sequence, line.text.as_str()))
    });
    let death_watermark = actor.death_latch.watermark();
    let damage_sources_match = actor.last_damage_sources.matches(&actor.snapshot);
    if actor.last_life_tick == Some(tick)
        && actor.last_life_tile == tile
        && actor.last_life_hitpoints == hitpoints
        && actor.last_life_hitmarks == hitmarks
        && actor.last_life_death_watermark == death_watermark
        && damage_sources_match
        && !death_observed
    {
        return;
    }

    let death_chat_lines = if death_observed {
        chat_lines
            .iter()
            .filter(|line| is_death_line(&line.text))
            .map(|line| json!({"sequence": line.sequence, "text": line.text}))
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    let death_sequence = death_chat_lines
        .iter()
        .filter_map(|line| line["sequence"].as_i64())
        .max()
        .and_then(|sequence| i32::try_from(sequence).ok());
    let respawn_square =
        tile.is_some_and(|(x, z, level)| RESPAWN_SQUARE.contains(WorldTile { x, z, level }));
    let hp_zero = hitpoints_zero(Some(actor.snapshot.stats()));
    let current_damage_sources =
        (!damage_sources_match).then(|| DamageSourceSnapshot::capture(&actor.snapshot));
    let damage_source_evidence = current_damage_sources
        .as_ref()
        .unwrap_or(&actor.last_damage_sources)
        .as_json(&actor.snapshot);
    let wall_ms = match time_ms() {
        Ok(wall_ms) => wall_ms,
        Err(error) => {
            actor.failure = Some(error);
            return;
        }
    };
    let mut event = event_base("life-frame", actor, run_started, wall_ms);
    event["tick"] = json!(tick);
    event["tile"] = json!(tile);
    event["hitpoints"] = json!({
        "effective": hitpoints.map(|(effective, _)| effective),
        "base": hitpoints.map(|(_, base)| base),
        "zero": hp_zero
    });
    event["hitmarks"] = json!(hitmarks);
    event["taking_damage"] = json!(actor.snapshot.taking_damage());
    event["damage_source_evidence"] = damage_source_evidence;
    event["death_chat_lines"] = json!(death_chat_lines);
    event["death_event_observed"] = json!(death_observed);
    event["death_chat_watermark_before"] = json!(previous_watermark);
    event["death_chat_watermark_after"] = json!(death_watermark);
    event["respawn_square"] = json!(respawn_square);
    if let Err(error) = writer.write(&event) {
        actor.failure = Some(error);
        return;
    }
    if let Some(current) = current_damage_sources {
        actor.last_damage_sources = current;
    }

    actor.last_life_tick = Some(tick);
    actor.last_life_tile = tile;
    actor.last_life_hitpoints = hitpoints;
    actor.last_life_hitmarks = hitmarks;
    actor.last_life_death_watermark = death_watermark;
    if death_observed {
        actor.death_events_observed = actor.death_events_observed.saturating_add(1);
        actor.last_death_sequence = death_sequence;
        let mut event = event_base("death-observed", actor, run_started, wall_ms);
        event["life_frame_game_tick"] = json!(tick);
        event["death_sequence"] = json!(death_sequence);
        event["death_chat_watermark_before"] = json!(previous_watermark);
        event["death_chat_watermark_after"] = json!(death_watermark);
        if let Err(error) = writer.write(&event) {
            actor.failure = Some(error);
        }
    }
}

fn drive_frame(
    actor: &mut Actor,
    client: &mut client::client::Client,
    frame: host_play::SlotFrameInput,
    writer: &EventWriter,
    run_started: Instant,
) {
    let drain = actor.pump.drain_client(client);
    host::publish_snapshot(&mut actor.snapshot, client, drain);
    record_scene2(actor, writer, run_started);
    record_life_frame(actor, writer, run_started);

    let tile = actor.snapshot.tile();
    let route_escape = tile.map(|tile| work_route_escape(actor.site.outer_bounds, tile));
    let route_update = actor
        .random_route
        .observe(frame.random_event, tile, route_escape);
    // Ordered records keep a same-frame native event start ahead of its
    // boundary departure and record each state transition once.
    for observation in route_update.log_events().into_iter().flatten() {
        record_random_route_observation(actor, writer, run_started, tile, observation);
    }

    if let Some(tile) = tile {
        let previous = actor.last_tile;
        if actor.script_started_at.is_some() && previous.is_some_and(|last| last != tile) {
            actor.walk_tile_changes = actor.walk_tile_changes.saturating_add(1);
            if worksite_exit(NORMAL_SAFE_WALK, previous, tile) {
                match time_ms() {
                    Ok(wall_ms) => {
                        let mut event = event_base("worksite-exit", actor, run_started, wall_ms);
                        event["previous_tile"] = json!(previous);
                        event["tile"] = json!(tile);
                        if let Err(error) = writer.write(&event) {
                            actor.failure = Some(error);
                        }
                    }
                    Err(error) => actor.failure = Some(error),
                }
            }
        }
        actor.last_tile = Some(tile);
        if route_oracle_failure(
            actor.script_started_at.is_some(),
            route_escape,
            &route_update,
        ) {
            fail_actor(
                actor,
                writer,
                run_started,
                "site-walk-boundary",
                &format!(
                    "work/bank route entered a danger envelope or left the outer bound at {tile:?}"
                ),
            );
        }
    }

    if matches!(
        actor.runner.status(),
        RunnerStatus::Passed | RunnerStatus::Failed(_)
    ) {
        if let RunnerStatus::Failed(message) = actor.runner.status() {
            fail_actor(actor, writer, run_started, "scenario", &message);
        }
        return;
    }
    if actor.script_requested && !actor.script_running_observed {
        // The host loop measures the real Start-to-Running budget. Do not
        // let the runner charge the cohort barrier's unobserved tick delta.
        return;
    }
    let at_relog = actor.runner.record_step().2 == GATHERER_RELOG_STEP && client.ingame;
    let before_output = client.out.pos;
    actor.runner.tick_with_hold(client, frame.hold);
    if at_relog && !actor.setup_relog_requested && client.out.pos > before_output {
        actor.setup_relog_requested = true;
        if let Ok(wall_ms) = time_ms() {
            let mut event = event_base("setup-relog-requested", actor, run_started, wall_ms);
            event["logout_packet_output_bytes"] = json!(client.out.pos - before_output);
            if let Err(error) = writer.write(&event) {
                actor.failure = Some(error);
            }
        }
    }
    if let RunnerStatus::Failed(message) = actor.runner.status() {
        fail_actor(actor, writer, run_started, "scenario", &message);
    }
}

fn record_unexpected_exit(
    actor: &mut Actor,
    writer: &EventWriter,
    run_started: Instant,
    kind: &str,
) {
    if actor.exit_recorded {
        return;
    }
    actor.exit_recorded = true;
    let detail = format!("unexpected {kind}");
    fail_actor(actor, writer, run_started, "unexpected-exit", &detail);
    if let Ok(wall_ms) = time_ms() {
        let mut event = event_base("unexpected-exit", actor, run_started, wall_ms);
        event["kind"] = json!(kind);
        let _ = writer.write(&event);
    }
}

fn record_setup_relog_exit(
    actor: &mut Actor,
    writer: &EventWriter,
    run_started: Instant,
    kind: &str,
) -> bool {
    if !actor.setup_relog_requested
        || actor.setup_relog_exit_observed
        || actor.runner.record_step().2 != GATHERER_RELOG_STEP
    {
        return false;
    }
    actor.setup_relog_exit_observed = true;
    if let Ok(wall_ms) = time_ms() {
        let mut event = event_base("setup-relog-exit", actor, run_started, wall_ms);
        event["kind"] = json!(kind);
        if let Err(error) = writer.write(&event) {
            actor.failure = Some(error);
        }
    }
    true
}

fn account_status_rows(play: &host_play::Play) -> BTreeMap<String, host_play::SlotStatus> {
    play.statuses()
        .into_iter()
        .map(|status| (status.username.clone(), status))
        .collect()
}

fn slot_sample_json(
    actor: &Actor,
    slot: Option<&host_play::SlotStatus>,
    script_state: script::RunState,
    now_monotonic_ns: u64,
) -> Value {
    json!({
        "account": actor.account,
        "site_id": actor.site.id,
        "worksite_start": [actor.site.start.x, actor.site.start.z, actor.site.start.level],
        "connected": slot.is_some_and(|status| status.connected),
        "stream_epoch": slot.map(|status| status.stream_epoch),
        "ingame": slot.is_some_and(|status| status.ingame),
        "scene_state": slot.map(|status| status.scene_state),
        "tile": slot.and_then(host_play::SlotStatus::ready_tile),
        "trapped_random_event": actor.snapshot.tile()
            .and_then(|(x, z, level)| api::random::trapped_area(x, z, level))
            .map(|kind| format!("{kind:?}")),
        "script_state": format!("{script_state:?}"),
        "scenario_step": actor.runner.record_step(),
        "scenario_status": format!("{:?}", actor.runner.status()),
        "script": status_json(actor.last_status.as_ref()),
        "scene2_unix_ms": actor.scene2_unix_ms,
        "scene2_monotonic_ns": actor.scene2_monotonic_ns,
        "scene2_continuous_seconds": actor.scene2_monotonic_ns
            .and_then(|since| now_monotonic_ns.checked_sub(since))
            .map(|elapsed| elapsed / 1_000_000_000),
        "script_started_unix_ms": actor.script_started_unix_ms,
        "bank_cycles": actor.bank_cycles,
        "death_events_observed": actor.death_events_observed,
        "last_death_sequence": actor.last_death_sequence,
        "walk_tile_changes": actor.walk_tile_changes,
        "warmup_qualified": actor.warmup_qualified,
        "slot_ready_monotonic_ns": actor.slot_ready_monotonic_ns,
    })
}

fn actor_summaries(
    actors: &BTreeMap<String, Arc<Mutex<Actor>>>,
    play: Option<&host_play::Play>,
    phase_zero: Option<&PhaseZero>,
    as_of_monotonic_ns: Option<u64>,
) -> Result<Vec<Value>, String> {
    let statuses = play.map(account_status_rows).unwrap_or_default();
    let phase_zero_ns = phase_zero.map(|value| value.monotonic_ns);
    actors
        .iter()
        .map(|(account, actor)| {
            let actor = actor.lock().map_err(|_| "actor state poisoned")?;
            let status = statuses.get(account);
            let script_state = play
                .map(|play| play.script_state(account))
                .unwrap_or(script::RunState::Idle);
            let scene2_elapsed_seconds = actor
                .scene2_monotonic_ns
                .zip(phase_zero_ns)
                .and_then(|(at, origin)| at.checked_sub(origin))
                .map(|elapsed| elapsed / 1_000_000_000);
            let bank_cycle_elapsed_seconds = actor
                .first_bank_cycle_monotonic_ns
                .zip(phase_zero_ns)
                .and_then(|(at, origin)| at.checked_sub(origin))
                .map(|elapsed| elapsed / 1_000_000_000);
            let scene2_continuous_seconds = actor
                .scene2_monotonic_ns
                .zip(as_of_monotonic_ns)
                .and_then(|(since, now)| now.checked_sub(since))
                .map(|elapsed| elapsed / 1_000_000_000);
            Ok(json!({
                "account": account,
                "site_id": actor.site.id,
                "resource": actor.site.resource,
                "worksite_start": [actor.site.start.x, actor.site.start.z, actor.site.start.level],
                "product_id": actor.site.product_id,
                "level": actor.site.level,
                "expected_bank_prefix": actor.site.bank_prefix,
                "scene2_unix_ms": actor.scene2_unix_ms,
                "scene2_monotonic_ns": actor.scene2_monotonic_ns,
                "scene2_phase_zero_elapsed_seconds": scene2_elapsed_seconds,
                "scene2_continuous_seconds_at_finish": scene2_continuous_seconds,
                "script_started_unix_ms": actor.script_started_unix_ms,
                "script_running_observed": actor.script_running_observed,
                "script_running_monotonic_ns": actor.script_running_monotonic_ns,
                "script_state": format!("{script_state:?}"),
                "bank_cycles": actor.bank_cycles,
                "death_events_observed": actor.death_events_observed,
                "last_death_sequence": actor.last_death_sequence,
                "first_bank_cycle_process_elapsed_seconds": actor.first_bank_cycle_elapsed_seconds,
                "first_bank_cycle_monotonic_ns": actor.first_bank_cycle_monotonic_ns,
                "first_bank_cycle_phase_zero_elapsed_seconds": bank_cycle_elapsed_seconds,
                "walk_tile_changes": actor.walk_tile_changes,
                "warmup_qualified": actor.warmup_qualified,
                "slot_ready_monotonic_ns": actor.slot_ready_monotonic_ns,
                "connected_ever": actor.connected_ever,
                "last_stream_epoch": actor.last_stream_epoch,
                "setup_relog_requested": actor.setup_relog_requested,
                "setup_relog_exit_observed": actor.setup_relog_exit_observed,
                "setup_relog_returned": actor.setup_relog_returned,
                "connected_at_finish": status.is_some_and(|row| row.connected),
                "worker_terminal": status.and_then(|row| row.worker_terminal.map(|terminal| format!("{terminal:?}"))),
                "last_task_status": status_json(actor.last_status.as_ref()),
                "failure": actor.failure,
            }))
        })
        .collect()
}

struct SummaryTiming<'a> {
    run_started: Instant,
    phase_zero: Option<&'a PhaseZero>,
    warmup_end_monotonic_ns: Option<u64>,
    end_monotonic_ns: Option<u64>,
}

fn write_summary(
    output: &Path,
    config: &RunConfig,
    timing: SummaryTiming<'_>,
    actors: &BTreeMap<String, Arc<Mutex<Actor>>>,
    play: Option<&host_play::Play>,
    outcome: &str,
    unexpected_exits: u64,
) -> Result<(), String> {
    let SummaryTiming {
        run_started,
        phase_zero,
        warmup_end_monotonic_ns,
        end_monotonic_ns,
    } = timing;
    let phase_zero_ns = phase_zero.map(|value| value.monotonic_ns);
    let warmup_elapsed_seconds = warmup_end_monotonic_ns
        .zip(phase_zero_ns)
        .and_then(|(end, origin)| end.checked_sub(origin))
        .map(|elapsed| elapsed as f64 / 1_000_000_000.0);
    let slope_window_seconds =
        warmup_elapsed_seconds.map(|elapsed| (config.duration.as_secs_f64() - elapsed).max(0.0));
    let phase_zero_record = phase_zero.map(|value| {
        json!({
            "monotonic_ns": value.monotonic_ns,
            "account": value.account,
            "seed": value.seed,
            "at_utc": value.at_utc,
            "elapsed_seconds_from_proxy_start": value.elapsed_seconds,
        })
    });
    let slots = actor_summaries(actors, play, phase_zero, end_monotonic_ns)?;
    let summary = json!({
        "schema_version": 1,
        "result": outcome,
        "qualification": config.qualification,
        "external_receipt_evaluation": "pending",
        "source_config": config.source_config,
        "run": {
            "slots": config.slots,
            "duration_seconds": config.duration.as_secs(),
            "duration_anchor": "first-script-start-after-all-slot-setup",
            "phase_zero": phase_zero_record,
            "phase_zero_monotonic_ns": phase_zero_ns,
            "warmup_min_seconds": config.warmup.as_secs(),
            "warmup_end_monotonic_ns": warmup_end_monotonic_ns,
            "warmup_end_elapsed_seconds": warmup_elapsed_seconds,
            "slope_window_seconds": slope_window_seconds,
            "window_end_monotonic_ns": end_monotonic_ns,
            "sample_seconds": config.sample.as_secs(),
            "hash_seconds": config.hash.as_secs(),
            "name_prefix": config.name_prefix,
            "process_elapsed_seconds": run_started.elapsed().as_secs(),
        },
        "limits": {
            "max_unexpected_exits": config.max_unexpected_exits,
            "unexpected_exits": unexpected_exits,
            "max_payload_divergences": config.max_payload_divergences,
            "connection_bytes": config.connection_bytes,
            "rss_slope_baseline_slack_bytes_per_hour": config.rss_slope_baseline_slack_bytes_per_hour,
            "rss_final_baseline_extra_bytes_per_connection": config.rss_final_baseline_extra_bytes_per_connection,
            "rss_final_baseline_extra_percent": config.rss_final_baseline_extra_percent,
            "fixed_threads": config.fixed_threads,
        },
        "sites": config.sites.iter().map(|site| json!({
            "site_id": site.id,
            "resource": site.resource,
            "product_id": site.product_id,
            "level": site.level,
            "expected_bank_prefix": site.bank_prefix,
            "start_tile": site.start,
        })).collect::<Vec<_>>(),
        "slots": slots,
    });

    let path = output.join("summary.json");
    let temporary = output.join("summary.json.tmp");
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(|error| format!("create {}: {error}", temporary.display()))?;
    let mut writer = BufWriter::new(file);
    serde_json::to_writer_pretty(&mut writer, &summary)
        .map_err(|error| format!("serialize summary: {error}"))?;
    writer
        .write_all(b"\n")
        .map_err(|error| format!("write summary newline: {error}"))?;
    writer
        .flush()
        .map_err(|error| format!("flush summary: {error}"))?;
    writer
        .get_ref()
        .sync_all()
        .map_err(|error| format!("sync summary: {error}"))?;
    fs::rename(temporary, path).map_err(|error| format!("publish summary: {error}"))
}

fn slot_ready_at_monotonic_ns(actor: &Actor, qualification: bool) -> Option<u64> {
    let scene2 = actor.scene2_monotonic_ns?;
    if qualification {
        let first_bank_cycle = actor.first_bank_cycle_monotonic_ns?;
        Some(scene2.max(first_bank_cycle))
    } else {
        let script_running = actor.script_running_monotonic_ns?;
        let task_status = actor.task_status_monotonic_ns?;
        Some(scene2.max(script_running).max(task_status))
    }
}

fn cohort_ready_at_monotonic_ns(
    actors: &BTreeMap<String, Arc<Mutex<Actor>>>,
) -> Result<Option<u64>, String> {
    let mut latest_ready_ns = 0u64;
    for actor in actors.values() {
        let actor = actor.lock().map_err(|_| "actor state poisoned")?;
        let Some(ready_ns) = actor.slot_ready_monotonic_ns else {
            return Ok(None);
        };
        latest_ready_ns = latest_ready_ns.max(ready_ns);
    }
    Ok(Some(latest_ready_ns))
}
fn run_live(config_path: &Path, output: &Path) -> Result<(), String> {
    if !cfg!(not(debug_assertions)) {
        return Err("T1.4 live soak must run with the release profile".into());
    }
    if std::env::var("LIVE").as_deref() != Ok("1") {
        return Err("T1.4 live soak requires LIVE=1".into());
    }
    let config = parse_config(config_path)?;
    validate_isolated_tls(&config, output)?;
    let output =
        fs::canonicalize(output).map_err(|error| format!("canonicalize output: {error}"))?;
    if std::env::var("BOT_LIVE_NAME_PREFIX").ok().as_deref() != Some(config.name_prefix.as_str()) {
        return Err("BOT_LIVE_NAME_PREFIX must equal run.name_prefix".into());
    }
    if std::env::var("BOT_SERVER_PROFILE").ok().as_deref() != Some(config.profile_name.as_str()) {
        return Err("BOT_SERVER_PROFILE must equal profile.name".into());
    }
    if !matches!(config.host.as_str(), "127.0.0.1" | "localhost")
        || !matches!(config.asset_host.as_str(), "127.0.0.1" | "localhost")
    {
        return Err("the soak may connect only to the configured loopback proxy".into());
    }

    let event_writer = Arc::new(EventWriter::create(&output)?);
    #[cfg(feature = "soak-observers")]
    candidate_observers::install_tls(&config.ca_pem)?;
    #[cfg(feature = "soak-observers")]
    let candidate_payloads = candidate_observers::CandidateObservers::install(&output)?;
    // The native-TLS baseline lacks this API. Candidate runs enable the
    // observer feature, which also installs their private test trust.
    #[cfg(feature = "soak-observers")]
    let _network_driver = host_play::PlayComposition::start()
        .map_err(|error| format!("T1.4 network composition startup: {error}"))?;

    let (_profile, template) = load_profile(&config)?;
    let names = host_play::mint_live_names(config.slots);
    if names.len() != config.slots
        || names
            .iter()
            .collect::<std::collections::BTreeSet<_>>()
            .len()
            != config.slots
    {
        return Err("host-play did not mint the configured number of unique accounts".into());
    }
    let credentials = host_play::mint_live_entries(&names);
    if credentials.len() != config.slots {
        return Err("host-play did not mint the configured number of account credentials".into());
    }
    let base_uid = time_ms()? % i32::MAX as u128;
    let profiles = credentials
        .into_iter()
        .enumerate()
        .map(|(index, (username, password))| {
            let uid = (base_uid as i32).saturating_add(index as i32);
            Profile {
                username,
                password: password.into(),
                uid,
                settings: ProfileSettings::default(),
            }
        })
        .collect::<Vec<_>>();

    let run_started = Instant::now();
    let mut actors = BTreeMap::new();
    for (index, account) in names.iter().enumerate() {
        let site = site_for_slot(index, &config.sites);
        let actor = make_actor(
            account.clone(),
            site,
            &template,
            config.duration + config.warmup,
        )?;
        actors.insert(account.clone(), Arc::new(Mutex::new(actor)));
    }
    let frame_actors = actors.clone();
    let frame_writer = Arc::clone(&event_writer);
    let finalizing = Arc::new(AtomicBool::new(false));
    let frame_finalizing = Arc::clone(&finalizing);

    // This receipt is the external sampler's handshake; publish it atomically
    // immediately before production Play starts its slot workers.
    write_pid_atomic(&output)?;
    let play = host_play::run_with_template(
        Arc::clone(&template),
        true,
        profiles,
        |_| (None, None),
        move |client, username, frame| {
            if frame_finalizing.load(Ordering::Acquire) {
                return;
            }
            let Some(actor) = frame_actors.get(username) else {
                return;
            };
            // WSS is deliberately classified Remote by the product, even on
            // loopback. Only this isolated LIVE test grants local Engine debug
            // seeding after checking the proxy, asset and Engine endpoints.
            client.set_cheat_admission(client::CheatAdmission::Granted);
            let mut actor = match actor.lock() {
                Ok(actor) => actor,
                Err(_) => return,
            };
            drive_frame(&mut actor, client, frame, &frame_writer, run_started);
        },
    )
    .map_err(|error| error.to_string())?;

    let mut unexpected_exits = 0u64;
    let run_start_monotonic_ns = monotonic_now_ns()?;
    let duration_ns = u64::try_from(config.duration.as_nanos())
        .map_err(|_| "configured run duration exceeds monotonic nanosecond range")?;
    let warmup_ns = u64::try_from(config.warmup.as_nanos())
        .map_err(|_| "configured warmup exceeds monotonic nanosecond range")?;
    let sample_ns = u64::try_from(config.sample.as_nanos())
        .map_err(|_| "configured sample interval exceeds monotonic nanosecond range")?;
    let startup_deadline_ns = run_start_monotonic_ns
        .checked_add(duration_ns)
        .ok_or("startup deadline exceeds monotonic nanosecond range")?;
    let mut proxy_events = ProxyEventReader::new(output.join("proxy").join("connections.jsonl"));
    let mut phase_zero: Option<PhaseZero> = None;
    let mut published_start: Option<(String, u64)> = None;
    let mut warmup_end_monotonic_ns = None;
    let mut next_sample_monotonic_ns = None;
    let mut end_monotonic_ns = None;
    let mut next_health_check = run_started + Duration::from_secs(1);
    let mut outcome = "incomplete";
    let loop_result = (|| -> Result<(), String> {
        loop {
            let now = Instant::now();
            let now_monotonic_ns = monotonic_now_ns()?;
            let at_run_deadline = phase_zero.as_ref().is_some_and(|phase_zero| {
                phase_zero
                    .monotonic_ns
                    .checked_add(duration_ns)
                    .is_some_and(|deadline| now_monotonic_ns >= deadline)
            });
            if now >= next_health_check || at_run_deadline {
                next_health_check = now + Duration::from_secs(1);
                let statuses = account_status_rows(&play);
                for (account, actor_ref) in &actors {
                    let mut actor = actor_ref.lock().map_err(|_| "actor state poisoned")?;
                    if let Some(status) = statuses.get(account) {
                        if status.connected {
                            // The first sampled status can be the logout gap.
                            // A connected return after an observed exit does not
                            // require a sampled pre-logout stream epoch.
                            if (actor.setup_relog_exit_observed && !actor.setup_relog_returned
                                || actor.connected_ever
                                    && actor
                                        .last_stream_epoch
                                        .is_some_and(|previous| previous != status.stream_epoch))
                                && !actor.exit_recorded
                            {
                                if published_start.is_none()
                                    && (actor.setup_relog_exit_observed
                                        && !actor.setup_relog_returned
                                        || record_setup_relog_exit(
                                            &mut actor,
                                            &event_writer,
                                            run_started,
                                            "stream-epoch-changed",
                                        ))
                                {
                                    actor.setup_relog_returned = true;
                                    let event = event_base(
                                        "setup-relog-returned",
                                        &actor,
                                        run_started,
                                        time_ms()?,
                                    );
                                    event_writer.write(&event)?;
                                } else {
                                    unexpected_exits = unexpected_exits.saturating_add(1);
                                    record_unexpected_exit(
                                        &mut actor,
                                        &event_writer,
                                        run_started,
                                        "stream-epoch-changed",
                                    );
                                }
                            }
                            actor.connected_ever = true;
                            actor.last_stream_epoch = Some(status.stream_epoch);
                        } else if (actor.connected_ever || status.stream_epoch != 0)
                            && !actor.exit_recorded
                        {
                            actor.connected_ever = true;
                            if published_start.is_none()
                                && (actor.setup_relog_exit_observed && !actor.setup_relog_returned
                                    || record_setup_relog_exit(
                                        &mut actor,
                                        &event_writer,
                                        run_started,
                                        "connection",
                                    ))
                            {
                                // A pending clean logout may take many seconds while
                                // Engine A saves; observe its return by stream epoch.
                            } else {
                                unexpected_exits = unexpected_exits.saturating_add(1);
                                record_unexpected_exit(
                                    &mut actor,
                                    &event_writer,
                                    run_started,
                                    "connection",
                                );
                            }
                        }
                        if status.worker_terminal.is_some() && !actor.exit_recorded {
                            unexpected_exits = unexpected_exits.saturating_add(1);
                            record_unexpected_exit(
                                &mut actor,
                                &event_writer,
                                run_started,
                                "worker-terminal",
                            );
                        }
                        if let Some(error) = status.error.as_ref() {
                            if !actor.connected_ever {
                                fail_actor(
                                    &mut actor,
                                    &event_writer,
                                    run_started,
                                    "slot-login-error",
                                    if error.is_empty() {
                                        "slot login error"
                                    } else {
                                        error.as_str()
                                    },
                                );
                            }
                        }
                    }
                    if actor.failure.is_some() {
                        continue;
                    }
                    if !actor.script_requested && actor.runner.on_start_script() {
                        if !actor.setup_relog_requested
                            || !actor.setup_relog_exit_observed
                            || !actor.setup_relog_returned
                            || !statuses.get(account).is_some_and(|status| status.connected)
                        {
                            fail_actor(
                        &mut actor, &event_writer, run_started, "setup-relog-proof",
                        "Gatherer reached Start without a requested logout, observed exit and connected return",
                    );
                            continue;
                        }
                        actor.script_requested = true;
                        let event =
                            event_base("script-start-ready", &actor, run_started, time_ms()?);
                        event_writer.write(&event)?;
                    }
                    if actor.script_started_at.is_some() {
                        let script_state = play.script_state(account);
                        if !actor.script_running_observed
                            && script_start_observation(
                                actor.script_start_tick,
                                actor.snapshot.tick(),
                                matches!(script_state, script::RunState::Running),
                                actor.script_start_budget_ticks,
                            ) == ScriptStartObservation::Expired
                        {
                            let message = format!(
                                "script_running not seen within {} game ticks after actual Start",
                                actor.script_start_budget_ticks
                            );
                            fail_actor(
                                &mut actor,
                                &event_writer,
                                run_started,
                                "script-start-budget",
                                &message,
                            );
                            continue;
                        }
                        match script_state {
                            script::RunState::Running if !actor.script_running_observed => {
                                actor.script_running_observed = true;
                                actor.script_running_monotonic_ns = Some(now_monotonic_ns);
                                actor.runner.observe_script_running();
                                let wall_ms = time_ms()?;
                                let mut event =
                                    event_base("script-running", &actor, run_started, wall_ms);
                                event["script_running_monotonic_ns"] = json!(now_monotonic_ns);
                                event_writer.write(&event)?;
                            }
                            script::RunState::Error
                            | script::RunState::Idle
                            | script::RunState::Stopping
                                if actor.script_running_observed =>
                            {
                                unexpected_exits = unexpected_exits.saturating_add(1);
                                record_unexpected_exit(
                                    &mut actor,
                                    &event_writer,
                                    run_started,
                                    "script",
                                );
                            }
                            _ => {}
                        }
                        if let Some(status) = play.script_native_status(account).as_deref() {
                            actor
                                .task_status_monotonic_ns
                                .get_or_insert(now_monotonic_ns);
                            let sample = status_sample(status);
                            let tile = actor.snapshot.tile();
                            let route_escape =
                                tile.map(|tile| work_route_escape(actor.site.outer_bounds, tile));
                            let route_update = actor
                                .random_route
                                .observe_work_progress(&sample, route_escape);
                            for observation in route_update.log_events().into_iter().flatten() {
                                record_random_route_observation(
                                    &mut actor,
                                    &event_writer,
                                    run_started,
                                    tile,
                                    observation,
                                );
                            }
                            let bank = sample.bank.as_deref().unwrap_or("");
                            let trips = sample.trips.unwrap_or(0);
                            if trips < actor.last_trips {
                                fail_actor(
                                    &mut actor,
                                    &event_writer,
                                    run_started,
                                    "task-status",
                                    "Gatherer trip counter decreased",
                                );
                            } else if trips > actor.last_trips {
                                if !bank.starts_with(actor.site.bank_prefix)
                                    || sample.deposited.unwrap_or(0) == 0
                                    || actor.walk_tile_changes == 0
                                {
                                    fail_actor(&mut actor, &event_writer, run_started, "bank-cycle-proof", "trip counter lacks the selected bank, deposited product and post-Start movement evidence");
                                } else {
                                    let new_cycles = trips - actor.last_trips;
                                    actor.bank_cycles =
                                        actor.bank_cycles.saturating_add(new_cycles);
                                    actor
                                        .first_bank_cycle_elapsed_seconds
                                        .get_or_insert_with(|| run_started.elapsed().as_secs());
                                    actor
                                        .first_bank_cycle_monotonic_ns
                                        .get_or_insert(now_monotonic_ns);
                                    let wall_ms = time_ms()?;
                                    let mut event =
                                        event_base("bank-cycle", &actor, run_started, wall_ms);
                                    event["bank_cycle_monotonic_ns"] = json!(now_monotonic_ns);
                                    event["trips"] = json!(trips);
                                    event["deposited"] = json!(sample.deposited);
                                    event["yielded"] = json!(sample.yielded);
                                    event["bank"] = json!(bank);
                                    event["product_id"] = json!(actor.site.product_id);
                                    event["walk_tile_changes"] = json!(actor.walk_tile_changes);
                                    event_writer.write(&event)?;
                                }
                                actor.last_trips = trips;
                            }
                            if sample.deaths.unwrap_or(0) > 0 {
                                fail_actor(
                                    &mut actor,
                                    &event_writer,
                                    run_started,
                                    "unexpected-death",
                                    "Gatherer reported a death",
                                );
                            }
                            if status.failure.is_some() {
                                fail_actor(
                                    &mut actor,
                                    &event_writer,
                                    run_started,
                                    "native-task-failure",
                                    "Gatherer published a failed native status",
                                );
                            }
                            actor.last_status = Some(sample);
                        }
                    }
                    if !actor.warmup_qualified {
                        if let Some(ready_monotonic_ns) =
                            slot_ready_at_monotonic_ns(&actor, config.qualification)
                        {
                            actor.warmup_qualified = true;
                            actor.slot_ready_monotonic_ns = Some(ready_monotonic_ns);
                            let wall_ms = time_ms()?;
                            let mut event =
                                event_base("slot-workload-ready", &actor, run_started, wall_ms);
                            event["ready_monotonic_ns"] = json!(ready_monotonic_ns);
                            event["has_bank_cycle_proof"] =
                                json!(actor.first_bank_cycle_monotonic_ns.is_some());
                            event_writer.write(&event)?;
                        }
                    }
                }
                // All slots must complete unfaulted setup, including the stock
                // Relog, before the first script Start opens the fault window.
                if published_start.is_none()
                    && actors.values().try_fold(true, |ready, actor| {
                        actor
                            .lock()
                            .map(|actor| ready && actor.script_requested)
                            .map_err(|_| "actor state poisoned")
                    })?
                {
                    for (account, actor_ref) in &actors {
                        let site = {
                            let actor = actor_ref.lock().map_err(|_| "actor state poisoned")?;
                            actor.site.clone()
                        };
                        let start_tick = {
                            let actor = actor_ref.lock().map_err(|_| "actor state poisoned")?;
                            event_writer.write(&event_base(
                                "script-start-requested",
                                &actor,
                                run_started,
                                time_ms()?,
                            ))?;
                            actor.snapshot.tick()
                        };
                        match play.script_start(
                            account,
                            script::CompiledId("Gatherer"),
                            bank_settings(&site),
                        ) {
                            Ok(()) => {
                                let observed_ns = monotonic_now_ns()?;
                                let wall_ms = time_ms()?;
                                if published_start.is_none() {
                                    write_soak_start_atomic(
                                        &output,
                                        account,
                                        config.fault_seed,
                                        observed_ns,
                                        wall_ms,
                                    )?;
                                    published_start = Some((account.clone(), observed_ns));
                                }
                                let mut actor =
                                    actor_ref.lock().map_err(|_| "actor state poisoned")?;
                                actor.script_started_at = Some(Instant::now());
                                actor.script_started_unix_ms = Some(wall_ms);
                                actor.script_start_tick = Some(start_tick);
                                let mut event =
                                    event_base("script-started", &actor, run_started, wall_ms);
                                event["script_start_monotonic_ns"] = json!(observed_ns);
                                event_writer.write(&event)?;
                            }
                            Err(error) => {
                                let mut actor =
                                    actor_ref.lock().map_err(|_| "actor state poisoned")?;
                                actor.runner.fail_start(&error);
                                fail_actor(
                                    &mut actor,
                                    &event_writer,
                                    run_started,
                                    "script-start",
                                    &error,
                                );
                                break;
                            }
                        }
                    }
                }
                if phase_zero.is_none() {
                    if let Some(record) = proxy_events.poll(config.fault_seed)? {
                        if record.monotonic_ns > startup_deadline_ns {
                            break Err(
                                "first script Start occurred after the startup deadline".into()
                            );
                        }
                        if published_start.as_ref()
                            != Some(&(record.account.clone(), record.monotonic_ns))
                        {
                            break Err(
                                "proxy fault phase does not match first Play script Start".into()
                            );
                        }
                        let event = json!({
                            "schema_version": 1,
                            "event": "phase-zero",
                            "elapsed_seconds": 0,
                            "phase_zero_monotonic_ns": record.monotonic_ns,
                            "account": record.account,
                            "seed": record.seed,
                            "at_utc": record.at_utc,
                            "elapsed_seconds_from_proxy_start": record.elapsed_seconds,
                        });
                        event_writer.write(&event)?;
                        #[cfg(feature = "soak-observers")]
                        candidate_payloads.sample(
                            now_monotonic_ns.saturating_sub(record.monotonic_ns) / 1_000_000_000,
                        )?;
                        next_sample_monotonic_ns =
                            Some(record.monotonic_ns.checked_add(sample_ns).ok_or(
                                "first sample deadline exceeds monotonic nanosecond range",
                            )?);
                        phase_zero = Some(record);
                    }
                }
            }

            let failures = actors.values().try_fold(false, |failed, actor| {
                actor
                    .lock()
                    .map(|actor| failed || actor.failure.is_some())
                    .map_err(|_| "actor state poisoned")
            })?;
            if failures {
                break Err("one or more Play slots failed their live soak witness".to_owned());
            }
            if phase_zero.is_none() && now_monotonic_ns >= startup_deadline_ns {
                break Err("no script Start was observed before the startup deadline".into());
            }
            if let Some(phase_zero) = phase_zero.as_ref() {
                let run_deadline_ns = phase_zero
                    .monotonic_ns
                    .checked_add(duration_ns)
                    .ok_or("run deadline exceeds monotonic nanosecond range")?;
                if now_monotonic_ns >= run_deadline_ns {
                    if config.qualification {
                        let Some(warmup_end) = warmup_end_monotonic_ns else {
                            break Err("configured phase-zero window ended before all slots completed warmup".into());
                        };
                        if warmup_end >= run_deadline_ns {
                            break Err("warmup left no positive RSS slope window".into());
                        }
                    }
                    end_monotonic_ns = Some(run_deadline_ns);
                    outcome = if config.qualification {
                        "workload-window-complete-awaiting-external-evaluation"
                    } else if config.slots == 20 {
                        "diagnostic-complete-nonqualifying"
                    } else {
                        "smoke-complete-nonqualifying"
                    };
                    break Ok(());
                }
                if warmup_end_monotonic_ns.is_none() {
                    if let Some(latest_ready_ns) = cohort_ready_at_monotonic_ns(&actors)? {
                        let minimum_warmup_end_ns = phase_zero
                            .monotonic_ns
                            .checked_add(warmup_ns)
                            .ok_or("warmup deadline exceeds monotonic nanosecond range")?;
                        let target_warmup_end_ns = latest_ready_ns.max(minimum_warmup_end_ns);
                        if config.qualification && target_warmup_end_ns >= run_deadline_ns {
                            break Err(
                                "cohort workload readiness leaves no positive RSS slope window"
                                    .into(),
                            );
                        }
                        if now_monotonic_ns >= target_warmup_end_ns {
                            warmup_end_monotonic_ns = Some(target_warmup_end_ns);
                            let warmup_elapsed_seconds =
                                (target_warmup_end_ns - phase_zero.monotonic_ns) as f64
                                    / 1_000_000_000.0;
                            let slope_window_seconds =
                                (config.duration.as_secs_f64() - warmup_elapsed_seconds).max(0.0);
                            let event = json!({
                                "schema_version": 1,
                                "event": "warmup-end",
                                "phase_zero_monotonic_ns": phase_zero.monotonic_ns,
                                "warmup_end_monotonic_ns": target_warmup_end_ns,
                                "warmup_end_elapsed_seconds": warmup_elapsed_seconds,
                                "slope_window_seconds": slope_window_seconds,
                                "qualification": config.qualification,
                            });
                            event_writer.write(&event)?;
                        }
                    }
                }
                if let Some(next_sample_ns) = next_sample_monotonic_ns {
                    if now_monotonic_ns >= next_sample_ns {
                        let elapsed_ns = now_monotonic_ns.saturating_sub(phase_zero.monotonic_ns);
                        let elapsed_seconds = elapsed_ns / 1_000_000_000;
                        let statuses = account_status_rows(&play);
                        let mut samples = Vec::with_capacity(actors.len());
                        for (account, actor_ref) in &actors {
                            let actor = actor_ref.lock().map_err(|_| "actor state poisoned")?;
                            samples.push(slot_sample_json(
                                &actor,
                                statuses.get(account),
                                play.script_state(account),
                                now_monotonic_ns,
                            ));
                        }
                        let sample = json!({
                            "schema_version": 1,
                            "event": "sample",
                            "elapsed_seconds": elapsed_seconds,
                            "sample_monotonic_ns": now_monotonic_ns,
                            "phase_zero_monotonic_ns": phase_zero.monotonic_ns,
                            "slots": samples,
                        });
                        event_writer.write(&sample)?;
                        #[cfg(feature = "soak-observers")]
                        candidate_payloads.sample(elapsed_seconds)?;
                        let next_index = elapsed_ns
                            .checked_div(sample_ns)
                            .and_then(|index| index.checked_add(1))
                            .ok_or("next sample index overflow")?;
                        next_sample_monotonic_ns = Some(
                            phase_zero
                                .monotonic_ns
                                .checked_add(
                                    next_index
                                        .checked_mul(sample_ns)
                                        .ok_or("next sample deadline overflow")?,
                                )
                                .ok_or("next sample deadline overflow")?,
                        );
                    }
                }
            }
            let tick = Duration::from_millis(TICK_MS);
            let remaining_ns = phase_zero
                .as_ref()
                .and_then(|phase_zero| phase_zero.monotonic_ns.checked_add(duration_ns))
                .unwrap_or(startup_deadline_ns)
                .checked_sub(now_monotonic_ns);
            let sleep = remaining_ns
                .map(Duration::from_nanos)
                .map_or(tick, |remaining| remaining.min(tick));
            thread::sleep(sleep);
        }
    })();

    if end_monotonic_ns.is_none() {
        end_monotonic_ns = monotonic_now_ns().ok();
    }
    if loop_result.is_err() {
        outcome = "failed";
    }
    // The live summary must observe Play before it shuts down the slots.
    let summary_result = write_summary(
        &output,
        &config,
        SummaryTiming {
            run_started,
            phase_zero: phase_zero.as_ref(),
            warmup_end_monotonic_ns,
            end_monotonic_ns,
        },
        &actors,
        Some(&play),
        outcome,
        unexpected_exits,
    );
    // Keep receive processing alive through the server's logout and Close.
    // Dropping Play first suspends the endpoint reader and is not a drain.
    finalizing.store(true, Ordering::Release);
    let completed = loop_result.is_ok() && outcome != "incomplete" && outcome != "failed";
    let mut barrier_result = Ok(None);
    if completed {
        barrier_result = (|| -> Result<Option<u64>, String> {
            for account in &names {
                let arm = play.arm(account).ok_or("final logout lacks slot arm")?;
                arm.request_logout();
                play.wake(account);
            }
            // Same logout bound as the host-play relog live proof.
            let deadline = Instant::now() + Duration::from_secs(30);
            loop {
                #[cfg(feature = "soak-observers")]
                _network_driver.maintain_services(Instant::now());
                let statuses = account_status_rows(&play);
                if names
                    .iter()
                    .all(|name| statuses.get(name).is_some_and(|row| !row.connected))
                {
                    break;
                }
                if Instant::now() >= deadline {
                    return Err("final receive barrier timed out awaiting ordinary logout".into());
                }
                thread::sleep(Duration::from_millis(TICK_MS));
            }
            Ok(None)
        })();
    }
    if completed && barrier_result.is_ok() {
        barrier_result = (|| -> Result<Option<u64>, String> {
            let mut barrier =
                final_barrier::CloseBarrier::new(output.join("proxy").join("connections.jsonl"));
            let deadline = Instant::now() + Duration::from_secs(30);
            loop {
                #[cfg(feature = "soak-observers")]
                _network_driver.maintain_services(Instant::now());
                if barrier.poll()? {
                    return monotonic_now_ns().map(Some);
                }
                if Instant::now() >= deadline {
                    return Err(
                        "final receive barrier lacks complete bidirectional empty Close receipts"
                            .into(),
                    );
                }
                thread::sleep(Duration::from_millis(TICK_MS));
            }
        })();
    }
    drop(play);
    #[cfg(feature = "soak-observers")]
    if let Ok(network) = client_adapter::service::NetworkService::current_started() {
        network.shutdown();
    }
    #[cfg(feature = "soak-observers")]
    let observer_finish = {
        let finish_elapsed_seconds = phase_zero
            .as_ref()
            .and_then(|phase_zero| {
                monotonic_now_ns()
                    .ok()?
                    .checked_sub(phase_zero.monotonic_ns)
            })
            .map(|elapsed| elapsed / 1_000_000_000)
            .unwrap_or(0);
        candidate_payloads.finish(finish_elapsed_seconds)
    };
    #[cfg(not(feature = "soak-observers"))]
    let observer_finish: Result<(), String> = Ok(());
    let mut errors = Vec::new();
    if let Err(error) = loop_result {
        errors.push(error);
    }
    if let Err(error) = summary_result {
        errors.push(format!("summary publication failed: {error}"));
    }
    if let Err(error) = observer_finish {
        errors.push(format!("candidate observer finalization failed: {error}"));
    }
    let barrier_ns = match barrier_result {
        Ok(value) => value,
        Err(error) => {
            errors.push(error);
            None
        }
    };
    let final_error = (!errors.is_empty()).then(|| errors.join("; "));
    if let Err(error) =
        final_barrier::publish(&output, completed, barrier_ns, final_error.as_deref())
    {
        errors.push(error);
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("; "))
    }
}

#[test]
#[ignore = "requires the external local WSS soak launcher and Engine A"]
fn t14_local_wss_soak() {
    let config = std::env::var_os(CONFIG_ENV)
        .map(PathBuf::from)
        .unwrap_or_else(|| panic!("{CONFIG_ENV} is required for the ignored live soak"));
    if !config.is_absolute() {
        panic!("{CONFIG_ENV} must be an absolute path");
    }
    let output = std::env::var_os(OUTPUT_ENV)
        .map(PathBuf::from)
        .unwrap_or_else(|| panic!("{OUTPUT_ENV} is required for the ignored live soak"));
    if !output.is_absolute() {
        panic!("{OUTPUT_ENV} must be an absolute path");
    }
    if let Err(error) = run_live(&config, &output) {
        panic!("T1.4 local WSS soak failed: {error}");
    }
}
