//! The static manifest reader of a Luau script (ADR 0023 item 18).
//!
//! The first statement of a `.luau` file is `manifest { ... }`. This module
//! reads that statement in Rust, with a closed literal grammar, and gives the
//! neutral [`ScriptManifest`] of the `script` crate. It starts no VM and runs no
//! script code: every refusal is a typed [`ManifestError`] with a source
//! position. [`erase_manifest`] gives the body that the run compiles. It
//! replaces the statement with spaces and keeps each line break, so the line
//! numbers of the VM's errors stay those of the file.
//!
//! The grammar and the field rules are those of the JavaScript v2 reader
//! (`script-js`, `library/manifest/`). The Luau reader differs in two places.
//! A `capabilities` entry is refused with its own reason, because Luau has no
//! packet API (design rule O6). The text of a field and the value of a setting
//! follow the same rules, so one neutral model describes both surfaces.

use std::collections::HashSet;
use std::fmt;
use std::ops::Range;

use script::card::SettingDef;
use script::manifest::{DeclaredHostCapabilities, ManifestText, ScriptManifest};

/// The largest `.luau` file of the product, in bytes (ADR 0023 item 18).
pub const SOURCE_LIMIT_BYTES: usize = 256 * 1024;

/// The deepest table nesting that the reader accepts. A valid manifest nests
/// five levels (`settings`, one setting, `showIf`, `anyOf`); the cap keeps the
/// recursion of the reader bounded on hostile input.
const MAX_DEPTH: usize = 16;

const BOM: &str = "\u{feff}";

/// Words that Luau reserves; none is a table key.
const RESERVED_WORDS: &[&str] = &[
    "and", "break", "do", "else", "elseif", "end", "false", "for", "function", "if", "in", "local",
    "nil", "not", "or", "repeat", "return", "then", "true", "until", "while",
];

/// The fields of a setting definition.
const SETTING_FIELDS: &[&str] = &[
    "type",
    "default",
    "label",
    "min",
    "max",
    "options",
    "optionLabels",
    "group",
    "showIf",
    "optionsFrom",
    "help",
];

/// The fields of the manifest table, in the order that a missing field reports.
const MANIFEST_FIELDS: &[&str] = &[
    "apiVersion",
    "name",
    "version",
    "author",
    "description",
    "category",
    "tags",
    "settings",
    "capabilities",
];

/// The kind of expression that the closed grammar refuses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExpressionKind {
    /// A name used as a value.
    Identifier,
    /// A call, a method call, or a call with a string or table argument.
    Call,
    /// A `function` expression.
    Function,
    /// The value `nil`.
    Nil,
    /// An operator, a length, a concatenation or an index.
    Operator,
    /// A backtick string.
    InterpolatedString,
    /// A long-bracket string.
    LongString,
    /// A hexadecimal or binary number.
    NonDecimalNumber,
}

impl ExpressionKind {
    fn text(self) -> &'static str {
        match self {
            Self::Identifier => "an identifier",
            Self::Call => "a call",
            Self::Function => "a function",
            Self::Nil => "nil",
            Self::Operator => "an operator",
            Self::InterpolatedString => "an interpolated string",
            Self::LongString => "a long-bracket string",
            Self::NonDecimalNumber => "a hexadecimal or binary number",
        }
    }
}

/// The key form that the closed grammar refuses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyKind {
    /// `[expression] = value` where the expression is not a quoted string.
    Computed,
    /// A reserved word of Luau used as a name key.
    Reserved(String),
    /// The same key twice in one table.
    Duplicate(String),
    /// A table with both `key = value` fields and positional items.
    Mixed,
}

