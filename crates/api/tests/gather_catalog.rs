//! The typed gathering catalog over the checked 289/274 assets. Placement anchors below are rows of the pinned
//! content maps (`maps/m38_50.jm2:5131` is `0 38 55: 2090`, i.e. plane 0, local 38/55 of map square 38,50).
use api::game_data::{for_revision, GatherForbiddenState};
use api::gather_methods::{
    first_gap, known_rows, AccessPolicy, GatherCatalog, GatherMethod, GatherSkill, GatherSpot,
    SceneRegionInput, SpotId, TargetClass,
};
use api::selected::{ClientRevision, EntityId, FactError, FamilyPreparation, Knowledge, Truth};
use api::WorldTile;
use std::sync::{Arc, LazyLock};

fn prepare(revision: ClientRevision) -> Arc<GatherCatalog> {
    let data = for_revision(revision).expect("selected data");
    FamilyPreparation::run(move |worker| data.prepare_gathering(worker))
        .expect("worker")
        .join()
        .expect("worker panicked")
        .expect("gathering family prepares")
}

fn c289() -> &'static Arc<GatherCatalog> {
    static CATALOG: LazyLock<Arc<GatherCatalog>> = LazyLock::new(|| prepare(ClientRevision::R289));
    &CATALOG
}

fn spots(method: &GatherMethod) -> &[GatherSpot] {
    match &method.spots {
        Knowledge::Known(spots) => spots,
        other => panic!("{} placements are not complete: {other:?}", method.id.0),
    }
}

fn world(level: i32) -> SceneRegionInput {
    SceneRegionInput {
        min_x: -1_000,
        min_z: -1_000,
        max_x: 20_000,
        max_z: 20_000,
        level,
    }
}

fn inside(region: &SceneRegionInput, spot: &GatherSpot) -> bool {
    spot.origin.level == region.level
        && (region.min_x..=region.max_x).contains(&spot.origin.x)
        && (region.min_z..=region.max_z).contains(&spot.origin.z)
}

fn alias(catalog: &GatherCatalog, item: i32) -> &str {
    catalog.alias(EntityId::Obj(item)).expect("item alias")
}

#[test]
fn a_known_rock_query_returns_the_content_row() {
    let catalog = c289();
    let copper = catalog.method("mining.copper").unwrap();
    let tile = SceneRegionInput {
        min_x: 2470,
        min_z: 3255,
        max_x: 2470,
        max_z: 3255,
        level: 0,
    };
    let found: Vec<_> = catalog.spots(copper, &tile).unwrap().collect();
    assert_eq!(found.len(), 1, "one copper rock stands on that tile");
    let spot = found[0];
    assert_eq!(spot.entity, EntityId::Loc(2090));
    assert_eq!(catalog.alias(spot.entity), Some("copperrock1"));
    assert_eq!((spot.width, spot.length), (1, 1));
    assert_eq!(
        (&*spot.source.file, spot.source.first, spot.source.last),
        ("maps/m38_50.jm2", 5131, 5131)
    );
    let (owner, same) = catalog
        .spot(spot.id)
        .expect("an id resolves in its own catalog");
    assert_eq!((&*owner.id.0, same.id), ("mining.copper", spot.id));

    // One tile away, one level up, and an inverted box all hold no copper: complete coverage, so empty is honest.
    for miss in [
        SceneRegionInput {
            min_x: 2471,
            max_x: 2471,
            ..tile
        },
        SceneRegionInput { level: 1, ..tile },
        SceneRegionInput {
            min_x: 2471,
            max_x: 2470,
            ..tile
        },
    ] {
        assert_eq!(catalog.spots(copper, &miss).unwrap().count(), 0, "{miss:?}");
    }
    // The same tile is no oak: a method only offers its own resource targets.
    let oak = catalog.method("woodcutting.oak").unwrap();
    assert_eq!(catalog.spots(oak, &tile).unwrap().count(), 0);
}

