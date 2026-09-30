//! TypeScript declarations for the **compat** (JS API v1) surface.
//!
//! Declarations are emitted from the frozen rs2b0t typed source by
//! `compat-js/generate.cjs`, with named host divergences in `overlay*.json`.
//! The checked-in `index.d.ts` is the overlay-applied product, never edited
//! by hand. This module gates it against live shim names, arity and async
//! returns, and exports relative declaration trees for catalog consumers.
//!
//! O-SCRIPT-API can append a typed sidecar `.d.ts` under `compat-js/` and
//! add its barrel export in `@rs2b0t/api`; the extension gate remains active.

use crate::shim::{shim_modules, PRELUDE};
use deno_ast::swc::ast::*;
use deno_ast::{MediaType, ParseParams, ParsedSource, ProgramRef};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct CompatSurface {
    pub modules: Vec<ShimModule>,
    pub prelude: Vec<CompatExport>,
}

#[derive(Debug, Clone)]
pub struct ShimModule {
    pub specifier: String,
    pub exports: Vec<CompatExport>,
    /// `export default Name` when the default is an already-exported binding
    /// (`export default Navigator`), not `export default class`.
    pub default_export: Option<String>,
    /// Unexported local classes that exported classes `extends` (e.g. HuntTask).
    pub privates: Vec<CompatExport>,
}

#[derive(Debug, Clone)]
pub enum CompatExport {
    Class {
        name: String,
        default: bool,
        super_name: Option<String>,
        members: Vec<Member>,
    },
    Object {
        name: String,
        members: Vec<Member>,
    },
    Function {
        name: String,
        params: Vec<FnParam>,
        is_async: bool,
    },
    Value {
        name: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemberKind {
    Method,
    Getter,
    Setter,
    Field,
    Constructor,
}

#[derive(Debug, Clone)]
pub struct Member {
    pub name: String,
    pub kind: MemberKind,
    pub params: Vec<FnParam>,
    pub is_async: bool,
    pub is_static: bool,
}

#[derive(Debug, Clone)]
pub struct FnParam {
    pub name: String,
    pub optional: bool,
    pub rest: bool,
}

/// `crates/script/compat-js/index.d.ts`
pub fn compat_dts_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("compat-js/index.d.ts")
}

/// Write one `.d.ts` per authored module under `root`, mirroring `/rs2b0t/bot/`.
/// Catalog scripts import relative `../../api/game/Game.js`; TypeScript
/// resolves those to files, not ambient wildcards. Extra O-SCRIPT-API
/// modules are included when they appear in the authored surface.
pub fn write_compat_dts_tree(root: &Path) -> Result<(), String> {
    write_authored_dts_tree(&load_authored_dts()?, root)
}

/// Specifier + source for every isolate shim module (barrels included).
pub fn shim_source_pairs() -> Vec<(String, String)> {
    shim_modules()
        .into_iter()
        .map(|m| {
            (
                m.filename().to_string_lossy().into_owned(),
                m.contents().to_string(),
            )
        })
        .collect()
}

/// Walk the isolate's shim modules and prelude.
pub fn collect_compat_surface() -> CompatSurface {
    let prelude = parse_prelude(PRELUDE);
    let mut modules = Vec::new();
    for m in shim_modules() {
        let specifier = m.filename().to_string_lossy().into_owned();
        if is_barrel(&specifier) {
            continue;
        }
        let parsed = parse_module_source(&specifier, m.contents());
        modules.push(parsed);
    }
    resolve_aliases(&mut modules, &prelude);
    CompatSurface { modules, prelude }
}

/// Build a surface from explicit (specifier, source) pairs plus the live
/// prelude. Used by the drift-gate proof so a method can be added to a
/// shim source without touching the worktree files.
pub fn collect_compat_surface_from(sources: &[(&str, &str)]) -> CompatSurface {
    let prelude = parse_prelude(PRELUDE);
    let mut modules = Vec::new();
    for (specifier, source) in sources {
        if is_barrel(specifier) {
            continue;
        }
        modules.push(parse_module_source(specifier, source));
    }
    resolve_aliases(&mut modules, &prelude);
    CompatSurface { modules, prelude }
}

fn is_barrel(specifier: &str) -> bool {
    specifier.ends_with("declared_surface.js") || specifier.ends_with("rs2b0t-api.js")
}

fn parse_js(specifier: &str, source: &str, media: MediaType) -> Option<ParsedSource> {
    let url = deno_ast::ModuleSpecifier::parse(&format!("file://{specifier}")).ok()?;
    deno_ast::parse_program(ParseParams {
        specifier: url,
        text: source.into(),
        media_type: media,
        capture_tokens: false,
        scope_analysis: false,
        maybe_syntax: None,
    })
    .ok()
}

pub fn parse_module_source(specifier: &str, source: &str) -> ShimModule {
    let Some(parsed) = parse_js(specifier, source, MediaType::JavaScript) else {
        return ShimModule {
            specifier: specifier.to_string(),
            exports: Vec::new(),
            default_export: None,
            privates: Vec::new(),
        };
    };
    let mut state = ParseState::new(specifier);
    match parsed.program_ref() {
        ProgramRef::Module(module) => {
            for item in &module.body {
                match item {
                    ModuleItem::ModuleDecl(decl) => collect_module_decl(&mut state, decl),
                    ModuleItem::Stmt(stmt) => collect_local_stmt(&mut state, stmt),
                }
            }
        }
        ProgramRef::Script(script) => {
            for stmt in &script.body {
                collect_script_stmt(stmt, &mut state.exports);
            }
        }
    }
    let exports = merge_reexport_placeholders(state.exports, state.pending);
    let privates = collect_private_supers(&exports, &state.locals);
    let mut module = ShimModule {
        specifier: specifier.to_string(),
        exports,
        default_export: state.default_export,
        privates,
    };
    infer_promises_in_source(&mut module, source);
    module
}

fn parse_prelude(source: &str) -> Vec<CompatExport> {
    let Some(parsed) = parse_js("/prelude.js", source, MediaType::JavaScript) else {
        return Vec::new();
    };
    let mut exports = Vec::new();
    match parsed.program_ref() {
        ProgramRef::Script(script) => {
            for stmt in &script.body {
                collect_script_stmt(stmt, &mut exports);
            }
        }
        ProgramRef::Module(module) => {
            for item in &module.body {
                if let ModuleItem::Stmt(stmt) = item {
                    collect_script_stmt(stmt, &mut exports);
                }
            }
        }
    }
    exports
}

struct PendingReexport {
    exported: String,
    orig: String,
    from: String,
}

struct ParseState {
    specifier: String,
    exports: Vec<CompatExport>,
    pending: Vec<PendingReexport>,
    imports: HashMap<String, (String, String)>,
    locals: HashMap<String, CompatExport>,
    default_export: Option<String>,
}

impl ParseState {
    fn new(specifier: &str) -> Self {
        Self {
            specifier: specifier.to_string(),
            exports: Vec::new(),
            pending: Vec::new(),
            imports: HashMap::new(),
            locals: HashMap::new(),
            default_export: None,
        }
    }

    fn bind(&mut self, name: &str, exp: CompatExport) {
        self.locals.insert(name.to_string(), exp);
    }

