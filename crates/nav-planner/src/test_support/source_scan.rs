//! Syntax-based scanning for the navigation planner surface.
//!
//! This test helper walks attached Rust sources to check planner allowlists,
//! test-only scope, unsupported source forms, and unattached Rust files. It
//! follows modules, includes, imports, aliases, and macro tokens to support
//! those source-level checks.
//!
//! The scanner does not implement Rust name resolution and is not the action
//! capability deny list. Compiler-resolved Clippy lints enforce that boundary.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fs;
use std::path::{Component, Path, PathBuf};

use syn::parse::Parser;
use syn::visit::Visit;
use syn::{Attribute, Expr, ExprLit, Item, ItemUse, Lit, Meta, UseTree};

/// Source files keyed by paths relative to a package root.
pub type SourceMap = BTreeMap<PathBuf, String>;

/// Whether a source path can be compiled in a normal build or only in tests.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum Scope {
    Production,
    Test,
}

/// The syntax form that supplied a resolved path.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum PathKind {
    Use,
    Qualified,
    Attribute,
    MacroToken,
    Glob,
}

/// A scanner finding or an unsupported source form.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum FindingKind {
    UsePath,
    QualifiedPath,
    AttributePath,
    MacroTokenPath,
    DeniedGlob,
    AncestorGlob,
    UnsupportedMacroForm,
    UnsupportedCfgAttr,
    UnattachedFile,
    UnsupportedReexportGlob,
    UnsupportedReexport,
    UnresolvedModule,
    ParseError,
    ReadError,
}

/// One path occurrence after alias and module-root resolution.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourcePath {
    pub path: String,
    pub kind: PathKind,
    pub scope: Scope,
    pub file: PathBuf,
}

/// One scanner finding.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct Finding {
    pub kind: FindingKind,
    pub path: Option<String>,
    pub scope: Scope,
    pub file: PathBuf,
    pub detail: String,
}

/// The complete result of one source scan.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ScanReport {
    pub paths: Vec<SourcePath>,
    pub findings: Vec<Finding>,
}

/// One workspace path whose first segment matches a selected crate root.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct WorkspacePath {
    /// The source file that contains the path, relative to the workspace root.
    pub file: PathBuf,
    /// The complete path, including the selected root.
    pub path: String,
}

#[derive(Clone)]
struct Entry {
    file: PathBuf,
    root_file: PathBuf,
    module_path: Vec<String>,
    module_dir: PathBuf,
    scope: Scope,
}

struct Unit {
    file: PathBuf,
    root_file: PathBuf,
    ast: syn::File,
    module_path: Vec<String>,
    scope: Scope,
}

struct ModuleTree {
    units: Vec<Unit>,
    findings: Vec<Finding>,
    modules: BTreeSet<Vec<String>>,
    reached: HashSet<PathBuf>,
    macro_shadows: BTreeSet<String>,
}

#[derive(Clone)]
struct UseLeaf {
    path: Vec<String>,
    local_name: Option<String>,
    glob: bool,
}

#[derive(Clone)]
struct AliasSpec {
    local_name: String,
    target: Vec<String>,
    module_path: Vec<String>,
    scope: Scope,
}

type AliasMap = HashMap<(Vec<String>, String, Scope), BTreeSet<Vec<String>>>;

#[derive(Clone, Copy)]
struct Resolution<'a> {
    package_name: &'a str,
    top_modules: &'a BTreeSet<String>,
    modules: &'a BTreeSet<Vec<String>>,
    aliases: &'a AliasMap,
}

/// Scan all assigned Rust roots in a crate directory.
///
/// Roots are `src/main.rs`, `src/lib.rs`, each direct `src/bin/*.rs`, `build.rs`,
/// each direct `examples/*.rs`, and each direct `tests/*.rs`. Files under `src`
/// that no root reaches are findings.
pub fn scan_package(
    package_root: &Path,
    package_name: &str,
    relevant_roots: &[&str],
    denied_prefixes: &[&str],
) -> ScanReport {
    let (files, mut findings) = read_source_map(package_root);
    let mut report = scan_sources(&files, package_name, relevant_roots, denied_prefixes);
    report.findings.append(&mut findings);
    sort_report(&mut report);
    report
}

/// Scan an in-memory package tree with the normal Cargo source roots.
pub fn scan_sources(
    files: &SourceMap,
    package_name: &str,
    relevant_roots: &[&str],
    denied_prefixes: &[&str],
) -> ScanReport {
    let entries = cargo_roots(files, package_name);
    scan_entries(
        files,
        &entries,
        package_name,
        relevant_roots,
        denied_prefixes,
        true,
    )
}

/// Scan one in-memory source file as a crate root.
pub fn scan_text(
    source: &str,
    file: &str,
    package_name: &str,
    scope: Scope,
    relevant_roots: &[&str],
    denied_prefixes: &[&str],
) -> ScanReport {
    let path = normalize_path(Path::new(file));
    let mut files = SourceMap::new();
    files.insert(path.clone(), source.to_owned());
    let entry = Entry {
        module_dir: path.parent().unwrap_or_else(|| Path::new("")).to_path_buf(),
        root_file: path.clone(),
        file: path,
        module_path: vec![package_name.to_owned()],
        scope,
    };
    scan_entries(
        &files,
        &[entry],
        package_name,
        relevant_roots,
        denied_prefixes,
        false,
    )
}

/// Collect paths rooted at `crate_root` from every Rust file under workspace `crates/`.
///
/// This scan reports only parsed paths. It does not apply the planner scanner's
/// scope rules or reject syntax such as `cfg_attr` and `$crate`.
pub fn scan_workspace_paths(
    workspace_root: &Path,
    crate_root: &str,
) -> Result<Vec<WorkspacePath>, Vec<Finding>> {
    let mut files = SourceMap::new();
    let mut findings = Vec::new();
    collect_rust_files(
        workspace_root,
        &workspace_root.join("crates"),
        &mut files,
        &mut findings,
    );
    match scan_workspace_sources(&files, crate_root) {
        Ok(mut paths) => {
            if findings.is_empty() {
                paths.sort();
                paths.dedup();
                Ok(paths)
            } else {
                findings.sort();
                Err(findings)
            }
        }
        Err(mut parse_findings) => {
            findings.append(&mut parse_findings);
            findings.sort();
            Err(findings)
        }
    }
}

/// Collect paths rooted at `crate_root` from an in-memory workspace source map.
pub fn scan_workspace_sources(
    files: &SourceMap,
    crate_root: &str,
) -> Result<Vec<WorkspacePath>, Vec<Finding>> {
    let mut paths = Vec::new();
    let mut findings = Vec::new();
    for (file, source) in files {
        match syn::parse_file(source) {
            Ok(syntax) => {
                let mut collector = WorkspacePathCollector {
                    crate_root,
                    file,
                    paths: &mut paths,
                };
                collector.visit_file(&syntax);
            }
            Err(error) => findings.push(Finding {
                kind: FindingKind::ParseError,
                path: None,
                scope: Scope::Production,
                file: file.clone(),
                detail: error.to_string(),
            }),
        }
    }
    paths.sort();
    paths.dedup();
    if findings.is_empty() {
        Ok(paths)
    } else {
        findings.sort();
        Err(findings)
    }
}

