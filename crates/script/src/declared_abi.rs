//! Names-only parse of the published rs2b0t-api `index.d.ts`.
//! This is the **JS catalog ABI**, not the Rust host API.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// One runtime export from `packages/rs2b0t-api/index.d.ts`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeclaredExport {
    pub name: String,
    pub kind: DeclaredKind,
    pub members: Vec<String>,
}

/// Runtime shape of a declared export (not a TypeScript `type`/`interface`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DeclaredKind {
    Object,
    Class,
    Function,
    Value,
}

/// `crates/script/tests/fixtures/js_declared_abi.json`
pub fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/js_declared_abi.json")
}

pub fn load_fixture() -> Result<Vec<DeclaredExport>, String> {
    load_fixture_from(&fixture_path())
}

pub fn load_fixture_from(path: &Path) -> Result<Vec<DeclaredExport>, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    serde_json::from_slice(&bytes).map_err(|e| format!("parse {}: {e}", path.display()))
}

pub fn write_fixture(exports: &[DeclaredExport]) -> Result<(), String> {
    write_fixture_to(&fixture_path(), exports)
}

pub fn write_fixture_to(path: &Path, exports: &[DeclaredExport]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("mkdir {}: {e}", parent.display()))?;
    }
    let mut ordered = exports.to_vec();
    normalize_exports(&mut ordered);
    let mut bytes = serde_json::to_vec_pretty(&ordered).map_err(|e| e.to_string())?;
    bytes.push(b'\n');
    std::fs::write(path, bytes).map_err(|e| format!("write {}: {e}", path.display()))?;
    Ok(())
}

fn normalize_exports(exports: &mut [DeclaredExport]) {
    for e in exports.iter_mut() {
        e.members.sort();
        e.members.dedup();
    }
    exports.sort_by(|a, b| a.name.cmp(&b.name));
}

/// Parse published `.d.ts` for runtime export names. Skips `export type` /
/// `export interface` and `@internal` members.
pub fn parse_index_dts(src: &str) -> Vec<DeclaredExport> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < src.len() {
        i = skip_trivia(src, i);
        if i >= src.len() {
            break;
        }
        if !src[i..].starts_with("export ") {
            i += 1;
            continue;
        }
        if let Some((exp, next)) = parse_export(src, i) {
            out.push(exp);
            i = next;
        } else {
            i += "export ".len();
        }
    }
    normalize_exports(&mut out);
    out
}

fn parse_export(src: &str, start: usize) -> Option<(DeclaredExport, usize)> {
    let rest = &src[start..];
    if !rest.starts_with("export ") {
        return None;
    }
    let mut i = start + "export ".len();
    i = skip_trivia(src, i);
    if src[i..].starts_with("type ") || src[i..].starts_with("interface ") {
        return None;
    }
    if src[i..].starts_with("abstract ") {
        i = skip_trivia(src, i + "abstract ".len());
    }
    if src[i..].starts_with("class ") {
        i = skip_trivia(src, i + "class ".len());
        let (name, after_name) = ident_at(src, i)?;
        let brace = find_char(src, after_name, '{')?;
        let end = match_brace(src, brace)?;
        let members = parse_members(src, brace + 1, end);
        return Some((
            DeclaredExport {
                name,
                kind: DeclaredKind::Class,
                members,
            },
            end + 1,
        ));
    }
    if src[i..].starts_with("function ") {
        i = skip_trivia(src, i + "function ".len());
        let (name, after_name) = ident_at(src, i)?;
        let semi = skip_decl_end(src, after_name);
        return Some((
            DeclaredExport {
                name,
                kind: DeclaredKind::Function,
                members: Vec::new(),
            },
            semi,
        ));
    }
    if src[i..].starts_with("const ") {
        i = skip_trivia(src, i + "const ".len());
        let (name, after_name) = ident_at(src, i)?;
        let mut j = skip_trivia(src, after_name);
        if j < src.len() && src.as_bytes()[j] == b':' {
            j = skip_trivia(src, j + 1);
        }
        if j < src.len() && src.as_bytes()[j] == b'{' {
            let end = match_brace(src, j)?;
            let members = parse_members(src, j + 1, end);
            return Some((
                DeclaredExport {
                    name,
                    kind: DeclaredKind::Object,
                    members,
                },
                end + 1,
            ));
        }
        let semi = skip_decl_end(src, after_name);
        return Some((
            DeclaredExport {
                name,
                kind: DeclaredKind::Value,
                members: Vec::new(),
            },
            semi,
        ));
    }
    None
}

fn parse_members(src: &str, start: usize, end: usize) -> Vec<String> {
    let mut members = Vec::new();
    let mut i = start;
    while i < end {
        i = skip_trivia(src, i);
        if i >= end {
            break;
        }
        let member_start = i;
        // Skip modifiers.
        loop {
            i = skip_trivia(src, i);
            if src[i..].starts_with("abstract ") {
                i += "abstract ".len();
                continue;
            }
            if src[i..].starts_with("static ") {
                i += "static ".len();
                continue;
            }
            if src[i..].starts_with("readonly ") {
                i += "readonly ".len();
                continue;
            }
            if src[i..].starts_with("async ") {
                i += "async ".len();
                continue;
            }
            if src[i..].starts_with("get ") || src[i..].starts_with("set ") {
                i += 4;
                continue;
            }
            break;
        }
        i = skip_trivia(src, i);
        if i >= end {
            break;
        }
        if src.as_bytes()[i] == b'}' {
            break;
        }
        let Some((name, after_name)) = ident_at(src, i) else {
            i += 1;
            continue;
        };
        if jsdoc_internal_before(src, start, member_start) {
            i = skip_member_tail(src, after_name, end);
            continue;
        }
        members.push(name);
        i = skip_member_tail(src, after_name, end);
    }
    members
}

