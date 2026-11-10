//! Loading and indexing RuneScript bytecode, command signatures, and server configs.

mod bytes;
mod opcode;
mod pack;
mod script;
mod signature;

use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};

use thiserror::Error;

use crate::{LocTypeId, ScriptId, Site, VarpId};

/// One decoded instruction operand.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Operand {
    /// A signed 32-bit integer or one-byte integer operand.
    Int(i32),
    /// A zero-terminated RuneScript string operand.
    Text(Box<str>),
}

/// One decoded bytecode instruction.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Instruction {
    /// The engine-specific opcode number.
    pub opcode: u16,
    /// The decoded operand.
    pub operand: Operand,
}

impl Instruction {
    /// Return an integer operand, or `None` for a string operand.
    pub fn int(&self) -> Option<i32> {
        match &self.operand {
            Operand::Int(value) => Some(*value),
            Operand::Text(_) => None,
        }
    }
}

/// One decoded RuneScript script.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Script {
    /// The script's index in the server script pack.
    pub id: ScriptId,
    /// The script name from its compiled header.
    pub name: Box<str>,
    /// The source path from the first `scripts/` directory, when present.
    pub source: Box<str>,
    /// The engine lookup key, or `-1` when the script is not indexed.
    pub lookup: i32,
    /// Parameter type bytes from the compiled script header.
    pub params: Vec<u8>,
    /// Number of integer local slots.
    pub int_locals: u16,
    /// Number of string local slots.
    pub string_locals: u16,
    /// Number of integer arguments.
    pub int_args: u16,
    /// Number of string arguments.
    pub string_args: u16,
    /// Instructions in program-counter order.
    pub code: Vec<Instruction>,
    /// Switch tables, indexed by the table operand.
    pub switches: Vec<BTreeMap<i32, i32>>,
    /// Source line for each recorded program counter.
    pub lines: BTreeMap<u32, u32>,
}

impl Script {
    /// Return the source line associated with `pc`, if the script has line data.
    pub fn line(&self, pc: u32) -> Option<u32> {
        self.lines.range(..=pc).next_back().map(|(_, line)| *line)
    }
}

/// Fixed input and output stack counts for one engine command.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Signature {
    /// Integer values consumed from the integer stack.
    pub int_in: usize,
    /// String values consumed from the string stack.
    pub string_in: usize,
    /// Integer values produced on the integer stack.
    pub int_out: usize,
    /// String values produced on the string stack.
    pub string_out: usize,
}

/// A value stored in an enum or loc parameter map.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConfigValue {
    /// A signed integer value.
    Int(i32),
    /// A string value.
    Text(Box<str>),
}

/// One parameter configuration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Param {
    /// Whether the parameter uses the string stack.
    pub string: bool,
    /// Default integer value.
    pub int_default: i32,
    /// Default string value. The engine uses `"null"` when none is stored.
    pub string_default: Box<str>,
}

/// One player variable configuration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Varp {
    /// Whether the variable uses the string stack.
    pub string: bool,
    /// Whether scripts require protected access to write the variable.
    pub protect: bool,
    /// Whether the variable is transmitted to the client.
    pub transmit: bool,
    /// Engine debug name, or an empty string when absent.
    pub name: Box<str>,
}

/// One variable-bit configuration from the server and client records.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Varbit {
    /// The backing player variable.
    pub base: VarpId,
    /// First bit in the inclusive range.
    pub lo: u8,
    /// Last bit in the inclusive range.
    pub hi: u8,
    /// Engine debug name, or an empty string when absent.
    pub name: Box<str>,
}

/// One enum configuration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Enum {
    /// Encoded enum input type.
    pub input: u8,
    /// Encoded enum output type.
    pub output: u8,
    /// Default integer value.
    pub int_default: i32,
    /// Default string value.
    pub string_default: Box<str>,
    /// Values indexed by the enum key.
    pub values: BTreeMap<i32, ConfigValue>,
}

/// Server-side configuration for one loc type.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LocConfig {
    /// Loc category from server config code 61.
    pub category: Option<i32>,
    /// Parameters indexed by parameter id.
    pub params: BTreeMap<i32, ConfigValue>,
    /// Engine debug name, or an empty string when absent.
    pub name: Box<str>,
}