/// The typed reason of a refusal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ManifestReason {
    /// The file is larger than [`SOURCE_LIMIT_BYTES`].
    FileTooLarge { bytes: usize },
    /// The first statement of the file is not `manifest`.
    MissingManifest,
    /// `manifest` is not followed by one table constructor.
    ExpectedTable,
    /// A block comment has no end.
    UnterminatedComment,
    /// A string has no end on its line.
    UnterminatedString,
    /// A table or the file ends before the manifest statement does.
    UnexpectedEnd,
    /// A token that the grammar does not allow here. The text says what it expects.
    UnexpectedToken(&'static str),
    /// Code follows the table in the same statement.
    TrailingExpression,
    /// An expression that is not a literal.
    Expression(ExpressionKind),
    /// A key form that the grammar refuses.
    Key(KeyKind),
    /// Tables nest deeper than the reader accepts.
    TooDeep,
    /// A number that is not a decimal literal.
    MalformedNumber,
    /// A number whose value is not finite.
    NonFiniteNumber,
    /// An escape that the grammar does not list.
    UnsupportedEscape(char),
    /// A `\u{...}` escape that is not one Unicode scalar value.
    InvalidUnicodeEscape,
    /// A field of the manifest that the model does not have.
    UnknownField(String),
    /// A field of the manifest that is absent.
    MissingField(&'static str),
    /// A value of the wrong type. The text names the expected type.
    WrongType(&'static str),
    /// An empty text.
    Empty,
    /// A text with a space, tab or line break at either end.
    Whitespace,
    /// A text with a control or bidirectional-format character.
    ControlCharacter,
    /// A tag that occurs twice.
    DuplicateTag(String),
    /// `apiVersion` is not an integer in the range of `u32`.
    MalformedApiVersion,
    /// `apiVersion` is an integer other than 2.
    UnsupportedApiVersion(u32),
    /// A capability that the neutral model does not have.
    UnknownCapability(String),
    /// A capability of the model that no Luau API member uses.
    UnsupportedCapability(String),
    /// A setting id that is empty, has outer space, or has a control character.
    InvalidSettingId,
    /// A setting with no `type`.
    MissingSettingType,
    /// A setting `type` outside the closed list.
    UnknownSettingType(String),
    /// A setting with no `default`.
    MissingSettingDefault,
    /// A field that a setting does not have.
    UnknownSettingField(String),
    /// A setting value of the wrong type. The text names the expected type.
    SettingWrongType(&'static str),
    /// A field that the setting type does not allow: the field and the type.
    SettingFieldNotAllowed { field: String, ty: String },
    /// A numeric bound that is not finite.
    NonFiniteBound,
    /// `min` is above `max`.
    InvertedRange,
    /// A numeric default outside `min` and `max`.
    DefaultOutsideRange,
    /// An `options` list with no entry.
    EmptyOptions,
    /// An option that occurs twice.
    DuplicateOption(String),
    /// A default that is not in `options`.
    DefaultNotOption(String),
    /// An `optionLabels` key that is not in `options`.
    UnknownOptionLabel(String),
    /// A `showIf` key that names no setting.
    UnknownReference(String),
    /// A `showIf` key that names its own setting or a setting that is no string.
    InvalidReference,
    /// A `showIf.anyOf` list with no entry.
    EmptyCondition,
    /// A `showIf.anyOf` value that occurs twice.
    DuplicateCondition(String),
    /// An `optionsFrom` outside the closed list.
    UnknownOptionSource(String),
    /// `options` and `optionsFrom` together.
    CompetingOptions,
    /// A tile default that is not `{ x, z, level }` of integers.
    InvalidTile,
}

impl ManifestReason {
    /// The stable machine-readable code of the reason.
    pub fn code(&self) -> &'static str {
        match self {
            Self::FileTooLarge { .. } => "luau-manifest-file-too-large",
            Self::MissingManifest => "luau-manifest-required",
            Self::ExpectedTable => "luau-manifest-expected-table",
            Self::UnterminatedComment => "luau-manifest-unterminated-comment",
            Self::UnterminatedString => "luau-manifest-unterminated-string",
            Self::UnexpectedEnd => "luau-manifest-unexpected-end",
            Self::UnexpectedToken(_) => "luau-manifest-unexpected-token",
            Self::TrailingExpression => "luau-manifest-trailing-expression",
            Self::Expression(_) => "luau-manifest-nonliteral",
            Self::Key(KeyKind::Computed) => "luau-manifest-computed-key",
            Self::Key(KeyKind::Reserved(_)) => "luau-manifest-reserved-key",
            Self::Key(KeyKind::Duplicate(_)) => "luau-manifest-duplicate-key",
            Self::Key(KeyKind::Mixed) => "luau-manifest-mixed-table",
            Self::TooDeep => "luau-manifest-too-deep",
            Self::MalformedNumber => "luau-manifest-number-malformed",
            Self::NonFiniteNumber => "luau-manifest-number-nonfinite",
            Self::UnsupportedEscape(_) => "luau-manifest-escape-unsupported",
            Self::InvalidUnicodeEscape => "luau-manifest-escape-unicode",
            Self::UnknownField(_) => "manifest-unknown-field",
            Self::MissingField(_) => "manifest-required-field",
            Self::WrongType(_) => "manifest-field-type",
            Self::Empty => "manifest-empty-field",
            Self::Whitespace => "manifest-field-whitespace",
            Self::ControlCharacter => "manifest-field-control",
            Self::DuplicateTag(_) => "manifest-duplicate-tag",
            Self::MalformedApiVersion => "api-version-malformed",
            Self::UnsupportedApiVersion(_) => "api-version-unsupported",
            Self::UnknownCapability(_) => "capability-unknown",
            Self::UnsupportedCapability(_) => "luau-capability-unsupported",
            Self::InvalidSettingId => "setting-id-invalid",
            Self::MissingSettingType => "setting-type-missing",
            Self::UnknownSettingType(_) => "setting-type-unknown",
            Self::MissingSettingDefault => "setting-default-missing",
            Self::UnknownSettingField(_) => "setting-unknown-field",
            Self::SettingWrongType(_) => "setting-field-type",
            Self::SettingFieldNotAllowed { .. } => "setting-field-not-allowed",
            Self::NonFiniteBound => "setting-number-nonfinite",
            Self::InvertedRange => "setting-range-inverted",
            Self::DefaultOutsideRange => "setting-default-outside-range",
            Self::EmptyOptions => "setting-options-empty",
            Self::DuplicateOption(_) => "setting-option-duplicate",
            Self::DefaultNotOption(_) => "setting-default-not-option",
            Self::UnknownOptionLabel(_) => "setting-option-label-unknown",
            Self::UnknownReference(_) => "setting-reference-unknown",
            Self::InvalidReference => "setting-reference-invalid",
            Self::EmptyCondition => "setting-condition-empty",
            Self::DuplicateCondition(_) => "setting-condition-duplicate",
            Self::UnknownOptionSource(_) => "setting-option-source-unknown",
            Self::CompetingOptions => "setting-options-competing",
            Self::InvalidTile => "setting-tile-invalid",
        }
    }
}

impl fmt::Display for ManifestReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::FileTooLarge { bytes } => write!(
                f,
                "the file has {bytes} bytes; a Luau script has at most {SOURCE_LIMIT_BYTES}"
            ),
            Self::MissingManifest => f.write_str("the first statement must be `manifest { ... }`"),
            Self::ExpectedTable => {
                f.write_str("`manifest` must be followed by one table `{ ... }`")
            }
            Self::UnterminatedComment => f.write_str("a block comment has no end"),
            Self::UnterminatedString => f.write_str("a string has no end on its line"),
            Self::UnexpectedEnd => f.write_str("the manifest ends before its table closes"),
            Self::UnexpectedToken(expected) => write!(f, "expected {expected}"),
            Self::TrailingExpression => {
                f.write_str("the manifest statement must end at its closing `}`")
            }
            Self::Expression(kind) => write!(
                f,
                "a manifest holds literals only, and {} is not a literal",
                kind.text()
            ),
            Self::Key(KeyKind::Computed) => {
                f.write_str("a computed key is not allowed; use a name or a quoted string key")
            }
            Self::Key(KeyKind::Reserved(word)) => {
                write!(f, "`{word}` is a reserved word and cannot be a name key")
            }
            Self::Key(KeyKind::Duplicate(key)) => write!(f, "the key `{key}` occurs twice"),
            Self::Key(KeyKind::Mixed) => {
                f.write_str("a table has either `key = value` fields or positional items, not both")
            }
            Self::TooDeep => write!(f, "tables nest deeper than {MAX_DEPTH} levels"),
            Self::MalformedNumber => f.write_str("expected a decimal number"),
            Self::NonFiniteNumber => f.write_str("the number is not finite"),
            Self::UnsupportedEscape(ch) => write!(
                f,
                "the escape `\\{}` is not allowed; use \\n \\r \\t \\\\ \\\" \\' or \\u{{...}}",
                ch.escape_default()
            ),
            Self::InvalidUnicodeEscape => {
                f.write_str("`\\u{...}` needs one Unicode scalar value in 1 to 8 hex digits")
            }
            Self::UnknownField(field) => write!(f, "the manifest has no field `{field}`"),
            Self::MissingField(field) => write!(f, "the manifest needs the field `{field}`"),
            Self::WrongType(expected) | Self::SettingWrongType(expected) => {
                write!(f, "expected {expected}")
            }
            Self::Empty => f.write_str("the text is empty"),
            Self::Whitespace => f.write_str("the text has white space at an end"),
            Self::ControlCharacter => {
                f.write_str("the text has a control or bidirectional-format character")
            }
            Self::DuplicateTag(tag) => write!(f, "the tag `{tag}` occurs twice"),
            Self::MalformedApiVersion => f.write_str("`apiVersion` must be an integer"),
            Self::UnsupportedApiVersion(version) => {
                write!(f, "`apiVersion` {version} is not supported; use 2")
            }
            Self::UnknownCapability(name) => write!(f, "the capability `{name}` is not known"),
            Self::UnsupportedCapability(name) => write!(
                f,
                "the Luau API has no member for the capability `{name}`; use `capabilities = {{}}`"
            ),
            Self::InvalidSettingId => f.write_str(
                "a setting id is non-empty text without outer space or control characters",
            ),
            Self::MissingSettingType => f.write_str("the setting needs a `type`"),
            Self::UnknownSettingType(ty) => write!(
                f,
                "the setting type `{ty}` is not one of boolean, number, string, string[], tile"
            ),
            Self::MissingSettingDefault => f.write_str("the setting needs a `default`"),
            Self::UnknownSettingField(field) => write!(f, "a setting has no field `{field}`"),
            Self::SettingFieldNotAllowed { field, ty } => {
                write!(f, "`{field}` is not allowed for a setting of type `{ty}`")
            }
            Self::NonFiniteBound => f.write_str("the number is not finite"),
            Self::InvertedRange => f.write_str("`max` is below `min`"),
            Self::DefaultOutsideRange => f.write_str("the default is outside `min` and `max`"),
            Self::EmptyOptions => f.write_str("`options` has no entry"),
            Self::DuplicateOption(option) => write!(f, "the option `{option}` occurs twice"),
            Self::DefaultNotOption(value) => {
                write!(f, "the default `{value}` is not one of `options`")
            }
            Self::UnknownOptionLabel(option) => {
                write!(f, "`optionLabels` names `{option}`, which is not an option")
            }
            Self::UnknownReference(key) => write!(f, "`showIf` names the unknown setting `{key}`"),
            Self::InvalidReference => {
                f.write_str("`showIf.key` must name another setting of type string or string[]")
            }
            Self::EmptyCondition => f.write_str("`showIf.anyOf` has no entry"),
            Self::DuplicateCondition(value) => {
                write!(f, "the `showIf.anyOf` value `{value}` occurs twice")
            }
            Self::UnknownOptionSource(source) => write!(
                f,
                "`optionsFrom` `{source}` is not one of loadouts, named-banks"
            ),
            Self::CompetingOptions => f.write_str("`options` and `optionsFrom` exclude each other"),
            Self::InvalidTile => {
                f.write_str("a tile is `{ x, z, level }` of integers, level 0 to 3")
            }
        }
    }
}

