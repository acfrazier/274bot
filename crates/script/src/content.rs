//! Script-side curated site configuration and the frozen hostile-attacker
//! predicate.
//!
//! These tables are curated training/teleport preferences, not generated
//! revision facts: revision facts come from `api::game_data::SelectedGameData`
//! and the shared loot predicate stays in `api::content`. The rows are posted
//! through the existing content payload (`shim::content_json`) for the
//! catalog shims that read them. Bank identities and access facts instead come
//! from `api::named_banks` and the selected generated content.

use api::snapshot::WorldTile;

/// CowKiller's named bank route. The native bank catalog supplies the selected
/// tile when available; the recorded tile keeps load-only isolates complete.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CowBankDestination {
    pub name: &'static str,
    pub tile: WorldTile,
}

/// A named cow field, anchored at its verified spawn cluster.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CowField {
    pub name: &'static str,
    pub x: i32,
    pub z: i32,
    pub level: i32,
    pub uses_al_kharid_toll: bool,
    pub bank_destination: Option<CowBankDestination>,
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

/// Supported field anchors are centered on their map-backed cow spawn groups:
/// `m50_51.jm2:7562-7568`, `m49_51.jm2:6318-6332` and
/// `m49_52.jm2:8831-8867`, `m47_51.jm2:6945-6952`, and
/// `m41_52.jm2:7520-7529` (NPC id 81). The north-west anchor covers the
/// western group; the separate group farther east is outside that field.
pub const COW_FIELDS: &[CowField] = &[
    CowField {
        name: "Lumbridge cow field",
        x: 3255,
        z: 3288,
        level: 0,
        uses_al_kharid_toll: true,
        bank_destination: None,
    },
    CowField {
        name: "North-west of Lumbridge",
        x: 3168,
        z: 3329,
        level: 0,
        uses_al_kharid_toll: false,
        bank_destination: Some(CowBankDestination {
            name: "Draynor",
            tile: DRAYNOR_BANK,
        }),
    },
    CowField {
        name: "South of Falador",
        x: 3033,
        z: 3306,
        level: 0,
        uses_al_kharid_toll: false,
        bank_destination: Some(CowBankDestination {
            name: "Falador East",
            tile: FALADOR_EAST_BANK,
        }),
    },
    CowField {
        name: "East Ardougne cow field",
        x: 2664,
        z: 3347,
        level: 0,
        uses_al_kharid_toll: false,
        bank_destination: Some(CowBankDestination {
            name: "Ardougne West",
            tile: ARDOUGNE_WEST_BANK,
        }),
    },
];

pub const AL_KHARID_BANK: WorldTile = WorldTile {
    x: 3269,
    z: 3167,
    level: 0,
};

pub const DRAYNOR_BANK: WorldTile = WorldTile {
    x: 3093,
    z: 3243,
    level: 0,
};

pub const FALADOR_EAST_BANK: WorldTile = WorldTile {
    x: 3013,
    z: 3355,
    level: 0,
};

pub const ARDOUGNE_WEST_BANK: WorldTile = WorldTile {
    x: 2616,
    z: 3332,
    level: 0,
};

/// Two crossings at the 10-coin fare in
/// `content/scripts/areas/area_alkharid/scripts/border_gate.rs2:33,47-54`.
pub const COW_TOLL_COIN_TARGET: i32 = 20;

/// Frozen `shouldBootstrapTollCoins` API guard; this is a tile-distance
/// threshold, not a wait or retry interval.
pub const COW_TOLL_BOOTSTRAP_RADIUS: i32 = 80;

pub fn cow_uses_al_kharid_toll(field: &CowField) -> bool {
    field.uses_al_kharid_toll
}

pub fn cow_location_named(name: &str) -> Option<&'static CowField> {
    COW_FIELDS
        .iter()
        .find(|field| field.name.eq_ignore_ascii_case(name))
}

pub fn resolve_cow_field(setting: &str, start: Option<WorldTile>) -> Option<&'static CowField> {
    let setting = setting.trim();
    if setting.eq_ignore_ascii_case("start tile") {
        return None;
    }
    if setting.eq_ignore_ascii_case("auto") {
        return start.and_then(nearest_cow_field);
    }
    cow_location_named(setting)
}

