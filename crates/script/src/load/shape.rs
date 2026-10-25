//! Shape detection, import scans, fingerprints, and picker selection helpers.
//!
//! V8-free except `transpile_ts` (feature `load`).

#[cfg(feature = "load")]
use std::collections::HashMap;
use std::collections::HashSet;
use std::fmt;
use std::path::{Path, PathBuf};

pub use crate::card::ScriptSel;
#[cfg(feature = "load")]
use crate::js_cache::{CacheMeta, JsCache};
use crate::rs2b0t_registry::ScriptSource;

/// Existing source bound applied to operator-loaded script bodies.
pub(crate) const MAX_SOURCE_BYTES: u64 = 8 * 1024 * 1024;
/// A reload resolves a finite module graph and charges all raw source bytes
/// together, not merely each file independently.
pub(crate) const MAX_PREPARATION_SOURCE_BYTES: usize = 8 * 1024 * 1024;
pub(crate) const MAX_PREPARATION_SIBLINGS: usize = 128;
pub(crate) const MAX_PREPARATION_RESULT_BYTES: usize = 16 * 1024 * 1024;

#[cfg(feature = "load")]
fn read_preparation_source(path: &Path) -> Result<String, String> {
    vault::read_regular_file(path, MAX_SOURCE_BYTES)
        .map_err(|error| format!("read {}: {error}", path.display()))
}

/// Which loader a JS source belongs to.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum LoadShape {
    /// Old rs2b0t `defineBot(...)` / manifest-flagged source.
    CompatDefineBot,
    /// Catalog shape: default-export `LoopingBot`/`TaskBot`/`TreeBot`
    /// subclass (TS, transpiled at Load and at isolate spawn).
    CompatClass,
    /// Modern source exporting a `tick` function.
    NativeTick,
    /// Not a recognized bot shape.
    Reject,
}

/// Classify a JS source by marker scan. Compat markers win over the
/// native `tick` export when a source carries both, and a default-export
/// `LoopingBot`/`TaskBot`/`TreeBot` subclass (the catalog shape) also
/// wins over the native `tick` export.
pub fn detect_shape(source: &str) -> LoadShape {
    if source.contains("defineBot(") || source.contains("__rs2b0tManifest") {
        LoadShape::CompatDefineBot
    } else if source.contains("export default class")
        && ["LoopingBot", "TaskBot", "TreeBot"]
            .iter()
            .any(|base| source.contains(&format!("extends {base}")))
    {
        LoadShape::CompatClass
    } else if source.contains("export function tick")
        || source.contains("export async function tick")
    {
        LoadShape::NativeTick
    } else {
        LoadShape::Reject
    }
}

/// Provenance for which public JS API a card was classified against.
/// Not part of identity or the cache key.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum ApiFamily {
    /// Unversioned `export function tick` — today's Proxy (`api.tick` only).
    Unversioned,
    /// rs2b0t compatibility load path.
    V1,
    /// Native host authoring (`export const apiVersion = 2`).
    V2,
}

impl ApiFamily {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Unversioned => "unversioned",
            Self::V1 => "v1",
            Self::V2 => "v2",
        }
    }
}

/// Named load diagnostic for an explicit `apiVersion` declaration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VersionDiag {
    Malformed(&'static str),
    Unsupported(u32),
}

impl VersionDiag {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Malformed(_) => "api-version-malformed",
            Self::Unsupported(_) => "api-version-unsupported",
        }
    }
}

impl fmt::Display for VersionDiag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Malformed(why) => write!(f, "api-version-malformed: {why}"),
            Self::Unsupported(n) => write!(f, "api-version-unsupported: {n}"),
        }
    }
}

/// Parse an exported `const apiVersion = <numeric>` declaration.
///
/// Comments and string literals are ignored. Numeric literal `2.0` equals `2`.
/// Without `load`, classification stays marker-only.
pub fn parse_declared_api_version(source: &str) -> Result<Option<u32>, VersionDiag> {
    #[cfg(feature = "load")]
    {
        parse_declared_api_version_ast(source)
    }
    #[cfg(not(feature = "load"))]
    {
        let _ = source;
        Ok(None)
    }
}

/// Combine [`detect_shape`] with [`parse_declared_api_version`].
/// Never silently falls back from an explicit version.
pub fn resolve_api_family(source: &str) -> Result<(LoadShape, ApiFamily), String> {
    let shape = detect_shape(source);
    let version = parse_declared_api_version(source).map_err(|d| d.to_string())?;
    match (shape, version) {
        (LoadShape::NativeTick, Some(2)) => Ok((shape, ApiFamily::V2)),
        (LoadShape::CompatDefineBot | LoadShape::CompatClass, Some(2)) => {
            Err("api-version-conflict: v2 with compatibility shape".into())
        }
        (LoadShape::NativeTick, Some(1)) => Err("api-version-conflict: v1 with native tick".into()),
        (LoadShape::CompatDefineBot | LoadShape::CompatClass, Some(1)) => {
            Ok((shape, ApiFamily::V1))
        }
        (LoadShape::Reject, Some(2)) => Err("api-version-missing-tick".into()),
        (LoadShape::Reject, Some(1)) => Ok((shape, ApiFamily::Unversioned)),
        (LoadShape::NativeTick, None) => Ok((shape, ApiFamily::Unversioned)),
        (LoadShape::CompatDefineBot | LoadShape::CompatClass, None) => Ok((shape, ApiFamily::V1)),
        (LoadShape::Reject, None) => Ok((shape, ApiFamily::Unversioned)),
        (_, Some(other)) => Err(format!("api-version-unsupported: {other}")),
    }
}

