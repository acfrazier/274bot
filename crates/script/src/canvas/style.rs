use std::sync::OnceLock;
use ab_glyph::{Font, FontRef, PxScale, ScaleFont};
use super::MAX_FONT_PX;

const SANS_BYTES: &[u8] = include_bytes!("../../fonts/LiberationSans-Regular.ttf");
const MONO_BYTES: &[u8] = include_bytes!("../../fonts/LiberationMono-Regular.ttf");

static SANS: OnceLock<FontRef<'static>> = OnceLock::new();
static MONO: OnceLock<FontRef<'static>> = OnceLock::new();

fn sans_font() -> &'static FontRef<'static> {
    SANS.get_or_init(|| FontRef::try_from_slice(SANS_BYTES).expect("Liberation Sans Regular"))
}

fn mono_font() -> &'static FontRef<'static> {
    MONO.get_or_init(|| FontRef::try_from_slice(MONO_BYTES).expect("Liberation Mono Regular"))
}

pub(crate) fn font_for(mono: bool) -> &'static FontRef<'static> {
    if mono {
        mono_font()
    } else {
        sans_font()
    }
}
/// Same metrics as [`measure_text`] for a concrete face/size (tests / panel).
pub fn measure_with(font_px: u16, mono: bool, text: &str) -> f64 {
    let font_px = font_px.clamp(1, MAX_FONT_PX);
    let font = font_for(mono);
    let scale = PxScale::from(font_px as f32);
    let scaled = font.as_scaled(scale);
    let mut width = 0.0f32;
    for ch in text.chars() {
        width += scaled.h_advance(font.glyph_id(ch));
    }
    width as f64
}
pub fn pack_rgba(r: u8, g: u8, b: u8, a: u8) -> u32 {
    (u32::from(r) << 24) | (u32::from(g) << 16) | (u32::from(b) << 8) | u32::from(a)
}

pub fn unpack_rgba(color: u32) -> [u8; 4] {
    [
        (color >> 24) as u8,
        (color >> 16) as u8,
        (color >> 8) as u8,
        color as u8,
    ]
}

pub fn parse_color(input: &str) -> Option<u32> {
    let s = input.trim();
    if s.eq_ignore_ascii_case("transparent") {
        return Some(pack_rgba(0, 0, 0, 0));
    }
    if let Some(hex) = s.strip_prefix('#') {
        match hex.len() {
            3 => {
                let n = u32::from_str_radix(hex, 16).ok()?;
                let r = (((n >> 8) & 0xf) * 0x11) as u8;
                let g = (((n >> 4) & 0xf) * 0x11) as u8;
                let b = ((n & 0xf) * 0x11) as u8;
                Some(pack_rgba(r, g, b, 255))
            }
            6 => {
                let n = u32::from_str_radix(hex, 16).ok()?;
                Some((n << 8) | 0xff)
            }
            8 => u32::from_str_radix(hex, 16).ok(),
            _ => None,
        }
    } else if let Some(inner) = s.strip_prefix("rgba(").and_then(|t| t.strip_suffix(')')) {
        parse_rgb_inner(inner, true)
    } else if let Some(inner) = s.strip_prefix("rgb(").and_then(|t| t.strip_suffix(')')) {
        parse_rgb_inner(inner, false)
    } else {
        None
    }
}

fn parse_rgb_inner(inner: &str, with_alpha: bool) -> Option<u32> {
    let mut parts = inner.split(',').map(|p| p.trim());
    let r = parse_rgb_channel(parts.next()?)?;
    let g = parse_rgb_channel(parts.next()?)?;
    let b = parse_rgb_channel(parts.next()?)?;
    let a = if with_alpha {
        let a = parts.next()?.parse::<f64>().ok()?;
        if !a.is_finite() {
            return None;
        }
        (a.clamp(0.0, 1.0) * 255.0).round() as u8
    } else {
        if parts.next().is_some() {
            return None;
        }
        255
    };
    if parts.next().is_some() {
        return None;
    }
    Some(pack_rgba(r, g, b, a))
}

fn parse_rgb_channel(s: &str) -> Option<u8> {
    let n: f64 = s.parse().ok()?;
    if !n.is_finite() {
        return None;
    }
    Some(n.round().clamp(0.0, 255.0) as u8)
}

/// Parse CSS font shorthand enough for `12px monospace` / `10px sans-serif`.
pub fn parse_font(input: &str) -> Option<(u16, bool)> {
    let s = input.trim();
    let px = s.find("px")?;
    let mut start = px;
    let bytes = s.as_bytes();
    while start > 0 {
        let c = bytes[start - 1];
        if c.is_ascii_digit() || c == b'.' {
            start -= 1;
        } else {
            break;
        }
    }
    if start == px {
        return None;
    }
    let size: f64 = s[start..px].parse().ok()?;
    if !size.is_finite() || size <= 0.0 || size > f64::from(MAX_FONT_PX) {
        return None;
    }
    let family = s[px + 2..]
        .trim()
        .trim_matches(|c| c == '"' || c == '\'')
        .to_ascii_lowercase();
    let mono = family == "monospace"
        || family == "courier"
        || family == "courier new"
        || family.contains("liberation mono");
    let sans = family == "sans-serif"
        || family == "sans"
        || family == "arial"
        || family == "helvetica"
        || family.contains("liberation sans");
    if !mono && !sans {
        return None;
    }
    Some((size.round() as u16, mono))
}
/// True when a decoded/recorded font size is inside the parser cap.
pub fn font_px_allowed(font_px: u16) -> bool {
    font_px > 0 && font_px <= MAX_FONT_PX
}
