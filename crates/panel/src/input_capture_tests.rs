use dear_imgui_rs::Key;
use host::InputEv;
use winit::keyboard::{Key as WinitKey, KeyLocation, NamedKey};

use super::{
    add_shifted_key_event, capture_key_ch, capture_keys, discard_unconsumed_native_capture,
    maybe_send_click, shifted_imgui_key, shifted_imgui_key_at_location, stream_capture,
};

#[test]
fn capture_keys_pass_colon_and_tilde_like_client_play() {
    // client-play KeyCodes.ts: `:` ch 58, `~` ch 126. Panel capture
    // used to map only letters+digits, so `::` / `~` never reached chat.
    assert_eq!(capture_key_ch(Key::Semicolon, true), Some(b':' as i32));
    assert_eq!(capture_key_ch(Key::Semicolon, false), Some(b';' as i32));
    assert_eq!(capture_key_ch(Key::GraveAccent, true), Some(b'~' as i32));
    assert_eq!(capture_key_ch(Key::GraveAccent, false), Some(b'`' as i32));
    assert_eq!(capture_key_ch(Key::Comma, false), Some(b',' as i32));
    assert_eq!(capture_key_ch(Key::Minus, true), Some(b'_' as i32));
}

#[test]
fn shifted_logical_characters_recover_key_lifecycle() {
    assert_eq!(
        shifted_imgui_key(&WinitKey::Character(":".into())),
        Some(Key::Semicolon)
    );
    assert_eq!(
        shifted_imgui_key(&WinitKey::Character("!".into())),
        Some(Key::Key1)
    );
    assert_eq!(
        shifted_imgui_key(&WinitKey::Character("?".into())),
        Some(Key::Slash)
    );
    assert_eq!(shifted_imgui_key(&WinitKey::Character("a".into())), None);
}

#[test]
fn shifted_event_capture_preserves_two_colons_and_release_pairing() {
    let press_key =
        shifted_imgui_key_at_location(&WinitKey::Character(":".into()), KeyLocation::Standard)
            .expect("shifted colon needs a physical ImGui key");
    let release_key =
        shifted_imgui_key_at_location(&WinitKey::Character(":".into()), KeyLocation::Standard)
            .expect("shifted colon release needs the same physical ImGui key");

    assert_eq!(press_key, release_key);
    assert_eq!(capture_key_ch(press_key, true), Some(b':' as i32));
    assert_eq!(capture_key_ch(press_key, true), Some(b':' as i32));
    assert_eq!(capture_key_ch(release_key, false), Some(b';' as i32));
}

#[test]
fn shifted_event_adapter_leaves_numpad_punctuation_to_backend() {
    assert_eq!(
        shifted_imgui_key_at_location(&WinitKey::Character("+".into()), KeyLocation::Numpad,),
        None
    );
    assert_eq!(
        shifted_imgui_key_at_location(&WinitKey::Character("*".into()), KeyLocation::Numpad,),
        None
    );
}

#[test]
fn shifted_event_reaches_capture_across_two_real_imgui_frames() {
    let _guard = crate::IMGUI_CTX_TEST_GUARD.lock().unwrap();
    discard_unconsumed_native_capture();
    let mut ctx = dear_imgui_rs::Context::create();
    let colon = WinitKey::Character(":".into());
    let mut captured = Vec::new();

    for _ in 0..2 {
        ctx.io_mut().add_key_event(Key::LeftShift, true);
        add_shifted_key_event(ctx.io_mut(), &colon, KeyLocation::Standard, true);
        ctx.prepare_frame(
            dear_imgui_rs::FramePrepareOptions::new([900.0, 700.0], 1.0 / 60.0)
                .renderer_has_textures(),
        );
        let frame = ctx.frame();
        captured.push(capture_keys(frame));
        ctx.render();

        add_shifted_key_event(ctx.io_mut(), &colon, KeyLocation::Standard, false);
        ctx.prepare_frame(
            dear_imgui_rs::FramePrepareOptions::new([900.0, 700.0], 1.0 / 60.0)
                .renderer_has_textures(),
        );
        let frame = ctx.frame();
        captured.push(capture_keys(frame));
        ctx.render();
        ctx.io_mut().add_key_event(Key::LeftShift, false);
        ctx.prepare_frame(
            dear_imgui_rs::FramePrepareOptions::new([900.0, 700.0], 1.0 / 60.0)
                .renderer_has_textures(),
        );
        let frame = ctx.frame();
        let _ = capture_keys(frame);
        ctx.render();
    }

    assert_eq!(
        captured,
        vec![
            vec![(true, b':' as i32)],
            vec![(false, b':' as i32)],
            vec![(true, b':' as i32)],
            vec![(false, b':' as i32)],
        ]
    );
}

