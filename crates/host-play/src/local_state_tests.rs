//! State fixtures hold the test-only lease through service and root teardown.
//! Mixed process cases keep that lease in the parent until the exact child exits.

use super::*;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc};
use std::time::Duration;

use api::identity::{Cancellation, DurableTimestamp, RequestId, Settlement};
use api::loadout::Loadout;
use api::local_state::{
    AccountKey, AuditCommit, AuditOrigin, BankHint, CommitAction, ContentScope, DomainEdit,
    DomainIdentity, ErrorCode, FailureReason, LoadoutEdit, LocalStateDomain, PreferencesEdit,
    RuntimeSettingsEdit, StoreVersion, SubmitReceipt, ValidationReason,
};
use script::IsolatedEnv;

const PROFILE: &str = "local-289";
const CONTENT: &str = "test-content";
const WAIT: Duration = Duration::from_secs(60);

fn service(scratch: &IsolatedEnv, domains: &[LocalStateDomain]) -> Arc<LocalStateService> {
    LocalStateService::open(
        StateRoot::new(scratch.home.join(".274bot")),
        domains,
        Arc::new(RequestIdMinter::new()),
    )
}

fn scoped(domain: LocalStateDomain, account: &str, content: &str) -> DomainIdentity {
    DomainIdentity::new(
        domain,
        AccountKey::try_new(PROFILE, account).unwrap(),
        ContentScope::try_new(client::io::ClientRevision::R289, content).unwrap(),
    )
}

fn identity(domain: LocalStateDomain, account: &str) -> DomainIdentity {
    scoped(domain, account, CONTENT)
}

fn bank_identity(account: &str) -> DomainIdentity {
    identity(LocalStateDomain::BankHints, account)
}

fn loadouts_identity() -> DomainIdentity {
    identity(LocalStateDomain::LoadoutsReferences, "alice")
}

fn request(receipt: SubmitReceipt) -> RequestId {
    match receipt {
        SubmitReceipt::Accepted { request } => request,
        SubmitReceipt::Refused(code) => panic!("submission refused: {code}"),
    }
}

/// `request`'s settlement, waiting for it.
fn settlement(service: &LocalStateService, request: RequestId) -> WriteSettlement {
    settled_within(service, request, WAIT)
        .unwrap_or_else(|| panic!("request {} did not settle", request.get()))
}

/// `request`'s settlement if it lands within `wait`.
fn settled_within(
    service: &LocalStateService,
    request: RequestId,
    wait: Duration,
) -> Option<WriteSettlement> {
    match service.wait_request(request, wait) {
        RequestState::Settled(settlement) => Some(settlement),
        RequestState::Pending | RequestState::Forgotten => None,
    }
}

fn completed(service: &LocalStateService, request: RequestId) -> DurableReceipt {
    match settlement(service, request) {
        Settlement::Completed(receipt) => receipt,
        other => panic!("request {} did not complete: {other:?}", request.get()),
    }
}

/// Retry an attempt while `transient` says its failure is one of the two
/// bounded waits this suite retries: the typed `NotReady` outcome (the
/// store was still hydrating) or the write staying queued past the
/// frontend's own bound. Both settle later under machine load (a product
/// deadline this suite must not change). Any other failure returns at
/// once, so refusals and genuine conflicts keep their fast verdict. The
/// bound is the suite's 60 s, as before.
fn patiently<E>(
    mut attempt: impl FnMut() -> Result<(), E>,
    transient: impl Fn(&E) -> bool,
) -> Result<(), E> {
    let start = std::time::Instant::now();
    loop {
        match attempt() {
            Ok(()) => return Ok(()),
            Err(error) if transient(&error) && start.elapsed() < Duration::from_secs(60) => {
                std::thread::yield_now();
            }
            Err(error) => return Err(error),
        }
    }
}
/// A preferences `set`, retrying the two transient waits: a load-slow
/// durability wait, or a read queued while the store still hydrates.
fn set_patiently(preferences: &Preferences, key: &str, value: serde_json::Value) {
    patiently(|| preferences.set(key, value.clone()), transient_wait).unwrap();
}
/// A preferences `update`, retrying the two transient waits. A write of an
/// earlier attempt that lands late is merged over, not duplicated: the
/// update re-reads and re-applies `merge` on a conflict.
fn update_patiently(
    preferences: &Preferences,
    mut merge: impl FnMut(&mut serde_json::Map<String, serde_json::Value>) -> std::io::Result<()>,
) {
    patiently(
        || preferences.update(|document| merge(document)),
        transient_wait,
    )
    .unwrap();
}
/// A loadouts-store wait this suite retries. The store's save seam reports
/// text, not a typed code (`script::LoadoutsStore::save` returns
/// `Result<(), String>`), so this reads the two wait messages the frontend
/// writes: the write stayed queued, or the domain was still hydrating.
fn transient_save_wait(error: &str) -> bool {
    error.contains("not durable within") || error.contains("is not ready")
}
/// A loadouts form `save`, retrying a load-slow durability wait. A write
/// of an earlier attempt that lands late conflicts with its own retry;
/// the save replays the store's latest on a conflict, so a retry converges
/// instead of duplicating rows.
fn save_patiently(form: &mut script::LoadoutsStore) {
    patiently(
        || form.save(),
        |error| transient_save_wait(error) || error.contains("changed since this list was loaded"),
    )
    .unwrap_or_else(|error| panic!("save failed: {error}"));
}

/// An attempt outcome for the retry-helper test: only the wait is
/// transient.
#[derive(Debug, PartialEq, Eq)]
enum TestAttempt {
    Wait,
    Refused,
}

/// The shared retry both ways: a typed `NotReady` attempt is retried and
/// the next attempt succeeds; a permanent failure returns after one
/// attempt. A service with the preferences domain closed reports the same
/// typed `NotReady` the fixture's Reading-phase wait reports, without a
/// five-second frontend wait.
#[test]
fn the_retry_helper_retries_not_ready_and_returns_a_permanent_failure() {
    let scratch = IsolatedEnv::enter("local-state-retry-outcomes");
    let closed = service(&scratch, &[]);
    let not_ready = closed.preferences();
    let attempts = AtomicUsize::new(0);
    let result = patiently(
        || {
            if attempts.fetch_add(1, Ordering::SeqCst) == 0 {
                not_ready.set("map_bake", serde_json::json!("always"))
            } else {
                Ok(())
            }
        },
        transient_wait,
    );
    assert!(result.is_ok(), "a NotReady attempt is retried");
    assert_eq!(attempts.load(Ordering::SeqCst), 2);
    let error = not_ready
        .set("map_bake", serde_json::json!("always"))
        .expect_err("a closed domain refuses the write");
    assert_eq!(
        wait_outcome(&error),
        WaitOutcome::NotReady,
        "the typed Reading-phase outcome: {error}"
    );

    let attempts = AtomicUsize::new(0);
    let result: Result<(), TestAttempt> = patiently(
        || {
            attempts.fetch_add(1, Ordering::SeqCst);
            Err(TestAttempt::Refused)
        },
        |error| *error == TestAttempt::Wait,
    );
    assert_eq!(result.unwrap_err(), TestAttempt::Refused);
    assert_eq!(
        attempts.load(Ordering::SeqCst),
        1,
        "a refusal is not retried"
    );

    // An error the fixture did not mint is not a wait.
    let terminal = std::io::Error::new(std::io::ErrorKind::InvalidData, "unreadable rows");
    assert_eq!(wait_outcome(&terminal), WaitOutcome::Terminal);
    assert!(!transient_wait(&terminal), "an unminted error is terminal");
}

/// How many stores the service holds.
fn held_stores(service: &LocalStateService) -> usize {
    service.shared.state.lock().stores.len()
}

fn hint_edit(
    identity: &DomainIdentity,
    expected: Option<StoreVersion>,
    rows: Vec<(i32, i32)>,
    observed_at: u64,
) -> DomainEdit {
    DomainEdit::Bank(
        BankHint::try_new(
            identity.clone(),
            expected,
            rows,
            DurableTimestamp::from_unix_seconds(observed_at),
        )
        .unwrap(),
    )
}

fn loadout_edit(expected: Option<StoreVersion>, loadout: Loadout) -> DomainEdit {
    DomainEdit::Loadout(LoadoutEdit::upsert(loadouts_identity(), expected, loadout).unwrap())
}

fn hint_document(
    profile: &str,
    account: &str,
    schema_version: u32,
    observed_at_unix: u64,
    rows: &[(i32, i32)],
) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "schema_version": schema_version,
        "profile": profile,
        "account": account,
        "revision": 289,
        "content": CONTENT,
        "observed_at_unix": observed_at_unix,
        "rows": rows,
    }))
    .unwrap()
}

fn write_private(path: &std::path::Path, bytes: &[u8]) {
    vault::write_private_file(path, bytes).unwrap();
}

/// `rows` written to the loadouts file by another writer (an older build's
/// form, a hand edit), in the pretty array every build writes.
fn write_loadouts(path: &std::path::Path, rows: &[Loadout]) {
    write_private(path, serde_json::to_string_pretty(rows).unwrap().as_bytes());
}

/// A frontend's loadouts form over `service`.
fn form(service: &Arc<LocalStateService>) -> script::LoadoutsStore {
    script::LoadoutsStore::over(Arc::new(service.operator_loadouts()))
}

/// A writer held inside the publication of `blocker` until released, so
/// later submissions queue behind it.
struct Stall {
    entered: mpsc::Receiver<()>,
    release: mpsc::Sender<()>,
}

fn stall_on(service: &LocalStateService, blocker: std::path::PathBuf) -> Stall {
    let (entered_tx, entered) = mpsc::channel();
    let (release, release_rx) = mpsc::channel::<()>();
    let release_rx = std::sync::Mutex::new(release_rx);
    service.set_write_hook(Arc::new(move |path, bytes| {
        if path == blocker {
            let _ = entered_tx.send(());
            let _ = release_rx.lock().unwrap().recv();
        }
        vault::write_private_file(path, bytes)
    }));
    Stall { entered, release }
}

/// Plain unsynced file writes and no directory flushes, for the tests that
/// count hundreds of settlements or stores (what they check is not
/// durability, and hundreds of real flushes slow every test beside them).
fn cheap_files(service: &LocalStateService) {
    service.set_write_hook(Arc::new(cheap_write));
    service.set_sync_hook(Arc::new(|_| Ok(())));
}

fn cheap_write(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    std::fs::create_dir_all(path.parent().expect("a state file has a directory"))?;
    std::fs::write(path, bytes)?;
    #[cfg(unix)]
    {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

/// Restores a directory's mode when the test ends, however it ends.
#[cfg(unix)]
struct ModeGuard(std::path::PathBuf);

#[cfg(unix)]
impl Drop for ModeGuard {
    fn drop(&mut self) {
        let _ = std::fs::set_permissions(&self.0, std::fs::Permissions::from_mode(0o700));
    }
}

#[cfg(unix)]
fn running_as_root() -> bool {
    // SAFETY: geteuid has no preconditions and cannot fail.
    unsafe { libc::geteuid() == 0 }
}

#[test]
fn cold_lease_hydrates_to_an_empty_initial_store_and_never_written_is_none() {
    let _state_lease = crate::test_process::state();
    let scratch = IsolatedEnv::enter("local-state-cold");
    let service = service(&scratch, &[LocalStateDomain::BankHints]);
    let identity = bank_identity("alice");

    assert!(matches!(
        service.lease(&identity),
        Err(ErrorCode::NotReady {
            domain: LocalStateDomain::BankHints
        })
    ));
    assert_eq!(
        service.readback_lease(&identity, WAIT).unwrap(),
        None,
        "a store that has never been written has no file lease"
    );
    let lease = service.lease(&identity).unwrap();
    assert_eq!(lease.version(), StoreVersion::INITIAL);
    assert!(
        lease.snapshot().as_bank_hints().unwrap().is_empty(),
        "after readback the missing file is a real empty lease"
    );
}

#[test]
fn loadouts_store_and_service_exchange_identical_rows_and_pretty_json() {
    let _state_lease = crate::test_process::state();
    let scratch = IsolatedEnv::enter("local-state-loadouts");
    let root = StateRoot::new(scratch.home.join(".274bot"));
    let path = root.loadouts_file();
    let first = Loadout::new("starter")
        .with_slot("righthand", "Bronze sword")
        .with_carry("Shrimps", 4);
    write_loadouts(&path, std::slice::from_ref(&first));

    let service = LocalStateService::open(
        root,
        &[LocalStateDomain::LoadoutsReferences],
        Arc::new(RequestIdMinter::new()),
    );
    let identity = loadouts_identity();
    let lease = service.readback_lease(&identity, WAIT).unwrap().unwrap();
    assert_eq!(
        lease.snapshot().as_loadouts().unwrap(),
        std::slice::from_ref(&first)
    );

    let second = Loadout::new("banking").with_carry("Coins", 250);
    let request = request(service.submit(loadout_edit(Some(lease.version()), second.clone())));
    let receipt = completed(&service, request);
    let rows = vec![first, second];
    let expected = serde_json::to_string_pretty(&rows).unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), expected.as_bytes());
    let operator_store = form(&service);
    assert_eq!(operator_store.loadouts(), rows.as_slice());
    assert_eq!(receipt.version(), lease.version().checked_next().unwrap());

    // The receipt is the durable source for the audit version and timestamp.
    // AuditCommit is deliberately not read back from log text.
    let audit = AuditCommit::after_commit(
        receipt.identity().clone(),
        CommitAction::LoadoutWrite,
        lease.version(),
        receipt.version(),
        AuditOrigin::Submitted { request },
        receipt.at(),
    );
    assert_eq!(audit.action(), CommitAction::LoadoutWrite);
    assert_eq!(audit.previous(), lease.version());
    assert_eq!(audit.version(), receipt.version());
    assert_eq!(audit.identity(), receipt.identity());
    assert_eq!(audit.origin(), AuditOrigin::Submitted { request });
    assert_eq!(audit.at(), receipt.at());
}

#[test]
fn expected_version_conflict_settles_without_touching_the_file() {
    let _state_lease = crate::test_process::state();
    let scratch = IsolatedEnv::enter("local-state-conflict");
    let root = StateRoot::new(scratch.home.join(".274bot"));
    let service = LocalStateService::open(
        root.clone(),
        &[LocalStateDomain::LoadoutsReferences],
        Arc::new(RequestIdMinter::new()),
    );
    let identity = loadouts_identity();
    let initial = service.readback_lease(&identity, WAIT).unwrap();
    assert!(initial.is_none());
    let stale = StoreVersion::INITIAL;
    let first = request(service.submit(loadout_edit(
        None,
        Loadout::new("first").with_carry("Coins", 10),
    )));
    let first_receipt = completed(&service, first);
    let path = root.loadouts_file();
    let before = std::fs::read(&path).unwrap();
    let conflict = request(service.submit(loadout_edit(
        Some(stale),
        Loadout::new("stale").with_carry("Coins", 99),
    )));
    assert_eq!(
        settlement(&service, conflict),
        Settlement::Failed(ErrorCode::Conflict {
            expected: stale,
            found: first_receipt.version(),
        })
    );
    assert_eq!(std::fs::read(path).unwrap(), before);
}

#[test]
fn unopened_preferences_are_not_ready_for_leases_or_submissions() {
    let _state_lease = crate::test_process::state();
    let scratch = IsolatedEnv::enter("local-state-closed-domain");
    let service = service(&scratch, &[LocalStateDomain::BankHints]);
    let identity = identity(LocalStateDomain::Preferences, "alice");
    assert_eq!(
        service.lease(&identity),
        Err(ErrorCode::NotReady {
            domain: LocalStateDomain::Preferences
        })
    );
    let edit = PreferencesEdit::try_new(identity, None, "theme", "dark").unwrap();
    assert_eq!(
        service.submit(DomainEdit::Preferences(edit)),
        SubmitReceipt::Refused(ErrorCode::NotReady {
            domain: LocalStateDomain::Preferences
        })
    );
}

#[test]
fn external_loadout_edit_is_read_back_at_a_higher_version() {
    let _state_lease = crate::test_process::state();
    let scratch = IsolatedEnv::enter("local-state-external-edit");
    let root = StateRoot::new(scratch.home.join(".274bot"));
    let path = root.loadouts_file();
    let first = Loadout::new("first");
    write_loadouts(&path, std::slice::from_ref(&first));
    let service = LocalStateService::open(
        root,
        &[LocalStateDomain::LoadoutsReferences],
        Arc::new(RequestIdMinter::new()),
    );
    let identity = loadouts_identity();
    let before = service.readback_lease(&identity, WAIT).unwrap().unwrap();
    assert_eq!(
        before.snapshot().as_loadouts().unwrap(),
        std::slice::from_ref(&first)
    );

    let second = Loadout::new("external");
    write_loadouts(&path, &[first.clone(), second.clone()]);
    let after = service.readback_lease(&identity, WAIT).unwrap().unwrap();
    assert_eq!(after.snapshot().as_loadouts().unwrap(), &[first, second]);
    assert!(after.version() > before.version());
}

