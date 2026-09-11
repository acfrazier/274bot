//! Resolve catalog bank aliases against bound NavWorld packs.

use std::path::PathBuf;

use api::named_banks::CANDIDATES;
use api::snapshot::WorldTile;
use nav::collision::{pack_walk, WorldCollision};
use nav::pack::{BankAccess, BankStand};
use nav::transport::TransportGraph;
use nav::world::NavWorld;

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

fn assert_resolved_against_world(world: &NavWorld) {
    let facts = world.named_bank_facts();
    let packed: Vec<WorldTile> = world
        .banks()
        .iter()
        .filter(|stand| matches!(stand.access, BankAccess::Booth { .. }))
        .map(|stand| stand.tile)
        .collect();
    let names: Vec<&str> = facts.banks().iter().map(|b| b.name).collect();
    assert_eq!(
        names,
        [
            "Falador East",
            "Varrock East",
            "Edgeville",
            "Draynor",
            "Al Kharid"
        ]
    );
    for bank in facts.banks() {
        let candidate = CANDIDATES
            .iter()
            .find(|c| c.name == bank.name)
            .expect("candidate");
        assert!(
            world.collision.walkable(bank.tile),
            "{} stand {:?} must be walkable",
            bank.name,
            bank.tile
        );
        assert!(
            !packed.contains(&bank.tile),
            "{} stand {:?} must not be a packed booth",
            bank.name,
            bank.tile
        );
        assert!(
            candidate.booths.iter().any(|booth| packed.contains(booth)
                && booth.level == bank.tile.level
                && cheb(bank.tile, *booth) == 1),
            "{} stand {:?} must be Chebyshev-1 to a packed cluster booth",
            bank.name,
            bank.tile
        );
    }
    let falador = facts
        .banks()
        .iter()
        .find(|b| b.name == "Falador East")
        .expect("Falador East");
    assert_eq!(
        falador.tile,
        WorldTile {
            x: 3013,
            z: 3355,
            level: 0
        }
    );
    let varrock = facts
        .banks()
        .iter()
        .find(|b| b.name == "Varrock East")
        .expect("Varrock East");
    assert_eq!(
        varrock.tile,
        WorldTile {
            x: 3253,
            z: 3420,
            level: 0
        }
    );
    for bank in facts.banks() {
        eprintln!(
            "named-bank {} stand=({},{},{})",
            bank.name, bank.tile.x, bank.tile.z, bank.tile.level
        );
    }
}

#[test]
fn pinned_274_and_289_packs_publish_all_five_validated_aliases() {
    let Some(world_274) = pinned_pack("274") else {
        return;
    };
    let Some(world_289) = pinned_pack("289") else {
        return;
    };
    assert_resolved_against_world(&world_274);
    assert_resolved_against_world(&world_289);
    let a = world_274.named_bank_facts();
    let b = world_289.named_bank_facts();
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
fn bound_world_without_falador_booths_omits_falador() {
    let origin = WorldTile {
        x: 3090,
        z: 3490,
        level: 0,
    };
    let world = open_world(
        origin,
        10,
        10,
        vec![
            BankStand {
                name: "Bank booth".into(),
                tile: WorldTile {
                    x: 3095,
                    z: 3491,
                    level: 0,
                },
                access: BankAccess::Booth { op: 2 },
            },
            BankStand {
                name: "Bank booth".into(),
                tile: WorldTile {
                    x: 3096,
                    z: 3493,
                    level: 0,
                },
                access: BankAccess::Booth { op: 2 },
            },
        ],
    );
    let facts = world.named_bank_facts();
    let names: Vec<&str> = facts.banks().iter().map(|b| b.name).collect();
    assert_eq!(names, ["Edgeville"]);
    let stand = facts.banks()[0].tile;
    assert!(world.collision.walkable(stand));
    assert_ne!(
        stand,
        WorldTile {
            x: 3094,
            z: 3493,
            level: 0
        }
    );
}

#[test]
fn npc_teller_is_not_a_named_booth_alias() {
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
            name: "Banker".into(),
            tile: WorldTile {
                x: 3011,
                z: 3354,
                level: 0,
            },
            access: BankAccess::Npc {
                name: "Banker".into(),
                op: 1,
                choose: None,
            },
        }],
    );
    assert!(world.named_bank_facts().banks().is_empty());
}
