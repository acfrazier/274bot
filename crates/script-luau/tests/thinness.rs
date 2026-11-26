//! The thinness census of `script-luau` (ADR 0023 item 20; rule O11 of
//! `LUAU-DESIGN-1114/design-r3.md` 7.4, with the L4a acceptance of
//! `LUAU-DESIGN-1114/MAIN-ACCEPTANCE.md` and design rounds 4, 5 and 7).
//!
//! It parses the binding sources with `syn` and follows every function that a
//! binding can reach in the crate, through direct calls, function paths used
//! as values and closure bodies. `syn` resolves no types, so a method name
//! that the crate defines counts as a call into the crate (rule T3). Each
//! negative fixture at the end is a source that the census must refuse.
//!
//! `THINNESS_BLESS=1` rewrites the two checked-in files under
//! `tests/thinness/` (the core paths of T5 and the size baseline of T10);
//! the diff then shows the change.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use syn::visit::Visit;

const BINDING_FILES: &[&str] = &["mod.rs", "host.rs", "convert.rs"];
const CORE_CRATES: &[&str] = &["script", "api", "vault"];
const NUMERIC_TYPES: &[&str] = &[
    "i8", "i16", "i32", "i64", "i128", "isize", "u8", "u16", "u32", "u64", "u128", "usize", "f32",
    "f64",
];
/// Adapters and producers that may only start at a parameter.
const ITERATOR_SOURCES: &[&str] = &[
    "iter",
    "iter_mut",
    "into_iter",
    "chars",
    "bytes",
    "char_indices",
    "lines",
    "split",
    "split_whitespace",
    "windows",
    "chunks",
    "drain",
];
/// Calls that are unbounded or walk a script value: never in the reach set.
const UNBOUNDED_CALLS: &[&str] = &[
    "pairs",
    "sequence_values",
    "for_each",
    "cycle",
    "repeat",
    "repeat_with",
    "from_fn",
    "successors",
];
const ITERATOR_SUFFIXES: &[&str] = &["iter", "iter_mut", "into_iter", "enumerate"];
const VM_REGISTRATIONS: &[&str] = &["create_function", "create_function_mut"];

// ---------------------------------------------------------------------------
// Sources

struct SourceFile {
    name: String,
    text: String,
    ast: syn::File,
}

fn parse(name: &str, text: String) -> SourceFile {
    let ast = syn::parse_file(&text).unwrap_or_else(|error| panic!("{name}: {error}"));
    SourceFile {
        name: name.to_owned(),
        text,
        ast,
    }
}

fn crate_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

/// Every Rust file under `src`, keyed by its path below `src`.
fn crate_sources() -> BTreeMap<String, String> {
    fn walk(dir: &Path, root: &Path, out: &mut BTreeMap<String, String>) {
        let mut entries = std::fs::read_dir(dir)
            .expect("the crate's source directory is readable")
            .map(|entry| entry.expect("a source directory entry is readable").path())
            .collect::<Vec<_>>();
        entries.sort();
        for path in entries {
            if path.is_dir() {
                walk(&path, root, out);
            } else if path.extension().is_some_and(|extension| extension == "rs") {
                let name = path
                    .strip_prefix(root)
                    .expect("a walked path lies below the walk's root")
                    .to_string_lossy()
                    .replace('\\', "/");
                out.insert(name, read(&path));
            }
        }
    }
    let root = crate_dir().join("src");
    let mut out = BTreeMap::new();
    walk(&root, &root, &mut out);
    out
}

// ---------------------------------------------------------------------------
// The census model

#[derive(Clone, Debug, PartialEq, Eq)]
struct Row {
    path: String,
    kind: String,
    call: String,
    function: String,
    js: Option<String>,
}

#[derive(Clone)]
enum Body {
    Free(syn::ItemFn),
    Method(syn::ImplItemFn),
}

impl Body {
    fn sig(&self) -> &syn::Signature {
        match self {
            Body::Free(item) => &item.sig,
            Body::Method(item) => &item.sig,
        }
    }

    fn block(&self) -> &syn::Block {
        match self {
            Body::Free(item) => &item.block,
            Body::Method(item) => &item.block,
        }
    }
}

/// A function of the binding directory: `name` or `Type::name`.
#[derive(Clone)]
struct Function {
    key: String,
    file: String,
    owner: Option<String>,
    body: Body,
}

#[derive(Default)]
struct Census {
    rows: Vec<Row>,
    /// The functions of the binding directory, by key.
    functions: BTreeMap<String, Function>,
    /// Free function names of the binding directory.
    free: BTreeMap<String, String>,
    /// Method names of the binding directory, with their keys.
    methods: BTreeMap<String, Vec<String>>,
    /// Types and enum variants that the binding directory defines.
    types: BTreeSet<String>,
    variants: BTreeSet<String>,
    /// Free functions, methods and types of the rest of the crate.
    outside_free: BTreeSet<String>,
    outside_methods: BTreeSet<String>,
    outside_types: BTreeSet<String>,
    /// The run-state methods that a Host method calls (runtime.rs).
    run_state: BTreeMap<String, syn::ImplItemFn>,
    /// The public methods of the seat client of `script`.
    seat_client: BTreeSet<String>,
    /// `use` imports of the binding directory: ident -> path.
    imports: BTreeMap<String, Vec<String>>,
    violations: Vec<String>,
    reach: BTreeSet<String>,
    edges: BTreeMap<String, BTreeSet<String>>,
    core_paths: BTreeSet<String>,
    host_calls: BTreeSet<String>,
}

fn path_idents(path: &syn::Path) -> Vec<String> {
    path.segments
        .iter()
        .map(|segment| segment.ident.to_string())
        .collect()
}

fn type_last_ident(ty: &syn::Type) -> Option<String> {
    match ty {
        syn::Type::Path(path) => path.path.segments.last().map(|s| s.ident.to_string()),
        syn::Type::Reference(reference) => type_last_ident(&reference.elem),
        _ => None,
    }
}

fn collect_type_idents(ty: &syn::Type, out: &mut Vec<String>) {
    struct Idents<'a>(&'a mut Vec<String>);
    impl<'ast> Visit<'ast> for Idents<'_> {
        fn visit_path_segment(&mut self, segment: &'ast syn::PathSegment) {
            self.0.push(segment.ident.to_string());
            syn::visit::visit_path_segment(self, segment);
        }
    }
    Idents(out).visit_type(ty);
}

fn has_numeric(ty: &syn::Type) -> bool {
    let mut idents = Vec::new();
    collect_type_idents(ty, &mut idents);
    idents
        .iter()
        .any(|ident| NUMERIC_TYPES.contains(&ident.as_str()))
}

fn use_imports(
    tree: &syn::UseTree,
    prefix: &mut Vec<String>,
    out: &mut BTreeMap<String, Vec<String>>,
) {
    match tree {
        syn::UseTree::Path(path) => {
            prefix.push(path.ident.to_string());
            use_imports(&path.tree, prefix, out);
            prefix.pop();
        }
        syn::UseTree::Name(name) => {
            let mut full = prefix.clone();
            let ident = name.ident.to_string();
            if ident == "self" {
                if let Some(last) = prefix.last() {
                    out.insert(last.clone(), full);
                }
            } else {
                full.push(ident.clone());
                out.insert(ident, full);
            }
        }
        syn::UseTree::Rename(rename) => {
            let mut full = prefix.clone();
            full.push(rename.ident.to_string());
            out.insert(rename.rename.to_string(), full);
        }
        syn::UseTree::Group(group) => {
            for item in &group.items {
                use_imports(item, prefix, out);
            }
        }
        syn::UseTree::Glob(_) => {}
    }
}

fn expr_lit_str(expr: &syn::Expr) -> Option<String> {
    match expr {
        syn::Expr::Lit(syn::ExprLit {
            lit: syn::Lit::Str(text),
            ..
        }) => Some(text.value()),
        _ => None,
    }
}

fn parse_rows(file: &syn::File) -> Vec<Row> {
    let mut rows = Vec::new();
    for item in &file.items {
        let syn::Item::Static(item) = item else {
            continue;
        };
        if item.ident != "BINDINGS" {
            continue;
        }
        let syn::Expr::Reference(reference) = &*item.expr else {
            panic!("BINDINGS must be `&[...]`");
        };
        let syn::Expr::Array(array) = &*reference.expr else {
            panic!("BINDINGS must be `&[...]`");
        };
        for element in &array.elems {
            let syn::Expr::Struct(row) = element else {
                panic!("a BINDINGS row must be a struct literal");
            };
            let field = |name: &str| {
                row.fields
                    .iter()
                    .find(
                        |field| matches!(&field.member, syn::Member::Named(ident) if ident == name),
                    )
                    .map(|field| field.expr.clone())
                    .unwrap_or_else(|| panic!("BINDINGS row without `{name}`"))
            };
            let path = expr_lit_str(&field("path")).expect("row path is a string literal");
            let kind = match field("kind") {
                syn::Expr::Path(path) => path_idents(&path.path)
                    .last()
                    .expect("a parsed path has at least one segment")
                    .clone(),
                other => panic!("row kind is a path: {}", quote_expr(&other)),
            };
            let (call, function) = match field("call") {
                syn::Expr::Call(call) => {
                    let syn::Expr::Path(variant) = &*call.func else {
                        panic!("row call is `Call::X(function)`");
                    };
                    let [syn::Expr::Path(function)] = call.args.iter().collect::<Vec<_>>()[..]
                    else {
                        panic!("row call names one function");
                    };
                    (
                        path_idents(&variant.path)
                            .last()
                            .expect("a parsed path has at least one segment")
                            .clone(),
                        path_idents(&function.path).join("::"),
                    )
                }
                other => panic!("row call is `Call::X(function)`: {}", quote_expr(&other)),
            };
            let js = match field("js") {
                syn::Expr::Call(call) => call.args.first().and_then(expr_lit_str),
                _ => None,
            };
            rows.push(Row {
                path,
                kind,
                call,
                function,
                js,
            });
        }
    }
    rows
}

fn quote_expr(expr: &syn::Expr) -> String {
    format!("{:?}", std::mem::discriminant(expr))
}

