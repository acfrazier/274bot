//! The operator-decision census scanner (test support for
//! `command_census_tests.rs`): a plain-`std`, syntactic scan of this
//! workspace's sources. [`scan`] enumerates (A) every `pub fn` of
//! `frontend-core` that takes a `&mut` parameter or has no `self` receiver
//! at all, and (B) every `fn` item of the `panel` and `tui` crates with its
//! calls into (A) plus a few textual effect probes. Comments and literals
//! are blanked first; braces are tracked; nothing is resolved by type
//! beyond the declared type of a field or parameter. See each public item
//! for the exact rules.

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;
use std::path::PathBuf;

/// Census result: core entries, surface functions, errors and notes.
pub struct Census {
    /// Core `pub fn` entries, sorted by key.
    pub core: Vec<CoreEntry>,
    /// Panel/tui `fn` items, sorted by display path.
    pub surface: Vec<SurfaceFn>,
    /// Problems that make the scan incomplete: unreadable files or
    /// directories, unbalanced bodies.
    pub errors: Vec<String>,
    /// Oddities the scan tolerated: an identifier declared with several
    /// core types (a call through it resolves against each).
    pub notes: Vec<String>,
}

/// One `pub fn` of frontend-core with `&mut` in its parameters or with no
/// `self` receiver at all.
pub struct CoreEntry {
    /// Path relative to `crates/frontend-core/src`, e.g. `scripts/mod.rs`.
    pub file: String,
    /// 1-based line of the `pub fn`.
    pub line: u32,
    /// Enclosing inherent-impl target type, if any.
    pub owner: Option<String>,
    /// Fn name.
    pub name: String,
    /// `<file>::<Owner>::<name>` or `<file>::<name>`.
    pub key: String,
    /// Whether the fn is externally reachable (rule A3).
    pub reachable: bool,
    /// Directly attached `#[cfg(...)]`, plus any inherited from the
    /// enclosing `impl`/`mod` block, joined with ` + ` (innermost first).
    pub cfg: Option<String>,
    /// What `self` form the first parameter has.
    pub receiver: Receiver,
    /// Whether the return type mentions `Self` or the owner type name.
    pub returns_self: bool,
}

/// What `self` form the first parameter of a core fn has.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Receiver {
    /// `&mut self` (a lifetime is allowed).
    MutSelf,
    /// `&self` (a lifetime is allowed).
    RefSelf,
    /// By-value `self` (`self`, `mut self`, `self: Type`).
    OwnedSelf,
    /// No `self` receiver: a free fn or an associated fn.
    None,
}

/// One call from a surface fn into a reachable core entry.
pub struct CoreCall {
    /// Key of the called core entry.
    pub key: String,
    /// 1-based line of the call.
    pub line: u32,
}

/// One `fn` item of panel or tui.
pub struct SurfaceFn {
    /// `panel` or `tui`.
    pub crate_name: &'static str,
    /// Path relative to `crates/<crate>/src`.
    pub file: String,
    /// 1-based line of the `fn` keyword.
    pub line: u32,
    /// Enclosing impl target type, if any.
    pub owner: Option<String>,
    /// Fn name.
    pub name: String,
    /// Calls into reachable core entries, in body order.
    pub core_calls: Vec<CoreCall>,
    /// Effect probes hit: (probe text, line).
    pub probes: Vec<(String, u32)>,
    /// Literal command rows named in the body (`OperatorCommand::Row`):
    /// (row name, line). A surface call of the command seam
    /// (`OperatorSession::submit`) issues the row it names.
    pub row_calls: Vec<(String, u32)>,
}

/// Enumerate the operator-decision census for the workspace at
/// `workspace_root`. Never panics on odd input: problems go to `errors`.
pub fn scan(workspace_root: &Path) -> Census {
    let mut errors = Vec::new();
    let mut notes = Vec::new();
    let core_dir = workspace_root.join("crates/frontend-core/src");
    let core_files = list_rs(&core_dir, &mut errors);
    let core = scan_core(&core_dir, &core_files, &mut errors);
    let mut method_of: BTreeMap<(String, String), String> = BTreeMap::new();
    let mut free_of: BTreeMap<String, String> = BTreeMap::new();
    let mut owners: BTreeSet<String> = BTreeSet::new();
    for e in &core {
        if !e.reachable {
            continue;
        }
        if let Some(o) = &e.owner {
            owners.insert(o.clone());
            method_of
                .entry((o.clone(), e.name.clone()))
                .or_insert_with(|| e.key.clone());
        } else {
            free_of
                .entry(e.name.clone())
                .or_insert_with(|| e.key.clone());
        }
    }
    let mut surface = Vec::new();
    for (tag, dir) in [
        ("panel", workspace_root.join("crates/panel/src")),
        ("tui", workspace_root.join("crates/tui/src")),
    ] {
        let files = list_rs(&dir, &mut errors);
        surface.extend(scan_surface(
            tag,
            &dir,
            &files,
            &owners,
            &method_of,
            &free_of,
            &mut errors,
            &mut notes,
        ));
    }
    surface.sort_by_key(display_surface);
    Census {
        core,
        surface,
        errors,
        notes,
    }
}

/// `crate/file::Owner::name`: the key the surface register uses.
pub fn display_surface(f: &SurfaceFn) -> String {
    match &f.owner {
        Some(o) => format!("{}/{}::{}::{}", f.crate_name, f.file, o, f.name),
        None => format!("{}/{}::{}", f.crate_name, f.file, f.name),
    }
}

// ---------------------------------------------------------------------------
// Preprocessing
// ---------------------------------------------------------------------------

/// Byte is an ASCII identifier character.
fn is_ident(b: u8) -> bool {
    matches!(b, b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'_')
}

/// Replace comments and string/char literals with spaces, keeping `\n` and
/// the exact byte length so offsets and line numbers still line up.
fn strip_literals(src: &str) -> String {
    let b = src.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        let prev_ident = i > 0 && is_ident(b[i - 1]);
        if c == b'/' && i + 1 < b.len() && b[i + 1] == b'/' {
            out.extend_from_slice(b"  ");
            i += 2;
            while i < b.len() && b[i] != b'\n' {
                out.push(b' ');
                i += 1;
            }
        } else if c == b'/' && i + 1 < b.len() && b[i + 1] == b'*' {
            out.extend_from_slice(b"  ");
            i += 2;
            let mut depth = 1u32;
            while i < b.len() && depth > 0 {
                if b[i] == b'/' && i + 1 < b.len() && b[i + 1] == b'*' {
                    out.extend_from_slice(b"  ");
                    depth += 1;
                    i += 2;
                } else if b[i] == b'*' && i + 1 < b.len() && b[i + 1] == b'/' {
                    out.extend_from_slice(b"  ");
                    depth -= 1;
                    i += 2;
                } else {
                    out.push(if b[i] == b'\n' { b'\n' } else { b' ' });
                    i += 1;
                }
            }
        } else if c == b'"' {
            i = blank_string(b, &mut out, i);
        } else if c == b'\'' {
            match char_end(b, i) {
                Some(e) => {
                    for byte in &b[i..=e] {
                        out.push(if *byte == b'\n' { b'\n' } else { b' ' });
                    }
                    i = e + 1;
                }
                None => {
                    out.push(c);
                    i += 1;
                }
            }
        } else if c == b'r' && !prev_ident && raw_hashes(b, i).is_some() {
            i = blank_raw(b, &mut out, i);
        } else if c == b'b' && !prev_ident {
            if i + 1 < b.len() && b[i + 1] == b'"' {
                out.push(b' ');
                i = blank_string(b, &mut out, i + 1);
            } else if i + 1 < b.len() && b[i + 1] == b'r' && raw_hashes(b, i + 1).is_some() {
                out.push(b' ');
                i = blank_raw(b, &mut out, i + 1);
            } else if i + 1 < b.len() && b[i + 1] == b'\'' {
                match char_end(b, i + 1) {
                    Some(e) => {
                        out.extend(std::iter::repeat_n(b' ', e + 1 - i));
                        i = e + 1;
                    }
                    None => {
                        out.push(c);
                        i += 1;
                    }
                }
            } else {
                out.push(c);
                i += 1;
            }
        } else {
            out.push(c);
            i += 1;
        }
    }
    match String::from_utf8(out) {
        Ok(s) => s,
        Err(_) => src.to_string(),
    }
}