/// Frozen `Tile.distanceTo`: Chebyshev xz, with different levels separated
/// by `1_000_000 + xz`. Compute in `i64` to avoid overflow for script tiles.
pub fn nearest_cow_field(from: WorldTile) -> Option<&'static CowField> {
    COW_FIELDS
        .iter()
        .min_by_key(|field| cow_field_distance(field, from))
}

fn cow_field_distance(field: &CowField, from: WorldTile) -> i64 {
    tile_distance(
        WorldTile {
            x: field.x,
            z: field.z,
            level: field.level,
        },
        from,
    )
}

pub fn is_cow_field_loot_tile(anchor: WorldTile, leash_radius: f64, tile: WorldTile) -> bool {
    anchor.level == tile.level && xz_distance(anchor, tile) as f64 <= leash_radius
}

pub fn needs_cow_toll_coins(uses_al_kharid_toll: bool, enabled: bool) -> bool {
    enabled && uses_al_kharid_toll
}

pub fn cow_bank_destination(
    uses_al_kharid_toll: bool,
    toll_enabled: bool,
    bank_destination: Option<CowBankDestination>,
) -> Option<CowBankDestination> {
    if needs_cow_toll_coins(uses_al_kharid_toll, toll_enabled) {
        Some(CowBankDestination {
            name: "Al Kharid",
            tile: AL_KHARID_BANK,
        })
    } else {
        bank_destination
    }
}

pub fn should_bootstrap_cow_toll_coins(
    uses_al_kharid_toll: bool,
    start: WorldTile,
    coins: f64,
    enabled: bool,
) -> bool {
    needs_cow_toll_coins(uses_al_kharid_toll, enabled)
        && coins < f64::from(COW_TOLL_COIN_TARGET)
        && tile_distance(AL_KHARID_BANK, start) <= i64::from(COW_TOLL_BOOTSTRAP_RADIUS)
}

fn xz_distance(left: WorldTile, right: WorldTile) -> i64 {
    let dx = (i64::from(left.x) - i64::from(right.x)).abs();
    let dz = (i64::from(left.z) - i64::from(right.z)).abs();
    dx.max(dz)
}