impl Census {
    /// `bindings` is the binding directory (`mod.rs`, `host.rs`,
    /// `convert.rs`); `outside` is every other source of the crate.
    fn build(bindings: &[SourceFile], outside: &[SourceFile]) -> Self {
        let mut census = Census::default();
        for file in bindings {
            if file.name == "mod.rs" {
                census.rows = parse_rows(&file.ast);
            }
            for item in &file.ast.items {
                census.index_item(file, item);
            }
        }
        for file in outside {
            census.index_outside(&file.ast.items);
        }
        let client = parse(
            "client.rs",
            read(&crate_dir().join("../script/src/surface/client.rs")),
        );
        for item in &client.ast.items {
            if let syn::Item::Impl(implementation) = item {
                if type_last_ident(&implementation.self_ty).as_deref() == Some("SeatClient") {
                    for item in &implementation.items {
                        if let syn::ImplItem::Fn(method) = item {
                            if matches!(method.vis, syn::Visibility::Public(_)) {
                                census.seat_client.insert(method.sig.ident.to_string());
                            }
                        }
                    }
                }
            }
        }
        census
    }

    fn index_item(&mut self, file: &SourceFile, item: &syn::Item) {
        match item {
            syn::Item::Fn(function) => {
                let name = function.sig.ident.to_string();
                self.free.insert(name.clone(), name.clone());
                self.functions.insert(
                    name.clone(),
                    Function {
                        key: name,
                        file: file.name.clone(),
                        owner: None,
                        body: Body::Free(function.clone()),
                    },
                );
            }
            syn::Item::Impl(implementation) => {
                let Some(owner) = type_last_ident(&implementation.self_ty) else {
                    return;
                };
                for item in &implementation.items {
                    if let syn::ImplItem::Fn(method) = item {
                        let name = method.sig.ident.to_string();
                        let key = format!("{owner}::{name}");
                        self.methods.entry(name).or_default().push(key.clone());
                        self.functions.insert(
                            key.clone(),
                            Function {
                                key,
                                file: file.name.clone(),
                                owner: Some(owner.clone()),
                                body: Body::Method(method.clone()),
                            },
                        );
                    }
                }
            }
            syn::Item::Struct(item) => {
                self.types.insert(item.ident.to_string());
            }
            syn::Item::Enum(item) => {
                self.types.insert(item.ident.to_string());
                for variant in &item.variants {
                    self.variants
                        .insert(format!("{}::{}", item.ident, variant.ident));
                }
            }
            syn::Item::Type(item) => {
                self.types.insert(item.ident.to_string());
            }
            syn::Item::Use(item) => {
                use_imports(&item.tree, &mut Vec::new(), &mut self.imports);
            }
            _ => {}
        }
    }

    fn index_outside(&mut self, items: &[syn::Item]) {
        for item in items {
            match item {
                syn::Item::Fn(function) => {
                    self.outside_free.insert(function.sig.ident.to_string());
                }
                syn::Item::Impl(implementation) => {
                    let owner = type_last_ident(&implementation.self_ty);
                    for item in &implementation.items {
                        if let syn::ImplItem::Fn(method) = item {
                            self.outside_methods.insert(method.sig.ident.to_string());
                            if owner.as_deref() == Some("RunState") {
                                self.run_state
                                    .insert(method.sig.ident.to_string(), method.clone());
                            }
                        }
                    }
                }
                syn::Item::Struct(item) => {
                    self.outside_types.insert(item.ident.to_string());
                }
                syn::Item::Enum(item) => {
                    self.outside_types.insert(item.ident.to_string());
                }
                syn::Item::Mod(module) => {
                    if let Some((_, items)) = &module.content {
                        self.index_outside(items);
                    }
                }
                _ => {}
            }
        }
    }

    fn violation(&mut self, text: String) {
        self.violations.push(text);
    }

    /// The seat client calls of the run-state method that a Host method
    /// calls. Each such method makes at most one ("about one native call
    /// for each method", plan slice L4a).
    fn record_seat_calls(&mut self, run_method: &str) {
        let Some(body) = self.run_state.get(run_method).cloned() else {
            return;
        };
        let mut refs = References::default();
        refs.visit_block(&body.block);
        let calls = refs
            .methods
            .iter()
            .map(|(method, _)| method.clone())
            .filter(|method| self.seat_client.contains(method))
            .collect::<Vec<_>>();
        if calls.len() > 1 {
            self.violation(format!(
                "T5: RunState::{run_method} makes {} seat client calls",
                calls.len()
            ));
        }
        for call in calls {
            self.core_paths.insert(format!(
                "script::surface::client::SeatClient::{call} <- RunState::{run_method}"
            ));
        }
    }

    /// Run rules T1 to T5 and the round 4, 5 and 7 additions.
    fn check(&mut self) {
        let rows = self.rows.clone();
        let mut work = Vec::new();
        for row in &rows {
            let Some(function) = self.functions.get(&row.function).cloned() else {
                self.violation(format!(
                    "T3: row `{}` names no function `{}`",
                    row.path, row.function
                ));
                continue;
            };
            if function.file != "mod.rs" || function.owner.is_some() {
                self.violation(format!(
                    "T3: binding `{}` must be a free function of mod.rs",
                    row.path
                ));
            }
            self.check_binding(row, &function);
            work.push(function.key.clone());
        }
        let bindings = work.iter().cloned().collect::<BTreeSet<_>>();
        while let Some(key) = work.pop() {
            if !self.reach.insert(key.clone()) {
                continue;
            }
            let function = self.functions[&key].clone();
            let calls = self.scan_function(&function, bindings.contains(&key));
            for callee in &calls {
                if !self.reach.contains(callee) {
                    work.push(callee.clone());
                }
            }
            self.edges.insert(key, calls);
        }
        // Rule T3: every function that a binding reaches, other than a
        // binding or a Host method, is a conversion helper of convert.rs.
        for key in &self.reach.clone() {
            let function = &self.functions[key];
            if !bindings.contains(key)
                && function.owner.as_deref() != Some("Host")
                && function.file != "convert.rs"
            {
                let file = function.file.clone();
                self.violation(format!(
                    "T3: `{key}` of {file} is reached from a binding: a helper outside convert.rs"
                ));
            }
        }
        self.check_cycles();
    }

    fn check_binding(&mut self, row: &Row, function: &Function) {
        let sig = function.body.sig();
        let mut host_params = 0;
        for input in &sig.inputs {
            let syn::FnArg::Typed(typed) = input else {
                continue;
            };
            if has_numeric(&typed.ty) {
                self.violation(format!(
                    "numeric: binding `{}` has a parameter of a Rust number type",
                    row.path
                ));
            }
            if let syn::Pat::Ident(ident) = &*typed.pat {
                if ident.ident == "host" {
                    host_params += 1;
                }
            }
        }
        let mut uses = HostUses::default();
        uses.visit_block(function.body.block());
        match row.call.as_str() {
            "Frame" => {
                if host_params != 0 || uses.total != 0 {
                    self.violation(format!("T1: read binding `{}` holds a host", row.path));
                }
            }
            "Host" | "Output" => {
                if host_params != 1 {
                    self.violation(format!(
                        "T1: binding `{}` has no single parameter named `host`",
                        row.path
                    ));
                }
                if uses.total != 1 || uses.receivers.len() != 1 {
                    self.violation(format!(
                        "T1: binding `{}` uses `host` {} times, {} as a method receiver",
                        row.path,
                        uses.total,
                        uses.receivers.len()
                    ));
                }
                for method in &uses.receivers {
                    let key = format!("Host::{method}");
                    if self.functions.contains_key(&key) {
                        self.host_calls.insert(format!("{} -> {key}", row.path));
                    } else {
                        self.violation(format!("T1: `host.{method}` is not a Host method"));
                    }
                }
            }
            other => self.violation(format!("T1: unknown call kind `{other}`")),
        }
        let mut shape = Shape::default();
        shape.visit_block(function.body.block());
        if shape.macros > 0 {
            self.violation(format!("T2: binding `{}` invokes a macro", row.path));
        }
        if shape.for_loops > 0 || shape.open_loops > 0 {
            self.violation(format!("T4: binding `{}` has a loop", row.path));
        }
    }

    fn parameter_names(sig: &syn::Signature) -> BTreeSet<String> {
        let mut names = BTreeSet::new();
        for input in &sig.inputs {
            match input {
                syn::FnArg::Receiver(_) => {
                    names.insert("self".to_owned());
                }
                syn::FnArg::Typed(typed) => {
                    let mut idents = PatIdents::default();
                    idents.visit_pat(&typed.pat);
                    names.extend(idents.0);
                }
            }
        }
        names
    }