/// Headed 870 failure: CUA typed `::give` faster than an ImGui frame.
/// Colon press/release and Shift release all arrive before sampling, so
/// reconstructing `ch` from later Shift / coalesced key state drops or
/// mutates the prefix. Capture must keep the produced character.
#[test]
fn native_colon_burst_survives_shift_release_before_imgui_frame() {
    let _guard = crate::IMGUI_CTX_TEST_GUARD.lock().unwrap();
    discard_unconsumed_native_capture();
    let mut ctx = dear_imgui_rs::Context::create();
    let colon = WinitKey::Character(":".into());
    let g = WinitKey::Character("g".into());
    let i = WinitKey::Character("i".into());
    let v = WinitKey::Character("v".into());
    let e = WinitKey::Character("e".into());

    ctx.io_mut().add_key_event(Key::LeftShift, true);
    add_shifted_key_event(ctx.io_mut(), &colon, KeyLocation::Standard, true);
    add_shifted_key_event(ctx.io_mut(), &colon, KeyLocation::Standard, false);
    add_shifted_key_event(ctx.io_mut(), &colon, KeyLocation::Standard, true);
    add_shifted_key_event(ctx.io_mut(), &colon, KeyLocation::Standard, false);
    ctx.io_mut().add_key_event(Key::LeftShift, false);
    for key in [&g, &i, &v, &e] {
        add_shifted_key_event(ctx.io_mut(), key, KeyLocation::Standard, true);
        add_shifted_key_event(ctx.io_mut(), key, KeyLocation::Standard, false);
    }

    ctx.prepare_frame(
        dear_imgui_rs::FramePrepareOptions::new([900.0, 700.0], 1.0 / 60.0).renderer_has_textures(),
    );
    let frame = ctx.frame();
    let captured = capture_keys(frame);
    ctx.render();

    assert_eq!(
        captured,
        vec![
            (true, b':' as i32),
            (false, b':' as i32),
            (true, b':' as i32),
            (false, b':' as i32),
            (true, b'g' as i32),
            (false, b'g' as i32),
            (true, b'i' as i32),
            (false, b'i' as i32),
            (true, b'v' as i32),
            (false, b'v' as i32),
            (true, b'e' as i32),
            (false, b'e' as i32),
        ]
    );

    ctx.prepare_frame(
        dear_imgui_rs::FramePrepareOptions::new([900.0, 700.0], 1.0 / 60.0).renderer_has_textures(),
    );
    let frame = ctx.frame();
    assert!(
        capture_keys(frame).is_empty(),
        "a consumed burst must not replay on the next frame"
    );
    ctx.render();
}

#[test]
fn native_capture_release_keeps_press_character_after_shift_up() {
    let _guard = crate::IMGUI_CTX_TEST_GUARD.lock().unwrap();
    discard_unconsumed_native_capture();
    let mut ctx = dear_imgui_rs::Context::create();
    let colon = WinitKey::Character(":".into());
    let semicolon = WinitKey::Character(";".into());
    add_shifted_key_event(ctx.io_mut(), &colon, KeyLocation::Standard, true);
    add_shifted_key_event(ctx.io_mut(), &semicolon, KeyLocation::Standard, false);
    ctx.prepare_frame(
        dear_imgui_rs::FramePrepareOptions::new([900.0, 700.0], 1.0 / 60.0).renderer_has_textures(),
    );
    let frame = ctx.frame();
    let captured = capture_keys(frame);
    ctx.render();
    assert_eq!(captured, vec![(true, b':' as i32), (false, b':' as i32)]);
}

