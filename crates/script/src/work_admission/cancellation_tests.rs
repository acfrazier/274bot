use super::*;

#[test]
fn handle_is_single_owner_and_binds_identity_spec_and_revision() {
    let shared = new_service(Arc::new(ManualClock::new()));
    let conditions = conditions(&shared);
    let input = TestInput {
        charge: WorkCharge::new(17, 31, 47, 1),
    };
    let trace = Arc::new(Mutex::new(Vec::new()));
    let (entered_tx, entered_rx) = mpsc::sync_channel(0);
    let (release_tx, release_rx) = mpsc::sync_channel(0);
    let ticket = submit(
        &shared,
        &input,
        QueueJob::new(5, trace, 0, Some((entered_tx, release_rx))),
        &conditions,
    );
    entered_rx.recv_timeout(Duration::from_secs(2)).unwrap();
    let mut handle = {
        let mut service = shared.lock();
        let foreign = service.mint_identity(Scope::Process, None).unwrap();
        let wrong_identity = WorkTicket::<TestSpec>::new(foreign, ticket.revision());
        let wrong_spec = WorkTicket::<ProfileSpec>::new(ticket.identity(), ticket.revision());
        let wrong_revision = WorkTicket::<TestSpec>::new(ticket.identity(), InputRevision::new(2));
        for result in [
            service.cancellation_handle(wrong_identity).map(|_| ()),
            service.cancellation_handle(wrong_spec).map(|_| ()),
            service.cancellation_handle(wrong_revision).map(|_| ()),
        ] {
            assert_eq!(result, Err(LedgerError::StaleIdentity));
        }
        let handle = service.cancellation_handle(ticket).unwrap();
        assert!(matches!(
            service.cancellation_handle(ticket),
            Err(LedgerError::StaleIdentity)
        ));
        handle
    };
    assert_eq!(
        handle.cancel(Cancellation::Operator),
        CancelOutcome::Requested
    );
    release_tx.send(()).unwrap();
    WorkService::wait_terminal(&shared, &ticket).unwrap();
    assert!(matches!(
        shared.lock().consume(&ticket).unwrap(),
        Settlement::Cancelled(Cancellation::Operator)
    ));
    wait_for_jobs(&shared, 0);
}

#[test]
fn dropping_under_navs_with_a_concurrent_service_operation_does_not_deadlock() {
    let shared = new_service(Arc::new(ManualClock::new()));
    let conditions = conditions(&shared);
    let input = TestInput {
        charge: WorkCharge::new(17, 31, 47, 1),
    };
    let trace = Arc::new(Mutex::new(Vec::new()));
    let (entered_tx, entered_rx) = mpsc::sync_channel(0);
    let (release_tx, release_rx) = mpsc::sync_channel(0);
    let ticket = submit(
        &shared,
        &input,
        QueueJob::new(6, trace, 0, Some((entered_tx, release_rx))),
        &conditions,
    );
    entered_rx.recv_timeout(Duration::from_secs(2)).unwrap();
    let (handle, charged) = {
        let service = shared.lock();
        (
            service.cancellation_handle(ticket).unwrap(),
            service.capacity(),
        )
    };
    let navs = Arc::new(Mutex::new(()));
    let nav_guard = navs.lock();
    let (service_held_tx, service_held_rx) = mpsc::sync_channel(1);
    let (done_tx, done_rx) = mpsc::sync_channel(1);
    let concurrent_service = Arc::clone(&shared);
    let concurrent_navs = Arc::clone(&navs);
    let operation = std::thread::spawn(move || {
        let service = concurrent_service.lock();
        service_held_tx.send(()).unwrap();
        // This bounded reverse-order probe would time out if handle Drop
        // tried to acquire the service mutex while its owner held navs.
        let acquired_navs = concurrent_navs
            .try_lock_for(Duration::from_secs(2))
            .is_some();
        done_tx.send((acquired_navs, service.capacity())).unwrap();
    });
    service_held_rx
        .recv_timeout(Duration::from_secs(2))
        .unwrap();
    drop(handle);
    drop(nav_guard);
    let (acquired_navs, still_charged) = done_rx.recv_timeout(Duration::from_secs(2)).unwrap();
    operation.join().unwrap();
    assert!(
        acquired_navs,
        "handle Drop waited for the service mutex under navs"
    );
    assert_eq!(
        still_charged, charged,
        "active worker custody was released early"
    );
    assert_eq!(shared.lock().capacity(), charged);
    release_tx.send(()).unwrap();
    WorkService::wait_terminal(&shared, &ticket).unwrap();
    wait_for_jobs(&shared, 0);
    assert_eq!(shared.lock().capacity(), WorkCapacity::new(0, 0, 0));
}

