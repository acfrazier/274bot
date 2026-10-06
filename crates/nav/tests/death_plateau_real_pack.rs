//! The Death Plateau scouting ridge must not require a thrower-troll grant.
use std::path::PathBuf;

use api::line_of_sight::{has_line_of_sight_local, Footprint};
use api::WorldTile;
use nav::router::{find_with, FindOptions, Leg};
use nav::world::NavWorld;
use nav::WorldState;

fn tile(x: i32, z: i32) -> WorldTile {
    WorldTile { x, z, level: 0 }
}

#[test]
#[ignore = "requires NAV_DEATH_PLATEAU_PACK naming a freshly baked real 289 pack"]
fn secret_way_reaches_scouting_zone_without_thrower_grant() {
    let pack = PathBuf::from(std::env::var_os("NAV_DEATH_PLATEAU_PACK").expect("explicit pack"));
    let mut world = NavWorld::load_pack(&pack).expect("real 289 pack");
    let (origin, width, height, flags) = nav::pack::decode_flags_sidecar(
        &std::fs::read(pack.with_extension("navflags")).expect("raw flags"),
    )
    .expect("decode flags");
    assert_eq!(
        (origin, width, height),
        (
            world.collision.origin,
            world.collision.width,
            world.collision.height
        )
    );
    world.collision.attach_flags(flags);
    let from = tile(2817, 3564);
    let to = tile(2865, 3609);
    let selected = api::game_data::for_revision(api::selected::ClientRevision::R289).unwrap();
    let boots = selected.item_by_alias("death_climbingboots").unwrap().id;
    let table = world.graph.zones.as_ref().expect("baked zones");
    let throwers: Vec<_> = table
        .zones()
        .iter()
        .filter(|zone| (1101..=1105).contains(&table.kinds()[usize::from(zone.kind)].npc_id))
        .collect();
    assert_eq!(throwers.len(), 6, "five NPC kinds, two placements of 1105");
    for worn in [false, true] {
        let mut state = WorldState::empty().with_map_members(true);
        if worn {
            state.worn.insert(boots);
        }
        let route = find_with(
            &world.collision,
            &world.graph,
            from,
            to,
            FindOptions::default(),
            &state,
        )
        .expect("safe secret way needs no cross grant, with or without boots");
        assert_eq!(route.dest, to);
        for leg in &route.legs {
            let Leg::Walk { tiles } = leg else {
                panic!("ridge is an ordinary walk after the stile: {leg:?}");
            };
            for here in tiles {
                assert_eq!(
                    table.at(*here).count(),
                    0,
                    "safe ridge tile {here:?} must not enter a danger area"
                );
                for zone in &throwers {
                    let within_hunt = (here.x - zone.spawn_x)
                        .abs()
                        .max((here.z - zone.spawn_z).abs())
                        <= 8;
                    if within_hunt {
                        assert!(
                            !has_line_of_sight_local(
                                &|x, z| Some(world.collision.flag(x, z, here.level) as i32),
                                Footprint {
                                    lx: here.x,
                                    lz: here.z,
                                    size: 1
                                },
                                Footprint {
                                    lx: zone.spawn_x,
                                    lz: zone.spawn_z,
                                    size: 1
                                },
                            ),
                            "ridge tile {here:?} must be occluded from thrower at {},{}",
                            zone.spawn_x,
                            zone.spawn_z
                        );
                    }
                }
            }
        }
        println!("safe ridge boots_worn={worn}: {:?}", route.legs);
    }
    for id in [3722, 3723] {
        let edges: Vec<_> = world
            .graph
            .edges
            .iter()
            .filter(|edge| edge.loc_id == id)
            .collect();
        assert!(!edges.is_empty(), "real Climb loc {id} must be present");
        let mut worn = WorldState::empty();
        worn.worn.insert(boots);
        for edge in edges {
            assert_eq!(edge.worn_all_req, [boots]);
            assert!(edge.worn_req.is_empty());
            assert!(worn.allows(edge));
            assert!(!WorldState::empty().allows(edge));
            let mut carried = WorldState::empty();
            carried.inv.insert(boots, 1);
            assert!(!carried.allows(edge));
        }
    }
}