/// Blank a regular `"..."` string starting at the opening quote `at`.
fn blank_string(b: &[u8], out: &mut Vec<u8>, at: usize) -> usize {
    out.push(b' ');
    let mut i = at + 1;
    while i < b.len() {
        if b[i] == b'\\' {
            out.push(b' ');
            i += 1;
            if i < b.len() {
                out.push(if b[i] == b'\n' { b'\n' } else { b' ' });
                i += 1;
            }
        } else if b[i] == b'"' {
            out.push(b' ');
            return i + 1;
        } else {
            out.push(if b[i] == b'\n' { b'\n' } else { b' ' });
            i += 1;
        }
    }
    i
}

/// If `r` at `at` starts a raw string, return its `#` count.
fn raw_hashes(b: &[u8], at: usize) -> Option<usize> {
    let mut k = 0;
    while at + 1 + k < b.len() && b[at + 1 + k] == b'#' {
        k += 1;
    }
    if at + 1 + k < b.len() && b[at + 1 + k] == b'"' {
        Some(k)
    } else {
        None
    }
}

/// Blank a raw `r#*"..."*#` string starting at its `r`.
fn blank_raw(b: &[u8], out: &mut Vec<u8>, r_at: usize) -> usize {
    let k = raw_hashes(b, r_at).unwrap_or(0);
    let mut i = r_at;
    while i < r_at + 1 + k + 1 && i < b.len() {
        out.push(b' ');
        i += 1;
    }
    while i < b.len() {
        if b[i] == b'"' {
            let mut ok = true;
            for h in 0..k {
                if i + 1 + h >= b.len() || b[i + 1 + h] != b'#' {
                    ok = false;
                    break;
                }
            }
            if ok {
                out.extend(std::iter::repeat_n(b' ', k + 1));
                return i + 1 + k;
            }
        }
        out.push(if b[i] == b'\n' { b'\n' } else { b' ' });
        i += 1;
    }
    i
}

/// Closing-quote index of a char literal starting at `at`, or `None` for a
/// lifetime such as `'a`.
fn char_end(b: &[u8], at: usize) -> Option<usize> {
    if at + 1 >= b.len() {
        return None;
    }
    if b[at + 1] == b'\\' {
        let mut j = at + 2;
        if j < b.len() && b[j] == b'u' && j + 1 < b.len() && b[j + 1] == b'{' {
            j += 2;
            while j < b.len() && b[j] != b'}' && j < at + 12 {
                j += 1;
            }
            if j < b.len() && b[j] == b'}' {
                j += 1;
            } else {
                return None;
            }
        } else {
            j += 1;
        }
        if j < b.len() && b[j] == b'\'' && j < at + 13 {
            Some(j)
        } else {
            None
        }
    } else if at + 2 < b.len() && b[at + 2] == b'\'' {
        Some(at + 2)
    } else {
        None
    }
}

/// Offsets at which each line starts.
fn line_starts(text: &str) -> Vec<usize> {
    let mut v = vec![0];
    for (i, c) in text.bytes().enumerate() {
        if c == b'\n' {
            v.push(i + 1);
        }
    }
    v
}

/// 1-based line number of byte offset `off`.
fn line_of(starts: &[usize], off: usize) -> u32 {
    let mut lo = 0;
    let mut hi = starts.len();
    while lo + 1 < hi {
        let mid = (lo + hi) / 2;
        if starts[mid] <= off {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    (lo + 1) as u32
}

// ---------------------------------------------------------------------------
// File inventory
// ---------------------------------------------------------------------------

/// Whether a file name is excluded as a test file.
fn is_test_name(name: &str) -> bool {
    name.ends_with("_tests.rs") || name == "tests.rs" || name == "test_support.rs"
}

/// Recursively list `.rs` files under `dir`, sorted for determinism.
fn list_rs(dir: &Path, errors: &mut Vec<String>) -> Vec<PathBuf> {
    let mut out = Vec::new();
    collect_rs(dir, &mut out, errors);
    out.sort();
    out
}

/// Recursive worker for [`list_rs`].
fn collect_rs(dir: &Path, out: &mut Vec<PathBuf>, errors: &mut Vec<String>) {
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) => {
            errors.push(format!("read dir {}: {e}", dir.display()));
            return;
        }
    };
    for ent in entries {
        let ent = match ent {
            Ok(e) => e,
            Err(e) => {
                errors.push(format!("read dir entry in {}: {e}", dir.display()));
                continue;
            }
        };
        let path = ent.path();
        let kind = match ent.file_type() {
            Ok(k) => k,
            Err(e) => {
                errors.push(format!("stat {}: {e}", path.display()));
                continue;
            }
        };
        if kind.is_dir() {
            collect_rs(&path, out, errors);
        } else if kind.is_file() && matches!(path.extension().and_then(|e| e.to_str()), Some("rs"))
        {
            out.push(path);
        }
    }
}

/// `/`-joined path of `path` relative to `base`.
fn rel_slash(path: &Path, base: &Path, errors: &mut Vec<String>) -> Option<String> {
    match path.strip_prefix(base) {
        Ok(r) => {
            let mut s = String::new();
            for c in r.components() {
                if !s.is_empty() {
                    s.push('/');
                }
                s.push_str(&c.as_os_str().to_string_lossy());
            }
            Some(s)
        }
        Err(e) => {
            errors.push(format!("relativize {}: {e}", path.display()));
            None
        }
    }
}

/// Read a file to a string, pushing to `errors` instead of panicking.
fn read_file(path: &Path, errors: &mut Vec<String>) -> Option<String> {
    match fs::read_to_string(path) {
        Ok(s) => Some(s),
        Err(e) => {
            errors.push(format!("read {}: {e}", path.display()));
            None
        }
    }
}

// ---------------------------------------------------------------------------
// Token scanning on cleaned text
// ---------------------------------------------------------------------------

/// Next whole-word occurrence of `kw` at or after `from`.
fn next_word(clean: &[u8], kw: &[u8], mut pos: usize) -> Option<usize> {
    while pos + kw.len() <= clean.len() {
        if &clean[pos..pos + kw.len()] == kw
            && (pos == 0 || !is_ident(clean[pos - 1]))
            && (pos + kw.len() == clean.len() || !is_ident(clean[pos + kw.len()]))
        {
            return Some(pos);
        }
        pos += 1;
    }
    None
}

/// Skip ASCII whitespace forward.
fn skip_ws_fwd(clean: &[u8], mut p: usize) -> usize {
    while p < clean.len() && matches!(clean[p], b' ' | b'\t' | b'\n' | b'\r') {
        p += 1;
    }
    p
}

