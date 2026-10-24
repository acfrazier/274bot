use api::snapshot::WorldTile;

use super::{TransportEdge, TransportKind};

/// Immutable identity and minimum-fare facts for one Shilo cart journey.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CartFareRoute {
    npc: i32,
    at: WorldTile,
    to: WorldTile,
    fare: (i32, i32),
}

impl CartFareRoute {
    /// The NPC id of the driver who starts this cart journey.
    pub const fn npc_id(&self) -> i32 {
        self.npc
    }

    /// The cart driver's interaction tile.
    pub const fn origin(&self) -> WorldTile {
        self.at
    }

    /// The cart journey's destination tile.
    pub const fn destination(&self) -> WorldTile {
        self.to
    }

    /// The minimum `(item id, count)` fare packed on this journey.
    pub const fn minimum_fare(&self) -> (i32, i32) {
        self.fare
    }
}

/// Canonical Shilo cart identity and minimum-fare facts.
/// Driver ids and origins come from `pack/npc.pack` and `content/maps/*.jm2`.
/// Destinations come from `area_brimhaven/scripts/hajedy.rs2` and
/// `area_shilo/scripts/vigroy.rs2`. Both use `calc_shilocart_cost`:
/// `(coins carried * 5) / 100`, clamped to 10–200 coins.
pub const SHILO_CART_ROUTES: &[CartFareRoute] = &[
    CartFareRoute {
        npc: 510,
        at: WorldTile {
            x: 2779,
            z: 3211,
            level: 0,
        },
        to: WorldTile {
            x: 2834,
            z: 2951,
            level: 0,
        },
        fare: (995, 10),
    },
    CartFareRoute {
        npc: 511,
        at: WorldTile {
            x: 2834,
            z: 2954,
            level: 0,
        },
        to: WorldTile {
            x: 2776,
            z: 3214,
            level: 0,
        },
        fare: (995, 10),
    },
];

/// The live content's `calc_shilocart_cost`, for these exact route-table
/// edges only. A static packed minimum must not become a fixed debit.
pub(super) fn cart_fare(edge: &TransportEdge, id: i32, carried: i32) -> Option<i32> {
    if edge.kind != TransportKind::Npc {
        return None;
    }
    SHILO_CART_ROUTES
        .iter()
        .any(|route| {
            edge.loc_id == route.npc
                && edge.at == route.at
                && edge.to == route.to
                && route.fare.0 == id
        })
        .then(|| ((i64::from(carried.max(0)) * 5) / 100).clamp(10, 200) as i32)
}