#[test]
fn fishing_spots_are_npc_spawns_with_their_wander_region() {
    let catalog = c289();
    let lure = catalog.method("fishing.freshfish.op1").unwrap();
    let tile = SceneRegionInput {
        min_x: 2210,
        min_z: 3237,
        max_x: 2210,
        max_z: 3237,
        level: 0,
    };
    let found: Vec<_> = catalog.spots(lure, &tile).unwrap().collect();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].entity, EntityId::Npc(1189));
    assert_eq!(
        (&*found[0].source.file, found[0].source.first),
        ("maps/m34_50.jm2", 5953)
    );
    match &found[0].movement {
        Knowledge::Known(Some(region)) => {
            assert!(
                (region.min_x..=region.max_x).contains(&2210)
                    && (region.min_z..=region.max_z).contains(&3237),
                "the wander region contains its own spawn: {region:?}"
            );
        }
        other => panic!("npc spawns carry a movement region, got {other:?}"),
    }
    // A loc spot never has a wander region: no movement applies, which is a known fact, not an unknown one.
    let copper = catalog.method("mining.copper").unwrap();
    assert!(matches!(spots(copper)[0].movement, Knowledge::Known(None)));
}

#[test]
fn region_queries_match_a_straight_filter_over_the_complete_placements() {
    let catalog = c289();
    let trees = catalog.method("woodcutting.normal").unwrap();
    let all = spots(trees);
    assert!(
        all.len() > 1000,
        "the shared placement index is complete, not a sample"
    );
    let anchor = all[all.len() / 3].origin;
    let boxes = [
        // Straddling 64-tile map-square edges, and one level apart from the anchor.
        SceneRegionInput {
            min_x: anchor.x - 70,
            max_x: anchor.x + 70,
            min_z: anchor.z - 70,
            max_z: anchor.z + 70,
            level: anchor.level,
        },
        SceneRegionInput {
            min_x: anchor.x,
            max_x: anchor.x,
            min_z: anchor.z,
            max_z: anchor.z,
            level: anchor.level,
        },
        SceneRegionInput {
            min_x: anchor.x - 300,
            max_x: anchor.x + 300,
            min_z: 0,
            max_z: 20_000,
            level: anchor.level,
        },
        world(anchor.level),
        world(3),
    ];
    for region in boxes {
        let indexed: Vec<SpotId> = catalog
            .spots(trees, &region)
            .unwrap()
            .map(|spot| spot.id)
            .collect();
        let scanned: Vec<SpotId> = all
            .iter()
            .filter(|spot| inside(&region, spot))
            .map(|spot| spot.id)
            .collect();
        assert_eq!(indexed, scanned, "{region:?}");
        assert!(
            indexed.windows(2).all(|pair| pair[0] < pair[1]),
            "ascending ids: {region:?}"
        );
    }
}

#[test]
fn unknown_placements_are_an_error_never_an_empty_iterator() {
    let catalog = c289();
    let jungle = catalog.method("woodcutting.jungle").unwrap();
    match &jungle.spots {
        Knowledge::Unknown(gap) => {
            assert_eq!(&*gap.code, "no-resource-target");
            assert!(
                !gap.sources.is_empty(),
                "the gap points at the content that lacks the target"
            );
        }
        other => panic!("jungle trees have no resource target in content: {other:?}"),
    }
    assert!(matches!(
        catalog.spots(jungle, &world(0)),
        Err(FactError::Invalid { .. })
    ));
    assert!(matches!(
        catalog.page(jungle, &world(0), None, 10),
        Err(FactError::Invalid { .. })
    ));
    let from = WorldTile {
        x: 3200,
        z: 3200,
        level: 0,
    };
    assert!(matches!(
        catalog.nearest(jungle, from, None, AccessPolicy::Any, 3),
        Err(FactError::Invalid { .. })
    ));
    // Every cell of that method is honestly unknown, none is an empty known set.
    assert!(matches!(jungle.targets, Knowledge::Unknown(_)));
    assert!(matches!(jungle.products, Knowledge::Unknown(_)));
    assert!(matches!(
        catalog.method("no.such.method"),
        Err(FactError::UnknownKey(_))
    ));
}

