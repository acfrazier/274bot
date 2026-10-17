use api::snapshot::WorldTile;

use crate::router::AvoidRect;
use crate::transport::{WildernessRules, WildernessZone};

use super::{Zone, ZoneClass, ZoneExempt, ZoneFilter, ZoneGroup, ZoneKey, ZoneKind, ZoneTable};

fn tile(x: i32, z: i32, level: i32) -> WorldTile {
    WorldTile { x, z, level }
}

fn table() -> ZoneTable {
    let mut grouped = Zone::npc(tile(5, 5, 0), 2, ZoneClass::LevelRule, 10, 0);
    grouped.group = 0;
    let overlapping = Zone::npc(tile(7, 5, 0), 2, ZoneClass::LevelRule, 10, 0);
    let hazard = Zone::hazard(
        AvoidRect {
            min_x: 10,
            max_x: 10,
            min_z: 10,
            max_z: 10,
            level: None,
        },
        0,
        1,
    );
    ZoneTable::from_parts(
        vec![grouped, overlapping, hazard],
        vec![
            ZoneKind::new("ranger", "Ranger", 123, 5, true, false),
            ZoneKind::new("bog", "Bog", -1, 0, false, false),
        ],
        vec![ZoneGroup::new(
            "ranger-pack",
            "Ranger pack",
            AvoidRect {
                min_x: 5,
                max_x: 5,
                min_z: 5,
                max_z: 5,
                level: Some(0),
            },
            vec![0].into_boxed_slice(),
        )],
        Vec::new(),
        Vec::new(),
        tile(0, 0, 0),
        12,
        12,
        &WildernessRules::default(),
    )
    .unwrap()
}

#[test]
fn blocking_detail_item_length_matches_formatter_without_allocating() {
    let table = table();
    for key in [
        ZoneKey::Group(0),
        ZoneKey::Zone(0),
        ZoneKey::Zone(1),
        ZoneKey::Zone(2),
        ZoneKey::Group(u16::MAX),
        ZoneKey::Zone(u16::MAX),
    ] {
        let mut text = String::new();
        table.write_blocking_detail_item(key, &mut text).unwrap();
        let mut bytes = 0;
        let allocation = allocation_counter::measure(|| {
            bytes = table.blocking_detail_item_bytes(key);
        });
        assert_eq!(allocation.count_total, 0);
        assert_eq!(bytes, text.len(), "{key:?}");
    }
}

#[test]
fn blocked_detail_writer_matches_legacy_format_with_cached_bound() {
    let table = table();
    let keys = [table.key(0), table.key(1), table.key(2)];
    let expected = format!(
        "blocked by danger zones: {}",
        keys.iter()
            .map(|&key| format!("{} [{}]", table.label(key), table.name(key)))
            .collect::<Vec<_>>()
            .join(", ")
    );
    let bound = table.blocking_detail_bytes_bound();
    assert_eq!(bound, expected.len());
    let mut actual = String::with_capacity(bound);
    table.write_blocking_detail_prefix(&mut actual).unwrap();
    for (index, key) in keys.into_iter().enumerate() {
        if index != 0 {
            actual.push_str(", ");
        }
        table.write_blocking_detail_item(key, &mut actual).unwrap();
    }
    assert_eq!(actual, expected);
    for key in keys {
        let item = format!("{} [{}]", table.label(key), table.name(key));
        assert_eq!(table.blocking_detail_item_bytes(key), item.len());
    }
}

#[test]
fn level_rules_and_always_zones_share_the_expected_activation_predicate() {
    let table = table();
    let wilderness = WildernessRules::default();
    let normal = ZoneFilter::new(&table, Some(10), &[], &ZoneExempt::NONE);
    assert!(normal
        .blocking_at(&wilderness, tile(5, 5, 0))
        .next()
        .is_some());

    let above_cap = ZoneFilter::new(&table, Some(11), &[], &ZoneExempt::NONE);
    assert!(above_cap
        .blocking_at(&wilderness, tile(5, 5, 0))
        .next()
        .is_none());
    assert!(above_cap
        .blocking_at(&wilderness, tile(10, 10, 0))
        .next()
        .is_some());

    let wilderness = WildernessRules {
        zones: vec![WildernessZone {
            x1: 5,
            z1: 5,
            x2: 5,
            z2: 5,
            level1: 0,
            level2: 0,
            origin_z: 5,
        }],
        divisor: 0,
        offset: 0,
    };
    assert!(above_cap
        .blocking_at(&wilderness, tile(5, 5, 0))
        .next()
        .is_some());

    let missing_level = ZoneFilter::new(&table, None, &[], &ZoneExempt::NONE);
    assert!(missing_level
        .blocking_at(&WildernessRules::default(), tile(5, 5, 0))
        .next()
        .is_some());
}

