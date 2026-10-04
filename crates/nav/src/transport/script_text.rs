// ---------------------------------------------------------------------------
// Script text helpers (m8aq regexes ported without a regex dependency).
// ---------------------------------------------------------------------------
/// A parsed `[kind,name]` header and its uninterpreted tail.
pub(super) struct ScriptHeader<'a> {
    pub(super) kind: &'a str,
    pub(super) name: &'a str,
    pub(super) tail: &'a str,
}

/// Whitespace and secondary-subject policy for the shared script-header grammar.
#[derive(Clone, Copy)]
pub(super) enum ScriptHeaderStyle {
    Exact,
    Trimmed,
    Secondary,
}

/// Split one `[kind,name]` header and preserve the uninterpreted tail.
pub(super) fn split_script_header(line: &str) -> Option<ScriptHeader<'_>> {
    let rest = line.strip_prefix('[')?;
    let (inner, tail) = rest.split_once(']')?;
    let (kind, name) = inner.split_once(',')?;
    Some(ScriptHeader { kind, name, tail })
}

/// Validate a split `[kind,name]` header under the caller's whitespace and
/// secondary-subject policy.
pub(super) fn parse_script_header(
    line: &str,
    style: ScriptHeaderStyle,
) -> Option<ScriptHeader<'_>> {
    let ScriptHeader {
        mut kind,
        mut name,
        tail,
    } = split_script_header(line)?;
    if matches!(style, ScriptHeaderStyle::Trimmed) {
        kind = kind.trim();
        name = name.trim();
    }
    let word = |value: &str| {
        !value.is_empty()
            && value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
    };
    let name_word = if matches!(style, ScriptHeaderStyle::Secondary) {
        name.strip_prefix('.').unwrap_or(name)
    } else {
        name
    };
    if !word(kind) || !word(name_word) {
        return None;
    }
    Some(ScriptHeader { kind, name, tail })
}

pub(super) enum ScriptBlockLine<'a, T> {
    Header {
        value: T,
        inline_body: Option<&'a str>,
        inline_body_newline: bool,
        keep: bool,
    },
    Boundary,
    Body(&'a str),
    BodyFollowing(&'a str),
    Ignore,
}

/// Shared block-body accumulator for the transport script-text readers.
/// Callers retain their existing header/comment policies in the classifier.
pub(super) fn collect_script_blocks<'a, T>(
    text: &'a str,
    mut classify: impl FnMut(&'a str) -> ScriptBlockLine<'a, T>,
) -> Vec<(T, String)> {
    let mut out = Vec::new();
    let mut current: Option<(T, String, bool)> = None;
    for raw in text.lines() {
        match classify(raw) {
            ScriptBlockLine::Header {
                value,
                inline_body,
                inline_body_newline,
                keep,
            } => {
                if let Some((previous, body, keep)) = current.take() {
                    if keep {
                        out.push((previous, body));
                    }
                }
                let mut body = String::new();
                if let Some(inline) = inline_body {
                    body.push_str(inline);
                    if inline_body_newline {
                        body.push('\n');
                    }
                }
                current = Some((value, body, keep));
            }
            ScriptBlockLine::Boundary => {
                if let Some((previous, body, keep)) = current.take() {
                    if keep {
                        out.push((previous, body));
                    }
                }
            }
            ScriptBlockLine::Body(line) => {
                if let Some((_, body, true)) = current.as_mut() {
                    body.push_str(line);
                    body.push('\n');
                }
            }
            ScriptBlockLine::BodyFollowing(line) => {
                if let Some((_, body, true)) = current.as_mut() {
                    body.push('\n');
                    body.push_str(line);
                }
            }
            ScriptBlockLine::Ignore => {}
        }
    }
    if let Some((last, body, keep)) = current {
        if keep {
            out.push((last, body));
        }
    }
    out
}

/// A block body normalized for exact comparison: `//` comments dropped and
/// all whitespace removed.
pub(super) fn normalized_body(body: &str) -> String {
    let mut out = String::new();
    for raw in body.lines() {
        let line = match raw.find("//") {
            Some(i) => &raw[..i],
            None => raw,
        };
        out.extend(line.chars().filter(|c| !c.is_whitespace()));
    }
    out
}

/// Every `[oploc1,_<category>]` category handler in a script text →
/// `(category, body)` (the category without its `_`). These inline bodies use
/// the shared accumulator's raw tail and preceding-newline policy.
pub(super) fn oploc1_category_bodies(text: &str) -> Vec<(String, String)> {
    super::script_text::collect_script_blocks(text, |raw| {
        let line = raw
            .split_once("//")
            .map_or(raw, |(before, _)| before)
            .trim();
        if let Some(header) = super::script_text::split_script_header(line) {
            let Some(category) = header.name.trim().strip_prefix('_') else {
                return super::script_text::ScriptBlockLine::Boundary;
            };
            if header.kind.trim() != "oploc1" || category.is_empty() {
                return super::script_text::ScriptBlockLine::Boundary;
            }
            return super::script_text::ScriptBlockLine::Header {
                value: category.to_string(),
                inline_body: Some(header.tail),
                inline_body_newline: false,
                keep: true,
            };
        }
        if line
            .strip_prefix('[')
            .is_some_and(|rest| rest.contains(']'))
        {
            return super::script_text::ScriptBlockLine::Boundary;
        }
        super::script_text::ScriptBlockLine::BodyFollowing(line)
    })
    .into_iter()
    .collect()
}

/// `[<name>]` config block header → the name.
pub(super) fn config_header(line: &str) -> Option<&str> {
    let name = line.strip_prefix('[')?.strip_suffix(']')?;
    if name.is_empty() || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
        return None;
    }
    Some(name)
}