/// Skip ASCII whitespace backward, returning the new exclusive end.
fn skip_ws_bwd(clean: &[u8], mut e: usize) -> usize {
    while e > 0 && matches!(clean[e - 1], b' ' | b'\t' | b'\n' | b'\r') {
        e -= 1;
    }
    e
}

/// Read the identifier ending at exclusive `e`; returns `(start, text)`.
fn read_ident_bwd(clean: &[u8], e: usize) -> (usize, &[u8]) {
    let mut s = e;
    while s > 0 && is_ident(clean[s - 1]) {
        s -= 1;
    }
    (s, &clean[s..e])
}

/// Whether the `fn` at `kw` is written exactly `pub fn` (allowing
/// `unsafe`/`async`/`const`/`extern` between, but not `pub(...)`).
fn is_plain_pub_fn(clean: &[u8], kw: usize) -> bool {
    let mut e = skip_ws_bwd(clean, kw);
    loop {
        let (s, w) = read_ident_bwd(clean, e);
        if w == b"pub" {
            return s == 0 || !is_ident(clean[s - 1]);
        }
        if w == b"unsafe" || w == b"async" || w == b"const" || w == b"extern" {
            e = skip_ws_bwd(clean, s);
            continue;
        }
        return false;
    }
}

/// Index of the closer matching the opener at `open`.
fn match_pair(clean: &[u8], open: usize, o: u8, c: u8) -> Option<usize> {
    let mut depth = 0u32;
    let mut i = open;
    while i < clean.len() {
        if clean[i] == o {
            depth += 1;
        } else if clean[i] == c {
            depth -= 1;
            if depth == 0 {
                return Some(i);
            }
        }
        i += 1;
    }
    None
}

/// Index of the `>` closing the `<` at `open`, ignoring `->` arrows.
fn match_angle(clean: &[u8], open: usize) -> Option<usize> {
    let mut depth = 0u32;
    let mut i = open;
    while i < clean.len() {
        match clean[i] {
            b'<' => depth += 1,
            b'>' => {
                if i > 0 && clean[i - 1] == b'-' {
                    i += 1;
                    continue;
                }
                if depth == 0 {
                    return None;
                }
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            b'(' => {
                i = match_pair(clean, i, b'(', b')')?;
            }
            b'[' => {
                i = match_pair(clean, i, b'[', b']')?;
            }
            b'{' | b';' | b'}' => return None,
            _ => {}
        }
        i += 1;
    }
    None
}

/// A parsed `fn` item: signature spans plus optional balanced body.
struct ParsedFn {
    /// Fn name.
    name: String,
    /// Inner span of the parameter list.
    params: (usize, usize),
    /// Index of the `{` or `;` ending the signature.
    sig_end: usize,
    /// Inner span of the body, if it has one.
    body: Option<(usize, usize)>,
}

/// Parse the `fn` item whose keyword is at `kw`.
fn parse_fn_at(clean: &[u8], kw: usize) -> Option<ParsedFn> {
    let mut p = skip_ws_fwd(clean, kw + 2);
    let ns = p;
    while p < clean.len() && is_ident(clean[p]) {
        p += 1;
    }
    if p == ns {
        return None;
    }
    let name = String::from_utf8_lossy(&clean[ns..p]).into_owned();
    // Optional generic parameter list, then the parameter list.
    let po = loop {
        p = skip_ws_fwd(clean, p);
        if p >= clean.len() {
            return None;
        }
        match clean[p] {
            b'<' => {
                p = match_angle(clean, p)? + 1;
            }
            b'(' => break p,
            _ => return None,
        }
    };
    let pc = match_pair(clean, po, b'(', b')')?;
    let params = (po + 1, pc);
    // To the first `{` or `;` at signature depth.
    let mut dp = 0u32;
    let mut db = 0u32;
    let mut dc = 0u32;
    let mut q = pc + 1;
    let mut sig_end = None;
    let mut bodied = false;
    while q < clean.len() {
        match clean[q] {
            b'(' => dp += 1,
            b')' => dp = dp.saturating_sub(1),
            b'[' => db += 1,
            b']' => db = db.saturating_sub(1),
            b'{' => {
                if dp == 0 && db == 0 && dc == 0 {
                    sig_end = Some(q);
                    bodied = true;
                    break;
                }
                dc += 1;
            }
            b'}' => {
                if dc > 0 {
                    dc -= 1;
                } else {
                    break;
                }
            }
            b';' if dp == 0 && db == 0 && dc == 0 => {
                sig_end = Some(q);
                break;
            }
            _ => {}
        }
        q += 1;
    }
    let se = sig_end?;
    let body = if bodied {
        match_pair(clean, se, b'{', b'}').map(|c| (se + 1, c))
    } else {
        None
    };
    // A `{` with no matching `}` is odd input: treat as bodiless.
    if bodied && body.is_none() {
        return None;
    }
    Some(ParsedFn {
        name,
        params,
        sig_end: se,
        body,
    })
}

/// Target type of an `impl` header (generics stripped, `for` handled).
fn impl_target(header: &str) -> Option<String> {
    let b = header.as_bytes();
    // Cut any `where` clause at nesting depth 0.
    let mut cut = b.len();
    let mut dp = 0u32;
    let mut db = 0u32;
    let mut da = 0u32;
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'(' => dp += 1,
            b')' => dp = dp.saturating_sub(1),
            b'[' => db += 1,
            b']' => db = db.saturating_sub(1),
            b'<' => da += 1,
            b'>' if i == 0 || b[i - 1] != b'-' => {
                da = da.saturating_sub(1);
            }
            _ => {}
        }
        if dp == 0 && db == 0 && da == 0 && b[i] == b'w' && header[i..].starts_with("where") {
            let before = i == 0 || !is_ident(b[i - 1]);
            let after = i + 5 >= b.len() || !is_ident(b[i + 5]);
            if before && after {
                cut = i;
                break;
            }
        }
        i += 1;
    }
    let head = header[..cut].trim();
    // Strip leading generic parameters (`impl<T> Type<T>`).
    let mut ty = head;
    if ty.trim_start().starts_with('<') {
        let rel = ty.find('<').unwrap_or(0);
        let c = match_angle(ty.as_bytes(), rel)?;
        ty = ty[c + 1..].trim();
    }
    // The type is after a top-level `for` that is not higher-ranked `for<`.
    let hb = ty.as_bytes();
    let mut k = 0;
    while k + 3 <= hb.len() {
        if hb[k] == b'f'
            && ty[k..].starts_with("for")
            && (k == 0 || !is_ident(hb[k - 1]))
            && (k + 3 >= hb.len() || !is_ident(hb[k + 3]))
        {
            let mut r = k + 3;
            while r < hb.len() && matches!(hb[r], b' ' | b'\t' | b'\n' | b'\r') {
                r += 1;
            }
            if r < hb.len() && hb[r] != b'<' {
                ty = ty[r..].trim();
                break;
            }
        }
        k += 1;
    }
    let last = ty.rsplit("::").next().unwrap_or("").trim();
    let base = last.split('<').next().unwrap_or("").trim();
    if base.is_empty() || !base.bytes().all(is_ident) {
        return None;
    }
    Some(base.to_string())
}

