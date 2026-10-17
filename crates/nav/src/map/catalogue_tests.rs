use super::*;
use crate::map::identity::Digest;
use crate::map::producer::ProducerStep;
use crate::map::producer::{derive_catalogue, ClientMapInput};
use api::identity::{Cancellation, Deadline};
use api::work::{
    MonotonicClock, SharedCancellation, SliceBudget, StepContext, SystemClock, WorkPhase,
};
use client::io::cache_289::synthetic_jag;
use std::num::NonZeroU32;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

static NEXT: AtomicUsize = AtomicUsize::new(0);

fn qualify_game_data_initializer(revision_number: u16) {
    let revision = match revision_number {
        274 => api::selected::ClientRevision::R274,
        289 => api::selected::ClientRevision::R289,
        _ => panic!("unsupported qualification revision"),
    };
    let source = game_data_source_bounds(revision_number).unwrap();
    let peak_bound = u64::try_from(game_data_initializer_peak_bound(source).unwrap())
        .expect("game-data peak bound fits allocation counter");
    let retained_bound = i64::try_from(game_data_retained_owner_bound(source).unwrap())
        .expect("retained game-data bound fits allocation counter");
    let started = Instant::now();
    let allocations = allocation_counter::measure(|| {
        let data = api::game_data::for_revision(revision).unwrap();
        assert_eq!(data.revision(), i32::from(revision_number));
        drop(data);
    });
    let elapsed = started.elapsed();
    eprintln!(
        "game_data_release_memory revision={revision_number} cold_peak_bytes={} retained_bytes={} elapsed={elapsed:?} source_peak_bound_bytes={peak_bound} source_owner_bound_bytes={retained_bound}",
        allocations.bytes_max,
        allocations.bytes_current,
    );
    assert!(
        allocations.bytes_max > 0,
        "initializer was already warm; run in a fresh process"
    );
    assert!(
        allocations.bytes_current > 0,
        "initializer retained no owner; run in a fresh process"
    );
    assert!(
        allocations.bytes_max <= peak_bound,
        "cold initializer exceeded source bound"
    );
    assert!(
        allocations.bytes_current <= retained_bound,
        "retained owner exceeded source bound"
    );
}

#[test]
#[ignore = "cold release qualification; run each exact filter in a fresh process"]
fn game_data_initializer_r274_release_cold_memory_bound() {
    qualify_game_data_initializer(274);
}

#[test]
#[ignore = "cold release qualification; run each exact filter in a fresh process"]
fn game_data_initializer_r289_release_cold_memory_bound() {
    qualify_game_data_initializer(289);
}

#[test]
fn catalogue_charge_reserves_cold_initializer_for_both_revisions() {
    for revision in [274, 289] {
        let source = game_data_source_bounds(revision).unwrap();
        let owner_bound = game_data_retained_owner_bound(source).unwrap();
        let peak_bound = game_data_initializer_peak_bound(source).unwrap();
        let jag = PathBuf::from("/verified/jag");
        let snapshot = PathBuf::from("/verified/snapshot");
        let input = ClientMapInput::new(revision, Digest([0; 32]), &jag, &snapshot).unwrap();
        let charge = CatalogueCursor::work_charge(input).unwrap();
        assert!(charge.input_bytes() >= owner_bound);
        assert!(
            charge.input_bytes() + charge.scratch_bytes() >= peak_bound,
            "revision {revision} omitted the cold initializer peak"
        );
    }
}

/// Anvil marker (Key row 10), minigame marker (Jagex's `???` row 38), a
/// nameless Trade loc without a mapfunction, and a nameless marker for 49,
/// which lies past the Key legend.
const LOCS: [&[u8]; 4] = [
    &[60, 0, 10, 0],
    &[60, 0, 38, 0],
    &[30, b'T', b'r', b'a', b'd', b'e', b'\n', 0],
    &[60, 0, 49, 0],
];

