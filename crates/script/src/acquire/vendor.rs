//! Borrowed vendor candidates and the acquisition owner's routed-pick policy.
//!
//! Pinned `keepers.npc` config/`owned_shop` joins supply identity; selected NPC
//! placements supply footprints. Display names never select a keeper. The host
//! searches the complete eligible domain; air order is only an equal-cost tie.

use super::{BlockReason, Looked, VendorOutcome, VendorTried};
use crate::native::ActionError;
use crate::native_bank::ServicePickOutcome;
use api::game_data::SelectedGameData;
use api::service_pick::{ResolvedShopKeeper, ShopKeeperFacts};
use api::snapshot::WorldTile;
use std::sync::Arc;

/// The acquisition owner's next step after one complete-domain service pick.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VendorDecision {
    /// Walk to this proven access, then trade with this row's exact NPC type.
    Go {
        /// The selected placement row, not a display-name or live-NPC index.
        index: u16,
        /// Nav's selected standable neighbour of the keeper footprint.
        access_tile: WorldTile,
        /// The complete-domain route's accumulated tick cost.
        cost: u32,
    },
    /// Repeat the same eligible domain once; reachability is still undecided.
    Retry,
    /// An established absence or the exhausted one-undecided-retry policy.
    Blocked(BlockReason),
}

/// Build the one retained index list for a vendor request.
///
/// Pinned item membership and non-empty validated access filter candidates.
/// `allowed` restricts explicit keeper identity or exhausted stock, never
/// area-entry eligibility. Membership, quest and skill entry gates belong to
/// the complete routed search. Access rings remain borrowed from shared world
/// facts. The exact-length collection makes one `Arc<[u16]>` allocation;
/// sorting it in place allocates nothing and never pages the routed domain.
pub fn eligible_indices(
    facts: &ShopKeeperFacts,
    data: &SelectedGameData,
    item_id: i32,
    members: bool,
    from: WorldTile,
    allowed: impl Fn(&ResolvedShopKeeper) -> bool,
) -> Arc<[u16]> {
    let candidates = || {
        facts
            .candidates_for(data, item_id, members)
            .filter(|candidate| allowed(&facts.rows()[usize::from(candidate.index)]))
    };
    let count = candidates().count();
    let mut candidates = candidates();
    let mut indices: Arc<[u16]> = (0..count)
        .map(|_| candidates.next().expect("counted vendor candidate").index)
        .collect();
    Arc::get_mut(&mut indices)
        .expect("fresh vendor index list")
        .sort_unstable_by_key(|&index| {
            let tile = facts.rows()[usize::from(index)].tile;
            (
                tile.level != from.level,
                (tile.x - from.x).abs().max((tile.z - from.z).abs()),
                index,
            )
        });
    indices
}

/// Remove exhausted placement identities without confusing repeated keeper
/// types at different shops.
pub(crate) fn exclude_placements(eligible: &[u16], excluded: &[u16]) -> Arc<[u16]> {
    let count = eligible
        .iter()
        .filter(|index| !excluded.contains(index))
        .count();
    let mut remaining = eligible
        .iter()
        .copied()
        .filter(|index| !excluded.contains(index));
    (0..count)
        .map(|_| remaining.next().expect("counted eligible placement"))
        .collect()
}

/// Map one typed service outcome without inventing an air or reachability result.
///
/// One `BudgetExhausted` or `NotSettled` retries the same domain. A second one
/// blocks as undecided, never unreachable. `NoPath` is the whole domain's proof.
/// A routed result must still name an eligible row and one of its retained
/// access tiles; an unrelated/stale result cannot grant a keeper identity.
pub fn choose_vendor(
    facts: &ShopKeeperFacts,
    eligible: &[u16],
    outcome: &ServicePickOutcome,
    undecided_retry_used: &mut bool,
    looked: &mut Looked,
) -> Result<VendorDecision, ActionError> {
    Ok(match outcome {
        ServicePickOutcome::Routed {
            index,
            access_tile,
            cost,
        } => {
            let row = facts
                .rows()
                .get(usize::from(*index))
                .filter(|row| eligible.contains(index) && row.access().contains(access_tile))
                .ok_or(ActionError::Stale)?;
            record_vendor(
                looked,
                row.keeper.keeper_id,
                VendorOutcome::Routed { cost: *cost },
            );
            VendorDecision::Go {
                index: *index,
                access_tile: *access_tile,
                cost: *cost,
            }
        }
        ServicePickOutcome::NoCandidate => VendorDecision::Blocked(BlockReason::NoVendor),
        ServicePickOutcome::NoPath { .. } => {
            record_domain(facts, eligible, looked, VendorOutcome::NoPath);
            VendorDecision::Blocked(BlockReason::VendorUnreachable)
        }
        ServicePickOutcome::BudgetExhausted | ServicePickOutcome::NotSettled => {
            if *undecided_retry_used {
                record_domain(facts, eligible, looked, VendorOutcome::Undecided);
                VendorDecision::Blocked(BlockReason::VendorPickUndecided)
            } else {
                *undecided_retry_used = true;
                VendorDecision::Retry
            }
        }
    })
}