/// Whether `hay` contains `word` as a whole word.
fn contains_word(hay: &str, word: &str) -> bool {
    let h = hay.as_bytes();
    let w = word.as_bytes();
    let mut i = 0;
    while i + w.len() <= h.len() {
        if &h[i..i + w.len()] == w
            && (i == 0 || !is_ident(h[i - 1]))
            && (i + w.len() == h.len() || !is_ident(h[i + w.len()]))
        {
            return true;
        }
        i += 1;
    }
    false
}

// ---------------------------------------------------------------------------
// Per-file parse: impls, inline mods, fns, cfgs
// ---------------------------------------------------------------------------

/// An `impl` block with its target type and directly attached cfg.
struct ImplInfo {
    /// Index of the opening `{`.
    open: usize,
    /// Index of the matching `}`.
    close: usize,
    /// Target type name.
    ty: String,
    /// Directly attached `#[cfg(...)]` text, if any.
    cfg: Option<String>,
}

/// An inline `mod name { ... }` block with its directly attached cfg.
struct ModBlock {
    /// Index of the opening `{`.
    open: usize,
    /// Index of the matching `}`.
    close: usize,
    /// Directly attached `#[cfg(...)]` text, if any.
    cfg: Option<String>,
}

/// Collect `impl` blocks (with cfg) from cleaned text.
fn impl_blocks(clean: &str, starts: &[usize], orig_lines: &[String]) -> Vec<ImplInfo> {
    let b = clean.as_bytes();
    let mut out = Vec::new();
    let mut pos = 0;
    while let Some(kw) = next_word(b, b"impl", pos) {
        pos = kw + 4;
        let mut dp = 0u32;
        let mut db = 0u32;
        let mut q = kw + 4;
        let mut brace = None;
        while q < b.len() {
            match b[q] {
                b'(' => dp += 1,
                b')' => dp = dp.saturating_sub(1),
                b'[' => db += 1,
                b']' => db = db.saturating_sub(1),
                b'{' => {
                    if dp == 0 && db == 0 {
                        brace = Some(q);
                        break;
                    }
                }
                b';' | b'}' => break,
                _ => {}
            }
            q += 1;
        }
        let bo = match brace {
            Some(o) => o,
            None => continue,
        };
        let header = clean[kw + 4..bo].to_string();
        let close = match match_pair(b, bo, b'{', b'}') {
            Some(c) => c,
            None => continue,
        };
        if let Some(ty) = impl_target(&header) {
            let cfg = cfg_above(orig_lines, line_of(starts, kw));
            out.push(ImplInfo {
                open: bo,
                close,
                ty,
                cfg,
            });
        }
    }
    out
}

/// Collect inline `mod name { ... }` blocks; `#[cfg(test)]` ones become skip
/// ranges instead.
fn mod_blocks(
    clean: &str,
    starts: &[usize],
    orig_lines: &[String],
) -> (Vec<ModBlock>, Vec<(usize, usize)>) {
    let b = clean.as_bytes();
    let mut mods = Vec::new();
    let mut skips = Vec::new();
    let mut pos = 0;
    while let Some(kw) = next_word(b, b"mod", pos) {
        pos = kw + 3;
        let mut p = skip_ws_fwd(b, kw + 3);
        let ns = p;
        while p < b.len() && is_ident(b[p]) {
            p += 1;
        }
        if p == ns {
            continue;
        }
        p = skip_ws_fwd(b, p);
        if p < b.len() && b[p] == b'{' {
            let close = match match_pair(b, p, b'{', b'}') {
                Some(c) => c,
                None => continue,
            };
            let line = line_of(starts, kw);
            if attr_block_has_test(orig_lines, line) {
                skips.push((p, close));
            } else {
                mods.push(ModBlock {
                    open: p,
                    close,
                    cfg: cfg_above(orig_lines, line),
                });
            }
        }
    }
    (mods, skips)
}

/// Whether the contiguous attribute block above 1-based `item_line`
/// mentions `test` inside a `#[cfg(...)]`.
fn attr_block_has_test(orig_lines: &[String], item_line: u32) -> bool {
    let mut i = item_line as usize;
    while i > 1 {
        i -= 1;
        let Some(line) = orig_lines.get(i - 1) else {
            return false;
        };
        let t = line.trim();
        if t.is_empty() {
            return false;
        }
        if t.starts_with("///") || t.starts_with("//!") {
            continue;
        }
        if t.starts_with("#[") {
            if t.starts_with("#[cfg") && cfg_mentions_test(t) {
                return true;
            }
            continue;
        }
        return false;
    }
    false
}

/// Whether a `#[cfg(...)]` line mentions `test` as a word.
fn cfg_mentions_test(attr: &str) -> bool {
    let mut words = Vec::new();
    let mut cur = String::new();
    for c in attr.bytes() {
        if is_ident(c) {
            cur.push(c as char);
        } else if !cur.is_empty() {
            words.push(cur.clone());
            cur.clear();
        }
    }
    if !cur.is_empty() {
        words.push(cur);
    }
    words.iter().any(|w| w == "test")
}

/// Text of the `#[cfg(...)]` lines directly attached above 1-based
/// `item_line` (scan over `///` and other attrs; blanks/other lines stop).
fn cfg_above(orig_lines: &[String], item_line: u32) -> Option<String> {
    let mut found = Vec::new();
    let mut i = item_line as usize;
    while i > 1 {
        i -= 1;
        let Some(line) = orig_lines.get(i - 1) else {
            break;
        };
        let t = line.trim();
        if t.is_empty() {
            break;
        }
        if t.starts_with("///") || t.starts_with("//!") {
            continue;
        }
        if t.starts_with("#[") {
            if t.starts_with("#[cfg(") {
                found.push(t.to_string());
            }
            continue;
        }
        break;
    }
    if found.is_empty() {
        None
    } else {
        found.reverse();
        Some(found.join(" "))
    }
}

/// Whether `off` lies inside any of `ranges`.
fn in_ranges(ranges: &[(usize, usize)], off: usize) -> bool {
    ranges.iter().any(|(s, e)| *s < off && off < *e)
}

// ---------------------------------------------------------------------------
// Core crate
// ---------------------------------------------------------------------------

/// Module path segments for a file relative to the crate `src` dir.
fn module_path(rel: &str) -> Vec<String> {
    let mut parts: Vec<String> = rel.split('/').map(str::to_string).collect();
    if let Some(last) = parts.last().cloned() {
        if last == "mod.rs" {
            parts.pop();
        } else if let Some(stem) = last.strip_suffix(".rs") {
            if let Some(slot) = parts.last_mut() {
                *slot = stem.to_string();
            }
        }
    }
    if parts == ["lib"] {
        parts.clear();
    }
    parts
}

/// `(mod name, is pub mod)` declarations in a file (depth 0 only).
fn mod_decls(clean: &str) -> Vec<(String, bool)> {
    let b = clean.as_bytes();
    let mut out = Vec::new();
    let mut depth = 0u32;
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'{' => depth += 1,
            b'}' => depth = depth.saturating_sub(1),
            _ => {}
        }
        let is_mod = b[i] == b'm'
            && i + 3 <= b.len()
            && &b[i..i + 3] == b"mod"
            && (i == 0 || !is_ident(b[i - 1]))
            && (i + 3 >= b.len() || !is_ident(b[i + 3]));
        if is_mod && depth == 0 {
            let e = skip_ws_bwd(b, i);
            let (s, w) = read_ident_bwd(b, e);
            let is_pub = w == b"pub" && (s == 0 || !is_ident(b[s - 1]));
            let mut p = skip_ws_fwd(b, i + 3);
            let ns = p;
            while p < b.len() && is_ident(b[p]) {
                p += 1;
            }
            if p > ns {
                out.push((String::from_utf8_lossy(&b[ns..p]).into_owned(), is_pub));
            }
        }
        i += 1;
    }
    out
}

