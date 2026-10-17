use super::*;
use std::sync::mpsc;
use std::time::Duration;

fn service() -> Arc<parking_lot::Mutex<WorkService>> {
    Arc::new(parking_lot::Mutex::new(WorkService::with_clock_for_tests(
        api::identity::ProcessIncarnation::from_start_nonce(
            std::num::NonZeroU64::new(701).unwrap(),
        ),
        Arc::new(api::work::SystemClock),
    )))
}

fn scope() -> Scope {
    Scope::Slot(api::identity::SlotKey::from_incarnation(701).unwrap())
}

fn pending_route() -> (Arc<Mutex<HashMap<String, NavBot>>>, WorldTile) {
    let request = diagnostic_tests::tiny_stage_request();
    let to = request.to;
    let bot = NavBot {
        work_scope: Some(scope()),
        route_generation: request.generation,
        walk_request_id: request.request_id,
        requested_route: Some((to, 0, false, false, false, ZoneExempt::NONE)),
        pending_route: Some(Box::new(RouteDemand::Pending {
            request,
            kind: RouteDemandKind::Walk { authority: None },
        })),
        ..NavBot::default()
    };
    (
        Arc::new(Mutex::new(HashMap::from([("alice".into(), bot)]))),
        to,
    )
}

fn input() -> RouteSubmission {
    *RouteJob::prepare(
        "test",
        diagnostic_tests::tiny_stage_request(),
        RouteDemandKind::Walk { authority: None },
    )
    .unwrap()
    .submission()
}

struct HeldRoute {
    entered: Option<mpsc::Sender<()>>,
    release: Option<mpsc::Receiver<()>>,
}

impl CooperativeJob for HeldRoute {
    type Output = RouteStageResult;
    type Error = RouteJobError;
    type Phase = RouteWorkPhase;

    fn request_thread(&self) -> Option<api::work::RequestThread<'_>> {
        Some(api::work::RequestThread::new("route-owner-test"))
    }

    fn step(
        &mut self,
        _: &mut StepContext<'_, Self::Phase>,
    ) -> StepOutcome<Self::Output, Self::Error> {
        if let Some(entered) = self.entered.take() {
            entered.send(()).unwrap();
        }
        if let Some(release) = self.release.take() {
            release.recv_timeout(Duration::from_secs(60)).unwrap();
        }
        StepOutcome::Failed(RouteJobError::Search)
    }

    fn retained(&self) -> RetainedState {
        RetainedState::clean(0)
    }
}

fn submit_held(
    shared: &Arc<parking_lot::Mutex<WorkService>>,
    gated: bool,
) -> (WorkTicket<RouteWorkSpec>, Option<mpsc::Sender<()>>) {
    let (entered, started) = mpsc::channel();
    let (release, released) = mpsc::channel();
    let mut service = shared.lock();
    let identity = service.mint_identity(scope(), None).unwrap();
    let conditions = ProcessConditions::new(service.incarnation());
    let ticket = service
        .try_submit::<RouteWorkSpec, _>(
            identity,
            &input(),
            HeldRoute {
                entered: gated.then_some(entered),
                release: gated.then_some(released),
            },
            &conditions,
        )
        .unwrap_or_else(|_| panic!("test route admitted"));
    drop(service);
    if gated {
        started.recv_timeout(Duration::from_secs(60)).unwrap();
    }
    (ticket, gated.then_some(release))
}

fn install_owner(
    shared: &Arc<parking_lot::Mutex<WorkService>>,
    bot: &mut NavBot,
    ticket: WorkTicket<RouteWorkSpec>,
) {
    bot.route_ticket = Some(Box::new(RouteTicketOwner {
        identity: ticket.identity(),
        revision: ticket.revision(),
        generation: 1,
        request_id: 1,
        kind: RouteDemandKind::Walk { authority: None },
        cancellation: shared.lock().cancellation_handle(ticket).unwrap(),
        active: true,
    }));
}

