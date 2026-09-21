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
                let src = settings_blob(sources, &binding.rel_path)?;
                Some(parse_settings_export(&src, &binding.export_name, Some(sources)))
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

/// File-Load SETTINGS: static `export const SETTINGS = { … }` parse.
/// No V8. Identifier-valued fields that the scanner cannot inline stay
/// empty rather than aborting the walk.
pub fn settings_schema_from_source(src: &str) -> Vec<SettingDef> {
    parse_settings_export(src, "SETTINGS", None)
}

/// Parse `export const NAME = { … }` into setting definitions.
/// Catalog golds type the export (`export const SETTINGS: SettingsSchema = {`).
fn parse_settings_export(
    file_src: &str,
    export_name: &str,
    sources: Option<&HashMap<String, String>>,
) -> Vec<SettingDef> {
    let Some(obj) = settings_export_object(file_src, export_name) else {
        return Vec::new();
    };
    parse_settings_object(obj, file_src, sources)
}

fn setting_object_body(file_src: &str, export_name: &str) -> Option<String> {
    settings_export_object(file_src, export_name).map(str::to_string)
}

/// Body of `export const NAME[: Type] = { … }`, including the braces.
fn settings_export_object<'a>(file_src: &'a str, export_name: &str) -> Option<&'a str> {
    let needle = format!("export const {export_name}");
    let idx = file_src.find(&needle)?;
    let mut after = file_src[idx + needle.len()..].trim_start();
    if after.starts_with(':') {
        after = skip_ts_type_to_eq(after)?;
    }
    let after = after.strip_prefix('=')?.trim_start();
    let obj_end = find_matching_bracket(after, '{', '}')?;
    Some(&after[..=obj_end])
}

/// `s` starts with `:`. Return the suffix at the assignment `=`, or `None`.
fn skip_ts_type_to_eq(s: &str) -> Option<&str> {
    let mut angle = 0i32;
    let mut paren = 0i32;
    let mut square = 0i32;
    for (i, c) in s.char_indices() {
        match c {
            '<' => angle += 1,
            '>' => angle = angle.saturating_sub(1),
            '(' => paren += 1,
            ')' => paren = paren.saturating_sub(1),
            '[' => square += 1,
            ']' => square = square.saturating_sub(1),
            '=' if i > 0 && angle == 0 && paren == 0 && square == 0 => {
                return Some(&s[i..]);
            }
            _ => {}
        }
    }
    None
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
        return setting_object_body(include_str!("shim/loadout_setting.js"), "LOADOUT_SETTING");
    }
    if ident == "PERIODIC_BANK_SETTINGS" {
        return setting_object_body(include_str!("shim/banking.js"), "PERIODIC_BANK_SETTINGS");
    }
    setting_object_body(file_src, ident)
}

/// Frozen-source equipment / drop tables that W1 must publish. The host
/// must not invent names here; empty options are the honest schema.
pub(crate) fn is_revision_fact_option_ident(ident: &str) -> bool {
    matches!(
        ident,
        "STAFFS"
            | "BOWS"
            | "CROSSBOWS"
            | "DARTS"
            | "ARROWS"
            | "BOLTS"
            | "AXES"
            | "MELEE_WEAPONS"
            | "RANGED_WEAPONS"
            | "ROCK_CRAB_RANGED_WEAPONS"
            | "DROP_DB"
    )
}

/// W1c `equipment_names` families backing a frozen settings `options` /
/// `optionsFrom` ident. `RANGED_WEAPONS` and `ROCK_CRAB_RANGED_WEAPONS` mirror
/// `.superpowers/release-0.1.8/reference/rs2b0t-beecd9126b/src/bot/api/combat/ranged.ts`
/// (`[...BOWS, ...DARTS]`). `AXES` and `DROP_DB` stay outside W1c.
pub(crate) fn w1c_equipment_option_families(ident: &str) -> Option<&'static [&'static str]> {
    Some(match ident {
        "STAFFS" => &["staffs"],
        "BOWS" => &["bows"],
        "CROSSBOWS" => &["crossbows"],
        "DARTS" => &["darts"],
        "ARROWS" => &["arrows"],
        "BOLTS" => &["bolts"],
        "MELEE_WEAPONS" => &["melee_weapons"],
        "RANGED_WEAPONS" | "ROCK_CRAB_RANGED_WEAPONS" => &["bows", "darts"],
        _ => return None,
    })
}

/// Host-owned finite option tables for imported / parent-dir identifiers
/// whose bodies are not in the same-directory settings blob. Values are
/// copied from the frozen rs2b0t pin; this is not a JS evaluator.
pub(crate) fn catalog_option_table(ident: &str) -> Option<&'static [&'static str]> {
    Some(match ident {
        "COOK_LOCATION_OPTIONS" => &[
            "Auto",
            "Varrock East",
            "Varrock West",
            "Al Kharid",
            "Draynor",
            "Falador East",
            "Falador West",
            "Edgeville",
            "Seers",
            "Catherby",
            "Yanille",
            "Ardougne West",
            "Ardougne East",
            "Canifis",
            "Shilo Village",
            "Fishing Guild",
            "Shantay Pass",
            "Mage Arena",
            "Grand Tree",
            "Duel Arena",
            "Custom",
        ],
        "LOG_LEVELS" => &[
            "Logs",
            "Oak logs",
            "Willow logs",
            "Maple logs",
            "Yew logs",
            "Magic logs",
        ],
        "FIRE_SPOTS" => &["Varrock East", "Varrock West", "Draynor", "Seers"],
        "HERB_OPTIONS" => &[
            "Guam leaf",
            "Marrentill",
            "Tarromin",
            "Harralander",
            "Ranarr weed",
            "Toadflax",
            "Irit leaf",
            "Avantoe",
            "Kwuarm",
            "Snapdragon",
            "Cadantine",
            "Lantadyme",
            "Dwarf weed",
            "Torstol",
            "Snake weed",
            "Ardrigal",
            "Sito foil",
            "Volencia moss",
            "Rogues purse",
        ],
        "RUNE_OPTIONS" => &[
            "Air rune",
            "Mind rune",
            "Water rune",
            "Earth rune",
            "Fire rune",
            "Body rune",
            "Cosmic rune",
            "Chaos rune",
            "Nature rune",
            "Law rune",
            "Death rune",
        ],
        "BANK_LOCATIONS" | "BANK_LOCATION_OPTIONS" => bank_location_names(ident),
        "PRODUCT_OPTIONS" => &[
            "Gold ring",
            "Sapphire ring",
            "Emerald ring",
            "Ruby ring",
            "Diamond ring",
            "Dragonstone ring",
            "Gold necklace",
            "Sapphire necklace",
            "Emerald necklace",
            "Ruby necklace",
            "Diamond necklace",
            "Dragonstone necklace",
            "Gold amulet",
            "Sapphire amulet",
            "Emerald amulet",
            "Ruby amulet",
            "Diamond amulet",
            "Dragonstone amulet",
        ],
        "JEWEL_OPTIONS" => &[
            "Sapphire ring",
            "Sapphire necklace",
            "Sapphire amulet",
            "Emerald ring",
            "Emerald amulet",
            "Ruby ring",
            "Ruby amulet",
            "Diamond ring",
            "Diamond amulet",
            "Dragonstone ring",
            "Dragonstone amulet",
        ],
        _ => return None,
    })
}

fn bank_location_names(ident: &str) -> &'static [&'static str] {
    const BANKS: &[&str] = &[
        "Varrock East",
        "Varrock West",
        "Al Kharid",
        "Draynor",
        "Falador East",
        "Falador West",
        "Edgeville",
        "Seers",
        "Catherby",
        "Yanille",
        "Ardougne West",
        "Ardougne East",
        "Canifis",
        "Shilo Village",
        "Fishing Guild",
        "Shantay Pass",
        "Mage Arena",
        "Grand Tree",
        "Duel Arena",
    ];
    const WITH_NEAREST: &[&str] = &[
        "Nearest",
        "Varrock East",
        "Varrock West",
        "Al Kharid",
        "Draynor",
        "Falador East",
        "Falador West",
        "Edgeville",
        "Seers",
        "Catherby",
        "Yanille",
        "Ardougne West",
        "Ardougne East",
        "Canifis",
        "Shilo Village",
        "Fishing Guild",
        "Shantay Pass",
        "Mage Arena",
        "Grand Tree",
        "Duel Arena",
    ];
    if ident == "BANK_LOCATION_OPTIONS" {
        WITH_NEAREST
    } else {
        BANKS
    }
}

fn catalog_option_values(ident: &str) -> Option<Vec<String>> {
    catalog_option_table(ident).map(|rows| rows.iter().map(|s| (*s).to_string()).collect())
}

/// Static `const NAME = ['a', 'b']` (or a same-file / shim alias of one).
/// Does not evaluate `Object.keys(...)` or other TypeScript.
fn resolve_string_array_ident(file_src: &str, ident: &str) -> Option<Vec<String>> {
    resolve_string_array_ident_visited(file_src, ident, &mut Vec::new())
}

fn resolve_string_array_ident_visited(
    file_src: &str,
    ident: &str,
    stack: &mut Vec<String>,
) -> Option<Vec<String>> {
    if stack.iter().any(|s| s == ident) {
        return None;
    }
    stack.push(ident.to_string());
    if let Some(arr) = string_array_literal_in(file_src, ident) {
        return Some(arr);
    }
    if let Some(alias) = string_array_alias_in(file_src, ident) {
        return resolve_string_array_ident_visited(file_src, &alias, stack);
    }
    for shim in [
        include_str!("shim/banking.js"),
        include_str!("shim/combat_style.js"),
        include_str!("shim/thieving_targets.js"),
        include_str!("shim/food.js"),
        include_str!("shim/steal_rules.js"),
    ] {
        if let Some(arr) = string_array_literal_in(shim, ident) {
            return Some(arr);
        }
        if let Some(alias) = string_array_alias_in(shim, ident) {
            return resolve_string_array_ident_visited(shim, &alias, stack);
        }
    }
    if let Some(rhs) = const_eq_rhs(file_src, ident) {
        let trimmed = rhs.trim_start();
        if let Some(arr) = parse_mapped_or_keys_rhs(trimmed, file_src) {
            return Some(arr);
        }
        if let Some(labels) = object_array_field_values(trimmed, "label", file_src) {
            return Some(labels);
        }
    }
    catalog_option_values(ident)
}

