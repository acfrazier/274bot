mod errors;
mod exports;
mod literal;
mod schema;

pub use errors::{
    CapabilityReason, DeclarationReason, ExportReason, ExpressionKind, ExtractionError, KeyKind,
    LiteralReason, ManifestReason, ScriptDeclarationError, SettingReason, UnresolvedModuleError,
    UnsupportedWrapper,
};

use crate::library::ast::strip_bom;
use crate::library::shape::LoadShape;
use deno_ast::swc::common::Span;
use deno_ast::{MediaType, ParseParams, ParsedSource, StartSourcePos};
use exports::{DirectConst, ExportSupplier, ModuleGraph, ModuleNode};
use literal::{parse_literal, LiteralValue, SpannedValue};
use script::manifest::{DeclaredHostCapabilities, HostCapability, ManifestText, ScriptManifest};
use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::path::Path;

/// A syntax-recognized legacy compatibility shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompatShape {
    /// A `defineBot` call or `__rs2b0tManifest` object property.
    DefineBot,
    /// A default-exported class that extends a supported compat base.
    Class,
}

/// Result of static source classification and declaration extraction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScriptAnalysis {
    /// The module has no recognized native or compat shape.
    NotBotShape,
    /// A recognized default-export compat module.
    Compat {
        shape: CompatShape,
        capabilities: DeclaredHostCapabilities,
        erased_spans: Vec<Range<usize>>,
    },
    /// A native entry with a checked manifest and direct exported `tick` function.
    Native {
        manifest: ScriptManifest,
        tick_span: Range<usize>,
        erased_spans: Vec<Range<usize>>,
    },
}

/// Analyze a JavaScript or TypeScript entry without creating or entering a V8 isolate.
///
/// The parser uses JavaScript media for `.js`, `.mjs`, and `.cjs` files. It uses
/// TypeScript media for `.ts` files. One leading BOM is removed from each parser
/// view, while every returned span addresses the original BOM-preserving bytes.
pub fn analyze(path: &Path, source: &str) -> Result<ScriptAnalysis, ExtractionError> {
    let graph = ModuleGraph::new(path, source)?;
    if graph.has_runtime_default() {
        let Some(shape) = graph.compat_shape() else {
            reject_non_compat_host_capabilities(&graph)?;
            return Ok(ScriptAnalysis::NotBotShape);
        };
        let capabilities = extract_host_capabilities(&graph)?;
        let erased_spans = graph
            .suppliers("hostCapabilities")
            .into_iter()
            .filter_map(|supplier| {
                supplier
                    .direct_const
                    .map(|declaration| declaration.statement_span)
            })
            .collect();
        return Ok(ScriptAnalysis::Compat {
            shape: match shape {
                LoadShape::CompatDefineBot => CompatShape::DefineBot,
                LoadShape::CompatClass => CompatShape::Class,
                _ => unreachable!("compat_shape only returns compat classifications"),
            },
            capabilities,
            erased_spans,
        });
    }
    let manifest_suppliers = graph.suppliers("manifest");
    let has_manifest_declaration = !manifest_suppliers.is_empty();
    if has_manifest_declaration {
        if let Some((name, span)) = first_conflict(&graph) {
            return Err(declaration_error(
                graph.entry(),
                span,
                "manifest",
                DeclarationReason::Manifest(ManifestReason::ConflictingExport(name)),
            )
            .into());
        }
        let direct = one_direct_const(manifest_suppliers, "manifest", graph.entry())?;
        let initializer = direct.initializer.ok_or_else(|| {
            declaration_error(
                graph.entry(),
                direct.statement_span.clone(),
                "manifest",
                DeclarationReason::Export(ExportReason::MissingInitializer),
            )
        })?;
        let parsed = parse_literal(
            &initializer,
            graph.entry().parsed.text(),
            graph.entry().source_offset(),
            path,
            source,
            "manifest",
        )?;
        let manifest = parse_manifest(&parsed, path, source)?;
        let Some(tick_span) = graph.direct_tick_function() else {
            return Err(declaration_error(
                graph.entry(),
                direct.statement_span,
                "tick",
                DeclarationReason::Manifest(ManifestReason::MissingTick),
            )
            .into());
        };
        let mut erased_spans = vec![direct.statement_span];
        graph.check_erased_references(&HashMap::from([(
            "manifest".to_owned(),
            direct
                .binding
                .expect("direct manifest has a simple binding"),
        )]))?;
        erased_spans.sort_by_key(|span| span.start);
        return Ok(ScriptAnalysis::Native {
            manifest,
            tick_span,
            erased_spans,
        });
    }

    reject_non_compat_host_capabilities(&graph)?;
    if graph.has_runtime_export("tick") || graph.has_runtime_export("apiVersion") {
        let span = graph
            .direct_tick_function()
            .or_else(|| {
                ["tick", "apiVersion"]
                    .into_iter()
                    .flat_map(|name| graph.suppliers(name))
                    .filter(|supplier| supplier.runtime)
                    .min_by_key(|supplier| supplier.span.start)
                    .map(|supplier| supplier.span)
            })
            .unwrap_or(0..0);
        return Err(declaration_error(
            graph.entry(),
            span,
            "manifest",
            DeclarationReason::Manifest(ManifestReason::MissingDeclaration),
        )
        .into());
    }
    Ok(ScriptAnalysis::NotBotShape)
}

impl ModuleNode {
    pub fn source_offset(&self) -> usize {
        self.source.len().saturating_sub(self.parsed.text().len())
    }
}

fn extract_host_capabilities(
    graph: &ModuleGraph,
) -> Result<DeclaredHostCapabilities, ExtractionError> {
    let suppliers = graph.suppliers("hostCapabilities");
    if suppliers.is_empty() {
        return Ok(DeclaredHostCapabilities::EMPTY);
    }
    let direct = one_direct_const(suppliers, "hostCapabilities", graph.entry())?;
    let initializer = direct.initializer.ok_or_else(|| {
        declaration_error(
            graph.entry(),
            direct.statement_span.clone(),
            "hostCapabilities",
            DeclarationReason::Export(ExportReason::MissingInitializer),
        )
    })?;
    let node = graph.entry();
    let value = parse_literal(
        &initializer,
        node.parsed.text(),
        node.source_offset(),
        &node.path,
        &node.source,
        "hostCapabilities",
    )?;
    let capabilities = parse_capabilities(&value, &node.path, &node.source, "hostCapabilities")?;
    graph.check_erased_references(&HashMap::from([(
        "hostCapabilities".to_owned(),
        direct
            .binding
            .expect("direct hostCapabilities has a simple binding"),
    )]))?;
    Ok(capabilities)
}

fn reject_non_compat_host_capabilities(graph: &ModuleGraph) -> Result<(), ExtractionError> {
    let suppliers = graph.suppliers("hostCapabilities");
    if suppliers.is_empty() {
        return Ok(());
    }
    let direct = one_direct_const(suppliers, "hostCapabilities", graph.entry())?;
    Err(declaration_error(
        graph.entry(),
        direct.statement_span,
        "hostCapabilities",
        DeclarationReason::Capability(CapabilityReason::WrongFamily),
    )
    .into())
}

fn parse_capabilities(
    value: &SpannedValue,
    path: &Path,
    source: &str,
    field_path: &str,
) -> Result<DeclaredHostCapabilities, ScriptDeclarationError> {
    let LiteralValue::Array(values) = &value.value else {
        return Err(manifest_error(
            path,
            source,
            value.span.clone(),
            field_path,
            ManifestReason::WrongType("array of strings"),
        ));
    };
    let mut seen = HashSet::with_capacity(values.len());
    let mut capabilities = DeclaredHostCapabilities::EMPTY;
    for item in values {
        let LiteralValue::String(name) = &item.value else {
            return Err(manifest_error(
                path,
                source,
                item.span.clone(),
                field_path,
                ManifestReason::WrongType("string"),
            ));
        };
        if !seen.insert(name.as_str()) {
            return Err(ScriptDeclarationError::new(
                path,
                source,
                item.span.clone(),
                field_path,
                DeclarationReason::Capability(CapabilityReason::Duplicate(name.clone())),
            ));
        }
        match name.as_str() {
            "host-packet-protocol" => {
                capabilities = [HostCapability::HostPacketProtocol].into_iter().collect()
            }
            _ => {
                return Err(ScriptDeclarationError::new(
                    path,
                    source,
                    item.span.clone(),
                    field_path,
                    DeclarationReason::Capability(CapabilityReason::Unknown(name.clone())),
                ));
            }
        }
    }
    Ok(capabilities)
}

fn first_conflict(graph: &ModuleGraph) -> Option<(String, Range<usize>)> {
    ["apiVersion", "SETTINGS", "hostCapabilities"]
        .into_iter()
        .flat_map(|name| {
            graph
                .suppliers(name)
                .into_iter()
                .map(|supplier| (name.to_owned(), supplier.span))
        })
        .min_by_key(|(_, span)| span.start)
}