    /// Check one function of the reach set and return the crate functions
    /// that it names.
    fn scan_function(&mut self, function: &Function, is_binding: bool) -> BTreeSet<String> {
        let key = &function.key;
        let is_host_method = function.owner.as_deref() == Some("Host");
        let sig = function.body.sig().clone();
        let parameters = Self::parameter_names(&sig);
        let mut locals = PatIdents::default();
        locals.visit_block(function.body.block());
        let locals = locals
            .0
            .into_iter()
            .chain(parameters.iter().cloned())
            .collect::<BTreeSet<_>>();
        if !is_binding && !is_host_method {
            for input in &sig.inputs {
                if let syn::FnArg::Typed(typed) = input {
                    let mut idents = Vec::new();
                    collect_type_idents(&typed.ty, &mut idents);
                    if idents.iter().any(|ident| ident == "Host") {
                        self.violation(format!("T4: `{key}` takes the host but is no binding"));
                    }
                }
            }
        }
        let mut shape = Shape::default();
        shape.visit_block(function.body.block());
        if shape.macros > 0 {
            self.violation(format!("T4: `{key}` invokes a macro"));
        }
        if shape.open_loops > 0 {
            self.violation(format!("T4: `{key}` has `while` or `loop`"));
        }
        for iterated in &shape.for_sources {
            if !rooted_at_parameter(iterated, &parameters, ITERATOR_SUFFIXES) {
                self.violation(format!(
                    "T4: `{key}` has a `for` over a value it did not get as a parameter"
                ));
            }
        }
        if shape.open_ranges > 0 {
            self.violation(format!("T4: `{key}` builds an open range"));
        }
        if is_host_method {
            let statements = &function.body.block().stmts;
            if statements.len() != 1 || !matches!(statements[0], syn::Stmt::Expr(_, None)) {
                self.violation(format!("T5: Host method `{key}` is not one expression"));
            }
            if shape.calls != 1 {
                self.violation(format!(
                    "T5: Host method `{key}` makes {} calls",
                    shape.calls
                ));
            }
        }

        let mut refs = References::default();
        refs.visit_block(function.body.block());
        let mut callees = BTreeSet::new();
        for closure_root in &refs.closure_roots {
            match closure_root {
                Some(root) if parameters.contains(root) => {}
                _ => self.violation(format!(
                    "T4: `{key}` has a closure outside a method chain that starts at a parameter"
                )),
            }
        }
        for (method, receiver_root) in &refs.methods {
            if UNBOUNDED_CALLS.contains(&method.as_str()) {
                self.violation(format!("T4: `{key}` calls the unbounded `{method}`"));
            }
            if ITERATOR_SOURCES.contains(&method.as_str())
                && !receiver_root
                    .as_ref()
                    .is_some_and(|root| parameters.contains(root))
            {
                self.violation(format!(
                    "T4: `{key}` starts an iterator chain `{method}` at a value it did not get as a parameter"
                ));
            }
            if is_host_method {
                // A Host method is the one place that calls the run state; T5
                // lists each such call and the seat client call behind it.
                self.core_paths.insert(format!("host {key} -> .{method}"));
                self.record_seat_calls(method);
                continue;
            }
            if receiver_root.as_deref() == Some("host") && is_binding {
                if let Some(keys) = self.methods.get(method) {
                    callees.extend(keys.iter().filter(|key| key.starts_with("Host::")).cloned());
                }
                continue;
            }
            let inside = self
                .methods
                .get(method)
                .map(|keys| {
                    keys.iter()
                        .filter(|key| !key.starts_with("Host::"))
                        .cloned()
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            let host_method = self
                .methods
                .get(method)
                .is_some_and(|keys| keys.iter().any(|key| key.starts_with("Host::")));
            if host_method && inside.is_empty() {
                self.violation(format!("T4: `{key}` calls the Host method `{method}`"));
            }
            if self.outside_methods.contains(method) {
                self.violation(format!(
                    "T3: `{key}` calls `.{method}`, a method that the crate defines outside the binding directory"
                ));
            }
            callees.extend(inside);
        }
        for (path, is_call) in &refs.paths {
            if path.len() == 1 && !is_call && locals.contains(&path[0]) {
                continue;
            }
            self.resolve_path(key, path, *is_call, &mut callees);
        }
        for path in &refs.struct_paths {
            self.resolve_core(key, path);
        }
        for generic in &refs.numeric_turbofish {
            self.violation(format!(
                "numeric: `{key}` reads a Rust number with `{generic}`"
            ));
        }
        // A call of the function itself stays an edge: rule T3 refuses it as
        // a cycle.
        callees
    }

    fn resolve_core(&mut self, key: &str, path: &[String]) {
        let full = match self.imports.get(&path[0]) {
            Some(import) => import
                .iter()
                .cloned()
                .chain(path[1..].iter().cloned())
                .collect(),
            None => path.to_vec(),
        };
        if CORE_CRATES.contains(&full[0].as_str()) {
            self.core_paths
                .insert(format!("{} <- {key}", full.join("::")));
        }
    }

    fn resolve_path(
        &mut self,
        key: &str,
        path: &[String],
        is_call: bool,
        callees: &mut BTreeSet<String>,
    ) {
        if path
            .iter()
            .any(|segment| segment == "FromLua" || segment == "from_lua")
        {
            self.violation(format!(
                "numeric: `{key}` uses the `FromLua` conversion of mlua"
            ));
        }
        // Rule T3: a `use` alias of a crate item outside the binding
        // directory is a call outside it under another name.
        if let Some(import) = self.imports.get(&path[0]) {
            let crate_outside = import.first().is_some_and(|first| first == "crate")
                && import.get(1).is_none_or(|second| second != "bindings");
            let above_bindings = import.starts_with(&["super".to_owned(), "super".to_owned()]);
            if crate_outside || above_bindings {
                self.violation(format!(
                    "T3: `{key}` names `{}`, an import of `{}` outside the binding directory",
                    path.join("::"),
                    import.join("::")
                ));
                return;
            }
        }
        let first = path[0].as_str();
        let rest = match first {
            "crate" | "self" | "super" => &path[1..],
            _ => path,
        };
        if rest.is_empty() {
            return;
        }
        // `convert::name` inside the binding directory.
        if rest.len() == 2 && rest[0] == "convert" {
            match self.free.get(&rest[1]) {
                Some(name) => {
                    callees.insert(name.clone());
                }
                None => self.violation(format!(
                    "T3: `{key}` names `{}` outside convert.rs",
                    rest.join("::")
                )),
            }
            return;
        }
        if rest.len() == 1 {
            let name = &rest[0];
            if let Some(found) = self.free.get(name) {
                callees.insert(found.clone());
            } else if self.outside_free.contains(name) && !self.imports.contains_key(name) {
                self.violation(format!(
                    "T3: `{key}` names `{name}`, a function outside the binding directory"
                ));
            } else if is_call || self.imports.contains_key(name) {
                self.resolve_core(key, rest);
            }
            return;
        }
        let owner = &rest[rest.len() - 2];
        let name = &rest[rest.len() - 1];
        let pair = format!("{owner}::{name}");
        if self.variants.contains(&pair) {
            return;
        }
        if self.types.contains(owner) {
            match self.functions.get(&pair) {
                Some(function) if function.owner.as_deref() != Some("Host") => {
                    callees.insert(pair);
                }
                Some(_) => self.violation(format!("T4: `{key}` names the Host method `{pair}`")),
                None => {
                    // A trait method of a crate type, such as `Out::from`.
                    if let Some(keys) = self.methods.get(name) {
                        callees.extend(
                            keys.iter()
                                .filter(|key| !key.starts_with("Host::"))
                                .cloned(),
                        );
                    }
                }
            }
            return;
        }
        if self.outside_types.contains(owner) && !self.imports.contains_key(owner) {
            self.violation(format!(
                "T3: `{key}` names `{pair}`, a function outside the binding directory"
            ));
            return;
        }
        self.resolve_core(key, rest);
    }

    fn check_cycles(&mut self) {
        fn visit(
            node: &str,
            edges: &BTreeMap<String, BTreeSet<String>>,
            stack: &mut Vec<String>,
            done: &mut BTreeSet<String>,
            cycles: &mut Vec<String>,
        ) {
            if done.contains(node) {
                return;
            }
            if let Some(position) = stack.iter().position(|entry| entry == node) {
                cycles.push(stack[position..].join(" -> "));
                return;
            }
            stack.push(node.to_owned());
            for next in edges.get(node).into_iter().flatten() {
                visit(next, edges, stack, done, cycles);
            }
            stack.pop();
            done.insert(node.to_owned());
        }
        let mut cycles = Vec::new();
        let mut done = BTreeSet::new();
        for node in self.edges.keys() {
            visit(node, &self.edges, &mut Vec::new(), &mut done, &mut cycles);
        }
        for cycle in cycles {
            self.violation(format!("T3: the reach set has a cycle: {cycle}"));
        }
    }
}

/// `x`, `x.field`, `x.method()`, `x.iter().enumerate()` and so on: the
/// identifier at the root of a field and method chain.
fn chain_root(expr: &syn::Expr) -> Option<String> {
    match expr {
        syn::Expr::Path(path) if path.path.segments.len() == 1 => {
            Some(path.path.segments[0].ident.to_string())
        }
        syn::Expr::Field(field) => chain_root(&field.base),
        syn::Expr::MethodCall(call) => chain_root(&call.receiver),
        syn::Expr::Reference(reference) => chain_root(&reference.expr),
        syn::Expr::Paren(paren) => chain_root(&paren.expr),
        syn::Expr::Unary(unary) => chain_root(&unary.expr),
        syn::Expr::Index(index) => chain_root(&index.expr),
        _ => None,
    }
}

/// `parameter`, a field of it, with only the listed adapters after it.
fn rooted_at_parameter(expr: &syn::Expr, parameters: &BTreeSet<String>, adapters: &[&str]) -> bool {
    match expr {
        syn::Expr::Path(path) if path.path.segments.len() == 1 => {
            parameters.contains(&path.path.segments[0].ident.to_string())
        }
        syn::Expr::Field(field) => rooted_at_parameter(&field.base, parameters, &[]),
        syn::Expr::Reference(reference) => {
            rooted_at_parameter(&reference.expr, parameters, adapters)
        }
        syn::Expr::Paren(paren) => rooted_at_parameter(&paren.expr, parameters, adapters),
        syn::Expr::MethodCall(call) => {
            adapters.contains(&call.method.to_string().as_str())
                && call.args.is_empty()
                && rooted_at_parameter(&call.receiver, parameters, adapters)
        }
        _ => false,
    }
}

#[derive(Default)]
struct PatIdents(Vec<String>);

impl<'ast> Visit<'ast> for PatIdents {
    fn visit_pat_ident(&mut self, ident: &'ast syn::PatIdent) {
        self.0.push(ident.ident.to_string());
        syn::visit::visit_pat_ident(self, ident);
    }
}

/// Uses of the identifier `host`.
#[derive(Default)]
struct HostUses {
    total: usize,
    receivers: Vec<String>,
}

impl<'ast> Visit<'ast> for HostUses {
    fn visit_expr_method_call(&mut self, call: &'ast syn::ExprMethodCall) {
        if let syn::Expr::Path(path) = &*call.receiver {
            if path.path.is_ident("host") {
                self.total += 1;
                self.receivers.push(call.method.to_string());
                for argument in &call.args {
                    self.visit_expr(argument);
                }
                return;
            }
        }
        syn::visit::visit_expr_method_call(self, call);
    }

    fn visit_expr_path(&mut self, path: &'ast syn::ExprPath) {
        if path.path.is_ident("host") {
            self.total += 1;
        }
        syn::visit::visit_expr_path(self, path);
    }

    fn visit_expr_closure(&mut self, closure: &'ast syn::ExprClosure) {
        let mut inner = HostUses::default();
        inner.visit_expr(&closure.body);
        // A closure that captures the host is a second holder of it.
        self.total += inner.total * 2;
        self.receivers.extend(inner.receivers);
    }
}

/// Loops, macros, ranges and the call count of one body.
#[derive(Default)]
struct Shape {
    macros: usize,
    open_loops: usize,
    for_loops: usize,
    for_sources: Vec<syn::Expr>,
    open_ranges: usize,
    calls: usize,
}

impl<'ast> Visit<'ast> for Shape {
    fn visit_macro(&mut self, mac: &'ast syn::Macro) {
        self.macros += 1;
        syn::visit::visit_macro(self, mac);
    }

    fn visit_expr_while(&mut self, expr: &'ast syn::ExprWhile) {
        self.open_loops += 1;
        syn::visit::visit_expr_while(self, expr);
    }

    fn visit_expr_loop(&mut self, expr: &'ast syn::ExprLoop) {
        self.open_loops += 1;
        syn::visit::visit_expr_loop(self, expr);
    }

    fn visit_expr_for_loop(&mut self, expr: &'ast syn::ExprForLoop) {
        self.for_loops += 1;
        self.for_sources.push((*expr.expr).clone());
        syn::visit::visit_expr_for_loop(self, expr);
    }

    fn visit_expr_range(&mut self, expr: &'ast syn::ExprRange) {
        if expr.end.is_none() {
            self.open_ranges += 1;
        }
        syn::visit::visit_expr_range(self, expr);
    }

    fn visit_expr_call(&mut self, expr: &'ast syn::ExprCall) {
        self.calls += 1;
        syn::visit::visit_expr_call(self, expr);
    }

    fn visit_expr_method_call(&mut self, expr: &'ast syn::ExprMethodCall) {
        self.calls += 1;
        syn::visit::visit_expr_method_call(self, expr);
    }
}

/// Every name that a body uses: method calls with their receiver root, path
/// calls and function paths used as values, struct paths, closures and
/// numeric turbofish reads.
#[derive(Default)]
struct References {
    methods: Vec<(String, Option<String>)>,
    paths: Vec<(Vec<String>, bool)>,
    struct_paths: Vec<Vec<String>>,
    closure_roots: Vec<Option<String>>,
    numeric_turbofish: Vec<String>,
}

impl<'ast> Visit<'ast> for References {
    fn visit_expr_method_call(&mut self, call: &'ast syn::ExprMethodCall) {
        self.methods
            .push((call.method.to_string(), chain_root(&call.receiver)));
        if let Some(turbofish) = &call.turbofish {
            for argument in &turbofish.args {
                if let syn::GenericArgument::Type(ty) = argument {
                    if has_numeric(ty) {
                        self.numeric_turbofish
                            .push(format!("{}::<..>", call.method));
                    }
                }
            }
        }
        let root = chain_root(&call.receiver);
        for argument in &call.args {
            if matches!(argument, syn::Expr::Closure(_)) {
                self.closure_roots.push(root.clone());
                if let syn::Expr::Closure(closure) = argument {
                    self.visit_expr(&closure.body);
                }
            } else {
                self.visit_expr(argument);
            }
        }
        self.visit_expr(&call.receiver);
    }

    fn visit_expr_call(&mut self, call: &'ast syn::ExprCall) {
        if let syn::Expr::Path(path) = &*call.func {
            self.paths.push((path_idents(&path.path), true));
            for segment in &path.path.segments {
                if let syn::PathArguments::AngleBracketed(arguments) = &segment.arguments {
                    for argument in &arguments.args {
                        if let syn::GenericArgument::Type(ty) = argument {
                            if has_numeric(ty) {
                                self.numeric_turbofish
                                    .push(path_idents(&path.path).join("::"));
                            }
                        }
                    }
                }
            }
            for argument in &call.args {
                self.visit_argument(argument);
            }
            return;
        }
        syn::visit::visit_expr_call(self, call);
    }

    fn visit_expr_path(&mut self, path: &'ast syn::ExprPath) {
        self.paths.push((path_idents(&path.path), false));
    }

    fn visit_expr_struct(&mut self, expr: &'ast syn::ExprStruct) {
        self.struct_paths.push(path_idents(&expr.path));
        for field in &expr.fields {
            self.visit_expr(&field.expr);
        }
    }

    fn visit_expr_closure(&mut self, closure: &'ast syn::ExprClosure) {
        self.closure_roots.push(None);
        self.visit_expr(&closure.body);
    }
}

impl References {
    fn visit_argument(&mut self, argument: &syn::Expr) {
        if let syn::Expr::Closure(closure) = argument {
            // A closure passed to a path call is never rooted at a parameter.
            self.closure_roots.push(None);
            self.visit_expr(&closure.body);
        } else {
            self.visit_expr(argument);
        }
    }
}

// ---------------------------------------------------------------------------
// Crate-wide rules: C9 registration, T7, T11

/// Rule C9 (ADR 0023 item 30) and the numeric registration of round 5: a
/// closure that the crate gives to `mlua` returns `Ok` on every path and
/// takes no Rust number.
#[derive(Default)]
struct Registrations {
    file: String,
    count: usize,
    violations: Vec<String>,
}

fn ok_call(expr: &syn::Expr) -> bool {
    match expr {
        syn::Expr::Call(call) => {
            matches!(&*call.func, syn::Expr::Path(path) if path.path.is_ident("Ok"))
        }
        syn::Expr::Block(block) => block_ends_ok(&block.block),
        syn::Expr::Match(expr) => expr.arms.iter().all(|arm| ok_call(&arm.body)),
        syn::Expr::If(expr) => {
            block_ends_ok(&expr.then_branch)
                && expr
                    .else_branch
                    .as_ref()
                    .is_some_and(|(_, branch)| ok_call(branch))
        }
        syn::Expr::Return(expr) => expr.expr.as_deref().is_some_and(ok_call),
        _ => false,
    }
}

fn block_ends_ok(block: &syn::Block) -> bool {
    match block.stmts.last() {
        Some(syn::Stmt::Expr(expr, None)) => ok_call(expr),
        Some(syn::Stmt::Expr(syn::Expr::Return(expr), Some(_))) => {
            expr.expr.as_deref().is_some_and(ok_call)
        }
        _ => false,
    }
}

/// `?` and `return` of the closure body itself, not of nested closures.
#[derive(Default)]
struct ClosureExits {
    tries: usize,
    bad_returns: usize,
}

impl<'ast> Visit<'ast> for ClosureExits {
    fn visit_expr_try(&mut self, expr: &'ast syn::ExprTry) {
        self.tries += 1;
        syn::visit::visit_expr_try(self, expr);
    }

    fn visit_expr_return(&mut self, expr: &'ast syn::ExprReturn) {
        if !expr.expr.as_deref().is_some_and(ok_call) {
            self.bad_returns += 1;
        }
        syn::visit::visit_expr_return(self, expr);
    }

    fn visit_expr_closure(&mut self, _closure: &'ast syn::ExprClosure) {}
}

impl<'ast> Visit<'ast> for Registrations {
    fn visit_expr_method_call(&mut self, call: &'ast syn::ExprMethodCall) {
        if VM_REGISTRATIONS.contains(&call.method.to_string().as_str()) {
            self.count += 1;
            match call.args.first() {
                Some(syn::Expr::Closure(closure)) => {
                    for input in &closure.inputs {
                        if let syn::Pat::Type(typed) = input {
                            if has_numeric(&typed.ty) {
                                self.violations.push(format!(
                                    "numeric: {}: a registration takes a Rust number",
                                    self.file
                                ));
                            }
                        }
                    }
                    let mut exits = ClosureExits::default();
                    exits.visit_expr(&closure.body);
                    let ends_ok = match &*closure.body {
                        syn::Expr::Block(block) => block_ends_ok(&block.block),
                        other => ok_call(other),
                    };
                    if exits.tries > 0 || exits.bad_returns > 0 || !ends_ok {
                        self.violations.push(format!(
                            "C9: {}: a function given to mlua can return an error",
                            self.file
                        ));
                    }
                }
                _ => self.violations.push(format!(
                    "C9: {}: a registration takes no closure literal",
                    self.file
                )),
            }
        }
        syn::visit::visit_expr_method_call(self, call);
    }
}

fn registration_violations(files: &[SourceFile]) -> (usize, Vec<String>) {
    let mut count = 0;
    let mut violations = Vec::new();
    for file in files {
        let mut registrations = Registrations {
            file: file.name.clone(),
            ..Default::default()
        };
        registrations.visit_file(&file.ast);
        count += registrations.count;
        violations.extend(registrations.violations);
    }
    (count, violations)
}

/// Rule T7: no machine. Rule T11: `unsafe` only in the FFI module.
#[derive(Default)]
struct CrateShape {
    file: String,
    violations: Vec<String>,
}

impl<'ast> Visit<'ast> for CrateShape {
    fn visit_item_impl(&mut self, item: &'ast syn::ItemImpl) {
        if let Some((_, path, _)) = &item.trait_ {
            let name = path
                .segments
                .last()
                .map(|segment| segment.ident.to_string());
            if matches!(name.as_deref(), Some("NativeMachine" | "Script")) {
                self.violations
                    .push(format!("T7: {} implements a machine trait", self.file));
            }
        }
        if item.unsafety.is_some() {
            self.unsafe_token();
        }
        syn::visit::visit_item_impl(self, item);
    }

    fn visit_expr_unsafe(&mut self, expr: &'ast syn::ExprUnsafe) {
        self.unsafe_token();
        syn::visit::visit_expr_unsafe(self, expr);
    }

    fn visit_signature(&mut self, sig: &'ast syn::Signature) {
        if sig.unsafety.is_some() {
            self.unsafe_token();
        }
        syn::visit::visit_signature(self, sig);
    }

    fn visit_item_trait(&mut self, item: &'ast syn::ItemTrait) {
        if item.unsafety.is_some() {
            self.unsafe_token();
        }
        syn::visit::visit_item_trait(self, item);
    }

    fn visit_item_foreign_mod(&mut self, item: &'ast syn::ItemForeignMod) {
        self.unsafe_token();
        syn::visit::visit_item_foreign_mod(self, item);
    }

    fn visit_attribute(&mut self, attribute: &'ast syn::Attribute) {
        let text = attribute.meta.to_token_stream_string();
        if (text.contains("allow") || text.contains("expect")) && text.contains("unsafe_code") {
            self.violations
                .push(format!("T11: {} lowers the unsafe_code lint", self.file));
        }
    }
}

trait TokenText {
    fn to_token_stream_string(&self) -> String;
}

impl TokenText for syn::Meta {
    fn to_token_stream_string(&self) -> String {
        let mut text = String::new();
        match self {
            syn::Meta::Path(path) => text.push_str(&path_idents(path).join("::")),
            syn::Meta::List(list) => {
                text.push_str(&path_idents(&list.path).join("::"));
                text.push('(');
                text.push_str(&list.tokens.to_string());
                text.push(')');
            }
            syn::Meta::NameValue(value) => text.push_str(&path_idents(&value.path).join("::")),
        }
        text
    }
}

impl CrateShape {
    fn unsafe_token(&mut self) {
        if self.file != "single_step.rs" {
            self.violations.push(format!(
                "T11: {} has `unsafe` outside the FFI module",
                self.file
            ));
        }
    }
}

fn crate_shape_violations(files: &[SourceFile]) -> Vec<String> {
    let mut violations = Vec::new();
    for file in files {
        let mut shape = CrateShape {
            file: file.name.clone(),
            ..Default::default()
        };
        if file.name == "lib.rs" {
            // The one recorded exception: `#[expect(unsafe_code)]` on
            // `mod single_step` (ADR 0023 item 15).
            for item in &file.ast.items {
                match item {
                    syn::Item::Mod(module) if module.ident == "single_step" => {}
                    other => shape.visit_item(other),
                }
            }
            for attribute in &file.ast.attrs {
                shape.visit_attribute(attribute);
            }
        } else {
            shape.visit_file(&file.ast);
        }
        violations.extend(shape.violations);
    }
    violations
}

/// Rule T9: the prelude is the crate's only Luau source. Code enters the VM
/// through `.load` and `.compile`. Their only arguments are the prelude, the
/// card's source and the bytecode compiled from it; no file is included.
#[derive(Default)]
struct LuauSources {
    file: String,
    prelude_loads: usize,
    prelude_consts: usize,
    violations: Vec<String>,
}

fn without_reference(expr: &syn::Expr) -> &syn::Expr {
    match expr {
        syn::Expr::Reference(reference) => without_reference(&reference.expr),
        syn::Expr::Paren(paren) => without_reference(&paren.expr),
        other => other,
    }
}

impl<'ast> Visit<'ast> for LuauSources {
    fn visit_item_mod(&mut self, module: &'ast syn::ItemMod) {
        // A `#[cfg(test)]` module is no product source: its chunks never
        // reach a run.
        let test_only = module.attrs.iter().any(|attribute| {
            attribute.meta.to_token_stream_string().replace(' ', "") == "cfg(test)"
        });
        if !test_only {
            syn::visit::visit_item_mod(self, module);
        }
    }

    fn visit_macro(&mut self, mac: &'ast syn::Macro) {
        if mac.path.is_ident("include_str") || mac.path.is_ident("include_bytes") {
            self.violations.push(format!(
                "T9: {} includes a file with `{}!`",
                self.file,
                path_idents(&mac.path).join("::")
            ));
        }
        syn::visit::visit_macro(self, mac);
    }

    fn visit_item_const(&mut self, item: &'ast syn::ItemConst) {
        if item.ident == "PRELUDE" {
            self.prelude_consts += 1;
            if self.file != "prelude.rs" {
                self.violations
                    .push(format!("T9: {} defines a second `PRELUDE`", self.file));
            }
        }
        syn::visit::visit_item_const(self, item);
    }

    fn visit_expr_method_call(&mut self, call: &'ast syn::ExprMethodCall) {
        let method = call.method.to_string();
        if method == "load" || method == "compile" {
            let argument = call.args.first().map(without_reference);
            let allowed = match (method.as_str(), argument) {
                ("load", Some(syn::Expr::Path(path))) if path.path.is_ident("PRELUDE") => {
                    self.prelude_loads += 1;
                    true
                }
                // The bytecode of the card's source, or an atomic load.
                ("load", Some(syn::Expr::Path(path))) => {
                    path.path.is_ident("bytecode")
                        || path
                            .path
                            .segments
                            .first()
                            .is_some_and(|segment| segment.ident == "Ordering")
                }
                ("compile", Some(argument)) => chain_root(argument).as_deref() == Some("card"),
                _ => false,
            };
            if !allowed {
                self.violations.push(format!(
                    "T9: {} gives `.{method}` a source other than the prelude or the card",
                    self.file
                ));
            }
        }
        syn::visit::visit_expr_method_call(self, call);
    }
}

fn luau_source_violations(files: &[SourceFile]) -> Vec<String> {
    let mut violations = Vec::new();
    let mut prelude_loads = 0;
    let mut prelude_consts = 0;
    for file in files {
        let mut sources = LuauSources {
            file: file.name.clone(),
            ..Default::default()
        };
        sources.visit_file(&file.ast);
        prelude_loads += sources.prelude_loads;
        prelude_consts += sources.prelude_consts;
        violations.extend(sources.violations);
    }
    if prelude_consts != 1 || prelude_loads != 1 {
        violations.push(format!(
            "T9: the crate defines `PRELUDE` {prelude_consts} times and loads it {prelude_loads} times; each must be once"
        ));
    }
    violations
}

// ---------------------------------------------------------------------------
// Running the census on the crate

struct Sources {
    bindings: Vec<SourceFile>,
    outside: Vec<SourceFile>,
}

fn sources_with(edit: impl Fn(&str, String) -> String) -> Sources {
    let mut bindings = Vec::new();
    let mut outside = Vec::new();
    for (name, text) in crate_sources() {
        if let Some(file) = name.strip_prefix("bindings/") {
            bindings.push(parse(file, edit(file, text)));
        } else {
            outside.push(parse(&name, edit(&name, text)));
        }
    }
    assert_eq!(
        bindings
            .iter()
            .map(|file| file.name.as_str())
            .collect::<BTreeSet<_>>(),
        BINDING_FILES.iter().copied().collect::<BTreeSet<_>>(),
        "the binding directory has exactly mod.rs, host.rs and convert.rs"
    );
    Sources { bindings, outside }
}

fn run_census(sources: &Sources) -> Census {
    let mut census = Census::build(&sources.bindings, &sources.outside);
    census.check();
    let all = sources
        .bindings
        .iter()
        .chain(&sources.outside)
        .map(|file| parse(&file.name, file.text.clone()))
        .collect::<Vec<_>>();
    let (_, registrations) = registration_violations(&all);
    census.violations.extend(registrations);
    census.violations.extend(luau_source_violations(&all));
    census
}

fn checked_in(name: &str) -> PathBuf {
    crate_dir().join("tests/thinness").join(name)
}

fn bless() -> bool {
    std::env::var_os("THINNESS_BLESS").is_some()
}

fn compare_or_bless(name: &str, actual: &str) {
    let path = checked_in(name);
    if bless() {
        std::fs::create_dir_all(checked_in("")).expect("the census directory is writable");
        std::fs::write(&path, actual).expect("the census file is writable");
        return;
    }
    let expected = read(&path);
    assert_eq!(
        expected,
        actual,
        "{} is stale; review the change and run with THINNESS_BLESS=1",
        path.display()
    );
}

#[test]
fn the_bindings_pass_the_census() {
    let sources = sources_with(|_, text| text);
    let census = run_census(&sources);
    assert!(
        census.violations.is_empty(),
        "thinness violations:\n{}",
        census.violations.join("\n")
    );
    assert_eq!(census.rows.len(), script_luau::api_members().count());
    for (row, member) in census.rows.iter().zip(script_luau::api_members()) {
        assert_eq!(
            row.path, member.path,
            "the parsed table is the compiled table"
        );
        assert_eq!(row.js.as_deref(), member.js);
    }

    // Rule T5: each core path and each host call is a visible line.
    let mut lines = census.host_calls.iter().cloned().collect::<Vec<_>>();
    lines.extend(census.core_paths.iter().cloned());
    let mut text = String::from(
        "# Generated by tests/thinness.rs (rule T5). Each line is a call that a binding\n\
         # can reach into `script`, `api` or `vault`, or a call of a Host method.\n",
    );
    for line in &lines {
        text.push_str(line);
        text.push('\n');
    }
    compare_or_bless("core-paths.txt", &text);

    println!(
        "thinness: {} rows, {} functions in the reach set",
        census.rows.len(),
        census.reach.len()
    );
    for key in &census.reach {
        println!("thinness: reach {key}");
    }
}

#[derive(Clone, Debug)]
struct LiveCell {
    command: String,
    covers_luau_source: bool,
}

fn live_cell(map: &str, cell_id: &str) -> Result<LiveCell, String> {
    let document = map
        .parse::<toml_edit::DocumentMut>()
        .map_err(|error| format!("tools/live-map/map.toml is invalid TOML: {error}"))?;
    let covers = document
        .get("cover")
        .and_then(|item| item.as_array_of_tables())
        .ok_or_else(|| format!("`{cell_id}` does not name a cell of tools/live-map/map.toml"))?;
    let mut cell: Option<LiveCell> = None;

    for cover in covers.iter() {
        let covers_luau_source = cover
            .get("globs")
            .and_then(|item| item.as_array())
            .is_some_and(|globs| {
                globs
                    .iter()
                    .any(|glob| glob.as_str() == Some("crates/script-luau/src/**"))
            });
        let Some(cells) = cover.get("cells").and_then(|item| item.as_array()) else {
            continue;
        };
        for spec in cells.iter().filter_map(|value| value.as_str()) {
            let Some((name, fields)) = spec.split_once('|') else {
                continue;
            };
            if name.trim() != cell_id {
                continue;
            }
            let command = fields.split('|').next().unwrap_or_default().trim();
            if let Some(previous) = &mut cell {
                if previous.command != command {
                    return Err(format!(
                        "cell id `{cell_id}` has conflicting commands in tools/live-map/map.toml"
                    ));
                }
                previous.covers_luau_source |= covers_luau_source;
            } else {
                cell = Some(LiveCell {
                    command: command.to_owned(),
                    covers_luau_source,
                });
            }
        }
    }

    cell.ok_or_else(|| format!("`{cell_id}` does not name a cell of tools/live-map/map.toml"))
}

/// Luau literals and file-local function/constant references in one reachable body.
struct LiveTestReach<'a> {
    invocation: &'a str,
    found: bool,
    references: BTreeSet<String>,
}

impl LiveTestReach<'_> {
    fn visit_macro_arguments(&mut self, mac: &syn::Macro) {
        use syn::parse::Parser;

        // format! and the assertion macros take comma-separated expressions.
        // Their tokens are not visited by syn's default visitor.
        let parser = syn::punctuated::Punctuated::<syn::Expr, syn::Token![,]>::parse_terminated;
        if let Ok(arguments) = parser.parse2(mac.tokens.clone()) {
            for argument in &arguments {
                self.visit_expr(argument);
            }
        }
    }
}

impl<'ast> Visit<'ast> for LiveTestReach<'_> {
    fn visit_lit_str(&mut self, literal: &'ast syn::LitStr) {
        if !self.found {
            self.found = literal.value().contains(self.invocation);
        }
    }