#[test]
fn native_capture_discard_does_not_replay_after_capture_off() {
    let _guard = crate::IMGUI_CTX_TEST_GUARD.lock().unwrap();
    discard_unconsumed_native_capture();
    let mut ctx = dear_imgui_rs::Context::create();
    let colon = WinitKey::Character(":".into());
    add_shifted_key_event(ctx.io_mut(), &colon, KeyLocation::Standard, true);
    add_shifted_key_event(ctx.io_mut(), &colon, KeyLocation::Standard, false);
    discard_unconsumed_native_capture();
    ctx.prepare_frame(
        dear_imgui_rs::FramePrepareOptions::new([900.0, 700.0], 1.0 / 60.0).renderer_has_textures(),
    );
    let frame = ctx.frame();
    assert!(
        capture_keys(frame).is_empty(),
        "capture-off must discard, not delay-replay"
    );
    ctx.render();
}

fn tap_character(io: &mut dear_imgui_rs::Io, ch: &str) {
    let key = WinitKey::Character(ch.into());
    add_shifted_key_event(io, &key, KeyLocation::Standard, true);
    add_shifted_key_event(io, &key, KeyLocation::Standard, false);
}

fn tap_named(io: &mut dear_imgui_rs::Io, named: NamedKey) {
    let key = WinitKey::Named(named);
    add_shifted_key_event(io, &key, KeyLocation::Standard, true);
    add_shifted_key_event(io, &key, KeyLocation::Standard, false);
}

fn capture_one_frame(ctx: &mut dear_imgui_rs::Context) -> Vec<(bool, i32)> {
    ctx.prepare_frame(
        dear_imgui_rs::FramePrepareOptions::new([900.0, 700.0], 1.0 / 60.0).renderer_has_textures(),
    );
    let frame = ctx.frame();
    let captured = capture_keys(frame);
    ctx.render();
    captured
}

fn down_up(ch: u8) -> [(bool, i32); 2] {
    [(true, ch as i32), (false, ch as i32)]
}

/// Same-frame `a`, Space, `b` must stay `a b`, not `ab ` from native-first
/// then named-append streams.
#[test]
fn native_capture_same_frame_letter_space_letter_keeps_order() {
    let _guard = crate::IMGUI_CTX_TEST_GUARD.lock().unwrap();
    discard_unconsumed_native_capture();
    let mut ctx = dear_imgui_rs::Context::create();
    tap_character(ctx.io_mut(), "a");
    tap_named(ctx.io_mut(), NamedKey::Space);
    tap_character(ctx.io_mut(), "b");
    let captured = capture_one_frame(&mut ctx);
    let mut expected = Vec::new();
    expected.extend_from_slice(&down_up(b'a'));
    expected.extend_from_slice(&down_up(b' '));
    expected.extend_from_slice(&down_up(b'b'));
    assert_eq!(captured, expected);
    assert!(
        capture_one_frame(&mut ctx).is_empty(),
        "a consumed mixed burst must not replay"
    );
}

/// Headed 870 command was `::give bones 25`. Spaces must stay between
/// words when the whole burst arrives before the ImGui sample.
#[test]
fn native_give_bones_25_burst_keeps_spaces_in_order() {
    let _guard = crate::IMGUI_CTX_TEST_GUARD.lock().unwrap();
    discard_unconsumed_native_capture();
    let mut ctx = dear_imgui_rs::Context::create();
    let colon = WinitKey::Character(":".into());
    ctx.io_mut().add_key_event(Key::LeftShift, true);
    add_shifted_key_event(ctx.io_mut(), &colon, KeyLocation::Standard, true);
    add_shifted_key_event(ctx.io_mut(), &colon, KeyLocation::Standard, false);
    add_shifted_key_event(ctx.io_mut(), &colon, KeyLocation::Standard, true);
    add_shifted_key_event(ctx.io_mut(), &colon, KeyLocation::Standard, false);
    ctx.io_mut().add_key_event(Key::LeftShift, false);
    for ch in ["g", "i", "v", "e"] {
        tap_character(ctx.io_mut(), ch);
    }
    tap_named(ctx.io_mut(), NamedKey::Space);
    for ch in ["b", "o", "n", "e", "s"] {
        tap_character(ctx.io_mut(), ch);
    }
    tap_named(ctx.io_mut(), NamedKey::Space);
    tap_character(ctx.io_mut(), "2");
    tap_character(ctx.io_mut(), "5");
    tap_named(ctx.io_mut(), NamedKey::Enter);

    let captured = capture_one_frame(&mut ctx);
    let mut expected = Vec::new();
    expected.extend_from_slice(&down_up(b':'));
    expected.extend_from_slice(&down_up(b':'));
    for ch in b"give" {
        expected.extend_from_slice(&down_up(*ch));
    }
    expected.extend_from_slice(&down_up(b' '));
    for ch in b"bones" {
        expected.extend_from_slice(&down_up(*ch));
    }
    expected.extend_from_slice(&down_up(b' '));
    expected.extend_from_slice(&down_up(b'2'));
    expected.extend_from_slice(&down_up(b'5'));
    expected.extend_from_slice(&down_up(b'\n'));
    assert_eq!(captured, expected);
}

