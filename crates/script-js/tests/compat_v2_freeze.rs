//! Freeze the JavaScript v2 members that still enter the compatibility layer
//! (ADR 0023 item 31).
//!
//! The test derives every compat-backed `NativeApi` member from the real
//! sources: `NATIVE_V2_MAIN` in `src/runtime/bindings.rs`, the host-function
//! registrations under `src/`, and the machine family tables in
//! `src/machine/mod.rs`. A member is compat-backed when the code it reaches
//! calls a host function whose Rust registration names `crate::compat::`, or
//! starts a machine family whose `FAMILIES` entry is a `crate::compat` type.
//! The test compares the derived set with `compat_v2_freeze.members`. That list
//! may only shrink from `FROZEN_MEMBERS`; growth needs a reviewed edit of this
//! test.
//!
//! The scanner resolves JavaScript names by scope and fails closed. These forms
//! stop the test with a "cannot classify" message: an unknown import, global
//! name or host function; a family argument that does not resolve to a string
//! literal; an unknown machine family; a family `NAME` that is not a string
//! literal; an alias of `api`, `globalThis` or a family start; a computed
//! access on `api` or `globalThis`; and an `api` write that is not a static
//! path or the reviewed generator loop.
//!
//! The `api` object and its sub-objects cannot escape: `api`, `api.x` or an
//! alias of either used as a value (an argument, an initializer, an element)
//! stops the test, except the uses in `API_VALUE_USES`. So do an assignment or
//! loop head whose pattern writes a member, and a generator loop whose binding
//! is not `const` or is written in its body. The scanner does not follow
//! values through object properties: a property write to any other object,
//! and a method call that passes a function, stop the test unless
//! `REVIEWED_PROPERTY_WRITES` or `REVIEWED_CODE_ARGUMENT_CALLS` names it.
//!
//! Limit: the Rust side parses the sources with `syn` and matches the literal
//! path prefix `crate::compat::`. A registration callback or a family entry
//! counts as compat only when it writes that prefix. This literal
//! `crate::compat::` scan does not resolve Rust `use` aliases (for example
//! `use crate::compat::hunt;` at the top of a file, then `hunt::begin()`), and
//! it does not follow calls from a callback into other Rust functions. A new
//! member that reaches compat only through such an alias is not detected. When
//! a refactor moves a listed member's compat path behind an alias, the member
//! leaves the derived set and the test fails with "no longer compat-backed";
//! that failure is not a family activation.

use deno_ast::swc::ast::{
    ArrowExpr, AssignExpr, AssignTarget, BinaryOp, BindingIdent, CallExpr, Callee, Class,
    ClassDecl, ClassExpr, Decl, Expr, FnDecl, FnExpr, ForHead, ForInStmt, ForOfStmt, Function,
    GetterProp, Ident, ImportDecl, ImportSpecifier, KeyValueProp, Lit, MemberExpr, MemberProp,
    MethodProp, Module, ModuleDecl, ModuleExportName, ModuleItem, NewExpr, ObjectLit, OptChainBase,
    Pat, Prop, PropName, PropOrSpread, SetterProp, SimpleAssignTarget, Stmt, UpdateExpr,
    VarDeclKind, VarDeclarator,
};
use deno_ast::swc::ecma_visit::{Visit, VisitWith};
use deno_ast::{ProgramRef, SourceRangedForSpanned};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fs;
use std::path::Path;
use syn::visit::Visit as SynVisit;

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

const BINDINGS_PATH: &str = "runtime/bindings.rs";
const MACHINE_PATH: &str = "machine/mod.rs";
const KERNEL_MODULE: &str = "../../shim/_kernel.js";
const SCRIPT_MODULE: &str = "./bot.js";
const MACHINE_START: &str = "__rs2b0t_machine_start";
const HOST_PREFIX: &str = "__rs2b0t_";

/// Host values that JavaScript defines, not a Rust registration, with the text
/// that defines them. Each one was reviewed: it holds seat state and calls no
/// host function.
const REVIEWED_JS_HOST_VALUES: &[(&str, &str)] = &[(
    "__rs2b0t_seat_host",
    "Object.defineProperty(globalThis, '__rs2b0t_seat_host'",
)];

/// The only places where the bootstrap uses `api`, or a path under it, as a
/// value, with the source line of each. Any other value use (an argument, an
/// initializer, an array or object element, a return value) stops the test:
/// a member call `api.a.b(…)` and a static write `api.a.b = …` are not value
/// uses.
const API_VALUE_USES: &[&str] = &[
    // The script entry receives the api: `globalThis.__rs_tick`.
    "const r = tick(api);",
    // The host keeps a handle to the api.
    "globalThis.__rs_api = api;",
];

/// Property writes to objects other than `api` and `globalThis`, by enclosing
/// function and statement text. The scanner does not follow values through
/// object properties, so every other such write stops the test. Each one below
/// stores seat state or a request row, never a function.
const REVIEWED_PROPERTY_WRITES: &[(&str, &str)] = &[
    // paintRecorder: the frame title and the painted frame.
    ("title", "rec.title = String(text)"),
    (
        "end",
        "host().paint = { title: rec.title, accent: rec.accent, lines: rec.lines, buttons: rec.buttons }",
    ),
    // enqueueRequest: the request row.
    ("enqueueRequest", "row[field] = op[field]"),
    ("enqueueRequest", "row.from_x = from.x"),
    ("enqueueRequest", "row.from_z = from.z"),
    (
        "enqueueRequest",
        "row.from_level = from.level == null ? 0 : from.level",
    ),
    ("enqueueRequest", "row.x = to.x"),
    ("enqueueRequest", "row.z = to.z"),
    ("enqueueRequest", "row.level = to.level == null ? 0 : to.level"),
    // NativeApi.tick, NativeApi.log and NativeApi.stop: seat state.
    ("tick", "host().tick = n"),
    ("log", "h.log = h.log || []"),
    ("stop", "h.stopRequested = true"),
    ("stop", "h.stopReason = reason"),
    // globalThis.__rs_tick: the tick error.
    ("__rs_tick", "host().lastError = String((e && e.message) || e)"),
];

/// Method calls that pass a function, by enclosing function and call text.
/// A method can store its argument (`push`, `set`, `Object.assign`,
/// `Reflect.set`), so every other such call stops the test. None of the calls
/// below stores into an object that a member reads back for a call.
const REVIEWED_CODE_ARGUMENT_CALLS: &[(&str, &str)] = &[
    // readOnlyView: the snapshot key filter and the proxy cache.
    (
        "ownKeys",
        "keys.filter((k) => typeof k !== 'string' || SNAPSHOT_KEYS.has(k))",
    ),
    ("readOnlyView", "cache.set(value, proxy)"),
    // NativeApi.prayerSet: a settled refusal.
    ("prayerSet", "Promise.resolve(helperErr('invalid-args'))"),
    // globalThis.__rs_tick: the settlement of an async tick.
    (
        "__rs_tick",
        "Promise.resolve(r).then(() => {\n      globalThis.__rs_v2_tick_pending = false;\n      recordSettlement(generation);\n    }, (e) => {\n      globalThis.__rs_v2_tick_pending = false;\n      host().lastError = String((e && e.message) || e);\n    })",
    ),
];

/// ECMAScript built-ins that a member may use. None of them is a host function.
const PURE_GLOBALS: &[&str] = &[
    "Array",
    "Boolean",
    "Error",
    "Infinity",
    "JSON",
    "Map",
    "Math",
    "NaN",
    "Number",
    "Object",
    "Promise",
    "Proxy",
    "RangeError",
    "Reflect",
    "Set",
    "String",
    "Symbol",
    "TypeError",
    "WeakMap",
    "WeakSet",
    "arguments",
    "undefined",
];

#[test]
fn compat_backed_native_v2_members_match_the_closed_list() {
    assert_eq!(
        frozen_members().len(),
        FROZEN_MEMBERS.len(),
        "FROZEN_MEMBERS names are unique"
    );
    let sources = real_sources();
    let rust = RustModel::from_sources(&sources).expect("classify the Rust sources");
    let bootstrap = native_v2_main(&sources).expect("NATIVE_V2_MAIN is present");
    let derived =
        derive_members(&bootstrap, &rust).unwrap_or_else(|error| panic!("{FREEZE_RULE}\n{error}"));
    let closed = parse_closed_members(CLOSED_MEMBERS).expect("parse checked-in closed list");

    if let Err(error) = check_closed_list(&closed, &derived) {
        panic!("{error}\nSource: crates/script-js/src/runtime/bindings.rs, NATIVE_V2_MAIN.");
    }
}

#[test]
fn real_sources_reject_all_reviewed_compat_member_forms() {
    let sources = real_sources();
    let rust = RustModel::from_sources(&sources).expect("classify the Rust sources");
    let mut bootstrap = native_v2_main(&sources).expect("NATIVE_V2_MAIN is present");
    bootstrap.push_str(
        r#"
api.addedRunMachine = (input) => runMachine('prayer', input);
api.addedNow = () => machineNow('prayer', {});
api.addedMachineStart = () => globalThis.__rs2b0t_machine_start('prayer', {}, {});
api.addedAlias = (payload) => {
  const prayer = globalThis.rustyscript.functions.__rs2b0t_prayer;
  return prayer(payload);
};
const prayerHost = globalThis.rustyscript.functions.__rs2b0t_prayer;
api.addedTopLevelAlias = (payload) => prayerHost(payload);
function addedPrayerReference(payload) {
  return globalThis.rustyscript.functions.__rs2b0t_prayer(payload);
}
api.addedReference = addedPrayerReference;
api.addedGroup = { prayerCall };
api.clue.addedNested = (payload) => globalThis.rustyscript.functions.__rs2b0t_clue(payload);
"#,
    );
    let injected = [
        "NativeApi.addedAlias",
        "NativeApi.addedGroup.prayerCall",
        "NativeApi.addedMachineStart",
        "NativeApi.addedNow",
        "NativeApi.addedReference",
        "NativeApi.addedRunMachine",
        "NativeApi.addedTopLevelAlias",
        "NativeApi.clue.addedNested",
    ]
    .map(String::from)
    .into_iter()
    .collect::<BTreeSet<_>>();
    let derived = derive_members(&bootstrap, &rust)
        .unwrap_or_else(|error| panic!("derive members with the injected forms: {error}"));
    let closed = parse_closed_members(CLOSED_MEMBERS).expect("parse checked-in closed list");

    let added = derived
        .difference(&closed)
        .cloned()
        .collect::<BTreeSet<_>>();
    assert_eq!(added, injected, "every injected form is compat-backed");
    let error = check_closed_list(&closed, &derived).expect_err("new members fail the freeze");
    assert!(error.contains("Not on the closed list"), "{error}");

    // A list edit alone cannot admit them: the list may only shrink.
    let grown = closed.union(&injected).cloned().collect::<BTreeSet<_>>();
    let error = check_closed_list(&grown, &derived).expect_err("a longer list fails the freeze");
    assert!(
        error.contains("may only shrink from FROZEN_MEMBERS"),
        "{error}"
    );
}

