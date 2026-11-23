//! Freeze the JavaScript v2 surface that still enters the compatibility layer.
//! The checked-in list may only shrink from `FROZEN_MEMBERS`; growth requires
//! an explicit reviewed change to this test. The source rule checks the literal
//! `crate::compat::` text, so an import alias hides a compat registration. A
//! listed member then disappears from the derived set and this test fails loudly.

use deno_ast::swc::ast::{
    AssignExpr, AssignTarget, BinaryOp, BlockStmtOrExpr, Callee, Decl, Expr, FnDecl, ForHead,
    ForOfStmt, Function, Lit, MemberExpr, MemberProp, Module, ModuleItem, ObjectLit, Pat, Prop,
    PropName, PropOrSpread, SimpleAssignTarget, Stmt, VarDeclarator,
};
use deno_ast::swc::ecma_visit::{Visit, VisitWith};
use deno_ast::ProgramRef;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};

const BINDINGS_SOURCE: &str = include_str!("../src/runtime/bindings.rs");
const MACHINE_SOURCE: &str = include_str!("../src/machine/mod.rs");
const CLOSED_MEMBERS: &str = include_str!("compat_v2_freeze.members");
const FROZEN_MEMBERS: &[&str] = &[
    "NativeApi.bankRun",
    "NativeApi.cellRun",
    "NativeApi.clue.begin",
    "NativeApi.clue.retry",
    "NativeApi.clue.run",
    "NativeApi.enterBegin",
    "NativeApi.enterRun",
    "NativeApi.enterValidate",
    "NativeApi.fightBegin",
    "NativeApi.fightBlocksLoot",
    "NativeApi.fightInterruptWatch",
    "NativeApi.fightReset",
    "NativeApi.fightRun",
    "NativeApi.fightValidate",
    "NativeApi.holdBegin",
    "NativeApi.holdRun",
    "NativeApi.holdValidate",
    "NativeApi.keyRun",
    "NativeApi.leaveRun",
    "NativeApi.prayerActive",
    "NativeApi.prayerAvailable",
    "NativeApi.prayerClear",
    "NativeApi.prayerFull",
    "NativeApi.prayerKnown",
    "NativeApi.prayerMax",
    "NativeApi.prayerPoints",
    "NativeApi.prayerSet",
    "NativeApi.questJournalBegin",
    "NativeApi.questJournalRun",
    "NativeApi.retreatBegin",
    "NativeApi.retreatRun",
    "NativeApi.retreatValidate",
    "NativeApi.walkspotBegin",
    "NativeApi.walkspotRun",
    "NativeApi.walkspotValidate",
];
const FREEZE_RULE: &str = "JavaScript v2 gets no new compat-backed member, and a new capability is a native row (ADR 0023 item 31).";

#[test]
fn compat_backed_native_v2_members_match_the_closed_list() {
    let sources = rust_sources(&Path::new(env!("CARGO_MANIFEST_DIR")).join("src"))
        .expect("read script-js Rust sources");
    let actual = compat_members_from_source(BINDINGS_SOURCE, &sources, MACHINE_SOURCE)
        .expect("derive compat-backed members from source");
    let closed = parse_closed_members(CLOSED_MEMBERS).expect("parse checked-in closed list");
    let frozen = FROZEN_MEMBERS
        .iter()
        .map(|member| (*member).to_string())
        .collect::<BTreeSet<_>>();
    assert_eq!(frozen.len(), FROZEN_MEMBERS.len(), "frozen member names are unique");
    assert!(
        closed.is_subset(&frozen),
        "the checked-in list may only shrink from FROZEN_MEMBERS; {FREEZE_RULE}"
    );

    assert_eq!(
        compare_closed_members(&closed, &actual),
        Ok(()),
        "{FREEZE_RULE} Source: crates/script-js/src/runtime/bindings.rs, NATIVE_V2_MAIN."
    );
}

#[test]
fn closed_list_rejects_a_new_member_that_calls_a_compat_registered_host() {
    let bootstrap = r#"
function prayerCall(payload) {
  return globalThis.rustyscript.functions.__rs2b0t_prayer(payload);
}
const api = {
  addedPrayer() { return prayerCall({ op: 'points' }); },
};
"#;
    let registrations = vec![(
        PathBuf::from("runtime/bindings.rs"),
        r#"runtime.register_function("__rs2b0t_prayer", |args| {
            crate::compat::prayer::dispatch(args)
        });"#
            .to_string(),
    )];
    let actual = compat_members(bootstrap, &registrations, "", &[]).expect("derive fixture member");
    let closed = BTreeSet::new();

    assert!(actual.contains("NativeApi.addedPrayer"));
    assert!(compare_closed_members(&closed, &actual).is_err());
}

