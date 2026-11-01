//! Closed reachability views. Raw atlas storage does not escape this module.

use api::snapshot::WorldTile;
use thiserror::Error;

use super::{NavLease, PackId};
use crate::pack::unpack_tile_key;
use crate::reachability::{ReachabilityAtlas, StandRow};

/// Pack-bound movement component key. Fields stay private.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BoundComponentKey {
    pack: PackId,
    index: u32,
}

impl BoundComponentKey {
    /// Originating pack lease.
    pub const fn pack(self) -> PackId {
        self.pack
    }

    /// Base component index in that pack.
    pub const fn index(self) -> u32 {
        self.index
    }
}

/// Placed loc identity: game tile, loc id, shape, and angle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LocPlacementKey {
    tile: WorldTile,
    loc_id: i32,
    shape: u8,
    angle: u8,
}

impl LocPlacementKey {
    /// Construct a placement key. Angle must be 0..=3.
    pub fn new(
        tile: WorldTile,
        loc_id: i32,
        shape: u8,
        angle: u8,
    ) -> Result<Self, ReachabilityError> {
        if angle > 3 {
            return Err(ReachabilityError::UnsupportedGeometry {
                key: Self {
                    tile,
                    loc_id,
                    shape,
                    angle,
                },
            });
        }
        Ok(Self {
            tile,
            loc_id,
            shape,
            angle,
        })
    }

    pub const fn tile(self) -> WorldTile {
        self.tile
    }

    pub const fn loc_id(self) -> i32 {
        self.loc_id
    }

    pub const fn shape(self) -> u8 {
        self.shape
    }

    pub const fn angle(self) -> u8 {
        self.angle
    }
}

/// Borrowed stand rows for one placement. No allocation.
#[derive(Debug, Clone, Copy)]
pub struct LocStandView<'a> {
    pack: PackId,
    coverage: u8,
    stands: &'a [StandRow],
}

impl<'a> LocStandView<'a> {
    pub const fn coverage(self) -> u8 {
        self.coverage
    }

    pub const fn is_complete(self) -> bool {
        self.coverage == crate::reachability::PLACEMENT_COMPLETE
    }

    pub fn len(self) -> usize {
        self.stands.len()
    }

    pub fn is_empty(self) -> bool {
        self.stands.is_empty()
    }

    /// Iterate packed stand tiles without allocating.
    pub fn tiles(
        self,
    ) -> impl Iterator<Item = Result<(WorldTile, BoundComponentKey), ReachabilityError>> + 'a {
        let pack = self.pack;
        self.stands.iter().map(move |row| {
            let tile =
                unpack_tile_key(row.tile).map_err(|_| ReachabilityError::MissingPlacement {
                    key: LocPlacementKey {
                        tile: WorldTile {
                            x: 0,
                            z: 0,
                            level: 0,
                        },
                        loc_id: 0,
                        shape: 0,
                        angle: 0,
                    },
                })?;
            Ok((
                tile,
                BoundComponentKey {
                    pack,
                    index: row.base,
                },
            ))
        })
    }
}

/// Typed lookup failures. Missing coverage is never an empty stand set.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ReachabilityError {
    #[error("tile has no covered component: {tile:?}")]
    MissingComponent { tile: WorldTile },
    #[error("placement is not covered: {key:?}")]
    MissingPlacement { key: LocPlacementKey },
    #[error("unsupported interaction geometry: {key:?}")]
    UnsupportedGeometry { key: LocPlacementKey },
    #[error("reachability key belongs to another world")]
    IdentityMismatch { expected: PackId, actual: PackId },
}

/// Borrowed atlas view bound to one lease.
#[derive(Clone, Copy)]
pub struct ReachabilityView<'a> {
    pack: PackId,
    atlas: &'a ReachabilityAtlas,
}

impl<'a> ReachabilityView<'a> {
    /// Base component at `tile`, or a typed coverage miss.
    pub fn component_at(&self, tile: WorldTile) -> Result<BoundComponentKey, ReachabilityError> {
        match self.atlas.base_at(tile) {
            Ok(Some(index)) => Ok(BoundComponentKey {
                pack: self.pack,
                index,
            }),
            Ok(None) => Err(ReachabilityError::MissingComponent { tile }),
            Err(_) => Err(ReachabilityError::MissingComponent { tile }),
        }
    }

