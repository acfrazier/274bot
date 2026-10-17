use super::*;
use api::work::{BankPickPhase, RequestThread};

struct RequestSpec;

impl WorkFamilySpec for RequestSpec {
    const FAMILY: WorkFamily = WorkFamily::BankPick;
    type Input = TestInput;
    type Output = u32;
    type Error = TestError;
    type Phase = BankPickPhase;

    fn revision(_input: &Self::Input) -> InputRevision {
        InputRevision::FIRST
    }

    fn charge(input: &Self::Input) -> WorkCharge {
        input.charge
    }

    fn map_cancellation(reason: Cancellation) -> Cancellation {
        reason
    }
}

struct RequestJob {
    name: String,
    remaining: usize,
    trace: Arc<Mutex<Vec<(thread::ThreadId, String)>>>,
    gate: Option<(SyncSender<()>, Receiver<()>)>,
    panic: bool,
}

impl RequestJob {
    fn new(name: &str) -> Self {
        Self {
            name: name.to_owned(),
            remaining: 2,
            trace: Arc::new(Mutex::new(Vec::new())),
            gate: None,
            panic: false,
        }
    }
}

impl CooperativeJob for RequestJob {
    type Output = u32;
    type Error = TestError;
    type Phase = BankPickPhase;

    fn request_thread(&self) -> Option<RequestThread<'_>> {
        Some(RequestThread::new(&self.name))
    }

    fn retained(&self) -> RetainedState {
        RetainedState::clean(0)
    }

    fn step(&mut self, context: &mut StepContext<'_, Self::Phase>) -> StepOutcome<u32, TestError> {
        let current = thread::current();
        self.trace.lock().push((
            current.id(),
            current.name().expect("request thread is named").to_owned(),
        ));
        let _ = context.checkpoint();
        if let Some((entered, release)) = self.gate.take() {
            entered.send(()).expect("test observes the active request");
            release
                .recv_timeout(Duration::from_secs(4))
                .expect("test releases the active request");
        }
        assert!(!self.panic, "injected request panic");
        let _ = context.consume_unit();
        if self.remaining == 0 {
            StepOutcome::Completed(7)
        } else {
            self.remaining -= 1;
            StepOutcome::Yielded
        }
    }
}

fn submit_request(
    shared: &Arc<Mutex<WorkService>>,
    input: &TestInput,
    job: RequestJob,
    conditions: &ProcessConditions,
) -> WorkTicket<RequestSpec> {
    let mut service = shared.lock();
    let identity = service.mint_identity(Scope::Process, None).unwrap();
    service
        .try_submit::<RequestSpec, _>(identity, input, job, conditions)
        .unwrap_or_else(|refused| panic!("request refused: {:?}", refused.refusal()))
}

#[test]
fn request_threads_run_independently_keep_names_and_ignore_the_global_byte_cap() {
    let shared = new_service(Arc::new(ManualClock::new()));
    let conditions = conditions(&shared);
    let background_input = TestInput {
        charge: WorkCharge::new(1, 2, 3, 0),
    };
    let (entered_tx, entered_rx) = mpsc::sync_channel(0);
    let (release_tx, release_rx) = mpsc::sync_channel(0);
    let background = submit(
        &shared,
        &background_input,
        QueueJob::new(
            1,
            Arc::new(Mutex::new(Vec::new())),
            0,
            Some((entered_tx, release_rx)),
        ),
        &conditions,
    );
    entered_rx.recv_timeout(Duration::from_secs(2)).unwrap();

    let input = TestInput {
        charge: WorkCharge::new(192 * 1024 * 1024, 0, 0, 0),
    };
    let mut requests = Vec::new();
    for name in ["bank-pick-alice", "bank-pick-bob", "bank-pick-carol"] {
        let (entered_tx, entered_rx) = mpsc::sync_channel(0);
        let (release_tx, release_rx) = mpsc::sync_channel(0);
        let mut job = RequestJob::new(name);
        assert_eq!(job.request_thread().unwrap().stack_bytes(), None);
        let trace = Arc::clone(&job.trace);
        job.gate = Some((entered_tx, release_rx));
        let ticket = submit_request(&shared, &input, job, &conditions);
        requests.push((ticket, name, entered_rx, release_tx, trace));
    }
    for (_, _, entered, _, _) in &requests {
        entered.recv_timeout(Duration::from_secs(2)).unwrap();
    }
    {
        let service = shared.lock();
        assert!(service.capacity().bytes() > service.bounds.max_bytes().get());
        assert_eq!(service.capacity().jobs(), 4);
        assert_eq!(
            service.shared.active.lock().as_ref().unwrap().entry,
            background.identity().job()
        );
        assert!(service.shared.queue.lock().jobs.is_empty());
    }
    for (_, _, _, release, _) in &requests {
        release.send(()).unwrap();
    }
    let mut ids = std::collections::HashSet::new();
    for (ticket, name, _, _, trace) in &requests {
        WorkService::wait_terminal(&shared, ticket).unwrap();
        assert!(matches!(
            shared.lock().consume(ticket),
            Ok(Settlement::Completed(7))
        ));
        let trace = trace.lock();
        assert_eq!(trace.len(), 3);
        let id = trace[0].0;
        assert!(ids.insert(id), "requests must not share their thread");
        assert!(trace
            .iter()
            .all(|(step_id, step_name)| *step_id == id && step_name == name));
    }
    wait_for_jobs(&shared, 1);
    release_tx.send(()).unwrap();
    WorkService::wait_terminal(&shared, &background).unwrap();
    shared.lock().consume(&background).unwrap();
    wait_for_jobs(&shared, 0);
    assert_eq!(WorkService::shutdown(&shared), Ok(()));
}