/// A refusal of the manifest reader, with the position of the cause.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManifestError {
    /// Zero-based byte range in the original source, with its byte-order mark.
    pub span: Range<usize>,
    /// The dotted path of the field, as `manifest.settings.mode.default`.
    pub field_path: String,
    /// The typed reason.
    pub reason: ManifestReason,
    line: usize,
    column: usize,
}

impl ManifestError {
    fn new(
        source: &str,
        span: Range<usize>,
        field_path: impl Into<String>,
        reason: ManifestReason,
    ) -> Self {
        let (line, column) = line_column(source, span.start);
        Self {
            span,
            field_path: field_path.into(),
            reason,
            line,
            column,
        }
    }

    /// The stable machine-readable code of the reason.
    pub fn code(&self) -> &'static str {
        self.reason.code()
    }

    /// The one-based line of the cause.
    pub fn line(&self) -> usize {
        self.line
    }

    /// The one-based column of the cause, in characters.
    pub fn column(&self) -> usize {
        self.column
    }
}

impl fmt::Display for ManifestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}: {} ({} at line {}, column {})",
            self.code(),
            self.reason,
            self.field_path,
            self.line,
            self.column
        )
    }
}

impl std::error::Error for ManifestError {}

/// Counts a line at each `\n`, as the Luau lexer does: `\r\n` is one break and
/// a lone `\r` is none.
fn line_column(source: &str, byte: usize) -> (usize, usize) {
    let mut line = 1;
    let mut column = 1;
    for (index, ch) in source.char_indices() {
        if index >= byte {
            break;
        }
        if ch == '\n' {
            line += 1;
            column = 1;
        } else {
            column += 1;
        }
    }
    (line, column)
}

/// The neutral manifest of a Luau file and the place of its statement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LuauManifest {
    manifest: ScriptManifest,
    statement: Range<usize>,
}

impl LuauManifest {
    /// The validated, language-neutral manifest.
    pub fn manifest(&self) -> &ScriptManifest {
        &self.manifest
    }

    /// Takes the neutral manifest.
    pub fn into_manifest(self) -> ScriptManifest {
        self.manifest
    }

    /// The byte range of the `manifest { ... }` statement in the source,
    /// with its optional closing `;`.
    pub fn statement(&self) -> Range<usize> {
        self.statement.clone()
    }

    /// The body that the run compiles: `source` with the statement and a
    /// leading byte-order mark replaced by spaces. Every line break stays, so
    /// a line number of the VM is a line number of the file. `source` must be
    /// the text that this manifest was read from.
    pub fn erased_source(&self, source: &str) -> String {
        erase(source, &self.statement)
    }
}

/// Reads the manifest of one `.luau` file without a VM.
pub fn read_manifest(source: &str) -> Result<LuauManifest, ManifestError> {
    match read_leading(source)? {
        Some(read) => Ok(read),
        None => Err(ManifestError::new(
            source,
            leading_trivia_end(source),
            "manifest",
            ManifestReason::MissingManifest,
        )),
    }
}

/// The body that the run compiles. `Ok(None)` when the first statement is not
/// `manifest`: the run then compiles the file as it is. A leading `manifest`
/// that the reader refuses is an error, so a broken manifest never reaches the
/// compiler.
pub(crate) fn erase_manifest(source: &str) -> Result<Option<String>, ManifestError> {
    Ok(read_leading(source)?.map(|read| read.erased_source(source)))
}

fn leading_trivia_end(source: &str) -> Range<usize> {
    let mut parser = Parser::new(source);
    // A refused comment leaves the cursor at the comment: that is the place to name.
    let _ = parser.trivia();
    let at = parser.pos.min(source.len());
    at..at
}

fn read_leading(source: &str) -> Result<Option<LuauManifest>, ManifestError> {
    if source.len() > SOURCE_LIMIT_BYTES {
        return Err(ManifestError::new(
            source,
            0..0,
            "manifest",
            ManifestReason::FileTooLarge {
                bytes: source.len(),
            },
        ));
    }
    let mut parser = Parser::new(source);
    parser.trivia()?;
    let start = parser.pos;
    if parser.identifier() != Some("manifest") {
        return Ok(None);
    }
    parser.trivia()?;
    if parser.peek() != Some(b'{') {
        return Err(parser.error(
            parser.pos..parser.pos,
            "manifest",
            ManifestReason::ExpectedTable,
        ));
    }
    let table = parser.table(1)?;
    let mut end = parser.pos;
    // A statement ends at the closing brace, or at one `;` after it. Without a
    // `;`, code that continues the expression would call the table's result.
    parser.trivia()?;
    if parser.peek() == Some(b';') {
        parser.pos += 1;
        end = parser.pos;
    } else if matches!(
        parser.peek(),
        Some(b'(' | b'[' | b'.' | b':' | b'{' | b'"' | b'\'' | b'`')
    ) {
        return Err(parser.error(
            parser.pos..parser.pos + 1,
            "manifest",
            ManifestReason::TrailingExpression,
        ));
    }
    let manifest = Projector { source }.manifest(&table)?;
    Ok(Some(LuauManifest {
        manifest,
        statement: start..end,
    }))
}

fn erase(source: &str, statement: &Range<usize>) -> String {
    let mut out = String::with_capacity(source.len());
    let (bom, rest) = match source.strip_prefix(BOM) {
        Some(rest) => {
            out.push_str(&" ".repeat(BOM.len()));
            (BOM.len(), rest)
        }
        None => (0, source),
    };
    let before = statement.start - bom;
    let inside = statement.end - bom;
    out.push_str(&rest[..before]);
    for ch in rest[before..inside].chars() {
        if matches!(ch, '\n' | '\r') {
            out.push(ch);
        } else {
            for _ in 0..ch.len_utf8() {
                out.push(' ');
            }
        }
    }
    out.push_str(&rest[inside..]);
    out
}

// ---------------------------------------------------------------------------
// The literal tree

#[derive(Debug)]
struct Spanned {
    span: Range<usize>,
    value: Value,
}

#[derive(Debug)]
enum Value {
    String(String),
    Number(f64),
    Boolean(bool),
    Table(Table),
}