#[test]
fn unclassified_rocks_stay_unknown_with_a_reason_and_are_never_resources() {
    let catalog = c289();
    let rocks = catalog.rocks();
    let count = |class| rocks.iter().filter(|rock| rock.class == class).count();
    assert_eq!(
        rocks.len(),
        count(TargetClass::Resource)
            + count(TargetClass::Depleted)
            + count(TargetClass::Hazard)
            + count(TargetClass::Unclassified),
        "every rock is in exactly one class"
    );
    for rock in &rocks {
        assert_eq!(
            rock.gap.is_some(),
            rock.class == TargetClass::Unclassified,
            "{}: only an unclassified rock carries a gap",
            rock.alias
        );
    }

    // The hero quest rockslide has its own handler script: content classifies it as neither resource nor empty.
    let slide = catalog.rock(2634).expect("the rockslide is a known rock");
    assert_eq!(slide.alias, "herorockslide");
    assert_eq!(slide.class, TargetClass::Unclassified);
    assert_eq!(slide.gap.map(|gap| &*gap.code), Some("custom-handler"));
    assert!(slide
        .gap
        .unwrap()
        .sources
        .iter()
        .any(|span| span.file.ends_with(".loc")));
    assert!(slide.method.is_none());
    // A loc with no handler anywhere is proven inert by content and engine dispatch
    // (M-215 R2), so it is excluded from the mining catalogue entirely: no rock fact.
    assert!(
        catalog.rock(4976).is_none(),
        "loc 4976 has no handler, so it is not a rock at all"
    );
    assert!(
        catalog.rock(3431).is_none(),
        "newbierocks1 has no handler, so it is not a rock at all"
    );
    // No method ever offers an unclassified rock as a target.
    for method in catalog.methods() {
        for target in known_rows(&method.targets) {
            if let EntityId::Loc(loc) = target.entity {
                assert!(
                    !matches!(catalog.rock(loc), Some(rock) if rock.class == TargetClass::Unclassified),
                    "{} offers unclassified rock {loc}",
                    method.id.0
                );
            }
        }
    }

    // Classified rocks say which method they belong to; shared empty stages belong to none.
    let copper = catalog.rock(2090).unwrap();
    assert_eq!(
        (copper.class, copper.method.map(|m| &*m.id.0)),
        (TargetClass::Resource, Some("mining.copper"))
    );
    // M-215: the tutorial Mine handlers provably yield ore, so those rocks are typed resources.
    let tutorial_copper = catalog.rock(3042).unwrap();
    assert_eq!(
        (
            tutorial_copper.class,
            tutorial_copper.alias,
            tutorial_copper.method.map(|m| &*m.id.0)
        ),
        (
            TargetClass::Resource,
            "newbiecopperrock",
            Some("mining.copper")
        )
    );
    let tutorial_tin = catalog.rock(3043).unwrap();
    assert_eq!(
        (
            tutorial_tin.class,
            tutorial_tin.alias,
            tutorial_tin.method.map(|m| &*m.id.0)
        ),
        (TargetClass::Resource, "newbietinrock", Some("mining.tin"))
    );
    let gas = catalog.rock(2119).unwrap();
    assert_eq!(
        (gas.class, gas.alias),
        (TargetClass::Hazard, "macro_copperrock1")
    );
    let stage = catalog.rock(450).unwrap();
    assert_eq!(stage.class, TargetClass::Depleted);
    assert!(stage.method.is_none(), "rocks1 is every ore's empty stage");
    // A tree is not a rock, and neither is an id nothing mentions.
    assert!(catalog.rock(1276).is_none());
    assert!(catalog.rock(-1).is_none());
}

#[test]
fn bounded_pages_say_truncated_and_resume_by_cursor() {
    let catalog = c289();
    let trees = catalog.method("woodcutting.normal").unwrap();
    let region = world(0);
    let all: Vec<SpotId> = catalog
        .spots(trees, &region)
        .unwrap()
        .map(|spot| spot.id)
        .collect();
    assert!(all.len() > 200, "enough placements to need several pages");

    let mut seen = Vec::new();
    let mut after = None;
    let mut pages = 0;
    loop {
        let page = catalog.page(trees, &region, after, 64).unwrap();
        assert!(page.spots.len() <= 64);
        seen.extend(page.spots.iter().map(|spot| spot.id));
        pages += 1;
        if !page.truncated {
            assert!(page.next.is_none(), "the last page has no cursor");
            break;
        }
        assert_eq!(page.spots.len(), 64, "a truncated page is full");
        assert_eq!(page.next, page.spots.last().map(|spot| spot.id));
        after = page.next;
    }
    assert_eq!(seen, all, "pages tile the complete result exactly");
    assert!(pages > 1);

    // The boundary: a page that holds exactly the remainder is not truncated, one short is.
    let exact = catalog.page(trees, &region, None, all.len()).unwrap();
    assert!(!exact.truncated && exact.next.is_none() && exact.spots.len() == all.len());
    let short = catalog.page(trees, &region, None, all.len() - 1).unwrap();
    assert!(short.truncated);
    let rest = catalog.page(trees, &region, short.next, all.len()).unwrap();
    assert_eq!(
        rest.spots.iter().map(|spot| spot.id).collect::<Vec<_>>(),
        vec![*all.last().unwrap()]
    );
    assert!(!rest.truncated);
    // A region with nothing in it is a complete empty page, and a zero limit is refused rather than looping.
    let empty = catalog.page(trees, &world(3), None, 5).unwrap();
    assert!(empty.spots.is_empty() && !empty.truncated && empty.next.is_none());
    assert!(matches!(
        catalog.page(trees, &region, None, 0),
        Err(FactError::Invalid { .. })
    ));
}