#[test]
fn real_sources_report_a_listed_member_that_is_no_longer_compat_backed() {
    let sources = real_sources();
    let rust = RustModel::from_sources(&sources).expect("classify the Rust sources");
    let bootstrap = native_v2_main(&sources).expect("NATIVE_V2_MAIN is present");
    let original = "api.prayerPoints = function () { return prayerCall({ op: 'points' }); };";
    assert!(bootstrap.contains(original), "fixture anchor is present");
    let bootstrap = bootstrap.replace(original, "api.prayerPoints = function () { return 1; };");
    let derived = derive_members(&bootstrap, &rust).expect("derive members");
    let closed = parse_closed_members(CLOSED_MEMBERS).expect("parse checked-in closed list");

    let error = check_closed_list(&closed, &derived).expect_err("a stale member fails");
    assert!(
        error.contains("No longer compat-backed: [\"NativeApi.prayerPoints\"]"),
        "{error}"
    );
}

#[test]
fn real_sources_fail_closed_on_forms_the_scanner_cannot_classify() {
    enum Change {
        Append(&'static str),
        Prepend(&'static str),
        Source {
            path: &'static str,
            from: &'static str,
            to: &'static str,
        },
    }
    let cases = [
        (
            Change::Append("api.added = (family) => runMachine(family, {});"),
            "the family argument of runMachine does not resolve",
        ),
        (
            Change::Append("api.added = () => runMachine('no-such-family', {});"),
            "unknown machine family \"no-such-family\"",
        ),
        (
            Change::Append("const start = runMachine;\napi.added = () => start('prayer', {});"),
            "runMachine is referenced outside a direct call",
        ),
        (
            Change::Append(
                "const start = globalThis.__rs2b0t_machine_start;\napi.added = () => start('prayer', {}, {});",
            ),
            "__rs2b0t_machine_start is referenced outside a direct call",
        ),
        (
            Change::Append("api.added = () => globalThis.__rs2b0t_no_such_host();"),
            "the host function __rs2b0t_no_such_host has no registration",
        ),
        (
            Change::Append("api.added = (name) => globalThis.rustyscript.functions[name]();"),
            "a computed access on globalThis",
        ),
        (
            Change::Append("const table = globalThis.rustyscript.functions;\napi.added = () => table.x();"),
            "globalThis.rustyscript.functions",
        ),
        (
            Change::Append("const g = globalThis;\napi.added = () => g.x();"),
            "the global globalThis used as a value",
        ),
        (
            Change::Append("api.added = () => eval('1');"),
            "the global name eval",
        ),
        (
            Change::Prepend("import { queue } from '../../shim/_kernel.js';"),
            "the import of queue from ../../shim/_kernel.js",
        ),
        (
            Change::Append("api['add' + 'ed'] = () => prayerCall({});"),
            "a computed write to api",
        ),
        (
            Change::Append("const alias = api;\nalias.added = () => prayerCall({});"),
            "a bare reference to api",
        ),
        (
            Change::Append("globalThis.__rs_api.added = () => prayerCall({});"),
            "globalThis.__rs_api.added",
        ),
        (
            Change::Append("function install(value) { api.added = value; }"),
            "NativeApi.added = value",
        ),
        (
            Change::Append("api.added = function () { return this.prayerPoints(); };"),
            "this",
        ),
        (
            Change::Source {
                path: "compat/prayer.rs",
                from: "const NAME: &'static str = \"prayer\";",
                to: "const NAME: &'static str = PRAYER;",
            },
            "has a non-literal NAME",
        ),
        (
            Change::Source {
                path: MACHINE_PATH,
                from: "entry::<crate::compat::prayer::Prayer>(),",
                to: "entry::<Prayer>(),",
            },
            "is not a crate path",
        ),
        // Round-2 review probes that passed silently at 09cc58c531: Grok's
        // six forms, then sol's forms A to E.
        (
            Change::Append(
                "const [clueApi] = [api.clue];\nclueApi.addedDestructure = (payload) => globalThis.rustyscript.functions.__rs2b0t_clue(payload);",
            ),
            "the api reference `api.clue` used as a value",
        ),
        (
            Change::Append(
                "Object.assign(api.clue, {\n  addedAssign: (payload) => globalThis.rustyscript.functions.__rs2b0t_clue(payload),\n});",
            ),
            "the api reference `api.clue` used as a value",
        ),
        (
            Change::Append(
                "Object.defineProperty(api.clue, \"addedDefine\", {\n  value: (payload) => globalThis.rustyscript.functions.__rs2b0t_clue(payload),\n});",
            ),
            "the api reference `api.clue` used as a value",
        ),
        (
            Change::Append(
                "Reflect.set(api.clue, \"addedReflect\", (payload) => globalThis.rustyscript.functions.__rs2b0t_clue(payload));",
            ),
            "the api reference `api.clue` used as a value",
        ),
        (
            Change::Append(
                "[api.addedPattern] = [(payload) => globalThis.rustyscript.functions.__rs2b0t_prayer(payload)];",
            ),
            "writes a member",
        ),
        (
            Change::Append(
                "({ prayer: api.addedPatternObj } = {\n  prayer: (payload) => globalThis.rustyscript.functions.__rs2b0t_prayer(payload),\n});",
            ),
            "writes a member",
        ),
        (
            Change::Append(
                "Object.assign(api.clue, { reviewAssignedPrayer: () => prayerCall({ op: 'points' }) });",
            ),
            "the api reference `api.clue` used as a value",
        ),
        (
            Change::Append(
                "Object.defineProperty(api.clue, 'reviewDefinedPrayer', { value: () => prayerCall({ op: 'points' }) });",
            ),
            "the api reference `api.clue` used as a value",
        ),
        (
            Change::Append(
                "const reviewHosts = {};\nreviewHosts.prayer = prayerCall;\nconst { prayer: reviewHostPrayer } = reviewHosts;\napi.reviewDestructuredAlias = () => reviewHostPrayer({ op: 'points' });",
            ),
            "the property write `reviewHosts.prayer = prayerCall`",
        ),
        (
            Change::Append("[api.reviewDestructuredWrite] = [() => prayerCall({ op: 'points' })];"),
            "writes a member",
        ),
        (
            Change::Append(
                "for (let [name, family] of [['prayer', 'prayer']]) {\n  name = 'reviewGenerated';\n  api[name + 'Max'] = () => runMachine(family, { op: 'max' });\n}",
            ),
            "a generator loop binding that is not const or is reassigned",
        ),
        // A method that can store its argument, the same flow as form C.
        (
            Change::Append(
                "const reviewTable = [];\nreviewTable.push(prayerCall);\napi.reviewPushed = () => reviewTable[0]({ op: 'points' });",
            ),
            "the method call `reviewTable.push(prayerCall)` in <module> passes a function",
        ),
    ];

    let real = real_sources();
    let real_rust = RustModel::from_sources(&real).expect("classify the Rust sources");
    let real_bootstrap = native_v2_main(&real).expect("NATIVE_V2_MAIN is present");
    for (change, expected) in cases {
        let result = match change {
            Change::Append(code) => {
                derive_members(&format!("{real_bootstrap}\n{code}\n"), &real_rust)
            }
            Change::Prepend(code) => {
                derive_members(&format!("{code}\n{real_bootstrap}"), &real_rust)
            }
            Change::Source { path, from, to } => {
                let mut sources = real.clone();
                let (_, source) = sources
                    .iter_mut()
                    .find(|(file, _)| file == path)
                    .unwrap_or_else(|| panic!("{path} is present"));
                assert!(source.contains(from), "{path} contains `{from}`");
                *source = source.replace(from, to);
                RustModel::from_sources(&sources)
                    .and_then(|rust| derive_members(&real_bootstrap, &rust))
            }
        };
        let error = result.expect_err(&format!(
            "the scanner refuses a form that gives `{expected}`"
        ));
        assert!(error.starts_with("cannot classify"), "{error}");
        assert!(
            error.contains(expected),
            "expected `{expected}` in: {error}"
        );
    }
}

fn frozen_members() -> BTreeSet<String> {
    FROZEN_MEMBERS
        .iter()
        .map(|member| member.to_string())
        .collect()
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

fn check_closed_list(closed: &BTreeSet<String>, derived: &BTreeSet<String>) -> Result<(), String> {
    let frozen = frozen_members();
    let unfrozen = closed.difference(&frozen).collect::<Vec<_>>();
    if !unfrozen.is_empty() {
        return Err(format!(
            "{FREEZE_RULE}\nThe closed list may only shrink from FROZEN_MEMBERS. Not frozen: {unfrozen:?}"
        ));
    }
    let additions = derived.difference(closed).collect::<Vec<_>>();
    let stale = closed.difference(derived).collect::<Vec<_>>();
    if additions.is_empty() && stale.is_empty() {
        return Ok(());
    }
    Err(format!(
        "{FREEZE_RULE}\nNot on the closed list: {additions:?}\nNo longer compat-backed: {stale:?}\n\
         A member leaves the list when its family activation moves it to the native core. \
         A Rust `use` alias in its registration also hides it (see the doc comment of this test)."
    ))
}

// JavaScript side: scope-resolved facts of NATIVE_V2_MAIN.

/// A binding: the symbol and the scope context that the resolver gave it.
type Name = (String, u32);
/// Family-name values of bindings, known from string literals.
type Env = BTreeMap<Name, String>;

fn name_of(ident: &Ident) -> Name {
    (ident.sym.to_string(), ident.ctxt.as_u32())
}

#[derive(Clone, Debug)]
enum Arg {
    Str(String),
    Name(Name),
    Other,
}

struct Scope {
    unresolved: u32,
    api: Name,
    /// Local names of the kernel family starts, with the imported name.
    starters: BTreeMap<Name, String>,
    /// Imports that call no host function (the script's own `tick`).
    inert_imports: BTreeSet<Name>,
}

impl Scope {
    fn is_global(&self, ident: &Ident) -> bool {
        ident.ctxt.as_u32() == self.unresolved
    }

    fn is_global_this(&self, ident: &Ident) -> bool {
        self.is_global(ident) && &*ident.sym == "globalThis"
    }

    fn is_api(&self, ident: &Ident) -> bool {
        name_of(ident) == self.api
    }

    fn arg(&self, expr: &Expr) -> Arg {
        match unparen(expr) {
            Expr::Lit(Lit::Str(value)) => Arg::Str(value.value.to_string()),
            Expr::Ident(ident) if !self.is_global(ident) => Arg::Name(name_of(ident)),
            _ => Arg::Other,
        }
    }
}

fn module_scope(module: &Module, unresolved: u32) -> Result<Scope, String> {
    let mut starters = BTreeMap::new();
    let mut inert_imports = BTreeSet::new();
    let mut api = Vec::new();
    for item in &module.body {
        match item {
            ModuleItem::ModuleDecl(ModuleDecl::Import(import)) => {
                let from = import.src.value.to_string();
                for specifier in &import.specifiers {
                    let ImportSpecifier::Named(named) = specifier else {
                        return Err(format!(
                            "cannot classify a default or namespace import from {from}"
                        ));
                    };
                    let imported = match &named.imported {
                        Some(ModuleExportName::Ident(ident)) => ident.sym.to_string(),
                        Some(ModuleExportName::Str(value)) => value.value.to_string(),
                        None => named.local.sym.to_string(),
                    };
                    match (from.as_str(), imported.as_str()) {
                        (KERNEL_MODULE, "runMachine" | "machineNow") => {
                            starters.insert(name_of(&named.local), imported);
                        }
                        (SCRIPT_MODULE, "tick") => {
                            inert_imports.insert(name_of(&named.local));
                        }
                        _ => {
                            return Err(format!(
                                "cannot classify the import of {imported} from {from}"
                            ))
                        }
                    }
                }
            }
            ModuleItem::ModuleDecl(_) => {
                return Err("cannot classify a module declaration other than an import".into())
            }
            ModuleItem::Stmt(Stmt::Decl(Decl::Var(var))) => {
                for declarator in &var.decls {
                    if let Pat::Ident(binding) = &declarator.name {
                        if &*binding.id.sym == "api" {
                            api.push(name_of(&binding.id));
                        }
                    }
                }
            }
            _ => {}
        }
    }
    let [api] = api.as_slice() else {
        return Err("cannot classify the bootstrap: it must declare one top-level api".into());
    };
    Ok(Scope {
        unresolved,
        api: api.clone(),
        starters,
        inert_imports,
    })
}

/// What one function (or initializer) can reach. Nested functions count as
/// part of the function that contains them.
#[derive(Clone, Debug, Default)]
struct Facts {
    params: Vec<Option<Name>>,
    calls: Vec<(Name, Vec<Arg>)>,
    starts: Vec<(String, Option<Arg>)>,
    refs: BTreeSet<Name>,
    api_uses: Vec<(String, Vec<Arg>)>,
    hosts: BTreeSet<String>,
    /// The node holds a function, method or class (its value can be code).
    functions: bool,
    errors: Vec<String>,
}

struct FactsCollector<'a> {
    scope: &'a Scope,
    facts: Facts,
}

impl<'a> FactsCollector<'a> {
    fn new(scope: &'a Scope, params: Vec<Option<Name>>) -> Self {
        Self {
            scope,
            facts: Facts {
                params,
                ..Facts::default()
            },
        }
    }

    fn error(&mut self, message: String) {
        self.facts.errors.push(message);
    }

    fn reference(&mut self, ident: &Ident) {
        let name = name_of(ident);
        if self.scope.is_api(ident) {
            self.error("a bare reference to api".into());
        } else if let Some(starter) = self.scope.starters.get(&name) {
            self.error(format!("{starter} is referenced outside a direct call"));
        } else if self.scope.is_global(ident) {
            self.global(ident);
        } else {
            self.facts.refs.insert(name);
        }
    }

    fn global(&mut self, ident: &Ident) {
        if !PURE_GLOBALS.contains(&&*ident.sym) {
            self.error(format!("the global name {}", ident.sym));
        }
    }

    fn global_this_chain(&mut self, parts: &[String]) {
        let host = match parts {
            [host, ..] if host.starts_with(HOST_PREFIX) => host,
            [namespace, functions, host, ..]
                if namespace == "rustyscript"
                    && functions == "functions"
                    && host.starts_with(HOST_PREFIX) =>
            {
                host
            }
            _ => {
                self.error(format!("globalThis.{}", parts.join(".")));
                return;
            }
        };
        self.facts.hosts.insert(host.clone());
    }

    fn callee(&mut self, callee: &Expr, args: Vec<Arg>) {
        if let Expr::Ident(ident) = callee {
            let name = name_of(ident);
            if let Some(starter) = self.scope.starters.get(&name) {
                let starter = starter.clone();
                self.facts.starts.push((starter, args.into_iter().next()));
            } else if self.scope.is_api(ident) {
                self.error("a bare reference to api".into());
            } else if self.scope.is_global(ident) {
                self.global(ident);
            } else {
                self.facts.calls.push((name, args));
            }
            return;
        }
        if let Some((root, parts)) = static_chain(callee) {
            if self.scope.is_api(root) {
                self.facts.api_uses.push((api_path(&parts), args));
                return;
            }
            if self.scope.is_global_this(root) && parts == [MACHINE_START] {
                self.facts
                    .starts
                    .push((MACHINE_START.to_string(), args.into_iter().next()));
                return;
            }
        }
        callee.visit_with(self);
    }
}

impl Visit for FactsCollector<'_> {
    fn visit_call_expr(&mut self, call: &CallExpr) {
        let args = call
            .args
            .iter()
            .map(|arg| {
                if arg.spread.is_some() {
                    Arg::Other
                } else {
                    self.scope.arg(&arg.expr)
                }
            })
            .collect::<Vec<_>>();
        match &call.callee {
            Callee::Import(_) => self.error("a dynamic import()".into()),
            Callee::Super(_) => {}
            Callee::Expr(callee) => self.callee(unparen(callee), args),
        }
        call.args.visit_with(self);
    }

    fn visit_member_expr(&mut self, member: &MemberExpr) {
        if let Some((root, parts)) = member_chain(member) {
            if self.scope.is_api(root) {
                self.facts.api_uses.push((api_path(&parts), Vec::new()));
                return;
            }
            if self.scope.is_global_this(root) {
                self.global_this_chain(&parts);
                return;
            }
        } else if let MemberProp::Computed(computed) = &member.prop {
            if string_literal(&computed.expr).is_none() {
                if let Some(root) = chain_root(&member.obj) {
                    if self.scope.is_api(root) || self.scope.is_global_this(root) {
                        self.error(format!("a computed access on {}", root.sym));
                    }
                }
            }
        }
        if let Some(name) = member_prop_name(&member.prop) {
            if name.starts_with(HOST_PREFIX) {
                self.facts.hosts.insert(name);
            }
        }
        member.visit_children_with(self);
    }

    fn visit_expr(&mut self, expr: &Expr) {
        match expr {
            Expr::Ident(ident) => {
                self.reference(ident);
                return;
            }
            Expr::This(_) => self.error("this".into()),
            Expr::Lit(Lit::Str(value)) if value.value.starts_with(HOST_PREFIX) => {
                self.facts.hosts.insert(value.value.to_string());
            }
            _ => {}
        }
        expr.visit_children_with(self);
    }

    fn visit_prop(&mut self, prop: &Prop) {
        if let Prop::Shorthand(ident) = prop {
            self.reference(ident);
            return;
        }
        prop.visit_children_with(self);
    }

    fn visit_function(&mut self, function: &Function) {
        self.facts.functions = true;
        function.visit_children_with(self);
    }

    fn visit_arrow_expr(&mut self, arrow: &ArrowExpr) {
        self.facts.functions = true;
        arrow.visit_children_with(self);
    }

    fn visit_getter_prop(&mut self, getter: &GetterProp) {
        self.facts.functions = true;
        getter.visit_children_with(self);
    }

    fn visit_setter_prop(&mut self, setter: &SetterProp) {
        self.facts.functions = true;
        setter.visit_children_with(self);
    }

    fn visit_class(&mut self, class: &Class) {
        self.facts.functions = true;
        class.visit_children_with(self);
    }
}

fn function_facts(scope: &Scope, function: &Function) -> Facts {
    let params = function
        .params
        .iter()
        .map(|param| pat_name(&param.pat))
        .collect();
    let mut collector = FactsCollector::new(scope, params);
    function.params.visit_with(&mut collector);
    function.body.visit_with(&mut collector);
    collector.facts
}

fn arrow_facts(scope: &Scope, arrow: &ArrowExpr) -> Facts {
    let mut collector = FactsCollector::new(scope, arrow.params.iter().map(pat_name).collect());
    arrow.params.visit_with(&mut collector);
    arrow.body.visit_with(&mut collector);
    collector.facts
}

fn expr_facts(scope: &Scope, expr: &Expr) -> Facts {
    match unparen(expr) {
        Expr::Fn(function) => Facts {
            functions: true,
            ..function_facts(scope, &function.function)
        },
        Expr::Arrow(arrow) => Facts {
            functions: true,
            ..arrow_facts(scope, arrow)
        },
        other => {
            let mut collector = FactsCollector::new(scope, Vec::new());
            other.visit_with(&mut collector);
            collector.facts
        }
    }
}

fn pat_name(pat: &Pat) -> Option<Name> {
    match pat {
        Pat::Ident(binding) => Some(name_of(&binding.id)),
        Pat::Assign(assign) => pat_name(&assign.left),
        _ => None,
    }
}

#[derive(Default)]
struct BindingNames(Vec<Name>);

impl Visit for BindingNames {
    fn visit_binding_ident(&mut self, binding: &BindingIdent) {
        self.0.push(name_of(&binding.id));
    }

    fn visit_expr(&mut self, _: &Expr) {}
}

fn binding_names<N: VisitWith<BindingNames>>(node: &N) -> Vec<Name> {
    let mut names = BindingNames::default();
    node.visit_with(&mut names);
    names.0
}

/// Every declared name, and the facts of each value bound to it.
struct BindingCollector<'a> {
    scope: &'a Scope,
    facts: &'a mut Vec<Facts>,
    by_name: BTreeMap<Name, Vec<usize>>,
    declared: BTreeSet<Name>,
}