/// Backspace and Enter share the named path; they must not be appended
/// after printables when the events share a frame.
#[test]
fn native_capture_same_frame_backspace_and_enter_keep_order() {
    let _guard = crate::IMGUI_CTX_TEST_GUARD.lock().unwrap();
    discard_unconsumed_native_capture();
    let mut ctx = dear_imgui_rs::Context::create();
    tap_character(ctx.io_mut(), "a");
    tap_named(ctx.io_mut(), NamedKey::Backspace);
    tap_character(ctx.io_mut(), "b");
    tap_named(ctx.io_mut(), NamedKey::Enter);
    let captured = capture_one_frame(&mut ctx);
    let mut expected = Vec::new();
    expected.extend_from_slice(&down_up(b'a'));
    expected.extend_from_slice(&down_up(8));
    expected.extend_from_slice(&down_up(b'b'));
    expected.extend_from_slice(&down_up(10));
    assert_eq!(captured, expected);
}

#[test]
fn native_capture_named_enter_uses_native_event_queue() {
    let _guard = crate::IMGUI_CTX_TEST_GUARD.lock().unwrap();
    discard_unconsumed_native_capture();
    let mut ctx = dear_imgui_rs::Context::create();
    let enter = WinitKey::Named(NamedKey::Enter);
    add_shifted_key_event(ctx.io_mut(), &enter, KeyLocation::Standard, true);
    assert_eq!(capture_one_frame(&mut ctx), vec![(true, 10)]);
    add_shifted_key_event(ctx.io_mut(), &enter, KeyLocation::Standard, false);
    assert_eq!(capture_one_frame(&mut ctx), vec![(false, 10)]);
}

#[test]
fn native_capture_space_character_and_named_are_both_space() {
    let _guard = crate::IMGUI_CTX_TEST_GUARD.lock().unwrap();
    discard_unconsumed_native_capture();
    let mut ctx = dear_imgui_rs::Context::create();
    tap_character(ctx.io_mut(), " ");
    tap_named(ctx.io_mut(), NamedKey::Space);
    let captured = capture_one_frame(&mut ctx);
    let mut expected = Vec::new();
    expected.extend_from_slice(&down_up(b' '));
    expected.extend_from_slice(&down_up(b' '));
    assert_eq!(captured, expected);
}

#[test]
fn native_capture_leaves_numpad_enter_unqueued() {
    let _guard = crate::IMGUI_CTX_TEST_GUARD.lock().unwrap();
    discard_unconsumed_native_capture();
    let mut ctx = dear_imgui_rs::Context::create();
    let enter = WinitKey::Named(NamedKey::Enter);
    add_shifted_key_event(ctx.io_mut(), &enter, KeyLocation::Numpad, true);
    add_shifted_key_event(ctx.io_mut(), &enter, KeyLocation::Numpad, false);
    assert!(
        capture_one_frame(&mut ctx).is_empty(),
        "numpad Enter stays with the backend, not game capture"
    );
}

/// Capture-off discard of a nonempty queue drops held ownership. An
/// empty-queue discard after a drained press must keep it so a later
/// Shift-up release still pairs with `:`.
#[test]
fn native_capture_empty_discard_keeps_drained_press_character() {
    let _guard = crate::IMGUI_CTX_TEST_GUARD.lock().unwrap();
    discard_unconsumed_native_capture();
    let mut ctx = dear_imgui_rs::Context::create();
    let colon = WinitKey::Character(":".into());
    let semicolon = WinitKey::Character(";".into());
    add_shifted_key_event(ctx.io_mut(), &colon, KeyLocation::Standard, true);
    assert_eq!(capture_one_frame(&mut ctx), vec![(true, b':' as i32)]);
    discard_unconsumed_native_capture();
    add_shifted_key_event(ctx.io_mut(), &semicolon, KeyLocation::Standard, false);
    assert_eq!(capture_one_frame(&mut ctx), vec![(false, b':' as i32)]);
}

