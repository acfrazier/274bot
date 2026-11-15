//! Shared source-view helpers for static parsing.

/// Removes one leading UTF-8 BOM from parser input and returns its byte length.
pub(crate) fn strip_bom(source: &str) -> (&str, usize) {
    source
        .strip_prefix('\u{feff}')
        .map_or((source, 0), |text| (text, '\u{feff}'.len_utf8()))
}

#[cfg(test)]
mod tests {
    use super::strip_bom;

    #[test]
    fn strips_only_one_leading_bom_and_keeps_its_byte_length() {
        assert_eq!(strip_bom("\u{feff}export {}"), ("export {}", 3));
        assert_eq!(
            strip_bom("\u{feff}\u{feff}export {}"),
            ("\u{feff}export {}", 3)
        );
        assert_eq!(strip_bom("export {}"), ("export {}", 0));
    }
}
