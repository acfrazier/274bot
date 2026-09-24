//! Script-side curated site configuration and the frozen hostile-attacker
//! predicate.
//!
//! These tables are curated training/teleport preferences, not generated
//! revision facts: revision facts come from `api::game_data::SelectedGameData`
//! and the shared loot predicate stays in `api::content`. The rows are posted
//! through the existing content payload (`shim::content_json`) for the
//! catalog shims that read them. The catalog bank aliases are the same class
//! of configuration: their names, preferred stands and cluster booth
//! preferences live here, and `nav::named_banks` projects them onto the bound
//! world's packed stands and walk surface before the payload is posted.

use api::named_banks::BankAliasCandidate;
use api::snapshot::WorldTile;

/// A named cow field: the walk-in tile, not a gathering camp.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CowField {
    pub name: &'static str,
    pub x: i32,
    pub z: i32,
    pub level: i32,
}

/// Fire plot: bank stand plus a rectangular grass AABB.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FirePlot {
    pub name: &'static str,
    pub bank: WorldTile,
    pub x0: i32,
    pub x1: i32,
    pub z0: i32,
    pub z1: i32,
}

/// Cook stand: bank tile plus the range loc's stand tile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CookStand {
    pub name: &'static str,
    pub bank: WorldTile,
    pub range: WorldTile,
}

/// Lumbridge pen interior is the nav cow-pen tile. NW Lumbridge sits between
/// the Draynor and Lumbridge walk pins, north of the river road. South of
/// Falador is south of the Falador walk pin.
pub const COW_FIELDS: &[CowField] = &[
    CowField {
        name: "Lumbridge cow field",
        x: 3253,
        z: 3282,
        level: 0,
    },
    CowField {
        name: "North-west of Lumbridge",
        x: 3162,
        z: 3311,
        level: 0,
    },
    CowField {
        name: "South of Falador",
        x: 3029,
        z: 3305,
        level: 0,
    },
];

pub const AL_KHARID_BANK: WorldTile = WorldTile {
    x: 3269,
    z: 3167,
    level: 0,
};

pub fn cow_uses_al_kharid_toll(field: &CowField) -> bool {
    field.name == "Lumbridge cow field"
}

/// Frozen `Tile.distanceTo` nearest field: Chebyshev xz, different level
/// is `1_000_000 + xz`. Ties keep the earlier table row. Computed in `i64`
/// so any script-supplied tile, however far out of range, cannot overflow.
pub fn nearest_cow_field(from: WorldTile) -> Option<&'static CowField> {
    COW_FIELDS
        .iter()
        .min_by_key(|field| cow_field_distance(field, from))
}

