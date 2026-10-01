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
    assert!(normal.blocks(&wilderness, tile(5, 5, 0)));

    let above_cap = ZoneFilter::new(&table, Some(11), &[], &ZoneExempt::NONE);
    assert!(!above_cap.blocks(&wilderness, tile(5, 5, 0)));
    assert!(above_cap.blocks(&wilderness, tile(10, 10, 0)));

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
    assert!(above_cap.blocks(&wilderness, tile(5, 5, 0)));

    let missing_level = ZoneFilter::new(&table, None, &[], &ZoneExempt::NONE);
    assert!(missing_level.blocks(&WildernessRules::default(), tile(5, 5, 0)));
}

#[test]
fn named_group_and_endpoint_exemptions_mask_whole_zone_identities() {
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
    assert!(by_group.masked(0));
    assert!(!by_group.masked(1));
    assert!(!by_group.blocks(&wilderness, tile(4, 5, 0)));
    assert_eq!(
        by_group
            .blocking_at(&wilderness, tile(5, 5, 0))
            .collect::<Vec<_>>(),
        vec![1]
    );

    let endpoint = ZoneFilter::new(&table, Some(11), &[tile(5, 5, 0)], &ZoneExempt::NONE);
    assert!(endpoint.masked(0));
    assert!(endpoint.masked(1));
    assert!(!endpoint.blocks(&wilderness, tile(5, 5, 0)));
}

#[test]
fn exemption_union_deduplicates_and_all_bypasses_mask_allocation() {
    let table = table();
    let first = ZoneExempt::named(&[ZoneKey::Group(0), ZoneKey::Zone(2)]).unwrap();
    let second = ZoneExempt::named(&[ZoneKey::Group(0), ZoneKey::Zone(1)]).unwrap();
    let combined = first.union(second).unwrap();
    let filter = ZoneFilter::new(&table, Some(10), &[], &combined);
    assert!(filter.masked(0));
    assert!(filter.masked(1));
    assert!(filter.masked(2));
    let duplicates = ZoneExempt::named(&[ZoneKey::Zone(0); 8]).unwrap();
    let extra = ZoneExempt::named(&[ZoneKey::Zone(1)]).unwrap();
    let deduplicated = duplicates.union(extra).unwrap();
    let deduplicated_filter = ZoneFilter::new(&table, Some(10), &[], &deduplicated);
    assert!(deduplicated_filter.masked(0));
    assert!(deduplicated_filter.masked(1));
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
    assert!(full.union(extra).is_err());
    assert!(ZoneExempt::named(&[ZoneKey::Zone(0x8000)]).is_err());

    let all = ZoneFilter::new(&table, Some(10), &[], &ZoneExempt::all());
    assert_eq!(ZoneExempt::default(), ZoneExempt::NONE);
    assert!(ZoneExempt::all().is_all());
    assert_eq!(all.mask_words(), 0);
    assert!(all.masked(0));
    assert!(!all.blocks(&WildernessRules::default(), tile(10, 10, 0)));
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

