use super::*;
use crate::servers::{LaunchProfile, LaunchTransport, LaunchWorld, LoginKey, Servers};
use rusqlite::OptionalExtension;
use script::IsolatedEnv;

const WAIT: Duration = Duration::from_secs(10);

fn custom() -> Servers {
    let world = |number, host: &str| LaunchWorld {
        number,
        host: host.into(),
        port: 40000 + number,
        node_id: i32::from(number) + 100,
        asset_host: format!("assets-{host}"),
        asset_port: 8000 + number,
    };
    Servers {
        schema_version: 1,
        servers: vec![LaunchProfile {
            name: "Custom".into(),
            revision: 377,
            transport: LaunchTransport::Tcp,
            worlds: vec![world(2, "game.example"), world(1, "other.example")],
            login_key: LoginKey::Inline {
                modulus: "12345678901234567890".into(),
                exponent: "65537".into(),
            },
            vault: "custom-vault".into(),
            members: Some(false),
            allow_plaintext_offhost: true,
        }],
    }
}

fn fixture() -> (IsolatedEnv, StateRoot) {
    let env = IsolatedEnv::enter("servers");
    let root = StateRoot::new(env.home.join(".274bot"));
    vault::create_private_dir(root.path()).unwrap();
    (env, root)
}

fn seed(root: &StateRoot, servers: &Servers) {
    vault::write_private_file(
        &root.path().join("servers.json"),
        &serde_json::to_vec(servers).unwrap(),
    )
    .unwrap();
}

fn roster(root: &StateRoot) -> (Arc<LocalStateService>, ServerRoster) {
    let service = LocalStateService::open_process(root.clone());
    let roster = service.wait_server_roster(WAIT).unwrap();
    (service, roster)
}

fn count(root: &StateRoot, query: &str) -> i64 {
    rusqlite::Connection::open(root.database_file())
        .unwrap()
        .query_row(query, [], |row| row.get(0))
        .unwrap()
}

#[test]
fn custom_roster_imports_before_unlock_and_preserves_durable_selection() {
    let _state_lease = crate::test_process::state();
    let (_env, root) = fixture();
    let expected = custom();
    seed(&root, &expected);
    let (service, first) = roster(&root);
    assert_eq!(first.servers(), &expected);
    assert_eq!(
        first.servers().resolve("CUSTOM").unwrap(),
        expected.servers[0]
    );
    assert!(!root.path().join("custom-vault").exists());
    assert!(!root.path().join("servers.json").exists());
    assert!(root.path().join("servers.json.migrated").is_file());
    let version = first.version();
    drop(service);
    let (_service, reopened) = roster(&root);
    assert_eq!(reopened.version(), version);
    assert_eq!(reopened.servers(), &expected);
}

#[test]
fn absent_source_seeds_sql_only_and_dormant_worlds_does_not_reopen_import() {
    let _state_lease = crate::test_process::state();
    let (_env, root) = fixture();
    let (service, first) = roster(&root);
    assert_eq!(first.servers(), &Servers::builtins());
    assert!(!root.path().join("servers.json").exists());
    let version = first.version();
    drop(service);
    vault::write_private_file(&root.path().join("worlds.json"), b"old worlds").unwrap();
    let (_service, reopened) = roster(&root);
    assert_eq!(reopened.version(), version);
    assert_eq!(reopened.servers(), first.servers());
}

#[test]
fn standalone_worlds_and_duplicate_members_never_commit_roster_or_ledger() {
    let _state_lease = crate::test_process::state();
    for source in ["worlds.json", "servers.json"] {
        let (_env, root) = fixture();
        let bytes = if source == "worlds.json" {
            b"old worlds".to_vec()
        } else {
            serde_json::to_vec(&custom()).unwrap()
        };
        let bytes = if source == "servers.json" {
            String::from_utf8(bytes)
                .unwrap()
                .replace("\"node_id\":102", "\"node_id\":102,\"node_id\":103")
                .into_bytes()
        } else {
            bytes
        };
        let path = root.path().join(source);
        vault::write_private_file(&path, &bytes).unwrap();
        let service = LocalStateService::open_process(root.clone());
        let error = match service.wait_server_roster(WAIT) {
            Ok(_) => panic!("invalid source was accepted"),
            Err(error) => error,
        };
        if source == "worlds.json" {
            assert!(error.to_string().contains("before 0.2.8"), "{error}");
        } else {
            assert!(
                matches!(error, ServerRosterError::Content { .. }),
                "{error}"
            );
        }
        assert_eq!(std::fs::read(path).unwrap(), bytes);
        assert_eq!(
            count(
                &root,
                "SELECT count(*) FROM domain_head WHERE domain='servers'"
            ),
            0
        );
        assert_eq!(
            count(
                &root,
                "SELECT count(*) FROM import_ledger WHERE domain='servers'"
            ),
            0
        );
    }
}