/// Decoded server configuration packs used by the interpreter.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Packs {
    /// Parameter configs indexed by id.
    pub params: BTreeMap<i32, Param>,
    /// Player variable configs indexed by id.
    pub varps: BTreeMap<VarpId, Varp>,
    /// Variable-bit configs indexed by id.
    pub varbits: BTreeMap<u16, Varbit>,
    /// NPC variable string flags, indexed by id.
    pub varns: BTreeMap<u16, bool>,
    /// Shared variable string flags, indexed by id.
    pub varss: BTreeMap<u16, bool>,
    /// Inventory debug names, indexed by id.
    pub invs: BTreeMap<i32, Box<str>>,
    /// Enum configs indexed by id.
    pub enums: BTreeMap<i32, Enum>,
    /// Loc configs indexed by id.
    pub locs: BTreeMap<LocTypeId, LocConfig>,
}

/// A typed failure while loading a program or one of its inputs.
#[derive(Debug, Error)]
#[error("{path} at byte {offset}: {kind}")]
pub struct LoadError {
    path: PathBuf,
    offset: usize,
    #[source]
    kind: LoadErrorKind,
}

impl LoadError {
    /// Create a file-read error with its path and byte offset.
    pub fn io(path: impl Into<PathBuf>, offset: usize, source: io::Error) -> Self {
        Self {
            path: path.into(),
            offset,
            kind: LoadErrorKind::Io(source),
        }
    }

    /// Return the input path associated with this error.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Return the byte offset associated with this error.
    pub const fn offset(&self) -> usize {
        self.offset
    }

    /// Return the typed failure reason.
    pub fn kind(&self) -> &LoadErrorKind {
        &self.kind
    }

    pub(super) fn invalid(
        path: impl Into<PathBuf>,
        offset: usize,
        message: impl Into<Box<str>>,
    ) -> Self {
        Self {
            path: path.into(),
            offset,
            kind: LoadErrorKind::InvalidFormat(message.into()),
        }
    }

    pub(super) fn truncated(
        path: impl Into<PathBuf>,
        offset: usize,
        needed: usize,
        remaining: usize,
    ) -> Self {
        Self {
            path: path.into(),
            offset,
            kind: LoadErrorKind::Truncated { needed, remaining },
        }
    }

    pub(super) fn unsupported_code(
        path: impl Into<PathBuf>,
        offset: usize,
        pack: &'static str,
        code: u8,
    ) -> Self {
        Self {
            path: path.into(),
            offset,
            kind: LoadErrorKind::UnsupportedCode { pack, code },
        }
    }
}

/// The specific class of a [`LoadError`].
#[derive(Debug, Error)]
pub enum LoadErrorKind {
    /// A file system operation failed.
    #[error("I/O error: {0}")]
    Io(#[source] io::Error),
    /// The input ended before a complete field was available.
    #[error("truncated input: needed {needed} bytes, found {remaining}")]
    Truncated {
        /// Number of bytes required by the field.
        needed: usize,
        /// Number of bytes available at the current offset.
        remaining: usize,
    },
    /// The input has a known structural or semantic defect.
    #[error("invalid format: {0}")]
    InvalidFormat(Box<str>),
    /// The pack contains a code outside the supported server-record format.
    #[error("unsupported {pack} config code {code}")]
    UnsupportedCode {
        /// Pack whose record contained the code.
        pack: &'static str,
        /// Unsupported record code.
        code: u8,
    },
}

/// One loaded engine program and its supported server data.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Program {
    /// Scripts indexed by their server-pack id.
    pub scripts: BTreeMap<ScriptId, Script>,
    /// Engine-specific opcode names indexed by opcode number.
    pub opcodes: BTreeMap<u16, Box<str>>,
    /// Engine server-trigger names indexed by their source names.
    pub triggers: BTreeMap<Box<str>, u8>,
    /// Command signatures indexed by command name.
    pub signatures: BTreeMap<Box<str>, Signature>,
    /// Server config packs used by the interpreter.
    pub packs: Packs,
}