    fn visit_expr_path(&mut self, path: &'ast syn::ExprPath) {
        if let Some(name) = path.path.get_ident() {
            self.references.insert(name.to_string());
        }
        syn::visit::visit_expr_path(self, path);
    }

    fn visit_macro(&mut self, mac: &'ast syn::Macro) {
        self.visit_macro_arguments(mac);
    }

    fn visit_item(&mut self, _item: &'ast syn::Item) {
        // A nested declaration does not run merely because its parent runs.
    }
}

fn selected_test_reaches(source: &str, test_name: &str, invocation: &str) -> Result<bool, String> {
    let file =
        syn::parse_file(source).map_err(|error| format!("test source is invalid Rust: {error}"))?;
    let items = file
        .items
        .iter()
        .filter_map(|item| match item {
            syn::Item::Fn(function) => Some((function.sig.ident.to_string(), item)),
            syn::Item::Const(constant) => Some((constant.ident.to_string(), item)),
            _ => None,
        })
        .collect::<BTreeMap<_, _>>();
    let Some(syn::Item::Fn(test)) = items.get(test_name).copied() else {
        return Err(format!("test function `{test_name}` is missing"));
    };
    if !test
        .attrs
        .iter()
        .any(|attribute| attribute.path().is_ident("test"))
    {
        return Err(format!("function `{test_name}` has no `#[test]`"));
    }

    let mut pending = vec![test_name.to_owned()];
    let mut visited = BTreeSet::new();
    while let Some(name) = pending.pop() {
        if !visited.insert(name.clone()) {
            continue;
        }
        let Some(item) = items.get(&name) else {
            continue;
        };
        let mut reach = LiveTestReach {
            invocation,
            found: false,
            references: BTreeSet::new(),
        };
        match item {
            syn::Item::Fn(function) => reach.visit_block(&function.block),
            syn::Item::Const(constant) => reach.visit_expr(&constant.expr),
            _ => unreachable!("only functions and constants were indexed"),
        }
        if reach.found {
            return Ok(true);
        }
        pending.extend(reach.references);
    }
    Ok(false)
}