fn cow_field_distance(field: &CowField, from: WorldTile) -> i64 {
    let dx = (i64::from(field.x) - i64::from(from.x)).abs();
    let dz = (i64::from(field.z) - i64::from(from.z)).abs();
    let xz = dx.max(dz);
    if field.level != from.level {
        1_000_000 + xz
    } else {
        xz
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RuneRoute {
    pub rune: &'static str,
    pub talisman: &'static str,
    pub level: i32,
    pub bank: &'static str,
    pub ruins: WorldTile,
}

pub const RUNE_ROUTES: &[RuneRoute] = &[
    RuneRoute {
        rune: "Air rune",
        talisman: "Air talisman",
        level: 1,
        bank: "Falador East",
        ruins: WorldTile {
            x: 2983,
            z: 3288,
            level: 0,
        },
    },
    RuneRoute {
        rune: "Mind rune",
        talisman: "Mind talisman",
        level: 2,
        bank: "Edgeville",
        ruins: WorldTile {
            x: 2980,
            z: 3511,
            level: 0,
        },
    },
    RuneRoute {
        rune: "Water rune",
        talisman: "Water talisman",
        level: 5,
        bank: "Draynor",
        ruins: WorldTile {
            x: 3182,
            z: 3162,
            level: 0,
        },
    },
    RuneRoute {
        rune: "Earth rune",
        talisman: "Earth talisman",
        level: 9,
        bank: "Varrock East",
        ruins: WorldTile {
            x: 3303,
            z: 3477,
            level: 0,
        },
    },
    RuneRoute {
        rune: "Fire rune",
        talisman: "Fire talisman",
        level: 14,
        bank: "Al Kharid",
        ruins: WorldTile {
            x: 3310,
            z: 3252,
            level: 0,
        },
    },
    RuneRoute {
        rune: "Body rune",
        talisman: "Body talisman",
        level: 20,
        bank: "Edgeville",
        ruins: WorldTile {
            x: 3050,
            z: 3442,
            level: 0,
        },
    },
];

pub const LOG_LEVELS: &[(&str, i32)] = &[
    ("Logs", 1),
    ("Oak logs", 15),
    ("Willow logs", 30),
    ("Maple logs", 45),
    ("Yew logs", 60),
    ("Magic logs", 75),
];

/// Bank tiles from walk pins / alcher stand. AABB is the plaza around the bank,
/// not a copied burn-lane search.
pub const FIRE_PLOTS: &[FirePlot] = &[
    FirePlot {
        name: "Varrock East",
        bank: WorldTile {
            x: 3253,
            z: 3420,
            level: 0,
        },
        x0: 3235,
        x1: 3275,
        z0: 3418,
        z1: 3432,
    },
    FirePlot {
        name: "Varrock West",
        bank: WorldTile {
            x: 3185,
            z: 3440,
            level: 0,
        },
        x0: 3170,
        x1: 3205,
        z0: 3426,
        z1: 3444,
    },
    FirePlot {
        name: "Draynor",
        bank: WorldTile {
            x: 3093,
            z: 3243,
            level: 0,
        },
        x0: 3078,
        x1: 3098,
        z0: 3240,
        z1: 3252,
    },
    FirePlot {
        name: "Seers",
        bank: WorldTile {
            x: 2725,
            z: 3491,
            level: 0,
        },
        x0: 2710,
        x1: 2735,
        z0: 3482,
        z1: 3494,
    },
];

/// Catherby range-house inside stand (nav_door DEST) and Catherby walk pin.
pub const COOK_STANDS: &[CookStand] = &[CookStand {
    name: "Catherby",
    bank: WorldTile {
        x: 2809,
        z: 3441,
        level: 0,
    },
    range: WorldTile {
        x: 2817,
        z: 3443,
        level: 0,
    },
}];

/// East Ardougne market pickpocket stands (Thiever / ArdyThiever Start).
/// Guard tile is the live-gold tele; other names share that plaza.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PickpocketSpot {
    pub name: &'static str,
    pub x: i32,
    pub z: i32,
    pub level: i32,
    pub leash: i32,
}

pub const PICKPOCKET_SPOTS: &[PickpocketSpot] = &[
    PickpocketSpot {
        name: "Guard",
        x: 2661,
        z: 3306,
        level: 0,
        leash: 19,
    },
    PickpocketSpot {
        name: "Knight of Ardougne",
        x: 2661,
        z: 3306,
        level: 0,
        leash: 29,
    },
    PickpocketSpot {
        name: "Paladin",
        x: 2661,
        z: 3306,
        level: 0,
        leash: 12,
    },
    PickpocketSpot {
        name: "Hero",
        x: 2661,
        z: 3306,
        level: 0,
        leash: 17,
    },
    PickpocketSpot {
        name: "Man",
        x: 3222,
        z: 3222,
        level: 0,
        leash: 19,
    },
    PickpocketSpot {
        name: "Woman",
        x: 3222,
        z: 3222,
        level: 0,
        leash: 19,
    },
];

pub fn pickpocket_spot(target: &str) -> Option<&'static PickpocketSpot> {
    PICKPOCKET_SPOTS
        .iter()
        .find(|spot| spot.name.eq_ignore_ascii_case(target))
        .or_else(|| {
            PICKPOCKET_SPOTS
                .iter()
                .find(|spot| spot.name.eq_ignore_ascii_case("guard"))
        })
        .or_else(|| PICKPOCKET_SPOTS.first())
}

/// Curated tile shortcut for the bank alias cluster tables below.
const fn t(x: i32, z: i32) -> WorldTile {
    WorldTile { x, z, level: 0 }
}

