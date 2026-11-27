use super::*;
use script::ScriptSource::{Catalog, File};

#[test]
fn operator_script_settings_import_exact_keys_and_retire_once() {
    crate::test_process::isolate_state_test!(
        operator_script_settings_import_exact_keys_and_retire_once
    );
    let scratch = IsolatedEnv::enter("operator-script-import");
    let root = root_of(&scratch);
    let path = root.path().join("script-settings.json");
    let bytes = br#"{"catalog:SmithingBot":{"bar":"Adamant","nested":[{"a:b":null},2]},"file:Upper:Case":{}}"#;
    write_private(&path, bytes);
    {
        let service = LocalStateService::open_process(root.clone());
        ready(&service.preferences());
        let settings = service.operator_script_settings();
        settings.wait_ready(WAIT).unwrap();
        assert_eq!(
            settings.card(Catalog, "SmithingBot").unwrap()["bar"],
            "Adamant"
        );
        assert_eq!(
            settings.card(Catalog, "SmithingBot").unwrap()["nested"],
            json!([{"a:b":null}, 2])
        );
        assert!(settings.card(File, "Upper:Case").unwrap().is_empty());
        assert!(settings.card(File, "upper:case").is_none());
        assert_eq!(sql::connections_opened_for_test(&root), 1);
        assert_eq!(
            std::fs::read(path.with_file_name("script-settings.json.migrated")).unwrap(),
            bytes
        );
        assert!(!path.exists());
        settings
            .set_value(Catalog, "SmithingBot", "bar", json!("Rune"))
            .unwrap()
            .wait(WAIT)
            .unwrap();
    }
    write_private(&path, b"not JSON");
    for expected_warnings in [1, 0] {
        let warnings = Arc::new(AtomicUsize::new(0));
        let observed = Arc::clone(&warnings);
        let service = LocalStateService::open_process_noticed_for_test(
            root.clone(),
            Arc::new(move |_, line| {
                if line.contains("retired") && line.contains("script-settings.json") {
                    observed.fetch_add(1, Ordering::Relaxed);
                }
            }),
        );
        let settings = service.operator_script_settings();
        settings.wait_ready(WAIT).unwrap();
        assert_eq!(settings.overrides(Catalog, "SmithingBot")["bar"], "Rune");
        assert!(settings.card(File, "Upper:Case").unwrap().is_empty());
        assert_eq!(warnings.load(Ordering::Relaxed), expected_warnings);
    }
    assert_eq!(std::fs::read(path).unwrap(), b"not JSON");
}