fn scan_entries(
    files: &SourceMap,
    entries: &[Entry],
    package_name: &str,
    relevant_roots: &[&str],
    denied_prefixes: &[&str],
    report_unattached: bool,
) -> ScanReport {
    let mut tree = load_units(files, entries);
    // The caller declares the external crate roots that this scan guards.
    tree.modules
        .extend(relevant_roots.iter().map(|root| vec![(*root).to_owned()]));
    let mut report = ScanReport::default();
    report.findings.extend(tree.findings);
    let mut paths = Vec::new();
    let mut path_findings = Vec::new();
    let top_modules = top_level_modules(&tree.modules, package_name);

    let aliases = file_aliases(&tree.units, package_name, &top_modules, &tree.modules);
    for unit in &tree.units {
        let mut collector = PathCollector {
            package_name,
            relevant_roots,
            denied_prefixes,
            top_modules: &top_modules,
            modules: &tree.modules,
            aliases: &aliases[&unit.root_file],
            file: &unit.file,
            module_path: unit.module_path.clone(),
            scope: unit.scope,
            paths: &mut paths,
            findings: &mut path_findings,
        };
        collector.visit_file(&unit.ast);
    }
    report.paths = paths;
    report.findings.append(&mut path_findings);

    if report_unattached {
        for path in files.keys() {
            if path.starts_with("src")
                && path.extension().is_some_and(|ext| ext == "rs")
                && !tree.reached.contains(path)
            {
                report.findings.push(Finding {
                    kind: FindingKind::UnattachedFile,
                    path: None,
                    scope: Scope::Production,
                    file: path.clone(),
                    detail: "Rust source file under src is not attached to a crate root".to_owned(),
                });
            }
        }
    }
    sort_report(&mut report);
    report
}

fn cargo_roots(files: &SourceMap, package_name: &str) -> Vec<Entry> {
    let mut roots = Vec::new();
    let mut add = |path: PathBuf, scope: Scope| {
        if files.contains_key(&path) {
            roots.push(Entry {
                module_dir: path.parent().unwrap_or_else(|| Path::new("")).to_path_buf(),
                root_file: path.clone(),
                file: path,
                module_path: vec![package_name.to_owned()],
                scope,
            });
        }
    };
    add(PathBuf::from("build.rs"), Scope::Production);
    add(PathBuf::from("src/main.rs"), Scope::Production);
    add(PathBuf::from("src/lib.rs"), Scope::Production);
    let mut bins: Vec<_> = files
        .keys()
        .filter(|path| {
            path.parent() == Some(Path::new("src/bin"))
                && path.extension().is_some_and(|e| e == "rs")
        })
        .cloned()
        .collect();
    bins.sort();
    for path in bins {
        add(path, Scope::Production);
    }
    let mut examples: Vec<_> = files
        .keys()
        .filter(|path| {
            path.parent() == Some(Path::new("examples"))
                && path.extension().is_some_and(|e| e == "rs")
        })
        .cloned()
        .collect();
    examples.sort();
    for path in examples {
        add(path, Scope::Production);
    }
    let mut tests: Vec<_> = files
        .keys()
        .filter(|path| {
            path.parent() == Some(Path::new("tests")) && path.extension().is_some_and(|e| e == "rs")
        })
        .cloned()
        .collect();
    tests.sort();
    for path in tests {
        add(path, Scope::Test);
    }
    roots
}

fn read_source_map(root: &Path) -> (SourceMap, Vec<Finding>) {
    let mut files = SourceMap::new();
    let mut findings = Vec::new();
    collect_rust_files(root, root, &mut files, &mut findings);
    // Includes can have any extension and can be outside the source directory.
    let mut pending: Vec<_> = files.keys().cloned().collect();
    let mut visited = HashSet::new();
    while let Some(file) = pending.pop() {
        if !visited.insert(file.clone()) {
            continue;
        }
        let Ok(ast) = syn::parse_file(&files[&file]) else {
            continue;
        };
        let mut includes = Vec::new();
        collect_include_files(&ast.items, &file, Scope::Production, &mut includes);
        for included in includes {
            if !files.contains_key(&included) {
                match fs::read_to_string(root.join(&included)) {
                    Ok(source) => {
                        files.insert(included.clone(), source);
                    }
                    Err(error) => {
                        findings.push(Finding {
                            kind: FindingKind::ReadError,
                            path: None,
                            scope: Scope::Production,
                            file: included,
                            detail: error.to_string(),
                        });
                        continue;
                    }
                }
            }
            pending.push(included);
        }
    }
    (files, findings)
}

fn collect_rust_files(root: &Path, dir: &Path, files: &mut SourceMap, findings: &mut Vec<Finding>) {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) => {
            findings.push(Finding {
                kind: FindingKind::ReadError,
                path: None,
                scope: Scope::Production,
                file: dir.strip_prefix(root).unwrap_or(dir).to_path_buf(),
                detail: error.to_string(),
            });
            return;
        }
    };
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                findings.push(Finding {
                    kind: FindingKind::ReadError,
                    path: None,
                    scope: Scope::Production,
                    file: dir.strip_prefix(root).unwrap_or(dir).to_path_buf(),
                    detail: error.to_string(),
                });
                continue;
            }
        };
        let path = entry.path();
        let file_type = match entry.file_type() {
            Ok(file_type) => file_type,
            Err(error) => {
                findings.push(Finding {
                    kind: FindingKind::ReadError,
                    path: None,
                    scope: Scope::Production,
                    file: path.strip_prefix(root).unwrap_or(&path).to_path_buf(),
                    detail: error.to_string(),
                });
                continue;
            }
        };
        if file_type.is_dir() {
            collect_rust_files(root, &path, files, findings);
        } else if file_type.is_file() && path.extension().is_some_and(|ext| ext == "rs") {
            let relative = normalize_path(path.strip_prefix(root).unwrap_or(&path));
            match fs::read_to_string(&path) {
                Ok(source) => {
                    files.insert(relative, source);
                }
                Err(error) => findings.push(Finding {
                    kind: FindingKind::ReadError,
                    path: None,
                    scope: Scope::Production,
                    file: relative,
                    detail: error.to_string(),
                }),
            }
        }
    }
}

fn load_units(files: &SourceMap, entries: &[Entry]) -> ModuleTree {
    let mut tree = ModuleTree {
        units: Vec::new(),
        findings: Vec::new(),
        modules: BTreeSet::new(),
        reached: HashSet::new(),
        macro_shadows: macro_shadows(files),
    };
    let mut active = HashSet::new();
    for entry in entries {
        load_unit(files, entry.clone(), &mut tree, &mut active);
    }
    tree
}