#[derive(Debug, Default)]
struct Table {
    /// `key = value` and `["key"] = value` fields, in source order.
    entries: Vec<Entry>,
    /// Positional items, in source order.
    items: Vec<Spanned>,
}

#[derive(Debug)]
struct Entry {
    key: String,
    key_span: Range<usize>,
    value: Spanned,
}

impl Spanned {
    /// The fields of a table that has no positional item. `{}` is one.
    fn object(&self) -> Option<&[Entry]> {
        match &self.value {
            Value::Table(table) if table.items.is_empty() => Some(&table.entries),
            _ => None,
        }
    }

    /// The items of a table that has no field. `{}` is one.
    fn array(&self) -> Option<&[Spanned]> {
        match &self.value {
            Value::Table(table) if table.entries.is_empty() => Some(&table.items),
            _ => None,
        }
    }

    fn string(&self) -> Option<&str> {
        match &self.value {
            Value::String(text) => Some(text),
            _ => None,
        }
    }
}

fn get<'a>(entries: &'a [Entry], key: &str) -> Option<&'a Spanned> {
    entries
        .iter()
        .find(|entry| entry.key == key)
        .map(|entry| &entry.value)
}

// ---------------------------------------------------------------------------
// The parser

struct Parser<'a> {
    source: &'a str,
    bytes: &'a [u8],
    pos: usize,
}

type Parsed<T> = Result<T, ManifestError>;

fn is_identifier_start(byte: u8) -> bool {
    byte.is_ascii_alphabetic() || byte == b'_'
}

fn is_identifier_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

impl<'a> Parser<'a> {
    fn new(source: &'a str) -> Self {
        Self {
            source,
            bytes: source.as_bytes(),
            pos: if source.starts_with(BOM) {
                BOM.len()
            } else {
                0
            },
        }
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    fn peek_at(&self, offset: usize) -> Option<u8> {
        self.bytes.get(self.pos + offset).copied()
    }

    fn error(&self, span: Range<usize>, field_path: &str, reason: ManifestReason) -> ManifestError {
        ManifestError::new(self.source, span, field_path, reason)
    }

    fn fail<T>(&self, span: Range<usize>, reason: ManifestReason) -> Parsed<T> {
        Err(self.error(span, "manifest", reason))
    }

    /// Skips white space and comments. A directive such as `--!strict` is a comment.
    fn trivia(&mut self) -> Parsed<()> {
        loop {
            match self.peek() {
                Some(b' ' | b'\t' | b'\r' | b'\n' | 0x0b | 0x0c) => self.pos += 1,
                Some(b'-') if self.peek_at(1) == Some(b'-') => {
                    let start = self.pos;
                    self.pos += 2;
                    if let Some(level) = self.long_bracket_level() {
                        self.skip_long_bracket(start, level)?;
                    } else {
                        while !matches!(self.peek(), None | Some(b'\n')) {
                            self.pos += 1;
                        }
                    }
                }
                _ => return Ok(()),
            }
        }
    }

    /// At `[` with `=`s and a second `[`: the level of the bracket, and the
    /// cursor stays on the first `[`.
    fn long_bracket_level(&self) -> Option<usize> {
        if self.peek() != Some(b'[') {
            return None;
        }
        let mut level = 0;
        while self.peek_at(1 + level) == Some(b'=') {
            level += 1;
        }
        (self.peek_at(1 + level) == Some(b'[')).then_some(level)
    }

    fn skip_long_bracket(&mut self, start: usize, level: usize) -> Parsed<()> {
        let close = {
            let mut close = String::with_capacity(level + 2);
            close.push(']');
            close.push_str(&"=".repeat(level));
            close.push(']');
            close
        };
        let body = self.pos + level + 2;
        match self.source[body..].find(&close) {
            Some(offset) => {
                self.pos = body + offset + close.len();
                Ok(())
            }
            None => self.fail(
                start..self.source.len(),
                ManifestReason::UnterminatedComment,
            ),
        }
    }

    /// Reads a name at the cursor.
    fn identifier(&mut self) -> Option<&'a str> {
        let start = self.pos;
        if !self.peek().is_some_and(is_identifier_start) {
            return None;
        }
        while self.peek().is_some_and(is_identifier_byte) {
            self.pos += 1;
        }
        Some(&self.source[start..self.pos])
    }

    fn table(&mut self, depth: usize) -> Parsed<Spanned> {
        let open = self.pos;
        if depth > MAX_DEPTH {
            return self.fail(open..open + 1, ManifestReason::TooDeep);
        }
        self.pos += 1;
        let mut table = Table::default();
        loop {
            self.trivia()?;
            match self.peek() {
                None => return self.fail(open..self.source.len(), ManifestReason::UnexpectedEnd),
                Some(b'}') => {
                    self.pos += 1;
                    return Ok(Spanned {
                        span: open..self.pos,
                        value: Value::Table(table),
                    });
                }
                Some(_) => {}
            }
            self.item(&mut table, depth)?;
            self.trivia()?;
            match self.peek() {
                Some(b',' | b';') => self.pos += 1,
                Some(b'}') => {}
                None => return self.fail(open..self.source.len(), ManifestReason::UnexpectedEnd),
                Some(byte) => return Err(self.after_value(byte)),
            }
        }
    }

    /// The refusal for a byte that follows a value where a separator is due.
    fn after_value(&self, byte: u8) -> ManifestError {
        let span = self.pos..self.pos + 1;
        let reason = match byte {
            b'(' | b':' => ManifestReason::Expression(ExpressionKind::Call),
            b'+' | b'-' | b'*' | b'/' | b'%' | b'^' | b'#' | b'<' | b'>' | b'=' | b'~' | b'&'
            | b'|' | b'.' | b'[' => ManifestReason::Expression(ExpressionKind::Operator),
            _ => ManifestReason::UnexpectedToken("`,`, `;` or `}`"),
        };
        self.error(span, "manifest", reason)
    }

    fn item(&mut self, table: &mut Table, depth: usize) -> Parsed<()> {
        let item_start = self.pos;
        let named = match self.peek() {
            Some(b'[') if self.long_bracket_level().is_none() => Some(self.bracket_key()?),
            Some(byte) if is_identifier_start(byte) => self.name_key()?,
            _ => None,
        };
        match named {
            Some((key, key_span)) => {
                if table.entries.iter().any(|entry| entry.key == key) {
                    return self.fail(key_span, ManifestReason::Key(KeyKind::Duplicate(key)));
                }
                self.trivia()?;
                let value = self.value(depth)?;
                table.entries.push(Entry {
                    key,
                    key_span,
                    value,
                });
            }
            None => {
                self.pos = item_start;
                let value = self.value(depth)?;
                table.items.push(value);
            }
        }
        if !table.entries.is_empty() && !table.items.is_empty() {
            return self.fail(item_start..self.pos, ManifestReason::Key(KeyKind::Mixed));
        }
        Ok(())
    }

    /// `["text"] =`. A key that is not a quoted string is a computed key.
    fn bracket_key(&mut self) -> Parsed<(String, Range<usize>)> {
        let open = self.pos;
        self.pos += 1;
        self.trivia()?;
        if !matches!(self.peek(), Some(b'"' | b'\'')) {
            return self.fail(open..self.pos + 1, ManifestReason::Key(KeyKind::Computed));
        }
        let (key, span) = self.string()?;
        self.trivia()?;
        if self.peek() != Some(b']') {
            return self.fail(open..self.pos + 1, ManifestReason::Key(KeyKind::Computed));
        }
        self.pos += 1;
        self.trivia()?;
        self.equals()?;
        Ok((key, span))
    }