/// r2 item 5 (sol P1, grok High): a hint file another profile, account or
/// schema wrote is refused visibly, never read as "no file".
#[test]
fn wrong_hint_profile_account_or_schema_is_refused_not_absent_and_never_loaded() {
    let _state_lease = crate::test_process::state();
    let scratch = IsolatedEnv::enter("local-state-invalid-hints");
    let root = StateRoot::new(scratch.home.join(".274bot"));
    let service = LocalStateService::open(
        root.clone(),
        &[LocalStateDomain::BankHints],
        Arc::new(RequestIdMinter::new()),
    );
    let alice = bank_identity("alice");
    let bob = bank_identity("bob");
    let carol = bank_identity("carol");
    write_private(
        &root.bank_hint_file(PROFILE, "alice"),
        &hint_document(
            "another-profile",
            "alice",
            HINT_SCHEMA_VERSION,
            1,
            &[(995, 1)],
        ),
    );
    write_private(
        &root.bank_hint_file(PROFILE, "bob"),
        &hint_document(PROFILE, "alice", HINT_SCHEMA_VERSION, 2, &[(995, 2)]),
    );
    write_private(
        &root.bank_hint_file(PROFILE, "carol"),
        &hint_document(PROFILE, "carol", HINT_SCHEMA_VERSION + 1, 3, &[(995, 3)]),
    );
    let refused = |reason| ErrorCode::Failed {
        domain: LocalStateDomain::BankHints,
        reason,
    };
    assert_eq!(
        service.readback_lease(&alice, WAIT),
        Err(refused(FailureReason::Identity))
    );
    assert_eq!(
        service.readback_lease(&bob, WAIT),
        Err(refused(FailureReason::Identity))
    );
    assert_eq!(
        service.readback_lease(&carol, WAIT),
        Err(refused(FailureReason::Schema))
    );
    assert_eq!(service.lease(&carol), Err(refused(FailureReason::Schema)));

    let memory = Arc::new(parking_lot::RwLock::new(
        api::bank_memory::BankMemory::default(),
    ));
    let port = crate::slot_bank_memory::SlotHintPort::new(Arc::clone(&service), carol);
    let slot = crate::slot_bank_memory::SlotBankMemory::new(Arc::clone(&memory), Some(port));
    slot.session_started();
    assert!(
        !memory.read().known(),
        "a refused hint leaves memory Unknown"
    );
}

/// Duplicate fields make a hint file ambiguous, so the service must refuse
/// it instead of using serde_json's last value. Refusals keep the source
/// bytes, write no empty hint, and name the path, cause and remedy.
#[test]
fn duplicate_fields_in_bank_hint_files_are_refused_and_never_written() {
    let _state_lease = crate::test_process::state();
    let scratch = IsolatedEnv::enter("local-state-duplicate-hints");
    let root = StateRoot::new(scratch.home.join(".274bot"));
    let cases: [(&str, Vec<u8>, &str); 3] = [
        (
            "alice",
            br#"{
                "schema_version": 1,
                "schema_version": 1,
                "profile": "local-289",
                "account": "alice",
                "revision": 289,
                "content": "test-content",
                "observed_at_unix": 5,
                "rows": [[995, 5]]
            }"#
            .to_vec(),
            r#"field "schema_version" is named twice in the top level; edit the file to remove one of them"#,
        ),
        (
            "bob",
            br#"{
                "schema_version": 1,
                "profile": "local-289",
                "account": "bob",
                "revision": 289,
                "content": "test-content",
                "observed_at_unix": 5,
                "rows": [[995, 5]],
                "future": {"same": 1, "same": 2}
            }"#
            .to_vec(),
            r#"field "same" is named twice in the object "future"; edit the file to remove one of them"#,
        ),
        (
            "carol",
            br#"{
                "profile": "local-289",
                "account": "carol",
                "observed_at_unix": 5,
                "rows": [[995, 5]],
                "future": 1,
                "future": 2
            }"#
            .to_vec(),
            r#"field "future" is named twice in the top level; edit the file to remove one of them"#,
        ),
    ];
    for (account, bytes, _) in &cases {
        write_private(&root.bank_hint_file(PROFILE, account), bytes);
    }
    let startup = crate::saved_data::SavedDataStartup::check(&root).unwrap();
    assert!(
        startup.refusal().is_none(),
        "a duplicate field is not an old-format move-aside prompt"
    );

    let notices = Arc::new(std::sync::Mutex::new(Vec::new()));
    let sink = Arc::clone(&notices);
    let state = State {
        hooks: Hooks {
            read_refusal: Some(Arc::new(move |_, line| {
                sink.lock().unwrap().push(line.to_string());
            })),
            ..Hooks::default()
        },
        ..State::default()
    };
    let service = LocalStateService::open_with_state(
        root.clone(),
        &[LocalStateDomain::BankHints],
        Arc::new(RequestIdMinter::new()),
        state,
    );
    for (account, bytes, cause) in &cases {
        let identity = bank_identity(account);
        let path = root.bank_hint_file(PROFILE, account);
        let refused = ErrorCode::Failed {
            domain: LocalStateDomain::BankHints,
            reason: FailureReason::Schema,
        };
        assert_eq!(
            service.readback_lease(&identity, WAIT),
            Err(refused),
            "{account}"
        );
        assert_eq!(service.lease(&identity), Err(refused), "{account}");
        assert_eq!(
            service.submit(hint_edit(&identity, None, vec![(995, 99)], 9)),
            SubmitReceipt::Refused(refused),
            "{account}"
        );
        assert_eq!(
            std::fs::read(&path).unwrap(),
            *bytes,
            "{account}: source kept"
        );
        assert_eq!(
            siblings(&path),
            vec![path.file_name().unwrap().to_string_lossy().into_owned()],
            "{account}: no empty or quarantine file"
        );
        let path_text = path.display().to_string();
        let notices = notices.lock().unwrap();
        let notice = notices
            .iter()
            .find(|notice| notice.contains(&path_text))
            .unwrap_or_else(|| panic!("{account}: no refusal notice for {path_text}"));
        assert!(notice.contains(cause), "{account}: {notice}");
        assert!(
            notice.contains("it is neither leased nor written until it reads cleanly"),
            "{account}: {notice}"
        );
    }
    assert_eq!(notices.lock().unwrap().len(), cases.len());
}
/// r2 item 5: every refused hint file the review attacked (unparseable,
/// another schema, another identity, group/other-writable, unreadable)
/// stays refused for leases and submissions and keeps its bytes.
#[cfg(unix)]
#[test]
fn refused_hint_files_are_never_overwritten() {
    let _state_lease = crate::test_process::state();
    if running_as_root() {
        return;
    }
    let scratch = IsolatedEnv::enter("local-state-refused-hints");
    let root = StateRoot::new(scratch.home.join(".274bot"));
    let service = LocalStateService::open(
        root.clone(),
        &[LocalStateDomain::BankHints],
        Arc::new(RequestIdMinter::new()),
    );
    let cases: [(&str, Vec<u8>, Option<u32>, FailureReason); 5] = [
        (
            "corrupt",
            b"{not json".to_vec(),
            None,
            FailureReason::Schema,
        ),
        (
            "schema",
            hint_document(PROFILE, "schema", 7, 1, &[(995, 1)]),
            None,
            FailureReason::Schema,
        ),
        (
            "identity",
            hint_document(PROFILE, "someone", HINT_SCHEMA_VERSION, 1, &[(995, 1)]),
            None,
            FailureReason::Identity,
        ),
        (
            "modesixsix",
            hint_document(PROFILE, "modesixsix", HINT_SCHEMA_VERSION, 1, &[(995, 1)]),
            Some(0o666),
            FailureReason::Io,
        ),
        (
            "modezero",
            hint_document(PROFILE, "modezero", HINT_SCHEMA_VERSION, 1, &[(995, 1)]),
            Some(0o000),
            FailureReason::Io,
        ),
    ];
    for (account, bytes, mode, reason) in cases {
        let identity = bank_identity(account);
        let path = root.bank_hint_file(PROFILE, account);
        write_private(&path, &bytes);
        if let Some(mode) = mode {
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode)).unwrap();
        }
        let code = ErrorCode::Failed {
            domain: LocalStateDomain::BankHints,
            reason,
        };
        assert_eq!(
            service.readback_lease(&identity, WAIT),
            Err(code),
            "{account}"
        );
        assert_eq!(
            service.submit(hint_edit(&identity, None, vec![(995, 99)], 9)),
            SubmitReceipt::Refused(code),
            "{account}"
        );
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), bytes, "{account} kept");
    }
}

/// r2 item 5 (sol probe: a valid file past the read bound; grok: a
/// document that is not an array): refused loadouts are never replaced.
#[test]
fn refused_loadouts_files_are_never_overwritten() {
    let _state_lease = crate::test_process::state();
    let scratch = IsolatedEnv::enter("local-state-refused-loadouts");
    let root = StateRoot::new(scratch.home.join(".274bot"));
    let path = root.loadouts_file();
    let wrapped = serde_json::to_vec(
        &serde_json::json!({ "loadouts": [{"name": "kept", "worn": {}, "carry": []}] }),
    )
    .unwrap();
    let big_row = Loadout::new("big").with_carry("x".repeat(1024), 1);
    let oversize = serde_json::to_vec_pretty(&vec![big_row; 4200]).unwrap();
    assert!(oversize.len() as u64 > files::MAX_LOADOUTS_BYTES);
    for (label, bytes) in [
        ("wrapped", wrapped),
        ("oversize", oversize),
        ("corrupt", b"[{\"name\": ".to_vec()),
    ] {
        write_private(&path, &bytes);
        let service = LocalStateService::open(
            root.clone(),
            &[LocalStateDomain::LoadoutsReferences],
            Arc::new(RequestIdMinter::new()),
        );
        let refused = service.readback_lease(&loadouts_identity(), WAIT);
        assert!(
            matches!(
                refused,
                Err(ErrorCode::Failed {
                    domain: LocalStateDomain::LoadoutsReferences,
                    ..
                })
            ),
            "{label}: {refused:?}"
        );
        assert!(
            matches!(
                service.submit(loadout_edit(None, Loadout::new("new"))),
                SubmitReceipt::Refused(ErrorCode::Failed { .. })
            ),
            "{label}"
        );
        assert!(std::fs::read(&path).unwrap() == bytes, "{label} kept");
    }
}

/// A loadout with repeated JSON fields is ambiguous, so the reader refuses
/// it instead of using the last value and never writes over the source.
#[test]
fn duplicate_fields_in_loadout_files_are_refused_and_never_written() {
    let _state_lease = crate::test_process::state();
    let scratch = IsolatedEnv::enter("local-state-duplicate-loadouts");
    let root = StateRoot::new(scratch.home.join(".274bot"));
    let path = root.loadouts_file();
    let cases: [(&str, Vec<u8>, &str); 2] = [
        (
            "top-level",
            br#"[{"name":"alice","name":"bob","worn":{},"carry":[]}]"#.to_vec(),
            r#"field "name" is named twice in the object "[0]"; edit the file to remove one of them"#,
        ),
        (
            "nested",
            br#"[{"name":"alice","worn":{},"carry":[{"item":"tuna","qty":1,"qty":2}]}]"#.to_vec(),
            r#"field "qty" is named twice in the object "[0].carry[0]"; edit the file to remove one of them"#,
        ),
    ];
    let notices = Arc::new(std::sync::Mutex::new(Vec::new()));
    for (label, bytes, cause) in &cases {
        write_private(&path, bytes);
        let sink = Arc::clone(&notices);
        let state = State {
            hooks: Hooks {
                read_refusal: Some(Arc::new(move |_, line| {
                    sink.lock().unwrap().push(line.to_string());
                })),
                ..Hooks::default()
            },
            ..State::default()
        };
        let service = LocalStateService::open_with_state(
            root.clone(),
            &[LocalStateDomain::LoadoutsReferences],
            Arc::new(RequestIdMinter::new()),
            state,
        );
        let refused = ErrorCode::Failed {
            domain: LocalStateDomain::LoadoutsReferences,
            reason: FailureReason::Schema,
        };
        assert_eq!(
            service.readback_lease(&loadouts_identity(), WAIT),
            Err(refused),
            "{label}"
        );
        assert_eq!(
            service.submit(loadout_edit(None, Loadout::new("new"))),
            SubmitReceipt::Refused(refused),
            "{label}"
        );
        assert_eq!(
            std::fs::read(&path).unwrap().as_slice(),
            bytes.as_slice(),
            "{label}: source kept"
        );
        let path_text = path.display().to_string();
        let notices = notices.lock().unwrap();
        let notice = notices
            .iter()
            .find(|notice| notice.contains(&path_text) && notice.contains(cause))
            .unwrap_or_else(|| panic!("{label}: no actionable refusal for {path_text}"));
        assert!(
            notice.contains("it is neither leased nor written until it reads cleanly"),
            "{label}: {notice}"
        );
    }
    assert_eq!(notices.lock().unwrap().len(), cases.len());
}

/// r2 item 5: rows the service cannot read back as they are (a malformed
/// row, an unknown field) are leased, but the file is never rewritten
/// without them.
#[test]
fn a_loadouts_file_the_service_cannot_round_trip_is_read_but_never_rewritten() {
    let _state_lease = crate::test_process::state();
    let scratch = IsolatedEnv::enter("local-state-lossy-loadouts");
    let root = StateRoot::new(scratch.home.join(".274bot"));
    let path = root.loadouts_file();
    let bytes = serde_json::to_vec_pretty(&serde_json::json!([
        {"name": "melee", "worn": {"righthand": "Rune scimitar"}, "carry": [], "note": "keep me"},
        {"worn": {"hat": "no name"}},
    ]))
    .unwrap();
    write_private(&path, &bytes);
    let service = LocalStateService::open(
        root,
        &[LocalStateDomain::LoadoutsReferences],
        Arc::new(RequestIdMinter::new()),
    );
    let lease = service
        .readback_lease(&loadouts_identity(), WAIT)
        .unwrap()
        .unwrap();
    assert_eq!(
        lease.snapshot().as_loadouts().unwrap(),
        &[Loadout::new("melee").with_slot("righthand", "Rune scimitar")]
    );
    assert_eq!(
        service.submit(loadout_edit(Some(lease.version()), Loadout::new("ranged"))),
        SubmitReceipt::Refused(ErrorCode::Failed {
            domain: LocalStateDomain::LoadoutsReferences,
            reason: FailureReason::Schema,
        })
    );
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
}

/// r2 item 4 (sol probe 2): a loadout edit written against the cached
/// version after the form saved another row conflicts; the saved row stays.
#[test]
fn a_loadout_edit_after_a_form_saved_a_row_conflicts_and_keeps_the_row() {
    let _state_lease = crate::test_process::state();
    let scratch = IsolatedEnv::enter("local-state-form-race");
    let root = StateRoot::new(scratch.home.join(".274bot"));
    let path = root.loadouts_file();
    let service = LocalStateService::open(
        root,
        &[LocalStateDomain::LoadoutsReferences],
        Arc::new(RequestIdMinter::new()),
    );
    let hydrated = service.readback_lease(&loadouts_identity(), WAIT).unwrap();
    assert!(hydrated.is_none());
    let version = service.lease(&loadouts_identity()).unwrap().version();
    // Another writer (an older build's form, a hand edit) saves a row.
    write_loadouts(&path, &[Loadout::new("frontend-saved")]);
    let saved = std::fs::read(&path).unwrap();

    let edit = request(service.submit(loadout_edit(Some(version), Loadout::new("service"))));
    assert!(matches!(
        settlement(&service, edit),
        Settlement::Failed(ErrorCode::Conflict { expected, .. }) if expected == version
    ));
    assert_eq!(std::fs::read(&path).unwrap(), saved);
    let lease = service.lease(&loadouts_identity()).unwrap();
    assert_eq!(
        lease.snapshot().as_loadouts().unwrap(),
        &[Loadout::new("frontend-saved")]
    );
}

/// The envelope from before schema 1 (no `schema_version`) is older than
/// the 0.2.8 format, which could not read it: it is refused whole.
#[test]
fn a_hint_without_schema_version_is_refused_and_kept() {
    let scratch = IsolatedEnv::enter("local-state-pre-schema-hint");
    let root = StateRoot::new(scratch.home.join(".274bot"));
    let path = root.bank_hint_file(PROFILE, "alice");
    let bytes = serde_json::to_vec(&serde_json::json!({
        "profile": PROFILE,
        "account": "alice",
        "observed_at_unix": 3,
        "rows": [[995, 3]],
    }))
    .unwrap();
    write_private(&path, &bytes);
    let service = service(&scratch, &[LocalStateDomain::BankHints]);
    let alice = bank_identity("alice");
    let refused = ErrorCode::Failed {
        domain: LocalStateDomain::BankHints,
        reason: FailureReason::UnsupportedSavedFormat,
    };

    assert_eq!(service.readback_lease(&alice, WAIT), Err(refused));
    assert_eq!(service.lease(&alice), Err(refused));
    assert_eq!(
        service.submit(hint_edit(&alice, None, vec![(995, 4)], 4)),
        SubmitReceipt::Refused(refused)
    );
    assert_eq!(service.readback_lease(&alice, WAIT), Err(refused));
    assert_eq!(std::fs::read(&path).unwrap(), bytes, "original kept");
}

/// r2 item 1: a write whose directory entry cannot be flushed is not
/// acknowledged durable and is not audited. The root is made `-wx` so the
/// file can be staged and renamed but the directory cannot be opened to
/// flush it.
#[cfg(unix)]
#[test]
fn a_write_whose_directory_entry_cannot_be_flushed_is_not_durable() {
    let _state_lease = crate::test_process::state();
    if running_as_root() {
        return;
    }
    let scratch = IsolatedEnv::enter("local-state-dir-sync");
    let root = StateRoot::new(scratch.home.join(".274bot"));
    let service = LocalStateService::open(
        root.clone(),
        &[LocalStateDomain::LoadoutsReferences],
        Arc::new(RequestIdMinter::new()),
    );
    assert!(service
        .readback_lease(&loadouts_identity(), WAIT)
        .unwrap()
        .is_none());
    let audits = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&audits);
    service.set_audit_hook(Arc::new(move |_| {
        counted.fetch_add(1, Ordering::SeqCst);
    }));
    vault::create_private_dir(root.path()).unwrap();
    let guard = ModeGuard(root.path().to_path_buf());
    std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o300)).unwrap();

    let edit = loadout_edit(None, Loadout::new("melee"));
    let unsynced = request(service.submit(edit.clone()));
    assert_eq!(
        settlement(&service, unsynced),
        Settlement::Failed(ErrorCode::Failed {
            domain: LocalStateDomain::LoadoutsReferences,
            reason: FailureReason::Io,
        }),
        "a rename whose directory was not flushed is not durable"
    );
    assert_eq!(
        audits.load(Ordering::SeqCst),
        0,
        "no audit before durability"
    );

    drop(guard);
    let retried = request(service.submit(edit));
    completed(&service, retried);
    assert_eq!(
        audits.load(Ordering::SeqCst),
        1,
        "the durable retry is audited"
    );
}