    fn push_export(&mut self, exp: CompatExport) {
        let name = export_ident(&exp).to_string();
        self.locals.insert(name, exp.clone());
        self.exports.push(exp);
    }
}

fn collect_module_decl(state: &mut ParseState, decl: &ModuleDecl) {
    match decl {
        ModuleDecl::Import(import) => record_import(state, import),
        ModuleDecl::ExportDecl(ExportDecl { decl, .. }) => bind_export_decl(state, decl),
        ModuleDecl::ExportDefaultDecl(ExportDefaultDecl { decl, .. }) => match decl {
            DefaultDecl::Class(class) => {
                let name = class
                    .ident
                    .as_ref()
                    .map(|i| i.sym.to_string())
                    .unwrap_or_else(|| "default".to_string());
                state.default_export = Some(name.clone());
                state.push_export(class_export(&name, &class.class, true));
            }
            DefaultDecl::Fn(func) => {
                let name = func
                    .ident
                    .as_ref()
                    .map(|i| i.sym.to_string())
                    .unwrap_or_else(|| "default".to_string());
                state.default_export = Some(name.clone());
                state.push_export(fn_export(&name, &func.function));
            }
            _ => {}
        },
        ModuleDecl::ExportDefaultExpr(expr) => {
            if let Expr::Ident(id) = expr.expr.as_ref() {
                state.default_export = Some(id.sym.to_string());
            }
        }
        ModuleDecl::ExportNamed(named) => {
            let from = named.src.as_ref().map(|s| s.value.to_string());
            for spec in &named.specifiers {
                let ExportSpecifier::Named(n) = spec else {
                    continue;
                };
                if n.is_type_only {
                    continue;
                }
                let orig = export_name(&n.orig);
                let exported = n
                    .exported
                    .as_ref()
                    .map(export_name)
                    .unwrap_or_else(|| orig.clone());
                if let Some(from) = &from {
                    state.pending.push(PendingReexport {
                        exported,
                        orig,
                        from: resolve_rel(&state.specifier, from),
                    });
                    continue;
                }
                if let Some((imported_orig, imported_from)) = state.imports.get(&orig).cloned() {
                    state.pending.push(PendingReexport {
                        exported,
                        orig: imported_orig,
                        from: imported_from,
                    });
                    continue;
                }
                if state.exports.iter().any(|e| export_ident(e) == exported) {
                    continue;
                }
                if let Some(found) = state.locals.get(&orig).cloned() {
                    state.exports.push(rename_export(found, &exported));
                } else {
                    state.exports.push(CompatExport::Value { name: exported });
                }
            }
        }
        ModuleDecl::ExportAll(all) => {
            state.pending.push(PendingReexport {
                exported: "*".to_string(),
                orig: "*".to_string(),
                from: resolve_rel(&state.specifier, &all.src.value),
            });
        }
        _ => {}
    }
}

fn record_import(state: &mut ParseState, import: &ImportDecl) {
    let from = resolve_rel(&state.specifier, &import.src.value);
    for spec in &import.specifiers {
        match spec {
            ImportSpecifier::Named(n) => {
                let orig = n
                    .imported
                    .as_ref()
                    .map(export_name)
                    .unwrap_or_else(|| n.local.sym.to_string());
                state
                    .imports
                    .insert(n.local.sym.to_string(), (orig, from.clone()));
            }
            ImportSpecifier::Default(d) => {
                state.imports.insert(
                    d.local.sym.to_string(),
                    ("default".to_string(), from.clone()),
                );
            }
            ImportSpecifier::Namespace(_) => {}
        }
    }
}

fn bind_export_decl(state: &mut ParseState, decl: &Decl) {
    match decl {
        Decl::Class(class) => {
            state.push_export(class_export(&class.ident.sym, &class.class, false));
        }
        Decl::Fn(func) => {
            state.push_export(fn_export(&func.ident.sym, &func.function));
        }
        Decl::Var(var) => {
            for d in &var.decls {
                if let Some(exp) = var_export(d, &state.locals) {
                    state.push_export(exp);
                }
            }
        }
        _ => {}
    }
}

fn collect_local_stmt(state: &mut ParseState, stmt: &Stmt) {
    let Stmt::Decl(decl) = stmt else {
        return;
    };
    match decl {
        Decl::Class(class) => {
            state.bind(
                &class.ident.sym,
                class_export(&class.ident.sym, &class.class, false),
            );
        }
        Decl::Fn(func) => {
            state.bind(&func.ident.sym, fn_export(&func.ident.sym, &func.function));
        }
        Decl::Var(var) => {
            for d in &var.decls {
                if let Some(exp) = var_export(d, &state.locals) {
                    let name = export_ident(&exp).to_string();
                    state.bind(&name, exp);
                }
            }
        }
        _ => {}
    }
}

fn export_name(n: &ModuleExportName) -> String {
    match n {
        ModuleExportName::Ident(i) => i.sym.to_string(),
        ModuleExportName::Str(s) => s.value.to_string(),
    }
}

fn collect_script_stmt(stmt: &Stmt, exports: &mut Vec<CompatExport>) {
    let Stmt::Expr(expr_stmt) = stmt else {
        return;
    };
    let Expr::Assign(assign) = expr_stmt.expr.as_ref() else {
        return;
    };
    let Some(name) = globalthis_prop(&assign.left) else {
        return;
    };
    if let Some(exp) = expr_as_export(&name, &assign.right, &HashMap::new()) {
        exports.push(exp);
    }
}

fn globalthis_prop(left: &AssignTarget) -> Option<String> {
    let AssignTarget::Simple(SimpleAssignTarget::Member(member)) = left else {
        return None;
    };
    let Expr::Ident(obj) = member.obj.as_ref() else {
        return None;
    };
    if obj.sym.as_ref() != "globalThis" {
        return None;
    }
    match &member.prop {
        MemberProp::Ident(id) => Some(id.sym.to_string()),
        _ => None,
    }
}

fn var_export(
    decl: &VarDeclarator,
    locals: &HashMap<String, CompatExport>,
) -> Option<CompatExport> {
    let Pat::Ident(id) = &decl.name else {
        return None;
    };
    let name = id.id.sym.to_string();
    let init = decl.init.as_deref()?;
    expr_as_export(&name, init, locals)
}

fn expr_as_export(
    name: &str,
    expr: &Expr,
    locals: &HashMap<String, CompatExport>,
) -> Option<CompatExport> {
    match expr {
        Expr::Paren(p) => expr_as_export(name, &p.expr, locals),
        Expr::Class(class) => Some(class_export(name, &class.class, false)),
        Expr::Fn(func) => Some(fn_export(name, &func.function)),
        Expr::Arrow(arrow) => Some(CompatExport::Function {
            name: name.to_string(),
            params: finish_params(pats_to_params(&arrow.params)),
            is_async: arrow.is_async,
        }),
        Expr::New(new_expr) if is_ident(&new_expr.callee, "Proxy") => {
            Some(proxy_export(name, new_expr, locals))
        }
        Expr::New(new_expr) => Some(new_ident_export(name, new_expr, locals)),
        Expr::Call(call) if is_callee_ident(&call.callee, "proxy") => {
            let members = call
                .args
                .get(1)
                .and_then(|a| object_members(&a.expr, locals))
                .unwrap_or_default();
            if members.is_empty() {
                Some(CompatExport::Value {
                    name: name.to_string(),
                })
            } else {
                Some(CompatExport::Object {
                    name: name.to_string(),
                    members,
                })
            }
        }
        Expr::Object(obj) => Some(CompatExport::Object {
            name: name.to_string(),
            members: object_lit_members(obj, locals),
        }),
        Expr::Member(member) => {
            // `export const LoopingBot = globalThis.LoopingBot`
            if let (Expr::Ident(obj), MemberProp::Ident(prop)) = (member.obj.as_ref(), &member.prop)
            {
                if obj.sym.as_ref() == "globalThis" {
                    return Some(CompatExport::Value {
                        name: format!("__alias__{name}__{}", prop.sym),
                    });
                }
            }
            Some(CompatExport::Value {
                name: name.to_string(),
            })
        }
        _ => Some(CompatExport::Value {
            name: name.to_string(),
        }),
    }
}

fn proxy_export(
    name: &str,
    new_expr: &NewExpr,
    locals: &HashMap<String, CompatExport>,
) -> CompatExport {
    let members = new_expr
        .args
        .as_ref()
        .and_then(|a| a.first())
        .and_then(|a| object_members(&a.expr, locals))
        .unwrap_or_default();
    if members.is_empty() {
        CompatExport::Value {
            name: name.to_string(),
        }
    } else {
        CompatExport::Object {
            name: name.to_string(),
            members,
        }
    }
}

fn new_ident_export(
    name: &str,
    new_expr: &NewExpr,
    locals: &HashMap<String, CompatExport>,
) -> CompatExport {
    if let Expr::Ident(id) = new_expr.callee.as_ref() {
        if let Some(CompatExport::Class { members, .. }) = locals.get(id.sym.as_ref()) {
            let instance = members
                .iter()
                .filter(|m| !m.is_static && m.kind != MemberKind::Constructor)
                .cloned()
                .collect::<Vec<_>>();
            if !instance.is_empty() {
                return CompatExport::Object {
                    name: name.to_string(),
                    members: instance,
                };
            }
        }
    }
    CompatExport::Value {
        name: name.to_string(),
    }
}

fn is_ident(expr: &Expr, want: &str) -> bool {
    matches!(expr, Expr::Ident(id) if id.sym.as_ref() == want)
}

fn is_callee_ident(callee: &Callee, want: &str) -> bool {
    matches!(callee, Callee::Expr(expr) if is_ident(expr, want))
}

fn class_export(name: &str, class: &Class, default: bool) -> CompatExport {
    let super_name = class.super_class.as_ref().and_then(|e| match e.as_ref() {
        Expr::Ident(id) => Some(id.sym.to_string()),
        Expr::Member(m) => match (m.obj.as_ref(), &m.prop) {
            (Expr::Ident(obj), MemberProp::Ident(prop)) if obj.sym.as_ref() == "globalThis" => {
                Some(prop.sym.to_string())
            }
            _ => None,
        },
        _ => None,
    });
    CompatExport::Class {
        name: name.to_string(),
        default,
        super_name,
        members: class_members(class),
    }
}

fn fn_export(name: &str, func: &Function) -> CompatExport {
    CompatExport::Function {
        name: name.to_string(),
        params: finish_params(func.params.iter().map(|p| pat_to_param(&p.pat)).collect()),
        is_async: func.is_async,
    }
}

fn finish_params(mut params: Vec<FnParam>) -> Vec<FnParam> {
    let mut seen_optional = false;
    for p in &mut params {
        if p.rest {
            continue;
        }
        if p.optional {
            seen_optional = true;
        } else if seen_optional {
            p.optional = true;
        }
    }
    params
}

fn class_members(class: &Class) -> Vec<Member> {
    let mut out = Vec::new();
    for member in &class.body {
        match member {
            ClassMember::Constructor(ctor) => {
                let params = finish_params(
                    ctor.params
                        .iter()
                        .filter_map(|p| match p {
                            ParamOrTsParamProp::Param(param) => Some(pat_to_param(&param.pat)),
                            ParamOrTsParamProp::TsParamProp(_) => None,
                        })
                        .collect(),
                );
                out.push(Member {
                    name: "constructor".to_string(),
                    kind: MemberKind::Constructor,
                    params,
                    is_async: false,
                    is_static: false,
                });
                for field in constructor_this_fields(ctor) {
                    if !out.iter().any(|m| m.name == field.name) {
                        out.push(field);
                    }
                }
            }
            ClassMember::Method(method) => {
                let Some(name) = prop_name(&method.key) else {
                    continue;
                };
                if name.starts_with('_') {
                    continue;
                }
                let kind = match method.kind {
                    MethodKind::Getter => MemberKind::Getter,
                    MethodKind::Setter => MemberKind::Setter,
                    MethodKind::Method => MemberKind::Method,
                };
                out.push(Member {
                    name,
                    kind,
                    params: finish_params(
                        method
                            .function
                            .params
                            .iter()
                            .map(|p| pat_to_param(&p.pat))
                            .collect(),
                    ),
                    is_async: method.function.is_async,
                    is_static: method.is_static,
                });
            }
            ClassMember::ClassProp(prop) => {
                let Some(name) = prop_name(&prop.key) else {
                    continue;
                };
                if name.starts_with('_') {
                    continue;
                }
                out.push(Member {
                    name,
                    kind: MemberKind::Field,
                    params: Vec::new(),
                    is_async: false,
                    is_static: prop.is_static,
                });
            }
            _ => {}
        }
    }
    out
}

fn constructor_this_fields(ctor: &Constructor) -> Vec<Member> {
    let Some(body) = &ctor.body else {
        return Vec::new();
    };
    let mut out: Vec<Member> = Vec::new();
    for stmt in &body.stmts {
        let Stmt::Expr(expr_stmt) = stmt else {
            continue;
        };
        let Expr::Assign(assign) = expr_stmt.expr.as_ref() else {
            continue;
        };
        let AssignTarget::Simple(SimpleAssignTarget::Member(member)) = &assign.left else {
            continue;
        };
        if !matches!(member.obj.as_ref(), Expr::This(_)) {
            continue;
        }
        let name = match &member.prop {
            MemberProp::Ident(id) => id.sym.to_string(),
            _ => continue,
        };
        if name.starts_with('_') || out.iter().any(|m| m.name == name) {
            continue;
        }
        out.push(Member {
            name,
            kind: MemberKind::Field,
            params: Vec::new(),
            is_async: false,
            is_static: false,
        });
    }
    out
}

fn object_members(expr: &Expr, locals: &HashMap<String, CompatExport>) -> Option<Vec<Member>> {
    match expr {
        Expr::Object(obj) => Some(object_lit_members(obj, locals)),
        Expr::Paren(p) => object_members(&p.expr, locals),
        _ => None,
    }
}

fn object_lit_members(obj: &ObjectLit, locals: &HashMap<String, CompatExport>) -> Vec<Member> {
    let mut out = Vec::new();
    for prop in &obj.props {
        let PropOrSpread::Prop(prop) = prop else {
            continue;
        };
        match prop.as_ref() {
            Prop::Method(m) => {
                let Some(name) = prop_name(&m.key) else {
                    continue;
                };
                out.push(Member {
                    name,
                    kind: MemberKind::Method,
                    params: finish_params(
                        m.function
                            .params
                            .iter()
                            .map(|p| pat_to_param(&p.pat))
                            .collect(),
                    ),
                    is_async: m.function.is_async,
                    is_static: false,
                });
            }
            Prop::Getter(g) => {
                let Some(name) = prop_name(&g.key) else {
                    continue;
                };
                out.push(Member {
                    name,
                    kind: MemberKind::Getter,
                    params: Vec::new(),
                    is_async: false,
                    is_static: false,
                });
            }
            Prop::Setter(s) => {
                let Some(name) = prop_name(&s.key) else {
                    continue;
                };
                out.push(Member {
                    name,
                    kind: MemberKind::Setter,
                    params: vec![pat_to_param(&s.param)],
                    is_async: false,
                    is_static: false,
                });
            }
            Prop::Shorthand(id) => {
                let name = id.sym.to_string();
                if let Some(member) = member_from_local(&name, locals) {
                    out.push(member);
                } else {
                    out.push(Member {
                        name,
                        kind: MemberKind::Field,
                        params: Vec::new(),
                        is_async: false,
                        is_static: false,
                    });
                }
            }
            Prop::KeyValue(kv) => {
                let Some(name) = prop_name(&kv.key) else {
                    continue;
                };
                match kv.value.as_ref() {
                    Expr::Fn(func) => out.push(Member {
                        name,
                        kind: MemberKind::Method,
                        params: finish_params(
                            func.function
                                .params
                                .iter()
                                .map(|p| pat_to_param(&p.pat))
                                .collect(),
                        ),
                        is_async: func.function.is_async,
                        is_static: false,
                    }),
                    Expr::Arrow(arrow) => out.push(Member {
                        name,
                        kind: MemberKind::Method,
                        params: finish_params(pats_to_params(&arrow.params)),
                        is_async: arrow.is_async,
                        is_static: false,
                    }),
                    Expr::Ident(id) => {
                        if let Some(member) = member_from_local(id.sym.as_ref(), locals) {
                            out.push(Member {
                                name,
                                kind: member.kind,
                                params: member.params,
                                is_async: member.is_async,
                                is_static: false,
                            });
                        } else {
                            out.push(Member {
                                name,
                                kind: MemberKind::Field,
                                params: Vec::new(),
                                is_async: false,
                                is_static: false,
                            });
                        }
                    }
                    _ => out.push(Member {
                        name,
                        kind: MemberKind::Field,
                        params: Vec::new(),
                        is_async: false,
                        is_static: false,
                    }),
                }
            }
            _ => {}
        }
    }
    out
}

fn member_from_local(name: &str, locals: &HashMap<String, CompatExport>) -> Option<Member> {
    match locals.get(name) {
        Some(CompatExport::Function {
            params, is_async, ..
        }) => Some(Member {
            name: name.to_string(),
            kind: MemberKind::Method,
            params: params.clone(),
            is_async: *is_async,
            is_static: false,
        }),
        Some(CompatExport::Class { members, .. }) => {
            let ctor = members.iter().find(|m| m.kind == MemberKind::Constructor);
            Some(Member {
                name: name.to_string(),
                kind: MemberKind::Field,
                params: ctor.map(|c| c.params.clone()).unwrap_or_default(),
                is_async: false,
                is_static: false,
            })
        }
        Some(CompatExport::Object { .. } | CompatExport::Value { .. }) => Some(Member {
            name: name.to_string(),
            kind: MemberKind::Field,
            params: Vec::new(),
            is_async: false,
            is_static: false,
        }),
        None => None,
    }
}

fn prop_name(name: &PropName) -> Option<String> {
    match name {
        PropName::Ident(id) => Some(id.sym.to_string()),
        PropName::Str(s) => Some(s.value.to_string()),
        _ => None,
    }
}

fn pats_to_params(pats: &[Pat]) -> Vec<FnParam> {
    pats.iter().map(pat_to_param).collect()
}

fn pat_to_param(pat: &Pat) -> FnParam {
    match pat {
        Pat::Ident(id) => FnParam {
            name: sanitize_ident(&id.id.sym),
            optional: id.id.optional,
            rest: false,
        },
        Pat::Assign(assign) => {
            let mut p = pat_to_param(&assign.left);
            p.optional = true;
            p
        }
        Pat::Rest(rest) => {
            let mut p = pat_to_param(&rest.arg);
            p.rest = true;
            p
        }
        Pat::Object(_) => FnParam {
            name: "opts".to_string(),
            optional: false,
            rest: false,
        },
        Pat::Array(_) => FnParam {
            name: "items".to_string(),
            optional: false,
            rest: false,
        },
        _ => FnParam {
            name: "arg".to_string(),
            optional: false,
            rest: false,
        },
    }
}

fn sanitize_ident(raw: &str) -> String {
    if is_ts_ident(raw) {
        raw.to_string()
    } else {
        "arg".to_string()
    }
}

fn is_ts_ident(s: &str) -> bool {
    let mut chars = s.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first.is_ascii_alphabetic() || first == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn merge_reexport_placeholders(
    mut exports: Vec<CompatExport>,
    pending: Vec<PendingReexport>,
) -> Vec<CompatExport> {
    for re in pending {
        exports.push(CompatExport::Value {
            name: format!("__reexport__{}__{}__{}", re.exported, re.orig, re.from),
        });
    }
    exports
}

fn collect_private_supers(
    exports: &[CompatExport],
    locals: &HashMap<String, CompatExport>,
) -> Vec<CompatExport> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for exp in exports {
        let CompatExport::Class {
            super_name: Some(sup),
            ..
        } = exp
        else {
            continue;
        };
        if exports.iter().any(|e| export_ident(e) == sup) {
            continue;
        }
        if !seen.insert(sup.clone()) {
            continue;
        }
        if let Some(local) = locals.get(sup) {
            out.push(local.clone());
        } else {
            out.push(CompatExport::Class {
                name: sup.clone(),
                default: false,
                super_name: None,
                members: Vec::new(),
            });
        }
    }
    out
}

fn resolve_aliases(modules: &mut [ShimModule], prelude: &[CompatExport]) {
    for module in modules.iter_mut() {
        let mut next = Vec::new();
        for exp in &module.exports {
            if let CompatExport::Value { name } = exp {
                if let Some((local, target)) = name
                    .strip_prefix("__alias__")
                    .and_then(|s| s.split_once("__"))
                {
                    if let Some(found) = find_named(prelude, target) {
                        next.push(rename_export(found, local));
                    } else {
                        next.push(CompatExport::Value {
                            name: local.to_string(),
                        });
                    }
                    continue;
                }
            }
            next.push(exp.clone());
        }
        module.exports = next;
    }
    let by_spec: HashMap<String, Vec<CompatExport>> = modules
        .iter()
        .map(|m| (m.specifier.clone(), m.exports.clone()))
        .collect();
    for module in modules.iter_mut() {
        let mut next = Vec::new();
        for exp in &module.exports {
            if let CompatExport::Value { name } = exp {
                if let Some((exported, rest)) = name.strip_prefix("__reexport__").and_then(|s| {
                    let mut parts = s.splitn(3, "__");
                    let exported = parts.next()?;
                    let orig = parts.next()?;
                    let from = parts.next()?;
                    Some((exported.to_string(), (orig.to_string(), from.to_string())))
                }) {
                    let (orig, from) = rest;
                    if orig == "*" {
                        if let Some(found) = by_spec.get(&from) {
                            for exp in found {
                                if next.iter().any(|e| export_ident(e) == export_ident(exp)) {
                                    continue;
                                }
                                next.push(exp.clone());
                            }
                        }
                    } else if let Some(found) =
                        by_spec.get(&from).and_then(|exps| find_named(exps, &orig))
                    {
                        next.push(rename_export(found, &exported));
                    } else {
                        next.push(CompatExport::Value { name: exported });
                    }
                    continue;
                }
            }
            next.push(exp.clone());
        }
        module.exports = next;
    }
}

fn find_named(exports: &[CompatExport], name: &str) -> Option<CompatExport> {
    if name == "default" {
        return exports
            .iter()
            .find(|e| matches!(e, CompatExport::Class { default: true, .. }))
            .cloned();
    }
    exports.iter().find(|e| export_ident(e) == name).cloned()
}

fn export_ident(exp: &CompatExport) -> &str {
    match exp {
        CompatExport::Class { name, .. }
        | CompatExport::Object { name, .. }
        | CompatExport::Function { name, .. }
        | CompatExport::Value { name } => name,
    }
}

fn rename_export(exp: CompatExport, name: &str) -> CompatExport {
    match exp {
        CompatExport::Class {
            default,
            super_name,
            members,
            ..
        } => CompatExport::Class {
            name: name.to_string(),
            default,
            super_name,
            members,
        },
        CompatExport::Object { members, .. } => CompatExport::Object {
            name: name.to_string(),
            members,
        },
        CompatExport::Function {
            params, is_async, ..
        } => CompatExport::Function {
            name: name.to_string(),
            params,
            is_async,
        },
        CompatExport::Value { .. } => CompatExport::Value {
            name: name.to_string(),
        },
    }
}

fn resolve_rel(from_file: &str, spec: &str) -> String {
    if !spec.starts_with('.') {
        return spec.to_string();
    }
    let dir = from_file.rsplit_once('/').map(|(d, _)| d).unwrap_or("");
    let mut parts: Vec<&str> = dir.split('/').filter(|s| !s.is_empty()).collect();
    for seg in spec.split('/') {
        match seg {
            "." => {}
            ".." => {
                parts.pop();
            }
            s => parts.push(s),
        }
    }
    let mut out = String::new();
    if from_file.starts_with('/') {
        out.push('/');
    }
    out.push_str(&parts.join("/"));
    out
}

fn module_suffix(specifier: &str) -> Option<&str> {
    specifier.strip_prefix("/rs2b0t/bot/")
}

/// Every member name an export contributes to the declared surface.
pub fn member_names(exp: &CompatExport) -> Vec<&str> {
    match exp {
        CompatExport::Class { members, .. } | CompatExport::Object { members, .. } => {
            members.iter().map(|m| m.name.as_str()).collect()
        }
        CompatExport::Function { name, .. } | CompatExport::Value { name } => vec![name.as_str()],
    }
}

/// Find a named export on the collected surface (any module or prelude).
pub fn find_export<'a>(surface: &'a CompatSurface, name: &str) -> Option<&'a CompatExport> {
    surface
        .modules
        .iter()
        .flat_map(|m| m.exports.iter())
        .chain(surface.prelude.iter())
        .find(|e| export_ident(e) == name)
}