    /// `name =`. `None` when the name is not a key: the cursor is then unspecified.
    fn name_key(&mut self) -> Parsed<Option<(String, Range<usize>)>> {
        let start = self.pos;
        let Some(name) = self.identifier() else {
            return Ok(None);
        };
        let span = start..self.pos;
        self.trivia()?;
        if self.peek() != Some(b'=') || self.peek_at(1) == Some(b'=') {
            return Ok(None);
        }
        if RESERVED_WORDS.contains(&name) {
            return self.fail(
                span,
                ManifestReason::Key(KeyKind::Reserved(name.to_owned())),
            );
        }
        self.pos += 1;
        Ok(Some((name.to_owned(), span)))
    }

    fn equals(&mut self) -> Parsed<()> {
        if self.peek() == Some(b'=') && self.peek_at(1) != Some(b'=') {
            self.pos += 1;
            Ok(())
        } else {
            self.fail(
                self.pos..self.pos + 1,
                ManifestReason::UnexpectedToken("`=` after the key"),
            )
        }
    }

    fn value(&mut self, depth: usize) -> Parsed<Spanned> {
        let start = self.pos;
        let value = match self.peek() {
            None => return self.fail(start..start, ManifestReason::UnexpectedEnd),
            Some(b'{') => return self.table(depth + 1),
            Some(b'"' | b'\'') => Value::String(self.string()?.0),
            Some(b'`') => {
                return self.fail(
                    start..start + 1,
                    ManifestReason::Expression(ExpressionKind::InterpolatedString),
                );
            }
            Some(b'[') => {
                let kind = if self.long_bracket_level().is_some() {
                    ExpressionKind::LongString
                } else {
                    ExpressionKind::Operator
                };
                return self.fail(start..start + 1, ManifestReason::Expression(kind));
            }
            Some(b'-') => Value::Number(self.number()?),
            Some(byte) if byte.is_ascii_digit() => Value::Number(self.number()?),
            Some(byte) if is_identifier_start(byte) => self.word()?,
            Some(
                b'+' | b'*' | b'/' | b'%' | b'^' | b'#' | b'<' | b'>' | b'=' | b'~' | b'&' | b'|'
                | b'.',
            ) => {
                return self.fail(
                    start..start + 1,
                    ManifestReason::Expression(ExpressionKind::Operator),
                );
            }
            Some(b'(') => {
                return self.fail(
                    start..start + 1,
                    ManifestReason::Expression(ExpressionKind::Call),
                );
            }
            Some(_) => {
                return self.fail(start..start + 1, ManifestReason::UnexpectedToken("a value"));
            }
        };
        Ok(Spanned {
            span: start..self.pos,
            value,
        })
    }

    /// `true` or `false`; any other name is a refused expression.
    fn word(&mut self) -> Parsed<Value> {
        let start = self.pos;
        let word = self.identifier().unwrap_or_default();
        let span = start..self.pos;
        let kind = match word {
            "true" => return Ok(Value::Boolean(true)),
            "false" => return Ok(Value::Boolean(false)),
            "nil" => ExpressionKind::Nil,
            "function" => ExpressionKind::Function,
            "not" | "and" | "or" => ExpressionKind::Operator,
            _ => {
                let mut ahead = Parser {
                    source: self.source,
                    bytes: self.bytes,
                    pos: self.pos,
                };
                ahead.trivia()?;
                if matches!(ahead.peek(), Some(b'(' | b'{' | b'"' | b'\'' | b'`'))
                    || ahead.long_bracket_level().is_some()
                {
                    ExpressionKind::Call
                } else {
                    ExpressionKind::Identifier
                }
            }
        };
        self.fail(span, ManifestReason::Expression(kind))
    }

    /// A decimal number with an optional leading minus.
    fn number(&mut self) -> Parsed<f64> {
        let start = self.pos;
        if self.peek() == Some(b'-') {
            self.pos += 1;
        }
        if !self.peek().is_some_and(|byte| byte.is_ascii_digit()) {
            return self.fail(start..self.pos + 1, ManifestReason::MalformedNumber);
        }
        if self.peek() == Some(b'0') && matches!(self.peek_at(1), Some(b'x' | b'X' | b'b' | b'B')) {
            let mut end = self.pos + 2;
            while self.bytes.get(end).copied().is_some_and(is_identifier_byte) {
                end += 1;
            }
            return self.fail(
                start..end,
                ManifestReason::Expression(ExpressionKind::NonDecimalNumber),
            );
        }
        self.digits();
        if self.peek() == Some(b'.') {
            if !self.peek_at(1).is_some_and(|byte| byte.is_ascii_digit()) {
                return self.fail(start..self.pos + 1, ManifestReason::MalformedNumber);
            }
            self.pos += 1;
            self.digits();
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            self.pos += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.pos += 1;
            }
            if !self.peek().is_some_and(|byte| byte.is_ascii_digit()) {
                return self.fail(start..self.pos + 1, ManifestReason::MalformedNumber);
            }
            self.digits();
        }
        if self.peek().is_some_and(is_identifier_byte) {
            return self.fail(start..self.pos + 1, ManifestReason::MalformedNumber);
        }
        let span = start..self.pos;
        match self.source[span.clone()].parse::<f64>() {
            Ok(number) if number.is_finite() => Ok(number),
            Ok(_) => self.fail(span, ManifestReason::NonFiniteNumber),
            Err(_) => self.fail(span, ManifestReason::MalformedNumber),
        }
    }

    fn digits(&mut self) {
        while self.peek().is_some_and(|byte| byte.is_ascii_digit()) {
            self.pos += 1;
        }
    }

    /// A quoted string with the closed list of escapes.
    fn string(&mut self) -> Parsed<(String, Range<usize>)> {
        let start = self.pos;
        let quote = self.bytes[start];
        self.pos += 1;
        let mut text = String::new();
        loop {
            let Some(ch) = self.source[self.pos..].chars().next() else {
                return self.fail(start..self.pos, ManifestReason::UnterminatedString);
            };
            match ch {
                '\n' | '\r' => {
                    return self.fail(start..self.pos, ManifestReason::UnterminatedString);
                }
                '\\' => {
                    let escape_start = self.pos;
                    self.pos += 1;
                    let Some(escape) = self.source[self.pos..].chars().next() else {
                        return self.fail(start..self.pos, ManifestReason::UnterminatedString);
                    };
                    self.pos += escape.len_utf8();
                    match escape {
                        'n' => text.push('\n'),
                        'r' => text.push('\r'),
                        't' => text.push('\t'),
                        '\\' | '"' | '\'' => text.push(escape),
                        'u' => text.push(self.unicode_escape(escape_start)?),
                        other => {
                            return self.fail(
                                escape_start..self.pos,
                                ManifestReason::UnsupportedEscape(other),
                            );
                        }
                    }
                }
                _ if u32::from(ch) == u32::from(quote) => {
                    self.pos += 1;
                    return Ok((text, start..self.pos));
                }
                _ => {
                    text.push(ch);
                    self.pos += ch.len_utf8();
                }
            }
        }
    }

    /// `{HEX}` after `\u`: one Unicode scalar value in at most eight digits.
    fn unicode_escape(&mut self, escape_start: usize) -> Parsed<char> {
        let invalid =
            |this: &Self| this.fail(escape_start..this.pos, ManifestReason::InvalidUnicodeEscape);
        if self.peek() != Some(b'{') {
            return invalid(self);
        }
        self.pos += 1;
        let digits_start = self.pos;
        while self.peek().is_some_and(|byte| byte.is_ascii_hexdigit()) {
            self.pos += 1;
        }
        let digits = &self.source[digits_start..self.pos];
        if self.peek() != Some(b'}') || digits.is_empty() || digits.len() > 8 {
            return invalid(self);
        }
        self.pos += 1;
        match u32::from_str_radix(digits, 16)
            .ok()
            .and_then(char::from_u32)
        {
            Some(ch) => Ok(ch),
            None => invalid(self),
        }
    }
}

