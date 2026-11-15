use super::*;

fn frame(hp: i32) -> CombatSample {
    CombatSample {
        session: 1,
        tick: 1,
        ingame: true,
        scene: 2,
        tile: Some([3200, 3200, 0]),
        hitpoints: Some(hp),
        in_combat: false,
        target: None,
        hitmark_cycle: Some(0),
    }
}

#[test]
fn the_same_damage_is_disclosed_in_transit_but_invalidates_a_window() {
    for measured in [false, true] {
        let mut observer = CombatObserver::default();
        observer.sample(frame(10));
        if measured {
            observer.begin("product", CombatExpectation::Quiet).unwrap();
        }
        observer.sample(CombatSample {
            tick: 2,
            ..frame(8)
        });
        if measured {
            observer.end("product").unwrap();
        }
        observer.terminal();
        assert_eq!(
            observer.invalid_reason(),
            measured.then_some("hitpoints-decreased")
        );
        let event = observer.evidence.events.last().unwrap();
        assert_eq!(event.previous.unwrap().hitpoints, Some(10));
        assert_eq!(event.sample.hitpoints, Some(8));
        assert_eq!(
            event.classification,
            if measured { "interference" } else { "transit" }
        );
    }
}

#[test]
fn only_a_typed_nonempty_death_label_exempts_intentional_death() {
    for expectation in [
        CombatExpectation::Quiet,
        CombatExpectation::LabelledDeath {
            label: "Haunted Mine ordinary death",
        },
    ] {
        let mut observer = CombatObserver::default();
        observer.sample(frame(10));
        observer
            .begin(
                "deliberate death in the step name is not an exemption",
                expectation,
            )
            .unwrap();
        observer.sample(CombatSample {
            tick: 2,
            in_combat: true,
            ..frame(0)
        });
        observer
            .end("deliberate death in the step name is not an exemption")
            .unwrap();
        observer.terminal();
        assert_eq!(
            observer.invalid_reason().is_none(),
            !matches!(expectation, CombatExpectation::Quiet)
        );
        let event = observer.evidence.events.last().unwrap();
        if let CombatExpectation::LabelledDeath { label } = expectation {
            assert_eq!(event.label, Some(label));
            assert_eq!(event.classification, "labelled_death");
        } else {
            assert_eq!(event.reason, "death");
            assert_eq!(event.classification, "interference");
        }
    }
    let mut observer = CombatObserver::default();
    observer.sample(frame(10));
    assert!(observer
        .begin("death", CombatExpectation::LabelledDeath { label: "" })
        .is_err());
}

#[test]
fn zero_hits_combat_and_targets_are_not_hidden_by_unchanged_hitpoints() {
    for sample in [
        CombatSample {
            hitmark_cycle: Some(70),
            ..frame(10)
        },
        CombatSample {
            in_combat: true,
            ..frame(10)
        },
        CombatSample {
            target: Some(ActorTargetView {
                kind: api::snapshot::ActorKind::Npc,
                index: 7,
            }),
            ..frame(10)
        },
    ] {
        let mut observer = CombatObserver::default();
        observer.sample(frame(10));
        observer.begin("quiet", CombatExpectation::Quiet).unwrap();
        observer.sample(sample);
        assert!(observer.invalid_reason().is_some());
    }
    let mut observer = CombatObserver::default();
    observer.sample(CombatSample {
        in_combat: true,
        ..frame(10)
    });
    observer
        .begin("already fighting", CombatExpectation::Quiet)
        .unwrap();
    assert_eq!(observer.invalid_reason(), Some("combat-active"));
}

#[test]
fn windows_require_matching_boundaries_and_a_continuous_session() {
    let mut observer = CombatObserver::default();
    assert!(observer.end("missing").is_err());
    assert!(observer.begin("missing", CombatExpectation::Quiet).is_err());
    observer.sample(frame(10));
    observer.begin("named", CombatExpectation::Quiet).unwrap();
    assert!(observer.begin("nested", CombatExpectation::Quiet).is_err());
    assert!(observer.end("wrong").is_err());
    observer.sample(CombatSample {
        session: 2,
        tick: 0,
        ..frame(10)
    });
    assert_eq!(observer.invalid_reason(), Some("measured-session-changed"));
    observer.terminal();
    let window = &observer.evidence.windows[0];
    assert_eq!(window.end.unwrap().session, 2);
    assert_eq!(window.end_reason, Some("terminal"));
    assert!(!window.valid);
    assert!(observer.begin("named", CombatExpectation::Quiet).is_err());

    let mut observer = CombatObserver::default();
    observer.sample(frame(10));
    observer
        .begin("unclosed", CombatExpectation::Quiet)
        .unwrap();
    observer.terminal();
    assert_eq!(observer.invalid_reason(), Some("measured-window-not-ended"));
}

#[test]
fn retention_overflow_cannot_certify_a_pass() {
    let mut observer = CombatObserver::default();
    for i in 0..=crate::evidence::MAX_RECEIPT_EVENTS {
        observer.sample(CombatSample {
            tick: i as u32,
            scene: (i % 2) as i32,
            ..frame(10)
        });
    }
    assert_eq!(
        observer.evidence.events.len(),
        crate::evidence::MAX_RECEIPT_EVENTS
    );
    assert_eq!(observer.evidence.dropped_events, 1);
    assert_eq!(observer.invalid_reason(), Some("combat-evidence-overflow"));
}
