//! Borrowed candidates for a nearest-service search over account-eligible rows.

use crate::game_data::SelectedGameData;
use crate::shop_facts::{self, KeeperFact};
use crate::snapshot::WorldTile;

/// The source of a candidate row. Search treats every kind the same way.
///
/// Bank rows come from the selected bank placements; shop keeper rows come
/// from the pinned `data/shops/keepers.npc` and selected NPC placements.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceKind {
    /// A selected bank stand from the bound named-bank facts.
    Bank,
    /// A pinned shop keeper's selected NPC placement.
    ShopKeeper,
}

/// A borrowed view of a retained row whose access tiles nav has validated.
#[derive(Debug, Clone, Copy)]
pub struct ServiceCandidate<'a> {
    /// The retained facts that own this row.
    pub kind: ServiceKind,
    /// The row index in its owner's shared facts.
    pub index: u16,
    /// The footprint's south-west corner, including its level.
    pub tile: WorldTile,
    /// Rotated east-west extent; NPC keeper placements use one tile.
    pub width: u8,
    /// Rotated north-south extent; NPC keeper placements use one tile.
    pub length: u8,
    /// Standable, permitted neighbours validated against the bound nav world.
    pub access: &'a [WorldTile],
}

/// A selected row and the standable tile from which it can be used.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ServicePick {
    /// The selected row in the candidate owner's shared facts.
    pub index: u16,
    /// The selected standable access tile, never the service's footprint.
    pub access_tile: WorldTile,
}

/// A pinned keeper placement with its nav-validated one-tile access ring.
///
/// Keeper identity is borrowed from `data/shops/keepers.npc`; placement is
/// selected-content `npc_placements`. A one-tile footprint has four neighbours,
/// so the access ring is retained inline, not in a per-request allocation.
#[derive(Debug)]
pub struct ResolvedShopKeeper {
    /// The row index in this world's [`ShopKeeperFacts`].
    pub index: u16,
    /// The pinned config/`owned_shop` join, not a display-name match.
    pub keeper: KeeperFact<'static>,
    /// This occurrence of the keeper in selected `npc_placements`.
    pub tile: WorldTile,
    access: [WorldTile; 4],
    access_len: u8,
}

impl ResolvedShopKeeper {
    /// Retain nav's at most four standable neighbours for a pinned NPC placement.
    ///
    /// `index` is this row's position in its owning facts. The supplied access
    /// must already satisfy the one-tile shared-edge rule; this value does not
    /// consult collision or add permission to a selected-content keeper.
    pub fn new(
        index: u16,
        keeper: KeeperFact<'static>,
        tile: WorldTile,
        access: impl IntoIterator<Item = WorldTile>,
    ) -> Self {
        let mut row = Self {
            index,
            keeper,
            tile,
            access: [WorldTile {
                x: 0,
                z: 0,
                level: 0,
            }; 4],
            access_len: 0,
        };
        for stand in access {
            row.access[usize::from(row.access_len)] = stand;
            row.access_len += 1;
        }
        row
    }

    /// The standable neighbours nav admitted for this selected placement.
    pub fn access(&self) -> &[WorldTile] {
        &self.access[..usize::from(self.access_len)]
    }

    /// Borrow this selected placement and its retained access ring for search.
    pub fn candidate(&self) -> ServiceCandidate<'_> {
        ServiceCandidate {
            kind: ServiceKind::ShopKeeper,
            index: self.index,
            tile: self.tile,
            width: 1,
            length: 1,
            access: self.access(),
        }
    }
}

/// Resolved pinned shop placements, bound once per revision-specific nav world.
///
/// The generated shop records join keeper configs to the selected NPC type and
/// every selected `npc_placements` occurrence. All bots borrow this owner's
/// rows; a pick neither copies the rows nor rebuilds their access rings.
#[derive(Debug, Default)]
pub struct ShopKeeperFacts {
    rows: Box<[ResolvedShopKeeper]>,
}

impl ShopKeeperFacts {
    /// Take ownership of nav-resolved pinned placement rows without copying.
    ///
    /// Each row's `index` must be its position in this selected-content owner.
    pub fn from_rows(rows: Box<[ResolvedShopKeeper]>) -> Self {
        Self { rows }
    }

