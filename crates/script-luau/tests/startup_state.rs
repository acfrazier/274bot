//! No Luau state exists before a run.
//!
//! A VM is created only by the spawn of an admitted Start of a Luau card, so no
//! source of this crate may hold state or run code before that Start. The scan
//! refuses a mutable or interior-mutable `static`, a `thread_local!`, an
//! attribute that runs code before `main`, any use of `std::thread`, and a VM
//! constructor. The slice that adds the run lifecycle adds the one constructor,
//! and then narrows the last rule to the file of the factory's spawn.

use std::path::{Path, PathBuf};
use syn::visit::Visit;

/// Types whose statics hold state that outlives a run. `Atomic*` joins them.
const STATE_TYPES: &[&str] = &[
    "Cell",
    "Condvar",
    "Lazy",
    "LazyCell",
    "LazyLock",
    "Mutex",
    "Once",
    "OnceCell",
    "OnceLock",
    "RefCell",
    "RwLock",
    "UnsafeCell",
];
const STATE_MACROS: &[&str] = &["lazy_static", "thread_local"];
/// Attributes that place code or data where the loader runs it before `main`.
const STARTUP_ATTRIBUTES: &[&str] = &["ctor", "dtor", "link_section", "used"];
/// The `mlua::Lua` associated functions that build a VM.
const VM_CONSTRUCTORS: &[&str] = &[
    "init_from_ptr",
    "new",
    "new_with",
    "unsafe_new",
    "unsafe_new_with",
];

#[derive(Default)]
struct Scan {
    violations: Vec<String>,
}

/// Collects the identifiers of every path segment inside one type.
#[derive(Default)]
struct TypeIdents(Vec<String>);

impl<'ast> Visit<'ast> for TypeIdents {
    fn visit_path_segment(&mut self, segment: &'ast syn::PathSegment) {
        self.0.push(segment.ident.to_string());
        syn::visit::visit_path_segment(self, segment);
    }
}

fn use_tree_idents(tree: &syn::UseTree, out: &mut Vec<String>) {
    match tree {
        syn::UseTree::Path(path) => {
            out.push(path.ident.to_string());
            use_tree_idents(&path.tree, out);
        }
        syn::UseTree::Name(name) => out.push(name.ident.to_string()),
        syn::UseTree::Rename(rename) => out.push(rename.ident.to_string()),
        syn::UseTree::Group(group) => {
            for item in &group.items {
                use_tree_idents(item, out);
            }
        }
        syn::UseTree::Glob(_) => {}
    }
}

impl<'ast> Visit<'ast> for Scan {
    fn visit_item_static(&mut self, item: &'ast syn::ItemStatic) {
        let name = &item.ident;
        if matches!(item.mutability, syn::StaticMutability::Mut(_)) {
            self.violations.push(format!("mutable static `{name}`"));
        }
        let mut idents = TypeIdents::default();
        idents.visit_type(&item.ty);
        if let Some(state) = idents
            .0
            .iter()
            .find(|ident| STATE_TYPES.contains(&ident.as_str()) || ident.starts_with("Atomic"))
        {
            self.violations
                .push(format!("static `{name}` holds state through `{state}`"));
        }
        syn::visit::visit_item_static(self, item);
    }

    fn visit_macro(&mut self, mac: &'ast syn::Macro) {
        if let Some(last) = mac.path.segments.last() {
            if STATE_MACROS.contains(&last.ident.to_string().as_str()) {
                self.violations.push(format!("macro `{}!`", last.ident));
            }
        }
        syn::visit::visit_macro(self, mac);
    }

    fn visit_attribute(&mut self, attribute: &'ast syn::Attribute) {
        if let Some(last) = attribute.path().segments.last() {
            if STARTUP_ATTRIBUTES.contains(&last.ident.to_string().as_str()) {
                self.violations
                    .push(format!("attribute `{}` runs before main", last.ident));
            }
        }
        syn::visit::visit_attribute(self, attribute);
    }

    fn visit_item_use(&mut self, item: &'ast syn::ItemUse) {
        let mut idents = Vec::new();
        use_tree_idents(&item.tree, &mut idents);
        if idents.iter().any(|ident| ident == "thread") {
            self.violations.push("use of `std::thread`".to_owned());
        }
        syn::visit::visit_item_use(self, item);
    }

    fn visit_path(&mut self, path: &'ast syn::Path) {
        if path
            .segments
            .iter()
            .any(|segment| segment.ident == "thread")
        {
            self.violations.push("path through `thread`".to_owned());
        }
        syn::visit::visit_path(self, path);
    }

    fn visit_expr_call(&mut self, call: &'ast syn::ExprCall) {
        if let syn::Expr::Path(callee) = &*call.func {
            let segments: Vec<String> = callee
                .path
                .segments
                .iter()
                .map(|segment| segment.ident.to_string())
                .collect();
            if let [.., owner, function] = segments.as_slice() {
                if owner == "Lua" && VM_CONSTRUCTORS.contains(&function.as_str()) {
                    self.violations
                        .push(format!("VM constructor `Lua::{function}`"));
                }
            }
        }
        syn::visit::visit_expr_call(self, call);
    }
}

fn scan_source(source: &str) -> Vec<String> {
    let file = syn::parse_file(source).expect("the source parses");
    let mut scan = Scan::default();
    scan.visit_file(&file);
    scan.violations
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).expect("the source directory reads") {
        let path = entry.expect("a directory entry reads").path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            out.push(path);
        }
    }
}

#[test]
fn crate_sources_hold_no_state_before_a_run() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    rust_files(&root, &mut files);
    assert!(files.iter().any(|file| file.ends_with("lib.rs")));
    for file in files {
        let source = std::fs::read_to_string(&file).expect("the source reads");
        let violations = scan_source(&source);
        assert!(
            violations.is_empty(),
            "{} holds state before a run: {violations:?}",
            file.display()
        );
    }
}

/// A scan that finds nothing passes for any crate, so each rule must fire on a
/// sample that breaks it, and the allowed forms must not.
#[test]
fn scan_flags_each_source_of_startup_state() {
    let violations = scan_source(
        r#"
        use std::thread;
        static COUNT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        static mut RAW: u32 = 0;
        static VM: std::sync::OnceLock<u8> = std::sync::OnceLock::new();
        thread_local! { static SLOT: u8 = 0; }
        #[ctor::ctor]
        fn boot() {}
        #[link_section = ".init_array"]
        #[used]
        static INIT: fn() = boot;
        fn make() { let _ = mlua::Lua::new(); }
        fn spawn() { std::thread::spawn(|| {}); }
        "#,
    );
    for expected in [
        "static `COUNT` holds state through `AtomicUsize`",
        "mutable static `RAW`",
        "static `VM` holds state through `OnceLock`",
        "macro `thread_local!`",
        "attribute `ctor` runs before main",
        "attribute `link_section` runs before main",
        "attribute `used` runs before main",
        "VM constructor `Lua::new`",
        "use of `std::thread`",
        "path through `thread`",
    ] {
        assert!(
            violations.iter().any(|found| found == expected),
            "{expected} not flagged: {violations:?}"
        );
    }
}

#[test]
fn scan_allows_immutable_tables_and_constants() {
    let violations = scan_source(
        r#"
        static NAMES: [&str; 2] = ["tick", "onStop"];
        const LIMIT: usize = 16;
        fn pure(lua: &mlua::Lua) -> usize { LIMIT + NAMES.len() + lua.used_memory() }
        "#,
    );
    assert!(violations.is_empty(), "{violations:?}");
}