#[test]
fn spawn_resolution_requires_the_exact_canonical_identity() {
    let table = table();
    assert_eq!(table.resolve("ranger@7,5,0"), Some(ZoneKey::Zone(1)));
    for name in [
        "ranger",
        "ranger@+7,5,0",
        "ranger@07,5,0",
        "ranger@7,5,-0",
        "ranger@7,5,00",
        "ranger@7,5,0,1",
        "ranger@7,5",
        "ranger@7,5,1",
        "ranger@2147483648,5,0",
        "other@7,5,0",
        "bog@10,10,0",
    ] {
        assert_eq!(table.resolve(name), None, "{name}");
    }
}

#[test]
fn named_exemptions_are_whole_walk_but_endpoint_exemptions_are_scoped() {
    let table = table();
    assert_eq!(table.resolve("ranger-pack"), Some(ZoneKey::Group(0)));
    assert_eq!(table.resolve("ranger@5,5,0"), Some(ZoneKey::Group(0)));
    assert_eq!(table.key(0), ZoneKey::Group(0));
    assert_eq!(table.name(ZoneKey::Group(0)), "ranger-pack");
    assert_eq!(table.label(ZoneKey::Group(0)), "Ranger pack");
    assert_eq!(table.resolve("bog"), Some(ZoneKey::Zone(2)));
    assert_eq!(table.name(ZoneKey::Zone(2)), "bog");
    assert_eq!(table.label(ZoneKey::Zone(2)), "Bog");

    let wilderness = WildernessRules::default();
    let group = ZoneExempt::named(&[ZoneKey::Group(0)]).unwrap();
    let by_group = ZoneFilter::new(&table, Some(10), &[], &group);
    assert!(by_group
        .blocking_at(&wilderness, tile(4, 5, 0))
        .next()
        .is_none());
    assert_eq!(
        by_group
            .blocking_at(&wilderness, tile(5, 5, 0))
            .collect::<Vec<_>>(),
        vec![1]
    );

    let endpoint = ZoneFilter::new(&table, Some(10), &[tile(5, 5, 0)], &ZoneExempt::NONE);
    assert!(endpoint
        .blocking_at(&wilderness, tile(5, 5, 0))
        .next()
        .is_some());
    assert!(
        endpoint
            .blocking_transition_at(&wilderness, tile(5, 5, 0), tile(6, 5, 0), &|| false)
            .next()
            .is_none(),
        "the source may leave and continue inside its active zones"
    );
    assert_eq!(
        endpoint
            .blocking_transition_at(&wilderness, tile(4, 5, 0), tile(5, 5, 0), &|| false)
            .collect::<Vec<_>>(),
        vec![1],
        "entering an overlapping non-origin zone stays blocked"
    );

    let destination = ZoneFilter::new(
        &table,
        Some(10),
        &[tile(0, 5, 0), tile(5, 5, 0)],
        &ZoneExempt::NONE,
    );
    assert!(
        destination
            .blocking_transition_at(&wilderness, tile(4, 5, 0), tile(5, 5, 0), &|| false)
            .next()
            .is_none(),
        "the selected destination permits movement within its active zones"
    );
    assert_eq!(
        destination
            .blocking_transition_at(&wilderness, tile(5, 5, 0), tile(4, 5, 0), &|| false)
            .collect::<Vec<_>>(),
        vec![1],
        "leaving a destination zone cannot turn it into transit"
    );
}

