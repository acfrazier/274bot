//! The native-row witness rule and promotion counts of ADR 0023 items 31–32.
//! The declarations own the row set. The ledger owns the inspected associations,
//! not a second row inventory. Source citations accompany those associations.
//!
//! Accepted limits: an op receives credit from its `ApiMember.js` label, not
//! from the request its body issues. A false label remains a review risk.
//! The Bank scan counts `host.bank` calls in dead code. It checks syntax,
//! not reachability. Diff review covers deliberate false edits and obfuscation.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use deno_ast::swc::ast::CallExpr;
use deno_ast::swc::ecma_visit::{Visit as JsVisit, VisitWith};
use deno_ast::{ProgramRef, SourceRangedForSpanned};
use script_luau::Kind;
use syn::visit::Visit;

const ROWS: &str = include_str!("native_rows/rows.txt");
const JS_BOOTSTRAP_PATH: &str = "crates/script-js/src/runtime/bindings.rs";
const RS2B0T_WITNESSES: &[(&str, &str)] =
    &[("CastNpc", "Game.castOnNpc"), ("Shop", "Shop.buyById")];
// Existing gaps only. A new row must have a complete script-surface pair.
const NO_LIVE_CELL_ROWS: &[&str] = &["CastNpc"];

#[derive(Clone, Copy)]
struct SeatWitness {
    row: &'static str,
    member: &'static str,
    line: usize,
    call: &'static str,
}

// Witness associations, not the row inventory. Each cited bootstrap line must
// retain its seat-start call. A new seat cannot borrow an existing association.
const JS_SEAT_WITNESSES: &[SeatWitness] = &[
    SeatWitness {
        row: "Bank",
        member: "NativeApi.bankNearestReachable",
        // crates/script-js/src/runtime/bindings.rs:1243
        line: 1243,
        call: "const out = await runMachine('bank_select', {",
    },
    SeatWitness {
        row: "Gather",
        member: "NativeApi.gather.run",
        // crates/script-js/src/runtime/bindings.rs:1185
        line: 1185,
        call: "return runMachine('gather', settings == null ? {} : settings, {});",
    },
    SeatWitness {
        row: "Combat",
        member: "NativeApi.combat.fight",
        // crates/script-js/src/runtime/bindings.rs:1197
        line: 1197,
        call: "return runMachine('combat', request, {});",
    },
    SeatWitness {
        row: "QuestProgress",
        member: "NativeApi.questProgress",
        // crates/script-js/src/runtime/bindings.rs:1213
        line: 1213,
        call: "return runMachine('quest-progress', { quest: input.quest }, {});",
    },
];

struct Sources {
    seats: String,
    types: String,
    frozen: String,
    live_map: String,
    bootstrap: String,
    luau_bindings: String,
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
            bootstrap: read(JS_BOOTSTRAP_PATH),
            luau_bindings: read("crates/script-luau/src/bindings/mod.rs"),
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

fn bootstrap_call_lines(source: &str) -> Result<BTreeSet<usize>, String> {
    let file = syn::parse_file(source).map_err(|error| format!("{JS_BOOTSTRAP_PATH}: {error}"))?;
    let mut constants = file.items.iter().filter_map(|item| match item {
        syn::Item::Const(item) if item.ident == "NATIVE_V2_MAIN" => Some(item),
        _ => None,
    });
    let constant = constants.next().ok_or("NATIVE_V2_MAIN is missing")?;
    if constants.next().is_some() {
        return Err("NATIVE_V2_MAIN has multiple declarations".into());
    }
    let syn::Expr::Lit(syn::ExprLit {
        lit: syn::Lit::Str(literal),
        ..
    }) = &*constant.expr
    else {
        return Err("NATIVE_V2_MAIN is not a string literal".into());
    };
    // Map the parsed literal back to its file lines without a second lexer.
    // A raw literal preserves the JavaScript line positions.
    let token = literal.token().to_string();
    let (opening, _) = token
        .split_once('"')
        .filter(|(opening, _)| {
            opening
                .strip_prefix('r')
                .is_some_and(|hashes| hashes.bytes().all(|byte| byte == b'#'))
        })
        .ok_or("NATIVE_V2_MAIN is not a raw string literal")?;
    let mut occurrences = source.match_indices(&token);
    let (offset, _) = occurrences
        .next()
        .ok_or("NATIVE_V2_MAIN literal has no source location")?;
    if occurrences.next().is_some() {
        return Err("NATIVE_V2_MAIN literal has multiple source locations".into());
    }
    let first_line = source[..offset + opening.len() + 1]
        .bytes()
        .filter(|byte| *byte == b'\n')
        .count()
        + 1;
    let specifier = deno_ast::ModuleSpecifier::parse("file:///native-v2-main.js")
        .map_err(|error| format!("bootstrap specifier: {error}"))?;
    let parsed = deno_ast::parse_module(deno_ast::ParseParams {
        specifier,
        text: literal.value().into(),
        media_type: deno_ast::MediaType::JavaScript,
        capture_tokens: false,
        scope_analysis: false,
        maybe_syntax: None,
    })
    .map_err(|error| format!("parse NATIVE_V2_MAIN: {error}"))?;
    let ProgramRef::Module(module) = parsed.program_ref() else {
        return Err("NATIVE_V2_MAIN did not parse as a module".into());
    };
    struct Calls<'a> {
        source: &'a deno_ast::SourceTextInfo,
        first_line: usize,
        lines: BTreeSet<usize>,
    }
    impl JsVisit for Calls<'_> {
        fn visit_call_expr(&mut self, call: &CallExpr) {
            self.lines
                .insert(self.first_line + self.source.line_index(call.start()));
            call.visit_children_with(self);
        }
    }
    let mut calls = Calls {
        source: parsed.text_info_lazy(),
        first_line,
        lines: BTreeSet::new(),
    };
    module.visit_with(&mut calls);
    Ok(calls.lines)
}