/// r2 item 2 (sol P1, grok probe): dropping the last service handle
/// finishes every accepted write, queued or in flight, and joins the
/// writer before it returns.
#[test]
fn dropping_the_service_finishes_every_accepted_write_and_joins_the_writer() {
    let _state_lease = crate::test_process::state();
    let scratch = IsolatedEnv::enter("local-state-drain");
    let root = StateRoot::new(scratch.home.join(".274bot"));
    let service = LocalStateService::open(
        root.clone(),
        &[
            LocalStateDomain::BankHints,
            LocalStateDomain::LoadoutsReferences,
        ],
        Arc::new(RequestIdMinter::new()),
    );
    let alice = bank_identity("alice");
    let bob = bank_identity("bob");
    let _ = service.readback_lease(&loadouts_identity(), WAIT).unwrap();
    assert_eq!(service.readback_lease(&alice, WAIT).unwrap(), None);
    assert_eq!(service.readback_lease(&bob, WAIT).unwrap(), None);
    let stall = stall_on(&service, root.loadouts_file());
    let _blocker = request(service.submit(loadout_edit(None, Loadout::new("blocker"))));
    stall.entered.recv_timeout(WAIT).unwrap();
    let _alice = request(service.submit(hint_edit(&alice, None, vec![(995, 64)], 64)));
    let _bob = request(service.submit(hint_edit(&bob, None, vec![(995, 65)], 65)));

    let shared = Arc::clone(&service.shared);
    let release = stall.release;
    let releaser = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(200));
        let _ = release.send(());
    });
    drop(service);
    releaser.join().unwrap();

    assert_eq!(shared.state.lock().writer, WriterPhase::Exited);
    assert!(shared.writer.lock().is_none(), "the writer was joined");
    assert!(std::fs::read_to_string(root.loadouts_file())
        .unwrap()
        .contains("blocker"));
    let reopened = LocalStateService::open(
        root.clone(),
        &[LocalStateDomain::BankHints],
        Arc::new(RequestIdMinter::new()),
    );
    for (account, coins) in [("alice", 64), ("bob", 65)] {
        let lease = reopened
            .readback_lease(&bank_identity(account), WAIT)
            .unwrap()
            .unwrap_or_else(|| panic!("{account}'s queued hint was dropped"));
        assert_eq!(lease.snapshot().as_bank_hints().unwrap(), &[(995, coins)]);
    }
}

/// r2 item 2: a write that panics settles its request (and those queued
/// behind it) instead of leaving them pending, and the writer keeps
/// serving.
#[test]
fn a_panicking_write_fails_its_request_and_the_writer_keeps_serving() {
    let _state_lease = crate::test_process::state();
    let scratch = IsolatedEnv::enter("local-state-panic");
    let root = StateRoot::new(scratch.home.join(".274bot"));
    let service = LocalStateService::open(
        root.clone(),
        &[LocalStateDomain::BankHints],
        Arc::new(RequestIdMinter::new()),
    );
    let alice = bank_identity("alice");
    assert_eq!(service.readback_lease(&alice, WAIT).unwrap(), None);
    let panicked = Arc::new(AtomicBool::new(false));
    let once = Arc::clone(&panicked);
    service.set_sql_seams_for_test(sql::Seams {
        point: Some(Arc::new(move |name| {
            if name == "before-commit" && !once.swap(true, Ordering::SeqCst) {
                panic!("injected writer panic");
            }
        })),
        ..sql::Seams::default()
    });
    let doomed = request(service.submit(hint_edit(&alice, None, vec![(995, 1)], 1)));
    assert_eq!(
        settlement(&service, doomed),
        Settlement::Failed(ErrorCode::Failed {
            domain: LocalStateDomain::BankHints,
            reason: FailureReason::Dirty,
        })
    );
    assert_eq!(
        service.readback_lease(&alice, WAIT).unwrap(),
        None,
        "the panicked save rolled back"
    );
    let next = request(service.submit(hint_edit(&alice, None, vec![(995, 2)], 2)));
    completed(&service, next);
    assert_eq!(
        service
            .readback_lease(&alice, WAIT)
            .unwrap()
            .unwrap()
            .snapshot()
            .as_bank_hints()
            .unwrap(),
        &[(995, 2)]
    );
}

/// r2 item 7 (sol P1): bank identities are capped; a new one past the cap
/// evicts the least recently used idle store, which reads its file again
/// when it is next asked for.
#[test]
fn bank_identities_are_capped_by_evicting_idle_stores() {
    let _state_lease = crate::test_process::state();
    let scratch = IsolatedEnv::enter("local-state-identity-cap");
    let service = service(&scratch, &[LocalStateDomain::BankHints]);
    let names: Vec<String> = (0..MAX_BANK_STORES + 44)
        .map(|n| format!("acct{n}"))
        .collect();
    for name in &names {
        assert_eq!(service.readback_lease(&bank_identity(name), WAIT), Ok(None));
    }
    assert!(
        held_stores(&service) <= MAX_BANK_STORES,
        "{} stores held",
        held_stores(&service)
    );
    let first = bank_identity(&names[0]);
    assert!(
        matches!(service.lease(&first), Err(ErrorCode::NotReady { .. })),
        "the evicted store is cold again"
    );
    assert_eq!(service.readback_lease(&first, WAIT), Ok(None));
    assert!(held_stores(&service) <= MAX_BANK_STORES);
}

/// r2 item 7: with every held store busy, a new identity is refused
/// `Busy`, visibly, instead of growing the cache.
#[test]
fn with_every_held_store_busy_a_new_identity_is_refused() {
    let _state_lease = crate::test_process::state();
    let scratch = IsolatedEnv::enter("local-state-identity-busy");
    let root = StateRoot::new(scratch.home.join(".274bot"));
    let service = LocalStateService::open(
        root.clone(),
        &[LocalStateDomain::BankHints],
        Arc::new(RequestIdMinter::new()),
    );
    let blocker = bank_identity("blocker");
    let others: Vec<DomainIdentity> = (1..MAX_BANK_STORES)
        .map(|n| bank_identity(&format!("acct{n}")))
        .collect();
    // Every store is read before the writer is held (a held writer reads
    // nothing).
    assert_eq!(service.readback_lease(&blocker, WAIT), Ok(None));
    for identity in &others {
        assert_eq!(service.readback_lease(identity, WAIT), Ok(None));
    }
    // Hold the first write at its start; every other save queues behind it.
    let (entered_tx, entered) = mpsc::channel();
    let (release, release_rx) = mpsc::channel::<()>();
    let release_rx = std::sync::Mutex::new(release_rx);
    let once = AtomicBool::new(false);
    service.set_write_stall_for_test(Arc::new(move || {
        if !once.swap(true, Ordering::SeqCst) {
            let _ = entered_tx.send(());
            let _ = release_rx.lock().unwrap().recv();
        }
    }));
    let held = request(service.submit(hint_edit(&blocker, None, vec![(995, 1)], 1)));
    entered.recv_timeout(WAIT).unwrap();
    for identity in &others {
        request(service.submit(hint_edit(identity, None, vec![(995, 1)], 1)));
    }
    let late = bank_identity("late");
    assert_eq!(
        service.lease(&late),
        Err(ErrorCode::Busy {
            domain: LocalStateDomain::BankHints
        })
    );
    assert_eq!(held_stores(&service), MAX_BANK_STORES);
    release.send(()).unwrap();
    completed(&service, held);
}

/// r2 item 7: an edit that would take the loadouts file past its row,
/// row-size or file-size budget is refused before it is accepted.
#[test]
fn loadout_edits_past_the_row_or_byte_budget_are_refused() {
    let _state_lease = crate::test_process::state();
    let scratch = IsolatedEnv::enter("local-state-loadout-budget");
    let root = StateRoot::new(scratch.home.join(".274bot"));
    let path = root.loadouts_file();
    let rows: Vec<Loadout> = (0..files::MAX_LOADOUT_ROWS)
        .map(|n| Loadout::new(format!("row{n}")))
        .collect();
    write_loadouts(&path, &rows);
    let service = LocalStateService::open(
        root.clone(),
        &[LocalStateDomain::LoadoutsReferences],
        Arc::new(RequestIdMinter::new()),
    );
    let version = service
        .readback_lease(&loadouts_identity(), WAIT)
        .unwrap()
        .unwrap()
        .version();
    assert_eq!(
        service.submit(loadout_edit(Some(version), Loadout::new("one-too-many"))),
        SubmitReceipt::Refused(ErrorCode::Validation {
            domain: LocalStateDomain::LoadoutsReferences,
            reason: ValidationReason::TooManyRows,
        })
    );
    let replaced = request(service.submit(loadout_edit(
        Some(version),
        Loadout::new("row0").with_carry("Coins", 1),
    )));
    completed(&service, replaced);

    let huge = Loadout::new("huge").with_carry("y".repeat(files::MAX_LOADOUT_ROW_BYTES), 1);
    assert_eq!(
        service.submit(loadout_edit(None, huge)),
        SubmitReceipt::Refused(ErrorCode::Validation {
            domain: LocalStateDomain::LoadoutsReferences,
            reason: ValidationReason::TooLong,
        })
    );

    // At the file bound: as many rows as fit, then one more is refused.
    let wide = |name: String| Loadout::new(name).with_carry("z".repeat(60 * 1024), 1);
    let row_bytes = files::row_bytes(&wide("wide000".into())).unwrap();
    let fit = (files::MAX_LOADOUTS_BYTES as usize - 2) / row_bytes;
    let wides: Vec<Loadout> = (0..fit).map(|n| wide(format!("wide{n:03}"))).collect();
    write_loadouts(&path, &wides);
    assert_eq!(
        std::fs::metadata(&path).unwrap().len() as usize,
        2 + fit * row_bytes,
        "the file is exactly its rows' share plus the brackets"
    );
    let version = service
        .readback_lease(&loadouts_identity(), WAIT)
        .unwrap()
        .unwrap()
        .version();
    assert_eq!(
        service.submit(loadout_edit(Some(version), wide("wide999".into()))),
        SubmitReceipt::Refused(ErrorCode::Validation {
            domain: LocalStateDomain::LoadoutsReferences,
            reason: ValidationReason::TooLong,
        })
    );
}

/// r2 item 8 (sol P1): one account's settlement is not displaced by other
/// accounts' settlements before its slot reads it.
#[test]
fn a_settlement_survives_other_accounts_settlements() {
    let _state_lease = crate::test_process::state();
    let scratch = IsolatedEnv::enter("local-state-retention");
    let service = service(&scratch, &[LocalStateDomain::BankHints]);
    let alice = bank_identity("alice");
    let bob = bank_identity("bob");
    assert_eq!(service.readback_lease(&alice, WAIT), Ok(None));
    assert_eq!(service.readback_lease(&bob, WAIT), Ok(None));
    cheap_files(&service);
    let alice_request = request(service.submit(hint_edit(&alice, None, vec![(995, 1)], 1)));
    completed(&service, alice_request);
    for n in 0..300 {
        let other = request(service.submit(hint_edit(&bob, None, vec![(995, n + 1)], 2)));
        completed(&service, other);
    }
    assert!(
        matches!(
            service.observe(alice_request),
            Some(Settlement::Completed(_))
        ),
        "alice's settlement was displaced by bob's"
    );
}

/// r2 item 8: a settlement the service no longer holds reads `Forgotten`,
/// never pending forever.
#[test]
fn a_settlement_no_longer_held_reads_forgotten() {
    let _state_lease = crate::test_process::state();
    let scratch = IsolatedEnv::enter("local-state-forgotten");
    let service = service(&scratch, &[LocalStateDomain::BankHints]);
    let alice = bank_identity("alice");
    assert_eq!(service.readback_lease(&alice, WAIT), Ok(None));
    cheap_files(&service);
    let old = request(service.submit(hint_edit(&alice, None, vec![(995, 1)], 1)));
    completed(&service, old);
    for n in 0..SETTLED_PER_STORE as i32 {
        let newer = request(service.submit(hint_edit(&alice, None, vec![(995, n + 2)], 2)));
        completed(&service, newer);
    }
    assert_eq!(service.request_state(old), RequestState::Forgotten);
    assert_eq!(service.observe(old), None);
}

/// Receipts are exact to their account, and versions come from one clock:
/// never reused, whichever account takes them.
#[test]
fn durable_receipts_are_exact_to_account_and_versions_are_never_reused() {
    let _state_lease = crate::test_process::state();
    let scratch = IsolatedEnv::enter("local-state-receipts");
    let service = service(&scratch, &[LocalStateDomain::BankHints]);
    let alice = bank_identity("alice");
    let bob = bank_identity("bob");
    assert_eq!(service.readback_lease(&alice, WAIT).unwrap(), None);
    assert_eq!(service.readback_lease(&bob, WAIT).unwrap(), None);
    let alice_hydrated = service.lease(&alice).unwrap().version();
    let bob_hydrated = service.lease(&bob).unwrap().version();

    let alice_request = request(service.submit(hint_edit(&alice, None, vec![(995, 10)], 10)));
    let alice_receipt = completed(&service, alice_request);
    let bob_request = request(service.submit(hint_edit(&bob, None, vec![(995, 20)], 20)));
    let bob_receipt = completed(&service, bob_request);

    let versions = [
        alice_hydrated,
        bob_hydrated,
        alice_receipt.version(),
        bob_receipt.version(),
    ];
    assert!(
        versions.windows(2).all(|pair| pair[0] < pair[1]),
        "{versions:?}"
    );
    assert!(alice_receipt.accepts(&alice));
    assert!(!alice_receipt.accepts(&bob));
    assert!(bob_receipt.accepts(&bob));
    assert!(!bob_receipt.accepts(&alice));

    let audit = AuditCommit::after_commit(
        alice_receipt.identity().clone(),
        CommitAction::BankHintWrite,
        alice_hydrated,
        alice_receipt.version(),
        AuditOrigin::Submitted {
            request: alice_request,
        },
        alice_receipt.at(),
    );
    assert_eq!(audit.previous(), alice_hydrated);
    assert_eq!(audit.version(), alice_receipt.version());
    assert_eq!(audit.action(), CommitAction::BankHintWrite);
    assert_eq!(
        audit.origin(),
        AuditOrigin::Submitted {
            request: alice_request
        }
    );
}

/// The composition's Start-time lease: what `ScriptStartHandle` posts as
/// the operator's rows. A row the frontend's form saved through the same
/// service is seen by the next Start, the slot's capability carries the
/// account the server would log in, and a name the server would refuse
/// gets neither rows nor a capability.
#[test]
fn composition_leases_the_form_edited_loadouts_at_start_and_refuses_non_login_names() {
    let _state_lease = crate::test_process::state();
    let scratch = IsolatedEnv::enter("local-state-composition");
    let root = StateRoot::new(scratch.home.join(".274bot"));
    let service = LocalStateService::open(
        root,
        &[
            LocalStateDomain::LoadoutsReferences,
            LocalStateDomain::BankHints,
        ],
        Arc::new(RequestIdMinter::new()),
    );
    let composition = StateComposition::new(
        service,
        PROFILE,
        ContentScope::try_new(client::io::ClientRevision::R289, CONTENT).unwrap(),
    );
    // Hydrated with no file: a Start posts no rows.
    assert_eq!(
        composition.operator_loadouts("Alice Smith").unwrap(),
        Vec::new()
    );

    // The frontend's form saves through the same service; the next Start's
    // readback sees the row.
    let melee = Loadout::new("melee").with_slot("righthand", "Rune scimitar");
    let mut form = form(composition.service());
    form.upsert(melee.clone());
    save_patiently(&mut form);
    assert_eq!(
        composition.operator_loadouts("Alice Smith").unwrap(),
        vec![melee.clone()]
    );

    // The slot's capability leases the same rows under the login identity.
    let state = composition.slot_state("Alice Smith").expect("a login name");
    assert_eq!(state.account().profile(), PROFILE);
    assert_eq!(state.account().account(), "alice_smith");
    let leased = state.operator_loadouts().unwrap();
    assert_eq!(leased.rows(), std::slice::from_ref(&melee));
    assert_eq!(
        leased.version(),
        composition
            .service()
            .lease(&state.identity(LocalStateDomain::LoadoutsReferences))
            .unwrap()
            .version()
    );

    // A name the server refuses at login is refused here, not keyed.
    assert!(composition.slot_state("ab3456789xyz1").is_none());
    assert!(composition
        .operator_loadouts("ab3456789xyz1")
        .unwrap_err()
        .contains("login identity"));
    assert!(composition
        .identity(LocalStateDomain::BankHints, "a/b")
        .is_err());
}

// ---- T5.2 r3 ----

/// Every audit line the service logs, collected.
fn audits(service: &LocalStateService) -> Arc<std::sync::Mutex<Vec<AuditCommit>>> {
    let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
    let sink = Arc::clone(&seen);
    service.set_audit_hook(Arc::new(move |audit| {
        sink.lock().unwrap().push(audit.clone())
    }));
    seen
}

/// Every terminal line the service logs, collected.
fn terminals(service: &LocalStateService) -> Arc<std::sync::Mutex<Vec<String>>> {
    let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
    let sink = Arc::clone(&seen);
    service.set_terminal_hook(Arc::new(move |line| {
        sink.lock().unwrap().push(line.to_string())
    }));
    seen
}

