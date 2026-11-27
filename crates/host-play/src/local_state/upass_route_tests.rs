//! The Underground Pass route store (`script` `quester::handlers::upass_store`)
//! against the real SQLite knowledge store, through the slot's `StatePort`:
//! the same reads and writes the grid step makes.

use std::path::PathBuf;
use std::sync::Arc;
use std::task::Poll;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use api::identity::DurableTimestamp;
use api::local_state::{
    validate_knowledge_payload, AccountKey, ContentScope, DomainIdentity, ErrorCode, KnowledgeKey,
    KnowledgeProvenance, KnowledgeRecord, KnowledgeRevocation, KnowledgeStatus, LocalStateDomain,
    LocalStatePort, StoreVersion, KNOWLEDGE_SCHEMA_VERSION,
};
use client::io::ClientRevision;
use script::local_state::{SlotState, StatePort, StoredKnowledge};
use script::quester::handlers::upass_route_probe::{ProbeLoaded, RouteProbe};
use script::IsolatedEnv;

use super::{LocalStateService, RequestState, StateRoot};

const WAIT: Duration = Duration::from_secs(60);
const PROFILE: &str = "local-289";
const CONTENT: &str = "test-content";
/// Column 1 is lane 3.
const ONE_COLUMN: [u8; 6] = [3, 0, 0, 0, 0, 0];
/// Column 1 lane 3; column 2 lane 3 trapped and lane 2 held; column 3 lane 2
/// trapped.
const TWO_COLUMNS: [u8; 6] = [3, 2, 0, 0, 0b00100, 0b00010];

fn root_of(scratch: &IsolatedEnv) -> StateRoot {
    StateRoot::new(scratch.home.join(".274bot"))
}

fn slot(service: &Arc<LocalStateService>, account: &str) -> SlotState {
    SlotState::new(
        Arc::clone(service) as Arc<dyn StatePort>,
        AccountKey::try_new(PROFILE, account).unwrap(),
        ContentScope::try_new(ClientRevision::R289, CONTENT).unwrap(),
    )
}

fn identity(account: &str) -> DomainIdentity {
    DomainIdentity::new(
        LocalStateDomain::Knowledge,
        AccountKey::try_new(PROFILE, account).unwrap(),
        ContentScope::try_new(ClientRevision::R289, CONTENT).unwrap(),
    )
}