fn seat_witnesses(
    source: &str,
    seats: &BTreeSet<String>,
    witnesses: &[SeatWitness],
) -> Result<BTreeMap<&'static str, &'static str>, String> {
    let mut rows = BTreeMap::new();
    let mut members = BTreeSet::new();
    let mut citations = BTreeSet::new();
    for witness in witnesses {
        if !seats.contains(witness.row) {
            return Err(format!("unknown JavaScript seat witness: {}", witness.row));
        }
        if rows.insert(witness.row, witness.member).is_some() {
            return Err(format!(
                "duplicate JavaScript seat witness: {}",
                witness.row
            ));
        }
        if !members.insert(witness.member) {
            return Err(format!(
                "duplicate JavaScript seat witness member: {}",
                witness.member
            ));
        }
        if !citations.insert((JS_BOOTSTRAP_PATH, witness.line)) {
            return Err(format!(
                "duplicate JavaScript seat witness citation: {JS_BOOTSTRAP_PATH}:{}",
                witness.line
            ));
        }
    }
    let call_lines = bootstrap_call_lines(source)?;
    for witness in witnesses {
        if witness
            .line
            .checked_sub(1)
            .and_then(|index| source.lines().nth(index))
            .map(str::trim)
            != Some(witness.call)
        {
            return Err(format!(
                "stale JavaScript seat witness citation: {} {JS_BOOTSTRAP_PATH}:{}",
                witness.row, witness.line
            ));
        }
        if !call_lines.contains(&witness.line) {
            return Err(format!(
                "JavaScript seat witness citation has no call expression: {} {JS_BOOTSTRAP_PATH}:{}",
                witness.row, witness.line
            ));
        }
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
            } else {
                out.insert(path);
            }
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

fn bank_bindings(source: &str) -> Result<BTreeSet<String>, String> {
    fn field<'a>(binding: &'a syn::ExprStruct, name: &str) -> Result<&'a syn::Expr, String> {
        binding
            .fields
            .iter()
            .find_map(|field| match &field.member {
                syn::Member::Named(ident) if ident == name => Some(&field.expr),
                _ => None,
            })
            .ok_or_else(|| format!("Luau binding has no {name} field"))
    }

    struct BankCall(bool);
    impl<'ast> Visit<'ast> for BankCall {
        fn visit_expr_method_call(&mut self, call: &'ast syn::ExprMethodCall) {
            if call.method == "bank"
                && matches!(&*call.receiver, syn::Expr::Path(receiver) if receiver.path.is_ident("host"))
            {
                self.0 = true;
            }
            syn::visit::visit_expr_method_call(self, call);
        }
    }

    let file = syn::parse_file(source).map_err(|error| format!("Luau bindings: {error}"))?;
    let mut tables = file.items.iter().filter_map(|item| match item {
        syn::Item::Static(item) if item.ident == "BINDINGS" => Some(item),
        _ => None,
    });
    let table = tables.next().ok_or("Luau BINDINGS is missing")?;
    if tables.next().is_some() {
        return Err("Luau BINDINGS has multiple declarations".into());
    }
    let syn::Expr::Reference(reference) = &*table.expr else {
        return Err("Luau BINDINGS is not an array reference".into());
    };
    let syn::Expr::Array(array) = &*reference.expr else {
        return Err("Luau BINDINGS is not an array".into());
    };
    let mut result = BTreeSet::new();
    for entry in &array.elems {
        let syn::Expr::Struct(binding) = entry else {
            return Err("unsupported Luau binding registration".into());
        };
        let syn::Expr::Lit(path) = field(binding, "path")? else {
            return Err("Luau binding path is not a string".into());
        };
        let syn::Lit::Str(path) = &path.lit else {
            return Err("Luau binding path is not a string".into());
        };
        let syn::Expr::Call(call) = field(binding, "call")? else {
            return Err("unsupported Luau binding call".into());
        };
        let syn::Expr::Path(kind) = &*call.func else {
            return Err("unsupported Luau binding call kind".into());
        };
        if kind.path.segments.len() != 2 || kind.path.segments[0].ident != "Call" {
            return Err("unsupported Luau binding call kind".into());
        }
        if kind.path.segments[1].ident == "Frame" || kind.path.segments[1].ident == "Output" {
            continue;
        }
        if kind.path.segments[1].ident != "Host" {
            return Err("unsupported Luau host binding call".into());
        }
        let Some(syn::Expr::Path(function)) = call.args.first() else {
            return Err("Luau binding call has no function".into());
        };
        let name = function
            .path
            .get_ident()
            .ok_or("unsupported Luau binding function")?;
        let body = file
            .items
            .iter()
            .find_map(|item| match item {
                syn::Item::Fn(item) if item.sig.ident == *name => Some(&item.block),
                _ => None,
            })
            .ok_or_else(|| format!("Luau binding function is missing: {name}"))?;
        let mut calls = BankCall(false);
        calls.visit_block(body);
        if calls.0 {
            result.insert(path.value());
        }
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
        .filter(|(_, command)| command.split_whitespace().next() == Some("LIVE=1"))
        .map(|(id, _)| id.to_owned())
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
    check_with_seat_witnesses(sources, text, JS_SEAT_WITNESSES)
}