#[test]
fn nearest_searches_the_complete_domain_not_the_first_page() {
    let catalog = c289();
    for id in [
        "mining.copper",
        "woodcutting.normal",
        "fishing.rarefish.op1",
    ] {
        let method = catalog.method(id).unwrap();
        let all = spots(method);
        for from in [
            WorldTile {
                x: 3200,
                z: 3200,
                level: 0,
            },
            WorldTile {
                x: 2500,
                z: 3400,
                level: 0,
            },
            WorldTile {
                x: 0,
                z: 0,
                level: 0,
            },
        ] {
            let mut expected: Vec<(u64, SpotId)> = all
                .iter()
                .filter(|spot| spot.origin.level == from.level)
                .map(|spot| {
                    let (dx, dz) = (
                        i64::from(spot.origin.x - from.x),
                        i64::from(spot.origin.z - from.z),
                    );
                    ((dx * dx + dz * dz) as u64, spot.id)
                })
                .collect();
            expected.sort_unstable();
            expected.truncate(5);
            let found: Vec<(u64, SpotId)> = catalog
                .nearest(method, from, None, AccessPolicy::Any, 5)
                .unwrap()
                .iter()
                .map(|near| (near.distance_sq, near.spot.id))
                .collect();
            assert_eq!(found, expected, "{id} from {from:?}");
        }
    }
    let copper = catalog.method("mining.copper").unwrap();
    let from = WorldTile {
        x: 3200,
        z: 3200,
        level: 0,
    };
    assert!(catalog
        .nearest(copper, from, None, AccessPolicy::Any, 0)
        .unwrap()
        .is_empty());
    // No copper stands on level 3: complete coverage, honest empty.
    let up = WorldTile { level: 3, ..from };
    assert!(catalog
        .nearest(copper, up, None, AccessPolicy::Any, 3)
        .unwrap()
        .is_empty());
    // `within` bounds the candidates.
    let bound = SceneRegionInput {
        min_x: 2400,
        max_x: 2500,
        min_z: 3200,
        max_z: 3300,
        level: 0,
    };
    let bounded = catalog
        .nearest(copper, from, Some(&bound), AccessPolicy::Any, 50)
        .unwrap();
    assert!(!bounded.is_empty() && bounded.iter().all(|near| inside(&bound, near.spot)));
}

