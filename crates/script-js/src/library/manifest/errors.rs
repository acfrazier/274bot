use std::fmt;
use std::ops::Range;
use std::path::PathBuf;

/// Typed reason for a rejected script declaration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeclarationReason {
    /// The source does not parse as the selected JavaScript or TypeScript media.
    Syntax,
    /// The reserved export has an invalid declaration form.
    Export(ExportReason),
    /// A literal value or property violates the closed grammar.
    Literal(LiteralReason),
    /// A manifest field is absent or invalid.
    Manifest(ManifestReason),
    /// A host capability is invalid for this declaration.
    Capability(CapabilityReason),
    /// A setting definition is invalid.
    Setting(SettingReason),
}

/// Invalid forms of the reserved `manifest` or `hostCapabilities` exports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExportReason {
    /// More than one declaration supplies the reserved export.
    Duplicate,
    /// The export is not one direct, simple `export const` declaration.
    NotDirectConst,
    /// A direct const declaration has no initializer.
    MissingInitializer,
    /// Runtime code reads or re-exports an erased declaration.
    RuntimeReference,
}

/// Closed expression categories used by literal diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExpressionKind {
    Identifier,
    PropertyAccess,
    Call,
    NewExpression,
    Function,
    ArrowFunction,
    Class,
    Spread,
    ShorthandProperty,
    Getter,
    Setter,
    Method,
    Template,
    Null,
    Undefined,
    NaN,
    Infinity,
    BigInt,
    UnaryExpression,
    BinaryExpression,
    ConditionalExpression,
    Assignment,
    Sequence,
    Await,
    Yield,
    Update,
    This,
    Super,
    Import,
    RegularExpression,
    Jsx,
    OtherExpression,
}

impl ExpressionKind {
    /// Stable spelling used by declaration diagnostics.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Identifier => "identifier",
            Self::PropertyAccess => "property access",
            Self::Call => "call",
            Self::NewExpression => "new expression",
            Self::Function => "function",
            Self::ArrowFunction => "arrow function",
            Self::Class => "class",
            Self::Spread => "spread",
            Self::ShorthandProperty => "shorthand property",
            Self::Getter => "getter",
            Self::Setter => "setter",
            Self::Method => "method",
            Self::Template => "template",
            Self::Null => "null",
            Self::Undefined => "undefined",
            Self::NaN => "NaN",
            Self::Infinity => "Infinity",
            Self::BigInt => "BigInt",
            Self::UnaryExpression => "unary expression",
            Self::BinaryExpression => "binary expression",
            Self::ConditionalExpression => "conditional expression",
            Self::Assignment => "assignment",
            Self::Sequence => "sequence",
            Self::Await => "await",
            Self::Yield => "yield",
            Self::Update => "update",
            Self::This => "this",
            Self::Super => "super",
            Self::Import => "import",
            Self::RegularExpression => "regular expression",
            Self::Jsx => "JSX",
            Self::OtherExpression => "other expression",
        }
    }
}

/// TypeScript wrappers that cannot appear in a compile-time declaration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnsupportedWrapper {
    /// The TypeScript non-null assertion operator.
    TsNonNull,
}

/// Invalid property-name categories in the closed literal grammar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyKind {
    Numeric,
    BigInt,
}

/// Literal grammar failures.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LiteralReason {
    /// The expression is not a supported literal.
    Expression(ExpressionKind),
    /// The wrapper is not accepted around a literal.
    UnsupportedWrapper(UnsupportedWrapper),
    /// The property name is computed.
    ComputedKey,
    /// The property name has a non-string key type.
    KeyKind(KeyKind),
    /// The object uses the special prototype key.
    PrototypeKey,
    /// The object repeats a cooked property key.
    DuplicateKey(String),
    /// The array contains a hole.
    SparseArray,
    /// The numeric value is not finite.
    NonFinite,
    /// A string token contains an unpaired UTF-16 surrogate escape.
    LoneSurrogate,
}

/// Manifest-level validation failures.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ManifestReason {
    MissingDeclaration,
    MissingField(&'static str),
    UnknownField(String),
    WrongType(&'static str),
    Empty,
    Whitespace,
    ControlCharacter,
    DuplicateTag(String),
    UnsupportedApiVersion(u32),
    MalformedApiVersion,
    ConflictingExport(String),
    WrongShape,
    MissingTick,
}

/// Capability declaration failures.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CapabilityReason {
    WrongFamily,
    Unknown(String),
    Duplicate(String),
}