#[test]
fn closed_import_reports_replacement_once_without_parsing_or_changing_head() {
    let _state_lease = crate::test_process::state();
    let (_env, root) = fixture();
    seed(&root, &custom());
    let (service, first) = roster(&root);
    drop(service);
    let source = root.path().join("servers.json");
    vault::write_private_file(&source, b"not JSON at all").unwrap();
    let (service, reopened) = roster(&root);
    assert_eq!(reopened.version(), first.version());
    assert_eq!(reopened.servers(), first.servers());
    let notices = service.take_server_notices();
    assert_eq!(notices.len(), 1, "{notices:?}");
    assert!(notices[0].contains(&source.display().to_string()));
    assert!(notices[0].contains("roster editor"));
    assert!(service.take_server_notices().is_empty());
    drop(service);
    vault::write_private_file(&source, b"changed again").unwrap();
    let (service, reopened) = roster(&root);
    assert_eq!(reopened.version(), first.version());
    assert!(service.take_server_notices().is_empty());
}

#[test]
fn competing_first_starts_share_one_import_version() {
    let _state_lease = crate::test_process::state();
    let (_env, root) = fixture();
    seed(&root, &custom());
    let barrier = Arc::new(std::sync::Barrier::new(3));
    let handles: Vec<_> = (0..2)
        .map(|_| {
            let root = root.clone();
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || {
                barrier.wait();
                let (_service, roster) = roster(&root);
                (roster.version(), roster.servers().clone())
            })
        })
        .collect();
    barrier.wait();
    let results: Vec<_> = handles
        .into_iter()
        .map(|handle| handle.join().unwrap())
        .collect();
    assert_eq!(results[0], results[1]);
    assert_eq!(results[0].1, custom());
    assert_eq!(
        count(
            &root,
            "SELECT count(*) FROM import_ledger WHERE domain='servers'"
        ),
        1
    );
    let ledger: Option<String> = rusqlite::Connection::open(root.database_file())
        .unwrap()
        .query_row(
            "SELECT kind FROM import_ledger WHERE domain='servers' AND source='servers.json'",
            [],
            |row| row.get(0),
        )
        .optional()
        .unwrap();
    assert_eq!(ledger.as_deref(), Some("imported"));
}

fn custom_profile(name: &str, vault: &str, port: u16) -> LaunchProfile {
    LaunchProfile {
        name: name.into(),
        revision: 289,
        transport: LaunchTransport::Tcp,
        worlds: vec![LaunchWorld {
            number: 1,
            host: "127.0.0.1".into(),
            port,
            node_id: 1,
            asset_host: "127.0.0.1".into(),
            asset_port: 1080,
        }],
        login_key: LoginKey::EngineDir { engine_dir: None },
        vault: vault.into(),
        members: None,
        allow_plaintext_offhost: false,
    }
}

#[test]
fn add_edit_remove_and_audit_persist_across_restart() {
    let _state_lease = crate::test_process::state();
    let (_env, root) = fixture();
    let (service, initial) = roster(&root);
    assert!(initial.servers().resolve("fr-r377-beta").is_ok());

    let audits = Arc::new(Mutex::new(Vec::<AuditCommit>::new()));
    let sink = Arc::clone(&audits);
    service.set_audit_hook(Arc::new(move |audit: &AuditCommit| {
        sink.lock().push(audit.clone());
    }));

    let added = service
        .add_server(
            initial.version(),
            custom_profile("MyServer", "vault-myserver", 45594),
            WAIT,
        )
        .unwrap();
    assert!(added.version() > initial.version());
    assert_eq!(
        added.servers().resolve("myserver").unwrap().worlds[0].port,
        45594
    );

    let mut updated_profile = custom_profile("MyServer", "vault-myserver", 46594);
    updated_profile.members = Some(true);
    let edited = service
        .update_server(added.version(), "myserver", updated_profile.clone(), WAIT)
        .unwrap();
    assert_eq!(
        edited.servers().resolve("MyServer").unwrap(),
        updated_profile
    );

    let removed = service
        .remove_server(edited.version(), "MyServer", WAIT)
        .unwrap();
    assert!(removed.servers().resolve("MyServer").is_err());

    let recorded = audits.lock().clone();
    assert_eq!(recorded.len(), 3);
    assert!(recorded.iter().all(|a| {
        a.identity().domain() == LocalStateDomain::Servers
            && a.action() == CommitAction::ServersWrite
    }));

    drop(service);
    let (_service, reopened) = roster(&root);
    assert_eq!(reopened.version(), removed.version());
    assert!(reopened.servers().resolve("MyServer").is_err());
}