impl BindingCollector<'_> {
    fn bind(&mut self, name: Name, facts: Facts) {
        self.declared.insert(name.clone());
        self.by_name.entry(name).or_default().push(self.facts.len());
        self.facts.push(facts);
    }
}

impl Visit for BindingCollector<'_> {
    fn visit_fn_decl(&mut self, function: &FnDecl) {
        let mut facts = function_facts(self.scope, &function.function);
        facts.functions = true;
        self.bind(name_of(&function.ident), facts);
        function.visit_children_with(self);
    }

    fn visit_fn_expr(&mut self, function: &FnExpr) {
        if let Some(ident) = &function.ident {
            let mut facts = function_facts(self.scope, &function.function);
            facts.functions = true;
            self.bind(name_of(ident), facts);
        }
        function.visit_children_with(self);
    }

    fn visit_class_decl(&mut self, class: &ClassDecl) {
        let mut collector = FactsCollector::new(self.scope, Vec::new());
        class.class.visit_with(&mut collector);
        self.bind(name_of(&class.ident), collector.facts);
        class.visit_children_with(self);
    }

    fn visit_class_expr(&mut self, class: &ClassExpr) {
        if let Some(ident) = &class.ident {
            let mut collector = FactsCollector::new(self.scope, Vec::new());
            class.class.visit_with(&mut collector);
            self.bind(name_of(ident), collector.facts);
        }
        class.visit_children_with(self);
    }

    fn visit_var_declarator(&mut self, declarator: &VarDeclarator) {
        if let Some(init) = &declarator.init {
            let facts = expr_facts(self.scope, init);
            for name in binding_names(&declarator.name) {
                self.bind(name, facts.clone());
            }
        }
        declarator.visit_children_with(self);
    }

    fn visit_assign_expr(&mut self, assignment: &AssignExpr) {
        let names = match &assignment.left {
            AssignTarget::Simple(SimpleAssignTarget::Ident(binding)) => vec![name_of(&binding.id)],
            AssignTarget::Pat(pattern) => binding_names(pattern),
            AssignTarget::Simple(_) => Vec::new(),
        };
        if !names.is_empty() {
            let facts = expr_facts(self.scope, &assignment.right);
            for name in names {
                self.bind(name, facts.clone());
            }
        }
        assignment.visit_children_with(self);
    }

    fn visit_binding_ident(&mut self, binding: &BindingIdent) {
        self.declared.insert(name_of(&binding.id));
    }

    fn visit_import_decl(&mut self, import: &ImportDecl) {
        for specifier in &import.specifiers {
            if let ImportSpecifier::Named(named) = specifier {
                self.declared.insert(name_of(&named.local));
            }
        }
    }
}