// ---------------------------------------------------------------------------
// The projection onto the neutral model

struct Projector<'a> {
    source: &'a str,
}

impl Projector<'_> {
    fn error(
        &self,
        span: &Range<usize>,
        field_path: &str,
        reason: ManifestReason,
    ) -> ManifestError {
        ManifestError::new(self.source, span.clone(), field_path, reason)
    }

    fn manifest(&self, table: &Spanned) -> Parsed<ScriptManifest> {
        let Some(entries) = table.object() else {
            return Err(self.error(&table.span, "manifest", ManifestReason::WrongType("object")));
        };
        let mut api_version = None;
        let mut texts: [Option<String>; 5] = Default::default();
        let mut tags = None;
        let mut settings = None;
        let mut capabilities = None;
        for entry in entries {
            let path = format!("manifest.{}", entry.key);
            let value = &entry.value;
            match entry.key.as_str() {
                "apiVersion" => api_version = Some(self.api_version(value, &path)?),
                "name" => texts[0] = Some(self.required_text(value, &path)?),
                "version" => texts[1] = Some(self.required_text(value, &path)?),
                "author" => texts[2] = Some(self.required_text(value, &path)?),
                "description" => texts[3] = Some(self.required_text(value, &path)?),
                "category" => texts[4] = Some(self.required_text(value, &path)?),
                "tags" => tags = Some(self.tags(value, &path)?),
                "settings" => settings = Some(self.settings(value, &path)?),
                "capabilities" => capabilities = Some(self.capabilities(value, &path)?),
                _ => {
                    return Err(self.error(
                        &entry.key_span,
                        &path,
                        ManifestReason::UnknownField(entry.key.clone()),
                    ));
                }
            }
        }
        let [name, version, author, description, category] = texts;
        let (
            Some(api_version),
            Some(name),
            Some(version),
            Some(author),
            Some(description),
            Some(category),
            Some(tags),
            Some(settings),
            Some(capabilities),
        ) = (
            api_version,
            name,
            version,
            author,
            description,
            category,
            tags,
            settings,
            capabilities,
        )
        else {
            let missing = MANIFEST_FIELDS
                .iter()
                .find(|field| get(entries, field).is_none())
                .copied()
                .unwrap_or("apiVersion");
            return Err(self.error(
                &table.span,
                &format!("manifest.{missing}"),
                ManifestReason::MissingField(missing),
            ));
        };
        Ok(ScriptManifest::new(
            api_version,
            ManifestText {
                name,
                version,
                author,
                description,
                category,
            },
            tags,
            settings,
            capabilities,
        ))
    }

    fn api_version(&self, value: &Spanned, path: &str) -> Parsed<u32> {
        let Value::Number(number) = value.value else {
            return Err(self.error(
                &value.span,
                path,
                ManifestReason::WrongType("integer numeric literal"),
            ));
        };
        if number.fract() != 0.0 || number < 0.0 || number > f64::from(u32::MAX) {
            return Err(self.error(&value.span, path, ManifestReason::MalformedApiVersion));
        }
        let version = number as u32;
        if version != 2 {
            return Err(self.error(
                &value.span,
                path,
                ManifestReason::UnsupportedApiVersion(version),
            ));
        }
        Ok(version)
    }

    /// A display text: control characters, then empty, then outer space.
    fn required_text(&self, value: &Spanned, path: &str) -> Parsed<String> {
        let Some(text) = value.string() else {
            return Err(self.error(&value.span, path, ManifestReason::WrongType("string")));
        };
        self.display_text(text, &value.span, path)?;
        if text.is_empty() {
            return Err(self.error(&value.span, path, ManifestReason::Empty));
        }
        if text.trim() != text {
            return Err(self.error(&value.span, path, ManifestReason::Whitespace));
        }
        Ok(text.to_owned())
    }

    fn display_text(&self, text: &str, span: &Range<usize>, path: &str) -> Parsed<()> {
        if text.chars().any(is_forbidden_display_character) {
            return Err(self.error(span, path, ManifestReason::ControlCharacter));
        }
        Ok(())
    }

    fn tags(&self, value: &Spanned, path: &str) -> Parsed<Vec<String>> {
        let Some(items) = value.array() else {
            return Err(self.error(
                &value.span,
                path,
                ManifestReason::WrongType("array of strings"),
            ));
        };
        let mut tags = Vec::with_capacity(items.len());
        let mut seen = HashSet::with_capacity(items.len());
        for item in items {
            let Some(tag) = item.string() else {
                return Err(self.error(&item.span, path, ManifestReason::WrongType("string")));
            };
            self.display_text(tag, &item.span, path)?;
            if tag.is_empty() {
                return Err(self.error(&item.span, path, ManifestReason::Empty));
            }
            if !seen.insert(tag) {
                return Err(self.error(
                    &item.span,
                    path,
                    ManifestReason::DuplicateTag(tag.to_owned()),
                ));
            }
            tags.push(tag.to_owned());
        }
        Ok(tags)
    }

    /// The neutral model has one capability, and no Luau API member uses it.
    /// So the only list that a Luau manifest can declare is the empty one.
    fn capabilities(&self, value: &Spanned, path: &str) -> Parsed<DeclaredHostCapabilities> {
        let Some(items) = value.array() else {
            return Err(self.error(
                &value.span,
                path,
                ManifestReason::WrongType("array of strings"),
            ));
        };
        let Some(item) = items.first() else {
            return Ok(DeclaredHostCapabilities::EMPTY);
        };
        let Some(name) = item.string() else {
            return Err(self.error(&item.span, path, ManifestReason::WrongType("string")));
        };
        let reason = match name {
            "host-packet-protocol" => ManifestReason::UnsupportedCapability(name.to_owned()),
            _ => ManifestReason::UnknownCapability(name.to_owned()),
        };
        Err(self.error(&item.span, path, reason))
    }

    fn settings(&self, value: &Spanned, path: &str) -> Parsed<Vec<SettingDef>> {
        let Some(entries) = value.object() else {
            return Err(self.error(&value.span, path, ManifestReason::WrongType("object")));
        };
        let mut settings = Vec::with_capacity(entries.len());
        let mut references = Vec::new();
        for entry in entries {
            let id = entry.key.as_str();
            let setting_path = format!("{path}.{id}");
            if id.is_empty() || id.trim() != id || id.chars().any(is_forbidden_display_character) {
                return Err(self.error(
                    &entry.key_span,
                    &setting_path,
                    ManifestReason::InvalidSettingId,
                ));
            }
            let Some(fields) = entry.value.object() else {
                return Err(self.error(
                    &entry.value.span,
                    &setting_path,
                    ManifestReason::SettingWrongType("object"),
                ));
            };
            let (setting, reference) = self.setting(id, &entry.value, fields, &setting_path)?;
            settings.push(setting);
            references.extend(reference);
        }
        for reference in references {
            let referenced = settings
                .iter()
                .find(|setting| setting.id == reference.key)
                .map(|setting| setting.ty.as_str());
            let Some(ty) = referenced else {
                return Err(self.error(
                    &reference.span,
                    &reference.path,
                    ManifestReason::UnknownReference(reference.key),
                ));
            };
            if reference.owner == reference.key || !matches!(ty, "string" | "string[]") {
                return Err(self.error(
                    &reference.span,
                    &reference.path,
                    ManifestReason::InvalidReference,
                ));
            }
        }
        Ok(settings)
    }

    fn setting(
        &self,
        id: &str,
        whole: &Spanned,
        fields: &[Entry],
        path: &str,
    ) -> Parsed<(SettingDef, Option<ShowReference>)> {
        for field in fields {
            if !SETTING_FIELDS.contains(&field.key.as_str()) {
                return Err(self.error(
                    &field.key_span,
                    &format!("{path}.{}", field.key),
                    ManifestReason::UnknownSettingField(field.key.clone()),
                ));
            }
        }
        let Some(ty_value) = get(fields, "type") else {
            return Err(self.error(
                &whole.span,
                &format!("{path}.type"),
                ManifestReason::MissingSettingType,
            ));
        };
        let ty = self.setting_string(ty_value, &format!("{path}.type"))?;
        if !matches!(ty, "boolean" | "number" | "string" | "string[]" | "tile") {
            return Err(self.error(
                &ty_value.span,
                &format!("{path}.type"),
                ManifestReason::UnknownSettingType(ty.to_owned()),
            ));
        }
        let Some(default) = get(fields, "default") else {
            return Err(self.error(
                &whole.span,
                &format!("{path}.default"),
                ManifestReason::MissingSettingDefault,
            ));
        };
        let default_text = self.default_text(default, ty, &format!("{path}.default"))?;
        let label = self.optional_text(fields, "label", path)?;
        let help = self.optional_text(fields, "help", path)?;
        let group = self.optional_text(fields, "group", path)?;
        let min = self.optional_number(fields, "min", ty, path)?;
        let max = self.optional_number(fields, "max", ty, path)?;
        if let (Some(min), Some(max_value)) = (min, max) {
            if min > max_value {
                let span = get(fields, "max").map_or(&whole.span, |value| &value.span);
                return Err(self.error(
                    span,
                    &format!("{path}.max"),
                    ManifestReason::InvertedRange,
                ));
            }
        }
        if let Value::Number(number) = default.value {
            if min.is_some_and(|min| number < min) || max.is_some_and(|max| number > max) {
                return Err(self.error(
                    &default.span,
                    &format!("{path}.default"),
                    ManifestReason::DefaultOutsideRange,
                ));
            }
        }
        if get(fields, "options").is_some() {
            if let Some(from) = get(fields, "optionsFrom") {
                return Err(self.error(
                    &from.span,
                    &format!("{path}.optionsFrom"),
                    ManifestReason::CompetingOptions,
                ));
            }
        }
        let options = self.options(fields, ty, path)?;
        let options_from = self.options_from(fields, ty, path)?;
        self.default_in_options(default, ty, &options, &format!("{path}.default"))?;
        let option_labels = self.option_labels(fields, ty, &options, path)?;
        let (show_if, reference) = self.show_if(fields, id, path)?;
        Ok((
            SettingDef {
                id: id.to_owned(),
                ty: ty.to_owned(),
                default: Some(default_text),
                label,
                min: min.map(|value| value.to_string()),
                max: max.map(|value| value.to_string()),
                step: None,
                options,
                option_labels,
                group,
                show_if,
                options_from,
                csv_toggle: None,
                help,
                item_option_spec: None,
            },
            reference,
        ))
    }

    fn setting_string<'v>(&self, value: &'v Spanned, path: &str) -> Parsed<&'v str> {
        value.string().ok_or_else(|| {
            self.error(
                &value.span,
                path,
                ManifestReason::SettingWrongType("string"),
            )
        })
    }

    fn optional_text(&self, fields: &[Entry], field: &str, path: &str) -> Parsed<Option<String>> {
        let Some(value) = get(fields, field) else {
            return Ok(None);
        };
        let value_path = format!("{path}.{field}");
        let text = self.setting_string(value, &value_path)?;
        self.display_text(text, &value.span, &value_path)?;
        Ok(Some(text.to_owned()))
    }

    fn not_allowed(&self, value: &Spanned, field: &str, ty: &str, path: &str) -> ManifestError {
        self.error(
            &value.span,
            &format!("{path}.{field}"),
            ManifestReason::SettingFieldNotAllowed {
                field: field.to_owned(),
                ty: ty.to_owned(),
            },
        )
    }

    fn optional_number(
        &self,
        fields: &[Entry],
        field: &str,
        ty: &str,
        path: &str,
    ) -> Parsed<Option<f64>> {
        let Some(value) = get(fields, field) else {
            return Ok(None);
        };
        if ty != "number" {
            return Err(self.not_allowed(value, field, ty, path));
        }
        match value.value {
            Value::Number(number) if number.is_finite() => Ok(Some(number)),
            Value::Number(_) => Err(self.error(
                &value.span,
                &format!("{path}.{field}"),
                ManifestReason::NonFiniteBound,
            )),
            _ => Err(self.error(
                &value.span,
                &format!("{path}.{field}"),
                ManifestReason::SettingWrongType("number"),
            )),
        }
    }

    /// The strings of a non-empty list, each distinct.
    fn distinct_strings<'v>(
        &self,
        value: &'v Spanned,
        path: &str,
        empty: ManifestReason,
        duplicate: fn(String) -> ManifestReason,
    ) -> Parsed<Vec<&'v str>> {
        let Some(items) = value.array() else {
            return Err(self.error(
                &value.span,
                path,
                ManifestReason::SettingWrongType("array of strings"),
            ));
        };
        if items.is_empty() {
            return Err(self.error(&value.span, path, empty));
        }
        let mut seen = HashSet::with_capacity(items.len());
        let mut strings = Vec::with_capacity(items.len());
        for item in items {
            let Some(text) = item.string() else {
                return Err(self.error(
                    &item.span,
                    path,
                    ManifestReason::SettingWrongType("string"),
                ));
            };
            if !seen.insert(text) {
                return Err(self.error(&item.span, path, duplicate(text.to_owned())));
            }
            strings.push(text);
        }
        Ok(strings)
    }

    fn options(&self, fields: &[Entry], ty: &str, path: &str) -> Parsed<Vec<String>> {
        let Some(value) = get(fields, "options") else {
            return Ok(Vec::new());
        };
        if !matches!(ty, "string" | "string[]") {
            return Err(self.not_allowed(value, "options", ty, path));
        }
        let options = self.distinct_strings(
            value,
            &format!("{path}.options"),
            ManifestReason::EmptyOptions,
            ManifestReason::DuplicateOption,
        )?;
        Ok(options.into_iter().map(str::to_owned).collect())
    }

    fn options_from(&self, fields: &[Entry], ty: &str, path: &str) -> Parsed<Option<String>> {
        let Some(value) = get(fields, "optionsFrom") else {
            return Ok(None);
        };
        if !matches!(ty, "string" | "string[]") {
            return Err(self.not_allowed(value, "optionsFrom", ty, path));
        }
        let from = self.setting_string(value, &format!("{path}.optionsFrom"))?;
        if !matches!(from, "loadouts" | "named-banks") {
            return Err(self.error(
                &value.span,
                &format!("{path}.optionsFrom"),
                ManifestReason::UnknownOptionSource(from.to_owned()),
            ));
        }
        Ok(Some(from.to_owned()))
    }

    fn default_in_options(
        &self,
        default: &Spanned,
        ty: &str,
        options: &[String],
        path: &str,
    ) -> Parsed<()> {
        if options.is_empty() {
            return Ok(());
        }
        let values: Vec<&str> = match (&default.value, ty) {
            (Value::String(value), "string") => vec![value],
            (Value::Table(table), "string[]") => {
                table.items.iter().filter_map(Spanned::string).collect()
            }
            _ => return Ok(()),
        };
        for value in values {
            if !options.iter().any(|option| option == value) {
                return Err(self.error(
                    &default.span,
                    path,
                    ManifestReason::DefaultNotOption(value.to_owned()),
                ));
            }
        }
        Ok(())
    }

    fn option_labels(
        &self,
        fields: &[Entry],
        ty: &str,
        options: &[String],
        path: &str,
    ) -> Parsed<Vec<String>> {
        let Some(value) = get(fields, "optionLabels") else {
            return Ok(Vec::new());
        };
        if !matches!(ty, "string" | "string[]") {
            return Err(self.not_allowed(value, "optionLabels", ty, path));
        }
        let labels_path = format!("{path}.optionLabels");
        let Some(labels) = value.object() else {
            return Err(self.error(
                &value.span,
                &labels_path,
                ManifestReason::SettingWrongType("object"),
            ));
        };
        let mut by_option: Vec<(&str, &str)> = Vec::with_capacity(labels.len());
        for label in labels {
            let label_path = format!("{labels_path}.{}", label.key);
            let text = self.setting_string(&label.value, &label_path)?;
            self.display_text(text, &label.value.span, &label_path)?;
            if !options.iter().any(|option| option == &label.key) {
                return Err(self.error(
                    &label.key_span,
                    &label_path,
                    ManifestReason::UnknownOptionLabel(label.key.clone()),
                ));
            }
            by_option.push((&label.key, text));
        }
        Ok(options
            .iter()
            .map(|option| {
                by_option
                    .iter()
                    .find(|(key, _)| *key == option)
                    .map_or_else(|| option.clone(), |(_, text)| (*text).to_owned())
            })
            .collect())
    }

    fn show_if(
        &self,
        fields: &[Entry],
        owner: &str,
        path: &str,
    ) -> Parsed<(Option<String>, Option<ShowReference>)> {
        let Some(value) = get(fields, "showIf") else {
            return Ok((None, None));
        };
        let condition_path = format!("{path}.showIf");
        let Some(condition) = value.object() else {
            return Err(self.error(
                &value.span,
                &condition_path,
                ManifestReason::SettingWrongType("object"),
            ));
        };
        for field in condition {
            if !matches!(field.key.as_str(), "key" | "anyOf") {
                return Err(self.error(
                    &field.key_span,
                    &format!("{condition_path}.{}", field.key),
                    ManifestReason::UnknownSettingField(field.key.clone()),
                ));
            }
        }
        let Some(key_value) = get(condition, "key") else {
            return Err(self.error(
                &value.span,
                &format!("{condition_path}.key"),
                ManifestReason::SettingWrongType("string"),
            ));
        };
        let key_path = format!("{condition_path}.key");
        let key = self.setting_string(key_value, &key_path)?.to_owned();
        let Some(any_of) = get(condition, "anyOf") else {
            return Err(self.error(
                &value.span,
                &format!("{condition_path}.anyOf"),
                ManifestReason::SettingWrongType("nonempty array of strings"),
            ));
        };
        let any_of_path = format!("{condition_path}.anyOf");
        if any_of.array().is_none() {
            return Err(self.error(
                &any_of.span,
                &any_of_path,
                ManifestReason::SettingWrongType("nonempty array of strings"),
            ));
        }
        let cooked = self.distinct_strings(
            any_of,
            &any_of_path,
            ManifestReason::EmptyCondition,
            ManifestReason::DuplicateCondition,
        )?;
        let serialized = serde_json::json!({
            "key": key,
            "anyOf": cooked,
        })
        .to_string();
        Ok((
            Some(serialized),
            Some(ShowReference {
                owner: owner.to_owned(),
                key,
                path: key_path,
                span: key_value.span.clone(),
            }),
        ))
    }

    fn default_text(&self, value: &Spanned, ty: &str, path: &str) -> Parsed<String> {
        let mismatch = match (ty, &value.value) {
            ("boolean", Value::Boolean(value)) => return Ok(value.to_string()),
            ("number", Value::Number(value)) => return Ok(value.to_string()),
            ("string", Value::String(value)) => return Ok(value.clone()),
            ("string[]", Value::Table(_)) => {
                let Some(items) = value.array() else {
                    return Err(self.error(
                        &value.span,
                        path,
                        ManifestReason::SettingWrongType("array of strings"),
                    ));
                };
                let mut strings = Vec::with_capacity(items.len());
                for item in items {
                    let Some(text) = item.string() else {
                        return Err(self.error(
                            &item.span,
                            path,
                            ManifestReason::SettingWrongType("array of strings"),
                        ));
                    };
                    strings.push(text);
                }
                return Ok(serde_json::json!(strings).to_string());
            }
            ("tile", Value::Table(_)) => {
                let tile = value.object().and_then(parse_tile);
                return match tile {
                    Some(tile) => Ok(tile.to_string()),
                    None => Err(self.error(&value.span, path, ManifestReason::InvalidTile)),
                };
            }
            ("boolean", _) => "boolean",
            ("number", _) => "number",
            ("string", _) => "string",
            ("string[]", _) => "array of strings",
            _ => "tile object",
        };
        Err(self.error(
            &value.span,
            path,
            ManifestReason::SettingWrongType(mismatch),
        ))
    }
}