#[test]
fn s8_vault_bindings_and_pending_writes_are_enforced_at_edit_time() {
    let _state_lease = crate::test_process::state();
    let (_env, root) = fixture();
    let (service, initial) = roster(&root);

    let added = service
        .add_server(
            initial.version(),
            custom_profile("bound-srv", "vault-bound", 45594),
            WAIT,
        )
        .unwrap();
    service
        .bind_created("bound-srv", "vault-bound")
        .unwrap()
        .unwrap();
    service
        .create_profiles(
            "bound-srv",
            vec![NewProfileRow {
                account: "alice".into(),
                username: "Alice".into(),
                uid: 10,
                settings: vault::ProfileSettings::default(),
            }],
        )
        .unwrap()
        .unwrap();

    let mut bad_vault = custom_profile("bound-srv", "vault-other", 45594);
    let err = service
        .update_server(added.version(), "bound-srv", bad_vault.clone(), WAIT)
        .unwrap_err();
    assert!(
        matches!(err, ServerRosterMutationError::Binding(_)),
        "{err}"
    );

    bad_vault.name = "renamed-srv".into();
    bad_vault.vault = "vault-bound".into();
    let err = service
        .update_server(added.version(), "bound-srv", bad_vault, WAIT)
        .unwrap_err();
    assert!(
        matches!(err, ServerRosterMutationError::Binding(_)),
        "{err}"
    );

    let same_ns = custom_profile("Bound-Srv", "vault-bound", 45600);
    let kept = service
        .update_server(added.version(), "bound-srv", same_ns, WAIT)
        .unwrap();
    assert_eq!(
        kept.servers().resolve("bound-srv").unwrap().name,
        "Bound-Srv"
    );

    let removed = service
        .remove_server(kept.version(), "bound-srv", WAIT)
        .unwrap();
    assert!(removed.servers().resolve("bound-srv").is_err());
    let state = service.profile_state("bound-srv", "vault-bound").unwrap();
    assert_eq!(state.binding.as_ref().unwrap().component, "vault-bound");
    assert_eq!(state.rows.len(), 1);

    let err = service
        .add_server(
            removed.version(),
            custom_profile("thief", "vault-bound", 45601),
            WAIT,
        )
        .unwrap_err();
    assert!(
        matches!(err, ServerRosterMutationError::Binding(_)),
        "{err}"
    );

    let err = service
        .add_server(
            removed.version(),
            custom_profile("bound-srv", "vault-wrong", 45601),
            WAIT,
        )
        .unwrap_err();
    assert!(
        matches!(err, ServerRosterMutationError::Binding(_)),
        "{err}"
    );

    let restored = service
        .add_server(
            removed.version(),
            custom_profile("bound-srv", "vault-bound", 45601),
            WAIT,
        )
        .unwrap();
    assert!(restored.servers().resolve("bound-srv").is_ok());
}

#[test]
fn existing_root_adds_shipped_preset_without_overwriting_or_reviving() {
    let _state_lease = crate::test_process::state();
    let (_env, root) = fixture();
    seed(&root, &custom());
    let (service, imported) = roster(&root);
    assert!(imported.servers().resolve("fr-r377-beta").is_err());
    assert!(imported
        .servers()
        .available_presets()
        .iter()
        .any(|p| p.name == "fr-r377-beta"));

    let with_beta = service
        .add_server_preset(imported.version(), "fr-r377-beta", WAIT)
        .unwrap();
    assert_eq!(with_beta.servers().servers.len(), 2);
    assert_eq!(with_beta.servers().servers[0], custom().servers[0]);
    assert_eq!(
        with_beta.servers().resolve("fr-r377-beta").unwrap(),
        Servers::shipped_preset("fr-r377-beta").unwrap()
    );

    let removed = service
        .remove_server(with_beta.version(), "fr-r377-beta", WAIT)
        .unwrap();
    drop(service);
    let (_service, reopened) = roster(&root);
    assert_eq!(reopened.version(), removed.version());
    assert!(reopened.servers().resolve("fr-r377-beta").is_err());
    assert_eq!(reopened.servers().servers, custom().servers);
}

