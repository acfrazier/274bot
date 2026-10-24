//! Game input capture: native keyboard queue, ImGui shifted-key adapter,
//! hover-independent key ownership, and hovered new-input streaming.

use std::sync::{Arc, Mutex};

use dear_imgui_rs::{Io, Key, Ui};
use frontend_core::surface::{map_image_to_applet, InputEv, InputSender, SlotInput};
use frontend_core::{GameInputEvent, MapDesk};
use winit::keyboard::{Key as WinitKey, KeyLocation, NamedKey};

/// Named GameShell `ch` values (arrows 1–4, ASCII controls, space).
const CAPTURE_NAMED: &[(Key, i32)] = &[
    (Key::LeftArrow, 1),
    (Key::RightArrow, 2),
    (Key::UpArrow, 3),
    (Key::DownArrow, 4),
    (Key::Backspace, 8),
    (Key::Delete, 8),
    (Key::Tab, 9),
    (Key::Enter, 10),
    (Key::Escape, 27),
    (Key::Space, 32),
];
const CAPTURE_LETTERS: [Key; 26] = [
    Key::A,
    Key::B,
    Key::C,
    Key::D,
    Key::E,
    Key::F,
    Key::G,
    Key::H,
    Key::I,
    Key::J,
    Key::K,
    Key::L,
    Key::M,
    Key::N,
    Key::O,
    Key::P,
    Key::Q,
    Key::R,
    Key::S,
    Key::T,
    Key::U,
    Key::V,
    Key::W,
    Key::X,
    Key::Y,
    Key::Z,
];
const CAPTURE_DIGITS: [(Key, u8, u8); 10] = [
    (Key::Key0, b'0', b')'),
    (Key::Key1, b'1', b'!'),
    (Key::Key2, b'2', b'@'),
    (Key::Key3, b'3', b'#'),
    (Key::Key4, b'4', b'$'),
    (Key::Key5, b'5', b'%'),
    (Key::Key6, b'6', b'^'),
    (Key::Key7, b'7', b'&'),
    (Key::Key8, b'8', b'*'),
    (Key::Key9, b'9', b'('),
];
/// Punctuation: same `ch` as client-play `key_codes::lookup` / KeyCodes.ts
/// 48–80. Without this, `::` (`:`) and `~` never reach chat.
const CAPTURE_PUNCT: [(Key, u8, u8); 11] = [
    (Key::GraveAccent, b'`', b'~'),
    (Key::Minus, b'-', b'_'),
    (Key::Equal, b'=', b'+'),
    (Key::LeftBracket, b'[', b'{'),
    (Key::RightBracket, b']', b'}'),
    (Key::Backslash, b'\\', b'|'),
    (Key::Semicolon, b';', b':'),
    (Key::Apostrophe, b'\'', b'"'),
    (Key::Comma, b',', b'<'),
    (Key::Period, b'.', b'>'),
    (Key::Slash, b'/', b'?'),
];

/// Recover the physical ImGui key for shifted printable characters. Winit's
/// logical key is the produced character (`:` rather than `;`). Game capture
/// records that produced character at the native event; this mapping is only
/// the ImGui key identity plus press-character ownership, not a later Shift
/// sample. The winit backend already queues `event.text` for text widgets.
pub(crate) fn shifted_imgui_key(key: &WinitKey) -> Option<Key> {
    let WinitKey::Character(character) = key else {
        return None;
    };
    match character.as_str() {
        ")" => Some(Key::Key0),
        "!" => Some(Key::Key1),
        "@" => Some(Key::Key2),
        "#" => Some(Key::Key3),
        "$" => Some(Key::Key4),
        "%" => Some(Key::Key5),
        "^" => Some(Key::Key6),
        "&" => Some(Key::Key7),
        "*" => Some(Key::Key8),
        "(" => Some(Key::Key9),
        "~" => Some(Key::GraveAccent),
        "_" => Some(Key::Minus),
        "+" => Some(Key::Equal),
        "{" => Some(Key::LeftBracket),
        "}" => Some(Key::RightBracket),
        "|" => Some(Key::Backslash),
        ":" => Some(Key::Semicolon),
        "\"" => Some(Key::Apostrophe),
        "<" => Some(Key::Comma),
        ">" => Some(Key::Period),
        "?" => Some(Key::Slash),
        _ => None,
    }
}