#[cfg(feature = "load")]
fn parse_declared_api_version_ast(source: &str) -> Result<Option<u32>, VersionDiag> {
    use deno_ast::swc::ast::{Decl, ModuleDecl, ModuleItem, Pat, VarDeclKind};
    use deno_ast::ProgramRef;

    let specifier = deno_ast::ModuleSpecifier::parse("file:///bot.ts")
        .map_err(|_| VersionDiag::Malformed("specifier"))?;
    let parsed = match deno_ast::parse_module(deno_ast::ParseParams {
        specifier,
        text: source.to_string().into(),
        media_type: deno_ast::MediaType::TypeScript,
        capture_tokens: false,
        scope_analysis: false,
        maybe_syntax: None,
    }) {
        Ok(parsed) => parsed,
        Err(_) => return Ok(None),
    };

    let mut found: Option<Result<u32, VersionDiag>> = None;
    let mut note = |next: Result<u32, VersionDiag>| -> Result<(), VersionDiag> {
        if found.is_some() {
            return Err(VersionDiag::Malformed("multiple apiVersion exports"));
        }
        found = Some(next);
        Ok(())
    };

    let body: &[ModuleItem] = match parsed.program_ref() {
        ProgramRef::Module(m) => &m.body,
        ProgramRef::Script(_) => return Ok(None),
    };
    for item in body {
        let ModuleItem::ModuleDecl(decl) = item else {
            continue;
        };
        if let ModuleDecl::ExportDecl(export) = decl {
            match &export.decl {
                Decl::Var(var) => {
                    for declarator in &var.decls {
                        let Pat::Ident(ident) = &declarator.name else {
                            continue;
                        };
                        if ident.id.sym.as_str() != "apiVersion" {
                            continue;
                        }
                        if var.kind != VarDeclKind::Const {
                            note(Err(VersionDiag::Malformed(
                                "apiVersion must be export const",
                            )))?;
                            continue;
                        }
                        let Some(init) = declarator.init.as_deref() else {
                            note(Err(VersionDiag::Malformed("apiVersion has no initializer")))?;
                            continue;
                        };
                        match numeric_literal_version(init) {
                            Ok(n) => note(Ok(n))?,
                            Err(d) => note(Err(d))?,
                        }
                    }
                }
                Decl::Fn(fun) if fun.ident.sym.as_str() == "apiVersion" => {
                    note(Err(VersionDiag::Malformed(
                        "apiVersion must be export const",
                    )))?;
                }
                Decl::Class(class) if class.ident.sym.as_str() == "apiVersion" => {
                    note(Err(VersionDiag::Malformed(
                        "apiVersion must be export const",
                    )))?;
                }
                _ => {}
            }
        }
    }

    match found {
        None => Ok(None),
        Some(Ok(n @ 1 | n @ 2)) => Ok(Some(n)),
        Some(Ok(other)) => Err(VersionDiag::Unsupported(other)),
        Some(Err(d)) => Err(d),
    }
}

#[cfg(feature = "load")]
fn numeric_literal_version(expr: &deno_ast::swc::ast::Expr) -> Result<u32, VersionDiag> {
    use deno_ast::swc::ast::{Expr, Lit};
    let inner = unwrap_ts_assertion(expr);
    match inner {
        Expr::Lit(Lit::Num(n)) => {
            if !n.value.is_finite() || n.value < 0.0 || n.value.fract() != 0.0 {
                return Err(VersionDiag::Malformed(
                    "apiVersion must be an integer numeric literal",
                ));
            }
            if n.value > u32::MAX as f64 {
                return Err(VersionDiag::Malformed("apiVersion out of range"));
            }
            Ok(n.value as u32)
        }
        Expr::Lit(Lit::Str(_)) => Err(VersionDiag::Malformed("apiVersion must not be a string")),
        Expr::Lit(Lit::Bool(_)) => Err(VersionDiag::Malformed("apiVersion must not be a boolean")),
        _ => Err(VersionDiag::Malformed(
            "apiVersion must be a numeric literal",
        )),
    }
}

#[cfg(feature = "load")]
fn unwrap_ts_assertion(expr: &deno_ast::swc::ast::Expr) -> &deno_ast::swc::ast::Expr {
    use deno_ast::swc::ast::Expr;
    match expr {
        Expr::TsAs(n) => unwrap_ts_assertion(&n.expr),
        Expr::TsTypeAssertion(n) => unwrap_ts_assertion(&n.expr),
        Expr::TsConstAssertion(n) => unwrap_ts_assertion(&n.expr),
        Expr::TsSatisfies(n) => unwrap_ts_assertion(&n.expr),
        other => other,
    }
}

/// Strip TypeScript from `source` (types, `private`/`override` markers,
/// type-only imports) and re-emit as a JavaScript module V8 can parse.
/// Plain JS passes through unchanged in behaviour. A parse failure means
/// the source is not readable TypeScript/JavaScript.
#[cfg(feature = "load")]
pub fn transpile_ts(source: &str) -> Result<String, String> {
    let specifier = deno_ast::ModuleSpecifier::parse("file:///bot.ts")
        .map_err(|e| format!("ts specifier: {e}"))?;
    let parsed = deno_ast::parse_module(deno_ast::ParseParams {
        specifier,
        text: source.to_string().into(),
        media_type: deno_ast::MediaType::TypeScript,
        capture_tokens: false,
        scope_analysis: false,
        maybe_syntax: None,
    })
    .map_err(|e| format!("ts parse: {e}"))?;
    let emitted = parsed
        .transpile(
            &deno_ast::TranspileOptions::default(),
            &deno_ast::TranspileModuleOptions::default(),
            &deno_ast::EmitOptions {
                source_map: deno_ast::SourceMapOption::None,
                ..Default::default()
            },
        )
        .map_err(|e| e.to_string())?
        .into_source();
    Ok(emitted.text)
}

/// Live gold file fixtures under `tests/fixtures/` (not catalog whales).
pub fn live_file_fixture_path(name: &str) -> Option<PathBuf> {
    let base = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    match name {
        "TradeBot" => Some(base.join("trade_bot.ts")),
        _ => None,
    }
}