fn infer_promises_in_source(module: &mut ShimModule, source: &str) {
    let Some(parsed) = parse_js(&module.specifier, source, MediaType::JavaScript) else {
        return;
    };
    let mut facts = HashMap::new();
    seed_module_facts(module, &mut facts);
    for _ in 0..12 {
        if !walk_parsed(&parsed, &mut facts) {
            break;
        }
    }
    apply_promise_facts(module, &facts);
}

fn seed_module_facts(module: &ShimModule, facts: &mut HashMap<String, bool>) {
    for exp in module.exports.iter().chain(module.privates.iter()) {
        seed_export_facts(exp, facts);
    }
}

fn seed_export_facts(exp: &CompatExport, facts: &mut HashMap<String, bool>) {
    match exp {
        CompatExport::Function { name, is_async, .. } => {
            if *is_async {
                facts.insert(name.clone(), true);
            }
        }
        CompatExport::Object { name, members } | CompatExport::Class { name, members, .. } => {
            for m in members {
                if m.is_async && m.kind == MemberKind::Method {
                    facts.insert(format!("{name}.{}", m.name), true);
                }
            }
        }
        CompatExport::Value { .. } => {}
    }
}

fn apply_promise_facts(module: &mut ShimModule, facts: &HashMap<String, bool>) {
    for exp in module.exports.iter_mut().chain(module.privates.iter_mut()) {
        match exp {
            CompatExport::Function { name, is_async, .. } => {
                if facts.get(name).copied().unwrap_or(false) {
                    *is_async = true;
                }
            }
            CompatExport::Object { name, members } | CompatExport::Class { name, members, .. } => {
                for m in members {
                    if m.kind == MemberKind::Method
                        && facts
                            .get(&format!("{name}.{}", m.name))
                            .copied()
                            .unwrap_or(false)
                    {
                        m.is_async = true;
                    }
                }
            }
            CompatExport::Value { .. } => {}
        }
    }
}