fn parse_mapped_or_keys_rhs(rhs: &str, file_src: &str) -> Option<Vec<String>> {
    if let Some(keys) = object_keys_call(rhs, file_src) {
        return Some(keys);
    }
    if let Some((base, after)) = take_ident(rhs) {
        if after.trim_start().starts_with(".map") {
            if let Some((vals, _)) = take_map_call(base, after.trim_start(), file_src) {
                return Some(vals);
            }
        }
    }
    None
}

fn const_eq_rhs<'a>(src: &'a str, ident: &str) -> Option<&'a str> {
    let needles = [format!("export const {ident}"), format!("const {ident}")];
    for needle in needles {
        let mut search = src;
        while let Some(idx) = search.find(&needle) {
            let abs = src.len() - search.len() + idx;
            let after = &search[idx + needle.len()..];
            let cont = after
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$');
            if cont || !const_decl_is_live(src, abs) {
                search = &search[idx + 1..];
                continue;
            }
            let mut rest = after.trim_start();
            if rest.starts_with(':') {
                rest = skip_ts_type_to_eq(rest)?;
            }
            let rest = rest.strip_prefix('=')?.trim_start();
            return Some(rest);
        }
    }
    None
}

/// `const` / `export const` at `idx` is a live declaration, not a comment.
fn const_decl_is_live(src: &str, idx: usize) -> bool {
    let line_start = src[..idx].rfind('\n').map(|i| i + 1).unwrap_or(0);
    if !src[line_start..idx].trim().is_empty() {
        return false;
    }
    !in_unclosed_block_comment(&src[..idx])
}

fn in_unclosed_block_comment(before: &str) -> bool {
    let bytes = before.as_bytes();
    let mut i = 0;
    let mut in_block = false;
    let mut in_line = false;
    while i < bytes.len() {
        let b = bytes[i];
        if in_line {
            if b == b'\n' {
                in_line = false;
            }
            i += 1;
            continue;
        }
        if in_block {
            if b == b'*' && i + 1 < bytes.len() && bytes[i + 1] == b'/' {
                in_block = false;
                i += 2;
                continue;
            }
            i += 1;
            continue;
        }
        if b == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'/' {
            in_line = true;
            i += 2;
            continue;
        }
        if b == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'*' {
            in_block = true;
            i += 2;
            continue;
        }
        i += 1;
    }
    in_block
}

fn string_array_literal_in(src: &str, ident: &str) -> Option<Vec<String>> {
    let rhs = const_eq_rhs(src, ident)?;
    parse_string_array_with_consts(rhs, src)
}

/// Parse a string array whose elements are literals, string consts, or a
/// recognized finite `...IDENT` / `...IDENT.map(p => p.field)` spread.
/// `.map(... .key)` stays unresolved so high-alchemy `item_option_spec`
/// keeps ownership. Anything else dynamic remains unresolved.
fn parse_string_array_with_consts(rhs: &str, file_src: &str) -> Option<Vec<String>> {
    let s = rhs.trim_start();
    if !s.starts_with('[') {
        return None;
    }
    let end = find_matching_bracket(s, '[', ']')?;
    let mut rest = s[1..end].trim_start();
    let mut out = Vec::new();
    while !rest.is_empty() {
        if rest.starts_with("...") {
            rest = rest[3..].trim_start();
            let (ident, after) = take_ident(rest)?;
            rest = after.trim_start();
            if rest.starts_with(".map") {
                let (vals, after_map) = take_map_call(ident, rest, file_src)?;
                out.extend(vals);
                rest = after_map.trim_start();
            } else {
                out.extend(resolve_string_array_ident(file_src, ident)?);
            }
        } else if let Some((value, n)) = scan_quoted(rest) {
            out.push(value);
            rest = rest[n..].trim_start();
        } else if let Some((ident, after)) = take_ident(rest) {
            rest = after.trim_start();
            if rest.starts_with(".map") {
                let (vals, after_map) = take_map_call(ident, rest, file_src)?;
                out.extend(vals);
                rest = after_map.trim_start();
            } else if let Some(after_dot) = rest.strip_prefix('.') {
                let after_dot = after_dot.trim_start();
                let (field, after_field) = take_ident(after_dot)?;
                out.push(quoted_object_const_field(file_src, ident, field)?);
                rest = after_field.trim_start();
            } else {
                out.push(resolve_string_const_ident(file_src, ident)?);
            }
        } else {
            return None;
        }
        if rest.starts_with(',') {
            rest = rest[1..].trim_start();
        } else if !rest.is_empty() {
            return None;
        }
    }
    Some(out)
}

/// `IDENT.map(param => param.field)` of a same-file object-literal array.
/// Field `key` is reserved for the high-alchemy item-option walker.
fn take_map_call<'a>(
    ident: &str,
    after_ident: &'a str,
    file_src: &str,
) -> Option<(Vec<String>, &'a str)> {
    let rest = after_ident.trim_start().strip_prefix('.')?.trim_start();
    let rest = rest.strip_prefix("map")?.trim_start();
    let rest = rest.strip_prefix('(')?.trim_start();
    let rest = rest.strip_prefix('(').unwrap_or(rest).trim_start();
    let (param, after_param) = take_ident(rest)?;
    let rest = after_param.trim_start();
    let rest = rest.strip_prefix(')').unwrap_or(rest).trim_start();
    let rest = rest.strip_prefix("=>")?.trim_start();
    let (recv, after_recv) = take_ident(rest)?;
    if recv != param {
        return None;
    }
    let rest = after_recv.trim_start().strip_prefix('.')?.trim_start();
    let (field, after_field) = take_ident(rest)?;
    if field == "key" {
        return None;
    }
    let rest = after_field.trim_start();
    let rest = rest.strip_prefix(')').unwrap_or(rest).trim_start();
    let rest = rest.strip_prefix(')').unwrap_or(rest).trim_start();
    let vals = if let Some(rhs) = const_eq_rhs(file_src, ident) {
        object_array_field_values(rhs, field, file_src)?
    } else {
        catalog_option_values(ident)?
    };
    Some((vals, rest))
}

fn quoted_object_const_field(file_src: &str, ident: &str, field: &str) -> Option<String> {
    let rhs = const_eq_rhs(file_src, ident)?.trim_start();
    if !rhs.starts_with('{') {
        return None;
    }
    quoted_field_in_object(rhs, field)
}

/// Quoted `field:` values from a same-file `[ { … }, … ]`. Constructor
/// elements (`row(...)`) fail closed so metadata can own those tables.
fn object_array_field_values(rhs: &str, field: &str, file_src: &str) -> Option<Vec<String>> {
    let _ = file_src;
    let s = rhs.trim_start();
    if !s.starts_with('[') {
        return None;
    }
    let end = find_matching_bracket(s, '[', ']')?;
    let mut rest = s[1..end].trim_start();
    let mut out = Vec::new();
    while !rest.is_empty() {
        if rest.starts_with(',') {
            rest = rest[1..].trim_start();
            continue;
        }
        if !rest.starts_with('{') {
            return None;
        }
        let obj_end = find_matching_bracket(rest, '{', '}')?;
        out.push(quoted_field_in_object(&rest[..=obj_end], field)?);
        rest = rest[obj_end + 1..].trim_start();
        if rest.starts_with(',') {
            rest = rest[1..].trim_start();
        } else if !rest.is_empty() {
            return None;
        }
    }
    Some(out)
}

fn quoted_field_in_object(obj: &str, field: &str) -> Option<String> {
    let s = obj.trim_start();
    if !s.starts_with('{') {
        return None;
    }
    let end = find_matching_bracket(s, '{', '}')?;
    let inner = s[1..end].trim();
    let mut rest = inner;
    while !rest.is_empty() {
        if rest.starts_with(',') {
            rest = rest[1..].trim_start();
            continue;
        }
        if rest.starts_with('[') {
            let e = find_matching_bracket(rest, '[', ']')?;
            rest = skip_object_value(rest[e + 1..].trim_start())?;
            continue;
        }
        let (key, after_key) = if let Some((quoted, n)) = scan_quoted(rest) {
            (quoted, rest[n..].trim_start())
        } else {
            let (ident, after) = take_ident(rest)?;
            (ident.to_string(), after.trim_start())
        };
        let after_colon = after_key.strip_prefix(':')?.trim_start();
        if key == field {
            let (value, _) = scan_quoted(after_colon)?;
            return Some(value);
        }
        rest = skip_object_value(after_colon)?;
    }
    None
}

fn skip_object_value(after_colon: &str) -> Option<&str> {
    let rest = skip_expression(after_colon)?;
    let rest = rest.trim_start();
    if rest.starts_with(',') {
        Some(rest[1..].trim_start())
    } else {
        Some(rest)
    }
}

fn string_array_alias_in(src: &str, ident: &str) -> Option<String> {
    let rhs = const_eq_rhs(src, ident)?;
    if rhs.starts_with('[') {
        return None;
    }
    ident_alias_rhs(rhs)
}

/// Quoted string `const NAME = 'x'` (or a same-file alias of one).
fn resolve_string_const_ident(file_src: &str, ident: &str) -> Option<String> {
    resolve_string_const_ident_visited(file_src, ident, &mut Vec::new())
}

fn resolve_string_const_ident_visited(
    file_src: &str,
    ident: &str,
    stack: &mut Vec<String>,
) -> Option<String> {
    if stack.iter().any(|s| s == ident) {
        return None;
    }
    stack.push(ident.to_string());
    if let Some(value) = string_const_literal_in(file_src, ident) {
        return Some(value);
    }
    if let Some(alias) = string_const_alias_in(file_src, ident) {
        return resolve_string_const_ident_visited(file_src, &alias, stack);
    }
    None
}

fn string_const_literal_in(src: &str, ident: &str) -> Option<String> {
    quoted_after(const_eq_rhs(src, ident)?)
}

fn string_const_alias_in(src: &str, ident: &str) -> Option<String> {
    let rhs = const_eq_rhs(src, ident)?;
    if quoted_after(rhs).is_some() || rhs.starts_with('[') || rhs.starts_with('{') {
        return None;
    }
    ident_alias_rhs(rhs)
}

fn ident_alias_rhs(rhs: &str) -> Option<String> {
    let (alias, rest) = take_ident(rhs)?;
    if alias == "Object" || alias == "new" {
        return None;
    }
    let rest = rest.trim_start();
    if rest.starts_with('.') || rest.starts_with('(') {
        return None;
    }
    Some(alias.to_string())
}