/// File stem registered by [`live_file_fixture_path`] for a gold card name.
pub fn live_file_fixture_stem(name: &str) -> Option<&'static str> {
    match name {
        "TradeBot" => Some("trade_bot"),
        _ => None,
    }
}

/// Exact in-tree example files loaded as File cards (not catalog, not fixtures).
/// Lookup is by file name including extension so `.ts` and `.js` stay distinct.
pub fn live_example_path(file_name: &str) -> Option<PathBuf> {
    match file_name {
        "bone_burier_v2.ts"
        | "bone_burier_v2.js"
        | "guardian_claim_toggle.js"
        | "route_inspect_brimhaven_v2.ts"
        | "prayer_v2.ts"
        | "prayer_v1.ts"
        | "boat_fare_v1.ts"
        | "line_of_sight_v2.ts"
        | "actor_observation_v2.ts"
        | "fight_field_v2.ts"
        | "hold_spot_v2.ts"
        | "retreat_spot_v2.ts"
        | "walk_spot_v2.ts"
        | "enter_lair_v2.ts"
        | "leave_lair_v2.ts"
        | "acquire_key_v2.ts"
        | "cell_v2.ts"
        | "bank_v2.ts" => {
            let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("examples")
                .join(file_name);
            path.is_file().then_some(path)
        }
        _ => None,
    }
}

/// Scan `source` for same-folder `./Name.js` import specifiers (quoted).
pub fn scan_same_folder_js_imports(source: &str) -> Vec<String> {
    let mut out = Vec::new();
    for quote in ['\'', '"'] {
        let needle = format!("{quote}./");
        let mut rest = source;
        while let Some(idx) = rest.find(&needle) {
            let after = &rest[idx + needle.len()..];
            if let Some(end) = after.find(quote) {
                let spec = &after[..end];
                if spec.ends_with(".js")
                    && !spec.contains("..")
                    && !spec.contains('/')
                    && !spec.contains('\\')
                {
                    let import = format!("./{spec}");
                    if !out.iter().any(|x| x == &import) {
                        out.push(import);
                    }
                }
            }
            rest = &rest[idx + 1..];
        }
    }
    out
}

/// Runtime import / re-export specifiers, first-seen source order. Type-only
/// declarations are skipped: the transpiled module never loads them.
pub fn scan_import_specifiers(source: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for import in crate::module_imports::module_imports(source) {
        if !import.is_runtime() {
            continue;
        }
        let specifier = import.into_specifier();
        if looks_like_module_specifier(&specifier) && !out.contains(&specifier) {
            out.push(specifier);
        }
    }
    out
}

fn looks_like_module_specifier(spec: &str) -> bool {
    spec.starts_with('.') || spec.starts_with('#') || spec.starts_with('@') || spec.starts_with('/')
}

pub fn scan_scripts_sibling_js_imports(source: &str) -> Vec<String> {
    scan_import_specifiers(source)
        .into_iter()
        .filter(|s| is_scripts_sibling_specifier(s))
        .collect()
}

fn is_same_folder_specifier(spec: &str) -> bool {
    let Some(name) = spec.strip_prefix("./") else {
        return false;
    };
    (spec.ends_with(".js") || spec.ends_with(".ts"))
        && !spec.contains("..")
        && !name.contains('/')
        && !name.contains('\\')
}

fn specifier_js(spec: &str) -> String {
    if let Some(stem) = spec.strip_suffix(".ts") {
        format!("{stem}.js")
    } else {
        spec.to_string()
    }
}

#[cfg(feature = "load")]
fn strip_relative_prefix(spec: &str) -> &str {
    let mut p = spec;
    loop {
        if let Some(rest) = p.strip_prefix("../") {
            p = rest;
            continue;
        }
        if let Some(rest) = p.strip_prefix("./") {
            p = rest;
            continue;
        }
        break;
    }
    p
}

fn specifier_remaps_to_shim(spec: &str) -> bool {
    #[cfg(feature = "load")]
    {
        let js = specifier_js(spec);
        let suffix = if let Some(rest) = js.strip_prefix("#/bot/") {
            rest.to_string()
        } else {
            strip_relative_prefix(&js).to_string()
        };
        if suffix.is_empty() || suffix.starts_with('/') {
            return false;
        }
        crate::shim::shim_modules().iter().any(|m| {
            m.filename()
                .to_str()
                .is_some_and(|url| url.ends_with(&suffix) || url.ends_with(&format!("/{suffix}")))
        })
    }
    #[cfg(not(feature = "load"))]
    {
        let _ = spec;
        false
    }
}

fn specifier_is_ok(spec: &str) -> bool {
    spec == "@rs2b0t/api"
        || is_same_folder_specifier(spec)
        || is_scripts_sibling_specifier(spec)
        || specifier_remaps_to_shim(spec)
}

/// `../HerbCleaner/HerbCleanerLogic.js` from a script folder: one hop up
/// into `src/bot/scripts`, not out of the clone.
fn is_scripts_sibling_specifier(spec: &str) -> bool {
    let Some(rest) = spec.strip_prefix("../") else {
        return false;
    };
    if rest.contains("..") {
        return false;
    }
    let mut parts = rest.split('/');
    matches!(
        (parts.next(), parts.next(), parts.next()),
        (Some(folder), Some(file), None)
            if !folder.is_empty()
                && (file.ends_with(".js") || file.ends_with(".ts"))
    )
}

/// First import specifier in `source` that is not remapped / same-folder / `@rs2b0t/api`.
pub fn first_unloadable_specifier(source: &str) -> Option<String> {
    scan_import_specifiers(source)
        .into_iter()
        .find(|s| !specifier_is_ok(s))
}

