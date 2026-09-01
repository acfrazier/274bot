//! Shared walk destinations: the 12 rs2b0t `WALK_DESTINATIONS` town pins
//! the TUI map draws (the headed picker may adopt them later). Each entry
//! is the world tile a WalkTo confirm arms.

/// One named walk destination pin: the town's world tile.
pub struct WalkDestination {
    pub name: &'static str,
    pub x: i32,
    pub z: i32,
    pub level: i32,
}

/// The 12 rs2b0t `WALK_DESTINATIONS` town pins (world tiles), in table
/// order: Lumbridge first, then the named towns.
pub static WALK_DESTINATIONS: &[WalkDestination] = &[
    WalkDestination {
        name: "Lumbridge",
        x: 3221,
        z: 3218,
        level: 0,
    },
    WalkDestination {
        name: "Varrock",
        x: 3213,
        z: 3424,
        level: 0,
    },
    WalkDestination {
        name: "Falador",
        x: 2965,
        z: 3378,
        level: 0,
    },
    WalkDestination {
        name: "Ardougne",
        x: 2661,
        z: 3301,
        level: 0,
    },
    WalkDestination {
        name: "Rellekka",
        x: 2668,
        z: 3660,
        level: 0,
    },
    WalkDestination {
        name: "Taverley",
        x: 2895,
        z: 3435,
        level: 0,
    },
    WalkDestination {
        name: "Draynor",
        x: 3093,
        z: 3243,
        level: 0,
    },
    WalkDestination {
        name: "Al Kharid",
        x: 3269,
        z: 3167,
        level: 0,
    },
    WalkDestination {
        name: "Edgeville",
        x: 3094,
        z: 3493,
        level: 0,
    },
    WalkDestination {
        name: "Seers' Village",
        x: 2725,
        z: 3491,
        level: 0,
    },
    WalkDestination {
        name: "Catherby",
        x: 2809,
        z: 3441,
        level: 0,
    },
    WalkDestination {
        name: "Yanille",
        x: 2612,
        z: 3092,
        level: 0,
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn walk_destinations_are_the_12_rs2b0t_towns() {
        assert_eq!(WALK_DESTINATIONS.len(), 12);
        let lumbridge = &WALK_DESTINATIONS[0];
        assert_eq!(lumbridge.name, "Lumbridge");
        assert_eq!((lumbridge.x, lumbridge.z, lumbridge.level), (3221, 3218, 0));
    }
}