impl Program {
    /// Load bytecode, opcode and trigger tables, command signatures, and server packs.
    ///
    /// `engine` is the Lost City engine root. `content` is the matching content root.
    /// `client_varbits` is the decoded `varbit.dat` file from the client `config` archive.
    pub fn load(engine: &Path, content: &Path, client_varbits: &[u8]) -> Result<Self, LoadError> {
        let opcode_path = engine.join("src/engine/script/ScriptOpcode.ts");
        let opcode_source = bytes::read_utf8(&opcode_path)?;
        let opcode_table = opcode::parse_numeric_enum(
            &opcode_path,
            &opcode_source,
            "export const enum ScriptOpcode",
        )?;
        let opcode_table = opcode::OpcodeTable::from_entries(&opcode_path, opcode_table)?;

        let trigger_path = engine.join("src/engine/script/ServerTriggerType.ts");
        let trigger_source = bytes::read_utf8(&trigger_path)?;
        let trigger_entries =
            opcode::parse_numeric_enum(&trigger_path, &trigger_source, "enum ServerTriggerType")?;
        let triggers = opcode::triggers_from_entries(&trigger_path, trigger_entries)?;

        let script_dat_path = engine.join("data/pack/server/script.dat");
        let script_idx_path = engine.join("data/pack/server/script.idx");
        let scripts = script::load_scripts(&script_dat_path, &script_idx_path, &opcode_table)?;

        let signature_path = content.join("scripts/engine.rs2");
        let signature_source = bytes::read_utf8(&signature_path)?;
        let signatures = signature::parse(&signature_path, &signature_source)?;

        let packs = pack::load(&engine.join("data/pack/server"), client_varbits)?;

        Ok(Self {
            scripts,
            opcodes: opcode_table.by_number,
            triggers,
            signatures,
            packs,
        })
    }

    /// Look up a script by trigger, preferring type, category, then global entries.
    pub fn lookup(
        &self,
        trigger: u8,
        subject: Option<i32>,
        category: Option<i32>,
    ) -> Option<ScriptId> {
        if let Some(subject) = subject {
            let key = lookup_key(trigger, 2, subject);
            if let Some(id) = self.script_with_key(key) {
                return Some(id);
            }
        }
        if let Some(category) = category {
            let key = lookup_key(trigger, 1, category);
            if let Some(id) = self.script_with_key(key) {
                return Some(id);
            }
        }
        self.script_with_key(i32::from(trigger))
    }

    /// Return a script by id.
    pub fn script(&self, id: ScriptId) -> Option<&Script> {
        self.scripts.get(&id)
    }

    /// Return an opcode name by engine-specific opcode number.
    pub fn opcode(&self, opcode: u16) -> Option<&str> {
        self.opcodes.get(&opcode).map(Box::as_ref)
    }

    /// Return the source path, line, script id, and program counter for a site.
    ///
    /// The source path is sanitized by the script decoder. It does not contain the build host's
    /// absolute path.
    pub fn format_site(&self, site: Site) -> String {
        let Some(script) = self.script(site.script) else {
            return format!("script {} pc {}", site.script.0, site.pc);
        };
        match script.line(site.pc) {
            Some(line) if !script.source.is_empty() => {
                format!(
                    "{}:{line} (script {} pc {})",
                    script.source, site.script.0, site.pc
                )
            }
            Some(line) => format!("script {} line {line} pc {}", site.script.0, site.pc),
            None if !script.source.is_empty() => {
                format!(
                    "{} (script {} pc {})",
                    script.source, site.script.0, site.pc
                )
            }
            None => format!("script {} pc {}", site.script.0, site.pc),
        }
    }

    fn script_with_key(&self, key: i32) -> Option<ScriptId> {
        self.scripts
            .values()
            .rev()
            .find(|script| script.lookup != -1 && script.lookup == key)
            .map(|script| script.id)
    }

    /// Return a trigger number by its source enum name.
    pub fn trigger(&self, name: &str) -> Option<u8> {
        self.triggers.get(name).copied()
    }
}

fn lookup_key(trigger: u8, kind: i32, subject: i32) -> i32 {
    i32::from(trigger) | (kind << 8) | subject.wrapping_shl(10)
}
