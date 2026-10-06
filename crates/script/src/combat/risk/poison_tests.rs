use super::consts::{LAG, POISON_PERIOD};
use super::input::{AcceptedClickWindow, PoisonEvent, PoisonMemory, PoisonState};

fn tick(value: i32) -> u16 {
    u16::try_from(value).expect("poison timing constants fit in u16")
}

fn accepted_window(t: u16) -> AcceptedClickWindow {
    let lag = tick(LAG);
    AcceptedClickWindow {
        t,
        click_sent_at: t.wrapping_sub(1),
        map_aim_at_t: true,
        tile_progress_at_t: true,
        main_clear_t: true,
        main_clear_t_plus_1: true,
        chat_clear_t: true,
        chat_clear_t_plus_1: true,
        no_hold_t: true,
        no_hold_t_plus_1: true,
        snapshots_from: t.wrapping_sub(1),
        snapshots_through: t.wrapping_add(1).wrapping_add(lag),
        snapshots_consecutive: true,
        poison_mark_seen_through_end: false,
    }
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
            *memory = memory.transition(
                PoisonEvent::Hitmark {
                    kind: 2,
                    value,
                    tick: *current_tick,
                },
                None,
            );
        }
        if server.severity > 0 {
            assert_ne!(memory.state, PoisonState::Clear);
        }
    }
}

#[test]
fn unknown_silence_and_long_modal_do_not_clear_without_a_full_click_window() {
    let mut memory = PoisonMemory::from_state(PoisonState::Unknown { since: 0 });
    for _ in 1..=60 {
        memory = memory.transition(PoisonEvent::None, None);
    }
    assert_still_unknown(memory, 0);

    // A modal and resumed walking are not evidence that the timer ran.
    for _ in 0..40 {
        memory = memory.transition(PoisonEvent::None, None);
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
        memory = memory.transition(PoisonEvent::None, None);
        assert_eq!(server.severity, server_severity);
        assert_eq!(memory.state, poisoned);
    }
    let t = now.wrapping_add(40).wrapping_add(tick(POISON_PERIOD));
    let mut window = accepted_window(t);
    window.poison_mark_seen_through_end = true;
    memory = memory.transition(PoisonEvent::None, Some(window));
    assert_eq!(memory.state, poisoned);
    assert!(server.severity > 0);
}

#[test]
fn complete_accepted_click_window_clears_unknown_at_the_period_boundary() {
    let since = 100u16;
    let t = since.wrapping_add(tick(POISON_PERIOD));
    let memory = PoisonMemory::from_state(PoisonState::Unknown { since })
        .transition(PoisonEvent::None, Some(accepted_window(t)));
    assert_eq!(memory.state, PoisonState::Clear);
    assert!(!memory.session_invalid);
}

#[test]
fn every_accepted_click_evidence_field_is_required() {
    let since = 10u16;
    let t = since.wrapping_add(tick(POISON_PERIOD));
    let reject = |window| {
        let memory = PoisonMemory::from_state(PoisonState::Unknown { since })
            .transition(PoisonEvent::None, Some(window));
        assert_still_unknown(memory, since);
    };

    let mut window = accepted_window(t);
    window.click_sent_at = t;
    reject(window);

    let mut window = accepted_window(t);
    window.map_aim_at_t = false;
    reject(window);

    let mut window = accepted_window(t);
    window.tile_progress_at_t = false;
    reject(window);

    let mut window = accepted_window(t);
    window.main_clear_t = false;
    reject(window);

    let mut window = accepted_window(t);
    window.main_clear_t_plus_1 = false;
    reject(window);

    let mut window = accepted_window(t);
    window.chat_clear_t = false;
    reject(window);

    let mut window = accepted_window(t);
    window.chat_clear_t_plus_1 = false;
    reject(window);

    let mut window = accepted_window(t);
    window.no_hold_t = false;
    reject(window);

    let mut window = accepted_window(t);
    window.no_hold_t_plus_1 = false;
    reject(window);

    let mut window = accepted_window(t);
    window.snapshots_from = t;
    reject(window);

    let mut window = accepted_window(t);
    window.snapshots_through = t.wrapping_add(1);
    reject(window);

    let mut window = accepted_window(t);
    window.snapshots_consecutive = false;
    reject(window);

    let mut window = accepted_window(t);
    window.poison_mark_seen_through_end = true;
    reject(window);

    let early = t.wrapping_sub(1);
    let memory = PoisonMemory::from_state(PoisonState::Unknown { since })
        .transition(PoisonEvent::None, Some(accepted_window(early)));
    assert_still_unknown(memory, since);
}