/// Every write that defines an `api` member, and the checks that keep the
/// `api` object from escaping through an alias.
struct ApiCollector<'a> {
    scope: &'a Scope,
    source: &'a deno_ast::ParsedSource,
    facts: &'a mut Vec<Facts>,
    by_name: &'a BTreeMap<Name, Vec<usize>>,
    /// Bindings whose value can be code: functions, host references, and
    /// values that hold either.
    code_names: &'a BTreeSet<Name>,
    members: BTreeMap<String, Vec<(usize, Env)>>,
    setters: BTreeSet<String>,
    generated: BTreeSet<(u32, u32)>,
    /// The names of the enclosing functions, innermost last.
    enclosing: Vec<String>,
    errors: Vec<String>,
}

impl ApiCollector<'_> {
    fn member(&mut self, path: String, facts: Facts, captured: Env) {
        self.members
            .entry(path)
            .or_default()
            .push((self.facts.len(), captured));
        self.facts.push(facts);
    }

    fn prop_path(&mut self, parent: &str, key: &PropName) -> Option<String> {
        match prop_name(key) {
            Some(name) => Some(format!("{parent}.{name}")),
            None => {
                self.errors.push(format!("a computed key in {parent}"));
                None
            }
        }
    }

    fn collect_object(&mut self, parent: &str, object: &ObjectLit) {
        for property in &object.props {
            let PropOrSpread::Prop(property) = property else {
                self.errors.push(format!("a spread in {parent}"));
                continue;
            };
            match property.as_ref() {
                Prop::Method(method) => {
                    if let Some(path) = self.prop_path(parent, &method.key) {
                        let facts = function_facts(self.scope, &method.function);
                        self.member(path, facts, Env::new());
                    }
                }
                Prop::Getter(getter) => {
                    if let Some(path) = self.prop_path(parent, &getter.key) {
                        let mut collector = FactsCollector::new(self.scope, Vec::new());
                        getter.body.visit_with(&mut collector);
                        self.member(path, collector.facts, Env::new());
                    }
                }
                Prop::Setter(setter) => {
                    if let Some(path) = self.prop_path(parent, &setter.key) {
                        let mut collector =
                            FactsCollector::new(self.scope, vec![pat_name(&setter.param)]);
                        setter.param.visit_with(&mut collector);
                        setter.body.visit_with(&mut collector);
                        self.setters.insert(path.clone());
                        self.member(path, collector.facts, Env::new());
                    }
                }
                Prop::KeyValue(value) => {
                    if let Some(path) = self.prop_path(parent, &value.key) {
                        self.collect_value(&path, &value.value);
                    }
                }
                Prop::Shorthand(ident) => {
                    let path = format!("{parent}.{}", ident.sym);
                    self.collect_value(&path, &Expr::Ident(ident.clone()));
                }
                Prop::Assign(_) => self
                    .errors
                    .push(format!("an assignment property in {parent}")),
            }
        }
    }

    fn collect_value(&mut self, path: &str, value: &Expr) {
        match unparen(value) {
            Expr::Object(object) => self.collect_object(path, object),
            Expr::Ident(ident)
                if !self.scope.is_global(ident)
                    && !self.scope.is_api(ident)
                    && !self.scope.starters.contains_key(&name_of(ident))
                    && !self.by_name.contains_key(&name_of(ident)) =>
            {
                self.errors.push(format!(
                    "{path} = {}: the value has no known definition",
                    ident.sym
                ));
            }
            other => {
                let facts = expr_facts(self.scope, other);
                self.member(path.to_string(), facts, Env::new());
            }
        }
    }

    /// The reviewed generator: `for (const [name, family] of [['a', 'f'], …])`
    /// with `api[name + 'Suffix'] = <function>` writes in its body.
    fn generator(&self, statement: &ForOfStmt) -> Option<GeneratedMembers> {
        let ForHead::VarDecl(declaration) = &statement.left else {
            return None;
        };
        let [first] = declaration.decls.as_slice() else {
            return None;
        };
        let Pat::Array(binding) = &first.name else {
            return None;
        };
        let [Some(Pat::Ident(row)), Some(Pat::Ident(family))] = binding.elems.as_slice() else {
            return None;
        };
        let Expr::Array(rows) = unparen(&statement.right) else {
            return None;
        };
        let mut table = Vec::new();
        for element in &rows.elems {
            let element = element.as_ref()?;
            if element.spread.is_some() {
                return None;
            }
            let Expr::Array(pair) = unparen(&element.expr) else {
                return None;
            };
            let [Some(name), Some(family_name)] = pair.elems.as_slice() else {
                return None;
            };
            if name.spread.is_some() || family_name.spread.is_some() {
                return None;
            }
            table.push((
                string_literal(&name.expr)?,
                string_literal(&family_name.expr)?,
            ));
        }
        let mut writes = GeneratedWrites {
            scope: self.scope,
            row: name_of(&row.id),
            loop_bindings: [name_of(&row.id), name_of(&family.id)],
            found: Vec::new(),
            unrecognized: false,
            reassigned: false,
        };
        statement.body.visit_with(&mut writes);
        if writes.unrecognized {
            return None;
        }
        Some(GeneratedMembers {
            family: name_of(&family.id),
            table,
            mutable: declaration.kind != VarDeclKind::Const || writes.reassigned,
            writes: writes.found,
        })
    }
}

struct GeneratedMembers {
    family: Name,
    table: Vec<(String, String)>,
    /// A loop binding is `let`/`var` or is written in the body, so the table
    /// row is not the key or family of the written member.
    mutable: bool,
    writes: Vec<((u32, u32), String, Facts)>,
}

struct GeneratedWrites<'a> {
    scope: &'a Scope,
    row: Name,
    loop_bindings: [Name; 2],
    found: Vec<((u32, u32), String, Facts)>,
    unrecognized: bool,
    reassigned: bool,
}

impl Visit for GeneratedWrites<'_> {
    fn visit_assign_expr(&mut self, assignment: &AssignExpr) {
        let written = match &assignment.left {
            AssignTarget::Simple(SimpleAssignTarget::Ident(binding)) => vec![name_of(&binding.id)],
            AssignTarget::Pat(pattern) => binding_names(pattern),
            AssignTarget::Simple(_) => Vec::new(),
        };
        if written.iter().any(|name| self.loop_bindings.contains(name)) {
            self.reassigned = true;
        }
        if let AssignTarget::Simple(SimpleAssignTarget::Member(member)) = &assignment.left {
            if member_chain(member).is_none()
                && chain_root(&member.obj).is_some_and(|root| self.scope.is_api(root))
            {
                match generated_suffix(member, &self.row) {
                    Some(suffix)
                        if matches!(unparen(&assignment.right), Expr::Fn(_) | Expr::Arrow(_)) =>
                    {
                        let facts = expr_facts(self.scope, &assignment.right);
                        self.found.push((span_key(assignment), suffix, facts));
                    }
                    _ => self.unrecognized = true,
                }
            }
        }
        assignment.visit_children_with(self);
    }

    fn visit_update_expr(&mut self, update: &UpdateExpr) {
        if let Expr::Ident(ident) = unparen(&update.arg) {
            if self.loop_bindings.contains(&name_of(ident)) {
                self.reassigned = true;
            }
        }
        update.visit_children_with(self);
    }
}

fn generated_suffix(member: &MemberExpr, row: &Name) -> Option<String> {
    if !matches!(member.obj.as_ref(), Expr::Ident(_)) {
        return None;
    }
    let MemberProp::Computed(computed) = &member.prop else {
        return None;
    };
    let Expr::Bin(binary) = computed.expr.as_ref() else {
        return None;
    };
    if binary.op != BinaryOp::Add
        || !matches!(binary.left.as_ref(), Expr::Ident(ident) if name_of(ident) == *row)
    {
        return None;
    }
    string_literal(&binary.right)
}

fn span_key(assignment: &AssignExpr) -> (u32, u32) {
    (assignment.span.lo.0, assignment.span.hi.0)
}