#[test]
fn transition_queries_goals_only_for_active_unexempted_entries() {
    let table = table();
    let wilderness = WildernessRules::default();
    for (combat, exemptions, to, queries) in [
        (10, ZoneExempt::NONE, tile(0, 0, 0), 0),
        (11, ZoneExempt::NONE, tile(5, 5, 0), 0),
        (10, ZoneExempt::all(), tile(5, 5, 0), 0),
        (10, ZoneExempt::NONE, tile(5, 5, 0), 1),
    ] {
        let filter = ZoneFilter::new(&table, Some(combat), &[tile(0, 0, 0)], &exemptions);
        let called = std::cell::Cell::new(0);
        let blocked: Vec<_> = filter
            .blocking_transition_at(&wilderness, tile(0, 0, 0), to, &|| {
                called.set(called.get() + 1);
                true
            })
            .collect();
        assert!(blocked.is_empty(), "{to:?}");
        assert_eq!(called.get(), queries, "{to:?}");
    }

    let filter = ZoneFilter::new(&table, Some(10), &[tile(0, 0, 0)], &ZoneExempt::NONE);
    let called = std::cell::Cell::new(0);
    let blocked: Vec<_> = filter
        .blocking_transition_at(&wilderness, tile(0, 0, 0), tile(5, 5, 0), &|| {
            called.set(called.get() + 1);
            false
        })
        .collect();
    assert_eq!(blocked, vec![0, 1]);
    assert_eq!(
        called.get(),
        1,
        "overlapping zones share the same goal query"
    );
}

#[test]
fn exemption_union_deduplicates_and_all_bypasses_mask_allocation() {
    let table = table();
    let first = ZoneExempt::named(&[ZoneKey::Group(0), ZoneKey::Zone(2)]).unwrap();
    let second = ZoneExempt::named(&[ZoneKey::Group(0), ZoneKey::Zone(1)]).unwrap();
    let combined = first.union(second).unwrap();
    let filter = ZoneFilter::new(&table, Some(10), &[], &combined);
    for tile in [tile(5, 5, 0), tile(7, 5, 0), tile(10, 10, 0)] {
        assert!(filter
            .blocking_at(&WildernessRules::default(), tile)
            .next()
            .is_none());
    }
    let duplicates = ZoneExempt::named(&[ZoneKey::Zone(0); 8]).unwrap();
    let extra = ZoneExempt::named(&[ZoneKey::Zone(1)]).unwrap();
    let deduplicated = duplicates.union(extra).unwrap();
    let deduplicated_filter = ZoneFilter::new(&table, Some(10), &[], &deduplicated);
    assert!(deduplicated_filter
        .blocking_at(&WildernessRules::default(), tile(7, 5, 0))
        .next()
        .is_none());
    let full = ZoneExempt::named(&[
        ZoneKey::Zone(0),
        ZoneKey::Zone(1),
        ZoneKey::Zone(2),
        ZoneKey::Zone(3),
        ZoneKey::Zone(4),
        ZoneKey::Zone(5),
        ZoneKey::Zone(6),
        ZoneKey::Zone(7),
    ])
    .unwrap();
    assert!(full
        .union(ZoneExempt::named(&[ZoneKey::Zone(8)]).unwrap())
        .is_err());
    assert!(ZoneExempt::named(&[ZoneKey::Zone(0x8000)]).is_err());

    let all = ZoneFilter::new(&table, Some(10), &[], &ZoneExempt::all());
    assert_eq!(ZoneExempt::default(), ZoneExempt::NONE);
    assert!(ZoneExempt::all().is_all());
    assert_eq!(all.mask_words(), 0);
    assert!(all
        .blocking_at(&WildernessRules::default(), tile(10, 10, 0))
        .next()
        .is_none());
    assert!(ZoneExempt::named(&[ZoneKey::Zone(0); 9]).is_err());
}

#[test]
fn constructors_do_not_wrap_invalid_world_levels_into_valid_zone_levels() {
    let kind = ZoneKind::new("ranger", "Ranger", 123, 5, true, false);
    for zone in [
        Zone::npc(tile(5, 5, 256), 1, ZoneClass::Always, u16::MAX, 0),
        Zone::shaped_npc(tile(5, 5, -256), 1, 1, ZoneClass::Always, u16::MAX, 0, 0),
    ] {
        let shapes = if zone.shape == super::NO_SHAPE {
            Vec::new()
        } else {
            vec![1]
        };
        assert!(ZoneTable::from_parts(
            vec![zone],
            vec![kind.clone()],
            Vec::new(),
            Vec::new(),
            shapes,
            tile(0, 0, 0),
            12,
            12,
            &WildernessRules::default(),
        )
        .is_err());
    }
}