fn load_unit(
    files: &SourceMap,
    mut entry: Entry,
    tree: &mut ModuleTree,
    active: &mut HashSet<(PathBuf, Vec<String>)>,
) {
    entry.file = normalize_path(&entry.file);
    tree.modules.insert(entry.module_path.clone());
    tree.reached.insert(entry.file.clone());
    let state = (entry.file.clone(), entry.module_path.clone());
    if !active.insert(state.clone()) {
        tree.findings.push(Finding {
            kind: FindingKind::UnresolvedModule,
            path: Some(join_path(&entry.module_path)),
            scope: entry.scope,
            file: entry.file,
            detail: "recursive module attachment".to_owned(),
        });
        return;
    }
    let Some(source) = files.get(&entry.file) else {
        tree.findings.push(Finding {
            kind: FindingKind::UnresolvedModule,
            path: Some(join_path(&entry.module_path)),
            scope: entry.scope,
            file: entry.file,
            detail: "attached module source file is absent".to_owned(),
        });
        active.remove(&state);
        return;
    };
    let ast = match syn::parse_file(source) {
        Ok(ast) => ast,
        Err(error) => {
            tree.findings.push(Finding {
                kind: FindingKind::ParseError,
                path: None,
                scope: entry.scope,
                file: entry.file,
                detail: error.to_string(),
            });
            active.remove(&state);
            return;
        }
    };
    load_items(files, &ast.items, &entry, tree, active);
    tree.units.push(Unit {
        file: entry.file,
        root_file: entry.root_file,
        ast,
        module_path: entry.module_path,
        scope: entry.scope,
    });
    active.remove(&state);
}

fn load_items(
    files: &SourceMap,
    items: &[Item],
    parent: &Entry,
    tree: &mut ModuleTree,
    active: &mut HashSet<(PathBuf, Vec<String>)>,
) {
    for item in items {
        let scope = scope_after_attrs(item_attrs(item), parent.scope);
        if let Item::Mod(module) = item {
            let mut child_path = parent.module_path.clone();
            child_path.push(module.ident.to_string());
            tree.modules.insert(child_path.clone());
            if let Some((_, inline_items)) = &module.content {
                let child = Entry {
                    file: parent.file.clone(),
                    root_file: parent.root_file.clone(),
                    module_path: child_path,
                    module_dir: parent.module_dir.join(module.ident.to_string()),
                    scope,
                };
                load_items(files, inline_items, &child, tree, active);
                continue;
            }
            match module_file(files, module, &parent.file, &parent.module_dir) {
                Ok((file, module_dir)) => load_unit(
                    files,
                    Entry {
                        file,
                        root_file: parent.root_file.clone(),
                        module_path: child_path,
                        module_dir,
                        scope,
                    },
                    tree,
                    active,
                ),
                Err(detail) => tree.findings.push(Finding {
                    kind: FindingKind::UnresolvedModule,
                    path: Some(join_path(&child_path)),
                    scope,
                    file: parent.file.clone(),
                    detail,
                }),
            }
        } else if let Item::Macro(invocation) = item {
            if scope == Scope::Production && is_include(invocation) {
                if macro_is_shadowed(invocation, &tree.macro_shadows) {
                    tree.findings.push(Finding {
                        kind: FindingKind::UnsupportedReexport,
                        path: Some(join_path(&parent.module_path)),
                        scope,
                        file: parent.file.clone(),
                        detail: "crate-local macro shadows include!".to_owned(),
                    });
                    continue;
                }
                match include_file(invocation, &parent.file) {
                    Ok(file) => load_unit(
                        files,
                        Entry {
                            module_dir: file
                                .parent()
                                .unwrap_or_else(|| Path::new(""))
                                .to_path_buf(),
                            file,
                            root_file: parent.root_file.clone(),
                            module_path: parent.module_path.clone(),
                            scope,
                        },
                        tree,
                        active,
                    ),
                    Err(detail) => tree.findings.push(Finding {
                        kind: FindingKind::UnsupportedReexport,
                        path: Some(join_path(&parent.module_path)),
                        scope,
                        file: parent.file.clone(),
                        detail,
                    }),
                }
            }
        }
    }
}

fn is_include(invocation: &syn::ItemMacro) -> bool {
    invocation.ident.is_none()
        && (invocation.mac.path.is_ident("include")
            || invocation
                .mac
                .path
                .segments
                .iter()
                .map(|segment| segment.ident.to_string())
                .eq(["std", "include"]))
}

fn include_file(invocation: &syn::ItemMacro, file: &Path) -> Result<PathBuf, String> {
    let parser = syn::punctuated::Punctuated::<syn::LitStr, syn::Token![,]>::parse_terminated;
    let paths = parser
        .parse2(invocation.mac.tokens.clone())
        .map_err(|error| format!("include! requires one literal source path: {error}"))?;
    if paths.len() != 1 {
        return Err("include! requires one literal source path".to_owned());
    }
    let directory = file.parent().unwrap_or_else(|| Path::new(""));
    Ok(normalize_path(&directory.join(paths[0].value())))
}

fn collect_include_files(
    items: &[Item],
    file: &Path,
    inherited_scope: Scope,
    output: &mut Vec<PathBuf>,
) {
    for item in items {
        let scope = scope_after_attrs(item_attrs(item), inherited_scope);
        if scope != Scope::Production {
            continue;
        }
        match item {
            Item::Macro(invocation) if is_include(invocation) => {
                if let Ok(included) = include_file(invocation, file) {
                    output.push(included);
                }
            }
            Item::Mod(module) => {
                if let Some((_, items)) = &module.content {
                    collect_include_files(items, file, scope, output);
                }
            }
            _ => {}
        }
    }
}

fn macro_shadows(files: &SourceMap) -> BTreeSet<String> {
    #[derive(Default)]
    struct Shadows(BTreeSet<String>);

    impl<'ast> Visit<'ast> for Shadows {
        fn visit_item(&mut self, item: &'ast Item) {
            if scope_after_attrs(item_attrs(item), Scope::Production) == Scope::Test {
                return;
            }
            match item {
                Item::Macro(definition) if definition.mac.path.is_ident("macro_rules") => {
                    if let Some(name) = &definition.ident {
                        self.0.insert(name.to_string());
                    }
                }
                Item::Use(import) => {
                    let mut leaves = Vec::new();
                    flatten_use_tree(&import.tree, &mut Vec::new(), &mut leaves);
                    self.0
                        .extend(leaves.into_iter().filter_map(|leaf| leaf.local_name));
                }
                _ => {}
            }
            syn::visit::visit_item(self, item);
        }
    }

    let mut shadows = Shadows::default();
    for source in files.values() {
        if let Ok(ast) = syn::parse_file(source) {
            shadows.visit_file(&ast);
        }
    }
    shadows.0
}

fn macro_is_shadowed(invocation: &syn::ItemMacro, shadows: &BTreeSet<String>) -> bool {
    invocation
        .mac
        .path
        .segments
        .last()
        .is_some_and(|segment| shadows.contains(&segment.ident.to_string()))
}