/// T5.2 r3 (sol F1): a form of a build before r3 (or a hand edit) saves a
/// loadout row while the service stages its own edit. The service never
/// writes over the saved row: its edit conflicts and the row stays.
#[test]
fn a_form_save_during_a_service_publication_is_never_overwritten() {
    let _state_lease = crate::test_process::state();
    let scratch = IsolatedEnv::enter("local-state-r3-form-publication");
    let root = StateRoot::new(scratch.home.join(".274bot"));
    let path = root.loadouts_file();
    write_loadouts(&path, &[Loadout::new("original")]);
    let service = service(&scratch, &[LocalStateDomain::LoadoutsReferences]);
    let version = service
        .readback_lease(&loadouts_identity(), WAIT)
        .unwrap()
        .unwrap()
        .version();
    let once = AtomicBool::new(false);
    service.set_publish_hook(Arc::new(move |path| {
        if !once.swap(true, Ordering::SeqCst) {
            // A form of a build before r3 saving in place, taking no lock.
            let rows = [Loadout::new("original"), Loadout::new("frontend-saved")];
            let bytes = serde_json::to_string_pretty(&rows).unwrap();
            cheap_write(path, bytes.as_bytes()).unwrap();
        }
    }));

    let edit = request(service.submit(loadout_edit(Some(version), Loadout::new("service-edit"))));
    let settled = settlement(&service, edit);
    assert!(
        matches!(settled, Settlement::Failed(ErrorCode::Conflict { .. })),
        "{settled:?}"
    );
    let rows: Vec<Loadout> = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(
        rows,
        [Loadout::new("original"), Loadout::new("frontend-saved")],
        "the saved row is never overwritten"
    );
}

/// T5.2 r3 (sol F1, grok P1): two services on one state root (two
/// compositions, or two processes on one `HOME`) never interleave on a
/// file: while one holds its write transaction the other waits for the
/// lock, then sees the first one's commit; no settlement is `Completed`
/// for a row the file does not hold.
#[test]
fn two_services_on_one_root_never_interleave_their_writes() {
    let _state_lease = crate::test_process::state();
    let scratch = IsolatedEnv::enter("local-state-r3-two-services");
    let root = StateRoot::new(scratch.home.join(".274bot"));
    let path = root.loadouts_file();
    write_loadouts(&path, &[Loadout::new("original")]);
    let first = service(&scratch, &[LocalStateDomain::LoadoutsReferences]);
    let second = service(&scratch, &[LocalStateDomain::LoadoutsReferences]);
    let first_version = first
        .readback_lease(&loadouts_identity(), WAIT)
        .unwrap()
        .unwrap()
        .version();
    let second_version = second
        .readback_lease(&loadouts_identity(), WAIT)
        .unwrap()
        .unwrap()
        .version();
    let (entered_tx, entered) = mpsc::channel();
    let (release, release_rx) = mpsc::channel::<()>();
    let release_rx = std::sync::Mutex::new(release_rx);
    first.set_publish_hook(Arc::new(move |_| {
        let _ = entered_tx.send(());
        let _ = release_rx.lock().unwrap().recv();
    }));

    let a = request(first.submit(loadout_edit(Some(first_version), Loadout::new("a-row"))));
    entered.recv_timeout(WAIT).unwrap();
    let b = request(second.submit(loadout_edit(Some(second_version), Loadout::new("b-row"))));
    assert_eq!(
        settled_within(&second, b, Duration::from_millis(300)),
        None,
        "the second writer must wait for the first one's transaction"
    );
    release.send(()).unwrap();
    let a_settled = settlement(&first, a);
    let b_settled = settlement(&second, b);

    let rows: Vec<Loadout> = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    for (row, settled) in [("a-row", &a_settled), ("b-row", &b_settled)] {
        if matches!(settled, Settlement::Completed(_)) {
            assert!(
                rows.iter().any(|held| held.name == row),
                "{row} was acknowledged but the file lost it: {rows:?}"
            );
        }
    }
    assert!(
        matches!(a_settled, Settlement::Completed(_)),
        "{a_settled:?}"
    );
    assert!(
        matches!(b_settled, Settlement::Failed(ErrorCode::Conflict { .. })),
        "the second edit was written against the file before the first commit: {b_settled:?}"
    );

    // Retried against what the file holds now, both rows survive.
    let current = second
        .readback_lease(&loadouts_identity(), WAIT)
        .unwrap()
        .unwrap()
        .version();
    completed(
        &second,
        request(second.submit(loadout_edit(Some(current), Loadout::new("b-row")))),
    );
    let rows: Vec<Loadout> = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    let names: Vec<&str> = rows.iter().map(|row| row.name.as_str()).collect();
    assert_eq!(names, ["original", "a-row", "b-row"]);
}

/// The other process of [`a_writer_in_another_process_holding_the_lock_makes_the_write_wait`]:
/// it takes the file's write lock, says so, holds it while it writes the
/// file, then lets it go.
#[test]
#[ignore = "lock-holding subprocess of the cross-process lock test"]
fn lock_holding_child() {
    let file = std::path::PathBuf::from(std::env::var_os("T52_LOCK_FILE").unwrap());
    let ready = std::path::PathBuf::from(std::env::var_os("T52_LOCK_READY").unwrap());
    let lock = files::lock_for_write(&file, std::time::Instant::now() + WAIT, &|| false)
        .expect("the child takes the lock");
    std::fs::write(&ready, b"locked").unwrap();
    std::thread::sleep(Duration::from_millis(500));
    write_loadouts(
        &file,
        &[Loadout::new("original"), Loadout::new("child-row")],
    );
    drop(lock);
}

/// T5.2 r3: a writer in another process (a second 274bot on the same
/// `HOME`) holds the file's lock: this service's write waits for it, then
/// finds the other process's commit and conflicts instead of overwriting
/// it.
#[test]
fn a_writer_in_another_process_holding_the_lock_makes_the_write_wait() {
    crate::test_process::isolate_state_test!(
        a_writer_in_another_process_holding_the_lock_makes_the_write_wait
    );
    let scratch = IsolatedEnv::enter("local-state-r3-other-process");
    let root = StateRoot::new(scratch.home.join(".274bot"));
    let path = root.loadouts_file();
    write_loadouts(&path, &[Loadout::new("original")]);
    let service = service(&scratch, &[LocalStateDomain::LoadoutsReferences]);
    let version = service
        .readback_lease(&loadouts_identity(), WAIT)
        .unwrap()
        .unwrap()
        .version();
    let ready = scratch.home.join("child-ready");
    let mut child = std::process::Command::new(crate::test_process::executable())
        .args([
            "--ignored",
            "--exact",
            "local_state::tests::lock_holding_child",
        ])
        .env("T52_LOCK_FILE", &path)
        .env("T52_LOCK_READY", &ready)
        .stdout(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let deadline = std::time::Instant::now() + WAIT;
    while !ready.exists() {
        assert!(
            std::time::Instant::now() < deadline,
            "the child never locked"
        );
        std::thread::sleep(Duration::from_millis(5));
    }

    let edit = request(service.submit(loadout_edit(Some(version), Loadout::new("parent-row"))));
    assert_eq!(
        settled_within(&service, edit, Duration::from_millis(200)),
        None,
        "the write must wait for the other process's lock"
    );
    assert!(child.wait().unwrap().success());
    let settled = settlement(&service, edit);
    assert!(
        matches!(settled, Settlement::Failed(ErrorCode::Conflict { .. })),
        "{settled:?}"
    );
    let rows: Vec<Loadout> = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(rows, [Loadout::new("original"), Loadout::new("child-row")]);
}

/// T5.2 r3 (grok P2): past the shutdown bound no accepted write is dropped
/// silently. A write the writer holds short of its transaction and a write
/// queued behind it each settle `Cancelled(Shutdown)` with their own
/// terminal line; the held one is never committed afterwards.
#[test]
fn past_the_shutdown_bound_every_accepted_write_settles_with_its_own_line() {
    let _state_lease = crate::test_process::state();
    let scratch = IsolatedEnv::enter("local-state-r3-shutdown-bound");
    let root = StateRoot::new(scratch.home.join(".274bot"));
    let service = service(&scratch, &[LocalStateDomain::BankHints]);
    service.set_shutdown_timeout(Duration::from_millis(300));
    let lines = terminals(&service);
    let (alice, bob) = (bank_identity("alice"), bank_identity("bob"));
    let (entered_tx, entered) = mpsc::channel();
    let (release, release_rx) = mpsc::channel::<()>();
    let release_rx = std::sync::Mutex::new(release_rx);
    service.set_write_stall_for_test(Arc::new(move || {
        let _ = entered_tx.send(());
        let _ = release_rx.lock().unwrap().recv();
    }));
    for identity in [&alice, &bob] {
        assert!(service.readback_lease(identity, WAIT).unwrap().is_none());
    }
    let held = request(service.submit(hint_edit(&alice, None, vec![(995, 1)], 1)));
    entered.recv_timeout(WAIT).unwrap();
    let queued = request(service.submit(hint_edit(&bob, None, vec![(995, 2)], 2)));
    let ledger = service.ledger();

    let started = std::time::Instant::now();
    drop(service);
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "drop is bounded"
    );
    for request in [held, queued] {
        assert_eq!(
            ledger.request_state(request),
            RequestState::Settled(Settlement::Cancelled(Cancellation::Shutdown)),
            "request {} has its terminal at the bound",
            request.get()
        );
        assert!(
            lines
                .lock()
                .unwrap()
                .iter()
                .any(|line| line.contains(&format!(
                    "request={} settlement=cancelled-shutdown",
                    request.get()
                ))),
            "request {} has its own terminal line: {:?}",
            request.get(),
            lines.lock().unwrap()
        );
    }

    release.send(()).unwrap();
    assert!(ledger.wait_writer_exit(WAIT), "the writer stops");
    assert_eq!(
        ledger.request_state(held),
        RequestState::Settled(Settlement::Cancelled(Cancellation::Shutdown)),
        "one terminal per request"
    );
    let reopened = LocalStateService::open(
        root,
        &[LocalStateDomain::BankHints],
        Arc::new(RequestIdMinter::new()),
    );
    for identity in [&alice, &bob] {
        assert_eq!(
            reopened.readback_lease(identity, WAIT).unwrap(),
            None,
            "never committed"
        );
    }
}

/// T5.2 r3: a write whose transaction has begun when the shutdown bound
/// passes is not cancelled: it settles when its COMMIT is known, durable and
/// audited.
#[test]
fn a_write_in_its_transaction_at_the_shutdown_bound_settles_when_it_finishes() {
    let _state_lease = crate::test_process::state();
    let scratch = IsolatedEnv::enter("local-state-r3-shutdown-publishing");
    let root = StateRoot::new(scratch.home.join(".274bot"));
    let service = service(&scratch, &[LocalStateDomain::BankHints]);
    service.set_shutdown_timeout(Duration::from_millis(300));
    let alice = bank_identity("alice");
    assert!(service.readback_lease(&alice, WAIT).unwrap().is_none());
    let (entered_tx, entered) = mpsc::channel();
    let (release, release_rx) = mpsc::channel::<()>();
    let release_rx = std::sync::Mutex::new(release_rx);
    service.set_sql_seams_for_test(sql::Seams {
        point: Some(Arc::new(move |name| {
            if name == "before-commit" {
                let _ = entered_tx.send(());
                let _ = release_rx.lock().unwrap().recv();
            }
        })),
        ..sql::Seams::default()
    });
    let audits = audits(&service);
    let write = request(service.submit(hint_edit(&alice, None, vec![(995, 3)], 3)));
    entered.recv_timeout(WAIT).unwrap();
    let ledger = service.ledger();
    drop(service);
    assert_eq!(ledger.request_state(write), RequestState::Pending);
    release.send(()).unwrap();
    assert!(ledger.wait_writer_exit(WAIT));
    assert!(matches!(
        ledger.request_state(write),
        RequestState::Settled(Settlement::Completed(_))
    ));
    assert_eq!(audits.lock().unwrap().len(), 1);
    let reopened = LocalStateService::open(
        root,
        &[LocalStateDomain::BankHints],
        Arc::new(RequestIdMinter::new()),
    );
    assert_eq!(
        reopened
            .readback_lease(&alice, WAIT)
            .unwrap()
            .unwrap()
            .snapshot()
            .as_bank_hints()
            .unwrap(),
        &[(995, 3)]
    );
}

fn preferences(scratch: &IsolatedEnv) -> Preferences {
    LocalStateService::open_process(StateRoot::new(scratch.home.join(".274bot"))).preferences()
}

/// Open the process preferences over a root whose legacy `panel-ui.json`
/// holds `legacy`, written privately before the open (T5.3 §5.4: the first
/// import reads it at hydration), and wait until they are read.
fn preferences_from(scratch: &IsolatedEnv, legacy: &[u8]) -> Preferences {
    let root = StateRoot::new(scratch.home.join(".274bot"));
    write_private(&root.preferences_file(), legacy);
    let preferences = preferences(scratch);
    preferences.reload().unwrap();
    preferences
}

/// The durable preference document, read again from the database.
fn durable_json(preferences: &Preferences) -> serde_json::Value {
    serde_json::Value::Object(preferences.reload().unwrap().as_ref().clone())
}

/// T5.2 r3 (scope), T5.3: the preferences domain imported from the shared
/// `panel-ui.json`. A write sets one member, keeps every other one (nested
/// values too), and is audited once; no JSON is written.
#[test]
fn a_preference_write_sets_one_member_and_keeps_every_other() {
    let _state_lease = crate::test_process::state();
    let scratch = IsolatedEnv::enter("local-state-r3-preferences");
    let preferences = preferences_from(
        &scratch,
        br#"{"last_focus":"alice","nav":{"allow_teleports":true,"custom":{"keep":7}},"collapsed":{}}"#,
    );
    let audits = audits(preferences.service());
    assert_eq!(preferences.value("last_focus").unwrap(), "alice");

    set_patiently(&preferences, "map_bake", serde_json::json!("always"));

    let mut expected: serde_json::Value = serde_json::json!({
        "last_focus": "alice",
        "nav": {"allow_teleports": true, "custom": {"keep": 7}},
        "collapsed": {},
    });
    expected["map_bake"] = serde_json::json!("always");
    assert_eq!(durable_json(&preferences), expected);
    assert!(
        !StateRoot::new(scratch.home.join(".274bot"))
            .preferences_file()
            .exists(),
        "no JSON is written"
    );
    let audits = audits.lock().unwrap();
    assert_eq!(audits.len(), 1);
    assert_eq!(audits[0].action(), CommitAction::PreferencesWrite);
    assert_eq!(preferences.value("map_bake").unwrap(), "always");
}

/// T5.2 r3: a preference object too long for one row (the panel's
/// per-profile `collapsed` map of a large fleet) is held one row per
/// member, and a change to one member writes only that member; the file
/// keeps its shape and every other member.
#[test]
fn an_oversize_preference_object_is_written_member_by_member() {
    let _state_lease = crate::test_process::state();
    let scratch = IsolatedEnv::enter("local-state-r3-preferences-wide");
    let collapsed: serde_json::Map<String, serde_json::Value> = (0..80)
        .map(|n| {
            (
                format!("profile_{n:03}"),
                serde_json::json!({"status": false, "resource": true, "profile": false,
                    "script": true, "debug": false, "log": false, "parameters": true}),
            )
        })
        .collect();
    let document = serde_json::json!({"collapsed": collapsed, "lowmem": true});
    assert!(document["collapsed"].to_string().len() > api::local_state::MAX_PREF_VALUE_BYTES);
    let preferences = preferences_from(&scratch, &serde_json::to_vec_pretty(&document).unwrap());
    let lease = preferences.service().lease(preferences.identity()).unwrap();
    let rows = lease.snapshot().as_preferences().unwrap();
    assert_eq!(rows.len(), 81, "one row per profile, plus lowmem");
    assert!(rows.iter().any(|row| row.key() == "collapsed.profile_007"));

    let audits = audits(preferences.service());
    update_patiently(&preferences, |document| {
        document["collapsed"]["profile_007"]["status"] = serde_json::json!(true);
        Ok(())
    });
    assert_eq!(audits.lock().unwrap().len(), 1, "one member, one write");
    let mut expected = document.clone();
    expected["collapsed"]["profile_007"]["status"] = serde_json::json!(true);
    assert_eq!(durable_json(&preferences), expected);
}

/// A preferences document holding a 600-member `order` object (too long
/// for one row, like the saved Load order of a large fleet): member
/// `"<uid>"` gets `key(index)`, members `skip` leaves out are absent.
fn order_document(
    key: impl Fn(i64) -> i64,
    skip: impl Fn(i64) -> bool,
    extra: Option<i64>,
) -> serde_json::Value {
    let order: serde_json::Map<String, serde_json::Value> = (0..600)
        .filter(|index| !skip(*index))
        .chain(extra)
        .map(|index| {
            (
                (274_000_001 + index).to_string(),
                serde_json::json!(key(index)),
            )
        })
        .collect();
    serde_json::json!({"lowmem": true, "order": order})
}

/// The stored order before the updates below: keys 7, 14, 21, …
fn order_before() -> serde_json::Value {
    order_document(|index| (index + 1) * 7, |_| false, None)
}

/// A renumbered order: every key changes, member 5 is removed and member
/// 700 is added.
fn order_renumbered() -> serde_json::Value {
    order_document(|index| (index + 1) << 16, |index| index == 5, Some(700))
}

fn preferences_with_order(scratch: &IsolatedEnv) -> Preferences {
    let before = order_before();
    assert!(before["order"].to_string().len() > api::local_state::MAX_PREF_VALUE_BYTES);
    preferences_from(scratch, &serde_json::to_vec_pretty(&before).unwrap())
}