pub(super) enum SwitchKind {
    Coord,
    Int,
}

pub(super) fn named_loc_has_open(text: &str, name: &str) -> bool {
    let mut in_block = false;
    let mut open = false;
    for raw in text.lines() {
        let line = raw.trim();
        if let Some(header) = config_header(line) {
            if in_block {
                return open;
            }
            in_block = header == name;
            open = false;
            continue;
        }
        if in_block && line == "op1=Open" {
            open = true;
        }
    }
    in_block && open
}

/// `oplocN` → `N`.
pub(super) fn oploc_option(header: &str) -> Option<i32> {
    let rest = header.strip_prefix("oploc")?;
    if rest.is_empty() || !rest.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    rest.parse().ok()
}

/// `def_coord $name = loc_coord[()]` → `$name`.
pub(super) fn def_coord_alias(line: &str) -> Option<String> {
    let (lhs, rhs) = line.split_once('=')?;
    let lhs = lhs.trim();
    let (kw, name) = lhs.split_once(char::is_whitespace)?;
    if kw != "def_coord" {
        return None;
    }
    let name = name.trim();
    if !name.starts_with('$') || name.len() == 1 {
        return None;
    }
    if !name
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'$' || b == b'_')
    {
        return None;
    }
    if !rhs.trim_start().starts_with("loc_coord") {
        return None;
    }
    Some(name.to_string())
}

/// `switch_(coord|int) (target) {` → `(kind, target)`.
pub(super) fn switch_kind(line: &str) -> Option<(SwitchKind, String)> {
    let rest = line.strip_prefix("switch_")?;
    let (kind, rest) = if let Some(r) = rest.strip_prefix("coord") {
        (SwitchKind::Coord, r)
    } else {
        let r = rest.strip_prefix("int")?;
        (SwitchKind::Int, r)
    };
    let after = rest.as_bytes().first();
    if !matches!(after, None | Some(b' ') | Some(b'\t') | Some(b'(')) {
        return None;
    }
    let inner = rest.trim_start().strip_prefix('(')?.split(')').next()?;
    Some((kind, inner.trim().to_string()))
}

