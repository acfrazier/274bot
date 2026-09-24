//! Static `import` / `export … from` declarations of a TS or JS module, read
//! from the `deno_ast` parse rather than a text scan. Type-only declarations
//! are flagged so load scans follow only what the transpiled module imports.

use deno_ast::swc::ast::{
    ExportSpecifier, ImportSpecifier, ModuleDecl, ModuleExportName, ModuleItem,
};
use deno_ast::ProgramRef;

/// One static module dependency of a source, in source order.
pub(crate) struct ModuleImport {
    /// The quoted module specifier.
    pub specifier: String,
    /// `(local, exported)` bindings of an `import` (default imports bind
    /// their local name to itself). Empty for re-exports and side-effect imports.
    pub bindings: Vec<(String, String)>,
    /// False when TS strips the whole declaration (`import type`,
    /// `import { type A }`, `export type { A } from`).
    pub runtime: bool,
}

/// Every static dependency of `source`. A source that does not parse as
/// TypeScript has none here; the transpile at Load reports that failure.
pub(crate) fn module_imports(source: &str) -> Vec<ModuleImport> {
    let Ok(specifier) = deno_ast::ModuleSpecifier::parse("file:///module.ts") else {
        return Vec::new();
    };
    let Ok(parsed) = deno_ast::parse_module(deno_ast::ParseParams {
        specifier,
        text: source.into(),
        media_type: deno_ast::MediaType::TypeScript,
        capture_tokens: false,
        scope_analysis: false,
        maybe_syntax: None,
    }) else {
        return Vec::new();
    };
    let ProgramRef::Module(module) = parsed.program_ref() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for item in &module.body {
        let ModuleItem::ModuleDecl(decl) = item else {
            continue;
        };
        match decl {
            ModuleDecl::Import(import) => {
                let bindings = import
                    .specifiers
                    .iter()
                    .filter_map(|spec| match spec {
                        ImportSpecifier::Named(named) => {
                            let local = named.local.sym.to_string();
                            let exported = named
                                .imported
                                .as_ref()
                                .map_or_else(|| local.clone(), export_name);
                            Some((local, exported))
                        }
                        ImportSpecifier::Default(default) => {
                            let local = default.local.sym.to_string();
                            Some((local.clone(), local))
                        }
                        ImportSpecifier::Namespace(_) => None,
                    })
                    .collect();
                let all_type_only = !import.specifiers.is_empty()
                    && import.specifiers.iter().all(
                        |spec| matches!(spec, ImportSpecifier::Named(named) if named.is_type_only),
                    );
                out.push(ModuleImport {
                    specifier: import.src.value.to_string(),
                    bindings,
                    runtime: !import.type_only && !all_type_only,
                });
            }
            ModuleDecl::ExportAll(export) => out.push(ModuleImport {
                specifier: export.src.value.to_string(),
                bindings: Vec::new(),
                runtime: !export.type_only,
            }),
            ModuleDecl::ExportNamed(export) => {
                let Some(src) = export.src.as_ref() else {
                    continue;
                };
                let all_type_only = !export.specifiers.is_empty()
                    && export.specifiers.iter().all(
                        |spec| matches!(spec, ExportSpecifier::Named(named) if named.is_type_only),
                    );
                out.push(ModuleImport {
                    specifier: src.value.to_string(),
                    bindings: Vec::new(),
                    runtime: !export.type_only && !all_type_only,
                });
            }
            _ => {}
        }
    }
    out
}

fn export_name(name: &ModuleExportName) -> String {
    name.atom().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn runtime(src: &str) -> Vec<String> {
        module_imports(src)
            .into_iter()
            .filter(|i| i.runtime)
            .map(|i| i.specifier)
            .collect()
    }

    #[test]
    fn type_only_declarations_are_not_runtime_imports() {
        let src = r#"
import type { DangerZoneRect } from './a.js';
import { type Task } from './b.js';
import { TaskBot, type Task as T2 } from './c.js';
export type { X } from './d.js';
export { type Y } from './e.js';
export * from './f.js';
export type * from './g.js';
import './h.js';
"#;
        assert_eq!(runtime(src), ["./c.js", "./f.js", "./h.js"]);
    }

    #[test]
    fn bindings_keep_local_and_exported_names() {
        let src = "import Card, { SETTINGS as CARD_SETTINGS, other } from './Card/Card.js';";
        let imports = module_imports(src);
        assert_eq!(imports.len(), 1);
        assert_eq!(
            imports[0].bindings,
            [
                ("Card".to_string(), "Card".to_string()),
                ("CARD_SETTINGS".to_string(), "SETTINGS".to_string()),
                ("other".to_string(), "other".to_string()),
            ]
        );
    }
}