#[test]
fn closed_list_rejects_a_new_member_that_starts_a_compat_machine_family() {
    let bootstrap = r#"
const api = {
  addedMachine(input) { return runMachine('prayer', input); },
};
"#;
    let family_table = r#"
const FAMILIES: &[Entry] = &[
    entry::<crate::compat::prayer::Prayer>(),
];
"#;
    let compat_source = r#"
impl Family for Prayer {
    const NAME: &'static str = "prayer";
}
"#;
    let actual = compat_members(bootstrap, &[], family_table, &[compat_source])
        .expect("derive fixture member");
    let closed = BTreeSet::new();

    assert!(actual.contains("NativeApi.addedMachine"));
    assert!(compare_closed_members(&closed, &actual).is_err());
}
#[test]
fn real_sources_reject_all_reviewed_compat_member_forms() {
    let mut bootstrap = native_v2_main(BINDINGS_SOURCE)
        .expect("NATIVE_V2_MAIN is present")
        .to_string();
    bootstrap.push_str(
        r#"
api.addedNow = () => machineNow('prayer', {});
api.addedMachineStart = () => globalThis.__rs2b0t_machine_start('prayer', {}, {});
api.addedAlias = (payload) => {
  const prayer = globalThis.rustyscript.functions.__rs2b0t_prayer;
  return prayer(payload);
};
function addedPrayerReference(payload) {
  return globalThis.rustyscript.functions.__rs2b0t_prayer(payload);
}
api.addedReference = addedPrayerReference;
api.clue.addedNested = (payload) => globalThis.rustyscript.functions.__rs2b0t_clue(payload);
"#,
    );
    let sources = rust_sources(&Path::new(env!("CARGO_MANIFEST_DIR")).join("src"))
        .expect("read script-js Rust sources");
    let family_sources = sources
        .iter()
        .map(|(_, source)| source.as_str())
        .collect::<Vec<_>>();
    let actual = compat_members(&bootstrap, &sources, MACHINE_SOURCE, &family_sources)
        .expect("derive members with real registrations and machine table");
    let closed = parse_closed_members(CLOSED_MEMBERS).expect("parse checked-in closed list");
    for member in [
        "NativeApi.addedNow",
        "NativeApi.addedMachineStart",
        "NativeApi.addedAlias",
        "NativeApi.addedReference",
        "NativeApi.clue.addedNested",
    ] {
        assert!(actual.contains(member), "missed {member}");
    }
    assert!(compare_closed_members(&closed, &actual).is_err());
}


#[test]
fn closed_list_rejects_a_member_that_is_no_longer_compat_backed() {
    let bootstrap = r#"
const api = {
  prayerPoints() { return 1; },
};
"#;
    let actual = compat_members(bootstrap, &[], "", &[]).expect("derive fixture members");
    let closed = BTreeSet::from(["NativeApi.prayerPoints".to_string()]);

    assert!(compare_closed_members(&closed, &actual).is_err());
}

fn native_v2_main(source: &str) -> Option<&str> {
    let marker = "const NATIVE_V2_MAIN: &str = r#\"";
    let body = source.split_once(marker)?.1;
    body.split_once("\"#;").map(|(bootstrap, _)| bootstrap)
}

fn parse_closed_members(source: &str) -> Result<BTreeSet<String>, String> {
    let mut members = BTreeSet::new();
    for (line_number, line) in source.lines().enumerate() {
        let member = line.trim();
        if member.is_empty() || member.starts_with('#') {
            continue;
        }
        if !member.starts_with("NativeApi.") {
            return Err(format!(
                "line {} is not a NativeApi member: {member}",
                line_number + 1
            ));
        }
        if !members.insert(member.to_string()) {
            return Err(format!("line {} repeats {member}", line_number + 1));
        }
    }
    Ok(members)
}