impl ApiCollector<'_> {
    fn enclosing_name(&self) -> &str {
        self.enclosing.last().map_or("<module>", String::as_str)
    }

    fn reviewed(&self, list: &[(&str, &str)], text: &str) -> bool {
        let enclosing = self.enclosing_name();
        list.iter()
            .any(|(function, reviewed)| *function == enclosing && *reviewed == text)
    }

    fn within(&mut self, name: Option<String>, visit: impl FnOnce(&mut Self)) {
        let pushed = name.is_some();
        if let Some(name) = name {
            self.enclosing.push(name);
        }
        visit(self);
        if pushed {
            self.enclosing.pop();
        }
    }

    fn visit_right(&mut self, assignment: &AssignExpr) {
        let name = function_value(&assignment.right)
            .then(|| assigned_name(&assignment.left))
            .flatten();
        self.within(name, |this| assignment.right.visit_with(this));
    }

    /// An identifier in a value position.
    fn value_ident(&mut self, ident: &Ident) {
        if self.scope.is_api(ident) {
            self.errors.push("a bare reference to api".into());
        } else if self.scope.is_global(ident)
            && !matches!(&*ident.sym, "undefined" | "NaN" | "Infinity")
        {
            self.errors
                .push(format!("the global {} used as a value", ident.sym));
        }
    }

    /// Whether the value of `expr` can be a function.
    fn carries_code(&self, expr: &Expr) -> bool {
        match unparen(expr) {
            Expr::Fn(_) | Expr::Arrow(_) | Expr::Class(_) => true,
            Expr::Ident(ident) => {
                let name = name_of(ident);
                self.code_names.contains(&name)
                    || self.scope.starters.contains_key(&name)
                    || self.scope.is_global_this(ident)
            }
            Expr::Member(_) | Expr::OptChain(_) => chain_root(expr).is_some_and(|root| {
                self.code_names.contains(&name_of(root))
                    || self.scope.is_global_this(root)
                    || self.scope.is_api(root)
            }),
            Expr::Array(array) => array
                .elems
                .iter()
                .flatten()
                .any(|element| self.carries_code(&element.expr)),
            Expr::Object(object) => object.props.iter().any(|property| match property {
                PropOrSpread::Spread(spread) => self.carries_code(&spread.expr),
                PropOrSpread::Prop(property) => match property.as_ref() {
                    Prop::KeyValue(value) => self.carries_code(&value.value),
                    Prop::Shorthand(ident) => self.carries_code(&Expr::Ident(ident.clone())),
                    _ => true,
                },
            }),
            Expr::Cond(cond) => self.carries_code(&cond.cons) || self.carries_code(&cond.alt),
            Expr::Bin(binary) => {
                self.carries_code(&binary.left) || self.carries_code(&binary.right)
            }
            Expr::Seq(sequence) => sequence
                .exprs
                .last()
                .is_some_and(|last| self.carries_code(last)),
            Expr::Assign(assignment) => self.carries_code(&assignment.right),
            Expr::Call(call) => matches!(
                &call.callee,
                Callee::Expr(callee)
                    if matches!(unparen(callee), Expr::Ident(ident) if self.code_names.contains(&name_of(ident)))
            ),
            _ => false,
        }
    }

    fn loop_head(&mut self, head: &ForHead) {
        if let ForHead::Pat(pattern) = head {
            if contains_member(pattern.as_ref()) {
                self.errors.push("a loop head that writes a member".into());
            }
        }
    }
}

fn function_value(expr: &Expr) -> bool {
    matches!(unparen(expr), Expr::Fn(_) | Expr::Arrow(_))
}

fn assigned_name(target: &AssignTarget) -> Option<String> {
    match target {
        AssignTarget::Simple(SimpleAssignTarget::Ident(binding)) => {
            Some(binding.id.sym.to_string())
        }
        AssignTarget::Simple(SimpleAssignTarget::Member(member)) => member_prop_name(&member.prop),
        _ => None,
    }
}

#[derive(Default)]
struct MemberFinder(bool);

impl Visit for MemberFinder {
    fn visit_member_expr(&mut self, _: &MemberExpr) {
        self.0 = true;
    }
}

fn contains_member<N: VisitWith<MemberFinder>>(node: &N) -> bool {
    let mut finder = MemberFinder::default();
    node.visit_with(&mut finder);
    finder.0
}

impl Visit for ApiCollector<'_> {
    fn visit_fn_decl(&mut self, function: &FnDecl) {
        self.within(Some(function.ident.sym.to_string()), |this| {
            function.visit_children_with(this)
        });
    }

    fn visit_method_prop(&mut self, method: &MethodProp) {
        self.within(prop_name(&method.key), |this| {
            method.visit_children_with(this)
        });
    }

    fn visit_getter_prop(&mut self, getter: &GetterProp) {
        self.within(prop_name(&getter.key), |this| {
            getter.visit_children_with(this)
        });
    }

    fn visit_setter_prop(&mut self, setter: &SetterProp) {
        self.within(prop_name(&setter.key), |this| {
            setter.visit_children_with(this)
        });
    }

    fn visit_key_value_prop(&mut self, property: &KeyValueProp) {
        let name = function_value(&property.value)
            .then(|| prop_name(&property.key))
            .flatten();
        self.within(name, |this| property.visit_children_with(this));
    }

    fn visit_var_declarator(&mut self, declarator: &VarDeclarator) {
        let mut name = None;
        if let Pat::Ident(binding) = &declarator.name {
            if self.scope.is_api(&binding.id) {
                match declarator.init.as_deref().map(unparen) {
                    Some(Expr::Object(object)) => self.collect_object("NativeApi", object),
                    _ => self
                        .errors
                        .push("api is not declared as an object literal".into()),
                }
            }
            if declarator.init.as_deref().is_some_and(function_value) {
                name = Some(binding.id.sym.to_string());
            }
        }
        self.within(name, |this| declarator.visit_children_with(this));
    }

    fn visit_for_of_stmt(&mut self, statement: &ForOfStmt) {
        self.loop_head(&statement.left);
        if let Some(generated) = self.generator(statement) {
            if generated.mutable && !generated.writes.is_empty() {
                self.errors
                    .push("a generator loop binding that is not const or is reassigned".into());
            }
            for (span, suffix, facts) in generated.writes {
                self.generated.insert(span);
                let index = self.facts.len();
                self.facts.push(facts);
                for (row, family) in &generated.table {
                    self.members
                        .entry(format!("NativeApi.{row}{suffix}"))
                        .or_default()
                        .push((
                            index,
                            Env::from([(generated.family.clone(), family.clone())]),
                        ));
                }
            }
        }
        statement.visit_children_with(self);
    }

    fn visit_for_in_stmt(&mut self, statement: &ForInStmt) {
        self.loop_head(&statement.left);
        statement.visit_children_with(self);
    }

    fn visit_assign_expr(&mut self, assignment: &AssignExpr) {
        match &assignment.left {
            AssignTarget::Pat(pattern) => {
                if contains_member(pattern) {
                    self.errors.push(format!(
                        "the pattern assignment `{}` writes a member",
                        assignment.text_fast(self.source)
                    ));
                }
            }
            AssignTarget::Simple(SimpleAssignTarget::Member(member)) => {
                match member_chain(member) {
                    Some((root, parts)) if self.scope.is_api(root) => {
                        let path = api_path(&parts);
                        if !self.setters.contains(&path) {
                            self.collect_value(&path, &assignment.right);
                        }
                        self.visit_right(assignment);
                        return;
                    }
                    Some((root, parts)) if self.scope.is_global_this(root) => {
                        // A reviewed api value use (API_VALUE_USES).
                        if parts == ["__rs_api"]
                            && matches!(unparen(&assignment.right), Expr::Ident(ident) if self.scope.is_api(ident))
                        {
                            return;
                        }
                        if parts
                            .first()
                            .is_some_and(|part| part.starts_with(HOST_PREFIX) || part == "__rs_api")
                        {
                            self.errors
                                .push(format!("a write to globalThis.{}", parts.join(".")));
                        }
                        self.visit_right(assignment);
                        return;
                    }
                    None if chain_root(&member.obj).is_some_and(|root| self.scope.is_api(root)) => {
                        if self.generated.contains(&span_key(assignment)) {
                            self.visit_right(assignment);
                        } else {
                            self.errors.push("a computed write to api".into());
                        }
                        return;
                    }
                    _ => {
                        let text = assignment.text_fast(self.source);
                        if !self.reviewed(REVIEWED_PROPERTY_WRITES, text) {
                            self.errors.push(format!(
                                "the property write `{text}` in {}",
                                self.enclosing_name()
                            ));
                        }
                    }
                }
            }
            AssignTarget::Simple(_) => {}
        }
        assignment.left.visit_with(self);
        self.visit_right(assignment);
    }

    fn visit_member_expr(&mut self, member: &MemberExpr) {
        if let Some(root) = chain_root(&member.obj) {
            if self.scope.is_api(root) && member_chain(member).is_none() {
                self.errors.push("a computed access on api".into());
            }
            if self.scope.is_global_this(root) {
                if let Some((_, parts)) = member_chain(member) {
                    if parts.first().is_some_and(|part| part == "__rs_api") {
                        self.errors.push(format!("globalThis.{}", parts.join(".")));
                    }
                }
            }
        }
        // Computed keys and a non-identifier root are values; the root
        // identifier of the chain is not a value use.
        let mut current = member;
        loop {
            if let MemberProp::Computed(computed) = &current.prop {
                computed.expr.visit_with(self);
            }
            match unparen(&current.obj) {
                Expr::Member(inner) => current = inner,
                Expr::Ident(_) => break,
                other => {
                    other.visit_with(self);
                    break;
                }
            }
        }
    }

    fn visit_expr(&mut self, expr: &Expr) {
        match expr {
            Expr::Ident(ident) => {
                self.value_ident(ident);
                return;
            }
            Expr::Member(_) | Expr::OptChain(_)
                if chain_root(expr).is_some_and(|root| self.scope.is_api(root)) =>
            {
                self.errors.push(format!(
                    "the api reference `{}` used as a value",
                    expr.text_fast(self.source)
                ));
                return;
            }
            _ => {}
        }
        expr.visit_children_with(self);
    }

    fn visit_prop(&mut self, prop: &Prop) {
        if let Prop::Shorthand(ident) = prop {
            self.value_ident(ident);
            return;
        }
        prop.visit_children_with(self);
    }

    fn visit_new_expr(&mut self, new: &NewExpr) {
        if !matches!(unparen(&new.callee), Expr::Ident(_)) {
            new.callee.visit_with(self);
        }
        new.args.visit_with(self);
    }

    fn visit_call_expr(&mut self, call: &CallExpr) {
        let mut script_entry = false;
        match &call.callee {
            Callee::Import(_) => self.errors.push("a dynamic import()".into()),
            Callee::Super(_) => {}
            Callee::Expr(callee) => match unparen(callee) {
                // A function or built-in callee is not a value use.
                Expr::Ident(ident) => {
                    script_entry = self.scope.inert_imports.contains(&name_of(ident));
                }
                // A call of an api member.
                Expr::Member(member)
                    if member_chain(member).is_some_and(|(root, _)| self.scope.is_api(root)) => {}
                Expr::Member(member) => {
                    self.visit_member_expr(member);
                    if call.args.iter().any(|arg| self.carries_code(&arg.expr)) {
                        let text = call.text_fast(self.source);
                        if !self.reviewed(REVIEWED_CODE_ARGUMENT_CALLS, text) {
                            self.errors.push(format!(
                                "the method call `{text}` in {} passes a function",
                                self.enclosing_name()
                            ));
                        }
                    }
                }
                other => other.visit_with(self),
            },
        }
        for arg in &call.args {
            // A reviewed api value use (API_VALUE_USES): the script's `tick(api)`.
            if script_entry
                && matches!(unparen(&arg.expr), Expr::Ident(ident) if self.scope.is_api(ident))
            {
                continue;
            }
            arg.visit_with(self);
        }
    }
}