#[test]
fn retired_handle_is_a_counted_noop_and_does_not_keep_or_cancel_replacement() {
    let shared = new_service(Arc::new(ManualClock::new()));
    let conditions = conditions(&shared);
    let input = TestInput {
        charge: WorkCharge::new(17, 31, 47, 1),
    };
    let trace = Arc::new(Mutex::new(Vec::new()));
    let (entered_tx, entered_rx) = mpsc::sync_channel(0);
    let (release_tx, release_rx) = mpsc::sync_channel(0);
    let old = submit(
        &shared,
        &input,
        QueueJob::new(7, Arc::clone(&trace), 0, Some((entered_tx, release_rx))),
        &conditions,
    );
    entered_rx.recv_timeout(Duration::from_secs(2)).unwrap();
    let (mut handle, weak_entry, strong_before) = {
        let service = shared.lock();
        let entry = service.jobs.get(&old.job()).unwrap();
        let strong_before = Arc::strong_count(entry);
        (
            service.cancellation_handle(old).unwrap(),
            Arc::downgrade(entry),
            strong_before,
        )
    };
    assert_eq!(weak_entry.strong_count(), strong_before);
    assert_eq!(
        handle.abandon(Cancellation::Superseded),
        CancelOutcome::Requested
    );
    let replacement = submit(
        &shared,
        &input,
        QueueJob::new(8, trace, 0, None),
        &conditions,
    );
    assert_eq!(
        handle.cancel(Cancellation::Operator),
        CancelOutcome::StaleIdentity
    );
    assert_eq!(
        handle.abandon(Cancellation::Operator),
        CancelOutcome::StaleIdentity
    );
    assert_eq!(shared.lock().cancellation_handle_noops(), 2);
    release_tx.send(()).unwrap();
    WorkService::wait_terminal(&shared, &replacement).unwrap();
    assert!(matches!(
        shared.lock().consume(&replacement).unwrap(),
        Settlement::Completed(8)
    ));
    wait_for_jobs(&shared, 0);
    shared.lock().cleanup_released();
    WorkService::shutdown(&shared).unwrap();
    assert!(
        weak_entry.upgrade().is_none(),
        "the handle must not pin a job"
    );
    assert_eq!(shared.lock().capacity(), WorkCapacity::new(0, 0, 0));
}

#[test]
fn cancellation_racing_completion_has_one_settlement_and_one_release() {
    let shared = new_service(Arc::new(ManualClock::new()));
    let conditions = conditions(&shared);
    let input = TestInput {
        charge: WorkCharge::new(17, 31, 47, 1),
    };
    for _ in 0..32 {
        let trace = Arc::new(Mutex::new(Vec::new()));
        let (entered_tx, entered_rx) = mpsc::sync_channel(0);
        let (release_tx, release_rx) = mpsc::sync_channel(0);
        let ticket = submit(
            &shared,
            &input,
            QueueJob::new(9, trace, 0, Some((entered_tx, release_rx))),
            &conditions,
        );
        entered_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        let mut handle = shared.lock().cancellation_handle(ticket).unwrap();
        let start = Arc::new(std::sync::Barrier::new(2));
        let cancel_start = Arc::clone(&start);
        let (done_tx, done_rx) = mpsc::sync_channel(1);
        let cancellation = std::thread::spawn(move || {
            cancel_start.wait();
            let outcome = handle.cancel(Cancellation::Operator);
            done_tx.send((handle, outcome)).unwrap();
        });
        start.wait();
        release_tx.send(()).unwrap();
        let (handle, outcome) = done_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        cancellation.join().unwrap();
        WorkService::wait_terminal(&shared, &ticket).unwrap();
        let settlement = shared.lock().consume(&ticket).unwrap();
        assert!(matches!(
            (outcome, settlement),
            (
                CancelOutcome::Requested,
                Settlement::Cancelled(Cancellation::Operator)
            ) | (CancelOutcome::AlreadyTerminal, Settlement::Completed(9))
        ));
        wait_for_jobs(&shared, 0);
        assert!(shared.lock().consume(&ticket).is_err());
        drop(handle);
        assert_eq!(shared.lock().capacity(), WorkCapacity::new(0, 0, 0));
    }
}

