use super::consts::POISON_PERIOD;
use api::condition::{PoisonEvent, PoisonMemory, PoisonState};

fn tick(value: i32) -> u16 {
    u16::try_from(value).expect("poison timing constants fit in u16")
}

fn assert_still_unknown(memory: PoisonMemory, since: u16) {
    assert_eq!(memory.state, PoisonState::Unknown { since });
}

#[derive(Debug, Default)]
struct IndependentServerPoison {
    severity: u8,
    ticks_until_timer: u16,
}

impl IndependentServerPoison {
    fn refresh(&mut self, severity: u8) -> bool {
        let onset_chat = self.severity == 0 && severity > 0;
        self.severity = self.severity.max(severity);
        if self.severity != 0 && self.ticks_until_timer == 0 {
            self.ticks_until_timer = tick(POISON_PERIOD);
        }
        onset_chat
    }

    /// One accessible server tick. The server timer decrements raw severity
    /// after damage `ceil(severity / 5)`; refreshes update `max` without chat.
    fn accessible_tick(&mut self) -> Option<u8> {
        if self.ticks_until_timer == 0 {
            return None;
        }
        self.ticks_until_timer = self.ticks_until_timer.saturating_sub(1);
        if self.ticks_until_timer != 0 {
            return None;
        }
        if self.severity == 0 {
            return None;
        }
        self.ticks_until_timer = tick(POISON_PERIOD);
        let damage = self.severity.saturating_sub(1) / 5 + 1;
        self.severity -= 1;
        Some(damage)
    }
}

fn advance_until_mark(
    server: &mut IndependentServerPoison,
    memory: &mut PoisonMemory,
    current_tick: &mut u16,
) {
    for _ in 0..tick(POISON_PERIOD) {
        *current_tick = current_tick.wrapping_add(1);
        if let Some(value) = server.accessible_tick() {
            *memory = memory.transition(PoisonEvent::Hitmark {
                kind: 2,
                value,
                tick: *current_tick,
            });
        }
        if server.severity > 0 {
            assert_ne!(memory.state, PoisonState::Clear);
        }
    }
}

#[test]
fn unknown_silence_and_long_modal_do_not_clear_poison_state() {
    let mut memory = PoisonMemory::from_state(PoisonState::Unknown { since: 0 });
    for _ in 1..=60 {
        memory = memory.transition(PoisonEvent::None);
    }
    assert_still_unknown(memory, 0);

    // A modal and resumed walking are not evidence that the timer ran.
    for _ in 0..40 {
        memory = memory.transition(PoisonEvent::None);
    }
    assert_still_unknown(memory, 0);
}

#[test]
fn a_long_modal_and_resumed_walk_do_not_clear_an_established_poison_mark() {
    let mut server = IndependentServerPoison::default();
    server.refresh(5);
    let mut memory = PoisonMemory::from_state(PoisonState::Unknown { since: 0 });
    let mut now = 0;
    advance_until_mark(&mut server, &mut memory, &mut now);
    let poisoned = memory.state;
    let server_severity = server.severity;

    for _ in 0..40 {
        memory = memory.transition(PoisonEvent::None);
        assert_eq!(server.severity, server_severity);
        assert_eq!(memory.state, poisoned);
    }
}

#[test]
fn same_value_silent_refresh_and_more_marks_never_clear_positive_server_poison() {
    let mut server = IndependentServerPoison::default();
    server.refresh(5);
    let mut memory = PoisonMemory::from_state(PoisonState::Unknown { since: 0 });
    let mut now = 0;

    advance_until_mark(&mut server, &mut memory, &mut now);
    assert!(matches!(
        memory.state,
        PoisonState::Poisoned { per_tick: 1, .. }
    ));
    assert_eq!(server.severity, 4);

    // The server applies max(4, 5) without a chat line or resetting its active timer.
    server.refresh(5);
    for expected_severity in [4, 3, 2, 1] {
        advance_until_mark(&mut server, &mut memory, &mut now);
        assert_eq!(server.severity, expected_severity);
        assert!(matches!(
            memory.state,
            PoisonState::Poisoned { per_tick: 1, .. }
        ));
        if server.severity > 0 {
            assert_ne!(memory.state, PoisonState::Clear);
        }
    }
    assert_eq!(server.severity, 1);
    assert!(matches!(
        memory.state,
        PoisonState::Poisoned { per_tick: 1, .. }
    ));

    let assessment = super::tests::assess_poison_state(memory.state, false);
    assert_eq!(assessment.verdict, super::Verdict::Survivable);
    assert!(assessment.plan.crossings.is_empty());
    assert_eq!(assessment.input.poison, memory.state);
    assert_eq!(assessment.hp_after, 89, "the copied reserve is replayed");
}

