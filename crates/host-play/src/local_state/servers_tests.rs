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