const FALADOR_EAST_BOOTHS: &[WorldTile] = &[
    t(3011, 3354),
    t(3012, 3354),
    t(3013, 3354),
    t(3014, 3354),
    t(3015, 3354),
];
const VARROCK_EAST_BOOTHS: &[WorldTile] =
    &[t(3252, 3419), t(3253, 3419), t(3254, 3419), t(3256, 3419)];
const EDGEVILLE_BOOTHS: &[WorldTile] = &[t(3095, 3491), t(3096, 3493)];
const DRAYNOR_BOOTHS: &[WorldTile] = &[t(3091, 3242), t(3091, 3243), t(3091, 3245)];
const AL_KHARID_BOOTHS: &[WorldTile] = &[
    t(3268, 3164),
    t(3268, 3165),
    t(3268, 3166),
    t(3268, 3167),
    t(3268, 3168),
    t(3268, 3169),
];

/// The five shim `RUNES.bank` catalog aliases: name, preferred walk stand,
/// and the packed booth loc tiles that identify that named cluster.
///
/// These are catalog preferences, not server world facts. `nav`'s resolver
/// projects them onto the bound world's packed booth set and walk surface —
/// a stand that fails adjacency or walkability is replaced by a derived
/// adjacent walkable tile, and an alias the world cannot support is omitted
/// so `BANK_LOCATIONS.find` stays undefined.
pub const BANK_ALIASES: &[BankAliasCandidate] = &[
    BankAliasCandidate {
        name: "Falador East",
        stand: t(3013, 3355),
        booths: FALADOR_EAST_BOOTHS,
    },
    BankAliasCandidate {
        name: "Varrock East",
        stand: t(3253, 3420),
        booths: VARROCK_EAST_BOOTHS,
    },
    BankAliasCandidate {
        name: "Edgeville",
        stand: t(3094, 3493),
        booths: EDGEVILLE_BOOTHS,
    },
    BankAliasCandidate {
        name: "Draynor",
        stand: t(3093, 3243),
        booths: DRAYNOR_BOOTHS,
    },
    BankAliasCandidate {
        name: "Al Kharid",
        stand: t(3269, 3167),
        booths: AL_KHARID_BOOTHS,
    },
];

/// Frozen Fight/Flee attacker names. Exact display-name match only.
pub const HOSTILE_ATTACKER_NAMES: &[&str] = &["Guard", "Knight of Ardougne", "Paladin", "Hero"];