fn object_keys_call(after: &str, file_src: &str) -> Option<Vec<String>> {
    let rest = after.strip_prefix("Object.keys(")?.trim_start();
    let (ident, _) = take_ident(rest)?;
    if let Some(keys) = object_keys_in(file_src, ident) {
        if !keys.is_empty() {
            return Some(keys);
        }
    }
    if let Some(keys) = object_keys_in(include_str!("shim/data/spelldb.js"), ident) {
        if !keys.is_empty() {
            return Some(keys);
        }
    }
    catalog_option_values(ident)
}

fn object_keys_in(src: &str, ident: &str) -> Option<Vec<String>> {
    let rhs = const_eq_rhs(src, ident)?;
    quoted_keys_in_object(rhs)
}

fn quoted_keys_in_object(rhs: &str) -> Option<Vec<String>> {
    if !rhs.starts_with('{') {
        return None;
    }
    let end = find_matching_bracket(rhs, '{', '}')?;
    let mut rest = rhs[1..end].trim_start();
    let mut keys = Vec::new();
    while !rest.is_empty() {
        if rest.starts_with(',') {
            rest = rest[1..].trim_start();
            continue;
        }
        if rest.starts_with('[') {
            let e = find_matching_bracket(rest, '[', ']')?;
            rest = rest[e + 1..].trim_start().strip_prefix(':')?.trim_start();
            rest = skip_expression(rest)?.trim_start();
            if rest.starts_with(',') {
                rest = rest[1..].trim_start();
            }
            continue;
        }
        let (k, after_key) = if let Some((quoted, n)) = scan_quoted(rest) {
            (quoted, rest[n..].trim_start())
        } else if let Some((ident, after)) = take_ident(rest) {
            (ident.to_string(), after.trim_start())
        } else {
            break;
        };
        let after_colon = after_key.strip_prefix(':')?.trim_start();
        keys.push(k);
        rest = skip_expression(after_colon)?.trim_start();
        if rest.starts_with(',') {
            rest = rest[1..].trim_start();
        }
    }
    Some(keys)
}

fn scan_key_show_if(obj: &str, file_src: &str) -> Option<String> {
    let raw = scan_key_raw_value(obj, "showIf")?;
    if raw.starts_with('{') {
        return Some(rewrite_show_if_any_of_idents(&raw, file_src));
    }
    if let Some(rhs) = const_eq_rhs(file_src, &raw) {
        if rhs.starts_with('{') {
            if let Some(end) = find_matching_bracket(rhs, '{', '}') {
                return Some(rewrite_show_if_any_of_idents(&rhs[..=end], file_src));
            }
        }
    }
    Some(raw)
}

/// Rewrite resolvable string-const idents inside `anyOf: […]`. Unknown or
/// computed elements leave the object unchanged so the condition is not dropped
/// and is not a partial successful parse.
fn rewrite_show_if_any_of_idents(raw: &str, file_src: &str) -> String {
    let Some(any_idx) = raw.find("anyOf:") else {
        return raw.to_string();
    };
    let after_key = &raw[any_idx + "anyOf:".len()..];
    let trimmed = after_key.trim_start();
    if !trimmed.starts_with('[') {
        return raw.to_string();
    }
    let Some(end) = find_matching_bracket(trimmed, '[', ']') else {
        return raw.to_string();
    };
    let Some(new_inner) = rewrite_any_of_elements(&trimmed[1..end], file_src) else {
        return raw.to_string();
    };
    let pad = after_key.len() - trimmed.len();
    format!(
        "{}{}[{}]{}",
        &raw[..any_idx + "anyOf:".len()],
        &after_key[..pad],
        new_inner,
        &trimmed[end + 1..]
    )
}

fn rewrite_any_of_elements(inner: &str, file_src: &str) -> Option<String> {
    let mut parts = Vec::new();
    let mut rest = inner;
    loop {
        rest = rest.trim_start();
        if rest.is_empty() {
            break;
        }
        if let Some((value, n)) = scan_quoted(rest) {
            parts.push(format!("'{value}'"));
            rest = &rest[n..];
        } else if let Some((ident, after)) = take_ident(rest) {
            let value = resolve_string_const_ident(file_src, ident)?;
            parts.push(format!("'{value}'"));
            rest = after;
        } else {
            return None;
        }
        rest = rest.trim_start();
        if rest.starts_with(',') {
            rest = &rest[1..];
        } else if rest.is_empty() {
            break;
        } else {
            return None;
        }
    }
    Some(parts.join(", "))
}

fn parse_settings_object(
    obj: &str,
    file_src: &str,
    sources: Option<&HashMap<String, String>>,
) -> Vec<SettingDef> {
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
        if let Some(after_dots) = rest.strip_prefix("...") {
            let expr = after_dots.trim_start();
            if let Some(body) = resolve_spread_settings(expr, file_src) {
                out.extend(parse_settings_object(&body, file_src, sources));
            }
            let Some(after_expr) = skip_expression(expr) else {
                break;
            };
            rest = after_expr.trim_start();
            if rest.starts_with(',') {
                rest = &rest[1..];
            }
            continue;
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
            out.push(parse_setting_def(
                &id,
                &after_colon[..=end],
                file_src,
                sources,
            ));
            rest = &after_colon[end + 1..];
        } else if let Some((ident, after_ident)) = take_ident(after_colon) {
            let leftover = after_ident.trim_start();
            if leftover.starts_with('.') || leftover.starts_with('(') {
                let Some(after_expr) = skip_expression(after_colon) else {
                    break;
                };
                rest = after_expr;
            } else if let Some(body) = resolve_setting_ident(file_src, ident) {
                out.push(parse_setting_def(&id, &body, file_src, sources));
                rest = after_ident;
            } else {
                let Some(after_expr) = skip_expression(after_colon) else {
                    break;
                };
                rest = after_expr;
            }
        } else {
            let Some(after_expr) = skip_expression(after_colon) else {
                break;
            };
            rest = after_expr;
        }
        rest = rest.trim_start();
        if rest.starts_with(',') {
            rest = &rest[1..];
        }
    }
    out
}

/// Bare `...IDENT` or `...Object.fromEntries(Object.entries(IDENT)…)`.
/// Unknown call expressions stay unresolved; the caller still skips them.
fn resolve_spread_settings(expr: &str, file_src: &str) -> Option<String> {
    let expr = expr.trim_start();
    if let Some((ident, after)) = take_ident(expr) {
        let after = after.trim_start();
        if !after.starts_with('.') && !after.starts_with('(') {
            return resolve_setting_ident(file_src, ident);
        }
    }
    resolve_from_entries_settings(expr, file_src)
}

fn resolve_from_entries_settings(expr: &str, file_src: &str) -> Option<String> {
    let rest = expr.strip_prefix("Object")?.trim_start();
    let rest = rest.strip_prefix('.')?.trim_start();
    let rest = rest.strip_prefix("fromEntries")?.trim_start();
    let rest = rest.strip_prefix('(')?.trim_start();
    let rest = rest.strip_prefix("Object")?.trim_start();
    let rest = rest.strip_prefix('.')?.trim_start();
    let rest = rest.strip_prefix("entries")?.trim_start();
    let rest = rest.strip_prefix('(')?.trim_start();
    let (ident, _) = take_ident(rest)?;
    resolve_setting_ident(file_src, ident)
}

/// Advance past one primary expression and its `.ident` / `(…)` / `[…]`
/// postfix so an unsupported spread cannot swallow the next field.
fn skip_expression(s: &str) -> Option<&str> {
    let mut rest = s.trim_start();
    if rest.is_empty() {
        return None;
    }
    if let Some(after_new) = rest.strip_prefix("new ") {
        rest = after_new.trim_start();
    }
    if rest.starts_with('{') {
        let end = find_matching_bracket(rest, '{', '}')?;
        rest = &rest[end + 1..];
    } else if rest.starts_with('[') {
        let end = find_matching_bracket(rest, '[', ']')?;
        rest = &rest[end + 1..];
    } else if rest.starts_with('(') {
        let end = find_matching_bracket(rest, '(', ')')?;
        rest = &rest[end + 1..];
    } else if let Some((_, n)) = scan_quoted(rest) {
        rest = &rest[n..];
    } else if let Some((_, after)) = take_ident(rest) {
        rest = after;
    } else if rest.starts_with(|c: char| c.is_ascii_digit() || c == '-' || c == '.') {
        rest = rest.trim_start_matches(|c: char| {
            c.is_ascii_digit() || c == '.' || c == '-' || c == 'e' || c == 'E'
        });
    } else {
        return None;
    }
    loop {
        rest = rest.trim_start();
        if let Some(after_dot) = rest.strip_prefix('.') {
            let after_dot = after_dot.trim_start();
            let Some((_, after_ident)) = take_ident(after_dot) else {
                break;
            };
            rest = after_ident;
            continue;
        }
        if rest.starts_with('(') {
            let end = find_matching_bracket(rest, '(', ')')?;
            rest = &rest[end + 1..];
            continue;
        }
        if rest.starts_with('[') {
            let end = find_matching_bracket(rest, '[', ']')?;
            rest = &rest[end + 1..];
            continue;
        }
        break;
    }
    Some(rest)
}

fn parse_setting_def(
    id: &str,
    obj: &str,
    file_src: &str,
    sources: Option<&HashMap<String, String>>,
) -> SettingDef {
    let options = scan_key_options(obj, "options", file_src, sources);
    let item_option_spec = if options.is_empty() {
        scan_item_option_spec(obj, file_src)
    } else {
        None
    };
    let options_from = scan_key_quoted(obj, "optionsFrom")
        .or_else(|| scan_key_ident(obj, "optionsFrom"))
        .or_else(|| inferred_options_from(obj, &options, item_option_spec.as_ref()));
    SettingDef {
        id: id.to_string(),
        ty: scan_key_quoted(obj, "type").unwrap_or_default(),
        default: scan_key_literal(obj, "default", Some(file_src)),
        label: scan_key_quoted(obj, "label"),
        min: scan_key_number(obj, "min"),
        max: scan_key_number(obj, "max"),
        step: scan_key_number(obj, "step"),
        options,
        option_labels: scan_key_option_labels(obj, file_src),
        group: scan_key_quoted(obj, "group"),
        show_if: scan_key_show_if(obj, file_src),
        options_from,
        csv_toggle: scan_key_raw_value(obj, "csvToggle"),
        help: scan_key_quoted(obj, "help"),
        item_option_spec,
    }
}

