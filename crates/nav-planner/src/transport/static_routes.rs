use api::snapshot::WorldTile;

use super::{TransportEdge, TransportKind};

#[derive(Debug, Clone, Copy)]
struct CartFareRoute {
    npc: i32,
    at: WorldTile,
    to: WorldTile,
    fare: (i32, i32),
}

const CART_FARE_ROUTES: &[CartFareRoute] = &[
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
    CART_FARE_ROUTES
        .iter()
        .any(|route| {
            edge.kind == TransportKind::Npc
                && edge.loc_id == route.npc
                && edge.at == route.at
                && edge.to == route.to
                && route.fare.0 == id
        })
        .then(|| ((i64::from(carried.max(0)) * 5) / 100).clamp(10, 200) as i32)
}