#[test]
fn route_admission_full_retains_demand_and_retries_only_after_capacity_changes() {
    let shared = service();
    let ledger = OwnerResultLedger::with_limits(1024 * 1024, 1024 * 1024);
    // Job slots, not the retired aggregate byte cap, produce Full.
    let held: Vec<_> = (0..64).map(|_| submit_held(&shared, false).0).collect();
    for ticket in &held {
        WorkService::wait_terminal(&shared, ticket).unwrap();
    }
    let full = shared.lock().capacity();
    assert_eq!(full.jobs(), 64);
    let (navs, to) = pending_route();
    pump_route_work_on_service(&shared, &navs, "alice", &ledger);
    let pointer = {
        let all = navs.lock().unwrap();
        let bot = &all["alice"];
        assert!(bot.route_ticket.is_none() && !bot.walk_outcome_failed);
        let demand = bot.pending_route.as_deref().unwrap();
        assert!(matches!(demand, RouteDemand::Prepared(job) if job.admission_capacity.is_some()));
        std::ptr::from_ref(demand)
    };
    let before = shared.lock().mint_identity(scope(), None).unwrap().job();
    let allocations = allocation_counter::measure(|| {
        for _ in 0..1000 {
            pump_route_work_on_service(&shared, &navs, "alice", &ledger);
        }
    });
    let after = shared.lock().mint_identity(scope(), None).unwrap().job();
    assert_eq!(
        after.get(),
        before.get() + 1,
        "Full frames mint no request identities"
    );
    assert_eq!(
        allocations.count_total, 0,
        "1,000 Full frames allocate nothing"
    );
    assert_eq!(
        std::ptr::from_ref(
            navs.lock().unwrap()["alice"]
                .pending_route
                .as_deref()
                .unwrap()
        ),
        pointer
    );
    assert_eq!(shared.lock().capacity(), full);
    shared.lock().consume(&held[0]).unwrap();
    pump_route_work_on_service(&shared, &navs, "alice", &ledger);
    assert!(navs.lock().unwrap()["alice"].route_ticket.is_some());
    for ticket in &held[1..] {
        shared.lock().consume(ticket).unwrap();
    }
    finish(&shared, &navs, &ledger);
    assert_eq!(navs.lock().unwrap()["alice"].route_destination(), Some(to));
    assert_eq!(shared.lock().capacity(), api::work::WorkCapacity::EMPTY);
    navs.lock().unwrap().clear();
    assert_eq!(ledger.used_bytes(), 0);
    println!(
        "route Full frames=1000 allocations={} request_ids=0",
        allocations.count_total
    );
}

fn finish(
    shared: &Arc<parking_lot::Mutex<WorkService>>,
    navs: &Arc<Mutex<HashMap<String, NavBot>>>,
    ledger: &OwnerResultLedger,
) {
    for _ in 0..64 {
        pump_route_work_on_service(shared, navs, "alice", ledger);
        let ticket = navs.lock().unwrap()["alice"]
            .route_ticket
            .as_ref()
            .map(|owner| owner.ticket());
        if let Some(ticket) = ticket {
            WorkService::wait_terminal(shared, &ticket).unwrap();
        }
        if !navs.lock().unwrap()["alice"].route_work_pending() {
            return;
        }
    }
    panic!("tiny route did not finish within 64 admitted stages");
}

#[test]
fn route_owner_ledger_full_retains_terminal_until_result_capacity_returns() {
    let shared = service();
    let ledger = OwnerResultLedger::with_limits(1024 * 1024, 1024 * 1024);
    let blocker = ledger.try_reserve_bytes(ledger.capacity_bytes()).unwrap();
    let (navs, to) = pending_route();
    pump_route_work_on_service(&shared, &navs, "alice", &ledger);
    let ticket = navs.lock().unwrap()["alice"]
        .route_ticket
        .as_ref()
        .unwrap()
        .ticket();
    WorkService::wait_terminal(&shared, &ticket).unwrap();
    let charged = shared.lock().capacity();
    for _ in 0..10 {
        pump_route_work_on_service(&shared, &navs, "alice", &ledger);
    }
    assert_eq!(shared.lock().capacity(), charged);
    assert!(shared.lock().borrow_terminal(&ticket).unwrap().is_some());
    assert_eq!(
        navs.lock().unwrap()["alice"]
            .route_ticket
            .as_ref()
            .unwrap()
            .identity,
        ticket.identity()
    );
    assert!(!navs.lock().unwrap()["alice"].walk_outcome_failed);
    drop(blocker);
    finish(&shared, &navs, &ledger);
    assert_eq!(navs.lock().unwrap()["alice"].route_destination(), Some(to));
    assert_eq!(shared.lock().capacity(), api::work::WorkCapacity::EMPTY);
    navs.lock().unwrap().clear();
    assert_eq!(ledger.used_bytes(), 0);
}