fn one_direct_const(
    mut suppliers: Vec<ExportSupplier>,
    name: &str,
    node: &ModuleNode,
) -> Result<DirectConst, ExtractionError> {
    suppliers.sort_by_key(|supplier| supplier.span.start);
    let mut direct_count = 0;
    for supplier in &suppliers {
        if let Some(reason) = supplier.reason.clone() {
            return Err(declaration_error(
                node,
                supplier.span.clone(),
                name,
                DeclarationReason::Export(reason),
            )
            .into());
        }
        if supplier.direct_const.is_none() {
            return Err(declaration_error(
                node,
                supplier.span.clone(),
                name,
                DeclarationReason::Export(ExportReason::NotDirectConst),
            )
            .into());
        }
        direct_count += 1;
        if direct_count > 1 {
            return Err(declaration_error(
                node,
                supplier.span.clone(),
                name,
                DeclarationReason::Export(ExportReason::Duplicate),
            )
            .into());
        }
    }
    Ok(suppliers
        .remove(0)
        .direct_const
        .expect("a validated export supplier has a direct const"))
}

fn parse_manifest(
    value: &SpannedValue,
    path: &Path,
    source: &str,
) -> Result<ScriptManifest, ScriptDeclarationError> {
    const REQUIRED: &[&str] = &[
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
    let LiteralValue::Object(properties) = &value.value else {
        return Err(manifest_error(
            path,
            source,
            value.span.clone(),
            "manifest",
            ManifestReason::WrongType("object"),
        ));
    };
    let mut api_version = None;
    let mut text_fields: HashMap<String, String> = HashMap::new();
    let mut tags = None;
    let mut settings = None;
    let mut capabilities = None;
    for property in properties {
        let field_path = format!("manifest.{}", property.key);
        match property.key.as_str() {
            "apiVersion" => {
                let LiteralValue::Number(number) = property.value.value else {
                    return Err(manifest_error(
                        path,
                        source,
                        property.value.span.clone(),
                        &field_path,
                        ManifestReason::WrongType("integer numeric literal"),
                    ));
                };
                if number.fract() != 0.0 || number < 0.0 || number > u32::MAX as f64 {
                    return Err(manifest_error(
                        path,
                        source,
                        property.value.span.clone(),
                        &field_path,
                        ManifestReason::MalformedApiVersion,
                    ));
                }
                let version = number as u32;
                if version != 2 {
                    return Err(manifest_error(
                        path,
                        source,
                        property.value.span.clone(),
                        &field_path,
                        ManifestReason::UnsupportedApiVersion(version),
                    ));
                }
                api_version = Some(version);
            }
            "name" | "version" | "author" | "description" | "category" => {
                text_fields.insert(
                    property.key.clone(),
                    required_text(&property.value, path, source, &field_path)?,
                );
            }
            "tags" => tags = Some(parse_tags(&property.value, path, source, &field_path)?),
            "settings" => {
                settings = Some(schema::project_settings(
                    &property.value,
                    source,
                    path,
                    &field_path,
                )?);
            }
            "capabilities" => {
                capabilities = Some(parse_capabilities(
                    &property.value,
                    path,
                    source,
                    &field_path,
                )?);
            }
            _ => {
                return Err(manifest_error(
                    path,
                    source,
                    property.key_span.clone(),
                    &field_path,
                    ManifestReason::UnknownField(property.key.clone()),
                ));
            }
        }
    }
    for field in REQUIRED {
        let present = match *field {
            "apiVersion" => api_version.is_some(),
            "name" | "version" | "author" | "description" | "category" => {
                text_fields.contains_key(*field)
            }
            "tags" => tags.is_some(),
            "settings" => settings.is_some(),
            "capabilities" => capabilities.is_some(),
            _ => unreachable!(),
        };
        if !present {
            return Err(manifest_error(
                path,
                source,
                value.span.clone(),
                &format!("manifest.{field}"),
                ManifestReason::MissingField(field),
            ));
        }
    }
    let take_text = |field: &'static str, values: &mut HashMap<String, String>| {
        values
            .remove(field)
            .expect("required string field was validated")
    };
    Ok(ScriptManifest::new(
        api_version.expect("required apiVersion was validated"),
        ManifestText {
            name: take_text("name", &mut text_fields),
            version: take_text("version", &mut text_fields),
            author: take_text("author", &mut text_fields),
            description: take_text("description", &mut text_fields),
            category: take_text("category", &mut text_fields),
        },
        tags.expect("required tags were validated"),
        settings.expect("required settings were validated"),
        capabilities.expect("required capabilities were validated"),
    ))
}

fn required_text(
    value: &SpannedValue,
    path: &Path,
    source: &str,
    field_path: &str,
) -> Result<String, ScriptDeclarationError> {
    let LiteralValue::String(text) = &value.value else {
        return Err(manifest_error(
            path,
            source,
            value.span.clone(),
            field_path,
            ManifestReason::WrongType("string"),
        ));
    };
    schema::validate_display_text(text, source, path, field_path, &value.span)?;
    if text.is_empty() {
        return Err(manifest_error(
            path,
            source,
            value.span.clone(),
            field_path,
            ManifestReason::Empty,
        ));
    }
    if text.trim() != text {
        return Err(manifest_error(
            path,
            source,
            value.span.clone(),
            field_path,
            ManifestReason::Whitespace,
        ));
    }
    Ok(text.clone())
}

fn parse_tags(
    value: &SpannedValue,
    path: &Path,
    source: &str,
    field_path: &str,
) -> Result<Vec<String>, ScriptDeclarationError> {
    let LiteralValue::Array(values) = &value.value else {
        return Err(manifest_error(
            path,
            source,
            value.span.clone(),
            field_path,
            ManifestReason::WrongType("array of strings"),
        ));
    };
    let mut tags = Vec::with_capacity(values.len());
    let mut seen = HashSet::with_capacity(values.len());
    for item in values {
        let LiteralValue::String(tag) = &item.value else {
            return Err(manifest_error(
                path,
                source,
                item.span.clone(),
                field_path,
                ManifestReason::WrongType("string"),
            ));
        };
        schema::validate_display_text(tag, source, path, field_path, &item.span)?;
        if tag.is_empty() {
            return Err(manifest_error(
                path,
                source,
                item.span.clone(),
                field_path,
                ManifestReason::Empty,
            ));
        }
        if !seen.insert(tag.as_str()) {
            return Err(manifest_error(
                path,
                source,
                item.span.clone(),
                field_path,
                ManifestReason::DuplicateTag(tag.clone()),
            ));
        }
        tags.push(tag.clone());
    }
    Ok(tags)
}

fn declaration_error(
    node: &ModuleNode,
    span: Range<usize>,
    field_path: &str,
    reason: DeclarationReason,
) -> ScriptDeclarationError {
    ScriptDeclarationError::new(&node.path, &node.source, span, field_path, reason)
}

fn manifest_error(
    path: &Path,
    source: &str,
    span: Range<usize>,
    field_path: &str,
    reason: ManifestReason,
) -> ScriptDeclarationError {
    ScriptDeclarationError::new(
        path,
        source,
        span,
        field_path,
        DeclarationReason::Manifest(reason),
    )
}

fn parse_source(path: &Path, source: &str) -> Result<(ParsedSource, usize), ExtractionError> {
    let (parser_text, source_offset) = strip_bom(source);
    let media_type = match path.extension().and_then(|extension| extension.to_str()) {
        Some("js" | "mjs" | "cjs") => MediaType::JavaScript,
        Some("ts") => MediaType::TypeScript,
        _ => {
            return Err(ScriptDeclarationError::new(
                path,
                source,
                0..0,
                "source",
                DeclarationReason::Syntax,
            )
            .into());
        }
    };
    let absolute = path.canonicalize().unwrap_or_else(|_| {
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            std::env::current_dir()
                .map(|cwd| cwd.join(path))
                .unwrap_or_else(|_| path.to_path_buf())
        }
    });
    let specifier = deno_ast::ModuleSpecifier::from_file_path(&absolute).unwrap_or_else(|_| {
        deno_ast::ModuleSpecifier::parse("file:///manifest.js")
            .expect("fixed manifest URL is valid")
    });
    let parsed = deno_ast::parse_module(ParseParams {
        specifier,
        text: parser_text.to_owned().into(),
        media_type,
        capture_tokens: true,
        scope_analysis: true,
        maybe_syntax: None,
    })
    .map_err(|error| {
        let parser_start = error
            .range
            .start
            .as_byte_index(StartSourcePos::START_SOURCE_POS);
        let parser_end = error
            .range
            .end
            .as_byte_index(StartSourcePos::START_SOURCE_POS);
        ScriptDeclarationError::new(
            path,
            source,
            parser_start.saturating_add(source_offset)..parser_end.saturating_add(source_offset),
            "source",
            DeclarationReason::Syntax,
        )
    })?;
    if let Some(diagnostic) = parsed.diagnostics().first() {
        let start = diagnostic
            .range
            .start
            .as_byte_index(StartSourcePos::START_SOURCE_POS);
        let end = diagnostic
            .range
            .end
            .as_byte_index(StartSourcePos::START_SOURCE_POS);
        return Err(ScriptDeclarationError::new(
            path,
            source,
            start.saturating_add(source_offset)..end.saturating_add(source_offset),
            "source",
            DeclarationReason::Syntax,
        )
        .into());
    }
    Ok((parsed, source_offset))
}