#[test]
fn terminal_abandonment_preserves_the_settlement_and_releases_custody_once() {
    let shared = new_service(Arc::new(ManualClock::new()));
    let conditions = conditions(&shared);
    let input = TestInput {
        charge: WorkCharge::new(17, 31, 47, 1),
    };
    let ticket = submit(
        &shared,
        &input,
        QueueJob::new(10, Arc::new(Mutex::new(Vec::new())), 0, None),
        &conditions,
    );
    let mut handle = shared.lock().cancellation_handle(ticket).unwrap();
    WorkService::wait_terminal(&shared, &ticket).unwrap();
    assert!(matches!(
        shared.lock().borrow_terminal(&ticket).unwrap(),
        Some(Settlement::Completed(10))
    ));
    assert_eq!(
        handle.abandon(Cancellation::ScopeInvalidated),
        CancelOutcome::AlreadyTerminal
    );
    wait_for_jobs(&shared, 0);
    assert!(matches!(
        shared.lock().borrow_terminal(&ticket).unwrap(),
        Some(Settlement::Completed(10))
    ));
    assert_eq!(
        handle.abandon(Cancellation::Operator),
        CancelOutcome::StaleIdentity
    );
    assert_eq!(shared.lock().cancellation_handle_noops(), 2);
    assert_eq!(shared.lock().capacity(), WorkCapacity::new(0, 0, 0));
}

#[test]
fn queued_abandonment_uses_the_existing_worker_cancellation_path() {
    let shared = new_service(Arc::new(ManualClock::new()));
    let conditions = conditions(&shared);
    let input = TestInput {
        charge: WorkCharge::new(17, 31, 47, 1),
    };
    let trace = Arc::new(Mutex::new(Vec::new()));
    let (entered_tx, entered_rx) = mpsc::sync_channel(0);
    let (release_tx, release_rx) = mpsc::sync_channel(0);
    let active = submit(
        &shared,
        &input,
        QueueJob::new(11, Arc::clone(&trace), 0, Some((entered_tx, release_rx))),
        &conditions,
    );
    entered_rx.recv_timeout(Duration::from_secs(2)).unwrap();
    let queued = submit(
        &shared,
        &input,
        QueueJob::new(12, Arc::clone(&trace), 0, None),
        &conditions,
    );
    let mut handle = shared.lock().cancellation_handle(queued).unwrap();
    assert_eq!(
        handle.abandon(Cancellation::Superseded),
        CancelOutcome::Requested
    );
    assert_eq!(shared.lock().capacity().jobs(), 2);
    release_tx.send(()).unwrap();
    WorkService::wait_terminal(&shared, &active).unwrap();
    assert!(matches!(
        shared.lock().consume(&active).unwrap(),
        Settlement::Completed(11)
    ));
    wait_for_jobs(&shared, 0);
    assert_eq!(*trace.lock(), [11]);
    assert_eq!(shared.lock().capacity(), WorkCapacity::new(0, 0, 0));
}

#[test]
fn dropping_a_normally_consumed_owner_is_a_counted_noop() {
    let shared = new_service(Arc::new(ManualClock::new()));
    let conditions = conditions(&shared);
    let input = TestInput {
        charge: WorkCharge::new(17, 31, 47, 1),
    };
    let ticket = submit(
        &shared,
        &input,
        QueueJob::new(13, Arc::new(Mutex::new(Vec::new())), 0, None),
        &conditions,
    );
    let handle = shared.lock().cancellation_handle(ticket).unwrap();
    WorkService::wait_terminal(&shared, &ticket).unwrap();
    assert!(matches!(
        shared.lock().consume(&ticket).unwrap(),
        Settlement::Completed(13)
    ));
    wait_for_jobs(&shared, 0);
    assert_eq!(shared.lock().cancellation_handle_noops(), 0);
    drop(handle);
    assert_eq!(shared.lock().cancellation_handle_noops(), 1);
    assert_eq!(shared.lock().capacity(), WorkCapacity::new(0, 0, 0));
}
