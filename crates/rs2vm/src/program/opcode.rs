use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use super::bytes;
use super::LoadError;

// Mirrors the numeric members used by src/engine/script/ScriptOpcode.ts and ServerTriggerType.ts.
pub(super) struct OpcodeTable {
    pub(super) by_number: BTreeMap<u16, Box<str>>,
    pub(super) string_operand: u16,
    pub(super) byte_operand_exceptions: BTreeSet<u16>,
}

impl OpcodeTable {
    pub(super) fn from_entries(
        path: &Path,
        entries: BTreeMap<Box<str>, u32>,
    ) -> Result<Self, LoadError> {
        let mut by_number = BTreeMap::new();
        let mut by_name = BTreeMap::new();
        for (name, raw_number) in entries {
            let number = u16::try_from(raw_number).map_err(|_| {
                LoadError::invalid(
                    path,
                    0,
                    format!("opcode {name} value {raw_number} exceeds u16"),
                )
            })?;
            by_name.insert(name.clone(), number);
            by_number.insert(number, name);
        }

        let string_operand = required_opcode(path, &by_name, "PUSH_CONSTANT_STRING")?;
        let byte_operand_exceptions = [
            "RETURN",
            "POP_INT_DISCARD",
            "POP_STRING_DISCARD",
            "GOSUB",
            "JUMP",
        ]
        .into_iter()
        .map(|name| required_opcode(path, &by_name, name))
        .collect::<Result<BTreeSet<_>, _>>()?;

        Ok(Self {
            by_number,
            string_operand,
            byte_operand_exceptions,
        })
    }
}

fn required_opcode(
    path: &Path,
    entries: &BTreeMap<Box<str>, u16>,
    name: &str,
) -> Result<u16, LoadError> {
    entries
        .get(name)
        .copied()
        .ok_or_else(|| LoadError::invalid(path, 0, format!("opcode enum is missing {name}")))
}

/// Parse the numeric members of a TypeScript enum block.
pub(super) fn parse_numeric_enum(
    path: &Path,
    source: &str,
    declaration: &str,
) -> Result<BTreeMap<Box<str>, u32>, LoadError> {
    let masked = bytes::mask_comments(path, source)?;
    let source = std::str::from_utf8(&masked).map_err(|error| {
        LoadError::invalid(
            path,
            error.valid_up_to(),
            "comment masking produced invalid UTF-8",
        )
    })?;
    let start = find_declaration(path, source, declaration)?;
    let mut brace = start + declaration.len();
    while source
        .as_bytes()
        .get(brace)
        .is_some_and(u8::is_ascii_whitespace)
    {
        brace += 1;
    }
    if source.as_bytes().get(brace) != Some(&b'{') {
        return Err(LoadError::invalid(
            path,
            brace,
            "expected enum body opening brace",
        ));
    }
    let body_start = brace + 1;
    let Some(relative_close) = source.as_bytes()[body_start..]
        .iter()
        .position(|byte| *byte == b'}')
    else {
        return Err(LoadError::invalid(
            path,
            source.len(),
            "enum body has no closing brace",
        ));
    };
    let body_end = body_start + relative_close;
    let body = &source[body_start..body_end];
    if let Some(relative_open) = body.as_bytes().iter().position(|byte| *byte == b'{') {
        return Err(LoadError::invalid(
            path,
            body_start + relative_open,
            "unexpected nested enum brace",
        ));
    }

    let mut entries = BTreeMap::new();
    let mut numbers = BTreeSet::new();
    let mut next_number = None;
    let mut line_offset = body_start;
    for line in body.split_inclusive('\n') {
        parse_enum_line(
            path,
            line,
            line_offset,
            &mut entries,
            &mut numbers,
            &mut next_number,
        )?;
        line_offset += line.len();
    }
    if entries.is_empty() {
        return Err(LoadError::invalid(
            path,
            body_start,
            "enum body has no members",
        ));
    }
    Ok(entries)
}

fn find_declaration(path: &Path, source: &str, declaration: &str) -> Result<usize, LoadError> {
    let mut search_from = 0;
    while let Some(relative) = source[search_from..].find(declaration) {
        let start = search_from + relative;
        let after = start + declaration.len();
        let next = source.as_bytes().get(after).copied();
        if next.is_none_or(|byte| byte.is_ascii_whitespace() || byte == b'{') {
            return Ok(start);
        }
        search_from = after;
    }
    Err(LoadError::invalid(
        path,
        source.len(),
        format!("missing enum declaration {declaration}"),
    ))
}

fn parse_enum_line(
    path: &Path,
    line: &str,
    line_offset: usize,
    entries: &mut BTreeMap<Box<str>, u32>,
    numbers: &mut BTreeSet<u32>,
    next_number: &mut Option<u32>,
) -> Result<(), LoadError> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return Ok(());
    }
    let entry = match trimmed.strip_suffix(',') {
        Some(entry) => entry.trim(),
        None => trimmed,
    };
    let leading = line.len() - line.trim_start().len();
    let offset = line_offset + leading;
    if entry.is_empty() || entry.contains(',') {
        return Err(LoadError::invalid(
            path,
            offset,
            "enum member must be one name and optional value",
        ));
    }

    let (name, number) = match entry.split_once('=') {
        Some((name, value)) if !value.contains('=') => {
            let name = name.trim();
            let value_offset = entry.len() - value.len() + value.len() - value.trim_start().len();
            let value = value.trim();
            if !is_identifier(name) {
                return Err(LoadError::invalid(path, offset, "invalid enum member name"));
            }
            let number = parse_enum_number(value).ok_or_else(|| {
                LoadError::invalid(
                    path,
                    offset + value_offset,
                    "enum value must be an unsigned integer",
                )
            })?;
            (name, number)
        }
        Some(_) => {
            return Err(LoadError::invalid(
                path,
                offset,
                "enum member has more than one assignment",
            ))
        }
        None => {
            if !is_identifier(entry) {
                return Err(LoadError::invalid(
                    path,
                    offset,
                    "unexpected text in enum body",
                ));
            }
            let number = next_number.ok_or_else(|| {
                LoadError::invalid(
                    path,
                    offset,
                    "first enum member must have an explicit value",
                )
            })?;
            (entry, number)
        }
    };

    let next = number
        .checked_add(1)
        .ok_or_else(|| LoadError::invalid(path, offset, "enum value cannot be incremented"))?;
    if entries.contains_key(name) {
        return Err(LoadError::invalid(
            path,
            offset,
            format!("duplicate enum member {name}"),
        ));
    }
    if !numbers.insert(number) {
        return Err(LoadError::invalid(
            path,
            offset,
            format!("duplicate enum value {number}"),
        ));
    }
    entries.insert(name.into(), number);
    *next_number = Some(next);
    Ok(())
}

fn parse_enum_number(value: &str) -> Option<u32> {
    if let Some(hex) = value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
    {
        u32::from_str_radix(hex, 16).ok()
    } else {
        value.parse().ok()
    }
}

fn is_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first.is_ascii_alphabetic() || first == '_')
        && chars.all(|character| character.is_ascii_alphanumeric() || character == '_')
}

pub(super) fn triggers_from_entries(
    path: &Path,
    entries: BTreeMap<Box<str>, u32>,
) -> Result<BTreeMap<Box<str>, u8>, LoadError> {
    let mut triggers = BTreeMap::new();
    for (name, value) in entries {
        let value = u8::try_from(value).map_err(|_| {
            LoadError::invalid(path, 0, format!("trigger {name} value {value} exceeds u8"))
        })?;
        triggers.insert(name, value);
    }
    Ok(triggers)
}