fn module_file(
    files: &SourceMap,
    module: &syn::ItemMod,
    current_file: &Path,
    module_dir: &Path,
) -> Result<(PathBuf, PathBuf), String> {
    if let Some(path_attr) = module
        .attrs
        .iter()
        .find(|attr| attr.path().is_ident("path"))
    {
        let Meta::NameValue(value) = &path_attr.meta else {
            return Err("#[path] must contain a string literal".to_owned());
        };
        let Expr::Lit(ExprLit {
            lit: Lit::Str(path),
            ..
        }) = &value.value
        else {
            return Err("#[path] must contain a string literal".to_owned());
        };
        let declaring_dir = current_file.parent().unwrap_or_else(|| Path::new(""));
        let file = normalize_path(&declaring_dir.join(path.value()));
        return Ok((file.clone(), module_directory_for_file(&file)));
    }
    let name = module.ident.to_string();
    let flat = normalize_path(&module_dir.join(format!("{name}.rs")));
    let nested = normalize_path(&module_dir.join(&name).join("mod.rs"));
    match (files.contains_key(&flat), files.contains_key(&nested)) {
        (true, false) => Ok((flat.clone(), module_directory_for_file(&flat))),
        (false, true) => Ok((nested.clone(), module_directory_for_file(&nested))),
        (true, true) => Err(format!(
            "both {} and {} define this module",
            flat.display(),
            nested.display()
        )),
        (false, false) => Err(format!(
            "neither {} nor {} defines this module",
            flat.display(),
            nested.display()
        )),
    }
}

fn module_directory_for_file(file: &Path) -> PathBuf {
    let parent = file.parent().unwrap_or_else(|| Path::new(""));
    if file.file_name().is_some_and(|name| name == "mod.rs") {
        parent.to_path_buf()
    } else {
        parent.join(file.file_stem().unwrap_or_default())
    }
}

fn item_attrs(item: &Item) -> &[Attribute] {
    match item {
        Item::Const(node) => &node.attrs,
        Item::Enum(node) => &node.attrs,
        Item::ExternCrate(node) => &node.attrs,
        Item::Fn(node) => &node.attrs,
        Item::ForeignMod(node) => &node.attrs,
        Item::Impl(node) => &node.attrs,
        Item::Macro(node) => &node.attrs,
        Item::Mod(node) => &node.attrs,
        Item::Static(node) => &node.attrs,
        Item::Struct(node) => &node.attrs,
        Item::Trait(node) => &node.attrs,
        Item::TraitAlias(node) => &node.attrs,
        Item::Type(node) => &node.attrs,
        Item::Union(node) => &node.attrs,
        Item::Use(node) => &node.attrs,
        Item::Verbatim(_) => &[],
        _ => &[],
    }
}

fn scope_after_attrs(attrs: &[Attribute], inherited: Scope) -> Scope {
    if attrs.iter().any(is_test_cfg) {
        Scope::Test
    } else {
        inherited
    }
}

fn is_test_cfg(attr: &Attribute) -> bool {
    let Meta::List(cfg) = &attr.meta else {
        return false;
    };
    if !cfg.path.is_ident("cfg") {
        return false;
    }
    parse_meta_list(&cfg.tokens).is_some_and(|items| items.iter().any(is_test_meta))
}

fn is_test_meta(meta: &Meta) -> bool {
    match meta {
        Meta::Path(path) => path.is_ident("test"),
        Meta::List(list) if list.path.is_ident("all") => {
            parse_meta_list(&list.tokens).is_some_and(|items| items.iter().any(is_test_meta))
        }
        _ => false,
    }
}

fn parse_meta_list(
    tokens: &proc_macro2::TokenStream,
) -> Option<syn::punctuated::Punctuated<Meta, syn::Token![,]>> {
    let parser = syn::punctuated::Punctuated::<Meta, syn::Token![,]>::parse_terminated;
    parser.parse2(tokens.clone()).ok()
}

fn top_level_modules(modules: &BTreeSet<Vec<String>>, package_name: &str) -> BTreeSet<String> {
    modules
        .iter()
        .filter(|path| path.len() == 2 && path.first().is_some_and(|root| root == package_name))
        .map(|path| path[1].clone())
        .collect()
}

fn normalize_path(path: &Path) -> PathBuf {
    let mut result = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => match result.components().next_back() {
                Some(Component::Normal(_)) => {
                    result.pop();
                }
                Some(Component::RootDir | Component::Prefix(_)) => {}
                _ => result.push(component.as_os_str()),
            },
            Component::RootDir | Component::Prefix(_) | Component::Normal(_) => {
                result.push(component.as_os_str());
            }
        }
    }
    result
}

fn join_path(path: &[String]) -> String {
    path.join("::")
}

fn is_at_or_under(path: &str, prefix: &str) -> bool {
    path == prefix
        || path
            .strip_prefix(prefix)
            .is_some_and(|suffix| suffix.starts_with("::"))
}

fn is_proper_prefix(path: &str, full: &str) -> bool {
    full.strip_prefix(path)
        .is_some_and(|suffix| suffix.starts_with("::"))
}

fn flatten_use_tree(tree: &UseTree, prefix: &mut Vec<String>, output: &mut Vec<UseLeaf>) {
    match tree {
        UseTree::Path(node) => {
            prefix.push(node.ident.to_string());
            flatten_use_tree(&node.tree, prefix, output);
            prefix.pop();
        }
        UseTree::Name(node) => {
            let name = node.ident.to_string();
            if name == "self" {
                output.push(UseLeaf {
                    path: prefix.clone(),
                    local_name: prefix.last().cloned(),
                    glob: false,
                });
            } else {
                prefix.push(name.clone());
                output.push(UseLeaf {
                    path: prefix.clone(),
                    local_name: Some(name),
                    glob: false,
                });
                prefix.pop();
            }
        }
        UseTree::Rename(node) => {
            let name = node.ident.to_string();
            let path = if name == "self" {
                prefix.clone()
            } else {
                let mut path = prefix.clone();
                path.push(name);
                path
            };
            output.push(UseLeaf {
                path,
                local_name: Some(node.rename.to_string()),
                glob: false,
            });
        }
        UseTree::Glob(_) => output.push(UseLeaf {
            path: prefix.clone(),
            local_name: None,
            glob: true,
        }),
        UseTree::Group(node) => {
            for item in &node.items {
                flatten_use_tree(item, prefix, output);
            }
        }
    }
}

fn collect_alias_specs(
    items: &[Item],
    module_path: &[String],
    inherited_scope: Scope,
    output: &mut Vec<AliasSpec>,
) {
    for item in items {
        let scope = scope_after_attrs(item_attrs(item), inherited_scope);
        match item {
            Item::Use(item_use) => {
                let mut leaves = Vec::new();
                flatten_use_tree(&item_use.tree, &mut Vec::new(), &mut leaves);
                for leaf in leaves.into_iter().filter(|leaf| !leaf.glob) {
                    if let Some(local_name) = leaf.local_name {
                        output.push(AliasSpec {
                            local_name,
                            target: leaf.path,
                            module_path: module_path.to_vec(),
                            scope,
                        });
                    }
                }
            }
            Item::ExternCrate(item_extern) => {
                output.push(AliasSpec {
                    local_name: item_extern
                        .rename
                        .as_ref()
                        .map(|(_, name)| name.to_string())
                        .unwrap_or_else(|| item_extern.ident.to_string()),
                    // The empty root marks an explicit extern-crate declaration.
                    // It must not resolve back through its own local alias.
                    target: if item_extern.ident == "self" {
                        vec!["crate".to_owned()]
                    } else {
                        vec![String::new(), item_extern.ident.to_string()]
                    },
                    module_path: module_path.to_vec(),
                    scope,
                });
            }
            Item::Mod(module) => {
                if let Some((_, nested)) = &module.content {
                    let mut child = module_path.to_vec();
                    child.push(module.ident.to_string());
                    collect_alias_specs(nested, &child, scope, output);
                }
            }
            _ => {
                let name = match item {
                    Item::Const(item) => Some(&item.ident),
                    Item::Enum(item) => Some(&item.ident),
                    Item::Fn(item) => Some(&item.sig.ident),
                    Item::Static(item) => Some(&item.ident),
                    Item::Struct(item) => Some(&item.ident),
                    Item::Trait(item) => Some(&item.ident),
                    Item::TraitAlias(item) => Some(&item.ident),
                    Item::Type(item) => Some(&item.ident),
                    Item::Union(item) => Some(&item.ident),
                    _ => None,
                };
                if let Some(name) = name {
                    let mut target = vec!["crate".to_owned()];
                    target.extend(module_path.iter().skip(1).cloned());
                    target.push(name.to_string());
                    output.push(AliasSpec {
                        local_name: name.to_string(),
                        target,
                        module_path: module_path.to_vec(),
                        scope,
                    });
                }
            }
        }
    }
}

