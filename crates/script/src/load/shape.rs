//! Shape detection, import scans, fingerprints, and picker selection helpers.
//!
//! V8-free except `transpile_ts` (feature `load`).

#[cfg(feature = "load")]
use std::collections::HashMap;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

#[cfg(feature = "load")]
use crate::js_cache::{CacheMeta, JsCache};
use crate::rs2b0t_registry::ScriptSource;

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

/// Every `from '…'` / `from "…"` specifier, first-seen order.
/// Template fragments inside log strings (`from '${who}'`) are not imports.
pub fn scan_import_specifiers(source: &str) -> Vec<String> {
    let mut out = Vec::new();
    for quote in ['\'', '"'] {
        let needle = format!("from {quote}");
        let mut rest = source;
        while let Some(idx) = rest.find(&needle) {
            let after = &rest[idx + needle.len()..];
            if let Some(end) = after.find(quote) {
                let spec = after[..end].trim();
                if looks_like_module_specifier(spec) && !out.iter().any(|x| x == spec) {
                    out.push(spec.to_string());
                }
            }
            rest = &rest[idx + 1..];
        }
    }
    out
}

fn looks_like_module_specifier(spec: &str) -> bool {
    !spec.is_empty()
        && !spec.contains('$')
        && !spec.contains('{')
        && (spec.starts_with('.')
            || spec.starts_with('#')
            || spec.starts_with('@')
            || spec.starts_with('/'))
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
    let card_dir = card_path
        .parent()
        .ok_or_else(|| format!("no parent dir for {}", card_path.display()))?;
    let mut pending: Vec<String> = scan_same_folder_js_imports(origin)
        .into_iter()
        .chain(scan_scripts_sibling_js_imports(origin))
        .collect();
    let mut seen = HashSet::new();
    let mut nodes: Vec<(String, String, String, String)> = Vec::new();
    while let Some(import_rel) = pending.pop() {
        if !seen.insert(import_rel.clone()) {
            continue;
        }
        let Some(url) = sibling_module_url(&import_rel) else {
            continue;
        };
        let Some(path) = resolve_sibling_path(card_dir, &import_rel) else {
            continue;
        };
        if same_path(&path, card_path) {
            continue;
        }
        let bytes = std::fs::read(&path).map_err(|e| format!("sibling {}: {e}", path.display()))?;
        let origin_text = String::from_utf8_lossy(&bytes).into_owned();
        pending.extend(scan_same_folder_js_imports(&origin_text));
        pending.extend(scan_scripts_sibling_js_imports(&origin_text));
        let cached = cache.get_or_transpile(&path, &bytes, meta.clone())?;
        nodes.push((import_rel, url, cached.js, origin_text));
    }
    Ok(order_siblings_deps_first(nodes))
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
    for (rel, _, _, origin) in &nodes {
        let mut needed = Vec::new();
        for imp in scan_same_folder_js_imports(origin)
            .into_iter()
            .chain(scan_scripts_sibling_js_imports(origin))
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
/// GatheringBot rows, quest-def cards, ClueSolver, MarketMaker.
pub const CATALOG_DIM: &[&str] = &[
    "AIOQuester",
    "ClueSolver",
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
pub(super) fn catalog_defect_reason(name: &str, origin_sha: &str, path: &Path) -> Option<&'static str> {
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

/// A picker selection: a compiled id or a loaded JS card by `(source, name)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScriptSel {
    Compiled(crate::registry::CompiledId),
    Loaded(ScriptSource, String),
}

impl ScriptSel {
    /// The label the picker shows and Start keys on.
    pub fn label(&self) -> String {
        match self {
            ScriptSel::Compiled(id) => id.0.to_string(),
            ScriptSel::Loaded(_, name) => name.clone(),
        }
    }
}