struct ShowReference {
    owner: String,
    key: String,
    path: String,
    span: Range<usize>,
}

/// `{ x = .., z = .., level = .. }` of integers, level 0 to 3.
fn parse_tile(entries: &[Entry]) -> Option<serde_json::Value> {
    if entries.len() != 3 {
        return None;
    }
    let mut x = None;
    let mut z = None;
    let mut level = None;
    for entry in entries {
        let Value::Number(number) = entry.value.value else {
            return None;
        };
        if number.fract() != 0.0 || number < f64::from(i32::MIN) || number > f64::from(i32::MAX) {
            return None;
        }
        match entry.key.as_str() {
            "x" => x = Some(number as i32),
            "z" => z = Some(number as i32),
            "level" if (0.0..=3.0).contains(&number) => level = Some(number as u8),
            _ => return None,
        }
    }
    Some(serde_json::json!({
        "x": x?,
        "z": z?,
        "level": level?,
    }))
}

/// Control characters and the bidirectional-format characters that text must not hold.
fn is_forbidden_display_character(ch: char) -> bool {
    matches!(u32::from(ch),
        0x0000..=0x001f
            | 0x007f..=0x009f
            | 0x2028..=0x2029
            | 0x202a..=0x202e
            | 0x2066..=0x2069
    )
}
