//! Reference conversion uses real private files and the single SQL owner.

use super::*;
use api::local_state::{CatalogueReferenceError, FileCardReference};
use rusqlite::Connection;
use std::path::Path;

fn fixture(tag: &str) -> (script::IsolatedEnv, StateRoot) {
    let env = script::IsolatedEnv::enter(tag);
    let root = StateRoot::new(env.dir.join("state-root"));
    std::fs::create_dir_all(root.path()).unwrap();
    (env, root)
}

fn private(root: &StateRoot, name: &str, raw: &str) {
    vault::write_private_file(&root.path().join(name), raw.as_bytes()).unwrap();
}

fn ledger(root: &StateRoot) -> Vec<(String, String)> {
    let conn = Connection::open(root.database_file()).unwrap();
    let mut stmt = conn.prepare("SELECT source, kind FROM import_ledger WHERE domain='catalogue-references' ORDER BY source").unwrap();
    stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

#[test]
fn catalogue_components_import_independently_conflict_independently_and_restart() {
    let _lease = crate::test_process::state();
    let (_env, root) = fixture("s6c-components");
    private(
        &root,
        "js-scripts.json",
        r#"[{"name":"same","path":"/missing/a/same.js"},{"name":"same","path":"/missing/b/same.js"},{"name":"unsafe","path":"relative.js"}]"#,
    );
    private(&root, "rs2b0t-path", "/unavailable/checkout\n");
    private(&root, "rs2b0t-import", " deferred \n");
    let service = LocalStateService::open_process(root.clone());
    let port = service.catalogue_references();
    let files = port.file_cards().unwrap();
    assert_eq!(files.value.len(), 3);
    assert_eq!(ledger(&root).len(), 1);
    assert!(matches!(
        port.save_file_cards(StoreVersion::INITIAL, Arc::clone(&files.value)),
        Err(CatalogueReferenceError::Conflict)
    ));
    let saved = port.saved_root().unwrap();
    assert_eq!(
        saved.value.as_deref(),
        Some(Path::new("/unavailable/checkout"))
    );
    let deferred = port.import_deferred().unwrap();
    assert!(deferred.value);
    assert_eq!(ledger(&root).len(), 3);
    assert_ne!(files.version, saved.version);
    assert_ne!(saved.version, deferred.version);
    let root_version = port
        .save_root(Some(saved.version), Some("/chosen/checkout".into()))
        .unwrap();
    let defer_version = port.save_deferred(Some(deferred.version), false).unwrap();
    let rows: Arc<[FileCardReference]> = vec![FileCardReference {
        name: "later".into(),
        path: "/later.js".into(),
    }]
    .into();
    let files_version = port
        .save_file_cards(files.version, Arc::clone(&rows))
        .unwrap();
    assert!(matches!(
        port.save_root(Some(saved.version), None),
        Err(CatalogueReferenceError::Conflict)
    ));
    assert!(matches!(
        port.save_deferred(Some(deferred.version), true),
        Err(CatalogueReferenceError::Conflict)
    ));
    let deferred_again = port.save_deferred(Some(defer_version), true).unwrap();
    drop(port);
    drop(service);
    let service = LocalStateService::open_process(root.clone());
    let port = service.catalogue_references();
    assert_eq!(
        port.file_cards().unwrap(),
        api::local_state::CatalogueComponent {
            version: files_version,
            value: rows
        }
    );
    assert_eq!(port.saved_root().unwrap().version, root_version);
    assert_eq!(
        port.saved_root().unwrap().value,
        Some("/chosen/checkout".into())
    );
    assert_eq!(
        port.import_deferred().unwrap(),
        api::local_state::CatalogueComponent {
            version: deferred_again,
            value: true
        }
    );
    assert_eq!(ledger(&root).len(), 3);
    for source in ["js-scripts.json", "rs2b0t-path", "rs2b0t-import"] {
        assert!(!root.path().join(source).exists());
        assert!(root.path().join(format!("{source}.migrated")).is_file());
    }
}

#[test]
fn catalogue_invalid_saved_roots_and_unknown_marker_retain_evidence() {
    let _lease = crate::test_process::state();
    for (index, raw) in ["relative/checkout", "/checkout/../other", "  \n"]
        .iter()
        .enumerate()
    {
        let (_env, root) = fixture(&format!("s6c-invalid-{index}"));
        private(&root, "rs2b0t-path", raw);
        private(&root, "rs2b0t-import", "future-choice");
        let service = LocalStateService::open_process(root.clone());
        let port = service.catalogue_references();
        assert_eq!(port.saved_root().unwrap().value, None);
        assert!(!port.import_deferred().unwrap().value);
        assert_eq!(
            std::fs::read_to_string(root.path().join("rs2b0t-path.migrated")).unwrap(),
            *raw
        );
        assert_eq!(
            std::fs::read_to_string(root.path().join("rs2b0t-import.migrated")).unwrap(),
            "future-choice"
        );
        assert_eq!(ledger(&root).len(), 2);
    }
}

#[test]
fn catalogue_refused_private_sources_commit_nothing_and_explicit_choices_leave_them_untouched() {
    let _lease = crate::test_process::state();
    let (_env, root) = fixture("s6c-private-refusal");
    for source in ["rs2b0t-path", "rs2b0t-import"] {
        std::fs::create_dir(root.path().join(source)).unwrap();
    }
    let service = LocalStateService::open_process(root.clone());
    let port = service.catalogue_references();
    assert!(port.saved_root().is_err());
    assert!(port.import_deferred().is_err());
    assert!(ledger(&root).is_empty());
    port.save_root(None, Some("/selected/root".into())).unwrap();
    port.save_deferred(None, true).unwrap();
    assert_eq!(
        port.saved_root().unwrap().value,
        Some("/selected/root".into())
    );
    assert!(port.import_deferred().unwrap().value);
    assert!(
        ledger(&root).is_empty(),
        "an explicit choice is not a successful source read"
    );
    for source in ["rs2b0t-path", "rs2b0t-import"] {
        assert!(root.path().join(source).is_dir());
        assert!(!root.path().join(format!("{source}.migrated")).exists());
    }
}

#[test]
fn catalogue_malformed_and_duplicate_file_lists_refuse_path_cause_and_remedy() {
    let _lease = crate::test_process::state();
    for (index, raw) in [
        r#"[{"name":"a","name":"b","path":"/a.js"}]"#,
        "[not-json]",
        "{}",
    ]
    .iter()
    .enumerate()
    {
        let (_env, root) = fixture(&format!("s6c-json-refusal-{index}"));
        private(&root, "js-scripts.json", raw);
        let service = LocalStateService::open_process(root.clone());
        let port = service.catalogue_references();
        let error = port.file_cards().unwrap_err().to_string();
        assert!(error.contains("js-scripts.json"), "{error}");
        assert!(
            error.contains("Correct") && error.contains("restart"),
            "{error}"
        );
        if index == 0 {
            assert!(error.contains("field \"name\" is named twice"), "{error}");
        }
        assert!(ledger(&root).is_empty());
        assert_eq!(
            std::fs::read_to_string(root.path().join("js-scripts.json")).unwrap(),
            *raw
        );
        let root_version = port.saved_root().unwrap().version;
        port.save_root(Some(root_version), Some("/independent".into()))
            .unwrap();
        assert!(port.file_cards().is_err());
    }
}

#[test]
fn catalogue_file_read_never_authorizes_another_component_and_failed_save_keeps_durable_value() {
    let _lease = crate::test_process::state();
    let (_env, root) = fixture("s6c-isolation");
    let service = LocalStateService::open_process(root.clone());
    let port = service.catalogue_references();
    let files = port.file_cards().unwrap();
    assert!(matches!(
        port.save_root(Some(files.version), Some("/wrong-version".into())),
        Err(CatalogueReferenceError::Conflict)
    ));
    assert!(matches!(
        port.save_deferred(Some(files.version), true),
        Err(CatalogueReferenceError::Conflict)
    ));
    assert_eq!(
        ledger(&root).len(),
        1,
        "a write cannot hydrate another component"
    );
    let saved = port.saved_root().unwrap();
    service.set_write_fault_for_test(Arc::new(|| true));
    assert!(port
        .save_root(Some(saved.version), Some("/not-saved".into()))
        .is_err());
    assert_eq!(port.saved_root().unwrap(), saved);
    assert_eq!(port.file_cards().unwrap(), files);
}
