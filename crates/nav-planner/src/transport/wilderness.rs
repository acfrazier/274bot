use api::snapshot::WorldTile;

use super::{TransportEdge, TransportGraph};

/// Inclusive AABB of one `wilderness_zones` coord pair, plus the script's
/// `$coord1` south-edge z used as the level origin.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WildernessZone {
    pub x1: i32,
    pub z1: i32,
    pub x2: i32,
    pub z2: i32,
    pub level1: i32,
    pub level2: i32,
    /// `coordz($coord1)` — the first listed pair coord, the formula origin.
    pub origin_z: i32,
}

impl WildernessZone {
    pub fn contains(self, t: WorldTile) -> bool {
        t.x >= self.x1
            && t.x <= self.x2
            && t.z >= self.z1
            && t.z <= self.z2
            && t.level >= self.level1
            && t.level <= self.level2
    }
}

/// Packed wilderness-level formula: zones plus `(z − origin_z) / divisor + offset`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WildernessRules {
    pub zones: Vec<WildernessZone>,
    pub divisor: i32,
    pub offset: i32,
}

impl WildernessRules {
    /// Content `~wilderness_level(coord)`: 0 outside every zone.
    pub fn level(&self, t: WorldTile) -> i32 {
        if self.divisor <= 0 {
            return 0;
        }
        for z in &self.zones {
            if z.contains(t) {
                return (t.z - z.origin_z) / self.divisor + self.offset;
            }
        }
        0
    }

    /// Inclusive `inzone` membership against packed `wilderness_zones`.
    /// A reversed bound in the listed pair is an empty zone, matching the
    /// engine (`x1..x2` / `level1..level2` as written, not min/max).
    pub fn contains(&self, t: WorldTile) -> bool {
        self.zones.iter().any(|z| z.contains(t))
    }
}

impl TransportGraph {
    /// Whether `edge` may be taken from `from` under the packed wilderness cap.
    /// Non-teleports and teleports with no derived cap stay unrestricted.
    pub fn teleport_legal_from(&self, from: WorldTile, edge: &TransportEdge) -> bool {
        Self::teleport_legal_at_level(self.wilderness.level(from), edge)
    }

    /// [`Self::teleport_legal_from`] with a precomputed wilderness level.
    pub fn teleport_legal_at_level(level: i32, edge: &TransportEdge) -> bool {
        match edge.wildy_cap {
            None => true,
            Some(cap) => level <= cap,
        }
    }
}