/// Card-level scan: the origin, then same-folder sibling origins that exist.
pub fn first_unloadable_for_card(origin: &str, card_path: &Path) -> Option<String> {
    if let Some(spec) = first_unloadable_specifier(origin) {
        return Some(spec);
    }
    let dir = card_path.parent()?;
    for rel in scan_same_folder_js_imports(origin) {
        let Some(sib) = resolve_sibling_path(dir, &rel) else {
            continue;
        };
        let Ok(text) = std::fs::read_to_string(&sib) else {
            continue;
        };
        if let Some(spec) = first_unloadable_specifier(&text) {
            return Some(spec);
        }
    }
    for rel in scan_scripts_sibling_js_imports(origin) {
        let Some(sib) = resolve_sibling_path(dir, &rel) else {
            continue;
        };
        let Ok(text) = std::fs::read_to_string(&sib) else {
            continue;
        };
        if let Some(spec) = first_unloadable_specifier(&text) {
            return Some(spec);
        }
    }
    None
}

/// Map a `./Foo.js` import from [`BOT_MODULE`] to the synthetic module URL
/// rustyscript resolves.
pub fn sibling_module_url(import_rel: &str) -> Option<String> {
    if let Some(name) = import_rel.strip_prefix("./") {
        if name.contains("..") || name.contains('/') || name.contains('\\') {
            return None;
        }
        return Some(format!("/rs2b0t/bot/scripts/bot/{name}"));
    }
    if is_scripts_sibling_specifier(import_rel) {
        let js = specifier_js(import_rel.trim_start_matches("../"));
        return Some(format!("/rs2b0t/bot/scripts/{js}"));
    }
    None
}

/// Resolve a same-folder `./Foo.js` import beside `card_path`, or a
/// `../Other/File.js` import under the parent `scripts/` folder.
pub fn resolve_sibling_path(card_dir: &Path, import_rel: &str) -> Option<PathBuf> {
    if let Some(rel) = import_rel.strip_prefix("./") {
        if rel.contains("..") {
            return None;
        }
        let verbatim = card_dir.join(rel);
        let candidate = file_or_ts_twin(card_dir, rel, verbatim)?;
        return canonical_under_dir(card_dir, &candidate);
    }
    if is_scripts_sibling_specifier(import_rel) {
        let rest = import_rel.strip_prefix("../")?;
        let scripts_root = card_dir.parent()?;
        let verbatim = scripts_root.join(rest);
        let candidate = file_or_ts_twin(scripts_root, rest, verbatim)?;
        return canonical_under_dir(scripts_root, &candidate);
    }
    None
}

fn file_or_ts_twin(root: &Path, rel: &str, verbatim: PathBuf) -> Option<PathBuf> {
    if verbatim.is_file() {
        return Some(verbatim);
    }
    let stem = rel.strip_suffix(".js")?;
    let ts = root.join(format!("{stem}.ts"));
    ts.is_file().then_some(ts)
}

fn canonical_under_dir(dir: &Path, path: &Path) -> Option<PathBuf> {
    if !path.starts_with(dir) {
        return None;
    }
    if let (Ok(canon_dir), Ok(canon_path)) = (dir.canonicalize(), path.canonicalize()) {
        if !canon_path.starts_with(&canon_dir) {
            return None;
        }
        return Some(canon_path);
    }
    Some(path.to_path_buf())
}

fn same_path(a: &Path, b: &Path) -> bool {
    if a == b {
        return true;
    }
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(x), Ok(y)) => x == y,
        _ => false,
    }
}

/// One sibling graph node in discovery order: relative import, module URL,
/// disk path, and bounded UTF-8 source.
#[cfg(feature = "load")]
pub(crate) type PreparationSibling = (String, String, PathBuf, String);

#[cfg(feature = "load")]
#[derive(Debug)]
pub(crate) struct PreparationSiblingWork {
    card_path: PathBuf,
    card_dir: PathBuf,
    source_limit: usize,
    source_bytes: usize,
    pending: Vec<String>,
    queued: HashSet<String>,
    seen: HashSet<String>,
    nodes: Vec<PreparationSibling>,
}

#[cfg(feature = "load")]
impl PreparationSiblingWork {
    pub(crate) fn new(card_path: &Path, origin: &str, source_limit: usize) -> Result<Self, String> {
        let source_limit = source_limit.min(MAX_PREPARATION_SOURCE_BYTES);
        if origin.len() > source_limit {
            return Err("script and sibling sources exceed the 8 MiB preparation limit".into());
        }
        let card_dir = card_path
            .parent()
            .ok_or_else(|| format!("no parent dir for {}", card_path.display()))?;
        let mut pending = Vec::new();
        let mut queued = HashSet::new();
        for import_rel in scan_same_folder_js_imports(origin)
            .into_iter()
            .chain(scan_scripts_sibling_js_imports(origin))
        {
            if queued.insert(import_rel.clone()) {
                if queued.len() > MAX_PREPARATION_SIBLINGS {
                    return Err("script has more than 128 sibling modules".into());
                }
                pending.push(import_rel);
            }
        }
        Ok(Self {
            card_path: card_path.to_path_buf(),
            card_dir: card_dir.to_path_buf(),
            source_limit,
            source_bytes: origin.len(),
            pending,
            queued,
            seen: HashSet::new(),
            nodes: Vec::new(),
        })
    }

