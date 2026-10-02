use client::client::GameShell;
use dear_imgui_rs::Key;
use host::InputEv;
use winit::keyboard::{Key as WinitKey, KeyLocation, NamedKey};

use super::{
    add_shifted_key_event, capture_key_ch, discard_unconsumed_native_capture,
    game_keyboard_available, maybe_send_click, shifted_imgui_key, shifted_imgui_key_at_location,
    stream_capture, KeyboardOwner,
};

struct KeyCapture {
    owner: KeyboardOwner,
    tx: std::sync::mpsc::Sender<InputEv>,
    rx: std::sync::mpsc::Receiver<InputEv>,
}

impl KeyCapture {
    fn new() -> Self {
        let (tx, rx) = std::sync::mpsc::channel();
        Self {
            owner: KeyboardOwner::default(),
            tx,
            rx,
        }
    }
}

fn capture_keys(ui: &dear_imgui_rs::Ui, capture: &mut KeyCapture) -> Vec<(bool, i32)> {
    capture
        .owner
        .process_ownership(ui, Some(&capture.tx), Some("game"));
    capture.owner.capture_keys(ui, Some(&capture.tx));
    key_chs(&capture.rx.try_iter().collect::<Vec<_>>())
}

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
    let _guard = crate::test_support::imgui_context_guard();
    discard_unconsumed_native_capture();
    let mut ctx = dear_imgui_rs::Context::create();
    let mut capture = KeyCapture::new();
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
        captured.push(capture_keys(frame, &mut capture));
        ctx.render();

        add_shifted_key_event(ctx.io_mut(), &colon, KeyLocation::Standard, false);
        ctx.prepare_frame(
            dear_imgui_rs::FramePrepareOptions::new([900.0, 700.0], 1.0 / 60.0)
                .renderer_has_textures(),
        );
        let frame = ctx.frame();
        captured.push(capture_keys(frame, &mut capture));
        ctx.render();
        ctx.io_mut().add_key_event(Key::LeftShift, false);
        ctx.prepare_frame(
            dear_imgui_rs::FramePrepareOptions::new([900.0, 700.0], 1.0 / 60.0)
                .renderer_has_textures(),
        );
        let frame = ctx.frame();
        let _ = capture_keys(frame, &mut capture);
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
    let _guard = crate::test_support::imgui_context_guard();
    discard_unconsumed_native_capture();
    let mut ctx = dear_imgui_rs::Context::create();
    let mut capture = KeyCapture::new();
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
    let captured = capture_keys(frame, &mut capture);
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
        capture_keys(frame, &mut capture).is_empty(),
        "a consumed burst must not replay on the next frame"
    );
    ctx.render();
}

#[test]
fn native_capture_release_keeps_press_character_after_shift_up() {
    let _guard = crate::test_support::imgui_context_guard();
    discard_unconsumed_native_capture();
    let mut ctx = dear_imgui_rs::Context::create();
    let mut capture = KeyCapture::new();
    let colon = WinitKey::Character(":".into());
    let semicolon = WinitKey::Character(";".into());
    add_shifted_key_event(ctx.io_mut(), &colon, KeyLocation::Standard, true);
    add_shifted_key_event(ctx.io_mut(), &semicolon, KeyLocation::Standard, false);
    ctx.prepare_frame(
        dear_imgui_rs::FramePrepareOptions::new([900.0, 700.0], 1.0 / 60.0).renderer_has_textures(),
    );
    let frame = ctx.frame();
    let captured = capture_keys(frame, &mut capture);
    ctx.render();
    assert_eq!(captured, vec![(true, b':' as i32), (false, b':' as i32)]);
}