/// Strict setting-schema validation failures.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SettingReason {
    InvalidId,
    MissingType,
    UnknownType(String),
    MissingDefault,
    UnknownField(String),
    WrongType(&'static str),
    FieldNotAllowed(Box<(String, String)>),
    NonFinite,
    InvertedRange,
    DefaultOutsideRange,
    EmptyOptions,
    DuplicateOption(String),
    DefaultNotOption(String),
    UnknownOptionLabel(String),
    UnknownReference(String),
    InvalidReference,
    EmptyCondition,
    DuplicateCondition(String),
    UnknownOptionSource(String),
    CompetingOptions,
    InvalidTile,
}

/// Structured source location and stable diagnostic for a rejected declaration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptDeclarationError {
    /// The origin file that contains the rejected declaration.
    pub path: PathBuf,
    /// Zero-based byte range in the original, BOM-preserving source.
    pub span: Range<usize>,
    /// Stable dotted path to the manifest field or declaration.
    pub field_path: String,
    /// Typed refusal reason.
    pub reason: DeclarationReason,
    line: usize,
    column: usize,
}

impl ScriptDeclarationError {
    pub(super) fn new(
        path: &std::path::Path,
        source: &str,
        span: Range<usize>,
        field_path: impl Into<String>,
        reason: DeclarationReason,
    ) -> Self {
        let (line, column) = line_column(source, span.start);
        Self {
            path: path.to_path_buf(),
            span,
            field_path: field_path.into(),
            reason,
            line,
            column,
        }
    }