#[test]
fn content_zones_decide_whether_a_placement_yields_to_the_player() {
    let catalog = c289();
    // Family Crest: the perfect gold replaces the product inside this cave rectangle.
    let gold = catalog.method("mining.gold").unwrap();
    let crest = SceneRegionInput {
        min_x: 2736,
        max_x: 2740,
        min_z: 9684,
        max_z: 9693,
        level: 0,
    };
    let in_cave: Vec<_> = catalog.spots(gold, &crest).unwrap().collect();
    assert!(!in_cave.is_empty(), "gold rocks stand in the crest cave");
    for spot in &in_cave {
        assert_eq!(catalog.access(gold, spot).unwrap(), Truth::False);
    }
    let elsewhere = spots(gold)
        .iter()
        .find(|spot| !inside(&crest, spot))
        .unwrap();
    assert_eq!(catalog.access(gold, elsewhere).unwrap(), Truth::True);
    let from = WorldTile {
        x: 2738,
        z: 9688,
        level: 0,
    };
    let any = catalog
        .nearest(gold, from, None, AccessPolicy::Any, 3)
        .unwrap();
    assert!(
        any.iter().all(|near| inside(&crest, near.spot)),
        "the cave rocks are closest"
    );
    let usable = catalog
        .nearest(gold, from, None, AccessPolicy::Usable, 3)
        .unwrap();
    assert!(!usable.is_empty());
    assert!(usable
        .iter()
        .all(|near| near.access == Truth::True && !inside(&crest, near.spot)));

    // Miscellania hands the chopped logs to its NPC: `Usable` never offers those trees, `Any` still shows them.
    let maple = catalog.method("woodcutting.maple").unwrap();
    let island = SceneRegionInput {
        min_x: 2496,
        max_x: 2582,
        min_z: 3840,
        max_z: 3903,
        level: 0,
    };
    assert!(catalog
        .zones(maple)
        .unwrap()
        .any(|zone| zone.min_x == 2496 && zone.max_z == 3903));
    let from = WorldTile {
        x: 2540,
        z: 3870,
        level: 0,
    };
    let tiles = catalog
        .nearest(maple, from, None, AccessPolicy::Any, 5)
        .unwrap();
    assert!(tiles
        .iter()
        .all(|near| inside(&island, near.spot) && near.access == Truth::False));
    let usable = catalog
        .nearest(maple, from, None, AccessPolicy::Usable, 5)
        .unwrap();
    assert!(usable.iter().all(|near| !inside(&island, near.spot)));
    let possible = catalog
        .nearest(maple, from, None, AccessPolicy::Possible, 5)
        .unwrap();
    assert!(possible.iter().all(|near| near.access != Truth::False));
}

#[test]
fn respawn_belongs_to_each_target_with_its_source_scaling() {
    let catalog = c289();
    let raw = |id: &str| -> Vec<Option<u32>> {
        known_rows(&catalog.method(id).unwrap().targets)
            .iter()
            .filter(|target| target.class == TargetClass::Resource)
            .map(|target| match &target.respawn {
                Knowledge::Known(Some(fact)) => Some(fact.raw),
                Knowledge::Known(None) => None,
                other => panic!("{id}: respawn is not a known fact: {other:?}"),
            })
            .collect()
    };
    assert_eq!(
        raw("mining.limestone"),
        [Some(10), Some(20), Some(40)],
        "three rocks, three rates"
    );
    assert_eq!(
        raw("mining.rune stones"),
        [None],
        "rune essence never respawns"
    );
    assert_eq!(
        raw("fishing.freshfish.op1").iter().flatten().count(),
        0,
        "fishing spots have no respawn fact"
    );

    let copper = catalog.method("mining.copper").unwrap();
    let rock = known_rows(&copper.targets)
        .iter()
        .find(|t| t.entity == EntityId::Loc(2090))
        .unwrap();
    let Knowledge::Known(Some(fact)) = &rock.respawn else {
        panic!("copper respawns")
    };
    assert_eq!(fact.raw, 10);
    let Knowledge::Known(scale) = &fact.scale else {
        panic!("scaling is source-derived")
    };
    assert_eq!((scale.min_ticks, scale.max_ticks), (5, 10));
    assert_eq!(&*scale.rule.0, "scale_by_playercount");
    assert!(!scale.sources.is_empty());
    assert!(fact.source.file.ends_with(".dbrow"));
    // Gas variants and empty stages are not respawning resources.
    for target in known_rows(&copper.targets)
        .iter()
        .filter(|t| t.class != TargetClass::Resource)
    {
        assert!(
            matches!(target.respawn, Knowledge::Known(None)),
            "{:?}",
            target.entity
        );
    }
}

