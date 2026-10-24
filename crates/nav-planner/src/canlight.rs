//! Static `area_allow_loc_add` subset baked as the `274L` canlight sidecar.
//!
//! The bit means the immutable content would allow a loc add: the tile is
//! on a loaded mapsquare, outside every `bank_zones` AABB, has no baked
//! engine FLOOR / FLOOR_DECORATION / LOC analogue (`WR_GRND` /
//! `WALK_SCENERY`), and is not covered by a static **active** map-loc
//! footprint (MAP_LOCADDUNSAFE rotation). It is not walkable, not a
//! handwritten bank box, and not a complete live predicate. Live loc
//! add/del/morph, open doors, F2P members tiles, and multi-tile live
//! footprints stay residual. Static map locs are a conservative baseline:
//! a later removal can leave a tile excluded.

use api::snapshot::WorldTile;
use sha2::{Digest, Sha256};

use crate::collision::WorldCollision;

/// Manual policy identity. Bump when the static mask rules change in a way
/// source bytes in [`crate::bake::GENERATOR_SOURCES`] should not have to
/// carry alone (the sidecar header binding includes this id).
pub const POLICY_ID: &str = "canlight-1";
/// Algorithm label mixed into the policy digest. Describes the static
/// `area_allow_loc_add` subset: bank AABBs, WR_GRND, WALK_SCENERY, static
/// active loc footprints, mapsquare coverage on every plane.
pub const ALGORITHM_ID: &str = "static-area-allow-loc-add-v1";
/// Content-root-relative bank_zones dbrow both revisions use.
pub const BANK_ZONES_REL: &str = "scripts/skill_firemaking/configs/bank_zones.dbrow";

const LEVELS: i32 = 4;


/// Policy digest mixed into the canlight identity: policy id, algorithm id,
/// revision, and the canonical `bank_zones.dbrow` bytes. Pack SHA and mask
/// bytes are **not** included here; [`identity_digest`] adds the baked mask
/// and [`header_binding`] concatenates the pack identity.
pub fn policy_digest(revision: u16, bank_zones_bytes: &[u8]) -> [u8; 32] {
    let mut digest = Sha256::new();
    digest.update(POLICY_ID.as_bytes());
    digest.update([0]);
    digest.update(ALGORITHM_ID.as_bytes());
    digest.update([0]);
    digest.update(revision.to_le_bytes());
    digest.update([0]);
    digest.update(bank_zones_bytes);
    digest.finalize().into()
}

/// Identity of the actual baked canonical bitmap. The words are serialized
/// explicitly as little-endian u64 values so the identity is independent of
/// host endianness and cannot collide for equal policy inputs with different
/// masks. Runtime validation uses this small digest from the identity row; it
/// never hashes or copies the resident mask.
pub fn identity_digest(revision: u16, bank_zones_bytes: &[u8], bits: &[u64]) -> [u8; 32] {
    let mut digest = Sha256::new();
    digest.update(b"canlight-mask-identity-v1");
    digest.update([0]);
    digest.update(policy_digest(revision, bank_zones_bytes));
    digest.update((bits.len() as u64).to_le_bytes());
    for word in bits {
        digest.update(word.to_le_bytes());
    }
    digest.finalize().into()
}

/// 32-byte sidecar header binding: pack digest then canlight identity digest.
/// Cheap to recompute at bundled load from identity-row hex; does not hash the mask.
pub fn header_binding(pack_digest: &[u8; 32], policy: &[u8; 32]) -> [u8; 32] {
    let mut digest = Sha256::new();
    digest.update(pack_digest);
    digest.update(policy);
    digest.finalize().into()
}

/// Recompute the expected header binding from identity-row hex. Fails closed
/// on a malformed digest rather than treating the sidecar as unbound.
pub fn expected_header_binding(
    nav_sha256_hex: &str,
    canlight_identity_hex: &str,
) -> Result<[u8; 32], String> {
    let pack =
        crate::pack::sha256_from_hex(nav_sha256_hex).map_err(|e| format!("pack identity {e}"))?;
    let policy = crate::pack::sha256_from_hex(canlight_identity_hex)
        .map_err(|e| format!("canlight identity {e}"))?;
    Ok(header_binding(&pack, &policy))
}


/// Walk-buffer index of `tile`, `None` outside the grid or on unknown levels.
pub fn canlight_cell_index(collision: &WorldCollision, tile: WorldTile) -> Option<usize> {
    if !(0..LEVELS).contains(&tile.level) {
        return None;
    }
    let lx = tile.x - collision.origin.x;
    let lz = tile.z - collision.origin.z;
    if lx < 0 || lz < 0 {
        return None;
    }
    let (lx, lz) = (lx as usize, lz as usize);
    if lx >= collision.width || lz >= collision.height {
        return None;
    }
    Some(tile.level as usize * collision.width * collision.height + lz * collision.width + lx)
}

/// Whether `tile` is set in a canlight bitset.
pub fn canlight_at(bits: &[u64], collision: &WorldCollision, tile: WorldTile) -> bool {
    let Some(idx) = canlight_cell_index(collision, tile) else {
        return false;
    };
    bits.get(idx / 64)
        .is_some_and(|word| word & (1 << (idx % 64)) != 0)
}


#[cfg(test)]
#[path = "canlight_tests.rs"]
mod tests;
