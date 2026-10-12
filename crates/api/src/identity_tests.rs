use super::*;
use std::sync::atomic::AtomicUsize;
use std::sync::{Arc, Condvar, Mutex as StdMutex};
use std::thread;

fn slot(incarnation: u64) -> SlotKey {
    SlotKey::from_incarnation(incarnation).unwrap()
}

fn run(slot: u64, run: u64, session: u64) -> RunKey {
    RunKey { slot, run, session }
}

fn incarnation(nonce: u64) -> ProcessIncarnation {
    ProcessIncarnation::from_start_nonce(NonZeroU64::new(nonce).unwrap())
}

#[test]
fn slot_keys_start_at_one_and_refuse_instead_of_wrapping() {
    let minter = SlotKeyMinter::new();
    assert_eq!(minter.mint().unwrap(), slot(1));
    assert_eq!(minter.mint().unwrap(), slot(2));

    let last = SlotKeyMinter::starting_at(NonZeroU64::MAX);
    assert_eq!(
        last.mint().unwrap(),
        slot(u64::MAX),
        "the last value is usable"
    );
    assert_eq!(
        last.mint(),
        Err(IdentityError::Exhausted(IdentityKind::Slot)),
        "the space ends instead of wrapping to 1"
    );
    assert_eq!(
        last.mint(),
        Err(IdentityError::Exhausted(IdentityKind::Slot)),
        "a refusal is sticky, never a reuse"
    );
}

#[test]
fn zero_incarnation_is_unbound_not_a_slot() {
    assert_eq!(SlotKey::from_incarnation(0), None);
    let placeholder = run(0, 0, 0);
    assert_eq!(placeholder.slot_key(), None);
    assert_eq!(placeholder.session_key(), None);
    assert_eq!(Scope::Run(placeholder).slot(), None);
    assert!(
        !Scope::Slot(slot(1)).contains(&Scope::Run(placeholder)),
        "an unbound placeholder belongs to no slot"
    );
}

#[test]
fn session_epochs_advance_without_wrapping_and_keep_the_slot() {
    let first = SessionKey::first(slot(9));
    assert_eq!(first.epoch(), 0);
    let next = first.next().unwrap();
    assert_eq!((next.slot(), next.epoch()), (slot(9), 1));
    assert_ne!(first, next);
    let last = SessionKey::new(slot(9), u64::MAX);
    assert_eq!(
        last.next(),
        Err(IdentityError::Exhausted(IdentityKind::SessionEpoch))
    );
}

#[test]
fn run_key_projections_agree_with_the_composite_stamp() {
    let key = run(7, 3, 2);
    assert_eq!(key.slot_key(), Some(slot(7)));
    assert_eq!(key.session_key(), Some(SessionKey::new(slot(7), 2)));

    let relogged = key.next_session().unwrap();
    assert_eq!(relogged.session_key(), Some(SessionKey::new(slot(7), 3)));
    assert!(key.same_run(relogged), "a relog re-stamps the session only");
    assert_ne!(
        key, relogged,
        "but the stamped key differs, so old evidence is stale"
    );

    assert!(!key.same_run(key.next_run().unwrap()));
    assert!(!key.same_run(run(8, 3, 2)));
    assert_eq!(
        run(7, u64::MAX, 2).next_run(),
        Err(IdentityError::Exhausted(IdentityKind::Run))
    );
    assert_eq!(
        run(7, 3, u64::MAX).next_session(),
        Err(IdentityError::Exhausted(IdentityKind::SessionEpoch))
    );
}

#[test]
fn execution_generations_advance_without_wrapping_and_keep_the_run() {
    let key = run(1, 1, 1);
    let first = ExecutionId::first(key);
    assert_eq!(first.generation().get(), 1);
    let restarted = first.next_generation().unwrap();
    assert_eq!(restarted.run(), key);
    assert_eq!(restarted.generation().get(), 2);
    assert_ne!(first, restarted);
    let last = ExecutionId::new(key, NonZeroU64::MAX);
    assert_eq!(
        last.next_generation(),
        Err(IdentityError::Exhausted(IdentityKind::ExecutionGeneration))
    );
}