/// `case <key> : <body>` with a `default`/coord-literal/int key.
pub(super) fn case_parts(line: &str) -> Option<(&str, &str)> {
    let rest = line.strip_prefix("case")?;
    let rest = rest.trim_start();
    let (key, body) = rest.split_once(':')?;
    let key = key.trim();
    if !case_key_valid(key) {
        return None;
    }
    Some((key, body.trim()))
}

pub(super) fn case_key_valid(key: &str) -> bool {
    if key == "default" {
        return true;
    }
    if key.is_empty() {
        return false;
    }
    let parts: Vec<&str> = key.split('_').collect();
    let digit_part = |p: &str| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit());
    if parts.len() == 1 {
        return digit_part(parts[0]);
    }
    parts.len() == 5 && parts.iter().all(|p| digit_part(p))
}

/// `if (target = <5-part coord literal>)` → `(target, literal)`.
pub(super) fn if_coord_target(line: &str) -> Option<(String, String)> {
    let rest = line.strip_prefix("if")?;
    let rest = rest.trim_start();
    let inner = rest.strip_prefix('(')?.split(')').next()?;
    let (target, value) = inner.split_once('=')?;
    let target = target.trim();
    if target.is_empty()
        || !target
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'$')
    {
        return None;
    }
    let value = value.trim();
    let parts: Vec<&str> = value.split('_').collect();
    if parts.len() != 5
        || !parts
            .iter()
            .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
    {
        return None;
    }
    Some((target.to_string(), value.to_string()))
}

/// `@name` preceded by start/whitespace/`:` → `name`.
pub(super) fn label_name(line: &str) -> Option<&str> {
    for (i, ch) in line.char_indices() {
        if ch != '@' {
            continue;
        }
        let prev_ok = i == 0 || matches!(line.as_bytes()[i - 1], b' ' | b'\t' | b':');
        if !prev_ok {
            continue;
        }
        let rest = &line[i + 1..];
        let end = rest
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
            .unwrap_or(rest.len());
        if end > 0 {
            return Some(&rest[..end]);
        }
    }
    None
}

/// Top-level args of `name(...)` in `text`, or None (m8aq `callArgs`).
pub(super) fn call_args(text: &str, name: &str) -> Option<Vec<String>> {
    let needle = format!("{name}(");
    let at = text.find(&needle)?;
    if at > 0 {
        let prev = text.as_bytes()[at - 1];
        if prev.is_ascii_alphanumeric() || prev == b'_' {
            return None;
        }
    }
    let after = &text[at + needle.len()..];
    let mut args = Vec::new();
    // Depth starts at 1: the call's own `(` was consumed by the needle.
    let mut depth = 1i32;
    let mut start = 0usize;
    for (i, ch) in after.char_indices() {
        match ch {
            '(' => depth += 1,
            ',' if depth == 1 => {
                args.push(after[start..i].trim().to_string());
                start = i + 1;
            }
            ')' => {
                depth -= 1;
                if depth == 0 {
                    args.push(after[start..i].trim().to_string());
                    return Some(args);
                }
            }
            _ => {}
        }
    }
    None
}

/// Every `name(...)` call's args in `text`, in source order (the
/// first-match [`call_args`] variant; the jewellery rub scripts carry one
/// teleport per `case`, and each must resolve).
pub(super) fn call_args_all(text: &str, name: &str) -> Vec<Vec<String>> {
    let needle = format!("{name}(");
    let mut out = Vec::new();
    let mut from = 0usize;
    while let Some(rel) = text[from..].find(&needle) {
        let at = from + rel;
        if at > 0 {
            let prev = text.as_bytes()[at - 1];
            if prev.is_ascii_alphanumeric() || prev == b'_' {
                from = at + needle.len();
                continue;
            }
        }
        let after = &text[at + needle.len()..];
        let mut args = Vec::new();
        let mut depth = 1i32;
        let mut start = 0usize;
        let mut closed = None;
        for (i, ch) in after.char_indices() {
            match ch {
                '(' => depth += 1,
                ',' if depth == 1 => {
                    args.push(after[start..i].trim().to_string());
                    start = i + 1;
                }
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        args.push(after[start..i].trim().to_string());
                        closed = Some(i + 1);
                        break;
                    }
                }
                _ => {}
            }
        }
        let Some(consumed) = closed else {
            break;
        };
        out.push(args);
        from = at + needle.len() + consumed;
    }
    out
}