// Included files share imports and declarations with their module. Keep
// independent Cargo roots separate, even with the same package and module path.
fn file_aliases(
    units: &[Unit],
    package_name: &str,
    top_modules: &BTreeSet<String>,
    modules: &BTreeSet<Vec<String>>,
) -> BTreeMap<PathBuf, AliasMap> {
    let mut groups = BTreeMap::<&PathBuf, Vec<AliasSpec>>::new();
    for unit in units {
        collect_alias_specs(
            &unit.ast.items,
            &unit.module_path,
            unit.scope,
            groups.entry(&unit.root_file).or_default(),
        );
    }
    groups
        .into_iter()
        .map(|(root, specs)| {
            (
                root.clone(),
                resolve_alias_specs(&specs, package_name, top_modules, modules),
            )
        })
        .collect()
}

fn resolve_alias_specs(
    specs: &[AliasSpec],
    package_name: &str,
    top_modules: &BTreeSet<String>,
    modules: &BTreeSet<Vec<String>>,
) -> AliasMap {
    let mut aliases: AliasMap = HashMap::new();
    let passes = specs.len().saturating_add(1).max(1);
    for _ in 0..passes {
        let mut changed = false;
        for spec in specs {
            for target in resolve_path(
                &spec.target,
                &spec.module_path,
                spec.scope,
                Resolution {
                    package_name,
                    top_modules,
                    modules,
                    aliases: &aliases,
                },
                0,
            ) {
                changed |= aliases
                    .entry((
                        spec.module_path.clone(),
                        spec.local_name.clone(),
                        spec.scope,
                    ))
                    .or_default()
                    .insert(target);
            }
        }
        if !changed {
            break;
        }
    }
    aliases
}

fn resolve_path(
    raw: &[String],
    module_path: &[String],
    scope: Scope,
    resolution: Resolution<'_>,
    depth: usize,
) -> Vec<Vec<String>> {
    let Resolution {
        package_name,
        top_modules,
        modules,
        aliases,
    } = resolution;
    if raw.is_empty() || depth > 16 {
        return Vec::new();
    }
    if raw[0].is_empty() {
        return vec![raw[1..].to_vec()];
    }
    let mut first = 0;
    let mut base = module_path.to_vec();
    while first < raw.len() {
        match raw[first].as_str() {
            "crate" => {
                base = vec![package_name.to_owned()];
                first += 1;
            }
            "self" => first += 1,
            "super" => {
                if base.len() > 1 {
                    base.pop();
                }
                first += 1;
            }
            _ => break,
        }
    }
    if first > 0 {
        base.extend(raw[first..].iter().cloned());
        return vec![base];
    }

    let leading = &raw[0];
    let mut targets = BTreeSet::new();
    if scope == Scope::Test {
        if let Some(production) =
            aliases.get(&(module_path.to_vec(), leading.clone(), Scope::Production))
        {
            targets.extend(production.iter().cloned());
        }
    }
    if let Some(current) = aliases.get(&(module_path.to_vec(), leading.clone(), scope)) {
        targets.extend(current.iter().cloned());
    }
    if !targets.is_empty() {
        let mut resolved = Vec::new();
        for target in targets {
            let mut expanded = target;
            expanded.extend(raw.iter().skip(1).cloned());
            resolved.extend(resolve_path(
                &expanded,
                module_path,
                scope,
                resolution,
                depth + 1,
            ));
        }
        return resolved;
    }

    if leading == package_name
        || top_modules.contains(leading)
        || modules.contains(&vec![leading.clone()])
    {
        let mut path = if leading == package_name || modules.contains(&vec![leading.clone()]) {
            raw.to_vec()
        } else {
            let mut path = vec![package_name.to_owned()];
            path.extend(raw.iter().cloned());
            path
        };
        return vec![std::mem::take(&mut path)];
    }

    for prefix_len in (1..=raw.len()).rev() {
        let mut local_candidate = module_path.to_vec();
        local_candidate.extend(raw[..prefix_len].iter().cloned());
        if modules.contains(&local_candidate) {
            local_candidate.extend(raw[prefix_len..].iter().cloned());
            return vec![local_candidate];
        }
    }

    if raw.len() > 1 || is_prelude_name(leading) {
        vec![raw.to_vec()]
    } else {
        Vec::new()
    }
}

fn is_prelude_name(name: &str) -> bool {
    matches!(
        name,
        "bool"
            | "char"
            | "str"
            | "u8"
            | "u16"
            | "u32"
            | "u64"
            | "u128"
            | "usize"
            | "i8"
            | "i16"
            | "i32"
            | "i64"
            | "i128"
            | "isize"
            | "f32"
            | "f64"
            | "Box"
            | "Vec"
            | "String"
            | "Option"
            | "Result"
            | "Send"
            | "Sync"
            | "Sized"
            | "Unpin"
            | "Copy"
            | "Clone"
            | "Drop"
            | "Fn"
            | "FnMut"
            | "FnOnce"
            | "IntoIterator"
            | "Iterator"
            | "DoubleEndedIterator"
            | "ExactSizeIterator"
            | "Extend"
            | "FromIterator"
            | "From"
            | "Into"
            | "TryFrom"
            | "TryInto"
            | "AsRef"
            | "AsMut"
            | "Default"
            | "Eq"
            | "PartialEq"
            | "Ord"
            | "PartialOrd"
            | "ToOwned"
            | "ToString"
    )
}

struct PathCollector<'a> {
    package_name: &'a str,
    relevant_roots: &'a [&'a str],
    denied_prefixes: &'a [&'a str],
    top_modules: &'a BTreeSet<String>,
    modules: &'a BTreeSet<Vec<String>>,
    aliases: &'a AliasMap,
    file: &'a Path,
    module_path: Vec<String>,
    scope: Scope,
    paths: &'a mut Vec<SourcePath>,
    findings: &'a mut Vec<Finding>,
}