#[test]
fn request_id_namespaces_are_disjoint_and_closed() {
    assert_eq!(
        RequestId::isolate(0),
        Err(IdentityError::OutOfNamespace {
            raw: 0,
            namespace: RequestNamespace::Isolate
        })
    );
    let first = RequestId::isolate(1).unwrap();
    let last = RequestId::isolate(RequestId::ISOLATE_LAST).unwrap();
    assert_eq!(first.namespace(), RequestNamespace::Isolate);
    assert_eq!(last.namespace(), RequestNamespace::Isolate);
    assert_eq!(
        RequestId::isolate(RequestId::HOST_FIRST),
        Err(IdentityError::OutOfNamespace {
            raw: RequestId::HOST_FIRST,
            namespace: RequestNamespace::Isolate
        })
    );

    let minter = RequestIdMinter::new();
    let host = minter.mint().unwrap();
    assert_eq!(host.get(), RequestId::HOST_FIRST);
    assert_eq!(host.namespace(), RequestNamespace::Host);
    assert_ne!(host, last, "the first host id is not the last isolate id");
    assert!(minter.mint().unwrap() > host);

    assert!(matches!(
        RequestIdMinter::starting_at(RequestId::ISOLATE_LAST),
        Err(IdentityError::OutOfNamespace { .. })
    ));
    let ending = RequestIdMinter::starting_at(u64::MAX).unwrap();
    assert_eq!(ending.mint().unwrap().get(), u64::MAX);
    assert_eq!(
        ending.mint(),
        Err(IdentityError::Exhausted(IdentityKind::HostRequest)),
        "exhaustion refuses; it never wraps into the isolate range"
    );
}

#[test]
fn raw_zero_decodes_as_no_request() {
    assert_eq!(RequestId::from_raw(0), None);
    assert_eq!(RequestId::from_raw(5).map(RequestId::get), Some(5));
}

#[test]
fn scope_nesting_contains_only_agreeing_identities() {
    let a = slot(1);
    let b = slot(2);
    let session_a1 = SessionKey::new(a, 1);
    let session_a2 = SessionKey::new(a, 2);
    let run_a1 = run(1, 4, 1);
    let run_a2 = run(1, 4, 2);
    let run_b1 = run(2, 4, 1);
    let exec_a1 = ExecutionId::first(run_a1);
    let exec_a1_next = exec_a1.next_generation().unwrap();

    let everything = [
        Scope::Process,
        Scope::Slot(a),
        Scope::Session(session_a1),
        Scope::Run(run_a1),
        Scope::Execution(exec_a1),
    ];
    for inner in &everything {
        assert!(Scope::Process.contains(inner));
        assert!(inner.contains(inner), "{inner:?} contains itself");
    }
    for inner in &everything[1..] {
        assert!(
            !inner.contains(&Scope::Process),
            "{inner:?} is narrower than the process"
        );
    }

    assert!(Scope::Slot(a).contains(&Scope::Session(session_a2)));
    assert!(Scope::Slot(a).contains(&Scope::Run(run_a2)));
    assert!(!Scope::Slot(a).contains(&Scope::Slot(b)));
    assert!(!Scope::Slot(b).contains(&Scope::Run(run_a1)));

    assert!(Scope::Session(session_a1).contains(&Scope::Run(run_a1)));
    assert!(Scope::Session(session_a1).contains(&Scope::Execution(exec_a1)));
    assert!(
        !Scope::Session(session_a1).contains(&Scope::Run(run_a2)),
        "the same run in a later session is outside the old session"
    );
    assert!(!Scope::Session(session_a1).contains(&Scope::Slot(a)));

    assert!(Scope::Run(run_a1).contains(&Scope::Execution(exec_a1_next)));
    assert!(!Scope::Run(run_a1).contains(&Scope::Execution(ExecutionId::first(run_b1))));
    assert!(!Scope::Run(run_a1).contains(&Scope::Session(session_a1)));

    assert!(!Scope::Execution(exec_a1).contains(&Scope::Execution(exec_a1_next)));
    assert!(!Scope::Execution(exec_a1).contains(&Scope::Run(run_a1)));
}