fn exception_cell_error(
    path: &str,
    cell_id: &str,
    map: &str,
    mut read_test_file: impl FnMut(&str) -> Option<String>,
) -> Result<(), String> {
    let cell = live_cell(map, cell_id)?;
    if cell.command.split_whitespace().next() != Some("LIVE=1") {
        return Err(format!(
            "cell `{cell_id}` command does not start with `LIVE=1`"
        ));
    }
    if !cell.covers_luau_source {
        return Err(format!(
            "cell `{cell_id}` cover does not include `crates/script-luau/src/**`"
        ));
    }

    let arguments = cell.command.split_whitespace().collect::<Vec<_>>();
    let mut test_names = arguments
        .windows(2)
        .filter_map(|pair| (pair[0] == "--test").then_some(pair[1]));
    let test_name = test_names
        .next()
        .ok_or_else(|| format!("cell `{cell_id}` command has no `--test <name>` argument"))?;
    if test_names.next().is_some() {
        return Err(format!("cell `{cell_id}` has more than one `--test`"));
    }
    if !test_name
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || matches!(character, '_' | '-'))
    {
        return Err(format!(
            "cell `{cell_id}` has an invalid `--test` name `{test_name}`"
        ));
    }
    let mut exact_arguments = arguments
        .iter()
        .enumerate()
        .filter(|(_, argument)| **argument == "--exact");
    let (exact_index, _) = exact_arguments
        .next()
        .ok_or_else(|| format!("cell `{cell_id}` has no `--exact <name>` argument"))?;
    if exact_arguments.next().is_some() {
        return Err(format!("cell `{cell_id}` has more than one `--exact`"));
    }
    let function_name = arguments
        .get(exact_index + 1)
        .filter(|name| {
            name.chars()
                .all(|character| character.is_ascii_alphanumeric() || character == '_')
        })
        .ok_or_else(|| format!("cell `{cell_id}` has no `--exact <name>` argument"))?;
    let test_source = read_test_file(test_name)
        .ok_or_else(|| format!("cell `{cell_id}` test file `{test_name}.rs` is missing"))?;
    let invocation = format!("api.{path}(");
    if !selected_test_reaches(&test_source, function_name, &invocation)
        .map_err(|error| format!("cell `{cell_id}` {error}"))?
    {
        return Err(format!(
            "cell `{cell_id}` test `{function_name}` does not reach `{invocation}`"
        ));
    }
    Ok(())
}

