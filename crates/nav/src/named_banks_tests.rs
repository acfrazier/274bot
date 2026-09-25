use super::*;
use api::named_banks::{BankOperation, BankPlacementKind};

const fn t(x: i32, z: i32) -> WorldTile { WorldTile { x, z, level: 0 } }
const BOOTH: BankDefinition = BankDefinition {
    name: "Booth", tile: t(100, 101), approach: None, skill: None, quest: None, setting: None,
    object: None, open_first: None, npc: None, choose: None,
};
const CATALOG: &[BankDefinition] = &[
    BOOTH,
    BankDefinition { name: "Chest", tile: t(200, 200), object: Some(BankOperation { name: "Open chest", op: "Bank" }), ..BOOTH },
    BankDefinition { name: "Teller", tile: t(300, 301), npc: Some(BankOperation { name: "Banker", op: "Bank" }), ..BOOTH },
];
fn placement(name: &str, kind: BankPlacementKind, x: i32, z: i32) -> BankPlacement {
    BankPlacement { name: name.into(), kind, id: 1, x, z, level: 0, width: 1, length: 1 }
}

#[test]
fn bank_access_stands_never_land_in_a_footprint_or_on_blocked_collision() {
    let mut chest = placement("Chest", BankPlacementKind::Object, 200, 200);
    chest.width = 2;
    chest.length = 3;
    let placements = [
        placement("Booth", BankPlacementKind::Object, 100, 100),
        chest,
        placement("Teller", BankPlacementKind::Npc, 300, 300),
    ];
    let facts = resolve(CATALOG, &placements, |tile| tile != BOOTH.tile);
    for bank in facts.banks() {
        let access = placements.iter().find(|access| access.name == bank.name).unwrap();
        assert!(bank.routable, "{} must resolve its supported access kind", bank.name);
        assert_ne!(bank.tile, BOOTH.tile);
        assert_eq!(footprint_distance(bank.tile, access), 1);
    }
    assert_eq!(facts.banks()[2].tile, CATALOG[2].tile, "valid preferred stand survives");
}

#[test]
fn unsupported_access_is_air_fallback_not_an_invented_route_target() {
    let mut wrong_plane = placement("Booth", BankPlacementKind::Object, 100, 100);
    wrong_plane.level = 1;
    let wrong_kind = placement("Teller", BankPlacementKind::Object, 300, 300);
    let facts = resolve(CATALOG, &[wrong_plane, wrong_kind], |_| true);
    for (bank, original) in facts.banks().iter().zip(CATALOG) {
        assert!(!bank.routable);
        assert_eq!(bank.name, original.name);
        assert_eq!(bank.tile, original.tile);
    }
    let blocked = resolve(CATALOG, &[placement("Booth", BankPlacementKind::Object, 100, 100)], |_| false);
    assert!(!blocked.banks()[0].routable);
    assert_eq!(blocked.banks()[0].tile, BOOTH.tile);
}

#[test]
fn a_bank_stand_can_have_a_wall_face_without_being_a_blocked_footprint() {
    use crate::{collision::{pack_walk, WorldCollision}, transport::TransportGraph, world::NavWorld};
    use client::dash3d::CollisionFlag;
    // Varrock West's preferred stand is west of its booth, against the
    // east-facing counter wall. The player can step onto it from the west.
    let mut flags = vec![CollisionFlag::SQ_BLOCKED as u32; 4 * 4 * 4];
    flags[2 * 4 + 1] = 0;
    flags[2 * 4 + 2] = CollisionFlag::W_E as u32;
    let (walk, blocked) = pack_walk(&flags);
    let world = NavWorld::from_parts(WorldCollision {
        origin: t(3183, 3438), width: 4, height: 4, walk, blocked, flags: None,
    }, TransportGraph::default(), Vec::new());
    let data = api::game_data::for_revision(client::io::ClientRevision::R289).unwrap();
    let facts = world.named_bank_facts(Some(&data));
    let bank = facts.banks().iter().find(|bank| bank.name == "Varrock West").unwrap();
    assert!(bank.routable, "a wall face is not a blocked bank stand");
    assert_eq!(bank.tile, t(3185, 3440));
    let route = crate::router::find(&world.collision, &world.graph, t(3184, 3440), bank.tile).unwrap();
    assert_eq!(route.ticks, 0.5);
}