/// Read the route the way a starting step does. The observed tick stays
/// fixed, so the step's read bound is not spent by a slow test host.
fn load(probe: &mut RouteProbe, state: &SlotState) -> ProbeLoaded {
    let deadline = Instant::now() + WAIT;
    while Instant::now() < deadline {
        if let Poll::Ready(result) = probe.load(state, 1) {
            return result.expect("the route reads");
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    panic!("the route did not read");
}

/// Pump one observed tick per call until nothing waits to be written.
fn settle(probe: &mut RouteProbe, state: &SlotState, tick: &mut u64) {
    let deadline = Instant::now() + WAIT;
    while Instant::now() < deadline {
        *tick += 1;
        probe.pump(state, *tick);
        if probe.settled() {
            return;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    panic!("the route write did not settle");
}

/// The committed route fact as the slot's port reads it.
fn stored(service: &Arc<LocalStateService>, account: &str) -> Option<StoredKnowledge> {
    let identity = identity(account);
    let ticket = StatePort::start_knowledge_read(&**service, &identity).unwrap();
    let deadline = Instant::now() + WAIT;
    while Instant::now() < deadline {
        if let Some(result) = StatePort::poll_knowledge_read(
            &**service,
            &identity,
            ticket,
            &KnowledgeKey::upass_grid_route(),
        ) {
            return result.unwrap();
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    panic!("the knowledge read did not complete");
}

/// A route the step learned is one `Known`, `Observed` fact of six bytes. A
/// new service on the same root (a restart) and a new slot for the same
/// account (a relog) read it back; another account starts with no fact.
#[test]
fn upass_route_is_saved_in_sql_and_read_back_after_restart_and_relog() {
    crate::test_process::isolate_state_test!(
        upass_route_is_saved_in_sql_and_read_back_after_restart_and_relog
    );
    let scratch = IsolatedEnv::enter("upass-route-sql-restart");
    let root = root_of(&scratch);
    let mut tick = 0;
    {
        let service = LocalStateService::open_process(root.clone());
        let state = slot(&service, "alice");
        let mut probe = RouteProbe::default();
        assert_eq!(load(&mut probe, &state), ProbeLoaded::Absent);
        assert_eq!(probe.expected(), StoreVersion::INITIAL);
        probe.learn(TWO_COLUMNS);
        settle(&mut probe, &state, &mut tick);
        assert!(!probe.unsaved());
        let fact = stored(&service, "alice").expect("the route is saved");
        assert_eq!(probe.expected(), fact.version());
        let record = fact.record();
        assert_eq!(record.status(), KnowledgeStatus::Known);
        assert_eq!(record.provenance(), KnowledgeProvenance::Observed);
        assert_eq!(
            record.payload(),
            TWO_COLUMNS,
            "six route bytes, no trap count"
        );
        assert_eq!(record.expected(), StoreVersion::INITIAL);
        assert_eq!(record.revocation(), None);
    }
    let service = LocalStateService::open_process(root);
    for _relog in 0..2 {
        let state = slot(&service, "alice");
        let mut probe = RouteProbe::default();
        assert_eq!(load(&mut probe, &state), ProbeLoaded::Saved(2));
        assert_eq!(probe.payload(), TWO_COLUMNS);
    }
    let mut other = RouteProbe::default();
    assert_eq!(
        load(&mut other, &slot(&service, "bob")),
        ProbeLoaded::Absent
    );
}

/// A `Partial` route fact is read from SQLite as the route it holds, after a
/// restart: the step's reader does not map `Partial` to `Unknown`.
#[test]
fn upass_route_reads_a_partial_sql_fact_as_its_route() {
    crate::test_process::isolate_state_test!(upass_route_reads_a_partial_sql_fact_as_its_route);
    let scratch = IsolatedEnv::enter("upass-route-sql-partial");
    let root = root_of(&scratch);
    {
        let service = LocalStateService::open_process(root.clone());
        assert!(stored(&service, "alice").is_none());
        let record = KnowledgeRecord::try_new(
            AccountKey::try_new(PROFILE, "alice").unwrap(),
            ContentScope::try_new(ClientRevision::R289, CONTENT).unwrap(),
            StoreVersion::INITIAL,
            "upass.grid_route",
            KnowledgeProvenance::Observed,
            KnowledgeStatus::Partial,
            validate_knowledge_payload(KNOWLEDGE_SCHEMA_VERSION, &TWO_COLUMNS).unwrap(),
            DurableTimestamp::from_unix_seconds(1_760_000_000),
        )
        .unwrap();
        let api::local_state::SubmitReceipt::Accepted { request } =
            service.submit(api::local_state::DomainEdit::Knowledge(record))
        else {
            panic!("the partial fact is accepted");
        };
        assert!(matches!(
            service.wait_request(request, WAIT),
            RequestState::Settled(api::identity::Settlement::Completed(_))
        ));
    }
    let service = LocalStateService::open_process(root);
    let fact = stored(&service, "alice").expect("the partial fact");
    assert_eq!(fact.record().status(), KnowledgeStatus::Partial);
    let mut probe = RouteProbe::default();
    assert_eq!(
        load(&mut probe, &slot(&service, "alice")),
        ProbeLoaded::Saved(2)
    );
    assert_eq!(probe.payload(), TWO_COLUMNS);
    assert_eq!(probe.expected(), fact.version());
}

/// Wait until the wall clock has just entered a new second, so the scenario
/// below runs inside one second.
fn start_of_a_second() {
    loop {
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
        if now.subsec_millis() < 20 {
            return;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
}

/// SD1 §4: within one wall-clock second the step writes `Known`, another
/// writer publishes an `Unknown` tombstone over it, and a step that read the
/// earlier version writes a positive route against it. The version check
/// refuses that write: the tombstone stands, ordered by the SQL transaction
/// sequence and not by the equal stamps, and a fresh process reads it back.
#[test]
fn upass_stale_positive_write_cannot_replace_a_same_second_tombstone() {
    crate::test_process::isolate_state_test!(
        upass_stale_positive_write_cannot_replace_a_same_second_tombstone
    );
    let scratch = IsolatedEnv::enter("upass-route-sql-tombstone");
    let root = root_of(&scratch);
    let service = LocalStateService::open_process(root.clone());
    let state = slot(&service, "alice");
    // Warm the store first, so the timed part does no first open.
    assert!(stored(&service, "alice").is_none());
    let mut tick = 0;

    start_of_a_second();
    let mut first = RouteProbe::default();
    assert_eq!(load(&mut first, &state), ProbeLoaded::Absent);
    first.learn(ONE_COLUMN);
    settle(&mut first, &state, &mut tick);
    let known = stored(&service, "alice").expect("the route is saved");
    assert_eq!(known.record().status(), KnowledgeStatus::Known);

    // A second step (another slot of the same account) reads that version.
    let mut stale = RouteProbe::default();
    assert_eq!(load(&mut stale, &state), ProbeLoaded::Saved(1));
    assert_eq!(stale.expected(), known.version());

    // Another writer invalidates the route in the same second.
    let stamp = known.record().observed_at();
    let receipt = service.invalidate_knowledge(
        &identity("alice"),
        KnowledgeKey::upass_grid_route(),
        known.version(),
        stamp,
        KnowledgeProvenance::Observed,
        KnowledgeRevocation::ObservationRefused,
    );
    let api::local_state::SubmitReceipt::Accepted { request } = receipt else {
        panic!("the invalidation is accepted");
    };
    let RequestState::Settled(api::identity::Settlement::Completed(tombstone)) =
        service.wait_request(request, WAIT)
    else {
        panic!("the invalidation is durable");
    };
    assert!(tombstone.version() > known.version());

    // The stale step learns a positive route and writes it against the
    // version it read: refused, and never sent again.
    stale.learn(TWO_COLUMNS);
    settle(&mut stale, &state, &mut tick);
    assert!(!stale.unsaved(), "a conflict is not owed a write");
    assert_eq!(stale.expected(), known.version());
    let after = stored(&service, "alice").expect("the tombstone");
    assert_eq!(after.version(), tombstone.version());
    assert_eq!(after.record().status(), KnowledgeStatus::Unknown);
    assert_eq!(
        after.record().revocation(),
        Some(KnowledgeRevocation::ObservationRefused)
    );
    assert!(after.record().payload().is_empty());
    assert_eq!(after.record().observed_at(), stamp);
    assert_eq!(
        DurableTimestamp::now(),
        stamp,
        "the scenario ran inside one wall-clock second"
    );
    drop(state);
    drop(service);

    crate::test_process::run(
        "local_state::upass_route_tests::upass_tombstone_restart_child",
        &[
            ("UPASS_CHILD_ROOT", root.path().to_str().unwrap()),
            ("UPASS_CHILD_STAMP", &stamp.unix_seconds().to_string()),
            (
                "UPASS_CHILD_VERSION",
                &tombstone.version().get().to_string(),
            ),
        ],
    );
}

#[test]
#[ignore = "fresh-process readback child for the same-second upass tombstone test"]
fn upass_tombstone_restart_child() {
    let var = |name: &str| std::env::var(name).unwrap_or_else(|_| panic!("{name} is set"));
    let root = StateRoot::new(PathBuf::from(var("UPASS_CHILD_ROOT")));
    let stamp = DurableTimestamp::from_unix_seconds(var("UPASS_CHILD_STAMP").parse().unwrap());
    let version: u64 = var("UPASS_CHILD_VERSION").parse().unwrap();
    let service = LocalStateService::open_process(root);
    let mut probe = RouteProbe::default();
    assert_eq!(
        load(&mut probe, &slot(&service, "alice")),
        ProbeLoaded::Invalidated
    );
    assert_eq!(probe.payload(), [0; 6]);
    let fact = stored(&service, "alice").expect("the tombstone");
    assert_eq!(fact.version().get(), version);
    assert_eq!(fact.record().status(), KnowledgeStatus::Unknown);
    assert_eq!(fact.record().provenance(), KnowledgeProvenance::Observed);
    assert_eq!(
        fact.record().revocation(),
        Some(KnowledgeRevocation::ObservationRefused)
    );
    assert_eq!(fact.record().observed_at(), stamp);
    let snapshot = {
        let ticket = service
            .start_knowledge_readback(&identity("alice"))
            .unwrap();
        service
            .wait_knowledge_readback(&identity("alice"), ticket, WAIT)
            .unwrap()
    };
    let sql = snapshot
        .fact(&KnowledgeKey::upass_grid_route())
        .expect("the tombstone row");
    assert_eq!(sql.transaction_sequence(), version);
}

/// The slot's lease of the Knowledge domain hydrates from SQLite: a cold
/// identity answers `NotReady` once and queues the read, and a later lease
/// carries the facts at the newest fact version.
#[test]
fn knowledge_lease_through_the_slot_port_hydrates_from_sql() {
    crate::test_process::isolate_state_test!(
        knowledge_lease_through_the_slot_port_hydrates_from_sql
    );
    let scratch = IsolatedEnv::enter("upass-route-sql-lease");
    let root = root_of(&scratch);
    let mut tick = 0;
    {
        let service = LocalStateService::open_process(root.clone());
        let state = slot(&service, "alice");
        let mut probe = RouteProbe::default();
        load(&mut probe, &state);
        probe.learn(ONE_COLUMN);
        settle(&mut probe, &state, &mut tick);
    }
    let service = LocalStateService::open_process(root);
    let state = slot(&service, "alice");
    let identity = identity("alice");
    assert_eq!(
        state.lease(LocalStateDomain::Knowledge).unwrap_err(),
        ErrorCode::NotReady {
            domain: LocalStateDomain::Knowledge
        }
    );
    let deadline = Instant::now() + WAIT;
    let lease = loop {
        match LocalStatePort::lease(&*service, &identity) {
            Ok(lease) => break lease,
            Err(ErrorCode::NotReady { .. }) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(1))
            }
            Err(code) => panic!("the lease did not hydrate: {code}"),
        }
    };
    let records = lease.snapshot().as_knowledge().expect("knowledge rows");
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].payload(), ONE_COLUMN);
    let fact = stored(&service, "alice").unwrap();
    assert_eq!(lease.version(), fact.version());
    assert_eq!(
        StatePort::lease_version(&*service, &identity).unwrap(),
        fact.version()
    );
}

/// Column 1 lane 3; column 2 lane 3 trapped and lane 2 held; column 3 lane 2
/// trapped and lane 1 held.
const THREE_COLUMNS: [u8; 6] = [3, 2, 1, 0, 0b00100, 0b00010];

/// RevS7b F1: a step learns a positive route that is not saved yet against
/// version V; another writer invalidates the route at V+1; survival drops the
/// step and the runner builds it again from the run's retained state. The
/// rebuilt step sends the owed route against V, SQLite refuses it, the
/// tombstone stands, and the run stops saving for every later step.
#[test]
fn upass_rebuilt_step_never_writes_a_stale_route_over_a_newer_tombstone() {
    crate::test_process::isolate_state_test!(
        upass_rebuilt_step_never_writes_a_stale_route_over_a_newer_tombstone
    );
    let scratch = IsolatedEnv::enter("upass-route-sql-rebuilt-tombstone");
    let service = LocalStateService::open_process(root_of(&scratch));
    let state = slot(&service, "alice");
    let mut tick = 0;
    let mut probe = RouteProbe::default();
    assert_eq!(load(&mut probe, &state), ProbeLoaded::Absent);
    probe.learn(ONE_COLUMN);
    settle(&mut probe, &state, &mut tick);
    let known = stored(&service, "alice").expect("column 1 is saved");
    // A positive change the run has not sent yet, against `known`.
    probe.learn(TWO_COLUMNS);
    assert!(probe.unsaved());
    let tombstone = settle_receipt(
        &service,
        service.invalidate_knowledge(
            &identity("alice"),
            KnowledgeKey::upass_grid_route(),
            known.version(),
            known.record().observed_at(),
            KnowledgeProvenance::Observed,
            KnowledgeRevocation::ObservationRefused,
        ),
    );
    assert!(tombstone > known.version());

    let mut rebuilt = probe.rebuilt();
    assert_eq!(
        rebuilt.expected(),
        known.version(),
        "the fence is the run's"
    );
    settle(&mut rebuilt, &state, &mut tick);
    assert!(rebuilt.stopped(), "the version conflict stops saving");
    assert!(!rebuilt.unsaved());
    let after = stored(&service, "alice").expect("the tombstone");
    assert_eq!(after.version(), tombstone);
    assert_eq!(after.record().status(), KnowledgeStatus::Unknown);
    assert_eq!(
        after.record().revocation(),
        Some(KnowledgeRevocation::ObservationRefused)
    );

    // The stop is the run's: a step built again saves nothing it learns.
    let mut later = rebuilt.rebuilt();
    assert!(later.stopped());
    later.learn(THREE_COLUMNS);
    settle(&mut later, &state, &mut tick);
    assert_eq!(stored(&service, "alice").unwrap().version(), tombstone);
}

/// RevS7b F1: a step's accepted write is still queued in SQLite (the writer
/// thread is busy with another read) when survival drops the step. The step
/// built again follows that request instead of reading the old version and
/// sending a second write against it, so the predecessor's commit is no
/// conflict, and a later discovery is saved over the committed version.
#[test]
fn upass_rebuilt_step_follows_a_queued_predecessor_write() {
    crate::test_process::isolate_state_test!(upass_rebuilt_step_follows_a_queued_predecessor_write);
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    let scratch = IsolatedEnv::enter("upass-route-sql-rebuilt-queued");
    let service = LocalStateService::open_process(root_of(&scratch));
    let state = slot(&service, "alice");
    let mut tick = 0;
    let mut probe = RouteProbe::default();
    assert_eq!(load(&mut probe, &state), ProbeLoaded::Absent);

    // Hold the writer thread inside another account's knowledge read.
    let open = Arc::new(AtomicBool::new(false));
    let held = Arc::new(AtomicUsize::new(0));
    {
        let open = Arc::clone(&open);
        let held = Arc::clone(&held);
        service.set_read_stall_for_test(Arc::new(move || {
            held.fetch_add(1, Ordering::SeqCst);
            let deadline = Instant::now() + WAIT;
            while !open.load(Ordering::SeqCst) {
                assert!(Instant::now() < deadline, "the writer was never released");
                std::thread::sleep(Duration::from_millis(1));
            }
        }));
    }
    let blocker = service.start_knowledge_readback(&identity("bob")).unwrap();
    let deadline = Instant::now() + WAIT;
    while held.load(Ordering::SeqCst) == 0 {
        assert!(Instant::now() < deadline, "the blocking read did not start");
        std::thread::sleep(Duration::from_millis(1));
    }

    // The predecessor's write is accepted and queued behind that read.
    probe.learn(ONE_COLUMN);
    tick += 1;
    probe.pump(&state, tick);
    assert!(!probe.settled(), "the write is queued");
    let mut rebuilt = probe.rebuilt();
    assert_eq!(rebuilt.expected(), StoreVersion::INITIAL);
    for _ in 0..5 {
        tick += 1;
        rebuilt.pump(&state, tick);
    }
    assert!(
        !rebuilt.settled(),
        "the rebuilt step follows the queued write"
    );

    open.store(true, Ordering::SeqCst);
    drop(
        service
            .wait_knowledge_readback(&identity("bob"), blocker, WAIT)
            .unwrap(),
    );
    settle(&mut rebuilt, &state, &mut tick);
    assert!(
        !rebuilt.stopped(),
        "the predecessor's commit is no conflict"
    );
    let first = stored(&service, "alice").expect("the predecessor's route");
    assert_eq!(first.record().payload(), ONE_COLUMN);
    assert_eq!(rebuilt.expected(), first.version());

    rebuilt.learn(TWO_COLUMNS);
    settle(&mut rebuilt, &state, &mut tick);
    assert!(!rebuilt.unsaved());
    let second = stored(&service, "alice").expect("the later discovery");
    assert_eq!(second.record().payload(), TWO_COLUMNS);
    assert_eq!(second.record().expected(), first.version());
    assert!(second.version() > first.version());
}

/// Wait for an accepted submission's durable version.
fn settle_receipt(
    service: &LocalStateService,
    receipt: api::local_state::SubmitReceipt,
) -> StoreVersion {
    let api::local_state::SubmitReceipt::Accepted { request } = receipt else {
        panic!("the submission is accepted");
    };
    let RequestState::Settled(api::identity::Settlement::Completed(durable)) =
        service.wait_request(request, WAIT)
    else {
        panic!("the submission is durable");
    };
    durable.version()
}