/// F2 r3 (review R4), T5.3: an update of an object too long for one row
/// that changes, removes and adds many members (a renumbered Load order)
/// is one edit and one database commit: every member lands together, at
/// one new version.
#[test]
fn a_many_member_update_is_one_file_replacement() {
    let _state_lease = crate::test_process::state();
    let scratch = IsolatedEnv::enter("local-state-f2-one-replacement");
    let preferences = preferences_with_order(&scratch);
    let audits = audits(preferences.service());
    let before = preferences.snapshot().unwrap().version();
    let after = order_renumbered();
    update_patiently(&preferences, |document| {
        document.insert("order".into(), after["order"].clone());
        Ok(())
    });
    assert_eq!(audits.lock().unwrap().len(), 1, "one durable commit");
    let audit = audits.lock().unwrap()[0].clone();
    assert_eq!(audit.previous(), before);
    assert_eq!(durable_json(&preferences)["order"], after["order"]);
    assert_eq!(preferences.value("order"), Some(after["order"].clone()));
}

/// F2 r3 (review R4), T5.3: a many-member update whose commit fails, or
/// whose writer dies inside the open write transaction (an interrupted
/// renumbering), leaves the durable document exactly as before: no member
/// of it lands. The next update lands whole.
#[test]
fn a_failed_or_interrupted_many_member_update_changes_nothing() {
    let _state_lease = crate::test_process::state();
    let scratch = IsolatedEnv::enter("local-state-f2-all-or-nothing");
    let preferences = preferences_with_order(&scratch);
    // 0: the database refuses the commit; 1: the writer dies in the
    // transaction; then the real commit.
    let step = Arc::new(AtomicUsize::new(0));
    let fault_step = Arc::clone(&step);
    let point_step = Arc::clone(&step);
    preferences.service().set_sql_seams_for_test(sql::Seams {
        commit_fault: Some(Arc::new(move || {
            (fault_step.load(Ordering::SeqCst) == 0).then_some(sql::CommitFault::Full)
        })),
        point: Some(Arc::new(move |name| {
            if name == "before-commit" && point_step.load(Ordering::SeqCst) == 1 {
                panic!("injected writer death mid-renumbering");
            }
        })),
        ..sql::Seams::default()
    });
    let after = order_renumbered();
    let renumber = |document: &mut serde_json::Map<String, serde_json::Value>| {
        document.insert("order".into(), after["order"].clone());
        Ok(())
    };
    for (failure, case) in [(0, "refused"), (1, "interrupted")] {
        step.store(failure, Ordering::SeqCst);
        let error = preferences.update(renumber).expect_err(case);
        assert!(
            !transient_wait(&error),
            "{case}: a load timeout is not the injected failure: {error}"
        );
        assert_eq!(
            durable_json(&preferences)["order"],
            order_before()["order"],
            "{case}: durable rows"
        );
    }
    step.store(2, Ordering::SeqCst);
    update_patiently(&preferences, renumber);
    assert_eq!(durable_json(&preferences)["order"], after["order"]);
}

/// F2 r3 (review R4), T5.3: two writers (two services on one root, as two
/// processes) each replace every member of the large object while the
/// first holds its write transaction open. The second waits (the guard and
/// the database lock are taken without waiting and retried), conflicts,
/// reads and merges again, and the database ends holding its object whole:
/// never a mix of the two, and both updates report durable.
#[test]
fn concurrent_many_member_updates_never_mix() {
    let _state_lease = crate::test_process::state();
    let scratch = IsolatedEnv::enter("local-state-f2-two-writers");
    let first = preferences_with_order(&scratch);
    let second = preferences(&scratch);
    second.reload().unwrap();
    let (entered_tx, entered) = mpsc::channel();
    let (release, release_rx) = mpsc::channel::<()>();
    let release_rx = std::sync::Mutex::new(release_rx);
    let entered_tx = std::sync::Mutex::new(entered_tx);
    let held = AtomicBool::new(false);
    first.service().set_sql_seams_for_test(sql::Seams {
        point: Some(Arc::new(move |name| {
            if name == "before-commit" && !held.swap(true, Ordering::SeqCst) {
                let _ = entered_tx.lock().unwrap().send(());
                let _ = release_rx.lock().unwrap().recv();
            }
        })),
        ..sql::Seams::default()
    });
    let first_order = order_document(|index| (index + 1) * 1000, |_| false, None);
    let second_order = order_renumbered();
    let update = |preferences: Preferences, document: serde_json::Value| {
        std::thread::spawn(move || {
            update_patiently(&preferences, |held| {
                held.insert("order".into(), document["order"].clone());
                Ok(())
            });
        })
    };
    let a = update(first.clone(), first_order);
    entered.recv_timeout(WAIT).unwrap();
    let b = update(second.clone(), second_order.clone());
    std::thread::sleep(Duration::from_millis(300));
    release.send(()).unwrap();
    a.join().unwrap();
    b.join().unwrap();
    assert_eq!(
        durable_json(&first)["order"],
        second_order["order"],
        "the later writer's object, whole"
    );
}

/// T5.2 r3 (grok P2, the same rule in every domain), T5.3: a legacy
/// preferences file with a member the database cannot hold as a row is
/// never imported in part: the preferences are refused with the recovery
/// commands, a write fails, and the file keeps every byte.
#[test]
fn a_preferences_file_the_service_cannot_hold_whole_is_never_rewritten() {
    let _state_lease = crate::test_process::state();
    let scratch = IsolatedEnv::enter("local-state-r3-preferences-refused");
    let root = StateRoot::new(scratch.home.join(".274bot"));
    let document = br#"{"last_focus":"alice","not a key":1}"#;
    write_private(&root.preferences_file(), document);
    let preferences = preferences(&scratch);
    let error = preferences.reload().unwrap_err().to_string();
    assert!(
        error.contains("durable preferences failure"),
        "the import is refused: {error}"
    );
    assert!(preferences
        .set("map_bake", serde_json::json!("always"))
        .is_err());
    assert_eq!(std::fs::read(root.preferences_file()).unwrap(), document);
}

/// T5.2 r3 (scope, sol F1 by construction): a frontend's loadouts form and
/// a slot-side edit write through the one process service: they never
/// overwrite each other. The form's stale save conflicts, and saving again
/// keeps both rows.
#[test]
fn a_form_save_and_a_slot_edit_through_one_service_keep_both_rows() {
    let _state_lease = crate::test_process::state();
    let scratch = IsolatedEnv::enter("local-state-r3-form-and-slot");
    let root = StateRoot::new(scratch.home.join(".274bot"));
    let service = LocalStateService::open_process(root.clone());
    let mut form = form(&service);
    assert!(form.loadouts().is_empty());
    let version = service.lease(&loadouts_identity()).unwrap().version();
    completed(
        &service,
        request(service.submit(loadout_edit(Some(version), Loadout::new("slot-row")))),
    );

    form.upsert(Loadout::new("form-row"));
    // The conflict is decided without waiting for durability, but a worker
    // starved by machine load answers the first save with one of the two
    // bounded waits instead: retry those, keep the first decisive answer.
    let start = std::time::Instant::now();
    let error = loop {
        match form.save() {
            Err(error)
                if transient_save_wait(&error) && start.elapsed() < Duration::from_secs(60) =>
            {
                std::thread::yield_now()
            }
            result => break result.unwrap_err(),
        }
    };
    assert!(error.contains("conflict"), "{error}");
    save_patiently(&mut form);
    let rows: Vec<Loadout> =
        serde_json::from_slice(&std::fs::read(root.loadouts_file()).unwrap()).unwrap();
    let names: Vec<&str> = rows.iter().map(|row| row.name.as_str()).collect();
    assert_eq!(names, ["slot-row", "form-row"]);
    assert_eq!(form.names(), ["slot-row", "form-row"]);
}

/// T5.2 r4 (Sol r3 F1, `review_r3_sol::rename_failure_must_not_delete_the_saved_loadout`
/// made permanent): a rename through the service-backed form never leaves
/// the file without the loadout. Whichever file publication of the save
/// fails, the file holds the row under its old name (the save failed and
/// nothing was written) or under its new one, with its gear, in its place.
/// Before r4 a rename was a removal then an upsert, two publications: a
/// failure of the second left neither name on disk.
#[test]
fn a_failed_rename_never_loses_the_saved_loadout() {
    let _state_lease = crate::test_process::state();
    for failing in 0..2 {
        let scratch = IsolatedEnv::enter(&format!("local-state-r4-rename-failure-{failing}"));
        let root = StateRoot::new(scratch.home.join(".274bot"));
        let path = root.loadouts_file();
        let original = Loadout::new("original").with_carry("Lobster", 8);
        write_loadouts(&path, &[original.clone(), Loadout::new("other")]);
        let before = std::fs::read(&path).unwrap();
        let service = service(&scratch, &[LocalStateDomain::LoadoutsReferences]);
        let mut editor = form(&service);
        let writes = AtomicUsize::new(0);
        service.set_write_hook(Arc::new(move |path, bytes| {
            if writes.fetch_add(1, Ordering::SeqCst) == failing {
                return Err(std::io::Error::other("publication fails (disk full)"));
            }
            vault::write_private_file(path, bytes)
        }));
        let mut renamed = original.clone();
        renamed.name = "renamed".into();
        assert!(editor.replace_at(0, renamed.clone()));
        let saved = editor.save();
        let saved = match saved {
            Err(error) if error.contains("not durable within") => {
                // The frontend's timeout does not mean publication failed.
                // Wait for that same request, without submitting the rename again.
                let request = {
                    let state = service.shared.state.lock();
                    assert_eq!(state.requests.len(), 1, "a rename is one publication");
                    *state.requests.keys().next().unwrap()
                };
                match settlement(&service, request) {
                    Settlement::Completed(_) => Ok(()),
                    Settlement::Failed(code) => Err(format!("{code:?}")),
                    other => panic!("the rename did not complete or fail: {other:?}"),
                }
            }
            other => other,
        };
        if saved.is_err() {
            // Both frontends discard the form's unsaved rows on a failure.
            editor.discard();
        }
        let rows: Vec<Loadout> = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        match &saved {
            Err(error) => {
                assert_eq!(
                    std::fs::read(&path).unwrap(),
                    before,
                    "publication {failing} failed ({error}): the file is untouched"
                );
                assert_eq!(editor.names(), ["original", "other"]);
            }
            Ok(()) => {
                assert_eq!(
                    rows,
                    [renamed.clone(), Loadout::new("other")],
                    "publication {failing}: the renamed row keeps its gear and its place"
                );
                assert_eq!(editor.names(), ["renamed", "other"]);
            }
        }
        assert!(
            rows.contains(&original) || rows.contains(&renamed),
            "publication {failing}: a failed rename deleted the only persisted copy: {rows:?}"
        );
    }
}

/// T5.2 r4 (F0 amendment 2): a rename edit replaces its row in place in
/// one publication, with one audit. A source that is gone is a conflict
/// and a name another row has is a duplicate: both are refused when the
/// edit is decided, and nothing is written or inserted.
#[test]
fn a_rename_edit_replaces_its_row_in_place_or_is_refused_whole() {
    let _state_lease = crate::test_process::state();
    let scratch = IsolatedEnv::enter("local-state-r4-rename-edit");
    let root = StateRoot::new(scratch.home.join(".274bot"));
    let path = root.loadouts_file();
    write_loadouts(
        &path,
        &[
            Loadout::new("melee").with_carry("Lobster", 8),
            Loadout::new("range"),
            Loadout::new("tank"),
        ],
    );
    let service = service(&scratch, &[LocalStateDomain::LoadoutsReferences]);
    let audits = audits(&service);
    let rename = |from: &str, to: Loadout| {
        DomainEdit::Loadout(LoadoutEdit::rename(loadouts_identity(), None, from, to).unwrap())
    };
    let names = || {
        let rows: Vec<Loadout> = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        rows.into_iter().map(|row| row.name).collect::<Vec<_>>()
    };
    assert!(service.readback_lease(&loadouts_identity(), WAIT).is_ok());

    let moved = Loadout::new("melee2").with_carry("Lobster", 9);
    completed(
        &service,
        request(service.submit(rename("melee", moved.clone()))),
    );
    assert_eq!(names(), ["melee2", "range", "tank"]);
    assert_eq!(audits.lock().unwrap().len(), 1, "one rename, one write");

    let bytes = std::fs::read(&path).unwrap();
    let taken = request(service.submit(rename("range", Loadout::new("tank"))));
    assert_eq!(
        settlement(&service, taken),
        Settlement::Failed(ErrorCode::Validation {
            domain: LocalStateDomain::LoadoutsReferences,
            reason: ValidationReason::DuplicateId,
        })
    );
    let gone = request(service.submit(rename("melee", Loadout::new("again"))));
    assert!(
        matches!(
            settlement(&service, gone),
            Settlement::Failed(ErrorCode::Conflict { .. })
        ),
        "a rename of a row that is gone conflicts and inserts nothing"
    );
    assert_eq!(std::fs::read(&path).unwrap(), bytes, "nothing written");
    assert_eq!(audits.lock().unwrap().len(), 1);
}

type NoticeLines = Arc<std::sync::Mutex<Vec<(Level, String)>>>;

fn loadouts_service_with_notices(scratch: &IsolatedEnv) -> (Arc<LocalStateService>, NoticeLines) {
    let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
    let sink = Arc::clone(&seen);
    let state = State {
        hooks: Hooks {
            read_refusal: Some(Arc::new(move |level, line| {
                sink.lock().unwrap().push((level, line.to_string()));
            })),
            ..Default::default()
        },
        ..Default::default()
    };
    let service = LocalStateService::open_with_state(
        StateRoot::new(scratch.home.join(".274bot")),
        &[LocalStateDomain::LoadoutsReferences],
        Arc::new(RequestIdMinter::new()),
        state,
    );
    (service, seen)
}

/// Preference recovery notices emitted through the service's test-only hook.
fn quarantine_lines(service: &LocalStateService) -> NoticeLines {
    let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
    let sink = Arc::clone(&seen);
    service.set_notice_hook(Arc::new(move |level, line| {
        sink.lock().unwrap().push((level, line.to_string()))
    }));
    seen
}

/// The names beside `path` that start with its name, except the lock.
fn siblings(path: &std::path::Path) -> Vec<String> {
    let name = path.file_name().unwrap().to_string_lossy().into_owned();
    let mut found: Vec<String> = std::fs::read_dir(path.parent().unwrap())
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|entry| entry.starts_with(&name) && *entry != format!("{name}.lock"))
        .collect();
    found.sort();
    found
}

/// A preferences service on its own root under `scratch`, opened over
/// `legacy` (written privately before the open, when given). Its first
/// hydration waits until `hooks` ran on the service, so a test's notice
/// hook sees the import's own lines.
fn preferences_at(
    scratch: &IsolatedEnv,
    case: &str,
    legacy: Option<&[u8]>,
    hooks: impl FnOnce(&LocalStateService),
) -> Preferences {
    let root = StateRoot::new(scratch.home.join(case).join(".274bot"));
    std::fs::create_dir_all(root.path()).unwrap();
    if let Some(bytes) = legacy {
        write_private(&root.preferences_file(), bytes);
    }
    let gate = Arc::new(AtomicBool::new(false));
    let open = Arc::clone(&gate);
    let service = LocalStateService::open_process_with_read_stall_for_test(
        root,
        Arc::new(move || {
            let deadline = Instant::now() + WAIT;
            while !open.load(Ordering::SeqCst) && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(1));
            }
        }),
    );
    hooks(&service);
    gate.store(true, Ordering::SeqCst);
    service.preferences()
}

/// A runtime settings service and its account/content-bound file.
fn runtime_settings_at(
    scratch: &IsolatedEnv,
    case: &str,
) -> (Arc<LocalStateService>, DomainIdentity, std::path::PathBuf) {
    let root = StateRoot::new(scratch.home.join(case).join(".274bot"));
    std::fs::create_dir_all(root.path()).unwrap();
    let identity = identity(LocalStateDomain::ScriptRuntimeSettings, "alice");
    let path = root.script_runtime_settings_file(
        identity.account().profile(),
        identity.account().account(),
        identity.scope().revision().as_i32(),
        identity.scope().content(),
    );
    let service = LocalStateService::open(
        root,
        &[LocalStateDomain::ScriptRuntimeSettings],
        Arc::new(RequestIdMinter::new()),
    );
    (service, identity, path)
}

#[test]
fn runtime_settings_are_written_from_a_cold_store_and_read_back() {
    let scratch = IsolatedEnv::enter("local-state-runtime-settings-round-trip");
    let (service, identity, path) = runtime_settings_at(&scratch, "settings");
    let edit = RuntimeSettingsEdit::try_set(
        identity.clone(),
        None,
        "catalog:ChickenKiller",
        "buryBones",
        "false",
    )
    .unwrap();
    let request = request(service.submit(DomainEdit::ScriptRuntimeSettings(edit)));
    completed(&service, request);

    let document: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(
        document,
        serde_json::json!({
            "schema_version": 1,
            "profile": PROFILE,
            "account": "alice",
            "revision": 289,
            "content": CONTENT,
            "rows": [{
                "namespace": "catalog:ChickenKiller",
                "key": "buryBones",
                "raw": "false"
            }]
        })
    );

    let lease = service
        .readback_lease(&identity, WAIT)
        .unwrap()
        .expect("the write created a settings file");
    let rows = lease
        .snapshot()
        .as_script_runtime_settings()
        .expect("script settings snapshot");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].namespace(), "catalog:ChickenKiller");
    assert_eq!(rows[0].key(), "buryBones");
    assert_eq!(rows[0].raw(), "false");
}