fn compare_closed_members(
    closed: &BTreeSet<String>,
    actual: &BTreeSet<String>,
) -> Result<(), String> {
    let additions = actual.difference(closed).cloned().collect::<Vec<_>>();
    let stale = closed.difference(actual).cloned().collect::<Vec<_>>();
    if additions.is_empty() && stale.is_empty() {
        return Ok(());
    }
    Err(format!(
        "{FREEZE_RULE}\nNot on the closed list: {additions:?}\nNo longer compat-backed: {stale:?}"
    ))
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum ArgValue {
    String(String),
    Ident(String),
    Other,
}

#[derive(Clone, Debug)]
struct CallFact {
    target: String,
    args: Vec<ArgValue>,
}

#[derive(Clone, Debug, Default)]
struct FunctionFacts {
    params: Vec<Option<String>>,
    calls: Vec<CallFact>,
    global_references: BTreeSet<String>,
    global_calls: BTreeSet<String>,
}

#[derive(Clone, Debug, Default)]
struct MemberFunction {
    facts: FunctionFacts,
    captured: BTreeMap<String, String>,
}

#[derive(Default)]
struct CallCollector {
    facts: FunctionFacts,
    aliases: BTreeMap<String, String>,
}

impl Visit for CallCollector {
    fn visit_call_expr(&mut self, call: &deno_ast::swc::ast::CallExpr) {
        if let Callee::Expr(callee) = &call.callee {
            let args = call
                .args
                .iter()
                .map(|arg| simple_arg(&arg.expr))
                .collect::<Vec<_>>();
            match &**callee {
                Expr::Ident(ident) => {
                    let target = ident.sym.to_string();
                    if let Some(global) = self.aliases.get(&target).cloned() {
                        self.facts.global_calls.insert(global.clone());
                        if global == "__rs2b0t_machine_start" {
                            self.facts.calls.push(CallFact { target: global, args });
                        }
                    } else {
                        self.facts.calls.push(CallFact { target, args });
                    }
                }
                Expr::Member(_) => {
                    if let Some(path) = api_call_path(callee) {
                        self.facts.calls.push(CallFact { target: path, args });
                    } else if let Some(global) = global_member_name(callee) {
                        self.facts.global_calls.insert(global.clone());
                        if global == "__rs2b0t_machine_start" {
                            self.facts.calls.push(CallFact {
                                target: global,
                                args,
                            });
                        }
                    }
                }
                _ => {}
            }
        }
        call.visit_children_with(self);
    }

    fn visit_var_declarator(&mut self, declarator: &VarDeclarator) {
        if let (Pat::Ident(ident), Some(value)) =
            (&declarator.name, declarator.init.as_deref())
        {
            if let Some(global) = global_member_name(value) {
                self.aliases.insert(ident.id.sym.to_string(), global);
            }
        }
        declarator.visit_children_with(self);
    }

    fn visit_member_expr(&mut self, member: &MemberExpr) {
        if let Some(name) = member_prop_name(&member.prop) {
            if name.starts_with("__rs2b0t_") {
                self.facts.global_references.insert(name);
            }
        }
        member.visit_children_with(self);
    }
}

fn simple_arg(expr: &Expr) -> ArgValue {
    match expr {
        Expr::Ident(ident) => ArgValue::Ident(ident.sym.to_string()),
        Expr::Lit(Lit::Str(value)) => ArgValue::String(value.value.to_string()),
        _ => ArgValue::Other,
    }
}

fn member_prop_name(prop: &MemberProp) -> Option<String> {
    match prop {
        MemberProp::Ident(ident) => Some(ident.sym.to_string()),
        MemberProp::Computed(computed) => match computed.expr.as_ref() {
            Expr::Lit(Lit::Str(value)) => Some(value.value.to_string()),
            _ => None,
        },
        MemberProp::PrivateName(_) => None,
    }
}

fn global_member_name(expr: &Expr) -> Option<String> {
    member_parts(expr)?
        .last()
        .filter(|name| name.starts_with("__rs2b0t_"))
        .cloned()
}

fn api_call_path(expr: &Expr) -> Option<String> {
    let mut parts = member_parts(expr)?;
    if parts.first().map(String::as_str) != Some("api") || parts.len() < 2 {
        return None;
    }
    parts.remove(0);
    Some(format!("NativeApi.{}", parts.join(".")))
}

fn member_parts(expr: &Expr) -> Option<Vec<String>> {
    match expr {
        Expr::Ident(ident) => Some(vec![ident.sym.to_string()]),
        Expr::Member(member) => {
            let mut parts = member_parts(&member.obj)?;
            parts.push(member_prop_name(&member.prop)?);
            Some(parts)
        }
        Expr::Paren(paren) => member_parts(&paren.expr),
        _ => None,
    }
}

fn function_facts(function: &Function) -> FunctionFacts {
    let mut collector = CallCollector {
        facts: FunctionFacts {
            params: function
                .params
                .iter()
                .map(|param| pat_name(&param.pat))
                .collect(),
            ..FunctionFacts::default()
        },
        ..CallCollector::default()
    };
    if let Some(body) = &function.body {
        body.visit_with(&mut collector);
    }
    collector.facts
}

fn function_expr_facts(expr: &Expr) -> Option<FunctionFacts> {
    match expr {
        Expr::Fn(function) => Some(function_facts(&function.function)),
        Expr::Arrow(arrow) => {
            let mut collector = CallCollector {
                facts: FunctionFacts {
                    params: arrow.params.iter().map(pat_name).collect(),
                    ..FunctionFacts::default()
                },
                ..CallCollector::default()
            };
            match arrow.body.as_ref() {
                BlockStmtOrExpr::BlockStmt(body) => body.visit_with(&mut collector),
                BlockStmtOrExpr::Expr(body) => body.visit_with(&mut collector),
            }
            Some(collector.facts)
        }
        Expr::Paren(paren) => function_expr_facts(&paren.expr),
        _ => None,
    }
}

fn pat_name(pat: &Pat) -> Option<String> {
    match pat {
        Pat::Ident(ident) => Some(ident.id.sym.to_string()),
        _ => None,
    }
}

#[derive(Default)]
struct HelperCollector {
    helpers: BTreeMap<String, Vec<FunctionFacts>>,
}

impl Visit for HelperCollector {
    fn visit_fn_decl(&mut self, function: &FnDecl) {
        self.helpers
            .entry(function.ident.sym.to_string())
            .or_default()
            .push(function_facts(&function.function));
        function.visit_children_with(self);
    }

    fn visit_var_declarator(&mut self, declarator: &VarDeclarator) {
        if let (Pat::Ident(ident), Some(value)) = (&declarator.name, declarator.init.as_deref()) {
            if let Some(facts) = function_expr_facts(value) {
                self.helpers
                    .entry(ident.id.sym.to_string())
                    .or_default()
                    .push(facts);
            }
        }
        declarator.visit_children_with(self);
    }
}

fn collect_api_members(
    module: &Module,
    helpers: &BTreeMap<String, Vec<FunctionFacts>>,
) -> Result<BTreeMap<String, Vec<MemberFunction>>, String> {
    let mut collector = ApiMemberCollector {
        helpers,
        members: BTreeMap::new(),
        error: None,
    };
    module.visit_with(&mut collector);
    match collector.error {
        Some(error) => Err(error),
        None => Ok(collector.members),
    }
}

struct ApiMemberCollector<'a> {
    helpers: &'a BTreeMap<String, Vec<FunctionFacts>>,
    members: BTreeMap<String, Vec<MemberFunction>>,
    error: Option<String>,
}

