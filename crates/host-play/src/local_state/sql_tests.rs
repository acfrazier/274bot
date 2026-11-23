//! T5.3 S4: the preferences database behind the process service, with
//! real SQLite, real files and, where the plan asks for it, real child
//! processes (`--ignored --exact` re-runs of this test binary).

use super::*;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc;

use api::local_state::{
    AccountKey, BankHint, ContentScope, DomainEdit, DomainIdentity, PreferenceChange,
    PreferencesEdit,
};
use rusqlite::{params, Connection, OpenFlags, OptionalExtension};
use script::IsolatedEnv;
use serde_json::json;

const WAIT: Duration = Duration::from_secs(60);

fn root_of(scratch: &IsolatedEnv) -> StateRoot {
    StateRoot::new(scratch.home.join(".274bot"))
}

fn open(scratch: &IsolatedEnv) -> Arc<LocalStateService> {
    LocalStateService::open_process(root_of(scratch))
}

/// The durable snapshot once the database is open, failing on a refusal.
fn ready(preferences: &Preferences) -> PreferenceSnapshot {
    let deadline = Instant::now() + WAIT;
    loop {
        match preferences.snapshot() {
            Ok(snapshot) => return snapshot,
            Err(error) if error.is_not_ready() => {}
            Err(error) => panic!("preferences refused: {error}"),
        }
        assert!(Instant::now() < deadline, "preferences never opened");
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// The refusal the database open ends with.
fn refusal(preferences: &Preferences) -> PreferenceError {
    let deadline = Instant::now() + WAIT;
    loop {
        match preferences.snapshot() {
            Ok(snapshot) => panic!(
                "preferences opened at {} but a refusal was expected",
                snapshot.version()
            ),
            Err(error) if error.is_not_ready() => {}
            Err(error) => return error,
        }
        assert!(Instant::now() < deadline, "preferences never settled");
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// Submit one member and wait for its terminal.
fn save(
    preferences: &Preferences,
    key: &str,
    value: serde_json::Value,
) -> Result<(), PreferenceError> {
    match preferences
        .begin_set(key, value)
        .expect("the save is accepted")
    {
        Update::Durable => Ok(()),
        Update::Queued(mut update) => update.wait(WAIT).expect("the save settles"),
    }
}

fn write_private(path: &std::path::Path, bytes: &[u8]) {
    if let Some(parent) = path.parent() {
        vault::create_private_dir(parent).unwrap();
    }
    vault::write_private_file(path, bytes).unwrap();
}

/// A connection of the test's own, for inspection.
fn inspect(root: &StateRoot) -> Connection {
    let conn = Connection::open_with_flags(
        root.database_file(),
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .unwrap();
    conn.busy_timeout(Duration::from_secs(5)).unwrap();
    conn
}

fn head(root: &StateRoot) -> u64 {
    let raw: Vec<u8> = inspect(root)
        .query_row(
            "SELECT version FROM domain_head WHERE domain = 'preferences'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    sql::decode_u64(&raw).unwrap()
}

fn ledger_kind(root: &StateRoot) -> Option<String> {
    inspect(root)
        .query_row("SELECT kind FROM legacy_source WHERE id = 1", [], |row| {
            row.get(0)
        })
        .optional()
        .unwrap()
}

fn ledger_evidence(root: &StateRoot) -> Option<String> {
    inspect(root)
        .query_row(
            "SELECT evidence FROM legacy_source WHERE id = 1",
            [],
            |row| row.get(0),
        )
        .optional()
        .unwrap()
}

fn preserved_sets(root: &StateRoot) -> usize {
    std::fs::read_dir(root.state_dir())
        .unwrap()
        .filter_map(|entry| entry.ok())
        .filter(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with("preserved-")
        })
        .count()
}

/// The stored preferences document, read through the test's own connection.
fn stored_document(root: &StateRoot) -> serde_json::Value {
    let body: Vec<u8> = inspect(root)
        .query_row(
            "SELECT body FROM preference_document WHERE id = 1",
            [],
            |row| row.get(0),
        )
        .unwrap();
    serde_json::from_slice(&body).unwrap()
}

fn wal_frames(conn: &Connection) -> i64 {
    conn.query_row("PRAGMA main.wal_checkpoint(NOOP)", [], |row| row.get(1))
        .unwrap()
}

fn epoch_of(root: &StateRoot) -> u64 {
    let raw: Vec<u8> = inspect(root)
        .query_row("SELECT epoch FROM store_meta WHERE id = 1", [], |row| {
            row.get(0)
        })
        .unwrap();
    sql::decode_u64(&raw).unwrap()
}

fn export_state(root: &StateRoot) -> Option<String> {
    inspect(root)
        .query_row("SELECT state FROM export_head WHERE id = 1", [], |row| {
            row.get(0)
        })
        .optional()
        .unwrap()
}

fn evidence_state(root: &StateRoot) -> Option<String> {
    inspect(root)
        .query_row(
            "SELECT evidence FROM legacy_source WHERE id = 1",
            [],
            |row| row.get(0),
        )
        .optional()
        .unwrap()
}

/// A reader that pins the WAL at its current end, so no checkpoint can
/// reset it.
fn pin_reader(root: &StateRoot) -> Connection {
    let reader = inspect(root);
    reader.execute_batch("BEGIN").unwrap();
    let _: i64 = reader
        .query_row("SELECT count(*) FROM preference_document", [], |row| {
            row.get(0)
        })
        .unwrap();
    reader
}

/// Grow the WAL, beside a pinned reader, to exactly `frames` frames with
/// one-page transactions on a scratch table of the test's own.
fn fill_wal_to(root: &StateRoot, frames: u64) {
    let writer = inspect(root);
    writer
        .execute_batch(
            "PRAGMA synchronous = OFF; PRAGMA wal_autocheckpoint = 0; \
             CREATE TABLE IF NOT EXISTS junk (id INTEGER PRIMARY KEY, v BLOB); \
             INSERT OR IGNORE INTO junk VALUES (1, zeroblob(64));",
        )
        .unwrap();
    while (wal_frames(&writer) as u64) < frames {
        writer
            .execute("UPDATE junk SET v = randomblob(64) WHERE id = 1", [])
            .unwrap();
    }
    assert_eq!(wal_frames(&writer) as u64, frames, "the fill is exact");
}

/// [`fill_wal_to`] over `pages` distinct database pages (one 3 KB row
/// each), so the idle TRUNCATE has that many pages to copy: its worst case
/// is the whole page cap, not a log of rewrites of one page.
fn fill_wal_spread(root: &StateRoot, frames: u64, pages: u32) {
    let writer = inspect(root);
    writer
        .execute_batch(
            "PRAGMA synchronous = OFF; PRAGMA wal_autocheckpoint = 0; \
             CREATE TABLE IF NOT EXISTS spread (id INTEGER PRIMARY KEY, v BLOB); BEGIN;",
        )
        .unwrap();
    for id in 1..=pages {
        writer
            .execute(
                "INSERT OR IGNORE INTO spread VALUES (?1, zeroblob(3000))",
                params![id],
            )
            .unwrap();
    }
    writer.execute_batch("COMMIT").unwrap();
    let mut id = 0;
    while (wal_frames(&writer) as u64) < frames {
        id = id % pages + 1;
        writer
            .execute(
                "UPDATE spread SET v = randomblob(3000) WHERE id = ?1",
                params![id],
            )
            .unwrap();
    }
    assert_eq!(wal_frames(&writer) as u64, frames, "the fill is exact");
}

fn wal_bytes(root: &StateRoot) -> u64 {
    let [_, wal, _] = sql::database_set(root);
    std::fs::metadata(wal).map_or(0, |meta| meta.len())
}

fn bank_identity(account: &str) -> DomainIdentity {
    DomainIdentity::new(
        LocalStateDomain::BankHints,
        AccountKey::try_new("local-289", account).unwrap(),
        ContentScope::try_new(client::io::ClientRevision::R289, "test-content").unwrap(),
    )
}

fn bank_hint(account: &str, at: u64) -> DomainEdit {
    DomainEdit::Bank(
        BankHint::try_new(
            bank_identity(account),
            None,
            vec![(995, 10)],
            DurableTimestamp::from_unix_seconds(at),
        )
        .unwrap(),
    )
}

/// Submit a bank hint, retrying while its store hydrates (a cold store
/// answers `NotReady` and queues its read).
fn submit_hint(service: &LocalStateService, account: &str, at: u64) -> RequestId {
    let deadline = Instant::now() + WAIT;
    loop {
        match service.submit(bank_hint(account, at)) {
            SubmitReceipt::Accepted { request } => return request,
            SubmitReceipt::Refused(ErrorCode::NotReady { .. }) => {
                assert!(Instant::now() < deadline, "the bank store never hydrated");
                std::thread::sleep(Duration::from_millis(2));
            }
            SubmitReceipt::Refused(code) => panic!("refused: {code}"),
        }
    }
}

fn accepted(receipt: SubmitReceipt) -> RequestId {
    match receipt {
        SubmitReceipt::Accepted { request } => request,
        SubmitReceipt::Refused(code) => panic!("refused: {code}"),
    }
}

fn settled(
    service: &LocalStateService,
    request: RequestId,
    wait: Duration,
) -> Option<WriteSettlement> {
    match service.wait_request(request, wait) {
        RequestState::Settled(settlement) => Some(settlement),
        RequestState::Pending | RequestState::Forgotten => None,
    }
}

fn running_as_root() -> bool {
    #[cfg(unix)]
    {
        // SAFETY: geteuid has no preconditions and cannot fail.
        unsafe { libc::geteuid() == 0 }
    }
    #[cfg(not(unix))]
    false
}

/// Run one ignored child test of this binary with `envs`; its exit status.
fn child(test: &str, envs: &[(&str, &std::path::Path)]) -> std::process::ExitStatus {
    let mut command = std::process::Command::new(crate::test_process::executable());
    command
        .args(["--ignored", "--exact", test, "--nocapture"])
        .stdout(std::process::Stdio::null());
    for (key, value) in envs {
        command.env(key, value);
    }
    command.status().unwrap()
}

/// The original ignored children use the same exact-process runner as the
/// other local-state tests.
fn isolated_child(test: &str, envs: &[(&str, &str)]) {
    crate::test_process::run(&format!("local_state::sql_tests::{test}"), envs);
}

fn env_path(key: &str) -> std::path::PathBuf {
    std::path::PathBuf::from(std::env::var_os(key).unwrap_or_else(|| panic!("{key} unset")))
}

#[test]
fn a_fresh_root_starts_empty_at_a_database_version_and_writes_no_json() {
    let _state_lease = crate::test_process::state();
    let scratch = IsolatedEnv::enter("t53s4-fresh");
    let root = root_of(&scratch);
    let service = open(&scratch);
    let preferences = service.preferences();
    let start = ready(&preferences);
    assert!(start.document().is_empty());
    assert!(start.version() > StoreVersion::INITIAL);
    save(&preferences, "last_focus", json!("alice")).unwrap();
    let after = ready(&preferences);
    assert_eq!(after.document()["last_focus"], json!("alice"));
    assert!(after.version() > start.version());
    assert_eq!(head(&root), after.version().get());
    assert_eq!(ledger_kind(&root).as_deref(), Some("absent"));
    assert!(!root.preferences_file().exists(), "no JSON is written");
    #[cfg(unix)]
    {
        assert_eq!(
            std::fs::metadata(root.state_dir())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        for file in sql::database_set(&root) {
            if let Ok(meta) = std::fs::metadata(&file) {
                assert_eq!(
                    meta.permissions().mode() & 0o777,
                    0o600,
                    "{}",
                    file.display()
                );
            }
        }
    }
}

/// The directories a `sql::open_connection` call flushed, in order, with the
/// names each held then, and the index of each in `events`.
fn flushed(events: &[sql::OpenEvent]) -> Vec<(usize, std::path::PathBuf, Vec<String>)> {
    events
        .iter()
        .enumerate()
        .filter_map(|(at, event)| match event {
            sql::OpenEvent::SyncDir { dir, names } => Some((at, dir.clone(), names.clone())),
            sql::OpenEvent::Pragma(_) => None,
        })
        .collect()
}

/// The directories of [`flushed`], in order.
fn flushed_dirs(synced: &[(usize, std::path::PathBuf, Vec<String>)]) -> Vec<std::path::PathBuf> {
    synced.iter().map(|(_, dir, _)| dir.clone()).collect()
}

/// The name of a `PRAGMA name = value;` statement as the recorder holds it:
/// the whole word after `PRAGMA`, so `fullfsync` is not
/// `checkpoint_fullfsync`.
fn pragma_name(statement: &str) -> &str {
    statement
        .trim_start_matches("PRAGMA")
        .trim_start()
        .split(|c: char| c.is_whitespace() || matches!(c, '=' | '(' | ';'))
        .next()
        .unwrap_or_default()
}

/// A commit that reaches the database file and leaves frames in the WAL: a
/// table, a checkpoint that copies it into the file, and one more row.
fn reach_the_database(conn: &Connection) {
    conn.execute("CREATE TABLE reached (x)", []).unwrap();
    let busy: i64 = conn
        .query_row("PRAGMA wal_checkpoint(PASSIVE)", [], |row| row.get(0))
        .unwrap();
    assert_eq!(busy, 0, "the checkpoint copied every frame");
    conn.execute("INSERT INTO reached VALUES (1)", []).unwrap();
}

/// Puts a directory's mode back to `0o700` when dropped, so a failed
/// assertion does not leave a scratch directory that the cleanup cannot
/// enter.
#[cfg(unix)]
struct RestoreMode(std::path::PathBuf);

#[cfg(unix)]
impl Drop for RestoreMode {
    fn drop(&mut self) {
        let _ = std::fs::set_permissions(&self.0, std::fs::Permissions::from_mode(0o700));
    }
}

#[test]
fn a_pragma_is_named_by_its_whole_word() {
    assert_eq!(pragma_name("PRAGMA fullfsync = ON;"), "fullfsync");
    assert_eq!(
        pragma_name("PRAGMA checkpoint_fullfsync = ON;"),
        "checkpoint_fullfsync"
    );
    assert_eq!(pragma_name("PRAGMA journal_mode = WAL"), "journal_mode");
    assert_eq!(pragma_name("PRAGMA synchronous = FULL;"), "synchronous");
    assert_eq!(pragma_name("PRAGMA page_size = 4096"), "page_size");
}

/// Plan §4.5 (macOS `fullfsync`): the durability pragmas run before the
/// switch to WAL mode, because a new file runs its first transaction inside
/// `PRAGMA journal_mode = WAL` and syncs it as set at that moment. The
/// recorded statements pin the order on every platform, each by its exact
/// name. They cannot show the sync call itself: the macOS syscall trace of a
/// creation does (an `F_FULLFSYNC` for the journal and the database file).
#[test]
fn a_new_database_is_set_up_with_its_durability_pragmas_before_wal_mode() {
    let _state_lease = crate::test_process::state();
    let scratch = IsolatedEnv::enter("s1-open-pragmas");
    let root = root_of(&scratch);
    sql::take_open_events();
    let conn = sql::open_connection(&root).unwrap_or_else(|_| panic!("the database opens"));
    let pragmas: Vec<String> = sql::take_open_events()
        .into_iter()
        .filter_map(|event| match event {
            sql::OpenEvent::Pragma(text) => Some(text),
            sql::OpenEvent::SyncDir { .. } => None,
        })
        .collect();
    let names: Vec<&str> = pragmas.iter().map(|text| pragma_name(text)).collect();
    let wal = names
        .iter()
        .position(|set| *set == "journal_mode")
        .expect("the journal mode is set");
    let mut durability = vec!["synchronous"];
    if cfg!(target_os = "macos") {
        durability.extend(["fullfsync", "checkpoint_fullfsync"]);
    }
    for name in durability {
        let at = names
            .iter()
            .position(|set| *set == name)
            .unwrap_or_else(|| panic!("{name} is set: {pragmas:?}"));
        assert!(at < wal, "{name} is set before journal_mode: {pragmas:?}");
    }
    sql::verify_settings(&conn).unwrap();
}

/// Plan §4.5: a new name needs its directory-publication sync. The empty
/// database file is published by a hard link and SQLite makes `-wal` and
/// `-shm` at the first read transaction, so the open flushes the state
/// directory, the root and the root's parent after those names exist.
#[test]
fn a_new_database_flushes_its_directory_chain_after_its_names_exist() {
    let _state_lease = crate::test_process::state();
    let scratch = IsolatedEnv::enter("s1-open-names");
    let root = root_of(&scratch);
    sql::take_open_events();
    let conn = sql::open_connection(&root).unwrap_or_else(|_| panic!("the database opens"));
    let events = sql::take_open_events();
    let last_pragma = events
        .iter()
        .rposition(|event| matches!(event, sql::OpenEvent::Pragma(_)))
        .expect("the pragmas run");
    let synced = flushed(&events);
    assert_eq!(
        flushed_dirs(&synced),
        vec![
            root.state_dir(),
            root.path().to_path_buf(),
            scratch.home.clone()
        ]
    );
    for (at, dir, _) in &synced {
        assert!(
            *at > last_pragma,
            "{} is flushed after setup",
            dir.display()
        );
    }
    let [database, wal, shm] = sql::database_set(&root);
    for file in [database, wal, shm] {
        let name = file.file_name().unwrap().to_string_lossy().into_owned();
        assert!(
            synced[0].2.contains(&name),
            "{name} exists when the state directory is flushed: {:?}",
            synced[0].2
        );
    }
    assert!(synced[1].2.contains(&"state".to_owned()));
    assert!(synced[2].2.contains(&".274bot".to_owned()));
    drop(conn);
}

/// A database that a commit has reached, with its WAL, flushes nothing when
/// it opens again. A WAL that is gone (another program removed it between two
/// runs) is a new name again, so only the state directory is flushed.
#[test]
fn a_reopen_flushes_nothing_and_a_new_wal_flushes_the_state_directory_only() {
    let _state_lease = crate::test_process::state();
    let scratch = IsolatedEnv::enter("s1-open-reopen");
    let root = root_of(&scratch);
    let first = sql::open_connection(&root).unwrap_or_else(|_| panic!("the database opens"));
    reach_the_database(&first);
    drop(first);
    let [_, wal, shm] = sql::database_set(&root);
    assert!(
        std::fs::metadata(&wal).is_ok_and(|meta| meta.len() > 0),
        "close-time checkpoints are off: the WAL stays, with its frames"
    );
    sql::take_open_events();
    let second = sql::open_connection(&root).unwrap_or_else(|_| panic!("the database reopens"));
    drop(second);
    assert!(flushed(&sql::take_open_events()).is_empty());
    std::fs::remove_file(&wal).unwrap();
    if shm.exists() {
        std::fs::remove_file(&shm).unwrap();
    }
    let third = sql::open_connection(&root).unwrap_or_else(|_| panic!("the database opens"));
    let synced = flushed(&sql::take_open_events());
    assert_eq!(synced.len(), 1, "{synced:?}");
    assert_eq!(synced[0].1, root.state_dir());
    assert!(synced[0].2.contains(&"state.sqlite3-wal".to_owned()));
    drop(third);
}

/// A database with its header and no table is one no commit has reached
/// (plan §4.3: uninitialized). Its names may come from an open that stopped
/// before it flushed them all, so every open flushes the chain again until a
/// commit reaches the database.
#[test]
fn a_database_no_commit_has_reached_is_published_again_at_every_open() {
    let _state_lease = crate::test_process::state();
    let scratch = IsolatedEnv::enter("s1-open-header");
    let root = root_of(&scratch);
    let chain = vec![
        root.state_dir(),
        root.path().to_path_buf(),
        scratch.home.clone(),
    ];
    sql::take_open_events();
    for attempt in 0..2 {
        let conn = sql::open_connection(&root).unwrap_or_else(|_| panic!("the database opens"));
        assert_eq!(
            flushed_dirs(&flushed(&sql::take_open_events())),
            chain,
            "open {attempt} of a database no commit has reached"
        );
        drop(conn);
    }
    let [database, wal, _] = sql::database_set(&root);
    assert!(
        std::fs::metadata(&database).unwrap().len() > 0,
        "the opens left the database with its header"
    );
    assert!(wal.exists());
    let conn = sql::open_connection(&root).unwrap_or_else(|_| panic!("the database opens"));
    reach_the_database(&conn);
    drop(conn);
    sql::take_open_events();
    let conn = sql::open_connection(&root).unwrap_or_else(|_| panic!("the database reopens"));
    assert!(
        flushed(&sql::take_open_events()).is_empty(),
        "a commit has reached the database: nothing is left to flush"
    );
    drop(conn);
}

/// A WAL no commit has reached is a name that may not be durable: an open can
/// stop after SQLite made the file and before it flushed the directory. The
/// next open flushes the state directory again, and no more than that.
#[test]
fn a_wal_no_commit_has_reached_is_published_again_in_the_state_directory_only() {
    let _state_lease = crate::test_process::state();
    let scratch = IsolatedEnv::enter("s1-open-empty-wal");
    let root = root_of(&scratch);
    let first = sql::open_connection(&root).unwrap_or_else(|_| panic!("the database opens"));
    reach_the_database(&first);
    let busy: i64 = first
        .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |row| row.get(0))
        .unwrap();
    assert_eq!(busy, 0, "the checkpoint reset the WAL");
    drop(first);
    let [_, wal, _] = sql::database_set(&root);
    assert_eq!(
        std::fs::metadata(&wal).unwrap().len(),
        0,
        "the WAL is empty"
    );
    sql::take_open_events();
    let second = sql::open_connection(&root).unwrap_or_else(|_| panic!("the database reopens"));
    let synced = flushed(&sql::take_open_events());
    assert_eq!(flushed_dirs(&synced), vec![root.state_dir()]);
    assert!(synced[0].2.contains(&"state.sqlite3-wal".to_owned()));
    second
        .execute("INSERT INTO reached VALUES (2)", [])
        .unwrap();
    drop(second);
    sql::take_open_events();
    let third = sql::open_connection(&root).unwrap_or_else(|_| panic!("the database reopens"));
    assert!(
        flushed(&sql::take_open_events()).is_empty(),
        "a commit has reached the WAL: nothing is left to flush"
    );
    drop(third);
}

/// A zero-byte database file is one no commit has reached (plan §4.3). Its
/// name may come from an open that stopped before it flushed, so the open
/// flushes the whole chain again.
#[test]
fn a_zero_byte_database_file_is_published_again() {
    let _state_lease = crate::test_process::state();
    let scratch = IsolatedEnv::enter("s1-open-empty");
    let root = root_of(&scratch);
    vault::create_private_dir(&root.state_dir()).unwrap();
    vault::create_private_file(&root.database_file(), b"").unwrap();
    sql::take_open_events();
    let conn = sql::open_connection(&root).unwrap_or_else(|_| panic!("the database opens"));
    let synced = flushed(&sql::take_open_events());
    assert_eq!(
        flushed_dirs(&synced),
        vec![
            root.state_dir(),
            root.path().to_path_buf(),
            scratch.home.clone()
        ]
    );
    drop(conn);
}

/// A directory that cannot be flushed refuses the database with the
/// directory named: a new name is never left unflushed without saying so.
#[cfg(unix)]
#[test]
fn a_directory_that_cannot_be_flushed_refuses_the_new_database() {
    use std::os::unix::fs::MetadataExt;

    let _state_lease = crate::test_process::state();
    let scratch = IsolatedEnv::enter("s1-open-flush-refused");
    let root = root_of(&scratch);
    vault::create_private_dir(&root.state_dir()).unwrap();
    if std::fs::metadata(&scratch.home).unwrap().uid() == 0 {
        // The superuser opens any directory: nothing to refuse.
        return;
    }
    let _restore = RestoreMode(scratch.home.clone());
    std::fs::set_permissions(&scratch.home, std::fs::Permissions::from_mode(0o100)).unwrap();
    match sql::open_connection(&root) {
        Err(sql::SqlFailure::Refused { detail, .. }) => {
            assert!(detail.contains("could not be flushed"), "{detail}");
            assert!(
                detail.contains(&scratch.home.display().to_string()),
                "{detail}"
            );
        }
        _ => panic!("a directory that cannot be flushed must refuse the database"),
    }
}

/// A flush that fails during the first open leaves the database with its
/// header and its WAL but no table. The next open must flush the chain again
/// (the round 5 review probe): a name that an earlier open could not make
/// durable is never taken for published.
#[cfg(unix)]
#[test]
fn a_failed_parent_flush_is_repeated_by_the_next_open() {
    use std::os::unix::fs::MetadataExt;

    let _state_lease = crate::test_process::state();
    let scratch = IsolatedEnv::enter("s1-open-parent-retry");
    let root = root_of(&scratch);
    vault::create_private_dir(&root.state_dir()).unwrap();
    if std::fs::metadata(&scratch.home).unwrap().uid() == 0 {
        // The superuser opens any directory: nothing to refuse.
        return;
    }
    let chain = vec![
        root.state_dir(),
        root.path().to_path_buf(),
        scratch.home.clone(),
    ];
    let _restore = RestoreMode(scratch.home.clone());
    std::fs::set_permissions(&scratch.home, std::fs::Permissions::from_mode(0o100)).unwrap();
    sql::take_open_events();
    let first = sql::open_connection(&root);
    std::fs::set_permissions(&scratch.home, std::fs::Permissions::from_mode(0o700)).unwrap();
    match first {
        Err(sql::SqlFailure::Refused { detail, .. }) => {
            assert!(detail.contains("could not be flushed"), "{detail}");
            assert!(
                detail.contains(&scratch.home.display().to_string()),
                "{detail}"
            );
        }
        _ => panic!("the first open must refuse the parent that cannot be flushed"),
    }
    assert_eq!(
        flushed_dirs(&flushed(&sql::take_open_events())),
        chain,
        "the first open tried the whole chain"
    );
    let [database, wal, _] = sql::database_set(&root);
    assert!(
        std::fs::metadata(&database).unwrap().len() > 0,
        "the failed open left the database with its header"
    );
    assert!(wal.exists(), "and its WAL");
    let second = sql::open_connection(&root)
        .unwrap_or_else(|_| panic!("the second open has its permissions back"));
    assert_eq!(
        flushed_dirs(&flushed(&sql::take_open_events())),
        chain,
        "the second open flushes the whole chain again"
    );
    reach_the_database(&second);
    drop(second);
    let third = sql::open_connection(&root).unwrap_or_else(|_| panic!("the database reopens"));
    assert!(
        flushed(&sql::take_open_events()).is_empty(),
        "the chain is published: nothing is left to flush"
    );
    drop(third);
}

/// A database that another connection holds locked answers a wait at open,
/// never a refusal: the open is retried at the delayed intervals (plan
/// §4.4). `PRAGMA synchronous` is the first statement that reads the schema,
/// so it must answer a lock as `journal_mode` does.
#[test]
fn a_database_locked_at_open_answers_a_wait_not_a_refusal() {
    let _state_lease = crate::test_process::state();
    let scratch = IsolatedEnv::enter("s1-open-locked");
    let root = root_of(&scratch);
    vault::create_private_dir(&root.state_dir()).unwrap();
    vault::create_private_file(&root.database_file(), b"").unwrap();
    let holder = inspect(&root);
    holder
        .execute_batch("CREATE TABLE held (x); BEGIN EXCLUSIVE;")
        .unwrap();
    match sql::open_connection(&root) {
        Err(sql::SqlFailure::Wait(sql::WaitCause::Busy)) => {}
        Err(sql::SqlFailure::Refused { detail, .. }) => {
            panic!("a lock must wait, not refuse: {detail}")
        }
        _ => panic!("the open of a locked database must answer a wait"),
    }
    holder.execute_batch("COMMIT").unwrap();
    drop(holder);
    sql::open_connection(&root).unwrap_or_else(|_| panic!("the database opens once it is free"));
}

#[test]
fn first_import_takes_the_whole_object_once_and_keeps_the_source_as_migrated_evidence() {
    let _state_lease = crate::test_process::state();
    let scratch = IsolatedEnv::enter("t53s4-import");
    let root = root_of(&scratch);
    let legacy = json!({
        "last_focus": "bob",
        "nav": {"allow_teleports": true, "hop_labels": false},
        "zz_unknown_future_member": {"kept": [1, 2, 3]},
        "version_marker": 7
    });
    let bytes = serde_json::to_vec_pretty(&legacy).unwrap();
    write_private(&root.preferences_file(), &bytes);
    let service = open(&scratch);
    let first = ready(&service.preferences());
    assert_eq!(
        serde_json::Value::Object((**first.document()).clone()),
        legacy
    );
    assert!(!root.preferences_file().exists());
    let migrated = root.path().join("panel-ui.json.migrated");
    assert_eq!(
        std::fs::read(&migrated).unwrap(),
        bytes,
        "exact source bytes kept"
    );
    assert_eq!(ledger_kind(&root).as_deref(), Some("imported"));
    drop(service);
    // Reopen: no second import, same version.
    let again = open(&scratch);
    let second = ready(&again.preferences());
    assert_eq!(second.version(), first.version());
    assert_eq!(second.document(), first.document());
}

/// The domain head row of the preferences, when the database has one.
fn head_row(root: &StateRoot) -> Option<u64> {
    inspect(root)
        .query_row(
            "SELECT version FROM domain_head WHERE domain = 'preferences'",
            [],
            |row| row.get::<_, Vec<u8>>(0),
        )
        .optional()
        .unwrap()
        .map(|raw| sql::decode_u64(&raw).unwrap())
}

/// FU:15 (S3a, S1 merge): a legacy `panel-ui.json` that names a field twice
/// in one object, at the top level or nested, is refused by the first
/// import as the file reader refuses it: the refusal names the path, the
/// cause (field and object) and the remedy, the source file keeps every
/// byte and its name, and the database gets no preference head and no
/// ledger row. A repaired file imports afterwards.
#[test]
fn a_legacy_file_with_a_duplicate_field_is_refused_by_the_import_and_changes_nothing() {
    let _state_lease = crate::test_process::state();
    let scratch = IsolatedEnv::enter("s1-import-duplicate");
    let root = root_of(&scratch);
    let source = root.preferences_file();
    for (case, bytes, cause) in [
        (
            "top level",
            br#"{"last_focus":"alice","last_focus":"bob"}"#.to_vec(),
            r#"field "last_focus" is named twice in the top level; edit the file to remove one of them"#,
        ),
        (
            "nested",
            br#"{"nav":{"allow_teleports":true,"allow_teleports":false}}"#.to_vec(),
            r#"field "allow_teleports" is named twice in the object "nav"; edit the file to remove one of them"#,
        ),
    ] {
        write_private(&source, &bytes);
        let service = open(&scratch);
        let error = refusal(&service.preferences());
        let text = error.to_string();
        assert!(
            text.contains(&format!("{:?}", source.display().to_string())),
            "{case}: {text}"
        );
        assert!(text.contains(cause), "{case}: {text}");
        assert!(
            text.contains("No source ledger or defaults were saved"),
            "{case}: {text}"
        );
        assert!(
            text.contains("Edit the file as said, or remove it to start empty."),
            "{case}: {text}"
        );
        assert_eq!(
            std::fs::read(&source).unwrap(),
            bytes,
            "{case}: every byte stays"
        );
        assert_eq!(
            kept_copies(&root, "corrupt").len() + kept_copies(&root, "migrated").len(),
            0,
            "{case}: no copy, no evidence"
        );
        assert!(
            !root.path().join("panel-ui.json.migrated").exists(),
            "{case}"
        );
        assert_eq!(head_row(&root), None, "{case}: no preference head");
        assert_eq!(ledger_kind(&root), None, "{case}: no ledger row");
        assert!(
            service
                .preferences()
                .set("map_bake", serde_json::json!("always"))
                .is_err(),
            "{case}: nothing saves over the refused file"
        );
        assert_eq!(head_row(&root), None, "{case}: a refused save adds no head");
        drop(service);
    }
    // The repaired file imports at the next start.
    write_private(&source, br#"{"last_focus":"bob"}"#);
    let service = open(&scratch);
    assert_eq!(
        ready(&service.preferences()).document()["last_focus"],
        json!("bob")
    );
    assert_eq!(ledger_kind(&root).as_deref(), Some("imported"));
}

/// A valid legacy file, with the same field name in different objects (not
/// a duplicate), imports with exactly the values a plain `serde_json` read
/// holds, decided from the fixture before anything reads it.
#[test]
fn a_legacy_file_without_duplicate_fields_imports_with_the_values_it_held() {
    let _state_lease = crate::test_process::state();
    let scratch = IsolatedEnv::enter("s1-import-valid");
    let root = root_of(&scratch);
    let bytes = br#"{
        "last_focus": "alice",
        "nav": {"allow_teleports": true},
        "pvp": {"start_fights": true},
        "layout": [{"name": "mine"}, {"name": "yours"}],
        "future": {"same": 1, "other": {"same": 2}}
    }"#
    .to_vec();
    write_private(&root.preferences_file(), &bytes);
    let expected: serde_json::Map<String, serde_json::Value> =
        serde_json::from_slice(&bytes).unwrap();
    let service = open(&scratch);
    let snapshot = ready(&service.preferences());
    assert_eq!(**snapshot.document(), expected);
    assert_eq!(stored_document(&root), serde_json::Value::Object(expected));
    assert_eq!(ledger_kind(&root).as_deref(), Some("imported"));
    assert!(head_row(&root).is_some());
    assert_eq!(
        std::fs::read(root.path().join("panel-ui.json.migrated")).unwrap(),
        bytes,
        "exact source bytes kept"
    );
}

/// A non-object root and a truncated file keep their malformed handling
/// under the duplicate check, whatever duplicates they also hold: kept in
/// a `.corrupt-` copy, defaults started, a quarantined ledger row.
#[test]
fn a_duplicate_in_a_malformed_legacy_file_is_still_the_malformed_file() {
    let _state_lease = crate::test_process::state();
    let scratch = IsolatedEnv::enter("s1-import-malformed-duplicate");
    for (case, bytes) in [
        ("truncated", br#"{"a":1,"a":"#.to_vec()),
        ("non-object root", br#"[{"a":1,"a":2}]"#.to_vec()),
    ] {
        let root = StateRoot::new(scratch.home.join(case).join(".274bot"));
        write_private(&root.preferences_file(), &bytes);
        let service = LocalStateService::open_process(root.clone());
        assert!(
            ready(&service.preferences()).document().is_empty(),
            "{case}"
        );
        assert_eq!(ledger_kind(&root).as_deref(), Some("quarantined"), "{case}");
        let copies = kept_copies(&root, "corrupt");
        assert_eq!(copies.len(), 1, "{case}");
        assert_eq!(std::fs::read(&copies[0]).unwrap(), bytes, "{case}");
    }
}

#[test]
fn a_newer_sql_edit_wins_over_the_original_json_still_present() {
    let _state_lease = crate::test_process::state();
    let scratch = IsolatedEnv::enter("t53s4-newer-sql");
    let root = root_of(&scratch);
    write_private(&root.preferences_file(), br#"{"last_focus":"old"}"#);
    let service = open(&scratch);
    let preferences = service.preferences();
    ready(&preferences);
    save(&preferences, "last_focus", json!("new")).unwrap();
    drop(preferences);
    drop(service);
    // An older program puts the original file back without an export.
    write_private(&root.preferences_file(), br#"{"last_focus":"old"}"#);
    let again = open(&scratch);
    let snapshot = ready(&again.preferences());
    assert_eq!(snapshot.document()["last_focus"], json!("new"));
    assert!(
        root.preferences_file().exists(),
        "the file is kept as it is"
    );
}

#[test]
fn malformed_legacy_json_is_kept_byte_for_byte_and_defaults_start_once() {
    let _state_lease = crate::test_process::state();
    let scratch = IsolatedEnv::enter("t53s4-malformed");
    let root = root_of(&scratch);
    let damaged = b"{\"last_focus\": \"cut off".to_vec();
    write_private(&root.preferences_file(), &damaged);
    let service = open(&scratch);
    let first = ready(&service.preferences());
    assert!(first.document().is_empty());
    let copies = || {
        std::fs::read_dir(root.path())
            .unwrap()
            .filter_map(|entry| entry.ok())
            .filter(|entry| entry.file_name().to_string_lossy().contains(".corrupt-"))
            .map(|entry| entry.path())
            .collect::<Vec<_>>()
    };
    let kept = copies();
    assert_eq!(kept.len(), 1);
    assert_eq!(std::fs::read(&kept[0]).unwrap(), damaged);
    assert_eq!(ledger_kind(&root).as_deref(), Some("quarantined"));
    drop(service);
    let again = open(&scratch);
    assert_eq!(ready(&again.preferences()).version(), first.version());
    assert_eq!(copies().len(), 1, "the recovery to defaults runs once");
}

#[test]
fn source_read_failures_refuse_without_a_ledger_and_import_after_the_repair() {
    let _state_lease = crate::test_process::state();
    let scratch = IsolatedEnv::enter("t53s4-source-failure");
    let root = root_of(&scratch);
    let source = root.preferences_file();
    // A directory at the source path: a wrong type, never "absent".
    std::fs::create_dir_all(&source).unwrap();
    let service = open(&scratch);
    let error = refusal(&service.preferences());
    let text = error.to_string();
    assert!(text.contains(&source.display().to_string()), "{text}");
    assert!(
        text.contains("No source ledger or defaults were saved"),
        "{text}"
    );
    assert!(
        text.contains("Restore readable private preferences JSON"),
        "{text}"
    );
    assert_eq!(ledger_kind(&root), None, "no absent ledger");
    drop(service);
    std::fs::remove_dir(&source).unwrap();
    if !running_as_root() {
        // Group-writable: unsafe mode, with its chmod remedy.
        write_private(&source, br#"{"last_focus":"carol"}"#);
        #[cfg(unix)]
        std::fs::set_permissions(&source, std::fs::Permissions::from_mode(0o664)).unwrap();
        let service = open(&scratch);
        let text = refusal(&service.preferences()).to_string();
        assert!(text.contains("chmod 600"), "{text}");
        assert!(
            text.contains("No source ledger or defaults were saved"),
            "{text}"
        );
        assert_eq!(ledger_kind(&root), None);
        drop(service);
        #[cfg(unix)]
        std::fs::set_permissions(&source, std::fs::Permissions::from_mode(0o600)).unwrap();
    } else {
        write_private(&source, br#"{"last_focus":"carol"}"#);
    }
    let service = open(&scratch);
    let snapshot = ready(&service.preferences());
    assert_eq!(snapshot.document()["last_focus"], json!("carol"));
    assert_eq!(ledger_kind(&root).as_deref(), Some("imported"));
}

#[test]
fn an_over_limit_source_is_refused_with_both_recovery_commands() {
    let _state_lease = crate::test_process::state();
    let scratch = IsolatedEnv::enter("t53s4-over-limit");
    let root = root_of(&scratch);
    let big = format!("{{\"blob\":\"{}\"}}", "x".repeat(1_100_000));
    write_private(&root.preferences_file(), big.as_bytes());
    let service = open(&scratch);
    let text = refusal(&service.preferences()).to_string();
    for bin in ["panel-play", "tui-play"] {
        assert!(
            text.contains(&format!(
                "Run '{bin} state preferences recover --source \"<private-valid-json>\"' or \
                 '{bin} state preferences recover --defaults'."
            )),
            "{text}"
        );
    }
    assert!(
        text.contains("Stop all 274bot processes for this root."),
        "{text}"
    );
    assert!(root.preferences_file().exists(), "the source is untouched");
    assert_eq!(ledger_kind(&root), None);
}

#[test]
fn two_services_on_a_zero_byte_database_initialize_once_and_neither_sees_a_foreign_file() {
    let _state_lease = crate::test_process::state();
    let scratch = IsolatedEnv::enter("t53s4-two-init");
    let root = root_of(&scratch);
    vault::create_private_dir(&root.state_dir()).unwrap();
    vault::create_private_file(&root.database_file(), b"").unwrap();
    let first = open(&scratch);
    let second = open(&scratch);
    let a = ready(&first.preferences());
    let b = ready(&second.preferences());
    assert_eq!(a.version(), b.version());
    assert_eq!(ledger_kind(&root).as_deref(), Some("absent"));
}

#[test]
fn foreign_and_future_databases_are_refused_and_bank_hints_still_save() {
    let _state_lease = crate::test_process::state();
    for (case, application, user_version, expected) in [
        ("foreign", 0x1234, 1, "not a 274bot state database"),
        (
            "future",
            sql::APPLICATION_ID,
            2,
            "newer than this build reads",
        ),
    ] {
        let scratch = IsolatedEnv::enter(&format!("t53s4-{case}"));
        let root = root_of(&scratch);
        vault::create_private_dir(&root.state_dir()).unwrap();
        vault::create_private_file(&root.database_file(), b"").unwrap();
        let conn = inspect(&root);
        conn.execute_batch(&format!(
            "PRAGMA journal_mode = WAL; CREATE TABLE other (x); PRAGMA application_id = \
             {application}; PRAGMA user_version = {user_version};"
        ))
        .unwrap();
        drop(conn);
        let service = open(&scratch);
        let text = refusal(&service.preferences()).to_string();
        assert!(text.contains(expected), "{case}: {text}");
        assert!(
            text.contains("Use a compatible 274bot version or a separate correctly owned root."),
            "{text}"
        );
        assert!(!root.preferences_file().exists(), "no JSON fallback");
        let hint = submit_hint(&service, "alice", 1_760_000_000);
        assert!(
            matches!(
                settled(&service, hint, WAIT),
                Some(Settlement::Completed(_))
            ),
            "{case}: bank hints keep working"
        );
    }
}

#[test]
fn a_damaged_database_names_recovery_and_recover_defaults_preserves_the_set_first() {
    let _state_lease = crate::test_process::state();
    let scratch = IsolatedEnv::enter("t53s4-damaged");
    let root = root_of(&scratch);
    vault::create_private_dir(&root.state_dir()).unwrap();
    let garbage = vec![0x5a_u8; 8192];
    vault::create_private_file(&root.database_file(), &garbage).unwrap();
    let service = open(&scratch);
    let text = refusal(&service.preferences()).to_string();
    assert!(
        text.contains("state preferences recover --defaults"),
        "{text}"
    );
    // A maintenance command is refused while the service uses the root.
    let busy =
        run_preferences_maintenance(&root, Frontend::Panel, &MaintenanceAction::RecoverDefaults)
            .unwrap_err()
            .to_string();
    assert!(
        busy.contains("another new-version process uses this root"),
        "{busy}"
    );
    assert!(
        busy.contains("panel-play state preferences recover --defaults"),
        "{busy}"
    );
    drop(service);
    let report =
        run_preferences_maintenance(&root, Frontend::Tui, &MaintenanceAction::RecoverDefaults)
            .unwrap();
    assert!(report.contains("preferences history restarted"), "{report}");
    let preserved: Vec<_> = std::fs::read_dir(root.state_dir())
        .unwrap()
        .filter_map(|entry| entry.ok())
        .filter(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with("preserved-")
        })
        .collect();
    assert_eq!(preserved.len(), 1);
    assert_eq!(
        std::fs::read(preserved[0].path().join("state.sqlite3")).unwrap(),
        garbage,
        "the old database is preserved byte for byte"
    );
    assert_eq!(head(&root), 1, "a new history starts at version one");
    assert_eq!(ledger_kind(&root).as_deref(), Some("recovered-defaults"));
    let reopened = open(&scratch);
    assert!(ready(&reopened.preferences()).document().is_empty());
}

#[test]
fn recover_from_a_private_source_starts_a_new_history_at_version_one() {
    let _state_lease = crate::test_process::state();
    let scratch = IsolatedEnv::enter("t53s4-recover-source");
    let root = root_of(&scratch);
    let service = open(&scratch);
    let preferences = service.preferences();
    ready(&preferences);
    for value in 0..3 {
        save(&preferences, "n", json!(value)).unwrap();
    }
    assert!(head(&root) > 1);
    drop(preferences);
    drop(service);
    let source = scratch.home.join("chosen.json");
    write_private(&source, br#"{"last_focus":"dave"}"#);
    let relative = run_preferences_maintenance(
        &root,
        Frontend::Panel,
        &MaintenanceAction::RecoverSource("chosen.json".into()),
    )
    .unwrap_err()
    .to_string();
    assert!(relative.contains("absolute path"), "{relative}");
    run_preferences_maintenance(
        &root,
        Frontend::Panel,
        &MaintenanceAction::RecoverSource(source.clone()),
    )
    .unwrap();
    assert_eq!(head(&root), 1);
    let reopened = open(&scratch);
    let snapshot = ready(&reopened.preferences());
    assert_eq!(snapshot.document()["last_focus"], json!("dave"));
    assert!(snapshot.document().get("n").is_none());
}

#[test]
fn same_root_export_is_ignored_unchanged_and_reimported_once_when_changed() {
    let _state_lease = crate::test_process::state();
    let scratch = IsolatedEnv::enter("t53s4-export");
    let root = root_of(&scratch);
    let service = open(&scratch);
    let preferences = service.preferences();
    ready(&preferences);
    save(&preferences, "last_focus", json!("erin")).unwrap();
    let saved = ready(&preferences).version();
    drop(preferences);
    drop(service);
    run_preferences_maintenance(&root, Frontend::Tui, &MaintenanceAction::ExportSameRoot).unwrap();
    // The file-only reader and writer of 0.2.9.
    let old_read = || -> serde_json::Value {
        serde_json::from_str(&vault::read_private_file(&root.preferences_file(), 1 << 20).unwrap())
            .unwrap()
    };
    assert_eq!(old_read()["last_focus"], json!("erin"));
    // Unchanged export bytes are ignored.
    let unchanged = open(&scratch);
    assert_eq!(ready(&unchanged.preferences()).version(), saved);
    drop(unchanged);
    // 0.2.9 changes the file: imported once as the latest intent.
    let mut changed = old_read();
    changed["last_focus"] = json!("frank");
    vault::write_private_file(
        &root.preferences_file(),
        &serde_json::to_vec_pretty(&changed).unwrap(),
    )
    .unwrap();
    let reimported = open(&scratch);
    let after = ready(&reimported.preferences());
    assert_eq!(after.document()["last_focus"], json!("frank"));
    assert!(after.version() > saved);
    drop(reimported);
    let reopened = open(&scratch);
    assert_eq!(
        ready(&reopened.preferences()).version(),
        after.version(),
        "the changed export is consumed once"
    );
    drop(reopened);
    // A later change without a new export never overwrites SQL: it is a
    // mixed-version (or outside) edit and refuses, file and SQL untouched.
    changed["last_focus"] = json!("grace");
    vault::write_private_file(
        &root.preferences_file(),
        &serde_json::to_vec_pretty(&changed).unwrap(),
    )
    .unwrap();
    let later = open(&scratch);
    let text = refusal(&later.preferences()).to_string();
    assert!(text.contains("no same-root export authorized it"), "{text}");
    assert_eq!(head(&root), after.version().get());
    drop(later);
    std::fs::remove_file(root.preferences_file()).unwrap();
    assert_eq!(
        ready(&open(&scratch).preferences()).document()["last_focus"],
        json!("frank")
    );
}

fn kept_copies(root: &StateRoot, kind: &str) -> Vec<std::path::PathBuf> {
    std::fs::read_dir(root.path())
        .unwrap()
        .filter_map(|entry| entry.ok())
        .filter(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .contains(&format!(".{kind}-"))
        })
        .map(|entry| entry.path())
        .collect()
}

/// A root with saved preferences and no process using it.
fn seeded_root(name: &str) -> (IsolatedEnv, StateRoot, u64) {
    let scratch = IsolatedEnv::enter(name);
    let root = root_of(&scratch);
    let service = open(&scratch);
    let preferences = service.preferences();
    ready(&preferences);
    save(&preferences, "last_focus", json!("erin")).unwrap();
    let version = head(&root);
    drop(preferences);
    drop(service);
    (scratch, root, version)
}

/// Plan §5.1, §5.2: a changed file with no outstanding published export is
/// a mixed-version or outside edit. The refusal names the exact path and
/// both remedies; the file and the saved preferences stay as they are.
#[test]
fn a_changed_file_without_a_published_export_is_a_mixed_version_refusal() {
    let _state_lease = crate::test_process::state();
    let (scratch, root, version) = seeded_root("t53s4-external-edit");
    let source = root.preferences_file();
    let changed = br#"{"last_focus":"outside"}"#;
    write_private(&source, changed);
    let service = open(&scratch);
    let text = refusal(&service.preferences()).to_string();
    assert!(text.contains(&source.display().to_string()), "{text}");
    assert!(text.contains("no same-root export authorized it"), "{text}");
    for bin in ["panel-play", "tui-play"] {
        assert!(
            text.contains(&format!(
                "'{bin} state preferences recover --source \"{}\"'",
                source.display()
            )),
            "{text}"
        );
    }
    assert_eq!(std::fs::read(&source).unwrap(), changed, "the file stays");
    assert_eq!(head(&root), version, "no automatic SQL overwrite");
    drop(service);
    // Moving the file away keeps the saved preferences.
    std::fs::remove_file(&source).unwrap();
    let service = open(&scratch);
    assert_eq!(
        ready(&service.preferences()).document()["last_focus"],
        json!("erin")
    );
    drop(service);
    // Taking its content instead is the explicit recovery, and the file it
    // leaves in place is accounted for: no second refusal.
    write_private(&source, changed);
    run_preferences_maintenance(
        &root,
        Frontend::Panel,
        &MaintenanceAction::RecoverSource(source.clone()),
    )
    .unwrap();
    let service = open(&scratch);
    assert_eq!(
        ready(&service.preferences()).document()["last_focus"],
        json!("outside")
    );
    drop(service);
    write_private(&source, br#"{"last_focus":"stray"}"#);
    run_preferences_maintenance(&root, Frontend::Tui, &MaintenanceAction::RecoverDefaults).unwrap();
    let service = open(&scratch);
    assert!(ready(&service.preferences()).document().is_empty());
    assert!(source.exists(), "recovery never deletes the file");
}

/// Plan §5.1: an authorized export that cannot be read refuses with its
/// exact path, cause and remedy, and the authorization stays unconsumed:
/// after the repair the changed export imports once.
#[test]
fn an_unreadable_authorized_export_refuses_with_its_cause_and_stays_authorized() {
    let _state_lease = crate::test_process::state();
    for cause in ["over-limit", "directory", "unsafe-mode"] {
        if cause == "unsafe-mode" && (!cfg!(unix) || running_as_root()) {
            continue;
        }
        let (scratch, root, version) = seeded_root(&format!("t53s4-export-{cause}"));
        let source = root.preferences_file();
        run_preferences_maintenance(&root, Frontend::Tui, &MaintenanceAction::ExportSameRoot)
            .unwrap();
        assert_eq!(export_state(&root).as_deref(), Some("published"));
        let repaired = br#"{"last_focus":"frank"}"#;
        let (needle, remedy): (&str, &str) = match cause {
            "over-limit" => {
                let big = format!("{{\"blob\":\"{}\"}}", "x".repeat(1_100_000));
                write_private(&source, big.as_bytes());
                (
                    "more than preferences may hold",
                    "Stop all 274bot processes for this root.",
                )
            }
            "directory" => {
                std::fs::remove_file(&source).unwrap();
                std::fs::create_dir_all(&source).unwrap();
                (
                    "it is not a regular file",
                    "or remove it to keep the saved preferences.",
                )
            }
            _ => {
                #[cfg(unix)]
                std::fs::set_permissions(&source, std::fs::Permissions::from_mode(0o664)).unwrap();
                ("chmod 600", "Retry after fixing this cause.")
            }
        };
        let service = open(&scratch);
        let text = refusal(&service.preferences()).to_string();
        assert!(
            text.contains(&source.display().to_string()),
            "{cause}: {text}"
        );
        assert!(text.contains(needle), "{cause}: {text}");
        assert!(text.contains(remedy), "{cause}: {text}");
        assert_eq!(head(&root), version, "{cause}: nothing was saved");
        assert_eq!(
            export_state(&root).as_deref(),
            Some("published"),
            "{cause}: the authorization is not consumed"
        );
        drop(service);
        if cause == "directory" {
            std::fs::remove_dir(&source).unwrap();
        }
        write_private(&source, repaired);
        let service = open(&scratch);
        assert_eq!(
            ready(&service.preferences()).document()["last_focus"],
            json!("frank"),
            "{cause}: the changed export imports after the repair"
        );
        assert_eq!(export_state(&root).as_deref(), Some("consumed"), "{cause}");
    }
}

/// Plan §5.2, Q3: a malformed authorized export (bytes read fine, not a
/// JSON object) gets the verified quarantine and the defaults, once; it
/// never silently restores stale settings.
#[test]
fn a_malformed_authorized_export_is_quarantined_and_defaults_start_once() {
    let _state_lease = crate::test_process::state();
    let (scratch, root, version) = seeded_root("t53s4-export-malformed");
    run_preferences_maintenance(&root, Frontend::Tui, &MaintenanceAction::ExportSameRoot).unwrap();
    let damaged = b"{\"last_focus\": \"cut off".to_vec();
    write_private(&root.preferences_file(), &damaged);
    let service = open(&scratch);
    let first = ready(&service.preferences());
    assert!(first.document().is_empty(), "defaults start");
    assert!(first.version().get() > version);
    let kept = kept_copies(&root, "corrupt");
    assert_eq!(kept.len(), 1);
    assert_eq!(std::fs::read(&kept[0]).unwrap(), damaged);
    assert_eq!(ledger_kind(&root).as_deref(), Some("quarantined"));
    assert_eq!(export_state(&root).as_deref(), Some("consumed"));
    drop(service);
    let again = open(&scratch);
    assert_eq!(ready(&again.preferences()).version(), first.version());
    assert_eq!(kept_copies(&root, "corrupt").len(), 1, "once");
}

/// Plan §5.1: every explicit recovery-source read error refuses with its
/// exact path, cause and remedy, and changes nothing (no preserved set, no
/// new history); the repaired source then recovers.
#[test]
fn recover_from_a_source_that_cannot_be_read_refuses_and_changes_nothing() {
    let _state_lease = crate::test_process::state();
    let (scratch, root, version) = seeded_root("t53s4-recover-unreadable");
    let source = scratch.home.join("chosen.json");
    let preserved = || {
        std::fs::read_dir(root.state_dir())
            .unwrap()
            .filter_map(|entry| entry.ok())
            .filter(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with("preserved-")
            })
            .count()
    };
    for cause in ["over-limit", "directory", "unsafe-mode"] {
        if cause == "unsafe-mode" && (!cfg!(unix) || running_as_root()) {
            continue;
        }
        let needle = match cause {
            "over-limit" => {
                let big = format!("{{\"blob\":\"{}\"}}", "x".repeat(1_100_000));
                write_private(&source, big.as_bytes());
                "more than preferences may hold"
            }
            "directory" => {
                std::fs::create_dir_all(&source).unwrap();
                "it is not a regular file"
            }
            _ => {
                write_private(&source, br#"{"last_focus":"dave"}"#);
                #[cfg(unix)]
                std::fs::set_permissions(&source, std::fs::Permissions::from_mode(0o664)).unwrap();
                "chmod 600"
            }
        };
        let error = run_preferences_maintenance(
            &root,
            Frontend::Panel,
            &MaintenanceAction::RecoverSource(source.clone()),
        )
        .unwrap_err()
        .to_string();
        assert!(
            error.contains(&source.display().to_string()),
            "{cause}: {error}"
        );
        assert!(error.contains(needle), "{cause}: {error}");
        assert!(error.contains("Nothing was changed."), "{cause}: {error}");
        assert_eq!(preserved(), 0, "{cause}: nothing was preserved or removed");
        assert_eq!(head(&root), version, "{cause}: the history is untouched");
        if cause == "directory" {
            std::fs::remove_dir(&source).unwrap();
        } else {
            std::fs::remove_file(&source).unwrap();
        }
    }
    write_private(&source, br#"{"last_focus":"dave"}"#);
    run_preferences_maintenance(
        &root,
        Frontend::Panel,
        &MaintenanceAction::RecoverSource(source),
    )
    .unwrap();
    assert_eq!(head(&root), 1, "the repaired source recovers");
    assert_eq!(preserved(), 1);
}

/// A compact JSON object of 63,000 short members (review R2-2): below the
/// document limit as the file holds it (882,001 bytes), above it as this
/// build stores it, pretty-printed (1,134,002 bytes).
fn compact_expansion() -> Vec<u8> {
    let mut text = String::from("{");
    for member in 0..63_000 {
        if member > 0 {
            text.push(',');
        }
        text.push_str(&format!("\"k{member:05}\":true"));
    }
    text.push('}');
    let bytes = text.into_bytes();
    assert!((bytes.len() as u64) < sql::DOCUMENT_MAX);
    let document: serde_json::Map<String, serde_json::Value> =
        serde_json::from_slice(&bytes).unwrap();
    assert!(files::preferences_bytes(&document).unwrap().len() as u64 > sql::DOCUMENT_MAX);
    bytes
}

/// What every refusal of a source that cannot be stored names (plan §5.1):
/// the source path and, for each binary, both full recovery commands.
fn assert_names_path_and_recovery_commands(text: &str, source: &std::path::Path, case: &str) {
    assert!(
        text.contains(&source.display().to_string()),
        "{case}: {text}"
    );
    assert!(
        text.contains("more than preferences may hold"),
        "{case}: {text}"
    );
    for bin in ["panel-play", "tui-play"] {
        assert!(
            text.contains(&format!(
                "Run '{bin} state preferences recover --source \"<private-valid-json>\"' or \
                 '{bin} state preferences recover --defaults'."
            )),
            "{case}: {text}"
        );
    }
}

/// Plan §5.1: a source below the size bound whose canonical stored form is
/// above it is unrepresentable on first import: refused untouched with the
/// source path and the full recovery commands, no ledger, no defaults.
#[test]
fn a_first_import_whose_canonical_form_is_over_the_limit_is_refused_untouched() {
    let _state_lease = crate::test_process::state();
    let scratch = IsolatedEnv::enter("t53s4-canonical-first-import");
    let root = root_of(&scratch);
    let source = root.preferences_file();
    let compact = compact_expansion();
    write_private(&source, &compact);
    let service = open(&scratch);
    let text = refusal(&service.preferences()).to_string();
    assert_names_path_and_recovery_commands(&text, &source, "first import");
    assert!(
        text.contains("Stop all 274bot processes for this root."),
        "{text}"
    );
    assert_eq!(std::fs::read(&source).unwrap(), compact, "untouched");
    assert_eq!(ledger_kind(&root), None, "no ledger, no defaults");
}

/// Plan §5.2: the same source as a changed authorized export is refused with
/// its path and the full recovery commands, and the export stays authorized
/// (not consumed) until the source is repaired.
#[test]
fn a_reimport_whose_canonical_form_is_over_the_limit_is_refused_and_stays_authorized() {
    let _state_lease = crate::test_process::state();
    let (scratch, root, version) = seeded_root("t53s4-canonical-reimport");
    run_preferences_maintenance(&root, Frontend::Panel, &MaintenanceAction::ExportSameRoot)
        .unwrap();
    let source = root.preferences_file();
    let compact = compact_expansion();
    write_private(&source, &compact);
    let service = open(&scratch);
    let text = refusal(&service.preferences()).to_string();
    assert_names_path_and_recovery_commands(&text, &source, "reimport");
    assert_eq!(head(&root), version, "nothing was saved");
    assert_eq!(export_state(&root).as_deref(), Some("published"));
    assert_eq!(std::fs::read(&source).unwrap(), compact, "untouched");
    drop(service);
    write_private(&source, br#"{"last_focus":"frank"}"#);
    let service = open(&scratch);
    assert_eq!(
        ready(&service.preferences()).document()["last_focus"],
        json!("frank"),
        "the repaired export imports"
    );
    assert_eq!(export_state(&root).as_deref(), Some("consumed"));
}

/// Plan §5.1, §5.3: recovery validates its source completely before it
/// preserves or replaces any database file. A source whose canonical form is
/// over the limit leaves the head, the document and the ledger as they were,
/// creates no preserved set, and the next start shows the saved preferences.
#[test]
fn recover_from_a_source_whose_canonical_form_is_over_the_limit_changes_nothing() {
    let _state_lease = crate::test_process::state();
    let (scratch, root, version) = seeded_root("t53s4-canonical-recover");
    let kind = ledger_kind(&root);
    let source = scratch.home.join("compact.json");
    write_private(&source, &compact_expansion());
    let text = run_preferences_maintenance(
        &root,
        Frontend::Headless,
        &MaintenanceAction::RecoverSource(source.clone()),
    )
    .unwrap_err()
    .to_string();
    assert_eq!(preserved_sets(&root), 0, "no preserved set was created");
    assert_eq!(head(&root), version, "the head is untouched");
    assert_eq!(stored_document(&root)["last_focus"], json!("erin"));
    assert_eq!(ledger_kind(&root), kind, "the ledger is untouched");
    assert_names_path_and_recovery_commands(&text, &source, "recover");
    assert!(text.contains("Nothing was changed."), "{text}");
    let service = open(&scratch);
    let snapshot = ready(&service.preferences());
    assert_eq!(snapshot.version().get(), version);
    assert_eq!(snapshot.document()["last_focus"], json!("erin"));
}

/// A child of the interrupted-export tests: the export command with an
/// abort at `T53_CRASH_AT`.
#[test]
#[ignore = "child process of the interrupted export tests"]
fn export_crash_child() {
    let root = StateRoot::new(env_path("T53_ROOT"));
    let at = std::env::var("T53_CRASH_AT").unwrap();
    let point: Arc<dyn Fn(&'static str) + Send + Sync> = Arc::new(move |name| {
        if name == at {
            std::process::abort();
        }
    });
    let _ = maintenance::run_preferences_maintenance_with_seams(
        &root,
        Frontend::Panel,
        &MaintenanceAction::ExportSameRoot,
        sql::Seams {
            point: Some(point),
            ..sql::Seams::default()
        },
    );
}

fn crash_export_at(root: &StateRoot, point: &str) {
    let status = std::process::Command::new(crate::test_process::executable())
        .args([
            "--ignored",
            "--exact",
            "local_state::sql_tests::export_crash_child",
        ])
        .env("T53_ROOT", root.path())
        .env("T53_CRASH_AT", point)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .unwrap();
    assert!(!status.success(), "the child crashed at {point}");
}

/// Plan §5.2, §5.4: an export interrupted before its publication, after
/// its rename, or before its published marker is completed by the next
/// start from its export ID and verified digest: never mistaken for a
/// 0.2.9 edit, never imported, and an unrelated file at the name is kept.
#[test]
fn an_interrupted_export_completes_from_its_staged_bytes_at_the_next_start() {
    crate::test_process::isolate_state_test!(
        an_interrupted_export_completes_from_its_staged_bytes_at_the_next_start
    );
    for point in [
        "export-before-publish",
        "export-after-rename",
        "export-before-marker",
    ] {
        let (scratch, root, version) = seeded_root(&format!("t53s4-export-crash-{point}"));
        let unrelated = br#"{"unrelated":true}"#;
        if point == "export-before-publish" {
            write_private(&root.preferences_file(), unrelated);
        }
        crash_export_at(&root, point);
        assert_eq!(export_state(&root).as_deref(), Some("prepared"), "{point}");
        let staged = root.path().join("panel-ui.json.export-1");
        assert_eq!(staged.exists(), point == "export-before-publish", "{point}");
        let service = open(&scratch);
        let snapshot = ready(&service.preferences());
        assert_eq!(
            snapshot.version().get(),
            version,
            "{point}: an interrupted export is not an import"
        );
        assert_eq!(snapshot.document()["last_focus"], json!("erin"), "{point}");
        assert_eq!(export_state(&root).as_deref(), Some("published"), "{point}");
        assert!(!staged.exists(), "{point}: the staged file was published");
        let published: serde_json::Value =
            serde_json::from_slice(&std::fs::read(root.preferences_file()).unwrap()).unwrap();
        assert_eq!(published["last_focus"], json!("erin"), "{point}");
        if point == "export-before-publish" {
            let kept = kept_copies(&root, "before-export");
            assert_eq!(kept.len(), 1);
            assert_eq!(std::fs::read(&kept[0]).unwrap(), unrelated);
        }
        drop(service);
        // The completed export is an ordinary published one: unchanged
        // bytes are ignored, and a 0.2.9 edit imports once.
        assert_eq!(
            ready(&open(&scratch).preferences()).version().get(),
            version,
            "{point}"
        );
        vault::write_private_file(&root.preferences_file(), br#"{"last_focus":"frank"}"#).unwrap();
        let reimported = open(&scratch);
        assert_eq!(
            ready(&reimported.preferences()).document()["last_focus"],
            json!("frank"),
            "{point}"
        );
        assert_eq!(export_state(&root).as_deref(), Some("consumed"), "{point}");
    }
}

/// Plan §5.2: an interrupted export whose staged bytes are gone or no
/// longer match the recorded digest cannot be completed: the start refuses
/// with the full retry command, and the old file is not taken for an edit.
#[test]
fn an_interrupted_export_that_cannot_be_completed_refuses_with_the_full_retry_command() {
    crate::test_process::isolate_state_test!(
        an_interrupted_export_that_cannot_be_completed_refuses_with_the_full_retry_command
    );
    for damage in ["missing", "changed"] {
        let (scratch, root, version) = seeded_root(&format!("t53s4-export-lost-{damage}"));
        crash_export_at(&root, "export-before-publish");
        let staged = root.path().join("panel-ui.json.export-1");
        if damage == "missing" {
            std::fs::remove_file(&staged).unwrap();
        } else {
            std::fs::write(&staged, br#"{"tampered":1}"#).unwrap();
        }
        let service = open(&scratch);
        let text = refusal(&service.preferences()).to_string();
        assert!(
            text.contains(&root.preferences_file().display().to_string()),
            "{damage}: {text}"
        );
        assert!(text.contains("same-root export 1"), "{damage}: {text}");
        assert!(text.contains("staged file"), "{damage}: {text}");
        assert!(
            text.contains(
                "Run 'panel-play state preferences export --same-root' or 'tui-play state \
                 preferences export --same-root' again."
            ),
            "{damage}: {text}"
        );
        assert_eq!(export_state(&root).as_deref(), Some("prepared"), "{damage}");
        assert!(!root.preferences_file().exists(), "{damage}");
        drop(service);
        // The named retry completes it; the next start is ordinary.
        run_preferences_maintenance(&root, Frontend::Panel, &MaintenanceAction::ExportSameRoot)
            .unwrap();
        assert_eq!(
            export_state(&root).as_deref(),
            Some("published"),
            "{damage}"
        );
        assert_eq!(
            ready(&open(&scratch).preferences()).version().get(),
            version,
            "{damage}"
        );
    }
}

/// Plan §5.2, §4.3: an export interrupted after its prepared row is
/// resolved before Ready in every ledger state. Here the import that
/// precedes it crashed before its evidence step, so the ledger is
/// `pending` (review R2-1): the start publishes the export (keeping a copy
/// of the file it replaces) or refuses with the retry command, never Ready
/// beside a prepared export. Each case runs in an ignored child of its own:
/// its evidence write meets its own guard, and no unrelated `fork` of the
/// suite is alive in its process (review R3-1).
#[test]
fn a_prepared_export_is_published_before_ready_beside_pending_import_evidence() {
    prepared_export_isolated("published");
}

#[test]
fn an_unfinishable_prepared_export_refuses_before_ready_beside_pending_import_evidence() {
    prepared_export_isolated("staged-file-missing");
}

fn prepared_export_isolated(case: &str) {
    isolated_child(
        "prepared_export_beside_pending_import_evidence_child",
        &[("T53_CASE", case)],
    );
}

/// Child of the two tests above; creates its root after `exec`.
#[test]
#[ignore = "child process of the prepared export tests"]
fn prepared_export_beside_pending_import_evidence_child() {
    let case = std::env::var("T53_CASE").unwrap();
    prepared_export_beside_pending_import_evidence(&case);
}

fn prepared_export_beside_pending_import_evidence(case: &str) {
    let scratch = IsolatedEnv::enter(&format!("t53s4-pending-export-{case}"));
    let root = root_of(&scratch);
    let original = br#"{"last_focus":"pending-import"}"#;
    write_private(&root.preferences_file(), original);
    crash_at(&scratch, "after-import-commit");
    assert_eq!(ledger_evidence(&root).as_deref(), Some("pending"), "{case}");
    let version = head(&root);
    crash_export_at(&root, "export-before-publish");
    assert_eq!(export_state(&root).as_deref(), Some("prepared"), "{case}");
    let staged = root.path().join("panel-ui.json.export-1");
    assert!(staged.exists(), "{case}");
    if case == "staged-file-missing" {
        std::fs::remove_file(&staged).unwrap();
        let service = open(&scratch);
        let text = refusal(&service.preferences()).to_string();
        assert!(text.contains("same-root export 1"), "{case}: {text}");
        assert!(
            text.contains(
                "Run 'panel-play state preferences export --same-root' or 'tui-play state \
                     preferences export --same-root' again."
            ),
            "{case}: {text}"
        );
        assert_eq!(export_state(&root).as_deref(), Some("prepared"), "{case}");
        assert_eq!(ledger_evidence(&root).as_deref(), Some("pending"), "{case}");
        drop(service);
        run_preferences_maintenance(&root, Frontend::Panel, &MaintenanceAction::ExportSameRoot)
            .unwrap();
    }
    let service = open(&scratch);
    let snapshot = ready(&service.preferences());
    assert_eq!(
        export_state(&root).as_deref(),
        Some("published"),
        "{case}: published before Ready"
    );
    assert_eq!(snapshot.version().get(), version, "{case}: no import");
    assert_eq!(snapshot.document()["last_focus"], json!("pending-import"));
    assert_eq!(
        ledger_evidence(&root).as_deref(),
        Some("completed"),
        "{case}: the evidence step finished after the export"
    );
    let published: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.preferences_file()).unwrap()).unwrap();
    assert_eq!(published["last_focus"], json!("pending-import"), "{case}");
    assert!(!staged.exists(), "{case}");
    let kept = kept_copies(&root, "before-export");
    assert_eq!(kept.len(), 1, "{case}");
    assert_eq!(std::fs::read(&kept[0]).unwrap(), original, "{case}");
    drop(service);
    assert_eq!(
        ready(&open(&scratch).preferences()).version().get(),
        version,
        "{case}: the next start is ordinary"
    );
}

/// Plan §4.4: startup is a mutating transaction too. With a pinned reader
/// and the WAL one frame past what a reservation fits (F + P > W), and with
/// a pre-existing WAL past both maxima, repeated opens draw no epoch and
/// append no frame; they show the WAL wait, and open normally after the
/// reader releases and the idle TRUNCATE resets the log.
#[test]
fn startup_writes_wait_for_the_idle_reset_while_the_wal_is_past_its_reservation() {
    let _state_lease = crate::test_process::state();
    for frames in [
        sql::WAL_MAX_FRAMES - sql::MAX_PAGES + 1,
        sql::WAL_MAX_FRAMES + 100,
    ] {
        let scratch = IsolatedEnv::enter(&format!("t53s4-wal-startup-{frames}"));
        let root = root_of(&scratch);
        ready(&open(&scratch).preferences());
        let epoch = epoch_of(&root);
        let probe = inspect(&root);
        let reader = pin_reader(&root);
        fill_wal_to(&root, frames);
        let bytes = wal_bytes(&root);
        if frames > sql::WAL_MAX_FRAMES {
            assert!(
                bytes > sql::WAL_MAX_BYTES,
                "a pre-existing WAL past the byte maximum"
            );
        }
        for attempt in 0..3 {
            let service = open(&scratch);
            let preferences = service.preferences();
            let deadline = Instant::now() + Duration::from_secs(10);
            while preferences.wait_cause().is_none() {
                assert!(
                    Instant::now() < deadline,
                    "{frames} frames, open {attempt}: no WAL wait shown ({:?})",
                    preferences.snapshot().map(|snapshot| snapshot.version())
                );
                std::thread::sleep(Duration::from_millis(5));
            }
            assert!(preferences
                .wait_cause()
                .unwrap()
                .contains("waiting for a SQLite reader to release the WAL"));
            assert!(matches!(preferences.snapshot(), Err(error) if error.is_not_ready()));
            std::thread::sleep(Duration::from_millis(150));
            assert_eq!(epoch_of(&root), epoch, "{frames}: no epoch was drawn");
            assert_eq!(
                wal_frames(&probe),
                frames as i64,
                "{frames}: no frame appended"
            );
            assert_eq!(wal_bytes(&root), bytes, "{frames}: the WAL did not grow");
        }
        reader.execute_batch("COMMIT").unwrap();
        drop(reader);
        let service = open(&scratch);
        ready(&service.preferences());
        assert_eq!(
            epoch_of(&root),
            epoch + 1,
            "{frames}: one epoch after the reset"
        );
        assert!(
            wal_frames(&probe) < 100,
            "{frames}: the idle TRUNCATE reset the log"
        );
        assert!(wal_bytes(&root) < sql::WAL_MAX_BYTES / 100, "{frames}");
    }
}

/// Plan §4.4, §5.1: the import evidence write takes the guard again and is
/// admitted against the WAL bound. While another process holds the guard,
/// or the WAL is past its reservation, the ledger stays `pending` (the
/// source is already `.migrated`), the open still succeeds, and the next
/// start records it. The body runs in an ignored child of its own so its
/// explicit holder is the only competing holder and no unrelated `fork` of
/// the suite copies either lock (review R3-1).
#[test]
fn import_evidence_waits_for_the_guard_and_for_the_wal_reservation() {
    isolated_child("import_evidence_guard_and_wal_child", &[]);
}

/// Child of the test above; creates both isolated roots after `exec`.
#[test]
#[ignore = "child process of the import evidence test"]
fn import_evidence_guard_and_wal_child() {
    for case in ["guard", "wal"] {
        let scratch = IsolatedEnv::enter(&format!("t53s4-evidence-{case}"));
        let root = root_of(&scratch);
        write_private(&root.preferences_file(), br#"{"last_focus":"judy"}"#);
        let held: Arc<std::sync::Mutex<Option<files::WriteLock>>> = Arc::default();
        let reader: Arc<std::sync::Mutex<Option<Connection>>> = Arc::default();
        let (hold, pin) = (Arc::clone(&held), Arc::clone(&reader));
        let at = root.clone();
        let guard_case = case == "guard";
        let point: Arc<dyn Fn(&'static str) + Send + Sync> = Arc::new(move |name| {
            if name != "before-evidence" {
                return;
            }
            if guard_case {
                let lock = files::try_lock_for_write(&at.preferences_file())
                    .unwrap()
                    .expect("the guard is free between the import and its evidence");
                *hold.lock().unwrap() = Some(lock);
            } else {
                *pin.lock().unwrap() = Some(pin_reader(&at));
                fill_wal_to(&at, sql::WAL_MAX_FRAMES - sql::MAX_PAGES + 1);
            }
        });
        let service = LocalStateService::open_process_with_seams_for_test(
            root.clone(),
            sql::Seams {
                point: Some(point),
                ..sql::Seams::default()
            },
            None,
        );
        let snapshot = ready(&service.preferences());
        assert_eq!(snapshot.document()["last_focus"], json!("judy"), "{case}");
        assert!(
            root.path().join("panel-ui.json.migrated").exists(),
            "{case}"
        );
        assert_eq!(
            evidence_state(&root).as_deref(),
            Some("pending"),
            "{case}: the evidence write waited"
        );
        if !guard_case {
            assert_eq!(
                wal_frames(&inspect(&root)) as u64,
                sql::WAL_MAX_FRAMES - sql::MAX_PAGES + 1,
                "the evidence write did not grow a full WAL"
            );
        }
        drop(service);
        drop(held.lock().unwrap().take());
        drop(reader.lock().unwrap().take());
        let notices: Arc<std::sync::Mutex<Vec<String>>> = Arc::default();
        let sink = Arc::clone(&notices);
        let service = LocalStateService::open_process_noticed_for_test(
            root.clone(),
            Arc::new(move |level, line| {
                sink.lock().unwrap().push(format!("{level:?}: {line}"));
            }),
        );
        ready(&service.preferences());
        assert_eq!(
            evidence_state(&root).as_deref(),
            Some("completed"),
            "{case}: the next start records it; its notices: {:?}",
            notices.lock().unwrap()
        );
    }
}

/// Plan §4.4, §5.2: the maintenance export's ledger rows are admitted
/// against the WAL bound too. A reader the idle TRUNCATE cannot get past
/// refuses the export with the cause and the full retry command, writes
/// nothing and stages nothing; after the reader is gone the same command
/// resets the log and succeeds.
#[test]
fn the_maintenance_export_never_grows_a_full_wal() {
    let _state_lease = crate::test_process::state();
    let (_scratch, root, _) = seeded_root("t53s4-export-wal");
    let probe = inspect(&root);
    let reader = pin_reader(&root);
    let frames = sql::WAL_MAX_FRAMES - sql::MAX_PAGES + 1;
    fill_wal_to(&root, frames);
    let bytes = wal_bytes(&root);
    let error =
        run_preferences_maintenance(&root, Frontend::Panel, &MaintenanceAction::ExportSameRoot)
            .unwrap_err()
            .to_string();
    assert!(error.contains("keeps reading the database"), "{error}");
    assert!(
        error.contains("Run 'panel-play state preferences export --same-root' again."),
        "{error}"
    );
    assert_eq!(wal_frames(&probe), frames as i64, "no frame was appended");
    assert!(wal_bytes(&root) <= bytes.max(sql::WAL_MAX_BYTES));
    assert_eq!(export_state(&root), None, "no ledger row was written");
    assert!(!root.preferences_file().exists());
    assert!(!root.path().join("panel-ui.json.export-1").exists());
    reader.execute_batch("COMMIT").unwrap();
    drop(reader);
    run_preferences_maintenance(&root, Frontend::Panel, &MaintenanceAction::ExportSameRoot)
        .unwrap();
    assert_eq!(export_state(&root).as_deref(), Some("published"));
    assert!(wal_frames(&probe) < 100, "the command reset the log first");
}

/// Plan §4.5 (R3-L1): the page size reads back on every connection, the
/// initial, the maintenance and the reconnect one (all open through the
/// same routine; the reconnect path is also read back in the COMMIT
/// outcome test). A database made with another page size refuses.
#[test]
fn every_connection_requires_the_expected_page_size() {
    let _state_lease = crate::test_process::state();
    let scratch = IsolatedEnv::enter("t53s4-page-size");
    let root = root_of(&scratch);
    vault::create_private_dir(&root.state_dir()).unwrap();
    vault::create_private_file(&root.database_file(), b"").unwrap();
    let setup = inspect(&root);
    setup
        .execute_batch(
            "PRAGMA page_size = 8192; PRAGMA journal_mode = WAL; CREATE TABLE other (x);",
        )
        .unwrap();
    drop(setup);
    match sql::open_connection(&root) {
        Err(sql::SqlFailure::Refused { detail, .. }) => {
            assert!(
                detail.contains("PRAGMA page_size is 8192, expected 4096"),
                "{detail}"
            );
            assert!(detail.contains(&root.database_file().display().to_string()));
        }
        _ => panic!("a connection with another page size must be refused"),
    }
    let service = open(&scratch);
    let text = refusal(&service.preferences()).to_string();
    assert!(text.contains("PRAGMA page_size is 8192"), "{text}");
    assert!(
        text.contains("Use a compatible 274bot version or a separate correctly owned root."),
        "{text}"
    );
    drop(service);
    let error =
        run_preferences_maintenance(&root, Frontend::Panel, &MaintenanceAction::ExportSameRoot)
            .unwrap_err()
            .to_string();
    assert!(error.contains("PRAGMA page_size is 8192"), "{error}");
}

#[test]
fn maintenance_command_lines_parse_exactly_and_refuse_combinations() {
    let args = |line: &str| line.split(' ').map(String::from).collect::<Vec<_>>();
    assert_eq!(
        parse_maintenance(
            &args("state preferences export --same-root"),
            Frontend::Panel
        ),
        Some(Ok(MaintenanceAction::ExportSameRoot))
    );
    assert_eq!(
        parse_maintenance(&args("state preferences recover --defaults"), Frontend::Tui),
        Some(Ok(MaintenanceAction::RecoverDefaults))
    );
    assert_eq!(
        parse_maintenance(
            &args("state preferences recover --source /x/private-valid.json"),
            Frontend::Tui
        ),
        Some(Ok(MaintenanceAction::RecoverSource(
            "/x/private-valid.json".into()
        )))
    );
    for bad in [
        "state preferences export --same-root recover --defaults",
        "state preferences recover --defaults --source /x.json",
        "state preferences export",
        "state preferences recover",
        "state",
    ] {
        let parsed = parse_maintenance(&args(bad), Frontend::Panel);
        assert!(matches!(parsed, Some(Err(_))), "{bad}: {parsed:?}");
    }
    assert_eq!(
        parse_maintenance(&args("--profile local-289"), Frontend::Panel),
        None
    );
    let help = maintenance_help(Frontend::Tui);
    assert!(help.contains("tui-play state preferences export --same-root"));
    assert!(help.contains("preserves the whole database set"));
    assert!(help.contains("new preference history"));
}

/// Plan §4.5, §5.4: FULL, READONLY, BUSY and IOERR at COMMIT. Each fault is
/// injected while the worker's own transaction is still active (a COMMIT
/// that errored without ending it), so the rollback has to be the
/// worker's. Per class: the worker connection's autocommit state, the
/// durable head, the cache, the audit count and the one terminal result.
#[test]
fn commit_outcomes_follow_their_own_rules_and_settle_once() {
    let _state_lease = crate::test_process::state();
    let scratch = IsolatedEnv::enter("t53s4-commit-faults");
    let root = root_of(&scratch);
    let service = open(&scratch);
    let preferences = service.preferences();
    ready(&preferences);
    let audits = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&audits);
    service.set_audit_hook(Arc::new(move |_| {
        counted.fetch_add(1, Ordering::SeqCst);
    }));
    let terminals = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&terminals);
    service.set_terminal_hook(Arc::new(move |_| {
        counted.fetch_add(1, Ordering::SeqCst);
    }));
    type States = std::sync::Mutex<Vec<Option<(bool, bool)>>>;
    let worker: Arc<States> = Arc::default();
    // The writer publishes a save's settlement before it logs the terminal
    // line, so a caller released by the settlement can read the counters
    // early. The job hook runs after the write and its terminal hook; the
    // attempt that settles the save leaves no preference save queued or in
    // flight (a requeued attempt leaves its save queued), so that attempt's
    // hook is the fence.
    let (job_done, job_done_rx) = mpsc::channel::<()>();
    let job_done = std::sync::Mutex::new(job_done);
    let writer = Arc::downgrade(&service);
    service.set_job_hook_for_test(Arc::new(move |class, _| {
        if class != "pref-save" {
            return;
        }
        let Some(service) = writer.upgrade() else {
            return;
        };
        if service.preference_occupancy_for_test() == (0, false) {
            let _ = job_done.lock().unwrap().send(());
        }
    }));
    let install = |fault: sql::CommitFault| {
        let once = Arc::new(std::sync::Mutex::new(Some(fault)));
        let states = Arc::clone(&worker);
        service.set_sql_seams_for_test(sql::Seams {
            commit_fault: Some(Arc::new(move || once.lock().unwrap().take())),
            after_commit: Some(Arc::new(move |state| states.lock().unwrap().push(state))),
            ..sql::Seams::default()
        });
    };
    let stored = || -> serde_json::Value {
        let body: Vec<u8> = inspect(&root)
            .query_row(
                "SELECT body FROM preference_document WHERE id = 1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        serde_json::from_slice(&body).unwrap()
    };
    // (fault, value written, whether the save ends durable)
    let classes = [
        (sql::CommitFault::Full, 1, false),
        (sql::CommitFault::ReadOnly, 2, false),
        (sql::CommitFault::IoErrWithoutCommit, 3, false),
        (sql::CommitFault::Busy, 4, true),
        (sql::CommitFault::IoErrAfterCommit, 5, true),
    ];
    for (fault, value, durable) in classes {
        let head_before = head(&root);
        let n_before = stored().get("n").cloned();
        let audits_before = audits.load(Ordering::SeqCst);
        let terminals_before = terminals.load(Ordering::SeqCst);
        worker.lock().unwrap().clear();
        install(fault);
        let Update::Queued(mut update) = preferences.begin_set("n", json!(value)).unwrap() else {
            panic!("queued");
        };
        let request = update.request_id();
        let result = update.wait(WAIT).expect("settles");
        job_done_rx
            .recv_timeout(WAIT)
            .unwrap_or_else(|_| panic!("{fault:?}: the writer job never completed"));
        let states = worker.lock().unwrap().clone();
        assert!(
            states.iter().all(|state| *state == Some((true, true))),
            "{fault:?}: the worker's own connection is back in autocommit with every setting \
             read back ({states:?})"
        );
        assert!(!states.is_empty(), "{fault:?}: the probe ran");
        let snapshot = ready(&preferences);
        if durable {
            result.unwrap_or_else(|error| panic!("{fault:?}: {error}"));
            assert!(
                matches!(
                    service.request_state(request),
                    RequestState::Settled(Settlement::Completed(_))
                ),
                "{fault:?}"
            );
            assert_eq!(
                head(&root),
                head_before + 1,
                "{fault:?}: one durable commit"
            );
            assert_eq!(stored()["n"], json!(value), "{fault:?}");
            assert_eq!(
                snapshot.document()["n"],
                json!(value),
                "{fault:?}: the cache"
            );
            assert_eq!(
                audits.load(Ordering::SeqCst),
                audits_before + 1,
                "{fault:?}: one audit"
            );
            assert_eq!(
                terminals.load(Ordering::SeqCst),
                terminals_before,
                "{fault:?}: a committed save's terminal line is its audit"
            );
        } else {
            let failure = result.unwrap_err();
            let error = failure.to_string();
            assert!(
                matches!(
                    service.request_state(request),
                    RequestState::Settled(Settlement::Failed(_))
                ),
                "{fault:?}"
            );
            assert_eq!(head(&root), head_before, "{fault:?}: nothing durable");
            assert_eq!(stored().get("n").cloned(), n_before, "{fault:?}");
            assert_eq!(
                snapshot.document().get("n").cloned(),
                n_before,
                "{fault:?}: the cache keeps the durable value"
            );
            assert_eq!(
                audits.load(Ordering::SeqCst),
                audits_before,
                "{fault:?}: no audit"
            );
            assert_eq!(
                terminals.load(Ordering::SeqCst),
                terminals_before + 1,
                "{fault:?}: exactly one terminal line"
            );
            match fault {
                sql::CommitFault::IoErrWithoutCommit => {
                    assert!(error.contains("outcome is unknown"), "{error}");
                    assert!(failure.outcome_unknown(), "{fault:?}: flagged unknown");
                }
                _ => {
                    assert!(error.contains("refused the commit"), "{error}");
                    assert!(
                        !failure.outcome_unknown(),
                        "{fault:?}: a refused commit is known"
                    );
                }
            }
        }
        assert_eq!(
            snapshot.version().get(),
            head(&root),
            "{fault:?}: the cache is at the durable head"
        );
    }
}

/// What one save writes together: the stored document, the domain head, the
/// version sequence and the recent-commit ledger, read through the test's
/// own connection.
#[derive(Debug, PartialEq, Eq)]
struct SaveRows {
    document: Vec<u8>,
    head: u64,
    sequence: u64,
    /// (epoch, request, new version, digest) of every ledger row, oldest first.
    commits: Vec<(u64, u64, u64, Vec<u8>)>,
}

fn save_rows(root: &StateRoot) -> SaveRows {
    let conn = inspect(root);
    let number = |sql: &str| -> u64 {
        let raw: Vec<u8> = conn.query_row(sql, [], |row| row.get(0)).unwrap();
        sql::decode_u64(&raw).unwrap()
    };
    let mut ledger = conn
        .prepare("SELECT epoch, request, new_version, digest FROM recent_commit ORDER BY seq")
        .unwrap();
    let commits = ledger
        .query_map([], |row| {
            let decoded = |index: usize| -> rusqlite::Result<u64> {
                let raw: Vec<u8> = row.get(index)?;
                Ok(sql::decode_u64(&raw).unwrap())
            };
            Ok((decoded(0)?, decoded(1)?, decoded(2)?, row.get(3)?))
        })
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    SaveRows {
        document: conn
            .query_row(
                "SELECT body FROM preference_document WHERE id = 1",
                [],
                |row| row.get(0),
            )
            .unwrap(),
        head: number("SELECT version FROM domain_head WHERE domain = 'preferences'"),
        sequence: number("SELECT sequence FROM store_meta WHERE id = 1"),
        commits,
    }
}

/// A COMMIT that ends in a disk error is uncertain, and stays so when the
/// reconnect that would settle it fails (plan §4.2): the save is not retried
/// (a retry would decide against the head its own COMMIT made), is never
/// said not to be saved, and settles once with its outcome unknown, whether
/// the reopening is busy or refused. The database holds the one commit the
/// save made, and the cache follows it.
#[test]
fn an_uncertain_commit_whose_reconnect_fails_stays_unknown_and_is_not_replayed() {
    let _state_lease = crate::test_process::state();
    let cases = [
        (
            "busy",
            sql::SqlFailure::Wait(sql::WaitCause::Busy),
            "another writer holds the preferences database lock",
        ),
        (
            "refused",
            sql::SqlFailure::Refused {
                reason: FailureReason::Io,
                detail: Arc::from(
                    "Preferences refused at \"/x/state.sqlite3\": the database did not open: \
                     disk gone.",
                ),
            },
            "the database did not open: disk gone",
        ),
    ];
    for (case, failure, cause) in cases {
        let scratch = IsolatedEnv::enter(&format!("t53s4-uncertain-reconnect-{case}"));
        let root = root_of(&scratch);
        let service = open(&scratch);
        let preferences = service.preferences();
        ready(&preferences);
        let audits = Arc::new(AtomicUsize::new(0));
        let counted = Arc::clone(&audits);
        service.set_audit_hook(Arc::new(move |_| {
            counted.fetch_add(1, Ordering::SeqCst);
        }));
        let terminals = Arc::new(AtomicUsize::new(0));
        let counted = Arc::clone(&terminals);
        service.set_terminal_hook(Arc::new(move |_| {
            counted.fetch_add(1, Ordering::SeqCst);
        }));
        // The attempt that settles the save leaves no preference save
        // queued or in flight (a requeued attempt leaves its save queued),
        // so that attempt's job hook, which runs after the terminal line, is
        // the fence for the counters.
        let (job_done, job_done_rx) = mpsc::channel::<()>();
        let job_done = std::sync::Mutex::new(job_done);
        let writer = Arc::downgrade(&service);
        service.set_job_hook_for_test(Arc::new(move |class, _| {
            if class != "pref-save" {
                return;
            }
            let Some(service) = writer.upgrade() else {
                return;
            };
            if service.preference_occupancy_for_test() == (0, false) {
                let _ = job_done.lock().unwrap().send(());
            }
        }));
        let attempts = Arc::new(AtomicUsize::new(0));
        let counted = Arc::clone(&attempts);
        let fault = Arc::new(std::sync::Mutex::new(Some(
            sql::CommitFault::IoErrAfterCommit,
        )));
        let reconnect = Arc::new(std::sync::Mutex::new(Some(failure)));
        service.set_sql_seams_for_test(sql::Seams {
            commit_fault: Some(Arc::new(move || fault.lock().unwrap().take())),
            reconnect_fault: Some(Arc::new(move || reconnect.lock().unwrap().take())),
            after_commit: Some(Arc::new(move |_| {
                counted.fetch_add(1, Ordering::SeqCst);
            })),
            ..sql::Seams::default()
        });

        let before = save_rows(&root);
        let Update::Queued(mut update) = preferences.begin_set("n", json!(5)).unwrap() else {
            panic!("queued");
        };
        let request = update.request_id();
        let failure = update
            .wait(WAIT)
            .expect("settles")
            .expect_err("a save of unknown outcome is not reported done");
        let error = failure.to_string();
        job_done_rx
            .recv_timeout(WAIT)
            .unwrap_or_else(|_| panic!("{case}: the writer job never completed"));

        assert!(error.contains("outcome is unknown"), "{case}: {error}");
        assert!(failure.outcome_unknown(), "{case}: flagged unknown");
        assert_eq!(
            failure.code(),
            Some(ErrorCode::Failed {
                domain: LocalStateDomain::Preferences,
                reason: FailureReason::Io,
            }),
            "{case}: the neutral code is the store I/O failure"
        );
        assert!(
            error.contains("could not be reopened to check it"),
            "{case}: {error}"
        );
        assert!(error.contains(cause), "{case}: {error}");
        assert!(!error.contains("not saved"), "{case}: {error}");
        assert!(
            matches!(
                service.request_state(request),
                RequestState::Settled(Settlement::Failed(_))
            ),
            "{case}"
        );
        assert_eq!(
            attempts.load(Ordering::SeqCst),
            1,
            "{case}: the save ran once and was not retried"
        );
        assert_eq!(audits.load(Ordering::SeqCst), 0, "{case}: no saved line");
        assert_eq!(
            terminals.load(Ordering::SeqCst),
            1,
            "{case}: exactly one terminal line"
        );
        // The COMMIT did reach the disk, once, at one version.
        let after = save_rows(&root);
        assert_eq!(after.head, before.head + 1, "{case}");
        assert_eq!(after.commits.len(), before.commits.len() + 1, "{case}");
        assert_eq!(stored_document(&root)["n"], json!(5), "{case}");
        // The preferences show what the database holds.
        preferences.reload().unwrap();
        let snapshot = ready(&preferences);
        assert_eq!(snapshot.version().get(), after.head, "{case}");
        assert_eq!(snapshot.document()["n"], json!(5), "{case}");
    }
}

/// The front-end test seam of an unknown outcome settles a save the way the
/// writer settles an unresolved COMMIT: `Failed(Io)`, flagged unknown, with
/// the writer's own wording, and nothing written. A later save, with the
/// seam off, is saved.
#[test]
fn the_front_end_seam_of_an_unknown_outcome_settles_like_an_unresolved_commit() {
    let _state_lease = crate::test_process::state();
    let scratch = IsolatedEnv::enter("t53s4-seam-unknown");
    let root = root_of(&scratch);
    let service = open(&scratch);
    let preferences = service.preferences();
    ready(&preferences);
    let unknown = Arc::new(AtomicBool::new(true));
    let asked = Arc::clone(&unknown);
    service.set_write_unknown_for_test(Arc::new(move || asked.load(Ordering::SeqCst)));
    let before = head(&root);

    let Update::Queued(mut update) = preferences.begin_set("n", json!(1)).unwrap() else {
        panic!("queued");
    };
    let failure = update
        .wait(WAIT)
        .expect("settles")
        .expect_err("a save of unknown outcome is not reported done");
    let error = failure.to_string();
    assert!(failure.outcome_unknown(), "{error}");
    assert_eq!(
        failure.code(),
        Some(ErrorCode::Failed {
            domain: LocalStateDomain::Preferences,
            reason: FailureReason::Io,
        })
    );
    assert!(error.contains("outcome is unknown"), "{error}");
    assert!(error.contains("it was not retried"), "{error}");
    assert!(!error.contains("not saved"), "{error}");
    assert_eq!(head(&root), before, "the seam writes nothing");

    unknown.store(false, Ordering::SeqCst);
    save(&preferences, "n", json!(2)).unwrap();
    assert_eq!(stored_document(&root)["n"], json!(2));
}

#[test]
fn a_held_sqlite_write_lock_fails_the_save_busy_at_its_deadline_while_bank_hints_save() {
    let _state_lease = crate::test_process::state();
    let scratch = IsolatedEnv::enter("t53s4-held-lock");
    let root = root_of(&scratch);
    let service = open(&scratch);
    let preferences = service.preferences();
    ready(&preferences);
    let holder = inspect(&root);
    holder.busy_timeout(Duration::ZERO).unwrap();
    holder.execute_batch("BEGIN IMMEDIATE").unwrap();
    let started = Instant::now();
    let Update::Queued(mut pending) = preferences.begin_set("n", json!(1)).unwrap() else {
        panic!("a change is queued");
    };
    let hint = submit_hint(&service, "bob", 1_760_000_001);
    // Not behind the waiting save: the hint settles while that save is still
    // pending. An order oracle, not a time limit (a loaded host can take
    // seconds for the hint's own file I/O); the bound is a hang guard.
    let mut saw_wait = false;
    let hint_settlement = loop {
        if let Some(settlement) = settled(&service, hint, Duration::ZERO) {
            break settlement;
        }
        assert!(
            pending.poll().is_none(),
            "a bank hint is behind the waiting preference save"
        );
        saw_wait |= preferences.wait_cause().is_some();
        assert!(started.elapsed() < WAIT, "the bank hint never settled");
        std::thread::sleep(Duration::from_millis(1));
    };
    assert!(
        matches!(hint_settlement, Settlement::Completed(_)),
        "{hint_settlement:?}"
    );
    while pending.poll().is_none() {
        saw_wait |= preferences.wait_cause().is_some();
        assert!(started.elapsed() < Duration::from_secs(30), "never settled");
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(saw_wait, "the pending save shows why it waits");
    let waited = started.elapsed();
    assert!(waited >= Duration::from_secs(5), "{waited:?}");
    holder.execute_batch("ROLLBACK").unwrap();
    // A new save after the release completes.
    save(&preferences, "n", json!(2)).unwrap();
    assert_eq!(ready(&preferences).document()["n"], json!(2));
}

#[test]
fn a_pinned_reader_fills_the_wal_to_its_bound_then_the_idle_reset_lets_the_next_save_through() {
    let _state_lease = crate::test_process::state();
    let scratch = IsolatedEnv::enter("t53s4-wal");
    let root = root_of(&scratch);
    let service = open(&scratch);
    let preferences = service.preferences();
    ready(&preferences);
    save(&preferences, "seed", json!(0)).unwrap();
    let reader = inspect(&root);
    reader.execute_batch("BEGIN").unwrap();
    let _: i64 = reader
        .query_row("SELECT count(*) FROM preference_document", [], |row| {
            row.get(0)
        })
        .unwrap();
    let probe = inspect(&root);
    let [_, wal, _] = sql::database_set(&root);
    // About 0.95 MiB per save: every member changes each time.
    let big_patch = |round: usize| {
        let mut patch = PreferencePatch::new();
        for member in 0..240 {
            let text = format!("{round:04}-{member:03}-{}", "v".repeat(3_900));
            patch
                .set_member(&["bulk", &format!("m{member:03}")], Some(json!(text)))
                .unwrap();
        }
        patch
    };
    let mut max_frames = 0;
    let mut wal_wait = None;
    for round in 0..60 {
        let Update::Queued(mut update) = preferences.begin_patch(big_patch(round)).unwrap() else {
            panic!("queued");
        };
        let result = update.wait(WAIT).expect("settles");
        let frames = wal_frames(&probe);
        max_frames = max_frames.max(frames);
        assert!(frames as u64 <= sql::WAL_MAX_FRAMES, "{frames} frames");
        assert!(
            std::fs::metadata(&wal).map_or(0, |meta| meta.len()) <= sql::WAL_MAX_BYTES,
            "WAL bytes past the bound"
        );
        if let Err(error) = result {
            wal_wait = Some(error.to_string());
            break;
        }
    }
    let error = wal_wait.expect("the pinned reader stops admission at the bound");
    assert!(
        error.contains("Preferences not saved: waiting for a SQLite reader to release the WAL."),
        "{error}"
    );
    assert!(
        max_frames as u64 >= sql::WAL_MAX_FRAMES - sql::MAX_PAGES,
        "the WAL crossed W - P = 8192 frames ({max_frames})"
    );
    // The reader stays pinned until the hint settles: this checks domain
    // independence, not write latency. The timeout only detects a hang.
    let hint = submit_hint(&service, "carol", 1_760_000_002);
    let hint_settlement = settled(&service, hint, Duration::from_secs(30))
        .expect("the bank hint settles while the reader stays pinned");
    assert!(
        matches!(hint_settlement, Settlement::Completed(_)),
        "{hint_settlement:?}"
    );
    reader.execute_batch("COMMIT").unwrap();
    drop(reader);
    let deadline = Instant::now() + WAIT;
    while service.truncate_counts_for_test().0 == 0 {
        assert!(Instant::now() < deadline, "the idle reset never ran");
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(wal_frames(&probe), 0, "TRUNCATE resets the log count");
    assert_eq!(
        std::fs::metadata(&wal).unwrap().len(),
        0,
        "a zero-length WAL"
    );
    save(&preferences, "after", json!(true)).unwrap();
    // Repeated cycles without a reader stay inside the bound.
    for round in 100..130 {
        let Update::Queued(mut update) = preferences.begin_patch(big_patch(round)).unwrap() else {
            panic!("queued");
        };
        update.wait(WAIT).expect("settles").unwrap();
        assert!(wal_frames(&probe) as u64 <= sql::WAL_MAX_FRAMES);
        assert!(std::fs::metadata(&wal).map_or(0, |meta| meta.len()) <= sql::WAL_MAX_BYTES);
    }
    assert!(
        service.truncate_counts_for_test().0 >= 2,
        "idle resets repeat"
    );
}

#[test]
fn close_does_not_checkpoint_and_the_committed_wal_survives_the_reopen() {
    let _state_lease = crate::test_process::state();
    let scratch = IsolatedEnv::enter("t53s4-close");
    let root = root_of(&scratch);
    let service = open(&scratch);
    let preferences = service.preferences();
    ready(&preferences);
    save(&preferences, "kept", json!("in the wal")).unwrap();
    drop(preferences);
    drop(service);
    let [_, wal, _] = sql::database_set(&root);
    assert!(
        std::fs::metadata(&wal).unwrap().len() > 0,
        "close-time checkpoints are off: the WAL keeps its frames"
    );
    let reopened = open(&scratch);
    assert_eq!(
        ready(&reopened.preferences()).document()["kept"],
        json!("in the wal")
    );
}

#[test]
fn every_connection_reads_back_its_settings() {
    let _state_lease = crate::test_process::state();
    let scratch = IsolatedEnv::enter("t53s4-settings");
    let root = root_of(&scratch);
    let service = open(&scratch);
    ready(&service.preferences());
    let conn = sql::open_connection(&root).unwrap_or_else(|_| panic!("a second connection opens"));
    sql::verify_settings(&conn).unwrap();
    // A connection whose settings differ is a mismatch, never accepted.
    conn.execute_batch("PRAGMA wal_autocheckpoint = 1000")
        .unwrap();
    assert!(sql::verify_settings(&conn)
        .unwrap_err()
        .contains("wal_autocheckpoint"));
}

#[test]
fn exact_u64_versions_round_trip_and_exhaustion_refuses_without_wrapping() {
    let _state_lease = crate::test_process::state();
    let scratch = IsolatedEnv::enter("t53s4-u64");
    let root = root_of(&scratch);
    let service = open(&scratch);
    ready(&service.preferences());
    drop(service);
    let high = (1_u64 << 63) + 5;
    inspect(&root)
        .execute(
            "UPDATE store_meta SET sequence = ?1 WHERE id = 1",
            params![sql::be(high)],
        )
        .unwrap();
    let service = open(&scratch);
    let preferences = service.preferences();
    ready(&preferences);
    save(&preferences, "n", json!(1)).unwrap();
    assert_eq!(head(&root), high + 1, "a full u64 version, not an i64");
    assert_eq!(ready(&preferences).version().get(), high + 1);
    drop(preferences);
    drop(service);
    inspect(&root)
        .execute(
            "UPDATE store_meta SET sequence = ?1 WHERE id = 1",
            params![sql::be(u64::MAX)],
        )
        .unwrap();
    let service = open(&scratch);
    let preferences = service.preferences();
    ready(&preferences);
    let error = save(&preferences, "n", json!(2)).unwrap_err().to_string();
    assert!(error.contains("version space is spent"), "{error}");
    drop(preferences);
    drop(service);
    inspect(&root)
        .execute(
            "UPDATE store_meta SET epoch = ?1 WHERE id = 1",
            params![sql::be(u64::MAX)],
        )
        .unwrap();
    let service = open(&scratch);
    let text = refusal(&service.preferences()).to_string();
    assert!(text.contains("epoch space"), "{text}");
}

#[test]
fn expected_versions_conflict_and_disjoint_patches_rebase_across_two_services() {
    let _state_lease = crate::test_process::state();
    let scratch = IsolatedEnv::enter("t53s4-two-writers");
    let first = open(&scratch);
    let second = open(&scratch);
    let a = first.preferences();
    let b = second.preferences();
    let version = ready(&a).version();
    assert_eq!(ready(&b).version(), version);
    let identity = a.identity().clone();
    let edit = |expected| {
        DomainEdit::Preferences(
            PreferencesEdit::batch(
                identity.clone(),
                Some(expected),
                vec![PreferenceChange::set("owner", "\"x\"").unwrap()],
            )
            .unwrap(),
        )
    };
    let won = accepted(first.submit(edit(version)));
    assert!(matches!(
        settled(&first, won, WAIT),
        Some(Settlement::Completed(_))
    ));
    let lost = accepted(second.submit(edit(version)));
    assert!(
        matches!(
            settled(&second, lost, WAIT),
            Some(Settlement::Failed(ErrorCode::Conflict { .. }))
        ),
        "the same expected version conflicts in the other service"
    );
    save(&a, "left", json!(1)).unwrap();
    save(&b, "right", json!(2)).unwrap();
    let merged = ready(&b);
    assert_eq!(merged.document()["left"], json!(1));
    assert_eq!(merged.document()["right"], json!(2));
}

/// A child of [`versions_epochs_and_keys_never_repeat_across_two_processes`]:
/// it accepts three saves while its startup (and so its epoch) is held,
/// then lets the database open when the parent says go.
#[test]
#[ignore = "child process of the two-process uniqueness test"]
fn two_process_child() {
    let root = StateRoot::new(env_path("T53_ROOT"));
    let go = env_path("T53_GO");
    let accepted_marker = env_path("T53_ACCEPTED");
    let held = Arc::new(move || {
        let deadline = Instant::now() + WAIT;
        while !go.exists() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
    });
    let service = LocalStateService::open_process_with_seams_for_test(
        root,
        sql::Seams::default(),
        Some(held),
    );
    let preferences = service.preferences();
    let mut updates = Vec::new();
    for n in 0..3 {
        let key = format!("p{}-{n}", std::process::id());
        match preferences.begin_set(&key, json!(n)).unwrap() {
            Update::Queued(update) => updates.push(update),
            Update::Durable => panic!("a change is queued"),
        }
    }
    std::fs::write(&accepted_marker, b"accepted").unwrap();
    for mut update in updates {
        update.wait(WAIT).expect("settles").unwrap();
    }
}

#[test]
fn versions_epochs_and_keys_never_repeat_across_two_processes() {
    crate::test_process::isolate_state_test!(
        versions_epochs_and_keys_never_repeat_across_two_processes
    );
    let scratch = IsolatedEnv::enter("t53s4-two-processes");
    let root = root_of(&scratch);
    let go = scratch.home.join("go");
    let children: Vec<_> = (0..2)
        .map(|n| {
            let marker = scratch.home.join(format!("accepted-{n}"));
            let mut command = std::process::Command::new(crate::test_process::executable());
            command
                .args([
                    "--ignored",
                    "--exact",
                    "local_state::sql_tests::two_process_child",
                ])
                .env("T53_ROOT", root.path())
                .env("T53_GO", &go)
                .env("T53_ACCEPTED", &marker)
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped());
            (command.spawn().unwrap(), marker)
        })
        .collect();
    let deadline = Instant::now() + WAIT;
    for (_, marker) in &children {
        while !marker.exists() {
            assert!(
                Instant::now() < deadline,
                "a child never accepted its saves"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    std::fs::write(&go, b"go").unwrap();
    for (child, _) in children {
        let output = child.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "child failed: {}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let conn = inspect(&root);
    let mut statement = conn
        .prepare("SELECT epoch, request, new_version FROM recent_commit ORDER BY seq")
        .unwrap();
    let rows: Vec<(u64, u64, u64)> = statement
        .query_map([], |row| {
            let epoch: Vec<u8> = row.get(0)?;
            let request: Vec<u8> = row.get(1)?;
            let version: Vec<u8> = row.get(2)?;
            Ok((
                sql::decode_u64(&epoch).unwrap(),
                sql::decode_u64(&request).unwrap(),
                sql::decode_u64(&version).unwrap(),
            ))
        })
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(rows.len(), 6, "{rows:?}");
    let epochs: std::collections::BTreeSet<u64> = rows.iter().map(|row| row.0).collect();
    assert_eq!(epochs.len(), 2, "each process drew its own epoch: {rows:?}");
    let raw: std::collections::BTreeSet<u64> = rows.iter().map(|row| row.1).collect();
    assert!(
        raw.len() < rows.len(),
        "equal process-local ids occur: {rows:?}"
    );
    let pairs: std::collections::BTreeSet<(u64, u64)> =
        rows.iter().map(|row| (row.0, row.1)).collect();
    assert_eq!(
        pairs.len(),
        rows.len(),
        "stored keys are distinct: {rows:?}"
    );
    let versions: std::collections::BTreeSet<u64> = rows.iter().map(|row| row.2).collect();
    assert_eq!(
        versions.len(),
        rows.len(),
        "durable versions are distinct: {rows:?}"
    );
}

/// A child of the frontend-pair test: one process of `T53_FRONTEND`
/// (`panel` or `tui`) on `T53_ROOT` saves its own member `T53_KEY`, then
/// waits until it reads the other process's member `T53_OTHER`.
#[test]
#[ignore = "child process of the frontend-pair test"]
fn frontend_pair_child() {
    let root = StateRoot::new(env_path("T53_ROOT"));
    let frontend = match std::env::var("T53_FRONTEND").unwrap().as_str() {
        "panel" => Frontend::Panel,
        "tui" => Frontend::Tui,
        other => panic!("unknown frontend {other}"),
    };
    let key = std::env::var("T53_KEY").unwrap();
    let other = std::env::var("T53_OTHER").unwrap();
    let service = LocalStateService::open_process_with(root, frontend, None);
    let preferences = service.preferences();
    ready(&preferences);
    save(&preferences, &key, json!(true)).expect("this process's save lands");
    let deadline = Instant::now() + WAIT;
    while preferences.value(&other).is_none() {
        assert!(Instant::now() < deadline, "never read {other}");
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(preferences.value(&key), Some(json!(true)), "kept its own");
}

/// Plan §9.5: panel+panel, TUI+TUI and panel+TUI on one local root. Two
/// processes of each pair save at once; both members land at distinct
/// versions, and each process reads the other's.
#[test]
fn panel_and_tui_processes_share_one_root_in_every_pairing() {
    crate::test_process::isolate_state_test!(
        panel_and_tui_processes_share_one_root_in_every_pairing
    );
    for (first, second) in [("panel", "panel"), ("tui", "tui"), ("panel", "tui")] {
        let scratch = IsolatedEnv::enter(&format!("t53s4-pair-{first}-{second}"));
        let root = root_of(&scratch);
        let keys = [format!("{first}_a"), format!("{second}_b")];
        let children: Vec<_> = [(first, 0), (second, 1)]
            .into_iter()
            .map(|(frontend, n)| {
                std::process::Command::new(crate::test_process::executable())
                    .args([
                        "--ignored",
                        "--exact",
                        "local_state::sql_tests::frontend_pair_child",
                    ])
                    .env("T53_ROOT", root.path())
                    .env("T53_FRONTEND", frontend)
                    .env("T53_KEY", &keys[n])
                    .env("T53_OTHER", &keys[1 - n])
                    .stdout(std::process::Stdio::piped())
                    .stderr(std::process::Stdio::piped())
                    .spawn()
                    .unwrap()
            })
            .collect();
        for child in children {
            let output = child.wait_with_output().unwrap();
            assert!(
                output.status.success(),
                "{first}+{second} child failed: {}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        }
        let conn = inspect(&root);
        let versions: Vec<Vec<u8>> = conn
            .prepare("SELECT new_version FROM recent_commit ORDER BY seq")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        let distinct: std::collections::BTreeSet<_> = versions.iter().collect();
        assert_eq!(versions.len(), 2, "{first}+{second}: two saves");
        assert_eq!(distinct.len(), 2, "{first}+{second}: distinct versions");
        let service = open(&scratch);
        let preferences = service.preferences();
        ready(&preferences);
        for key in &keys {
            assert_eq!(
                preferences.value(key),
                Some(json!(true)),
                "{first}+{second}: {key}"
            );
        }
    }
}

/// A child of the crash tests: open with an abort at `T53_CRASH_AT`.
#[test]
#[ignore = "child process of the crash tests"]
fn crash_child() {
    let root = StateRoot::new(env_path("T53_ROOT"));
    let at = std::env::var("T53_CRASH_AT").unwrap();
    let point: Arc<dyn Fn(&'static str) + Send + Sync> = Arc::new(move |name| {
        if name == at {
            std::process::abort();
        }
    });
    let service = LocalStateService::open_process_with_seams_for_test(
        root,
        sql::Seams {
            point: Some(point),
            ..sql::Seams::default()
        },
        None,
    );
    let _ = ready(&service.preferences());
}

fn crash_at(scratch: &IsolatedEnv, point: &str) {
    let status = std::process::Command::new(crate::test_process::executable())
        .args([
            "--ignored",
            "--exact",
            "local_state::sql_tests::crash_child",
        ])
        .env("T53_ROOT", root_of(scratch).path())
        .env("T53_CRASH_AT", point)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .unwrap();
    assert!(!status.success(), "the child crashed at {point}");
}

#[test]
fn a_crash_before_the_import_commit_leaves_no_ledger_and_the_next_open_imports() {
    crate::test_process::isolate_state_test!(
        a_crash_before_the_import_commit_leaves_no_ledger_and_the_next_open_imports
    );
    let scratch = IsolatedEnv::enter("t53s4-crash-before-import");
    let root = root_of(&scratch);
    let bytes = br#"{"last_focus":"heidi"}"#;
    write_private(&root.preferences_file(), bytes);
    crash_at(&scratch, "before-import-commit");
    assert_eq!(ledger_kind(&root), None);
    assert_eq!(std::fs::read(root.preferences_file()).unwrap(), bytes);
    let service = open(&scratch);
    assert_eq!(
        ready(&service.preferences()).document()["last_focus"],
        json!("heidi")
    );
}

#[test]
fn a_crash_after_the_import_commit_finishes_only_the_evidence_on_reopen() {
    crate::test_process::isolate_state_test!(
        a_crash_after_the_import_commit_finishes_only_the_evidence_on_reopen
    );
    let scratch = IsolatedEnv::enter("t53s4-crash-after-import");
    let root = root_of(&scratch);
    let bytes = br#"{"last_focus":"ivan"}"#;
    write_private(&root.preferences_file(), bytes);
    crash_at(&scratch, "before-evidence");
    assert_eq!(ledger_kind(&root).as_deref(), Some("imported"));
    let version = head(&root);
    assert!(
        root.preferences_file().exists(),
        "the crash left the source"
    );
    let service = open(&scratch);
    let snapshot = ready(&service.preferences());
    assert_eq!(snapshot.version().get(), version, "no second import");
    assert_eq!(snapshot.document()["last_focus"], json!("ivan"));
    assert!(!root.preferences_file().exists());
    assert_eq!(
        std::fs::read(root.path().join("panel-ui.json.migrated")).unwrap(),
        bytes
    );
}

#[test]
fn a_crash_after_the_malformed_copy_commits_defaults_once_and_never_loses_the_source() {
    crate::test_process::isolate_state_test!(
        a_crash_after_the_malformed_copy_commits_defaults_once_and_never_loses_the_source
    );
    let scratch = IsolatedEnv::enter("t53s4-crash-quarantine");
    let root = root_of(&scratch);
    let damaged = b"not json at all";
    write_private(&root.preferences_file(), damaged);
    crash_at(&scratch, "after-quarantine-copy");
    assert_eq!(ledger_kind(&root), None);
    assert_eq!(std::fs::read(root.preferences_file()).unwrap(), damaged);
    let service = open(&scratch);
    assert!(ready(&service.preferences()).document().is_empty());
    assert_eq!(ledger_kind(&root).as_deref(), Some("quarantined"));
    let copies = std::fs::read_dir(root.path())
        .unwrap()
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.file_name().to_string_lossy().contains(".corrupt-"))
        .count();
    assert!(copies >= 1, "the damaged bytes are kept");
}

/// A child of [`a_process_that_dies_in_a_save_leaves_all_of_it_or_none_of_it`]:
/// one save through the service, and the process aborts at the database's
/// named point `T53_CRASH_AT` (`before-commit`: inside the open write
/// transaction, every statement of the save run and none committed;
/// `after-commit`: the COMMIT returned and nothing of the save is settled).
#[test]
#[ignore = "child process of the save crash test"]
fn save_crash_child() {
    let root = StateRoot::new(env_path("T53_ROOT"));
    let at = std::env::var("T53_CRASH_AT").unwrap();
    let point: Arc<dyn Fn(&'static str) + Send + Sync> = Arc::new({
        let at = at.clone();
        move |name| {
            if name == at {
                std::process::abort();
            }
        }
    });
    let service = LocalStateService::open_process_with_seams_for_test(
        root,
        sql::Seams {
            point: Some(point),
            ..sql::Seams::default()
        },
        None,
    );
    let preferences = service.preferences();
    ready(&preferences);
    let _ = save(&preferences, "crashed", json!(2));
    panic!("the child should have aborted at {at}");
}

/// A real process that dies in a save leaves all of it or none of it (plan
/// §5.4, §9.5). Inside the open write transaction, with every statement of
/// the save run and none committed, the document, the domain head, the
/// version sequence and the commit ledger are all as they were. Right after
/// the COMMIT returned, all four hold the save, at one version, and the
/// ledger's digest is the document's. A new process then reads what is
/// durable, from a database that passes its own integrity check.
#[cfg(unix)]
#[test]
fn a_process_that_dies_in_a_save_leaves_all_of_it_or_none_of_it() {
    use std::os::unix::process::ExitStatusExt;
    crate::test_process::isolate_state_test!(
        a_process_that_dies_in_a_save_leaves_all_of_it_or_none_of_it
    );
    for (point, durable) in [("before-commit", false), ("after-commit", true)] {
        let scratch = IsolatedEnv::enter(&format!("t53s4-save-crash-{point}"));
        let root = root_of(&scratch);
        {
            let service = open(&scratch);
            let preferences = service.preferences();
            ready(&preferences);
            save(&preferences, "kept", json!(1)).unwrap();
        }
        let before = save_rows(&root);
        let status = std::process::Command::new(crate::test_process::executable())
            .args([
                "--ignored",
                "--exact",
                "local_state::sql_tests::save_crash_child",
            ])
            .env("T53_ROOT", root.path())
            .env("T53_CRASH_AT", point)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .unwrap();
        assert_eq!(
            status.signal(),
            Some(libc::SIGABRT),
            "{point}: the child died by its abort at the point ({status})"
        );

        let after = save_rows(&root);
        let integrity: String = inspect(&root)
            .query_row("PRAGMA integrity_check", [], |row| row.get(0))
            .unwrap();
        assert_eq!(integrity, "ok", "{point}");
        let document: serde_json::Value = serde_json::from_slice(&after.document).unwrap();
        if durable {
            assert_eq!(after.head, before.head + 1, "{point}: one version");
            assert_eq!(after.sequence, after.head, "{point}: the sequence follows");
            assert_eq!(document["kept"], json!(1), "{point}");
            assert_eq!(document["crashed"], json!(2), "{point}: the whole save");
            assert_eq!(
                after.commits.len(),
                before.commits.len() + 1,
                "{point}: its ledger row"
            );
            let (_, _, version, digest) = after.commits.last().unwrap();
            assert_eq!(*version, after.head, "{point}");
            assert_eq!(
                digest.as_slice(),
                files::digest(&after.document).as_slice(),
                "{point}: the ledger names the stored document"
            );
        } else {
            assert_eq!(after, before, "{point}: nothing of the save is visible");
            assert!(document.get("crashed").is_none(), "{point}");
        }

        let service = open(&scratch);
        let snapshot = ready(&service.preferences());
        assert_eq!(snapshot.version().get(), after.head, "{point}");
        assert_eq!(snapshot.document()["kept"], json!(1), "{point}");
        assert_eq!(
            snapshot.document().get("crashed"),
            durable.then_some(&json!(2)),
            "{point}"
        );
    }
}

/// A child of [`the_live_metadata_check_never_cancels_sqlite_locks`]: it
/// probes the write lock directly with no busy wait. Exit 3 for BUSY, 0
/// when it got the lock.
#[test]
#[ignore = "child process of the live metadata check test"]
fn lock_probe_child() {
    let conn = Connection::open_with_flags(
        env_path("T53_DB"),
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .unwrap();
    conn.busy_timeout(Duration::ZERO).unwrap();
    match conn.execute_batch("BEGIN IMMEDIATE") {
        Ok(()) => {
            conn.execute_batch("ROLLBACK").unwrap();
            std::process::exit(0)
        }
        Err(error) if sql::wait_of(&error).is_some() => std::process::exit(3),
        Err(error) => panic!("{error}"),
    }
}

#[test]
fn the_live_metadata_check_never_cancels_sqlite_locks() {
    crate::test_process::isolate_state_test!(the_live_metadata_check_never_cancels_sqlite_locks);
    let scratch = IsolatedEnv::enter("t53s4-live-check");
    let root = root_of(&scratch);
    let service = open(&scratch);
    ready(&service.preferences());
    let holder = inspect(&root);
    holder.execute_batch("BEGIN IMMEDIATE").unwrap();
    holder
        .execute("UPDATE store_meta SET epoch = epoch WHERE id = 1", [])
        .unwrap();
    for file in sql::database_set(&root) {
        sql::check_private(&file, vault::PrivateKind::File).unwrap();
    }
    sql::check_private(&root.state_dir(), vault::PrivateKind::Dir).unwrap();
    let db = root.database_file();
    let probe = || {
        child(
            "local_state::sql_tests::lock_probe_child",
            &[("T53_DB", &db)],
        )
    };
    assert_eq!(
        probe().code(),
        Some(3),
        "the write lock still holds after the checks"
    );
    holder.execute_batch("COMMIT").unwrap();
    assert_eq!(probe().code(), Some(0));
}

/// The permission bits of `path`.
#[cfg(unix)]
fn mode_of(path: &std::path::Path) -> u32 {
    std::fs::metadata(path).unwrap().permissions().mode() & 0o777
}

/// The state directory is 0700 and every file of the live database set 0600.
#[cfg(unix)]
fn assert_private_live_set(root: &StateRoot) {
    assert_eq!(mode_of(&root.state_dir()), 0o700);
    for file in sql::database_set(root) {
        assert_eq!(mode_of(&file), 0o600, "{}", file.display());
    }
}

/// A child of [`private_files_stay_private_under_both_umasks`]: the live
/// set, then, after a recovery that preserves it and starts a new history,
/// the preserved set and the new live set. Every directory is 0700 and every
/// file 0600 whatever the umask the child runs under.
#[test]
#[ignore = "child process of the umask test"]
fn umask_child() {
    let root = StateRoot::new(env_path("T53_ROOT"));
    {
        let service = LocalStateService::open_process(root.clone());
        let preferences = service.preferences();
        ready(&preferences);
        save(&preferences, "n", json!(1)).unwrap();
        #[cfg(unix)]
        assert_private_live_set(&root);
    }
    // The service is closed: the recovery takes the root exclusively.
    run_preferences_maintenance(
        &root,
        Frontend::Headless,
        &MaintenanceAction::RecoverDefaults,
    )
    .unwrap();
    #[cfg(unix)]
    {
        assert_private_live_set(&root);
        let preserved: Vec<_> = std::fs::read_dir(root.state_dir())
            .unwrap()
            .filter_map(|entry| entry.ok())
            .filter(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with("preserved-")
            })
            .collect();
        assert_eq!(preserved.len(), 1, "the recovery preserved the old set");
        let preserved = preserved[0].path();
        assert_eq!(mode_of(&preserved), 0o700, "{}", preserved.display());
        let copies: Vec<_> = std::fs::read_dir(&preserved)
            .unwrap()
            .filter_map(|entry| entry.ok())
            .collect();
        assert!(!copies.is_empty(), "the preserved set holds the database");
        for copy in copies {
            assert_eq!(mode_of(&copy.path()), 0o600, "{}", copy.path().display());
        }
    }
}

#[cfg(unix)]
#[test]
fn private_files_stay_private_under_both_umasks() {
    crate::test_process::isolate_state_test!(private_files_stay_private_under_both_umasks);
    for mask in ["0002", "0022"] {
        let scratch = IsolatedEnv::enter(&format!("t53s4-umask-{mask}"));
        let root = root_of(&scratch);
        let output = std::process::Command::new("/bin/sh")
            .arg("-c")
            .arg(format!(
                "umask {mask} && exec \"$0\" --ignored --exact local_state::sql_tests::umask_child"
            ))
            .arg(crate::test_process::executable())
            .env("T53_ROOT", root.path())
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "umask {mask}: {}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
fn shutdown_while_the_sql_lock_is_held_settles_the_save_busy_once_and_stops() {
    let _state_lease = crate::test_process::state();
    let scratch = IsolatedEnv::enter("t53s4-shutdown-held");
    let root = root_of(&scratch);
    let service = open(&scratch);
    let preferences = service.preferences();
    ready(&preferences);
    let terminals = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&terminals);
    service.set_terminal_hook(Arc::new(move |_| {
        counted.fetch_add(1, Ordering::SeqCst);
    }));
    let holder = inspect(&root);
    holder.execute_batch("BEGIN IMMEDIATE").unwrap();
    let Update::Queued(update) = preferences.begin_set("n", json!(1)).unwrap() else {
        panic!("queued");
    };
    let request = update.request_id();
    drop(update);
    drop(preferences);
    let ledger = service.ledger();
    let started = Instant::now();
    drop(service);
    assert!(ledger.wait_writer_exit(WAIT), "the writer stops");
    assert!(started.elapsed() < Duration::from_secs(30));
    assert!(matches!(
        ledger.request_state(request),
        RequestState::Settled(Settlement::Failed(ErrorCode::Busy { .. }))
            | RequestState::Settled(Settlement::Cancelled(_))
    ));
    assert_eq!(terminals.load(Ordering::SeqCst), 1, "one terminal line");
    holder.execute_batch("ROLLBACK").unwrap();
}

/// Plan §4.4, §4.5: a transaction that has begun when the shutdown bound
/// passes keeps its own outcome. Held inside its open transaction past the
/// bound, the save is neither cancelled nor lost: it commits when released,
/// with one audit and one terminal result (completed). A save accepted
/// behind it never began and is cancelled at the bound, once.
#[test]
fn a_transaction_begun_before_the_shutdown_bound_keeps_its_own_outcome() {
    let _state_lease = crate::test_process::state();
    let scratch = IsolatedEnv::enter("t53s4-shutdown-begun");
    let root = root_of(&scratch);
    let service = open(&scratch);
    let preferences = service.preferences();
    ready(&preferences);
    let before = head(&root);
    let audits = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&audits);
    service.set_audit_hook(Arc::new(move |_| {
        counted.fetch_add(1, Ordering::SeqCst);
    }));
    let terminals = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&terminals);
    service.set_terminal_hook(Arc::new(move |_| {
        counted.fetch_add(1, Ordering::SeqCst);
    }));
    service.set_shutdown_timeout(Duration::from_millis(300));
    let (entered_tx, entered) = mpsc::channel();
    let (release, released) = mpsc::channel::<()>();
    let released = std::sync::Mutex::new(released);
    let point: Arc<dyn Fn(&'static str) + Send + Sync> = Arc::new(move |name| {
        if name == "before-commit" {
            entered_tx.send(()).unwrap();
            let _ = released.lock().unwrap().recv_timeout(WAIT);
        }
    });
    service.set_sql_seams_for_test(sql::Seams {
        point: Some(point),
        ..sql::Seams::default()
    });
    let Update::Queued(first) = preferences.begin_set("held", json!(1)).unwrap() else {
        panic!("queued");
    };
    let held = first.request_id();
    entered.recv_timeout(WAIT).expect("the transaction began");
    let Update::Queued(second) = preferences.begin_set("never", json!(2)).unwrap() else {
        panic!("queued");
    };
    let behind = second.request_id();
    drop((first, second));
    drop(preferences);
    let ledger = service.ledger();
    // The 300 ms bound passes while the transaction is held.
    drop(service);
    assert!(
        matches!(ledger.request_state(held), RequestState::Pending),
        "a begun transaction is not settled cancelled at the bound: {:?}",
        ledger.request_state(held)
    );
    assert!(
        matches!(
            ledger.request_state(behind),
            RequestState::Settled(Settlement::Cancelled(_))
        ),
        "work that never began is cancelled at the bound"
    );
    assert_eq!(head(&root), before, "nothing committed while it is held");
    release.send(()).unwrap();
    assert!(ledger.wait_writer_exit(WAIT), "the writer stops");
    assert!(
        matches!(
            ledger.request_state(held),
            RequestState::Settled(Settlement::Completed(_))
        ),
        "the begun save keeps its commit: {:?}",
        ledger.request_state(held)
    );
    let body: Vec<u8> = inspect(&root)
        .query_row(
            "SELECT body FROM preference_document WHERE id = 1",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let stored: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(stored["held"], json!(1), "the begun save is durable");
    assert!(stored.get("never").is_none(), "the cancelled save is not");
    assert!(head(&root) > before);
    assert_eq!(audits.load(Ordering::SeqCst), 1, "one audit");
    assert_eq!(
        terminals.load(Ordering::SeqCst),
        1,
        "one terminal line, the cancelled save's: the committed save's is its audit"
    );
}

/// Work the writer selected before the bound but had not begun when it
/// passed never begins its SQL transaction afterwards: it stays cancelled,
/// writes nothing and audits nothing.
#[test]
fn a_save_selected_before_the_shutdown_bound_never_begins_after_it() {
    let _state_lease = crate::test_process::state();
    let scratch = IsolatedEnv::enter("t53s4-shutdown-selected");
    let root = root_of(&scratch);
    let service = open(&scratch);
    let preferences = service.preferences();
    ready(&preferences);
    let before = head(&root);
    let audits = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&audits);
    service.set_audit_hook(Arc::new(move |_| {
        counted.fetch_add(1, Ordering::SeqCst);
    }));
    let terminals = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&terminals);
    service.set_terminal_hook(Arc::new(move |_| {
        counted.fetch_add(1, Ordering::SeqCst);
    }));
    service.set_shutdown_timeout(Duration::from_millis(300));
    let (entered_tx, entered) = mpsc::channel();
    let (release, released) = mpsc::channel::<()>();
    let released = std::sync::Mutex::new(released);
    service.set_write_stall_for_test(Arc::new(move || {
        let _ = entered_tx.send(());
        let _ = released.lock().unwrap().recv_timeout(WAIT);
    }));
    let Update::Queued(update) = preferences.begin_set("late", json!(1)).unwrap() else {
        panic!("queued");
    };
    let request = update.request_id();
    entered.recv_timeout(WAIT).expect("the save was selected");
    drop(update);
    drop(preferences);
    let ledger = service.ledger();
    drop(service);
    assert!(
        matches!(
            ledger.request_state(request),
            RequestState::Settled(Settlement::Cancelled(_))
        ),
        "an unbegun save is cancelled at the bound"
    );
    release.send(()).unwrap();
    assert!(ledger.wait_writer_exit(WAIT), "the writer stops");
    assert_eq!(head(&root), before, "the cancelled save wrote nothing");
    assert_eq!(audits.load(Ordering::SeqCst), 0, "no audit");
    assert_eq!(terminals.load(Ordering::SeqCst), 1, "one terminal line");
    assert!(matches!(
        ledger.request_state(request),
        RequestState::Settled(Settlement::Cancelled(_))
    ));
}

#[test]
fn cold_walk_reads_return_at_once_while_the_writer_is_inside_a_real_sql_call() {
    let _state_lease = crate::test_process::state();
    let scratch = IsolatedEnv::enter("t53s4-walk-in-sql");
    let (entered_tx, entered) = mpsc::channel();
    let (release, released) = mpsc::channel::<()>();
    let released = std::sync::Mutex::new(released);
    let stalled = AtomicBool::new(false);
    let stall: Arc<dyn Fn() + Send + Sync> = Arc::new(move || {
        if !stalled.swap(true, Ordering::SeqCst) {
            entered_tx.send(()).unwrap();
            let _ = released.lock().unwrap().recv_timeout(WAIT);
        }
    });
    let service = LocalStateService::open_process_with_seams_for_test(
        root_of(&scratch),
        sql::Seams {
            stall_in_sql: Some(stall),
            ..sql::Seams::default()
        },
        None,
    );
    entered.recv_timeout(WAIT).expect("the writer entered SQL");
    let preferences = service.preferences();
    let started = Instant::now();
    let walk = preferences.walk_globals();
    let document = preferences.document();
    assert!(walk.unwrap_err().is_not_ready());
    assert!(document.unwrap_err().is_not_ready());
    assert!(
        started.elapsed() < Duration::from_millis(16),
        "{:?}",
        started.elapsed()
    );
    release.send(()).unwrap();
    ready(&preferences);
    assert!(preferences.walk_globals().is_ok());
}

/// A Start through the service readback API waits for the preferences'
/// first import while it runs. It never queues behind the import on the
/// shared writer, which cannot preempt a job in flight (operator decision
/// on the S1 occupancy gate, 2026-11-15). The import is held inside a real
/// SQL call. The Start's read, as a queue request and as a bounded call,
/// ends `NotReady` and leaves nothing of itself in the writer's queue; once
/// the import is released, a Start that waited for it reads, and so does
/// the composition's Start read of the loadouts.
#[test]
fn a_start_readback_waits_for_the_first_import_and_never_queues_behind_it() {
    let _state_lease = crate::test_process::state();
    let scratch = IsolatedEnv::enter("t53s4-start-vs-import");
    let (entered_tx, entered) = mpsc::channel();
    let (release, released) = mpsc::channel::<()>();
    let released = std::sync::Mutex::new(released);
    let stalled = AtomicBool::new(false);
    let stall: Arc<dyn Fn() + Send + Sync> = Arc::new(move || {
        if !stalled.swap(true, Ordering::SeqCst) {
            entered_tx.send(()).unwrap();
            let _ = released.lock().unwrap().recv_timeout(WAIT);
        }
    });
    let service = LocalStateService::open_process_with_seams_for_test(
        root_of(&scratch),
        sql::Seams {
            stall_in_sql: Some(stall),
            ..sql::Seams::default()
        },
        None,
    );
    entered
        .recv_timeout(WAIT)
        .expect("the first import is in flight");
    let scope = ContentScope::try_new(client::io::ClientRevision::R289, "test-content").unwrap();
    let composition = StateComposition::new(Arc::clone(&service), "local-289", scope);
    let loadouts = composition
        .identity(LocalStateDomain::LoadoutsReferences, "alice")
        .unwrap();
    let not_ready = ErrorCode::NotReady {
        domain: LocalStateDomain::LoadoutsReferences,
    };
    let queued = || {
        service
            .shared
            .state
            .lock()
            .stores
            .get(&StoreKey::Loadouts)
            .is_some_and(|store| store.readback != Readback::Idle)
    };
    assert!(
        !queued(),
        "the open's own loadouts read ran before the import"
    );

    // As a queue request: refused at once, nothing queued.
    assert_eq!(service.start_readback(&loadouts).err(), Some(not_ready));
    assert!(
        !queued(),
        "the Start's read is not queued behind the import"
    );
    // As the bounded call: the wait for the import counts against its
    // timeout, and nothing is queued behind the import when it ends.
    let started = Instant::now();
    assert_eq!(
        service.readback_lease(&loadouts, Duration::from_millis(250)),
        Err(not_ready)
    );
    assert!(
        started.elapsed() >= Duration::from_millis(250),
        "the Start waited for the import, not for a refusal: {:?}",
        started.elapsed()
    );
    assert!(
        !queued(),
        "the Start's read is not queued behind the import"
    );

    // A Start with room to wait waits for the import, then reads.
    let waiting = {
        let service = Arc::clone(&service);
        let loadouts = loadouts.clone();
        std::thread::spawn(move || service.readback_lease(&loadouts, WAIT))
    };
    release.send(()).unwrap();
    assert_eq!(
        waiting.join().unwrap(),
        Ok(None),
        "no loadouts file is no rows"
    );
    ready(&service.preferences());
    assert!(composition.operator_loadouts("alice").unwrap().is_empty());
}

/// A first read of the preferences that only waits, because another
/// connection holds the SQL write lock, is no import in flight: a Start's
/// read and a bank load are not held back by it (plan §4.4, §5.4: bank,
/// Start and exit jobs keep running while the lock is held, and file reads
/// go before preference jobs). The first read keeps waiting for the lock
/// meanwhile, and opens when the lock is released.
#[test]
fn a_start_readback_is_not_held_back_by_a_first_read_that_waits_for_the_sql_write_lock() {
    let _state_lease = crate::test_process::state();
    let scratch = IsolatedEnv::enter("t53s4-start-vs-sql-lock");
    let root = root_of(&scratch);
    ready(&open(&scratch).preferences());
    let holder = inspect(&root);
    holder.busy_timeout(Duration::ZERO).unwrap();
    holder.execute_batch("BEGIN IMMEDIATE").unwrap();
    let service = open(&scratch);
    let preferences = service.preferences();
    let deadline = Instant::now() + Duration::from_secs(10);
    while preferences.wait_cause().is_none() {
        assert!(
            Instant::now() < deadline,
            "the first read never waited for the lock"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    let scope = ContentScope::try_new(client::io::ClientRevision::R289, "test-content").unwrap();
    let composition = StateComposition::new(Arc::clone(&service), "local-289", scope);
    let loadouts = composition
        .identity(LocalStateDomain::LoadoutsReferences, "alice")
        .unwrap();
    // Two seconds are a hang guard, not a time limit: a read does not wait
    // for the lock, so it answers at once.
    assert_eq!(
        service.readback_lease(&loadouts, Duration::from_secs(2)),
        Ok(None),
        "a Start's read waited for the first read that waits for the lock"
    );
    assert_eq!(
        service.readback_lease(&bank_identity("alice"), Duration::from_secs(2)),
        Ok(None),
        "a bank load waited for the first read that waits for the lock"
    );
    assert!(
        matches!(preferences.snapshot(), Err(error) if error.is_not_ready()),
        "the first read still waits for the lock"
    );
    holder.execute_batch("ROLLBACK").unwrap();
    ready(&preferences);
}

/// A first read of the preferences that panics (a contained writer-job
/// panic) refuses the preferences for the run, with its cause and remedy,
/// as any other refusal does. They would otherwise stay cold, a save
/// accepted afterwards would never settle, nothing would ask for another
/// read, and every Start and bank load would wait for nothing. A Start's
/// read and a bank load go on, and a save gets the refusal.
#[test]
fn a_panicking_first_preferences_read_refuses_them_and_holds_nothing_back() {
    let _state_lease = crate::test_process::state();
    let scratch = IsolatedEnv::enter("t53s4-first-read-panics");
    let calls = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&calls);
    let service = LocalStateService::open_process_with_read_stall_for_test(
        root_of(&scratch),
        Arc::new(move || {
            // Call 1 is the loadouts read the open queues first (file reads
            // go before preference jobs), call 2 the preferences' first read.
            if counted.fetch_add(1, Ordering::SeqCst) == 1 {
                panic!("the first preferences read panics");
            }
        }),
    );
    let preferences = service.preferences();
    let deadline = Instant::now() + Duration::from_secs(10);
    let refusal = loop {
        match preferences.snapshot() {
            Err(error) if error.is_not_ready() => {}
            Err(error) => break error,
            Ok(snapshot) => panic!(
                "the preferences opened at {} after their first read panicked",
                snapshot.version()
            ),
        }
        assert!(
            Instant::now() < deadline,
            "the panicked first read left the preferences cold"
        );
        std::thread::sleep(Duration::from_millis(5));
    };
    let text = refusal.to_string();
    assert!(
        text.contains("Preferences refused at")
            && text.contains("the first preferences read panics")
            && text.contains("Restart 274bot"),
        "the refusal names its path, cause and remedy: {text}"
    );
    let scope = ContentScope::try_new(client::io::ClientRevision::R289, "test-content").unwrap();
    let composition = StateComposition::new(Arc::clone(&service), "local-289", scope);
    let loadouts = composition
        .identity(LocalStateDomain::LoadoutsReferences, "alice")
        .unwrap();
    assert_eq!(
        service.readback_lease(&loadouts, Duration::from_secs(10)),
        Ok(None),
        "a Start's read goes on"
    );
    assert_eq!(
        service.readback_lease(&bank_identity("alice"), Duration::from_secs(10)),
        Ok(None),
        "a bank load goes on"
    );
    match preferences.begin_set("n", json!(1)) {
        Err(error) => assert!(
            error.to_string().contains("Preferences refused at"),
            "{error}"
        ),
        Ok(Update::Queued(mut update)) => {
            let failure = update
                .wait(Duration::from_secs(10))
                .expect("a save over refused preferences settles")
                .expect_err("refused preferences save nothing");
            assert!(
                failure.to_string().contains("Preferences refused at"),
                "{failure}"
            );
        }
        Ok(Update::Durable) => panic!("refused preferences saved"),
    }
}

/// Run `read` on its own thread while the writer is held at `phase`, and
/// return what it returned. It must finish before `release` is sent; the 4 s
/// guard only turns a hang into a failure.
fn answers_while_held<T: Send + 'static>(
    phase: &str,
    release: &mpsc::Sender<()>,
    read: impl FnOnce() -> T + Send + 'static,
) -> T {
    let (done_tx, done) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        let _ = done_tx.send(read());
    });
    match done.recv_timeout(Duration::from_secs(4)) {
        Ok(value) => {
            worker.join().unwrap();
            value
        }
        Err(_) => {
            let _ = release.send(());
            let _ = worker.join();
            panic!("{phase}: a cached read waited for the held writer");
        }
    }
}

/// The cold-start test above holds the first hydration. A warm service has
/// two more places where the writer is busy inside a real SQL call and no
/// cache-only reader may wait for it (plan §3.2, §9.5): the refresh that
/// follows another process's commit, and a save. Hold the writer inside
/// the SQL of each, and the cached walk view, document and snapshot still
/// answer at once with what the cache held; after the release the refresh
/// has adopted the other process's commit and the save is durable. The
/// build of the lease rows has its own test below, which watches the build.
#[test]
fn warm_cached_reads_never_wait_for_the_writer_in_a_refresh_or_a_save() {
    let _state_lease = crate::test_process::state();
    for phase in ["refresh", "save"] {
        let scratch = IsolatedEnv::enter(&format!("t53s4-warm-hold-{phase}"));
        let service = open(&scratch);
        let preferences = service.preferences();
        ready(&preferences);

        // The hold is installed while the cache is fresh and nobody reads
        // it, so no refresh can run before the one this test asks for.
        let (entered_tx, entered) = mpsc::channel();
        let (release, released) = mpsc::channel::<()>();
        let released = std::sync::Mutex::new(released);
        let armed = AtomicBool::new(true);
        let hold: Arc<dyn Fn() + Send + Sync> = Arc::new(move || {
            if armed.swap(false, Ordering::SeqCst) {
                entered_tx.send(()).unwrap();
                let _ = released.lock().unwrap().recv_timeout(WAIT);
            }
        });
        service.set_sql_seams_for_test(sql::Seams {
            stall_in_sql: Some(hold),
            ..sql::Seams::default()
        });
        // Another process of the root commits: this service has a version to
        // adopt at its next refresh.
        let peer = open(&scratch);
        save(&peer.preferences(), "peer", json!(1)).unwrap();

        let mut saving = None;
        if phase == "save" {
            let Update::Queued(update) = preferences.begin_set("n", json!(1)).unwrap() else {
                panic!("queued");
            };
            saving = Some(update);
        } else {
            // A stale cache asks for a refresh.
            service.expire_preference_cache_for_test();
            let _ = preferences.walk_globals();
        }
        entered
            .recv_timeout(WAIT)
            .unwrap_or_else(|_| panic!("{phase}: the writer never reached the held point"));

        let reader = preferences.clone();
        let (walk, document, snapshot) = answers_while_held(phase, &release, move || {
            (reader.walk_globals(), reader.document(), reader.snapshot())
        });
        assert!(walk.is_ok(), "{phase}: the walk view reads while held");
        let document = document.unwrap_or_else(|error| panic!("{phase}: {error}"));
        assert!(snapshot.is_ok(), "{phase}: the snapshot reads while held");
        assert!(
            document.get("peer").is_none() && document.get("n").is_none(),
            "{phase}: what the cache held, before the release"
        );

        release.send(()).unwrap();
        match saving {
            Some(mut update) => {
                update.wait(WAIT).expect("settles").expect("durable");
                assert_eq!(ready(&preferences).document()["n"], json!(1));
            }
            None => {
                let deadline = Instant::now() + WAIT;
                while ready(&preferences).document().get("peer").is_none() {
                    assert!(
                        Instant::now() < deadline,
                        "{phase}: the held refresh never adopted the other process's commit"
                    );
                    std::thread::sleep(Duration::from_millis(5));
                }
            }
        }
    }
}

/// The lease rows of a version the writer adopts are built on the writer
/// thread with no service lock held (plan §3.2, §9.5), so a cache-only
/// reader answers while they are built. The test watches the build itself,
/// not a marker beside it: `files::preference_rows` hands the named point
/// `lease-rows` to the hand below on the writer thread at every build,
/// wherever the build runs and whatever lock its caller holds. At each
/// build the writer is parked at its start, and the cached walk view,
/// document and snapshot are read on another thread: they must answer
/// before the build goes on. A build under the service lock, the first or
/// a second one, leaves them waiting. The rows of the adopted version are
/// built once.
#[test]
fn lease_rows_are_built_on_the_writer_with_no_service_lock_held_at_every_build() {
    let _state_lease = crate::test_process::state();
    let scratch = IsolatedEnv::enter("t53s4-lease-rows-build");
    let service = open(&scratch);
    let preferences = service.preferences();
    ready(&preferences);

    // From the writer's next job, every build of lease rows on its thread
    // reports here and waits for the release.
    let (entered_tx, entered) = mpsc::channel::<usize>();
    let (release, released) = mpsc::channel::<()>();
    let released = std::sync::Mutex::new(released);
    let builds = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&builds);
    service.set_sql_seams_for_test(sql::Seams {
        point: Some(Arc::new(move |name| {
            if name == "lease-rows" {
                let build = counted.fetch_add(1, Ordering::SeqCst) + 1;
                entered_tx.send(build).unwrap();
                let _ = released.lock().unwrap().recv_timeout(WAIT);
            }
        })),
        ..sql::Seams::default()
    });
    // Another process of the root commits: this service has a version to
    // adopt at its next refresh.
    let peer = open(&scratch);
    save(&peer.preferences(), "peer", json!(1)).unwrap();
    // A stale cache asks for a refresh.
    service.expire_preference_cache_for_test();
    let _ = preferences.walk_globals();

    let mut waited = Vec::new();
    let deadline = Instant::now() + WAIT;
    loop {
        match entered.recv_timeout(Duration::from_millis(20)) {
            Ok(build) => {
                let reader = preferences.clone();
                let (answered_tx, answered) = mpsc::channel();
                let worker = std::thread::spawn(move || {
                    let _ = answered_tx.send((
                        reader.walk_globals().is_ok(),
                        reader.document().is_ok(),
                        reader.snapshot().is_ok(),
                    ));
                });
                match answered.recv_timeout(Duration::from_secs(4)) {
                    Ok((true, true, true)) => {}
                    Ok(read) => {
                        waited.push(format!("build {build}: a cached read failed {read:?}"))
                    }
                    Err(_) => waited.push(format!("build {build}: a cached read waited")),
                }
                // A build under the service lock goes on only now, and
                // the waiting read ends with it.
                release.send(()).unwrap();
                worker.join().unwrap();
            }
            Err(_) => {
                if ready(&preferences).document().get("peer").is_some() {
                    break;
                }
                assert!(
                    Instant::now() < deadline,
                    "lease-rows: the refresh never adopted the other process's commit"
                );
            }
        }
    }
    assert!(
        waited.is_empty(),
        "lease-rows: a cached read waited for the writer inside a build: {waited:?}"
    );
    assert_eq!(
        builds.load(Ordering::SeqCst),
        1,
        "lease-rows: the rows of the adopted version are built once"
    );
}

#[test]
fn a_reopened_database_keeps_its_epoch_counter_moving() {
    let _state_lease = crate::test_process::state();
    let scratch = IsolatedEnv::enter("t53s4-epochs");
    let root = root_of(&scratch);
    let epoch = || -> u64 {
        let raw: Vec<u8> = inspect(&root)
            .query_row("SELECT epoch FROM store_meta WHERE id = 1", [], |row| {
                row.get(0)
            })
            .unwrap();
        sql::decode_u64(&raw).unwrap()
    };
    let first = open(&scratch);
    ready(&first.preferences());
    let one = epoch();
    let second = open(&scratch);
    ready(&second.preferences());
    assert_eq!(epoch(), one + 1, "each opening service draws its own epoch");
}

/// SQLite's own heap statistics: the bytes it holds now and its high-water
/// mark. rusqlite has no safe wrapper for these (plan §9.2; Main's ruling
/// r2 N-L4 permits this FFI under the AGENTS.md `unsafe` rule).
fn sqlite_heap() -> (i64, i64) {
    // SAFETY: the bundled SQLite library is initialized, because every
    // caller opens a connection first; memory-status accounting is on
    // (libsqlite3-sys's bundled build leaves SQLITE_DEFAULT_MEMSTATUS at 1,
    // which the focused test below proves by seeing non-zero use). Both
    // calls take no pointer and only read process-wide counters; the
    // high-water reset flag is 0, so nothing is changed.
    unsafe {
        (
            rusqlite::ffi::sqlite3_memory_used(),
            rusqlite::ffi::sqlite3_memory_highwater(0),
        )
    }
}

/// Reset SQLite's high-water mark to its current use. Only the measuring
/// child process calls it, alone in its process, so no other test can
/// reset the peak it measures.
fn reset_sqlite_highwater() {
    // SAFETY: as for `sqlite_heap`; the reset flag only rewrites SQLite's
    // own process-wide counter and takes no pointer.
    unsafe {
        rusqlite::ffi::sqlite3_memory_highwater(1);
    }
}

/// The focused test of the two statistics calls, alone in its process:
/// accounting is on (use is not zero once SQLite holds pages), use grows
/// while a connection caches a large table and falls when it closes, and
/// the peak covers the current use.
#[test]
#[ignore = "child process of the SQLite heap statistics test"]
fn sqlite_heap_statistics_child() {
    let dir = env_path("T53_DIR");
    let conn = Connection::open(dir.join("heap.sqlite3")).unwrap();
    conn.execute_batch("CREATE TABLE t (b BLOB)").unwrap();
    let (opened, _) = sqlite_heap();
    assert!(opened > 0, "memory-status accounting is on: {opened}");
    for _ in 0..64 {
        conn.execute("INSERT INTO t (b) VALUES (zeroblob(16384))", [])
            .unwrap();
    }
    let _: i64 = conn
        .query_row("SELECT sum(length(b)) FROM t", [], |row| row.get(0))
        .unwrap();
    let (grown, peak) = sqlite_heap();
    assert!(grown > opened, "the page cache grew: {opened} -> {grown}");
    assert!(
        peak >= grown,
        "the peak covers current use: {peak} < {grown}"
    );
    drop(conn);
    let (closed, peak_after) = sqlite_heap();
    assert!(
        closed < grown,
        "closing released memory: {grown} -> {closed}"
    );
    assert!(peak_after >= grown);
}

#[test]
fn sqlite_heap_statistics_see_allocation_and_release() {
    crate::test_process::isolate_state_test!(sqlite_heap_statistics_see_allocation_and_release);
    let scratch = IsolatedEnv::enter("t53s4-heap-stats");
    let status = child(
        "local_state::sql_tests::sqlite_heap_statistics_child",
        &[("T53_DIR", &scratch.home)],
    );
    assert!(status.success(), "{status}");
}

/// The C-heap tripwire (plan §9.2): SQLite's current and high-water use
/// stay at or below 8D for the maximum-state fixture (a D-sized document,
/// eight queued saves and one active), in a process of its own.
#[test]
#[ignore = "child process of the SQLite heap tripwire"]
fn sqlite_heap_tripwire_child() {
    let root = StateRoot::new(env_path("T53_ROOT"));
    let near_d = format!(
        "{{\"bulk\":{{{}}}}}",
        (0..250)
            .map(|n| format!("\"m{n:03}\":\"{}\"", "a".repeat(3_900)))
            .collect::<Vec<_>>()
            .join(",")
    );
    write_private(&root.preferences_file(), near_d.as_bytes());
    let service = LocalStateService::open_process(root.clone());
    let preferences = service.preferences();
    ready(&preferences);
    reset_sqlite_highwater();
    let (release, gate) = mpsc::channel::<()>();
    let gate = std::sync::Mutex::new(gate);
    let first = AtomicBool::new(true);
    service.set_write_stall_for_test(Arc::new(move || {
        if first.swap(false, Ordering::SeqCst) {
            let _ = gate.lock().unwrap().recv();
        }
    }));
    let mut updates = Vec::new();
    for round in 0..9 {
        let mut patch = PreferencePatch::new();
        for member in 0..25 {
            patch
                .set_member(
                    &["bulk", &format!("m{:03}", round * 25 + member)],
                    Some(json!(format!("{round}-{}", "b".repeat(3_800)))),
                )
                .unwrap();
        }
        match preferences.begin_patch(patch).unwrap() {
            Update::Queued(update) => updates.push(update),
            Update::Durable => panic!("queued"),
        }
    }
    release.send(()).unwrap();
    for mut update in updates {
        update.wait(WAIT).unwrap().unwrap();
    }
    let (current, high) = sqlite_heap();
    let bound = 8 * sql::DOCUMENT_MAX as i64;
    println!("T53_SQLITE_HEAP current={current} highwater={high} bound={bound}");
    assert!(
        current <= bound,
        "current SQLite heap {current} > 8D {bound}"
    );
    assert!(high <= bound, "SQLite heap high-water {high} > 8D {bound}");
}

#[test]
fn sqlite_heap_stays_within_eight_documents_at_the_maximum_state() {
    crate::test_process::isolate_state_test!(
        sqlite_heap_stays_within_eight_documents_at_the_maximum_state
    );
    let scratch = IsolatedEnv::enter("t53s4-heap-tripwire");
    let root = root_of(&scratch);
    let output = std::process::Command::new(crate::test_process::executable())
        .args([
            "--ignored",
            "--exact",
            "local_state::sql_tests::sqlite_heap_tripwire_child",
            "--nocapture",
        ])
        .env("T53_ROOT", root.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Every writer job of one service, in order: its class and milliseconds.
struct Timeline(Arc<std::sync::Mutex<Vec<(&'static str, f64)>>>);

impl Timeline {
    /// Open the process service on `root` with the hook watching from the
    /// first hydration.
    fn open_timed(root: &StateRoot) -> (Arc<LocalStateService>, Self) {
        let records: Arc<std::sync::Mutex<Vec<(&'static str, f64)>>> = Arc::default();
        let sink = Arc::clone(&records);
        let service = LocalStateService::open_process_timed_for_test(
            root.clone(),
            Arc::new(move |class, took| {
                sink.lock()
                    .unwrap()
                    .push((class, took.as_secs_f64() * 1000.0));
            }),
        );
        (service, Self(records))
    }
}

impl Timeline {
    fn attach(service: &LocalStateService) -> Self {
        let records: Arc<std::sync::Mutex<Vec<(&'static str, f64)>>> = Arc::default();
        let sink = Arc::clone(&records);
        service.set_job_hook_for_test(Arc::new(move |class, took| {
            sink.lock()
                .unwrap()
                .push((class, took.as_secs_f64() * 1000.0));
        }));
        Self(records)
    }

    fn take(&self) -> Vec<(&'static str, f64)> {
        std::mem::take(&mut *self.0.lock().unwrap())
    }
}

/// A D-sized legacy document (250 members of about 4 KiB).
fn d_document(round: usize) -> Vec<u8> {
    let mut bulk = serde_json::Map::new();
    for member in 0..250 {
        bulk.insert(
            format!("m{member:03}"),
            json!(format!("{round:05}-{}", "d".repeat(3_900))),
        );
    }
    let mut document = serde_json::Map::new();
    document.insert("bulk".into(), serde_json::Value::Object(bulk));
    let bytes = files::preferences_bytes(&document).unwrap();
    assert!(bytes.len() as u64 <= sql::DOCUMENT_MAX);
    bytes
}

/// A save that rewrites 240 members of the D-sized document.
fn wide_patch(salt: usize) -> PreferencePatch {
    let mut patch = PreferencePatch::new();
    for member in 0..240 {
        patch
            .set_member(
                &["bulk", &format!("m{member:03}")],
                Some(json!(format!("{salt:05}-{}", "s".repeat(3_900)))),
            )
            .unwrap();
    }
    patch
}

fn save_wide(preferences: &Preferences, salt: usize) {
    let Update::Queued(mut update) = preferences.begin_patch(wide_patch(salt)).unwrap() else {
        panic!("queued");
    };
    update.wait(WAIT).expect("settles").unwrap();
}

fn phase_root(scratch: &IsolatedEnv, name: &str) -> StateRoot {
    StateRoot::new(scratch.home.join(name).join(".274bot"))
}

fn file_under(classes: &mut occupancy_tests::Samples, label: &str, values: Vec<f64>) {
    classes.entry(label.to_string()).or_default().extend(values);
}
// Operator-approved gate: per-class p99 <= 250 ms and max <= 500 ms. The p99
// bounds recurring latency; the hard ceiling permits a transient but rejects
// severe stalls. First-import stays diagnostic because it runs once per root
// off the UI thread.
const OCCUPANCY_P99_LIMIT_MS: f64 = occupancy_tests::THRESHOLD_MS;
const OCCUPANCY_HARD_CEILING_MS: f64 = 500.0;

fn occupancy_class_verdict(p99_ms: f64, max_ms: f64, diagnostic: bool) -> &'static str {
    if diagnostic {
        "diagnostic"
    } else if p99_ms <= OCCUPANCY_P99_LIMIT_MS && max_ms <= OCCUPANCY_HARD_CEILING_MS {
        "pass"
    } else {
        "fail"
    }
}

#[test]
fn occupancy_class_verdict_uses_p99_and_hard_ceiling() {
    assert_eq!(
        occupancy_class_verdict(OCCUPANCY_P99_LIMIT_MS, OCCUPANCY_HARD_CEILING_MS, false),
        "pass"
    );
    assert_eq!(
        occupancy_class_verdict(
            OCCUPANCY_P99_LIMIT_MS + 0.001,
            OCCUPANCY_HARD_CEILING_MS,
            false
        ),
        "fail"
    );
    assert_eq!(
        occupancy_class_verdict(
            OCCUPANCY_P99_LIMIT_MS,
            OCCUPANCY_HARD_CEILING_MS + 0.001,
            false
        ),
        "fail"
    );
    assert_eq!(
        occupancy_class_verdict(
            OCCUPANCY_HARD_CEILING_MS + 1.0,
            OCCUPANCY_HARD_CEILING_MS + 1.0,
            true
        ),
        "diagnostic"
    );
}

/// A11's CI microbenchmark entrypoint runs the local-state SQL occupancy gate
/// in a locked release build and preserves its JSON report. The gate measures
/// every SQL job class through typed-view publication under the §4.4 load:
/// two OS processes on one root, each with eight queued and one active D-sized
/// preference save in every round, with the bank exit save, the Start loadout
/// readback and a bank load queued beside them. The classes that run once
/// (first import, quarantine, hard-maximum WAL recovery on open, uncertain
/// COMMIT, rollbacks, connection setup, shutdown close, same-root export and
/// recovery) run on roots of their own while the load runs, except the
/// maintenance commands, which need the exclusive root and run after it.
/// Writes `T53_OCCUPANCY_OUT` (JSON) with per-class max, p99, over_limit
/// count and verdict; when `T53_JSON_BASELINE` is set, it also reports matching
/// jobs. A gating class fails when its p99 exceeds 250 ms or its max exceeds
/// the 500 ms hard ceiling. The load must also be whole.
#[test]
#[ignore = "the S4 occupancy gate; run alone with T53_OCCUPANCY_OUT (and T53_JSON_BASELINE)"]
fn occupancy_gate_every_sql_job_class() {
    // First-import jobs run once per state root off the UI thread, so they are diagnostic only.
    const FIRST_IMPORT_DIAGNOSTIC_CLASSES: &[&str] =
        &["first-import/file-readback", "first-import/pref-readback"];
    let out = env_path("T53_OCCUPANCY_OUT");
    let scratch = IsolatedEnv::enter("t53s4-occupancy-gate");
    let load =
        occupancy_tests::start_load(&root_of(&scratch), &scratch.home, occupancy_tests::ROUNDS);
    load.go();
    let mut classes = occupancy_tests::Samples::new();
    let mut fault_jobs = Vec::new();

    // First import of a D-sized legacy file (open, schema, epoch, import,
    // `.migrated` rename and directory sync, typed view) and the close.
    let mut maintenance_roots = Vec::new();
    for round in 0..3 {
        let root = phase_root(&scratch, &format!("import-{round}"));
        write_private(&root.preferences_file(), &d_document(round));
        let (service, timeline) = Timeline::open_timed(&root);
        ready(&service.preferences());
        for (class, took) in timeline.take() {
            file_under(&mut classes, &format!("first-import/{class}"), vec![took]);
        }
        let closing = Instant::now();
        drop(service);
        file_under(
            &mut classes,
            "shutdown-close",
            vec![closing.elapsed().as_secs_f64() * 1000.0],
        );
        maintenance_roots.push(root);
    }

    // A malformed D-sized source: verified quarantine copy, sync, defaults.
    for round in 0..3 {
        let root = phase_root(&scratch, &format!("quarantine-{round}"));
        let mut damaged = b"{ damaged ".to_vec();
        damaged.extend(std::iter::repeat_n(b'x', 900_000));
        write_private(&root.preferences_file(), &damaged);
        let (service, timeline) = Timeline::open_timed(&root);
        ready(&service.preferences());
        for (class, took) in timeline.take() {
            file_under(
                &mut classes,
                &format!("malformed-quarantine/{class}"),
                vec![took],
            );
        }
    }

    // Steady commits (the first after the open, then the first after an
    // idle reset), the idle TRUNCATE, and D-sized refreshes after the
    // other service's commits.
    {
        let root = phase_root(&scratch, "steady");
        write_private(&root.preferences_file(), &d_document(0));
        let writer = LocalStateService::open_process(root.clone());
        let reader = LocalStateService::open_process(root.clone());
        let (writes, reads) = (Timeline::attach(&writer), Timeline::attach(&reader));
        let (wp, rp) = (writer.preferences(), reader.preferences());
        ready(&wp);
        ready(&rp);
        writes.take();
        reads.take();
        let mut after_reset = false;
        let mut first = true;
        for step in 0..24 {
            save_wide(&wp, step + 1);
            for (class, took) in writes.take() {
                let label = match class {
                    "pref-save" if first => "steady/commit-first",
                    "pref-save" if after_reset => "steady/commit-after-reset",
                    "pref-save" => "steady/commit-steady",
                    "wal-truncate" => "steady/idle-truncate",
                    other => other,
                };
                after_reset = class == "wal-truncate";
                first = false;
                file_under(&mut classes, label, vec![took]);
            }
            if step % 4 == 3 {
                // The other process follows: its one-second freshness
                // demand starts a refresh (read, parse, typed view).
                std::thread::sleep(Duration::from_millis(1_100));
                let seen = rp.snapshot().map(|snapshot| snapshot.version());
                let watch = Instant::now();
                while rp.snapshot().map(|snapshot| snapshot.version()) == seen
                    && watch.elapsed() < Duration::from_secs(3)
                {
                    std::thread::sleep(Duration::from_millis(20));
                }
                for (class, took) in reads.take() {
                    assert_eq!(class, "pref-readback");
                    file_under(&mut classes, "refresh-d-sized", vec![took]);
                }
            }
        }
        // A pinned reader keeps every TRUNCATE busy: its partial copy work.
        let probe = inspect(&root);
        let pinned = pin_reader(&root);
        let mut step = 100;
        while writer.truncate_counts_for_test().1 == 0 {
            save_wide(&wp, step);
            step += 1;
            assert!(step < 140, "the WAL never reached the TRUNCATE trigger");
        }
        std::thread::sleep(Duration::from_millis(300));
        for (class, took) in writes.take() {
            if class == "wal-truncate" {
                file_under(&mut classes, "truncate-busy", vec![took]);
            }
        }
        pinned.execute_batch("COMMIT").unwrap();
        drop(pinned);
        let deadline = Instant::now() + WAIT;
        while wal_frames(&probe) != 0 {
            assert!(Instant::now() < deadline, "the idle reset never ran");
            std::thread::sleep(Duration::from_millis(10));
        }
        for (class, took) in writes.take() {
            if class == "wal-truncate" {
                file_under(&mut classes, "steady/idle-truncate", vec![took]);
            }
        }
    }

    // Rollback, uncertain-COMMIT resolution with its reconnect, a declined
    // edit, and an uncertain COMMIT that did not land. Every fault case
    // changes the document, proves that its own COMMIT branch ran and what
    // it left, and only then gives its job as a timing sample.
    {
        let root = phase_root(&scratch, "faults");
        write_private(&root.preferences_file(), &d_document(1));
        let service = LocalStateService::open_process(root.clone());
        let preferences = service.preferences();
        ready(&preferences);
        save_wide(&preferences, 1);
        let jobs = commit_fault_jobs(&service, &root);
        for job in &jobs {
            file_under(&mut classes, job.label, vec![job.took_ms]);
        }
        fault_jobs.extend(jobs.iter().map(FaultJob::report));
        let timeline = Timeline::attach(&service);
        let identity = preferences.identity().clone();
        let stale = StoreVersion::INITIAL;
        let request = accepted(
            service.submit(DomainEdit::Preferences(
                PreferencesEdit::batch(
                    identity,
                    Some(stale),
                    vec![PreferenceChange::set("owner", "\"x\"").unwrap()],
                )
                .unwrap(),
            )),
        );
        assert!(matches!(
            settled(&service, request, WAIT),
            Some(Settlement::Failed(ErrorCode::Conflict { .. }))
        ));
        for (class, took) in timeline.take() {
            if class == "pref-save" {
                file_under(&mut classes, "rollback-declined", vec![took]);
            }
        }
    }

    // Hard-maximum WAL recovery on open: a pre-existing WAL past both
    // maxima, a pinned reader, then its release (the idle TRUNCATE copies
    // the whole log) and the open that follows.
    {
        let root = phase_root(&scratch, "wal-recovery");
        write_private(&root.preferences_file(), &d_document(2));
        {
            let service = LocalStateService::open_process(root.clone());
            ready(&service.preferences());
        }
        let probe = inspect(&root);
        let reader = pin_reader(&root);
        fill_wal_spread(&root, sql::WAL_MAX_FRAMES + 100, 700);
        let (service, timeline) = Timeline::open_timed(&root);
        let preferences = service.preferences();
        let deadline = Instant::now() + WAIT;
        while preferences.wait_cause().is_none() {
            assert!(Instant::now() < deadline, "no WAL wait shown");
            std::thread::sleep(Duration::from_millis(5));
        }
        std::thread::sleep(Duration::from_millis(300));
        for (class, took) in timeline.take() {
            file_under(
                &mut classes,
                &format!("wal-recovery/{class}-while-waiting"),
                vec![took],
            );
        }
        reader.execute_batch("COMMIT").unwrap();
        drop(reader);
        ready(&preferences);
        for (class, took) in timeline.take() {
            file_under(
                &mut classes,
                &format!("wal-recovery/{class}-after-release"),
                vec![took],
            );
        }
        assert!(wal_frames(&probe) < 100, "the reset ran");
    }

    // Connection open and setup on a populated database.
    {
        let root = &maintenance_roots[0];
        for _ in 0..10 {
            let started = Instant::now();
            let conn = sql::open_connection(root).unwrap_or_else(|_| panic!("open"));
            file_under(
                &mut classes,
                "connection-open-setup",
                vec![started.elapsed().as_secs_f64() * 1000.0],
            );
            drop(conn);
        }
    }

    // The load, then the maintenance commands (exclusive root).
    let report = load.finish();
    report.assert_whole();
    for (class, values) in &report.samples {
        file_under(&mut classes, &format!("load/{class}"), values.clone());
    }
    let source = scratch.home.join("recovery-source.json");
    write_private(&source, &d_document(9));
    for root in &maintenance_roots {
        let started = Instant::now();
        run_preferences_maintenance(root, Frontend::Panel, &MaintenanceAction::ExportSameRoot)
            .unwrap();
        file_under(
            &mut classes,
            "export-same-root",
            vec![started.elapsed().as_secs_f64() * 1000.0],
        );
        let started = Instant::now();
        run_preferences_maintenance(
            root,
            Frontend::Panel,
            &MaintenanceAction::RecoverSource(source.clone()),
        )
        .unwrap();
        file_under(
            &mut classes,
            "recover-source",
            vec![started.elapsed().as_secs_f64() * 1000.0],
        );
        let started = Instant::now();
        run_preferences_maintenance(root, Frontend::Panel, &MaintenanceAction::RecoverDefaults)
            .unwrap();
        file_under(
            &mut classes,
            "recover-defaults",
            vec![started.elapsed().as_secs_f64() * 1000.0],
        );
    }

    let mut missing: Vec<&str> = [
        "load/pref-save",
        "load/pref-readback",
        "load/wal-truncate",
        "load/file-write",
        "load/file-readback",
        "load/bank-exit-save-residence",
        "load/start-loadout-readback-residence",
        "load/bank-load-residence",
        "load/service-close-shutdown",
        "malformed-quarantine/pref-readback",
        "steady/commit-first",
        "steady/commit-steady",
        "steady/commit-after-reset",
        "steady/idle-truncate",
        "refresh-d-sized",
        "truncate-busy",
        "rollback-declined",
        "rollback-full",
        "rollback-busy-requeue",
        "uncertain-commit-resolved",
        "uncertain-commit-unresolved",
        "wal-recovery/pref-readback-while-waiting",
        "wal-recovery/wal-truncate-after-release",
        "wal-recovery/pref-readback-after-release",
        "connection-open-setup",
        "shutdown-close",
        "export-same-root",
        "recover-source",
        "recover-defaults",
    ]
    .into_iter()
    .filter(|required| {
        !classes
            .get(*required)
            .is_some_and(|values| !values.is_empty())
    })
    .collect();
    missing.extend(
        FIRST_IMPORT_DIAGNOSTIC_CLASSES
            .iter()
            .copied()
            .filter(|required| {
                !classes
                    .get(*required)
                    .is_some_and(|values| !values.is_empty())
            }),
    );

    let worst = occupancy_tests::worst_ms(&classes);
    let mut gating_worst = 0.0_f64;
    let mut gating_worst_p99 = 0.0_f64;
    let mut gate_failures = Vec::new();
    let mut class_summaries = serde_json::Map::new();
    for (class, values) in &classes {
        let mut class_summary = occupancy_tests::summary(values);
        let max_ms = class_summary
            .get("max_ms")
            .and_then(serde_json::Value::as_f64)
            .expect("class summary contains max_ms");
        let p99_ms = class_summary
            .get("p99_ms")
            .and_then(serde_json::Value::as_f64)
            .expect("class summary contains p99_ms");
        let diagnostic = FIRST_IMPORT_DIAGNOSTIC_CLASSES.contains(&class.as_str());
        let verdict = occupancy_class_verdict(p99_ms, max_ms, diagnostic);
        if !diagnostic {
            gating_worst = gating_worst.max(max_ms);
            gating_worst_p99 = gating_worst_p99.max(p99_ms);
            if verdict == "fail" {
                gate_failures.push(class.clone());
            }
        }
        class_summary
            .as_object_mut()
            .expect("class summary is an object")
            .insert("verdict".to_string(), json!(verdict));
        class_summaries.insert(class.clone(), class_summary);
    }
    let mut comparison = serde_json::Map::new();
    if let Some(path) = std::env::var_os("T53_JSON_BASELINE") {
        let baseline: serde_json::Value =
            serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        for class in [
            "pref-save",
            "pref-readback",
            "file-write",
            "file-readback",
            "bank-exit-save-residence",
            "start-loadout-readback-residence",
            "bank-load-residence",
            "service-close-shutdown",
        ] {
            let json_build = &baseline["classes"][class];
            if json_build.is_null() {
                continue;
            }
            comparison.insert(
                class.to_string(),
                json!({
                    "sql": occupancy_tests::summary(&classes[&format!("load/{class}")]),
                    "json": json_build,
                }),
            );
        }
    }
    let report = json!({
        "threshold_ms": OCCUPANCY_P99_LIMIT_MS,
        "hard_ceiling_ms": OCCUPANCY_HARD_CEILING_MS,
        "worst_class_max_ms": worst,
        "gating_worst_class_max_ms": gating_worst,
        "gating_worst_class_p99_ms": gating_worst_p99,
        "os": std::env::consts::OS,
        "load": {
            "rounds": report.rounds,
            "accepted": report.accepted,
            "saves_ok": report.saves_ok,
            "occupancy_rounds": report.occupancy_rounds,
        },
        "missing_classes": missing,
        "fault_jobs": fault_jobs,
        "gating_failures": gate_failures,
        "classes": class_summaries,
        "diagnostic_classes": {
            "reason": "First import runs once per state root off the UI thread.",
            "names": FIRST_IMPORT_DIAGNOSTIC_CLASSES,
        },
        "matching_json_build_jobs": comparison,
    });
    std::fs::write(&out, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    assert!(missing.is_empty(), "no sample of the classes {missing:?}");
    assert!(
        gate_failures.is_empty(),
        "gating SQL job class exceeded p99 <= {OCCUPANCY_P99_LIMIT_MS} ms or max <= {OCCUPANCY_HARD_CEILING_MS} ms: {report}"
    );
}

/// One job order of a fault case: the branch event a COMMIT fault arm
/// reported, and the writer jobs that followed it.
#[derive(Debug, Clone, PartialEq)]
enum Trace {
    Event(&'static str),
    Job(&'static str, f64),
}

/// One accepted fault sample of the occupancy gate: the whole job with the
/// proof that its COMMIT branch ran and what it left behind.
struct FaultJob {
    label: &'static str,
    fault: sql::CommitFault,
    /// The branch event the writer reported for this case.
    event: &'static str,
    /// The `pref-save` job that ran the branch.
    took_ms: f64,
    /// Every `pref-save` job of the request (two when BUSY requeues it).
    save_jobs: usize,
    head_before: u64,
    head_after: u64,
    audits: usize,
    terminals: usize,
    /// The worker connection after each commit attempt: back in
    /// autocommit, every setting read back.
    connection: Vec<Option<(bool, bool)>>,
}

impl FaultJob {
    fn report(&self) -> serde_json::Value {
        json!({
            "label": self.label,
            "fault": format!("{:?}", self.fault),
            "branch_event": self.event,
            "job_class": "pref-save",
            "job_ms": self.took_ms,
            "save_jobs": self.save_jobs,
            "head_before": self.head_before,
            "head_after": self.head_after,
            "audit_lines": self.audits,
            "terminal_lines": self.terminals,
            "worker_connection": format!("{:?}", self.connection),
        })
    }
}

/// The four COMMIT fault cases of the occupancy gate, each on a document
/// of its own that differs from the stored one: its COMMIT fault arm must
/// report its event, and the request's outcome, durable head, cache, audit
/// count and worker connection must be what that arm leaves (plan §4.5),
/// before the job it ran is returned as a timing sample. The service holds
/// a document and no queued work.
fn commit_fault_jobs(service: &Arc<LocalStateService>, root: &StateRoot) -> Vec<FaultJob> {
    type Probe = std::sync::Mutex<Vec<Option<(bool, bool)>>>;
    let preferences = service.preferences();
    let audits = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&audits);
    service.set_audit_hook(Arc::new(move |_| {
        counted.fetch_add(1, Ordering::SeqCst);
    }));
    let terminals = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&terminals);
    service.set_terminal_hook(Arc::new(move |_| {
        counted.fetch_add(1, Ordering::SeqCst);
    }));
    let trace: Arc<std::sync::Mutex<Vec<Trace>>> = Arc::default();
    let jobs = Arc::clone(&trace);
    service.set_job_hook_for_test(Arc::new(move |class, took| {
        jobs.lock()
            .unwrap()
            .push(Trace::Job(class, took.as_secs_f64() * 1000.0));
    }));
    let states: Arc<Probe> = Arc::default();
    // (fault, label, the branch's event, the save ends durable, `pref-save`
    // jobs of the request, a fragment of the failure text)
    let cases = [
        (
            sql::CommitFault::IoErrAfterCommit,
            "uncertain-commit-resolved",
            "commit-uncertain-found",
            true,
            1,
            "",
        ),
        (
            sql::CommitFault::IoErrWithoutCommit,
            "uncertain-commit-unresolved",
            "commit-uncertain-lost",
            false,
            1,
            "outcome is unknown",
        ),
        (
            sql::CommitFault::Full,
            "rollback-full",
            "commit-refused-rolled-back",
            false,
            1,
            "refused the commit",
        ),
        (
            sql::CommitFault::Busy,
            "rollback-busy-requeue",
            "commit-busy-rolled-back",
            true,
            2,
            "",
        ),
    ];
    let mut accepted = Vec::new();
    for (case, (fault, label, event, durable, save_jobs, fragment)) in cases.into_iter().enumerate()
    {
        let salt = 100 + case;
        let value = json!(format!("{salt:05}-{}", "s".repeat(3_900)));
        let before = stored_document(root);
        assert_ne!(
            before["bulk"]["m000"], value,
            "{label}: the case's document differs from the stored one"
        );
        let cached = ready(&preferences).document().get("bulk").cloned();
        let (head_before, audits_before, terminals_before) = (
            head(root),
            audits.load(Ordering::SeqCst),
            terminals.load(Ordering::SeqCst),
        );
        trace.lock().unwrap().clear();
        states.lock().unwrap().clear();
        let once = Arc::new(std::sync::Mutex::new(Some(fault)));
        let fault_taken = Arc::clone(&once);
        let events = Arc::clone(&trace);
        let probe = Arc::clone(&states);
        service.set_sql_seams_for_test(sql::Seams {
            commit_fault: Some(Arc::new(move || once.lock().unwrap().take())),
            point: Some(Arc::new(move |name| {
                if name.starts_with("commit-") {
                    events.lock().unwrap().push(Trace::Event(name));
                }
            })),
            after_commit: Some(Arc::new(move |state| probe.lock().unwrap().push(state))),
            ..sql::Seams::default()
        });
        let Update::Queued(mut update) = preferences.begin_patch(wide_patch(salt)).unwrap() else {
            panic!("queued");
        };
        let request = update.request_id();
        let result = update.wait(WAIT).expect("settles");
        // The job hook runs after the job's own settlement: wait for it.
        let deadline = Instant::now() + WAIT;
        let saves = loop {
            let saves = trace
                .lock()
                .unwrap()
                .iter()
                .filter(|entry| matches!(entry, Trace::Job("pref-save", _)))
                .count();
            if saves >= save_jobs || Instant::now() >= deadline {
                break saves;
            }
            std::thread::sleep(Duration::from_millis(2));
        };
        let trace_now = trace.lock().unwrap().clone();
        assert!(
            fault_taken.lock().unwrap().is_none(),
            "{label}: the fault was drawn by a commit"
        );
        assert_eq!(
            saves, save_jobs,
            "{label}: the request's save jobs ({trace_now:?})"
        );
        let events: Vec<_> = trace_now
            .iter()
            .filter_map(|entry| match entry {
                Trace::Event(name) => Some(*name),
                Trace::Job(..) => None,
            })
            .collect();
        assert_eq!(
            events,
            [event],
            "{label}: exactly its own COMMIT branch ran ({trace_now:?})"
        );
        let at = trace_now
            .iter()
            .position(|entry| *entry == Trace::Event(event))
            .unwrap();
        let Some(Trace::Job("pref-save", took_ms)) = trace_now.get(at + 1).cloned() else {
            panic!("{label}: the branch's job follows its event ({trace_now:?})");
        };
        let connection = states.lock().unwrap().clone();
        assert!(
            !connection.is_empty() && connection.iter().all(|state| *state == Some((true, true))),
            "{label}: the worker's connection is back in autocommit with every setting read \
             back ({connection:?})"
        );
        let snapshot = ready(&preferences);
        let (head_after, audit_lines, terminal_lines) = (
            head(root),
            audits.load(Ordering::SeqCst) - audits_before,
            terminals.load(Ordering::SeqCst) - terminals_before,
        );
        if durable {
            result.unwrap_or_else(|error| panic!("{label}: {error}"));
            assert!(
                matches!(
                    service.request_state(request),
                    RequestState::Settled(Settlement::Completed(_))
                ),
                "{label}"
            );
            assert_eq!(head_after, head_before + 1, "{label}: one durable commit");
            assert_eq!(stored_document(root)["bulk"]["m000"], value, "{label}");
            assert_eq!(
                snapshot.document()["bulk"]["m000"],
                value,
                "{label}: the cache"
            );
            assert_eq!((audit_lines, terminal_lines), (1, 0), "{label}: one audit");
        } else {
            let failure = result.unwrap_err();
            let error = failure.to_string();
            assert!(error.contains(fragment), "{label}: {error}");
            assert_eq!(
                failure.outcome_unknown(),
                event == "commit-uncertain-lost",
                "{label}: only the unresolved COMMIT is flagged unknown"
            );
            assert!(
                matches!(
                    service.request_state(request),
                    RequestState::Settled(Settlement::Failed(_))
                ),
                "{label}"
            );
            assert_eq!(head_after, head_before, "{label}: nothing durable");
            assert_eq!(stored_document(root), before, "{label}");
            assert_eq!(
                snapshot.document().get("bulk").cloned(),
                cached,
                "{label}: the cache keeps the durable document"
            );
            assert_eq!(
                (audit_lines, terminal_lines),
                (0, 1),
                "{label}: no audit and one terminal line"
            );
        }
        accepted.push(FaultJob {
            label,
            fault,
            event,
            took_ms,
            save_jobs: saves,
            head_before,
            head_after,
            audits: audit_lines,
            terminals: terminal_lines,
            connection,
        });
    }
    service.set_sql_seams_for_test(sql::Seams::default());
    accepted
}

/// The gate's fault cases without its load: each one changes the document,
/// runs its own COMMIT branch, leaves its own outcome, and only then yields
/// a timing sample (review R2-3). The body runs in an ignored child of its
/// own so each request's save jobs are its own and no unrelated `fork` of
/// the suite copies the guard it must take (review R3-1).
#[test]
fn the_gate_fault_cases_each_run_their_own_commit_branch_on_a_changed_document() {
    isolated_child("gate_fault_cases_child", &[]);
}

/// Child of the test above; creates its root after `exec`.
#[test]
#[ignore = "child process of the gate fault case test"]
fn gate_fault_cases_child() {
    let scratch = IsolatedEnv::enter("t53s4-gate-faults");
    let root = root_of(&scratch);
    write_private(&root.preferences_file(), &d_document(1));
    let service = LocalStateService::open_process(root.clone());
    ready(&service.preferences());
    save_wide(&service.preferences(), 1);
    let jobs = commit_fault_jobs(&service, &root);
    let seen: Vec<_> = jobs
        .iter()
        .map(|job| (job.label, job.event, job.save_jobs))
        .collect();
    assert_eq!(
        seen,
        [
            ("uncertain-commit-resolved", "commit-uncertain-found", 1),
            ("uncertain-commit-unresolved", "commit-uncertain-lost", 1),
            ("rollback-full", "commit-refused-rolled-back", 1),
            ("rollback-busy-requeue", "commit-busy-rolled-back", 2),
        ]
    );
    for job in &jobs {
        assert!(job.took_ms > 0.0, "{}", job.label);
    }
}

/// The steady-commit sync load for the strace count (plan §4.5): open, then
/// `T53_COMMITS` commits of a quarter-D document, so the WAL crosses W / 2
/// and one idle TRUNCATE runs. The driver counts fsync/fdatasync calls.
#[test]
#[ignore = "sync-count load; run alone under strace with T53_ROOT and T53_COMMITS"]
fn sync_count_steady_commits_load() {
    let root = StateRoot::new(env_path("T53_ROOT"));
    let commits: usize = std::env::var("T53_COMMITS").unwrap().parse().unwrap();
    let service = LocalStateService::open_process(root);
    let preferences = service.preferences();
    ready(&preferences);
    for n in 0..commits {
        let mut patch = PreferencePatch::new();
        for member in 0..64 {
            patch
                .set_member(
                    &["bulk", &format!("m{member:03}")],
                    Some(json!(format!("{n:05}-{}", "s".repeat(3_900)))),
                )
                .unwrap();
        }
        let Update::Queued(mut update) = preferences.begin_patch(patch).unwrap() else {
            panic!("queued");
        };
        update.wait(WAIT).unwrap().unwrap();
    }
    let (resets, busy, _) = service.truncate_counts_for_test();
    println!("T53_SYNC_LOAD commits={commits} truncates={resets} busy={busy}");
}