fn scan_key_literal(block: &str, key: &str, file_src: Option<&str>) -> Option<String> {
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
        if let Some(src) = file_src {
            if let Some((ident, _)) = take_ident(after) {
                if let Some(value) = resolve_string_const_ident(src, ident) {
                    return Some(value);
                }
                if let Some(arr) = resolve_string_array_ident(src, ident) {
                    return Some(serde_json::to_string(&arr).expect("string array default json"));
                }
                // Unresolved ident is not a scalar value.
                return None;
            }
        }
        rest = after;
    }
    None
}

fn scan_key_number(block: &str, key: &str) -> Option<String> {
    scan_key_literal(block, key, None)
}

fn inferred_options_from(
    obj: &str,
    options: &[String],
    item_option_spec: Option<&ItemOptionSpec>,
) -> Option<String> {
    if !options.is_empty() || item_option_spec.is_some() {
        return None;
    }
    let ident = options_ident(obj)?;
    if is_revision_fact_option_ident(ident) || catalog_option_table(ident).is_some() {
        return Some(ident.to_string());
    }
    None
}

fn scan_key_options(
    block: &str,
    key: &str,
    file_src: &str,
    sources: Option<&HashMap<String, String>>,
) -> Vec<String> {
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
        if let Some(arr) = parse_string_array_with_consts(after, file_src) {
            return arr;
        }
        if let Some(arr) = parse_string_array(after) {
            return arr;
        }
        if let Some(keys) = object_keys_call(after, file_src) {
            return keys;
        }
        if let Some((ident, leftover)) = take_ident(after) {
            let leftover = leftover.trim_start();
            if leftover.starts_with(".map") {
                return take_map_call(ident, leftover, file_src)
                    .map(|(vals, _)| vals)
                    .unwrap_or_default();
            }
            if ident == "presetBuyableNames" {
                if take_empty_call_expression(leftover).is_some() {
                    return resolve_preset_buyable_names(file_src, sources).unwrap_or_default();
                }
                return Vec::new();
            }
            if leftover.starts_with('(') {
                return Vec::new();
            }
            if leftover.starts_with('.') {
                return Vec::new();
            }
            if is_revision_fact_option_ident(ident) {
                return Vec::new();
            }
            return resolve_string_array_ident(file_src, ident).unwrap_or_default();
        }
        return Vec::new();
    }
    Vec::new()
}

fn scan_key_option_labels(block: &str, file_src: &str) -> Vec<String> {
    let mut rest = block;
    while let Some(idx) = rest.find("optionLabels") {
        let after = &rest[idx + "optionLabels".len()..];
        let after = after.trim_start();
        let Some(after) = after.strip_prefix(':').map(str::trim_start) else {
            rest = after;
            continue;
        };
        if let Some(labels) = parse_string_array(after) {
            return labels;
        }
        if let Some((ident, _)) = take_ident(after) {
            if let Some(rhs) = const_eq_rhs(file_src, ident) {
                if let Some(labels) = parse_string_record(rhs, file_src) {
                    return labels;
                }
            }
        }
        return Vec::new();
    }
    Vec::new()
}

