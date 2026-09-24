//! Fail-closed condition and script-block text helpers extracted from
//! `transport.rs` for maintainability. Same behavior as before the split; not a
//! new parser or grammar.

use std::collections::HashMap;

/// Comment-stripped top-level statements of a script block, in source
/// order. An `if (…) { … }` (with any attached `else`) is one statement;
/// other statements end at a depth-0 `;`. Nested braces stay inside their
/// statement, so a wrapping `if` is not scanned for an inner free arm.
pub(super) fn top_level_statements(block: &str) -> Vec<String> {
    let mut text = String::new();
    for raw in block.lines() {
        let line = match raw.find("//") {
            Some(i) => &raw[..i],
            None => raw,
        };
        text.push_str(line);
        text.push('\n');
    }
    let mut out = Vec::new();
    let mut i = 0usize;
    while i < text.len() {
        while i < text.len() && text.as_bytes()[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= text.len() {
            break;
        }
        let start = i;
        if starts_with_if_kw(&text[i..]) {
            let Some(end) = if_statement_end(&text, i) else {
                break;
            };
            i = end;
        } else {
            let mut depth = 0i32;
            while i < text.len() {
                match text.as_bytes()[i] {
                    b'{' => depth += 1,
                    b'}' => depth -= 1,
                    b';' if depth == 0 => {
                        i += 1;
                        break;
                    }
                    _ => {}
                }
                i += 1;
            }
        }
        let stmt = text[start..i].trim();
        if !stmt.is_empty() {
            out.push(stmt.to_string());
        }
    }
    out
}

/// True when `s` starts with the `if` keyword, not `if_settext` / `if_openmain`.
fn starts_with_if_kw(s: &str) -> bool {
    let s = s.trim_start();
    s.starts_with("if")
        && s[2..]
            .chars()
            .next()
            .is_none_or(|c| c.is_whitespace() || c == '(')
}

/// Byte index just past an `if (…) { … }` starting at `from`, including an
/// attached `else` / `else if` chain so a trailing else cannot be mistaken
/// for a later independent statement.
fn if_statement_end(text: &str, from: usize) -> Option<usize> {
    let mut i = skip_ws(text, from);
    i = parse_if_chunk(text, i)?;
    loop {
        let j = skip_ws(text, i);
        if !text[j..].starts_with("else") {
            return Some(i);
        }
        let after = &text[j + 4..];
        if after.starts_with(|c: char| c.is_ascii_alphanumeric() || c == '_') {
            return Some(i);
        }
        let k = skip_ws(text, j + 4);
        if starts_with_if_kw(&text[k..]) {
            i = parse_if_chunk(text, k)?;
            continue;
        }
        if k < text.len() && text.as_bytes()[k] == b'{' {
            i = matching_delim(text, k, b'{', b'}')? + 1;
            continue;
        }
        return None;
    }
}

fn skip_ws(text: &str, mut i: usize) -> usize {
    while i < text.len() && text.as_bytes()[i].is_ascii_whitespace() {
        i += 1;
    }
    i
}

/// `if (…) { … }` starting at `from` (already trimmed) → index past the arm.
fn parse_if_chunk(text: &str, from: usize) -> Option<usize> {
    if !starts_with_if_kw(&text[from..]) {
        return None;
    }
    let mut i = from + 2;
    i = skip_ws(text, i);
    if i >= text.len() || text.as_bytes()[i] != b'(' {
        return None;
    }
    i = matching_delim(text, i, b'(', b')')? + 1;
    i = skip_ws(text, i);
    if i >= text.len() || text.as_bytes()[i] != b'{' {
        return None;
    }
    Some(matching_delim(text, i, b'{', b'}')? + 1)
}

/// Index of the matching closer for `text[open]` (`open`/`close` pair).
fn matching_delim(text: &str, open: usize, open_ch: u8, close_ch: u8) -> Option<usize> {
    let bytes = text.as_bytes();
    if open >= bytes.len() || bytes[open] != open_ch {
        return None;
    }
    let mut depth = 0i32;
    for (i, &b) in bytes.iter().enumerate().skip(open) {
        if b == open_ch {
            depth += 1;
        } else if b == close_ch {
            depth -= 1;
            if depth == 0 {
                return Some(i);
            }
        }
    }
    None
}

/// `def_boolean $<name> = ~check_axis(coord, loc_coord, loc_angle);` as a
/// whole statement → `$<name>`. Nested defs inside another statement do
/// not match.
pub(super) fn check_axis_def(stmt: &str) -> Option<String> {
    let stmt = stmt.trim();
    let (lhs, rhs) = stmt.split_once('=')?;
    let mut words = lhs.split_whitespace();
    let (Some("def_boolean"), Some(name), None) = (words.next(), words.next(), words.next()) else {
        return None;
    };
    if !name.starts_with('$') {
        return None;
    }
    let rhs: String = rhs.chars().filter(|c| !c.is_whitespace()).collect();
    if rhs == "~check_axis(coord,loc_coord,loc_angle);"
        || rhs == "~check_axis(coord,loc_coord,loc_angle)"
    {
        Some(name.to_string())
    } else {
        None
    }
}

/// `if (<head>) { <arm> }` as a whole statement → the head and arm. A
/// trailing `else` or any other remainder fails closed.
pub(super) fn if_head_and_arm(stmt: &str) -> Option<(String, String)> {
    let s = stmt.trim();
    if !starts_with_if_kw(s) {
        return None;
    }
    let after_if = s[2..].trim_start();
    let inner = after_if.strip_prefix('(')?;
    let close = matching_delim(inner, 0, b'(', b')').or_else(|| {
        // `inner` is already past the opening `(`, so match as if the
        // opener sat just before index 0 by scanning depth from 1.
        let mut depth = 1i32;
        for (i, b) in inner.bytes().enumerate() {
            if b == b'(' {
                depth += 1;
            } else if b == b')' {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
        }
        None
    })?;
    let head = inner[..close].trim().to_string();
    let after_head = inner[close + 1..].trim_start();
    if !after_head.starts_with('{') {
        return None;
    }
    let arm_end = matching_delim(after_head, 0, b'{', b'}')?;
    let arm = after_head[..=arm_end].to_string();
    if !after_head[arm_end + 1..].trim().is_empty() {
        return None;
    }
    Some((head, arm))
}

/// True when the `if` arm opens the door directly: the first top-level
/// statement is the canonical `~open_and_close_door2(<loc>, $<b>, door_open)`
/// call, any later statements are a bare `return`, and there are no nested
/// braces. Quoted text, an earlier terminal, `~open_overlay`, and any other
/// procedure name fail closed. Labels are not followed.
pub(super) fn arm_opens_directly(arm: &str, axis_bool: &str) -> bool {
    let inner = arm.trim();
    let Some(inner) = inner.strip_prefix('{').and_then(|s| s.strip_suffix('}')) else {
        return false;
    };
    if inner.contains('{') {
        return false;
    }
    let stmts = top_level_statements(inner);
    let Some((first, rest)) = stmts.split_first() else {
        return false;
    };
    canonical_door_open(first, axis_bool) && rest.iter().all(|s| bare_return(s))
}

/// `~open_and_close_door2(<loc>, $<axis>, door_open);` as a whole statement.
/// `<loc>` is `loc_N` or a script identifier; the axis name must be the
/// check-axis boolean this arm proved. An indiscriminate `~open_` prefix
/// is not enough.
fn canonical_door_open(stmt: &str, axis_bool: &str) -> bool {
    let flat: String = stmt.chars().filter(|c| !c.is_whitespace()).collect();
    let flat = flat.strip_suffix(';').unwrap_or(flat.as_str());
    let Some(args) = flat
        .strip_prefix("~open_and_close_door2(")
        .and_then(|s| s.strip_suffix(')'))
    else {
        return false;
    };
    let mut parts = args.split(',');
    match (parts.next(), parts.next(), parts.next(), parts.next()) {
        (Some(open_loc), Some(axis), Some("door_open"), None) => {
            axis == axis_bool && loc_open_arg(open_loc)
        }
        _ => false,
    }
}

fn loc_open_arg(s: &str) -> bool {
    if let Some(n) = s.strip_prefix("loc_") {
        !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit())
    } else {
        script_ident(s)
    }
}

fn bare_return(stmt: &str) -> bool {
    let flat: String = stmt.chars().filter(|c| !c.is_whitespace()).collect();
    flat == "return;" || flat == "return"
}

/// A door head `$<b> = <true|false> | ~<proc> >= ^<const>` — exactly two
/// disjuncts, one of each, no nesting and no `&` — → the boolean's value and
/// the proc comparison's `(proc, const)` names.
pub(super) fn check_axis_or_proc(head: &str, axis_bool: &str) -> Option<(bool, String, String)> {
    if head.contains(['(', ')', '&']) {
        return None;
    }
    let mut disjuncts = head.split('|');
    let (first, second) = (disjuncts.next()?.trim(), disjuncts.next()?.trim());
    if disjuncts.next().is_some() {
        return None;
    }
    let mut boolean = None;
    let mut compare = None;
    for disjunct in [first, second] {
        if let Some(value) = boolean_value_disjunct(disjunct, axis_bool) {
            if boolean.replace(value).is_some() {
                return None;
            }
        } else {
            let pair = proc_const_disjunct(disjunct)?;
            if compare.replace(pair).is_some() {
                return None;
            }
        }
    }
    let (proc, cname) = compare?;
    Some((boolean?, proc, cname))
}

/// `$<name> = <true|false>` → the value; any other left side is not the
/// check-axis test.
fn boolean_value_disjunct(disjunct: &str, name: &str) -> Option<bool> {
    let (lhs, rhs) = disjunct.split_once('=')?;
    if lhs.trim() != name {
        return None;
    }
    match rhs.trim() {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

/// `~<proc>` (or `~<proc>()`) `>= ^<const>` → the `(proc, const)` names.
fn proc_const_disjunct(disjunct: &str) -> Option<(String, String)> {
    let (lhs, rhs) = disjunct.split_once(">=")?;
    let proc = lhs.trim().strip_prefix('~')?.trim();
    let proc = proc.strip_suffix("()").unwrap_or(proc).trim_end();
    let cname = rhs.trim().strip_prefix('^')?;
    if !script_ident(proc) || !script_ident(cname) {
        return None;
    }
    Some((proc.to_string(), cname.to_string()))
}

/// A script identifier: `[A-Za-z0-9_]+`.
fn script_ident(name: &str) -> bool {
    !name.is_empty() && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

/// The `(varp id, lo, hi)` of a `[proc,<name>](…)(…)` block whose body is
/// exactly `return (getbit_range(%<varp>, ^<lo>, ^<hi>));` (whitespace-tolerant),
/// with `<varp>` in `pack/varp.pack` and `<lo>`/`<hi>` in the script
/// constants. A missing, duplicated, or differently-shaped proc — another
/// varp, another read, an extra statement — yields `None`.
pub(super) fn proc_bitfield_varp(
    script_text: &str,
    name: &str,
    constants: &HashMap<String, i32>,
    varps: &HashMap<String, i32>,
) -> Option<(i32, i32, i32)> {
    let bodies = proc_bodies(script_text, name);
    let [body] = bodies.as_slice() else {
        return None;
    };
    let flat: String = body.chars().filter(|c| !c.is_whitespace()).collect();
    let args = flat
        .strip_prefix("return(getbit_range(")?
        .strip_suffix("));")?;
    let mut parts = args.split(',');
    let (Some(varp), Some(lo), Some(hi)) = (parts.next(), parts.next(), parts.next()) else {
        return None;
    };
    if parts.next().is_some() {
        return None;
    }
    let varp = varp.strip_prefix('%')?;
    let lo = lo.strip_prefix('^')?;
    let hi = hi.strip_prefix('^')?;
    Some((*varps.get(varp)?, *constants.get(lo)?, *constants.get(hi)?))
}

/// Every `[proc,<name>](…)(…)` body in a script text. [`script_blocks`]
/// cannot see these headers — a proc header carries its argument list and
/// return type after the closing bracket — so the body is delimited here,
/// from the header line to the next `[` line.
pub(super) fn proc_bodies(script_text: &str, name: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = false;
    let mut body = String::new();
    for raw in script_text.lines() {
        let line = raw.trim();
        if let Some(rest) = line.strip_prefix("[proc,") {
            if cur {
                out.push(std::mem::take(&mut body));
            }
            cur = rest.split_once(']').is_some_and(|(n, _)| n.trim() == name);
        } else if cur && line.starts_with('[') {
            out.push(std::mem::take(&mut body));
            cur = false;
        } else if cur {
            body.push_str(line);
            body.push('\n');
        }
    }
    if cur {
        out.push(body);
    }
    out
}

/// `[<a>,<b>]` blocks in a script text → `(a, b, body)`; the body is the
/// raw text until the next header. Headers may carry a trailing `//`
/// comment (the existing parsers' convention).
pub(super) fn script_blocks(text: &str) -> Vec<(String, String, String)> {
    let mut out = Vec::new();
    let mut cur: Option<(String, String)> = None;
    let mut body = String::new();
    for raw in text.lines() {
        let line = raw.trim();
        let header_line = match line.find("//") {
            Some(i) => line[..i].trim(),
            None => line,
        };
        if let Some((a, b)) = script_header(header_line) {
            if let Some(prev) = cur.take() {
                out.push((prev.0, prev.1, std::mem::take(&mut body)));
            }
            cur = Some((a.to_string(), b.to_string()));
        } else if cur.is_some() {
            body.push_str(line);
            body.push('\n');
        }
    }
    if let Some((a, b)) = cur.take() {
        out.push((a, b, body));
    }
    out
}

/// The `(varp name, min value)` gates a door's open script declares: a
/// `switch_int(%<varp>)` whose opening cases carry the open call, or an
/// `if (%<varp> (>=|=) ^<const> & …)` whose arm opens (every ANDed varp
/// condition is carried).
pub(super) fn script_varp_gate(
    block: &str,
    script_text: &str,
    constants: &HashMap<String, i32>,
) -> Option<Vec<(String, i32)>> {
    if let Some((varp, cases)) = switch_varp_cases(block) {
        let mut opens = Vec::new();
        for (keys, body) in cases {
            if !body_opens(&body, script_text) {
                continue;
            }
            for key in keys {
                if let Some(v) = case_value(&key, constants) {
                    opens.push(v);
                }
            }
        }
        if let Some(min) = opens.into_iter().min() {
            return Some(vec![(varp, min)]);
        }
    }
    if let Some((gates, arm)) = if_varp_gate(block, constants) {
        if body_opens(&arm, script_text) {
            return Some(gates);
        }
    }
    None
}

/// `^<const>` (resolved through the constants map) or a bare integer.
fn case_value(key: &str, constants: &HashMap<String, i32>) -> Option<i32> {
    let key = key.trim();
    if let Some(name) = key.strip_prefix('^') {
        constants.get(name).copied()
    } else {
        key.parse().ok()
    }
}

/// `switch_int (%<varp>) { case … }` from a block: the varp name and every
/// case's `(keys, body)`. `default` is kept as a key so a gate that only
/// opens on `default` can never match (it has no numeric keys).
type SwitchVarpCases = (String, Vec<(Vec<String>, String)>);
fn switch_varp_cases(block: &str) -> Option<SwitchVarpCases> {
    let lines: Vec<&str> = block.lines().collect();
    let mut si = None;
    for (idx, raw) in lines.iter().enumerate() {
        let line = raw.trim();
        let Some(rest) = line.strip_prefix("switch_int") else {
            continue;
        };
        let rest = rest.trim_start().strip_prefix('(')?;
        let name = rest.split(')').next()?.trim();
        if let Some(name) = name.strip_prefix('%') {
            si = Some((idx, name.to_string()));
            break;
        }
    }
    let (start, varp) = si?;
    let mut cases: Vec<(Vec<String>, String)> = Vec::new();
    let mut depth = 0i32;
    let mut cur: Option<Vec<String>> = None;
    let mut body = String::new();
    for raw in lines.iter().skip(start + 1) {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        // A `case` at the switch's own brace depth starts a new case; the
        // switch's closing `}` at that depth ends the parse.
        if depth == 0 {
            if let Some((keys, rest)) = switch_case(line) {
                if let Some(prev) = cur.take() {
                    cases.push((prev, std::mem::take(&mut body)));
                }
                cur = Some(keys);
                if let Some(rest) = rest {
                    body.push_str(rest);
                    body.push('\n');
                }
                continue;
            }
            if line.starts_with('}') {
                break;
            }
        }
        if cur.is_some() {
            body.push_str(line);
            body.push('\n');
        }
        depth += line.matches('{').count() as i32 - line.matches('}').count() as i32;
    }
    if let Some(keys) = cur.take() {
        cases.push((keys, body));
    }
    Some((varp, cases))
}

/// `case <key, key, …> : <body>` → (keys, body). `default` is a key.
fn switch_case(line: &str) -> Option<(Vec<String>, Option<&str>)> {
    let rest = line.strip_prefix("case")?;
    let rest = rest.trim_start();
    let (keys, body) = rest.split_once(':')?;
    let keys: Vec<String> = keys
        .split(',')
        .map(|k| k.trim().to_string())
        .filter(|k| !k.is_empty())
        .collect();
    if keys.is_empty() {
        return None;
    }
    let body = body.trim();
    Some((keys, if body.is_empty() { None } else { Some(body) }))
}

/// `if (%<varp> (>=|=) ^<const> [& …]) { <arm> }` in a block: every
/// `(varp, const value)` condition and the arm body. The first matching
/// guard is read.
fn if_varp_gate(
    block: &str,
    constants: &HashMap<String, i32>,
) -> Option<(Vec<(String, i32)>, String)> {
    let bytes = block.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i..].starts_with(b"if") {
            let tail = &block[i + 2..];
            let rest = tail.trim_start();
            let ws = tail.len() - rest.len();
            if let Some(inner) = rest.strip_prefix('(') {
                if let Some(close) = inner.find(')') {
                    let head = inner[..close].trim();
                    let conds = varp_gate_consts(head);
                    if !conds.is_empty() {
                        let mut gates = Vec::new();
                        for (varp, cname) in conds {
                            if let Some(&value) = constants.get(&cname) {
                                gates.push((varp.to_string(), value));
                            }
                        }
                        if !gates.is_empty() {
                            let arm_from = i + 2 + ws + 1 + close + 1;
                            if let Some(arm) = balanced_arm(block, arm_from) {
                                return Some((gates, arm));
                            }
                        }
                    }
                }
            }
        }
        i += 1;
    }
    None
}

/// `%<varp> (>=|=) ^<const>` conditions in an if-head (ANDed together),
/// whitespace tolerant; clauses that are not varp-gated (e.g. a `|`
/// leaving-side term) are skipped.
fn varp_gate_consts(head: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for clause in head.split('&') {
        let clause = clause.trim();
        let (varp, cmp) = if let Some((a, b)) = clause.split_once(">=") {
            (a, b)
        } else if let Some((a, b)) = clause.split_once('=') {
            (a, b)
        } else {
            continue;
        };
        let varp = varp.trim();
        let Some(varp) = varp.strip_prefix('%') else {
            continue;
        };
        let cmp = cmp.trim();
        let Some(cmp) = cmp.strip_prefix('^') else {
            continue;
        };
        let end = cmp
            .find(|c: char| c.is_whitespace() || c == '|')
            .unwrap_or(cmp.len());
        let cname = &cmp[..end];
        if !cname.is_empty() {
            out.push((varp.to_string(), cname.to_string()));
        }
    }
    out
}

/// The balanced `{ … }` starting at or after `from`.
fn balanced_arm(block: &str, from: usize) -> Option<String> {
    let rest = &block[from.min(block.len())..];
    let start = rest.find('{')?;
    let mut depth = 1i32;
    for (off, ch) in rest[start + 1..].char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(rest[start..=start + 1 + off].to_string());
                }
            }
            _ => {}
        }
    }
    None
}