fn walk_parsed(parsed: &ParsedSource, facts: &mut HashMap<String, bool>) -> bool {
    let mut changed = false;
    match parsed.program_ref() {
        ProgramRef::Module(module) => {
            for item in &module.body {
                changed |= walk_item(item, facts);
            }
        }
        ProgramRef::Script(script) => {
            for stmt in &script.body {
                changed |= walk_stmt(stmt, facts);
            }
        }
    }
    changed
}

fn walk_item(item: &ModuleItem, facts: &mut HashMap<String, bool>) -> bool {
    match item {
        ModuleItem::ModuleDecl(ModuleDecl::ExportDecl(ExportDecl { decl, .. })) => {
            walk_decl(decl, facts)
        }
        ModuleItem::ModuleDecl(ModuleDecl::ExportDefaultDecl(ExportDefaultDecl {
            decl, ..
        })) => match decl {
            DefaultDecl::Fn(func) => {
                let name = func
                    .ident
                    .as_ref()
                    .map(|i| i.sym.to_string())
                    .unwrap_or_else(|| "default".to_string());
                set_promise_fact(facts, name, function_returns_promise(&func.function, facts))
            }
            DefaultDecl::Class(class) => walk_class(
                class
                    .ident
                    .as_ref()
                    .map(|i| i.sym.as_ref())
                    .unwrap_or("default"),
                &class.class,
                facts,
            ),
            _ => false,
        },
        ModuleItem::Stmt(stmt) => walk_stmt(stmt, facts),
        _ => false,
    }
}

fn walk_stmt(stmt: &Stmt, facts: &mut HashMap<String, bool>) -> bool {
    match stmt {
        Stmt::Decl(decl) => walk_decl(decl, facts),
        _ => false,
    }
}

fn walk_decl(decl: &Decl, facts: &mut HashMap<String, bool>) -> bool {
    match decl {
        Decl::Fn(func) => set_promise_fact(
            facts,
            func.ident.sym.to_string(),
            function_returns_promise(&func.function, facts),
        ),
        Decl::Class(class) => walk_class(&class.ident.sym, &class.class, facts),
        Decl::Var(var) => {
            let mut changed = false;
            for d in &var.decls {
                changed |= walk_var_declarator(d, facts);
            }
            changed
        }
        _ => false,
    }
}

fn walk_class(name: &str, class: &Class, facts: &mut HashMap<String, bool>) -> bool {
    let mut changed = false;
    for member in &class.body {
        if let ClassMember::Method(method) = member {
            let Some(mname) = prop_name(&method.key) else {
                continue;
            };
            if method.kind != MethodKind::Method {
                continue;
            }
            changed |= set_promise_fact(
                facts,
                format!("{name}.{mname}"),
                function_returns_promise(&method.function, facts),
            );
        }
    }
    changed
}

fn walk_var_declarator(decl: &VarDeclarator, facts: &mut HashMap<String, bool>) -> bool {
    let Pat::Ident(id) = &decl.name else {
        return false;
    };
    let name = id.id.sym.to_string();
    let Some(init) = decl.init.as_deref() else {
        return false;
    };
    walk_binding_expr(&name, init, facts)
}