/// WalkTo saves `customTile`, and its change listener saves `destination`
/// in the same call; a MapPicker row is written just before. All three
/// writes queue back to back. After they settle, the file and every read
/// hold all three rows (the restart in the WalkTo live cell found only
/// `destination`).
#[test]
fn back_to_back_runtime_settings_writes_keep_every_row() {
    let scratch = IsolatedEnv::enter("local-state-runtime-settings-back-to-back");
    let (service, identity, path) = runtime_settings_at(&scratch, "settings");
    service
        .readback_lease(&identity, WAIT)
        .expect("an absent file is ready");
    let rows = [
        ("MapPicker", "showBasemap", "false"),
        ("WalkTo", "customTile", "3110,3106,0"),
        ("WalkTo", "destination", "Map pick"),
    ];
    let requests: Vec<_> = rows
        .iter()
        .map(|(namespace, key, raw)| {
            let edit =
                RuntimeSettingsEdit::try_set(identity.clone(), None, namespace, key, raw).unwrap();
            request(service.submit(DomainEdit::ScriptRuntimeSettings(edit)))
        })
        .collect();
    for request in requests {
        completed(&service, request);
    }

    let document: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(
        document["rows"],
        serde_json::json!([
            { "namespace": "MapPicker", "key": "showBasemap", "raw": "false" },
            { "namespace": "WalkTo", "key": "customTile", "raw": "3110,3106,0" },
            { "namespace": "WalkTo", "key": "destination", "raw": "Map pick" },
        ]),
        "the file keeps every row"
    );
    for (namespace, key, raw) in rows {
        assert_eq!(
            LocalStatePort::runtime_setting_raw(&*service, &identity, namespace, key).unwrap(),
            Some(raw.to_string()),
            "{namespace}.{key} reads from the settled cache"
        );
    }
    let lease = service
        .readback_lease(&identity, WAIT)
        .unwrap()
        .expect("the writes created a settings file");
    assert_eq!(
        lease.snapshot().as_script_runtime_settings().unwrap().len(),
        3,
        "a later readback keeps every row"
    );
    for (namespace, key, raw) in rows {
        assert_eq!(
            LocalStatePort::runtime_setting_raw(&*service, &identity, namespace, key).unwrap(),
            Some(raw.to_string()),
            "{namespace}.{key} reads after a readback"
        );
    }
}

#[test]
fn malformed_runtime_settings_are_kept_and_quarantined_by_a_save() {
    let scratch = IsolatedEnv::enter("local-state-runtime-settings-quarantine");
    for (case, bytes) in [
        ("not-json", b"{not valid json".to_vec()),
        ("not-an-object", b"[1, 2, 3]".to_vec()),
    ] {
        let (service, identity, path) = runtime_settings_at(&scratch, case);
        write_private(&path, &bytes);
        let notices = quarantine_lines(&service);

        assert!(service.readback_lease(&identity, WAIT).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        assert_eq!(siblings(&path), std::slice::from_ref(&name));

        let edit = RuntimeSettingsEdit::try_set(
            identity.clone(),
            None,
            "catalog:ChickenKiller",
            "buryBones",
            "false",
        )
        .unwrap();
        let request = request(service.submit(DomainEdit::ScriptRuntimeSettings(edit)));
        completed(&service, request);

        let names = siblings(&path);
        assert_eq!(names.len(), 2, "{case}: {names:?}");
        let quarantined = path.with_file_name(&names[1]);
        assert!(
            names[1].starts_with(&format!("{name}.corrupt-")),
            "{case}: {names:?}"
        );
        assert_eq!(std::fs::read(&quarantined).unwrap(), bytes);
        let lease = service.readback_lease(&identity, WAIT).unwrap().unwrap();
        let rows = lease.snapshot().as_script_runtime_settings().unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].raw(), "false");
        let notices = notices.lock().unwrap();
        assert_eq!(notices.len(), 2, "{case}: {notices:?}");
        assert_eq!(notices[0].0, Level::Warn);
        assert_eq!(notices[1].0, Level::Info);
        assert!(
            notices[1].1.starts_with(
                "audit domain=script-runtime-settings action=script-runtime-settings-quarantine"
            ),
            "{case}: {notices:?}"
        );
    }
}

#[test]
fn quarantine_copy_flushes_its_candidate() {
    let _state_lease = crate::test_process::state();
    let scratch = IsolatedEnv::enter("local-state-quarantine-copy-flush");
    let path = scratch.home.join(".274bot").join("panel-ui.json");
    let bytes = b"quarantine contents".to_vec();
    write_private(&path, &bytes);

    let quarantined = files::quarantine_copy(&path, &bytes, "regression").unwrap();

    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    assert_eq!(std::fs::read(quarantined).unwrap(), bytes);
}

/// T5.2 r4, T5.3 §5.1 (Q3): a legacy preferences file whose content is
/// malformed (not JSON, not UTF-8, JSON that is not an object) is kept
/// byte for byte in a private `panel-ui.json.corrupt-<time>` copy, synced
/// with its directory entry, before default preferences are committed
/// with their ledger. The source itself is left as it is; the operator is
/// told (a warning) and one info audit line names the action. A file that
/// is also truncated and names a field twice is the truncated one first:
/// it is refused as not JSON, not as the duplicate, and quarantined the
/// same way.
#[test]
fn a_malformed_preferences_file_is_quarantined_byte_for_byte_and_started_fresh() {
    let _state_lease = crate::test_process::state();
    let scratch = IsolatedEnv::enter("local-state-r4-preferences-quarantine");
    for (case, bytes) in [
        ("not-json", b"{not valid json".to_vec()),
        ("not-utf8", vec![b'{', 0xff, 0xfe, b'}']),
        ("not-an-object", b"[1, 2, 3]".to_vec()),
        ("duplicate-and-truncated", br#"{"a":1,"a":"#.to_vec()),
        (
            "duplicate-in-a-non-object-root",
            br#"[{"a":1,"a":2}]"#.to_vec(),
        ),
    ] {
        let mut notices = None;
        let preferences = preferences_at(&scratch, case, Some(&bytes), |service| {
            notices = Some(quarantine_lines(service));
        });
        let notices = notices.unwrap();
        let path = preferences.service().root().preferences_file();
        assert!(preferences.reload().unwrap().is_empty(), "{case}: defaults");
        assert_eq!(
            std::fs::read(&path).unwrap(),
            bytes,
            "{case}: kept in place"
        );

        let found = siblings(&path);
        assert_eq!(found.len(), 2, "{case}: {found:?}");
        assert!(
            found[1].starts_with("panel-ui.json.corrupt-"),
            "{case}: {found:?}"
        );
        let quarantined = path.with_file_name(&found[1]);
        assert_eq!(
            std::fs::read(&quarantined).unwrap(),
            bytes,
            "{case}: the quarantine holds the exact bytes"
        );
        set_patiently(&preferences, "map_bake", serde_json::json!("always"));
        assert_eq!(preferences.value("map_bake").unwrap(), "always");
        let notices = notices.lock().unwrap().clone();
        let warn = notices
            .iter()
            .find(|(level, _)| *level == Level::Warn)
            .unwrap_or_else(|| panic!("{case}: {notices:?}"));
        assert!(
            warn.1.contains(&quarantined.display().to_string()),
            "{case}: {notices:?}"
        );
        let audits: Vec<_> = notices
            .iter()
            .filter(|(level, line)| *level == Level::Info && line.starts_with("audit "))
            .collect();
        assert_eq!(audits.len(), 1, "{case}: {notices:?}");
        assert!(
            audits[0]
                .1
                .contains("action=preferences-defaults-after-quarantine"),
            "{case}: {notices:?}"
        );
    }
}

/// T5.2 r4, T5.3 §4.3: the import runs under the preferences guard
/// (`panel-ui.json.lock`): while another writer holds it, the database
/// does not open, the malformed file stays where it is, and nothing is
/// copied; once released, the copy is kept and defaults start.
#[test]
fn a_preferences_quarantine_waits_for_the_files_lock() {
    let _state_lease = crate::test_process::state();
    let scratch = IsolatedEnv::enter("local-state-r4-preferences-quarantine-lock");
    let root = StateRoot::new(scratch.home.join("locked").join(".274bot"));
    let path = root.preferences_file();
    let bytes = b"{not valid json".to_vec();
    write_private(&path, &bytes);
    let lock = files::lock_for_write(&path, std::time::Instant::now() + WAIT, &|| false).unwrap();
    let preferences = preferences_at(&scratch, "locked", None, |_| {});
    std::thread::sleep(Duration::from_millis(300));
    assert!(preferences.document().unwrap_err().is_not_ready());
    assert_eq!(
        std::fs::read(&path).unwrap(),
        bytes,
        "not moved under another's lock"
    );
    assert_eq!(siblings(&path), ["panel-ui.json"]);
    drop(lock);
    assert!(preferences.reload().unwrap().is_empty());
    let found = siblings(&path);
    assert_eq!(found.len(), 2, "{found:?}");
    assert_eq!(
        std::fs::read(path.with_file_name(&found[1])).unwrap(),
        bytes
    );
}

/// The refusal the first open of `preferences` ends with.
fn refused_snapshot(preferences: &Preferences) -> PreferenceError {
    let deadline = Instant::now() + WAIT;
    loop {
        match preferences.snapshot() {
            Err(error) if error.is_not_ready() => {}
            Err(error) => return error,
            Ok(_) => panic!("the preferences opened"),
        }
        assert!(Instant::now() < deadline, "never settled");
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// T5.2 r4, r5 (Sol r4 F2), T5.3 §5.1: only malformed content is
/// quarantined. A legacy file refused for its size or holding a member this
/// build cannot hold as a row (`nav.v2`: a member name with a `.`) is never
/// moved, replaced or partly imported: the preferences are refused with the
/// recovery commands, every byte stays, and no copy appears.
#[test]
fn preferences_refused_for_size_or_an_unrepresentable_member_are_never_quarantined() {
    let _state_lease = crate::test_process::state();
    let scratch = IsolatedEnv::enter("local-state-r4-preferences-refused");
    let oversize = vec![b'x'; MAX_PREFERENCES_BYTES as usize + 1];
    let unrepresentable = br#"{"last_focus":"alice","nav.v2":{"zones":"all"}}"#.to_vec();
    for (case, bytes) in [("oversize", oversize), ("unrepresentable", unrepresentable)] {
        let mut notices = None;
        let preferences = preferences_at(&scratch, case, Some(&bytes), |service| {
            notices = Some(quarantine_lines(service));
        });
        let path = preferences.service().root().preferences_file();
        let text = refused_snapshot(&preferences).to_string();
        assert!(
            text.contains("state preferences recover --defaults"),
            "{case}: {text}"
        );
        assert!(preferences
            .set("map_bake", serde_json::json!("always"))
            .is_err());
        assert_eq!(std::fs::read(&path).unwrap(), bytes, "{case}: kept");
        assert_eq!(siblings(&path), ["panel-ui.json"], "{case}");
        assert!(notices.unwrap().lock().unwrap().is_empty(), "{case}");
    }
}

/// T5.2 r5 (Sol r4 F2), T5.3: the preferences rule is lossless
/// representability, not a version or known-member check. A
/// `schema_version` member another build wrote is a member like any other:
/// the import keeps it and every member this build does not know, a save
/// adds its own, and nothing is refused or copied aside.
#[test]
fn a_preferences_version_marker_is_kept_by_a_save_like_any_other_member() {
    let _state_lease = crate::test_process::state();
    let scratch = IsolatedEnv::enter("local-state-r5-preferences-version-marker");
    let preferences = preferences_at(
        &scratch,
        "marker",
        Some(br#"{"schema_version":99,"last_focus":"alice","future_preference":{"enabled":true}}"#),
        |_| {},
    );
    preferences.reload().unwrap();
    assert_eq!(preferences.value("last_focus").unwrap(), "alice");
    preferences
        .set("map_bake", serde_json::json!("always"))
        .expect("a version marker and an unknown member do not refuse a save");
    assert_eq!(
        durable_json(&preferences),
        serde_json::json!({
            "schema_version": 99,
            "last_focus": "alice",
            "future_preference": {"enabled": true},
            "map_bake": "always",
        }),
        "every member is kept, the marker and the unknown one too"
    );
    let path = preferences.service().root().preferences_file();
    assert_eq!(siblings(&path), ["panel-ui.json.migrated"]);
}

/// T5.2 r4, T5.3 §5.1: a legacy file refused for its permissions is never
/// quarantined, whatever it holds: group/other-writable or unreadable,
/// even with malformed content, it is refused, no ledger or defaults are
/// saved, and it keeps every byte.
#[cfg(unix)]
#[test]
fn preferences_refused_for_permissions_are_never_quarantined() {
    let _state_lease = crate::test_process::state();
    if running_as_root() {
        return;
    }
    let scratch = IsolatedEnv::enter("local-state-r4-preferences-permissions");
    for mode in [0o666, 0o000] {
        let case = format!("mode-{mode:o}");
        let root = StateRoot::new(scratch.home.join(&case).join(".274bot"));
        let path = root.preferences_file();
        let bytes = b"{not valid json".to_vec();
        write_private(&path, &bytes);
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode)).unwrap();
        let mut notices = None;
        let preferences = preferences_at(&scratch, &case, None, |service| {
            notices = Some(quarantine_lines(service));
        });
        let refused = refused_snapshot(&preferences);
        assert!(
            refused
                .to_string()
                .contains("No source ledger or defaults were saved"),
            "mode {mode:o}: {refused}"
        );
        assert!(
            preferences
                .set("map_bake", serde_json::json!("always"))
                .is_err(),
            "mode {mode:o}"
        );
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), bytes, "mode {mode:o}: kept");
        assert_eq!(siblings(&path), ["panel-ui.json"], "mode {mode:o}");
        assert!(notices.unwrap().lock().unwrap().is_empty(), "mode {mode:o}");
    }
}

/// T5.3 §6 (S4 headed check D-2): a setting saved while the state database
/// is refused for its permissions fails at once with the refusal's own
/// path, cause and remedy, not only the code ("store I/O error"), so the
/// setting that tried to save tells the operator what to fix.
#[cfg(unix)]
#[test]
fn a_save_refused_by_database_permissions_names_the_cause_and_the_remedy() {
    let _state_lease = crate::test_process::state();
    if running_as_root() {
        return;
    }
    let scratch = IsolatedEnv::enter("local-state-s4-refused-save-cause");
    let case = "group-writable";
    let database = {
        let preferences = preferences_at(&scratch, case, None, |_| {});
        let Update::Queued(mut initial) = preferences
            .begin_set("map_bake", serde_json::json!("always"))
            .expect("the first save is accepted")
        else {
            panic!("the first save is queued");
        };
        initial
            .wait(WAIT)
            .expect("the first save settles")
            .expect("the first save creates the database");
        preferences.service().root().database_file()
    };
    std::fs::set_permissions(&database, std::fs::Permissions::from_mode(0o664)).unwrap();
    let preferences = preferences_at(&scratch, case, None, |_| {});
    let refused = refused_snapshot(&preferences).to_string();
    let error = match preferences.begin_set("map_bake", serde_json::json!("never")) {
        Err(error) => error.to_string(),
        Ok(_) => panic!("a refused store accepted a save"),
    };
    std::fs::set_permissions(&database, std::fs::Permissions::from_mode(0o600)).unwrap();
    let shown = database.display().to_string();
    for needle in [
        shown.as_str(),
        "writable by other users",
        &format!("chmod 600 {shown}"),
    ] {
        assert!(
            refused.contains(needle),
            "snapshot lacks {needle:?}: {refused}"
        );
        assert!(error.contains(needle), "save lacks {needle:?}: {error}");
    }
    assert!(!error.contains("store I/O error"), "{error}");
}

/// FU:15 (S3a): the duplicate-field check costs no more than the parse it
/// checks. This document — a 768 KiB first field name and 120 object
/// levels, the shape the round-1 review measured, where the checked
/// reader's peak was 188,772,239 bytes against the plain reader's 829,271
/// — holds a value's position as a chain of the parsing frames' own
/// segments, so no path text is built while the document parses and every
/// field name is still owned exactly once. The bound: the checked parse's
/// peak is the plain `serde_json` read's peak plus one small step for the
/// parser's own bookkeeping, never a copy of the path per level.
#[test]
fn a_deep_document_parses_within_the_plain_reader_s_peak_allocation() {
    let mut raw = String::new();
    raw.push_str("{\"");
    raw.push_str(&"k".repeat(768 * 1024));
    raw.push_str("\":");
    for level in 0..120 {
        raw.push('{');
        raw.push_str(&format!("\"{level}\":"));
    }
    raw.push('1');
    raw.push_str(&"}".repeat(121));
    assert!(raw.len() > 786_000, "the document is the reviewed shape");
    let plain = allocation_counter::measure(|| {
        drop(std::hint::black_box(
            serde_json::from_str::<serde_json::Value>(&raw).unwrap(),
        ));
    });
    let checked = allocation_counter::measure(|| {
        drop(std::hint::black_box(
            files::object_without_duplicate_fields(&raw).unwrap(),
        ));
    });
    assert_eq!(
        checked.bytes_current, 0,
        "the checked parse leaves nothing allocated"
    );
    assert!(
        checked.bytes_max <= plain.bytes_max + 64 * 1024,
        "peak {} bytes against the plain read's {}",
        checked.bytes_max,
        plain.bytes_max
    );
}

/// A refused loadouts file stays byte-for-byte intact and is never renamed,
/// quarantined, leased or written. The refusal hook is installed before eager
/// hydration starts, at the same boundary as the Lifecycle warning.
fn assert_loadouts_refused_and_untouched(
    service: &LocalStateService,
    path: &std::path::Path,
    bytes: &[u8],
    notices: &std::sync::Mutex<Vec<(Level, String)>>,
    reason: FailureReason,
    message: &str,
) {
    let refused = ErrorCode::Failed {
        domain: LocalStateDomain::LoadoutsReferences,
        reason,
    };
    let identity = loadouts_identity();
    assert_eq!(service.readback_lease(&identity, WAIT), Err(refused));
    assert_eq!(service.lease(&identity), Err(refused));
    assert_eq!(
        service.submit(loadout_edit(None, Loadout::new("new"))),
        SubmitReceipt::Refused(refused)
    );
    let _ = service.readback_lease(&identity, WAIT);
    assert_eq!(std::fs::read(path).unwrap(), bytes);
    assert_eq!(siblings(path), ["loadouts.json"]);
    let lines = notices.lock().unwrap().clone();
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert_eq!(lines[0].0, Level::Warn, "{lines:?}");
    assert!(
        lines[0].1.contains(&path.display().to_string()) && lines[0].1.contains(message),
        "{lines:?}"
    );
}