#[test]
fn operator_script_settings_duplicates_do_not_complete_import() {
    crate::test_process::isolate_state_test!(
        operator_script_settings_duplicates_do_not_complete_import
    );
    let scratch = IsolatedEnv::enter("operator-script-duplicates");
    let root = root_of(&scratch);
    let path = root.path().join("script-settings.json");
    for bytes in [
        br#"{"catalog:Card":{},"catalog:Card":{"x":1}}"#.as_slice(),
        br#"{"catalog:Card":{"nested":[{"key":1,"key":2}]}}"#.as_slice(),
    ] {
        write_private(&path, bytes);
        let service = LocalStateService::open_process(root.clone());
        let settings = service.operator_script_settings();
        let error = settings.wait_ready(WAIT).unwrap_err();
        assert!(matches!(error, ScriptSettingsError::Decode { .. }));
        assert_eq!(error.path(), path);
        assert!(error.to_string().contains("named twice"));
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        let ledger: u32 = inspect(&root)
            .query_row(
                "SELECT count(*) FROM import_ledger WHERE domain = 'script-settings'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(ledger, 0);
    }
    write_private(&path, br#"{"catalog:Card":{"nested":[{"key":2}]}}"#);
    let service = LocalStateService::open_process(root);
    let settings = service.operator_script_settings();
    settings.wait_ready(WAIT).unwrap();
    assert_eq!(
        settings.overrides(Catalog, "Card")["nested"],
        json!([{"key":2}])
    );
}

#[test]
fn operator_script_settings_publishes_after_commit_and_merges_queued_edits() {
    crate::test_process::isolate_state_test!(
        operator_script_settings_publishes_after_commit_and_merges_queued_edits
    );
    let scratch = IsolatedEnv::enter("operator-script-commit");
    let service = open(&scratch);
    let settings = service.operator_script_settings();
    settings.wait_ready(WAIT).unwrap();
    let (reached, signal) = mpsc::sync_channel(1);
    let (release, gate) = mpsc::sync_channel(1);
    let gate = std::sync::Mutex::new(Some(gate));
    service.set_sql_seams_for_test(sql::Seams {
        point: Some(Arc::new(move |point| {
            if point == "before-commit" {
                if let Some(gate) = gate.lock().unwrap().take() {
                    reached.send(()).unwrap();
                    gate.recv_timeout(WAIT).unwrap();
                }
            }
        })),
        ..Default::default()
    });
    let first = settings
        .set_value(Catalog, "Card", "one", json!(false))
        .unwrap();
    signal.recv_timeout(WAIT).unwrap();
    assert!(first.poll().is_none());
    assert!(settings.card(Catalog, "Card").is_none());
    let second = settings
        .set_value(Catalog, "Card", "two", json!([2]))
        .unwrap();
    release.send(()).unwrap();
    first.wait(WAIT).unwrap();
    second.wait(WAIT).unwrap();
    let bag = settings.card(Catalog, "Card").unwrap();
    assert_eq!(bag["one"], false);
    assert_eq!(bag["two"], json!([2]));
}

#[test]
fn preferences_recovery_carries_both_script_settings_domains() {
    crate::test_process::isolate_state_test!(
        preferences_recovery_carries_both_script_settings_domains
    );
    let scratch = IsolatedEnv::enter("recover-script-settings");
    let root = root_of(&scratch);
    let operator_path = root.path().join("script-settings.json");
    let runtime_path = root.script_runtime_settings_file("local-289", "alice", 289, "test-content");
    let identity = DomainIdentity::new(
        LocalStateDomain::ScriptRuntimeSettings,
        AccountKey::try_new("local-289", "alice").unwrap(),
        ContentScope::try_new(client::io::ClientRevision::R289, "test-content").unwrap(),
    );
    let raw = "  {\"raw\": 2}\n";
    write_private(
        &operator_path,
        br#"{"catalog:Card":{"old":null},"file:empty":{}}"#,
    );
    write_private(
        &runtime_path,
        &serde_json::to_vec(&json!({
            "schema_version": 1, "profile": "local-289", "account": "alice",
            "revision": 289, "content": "test-content",
            "rows": [{"namespace": "box", "key": "id", "raw": raw}],
        }))
        .unwrap(),
    );
    let version;
    {
        let service = open(&scratch);
        let operator = service.operator_script_settings();
        operator.wait_ready(WAIT).unwrap();
        operator
            .set_value(Catalog, "Card", "extra", json!([42]))
            .unwrap()
            .wait(WAIT)
            .unwrap();
        version = service
            .readback_lease(&identity, WAIT)
            .unwrap()
            .unwrap()
            .version();
        assert_eq!(sql::connections_opened_for_test(&root), 1);
    }
    assert!(!operator_path.exists());
    assert!(!runtime_path.exists());
    write_private(&operator_path, b"{}");
    write_private(&runtime_path, b"{}");

    let note =
        run_preferences_maintenance(&root, Frontend::Panel, &MaintenanceAction::RecoverDefaults)
            .unwrap();
    assert!(
        note.contains("2 operator card(s) and 1 runtime settings store(s)"),
        "{note}"
    );
    let service = open(&scratch);
    let operator = service.operator_script_settings();
    operator.wait_ready(WAIT).unwrap();
    assert!(operator.card(File, "empty").unwrap().is_empty());
    assert_eq!(operator.overrides(Catalog, "Card")["extra"], json!([42]));
    let lease = service.readback_lease(&identity, WAIT).unwrap().unwrap();
    assert_eq!(lease.version(), version);
    assert_eq!(
        lease.snapshot().as_script_runtime_settings().unwrap()[0].raw(),
        raw
    );
    assert_eq!(std::fs::read(operator_path).unwrap(), b"{}");
    assert_eq!(std::fs::read(runtime_path).unwrap(), b"{}");
}

#[test]
fn version_two_database_keeps_preferences_and_bank_data_when_settings_import() {
    crate::test_process::isolate_state_test!(
        version_two_database_keeps_preferences_and_bank_data_when_settings_import
    );
    let scratch = IsolatedEnv::enter("version-two-script-settings");
    let root = root_of(&scratch);
    {
        let service = open(&scratch);
        let preferences = service.preferences();
        ready(&preferences);
        save(&preferences, "ownership", json!("kept")).unwrap();
        let hint = submit_hint(&service, "alice", 1_760_000_000);
        assert!(matches!(
            service.wait_durable(hint, WAIT),
            Durability::Durable(_)
        ));
    }
    // Remove the additions after v2; none of these domains has been used yet.
    inspect(&root)
        .execute_batch(
            "BEGIN IMMEDIATE;
         DROP TABLE operator_script_card;
         DROP TABLE operator_script_head;
         DROP TABLE script_runtime_settings;
         DROP TABLE server_world;
         DROP TABLE server_profile;
         PRAGMA user_version = 2;
         COMMIT;",
        )
        .unwrap();
    write_private(
        &root.path().join("script-settings.json"),
        br#"{"file:Probe":{"x":1}}"#,
    );
    let service = open(&scratch);
    let operator = service.operator_script_settings();
    operator.wait_ready(WAIT).unwrap();
    assert_eq!(operator.overrides(File, "Probe")["x"], 1);
    let prefs = ready(&service.preferences());
    assert_eq!(prefs.document().get("ownership"), Some(&json!("kept")));
    let bank = service
        .readback_lease(&bank_identity("alice"), WAIT)
        .unwrap()
        .unwrap();
    assert_eq!(bank.snapshot().as_bank_hints().unwrap(), &[(995, 10)]);
}

#[test]
fn committed_script_settings_finish_pending_archive_on_restart() {
    crate::test_process::isolate_state_test!(
        committed_script_settings_finish_pending_archive_on_restart
    );
    let scratch = IsolatedEnv::enter("script-settings-pending-archive");
    let root = root_of(&scratch);
    let path = root.path().join("script-settings.json");
    let source = br#"{"file:Probe":{"exact":[null,2]}}"#;
    write_private(&path, source);
    let previous;
    {
        let service = LocalStateService::open_process_with_seams_for_test(
            root.clone(),
            sql::Seams {
                point: Some(Arc::new(|point| {
                    assert_ne!(point, "before-evidence", "interrupt before the archive");
                })),
                ..sql::Seams::default()
            },
            None,
        );
        assert!(service.operator_script_settings().wait_ready(WAIT).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), source);
        previous = inspect(&root)
            .query_row(
                "SELECT version FROM operator_script_head WHERE id = 1",
                [],
                |row| row.get::<_, Vec<u8>>(0),
            )
            .unwrap();
    }
    let service = open(&scratch);
    let operator = service.operator_script_settings();
    operator.wait_ready(WAIT).unwrap();
    assert_eq!(operator.overrides(File, "Probe")["exact"], json!([null, 2]));
    assert!(operator.take_notice().is_none());
    assert!(!path.exists());
    let (version, evidence, archive) = inspect(&root)
        .query_row(
            "SELECT version, evidence, path FROM operator_script_head, import_ledger
         WHERE domain = 'script-settings' AND id = 1",
            [],
            |row| {
                Ok((
                    row.get::<_, Vec<u8>>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            },
        )
        .unwrap();
    assert_eq!(version, previous);
    assert_eq!(evidence, "completed");
    assert_eq!(std::fs::read(archive).unwrap(), source);
}
