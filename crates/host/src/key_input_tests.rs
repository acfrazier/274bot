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