    /// Process no more than one queued sibling source.
    pub(crate) fn step(&mut self) -> Result<bool, String> {
        let Some(import_rel) = self.pending.pop() else {
            return Ok(false);
        };
        self.queued.remove(&import_rel);
        if !self.seen.insert(import_rel.clone()) {
            return Ok(true);
        }
        if self.seen.len() > MAX_PREPARATION_SIBLINGS {
            return Err("script has more than 128 sibling modules".into());
        }
        let Some(url) = sibling_module_url(&import_rel) else {
            return Ok(true);
        };
        let Some(path) = resolve_sibling_path(&self.card_dir, &import_rel) else {
            return Ok(true);
        };
        if same_path(&path, &self.card_path) {
            return Ok(true);
        }
        let origin_text = read_preparation_source(&path)
            .map_err(|error| format!("sibling {}: {error}", path.display()))?;
        self.source_bytes = self
            .source_bytes
            .checked_add(origin_text.len())
            .filter(|bytes| *bytes <= self.source_limit)
            .ok_or_else(|| {
                "script and sibling sources exceed the 8 MiB preparation limit".to_string()
            })?;
        let mut imports = scan_same_folder_js_imports(&origin_text);
        imports.extend(scan_scripts_sibling_js_imports(&origin_text));
        for dependency in imports {
            if self.queued.insert(dependency.clone()) {
                if self.queued.len() + self.seen.len() > MAX_PREPARATION_SIBLINGS {
                    return Err("script has more than 128 sibling modules".into());
                }
                self.pending.push(dependency);
            }
        }
        self.nodes.push((import_rel, url, path, origin_text));
        Ok(true)
    }

    pub(crate) fn finish(self) -> Vec<PreparationSibling> {
        self.nodes
    }
}

#[cfg(feature = "load")]
#[derive(Debug)]
pub(crate) struct PreparationFingerprintWork {
    graph: Option<PreparationSiblingWork>,
    nodes: Option<Vec<PreparationSibling>>,
    entry_hash: Option<String>,
    sibling_hashes: Vec<(String, String)>,
    sibling_index: usize,
    finished: bool,
}

#[cfg(feature = "load")]
impl PreparationFingerprintWork {
    pub(crate) fn new(card_path: &Path, origin: &str, source_limit: usize) -> Result<Self, String> {
        Ok(Self {
            graph: Some(PreparationSiblingWork::new(
                card_path,
                origin,
                source_limit,
            )?),
            nodes: None,
            entry_hash: None,
            sibling_hashes: Vec::new(),
            sibling_index: 0,
            finished: false,
        })
    }

    /// Read/hash no more than one graph node before yielding.
    pub(crate) fn step(&mut self, origin: &str) -> Result<Option<String>, String> {
        if self.finished {
            return Err("preparation fingerprint work completed more than once".into());
        }
        if let Some(graph) = self.graph.as_mut() {
            if graph.step()? {
                return Ok(None);
            }
            let graph = self.graph.take().expect("fingerprint graph exists");
            self.nodes = Some(graph.finish());
            self.entry_hash = Some(crate::identity::raw_sha(origin.as_bytes()));
            return Ok(None);
        }
        let nodes = self.nodes.as_ref().expect("fingerprint nodes exist");
        if let Some((relative, _, _, text)) = nodes.get(self.sibling_index) {
            self.sibling_hashes
                .push((relative.clone(), crate::identity::raw_sha(text.as_bytes())));
            self.sibling_index += 1;
            return Ok(None);
        }
        let fingerprint = crate::identity::combine_fingerprints(
            self.entry_hash
                .as_deref()
                .expect("entry fingerprint exists"),
            &self.sibling_hashes,
        );
        self.nodes = None;
        self.finished = true;
        Ok(Some(fingerprint))
    }
}

#[cfg(feature = "load")]
#[derive(Debug)]
pub(crate) struct FirstUnloadableWork {
    result: Option<Option<String>>,
    graph: Option<PreparationSiblingWork>,
    nodes: Option<Vec<PreparationSibling>>,
    imports: Vec<String>,
    import_index: usize,
}

#[cfg(feature = "load")]
impl FirstUnloadableWork {
    pub(crate) fn new(origin: &str, card_path: &Path, source_limit: usize) -> Result<Self, String> {
        if let Some(specifier) = first_unloadable_specifier(origin) {
            return Ok(Self {
                result: Some(Some(specifier)),
                graph: None,
                nodes: None,
                imports: Vec::new(),
                import_index: 0,
            });
        }
        let imports = scan_same_folder_js_imports(origin)
            .into_iter()
            .chain(scan_scripts_sibling_js_imports(origin))
            .collect();
        Ok(Self {
            result: None,
            graph: Some(PreparationSiblingWork::new(
                card_path,
                origin,
                source_limit,
            )?),
            nodes: None,
            imports,
            import_index: 0,
        })
    }

    /// Read no more than one graph node or inspect one direct dependency.
    pub(crate) fn step(&mut self) -> Result<Option<Option<String>>, String> {
        if let Some(result) = self.result.take() {
            return Ok(Some(result));
        }
        if let Some(graph) = self.graph.as_mut() {
            if graph.step()? {
                return Ok(None);
            }
            let graph = self.graph.take().expect("unloadable graph exists");
            self.nodes = Some(graph.finish());
            return Ok(None);
        }
        if let Some(import_rel) = self.imports.get(self.import_index) {
            self.import_index += 1;
            let siblings = self.nodes.as_ref().expect("unloadable nodes exist");
            if let Some((_, _, _, source)) = siblings
                .iter()
                .find(|(relative, _, _, _)| relative == import_rel)
            {
                if let Some(specifier) = first_unloadable_specifier(source) {
                    self.result = Some(Some(specifier));
                }
            }
            if let Some(result) = self.result.take() {
                return Ok(Some(result));
            }
            return Ok(None);
        }
        Ok(Some(None))
    }
}

#[cfg(feature = "load")]
#[derive(Debug)]
pub(crate) struct SiblingCompilationWork {
    graph: Option<PreparationSiblingWork>,
    nodes: Option<std::vec::IntoIter<PreparationSibling>>,
    cache: JsCache,
    meta: CacheMeta,
    used_result_bytes: usize,
    compiled: Vec<(String, String, String, String)>,
}

#[cfg(feature = "load")]
impl SiblingCompilationWork {
    pub(crate) fn new(
        card_path: &Path,
        origin: &str,
        cache: JsCache,
        meta: CacheMeta,
        current_result_bytes: usize,
        source_limit: usize,
    ) -> Result<Self, String> {
        Ok(Self {
            graph: Some(PreparationSiblingWork::new(
                card_path,
                origin,
                source_limit,
            )?),
            nodes: None,
            cache,
            meta,
            used_result_bytes: current_result_bytes,
            compiled: Vec::new(),
        })
    }

