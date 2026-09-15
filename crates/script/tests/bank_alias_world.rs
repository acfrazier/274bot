//! The shipped catalog bank alias configuration against the bound world.
//!
//! `script::content::BANK_ALIASES` is catalog configuration and the resolver
//! now lives in `nav::named_banks`, so this suite owns the check that the
//! configuration still resolves to the five published aliases, their
//! preferred stands and the derived-stand fallback on the bound world. The
//! pinned packs are local bake artifacts (`docs/compat` evidence), so the
//! real-world test skips when they are absent — the same rule the alias test
//! used while the table lived below `script`.

use std::path::PathBuf;

use api::snapshot::WorldTile;
use nav::pack::BankAccess;
use nav::world::NavWorld;
use script::content::BANK_ALIASES;

const fn t(x: i32, z: i32) -> WorldTile {
    WorldTile { x, z, level: 0 }
}

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

fn tile_of(facts: &api::named_banks::NamedBankFacts, name: &str) -> WorldTile {
    facts
        .banks()
        .iter()
        .find(|b| b.name == name)
        .unwrap_or_else(|| panic!("{name} must resolve against the pinned world"))
        .tile
}

fn assert_shipped_aliases_resolve(world: &NavWorld) {
    let facts = world.named_bank_facts(BANK_ALIASES);
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
        let candidate = BANK_ALIASES
            .iter()
            .find(|c| c.name == bank.name)
            .expect("published alias has a configuration row");
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
    // Preferred stands the packed world supports are kept verbatim.
    assert_eq!(tile_of(&facts, "Falador East"), t(3013, 3355));
    assert_eq!(tile_of(&facts, "Varrock East"), t(3253, 3420));
    assert_eq!(tile_of(&facts, "Al Kharid"), t(3269, 3167));
    // The remaining two prefer a stand the world does not offer, so the
    // derived adjacent-walkable fallback is what gets published.
    let edgeville = tile_of(&facts, "Edgeville");
    assert_ne!(edgeville, t(3094, 3493));
    assert!(world.collision.walkable(edgeville));
    let draynor = tile_of(&facts, "Draynor");
    assert_ne!(draynor, t(3093, 3243));
    assert!(world.collision.walkable(draynor));
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
    assert_shipped_aliases_resolve(&world_274);
    assert_shipped_aliases_resolve(&world_289);
    let a = world_274.named_bank_facts(BANK_ALIASES);
    let b = world_289.named_bank_facts(BANK_ALIASES);
    assert_eq!(a, b, "selected 274/289 packed booth tiles resolve equally");
}