fn walk_binding_expr(name: &str, expr: &Expr, facts: &mut HashMap<String, bool>) -> bool {
    match expr {
        Expr::Paren(p) => walk_binding_expr(name, &p.expr, facts),
        Expr::Fn(func) => set_promise_fact(
            facts,
            name.to_string(),
            function_returns_promise(&func.function, facts),
        ),
        Expr::Arrow(arrow) => {
            set_promise_fact(facts, name.to_string(), arrow_returns_promise(arrow, facts))
        }
        Expr::Object(obj) => walk_object_lit(name, obj, facts),
        Expr::Call(call) if is_callee_ident(&call.callee, "proxy") => call
            .args
            .get(1)
            .map(|a| walk_binding_expr(name, &a.expr, facts))
            .unwrap_or(false),
        Expr::New(new_expr) if is_ident(&new_expr.callee, "Proxy") => new_expr
            .args
            .as_ref()
            .and_then(|a| a.first())
            .map(|a| walk_binding_expr(name, &a.expr, facts))
            .unwrap_or(false),
        _ => false,
    }
}

fn walk_object_lit(obj_name: &str, obj: &ObjectLit, facts: &mut HashMap<String, bool>) -> bool {
    let mut changed = false;
    for prop in &obj.props {
        let PropOrSpread::Prop(prop) = prop else {
            continue;
        };
        match prop.as_ref() {
            Prop::Method(m) => {
                let Some(name) = prop_name(&m.key) else {
                    continue;
                };
                changed |= set_promise_fact(
                    facts,
                    format!("{obj_name}.{name}"),
                    function_returns_promise(&m.function, facts),
                );
            }
            Prop::KeyValue(kv) => {
                let Some(name) = prop_name(&kv.key) else {
                    continue;
                };
                let key = format!("{obj_name}.{name}");
                match kv.value.as_ref() {
                    Expr::Fn(func) => {
                        changed |= set_promise_fact(
                            facts,
                            key,
                            function_returns_promise(&func.function, facts),
                        );
                    }
                    Expr::Arrow(arrow) => {
                        changed |=
                            set_promise_fact(facts, key, arrow_returns_promise(arrow, facts));
                    }
                    Expr::Ident(id) if facts.get(id.sym.as_ref()).copied().unwrap_or(false) => {
                        changed |= set_promise_fact(facts, key, true);
                    }
                    _ => {}
                }
            }
            Prop::Shorthand(id) if facts.get(id.sym.as_ref()).copied().unwrap_or(false) => {
                changed |= set_promise_fact(facts, format!("{obj_name}.{}", id.sym), true);
            }
            _ => {}
        }
    }
    changed
}

fn set_promise_fact(facts: &mut HashMap<String, bool>, key: String, val: bool) -> bool {
    if !val {
        return false;
    }
    match facts.get(&key) {
        Some(true) => false,
        _ => {
            facts.insert(key, true);
            true
        }
    }
}

fn function_returns_promise(func: &Function, facts: &HashMap<String, bool>) -> bool {
    if func.is_async {
        return true;
    }
    let Some(body) = &func.body else {
        return false;
    };
    block_returns_promise(&body.stmts, facts)
}

fn arrow_returns_promise(arrow: &ArrowExpr, facts: &HashMap<String, bool>) -> bool {
    if arrow.is_async {
        return true;
    }
    match arrow.body.as_ref() {
        BlockStmtOrExpr::Expr(expr) => expr_is_promise(expr, facts),
        BlockStmtOrExpr::BlockStmt(block) => block_returns_promise(&block.stmts, facts),
    }
}

fn block_returns_promise(stmts: &[Stmt], facts: &HashMap<String, bool>) -> bool {
    let mut returns = Vec::new();
    for stmt in stmts {
        collect_returns(stmt, &mut returns);
    }
    if returns.is_empty() {
        return false;
    }
    returns
        .iter()
        .all(|expr| expr.map(|e| expr_is_promise(e, facts)).unwrap_or(false))
}

fn collect_returns<'a>(stmt: &'a Stmt, out: &mut Vec<Option<&'a Expr>>) {
    match stmt {
        Stmt::Return(r) => out.push(r.arg.as_deref()),
        Stmt::Block(b) => {
            for s in &b.stmts {
                collect_returns(s, out);
            }
        }
        Stmt::If(i) => {
            collect_returns(&i.cons, out);
            if let Some(alt) = &i.alt {
                collect_returns(alt, out);
            }
        }
        Stmt::Try(t) => {
            for s in &t.block.stmts {
                collect_returns(s, out);
            }
            if let Some(h) = &t.handler {
                for s in &h.body.stmts {
                    collect_returns(s, out);
                }
            }
            if let Some(f) = &t.finalizer {
                for s in &f.stmts {
                    collect_returns(s, out);
                }
            }
        }
        Stmt::Switch(s) => {
            for c in &s.cases {
                for st in &c.cons {
                    collect_returns(st, out);
                }
            }
        }
        Stmt::While(w) => collect_returns(&w.body, out),
        Stmt::DoWhile(d) => collect_returns(&d.body, out),
        Stmt::For(f) => collect_returns(&f.body, out),
        Stmt::ForIn(f) => collect_returns(&f.body, out),
        Stmt::ForOf(f) => collect_returns(&f.body, out),
        Stmt::Labeled(l) => collect_returns(&l.body, out),
        _ => {}
    }
}

fn expr_is_promise(expr: &Expr, facts: &HashMap<String, bool>) -> bool {
    match expr {
        Expr::Paren(p) => expr_is_promise(&p.expr, facts),
        Expr::New(n) if is_ident(&n.callee, "Promise") => true,
        Expr::Call(c) => call_is_promise(c, facts),
        Expr::Cond(c) => expr_is_promise(&c.cons, facts) && expr_is_promise(&c.alt, facts),
        _ => false,
    }
}

fn call_is_promise(call: &CallExpr, facts: &HashMap<String, bool>) -> bool {
    match &call.callee {
        Callee::Expr(e) => match e.as_ref() {
            Expr::Ident(id) => facts.get(id.sym.as_ref()).copied().unwrap_or(false),
            Expr::Member(m) => {
                let MemberProp::Ident(prop) = &m.prop else {
                    return false;
                };
                let Expr::Ident(obj) = m.obj.as_ref() else {
                    return false;
                };
                if obj.sym.as_ref() == "Promise"
                    && matches!(prop.sym.as_ref(), "resolve" | "reject")
                {
                    return true;
                }
                facts
                    .get(&format!("{}.{}", obj.sym, prop.sym))
                    .copied()
                    .unwrap_or(false)
            }
            _ => false,
        },
        _ => false,
    }
}

/// Authored compat declarations: types live here; the shim walk gates names.
#[derive(Debug, Clone)]
pub struct AuthoredSurface {
    pub modules: Vec<AuthoredModule>,
}

#[derive(Debug, Clone)]
pub struct AuthoredModule {
    pub specifier: String,
    pub body: String,
    pub exports: Vec<CompatExport>,
    pub privates: Vec<CompatExport>,
    pub default_export: Option<String>,
    /// True when the module came from a sidecar `.d.ts` file, not `index.d.ts`.
    pub extension_file: bool,
}

pub fn authored_dts_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("compat-js")
}

pub fn load_authored_dts() -> Result<AuthoredSurface, String> {
    load_authored_dts_from(&authored_dts_dir())
}

pub fn load_authored_dts_from(dir: &Path) -> Result<AuthoredSurface, String> {
    let index = dir.join("index.d.ts");
    let src =
        std::fs::read_to_string(&index).map_err(|e| format!("read {}: {e}", index.display()))?;
    let mut modules = parse_authored_source(&src, false)?;
    let mut files = Vec::new();
    collect_dts_files(dir, &mut files)?;
    files.sort();
    for path in files {
        if path.file_name().is_some_and(|n| n == "index.d.ts") {
            continue;
        }
        let src =
            std::fs::read_to_string(&path).map_err(|e| format!("read {}: {e}", path.display()))?;
        let rel = path
            .strip_prefix(dir)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");
        let parsed = parse_authored_source(&src, true)?;
        if parsed
            .iter()
            .any(|m| m.specifier == "@rs2b0t/api" || m.specifier.starts_with('*'))
        {
            modules.extend(parsed);
            continue;
        }
        let spec = file_rel_to_specifier(&rel);
        let mut module = parse_dts_module(&spec, &src)?;
        module.body = src;
        module.extension_file = true;
        modules.push(module);
    }
    let mut aliases: Vec<ShimModule> = modules
        .iter()
        .map(|m| ShimModule {
            specifier: m.specifier.clone(),
            exports: m.exports.clone(),
            privates: m.privates.clone(),
            default_export: m.default_export.clone(),
        })
        .collect();
    resolve_aliases(&mut aliases, &[]);
    for (module, resolved) in modules.iter_mut().zip(aliases) {
        module.exports = resolved.exports;
    }
    Ok(AuthoredSurface { modules })
}

fn file_rel_to_specifier(rel: &str) -> String {
    let trimmed = rel.strip_prefix("./").unwrap_or(rel);
    let stem = trimmed.strip_suffix(".d.ts").unwrap_or(trimmed);
    format!("*{stem}.js")
}

fn collect_dts_files(dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), String> {
    let entries = std::fs::read_dir(dir).map_err(|e| format!("read_dir {}: {e}", dir.display()))?;
    for ent in entries {
        let ent = ent.map_err(|e| format!("read_dir {}: {e}", dir.display()))?;
        let path = ent.path();
        if path.is_dir() && path.file_name().is_some_and(|n| n != "node_modules") {
            collect_dts_files(&path, out)?;
        } else if path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.ends_with(".d.ts"))
        {
            out.push(path);
        }
    }
    Ok(())
}