impl Visit for ApiMemberCollector<'_> {
    fn visit_var_declarator(&mut self, declarator: &VarDeclarator) {
        if matches!(&declarator.name, Pat::Ident(ident) if ident.id.sym.as_ref() == "api") {
            if let Some(Expr::Object(object)) = declarator.init.as_deref() {
                collect_object_members(
                    "NativeApi",
                    object,
                    self.helpers,
                    &mut self.members,
                );
            }
        }
        declarator.visit_children_with(self);
    }

    fn visit_assign_expr(&mut self, assignment: &AssignExpr) {
        if let Some(path) = api_assignment_path(&assignment.left) {
            collect_member_value(
                &path,
                &assignment.right,
                self.helpers,
                &mut self.members,
            );
        }
        assignment.visit_children_with(self);
    }

    fn visit_for_of_stmt(&mut self, loop_statement: &ForOfStmt) {
        if let Err(error) = collect_generated_members(loop_statement, &mut self.members) {
            self.error = Some(error);
        }
        loop_statement.visit_children_with(self);
    }
}

fn collect_member_value(
    path: &str,
    value: &Expr,
    helpers: &BTreeMap<String, Vec<FunctionFacts>>,
    members: &mut BTreeMap<String, Vec<MemberFunction>>,
) {
    if let Some(facts) = function_expr_facts(value) {
        members
            .entry(path.to_string())
            .or_default()
            .push(MemberFunction {
                facts,
                captured: BTreeMap::new(),
            });
    } else if let Expr::Ident(ident) = value {
        if let Some(functions) = helpers.get(ident.sym.as_ref()) {
            for facts in functions {
                members
                    .entry(path.to_string())
                    .or_default()
                    .push(MemberFunction {
                        facts: facts.clone(),
                        captured: BTreeMap::new(),
                    });
            }
        }
    } else if let Expr::Object(object) = value {
        collect_object_members(path, object, helpers, members);
    }
}