    /// Returns the stable machine-readable diagnostic code.
    pub fn code(&self) -> &'static str {
        match &self.reason {
            DeclarationReason::Syntax => "script-source-syntax",
            DeclarationReason::Export(ExportReason::Duplicate) => "script-declaration-duplicate",
            DeclarationReason::Export(ExportReason::NotDirectConst) => "script-declaration-form",
            DeclarationReason::Export(ExportReason::MissingInitializer) => {
                "script-declaration-initializer"
            }
            DeclarationReason::Export(ExportReason::RuntimeReference) => {
                "script-declaration-runtime-reference"
            }
            DeclarationReason::Literal(LiteralReason::Expression(_)) => {
                "script-declaration-nonliteral"
            }
            DeclarationReason::Literal(LiteralReason::UnsupportedWrapper(_)) => {
                "script-declaration-wrapper"
            }
            DeclarationReason::Literal(LiteralReason::ComputedKey) => {
                "script-declaration-computed-key"
            }
            DeclarationReason::Literal(LiteralReason::KeyKind(_)) => "script-declaration-key-kind",
            DeclarationReason::Literal(LiteralReason::PrototypeKey) => {
                "script-declaration-prototype-key"
            }
            DeclarationReason::Literal(LiteralReason::DuplicateKey(_)) => {
                "script-declaration-duplicate-key"
            }
            DeclarationReason::Literal(LiteralReason::SparseArray) => {
                "script-declaration-sparse-array"
            }
            DeclarationReason::Literal(LiteralReason::NonFinite) => {
                "script-declaration-number-nonfinite"
            }
            DeclarationReason::Literal(LiteralReason::LoneSurrogate) => {
                "script-declaration-lone-surrogate"
            }
            DeclarationReason::Manifest(ManifestReason::MissingDeclaration) => "manifest-required",
            DeclarationReason::Manifest(ManifestReason::MissingField(_)) => {
                "manifest-required-field"
            }
            DeclarationReason::Manifest(ManifestReason::UnknownField(_)) => {
                "manifest-unknown-field"
            }
            DeclarationReason::Manifest(ManifestReason::WrongType(_)) => "manifest-field-type",
            DeclarationReason::Manifest(ManifestReason::Empty) => "manifest-empty-field",
            DeclarationReason::Manifest(ManifestReason::Whitespace) => "manifest-field-whitespace",
            DeclarationReason::Manifest(ManifestReason::ControlCharacter) => {
                "manifest-field-control"
            }
            DeclarationReason::Manifest(ManifestReason::DuplicateTag(_)) => {
                "manifest-duplicate-tag"
            }
            DeclarationReason::Manifest(ManifestReason::UnsupportedApiVersion(_)) => {
                "api-version-unsupported"
            }
            DeclarationReason::Manifest(ManifestReason::MalformedApiVersion) => {
                "api-version-malformed"
            }
            DeclarationReason::Manifest(ManifestReason::ConflictingExport(_)) => {
                "manifest-conflicting-export"
            }
            DeclarationReason::Manifest(ManifestReason::WrongShape) => "api-version-conflict",
            DeclarationReason::Manifest(ManifestReason::MissingTick) => "api-version-missing-tick",
            DeclarationReason::Capability(CapabilityReason::WrongFamily) => {
                "host-capabilities-api-conflict"
            }
            DeclarationReason::Capability(CapabilityReason::Unknown(_)) => {
                "host-capability-unknown"
            }
            DeclarationReason::Capability(CapabilityReason::Duplicate(_)) => {
                "host-capability-duplicate"
            }
            DeclarationReason::Setting(SettingReason::InvalidId) => "setting-id-invalid",
            DeclarationReason::Setting(SettingReason::MissingType) => "setting-required-type",
            DeclarationReason::Setting(SettingReason::UnknownType(_)) => "setting-unknown-type",
            DeclarationReason::Setting(SettingReason::MissingDefault) => "setting-required-default",
            DeclarationReason::Setting(SettingReason::UnknownField(_)) => "setting-unknown-field",
            DeclarationReason::Setting(SettingReason::WrongType(_)) => "setting-field-type",
            DeclarationReason::Setting(SettingReason::FieldNotAllowed(_)) => {
                "setting-field-not-allowed"
            }
            DeclarationReason::Setting(SettingReason::NonFinite) => "setting-number-nonfinite",
            DeclarationReason::Setting(SettingReason::InvertedRange) => "setting-range-inverted",
            DeclarationReason::Setting(SettingReason::DefaultOutsideRange) => {
                "setting-default-range"
            }
            DeclarationReason::Setting(SettingReason::EmptyOptions) => "setting-options-empty",
            DeclarationReason::Setting(SettingReason::DuplicateOption(_)) => {
                "setting-option-duplicate"
            }
            DeclarationReason::Setting(SettingReason::DefaultNotOption(_)) => {
                "setting-default-option"
            }
            DeclarationReason::Setting(SettingReason::UnknownOptionLabel(_)) => {
                "setting-option-label-key"
            }
            DeclarationReason::Setting(SettingReason::UnknownReference(_)) => {
                "setting-reference-missing"
            }
            DeclarationReason::Setting(SettingReason::InvalidReference) => {
                "setting-reference-invalid"
            }
            DeclarationReason::Setting(SettingReason::EmptyCondition) => "setting-condition-empty",
            DeclarationReason::Setting(SettingReason::DuplicateCondition(_)) => {
                "setting-condition-duplicate"
            }
            DeclarationReason::Setting(SettingReason::UnknownOptionSource(_)) => {
                "setting-option-source-unknown"
            }
            DeclarationReason::Setting(SettingReason::CompetingOptions) => {
                "setting-options-conflict"
            }
            DeclarationReason::Setting(SettingReason::InvalidTile) => "setting-tile-invalid",
        }
    }

    /// Returns the stable, plain-text reason associated with this diagnostic.
    pub fn reason_text(&self) -> String {
        match &self.reason {
            DeclarationReason::Syntax => {
                "source is not a valid JavaScript or TypeScript module".into()
            }
            DeclarationReason::Export(ExportReason::Duplicate) => {
                "declaration must occur exactly once".into()
            }
            DeclarationReason::Export(ExportReason::NotDirectConst) => {
                "declaration must be a direct export const".into()
            }
            DeclarationReason::Export(ExportReason::MissingInitializer) => {
                "declaration has no initializer".into()
            }
            DeclarationReason::Export(ExportReason::RuntimeReference) => {
                "declaration is host metadata and cannot be used at runtime".into()
            }
            DeclarationReason::Literal(LiteralReason::Expression(kind)) => {
                format!("expected a literal value, found {}", kind.as_str())
            }
            DeclarationReason::Literal(LiteralReason::UnsupportedWrapper(
                UnsupportedWrapper::TsNonNull,
            )) => "non-null assertion is not allowed".into(),
            DeclarationReason::Literal(LiteralReason::ComputedKey) => {
                "computed property names are not allowed".into()
            }
            DeclarationReason::Literal(LiteralReason::KeyKind(kind)) => {
                let kind = match kind {
                    KeyKind::Numeric => "numeric",
                    KeyKind::BigInt => "BigInt",
                };
                format!("{kind} property names are not allowed")
            }
            DeclarationReason::Literal(LiteralReason::PrototypeKey) => {
                "__proto__ property is not allowed".into()
            }
            DeclarationReason::Literal(LiteralReason::DuplicateKey(key)) => {
                format!("duplicate property {}", quote(key))
            }
            DeclarationReason::Literal(LiteralReason::SparseArray) => {
                "array holes are not allowed".into()
            }
            DeclarationReason::Literal(LiteralReason::NonFinite) => "number must be finite".into(),
            DeclarationReason::Literal(LiteralReason::LoneSurrogate) => {
                "unpaired surrogate escape is not allowed".into()
            }
            DeclarationReason::Manifest(ManifestReason::MissingDeclaration) => {
                "native JavaScript and TypeScript scripts require a manifest".into()
            }
            DeclarationReason::Manifest(ManifestReason::MissingField(field)) => {
                format!("required field {} is missing", quote(field))
            }
            DeclarationReason::Manifest(ManifestReason::UnknownField(field)) => {
                format!("unknown field {}", quote(field))
            }
            DeclarationReason::Manifest(ManifestReason::WrongType(expected)) => {
                format!("expected {expected}")
            }
            DeclarationReason::Manifest(ManifestReason::Empty) => "value must not be empty".into(),
            DeclarationReason::Manifest(ManifestReason::Whitespace) => {
                "leading or trailing whitespace is not allowed".into()
            }
            DeclarationReason::Manifest(ManifestReason::ControlCharacter) => {
                "forbidden display character is not allowed".into()
            }
            DeclarationReason::Manifest(ManifestReason::DuplicateTag(tag)) => {
                format!("duplicate tag {}", quote(tag))
            }
            DeclarationReason::Manifest(ManifestReason::UnsupportedApiVersion(version)) => {
                format!("unsupported apiVersion {version}")
            }
            DeclarationReason::Manifest(ManifestReason::MalformedApiVersion) => {
                "apiVersion must be an integer numeric literal".into()
            }
            DeclarationReason::Manifest(ManifestReason::ConflictingExport(name)) => {
                format!("manifest must not be combined with export {}", quote(name))
            }
            DeclarationReason::Manifest(ManifestReason::WrongShape) => {
                "manifest requires the native v2 tick shape".into()
            }
            DeclarationReason::Manifest(ManifestReason::MissingTick) => {
                "manifest requires an exported tick function".into()
            }
            DeclarationReason::Capability(CapabilityReason::WrongFamily) => {
                "hostCapabilities is a compat v1 extension only".into()
            }
            DeclarationReason::Capability(CapabilityReason::Unknown(name)) => {
                format!("unknown host capability {}", quote(name))
            }
            DeclarationReason::Capability(CapabilityReason::Duplicate(name)) => {
                format!("duplicate host capability {}", quote(name))
            }
            DeclarationReason::Setting(SettingReason::InvalidId) => {
                "setting id must be a nonempty single-line string without surrounding whitespace"
                    .into()
            }
            DeclarationReason::Setting(SettingReason::MissingType) => {
                "setting type is missing".into()
            }
            DeclarationReason::Setting(SettingReason::UnknownType(name)) => {
                format!("unknown setting type {}", quote(name))
            }
            DeclarationReason::Setting(SettingReason::MissingDefault) => {
                "setting default is missing".into()
            }
            DeclarationReason::Setting(SettingReason::UnknownField(name)) => {
                format!("unknown setting field {}", quote(name))
            }
            DeclarationReason::Setting(SettingReason::WrongType(expected)) => {
                format!("expected {expected}")
            }
            DeclarationReason::Setting(SettingReason::FieldNotAllowed(fields)) => {
                format!(
                    "{} is not allowed for setting type {}",
                    quote(&fields.0),
                    quote(&fields.1)
                )
            }
            DeclarationReason::Setting(SettingReason::NonFinite) => "number must be finite".into(),
            DeclarationReason::Setting(SettingReason::InvertedRange) => {
                "min must not exceed max".into()
            }
            DeclarationReason::Setting(SettingReason::DefaultOutsideRange) => {
                "default is outside min and max".into()
            }
            DeclarationReason::Setting(SettingReason::EmptyOptions) => {
                "options must not be empty".into()
            }
            DeclarationReason::Setting(SettingReason::DuplicateOption(value)) => {
                format!("duplicate option {}", quote(value))
            }
            DeclarationReason::Setting(SettingReason::DefaultNotOption(value)) => {
                format!("default value {} is not an option", quote(value))
            }
            DeclarationReason::Setting(SettingReason::UnknownOptionLabel(value)) => {
                format!("option label key {} is not an option", quote(value))
            }
            DeclarationReason::Setting(SettingReason::UnknownReference(key)) => {
                format!("referenced setting {} does not exist", quote(key))
            }
            DeclarationReason::Setting(SettingReason::InvalidReference) => {
                "reference has the wrong type or refers to itself".into()
            }
            DeclarationReason::Setting(SettingReason::EmptyCondition) => {
                "showIf.anyOf must not be empty".into()
            }
            DeclarationReason::Setting(SettingReason::DuplicateCondition(value)) => {
                format!("duplicate showIf value {}", quote(value))
            }
            DeclarationReason::Setting(SettingReason::UnknownOptionSource(name)) => {
                format!("unknown option source {}", quote(name))
            }
            DeclarationReason::Setting(SettingReason::CompetingOptions) => {
                "options and optionsFrom must not be combined".into()
            }
            DeclarationReason::Setting(SettingReason::InvalidTile) => {
                "tile requires bounded integer x, z, and level".into()
            }
        }
    }
}