fn running_owner_release(drop_bot: bool) {
    let shared = service();
    let (old, release) = submit_held(&shared, true);
    let mut bot = NavBot::default();
    install_owner(&shared, &mut bot, old);
    let old_charge = shared.lock().capacity();
    assert_eq!(old_charge.jobs(), 1);
    if drop_bot {
        drop(bot);
        bot = NavBot::default();
    } else {
        bot.supersede_route_work();
        bot.supersede_route_work();
    }
    assert_eq!(
        shared.lock().capacity(),
        old_charge,
        "running work keeps its charge until return"
    );
    let (replacement, replacement_release) = submit_held(&shared, true);
    install_owner(&shared, &mut bot, replacement);
    assert_eq!(shared.lock().capacity().jobs(), 2);
    release.unwrap().send(()).unwrap();
    let deadline = Instant::now() + Duration::from_secs(60);
    while shared.lock().capacity().jobs() != 1 {
        assert!(
            Instant::now() < deadline,
            "abandoned running route did not release after return"
        );
        std::thread::yield_now();
    }
    assert_eq!(
        shared.lock().capacity(),
        old_charge,
        "exactly one job charge remains"
    );
    assert_eq!(
        bot.route_ticket.as_ref().unwrap().identity,
        replacement.identity()
    );
    assert_eq!(
        shared.lock().state(&replacement),
        Some(api::work::JobState::Executing)
    );
    replacement_release.unwrap().send(()).unwrap();
    WorkService::wait_terminal(&shared, &replacement).unwrap();
    assert!(matches!(
        shared.lock().consume(&replacement).unwrap(),
        Settlement::Failed(api::work::JobError::Family(RouteJobError::Search))
    ));
    drop(bot);
    assert_eq!(
        shared.lock().capacity(),
        api::work::WorkCapacity::EMPTY,
        "settled owner drop cannot release twice"
    );
}

#[test]
fn replacing_a_running_route_releases_its_charge_once_after_return() {
    running_owner_release(false);
}

#[test]
fn dropping_navbot_releases_its_running_route_charge_once_after_return() {
    running_owner_release(true);
}

#[test]
fn a_completed_search_starts_its_continuation_in_the_harvesting_pump_pass() {
    let shared = service();
    let ledger = OwnerResultLedger::with_limits(1024 * 1024, 1024 * 1024);
    let (navs, to) = pending_route();
    pump_route_work_on_service(&shared, &navs, "alice", &ledger);
    let first = navs.lock().unwrap()["alice"]
        .route_ticket
        .as_ref()
        .unwrap()
        .ticket();
    WorkService::wait_terminal(&shared, &first).unwrap();
    assert!(matches!(
        shared.lock().borrow_terminal(&first).unwrap(),
        Some(Settlement::Completed(RouteStageResult::Continue(_)))
    ));
    pump_route_work_on_service(&shared, &navs, "alice", &ledger);
    let next = navs.lock().unwrap()["alice"]
        .route_ticket
        .as_ref()
        .unwrap()
        .ticket();
    assert_ne!(next.identity(), first.identity());
    assert!(navs.lock().unwrap()["alice"].pending_route.is_none());
    assert_eq!(shared.lock().capacity().jobs(), 1);
    finish(&shared, &navs, &ledger);
    assert_eq!(navs.lock().unwrap()["alice"].route_destination(), Some(to));
    navs.lock().unwrap().clear();
    assert_eq!(ledger.used_bytes(), 0);
}

#[test]
fn route_owner_layout_is_bounded() {
    // The 484f7ca0b fields, with names prefixed only to mark this size-only model.
    struct BasePendingBankFetch {
        _steps: VecDeque<BankStep>,
        _dest: WorldTile,
        _opts: FindOptions,
        _final_route: Arc<Route>,
        _avoid: Vec<AvoidRect>,
        _progress: crate::StepProgress,
    }
    println!(
        "NavBot={} route_ticket_inline={} route_ticket_boxed={} ScriptSlot={}",
        std::mem::size_of::<NavBot>(),
        std::mem::size_of::<Option<RouteTicketOwner>>(),
        std::mem::size_of::<Option<Box<RouteTicketOwner>>>(),
        std::mem::size_of::<script::SlotScript>()
    );
    println!(
        "NavBot fields: work_scope={} advisory_request_id={} route_ticket={} route_result_custody={} compact_route={} bank_fetch_base={} bank_fetch_head={} removed_owner_fields={}",
        std::mem::size_of::<Option<Scope>>(),
        std::mem::size_of::<u64>(),
        std::mem::size_of::<Option<Box<RouteTicketOwner>>>(),
        std::mem::size_of::<Option<Arc<dyn std::any::Any + Send + Sync>>>(),
        std::mem::size_of::<Option<RoutePath>>(),
        std::mem::size_of::<Option<BasePendingBankFetch>>(),
        std::mem::size_of::<Option<PendingBankFetch>>(),
        3 * std::mem::size_of::<Option<Arc<()>>>(),
    );
    assert_eq!(
        std::mem::size_of::<Option<Box<RouteTicketOwner>>>(),
        std::mem::size_of::<usize>()
    );
}

static SEARCH_EVENTS: Mutex<Vec<(RouteSearchPurpose, bool, Instant)>> = Mutex::new(Vec::new());

