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
