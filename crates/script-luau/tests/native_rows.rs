//! The native-row witness rule and promotion counts of ADR 0023 items 31–32.
//! The declarations own the row set. The ledger owns the inspected associations,
//! not a second row inventory. Source citations accompany those associations.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use script_luau::Kind;

const ROWS: &str = include_str!("native_rows/rows.txt");
const RS2B0T_WITNESSES: &[(&str, &str)] =
    &[("CastNpc", "Game.castOnNpc"), ("Shop", "Shop.buyById")];
// Existing gaps only. A new row must have a complete script-surface pair.
const NO_LIVE_CELL_ROWS: &[&str] = &[
    "CastNpc",
    "deposit",
    "open-booth",
    "puzzle-move",
    "set-note-mode",
    "withdraw",
    "withdraw-x",
];

struct Sources {
    seats: String,
    types: String,
    frozen: String,
    live_map: String,
}

impl Sources {
    fn read() -> Self {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let read = |path: &str| {
            std::fs::read_to_string(root.join(path))
                .unwrap_or_else(|error| panic!("{path}: {error}"))
        };
        Self {
            seats: read("crates/script/src/native/seat_rows.rs"),
            types: read("crates/script-js/host-js/index.d.ts"),
            frozen: read("crates/script-js/tests/compat_v2_freeze.members"),
            live_map: read("tools/live-map/map.toml"),
        }
    }
}

fn seat_rows(source: &str) -> Result<BTreeSet<String>, String> {
    let file = syn::parse_file(source).map_err(|error| format!("seat_rows.rs: {error}"))?;
    let mut declarations = file.items.iter().filter_map(|item| match item {
        syn::Item::Enum(item) if item.ident == "SeatStartArgs" => Some(item),
        _ => None,
    });
    let declaration = declarations.next().ok_or("SeatStartArgs is missing")?;
    if declarations.next().is_some() {
        return Err("SeatStartArgs has multiple declarations".into());
    }
    let rows: BTreeSet<_> = declaration
        .variants
        .iter()
        .filter(|variant| variant.ident != "Empty")
        .map(|variant| variant.ident.to_string())
        .collect();
    if rows.is_empty() {
        return Err("SeatStartArgs has no native rows".into());
    }
    Ok(rows)
}

// Tokenize before examining declarations: comments and quoted examples are not members.
fn ts_tokens(source: &str) -> Result<Vec<&str>, String> {
    let bytes = source.as_bytes();
    let mut tokens = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let start = i;
        match bytes[i] {
            byte if byte.is_ascii_whitespace() => i += 1,
            b'/' if bytes.get(i + 1) == Some(&b'/') => {
                i += 2;
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
            }
            b'/' if bytes.get(i + 1) == Some(&b'*') => {
                let end = source[i + 2..]
                    .find("*/")
                    .ok_or("unterminated TypeScript comment")?;
                i += end + 4;
            }
            quote @ (b'\'' | b'"' | b'`') => {
                i += 1;
                while i < bytes.len() && bytes[i] != quote {
                    if bytes[i] == b'\\' {
                        i += 1;
                    }
                    i += 1;
                }
                if i >= bytes.len() {
                    return Err("unterminated TypeScript string".into());
                }
                i += 1;
                tokens.push(&source[start..i]);
            }
            byte if byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'$') => {
                i += 1;
                while i < bytes.len()
                    && (bytes[i].is_ascii_alphanumeric() || matches!(bytes[i], b'_' | b'$'))
                {
                    i += 1;
                }
                tokens.push(&source[start..i]);
            }
            byte if byte.is_ascii() => {
                i += 1;
                tokens.push(&source[start..i]);
            }
            _ => return Err("unsupported non-ASCII TypeScript token".into()),
        }
    }
    Ok(tokens)
}

fn declaration_start(tokens: &[&str], header: &[&str]) -> Result<usize, String> {
    let mut matches = tokens
        .windows(header.len())
        .enumerate()
        .filter(|(_, window)| *window == header);
    let (start, _) = matches
        .next()
        .ok_or_else(|| format!("missing declaration: {}", header.join(" ")))?;
    if matches.next().is_some() {
        return Err(format!("duplicate declaration: {}", header.join(" ")));
    }
    Ok(start + header.len())
}

fn group_end(tokens: &[&str], start: usize) -> Result<usize, String> {
    let close = match tokens.get(start) {
        Some(&"{") => "}",
        Some(&"(") => ")",
        Some(&"[") => "]",
        _ => return Err("expected a TypeScript group".into()),
    };
    let mut i = start + 1;
    while let Some(&token) = tokens.get(i) {
        if token == close {
            return Ok(i);
        }
        match token {
            "{" | "(" | "[" => i = group_end(tokens, i)?,
            "}" | ")" | "]" => return Err("mismatched TypeScript group".into()),
            _ => {}
        }
        i += 1;
    }
    Err("unterminated TypeScript group".into())
}