fn collect_object_members(
    parent: &str,
    object: &ObjectLit,
    helpers: &BTreeMap<String, Vec<FunctionFacts>>,
    members: &mut BTreeMap<String, Vec<MemberFunction>>,
) {
    for property in &object.props {
        let PropOrSpread::Prop(property) = property else {
            continue;
        };
        match property.as_ref() {
            Prop::Method(method) => {
                if let Some(name) = prop_name(&method.key) {
                    members
                        .entry(format!("{parent}.{name}"))
                        .or_default()
                        .push(MemberFunction {
                            facts: function_facts(&method.function),
                            captured: BTreeMap::new(),
                        });
                }
            }
            Prop::KeyValue(value) => {
                if let Some(name) = prop_name(&value.key) {
                    collect_member_value(
                        &format!("{parent}.{name}"),
                        &value.value,
                        helpers,
                        members,
                    );
                }
            }
            _ => {}
        }
    }
}

fn prop_name(name: &PropName) -> Option<String> {
    match name {
        PropName::Ident(ident) => Some(ident.sym.to_string()),
        PropName::Str(value) => Some(value.value.to_string()),
        PropName::Computed(computed) => match computed.expr.as_ref() {
            Expr::Lit(Lit::Str(value)) => Some(value.value.to_string()),
            _ => None,
        },
        PropName::Num(value) => Some(value.value.to_string()),
        PropName::BigInt(value) => Some(value.value.to_string()),
    }
}

fn api_assignment_path(target: &AssignTarget) -> Option<String> {
    let AssignTarget::Simple(SimpleAssignTarget::Member(member)) = target else {
        return None;
    };
    let mut parts = member_parts(&member.obj)?;
    if parts.first().map(String::as_str) != Some("api") {
        return None;
    }
    parts.push(member_prop_name(&member.prop)?);
    parts.remove(0);
    Some(format!("NativeApi.{}", parts.join(".")))
}

#[derive(Default)]
struct DynamicApiCollector {
    name_variable: String,
    assignments: Vec<(String, FunctionFacts)>,
}

impl Visit for DynamicApiCollector {
    fn visit_assign_expr(&mut self, assignment: &AssignExpr) {
        if let Some(suffix) = dynamic_api_suffix(&assignment.left, &self.name_variable) {
            if let Some(facts) = function_expr_facts(&assignment.right) {
                self.assignments.push((suffix, facts));
            }
        }
        assignment.visit_children_with(self);
    }
}

fn dynamic_api_suffix(target: &AssignTarget, name_variable: &str) -> Option<String> {
    let AssignTarget::Simple(SimpleAssignTarget::Member(member)) = target else {
        return None;
    };
    if !matches!(member.obj.as_ref(), Expr::Ident(ident) if ident.sym.as_ref() == "api") {
        return None;
    }
    let MemberProp::Computed(computed) = &member.prop else {
        return None;
    };
    let Expr::Bin(binary) = computed.expr.as_ref() else {
        return None;
    };
    if binary.op != BinaryOp::Add
        || !matches!(binary.left.as_ref(), Expr::Ident(ident) if ident.sym.as_ref() == name_variable)
    {
        return None;
    }
    match binary.right.as_ref() {
        Expr::Lit(Lit::Str(value)) => Some(value.value.to_string()),
        _ => None,
    }
}

