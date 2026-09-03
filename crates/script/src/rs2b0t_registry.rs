//! `$RS2B0T` registry parse: static scan of `src/bot/scripts/index.ts`.
//! The catalog is never executed here — no rustyscript, no V8 Runtime, no
//! isolate. Register names become the picker names (they may differ from
//! the folder); the matched `import … from './…'` path is the file to read
//! on Start. The first successful parse persists the root so later boots
//! can find the catalog without `$RS2B0T` set.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

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

/// The registry entry file below a root: `src/bot/scripts/index.ts`.
pub fn registry_index_path(root: &Path) -> PathBuf {
    root.join("src/bot/scripts/index.ts")
}

/// The on-disk file for a card's `./…` import path. rs2b0t imports end in
/// `.js` while the files on disk are `.ts`, so the `.ts` twin wins when
/// the verbatim path is absent. Returns `None` when `rel_path` is absolute,
/// contains `..`, or would resolve outside `root/src/bot/scripts`.
pub fn script_file_path(root: &Path, rel_path: &str) -> Option<PathBuf> {
    let base = root.join("src/bot/scripts");
    let resolved = resolve_under_catalog(&base, rel_path)?;

    let verbatim = base.join(strip_leading_dot_slash(rel_path));
    let candidate = if verbatim.is_file() {
        verbatim
    } else if let Some(stem) = rel_path.strip_suffix(".js") {
        let ts = base.join(format!("{}.ts", strip_leading_dot_slash(stem)));
        if ts.is_file() {
            ts
        } else {
            resolved
        }
    } else {
        resolved
    };

    canonical_under(&base, &candidate)
}

/// Join `rel_path` under `catalog` without touching the filesystem. Rejects
/// absolute paths, `..`, and anything not starting with `./`.
fn resolve_under_catalog(catalog: &Path, rel_path: &str) -> Option<PathBuf> {
    if rel_path.starts_with('/') || rel_path.starts_with('\\') {
        return None;
    }
    let rel = rel_path.strip_prefix("./")?;
    let mut out = catalog.to_path_buf();
    for component in Path::new(rel).components() {
        match component {
            std::path::Component::Normal(part) => out.push(part),
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir
            | std::path::Component::RootDir
            | std::path::Component::Prefix(_) => return None,
        }
    }
    Some(out)
}

fn strip_leading_dot_slash(rel_path: &str) -> &str {
    rel_path.strip_prefix("./").unwrap_or(rel_path)
}

/// Canonicalize `path` when possible and require it stay under `catalog`.
fn canonical_under(catalog: &Path, path: &Path) -> Option<PathBuf> {
    if !path.starts_with(catalog) {
        return None;
    }
    if let Ok(canon_catalog) = catalog.canonicalize() {
        if let Ok(canon_path) = path.canonicalize() {
            if !canon_path.starts_with(&canon_catalog) {
                return None;
            }
            return Some(canon_path);
        }
    }
    Some(path.to_path_buf())
}

const RS2B0T_IMPORT_DEFERRED: &str = "deferred";

/// Default first-run defer flag file (`~/.274bot/rs2b0t-import`).
pub fn default_rs2b0t_import_file() -> PathBuf {
    crate::bot_file("rs2b0t-import")
}

/// Whether the operator chose **Not now** on the first Browse catalog prompt.
pub fn rs2b0t_import_deferred_at(path: &Path) -> bool {
    std::fs::read_to_string(path)
        .map(|s| s.trim() == RS2B0T_IMPORT_DEFERRED)
        .unwrap_or(false)
}

/// [`rs2b0t_import_deferred_at`] against the default import file.
pub fn rs2b0t_import_deferred() -> bool {
    rs2b0t_import_deferred_at(&default_rs2b0t_import_file())
}

/// Record that the operator deferred the rs2b0t catalog import.
pub fn set_rs2b0t_import_deferred_at(path: &Path) -> Result<(), String> {
    vault::write_private_file(path, RS2B0T_IMPORT_DEFERRED.as_bytes())
        .map_err(|e| format!("rs2b0t-import: {e}"))
}