#[test]
fn three_fishing_methods_declare_their_tool_bait_and_catches() {
    let catalog = c289();
    let describe = |id: &str| {
        let method = catalog.method(id).unwrap();
        let names = |items: Vec<i32>| {
            items
                .into_iter()
                .map(|item| alias(catalog, item))
                .collect::<Vec<_>>()
        };
        (
            catalog
                .op(method)
                .unwrap()
                .map(|(slot, label)| (slot, label.to_string())),
            names(
                known_rows(&method.tools)
                    .iter()
                    .map(|tool| tool.item)
                    .collect(),
            ),
            known_rows(&method.consumes)
                .iter()
                .map(|c| (alias(catalog, c.item), c.count))
                .collect::<Vec<_>>(),
            known_rows(&method.products)
                .iter()
                .map(|p| (alias(catalog, p.item), p.level))
                .collect::<Vec<_>>(),
            method,
        )
    };

    let (op, tools, bait, catches, lure) = describe("fishing.freshfish.op1");
    assert_eq!(op, Some((1, "Lure".to_string())));
    assert_eq!(tools, ["fly_fishing_rod"]);
    assert_eq!(bait, [("feather", 1)]);
    assert_eq!(
        catches,
        [("raw_trout", 20), ("raw_salmon", 30)],
        "alternative catches with their own levels"
    );
    assert!(
        matches!(lure.products, Knowledge::Known(_))
            && matches!(lure.consumes, Knowledge::Known(_))
    );

    // The same spawn, another option: rod and bait, one catch.
    let (op, tools, bait, catches, _) = describe("fishing.freshfish.op3");
    assert_eq!(op, Some((3, "Bait".to_string())));
    assert_eq!(
        (tools, bait, catches),
        (
            vec!["fishing_rod"],
            vec![("fishing_bait", 1)],
            vec![("raw_pike", 25)]
        )
    );

    // Netting shrimp needs a tool and no bait: an empty consumed set that is known, not missing.
    let (_, tools, bait, catches, net) = describe("fishing.saltfish.op1");
    assert_eq!(tools, ["net"]);
    assert!(bait.is_empty() && matches!(net.consumes, Knowledge::Known(_)));
    assert_eq!(catches, [("raw_shrimp", 0), ("raw_anchovies", 15)]);

    let (op, tools, bait, catches, _) = describe("fishing.rarefish.op3");
    assert_eq!(op, Some((3, "Harpoon".to_string())));
    assert_eq!((tools, bait), (vec!["harpoon"], vec![]));
    assert_eq!(catches, [("raw_tuna", 35), ("raw_swordfish", 50)]);

    // Level gates come from the handler: the lure needs fishing 20 and nothing else the extractor could prove.
    let gate = known_rows(&lure.requirements);
    assert_eq!(gate.len(), 1);
    assert!(
        matches!(&gate[0].kind, api::selected::RequirementKind::Skill(min) if (min.skill, min.level) == (10, 20))
    );

    // Content constructs the extractor cannot evaluate stay partial with a reason, never a silent full set.
    let karambwan = catalog.method("fishing.category_633.op1").unwrap();
    assert!(matches!(karambwan.products, Knowledge::Partial { .. }));
    assert!(matches!(karambwan.requirements, Knowledge::Partial { .. }));
    assert_eq!(
        first_gap(&karambwan.requirements).map(|gap| &*gap.code),
        Some("varp-gate")
    );
    // The greegree guard is a forbidden state, not a gap: the method stays selectable and the Gatherer refuses at begin.
    let members = catalog.method("fishing.memberfish.op3").unwrap();
    assert!(first_gap(&members.requirements).is_none());
    assert_eq!(
        catalog.forbidden_states(members).unwrap(),
        [GatherForbiddenState::MonkeyForm]
    );
}

#[test]
fn mining_products_are_partial_where_a_gem_roll_can_replace_the_ore() {
    let catalog = c289();
    let copper = catalog.method("mining.copper").unwrap();
    let Knowledge::Partial { known, gaps } = &copper.products else {
        panic!("a rock's yield includes an incidental gem roll the product set must not hide")
    };
    assert_eq!(known.len(), 1);
    assert_eq!(alias(catalog, known[0].item), "copper_ore");
    assert_eq!(&*gaps[0].code, "incidental-gem-roll");
    // Pickaxe tiers carry both the mining gate and the wield gate.
    let tools = known_rows(&copper.tools);
    assert!(tools.len() >= 6);
    assert!(tools
        .iter()
        .all(|tool| tool.use_gate.is_some() && tool.wield_gate.is_some()));
}