#[test]
fn request_spawn_failure_returns_the_original_job_and_rolls_back_admission() {
    let shared = new_service(Arc::new(ManualClock::new()));
    let conditions = conditions(&shared);
    let input = TestInput {
        charge: WorkCharge::new(123, 456, 789, 1),
    };
    let job = RequestJob::new("bank-pick-alice");
    let trace = Arc::clone(&job.trace);
    let mut service = shared.lock();
    let identity = service.mint_identity(Scope::Process, None).unwrap();
    let refused = service
        .try_submit_with_spawn::<RequestSpec, _>(
            identity,
            &input,
            job,
            &conditions,
            None,
            |_, _, _| Err(std::io::Error::other("injected thread start failure")),
        )
        .expect_err("the injected spawn must refuse");
    let (refusal, job) = refused.into_parts();
    assert_eq!(refusal, SubmissionRefusal::ThreadStartFailed);
    assert!(Arc::ptr_eq(&job.trace, &trace));
    assert!(trace.lock().is_empty());
    assert_eq!(service.capacity().jobs(), 0);
    assert_eq!(service.capacity().bytes(), 0);
    assert_eq!(service.capacity().handles(), 0);
    assert!(service.jobs.is_empty());
    assert!(service.request_threads.is_empty());
    let ticket = service
        .try_submit::<RequestSpec, _>(identity, &input, job, &conditions)
        .unwrap_or_else(|refused| panic!("retry refused: {:?}", refused.refusal()));
    drop(service);
    WorkService::wait_terminal(&shared, &ticket).unwrap();
    shared.lock().consume(&ticket).unwrap();
    wait_for_jobs(&shared, 0);
    assert_eq!(WorkService::shutdown(&shared), Ok(()));
}

#[test]
fn request_cancellation_keeps_the_first_reason_and_a_request_panic_stays_local() {
    let shared = new_service(Arc::new(ManualClock::new()));
    let conditions = conditions(&shared);
    let input = TestInput {
        charge: WorkCharge::new(0, 0, 0, 0),
    };
    let (entered_tx, entered_rx) = mpsc::sync_channel(0);
    let (release_tx, release_rx) = mpsc::sync_channel(0);
    let mut job = RequestJob::new("bank-pick-alice");
    let trace = Arc::clone(&job.trace);
    job.gate = Some((entered_tx, release_rx));
    let cancelled = submit_request(&shared, &input, job, &conditions);
    entered_rx.recv_timeout(Duration::from_secs(2)).unwrap();
    assert_eq!(
        shared.lock().cancel(cancelled, Cancellation::Operator),
        CancelOutcome::Requested
    );
    assert_eq!(
        shared
            .lock()
            .cancel(cancelled, Cancellation::ScopeInvalidated),
        CancelOutcome::Requested
    );
    release_tx.send(()).unwrap();
    WorkService::wait_terminal(&shared, &cancelled).unwrap();
    assert!(matches!(
        shared.lock().consume(&cancelled),
        Ok(Settlement::Cancelled(Cancellation::Operator))
    ));
    assert_eq!(trace.lock().len(), 1);

    let mut job = RequestJob::new("bank-pick-bob");
    job.panic = true;
    let panicked = submit_request(&shared, &input, job, &conditions);
    WorkService::wait_terminal(&shared, &panicked).unwrap();
    assert!(matches!(
        shared.lock().consume(&panicked),
        Ok(Settlement::Failed(JobError::FailedInternally))
    ));
    let healthy = submit_request(
        &shared,
        &input,
        RequestJob::new("bank-pick-carol"),
        &conditions,
    );
    WorkService::wait_terminal(&shared, &healthy).unwrap();
    assert!(matches!(
        shared.lock().consume(&healthy),
        Ok(Settlement::Completed(7))
    ));
    assert_eq!(shared.lock().health(), None);
    wait_for_jobs(&shared, 0);
    assert_eq!(WorkService::shutdown(&shared), Ok(()));
}

#[test]
fn request_shutdown_deadline_retains_custody_until_the_thread_returns() {
    let shared = new_service(Arc::new(ManualClock::new()));
    let conditions = conditions(&shared);
    let input = TestInput {
        charge: WorkCharge::new(1, 2, 3, 1),
    };
    let (entered_tx, entered_rx) = mpsc::sync_channel(0);
    let (release_tx, release_rx) = mpsc::sync_channel(0);
    let mut job = RequestJob::new("bank-pick-alice");
    job.gate = Some((entered_tx, release_rx));
    let ticket = submit_request(&shared, &input, job, &conditions);
    entered_rx.recv_timeout(Duration::from_secs(2)).unwrap();
    assert!(matches!(
        WorkService::shutdown_with_timeout(&shared, Duration::from_millis(25)),
        Err(WorkShutdownError::ThreadsNotReturned {
            request_threads: 1,
            ..
        })
    ));
    assert_eq!(shared.lock().capacity().jobs(), 1);
    assert_eq!(shared.lock().capacity().handles(), 1);
    release_tx.send(()).unwrap();
    WorkService::wait_terminal(&shared, &ticket).unwrap();
    assert!(matches!(
        shared.lock().consume(&ticket),
        Ok(Settlement::Cancelled(Cancellation::Shutdown))
    ));
    wait_for_jobs(&shared, 0);
    assert_eq!(
        WorkService::shutdown_with_timeout(&shared, Duration::from_secs(1)),
        Ok(())
    );
    assert!(shared.lock().request_threads.is_empty());
}
