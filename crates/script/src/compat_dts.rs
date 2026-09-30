//! TypeScript declarations for the **compat** (JS API v1) surface.
//!
//! Source of truth is the JS shim name maps (`shim_modules` + prelude),
//! the same modules the isolate loads. Catalog `index.d.ts` names that no
//! shim owns stay as `not impl` stubs (the declared ABI fixture). There is
//! no handwritten method list.
//!
//! O-SCRIPT-API can append modules through [`DtsExtension`] without forking
//! the generator. Regen: `cargo test -p script --test compat_dts regen_compat_dts -- --ignored`

use crate::declared_abi::{load_fixture, DeclaredExport, DeclaredKind};
use crate::shim::{shim_modules, PRELUDE};
use deno_ast::swc::ast::*;
use deno_ast::{MediaType, ParseParams, ParsedSource, ProgramRef};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

/// Extra modules later APIs (O-SCRIPT-API) append to the generated `.d.ts`.
#[derive(Debug, Clone)]
pub struct DtsExtension {
    pub specifier: String,
    pub exports: Vec<CompatExport>,
}

#[derive(Debug, Clone)]
pub struct CompatSurface {
    pub modules: Vec<ShimModule>,
    pub prelude: Vec<CompatExport>,
    pub extras: Vec<DtsExtension>,
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

pub fn write_compat_dts() -> Result<(), String> {
    write_compat_dts_to(&compat_dts_path())
}

pub fn write_compat_dts_to(path: &Path) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("mkdir {}: {e}", parent.display()))?;
    }
    let src = render_compat_dts();
    std::fs::write(path, src).map_err(|e| format!("write {}: {e}", path.display()))
}

/// Write one `.d.ts` per shim module under `root`, mirroring `/rs2b0t/bot/`.
/// Catalog scripts import relative `../../api/game/Game.js`; TypeScript
/// resolves those to files, not ambient wildcards. O-SCRIPT-API extras are
/// not written here — pass them through [`render_compat_dts_with`].
pub fn write_compat_dts_tree(root: &Path) -> Result<(), String> {
    write_compat_dts_tree_from(&collect_compat_surface(), root)
}

pub fn write_compat_dts_tree_from(surface: &CompatSurface, root: &Path) -> Result<(), String> {
    std::fs::create_dir_all(root).map_err(|e| format!("mkdir {}: {e}", root.display()))?;
    let mut modules: Vec<&ShimModule> = surface.modules.iter().collect();
    modules.sort_by(|a, b| a.specifier.cmp(&b.specifier));
    for module in modules {
        let Some(suffix) = module_suffix(&module.specifier) else {
            continue;
        };
        let file = suffix
            .strip_suffix(".js")
            .or_else(|| suffix.strip_suffix(".ts"))
            .unwrap_or(suffix);
        let path = root.join(format!("{file}.d.ts"));
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("mkdir {}: {e}", parent.display()))?;
        }
        let mut src = String::from("// Generated from the JS shim name maps — do not edit.\n");
        render_module_body(&mut src, module, true);
        for extra in &surface.extras {
            if module_suffix(&extra.specifier) == Some(suffix) {
                for exp in &extra.exports {
                    render_export(&mut src, exp, true);
                }
            }
        }
        std::fs::write(&path, src).map_err(|e| format!("write {}: {e}", path.display()))?;
    }
    Ok(())
}

/// Render the living compat declarations from the shim name maps.
pub fn render_compat_dts() -> String {
    render_compat_dts_with(&[])
}

/// Same as [`render_compat_dts`], with extra modules for later APIs.
pub fn render_compat_dts_with(extras: &[DtsExtension]) -> String {
    let mut surface = collect_compat_surface();
    surface.extras.extend(extras.iter().cloned());
    render_compat_surface(&surface)
}