/// `L_XHI_ZHI_XLO_ZLO` → `(level, x, z)` with `x = XHI<<6|XLO` (m8aq
/// `parseCoordLiteral`).
pub(super) fn coord_literal(text: &str) -> Option<(i32, i32, i32)> {
    let t = text.trim();
    let mut parts = t.split('_');
    let level = parts.next()?;
    let x_hi = parts.next()?;
    let z_hi = parts.next()?;
    let x_lo = parts.next()?;
    let z_lo = parts.next()?;
    if parts.next().is_some() {
        return None;
    }
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    if !(digits(level) && digits(x_hi) && digits(z_hi) && digits(x_lo) && digits(z_lo)) {
        return None;
    }
    Some((
        level.parse().ok()?,
        (x_hi.parse::<i32>().ok()? << 6) | x_lo.parse::<i32>().ok()?,
        (z_hi.parse::<i32>().ok()? << 6) | z_lo.parse::<i32>().ok()?,
    ))
}

/// `-?\d+` (m8aq `intOrNull`).
pub(super) fn int_or_null(text: &str) -> Option<i32> {
    let t = text.trim();
    let digits = t.strip_prefix('-').unwrap_or(t);
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    t.parse().ok()
}
#[cfg(test)]
mod tests {
    use super::{
        collect_script_blocks, oploc1_category_bodies, parse_script_header, ScriptBlockLine,
        ScriptHeaderStyle,
    };

    #[test]
    fn script_header_styles_preserve_the_tail_and_secondary_name() {
        let header = parse_script_header(
            "[oploc1,_gate_main_closed] ~open_gate;",
            ScriptHeaderStyle::Exact,
        )
        .unwrap();
        assert_eq!(
            (header.kind, header.name, header.tail),
            ("oploc1", "_gate_main_closed", " ~open_gate;")
        );
        assert!(
            parse_script_header("[proc,.open_gate](int $x)", ScriptHeaderStyle::Exact).is_none()
        );
        let secondary =
            parse_script_header("[proc,.open_gate](int $x)", ScriptHeaderStyle::Secondary).unwrap();
        assert_eq!((secondary.kind, secondary.name), ("proc", ".open_gate"));
    }

    #[test]
    fn shared_block_accumulator_keeps_inline_and_following_lines() {
        let blocks = collect_script_blocks("[proc,first] inline\n body\n[proc,second]", |line| {
            if let Some(header) = parse_script_header(line, ScriptHeaderStyle::Exact) {
                ScriptBlockLine::Header {
                    value: header.name.to_string(),
                    inline_body: Some(header.tail),
                    inline_body_newline: true,
                    keep: true,
                }
            } else {
                ScriptBlockLine::Body(line)
            }
        });
        assert_eq!(
            blocks,
            vec![
                ("first".to_string(), " inline\n body\n".to_string()),
                ("second".to_string(), "\n".to_string()),
            ]
        );
    }

    #[test]
    fn oploc1_categories_preserve_inline_and_raw_body_layout() {
        let blocks = oploc1_category_bodies(
            "[oploc1,_inline] ~open;\n[unfinished\n  @next;\n[oploc1,_next]\n@next;\n[oploc2,ignored]\n[oploc1,_last] ~last;",
        );
        assert_eq!(
            blocks,
            vec![
                (
                    "inline".to_string(),
                    " ~open;\n[unfinished\n@next;".to_string(),
                ),
                ("next".to_string(), "\n@next;".to_string()),
                ("last".to_string(), " ~last;".to_string()),
            ]
        );
    }
}