#[test]
fn concurrent_roster_edits_with_same_expected_version_commit_one_and_conflict_one() {
    let _state_lease = crate::test_process::state();
    let (_env, root) = fixture();
    let (service, initial) = roster(&root);
    let expected = initial.version();
    let barrier = Arc::new(std::sync::Barrier::new(2));
    let handles: Vec<_> = [
        ("race-a", "vault-race-a", 45701),
        ("race-b", "vault-race-b", 45702),
    ]
    .into_iter()
    .map(|(name, vault, port)| {
        let svc = Arc::clone(&service);
        let barrier = Arc::clone(&barrier);
        std::thread::spawn(move || {
            barrier.wait();
            svc.add_server(expected, custom_profile(name, vault, port), WAIT)
        })
    })
    .collect();
    let results: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
    let winners: Vec<_> = results.iter().filter_map(|r| r.as_ref().ok()).collect();
    let conflicts: Vec<_> = results.iter().filter_map(|r| r.as_ref().err()).collect();
    assert_eq!(winners.len(), 1, "expected exactly one winner: {results:?}");
    assert_eq!(
        conflicts.len(),
        1,
        "expected exactly one conflict: {results:?}"
    );
    assert_eq!(
        conflicts[0],
        &ServerRosterMutationError::Conflict {
            expected,
            found: winners[0].version(),
        }
    );
}

#[test]
fn cli_state_servers_show_is_readonly_and_mutators_enforce_maintenance_lease() {
    let _state_lease = crate::test_process::state();
    let (_env, root) = fixture();
    seed(&root, &custom());
    let (service, _initial) = roster(&root);
    let show = parse_maintenance(
        &["state", "servers", "show", "--json"]
            .into_iter()
            .map(String::from)
            .collect::<Vec<_>>(),
        Frontend::Headless,
    )
    .unwrap()
    .unwrap();
    let shown = run_maintenance(&root, Frontend::Headless, &show, &|_| false).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&shown).unwrap();
    assert_eq!(parsed["servers"].as_array().unwrap().len(), 1);
    assert!(parsed["available_presets"]
        .as_array()
        .unwrap()
        .iter()
        .any(|v| v == "fr-r377-beta"));

    let add_preset = parse_maintenance(
        &["state", "servers", "add", "--preset", "fr-r377-beta"]
            .into_iter()
            .map(String::from)
            .collect::<Vec<_>>(),
        Frontend::Headless,
    )
    .unwrap()
    .unwrap();
    let err = run_maintenance(&root, Frontend::Headless, &add_preset, &|_| false).unwrap_err();
    assert!(
        err.message().contains("shared root lease") && err.message().contains("panel or TUI"),
        "{err}"
    );

    drop(service);
    let report = run_maintenance(&root, Frontend::Headless, &add_preset, &|_| false).unwrap();
    assert!(report.contains("fr-r377-beta"), "{report}");

    let add_custom = parse_maintenance(
        &[
            "state",
            "servers",
            "add",
            "--name",
            "cli-custom",
            "--revision",
            "289",
            "--transport",
            "tcp",
            "--vault",
            "vault-cli-custom",
            "--world",
            "1:127.0.0.1:45894:1:127.0.0.1:1080",
        ]
        .into_iter()
        .map(String::from)
        .collect::<Vec<_>>(),
        Frontend::Headless,
    )
    .unwrap()
    .unwrap();
    run_maintenance(&root, Frontend::Headless, &add_custom, &|_| false).unwrap();

    let edit_custom = parse_maintenance(
        &[
            "state",
            "servers",
            "edit",
            "cli-custom",
            "--members",
            "true",
        ]
        .into_iter()
        .map(String::from)
        .collect::<Vec<_>>(),
        Frontend::Headless,
    )
    .unwrap()
    .unwrap();
    run_maintenance(&root, Frontend::Headless, &edit_custom, &|_| false).unwrap();

    let remove_custom = parse_maintenance(
        &["state", "servers", "remove", "cli-custom"]
            .into_iter()
            .map(String::from)
            .collect::<Vec<_>>(),
        Frontend::Headless,
    )
    .unwrap()
    .unwrap();
    run_maintenance(&root, Frontend::Headless, &remove_custom, &|_| false).unwrap();

    let (_service, final_roster) = roster(&root);
    assert!(final_roster.servers().resolve("fr-r377-beta").is_ok());
    assert!(final_roster.servers().resolve("cli-custom").is_err());
}