fn members<'a, 's>(tokens: &'a [&'s str]) -> Result<Vec<(&'s str, &'a [&'s str])>, String> {
    let mut result = Vec::new();
    let mut i = 0;
    while i < tokens.len() {
        if tokens[i] == "readonly" {
            i += 1;
        }
        let name = *tokens.get(i).ok_or("missing TypeScript member name")?;
        if !name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'$'))
        {
            return Err(format!("unsupported TypeScript member name: {name}"));
        }
        i += 1;
        if tokens.get(i) == Some(&"?") {
            i += 1;
        }
        let body = match tokens.get(i) {
            Some(&":") => i + 1,
            Some(&"(") => i,
            _ => return Err(format!("unsupported TypeScript member: {name}")),
        };
        i = body;
        while i < tokens.len() && tokens[i] != ";" {
            if matches!(tokens[i], "{" | "(" | "[") {
                i = group_end(tokens, i)?;
            }
            i += 1;
        }
        if body == i || result.iter().any(|(previous, _)| *previous == name) {
            return Err(format!("empty or duplicate TypeScript member: {name}"));
        }
        result.push((name, &tokens[body..i]));
        i += usize::from(i < tokens.len());
    }
    Ok(result)
}

fn native_ops(tokens: &[&str]) -> Result<BTreeSet<String>, String> {
    let mut i = declaration_start(tokens, &["export", "type", "NativeOp", "="])?;
    let mut ops = BTreeSet::new();
    loop {
        if tokens.get(i) == Some(&"|") {
            i += 1;
        }
        if tokens.get(i) != Some(&"{") {
            return Err("NativeOp must be a union of object arms".into());
        }
        let end = group_end(tokens, i)?;
        let fields = members(&tokens[i + 1..end])?;
        let (_, value) = fields
            .iter()
            .find(|(name, _)| *name == "op")
            .ok_or("NativeOp arm has no op discriminant")?;
        let [literal] = *value else {
            return Err("NativeOp op must be one string literal".into());
        };
        let op = literal
            .strip_prefix('\'')
            .and_then(|text| text.strip_suffix('\''))
            .or_else(|| {
                literal
                    .strip_prefix('"')
                    .and_then(|text| text.strip_suffix('"'))
            })
            .filter(|text| {
                !text.is_empty() && text.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
            })
            .ok_or("NativeOp op must be a nonempty, unescaped string literal")?;
        if !ops.insert(op.to_owned()) {
            return Err(format!("duplicate NativeOp: {op}"));
        }
        i = end + 1;
        match tokens.get(i) {
            Some(&"|") => {}
            Some(&";") => return Ok(ops),
            _ => return Err("unsupported NativeOp union syntax".into()),
        }
    }
}

fn api_members(tokens: &[&str]) -> Result<BTreeSet<String>, String> {
    fn collect(tokens: &[&str], prefix: &str, out: &mut BTreeSet<String>) -> Result<(), String> {
        for (name, value) in members(tokens)? {
            let path = format!("{prefix}.{name}");
            if value.first() == Some(&"{") {
                let end = group_end(value, 0)?;
                if end + 1 != value.len() {
                    return Err(format!("unsupported object member type: {path}"));
                }
                collect(&value[1..end], &path, out)?;
            }
            out.insert(path);
        }
        Ok(())
    }
    let start = declaration_start(tokens, &["export", "interface", "NativeApi"])?;
    let end = group_end(tokens, start)?;
    let mut result = BTreeSet::new();
    collect(&tokens[start + 1..end], "NativeApi", &mut result)?;
    if result.is_empty() {
        return Err("NativeApi has no members".into());
    }
    Ok(result)
}

fn live_cells(source: &str) -> Result<BTreeSet<String>, String> {
    let document: toml_edit::DocumentMut = source
        .parse()
        .map_err(|error| format!("invalid live-map TOML: {error}"))?;
    let covers = document["cover"]
        .as_array_of_tables()
        .ok_or("live map has no cover tables")?;
    let mut commands = BTreeMap::new();
    for cover in covers {
        let cells = cover["cells"]
            .as_array()
            .ok_or("live-map cover has no cells")?;
        for cell in cells {
            let spec = cell.as_str().ok_or("live-map cell is not text")?;
            let mut parts = spec.split('|');
            let id = parts.next().unwrap_or_default().trim();
            let command = parts.next().ok_or("live-map cell has no command")?.trim();
            if id.is_empty() || command.is_empty() {
                return Err("live-map cell has an empty id or command".into());
            }
            if let Some(previous) = commands.insert(id, command) {
                if previous != command {
                    return Err(format!("conflicting live-map commands: {id}"));
                }
            }
        }
    }
    Ok(commands
        .into_iter()
        .filter_map(|(id, command)| {
            (command.split_whitespace().next() == Some("LIVE=1")).then(|| id.to_owned())
        })
        .collect())
}