/// Values from a static `Record<string, string>` object, in declaration order.
/// Computed keys/values are accepted only when they resolve to string consts.
fn parse_string_record(rhs: &str, file_src: &str) -> Option<Vec<String>> {
    let s = rhs.trim_start();
    if !s.starts_with('{') {
        return None;
    }
    let end = find_matching_bracket(s, '{', '}')?;
    let mut rest = s[1..end].trim_start();
    let mut out = Vec::new();
    while !rest.is_empty() {
        if rest.starts_with(',') {
            rest = rest[1..].trim_start();
            continue;
        }
        let (_key, after_key) = if let Some((key, n)) = scan_quoted(rest) {
            (key, &rest[n..])
        } else if let Some(after_open) = rest.strip_prefix('[') {
            let (ident, after_ident) = take_ident(after_open.trim_start())?;
            let after_ident = after_ident.trim_start();
            let after_close = after_ident.strip_prefix(']')?.trim_start();
            (resolve_string_const_ident(file_src, ident)?, after_close)
        } else {
            let (key, after) = take_ident(rest)?;
            (key.to_string(), after)
        };
        let after_key = after_key.trim_start().strip_prefix(':')?.trim_start();
        let (value, n) = if let Some((value, n)) = scan_quoted(after_key) {
            (value, n)
        } else {
            let (ident, after) = take_ident(after_key)?;
            let value = resolve_string_const_ident(file_src, ident)?;
            (value, after_key.len() - after.len())
        };
        out.push(value);
        rest = after_key[n..].trim_start();
        if rest.starts_with(',') {
            rest = rest[1..].trim_start();
        } else if !rest.is_empty() {
            return None;
        }
    }
    Some(out)
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

/// Recognized high-alchemy `options: IDENT` whose sibling RHS is
/// `[prefix, ...items.map(i => i.key)]` and `items` is the FODDER.flatMap
/// / ITEM_DB / Math.floor(cost * 0.6) / sort chain. Unknown maps stay
/// unresolved. Never evaluates TypeScript.
fn scan_item_option_spec(block: &str, file_src: &str) -> Option<ItemOptionSpec> {
    let ident = options_ident(block)?;
    let rhs = const_eq_rhs(file_src, ident)?;
    let (prefix_token, items_ident, sort_keys_by_label) = parse_alch_options_rhs(rhs)?;
    let prefix = match prefix_token {
        AlchPrefix::Quoted(value) => value,
        AlchPrefix::Ident(name) => resolve_string_const_ident(file_src, &name)?,
    };
    if prefix.is_empty() {
        return None;
    }
    let items_rhs = const_eq_rhs(file_src, &items_ident)?;
    let fodder_ident = high_alchemy_fodder_ident(file_src, items_rhs)?;
    let fodder_rhs = const_eq_rhs(file_src, fodder_ident)?;
    let candidates = parse_fodder_candidates(fodder_rhs)?;
    Some(ItemOptionSpec {
        prefix: vec![prefix],
        candidates,
        sort_keys_by_label,
    })
}

fn options_ident(block: &str) -> Option<&str> {
    let mut rest = block;
    while let Some(idx) = rest.find("options") {
        let after = &rest[idx + "options".len()..];
        let after = after.trim_start();
        let after = match after.strip_prefix(':') {
            Some(a) => a.trim_start(),
            None => {
                rest = after;
                continue;
            }
        };
        if after.starts_with('[') || after.starts_with("Object.keys(") {
            return None;
        }
        let (ident, _) = take_ident(after)?;
        return Some(ident);
    }
    None
}

enum AlchPrefix {
    Quoted(String),
    Ident(String),
}

/// `[PREFIX, ...ITEMS.map(param => param.key)]` or the frozen sort-wrapper
/// spread `[PREFIX, ...[...ITEMS].sort((a,b)=>a.label.localeCompare(b.label)).map(i=>i.key)]`.
fn parse_alch_options_rhs(rhs: &str) -> Option<(AlchPrefix, String, bool)> {
    let s = rhs.trim_start();
    if !s.starts_with('[') {
        return None;
    }
    let end = find_matching_bracket(s, '[', ']')?;
    let mut inner = s[1..end].trim_start();
    let prefix = if let Some((value, n)) = scan_quoted(inner) {
        inner = inner[n..].trim_start();
        AlchPrefix::Quoted(value)
    } else {
        let (ident, after) = take_ident(inner)?;
        inner = after.trim_start();
        AlchPrefix::Ident(ident.to_string())
    };
    inner = inner.strip_prefix(',')?.trim_start();
    inner = inner.strip_prefix("...")?.trim_start();
    let (items, mut inner) = if inner.starts_with('[') {
        let wrap_end = find_matching_bracket(inner, '[', ']')?;
        let inside = inner[1..wrap_end].trim_start();
        let inside = inside.strip_prefix("...")?.trim_start();
        let (items, after_items) = take_ident(inside)?;
        if !after_items.trim().is_empty() {
            return None;
        }
        (items.to_string(), &inner[wrap_end + 1..])
    } else {
        let (items, after) = take_ident(inner)?;
        (items.to_string(), after)
    };
    inner = inner.trim_start();
    let mut sort_keys_by_label = false;
    if let Some(after_sort) = skip_bounded_alch_label_sort(inner) {
        sort_keys_by_label = true;
        inner = after_sort;
    }
    inner = inner.strip_prefix('.')?.trim_start();
    inner = inner.strip_prefix("map")?.trim_start();
    inner = inner.strip_prefix('(')?.trim_start();
    let (param, after) = if let Some(after_paren) = inner.strip_prefix('(') {
        let after_paren = after_paren.trim_start();
        let (param, after) = take_ident(after_paren)?;
        let after = after.trim_start().strip_prefix(')')?.trim_start();
        (param, after)
    } else {
        let (param, after) = take_ident(inner)?;
        (param, after.trim_start())
    };
    inner = after;
    inner = inner.strip_prefix("=>")?.trim_start();
    let (recv, after) = take_ident(inner)?;
    if recv != param {
        return None;
    }
    inner = after.trim_start();
    inner = inner.strip_prefix('.')?.trim_start();
    inner = inner.strip_prefix("key")?.trim_start();
    inner = inner.strip_prefix(')')?.trim_start();
    if !inner.is_empty() {
        return None;
    }
    if !value_expression_complete(&s[end + 1..]) {
        return None;
    }
    Some((prefix, items, sort_keys_by_label))
}

/// Frozen high-alchemy chip order: `.sort((a,b)=>a.label.localeCompare(b.label))`.
fn skip_bounded_alch_label_sort(after: &str) -> Option<&str> {
    let rest = after.strip_prefix('.')?.trim_start();
    let rest = rest.strip_prefix("sort")?.trim_start();
    let rest = rest.strip_prefix('(')?.trim_start();
    let rest = rest.strip_prefix('(')?.trim_start();
    let (a, rest) = take_ident(rest)?;
    let rest = rest.trim_start().strip_prefix(',')?.trim_start();
    let (b, rest) = take_ident(rest)?;
    let rest = rest.trim_start().strip_prefix(')')?.trim_start();
    let rest = rest.strip_prefix("=>")?.trim_start();
    let (lhs, rest) = take_ident(rest)?;
    if lhs != a {
        return None;
    }
    let rest = rest.strip_prefix('.')?.trim_start();
    let (field, rest) = take_ident(rest)?;
    if field != "label" {
        return None;
    }
    let rest = rest.strip_prefix('.')?.trim_start();
    let rest = rest.strip_prefix("localeCompare")?.trim_start();
    let rest = rest.strip_prefix('(')?.trim_start();
    let (cmp_recv, rest) = take_ident(rest)?;
    let rest = rest.strip_prefix('.')?.trim_start();
    let (cmp_field, rest) = take_ident(rest)?;
    if cmp_recv != b || cmp_field != "label" {
        return None;
    }
    let rest = rest.strip_prefix(')')?.trim_start();
    let rest = rest.strip_prefix(')')?.trim_start();
    Some(rest)
}

/// `presetBuyableNames()` with no args and no trailing expression suffix.
fn take_empty_call_expression(after: &str) -> Option<&str> {
    let rest = after.trim_start().strip_prefix('(')?.trim_start();
    let rest = rest.strip_prefix(')')?;
    if value_expression_complete(rest) {
        Some(rest)
    } else {
        None
    }
}

/// Remainder after a finished value: empty, terminator, or the next declaration.
fn value_expression_complete(after: &str) -> bool {
    let rest = after.trim_start();
    if rest.is_empty() || rest.starts_with(';') || rest.starts_with(',') || rest.starts_with('}') {
        return true;
    }
    rest.starts_with("export")
        || rest.starts_with("const ")
        || rest.starts_with("function ")
        || rest.starts_with("let ")
        || rest.starts_with("var ")
        || rest.starts_with("import")
}

struct ShopDbRecord {
    keepers: Vec<String>,
    item_names: Vec<String>,
}

/// `presetBuyableNames()` from frozen `shopPresets.ts` × parsed `SHOP_DB`.
fn resolve_preset_buyable_names(
    file_src: &str,
    sources: Option<&HashMap<String, String>>,
) -> Option<Vec<String>> {
    let presets_rhs = const_eq_rhs(file_src, "SHOP_PRESETS")?;
    let preset_keepers = object_array_field_values(presets_rhs, "keeper", file_src)?;
    let shop_db_src = shop_db_source_text(sources, file_src)?;
    let records = parse_shop_db_records(&shop_db_src)?;
    if records.is_empty() || preset_keepers.is_empty() {
        return None;
    }
    let mut names = std::collections::BTreeSet::new();
    for keeper in preset_keepers {
        let Some(rec) = records
            .iter()
            .find(|r| r.keepers.iter().any(|k| k == &keeper))
        else {
            continue;
        };
        for name in &rec.item_names {
            names.insert(name.clone());
        }
    }
    if names.is_empty() {
        return None;
    }
    Some(names.into_iter().collect())
}

const SHOP_DB_SOURCE_KEYS: [&str; 2] = ["./src/bot/data/shopdb.ts", "./src/bot/data/shopdb.js"];

/// Load `SHOP_DB` from the registered catalog root at the known data path.
pub(crate) fn insert_shop_db_from_root(sources: &mut HashMap<String, String>, root: &Path) {
    let ts = root.join("src/bot/data/shopdb.ts");
    let js = root.join("src/bot/data/shopdb.js");
    let text = std::fs::read_to_string(&ts)
        .ok()
        .or_else(|| std::fs::read_to_string(&js).ok());
    let Some(text) = text else {
        return;
    };
    sources
        .entry(SHOP_DB_SOURCE_KEYS[0].into())
        .or_insert_with(|| text.clone());
    sources.entry(SHOP_DB_SOURCE_KEYS[1].into()).or_insert(text);
}

fn shop_db_source_text(sources: Option<&HashMap<String, String>>, file_src: &str) -> Option<String> {
    if let Some(map) = sources {
        for key in SHOP_DB_SOURCE_KEYS {
            if let Some(text) = lookup_source(map, key) {
                return Some(text);
            }
        }
        if const_eq_rhs(file_src, "SHOP_DB").is_some() {
            return Some(file_src.to_string());
        }
        return None;
    }
    if const_eq_rhs(file_src, "SHOP_DB").is_some() {
        return Some(file_src.to_string());
    }
    None
}

fn parse_shop_db_records(src: &str) -> Option<Vec<ShopDbRecord>> {
    let rhs = const_eq_rhs(src, "SHOP_DB")?;
    let s = rhs.trim_start();
    if !s.starts_with('{') {
        return None;
    }
    let end = find_matching_bracket(s, '{', '}')?;
    let mut rest = s[1..end].trim_start();
    let mut out = Vec::new();
    while !rest.is_empty() {
        if rest.starts_with(',') {
            rest = rest[1..].trim_start();
            continue;
        }
        let Some((_, key_end)) = scan_quoted(rest) else {
            return None;
        };
        let after_key = &rest[key_end..];
        let after_colon = after_key.trim_start().strip_prefix(':')?.trim_start();
        if !after_colon.starts_with('{') {
            return None;
        }
        let obj_end = find_matching_bracket(after_colon, '{', '}')?;
        let record_obj = &after_colon[..=obj_end];
        out.push(parse_one_shop_db_record(record_obj)?);
        rest = after_colon[obj_end + 1..].trim_start();
        if rest.starts_with(',') {
            rest = rest[1..].trim_start();
        } else if !rest.is_empty() {
            return None;
        }
    }
    Some(out)
}

fn parse_one_shop_db_record(obj: &str) -> Option<ShopDbRecord> {
    let keepers = parse_json_string_array_field(obj, "keepers")?;
    let item_names = parse_shop_items_names(obj)?;
    Some(ShopDbRecord {
        keepers,
        item_names,
    })
}

fn parse_json_string_array_field(obj: &str, field: &str) -> Option<Vec<String>> {
    let needle = format!("\"{field}\":");
    let idx = obj.find(&needle)?;
    let after = obj[idx + needle.len()..].trim_start();
    if !after.starts_with('[') {
        return None;
    }
    let end = find_matching_bracket(after, '[', ']')?;
    parse_string_array(&after[..=end])
}

fn parse_shop_items_names(obj: &str) -> Option<Vec<String>> {
    let needle = "\"items\":";
    let idx = obj.find(needle)?;
    let after = obj[idx + needle.len()..].trim_start();
    if !after.starts_with('[') {
        return None;
    }
    let end = find_matching_bracket(after, '[', ']')?;
    let mut rest = after[1..end].trim_start();
    let mut out = Vec::new();
    while !rest.is_empty() {
        if rest.starts_with(',') {
            rest = rest[1..].trim_start();
            continue;
        }
        if !rest.starts_with('{') {
            return None;
        }
        let obj_end = find_matching_bracket(rest, '{', '}')?;
        if let Some(name) = quoted_field_in_object(&rest[..=obj_end], "name") {
            out.push(name);
        }
        rest = rest[obj_end + 1..].trim_start();
        if rest.starts_with(',') {
            rest = rest[1..].trim_start();
        } else if !rest.is_empty() {
            return None;
        }
    }
    Some(out)
}

/// `FODDER.flatMap(... ITEM_DB.find ... Math.floor(rec.cost * ALCH_RATE|0.6) ...).sort(...)`.
fn high_alchemy_fodder_ident<'a>(file_src: &str, rhs: &'a str) -> Option<&'a str> {
    let rhs = rhs.trim_start();
    let (fodder, rest) = take_ident(rhs)?;
    if fodder == "ITEM_DB" || fodder == "Object" {
        return None;
    }
    if !rest.trim_start().starts_with('.') {
        return None;
    }
    let compact: String = rhs.chars().filter(|c| !c.is_whitespace()).collect();
    if !compact.contains(".flatMap(") {
        return None;
    }
    if !compact.contains("ITEM_DB.find") {
        return None;
    }
    if !compact.contains(".obj===") && !compact.contains(".obj==") {
        return None;
    }
    if !compact.contains("Math.floor(") && !compact.contains("alchValueOf(") {
        return None;
    }
    if !compact.contains(".sort(") {
        return None;
    }
    if !flat_map_mentions_obj(&compact) {
        return None;
    }
    if !high_alchemy_rate_operand(file_src, &compact)
        && !high_alchemy_alch_value_of(&compact, file_src)
    {
        return None;
    }
    Some(fodder)
}

/// Frozen `AlcherLogic.ts` uses `alchValueOf(rec.cost)` (default `HIGH_ALCH_RATE`).
fn high_alchemy_alch_value_of(compact: &str, file_src: &str) -> bool {
    if !compact.contains("alchValueOf(rec.cost") {
        return false;
    }
    if compact.contains("alchValueOf(rec.cost,HIGH_ALCH_RATE)") {
        let Some(rhs) = const_eq_rhs(file_src, "HIGH_ALCH_RATE") else {
            return false;
        };
        return number_is_point_six(rhs.trim_start());
    }
    if compact.contains("alchValueOf(rec.cost)") {
        let Some(rhs) = const_eq_rhs(file_src, "HIGH_ALCH_RATE") else {
            return const_eq_rhs(file_src, "ALCH_RATE")
                .map(|r| number_is_point_six(r.trim_start()))
                .unwrap_or(false);
        };
        return number_is_point_six(rhs.trim_start());
    }
    false
}

fn flat_map_mentions_obj(compact: &str) -> bool {
    let Some(start) = compact.find(".flatMap(") else {
        return false;
    };
    let after = &compact[start + ".flatMap(".len()..];
    after.contains("obj")
}

fn high_alchemy_rate_operand(file_src: &str, compact: &str) -> bool {
    if compact.contains(".cost*0.6") {
        return rate_token_is_point_six(compact);
    }
    if compact.contains(".cost*ALCH_RATE") {
        let Some(rhs) = const_eq_rhs(file_src, "ALCH_RATE") else {
            return false;
        };
        return number_is_point_six(rhs.trim_start());
    }
    false
}

fn rate_token_is_point_six(compact: &str) -> bool {
    let Some(idx) = compact.find(".cost*0.6") else {
        return false;
    };
    number_is_point_six(&compact[idx + ".cost*".len()..])
}

fn number_is_point_six(s: &str) -> bool {
    let s = s.trim_start();
    let Some(rest) = s.strip_prefix("0.6") else {
        return false;
    };
    !rest.starts_with(|c: char| c.is_ascii_digit())
}