#[test]
fn work_job_ids_are_host_minted_and_unique_across_runs_in_one_incarnation() {
    let incarnation = incarnation(7);
    let minter = RequestIdMinter::new();
    // Two runs each submit their isolate request 1 as the origin of a job.
    let origin = RequestId::isolate(1).unwrap();
    let first_run = Scope::Run(run(1, 1, 1));
    let second_run = Scope::Run(run(1, 2, 1));
    let first =
        WorkIdentity::new(minter.mint().unwrap(), incarnation, first_run, Some(origin)).unwrap();
    let second = WorkIdentity::new(
        minter.mint().unwrap(),
        incarnation,
        second_run,
        Some(origin),
    )
    .unwrap();
    assert_ne!(
        first.job(),
        second.job(),
        "job ids are unique within the incarnation, whatever run submitted them"
    );
    assert_eq!(first.origin(), Some(origin));
    assert_eq!(second.origin(), Some(origin));
    assert_eq!(first.job().namespace(), RequestNamespace::Host);
    assert_eq!(first.incarnation(), incarnation);
    assert_eq!(first.scope(), first_run);

    // An isolate-minted id names no job, in any scope.
    for scope in [
        Scope::Process,
        Scope::Slot(slot(1)),
        Scope::Session(SessionKey::new(slot(1), 1)),
        first_run,
        Scope::Execution(ExecutionId::first(run(1, 1, 1))),
    ] {
        assert_eq!(
            WorkIdentity::new(origin, incarnation, scope, None).map(|_| ()),
            Err(IdentityError::JobIdNotHostMinted { raw: 1 }),
            "{scope:?}"
        );
    }

    // An isolate-minted origin is unique only per run, so correlating it
    // needs a run or execution scope; a host-minted origin needs none.
    let host_origin = minter.mint().unwrap();
    for scope in [
        Scope::Process,
        Scope::Slot(slot(1)),
        Scope::Session(SessionKey::new(slot(1), 1)),
    ] {
        assert_eq!(
            WorkIdentity::new(minter.mint().unwrap(), incarnation, scope, Some(origin)).map(|_| ()),
            Err(IdentityError::IsolateRequestWithoutRun),
            "{scope:?}"
        );
        let process_job = WorkIdentity::new(
            minter.mint().unwrap(),
            incarnation,
            scope,
            Some(host_origin),
        )
        .unwrap();
        assert_eq!(process_job.scope(), scope);
        let unsolicited =
            WorkIdentity::new(minter.mint().unwrap(), incarnation, scope, None).unwrap();
        assert_eq!(unsolicited.origin(), None);
    }
    assert!(WorkIdentity::new(
        minter.mint().unwrap(),
        incarnation,
        Scope::Execution(ExecutionId::first(run(1, 1, 1))),
        Some(origin)
    )
    .is_ok());
}