fn collect_generated_members(
    loop_statement: &ForOfStmt,
    members: &mut BTreeMap<String, Vec<MemberFunction>>,
) -> Result<(), String> {
    let ForHead::VarDecl(declaration) = &loop_statement.left else {
        return Ok(());
    };
    let Some(first) = declaration.decls.first() else {
        return Ok(());
    };
    let Pat::Array(binding) = &first.name else {
        return Ok(());
    };
    let Some(Some(name_variable)) = binding
        .elems
        .first()
        .map(|pat| pat.as_ref().and_then(pat_name))
    else {
        return Ok(());
    };
    let Some(Some(family_variable)) = binding
        .elems
        .get(1)
        .map(|pat| pat.as_ref().and_then(pat_name))
    else {
        return Ok(());
    };
    let Expr::Array(rows) = loop_statement.right.as_ref() else {
        return Ok(());
    };
    let mut collector = DynamicApiCollector {
        name_variable,
        ..DynamicApiCollector::default()
    };
    loop_statement.body.visit_with(&mut collector);

    for row in &rows.elems {
        let Some(row) = row else { continue };
        let Expr::Array(pair) = row.expr.as_ref() else {
            continue;
        };
        let Some(Some(name)) = pair
            .elems
            .first()
            .map(|item| item.as_ref().and_then(|e| string_literal(&e.expr)))
        else {
            continue;
        };
        let Some(Some(family)) = pair
            .elems
            .get(1)
            .map(|item| item.as_ref().and_then(|e| string_literal(&e.expr)))
        else {
            continue;
        };
        for (suffix, facts) in &collector.assignments {
            members
                .entry(format!("NativeApi.{name}{suffix}"))
                .or_default()
                .push(MemberFunction {
                    facts: facts.clone(),
                    captured: BTreeMap::from([(family_variable.clone(), family.clone())]),
                });
        }
    }
    Ok(())
}

fn string_literal(expr: &Expr) -> Option<String> {
    match expr {
        Expr::Lit(Lit::Str(value)) => Some(value.value.to_string()),
        _ => None,
    }
}

fn compat_members(
    bootstrap: &str,
    rust_sources: &[(PathBuf, String)],
    machine_source: &str,
    compat_sources: &[&str],
) -> Result<BTreeSet<String>, String> {
    let specifier = deno_ast::ModuleSpecifier::parse("file:///native-v2-main.js")
        .map_err(|error| format!("bootstrap specifier: {error}"))?;
    let parsed = deno_ast::parse_module(deno_ast::ParseParams {
        specifier,
        text: bootstrap.to_string().into(),
        media_type: deno_ast::MediaType::JavaScript,
        capture_tokens: false,
        scope_analysis: false,
        maybe_syntax: None,
    })
    .map_err(|error| format!("parse NATIVE_V2_MAIN: {error}"))?;
    let ProgramRef::Module(module) = parsed.program_ref() else {
        return Err("NATIVE_V2_MAIN did not parse as a module".to_string());
    };

    let mut helper_collector = HelperCollector::default();
    module.visit_with(&mut helper_collector);
    let members = collect_api_members(module, &helper_collector.helpers)?;
    let compat_globals = compat_registered_globals(rust_sources);
    let families = compat_family_names(machine_source, compat_sources)?;
    let mut found = BTreeSet::new();
    for (path, functions) in &members {
        for function in functions {
            if reaches_compat(
                &function.facts,
                &function.captured,
                &helper_collector.helpers,
                &members,
                &compat_globals,
                &families,
            )? {
                found.insert(path.clone());
                break;
            }
        }
    }
    Ok(found)
}

fn reaches_compat(
    start: &FunctionFacts,
    captured: &BTreeMap<String, String>,
    helpers: &BTreeMap<String, Vec<FunctionFacts>>,
    members: &BTreeMap<String, Vec<MemberFunction>>,
    compat_globals: &BTreeSet<String>,
    families: &FamilyNames,
) -> Result<bool, String> {
    let mut pending = VecDeque::from([(start.clone(), captured.clone())]);
    let mut visited = BTreeSet::new();
    while let Some((facts, environment)) = pending.pop_front() {
        if facts
            .global_references
            .iter()
            .any(|global| compat_globals.contains(global))
        {
            return Ok(true);
        }
        for call in &facts.calls {
            if matches!(
                call.target.as_str(),
                "runMachine" | "machineNow" | "__rs2b0t_machine_start"
            ) {
                let family = call
                    .args
                    .first()
                    .and_then(|arg| resolve_arg(arg, &environment))
                    .ok_or_else(|| {
                        format!(
                            "unresolved family argument to {}",
                            call.target
                        )
                    })?;
                if !families.all.contains(&family) {
                    return Err(format!(
                        "unknown machine family {family:?} passed to {}",
                        call.target
                    ));
                }
                if families.compat.contains(&family) {
                    return Ok(true);
                }
                continue;
            }
            if let Some(callees) = helpers.get(&call.target) {
                for callee in callees {
                    let environment = bind_arguments(callee, &call.args, &environment);
                    let key = format!("helper:{}:{environment:?}", call.target);
                    if visited.insert(key) {
                        pending.push_back((callee.clone(), environment));
                    }
                }
            }
            if let Some(callees) = members.get(&call.target) {
                for callee in callees {
                    let mut environment = callee.captured.clone();
                    environment.extend(bind_arguments(&callee.facts, &call.args, &environment));
                    let key = format!("member:{}:{environment:?}", call.target);
                    if visited.insert(key) {
                        pending.push_back((callee.facts.clone(), environment));
                    }
                }
            }
        }
    }
    Ok(false)
}