/// Whether `rel` (relative to the core `src` dir) is a public module path.
fn file_is_public(rel: &str, decls: &BTreeMap<(String, String), bool>) -> bool {
    let path = module_path(rel);
    for (level, name) in path.iter().enumerate() {
        let parent = if level == 0 {
            "lib.rs".to_string()
        } else {
            let prefix = path[..level].join("/");
            let rs = format!("{prefix}.rs");
            if decls.keys().any(|(f, _)| f == &rs) {
                rs
            } else {
                format!("{prefix}/mod.rs")
            }
        };
        if decls.get(&(parent, name.clone())) != Some(&true) {
            return false;
        }
    }
    true
}

/// `pub struct` / `pub enum` names declared in cleaned text.
fn type_decls(clean: &str, skips: &[(usize, usize)]) -> Vec<String> {
    let b = clean.as_bytes();
    let mut out = Vec::new();
    for kw in [b"struct".as_slice(), b"enum".as_slice()] {
        let mut pos = 0;
        while let Some(hit) = next_word(b, kw, pos) {
            pos = hit + kw.len();
            if in_ranges(skips, hit) {
                continue;
            }
            let e = skip_ws_bwd(b, hit);
            let (s, w) = read_ident_bwd(b, e);
            if w == b"pub" && (s == 0 || !is_ident(b[s - 1])) {
                let mut p = skip_ws_fwd(b, hit + kw.len());
                let ns = p;
                while p < b.len() && is_ident(b[p]) {
                    p += 1;
                }
                if p > ns {
                    out.push(String::from_utf8_lossy(&b[ns..p]).into_owned());
                }
            }
        }
    }
    out
}