fn record_domain(
    facts: &ShopKeeperFacts,
    eligible: &[u16],
    looked: &mut Looked,
    outcome: VendorOutcome,
) {
    for (position, &index) in eligible.iter().enumerate() {
        let Some(row) = facts.rows().get(usize::from(index)) else {
            continue;
        };
        let keeper_id = row.keeper.keeper_id;
        if eligible[..position].iter().any(|&prior| {
            facts
                .rows()
                .get(usize::from(prior))
                .is_some_and(|row| row.keeper.keeper_id == keeper_id)
        }) {
            continue;
        }
        record_vendor(looked, keeper_id, outcome);
    }
}

pub(crate) fn record_vendor(looked: &mut Looked, keeper_id: i32, outcome: VendorOutcome) {
    if let Some(prior) = looked
        .vendors
        .iter_mut()
        .flatten()
        .find(|prior| prior.keeper_id == keeper_id)
    {
        prior.outcome = outcome;
    } else if let Some(slot) = looked.vendors.iter_mut().find(|slot| slot.is_none()) {
        *slot = Some(VendorTried { keeper_id, outcome });
    } else {
        looked.more_vendors = looked.more_vendors.saturating_add(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use api::selected::ClientRevision;
    use api::shop_facts::{keepers, KeeperFact};

    fn tile(x: i32, z: i32) -> WorldTile {
        WorldTile { x, z, level: 0 }
    }

    fn fixture() -> (Arc<SelectedGameData>, ShopKeeperFacts, i32) {
        let data = api::game_data::for_revision(ClientRevision::R289).expect("selected content");
        let gerrant = keepers(&data)
            .unwrap()
            .find(|row| row.keeper_config == "gerrant")
            .unwrap();
        let harry = keepers(&data)
            .unwrap()
            .find(|row| row.keeper_config == "harry")
            .unwrap();
        let gerrant = KeeperFact {
            keeper_name: "Same keeper",
            ..gerrant
        };
        let harry = KeeperFact {
            keeper_name: "Same keeper",
            ..harry
        };
        assert_ne!(gerrant.keeper_id, harry.keeper_id);
        let rows = vec![
            ResolvedShopKeeper::new(0, gerrant, tile(10, 10), [tile(9, 10)]),
            ResolvedShopKeeper::new(1, harry, tile(20, 20), [tile(19, 20)]),
            ResolvedShopKeeper::new(2, gerrant, tile(30, 30), [tile(29, 30)]),
            ResolvedShopKeeper::new(3, harry, tile(40, 40), []),
        ];
        let item = data.item_by_alias("harpoon").expect("harpoon").id;
        (
            data,
            ShopKeeperFacts::from_rows(rows.into_boxed_slice()),
            item,
        )
    }

    #[test]
    fn vendor_index_list_is_one_allocation_and_borrows_access_without_a_name_join() {
        if !api::test_support::game_data_available() {
            return;
        }
        let (data, facts, item) = fixture();
        let access = facts.rows()[0].access().as_ptr();
        let mut result = None;
        let measured = allocation_counter::measure(|| {
            result = Some(eligible_indices(
                &facts,
                &data,
                item,
                true,
                tile(20, 20),
                |_| true,
            ));
        });
        let indices = result.expect("index list");
        assert_eq!(&*indices, &[1, 0, 2]);
        assert_eq!(measured.count_total, 1);
        assert_eq!(facts.rows()[0].access().as_ptr(), access);
        println!(
            "A3 vendor indices: n={} allocations={} bytes={}",
            indices.len(),
            measured.count_total,
            measured.bytes_total
        );
    }

    #[test]
    fn explicit_keeper_identity_and_empty_access_filter_before_the_request() {
        if !api::test_support::game_data_available() {
            return;
        }
        let (data, facts, item) = fixture();
        let indices = eligible_indices(&facts, &data, item, true, tile(0, 0), |row| {
            row.keeper.keeper_config != "harry"
        });
        assert_eq!(&*indices, &[0, 2]);
        assert!(!indices.contains(&3));
        assert!(eligible_indices(&facts, &data, i32::MAX, true, tile(0, 0), |_| true).is_empty());
    }

    #[test]
    fn routed_identity_is_the_exact_eligible_row_even_with_repeated_display_names() {
        if !api::test_support::game_data_available() {
            return;
        }
        let (_, facts, _) = fixture();
        let mut looked = Looked::default();
        let mut retry = false;
        let choice = choose_vendor(
            &facts,
            &[0, 1, 2],
            &ServicePickOutcome::Routed {
                index: 1,
                access_tile: tile(19, 20),
                cost: 7,
            },
            &mut retry,
            &mut looked,
        )
        .unwrap();
        assert_eq!(
            choice,
            VendorDecision::Go {
                index: 1,
                access_tile: tile(19, 20),
                cost: 7
            }
        );
        assert_eq!(
            looked.vendors[0].unwrap().keeper_id,
            facts.rows()[1].keeper.keeper_id
        );
        assert_eq!(looked.vendors.iter().flatten().count(), 1);
        assert_eq!(
            looked.vendors[0].unwrap().outcome,
            VendorOutcome::Routed { cost: 7 },
            "a winning First search proves nothing about unchosen vendors",
        );
        assert!(!retry);
    }

    #[test]
    fn unrelated_or_non_access_routed_results_fail_closed() {
        if !api::test_support::game_data_available() {
            return;
        }
        let (_, facts, _) = fixture();
        for (index, access_tile) in [
            (1, tile(19, 20)),
            (0, tile(10, 10)),
            (u16::MAX, tile(9, 10)),
        ] {
            let mut looked = Looked::default();
            assert!(matches!(
                choose_vendor(
                    &facts,
                    &[0, 2],
                    &ServicePickOutcome::Routed {
                        index,
                        access_tile,
                        cost: 1
                    },
                    &mut false,
                    &mut looked,
                ),
                Err(ActionError::Stale)
            ));
            assert_eq!(looked, Looked::default());
        }
    }

    #[test]
    fn established_absence_never_spends_an_undecided_retry_and_deduplicates_placements() {
        if !api::test_support::game_data_available() {
            return;
        }
        let (_, facts, _) = fixture();
        let mut retry = false;
        let mut looked = Looked::default();
        assert_eq!(
            choose_vendor(
                &facts,
                &[0, 1, 2],
                &ServicePickOutcome::NoCandidate,
                &mut retry,
                &mut looked
            )
            .unwrap(),
            VendorDecision::Blocked(BlockReason::NoVendor)
        );
        let measured = allocation_counter::measure(|| {
            assert_eq!(
                choose_vendor(
                    &facts,
                    &[0, 1, 2],
                    &ServicePickOutcome::NoPath { unresolved: None },
                    &mut retry,
                    &mut looked
                )
                .unwrap(),
                VendorDecision::Blocked(BlockReason::VendorUnreachable)
            );
        });
        assert_eq!(measured.count_total, 0);
        assert!(!retry);
        assert_eq!(looked.vendors.iter().flatten().count(), 2);
        assert_eq!(looked.more_vendors, 0);
        assert!(looked
            .vendors
            .iter()
            .flatten()
            .all(|row| row.outcome == VendorOutcome::NoPath));
    }

    #[test]
    fn every_undecided_pair_retries_once_then_blocks_without_claiming_unreachable() {
        if !api::test_support::game_data_available() {
            return;
        }
        let (_, facts, _) = fixture();
        for first in [
            ServicePickOutcome::BudgetExhausted,
            ServicePickOutcome::NotSettled,
        ] {
            for second in [
                ServicePickOutcome::BudgetExhausted,
                ServicePickOutcome::NotSettled,
            ] {
                let mut retry = false;
                let mut looked = Looked::default();
                assert_eq!(
                    choose_vendor(&facts, &[0, 1, 2], &first, &mut retry, &mut looked).unwrap(),
                    VendorDecision::Retry
                );
                assert!(retry);
                assert_eq!(looked, Looked::default());
                assert_eq!(
                    choose_vendor(&facts, &[0, 1, 2], &second, &mut retry, &mut looked).unwrap(),
                    VendorDecision::Blocked(BlockReason::VendorPickUndecided)
                );
                assert!(looked
                    .vendors
                    .iter()
                    .flatten()
                    .all(|row| row.outcome == VendorOutcome::Undecided));
            }
        }
    }

    #[test]
    fn diagnostic_records_keep_three_keepers_and_count_the_rest_without_allocating() {
        if !api::test_support::game_data_available() {
            return;
        }
        let data = api::game_data::for_revision(ClientRevision::R289).expect("selected content");
        let rows: Box<[_]> = keepers(&data)
            .unwrap()
            .take(5)
            .enumerate()
            .map(|(index, keeper)| {
                let x = i32::try_from(index).unwrap() * 10 + 10;
                ResolvedShopKeeper::new(
                    u16::try_from(index).unwrap(),
                    keeper,
                    tile(x, 10),
                    [tile(x - 1, 10)],
                )
            })
            .collect();
        assert_eq!(rows.len(), 5);
        let facts = ShopKeeperFacts::from_rows(rows);
        let mut looked = Looked::default();
        let measured = allocation_counter::measure(|| {
            choose_vendor(
                &facts,
                &[0, 1, 2, 3, 4],
                &ServicePickOutcome::NoPath { unresolved: None },
                &mut false,
                &mut looked,
            )
            .unwrap();
        });
        assert_eq!(measured.count_total, 0);
        assert_eq!(looked.vendors.iter().flatten().count(), 3);
        assert_eq!(looked.more_vendors, 2);
    }
}
