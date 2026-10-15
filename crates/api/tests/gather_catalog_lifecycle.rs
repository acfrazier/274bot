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
fn the_catalog_is_lazy_shared_and_retained_for_the_process() {
    let data = for_revision(ClientRevision::R289).unwrap();
    assert!(
        data.try_gathering().is_none(),
        "loading selected data does not decode gathering placements"
    );

    let first = prepare(&data);
    let hit = data
        .try_gathering()
        .expect("the prepared catalog is cached");
    assert!(
        Arc::ptr_eq(&first, &hit),
        "the tick path shares the allocation"
    );
    let second = prepare(&data);
    assert!(
        Arc::ptr_eq(&first, &second),
        "another preparation shares it"
    );
    let identity = Arc::downgrade(&first);

    drop((first, hit, second));
    let cached = data
        .try_gathering()
        .expect("the bounded cache owns the catalog after the last caller");
    assert!(Arc::ptr_eq(&cached, &identity.upgrade().unwrap()));
    let again = prepare(&data);
    assert!(Arc::ptr_eq(&again, &cached), "the family is decoded once");

    // The other pinned revision occupies its own one of the two fixed slots.
    let data_274 = for_revision(ClientRevision::R274).unwrap();
    assert!(data_274.try_gathering().is_none());
    let old = prepare(&data_274);
    assert!(!Arc::ptr_eq(&old, &again));
    let old_identity = Arc::downgrade(&old);
    drop(old);
    assert!(Arc::ptr_eq(
        &data_274.try_gathering().unwrap(),
        &old_identity.upgrade().unwrap()
    ));
    assert!(Arc::ptr_eq(&data.try_gathering().unwrap(), &again));
}
