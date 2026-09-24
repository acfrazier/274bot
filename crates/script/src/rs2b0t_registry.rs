//! `$RS2B0T` registry parse: static scan of `src/bot/scripts/index.ts`.
//! The catalog is never executed here — no rustyscript, no V8 Runtime, no
//! isolate. Register names become the picker names (they may differ from
//! the folder); the matched `import … from './…'` path is the file to read
//! on Start. The first successful parse persists the root so later boots
//! can find the catalog without `$RS2B0T` set.

use std::collections::HashMap;

#[cfg(test)]
use std::path::PathBuf;

mod paths;
mod settings;

pub use paths::{
    clear_rs2b0t_import_at, default_rs2b0t_import_file, default_rs2b0t_path_file,
    persist_rs2b0t_root, persist_rs2b0t_root_at, registry_index_path, rs2b0t_import_deferred,
    rs2b0t_import_deferred_at, rs2b0t_root, rs2b0t_root_at, script_file_path,
    set_rs2b0t_import_deferred_at,
};
#[cfg(test)]
use settings::catalog_option_values;
use settings::parse_settings_export;
pub use settings::settings_schema_from_source;
pub(crate) use settings::{
    catalog_option_table, insert_shop_db_from_root, is_revision_fact_option_ident,
    w1c_equipment_option_families,
};

/// How a script card is executed in the host.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScriptKind {
    Compat,
    NativeTick,
    Compiled,
}

/// Where a script card's metadata came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScriptSource {
    Catalog,
    File,
    Builtin,
}

/// One catalog-declared selected-item candidate. `key` is the persisted
/// alias; `label` is an optional identity override from the source table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemOptionCandidate {
    pub key: String,
    pub label: Option<String>,
}

/// High-alchemy selected-item choice descriptor. Prefix keys are always
/// published; candidates are filtered/sorted later from borrowed facts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemOptionSpec {
    pub prefix: Vec<String>,
    pub candidates: Vec<ItemOptionCandidate>,
    /// When true, resolved chip keys follow alphabetical label order (frozen
    /// `[...ALCH_ITEMS].sort((a,b)=>a.label.localeCompare(b.label)).map`).
    pub sort_keys_by_label: bool,
}

/// One setting field from a script's `settingsSchema` export.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingDef {
    pub id: String,
    pub ty: String,
    pub default: Option<String>,
    pub label: Option<String>,
    pub min: Option<String>,
    pub max: Option<String>,
    pub step: Option<String>,
    pub options: Vec<String>,
    pub option_labels: Vec<String>,
    pub group: Option<String>,
    pub show_if: Option<String>,
    pub options_from: Option<String>,
    pub csv_toggle: Option<String>,
    pub help: Option<String>,
    pub item_option_spec: Option<ItemOptionSpec>,
}

/// One catalog card: register metadata plus the `./…` import path of the
/// script file to read on Start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegistryCard {
    pub name: String,
    pub rel_path: String,
    pub description: String,
    pub category: String,
    pub tags: Vec<String>,
    pub version: String,
    pub settings_schema: Vec<SettingDef>,
    pub kind: ScriptKind,
    pub source: ScriptSource,
}

/// Named import binding: local alias maps to module path and exported const name.
#[derive(Debug, Clone)]
struct ImportBinding {
    rel_path: String,
    export_name: String,
}

/// Parsed register block fields from `ScriptRegistry.register({…})`.
#[derive(Debug, Clone)]
struct RegisterInfo {
    name: String,
    ctor: String,
    description: String,
    category: String,
    tags: Vec<String>,
    version: String,
    settings_schema_ident: Option<String>,
}

/// Static parse of the catalog `index.ts` without reading sibling script files.
pub fn parse_registry(index_ts: &str) -> Result<Vec<RegistryCard>, String> {
    parse_registry_with_sources(index_ts, &HashMap::new())
}

/// Static parse of the catalog `index.ts`. Each
/// `ScriptRegistry.register({…name: '…'…})` is matched to its
/// `import … from './…'` by the `create: () => new X()` constructor
/// ident. When `sources` maps a `./…` import path to file text, `settingsSchema`
/// is resolved through named imports (including `as` aliases). No JS runs.
pub fn parse_registry_with_sources(
    index_ts: &str,
    sources: &HashMap<String, String>,
) -> Result<Vec<RegistryCard>, String> {
    let imports = scan_imports(index_ts);
    let registers = scan_registers(index_ts);
    if registers.is_empty() {
        return Err("no ScriptRegistry.register calls".to_string());
    }
    let mut cards = Vec::new();
    for reg in registers {
        let Some(path) = imports.get(&reg.ctor).map(|b| b.rel_path.clone()) else {
            continue;
        };
        let settings_schema = reg
            .settings_schema_ident
            .as_ref()
            .and_then(|ident| {
                let binding = imports.get(ident)?;
                let src = settings_blob(sources, &binding.rel_path)?;
                Some(parse_settings_export(
                    &src,
                    &binding.export_name,
                    Some(sources),
                ))
            })
            .unwrap_or_default();
        cards.push(RegistryCard {
            name: reg.name,
            rel_path: path,
            description: reg.description,
            category: reg.category,
            tags: reg.tags,
            version: reg.version,
            settings_schema,
            kind: ScriptKind::Compat,
            source: ScriptSource::Catalog,
        });
    }
    if cards.is_empty() {
        return Err("register matched no import".to_string());
    }
    Ok(cards)
}