/// Clear the defer flag after a successful import.
pub fn clear_rs2b0t_import_at(path: &Path) -> Result<(), String> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("rs2b0t-import: {e}")),
    }
}

/// Default persisted rs2b0t root file (`~/.274bot/rs2b0t-path`).
pub fn default_rs2b0t_path_file() -> PathBuf {
    crate::bot_file("rs2b0t-path")
}

/// The rs2b0t checkout root: `$RS2B0T` first, else the path persisted by a
/// previous successful parse.
pub fn rs2b0t_root_at(path_file: &Path) -> Option<PathBuf> {
    if let Some(root) = crate::rs2b0t_env() {
        if !root.as_os_str().is_empty() {
            return Some(root);
        }
    }
    let persisted = std::fs::read_to_string(path_file).ok()?;
    let root = persisted.trim();
    if root.is_empty() {
        return None;
    }
    Some(PathBuf::from(root))
}

/// [`rs2b0t_root_at`] against the default persisted file.
pub fn rs2b0t_root() -> Option<PathBuf> {
    rs2b0t_root_at(&default_rs2b0t_path_file())
}

/// Persist `root` to `path_file` (a previous successful parse recorded the
/// checkout path). Writes only when the file would change.
pub fn persist_rs2b0t_root_at(root: &Path, path_file: &Path) -> Result<(), String> {
    if let Ok(existing) = std::fs::read_to_string(path_file) {
        if existing.trim() == root.to_string_lossy() {
            return Ok(());
        }
    }
    vault::write_private_file(path_file, root.to_string_lossy().as_bytes())
        .map_err(|e| format!("rs2b0t-path: {e}"))
}

/// [`persist_rs2b0t_root_at`] against the default persisted file.
pub fn persist_rs2b0t_root(root: &Path) -> Result<(), String> {
    persist_rs2b0t_root_at(root, &default_rs2b0t_path_file())
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
                let src = lookup_source(sources, &binding.rel_path)?;
                Some(parse_settings_export(&src, &binding.export_name))
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

/// `ident -> import binding` for every default and named import of a relative
/// module under `./`. Parent-dir and absolute imports are ignored.
fn scan_imports(src: &str) -> HashMap<String, ImportBinding> {
    let mut imports = HashMap::new();
    let mut pos = 0;
    while let Some(rel) = src[pos..].find("import") {
        let start = pos + rel;
        let before = src[..start].chars().next_back();
        if !matches!(
            before,
            None | Some('\n') | Some('\r') | Some(' ') | Some('\t') | Some(';') | Some('}')
        ) {
            pos = start + 1;
            continue;
        }
        let tail = &src[start + "import".len()..];
        let stmt_len = tail.find(';').unwrap_or(tail.len());
        if let Some((bindings, path)) = parse_import_stmt(&tail[..stmt_len]) {
            for (local, export_name) in bindings {
                imports.entry(local).or_insert_with(|| ImportBinding {
                    rel_path: path.clone(),
                    export_name,
                });
            }
        }
        pos = start + "import".len() + stmt_len;
    }
    imports
}

/// Parse one import statement body (the text after `import`, before `;`).
/// Returns local-name/export-name pairs and the module path.
fn parse_import_stmt(stmt: &str) -> Option<(Vec<(String, String)>, String)> {
    let stmt = stmt.trim_start();
    let stmt = stmt.strip_prefix("type ").unwrap_or(stmt);
    let fi = stmt.rfind("from ")?;
    let after = stmt[fi + "from ".len()..].trim_start();
    let path = quoted_after(after)?;
    if !path.starts_with("./") {
        return None;
    }
    let spec = stmt[..fi].trim();
    let mut bindings = Vec::new();
    if let Some(rest) = spec.strip_prefix('{') {
        let inner = rest.strip_suffix('}').unwrap_or(rest);
        for part in inner.split(',') {
            parse_named_import_part(part, &mut bindings);
        }
    } else {
        let group = spec.find('{');
        let head = group.map_or(spec, |i| &spec[..i]);
        if let Some(ident) = head
            .split(|c: char| c.is_whitespace() || c == ',')
            .find(|t| !t.is_empty())
        {
            bindings.push((ident.to_string(), ident.to_string()));
        }
        if let Some(i) = group {
            if let Some(j) = spec.rfind('}') {
                for part in spec[i + 1..j].split(',') {
                    parse_named_import_part(part, &mut bindings);
                }
            }
        }
    }
    if bindings.is_empty() {
        return None;
    }
    Some((bindings, path))
}

fn parse_named_import_part(part: &str, bindings: &mut Vec<(String, String)>) {
    let part = part.trim();
    if part.is_empty() {
        return;
    }
    if let Some((export_name, local)) = part.split_once(" as ") {
        let export_name = export_name.trim();
        let local = local.trim();
        if !local.is_empty() {
            bindings.push((
                local.to_string(),
                if export_name.is_empty() {
                    local.to_string()
                } else {
                    export_name.to_string()
                },
            ));
        }
    } else {
        let ident = part.split_whitespace().next().unwrap_or(part).to_string();
        bindings.push((ident.clone(), ident));
    }
}

/// The first `'…'` or `"…"` string in `s`.
fn quoted_after(s: &str) -> Option<String> {
    let s = s.trim_start();
    let q = s.chars().next()?;
    if q != '\'' && q != '"' {
        return None;
    }
    for (i, c) in s[1..].char_indices() {
        if c == q {
            return Some(s[1..1 + i].to_string());
        }
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
            break;
        }
        let Some(value) = quoted_after(rest) else {
            break;
        };
        out.push(value);
        let close_idx = rest[1..].find(q)?;
        rest = &rest[1 + close_idx + 1..];
        rest = rest.trim_start();
        if rest.starts_with(',') {
            rest = &rest[1..];
        } else {
            break;
        }
    }
    Some(out)
}