/// The accepted policy (CSPRNG nonce drawn once by T1 composition,
/// probabilistic non-reuse) leaves this type the identity mapping: every
/// nonzero nonce round-trips, zero is the only value with no incarnation
/// (refused by `NonZeroU64` before the constructor), and nothing but the
/// nonce is read.
#[test]
fn a_start_nonce_names_exactly_one_incarnation_and_only_zero_names_none() {
    assert_eq!(
        NonZeroU64::new(0).map(ProcessIncarnation::from_start_nonce),
        None,
        "zero is unrepresentable by construction"
    );
    let samples = [
        1,
        2,
        3,
        0x5555_5555_5555_5555,
        0x8000_0000_0000_0000,
        0xAAAA_AAAA_AAAA_AAAA,
        u64::MAX - 1,
        u64::MAX,
    ];
    for raw in samples {
        let nonce = NonZeroU64::new(raw).unwrap();
        assert_eq!(
            ProcessIncarnation::from_start_nonce(nonce).start_nonce(),
            nonce,
            "{raw:#x} round-trips"
        );
        for other in samples {
            assert_eq!(
                incarnation(raw) == incarnation(other),
                raw == other,
                "{raw:#x} vs {other:#x}: distinct nonces never collapse, equal nonces never differ"
            );
        }
    }

    // No clock, pid or counter is mixed in: the mapping is evaluable at
    // compile time, and the same nonce later or on another thread names
    // the same incarnation.
    const AT_COMPILE_TIME: ProcessIncarnation =
        ProcessIncarnation::from_start_nonce(NonZeroU64::MAX);
    assert_eq!(AT_COMPILE_TIME, incarnation(u64::MAX));
    let here = incarnation(0x5555_5555_5555_5555);
    let elsewhere = thread::spawn(|| incarnation(0x5555_5555_5555_5555))
        .join()
        .unwrap();
    assert_eq!(here, elsewhere);
    assert_eq!(here, incarnation(0x5555_5555_5555_5555));
}

#[test]
fn fence_rejects_stale_and_revoked_completions() {
    let key = run(1, 1, 1);
    let first = ExecutionId::first(key);
    let second = first.next_generation().unwrap();
    let mut fence = Fence::new();
    assert_eq!(fence.current(), None);
    let unarmed = fence.admit(first).unwrap_err();
    assert_eq!(unarmed, FenceError::Unarmed { stamped: first });
    assert_eq!((unarmed.current(), unarmed.stamped()), (None, first));

    fence.arm(first);
    assert_eq!(fence.admit(first), Ok(first));
    assert_eq!(
        fence.admit(first),
        Ok(first),
        "admission is a check, not a consumption"
    );

    fence.arm(second);
    let stale = fence.admit(first).unwrap_err();
    assert_eq!(
        stale,
        FenceError::Stale {
            current: second,
            stamped: first
        }
    );
    assert_eq!(stale.current(), Some(second));
    assert_eq!(stale.stamped(), first);
    assert_eq!(fence.admit(second), Ok(second));

    fence.revoke();
    assert_eq!(
        fence.admit(second),
        Err(FenceError::Unarmed { stamped: second }),
        "revocation admits nothing"
    );
    assert!(fence.admit(first).is_err());
}

#[test]
fn terminal_publishes_once_and_is_consumed_once() {
    let mut terminal = Terminal::new();
    assert!(terminal.is_pending());
    assert_eq!(terminal.consume(), Err(TerminalError::NotPublished));
    assert!(
        terminal.is_pending(),
        "a failed consume leaves the terminal pending"
    );

    terminal.publish("first").unwrap();
    assert_eq!(
        terminal.publish("second"),
        Err(TerminalError::AlreadyPublished)
    );
    assert_eq!(
        terminal.borrow(),
        Some(&"first"),
        "the first publication is immutable"
    );
    assert_eq!(
        terminal.borrow(),
        Some(&"first"),
        "polling does not consume"
    );

    assert_eq!(terminal.consume(), Ok("first"));
    assert!(terminal.is_consumed());
    assert_eq!(terminal.borrow(), None);
    assert_eq!(terminal.consume(), Err(TerminalError::AlreadyConsumed));
    assert_eq!(
        terminal.publish("late"),
        Err(TerminalError::AlreadyPublished)
    );
}

#[test]
fn settlement_names_exactly_one_outcome() {
    let completed: Settlement<u8, &str> = Settlement::Completed(1);
    let failed: Settlement<u8, &str> = Settlement::Failed("no");
    let cancelled: Settlement<u8, &str> = Settlement::Cancelled(Cancellation::Superseded);
    assert!(completed.is_completed());
    assert!(!failed.is_completed());
    assert_eq!(completed.cancellation(), None);
    assert_eq!(failed.cancellation(), None);
    assert_eq!(cancelled.cancellation(), Some(Cancellation::Superseded));
}