fn ledger(source: &str) -> Result<BTreeMap<&str, [&str; 4]>, String> {
    let mut rows = BTreeMap::new();
    for (number, line) in source.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let fields: Vec<_> = line.split_whitespace().collect();
        let [row, js, js_cell, luau, luau_cell] = fields.as_slice() else {
            return Err(format!("rows.txt:{}: expected five fields", number + 1));
        };
        if rows
            .insert(*row, [*js, *js_cell, *luau, *luau_cell])
            .is_some()
        {
            return Err(format!("duplicate ledger row: {row}"));
        }
    }
    Ok(rows)
}

fn check(sources: &Sources, text: &str) -> Result<(usize, usize), String> {
    let tokens = ts_tokens(&sources.types)?;
    let ops = native_ops(&tokens)?;
    let mut expected = seat_rows(&sources.seats)?;
    for op in &ops {
        if !expected.insert(op.clone()) {
            return Err(format!("seat and op row names collide: {op}"));
        }
    }
    let rows = ledger(text)?;
    for row in &expected {
        if !rows.contains_key(row.as_str()) {
            return Err(format!("missing ledger row: {row}"));
        }
    }
    for row in rows.keys() {
        if !expected.contains(*row) {
            return Err(format!("unknown ledger row: {row}"));
        }
    }
    let declared = api_members(&tokens)?;
    let mut frozen = BTreeSet::new();
    for line in sources
        .frozen
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
    {
        if !declared.contains(line) || !frozen.insert(line) {
            return Err(format!("invalid JSV2-FREEZE member: {line}"));
        }
    }
    if frozen.is_empty() {
        return Err("JSV2-FREEZE closed list is empty".into());
    }
    let cells = live_cells(&sources.live_map)?;
    let bindings: BTreeSet<_> = script_luau::api_members()
        .filter(|member| matches!(member.kind, Kind::Request | Kind::Await))
        .map(|member| member.path)
        .collect();
    let mut without_cells = BTreeSet::new();
    let mut promotion = 0;
    let mut luau_covered = 0;
    for (row, [js, js_cell, luau, luau_cell]) in rows {
        if js != "-" {
            if let Some(member) = js.strip_prefix("rs2b0t:") {
                if !RS2B0T_WITNESSES.contains(&(row, member)) {
                    return Err(format!("rs2b0t witness is not allowed: {row} {js}"));
                }
            } else if let Some(op) = js.strip_prefix("NativeOp:") {
                if !ops.contains(op) || op != row {
                    return Err(format!(
                        "undeclared or mismatched NativeOp witness: {row} {js}"
                    ));
                }
            } else if !declared.contains(js) {
                return Err(format!("undeclared NativeApi witness: {row} {js}"));
            } else if frozen.contains(js) {
                return Err(format!("compat-backed witness: {row} {js}"));
            }
        }
        if luau != "-" && !bindings.contains(luau) {
            return Err(format!("not a Request/Await Luau binding: {row} {luau}"));
        }
        for (witness, cell) in [(js, js_cell), (luau, luau_cell)] {
            if cell != "-" && (witness == "-" || !cells.contains(cell)) {
                return Err(format!("orphan or non-live cell: {row} {cell}"));
            }
        }
        let js_pair = js != "-" && js_cell != "-";
        let luau_pair = luau != "-" && luau_cell != "-";
        if !js_pair && !luau_pair {
            if !NO_LIVE_CELL_ROWS.contains(&row) || (js == "-" && luau == "-") {
                return Err(format!("no complete witness/live-cell pair: {row}"));
            }
            without_cells.insert(row);
        }
        if !js.starts_with("rs2b0t:") {
            promotion += 1;
            luau_covered += usize::from(luau_pair);
        }
    }
    let exceptions: BTreeSet<_> = NO_LIVE_CELL_ROWS.iter().copied().collect();
    if without_cells != exceptions {
        return Err(format!(
            "stale no-live-cell exceptions: {exceptions:?}; actual: {without_cells:?}"
        ));
    }
    Ok((promotion, luau_covered))
}

#[test]
fn native_rows_have_script_surface_witnesses() {
    let (promotion, luau) = check(&Sources::read(), ROWS).unwrap_or_else(|error| panic!("{error}"));
    println!("promotion rows: {promotion}; with a Luau binding and a Luau live cell: {luau}");
}

fn replace_once(text: &str, before: &str, after: &str) -> String {
    assert_eq!(
        text.matches(before).count(),
        1,
        "fixture must alter one input"
    );
    text.replacen(before, after, 1)
}