fn parse_fodder_candidates(rhs: &str) -> Option<Vec<ItemOptionCandidate>> {
    let s = rhs.trim_start();
    if !s.starts_with('[') {
        return None;
    }
    let end = find_matching_bracket(s, '[', ']')?;
    let mut rest = s[1..end].trim_start();
    let mut out = Vec::new();
    while !rest.is_empty() {
        if rest.starts_with(',') {
            rest = rest[1..].trim_start();
            continue;
        }
        if !rest.starts_with('{') {
            return None;
        }
        let obj_end = find_matching_bracket(rest, '{', '}')?;
        out.push(parse_fodder_object(&rest[..=obj_end])?);
        rest = rest[obj_end + 1..].trim_start();
        if rest.starts_with(',') {
            rest = rest[1..].trim_start();
        } else if rest.is_empty() {
            break;
        } else {
            return None;
        }
    }
    Some(out)
}

fn parse_fodder_object(obj: &str) -> Option<ItemOptionCandidate> {
    let inner = obj.trim();
    let inner = inner.strip_prefix('{')?;
    let inner = inner.strip_suffix('}')?;
    let mut rest = inner.trim();
    let mut key = None;
    let mut label = None;
    while !rest.is_empty() {
        let (ident, after) = take_ident(rest)?;
        let after = after.trim_start().strip_prefix(':')?.trim_start();
        let (value, n) = scan_quoted(after)?;
        match ident {
            "obj" if key.is_none() => key = Some(value),
            "label" if label.is_none() => label = Some(value),
            _ => return None,
        }
        rest = after[n..].trim_start();
        if rest.starts_with(',') {
            rest = rest[1..].trim_start();
        } else if rest.is_empty() {
            break;
        } else {
            return None;
        }
    }
    Some(ItemOptionCandidate {
        key: key.filter(|k| !k.is_empty())?,
        label,
    })
}

#[cfg(test)]
mod settings_extraction_tests {
    use super::*;

    fn ids(schema: &[SettingDef]) -> Vec<&str> {
        schema.iter().map(|s| s.id.as_str()).collect()
    }