struct Snapshot(PathBuf);
impl Snapshot {
    /// One mapsquare (50, 50) with flat land and loc `id` placed at local (0, id).
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!(
            "catalogue-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        // `LocType::unpack` starts reading loc.dat at byte 2, after the count.
        let mut dat = vec![0, LOCS.len() as u8];
        let mut idx = vec![0, LOCS.len() as u8];
        for loc in LOCS {
            dat.extend(loc);
            idx.extend((loc.len() as u16).to_be_bytes());
        }
        std::fs::write(
            dir.join("config"),
            synthetic_jag(&[("loc.dat", &dat), ("loc.idx", &idx)]),
        )
        .unwrap();
        let mut index = (50u16 << 8 | 50).to_be_bytes().to_vec();
        index.extend(0u16.to_be_bytes());
        index.extend(1u16.to_be_bytes());
        index.push(0);
        std::fs::write(
            dir.join("versionlist"),
            synthetic_jag(&[("map_index", &index)]),
        )
        .unwrap();
        // Every id delta is 1; each position delta (x<<6|z) + 1 is a one-byte smart.
        let mut locs = Vec::new();
        for id in 0..LOCS.len() as u8 {
            locs.extend([1, id + 1, 22 << 2, 0]);
        }
        locs.push(0);
        let land = vec![0; 4 * 64 * 64];
        let mut maps = Vec::new();
        for (file, payload) in [land, locs].into_iter().enumerate() {
            maps.extend((file as u32).to_le_bytes());
            maps.extend((payload.len() as u32).to_le_bytes());
            maps.extend(payload);
        }
        std::fs::write(dir.join("maps.bin"), maps).unwrap();
        Self(dir)
    }

