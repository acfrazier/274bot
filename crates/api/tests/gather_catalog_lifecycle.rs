//! Own test binary with one test: the prepared-catalog cache is process-wide, so its lifecycle cannot share a
//! process with other users of the same revision.
use api::game_data::{for_revision, SelectedGameData};
use api::gather_methods::GatherCatalog;
use api::selected::{ClientRevision, FamilyPreparation};
use std::sync::Arc;

fn prepare(data: &Arc<SelectedGameData>) -> Arc<GatherCatalog> {
    let data = Arc::clone(data);
    FamilyPreparation::run(move |worker| data.prepare_gathering(worker))
        .expect("worker")
        .join()
        .expect("worker panicked")
        .expect("gathering family prepares")
}

#[test]
fn the_catalog_is_lazy_shared_and_released_after_its_last_user() {
    let data = for_revision(ClientRevision::R289).unwrap();
    let pin = data.selected_pin().unwrap();
    let yew = api::snapshot::WorldTile {
        x: 3085,
        z: 3468,
        level: 0,
    };
    assert_eq!(api::gather_methods::loc_footprint_at(&pin, yew), None);
    assert!(
        data.try_gathering().is_none(),
        "loading the selected data decodes no gathering placements"
    );

    let first = prepare(&data);
    let hit = data.try_gathering().expect("a held catalog is a cache hit");
    assert!(
        Arc::ptr_eq(&first, &hit),
        "the tick path shares the allocation"
    );
    let second = prepare(&data);
    assert!(
        Arc::ptr_eq(&first, &second),
        "a second preparation does not decode another copy"
    );
    assert_eq!(
        api::gather_methods::loc_footprint_at(&pin, yew),
        Some((3, 3)),
        "navigation borrows the prepared catalog's rotated placement footprint"
    );

    drop((first, hit));
    assert!(data.try_gathering().is_some(), "one user still holds it");
    drop(second);
    assert!(
        data.try_gathering().is_none(),
        "the allocation is released after the last user, and a tick cannot revive it"
    );
    assert_eq!(
        api::gather_methods::loc_footprint_at(&pin, yew),
        None,
        "navigation lookup neither pins nor revives the released family"
    );

    let again = prepare(&data);
    assert!(!again.methods().is_empty());
    assert!(data.try_gathering().is_some());

    // Another revision is a separate cache entry; releasing one leaves the other.
    let data_274 = for_revision(ClientRevision::R274).unwrap();
    assert!(data_274.try_gathering().is_none());
    let old = prepare(&data_274);
    assert!(!Arc::ptr_eq(&old, &again));
    assert!(Arc::ptr_eq(&old, &data_274.try_gathering().unwrap()));
    drop(old);
    assert!(data_274.try_gathering().is_none());
    assert!(data.try_gathering().is_some());
}