fn refused(sources: &Sources, rows: &str, expected: &str) {
    let error = check(sources, rows).expect_err("negative fixture must be refused");
    assert!(
        error.contains(expected),
        "expected {expected:?}, got {error:?}"
    );
}

#[test]
fn a_new_seat_without_a_ledger_line_is_refused() {
    let mut sources = Sources::read();
    sources.seats = replace_once(
        &sources.seats,
        "pub enum SeatStartArgs {",
        "pub enum SeatStartArgs { FixtureSeat,",
    );
    refused(&sources, ROWS, "missing ledger row: FixtureSeat");
}

#[test]
fn a_new_op_without_a_ledger_line_is_refused() {
    let mut sources = Sources::read();
    sources.types = replace_once(
        &sources.types,
        "export type NativeOp =",
        "export type NativeOp = | { op: 'fixture-op' }",
    );
    refused(&sources, ROWS, "missing ledger row: fixture-op");
}

#[test]
fn a_witness_without_a_live_cell_is_refused() {
    let rows = replace_once(
        ROWS,
        "Gather NativeApi.gather.run host.script-api-gather-and-quest",
        "Gather NativeApi.gather.run -",
    );
    refused(
        &Sources::read(),
        &rows,
        "no complete witness/live-cell pair: Gather",
    );
}

#[test]
fn a_third_rs2b0t_row_is_refused() {
    let rows = replace_once(
        ROWS,
        "Gather NativeApi.gather.run",
        "Gather rs2b0t:Game.castOnNpc",
    );
    refused(
        &Sources::read(),
        &rows,
        "rs2b0t witness is not allowed: Gather",
    );
}

#[test]
fn a_compat_backed_witness_is_refused() {
    let rows = replace_once(
        ROWS,
        "Gather NativeApi.gather.run",
        "Gather NativeApi.prayerFull",
    );
    refused(&Sources::read(), &rows, "compat-backed witness: Gather");
}

#[test]
fn invalid_ledger_evidence_is_refused() {
    let sources = Sources::read();
    let cases = [
        (
            format!("{ROWS}\nFixture - - - -\n"),
            "unknown ledger row: Fixture",
        ),
        (
            format!("{ROWS}\nGather - - - -\n"),
            "duplicate ledger row: Gather",
        ),
        (
            replace_once(
                ROWS,
                "Gather NativeApi.gather.run",
                "Gather NativeApi.noSuchMember",
            ),
            "undeclared NativeApi witness",
        ),
        (
            replace_once(
                ROWS,
                "Gather NativeApi.gather.run",
                "Gather NativeApi.quest",
            ),
            "undeclared NativeApi witness",
        ),
        (
            replace_once(ROWS, "walk NativeOp:walk", "walk NativeOp:fixture-op"),
            "undeclared or mismatched NativeOp witness",
        ),
        (
            replace_once(
                ROWS,
                "bank.openNearest host.luau-bank-interact",
                "status.tick host.luau-bank-interact",
            ),
            "not a Request/Await Luau binding",
        ),
        (
            replace_once(
                ROWS,
                "bank.openNearest host.luau-bank-interact",
                "bank.noSuchBinding host.luau-bank-interact",
            ),
            "not a Request/Await Luau binding",
        ),
        (
            replace_once(
                ROWS,
                "bank.openNearest host.luau-bank-interact",
                "- host.luau-bank-interact",
            ),
            "orphan or non-live cell",
        ),
        (
            replace_once(
                ROWS,
                "bank.openNearest host.luau-bank-interact",
                "bank.openNearest missing-cell",
            ),
            "orphan or non-live cell",
        ),
        (
            replace_once(
                ROWS,
                "bank.openNearest host.luau-bank-interact",
                "bank.openNearest script.load-headless",
            ),
            "orphan or non-live cell",
        ),
    ];
    for (rows, expected) in cases {
        refused(&sources, &rows, expected);
    }
    let mut sources = sources;
    sources
        .live_map
        .push_str("\n[[cover]]\nglobs = []\ncells = ['host.luau-walk|not-live']\n");
    refused(&sources, ROWS, "conflicting live-map commands");
}

#[test]
fn comments_and_unrelated_types_cannot_declare_witnesses() {
    let mut sources = Sources::read();
    sources.types.push_str("\n// export type NativeOp = | { op: 'comment-op' };\n/* export interface NativeApi { noSuchMember(): void; } */\nexport interface Unrelated { noSuchMember(): void; }\n");
    let rows = replace_once(
        ROWS,
        "Gather NativeApi.gather.run",
        "Gather NativeApi.noSuchMember",
    );
    refused(&sources, &rows, "undeclared NativeApi witness");
}