fn bind_arguments(
    function: &FunctionFacts,
    args: &[ArgValue],
    inherited: &BTreeMap<String, String>,
) -> BTreeMap<String, String> {
    let mut environment = inherited.clone();
    for (parameter, argument) in function.params.iter().zip(args) {
        if let (Some(parameter), Some(value)) = (parameter, resolve_arg(argument, inherited)) {
            environment.insert(parameter.clone(), value);
        }
    }
    environment
}

fn resolve_arg(arg: &ArgValue, environment: &BTreeMap<String, String>) -> Option<String> {
    match arg {
        ArgValue::String(value) => Some(value.clone()),
        ArgValue::Ident(name) => environment.get(name).cloned(),
        ArgValue::Other => None,
    }
}

#[derive(Default)]
struct FamilyNames {
    all: BTreeSet<String>,
    compat: BTreeSet<String>,
}

fn compat_family_names(
    machine_source: &str,
    family_sources: &[&str],
) -> Result<FamilyNames, String> {
    let Some((_, rest)) = machine_source.split_once("const FAMILIES:") else {
        return Ok(FamilyNames::default());
    };
    let (table, _) = rest
        .split_once("];")
        .ok_or_else(|| "machine FAMILIES table is unterminated".to_string())?;

    let mut families = FamilyNames::default();
    for (line_number, line) in table.lines().enumerate() {
        let Some((_, rest)) = line.split_once("entry::<") else {
            continue;
        };
        let type_path = rest.split_once(">()").map(|(path, _)| path).ok_or_else(|| {
            format!(
                "machine FAMILIES line {} has an unrecognized entry",
                line_number + 1
            )
        })?;
        let (trait_name, type_name, is_compat) = family_impl_identity(type_path)?;
        let marker = format!("impl {trait_name} for {type_name}");
        let name = literal_name_for_impl(family_sources, &marker)?;
        if !families.all.insert(name.clone()) {
            return Err(format!(
                "machine FAMILIES contains duplicate family name {name:?}"
            ));
        }
        if is_compat {
            families.compat.insert(name);
        }
    }
    Ok(families)
}

fn family_impl_identity(type_path: &str) -> Result<(&'static str, String, bool), String> {
    let type_path = type_path.trim();
    let is_compat = type_path.contains("crate::compat::");
    if let Some((family_type, arguments)) = type_path.split_once('<') {
        let family_type = family_type.rsplit("::").next().unwrap_or(family_type);
        if family_type != "Hunt" {
            return Err(format!(
                "unrecognized generic machine family type {type_path:?}"
            ));
        }
        let arguments = arguments
            .strip_suffix('>')
            .ok_or_else(|| format!("unterminated machine family type {type_path:?}"))?;
        let kind = arguments
            .rsplit("::")
            .next()
            .ok_or_else(|| format!("missing HuntKind in machine family type {type_path:?}"))?
            .trim_end_matches('>');
        if kind.is_empty() {
            return Err(format!("missing HuntKind in machine family type {type_path:?}"));
        }
        return Ok(("HuntKind", kind.to_string(), is_compat));
    }
    let type_name = type_path
        .rsplit("::")
        .next()
        .filter(|name| !name.is_empty())
        .ok_or_else(|| format!("missing type in machine family entry {type_path:?}"))?;
    Ok(("Family", type_name.to_string(), is_compat))
}

fn literal_name_for_impl(sources: &[&str], marker: &str) -> Result<String, String> {
    let mut names = BTreeSet::new();
    for source in sources {
        let Some(rest) = source.split_once(marker).map(|(_, rest)| rest) else {
            continue;
        };
        let end = rest
            .split_inclusive('\n')
            .scan(0usize, |offset, line| {
                let current = *offset;
                *offset += line.len();
                Some((current, line))
            })
            .skip(1)
            .find(|(_, line)| line.trim_start().starts_with("impl "))
            .map_or(rest.len(), |(offset, _)| offset);
        let body = &rest[..end];
        let declaration = body
            .split_once("const NAME")
            .and_then(|(_, rest)| rest.split_once(';').map(|(declaration, _)| declaration))
            .ok_or_else(|| format!("{marker} has no NAME declaration"))?;
        let name = declaration
            .split_once('=')
            .map(|(_, value)| value.trim())
            .and_then(|value| value.strip_prefix('"'))
            .and_then(|value| value.strip_suffix('"'))
            .ok_or_else(|| format!("{marker} has a non-literal NAME"))?;
        names.insert(name.to_string());
    }
    match names.len() {
        0 => Err(format!("no NAME implementation for {marker}")),
        1 => Ok(names.into_iter().next().expect("one family name")),
        _ => Err(format!("multiple NAME implementations for {marker}")),
    }
}

