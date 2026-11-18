use super::errors::{
    DeclarationReason, ExportReason, ExtractionError, ScriptDeclarationError, UnresolvedModuleError,
};
use super::parse_source;
use crate::library::shape::{
    is_host_shim_specifier, resolve_sibling_path, MAX_PREPARATION_SIBLINGS,
    MAX_PREPARATION_SOURCE_BYTES,
};
use deno_ast::swc::ast::{
    Decl, ExportSpecifier, Expr, Ident, ImportSpecifier, ModuleDecl, ModuleExportName, ModuleItem,
    ObjectPatProp, Pat,
};
use deno_ast::swc::common::Span;
use deno_ast::swc::ecma_visit::{Visit, VisitWith};
use deno_ast::{ParsedSource, ProgramRef};
use std::collections::{HashMap, HashSet, VecDeque};
use std::fs;
use std::ops::Range;
use std::path::{Path, PathBuf};

#[derive(Debug)]
pub(super) struct ModuleGraph {
    entry: PathBuf,
    modules: HashMap<PathBuf, ModuleNode>,
    /// The first import the graph could not resolve. Export analysis treats
    /// that module as supplying nothing; a native entry refuses on it, while
    /// a compat entry keeps its classification (its card stays unloadable).
    unresolved: Option<UnresolvedModuleError>,
}

