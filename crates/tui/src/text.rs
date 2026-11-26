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

/// `text` cut to at most `max` display cells from the front, keeping the end
/// and starting with `…` when it was cut: where two paths differ.
pub fn tail_elide(text: &str, max: usize) -> String {
    if cells(text) <= max {
        return text.to_owned();
    }
    if max == 0 {
        return String::new();
    }
    let room = max - 1;
    let mut used = 0;
    let mut start = text.len();
    let mut buf = [0u8; 4];
    for (index, ch) in text.char_indices().rev() {
        let width = cells(ch.encode_utf8(&mut buf));
        if used + width > room {
            break;
        }
        used += width;
        start = index;
    }
    format!("…{}", &text[start..])
}

/// `text` wrapped to rows of at most `width` display cells. The first row
/// starts with `first_indent`, every later row with `rest_indent`. Words
/// break at spaces; a word longer than a row (a path) breaks between
/// characters, so nothing is cut and no row is wider than `width`. Always
/// returns at least one row.
pub fn wrap_cells(text: &str, first_indent: &str, rest_indent: &str, width: usize) -> Vec<String> {
    let width = width.max(cells(first_indent).max(cells(rest_indent)) + 1);
    let mut rows = Vec::new();
    let mut row = first_indent.to_owned();
    let mut used = cells(&row);
    let mut has_word = false;
    for word in text.split_whitespace() {
        let mut rest = word;
        while !rest.is_empty() {
            let need = cells(rest) + usize::from(has_word);
            if used + need <= width {
                if has_word {
                    row.push(' ');
                }
                row.push_str(rest);
                used += need;
                has_word = true;
                break;
            }
            if has_word {
                rows.push(std::mem::replace(&mut row, rest_indent.to_owned()));
                used = cells(&row);
                has_word = false;
                continue;
            }
            // A word wider than the room on an empty row: break it between
            // characters, at least one per row.
            let room = width - used;
            let mut taken = 0;
            let mut end = 0;
            let mut buf = [0u8; 4];
            for (index, ch) in rest.char_indices() {
                let ch_cells = cells(ch.encode_utf8(&mut buf));
                if taken + ch_cells > room && taken > 0 {
                    break;
                }
                taken += ch_cells;
                end = index + ch.len_utf8();
            }
            row.push_str(&rest[..end]);
            rows.push(std::mem::replace(&mut row, rest_indent.to_owned()));
            used = cells(&row);
            rest = &rest[end..];
        }
    }
    if has_word || rows.is_empty() {
        rows.push(row);
    }
    rows
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

    #[test]
    fn wrapped_rows_fit_the_width_and_keep_every_character() {
        let rows = wrap_cells(
            "see /Users/alice/scripts/a/very/long/path/miner.ts for it",
            "    ",
            "    ",
            24,
        );
        assert!(rows.iter().all(|row| cells(row) <= 24), "{rows:?}");
        let joined: String = rows
            .iter()
            .map(|row| row.trim())
            .collect::<Vec<_>>()
            .join("");
        assert_eq!(
            joined.replace(' ', ""),
            "see/Users/alice/scripts/a/very/long/path/miner.tsforit"
        );
        assert!(rows.len() >= 3, "{rows:?}");
    }

    #[test]
    fn short_text_is_one_row_and_a_narrow_width_still_makes_progress() {
        assert_eq!(wrap_cells("hello world", "  ", "  ", 40), ["  hello world"]);
        assert_eq!(wrap_cells("", "> ", "  ", 10), ["> "]);
        let rows = wrap_cells("日日日日", "", "", 1);
        assert_eq!(rows.concat(), "日日日日");
    }

    #[test]
    fn a_tail_elision_keeps_the_end_where_paths_differ() {
        assert_eq!(tail_elide("/a/b/c.ts", 20), "/a/b/c.ts");
        let cut = tail_elide("/work/shared/prefix/one/miner.ts", 14);
        assert_eq!(cut, "…/one/miner.ts");
        assert_eq!(cells(&cut), 14);
        assert_eq!(tail_elide("日日日日", 5), "…日日");
        assert_eq!(tail_elide("abc", 0), "");
    }
}
