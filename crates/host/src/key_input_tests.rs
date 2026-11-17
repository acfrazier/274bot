use super::{InputEv, KeyOwner, SlotInput};
use api::native_input::{GestureKey, KeyGesture, KeyInputError, KeyPhase};
use client::client::GameShell;
use std::sync::Arc;
use std::time::{Duration, Instant};

fn gesture() -> KeyGesture {
    KeyGesture::new(GestureKey::ArrowLeft, 100).unwrap()
}

fn consume(input: &SlotInput, shell: &mut GameShell, now: Instant) {
    let mut permit = input.authority.lock();
    input.drain_user(shell);
    input.consume_script_key(shell, &mut permit, now);
}

fn user_input() -> (Arc<SlotInput>, std::sync::mpsc::Sender<InputEv>, GameShell) {
    let input = SlotInput::new();
    let (tx, rx) = std::sync::mpsc::channel();
    input.connect_rx(rx);
    input.set_enabled(true);
    (input, tx, GameShell::new())
}

#[test]
fn key_duration_completes_and_releases_only_the_script_key() {
    let (input, tx, mut shell) = user_input();
    tx.send(InputEv::Key {
        down: true,
        ch: i32::from(b'7'),
    })
    .unwrap();
    let lease = input.authority.key_gesture(gesture()).unwrap();
    let now = Instant::now();
    let ch = GestureKey::ArrowLeft.shell_char();
    consume(&input, &mut shell, now);
    assert_eq!(shell.key_held[ch as usize], 1);
    assert!(matches!(input.key_owner(ch), KeyOwner::Script { .. }));
    consume(&input, &mut shell, now + Duration::from_millis(99));
    assert_eq!(shell.key_held[ch as usize], 1);
    consume(&input, &mut shell, now + Duration::from_millis(100));
    assert_eq!(shell.key_held[ch as usize], 0);
    assert_eq!(shell.key_held[b'7' as usize], 1);
    assert_eq!(input.key_owner(ch), KeyOwner::None);
    let progress = lease.progress().unwrap();
    assert_eq!(progress.phase, KeyPhase::Released);
    assert_eq!(progress.applied_frames, 2);
    assert_eq!(progress.held_ms, 100);
    drop(lease);
    assert!(!input.key_input_active());
}

#[test]
fn held_user_key_refuses_script_without_clearing_the_user() {
    let (input, tx, mut shell) = user_input();
    let ch = GestureKey::ArrowLeft.shell_char();
    tx.send(InputEv::Key { down: true, ch }).unwrap();
    input.consume_native_frame(&mut shell);
    let lease = input.authority.key_gesture(gesture()).unwrap();
    input.consume_native_frame(&mut shell);
    assert_eq!(lease.progress().unwrap().phase, KeyPhase::Cancelled);
    assert_eq!(input.key_owner(ch), KeyOwner::User);
    assert_eq!(shell.key_held[ch as usize], 1);
}

#[test]
fn human_keydown_preempts_script_and_its_held_bit_survives_script_cancel() {
    let (input, tx, mut shell) = user_input();
    let lease = input.authority.key_gesture(gesture()).unwrap();
    let ch = GestureKey::ArrowLeft.shell_char();
    input.consume_native_frame(&mut shell);
    tx.send(InputEv::Key { down: true, ch }).unwrap();
    input.consume_native_frame(&mut shell);
    assert_eq!(lease.progress().unwrap().phase, KeyPhase::Cancelled);
    drop(lease);
    input.cancel_native_keys(&mut shell);
    assert_eq!(input.key_owner(ch), KeyOwner::User);
    assert_eq!(shell.key_held[ch as usize], 1);
    tx.send(InputEv::Key { down: false, ch }).unwrap();
    input.consume_native_frame(&mut shell);
    assert_eq!(shell.key_held[ch as usize], 0);
}