fn parse_exceptions(text: &str) -> BTreeMap<String, String> {
    let mut exceptions = BTreeMap::new();
    for line in text.lines().map(str::trim) {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (path, cell_id) = line
            .split_once(':')
            .unwrap_or_else(|| panic!("exception line `{line}` must be `path: live cell id`"));
        let path = path.trim();
        let cell_id = cell_id.trim();
        assert!(!path.is_empty(), "exception line `{line}` has no path");
        assert!(
            !cell_id.is_empty(),
            "exception `{path}` has no live cell id"
        );
        assert!(
            exceptions
                .insert(path.to_owned(), cell_id.to_owned())
                .is_none(),
            "exception `{path}` is listed more than once"
        );
    }
    exceptions
}

#[test]
fn every_request_has_a_js_v2_counterpart_or_a_written_exception() {
    let sources = sources_with(|_, text| text);
    let census = Census::build(&sources.bindings, &sources.outside);
    let exceptions = parse_exceptions(&read(&checked_in("exceptions.txt")));
    let types = read(&crate_dir().join("../script-js/host-js/index.d.ts"));
    let live_map = read(&crate_dir().join("../../tools/live-map/map.toml"));
    let host_play_tests = crate_dir().join("../host-play/tests");
    for row in &census.rows {
        match &row.js {
            Some(js) => assert!(
                js_member_exists(&types, js),
                "rule T6: `{}` names `{js}`, which host-js/index.d.ts does not declare",
                row.path
            ),
            None => assert!(
                row.kind != "Request" && row.kind != "Await" || exceptions.contains_key(&row.path),
                "rule T6: `{}` has no JS v2 counterpart and no written exception",
                row.path
            ),
        }
    }
    for (path, cell_id) in &exceptions {
        assert!(
            census
                .rows
                .iter()
                .any(|row| &row.path == path && row.js.is_none()),
            "exception `{path}` names no row without a counterpart"
        );
        exception_cell_error(path, cell_id, &live_map, |test_name| {
            std::fs::read_to_string(host_play_tests.join(format!("{test_name}.rs"))).ok()
        })
        .unwrap_or_else(|error| {
            panic!("rule T6: exception `{path}` has an invalid live cell `{cell_id}`: {error}")
        });
    }
}

fn live_map_fixture(command: &str, glob: &str) -> String {
    format!("[[cover]]\nglobs = [\"{glob}\"]\ncells = [\"host.luau-lifecycle|{command}\"]\n")
}

#[test]
fn every_t6_cell_requirement_fails_closed_in_negative_fixtures() {
    let fixtures = [
        (
            "an id absent from the live map",
            "host.missing",
            "LIVE=1 cargo test --test script_api_live",
            "crates/script-luau/src/**",
            "api.wait()",
            "does not name a cell",
        ),
        (
            "a cell that is not live",
            "host.luau-lifecycle",
            "cargo test --test script_api_live",
            "crates/script-luau/src/**",
            "api.wait()",
            "does not start with `LIVE=1`",
        ),
        (
            "a cell without the required source cover",
            "host.luau-lifecycle",
            "LIVE=1 cargo test --test script_api_live",
            "crates/host-play/src/**",
            "api.wait()",
            "does not include `crates/script-luau/src/**`",
        ),
        (
            "a test file without the API call",
            "host.luau-lifecycle",
            "LIVE=1 cargo test --test script_api_live -- --exact no_wait_call",
            "crates/script-luau/src/**",
            "#[test] fn no_wait_call() {}",
            "test `no_wait_call` does not reach `api.wait(`",
        ),
    ];
    for (name, cell_id, command, glob, test_source, expected) in fixtures {
        let map = live_map_fixture(command, glob);
        let error = exception_cell_error("wait", cell_id, &map, |_| Some(test_source.to_owned()))
            .expect_err(name);
        assert!(error.contains(expected), "{name}: {error}");
    }
}

#[test]
fn t6_refuses_a_binding_found_only_in_another_cell() {
    let map = read(&crate_dir().join("../../tools/live-map/map.toml"));
    let tests = crate_dir().join("../host-play/tests");
    let error = exception_cell_error("interact.held", "host.luau-walk", &map, |name| {
        std::fs::read_to_string(tests.join(format!("{name}.rs"))).ok()
    })
    .expect_err("the walk cell cannot witness the held binding in its sibling cell");
    assert!(error.contains("test `luau_walk` does not reach"), "{error}");
}

#[test]
fn t6_follows_only_the_selected_tests_helpers_and_sources() {
    let command = "LIVE=1 cargo test --test script_api_live -- --ignored --exact selected";
    let map = live_map_fixture(command, "crates/script-luau/src/**");
    for source in [
        r#"#[test] fn selected() { let source = "api.wait()"; }"#,
        r#"const SOURCE: &str = "api.wait()";
           fn body() { run(SOURCE); }
           fn capture(body: fn()) { body(); }
           #[test] fn selected() { capture(body); }"#,
        r#"fn body() { run(format!("api.wait()")); }
           #[test] fn selected() { body(); }"#,
    ] {
        exception_cell_error("wait", "host.luau-lifecycle", &map, |_| {
            Some(source.to_owned())
        })
        .unwrap_or_else(|error| panic!("reachable Luau source must count: {error}"));
    }
    for source in [
        r#"#[test] fn selected() {} #[test] fn sibling() { run("api.wait()"); }"#,
        r#"#[test] fn selected() { chosen(); }
           fn chosen() {} fn unused() { run("api.wait()"); }"#,
        r#"const UNUSED: &str = "api.wait()"; #[test] fn selected() {}"#,
        "#[test] fn selected() { /* api.wait() */ }",
    ] {
        let error = exception_cell_error("wait", "host.luau-lifecycle", &map, |_| {
            Some(source.to_owned())
        })
        .expect_err("unreachable or commented Luau source cannot witness the selected test");
        assert!(error.contains("test `selected` does not reach"), "{error}");
    }
}