    fn setting<'a>(schema: &'a [SettingDef], id: &str) -> &'a SettingDef {
        schema
            .iter()
            .find(|s| s.id == id)
            .unwrap_or_else(|| panic!("missing setting {id} in {:?}", ids(schema)))
    }

    #[test]
    fn rockcrab_from_entries_keeps_bank_keys_and_trailing_solve_clues() {
        let src = r#"
export const SETTINGS = {
    combatStyle: { type: 'string', default: 'melee', options: ['melee', 'mage', 'range'] },
    loadout: LOADOUT_SETTING,
    ...Object.fromEntries(Object.entries(PERIODIC_BANK_SETTINGS).map(([key, def]) => [key, { ...def, group: 'Banking & loot' }])),
    solveClues: { type: 'boolean', default: true, label: 'Solve easy clues', group: 'Clues' }
};
"#;
        let schema = settings_schema_from_source(src);
        let found = ids(&schema);
        for id in [
            "combatStyle",
            "loadout",
            "bankStrategy",
            "bankEveryItems",
            "bankEveryMinutes",
            "bankCommonJunk",
            "solveClues",
        ] {
            assert!(found.contains(&id), "lost {id}: {found:?}");
        }
        assert!(
            !found.contains(&"group"),
            "fromEntries leftover must not invent group: {found:?}"
        );
        let bank = setting(&schema, "bankStrategy");
        assert_eq!(bank.options, ["Off", "Loot count", "Time", "Either"]);
        assert_eq!(
            setting(&schema, "solveClues").default.as_deref(),
            Some("true")
        );
        assert_eq!(
            setting(&schema, "loadout").options_from.as_deref(),
            Some("loadouts")
        );
    }

    #[test]
    fn unknown_or_malformed_spread_skips_one_expression() {
        let src = r#"
export const SETTINGS = {
    before: { type: 'boolean', default: true },
    ...Unknown.fromEntries({ group: 'poison', later: { type: 'string' } }),
    ...NOT_A_REAL_SETTINGS,
    after: { type: 'boolean', default: false, label: 'survives' }
};
"#;
        let schema = settings_schema_from_source(src);
        let found = ids(&schema);
        assert_eq!(
            found,
            ["before", "after"],
            "spread must not swallow later fields or invent ids: {found:?}"
        );
        assert_eq!(setting(&schema, "after").default.as_deref(), Some("false"));
    }

    #[test]
    fn cookbot_location_surface_and_log_options_are_usable() {
        let src = r#"
export const SURFACE_OPTIONS = ['Range', 'Fire'] as const;
export const SETTINGS = {
    location: { type: 'string', default: 'Catherby', options: [...COOK_LOCATION_OPTIONS] },
    surface: { type: 'string', default: 'Range', options: [...SURFACE_OPTIONS] },
    logType: { type: 'string', default: 'Logs', options: Object.keys(LOG_LEVELS) },
    fireSpot: { type: 'string', default: 'Varrock East', options: Object.keys(FIRE_SPOTS) }
};
"#;
        let schema = settings_schema_from_source(src);
        let location = setting(&schema, "location");
        assert!(location.options.contains(&"Auto".into()));
        assert!(location.options.contains(&"Catherby".into()));
        assert!(location.options.contains(&"Custom".into()));
        assert_eq!(setting(&schema, "surface").options, ["Range", "Fire"]);
        assert_eq!(
            setting(&schema, "logType").options,
            [
                "Logs",
                "Oak logs",
                "Willow logs",
                "Maple logs",
                "Yew logs",
                "Magic logs"
            ]
        );
        assert_eq!(
            setting(&schema, "fireSpot").options,
            ["Varrock East", "Varrock West", "Draynor", "Seers"]
        );
    }

    #[test]
    fn source_backed_mapped_keys_spreads_and_labels() {
        let src = r#"
export const BEST_AVAILABLE = 'Best available';
export const CUSTOM = 'Custom';
export const PICK_TIERS = [
    { tier: 'Rune', item: 'Rune pickaxe', level: 41 },
    { tier: 'Bronze', item: 'Bronze pickaxe', level: 1 }
];
export const PICK_OPTIONS = [BEST_AVAILABLE, ...PICK_TIERS.map(t => t.tier)];
export const HERBS = [
    { key: 'guam', name: 'Guam leaf', id: 249 },
    { key: 'ranarr', name: 'Ranarr weed', id: 257 }
];
export const HERB_OPTIONS = HERBS.map(h => h.name);
export const SECONDARIES = [
    { id: 'eggs', name: "Red spiders' eggs" },
    { id: 'newt', name: 'Eye of newt' }
];
export const SECONDARY_OPTIONS = SECONDARIES.map(s => s.name);
export const RECIPES = [
    { bar: 'Bronze', level: 1 },
    { bar: 'Iron', level: 15 }
];
const BLURITE = { bar: 'Blurite', level: 13 };
export const BAR_OPTIONS = [...RECIPES.map(r => r.bar), BLURITE.bar];
export const GEMS = [
    { key: 'sapphire', name: 'Sapphire' },
    { key: 'ruby', name: 'Ruby' }
];
export const GEM_OPTIONS = GEMS.map(g => g.name);
export const RUNES = {
    'Nature runes': { rune: 'Nature rune' },
    'Air runes': { rune: 'Air rune' }
};
export const RUNE_OPTIONS = Object.keys(RUNES);
const LEATHERS = {
    Leather: { leatherId: 1741 },
    'Hard leather': { leatherId: 1743 }
};
export const SHOP_PRESETS = [
    { label: "Aemad's vials — East Ardougne (Ardougne East bank)", keeper: 'Aemad', shopStand: new Tile(2613, 3294, 0) },
    { label: 'Wizard Guild runes — Yanille (Yanille bank)', keeper: 'Magic Store owner' }
];
export const NEAREST_BANK = 'Nearest';
export const SETTINGS = {
    bank: { type: 'string', default: NEAREST_BANK, options: [NEAREST_BANK, ...BANK_LOCATIONS.map(b => b.name)] },
    pickaxe: { type: 'string', options: PICK_OPTIONS },
    herbs: { type: 'string[]', options: [...HERB_OPTIONS, CUSTOM] },
    secondary: { type: 'string', options: SECONDARY_OPTIONS },
    bar: { type: 'string', options: [...BAR_OPTIONS] },
    gems: { type: 'string[]', options: GEM_OPTIONS },
    rune: { type: 'string', options: RUNE_OPTIONS },
    leatherType: { type: 'string', options: Object.keys(LEATHERS) },
    shop: { type: 'string', options: SHOP_PRESETS.map(p => p.label) },
    jiveProduct: { type: 'string', options: PRODUCT_OPTIONS },
    staff: { type: 'string', default: 'Staff of air', options: STAFFS }
};
"#;
        let schema = settings_schema_from_source(src);
        let bank = setting(&schema, "bank");
        assert_eq!(bank.options[0], "Nearest");
        assert!(
            bank.options.contains(&"Catherby".into()),
            "{:?}",
            bank.options
        );
        assert_eq!(
            setting(&schema, "pickaxe").options,
            ["Best available", "Rune", "Bronze"]
        );
        assert_eq!(
            setting(&schema, "herbs").options,
            ["Guam leaf", "Ranarr weed", "Custom"]
        );
        assert_eq!(
            setting(&schema, "secondary").options,
            ["Red spiders' eggs", "Eye of newt"]
        );
        assert_eq!(
            setting(&schema, "bar").options,
            ["Bronze", "Iron", "Blurite"]
        );
        assert_eq!(setting(&schema, "gems").options, ["Sapphire", "Ruby"]);
        assert_eq!(
            setting(&schema, "rune").options,
            ["Nature runes", "Air runes"]
        );
        assert_eq!(
            setting(&schema, "leatherType").options,
            ["Leather", "Hard leather"]
        );
        assert_eq!(
            setting(&schema, "shop").options,
            [
                "Aemad's vials — East Ardougne (Ardougne East bank)",
                "Wizard Guild runes — Yanille (Yanille bank)"
            ]
        );
        assert!(
            setting(&schema, "jiveProduct")
                .options
                .contains(&"Gold ring".into()),
            "constructor-built PRODUCT_OPTIONS uses frozen metadata"
        );
        let staff = setting(&schema, "staff");
        assert!(
            staff.options.is_empty(),
            "W1 equipment must stay unpublished at parse time: {:?}",
            staff.options
        );
        assert_eq!(staff.options_from.as_deref(), Some("STAFFS"));
    }

    #[test]
    fn w1c_equipment_union_idents_match_frozen_ranged_ts() {
        assert_eq!(
            super::w1c_equipment_option_families("RANGED_WEAPONS"),
            Some(&["bows", "darts"][..])
        );
        assert_eq!(
            super::w1c_equipment_option_families("ROCK_CRAB_RANGED_WEAPONS"),
            Some(&["bows", "darts"][..])
        );
        assert!(super::w1c_equipment_option_families("AXES").is_none());
        assert!(super::w1c_equipment_option_families("DROP_DB").is_none());
    }

    #[test]
    fn preset_buyable_names_dedupes_and_sorts() {
        let src = r#"
export const SHOP_PRESETS = [
    { label: 'A', keeper: 'Aemad' },
    { label: 'B', keeper: 'Lowe' }
];
export const SHOP_DB = {
    "adventurershop": {"keepers":["Aemad"],"items":[{"name":"Vial of water"},{"name":"Feather"}]},
    "archeryshop": {"keepers":["Lowe"],"items":[{"name":"Bronze arrow"},{"name":"Vial of water"}]}
};
export const SETTINGS = {
    buyItems: { type: 'string[]', default: [], options: presetBuyableNames() }
};
"#;
        let schema = settings_schema_from_source(src);
        let buy = setting(&schema, "buyItems");
        assert_eq!(
            buy.options,
            ["Bronze arrow", "Feather", "Vial of water"],
            "union sorted; duplicate Vial of water once"
        );
    }

    #[test]
    fn preset_buyable_names_malformed_shop_db_stays_empty() {
        let src = r#"
export const SHOP_PRESETS = [{ label: 'A', keeper: 'Nobody' }];
export const SHOP_DB = { broken: true };
export const SETTINGS = {
    buyItems: { type: 'string[]', default: [], options: presetBuyableNames() }
};
"#;
        let schema = settings_schema_from_source(src);
        let buy = setting(&schema, "buyItems");
        assert!(buy.options.is_empty(), "unknown keeper must not fabricate names");
    }

    #[test]
    fn frozen_alch_sort_wrapper_item_option_spec() {
        let src = r#"
const ALCH_RATE = 0.6;
const FODDER = [{ obj: 'maple_longbow' }, { obj: 'yew_longbow', label: 'Yew longbow' }];
export const CUSTOM_ALCH_KEY = 'custom';
export const ALCH_ITEMS = FODDER.flatMap(({ obj, label }) => {
    const rec = ITEM_DB.find(r => r.obj === obj);
    return rec ? [{ key: obj, id: rec.id, name: rec.name, label: label ?? rec.name, alchValue: Math.floor(rec.cost * ALCH_RATE) }] : [];
}).sort((a, b) => b.alchValue - a.alchValue);
export const ALCH_OPTIONS = [
    CUSTOM_ALCH_KEY,
    ...[...ALCH_ITEMS].sort((a, b) => a.label.localeCompare(b.label)).map(i => i.key)
];
export const SETTINGS = {
    items: { type: 'string[]', default: [], options: ALCH_OPTIONS }
};
"#;
        let schema = settings_schema_from_source(src);
        let items = setting(&schema, "items");
        assert!(items.options.is_empty());
        let spec = items.item_option_spec.as_ref().expect("sort-wrapped spec");
        assert_eq!(spec.prefix, vec!["custom".to_string()]);
        assert!(spec.sort_keys_by_label);
        assert_eq!(spec.candidates.len(), 2);
    }

    #[test]
    fn computed_alch_map_key_stays_unresolved() {
        let src = r#"
export const CUSTOM_ALCH_KEY = 'custom';
export const ALCH_ITEMS = [];
export const ALCH_OPTIONS = [CUSTOM_ALCH_KEY, ...ALCH_ITEMS.map(i => i.key)];
export const SETTINGS = {
    items: { type: 'string[]', default: ['steel_platebody'], options: ALCH_OPTIONS }
};
"#;
        let schema = settings_schema_from_source(src);
        let items = setting(&schema, "items");
        assert!(
            items.options.is_empty(),
            "ALCH .key map must stay unresolved, got {:?}",
            items.options
        );
        assert!(items.item_option_spec.is_none());
    }

    #[test]
    fn unsupported_alch_wrapper_shapes_stay_unresolved() {
        let fodder = r#"
const ALCH_RATE = 0.6;
const FODDER = [{ obj: 'maple_longbow' }, { obj: 'yew_longbow', label: 'Yew longbow' }];
export const CUSTOM_ALCH_KEY = 'custom';
export const ALCH_ITEMS = FODDER.flatMap(({ obj, label }) => {
    const rec = ITEM_DB.find(r => r.obj === obj);
    return rec ? [{ key: obj, id: rec.id, name: rec.name, label: label ?? rec.name, alchValue: Math.floor(rec.cost * ALCH_RATE) }] : [];
}).sort((a, b) => b.alchValue - a.alchValue);
"#;
        let filter = format!(
            r#"{fodder}
export const ALCH_OPTIONS = [CUSTOM_ALCH_KEY, ...[...ALCH_ITEMS.filter(x => true)].sort((a,b)=>a.label.localeCompare(b.label)).map(i=>i.key)];
export const SETTINGS = {{
    items: {{ type: 'string[]', default: [], options: ALCH_OPTIONS }},
    later: {{ type: 'string', options: ['KeepMe'] }}
}};
"#
        );
        let schema = settings_schema_from_source(&filter);
        let items = setting(&schema, "items");
        assert!(items.options.is_empty());
        assert!(
            items.item_option_spec.is_none(),
            "filter remainder must not parse as ALCH_ITEMS: {:?}",
            items.item_option_spec
        );
        assert_eq!(setting(&schema, "later").options, ["KeepMe"]);

        let concat = format!(
            r#"{fodder}
export const ALCH_OPTIONS = [CUSTOM_ALCH_KEY, ...[...ALCH_ITEMS].sort((a,b)=>a.label.localeCompare(b.label)).map(i=>i.key)].concat(evil);
export const SETTINGS = {{
    items: {{ type: 'string[]', default: [], options: ALCH_OPTIONS }},
    later: {{ type: 'string', options: ['KeepMe'] }}
}};
"#
        );
        let schema = settings_schema_from_source(&concat);
        let items = setting(&schema, "items");
        assert!(
            items.item_option_spec.is_none(),
            "trailing .concat must stay unresolved"
        );
        assert_eq!(setting(&schema, "later").options, ["KeepMe"]);
    }

    #[test]
    fn preset_buyable_names_requires_empty_call_and_full_expression() {
        let db = r#"
export const SHOP_PRESETS = [{ label: 'A', keeper: 'Aemad' }];
export const SHOP_DB = {
    "adventurershop": {"keepers":["Aemad"],"items":[{"name":"Vial of water"}]}
};
"#;
        let args = format!(
            r#"{db}
export const SETTINGS = {{
    buyItems: {{ type: 'string[]', default: [], options: presetBuyableNames(x) }},
    later: {{ type: 'string', options: ['KeepMe'] }}
}};
"#
        );
        let schema = settings_schema_from_source(&args);
        assert!(
            setting(&schema, "buyItems").options.is_empty(),
            "presetBuyableNames(x) must stay unresolved"
        );
        assert_eq!(setting(&schema, "later").options, ["KeepMe"]);

        let concat = format!(
            r#"{db}
export const SETTINGS = {{
    buyItems: {{ type: 'string[]', default: [], options: presetBuyableNames().concat(x) }},
    later: {{ type: 'string', options: ['KeepMe'] }}
}};
"#
        );
        let schema = settings_schema_from_source(&concat);
        assert!(
            setting(&schema, "buyItems").options.is_empty(),
            "presetBuyableNames().concat must stay unresolved"
        );
        assert_eq!(setting(&schema, "later").options, ["KeepMe"]);
    }

    #[test]
    fn incomplete_shop_db_parse_stays_empty() {
        let src = r#"
export const SHOP_PRESETS = [{ label: 'A', keeper: 'Aemad' }];
export const SHOP_DB = {
    "adventurershop": {"keepers":["Aemad"],"items":[{"name":"Vial of water"}]},
    junk
};
export const SETTINGS = {
    buyItems: { type: 'string[]', default: [], options: presetBuyableNames() }
};
"#;
        let schema = settings_schema_from_source(src);
        assert!(
            setting(&schema, "buyItems").options.is_empty(),
            "partial SHOP_DB must not publish a keeper union"
        );
    }

    #[test]
    fn shop_db_sources_use_explicit_keys_and_never_ambient() {
        let iso = crate::IsolatedEnv::enter("shopdb-parser-ambient");
        let foreign = iso.dir.join("foreign-root");
        std::fs::create_dir_all(foreign.join("src/bot/data")).unwrap();
        std::fs::write(
            foreign.join("src/bot/data/shopdb.ts"),
            r#"export const SHOP_DB = {
    "adventurershop": {"keepers":["Aemad"],"items":[{"name":"ZZZ_FOREIGN_TONIC"}]}
};
"#,
        )
        .unwrap();
        iso.set_rs2b0t(&foreign);
        persist_rs2b0t_root(&foreign).expect("persist foreign root");

        let shop = r#"