fn unparen(expr: &Expr) -> &Expr {
    match expr {
        Expr::Paren(paren) => unparen(&paren.expr),
        _ => expr,
    }
}

fn string_literal(expr: &Expr) -> Option<String> {
    match unparen(expr) {
        Expr::Lit(Lit::Str(value)) => Some(value.value.to_string()),
        _ => None,
    }
}

fn member_prop_name(prop: &MemberProp) -> Option<String> {
    match prop {
        MemberProp::Ident(ident) => Some(ident.sym.to_string()),
        MemberProp::Computed(computed) => string_literal(&computed.expr),
        MemberProp::PrivateName(_) => None,
    }
}

fn prop_name(name: &PropName) -> Option<String> {
    match name {
        PropName::Ident(ident) => Some(ident.sym.to_string()),
        PropName::Str(value) => Some(value.value.to_string()),
        PropName::Computed(computed) => string_literal(&computed.expr),
        PropName::Num(value) => Some(value.value.to_string()),
        PropName::BigInt(value) => Some(value.value.to_string()),
    }
}

/// The root identifier and static property names of `a.b['c']`.
fn static_chain(expr: &Expr) -> Option<(&Ident, Vec<String>)> {
    match unparen(expr) {
        Expr::Ident(ident) => Some((ident, Vec::new())),
        Expr::Member(member) => member_chain(member),
        _ => None,
    }
}

fn member_chain(member: &MemberExpr) -> Option<(&Ident, Vec<String>)> {
    let (root, mut parts) = static_chain(&member.obj)?;
    parts.push(member_prop_name(&member.prop)?);
    Some((root, parts))
}

/// The root identifier of a member chain, computed keys included.
fn chain_root(expr: &Expr) -> Option<&Ident> {
    match unparen(expr) {
        Expr::Ident(ident) => Some(ident),
        Expr::Member(member) => chain_root(&member.obj),
        Expr::OptChain(chain) => match chain.base.as_ref() {
            OptChainBase::Member(member) => chain_root(&member.obj),
            OptChainBase::Call(call) => chain_root(&call.callee),
        },
        _ => None,
    }
}

fn api_path(parts: &[String]) -> String {
    let mut path = String::from("NativeApi");
    for part in parts {
        path.push('.');
        path.push_str(part);
    }
    path
}

struct Model<'a> {
    scope: &'a Scope,
    facts: &'a [Facts],
    by_name: &'a BTreeMap<Name, Vec<usize>>,
    declared: &'a BTreeSet<Name>,
    members: &'a BTreeMap<String, Vec<(usize, Env)>>,
    hosts: &'a Hosts,
    families: &'a Families,
}

impl Model<'_> {
    fn targets(&self, name: &Name) -> Result<&[usize], String> {
        if self.scope.inert_imports.contains(name) {
            return Ok(&[]);
        }
        if let Some(targets) = self.by_name.get(name) {
            return Ok(targets);
        }
        if self.declared.contains(name) {
            return Ok(&[]);
        }
        Err(format!("the unresolved name {}", name.0))
    }

    fn api_targets(&self, path: &str) -> Result<Vec<&(usize, Env)>, String> {
        let found = self
            .members
            .iter()
            .filter(|(member, _)| {
                *member == path
                    || member.starts_with(&format!("{path}."))
                    || path.starts_with(&format!("{member}."))
            })
            .flat_map(|(_, functions)| functions)
            .collect::<Vec<_>>();
        if found.is_empty() {
            return Err(format!("{path} has no definition"));
        }
        Ok(found)
    }

    fn reaches_compat(&self, start: &(usize, Env)) -> Result<bool, String> {
        let mut pending = VecDeque::from([start.clone()]);
        let mut visited = BTreeSet::new();
        while let Some((index, env)) = pending.pop_front() {
            if !visited.insert((index, env.clone())) {
                continue;
            }
            let facts = &self.facts[index];
            if let Some(error) = facts.errors.first() {
                return Err(error.clone());
            }
            for host in &facts.hosts {
                if self.hosts.is_compat(host)? {
                    return Ok(true);
                }
            }
            for (starter, arg) in &facts.starts {
                let family = arg
                    .as_ref()
                    .and_then(|arg| resolve(arg, &env))
                    .ok_or_else(|| {
                        format!(
                            "the family argument of {starter} does not resolve to a string literal"
                        )
                    })?;
                if self.families.is_compat(&family)? {
                    return Ok(true);
                }
            }
            for (name, args) in &facts.calls {
                for &target in self.targets(name)? {
                    pending.push_back((target, bind(&self.facts[target].params, args, &env)));
                }
            }
            for name in &facts.refs {
                for &target in self.targets(name)? {
                    pending.push_back((target, env.clone()));
                }
            }
            for (path, args) in &facts.api_uses {
                for (target, captured) in self.api_targets(path)? {
                    let mut inner = captured.clone();
                    inner.extend(env.clone());
                    pending.push_back((*target, bind(&self.facts[*target].params, args, &inner)));
                }
            }
        }
        Ok(false)
    }
}

fn resolve(arg: &Arg, env: &Env) -> Option<String> {
    match arg {
        Arg::Str(value) => Some(value.clone()),
        Arg::Name(name) => env.get(name).cloned(),
        Arg::Other => None,
    }
}

fn bind(params: &[Option<Name>], args: &[Arg], env: &Env) -> Env {
    let mut bound = env.clone();
    for (param, arg) in params.iter().zip(args) {
        if let (Some(param), Some(value)) = (param, resolve(arg, env)) {
            bound.insert(param.clone(), value);
        }
    }
    bound
}

fn derive_members(bootstrap: &str, rust: &RustModel) -> Result<BTreeSet<String>, String> {
    let specifier = deno_ast::ModuleSpecifier::parse("file:///native-v2-main.js")
        .map_err(|error| format!("bootstrap specifier: {error}"))?;
    let parsed = deno_ast::parse_module(deno_ast::ParseParams {
        specifier,
        text: bootstrap.to_string().into(),
        media_type: deno_ast::MediaType::JavaScript,
        capture_tokens: false,
        scope_analysis: true,
        maybe_syntax: None,
    })
    .map_err(|error| format!("parse NATIVE_V2_MAIN: {error}"))?;
    let ProgramRef::Module(module) = parsed.program_ref() else {
        return Err("NATIVE_V2_MAIN did not parse as a module".to_string());
    };
    let scope = module_scope(module, parsed.unresolved_context().as_u32())?;

    let mut facts = Vec::new();
    let mut bindings = BindingCollector {
        scope: &scope,
        facts: &mut facts,
        by_name: BTreeMap::new(),
        declared: BTreeSet::new(),
    };
    module.visit_with(&mut bindings);
    let BindingCollector {
        by_name, declared, ..
    } = bindings;

    // Bindings whose value can be code: functions, host references, and
    // values that hold either.
    let mut code_names = BTreeSet::new();
    loop {
        let before = code_names.len();
        for (name, indexes) in &by_name {
            if !code_names.contains(name)
                && indexes.iter().any(|&index| {
                    let value = &facts[index];
                    value.functions
                        || !value.hosts.is_empty()
                        || value.refs.iter().any(|name| code_names.contains(name))
                })
            {
                code_names.insert(name.clone());
            }
        }
        if code_names.len() == before {
            break;
        }
    }
    let reviewed_texts = API_VALUE_USES.iter().copied().chain(
        REVIEWED_PROPERTY_WRITES
            .iter()
            .chain(REVIEWED_CODE_ARGUMENT_CALLS)
            .map(|(_, text)| *text),
    );
    for text in reviewed_texts {
        if !bootstrap.contains(text) {
            return Err(format!(
                "cannot classify: the reviewed bootstrap text `{text}` is missing"
            ));
        }
    }

    let mut api = ApiCollector {
        scope: &scope,
        source: &parsed,
        facts: &mut facts,
        by_name: &by_name,
        code_names: &code_names,
        members: BTreeMap::new(),
        setters: BTreeSet::new(),
        generated: BTreeSet::new(),
        enclosing: Vec::new(),
        errors: Vec::new(),
    };
    module.visit_with(&mut api);
    if !api.errors.is_empty() {
        return Err(api
            .errors
            .iter()
            .map(|error| format!("cannot classify {error}"))
            .collect::<Vec<_>>()
            .join("\n"));
    }
    let members = api.members;

    let model = Model {
        scope: &scope,
        facts: &facts,
        by_name: &by_name,
        declared: &declared,
        members: &members,
        hosts: &rust.hosts,
        families: &rust.families,
    };
    let mut found = BTreeSet::new();
    for (path, functions) in &members {
        for function in functions {
            let compat = model
                .reaches_compat(function)
                .map_err(|error| format!("cannot classify {path}: {error}"))?;
            if compat {
                found.insert(path.clone());
                break;
            }
        }
    }
    Ok(found)
}

// Rust side: `syn` scans of host registrations and machine family tables.