/// Split a use-tree on top-level commas.
fn split_use_tree(tree: &str) -> Vec<&str> {
    let b = tree.as_bytes();
    let mut parts = Vec::new();
    let mut depth = 0u32;
    let mut start = 0;
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'{' => depth += 1,
            b'}' => depth = depth.saturating_sub(1),
            b',' if depth == 0 => {
                parts.push(&tree[start..i]);
                start = i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    parts.push(&tree[start..]);
    parts
}

/// Alias of an `X as Y` item, if it has one.
fn use_alias(item: &str) -> Option<String> {
    let toks: Vec<&str> = item.split_whitespace().collect();
    if toks.len() == 3 && toks[1] == "as" {
        Some(toks[2].to_string())
    } else {
        None
    }
}

/// Names introduced by a `pub use` tree.
fn use_names(tree: &str, out: &mut Vec<String>) {
    for part in split_use_tree(tree) {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        if let Some(o) = part.find('{') {
            let c = part.rfind('}').unwrap_or(part.len());
            let prefix = part[..o].trim().trim_end_matches(':').trim();
            let pre_last = prefix.rsplit("::").next().unwrap_or("").trim();
            for item in split_use_tree(&part[o + 1..c]) {
                let item = item.trim();
                if item.is_empty() {
                    continue;
                }
                if item.contains('{') {
                    use_names(item, out);
                } else if let Some(alias) = use_alias(item) {
                    out.push(alias);
                } else if item == "self" {
                    if !pre_last.is_empty() && pre_last != "*" {
                        out.push(pre_last.to_string());
                    }
                } else {
                    let n = item.rsplit("::").next().unwrap_or("").trim();
                    if !n.is_empty() && n != "*" {
                        out.push(n.to_string());
                    }
                }
            }
        } else if part == "*" || part.ends_with("::*") {
            continue;
        } else if let Some(alias) = use_alias(part) {
            out.push(alias);
        } else {
            let n = part
                .rsplit("::")
                .next()
                .unwrap_or("")
                .trim()
                .trim_end_matches(';');
            if !n.is_empty() && n != "*" {
                out.push(n.to_string());
            }
        }
    }
}

/// Imported names of every `pub use ...;` in cleaned text.
fn pub_use_names(clean: &str, skips: &[(usize, usize)]) -> Vec<String> {
    let b = clean.as_bytes();
    let mut out = Vec::new();
    let mut pos = 0;
    while let Some(hit) = next_word(b, b"use", pos) {
        pos = hit + 3;
        if in_ranges(skips, hit) {
            continue;
        }
        let e = skip_ws_bwd(b, hit);
        let (s, w) = read_ident_bwd(b, e);
        if w != b"pub" || (s > 0 && is_ident(b[s - 1])) {
            continue;
        }
        let mut depth = 0u32;
        let mut q = hit + 3;
        let mut end = None;
        while q < b.len() {
            match b[q] {
                b'{' => depth += 1,
                b'}' => depth = depth.saturating_sub(1),
                b';' if depth == 0 => {
                    end = Some(q);
                    break;
                }
                _ => {}
            }
            q += 1;
        }
        if let Some(e2) = end {
            if let Ok(tree) = std::str::from_utf8(&b[hit + 3..e2]) {
                use_names(tree, &mut out);
            }
        }
    }
    out
}

/// What `self` form the first parameter has.
fn receiver_of(params: &str) -> Receiver {
    let first = split_params(params)
        .first()
        .map(|s| s.trim().to_string())
        .unwrap_or_default();
    if first.is_empty() {
        return Receiver::None;
    }
    if first == "self"
        || first == "mut self"
        || first.starts_with("self:")
        || first.starts_with("mut self:")
    {
        return Receiver::OwnedSelf;
    }
    if let Some(after_amp) = first.strip_prefix('&') {
        let mut rest = after_amp.trim();
        if let Some(lt) = rest.strip_prefix('\'') {
            let mut k = 0;
            while k < lt.len() && is_ident(lt.as_bytes()[k]) {
                k += 1;
            }
            rest = lt[k..].trim();
        }
        if rest == "self" {
            return Receiver::RefSelf;
        }
        if let Some(after_mut) = rest.strip_prefix("mut") {
            if after_mut.is_empty() || after_mut.starts_with(' ') || after_mut.starts_with('\t') {
                let after_mut = after_mut.trim();
                if after_mut == "self" || after_mut.starts_with("self:") {
                    return Receiver::MutSelf;
                }
            }
        }
    }
    Receiver::None
}

/// Whether the return type (after `->`, before `where`/end) mentions `Self`
/// or the owner type name as a whole word.
fn returns_self_mentioned(tail: &str, owner: Option<&str>) -> bool {
    let b = tail.as_bytes();
    let mut dp = 0u32;
    let mut db = 0u32;
    let mut i = 0;
    let mut arrow = None;
    while i + 1 < b.len() {
        match b[i] {
            b'(' => dp += 1,
            b')' => dp = dp.saturating_sub(1),
            b'[' => db += 1,
            b']' => db = db.saturating_sub(1),
            b'-' if b[i + 1] == b'>' && dp == 0 && db == 0 => {
                arrow = Some(i + 2);
                break;
            }
            _ => {}
        }
        i += 1;
    }
    let start = match arrow {
        Some(s) => s,
        None => return false,
    };
    let tb = tail.as_bytes();
    let mut end = tb.len();
    let mut k = start;
    let (mut p2, mut b2) = (0u32, 0u32);
    while k < tb.len() {
        match tb[k] {
            b'(' => p2 += 1,
            b')' => p2 = p2.saturating_sub(1),
            b'[' => b2 += 1,
            b']' => b2 = b2.saturating_sub(1),
            _ => {}
        }
        if p2 == 0 && b2 == 0 && tb[k] == b'w' && tail[k..].starts_with("where") {
            let before = k == 0 || !is_ident(tb[k - 1]);
            let after = k + 5 >= tb.len() || !is_ident(tb[k + 5]);
            if before && after {
                end = k;
                break;
            }
        }
        k += 1;
    }
    let ret = tail[start..end].trim();
    if contains_word(ret, "Self") {
        return true;
    }
    match owner {
        Some(o) => contains_word(ret, o),
        None => false,
    }
}

/// Loaded core file with cleaned text, line table, and original lines.
struct CoreFile {
    /// Path relative to the core `src` dir.
    rel: String,
    /// Comment/string-stripped source.
    clean: String,
    /// Line-start offsets shared by original and cleaned text.
    starts: Vec<usize>,
    /// Original lines for cfg attachment.
    lines: Vec<String>,
}

/// Scan the core crate into entries.
fn scan_core(dir: &Path, files: &[PathBuf], errors: &mut Vec<String>) -> Vec<CoreEntry> {
    let mut loaded: Vec<CoreFile> = Vec::new();
    for path in files {
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if is_test_name(name) {
            continue;
        }
        let Some(rel) = rel_slash(path, dir, errors) else {
            continue;
        };
        let Some(text) = read_file(path, errors) else {
            continue;
        };
        loaded.push(CoreFile {
            rel,
            clean: strip_literals(&text),
            starts: line_starts(&text),
            lines: text.lines().map(str::to_string).collect(),
        });
    }
    // Auxiliary facts from every non-test file (universe files plus lib.rs,
    // command.rs, process_status.rs: their `pub use` lines still count).
    let mut decls: BTreeMap<(String, String), bool> = BTreeMap::new();
    let mut reexported: BTreeSet<String> = BTreeSet::new();
    let mut public_types: BTreeSet<String> = BTreeSet::new();
    let mut public_file: BTreeMap<String, bool> = BTreeMap::new();
    for l in &loaded {
        for (name, is_pub) in mod_decls(&l.clean) {
            decls.insert((l.rel.clone(), name), is_pub);
        }
    }
    for l in &loaded {
        public_file.insert(l.rel.clone(), file_is_public(&l.rel, &decls));
    }
    for l in &loaded {
        let (_, skips) = mod_blocks(&l.clean, &l.starts, &l.lines);
        for n in pub_use_names(&l.clean, &skips) {
            reexported.insert(n);
        }
        if public_file.get(&l.rel).copied().unwrap_or(false) {
            for t in type_decls(&l.clean, &skips) {
                public_types.insert(t);
            }
        }
    }
    let type_is_public = |t: &str| public_types.contains(t) || reexported.contains(t);
    // Universe: skip lib.rs, command.rs, process_status.rs.
    let mut entries = Vec::new();
    for l in &loaded {
        let fname = l.rel.rsplit('/').next().unwrap_or("");
        if matches!(
            fname,
            "lib.rs" | "command.rs" | "process_status.rs" | "command_census_scan.rs"
        ) {
            continue;
        }
        let impls = impl_blocks(&l.clean, &l.starts, &l.lines);
        let (mods, skips) = mod_blocks(&l.clean, &l.starts, &l.lines);
        let b = l.clean.as_bytes();
        let mut pos = 0;
        while let Some(kw) = next_word(b, b"fn", pos) {
            pos = kw + 2;
            if in_ranges(&skips, kw) || !is_plain_pub_fn(b, kw) {
                continue;
            }
            let Some(f) = parse_fn_at(b, kw) else {
                continue;
            };
            let params_text = &l.clean[f.params.0..f.params.1];
            let receiver = receiver_of(params_text);
            if !params_text.contains("&mut") && receiver != Receiver::None {
                continue;
            }
            let enclosing_impl = impls
                .iter()
                .filter(|im| im.open < kw && kw < im.close)
                .max_by_key(|im| im.open);
            let owner = enclosing_impl.map(|im| im.ty.clone());
            let file = l.rel.clone();
            let reachable = match &owner {
                Some(o) => type_is_public(o),
                None => {
                    public_file.get(&l.rel).copied().unwrap_or(false)
                        || reexported.contains(&f.name)
                }
            };
            let tail = &l.clean[f.params.1 + 1..f.sig_end];
            let returns_self = returns_self_mentioned(tail, owner.as_deref());
            let line = line_of(&l.starts, kw);
            // Own cfg, then enclosing impl, then enclosing mods (innermost).
            let mut cfgs = Vec::new();
            if let Some(c) = cfg_above(&l.lines, line) {
                cfgs.push(c);
            }
            if let Some(im) = enclosing_impl {
                if let Some(c) = &im.cfg {
                    cfgs.push(c.clone());
                }
            }
            let mut enclosing: Vec<&ModBlock> = mods
                .iter()
                .filter(|m| m.open < kw && kw < m.close)
                .collect();
            enclosing.sort_by_key(|m| std::cmp::Reverse(m.open));
            for m in enclosing {
                if let Some(c) = &m.cfg {
                    cfgs.push(c.clone());
                }
            }
            entries.push(CoreEntry {
                file,
                line,
                owner,
                name: f.name.clone(),
                key: String::new(),
                reachable,
                cfg: if cfgs.is_empty() {
                    None
                } else {
                    Some(cfgs.join(" + "))
                },
                receiver,
                returns_self,
            });
        }
    }
    for e in &mut entries {
        e.key = match &e.owner {
            Some(o) => format!("{}::{o}::{}", e.file, e.name),
            None => format!("{}::{}", e.file, e.name),
        };
    }
    entries.sort_by(|a, b| a.key.cmp(&b.key));
    entries
}

// ---------------------------------------------------------------------------
// Surface crates
// ---------------------------------------------------------------------------

/// Effect probes: exact substrings searched in fn bodies.
const PROBES: &[&str] = &[
    ".save()",
    "script::set_rs2b0t_import_deferred_at(",
    "script::clear_rs2b0t_import_at(",
    "Vault::create(",
    "Vault::unlock(",
    "Vault::reset(",
    "vault::reset",
    "std::fs::write(",
    "std::fs::remove_file(",
    "std::fs::remove_dir",
    "std::fs::rename(",
    "fs::write(",
    "fs::remove_file(",
    "fs::rename(",
    "host_play::open_map_demand(",
    "host_play::bake_shipped_map_images(",
    ".open_profile(",
    "set_value(",
    "map_demand = None",
    "map_demand.take()",
    "std::process::exit(",
];

/// Split parameters on top-level commas.
fn split_params(params: &str) -> Vec<&str> {
    let b = params.as_bytes();
    let mut parts = Vec::new();
    let (mut dp, mut db, mut dc) = (0u32, 0u32, 0u32);
    let mut start = 0;
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'(' => dp += 1,
            b')' => dp = dp.saturating_sub(1),
            b'[' => db += 1,
            b']' => db = db.saturating_sub(1),
            b'{' => dc += 1,
            b'}' => dc = dc.saturating_sub(1),
            b',' if dp == 0 && db == 0 && dc == 0 => {
                parts.push(&params[start..i]);
                start = i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    parts.push(&params[start..]);
    parts
}

/// `(name, type)` of a `name: Type` parameter piece, if plain.
fn param_name_type(piece: &str) -> Option<(&str, &str)> {
    let b = piece.as_bytes();
    let mut dp = 0u32;
    let mut db = 0u32;
    let mut dc = 0u32;
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'(' => dp += 1,
            b')' => dp = dp.saturating_sub(1),
            b'[' => db += 1,
            b']' => db = db.saturating_sub(1),
            b'{' => dc += 1,
            b'}' => dc = dc.saturating_sub(1),
            b':' if dp == 0 && db == 0 && dc == 0 => {
                if i + 1 < b.len() && b[i + 1] == b':' {
                    i += 2;
                    continue;
                }
                let mut name = piece[..i].trim();
                if let Some(rest) = name.strip_prefix("mut") {
                    if rest.is_empty() || rest.starts_with(' ') || rest.starts_with('\t') {
                        name = rest.trim();
                    }
                }
                let ty = piece[i + 1..].trim();
                if name.is_empty() || name == "_" || !name.bytes().all(is_ident) || ty.is_empty() {
                    return None;
                }
                return Some((name, ty));
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// Last path segment of a declared type, generics and refs stripped.
fn last_segment(ty: &str) -> Option<String> {
    let mut t = ty.trim();
    loop {
        t = t.trim();
        if let Some(rest) = t.strip_prefix('&') {
            t = rest;
            continue;
        }
        if let Some(rest) = t.strip_prefix('*') {
            t = rest;
            continue;
        }
        if let Some(rest) = t.strip_prefix("mut") {
            if rest.is_empty() || rest.starts_with(' ') || rest.starts_with('\t') {
                t = rest;
                continue;
            }
        }
        if let Some(rest) = t.strip_prefix("const") {
            if rest.is_empty() || rest.starts_with(' ') || rest.starts_with('\t') {
                t = rest;
                continue;
            }
        }
        if let Some(rest) = t.strip_prefix('\'') {
            let mut k = 0;
            while k < rest.len() && is_ident(rest.as_bytes()[k]) {
                k += 1;
            }
            if k == 0 {
                return None;
            }
            t = rest[k..].trim();
            continue;
        }
        break;
    }
    let mut end = t.len();
    for (i, c) in t.bytes().enumerate() {
        if matches!(
            c,
            b'<' | b'(' | b'[' | b'{' | b' ' | b'\t' | b'\n' | b'\r' | b',' | b';'
        ) {
            end = i;
            break;
        }
    }
    let head = t[..end].trim();
    let seg = head.rsplit("::").next().unwrap_or("").trim();
    if seg.is_empty() || !seg.bytes().all(is_ident) {
        return None;
    }
    Some(seg.to_string())
}

/// Loaded surface file with cleaned text, line table, and original lines.
struct SurfaceFile {
    /// Path relative to the crate `src` dir.
    rel: String,
    /// Comment/string-stripped source.
    clean: String,
    /// Line-start offsets shared by original and cleaned text.
    starts: Vec<usize>,
    /// Original lines for test-mod detection.
    lines: Vec<String>,
}

/// Scan a surface crate into functions.
#[allow(clippy::too_many_arguments)]
fn scan_surface(
    tag: &'static str,
    dir: &Path,
    files: &[PathBuf],
    owners: &BTreeSet<String>,
    method_of: &BTreeMap<(String, String), String>,
    free_of: &BTreeMap<String, String>,
    errors: &mut Vec<String>,
    notes: &mut Vec<String>,
) -> Vec<SurfaceFn> {
    let mut loaded: Vec<SurfaceFile> = Vec::new();
    for path in files {
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if is_test_name(name) {
            continue;
        }
        let Some(rel) = rel_slash(path, dir, errors) else {
            continue;
        };
        let Some(text) = read_file(path, errors) else {
            continue;
        };
        loaded.push(SurfaceFile {
            rel,
            clean: strip_literals(&text),
            starts: line_starts(&text),
            lines: text.lines().map(str::to_string).collect(),
        });
    }
    // Pass 1: core-holding identifiers from struct fields and fn params.
    let mut holders: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for l in &loaded {
        let b = l.clean.as_bytes();
        let (_, skips) = mod_blocks(&l.clean, &l.starts, &l.lines);
        // Struct fields.
        let mut pos = 0;
        while let Some(kw) = next_word(b, b"struct", pos) {
            pos = kw + 6;
            if in_ranges(&skips, kw) {
                continue;
            }
            let mut q = kw + 6;
            let mut brace = None;
            let (mut dp, mut db) = (0u32, 0u32);
            while q < b.len() {
                match b[q] {
                    b'(' => dp += 1,
                    b')' => dp = dp.saturating_sub(1),
                    b'[' => db += 1,
                    b']' => db = db.saturating_sub(1),
                    b'{' => {
                        if dp == 0 && db == 0 {
                            brace = Some(q);
                            break;
                        }
                    }
                    b';' => break,
                    _ => {}
                }
                q += 1;
            }
            let bo = match brace {
                Some(o) => o,
                None => continue,
            };
            let bc = match match_pair(b, bo, b'{', b'}') {
                Some(c) => c,
                None => {
                    errors.push(format!("{tag}/{}: unbalanced struct body", l.rel));
                    continue;
                }
            };
            for (ident, seg) in struct_fields_in(&l.clean, bo + 1, bc) {
                if owners.contains(&seg) {
                    holders.entry(ident).or_default().insert(seg);
                }
            }
        }
        // Fn parameters (every fn, with or without body).
        let mut fp = 0;
        while let Some(kw) = next_word(b, b"fn", fp) {
            fp = kw + 2;
            if in_ranges(&skips, kw) {
                continue;
            }
            let Some(f) = parse_fn_at(b, kw) else {
                continue;
            };
            let params_text = l.clean[f.params.0..f.params.1].to_string();
            for piece in split_params(&params_text) {
                if let Some((n, t)) = param_name_type(piece) {
                    if let Some(seg) = last_segment(t) {
                        if owners.contains(&seg) {
                            holders.entry(n.to_string()).or_default().insert(seg);
                        }
                    }
                }
            }
        }
    }
    for (ident, tys) in &holders {
        if tys.len() > 1 {
            let mut v: Vec<&str> = tys.iter().map(String::as_str).collect();
            v.sort();
            notes.push(format!("ambiguous core holder `{ident}`: {}", v.join(", ")));
        }
    }
    // Pass 2: calls and probes per fn body.
    let mut out = Vec::new();
    for l in &loaded {
        let b = l.clean.as_bytes();
        let impls = impl_blocks(&l.clean, &l.starts, &l.lines);
        let (_, skips) = mod_blocks(&l.clean, &l.starts, &l.lines);
        let mut pos = 0;
        while let Some(kw) = next_word(b, b"fn", pos) {
            pos = kw + 2;
            if in_ranges(&skips, kw) {
                continue;
            }
            let Some(f) = parse_fn_at(b, kw) else {
                continue;
            };
            let Some((bs, be)) = f.body else { continue };
            let owner = impls
                .iter()
                .filter(|im| im.open < kw && kw < im.close)
                .max_by_key(|im| im.open)
                .map(|im| im.ty.clone());
            let mut calls: Vec<(usize, CoreCall)> = Vec::new();
            method_calls_in(&l.clean, bs, be, &holders, method_of, &l.starts, &mut calls);
            free_calls_in(&l.clean, bs, be, free_of, &l.starts, &mut calls);
            calls.sort_by_key(|(o, _)| *o);
            let core_calls = calls.into_iter().map(|(_, c)| c).collect();
            let mut probes = probe_hits(&l.clean, bs, be, &l.starts);
            probes.dedup();
            let row_calls = row_calls_in(&l.clean, bs, be, &l.starts);
            out.push(SurfaceFn {
                crate_name: tag,
                file: l.rel.clone(),
                line: line_of(&l.starts, kw),
                owner,
                name: f.name.clone(),
                core_calls,
                probes,
                row_calls,
            });
        }
    }
    out
}

/// `(ident, last-segment)` struct fields in `clean[s..e]`.
fn struct_fields_in(clean: &str, s: usize, e: usize) -> Vec<(String, String)> {
    let b = clean.as_bytes();
    let mut found = Vec::new();
    let (mut dp, mut db, mut dc) = (0u32, 0u32, 0u32);
    let mut i = s;
    while i < e {
        match b[i] {
            b'(' => dp += 1,
            b')' => dp = dp.saturating_sub(1),
            b'[' => db += 1,
            b']' => db = db.saturating_sub(1),
            b'{' => dc += 1,
            b'}' => dc = dc.saturating_sub(1),
            b':' if dp == 0 && db == 0 && dc == 0 => {
                if i + 1 < e && b[i + 1] == b':' {
                    i += 2;
                    continue;
                }
                let en = skip_ws_bwd(b, i);
                let (sn, name) = read_ident_bwd(b, en);
                if sn < en {
                    let mut j = i + 1;
                    let (mut qp, mut qb, mut qc) = (0u32, 0u32, 0u32);
                    while j < e {
                        match b[j] {
                            b'(' => qp += 1,
                            b')' => qp = qp.saturating_sub(1),
                            b'[' => qb += 1,
                            b']' => qb = qb.saturating_sub(1),
                            b'{' => qc += 1,
                            b'}' => qc = qc.saturating_sub(1),
                            b',' if qp == 0 && qb == 0 && qc == 0 => break,
                            _ => {}
                        }
                        j += 1;
                    }
                    if let (Ok(nm), Ok(ty)) =
                        (std::str::from_utf8(name), std::str::from_utf8(&b[i + 1..j]))
                    {
                        if let Some(seg) = last_segment(ty) {
                            found.push((nm.to_string(), seg));
                        }
                    }
                    i = j;
                    continue;
                }
            }
            _ => {}
        }
        i += 1;
    }
    found
}

/// Skip an optional `::<...>` turbofish at `p`; return the new position.
fn skip_turbofish(clean: &[u8], mut p: usize) -> Option<usize> {
    p = skip_ws_fwd(clean, p);
    if p + 1 < clean.len() && clean[p] == b':' && clean[p + 1] == b':' {
        let mut q = skip_ws_fwd(clean, p + 2);
        if q < clean.len() && clean[q] == b'<' {
            q = match_angle(clean, q)? + 1;
        }
        return Some(q);
    }
    Some(p)
}

/// `.NAME(` method calls on core-holding identifiers.
fn method_calls_in(
    clean: &str,
    bs: usize,
    be: usize,
    holders: &BTreeMap<String, BTreeSet<String>>,
    method_of: &BTreeMap<(String, String), String>,
    starts: &[usize],
    calls: &mut Vec<(usize, CoreCall)>,
) {
    let b = clean.as_bytes();
    let mut p = bs;
    while p < be {
        if b[p] == b'.' {
            let mut q = p + 1;
            let ns = q;
            while q < be && is_ident(b[q]) {
                q += 1;
            }
            if q > ns {
                if let Ok(name) = std::str::from_utf8(&b[ns..q]) {
                    if let Some(r) = skip_turbofish(b, q) {
                        if r < be && b[r] == b'(' {
                            // The receiver may sit on the previous line
                            // (`session\n    .scripts\n    .start_all(`).
                            let e = skip_ws_bwd(b, p).max(bs);
                            let (s, ident) = read_ident_bwd(&b[..e], e);
                            if s < e && s >= bs {
                                if let Ok(ident) = std::str::from_utf8(ident) {
                                    if let Some(tys) = holders.get(ident) {
                                        for t in tys {
                                            if let Some(key) =
                                                method_of.get(&(t.clone(), name.to_string()))
                                            {
                                                calls.push((
                                                    ns,
                                                    CoreCall {
                                                        key: key.clone(),
                                                        line: line_of(starts, ns),
                                                    },
                                                ));
                                                break;
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            p = q.max(p + 1);
            continue;
        }
        p += 1;
    }
}

/// `NAME(` free-fn calls for reachable owner-less core fns.
fn free_calls_in(
    clean: &str,
    bs: usize,
    be: usize,
    free_of: &BTreeMap<String, String>,
    starts: &[usize],
    calls: &mut Vec<(usize, CoreCall)>,
) {
    let b = clean.as_bytes();
    for (name, key) in free_of {
        let nb = name.as_bytes();
        let mut p = bs;
        while let Some(m) = next_word_in(b, nb, p, be) {
            p = m + nb.len().max(1);
            let r = match skip_turbofish(b, m + nb.len()) {
                Some(r) => r,
                None => continue,
            };
            if r >= be || b[r] != b'(' {
                continue;
            }
            if m > bs && (b[m - 1] == b'.' || is_ident(b[m - 1])) {
                continue;
            }
            // Skip nested `fn NAME(` definitions.
            let e = skip_ws_bwd(b, m);
            let (s, w) = read_ident_bwd(b, e);
            if w == b"fn" && (s == 0 || !is_ident(b[s - 1])) {
                continue;
            }
            calls.push((
                m,
                CoreCall {
                    key: key.clone(),
                    line: line_of(starts, m),
                },
            ));
        }
    }
}

/// Whole-word search for `needle` in `b[..end]` from `pos`.
fn next_word_in(b: &[u8], needle: &[u8], mut pos: usize, end: usize) -> Option<usize> {
    let end = end.min(b.len());
    while pos + needle.len() <= end {
        if &b[pos..pos + needle.len()] == needle
            && (pos == 0 || !is_ident(b[pos - 1]))
            && (pos + needle.len() == b.len() || !is_ident(b[pos + needle.len()]))
        {
            return Some(pos);
        }
        pos += 1;
    }
    None
}

/// Effect-probe hits in a body: (probe text, line), occurrence order.
/// `OperatorCommand::<Row>` names in `clean[bs..be]`, in body order.
fn row_calls_in(clean: &str, bs: usize, be: usize, starts: &[usize]) -> Vec<(String, u32)> {
    const PREFIX: &str = "OperatorCommand::";
    let body = &clean[bs..be.min(clean.len())];
    let mut rows = Vec::new();
    let mut from = 0;
    while let Some(k) = body[from..].find(PREFIX) {
        let start = from + k + PREFIX.len();
        let name: String = body[start..]
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
            .collect();
        if name.starts_with(|c: char| c.is_ascii_uppercase()) && name != "ALL" {
            rows.push((name, line_of(starts, bs + from + k)));
        }
        from = start;
    }
    rows
}

fn probe_hits(clean: &str, bs: usize, be: usize, starts: &[usize]) -> Vec<(String, u32)> {
    let body = &clean[bs..be.min(clean.len())];
    let mut hits: Vec<(usize, (String, u32))> = Vec::new();
    for probe in PROBES {
        let mut from = 0;
        while let Some(k) = body[from..].find(probe) {
            let off = bs + from + k;
            hits.push((off, (probe.to_string(), line_of(starts, off))));
            from += k + probe.len().max(1);
        }
    }
    hits.sort_by_key(|(o, _)| *o);
    hits.into_iter().map(|(_, h)| h).collect()
}
