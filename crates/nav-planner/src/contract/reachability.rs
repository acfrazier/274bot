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
        let lease = NavLease::wrap(world).unwrap();
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

    /// A labelled atlas on a 2x1 grid: tile (3200,3200) is component 0 in a
    /// two-entry palette chunk (the indexed path), tile (3201,3200) has label
    /// 0 (uncovered), and one placement has one stand.
    fn labelled_world() -> (Arc<NavWorld>, LocPlacementKey, WorldTile, WorldTile) {
        use crate::reachability::{ClosureRun, PlacementRow, StandRow};
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
        let uncovered = WorldTile {
            x: 3201,
            z: 3200,
            level: 0,
        };
        let origin = crate::pack::pack_tile_key(tile).unwrap();
        let mut atlas = ReachabilityAtlas::unlabeled(&collision).unwrap();
        atlas.palette = Box::from([0, 1]);
        atlas.chunks[0] = crate::reachability::Chunk {
            palette_start: 0,
            palette_count: 2,
            index_start: 0,
            index_len: 512,
            bit_width: 1,
        };
        let mut indexes = vec![0u8; 512];
        indexes[0] = 1;
        atlas.indexes = indexes.into_boxed_slice();
        atlas.base_to_condensed = Box::from([0]);
        atlas.base_sizes = Box::from([1]);
        atlas.movement_offsets = Box::from([0, 1]);
        atlas.movement_runs = Box::from([ClosureRun { first: 0, count: 1 }]);
        atlas.transport_offsets = Box::from([0, 1]);
        atlas.transport_runs = Box::from([ClosureRun { first: 0, count: 1 }]);
        atlas.conditional_offsets = Box::from([0, 0]);
        atlas.placements = Box::from([PlacementRow {
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
        atlas.stands = Box::from([StandRow {
            tile: origin,
            base: 0,
        }]);
        let graph = crate::transport::TransportGraph::default();
        atlas
            .validate(&collision, &graph)
            .expect("the labelled fixture satisfies the atlas invariants");
        let world = Arc::new(NavWorld::from_decoded(collision, graph, Vec::new(), atlas));
        let key = LocPlacementKey::new(tile, 42, 0, 0).unwrap();
        (world, key, tile, uncovered)
    }

    #[test]
    fn labelled_component_lookup_hits_and_misses_by_label() {
        let (world, _, tile, uncovered) = labelled_world();
        let lease = NavLease::wrap(world).unwrap();
        let view = lease.reachability();
        assert!(view.component_at(tile).is_ok());
        assert_eq!(
            view.component_at(uncovered).unwrap_err(),
            ReachabilityError::MissingComponent { tile: uncovered }
        );
    }

    /// The lease holds the shared world handle and nothing of the atlas: a
    /// hundred leases add a hundred handles, not a hundred payloads.
    #[test]
    fn hundred_leases_share_one_atlas_payload() {
        let (world, _, _, _) = labelled_world();
        let leases: Vec<_> = (0..100)
            .map(|_| NavLease::wrap(Arc::clone(&world)).unwrap())
            .collect();
        assert_eq!(Arc::strong_count(&world), 101);
        let first = leases[0].reachability();
        for lease in &leases {
            let view = lease.reachability();
            assert!(
                std::ptr::eq(view.atlas, first.atlas),
                "each bound lease must borrow the same atlas payload"
            );
            assert!(std::ptr::eq(view.atlas, world.reachability()));
        }
    }

    /// Per-bot tripwires: the lease stays a handle plus identity, and the
    /// borrowed view is two words. A field that copies atlas data grows them.
    #[test]
    fn lease_and_view_sizes_stay_per_bot_constant() {
        assert_eq!(std::mem::size_of::<ReachabilityView<'_>>(), 16);
        assert!(
            std::mem::size_of::<NavLease>() <= 128,
            "NavLease is {} bytes",
            std::mem::size_of::<NavLease>()
        );
    }

    #[test]
    fn borrowed_component_and_stand_lookups_do_not_allocate() {
        let (world, key, tile, uncovered) = labelled_world();
        let lease = NavLease::wrap(world).unwrap();
        let view = lease.reachability();
        let measured = allocation_counter::measure(|| {
            let component = view.component_at(tile);
            let miss = view.component_at(uncovered);
            let stands = view.loc_stands(key).expect("fixture placement is covered");
            assert_eq!(stands.len(), 1);
            assert!(component.is_ok() && miss.is_err());
            let _ = std::hint::black_box((component, miss, stands));
        });
        assert_eq!(
            measured.count_total, 0,
            "borrowed component and stand lookups must not allocate: {measured:?}"
        );
    }

    /// Upper bound on the real 289 atlas tables in memory. The v18 289 tables
    /// hold about 6.20 MB (the pack grew 6,216,880 bytes over v17, minus the
    /// multiway keys and section directories). Growth past this limit is a
    /// new once-per-process cost to review, not a per-bot cost.
    const REAL_ATLAS_RESIDENT_LIMIT: usize = 6_300_000;

    #[test]
    fn real_289_atlas_stays_inside_its_size_tripwire_and_lookups_do_not_allocate() {
        let Some(world) = NavWorld::load_default_pack_or_skip() else {
            return;
        };
        let bytes = world.reachability().resident_bytes();
        eprintln!("real 289 atlas tables hold {bytes} bytes");
        let started = std::time::Instant::now();
        world
            .reachability()
            .validate(&world.collision, &world.graph)
            .expect("the decoded real atlas validates again");
        eprintln!("real 289 atlas validation took {:?}", started.elapsed());
        assert!(
            bytes > 1_000_000 && bytes <= REAL_ATLAS_RESIDENT_LIMIT,
            "atlas tables hold {bytes} bytes"
        );
        let world = Arc::new(world);
        // The host binds a bot with `producer::lease(&world)`. A hundred of
        // them add a hundred world handles and no atlas copy.
        let leases: Vec<_> = (0..100)
            .map(|_| crate::producer::lease(&world).expect("the real pack leases"))
            .collect();
        assert_eq!(Arc::strong_count(&world), 101);
        assert!(leases
            .iter()
            .all(|lease| std::ptr::eq(lease.reachability().atlas, world.reachability())));
        let lease = NavLease::wrap(Arc::clone(&world)).unwrap();
        let view = lease.reachability();
        let lumbridge = WorldTile {
            x: 3222,
            z: 3218,
            level: 0,
        };
        let row = *world
            .reachability()
            .placements
            .iter()
            .find(|row| {
                row.coverage == crate::reachability::PLACEMENT_COMPLETE && row.stand_count > 0
            })
            .expect("the real pack has a complete placement with stands");
        let origin = crate::pack::unpack_tile_key(row.origin).unwrap();
        let key = LocPlacementKey::new(origin, row.loc_id, row.shape, row.angle).unwrap();
        let measured = allocation_counter::measure(|| {
            let component = view.component_at(lumbridge);
            let stands = view
                .loc_stands(key)
                .expect("the chosen placement is covered");
            assert!(component.is_ok());
            assert!(!stands.is_empty());
            let _ = std::hint::black_box((component, stands));
        });
        assert_eq!(measured.count_total, 0, "{measured:?}");
    }
}
