//! Cell-width text helpers shared by the panes.

use std::borrow::Cow;

use ratatui::text::Span;

/// Display cells `text` takes, by the width function ratatui draws with.
pub fn cells(text: &str) -> usize {
    Span::raw(text).width()
}

/// `text` cut to at most `max` display cells, ending in `…` when it was cut.
/// The cut falls between characters by their cell width, so a wide character
/// is never split across the last column. Borrowed when the text fits.
pub fn ellipsize(text: &str, max: usize) -> Cow<'_, str> {
    if cells(text) <= max {
        return Cow::Borrowed(text);
    }
    let mut out = String::new();
    // `…` takes one cell.
    let room = max.saturating_sub(1);
    let mut used = 0;
    let mut buf = [0u8; 4];
    for ch in text.chars() {
        let width = cells(ch.encode_utf8(&mut buf));
        if used + width > room {
            break;
        }
        out.push(ch);
        used += width;
    }
    if max > 0 {
        out.push('…');
    }
    Cow::Owned(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_text_that_fits_is_borrowed_whole() {
        assert!(matches!(ellipsize("short", 5), Cow::Borrowed("short")));
        assert!(matches!(ellipsize("", 0), Cow::Borrowed("")));
    }

    #[test]
    fn a_cut_text_ends_in_an_ellipsis_within_the_cells() {
        assert_eq!(ellipsize("abcdefgh", 5), "abcd…");
        assert_eq!(cells(&ellipsize("abcdefgh", 5)), 5);
        assert_eq!(ellipsize("abcdefgh", 1), "…");
        assert_eq!(ellipsize("abcdefgh", 0), "");
    }

    #[test]
    fn a_wide_character_is_never_split_across_the_last_column() {
        // Each 日 takes two cells. Five cells keep two of them and `…`.
        let cut = ellipsize("日日日日日", 5);
        assert_eq!(cut, "日日…");
        assert_eq!(cells(&cut), 5);
        // Four cells: a third 日 would need cells 5 and 6, so only one fits
        // beside `…` and a spare cell stays empty.
        let cut = ellipsize("日日日日日", 4);
        assert_eq!(cut, "日…");
        assert!(cells(&cut) <= 4);
    }
}