fn lookup_source(sources: &HashMap<String, String>, rel_path: &str) -> Option<String> {
    if let Some(src) = sources.get(rel_path) {
        return Some(src.clone());
    }
    if let Some(stem) = rel_path.strip_suffix(".js") {
        let ts_path = format!("{stem}.ts");
        if let Some(src) = sources.get(&ts_path) {
            return Some(src.clone());
        }
    }
    None
}

fn rel_dir(rel: &str) -> Option<&str> {
    let rel = rel.strip_prefix("./").unwrap_or(rel);
    rel.rsplit_once('/').map(|(d, _)| d)
}

/// Script text plus same-directory siblings (BankFletcherLogic, AlcherLogic).
fn settings_blob(sources: &HashMap<String, String>, rel_path: &str) -> Option<String> {
    let primary = lookup_source(sources, rel_path)?;
    let Some(dir) = rel_dir(rel_path) else {
        return Some(primary);
    };
    let mut blob = primary;
    for (path, text) in sources {
        if path == rel_path {
            continue;
        }
        if rel_dir(path) == Some(dir) {
            blob.push('\n');
            blob.push_str(text);
        }
    }
    Some(blob)
}

/// `import … from './Logic.js'` next to `./Folder/Script.js` → `./Folder/Logic.js`.
pub(crate) fn same_dir_import_rel(script_rel: &str, import_rel: &str) -> Option<String> {
    let import = import_rel.strip_prefix("./")?;
    if import.contains('/') || import.contains('\\') || import.contains("..") {
        return None;
    }
    let dir = rel_dir(script_rel)?;
    Some(format!("./{dir}/{import}"))
}

/// `ident -> import binding` for every default and named import of a relative
/// module under `./`. Parent-dir and absolute imports are ignored.
fn scan_imports(src: &str) -> HashMap<String, ImportBinding> {
    let mut imports = HashMap::new();
    for import in crate::module_imports::module_imports(src) {
        if !import.specifier.starts_with("./") {
            continue;
        }
        for (local, export_name) in import.bindings {
            imports.entry(local).or_insert_with(|| ImportBinding {
                rel_path: import.specifier.clone(),
                export_name,
            });
        }
    }
    imports
}

pub(crate) fn same_dir_import_rels(script_rel: &str, src: &str) -> Vec<String> {
    let mut out = Vec::new();
    for binding in scan_imports(src).into_values() {
        if let Some(sib) = same_dir_import_rel(script_rel, &binding.rel_path) {
            if !out.contains(&sib) {
                out.push(sib);
            }
        }
    }
    out
}

/// The first `'…'` or `"…"` string in `s`, unescaping `\'` `\"` `\\`.
fn quoted_after(s: &str) -> Option<String> {
    scan_quoted(s).map(|(value, _)| value)
}

/// Quoted literal at the start of `s` (after leading whitespace).
/// Returns unescaped content and the byte length consumed from `s`.
fn scan_quoted(s: &str) -> Option<(String, usize)> {
    let trimmed = s.trim_start();
    let pad = s.len() - trimmed.len();
    let q = trimmed.chars().next()?;
    if q != '\'' && q != '"' {
        return None;
    }
    let q_len = q.len_utf8();
    let mut out = String::new();
    let mut chars = trimmed[q_len..].char_indices();
    while let Some((rel, c)) = chars.next() {
        if c == '\\' {
            let (_, next) = chars.next()?;
            out.push(match next {
                'n' => '\n',
                't' => '\t',
                'r' => '\r',
                other => other,
            });
            continue;
        }
        if c == q {
            let consumed = pad + q_len + rel + q_len;
            return Some((out, consumed));
        }
        out.push(c);
    }
    None
}

/// Every register block in file order.
fn scan_registers(src: &str) -> Vec<RegisterInfo> {
    let mut out = Vec::new();
    let mut pos = 0;
    while let Some(rel) = src[pos..].find("ScriptRegistry.register(") {
        let open = pos + rel + "ScriptRegistry.register(".len();
        let Some(block) = extract_braced_block(src, open) else {
            pos = open;
            continue;
        };
        let block_end = open + block.len();
        out.push(RegisterInfo {
            name: scan_key_quoted(&block, "name").unwrap_or_default(),
            ctor: scan_new_after(&block, "create").unwrap_or_default(),
            description: scan_key_quoted(&block, "description").unwrap_or_default(),
            category: scan_key_quoted(&block, "category").unwrap_or_default(),
            tags: scan_key_string_array(&block, "tags").unwrap_or_default(),
            version: scan_key_quoted(&block, "version").unwrap_or_default(),
            settings_schema_ident: scan_key_ident(&block, "settingsSchema"),
        });
        pos = block_end + 1;
    }
    out
}