fn find_matching_bracket(s: &str, open: char, close: char) -> Option<usize> {
    let mut depth = 0i32;
    let mut in_str: Option<char> = None;
    for (i, c) in s.char_indices() {
        if let Some(q) = in_str {
            if c == '\\' {
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

/// File-Load SETTINGS: static `export const SETTINGS = { … }` parse.
/// No V8. Identifier-valued fields that the scanner cannot inline stay
/// empty rather than aborting the walk.
pub fn settings_schema_from_source(src: &str) -> Vec<SettingDef> {
    parse_settings_export(src, "SETTINGS")
}

/// Parse `export const NAME = { … }` into setting definitions.
fn parse_settings_export(file_src: &str, export_name: &str) -> Vec<SettingDef> {
    let needle = format!("export const {export_name}");
    let Some(idx) = file_src.find(&needle) else {
        return Vec::new();
    };
    let after = &file_src[idx + needle.len()..];
    let after = after.trim_start();
    let after = match after.strip_prefix('=') {
        Some(a) => a.trim_start(),
        None => return Vec::new(),
    };
    let Some(obj_end) = find_matching_bracket(after, '{', '}') else {
        return Vec::new();
    };
    parse_settings_object(&after[..=obj_end], file_src)
}

fn setting_object_body(file_src: &str, export_name: &str) -> Option<String> {
    let needle = format!("export const {export_name}");
    let idx = file_src.find(&needle)?;
    let after = file_src[idx + needle.len()..].trim_start();
    let after = after.strip_prefix('=')?.trim_start();
    let obj_end = find_matching_bracket(after, '{', '}')?;
    Some(after[..=obj_end].to_string())
}

fn take_ident(s: &str) -> Option<(&str, &str)> {
    let n = s
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '$')
        .count();
    if n == 0 {
        None
    } else {
        Some((&s[..n], &s[n..]))
    }
}

/// Name-map only: inlined `export const` in the same file, or the host's
/// `LOADOUT_SETTING` shim. Do not evaluate TypeScript identifiers.
fn resolve_setting_ident(file_src: &str, ident: &str) -> Option<String> {
    if ident == "LOADOUT_SETTING" {
        return setting_object_body(
            include_str!("shim/loadout_setting.js"),
            "LOADOUT_SETTING",
        );
    }
    setting_object_body(file_src, ident)
}

fn parse_settings_object(obj: &str, file_src: &str) -> Vec<SettingDef> {
    let inner = obj.trim();
    let inner = inner.strip_prefix('{').unwrap_or(inner);
    let inner = inner.strip_suffix('}').unwrap_or(inner);
    let mut out = Vec::new();
    let mut rest = inner;
    while !rest.trim().is_empty() {
        rest = rest.trim_start();
        if rest.starts_with('}') || rest.is_empty() {
            break;
        }
        let Some(colon) = rest.find(':') else {
            break;
        };
        let id = rest[..colon]
            .trim()
            .trim_matches(|c: char| !c.is_alphanumeric() && c != '_' && c != '$')
            .to_string();
        if id.is_empty() {
            break;
        }
        let after_colon = rest[colon + 1..].trim_start();
        if after_colon.starts_with('{') {
            let Some(end) = find_matching_bracket(after_colon, '{', '}') else {
                break;
            };
            out.push(parse_setting_def(&id, &after_colon[..=end]));
            rest = &after_colon[end + 1..];
        } else if let Some((ident, after_ident)) = take_ident(after_colon) {
            if let Some(body) = resolve_setting_ident(file_src, ident) {
                out.push(parse_setting_def(&id, &body));
            }
            rest = after_ident;
        } else {
            break;
        }
        rest = rest.trim_start();
        if rest.starts_with(',') {
            rest = &rest[1..];
        }
    }
    out
}

fn parse_setting_def(id: &str, obj: &str) -> SettingDef {
    SettingDef {
        id: id.to_string(),
        ty: scan_key_quoted(obj, "type").unwrap_or_default(),
        default: scan_key_literal(obj, "default"),
        label: scan_key_quoted(obj, "label"),
        min: scan_key_number(obj, "min"),
        max: scan_key_number(obj, "max"),
        step: scan_key_number(obj, "step"),
        options: scan_key_options(obj, "options"),
        option_labels: scan_key_string_array(obj, "optionLabels").unwrap_or_default(),
        group: scan_key_quoted(obj, "group"),
        show_if: scan_key_raw_value(obj, "showIf"),
        options_from: scan_key_quoted(obj, "optionsFrom")
            .or_else(|| scan_key_ident(obj, "optionsFrom")),
        csv_toggle: scan_key_raw_value(obj, "csvToggle"),
        help: scan_key_quoted(obj, "help"),
    }
}

fn scan_key_literal(block: &str, key: &str) -> Option<String> {
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
        if after.starts_with("true") {
            return Some("true".to_string());
        }
        if after.starts_with("false") {
            return Some("false".to_string());
        }
        let num: String = after
            .chars()
            .take_while(|c| c.is_ascii_digit() || *c == '.' || *c == '-')
            .collect();
        if !num.is_empty() {
            return Some(num);
        }
        rest = after;
    }
    None
}

fn scan_key_number(block: &str, key: &str) -> Option<String> {
    scan_key_literal(block, key)
}

fn scan_key_options(block: &str, key: &str) -> Vec<String> {
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
            return arr;
        }
        // Identifier reference — do not evaluate TypeScript.
        return Vec::new();
    }
    Vec::new()
}

fn scan_key_raw_value(block: &str, key: &str) -> Option<String> {
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
        if after.starts_with('{') {
            if let Some(end) = find_matching_bracket(after, '{', '}') {
                return Some(after[..=end].to_string());
            }
        }
        if after.starts_with('[') {
            if let Some(end) = find_matching_bracket(after, '[', ']') {
                return Some(after[..=end].to_string());
            }
        }
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
