// Host-owned 274 stands. JS readers in the script shim copy these names/tiles.
use crate::snapshot::WorldTile;

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

pub const ROCK_TYPE_NAMES: &[&str] = &[
    "Clay",
    "Copper",
    "Tin",
    "Iron",
    "Silver",
    "Coal",
    "Gold",
    "Mithril",
    "Adamantite",
    "Runite",
];

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
    fn fire_plots_are_the_four_named_stands() {
        let names: Vec<_> = FIRE_PLOTS.iter().map(|p| p.name).collect();
        assert_eq!(
            names,
            ["Varrock East", "Varrock West", "Draynor", "Seers"]
        );
    }

    #[test]
    fn cook_stands_include_catherby_range_house() {
        assert_eq!(COOK_STANDS[0].name, "Catherby");
        assert_eq!(
            (COOK_STANDS[0].range.x, COOK_STANDS[0].range.z),
            (2817, 3443)
        );
    }
}