#[test]
fn table_rejects_npc_rows_that_cannot_be_encoded_canonically() {
    let kind = ZoneKind::new("ranger", "Ranger", 123, 5, true, false);
    let rejects = |zone, shapes| {
        ZoneTable::from_parts(
            vec![zone],
            vec![kind.clone()],
            Vec::new(),
            Vec::new(),
            shapes,
            tile(0, 0, 0),
            16,
            16,
            &WildernessRules::default(),
        )
        .is_err()
    };

    assert!(rejects(
        Zone::npc(tile(5, 5, 0), 1, ZoneClass::LevelRule, 9, 0),
        Vec::new()
    ));
    let mut asymmetric = Zone::npc(tile(5, 5, 0), 1, ZoneClass::Always, u16::MAX, 0);
    asymmetric.max_x += 1;
    assert!(rejects(asymmetric, Vec::new()));
    assert!(rejects(
        Zone::shaped_npc(tile(5, 5, 0), 7, 1, ZoneClass::Always, u16::MAX, 0, 0),
        vec![(1u64 << 27) - 1]
    ));
}

#[test]
fn hunter_reach_overhanging_the_map_indexes_only_its_in_grid_cells() {
    let zones = vec![
        Zone::npc(tile(10, 20, 0), 4, ZoneClass::Always, u16::MAX, 0),
        Zone::npc(tile(25, 35, 0), 4, ZoneClass::Always, u16::MAX, 0),
        Zone::npc(tile(0, 0, 0), 1, ZoneClass::Always, u16::MAX, 0),
    ];
    let table = ZoneTable::from_parts(
        zones,
        vec![ZoneKind::new("edge", "Edge hunter", 1, 1, false, false)],
        Vec::new(),
        Vec::new(),
        Vec::new(),
        tile(10, 20, 0),
        16,
        16,
        &WildernessRules::default(),
    )
    .unwrap();
    let wilderness = WildernessRules::default();
    let filter = ZoneFilter::new(&table, None, &[], &ZoneExempt::NONE);
    assert!(filter
        .blocking_at(&wilderness, tile(10, 20, 0))
        .next()
        .is_some());
    assert!(filter
        .blocking_at(&wilderness, tile(14, 24, 0))
        .next()
        .is_some());
    assert!(filter
        .blocking_at(&wilderness, tile(21, 31, 0))
        .next()
        .is_some());
    assert!(filter
        .blocking_at(&wilderness, tile(25, 35, 0))
        .next()
        .is_some());
    for cell in [
        tile(15, 24, 0),
        tile(20, 31, 0),
        tile(10, 35, 0),
        tile(9, 20, 0),
        tile(26, 35, 0),
        tile(10, 20, 1),
    ] {
        assert!(
            filter.blocking_at(&wilderness, cell).next().is_none(),
            "{cell:?}"
        );
    }
}

struct PackMembershipUnitBound {
    revision: u16,
    nav_sha256: &'static str,
    bucket_candidates: usize,
    carves_per_candidate: usize,
    carves_per_bucket: usize,
    executed_rectangle_tests: usize,
    partition_comparisons: usize,
    wilderness_zones: usize,
}

// A whole negative contains call is one accounted route unit. Qualification
// observed <19 us maximum for each pack; these content envelopes, not a timing
// assertion affected by host scheduling, fail if a supported pack grows.
const PACK_274_MEMBERSHIP_UNIT: PackMembershipUnitBound = PackMembershipUnitBound {
    revision: 274,
    nav_sha256: "357ab9027003f9da254b2d0cb1a5f86b4b0e94b73465529460707a767b004fe0",
    bucket_candidates: 32,
    carves_per_candidate: 141,
    carves_per_bucket: 1_334,
    executed_rectangle_tests: 1_290,
    partition_comparisons: 527,
    wilderness_zones: 2,
};
const PACK_289_MEMBERSHIP_UNIT: PackMembershipUnitBound = PackMembershipUnitBound {
    revision: 289,
    nav_sha256: "4270f02711447267e0ee639579d41532996a3d37f712d5d63cdb79051e45300f",
    bucket_candidates: 32,
    carves_per_candidate: 141,
    carves_per_bucket: 1_749,
    executed_rectangle_tests: 1_729,
    partition_comparisons: 527,
    wilderness_zones: 2,
};