/// Apply the adapter only where dear-imgui-winit does not already own the
/// physical key. Its backend maps numpad `+`/`*` to keypad keys; translating
/// those same logical characters to `Equal`/`Key8` would double-deliver them
/// to game capture.
pub(crate) fn shifted_imgui_key_at_location(key: &WinitKey, location: KeyLocation) -> Option<Key> {
    if location == KeyLocation::Numpad {
        return None;
    }
    shifted_imgui_key(key)
}

/// Queue the missing ImGui key lifecycle for a logical shifted character,
/// and record the produced capture character (or named GameShell ch) at
/// this native event so mixed printable/named order is preserved.
/// The backend already queues text for widgets; this must not add text.
pub(crate) fn add_shifted_key_event(
    io: &mut Io,
    logical_key: &WinitKey,
    location: KeyLocation,
    down: bool,
) {
    if let Some(key) = shifted_imgui_key_at_location(logical_key, location) {
        io.add_key_event(key, down);
    }
    note_native_capture_key(logical_key, location, down);
}

static NATIVE_CAPTURE: Mutex<Vec<(bool, i32)>> = Mutex::new(Vec::new());
static NATIVE_PRESS_CH: Mutex<Vec<(Key, i32)>> = Mutex::new(Vec::new());

fn physical_capture_key(logical_key: &WinitKey, location: KeyLocation) -> Option<Key> {
    if let Some(key) = shifted_imgui_key_at_location(logical_key, location) {
        return Some(key);
    }
    let WinitKey::Character(character) = logical_key else {
        return None;
    };
    let mut chars = character.chars();
    let ch = chars.next()?;
    if chars.next().is_some() {
        return None;
    }
    if location == KeyLocation::Numpad {
        if ch.is_ascii_digit() {
            return Some(CAPTURE_DIGITS[(ch as u8 - b'0') as usize].0);
        }
        return None;
    }
    if ch.is_ascii_alphabetic() {
        let i = (ch.to_ascii_uppercase() as u8 - b'A') as usize;
        return CAPTURE_LETTERS.get(i).copied();
    }
    for &(key, unshifted, shifted) in &CAPTURE_DIGITS {
        if ch == unshifted as char || ch == shifted as char {
            return Some(key);
        }
    }
    for &(key, unshifted, shifted) in &CAPTURE_PUNCT {
        if ch == unshifted as char || ch == shifted as char {
            return Some(key);
        }
    }
    None
}

fn produced_capture_ch(logical_key: &WinitKey, location: KeyLocation) -> Option<i32> {
    let WinitKey::Character(character) = logical_key else {
        return None;
    };
    let mut chars = character.chars();
    let ch = chars.next()?;
    if chars.next().is_some() || !ch.is_ascii() {
        return None;
    }
    physical_capture_key(logical_key, location)?;
    Some(ch as i32)
}

fn imgui_named_capture_key(logical_key: &WinitKey, location: KeyLocation) -> Option<Key> {
    match logical_key {
        WinitKey::Character(s) if s.as_str() == " " => Some(Key::Space),
        WinitKey::Named(named) => match named {
            NamedKey::ArrowLeft => Some(Key::LeftArrow),
            NamedKey::ArrowRight => Some(Key::RightArrow),
            NamedKey::ArrowUp => Some(Key::UpArrow),
            NamedKey::ArrowDown => Some(Key::DownArrow),
            NamedKey::Backspace => Some(Key::Backspace),
            NamedKey::Delete => Some(Key::Delete),
            NamedKey::Tab => Some(Key::Tab),
            NamedKey::Enter if location != KeyLocation::Numpad => Some(Key::Enter),
            NamedKey::Escape => Some(Key::Escape),
            NamedKey::Space => Some(Key::Space),
            _ => None,
        },
        _ => None,
    }
}

fn named_capture_ch(logical_key: &WinitKey, location: KeyLocation) -> Option<i32> {
    let key = imgui_named_capture_key(logical_key, location)?;
    CAPTURE_NAMED
        .iter()
        .find(|(k, _)| *k == key)
        .map(|&(_, ch)| ch)
}