fn real_sources() -> Vec<(String, String)> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut sources = Vec::new();
    let mut directories = vec![root.clone()];
    while let Some(directory) = directories.pop() {
        let entries = fs::read_dir(&directory)
            .unwrap_or_else(|error| panic!("read {}: {error}", directory.display()));
        for entry in entries {
            let path = entry
                .unwrap_or_else(|error| panic!("read {}: {error}", directory.display()))
                .path();
            if path.is_dir() {
                directories.push(path);
            } else if path.extension().is_some_and(|extension| extension == "rs") {
                let relative = path
                    .strip_prefix(&root)
                    .expect("source is under src")
                    .components()
                    .map(|part| part.as_os_str().to_string_lossy())
                    .collect::<Vec<_>>()
                    .join("/");
                let source = fs::read_to_string(&path)
                    .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
                sources.push((relative, source));
            }
        }
    }
    sources.sort();
    sources
}

fn source<'a>(sources: &'a [(String, String)], path: &str) -> Result<&'a str, String> {
    sources
        .iter()
        .find(|(file, _)| file == path)
        .map(|(_, source)| source.as_str())
        .ok_or_else(|| format!("cannot classify: {path} is missing"))
}

fn native_v2_main(sources: &[(String, String)]) -> Result<String, String> {
    let bindings = parse_rust(BINDINGS_PATH, source(sources, BINDINGS_PATH)?)?;
    top_const(&bindings, BINDINGS_PATH, "NATIVE_V2_MAIN")
        .ok()
        .and_then(lit_str)
        .ok_or_else(|| "cannot classify: NATIVE_V2_MAIN is not a string constant".to_string())
}

/// The Rust facts that the scanner needs: host registrations and families.
struct RustModel {
    hosts: Hosts,
    families: Families,
}

impl RustModel {
    fn from_sources(sources: &[(String, String)]) -> Result<Self, String> {
        Ok(Self {
            hosts: Hosts::from_sources(sources)?,
            families: Families::from_sources(sources)?,
        })
    }
}

fn parse_rust(path: &str, source: &str) -> Result<syn::File, String> {
    syn::parse_file(source).map_err(|error| format!("cannot classify {path}: {error}"))
}

fn top_const<'a>(file: &'a syn::File, path: &str, name: &str) -> Result<&'a syn::Expr, String> {
    file.items
        .iter()
        .find_map(|item| match item {
            syn::Item::Const(constant) if constant.ident == name => Some(constant.expr.as_ref()),
            _ => None,
        })
        .ok_or_else(|| format!("cannot classify: {path} has no const {name}"))
}

fn lit_str(expr: &syn::Expr) -> Option<String> {
    match strip_expr(expr) {
        syn::Expr::Lit(syn::ExprLit {
            lit: syn::Lit::Str(value),
            ..
        }) => Some(value.value()),
        _ => None,
    }
}

fn host_lit(expr: &syn::Expr) -> Option<String> {
    lit_str(expr).filter(|name| name.starts_with(HOST_PREFIX))
}

/// The expression under parentheses, `&` and a trailing `.into()`.
fn strip_expr(expr: &syn::Expr) -> &syn::Expr {
    match expr {
        syn::Expr::Paren(inner) => strip_expr(&inner.expr),
        syn::Expr::Group(inner) => strip_expr(&inner.expr),
        syn::Expr::Reference(inner) => strip_expr(&inner.expr),
        syn::Expr::MethodCall(call) if call.method == "into" && call.args.is_empty() => {
            strip_expr(&call.receiver)
        }
        _ => expr,
    }
}

fn local_ident(expr: &syn::Expr) -> Option<String> {
    match strip_expr(expr) {
        syn::Expr::Path(path) => path.path.get_ident().map(ToString::to_string),
        _ => None,
    }
}

fn local_name(pat: &syn::Pat) -> Option<String> {
    match pat {
        syn::Pat::Ident(binding) => Some(binding.ident.to_string()),
        syn::Pat::Type(typed) => local_name(&typed.pat),
        _ => None,
    }
}

fn path_idents(path: &syn::Path) -> Vec<String> {
    path.segments
        .iter()
        .map(|segment| segment.ident.to_string())
        .collect()
}

/// Whether a node writes a path that starts with `crate::compat`, in code, in
/// a `use` item or in macro tokens.
#[derive(Default)]
struct CompatPath(bool);

impl<'ast> SynVisit<'ast> for CompatPath {
    fn visit_path(&mut self, path: &'ast syn::Path) {
        let mut idents = path.segments.iter().map(|segment| &segment.ident);
        if idents.next().is_some_and(|ident| ident == "crate")
            && idents.next().is_some_and(|ident| ident == "compat")
        {
            self.0 = true;
        }
        syn::visit::visit_path(self, path);
    }

    fn visit_use_path(&mut self, path: &'ast syn::UsePath) {
        if path.ident == "crate"
            && matches!(path.tree.as_ref(), syn::UseTree::Path(inner) if inner.ident == "compat")
        {
            self.0 = true;
        }
        syn::visit::visit_use_path(self, path);
    }

    fn visit_macro(&mut self, mac: &'ast syn::Macro) {
        if mac
            .tokens
            .to_string()
            .replace(' ', "")
            .contains("crate::compat::")
        {
            self.0 = true;
        }
        syn::visit::visit_macro(self, mac);
    }
}

fn names_compat(visit: impl FnOnce(&mut CompatPath)) -> bool {
    let mut compat = CompatPath::default();
    visit(&mut compat);
    compat.0
}

/// Host functions by name, with one compat verdict per registration found.
struct Hosts {
    registrations: BTreeMap<String, Vec<Result<bool, String>>>,
    js_values: BTreeSet<String>,
}

impl Hosts {
    fn from_sources(sources: &[(String, String)]) -> Result<Self, String> {
        let mut registrations = BTreeMap::new();
        for (path, source) in sources {
            if !source.contains("\"__rs2b0t_") {
                continue;
            }
            let file = parse_rust(path, source)?;
            let mut functions = LocalFns::default();
            functions.visit_file(&file);
            let mut scan = Registrations {
                path,
                functions: &functions.0,
                found: &mut registrations,
            };
            scan.visit_file(&file);
        }
        let mut js_values = BTreeSet::new();
        for (name, definition) in REVIEWED_JS_HOST_VALUES {
            if !sources
                .iter()
                .any(|(_, source)| source.contains(definition))
            {
                return Err(format!(
                    "cannot classify the reviewed host value {name}: `{definition}` is missing"
                ));
            }
            js_values.insert(name.to_string());
        }
        Ok(Self {
            registrations,
            js_values,
        })
    }

    fn is_compat(&self, name: &str) -> Result<bool, String> {
        if name == MACHINE_START {
            return Err(format!(
                "{MACHINE_START} is referenced outside a direct call"
            ));
        }
        if let Some(registrations) = self.registrations.get(name) {
            let mut compat = false;
            for registration in registrations {
                compat |= registration.clone()?;
            }
            return Ok(compat);
        }
        if self.js_values.contains(name) {
            return Ok(false);
        }
        Err(format!("the host function {name} has no registration"))
    }
}

/// Every free function of one file by name, with whether its body names
/// `crate::compat`.
#[derive(Default)]
struct LocalFns(BTreeMap<String, Vec<bool>>);

impl<'ast> SynVisit<'ast> for LocalFns {
    fn visit_item_fn(&mut self, function: &'ast syn::ItemFn) {
        let compat = names_compat(|visitor| visitor.visit_block(&function.block));
        self.0
            .entry(function.sig.ident.to_string())
            .or_default()
            .push(compat);
        syn::visit::visit_item_fn(self, function);
    }
}

/// Recognizes `register_function("__rs2b0t_x", closure)`,
/// `…::install(runtime, "__rs2b0t_x", callback)`, and the V8 global
/// `global.set(scope, name.into(), function.into())` where `name` is bound to
/// `v8::String::new(scope, "__rs2b0t_x")` and `function` to
/// `v8::Function::new(scope, callback)` in the same block.
struct Registrations<'a> {
    path: &'a str,
    functions: &'a BTreeMap<String, Vec<bool>>,
    found: &'a mut BTreeMap<String, Vec<Result<bool, String>>>,
}

impl Registrations<'_> {
    fn push(&mut self, name: String, verdict: Result<bool, String>) {
        self.found.entry(name).or_default().push(verdict);
    }

    fn callback(&self, name: &str, callback: &syn::Expr) -> Result<bool, String> {
        match strip_expr(callback) {
            syn::Expr::Closure(closure) => {
                Ok(names_compat(|visitor| visitor.visit_expr_closure(closure)))
            }
            syn::Expr::Path(function) => {
                let ident = function.path.get_ident().ok_or_else(|| {
                    format!("{}: the callback of {name} is not a local fn", self.path)
                })?;
                self.functions
                    .get(&ident.to_string())
                    .map(|verdicts| verdicts.contains(&true))
                    .ok_or_else(|| {
                        format!(
                            "{}: the callback {ident} of {name} is not a local fn",
                            self.path
                        )
                    })
            }
            _ => Err(format!("{}: cannot read the callback of {name}", self.path)),
        }
    }

    fn v8_globals(&mut self, block: &syn::Block) {
        let mut strings = BTreeMap::new();
        let mut functions = BTreeMap::new();
        for statement in &block.stmts {
            let syn::Stmt::Local(local) = statement else {
                continue;
            };
            let (Some(binding), Some(init)) = (local_name(&local.pat), &local.init) else {
                continue;
            };
            let mut calls = V8Calls::default();
            calls.visit_expr(&init.expr);
            if let Some(host) = calls.string {
                strings.insert(binding.clone(), host);
            }
            if let Some(callback) = calls.function {
                functions.insert(binding, callback);
            }
        }
        if strings.is_empty() {
            return;
        }
        let mut sets = SetCalls::default();
        for statement in &block.stmts {
            sets.visit_stmt(statement);
        }
        for (name, function) in sets.0 {
            let Some(host) = name.and_then(|name| strings.get(&name)) else {
                continue;
            };
            let verdict = match function.and_then(|function| functions.get(&function)) {
                Some(callback) => self.callback(host, callback),
                None => Err(format!(
                    "{}: cannot read the function set as {host}",
                    self.path
                )),
            };
            self.push(host.clone(), verdict);
        }
    }
}

