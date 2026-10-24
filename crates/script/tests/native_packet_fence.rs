//! Syntax checks for the native/external packet-authority boundary.
use std::path::{Path, PathBuf};
use syn::visit::{self, Visit};

const ADVANCED_NAMES: &[&str] = &[
    "AdvancedPacketOps",
    "ScriptPacketOpsPermit",
    "ScriptPacketOpsContext",
    "packet_context_for_test",
    "packet_context_for_test_with_debug",
];

#[derive(Default)]
struct NativePaths {
    forbidden: Vec<String>,
}

impl<'ast> Visit<'ast> for NativePaths {
    fn visit_path_segment(&mut self, segment: &'ast syn::PathSegment) {
        if ADVANCED_NAMES.iter().any(|name| segment.ident == *name) {
            self.forbidden.push(segment.ident.to_string());
        }
        visit::visit_path_segment(self, segment);
    }

    fn visit_use_tree(&mut self, tree: &'ast syn::UseTree) {
        let name = match tree {
            syn::UseTree::Path(path) => {
                if path.ident == "surface" && matches!(path.tree.as_ref(), syn::UseTree::Glob(_)) {
                    self.forbidden.push("external surface glob import".into());
                }
                Some(&path.ident)
            }
            syn::UseTree::Name(name) => Some(&name.ident),
            syn::UseTree::Rename(rename) => Some(&rename.ident),
            syn::UseTree::Glob(_) => None,
            syn::UseTree::Group(_) => None,
        };
        if let Some(name) = name {
            if ADVANCED_NAMES.iter().any(|forbidden| name == forbidden) {
                self.forbidden.push(name.to_string());
            }
        }
        visit::visit_use_tree(self, tree);
    }
}

fn rust_files(root: &Path, files: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(root).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            rust_files(&path, files);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            files.push(path);
        }
    }
}

#[test]
fn native_sources_cannot_name_external_packet_authority() {
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    rust_files(&source, &mut files);
    files.sort();
    let mut scanned = 0;
    for path in files {
        let relative = path.strip_prefix(&source).unwrap();
        let first = relative.components().next().unwrap().as_os_str();
        let filename = path.file_name().unwrap().to_string_lossy();
        let is_native = first == "native"
            || first == "gatherer"
            || first == "combat"
            || filename.starts_with("native_")
            || filename == "combat_session.rs";
        if !is_native {
            continue;
        }
        let syntax = syn::parse_file(&std::fs::read_to_string(&path).unwrap()).unwrap();
        let mut paths = NativePaths::default();
        paths.visit_file(&syntax);
        assert!(
            paths.forbidden.is_empty(),
            "{}: {:?}",
            relative.display(),
            paths.forbidden
        );
        scanned += 1;
    }
    assert!(
        scanned > 20,
        "native source selection must not become empty"
    );
}

#[test]
fn advanced_envelope_permit_and_context_have_no_public_fields_or_constructors() {
    let syntax = syn::parse_file(include_str!("../src/surface.rs")).unwrap();
    let sealed = [
        "ServerOrderedEnvelope",
        "AdvancedPacketOps",
        "ScriptPacketOpsPermit",
        "ScriptPacketOpsContext",
    ];
    for name in sealed {
        let structure = syntax
            .items
            .iter()
            .find_map(|item| match item {
                syn::Item::Struct(item) if item.ident == name => Some(item),
                _ => None,
            })
            .unwrap();
        assert!(
            structure
                .fields
                .iter()
                .all(|field| matches!(field.vis, syn::Visibility::Inherited)),
            "{name} must not expose construction fields"
        );
    }
    for item in &syntax.items {
        let syn::Item::Impl(item) = item else {
            continue;
        };
        let syn::Type::Path(target) = item.self_ty.as_ref() else {
            continue;
        };
        let Some(target) = target.path.segments.last() else {
            continue;
        };
        if !sealed.iter().any(|name| target.ident == *name) {
            continue;
        }
        for member in &item.items {
            let syn::ImplItem::Fn(function) = member else {
                continue;
            };
            if !matches!(function.vis, syn::Visibility::Public(_)) {
                continue;
            }
            assert!(
                !matches!(function.sig.output, syn::ReturnType::Type(_, ref ty)
                if matches!(ty.as_ref(), syn::Type::Path(path)
                    if path.path.is_ident("Self") || path.path.is_ident(&target.ident.to_string()))),
                "public constructor escapes the external authority boundary"
            );
        }
    }
}