#[test]
fn same_frame_user_down_up_still_cancels_script_ownership() {
    let (input, tx, mut shell) = user_input();
    let lease = input.authority.key_gesture(gesture()).unwrap();
    input.consume_native_frame(&mut shell);
    let ch = GestureKey::ArrowLeft.shell_char();
    for down in [true, false] {
        tx.send(InputEv::Key { down, ch }).unwrap();
    }
    input.consume_native_frame(&mut shell);
    assert_eq!(lease.progress().unwrap().phase, KeyPhase::Cancelled);
    assert_eq!(input.key_owner(ch), KeyOwner::None);
    assert_eq!(shell.key_held[ch as usize], 0);
}

#[test]
fn a_released_user_tap_does_not_preempt_a_later_script_hold() {
    let (input, tx, mut shell) = user_input();
    let ch = GestureKey::ArrowLeft.shell_char();
    for down in [true, false] {
        tx.send(InputEv::Key { down, ch }).unwrap();
    }
    let now = Instant::now();
    consume(&input, &mut shell, now);
    assert_eq!(shell.key_held[ch as usize], 0);
    assert_eq!(input.key_owner(ch), KeyOwner::None);

    let identity = input.authority.publish_live();
    let ticket = input
        .authority
        .script_key_gesture(identity, gesture())
        .unwrap();
    consume(&input, &mut shell, now + Duration::from_millis(20));
    assert_eq!(
        input.authority.key_progress(ticket).unwrap().phase,
        KeyPhase::Held
    );
    assert_eq!(shell.key_held[ch as usize], 1);
    assert_eq!(input.key_owner(ch), KeyOwner::Script { ticket });
    consume(&input, &mut shell, now + Duration::from_millis(120));
    assert_eq!(
        input.authority.key_progress(ticket).unwrap().phase,
        KeyPhase::Released
    );
    assert_eq!(shell.key_held[ch as usize], 0);
    assert!(!input.key_input_active());
}

#[test]
fn unowned_user_keyup_and_focus_detach_cannot_release_a_script_key() {
    let (input, tx, mut shell) = user_input();
    let lease = input.authority.key_gesture(gesture()).unwrap();
    input.consume_native_frame(&mut shell);
    let ch = GestureKey::ArrowLeft.shell_char();
    tx.send(InputEv::Key { down: false, ch }).unwrap();
    input.consume_native_frame(&mut shell);
    input.set_keyboard_enabled(false);
    input.disconnect_rx();
    input.set_enabled(false);
    input.consume_native_frame(&mut shell);
    assert_eq!(shell.key_held[ch as usize], 1);
    assert!(matches!(input.key_owner(ch), KeyOwner::Script { .. }));
    drop(lease);
    assert!(
        input.key_input_active(),
        "a cancelled physical hold still needs its release frame"
    );
    input.consume_native_frame(&mut shell);
    assert_eq!(shell.key_held[ch as usize], 0);
    assert!(!input.key_input_active());
}

#[test]
fn script_error_stop_and_replacement_revoke_only_the_producing_identity() {
    for exit in ["error", "stop", "replacement", "watchdog"] {
        let input = SlotInput::new();
        let identity = input.authority.publish_live();
        let ticket = input
            .authority
            .script_key_gesture(identity, gesture())
            .unwrap();
        let mut shell = GameShell::new();
        input.consume_native_frame(&mut shell);
        match exit {
            "replacement" => {
                input.authority.publish_live();
            }
            "watchdog" => input.authority.sync_live(false),
            _ => input.authority.revoke(),
        }
        assert_eq!(
            input.authority.key_progress(ticket).unwrap().phase,
            KeyPhase::Cancelled,
            "{exit}"
        );
        input.consume_native_frame(&mut shell);
        assert_eq!(
            shell.key_held[GestureKey::ArrowLeft.shell_char() as usize],
            0,
            "{exit}"
        );
        input.authority.resume();
        assert_eq!(
            input.authority.script_key_gesture(identity, gesture()),
            Err(KeyInputError::StaleOwner)
        );
        input.consume_native_frame(&mut shell);
        assert_eq!(
            input.key_owner(GestureKey::ArrowLeft.shell_char()),
            KeyOwner::None
        );
    }
}

