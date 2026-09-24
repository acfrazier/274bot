//! Game input capture: native keyboard queue, ImGui shifted-key adapter,
//! and hovered-frame mouse/key streaming to slot channels.

use std::sync::mpsc::Sender;
use std::sync::Mutex;

use dear_imgui_rs::{Io, Key, Ui};
use host::{map_image_to_applet, InputEv};
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

fn take_native_capture() -> Vec<(bool, i32)> {
    std::mem::take(&mut *NATIVE_CAPTURE.lock().expect("native capture"))
}

/// Drop unconsumed capture so capture-off / unhovered frames cannot
/// replay later. Held press-character ownership is cleared only when
/// events were actually discarded: an empty queue after a drained press
/// must keep ownership so a later Shift-up release still pairs. Clearing
/// held on every empty capture-off frame would rewrite `:` into `;` if
/// the cursor left the pane between press and release.
pub(crate) fn discard_unconsumed_native_capture() {
    let mut queued = NATIVE_CAPTURE.lock().expect("native capture");
    if queued.is_empty() {
        return;
    }
    queued.clear();
    NATIVE_PRESS_CH.lock().expect("native press ch").clear();
}

/// GameShell `ch` for one ImGui key, Shift applied the way client-play
/// maps DOM `KeyboardEvent.key` (`:` is 58, `~` is 126).
#[cfg_attr(not(test), allow(dead_code))]
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

/// Map hovered keys to GameShell `ch` values (arrows 1–4, ASCII).
/// Printable and named capture keys share one native event queue so a
/// same-frame `a`/Space/`b` stays `a b`. Produced printables are the
/// characters recorded at KeyboardInput so a later Shift sample cannot
/// rewrite `:` into `;`.
pub(crate) fn capture_keys(_ui: &Ui) -> Vec<(bool, i32)> {
    take_native_capture()
}

/// Click-through helper: maps a click inside the Game Image (local coords,
/// Image widget size) to applet coords and enqueues `InputEv::Down`. No-op
/// when the capture channel has been dropped (capture off) or the point is
/// outside the Image.
pub fn maybe_send_click(tx: &Option<Sender<InputEv>>, lx: f32, ly: f32, w: f32, h: f32) {
    let Some(tx) = tx else {
        return;
    };
    let Some((x, y)) = map_image_to_applet(lx, ly, w, h) else {
        return;
    };
    let _ = tx.send(InputEv::Down { button: 1, x, y });
}

/// Stream one hovered capture frame: `Move` first, then `Down` (left=1,
/// right=2), then `Up`, then keys. No-op when `tx` is `None` (capture off).
// All capture state arrives flattened from the applet; a param struct would
// only shuffle names across the one call site.
#[allow(clippy::too_many_arguments)]
pub fn stream_capture(
    tx: &Option<Sender<InputEv>>,
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
    let Some(tx) = tx else {
        return;
    };
    if let Some((x, y)) = map_image_to_applet(lx, ly, w, h) {
        let _ = tx.send(InputEv::Move { x, y });
        if left_down {
            let _ = tx.send(InputEv::Down { button: 1, x, y });
        }
        if right_down {
            let _ = tx.send(InputEv::Down { button: 2, x, y });
        }
    }
    if left_up || right_up {
        let _ = tx.send(InputEv::Up);
    }
    for &(down, ch) in keys {
        let _ = tx.send(InputEv::Key { down, ch });
    }
}

#[cfg(test)]
#[path = "input_capture_tests.rs"]
mod tests;