fn jsdoc_internal_before(src: &str, block_start: usize, member_start: usize) -> bool {
    let mut i = member_start;
    while i > block_start {
        i -= 1;
        let b = src.as_bytes()[i];
        if b.is_ascii_whitespace() {
            continue;
        }
        if i >= 1 && src.as_bytes()[i] == b'/' && src.as_bytes()[i - 1] == b'*' {
            // Walk back to /*
            let mut j = i - 1;
            while j > block_start {
                if src.as_bytes()[j] == b'*' && j > 0 && src.as_bytes()[j - 1] == b'/' {
                    let comment = &src[j - 1..=i];
                    return comment.contains("@internal");
                }
                j -= 1;
            }
            return false;
        }
        break;
    }
    false
}

fn skip_member_tail(src: &str, mut i: usize, end: usize) -> usize {
    i = skip_trivia(src, i);
    if i < end && src.as_bytes()[i] == b'?' {
        i += 1;
        i = skip_trivia(src, i);
    }
    if i < end && src.as_bytes()[i] == b'<' {
        if let Some(gt) = match_angle(src, i) {
            i = gt + 1;
        }
        i = skip_trivia(src, i);
    }
    if i < end && src.as_bytes()[i] == b'(' {
        if let Some(close) = match_paren(src, i) {
            i = close + 1;
        }
        i = skip_trivia(src, i);
        // Return type may contain `{` / `(` — skip until `;` or next member at depth 0 of this object… 
        // Members are separated by `;` or newline-then-ident. Consume until `;` at paren/brace depth 0.
        return skip_until_semi_or_member(src, i, end);
    }
    if i < end && src.as_bytes()[i] == b':' {
        return skip_until_semi_or_member(src, i + 1, end);
    }
    skip_until_semi_or_member(src, i, end)
}

fn skip_until_semi_or_member(src: &str, mut i: usize, end: usize) -> usize {
    let mut depth_brace = 0i32;
    let mut depth_paren = 0i32;
    let mut depth_angle = 0i32;
    while i < end {
        match src.as_bytes()[i] {
            b'{' => depth_brace += 1,
            b'}' => {
                if depth_brace == 0 {
                    return i;
                }
                depth_brace -= 1;
            }
            b'(' => depth_paren += 1,
            b')' => depth_paren -= 1,
            b'<' => depth_angle += 1,
            b'>' => depth_angle -= 1,
            b';' if depth_brace == 0 && depth_paren == 0 && depth_angle <= 0 => {
                return i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    end
}

fn skip_decl_end(src: &str, mut i: usize) -> usize {
    let mut depth_brace = 0i32;
    let mut depth_paren = 0i32;
    while i < src.len() {
        match src.as_bytes()[i] {
            b'{' => depth_brace += 1,
            b'}' => depth_brace -= 1,
            b'(' => depth_paren += 1,
            b')' => depth_paren -= 1,
            b';' if depth_brace == 0 && depth_paren == 0 => return i + 1,
            _ => {}
        }
        i += 1;
    }
    src.len()
}

fn ident_at(src: &str, i: usize) -> Option<(String, usize)> {
    let bytes = src.as_bytes();
    if i >= bytes.len() {
        return None;
    }
    let c = bytes[i];
    if !c.is_ascii_alphabetic() && c != b'_' {
        return None;
    }
    let mut j = i + 1;
    while j < bytes.len() && (bytes[j].is_ascii_alphanumeric() || bytes[j] == b'_') {
        j += 1;
    }
    Some((src[i..j].to_string(), j))
}

fn skip_trivia(src: &str, mut i: usize) -> usize {
    let bytes = src.as_bytes();
    loop {
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i + 1 < bytes.len() && bytes[i] == b'/' && bytes[i + 1] == b'/' {
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if i + 1 < bytes.len() && bytes[i] == b'/' && bytes[i + 1] == b'*' {
            i += 2;
            while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                i += 1;
            }
            if i + 1 < bytes.len() {
                i += 2;
            }
            continue;
        }
        break;
    }
    i
}

fn find_char(src: &str, mut i: usize, needle: char) -> Option<usize> {
    while i < src.len() {
        i = skip_trivia(src, i);
        if i >= src.len() {
            return None;
        }
        if src[i..].starts_with(needle) {
            return Some(i);
        }
        i += 1;
    }
    None
}

fn match_brace(src: &str, open: usize) -> Option<usize> {
    match_pair(src, open, b'{', b'}')
}

fn match_paren(src: &str, open: usize) -> Option<usize> {
    match_pair(src, open, b'(', b')')
}

fn match_angle(src: &str, open: usize) -> Option<usize> {
    match_pair(src, open, b'<', b'>')
}

fn match_pair(src: &str, open: usize, open_ch: u8, close_ch: u8) -> Option<usize> {
    let bytes = src.as_bytes();
    if open >= bytes.len() || bytes[open] != open_ch {
        return None;
    }
    let mut depth = 0i32;
    let mut i = open;
    while i < bytes.len() {
        if bytes[i] == open_ch {
            depth += 1;
        } else if bytes[i] == close_ch {
            depth -= 1;
            if depth == 0 {
                return Some(i);
            }
        }
        i += 1;
    }
    None
}