#[test]
fn poison_mark_prevents_a_click_window_from_clearing_unknown() {
    let since = 0u16;
    let t = tick(POISON_PERIOD);
    let memory = PoisonMemory::from_state(PoisonState::Unknown { since }).transition(
        PoisonEvent::Hitmark {
            kind: 2,
            value: 1,
            tick: t,
        },
        Some(accepted_window(t)),
    );
    assert_eq!(
        memory.state,
        PoisonState::Poisoned {
            per_tick: 1,
            last_tick: t,
        }
    );
}

#[test]
fn same_value_silent_refresh_and_more_marks_never_clear_positive_server_poison() {
    let mut server = IndependentServerPoison::default();
    server.refresh(5);
    let mut memory = PoisonMemory::from_state(PoisonState::Unknown { since: 0 });
    let mut now = 0;

    advance_until_mark(&mut server, &mut memory, &mut now);
    assert_eq!(
        memory.state,
        PoisonState::Poisoned {
            per_tick: 1,
            last_tick: now
        }
    );
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
fn higher_silent_refresh_and_later_lower_marks_keep_the_largest_damage_seen() {
    let mut server = IndependentServerPoison::default();
    server.refresh(5);
    let mut memory = PoisonMemory::from_state(PoisonState::Unknown { since: 0 });
    let mut now = 0;
    advance_until_mark(&mut server, &mut memory, &mut now);
    assert_eq!(
        memory.state,
        PoisonState::Poisoned {
            per_tick: 1,
            last_tick: now
        }
    );

    server.refresh(10);
    advance_until_mark(&mut server, &mut memory, &mut now);
    assert_eq!(server.severity, 9);
    assert_eq!(
        memory.state,
        PoisonState::Poisoned {
            per_tick: 2,
            last_tick: now
        }
    );

    for expected_severity in [8, 7, 6, 5] {
        advance_until_mark(&mut server, &mut memory, &mut now);
        assert_eq!(server.severity, expected_severity);
        assert_eq!(
            memory.state,
            PoisonState::Poisoned {
                per_tick: 2,
                last_tick: now
            }
        );
    }
    advance_until_mark(&mut server, &mut memory, &mut now);
    assert_eq!(server.severity, 4);
    assert_eq!(
        memory.state,
        PoisonState::Poisoned {
            per_tick: 2,
            last_tick: now,
        }
    );
}

#[test]
fn two_hundred_accessible_silent_ticks_retain_the_largest_poison_reserve() {
    let mut server = IndependentServerPoison::default();
    server.refresh(5);
    let mut memory = PoisonMemory::from_state(PoisonState::Unknown { since: 0 });
    let mut now = 0;
    advance_until_mark(&mut server, &mut memory, &mut now);
    assert_eq!(
        memory.state,
        PoisonState::Poisoned {
            per_tick: 1,
            last_tick: now
        }
    );

    for _ in 0..200 {
        now = now.wrapping_add(1);
        let _server_mark = server.accessible_tick();
        memory = memory.transition(PoisonEvent::None, None);
        if server.severity > 0 {
            assert_ne!(memory.state, PoisonState::Clear);
        }
    }
    assert_eq!(server.severity, 0);
    assert_eq!(
        memory.state,
        PoisonState::Poisoned {
            per_tick: 1,
            last_tick: tick(POISON_PERIOD)
        }
    );
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
    assert_eq!(
        memory.state,
        PoisonState::Poisoned {
            per_tick: 1,
            last_tick: now
        }
    );
}

#[test]
fn unsupported_mark_rejects_clear_evidence_until_a_new_session() {
    let mut memory = PoisonMemory::from_state(PoisonState::Unknown { since: 0 }).transition(
        PoisonEvent::Hitmark {
            kind: 3,
            value: 7,
            tick: 12,
        },
        None,
    );
    assert_still_unknown(memory, 12);
    assert!(memory.session_invalid);

    let valid = accepted_window(12u16.wrapping_add(tick(POISON_PERIOD)));
    memory = memory.transition(PoisonEvent::None, Some(valid));
    assert_still_unknown(memory, 12);
    memory = memory.transition(
        PoisonEvent::Hitmark {
            kind: 2,
            value: 2,
            tick: 50,
        },
        None,
    );
    assert_still_unknown(memory, 12);

    memory = memory.transition(PoisonEvent::Reconnect { tick: 60 }, None);
    assert_still_unknown(memory, 60);
    assert!(!memory.session_invalid);
    memory = memory.transition(PoisonEvent::ConfirmedAntipoisonDecrement, None);
    assert_eq!(memory.state, PoisonState::Clear);
    assert!(!memory.session_invalid);

    memory = PoisonMemory::from_state(PoisonState::Poisoned {
        per_tick: 4,
        last_tick: 1,
    });
    memory = memory.transition(PoisonEvent::DeathObserved, None);
    assert_eq!(
        memory.state,
        PoisonState::Poisoned {
            per_tick: 4,
            last_tick: 1,
        }
    );
    memory = memory.transition(PoisonEvent::RespawnObserved, None);
    assert_eq!(memory.state, PoisonState::Clear);

    memory = memory.transition(PoisonEvent::Reconnect { tick: 70 }, None);
    assert_eq!(memory.state, PoisonState::Unknown { since: 70 });
}

#[test]
fn r1_m1_poison_chat_rearms_unknown_even_after_an_established_mark() {
    let chat_tick = 25;
    let onset = PoisonMemory::from_state(PoisonState::Clear)
        .transition(PoisonEvent::PoisonChat { tick: chat_tick }, None);
    assert_eq!(onset.state, PoisonState::Unknown { since: chat_tick });

    let marked = PoisonMemory::from_state(PoisonState::Clear).transition(
        PoisonEvent::Hitmark {
            kind: 2,
            value: 3,
            tick: 20,
        },
        None,
    );
    let after_chat = marked.transition(PoisonEvent::PoisonChat { tick: 25 }, None);
    assert_eq!(after_chat.state, PoisonState::Unknown { since: 25 });
}

#[test]
fn r1_m1_expired_poison_then_new_onset_refuses_until_the_first_new_mark() {
    let mut server = IndependentServerPoison::default();
    assert!(server.refresh(15));
    let mut memory = PoisonMemory::from_state(PoisonState::Clear)
        .transition(PoisonEvent::PoisonChat { tick: 0 }, None);
    let mut now = 0;
    for _ in 0..15 {
        advance_until_mark(&mut server, &mut memory, &mut now);
    }
    assert_eq!(server.severity, 0);
    assert!(matches!(
        memory.state,
        PoisonState::Poisoned { per_tick: 3, .. }
    ));
    assert!(server.refresh(55), "a fresh onset emits the chat line");
    memory = memory.transition(PoisonEvent::PoisonChat { tick: now }, None);
    assert_eq!(memory.state, PoisonState::Unknown { since: now });
    assert_eq!(
        super::tests::assess_poison_state(memory.state, true).verdict,
        super::Verdict::Unknown(super::UnknownWhy::Poison)
    );
    advance_until_mark(&mut server, &mut memory, &mut now);
    assert_eq!(
        memory.state,
        PoisonState::Poisoned {
            per_tick: 11,
            last_tick: now,
        }
    );
}

#[test]
fn unsupported_mark_stays_unknown_for_the_entire_session() {
    let bad = PoisonMemory::from_state(PoisonState::Clear).transition(
        PoisonEvent::Hitmark {
            kind: 3,
            value: 7,
            tick: 12,
        },
        None,
    );
    let cured = bad.transition(PoisonEvent::ConfirmedAntipoisonDecrement, None);
    assert_still_unknown(cured, 12);
    assert!(cured.session_invalid);
    let respawned = bad
        .transition(PoisonEvent::DeathObserved, None)
        .transition(PoisonEvent::RespawnObserved, None);
    assert_still_unknown(respawned, 12);
    assert!(respawned.session_invalid);
    let new_session = bad.transition(PoisonEvent::Reconnect { tick: 90 }, None);
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
