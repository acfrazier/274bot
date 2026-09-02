//! `$RS2B0T` registry parse: static scan of `src/bot/scripts/index.ts`.
//! The catalog is never executed here — no rustyscript, no V8 Runtime, no
//! isolate. Register names become the picker names (they may differ from
//! the folder); the matched `import … from './…'` path is the file to read
//! on Start. The first successful parse persists the root so later boots
//! can find the catalog without `$RS2B0T` set.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// One catalog card: the register `name` (picker label) and the `./…`
/// import path of the script file to read on Start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegistryCard {
    pub name: String,
    pub rel_path: String,
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
            std::path::Component::ParentDir | std::path::Component::RootDir
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

/// Default persisted rs2b0t root file (`~/.274bot/rs2b0t-path`).
pub fn default_rs2b0t_path_file() -> PathBuf {
    match std::env::var("HOME") {
        Ok(home) => PathBuf::from(format!("{home}/.274bot/rs2b0t-path")),
        Err(_) => PathBuf::from(".274bot/rs2b0t-path"),
    }
}

/// The rs2b0t checkout root: `$RS2B0T` first, else the path persisted by a
/// previous successful parse.
pub fn rs2b0t_root_at(path_file: &Path) -> Option<PathBuf> {
    if let Ok(root) = std::env::var("RS2B0T") {
        if !root.is_empty() {
            return Some(PathBuf::from(root));
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

/// Static parse of the catalog `index.ts`: each
/// `ScriptRegistry.register({…name: '…'…})` matched to its
/// `import … from './…'` by the `create: () => new X()` constructor
/// ident. No JS runs. Errors when the file carries no `register` call or
/// when no register matched an import.
pub fn parse_registry(index_ts: &str) -> Result<Vec<RegistryCard>, String> {
    let imports = scan_imports(index_ts);
    let registers = scan_registers(index_ts);
    if registers.is_empty() {
        return Err("no ScriptRegistry.register calls".to_string());
    }
    let mut cards = Vec::new();
    for (name, ctor) in registers {
        let Some(path) = imports.get(&ctor) else {
            continue;
        };
        cards.push(RegistryCard {
            name,
            rel_path: path.clone(),
        });
    }
    if cards.is_empty() {
        return Err("register matched no import".to_string());
    }
    Ok(cards)
}

/// `ident -> rel path` for every default and named import of a relative
/// module under `./`. Parent-dir and absolute imports are ignored.
fn scan_imports(src: &str) -> HashMap<String, String> {
    let mut imports = HashMap::new();
    let mut pos = 0;
    while let Some(rel) = src[pos..].find("import") {
        let start = pos + rel;
        let before = src[..start].chars().next_back();
        if !matches!(
            before,
            None | Some('\n') | Some('\r') | Some(' ') | Some('\t') | Some(';') | Some('}')
        ) {
            // "import" inside another token or a comment; skip it.
            pos = start + 1;
            continue;
        }
        let tail = &src[start + "import".len()..];
        let stmt_len = tail.find(';').unwrap_or(tail.len());
        if let Some((idents, path)) = parse_import_stmt(&tail[..stmt_len]) {
            for ident in idents {
                imports.entry(ident).or_insert_with(|| path.clone());
            }
        }
        pos = start + "import".len() + stmt_len;
    }
    imports
}

/// Parse one import statement body (the text after `import`, before `;`).
/// Returns the (ident, rel path) pairs: the default ident when present and
/// every named ident in braces, all mapping to the statement's path.
fn parse_import_stmt(stmt: &str) -> Option<(Vec<String>, String)> {
    let stmt = stmt.trim_start();
    let stmt = stmt.strip_prefix("type ").unwrap_or(stmt);
    let fi = stmt.rfind("from ")?;
    let after = stmt[fi + "from ".len()..].trim_start();
    let path = quoted_after(after)?;
    if !path.starts_with("./") {
        return None;
    }
    let spec = stmt[..fi].trim();
    let mut idents = Vec::new();
    if let Some(rest) = spec.strip_prefix('{') {
        let inner = rest.strip_suffix('}').unwrap_or(rest);
        for part in inner.split(',') {
            let part = part.trim();
            if part.is_empty() {
                continue;
            }
            let ident = part.split_whitespace().next().unwrap_or(part);
            idents.push(ident.to_string());
        }
    } else {
        let group = spec.find('{');
        let head = group.map_or(spec, |i| &spec[..i]);
        if let Some(ident) = head
            .split(|c: char| c.is_whitespace() || c == ',')
            .find(|t| !t.is_empty())
        {
            idents.push(ident.to_string());
        }
        if let Some(i) = group {
            if let Some(j) = spec.rfind('}') {
                for part in spec[i + 1..j].split(',') {
                    let part = part.trim();
                    if part.is_empty() {
                        continue;
                    }
                    let ident = part.split_whitespace().next().unwrap_or(part);
                    idents.push(ident.to_string());
                }
            }
        }
    }
    if idents.is_empty() {
        return None;
    }
    Some((idents, path))
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

/// Every register block as `(name, constructor ident)`, in file order.
fn scan_registers(src: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut pos = 0;
    while let Some(rel) = src[pos..].find("ScriptRegistry.register(") {
        let open = pos + rel + "ScriptRegistry.register(".len();
        let mut i = open;
        while i < src.len() && src[i..].chars().next().unwrap().is_whitespace() {
            i += src[i..].chars().next().unwrap().len_utf8();
        }
        if i >= src.len() || !src[i..].starts_with('{') {
            pos = open;
            continue;
        }
        // Balanced braces, honoring quoted strings and escapes.
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
                    break;
                }
            }
            j += c.len_utf8();
        }
        let block = &src[i..=j];
        let name = scan_key_quoted(block, "name").unwrap_or_default();
        let ctor = scan_new_after(block, "create").unwrap_or_default();
        out.push((name, ctor));
        pos = j + 1;
    }
    out
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