#[test]
fn higher_silent_refresh_and_later_lower_marks_use_the_latest_pulse() {
    let mut server = IndependentServerPoison::default();
    server.refresh(5);
    let mut memory = PoisonMemory::from_state(PoisonState::Unknown { since: 0 });
    let mut now = 0;
    advance_until_mark(&mut server, &mut memory, &mut now);
    assert!(matches!(
        memory.state,
        PoisonState::Poisoned { per_tick: 1, .. }
    ));

    server.refresh(10);
    advance_until_mark(&mut server, &mut memory, &mut now);
    assert_eq!(server.severity, 9);
    assert!(matches!(
        memory.state,
        PoisonState::Poisoned { per_tick: 2, .. }
    ));

    for expected_severity in [8, 7, 6, 5] {
        advance_until_mark(&mut server, &mut memory, &mut now);
        assert_eq!(server.severity, expected_severity);
        assert!(matches!(
            memory.state,
            PoisonState::Poisoned { per_tick: 2, .. }
        ));
    }
    advance_until_mark(&mut server, &mut memory, &mut now);
    assert_eq!(server.severity, 4);
    assert!(matches!(
        memory.state,
        PoisonState::Poisoned { per_tick: 1, .. }
    ));
}

#[test]
fn two_hundred_accessible_silent_ticks_retain_the_observed_poison_reserve() {
    let mut server = IndependentServerPoison::default();
    server.refresh(5);
    let mut memory = PoisonMemory::from_state(PoisonState::Unknown { since: 0 });
    let mut now = 0;
    advance_until_mark(&mut server, &mut memory, &mut now);
    assert!(matches!(
        memory.state,
        PoisonState::Poisoned { per_tick: 1, .. }
    ));

    for _ in 0..200 {
        now = now.wrapping_add(1);
        let _server_mark = server.accessible_tick();
        memory = memory.transition(PoisonEvent::None);
        if server.severity > 0 {
            assert_ne!(memory.state, PoisonState::Clear);
        }
    }
    assert_eq!(server.severity, 0);
    assert!(matches!(
        memory.state,
        PoisonState::Poisoned { per_tick: 1, .. }
    ));
}

#[test]
fn five_one_point_marks_without_refresh_do_not_infer_expiration() {
    let mut server = IndependentServerPoison::default();
    server.refresh(5);
    let mut memory = PoisonMemory::from_state(PoisonState::Unknown { since: 0 });
    let mut now = 0;
    for _ in 0..5 {
        advance_until_mark(&mut server, &mut memory, &mut now);
    }
    assert_eq!(server.severity, 0);
    assert!(matches!(
        memory.state,
        PoisonState::Poisoned { per_tick: 1, .. }
    ));
}

#[test]
fn unsupported_mark_rejects_other_poison_events_until_a_new_session() {
    let mut memory = PoisonMemory::from_state(PoisonState::Unknown { since: 0 }).transition(
        PoisonEvent::Hitmark {
            kind: 3,
            value: 7,
            tick: 12,
        },
    );
    assert_still_unknown(memory, 12);
    assert!(memory.session_invalid);

    memory = memory.transition(PoisonEvent::None);
    assert_still_unknown(memory, 12);
    memory = memory.transition(PoisonEvent::Hitmark {
        kind: 2,
        value: 2,
        tick: 50,
    });
    assert_still_unknown(memory, 12);

    memory = memory.transition(PoisonEvent::Reconnect { tick: 60 });
    assert_still_unknown(memory, 60);
    assert!(!memory.session_invalid);
    memory = memory.transition(PoisonEvent::ProvenDrink { origin: 60 });
    assert_eq!(memory.state, PoisonState::Immune { until_lower: 180 });
    assert!(!memory.session_invalid);

    memory = PoisonMemory::from_state(PoisonState::Clear).transition(PoisonEvent::Hitmark {
        kind: 2,
        value: 4,
        tick: 1,
    });
    memory = memory.transition(PoisonEvent::DeathObserved);
    assert!(matches!(
        memory.state,
        PoisonState::Poisoned { per_tick: 4, .. }
    ));
    memory = memory.transition(PoisonEvent::RespawnObserved);
    assert_eq!(memory.state, PoisonState::Clear);

    memory = memory.transition(PoisonEvent::Reconnect { tick: 70 });
    assert_eq!(memory.state, PoisonState::Unknown { since: 70 });
}