#[test]
fn t6_requires_one_exact_existing_test_function() {
    for (command, source, expected) in [
        (
            "LIVE=1 cargo test --test script_api_live",
            "#[test] fn selected() {}",
            "has no `--exact <name>`",
        ),
        (
            "LIVE=1 cargo test --test script_api_live -- --exact selected --exact sibling",
            "#[test] fn selected() {}",
            "has more than one `--exact`",
        ),
        (
            "LIVE=1 cargo test --test script_api_live -- --exact missing",
            "#[test] fn selected() {}",
            "test function `missing` is missing",
        ),
        (
            "LIVE=1 cargo test --test script_api_live -- --exact selected",
            "fn selected() { run(\"api.wait()\"); }",
            "function `selected` has no `#[test]`",
        ),
    ] {
        let map = live_map_fixture(command, "crates/script-luau/src/**");
        let error = exception_cell_error("wait", "host.luau-lifecycle", &map, |_| {
            Some(source.to_owned())
        })
        .expect_err("the command must select one real test");
        assert!(error.contains(expected), "{error}");
    }
}

#[test]
fn commented_or_out_of_array_map_values_do_not_count() {
    let fixtures = [
        (
            "a commented-out cell",
            r#"
[[cover]]
globs = ["crates/script-luau/src/**"]
cells = [
    # "host.luau-lifecycle|LIVE=1 cargo test --test script_api_live",
]
"#,
            "does not name a cell",
        ),
        (
            "a commented-out glob",
            r#"
[[cover]]
# globs = ["crates/script-luau/src/**"]
globs = ["crates/host-play/src/**"]
cells = ["host.luau-lifecycle|LIVE=1 cargo test --test script_api_live"]
"#,
            "does not include `crates/script-luau/src/**`",
        ),
        (
            "cell text outside the cells array",
            r#"
[[cover]]
globs = ["crates/script-luau/src/**"]
cells = []
other = ["host.luau-lifecycle|LIVE=1 cargo test --test script_api_live"]
"#,
            "does not name a cell",
        ),
        (
            "glob text outside the globs array",
            r#"
[[cover]]
globs = ["crates/host-play/src/**"]
extra_globs = ["crates/script-luau/src/**"]
cells = ["host.luau-lifecycle|LIVE=1 cargo test --test script_api_live"]
"#,
            "does not include `crates/script-luau/src/**`",
        ),
    ];
    for (name, map, expected) in fixtures {
        let error = exception_cell_error("wait", "host.luau-lifecycle", map, |_| {
            Some("api.wait()".to_owned())
        })
        .expect_err(name);
        assert!(error.contains(expected), "{name}: {error}");
    }
}

#[test]
fn repeated_live_cell_with_identical_command_is_accepted() {
    let command = "LIVE=1 cargo test --test script_api_live -- --exact selected";
    let map = format!(
        "{}{}",
        live_map_fixture(command, "crates/host-play/src/**"),
        live_map_fixture(command, "crates/script-luau/src/**")
    );
    exception_cell_error("wait", "host.luau-lifecycle", &map, |_| {
        Some("#[test] fn selected() { run(\"api.wait()\"); }".to_owned())
    })
    .unwrap_or_else(|error| panic!("identical cell commands should be accepted: {error}"));
}

#[test]
fn repeated_live_cell_with_conflicting_command_is_refused() {
    let map = format!(
        "{}{}",
        live_map_fixture(
            "LIVE=1 cargo test --test script_api_live",
            "crates/script-luau/src/**"
        ),
        live_map_fixture(
            "LIVE=1 cargo test --test script_api_live -- --ignored",
            "crates/script-luau/src/**"
        )
    );
    let error = live_cell(&map, "host.luau-lifecycle")
        .expect_err("a duplicated cell id with different commands must be refused");
    assert!(error.contains("conflicting commands"), "{error}");
}

/// `Interface.member` is a member line of that interface block;
/// `NativeOp:op` is one `op: '<op>'` arm.
fn js_member_exists(types: &str, member: &str) -> bool {
    if let Some(op) = member.strip_prefix("NativeOp:") {
        return types.contains(&format!("{{ op: '{op}';"))
            || types.contains(&format!("{{ op: '{op}'}}"));
    }
    let Some((interface, name)) = member.split_once('.') else {
        return false;
    };
    let header = format!("export interface {interface} {{");
    let Some(start) = types.find(&header) else {
        return false;
    };
    let block = &types[start + header.len()..];
    let block = &block[..block.find("\n}").unwrap_or(block.len())];
    block.lines().any(|line| {
        let line = line.trim_start().trim_start_matches("readonly ");
        line.starts_with(&format!("{name}:"))
            || line.starts_with(&format!("{name}?:"))
            || line.starts_with(&format!("{name}("))
    })
}

#[test]
fn the_crate_has_no_machine_and_no_other_unsafe() {
    let sources = sources_with(|_, text| text);
    let mut all = Vec::new();
    for file in sources.bindings.iter().chain(&sources.outside) {
        all.push(parse(&file.name, file.text.clone()));
    }
    let violations = crate_shape_violations(&all);
    assert!(violations.is_empty(), "{}", violations.join("\n"));
    let (registrations, violations) = registration_violations(&all);
    assert!(violations.is_empty(), "{}", violations.join("\n"));
    assert!(
        registrations >= 1,
        "the census sees the registration function"
    );
}

#[test]
fn the_prelude_is_the_only_luau_source() {
    fn other_files(dir: &Path, out: &mut Vec<PathBuf>) {
        for entry in std::fs::read_dir(dir).expect("the crate's source directory is readable") {
            let path = entry.expect("a source directory entry is readable").path();
            if path.is_dir() {
                other_files(&path, out);
            } else if path.extension().is_none_or(|extension| extension != "rs") {
                out.push(path);
            }
        }
    }
    let mut others = Vec::new();
    other_files(&crate_dir().join("src"), &mut others);
    assert!(
        others.is_empty(),
        "rule T9: src holds files that are not Rust: {others:?}"
    );
    let sources = sources_with(|_, text| text);
    let all = sources
        .bindings
        .iter()
        .chain(&sources.outside)
        .map(|file| parse(&file.name, file.text.clone()))
        .collect::<Vec<_>>();
    let violations = luau_source_violations(&all);
    assert!(violations.is_empty(), "{}", violations.join("\n"));
}

#[test]
fn the_prelude_names_no_api_member() {
    let prelude = read(&crate_dir().join("src/prelude.rs"));
    assert!(
        !prelude.contains("api."),
        "rule T9: the prelude names the API table"
    );
    // The prelude may use a group name as an ordinary word (`status` is a
    // local of the wrappers); it must never index a table by it.
    for member in script_luau::api_members() {
        if let Some((group, _)) = member.path.split_once('.') {
            assert!(
                !prelude.contains(member.path),
                "rule T9: the prelude names `{}`",
                member.path
            );
            for access in [
                format!("\"{group}\""),
                format!("'{group}'"),
                format!("[{group}]"),
            ] {
                assert!(
                    !prelude.contains(&access),
                    "rule T9: the prelude indexes the group `{group}`"
                );
            }
            let dotted = prelude
                .match_indices(&format!(".{group}"))
                .any(|(at, text)| {
                    !prelude[at + text.len()..]
                        .starts_with(|c: char| c.is_alphanumeric() || c == '_')
                });
            assert!(!dotted, "rule T9: the prelude indexes the group `{group}`");
        }
    }
}

#[test]
fn the_size_stays_within_its_baseline() {
    let sources = sources_with(|_, text| text);
    let census = Census::build(&sources.bindings, &sources.outside);
    let file = |name: &str| {
        sources
            .bindings
            .iter()
            .find(|file| file.name == name)
            .map(|file| file.text.lines().count())
            .expect("the binding directory has this file")
    };
    let binding_lines = census
        .rows
        .iter()
        .map(|row| {
            let function = &census.functions[&row.function];
            let text = &sources
                .bindings
                .iter()
                .find(|file| file.name == function.file)
                .expect("a row's function lives in the binding directory")
                .text;
            function_lines(text, &row.function)
        })
        .collect::<Vec<_>>();
    let count = binding_lines.len();
    let mean = binding_lines.iter().sum::<usize>().div_ceil(count.max(1));
    let largest = binding_lines.iter().copied().max().unwrap_or(0);
    let prelude = read(&crate_dir().join("src/prelude.rs")).lines().count();
    let measured = [
        ("bindings", count),
        ("mean_binding_lines", mean),
        ("largest_binding_lines", largest),
        ("bindings_dir_lines", file("mod.rs") + file("host.rs")),
        ("convert_lines", file("convert.rs")),
        ("prelude_lines", prelude),
    ];
    for (name, value) in measured {
        println!("thinness: {name} = {value}");
    }
    // The ratchet keys are the three of `design-r3.md` 7.4 rule T10; the
    // other lines are printed for the report.
    let ratcheted = ["mean_binding_lines", "convert_lines", "prelude_lines"];
    if bless() {
        let mut text = String::from(
            "# Size baseline of rule T10. A rise needs a new line here with its reason.\n",
        );
        for (name, value) in measured {
            if ratcheted.contains(&name) {
                text.push_str(name);
                text.push_str(" = ");
                text.push_str(&value.to_string());
                text.push('\n');
            }
        }
        std::fs::write(checked_in("baseline.txt"), text).expect("the baseline is writable");
        return;
    }
    let baseline = read(&checked_in("baseline.txt"));
    for (name, value) in measured {
        if !ratcheted.contains(&name) {
            continue;
        }
        let limit = baseline
            .lines()
            .filter_map(|line| line.split_once('='))
            .find(|(key, _)| key.trim() == name)
            .map(|(_, value)| {
                value
                    .trim()
                    .parse::<usize>()
                    .expect("a baseline value is a whole number")
            })
            .unwrap_or_else(|| panic!("baseline.txt has no `{name}`"));
        assert!(
            value <= limit,
            "rule T10: {name} rose from {limit} to {value}; record the reason in baseline.txt"
        );
    }
}

/// The source lines of one free function.
fn function_lines(text: &str, name: &str) -> usize {
    let header = format!("fn {name}(");
    let start = text
        .find(&header)
        .unwrap_or_else(|| panic!("no `{header}`"));
    let body = &text[start..];
    let end = body.find("\n}\n").map_or(body.len(), |end| end + 2);
    body[..end].lines().count()
}

// ---------------------------------------------------------------------------
// Negative fixtures: each must be refused.

struct Fixture {
    name: &'static str,
    /// Appended to mod.rs; it holds `fn fixture_binding`.
    binding: &'static str,
    /// Appended to convert.rs.
    convert: &'static str,
    /// Appended to runtime.rs.
    outside: &'static str,
    expected: &'static str,
}

const HOST_BINDING: &str = r#"
fn fixture_binding(_lua: &Lua, host: Host<'_>, args: &Args) -> Reply {
    Ok(Out::from(host.wait_next_tick()?)).and(convert::fixture_helper(args))
}
"#;