/// A notifier that reads the publication from inside `notify`, so it proves
/// the new state was readable before the notice was issued.
struct ReadsBack<'a> {
    publication: &'a Publication<u32>,
    seen: StdMutex<Vec<(Notice, PublishSequence, u32)>>,
}

impl Notifier for ReadsBack<'_> {
    fn notify(&self, notice: Notice) {
        let (sequence, value) = self.publication.read(|sequence, value| (sequence, *value));
        self.seen.lock().unwrap().push((notice, sequence, value));
    }
}

#[test]
fn publication_is_readable_before_its_notice() {
    let scope = Scope::Slot(slot(3));
    let publication = Publication::new(scope, 0);
    assert_eq!(publication.sequence(), PublishSequence::default());
    let notifier = ReadsBack {
        publication: &publication,
        seen: StdMutex::new(Vec::new()),
    };

    let first = publication.publish(10, &notifier).unwrap();
    let second = publication.publish(20, &notifier).unwrap();
    assert!(second > first);

    let seen = notifier.seen.lock().unwrap();
    assert_eq!(seen.len(), 2, "one notice per publish");
    for (notice, observed_sequence, _observed_value) in seen.iter() {
        assert_eq!(notice.scope(), scope);
        assert_eq!(
            *observed_sequence,
            notice.sequence(),
            "the notified sequence is already readable inside the notice"
        );
    }
    assert_eq!(seen[0].2, 10);
    assert_eq!(seen[1].2, 20);
    assert_eq!(
        publication.read(|sequence, value| (sequence, *value)),
        (second, 20)
    );
}

#[test]
fn an_exhausted_publication_neither_replaces_its_value_nor_notifies() {
    let scope = Scope::Process;
    let publication = Publication::new(scope, 10);
    publication.inner.lock().sequence = PublishSequence(u64::MAX - 1);
    let notifier = ReadsBack {
        publication: &publication,
        seen: StdMutex::new(Vec::new()),
    };
    let last = publication.publish(20, &notifier).unwrap();
    assert_eq!(
        last,
        PublishSequence(u64::MAX),
        "the last sequence is usable"
    );

    for _ in 0..2 {
        assert_eq!(
            publication.publish(30, &notifier),
            Err(IdentityError::Exhausted(IdentityKind::PublishSequence)),
            "the refusal is sticky, never a wrap to 0"
        );
    }
    assert_eq!(
        publication.read(|sequence, value| (sequence, *value)),
        (last, 20),
        "a refused publish replaces nothing"
    );
    let seen = notifier.seen.lock().unwrap();
    assert_eq!(seen.len(), 1, "a refused publish issues no notice");
    assert_eq!(seen[0].0.sequence(), last);
}

/// A condvar-backed notifier: the shape T1 maps onto its park.
#[derive(Default)]
struct Wake {
    latest: StdMutex<Option<PublishSequence>>,
    condvar: Condvar,
    notices: AtomicUsize,
}

impl Notifier for Wake {
    fn notify(&self, notice: Notice) {
        *self.latest.lock().unwrap() = Some(notice.sequence());
        self.notices.fetch_add(1, Ordering::SeqCst);
        self.condvar.notify_all();
    }
}