impl fmt::Display for ScriptDeclarationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}:{}:{} {} at {}: {}",
            self.path.display(),
            self.line,
            self.column,
            self.code(),
            self.field_path,
            self.reason_text()
        )
    }
}

impl std::error::Error for ScriptDeclarationError {}

/// An unresolved module needed to decide whether a reserved export is supplied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnresolvedModuleError {
    /// The module that contains the unresolved import or re-export.
    pub path: PathBuf,
    /// Zero-based byte range in the original source.
    pub span: Range<usize>,
    /// The unresolved module specifier.
    pub specifier: String,
}

impl fmt::Display for UnresolvedModuleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}: unresolved module {}",
            self.path.display(),
            quote(&self.specifier)
        )
    }
}

impl std::error::Error for UnresolvedModuleError {}

/// Error returned by static declaration extraction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExtractionError {
    /// A source declaration failed typed validation.
    Declaration(ScriptDeclarationError),
    /// An unresolved source prevents safe export-name analysis.
    UnresolvedModule(UnresolvedModuleError),
}

impl fmt::Display for ExtractionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Declaration(error) => error.fmt(f),
            Self::UnresolvedModule(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for ExtractionError {}

impl From<ScriptDeclarationError> for ExtractionError {
    fn from(value: ScriptDeclarationError) -> Self {
        Self::Declaration(value)
    }
}

impl From<UnresolvedModuleError> for ExtractionError {
    fn from(value: UnresolvedModuleError) -> Self {
        Self::UnresolvedModule(value)
    }
}

fn quote(value: &str) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| format!("\"{value}\""))
}

fn line_column(source: &str, byte: usize) -> (usize, usize) {
    let mut end = byte.min(source.len());
    while !source.is_char_boundary(end) {
        end -= 1;
    }
    let mut line = 1;
    let mut column = 1;
    let mut previous_cr = false;
    for ch in source[..end].chars() {
        match ch {
            '\r' => {
                line += 1;
                column = 1;
                previous_cr = true;
            }
            '\n' => {
                if !previous_cr {
                    line += 1;
                }
                column = 1;
                previous_cr = false;
            }
            '\u{2028}' | '\u{2029}' => {
                line += 1;
                column = 1;
                previous_cr = false;
            }
            _ => {
                column += 1;
                previous_cr = false;
            }
        }
    }
    (line, column)
}