fn source_range(span: Span, parsed_text: &str, source_offset: usize) -> Range<usize> {
    let parser_start = StartSourcePos::START_SOURCE_POS.as_byte_pos().0 as usize;
    let limit = parsed_text.len().saturating_add(source_offset);
    let start = (span.lo.0 as usize)
        .saturating_sub(parser_start)
        .saturating_add(source_offset)
        .min(limit);
    let end = (span.hi.0 as usize)
        .saturating_sub(parser_start)
        .saturating_add(source_offset)
        .min(limit);
    start..end.max(start)
}

#[cfg(test)]
fn erase_declarations(source: &str, spans: &[Range<usize>]) -> String {
    let mut bytes = source.as_bytes().to_vec();
    for span in spans {
        let start = span.start.min(bytes.len());
        let end = span.end.min(bytes.len()).max(start);
        if start == end {
            continue;
        }
        bytes[start] = b';';
        let mut cursor = start + 1;
        while cursor < end {
            match bytes[cursor] {
                b'\r' | b'\n' => cursor += 1,
                0xe2 if cursor + 2 < end
                    && bytes[cursor + 1] == 0x80
                    && matches!(bytes[cursor + 2], 0xa8 | 0xa9) =>
                {
                    cursor += 3;
                }
                _ => {
                    bytes[cursor] = b' ';
                    cursor += 1;
                }
            }
        }
    }
    String::from_utf8(bytes).expect("replacing source bytes preserves UTF-8")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn good_manifest() -> &'static str {
        "{ apiVersion: 2, name: 'Miner', version: '1', author: 'Alice', description: 'Mine ore', category: 'Mining', tags: [], settings: {}, capabilities: [] }"
    }

    fn source(manifest: &str) -> String {
        format!("export const manifest = {manifest};\nexport function tick() {{}}\n")
    }

    static NEXT_TEMP_TREE: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

    struct TempTree(std::path::PathBuf);

    impl TempTree {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "script-js-manifest-{}-{}",
                std::process::id(),
                NEXT_TEMP_TREE.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            ));
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempTree {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.0).unwrap();
        }
    }

    fn declaration_error(input: &str) -> ScriptDeclarationError {
        match analyze(Path::new("manifest.ts"), input).unwrap_err() {
            ExtractionError::Declaration(error) => error,
            ExtractionError::UnresolvedModule(error) => panic!("unexpected graph error: {error}"),
        }
    }

    fn assert_error_code(input: &str, code: &str) -> ScriptDeclarationError {
        let error = declaration_error(input);
        assert_eq!(error.code(), code, "{error}");
        error
    }

    fn manifest_source_replacing(from: &str, to: &str) -> String {
        source(&good_manifest().replace(from, to))
    }

    fn assert_expression_kind(input: &str, expected: ExpressionKind) -> ScriptDeclarationError {
        let error = assert_error_code(input, "script-declaration-nonliteral");
        assert!(
            matches!(
                &error.reason,
                DeclarationReason::Literal(LiteralReason::Expression(actual))
                    if *actual == expected
            ),
            "{error:?}"
        );
        assert_eq!(
            error.reason_text(),
            expected_expression_reason(expected),
            "{error}"
        );
        error
    }

    fn expected_expression_reason(kind: ExpressionKind) -> &'static str {
        match kind {
            ExpressionKind::Identifier => "expected a literal value, found identifier",
            ExpressionKind::PropertyAccess => "expected a literal value, found property access",
            ExpressionKind::Call => "expected a literal value, found call",
            ExpressionKind::NewExpression => "expected a literal value, found new expression",
            ExpressionKind::Function => "expected a literal value, found function",
            ExpressionKind::ArrowFunction => "expected a literal value, found arrow function",
            ExpressionKind::Class => "expected a literal value, found class",
            ExpressionKind::Spread => "expected a literal value, found spread",
            ExpressionKind::ShorthandProperty => {
                "expected a literal value, found shorthand property"
            }
            ExpressionKind::Getter => "expected a literal value, found getter",
            ExpressionKind::Setter => "expected a literal value, found setter",
            ExpressionKind::Method => "expected a literal value, found method",
            ExpressionKind::Template => "expected a literal value, found template",
            ExpressionKind::Null => "expected a literal value, found null",
            ExpressionKind::Undefined => "expected a literal value, found undefined",
            ExpressionKind::NaN => "expected a literal value, found NaN",
            ExpressionKind::Infinity => "expected a literal value, found Infinity",
            ExpressionKind::BigInt => "expected a literal value, found BigInt",
            ExpressionKind::UnaryExpression => "expected a literal value, found unary expression",
            ExpressionKind::BinaryExpression => "expected a literal value, found binary expression",
            ExpressionKind::ConditionalExpression => {
                "expected a literal value, found conditional expression"
            }
            ExpressionKind::Assignment => "expected a literal value, found assignment",
            ExpressionKind::Sequence => "expected a literal value, found sequence",
            ExpressionKind::Await => "expected a literal value, found await",
            ExpressionKind::Yield => "expected a literal value, found yield",
            ExpressionKind::Update => "expected a literal value, found update",
            ExpressionKind::This => "expected a literal value, found this",
            ExpressionKind::Super => "expected a literal value, found super",
            ExpressionKind::Import => "expected a literal value, found import",
            ExpressionKind::RegularExpression => {
                "expected a literal value, found regular expression"
            }
            ExpressionKind::Jsx => "expected a literal value, found JSX",
            ExpressionKind::OtherExpression => "expected a literal value, found other expression",
        }
    }

    #[test]
    fn extraction_is_static_and_accepts_a_valid_native_manifest_without_v8() {
        let result = analyze(Path::new("manifest.ts"), &source(good_manifest())).unwrap();
        let ScriptAnalysis::Native { manifest, .. } = result else {
            panic!("expected native manifest result");
        };
        assert_eq!(manifest.name(), "Miner");
        assert_eq!(manifest.api_version(), 2);
    }

    #[test]
    fn javascript_media_accepts_javascript_and_rejects_typescript_syntax() {
        let path = Path::new("manifest.js");
        assert!(matches!(
            analyze(path, "export function tick() {}"),
            Err(ExtractionError::Declaration(error)) if error.code() == "manifest-required"
        ));
        let native = source(good_manifest());
        assert!(matches!(
            analyze(path, &native),
            Ok(ScriptAnalysis::Native { .. })
        ));
        let typescript_in_javascript = analyze(path, "const value: number = 1;");
        assert!(
            matches!(
                &typescript_in_javascript,
                Err(ExtractionError::Declaration(error))
                    if error.code() == "script-source-syntax"
            ),
            "JavaScript media must reject TypeScript syntax: {typescript_in_javascript:?}"
        );

        let compat = "\u{feff}export default class Miner extends TaskBot {};\n";
        assert!(matches!(
            analyze(Path::new("compat.js"), compat),
            Ok(ScriptAnalysis::Compat {
                shape: CompatShape::Class,
                erased_spans,
                ..
            }) if erased_spans.is_empty()
        ));
        assert_eq!(erase_declarations(compat, &[]), compat);
    }

    #[test]
    fn bom_source_parses_and_erases_from_the_export_token() {
        let source = format!("\u{feff}{}", source(good_manifest()));
        let ScriptAnalysis::Native { erased_spans, .. } =
            analyze(Path::new("manifest.ts"), &source).unwrap()
        else {
            panic!("expected native manifest result");
        };
        assert_eq!(source.as_bytes()[erased_spans[0].start], b'e');
        let erased = erase_declarations(&source, &erased_spans);
        assert_eq!(erased.as_bytes()[0..3], [0xef, 0xbb, 0xbf]);
        assert_eq!(erased.as_bytes()[3], b';');
        assert!(!erased.contains("manifest"));
    }
    #[test]
    fn defaults_gate_compat_markers_and_keep_named_manifest_code() {
        let input =
            "export const manifest = defineBot({ name: 'compat' }); export default manifest;";
        let ScriptAnalysis::Compat {
            shape,
            erased_spans,
            ..
        } = analyze(Path::new("compat.ts"), input).unwrap()
        else {
            panic!("default compat export should keep legacy classification");
        };
        assert_eq!(shape, CompatShape::DefineBot);
        assert!(erased_spans.is_empty());
        assert_eq!(erase_declarations(input, &erased_spans), input);

        let unrecognized_default = "export default 1; export const manifest = 2;";
        assert!(matches!(
            analyze(Path::new("plain.ts"), unrecognized_default),
            Ok(ScriptAnalysis::NotBotShape)
        ));
        let no_default_marker = "const legacy = defineBot({});";
        assert!(matches!(
            analyze(Path::new("plain.ts"), no_default_marker),
            Ok(ScriptAnalysis::NotBotShape)
        ));
        let native_marker_call = format!(
            "function helper() {{ defineBot({{}}); }}\n{}",
            source(good_manifest())
        );
        assert!(matches!(
            analyze(Path::new("manifest.ts"), &native_marker_call),
            Ok(ScriptAnalysis::Native { .. })
        ));
        for marker_source in [
            "export default 1; // defineBot({})",
            "export default 1; const marker = 'defineBot(';",
            "export default 1; const marker = '__rs2b0tManifest';",
        ] {
            assert!(matches!(
                analyze(Path::new("plain.ts"), marker_source),
                Ok(ScriptAnalysis::NotBotShape)
            ));
        }
        for marker_source in [
            "const __rs2b0tManifest = {}; export default 1;",
            "export default { '__rs2b0tManifest': true };",
        ] {
            assert!(matches!(
                analyze(Path::new("compat.ts"), marker_source),
                Ok(ScriptAnalysis::Compat {
                    shape: CompatShape::DefineBot,
                    ..
                })
            ));
        }
    }

    #[test]
    fn type_only_default_does_not_select_compat() {
        for declaration in [
            "export interface Manifest {}",
            "export default interface Shape {}",
        ] {
            let input = format!("{declaration}\n{}", source(good_manifest()));
            assert!(matches!(
                analyze(Path::new("manifest.ts"), &input),
                Ok(ScriptAnalysis::Native { .. })
            ));
        }
    }
    #[test]
    fn missing_manifest_and_default_discrimination_follow_runtime_exports() {
        for path in ["missing.ts", "missing.js"] {
            for input in [
                "export function tick() {}",
                "export async function tick() {}",
                "export const apiVersion = 2; export function tick() {}",
            ] {
                assert!(matches!(
                    analyze(Path::new(path), input),
                    Err(ExtractionError::Declaration(error))
                        if error.code() == "manifest-required"
                ));
            }
        }
        for lookalike in [
            "const note = 'export function tick()';",
            "// export const apiVersion = 2;\nconst value = 1;",
        ] {
            assert!(matches!(
                analyze(Path::new("missing.ts"), lookalike),
                Ok(ScriptAnalysis::NotBotShape)
            ));
        }
        let path = Path::new("cached.ts");
        assert!(matches!(
            analyze(path, "export function tick() {}"),
            Err(ExtractionError::Declaration(error)) if error.code() == "manifest-required"
        ));
        assert!(matches!(
            analyze(path, &source(good_manifest())),
            Ok(ScriptAnalysis::Native { .. })
        ));
        let top_level_throw = format!(
            "throw new Error('must not execute'); {}",
            source(good_manifest())
        );
        assert!(matches!(
            analyze(Path::new("throwing.ts"), &top_level_throw),
            Ok(ScriptAnalysis::Native { .. })
        ));
        let default_with_native_names =
            "export default 1; export function tick() {} export const manifest = {};";
        assert!(matches!(
            analyze(Path::new("default.ts"), default_with_native_names),
            Ok(ScriptAnalysis::NotBotShape)
        ));
        let native_default = format!("{} export default manifest;", source(good_manifest()));
        assert!(matches!(
            analyze(Path::new("default-manifest.ts"), &native_default),
            Ok(ScriptAnalysis::NotBotShape)
        ));
        let compat_marker_without_default = "const bot = defineBot({}); export function tick() {}";
        assert_eq!(
            assert_error_code(compat_marker_without_default, "manifest-required").field_path,
            "manifest"
        );
        let host_capability = "export const hostCapabilities = notAStaticLiteral;";
        let wrong_family = analyze(Path::new("plain.ts"), host_capability).unwrap_err();
        assert!(matches!(
            wrong_family,
            ExtractionError::Declaration(error)
                if error.code() == "host-capabilities-api-conflict"
        ));
        assert!(matches!(
            analyze(
                Path::new("plain.ts"),
                "export default 1; export const hostCapabilities = [];"
            ),
            Err(ExtractionError::Declaration(error))
                if error.code() == "host-capabilities-api-conflict"
        ));
        assert!(matches!(
            analyze(
                Path::new("compat.ts"),
                "export default class Miner extends TaskBot {}"
            ),
            Ok(ScriptAnalysis::Compat {
                shape: CompatShape::Class,
                ..
            })
        ));
    }

    #[test]
    fn type_only_local_and_reexported_defaults_do_not_select_compat() {
        for input in [
            format!(
                "interface TypeOnly {{}}\nexport {{ TypeOnly as default }};\n{}",
                source(good_manifest())
            ),
            format!(
                "export type {{ TypeOnly as default }} from './missing.ts';\n{}",
                source(good_manifest())
            ),
            format!(
                "export {{ type TypeOnly as default }} from './missing.ts';\n{}",
                source(good_manifest())
            ),
        ] {
            assert!(matches!(
                analyze(Path::new("manifest.ts"), &input),
                Ok(ScriptAnalysis::Native { .. })
            ));
        }
        assert!(matches!(
            analyze(
                Path::new("default.ts"),
                "const value = 1; export { value as default }; export function tick() {}"
            ),
            Ok(ScriptAnalysis::NotBotShape)
        ));
    }

    #[test]
    fn manifest_conflicts_with_legacy_exports_in_both_orders() {
        for (declaration, name) in [
            ("export const hostCapabilities = [];", "hostCapabilities"),
            ("export const apiVersion = 2;", "apiVersion"),
            ("export const SETTINGS = {};", "SETTINGS"),
        ] {
            for input in [
                format!("{declaration} {}", source(good_manifest())),
                format!("{} {declaration}", source(good_manifest())),
            ] {
                let error = analyze(Path::new("manifest.ts"), &input).unwrap_err();
                let ExtractionError::Declaration(error) = error else {
                    panic!("wrong error type");
                };
                assert_eq!(error.code(), "manifest-conflicting-export");
                assert_eq!(
                    error.reason_text(),
                    format!("manifest must not be combined with export \"{name}\"")
                );
                assert!(matches!(
                    &error.reason,
                    DeclarationReason::Manifest(ManifestReason::ConflictingExport(conflict))
                        if conflict == name
                ));
            }
        }
    }

    #[test]
    fn direct_tick_function_is_required_and_only_runtime_exports_trigger_migration() {
        let no_tick = format!(
            "export const manifest = {}; export const tick = () => {{}};",
            good_manifest()
        );
        assert!(
            matches!(analyze(Path::new("manifest.ts"), &no_tick), Err(ExtractionError::Declaration(error)) if error.code() == "api-version-missing-tick")
        );
        let type_tick = format!(
            "export type {{ tick }} from './types.ts'; export const manifest = {};",
            good_manifest()
        );
        assert!(
            matches!(analyze(Path::new("manifest.ts"), &type_tick), Err(ExtractionError::Declaration(error)) if error.code() == "api-version-missing-tick")
        );
    }

    #[test]
    fn export_declaration_spans_include_statement_comments_and_semicolons() {
        let input = format!(
            "export const manifest = {} /* inline */;\nexport function tick() {{}}",
            good_manifest()
        );
        let ScriptAnalysis::Native { erased_spans, .. } =
            analyze(Path::new("manifest.ts"), &input).unwrap()
        else {
            panic!("expected native")
        };
        assert_eq!(
            &input[erased_spans[0].clone()],
            format!("export const manifest = {} /* inline */;", good_manifest())
        );
    }

    #[test]
    fn accepted_literal_wrappers_work_at_root_and_nested_depth() {
        let manifest = good_manifest();
        let wrappers = [
            format!("(({manifest}))"),
            format!("({manifest} as ScriptManifest)"),
            format!("(<ScriptManifest>{manifest})"),
            format!("({manifest} as const)"),
            format!("({manifest} satisfies ScriptManifest)"),
        ];
        for wrapped in wrappers {
            assert!(
                matches!(
                    analyze(Path::new("manifest.ts"), &source(&wrapped)),
                    Ok(ScriptAnalysis::Native { .. })
                ),
                "{wrapped}"
            );
        }

        let nested = manifest_source_replacing(
            "tags: []",
            "tags: [('ore' as string), <string>'gem', 'ruby' satisfies string, ('steel' as const)]",
        );
        assert!(matches!(
            analyze(Path::new("manifest.ts"), &nested),
            Ok(ScriptAnalysis::Native { .. })
        ));
        for manifest in [
            good_manifest().replace("tags: []", "tags: ['ore'!]"),
            good_manifest().replace(
                "settings: {}",
                "settings: { text: { type: 'string', default: ('ore'!) } }",
            ),
        ] {
            let error = assert_error_code(&source(&manifest), "script-declaration-wrapper");
            assert!(matches!(
                error.reason,
                DeclarationReason::Literal(LiteralReason::UnsupportedWrapper(
                    UnsupportedWrapper::TsNonNull
                ))
            ));
            assert_eq!(error.reason_text(), "non-null assertion is not allowed");
        }
        let root_non_null = source(&format!("({manifest})!"));
        let error = assert_error_code(&root_non_null, "script-declaration-wrapper");
        assert!(matches!(
            error.reason,
            DeclarationReason::Literal(LiteralReason::UnsupportedWrapper(
                UnsupportedWrapper::TsNonNull
            ))
        ));
        assert_eq!(error.reason_text(), "non-null assertion is not allowed");
    }
    #[test]
    fn reserved_exports_require_one_direct_simple_const() {
        for input in [
            "export let manifest = {}; export function tick() {}",
            "export const { manifest } = {}; export function tick() {}",
            "export const manifest = {}, other = 1; export function tick() {}",
            "export interface manifest {} export function tick() {}",
            "export type manifest = {}; export function tick() {}",
            "export enum manifest { A } export function tick() {}",
            "export namespace manifest {} export function tick() {}",
        ] {
            assert!(
                matches!(analyze(Path::new("manifest.ts"), input), Err(ExtractionError::Declaration(error)) if error.code() == "script-declaration-form")
            );
        }
    }

    #[test]
    fn standalone_tick_or_api_version_requires_manifest() {
        for input in [
            "export function tick() {}",
            "export const apiVersion = 2; export function tick() {}",
        ] {
            assert!(
                matches!(analyze(Path::new("manifest.ts"), input), Err(ExtractionError::Declaration(error)) if error.code() == "manifest-required")
            );
        }
    }

    #[test]
    fn missing_manifest_message_contains_the_migration_example() {
        let error = declaration_error("export function tick() {}");
        assert_eq!(error.code(), "manifest-required");
        assert!(error.to_string().contains(
            "export const manifest = { apiVersion: 2, name: 'My script', version: '1', author: 'Your name', description: 'What it does', category: 'Examples', tags: [], settings: {}, capabilities: [] };"
        ));
    }

    #[test]
    fn missing_manifest_prefers_tick_span_then_first_entry_export() {
        let tick_source = "export const apiVersion = 2; export function tick() {}";
        let tick_error = declaration_error(tick_source);
        assert_eq!(tick_error.code(), "manifest-required");
        assert_eq!(
            tick_error.span.start,
            tick_source.find("export function").unwrap()
        );

        let first_export_source = "export const apiVersion = 2; export const tick = () => {};";
        let first_export_error = declaration_error(first_export_source);
        assert_eq!(first_export_error.code(), "manifest-required");
        assert_eq!(
            first_export_error.span.start,
            first_export_source.find("export const apiVersion").unwrap()
        );
    }

    #[test]
    fn numeric_setting_ranges_compare_numbers_and_refuse_inversion() {
        for (min, max, default, expected) in [
            ("9", "10", "9", None),
            ("10", "9", "10", Some("setting-range-inverted")),
            ("-1.2", "-1.1", "-1.2", None),
            ("-1.1", "-1.2", "-1.1", Some("setting-range-inverted")),
        ] {
            let settings = format!(
                "settings: {{ amount: {{ type: 'number', default: {default}, min: {min}, max: {max} }} }}"
            );
            let input = manifest_source_replacing("settings: {}", &settings);
            match expected {
                None => assert!(
                    matches!(
                        analyze(Path::new("manifest.ts"), &input),
                        Ok(ScriptAnalysis::Native { .. })
                    ),
                    "valid range {min}..={max} was refused"
                ),
                Some(code) => {
                    let error = assert_error_code(&input, code);
                    assert_eq!(error.reason_text(), "min must not exceed max");
                }
            }
        }
    }

    #[test]
    fn erasure_preserves_all_supported_line_terminators_and_utf8() {
        for line in ["\n", "\r\n", "\r", "\u{2028}", "\u{2029}"] {
            let input = format!(
                "export const manifest = {};{}export function tick() {{}}",
                good_manifest(),
                line
            );
            let ScriptAnalysis::Native { erased_spans, .. } =
                analyze(Path::new("manifest.ts"), &input).unwrap()
            else {
                panic!("expected native")
            };
            let erased = erase_declarations(&input, &erased_spans);
            assert!(erased.starts_with(';'));
            assert!(erased.contains(line));
            assert!(erased.ends_with("export function tick() {}"));
        }
    }

    #[test]
    fn literal_keys_and_properties_return_exact_typed_reasons() {
        for (key, expected) in [("1", KeyKind::Numeric), ("1n", KeyKind::BigInt)] {
            let manifest = good_manifest()
                .replace("name: 'Miner',", &format!("name: 'Miner', {key}: 'other',"));
            let error = assert_error_code(&source(&manifest), "script-declaration-key-kind");
            assert!(matches!(
                error.reason,
                DeclarationReason::Literal(LiteralReason::KeyKind(actual))
                    if actual == expected
            ));
        }
        let computed =
            manifest_source_replacing("name: 'Miner',", "name: 'Miner', [name]: 'other',");
        let error = assert_error_code(&computed, "script-declaration-computed-key");
        assert!(matches!(
            error.reason,
            DeclarationReason::Literal(LiteralReason::ComputedKey)
        ));

        let prototype =
            manifest_source_replacing("name: 'Miner',", "name: 'Miner', __proto__: {},");
        let error = assert_error_code(&prototype, "script-declaration-prototype-key");
        assert!(matches!(
            error.reason,
            DeclarationReason::Literal(LiteralReason::PrototypeKey)
        ));

        for duplicate in [
            "name: 'Miner', 'name': 'Other',",
            "name: 'Miner', 'na\\u006de': 'Other',",
            "name: 'Miner', na\\u006de: 'Other',",
        ] {
            let input = manifest_source_replacing("name: 'Miner',", duplicate);
            let error = assert_error_code(&input, "script-declaration-duplicate-key");
            assert!(matches!(
                &error.reason,
                DeclarationReason::Literal(LiteralReason::DuplicateKey(key))
                    if key == "name"
            ));
        }

        let non_finite = manifest_source_replacing("apiVersion: 2", "apiVersion: 1e400");
        let error = assert_error_code(&non_finite, "script-declaration-number-nonfinite");
        assert!(matches!(
            error.reason,
            DeclarationReason::Literal(LiteralReason::NonFinite)
        ));

        let sparse = manifest_source_replacing("tags: []", "tags: [,]");
        let error = assert_error_code(&sparse, "script-declaration-sparse-array");
        assert!(matches!(
            error.reason,
            DeclarationReason::Literal(LiteralReason::SparseArray)
        ));

        let spread = manifest_source_replacing("tags: []", "tags: [...[]]");
        let error = assert_expression_kind(&spread, ExpressionKind::Spread);
        assert!(error.field_path.starts_with("manifest.tags"));

        for (member, kind) in [
            ("name", ExpressionKind::ShorthandProperty),
            ("get name() { return 'Miner'; }", ExpressionKind::Getter),
            ("set name(value) {}", ExpressionKind::Setter),
            ("name() {}", ExpressionKind::Method),
        ] {
            let input = manifest_source_replacing("name: 'Miner',", &format!("{member},"));
            assert_expression_kind(&input, kind);
        }
        let object_spread = manifest_source_replacing("settings: {}", "settings: { ...{} }");
        assert_expression_kind(&object_spread, ExpressionKind::Spread);
    }

    #[test]
    fn invalid_literal_expression_kinds_are_stable_and_specific() {
        for (expression, expected) in [
            ("value", ExpressionKind::Identifier),
            ("value.name", ExpressionKind::PropertyAccess),
            ("call()", ExpressionKind::Call),
            ("import('./other.ts')", ExpressionKind::Import),
            ("new Date()", ExpressionKind::NewExpression),
            ("function () {}", ExpressionKind::Function),
            ("() => 1", ExpressionKind::ArrowFunction),
            ("class {}", ExpressionKind::Class),
            ("`text`", ExpressionKind::Template),
            ("null", ExpressionKind::Null),
            ("undefined", ExpressionKind::Undefined),
            ("void 0", ExpressionKind::Undefined),
            ("NaN", ExpressionKind::NaN),
            ("Infinity", ExpressionKind::Infinity),
            ("1n", ExpressionKind::BigInt),
            ("!true", ExpressionKind::UnaryExpression),
            ("1 + 2", ExpressionKind::BinaryExpression),
            ("true ? 1 : 2", ExpressionKind::ConditionalExpression),
            ("value = 1", ExpressionKind::Assignment),
            ("(1, 2)", ExpressionKind::Sequence),
            ("await 1", ExpressionKind::Await),
            ("++value", ExpressionKind::Update),
            ("this", ExpressionKind::This),
            ("import.meta", ExpressionKind::Import),
            ("/value/", ExpressionKind::RegularExpression),
        ] {
            let input = manifest_source_replacing("name: 'Miner'", &format!("name: {expression}"));
            assert_expression_kind(&input, expected);
        }
    }

    #[test]
    fn literal_reason_texts_are_exact() {
        for (from, to, code, reason) in [
            (
                "name: 'Miner',",
                "name: 'Miner', 1: 'other',",
                "script-declaration-key-kind",
                "numeric property names are not allowed",
            ),
            (
                "name: 'Miner',",
                "name: 'Miner', 1n: 'other',",
                "script-declaration-key-kind",
                "BigInt property names are not allowed",
            ),
            (
                "name: 'Miner',",
                "name: 'Miner', [name]: 'other',",
                "script-declaration-computed-key",
                "computed property names are not allowed",
            ),
            (
                "name: 'Miner',",
                "name: 'Miner', __proto__: {},",
                "script-declaration-prototype-key",
                "__proto__ property is not allowed",
            ),
            (
                "name: 'Miner',",
                "name: 'Miner', 'name': 'Other',",
                "script-declaration-duplicate-key",
                "duplicate property \"name\"",
            ),
            (
                "tags: []",
                "tags: [,]",
                "script-declaration-sparse-array",
                "array holes are not allowed",
            ),
            (
                "apiVersion: 2",
                "apiVersion: 1e400",
                "script-declaration-number-nonfinite",
                "number must be finite",
            ),
            (
                "name: 'Miner'",
                "name: '\\uD800'",
                "script-declaration-lone-surrogate",
                "unpaired surrogate escape is not allowed",
            ),
            (
                "name: 'Miner'",
                "name: 'Miner'!",
                "script-declaration-wrapper",
                "non-null assertion is not allowed",
            ),
        ] {
            let error = assert_error_code(&manifest_source_replacing(from, to), code);
            assert_eq!(error.reason_text(), reason);
        }
    }

    #[test]
    fn unpaired_surrogate_escapes_are_refused_in_all_manifest_string_positions() {
        for escape in [r"\uD800", r"\uDC00"] {
            let variants = [
                good_manifest().replace("name: 'Miner'", &format!("name: '{escape}'")),
                good_manifest().replace("version: '1'", &format!("version: '{escape}'")),
                good_manifest().replace("author: 'Alice'", &format!("author: '{escape}'")),
                good_manifest().replace(
                    "description: 'Mine ore'",
                    &format!("description: '{escape}'"),
                ),
                good_manifest().replace("category: 'Mining'", &format!("category: '{escape}'")),
                good_manifest().replace(
                    "name: 'Miner',",
                    &format!("'{escape}': 'other', name: 'Miner',"),
                ),
                good_manifest().replace("tags: []", &format!("tags: ['{escape}']")),
                good_manifest().replace(
                    "capabilities: []",
                    &format!("capabilities: ['{escape}']"),
                ),
                good_manifest().replace(
                    "settings: {}",
                    &format!("settings: {{ '{escape}': {{ type: 'string', default: '' }} }}"),
                ),
                good_manifest().replace(
                    "settings: {}",
                    &format!("settings: {{ text: {{ type: '{escape}', default: '' }} }}"),
                ),
                good_manifest().replace(
                    "settings: {}",
                    &format!("settings: {{ text: {{ type: 'string', default: '{escape}' }} }}"),
                ),
                good_manifest().replace(
                    "settings: {}",
                    &format!("settings: {{ text: {{ type: 'string', default: '', label: '{escape}' }} }}"),
                ),
                good_manifest().replace(
                    "settings: {}",
                    &format!("settings: {{ text: {{ type: 'string', default: '', help: '{escape}' }} }}"),
                ),
                good_manifest().replace(
                    "settings: {}",
                    &format!("settings: {{ text: {{ type: 'string', default: '', group: '{escape}' }} }}"),
                ),
                good_manifest().replace(
                    "settings: {}",
                    &format!("settings: {{ text: {{ type: 'string', default: '', options: ['{escape}'] }} }}"),
                ),
                good_manifest().replace(
                    "settings: {}",
                    &format!("settings: {{ text: {{ type: 'string', default: '', optionsFrom: '{escape}' }} }}"),
                ),
                good_manifest().replace(
                    "settings: {}",
                    &format!("settings: {{ text: {{ type: 'string', default: '', optionLabels: {{ '{escape}': 'label' }} }} }}"),
                ),
                good_manifest().replace("name: 'Miner'", &format!("name: \"{escape}\"")),
                good_manifest().replace(
                    "settings: {}",
                    &format!("settings: {{ text: {{ type: 'string', default: '', optionLabels: {{ option: '{escape}' }} }} }}"),
                ),
                good_manifest().replace(
                    "settings: {}",
                    &format!("settings: {{ text: {{ type: 'string', default: '', showIf: {{ key: '{escape}', anyOf: ['x'] }} }} }}"),
                ),
                good_manifest().replace(
                    "settings: {}",
                    &format!("settings: {{ text: {{ type: 'string', default: '', showIf: {{ key: 'mode', anyOf: ['{escape}'] }} }} }}"),
                ),
            ];
            for manifest in variants {
                let error =
                    assert_error_code(&source(&manifest), "script-declaration-lone-surrogate");
                assert!(matches!(
                    error.reason,
                    DeclarationReason::Literal(LiteralReason::LoneSurrogate)
                ));
            }
        }

        let valid_pair = manifest_source_replacing("name: 'Miner'", r"name: '\uD83D\uDE00'");
        let ScriptAnalysis::Native { manifest, .. } =
            analyze(Path::new("manifest.ts"), &valid_pair).unwrap()
        else {
            panic!("valid surrogate pair should decode as a name");
        };
        assert_eq!(manifest.name(), "😀");
        let braced_pair = manifest_source_replacing("name: 'Miner'", r"name: '\u{D83D}\u{DE00}'");
        let ScriptAnalysis::Native { manifest, .. } =
            analyze(Path::new("manifest.ts"), &braced_pair).unwrap()
        else {
            panic!("valid braced surrogate pair should decode as a name");
        };
        assert_eq!(manifest.name(), "😀");

        let literal_backslash = manifest_source_replacing("name: 'Miner'", r"name: '\\uD800'");
        let ScriptAnalysis::Native { manifest, .. } =
            analyze(Path::new("manifest.ts"), &literal_backslash).unwrap()
        else {
            panic!("literal backslash spelling should remain ordinary text");
        };
        assert_eq!(manifest.name(), r"\uD800");
    }

    #[test]
    fn display_fields_reject_every_forbidden_control_and_bidi_character() {
        let forbidden = (0x0000..=0x001f)
            .chain([0x007f])
            .chain(0x0080..=0x009f)
            .chain(0x2028..=0x202e)
            .chain(0x2066..=0x2069);
        for codepoint in forbidden {
            let escaped = format!("\\u{codepoint:04X}");
            let input = manifest_source_replacing("name: 'Miner'", &format!("name: '{escaped}'"));
            let error = assert_error_code(&input, "manifest-field-control");
            assert_eq!(error.field_path, "manifest.name");
            assert!(matches!(
                error.reason,
                DeclarationReason::Manifest(ManifestReason::ControlCharacter)
            ));
        }
    }

    #[test]
    fn display_validation_covers_manifest_and_setting_metadata_fields() {
        for (from, to) in [
            ("version: '1'", "version: '\\u202E'"),
            ("author: 'Alice'", "author: '\\u202E'"),
            ("description: 'Mine ore'", "description: '\\u202E'"),
            ("category: 'Mining'", "category: '\\u202E'"),
            ("tags: []", "tags: ['\\u202E']"),
            (
                "settings: {}",
                "settings: { text: { type: 'string', default: '', label: '\\u202E' } }",
            ),
            (
                "settings: {}",
                "settings: { text: { type: 'string', default: '', help: '\\u202E' } }",
            ),
            (
                "settings: {}",
                "settings: { text: { type: 'string', default: '', group: '\\u202E' } }",
            ),
            (
                "settings: {}",
                "settings: { text: { type: 'string', default: 'option', options: ['option'], optionLabels: { option: '\\u202E' } } }",
            ),
        ] {
            assert_error_code(
                &manifest_source_replacing(from, to),
                "manifest-field-control",
            );
        }

        let invalid_id = manifest_source_replacing(
            "settings: {}",
            "settings: { 'text\\u202E': { type: 'string', default: '' } }",
        );
        assert_error_code(&invalid_id, "setting-id-invalid");
    }

    #[test]
    fn diagnostics_escape_c1_line_separators_and_bidi_characters() {
        for codepoint in [0x0085, 0x2028, 0x2029, 0x202e, 0x2066, 0x2069] {
            let escaped = format!("\\u{{{codepoint:X}}}");
            let input = manifest_source_replacing(
                "capabilities: []",
                &format!("capabilities: [], '{escaped}': 1"),
            );
            let error = assert_error_code(&input, "manifest-unknown-field");
            let message = error.to_string();
            assert!(
                !message.contains(char::from_u32(codepoint).unwrap()),
                "diagnostic contains U+{codepoint:04X}: {message:?}"
            );
        }
    }

    #[test]
    fn manifest_fields_types_and_conflicting_api_exports_are_checked() {
        for (fragment, field) in [
            ("apiVersion: 2, ", "apiVersion"),
            ("name: 'Miner', ", "name"),
            ("version: '1', ", "version"),
            ("author: 'Alice', ", "author"),
            ("description: 'Mine ore', ", "description"),
            ("category: 'Mining', ", "category"),
            ("tags: [], ", "tags"),
            ("settings: {}, ", "settings"),
            ("capabilities: []", "capabilities"),
        ] {
            let input = source(&good_manifest().replace(fragment, ""));
            let error = assert_error_code(&input, "manifest-required-field");
            assert_eq!(error.field_path, format!("manifest.{field}"));
            assert!(matches!(
                error.reason,
                DeclarationReason::Manifest(ManifestReason::MissingField(missing))
                    if missing == field
            ));
        }

        for (from, to, code) in [
            ("apiVersion: 2", "apiVersion: '2'", "manifest-field-type"),
            ("apiVersion: 2", "apiVersion: 3", "api-version-unsupported"),
            ("name: 'Miner'", "name: 1", "manifest-field-type"),
            ("tags: []", "tags: 'ore'", "manifest-field-type"),
            ("settings: {}", "settings: []", "manifest-field-type"),
            (
                "capabilities: []",
                "capabilities: {}",
                "manifest-field-type",
            ),
        ] {
            let error = assert_error_code(&manifest_source_replacing(from, to), code);
            if to == "apiVersion: '2'" {
                assert!(matches!(
                    &error.reason,
                    DeclarationReason::Manifest(ManifestReason::WrongType(
                        "integer numeric literal"
                    ))
                ));
                assert_eq!(error.reason_text(), "expected integer numeric literal");
            }
            if to == "capabilities: {}" {
                assert!(matches!(
                    &error.reason,
                    DeclarationReason::Manifest(ManifestReason::WrongType("array of strings"))
                ));
            }
        }
        for (from, to) in [
            ("apiVersion: 2", "apiVersion: 2.5"),
            ("apiVersion: 2", "apiVersion: -1"),
            ("version: '1'", "version: 1"),
            ("author: 'Alice'", "author: 1"),
            ("description: 'Mine ore'", "description: 1"),
            ("category: 'Mining'", "category: 1"),
            ("tags: []", "tags: [1]"),
            ("capabilities: []", "capabilities: [1]"),
        ] {
            let error = assert_error_code(
                &manifest_source_replacing(from, to),
                match to {
                    "apiVersion: 2.5" | "apiVersion: -1" => "api-version-malformed",
                    _ => "manifest-field-type",
                },
            );
            if to.starts_with("apiVersion") {
                assert!(matches!(
                    &error.reason,
                    DeclarationReason::Manifest(ManifestReason::MalformedApiVersion)
                ));
            }
            if to == "capabilities: [1]" {
                assert!(matches!(
                    &error.reason,
                    DeclarationReason::Manifest(ManifestReason::WrongType("string"))
                ));
            }
        }
        let empty = assert_error_code(
            &manifest_source_replacing("name: 'Miner'", "name: ''"),
            "manifest-empty-field",
        );
        assert!(matches!(
            empty.reason,
            DeclarationReason::Manifest(ManifestReason::Empty)
        ));
        let whitespace = assert_error_code(
            &manifest_source_replacing("name: 'Miner'", "name: ' Miner'"),
            "manifest-field-whitespace",
        );
        assert!(matches!(
            whitespace.reason,
            DeclarationReason::Manifest(ManifestReason::Whitespace)
        ));
        let unknown = assert_error_code(
            &manifest_source_replacing("capabilities: []", "capabilities: [], extra: 1"),
            "manifest-unknown-field",
        );
        assert!(matches!(
            &unknown.reason,
            DeclarationReason::Manifest(ManifestReason::UnknownField(field))
                if field == "extra"
        ));
        assert_error_code(
            &manifest_source_replacing("capabilities: []", "capabilities: ['unknown-capability']"),
            "host-capability-unknown",
        );
        let duplicate_tags = manifest_source_replacing("tags: []", "tags: ['ore', 'ore']");
        let error = assert_error_code(&duplicate_tags, "manifest-duplicate-tag");
        assert!(matches!(
            &error.reason,
            DeclarationReason::Manifest(ManifestReason::DuplicateTag(tag))
                if tag == "ore"
        ));

        for (declaration, name) in [
            ("export const apiVersion = 2;", "apiVersion"),
            ("export const SETTINGS = {};", "SETTINGS"),
        ] {
            let input = format!("{declaration}\n{}", source(good_manifest()));
            let error = analyze(Path::new("manifest.ts"), &input).unwrap_err();
            let ExtractionError::Declaration(error) = error else {
                panic!("unexpected unresolved module");
            };
            assert_eq!(error.code(), "manifest-conflicting-export");
            assert!(matches!(
                &error.reason,
                DeclarationReason::Manifest(ManifestReason::ConflictingExport(conflict))
                    if conflict == name
            ));
        }
    }

    #[test]
    fn native_tick_must_be_a_direct_exported_function() {
        for input in [
            format!(
                "export const manifest = {}; function tick() {{}} export {{ tick }};",
                good_manifest()
            ),
            format!(
                "export const manifest = {}; export const tick = () => {{}};",
                good_manifest()
            ),
            format!(
                "export const manifest = {}; export type {{ tick }} from './missing.ts';",
                good_manifest()
            ),
        ] {
            let error = assert_error_code(&input, "api-version-missing-tick");
            assert!(matches!(
                error.reason,
                DeclarationReason::Manifest(ManifestReason::MissingTick)
            ));
        }
        let async_tick = format!(
            "export const manifest = {}; export async function tick() {{}}",
            good_manifest()
        );
        assert!(matches!(
            analyze(Path::new("manifest.ts"), &async_tick),
            Ok(ScriptAnalysis::Native { .. })
        ));
        let tree = TempTree::new();
        let entry = tree.path().join("entry.ts");
        let input = format!(
            "export const manifest = {}; export {{ tick }} from './tick.ts';",
            good_manifest()
        );
        std::fs::write(&entry, &input).unwrap();
        std::fs::write(tree.path().join("tick.ts"), "export function tick() {}").unwrap();
        let error = analyze(&entry, &input).unwrap_err();
        assert!(matches!(
            error,
            ExtractionError::Declaration(error)
                if error.code() == "api-version-missing-tick"
                    && matches!(error.reason, DeclarationReason::Manifest(ManifestReason::MissingTick))
        ));
    }

    #[test]
    fn settings_projection_accepts_dynamic_defaults_and_rejects_legacy_fields() {
        for source_name in ["loadouts", "named-banks"] {
            for (setting_type, default) in [("string", "''"), ("string[]", "[]")] {
                let replacement = format!(
                    "settings: {{ choice: {{ type: '{setting_type}', default: {default}, optionsFrom: '{source_name}' }} }}"
                );
                let input = manifest_source_replacing("settings: {}", &replacement);
                assert!(
                    matches!(
                        analyze(Path::new("manifest.ts"), &input),
                        Ok(ScriptAnalysis::Native { .. })
                    ),
                    "{setting_type} / {source_name}"
                );
            }
        }

        let conflict = manifest_source_replacing(
            "settings: {}",
            "settings: { choice: { type: 'string', default: '', options: [''], optionsFrom: 'loadouts' } }",
        );
        let error = assert_error_code(&conflict, "setting-options-conflict");
        assert_eq!(error.field_path, "manifest.settings.choice.optionsFrom");
        assert!(matches!(
            error.reason,
            DeclarationReason::Setting(SettingReason::CompetingOptions)
        ));

        for (field, value) in [
            ("step", "-1"),
            ("step", "0"),
            ("step", "1"),
            ("csvToggle", "true"),
            ("color", "'red'"),
        ] {
            let replacement = format!(
                "settings: {{ choice: {{ type: 'number', default: 1, {field}: {value} }} }}"
            );
            let input = manifest_source_replacing("settings: {}", &replacement);
            let error = assert_error_code(&input, "setting-unknown-field");
            assert_eq!(
                error.field_path,
                format!("manifest.settings.choice.{field}")
            );
            assert!(matches!(
                &error.reason,
                DeclarationReason::Setting(SettingReason::UnknownField(unknown))
                    if unknown == field
            ));
        }

        let supported = manifest_source_replacing(
            "settings: {}",
            "settings: { enabled: { type: 'boolean', default: false }, amount: { type: 'number', default: 0, min: 0, max: 0 }, mode: { type: 'string', default: 'a', label: 'Mode', help: 'Pick one', group: 'General', options: ['a', 'b'], optionLabels: { a: 'A', b: 'B' } }, list: { type: 'string[]', default: [] }, location: { type: 'tile', default: { x: 1, z: 2, level: 0 } }, detail: { type: 'string', default: '', showIf: { key: 'mode', anyOf: ['a'] } } }",
        );
        assert!(matches!(
            analyze(Path::new("manifest.ts"), &supported),
            Ok(ScriptAnalysis::Native { .. })
        ));

        let inverted = manifest_source_replacing(
            "settings: {}",
            "settings: { amount: { type: 'number', default: 1, min: 2, max: 1 } }",
        );
        assert_error_code(&inverted, "setting-range-inverted");
        let outside = manifest_source_replacing(
            "settings: {}",
            "settings: { amount: { type: 'number', default: 3, min: 1, max: 2 } }",
        );
        assert_error_code(&outside, "setting-default-range");
    }

    #[test]
    fn reserved_names_reject_non_const_declaration_forms_for_both_families() {
        for (name, form) in [
            ("manifest", "export let manifest = {};"),
            ("manifest", "export const { manifest } = {};"),
            ("manifest", "export const manifest = {}, other = 1;"),
            ("manifest", "export interface manifest {}"),
            ("manifest", "export type manifest = {};"),
            ("manifest", "export enum manifest { A }"),
            ("manifest", "export namespace manifest {}"),
            ("manifest", "export declare const manifest: unknown;"),
            ("hostCapabilities", "export let hostCapabilities = [];"),
            (
                "hostCapabilities",
                "export const { hostCapabilities } = {};",
            ),
            (
                "hostCapabilities",
                "export const hostCapabilities = [], other = 1;",
            ),
            ("hostCapabilities", "export interface hostCapabilities {}"),
            ("hostCapabilities", "export type hostCapabilities = {};"),
            ("hostCapabilities", "export enum hostCapabilities { A }"),
            ("hostCapabilities", "export namespace hostCapabilities {}"),
            (
                "hostCapabilities",
                "export declare const hostCapabilities: unknown;",
            ),
        ] {
            let input = if name == "manifest" {
                format!("{form} export function tick() {{}}")
            } else {
                format!("{form} export default 1;")
            };
            assert_error_code(&input, "script-declaration-form");
        }
    }

    #[test]
    fn compat_capabilities_have_exact_typed_errors() {
        let valid = "export const hostCapabilities = ['host-packet-protocol']; export default class Miner extends TaskBot {}";
        let ScriptAnalysis::Compat { capabilities, .. } =
            analyze(Path::new("compat.ts"), valid).unwrap()
        else {
            panic!("expected compat result");
        };
        assert!(capabilities.contains(HostCapability::HostPacketProtocol));
        let legacy_settings =
            "export const SETTINGS = { mode: 'legacy' }; export default class Miner extends TaskBot {}";
        let ScriptAnalysis::Compat { erased_spans, .. } =
            analyze(Path::new("compat.ts"), legacy_settings).unwrap()
        else {
            panic!("legacy SETTINGS must remain an ordinary compat export");
        };
        assert!(erased_spans.is_empty());
        assert_eq!(
            erase_declarations(legacy_settings, &erased_spans),
            legacy_settings
        );
        for reference in [
            "const compat = defineBot({}); export default hostCapabilities;",
            "const compat = defineBot({}); const alias = hostCapabilities; export default class Miner extends TaskBot {}",
        ] {
            let erased_reference =
                format!("export const hostCapabilities = []; {reference}");
            assert_error_code(&erased_reference, "script-declaration-runtime-reference");
        }

        for (list, code, reason) in [
            (
                "['unknown']",
                "host-capability-unknown",
                CapabilityReason::Unknown("unknown".to_owned()),
            ),
            (
                "['host-packet-protocol', 'host-packet-protocol']",
                "host-capability-duplicate",
                CapabilityReason::Duplicate("host-packet-protocol".to_owned()),
            ),
        ] {
            let input = format!(
                "export const hostCapabilities = {list}; export default class Miner extends TaskBot {{}}"
            );
            let error = analyze(Path::new("compat.ts"), &input).unwrap_err();
            let ExtractionError::Declaration(error) = error else {
                panic!("unexpected unresolved module");
            };
            assert_eq!(error.code(), code);
            assert!(matches!(
                error.reason,
                DeclarationReason::Capability(actual) if actual == reason
            ));
        }
    }

    #[test]
    fn graph_resolves_star_chains_cycles_and_only_known_host_shims() {
        let chain = TempTree::new();
        let entry = chain.path().join("entry.ts");
        std::fs::write(
            &entry,
            "export * from './first.ts'; export function tick() {}",
        )
        .unwrap();
        std::fs::write(
            chain.path().join("first.ts"),
            "export * from './second.ts';",
        )
        .unwrap();
        std::fs::write(
            chain.path().join("second.ts"),
            "export const manifest = {};",
        )
        .unwrap();
        let input = std::fs::read_to_string(&entry).unwrap();
        let error = analyze(&entry, &input).unwrap_err();
        assert!(matches!(
            error,
            ExtractionError::Declaration(error)
                if error.code() == "script-declaration-form"
                    && matches!(error.reason, DeclarationReason::Export(ExportReason::NotDirectConst))
        ));

        let cycle = TempTree::new();
        let entry = cycle.path().join("entry.ts");
        let input = format!(
            "export * from '@rs2b0t/api'; export * from '#/bot/api/bot/Bot.js'; export * from './first.ts'; {}",
            source(good_manifest())
        );
        std::fs::write(&entry, &input).unwrap();
        std::fs::write(
            cycle.path().join("first.ts"),
            "export * from './second.ts';",
        )
        .unwrap();
        std::fs::write(
            cycle.path().join("second.ts"),
            "export * from './first.ts';",
        )
        .unwrap();
        assert!(matches!(
            analyze(&entry, &input),
            Ok(ScriptAnalysis::Native { .. })
        ));

        let namespace = TempTree::new();
        let entry = namespace.path().join("entry.ts");
        let input = "export * as manifest from './first.ts'; export function tick() {}".to_owned();
        std::fs::write(&entry, &input).unwrap();
        std::fs::write(namespace.path().join("first.ts"), "export const value = 1;").unwrap();
        let error = analyze(&entry, &input).unwrap_err();
        assert!(matches!(
            error,
            ExtractionError::Declaration(error)
                if error.code() == "script-declaration-form"
                    && matches!(error.reason, DeclarationReason::Export(ExportReason::NotDirectConst))
        ));

        let unknown = format!(
            "export * from '@unmapped/private'; {}",
            source(good_manifest())
        );
        assert!(matches!(
            analyze(Path::new("entry.ts"), &unknown),
            Err(ExtractionError::UnresolvedModule(error))
                if error.specifier == "@unmapped/private"
        ));
    }

    #[test]
    fn default_entry_star_supplying_host_capabilities_is_refused() {
        let tree = TempTree::new();
        let entry = tree.path().join("entry.ts");
        let input = "export * from './caps.ts'; export default class Miner extends TaskBot {}";
        std::fs::write(&entry, input).unwrap();
        std::fs::write(
            tree.path().join("caps.ts"),
            "export const hostCapabilities = [];",
        )
        .unwrap();

        let error = analyze(&entry, input).unwrap_err();
        assert!(matches!(
            error,
            ExtractionError::Declaration(error)
                if error.code() == "script-declaration-form"
                    && matches!(error.reason, DeclarationReason::Export(ExportReason::NotDirectConst))
        ));
    }

    #[test]
    fn erased_manifest_references_are_binding_aware_across_modules() {
        for reference in [
            "export function tick() { return manifest; }",
            "export function tick() { return { manifest }; }",
            "const alias = manifest; export function tick() {}",
            "export { manifest as metadata }; export function tick() {}",
        ] {
            let input = format!("export const manifest = {}; {reference}", good_manifest());
            assert_error_code(&input, "script-declaration-runtime-reference");
        }

        let shadowed = format!(
            "export const manifest = {}; export function tick(manifest: string) {{ return manifest; }}",
            good_manifest()
        );
        assert!(matches!(
            analyze(Path::new("manifest.ts"), &shadowed),
            Ok(ScriptAnalysis::Native { .. })
        ));
        let type_only = format!(
            "export const manifest = {}; type Metadata = typeof manifest; export function tick() {{}}",
            good_manifest()
        );
        assert!(matches!(
            analyze(Path::new("manifest.ts"), &type_only),
            Ok(ScriptAnalysis::Native { .. })
        ));

        for reader_source in [
            "import { manifest } from './entry.ts'; export const alias = manifest;",
            "export { manifest as alias } from './entry.ts';",
        ] {
            let tree = TempTree::new();
            let entry = tree.path().join("entry.ts");
            let input = format!("\u{feff}import './reader.ts';\n{}", source(good_manifest()));
            std::fs::write(&entry, &input).unwrap();
            std::fs::write(
                tree.path().join("reader.ts"),
                format!("\u{feff}{reader_source}"),
            )
            .unwrap();
            let error = analyze(&entry, &input).unwrap_err();
            assert!(matches!(
                error,
                ExtractionError::Declaration(error)
                    if error.code() == "script-declaration-runtime-reference"
                        && error.path.file_name().and_then(|name| name.to_str()) == Some("reader.ts")
            ));
        }
        let tree = TempTree::new();
        let entry = tree.path().join("entry.ts");
        let input = "import './reader.ts'; export const hostCapabilities = []; const __rs2b0tManifest = {}; export default 1;";
        std::fs::write(&entry, input).unwrap();
        std::fs::write(
            tree.path().join("reader.ts"),
            "import { hostCapabilities } from './entry.ts'; export const alias = hostCapabilities;",
        )
        .unwrap();
        let error = analyze(&entry, input).unwrap_err();
        assert!(matches!(
            error,
            ExtractionError::Declaration(error)
                if error.code() == "script-declaration-runtime-reference"
                    && error.path.file_name().and_then(|name| name.to_str()) == Some("reader.ts")
        ));

        let self_namespace = TempTree::new();
        let entry = self_namespace.path().join("entry.ts");
        for source in [
            format!(
                "import * as self from './entry.ts'; {}",
                source(good_manifest()).replace(
                    "export function tick() {}",
                    "export function tick() { return self.manifest; }"
                )
            ),
            format!(
                "import * as self from './entry.ts'; export {{ self as api }}; {}",
                source(good_manifest())
            ),
            format!(
                "export * as api from './entry.ts'; {}",
                source(good_manifest())
            ),
        ] {
            std::fs::write(&entry, &source).unwrap();
            let error = analyze(&entry, &source).unwrap_err();
            assert!(matches!(
                error,
                ExtractionError::Declaration(error)
                    if error.code() == "script-declaration-runtime-reference"
            ));
        }
    }
}
