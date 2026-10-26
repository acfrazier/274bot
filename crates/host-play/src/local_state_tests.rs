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
    StoreVersion, SubmitReceipt, ValidationReason,
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

/// Retry a frontend write while it reports the write queued past the
/// frontend's bounded durability wait: under machine load a write stays
/// queued past the frontend timeout (a product deadline this suite must
/// not change) and settles later. Any other error returns at once, so
/// refusals and genuine conflicts keep their fast verdict.
fn patiently(mut attempt: impl FnMut() -> Result<(), String>) -> Result<(), String> {
    let start = std::time::Instant::now();
    loop {
        match attempt() {
            Ok(()) => return Ok(()),
            Err(error)
                if error.contains("not durable within")
                    && start.elapsed() < Duration::from_secs(60) =>
            {
                std::thread::yield_now();
            }
            Err(error) => return Err(error),
        }
    }
}
/// A preferences `set`, retrying a load-slow durability wait.
fn set_patiently(preferences: &Preferences, key: &str, value: serde_json::Value) {
    patiently(|| {
        preferences
            .set(key, value.clone())
            .map_err(|error| error.to_string())
    })
    .unwrap();
}
/// A preferences `update`, retrying a load-slow durability wait. A write
/// of an earlier attempt that lands late is merged over, not duplicated:
/// the update re-reads and re-applies `merge` on a conflict.
fn update_patiently(
    preferences: &Preferences,
    mut merge: impl FnMut(&mut serde_json::Map<String, serde_json::Value>) -> std::io::Result<()>,
) {
    patiently(|| {
        preferences
            .update(|document| merge(document))
            .map_err(|error| error.to_string())
    })
    .unwrap();
}
/// A loadouts form `save`, retrying a load-slow durability wait. A write
/// of an earlier attempt that lands late conflicts with its own retry;
/// the save replays the store's latest on a conflict, so a retry converges
/// instead of duplicating rows.
fn save_patiently(form: &mut script::LoadoutsStore) {
    let start = std::time::Instant::now();
    loop {
        match form.save() {
            Ok(()) => return,
            Err(error)
                if (error.contains("not durable within")
                    || error.contains("changed since this list was loaded"))
                    && start.elapsed() < Duration::from_secs(60) =>
            {
                std::thread::yield_now();
            }
            Err(error) => panic!("save failed: {error}"),
        }
    }
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
fn bank_hint_submission_writes_schema_one_and_readback_preserves_rows_and_time() {
    let scratch = IsolatedEnv::enter("local-state-bank-format");
    let root = StateRoot::new(scratch.home.join(".274bot"));
    let service = LocalStateService::open(
        root.clone(),
        &[LocalStateDomain::BankHints],
        Arc::new(RequestIdMinter::new()),
    );
    let alice = bank_identity("alice");
    let at = 1_759_700_123;
    assert_eq!(service.readback_lease(&alice, WAIT).unwrap(), None);
    let request =
        request(service.submit(hint_edit(&alice, None, vec![(379, 12), (995, 3400)], at)));
    let receipt = completed(&service, request);
    let path = root.bank_hint_file(PROFILE, "alice");
    let written: serde_json::Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    assert_eq!(written["schema_version"], serde_json::json!(1));
    assert_eq!(written["profile"], serde_json::json!(PROFILE));
    assert_eq!(written["account"], serde_json::json!("alice"));
    assert_eq!(written["revision"], serde_json::json!(289));
    assert_eq!(written["content"], serde_json::json!(CONTENT));
    assert_eq!(written["observed_at_unix"], serde_json::json!(at));
    assert_eq!(written["rows"], serde_json::json!([[379, 12], [995, 3400]]));
    assert_eq!(
        receipt.version(),
        StoreVersion::INITIAL.checked_next().unwrap()
    );

    let bob = bank_identity("bob");
    let bob_at = 1_759_700_456;
    let bob_rows = [(526, 5), (995, 70)];
    let bob_path = root.bank_hint_file(PROFILE, "bob");
    write_private(
        &bob_path,
        &hint_document(PROFILE, "bob", HINT_SCHEMA_VERSION, bob_at, &bob_rows),
    );
    let lease = service.readback_lease(&bob, WAIT).unwrap().unwrap();
    assert_eq!(lease.snapshot().as_bank_hints().unwrap(), bob_rows);
    assert_eq!(lease.at().unix_seconds(), bob_at);
}

#[test]
fn expected_version_conflict_settles_without_touching_the_file() {
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
fn a_new_queued_hint_supersedes_the_old_one_which_follows_it_to_durability() {
    let scratch = IsolatedEnv::enter("local-state-superseded");
    let root = StateRoot::new(scratch.home.join(".274bot"));
    let service = LocalStateService::open(
        root.clone(),
        &[
            LocalStateDomain::BankHints,
            LocalStateDomain::LoadoutsReferences,
        ],
        Arc::new(RequestIdMinter::new()),
    );
    let bank = bank_identity("alice");
    let loadouts = loadouts_identity();
    let _ = service.readback_lease(&bank, WAIT).unwrap();
    let _ = service.readback_lease(&loadouts, WAIT).unwrap();
    let blocker_path = root.loadouts_file();
    let bank_path = root.bank_hint_file(PROFILE, "alice");
    let hook_bank_path = bank_path.clone();
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel::<()>();
    let release_rx = std::sync::Mutex::new(release_rx);
    let bank_writes = Arc::new(AtomicUsize::new(0));
    let counted_bank_writes = Arc::clone(&bank_writes);
    service.set_write_hook(Arc::new(move |path, bytes| {
        if path == blocker_path {
            entered_tx
                .send(())
                .map_err(|error| std::io::Error::other(error.to_string()))?;
            release_rx
                .lock()
                .unwrap()
                .recv()
                .map_err(|error| std::io::Error::other(error.to_string()))?;
        }
        if path == hook_bank_path {
            counted_bank_writes.fetch_add(1, Ordering::SeqCst);
        }
        vault::write_private_file(path, bytes)
    }));

    let blocker = request(service.submit(loadout_edit(
        None,
        Loadout::new("blocker").with_carry("Coins", 1),
    )));
    entered_rx.recv_timeout(WAIT).unwrap();
    let first = request(service.submit(hint_edit(&bank, None, vec![(995, 1)], 100)));
    let second = request(service.submit(hint_edit(&bank, None, vec![(995, 2)], 101)));
    assert_eq!(
        service.observe(first),
        Some(Settlement::Cancelled(Cancellation::Superseded))
    );
    assert_eq!(service.observe(second), None, "the newest hint is queued");
    assert_eq!(
        service.wait_durable(first, Duration::from_millis(50)),
        Durability::Pending,
        "a superseded hint is durable only when its replacement is"
    );

    release_tx.send(()).unwrap();
    assert!(matches!(
        settlement(&service, blocker),
        Settlement::Completed(_)
    ));
    let durable = service.wait_durable(first, WAIT);
    assert!(
        matches!(&durable, Durability::Durable(receipt) if receipt.request() == Some(second)),
        "{durable:?}"
    );
    assert_eq!(
        bank_writes.load(Ordering::SeqCst),
        1,
        "only the newest queued hint was written"
    );
    let written: serde_json::Value =
        serde_json::from_slice(&std::fs::read(bank_path).unwrap()).unwrap();
    assert_eq!(written["rows"], serde_json::json!([[995, 2]]));
}

#[test]
fn identical_hint_bytes_complete_at_the_existing_version_without_a_write() {
    let scratch = IsolatedEnv::enter("local-state-identical");
    let root = StateRoot::new(scratch.home.join(".274bot"));
    let service = LocalStateService::open(
        root.clone(),
        &[LocalStateDomain::BankHints],
        Arc::new(RequestIdMinter::new()),
    );
    let identity = bank_identity("alice");
    assert_eq!(service.readback_lease(&identity, WAIT).unwrap(), None);
    let writes = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&writes);
    service.set_write_hook(Arc::new(move |path, bytes| {
        counted.fetch_add(1, Ordering::SeqCst);
        vault::write_private_file(path, bytes)
    }));
    let rows = vec![(995, 3400)];
    let at = 1_759_700_123;
    let first = request(service.submit(hint_edit(&identity, None, rows.clone(), at)));
    let first_receipt = completed(&service, first);
    assert_eq!(writes.load(Ordering::SeqCst), 1);

    let second = request(service.submit(hint_edit(&identity, None, rows, at)));
    let second_receipt = completed(&service, second);
    assert_eq!(second_receipt.version(), first_receipt.version());
    assert_eq!(writes.load(Ordering::SeqCst), 1);
}

#[test]
fn failed_write_keeps_previous_rows_and_a_following_submission_can_succeed() {
    let scratch = IsolatedEnv::enter("local-state-write-failure");
    let root = StateRoot::new(scratch.home.join(".274bot"));
    let service = LocalStateService::open(
        root.clone(),
        &[LocalStateDomain::BankHints],
        Arc::new(RequestIdMinter::new()),
    );
    let identity = bank_identity("alice");
    assert_eq!(service.readback_lease(&identity, WAIT).unwrap(), None);
    let original = request(service.submit(hint_edit(&identity, None, vec![(995, 20)], 200)));
    completed(&service, original);
    let path = root.bank_hint_file(PROFILE, "alice");
    let calls = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&calls);
    service.set_write_hook(Arc::new(move |path, bytes| {
        if counted.fetch_add(1, Ordering::SeqCst) == 0 {
            return Err(std::io::Error::other("disk full"));
        }
        vault::write_private_file(path, bytes)
    }));

    let failed = request(service.submit(hint_edit(&identity, None, vec![(995, 25)], 201)));
    assert_eq!(
        settlement(&service, failed),
        Settlement::Failed(ErrorCode::Failed {
            domain: LocalStateDomain::BankHints,
            reason: FailureReason::Io,
        })
    );
    assert_eq!(
        service
            .lease(&identity)
            .unwrap()
            .snapshot()
            .as_bank_hints()
            .unwrap(),
        &[(995, 20)]
    );
    let before_retry = std::fs::read(&path).unwrap();
    let retry = request(service.submit(hint_edit(&identity, None, vec![(995, 25)], 201)));
    completed(&service, retry);
    assert_ne!(std::fs::read(path).unwrap(), before_retry);
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[test]
fn unopened_preferences_are_not_ready_for_leases_or_submissions() {
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

/// r2 item 5: every refused hint file the review attacked (unparseable,
/// another schema, another identity, group/other-writable, unreadable)
/// stays refused for leases and submissions and keeps its bytes.
#[cfg(unix)]
#[test]
fn refused_hint_files_are_never_overwritten() {
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

/// r2 item 5: rows the service cannot read back as they are (a malformed
/// row, an unknown field) are leased, but the file is never rewritten
/// without them.
#[test]
fn a_loadouts_file_the_service_cannot_round_trip_is_read_but_never_rewritten() {
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

/// r2 item 5: a refused file is read again on demand; once repaired, it is
/// leased and written normally.
#[test]
fn a_repaired_file_is_accepted_again() {
    let scratch = IsolatedEnv::enter("local-state-repaired");
    let root = StateRoot::new(scratch.home.join(".274bot"));
    let service = LocalStateService::open(
        root.clone(),
        &[LocalStateDomain::BankHints],
        Arc::new(RequestIdMinter::new()),
    );
    let alice = bank_identity("alice");
    let path = root.bank_hint_file(PROFILE, "alice");
    write_private(&path, b"{oops");
    assert!(service.readback_lease(&alice, WAIT).is_err());
    std::fs::remove_file(&path).unwrap();
    assert_eq!(service.readback_lease(&alice, WAIT).unwrap(), None);
    let written = request(service.submit(hint_edit(&alice, None, vec![(995, 5)], 5)));
    completed(&service, written);
    assert!(std::fs::read_to_string(&path)
        .unwrap()
        .contains("[[995,5]]"));
}

/// r2 item 4 (sol probe 1): after another writer replaced the hint, an
/// identical resubmission is not a success over bytes it never wrote.
#[test]
fn a_hint_resubmitted_after_an_outside_write_conflicts_and_keeps_the_outside_bytes() {
    let scratch = IsolatedEnv::enter("local-state-outside-hint");
    let root = StateRoot::new(scratch.home.join(".274bot"));
    let service = LocalStateService::open(
        root.clone(),
        &[LocalStateDomain::BankHints],
        Arc::new(RequestIdMinter::new()),
    );
    let alice = bank_identity("alice");
    assert_eq!(service.readback_lease(&alice, WAIT).unwrap(), None);
    let ten = hint_edit(&alice, None, vec![(995, 10)], 10);
    completed(&service, request(service.submit(ten.clone())));
    let path = root.bank_hint_file(PROFILE, "alice");
    let outside = hint_document(PROFILE, "alice", HINT_SCHEMA_VERSION, 20, &[(995, 20)]);
    write_private(&path, &outside);

    let again = request(service.submit(ten));
    assert!(
        matches!(
            settlement(&service, again),
            Settlement::Failed(ErrorCode::Conflict { .. })
        ),
        "the resubmission did not fabricate a write"
    );
    assert_eq!(std::fs::read(&path).unwrap(), outside);
    let lease = service.lease(&alice).unwrap();
    assert_eq!(lease.snapshot().as_bank_hints().unwrap(), &[(995, 20)]);
}

/// r2 item 4 (sol probe 2): a loadout edit written against the cached
/// version after the form saved another row conflicts; the saved row stays.
#[test]
fn a_loadout_edit_after_a_form_saved_a_row_conflicts_and_keeps_the_row() {
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

/// r2 item 4: an edit accepted before another writer changed the file
/// conflicts instead of writing over the change, also when a readback saw
/// the change first (the precondition then matches the cache, but the edit
/// was accepted against what came before).
#[test]
fn an_edit_accepted_before_an_outside_change_conflicts() {
    let scratch = IsolatedEnv::enter("local-state-accepted-then-changed");
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
    assert_eq!(service.readback_lease(&alice, WAIT).unwrap(), None);
    let _ = service.readback_lease(&loadouts_identity(), WAIT).unwrap();
    let stall = stall_on(&service, root.loadouts_file());
    let blocker = request(service.submit(loadout_edit(None, Loadout::new("blocker"))));
    stall.entered.recv_timeout(WAIT).unwrap();
    let queued = request(service.submit(hint_edit(&alice, None, vec![(995, 1)], 1)));
    let path = root.bank_hint_file(PROFILE, "alice");
    let outside = hint_document(PROFILE, "alice", HINT_SCHEMA_VERSION, 2, &[(995, 2)]);
    write_private(&path, &outside);
    // A session boundary asks for the file while the writer is held: its
    // readback runs before the queued write and adopts the change.
    let reader = {
        let service = Arc::clone(&service);
        let alice = alice.clone();
        std::thread::spawn(move || service.readback_lease(&alice, WAIT))
    };
    let key = files::StoreKey::of(&alice).unwrap();
    while service.shared.state.lock().stores[&key].readback != Readback::Queued {
        std::thread::yield_now();
    }
    stall.release.send(()).unwrap();
    let seen = reader.join().unwrap().unwrap().unwrap();
    assert_eq!(seen.snapshot().as_bank_hints().unwrap(), &[(995, 2)]);
    completed(&service, blocker);
    assert!(matches!(
        settlement(&service, queued),
        Settlement::Failed(ErrorCode::Conflict { .. })
    ));
    assert_eq!(std::fs::read(&path).unwrap(), outside);
}

/// r2 item 6 (sol probe): rows observed under one content scope are never
/// leased, or overwritten, under another.
#[test]
fn a_hint_written_under_one_content_scope_is_refused_under_another() {
    let scratch = IsolatedEnv::enter("local-state-content-scope");
    let root = StateRoot::new(scratch.home.join(".274bot"));
    let service = LocalStateService::open(
        root.clone(),
        &[LocalStateDomain::BankHints],
        Arc::new(RequestIdMinter::new()),
    );
    let under_a = scoped(LocalStateDomain::BankHints, "alice", "content-a");
    let under_b = scoped(LocalStateDomain::BankHints, "alice", "content-b");
    assert_eq!(service.readback_lease(&under_a, WAIT).unwrap(), None);
    completed(
        &service,
        request(service.submit(hint_edit(&under_a, None, vec![(995, 7)], 7))),
    );
    let path = root.bank_hint_file(PROFILE, "alice");
    let written = std::fs::read(&path).unwrap();

    let identity_refusal = ErrorCode::Failed {
        domain: LocalStateDomain::BankHints,
        reason: FailureReason::Identity,
    };
    assert_eq!(
        service.readback_lease(&under_b, WAIT),
        Err(identity_refusal)
    );
    assert_eq!(service.lease(&under_b), Err(identity_refusal));
    assert_eq!(
        service.submit(hint_edit(&under_b, None, vec![(995, 8)], 8)),
        SubmitReceipt::Refused(identity_refusal)
    );
    assert_eq!(std::fs::read(&path).unwrap(), written);
    let lease = service.lease(&under_a).unwrap();
    assert_eq!(lease.identity(), &under_a);
    assert_eq!(lease.snapshot().as_bank_hints().unwrap(), &[(995, 7)]);
}

/// r2 item 6: a hint written before scopes existed is read under the scope
/// that asks, and the first write stamps it with that scope.
#[test]
fn a_hint_written_before_scopes_is_read_then_stamped() {
    let scratch = IsolatedEnv::enter("local-state-legacy-hint");
    let root = StateRoot::new(scratch.home.join(".274bot"));
    let path = root.bank_hint_file(PROFILE, "alice");
    write_private(
        &path,
        &hint_document(PROFILE, "alice", HINT_SCHEMA_VERSION, 3, &[(995, 3)]),
    );
    let service = LocalStateService::open(
        root,
        &[LocalStateDomain::BankHints],
        Arc::new(RequestIdMinter::new()),
    );
    let alice = bank_identity("alice");
    let lease = service.readback_lease(&alice, WAIT).unwrap().unwrap();
    assert_eq!(lease.snapshot().as_bank_hints().unwrap(), &[(995, 3)]);
    completed(
        &service,
        request(service.submit(hint_edit(&alice, None, vec![(995, 4)], 4))),
    );
    let written: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(written["revision"], serde_json::json!(289));
    assert_eq!(written["content"], serde_json::json!(CONTENT));
}

/// r2 item 1: a write whose directory entry cannot be flushed is not
/// acknowledged durable and is not audited. The root is made `-wx` so the
/// file can be staged and renamed but the directory cannot be opened to
/// flush it.
#[cfg(unix)]
#[test]
fn a_write_whose_directory_entry_cannot_be_flushed_is_not_durable() {
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

/// r2 item 1: nothing acknowledges a write until its directory entries
/// are flushed: the receipt and the audit line both follow the flush, and
/// a file created in new directories flushes every entry up to the root's
/// parent.
#[test]
fn the_receipt_and_audit_follow_the_directory_flush() {
    let scratch = IsolatedEnv::enter("local-state-flush-order");
    let root = StateRoot::new(scratch.home.join(".274bot"));
    let service = LocalStateService::open(
        root.clone(),
        &[LocalStateDomain::BankHints],
        Arc::new(RequestIdMinter::new()),
    );
    let alice = bank_identity("alice");
    assert_eq!(service.readback_lease(&alice, WAIT).unwrap(), None);
    let events = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let (entered_tx, entered) = mpsc::channel();
    let (release, release_rx) = mpsc::channel::<()>();
    let release_rx = std::sync::Mutex::new(release_rx);
    let synced = Arc::clone(&events);
    service.set_sync_hook(Arc::new(move |dir| {
        if synced.lock().unwrap().is_empty() {
            let _ = entered_tx.send(());
            let _ = release_rx.lock().unwrap().recv();
        }
        synced.lock().unwrap().push(dir.display().to_string());
        files::sync_directory(dir)
    }));
    let audited = Arc::clone(&events);
    service.set_audit_hook(Arc::new(move |audit| {
        audited
            .lock()
            .unwrap()
            .push(format!("audit {}", audit.version()));
    }));

    let write = request(service.submit(hint_edit(&alice, None, vec![(995, 1)], 1)));
    entered.recv_timeout(WAIT).unwrap();
    assert_eq!(
        service.request_state(write),
        RequestState::Pending,
        "renamed but not flushed: not acknowledged"
    );
    release.send(()).unwrap();
    let receipt = completed(&service, write);
    let hint_dir = root.path().join("bank-hints").join(PROFILE);
    assert_eq!(
        *events.lock().unwrap(),
        vec![
            hint_dir.display().to_string(),
            root.path().join("bank-hints").display().to_string(),
            root.path().display().to_string(),
            scratch.home.display().to_string(),
            format!("audit {}", receipt.version()),
        ]
    );

    // A later write of the same file flushes its own directory only.
    events.lock().unwrap().clear();
    let second = request(service.submit(hint_edit(&alice, None, vec![(995, 2)], 2)));
    entered.recv_timeout(WAIT).unwrap();
    release.send(()).unwrap();
    let receipt = completed(&service, second);
    assert_eq!(
        *events.lock().unwrap(),
        vec![
            hint_dir.display().to_string(),
            format!("audit {}", receipt.version()),
        ]
    );
}

/// r2 item 2 (sol P1, grok probe): dropping the last service handle
/// finishes every accepted write, queued or in flight, and joins the
/// writer before it returns.
#[test]
fn dropping_the_service_finishes_every_accepted_write_and_joins_the_writer() {
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
    for (account, coins) in [("alice", 64), ("bob", 65)] {
        let raw = std::fs::read_to_string(root.bank_hint_file(PROFILE, account))
            .unwrap_or_else(|error| panic!("{account}'s queued hint was dropped: {error}"));
        assert!(raw.contains(&format!("[[995,{coins}]]")), "{raw}");
    }
}

/// r2 item 2: a write that panics settles its request (and those queued
/// behind it) instead of leaving them pending, and the writer keeps
/// serving.
#[test]
fn a_panicking_write_fails_its_request_and_the_writer_keeps_serving() {
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
    service.set_write_hook(Arc::new(move |path, bytes| {
        if !once.swap(true, Ordering::SeqCst) {
            panic!("injected writer panic");
        }
        vault::write_private_file(path, bytes)
    }));
    let doomed = request(service.submit(hint_edit(&alice, None, vec![(995, 1)], 1)));
    assert_eq!(
        settlement(&service, doomed),
        Settlement::Failed(ErrorCode::Failed {
            domain: LocalStateDomain::BankHints,
            reason: FailureReason::Dirty,
        })
    );
    assert_eq!(service.readback_lease(&alice, WAIT).unwrap(), None);
    let next = request(service.submit(hint_edit(&alice, None, vec![(995, 2)], 2)));
    completed(&service, next);
    assert!(
        std::fs::read_to_string(root.bank_hint_file(PROFILE, "alice"))
            .unwrap()
            .contains("[[995,2]]")
    );
}

/// r2 item 7 (sol P1): bank identities are capped; a new one past the cap
/// evicts the least recently used idle store, which reads its file again
/// when it is next asked for.
#[test]
fn bank_identities_are_capped_by_evicting_idle_stores() {
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
    let (entered_tx, entered) = mpsc::channel();
    let (release, release_rx) = mpsc::channel::<()>();
    let release_rx = std::sync::Mutex::new(release_rx);
    let blocker_path = root.bank_hint_file(PROFILE, "blocker");
    service.set_sync_hook(Arc::new(|_| Ok(())));
    service.set_write_hook(Arc::new(move |path, bytes| {
        if path == blocker_path {
            let _ = entered_tx.send(());
            let _ = release_rx.lock().unwrap().recv();
        }
        cheap_write(path, bytes)
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

/// A schema-1 hint document stamped for `identity`, as another writer of
/// this build would leave it.
fn stamped_hint(identity: &DomainIdentity, rows: &[(i32, i32)], at: u64) -> Vec<u8> {
    files::hint_bytes(
        &StoreKey::of(identity).unwrap(),
        rows,
        DurableTimestamp::from_unix_seconds(at),
    )
    .unwrap()
}

/// T5.2 r3 (grok P1 "between read and rename"): a writer that does not take
/// the lock (a hand edit, a build before r3) replaces the hint while this
/// service stages its write. The write is not acknowledged, nothing is
/// audited for it, the file keeps the other writer's bytes, and the
/// service now holds them.
#[test]
fn an_outside_write_while_a_write_is_staged_conflicts_and_keeps_the_outside_bytes() {
    let scratch = IsolatedEnv::enter("local-state-r3-staged-race");
    let root = StateRoot::new(scratch.home.join(".274bot"));
    let service = service(&scratch, &[LocalStateDomain::BankHints]);
    let alice = bank_identity("alice");
    let path = root.bank_hint_file(PROFILE, "alice");
    assert!(service.readback_lease(&alice, WAIT).unwrap().is_none());
    completed(
        &service,
        request(service.submit(hint_edit(&alice, None, vec![(995, 10)], 10))),
    );
    let audits = audits(&service);
    let outside = stamped_hint(&alice, &[(995, 77)], 77);
    let once = AtomicBool::new(false);
    let written = outside.clone();
    service.set_publish_hook(Arc::new(move |path| {
        if !once.swap(true, Ordering::SeqCst) {
            // An editor saving in place, taking no lock.
            cheap_write(path, &written).unwrap();
        }
    }));

    let edit = request(service.submit(hint_edit(&alice, None, vec![(995, 20)], 20)));
    let settled = settlement(&service, edit);
    assert!(
        matches!(settled, Settlement::Failed(ErrorCode::Conflict { .. })),
        "an outside replace during staging must conflict, not complete: {settled:?}"
    );
    assert_eq!(
        std::fs::read(&path).unwrap(),
        outside,
        "the outside bytes stay"
    );
    assert!(
        audits.lock().unwrap().is_empty(),
        "nothing committed, nothing audited"
    );
    let lease = service.lease(&alice).unwrap();
    assert_eq!(lease.snapshot().as_bank_hints().unwrap(), &[(995, 77)]);
}

/// T5.2 r3 (sol F1): a form of a build before r3 (or a hand edit) saves a
/// loadout row while the service stages its own edit. The service never
/// writes over the saved row: its edit conflicts and the row stays.
#[test]
fn a_form_save_during_a_service_publication_is_never_overwritten() {
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

/// T5.2 r3 (grok P1 "during the directory flush"): another writer replaces
/// the file after the rename, while the directory entry is being flushed.
/// The service reads the file back before acknowledging: it no longer holds
/// the written bytes, so the write is a conflict, never `Completed`, and no
/// audit line names it.
#[test]
fn an_outside_write_during_the_directory_flush_is_not_acknowledged() {
    let scratch = IsolatedEnv::enter("local-state-r3-flush-race");
    let root = StateRoot::new(scratch.home.join(".274bot"));
    let service = service(&scratch, &[LocalStateDomain::BankHints]);
    let alice = bank_identity("alice");
    let path = root.bank_hint_file(PROFILE, "alice");
    assert!(service.readback_lease(&alice, WAIT).unwrap().is_none());
    completed(
        &service,
        request(service.submit(hint_edit(&alice, None, vec![(995, 10)], 10))),
    );
    let audits = audits(&service);
    let outside = stamped_hint(&alice, &[(995, 88)], 88);
    let once = AtomicBool::new(false);
    let (hint_file, written) = (path.clone(), outside.clone());
    service.set_sync_hook(Arc::new(move |dir| {
        if !once.swap(true, Ordering::SeqCst) {
            write_private(&hint_file, &written);
        }
        files::sync_directory(dir)
    }));

    let edit = request(service.submit(hint_edit(&alice, None, vec![(995, 20)], 20)));
    let settled = settlement(&service, edit);
    assert!(
        !matches!(settled, Settlement::Completed(_)),
        "a receipt for bytes the file no longer holds is a false acknowledgement: {settled:?}"
    );
    assert!(
        matches!(settled, Settlement::Failed(ErrorCode::Conflict { .. })),
        "{settled:?}"
    );
    assert!(
        audits.lock().unwrap().is_empty(),
        "no audit for an unheld commit"
    );
    assert_eq!(std::fs::read(&path).unwrap(), outside);
    assert_eq!(
        service
            .lease(&alice)
            .unwrap()
            .snapshot()
            .as_bank_hints()
            .unwrap(),
        &[(995, 88)]
    );
}

/// T5.2 r3 (sol F1, grok P1): two services on one state root (two
/// compositions, or two processes on one `HOME`) never interleave on a
/// file: while one holds its write transaction the other waits for the
/// lock, then sees the first one's commit; no settlement is `Completed`
/// for a row the file does not hold.
#[test]
fn two_services_on_one_root_never_interleave_their_writes() {
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
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
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

/// T5.2 r3 (sol F2): the first write of a new hint fails flushing the state
/// root. The retry, though its bytes and digest match what the failed
/// attempt left, must flush every link of the chain again (profile
/// directory, `bank-hints`, the root, the root's parent) before it is
/// acknowledged and audited.
#[test]
fn a_retry_after_a_failed_ancestor_flush_flushes_the_whole_chain() {
    let scratch = IsolatedEnv::enter("local-state-r3-flush-retry");
    let root = StateRoot::new(scratch.home.join(".274bot"));
    let service = service(&scratch, &[LocalStateDomain::BankHints]);
    let alice = bank_identity("alice");
    assert_eq!(service.readback_lease(&alice, WAIT).unwrap(), None);
    let flushed = Arc::new(std::sync::Mutex::new(Vec::<std::path::PathBuf>::new()));
    let fail_root = Arc::new(AtomicBool::new(true));
    let (seen, fail, state_root) = (
        Arc::clone(&flushed),
        Arc::clone(&fail_root),
        root.path().to_path_buf(),
    );
    service.set_sync_hook(Arc::new(move |dir| {
        seen.lock().unwrap().push(dir.to_path_buf());
        if dir == state_root && fail.swap(false, Ordering::SeqCst) {
            return Err(std::io::Error::other("root flush failed"));
        }
        files::sync_directory(dir)
    }));
    let audits = audits(&service);

    let first = request(service.submit(hint_edit(&alice, None, vec![(995, 5)], 5)));
    assert!(matches!(
        settlement(&service, first),
        Settlement::Failed(ErrorCode::Failed {
            reason: FailureReason::Io,
            ..
        })
    ));
    assert!(audits.lock().unwrap().is_empty());

    flushed.lock().unwrap().clear();
    let retry = request(service.submit(hint_edit(&alice, None, vec![(995, 5)], 5)));
    let receipt = completed(&service, retry);
    let hint_dir = root.path().join("bank-hints").join(PROFILE);
    assert_eq!(
        *flushed.lock().unwrap(),
        vec![
            hint_dir,
            root.path().join("bank-hints"),
            root.path().to_path_buf(),
            scratch.home.clone(),
        ],
        "the retry re-flushes the unfinished chain"
    );
    let audits = audits.lock().unwrap();
    assert_eq!(audits.len(), 1);
    assert_eq!(audits[0].version(), receipt.version());
}

/// T5.2 r3 (grok P2): past the shutdown bound no accepted write is dropped
/// silently. A write the writer holds short of its rename and a write
/// queued behind it each settle `Cancelled(Shutdown)` with their own
/// terminal line; the held one is never renamed into place afterwards.
#[test]
fn past_the_shutdown_bound_every_accepted_write_settles_with_its_own_line() {
    let scratch = IsolatedEnv::enter("local-state-r3-shutdown-bound");
    let root = StateRoot::new(scratch.home.join(".274bot"));
    let service = service(&scratch, &[LocalStateDomain::BankHints]);
    service.set_shutdown_timeout(Duration::from_millis(300));
    let lines = terminals(&service);
    let (alice, bob) = (bank_identity("alice"), bank_identity("bob"));
    let (entered_tx, entered) = mpsc::channel();
    let (release, release_rx) = mpsc::channel::<()>();
    let release_rx = std::sync::Mutex::new(release_rx);
    service.set_publish_hook(Arc::new(move |_| {
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
    assert!(
        !root.bank_hint_file(PROFILE, "alice").exists(),
        "never renamed"
    );
    assert!(!root.bank_hint_file(PROFILE, "bob").exists());
}

/// T5.2 r3: a write already being renamed into place when the shutdown
/// bound passes is not cancelled: it settles when it finishes, durable and
/// audited.
#[test]
fn a_write_being_renamed_at_the_shutdown_bound_settles_when_it_finishes() {
    let scratch = IsolatedEnv::enter("local-state-r3-shutdown-publishing");
    let root = StateRoot::new(scratch.home.join(".274bot"));
    let service = service(&scratch, &[LocalStateDomain::BankHints]);
    service.set_shutdown_timeout(Duration::from_millis(300));
    let alice = bank_identity("alice");
    let path = root.bank_hint_file(PROFILE, "alice");
    assert!(service.readback_lease(&alice, WAIT).unwrap().is_none());
    let stall = stall_on(&service, path.clone());
    let audits = audits(&service);
    let write = request(service.submit(hint_edit(&alice, None, vec![(995, 3)], 3)));
    stall.entered.recv_timeout(WAIT).unwrap();
    let ledger = service.ledger();
    drop(service);
    assert_eq!(ledger.request_state(write), RequestState::Pending);
    stall.release.send(()).unwrap();
    assert!(ledger.wait_writer_exit(WAIT));
    assert!(matches!(
        ledger.request_state(write),
        RequestState::Settled(Settlement::Completed(_))
    ));
    assert_eq!(audits.lock().unwrap().len(), 1);
    assert!(path.exists());
}

/// T5.2 r3 (grok P2): a hint carrying a field this build does not know is
/// read, but never rewritten (the loadouts rule): the submission is refused
/// and the field stays.
#[test]
fn a_hint_with_a_field_this_build_does_not_know_is_read_but_never_rewritten() {
    let scratch = IsolatedEnv::enter("local-state-r3-hint-field");
    let root = StateRoot::new(scratch.home.join(".274bot"));
    let service = service(&scratch, &[LocalStateDomain::BankHints]);
    let alice = bank_identity("alice");
    let path = root.bank_hint_file(PROFILE, "alice");
    let document = serde_json::to_vec(&serde_json::json!({
        "schema_version": HINT_SCHEMA_VERSION,
        "profile": PROFILE,
        "account": "alice",
        "observed_at_unix": 7,
        "rows": [[995, 7]],
        "note": "keep",
    }))
    .unwrap();
    write_private(&path, &document);
    let lease = service.readback_lease(&alice, WAIT).unwrap().unwrap();
    assert_eq!(lease.snapshot().as_bank_hints().unwrap(), &[(995, 7)]);

    assert_eq!(
        service.submit(hint_edit(&alice, None, vec![(995, 8)], 8)),
        SubmitReceipt::Refused(ErrorCode::Failed {
            domain: LocalStateDomain::BankHints,
            reason: FailureReason::Schema,
        })
    );
    assert_eq!(std::fs::read(&path).unwrap(), document, "the note is kept");
}

fn preferences(scratch: &IsolatedEnv) -> Preferences {
    LocalStateService::open_process(StateRoot::new(scratch.home.join(".274bot"))).preferences()
}

/// T5.2 r3 (scope): the shared `panel-ui.json` is the preferences domain.
/// A write sets one member, keeps every other one (nested values too),
/// writes the pretty object the old shared writer wrote, and is audited.
#[test]
fn a_preference_write_sets_one_member_and_keeps_every_other() {
    let scratch = IsolatedEnv::enter("local-state-r3-preferences");
    let preferences = preferences(&scratch);
    let path = preferences.path();
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    write_private(
        &path,
        br#"{"last_focus":"alice","nav":{"allow_teleports":true,"custom":{"keep":7}},"collapsed":{}}"#,
    );
    let audits = audits(preferences.service());
    preferences.reload().unwrap();
    assert_eq!(preferences.value("last_focus").unwrap(), "alice");

    set_patiently(&preferences, "map_bake", serde_json::json!("always"));

    let mut expected: serde_json::Value = serde_json::json!({
        "last_focus": "alice",
        "nav": {"allow_teleports": true, "custom": {"keep": 7}},
        "collapsed": {},
    });
    expected["map_bake"] = serde_json::json!("always");
    assert_eq!(
        std::fs::read(&path).unwrap(),
        serde_json::to_vec_pretty(&expected).unwrap()
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
    let scratch = IsolatedEnv::enter("local-state-r3-preferences-wide");
    let preferences = preferences(&scratch);
    let path = preferences.path();
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
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
    write_private(&path, &serde_json::to_vec_pretty(&document).unwrap());
    preferences.reload().unwrap();
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
    let written: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(written, expected);
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
    let preferences = preferences(scratch);
    let path = preferences.path();
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let before = order_before();
    assert!(before["order"].to_string().len() > api::local_state::MAX_PREF_VALUE_BYTES);
    write_private(&path, &serde_json::to_vec_pretty(&before).unwrap());
    preferences.reload().unwrap();
    preferences
}

fn file_json(path: &std::path::Path) -> serde_json::Value {
    serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
}

/// F2 r3 (review R4): an update of an object too long for one row that
/// changes, removes and adds many members (a renumbered Load order) is
/// one edit and one file replacement: every member lands together.
#[test]
fn a_many_member_update_is_one_file_replacement() {
    let scratch = IsolatedEnv::enter("local-state-f2-one-replacement");
    let preferences = preferences_with_order(&scratch);
    let path = preferences.path();
    let writes = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&writes);
    preferences
        .service()
        .set_write_hook(Arc::new(move |path, bytes| {
            counted.fetch_add(1, Ordering::SeqCst);
            vault::write_private_file(path, bytes)
        }));
    let audits = audits(preferences.service());
    let after = order_renumbered();
    update_patiently(&preferences, |document| {
        document.insert("order".into(), after["order"].clone());
        Ok(())
    });
    assert_eq!(writes.load(Ordering::SeqCst), 1, "one file replacement");
    assert_eq!(audits.lock().unwrap().len(), 1, "one durable commit");
    assert_eq!(file_json(&path), after);
    assert_eq!(preferences.value("order"), Some(after["order"].clone()));
}

/// F2 r3 (review R4): a many-member update whose file replacement fails,
/// or whose writer dies inside it (an interrupted renumbering), leaves the
/// file and the held rows exactly as before: no member of it lands. The
/// next update lands whole.
#[test]
fn a_failed_or_interrupted_many_member_update_changes_nothing() {
    let scratch = IsolatedEnv::enter("local-state-f2-all-or-nothing");
    let preferences = preferences_with_order(&scratch);
    let path = preferences.path();
    let bytes_before = std::fs::read(&path).unwrap();
    // 0: the disk refuses the replacement; 1: the writer dies in it; then
    // the real write.
    let step = Arc::new(AtomicUsize::new(0));
    let hook_step = Arc::clone(&step);
    preferences
        .service()
        .set_write_hook(Arc::new(move |path, bytes| {
            match hook_step.load(Ordering::SeqCst) {
                0 => Err(std::io::Error::other("disk full")),
                1 => panic!("injected writer death mid-renumbering"),
                _ => vault::write_private_file(path, bytes),
            }
        }));
    let after = order_renumbered();
    let renumber = |document: &mut serde_json::Map<String, serde_json::Value>| {
        document.insert("order".into(), after["order"].clone());
        Ok(())
    };
    for (failure, case) in [(0, "refused"), (1, "interrupted")] {
        step.store(failure, Ordering::SeqCst);
        let error = preferences.update(renumber).expect_err(case);
        assert!(
            !error.to_string().contains("not durable within"),
            "{case}: {error}"
        );
        assert_eq!(std::fs::read(&path).unwrap(), bytes_before, "{case}: file");
        preferences.reload().unwrap();
        assert_eq!(
            preferences.value("order"),
            Some(order_before()["order"].clone()),
            "{case}: held rows"
        );
    }
    step.store(2, Ordering::SeqCst);
    update_patiently(&preferences, renumber);
    assert_eq!(file_json(&path), after);
}

/// F2 r3 (review R4): two writers (two services on one root, as two
/// processes) each replace every member of the large object while the
/// first holds its transaction. The second's edit conflicts, it reads and
/// merges again, and the file ends holding its object whole: never a mix
/// of the two, and both updates report durable.
#[test]
fn concurrent_many_member_updates_never_mix() {
    let scratch = IsolatedEnv::enter("local-state-f2-two-writers");
    let first = preferences_with_order(&scratch);
    let second = preferences(&scratch);
    second.reload().unwrap();
    let path = first.path();
    let (entered_tx, entered) = mpsc::channel();
    let (release, release_rx) = mpsc::channel::<()>();
    let release_rx = std::sync::Mutex::new(release_rx);
    let held = AtomicBool::new(false);
    first.service().set_publish_hook(Arc::new(move |_| {
        if !held.swap(true, Ordering::SeqCst) {
            let _ = entered_tx.send(());
            let _ = release_rx.lock().unwrap().recv();
        }
    }));
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
        file_json(&path),
        second_order,
        "the later writer's object, whole"
    );
}

/// T5.2 r3 (grok P2, the same rule in every domain): a preferences file
/// with a member the service cannot hold as a row is read, but never
/// rewritten: a write fails and the file keeps every byte.
#[test]
fn a_preferences_file_the_service_cannot_hold_whole_is_never_rewritten() {
    let scratch = IsolatedEnv::enter("local-state-r3-preferences-refused");
    let preferences = preferences(&scratch);
    let path = preferences.path();
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let document = br#"{"last_focus":"alice","not a key":1}"#;
    write_private(&path, document);
    preferences.reload().unwrap();
    assert_eq!(preferences.value("last_focus").unwrap(), "alice");
    assert!(preferences
        .set("map_bake", serde_json::json!("always"))
        .is_err());
    assert_eq!(std::fs::read(&path).unwrap(), document);
}

/// T5.2 r3 (scope, sol F1 by construction): a frontend's loadouts form and
/// a slot-side edit write through the one process service: they never
/// overwrite each other. The form's stale save conflicts, and saving again
/// keeps both rows.
#[test]
fn a_form_save_and_a_slot_edit_through_one_service_keep_both_rows() {
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
    // starved by machine load answers the first save with the bounded-wait
    // timeout instead: retry those, keep the first decisive answer.
    let start = std::time::Instant::now();
    let error = loop {
        match form.save() {
            Err(error)
                if error.contains("not durable within")
                    && start.elapsed() < Duration::from_secs(60) =>
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

/// Every line of a repair (a preferences quarantine, loadouts renamed
/// apart) the service logs, with its level.
fn quarantine_lines(service: &LocalStateService) -> Arc<std::sync::Mutex<Vec<(Level, String)>>> {
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

/// A preferences service on its own root under `scratch`.
fn preferences_at(scratch: &IsolatedEnv, case: &str) -> Preferences {
    let root = StateRoot::new(scratch.home.join(case).join(".274bot"));
    std::fs::create_dir_all(root.path()).unwrap();
    LocalStateService::open(
        root,
        &[LocalStateDomain::Preferences],
        Arc::new(RequestIdMinter::new()),
    )
    .preferences()
}

#[test]
fn quarantine_copy_flushes_its_candidate() {
    let scratch = IsolatedEnv::enter("local-state-quarantine-copy-flush");
    let path = scratch.home.join(".274bot").join("panel-ui.json");
    let bytes = b"quarantine contents".to_vec();
    write_private(&path, &bytes);

    let quarantined = files::quarantine_copy(&path, &bytes, "regression").unwrap();

    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    assert_eq!(std::fs::read(quarantined).unwrap(), bytes);
}

/// T5.2 r4 (the pre-T5.2 recovery, made safe): a preferences file whose
/// content is malformed (not JSON, not UTF-8, JSON that is not an object)
/// is refused by reads, so readers fail closed, and untouched by them. The
/// next save moves it aside to `panel-ui.json.corrupt-<time>` holding
/// exactly its bytes, flushed with its directory entry before the file is
/// replaced with fresh preferences, and then lands; the operator is told
/// (a warning) and one info audit line names the quarantine.
#[test]
fn a_malformed_preferences_file_is_quarantined_byte_for_byte_and_started_fresh() {
    let scratch = IsolatedEnv::enter("local-state-r4-preferences-quarantine");
    for (case, bytes) in [
        ("not-json", b"{not valid json".to_vec()),
        ("not-utf8", vec![b'{', 0xff, 0xfe, b'}']),
        ("not-an-object", b"[1, 2, 3]".to_vec()),
    ] {
        let preferences = preferences_at(&scratch, case);
        let path = preferences.path();
        write_private(&path, &bytes);
        let notices = quarantine_lines(preferences.service());
        let order = Arc::new(std::sync::Mutex::new(Vec::new()));
        let synced = Arc::clone(&order);
        preferences.service().set_sync_hook(Arc::new(move |dir| {
            synced
                .lock()
                .unwrap()
                .push(format!("sync {}", dir.display()));
            files::sync_directory(dir)
        }));
        let written = Arc::clone(&order);
        preferences
            .service()
            .set_write_hook(Arc::new(move |path, bytes| {
                written
                    .lock()
                    .unwrap()
                    .push(format!("write {}", path.display()));
                vault::write_private_file(path, bytes)
            }));

        assert!(preferences.reload().is_err(), "{case}: reads refuse it");
        assert_eq!(
            std::fs::read(&path).unwrap(),
            bytes,
            "{case}: reads keep it"
        );
        assert_eq!(siblings(&path), ["panel-ui.json"], "{case}");
        assert!(
            order.lock().unwrap().is_empty(),
            "{case}: reads write nothing"
        );

        patiently(|| {
            preferences
                .set("map_bake", serde_json::json!("always"))
                .map_err(|error| error.to_string())
        })
        .expect(case);
        assert_eq!(
            std::fs::read(&path).unwrap(),
            serde_json::to_vec_pretty(&serde_json::json!({"map_bake": "always"})).unwrap(),
            "{case}: the save lands in fresh preferences"
        );
        assert_eq!(preferences.value("map_bake").unwrap(), "always");
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
        assert_eq!(
            order.lock().unwrap()[..3],
            [
                format!("sync {}", path.parent().unwrap().display()),
                format!("write {}", path.display()),
                format!("write {}", path.display()),
            ],
            "{case}: the quarantine's entry is flushed before the file is replaced \
             with fresh preferences, and then the save is written"
        );
        let notices = notices.lock().unwrap().clone();
        assert_eq!(notices.len(), 2, "{case}: {notices:?}");
        assert_eq!(notices[0].0, Level::Warn, "{case}: the operator's notice");
        assert!(
            notices[0].1.contains(&quarantined.display().to_string()),
            "{case}: {notices:?}"
        );
        assert_eq!(notices[1].0, Level::Info, "{case}: the audit line");
        assert!(
            notices[1]
                .1
                .starts_with("audit domain=preferences action=preferences-quarantine"),
            "{case}: {notices:?}"
        );
        assert!(
            notices[1]
                .1
                .contains(&format!("quarantined={}", quarantined.display())),
            "{case}: {notices:?}"
        );
        assert_eq!(std::fs::read(&quarantined).unwrap(), bytes, "{case}: kept");
    }
}

/// T5.2 r4: the quarantine runs under the preferences file's write lock:
/// while another writer holds it, a save waits and the malformed file
/// stays where it is.
#[test]
fn a_preferences_quarantine_waits_for_the_files_lock() {
    let scratch = IsolatedEnv::enter("local-state-r4-preferences-quarantine-lock");
    let preferences = preferences_at(&scratch, "locked");
    let path = preferences.path();
    let bytes = b"{not valid json".to_vec();
    write_private(&path, &bytes);
    let lock = files::lock_for_write(&path, std::time::Instant::now() + WAIT, &|| false).unwrap();
    let saver = {
        let preferences = preferences.clone();
        std::thread::spawn(move || preferences.set("map_bake", serde_json::json!("always")))
    };
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(
        std::fs::read(&path).unwrap(),
        bytes,
        "not moved under another's lock"
    );
    assert_eq!(siblings(&path), ["panel-ui.json"]);
    drop(lock);
    // The save stays queued past the frontend's bounded wait under load and
    // settles later: retry a timed-out save against what is on disk now.
    let mut saved = saver.join().unwrap();
    let start = std::time::Instant::now();
    while start.elapsed() < Duration::from_secs(60) {
        match saved {
            Ok(()) => break,
            Err(error) if error.to_string().contains("not durable within") => {
                saved = preferences.set("map_bake", serde_json::json!("always"));
            }
            Err(error) => panic!("save failed: {error}"),
        }
    }
    saved.unwrap();
    let found = siblings(&path);
    assert_eq!(found.len(), 2, "{found:?}");
    assert_eq!(
        std::fs::read(path.with_file_name(&found[1])).unwrap(),
        bytes
    );
}

/// T5.2 r4, r5 (Sol r4 F2): only malformed content is quarantined. A
/// preferences file refused for the file itself (its size) or holding a
/// member this build cannot hold as a row (here `nav.v2`: a member name
/// with a `.` in it, which no row key can carry back as it was read) is
/// never moved or replaced: reads or writes refuse visibly, every byte
/// stays, and no quarantine file appears. The file carries no version
/// marker: the refusal is the unrepresentable member's alone (the marker
/// case is the next test).
#[test]
fn preferences_refused_for_size_or_an_unrepresentable_member_are_never_quarantined() {
    let scratch = IsolatedEnv::enter("local-state-r4-preferences-refused");

    // Larger than the bound (and not JSON either): refused for its size.
    let preferences = preferences_at(&scratch, "oversize");
    let path = preferences.path();
    let oversize = vec![b'x'; MAX_PREFERENCES_BYTES as usize + 1];
    write_private(&path, &oversize);
    let notices = quarantine_lines(preferences.service());
    assert!(preferences.reload().is_err(), "oversize is refused");
    assert!(preferences
        .set("map_bake", serde_json::json!("always"))
        .is_err());
    assert!(std::fs::read(&path).unwrap() == oversize, "oversize kept");
    assert_eq!(siblings(&path), ["panel-ui.json"]);
    assert!(notices.lock().unwrap().is_empty());

    // Valid JSON with a member this build cannot hold as a row, and no
    // version marker: read, never rewritten.
    let preferences = preferences_at(&scratch, "unrepresentable");
    let path = preferences.path();
    let unrepresentable = br#"{"last_focus":"alice","nav.v2":{"zones":"all"}}"#;
    write_private(&path, unrepresentable);
    let notices = quarantine_lines(preferences.service());
    preferences.reload().unwrap();
    assert_eq!(preferences.value("last_focus").unwrap(), "alice");
    assert!(preferences
        .set("map_bake", serde_json::json!("always"))
        .is_err());
    assert_eq!(
        std::fs::read(&path).unwrap(),
        unrepresentable,
        "the unrepresentable document is kept"
    );
    assert_eq!(siblings(&path), ["panel-ui.json"]);
    assert!(notices.lock().unwrap().is_empty());
}

/// T5.2 r5 (Sol r4 F2): the preferences rule is lossless
/// representability, not a version or known-member check. `panel-ui.json`
/// has no version marker of its own, so a `schema_version` member another
/// build wrote is a member like any other: when every member can be held
/// as a row, a save keeps them all, a member this build does not know
/// among them, adds its own, and nothing is refused or moved aside.
#[test]
fn a_preferences_version_marker_is_kept_by_a_save_like_any_other_member() {
    let scratch = IsolatedEnv::enter("local-state-r5-preferences-version-marker");
    let preferences = preferences_at(&scratch, "marker");
    let path = preferences.path();
    write_private(
        &path,
        br#"{"schema_version":99,"last_focus":"alice","future_preference":{"enabled":true}}"#,
    );
    let notices = quarantine_lines(preferences.service());
    preferences.reload().unwrap();
    assert_eq!(preferences.value("last_focus").unwrap(), "alice");
    preferences
        .set("map_bake", serde_json::json!("always"))
        .expect("a version marker and an unknown member do not refuse a save");
    let saved: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(
        saved,
        serde_json::json!({
            "schema_version": 99,
            "last_focus": "alice",
            "future_preference": {"enabled": true},
            "map_bake": "always",
        }),
        "every member is kept, the marker and the unknown one too"
    );
    assert_eq!(siblings(&path), ["panel-ui.json"]);
    assert!(notices.lock().unwrap().is_empty());
}

/// T5.2 r4: a preferences file refused for its permissions is never
/// quarantined, whatever it holds: group/other-writable or unreadable,
/// even with malformed content, it is refused and keeps every byte.
#[cfg(unix)]
#[test]
fn preferences_refused_for_permissions_are_never_quarantined() {
    if running_as_root() {
        return;
    }
    let scratch = IsolatedEnv::enter("local-state-r4-preferences-permissions");
    for mode in [0o666, 0o000] {
        let preferences = preferences_at(&scratch, &format!("mode-{mode:o}"));
        let path = preferences.path();
        let bytes = b"{not valid json".to_vec();
        write_private(&path, &bytes);
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode)).unwrap();
        let notices = quarantine_lines(preferences.service());
        let refused = preferences.reload();
        assert!(refused.is_err(), "mode {mode:o}: refused");
        assert!(
            preferences
                .set("map_bake", serde_json::json!("always"))
                .is_err(),
            "mode {mode:o}"
        );
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), bytes, "mode {mode:o}: kept");
        assert_eq!(siblings(&path), ["panel-ui.json"], "mode {mode:o}");
        assert!(notices.lock().unwrap().is_empty(), "mode {mode:o}");
    }
}

/// The rows of the loadouts file at `path`.
fn rows_on_disk(path: &std::path::Path) -> Vec<Loadout> {
    serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
}

/// T5.2 r5 (Sol r4 F1, the one-time migration): a `loadouts.json` whose
/// rows share a name (an older editor could leave one) is renamed apart by
/// the read that finds it, before anything is leased or written. Its exact
/// bytes are kept first in a new `loadouts.json.duplicates-<time>`, whose
/// directory entry is flushed before the file is replaced; every later row
/// of a name becomes `<name> (2)`, `(3)`, … in row order, a suffix another
/// row has skipped; every row keeps its content and its place; the result
/// is one publication, flushed. The operator is told (a warning), and one
/// info audit line names the copy and each rename. Later reads, by this
/// service or another, rename nothing.
#[test]
fn rows_sharing_a_name_are_renamed_apart_once_and_the_file_is_kept_byte_for_byte() {
    let scratch = IsolatedEnv::enter("local-state-r5-dedupe");
    let root = StateRoot::new(scratch.home.join(".274bot"));
    let path = root.loadouts_file();
    let service = service(&scratch, &[LocalStateDomain::LoadoutsReferences]);
    assert_eq!(
        service.readback_lease(&loadouts_identity(), WAIT).unwrap(),
        None
    );
    let notices = quarantine_lines(&service);
    let commits = audits(&service);
    let order = Arc::new(std::sync::Mutex::new(Vec::new()));
    let synced = Arc::clone(&order);
    service.set_sync_hook(Arc::new(move |dir| {
        synced
            .lock()
            .unwrap()
            .push(format!("sync {}", dir.display()));
        files::sync_directory(dir)
    }));
    let written = Arc::clone(&order);
    service.set_write_hook(Arc::new(move |path, bytes| {
        written
            .lock()
            .unwrap()
            .push(format!("write {}", path.display()));
        vault::write_private_file(path, bytes)
    }));
    // As an older build's form could leave it, written compact with a
    // trailing newline, so the copy is shown to be the file's own bytes
    // and not a rewrite of its rows.
    let legacy = [
        Loadout::new("same").with_carry("Lobster", 1),
        Loadout::new("same")
            .with_slot("righthand", "Rune scimitar")
            .with_carry("Shark", 2),
        Loadout::new("same (2)").with_carry("Tuna", 3),
        Loadout::new("other"),
        Loadout::new("same").with_carry("Trout", 4),
    ];
    let mut bytes = serde_json::to_vec(&legacy).unwrap();
    bytes.push(b'\n');
    write_private(&path, &bytes);

    let lease = service
        .readback_lease(&loadouts_identity(), WAIT)
        .unwrap()
        .unwrap();
    let mut apart = legacy.to_vec();
    apart[1].name = "same (3)".into();
    apart[4].name = "same (4)".into();
    assert_eq!(
        lease.snapshot().as_loadouts().unwrap(),
        apart.as_slice(),
        "the later rows of `same` are renamed apart (`same (2)` is another row's), each \
         keeping its gear and its place"
    );
    assert_eq!(
        std::fs::read(&path).unwrap(),
        serde_json::to_vec_pretty(&apart).unwrap(),
        "one publication of the renamed rows"
    );
    let found = siblings(&path);
    assert_eq!(found.len(), 2, "{found:?}");
    assert!(
        found[1].starts_with("loadouts.json.duplicates-"),
        "{found:?}"
    );
    let quarantined = path.with_file_name(&found[1]);
    assert_eq!(
        std::fs::read(&quarantined).unwrap(),
        bytes,
        "the copy holds the file's exact bytes"
    );
    let dir = path.parent().unwrap().display().to_string();
    assert_eq!(
        *order.lock().unwrap(),
        [
            format!("sync {dir}"),
            format!("write {}", path.display()),
            format!("sync {dir}"),
        ],
        "the copy's entry is flushed before the file is replaced, the replacement's after"
    );
    let renames = r#"row 2 "same" → "same (3)", row 5 "same" → "same (4)""#;
    let lines = notices.lock().unwrap().clone();
    assert_eq!(lines.len(), 2, "{lines:?}");
    assert_eq!(lines[0].0, Level::Warn, "the operator's notice");
    assert!(
        lines[0].1.contains(&quarantined.display().to_string()) && lines[0].1.contains(renames),
        "{lines:?}"
    );
    assert_eq!(lines[1].0, Level::Info, "the audit line");
    assert!(
        lines[1]
            .1
            .starts_with("audit domain=loadouts-references action=loadouts-dedupe "),
        "{lines:?}"
    );
    assert!(
        lines[1].1.contains(&format!(
            "path={} quarantined={} renamed=[{renames}]",
            path.display(),
            quarantined.display()
        )),
        "{lines:?}"
    );
    assert!(
        commits.lock().unwrap().is_empty(),
        "a repair, not a submitted write"
    );

    // Apart now: nothing more to rename, here or in another process.
    let again = LocalStateService::open(
        root,
        &[LocalStateDomain::LoadoutsReferences],
        Arc::new(RequestIdMinter::new()),
    );
    for reader in [&service, &again] {
        let lease = reader
            .readback_lease(&loadouts_identity(), WAIT)
            .unwrap()
            .unwrap();
        assert_eq!(lease.snapshot().as_loadouts().unwrap(), apart.as_slice());
    }
    assert_eq!(siblings(&path), found, "no second copy");
    assert_eq!(notices.lock().unwrap().len(), 2, "nothing more to tell");
}

/// T5.2 r5 (Sol r4 F1, `probe-legacy-duplicate` made permanent): on a
/// `loadouts.json` an older editor saved with two loadouts named `same`,
/// renaming the second through the service-backed form (as both editors
/// do: by its place in the list, with `replace_at`) renames that row, with
/// its gear, in its place, and the first keeps its own gear. Before r5 the
/// rename replaced the first row of the name, and the first row's gear was
/// gone.
#[test]
fn renaming_the_second_of_two_same_named_loadouts_keeps_the_first_ones_gear() {
    let scratch = IsolatedEnv::enter("local-state-r5-legacy-duplicate");
    let root = StateRoot::new(scratch.home.join(".274bot"));
    let path = root.loadouts_file();
    let first = Loadout::new("same").with_carry("Lobster", 1);
    let second = Loadout::new("same").with_carry("Shark", 2);
    write_loadouts(
        &path,
        &[first.clone(), second.clone(), Loadout::new("other")],
    );
    let service = service(&scratch, &[LocalStateDomain::LoadoutsReferences]);
    let mut editor = form(&service);
    let mut renamed = editor.loadouts()[1].clone();
    renamed.name = "renamed-second".into();
    assert!(editor.replace_at(1, renamed));
    save_patiently(&mut editor);
    let rows = rows_on_disk(&path);
    assert!(
        rows.contains(&first),
        "renaming the second duplicate-name row destroyed the first saved loadout's gear: \
         {rows:?}"
    );
    let mut second_renamed = second;
    second_renamed.name = "renamed-second".into();
    assert_eq!(
        rows,
        [first, second_renamed, Loadout::new("other")],
        "the second row is renamed, with its gear, in its place"
    );
    assert_eq!(editor.snapshot(), rows);
}

/// A root whose `loadouts.json` an older editor left with three loadouts
/// named `same`, each with its own gear, around another, and a
/// service-backed form loaded over it (the file is migrated by then).
struct Legacy {
    path: std::path::PathBuf,
    editor: script::LoadoutsStore,
    _service: Arc<LocalStateService>,
    _scratch: IsolatedEnv,
}

fn legacy(case: &str) -> Legacy {
    let scratch = IsolatedEnv::enter(case);
    let path = StateRoot::new(scratch.home.join(".274bot")).loadouts_file();
    write_loadouts(
        &path,
        &[
            Loadout::new("same").with_carry("Lobster", 1),
            Loadout::new("same").with_carry("Shark", 2),
            Loadout::new("other").with_carry("Coal", 3),
            Loadout::new("same").with_carry("Trout", 4),
        ],
    );
    let service = service(&scratch, &[LocalStateDomain::LoadoutsReferences]);
    service
        .readback_lease(&loadouts_identity(), WAIT)
        .expect("initial legacy loadouts readback")
        .expect("the legacy fixture has a saved loadouts file");
    let editor = form(&service);
    assert!(
        editor.load_error().is_none(),
        "legacy loadouts form failed to load: {:?}",
        editor.load_error()
    );
    Legacy {
        path,
        editor,
        _service: service,
        _scratch: scratch,
    }
}

/// T5.2 r5 (F1): on a migrated legacy file every edit names one row, so
/// saving the selected loadout changed changes it and no other. The
/// editors select a row by its place in the list and save it with
/// `replace_at` (panel `save_draft`, TUI `commit_scratch`): an upsert of
/// its name. Before r5 the name was every `same` row's: the upsert
/// replaced the first, and the save's other rows of the name overwrote it.
#[test]
fn on_a_migrated_legacy_file_saving_a_changed_loadout_changes_only_it() {
    for selected in [0, 1, 3] {
        let mut case = legacy(&format!("local-state-r5-legacy-upsert-{selected}"));
        let mut expected = case.editor.snapshot();
        let changed = expected[selected].clone().with_carry("Coins", 99);
        assert!(case.editor.replace_at(selected, changed.clone()));
        save_patiently(&mut case.editor);
        expected[selected] = changed;
        assert_eq!(
            rows_on_disk(&case.path),
            expected,
            "saving row {selected} changed that row and no other"
        );
        assert_eq!(case.editor.snapshot(), expected);
    }
}

/// T5.2 r5 (F1): on a migrated legacy file removing the selected loadout
/// removes it and no other. The editors remove the selected row by its
/// name (panel `delete_loadout`, TUI row 7). Before r5 the name was every
/// `same` row's, so no removal could name the selected one.
#[test]
fn on_a_migrated_legacy_file_removing_a_loadout_removes_only_it() {
    for selected in [0, 1, 3] {
        let mut case = legacy(&format!("local-state-r5-legacy-remove-{selected}"));
        let mut expected = case.editor.snapshot();
        let name = expected.remove(selected).name;
        assert!(case.editor.remove(&name));
        save_patiently(&mut case.editor);
        assert_eq!(
            rows_on_disk(&case.path),
            expected,
            "removing row {selected} removed that row and no other"
        );
        assert_eq!(case.editor.snapshot(), expected);
    }
}

/// T5.2 r5 (F1): on a migrated legacy file renaming the selected loadout
/// renames it, with its gear and in its place, and no other (the editors'
/// `replace_at` with a new name: one rename of its name). Before r5 the
/// rename took the first `same` row.
#[test]
fn on_a_migrated_legacy_file_renaming_a_loadout_renames_only_it() {
    for selected in [0, 1, 3] {
        let mut case = legacy(&format!("local-state-r5-legacy-rename-{selected}"));
        let mut expected = case.editor.snapshot();
        expected[selected].name = "picked".into();
        assert!(case.editor.replace_at(selected, expected[selected].clone()));
        save_patiently(&mut case.editor);
        assert_eq!(
            rows_on_disk(&case.path),
            expected,
            "renaming row {selected} renamed that row and no other"
        );
        assert_eq!(case.editor.snapshot(), expected);
    }
}

/// T5.2 r5 (F1): the names a rename apart gives. Per name, the second row
/// becomes `<name> (2)`, the third `<name> (3)`, and so on in row order; a
/// suffix another row (or an earlier rename) already has is skipped; the
/// first row of each name keeps it, and every row keeps its content and
/// its place. The same rows always get the same names, and rows already
/// apart get none.
#[test]
fn rows_sharing_a_name_get_the_next_free_suffix_in_row_order() {
    let gear = |name: &str, coins: u32| Loadout::new(name).with_carry("Coins", coins);
    let rows = vec![
        gear("a", 1),
        gear("a", 2),
        gear("a (2)", 3),
        gear("b", 4),
        gear("a", 5),
        gear("b", 6),
        gear("a (2)", 7),
    ];
    let mut apart = rows.clone();
    let made = files::rename_apart(&mut apart);
    let names: Vec<&str> = apart.iter().map(|row| row.name.as_str()).collect();
    assert_eq!(
        names,
        ["a", "a (3)", "a (2)", "b", "a (4)", "b (2)", "a (2) (2)"]
    );
    for (before, after) in rows.iter().zip(&apart) {
        assert_eq!(
            before.carry, after.carry,
            "each row keeps its gear, in place"
        );
    }
    let renamed = |row: usize, from: &str, to: &str| files::RenamedApart {
        row,
        from: from.into(),
        to: to.into(),
    };
    assert_eq!(
        made,
        [
            renamed(1, "a", "a (3)"),
            renamed(4, "a", "a (4)"),
            renamed(5, "b", "b (2)"),
            renamed(6, "a (2)", "a (2) (2)"),
        ]
    );
    let mut again = rows.clone();
    assert_eq!(files::rename_apart(&mut again), made, "deterministic");
    assert!(
        files::rename_apart(&mut apart).is_empty(),
        "rows already apart get no new names"
    );
}

/// T5.2 r5 review (R5-F1): a renamed-apart row stays addressable. Remove and
/// Rename refuse a trimmed name over `MAX_PREF_KEY_CHARS` bytes, so the new
/// name is shortened (on a character boundary) to fit, for names at the
/// bound in one-byte and multi-byte text and with leading spaces, and the
/// shortened names still come out distinct from each other and from rows
/// already named that way.
#[test]
fn names_made_apart_stay_within_the_remove_and_rename_bound() {
    let max = api::local_state::MAX_PREF_KEY_CHARS;
    let ascii = "a".repeat(max);
    let accented = "é".repeat(max / 2);
    let spaced = format!("  {}", "b".repeat(max));
    let taken = files::new_name_apart(&ascii, 2);
    let gear = |name: &str, coins: u32| Loadout::new(name).with_carry("Coins", coins);
    let rows = vec![
        gear(&ascii, 1),
        gear(&ascii, 2),
        gear(&ascii, 3),
        gear(&taken, 4),
        gear(&accented, 5),
        gear(&accented, 6),
        gear(&spaced, 7),
        gear(&spaced, 8),
    ];
    let mut apart = rows.clone();
    let made = files::rename_apart(&mut apart);
    assert_eq!(made.len(), 4, "the second and third of each name: {made:?}");
    for change in &made {
        let trimmed = change.to.trim();
        assert!(
            trimmed.len() <= max,
            "{:?} is {} bytes trimmed",
            change.to,
            trimmed.len()
        );
        assert!(!trimmed.is_empty());
    }
    let mut names: Vec<&str> = apart.iter().map(|row| row.name.as_str()).collect();
    names.sort_unstable();
    names.dedup();
    assert_eq!(names.len(), rows.len(), "every row has its own name");
    for (before, after) in rows.iter().zip(&apart) {
        assert_eq!(before.carry, after.carry, "each row keeps its gear");
    }
}

/// T5.2 r5 (F1): rows another writer leaves sharing a name while this
/// service holds the file (an older build's form on the same home) are
/// renamed apart by the next write's own read, under that write's lock,
/// before the write is decided. That write settles as a conflict (the file
/// changed under it), and the next edit names one row: no edit runs against
/// rows that share a name.
#[test]
fn rows_another_writer_leaves_sharing_a_name_are_renamed_apart_before_an_edit() {
    let scratch = IsolatedEnv::enter("local-state-r5-outside-duplicates");
    let root = StateRoot::new(scratch.home.join(".274bot"));
    let path = root.loadouts_file();
    write_loadouts(&path, &[Loadout::new("same"), Loadout::new("other")]);
    let service = service(&scratch, &[LocalStateDomain::LoadoutsReferences]);
    let version = service
        .readback_lease(&loadouts_identity(), WAIT)
        .unwrap()
        .unwrap()
        .version();
    let notices = quarantine_lines(&service);
    let left = [
        Loadout::new("same"),
        Loadout::new("other"),
        Loadout::new("same").with_carry("Shark", 2),
    ];
    write_loadouts(&path, &left);
    let remove = |expected| {
        DomainEdit::Loadout(LoadoutEdit::remove(loadouts_identity(), expected, "same").unwrap())
    };
    let stale = request(service.submit(remove(Some(version))));
    assert!(
        matches!(
            settlement(&service, stale),
            Settlement::Failed(ErrorCode::Conflict { .. })
        ),
        "the removal was written against another file"
    );
    let mut apart = left.to_vec();
    apart[2].name = "same (2)".into();
    assert_eq!(
        rows_on_disk(&path),
        apart,
        "renamed apart under the write's lock, and the stale removal not applied"
    );
    assert!(
        notices.lock().unwrap().iter().any(|(level, line)| {
            *level == Level::Info
                && line.starts_with("audit domain=loadouts-references action=loadouts-dedupe ")
        }),
        "audited"
    );
    let current = service.lease(&loadouts_identity()).unwrap().version();
    completed(&service, request(service.submit(remove(Some(current)))));
    assert_eq!(
        rows_on_disk(&path),
        &apart[1..],
        "the removal named one row"
    );
}

/// T5.2 r5 (F1): the copy a rename apart keeps never replaces an existing
/// file: with the `loadouts.json.duplicates-<time>` names of the next few
/// seconds already taken (earlier copies), the new copy takes the next
/// free name, holds the file's exact bytes, and every earlier copy keeps
/// its own.
#[test]
fn a_rename_apart_never_overwrites_an_earlier_copy() {
    let scratch = IsolatedEnv::enter("local-state-r5-dedupe-copy-names");
    let root = StateRoot::new(scratch.home.join(".274bot"));
    let path = root.loadouts_file();
    write_loadouts(
        &path,
        &[
            Loadout::new("same").with_carry("Lobster", 1),
            Loadout::new("same").with_carry("Shark", 2),
        ],
    );
    let bytes = std::fs::read(&path).unwrap();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let earlier: Vec<String> = (now..now + 3)
        .map(|second| format!("loadouts.json.duplicates-{second}"))
        .collect();
    for name in &earlier {
        write_private(&path.with_file_name(name), b"an earlier copy");
    }
    let service = service(&scratch, &[LocalStateDomain::LoadoutsReferences]);
    let lease = service
        .readback_lease(&loadouts_identity(), WAIT)
        .unwrap()
        .unwrap();
    assert_eq!(lease.snapshot().as_loadouts().unwrap()[1].name, "same (2)");
    for name in &earlier {
        assert_eq!(
            std::fs::read(path.with_file_name(name)).unwrap(),
            b"an earlier copy",
            "{name} keeps its bytes"
        );
    }
    let made: Vec<String> = siblings(&path)
        .into_iter()
        .filter(|name| name.starts_with("loadouts.json.duplicates-") && !earlier.contains(name))
        .collect();
    assert_eq!(made.len(), 1, "one new copy: {made:?}");
    assert_eq!(
        std::fs::read(path.with_file_name(&made[0])).unwrap(),
        bytes,
        "the new copy holds the file's exact bytes"
    );
}

/// T5.2 r5 (F1): the rename apart runs under the loadouts file's write
/// lock: while another writer holds it, nothing is renamed or copied; once
/// it is released, the waiting read renames the file apart.
#[test]
fn a_rename_apart_waits_for_the_files_lock() {
    let scratch = IsolatedEnv::enter("local-state-r5-dedupe-lock");
    let root = StateRoot::new(scratch.home.join(".274bot"));
    let path = root.loadouts_file();
    let service = service(&scratch, &[LocalStateDomain::LoadoutsReferences]);
    assert_eq!(
        service.readback_lease(&loadouts_identity(), WAIT).unwrap(),
        None
    );
    let rows = [
        Loadout::new("same").with_carry("Lobster", 1),
        Loadout::new("same").with_carry("Shark", 2),
    ];
    write_loadouts(&path, &rows);
    let bytes = std::fs::read(&path).unwrap();
    let lock = files::lock_for_write(&path, std::time::Instant::now() + WAIT, &|| false).unwrap();
    let reader = {
        let service = Arc::clone(&service);
        std::thread::spawn(move || service.readback_lease(&loadouts_identity(), WAIT))
    };
    // The read has begun, found the rows sharing a name, and waits for the
    // lock to rename them apart.
    let deadline = std::time::Instant::now() + WAIT;
    while service.shared.state.lock().stores[&files::StoreKey::Loadouts].readback
        != Readback::InFlight
        && !reader.is_finished()
        && std::time::Instant::now() < deadline
    {
        std::thread::yield_now();
    }
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(
        std::fs::read(&path).unwrap(),
        bytes,
        "not renamed under another's lock"
    );
    assert_eq!(
        siblings(&path),
        ["loadouts.json"],
        "no copy under another's lock"
    );
    drop(lock);
    let lease = reader.join().unwrap().unwrap().unwrap();
    let mut apart = rows.to_vec();
    apart[1].name = "same (2)".into();
    assert_eq!(lease.snapshot().as_loadouts().unwrap(), apart.as_slice());
    let found = siblings(&path);
    assert_eq!(found.len(), 2, "{found:?}");
    assert_eq!(
        std::fs::read(path.with_file_name(&found[1])).unwrap(),
        bytes
    );
}

/// What a rename apart that was not made leaves: the file refused (no
/// lease, and no edit: none runs against rows that share a name), every
/// byte of it kept, any copy made holding exactly those bytes, and the
/// operator told why, with nothing audited.
fn assert_refused_and_untouched(
    service: &LocalStateService,
    path: &std::path::Path,
    bytes: &[u8],
    notices: &std::sync::Mutex<Vec<(Level, String)>>,
    case: &str,
) {
    let refused = service.readback_lease(&loadouts_identity(), WAIT);
    assert!(
        matches!(
            refused,
            Err(ErrorCode::Failed {
                domain: LocalStateDomain::LoadoutsReferences,
                reason: FailureReason::Schema,
            })
        ),
        "{case}: refused, not leased: {refused:?}"
    );
    assert!(
        matches!(
            service.lease(&loadouts_identity()),
            Err(ErrorCode::Failed { .. })
        ),
        "{case}: no lease"
    );
    assert!(
        matches!(
            service.submit(loadout_edit(None, Loadout::new("new"))),
            SubmitReceipt::Refused(ErrorCode::Failed { .. })
        ),
        "{case}: no edit"
    );
    // The refused submission asked for another read: let it finish.
    let _ = service.readback_lease(&loadouts_identity(), WAIT);
    assert_eq!(
        std::fs::read(path).unwrap(),
        bytes,
        "{case}: the file keeps every byte"
    );
    for copy in siblings(path)
        .iter()
        .filter(|name| name.starts_with("loadouts.json.duplicates-"))
    {
        assert_eq!(
            std::fs::read(path.with_file_name(copy)).unwrap(),
            bytes,
            "{case}: a copy left behind holds the file's exact bytes"
        );
    }
    let lines = notices.lock().unwrap().clone();
    assert!(!lines.is_empty(), "{case}: the operator is told");
    assert!(
        lines.iter().all(|(level, line)| *level == Level::Warn
            && line.contains("could not be renamed apart")
            && line.contains(&path.display().to_string())),
        "{case}: told why, and nothing audited: {lines:?}"
    );
}

/// T5.2 r5 (F1): a rename apart that cannot be made whole is not made: the
/// file keeps every byte, the operator is told why, and the file is
/// refused, no lease and no edit, until a later read can make it. The
/// faults: the copy's directory entry cannot be flushed, the publication
/// fails, or the file also holds a row this build cannot write back as
/// read (a rewrite would lose that row's unknown field).
#[test]
fn a_rename_apart_that_cannot_be_made_whole_leaves_the_file_untouched_and_refused() {
    for fault in ["copy-entry", "publication", "unknown-field"] {
        let scratch = IsolatedEnv::enter(&format!("local-state-r5-dedupe-fault-{fault}"));
        let path = StateRoot::new(scratch.home.join(".274bot")).loadouts_file();
        let service = service(&scratch, &[LocalStateDomain::LoadoutsReferences]);
        assert_eq!(
            service.readback_lease(&loadouts_identity(), WAIT).unwrap(),
            None
        );
        let failing = Arc::new(AtomicBool::new(true));
        let fails = Arc::clone(&failing);
        service.set_sync_hook(Arc::new(move |dir| {
            if fault == "copy-entry" && fails.load(Ordering::SeqCst) {
                return Err(std::io::Error::other("the directory flush fails"));
            }
            files::sync_directory(dir)
        }));
        let fails = Arc::clone(&failing);
        service.set_write_hook(Arc::new(move |path, bytes| {
            if fault == "publication" && fails.load(Ordering::SeqCst) {
                return Err(std::io::Error::other("the publication fails (disk full)"));
            }
            vault::write_private_file(path, bytes)
        }));
        let notices = quarantine_lines(&service);
        let rows = [
            Loadout::new("same").with_carry("Lobster", 1),
            Loadout::new("same").with_carry("Shark", 2),
        ];
        let plain = serde_json::to_vec_pretty(&rows).unwrap();
        let bytes = if fault == "unknown-field" {
            let mut document = serde_json::to_value(&rows).unwrap();
            document[1]["note"] = serde_json::json!("a field this build does not know");
            serde_json::to_vec_pretty(&document).unwrap()
        } else {
            plain.clone()
        };
        write_private(&path, &bytes);
        assert_refused_and_untouched(&service, &path, &bytes, &notices, fault);

        // The fault is gone (the unknown field: the operator removed it).
        failing.store(false, Ordering::SeqCst);
        if fault == "unknown-field" {
            write_private(&path, &plain);
        }
        notices.lock().unwrap().clear();
        let lease = service
            .readback_lease(&loadouts_identity(), WAIT)
            .unwrap()
            .unwrap();
        let mut apart = rows.to_vec();
        apart[1].name = "same (2)".into();
        assert_eq!(
            lease.snapshot().as_loadouts().unwrap(),
            apart.as_slice(),
            "{fault}: renamed apart once it can be"
        );
        assert_eq!(rows_on_disk(&path), apart, "{fault}");
        assert!(
            notices.lock().unwrap().iter().any(|(level, line)| {
                *level == Level::Info
                    && line.starts_with("audit domain=loadouts-references action=loadouts-dedupe ")
            }),
            "{fault}: audited once made"
        );
    }
}

/// T5.2 r5 (F1): when the copy of the file cannot be made (its directory
/// takes no new entry), nothing is renamed: the file keeps every byte and
/// is refused, and once the directory takes entries again the next read
/// renames it apart.
#[cfg(unix)]
#[test]
fn a_rename_apart_whose_copy_cannot_be_made_leaves_the_file_untouched_and_refused() {
    if running_as_root() {
        return;
    }
    let scratch = IsolatedEnv::enter("local-state-r5-dedupe-fault-copy");
    let root = StateRoot::new(scratch.home.join(".274bot"));
    let path = root.loadouts_file();
    let service = service(&scratch, &[LocalStateDomain::LoadoutsReferences]);
    assert_eq!(
        service.readback_lease(&loadouts_identity(), WAIT).unwrap(),
        None
    );
    let notices = quarantine_lines(&service);
    let rows = [
        Loadout::new("same").with_carry("Lobster", 1),
        Loadout::new("same").with_carry("Shark", 2),
    ];
    write_loadouts(&path, &rows);
    let bytes = std::fs::read(&path).unwrap();
    // The repair can take the file's lock (its lock file exists), but the
    // directory takes no new entry, so no copy can be made.
    drop(files::lock_for_write(&path, std::time::Instant::now() + WAIT, &|| false).unwrap());
    std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o500)).unwrap();
    let restore = ModeGuard(root.path().to_path_buf());
    assert_refused_and_untouched(&service, &path, &bytes, &notices, "copy");
    assert_eq!(siblings(&path), ["loadouts.json"], "no copy was made");

    drop(restore);
    let lease = service
        .readback_lease(&loadouts_identity(), WAIT)
        .unwrap()
        .unwrap();
    let mut apart = rows.to_vec();
    apart[1].name = "same (2)".into();
    assert_eq!(lease.snapshot().as_loadouts().unwrap(), apart.as_slice());
    let found = siblings(&path);
    assert_eq!(found.len(), 2, "{found:?}");
    assert_eq!(
        std::fs::read(path.with_file_name(&found[1])).unwrap(),
        bytes
    );
}

/// A compound preference patch remains queued while its file publication is
/// held. The service owns its whole intent; polling only observes the outcome.
#[test]
fn a_queued_preferences_update_finishes_without_losing_compound_intent() {
    let scratch = IsolatedEnv::enter("local-state-t33-queued-update");
    let preferences = preferences_at(&scratch, "queued");
    preferences.reload().unwrap();
    let (release, gate) = mpsc::channel::<()>();
    let gate = std::sync::Mutex::new(gate);
    let first = AtomicBool::new(true);
    preferences
        .service()
        .set_write_hook(Arc::new(move |path, bytes| {
            if first.swap(false, Ordering::SeqCst) {
                // The busy disk: the first file write waits for the test.
                let _ = gate.lock().unwrap().recv();
            }
            vault::write_private_file(path, bytes)
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
    assert!(!preferences.path().exists(), "nothing written yet");

    release.send(()).unwrap();
    // A hang guard, not a pass condition: the writer settles both edits
    // once released.
    queued
        .wait(Duration::from_secs(600))
        .expect("the queued update settles")
        .expect("both edits are durable");
    let saved: serde_json::Value =
        serde_json::from_slice(&std::fs::read(preferences.path()).unwrap()).unwrap();
    assert_eq!(saved, serde_json::json!({"a": 1, "b": 2}));
    assert_eq!(preferences.value("b").unwrap(), 2);
}
#[test]
fn preference_patch_rebases_nested_intent_and_refreshes_walk_projection() {
    let scratch = IsolatedEnv::enter("local-state-preference-patch-rebase");
    let preferences = preferences(&scratch);
    let path = preferences.path();
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    write_private(
        &path,
        br#"{"nav":{"allow_teleports":false,"allow_wilderness":false},"theme":"dark"}"#,
    );
    preferences.reload().unwrap();

    let before = preferences.document().unwrap();
    let mut after = before.as_ref().clone();
    after["nav"]["allow_teleports"] = serde_json::json!(true);
    after["theme"] = serde_json::json!({"density": "compact"});
    let patch = PreferencePatch::between(&before, &after).unwrap();

    // The worker observes this outside edit before it applies member intent.
    write_private(
        &path,
        br#"{"nav":{"allow_teleports":false,"allow_wilderness":true},"theme":"light","outside":7}"#,
    );
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
fn malformed_walk_projection_reports_safe_typed_cause() {
    let scratch = IsolatedEnv::enter("local-state-walk-projection-error");
    let preferences = preferences(&scratch);
    write_private(
        &preferences.path(),
        br#"{"nav":{"allow_teleports":"private-sensitive-value"}}"#,
    );
    preferences.reload().unwrap();

    let error = preferences
        .walk_globals()
        .expect_err("a non-boolean walk setting is not a typed view");
    assert!(error.path().ends_with("panel-ui.json"));
    assert!(error.cause().contains("nav.allow_teleports"));
    assert!(!error.cause().contains("private-sensitive-value"));
    assert_eq!(error.code(), None);
}

#[test]
fn dropping_preference_watch_keeps_service_owned_request_and_result() {
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
    assert!(preferences.document().unwrap().is_empty());
    assert!(
        !preferences.path().exists(),
        "readiness does not save defaults"
    );
    assert_cached_reads_allocate_nothing();

    write_private(&preferences.path(), b"{not-json");
    assert!(preferences.reload().is_err());
    assert!(preferences.document().is_err());
    assert!(preferences.walk_globals().is_err());
    assert_cached_reads_allocate_nothing();
}

#[test]
fn preference_results_outlive_neutral_history_and_bound_new_admission() {
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
    let saved: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.preferences_file()).unwrap()).unwrap();
    assert_eq!(
        saved,
        serde_json::json!({"nav":{"allow_teleports":true,"allow_wilderness":true}})
    );
}

#[test]
fn fix_r1_semantic_noop_rewrite_has_one_saved_state_info_line() {
    let scratch = IsolatedEnv::enter("local-state-preference-noop-audit");
    let preferences = preferences(&scratch);
    let bytes = b"{ \"nav\" : { \"allow_teleports\" : true } }\n";
    write_private(&preferences.path(), bytes);
    preferences.reload().unwrap();
    let commits = audits(preferences.service());
    let publications = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&publications);
    preferences
        .service()
        .set_write_hook(Arc::new(move |path, bytes| {
            counted.fetch_add(1, Ordering::SeqCst);
            vault::write_private_file(path, bytes)
        }));
    let Update::Queued(mut watch) = preferences
        .begin_patch(
            PreferencePatch::member(&["nav", "allow_teleports"], Some(true.into())).unwrap(),
        )
        .unwrap()
    else {
        panic!("the semantic no-op is accepted");
    };
    assert_eq!(watch.wait(WAIT), Some(Ok(())));
    assert_eq!(
        publications.load(Ordering::SeqCst),
        1,
        "the file is rewritten"
    );
    let commits = commits.lock().unwrap();
    assert_eq!(commits.len(), 1, "every actual rewrite logs one info line");
    assert_eq!(commits[0].identity().account().profile(), "operator");
    assert_eq!(commits[0].action(), CommitAction::PreferencesWrite);
    let saved: serde_json::Value =
        serde_json::from_slice(&std::fs::read(preferences.path()).unwrap()).unwrap();
    assert_eq!(saved, serde_json::json!({"nav":{"allow_teleports":true}}));
}

#[test]
fn shutdown_cancels_a_cold_compound_preference_once_without_saving_defaults() {
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
    assert!(!preferences.path().exists());
}

#[test]
fn nested_preference_intent_replaces_a_scalar_parent_without_losing_siblings() {
    let scratch = IsolatedEnv::enter("local-state-preference-scalar-parent");
    let preferences = preferences(&scratch);
    write_private(&preferences.path(), br#"{"nav":7,"other":{"keep":true}}"#);
    preferences.reload().unwrap();
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
    let scratch = IsolatedEnv::enter("local-state-preference-snapshot");
    let preferences = preferences(&scratch);
    let path = preferences.path();
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    write_private(&path, br#"{"theme":"dark"}"#);
    preferences.reload().unwrap();

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

    // Another writer's change, seen once the document is read again.
    write_private(&path, br#"{"theme":"other","outside":1}"#);
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