#[derive(Debug)]
pub(super) struct ModuleNode {
    pub path: PathBuf,
    pub source: String,
    pub parsed: ParsedSource,
    source_offset: usize,
    targets: HashMap<String, Target>,
    type_only: HashSet<deno_ast::swc::ast::Id>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum Target {
    Local(PathBuf),
    Host,
    /// An import that names no readable sibling.
    Unresolved,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum Origin {
    Local(PathBuf, String),
    Namespace(PathBuf),
}

#[derive(Debug)]
pub(super) struct ExportSupplier {
    pub span: Range<usize>,
    pub direct_const: Option<DirectConst>,
    pub reason: Option<ExportReason>,
    pub runtime: bool,
}

#[derive(Debug, Clone)]
pub(super) struct DirectConst {
    pub initializer: Option<Box<Expr>>,
    pub statement_span: Range<usize>,
    pub binding: Option<deno_ast::swc::ast::Id>,
}

/// Where the module graph reads sibling sources.
#[derive(Debug, Clone)]
pub(crate) enum ModuleSources {
    /// Siblings are origin files beside the entry on disk.
    Disk,
    /// Siblings are in memory, keyed by their runtime module URL
    /// (`/rs2b0t/bot/scripts/...`). The entry path uses the same URL space.
    /// Only the raw-origin test adapter reads in-memory siblings.
    #[cfg(any(test, feature = "test-hooks"))]
    Memory(Vec<(String, String)>),
}

impl ModuleSources {
    fn entry_key(&self, path: &Path) -> PathBuf {
        match self {
            Self::Disk => absolute_path(path),
            #[cfg(any(test, feature = "test-hooks"))]
            Self::Memory(_) => path.to_path_buf(),
        }
    }

    fn resolve(&self, base: &Path, specifier: &str) -> Option<PathBuf> {
        match self {
            Self::Disk => resolve_sibling_path(base, specifier).map(|path| absolute_path(&path)),
            #[cfg(any(test, feature = "test-hooks"))]
            Self::Memory(siblings) => {
                let url = memory_module_url(base, specifier)?;
                siblings
                    .iter()
                    .any(|(sibling, _)| sibling == &url)
                    .then(|| PathBuf::from(url))
            }
        }
    }

    fn read(&self, path: &Path) -> Option<String> {
        match self {
            Self::Disk => fs::read_to_string(path).ok(),
            #[cfg(any(test, feature = "test-hooks"))]
            Self::Memory(siblings) => siblings
                .iter()
                .find(|(url, _)| Path::new(url) == path)
                .map(|(_, source)| source.clone()),
        }
    }
}

/// Join a relative specifier to an in-memory importer directory, as the
/// runtime module loader does. `.ts` specifiers name their `.js` module.
#[cfg(any(test, feature = "test-hooks"))]
fn memory_module_url(base: &Path, specifier: &str) -> Option<String> {
    if !(specifier.starts_with("./") || specifier.starts_with("../")) {
        return None;
    }
    let mut parts: Vec<&str> = base
        .to_str()?
        .split('/')
        .filter(|part| !part.is_empty())
        .collect();
    for part in specifier.split('/') {
        match part {
            "." | "" => {}
            ".." => {
                parts.pop()?;
            }
            other => parts.push(other),
        }
    }
    let joined = format!("/{}", parts.join("/"));
    Some(match joined.strip_suffix(".ts") {
        Some(stem) => format!("{stem}.js"),
        None => joined,
    })
}

impl ModuleGraph {
    pub fn with_sources(
        path: &Path,
        source: &str,
        sources: ModuleSources,
    ) -> Result<Self, ExtractionError> {
        let entry = sources.entry_key(path);
        let mut modules = HashMap::new();
        let mut pending = VecDeque::new();
        let mut source_bytes = source.len();
        let mut unresolved = None;
        modules.insert(entry.clone(), parse_node(entry.clone(), source.to_owned())?);
        pending.push_back(entry.clone());

        while let Some(path) = pending.pop_front() {
            let dependencies =
                runtime_dependencies(modules.get(&path).expect("queued module exists"));
            for (specifier, span) in dependencies {
                let importer = modules.get(&path).expect("queued module exists");
                let target = if is_host_shim_specifier(&specifier) {
                    Target::Host
                } else {
                    let base = path.parent().unwrap_or_else(|| Path::new("."));
                    let resolved = sources.resolve(base, &specifier).and_then(|target_path| {
                        if modules.contains_key(&target_path) {
                            return Some((target_path, None));
                        }
                        sources
                            .read(&target_path)
                            .map(|source| (target_path, Some(source)))
                    });
                    let Some((target_path, sibling_source)) = resolved else {
                        unresolved.get_or_insert(UnresolvedModuleError {
                            path: importer.path.clone(),
                            span,
                            specifier: specifier.clone(),
                        });
                        modules
                            .get_mut(&path)
                            .expect("queued module exists")
                            .targets
                            .insert(specifier, Target::Unresolved);
                        continue;
                    };
                    match sibling_source {
                        None => Target::Local(target_path),
                        Some(sibling_source) => {
                            if modules.len().saturating_sub(1) >= MAX_PREPARATION_SIBLINGS {
                                return Err(UnresolvedModuleError {
                                    path: importer.path.clone(),
                                    span,
                                    specifier: format!("{specifier} (sibling limit exceeded)"),
                                }
                                .into());
                            }
                            source_bytes = source_bytes.saturating_add(sibling_source.len());
                            if source_bytes > MAX_PREPARATION_SOURCE_BYTES {
                                return Err(UnresolvedModuleError {
                                    path: importer.path.clone(),
                                    span,
                                    specifier: format!("{specifier} (source limit exceeded)"),
                                }
                                .into());
                            }
                            modules.insert(
                                target_path.clone(),
                                parse_node(target_path.clone(), sibling_source)?,
                            );
                            pending.push_back(target_path.clone());
                            Target::Local(target_path)
                        }
                    }
                };
                modules
                    .get_mut(&path)
                    .expect("queued module exists")
                    .targets
                    .insert(specifier, target);
            }
        }
        Ok(Self {
            entry,
            modules,
            unresolved,
        })
    }

    /// The first unresolved import, if any.
    pub fn unresolved(&self) -> Option<&UnresolvedModuleError> {
        self.unresolved.as_ref()
    }

    pub fn entry(&self) -> &ModuleNode {
        self.modules.get(&self.entry).expect("entry is present")
    }

    pub fn has_runtime_export(&self, name: &str) -> bool {
        !self
            .origins(&self.entry, name, &mut HashSet::new())
            .is_empty()
    }

    pub fn suppliers(&self, name: &str) -> Vec<ExportSupplier> {
        let entry = self.entry();
        let ProgramRef::Module(module) = entry.parsed.program_ref() else {
            return Vec::new();
        };
        let mut suppliers = Vec::new();
        for item in &module.body {
            let ModuleItem::ModuleDecl(declaration) = item else {
                continue;
            };
            match declaration {
                ModuleDecl::ExportDecl(export) => match &export.decl {
                    Decl::Var(var) => {
                        for declarator in &var.decls {
                            if !pattern_idents(&declarator.name)
                                .iter()
                                .any(|ident| ident.sym.as_ref() == name)
                            {
                                continue;
                            }
                            let direct = var.kind == deno_ast::swc::ast::VarDeclKind::Const
                                && var.decls.len() == 1
                                && matches!(declarator.name, Pat::Ident(_))
                                && !var.declare;
                            let span = statement_range(entry, export.span);
                            suppliers.push(ExportSupplier {
                                span: span.clone(),
                                direct_const: direct.then(|| DirectConst {
                                    initializer: declarator.init.clone(),
                                    statement_span: span,
                                    binding: match &declarator.name {
                                        Pat::Ident(binding) => Some(binding.id.to_id()),
                                        _ => None,
                                    },
                                }),
                                reason: if direct {
                                    declarator
                                        .init
                                        .is_none()
                                        .then_some(ExportReason::MissingInitializer)
                                } else {
                                    Some(ExportReason::NotDirectConst)
                                },
                                runtime: !var.declare,
                            });
                        }
                    }
                    decl => {
                        if declaration_all_idents(decl)
                            .iter()
                            .any(|ident| ident.sym.as_ref() == name)
                        {
                            suppliers.push(ExportSupplier {
                                span: statement_range(entry, export.span),
                                direct_const: None,
                                reason: Some(ExportReason::NotDirectConst),
                                runtime: declaration_is_runtime(decl),
                            });
                        }
                    }
                },
                ModuleDecl::ExportNamed(export) if !export.type_only => {
                    for specifier in &export.specifiers {
                        let (exported, specifier_span, runtime) = match specifier {
                            ExportSpecifier::Named(named) => (
                                export_name(named.exported.as_ref().unwrap_or(&named.orig)),
                                named.span,
                                !named.is_type_only,
                            ),
                            ExportSpecifier::Namespace(namespace) => {
                                (export_name(&namespace.name), namespace.span, true)
                            }
                            ExportSpecifier::Default(default) => (
                                default.exported.sym.to_string(),
                                default.exported.span,
                                true,
                            ),
                        };
                        if exported != name || !runtime {
                            continue;
                        }
                        if let Some(source) = &export.src {
                            let Some(target) = entry.targets.get(source.value.as_ref()) else {
                                continue;
                            };
                            let imported = match specifier {
                                ExportSpecifier::Named(named) => export_name(&named.orig),
                                ExportSpecifier::Default(_) => "default".to_owned(),
                                ExportSpecifier::Namespace(_) => String::new(),
                            };
                            if matches!(specifier, ExportSpecifier::Namespace(_))
                                || !self.target_origins(target, &imported).is_empty()
                            {
                                suppliers.push(non_direct_supplier(entry.range(specifier_span)));
                            }
                        } else if let ExportSpecifier::Named(named) = specifier {
                            if let ModuleExportName::Ident(local) = &named.orig {
                                if entry.type_only.contains(&local.to_id()) {
                                    continue;
                                }
                                if !self
                                    .origins(&self.entry, name, &mut HashSet::new())
                                    .is_empty()
                                {
                                    suppliers
                                        .push(non_direct_supplier(entry.range(specifier_span)));
                                }
                            }
                        }
                    }
                }
                ModuleDecl::ExportAll(export) if !export.type_only => {
                    if let Some(target) = entry.targets.get(export.src.value.as_ref()) {
                        if !self.target_origins(target, name).is_empty() {
                            suppliers.push(non_direct_supplier(entry.range(export.span)));
                        }
                    }
                }
                _ => {}
            }
        }
        suppliers
    }

    pub fn direct_tick_function(&self) -> Option<Range<usize>> {
        let entry = self.entry();
        let ProgramRef::Module(module) = entry.parsed.program_ref() else {
            return None;
        };
        module.body.iter().find_map(|item| {
            let ModuleItem::ModuleDecl(ModuleDecl::ExportDecl(export)) = item else {
                return None;
            };
            match &export.decl {
                Decl::Fn(function)
                    if !function.declare && function.ident.sym.as_ref() == "tick" =>
                {
                    Some(statement_range(entry, export.span))
                }
                _ => None,
            }
        })
    }

    pub fn compat_shape(&self) -> Option<crate::library::shape::LoadShape> {
        use crate::library::shape::LoadShape;
        let entry = self.entry();
        let ProgramRef::Module(module) = entry.parsed.program_ref() else {
            return None;
        };
        let mut markers = CompatMarkers::default();
        module.visit_with(&mut markers);
        if markers.define_bot || markers.compat_property || markers.compat_declaration {
            return Some(LoadShape::CompatDefineBot);
        }
        for item in &module.body {
            let ModuleItem::ModuleDecl(ModuleDecl::ExportDefaultDecl(export)) = item else {
                continue;
            };
            let deno_ast::swc::ast::DefaultDecl::Class(class) = &export.decl else {
                continue;
            };
            if let Some(super_class) = &class.class.super_class {
                if matches!(unwrap_expression(super_class), Expr::Ident(ident) if matches!(ident.sym.as_ref(), "LoopingBot" | "TaskBot" | "TreeBot"))
                {
                    return Some(LoadShape::CompatClass);
                }
            }
        }
        None
    }

    pub fn has_runtime_default(&self) -> bool {
        self.has_runtime_export("default")
    }

    pub fn check_erased_references(
        &self,
        erased: &HashMap<String, deno_ast::swc::ast::Id>,
    ) -> Result<(), ScriptDeclarationError> {
        let erased_names: HashSet<String> = erased.keys().cloned().collect();
        let entry = self.entry();
        let mut visitor = RuntimeIdentifierVisitor {
            bindings: erased.values().collect(),
            found: None,
        };
        if let ProgramRef::Module(module) = entry.parsed.program_ref() {
            module.visit_with(&mut visitor);
        }
        if let Some(span) = visitor.found {
            return Err(runtime_reference_error(entry, entry.range(span)));
        }

        for node in self.modules.values() {
            self.check_node_references(node, &erased_names, erased)?;
        }
        Ok(())
    }

    fn check_node_references(
        &self,
        node: &ModuleNode,
        erased_names: &HashSet<String>,
        erased_ids: &HashMap<String, deno_ast::swc::ast::Id>,
    ) -> Result<(), ScriptDeclarationError> {
        let ProgramRef::Module(module) = node.parsed.program_ref() else {
            return Ok(());
        };
        let mut namespaces = HashMap::new();
        for item in &module.body {
            let ModuleItem::ModuleDecl(declaration) = item else {
                continue;
            };
            match declaration {
                ModuleDecl::Import(import) if !import.type_only => {
                    let Some(target) = node.targets.get(import.src.value.as_ref()) else {
                        continue;
                    };
                    for specifier in &import.specifiers {
                        if specifier.is_type_only() {
                            continue;
                        }
                        match specifier {
                            ImportSpecifier::Named(named) => {
                                let imported = named
                                    .imported
                                    .as_ref()
                                    .map_or_else(|| named.local.sym.to_string(), export_name);
                                if self.target_exports_erased(target, &imported, erased_names) {
                                    return Err(runtime_reference_error(
                                        node,
                                        node.range(named.span),
                                    ));
                                }
                            }
                            ImportSpecifier::Default(default) => {
                                if self.target_exports_erased(target, "default", erased_names) {
                                    return Err(runtime_reference_error(
                                        node,
                                        node.range(default.span),
                                    ));
                                }
                            }
                            ImportSpecifier::Namespace(namespace) => {
                                namespaces.insert(namespace.local.to_id(), target.clone());
                            }
                        }
                    }
                }
                ModuleDecl::ExportNamed(export) if !export.type_only => {
                    if let Some(source) = &export.src {
                        if let Some(target) = node.targets.get(source.value.as_ref()) {
                            for specifier in &export.specifiers {
                                let (leaks_erased_export, specifier_span) = match specifier {
                                    ExportSpecifier::Named(named) if !named.is_type_only => (
                                        self.target_exports_erased(
                                            target,
                                            &export_name(&named.orig),
                                            erased_names,
                                        ),
                                        named.span,
                                    ),
                                    ExportSpecifier::Default(default) => (
                                        self.target_exports_erased(target, "default", erased_names),
                                        default.exported.span,
                                    ),
                                    ExportSpecifier::Namespace(namespace) => (
                                        erased_names.iter().any(|name| {
                                            self.target_exports_erased(target, name, erased_names)
                                        }),
                                        namespace.span,
                                    ),
                                    _ => continue,
                                };
                                if leaks_erased_export {
                                    return Err(runtime_reference_error(
                                        node,
                                        node.range(specifier_span),
                                    ));
                                }
                            }
                        }
                    } else if node.path == self.entry {
                        for specifier in &export.specifiers {
                            if let ExportSpecifier::Named(named) = specifier {
                                if !named.is_type_only {
                                    if let ModuleExportName::Ident(local) = &named.orig {
                                        if erased_ids.values().any(|id| id == &local.to_id()) {
                                            return Err(runtime_reference_error(
                                                node,
                                                node.range(named.span),
                                            ));
                                        }
                                        if let Some((target, None)) =
                                            import_target(node, &local.to_id())
                                        {
                                            if erased_names.iter().any(|name| {
                                                self.target_exports_erased(
                                                    target,
                                                    name,
                                                    erased_names,
                                                )
                                            }) {
                                                return Err(runtime_reference_error(
                                                    node,
                                                    node.range(named.span),
                                                ));
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                ModuleDecl::ExportAll(export) if !export.type_only => {
                    if let Some(target) = node.targets.get(export.src.value.as_ref()) {
                        if erased_names
                            .iter()
                            .any(|name| self.target_exports_erased(target, name, erased_names))
                        {
                            return Err(runtime_reference_error(node, node.range(export.span)));
                        }
                    }
                }
                _ => {}
            }
        }
        if !namespaces.is_empty() {
            let mut visitor = NamespaceReferenceVisitor {
                namespaces: &namespaces,
                erased_names,
                graph: self,
                found: None,
            };
            module.visit_with(&mut visitor);
            if let Some(span) = visitor.found {
                return Err(runtime_reference_error(node, node.range(span)));
            }
        }
        Ok(())
    }

    fn target_exports_erased(&self, target: &Target, name: &str, erased: &HashSet<String>) -> bool {
        self.target_origins(target, name)
            .iter()
            .any(|origin| match origin {
                Origin::Local(path, binding) => path == &self.entry && erased.contains(binding),
                Origin::Namespace(path) => erased
                    .iter()
                    .any(|name| !self.origins(path, name, &mut HashSet::new()).is_empty()),
            })
    }

    fn target_origins(&self, target: &Target, name: &str) -> HashSet<Origin> {
        match target {
            Target::Local(path) => self.origins(path, name, &mut HashSet::new()),
            Target::Host | Target::Unresolved => HashSet::new(),
        }
    }

    fn origins(
        &self,
        path: &Path,
        name: &str,
        visited: &mut HashSet<(PathBuf, String)>,
    ) -> HashSet<Origin> {
        if !visited.insert((path.to_path_buf(), name.to_owned())) {
            return HashSet::new();
        }
        let Some(node) = self.modules.get(path) else {
            return HashSet::new();
        };
        let ProgramRef::Module(module) = node.parsed.program_ref() else {
            return HashSet::new();
        };
        let mut origins = HashSet::new();
        for item in &module.body {
            let ModuleItem::ModuleDecl(declaration) = item else {
                continue;
            };
            match declaration {
                ModuleDecl::ExportDecl(export) => {
                    for ident in declaration_value_idents(&export.decl) {
                        if ident.sym.as_ref() == name {
                            origins.insert(Origin::Local(path.to_path_buf(), name.to_owned()));
                        }
                    }
                }
                ModuleDecl::ExportDefaultDecl(export) if name == "default" => {
                    if !matches!(
                        export.decl,
                        deno_ast::swc::ast::DefaultDecl::TsInterfaceDecl(_)
                    ) {
                        origins.insert(Origin::Local(path.to_path_buf(), "default".to_owned()));
                    }
                }
                ModuleDecl::ExportDefaultExpr(export) if name == "default" => {
                    if !matches!(&*export.expr, Expr::Ident(ident) if node.type_only.contains(&ident.to_id()))
                    {
                        origins.insert(Origin::Local(path.to_path_buf(), "default".to_owned()));
                    }
                }
                ModuleDecl::ExportNamed(export) if !export.type_only => {
                    if let Some(source) = &export.src {
                        let Some(target) = node.targets.get(source.value.as_ref()) else {
                            continue;
                        };
                        for specifier in &export.specifiers {
                            match specifier {
                                ExportSpecifier::Named(named)
                                    if !named.is_type_only
                                        && export_name(
                                            named.exported.as_ref().unwrap_or(&named.orig),
                                        ) == name =>
                                {
                                    origins.extend(self.target_origins_seen(
                                        target,
                                        &export_name(&named.orig),
                                        visited,
                                    ));
                                }
                                ExportSpecifier::Namespace(namespace)
                                    if export_name(&namespace.name) == name =>
                                {
                                    if let Target::Local(target) = target {
                                        origins.insert(Origin::Namespace(target.clone()));
                                    }
                                }
                                ExportSpecifier::Default(default)
                                    if default.exported.sym.as_ref() == name =>
                                {
                                    origins.extend(
                                        self.target_origins_seen(target, "default", visited),
                                    );
                                }
                                _ => {}
                            }
                        }
                    } else {
                        for specifier in &export.specifiers {
                            let ExportSpecifier::Named(named) = specifier else {
                                continue;
                            };
                            if named.is_type_only
                                || export_name(named.exported.as_ref().unwrap_or(&named.orig))
                                    != name
                            {
                                continue;
                            }
                            let ModuleExportName::Ident(local) = &named.orig else {
                                continue;
                            };
                            if node.type_only.contains(&local.to_id()) {
                                continue;
                            }
                            if let Some((target, imported)) = import_target(node, &local.to_id()) {
                                match imported {
                                    Some(imported) => origins.extend(
                                        self.target_origins_seen(target, &imported, visited),
                                    ),
                                    None => {
                                        if let Target::Local(path) = target {
                                            origins.insert(Origin::Namespace(path.clone()));
                                        }
                                    }
                                }
                            } else {
                                origins.insert(Origin::Local(
                                    path.to_path_buf(),
                                    local.sym.to_string(),
                                ));
                            }
                        }
                    }
                }
                ModuleDecl::ExportAll(export) if !export.type_only && name != "default" => {
                    if let Some(target) = node.targets.get(export.src.value.as_ref()) {
                        origins.extend(self.target_origins_seen(target, name, visited));
                    }
                }
                _ => {}
            }
        }
        origins
    }

    fn target_origins_seen(
        &self,
        target: &Target,
        name: &str,
        visited: &mut HashSet<(PathBuf, String)>,
    ) -> HashSet<Origin> {
        match target {
            Target::Local(path) => self.origins(path, name, visited),
            Target::Host | Target::Unresolved => HashSet::new(),
        }
    }
}

impl ModuleNode {
    pub fn range(&self, span: Span) -> Range<usize> {
        super::source_range(span, self.parsed.text(), self.source_offset)
    }
}

fn parse_node(path: PathBuf, source: String) -> Result<ModuleNode, ExtractionError> {
    let (parsed, source_offset) = parse_source(&path, &source)?;
    let type_only = type_only_bindings(&parsed);
    Ok(ModuleNode {
        path,
        source,
        parsed,
        source_offset,
        targets: HashMap::new(),
        type_only,
    })
}

fn absolute_path(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| {
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            std::env::current_dir()
                .map(|cwd| cwd.join(path))
                .unwrap_or_else(|_| path.to_path_buf())
        }
    })
}

fn runtime_dependencies(node: &ModuleNode) -> Vec<(String, Range<usize>)> {
    let ProgramRef::Module(module) = node.parsed.program_ref() else {
        return Vec::new();
    };
    let mut dependencies = Vec::new();
    for item in &module.body {
        let ModuleItem::ModuleDecl(declaration) = item else {
            continue;
        };
        match declaration {
            ModuleDecl::Import(import)
                if !import.type_only
                    && (import.specifiers.is_empty()
                        || import
                            .specifiers
                            .iter()
                            .any(|specifier| !specifier.is_type_only())) =>
            {
                dependencies.push((import.src.value.to_string(), node.range(import.span)));
            }
            ModuleDecl::ExportAll(export) if !export.type_only => {
                dependencies.push((export.src.value.to_string(), node.range(export.span)));
            }
            ModuleDecl::ExportNamed(export)
                if export.src.is_some()
                    && !export.type_only
                    && export.specifiers.iter().any(|specifier| match specifier {
                        ExportSpecifier::Named(named) => !named.is_type_only,
                        _ => true,
                    }) =>
            {
                dependencies.push((
                    export
                        .src
                        .as_ref()
                        .expect("source is present")
                        .value
                        .to_string(),
                    node.range(export.span),
                ));
            }
            _ => {}
        }
    }
    dependencies
}

fn type_only_bindings(parsed: &ParsedSource) -> HashSet<deno_ast::swc::ast::Id> {
    let ProgramRef::Module(module) = parsed.program_ref() else {
        return HashSet::new();
    };
    let mut types = HashSet::new();
    let mut values = HashSet::new();
    for item in &module.body {
        match item {
            ModuleItem::ModuleDecl(ModuleDecl::Import(import)) => {
                for specifier in &import.specifiers {
                    let id = import_local(specifier).to_id();
                    if import.type_only || specifier.is_type_only() {
                        types.insert(id);
                    } else {
                        values.insert(id);
                    }
                }
            }
            ModuleItem::ModuleDecl(ModuleDecl::ExportDecl(export)) => {
                collect_decl_bindings(&export.decl, &mut types, &mut values);
            }
            ModuleItem::Stmt(deno_ast::swc::ast::Stmt::Decl(declaration)) => {
                collect_decl_bindings(declaration, &mut types, &mut values);
            }
            _ => {}
        }
    }
    types.retain(|id| !values.contains(id));
    types
}

fn collect_decl_bindings(
    declaration: &Decl,
    types: &mut HashSet<deno_ast::swc::ast::Id>,
    values: &mut HashSet<deno_ast::swc::ast::Id>,
) {
    match declaration {
        Decl::TsInterface(declaration) => {
            types.insert(declaration.id.to_id());
        }
        Decl::TsTypeAlias(declaration) => {
            types.insert(declaration.id.to_id());
        }
        Decl::Var(var) if var.declare => {
            types.extend(
                var.decls
                    .iter()
                    .flat_map(|decl| pattern_idents(&decl.name))
                    .map(|ident| ident.to_id()),
            );
        }
        Decl::Fn(function) if function.declare => {
            types.insert(function.ident.to_id());
        }
        Decl::Class(class) if class.declare => {
            types.insert(class.ident.to_id());
        }
        Decl::TsEnum(declaration) if declaration.declare => {
            types.insert(declaration.id.to_id());
        }
        Decl::TsModule(declaration) if declaration.declare => {
            if let deno_ast::swc::ast::TsModuleName::Ident(ident) = &declaration.id {
                types.insert(ident.to_id());
            }
        }
        Decl::Var(var) => {
            values.extend(
                var.decls
                    .iter()
                    .flat_map(|decl| pattern_idents(&decl.name))
                    .map(|ident| ident.to_id()),
            );
        }
        Decl::Fn(function) => {
            values.insert(function.ident.to_id());
        }
        Decl::Class(class) => {
            values.insert(class.ident.to_id());
        }
        Decl::TsEnum(declaration) => {
            values.insert(declaration.id.to_id());
        }
        Decl::TsModule(declaration) => {
            if let deno_ast::swc::ast::TsModuleName::Ident(ident) = &declaration.id {
                values.insert(ident.to_id());
            }
        }
        Decl::Using(using) => {
            values.extend(
                using
                    .decls
                    .iter()
                    .flat_map(|decl| pattern_idents(&decl.name))
                    .map(|ident| ident.to_id()),
            );
        }
    }
}

fn declaration_all_idents(declaration: &Decl) -> Vec<&Ident> {
    match declaration {
        Decl::Var(var) => var
            .decls
            .iter()
            .flat_map(|decl| pattern_idents(&decl.name))
            .collect(),
        Decl::Fn(function) => vec![&function.ident],
        Decl::Class(class) => vec![&class.ident],
        Decl::TsEnum(declaration) => vec![&declaration.id],
        Decl::TsModule(declaration) => match &declaration.id {
            deno_ast::swc::ast::TsModuleName::Ident(ident) => vec![ident],
            deno_ast::swc::ast::TsModuleName::Str(_) => Vec::new(),
        },
        Decl::TsInterface(declaration) => vec![&declaration.id],
        Decl::TsTypeAlias(declaration) => vec![&declaration.id],
        Decl::Using(using) => using
            .decls
            .iter()
            .flat_map(|decl| pattern_idents(&decl.name))
            .collect(),
    }
}

fn declaration_value_idents(declaration: &Decl) -> Vec<&Ident> {
    match declaration {
        Decl::Var(var) if !var.declare => var
            .decls
            .iter()
            .flat_map(|decl| pattern_idents(&decl.name))
            .collect(),
        Decl::Fn(function) if !function.declare => vec![&function.ident],
        Decl::Class(class) if !class.declare => vec![&class.ident],
        Decl::TsEnum(declaration) if !declaration.declare => vec![&declaration.id],
        Decl::TsModule(declaration) if !declaration.declare => match &declaration.id {
            deno_ast::swc::ast::TsModuleName::Ident(ident) => vec![ident],
            deno_ast::swc::ast::TsModuleName::Str(_) => Vec::new(),
        },
        Decl::Using(using) => using
            .decls
            .iter()
            .flat_map(|decl| pattern_idents(&decl.name))
            .collect(),
        _ => Vec::new(),
    }
}

fn declaration_is_runtime(declaration: &Decl) -> bool {
    !declaration_value_idents(declaration).is_empty()
}

fn pattern_idents(pattern: &Pat) -> Vec<&Ident> {
    match pattern {
        Pat::Ident(binding) => vec![&binding.id],
        Pat::Array(array) => array
            .elems
            .iter()
            .flatten()
            .flat_map(pattern_idents)
            .collect(),
        Pat::Rest(rest) => pattern_idents(&rest.arg),
        Pat::Object(object) => object
            .props
            .iter()
            .flat_map(|property| match property {
                ObjectPatProp::KeyValue(property) => pattern_idents(&property.value),
                ObjectPatProp::Assign(property) => vec![&property.key.id],
                ObjectPatProp::Rest(property) => pattern_idents(&property.arg),
            })
            .collect(),
        Pat::Assign(assignment) => pattern_idents(&assignment.left),
        Pat::Invalid(_) | Pat::Expr(_) => Vec::new(),
    }
}

fn import_local(specifier: &ImportSpecifier) -> &Ident {
    match specifier {
        ImportSpecifier::Named(named) => &named.local,
        ImportSpecifier::Default(default) => &default.local,
        ImportSpecifier::Namespace(namespace) => &namespace.local,
    }
}

fn import_target<'a>(
    node: &'a ModuleNode,
    id: &deno_ast::swc::ast::Id,
) -> Option<(&'a Target, Option<String>)> {
    let ProgramRef::Module(module) = node.parsed.program_ref() else {
        return None;
    };
    for item in &module.body {
        let ModuleItem::ModuleDecl(ModuleDecl::Import(import)) = item else {
            continue;
        };
        if import.type_only {
            continue;
        }
        for specifier in &import.specifiers {
            if specifier.is_type_only() || &import_local(specifier).to_id() != id {
                continue;
            }
            let target = node.targets.get(import.src.value.as_ref())?;
            let imported = match specifier {
                ImportSpecifier::Named(named) => Some(
                    named
                        .imported
                        .as_ref()
                        .map_or_else(|| named.local.sym.to_string(), export_name),
                ),
                ImportSpecifier::Default(_) => Some("default".to_owned()),
                ImportSpecifier::Namespace(_) => None,
            };
            return Some((target, imported));
        }
    }
    None
}

fn export_name(name: &ModuleExportName) -> String {
    name.atom().to_string()
}

fn statement_range(node: &ModuleNode, span: Span) -> Range<usize> {
    let mut range = node.range(span);
    let bytes = node.source.as_bytes();
    let mut cursor = range.end.min(bytes.len());
    loop {
        while cursor < bytes.len() && matches!(bytes[cursor], b' ' | b'\t' | 0x0b | 0x0c) {
            cursor += 1;
        }
        if cursor < bytes.len() && bytes[cursor] == b';' {
            cursor += 1;
            range.end = cursor;
            continue;
        }
        if cursor + 1 < bytes.len() && bytes[cursor..cursor + 2] == *b"/*" {
            if let Some(end) = bytes[cursor + 2..]
                .windows(2)
                .position(|window| window == b"*/")
            {
                cursor += end + 4;
                range.end = cursor;
                continue;
            }
        }
        if cursor + 1 < bytes.len() && bytes[cursor..cursor + 2] == *b"//" {
            while cursor < bytes.len() && !matches!(bytes[cursor], b'\r' | b'\n') {
                cursor += 1;
            }
            range.end = cursor;
        }
        break;
    }
    range
}

fn non_direct_supplier(span: Range<usize>) -> ExportSupplier {
    ExportSupplier {
        span,
        direct_const: None,
        reason: Some(ExportReason::NotDirectConst),
        runtime: true,
    }
}

fn declaration_error(
    node: &ModuleNode,
    span: Range<usize>,
    reason: DeclarationReason,
) -> ScriptDeclarationError {
    ScriptDeclarationError::new(&node.path, &node.source, span, "manifest", reason)
}

fn runtime_reference_error(node: &ModuleNode, span: Range<usize>) -> ScriptDeclarationError {
    declaration_error(
        node,
        span,
        DeclarationReason::Export(ExportReason::RuntimeReference),
    )
}

struct RuntimeIdentifierVisitor<'a> {
    bindings: Vec<&'a deno_ast::swc::ast::Id>,
    found: Option<Span>,
}

impl Visit for RuntimeIdentifierVisitor<'_> {
    fn visit_expr(&mut self, expression: &Expr) {
        if let Expr::Ident(ident) = expression {
            if self
                .bindings
                .iter()
                .any(|binding| **binding == ident.to_id())
            {
                self.found.get_or_insert(ident.span);
            }
        }
        expression.visit_children_with(self);
    }

    fn visit_prop(&mut self, property: &deno_ast::swc::ast::Prop) {
        if let deno_ast::swc::ast::Prop::Shorthand(identifier) = property {
            if self
                .bindings
                .iter()
                .any(|binding| **binding == identifier.to_id())
            {
                self.found.get_or_insert(identifier.span);
            }
        }
        property.visit_children_with(self);
    }
}

#[derive(Default)]
struct CompatMarkers {
    define_bot: bool,
    compat_property: bool,
    compat_declaration: bool,
}

impl Visit for CompatMarkers {
    fn visit_decl(&mut self, declaration: &Decl) {
        if declaration_value_idents(declaration)
            .iter()
            .any(|ident| ident.sym.as_ref() == "__rs2b0tManifest")
        {
            self.compat_declaration = true;
        }
        declaration.visit_children_with(self);
    }

    fn visit_call_expr(&mut self, call: &deno_ast::swc::ast::CallExpr) {
        if let deno_ast::swc::ast::Callee::Expr(callee) = &call.callee {
            if matches!(unwrap_expression(callee), Expr::Ident(ident) if ident.sym.as_ref() == "defineBot")
            {
                self.define_bot = true;
            }
        }
        call.visit_children_with(self);
    }

    fn visit_prop(&mut self, property: &deno_ast::swc::ast::Prop) {
        let key = match property {
            deno_ast::swc::ast::Prop::KeyValue(property) => Some(&property.key),
            deno_ast::swc::ast::Prop::Method(property) => Some(&property.key),
            deno_ast::swc::ast::Prop::Getter(property) => Some(&property.key),
            deno_ast::swc::ast::Prop::Setter(property) => Some(&property.key),
            deno_ast::swc::ast::Prop::Shorthand(_) | deno_ast::swc::ast::Prop::Assign(_) => None,
        };
        if key.is_some_and(|key| match key {
            deno_ast::swc::ast::PropName::Ident(ident) => ident.sym.as_ref() == "__rs2b0tManifest",
            deno_ast::swc::ast::PropName::Str(value) => value.value.as_ref() == "__rs2b0tManifest",
            _ => false,
        }) {
            self.compat_property = true;
        }
        property.visit_children_with(self);
    }
}

struct NamespaceReferenceVisitor<'a> {
    namespaces: &'a HashMap<deno_ast::swc::ast::Id, Target>,
    erased_names: &'a HashSet<String>,
    graph: &'a ModuleGraph,
    found: Option<Span>,
}

impl Visit for NamespaceReferenceVisitor<'_> {
    fn visit_expr(&mut self, expression: &Expr) {
        if let Expr::Member(member) = expression {
            if let Expr::Ident(namespace) = &*member.obj {
                if let Some(target) = self.namespaces.get(&namespace.to_id()) {
                    match &member.prop {
                        deno_ast::swc::ast::MemberProp::Ident(name) => {
                            if self.erased_names.contains(name.sym.as_ref())
                                && self.graph.target_exports_erased(
                                    target,
                                    name.sym.as_ref(),
                                    self.erased_names,
                                )
                            {
                                self.found.get_or_insert(member.span);
                            }
                        }
                        deno_ast::swc::ast::MemberProp::Computed(computed) => {
                            let key = match &*computed.expr {
                                Expr::Lit(deno_ast::swc::ast::Lit::Str(value)) => {
                                    Some(value.value.as_ref())
                                }
                                _ => None,
                            };
                            if key.map_or_else(
                                || {
                                    self.erased_names.iter().any(|name| {
                                        self.graph.target_exports_erased(
                                            target,
                                            name,
                                            self.erased_names,
                                        )
                                    })
                                },
                                |name| {
                                    self.erased_names.contains(name)
                                        && self.graph.target_exports_erased(
                                            target,
                                            name,
                                            self.erased_names,
                                        )
                                },
                            ) {
                                self.found.get_or_insert(member.span);
                            }
                        }
                        deno_ast::swc::ast::MemberProp::PrivateName(_) => {}
                    }
                    return;
                }
            }
        }
        if let Expr::Ident(ident) = expression {
            if let Some(target) = self.namespaces.get(&ident.to_id()) {
                if self.erased_names.iter().any(|name| {
                    self.graph
                        .target_exports_erased(target, name, self.erased_names)
                }) {
                    self.found.get_or_insert(ident.span);
                    return;
                }
            }
        }
        expression.visit_children_with(self);
    }
}

fn unwrap_expression(mut expr: &Expr) -> &Expr {
    loop {
        expr = match expr {
            Expr::Paren(wrapper) => &wrapper.expr,
            Expr::TsAs(wrapper) => &wrapper.expr,
            Expr::TsTypeAssertion(wrapper) => &wrapper.expr,
            Expr::TsConstAssertion(wrapper) => &wrapper.expr,
            Expr::TsSatisfies(wrapper) => &wrapper.expr,
            _ => return expr,
        };
    }
}
