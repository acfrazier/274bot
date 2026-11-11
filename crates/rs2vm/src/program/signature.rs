use std::collections::BTreeMap;
use std::path::Path;

use super::bytes;
use super::{LoadError, Signature};

/// Parse command stack counts from the declarations in `scripts/engine.rs2`.
///
/// This reads the command header grammar in the selected content source. Only the literal `string`
/// type uses the string stack; every other declared type uses the integer stack.
pub(super) fn parse(path: &Path, source: &str) -> Result<BTreeMap<Box<str>, Signature>, LoadError> {
    let masked = bytes::mask_comments(path, source)?;
    let source = std::str::from_utf8(&masked).map_err(|error| {
        LoadError::invalid(
            path,
            error.valid_up_to(),
            "comment masking produced invalid UTF-8",
        )
    })?;
    let mut signatures = BTreeMap::new();
    let mut search_from = 0;

    while let Some(relative) = source[search_from..].find("[command") {
        let start = search_from + relative;
        let line_start = source[..start].rfind('\n').map_or(0, |offset| offset + 1);
        if !source[line_start..start].trim().is_empty() {
            search_from = start + "[command".len();
            continue;
        }

        let (name, signature, end) = parse_declaration(path, source, start)?;
        if signatures.insert(name.clone(), signature).is_some() {
            return Err(LoadError::invalid(
                path,
                start,
                format!("duplicate command signature {name}"),
            ));
        }
        search_from = end;
    }

    if signatures.is_empty() {
        return Err(LoadError::invalid(path, 0, "no command declarations found"));
    }
    Ok(signatures)
}

fn parse_declaration(
    path: &Path,
    source: &str,
    start: usize,
) -> Result<(Box<str>, Signature, usize), LoadError> {
    let bytes = source.as_bytes();
    let mut position = start + 1;
    position = expect_word(path, bytes, position, "command")?;
    position = skip_whitespace(bytes, position);
    if bytes.get(position) != Some(&b',') {
        return Err(LoadError::invalid(
            path,
            position,
            "expected comma after command",
        ));
    }
    position = skip_whitespace(bytes, position + 1);
    let name_start = position;
    while let Some(byte) = bytes.get(position) {
        if *byte == b']' {
            break;
        }
        position += 1;
    }
    if position >= bytes.len() {
        return Err(LoadError::invalid(
            path,
            bytes.len(),
            "unterminated command name",
        ));
    }
    let name = source[name_start..position].trim();
    if !is_command_name(name) {
        return Err(LoadError::invalid(path, name_start, "invalid command name"));
    }
    position = skip_whitespace(bytes, position + 1);

    let (int_in, string_in) = if bytes.get(position) == Some(&b'(') {
        let (arguments, end) = parenthesized(path, source, position)?;
        position = end;
        type_counts(path, arguments, end - arguments.len() - 1, true)?
    } else {
        (0, 0)
    };
    position = skip_whitespace(bytes, position);
    let (int_out, string_out) = if bytes.get(position) == Some(&b'(') {
        let (returns, end) = parenthesized(path, source, position)?;
        position = end;
        type_counts(path, returns, end - returns.len() - 1, false)?
    } else {
        (0, 0)
    };

    Ok((
        name.into(),
        Signature {
            int_in,
            string_in,
            int_out,
            string_out,
        },
        position,
    ))
}

fn expect_word(
    path: &Path,
    bytes: &[u8],
    mut position: usize,
    expected: &str,
) -> Result<usize, LoadError> {
    let end = position.saturating_add(expected.len());
    if bytes.get(position..end) != Some(expected.as_bytes()) {
        return Err(LoadError::invalid(
            path,
            position,
            format!("expected {expected}"),
        ));
    }
    position = end;
    Ok(position)
}

fn skip_whitespace(bytes: &[u8], mut position: usize) -> usize {
    while bytes.get(position).is_some_and(u8::is_ascii_whitespace) {
        position += 1;
    }
    position
}

fn parenthesized<'a>(
    path: &Path,
    source: &'a str,
    start: usize,
) -> Result<(&'a str, usize), LoadError> {
    let bytes = source.as_bytes();
    let mut depth = 0usize;
    for position in start..bytes.len() {
        match bytes[position] {
            b'(' => depth += 1,
            b')' if depth == 0 => {
                return Err(LoadError::invalid(
                    path,
                    position,
                    "unmatched command parenthesis",
                ));
            }
            b')' => {
                depth -= 1;
                if depth == 0 {
                    return Ok((&source[start + 1..position], position + 1));
                }
            }
            _ => {}
        }
    }
    Err(LoadError::invalid(
        path,
        start,
        "unterminated command parameter list",
    ))
}

fn type_counts(
    path: &Path,
    list: &str,
    list_offset: usize,
    arguments: bool,
) -> Result<(usize, usize), LoadError> {
    let mut int_count = 0usize;
    let mut string_count = 0usize;
    if list.trim().is_empty() {
        return Ok((0, 0));
    }

    let mut item_offset = list_offset;
    for item in list.split(',') {
        let trimmed = item.trim();
        let leading = item.len() - item.trim_start().len();
        let offset = item_offset + leading;
        let mut words = trimmed.split_ascii_whitespace();
        let Some(kind) = words.next() else {
            return Err(LoadError::invalid(path, offset, "empty command parameter"));
        };
        if !is_type_name(kind) {
            return Err(LoadError::invalid(
                path,
                offset,
                "invalid command parameter type",
            ));
        }
        if arguments {
            let Some(name) = words.next() else {
                return Err(LoadError::invalid(
                    path,
                    offset,
                    "command argument has no parameter name",
                ));
            };
            if !is_parameter_name(name) || words.next().is_some() {
                return Err(LoadError::invalid(
                    path,
                    offset,
                    "invalid command argument declaration",
                ));
            }
        } else if words.next().is_some() {
            return Err(LoadError::invalid(
                path,
                offset,
                "command return must contain only a type",
            ));
        }

        if kind == "string" {
            string_count += 1;
        } else {
            int_count += 1;
        }
        item_offset += item.len() + 1;
    }
    Ok((int_count, string_count))
}

fn is_command_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b'*'))
}

fn is_type_name(name: &str) -> bool {
    let mut bytes = name.bytes();
    let Some(first) = bytes.next() else {
        return false;
    };
    (first.is_ascii_alphabetic() || first == b'_')
        && bytes.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

fn is_parameter_name(name: &str) -> bool {
    let Some(name) = name.strip_prefix('$') else {
        return false;
    };
    let mut bytes = name.bytes();
    let Some(first) = bytes.next() else {
        return false;
    };
    (first.is_ascii_alphabetic() || first == b'_')
        && bytes.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}