/// Exact frozen `isHostileAttacker` facts: named hostile, in combat, not
/// targeting another player, within `max_distance`, and offering Attack.
pub fn is_hostile_attacker(
    name: Option<&str>,
    in_combat: bool,
    targets_another_player: bool,
    distance: i32,
    actions: &[&str],
    max_distance: i32,
) -> bool {
    let Some(name) = name else {
        return false;
    };
    HOSTILE_ATTACKER_NAMES.contains(&name)
        && in_combat
        && !targets_another_player
        && distance <= max_distance
        && actions.contains(&"Attack")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cow_fields_include_lumbridge_pen() {
        let lum = COW_FIELDS
            .iter()
            .find(|f| f.name == "Lumbridge cow field")
            .expect("lumbridge");
        assert_eq!((lum.x, lum.z, lum.level), (3253, 3282, 0));
    }

    #[test]
    fn nearest_cow_field_uses_tile_distance() {
        let falador_east = WorldTile {
            x: 3013,
            z: 3355,
            level: 0,
        };
        assert_eq!(
            nearest_cow_field(falador_east).map(|field| field.name),
            Some("South of Falador")
        );
        let lumbridge = WorldTile {
            x: 3253,
            z: 3282,
            level: 0,
        };
        assert_eq!(
            nearest_cow_field(lumbridge).map(|field| field.name),
            Some("Lumbridge cow field")
        );
        let north_west = WorldTile {
            x: 3162,
            z: 3311,
            level: 0,
        };
        assert_eq!(
            nearest_cow_field(north_west).map(|field| field.name),
            Some("North-west of Lumbridge")
        );
    }

    /// A script can pass any tile. Extreme coordinates must not overflow
    /// (which would panic inside the V8 callback and abort the process);
    /// they still resolve to a field by the frozen rule.
    #[test]
    fn nearest_cow_field_survives_extreme_tiles() {
        for from in [
            WorldTile {
                x: i32::MAX,
                z: 0,
                level: 1,
            },
            WorldTile {
                x: i32::MIN,
                z: i32::MIN,
                level: 0,
            },
        ] {
            assert!(nearest_cow_field(from).is_some(), "{from:?}");
        }
    }

    #[test]
    fn fire_plots_are_the_four_named_stands() {
        let names: Vec<_> = FIRE_PLOTS.iter().map(|p| p.name).collect();
        assert_eq!(names, ["Varrock East", "Varrock West", "Draynor", "Seers"]);
    }

    #[test]
    fn cook_stands_include_catherby_range_house() {
        assert_eq!(COOK_STANDS[0].name, "Catherby");
        assert_eq!(
            (COOK_STANDS[0].range.x, COOK_STANDS[0].range.z),
            (2817, 3443)
        );
    }

    #[test]
    fn pickpocket_guard_is_ardougne_market_gold_tile() {
        let guard = PICKPOCKET_SPOTS
            .iter()
            .find(|p| p.name == "Guard")
            .expect("Guard");
        assert_eq!((guard.x, guard.z, guard.level), (2661, 3306, 0));
    }

    #[test]
    fn bank_aliases_are_the_five_catalog_names_in_payload_order() {
        let names: Vec<_> = BANK_ALIASES.iter().map(|a| a.name).collect();
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
    }

    #[test]
    fn bank_aliases_name_a_preferred_stand_off_the_cluster_booth_locs() {
        let stands: Vec<_> = BANK_ALIASES
            .iter()
            .map(|a| (a.name, a.stand.x, a.stand.z, a.stand.level))
            .collect();
        assert_eq!(
            stands,
            [
                ("Falador East", 3013, 3355, 0),
                ("Varrock East", 3253, 3420, 0),
                ("Edgeville", 3094, 3493, 0),
                ("Draynor", 3093, 3243, 0),
                ("Al Kharid", 3269, 3167, 0),
            ]
        );
        for alias in BANK_ALIASES {
            assert!(
                !alias.booths.is_empty(),
                "{} needs the packed cluster booths it is identified by",
                alias.name
            );
            assert!(
                !alias.booths.contains(&alias.stand),
                "{} stand must be a walk tile, not one of its booth loc tiles",
                alias.name
            );
            assert!(
                alias.booths.iter().all(|b| b.level == alias.stand.level),
                "{} cluster booths must share the preferred stand's plane",
                alias.name
            );
        }
    }

    #[test]
    fn hostile_attacker_requires_every_frozen_fact() {
        let attack = ["Attack"];
        assert!(is_hostile_attacker(
            Some("Guard"),
            true,
            false,
            1,
            &attack,
            8
        ));
        assert!(is_hostile_attacker(
            Some("Knight of Ardougne"),
            true,
            false,
            8,
            &attack,
            8
        ));
        assert!(is_hostile_attacker(
            Some("Paladin"),
            true,
            false,
            0,
            &attack,
            8
        ));
        assert!(is_hostile_attacker(
            Some("Hero"),
            true,
            false,
            4,
            &attack,
            8
        ));
        assert!(!is_hostile_attacker(
            Some("guard"),
            true,
            false,
            1,
            &attack,
            8
        ));
        assert!(!is_hostile_attacker(
            Some("Man"),
            true,
            false,
            1,
            &attack,
            8
        ));
        assert!(!is_hostile_attacker(None, true, false, 1, &attack, 8));
        assert!(!is_hostile_attacker(
            Some("Guard"),
            false,
            false,
            1,
            &attack,
            8
        ));
        assert!(!is_hostile_attacker(
            Some("Guard"),
            true,
            true,
            1,
            &attack,
            8
        ));
        assert!(!is_hostile_attacker(
            Some("Guard"),
            true,
            false,
            9,
            &attack,
            8
        ));
        assert!(!is_hostile_attacker(
            Some("Guard"),
            true,
            false,
            1,
            &["Talk-to"],
            8
        ));
    }
}