fn check_with_seat_witnesses(
    sources: &Sources,
    text: &str,
    witnesses: &[SeatWitness],
) -> Result<(usize, usize), String> {
    let tokens = ts_tokens(&sources.types)?;
    let ops = native_ops(&tokens)?;
    let mut expected = seat_rows(&sources.seats)?;
    let seat_witnesses = seat_witnesses(&sources.bootstrap, &expected, witnesses)?;
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
    let bank_paths = bank_bindings(&sources.luau_bindings)?;
    let bindings: BTreeMap<_, _> = script_luau::api_members()
        .filter(|member| matches!(member.kind, Kind::Request | Kind::Await))
        .map(|member| (member.path, member))
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
            } else if ops.contains(row) {
                return Err(format!("mismatched JavaScript op witness: {row} {js}"));
            } else if seat_witnesses.get(row).copied() != Some(js) {
                return Err(format!("mismatched JavaScript seat witness: {row} {js}"));
            }
        }
        if luau != "-" {
            let member = bindings
                .get(luau)
                .ok_or_else(|| format!("not a Request/Await Luau binding: {row} {luau}"))?;
            let same_row = if ops.contains(row) {
                member.js.and_then(|js| js.strip_prefix("NativeOp:")) == Some(row)
            } else if row == "Bank" {
                // Host::bank issues SeatStartArgs::Bank through await_bank
                // (bindings/host.rs:48-49 -> runtime.rs:1346-1358).
                bank_paths.contains(luau)
            } else {
                seat_witnesses
                    .get(row)
                    .is_some_and(|js| member.js == Some(*js))
            };
            if !same_row {
                return Err(format!("mismatched Luau row witness: {row} {luau}"));
            }
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
    assert_refused(check(sources, rows), expected);
}