impl PathCollector<'_> {
    fn resolution(&self) -> Resolution<'_> {
        Resolution {
            package_name: self.package_name,
            top_modules: self.top_modules,
            modules: self.modules,
            aliases: self.aliases,
        }
    }

    fn record_path(&mut self, raw: &[String], kind: PathKind, scope: Scope) {
        let paths = resolve_path(raw, &self.module_path, scope, self.resolution(), 0);
        if paths.is_empty() && matches!(kind, PathKind::Use | PathKind::Glob) {
            self.findings.push(Finding {
                kind: FindingKind::UnsupportedReexport,
                path: Some(join_path(raw)),
                scope,
                file: self.file.to_path_buf(),
                detail: "import target does not resolve to a known item or extern crate".to_owned(),
            });
        }
        for path in paths {
            let rendered = join_path(&path);
            self.paths.push(SourcePath {
                path: rendered.clone(),
                kind,
                scope,
                file: self.file.to_path_buf(),
            });
            if scope != Scope::Production {
                continue;
            }
            let denied = self
                .denied_prefixes
                .iter()
                .any(|prefix| is_at_or_under(&rendered, prefix));
            if denied {
                let finding_kind = match kind {
                    PathKind::Use => FindingKind::UsePath,
                    PathKind::Qualified => FindingKind::QualifiedPath,
                    PathKind::Attribute => FindingKind::AttributePath,
                    PathKind::MacroToken => FindingKind::MacroTokenPath,
                    PathKind::Glob => continue,
                };
                self.findings.push(Finding {
                    kind: finding_kind,
                    path: Some(rendered),
                    scope,
                    file: self.file.to_path_buf(),
                    detail: "production source names a denied path".to_owned(),
                });
            }
        }
    }

    fn record_glob(&mut self, raw: &[String], scope: Scope) {
        let paths = resolve_path(raw, &self.module_path, scope, self.resolution(), 0);
        if paths.is_empty() {
            self.findings.push(Finding {
                kind: FindingKind::UnsupportedReexportGlob,
                path: Some(format!("{}::*", join_path(raw))),
                scope,
                file: self.file.to_path_buf(),
                detail: "glob target does not resolve to a known item or extern crate".to_owned(),
            });
        }
        for path in paths {
            let rendered = join_path(&path);
            self.paths.push(SourcePath {
                path: rendered.clone(),
                kind: PathKind::Glob,
                scope,
                file: self.file.to_path_buf(),
            });
            if scope != Scope::Production {
                continue;
            }
            let kind = if self
                .denied_prefixes
                .iter()
                .any(|prefix| is_at_or_under(&rendered, prefix))
            {
                Some(FindingKind::DeniedGlob)
            } else if self
                .denied_prefixes
                .iter()
                .any(|prefix| is_proper_prefix(&rendered, prefix))
            {
                Some(FindingKind::AncestorGlob)
            } else {
                None
            };
            if let Some(kind) = kind {
                self.findings.push(Finding {
                    kind,
                    path: Some(format!("{rendered}::*")),
                    scope,
                    file: self.file.to_path_buf(),
                    detail: "glob can expose a denied path".to_owned(),
                });
            }
        }
    }

    fn record_use_item(&mut self, item: &ItemUse, scope: Scope) {
        let mut leaves = Vec::new();
        flatten_use_tree(&item.tree, &mut Vec::new(), &mut leaves);
        for leaf in leaves {
            if leaf.glob {
                self.record_glob(&leaf.path, scope);
            } else {
                self.record_path(&leaf.path, PathKind::Use, scope);
            }
        }
    }

    fn record_meta(&mut self, meta: &Meta, scope: Scope) {
        match meta {
            Meta::Path(path) => {
                let raw = path
                    .segments
                    .iter()
                    .map(|segment| segment.ident.to_string())
                    .collect::<Vec<_>>();
                self.record_path(&raw, PathKind::Attribute, scope);
            }
            Meta::NameValue(value) => {
                let raw = value
                    .path
                    .segments
                    .iter()
                    .map(|segment| segment.ident.to_string())
                    .collect::<Vec<_>>();
                self.record_path(&raw, PathKind::Attribute, scope);
                self.visit_expr(&value.value);
            }
            Meta::List(list) => {
                let raw = list
                    .path
                    .segments
                    .iter()
                    .map(|segment| segment.ident.to_string())
                    .collect::<Vec<_>>();
                self.record_path(&raw, PathKind::Attribute, scope);
                if let Some(items) = parse_meta_list(&list.tokens) {
                    for nested in &items {
                        self.record_meta(nested, scope);
                    }
                }
            }
        }
    }

    fn record_macro(&mut self, mac: &syn::Macro, scope: Scope) {
        let result = read_macro_tokens(
            &mac.tokens,
            &self.module_path,
            scope,
            self.resolution(),
            self.relevant_roots,
        );
        for path in result.paths {
            if path.glob {
                self.record_glob(&path.path, scope);
            } else {
                self.record_path(&path.path, PathKind::MacroToken, scope);
            }
        }
        for issue in result.issues {
            self.findings.push(Finding {
                kind: FindingKind::UnsupportedMacroForm,
                path: issue.path,
                scope,
                file: self.file.to_path_buf(),
                detail: issue.detail,
            });
        }
    }

    fn with_attrs(&mut self, attrs: &[Attribute], visit: impl FnOnce(&mut Self)) {
        let previous = self.scope;
        self.scope = scope_after_attrs(attrs, previous);
        visit(self);
        self.scope = previous;
    }
}

impl<'ast> Visit<'ast> for PathCollector<'_> {
    fn visit_file(&mut self, node: &'ast syn::File) {
        self.with_attrs(&node.attrs, |this| syn::visit::visit_file(this, node));
    }

    fn visit_item(&mut self, node: &'ast Item) {
        self.with_attrs(item_attrs(node), |this| syn::visit::visit_item(this, node));
    }

    fn visit_item_mod(&mut self, node: &'ast syn::ItemMod) {
        if node.content.is_some() {
            self.module_path.push(node.ident.to_string());
            syn::visit::visit_item_mod(self, node);
            self.module_path.pop();
        } else {
            syn::visit::visit_item_mod(self, node);
        }
    }

    fn visit_impl_item(&mut self, node: &'ast syn::ImplItem) {
        self.with_attrs(impl_item_attrs(node), |this| {
            syn::visit::visit_impl_item(this, node)
        });
    }

    fn visit_trait_item(&mut self, node: &'ast syn::TraitItem) {
        self.with_attrs(trait_item_attrs(node), |this| {
            syn::visit::visit_trait_item(this, node)
        });
    }

    fn visit_foreign_item(&mut self, node: &'ast syn::ForeignItem) {
        self.with_attrs(foreign_item_attrs(node), |this| {
            syn::visit::visit_foreign_item(this, node)
        });
    }

    fn visit_field(&mut self, node: &'ast syn::Field) {
        self.with_attrs(&node.attrs, |this| syn::visit::visit_field(this, node));
    }

    fn visit_variant(&mut self, node: &'ast syn::Variant) {
        self.with_attrs(&node.attrs, |this| syn::visit::visit_variant(this, node));
    }

    fn visit_stmt(&mut self, node: &'ast syn::Stmt) {
        match node {
            syn::Stmt::Local(local) => {
                self.with_attrs(&local.attrs, |this| syn::visit::visit_stmt(this, node))
            }
            syn::Stmt::Macro(mac) => {
                self.with_attrs(&mac.attrs, |this| syn::visit::visit_stmt(this, node))
            }
            _ => syn::visit::visit_stmt(self, node),
        }
    }

    fn visit_item_use(&mut self, node: &'ast ItemUse) {
        self.record_use_item(node, self.scope);
        syn::visit::visit_item_use(self, node);
    }

    fn visit_item_extern_crate(&mut self, node: &'ast syn::ItemExternCrate) {
        self.record_path(&[node.ident.to_string()], PathKind::Use, self.scope);
        syn::visit::visit_item_extern_crate(self, node);
    }

    fn visit_attribute(&mut self, node: &'ast Attribute) {
        if node.path().is_ident("cfg_attr") {
            self.findings.push(Finding {
                kind: FindingKind::UnsupportedCfgAttr,
                path: None,
                scope: self.scope,
                file: self.file.to_path_buf(),
                detail: "cfg_attr changes source scope in a form the scanner does not resolve"
                    .to_owned(),
            });
        }
        self.record_meta(&node.meta, self.scope);
    }

    fn visit_path(&mut self, node: &'ast syn::Path) {
        let raw = node
            .segments
            .iter()
            .map(|segment| segment.ident.to_string())
            .collect::<Vec<_>>();
        self.record_path(&raw, PathKind::Qualified, self.scope);
        syn::visit::visit_path(self, node);
    }

    fn visit_macro(&mut self, node: &'ast syn::Macro) {
        self.record_macro(node, self.scope);
        syn::visit::visit_macro(self, node);
    }
}