    /// Bind the pinned keeper placements using nav's shared-edge access filter.
    ///
    /// The callback must yield the at most four standable neighbours of a
    /// one-tile, all-sides-open footprint, excluding the footprint itself and
    /// walls on the shared edge. The pin bounds the row count to a `u16` index.
    /// This is world assembly, not a bot or per-request operation.
    pub fn resolve<I>(data: &SelectedGameData, mut access: impl FnMut(WorldTile) -> I) -> Self
    where
        I: IntoIterator<Item = WorldTile>,
    {
        let placements = data
            .npc_placements()
            .map_or(&[][..], |facts| facts.rows.as_slice());
        let source = || {
            shop_facts::keepers(data)
                .into_iter()
                .flatten()
                .flat_map(move |keeper| {
                    placements
                        .iter()
                        .filter(move |placement| placement.npc_id == keeper.keeper_id)
                        .map(move |placement| {
                            (
                                keeper,
                                WorldTile {
                                    x: placement.x,
                                    z: placement.z,
                                    level: placement.plane,
                                },
                            )
                        })
                })
        };
        let count = source().count();
        assert!(count <= usize::from(u16::MAX), "pinned keeper row domain");
        let mut source = source();
        let rows = (0..count)
            .map(|index| {
                let (keeper, tile) = source.next().expect("counted keeper placement");
                ResolvedShopKeeper::new(
                    u16::try_from(index).expect("checked keeper row domain"),
                    keeper,
                    tile,
                    access(tile),
                )
            })
            .collect();
        Self::from_rows(rows)
    }

    /// Every selected placement of every joined pinned keeper, in content order.
    pub fn rows(&self) -> &[ResolvedShopKeeper] {
        &self.rows
    }

    /// Borrow rows stocking this selected item and permitted on this world.
    ///
    /// Stock and membership come from the generated shop records joined to the
    /// selected item rows. Empty nav access excludes a placement. Account-area
    /// gates remain the caller's filter before collecting a routed index list.
    pub fn candidates_for<'a>(
        &'a self,
        data: &'a SelectedGameData,
        item_id: i32,
        members: bool,
    ) -> impl Iterator<Item = ServiceCandidate<'a>> + 'a {
        let alias = data
            .item_by_id(item_id)
            .and_then(|item| item.alias.as_deref());
        self.rows.iter().filter_map(move |row| {
            let item = row
                .keeper
                .record
                .items
                .iter()
                .find(|item| Some(item.obj.as_str()) == alias)?;
            (!row.access().is_empty() && (members || !item.members)).then(|| row.candidate())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::selected::ClientRevision;

    fn tile(x: i32, z: i32) -> WorldTile {
        WorldTile { x, z, level: 0 }
    }

    #[test]
    fn keeper_candidates_borrow_access_and_apply_stock_membership_and_access_gates() {
        for revision in [ClientRevision::R274, ClientRevision::R289] {
            let data = crate::game_data::for_revision(revision).expect("selected content");
            let keeper = shop_facts::keepers(&data)
                .expect("generated shop facts")
                .find(|keeper| keeper.keeper_config == "harry")
                .expect("pinned Harry");
            let facts = ShopKeeperFacts::from_rows(
                vec![
                    ResolvedShopKeeper::new(0, keeper, tile(10, 10), [tile(9, 10)]),
                    ResolvedShopKeeper::new(1, keeper, tile(20, 20), []),
                ]
                .into_boxed_slice(),
            );
            let big_net = data.item_by_alias("big_net").expect("members big net").id;
            let harpoon = data.item_by_alias("harpoon").expect("free harpoon").id;
            let axe = data
                .item_by_alias("bronze_axe")
                .expect("not Harry's stock")
                .id;
            let access = facts.rows()[0].access().as_ptr();
            let measured = allocation_counter::measure(|| {
                assert!(facts.candidates_for(&data, big_net, false).next().is_none());
                assert!(facts.candidates_for(&data, axe, true).next().is_none());
                assert!(facts.candidates_for(&data, i32::MAX, true).next().is_none());
                for (item, members) in [(big_net, true), (harpoon, false)] {
                    let mut candidates = facts.candidates_for(&data, item, members);
                    let candidate = candidates.next().expect("one accessible Harry placement");
                    assert_eq!(candidate.kind, ServiceKind::ShopKeeper);
                    assert_eq!(candidate.index, 0);
                    assert_eq!(candidate.tile, tile(10, 10));
                    assert_eq!((candidate.width, candidate.length), (1, 1));
                    assert_eq!(candidate.access.as_ptr(), access);
                    assert!(
                        candidates.next().is_none(),
                        "an empty access row is excluded"
                    );
                }
            });
            assert_eq!(measured.count_total, 0, "{revision:?}");
        }
    }
}