fn assert_refused(result: Result<(usize, usize), String>, expected: &str) {
    let error = result.expect_err("negative fixture must be refused");
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
        "Gather NativeApi.gather.run host.script-api-gather-and-quest gather.run host.luau-gather-session",
        "Gather NativeApi.gather.run - gather.run -",
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

#[test]
fn witness_identity_b1_refuses_a_new_seat_with_another_seats_js_member() {
    let mut sources = Sources::read();
    sources.seats = replace_once(
        &sources.seats,
        "pub enum SeatStartArgs {",
        "pub enum SeatStartArgs { ProbeSeat(u8),",
    );
    let rows =
        format!("{ROWS}\nProbeSeat NativeApi.gather.run host.script-api-gather-and-quest - -\n");
    refused(
        &sources,
        &rows,
        "mismatched JavaScript seat witness: ProbeSeat",
    );
}

#[test]
fn witness_identity_b2_refuses_a_bank_binding_for_the_close_op() {
    let rows = replace_once(
        ROWS,
        "close NativeOp:close scenario.bone-burier-v2-live bank.closeNow host.luau-bank-ops",
        "close NativeOp:close scenario.bone-burier-v2-live bank.close host.luau-bank-interact",
    );
    refused(
        &Sources::read(),
        &rows,
        "mismatched Luau row witness: close bank.close",
    );
}

#[test]
fn witness_identity_b3_refuses_walk_to_for_walk_near() {
    let rows = replace_once(
        ROWS,
        "walk.near host.luau-walk-route",
        "walk.to host.luau-walk",
    );
    refused(
        &Sources::read(),
        &rows,
        "mismatched Luau row witness: walk-near walk.to",
    );
}

#[test]
fn witness_identity_b4_refuses_a_setting_read_for_combat() {
    let rows = replace_once(
        ROWS,
        "Combat NativeApi.combat.fight host.api-combat-melee combat.fight host.luau-combat-session",
        "Combat NativeApi.combat.fight host.api-combat-melee settings.str host.luau-walk",
    );
    refused(
        &Sources::read(),
        &rows,
        "mismatched Luau row witness: Combat settings.str",
    );
}

#[test]
fn witness_identity_b5_refuses_a_bank_binding_for_gather() {
    let rows = replace_once(
        ROWS,
        "Gather NativeApi.gather.run host.script-api-gather-and-quest gather.run host.luau-gather-session",
        "Gather NativeApi.gather.run host.script-api-gather-and-quest bank.openNearest host.luau-bank-interact",
    );
    refused(
        &Sources::read(),
        &rows,
        "mismatched Luau row witness: Gather bank.openNearest",
    );
}

#[test]
fn witness_identity_b6_refuses_another_seats_js_member() {
    let rows = replace_once(
        ROWS,
        "Combat NativeApi.combat.fight host.api-combat-melee",
        "Combat NativeApi.gather.run host.script-api-gather-and-quest",
    );
    refused(
        &Sources::read(),
        &rows,
        "mismatched JavaScript seat witness: Combat",
    );
}

#[test]
fn witness_identity_b7_refuses_a_native_api_member_for_an_op() {
    let rows = replace_once(
        ROWS,
        "held NativeOp:held scenario.bone-burier-v2-live",
        "held NativeApi.request scenario.bone-burier-v2-live",
    );
    refused(
        &Sources::read(),
        &rows,
        "mismatched JavaScript op witness: held",
    );
}

#[test]
fn witness_identity_refuses_a_namespace_as_a_js_witness() {
    let rows = replace_once(ROWS, "Gather NativeApi.gather.run", "Gather NativeApi.clue");
    refused(
        &Sources::read(),
        &rows,
        "undeclared NativeApi witness: Gather NativeApi.clue",
    );
}

#[test]
fn witness_identity_refuses_a_changed_bootstrap_call_at_its_citation() {
    let mut sources = Sources::read();
    sources.bootstrap = replace_once(
        &sources.bootstrap,
        "return runMachine('gather', settings == null ? {} : settings, {});",
        "return runMachine('combat', settings == null ? {} : settings, {});",
    );
    refused(
        &sources,
        ROWS,
        "stale JavaScript seat witness citation: Gather",
    );
}

#[test]
fn witness_identity_refuses_a_bank_binding_without_the_bank_host_call() {
    let mut sources = Sources::read();
    sources.luau_bindings = replace_once(
        &sources.luau_bindings,
        "host.bank(convert::bank_request(\n        BankSeatOperation::OpenNearestWorld",
        "host.interact(convert::bank_request(\n        BankSeatOperation::OpenNearestWorld",
    );
    sources
        .luau_bindings
        .push_str("\n// host.bank(convert::bank_request(BankSeatOperation::OpenNearestWorld))\n");
    refused(
        &sources,
        ROWS,
        "mismatched Luau row witness: Bank bank.openNearest",
    );
}

#[test]
fn witness_identity_refuses_a_bank_registration_with_another_function() {
    let mut sources = Sources::read();
    sources.luau_bindings = replace_once(
        &sources.luau_bindings,
        "call: Call::Host(bank_open_nearest)",
        "call: Call::Host(settings_str)",
    );
    refused(
        &sources,
        ROWS,
        "mismatched Luau row witness: Bank bank.openNearest",
    );
}

#[test]
fn seat_citation_f4_refuses_a_new_seat_borrowing_gathers_member_and_call() {
    let mut sources = Sources::read();
    sources.seats = replace_once(
        &sources.seats,
        "pub enum SeatStartArgs {",
        "pub enum SeatStartArgs { ProbeSeat(u8),",
    );
    let rows =
        format!("{ROWS}\nProbeSeat NativeApi.gather.run host.script-api-gather-and-quest - -\n");
    let mut witnesses = JS_SEAT_WITNESSES.to_vec();
    witnesses.push(SeatWitness {
        row: "ProbeSeat",
        ..JS_SEAT_WITNESSES[1]
    });
    assert_refused(
        check_with_seat_witnesses(&sources, &rows, &witnesses),
        "duplicate JavaScript seat witness member: NativeApi.gather.run",
    );
}

#[test]
fn seat_citation_refuses_an_unknown_seat() {
    let mut witnesses = JS_SEAT_WITNESSES.to_vec();
    witnesses.push(SeatWitness {
        row: "UnknownSeat",
        ..JS_SEAT_WITNESSES[1]
    });
    assert_refused(
        check_with_seat_witnesses(&Sources::read(), ROWS, &witnesses),
        "unknown JavaScript seat witness: UnknownSeat",
    );
}

#[test]
fn seat_citation_refuses_a_duplicate_seat() {
    let mut witnesses = JS_SEAT_WITNESSES.to_vec();
    witnesses.push(JS_SEAT_WITNESSES[1]);
    assert_refused(
        check_with_seat_witnesses(&Sources::read(), ROWS, &witnesses),
        "duplicate JavaScript seat witness: Gather",
    );
}

#[test]
fn seat_citation_refuses_a_duplicate_member_with_a_distinct_call() {
    let mut witnesses = JS_SEAT_WITNESSES.to_vec();
    witnesses[2].member = witnesses[1].member;
    assert_refused(
        check_with_seat_witnesses(&Sources::read(), ROWS, &witnesses),
        "duplicate JavaScript seat witness member: NativeApi.gather.run",
    );
}

#[test]
fn seat_citation_refuses_a_duplicate_citation_with_a_distinct_member() {
    let mut witnesses = JS_SEAT_WITNESSES.to_vec();
    witnesses[2].line = witnesses[1].line;
    witnesses[2].call = witnesses[1].call;
    assert_refused(
        check_with_seat_witnesses(&Sources::read(), ROWS, &witnesses),
        "duplicate JavaScript seat witness citation: crates/script-js/src/runtime/bindings.rs:1185",
    );
}

#[test]
fn seat_citation_f3_refuses_a_call_inside_a_block_comment() {
    let mut sources = Sources::read();
    sources.bootstrap = replace_once(
        &sources.bootstrap,
        "      }\n      return runMachine('gather', settings == null ? {} : settings, {});\n    },",
        "      } /*\n      return runMachine('gather', settings == null ? {} : settings, {});\n    */ },",
    );
    refused(
        &sources,
        ROWS,
        "JavaScript seat witness citation has no call expression: Gather",
    );
}

#[test]
fn seat_citation_f3_refuses_a_comment_matching_the_updated_constant() {
    let mut sources = Sources::read();
    let comment = "// return runMachine('gather', settings == null ? {} : settings, {});";
    sources.bootstrap = replace_once(&sources.bootstrap, JS_SEAT_WITNESSES[1].call, comment);
    let mut witnesses = JS_SEAT_WITNESSES.to_vec();
    witnesses[1].call = comment;
    assert_refused(
        check_with_seat_witnesses(&sources, ROWS, &witnesses),
        "JavaScript seat witness citation has no call expression: Gather",
    );
}

#[test]
fn seat_citation_n4b_refuses_a_commented_call_after_another_seat_call() {
    let mut sources = Sources::read();
    sources.bootstrap = replace_once(
        &sources.bootstrap,
        "      }\n      return runMachine('gather', settings == null ? {} : settings, {});\n    },",
        "      } return runMachine('combat', settings == null ? {} : settings, {}); /*\n      return runMachine('gather', settings == null ? {} : settings, {});\n    */ },",
    );
    refused(
        &sources,
        ROWS,
        "JavaScript seat witness citation has no call expression: Gather",
    );
}