#[test]
fn reducer_onset_then_mark() {
    let chat_tick = 25;
    let onset = PoisonMemory::from_state(PoisonState::Clear)
        .transition(PoisonEvent::PoisonChat { tick: chat_tick });
    assert!(!matches!(
        onset.state,
        PoisonState::Unknown { .. } | PoisonState::Clear
    ));

    let marked = onset.transition(PoisonEvent::Hitmark {
        kind: 2,
        value: 6,
        tick: chat_tick.wrapping_add(1),
    });
    assert!(matches!(
        marked.state,
        PoisonState::Poisoned { per_tick: 6, .. }
    ));

    let after_chat = marked.transition(PoisonEvent::PoisonChat { tick: 30 });
    assert!(!matches!(
        after_chat.state,
        PoisonState::Unknown { .. } | PoisonState::Clear
    ));
}

#[test]
fn reducer_smaller_mark_lowers_reserve() {
    let memory = PoisonMemory::from_state(PoisonState::Clear)
        .transition(PoisonEvent::Hitmark {
            kind: 2,
            value: 6,
            tick: 10,
        })
        .transition(PoisonEvent::Hitmark {
            kind: 2,
            value: 5,
            tick: 40,
        });
    assert!(matches!(
        memory.state,
        PoisonState::Poisoned { per_tick: 5, .. }
    ));
}

#[test]
fn reducer_new_onset_replaces_established_mark_until_next_fire() {
    let mut server = IndependentServerPoison::default();
    assert!(server.refresh(15));
    let mut memory = PoisonMemory::from_state(PoisonState::Clear)
        .transition(PoisonEvent::PoisonChat { tick: 0 });
    let mut now = 0;
    for _ in 0..15 {
        advance_until_mark(&mut server, &mut memory, &mut now);
    }
    assert_eq!(server.severity, 0);
    assert!(matches!(
        memory.state,
        PoisonState::Poisoned { per_tick: 1, .. }
    ));
    assert!(server.refresh(55), "a fresh onset emits the chat line");
    memory = memory.transition(PoisonEvent::PoisonChat { tick: now });
    assert!(!matches!(
        memory.state,
        PoisonState::Unknown { .. } | PoisonState::Clear
    ));
    advance_until_mark(&mut server, &mut memory, &mut now);
    assert!(matches!(
        memory.state,
        PoisonState::Poisoned { per_tick: 11, .. }
    ));
}

#[test]
fn unsupported_mark_stays_unknown_for_the_entire_session() {
    let bad = PoisonMemory::from_state(PoisonState::Clear).transition(PoisonEvent::Hitmark {
        kind: 3,
        value: 7,
        tick: 12,
    });
    let cured = bad.transition(PoisonEvent::ProvenDrink { origin: 12 });
    assert_still_unknown(cured, 12);
    assert!(cured.session_invalid);
    let respawned = bad
        .transition(PoisonEvent::DeathObserved)
        .transition(PoisonEvent::RespawnObserved);
    assert_still_unknown(respawned, 12);
    assert!(respawned.session_invalid);
    let new_session = bad.transition(PoisonEvent::Reconnect { tick: 90 });
    assert_still_unknown(new_session, 90);
    assert!(!new_session.session_invalid);
}

#[test]
fn independent_server_refresh_does_not_postpone_an_active_timer() {
    let mut server = IndependentServerPoison::default();
    server.refresh(5);
    for _ in 0..10 {
        assert_eq!(server.accessible_tick(), None);
    }
    server.refresh(10);
    for _ in 0..19 {
        assert_eq!(server.accessible_tick(), None);
    }
    assert_eq!(server.accessible_tick(), Some(2));
    assert_eq!(server.severity, 9);
}