fn note_native_capture_key(logical_key: &WinitKey, location: KeyLocation, down: bool) {
    let ch = if let Some(produced) = produced_capture_ch(logical_key, location) {
        match physical_capture_key(logical_key, location) {
            Some(key) if down => {
                let mut held = NATIVE_PRESS_CH.lock().expect("native press ch");
                held.retain(|(k, _)| *k != key);
                held.push((key, produced));
                produced
            }
            Some(key) => {
                let mut held = NATIVE_PRESS_CH.lock().expect("native press ch");
                if let Some(index) = held.iter().position(|(k, _)| *k == key) {
                    held.remove(index).1
                } else {
                    produced
                }
            }
            None => produced,
        }
    } else if let Some(ch) = named_capture_ch(logical_key, location) {
        ch
    } else {
        return;
    };
    NATIVE_CAPTURE
        .lock()
        .expect("native capture")
        .push((down, ch));
}

/// Drop new input not consumed by a hovered Game Image. Native press-character
/// identities live until their matching release (or next press), even when
/// unrelated panel input is discarded between a shifted down and its up.
pub(crate) fn discard_unconsumed_native_capture() {
    let mut queued = NATIVE_CAPTURE.lock().expect("native capture");
    if queued.is_empty() {
        return;
    }
    queued.clear();
}

/// GameShell `ch` for one ImGui key, Shift applied the way client-play
/// maps DOM `KeyboardEvent.key` (`:` is 58, `~` is 126).
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "test-only key mapping helper; exercised by input_capture_tests"
    )
)]
pub(crate) fn capture_key_ch(key: Key, shift: bool) -> Option<i32> {
    for &(k, ch) in CAPTURE_NAMED {
        if k == key {
            return Some(ch);
        }
    }
    for (i, &k) in CAPTURE_LETTERS.iter().enumerate() {
        if k == key {
            let base = if shift { b'A' } else { b'a' };
            return Some((base + i as u8) as i32);
        }
    }
    for &(k, unshifted, shifted) in &CAPTURE_DIGITS {
        if k == key {
            return Some(if shift { shifted } else { unshifted } as i32);
        }
    }
    for &(k, unshifted, shifted) in &CAPTURE_PUNCT {
        if k == key {
            return Some(if shift { shifted } else { unshifted } as i32);
        }
    }
    None
}

/// Keys belong to the slot that received their downs, not to the pointer.
/// One instance lives at the frame owner, outside either Game-pane layout.
pub(crate) struct KeyboardOwner {
    down: [bool; 128],
    held_count: usize,
    target: Option<String>,
}

impl Default for KeyboardOwner {
    fn default() -> Self {
        Self {
            down: [false; 128],
            held_count: 0,
            target: None,
        }
    }
}

pub(crate) fn game_keyboard_available(ui: &Ui) -> bool {
    // SAFETY: called on the UI thread during the active Ui frame, so the
    // current ImGui context and its IO are alive. Copy the platform focus
    // flag immediately; no raw reference survives another ImGui call.
    let focus_lost = unsafe { (*dear_imgui_rs::sys::igGetIO_Nil()).AppFocusLost };
    !focus_lost && !ui.io().want_capture_keyboard()
}

impl KeyboardOwner {
    /// Every frame, before any Image-hover gate, pair physical ups for the
    /// still-attached slot. Detach releases are authoritative in SlotInput,
    /// not synthetic events sent to a receiver that may already be gone.
    pub(crate) fn process_ownership(
        &mut self,
        ui: &Ui,
        map_desk: &mut MapDesk,
        target: Option<&str>,
    ) {
        let connected = map_desk.input_connected();
        let changed = self.target.as_deref() != target;
        if self.held_count != 0 && (changed || !connected || !game_keyboard_available(ui)) {
            self.down.fill(false);
            self.held_count = 0;
        }
        let mut queued = NATIVE_CAPTURE.lock().expect("native capture");
        queued.retain(|&(down, ch)| {
            if !down && self.down[ch as usize] {
                if !map_desk
                    .stage_game_input(GameInputEvent::Captured(InputEv::Key { down: false, ch }))
                {
                    return true;
                }
                self.down[ch as usize] = false;
                self.held_count -= 1;
                false
            } else {
                true
            }
        });

        if !connected {
            self.target = None;
        } else if changed {
            self.target = target.map(str::to_owned);
        }
    }