fn parse_authored_source(src: &str, extension_file: bool) -> Result<Vec<AuthoredModule>, String> {
    let mut out = Vec::new();
    for (spec, body) in split_ambient_modules(src) {
        let mut module = parse_dts_module(&spec, &body)?;
        module.body = body.trim().to_string();
        if !module.body.is_empty() && !module.body.ends_with('\n') {
            module.body.push('\n');
        }
        module.extension_file = extension_file;
        out.push(module);
    }
    Ok(out)
}

fn split_ambient_modules(src: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < src.len() {
        let rest = &src[i..];
        let Some(rel) = rest.find("declare module ") else {
            break;
        };
        i += rel + "declare module ".len();
        i = skip_ws_bytes(src, i);
        let Some(&q) = src.as_bytes().get(i) else {
            break;
        };
        if q != b'\'' && q != b'"' {
            continue;
        }
        i += 1;
        let start = i;
        while i < src.len() && src.as_bytes()[i] != q {
            i += 1;
        }
        if i >= src.len() {
            break;
        }
        let name = src[start..i].to_string();
        i += 1;
        i = skip_ws_bytes(src, i);
        if src.as_bytes().get(i) != Some(&b'{') {
            continue;
        }
        let Some(end) = match_brace(src, i) else {
            break;
        };
        let body = src[i + 1..end].to_string();
        out.push((name, body));
        i = end + 1;
    }
    out
}

fn skip_ws_bytes(src: &str, mut i: usize) -> usize {
    while i < src.len() && src.as_bytes()[i].is_ascii_whitespace() {
        i += 1;
    }
    i
}

fn match_brace(src: &str, open: usize) -> Option<usize> {
    let bytes = src.as_bytes();
    if bytes.get(open) != Some(&b'{') {
        return None;
    }
    let mut depth = 0;
    let mut i = open;
    let mut in_str: Option<u8> = None;
    let mut in_line = false;
    let mut in_block = false;
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
            if b == b'*' && bytes.get(i + 1) == Some(&b'/') {
                in_block = false;
                i += 2;
                continue;
            }
            i += 1;
            continue;
        }
        if let Some(q) = in_str {
            if b == b'\\' {
                i += 2;
                continue;
            }
            if b == q {
                in_str = None;
            }
            i += 1;
            continue;
        }
        match b {
            b'/' if bytes.get(i + 1) == Some(&b'/') => {
                in_line = true;
                i += 2;
            }
            b'/' if bytes.get(i + 1) == Some(&b'*') => {
                in_block = true;
                i += 2;
            }
            b'\'' | b'"' | b'`' => {
                in_str = Some(b);
                i += 1;
            }
            b'{' => {
                depth += 1;
                i += 1;
            }
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
                i += 1;
            }
            _ => i += 1,
        }
    }
    None
}

fn parse_dts_module(specifier: &str, body: &str) -> Result<AuthoredModule, String> {
    let dummy: String = specifier
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '/' || c == '.' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let dummy = format!("/{dummy}.d.ts");
    let Some(parsed) = parse_js(&dummy, body, MediaType::Dts) else {
        return Err(format!("parse dts module {specifier}"));
    };
    let mut exports = Vec::new();
    let mut privates = Vec::new();
    let mut default_export = None;
    match parsed.program_ref() {
        ProgramRef::Module(module) => {
            for item in &module.body {
                collect_dts_item(item, &mut exports, &mut privates, &mut default_export);
            }
        }
        ProgramRef::Script(script) => {
            for stmt in &script.body {
                if let Stmt::Decl(decl) = stmt {
                    collect_dts_decl(decl, false, &mut exports, &mut privates);
                }
            }
        }
    }
    Ok(AuthoredModule {
        specifier: specifier.to_string(),
        body: body.to_string(),
        exports,
        privates,
        default_export,
        extension_file: false,
    })
}

fn collect_dts_item(
    item: &ModuleItem,
    exports: &mut Vec<CompatExport>,
    privates: &mut Vec<CompatExport>,
    default_export: &mut Option<String>,
) {
    match item {
        ModuleItem::ModuleDecl(decl) => match decl {
            ModuleDecl::ExportDecl(ExportDecl { decl, .. }) => {
                collect_dts_decl(decl, true, exports, privates);
            }
            ModuleDecl::ExportDefaultDecl(ExportDefaultDecl { decl, .. }) => match decl {
                DefaultDecl::Class(class) => {
                    let name = class
                        .ident
                        .as_ref()
                        .map(|i| i.sym.to_string())
                        .unwrap_or_else(|| "default".to_string());
                    *default_export = Some(name.clone());
                    exports.push(dts_class_export(&name, &class.class, true));
                }
                DefaultDecl::Fn(func) => {
                    let name = func
                        .ident
                        .as_ref()
                        .map(|i| i.sym.to_string())
                        .unwrap_or_else(|| "default".to_string());
                    *default_export = Some(name.clone());
                    exports.push(dts_fn_export(&name, &func.function));
                }
                _ => {}
            },
            ModuleDecl::ExportDefaultExpr(expr) => {
                if let Expr::Ident(id) = expr.expr.as_ref() {
                    *default_export = Some(id.sym.to_string());
                }
            }
            ModuleDecl::ExportNamed(named) if !named.type_only => {
                for spec in &named.specifiers {
                    let ExportSpecifier::Named(n) = spec else {
                        continue;
                    };
                    if n.is_type_only {
                        continue;
                    }
                    let exported = n
                        .exported
                        .as_ref()
                        .map(export_name)
                        .unwrap_or_else(|| export_name(&n.orig));
                    if exports.iter().any(|e| export_ident(e) == exported) {
                        continue;
                    }
                    let name = named.src.as_ref().map_or_else(
                        || exported.clone(),
                        |src| {
                            format!(
                                "__reexport__{exported}__{}__{}",
                                export_name(&n.orig),
                                src.value
                            )
                        },
                    );
                    exports.push(CompatExport::Value { name });
                }
            }
            _ => {}
        },
        ModuleItem::Stmt(Stmt::Decl(decl)) => collect_dts_decl(decl, false, exports, privates),
        _ => {}
    }
}

fn collect_dts_decl(
    decl: &Decl,
    exported: bool,
    exports: &mut Vec<CompatExport>,
    privates: &mut Vec<CompatExport>,
) {
    match decl {
        Decl::Class(class) => {
            let exp = dts_class_export(&class.ident.sym, &class.class, false);
            if exported {
                exports.push(exp);
            } else {
                privates.push(exp);
            }
        }
        Decl::Fn(func) => {
            if exported {
                exports.push(dts_fn_export(&func.ident.sym, &func.function));
            }
        }
        Decl::Var(var) => {
            if !exported {
                return;
            }
            for d in &var.decls {
                if let Some(exp) = dts_var_export(d) {
                    exports.push(exp);
                }
            }
        }
        Decl::TsInterface(_) | Decl::TsTypeAlias(_) | Decl::TsEnum(_) | Decl::TsModule(_) => {}
        _ => {}
    }
}

fn dts_fn_export(name: &str, func: &Function) -> CompatExport {
    CompatExport::Function {
        name: name.to_string(),
        params: finish_params(func.params.iter().map(|p| pat_to_param(&p.pat)).collect()),
        is_async: func.is_async || func.return_type.as_deref().is_some_and(type_ann_is_promise),
    }
}

fn dts_class_export(name: &str, class: &Class, default: bool) -> CompatExport {
    let super_name = class.super_class.as_ref().and_then(|e| match e.as_ref() {
        Expr::Ident(id) => Some(id.sym.to_string()),
        _ => None,
    });
    CompatExport::Class {
        name: name.to_string(),
        default,
        super_name,
        members: dts_class_members(class),
    }
}

fn dts_class_members(class: &Class) -> Vec<Member> {
    let mut out = Vec::new();
    for member in &class.body {
        match member {
            ClassMember::Constructor(ctor) => {
                let params = finish_params(
                    ctor.params
                        .iter()
                        .filter_map(|p| match p {
                            ParamOrTsParamProp::Param(param) => Some(pat_to_param(&param.pat)),
                            ParamOrTsParamProp::TsParamProp(_) => None,
                        })
                        .collect(),
                );
                out.push(Member {
                    name: "constructor".to_string(),
                    kind: MemberKind::Constructor,
                    params,
                    is_async: false,
                    is_static: false,
                });
            }
            ClassMember::Method(method) => {
                let Some(name) = prop_name(&method.key) else {
                    continue;
                };
                let kind = match method.kind {
                    MethodKind::Getter => MemberKind::Getter,
                    MethodKind::Setter => MemberKind::Setter,
                    MethodKind::Method => MemberKind::Method,
                };
                let is_async = method.function.is_async
                    || method
                        .function
                        .return_type
                        .as_deref()
                        .is_some_and(type_ann_is_promise);
                out.push(Member {
                    name,
                    kind,
                    params: finish_params(
                        method
                            .function
                            .params
                            .iter()
                            .map(|p| pat_to_param(&p.pat))
                            .collect(),
                    ),
                    is_async,
                    is_static: method.is_static,
                });
            }
            ClassMember::ClassProp(prop) => {
                let Some(name) = prop_name(&prop.key) else {
                    continue;
                };
                out.push(Member {
                    name,
                    kind: MemberKind::Field,
                    params: Vec::new(),
                    is_async: false,
                    is_static: prop.is_static,
                });
            }
            _ => {}
        }
    }
    out
}