pub(super) fn record_search_event(name: &str, purpose: RouteSearchPurpose, started: bool) {
    if name == "nav-find-route-latency" {
        SEARCH_EVENTS
            .lock()
            .unwrap()
            .push((purpose, started, Instant::now()));
    }
}

#[test]
#[ignore = "requires BOT_ROUTE_PLAN_NAVPACK and BOT_ROUTE_LATENCY_CAPTURE_OUT"]
fn qualifies_real_bank_route_adapter_search_latency() {
    let pack = std::path::PathBuf::from(std::env::var_os("BOT_ROUTE_PLAN_NAVPACK").unwrap());
    let output =
        std::path::PathBuf::from(std::env::var_os("BOT_ROUTE_LATENCY_CAPTURE_OUT").unwrap());
    let loaded = NavWorld::load_pack(&pack).unwrap();
    loaded
        .bind_named_bank_facts(
            &api::game_data::for_revision(api::selected::ClientRevision::R289).unwrap(),
        )
        .unwrap();
    let world = Arc::new(loaded);
    let mut cases = Vec::new();
    for (case, item, radius, arrival) in [
        ("carry-knife", 946, 0, ArrivalKind::Reach),
        ("worn-blade", 1277, 1, ArrivalKind::Area),
    ] {
        let shared = service();
        let ledger = OwnerResultLedger::with_limits(256 * 1024 * 1024, 64 * 1024 * 1024);
        let mut request = diagnostic_tests::tiny_stage_request();
        request.world = Arc::clone(&world);
        request.from = WorldTile {
            x: 3202,
            z: 9909,
            level: 0,
        };
        request.to = WorldTile {
            x: 3209,
            z: 9891,
            level: 0,
        };
        request.radius = radius;
        request.arrival = arrival;
        request.opts.allow_bank_fetch = true;
        request.opts.allow_wilderness = true;
        request.admission = crate::admission::Admission::manual(
            request.opts,
            Default::default(),
            1,
            crate::WalkGlobals::default(),
        );
        request.state = Some(WorldState {
            stats: (0..25).map(|skill| (skill, 99)).collect(),
            combat_level: Some(126),
            map_members: true,
            ..WorldState::empty()
        });
        request.bank_origin = Origin::Session;
        request.bank = vec![(item, 1)];
        let bot = NavBot {
            work_scope: Some(scope()),
            route_generation: 1,
            walk_request_id: 1,
            requested_route: Some((request.to, radius, false, true, true, ZoneExempt::NONE)),
            pending_route: Some(Box::new(RouteDemand::Pending {
                request,
                kind: RouteDemandKind::Walk { authority: None },
            })),
            ..NavBot::default()
        };
        let navs = Arc::new(Mutex::new(HashMap::from([("route-latency".into(), bot)])));
        SEARCH_EVENTS.lock().unwrap().clear();
        let started = Instant::now();
        loop {
            pump_route_work_on_service(&shared, &navs, "route-latency", &ledger);
            if !navs.lock().unwrap()["route-latency"].route_work_pending() {
                break;
            }
            assert!(
                started.elapsed() < Duration::from_secs(180),
                "bank route deadline"
            );
            // The slot frame cadence is 20 ms (play_slots.rs). Do not wait
            // for a terminal here: this measures the actual adapter pump gap.
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(
            navs.lock().unwrap()["route-latency"].bank_fetch.is_some(),
            "{case}"
        );
        let events = std::mem::take(&mut *SEARCH_EVENTS.lock().unwrap());
        let searches = events.iter().filter(|(_, start, _)| *start).count();
        assert!(searches > 1);
        let waits: Vec<_> = events
            .windows(2)
            .filter_map(|pair| {
                let [(previous, false, returned), (next, true, begun)] = pair else {
                    return None;
                };
                Some(serde_json::json!({
                    "previous": format!("{previous:?}"), "next": format!("{next:?}"),
                    "wait_ns": begun.saturating_duration_since(*returned).as_nanos(),
                }))
            })
            .collect();
        let total: u64 = waits
            .iter()
            .map(|row| row["wait_ns"].as_u64().unwrap())
            .sum();
        cases.push(serde_json::json!({
            "case": case, "searches": searches, "waits": waits,
            "total_wait_ns": total, "elapsed_ns": started.elapsed().as_nanos(),
            "pump_interval_ms": 20,
        }));
        navs.lock().unwrap().clear();
        assert_eq!(shared.lock().capacity(), api::work::WorkCapacity::EMPTY);
        assert_eq!(ledger.used_bytes(), 0);
    }
    std::fs::write(
        output,
        serde_json::to_vec_pretty(&serde_json::json!({
            "navpack": pack, "cases": cases,
        }))
        .unwrap(),
    )
    .unwrap();
}