/// Count the operations actually reached by the native predicate. Every probe
/// below is checked against that predicate, including shape and carve exits.
fn membership_probe(table: &ZoneTable, index: u16, cell: WorldTile) -> (bool, usize, usize) {
    let zone = &table.zones[usize::from(index)];
    if i32::from(zone.level) != cell.level
        || cell.x < zone.min_x
        || cell.x > zone.max_x
        || cell.z < zone.min_z
        || cell.z > zone.max_z
    {
        return (false, 0, 0);
    }
    if zone.shape != super::NO_SHAPE {
        let width = (i64::from(zone.max_x) - i64::from(zone.min_x) + 1) as u32;
        let bit = (cell.z - zone.min_z) as u32 * width + (cell.x - zone.min_x) as u32;
        if table.shapes[usize::from(zone.shape)] & (1u64 << bit) == 0 {
            return (false, 0, 0);
        }
    }
    let mut partition_comparisons = 0;
    let start = table.carves.partition_point(|(candidate, _)| {
        partition_comparisons += 1;
        *candidate < index
    });
    let mut rectangle_tests = 0;
    let carved = table.carves[start..]
        .iter()
        .take_while(|(candidate, _)| *candidate == index)
        .any(|(_, rect)| {
            rectangle_tests += 1;
            rect.contains(cell)
        });
    (!carved, rectangle_tests, partition_comparisons)
}

fn membership_call_timings(table: &ZoneTable, cell: WorldTile) -> serde_json::Value {
    let mut nanoseconds = Vec::with_capacity(4096);
    for _ in 0..4096 {
        let started = std::time::Instant::now();
        // An absent key forces the same full native iterator scan that a
        // negative ZoneView::contains performs; nothing is expanded or copied.
        let contained = std::hint::black_box(table)
            .at(std::hint::black_box(cell))
            .any(|index| index == std::hint::black_box(u16::MAX));
        std::hint::black_box(contained);
        nanoseconds.push(started.elapsed().as_nanos());
    }
    nanoseconds.sort_unstable();
    serde_json::json!({
        "tile": [cell.x, cell.z, cell.level],
        "samples": nanoseconds.len(),
        "p50_ns": nanoseconds[nanoseconds.len() / 2],
        "p99_ns": nanoseconds[nanoseconds.len() * 99 / 100],
        "max_ns": nanoseconds[nanoseconds.len() - 1],
    })
}

