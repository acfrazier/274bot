use std::collections::BTreeMap;
use std::path::Path;

use crate::ScriptId;

use super::bytes::Cursor;
use super::opcode::OpcodeTable;
use super::{Instruction, LoadError, Operand, Script};

// Mirrors src/engine/script/ScriptFile.ts:58-124.
const COMPILER_VERSION: i32 = 27;
const MIN_SCRIPT_LENGTH: usize = 16;

pub(super) fn load_scripts(
    dat_path: &Path,
    idx_path: &Path,
    opcodes: &OpcodeTable,
) -> Result<BTreeMap<ScriptId, Script>, LoadError> {
    let dat_bytes = super::bytes::read_file(dat_path)?;
    let idx_bytes = super::bytes::read_file(idx_path)?;
    let mut dat = Cursor::new(dat_path, &dat_bytes, 0);
    let mut idx = Cursor::new(idx_path, &idx_bytes, 0);

    let entry_count_offset = dat.offset();
    let entry_count = dat.i32()?;
    if entry_count < 0 {
        return Err(LoadError::invalid(
            dat_path,
            entry_count_offset,
            "negative script count",
        ));
    }
    idx.skip(4)?;
    let version_offset = dat.offset();
    let version = dat.i32()?;
    if version != COMPILER_VERSION {
        return Err(LoadError::invalid(
            dat_path,
            version_offset,
            format!("script compiler version {version}, expected {COMPILER_VERSION}"),
        ));
    }

    let mut scripts = BTreeMap::new();
    for raw_id in 0..entry_count as u32 {
        let size_offset = idx.offset();
        let size = idx.i32()?;
        if size < 0 {
            return Err(LoadError::invalid(
                idx_path,
                size_offset,
                "negative script size",
            ));
        }
        if size == 0 {
            continue;
        }
        let (base, bytes) = dat.take_with_offset(size as usize)?;
        let script = decode_script(dat_path, bytes, base, ScriptId(raw_id), opcodes)?;
        scripts.insert(script.id, script);
    }
    idx.finish()?;
    dat.finish()?;
    Ok(scripts)
}

fn decode_script(
    path: &Path,
    bytes: &[u8],
    base: usize,
    id: ScriptId,
    opcodes: &OpcodeTable,
) -> Result<Script, LoadError> {
    if bytes.len() < MIN_SCRIPT_LENGTH {
        return Err(LoadError::invalid(
            path,
            base,
            "script is shorter than the engine minimum",
        ));
    }
    let trailer_length_offset = base + bytes.len() - 2;
    let trailer_length = usize::from(u16::from_be_bytes([
        bytes[bytes.len() - 2],
        bytes[bytes.len() - 1],
    ]));
    let trailer_start = bytes
        .len()
        .checked_sub(trailer_length.saturating_add(14))
        .ok_or_else(|| {
            LoadError::invalid(
                path,
                trailer_length_offset,
                "script trailer starts before the file",
            )
        })?;
    let trailer_end = bytes.len() - 2;
    if trailer_start > trailer_end {
        return Err(LoadError::invalid(
            path,
            trailer_length_offset,
            "script trailer starts after its end",
        ));
    }

    let mut trailer = Cursor::new(
        path,
        &bytes[trailer_start..trailer_end],
        base + trailer_start,
    );
    let _instruction_count = trailer.i32()?;
    let int_locals = trailer.u16()?;
    let string_locals = trailer.u16()?;
    let int_args = trailer.u16()?;
    let string_args = trailer.u16()?;
    let switch_count = trailer.u8()?;
    let mut switches = Vec::with_capacity(usize::from(switch_count));
    for _ in 0..switch_count {
        let count = trailer.u16()?;
        let mut table = BTreeMap::new();
        for _ in 0..count {
            table.insert(trailer.i32()?, trailer.i32()?);
        }
        switches.push(table);
    }
    trailer.finish()?;

    let mut head = Cursor::new(path, &bytes[..trailer_start], base);
    let name = head.string(0)?;
    let source = sanitize_source(head.string(0)?);
    let lookup = head.i32()?;
    let parameter_count = usize::from(head.u8()?);
    let params = head.take(parameter_count)?.to_vec();
    let line_count = head.u16()?;
    let mut lines = BTreeMap::new();
    for _ in 0..line_count {
        let pc_offset = head.offset();
        let pc = head.i32()?;
        let line = head.i32()?;
        let pc = u32::try_from(pc).map_err(|_| {
            LoadError::invalid(
                path,
                pc_offset,
                "negative script line-table program counter",
            )
        })?;
        let line = u32::try_from(line)
            .map_err(|_| LoadError::invalid(path, pc_offset + 4, "negative source line number"))?;
        lines.insert(pc, line);
    }

    let mut code = Vec::new();
    let byte_operand_exceptions = &opcodes.byte_operand_exceptions;
    while head.remaining() > 0 {
        let opcode_offset = head.offset();
        let opcode = head.u16()?;
        if !opcodes.by_number.contains_key(&opcode) {
            return Err(LoadError::invalid(
                path,
                opcode_offset,
                format!("script uses unknown opcode {opcode}"),
            ));
        }
        let operand = if opcode == opcodes.string_operand {
            Operand::Text(head.string(0)?)
        } else if opcode > 100 || byte_operand_exceptions.contains(&opcode) {
            Operand::Int(i32::from(head.u8()?))
        } else {
            Operand::Int(head.i32()?)
        };
        if code.len() >= u32::MAX as usize {
            return Err(LoadError::invalid(
                path,
                opcode_offset,
                "script program counter exceeds u32",
            ));
        }
        code.push(Instruction { opcode, operand });
    }

    Ok(Script {
        id,
        name,
        source,
        lookup,
        params,
        int_locals,
        string_locals,
        int_args,
        string_args,
        code,
        switches,
        lines,
    })
}

fn sanitize_source(source: Box<str>) -> Box<str> {
    if let Some(start) = source.find("scripts/") {
        return source[start..].into();
    }
    if !source.contains('\\') {
        if source.starts_with('/') {
            return match source.rsplit('/').next() {
                Some(name) => name.into(),
                None => String::new().into_boxed_str(),
            };
        }
        return source;
    }

    let normalized = source.replace('\\', "/");
    if let Some(start) = normalized.find("scripts/") {
        return normalized[start..].into();
    }
    let bytes = normalized.as_bytes();
    let absolute = normalized.starts_with('/') || bytes.get(1) == Some(&b':');
    if absolute {
        match normalized.rsplit('/').next() {
            Some(name) => name.into(),
            None => String::new().into_boxed_str(),
        }
    } else {
        normalized.into_boxed_str()
    }
}