    /// Borrowed stands for a placed loc. Unsupported geometry stays typed.
    pub fn loc_stands(&self, key: LocPlacementKey) -> Result<LocStandView<'a>, ReachabilityError> {
        let Some(row) = self
            .atlas
            .placement(key.tile, key.loc_id, key.shape, key.angle)
        else {
            return Err(ReachabilityError::MissingPlacement { key });
        };
        if row.coverage == crate::reachability::PLACEMENT_UNSUPPORTED_GEOMETRY {
            return Err(ReachabilityError::UnsupportedGeometry { key });
        }
        let stands = self
            .atlas
            .stands_for(row)
            .map_err(|_| ReachabilityError::MissingPlacement { key })?;
        Ok(LocStandView {
            pack: self.pack,
            coverage: row.coverage,
            stands,
        })
    }

    pub fn bind_component(
        &self,
        key: BoundComponentKey,
    ) -> Result<BoundComponentKey, ReachabilityError> {
        if key.pack != self.pack {
            return Err(ReachabilityError::IdentityMismatch {
                expected: self.pack,
                actual: key.pack,
            });
        }
        Ok(key)
    }
}

impl NavLease {
    /// Borrowed reachability atlas for this pack.
    pub fn reachability(&self) -> ReachabilityView<'_> {
        ReachabilityView {
            pack: self.identity.pack(),
            atlas: self.world.reachability(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collision::{pack_walk, WorldCollision};
    use crate::contract::PackDigest;
    use crate::reachability::ReachabilityAtlas;
    use crate::world::NavWorld;
    use std::sync::Arc;

    fn tiny_world() -> Arc<NavWorld> {
        let (walk, blocked) = pack_walk(&[0; 8]);
        let collision = WorldCollision {
            origin: WorldTile {
                x: 3200,
                z: 3200,
                level: 0,
            },
            width: 2,
            height: 1,
            walk,
            blocked,
            flags: None,
        };
        let atlas = ReachabilityAtlas::unlabeled(&collision).unwrap();
        Arc::new(NavWorld::from_decoded(
            collision,
            crate::transport::TransportGraph::default(),
            Vec::new(),
            atlas,
        ))
    }

    #[test]
    fn unlabeled_component_lookup_is_missing_not_empty() {
        let world = tiny_world();
        let lease = NavLease::wrap(world, PackDigest::of(b"test")).unwrap();
        let view = lease.reachability();
        let err = view
            .component_at(WorldTile {
                x: 3200,
                z: 3200,
                level: 0,
            })
            .unwrap_err();
        assert!(matches!(err, ReachabilityError::MissingComponent { .. }));
    }

    fn world_with_one_stand() -> (Arc<NavWorld>, LocPlacementKey, WorldTile) {
        let (walk, blocked) = pack_walk(&[0; 8]);
        let collision = WorldCollision {
            origin: WorldTile {
                x: 3200,
                z: 3200,
                level: 0,
            },
            width: 2,
            height: 1,
            walk,
            blocked,
            flags: None,
        };
        let tile = WorldTile {
            x: 3200,
            z: 3200,
            level: 0,
        };
        let origin = crate::pack::pack_tile_key(tile).unwrap();
        let mut atlas = ReachabilityAtlas::unlabeled(&collision).unwrap();
        atlas.placements = Box::from([crate::reachability::PlacementRow {
            origin,
            loc_id: 42,
            shape: 0,
            angle: 0,
            coverage: crate::reachability::PLACEMENT_COMPLETE,
            trigger: crate::reachability::TRIGGER_OP,
            stand_start: 0,
            stand_count: 1,
            detail: 0,
        }]);
        atlas.stands = Box::from([crate::reachability::StandRow {
            tile: origin,
            base: 0,
        }]);
        let world = Arc::new(NavWorld::from_decoded(
            collision,
            crate::transport::TransportGraph::default(),
            Vec::new(),
            atlas,
        ));
        let key = LocPlacementKey::new(tile, 42, 0, 0).unwrap();
        (world, key, tile)
    }

    #[test]
    fn hundred_bound_leases_share_one_atlas_payload() {
        let world = tiny_world();
        let digest = PackDigest::of(b"share-atlas");
        let leases: Vec<_> = (0..100)
            .map(|_| NavLease::wrap(Arc::clone(&world), digest).unwrap())
            .collect();
        let first = leases[0].reachability();
        for lease in &leases {
            let view = lease.reachability();
            assert!(
                std::ptr::eq(view.atlas, first.atlas),
                "each bound lease must borrow the same atlas payload"
            );
        }
    }

    #[test]
    fn borrowed_component_and_stand_lookups_do_not_allocate() {
        let (world, key, tile) = world_with_one_stand();
        let lease = NavLease::wrap(world, PackDigest::of(b"zero-alloc")).unwrap();
        let view = lease.reachability();
        let measured = allocation_counter::measure(|| {
            let component = view.component_at(tile);
            let stands = view.loc_stands(key).expect("fixture placement is covered");
            assert_eq!(stands.len(), 1);
            let _ = std::hint::black_box((component, stands));
        });
        assert_eq!(
            measured.count_total, 0,
            "borrowed component and stand lookups must not allocate: {measured:?}"
        );
    }
}