/// Loadout rows with an array `worn` or string `carry` entries are older
/// than the shape 0.2.8 wrote, but 0.2.8 read them without asking the user
/// and never rewrote them. 0.2.9 reads them the same way: the array items
/// are unassigned gear, a string carry row is one of that item, and the
/// file is read but never rewritten.
#[test]
fn loadout_rows_in_shapes_0_2_8_read_are_read_but_never_rewritten() {
    let cases: [(&str, &[u8], Loadout); 2] = [
        (
            "array-worn",
            br#"[
                {"name":"current","worn":{"hat":"Helm"},"carry":[{"item":"Shark","qty":2}]},
                {"name":"old","worn":["Rune scimitar"],"carry":[]}
            ]"#,
            Loadout::new("old").with_slot("not-a-slot", "Rune scimitar"),
        ),
        (
            "string-carry",
            br#"[
                {"name":"current","worn":{"hat":"Helm"},"carry":[{"item":"Shark","qty":2}]},
                {"name":"old","worn":{},"carry":["Shark"]}
            ]"#,
            Loadout::new("old").with_carry("Shark", 1),
        ),
    ];
    for (case, bytes, old) in cases {
        let scratch = IsolatedEnv::enter(&format!("local-state-loadout-{case}"));
        let path = StateRoot::new(scratch.home.join(".274bot")).loadouts_file();
        write_private(&path, bytes);
        let service = service(&scratch, &[LocalStateDomain::LoadoutsReferences]);

        let lease = service
            .readback_lease(&loadouts_identity(), WAIT)
            .unwrap()
            .unwrap();
        assert_eq!(
            lease.snapshot().as_loadouts().unwrap(),
            &[
                Loadout::new("current")
                    .with_slot("hat", "Helm")
                    .with_carry("Shark", 2),
                old,
            ],
            "{case}"
        );
        assert_eq!(
            service.submit(loadout_edit(Some(lease.version()), Loadout::new("ranged"))),
            SubmitReceipt::Refused(ErrorCode::Failed {
                domain: LocalStateDomain::LoadoutsReferences,
                reason: FailureReason::Schema,
            }),
            "{case}"
        );
        assert_eq!(
            std::fs::read(&path).unwrap(),
            bytes,
            "{case}: original kept"
        );
    }
}

#[test]
fn current_loadouts_with_duplicate_names_are_refused_and_kept() {
    let scratch = IsolatedEnv::enter("local-state-loadout-duplicate-names");
    let path = StateRoot::new(scratch.home.join(".274bot")).loadouts_file();
    let rows = [
        Loadout::new("same").with_carry("Lobster", 1),
        Loadout::new("same").with_carry("Shark", 2),
    ];
    let bytes = serde_json::to_vec_pretty(&rows).unwrap();
    write_private(&path, &bytes);
    let (service, notices) = loadouts_service_with_notices(&scratch);

    assert_loadouts_refused_and_untouched(
        &service,
        &path,
        &bytes,
        &notices,
        FailureReason::Schema,
        "share a name with an earlier row",
    );
}

/// A compound preference patch remains queued while its database write is
/// held. The service owns its whole intent; polling only observes the outcome.
#[test]
fn a_queued_preferences_update_finishes_without_losing_compound_intent() {
    let _state_lease = crate::test_process::state();
    let scratch = IsolatedEnv::enter("local-state-t33-queued-update");
    let preferences = preferences_at(&scratch, "queued", None, |_| {});
    preferences.reload().unwrap();
    let (release, gate) = mpsc::channel::<()>();
    let gate = std::sync::Mutex::new(gate);
    let first = AtomicBool::new(true);
    preferences
        .service()
        .set_write_stall_for_test(Arc::new(move || {
            if first.swap(false, Ordering::SeqCst) {
                // The busy disk: the first write waits for the test.
                let _ = gate.lock().unwrap().recv();
            }
        }));

    let mut patch = PreferencePatch::new();
    patch
        .set_member(&["a"], Some(serde_json::json!(1)))
        .unwrap();
    patch
        .set_member(&["b"], Some(serde_json::json!(2)))
        .unwrap();
    let update = preferences.begin_patch(patch).unwrap();
    let Update::Queued(mut queued) = update else {
        panic!("the held first write outlasts the bounded wait: {update:?}");
    };
    assert!(
        queued.poll().is_none(),
        "still queued while the disk is held"
    );
    assert!(preferences.value("a").is_none(), "nothing written yet");

    release.send(()).unwrap();
    // A hang guard, not a pass condition: the writer settles both edits
    // once released.
    queued
        .wait(Duration::from_secs(600))
        .expect("the queued update settles")
        .expect("both edits are durable");
    assert_eq!(
        durable_json(&preferences),
        serde_json::json!({"a": 1, "b": 2})
    );
    assert_eq!(preferences.value("b").unwrap(), 2);
}
#[test]
fn preference_patch_rebases_nested_intent_and_refreshes_walk_projection() {
    let _state_lease = crate::test_process::state();
    let scratch = IsolatedEnv::enter("local-state-preference-patch-rebase");
    let preferences = preferences_from(
        &scratch,
        br#"{"nav":{"allow_teleports":false,"allow_wilderness":false},"theme":"dark"}"#,
    );

    let before = preferences.document().unwrap();
    let mut after = before.as_ref().clone();
    after["nav"]["allow_teleports"] = serde_json::json!(true);
    after["theme"] = serde_json::json!({"density": "compact"});
    let patch = PreferencePatch::between(&before, &after).unwrap();

    // Another writer (a second service on the root, as another process)
    // commits first: the worker reads it inside its own transaction before
    // it applies member intent.
    let outside = self::preferences(&scratch);
    outside.reload().unwrap();
    let mut edit =
        PreferencePatch::member(&["nav", "allow_wilderness"], Some(true.into())).unwrap();
    edit.set_member(&["theme"], Some("light".into())).unwrap();
    edit.set_member(&["outside"], Some(7.into())).unwrap();
    let Update::Queued(mut done) = outside.begin_patch(edit).unwrap() else {
        panic!("the outside edit is queued");
    };
    done.wait(WAIT).unwrap().unwrap();
    let audits = audits(preferences.service());
    let update = preferences.begin_patch(patch).unwrap();
    match update {
        Update::Durable => {}
        Update::Queued(mut watch) => watch
            .wait(WAIT)
            .expect("the accepted patch settles")
            .expect("the accepted patch is durable"),
    }

    let saved = preferences.document().unwrap();
    assert_eq!(saved["nav"]["allow_teleports"], true);
    assert_eq!(saved["nav"]["allow_wilderness"], true);
    assert_eq!(saved["theme"], serde_json::json!({"density": "compact"}));
    assert_eq!(saved["outside"], 7);
    assert!(preferences.walk_globals().unwrap().allow_teleports);
    let noop = preferences
        .begin_set("theme", saved["theme"].clone())
        .unwrap();
    if let Update::Queued(mut queued) = noop {
        queued
            .wait(WAIT)
            .expect("the no-op request settles")
            .expect("the no-op request remains durable");
    }
    let audits = audits.lock().unwrap();
    assert_eq!(audits.len(), 1);
    assert_eq!(audits[0].action(), CommitAction::PreferencesWrite);
}