#[test]
fn scenario_hold_survives_unrelated_script_stop_replacement_and_pause() {
    for event in ["stop", "replacement", "pause", "sync-not-live"] {
        let input = SlotInput::new();
        input.authority.publish_live();
        let lease = input.authority.key_gesture(gesture()).unwrap();
        let mut shell = GameShell::new();
        let now = Instant::now();
        let ch = GestureKey::ArrowLeft.shell_char();
        consume(&input, &mut shell, now);
        assert_eq!(lease.progress().unwrap().phase, KeyPhase::Held);
        match event {
            "replacement" => {
                input.authority.publish_live();
            }
            "sync-not-live" => input.authority.sync_live(false),
            _ => input.authority.revoke(),
        }
        assert_eq!(lease.progress().unwrap().phase, KeyPhase::Held, "{event}");
        consume(&input, &mut shell, now + Duration::from_millis(20));
        assert_eq!(lease.progress().unwrap().phase, KeyPhase::Held, "{event}");
        assert_eq!(shell.key_held[ch as usize], 1, "{event}");
        assert_eq!(
            input.key_owner(ch),
            KeyOwner::Script {
                ticket: lease.progress().unwrap().ticket
            },
            "{event}"
        );
        drop(lease);
        consume(&input, &mut shell, now + Duration::from_millis(40));
        assert_eq!(shell.key_held[ch as usize], 0, "{event}");
        assert!(!input.key_input_active(), "{event}");
    }
}

#[test]
fn closing_the_host_consume_gate_cancels_and_releases_a_held_key() {
    for script_owned in [false, true] {
        let input = SlotInput::new();
        let identity = input.authority.publish_live();
        let lease = (!script_owned).then(|| input.authority.key_gesture(gesture()).unwrap());
        let ticket = match &lease {
            Some(lease) => lease.progress().unwrap().ticket,
            None => input
                .authority
                .script_key_gesture(identity, gesture())
                .unwrap(),
        };
        let mut shell = GameShell::new();
        let now = Instant::now();
        let ch = GestureKey::ArrowLeft.shell_char();
        consume(&input, &mut shell, now);
        assert_eq!(shell.key_held[ch as usize], 1);
        input.set_host_consume_allowed(false);
        consume(&input, &mut shell, now + Duration::from_millis(20));
        assert_eq!(
            input.authority.key_progress(ticket).unwrap().phase,
            KeyPhase::Cancelled
        );
        assert_eq!(shell.key_held[ch as usize], 0);
        assert_eq!(input.key_owner(ch), KeyOwner::None);
        assert!(!input.key_input_active());
        input.set_host_consume_allowed(true);
        consume(&input, &mut shell, now + Duration::from_millis(40));
        assert_eq!(shell.key_held[ch as usize], 0, "reopening cannot replay");
        assert!(!input.key_input_active());
    }
}

#[test]
fn old_lease_drop_cannot_release_a_later_ticket() {
    let input = SlotInput::new();
    let first = input.authority.key_gesture(gesture()).unwrap();
    let mut shell = GameShell::new();
    let now = Instant::now();
    consume(&input, &mut shell, now);
    consume(&input, &mut shell, now + Duration::from_millis(100));
    let next = input.authority.key_gesture(gesture()).unwrap();
    consume(&input, &mut shell, now + Duration::from_millis(101));
    drop(first);
    input.consume_native_frame(&mut shell);
    assert_eq!(next.progress().unwrap().phase, KeyPhase::Held);
    assert_eq!(
        shell.key_held[GestureKey::ArrowLeft.shell_char() as usize],
        1
    );
}

#[test]
fn a_busy_key_owner_cannot_be_overwritten_by_another_producer() {
    let input = SlotInput::new();
    let identity = input.authority.publish_live();
    let first = input.authority.key_gesture(gesture()).unwrap();
    let other = KeyGesture::new(GestureKey::ArrowRight, 100).unwrap();
    assert!(matches!(
        input.authority.key_gesture(other),
        Err(KeyInputError::Busy)
    ));
    assert_eq!(
        input.authority.script_key_gesture(identity, other),
        Err(KeyInputError::Busy)
    );
    let mut shell = GameShell::new();
    input.consume_native_frame(&mut shell);
    assert_eq!(
        input.key_owner(GestureKey::ArrowLeft.shell_char()),
        KeyOwner::Script {
            ticket: first.progress().unwrap().ticket
        }
    );
    assert_eq!(
        shell.key_held[GestureKey::ArrowRight.shell_char() as usize],
        0
    );
    drop(first);
    input.consume_native_frame(&mut shell);
    assert_eq!(
        shell.key_held[GestureKey::ArrowLeft.shell_char() as usize],
        0
    );
}