const FIXTURES: &[Fixture] = &[
    Fixture {
        name: "1: a helper with `while`",
        binding: HOST_BINDING,
        convert: "pub(crate) fn fixture_helper(_args: &Args) -> Reply { let mut n = 0; while n < 3 { n += 1; } Ok(Out::Nothing) }",
        outside: "",
        expected: "has `while` or `loop`",
    },
    Fixture {
        name: "2: a helper with a macro",
        binding: HOST_BINDING,
        convert: "pub(crate) fn fixture_helper(_args: &Args) -> Reply { let _ = format!(\"x\"); Ok(Out::Nothing) }",
        outside: "",
        expected: "invokes a macro",
    },
    Fixture {
        name: "3: `for _ in 0..` in convert.rs",
        binding: HOST_BINDING,
        convert: "pub(crate) fn fixture_helper(_args: &Args) -> Reply { for _ in 0.. { } Ok(Out::Nothing) }",
        outside: "",
        expected: "`for` over a value it did not get as a parameter",
    },
    Fixture {
        name: "4: a helper that calls itself",
        binding: HOST_BINDING,
        convert: "pub(crate) fn fixture_helper(args: &Args) -> Reply { fixture_helper(args) }",
        outside: "",
        expected: "cycle",
    },
    Fixture {
        name: "5: convert.rs calls a crate function outside it",
        binding: HOST_BINDING,
        convert: "pub(crate) fn fixture_helper(_args: &Args) -> Reply { fixture_outside(); Ok(Out::Nothing) }",
        outside: "pub(crate) fn fixture_outside() {}",
        expected: "a function outside the binding directory",
    },
    Fixture {
        name: "6: a path into `script` that is not listed",
        binding: HOST_BINDING,
        convert: "pub(crate) fn fixture_helper(_args: &Args) -> Reply { script::native::fixture_unlisted(); Ok(Out::Nothing) }",
        outside: "",
        expected: "core-paths.txt",
    },
    Fixture {
        name: "7: a `for` over a value the crate built",
        binding: HOST_BINDING,
        convert: "pub(crate) fn fixture_helper(_args: &Args) -> Reply { let built = vec_of(); for _ in built.iter() { } Ok(Out::Nothing) }\nfn vec_of() -> Vec<u8> { Vec::new() }",
        outside: "",
        expected: "`for` over a value it did not get as a parameter",
    },
    Fixture {
        name: "8: an unbounded iterator chain over a script table",
        binding: HOST_BINDING,
        convert: "pub(crate) fn fixture_helper(args: &Args) -> Reply { if let Value::Table(table) = args.first() { let _ = table.pairs::<Value, Value>().count(); } Ok(Out::Nothing) }",
        outside: "",
        expected: "unbounded `pairs`",
    },
    Fixture {
        name: "9: a binding with a numeric parameter",
        binding: "fn fixture_binding(_lua: &Lua, host: Host<'_>, count: i32) -> Reply { let _ = count; Ok(Out::from(host.wait_next_tick()?)) }",
        convert: "",
        outside: "",
        expected: "parameter of a Rust number type",
    },
    Fixture {
        name: "10: a typed numeric read of mlua",
        binding: HOST_BINDING,
        convert: "pub(crate) fn fixture_helper(args: &Args) -> Reply { if let Value::Table(table) = args.first() { let _ = table.raw_get::<i32>(\"x\")?; } Ok(Out::Nothing) }",
        outside: "",
        expected: "reads a Rust number",
    },
    Fixture {
        name: "11: a function given to mlua that returns an error",
        binding: HOST_BINDING,
        convert: "pub(crate) fn fixture_helper(_args: &Args) -> Reply { Ok(Out::Nothing) }",
        outside: "fn fixture_register(lua: &Lua) -> mlua::Result<Function> { lua.create_function(|_, value: Value| { let _ = value; Err(mlua::Error::runtime(\"x\")) }) }",
        expected: "C9:",
    },
    Fixture {
        name: "12: an indirect call of a crate function outside convert.rs",
        binding: HOST_BINDING,
        convert: "pub(crate) fn fixture_helper(args: &Args) -> Reply { let _ = Some(args).map(fixture_outside); Ok(Out::Nothing) }",
        outside: "pub(crate) fn fixture_outside<T>(_value: T) {}",
        expected: "a function outside the binding directory",
    },
    Fixture {
        name: "13: a closure body that calls a crate function outside convert.rs",
        binding: HOST_BINDING,
        convert: "pub(crate) fn fixture_helper(args: &Args) -> Reply { let _ = args.first().as_boolean().map(|_| fixture_outside()); Ok(Out::Nothing) }",
        outside: "pub(crate) fn fixture_outside() {}",
        expected: "a function outside the binding directory",
    },
    Fixture {
        name: "14: a binding that uses the host twice",
        binding: "fn fixture_binding(_lua: &Lua, host: Host<'_>, _args: &Args) -> Reply { host.wait_next_tick()?; Ok(Out::from(host.wait_next_tick()?)) }",
        convert: "",
        outside: "",
        expected: "uses `host` 2 times",
    },
    Fixture {
        name: "15: a binding with a macro",
        binding: "fn fixture_binding(_lua: &Lua, host: Host<'_>, _args: &Args) -> Reply { let _ = format!(\"x\"); Ok(Out::from(host.wait_next_tick()?)) }",
        convert: "",
        outside: "",
        expected: "invokes a macro",
    },
    Fixture {
        name: "16: a conversion helper that takes the host",
        binding: "fn fixture_binding(_lua: &Lua, host: Host<'_>, _args: &Args) -> Reply { convert::fixture_helper(host.setting(\"x\")) }",
        convert: "pub(crate) fn fixture_helper(_value: Option<serde_json::Value>) -> Reply { fixture_with_host(None) }\nfn fixture_with_host(_host: Option<Host<'_>>) -> Reply { Ok(Out::Nothing) }",
        outside: "",
        expected: "",
    },
    Fixture {
        name: "17: a Host method with two calls",
        binding: HOST_BINDING,
        convert: "pub(crate) fn fixture_helper(_args: &Args) -> Reply { Ok(Out::Nothing) }",
        outside: "",
        expected: "Host method `Host::wait_next_tick` makes 3 calls",
    },
    Fixture {
        name: "18: a registration that takes a Rust number",
        binding: HOST_BINDING,
        convert: "pub(crate) fn fixture_helper(_args: &Args) -> Reply { Ok(Out::Nothing) }",
        outside: "fn fixture_register(lua: &Lua) -> mlua::Result<Function> { lua.create_function(|_, count: i32| Ok(count)) }",
        expected: "a registration takes a Rust number",
    },
    Fixture {
        name: "19: a converter calls a runtime helper through an import alias",
        binding: HOST_BINDING,
        convert: "use crate::runtime::fixture_outside as outside_alias;\npub(crate) fn fixture_helper(_args: &Args) -> Reply { outside_alias(); Ok(Out::Nothing) }",
        outside: "pub(crate) fn fixture_outside() {}",
        expected: "outside the binding directory",
    },
    Fixture {
        name: "20: a binding calls an argument-conversion helper of mod.rs",
        binding: "fn fixture_binding(_lua: &Lua, host: Host<'_>, args: &Args) -> Reply { Ok(Out::from(host.wait_next_tick()?)).and(fixture_policy(args)) }\nfn fixture_policy(_args: &Args) -> Reply { Ok(Out::Nothing) }",
        convert: "",
        outside: "",
        expected: "a helper outside convert.rs",
    },
    Fixture {
        name: "21: Luau source in a string outside the prelude",
        binding: HOST_BINDING,
        convert: "pub(crate) fn fixture_helper(_args: &Args) -> Reply { Ok(Out::Nothing) }",
        outside: "fn fixture_chunk(lua: &Lua) -> mlua::Result<()> { lua.load(\"return 1\").exec() }",
        expected: "T9:",
    },
    Fixture {
        name: "22: Luau source included from a file",
        binding: HOST_BINDING,
        convert: "pub(crate) fn fixture_helper(_args: &Args) -> Reply { Ok(Out::Nothing) }",
        outside: "const FIXTURE_CHUNK: &str = include_str!(\"fixture.luau\");\nfn fixture_chunk(lua: &Lua) -> mlua::Result<()> { lua.load(FIXTURE_CHUNK).exec() }",
        expected: "T9:",
    },
];

fn fixture_sources(fixture: &Fixture) -> Sources {
    sources_with(|name, mut text| {
        match name {
            "mod.rs" => {
                text = text.replace(
                    "pub(crate) static BINDINGS: &[Binding] = &[",
                    "pub(crate) static BINDINGS: &[Binding] = &[\n    Binding { path: \"fixture\", kind: Kind::Request, arity: 5, call: Call::Host(fixture_binding), signature: \"\", doc: \"\", js: None },",
                );
                text.push_str(fixture.binding);
            }
            "convert.rs" => text.push_str(fixture.convert),
            "host.rs" if fixture.name.starts_with("17:") => {
                text = text.replace(
                    "self.state.await_next_tick()",
                    "self.state.await_next_tick().and(self.state.await_next_tick())",
                );
            }
            "runtime.rs" => text.push_str(fixture.outside),
            _ => {}
        }
        text
    })
}

#[test]
fn the_census_refuses_every_negative_fixture() {
    let blessed = read(&checked_in("core-paths.txt"));
    let mut accepted = Vec::new();
    for fixture in FIXTURES {
        let sources = fixture_sources(fixture);
        let census = run_census(&sources);
        let mut violations = census.violations.clone();
        // Rule T5 compares the reached core paths with the checked-in file.
        for line in &census.core_paths {
            if !blessed.lines().any(|blessed| blessed == line) {
                violations.push(format!("T5: `{line}` is not in core-paths.txt"));
            }
        }
        // Fixture 16 passes the host's value, not the host; the helper that
        // names `Host` must still be refused.
        let expected = if fixture.name.starts_with("16:") {
            "takes the host but is no binding"
        } else {
            fixture.expected
        };
        let refused = violations.iter().any(|v| v.contains(expected));
        println!(
            "fixture {}: {} (expected {expected:?}); violations: {violations:#?}",
            fixture.name,
            if refused { "refused" } else { "ACCEPTED" },
        );
        if !refused {
            accepted.push(fixture.name);
        }
    }
    assert!(
        accepted.is_empty(),
        "the census accepted these negative fixtures: {accepted:#?}"
    );
}