/// Qualify the opaque membership units against both prepared content packs.
/// Missing inputs fail rather than silently omitting either supported pack.
#[test]
#[ignore = "requires explicit matching T5W_MEMBERSHIP_NAVPACK_274 and _289 inputs"]
fn packed_membership_work_bound() {
    for bound in [PACK_274_MEMBERSHIP_UNIT, PACK_289_MEMBERSHIP_UNIT] {
        let revision = bound.revision;
        let variable = format!("T5W_MEMBERSHIP_NAVPACK_{revision}");
        let path = std::path::PathBuf::from(std::env::var_os(&variable).expect(&variable));
        let manifest: crate::manifest::NavManifest = serde_json::from_slice(
            &std::fs::read(crate::manifest::nav_manifest_path(&path)).unwrap(),
        )
        .unwrap();
        assert_eq!(manifest.revision, revision);
        assert_eq!(manifest.nav_sha256, bound.nav_sha256);
        let world = crate::world::NavWorld::load_pack(&path).unwrap();
        let table = world.graph.zones.as_ref().expect("content zone table");
        let mut carves_per_zone = vec![0usize; table.zones.len()];
        for &(zone, _) in &table.carves {
            carves_per_zone[usize::from(zone)] += 1;
        }
        let max_carves_per_candidate = carves_per_zone.iter().copied().max().unwrap_or(0);
        let mut max_candidates_per_tile = 0;
        let mut max_carves_per_bucket = 0;
        let mut worst_bucket = 0;
        let bucket_cols = table.cols.div_ceil(super::BUCKET_SIDE as u32);
        let bucket_rows = table.rows.div_ceil(super::BUCKET_SIDE as u32);
        let per_level = bucket_cols * bucket_rows;
        let mut max_executed_rectangles = 0;
        let mut max_partition_comparisons = 0;
        let mut worst_rectangle_tile = table.origin;
        let mut worst_partition_tile = table.origin;
        let mut max_candidate_tile = table.origin;
        let mut visited_tiles = 0usize;
        for (position, &(key, start)) in table.buckets.iter().enumerate() {
            let start = start as usize;
            let end = table
                .buckets
                .get(position + 1)
                .map_or(table.entries.len(), |&(_, end)| end as usize);
            let candidates = &table.entries[start..end];
            let bx = key % bucket_cols;
            let bz = key % per_level / bucket_cols;
            let level = (key / per_level) as i32;
            for dz in 0..super::BUCKET_SIDE {
                for dx in 0..super::BUCKET_SIDE {
                    let x = bx * super::BUCKET_SIDE as u32 + dx as u32;
                    let z = bz * super::BUCKET_SIDE as u32 + dz as u32;
                    if x >= table.cols || z >= table.rows {
                        continue;
                    }
                    let cell = tile(table.origin.x + x as i32, table.origin.z + z as i32, level);
                    let mut rectangles = 0;
                    let mut comparisons = 0;
                    for &index in candidates {
                        let (contained, tested, partitioned) = membership_probe(table, index, cell);
                        assert_eq!(contained, table.zone_contains(index, cell));
                        rectangles += tested;
                        comparisons += partitioned;
                    }
                    visited_tiles += 1;
                    if rectangles > max_executed_rectangles {
                        max_executed_rectangles = rectangles;
                        worst_rectangle_tile = cell;
                    }
                    if comparisons > max_partition_comparisons {
                        max_partition_comparisons = comparisons;
                        worst_partition_tile = cell;
                    }
                    if candidates.len() > max_candidates_per_tile {
                        max_candidate_tile = cell;
                    }
                }
            }
            max_candidates_per_tile = max_candidates_per_tile.max(candidates.len());
            let carves = candidates
                .iter()
                .map(|&zone| carves_per_zone[usize::from(zone)])
                .sum::<usize>();
            if carves > max_carves_per_bucket {
                max_carves_per_bucket = carves;
                worst_bucket = key;
            }
        }
        assert!(max_candidates_per_tile <= bound.bucket_candidates);
        assert!(max_carves_per_candidate <= bound.carves_per_candidate);
        assert!(max_carves_per_bucket <= bound.carves_per_bucket);
        assert!(max_executed_rectangles <= bound.executed_rectangle_tests);
        assert!(max_partition_comparisons <= bound.partition_comparisons);
        assert!(world.graph.wilderness.zones.len() <= bound.wilderness_zones);
        eprintln!(
            "{}",
            serde_json::json!({
                "case": "packed_membership_work_bound",
                "revision": revision,
                "nav_sha256": manifest.nav_sha256,
                "zone_count": table.zones.len(),
                "carve_count": table.carves.len(),
                "max_candidates_per_tile": max_candidates_per_tile,
                "max_carves_per_candidate": max_carves_per_candidate,
                "max_carves_per_bucket": max_carves_per_bucket,
                "worst_carve_bucket": worst_bucket,
                "occupied_buckets": table.buckets.len(),
                "wilderness_zone_count": world.graph.wilderness.zones.len(),
                "visited_tiles": visited_tiles,
                "max_executed_rectangle_tests": max_executed_rectangles,
                "max_partition_comparisons": max_partition_comparisons,
                "worst_rectangle_call": membership_call_timings(table, worst_rectangle_tile),
                "worst_partition_call": membership_call_timings(table, worst_partition_tile),
                "most_candidates_call": membership_call_timings(table, max_candidate_tile),
            })
        );
    }
}