fn dts_var_export(decl: &VarDeclarator) -> Option<CompatExport> {
    let Pat::Ident(id) = &decl.name else {
        return None;
    };
    let name = id.id.sym.to_string();
    if let Some(ann) = id.type_ann.as_deref() {
        if let Some(members) = dts_type_members(&ann.type_ann) {
            return Some(CompatExport::Object { name, members });
        }
        if let TsType::TsFnOrConstructorType(TsFnOrConstructorType::TsFnType(fun)) =
            ann.type_ann.as_ref()
        {
            return Some(CompatExport::Function {
                name,
                params: finish_params(fun.params.iter().map(ts_fn_param).collect()),
                is_async: type_ann_is_promise(&fun.type_ann),
            });
        }
    }
    if let Some(init) = decl.init.as_deref() {
        return expr_as_export(&name, init, &HashMap::new());
    }
    Some(CompatExport::Value { name })
}

fn dts_type_members(ty: &TsType) -> Option<Vec<Member>> {
    match ty {
        TsType::TsTypeLit(lit) => Some(dts_type_lit_members(&lit.members)),
        TsType::TsParenthesizedType(p) => dts_type_members(&p.type_ann),
        _ => None,
    }
}

fn dts_type_lit_members(elements: &[TsTypeElement]) -> Vec<Member> {
    let mut out = Vec::new();
    for el in elements {
        match el {
            TsTypeElement::TsMethodSignature(m) => {
                let Some(name) = expr_key_name(&m.key) else {
                    continue;
                };
                out.push(Member {
                    name,
                    kind: MemberKind::Method,
                    params: finish_params(m.params.iter().map(ts_fn_param).collect()),
                    is_async: m.type_ann.as_deref().is_some_and(type_ann_is_promise),
                    is_static: false,
                });
            }
            TsTypeElement::TsGetterSignature(g) => {
                let Some(name) = expr_key_name(&g.key) else {
                    continue;
                };
                out.push(Member {
                    name,
                    kind: MemberKind::Getter,
                    params: Vec::new(),
                    is_async: false,
                    is_static: false,
                });
            }
            TsTypeElement::TsSetterSignature(s) => {
                let Some(name) = expr_key_name(&s.key) else {
                    continue;
                };
                out.push(Member {
                    name,
                    kind: MemberKind::Setter,
                    params: vec![ts_fn_param(&s.param)],
                    is_async: false,
                    is_static: false,
                });
            }
            TsTypeElement::TsPropertySignature(p) => {
                let Some(name) = expr_key_name(&p.key) else {
                    continue;
                };
                if let Some(ann) = p.type_ann.as_deref() {
                    if let TsType::TsFnOrConstructorType(TsFnOrConstructorType::TsFnType(f)) =
                        ann.type_ann.as_ref()
                    {
                        out.push(Member {
                            name,
                            kind: MemberKind::Method,
                            params: finish_params(f.params.iter().map(ts_fn_param).collect()),
                            is_async: type_ann_is_promise(&f.type_ann),
                            is_static: false,
                        });
                        continue;
                    }
                }
                out.push(Member {
                    name,
                    kind: MemberKind::Field,
                    params: Vec::new(),
                    is_async: false,
                    is_static: false,
                });
            }
            TsTypeElement::TsConstructSignatureDecl(c) => {
                out.push(Member {
                    name: "constructor".to_string(),
                    kind: MemberKind::Constructor,
                    params: finish_params(c.params.iter().map(ts_fn_param).collect()),
                    is_async: false,
                    is_static: false,
                });
            }
            _ => {}
        }
    }
    out
}

fn ts_fn_param(p: &TsFnParam) -> FnParam {
    match p {
        TsFnParam::Ident(id) => FnParam {
            name: sanitize_ident(&id.id.sym),
            optional: id.id.optional,
            rest: false,
        },
        TsFnParam::Rest(rest) => {
            let mut p = pat_to_param(&rest.arg);
            p.rest = true;
            p
        }
        TsFnParam::Object(_) => FnParam {
            name: "opts".to_string(),
            optional: false,
            rest: false,
        },
        TsFnParam::Array(_) => FnParam {
            name: "items".to_string(),
            optional: false,
            rest: false,
        },
    }
}

fn expr_key_name(expr: &Expr) -> Option<String> {
    match expr {
        Expr::Ident(id) => Some(id.sym.to_string()),
        Expr::Lit(Lit::Str(s)) => Some(s.value.to_string()),
        _ => None,
    }
}

fn type_ann_is_promise(ann: &TsTypeAnn) -> bool {
    type_is_promise(&ann.type_ann)
}

fn type_is_promise(ty: &TsType) -> bool {
    match ty {
        TsType::TsTypeRef(r) => match &r.type_name {
            TsEntityName::Ident(id) => id.sym.as_ref() == "Promise",
            TsEntityName::TsQualifiedName(_) => false,
        },
        TsType::TsParenthesizedType(p) => type_is_promise(&p.type_ann),
        TsType::TsUnionOrIntersectionType(TsUnionOrIntersectionType::TsUnionType(u)) => {
            u.types.iter().any(|t| type_is_promise(t))
        }
        _ => false,
    }
}

pub fn write_authored_dts_tree(authored: &AuthoredSurface, root: &Path) -> Result<(), String> {
    std::fs::create_dir_all(root).map_err(|e| format!("mkdir {}: {e}", root.display()))?;
    for module in &authored.modules {
        if module.specifier == "@rs2b0t/api" {
            continue;
        }
        let Some(rel) = tree_rel(&module.specifier) else {
            continue;
        };
        let path = root.join(&rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("mkdir {}: {e}", parent.display()))?;
        }
        let mut src =
            String::from("// Compat JS API v1 — copied from authored compat-js declarations.\n");
        let from_dir = Path::new(&rel)
            .parent()
            .map(|p| p.to_string_lossy().replace('\\', "/"))
            .unwrap_or_default();
        src.push_str(&rewrite_quoted_star_imports(
            module.body.trim_start(),
            &from_dir,
        ));
        if !src.ends_with('\n') {
            src.push('\n');
        }
        std::fs::write(&path, src).map_err(|e| format!("write {}: {e}", path.display()))?;
    }
    Ok(())
}

fn tree_rel(specifier: &str) -> Option<String> {
    let suffix = specifier
        .strip_prefix('*')
        .or_else(|| module_suffix(specifier))?;
    let file = suffix
        .strip_suffix(".js")
        .or_else(|| suffix.strip_suffix(".ts"))
        .unwrap_or(suffix);
    Some(format!("{file}.d.ts"))
}

/// Structural disagreements between the authored `.d.ts` and the live shim.
pub fn drift_against_shim(authored: &AuthoredSurface, shim: &CompatSurface) -> Vec<String> {
    let mut drifts = Vec::new();
    let mut by_spec: HashMap<String, &AuthoredModule> = HashMap::new();
    for module in &authored.modules {
        by_spec.insert(wildcard_spec(&module.specifier), module);
    }
    let barrel = authored
        .modules
        .iter()
        .find(|m| m.specifier == "@rs2b0t/api");
    let barrel_names: HashSet<String> = barrel
        .map(|m| {
            m.exports
                .iter()
                .map(|e| export_ident(e).to_string())
                .collect()
        })
        .unwrap_or_default();

    for sm in &shim.modules {
        let Some(suffix) = module_suffix(&sm.specifier) else {
            continue;
        };
        let key = format!("*{suffix}");
        let Some(am) = by_spec.get(&key) else {
            drifts.push(format!("missing module {key}"));
            continue;
        };
        compare_module(sm, am, &mut drifts);
    }

    for am in &authored.modules {
        if am.specifier == "@rs2b0t/api" {
            continue;
        }
        let in_shim = shim.modules.iter().any(|sm| {
            module_suffix(&sm.specifier)
                .is_some_and(|s| format!("*{s}") == wildcard_spec(&am.specifier))
        });
        if in_shim {
            continue;
        }
        let value_exports: Vec<&str> = am
            .exports
            .iter()
            .map(export_ident)
            .filter(|n| !n.starts_with("__"))
            .collect();
        if value_exports.is_empty() {
            drifts.push(format!(
                "extension module {} has no value exports (add a typed export and barrel it)",
                am.specifier
            ));
            continue;
        }
        for name in value_exports {
            if !barrel_names.contains(name) {
                drifts.push(format!(
                    "extension module {} export {name} is missing from the @rs2b0t/api barrel",
                    am.specifier
                ));
            }
        }
    }
    if let Some(barrel) = barrel {
        for dup in barrel_duplicate_value_exports(&barrel.body) {
            drifts.push(format!("@rs2b0t/api duplicate export {dup}"));
        }
    }
    drifts.sort();
    drifts.dedup();
    drifts
}

fn wildcard_spec(spec: &str) -> String {
    if spec.starts_with('*') || spec == "@rs2b0t/api" {
        spec.to_string()
    } else if let Some(suffix) = module_suffix(spec) {
        format!("*{suffix}")
    } else {
        spec.to_string()
    }
}

fn inherited_export(exp: &CompatExport, available: &[&CompatExport]) -> CompatExport {
    let mut resolved = exp.clone();
    let mut base = match exp {
        CompatExport::Class { super_name, .. } => super_name.as_deref(),
        _ => return resolved,
    };
    let mut seen = HashSet::new();
    while let Some(name) = base {
        if !seen.insert(name) {
            break;
        }
        let Some(CompatExport::Class {
            members: inherited,
            super_name,
            ..
        }) = available.iter().find(|e| export_ident(e) == name).copied()
        else {
            break;
        };
        if let CompatExport::Class { members, .. } = &mut resolved {
            for member in inherited {
                if member.kind != MemberKind::Constructor
                    && !members.iter().any(|m| m.name == member.name)
                {
                    members.push(member.clone());
                }
            }
        }
        base = super_name.as_deref();
    }
    resolved
}