/// Render a surface already collected (tests inject patched shim sources).
pub fn render_compat_surface(surface: &CompatSurface) -> String {
    render_surface(surface)
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
    CompatSurface {
        modules,
        prelude,
        extras: Vec::new(),
    }
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
    CompatSurface {
        modules,
        prelude,
        extras: Vec::new(),
    }
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
    ShimModule {
        specifier: specifier.to_string(),
        exports,
        default_export: state.default_export,
        privates,
    }
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
            Some(proxy_export(name, new_expr))
        }
        Expr::New(new_expr) => Some(new_ident_export(name, new_expr, locals)),
        Expr::Call(call) if is_callee_ident(&call.callee, "proxy") => {
            let members = call
                .args
                .get(1)
                .and_then(|a| object_members(&a.expr))
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
            members: object_lit_members(obj),
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

fn proxy_export(name: &str, new_expr: &NewExpr) -> CompatExport {
    let members = new_expr
        .args
        .as_ref()
        .and_then(|a| a.first())
        .and_then(|a| object_members(&a.expr))
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

fn object_members(expr: &Expr) -> Option<Vec<Member>> {
    match expr {
        Expr::Object(obj) => Some(object_lit_members(obj)),
        Expr::Paren(p) => object_members(&p.expr),
        _ => None,
    }
}

fn object_lit_members(obj: &ObjectLit) -> Vec<Member> {
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

fn render_surface(surface: &CompatSurface) -> String {
    let mut out = String::from(
        "// Generated from the JS shim name maps (shim_modules + prelude) — do not edit by hand.\n\
         // Regen: cargo test -p script --test compat_dts regen_compat_dts -- --ignored\n\
         // Compat (JS API v1) as the isolate exposes it. O-SCRIPT-API appends via DtsExtension.\n\
         // Not a clone of rs2b0t-api; stubs are declared ABI names with no shim owner.\n\n",
    );

    let barrel = barrel_exports(surface);
    out.push_str("declare module '@rs2b0t/api' {\n");
    for exp in &barrel {
        render_export(&mut out, exp, false);
        out.push('\n');
    }
    out.push_str("}\n\n");

    let mut modules: Vec<&ShimModule> = surface.modules.iter().collect();
    modules.sort_by(|a, b| a.specifier.cmp(&b.specifier));
    for module in modules {
        let Some(suffix) = module_suffix(&module.specifier) else {
            continue;
        };
        out.push_str("declare module '*");
        out.push_str(suffix);
        out.push_str("' {\n");
        render_module_body(&mut out, module, false);
        out.push_str("}\n\n");
    }
    for extra in &surface.extras {
        let suffix = module_suffix(&extra.specifier).unwrap_or(extra.specifier.as_str());
        out.push_str("declare module '*");
        out.push_str(suffix);
        out.push_str("' {\n");
        for exp in &extra.exports {
            render_export(&mut out, exp, false);
        }
        out.push_str("}\n\n");
    }
    out
}

fn render_module_body(out: &mut String, module: &ShimModule, top: bool) {
    for exp in &module.privates {
        render_item(out, exp, top, false);
    }
    for exp in &module.exports {
        render_item(out, exp, top, true);
    }
    emit_default_alias(out, module, if top { "" } else { "  " });
    if module.exports.is_empty() && module.privates.is_empty() && module.default_export.is_none() {
        out.push_str(if top {
            "export {};\n"
        } else {
            "  export {};\n"
        });
    }
}

fn emit_default_alias(out: &mut String, module: &ShimModule, pad: &str) {
    let Some(name) = &module.default_export else {
        return;
    };
    if module
        .exports
        .iter()
        .any(|e| matches!(e, CompatExport::Class { default: true, .. }))
    {
        return;
    }
    out.push_str(pad);
    out.push_str("export default ");
    out.push_str(name);
    out.push_str(";\n");
}

fn module_suffix(specifier: &str) -> Option<&str> {
    specifier.strip_prefix("/rs2b0t/bot/")
}

fn barrel_exports(surface: &CompatSurface) -> Vec<CompatExport> {
    let fixture = load_fixture().unwrap_or_default();
    let mut by_name: BTreeMap<String, CompatExport> = BTreeMap::new();
    for module in &surface.modules {
        for exp in &module.exports {
            by_name
                .entry(export_ident(exp).to_string())
                .or_insert_with(|| exp.clone());
        }
        for exp in &module.privates {
            by_name
                .entry(export_ident(exp).to_string())
                .or_insert_with(|| exp.clone());
        }
    }
    for exp in &surface.prelude {
        by_name
            .entry(export_ident(exp).to_string())
            .or_insert_with(|| exp.clone());
    }
    for extra in &surface.extras {
        for exp in &extra.exports {
            by_name
                .entry(export_ident(exp).to_string())
                .or_insert_with(|| exp.clone());
        }
    }

    let mut out = Vec::new();
    if let Some(define) = by_name.get("defineBot") {
        out.push(define.clone());
    } else {
        out.push(CompatExport::Function {
            name: "defineBot".to_string(),
            params: vec![FnParam {
                name: "manifest".to_string(),
                optional: false,
                rest: false,
            }],
            is_async: false,
        });
    }
    let mut names: Vec<&DeclaredExport> = fixture.iter().collect();
    names.sort_by(|a, b| a.name.cmp(&b.name));
    for exp in names {
        if exp.name == "defineBot" {
            continue;
        }
        if let Some(found) = by_name.get(&exp.name) {
            let mut found = found.clone();
            if let CompatExport::Class { default, .. } = &mut found {
                *default = false;
            }
            out.push(found);
        } else {
            out.push(stub_from_declared(exp));
        }
    }
    let needed: Vec<String> = out
        .iter()
        .filter_map(|e| match e {
            CompatExport::Class {
                super_name: Some(s),
                ..
            } => Some(s.clone()),
            _ => None,
        })
        .collect();
    for name in needed {
        if out.iter().any(|e| export_ident(e) == name) {
            continue;
        }
        if let Some(found) = by_name.get(&name) {
            let mut found = found.clone();
            if let CompatExport::Class { default, .. } = &mut found {
                *default = false;
            }
            out.push(found);
        }
    }
    out
}

fn stub_from_declared(exp: &DeclaredExport) -> CompatExport {
    match exp.kind {
        DeclaredKind::Value => CompatExport::Value {
            name: exp.name.clone(),
        },
        DeclaredKind::Function => CompatExport::Function {
            name: exp.name.clone(),
            params: vec![FnParam {
                name: "args".to_string(),
                optional: false,
                rest: true,
            }],
            is_async: false,
        },
        DeclaredKind::Object => CompatExport::Object {
            name: exp.name.clone(),
            members: exp
                .members
                .iter()
                .map(|m| Member {
                    name: m.clone(),
                    kind: MemberKind::Method,
                    params: vec![FnParam {
                        name: "args".to_string(),
                        optional: false,
                        rest: true,
                    }],
                    is_async: false,
                    is_static: false,
                })
                .collect(),
        },
        DeclaredKind::Class => {
            let mut members: Vec<Member> = exp
                .members
                .iter()
                .filter(|m| m.as_str() != "constructor")
                .map(|m| {
                    let is_static =
                        exp.name == "Area" && matches!(m.as_str(), "rectangular" | "circular");
                    Member {
                        name: m.clone(),
                        kind: MemberKind::Method,
                        params: vec![FnParam {
                            name: "args".to_string(),
                            optional: false,
                            rest: true,
                        }],
                        is_async: false,
                        is_static,
                    }
                })
                .collect();
            members.insert(
                0,
                Member {
                    name: "constructor".to_string(),
                    kind: MemberKind::Constructor,
                    params: Vec::new(),
                    is_async: false,
                    is_static: false,
                },
            );
            CompatExport::Class {
                name: exp.name.clone(),
                default: false,
                super_name: None,
                members,
            }
        }
    }
}

fn render_export(out: &mut String, exp: &CompatExport, top: bool) {
    render_item(out, exp, top, true);
}

fn render_item(out: &mut String, exp: &CompatExport, top: bool, exported: bool) {
    let pad = if top { "" } else { "  " };
    match exp {
        CompatExport::Class {
            name,
            default,
            super_name,
            members,
        } => {
            out.push_str(pad);
            if exported {
                out.push_str("export class ");
            } else if top {
                out.push_str("declare class ");
            } else {
                out.push_str("class ");
            }
            out.push_str(name);
            if let Some(sup) = super_name {
                out.push_str(" extends ");
                out.push_str(sup);
            }
            out.push_str(" {\n");
            render_members(out, members, if top { "  " } else { "    " });
            out.push_str(pad);
            out.push_str("}\n");
            if *default && exported {
                out.push_str(pad);
                out.push_str("export default ");
                out.push_str(name);
                out.push_str(";\n");
            }
        }
        CompatExport::Object { name, members } => {
            out.push_str(pad);
            out.push_str("export const ");
            out.push_str(name);
            out.push_str(": {\n");
            render_members(out, members, if top { "  " } else { "    " });
            out.push_str(pad);
            out.push_str("};\n");
        }
        CompatExport::Function {
            name,
            params,
            is_async,
        } => {
            out.push_str(pad);
            out.push_str("export function ");
            out.push_str(name);
            render_params(out, params);
            out.push_str(": ");
            if *is_async {
                out.push_str("Promise<any>");
            } else {
                out.push_str("any");
            }
            out.push_str(";\n");
        }
        CompatExport::Value { name } => {
            if name.starts_with("__") || name == "*" {
                return;
            }
            out.push_str(pad);
            if name == "apiVersion" {
                out.push_str("export const apiVersion: number;\n");
            } else {
                out.push_str("export const ");
                out.push_str(name);
                out.push_str(": any;\n");
            }
        }
    }
}

fn render_members(out: &mut String, members: &[Member], pad: &str) {
    for m in members {
        out.push_str(pad);
        if m.is_static && m.kind != MemberKind::Constructor {
            out.push_str("static ");
        }
        match m.kind {
            MemberKind::Getter => {
                out.push_str("get ");
                write_member_name(out, &m.name);
                out.push_str("(): any;\n");
            }
            MemberKind::Setter => {
                out.push_str("set ");
                write_member_name(out, &m.name);
                render_params(out, &m.params);
                out.push_str(";\n");
            }
            MemberKind::Field => {
                write_member_name(out, &m.name);
                out.push_str(": any;\n");
            }
            MemberKind::Constructor => {
                out.push_str("constructor");
                render_params(out, &m.params);
                out.push_str(";\n");
            }
            MemberKind::Method => {
                write_member_name(out, &m.name);
                render_params(out, &m.params);
                out.push_str(": ");
                if m.is_async {
                    out.push_str("Promise<any>");
                } else {
                    out.push_str("any");
                }
                out.push_str(";\n");
            }
        }
    }
}

fn write_member_name(out: &mut String, name: &str) {
    if is_ts_ident(name) {
        out.push_str(name);
    } else {
        out.push('"');
        out.push_str(name);
        out.push('"');
    }
}

fn render_params(out: &mut String, params: &[FnParam]) {
    out.push('(');
    for (i, p) in params.iter().enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        if p.rest {
            out.push_str("...");
        }
        out.push_str(&p.name);
        if p.optional && !p.rest {
            out.push('?');
        }
        out.push_str(": any");
    }
    out.push(')');
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