    /// Only called for a hovered Game Image. New downs/text remain gated by
    /// panel capture and OS focus. Same-frame down/up bursts retain native
    /// event order and the character recorded at the press, not later Shift.
    /// If GameInput fills, the unconsumed native tail stays queued for the
    /// next frame.
    pub(crate) fn capture_keys(&mut self, ui: &Ui, map_desk: &mut MapDesk) {
        if !game_keyboard_available(ui) || !map_desk.input_connected() {
            return;
        }
        let mut queued = NATIVE_CAPTURE.lock().expect("native capture");
        let mut consumed = 0;
        while consumed < queued.len() {
            let (down, ch) = queued[consumed];
            let index = ch as usize;
            let held = self.down[index];
            if !down && !held {
                consumed += 1;
                continue;
            }
            if !map_desk.stage_game_input(GameInputEvent::Captured(InputEv::Key { down, ch })) {
                break;
            }
            if down {
                if !held {
                    self.held_count += 1;
                }
                self.down[index] = true;
            } else {
                self.held_count -= 1;
                self.down[index] = false;
            }
            consumed += 1;
        }
        queued.drain(..consumed).for_each(drop);
    }
}

/// Where one mouse button started, including the exact slot input channel.
enum PressOrigin {
    Unowned,
    Paint,
    Game(GameMouseOwner),
}

struct GameMouseOwner {
    slot: String,
    input: Arc<SlotInput>,
    sender: InputSender,
}

/// The slot that can receive a new game press this frame.
#[derive(Clone, Copy)]
pub(crate) struct MouseTarget<'a> {
    pub(crate) slot: &'a str,
    pub(crate) input: &'a Arc<SlotInput>,
}

/// One frame of pointer and mouse-button state for the shared gate.
pub(crate) struct MouseFrame<'a> {
    pub(crate) target: Option<MouseTarget<'a>>,
    pub(crate) sender: Option<InputSender>,
    pub(crate) image_hovered: bool,
    pub(crate) pointer_in_paint: bool,
    pub(crate) applet_point: Option<(i32, i32)>,
    pub(crate) buttons: MouseButtons,
}

/// One frame's mouse-button edges.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MouseButtons {
    pub(crate) left_down: bool,
    pub(crate) right_down: bool,
    pub(crate) left_up: bool,
    pub(crate) right_up: bool,
}
#[derive(Clone, Copy)]
struct MouseButtonInput {
    button: i32,
    down: bool,
    up: bool,
}

/// Routes each mouse button to the surface and slot that received its press.
/// One gate is shared by both Game panes. It does not use release position
/// to select an owner.
#[derive(Default)]
pub(crate) struct PaintMouseGate {
    left: Option<PressOrigin>,
    right: Option<PressOrigin>,
}

impl PaintMouseGate {
    /// Stage button events for the slot that received each press.
    ///
    /// A slot change ends the old slot's hold on this frame. A physical
    /// release ends the hold when the slot stays the same.
    pub(crate) fn route(
        &mut self,
        map_desk: &mut MapDesk,
        frame: MouseFrame<'_>,
        mut slot_is_live: impl FnMut(&str, &Arc<SlotInput>) -> bool,
    ) {
        let left_cancelled = Self::take_changed(&mut self.left, frame.target);
        let right_cancelled = Self::take_changed(&mut self.right, frame.target);
        let cancelled = [left_cancelled, right_cancelled];
        for (index, owner) in cancelled.iter().enumerate() {
            let Some(owner) = owner.as_ref() else {
                continue;
            };
            if !slot_is_live(&owner.slot, &owner.input) {
                continue;
            }
            let already_released = cancelled[..index]
                .iter()
                .flatten()
                .any(|other| Self::same_owner(other, owner));
            if !already_released {
                let _ = map_desk.stage_captured_input_to(&owner.sender, InputEv::Up);
            }
        }

        Self::route_button(
            map_desk,
            &mut self.left,
            MouseButtonInput {
                button: 1,
                down: frame.buttons.left_down,
                up: frame.buttons.left_up,
            },
            &frame,
            &mut slot_is_live,
        );
        Self::route_button(
            map_desk,
            &mut self.right,
            MouseButtonInput {
                button: 2,
                down: frame.buttons.right_down,
                up: frame.buttons.right_up,
            },
            &frame,
            &mut slot_is_live,
        );
    }

    /// Forget presses after capture or the pane closes. The host input
    /// detach releases any button that it still holds.
    pub(crate) fn clear(&mut self) {
        self.left = None;
        self.right = None;
    }

    fn take_changed(
        origin: &mut Option<PressOrigin>,
        target: Option<MouseTarget<'_>>,
    ) -> Option<GameMouseOwner> {
        let changed = matches!(
            origin.as_ref(),
            Some(PressOrigin::Game(owner)) if !Self::matches_target(owner, target)
        );
        if changed {
            match origin.take() {
                Some(PressOrigin::Game(owner)) => Some(owner),
                _ => None,
            }
        } else {
            None
        }
    }