fn compare_module(shim: &ShimModule, authored: &AuthoredModule, drifts: &mut Vec<String>) {
    let label = wildcard_spec(&authored.specifier);
    let shim_types: Vec<_> = shim.exports.iter().chain(&shim.privates).collect();
    let declaration_types: Vec<_> = authored.exports.iter().chain(&authored.privates).collect();
    for exp in &shim.exports {
        let name = export_ident(exp);
        if name.starts_with("__") {
            continue;
        }
        let Some(dts) = authored
            .exports
            .iter()
            .find(|e| export_ident(e) == name)
            .or_else(|| authored.privates.iter().find(|e| export_ident(e) == name))
        else {
            drifts.push(format!("{label} missing export {name}"));
            continue;
        };
        compare_export(
            &label,
            &inherited_export(exp, &shim_types),
            &inherited_export(dts, &declaration_types),
            drifts,
        );
    }
    for exp in &authored.exports {
        let name = export_ident(exp);
        if name.starts_with("__") {
            continue;
        }
        let found = shim
            .exports
            .iter()
            .chain(shim.privates.iter())
            .any(|e| export_ident(e) == name);
        if !found {
            drifts.push(format!("{label} extra authored export {name}"));
        }
    }
    for exp in &shim.privates {
        let name = export_ident(exp);
        let found = authored
            .privates
            .iter()
            .chain(authored.exports.iter())
            .any(|e| export_ident(e) == name);
        if !found {
            drifts.push(format!("{label} missing private type {name}"));
        }
    }
    if let Some(def) = &shim.default_export {
        match &authored.default_export {
            Some(d) if d == def => {}
            Some(d) => drifts.push(format!("{label} default export {d} != shim {def}")),
            None => {
                let class_default = authored.exports.iter().any(
                    |e| matches!(e, CompatExport::Class { name, default: true, .. } if name == def),
                );
                if !class_default {
                    drifts.push(format!("{label} missing default export {def}"));
                }
            }
        }
    }
}

fn compare_export(label: &str, shim: &CompatExport, dts: &CompatExport, drifts: &mut Vec<String>) {
    match (shim, dts) {
        (
            CompatExport::Function {
                name,
                params,
                is_async,
            },
            CompatExport::Function {
                params: dparams,
                is_async: dasync,
                ..
            },
        ) => {
            compare_params(label, name, params, dparams, drifts);
            if *is_async && !*dasync {
                drifts.push(format!(
                    "{label} {name} async-ness: shim returns a Promise, declaration is not Promise<T>"
                ));
            }
        }
        (
            CompatExport::Object { name, members },
            CompatExport::Object {
                members: dmembers, ..
            },
        )
        | (
            CompatExport::Class { name, members, .. },
            CompatExport::Class {
                members: dmembers, ..
            },
        )
        | (
            CompatExport::Object { name, members },
            CompatExport::Class {
                members: dmembers, ..
            },
        )
        | (
            CompatExport::Class { name, members, .. },
            CompatExport::Object {
                members: dmembers, ..
            },
        ) => {
            compare_members(label, name, members, dmembers, drifts);
        }
        (CompatExport::Value { name }, _) => {
            if export_ident(dts) != name.as_str() {
                drifts.push(format!("{label} {name} kind mismatch"));
            }
        }
        (shim, dts) => {
            let sn = export_ident(shim);
            let dn = export_ident(dts);
            if kind_tag(shim) != kind_tag(dts) {
                drifts.push(format!("{label} {sn}/{dn} kind mismatch"));
            }
        }
    }
}

fn kind_tag(exp: &CompatExport) -> &'static str {
    match exp {
        CompatExport::Class { .. } => "class",
        CompatExport::Object { .. } => "object",
        CompatExport::Function { .. } => "function",
        CompatExport::Value { .. } => "value",
    }
}

fn compare_members(
    label: &str,
    owner: &str,
    shim: &[Member],
    dts: &[Member],
    drifts: &mut Vec<String>,
) {
    for m in shim {
        let Some(d) = dts.iter().find(|x| x.name == m.name) else {
            drifts.push(format!("{label} {owner} missing member {}", m.name));
            continue;
        };
        if m.kind == MemberKind::Method
            && d.kind != MemberKind::Method
            && d.kind != MemberKind::Field
        {
            // getters vs methods still count as present; arity is compared for methods
        }
        if m.kind == MemberKind::Method || m.kind == MemberKind::Constructor {
            compare_params(
                label,
                &format!("{owner}.{}", m.name),
                &m.params,
                &d.params,
                drifts,
            );
            if m.is_async && !d.is_async {
                drifts.push(format!(
                    "{label} {owner}.{} async-ness: shim returns a Promise, declaration is not Promise<T>",
                    m.name
                ));
            }
        }
    }
    for d in dts {
        if !shim.iter().any(|m| m.name == d.name) {
            drifts.push(format!("{label} {owner} extra authored member {}", d.name));
        }
    }
}

fn compare_params(
    label: &str,
    owner: &str,
    shim: &[FnParam],
    dts: &[FnParam],
    drifts: &mut Vec<String>,
) {
    let shim_req = shim.iter().filter(|p| !p.optional && !p.rest).count();
    let dts_req = dts.iter().filter(|p| !p.optional && !p.rest).count();
    if dts_req > shim_req {
        drifts.push(format!(
            "{label} {owner} arity: declaration requires {dts_req} params, shim requires {shim_req}"
        ));
    }
    let dts_slots = dts.iter().filter(|p| !p.rest).count();
    let dts_has_rest = dts.iter().any(|p| p.rest);
    if dts_slots < shim_req && !dts_has_rest {
        drifts.push(format!(
            "{label} {owner} arity: declaration has {dts_slots} params, shim requires {shim_req}"
        ));
    }
    if dts.len() > shim.len() {
        for p in &dts[shim.len()..] {
            if !p.optional && !p.rest {
                drifts.push(format!(
                    "{label} {owner} extra required parameter {}",
                    p.name
                ));
            }
        }
    }
}

fn rewrite_quoted_star_imports(body: &str, from_dir: &str) -> String {
    let mut out = String::with_capacity(body.len());
    let mut i = 0;
    let bytes = body.as_bytes();
    while i < bytes.len() {
        let q = bytes[i];
        if (q == b'\'' || q == b'"') && bytes.get(i + 1) == Some(&b'*') {
            if let Some(end) = body[i + 2..].find(q as char) {
                let inner = &body[i + 2..i + 2 + end];
                if !inner.contains('\n') && (inner.ends_with(".js") || inner.ends_with(".ts")) {
                    out.push(q as char);
                    out.push_str(&relative_from_dir(from_dir, inner));
                    out.push(q as char);
                    i = i + 3 + end;
                    continue;
                }
            }
        }
        let ch = body[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

fn relative_from_dir(from_dir: &str, target: &str) -> String {
    if from_dir.is_empty() {
        return format!("./{target}");
    }
    let from: Vec<&str> = from_dir.split('/').filter(|s| !s.is_empty()).collect();
    let to: Vec<&str> = target.split('/').filter(|s| !s.is_empty()).collect();
    let mut i = 0;
    while i < from.len() && i < to.len() && from[i] == to[i] {
        i += 1;
    }
    let mut parts: Vec<&str> = std::iter::repeat_n("..", from.len() - i).collect();
    parts.extend(to[i..].iter().copied());
    if parts.is_empty() {
        format!("./{}", to.last().copied().unwrap_or(target))
    } else {
        parts.join("/")
    }
}

fn barrel_duplicate_value_exports(body: &str) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut dups = Vec::new();
    for name in barrel_value_export_names(body) {
        if !seen.insert(name.clone()) {
            dups.push(name);
        }
    }
    dups.sort();
    dups.dedup();
    dups
}

fn barrel_value_export_names(body: &str) -> Vec<String> {
    let mut names = Vec::new();
    for line in body.lines() {
        let t = line.trim();
        if t.starts_with("export type ") {
            continue;
        }
        if let Some(rest) = t.strip_prefix("export {") {
            if let Some(inside) = rest.split('}').next() {
                for n in inside.split(',') {
                    let n = n.trim();
                    if !n.is_empty() {
                        names.push(n.to_string());
                    }
                }
            }
            continue;
        }
        if let Some(rest) = t.strip_prefix("export const ") {
            if let Some(n) = rest.split(|c: char| c == ':' || c.is_whitespace()).next() {
                if !n.is_empty() {
                    names.push(n.to_string());
                }
            }
            continue;
        }
        if let Some(rest) = t.strip_prefix("export function ") {
            if let Some(n) = rest.split('(').next() {
                let n = n.trim();
                if !n.is_empty() {
                    names.push(n.to_string());
                }
            }
            continue;
        }
        if let Some(rest) = t.strip_prefix("export class ") {
            if let Some(n) = rest.split(|c: char| c.is_whitespace() || c == '{').next() {
                if !n.is_empty() {
                    names.push(n.to_string());
                }
            }
        }
    }
    names
}

pub fn check_compat_dts_drift() -> Result<(), String> {
    let authored = load_authored_dts()?;
    let shim = collect_compat_surface();
    let drifts = drift_against_shim(&authored, &shim);
    if drifts.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "authored compat-js declarations drifted from the live shim:\n  {}",
            drifts.join("\n  ")
        ))
    }
}