/// True when a case/if arm opens the door: it calls an `~open*` proc
/// directly, or calls a `@label` whose own body does.
fn body_opens(body: &str, script_text: &str) -> bool {
    if body.contains("open_and_close") || body.contains("~open_") {
        return true;
    }
    for label in body_labels(body) {
        if let Some(lb) = label_block(script_text, &label) {
            if body_opens(&lb, script_text) {
                return true;
            }
        }
    }
    false
}

/// `@name` tokens in a body.
pub(super) fn body_labels(body: &str) -> Vec<String> {
    let mut out = Vec::new();
    for part in body.split('@').skip(1) {
        let end = part
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
            .unwrap_or(part.len());
        if end > 0 {
            out.push(part[..end].to_string());
        }
    }
    out
}

/// The body of `[label,<name>]` in a script text.
fn label_block(script_text: &str, name: &str) -> Option<String> {
    for (op, n, body) in script_blocks(script_text) {
        if op == "label" && n == name {
            return Some(body);
        }
    }
    None
}

/// `[a,b]` where both parts are identifiers.
pub(super) fn script_header(line: &str) -> Option<(&str, &str)> {
    let inner = line.strip_prefix('[')?.strip_suffix(']')?;
    let (a, b) = inner.split_once(',')?;
    let word = |s: &str| !s.is_empty() && s.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_');
    if !word(a) || !word(b) {
        return None;
    }
    Some((a, b))
}