impl<'ast> SynVisit<'ast> for Registrations<'_> {
    fn visit_expr_method_call(&mut self, call: &'ast syn::ExprMethodCall) {
        if call.method == "register_function" {
            if let Some(name) = call.args.first().and_then(host_lit) {
                let compat = names_compat(|visitor| {
                    call.args.iter().for_each(|arg| visitor.visit_expr(arg))
                });
                self.push(name, Ok(compat));
            }
        }
        syn::visit::visit_expr_method_call(self, call);
    }

    fn visit_expr_call(&mut self, call: &'ast syn::ExprCall) {
        if let syn::Expr::Path(function) = call.func.as_ref() {
            let install = function
                .path
                .segments
                .last()
                .is_some_and(|segment| segment.ident == "install");
            if let (true, [_, name, callback]) =
                (install, call.args.iter().collect::<Vec<_>>().as_slice())
            {
                if let Some(name) = host_lit(name) {
                    let verdict = self.callback(&name, callback);
                    self.push(name, verdict);
                }
            }
        }
        syn::visit::visit_expr_call(self, call);
    }

    fn visit_block(&mut self, block: &'ast syn::Block) {
        self.v8_globals(block);
        syn::visit::visit_block(self, block);
    }
}

/// `v8::String::new(scope, "__rs2b0t_x")` and `v8::Function::new(scope, f)`
/// inside one initializer.
#[derive(Default)]
struct V8Calls {
    string: Option<String>,
    function: Option<syn::Expr>,
}

impl<'ast> SynVisit<'ast> for V8Calls {
    fn visit_expr_call(&mut self, call: &'ast syn::ExprCall) {
        if let syn::Expr::Path(function) = call.func.as_ref() {
            let idents = path_idents(&function.path);
            let args = call.args.iter().collect::<Vec<_>>();
            match (idents.as_slice(), args.as_slice()) {
                ([.., kind, new], [_, name]) if kind == "String" && new == "new" => {
                    if let Some(host) = host_lit(name) {
                        self.string = Some(host);
                    }
                }
                ([.., kind, new], [_, callback]) if kind == "Function" && new == "new" => {
                    self.function = Some((*callback).clone());
                }
                _ => {}
            }
        }
        syn::visit::visit_expr_call(self, call);
    }
}

/// The local names passed as the key and the value of each `.set(_, k, v)`.
#[derive(Default)]
struct SetCalls(Vec<(Option<String>, Option<String>)>);

impl<'ast> SynVisit<'ast> for SetCalls {
    fn visit_expr_method_call(&mut self, call: &'ast syn::ExprMethodCall) {
        if call.method == "set" && call.args.len() == 3 {
            self.0
                .push((local_ident(&call.args[1]), local_ident(&call.args[2])));
        }
        syn::visit::visit_expr_method_call(self, call);
    }
}

/// Machine family names that the start path accepts.
#[derive(Default)]
struct Families {
    compat: BTreeSet<String>,
    native: BTreeSet<String>,
}

impl Families {
    fn from_sources(sources: &[(String, String)]) -> Result<Self, String> {
        let machine = parse_rust(MACHINE_PATH, source(sources, MACHINE_PATH)?)?;
        let mut families = Families::default();
        // The start path tries the shop family and the bank descriptors before
        // FAMILIES, so those names are native whatever FAMILIES holds.
        let shop = top_const(&machine, MACHINE_PATH, "SHOP_BUY_BY_ID_FAMILY")?;
        families.native.insert(
            lit_str(shop)
                .ok_or("cannot classify: SHOP_BUY_BY_ID_FAMILY is not a string literal")?,
        );
        let descriptors = top_const(&machine, MACHINE_PATH, "BANK_FAMILY_DESCRIPTORS")?;
        for descriptor in array_elements(descriptors)? {
            families.native.insert(bank_descriptor_name(descriptor)?);
        }
        let mut parsed = BTreeMap::new();
        let mut names = BTreeSet::new();
        for entry in array_elements(top_const(&machine, MACHINE_PATH, "FAMILIES")?)? {
            let Some(entry) = family_entry(entry)? else {
                continue;
            };
            let (name, compat) = family_name(sources, &mut parsed, entry)?;
            if !names.insert(name.clone()) {
                return Err(format!(
                    "cannot classify FAMILIES: duplicate family name {name:?}"
                ));
            }
            if families.native.contains(&name) {
                continue;
            }
            if compat {
                families.compat.insert(name);
            } else {
                families.native.insert(name);
            }
        }
        Ok(families)
    }

    fn is_compat(&self, family: &str) -> Result<bool, String> {
        if self.native.contains(family) {
            Ok(false)
        } else if self.compat.contains(family) {
            Ok(true)
        } else {
            Err(format!("unknown machine family {family:?}"))
        }
    }
}

fn array_elements(expr: &syn::Expr) -> Result<Vec<&syn::Expr>, String> {
    match expr {
        syn::Expr::Reference(reference) => array_elements(&reference.expr),
        syn::Expr::Array(array) => Ok(array.elems.iter().collect()),
        _ => Err("cannot classify a machine table that is not an array literal".into()),
    }
}

fn bank_descriptor_name(expr: &syn::Expr) -> Result<String, String> {
    let syn::Expr::Struct(descriptor) = expr else {
        return Err("cannot classify a bank descriptor that is not a struct literal".into());
    };
    descriptor
        .fields
        .iter()
        .find(|field| matches!(&field.member, syn::Member::Named(ident) if ident == "name"))
        .and_then(|field| lit_str(&field.expr))
        .ok_or_else(|| "cannot classify a bank descriptor without a literal name".into())
}

/// The type of one `entry::<T>()` element; `None` for a `#[cfg(test)]` entry,
/// which is not in the library this test links.
fn family_entry(expr: &syn::Expr) -> Result<Option<&syn::Path>, String> {
    let syn::Expr::Call(call) = expr else {
        return Err("cannot classify a FAMILIES element that is not an entry call".into());
    };
    let mut test_only = false;
    for attribute in &call.attrs {
        let cfg_test = attribute.path().is_ident("cfg")
            && attribute
                .meta
                .require_list()
                .is_ok_and(|list| list.tokens.to_string() == "test");
        if !cfg_test {
            return Err("cannot classify a FAMILIES attribute other than #[cfg(test)]".into());
        }
        test_only = true;
    }
    let entry = match (call.func.as_ref(), call.args.is_empty()) {
        (syn::Expr::Path(function), true) => function.path.segments.last(),
        _ => None,
    };
    let Some(syn::PathSegment {
        ident,
        arguments: syn::PathArguments::AngleBracketed(arguments),
    }) = entry
    else {
        return Err("cannot classify a FAMILIES element that is not entry::<T>()".into());
    };
    let [syn::GenericArgument::Type(syn::Type::Path(family))] =
        arguments.args.iter().collect::<Vec<_>>().as_slice()
    else {
        return Err("cannot classify a FAMILIES element that is not entry::<T>()".into());
    };
    if ident != "entry" || family.qself.is_some() {
        return Err("cannot classify a FAMILIES element that is not entry::<T>()".into());
    }
    Ok((!test_only).then_some(&family.path))
}

/// The `NAME` of a FAMILIES entry, and whether the entry is a `crate::compat`
/// type. `NAME` is read from the module that the entry path names.
fn family_name(
    sources: &[(String, String)],
    parsed: &mut BTreeMap<String, syn::File>,
    entry: &syn::Path,
) -> Result<(String, bool), String> {
    let idents = path_idents(entry);
    let display = idents.join("::");
    let compat = idents.starts_with(&["crate".to_string(), "compat".to_string()]);
    let last = entry
        .segments
        .last()
        .ok_or_else(|| format!("cannot classify the family entry {display}"))?;
    let (trait_name, type_path) = match &last.arguments {
        syn::PathArguments::None => ("Family", entry),
        syn::PathArguments::AngleBracketed(arguments)
            if idents == ["crate", "compat", "hunt", "Hunt"] =>
        {
            match arguments.args.iter().collect::<Vec<_>>().as_slice() {
                [syn::GenericArgument::Type(syn::Type::Path(kind))] if kind.qself.is_none() => {
                    ("HuntKind", &kind.path)
                }
                _ => return Err(format!("cannot classify the family entry {display}")),
            }
        }
        _ => {
            return Err(format!(
                "cannot classify the generic family entry {display}"
            ))
        }
    };
    if type_path
        .segments
        .iter()
        .any(|segment| !segment.arguments.is_none())
    {
        return Err(format!(
            "cannot classify the generic family entry {display}"
        ));
    }
    let segments = path_idents(type_path);
    let [first, module @ .., type_name] = segments.as_slice() else {
        return Err(format!(
            "cannot classify the family entry {display}: it is not a crate path"
        ));
    };
    if first != "crate" || module.is_empty() {
        return Err(format!(
            "cannot classify the family entry {display}: it is not a crate path"
        ));
    }
    let base = module.join("/");
    let mut names = Vec::new();
    for (file, source) in sources {
        if *file != format!("{base}.rs")
            && *file != format!("{base}/mod.rs")
            && !file.starts_with(&format!("{base}/"))
        {
            continue;
        }
        if !parsed.contains_key(file) {
            parsed.insert(file.clone(), parse_rust(file, source)?);
        }
        let mut finder = ImplNames {
            trait_name,
            type_name,
            names: Vec::new(),
        };
        finder.visit_file(&parsed[file]);
        for name in finder.names {
            names.push(name.map_err(|error| format!("cannot classify {file}: {error}"))?);
        }
    }
    match names.as_slice() {
        [name] => Ok((name.clone(), compat)),
        [] => Err(format!(
            "cannot classify the family entry {display}: no `impl {trait_name} for {type_name}`"
        )),
        _ => Err(format!(
            "cannot classify the family entry {display}: several `impl {trait_name} for {type_name}`"
        )),
    }
}

/// The `NAME` of each `impl <trait> for <type>` in one file.
struct ImplNames<'a> {
    trait_name: &'a str,
    type_name: &'a str,
    names: Vec<Result<String, String>>,
}

impl<'ast> SynVisit<'ast> for ImplNames<'_> {
    fn visit_item_impl(&mut self, item: &'ast syn::ItemImpl) {
        let trait_matches = item
            .trait_
            .as_ref()
            .and_then(|(_, path, _)| path.segments.last())
            .is_some_and(|segment| segment.ident == self.trait_name);
        let type_matches = matches!(
            item.self_ty.as_ref(),
            syn::Type::Path(ty) if ty.qself.is_none() && ty.path.is_ident(self.type_name)
        );
        if trait_matches && type_matches {
            let marker = format!("impl {} for {}", self.trait_name, self.type_name);
            let constant = item.items.iter().find_map(|member| match member {
                syn::ImplItem::Const(constant) if constant.ident == "NAME" => Some(constant),
                _ => None,
            });
            self.names.push(match constant {
                None => Err(format!("`{marker}` has no NAME")),
                Some(constant) => lit_str(&constant.expr)
                    .ok_or_else(|| format!("`{marker}` has a non-literal NAME")),
            });
        }
        syn::visit::visit_item_impl(self, item);
    }
}