fn impl_item_attrs(item: &syn::ImplItem) -> &[Attribute] {
    match item {
        syn::ImplItem::Const(node) => &node.attrs,
        syn::ImplItem::Fn(node) => &node.attrs,
        syn::ImplItem::Type(node) => &node.attrs,
        syn::ImplItem::Macro(node) => &node.attrs,
        syn::ImplItem::Verbatim(_) => &[],
        _ => &[],
    }
}

fn trait_item_attrs(item: &syn::TraitItem) -> &[Attribute] {
    match item {
        syn::TraitItem::Const(node) => &node.attrs,
        syn::TraitItem::Fn(node) => &node.attrs,
        syn::TraitItem::Type(node) => &node.attrs,
        syn::TraitItem::Macro(node) => &node.attrs,
        syn::TraitItem::Verbatim(_) => &[],
        _ => &[],
    }
}

fn foreign_item_attrs(item: &syn::ForeignItem) -> &[Attribute] {
    match item {
        syn::ForeignItem::Fn(node) => &node.attrs,
        syn::ForeignItem::Static(node) => &node.attrs,
        syn::ForeignItem::Type(node) => &node.attrs,
        syn::ForeignItem::Macro(node) => &node.attrs,
        syn::ForeignItem::Verbatim(_) => &[],
        _ => &[],
    }
}

#[derive(Clone)]
struct MacroPath {
    path: Vec<String>,
    glob: bool,
}

struct MacroIssue {
    path: Option<String>,
    detail: String,
}

#[derive(Default)]
struct MacroRead {
    paths: Vec<MacroPath>,
    issues: Vec<MacroIssue>,
}

fn read_macro_tokens(
    tokens: &proc_macro2::TokenStream,
    module_path: &[String],
    scope: Scope,
    resolution: Resolution<'_>,
    relevant_roots: &[&str],
) -> MacroRead {
    use proc_macro2::{Delimiter, TokenTree};
    let aliases = resolution.aliases;

    let mut result = MacroRead::default();
    let mut stack = vec![tokens.clone()];
    while let Some(stream) = stack.pop() {
        let trees: Vec<_> = stream.into_iter().collect();
        let mut i = 0;
        'scan: while i < trees.len() {
            if let TokenTree::Punct(punct) = &trees[i] {
                if punct.as_char() == '$' {
                    if matches!(trees.get(i + 1), Some(TokenTree::Ident(ident)) if ident == "crate")
                    {
                        result.issues.push(MacroIssue {
                            path: Some("$crate".to_owned()),
                            detail: "$crate cannot be resolved by the source scanner".to_owned(),
                        });
                    } else if matches!(trees.get(i + 1), Some(TokenTree::Ident(_)))
                        && is_double_colon(&trees, i + 2)
                    {
                        result.issues.push(MacroIssue {
                            path: None,
                            detail: "macro fragment is adjacent to `::`".to_owned(),
                        });
                    }
                    i += 1;
                    continue;
                }
            }
            if matches!(trees.get(i), Some(TokenTree::Ident(word)) if word == "extern")
                && matches!(trees.get(i + 1), Some(TokenTree::Ident(word)) if word == "crate")
            {
                result.issues.push(MacroIssue {
                    path: None,
                    detail: "extern crate inside a macro is unsupported".to_owned(),
                });
            }
            if matches!(trees.get(i), Some(TokenTree::Ident(word)) if word == "use") {
                if let (Some(TokenTree::Ident(root)), Some(TokenTree::Ident(as_word))) =
                    (trees.get(i + 1), trees.get(i + 2))
                {
                    let root_path = [root.to_string()];
                    if as_word == "as"
                        && root_is_relevant(&root_path, module_path, scope, aliases, relevant_roots)
                    {
                        result.issues.push(MacroIssue {
                            path: Some(root.to_string()),
                            detail: "crate-root alias inside a macro is unsupported".to_owned(),
                        });
                    }
                }
            }

            if let TokenTree::Ident(first) = &trees[i] {
                let mut raw = vec![first.to_string()];
                let mut j = i + 1;
                loop {
                    if !is_double_colon(&trees, j) {
                        break;
                    }
                    let next = j + 2;
                    match trees.get(next) {
                        Some(TokenTree::Ident(ident)) => {
                            raw.push(ident.to_string());
                            j = next + 1;
                        }
                        Some(TokenTree::Group(group)) if group.delimiter() == Delimiter::Brace => {
                            if root_is_relevant(&raw, module_path, scope, aliases, relevant_roots) {
                                if stream_has_dollar(&group.stream()) {
                                    result.issues.push(MacroIssue {
                                        path: Some(join_path(&raw)),
                                        detail: "relevant crate root has an unresolved macro group"
                                            .to_owned(),
                                    });
                                } else {
                                    match parse_grouped_use(&raw, group) {
                                        Ok(item_use) => {
                                            let mut leaves = Vec::new();
                                            flatten_use_tree(
                                                &item_use.tree,
                                                &mut Vec::new(),
                                                &mut leaves,
                                            );
                                            for leaf in leaves {
                                                let resolved = resolve_path(
                                                    &leaf.path,
                                                    module_path,
                                                    scope,
                                                    resolution,
                                                    0,
                                                );
                                                for path in resolved {
                                                    if leaf.local_name.as_ref().is_some_and(
                                                        |local| {
                                                            path.len() == 1 && local != &path[0]
                                                        },
                                                    ) {
                                                        result.issues.push(MacroIssue {
                                                            path: Some(join_path(&path)),
                                                            detail: "crate-root alias inside a macro is unsupported".to_owned(),
                                                        });
                                                    }
                                                    result.paths.push(MacroPath {
                                                        path,
                                                        glob: leaf.glob,
                                                    });
                                                }
                                            }
                                        }
                                        Err(error) => result.issues.push(MacroIssue {
                                            path: Some(join_path(&raw)),
                                            detail: format!(
                                                "grouped use cannot be resolved: {error}"
                                            ),
                                        }),
                                    }
                                }
                            } else {
                                stack.push(group.stream());
                            }
                            i = next + 1;
                            continue 'scan;
                        }
                        Some(TokenTree::Punct(punct)) if punct.as_char() == '$' => {
                            if root_is_relevant(&raw, module_path, scope, aliases, relevant_roots) {
                                result.issues.push(MacroIssue {
                                    path: Some(join_path(&raw)),
                                    detail: "macro fragment is adjacent to `::`".to_owned(),
                                });
                            }
                            j = next + 1;
                            break;
                        }
                        _ => break,
                    }
                }
                if j > i + 1 && raw.len() > 1 {
                    let resolved = resolve_path(&raw, module_path, scope, resolution, 0);
                    result.paths.extend(
                        resolved
                            .into_iter()
                            .map(|path| MacroPath { path, glob: false }),
                    );
                    i = j;
                } else if i < trees.len() {
                    if let TokenTree::Group(group) = &trees[i] {
                        stack.push(group.stream());
                    }
                    i += 1;
                }
            } else {
                if let TokenTree::Group(group) = &trees[i] {
                    stack.push(group.stream());
                }
                i += 1;
            }
        }
    }
    result
}