    /// Read one sibling or compile one node before yielding.
    pub(crate) fn step(&mut self) -> Result<Option<Vec<(String, String)>>, String> {
        if let Some(graph) = self.graph.as_mut() {
            if graph.step()? {
                return Ok(None);
            }
            let graph = self.graph.take().expect("sibling graph exists");
            let nodes = graph.finish();
            self.compiled = Vec::with_capacity(nodes.len());
            self.nodes = Some(nodes.into_iter());
            return Ok(None);
        }
        if let Some((relative, url, path, source)) =
            self.nodes.as_mut().and_then(|nodes| nodes.next())
        {
            let js = cached_preparation_js(
                &self.cache,
                &path,
                &source,
                &self.meta,
                self.used_result_bytes,
            )?;
            self.used_result_bytes = self
                .used_result_bytes
                .checked_add(js.len())
                .ok_or_else(|| "prepared JavaScript exceeds the 16 MiB result limit".to_string())?;
            self.compiled.push((relative, url, js, source));
            return Ok(None);
        }
        Ok(Some(order_siblings_deps_first(std::mem::take(
            &mut self.compiled,
        ))))
    }
}

#[cfg(feature = "load")]
fn cached_preparation_js(
    cache: &JsCache,
    path: &Path,
    source: &str,
    meta: &CacheMeta,
    current_result_bytes: usize,
) -> Result<String, String> {
    let remaining = MAX_PREPARATION_RESULT_BYTES
        .checked_sub(current_result_bytes)
        .ok_or_else(|| "prepared JavaScript exceeds the 16 MiB result limit".to_string())?;
    let object = cache.object_path(&JsCache::origin_sha(source.as_bytes()));
    if std::fs::metadata(&object).is_ok_and(|metadata| metadata.len() > remaining as u64) {
        return Err("cached JavaScript exceeds the 16 MiB result limit".into());
    }
    let cached = cache.get_or_transpile(path, source.as_bytes(), meta.clone())?;
    if cached.js.len() > remaining {
        return Err("prepared JavaScript exceeds the 16 MiB result limit".into());
    }
    Ok(cached.js)
}

/// Same-folder `./Foo.js` imports beside `card_path`: read the `.ts` twin
/// (or `.js` origin), cache under `js-cache`, return `(module_url, js)` pairs
/// for extra rustyscript modules at Start.
#[cfg(feature = "load")]
pub fn resolve_sibling_modules(
    card_path: &Path,
    origin: &str,
    cache: &JsCache,
    meta: CacheMeta,
) -> Result<Vec<(String, String)>, String> {
    resolve_sibling_modules_bounded(
        card_path,
        origin,
        cache,
        meta,
        0,
        MAX_PREPARATION_SOURCE_BYTES,
    )
}

/// Resolve the supported sibling graph with source and result bounds. The
/// entry script's JavaScript output is included in `current_result_bytes`.
#[cfg(feature = "load")]
pub(crate) fn resolve_sibling_modules_bounded(
    card_path: &Path,
    origin: &str,
    cache: &JsCache,
    meta: CacheMeta,
    current_result_bytes: usize,
    source_limit: usize,
) -> Result<Vec<(String, String)>, String> {
    let mut work = SiblingCompilationWork::new(
        card_path,
        origin,
        JsCache::new(cache.root().to_path_buf()),
        meta,
        current_result_bytes,
        source_limit,
    )?;
    loop {
        if let Some(siblings) = work.step()? {
            return Ok(siblings);
        }
    }
}

/// Raw entry plus the supported sibling graph fingerprint, under the same
/// source/count limits as candidate preparation.
#[cfg(feature = "load")]
pub(crate) fn raw_content_fingerprint_bounded(
    card_path: &Path,
    origin: &str,
    source_limit: usize,
) -> Result<String, String> {
    let mut work = PreparationFingerprintWork::new(card_path, origin, source_limit)?;
    loop {
        if let Some(fingerprint) = work.step(origin)? {
            return Ok(fingerprint);
        }
    }
}

/// Bounded counterpart used only while preparing or revalidating a reload.
#[cfg(feature = "load")]
pub(crate) fn first_unloadable_for_card_bounded(
    origin: &str,
    card_path: &Path,
    source_limit: usize,
) -> Result<Option<String>, String> {
    let mut work = FirstUnloadableWork::new(origin, card_path, source_limit)?;
    loop {
        if let Some(result) = work.step()? {
            return Ok(result);
        }
    }
}

/// Raw entry + supported sibling hashes, no transpile.
pub fn collect_raw_sibling_hashes(card_path: &Path, origin: &str) -> Vec<(String, String)> {
    let Some(card_dir) = card_path.parent() else {
        return Vec::new();
    };
    let mut pending: Vec<String> = scan_same_folder_js_imports(origin)
        .into_iter()
        .chain(scan_scripts_sibling_js_imports(origin))
        .collect();
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    while let Some(import_rel) = pending.pop() {
        if !seen.insert(import_rel.clone()) {
            continue;
        }
        let Some(path) = resolve_sibling_path(card_dir, &import_rel) else {
            continue;
        };
        if same_path(&path, card_path) {
            continue;
        }
        let Ok(bytes) = std::fs::read(&path) else {
            continue;
        };
        let origin_text = String::from_utf8_lossy(&bytes).into_owned();
        pending.extend(scan_same_folder_js_imports(&origin_text));
        pending.extend(scan_scripts_sibling_js_imports(&origin_text));
        out.push((import_rel, crate::identity::raw_sha(&bytes)));
    }
    out
}