#[test]
fn the_pvp_setting_projects_off_by_default_and_follows_one_member_write() {
    let _state_lease = crate::test_process::state();
    let scratch = IsolatedEnv::enter("local-state-pvp-projection");
    let preferences = preferences_from(&scratch, br#"{"nav":{"allow_teleports":true}}"#);
    assert!(
        !preferences.pvp_globals().unwrap().start_fights,
        "absent = off"
    );

    // One write of the member is one audit line, and the projection follows it.
    let audits = audits(preferences.service());
    let patch =
        PreferencePatch::member(&["pvp", "start_fights"], Some(serde_json::json!(true))).unwrap();
    match preferences.begin_patch(patch).unwrap() {
        Update::Durable => {}
        Update::Queued(mut watch) => watch
            .wait(WAIT)
            .expect("the accepted write settles")
            .expect("the accepted write is durable"),
    }
    assert!(preferences.pvp_globals().unwrap().start_fights);
    assert!(
        preferences.walk_globals().unwrap().allow_teleports,
        "nav is kept"
    );
    let audits = audits.lock().unwrap();
    assert_eq!(audits.len(), 1, "one write, one audit line");
    assert_eq!(audits[0].action(), CommitAction::PreferencesWrite);
}

#[test]
fn a_malformed_pvp_member_fails_closed_with_a_safe_typed_cause() {
    for (label, document) in [
        (
            "string",
            br#"{"pvp":{"start_fights":"private-sensitive-value"}}"#.as_slice(),
        ),
        ("not-an-object", br#"{"pvp":true}"#.as_slice()),
    ] {
        let scratch = IsolatedEnv::enter(&format!("local-state-pvp-projection-error-{label}"));
        let preferences = preferences_from(&scratch, document);
        let error = preferences
            .pvp_globals()
            .expect_err("a malformed pvp member is not a setting");
        assert!(error.path().ends_with("state.sqlite3"));
        assert!(error.cause().contains("pvp"), "{label}");
        assert!(!error.cause().contains("private-sensitive-value"));
        // The rest of the document stays readable and the walk view is not hurt.
        assert!(preferences.document().is_ok());
        assert!(preferences.walk_globals().is_ok());
    }
}

#[test]
fn the_pvp_setting_is_not_ready_while_the_store_is_cold_or_refused() {
    let _state_lease = crate::test_process::state();
    let scratch = IsolatedEnv::enter("local-state-pvp-cold");
    // An over-size legacy file is refused at the import: no store, no setting.
    let root = StateRoot::new(scratch.home.join(".274bot"));
    write_private(
        &root.preferences_file(),
        &vec![b' '; MAX_PREFERENCES_BYTES as usize + 1],
    );
    let preferences = preferences(&scratch);
    assert!(preferences.reload().is_err());
    assert!(
        preferences.pvp_globals().is_err(),
        "a refused store is no setting"
    );
}

#[test]
fn malformed_walk_projection_reports_safe_typed_cause() {
    let _state_lease = crate::test_process::state();
    let scratch = IsolatedEnv::enter("local-state-walk-projection-error");
    let preferences = preferences_from(
        &scratch,
        br#"{"nav":{"allow_teleports":"private-sensitive-value"}}"#,
    );

    let error = preferences
        .walk_globals()
        .expect_err("a non-boolean walk setting is not a typed view");
    assert!(error.path().ends_with("state.sqlite3"));
    assert!(error.cause().contains("nav.allow_teleports"));
    assert!(!error.cause().contains("private-sensitive-value"));
    assert_eq!(error.code(), None);
}

#[test]
fn dropping_preference_watch_keeps_service_owned_request_and_result() {
    let _state_lease = crate::test_process::state();
    let scratch = IsolatedEnv::enter("local-state-preference-watch-drop");
    let preferences = preferences(&scratch);
    preferences.reload().unwrap();
    let service = Arc::clone(preferences.service());
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let release_rx = std::sync::Mutex::new(release_rx);
    let first = AtomicBool::new(true);
    service.set_write_stall_for_test(Arc::new(move || {
        if first.swap(false, Ordering::SeqCst) {
            entered_tx.send(()).unwrap();
            release_rx.lock().unwrap().recv().unwrap();
        }
    }));

    let Update::Queued(watch) = preferences
        .begin_set("nav", serde_json::json!({"allow_wilderness": true}))
        .unwrap()
    else {
        panic!("a non-empty patch is service-owned")
    };
    let request = watch.request_id();
    entered_rx
        .recv_timeout(WAIT)
        .expect("the writer reaches the held request");
    drop(watch);
    drop(preferences);

    release_tx.send(()).unwrap();
    let preferences = service.preferences();
    let mut watch = preferences
        .watch(request)
        .expect("the result remains watchable");
    assert_eq!(
        watch.wait(WAIT),
        Some(Ok(())),
        "dropping the original watch does not cancel its accepted request"
    );
    assert!(preferences.unprojected_requests().is_empty());
    assert!(matches!(
        service.request_state(request),
        RequestState::Settled(Settlement::Completed(_))
    ));
    assert_eq!(
        preferences.value("nav"),
        Some(serde_json::json!({"allow_wilderness": true}))
    );
}

#[test]
fn compound_preference_patch_rebases_after_cold_hydration_and_frontend_drop() {
    let _state_lease = crate::test_process::state();
    let scratch = IsolatedEnv::enter("local-state-compound-preference-drop");
    let root = StateRoot::new(scratch.home.join(".274bot"));
    let path = root.preferences_file();
    write_private(
        &path,
        br#"{"nav":{"allow_teleports":false,"allow_wilderness":false},"theme":"dark"}"#,
    );

    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let release_rx = std::sync::Mutex::new(release_rx);
    let first = AtomicBool::new(true);
    let service = LocalStateService::open_process_with_read_stall_for_test(
        root.clone(),
        Arc::new(move || {
            if first.swap(false, Ordering::SeqCst) {
                entered_tx.send(()).unwrap();
                let _ = release_rx.lock().unwrap().recv();
            }
        }),
    );
    let preferences = service.preferences();
    let audits = audits(&service);
    entered_rx
        .recv_timeout(WAIT)
        .expect("the worker is held before cold hydration");

    let before = serde_json::json!({
        "nav": {"allow_teleports": false, "allow_wilderness": false},
        "theme": "dark"
    });
    let after = serde_json::json!({
        "nav": {"allow_teleports": true, "allow_wilderness": false},
        "theme": {"density": "compact"}
    });
    let patch = PreferencePatch::between(
        before.as_object().expect("an object"),
        after.as_object().expect("an object"),
    )
    .unwrap();
    let Update::Queued(mut watch) = preferences.begin_patch(patch).unwrap() else {
        panic!("a compound patch remains service-owned while hydration is held");
    };
    let request = watch.request_id();
    assert!(watch.poll().is_none(), "the held read has not completed");

    // An outside edit lands after the caller formed its patch but before the
    // worker reads the current document.
    write_private(
        &path,
        br#"{"nav":{"allow_teleports":false,"allow_wilderness":true},"theme":"light","outside":7}"#,
    );
    drop(watch);
    drop(preferences);
    release_tx.send(()).unwrap();

    let preferences = service.preferences();
    let mut watch = preferences
        .watch(request)
        .expect("the accepted compound request remains service-owned");
    assert_eq!(
        watch.wait(WAIT),
        Some(Ok(())),
        "the rebased compound patch has one durable result"
    );
    assert!(preferences.unprojected_requests().is_empty());
    assert!(matches!(
        service.request_state(request),
        RequestState::Settled(Settlement::Completed(_))
    ));
    assert_eq!(audits.lock().unwrap().len(), 1);
    let expected = serde_json::json!({
        "nav": {"allow_teleports": true, "allow_wilderness": true},
        "theme": {"density": "compact"},
        "outside": 7
    });
    assert_eq!(
        preferences.document().unwrap().as_ref(),
        expected.as_object().expect("an object")
    );
    assert!(preferences.walk_globals().unwrap().allow_teleports);
    assert!(preferences.walk_globals().unwrap().allow_wilderness);

    drop(watch);
    drop(preferences);
    drop(service);
    let reopened = LocalStateService::open_process(root);
    let preferences = reopened.preferences();
    preferences.reload().unwrap();
    assert_eq!(
        preferences.document().unwrap().as_ref(),
        expected.as_object().expect("an object"),
        "reopening reads the complete compound publication"
    );
}

#[test]
fn cold_preference_document_and_walk_fail_before_hydration() {
    let _state_lease = crate::test_process::state();
    let scratch = IsolatedEnv::enter("local-state-preference-cold-read");
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let release_rx = std::sync::Mutex::new(release_rx);
    let first = AtomicBool::new(true);
    let service = LocalStateService::open_process_with_read_stall_for_test(
        StateRoot::new(scratch.home.join(".274bot")),
        Arc::new(move || {
            if first.swap(false, Ordering::SeqCst) {
                entered_tx.send(()).unwrap();
                release_rx.lock().unwrap().recv().unwrap();
            }
        }),
    );
    let preferences = service.preferences();
    entered_rx
        .recv_timeout(WAIT)
        .expect("the worker is held before the first hydration read");

    assert!(preferences.document().unwrap_err().is_not_ready());
    assert!(preferences.walk_globals().unwrap_err().is_not_ready());
    assert_eq!(preferences.value("nav"), None);

    // Slots copy five flags or one shared handle, never a preference document.
    assert!(std::mem::size_of::<crate::WalkGlobals>() <= 8);
    assert!(std::mem::size_of::<Preferences>() <= std::mem::size_of::<Arc<()>>());
    let assert_cached_reads_allocate_nothing = || {
        let allocations = allocation_counter::measure(|| {
            for _ in 0..256 {
                let _ = std::hint::black_box(preferences.document());
                let _ = std::hint::black_box(preferences.walk_globals());
            }
        });
        assert_eq!(allocations.count_total, 0, "{allocations:?}");
        assert_eq!(allocations.bytes_total, 0, "{allocations:?}");
    };
    assert_cached_reads_allocate_nothing();

    release_tx.send(()).unwrap();
    preferences.reload().unwrap();
    assert!(
        preferences.document().unwrap().is_empty(),
        "readiness fabricates no saved keys"
    );
    assert!(
        !preferences.service().root().preferences_file().exists(),
        "readiness writes no JSON"
    );
    assert_cached_reads_allocate_nothing();

    // A refused store: a damaged database at another root.
    let refused_root = StateRoot::new(scratch.home.join("refused").join(".274bot"));
    vault::create_private_dir(&refused_root.state_dir()).unwrap();
    vault::create_private_file(&refused_root.database_file(), &[0x5a; 4096]).unwrap();
    let refused = LocalStateService::open_process(refused_root).preferences();
    let _ = refused_snapshot(&refused);
    assert!(refused.document().is_err());
    assert!(refused.walk_globals().is_err());
    let allocations = allocation_counter::measure(|| {
        for _ in 0..256 {
            let _ = std::hint::black_box(refused.document());
            let _ = std::hint::black_box(refused.walk_globals());
        }
    });
    assert_eq!(allocations.count_total, 0, "{allocations:?}");
    assert_eq!(allocations.bytes_total, 0, "{allocations:?}");
}

#[test]
fn preference_results_outlive_neutral_history_and_bound_new_admission() {
    let _state_lease = crate::test_process::state();
    let scratch = IsolatedEnv::enter("local-state-preference-result-bound");
    let preferences = preferences(&scratch);
    preferences.reload().unwrap();
    let service = Arc::clone(preferences.service());
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let release_rx = std::sync::Mutex::new(release_rx);
    let first = AtomicBool::new(true);
    service.set_write_stall_for_test(Arc::new(move || {
        if first.swap(false, Ordering::SeqCst) {
            entered_tx.send(()).unwrap();
            let _ = release_rx.lock().unwrap().recv();
        }
    }));
    let Update::Queued(first_watch) = preferences.begin_set("first", true.into()).unwrap() else {
        panic!("a changed preference is accepted");
    };
    entered_rx.recv_timeout(WAIT).unwrap();
    let mut watches = vec![first_watch];
    for index in 0..OPERATOR_QUEUE_CAP {
        let Update::Queued(watch) = preferences
            .begin_set(&format!("queued_{index}"), true.into())
            .unwrap()
        else {
            panic!("each changed preference is accepted");
        };
        watches.push(watch);
    }
    assert!(matches!(
        preferences
            .begin_set("queue_overflow", true.into())
            .unwrap_err()
            .code(),
        Some(ErrorCode::Busy { .. })
    ));
    let first_request = watches[0].request_id();
    let last_request = watches.last().unwrap().request_id();
    release_tx.send(()).unwrap();
    assert!(matches!(
        service.wait_request(last_request, WAIT),
        RequestState::Settled(Settlement::Completed(_))
    ));
    assert!(matches!(
        service.request_state(first_request),
        RequestState::Forgotten
    ));
    assert_eq!(
        preferences.unprojected_requests().len(),
        PREFERENCE_WATCH_CAP
    );
    assert!(preferences.watch(first_request).is_some());
    assert!(matches!(
        preferences
            .begin_set("result_overflow", true.into())
            .unwrap_err()
            .code(),
        Some(ErrorCode::Busy { .. })
    ));
    assert_eq!(watches[0].wait(WAIT), Some(Ok(())));
    assert!(preferences.watch(first_request).is_none());
    let Update::Queued(mut next) = preferences.begin_set("next", true.into()).unwrap() else {
        panic!("projecting a result releases one admission credit");
    };
    for watch in watches.iter_mut().skip(1) {
        assert_eq!(watch.wait(WAIT), Some(Ok(())));
    }
    assert_eq!(next.wait(WAIT), Some(Ok(())));
    assert!(preferences.unprojected_requests().is_empty());
}

#[test]
fn preference_byte_limit_refuses_before_queue_capacity_and_releases_credit() {
    let _state_lease = crate::test_process::state();
    let scratch = IsolatedEnv::enter("local-state-preference-byte-bound");
    let preferences = preferences(&scratch);
    preferences.reload().unwrap();
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let release_rx = std::sync::Mutex::new(release_rx);
    let first = AtomicBool::new(true);
    preferences
        .service()
        .set_write_stall_for_test(Arc::new(move || {
            if first.swap(false, Ordering::SeqCst) {
                entered_tx.send(()).unwrap();
                let _ = release_rx.lock().unwrap().recv();
            }
        }));
    let bulk = || {
        serde_json::Value::Object(
            (0..200)
                .map(|index| {
                    (
                        format!("member_{index}"),
                        serde_json::Value::String("x".repeat(3000)),
                    )
                })
                .collect(),
        )
    };
    let Update::Queued(mut watch) = preferences.begin_set("bulk", bulk()).unwrap() else {
        panic!("one bounded large patch is accepted");
    };
    entered_rx.recv_timeout(WAIT).unwrap();
    assert!(matches!(
        preferences.begin_set("second", bulk()).unwrap_err().code(),
        Some(ErrorCode::Validation {
            reason: ValidationReason::TooLong,
            ..
        })
    ));
    let mut small = Vec::new();
    for index in 0..OPERATOR_QUEUE_CAP {
        let Update::Queued(queued) = preferences
            .begin_set(&format!("small_{index}"), true.into())
            .expect("small patches still fit beside the held large intent")
        else {
            panic!("the held writer keeps every small patch queued");
        };
        small.push(queued);
    }
    assert_eq!(
        preferences
            .begin_set("queue_overflow", true.into())
            .unwrap_err()
            .code(),
        Some(ErrorCode::Busy {
            domain: LocalStateDomain::Preferences
        }),
        "queue capacity is separate from aggregate intent bytes"
    );
    assert!(PreferencePatch::set(
        "unrepresentable",
        serde_json::Value::String("x".repeat(api::local_state::MAX_PREF_VALUE_BYTES + 1)),
    )
    .is_err());
    release_tx.send(()).unwrap();
    assert_eq!(watch.wait(WAIT), Some(Ok(())));
    for queued in &mut small {
        assert_eq!(queued.wait(WAIT), Some(Ok(())));
    }
    let Update::Queued(mut large_after_release) = preferences.begin_set("bulk", bulk()).unwrap()
    else {
        panic!("settlement releases enough intent bytes for another large patch");
    };
    assert_eq!(large_after_release.wait(WAIT), Some(Ok(())));
    let Update::Queued(mut next) = preferences.begin_set("next", true.into()).unwrap() else {
        panic!("settlement releases queued intent bytes");
    };
    assert_eq!(next.wait(WAIT), Some(Ok(())));
    assert!(preferences.value("second").is_none());
    assert_eq!(preferences.value("next"), Some(true.into()));
    for index in 0..OPERATOR_QUEUE_CAP {
        assert_eq!(
            preferences.value(&format!("small_{index}")),
            Some(true.into())
        );
    }
    assert!(preferences.value("queue_overflow").is_none());
}

#[test]
fn fix_r1_shutdown_finishes_a_cold_preference_before_the_nonzero_bound() {
    let _state_lease = crate::test_process::state();
    let scratch = IsolatedEnv::enter("local-state-preference-cold-shutdown-finish");
    let root = StateRoot::new(scratch.home.join(".274bot"));
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let release_rx = std::sync::Mutex::new(release_rx);
    let first = AtomicBool::new(true);
    let service = LocalStateService::open_process_with_read_stall_for_test(
        root.clone(),
        Arc::new(move || {
            if first.swap(false, Ordering::SeqCst) {
                entered_tx.send(()).unwrap();
                let _ = release_rx.lock().unwrap().recv();
            }
        }),
    );
    service.set_shutdown_timeout(WAIT);
    let commits = audits(&service);
    let lines = terminals(&service);
    let ledger = service.ledger();
    entered_rx
        .recv_timeout(WAIT)
        .expect("the loadouts read holds the preferences store cold");
    let preferences = service.preferences();
    let mut patch =
        PreferencePatch::member(&["nav", "allow_teleports"], Some(true.into())).unwrap();
    patch
        .set_member(&["nav", "allow_wilderness"], Some(true.into()))
        .unwrap();
    let Update::Queued(watch) = preferences.begin_patch(patch).unwrap() else {
        panic!("the cold store accepts the compound preference patch");
    };
    let request = watch.request_id();
    drop(watch);
    drop(preferences);
    let shutdown = std::thread::spawn(move || drop(service));
    {
        let deadline = Instant::now() + WAIT;
        let mut state = ledger.0.state.lock();
        while !state.shutdown {
            assert!(
                !ledger.0.wake.wait_until(&mut state, deadline).timed_out(),
                "shutdown starts before the loadouts read is released"
            );
        }
    }
    release_tx.send(()).unwrap();
    shutdown.join().unwrap();
    assert!(ledger.wait_writer_exit(WAIT));
    assert!(
        matches!(
            ledger.request_state(request),
            RequestState::Settled(Settlement::Completed(_))
        ),
        "the accepted cold patch gets its terminal result before shutdown returns"
    );
    let state = ledger.0.state.lock();
    let store = state.stores.get(&StoreKey::Preferences).unwrap();
    assert_eq!(
        store
            .settled
            .iter()
            .filter(|entry| entry.request == request)
            .count(),
        1,
        "the accepted patch settles exactly once"
    );
    assert!(state.preference_watches[&request].settlement.is_some());
    drop(state);
    let commits = commits.lock().unwrap();
    assert_eq!(commits.len(), 1, "one saved-state info line");
    assert_eq!(commits[0].identity().account().profile(), "operator");
    assert_eq!(commits[0].action(), CommitAction::PreferencesWrite);
    assert_eq!(commits[0].origin(), AuditOrigin::Submitted { request });
    assert!(
        lines.lock().unwrap().is_empty(),
        "no duplicate terminal line"
    );
    drop(state_after_shutdown(&root));
}

/// What a fresh service at `root` reads as durable, and that service.
fn state_after_shutdown(root: &StateRoot) -> Arc<LocalStateService> {
    let reopened = LocalStateService::open_process(root.clone());
    let saved =
        serde_json::Value::Object(reopened.preferences().reload().unwrap().as_ref().clone());
    assert_eq!(
        saved,
        serde_json::json!({"nav":{"allow_teleports":true,"allow_wilderness":true}})
    );
    reopened
}

/// T5.3: a save that changes nothing durable (the member already holds
/// the value) commits nothing: no version, no write, no saved-state line.
#[test]
fn a_semantic_noop_save_commits_nothing_and_logs_no_saved_state_line() {
    let _state_lease = crate::test_process::state();
    let scratch = IsolatedEnv::enter("local-state-preference-noop-audit");
    let preferences = preferences_from(&scratch, b"{ \"nav\" : { \"allow_teleports\" : true } }\n");
    let commits = audits(preferences.service());
    let before = preferences.snapshot().unwrap().version();
    let Update::Queued(mut watch) = preferences
        .begin_patch(
            PreferencePatch::member(&["nav", "allow_teleports"], Some(true.into())).unwrap(),
        )
        .unwrap()
    else {
        panic!("the semantic no-op is accepted");
    };
    assert_eq!(watch.wait(WAIT), Some(Ok(())));
    assert!(commits.lock().unwrap().is_empty(), "no saved-state line");
    assert_eq!(preferences.snapshot().unwrap().version(), before);
    assert_eq!(
        durable_json(&preferences),
        serde_json::json!({"nav":{"allow_teleports":true}})
    );
}

#[test]
fn shutdown_cancels_a_cold_compound_preference_once_without_saving_defaults() {
    let _state_lease = crate::test_process::state();
    let scratch = IsolatedEnv::enter("local-state-preference-cold-cancel");
    let root = StateRoot::new(scratch.home.join(".274bot"));
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let release_rx = std::sync::Mutex::new(release_rx);
    let first = AtomicBool::new(true);
    let service = LocalStateService::open_process_with_read_stall_for_test(
        root.clone(),
        Arc::new(move || {
            if first.swap(false, Ordering::SeqCst) {
                entered_tx.send(()).unwrap();
                let _ = release_rx.lock().unwrap().recv();
            }
        }),
    );
    service.set_shutdown_timeout(Duration::ZERO);
    let lines = terminals(&service);
    let ledger = service.ledger();
    entered_rx.recv_timeout(WAIT).unwrap();
    let preferences = service.preferences();
    let mut patch = PreferencePatch::member(&["first"], Some(1.into())).unwrap();
    patch.set_member(&["second"], Some(2.into())).unwrap();
    let Update::Queued(watch) = preferences.begin_patch(patch).unwrap() else {
        panic!("the compound request is queued behind cold hydration");
    };
    let request = watch.request_id();
    drop(watch);
    drop(preferences);
    drop(service);
    assert_eq!(
        ledger.request_state(request),
        RequestState::Settled(Settlement::Cancelled(Cancellation::Shutdown))
    );
    release_tx.send(()).unwrap();
    assert!(ledger.wait_writer_exit(WAIT));
    assert_eq!(lines.lock().unwrap().len(), 1);
    assert!(!root.preferences_file().exists());
    let reopened = LocalStateService::open_process(root);
    let preferences = reopened.preferences();
    preferences.reload().unwrap();
    assert!(preferences.document().unwrap().is_empty());
    assert!(!preferences.service().root().preferences_file().exists());
}

#[test]
fn nested_preference_intent_replaces_a_scalar_parent_without_losing_siblings() {
    let _state_lease = crate::test_process::state();
    let scratch = IsolatedEnv::enter("local-state-preference-scalar-parent");
    let preferences = preferences_from(&scratch, br#"{"nav":7,"other":{"keep":true}}"#);
    let patch = PreferencePatch::member(&["nav", "allow_teleports"], Some(true.into())).unwrap();
    let Update::Queued(mut watch) = preferences.begin_patch(patch).unwrap() else {
        panic!("the scalar-to-object edit is accepted");
    };
    assert_eq!(watch.wait(WAIT), Some(Ok(())));
    assert_eq!(
        preferences.value("nav"),
        Some(serde_json::json!({"allow_teleports":true}))
    );
    assert_eq!(
        preferences.value("other"),
        Some(serde_json::json!({"keep":true}))
    );
    let Update::Queued(mut watch) = preferences.begin_set("nav", 9.into()).unwrap() else {
        panic!("the object-to-scalar edit is accepted");
    };
    assert_eq!(watch.wait(WAIT), Some(Ok(())));
    assert_eq!(preferences.value("nav"), Some(9.into()));
    assert!(preferences.walk_globals().is_err());
    assert_eq!(
        preferences.value("other"),
        Some(serde_json::json!({"keep":true}))
    );
}

#[test]
fn preference_audit_is_bounded_redacted_and_absent_for_a_noop() {
    let before = serde_json::json!({"nav":{"allow_teleports":false},"quester_paths":{"folder":"old-private-path"}});
    let after = serde_json::json!({"nav":{"allow_teleports":true},"quester_paths":{"folder":"new-private-path"},"password":"private-password"});
    let before = before.as_object().unwrap();
    let after = after.as_object().unwrap();
    let summary = preference_audit_summary(before, after).unwrap().text;
    assert!(summary.contains("nav.allow_teleports"));
    assert!(summary.contains("false") && summary.contains("true"));
    assert!(!summary.contains("old-private-path"));
    assert!(!summary.contains("new-private-path"));
    assert!(!summary.contains("private-password"));
    assert!(preference_audit_summary(after, after).is_none());
    let wide = (0..1000)
        .map(|index| (format!("member_{index}_{}", "x".repeat(100)), true.into()))
        .collect();
    let summary = preference_audit_summary(&serde_json::Map::new(), &wide)
        .unwrap()
        .text;
    assert!(summary.len() <= PREFERENCE_AUDIT_MAX_BYTES);
    assert!(summary.contains(" omitted"));
    assert!(summary.split("; ").count() <= PREFERENCE_AUDIT_MAX_CHANGES + 1);
}

/// A snapshot is the cached document under the durable version it was
/// published at: a repeated read is the same version and the same shared
/// document, this service's own write and another writer's change each move
/// the version once, and a cold store has no snapshot.
#[test]
fn preference_snapshot_versions_move_once_per_durable_change() {
    let _state_lease = crate::test_process::state();
    let scratch = IsolatedEnv::enter("local-state-preference-snapshot");
    let preferences = preferences_from(&scratch, br#"{"theme":"dark"}"#);

    let first = preferences.snapshot().unwrap();
    let again = preferences.snapshot().unwrap();
    assert_eq!(first.version(), again.version(), "a repeated read");
    assert!(
        Arc::ptr_eq(first.document(), again.document()),
        "the document is shared, not copied"
    );
    assert_eq!(first.document()["theme"], "dark");

    // This service's own write.
    match preferences
        .begin_set("theme", serde_json::json!("light"))
        .unwrap()
    {
        Update::Durable => {}
        Update::Queued(mut watch) => watch
            .wait(WAIT)
            .expect("the write settles")
            .expect("the write is durable"),
    }
    let written = preferences.snapshot().unwrap();
    assert!(written.version() > first.version());
    assert_eq!(written.document()["theme"], "light");

    // Another writer's change (a second service, as another process), seen
    // once the document is read again.
    let other = self::preferences(&scratch);
    other.reload().unwrap();
    let mut edit = PreferencePatch::member(&["theme"], Some("other".into())).unwrap();
    edit.set_member(&["outside"], Some(1.into())).unwrap();
    let Update::Queued(mut done) = other.begin_patch(edit).unwrap() else {
        panic!("the outside edit is queued");
    };
    done.wait(WAIT).unwrap().unwrap();
    preferences.reload().unwrap();
    let outside = preferences.snapshot().unwrap();
    assert!(outside.version() > written.version());
    assert_eq!(outside.document()["outside"], 1);
    assert_eq!(
        preferences.snapshot().unwrap().version(),
        outside.version(),
        "no change, no new version"
    );
}

#[path = "local_state/bank_tests.rs"]
mod bank_hints;