#[test]
fn native_capture_discard_does_not_replay_after_capture_off() {
    let _guard = crate::test_support::imgui_context_guard();
    discard_unconsumed_native_capture();
    let mut ctx = dear_imgui_rs::Context::create();
    let mut capture = KeyCapture::new();
    let colon = WinitKey::Character(":".into());
    add_shifted_key_event(ctx.io_mut(), &colon, KeyLocation::Standard, true);
    add_shifted_key_event(ctx.io_mut(), &colon, KeyLocation::Standard, false);
    discard_unconsumed_native_capture();
    ctx.prepare_frame(
        dear_imgui_rs::FramePrepareOptions::new([900.0, 700.0], 1.0 / 60.0).renderer_has_textures(),
    );
    let frame = ctx.frame();
    assert!(
        capture_keys(frame, &mut capture).is_empty(),
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

fn capture_one_frame(
    ctx: &mut dear_imgui_rs::Context,
    capture: &mut KeyCapture,
) -> Vec<(bool, i32)> {
    ctx.prepare_frame(
        dear_imgui_rs::FramePrepareOptions::new([900.0, 700.0], 1.0 / 60.0).renderer_has_textures(),
    );
    let frame = ctx.frame();
    let captured = capture_keys(frame, capture);
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
    let _guard = crate::test_support::imgui_context_guard();
    discard_unconsumed_native_capture();
    let mut ctx = dear_imgui_rs::Context::create();
    let mut capture = KeyCapture::new();
    tap_character(ctx.io_mut(), "a");
    tap_named(ctx.io_mut(), NamedKey::Space);
    tap_character(ctx.io_mut(), "b");
    let captured = capture_one_frame(&mut ctx, &mut capture);
    let mut expected = Vec::new();
    expected.extend_from_slice(&down_up(b'a'));
    expected.extend_from_slice(&down_up(b' '));
    expected.extend_from_slice(&down_up(b'b'));
    assert_eq!(captured, expected);
    assert!(
        capture_one_frame(&mut ctx, &mut capture).is_empty(),
        "a consumed mixed burst must not replay"
    );
}

/// Headed 870 command was `::give bones 25`. Spaces must stay between
/// words when the whole burst arrives before the ImGui sample.
#[test]
fn native_give_bones_25_burst_keeps_spaces_in_order() {
    let _guard = crate::test_support::imgui_context_guard();
    discard_unconsumed_native_capture();
    let mut ctx = dear_imgui_rs::Context::create();
    let mut capture = KeyCapture::new();
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

    let captured = capture_one_frame(&mut ctx, &mut capture);
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
    let _guard = crate::test_support::imgui_context_guard();
    discard_unconsumed_native_capture();
    let mut ctx = dear_imgui_rs::Context::create();
    let mut capture = KeyCapture::new();
    tap_character(ctx.io_mut(), "a");
    tap_named(ctx.io_mut(), NamedKey::Backspace);
    tap_character(ctx.io_mut(), "b");
    tap_named(ctx.io_mut(), NamedKey::Enter);
    let captured = capture_one_frame(&mut ctx, &mut capture);
    let mut expected = Vec::new();
    expected.extend_from_slice(&down_up(b'a'));
    expected.extend_from_slice(&down_up(8));
    expected.extend_from_slice(&down_up(b'b'));
    expected.extend_from_slice(&down_up(10));
    assert_eq!(captured, expected);
}

#[test]
fn native_capture_named_enter_uses_native_event_queue() {
    let _guard = crate::test_support::imgui_context_guard();
    discard_unconsumed_native_capture();
    let mut ctx = dear_imgui_rs::Context::create();
    let mut capture = KeyCapture::new();
    let enter = WinitKey::Named(NamedKey::Enter);
    add_shifted_key_event(ctx.io_mut(), &enter, KeyLocation::Standard, true);
    assert_eq!(capture_one_frame(&mut ctx, &mut capture), vec![(true, 10)]);
    add_shifted_key_event(ctx.io_mut(), &enter, KeyLocation::Standard, false);
    assert_eq!(capture_one_frame(&mut ctx, &mut capture), vec![(false, 10)]);
}

#[test]
fn native_capture_space_character_and_named_are_both_space() {
    let _guard = crate::test_support::imgui_context_guard();
    discard_unconsumed_native_capture();
    let mut ctx = dear_imgui_rs::Context::create();
    let mut capture = KeyCapture::new();
    tap_character(ctx.io_mut(), " ");
    tap_named(ctx.io_mut(), NamedKey::Space);
    let captured = capture_one_frame(&mut ctx, &mut capture);
    let mut expected = Vec::new();
    expected.extend_from_slice(&down_up(b' '));
    expected.extend_from_slice(&down_up(b' '));
    assert_eq!(captured, expected);
}

#[test]
fn native_capture_leaves_numpad_enter_unqueued() {
    let _guard = crate::test_support::imgui_context_guard();
    discard_unconsumed_native_capture();
    let mut ctx = dear_imgui_rs::Context::create();
    let mut capture = KeyCapture::new();
    let enter = WinitKey::Named(NamedKey::Enter);
    add_shifted_key_event(ctx.io_mut(), &enter, KeyLocation::Numpad, true);
    add_shifted_key_event(ctx.io_mut(), &enter, KeyLocation::Numpad, false);
    assert!(
        capture_one_frame(&mut ctx, &mut capture).is_empty(),
        "numpad Enter stays with the backend, not game capture"
    );
}

/// Leaving the Image between a shifted press and release must not rewrite
/// the game-owned character, even if no new input was discarded.
#[test]
fn native_capture_empty_discard_keeps_drained_press_character() {
    let _guard = crate::test_support::imgui_context_guard();
    discard_unconsumed_native_capture();
    let mut ctx = dear_imgui_rs::Context::create();
    let mut capture = KeyCapture::new();
    let colon = WinitKey::Character(":".into());
    let semicolon = WinitKey::Character(";".into());
    add_shifted_key_event(ctx.io_mut(), &colon, KeyLocation::Standard, true);
    assert_eq!(
        capture_one_frame(&mut ctx, &mut capture),
        vec![(true, b':' as i32)]
    );
    discard_unconsumed_native_capture();
    add_shifted_key_event(ctx.io_mut(), &semicolon, KeyLocation::Standard, false);
    assert_eq!(
        capture_one_frame(&mut ctx, &mut capture),
        vec![(false, b':' as i32)]
    );
}

#[test]
fn native_capture_leaves_numpad_punctuation_unqueued() {
    let _guard = crate::test_support::imgui_context_guard();
    discard_unconsumed_native_capture();
    let mut ctx = dear_imgui_rs::Context::create();
    let mut capture = KeyCapture::new();
    let plus = WinitKey::Character("+".into());
    add_shifted_key_event(ctx.io_mut(), &plus, KeyLocation::Numpad, true);
    add_shifted_key_event(ctx.io_mut(), &plus, KeyLocation::Numpad, false);
    ctx.prepare_frame(
        dear_imgui_rs::FramePrepareOptions::new([900.0, 700.0], 1.0 / 60.0).renderer_has_textures(),
    );
    let frame = ctx.frame();
    assert!(
        capture_keys(frame, &mut capture).is_empty(),
        "numpad + stays with the backend, not game capture"
    );
    ctx.render();
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

fn prepare_opts() -> dear_imgui_rs::FramePrepareOptions {
    dear_imgui_rs::FramePrepareOptions::new([900.0, 700.0], 1.0 / 60.0).renderer_has_textures()
}

/// Debug-tab search field: the operator-reported leak source.
fn debug_search_frame(ctx: &mut dear_imgui_rs::Context, buf: &mut String, focus: bool) {
    ctx.prepare_frame(prepare_opts());
    {
        let ui = ctx.frame();
        let _ = ui
            .window("debug-search-probe")
            .position([0.0, 0.0], dear_imgui_rs::Condition::Always)
            .size([400.0, 200.0], dear_imgui_rs::Condition::Always)
            .build(|| {
                if focus {
                    ui.set_keyboard_focus_here();
                }
                ui.input_text("##debug-search", buf)
                    .hint("Search commands, categories, descriptions")
                    .build();
            });
    }
    ctx.render();
}

/// Production frame ownership pass followed by a hovered Game-pane sample.
fn stream_hovered_keys(
    ctx: &mut dear_imgui_rs::Context,
    buf: Option<&mut String>,
    left_down: bool,
    require_keyboard: bool,
    capture: &mut KeyCapture,
) -> Vec<InputEv> {
    ctx.prepare_frame(prepare_opts());
    {
        let ui = ctx.frame();
        capture
            .owner
            .process_ownership(ui, Some(&capture.tx), Some("game"));
        if let Some(buf) = buf {
            let _ = ui
                .window("debug-search-probe")
                .position([0.0, 0.0], dear_imgui_rs::Condition::Always)
                .size([400.0, 200.0], dear_imgui_rs::Condition::Always)
                .build(|| {
                    ui.input_text("##debug-search", buf)
                        .hint("Search commands, categories, descriptions")
                        .build();
                });
        }
        if require_keyboard {
            assert!(
                ui.io().want_capture_keyboard() || ui.io().want_text_input(),
                "Debug search InputText must own the keyboard (capture={} text={})",
                ui.io().want_capture_keyboard(),
                ui.io().want_text_input()
            );
        }
        stream_capture(
            &Some(capture.tx.clone()),
            0.0,
            0.0,
            765.0,
            503.0,
            left_down,
            false,
            false,
            false,
            &[],
        );
        capture.owner.capture_keys(ui, Some(&capture.tx));
    }
    ctx.render();
    capture.rx.try_iter().collect()
}

fn key_chs(evs: &[InputEv]) -> Vec<(bool, i32)> {
    evs.iter()
        .filter_map(|ev| match ev {
            InputEv::Key { down, ch } => Some((*down, *ch)),
            _ => None,
        })
        .collect()
}

/// Same KeyboardInput adapter `window_event` uses after the winit backend
/// handles the event: non-repeat `add_shifted_key_event`. winit's `KeyEvent`
/// is not constructible outside that crate, so tests cannot build a
/// `WindowEvent::KeyboardInput`.
fn window_keyboard(io: &mut dear_imgui_rs::Io, key: WinitKey, down: bool) {
    add_shifted_key_event(io, &key, KeyLocation::Standard, down);
}

fn native_type_hi_enter(io: &mut dear_imgui_rs::Io) {
    tap_character(io, "h");
    tap_character(io, "i");
    tap_named(io, NamedKey::Enter);
    tap_named(io, NamedKey::Escape);
    tap_named(io, NamedKey::ArrowLeft);
}

/// Headed report: typing in a panel text field (Debug search) also lands in
/// the client's chat. Drive the native KeyboardInput adapter used by
/// `window_event` (`add_shifted_key_event`) → native queue → hovered
/// `KeyboardOwner::capture_keys`. ImGui owning the keyboard must drop new key-downs
/// (letters, Enter/Escape, arrows); the game pane (no active widget) must
/// still forward them. Mouse Down stays on the capture channel either way.
/// Settled-focus gating only — not a click-back timing or IME-commit proof.
#[test]
fn focused_panel_text_field_does_not_forward_keys_to_client() {
    let _guard = crate::test_support::imgui_context_guard();
    discard_unconsumed_native_capture();
    let mut ctx = dear_imgui_rs::Context::create();
    let mut capture = KeyCapture::new();
    let _ = ctx.set_ini_filename(None::<String>);
    let mut buf = String::new();

    debug_search_frame(&mut ctx, &mut buf, true);
    debug_search_frame(&mut ctx, &mut buf, false);

    native_type_hi_enter(ctx.io_mut());
    let focused = stream_hovered_keys(&mut ctx, Some(&mut buf), true, true, &mut capture);
    assert!(
        focused
            .iter()
            .any(|ev| matches!(ev, InputEv::Move { .. }))
            && focused
                .iter()
                .any(|ev| matches!(ev, InputEv::Down { button: 1, .. })),
        "mouse-owner path must still stream Move/Down while ImGui has the keyboard, got {focused:?}"
    );
    assert!(
        key_chs(&focused).is_empty(),
        "panel text focus must not forward new keys to the client, got {focused:?}"
    );
    drop(ctx);

    discard_unconsumed_native_capture();
    let mut ctx = dear_imgui_rs::Context::create();
    let mut capture = KeyCapture::new();
    let _ = ctx.set_ini_filename(None::<String>);
    native_type_hi_enter(ctx.io_mut());
    let game = stream_hovered_keys(&mut ctx, None, true, false, &mut capture);
    let keys = key_chs(&game);
    let mut expected = Vec::new();
    expected.extend_from_slice(&down_up(b'h'));
    expected.extend_from_slice(&down_up(b'i'));
    expected.extend_from_slice(&down_up(10));
    expected.extend_from_slice(&down_up(27));
    expected.extend_from_slice(&down_up(1));
    assert_eq!(
        keys, expected,
        "game pane focus must still forward keys, Enter, Escape, and arrows"
    );
    assert!(
        game.iter()
            .any(|ev| matches!(ev, InputEv::Down { button: 1, .. })),
        "mouse Down must still reach the client with the game focused, got {game:?}"
    );
}

/// Hold ArrowLeft over the game, focus Debug search, then rehover the game
/// while that field still owns the keyboard. The real consumer must release
/// the held key and must not restart it when the physical up arrives.
#[test]
fn held_game_key_release_reaches_client_after_panel_focus() {
    let _guard = crate::test_support::imgui_context_guard();
    let mut probe = OwnershipProbe::new();
    probe.hold_arrow();
    probe.ctx.io_mut().add_mouse_pos_event(probe.field);
    probe
        .ctx
        .io_mut()
        .add_mouse_button_event(dear_imgui_rs::MouseButton::Left, true);
    probe.frame("click panel search");
    probe
        .ctx
        .io_mut()
        .add_mouse_button_event(dear_imgui_rs::MouseButton::Left, false);
    probe.frame("panel search mouse up");
    probe.frame("settled panel search");
    probe.ctx.io_mut().add_mouse_pos_event(probe.game);
    assert_eq!(
        probe.frame("game hovered with search focused"),
        (true, true)
    );
    assert_eq!(probe.shell.key_held, [0; 128]);
    probe.key(NamedKey::ArrowLeft, false);
    assert_eq!(probe.frame("physical up with search focused"), (true, true));
    assert_eq!(probe.shell.key_held, [0; 128]);
}

/// Real ImGui Image hover and InputText click transitions, through the
/// production keyboard adapter/channel and SlotInput → GameShell consumer.
struct OwnershipProbe {
    ctx: dear_imgui_rs::Context,
    text: String,
    field: [f32; 2],
    game: [f32; 2],
    input: std::sync::Arc<host::SlotInput>,
    keyboard: KeyboardOwner,
    tx: Option<std::sync::mpsc::Sender<InputEv>>,
    shell: GameShell,
}

impl OwnershipProbe {
    fn new() -> Self {
        discard_unconsumed_native_capture();
        let mut ctx = dear_imgui_rs::Context::create();
        ctx.set_ini_filename(None::<String>).unwrap();
        let input = host::SlotInput::new();
        input.set_enabled(true);
        let (tx, rx) = std::sync::mpsc::channel();
        input.connect_rx(rx);
        let mut probe = Self {
            ctx,
            text: String::new(),
            field: [0.0; 2],
            game: [0.0; 2],
            input,
            tx: Some(tx),
            shell: GameShell::new(),
            keyboard: KeyboardOwner::default(),
        };
        probe.frame("layout 1");
        probe.frame("layout 2");
        probe.ctx.io_mut().add_mouse_pos_event(probe.game);
        probe.frame("hover game");
        probe
    }

    fn frame(&mut self, label: &str) -> (bool, bool) {
        self.ctx.prepare_frame(prepare_opts());
        let ui = self.ctx.frame();
        let captured = ui.io().want_capture_keyboard();
        self.input.set_keyboard_enabled(game_keyboard_available(ui));
        self.keyboard
            .process_ownership(ui, self.tx.as_ref(), Some("game"));
        let mut hovered = false;
        ui.window("Debug")
            .position([0.0, 0.0], dear_imgui_rs::Condition::Always)
            .size([400.0, 200.0], dear_imgui_rs::Condition::Always)
            .build(|| {
                ui.input_text("Search", &mut self.text).build();
                let lo = ui.item_rect_min();
                let hi = ui.item_rect_max();
                self.field = [(lo[0] + hi[0]) / 2.0, (lo[1] + hi[1]) / 2.0];
            });
        ui.window("Game")
            .position([450.0, 0.0], dear_imgui_rs::Condition::Always)
            .size([400.0, 300.0], dear_imgui_rs::Condition::Always)
            .build(|| {
                ui.image(dear_imgui_rs::TextureId::new(0), [250.0, 180.0]);
                let lo = ui.item_rect_min();
                let hi = ui.item_rect_max();
                self.game = [(lo[0] + hi[0]) / 2.0, (lo[1] + hi[1]) / 2.0];
                hovered = ui.is_item_hovered();
                if hovered && self.tx.is_some() {
                    stream_capture(
                        &self.tx,
                        0.0,
                        0.0,
                        765.0,
                        503.0,
                        false,
                        false,
                        false,
                        false,
                        &[],
                    );
                    self.keyboard.capture_keys(ui, self.tx.as_ref());
                }
            });
        discard_unconsumed_native_capture();
        self.ctx.render();
        self.input.drain(&mut self.shell);
        println!(
            "{label}: capture={captured} hovered={hovered} key_held[1]={}",
            self.shell.key_held[1]
        );
        (captured, hovered)
    }

    fn key(&mut self, key: NamedKey, down: bool) {
        window_keyboard(self.ctx.io_mut(), WinitKey::Named(key), down);
    }

    fn hold_arrow(&mut self) {
        self.key(NamedKey::ArrowLeft, true);
        assert_eq!(self.frame("ArrowLeft down"), (false, true));
        assert_eq!(self.shell.key_held[1], 1);
    }

    /// Ownership runs before the Game/grid/rail/chooser windows. Do not
    /// drain the old slot here: its disabled wake tick already ran.
    fn ownership_frame(&mut self, target: &str) {
        self.ctx.prepare_frame(prepare_opts());
        let ui = self.ctx.frame();
        self.input.set_keyboard_enabled(game_keyboard_available(ui));
        self.keyboard
            .process_ownership(ui, self.tx.as_ref(), Some(target));
        discard_unconsumed_native_capture();
        self.ctx.render();
    }
}

#[test]
fn held_game_key_releases_through_offhover_panel_focus_cycle() {
    let _guard = crate::test_support::imgui_context_guard();
    let mut probe = OwnershipProbe::new();
    probe.hold_arrow();
    probe.ctx.io_mut().add_mouse_pos_event(probe.field);
    probe
        .ctx
        .io_mut()
        .add_mouse_button_event(dear_imgui_rs::MouseButton::Left, true);
    probe.frame("click field");
    probe
        .ctx
        .io_mut()
        .add_mouse_button_event(dear_imgui_rs::MouseButton::Left, false);
    probe.frame("field mouse up");
    assert_eq!(probe.frame("settled panel capture"), (true, false));
    probe.key(NamedKey::ArrowLeft, false);
    assert_eq!(probe.frame("ArrowLeft release over field"), (true, false));
    probe.ctx.io_mut().add_key_event(Key::Escape, true);
    probe.key(NamedKey::Escape, true);
    probe.frame("Escape panel focus");
    probe.ctx.io_mut().add_key_event(Key::Escape, false);
    probe.key(NamedKey::Escape, false);
    probe.frame("Escape up");
    assert_eq!(probe.frame("focus settled off"), (false, false));
    probe.ctx.io_mut().add_mouse_pos_event(probe.game);
    assert_eq!(probe.frame("rehover game after focus ended"), (false, true));
    assert_eq!(
        probe.shell.key_held[1], 0,
        "ArrowLeft must be released after the offhover panel focus cycle"
    );
}

#[test]
fn held_game_key_releases_on_window_focus_loss() {
    let _guard = crate::test_support::imgui_context_guard();
    let mut probe = OwnershipProbe::new();
    probe.hold_arrow();
    probe.ctx.io_mut().add_focus_event(false);
    probe.ctx.io_mut().add_mouse_pos_event([-100.0, -100.0]);
    assert!(!probe.frame("window focus lost without native key-up").1);
    assert_eq!(
        probe.shell.key_held[1], 0,
        "losing window focus must release ArrowLeft without a native key-up"
    );
}

#[test]
fn held_game_key_releases_when_capture_turns_off() {
    let _guard = crate::test_support::imgui_context_guard();
    let mut probe = OwnershipProbe::new();
    probe.hold_arrow();
    probe.input.set_enabled(false);
    probe.tx = None;
    probe.frame("capture off without native key-up");
    assert_eq!(probe.shell.key_held[1], 0);
}

#[test]
fn offhover_release_keeps_shifted_ownership_when_other_input_is_discarded() {
    let _guard = crate::test_support::imgui_context_guard();
    let mut probe = OwnershipProbe::new();
    probe.hold_arrow();
    window_keyboard(probe.ctx.io_mut(), WinitKey::Character(":".into()), true);
    probe.frame("shifted colon down");
    assert_eq!(probe.shell.key_held[58], 1);
    assert_eq!(probe.shell.poll_key(), 58);

    // No field activation: hover alone must not revoke ArrowLeft, but
    // unhovered new text must not reach the game or destroy colon ownership.
    probe.ctx.io_mut().add_mouse_pos_event(probe.field);
    tap_character(probe.ctx.io_mut(), "x");
    assert_eq!(
        probe.frame("discard unrelated offhover input"),
        (false, false)
    );
    assert_eq!(probe.shell.key_held[1], 1);
    window_keyboard(probe.ctx.io_mut(), WinitKey::Character(";".into()), false);
    probe.key(NamedKey::ArrowLeft, false);
    assert_eq!(probe.frame("offhover native releases"), (false, false));
    assert_eq!(probe.shell.key_held[1], 0);
    assert_eq!(probe.shell.key_held[58], 0);
    assert_eq!(probe.shell.key_held[59], 0);
    probe.ctx.io_mut().add_mouse_pos_event(probe.game);
    probe.frame("rehover without replay");
    assert_eq!(probe.shell.poll_key(), -1);
}

#[test]
fn reconnecting_capture_for_same_slot_keeps_keys_on_current_receiver() {
    let _guard = crate::test_support::imgui_context_guard();
    let mut probe = OwnershipProbe::new();
    probe.hold_arrow();
    let (tx, rx) = std::sync::mpsc::channel();
    probe.input.connect_rx(rx);
    probe.tx = Some(tx);
    probe.ctx.io_mut().add_mouse_pos_event(probe.field);
    probe.key(NamedKey::ArrowLeft, false);
    probe.frame("release after same-slot channel reconnect");
    assert_eq!(probe.shell.key_held[1], 0);

    probe.ctx.io_mut().add_mouse_pos_event(probe.game);
    probe.key(NamedKey::ArrowRight, true);
    probe.frame("new press after reconnect");
    assert_eq!(probe.shell.key_held[2], 1);
    probe.key(NamedKey::ArrowRight, false);
    probe.frame("release new press after reconnect");
    assert_eq!(probe.shell.key_held[2], 0);
}

#[test]
fn reselect_after_disabled_wake_does_not_lose_held_key_release() {
    let _guard = crate::test_support::imgui_context_guard();
    let mut probe = OwnershipProbe::new();
    probe.hold_arrow();
    // The rail/grid/chooser select occurs after this frame's ownership pass.
    probe.ownership_frame("game");
    probe.input.set_enabled(false);
    probe.input.drain(&mut probe.shell); // outgoing slot's immediate wake
    println!(
        "reselect disabled wake: alice[1]={}",
        probe.shell.key_held[1]
    );

    let bob = host::SlotInput::new();
    let (bob_tx, bob_rx) = std::sync::mpsc::channel();
    bob.connect_rx(bob_rx);
    bob.set_enabled(true);
    probe.tx = Some(bob_tx);
    probe.ownership_frame("bob"); // R3 queues alice's up after its wake

    let (alice_tx, alice_rx) = std::sync::mpsc::channel();
    probe.input.connect_rx(alice_rx); // reselect drops the old queued up
    probe.input.set_enabled(true);
    probe.tx = Some(alice_tx);
    probe.key(NamedKey::ArrowLeft, false);
    probe.ownership_frame("game");
    probe.input.drain(&mut probe.shell);
    let mut bob_shell = GameShell::new();
    bob.drain(&mut bob_shell);
    println!(
        "reselect and physical up: alice[1]={} bob[1]={}",
        probe.shell.key_held[1], bob_shell.key_held[1]
    );
    assert_eq!(probe.shell.key_held, [0; 128]);
    assert_eq!(bob_shell.key_held, [0; 128]);
}

#[test]
fn capture_reenable_after_disabled_wake_does_not_lose_held_key_release() {
    let _guard = crate::test_support::imgui_context_guard();
    let mut probe = OwnershipProbe::new();
    probe.hold_arrow();
    probe.input.set_enabled(false);
    probe.tx = None;
    probe.input.drain(&mut probe.shell); // wake precedes the ownership pass
    println!(
        "capture disabled wake: key_held[1]={}",
        probe.shell.key_held[1]
    );
    probe.ownership_frame("game"); // R3 queues an up after the disabled drain

    let (tx, rx) = std::sync::mpsc::channel();
    probe.input.connect_rx(rx); // capture_on replaces the undrained receiver
    probe.input.set_enabled(true);
    probe.tx = Some(tx);
    probe.key(NamedKey::ArrowLeft, false);
    probe.ownership_frame("game");
    probe.input.drain(&mut probe.shell);
    println!(
        "capture reenable and physical up: key_held[1]={}",
        probe.shell.key_held[1]
    );
    assert_eq!(probe.shell.key_held, [0; 128]);
}