pub fn raw_content_fingerprint(card_path: &Path, origin: &str) -> String {
    let entry = crate::identity::raw_sha(origin.as_bytes());
    let siblings = collect_raw_sibling_hashes(card_path, origin);
    crate::identity::combine_fingerprints(&entry, &siblings)
}

/// rustyscript evaluates side modules in vec order. An importer whose
/// `./dep.js` is later in the list gets "module … is not loaded".
#[cfg(feature = "load")]
fn order_siblings_deps_first(
    nodes: Vec<(String, String, String, String)>,
) -> Vec<(String, String)> {
    if nodes.len() <= 1 {
        return nodes.into_iter().map(|(_, url, js, _)| (url, js)).collect();
    }
    let rels: HashSet<String> = nodes.iter().map(|(rel, _, _, _)| rel.clone()).collect();
    let mut deps: HashMap<String, Vec<String>> = HashMap::new();
    for (rel, _, js, _) in &nodes {
        let mut needed = Vec::new();
        // Order by emitted runtime imports. Type-only edges in the TypeScript
        // origin disappear during transpilation and must not manufacture a
        // dependency cycle (JiveKQ route ↔ party is one such graph).
        for imp in scan_same_folder_js_imports(js)
            .into_iter()
            .chain(scan_scripts_sibling_js_imports(js))
        {
            if rels.contains(&imp) && imp != *rel {
                needed.push(imp);
            }
        }
        deps.insert(rel.clone(), needed);
    }
    let mut remaining: HashSet<String> = rels;
    let mut sorted_rels = Vec::new();
    while !remaining.is_empty() {
        let ready: Vec<String> = remaining
            .iter()
            .filter(|r| {
                deps.get(*r)
                    .into_iter()
                    .flatten()
                    .all(|d| !remaining.contains(d))
            })
            .cloned()
            .collect();
        if ready.is_empty() {
            let mut rest: Vec<String> = remaining.iter().cloned().collect();
            rest.sort();
            sorted_rels.extend(rest);
            break;
        }
        let mut ready = ready;
        ready.sort();
        for r in ready {
            remaining.remove(&r);
            sorted_rels.push(r);
        }
    }
    let mut by_rel: HashMap<String, (String, String)> = nodes
        .into_iter()
        .map(|(rel, url, js, _)| (rel, (url, js)))
        .collect();
    sorted_rels
        .into_iter()
        .filter_map(|rel| by_rel.remove(&rel))
        .collect()
}

/// True when `name` collides with a reserved picker id. Only WalkTo is
/// reserved: it is host nav, never a JS card. The abandoned rust-first
/// smokes (`BoneBurier` …) are free again — the shim catalog Loads them.
pub fn is_reserved(name: &str) -> bool {
    name == "WalkTo"
}

/// Catalog picker names that stay dim even if their imports later remap.
/// GatheringBot rows, quest-def cards, and MarketMaker.
pub const CATALOG_DIM: &[&str] = &[
    "AIOQuester",
    "Woodcutter",
    "Miner",
    "Fisher",
    "ArravSupplier",
    "Barcrawl",
    "RoguesPurse",
    "MarketMaker",
];

/// True when a **catalog** card of this register name must not Start.
pub fn is_catalog_dim(name: &str) -> bool {
    CATALOG_DIM.contains(&name)
}

#[cfg(feature = "load")]
pub(super) fn catalog_unloadable(
    name: &str,
    source: ScriptSource,
    origin_sha: &str,
    path: &Path,
    scanned: Option<String>,
) -> Option<String> {
    if source == ScriptSource::Catalog {
        if name == "BankSorter" {
            return Some(
                "dim: BankSorter is unavailable until native bank sorting is implemented".into(),
            );
        }
        if name == "EssMiner" {
            return Some(
                "dim: EssMiner is unavailable until the native Gatherer replaces it".into(),
            );
        }
        if name == "JiveKQ" {
            // The broker, the fixture and the script_jive_kq_four witness
            // stay for Beta 1; the live harness Starts it past this dim.
            return Some("dim: JiveKQ is unavailable: four-player qualification incomplete".into());
        }
        if name == "AIOQuester" {
            return Some("dim: AIOQuester is replaced by the native Quester card".into());
        }
        if is_catalog_dim(name) {
            return Some(format!("dim: {name}"));
        }
        if let Some(reason) = catalog_defect_reason(name, origin_sha, path) {
            return Some(reason.into());
        }
    }
    scanned
}