#[test]
fn content_hazards_and_incidental_gems_are_typed_catalog_facts() {
    for revision in [ClientRevision::R274, ClientRevision::R289] {
        let catalog = prepare(revision);
        let hazards = catalog.hazard_npcs();
        assert!(!hazards.is_empty(), "{revision:?}");
        assert!(
            hazards.windows(2).all(|pair| pair[0] < pair[1]),
            "{revision:?}"
        );
        let names: Vec<_> = hazards
            .iter()
            .map(|id| {
                catalog
                    .alias(EntityId::Npc(*id))
                    .expect("hazard id joins to an NPC")
            })
            .collect();
        assert!(
            names.iter().any(|name| name.starts_with("macro_ent_")),
            "{revision:?}"
        );
        assert!(
            names
                .iter()
                .any(|name| name.starts_with("macro_whirlpool_")),
            "{revision:?}"
        );

        let fishing_hazards: Vec<_> = catalog
            .methods()
            .iter()
            .filter(|method| method.skill == GatherSkill::Fishing)
            .flat_map(|method| known_rows(&method.targets))
            .filter_map(|target| match (target.class, target.entity) {
                (TargetClass::Hazard, EntityId::Npc(id)) => Some(id),
                _ => None,
            })
            .collect();
        assert!(!fishing_hazards.is_empty(), "{revision:?}");
        assert!(
            fishing_hazards.iter().all(|id| hazards.contains(id)),
            "{revision:?}"
        );

        let gems = catalog.incidental_gem_ids();
        assert!(!gems.is_empty(), "{revision:?}");
        assert!(
            gems.windows(2).all(|pair| pair[0] < pair[1]),
            "{revision:?}"
        );
        for id in gems {
            assert!(
                alias(&catalog, *id).starts_with("uncut_"),
                "{revision:?}: {id}"
            );
        }
        let copper = catalog.method("mining.copper").unwrap();
        assert!(known_rows(&copper.products)
            .iter()
            .all(|product| !gems.contains(&product.item)));
    }
}

/// The core's `mining_hazards` slice is exactly the family's mining hazard targets under the same pin, so the
/// gas-rock ids read from it are the family's own.
#[test]
fn gas_rock_ids_are_the_hazard_targets_of_the_ore_ladder_on_both_revisions() {
    let expected: Vec<i32> = (2119..=2139).collect();
    for revision in [ClientRevision::R274, ClientRevision::R289] {
        let data = for_revision(revision).unwrap();
        let catalog = prepare(revision);
        let from_family: Vec<api::gather_methods::MiningHazard> = catalog
            .methods()
            .iter()
            .filter(|method| method.skill == api::gather_methods::GatherSkill::Mining)
            .map(|method| api::gather_methods::MiningHazard {
                resources: method
                    .resources
                    .iter()
                    .map(|key| key.0.to_string())
                    .collect(),
                locs: known_rows(&method.targets)
                    .iter()
                    .filter_map(|target| match (target.class, target.entity) {
                        (TargetClass::Hazard, EntityId::Loc(id)) => Some(id),
                        _ => None,
                    })
                    .collect(),
            })
            .filter(|row| !row.locs.is_empty())
            .collect();
        assert_eq!(
            data.mining_hazards(),
            Some(from_family.as_slice()),
            "{revision:?}"
        );
        assert_eq!(
            api::gather_methods::gas_rock_ids(data.as_ref(), data.mining_hazards()).unwrap(),
            expected,
            "{revision:?}: gem rock 2140 is not a frozen gas rock"
        );
    }
}

#[test]
fn each_revision_prepares_its_own_catalog_for_its_own_pin() {
    let (a, b) = (prepare(ClientRevision::R274), c289().clone());
    for (catalog, revision) in [(&a, ClientRevision::R274), (&b, ClientRevision::R289)] {
        let pin = for_revision(revision).unwrap().selected_pin().unwrap();
        assert_eq!(catalog.pin(), &*pin);
    }
    assert_ne!(a.pin().revision, b.pin().revision);
    // Ids are catalog-local: one past the last placement resolves to nothing.
    let last = catalog_last_id(&b);
    assert!(b.spot(last).is_some() && b.spot(SpotId(last.0 + 1)).is_none());
    // 274 lacks 289-only Miscellania rules but keeps the crest cave rule.
    let copper = a.method("mining.copper").unwrap();
    assert!(a.zones(copper).unwrap().count() >= 1);
    let oak274 = a.method("woodcutting.oak").unwrap();
    assert_eq!(a.zones(oak274).unwrap().count(), 0);
}

fn catalog_last_id(catalog: &GatherCatalog) -> SpotId {
    catalog
        .methods()
        .iter()
        .flat_map(|method| known_rows(&method.spots).iter())
        .map(|spot| spot.id)
        .max()
        .unwrap()
}