fn compat_registered_globals(sources: &[(PathBuf, String)]) -> BTreeSet<String> {
    let mut globals = BTreeSet::new();
    for (_, source) in sources {
        let lines = source.lines().collect::<Vec<_>>();
        for (index, line) in lines.iter().enumerate() {
            if line.contains("register_function(") {
                let mut call = String::new();
                for part in lines.iter().skip(index) {
                    call.push_str(part);
                    call.push('\n');
                    let part = part.trim();
                    if part.contains(".map_err")
                        || part.ends_with("});")
                        || part.ends_with(")?;")
                        || part == "})"
                    {
                        break;
                    }
                }
                if let (true, Some(global)) =
                    (call.contains("crate::compat::"), quoted_global(&call))
                {
                    globals.insert(global);
                }
            }
            if line.contains("install(") {
                let mut call = String::new();
                for part in lines.iter().skip(index) {
                    call.push_str(part);
                    call.push('\n');
                    let part = part.trim();
                    if part.ends_with(")?;") || part.ends_with(");") || part.ends_with(')') {
                        break;
                    }
                }
                match (quoted_global(&call), callback_name(&call)) {
                    (Some(global), Some(callback)) if callback_uses_compat(source, callback) => {
                        globals.insert(global);
                    }
                    _ => {}
                }
            }
        }
    }
    globals
}

fn callback_name(line: &str) -> Option<&str> {
    let candidate = line
        .rsplit_once(',')?
        .1
        .trim()
        .trim_end_matches(")?;")
        .trim_end_matches(')')
        .trim();
    let end = candidate
        .find(|character: char| !(character.is_ascii_alphanumeric() || character == '_'))
        .unwrap_or(candidate.len());
    (end > 0).then_some(&candidate[..end])
}

fn callback_uses_compat(source: &str, callback: &str) -> bool {
    let marker = format!("fn {callback}");
    let Some(start) = source.find(&marker).map(|index| index + marker.len()) else {
        return false;
    };
    let Some(body_start) = source[start..].find('{').map(|index| start + index + 1) else {
        return false;
    };
    let body = &source[body_start..];
    let end = ["\nfn ", "\npub fn ", "\npub(crate) fn ", "\npub(super) fn "]
        .into_iter()
        .filter_map(|marker| body.find(marker))
        .min()
        .unwrap_or(body.len());
    body[..end].contains("crate::compat::")
}

fn quoted_global(source: &str) -> Option<String> {
    let start = source.find("\"__rs2b0t_")? + 1;
    let value = &source[start..];
    let end = value.find('"')?;
    Some(value[..end].to_string())
}

fn rust_sources(root: &Path) -> Result<Vec<(PathBuf, String)>, String> {
    let mut sources = Vec::new();
    let mut directories = vec![root.to_path_buf()];
    while let Some(directory) = directories.pop() {
        let entries = fs::read_dir(&directory)
            .map_err(|error| format!("read {}: {error}", directory.display()))?;
        for entry in entries {
            let entry = entry.map_err(|error| format!("read {}: {error}", directory.display()))?;
            let path = entry.path();
            if path.is_dir() {
                directories.push(path);
            } else if path.extension().is_some_and(|extension| extension == "rs") {
                let source = fs::read_to_string(&path)
                    .map_err(|error| format!("read {}: {error}", path.display()))?;
                sources.push((path, source));
            }
        }
    }
    Ok(sources)
}

fn compat_members_from_source(
    bootstrap_source: &str,
    rust_sources: &[(PathBuf, String)],
    machine_source: &str,
) -> Result<BTreeSet<String>, String> {
    let bootstrap =
        native_v2_main(bootstrap_source).ok_or_else(|| "NATIVE_V2_MAIN is missing".to_string())?;
    let family_sources = rust_sources
        .iter()
        .map(|(_, source)| source.as_str())
        .collect::<Vec<_>>();
    compat_members(&bootstrap, rust_sources, machine_source, &family_sources)
}