    fn records(&self) -> Vec<PoiRecord> {
        let input = ClientMapInput::new(289, Digest([7; 32]), &self.0, &self.0).unwrap();
        let (pois, _) = derive_catalogue(input).unwrap();
        pois.records.as_slice().to_vec()
    }
}
impl Drop for Snapshot {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn nameless_mapfunction_loc_takes_its_key_legend_name() {
    let records = Snapshot::new().records();
    let named: Vec<_> = records
        .iter()
        .filter(|record| record.key.id < 2)
        .map(|record| (record.key.id, record.key.entity, record.name.as_str()))
        .collect();
    assert_eq!(
        named,
        [
            (0, EntityKind::MapFunction, "Anvil"),
            (1, EntityKind::MapFunction, "Minigames"),
        ]
    );
}

#[test]
fn nameless_loc_without_a_key_legend_name_is_not_a_poi() {
    let records = Snapshot::new().records();
    assert!(
        records.iter().all(|record| record.key.id < 2),
        "unlabelled places leaked into the catalogue: {records:?}"
    );
}

#[test]
fn quest_start_names_aggregate_nearby_targets_and_keep_far_markers_generic() {
    let catalog = QuestStartCatalog {
        entries: vec![
            QuestStartEntry {
                display: "Rune Mysteries Quest".to_string(),
                kind: QuestStartTargetKind::Npc,
                id: 11,
            },
            QuestStartEntry {
                display: "Cook's Assistant".to_string(),
                kind: QuestStartTargetKind::Npc,
                id: 10,
            },
            QuestStartEntry {
                display: "Arena Quest".to_string(),
                kind: QuestStartTargetKind::Loc,
                id: 20,
            },
        ],
        npc_placements: vec![
            QuestStartPlacement {
                id: 10,
                x: 100,
                z: 200,
                plane: 0,
            },
            QuestStartPlacement {
                id: 11,
                x: 101,
                z: 200,
                plane: 0,
            },
        ],
    };
    let wrong_plane_loc = [QuestStartPlacement {
        id: 20,
        x: 100,
        z: 200,
        plane: 1,
    }];
    let matching_loc = [QuestStartPlacement {
        id: 20,
        x: 100,
        z: 200,
        plane: 0,
    }];
    let without_loc = quest_start_name_at(&catalog, 0, 100.5, 200.5, &wrong_plane_loc).unwrap();
    assert_eq!(
        without_loc.as_str(),
        "Cook's Assistant, Rune Mysteries Quest"
    );
    let with_loc = quest_start_name_at(&catalog, 0, 100.5, 200.5, &matching_loc).unwrap();
    assert_eq!(
        with_loc.as_str(),
        "Arena Quest, Cook's Assistant, Rune Mysteries Quest"
    );
    assert!(quest_start_name_at(&catalog, 0, 500.5, 500.5, &matching_loc).is_none());
}

/// Rare-Trees ties retain source order and ignore other planes. Quest Start
/// targets retain same-plane filtering and lexical output order. Both marker
/// families share one map-square matching pass.
fn catalogue_matching_fixture(candidate_limit: usize) -> Vec<u8> {
    fn tree_loc(name: &str) -> Vec<u8> {
        let mut loc = vec![2];
        loc.extend(name.bytes());
        loc.push(10);
        loc.push(30);
        loc.extend("Chop down".bytes());
        loc.push(10);
        loc.push(0);
        loc
    }
    fn named_loc(name: &str) -> Vec<u8> {
        let mut loc = vec![2];
        loc.extend(name.bytes());
        loc.extend([10, 0]);
        loc
    }
    let yew = tree_loc("Yew tree");
    let magic = tree_loc("Magic tree");
    let maple = tree_loc("Maple tree");
    let willow = tree_loc("Willow");
    let target_a = named_loc("Target A");
    let target_b = named_loc("Target B");
    let rare_marker: &[u8] = &[60, 0, 34, 0];
    let quest_marker: &[u8] = &[60, 0, 6, 0];
    // ids 0-7 are the rare-tree fixture; targets 8/10 flank Quest Start
    // markers 9/11, with target 10 deliberately on another plane.
    let locs: Vec<&[u8]> = vec![
        &yew,
        &magic,
        rare_marker,
        rare_marker,
        &maple,
        rare_marker,
        rare_marker,
        &willow,
        &target_a,
        quest_marker,
        &target_b,
        quest_marker,
    ];
    // Yew and Magic tie around marker 2. Placement order makes Yew win.
    let tiles: [(u16, u16); 12] = [
        (0, 4),
        (0, 6),
        (0, 5),
        (0, 6),
        (0, 10),
        (0, 10),
        (0, 20),
        (1, 20),
        (0, 30),
        (0, 30),
        (1, 40),
        (0, 40),
    ];
    let dir = std::env::temp_dir().join(format!(
        "catalogue-rare-trees-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let mut dat = vec![0, locs.len() as u8];
    let mut idx = vec![0, locs.len() as u8];
    for loc in &locs {
        dat.extend(*loc);
        idx.extend((loc.len() as u16).to_be_bytes());
    }
    std::fs::write(
        dir.join("config"),
        synthetic_jag(&[("loc.dat", &dat), ("loc.idx", &idx)]),
    )
    .unwrap();
    let mut index = (50u16 << 8 | 50).to_be_bytes().to_vec();
    index.extend(0u16.to_be_bytes());
    index.extend(1u16.to_be_bytes());
    index.push(0);
    std::fs::write(
        dir.join("versionlist"),
        synthetic_jag(&[("map_index", &index)]),
    )
    .unwrap();
    let mut placements = Vec::new();
    for (plane, z) in tiles {
        // Position delta is packed (plane << 12 | x << 6 | z) + 1 as a smart.
        let delta = (plane << 12 | z) + 1;
        placements.push(1);
        if delta < 128 {
            placements.push(delta as u8);
        } else {
            placements.extend((delta + 32768).to_be_bytes());
        }
        placements.extend([22 << 2, 0]);
    }
    placements.push(0);
    let land = vec![0; 4 * 64 * 64];
    let mut maps = Vec::new();
    for (file, payload) in [land, placements].into_iter().enumerate() {
        maps.extend((file as u32).to_le_bytes());
        maps.extend((payload.len() as u32).to_le_bytes());
        maps.extend(payload);
    }
    std::fs::write(dir.join("maps.bin"), maps).unwrap();
    let input = ClientMapInput::new(289, Digest([7; 32]), &dir, &dir).unwrap();
    let mut cursor = CatalogueCursor::new(input).unwrap();
    cursor.retained_candidates = Some(Vec::with_capacity(candidate_limit));
    let clock = SystemClock;
    let cancellation = SharedCancellation::new();
    while cursor.progress().stage == CatalogueStage::Preparing {
        let mut context = test_context(&clock, &cancellation);
        assert!(matches!(
            cursor.step(&mut context).unwrap(),
            ProducerStep::Yielded
        ));
        assert_eq!(context.units_this_slice(), 1);
        assert!(!context.continued_after_yield());
    }
    assert_eq!(cursor.progress().stage, CatalogueStage::Indexing);
    cursor.quest_starts = Some(QuestStartCatalog {
        entries: vec![
            QuestStartEntry {
                display: "Zulu Quest".to_string(),
                kind: QuestStartTargetKind::Loc,
                id: 8,
            },
            QuestStartEntry {
                display: "Alpha Quest".to_string(),
                kind: QuestStartTargetKind::Loc,
                id: 8,
            },
            QuestStartEntry {
                display: "Another Plane Quest".to_string(),
                kind: QuestStartTargetKind::Loc,
                id: 10,
            },
        ],
        npc_placements: Vec::new(),
    });
    cursor.quest_loc_targets = vec![(8, 0), (8, 1), (10, 2)];
    let mut matching_squares_completed = 0;
    let mut saw_matching = false;
    let pois = loop {
        let before = cursor.progress().units_completed;
        let mut context = test_context(&clock, &cancellation);
        match cursor.step(&mut context).unwrap() {
            ProducerStep::Yielded => {
                assert_eq!(context.units_this_slice(), 1);
                assert!(!context.continued_after_yield());
                assert_eq!(cursor.progress().units_completed, before + 1);
                let progress = cursor.progress();
                if !saw_matching && progress.stage == CatalogueStage::Matching {
                    assert_eq!(cursor.retained_candidates.is_some(), candidate_limit >= 6);
                    if let Some(candidates) = &cursor.retained_candidates {
                        assert_eq!(candidates.len(), 6);
                        assert_eq!(candidates.capacity(), candidate_limit);
                    } else {
                        assert_eq!(cursor.retained_square_ends.capacity(), 0);
                    }
                    let cancelled = SharedCancellation::new();
                    cancelled.register(Cancellation::Operator).unwrap();
                    let mut stopped = test_context(&clock, &cancelled);
                    assert!(matches!(
                        cursor.step(&mut stopped).unwrap(),
                        ProducerStep::Cancelled
                    ));
                    assert_eq!(stopped.units_this_slice(), 0);
                    assert_eq!(cursor.progress().units_completed, progress.units_completed);
                }
                saw_matching |= progress.stage == CatalogueStage::Matching;
                if progress.stage == CatalogueStage::Matching {
                    assert_eq!(progress.map_squares_total, Some(1));
                    matching_squares_completed =
                        matching_squares_completed.max(progress.map_squares_completed);
                }
            }
            ProducerStep::Completed(result) => break result.pois,
            ProducerStep::Cancelled => panic!("unexpected catalogue cancellation"),
        }
    };
    assert_eq!(
        cursor
            .reader
            .as_ref()
            .expect("completed cursor retains its cache reader")
            .decoded_read_counts(),
        if candidate_limit >= 6 { (1, 1) } else { (2, 2) },
        "retention avoids the second scan; overflow preserves the fallback"
    );
    assert!(saw_matching, "cursor skipped marker matching");
    assert_eq!(matching_squares_completed, 1);
    let _ = std::fs::remove_dir_all(&dir);
    let records = pois.records.as_slice();
    assert_eq!(records.len(), 6, "{records:?}");
    let mut names: Vec<(u32, &str)> = records
        .iter()
        .map(|record| (record.key.id, record.name.as_str()))
        .collect();
    names.sort_unstable();
    assert_eq!(
        names,
        vec![
            (2, "Yew tree"),
            (3, "Magic tree"),
            (5, "Maple tree"),
            (6, "Rare Trees"),
            (9, "Alpha Quest, Zulu Quest"),
            (11, "Quest Start"),
        ],
        "{records:?}"
    );
    let entry = cursor.reader.as_ref().unwrap().entries[0];
    let west = f64::from(entry.square_x) * f64::from(MAP_SQUARE_SIZE);
    let south = f64::from(entry.square_z) * f64::from(MAP_SQUARE_SIZE);
    for (kind, centre_extent) in [
        (MapMarkerKind::RareTrees, f64::from(u8::MAX) / 2.0),
        (MapMarkerKind::QuestStart, 0.5),
    ] {
        let mut marker = records[0].clone();
        marker.display.x = west + f64::from(MAP_SQUARE_SIZE) - 1.0 + centre_extent + 8.0;
        marker.display.z = south + 0.5;
        assert!(
            marker_may_match_square(&marker, kind, entry),
            "maximum footprint and exact matching-radius boundary remain eligible"
        );
        marker.display.x += 0.5;
        assert!(!marker_may_match_square(&marker, kind, entry));
        marker.display.x = west + 0.5;
        marker.display.z = south - 8.0;
        assert!(marker_may_match_square(&marker, kind, entry));
        marker.display.z -= 0.5;
        assert!(!marker_may_match_square(&marker, kind, entry));
    }
    let unknown = pois
        .coverage
        .unresolved
        .as_slice()
        .iter()
        .find(|issue| {
            matches!(
                issue.reason,
                crate::map::formats::CoverageReason::UnknownMapFunction
            )
        })
        .map(|issue| issue.count);
    assert_eq!(
        unknown,
        Some(2),
        "{:?}",
        pois.coverage.unresolved.as_slice()
    );
    pois.encode().unwrap()
}

#[test]
fn catalogue_matches_at_retention_limit_and_falls_back_on_overflow() {
    let retained = catalogue_matching_fixture(6);
    let overflowed = catalogue_matching_fixture(5);
    let no_retention = catalogue_matching_fixture(0);
    assert_eq!(
        retained, overflowed,
        "overflow must not change catalogue bytes"
    );
    assert_eq!(
        retained, no_retention,
        "the full second pass remains equivalent"
    );
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CursorTestPhase;

impl WorkPhase for CursorTestPhase {
    fn index(self) -> u16 {
        0
    }
}

fn test_context<'a>(
    clock: &'a SystemClock,
    cancellation: &'a SharedCancellation,
) -> StepContext<'a, CursorTestPhase> {
    let budget = SliceBudget::new(
        NonZeroU32::new(1).unwrap(),
        Deadline::after(clock.now(), Duration::from_secs(60)),
    );
    StepContext::new(clock, budget, cancellation)
}

fn cursor_with_placement_batch(snapshot: &Snapshot) -> CatalogueCursor {
    let input = ClientMapInput::new(289, Digest([7; 32]), &snapshot.0, &snapshot.0).unwrap();
    let mut cursor = CatalogueCursor::new(input).unwrap();
    let clock = SystemClock;
    let cancellation = SharedCancellation::new();
    for _ in 0..1_000 {
        if matches!(cursor.phase, CataloguePhase::Placement) {
            let placement = cursor.placements[0];
            cursor.placements = vec![placement; 2 * PLACEMENT_OPERATIONS_PER_UNIT as usize];
            return cursor;
        }
        let mut context = test_context(&clock, &cancellation);
        assert!(matches!(
            cursor.step(&mut context).unwrap(),
            ProducerStep::Yielded
        ));
    }
    panic!("catalogue never reached placement processing");
}

#[test]
fn catalogue_placement_unit_groups_at_most_64_operations() {
    let snapshot = Snapshot::new();
    let mut cursor = cursor_with_placement_batch(&snapshot);
    let clock = SystemClock;
    let cancellation = SharedCancellation::new();
    let mut context = test_context(&clock, &cancellation);
    assert!(matches!(
        cursor.step(&mut context).unwrap(),
        ProducerStep::Yielded
    ));
    assert_eq!(context.units_this_slice(), 1);
    assert!(!context.continued_after_yield());
    assert_eq!(
        cursor.placement_index,
        PLACEMENT_OPERATIONS_PER_UNIT as usize
    );
    assert_eq!(
        cursor.stats.loc_placements,
        u64::from(PLACEMENT_OPERATIONS_PER_UNIT)
    );
}

#[test]
fn catalogue_placement_batch_checks_the_deadline_between_operations() {
    struct ExpiringClock {
        started: Instant,
        checks: AtomicUsize,
    }
    impl MonotonicClock for ExpiringClock {
        fn now(&self) -> Instant {
            if self.checks.fetch_add(1, Ordering::Relaxed) < 3 {
                self.started
            } else {
                self.started + Duration::from_secs(2)
            }
        }
    }
    let snapshot = Snapshot::new();
    let mut cursor = cursor_with_placement_batch(&snapshot);
    let clock = ExpiringClock {
        started: Instant::now(),
        checks: AtomicUsize::new(0),
    };
    let cancellation = SharedCancellation::new();
    let budget = SliceBudget::new(
        NonZeroU32::new(1).unwrap(),
        Deadline::after(clock.started, Duration::from_secs(1)),
    );
    let mut context = StepContext::<CursorTestPhase>::new(&clock, budget, &cancellation);
    assert!(matches!(
        cursor.step(&mut context).unwrap(),
        ProducerStep::Yielded
    ));
    assert_eq!(cursor.placement_index, 3);
    assert_eq!(context.units_this_slice(), 0);
    assert!(!context.continued_after_yield());
}

#[test]
fn catalogue_cursor_yields_after_one_unit_and_matches_sync_bytes() {
    let snapshot = Snapshot::new();
    let input = ClientMapInput::new(289, Digest([7; 32]), &snapshot.0, &snapshot.0).unwrap();
    let expected = derive_catalogue(input).unwrap().0.encode().unwrap();
    let mut cursor = CatalogueCursor::new(input).unwrap();
    let clock = SystemClock;
    let cancellation = SharedCancellation::new();
    let mut slices = 0;

    loop {
        let before = cursor.progress().units_completed;
        let mut context = test_context(&clock, &cancellation);
        match cursor.step(&mut context).unwrap() {
            ProducerStep::Yielded => {
                assert_eq!(context.units_this_slice(), 1);
                assert!(!context.continued_after_yield());
                assert_eq!(cursor.progress().units_completed, before + 1);
                slices += 1;
                assert!(slices < 10_000, "catalogue cursor did not finish");
            }
            ProducerStep::Completed(result) => {
                assert_eq!(result.pois.encode().unwrap(), expected);
                assert_eq!(result.stats.map_squares, 1);
                break;
            }
            ProducerStep::Cancelled => panic!("unexpected catalogue cancellation"),
        }
    }
}

#[test]
fn catalogue_cursor_observes_cancellation_in_every_reported_stage() {
    for target in [
        CatalogueStage::Preparing,
        CatalogueStage::Indexing,
        CatalogueStage::Scanning,
        CatalogueStage::Matching,
        CatalogueStage::Finalizing,
    ] {
        let snapshot = Snapshot::new();
        let input = ClientMapInput::new(289, Digest([7; 32]), &snapshot.0, &snapshot.0).unwrap();
        let mut cursor = CatalogueCursor::new(input).unwrap();
        let clock = SystemClock;
        let cancellation = SharedCancellation::new();
        let mut reached = false;

        for _ in 0..10_000 {
            if cursor.progress().stage == target {
                cancellation.register(Cancellation::Operator).unwrap();
                let mut context = test_context(&clock, &cancellation);
                assert!(matches!(
                    cursor.step(&mut context),
                    Ok(ProducerStep::Cancelled)
                ));
                reached = true;
                break;
            }
            let mut context = test_context(&clock, &cancellation);
            match cursor.step(&mut context).unwrap() {
                ProducerStep::Yielded => {
                    assert_eq!(context.units_this_slice(), 1);
                    assert!(!context.continued_after_yield());
                }
                ProducerStep::Completed(_) => panic!("catalogue completed before {target:?}"),
                ProducerStep::Cancelled => panic!("unexpected catalogue cancellation"),
            }
        }
        assert!(reached, "catalogue cursor never reached {target:?}");
    }
}

#[test]
fn catalogue_cursor_reopens_maps_after_each_index_yield() {
    let snapshot = Snapshot::new();
    let input = ClientMapInput::new(289, Digest([7; 32]), &snapshot.0, &snapshot.0).unwrap();
    let mut cursor = CatalogueCursor::new(input).unwrap();
    let clock = SystemClock;
    let cancellation = SharedCancellation::new();
    let mut index_units = 0;

    while index_units < 5 {
        let mut context = test_context(&clock, &cancellation);
        match cursor.step(&mut context).unwrap() {
            ProducerStep::Yielded => {
                if cursor.progress().stage == CatalogueStage::Indexing {
                    index_units += context.units_this_slice();
                }
            }
            ProducerStep::Completed(_) => panic!("catalogue completed before cache indexing"),
            ProducerStep::Cancelled => panic!("unexpected catalogue cancellation"),
        }
    }

    std::fs::remove_file(snapshot.0.join("maps.bin")).unwrap();
    let mut observed_missing_path = false;
    for _ in 0..10 {
        let mut context = test_context(&clock, &cancellation);
        match cursor.step(&mut context) {
            Err(_) => {
                observed_missing_path = true;
                break;
            }
            Ok(ProducerStep::Yielded) => {}
            Ok(ProducerStep::Completed(_)) => {
                panic!("cursor continued using maps.bin after it was removed")
            }
            Ok(ProducerStep::Cancelled) => panic!("unexpected catalogue cancellation"),
        }
    }
    assert!(observed_missing_path, "indexing did not reopen maps.bin");
}

#[test]
#[ignore = "real-content candidate census; requires prepared 274/289 cache inputs"]
fn catalogue_real_candidate_census() {
    for revision in [274, 289] {
        let jag =
            PathBuf::from(std::env::var(format!("BOT_MAP_QUALIFY_R{revision}_JAG_DIR")).unwrap());
        let snapshot = PathBuf::from(
            std::env::var(format!("BOT_MAP_QUALIFY_R{revision}_SNAPSHOT_DIR")).unwrap(),
        );
        let selected = match revision {
            274 => ClientRevision::R274,
            289 => ClientRevision::R289,
            _ => unreachable!(),
        };
        let data = api::game_data::for_revision(selected).unwrap();
        let content = Digest::from_hex(data.content_id().unwrap()).unwrap();
        let input = ClientMapInput::new(revision, content, &jag, &snapshot).unwrap();
        let mut cursor = CatalogueCursor::new(input).unwrap();
        while !matches!(cursor.phase, CataloguePhase::ReaderInit) {
            cursor.advance().unwrap();
        }
        let quest_ids: HashSet<_> = cursor
            .quest_starts
            .as_ref()
            .unwrap()
            .entries
            .iter()
            .filter(|quest| quest.kind == QuestStartTargetKind::Loc)
            .map(|quest| quest.id)
            .collect();
        let mut reader_cursor = CacheReaderCursor::new(input);
        let mut reader = loop {
            if let Some(reader) = reader_cursor.advance().unwrap() {
                break reader;
            }
        };
        let mut trees = 0usize;
        let mut quests = 0usize;
        let mut candidates = 0usize;
        let mut placements = 0usize;
        for index in 0..reader.entries.len() {
            let entry = reader.entries[index];
            for placement in reader.read_locs(entry).unwrap() {
                let tree = is_woodcut_tree(&cursor.locs[placement.id as usize]);
                let quest = quest_ids.contains(&(placement.id as i32));
                trees += usize::from(tree);
                quests += usize::from(quest);
                candidates += usize::from(tree || quest);
                placements += 1;
            }
        }
        eprintln!(
            "catalogue_candidates revision={revision} squares={} placements={placements} trees={trees} quests={quests} candidates={candidates}",
            reader.entries.len()
        );
    }
}

#[test]
fn catalogue_candidate_storage_is_fixed_and_fully_charged() {
    let snapshot = Snapshot::new();
    let input = ClientMapInput::new(289, Digest([7; 32]), &snapshot.0, &snapshot.0).unwrap();
    let charge = CatalogueCursor::work_charge(input).unwrap();
    let retained_bytes = MAX_RETAINED_CANDIDATES * std::mem::size_of::<RetainedCandidate>()
        + MAX_MAP_INDEX_ROWS * std::mem::size_of::<usize>();
    assert!(charge.input_bytes() >= retained_bytes + std::mem::size_of::<CatalogueCursor>());
    let mut cursor = CatalogueCursor::new(input).unwrap();
    cursor.locs.push(LocDefinition {
        name: "Tree".to_string(),
        operations: [Some("Chop down".to_string()), None, None, None, None],
        width: 1,
        length: 1,
        active: true,
        mapfunction: None,
    });
    let placement = LocPlacement {
        id: 0,
        x: 0,
        z: 0,
        plane: 0,
        shape: 10,
        rotation: 0,
    };
    let capacity = cursor.retained_candidates.as_ref().unwrap().capacity();
    assert_eq!(capacity, MAX_RETAINED_CANDIDATES);
    for _ in 0..capacity {
        cursor.retain_candidate(placement, true).unwrap();
    }
    assert_eq!(cursor.retained_candidates.as_ref().unwrap().len(), capacity);
    assert_eq!(
        cursor.retained_candidates.as_ref().unwrap().capacity(),
        capacity
    );
    cursor.retain_candidate(placement, true).unwrap();
    assert!(cursor.retained_candidates.is_none());
    assert_eq!(cursor.retained_square_ends.capacity(), 0);
    cursor.retain_candidate(placement, true).unwrap();
    assert!(
        cursor.retained_candidates.is_none(),
        "overflow never restarts retention"
    );
}
