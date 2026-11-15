//! Fixed-input observer proof, including the real producer thread and reallocation.
use api::snapshot::GameSnapshot;
use scenario::interference::{CombatObserver, CombatSample};

#[test]
fn disarmed_combat_observation_allocates_nothing_after_repeated_owner_and_session_changes() {
    std::thread::spawn(|| {
        let snapshot = GameSnapshot::new();
        for session in 0..8 {
            let mut owner = Some(Box::<CombatObserver>::default());
            CombatObserver::observe(owner.as_deref_mut(), &snapshot, session);
            drop(owner.take());
            CombatObserver::observe(owner.as_deref_mut(), &snapshot, session + 1);
            let counts = allocation_counter::measure(|| {
                for _ in 0..1000 {
                    CombatObserver::observe(
                        std::hint::black_box(owner.as_deref_mut()),
                        &snapshot,
                        session + 1,
                    );
                }
            });
            assert_eq!(
                counts.count_total, 0,
                "disarmed combat observer allocated or reallocated: {counts:?}"
            );
            assert_eq!(
                counts.bytes_total, 0,
                "disarmed combat observer retained bytes"
            );
        }
    })
    .join()
    .expect("observer producer thread");
}

#[test]
fn measured_combat_owner_has_fixed_state_size() {
    println!(
        "CombatSample={} CombatObserver={} ScenarioRunner={}",
        std::mem::size_of::<CombatSample>(),
        std::mem::size_of::<CombatObserver>(),
        std::mem::size_of::<scenario::ScenarioRunner>()
    );
    assert_eq!(std::mem::size_of::<Option<Box<CombatObserver>>>(), 8);
    assert_eq!(std::mem::size_of::<CombatSample>(), 72);
    assert_eq!(std::mem::size_of::<CombatObserver>(), 160);
    // The existing 5,784 B runner gains one owner pointer and one receipt pointer.
    assert_eq!(std::mem::size_of::<scenario::ScenarioRunner>(), 5800);
}