fn tile_distance(left: WorldTile, right: WorldTile) -> i64 {
    let xz = xz_distance(left, right);
    if left.level != right.level {
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
        bank: DRAYNOR_BANK,
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
        // 289 content: api/data/game-data/289.json:103317-103329 (maxrange 4),
        // 206121-206125 (spawn 2653,3315; 9 tiles from this anchor): leash = 9 + 4.
        leash: 13,
    },
    PickpocketSpot {
        name: "Hero",
        x: 2661,
        z: 3306,
        level: 0,
        // 289 content: api/data/game-data/289.json:103349-103357 (maxrange 7),
        // 206086-206090 (spawn 2647,3306; 14 tiles from the anchor) and
        // 206233-206237 (spawn 2667,3316; 10 tiles from the anchor).
        leash: 21,
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
    fn cow_fields_match_verified_spawn_anchors() {
        let anchors: Vec<_> = COW_FIELDS
            .iter()
            .map(|field| (field.name, field.x, field.z, field.level))
            .collect();
        assert_eq!(
            anchors,
            vec![
                ("Lumbridge cow field", 3255, 3288, 0),
                ("North-west of Lumbridge", 3168, 3329, 0),
                ("South of Falador", 3033, 3306, 0),
                ("East Ardougne cow field", 2664, 3347, 0),
            ]
        );
        assert_eq!(
            COW_FIELDS[1].bank_destination,
            Some(CowBankDestination {
                name: "Draynor",
                tile: DRAYNOR_BANK,
            })
        );
        assert_eq!(
            COW_FIELDS[2].bank_destination,
            Some(CowBankDestination {
                name: "Falador East",
                tile: FALADOR_EAST_BANK,
            })
        );
        assert_eq!(
            COW_FIELDS[3].bank_destination,
            Some(CowBankDestination {
                name: "Ardougne West",
                tile: ARDOUGNE_WEST_BANK,
            })
        );
    }

    #[test]
    fn resolve_cow_field_matches_names_and_auto_distance() {
        assert_eq!(
            resolve_cow_field(
                "south of falador",
                Some(WorldTile {
                    x: 0,
                    z: 0,
                    level: 0
                })
            )
            .map(|field| field.name),
            Some("South of Falador")
        );
        assert_eq!(resolve_cow_field("Start tile", Some(AL_KHARID_BANK)), None);
        assert_eq!(
            resolve_cow_field("Auto", Some(AL_KHARID_BANK)).map(|field| field.name),
            Some("Lumbridge cow field")
        );
        assert_eq!(resolve_cow_field("Auto", None), None);
        for (tile, expected) in [
            (
                WorldTile {
                    x: 3185,
                    z: 3440,
                    level: 0,
                },
                "North-west of Lumbridge",
            ),
            (
                WorldTile {
                    x: 3013,
                    z: 3355,
                    level: 0,
                },
                "South of Falador",
            ),
            (
                WorldTile {
                    x: 2616,
                    z: 3332,
                    level: 0,
                },
                "East Ardougne cow field",
            ),
        ] {
            assert_eq!(
                nearest_cow_field(tile).map(|field| field.name),
                Some(expected)
            );
        }
    }

    #[test]
    fn cow_loot_leash_uses_same_level_chebyshev_distance() {
        let lumbridge = WorldTile {
            x: COW_FIELDS[0].x,
            z: COW_FIELDS[0].z,
            level: COW_FIELDS[0].level,
        };
        assert!(!is_cow_field_loot_tile(
            lumbridge,
            18.0,
            WorldTile {
                x: 3233,
                z: 3298,
                level: 0,
            }
        ));
        assert!(is_cow_field_loot_tile(
            lumbridge,
            18.0,
            WorldTile {
                x: 3237,
                z: 3298,
                level: 0,
            }
        ));
        assert!(!is_cow_field_loot_tile(
            lumbridge,
            18.0,
            WorldTile {
                x: 3255,
                z: 3288,
                level: 1,
            }
        ));
    }

    #[test]
    fn cow_toll_helpers_follow_the_gate_fare_and_location_policy() {
        let lumbridge = &COW_FIELDS[0];
        let falador = &COW_FIELDS[2];
        assert!(needs_cow_toll_coins(lumbridge.uses_al_kharid_toll, true));
        assert!(!needs_cow_toll_coins(lumbridge.uses_al_kharid_toll, false));
        assert!(!needs_cow_toll_coins(falador.uses_al_kharid_toll, true));
        assert!(!needs_cow_toll_coins(false, true));
        assert_eq!(
            cow_bank_destination(lumbridge.uses_al_kharid_toll, true, None),
            Some(CowBankDestination {
                name: "Al Kharid",
                tile: AL_KHARID_BANK,
            })
        );
        assert_eq!(
            cow_bank_destination(lumbridge.uses_al_kharid_toll, false, None),
            None
        );
        assert_eq!(
            cow_bank_destination(
                COW_FIELDS[1].uses_al_kharid_toll,
                true,
                COW_FIELDS[1].bank_destination
            ),
            COW_FIELDS[1].bank_destination
        );
        assert_eq!(
            cow_bank_destination(
                COW_FIELDS[3].uses_al_kharid_toll,
                false,
                COW_FIELDS[3].bank_destination
            ),
            COW_FIELDS[3].bank_destination
        );
        assert!(should_bootstrap_cow_toll_coins(
            lumbridge.uses_al_kharid_toll,
            AL_KHARID_BANK,
            0.0,
            true
        ));
        assert!(!should_bootstrap_cow_toll_coins(
            lumbridge.uses_al_kharid_toll,
            AL_KHARID_BANK,
            f64::from(COW_TOLL_COIN_TARGET),
            true
        ));
        assert!(!should_bootstrap_cow_toll_coins(
            falador.uses_al_kharid_toll,
            AL_KHARID_BANK,
            0.0,
            true
        ));
        assert!(!should_bootstrap_cow_toll_coins(
            lumbridge.uses_al_kharid_toll,
            WorldTile {
                x: 3210,
                z: 3424,
                level: 0,
            },
            0.0,
            true
        ));
    }

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
    fn pickpocket_guard_is_ardougne_market_gold_tile() {
        let guard = PICKPOCKET_SPOTS
            .iter()
            .find(|p| p.name == "Guard")
            .expect("Guard");
        assert_eq!((guard.x, guard.z, guard.level), (2661, 3306, 0));
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