#[test]
fn unused_and_warmed_key_consumers_allocate_nothing_on_the_owner_thread() {
    std::thread::spawn(|| {
        let input = SlotInput::new();
        let mut shell = GameShell::new();
        input.consume_native_frame(&mut shell);
        let disabled = allocation_counter::measure(|| {
            for _ in 0..1000 { input.consume_native_frame(&mut shell); }
        });
        assert_eq!(disabled.count_total, 0);
        assert_eq!(disabled.bytes_current, 0);
        let warm = input.authority.key_gesture(gesture()).unwrap();
        input.consume_native_frame(&mut shell);
        drop(warm);
        input.consume_native_frame(&mut shell);
        let armed = allocation_counter::measure(|| {
            for _ in 0..128 {
                let lease = input.authority.key_gesture(gesture()).unwrap();
                input.consume_native_frame(&mut shell);
                drop(lease);
                input.consume_native_frame(&mut shell);
            }
        });
        assert_eq!(armed.count_total, 0);
        assert_eq!(armed.bytes_current, 0);
        assert_eq!(shell.key_held[GestureKey::ArrowLeft.shell_char() as usize], 0);
        eprintln!("key input: 1000 disabled frames={} allocations; 128 warmed owner cycles={} allocations", disabled.count_total, armed.count_total);
    }).join().unwrap();
}

#[test]
fn key_slot_storage_size_ceiling() {
    let user_bytes = std::mem::size_of::<super::UserInput>();
    let slot_bytes = std::mem::size_of::<SlotInput>();
    assert!(
        user_bytes <= 192,
        "user input uses {user_bytes} bytes, ceiling 192"
    );
    assert!(
        slot_bytes <= 576,
        "slot input uses {slot_bytes} bytes, ceiling 576"
    );
    eprintln!("key slot storage: user={user_bytes} slot={slot_bytes}");
}

#[test]
fn slot_construction_and_first_script_key_have_bounded_allocations() {
    std::thread::spawn(|| {
        for slot in 0..2 {
            let mut input = None;
            let construction = allocation_counter::measure(|| {
                input = Some(SlotInput::new());
            });
            let input = input.unwrap();
            let mut shell = GameShell::new();
            let now = Instant::now();
            let first_use = allocation_counter::measure(|| {
                let identity = input.authority.publish_live();
                let ticket = input
                    .authority
                    .script_key_gesture(identity, gesture())
                    .unwrap();
                consume(&input, &mut shell, now);
                assert_eq!(
                    input.authority.key_progress(ticket).unwrap().phase,
                    KeyPhase::Held
                );
                consume(&input, &mut shell, now + Duration::from_millis(100));
                assert_eq!(
                    input.authority.key_progress(ticket).unwrap().phase,
                    KeyPhase::Released
                );
            });
            assert!(construction.count_total <= 2, "{construction:?}");
            assert!(construction.bytes_total <= 648, "{construction:?}");
            // Cold consumption also initializes the platform's four slot mutexes.
            assert!(first_use.count_total <= 5, "{first_use:?}");
            assert!(first_use.bytes_total <= 336, "{first_use:?}");
            assert!(construction.bytes_total + first_use.bytes_total <= 984);
            let reused = allocation_counter::measure(|| {
                let identity = input.authority.publish_live();
                input
                    .authority
                    .script_key_gesture(identity, gesture())
                    .unwrap();
                consume(&input, &mut shell, now);
                consume(&input, &mut shell, now + Duration::from_millis(100));
            });
            assert_eq!(reused.count_total, 0, "{reused:?}");
            assert_eq!(reused.bytes_current, 0, "{reused:?}");
            eprintln!(
                "key slot {slot}: construction={construction:?}; first_use={first_use:?}; reused={reused:?}"
            );
        }
    })
    .join()
    .unwrap();
}