fn is_double_colon(trees: &[proc_macro2::TokenTree], index: usize) -> bool {
    use proc_macro2::TokenTree;
    matches!(trees.get(index), Some(TokenTree::Punct(first)) if first.as_char() == ':')
        && matches!(trees.get(index + 1), Some(TokenTree::Punct(second)) if second.as_char() == ':')
}

fn stream_has_dollar(tokens: &proc_macro2::TokenStream) -> bool {
    use proc_macro2::TokenTree;
    tokens.clone().into_iter().any(|tree| match tree {
        TokenTree::Punct(punct) => punct.as_char() == '$',
        TokenTree::Group(group) => stream_has_dollar(&group.stream()),
        _ => false,
    })
}

fn root_is_relevant(
    raw: &[String],
    module_path: &[String],
    scope: Scope,
    aliases: &AliasMap,
    relevant_roots: &[&str],
) -> bool {
    if raw
        .first()
        .is_some_and(|first| relevant_roots.contains(&first.as_str()))
    {
        return true;
    }
    let Some(first) = raw.first() else {
        return false;
    };
    let targets_match = |alias_scope| {
        aliases
            .get(&(module_path.to_vec(), first.clone(), alias_scope))
            .is_some_and(|targets| {
                targets.iter().any(|target| {
                    target
                        .first()
                        .is_some_and(|root| relevant_roots.contains(&root.as_str()))
                })
            })
    };
    targets_match(scope) || (scope == Scope::Test && targets_match(Scope::Production))
}

fn parse_grouped_use(prefix: &[String], group: &proc_macro2::Group) -> syn::Result<ItemUse> {
    let source = format!("use {}::{{{}}};", join_path(prefix), group.stream());
    syn::parse_str(&source)
}

struct WorkspacePathCollector<'a> {
    crate_root: &'a str,
    file: &'a Path,
    paths: &'a mut Vec<WorkspacePath>,
}

impl WorkspacePathCollector<'_> {
    fn matches_root(&self, segment: &str) -> bool {
        segment.strip_prefix("r#").unwrap_or(segment) == self.crate_root
    }

    fn record(&mut self, segments: &[String]) {
        self.paths.push(WorkspacePath {
            file: self.file.to_path_buf(),
            path: join_path(segments),
        });
    }
}

impl Visit<'_> for WorkspacePathCollector<'_> {
    fn visit_item_use(&mut self, node: &ItemUse) {
        let mut leaves = Vec::new();
        flatten_use_tree(&node.tree, &mut Vec::new(), &mut leaves);
        for leaf in leaves {
            if leaf
                .path
                .first()
                .is_some_and(|segment| self.matches_root(segment))
            {
                self.record(&leaf.path);
            }
        }
        syn::visit::visit_item_use(self, node);
    }

    fn visit_item_extern_crate(&mut self, node: &syn::ItemExternCrate) {
        if self.matches_root(&node.ident.to_string()) {
            self.record(&[self.crate_root.to_owned()]);
        }
        syn::visit::visit_item_extern_crate(self, node);
    }

    fn visit_path(&mut self, node: &syn::Path) {
        let segments = node
            .segments
            .iter()
            .map(|segment| segment.ident.to_string())
            .collect::<Vec<_>>();
        if segments.len() > 1 && self.matches_root(&segments[0]) {
            self.record(&segments);
        }
        syn::visit::visit_path(self, node);
    }

    fn visit_macro(&mut self, node: &syn::Macro) {
        collect_macro_workspace_paths(&node.tokens, self.crate_root, self.file, self.paths);
        syn::visit::visit_macro(self, node);
    }
}

fn collect_macro_workspace_paths(
    tokens: &proc_macro2::TokenStream,
    crate_root: &str,
    file: &Path,
    paths: &mut Vec<WorkspacePath>,
) {
    use proc_macro2::TokenTree;

    let trees = tokens.clone().into_iter().collect::<Vec<_>>();
    let root_matches = |ident: &proc_macro2::Ident| {
        let name = ident.to_string();
        name.strip_prefix("r#").unwrap_or(&name) == crate_root
    };
    let mut index = 0;
    while index < trees.len() {
        if matches!(trees.get(index), Some(TokenTree::Ident(word)) if word == "use") {
            if let Some(TokenTree::Ident(root)) = trees.get(index + 1) {
                if root_matches(root) && !is_double_colon(&trees, index + 2) {
                    paths.push(WorkspacePath {
                        file: file.to_path_buf(),
                        path: crate_root.to_owned(),
                    });
                }
            }
        }
        if matches!(trees.get(index), Some(TokenTree::Ident(word)) if word == "extern")
            && matches!(trees.get(index + 1), Some(TokenTree::Ident(word)) if word == "crate")
        {
            if let Some(TokenTree::Ident(root)) = trees.get(index + 2) {
                if root_matches(root) {
                    paths.push(WorkspacePath {
                        file: file.to_path_buf(),
                        path: crate_root.to_owned(),
                    });
                }
            }
        }
        if let Some(TokenTree::Ident(root)) = trees.get(index) {
            if root_matches(root) && is_double_colon(&trees, index + 1) {
                let mut segments = vec![crate_root.to_owned()];
                let mut next = index + 3;
                while let Some(TokenTree::Ident(segment)) = trees.get(next) {
                    segments.push(segment.to_string());
                    next += 1;
                    if is_double_colon(&trees, next) {
                        next += 2;
                    } else {
                        break;
                    }
                }
                paths.push(WorkspacePath {
                    file: file.to_path_buf(),
                    path: join_path(&segments),
                });
            }
        }
        if let Some(TokenTree::Group(group)) = trees.get(index) {
            collect_macro_workspace_paths(&group.stream(), crate_root, file, paths);
        }
        index += 1;
    }
}

fn sort_report(report: &mut ScanReport) {
    report.paths.sort_by(|left, right| {
        (&left.file, left.scope, left.kind, &left.path).cmp(&(
            &right.file,
            right.scope,
            right.kind,
            &right.path,
        ))
    });
    report.findings.sort();
}