/// A diagnosed foreign-script defect applies only to the exact audited pair.
/// A changed card or helper is a different version, not a global name ban.
#[cfg(feature = "load")]
pub(super) fn catalog_defect_reason(
    name: &str,
    origin_sha: &str,
    path: &Path,
) -> Option<&'static str> {
    if name != "BrimhavenAgility"
        || origin_sha != "771daff07bd4b3d6f2826ab1300d4fd66bcbae0f9d7a76e4a2ad07a4d050e859"
    {
        return None;
    }
    let helper = path.parent()?.join("BrimhavenAgilityLogic.ts");
    let bytes = std::fs::read(helper).ok()?;
    (JsCache::origin_sha(&bytes)
        == "fedf5f8e05fd43642efb0370352e71e5feebf258a784bc503a2e145c33b46d4d")
        .then_some("dim: known script defect: retries a broken Brimhaven plank instead of an alternate route")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::{card_identity_id, file_identity};

    #[test]
    fn live_file_fixture_path_still_only_tradebot() {
        assert!(live_file_fixture_path("TradeBot").is_some());
        assert!(live_file_fixture_stem("TradeBot").is_some());
        assert!(live_file_fixture_path("bone_burier_v2").is_none());
        assert!(live_file_fixture_stem("bone_burier_v2").is_none());
        assert!(live_file_fixture_path("BoneBurier").is_none());
    }

    #[test]
    fn live_example_paths_are_exact_distinct_files() {
        let ts = live_example_path("bone_burier_v2.ts").expect("checked-in ts example");
        let js = live_example_path("bone_burier_v2.js").expect("checked-in js example");
        assert!(ts.ends_with("bone_burier_v2.ts"), "{}", ts.display());
        assert!(js.ends_with("bone_burier_v2.js"), "{}", js.display());
        assert_ne!(ts, js);
        assert_ne!(file_identity(&ts), file_identity(&js));
        assert_ne!(
            card_identity_id(ScriptSource::File, &ts, "bone_burier_v2"),
            card_identity_id(ScriptSource::File, &js, "bone_burier_v2")
        );
        assert!(live_example_path("bone_burier_v2").is_none());
        assert!(live_example_path("TradeBot").is_none());
        assert!(live_example_path("bone_burier.ts").is_none());
    }

    #[test]
    fn prayer_qualification_examples_are_exact_file_cards() {
        let v2 = live_example_path("prayer_v2.ts").expect("checked-in prayer v2 example");
        let v1 = live_example_path("prayer_v1.ts").expect("checked-in prayer v1 adapter");
        assert!(v2.ends_with("prayer_v2.ts"), "{}", v2.display());
        assert!(v1.ends_with("prayer_v1.ts"), "{}", v1.display());
        assert_ne!(file_identity(&v2), file_identity(&v1));
        assert!(live_example_path("prayer_v2").is_none());
        assert!(live_example_path("prayer_v1").is_none());
        assert!(live_example_path("Prayer.ts").is_none());
    }

    #[test]
    fn line_of_sight_qualification_example_is_an_exact_file_card() {
        let v2 = live_example_path("line_of_sight_v2.ts").expect("checked-in los v2 example");
        assert!(v2.ends_with("line_of_sight_v2.ts"), "{}", v2.display());
        assert!(live_example_path("line_of_sight_v2").is_none());
        assert!(live_example_path("LineOfSight.ts").is_none());
    }

    #[test]
    fn actor_observation_qualification_example_is_an_exact_file_card() {
        let v2 = live_example_path("actor_observation_v2.ts")
            .expect("checked-in actor observation v2 example");
        assert!(v2.ends_with("actor_observation_v2.ts"), "{}", v2.display());
        assert!(live_example_path("actor_observation_v2").is_none());
        assert!(live_example_path("ActorObservation.ts").is_none());
    }

    #[test]
    fn fight_field_qualification_example_is_an_exact_file_card() {
        let v2 = live_example_path("fight_field_v2.ts").expect("checked-in fight field v2 example");
        assert!(v2.ends_with("fight_field_v2.ts"), "{}", v2.display());
        assert!(live_example_path("fight_field_v2").is_none());
        assert!(live_example_path("FightField.ts").is_none());
    }

    #[test]
    fn hold_spot_qualification_example_is_an_exact_file_card() {
        let v2 = live_example_path("hold_spot_v2.ts").expect("checked-in hold spot v2 example");
        assert!(v2.ends_with("hold_spot_v2.ts"), "{}", v2.display());
        assert!(live_example_path("hold_spot_v2").is_none());
        assert!(live_example_path("HoldSpot.ts").is_none());
    }

    #[test]
    fn retreat_spot_qualification_example_is_an_exact_file_card() {
        let v2 =
            live_example_path("retreat_spot_v2.ts").expect("checked-in retreat spot v2 example");
        assert!(v2.ends_with("retreat_spot_v2.ts"), "{}", v2.display());
        assert!(live_example_path("retreat_spot_v2").is_none());
        assert!(live_example_path("RetreatSpot.ts").is_none());
    }

    #[test]
    fn walk_spot_qualification_example_is_an_exact_file_card() {
        let v2 = live_example_path("walk_spot_v2.ts").expect("checked-in walk spot v2 example");
        assert!(v2.ends_with("walk_spot_v2.ts"), "{}", v2.display());
        assert!(live_example_path("walk_spot_v2").is_none());
        assert!(live_example_path("WalkSpot.ts").is_none());
    }

    #[test]
    fn enter_lair_qualification_example_is_an_exact_file_card() {
        let v2 = live_example_path("enter_lair_v2.ts").expect("checked-in enter lair v2 example");
        assert!(v2.ends_with("enter_lair_v2.ts"), "{}", v2.display());
        assert!(live_example_path("enter_lair_v2").is_none());
        assert!(live_example_path("EnterLair.ts").is_none());
    }

    #[test]
    fn leave_lair_qualification_example_is_an_exact_file_card() {
        let v2 = live_example_path("leave_lair_v2.ts").expect("checked-in leave lair v2 example");
        assert!(v2.ends_with("leave_lair_v2.ts"), "{}", v2.display());
        assert!(live_example_path("leave_lair_v2").is_none());
        assert!(live_example_path("LeaveLair.ts").is_none());
    }

    #[test]
    fn acquire_key_qualification_example_is_an_exact_file_card() {
        let v2 = live_example_path("acquire_key_v2.ts").expect("checked-in acquire key v2 example");
        assert!(v2.ends_with("acquire_key_v2.ts"), "{}", v2.display());
        assert!(live_example_path("acquire_key_v2").is_none());
        assert!(live_example_path("AcquireKey.ts").is_none());
    }

    #[test]
    fn cell_qualification_example_is_an_exact_file_card() {
        let v2 = live_example_path("cell_v2.ts").expect("checked-in cell v2 example");
        assert!(v2.ends_with("cell_v2.ts"), "{}", v2.display());
        assert!(live_example_path("cell_v2").is_none());
        assert!(live_example_path("Cell.ts").is_none());
    }

    #[test]
    fn bank_qualification_example_is_an_exact_file_card() {
        let v2 = live_example_path("bank_v2.ts").expect("checked-in bank v2 example");
        assert!(v2.ends_with("bank_v2.ts"), "{}", v2.display());
        assert!(live_example_path("bank_v2").is_none());
        assert!(live_example_path("Bank.ts").is_none());
    }
}