#[test]
fn native_capture_discard_clears_held_when_queue_nonempty() {
    let _guard = crate::IMGUI_CTX_TEST_GUARD.lock().unwrap();
    discard_unconsumed_native_capture();
    let mut ctx = dear_imgui_rs::Context::create();
    let colon = WinitKey::Character(":".into());
    let semicolon = WinitKey::Character(";".into());
    add_shifted_key_event(ctx.io_mut(), &colon, KeyLocation::Standard, true);
    discard_unconsumed_native_capture();
    add_shifted_key_event(ctx.io_mut(), &semicolon, KeyLocation::Standard, false);
    assert_eq!(
        capture_one_frame(&mut ctx),
        vec![(false, b';' as i32)],
        "undrained capture-off must not reconstruct : from discarded press ownership"
    );
}

#[test]
fn native_capture_leaves_numpad_punctuation_unqueued() {
    let _guard = crate::IMGUI_CTX_TEST_GUARD.lock().unwrap();
    discard_unconsumed_native_capture();
    let mut ctx = dear_imgui_rs::Context::create();
    let plus = WinitKey::Character("+".into());
    add_shifted_key_event(ctx.io_mut(), &plus, KeyLocation::Numpad, true);
    add_shifted_key_event(ctx.io_mut(), &plus, KeyLocation::Numpad, false);
    ctx.prepare_frame(
        dear_imgui_rs::FramePrepareOptions::new([900.0, 700.0], 1.0 / 60.0).renderer_has_textures(),
    );
    let frame = ctx.frame();
    assert!(
        capture_keys(frame).is_empty(),
        "numpad + stays with the backend, not game capture"
    );
    ctx.render();
}

#[test]
fn maybe_send_click_is_noop_without_tx() {
    maybe_send_click(&None, 1.0, 1.0, 765.0, 503.0);
}

#[test]
fn stream_capture_is_noop_without_tx() {
    stream_capture(
        &None,
        1.0,
        1.0,
        765.0,
        503.0,
        true,
        true,
        true,
        true,
        &[(true, b'a' as i32)],
    );
}

#[test]
fn stream_capture_sends_move_then_down() {
    let (tx, rx) = std::sync::mpsc::channel();
    stream_capture(
        &Some(tx),
        0.0,
        0.0,
        765.0,
        503.0,
        true,
        false,
        false,
        false,
        &[],
    );
    match rx.try_recv() {
        Ok(InputEv::Move { x, y }) => assert_eq!((x, y), (0, 0)),
        other => panic!("{other:?}"),
    }
    match rx.try_recv() {
        Ok(InputEv::Down { button, x, y }) => assert_eq!((button, x, y), (1, 0, 0)),
        other => panic!("{other:?}"),
    }
    assert!(rx.try_recv().is_err());
}

#[test]
fn stream_capture_sends_right_up_and_key() {
    let (tx, rx) = std::sync::mpsc::channel();
    stream_capture(
        &Some(tx),
        0.0,
        0.0,
        765.0,
        503.0,
        false,
        true,
        true,
        false,
        &[(true, 10)],
    );
    let evs: Vec<_> = std::iter::from_fn(|| rx.try_recv().ok()).collect();
    assert!(matches!(evs[0], InputEv::Move { x: 0, y: 0 }));
    assert!(matches!(
        evs[1],
        InputEv::Down {
            button: 2,
            x: 0,
            y: 0
        }
    ));
    assert!(matches!(evs[2], InputEv::Up));
    assert!(matches!(evs[3], InputEv::Key { down: true, ch: 10 }));
}

#[test]
fn maybe_send_click_sends_when_tx_present() {
    let (tx, rx) = std::sync::mpsc::channel();
    maybe_send_click(&Some(tx), 0.0, 0.0, 765.0, 503.0);
    match rx.try_recv() {
        Ok(InputEv::Down { x, y, .. }) => assert_eq!((x, y), (0, 0)),
        other => panic!("{other:?}"),
    }
}

#[test]
fn maybe_send_click_outside_image_sends_nothing() {
    let (tx, rx) = std::sync::mpsc::channel();
    maybe_send_click(&Some(tx), -5.0, 10.0, 765.0, 503.0);
    assert!(rx.try_recv().is_err());
}