fn extract_braced_block(src: &str, open: usize) -> Option<String> {
    let mut i = open;
    while i < src.len() && src[i..].chars().next().unwrap().is_whitespace() {
        i += src[i..].chars().next().unwrap().len_utf8();
    }
    if i >= src.len() || !src[i..].starts_with('{') {
        return None;
    }
    let mut depth = 0u32;
    let mut j = i;
    let mut in_str: Option<char> = None;
    while j < src.len() {
        let c = src[j..].chars().next().unwrap();
        if let Some(q) = in_str {
            if c == '\\' {
                j += c.len_utf8();
                if j < src.len() {
                    j += src[j..].chars().next().unwrap().len_utf8();
                }
                continue;
            }
            if c == q {
                in_str = None;
            }
        } else if c == '\'' || c == '"' {
            in_str = Some(c);
        } else if c == '{' {
            depth += 1;
        } else if c == '}' {
            depth -= 1;
            if depth == 0 {
                return Some(src[i..=j].to_string());
            }
        }
        j += c.len_utf8();
    }
    None
}

/// The quoted string following the first `key:` in `block`.
fn scan_key_quoted(block: &str, key: &str) -> Option<String> {
    let mut rest = block;
    while let Some(idx) = rest.find(key) {
        let after = &rest[idx + key.len()..];
        let after = after.trim_start();
        let after = match after.strip_prefix(':') {
            Some(a) => a.trim_start(),
            None => {
                rest = after;
                continue;
            }
        };
        if let Some(v) = quoted_after(after) {
            return Some(v);
        }
        rest = after;
    }
    None
}

/// Identifier following `key:` (e.g. `settingsSchema: CHICKEN_SETTINGS`).
fn scan_key_ident(block: &str, key: &str) -> Option<String> {
    let mut rest = block;
    while let Some(idx) = rest.find(key) {
        let after = &rest[idx + key.len()..];
        let after = after.trim_start();
        let after = match after.strip_prefix(':') {
            Some(a) => a.trim_start(),
            None => {
                rest = after;
                continue;
            }
        };
        let ident = after
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '$')
            .collect::<String>();
        if !ident.is_empty() {
            return Some(ident);
        }
        rest = after;
    }
    None
}

/// String array following `key:` (e.g. `tags: ['combat', 'money']`).
fn scan_key_string_array(block: &str, key: &str) -> Option<Vec<String>> {
    let mut rest = block;
    while let Some(idx) = rest.find(key) {
        let after = &rest[idx + key.len()..];
        let after = after.trim_start();
        let after = match after.strip_prefix(':') {
            Some(a) => a.trim_start(),
            None => {
                rest = after;
                continue;
            }
        };
        if let Some(arr) = parse_string_array(after) {
            return Some(arr);
        }
        rest = after;
    }
    None
}

fn parse_string_array(s: &str) -> Option<Vec<String>> {
    let s = s.trim_start();
    if !s.starts_with('[') {
        return None;
    }
    let end = find_matching_bracket(s, '[', ']')?;
    let inner = &s[1..end];
    let mut out = Vec::new();
    let mut rest = inner;
    loop {
        rest = rest.trim_start();
        if rest.is_empty() {
            break;
        }
        let q = rest.chars().next()?;
        if q != '\'' && q != '"' {
            // Mixed, computed, or unquoted remainder is not a literal string array.
            return None;
        }
        let (value, n) = scan_quoted(rest)?;
        out.push(value);
        rest = &rest[n..];
        rest = rest.trim_start();
        if rest.starts_with(',') {
            rest = &rest[1..];
        } else if rest.is_empty() {
            break;
        } else {
            return None;
        }
    }
    Some(out)
}

fn find_matching_bracket(s: &str, open: char, close: char) -> Option<usize> {
    let mut depth = 0i32;
    let mut in_str: Option<char> = None;
    let mut skip = false;
    for (i, c) in s.char_indices() {
        if skip {
            skip = false;
            continue;
        }
        if let Some(q) = in_str {
            if c == '\\' {
                skip = true;
                continue;
            }
            if c == q {
                in_str = None;
            }
            continue;
        }
        if c == '\'' || c == '"' {
            in_str = Some(c);
        } else if c == open {
            depth += 1;
        } else if c == close {
            depth -= 1;
            if depth == 0 {
                return Some(i);
            }
        }
    }
    None
}

/// The constructor ident in `create: () => new X()`.
fn scan_new_after(block: &str, key: &str) -> Option<String> {
    let mut rest = block;
    while let Some(idx) = rest.find(key) {
        let after = &rest[idx + key.len()..];
        let Some(arrow) = after.find("=>") else {
            rest = after;
            continue;
        };
        let after_arrow = after[arrow + 2..].trim_start();
        let after_new = match after_arrow.strip_prefix("new ") {
            Some(a) => a.trim_start(),
            None => {
                rest = after;
                continue;
            }
        };
        let ident = after_new
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '$')
            .collect::<String>();
        if !ident.is_empty() {
            return Some(ident);
        }
        rest = after;
    }
    None
}

#[cfg(test)]
#[path = "rs2b0t_registry_tests.rs"]
mod settings_extraction_tests;
