//! The selected prerequisite table is process-shared; warm preparation and
//! borrowed routing queries must not allocate for another bot or request.
use api::cook_locations::{
    cook_camps, default_obstacles, derived_arrive_radius, max_surface_cheb, CookCamp,
};
use api::game_data::for_revision;
use api::identity::Deadline;
use api::named_banks::{bank_catalog, BankDefinition};
use api::prereq_steps::{ActionableGate, CrossingId, PrereqCatalog, PrereqStep};
use api::selected::{
    ClientRevision, FactKey, FamilyPreparation, InclusiveRange, QuestGate, StageWindow,
};
use api::test_support::local_asset_path;
use api::work::{PreparationPhase, SharedCancellation, SliceBudget, StepContext, SystemClock};
use std::hint::black_box;
use std::mem::{size_of, size_of_val};
use std::num::NonZeroU32;
use std::sync::Arc;
use std::time::{Duration, Instant};

#[test]
fn process_catalog_is_bounded_and_warm_preparation_and_queries_allocate_nothing() {
    assert!(size_of::<PrereqCatalog>() <= 32);
    assert!(size_of::<PrereqStep>() <= 768);
    assert!(size_of::<CrossingId>() <= 48);
    assert!(size_of::<ActionableGate>() <= 128);
    if local_asset_path(
        "",
        concat!(env!("CARGO_MANIFEST_DIR"), "/data/game-data/289.json"),
        "revision 289 game data for prerequisite catalog allocation test",
    )
    .is_none()
        || !api::test_support::embedded_game_data_available()
    {
        return;
    }

    let mut static_catalogs = None;
    let static_cold = allocation_counter::measure(|| {
        static_catalogs = Some((
            bank_catalog().expect("generated bank catalog"),
            cook_camps().expect("generated cook catalog"),
            default_obstacles().expect("generated cook obstacles"),
            max_surface_cheb().expect("generated cook pairing distance"),
            derived_arrive_radius().expect("generated cook arrival radius"),
        ));
    });
    let (banks, camps, obstacles, distance, radius) =
        static_catalogs.expect("cold catalog queries return borrowed tables");
    let static_warm = allocation_counter::measure(|| {
        for _ in 0..1_000 {
            assert!(std::ptr::eq(black_box(bank_catalog().unwrap()), banks));
            assert!(std::ptr::eq(black_box(cook_camps().unwrap()), camps));
            assert!(std::ptr::eq(
                black_box(default_obstacles().unwrap()),
                obstacles
            ));
            assert_eq!(black_box(max_surface_cheb().unwrap()), distance);
            assert_eq!(black_box(derived_arrive_radius().unwrap()), radius);
        }
    });
    eprintln!(
        "bank/cook static catalogs: cold={static_cold:?} warm={static_warm:?}; \
         BankDefinition={} CookCamp={} banks={} camps={} obstacles={} \
         bank_rows_bytes={} cook_rows_bytes={} obstacle_refs_bytes={}",
        size_of::<BankDefinition>(),
        size_of::<CookCamp>(),
        banks.len(),
        camps.len(),
        obstacles.len(),
        size_of_val(banks),
        size_of_val(camps),
        size_of_val(obstacles),
    );
    for allocations in [static_cold, static_warm] {
        assert_eq!(
            allocations.count_total, 0,
            "JSON catalogs are compiled to borrowed static tables, not loaded at runtime"
        );
        assert_eq!(allocations.bytes_total, 0);
        assert_eq!(allocations.bytes_current, 0);
    }

    let data = for_revision(ClientRevision::R289).expect("selected R289 data");
    let clock = SystemClock;
    let cancel = SharedCancellation::new();
    let budget = SliceBudget::new(
        NonZeroU32::MIN,
        Deadline::at(Instant::now() + Duration::from_secs(60)),
    );
    let mut step = StepContext::<PreparationPhase>::new(&clock, budget, &cancel);
    let mut prepared = None;
    let cold = allocation_counter::measure(|| {
        prepared = Some(
            FamilyPreparation::within(&mut step, |worker| data.prepare_prereq_steps(worker))
                .expect("verified prerequisite family prepares"),
        );
    });
    eprintln!("prereq process allocation: cold={cold:?}");
    let catalog = prepared.expect("cold preparation returns the shared table");
    assert_eq!(
        catalog.len(),
        5,
        "two dialogue rows and three journal sources"
    );
    let (ordinal, row) = catalog.iter().next().expect("A1 exists");
    let crossing = row.crossings[0].crossing;
    let gate = QuestGate::Window(row.establishes.clone());
    let windows = catalog.establishable_windows();
    let journal_gate = QuestGate::Window(StageWindow {
        quest: FactKey::new("blackarmgang"),
        signal: FactKey::new("phoenixgang"),
        values: InclusiveRange {
            min: Some(10),
            max: Some(10),
        },
    });
    let (journal_ordinal, journal) = catalog
        .for_journal(&journal_gate)
        .expect("gang journal source");

    let warm = allocation_counter::measure(|| {
        for _ in 0..1_000 {
            let prepared =
                FamilyPreparation::within(&mut step, |worker| data.prepare_prereq_steps(worker))
                    .expect("warm preparation");
            assert!(Arc::ptr_eq(&prepared, &catalog));
            let cached = data.try_prereq_steps().expect("cache-only lookup");
            assert!(Arc::ptr_eq(&cached, &catalog));
            let (found, _) = catalog
                .for_gate(&gate, &crossing)
                .expect("exact gate and crossing");
            assert_eq!(found, ordinal);
            assert!(catalog.reference(ordinal, 0).is_some());
            assert!(Arc::ptr_eq(&windows, &catalog.establishable_windows()));
            assert_eq!(
                catalog.for_journal(&journal_gate).unwrap().0,
                journal_ordinal
            );
        }
    });
    eprintln!(
        "prereq process allocation: cold={cold:?} warm={warm:?} catalog={} row={} crossing={} actionable={}",
        size_of::<PrereqCatalog>(),
        size_of::<PrereqStep>(),
        size_of::<CrossingId>(),
        size_of::<ActionableGate>(),
    );
    assert_eq!(
        warm.count_total, 0,
        "another bot/request does not decode or allocate the shared family"
    );
    assert_eq!(warm.bytes_current, 0);

    assert!(
        cold.count_total > 0,
        "counter observes cold family preparation"
    );
    assert!(
        cold.count_total <= 100_000,
        "cold allocation event tripwire"
    );
    assert!(
        cold.bytes_total <= 32 * 1024 * 1024,
        "cold allocation byte tripwire"
    );
    assert!(
        cold.bytes_current > 0,
        "the process cache retains its table"
    );
    assert_eq!(
        warm.count_total, 0,
        "another bot/request does not decode or allocate the shared family"
    );
    assert_eq!(warm.bytes_current, 0);

    use api::prereq_steps::PrereqCatalogError;
    let invalid = |row| PrereqCatalog::from_rows(vec![row]).unwrap_err();
    let mut malformed = row.clone();
    malformed.id = Arc::from("");
    assert_eq!(invalid(malformed), PrereqCatalogError::InvalidRowIdentity);
    let mut malformed = row.clone();
    malformed.crossings = Arc::from([row.crossings[0].clone(), row.crossings[0].clone()]);
    assert_eq!(invalid(malformed), PrereqCatalogError::DuplicateCrossing);
    let mut malformed = row.clone();
    let mut crossing = row.crossings[0].clone();
    crossing.pass.proves.min = Some(i32::MAX);
    crossing.pass.proves.max = Some(i32::MIN);
    malformed.crossings = Arc::from([crossing]);
    assert_eq!(invalid(malformed), PrereqCatalogError::InvalidCrossingFacts);
    let mut malformed = row.clone();
    malformed.establishes.values.min = Some(i32::MAX);
    malformed.establishes.values.max = Some(i32::MIN);
    assert_eq!(invalid(malformed), PrereqCatalogError::InvalidStageRange);
    let mut malformed = journal.clone();
    malformed.rules = Arc::from([]);
    assert_eq!(
        PrereqCatalog::from_entries(Vec::new(), vec![malformed]).unwrap_err(),
        PrereqCatalogError::InvalidJournalFacts,
    );
    let mut malformed = journal.clone();
    malformed.after = Arc::from([]);
    assert_eq!(
        PrereqCatalog::from_entries(Vec::new(), vec![malformed]).unwrap_err(),
        PrereqCatalogError::InvalidJournalFacts,
    );
}
