//! Resolve injected catalog bank aliases against bound NavWorld packs.
//!
//! The catalog alias table itself lives in `script::content::BANK_ALIASES`
//! and is verified against the pinned packs in `crates/script/tests`; this
//! suite drives `NavWorld::named_bank_facts` with its own fixture rows so
//! the world-side controls (packed stand gathering, walk surface, plane)
//! stay independent of the catalog table.

use std::path::PathBuf;

use api::named_banks::BankAliasCandidate;
use api::snapshot::WorldTile;
use nav::collision::{pack_walk, WorldCollision};
use nav::pack::{BankAccess, BankStand};
use nav::transport::TransportGraph;
use nav::world::NavWorld;

const fn t(x: i32, z: i32) -> WorldTile {
    WorldTile { x, z, level: 0 }
}

/// Fixture cluster: the packed Edgeville booth tiles (a world fact, not the
/// catalog table) with a preferred stand that is not adjacent to the
/// cluster, so resolution has to derive one.
const FIXTURE_BOOTHS: &[WorldTile] = &[t(3095, 3491), t(3096, 3493)];
const FIXTURE_ALIASES: &[BankAliasCandidate] = &[BankAliasCandidate {
    name: "Fixture",
    stand: t(3094, 3493),
    booths: FIXTURE_BOOTHS,
}];

fn pinned_pack(rev: &str) -> Option<NavWorld> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(".superpowers/world-capabilities")
        .join(rev)
        .join("274bot.navpack");
    match NavWorld::load_pack(&path) {
        Ok(world) => Some(world),
        Err(err) => {
            eprintln!("SKIP: no pinned {rev} pack at {} ({err:?})", path.display());
            None
        }
    }
}

fn cheb(a: WorldTile, b: WorldTile) -> i32 {
    (a.x - b.x).abs().max((a.z - b.z).abs())
}

fn packed_booths(world: &NavWorld) -> Vec<WorldTile> {
    world
        .banks()
        .iter()
        .filter(|stand| matches!(stand.access, BankAccess::Booth { .. }))
        .map(|stand| stand.tile)
        .collect()
}

fn facts_of(world: &NavWorld) -> api::named_banks::NamedBankFacts {
    world.named_bank_facts(FIXTURE_ALIASES)
}

fn assert_fixture_cluster_resolves(world: &NavWorld) {
    let facts = facts_of(world);
    let packed = packed_booths(world);
    assert!(
        FIXTURE_BOOTHS.iter().any(|booth| packed.contains(booth)),
        "no fixture cluster booth is a packed booth stand in this world"
    );
    let row = facts
        .banks()
        .iter()
        .find(|b| b.name == "Fixture")
        .expect("fixture cluster must resolve");
    assert!(
        world.collision.walkable(row.tile),
        "{} stand {:?} must be walkable",
        row.name,
        row.tile
    );
    assert!(
        !packed.contains(&row.tile),
        "{} stand {:?} must not be a packed booth",
        row.name,
        row.tile
    );
    assert!(
        FIXTURE_BOOTHS.iter().any(|booth| packed.contains(booth)
            && booth.level == row.tile.level
            && cheb(row.tile, *booth) == 1),
        "{} stand {:?} must be Chebyshev-1 to a packed cluster booth",
        row.name,
        row.tile
    );
    eprintln!(
        "fixture named-bank stand=({},{},{})",
        row.tile.x, row.tile.z, row.tile.level
    );
}

#[test]
fn pinned_274_and_289_packs_resolve_the_fixture_cluster() {
    let Some(world_274) = pinned_pack("274") else {
        return;
    };
    let Some(world_289) = pinned_pack("289") else {
        return;
    };
    assert_fixture_cluster_resolves(&world_274);
    assert_fixture_cluster_resolves(&world_289);
    let a = facts_of(&world_274);
    let b = facts_of(&world_289);
    assert_eq!(a, b, "selected 274/289 packed booth tiles resolve equally");
}

fn open_world(origin: WorldTile, width: usize, height: usize, banks: Vec<BankStand>) -> NavWorld {
    let cells = 4 * width * height;
    let flags = vec![0u32; cells];
    let (walk, blocked) = pack_walk(&flags);
    let collision = WorldCollision {
        origin,
        width,
        height,
        walk,
        blocked,
        flags: None,
    };
    NavWorld::from_parts(collision, TransportGraph::default(), banks)
}

#[test]
fn bound_world_without_the_fixture_booths_omits_the_alias() {
    let origin = WorldTile {
        x: 3010,
        z: 3350,
        level: 0,
    };
    let world = open_world(
        origin,
        10,
        10,
        vec![BankStand {
            name: "Bank booth".into(),
            tile: t(3011, 3354),
            access: BankAccess::Booth { op: 2 },
        }],
    );
    assert!(
        world.named_bank_facts(FIXTURE_ALIASES).banks().is_empty(),
        "a world without the fixture cluster publishes no fixture alias"
    );
}

#[test]
fn npc_teller_is_not_a_named_booth_alias() {
    let origin = WorldTile {
        x: 3090,
        z: 3490,
        level: 0,
    };
    let world = open_world(
        origin,
        10,
        10,
        vec![BankStand {
            name: "Banker".into(),
            tile: t(3095, 3491),
            access: BankAccess::Npc {
                name: "Banker".into(),
                op: 1,
                choose: None,
            },
        }],
    );
    assert!(facts_of(&world).banks().is_empty());
}