import { presetBuyableNames } from './shopPresets.js';
export const SHOP_PRESETS = [{ label: 'A', keeper: 'Aemad' }];
export const SETTINGS = {
    buyItems: { type: 'string[]', default: [], options: presetBuyableNames() }
};
"#;
        let isolated = settings_schema_from_source(shop);
        assert!(
            setting(&isolated, "buyItems").options.is_empty(),
            "source-only parse must not read ambient shopdb: {:?}",
            setting(&isolated, "buyItems").options
        );

        let mut empty = HashMap::new();
        empty.insert("./ShopBuyout/ShopBuyout.js".into(), shop.to_string());
        let cards = parse_registry_with_sources(
            r#"
import ShopBuyout, { SETTINGS } from './ShopBuyout/ShopBuyout.js';
ScriptRegistry.register({ name: 'ShopBuyout', settingsSchema: SETTINGS, create: () => new ShopBuyout() });
"#,
            &empty,
        )
        .expect("partial sources parse");
        assert!(
            setting(&cards[0].settings_schema, "buyItems")
                .options
                .is_empty(),
            "Some(partial) must not borrow another catalog: {:?}",
            setting(&cards[0].settings_schema, "buyItems").options
        );

        let mut decoy = empty.clone();
        decoy.insert(
            "./injected/shopdb-extra.ts".into(),
            r#"export const SHOP_DB = {
    "adventurershop": {"keepers":["Aemad"],"items":[{"name":"Wrong Name"}]}
};
"#
            .into(),
        );
        let decoy_cards = parse_registry_with_sources(
            r#"
import ShopBuyout, { SETTINGS } from './ShopBuyout/ShopBuyout.js';
ScriptRegistry.register({ name: 'ShopBuyout', settingsSchema: SETTINGS, create: () => new ShopBuyout() });
"#,
            &decoy,
        )
        .expect("decoy sources parse");
        assert!(
            setting(&decoy_cards[0].settings_schema, "buyItems")
                .options
                .is_empty(),
            "contains(shopdb) first-hit must not select a non-explicit key"
        );

        let mut exact = empty;
        exact.insert(
            "./src/bot/data/shopdb.ts".into(),
            r#"export const SHOP_DB = {
    "adventurershop": {"keepers":["Aemad"],"items":[{"name":"Right Name"}]}
};
"#
            .into(),
        );
        let exact_cards = parse_registry_with_sources(
            r#"
import ShopBuyout, { SETTINGS } from './ShopBuyout/ShopBuyout.js';
ScriptRegistry.register({ name: 'ShopBuyout', settingsSchema: SETTINGS, create: () => new ShopBuyout() });
"#,
            &exact,
        )
        .expect("exact shopdb key parses");
        assert_eq!(
            setting(&exact_cards[0].settings_schema, "buyItems").options,
            ["Right Name"]
        );
    }

    /// Bounded frozen same-dir walk. Not UI proof. Requires `$RS2B0T`.
    #[test]
    #[ignore = "requires absolute RS2B0T frozen catalog"]
    fn frozen_catalog_settings_audit() {
        let root = PathBuf::from(
            std::env::var("RS2B0T").expect("RS2B0T must name the frozen catalog root"),
        );
        assert!(root.is_absolute(), "RS2B0T must be absolute");
        let index_path = registry_index_path(&root);
        let index = std::fs::read_to_string(&index_path)
            .unwrap_or_else(|e| panic!("read {}: {e}", index_path.display()));
        let skeleton = parse_registry_with_sources(&index, &HashMap::new())
            .expect("frozen index parses without sources");
        let mut sources = HashMap::new();
        for card in &skeleton {
            let Some(path) = script_file_path(&root, &card.rel_path) else {
                continue;
            };
            if let Ok(text) = std::fs::read_to_string(&path) {
                sources.insert(card.rel_path.clone(), text);
            }
            let Some(dir) = path.parent() else {
                continue;
            };
            let Some(card_dir) = rel_dir(&card.rel_path) else {
                continue;
            };
            let Ok(entries) = std::fs::read_dir(dir) else {
                continue;
            };
            for ent in entries.flatten() {
                let name = ent.file_name();
                let name = name.to_string_lossy();
                if !(name.ends_with(".ts") || name.ends_with(".js")) {
                    continue;
                }
                let Ok(text) = std::fs::read_to_string(ent.path()) else {
                    continue;
                };
                let rel_native = format!("./{card_dir}/{name}");
                sources.entry(rel_native).or_insert_with(|| text.clone());
                if let Some(stem) = name.strip_suffix(".ts") {
                    sources
                        .entry(format!("./{card_dir}/{stem}.js"))
                        .or_insert(text);
                }
            }
        }
        let shopdb_path = root.join("src/bot/data/shopdb.ts");
        if let Ok(text) = std::fs::read_to_string(&shopdb_path) {
            sources
                .entry("./src/bot/data/shopdb.ts".into())
                .or_insert_with(|| text.clone());
            sources
                .entry("./src/bot/data/shopdb.js".into())
                .or_insert(text);
        }
        let cards = parse_registry_with_sources(&index, &sources).expect("frozen catalog parses");
        let by_name: HashMap<&str, &RegistryCard> =
            cards.iter().map(|c| (c.name.as_str(), c)).collect();

        let want = [
            ("JiveCrafting", "product"),
            ("JiveEnchanter", "jewel"),
            ("JiveMarketDumper", "bank"),
            ("EssMiner", "pickaxe"),
            ("RuneCrafter", "rune"),
            ("NatureCrafter", "rune"),
            ("CookBot", "location"),
            ("CookBot", "surface"),
            ("CookBot", "logType"),
            ("DartFletcher", "tier"),
            ("HerbloreSecondaries", "secondary"),
            ("HerbCleaner", "herbs"),
            ("PotionMaker", "herb"),
            ("PotionMaker", "secondary"),
            ("SmelterBot", "bar"),
            ("Superheater", "bar"),
            ("Alcher", "items"),
            ("GemCutter", "gems"),
            ("MuleCrafter", "rune"),
            ("ShopBuyout", "shop"),
            ("ShopBuyout", "buyItems"),
            ("LeatherCrafter", "leatherType"),
            ("Firemaker", "logType"),
            ("Firemaker", "location"),
        ];
        let mut recovered = 0usize;
        let mut empty = 0usize;
        for (card_name, setting_id) in want {
            let card = by_name
                .get(card_name)
                .unwrap_or_else(|| panic!("missing catalog card {card_name}"));
            let def = card
                .settings_schema
                .iter()
                .find(|s| s.id == setting_id)
                .unwrap_or_else(|| panic!("{card_name}.{setting_id} missing from schema"));
            eprintln!(
                "AUDIT {card_name}.{setting_id} n={} spec={} from={:?} opts={:?}",
                def.options.len(),
                def.item_option_spec.is_some(),
                def.options_from,
                def.options
            );
            if def.options.is_empty() && def.item_option_spec.is_none() {
                empty += 1;
            } else {
                recovered += 1;
            }
        }

        let rock = by_name.get("RockCrab").expect("RockCrab card");
        let rock_ids: Vec<&str> = rock.settings_schema.iter().map(|s| s.id.as_str()).collect();
        eprintln!("AUDIT RockCrab ids={rock_ids:?}");
        for id in [
            "bankStrategy",
            "bankEveryItems",
            "bankEveryMinutes",
            "bankCommonJunk",
            "solveClues",
        ] {
            assert!(rock_ids.contains(&id), "RockCrab lost {id}: {rock_ids:?}");
        }
        assert!(
            !rock_ids.contains(&"group"),
            "RockCrab must not invent group: {rock_ids:?}"
        );

        let rune_crafter = setting(&by_name["RuneCrafter"].settings_schema, "rune");
        assert_eq!(
            rune_crafter.options,
            ["Air runes", "Earth runes"],
            "RuneCrafter local Object.keys must win over mule altar table: {:?}",
            rune_crafter.options
        );
        let nature = setting(&by_name["NatureCrafter"].settings_schema, "rune");
        assert_eq!(
            nature.options,
            ["Nature runes", "Air runes"],
            "NatureCrafter local keys must win: {:?}",
            nature.options
        );
        let mule = setting(&by_name["MuleCrafter"].settings_schema, "rune");
        assert_eq!(
            mule.options,
            catalog_option_values("RUNE_OPTIONS").unwrap(),
            "MuleCrafter re-export uses imported 11 altar names: {:?}",
            mule.options
        );

        let cook_loc = setting(&by_name["CookBot"].settings_schema, "location");
        assert!(cook_loc.options.contains(&"Catherby".into()));
        assert_eq!(
            setting(&by_name["CookBot"].settings_schema, "surface").options,
            ["Range", "Fire"]
        );
        assert!(!setting(&by_name["CookBot"].settings_schema, "logType")
            .options
            .is_empty());

        let potion_herbs = &setting(&by_name["PotionMaker"].settings_schema, "herb").options;
        assert_eq!(
            potion_herbs.len(),
            15,
            "PotionMaker HERBS is 14 through Torstol plus CUSTOM: {potion_herbs:?}"
        );
        assert_eq!(potion_herbs.last().map(String::as_str), Some("Custom"));

        let alcher = setting(&by_name["Alcher"].settings_schema, "items");
        assert!(
            alcher.options.is_empty(),
            "Alcher.items keys must not bake into options: {:?}",
            alcher.options
        );
        let spec = alcher
            .item_option_spec
            .as_ref()
            .expect("Alcher.items sort-wrapped item_option_spec");
        assert_eq!(spec.prefix, vec!["custom".to_string()]);
        assert!(
            spec.sort_keys_by_label,
            "frozen ALCH_OPTIONS uses label sort before .map(i => i.key)"
        );
        assert!(
            !spec.candidates.is_empty(),
            "Alcher FODDER chain must yield candidates"
        );
        let shop = setting(&by_name["ShopBuyout"].settings_schema, "shop");
        assert!(
            shop.options.iter().any(|s| s.contains("Aemad")),
            "ShopBuyout.shop SHOP_PRESETS.map(p => p.label) must emit frozen labels: {:?}",
            shop.options
        );
        let buy = setting(&by_name["ShopBuyout"].settings_schema, "buyItems");
        assert!(
            !buy.options.is_empty(),
            "ShopBuyout.buyItems presetBuyableNames join must emit options: {:?}",
            buy.options
        );
        assert!(buy.item_option_spec.is_none());
        let buy_sorted = buy.options.clone();
        let mut sorted = buy_sorted.clone();
        sorted.sort();
        assert_eq!(
            buy_sorted, sorted,
            "presetBuyableNames must be alphabetically sorted: first={:?} last={:?}",
            buy_sorted.first(),
            buy_sorted.last()
        );
        let buy_lower: Vec<String> = buy.options.iter().map(|s| s.to_ascii_lowercase()).collect();
        assert!(buy_lower.iter().any(|n| n.contains("rune")));
        assert!(buy_lower.iter().any(|n| n.contains("arrow")));
        assert!(buy_lower.contains(&"feather".to_string()));
        assert!(buy_lower.contains(&"vial of water".to_string()));

        eprintln!(
            "AUDIT summary recovered={recovered} empty={empty} of {}",
            want.len()
        );
        assert_eq!(
            recovered + empty,
            want.len(),
            "row accounting {recovered}+{empty}"
        );
        assert_eq!(empty, 0, "all 24 audited option rows must recover");
        assert_eq!(recovered, 24);
    }
}