#[test]
fn a_reader_woken_by_a_notice_observes_that_sequence_or_later() {
    let publication = Arc::new(Publication::new(Scope::Process, 0u64));
    let wake = Arc::new(Wake::default());
    let rounds = 200u64;

    let reader = {
        let publication = Arc::clone(&publication);
        let wake = Arc::clone(&wake);
        thread::spawn(move || {
            let mut highest = PublishSequence::default();
            let mut violations = 0;
            while highest.get() < rounds {
                let woken = {
                    let mut latest = wake.latest.lock().unwrap();
                    while latest.is_none_or(|sequence| sequence <= highest) {
                        latest = wake.condvar.wait(latest).unwrap();
                    }
                    latest.unwrap()
                };
                let (sequence, value) = publication.read(|sequence, value| (sequence, *value));
                if sequence < woken || value != sequence.get() {
                    violations += 1;
                }
                highest = woken;
            }
            violations
        })
    };

    for round in 1..=rounds {
        publication.publish(round, wake.as_ref()).unwrap();
    }
    assert_eq!(reader.join().unwrap(), 0);
    assert_eq!(wake.notices.load(Ordering::SeqCst), rounds as usize);
}

#[test]
fn deadlines_are_absolute_and_rechecked_against_now() {
    let now = Instant::now();
    let deadline = Deadline::after(now, Duration::from_secs(5));
    assert!(!deadline.expired(now));
    assert_eq!(
        deadline.remaining(now + Duration::from_secs(2)),
        Duration::from_secs(3)
    );
    assert!(deadline.expired(now + Duration::from_secs(5)));
    assert_eq!(
        deadline.remaining(now + Duration::from_secs(9)),
        Duration::ZERO
    );
    assert_eq!(deadline.instant(), now + Duration::from_secs(5));
    let sooner = Deadline::after(now, Duration::from_secs(1));
    assert_eq!(deadline.earlier(sooner), sooner);
    assert_eq!(sooner.earlier(deadline), sooner);
    assert_eq!(Deadline::at(now), Deadline::after(now, Duration::ZERO));
}

#[test]
fn active_time_does_not_advance_while_ineligible() {
    let start = Instant::now();
    let mut clock = ActiveClock::new();
    assert_eq!(clock.now(start), ActiveTime::ZERO);
    assert!(!clock.is_eligible());
    clock.observe(start, true);
    assert!(clock.is_eligible());
    clock.observe(start + Duration::from_secs(2), false);
    assert_eq!(
        clock.now(start + Duration::from_secs(60)).as_duration(),
        Duration::from_secs(2),
        "a hold spends no active time"
    );
    clock.observe(start + Duration::from_secs(60), false);
    clock.observe(start + Duration::from_secs(120), true);
    clock.observe(start + Duration::from_secs(121), true);
    assert_eq!(
        clock.now(start + Duration::from_secs(123)).as_duration(),
        Duration::from_secs(5),
        "repeating eligibility does not restart the open span"
    );
    clock.observe(start + Duration::from_secs(123), false);
    assert_eq!(
        clock.now(start + Duration::from_secs(180)).as_duration(),
        Duration::from_secs(5)
    );
    assert!(
        ActiveTime::ZERO.saturating_add(Duration::from_secs(1))
            < clock.now(start + Duration::from_secs(180))
    );
}

#[test]
fn process_time_counts_from_the_incarnation_origin() {
    let start = Instant::now();
    let origin = ProcessOrigin::at(start);
    assert_eq!(origin.elapsed(start), ProcessTime::ZERO);
    assert_eq!(
        origin
            .elapsed(start + Duration::from_millis(1500))
            .as_duration(),
        Duration::from_millis(1500)
    );
    assert_eq!(
        origin.elapsed(start - Duration::from_secs(1)),
        ProcessTime::ZERO
    );
    assert!(ProcessOrigin::now().elapsed(Instant::now()) < ProcessTime(Duration::from_secs(60)));
}

#[test]
fn durable_timestamps_are_whole_unix_seconds() {
    let stamp =
        DurableTimestamp::from_system_time(UNIX_EPOCH + Duration::from_millis(1_234_999)).unwrap();
    assert_eq!(stamp.unix_seconds(), 1_234);
    assert_eq!(stamp, DurableTimestamp::from_unix_seconds(1_234));
    assert_eq!(
        DurableTimestamp::from_system_time(UNIX_EPOCH - Duration::from_secs(1)),
        None
    );
    assert!(DurableTimestamp::now() >= stamp);
}