    fn route_button(
        map_desk: &mut MapDesk,
        origin: &mut Option<PressOrigin>,
        button: MouseButtonInput,
        frame: &MouseFrame<'_>,
        slot_is_live: &mut impl FnMut(&str, &Arc<SlotInput>) -> bool,
    ) {
        if button.down {
            if let Some(PressOrigin::Game(owner)) = origin.take() {
                if slot_is_live(&owner.slot, &owner.input) {
                    let _ = map_desk.stage_captured_input_to(&owner.sender, InputEv::Up);
                }
            }
            *origin = if frame.pointer_in_paint {
                Some(PressOrigin::Paint)
            } else if frame.image_hovered {
                match (frame.target, frame.sender.as_ref(), frame.applet_point) {
                    (Some(target), Some(sender), Some((x, y))) => {
                        let owner = GameMouseOwner {
                            slot: target.slot.to_owned(),
                            input: Arc::clone(target.input),
                            sender: sender.duplicate(),
                        };
                        let staged = map_desk.stage_captured_input_to(
                            &owner.sender,
                            InputEv::Down {
                                button: button.button,
                                x,
                                y,
                            },
                        );
                        Some(if staged {
                            PressOrigin::Game(owner)
                        } else {
                            PressOrigin::Unowned
                        })
                    }
                    _ => Some(PressOrigin::Unowned),
                }
            } else {
                Some(PressOrigin::Unowned)
            };
        }
        if button.up {
            if let Some(PressOrigin::Game(owner)) = origin.take() {
                if slot_is_live(&owner.slot, &owner.input) {
                    let _ = map_desk.stage_captured_input_to(&owner.sender, InputEv::Up);
                }
            } else {
                *origin = None;
            }
        }
    }

    fn matches_target(owner: &GameMouseOwner, target: Option<MouseTarget<'_>>) -> bool {
        target.is_some_and(|target| {
            owner.slot == target.slot && Arc::ptr_eq(&owner.input, target.input)
        })
    }

    fn same_owner(left: &GameMouseOwner, right: &GameMouseOwner) -> bool {
        left.slot == right.slot && Arc::ptr_eq(&left.input, &right.input)
    }
}

/// Click-through helper: maps a click inside the Game Image (local coords,
/// Image widget size) to applet coords and stages `InputEv::Down`.
pub fn maybe_send_click(map_desk: &mut MapDesk, lx: f32, ly: f32, w: f32, h: f32) {
    if !map_desk.input_connected() {
        return;
    }
    let Some((x, y)) = map_image_to_applet(lx, ly, w, h) else {
        return;
    };
    let _ = map_desk.stage_game_input(GameInputEvent::Captured(InputEv::Down { button: 1, x, y }));
}

/// Stream one hovered capture frame: `Move` first, then `Down` (left=1,
/// right=2), then `Up`, then keys. No-op when capture is off.
// All capture state arrives flattened from the applet; a param struct would
// only shuffle names across the one call site.
#[allow(clippy::too_many_arguments)]
pub fn stream_capture(
    map_desk: &mut MapDesk,
    lx: f32,
    ly: f32,
    w: f32,
    h: f32,
    left_down: bool,
    right_down: bool,
    left_up: bool,
    right_up: bool,
    keys: &[(bool, i32)],
) {
    if !map_desk.input_connected() {
        return;
    }
    if let Some((x, y)) = map_image_to_applet(lx, ly, w, h) {
        let _ = map_desk.stage_game_input(GameInputEvent::Captured(InputEv::Move { x, y }));
        if left_down {
            let _ = map_desk.stage_game_input(GameInputEvent::Captured(InputEv::Down {
                button: 1,
                x,
                y,
            }));
        }
        if right_down {
            let _ = map_desk.stage_game_input(GameInputEvent::Captured(InputEv::Down {
                button: 2,
                x,
                y,
            }));
        }
    }
    if left_up || right_up {
        let _ = map_desk.stage_game_input(GameInputEvent::Captured(InputEv::Up));
    }
    for &(down, ch) in keys {
        let _ = map_desk.stage_game_input(GameInputEvent::Captured(InputEv::Key { down, ch }));
    }
}

#[cfg(test)]
#[path = "input_capture_tests.rs"]
mod tests;
