use super::*;
use api::bank_memory::Origin;
use api::snapshot::WorldTile;
use api::work::WorkFamilySpec;
use client::config::{Cache, LocType, NpcType, ObjType};
use nav::bank_fetch::BankRows;
use nav::collision::{pack_walk, WorldCollision};
use nav::pack::{BankAccess, BankStand};
use nav::transport::{TransportEdge, TransportGraph};
use nav::world::NavWorld;
use sha2::Digest;
use std::thread;
use std::time::{Duration, Instant};

fn tile(x: i32, z: i32, level: i32) -> WorldTile {
    WorldTile { x, z, level }
}

fn open_world(width: usize, height: usize) -> WorldCollision {
    let flags = vec![0u32; width * height * 4];
    let (walk, blocked) = pack_walk(&flags);
    WorldCollision {
        origin: tile(0, 0, 0),
        width,
        height,
        walk,
        blocked,
        flags: None,
    }
}

fn edge(
    kind: TransportKind,
    at: WorldTile,
    to: WorldTile,
    loc_id: i32,
    consumed_req: Vec<(i32, i32)>,
) -> TransportEdge {
    TransportEdge {
        takeoff: None,
        worn_all_req: Vec::new(),
        kind,
        player_delta: None,
        at,
        to,
        loc_id,
        option: 1,
        ticks: 2,
        dir: None,
        open_loc_id: None,
        skill_req: vec![],
        item_req: vec![],
        consumed_req,
        item_returns: vec![],
        quest_req: vec![],
        varp_req: vec![],
        worn_req: vec![],
        members_req: false,
        wildy_cap: None,
        quest_gates: None,
    }
}

fn world_with(graph: TransportGraph, banks: Vec<BankStand>) -> Arc<NavWorld> {
    Arc::new(NavWorld::from_parts(open_world(8, 8), graph, banks))
}

fn zoned_line_world(mut graph: TransportGraph, banks: Vec<BankStand>) -> Arc<NavWorld> {
    let collision = open_world(40, 1);
    let zones = vec![nav::zones::Zone::npc(
        tile(2, 0, 0),
        0,
        nav::zones::ZoneClass::Always,
        u16::MAX,
        0,
    )];
    let kinds = vec![nav::zones::ZoneKind::new(
        "test-barrier",
        "Test barrier",
        123,
        0,
        false,
        false,
    )];
    graph.zones = Some(
        nav::zones::ZoneTable::from_parts(
            zones,
            kinds,
            vec![],
            vec![],
            vec![],
            collision.origin,
            collision.width as u32,
            collision.height as u32,
            &graph.wilderness,
        )
        .unwrap(),
    );
    Arc::new(NavWorld::from_parts(collision, graph, banks))
}

fn named_cache() -> Arc<Cache> {
    let mut cache = Cache::default();
    while cache.npcs.len() <= 381 {
        cache.npcs.push(NpcType::default());
    }
    cache.npcs[170] = NpcType {
        id: 170,
        name: "Captain Klemfoodle".into(),
        op: vec![Some("Talk-to".into())],
        ..Default::default()
    };
    cache.npcs[378] = NpcType {
        id: 378,
        name: "Seaman Thresnor".into(),
        op: vec![Some("Pay-fare".into())],
        ..Default::default()
    };
    cache.npcs[381] = NpcType {
        id: 381,
        name: "Captain Barnaby".into(),
        op: vec![Some("Pay-fare".into())],
        ..Default::default()
    };
    while cache.locs.len() <= 2082 {
        cache.locs.push(LocType::default());
    }
    cache.locs[2082] = LocType {
        id: 2082,
        name: "Gangplank".into(),
        op: vec![Some("Cross".into())],
        ..Default::default()
    };
    while cache.objs.len() <= 1712 {
        cache.objs.push(ObjType::default());
    }
    cache.objs[1712] = ObjType {
        id: 1712,
        name: "Glory".into(),
        op: [None, None, None, Some("Rub".into()), None],
        ..Default::default()
    };
    Arc::new(cache)
}

fn names(cache: &Cache) -> Arc<ObjNames> {
    Arc::new(ObjNames::from_objs(&cache.objs))
}

fn req(from: WorldTile, to: WorldTile, request_id: u64) -> InspectRequest {
    InspectRequest {
        from,
        to,
        allow_teleports: false,
        allow_wilderness: true,
        allow_bank_fetch: false,
        zones: nav::zones::ZoneExempt::NONE,
        avoid: Vec::new(),
        request_id,
        invalid_args: false,
    }
}

fn wait_latest(navs: &Arc<Mutex<HashMap<String, NavBot>>>, id: u64) -> InspectTerminal {
    let start = std::time::Instant::now();
    loop {
        poll_inspection(navs, "p");
        {
            let all = navs.lock().unwrap();
            if let Some(bot) = all.get("p") {
                if let Some(term) = bot.inspect.latest.as_ref() {
                    if term.request_id == id {
                        return term.clone();
                    }
                }
                if let Some(term) = bot.inspect.prev.as_ref() {
                    if term.request_id == id {
                        return term.clone();
                    }
                }
                if let Some(term) = bot.inspect.held.as_ref() {
                    if term.request_id == id {
                        return term.clone();
                    }
                }
            }
        }
        assert!(
            start.elapsed() < Duration::from_secs(60),
            "inspect {id} did not publish"
        );
        thread::yield_now();
    }
}
fn wait_running(navs: &Arc<Mutex<HashMap<String, NavBot>>>, id: u64) {
    let start = std::time::Instant::now();
    loop {
        if navs
            .lock()
            .unwrap()
            .get("p")
            .is_some_and(|bot| bot.inspect.executing && bot.inspect.running_id == id)
        {
            return;
        }
        assert!(
            start.elapsed() < Duration::from_secs(60),
            "inspect {id} was not accepted by WorkService"
        );
        poll_inspection(navs, "p");
        thread::yield_now();
    }
}

fn wait_idle(navs: &Arc<Mutex<HashMap<String, NavBot>>>) {
    let start = std::time::Instant::now();
    loop {
        poll_inspection(navs, "p");
        {
            let all = navs.lock().unwrap();
            if let Some(bot) = all.get("p") {
                if !bot.inspect.executing && bot.inspect.pending.is_none() {
                    return;
                }
            }
        }
        assert!(
            start.elapsed() < Duration::from_secs(60),
            "inspect job did not idle"
        );
        thread::yield_now();
    }
}

fn fill_three_id0(navs: &Arc<Mutex<HashMap<String, NavBot>>>, world: &Arc<NavWorld>) {
    for _ in 0..3 {
        queue_inspect(
            navs,
            "p",
            &Some(Arc::clone(world)),
            Some(WorldState::empty()),
            BankRows::default(),
            None,
            None,
            req(tile(0, 0, 0), tile(2, 2, 1), 0),
        );
        wait_idle(navs);
    }
    let all = navs.lock().unwrap();
    let bot = all.get("p").unwrap();
    assert_eq!(bot.inspect.unobserved_count(), 3);
    assert!(!bot.inspect.can_admit(false));
}

#[test]
fn path_attribute_module_compiles() {
    let _ = std::any::type_name::<InspectNav>();
}

#[test]
fn inspect_ticket_ref_round_trips_and_rejects_wrong_revision() {
    let identity = WorkService::shared()
        .expect("work service")
        .lock()
        .mint_identity(Scope::Process, None)
        .expect("work identity");
    let revision = InputRevision::new(7);
    let stored = InspectTicketRef { identity, revision };
    let exact = stored.ticket();
    assert_eq!(exact.identity(), identity);
    assert_eq!(exact.revision(), revision);

    let wrong_revision = WorkTicket::<InspectWorkFamily>::new(identity, InputRevision::new(8));
    assert_ne!(stored, InspectTicketRef::new(wrong_revision));
}

#[test]
fn glider_and_boats_use_npc_table() {
    let cache = named_cache();
    let names = names(&cache);
    let glider = project_hop(
        &edge(
            TransportKind::Glider,
            tile(1, 1, 0),
            tile(2, 2, 0),
            170,
            vec![],
        ),
        Some(cache.as_ref()),
        Some(names.as_ref()),
    );
    assert_eq!(glider.kind, "glider");
    assert_eq!(glider.loc_name, "Captain Klemfoodle");
    let barnaby = project_hop(
        &edge(
            TransportKind::Boat,
            tile(1, 1, 0),
            tile(2, 2, 0),
            381,
            vec![],
        ),
        Some(cache.as_ref()),
        Some(names.as_ref()),
    );
    assert_eq!(barnaby.loc_name, "Captain Barnaby");
    let thresnor = project_hop(
        &edge(
            TransportKind::Boat,
            tile(1, 1, 0),
            tile(2, 2, 0),
            378,
            vec![],
        ),
        Some(cache.as_ref()),
        Some(names.as_ref()),
    );
    assert_eq!(thresnor.loc_name, "Seaman Thresnor");
    let plank = project_hop(
        &edge(
            TransportKind::Ladder,
            tile(1, 1, 0),
            tile(2, 2, 0),
            2082,
            vec![],
        ),
        Some(cache.as_ref()),
        Some(names.as_ref()),
    );
    assert_eq!(plank.kind, "ladder");
    assert_eq!(plank.loc_name, "Gangplank");
    let jewellery = project_hop(
        &TransportEdge {
            takeoff: None,
            option: 4,
            loc_id: 1712,
            kind: TransportKind::Teleport,
            ..edge(
                TransportKind::Teleport,
                tile(0, 0, 0),
                tile(1, 1, 0),
                1712,
                vec![],
            )
        },
        Some(cache.as_ref()),
        Some(names.as_ref()),
    );
    assert_eq!(jewellery.loc_name, "Glory");
    assert_eq!(jewellery.action, "Rub");
    let spell = project_hop(
        &edge(
            TransportKind::Teleport,
            tile(0, 0, 0),
            tile(1, 1, 0),
            0,
            vec![],
        ),
        Some(cache.as_ref()),
        Some(names.as_ref()),
    );
    assert_eq!(spell.loc_name, "");
    let unknown = project_hop(
        &edge(TransportKind::Boat, tile(0, 0, 0), tile(1, 1, 0), 9, vec![]),
        Some(cache.as_ref()),
        Some(names.as_ref()),
    );
    assert_eq!(unknown.loc_name, "");
}

#[test]
fn bank_false_fare_is_nopath() {
    let mut graph = TransportGraph::default();
    let at = tile(0, 0, 0);
    let to = tile(4, 4, 1);
    graph.at.entry(at).or_default().push(0);
    graph
        .edges
        .push(edge(TransportKind::Boat, at, to, 381, vec![(995, 30)]));
    let world = world_with(graph, vec![]);
    let capture = InspectCapture {
        request_id: 1,
        generation: 0,
        revision: api::work::InputRevision::FIRST,
        lease: nav::producer::lease(&world).expect("test world has a nav lease"),
        world,
        state: Arc::new(WorldState::empty()),
        bank_origin: Origin::Unknown,
        bank: Vec::new(),
        from: at,
        to,
        opts: FindOptions {
            allow_wilderness: true,
            ..FindOptions::default()
        },
        avoid: vec![],
        cache: None,
        obj_names: None,
        search_budget_strict: None,
        search_budget_stand: None,
        search_budget_post: None,
    };
    let term = calculate(&capture);
    assert!(!term.ok);
    assert_eq!(term.reason, "NoPath");
    assert!(!term.bank_planned);
}

#[test]
fn pre_state_stand_proof_rejects_post_only_stand() {
    let mut graph = TransportGraph::default();
    let from = tile(0, 0, 0);
    let dest = tile(0, 4, 1);
    let stand = tile(4, 4, 1);
    graph.at.entry(from).or_default().push(0);
    graph
        .edges
        .push(edge(TransportKind::Boat, from, dest, 381, vec![(995, 10)]));
    graph.at.entry(from).or_default().push(1);
    graph.edges.push(edge(
        TransportKind::Door,
        from,
        stand,
        1530,
        vec![(995, 10)],
    ));
    let world = world_with(
        graph,
        vec![BankStand {
            name: "Bank booth".into(),
            tile: stand,
            access: BankAccess::Booth { op: 2 },
        }],
    );
    let capture = InspectCapture {
        request_id: 2,
        generation: 0,
        revision: api::work::InputRevision::FIRST,
        lease: nav::producer::lease(&world).expect("test world has a nav lease"),
        world,
        state: Arc::new(WorldState::empty()),
        bank_origin: Origin::Session,
        bank: vec![(995, 10)],
        from,
        to: dest,
        opts: FindOptions {
            allow_wilderness: true,
            allow_bank_fetch: true,
            ..FindOptions::default()
        },
        avoid: vec![],
        cache: None,
        obj_names: None,
        search_budget_strict: None,
        search_budget_stand: None,
        search_budget_post: None,
    };
    let term = calculate(&capture);
    assert!(!term.ok, "PRE stand search must fail without the coins");
    assert!(!term.bank_planned);
    assert_eq!(term.reason, "NoPath");
}

#[test]
fn no_path_reason_names_the_active_zone_witness() {
    let from = tile(0, 0, 0);
    let world = zoned_line_world(TransportGraph::default(), vec![]);
    let capture = InspectCapture {
        request_id: 3,
        generation: 0,
        revision: api::work::InputRevision::FIRST,
        lease: nav::producer::lease(&world).expect("test world has a nav lease"),
        world,
        state: Arc::new(WorldState::empty()),
        bank_origin: Origin::Unknown,
        bank: Vec::new(),
        from,
        to: tile(4, 0, 0),
        opts: FindOptions {
            allow_wilderness: true,
            ..FindOptions::default()
        },
        avoid: vec![],
        cache: None,
        obj_names: None,
        search_budget_strict: None,
        search_budget_stand: None,
        search_budget_post: None,
    };
    let term = calculate(&capture);
    assert!(!term.ok);
    assert!(term.reason.starts_with("blocked by danger zones:"));
    assert!(term.reason.contains("test-barrier@2,0,0"));
}

#[test]
fn bank_access_no_path_reason_names_the_active_zone_witness() {
    let from = tile(0, 0, 0);
    let destination = tile(0, 0, 1);
    let stand = tile(4, 0, 0);
    let mut graph = TransportGraph::default();
    graph.at.entry(from).or_default().push(0);
    graph.edges.push(edge(
        TransportKind::Boat,
        from,
        destination,
        381,
        vec![(995, 1)],
    ));
    let world = zoned_line_world(
        graph,
        vec![BankStand {
            name: "Bank booth".into(),
            tile: stand,
            access: BankAccess::Booth { op: 2 },
        }],
    );
    let capture = InspectCapture {
        request_id: 4,
        generation: 0,
        revision: api::work::InputRevision::FIRST,
        lease: nav::producer::lease(&world).expect("test world has a nav lease"),
        world,
        state: Arc::new(WorldState::empty()),
        bank_origin: Origin::Session,
        bank: vec![(995, 1)],
        from,
        to: destination,
        opts: FindOptions {
            allow_wilderness: true,
            allow_bank_fetch: true,
            ..FindOptions::default()
        },
        avoid: vec![],
        cache: None,
        obj_names: None,
        search_budget_strict: None,
        search_budget_stand: None,
        search_budget_post: None,
    };
    let term = calculate(&capture);
    assert!(!term.ok);
    assert!(!term.bank_planned, "the bank-access path itself is blocked");
    assert!(term.reason.starts_with("blocked by danger zones:"));
    assert!(term.reason.contains("test-barrier@2,0,0"));

    let (_, preflights) = InspectSubmission::for_capture("test", &capture).expect("bank preflight");
    let mut job = InspectJob::new("test", capture, preflights);
    let (output, _, _) = drive_inspection_job(&mut job);
    assert!(!output.ok);
    assert!(
        !output.bank_planned,
        "the bank-access path itself is blocked"
    );
    assert!(output
        .reason
        .as_str()
        .starts_with("blocked by danger zones:"));
    assert!(output.reason.as_str().contains("test-barrier@2,0,0"));
}

#[test]
fn ok_hops_are_post_from_to_not_bank_steps() {
    let mut graph = TransportGraph::default();
    let from = tile(0, 0, 0);
    let dest = tile(4, 0, 1);
    graph.at.entry(from).or_default().push(0);
    graph
        .edges
        .push(edge(TransportKind::Boat, from, dest, 381, vec![(995, 10)]));
    let world = world_with(
        graph,
        vec![BankStand {
            name: "Bank booth".into(),
            tile: tile(1, 1, 0),
            access: BankAccess::Booth { op: 2 },
        }],
    );
    let capture = InspectCapture {
        request_id: 3,
        generation: 0,
        revision: api::work::InputRevision::FIRST,
        lease: nav::producer::lease(&world).expect("test world has a nav lease"),
        world,
        state: Arc::new(WorldState::empty()),
        bank_origin: Origin::Session,
        bank: vec![(995, 10)],
        from,
        to: dest,
        opts: FindOptions {
            allow_wilderness: true,
            allow_bank_fetch: true,
            ..FindOptions::default()
        },
        avoid: vec![],
        cache: Some(named_cache()),
        obj_names: None,
        search_budget_strict: None,
        search_budget_stand: None,
        search_budget_post: None,
    };
    let term = calculate(&capture);
    assert!(
        term.ok,
        "open walk to stand plus post boat should plan, got {}",
        term.reason
    );
    assert!(term.bank_planned);
    assert!(term.hops.iter().all(|h| h.kind != "bank"));
    assert_eq!(term.hops.iter().next().unwrap().kind, "boat");
    assert!(term.ticks > 0.0);
}

#[test]
fn cooperative_bank_fetch_postroute_uses_unique_world_state() {
    let from = tile(0, 0, 0);
    let destination = tile(4, 0, 1);
    let mut graph = TransportGraph::default();
    graph.at.entry(from).or_default().push(0);
    graph.edges.push(edge(
        TransportKind::Boat,
        from,
        destination,
        381,
        vec![(995, 10)],
    ));
    let world = world_with(
        graph,
        vec![BankStand {
            name: "Bank booth".into(),
            tile: tile(1, 1, 0),
            access: BankAccess::Booth { op: 2 },
        }],
    );
    let cache = named_cache();
    let state = Arc::new(WorldState::empty());
    let bank = BankRows {
        origin: Origin::Session,
        rows: vec![(995, 10)],
    };
    let options = FindOptions {
        allow_wilderness: true,
        allow_bank_fetch: true,
        ..FindOptions::default()
    };
    let capture = InspectCapture {
        request_id: 57,
        generation: 0,
        revision: api::work::InputRevision::FIRST,
        lease: nav::producer::lease(&world).expect("test world has a nav lease"),
        world: Arc::clone(&world),
        state: Arc::clone(&state),
        bank_origin: bank.origin,
        bank: bank.rows.clone(),
        from,
        to: destination,
        opts: options,
        avoid: Vec::new(),
        cache: Some(Arc::clone(&cache)),
        obj_names: None,
        search_budget_strict: None,
        search_budget_stand: None,
        search_budget_post: None,
    };
    let mut expected = calculate(&capture);
    expected.seq = 1;

    let navs = Arc::new(Mutex::new(HashMap::new()));
    let mut request = req(from, destination, 57);
    request.allow_bank_fetch = true;
    queue_inspect(
        &navs,
        "p",
        &Some(world),
        Some((*state).clone()),
        bank,
        Some(cache),
        None,
        request,
    );
    let actual = wait_latest(&navs, 57);
    assert_eq!(
        actual, expected,
        "cooperative bank-fetch/post-route outcome"
    );
    assert!(
        actual.hops.owner.is_some(),
        "published hops borrow the compact result"
    );
    let measured = allocation_counter::measure(|| {
        for hop in actual.hops.iter() {
            std::hint::black_box((hop.kind, hop.loc_id, hop.loc_name, hop.action, hop.option));
        }
    });
    assert_eq!(
        measured.count_total, 0,
        "published hop projection must not allocate"
    );
    assert_eq!(actual.hops.iter().next().unwrap().kind, "boat");
}

#[test]
fn cooperative_hint_overlay_adds_missing_carry_before_planning() {
    let from = tile(0, 0, 0);
    let destination = tile(4, 0, 1);
    let mut graph = TransportGraph::default();
    graph.at.entry(from).or_default().push(0);
    graph.edges.push(edge(
        TransportKind::Boat,
        from,
        destination,
        381,
        vec![(995, 10)],
    ));
    let world = world_with(
        graph,
        vec![BankStand {
            name: "Bank booth".into(),
            tile: tile(1, 1, 0),
            access: BankAccess::Booth { op: 2 },
        }],
    );
    let cache = named_cache();
    let state = Arc::new(WorldState::empty());
    let bank = BankRows {
        origin: Origin::Hint,
        rows: vec![(995, 1)],
    };
    let options = FindOptions {
        allow_wilderness: true,
        allow_bank_fetch: true,
        ..FindOptions::default()
    };
    let capture = InspectCapture {
        request_id: 58,
        generation: 0,
        revision: api::work::InputRevision::FIRST,
        lease: nav::producer::lease(&world).expect("test world has a nav lease"),
        world: Arc::clone(&world),
        state: Arc::clone(&state),
        bank_origin: bank.origin,
        bank: bank.rows.clone(),
        from,
        to: destination,
        opts: options,
        avoid: Vec::new(),
        cache: Some(Arc::clone(&cache)),
        obj_names: None,
        search_budget_strict: None,
        search_budget_stand: None,
        search_budget_post: None,
    };
    let mut expected = calculate(&capture);
    expected.seq = 1;

    let navs = Arc::new(Mutex::new(HashMap::new()));
    let mut request = req(from, destination, 58);
    request.allow_bank_fetch = true;
    queue_inspect(
        &navs,
        "p",
        &Some(world),
        Some((*state).clone()),
        bank,
        Some(cache),
        None,
        request,
    );
    let actual = wait_latest(&navs, 58);
    assert_eq!(actual, expected, "cooperative hint-overlay outcome");
    assert!(
        actual.ok,
        "the overlay must raise the one-count hint to ten"
    );
    assert!(actual.bank_planned);
    assert_eq!(actual.hops.iter().next().unwrap().kind, "boat");
}

#[test]
fn cooperative_work_service_publishes_inspection() {
    let world = world_with(TransportGraph::default(), vec![]);
    let navs = Arc::new(Mutex::new(HashMap::new()));
    queue_inspect(
        &navs,
        "p",
        &Some(world),
        Some(WorldState::empty()),
        BankRows::default(),
        None,
        None,
        req(tile(0, 0, 0), tile(2, 2, 1), 11),
    );
    wait_running(&navs, 11);
    {
        let all = navs.lock().unwrap();
        let bot = all.get("p").unwrap();
        assert_eq!(bot.inspect.running_id, 11);
        assert!(bot.inspect.executing);
    }
    let term = wait_latest(&navs, 11);
    assert!(!term.ok);
    wait_idle(&navs);
}

#[test]
fn inspection_publication_retries_when_owner_ledger_is_full() {
    let world = world_with(TransportGraph::default(), vec![]);
    let navs = Arc::new(Mutex::new(HashMap::new()));
    let request_id = 93;
    queue_inspect(
        &navs,
        "p",
        &Some(world),
        Some(WorldState::empty()),
        BankRows::default(),
        None,
        None,
        req(tile(0, 0, 0), tile(2, 2, 1), request_id),
    );

    let started = std::time::Instant::now();
    let ticket_ref = loop {
        let active = {
            let all = navs.lock().unwrap();
            all.get("p").and_then(|bot| {
                (bot.inspect.running_id == request_id)
                    .then_some(bot.inspect.active_ticket)
                    .flatten()
            })
        };
        if let Some(ticket) = active {
            break ticket;
        }
        drive_pending(&navs, "p");
        assert!(
            started.elapsed() < Duration::from_secs(60),
            "inspection did not enter WorkService"
        );
        thread::yield_now();
    };
    let ticket = ticket_ref.ticket();
    let service = WorkService::shared().expect("work service");
    WorkService::wait_terminal(&service, &ticket).expect("inspection terminal");

    let ledger = nav::producer::OwnerResultLedger::with_limits(1024, 1024);
    let occupied = ledger
        .try_reserve_bytes(1024)
        .expect("test reservation fills its private ledger");
    for retry in 0..8 {
        poll_inspection_with_ledger(&navs, "p", &ledger);
        {
            let all = navs.lock().unwrap();
            let inspect = &all.get("p").expect("inspection owner").inspect;
            assert_eq!(inspect.active_ticket, Some(ticket_ref), "retry {retry}");
            assert_eq!(inspect.running_id, request_id, "retry {retry}");
            assert!(
                inspect.pending.is_none(),
                "retry {retry} must not add demand"
            );
            assert!(
                !inspect.polling,
                "retry {retry} must release polling custody"
            );
            assert!(inspect.latest.is_none(), "retry {retry} must not publish");
        }
        assert_eq!(ledger.used_bytes(), 1024, "retry {retry} must remain Full");
        assert!(
            service
                .lock()
                .borrow_terminal(&ticket)
                .expect("completed result remains in WorkService")
                .is_some(),
            "retry {retry} must retain the same terminal"
        );
    }

    drop(occupied);
    poll_inspection_with_ledger(&navs, "p", &ledger);
    {
        let mut all = navs.lock().unwrap();
        let result_bytes = {
            let inspect = &all.get("p").expect("inspection owner").inspect;
            let latest = inspect.latest.as_ref().expect("published inspection");
            assert_eq!(latest.request_id, request_id);
            assert_eq!(latest.reason, "NoPath");
            assert!(inspect.active_ticket.is_none());
            latest
                .hops
                .owner
                .as_ref()
                .expect("owner result remains charged")
                .charged_bytes()
        };
        assert_eq!(ledger.used_bytes(), result_bytes);
        reset_inspect(all.get_mut("p").expect("inspection owner"));
    }
    assert_eq!(ledger.used_bytes(), 0);
}

#[test]
fn oversized_route_result_publishes_typed_refusal_not_nopath() {
    let world = world_with(TransportGraph::default(), vec![]);
    let navs = Arc::new(Mutex::new(HashMap::new()));
    let request_id = 94;
    queue_inspect(
        &navs,
        "p",
        &Some(world),
        Some(WorldState::empty()),
        BankRows::default(),
        None,
        None,
        req(tile(0, 0, 0), tile(2, 2, 0), request_id),
    );

    let started = std::time::Instant::now();
    let ticket_ref = loop {
        let active = {
            let all = navs.lock().unwrap();
            all.get("p").and_then(|bot| {
                (bot.inspect.running_id == request_id)
                    .then_some(bot.inspect.active_ticket)
                    .flatten()
            })
        };
        if let Some(ticket) = active {
            break ticket;
        }
        drive_pending(&navs, "p");
        assert!(
            started.elapsed() < Duration::from_secs(60),
            "oversized route inspection did not enter WorkService"
        );
        thread::yield_now();
    };
    let ticket = ticket_ref.ticket();
    let service = WorkService::shared().expect("work service");
    WorkService::wait_terminal(&service, &ticket).expect("inspection terminal");
    let base_result_bytes =
        nav::producer::OwnerResultLedger::owner_result_bytes::<InspectStored>(0)
            .expect("fixed owner-result size");
    let ledger =
        nav::producer::OwnerResultLedger::with_limits(base_result_bytes, base_result_bytes);
    {
        let work_service = service.lock();
        let Some(Settlement::Completed(output)) = work_service
            .borrow_terminal(&ticket)
            .expect("completed owner result remains retained")
        else {
            panic!("inspection produced a completed route");
        };
        assert!(
            output.route.is_some(),
            "the terminal contains the found route"
        );
        assert!(matches!(
            reserve_inspect_owner_result(output, &ledger),
            Err(nav::producer::OwnerResultRefusal::TooLarge { .. })
        ));
    }
    poll_inspection_with_ledger(&navs, "p", &ledger);

    {
        let mut all = navs.lock().unwrap();
        let inspect = &all.get("p").expect("inspection owner").inspect;
        let terminal = inspect.latest.as_ref().expect("typed oversized refusal");
        assert_eq!(terminal.reason, "route-too-large");
        assert!(!terminal.ok, "oversize is not a successful route");
        assert!(terminal.hops.owner.is_none(), "oversize output is dropped");
        assert!(inspect.active_ticket.is_none());
        reset_inspect(all.get_mut("p").expect("inspection owner"));
    }
    assert_eq!(ledger.used_bytes(), 0);
}

#[test]
fn reset_abandons_accepted_inspection_and_fences_generation() {
    let world = world_with(TransportGraph::default(), vec![]);
    let navs = Arc::new(Mutex::new(HashMap::new()));
    queue_inspect(
        &navs,
        "p",
        &Some(Arc::clone(&world)),
        Some(WorldState::empty()),
        BankRows::default(),
        None,
        None,
        req(tile(0, 0, 0), tile(2, 2, 0), 21),
    );
    wait_running(&navs, 21);
    let next_generation = {
        let mut all = navs.lock().unwrap();
        let bot = all.get_mut("p").unwrap();
        reset_inspect(bot);
        assert!(!bot.inspect.executing);
        assert!(bot.inspect.pending.is_none());
        assert!(bot.inspect.latest.is_none());
        bot.inspect.generation
    };
    queue_inspect(
        &navs,
        "p",
        &Some(world),
        Some(WorldState::empty()),
        BankRows::default(),
        None,
        None,
        req(tile(0, 0, 0), tile(3, 3, 0), 22),
    );
    let term = wait_latest(&navs, 22);
    assert_eq!(term.generation, next_generation);
    let all = navs.lock().unwrap();
    let bot = all.get("p").unwrap();
    assert!(!bot
        .inspect
        .latest
        .as_ref()
        .is_some_and(|term| term.request_id == 21));
    assert!(!bot
        .inspect
        .prev
        .as_ref()
        .is_some_and(|term| term.request_id == 21));
}

#[test]
fn reset_releases_inspection_charge_once_and_preserves_replacement() {
    const WIDTH: usize = 2001;
    const HEIGHT: usize = 2001;
    let world = Arc::new(NavWorld::from_parts(
        open_world(WIDTH, HEIGHT),
        TransportGraph::default(),
        vec![],
    ));
    let navs = Arc::new(Mutex::new(HashMap::new()));
    let incarnation = api::identity::ProcessIncarnation::from_start_nonce(
        std::num::NonZeroU64::new(23).expect("test nonce is nonzero"),
    );
    let clock: Arc<dyn api::work::MonotonicClock + Send + Sync> = Arc::new(api::work::SystemClock);
    let service = Arc::new(parking_lot::Mutex::new(WorkService::with_clock_for_tests(
        incarnation,
        clock,
    )));
    let _service_override = InspectionWorkServiceOverride::install(Arc::clone(&service));
    let baseline = service.lock().capacity();

    queue_inspect(
        &navs,
        "p",
        &Some(Arc::clone(&world)),
        Some(WorldState::empty()),
        BankRows::default(),
        None,
        None,
        req(tile(0, 0, 0), tile(WIDTH as i32, HEIGHT as i32, 0), 23),
    );
    wait_running(&navs, 23);
    let old_ticket = {
        let all = navs.lock().unwrap();
        let inspect = &all.get("p").expect("inspection owner").inspect;
        assert!(inspect.executing);
        inspect
            .active_ticket
            .expect("accepted inspection ticket")
            .ticket()
    };
    {
        let service = service.lock();
        let charged = service.capacity();
        assert!(
            charged.jobs() > baseline.jobs(),
            "the accepted inspection must add a WorkService charge"
        );
        assert!(
            service.state(&old_ticket).is_some(),
            "the accepted inspection must remain in WorkService"
        );
        assert!(
            service
                .borrow_terminal(&old_ticket)
                .expect("accepted inspection remains in WorkService")
                .is_none(),
            "reset must start before the inspection returns"
        );
    }

    let next_generation = {
        let mut all = navs.lock().unwrap();
        let bot = all.get_mut("p").expect("inspection owner");
        reset_inspect(bot);
        assert!(!bot.inspect.executing);
        assert!(bot.inspect.pending.is_none());
        assert!(bot.inspect.latest.is_none());
        bot.inspect.generation
    };
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let capacity = service.lock().capacity();
        if capacity == baseline {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "reset did not return the old inspection charge to baseline: {capacity:?} vs {baseline:?}"
        );
        thread::yield_now();
    }
    for _ in 0..8 {
        thread::yield_now();
        assert_eq!(
            service.lock().capacity(),
            baseline,
            "reset must release the old inspection charge exactly once"
        );
    }

    const REPLACEMENT_ID: u64 = 24;
    queue_inspect(
        &navs,
        "p",
        &Some(world),
        Some(WorldState::empty()),
        BankRows::default(),
        None,
        None,
        req(tile(0, 0, 0), tile(2, 2, 0), REPLACEMENT_ID),
    );
    wait_running(&navs, REPLACEMENT_ID);
    assert!(
        service.lock().capacity().jobs() > baseline.jobs(),
        "the replacement inspection must receive its own WorkService charge"
    );
    let replacement = wait_latest(&navs, REPLACEMENT_ID);
    assert!(replacement.ok, "the replacement inspection must complete");
    assert_eq!(replacement.generation, next_generation);
    let replacement_deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let capacity = service.lock().capacity();
        if capacity == baseline {
            break;
        }
        assert!(
            Instant::now() < replacement_deadline,
            "replacement charge did not return to baseline: {capacity:?} vs {baseline:?}"
        );
        thread::yield_now();
    }
    for _ in 0..8 {
        thread::yield_now();
        assert_eq!(
            service.lock().capacity(),
            baseline,
            "the old reset must not retain or release the replacement charge"
        );
    }
}

#[test]
fn pending_replace_publishes_stale_for_replaced_request() {
    let world = world_with(TransportGraph::default(), vec![]);
    let navs = Arc::new(Mutex::new(HashMap::new()));
    queue_inspect(
        &navs,
        "p",
        &Some(Arc::clone(&world)),
        Some(WorldState::empty()),
        BankRows::default(),
        None,
        None,
        req(tile(0, 0, 0), tile(1, 1, 0), 41),
    );
    wait_running(&navs, 41);
    queue_inspect(
        &navs,
        "p",
        &Some(Arc::clone(&world)),
        Some(WorldState::empty()),
        BankRows::default(),
        None,
        None,
        req(tile(0, 0, 0), tile(2, 2, 0), 42),
    );
    queue_inspect(
        &navs,
        "p",
        &Some(world),
        Some(WorldState::empty()),
        BankRows::default(),
        None,
        None,
        req(tile(0, 0, 0), tile(3, 3, 0), 43),
    );
    {
        let all = navs.lock().unwrap();
        let bot = all.get("p").unwrap();
        assert_eq!(bot.inspect.running_id, 41);
        assert_eq!(bot.inspect.pending_id, 43);
        assert_eq!(bot.inspect.replaced_id, 42);
        assert_eq!(bot.inspect.latest.as_ref().unwrap().request_id, 42);
        assert_eq!(bot.inspect.latest.as_ref().unwrap().reason, "stale");
    }
    assert_eq!(wait_latest(&navs, 43).request_id, 43);
    assert_eq!(wait_latest(&navs, 41).request_id, 41);
}

#[test]
fn replay_ring_keeps_three_independent_owner_results_until_ack_or_eviction() {
    let one_result = nav::producer::OwnerResultLedger::owner_result_bytes::<InspectStored>(0)
        .expect("fixed owner result size");
    let ledger = nav::producer::OwnerResultLedger::with_limits(one_result * 3, one_result);
    let terminal = |request_id| {
        let reservation = ledger
            .try_reserve_bytes(one_result)
            .expect("each independent terminal has a reservation");
        let stored = InspectStored {
            ok: false,
            reason: InspectMessage::Static("NoPath"),
            bank_planned: false,
            ticks: 0.0,
            route: None,
            cache: None,
            obj_names: None,
        };
        InspectTerminal::owned(request_id, 0, reservation.hold(stored))
    };

    let mut nav = InspectNav::default();
    nav.publish(terminal(1));
    nav.publish(terminal(2));
    nav.publish(terminal(3));
    assert_eq!(ledger.used_bytes(), one_result * 3);
    assert_eq!(nav.latest.as_ref().unwrap().request_id, 2);
    assert_eq!(nav.prev.as_ref().unwrap().request_id, 1);
    assert_eq!(nav.held.as_ref().unwrap().request_id, 3);

    let first_seq = nav.prev.as_ref().unwrap().seq;
    nav.apply_ack(first_seq, 0);
    assert_eq!(ledger.used_bytes(), one_result * 2);
    assert_eq!(nav.latest.as_ref().unwrap().request_id, 3);
    assert_eq!(nav.prev.as_ref().unwrap().request_id, 2);
    assert!(nav.held.is_none());

    nav.publish(terminal(4));
    assert_eq!(ledger.used_bytes(), one_result * 3);
    assert_eq!(nav.held.as_ref().unwrap().request_id, 4);
    let second_seq = nav.prev.as_ref().unwrap().seq;
    nav.apply_ack(second_seq, 0);
    assert_eq!(ledger.used_bytes(), one_result * 2);
    assert_eq!(nav.latest.as_ref().unwrap().request_id, 4);
    assert_eq!(nav.prev.as_ref().unwrap().request_id, 3);
    assert!(nav.held.is_none());

    nav.clear_published();
    assert_eq!(ledger.used_bytes(), 0);
}

#[test]
fn invalid_args_and_missing_graph() {
    let navs = Arc::new(Mutex::new(HashMap::new()));
    let mut bad = req(tile(0, 0, 9), tile(1, 1, 0), 51);
    bad.invalid_args = true;
    queue_inspect(
        &navs,
        "p",
        &None,
        None,
        BankRows::default(),
        None,
        None,
        bad,
    );
    assert_eq!(
        navs.lock()
            .unwrap()
            .get("p")
            .unwrap()
            .inspect
            .latest
            .as_ref()
            .unwrap()
            .reason,
        "invalid-args"
    );
    queue_inspect(
        &navs,
        "p",
        &None,
        None,
        BankRows::default(),
        None,
        None,
        req(tile(0, 0, 0), tile(1, 1, 0), 52),
    );
    assert_eq!(
        navs.lock()
            .unwrap()
            .get("p")
            .unwrap()
            .inspect
            .latest
            .as_ref()
            .unwrap()
            .reason,
        "missing-graph"
    );
}

#[test]
fn abort_script_walk_does_not_cancel_inspection() {
    let world = world_with(TransportGraph::default(), vec![]);
    let navs = Arc::new(Mutex::new(HashMap::new()));
    queue_inspect(
        &navs,
        "p",
        &Some(world),
        Some(WorldState::empty()),
        BankRows::default(),
        None,
        None,
        req(tile(0, 0, 0), tile(2, 2, 0), 61),
    );
    wait_running(&navs, 61);
    crate::abort_script_walk(&navs, "p");
    {
        let all = navs.lock().unwrap();
        let bot = all.get("p").unwrap();
        assert!(bot.inspect.executing);
        assert_eq!(bot.inspect.running_id, 61);
        assert!(!bot.route_work_pending());
    }
    let term = wait_latest(&navs, 61);
    assert_eq!(term.request_id, 61);
}

#[allow(clippy::too_many_arguments)] // test capture packs world/tiles/budget fields
fn capture(
    world: Arc<NavWorld>,
    from: WorldTile,
    to: WorldTile,
    opts: FindOptions,
    request_id: u64,
    bank: BankRows,
    strict: Option<usize>,
    stand: Option<usize>,
    post: Option<usize>,
) -> InspectCapture {
    InspectCapture {
        request_id,
        generation: 0,
        revision: api::work::InputRevision::FIRST,
        lease: nav::producer::lease(&world).expect("test world has a nav lease"),
        world,
        state: Arc::new(WorldState::empty()),
        bank_origin: bank.origin,
        bank: bank.rows,
        from,
        to,
        opts,
        avoid: vec![],
        cache: None,
        obj_names: None,
        search_budget_strict: strict,
        search_budget_stand: stand,
        search_budget_post: post,
    }
}
fn capture_with_named_zone(allow_bank_fetch: bool) -> InspectCapture {
    let world = zoned_line_world(TransportGraph::default(), vec![]);
    let options = FindOptions {
        allow_bank_fetch,
        zones: nav::zones::ZoneExempt::named(&[nav::zones::ZoneKey::Zone(0)])
            .expect("one named zone"),
        ..FindOptions::default()
    };
    let mut capture = capture(
        world,
        tile(0, 0, 0),
        tile(4, 0, 0),
        options,
        82,
        BankRows::default(),
        None,
        None,
        None,
    );
    capture.avoid.push(AvoidRect {
        min_x: 30,
        max_x: 30,
        min_z: 0,
        max_z: 0,
        level: Some(0),
    });
    capture
}

#[test]
fn inspection_charge_is_checked_and_bounded() {
    let world = Arc::new(NavWorld::from_parts(
        open_world(8, 8),
        TransportGraph::default(),
        vec![],
    ));
    let mut capture = capture(
        world,
        tile(0, 0, 0),
        tile(2, 2, 0),
        FindOptions::default(),
        72,
        BankRows::default(),
        None,
        None,
        None,
    );
    let mut state = WorldState::empty();
    state.inv.insert(995, 10);
    state.worn.insert(1725);
    state.stats.insert(2, 55);
    state.combat_level = Some(54);
    capture.state = Arc::new(state);
    capture.bank = Vec::with_capacity(4);
    capture.bank.extend([(995, 10), (556, 20)]);
    capture.avoid = Vec::with_capacity(3);
    capture.avoid.push(AvoidRect {
        min_x: 100,
        max_x: 100,
        min_z: 100,
        max_z: 100,
        level: Some(0),
    });
    let capture_input_bytes = capture
        .input_bytes()
        .expect("complete captured input charge");
    let expected_capture_bytes = capture
        .bank
        .capacity()
        .checked_mul(std::mem::size_of::<(i32, i32)>())
        .and_then(|bytes| {
            capture
                .avoid
                .capacity()
                .checked_mul(std::mem::size_of::<AvoidRect>())
                .and_then(|avoid| bytes.checked_add(avoid))
        })
        .and_then(|bytes| {
            capture
                .state
                .try_shared_retained_bytes()
                .and_then(|state| bytes.checked_add(state))
        })
        .expect("all captured allocations are charged");
    assert_eq!(capture_input_bytes, expected_capture_bytes);
    let (submission, preflights) =
        InspectSubmission::for_capture("test", &capture).expect("bounded route charge");
    assert!(preflights.strict.is_some());
    assert_eq!(strict_budget(&capture), 4_000_000);
    let strict = preflights.strict.as_ref().expect("strict route preflight");
    let expected_search_peak = strict
        .input_bytes()
        .checked_add(strict.reservation().future_layout_bytes)
        .and_then(|bytes| bytes.checked_add(strict.reservation().peak_bytes))
        .expect("search future and peak overlap");
    assert_eq!(submission.phase_peak_bytes, expected_search_peak);
    assert_eq!(
        submission.input_bytes,
        capture_input_bytes
            + std::mem::size_of::<InspectJob>()
            + std::mem::size_of::<InspectJobPhase>()
            + "nav-inspect-test".len(),
        "the charge includes both phase locations and the request thread name"
    );
    let route_payload = search_result_payload_bytes(strict.reservation().result_bytes)
        .expect("route payload excludes inline search wrappers");
    let expected_result =
        std::mem::size_of::<api::work::JobSettlement<InspectWorkFamily>>() + route_payload;
    assert_eq!(submission.result_bytes, expected_result);
    let charge = InspectWorkFamily::charge(&submission);
    let expected_total = submission.input_bytes + submission.phase_peak_bytes.max(expected_result);
    assert_eq!(charge.input_bytes(), submission.input_bytes);
    assert_eq!(
        charge.scratch_bytes(),
        submission
            .phase_peak_bytes
            .saturating_sub(submission.result_bytes)
    );
    assert_eq!(charge.result_bytes(), expected_result);
    assert_eq!(charge.total_bytes(), Some(expected_total));
    assert!(expected_total <= MAX_INSPECT_CHARGE);
}

#[test]
fn inspection_preflight_reserves_shapes_without_allocating_request_inputs() {
    let capture = capture_with_named_zone(true);
    let mut prepared = None;
    let measured = allocation_counter::measure(|| {
        prepared =
            Some(InspectSubmission::for_capture("test", &capture).expect("preflight shapes"));
    });
    assert_eq!(measured.count_total, 0);
    assert_eq!(measured.bytes_total, 0);

    let (submission, preflights) = prepared.expect("preflight result");
    assert!(preflights.strict.is_some());
    assert!(preflights.missing.is_some());
    assert!(preflights.stand.is_some());
    assert!(preflights.post.is_some());
    assert!(preflights.blockers_with_bank.is_some());
    assert!(preflights.blockers_without_bank.is_some());
    assert!(InspectWorkFamily::charge(&submission)
        .total_bytes()
        .is_some_and(|bytes| bytes <= MAX_INSPECT_CHARGE));
}

#[test]
fn inspection_worker_materializes_inputs_and_cancellation_releases_state() {
    let capture = capture_with_named_zone(true);
    let (_submission, preflights) =
        InspectSubmission::for_capture("test", &capture).expect("bounded worker inputs");
    let expected_missing = preflights
        .missing
        .as_ref()
        .expect("bank-fetch missing preflight")
        .reservation();
    let state = Arc::clone(&capture.state);
    let mut job = InspectJob::new("test", capture, preflights);
    job.start_missing().expect("missing search builder");
    let phase = std::mem::replace(&mut job.phase, InspectJobPhase::Start);
    let InspectJobPhase::BuildSearch { mut builder } = phase else {
        panic!("missing search should wait in its materialization phase");
    };
    assert_eq!(builder.retained_bytes(), Some(0));

    let mut built = None;
    let arrays = allocation_counter::measure(|| {
        loop {
            let ready = {
                let capture = job.capture_ref();
                builder
                    .advance(capture)
                    .expect("bounded input materialization")
            };
            if ready {
                break;
            }
        }
        built = Some(builder.into_request());
    });
    assert!(arrays.count_total > 0);
    let (request, purpose, preflight) = built.expect("materialized request");
    let cursor = allocation_counter::measure(|| {
        job.start_search(request, purpose, preflight)
            .expect("matching preflight token");
    });
    assert!(cursor.count_total > 0);

    let InspectJobPhase::Search { cursor, .. } = &job.phase else {
        panic!("search cursor should own the materialized arrays");
    };
    let expected_inputs = std::mem::size_of::<WorldTile>() + std::mem::size_of::<AvoidRect>();
    assert_eq!(cursor.input_bytes(), expected_inputs);
    assert_eq!(
        cursor.future_layout_bytes(),
        expected_missing.future_layout_bytes
    );
    assert_eq!(cursor.retained_bytes(), expected_missing.peak_bytes);
    let expected_retained = std::mem::size_of::<InspectJob>()
        + job.thread_name.capacity()
        + job.capture_ref().input_bytes().expect("capture charge")
        + cursor.input_bytes()
        + cursor.future_layout_bytes()
        + cursor.retained_bytes();
    assert_eq!(job.retained_bytes(), Some(expected_retained));
    assert_eq!(Arc::strong_count(&state), 3);

    let _cancelled = job.cancelled_output();
    assert_eq!(Arc::strong_count(&state), 1);
    assert!(job.preflights.strict.is_none());
    assert!(job.preflights.missing.is_none());
    assert!(job.preflights.stand.is_none());
    assert!(job.preflights.post.is_none());
    assert!(job.preflights.blockers_with_bank.is_none());
    assert!(job.preflights.blockers_without_bank.is_none());

    let capture = capture_with_named_zone(false);
    let (_, preflights) =
        InspectSubmission::for_capture("test", &capture).expect("blocker preflight");
    let mut job = InspectJob::new("test", capture, preflights);
    let capture = job.capture_ref();
    let (from, to, options) = (capture.from, capture.to, capture.opts);
    let filter = allocation_counter::measure(|| {
        job.start_blockers(from, to, options, false)
            .expect("blocker filter builder");
    });
    assert!(filter.count_total > 0);
    let InspectJobPhase::BuildZoneFilter { filter: owned, .. } = &job.phase else {
        panic!("zone filter must be created in the admitted worker phase");
    };
    assert!(
        owned.retained_bytes().expect("filter storage") > std::mem::size_of::<ZoneFilterOwned>()
    );
    let _cancelled = job.cancelled_output();
}

#[test]
fn inspection_preflight_mismatch_fails_before_cursor_allocation() {
    let capture = capture(
        Arc::new(NavWorld::from_parts(
            open_world(8, 8),
            TransportGraph::default(),
            vec![],
        )),
        tile(0, 0, 0),
        tile(2, 2, 0),
        FindOptions::default(),
        83,
        BankRows::default(),
        None,
        None,
        None,
    );
    let preflight = nav::router::SearchCursor::preflight(
        nav::router::SearchRequestShape::Single {
            options: capture.opts,
            avoid_capacity: 0,
            max_settles: nav::router::NODE_BUDGET,
        },
        &capture.lease,
        &capture.state,
    )
    .expect("single request preflight");
    let request = nav::router::SearchRequest::Single {
        from: capture.from,
        to: capture.to,
        options: capture.opts,
        avoid: Box::new([AvoidRect {
            min_x: 1,
            max_x: 1,
            min_z: 1,
            max_z: 1,
            level: Some(0),
        }]),
        max_settles: nav::router::NODE_BUDGET,
    };
    let lease = capture.lease.clone();
    let state = Arc::clone(&capture.state);
    let mut result = None;
    let measured = allocation_counter::measure(|| {
        result = Some(nav::router::SearchCursor::new_preflighted(
            request, lease, state, preflight,
        ));
    });
    assert_eq!(measured.count_total, 0);
    assert!(matches!(
        result.expect("cursor result"),
        Err(nav::router::SearchCursorError::PreflightMismatch)
    ));
}

#[test]
fn inspection_bounds_charge_future_layout_and_zone_peak_once() {
    let capture = capture_with_named_zone(true);
    let (_, preflights) =
        InspectSubmission::for_capture("test", &capture).expect("preflight search phases");
    let strict = preflights.strict.as_ref().expect("strict route preflight");
    let strict_request = nav::router::SearchRequest::Single {
        from: capture.from,
        to: capture.to,
        options: capture.opts,
        avoid: capture.avoid.clone().into_boxed_slice(),
        max_settles: strict_budget(&capture),
    };
    assert_eq!(
        strict.reservation().future_layout_bytes,
        nav::router::SearchCursor::future_layout_bytes_for(
            &strict_request,
            &capture.lease,
            &capture.state,
        )
        .expect("exact strict future layout")
    );

    let blockers = preflights
        .blockers_with_bank
        .as_ref()
        .expect("bank-fetch blocker preflight");
    let zone_table = capture.world.graph.zones.as_ref().expect("zoned fixture");
    let mut filter = ZoneFilterOwned::new(
        zone_table.zones().len(),
        capture.state.combat_level,
        &[capture.from, capture.to],
        capture.opts.zones,
    )
    .expect("owned blocker filter");
    filter.select_destination(Some(capture.to));
    let blocker_request = nav::router::SearchRequest::BlockingZones {
        from: capture.from,
        targets: Box::new([capture.to]),
        diagnosis: nav::router::BlockerDiagnosis::SingleTarget,
        options: capture.opts,
        avoid: capture.avoid.clone().into_boxed_slice(),
        max_settles: nav::router::NODE_BUDGET,
        zone_filter: filter,
    };
    let reservation = blockers.reservation();
    assert_eq!(
        reservation.future_layout_bytes,
        nav::router::SearchCursor::future_layout_bytes_for(
            &blocker_request,
            &capture.lease,
            &capture.state,
        )
        .expect("exact blocker future layout")
    );
    assert!(reservation.zone_filter_bytes > 0);
    assert!(reservation.peak_bytes >= reservation.zone_filter_bytes);

    let mut bounds = InspectSearchBounds::default();
    bounds
        .include(blockers, InspectSearchKind::Blockers)
        .expect("blocker phase charge");
    let expected =
        blockers.input_bytes() + reservation.future_layout_bytes + reservation.peak_bytes;
    assert_eq!(bounds.blocker_search_bytes, expected);
    assert_ne!(
        bounds.blocker_search_bytes,
        expected + reservation.zone_filter_bytes,
        "zone_filter_bytes is informational and already included in peak_bytes"
    );
}

#[test]
fn shared_world_state_charge_matches_one_arc_allocation() {
    let state = WorldState::empty();
    let retained = state
        .try_shared_retained_bytes()
        .expect("empty world state has a bounded backing charge");
    let mut shared = None;
    let measured = allocation_counter::measure(|| {
        shared = Some(Arc::new(state));
    });
    assert_eq!(measured.count_total, 1);
    assert_eq!(usize::try_from(measured.bytes_total).unwrap(), retained);
    drop(shared);
}

#[test]
fn budget_exhausted_distinct_from_nopath_strict_stand_post() {
    let from = tile(0, 0, 0);
    let far = tile(9, 9, 0);
    let open = Arc::new(NavWorld::from_parts(
        open_world(10, 10),
        TransportGraph::default(),
        vec![],
    ));
    let exhausted = calculate(&capture(
        Arc::clone(&open),
        from,
        far,
        FindOptions {
            allow_wilderness: true,
            ..FindOptions::default()
        },
        71,
        BankRows::default(),
        Some(8),
        None,
        None,
    ));
    assert!(!exhausted.ok);
    assert_eq!(exhausted.reason, "BudgetExhausted");
    assert!(!exhausted.bank_planned);

    let blocked = calculate(&capture(
        Arc::clone(&open),
        from,
        far,
        FindOptions {
            allow_wilderness: true,
            ..FindOptions::default()
        },
        72,
        BankRows::default(),
        None,
        None,
        None,
    ));
    assert!(
        blocked.ok,
        "unbounded open walk must path, got {}",
        blocked.reason
    );

    let bank_opts = FindOptions {
        allow_wilderness: true,
        allow_bank_fetch: true,
        ..FindOptions::default()
    };
    let mut stand_graph = TransportGraph::default();
    let stand_far = tile(9, 9, 0);
    let stand_dest = tile(2, 2, 1);
    stand_graph.at.entry(from).or_default().push(0);
    stand_graph.edges.push(edge(
        TransportKind::Boat,
        from,
        stand_dest,
        381,
        vec![(995, 10)],
    ));
    let stand_world = Arc::new(NavWorld::from_parts(
        open_world(10, 10),
        stand_graph,
        vec![BankStand {
            name: "Bank booth".into(),
            tile: stand_far,
            access: BankAccess::Booth { op: 2 },
        }],
    ));
    let stand_ex = calculate(&capture(
        stand_world,
        from,
        stand_dest,
        bank_opts,
        73,
        BankRows {
            origin: Origin::Session,
            rows: vec![(995, 10)],
        },
        None,
        Some(8),
        None,
    ));
    assert_eq!(stand_ex.reason, "BudgetExhausted");
    assert!(!stand_ex.bank_planned);

    let mut post_graph = TransportGraph::default();
    let post_dest = tile(9, 9, 1);
    let stand_near = tile(1, 1, 0);
    post_graph.at.entry(from).or_default().push(0);
    post_graph.edges.push(edge(
        TransportKind::Boat,
        from,
        tile(0, 1, 1),
        381,
        vec![(995, 10)],
    ));
    let post_world = Arc::new(NavWorld::from_parts(
        open_world(10, 10),
        post_graph,
        vec![BankStand {
            name: "Bank booth".into(),
            tile: stand_near,
            access: BankAccess::Booth { op: 2 },
        }],
    ));
    let stand_ok_post_ex = calculate(&capture(
        post_world,
        from,
        post_dest,
        bank_opts,
        74,
        BankRows {
            origin: Origin::Session,
            rows: vec![(995, 10)],
        },
        None,
        None,
        Some(8),
    ));
    assert_eq!(stand_ok_post_ex.reason, "BudgetExhausted");
    assert!(stand_ok_post_ex.bank_planned);
}

#[test]
fn admission_refuses_fourth_when_ring_and_held_full() {
    let world = world_with(TransportGraph::default(), vec![]);
    let navs = Arc::new(Mutex::new(HashMap::new()));
    for id in 1..=3 {
        queue_inspect(
            &navs,
            "p",
            &Some(Arc::clone(&world)),
            Some(WorldState::empty()),
            BankRows::default(),
            None,
            None,
            req(tile(0, 0, 0), tile(1, 1, 0), id),
        );
        let _ = wait_latest(&navs, id);
    }
    {
        let all = navs.lock().unwrap();
        let bot = all.get("p").unwrap();
        assert_eq!(bot.inspect.latest.as_ref().unwrap().request_id, 2);
        assert_eq!(bot.inspect.prev.as_ref().unwrap().request_id, 1);
        assert_eq!(bot.inspect.held.as_ref().unwrap().request_id, 3);
        assert_eq!(bot.inspect.accepted_id, 3);
    }
    queue_inspect(
        &navs,
        "p",
        &Some(world),
        Some(WorldState::empty()),
        BankRows::default(),
        None,
        None,
        req(tile(0, 0, 0), tile(2, 2, 0), 4),
    );
    {
        let all = navs.lock().unwrap();
        let bot = all.get("p").unwrap();
        assert_eq!(bot.inspect.accepted_id, 3, "fourth must not be accepted");
        assert_eq!(bot.inspect.held.as_ref().unwrap().request_id, 3);
        assert_eq!(bot.inspect.latest.as_ref().unwrap().request_id, 2);
        assert_eq!(bot.inspect.refused, [4, 0, 0]);
    }
}

#[test]
fn apply_ack_rejects_future_and_wrong_generation() {
    let mut nav = InspectNav::default();
    nav.publish(InspectTerminal::refusal(1, 0, "NoPath"));
    nav.publish(InspectTerminal::refusal(2, 0, "NoPath"));
    nav.publish(InspectTerminal::refusal(3, 0, "NoPath"));
    let prev_seq = nav.prev.as_ref().unwrap().seq;
    nav.apply_ack(99, 0);
    assert_eq!(nav.held.as_ref().unwrap().request_id, 3);
    nav.apply_ack(prev_seq, 7);
    assert_eq!(nav.held.as_ref().unwrap().request_id, 3);
    nav.apply_ack(prev_seq, 0);
    assert!(nav.held.is_none());
    assert_eq!(nav.latest.as_ref().unwrap().request_id, 3);
}

#[test]
fn request_id_zero_publishes_preview() {
    let world = world_with(TransportGraph::default(), vec![]);
    let navs = Arc::new(Mutex::new(HashMap::new()));
    queue_inspect(
        &navs,
        "p",
        &Some(world),
        Some(WorldState::empty()),
        BankRows::default(),
        None,
        None,
        req(tile(0, 0, 0), tile(2, 2, 1), 0),
    );
    let term = wait_latest(&navs, 0);
    assert_eq!(term.request_id, 0);
    assert!(!term.ok);
    assert_eq!(term.reason, "NoPath");
    assert!(term.seq != 0);
}

#[test]
fn can_admit_counts_only_unobserved_after_full_ack() {
    let mut nav = InspectNav::default();
    nav.publish(InspectTerminal::refusal(1, 0, "NoPath"));
    nav.publish(InspectTerminal::refusal(2, 0, "NoPath"));
    nav.publish(InspectTerminal::refusal(3, 0, "NoPath"));
    assert!(!nav.can_admit(false));
    let prev = nav.prev.as_ref().unwrap().seq;
    nav.apply_ack(prev, 0);
    let latest = nav.latest.as_ref().unwrap().seq;
    nav.apply_ack(latest, 0);
    assert_eq!(nav.unobserved_count(), 0);
    assert!(nav.can_admit(false));
    nav.executing = true;
    assert!(
        nav.can_admit(false),
        "acked ring + running still admits pending"
    );
    nav.executing = true;
    assert!(nav.can_admit(true));
}

#[test]
fn admission_refuse_does_not_accept_when_unobserved_full() {
    let world = world_with(TransportGraph::default(), vec![]);
    let navs = Arc::new(Mutex::new(HashMap::new()));
    for id in 1..=3 {
        queue_inspect(
            &navs,
            "p",
            &Some(Arc::clone(&world)),
            Some(WorldState::empty()),
            BankRows::default(),
            None,
            None,
            req(tile(0, 0, 0), tile(1, 1, 0), id),
        );
        let _ = wait_latest(&navs, id);
    }
    queue_inspect(
        &navs,
        "p",
        &Some(Arc::clone(&world)),
        Some(WorldState::empty()),
        BankRows::default(),
        None,
        None,
        req(tile(0, 0, 0), tile(2, 2, 0), 4),
    );
    {
        let all = navs.lock().unwrap();
        let bot = all.get("p").unwrap();
        assert_eq!(bot.inspect.accepted_id, 3);
        assert!(bot.inspect.pending.is_none());
        assert_eq!(bot.inspect.held.as_ref().unwrap().request_id, 3);
        assert_eq!(bot.inspect.refused, [4, 0, 0]);
        assert_eq!(bot.inspect.latest.as_ref().unwrap().request_id, 2);
    }
    {
        let mut all = navs.lock().unwrap();
        let bot = all.get_mut("p").unwrap();
        let latest = bot.inspect.latest.as_ref().unwrap().seq;
        bot.inspect.apply_ack(latest, bot.inspect.generation);
    }
    queue_inspect(
        &navs,
        "p",
        &Some(world),
        Some(WorldState::empty()),
        BankRows::default(),
        None,
        None,
        req(tile(0, 0, 0), tile(3, 3, 0), 5),
    );
    let term = wait_latest(&navs, 5);
    assert_eq!(term.request_id, 5);
}

#[test]
fn snapshot_only_overload_does_not_mailbox_or_clobber_latest() {
    let world = world_with(TransportGraph::default(), vec![]);
    let navs = Arc::new(Mutex::new(HashMap::new()));
    fill_three_id0(&navs, &world);
    let before = {
        let all = navs.lock().unwrap();
        let bot = all.get("p").unwrap();
        (
            bot.inspect.latest.as_ref().map(|t| t.seq),
            bot.inspect.held.as_ref().map(|t| t.seq),
            bot.inspect.refused,
        )
    };
    queue_inspect(
        &navs,
        "p",
        &Some(world),
        Some(WorldState::empty()),
        BankRows::default(),
        None,
        None,
        req(tile(0, 0, 0), tile(2, 2, 1), 0),
    );
    {
        let all = navs.lock().unwrap();
        let bot = all.get("p").unwrap();
        assert_eq!(bot.inspect.latest.as_ref().map(|t| t.seq), before.0);
        assert_eq!(bot.inspect.held.as_ref().map(|t| t.seq), before.1);
        assert_eq!(bot.inspect.refused, [0, 0, 0]);
        assert_eq!(bot.inspect.accepted_id, 0);
        assert!(bot.inspect.pending.is_none());
    }
}

#[test]
fn duplicate_running_id_does_not_double_reserve() {
    let world = world_with(TransportGraph::default(), vec![]);
    let navs = Arc::new(Mutex::new(HashMap::new()));
    queue_inspect(
        &navs,
        "p",
        &Some(Arc::clone(&world)),
        Some(WorldState::empty()),
        BankRows::default(),
        None,
        None,
        req(tile(0, 0, 0), tile(1, 1, 0), 81),
    );
    wait_running(&navs, 81);
    queue_inspect(
        &navs,
        "p",
        &Some(world),
        Some(WorldState::empty()),
        BankRows::default(),
        None,
        None,
        req(tile(0, 0, 0), tile(2, 2, 0), 81),
    );
    {
        let all = navs.lock().unwrap();
        let bot = all.get("p").unwrap();
        assert_eq!(bot.inspect.running_id, 81);
        assert!(bot.inspect.pending.is_none());
        assert_eq!(bot.inspect.pending_id, 0);
    }
    let _ = wait_latest(&navs, 81);
    wait_idle(&navs);
}

#[test]
fn reset_rejects_old_generation_ack() {
    let mut nav = InspectNav::default();
    nav.publish(InspectTerminal::refusal(1, 0, "NoPath"));
    nav.publish(InspectTerminal::refusal(2, 0, "NoPath"));
    nav.publish(InspectTerminal::refusal(3, 0, "NoPath"));
    let old_seq = nav.prev.as_ref().unwrap().seq;
    let old_gen = nav.generation;
    let mut bot = NavBot {
        inspect: nav,
        ..Default::default()
    };
    reset_inspect(&mut bot);
    bot.inspect.publish(InspectTerminal::refusal(
        11,
        bot.inspect.generation,
        "NoPath",
    ));
    bot.inspect.publish(InspectTerminal::refusal(
        12,
        bot.inspect.generation,
        "NoPath",
    ));
    bot.inspect.publish(InspectTerminal::refusal(
        13,
        bot.inspect.generation,
        "NoPath",
    ));
    assert_eq!(bot.inspect.held.as_ref().unwrap().request_id, 13);
    bot.inspect.apply_ack(old_seq, old_gen);
    assert_eq!(
        bot.inspect.held.as_ref().unwrap().request_id,
        13,
        "old-session ACK cannot free the new generation"
    );
    let prev = bot.inspect.prev.as_ref().unwrap().seq;
    bot.inspect.apply_ack(prev, bot.inspect.generation);
    assert!(bot.inspect.held.is_none());
    assert_eq!(bot.inspect.latest.as_ref().unwrap().request_id, 13);
}

#[test]
#[should_panic(expected = "inspect publish overflow")]
fn publish_overflow_panics_when_held_full() {
    let mut nav = InspectNav::default();
    nav.publish(InspectTerminal::refusal(1, 0, "NoPath"));
    nav.publish(InspectTerminal::refusal(2, 0, "NoPath"));
    nav.publish(InspectTerminal::refusal(3, 0, "NoPath"));
    assert!(!nav.can_admit(false));
    nav.publish(InspectTerminal::refusal(4, 0, "NoPath"));
}

fn encode_inspect_bytes(nav: &InspectNav, tick: u64, hold: bool) -> Vec<u8> {
    use script::isolate_fb::{
        encode_snapshot_with_native, NativeFactsInput, ReachViewInput, RouteInspectFactsInput,
        SnapshotInput,
    };
    let latest_hops = nav
        .latest
        .as_ref()
        .map(|t| hop_inputs(&t.hops))
        .unwrap_or_default();
    let prev_hops = nav
        .prev
        .as_ref()
        .map(|t| hop_inputs(&t.hops))
        .unwrap_or_default();
    let latest_in = nav
        .latest
        .as_ref()
        .map(|t| terminal_to_input(t, &latest_hops))
        .unwrap_or_default();
    let prev_in = nav
        .prev
        .as_ref()
        .map(|t| terminal_to_input(t, &prev_hops))
        .unwrap_or_default();
    let posted = nav.posted();
    encode_snapshot_with_native(
        &SnapshotInput {
            tick,
            here: None,
            ingame: true,
            walk_outcome_cancel_reason: script::isolate_fb::WalkCancelReason::None,
            user_move_intent_seq: 0,
            inv: &[],
            inv_size: 28,
            stats: &[],
            booths: &[],
            nearest_booth: None,
            banks: &[],
            bank: &[],
            bank_side: &[],
            bank_open: false,
            bank_loaded: false,
            bank_generation: 0,
            count_dialog_open: false,
            withdraw_x_result_seq: 0,
            withdraw_x_result: false,
            withdraw_load_result_seq: 0,
            withdraw_load_result: false,
            bank_op_result_seq: 0,
            bank_op_result: false,
            hold,
            ours: false,
            npcs: &[],
            locs: &[],
            players: &[],
            ground: &[],
            equipment: &[],
            chat_open: false,
            chat_continue: false,
            chat_text: None,
            chat_options: &[],
            side_tab: -1,
            varps: &[],
            combat_styles: &[],
            run_energy: 0,
            run_enabled: false,
            retaliate_enabled: false,
            my_name: None,
            in_combat: false,
            animating: false,
            main_modal_id: -1,
            chat_modal_id: -1,
            make_products: &[],
            side_tab_ifaces: &[],
            spell_buttons: &[],
            chat_lines: &[],
            bank_note_on: -1,
            bank_note_off: -1,
            scene_state: 0,
            weight: 0,
            combat_level: 0,
            camera_yaw: 0,
            camera_pitch: 0,
            teleports_enabled: false,
            self_slot: 0,
            trade_offer_open: false,
            trade_confirm_open: false,
            trade_partner: None,
            trade_mine: &[],
            trade_theirs: &[],
            trade_side: &[],
            trade_accept_id: -1,
            trade_decline_id: -1,
            shop_open: false,
            shop_stock: &[],
            reach: ReachViewInput::UNAVAILABLE,
            attacked_by_player: false,
            self_target_kind: 0,
            self_target_index: -1,
            widgets: &[],
        },
        NativeFactsInput {
            route_inspect: RouteInspectFactsInput {
                latest: latest_in,
                prev: prev_in,
                running_id: posted.running_id,
                pending_id: posted.pending_id,
                accepted_id: posted.accepted_id,
                replaced_id: posted.replaced_id,
                replaced_prev_id: posted.replaced_prev_id,
                refused_id: posted.refused_id,
                refused_id_2: posted.refused_id_2,
                refused_id_3: posted.refused_id_3,
                unobserved: posted.unobserved,
            },
            ..Default::default()
        },
    )
}

fn inspect_begin(iso: &script::LoadIsolate) -> u64 {
    inspect_begin_at(iso, tile(0, 0, 0), tile(1, 1, 0))
}

fn inspect_begin_at(iso: &script::LoadIsolate, from: WorldTile, to: WorldTile) -> u64 {
    iso.probe(&format!(
            "globalThis.rustyscript.functions.__rs2b0t_inspect({{op:'begin',from:{{x:{},z:{},level:{}}},to:{{x:{},z:{},level:{}}},allow_wilderness:true}})",
            from.x, from.z, from.level, to.x, to.z, to.level
        ))
        .unwrap()
        .as_u64()
        .expect("token")
}

fn inspect_settled(iso: &script::LoadIsolate, token: u64) -> bool {
    iso.probe(&format!(
        "globalThis.rustyscript.functions.__rs2b0t_inspect({{op:'settled',token:{token}}})"
    ))
    .unwrap()
    .as_bool()
    .unwrap_or(false)
}

fn inspect_value(iso: &script::LoadIsolate, token: u64) -> serde_json::Value {
    iso.probe(&format!(
        "globalThis.rustyscript.functions.__rs2b0t_inspect({{op:'value',token:{token}}})"
    ))
    .unwrap()
}

fn drain_inspect_acks(iso: &script::LoadIsolate) -> Vec<(u64, u64)> {
    iso.drain_interacts()
        .into_iter()
        .filter_map(|req| match req {
            script::shim::InteractReq::InspectAck { seq, generation } => Some((seq, generation)),
            _ => None,
        })
        .collect()
}

fn isolate_begin_and_authorize(
    iso: &script::LoadIsolate,
    from: WorldTile,
    to: WorldTile,
    tick: u64,
) -> Option<u64> {
    let token = inspect_begin_at(iso, from, to);
    if inspect_settled(iso, token) {
        return None;
    }
    iso.probe(&format!(
            "globalThis.__rs_api.request({{op:'inspect-route',from:{{x:{},z:{},level:{}}},to:{{x:{},z:{},level:{}}},allow_wilderness:true,request_id:{token}}})",
            from.x, from.z, from.level, to.x, to.z, to.level
        ))
        .expect("inspect-route request");
    iso.on_game_tick(tick);
    let _ = iso.probe("true").expect("tick flush");
    let forwarded = iso.drain_interacts().into_iter().any(|req| {
        matches!(
            req,
            script::shim::InteractReq::InspectRoute { request_id, .. } if request_id == token
        )
    });
    forwarded.then_some(token)
}

fn inspect_force_timeout(iso: &script::LoadIsolate, token: u64) {
    iso.probe(&format!(
            "globalThis.rustyscript.functions.__rs2b0t_inspect({{op:'value',token:{token},timed_out:true}})"
        ))
        .unwrap();
}

fn apply_isolate_acks(navs: &Arc<Mutex<HashMap<String, NavBot>>>, iso: &script::LoadIsolate) {
    let acks = drain_inspect_acks(iso);
    let mut all = navs.lock().unwrap();
    let bot = all.get_mut("p").unwrap();
    for (seq, generation) in acks {
        bot.inspect.apply_ack(seq, generation);
    }
}

fn post_and_apply(
    iso: &script::LoadIsolate,
    navs: &Arc<Mutex<HashMap<String, NavBot>>>,
    tick: u64,
) {
    iso.post_snapshot(encode_inspect_bytes(
        &navs.lock().unwrap().get("p").unwrap().inspect,
        tick,
        false,
    ));
    let _ = iso.probe("true").unwrap();
    apply_isolate_acks(navs, iso);
}

#[test]
fn integrated_host_publish_bytes_isolate_ack_without_new_request() {
    let src = r#"
export const apiVersion = 2;
export function tick() {}
"#;
    let iso =
        script::LoadIsolate::spawn(src.into(), script::LoadShape::NativeTick, vec![]).unwrap();
    let world = world_with(TransportGraph::default(), vec![]);
    let navs = Arc::new(Mutex::new(HashMap::new()));

    let a = inspect_begin_at(&iso, tile(0, 0, 0), tile(1, 1, 0));
    let b = inspect_begin_at(&iso, tile(0, 0, 0), tile(2, 2, 0));
    queue_inspect(
        &navs,
        "p",
        &Some(Arc::clone(&world)),
        Some(WorldState::empty()),
        BankRows::default(),
        None,
        None,
        req(tile(0, 0, 0), tile(1, 1, 0), a),
    );
    wait_running(&navs, a);
    queue_inspect(
        &navs,
        "p",
        &Some(Arc::clone(&world)),
        Some(WorldState::empty()),
        BankRows::default(),
        None,
        None,
        req(tile(0, 0, 0), tile(2, 2, 0), b),
    );
    let c = inspect_begin_at(&iso, tile(0, 0, 0), tile(3, 3, 0));
    queue_inspect(
        &navs,
        "p",
        &Some(Arc::clone(&world)),
        Some(WorldState::empty()),
        BankRows::default(),
        None,
        None,
        req(tile(0, 0, 0), tile(3, 3, 0), c),
    );
    {
        let all = navs.lock().unwrap();
        let bot = all.get("p").unwrap();
        assert_eq!(bot.inspect.running_id, a);
        assert_eq!(bot.inspect.pending_id, c);
        assert_eq!(bot.inspect.replaced_id, b);
        assert_eq!(bot.inspect.latest.as_ref().unwrap().request_id, b);
        assert_eq!(bot.inspect.latest.as_ref().unwrap().reason, "stale");
        assert!(bot.inspect.executing);
    }
    iso.post_snapshot(encode_inspect_bytes(
        &navs.lock().unwrap().get("p").unwrap().inspect,
        1,
        false,
    ));
    let _ = iso.probe("true").unwrap();
    assert!(inspect_settled(&iso, b));
    assert_eq!(inspect_value(&iso, b)["reason"], "stale");
    assert!(
        !inspect_settled(&iso, a),
        "A must not stale before own drain"
    );
    assert!(!inspect_settled(&iso, c));
    let acks = drain_inspect_acks(&iso);
    assert!(
        !acks.is_empty(),
        "ACK must flush without a new inspect request"
    );
    {
        let mut all = navs.lock().unwrap();
        let bot = all.get_mut("p").unwrap();
        for (seq, generation) in acks {
            bot.inspect.apply_ack(seq, generation);
        }
    }
    let _ = wait_latest(&navs, a);
    let _ = wait_latest(&navs, c);
    iso.post_snapshot(encode_inspect_bytes(
        &navs.lock().unwrap().get("p").unwrap().inspect,
        2,
        false,
    ));
    let _ = iso.probe("true").unwrap();
    assert!(inspect_settled(&iso, a));
    assert!(inspect_settled(&iso, c));
    assert_eq!(inspect_value(&iso, a)["request_id"], a);
    assert_eq!(inspect_value(&iso, c)["request_id"], c);
    apply_isolate_acks(&navs, &iso);
    assert!(!navs.lock().unwrap().get("p").unwrap().inspect.executing);

    let d = inspect_begin_at(&iso, tile(0, 0, 0), tile(4, 4, 0));
    queue_inspect(
        &navs,
        "p",
        &Some(Arc::clone(&world)),
        Some(WorldState::empty()),
        BankRows::default(),
        None,
        None,
        req(tile(0, 0, 0), tile(4, 4, 0), d),
    );
    wait_running(&navs, d);
    let e = inspect_begin_at(&iso, tile(0, 0, 0), tile(5, 5, 0));
    queue_inspect(
        &navs,
        "p",
        &Some(Arc::clone(&world)),
        Some(WorldState::empty()),
        BankRows::default(),
        None,
        None,
        req(tile(0, 0, 0), tile(5, 5, 0), e),
    );
    {
        let all = navs.lock().unwrap();
        let bot = all.get("p").unwrap();
        assert_eq!(bot.inspect.running_id, d);
        assert_eq!(bot.inspect.pending_id, e);
        assert!(
            bot.inspect.can_admit(false) || bot.inspect.pending.is_some(),
            "fully acked ring must still admit running+pending"
        );
    }
    let _ = wait_latest(&navs, d);
    let _ = wait_latest(&navs, e);
    post_and_apply(&iso, &navs, 3);
    assert!(inspect_settled(&iso, d));
    assert!(inspect_settled(&iso, e));

    let f = inspect_begin_at(&iso, tile(0, 0, 0), tile(1, 2, 0));
    queue_inspect(
        &navs,
        "p",
        &Some(Arc::clone(&world)),
        Some(WorldState::empty()),
        BankRows::default(),
        None,
        None,
        req(tile(0, 0, 0), tile(1, 2, 0), f),
    );
    let _ = wait_latest(&navs, f);
    let g = inspect_begin_at(&iso, tile(0, 0, 0), tile(1, 3, 0));
    queue_inspect(
        &navs,
        "p",
        &Some(Arc::clone(&world)),
        Some(WorldState::empty()),
        BankRows::default(),
        None,
        None,
        req(tile(0, 0, 0), tile(1, 3, 0), g),
    );
    let _ = wait_latest(&navs, g);
    let h = inspect_begin_at(&iso, tile(0, 0, 0), tile(1, 4, 0));
    queue_inspect(
        &navs,
        "p",
        &Some(Arc::clone(&world)),
        Some(WorldState::empty()),
        BankRows::default(),
        None,
        None,
        req(tile(0, 0, 0), tile(1, 4, 0), h),
    );
    let _ = wait_latest(&navs, h);
    {
        let all = navs.lock().unwrap();
        let bot = all.get("p").unwrap();
        assert_eq!(bot.inspect.held.as_ref().unwrap().request_id, h);
    }
    iso.post_snapshot(encode_inspect_bytes(
        &navs.lock().unwrap().get("p").unwrap().inspect,
        4,
        false,
    ));
    let _ = iso.probe("true").unwrap();
    assert!(inspect_settled(&iso, f));
    assert!(inspect_settled(&iso, g));
    assert!(!inspect_settled(&iso, h), "held is not posted until ack");
    let flush_acks = drain_inspect_acks(&iso);
    assert!(!flush_acks.is_empty());
    {
        let mut all = navs.lock().unwrap();
        let bot = all.get_mut("p").unwrap();
        for (seq, generation) in flush_acks {
            bot.inspect.apply_ack(seq, generation);
        }
        assert!(bot.inspect.held.is_none());
        assert_eq!(bot.inspect.latest.as_ref().unwrap().request_id, h);
    }
    iso.post_snapshot(encode_inspect_bytes(
        &navs.lock().unwrap().get("p").unwrap().inspect,
        5,
        false,
    ));
    let _ = iso.probe("true").unwrap();
    assert!(inspect_settled(&iso, h));
    apply_isolate_acks(&navs, &iso);

    let held_before_id0 = navs
        .lock()
        .unwrap()
        .get("p")
        .unwrap()
        .inspect
        .latest
        .as_ref()
        .map(|t| t.request_id);
    queue_inspect(
        &navs,
        "p",
        &Some(Arc::clone(&world)),
        Some(WorldState::empty()),
        BankRows::default(),
        None,
        None,
        req(tile(0, 0, 0), tile(6, 6, 0), 0),
    );
    let zero = wait_latest(&navs, 0);
    assert_eq!(zero.request_id, 0);
    assert_ne!(zero.seq, 0);
    assert!(!inspect_settled(&iso, 0));
    post_and_apply(&iso, &navs, 6);
    let _ = held_before_id0;

    let before_invented = navs.lock().unwrap().get("p").unwrap().inspect.accepted_id;
    iso.probe(
            "globalThis.__rs_api.request({op:'inspect-route',from:{x:0,z:0,level:0},to:{x:1,z:1,level:0},request_id:99})",
        )
        .unwrap();
    iso.on_game_tick(7);
    let leaked = iso.drain_interacts().into_iter().any(|req| {
        matches!(
            req,
            script::shim::InteractReq::InspectRoute { request_id: 99, .. }
        )
    });
    assert!(!leaked, "invented token must not reach the host queue");
    assert!(inspect_settled(&iso, 99));
    assert_eq!(
        navs.lock().unwrap().get("p").unwrap().inspect.accepted_id,
        before_invented
    );

    let hold_tok = inspect_begin(&iso);
    iso.post_snapshot(encode_inspect_bytes(
        &navs.lock().unwrap().get("p").unwrap().inspect,
        8,
        true,
    ));
    let _ = iso.probe("true").unwrap();
    assert!(!inspect_settled(&iso, hold_tok));

    let live = inspect_begin_at(&iso, tile(0, 0, 0), tile(7, 7, 0));
    queue_inspect(
        &navs,
        "p",
        &Some(world),
        Some(WorldState::empty()),
        BankRows::default(),
        None,
        None,
        req(tile(0, 0, 0), tile(7, 7, 0), live),
    );
    wait_running(&navs, live);
    let stale_ack = {
        let all = navs.lock().unwrap();
        let bot = all.get("p").unwrap();
        (
            bot.inspect.latest.as_ref().map(|t| t.seq).unwrap_or(1),
            bot.inspect.generation,
        )
    };
    {
        let mut all = navs.lock().unwrap();
        reset_inspect(all.get_mut("p").unwrap());
        let bot = all.get_mut("p").unwrap();
        assert!(!bot.inspect.executing);
        bot.inspect.apply_ack(stale_ack.0, stale_ack.1);
        assert_eq!(bot.inspect.observed_seq, 0);
        assert!(bot.inspect.latest.is_none());
    }
    iso.join();
}

fn spawn_native_iso() -> script::LoadIsolate {
    script::LoadIsolate::spawn(
        r#"
export const apiVersion = 2;
export function tick() {}
"#
        .into(),
        script::LoadShape::NativeTick,
        vec![],
    )
    .unwrap()
}

#[test]
fn integrated_id0_full_then_authorized_registered_refuses_not_timeout() {
    let iso = spawn_native_iso();
    let world = world_with(TransportGraph::default(), vec![]);
    let navs = Arc::new(Mutex::new(HashMap::new()));
    fill_three_id0(&navs, &world);
    let ring_before = {
        let all = navs.lock().unwrap();
        let bot = all.get("p").unwrap();
        (
            bot.inspect.latest.as_ref().map(|t| t.seq),
            bot.inspect.prev.as_ref().map(|t| t.seq),
            bot.inspect.held.as_ref().map(|t| t.seq),
            bot.inspect.accepted_id,
        )
    };
    let dest = tile(2, 3, 1);
    let token = isolate_begin_and_authorize(&iso, tile(0, 0, 0), dest, 1)
        .expect("authorize must succeed when isolate has not seen host fullness");
    queue_inspect(
        &navs,
        "p",
        &Some(world),
        Some(WorldState::empty()),
        BankRows::default(),
        None,
        None,
        req(tile(0, 0, 0), dest, token),
    );
    {
        let all = navs.lock().unwrap();
        let bot = all.get("p").unwrap();
        assert_eq!(bot.inspect.latest.as_ref().map(|t| t.seq), ring_before.0);
        assert_eq!(bot.inspect.prev.as_ref().map(|t| t.seq), ring_before.1);
        assert_eq!(bot.inspect.held.as_ref().map(|t| t.seq), ring_before.2);
        assert_eq!(bot.inspect.accepted_id, ring_before.3);
        assert!(bot.inspect.refused.contains(&token));
        assert!(bot.inspect.pending.is_none());
    }
    iso.post_snapshot(encode_inspect_bytes(
        &navs.lock().unwrap().get("p").unwrap().inspect,
        2,
        false,
    ));
    let _ = iso.probe("true").unwrap();
    assert!(inspect_settled(&iso, token));
    assert_eq!(inspect_value(&iso, token)["reason"], "stale");
    assert_ne!(inspect_value(&iso, token)["reason"], "waiter-timeout");
    iso.join();
}

#[test]
fn integrated_timeout_orphans_then_authorized_registered_refuses_not_timeout() {
    let iso = spawn_native_iso();
    let world = world_with(TransportGraph::default(), vec![]);
    let navs = Arc::new(Mutex::new(HashMap::new()));
    let mut tokens = Vec::new();
    for (i, dest) in [tile(2, 2, 1), tile(2, 3, 1), tile(2, 4, 1)]
        .into_iter()
        .enumerate()
    {
        let token = isolate_begin_and_authorize(&iso, tile(0, 0, 0), dest, 10 + i as u64)
            .expect("registered begin while isolate unsettled < 3");
        queue_inspect(
            &navs,
            "p",
            &Some(Arc::clone(&world)),
            Some(WorldState::empty()),
            BankRows::default(),
            None,
            None,
            req(tile(0, 0, 0), dest, token),
        );
        wait_idle(&navs);
        tokens.push(token);
    }
    {
        let all = navs.lock().unwrap();
        let bot = all.get("p").unwrap();
        assert_eq!(bot.inspect.unobserved_count(), 3);
        assert!(!bot.inspect.can_admit(false));
    }
    for token in &tokens {
        inspect_force_timeout(&iso, *token);
        assert_eq!(inspect_value(&iso, *token)["reason"], "waiter-timeout");
    }
    let dest = tile(2, 5, 1);
    let next = isolate_begin_and_authorize(&iso, tile(0, 0, 0), dest, 20)
        .expect("new begin after timeout; isolate has not observed host fullness");
    queue_inspect(
        &navs,
        "p",
        &Some(world),
        Some(WorldState::empty()),
        BankRows::default(),
        None,
        None,
        req(tile(0, 0, 0), dest, next),
    );
    {
        let all = navs.lock().unwrap();
        let bot = all.get("p").unwrap();
        assert!(bot.inspect.refused.contains(&next));
        assert_eq!(bot.inspect.unobserved_count(), 3);
        assert_ne!(bot.inspect.accepted_id, next);
        assert_eq!(bot.inspect.held.as_ref().unwrap().request_id, tokens[2]);
    }
    iso.post_snapshot(encode_inspect_bytes(
        &navs.lock().unwrap().get("p").unwrap().inspect,
        21,
        false,
    ));
    let _ = iso.probe("true").unwrap();
    assert!(inspect_settled(&iso, next));
    assert_eq!(inspect_value(&iso, next)["reason"], "stale");
    assert_ne!(inspect_value(&iso, next)["reason"], "waiter-timeout");
    iso.join();
}

#[test]
fn integrated_mixed_id0_registered_ack_progress_does_not_starve() {
    let iso = spawn_native_iso();
    let world = world_with(TransportGraph::default(), vec![]);
    let navs = Arc::new(Mutex::new(HashMap::new()));
    for i in 0..6u64 {
        queue_inspect(
            &navs,
            "p",
            &Some(Arc::clone(&world)),
            Some(WorldState::empty()),
            BankRows::default(),
            None,
            None,
            req(tile(0, 0, 0), tile(2, 2, 1), 0),
        );
        wait_idle(&navs);
        assert_eq!(
            navs.lock()
                .unwrap()
                .get("p")
                .unwrap()
                .inspect
                .latest
                .as_ref()
                .map(|t| t.request_id),
            Some(0),
            "id0 stays enabled under mixed traffic"
        );
        let dest = tile(2, 6 + i as i32, 1);
        let token = isolate_begin_and_authorize(&iso, tile(0, 0, 0), dest, 30 + i)
            .expect("ACK progress must keep registered begin authorizable");
        queue_inspect(
            &navs,
            "p",
            &Some(Arc::clone(&world)),
            Some(WorldState::empty()),
            BankRows::default(),
            None,
            None,
            req(tile(0, 0, 0), dest, token),
        );
        wait_idle(&navs);
        post_and_apply(&iso, &navs, 40 + i);
        assert!(inspect_settled(&iso, token));
        let value = inspect_value(&iso, token);
        assert_ne!(value["reason"], "waiter-timeout");
        assert_eq!(value["request_id"], token);
    }
    fill_three_id0(&navs, &world);
    post_and_apply(&iso, &navs, 49);
    post_and_apply(&iso, &navs, 50);
    let dest = tile(2, 20, 1);
    let after = isolate_begin_and_authorize(&iso, tile(0, 0, 0), dest, 51)
        .expect("ACK after id0 fill must admit registered traffic");
    queue_inspect(
        &navs,
        "p",
        &Some(world),
        Some(WorldState::empty()),
        BankRows::default(),
        None,
        None,
        req(tile(0, 0, 0), dest, after),
    );
    wait_idle(&navs);
    post_and_apply(&iso, &navs, 52);
    assert!(inspect_settled(&iso, after));
    assert_ne!(inspect_value(&iso, after)["reason"], "waiter-timeout");
    assert_eq!(inspect_value(&iso, after)["request_id"], after);
    iso.join();
}

#[derive(Default)]
struct CaptureDriver {
    out: CaptureOut,
}

#[derive(Default)]
struct CaptureOut;

impl api::prot::Out for CaptureOut {
    fn p1_enc(&mut self, _opcode: i32) {}
    fn p1(&mut self, _value: i32) {}
    fn p2(&mut self, _value: i32) {}
    fn p4(&mut self, _value: i32) {}
    fn pjstr(&mut self, _value: &str) {}
}

impl api::interact::Driver for CaptureDriver {
    fn set_menu(&mut self, _slot: i32, _action: i32, _a: i32, _b: i32, _c: i32) {}

    fn do_action(&mut self, _slot: i32) -> bool {
        true
    }

    fn try_move(
        &mut self,
        _src_x: i32,
        _src_z: i32,
        _dx: i32,
        _dz: i32,
        _try_nearest: bool,
        _loc_width: i32,
        _loc_length: i32,
        _loc_angle: i32,
        _loc_shape: i32,
        _forceapproach: i32,
        _type: i32,
    ) -> bool {
        true
    }

    fn local_route(&self) -> Option<(i32, i32)> {
        None
    }

    fn build_base(&self) -> (i32, i32) {
        (0, 0)
    }

    fn loc_typecode(&self, _scene_x: i32, _scene_z: i32) -> Option<i32> {
        None
    }

    fn out(&mut self) -> &mut dyn api::prot::Out {
        &mut self.out
    }

    fn login(&mut self, _username: &str, _password: &str, _reconnect: bool) -> bool {
        false
    }
}

#[test]
fn native_observation_without_compat_harvests_completed_inspection() {
    struct Observer;
    impl script::native::Script for Observer {
        fn tick(
            &mut self,
            _: &mut script::native::NativeTick<'_>,
        ) -> Result<script::native::ScriptFlow, script::native::ScriptFailure> {
            Ok(script::native::ScriptFlow::Continue)
        }
    }

    let world = Some(world_with(TransportGraph::default(), vec![]));
    let navs = Arc::new(Mutex::new(HashMap::new()));
    let scripts = Arc::new(Mutex::new(HashMap::new()));
    let slot = super::super::script_slot_or_insert(&scripts, "p");
    {
        let mut slot = slot.lock().unwrap();
        slot.start_test_script(Box::new(Observer), None)
            .expect("native observation slot starts");
        assert!(!slot.load_active(), "there is no active compat isolate");
    }
    let request_id = 94;
    queue_inspect(
        &navs,
        "p",
        &world,
        Some(WorldState::empty()),
        BankRows::default(),
        None,
        None,
        req(tile(0, 0, 0), tile(2, 2, 1), request_id),
    );
    wait_running(&navs, request_id);
    let ticket = navs.lock().unwrap()["p"]
        .inspect
        .active_ticket
        .expect("accepted inspection")
        .ticket();
    let service = WorkService::shared().expect("work service");
    WorkService::wait_terminal(&service, &ticket).expect("completed inspection");
    assert!(navs.lock().unwrap()["p"].inspect.latest.is_none());

    let mut driver = CaptureDriver::default();
    let cheats = Arc::new(Mutex::new(HashMap::new()));
    super::super::script_observe(
        &mut driver,
        "p",
        true,
        false,
        0,
        Some((0, 0, 0)),
        None,
        Some(WorldState::empty()),
        None,
        None,
        &scripts,
        &cheats,
        &navs,
        &world,
        false,
        false,
    );
    let all = navs.lock().unwrap();
    let terminal = all["p"]
        .inspect
        .latest
        .as_ref()
        .expect("a native non-tick observation harvests the completed inspection");
    assert_eq!(terminal.request_id, request_id);
    assert_eq!(terminal.seq, 1);
    assert_eq!(terminal.reason, "NoPath");
}

fn real_content_inspection_world() -> (Arc<NavWorld>, Arc<Cache>, Arc<ObjNames>) {
    const PACK: &str =
        "/srv/dev/scratch/274bot-evidence/NAV-SHANTAY-EDGE/nav/289/bake-r2-a.navpack";
    const CACHE: &str = "/srv/dev/scratch/274bot-evidence/NAV-SHANTAY-EDGE/cache-snapshots/revision-289/90f072568dab9327ad49aaecb804f8335d3803504ed56f72626b4b83fab84f47/37214163f1e6ceca4c349c15c867331510b9ccbb7e11f7ecbf434dfb331c9afb/37214163f1e6ceca";
    const CACHE_VERSION_HASH: &str = "37214163f1e6ceca";
    const CONFIG_SHA256: &str = "3eeb1085547fb3af66db55ec4368357dbbad5178b7747f8b7cb0339fdd04a4ce";
    const FLAGS_SHA256: &str = "67e4094dff06def5cf8abc172ce751f4ca8679532ba04c1ba15ab6bf668c7a4a";
    const PACK_CACHE_ID: &str = "c4d8ab36bcfd2a7907535b4f619e28623b0a22e98d496fd2a9620d544c5b5b09";
    use sha2::Digest as _;

    let pack = std::path::PathBuf::from(PACK);
    let pack_manifest: serde_json::Value = serde_json::from_slice(
        &std::fs::read(pack.with_extension("navpack.json")).expect("revision-289 pack manifest"),
    )
    .expect("valid pack manifest");
    assert_eq!(pack_manifest["revision"], 289);
    assert_eq!(pack_manifest["cache_id"], PACK_CACHE_ID);
    for extension in ["navflags", "navreach", "navcanlight", "navpois"] {
        assert!(
            pack.with_extension(extension).is_file(),
            "missing real-content sidecar .{extension}"
        );
    }

    let cache_dir = std::path::Path::new(CACHE);
    let versionlist =
        std::fs::read(cache_dir.join("versionlist")).expect("matching decoded versionlist");
    assert_eq!(
        client::unpack::version_hash(&versionlist),
        CACHE_VERSION_HASH
    );
    let config_bytes =
        std::fs::read(cache_dir.join("config")).expect("matching decoded config archive");
    let config_digest: [u8; 32] = sha2::Sha256::digest(&config_bytes).into();
    assert_eq!(nav::pack::sha256_hex(&config_digest), CONFIG_SHA256);
    let cache = Arc::new(Cache::unpack(&client::io::JagFile::new(config_bytes)));
    let obj_names = names(&cache);

    let mut loaded_world = NavWorld::load_pack(&pack).expect("real revision-289 navpack");
    let flags = nav::pack::read_flags_sidecar(&pack.with_extension("navflags"), true)
        .expect("matching raw flags sidecar");
    assert_eq!(flags.content_sha256.as_deref(), Some(FLAGS_SHA256));
    assert_eq!(flags.origin, loaded_world.collision.origin);
    assert_eq!(
        (flags.width, flags.height),
        (loaded_world.collision.width, loaded_world.collision.height)
    );
    loaded_world
        .collision
        .attach_flags(Arc::try_unwrap(flags.flags).expect("sole real flags owner"));
    (Arc::new(loaded_world), cache, obj_names)
}

/// Deterministic old/new inspection handoff over the pinned real revision-289
/// content. Optionally writes the exact snapshot when
/// `T5W_INSPECT_CAPTURE_OUT=<path>` is set.
#[test]
fn real_content_inspection_capture_bytes() {
    use crate::script_runtime::SettledStart as _;
    let output = std::env::var_os("T5W_INSPECT_CAPTURE_OUT").map(std::path::PathBuf::from);
    const OLD_CAPTURE: &str =
        "/srv/dev/scratch/274bot-evidence/TRACK-T5/T5W/inspection-old-capture-r5/inspect.fb";
    const REQUEST_ID: u64 = 2_890_001;
    let (world, cache, obj_names) = real_content_inspection_world();

    // The recorded Al Kharid toll Gate (loc 2882) consumes ten coins.
    // `calculate` is the retained old implementation/oracle; actual output
    // below goes through queue_inspect and the host observation pump.
    let mut gate_state = WorldState::empty().with_map_members(false);
    gate_state.inv.insert(995, 10);
    let (capture, expected, gate_name) = world
        .graph
        .edges
        .iter()
        .filter_map(|edge| {
            if edge.kind != TransportKind::Door || edge.loc_id != 2882 || !gate_state.allows(edge) {
                return None;
            }
            let loc = usize::try_from(edge.loc_id)
                .ok()
                .and_then(|id| cache.locs.get(id))?;
            if !loc.name.to_ascii_lowercase().contains("gate") {
                return None;
            }
            let from = edge.takeoff.unwrap_or(edge.at);
            let to = edge.to;
            if (i64::from(from.x) - 3268).abs() > 1
                || (i64::from(from.z) - 3227).abs() > 1
                || from == to
                || !world.collision.standable(from)
                || !world.collision.standable(to)
            {
                return None;
            }
            let capture = InspectCapture {
                request_id: REQUEST_ID,
                generation: 0,
                revision: api::work::InputRevision::FIRST,
                lease: nav::producer::lease(&world).expect("test world has a nav lease"),
                world: Arc::clone(&world),
                state: Arc::new(gate_state.clone()),
                bank_origin: Origin::Unknown,
                bank: Vec::new(),
                from,
                to,
                opts: FindOptions {
                    allow_wilderness: true,
                    ..FindOptions::default()
                },
                avoid: Vec::new(),
                cache: Some(Arc::clone(&cache)),
                obj_names: Some(Arc::clone(&obj_names)),
                search_budget_strict: None,
                search_budget_stand: None,
                search_budget_post: None,
            };
            let expected = calculate(&capture);
            (expected.ok
                && expected
                    .hops
                    .iter()
                    .any(|hop| hop.loc_id == edge.loc_id && !hop.loc_name.is_empty()))
            .then_some((capture, expected, loc.name.clone()))
        })
        .next()
        .expect("real Al Kharid toll Gate is routable with its ten-coin fee");

    let navs = Arc::new(Mutex::new(HashMap::new()));
    let scripts = Arc::new(Mutex::new(HashMap::new()));
    let script_slot = super::super::script_slot_or_insert(&scripts, "p");
    script_slot
        .lock()
        .unwrap()
        .start_load_settled(
            "export const apiVersion = 2; export function tick() {}".to_string(),
            script::LoadShape::NativeTick,
            vec![],
        )
        .expect("native observation slot starts");
    let cheats = Arc::new(Mutex::new(HashMap::new()));
    let world_arg = Some(Arc::clone(&world));
    queue_inspect(
        &navs,
        "p",
        &world_arg,
        Some((*capture.state).clone()),
        BankRows::default(),
        Some(Arc::clone(&cache)),
        Some(Arc::clone(&obj_names)),
        req(capture.from, capture.to, REQUEST_ID),
    );

    let mut driver = CaptureDriver::default();
    let inv: [(i32, i32); 0] = [];
    let start = std::time::Instant::now();
    let mut tick = 1u64;
    loop {
        super::super::script_observe(
            &mut driver,
            "p",
            true,
            true,
            tick,
            Some((capture.from.x, capture.from.z, capture.from.level)),
            Some(&inv),
            Some((*capture.state).clone()),
            None,
            Some(obj_names.as_ref()),
            &scripts,
            &cheats,
            &navs,
            &world_arg,
            false,
            false,
        );
        let published = navs.lock().unwrap().get("p").is_some_and(|bot| {
            bot.inspect
                .latest
                .as_ref()
                .is_some_and(|terminal| terminal.request_id == REQUEST_ID)
        });
        if published {
            break;
        }
        assert!(
            start.elapsed() < Duration::from_secs(120),
            "real-content inspection request did not publish"
        );
        tick = tick.checked_add(1).expect("fixture tick does not wrap");
        thread::yield_now();
    }

    let actual = navs
        .lock()
        .unwrap()
        .get("p")
        .and_then(|bot| bot.inspect.latest.clone())
        .expect("published success terminal");
    let mut expected = expected;
    expected.seq = actual.seq;
    assert_eq!(actual, expected, "old/new real-content terminal");

    let mut expected_nav = InspectNav::default();
    expected_nav.publish(expected.clone());
    let mut actual_nav = InspectNav::default();
    actual_nav.publish(actual.clone());
    assert_eq!(
        encode_inspect_bytes(&actual_nav, 289_000, false),
        encode_inspect_bytes(&expected_nav, 289_000, false),
        "old/new real-content snapshot serialization"
    );
    assert!(actual.ok);
    assert!(
        actual
            .hops
            .iter()
            .any(|hop| hop.loc_name == gate_name && !hop.action.is_empty()),
        "real Gate action is serialized from the matching decoded cache"
    );

    // Retain a meaningful nonempty reason alongside the successful hop so the
    // captured snapshot covers both terminal payloads and custody metadata.
    let missing_id = REQUEST_ID + 1;
    let missing_world: Option<Arc<NavWorld>> = None;
    queue_inspect(
        &navs,
        "p",
        &missing_world,
        None,
        BankRows::default(),
        None,
        None,
        req(capture.from, capture.to, missing_id),
    );
    let bytes = {
        let all = navs.lock().unwrap();
        let bot = all.get("p").expect("inspection bot");
        let latest = bot.inspect.latest.as_ref().expect("missing-graph terminal");
        let previous = bot
            .inspect
            .prev
            .as_ref()
            .expect("successful terminal remains held");
        assert_eq!(latest.request_id, missing_id);
        assert_eq!(latest.reason, "missing-graph");
        assert_eq!(previous.request_id, REQUEST_ID);
        assert!(previous.ok);
        encode_inspect_bytes(&bot.inspect, 289_001, false)
    };
    let mut expected_snapshot = InspectNav {
        accepted_id: missing_id,
        ..Default::default()
    };
    expected_snapshot.publish(expected.clone());
    expected_snapshot.publish(InspectTerminal::refusal(
        missing_id,
        expected.generation,
        "missing-graph",
    ));
    assert_eq!(
        bytes,
        encode_inspect_bytes(&expected_snapshot, 289_001, false),
        "old/new real-content snapshot bytes include the dynamic route and refusal"
    );
    let old_bytes = std::fs::read(OLD_CAPTURE).expect("immutable old inspection capture");
    assert_eq!(
        bytes, old_bytes,
        "candidate must preserve captured inspection bytes"
    );
    assert_eq!(bytes.len(), 1_376);
    let digest: [u8; 32] = sha2::Sha256::digest(&bytes).into();
    assert_eq!(
        nav::pack::sha256_hex(&digest),
        "886f2db744cd8f7f7b90004af412b6a24b0211b2fdd23a460ccc54eb531188ab",
        "real-content inspection wire must match the old captured bytes"
    );
    if let Some(output) = output {
        std::fs::write(&output, &bytes).expect("write exact inspection snapshot bytes");
        eprintln!(
            "T5W real-content inspection capture: gate={gate_name:?} from={:?} to={:?} output={} bytes={}",
            capture.from,
            capture.to,
            output.display(),
            bytes.len()
        );
    } else {
        eprintln!(
            "T5W real-content inspection capture: gate={gate_name:?} from={:?} to={:?} bytes={}",
            capture.from,
            capture.to,
            bytes.len()
        );
    }
}
const MAX_INSPECT_CORPUS_CASES: usize = 128;
const MIN_INSPECT_CORPUS_CASES: usize = 100;
const MAX_INSPECT_CORPUS_LINE_BYTES: usize = 16 * 1024;
const MAX_INSPECT_CORPUS_FILE_BYTES: usize =
    MAX_INSPECT_CORPUS_CASES * MAX_INSPECT_CORPUS_LINE_BYTES;
const MAX_INSPECT_STEPS_PER_CASE: usize = 500_000;

#[derive(Debug, Default, serde::Deserialize)]
#[serde(default, deny_unknown_fields)]
struct InspectCorpusCase {
    from: [i32; 3],
    to: [i32; 3],
    allow_teleports: bool,
    #[serde(default = "inspect_corpus_allow_wilderness")]
    allow_wilderness: bool,
    allow_bank_fetch: bool,
    all_zones: bool,
    zones: Vec<String>,
    avoid: Vec<InspectCorpusAvoid>,
    inventory: Vec<[i32; 2]>,
    worn: Vec<i32>,
    stats: Vec<[i32; 2]>,
    combat_level: Option<i32>,
    varps: Vec<[i32; 2]>,
    quests: Vec<String>,
    map_members: bool,
    allow_fetchable_worn: bool,
    bank_origin: InspectCorpusBankOrigin,
    bank_rows: Vec<[i32; 2]>,
    quest_evidence_present: bool,
}

#[derive(Debug, Default, serde::Deserialize)]
#[serde(default, deny_unknown_fields)]
struct InspectCorpusAvoid {
    min_x: i32,
    max_x: i32,
    min_z: i32,
    max_z: i32,
    level: Option<i32>,
}

#[derive(Clone, Copy, Debug, Default, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
enum InspectCorpusBankOrigin {
    #[default]
    Unknown,
    Hint,
    Session,
}

fn inspect_corpus_allow_wilderness() -> bool {
    true
}

fn inspect_corpus_capture(
    case: InspectCorpusCase,
    world: Arc<NavWorld>,
    cache: Arc<Cache>,
    obj_names: Arc<ObjNames>,
    request_id: u64,
) -> InspectCapture {
    assert!(
        !case.quest_evidence_present,
        "the fixture cannot synthesize a caller-owned quest evidence provider"
    );
    assert!(
        !case.all_zones || case.zones.is_empty(),
        "all_zones cannot combine with named zone exemptions"
    );
    assert!(case.zones.len() <= nav::zones::MAX_ZONE_KEYS);
    assert!(case.avoid.len() <= MAX_AVOID);
    assert!(case.bank_rows.len() <= api::bank_memory::MAX_HINT_ROWS);

    let zones = if case.all_zones {
        nav::zones::ZoneExempt::all()
    } else if case.zones.is_empty() {
        nav::zones::ZoneExempt::NONE
    } else {
        let table = world
            .graph
            .zones
            .as_ref()
            .expect("named exemptions require the real zone table");
        let keys = case
            .zones
            .iter()
            .map(|id| {
                table
                    .resolve(id)
                    .unwrap_or_else(|| panic!("unknown zone id {id:?}"))
            })
            .collect::<Vec<_>>();
        nav::zones::ZoneExempt::named(&keys).expect("bounded zone exemptions")
    };
    let options = FindOptions {
        allow_teleports: case.allow_teleports,
        allow_wilderness: case.allow_wilderness,
        allow_bank_fetch: case.allow_bank_fetch,
        essence: None,
        zones,
    };
    let bank_origin = match case.bank_origin {
        InspectCorpusBankOrigin::Unknown => Origin::Unknown,
        InspectCorpusBankOrigin::Hint => Origin::Hint,
        InspectCorpusBankOrigin::Session => Origin::Session,
    };
    let bank = BankRows {
        origin: bank_origin,
        rows: case
            .bank_rows
            .into_iter()
            .map(|[item, count]| (item, count))
            .collect(),
    };
    let mut state = WorldState::empty();
    state.inv.extend(
        case.inventory
            .into_iter()
            .map(|[item, count]| (item, count)),
    );
    state.worn.extend(case.worn);
    state
        .stats
        .extend(case.stats.into_iter().map(|[skill, level]| (skill, level)));
    state.combat_level = case.combat_level;
    state
        .varps
        .extend(case.varps.into_iter().map(|[index, value]| (index, value)));
    state.quests.extend(case.quests);
    state.map_members = case.map_members;
    state.allow_fetchable_worn = case.allow_fetchable_worn;

    let mut capture = capture(
        Arc::clone(&world),
        tile(case.from[0], case.from[1], case.from[2]),
        tile(case.to[0], case.to[1], case.to[2]),
        options,
        request_id,
        bank,
        None,
        None,
        None,
    );
    capture.state = Arc::new(state);
    capture.avoid = case
        .avoid
        .into_iter()
        .map(|rect| AvoidRect {
            min_x: rect.min_x,
            max_x: rect.max_x,
            min_z: rect.min_z,
            max_z: rect.max_z,
            level: rect.level,
        })
        .collect();
    assert!(
        validate_request(capture.from, capture.to, &capture.avoid).is_ok(),
        "corpus rows must represent accepted inspection requests"
    );
    capture.cache = Some(cache);
    capture.obj_names = Some(obj_names);
    capture
}

fn measured_inspection_peak(bytes_max: u64) -> usize {
    usize::try_from(bytes_max)
        .expect("allocation peak fits address space")
        .checked_add(std::mem::size_of::<
            api::work::JobSettlement<InspectWorkFamily>,
        >())
        .expect("measured settlement peak does not overflow")
}

fn drive_inspection_job(job: &mut InspectJob) -> (InspectJobOutput, u64, Duration) {
    let cancellation = api::work::SharedCancellation::new();
    let clock = api::work::SystemClock;
    let units = std::num::NonZeroU32::new(64).expect("nonzero inspection slice");
    let mut total_units = 0u64;
    let mut largest_slice = Duration::ZERO;
    for slice in 0..MAX_INSPECT_STEPS_PER_CASE {
        let deadline =
            api::identity::Deadline::after(std::time::Instant::now(), Duration::from_millis(10));
        let budget = api::work::SliceBudget::new(units, deadline);
        let mut context = api::work::StepContext::<api::work::InspectionPhase>::new(
            &clock,
            budget,
            &cancellation,
        );
        let started = Instant::now();
        let outcome = api::work::CooperativeJob::step(job, &mut context);
        largest_slice = largest_slice.max(started.elapsed());
        total_units = total_units
            .checked_add(u64::from(context.units_this_slice()))
            .expect("inspection work-unit counter does not overflow");
        match outcome {
            api::work::StepOutcome::Yielded => {}
            api::work::StepOutcome::Completed(output) => {
                return (output, total_units, largest_slice);
            }
            api::work::StepOutcome::Failed(error) => {
                panic!("inspection job failed at slice {slice}: {error:?}")
            }
        }
    }
    panic!("inspection job exceeded its finite per-case slice bound");
}

#[test]
#[ignore = "requires immutable real revision-289 navigation and cache inputs"]
fn real_content_inspection_whole_job_memory() {
    let (world, cache, obj_names) = real_content_inspection_world();
    let mut charge_bytes = 0;
    let mut result = None;
    let measured = allocation_counter::measure(|| {
        let case = InspectCorpusCase {
            from: [3080, 3472, 0],
            to: [3222, 3218, 0],
            map_members: true,
            stats: (0..=20).map(|skill| [skill, 99]).collect(),
            combat_level: Some(126),
            ..InspectCorpusCase::default()
        };
        let capture = inspect_corpus_capture(
            case,
            Arc::clone(&world),
            Arc::clone(&cache),
            Arc::clone(&obj_names),
            2_890_098,
        );
        let (submission, preflights) = InspectSubmission::for_capture("test", &capture)
            .expect("real whole-job inspection charge");
        charge_bytes = InspectWorkFamily::charge(&submission)
            .total_bytes()
            .expect("checked whole-job charge");
        let mut job = InspectJob::new("test", capture, preflights);
        result = Some(drive_inspection_job(&mut job));
    });
    let (output, units, largest_slice) = result.expect("the real inspection completes");
    assert!(
        output.route.is_some(),
        "the real Edgeville route must be reachable"
    );
    let ledger = nav::producer::OwnerResultLedger::with_limits(
        nav::producer::OWNER_RESULT_CAPACITY_BYTES,
        nav::producer::OWNER_RESULT_MAX_BYTES,
    );
    let result_bytes = match reserve_inspect_owner_result(&output, &ledger) {
        Ok(reservation) => Some(reservation.bytes()),
        Err(nav::producer::OwnerResultRefusal::TooLarge { requested, maximum }) => {
            assert_eq!(maximum, nav::producer::OWNER_RESULT_MAX_BYTES);
            assert!(requested > maximum);
            None
        }
        Err(_) => panic!("real owner-result reservation failed unexpectedly"),
    };
    let measured_peak = measured_inspection_peak(measured.bytes_max);
    eprintln!(
        "inspection_whole_job peak_bytes={measured_peak} charge_bytes={charge_bytes} \
         result_bytes={result_bytes:?} work_units={units} largest_slice_ns={}",
        largest_slice.as_nanos(),
    );
    assert!(charge_bytes <= MAX_INSPECT_CHARGE);
    assert!(
        measured_peak <= charge_bytes,
        "whole-job allocations exceed the declared charge: {measured:?}"
    );
    if let Some(result_bytes) = result_bytes {
        assert!(result_bytes <= nav::producer::OWNER_RESULT_MAX_BYTES);
    }
}

#[test]
#[ignore = "requires immutable real revision-289 navigation and cache inputs"]
fn real_content_inspection_bank_fetch_whole_job_memory() {
    let (world, cache, obj_names) = real_content_inspection_world();
    let from = tile(3202, 9909, 0);
    let to = tile(3209, 9891, 0);
    let mut state = WorldState::empty().with_map_members(true);
    state.stats = (0..25).map(|skill| (skill, 99)).collect();
    state.combat_level = Some(126);
    let options = FindOptions {
        allow_teleports: false,
        allow_wilderness: true,
        allow_bank_fetch: true,
        ..FindOptions::default()
    };
    assert!(
        world
            .graph
            .edges
            .iter()
            .any(|edge| { edge.loc_id == 733 && edge.item_req.contains(&(946, 1)) }),
        "the real sewer web retains its knife gate"
    );
    assert_eq!(
        find_with_avoid(
            &world.collision,
            &world.graph,
            from,
            to,
            options,
            &state,
            &[]
        )
        .map(|_| ()),
        Err(RouteError::NoPath),
        "the unseeded sewer route must require banked equipment",
    );
    state.inv.insert(946, 1);
    find_with_avoid(
        &world.collision,
        &world.graph,
        from,
        to,
        options,
        &state,
        &[],
    )
    .expect("the carried knife opens the real sewer web");

    for (index, bank_origin) in [
        InspectCorpusBankOrigin::Hint,
        InspectCorpusBankOrigin::Session,
    ]
    .into_iter()
    .enumerate()
    {
        let mut charge_bytes = 0;
        let mut result = None;
        let measured = allocation_counter::measure(|| {
            let case = InspectCorpusCase {
                from: [from.x, from.z, from.level],
                to: [to.x, to.z, to.level],
                allow_bank_fetch: true,
                allow_teleports: false,
                allow_wilderness: true,
                bank_origin,
                bank_rows: vec![[946, 1]],
                stats: (0..25).map(|skill| [skill, 99]).collect(),
                combat_level: Some(126),
                map_members: true,
                ..InspectCorpusCase::default()
            };
            let capture = inspect_corpus_capture(
                case,
                Arc::clone(&world),
                Arc::clone(&cache),
                Arc::clone(&obj_names),
                2_890_110 + u64::try_from(index).expect("two bank-origin cases"),
            );
            let (submission, preflights) =
                InspectSubmission::for_capture("test", &capture).expect("real bank-fetch charge");
            let missing = preflights
                .missing
                .as_ref()
                .expect("missing-item reservation");
            let reservation = missing.reservation();
            assert_eq!(reservation.max_missing_carry_ids, 221);
            assert_eq!(
                reservation
                    .max_plan_item_rows
                    .checked_sub(reservation.max_missing_carry_ids),
                Some(3_603),
                "the production effective bound includes every loaded worn row",
            );
            assert_eq!(reservation.max_plan_item_rows, 3_824);
            assert_eq!(strict_budget(&capture), nav::router::NODE_BUDGET);
            assert_eq!(stand_budget(&capture), nav::router::NODE_BUDGET);
            assert_eq!(post_budget(&capture), nav::router::NODE_BUDGET);
            charge_bytes = InspectWorkFamily::charge(&submission)
                .total_bytes()
                .expect("checked real bank-fetch charge");
            let mut job = InspectJob::new("test", capture, preflights);
            result = Some(drive_inspection_job(&mut job));
        });
        let (output, units, largest_slice) = result.expect("real bank-fetch inspection completes");
        assert!(
            output.ok,
            "real sewer-web bank route: {}",
            output.reason.as_str()
        );
        assert!(
            output.bank_planned,
            "real sewer-web plan uses the banked knife"
        );
        assert!(output.route.is_some(), "real sewer-web route is retained");
        let ledger = nav::producer::OwnerResultLedger::with_limits(
            nav::producer::OWNER_RESULT_CAPACITY_BYTES,
            nav::producer::OWNER_RESULT_MAX_BYTES,
        );
        let result_bytes = match reserve_inspect_owner_result(&output, &ledger) {
            Ok(reservation) => Some(reservation.bytes()),
            Err(nav::producer::OwnerResultRefusal::TooLarge { requested, maximum }) => {
                assert_eq!(maximum, nav::producer::OWNER_RESULT_MAX_BYTES);
                assert!(requested > maximum);
                None
            }
            Err(_) => panic!("real bank owner-result reservation failed unexpectedly"),
        };
        let measured_peak = measured_inspection_peak(measured.bytes_max);
        eprintln!(
            "inspection_real_bank_whole_job origin={bank_origin:?} peak_bytes={measured_peak} \
             charge_bytes={charge_bytes} result_bytes={result_bytes:?} work_units={units} \
             largest_slice_ns={}",
            largest_slice.as_nanos(),
        );
        assert!(charge_bytes <= MAX_INSPECT_CHARGE);
        assert!(
            measured_peak <= charge_bytes,
            "real bank-fetch allocations exceed the declared charge: {measured:?}"
        );
        if let Some(result_bytes) = result_bytes {
            assert!(result_bytes <= nav::producer::OWNER_RESULT_MAX_BYTES);
        }
    }
}

/// Measure one full-budget exhaustion against an open real-shaped collision
/// domain. This test allocates a bounded 2001 × 2001 grid and runs 4,000,000
/// settled transitions. It is ignored in the normal suite.
#[test]
#[ignore = "bounded 4,000,000-settle inspection allocation qualification"]
fn inspection_budget_hit_allocation_stays_within_charge() {
    const WIDTH: usize = 2001;
    const HEIGHT: usize = 2001;
    let world = Arc::new(NavWorld::from_parts(
        open_world(WIDTH, HEIGHT),
        TransportGraph::default(),
        vec![],
    ));
    let capture = capture(
        world,
        tile(0, 0, 0),
        tile(WIDTH as i32, HEIGHT as i32, 0),
        FindOptions::default(),
        2_890_099,
        BankRows::default(),
        None,
        None,
        None,
    );
    let (submission, preflights) =
        InspectSubmission::for_capture("test", &capture).expect("full-budget inspection charge");
    let charge = InspectWorkFamily::charge(&submission);
    let mut job = InspectJob::new("test", capture, preflights);
    let mut output = None;
    let measured = allocation_counter::measure(|| {
        output = Some(drive_inspection_job(&mut job));
    });
    let (output, units, largest_slice) = output.expect("bounded driver completes the inspection");
    assert_eq!(output.reason.as_str(), "BudgetExhausted");
    assert!(output.route.is_none());
    assert!(
        units >= 4_000_000,
        "the search must consume the full budget"
    );
    let measured_peak = measured_inspection_peak(measured.bytes_max);
    let declared_peak = charge
        .input_bytes()
        .checked_add(measured_peak)
        .expect("measured inspection peak does not overflow");
    let declared_total = charge.total_bytes().expect("checked work charge");
    assert!(
        declared_peak <= declared_total,
        "measured inspection peak {declared_peak} exceeds declared charge {declared_total}"
    );
    eprintln!(
        "inspection budget hit: work_units={units} allocations={} measured_peak={} input={} declared_total={declared_total} largest_slice_ns={}",
        measured.count_total,
        measured.bytes_max,
        charge.input_bytes(),
        largest_slice.as_nanos(),
    );
}

/// Measure the owner-ledger bytes for 100–128 real revision-289 inspection
/// requests. Supply `T5W_INSPECT_RESULT_CORPUS` as JSONL, one capture per line.
/// Each row has `from` and `to` `[x,z,level]` arrays, plus optional
/// `allow_teleports`, `allow_wilderness`, `allow_bank_fetch`, `all_zones`,
/// `zones`, `avoid`, state fields, `bank_origin`, and `bank_rows`.
/// Stage-gated rows are not supported because they need a caller-owned
/// `QuestEvidence` provider.
#[test]
#[ignore = "requires a bounded real revision-289 inspection request corpus"]
fn real_content_inspection_result_distribution() {
    let path = std::env::var_os("T5W_INSPECT_RESULT_CORPUS")
        .map(std::path::PathBuf::from)
        .expect("set T5W_INSPECT_RESULT_CORPUS to real inspection captures");
    use std::io::Read as _;

    let metadata = std::fs::metadata(&path).expect("inspect corpus metadata");
    assert!(
        metadata.is_file(),
        "inspection corpus must be a regular file"
    );
    assert!(
        metadata.len() <= MAX_INSPECT_CORPUS_FILE_BYTES as u64,
        "inspection corpus exceeds its two-MiB file bound"
    );
    let mut bytes = Vec::new();
    std::fs::File::open(&path)
        .expect("open inspection corpus")
        .take(MAX_INSPECT_CORPUS_FILE_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .expect("read bounded inspection corpus");
    assert!(
        bytes.len() <= MAX_INSPECT_CORPUS_FILE_BYTES,
        "inspection corpus exceeds its two-MiB file bound"
    );
    let text = std::str::from_utf8(&bytes).expect("inspection corpus is UTF-8 JSONL");
    let mut cases = Vec::with_capacity(MAX_INSPECT_CORPUS_CASES);
    for (line_index, line) in text.lines().enumerate() {
        if line.is_empty() {
            continue;
        }
        assert!(
            line.len() <= MAX_INSPECT_CORPUS_LINE_BYTES,
            "inspection corpus line {} exceeds 16 KiB",
            line_index + 1
        );
        assert!(
            cases.len() < MAX_INSPECT_CORPUS_CASES,
            "inspection corpus exceeds {} requests",
            MAX_INSPECT_CORPUS_CASES
        );
        let case = serde_json::from_str::<InspectCorpusCase>(line).unwrap_or_else(|error| {
            panic!("invalid inspection corpus line {}: {error}", line_index + 1)
        });
        cases.push(case);
    }
    assert!(
        (MIN_INSPECT_CORPUS_CASES..=MAX_INSPECT_CORPUS_CASES).contains(&cases.len()),
        "inspection corpus needs 100–128 requests for a useful p99"
    );

    let (world, cache, obj_names) = real_content_inspection_world();
    let ledger = nav::producer::OwnerResultLedger::with_limits(
        nav::producer::OWNER_RESULT_CAPACITY_BYTES,
        nav::producer::OWNER_RESULT_MAX_BYTES,
    );
    let mut sizes = Vec::with_capacity(cases.len());
    let mut too_large_results = 0usize;
    for (index, case) in cases.into_iter().enumerate() {
        let request_id = 2_890_100 + u64::try_from(index).expect("bounded case index");
        let capture = inspect_corpus_capture(
            case,
            Arc::clone(&world),
            Arc::clone(&cache),
            Arc::clone(&obj_names),
            request_id,
        );
        let (submission, preflights) =
            InspectSubmission::for_capture("test", &capture).expect("admissible corpus charge");
        let charge = InspectWorkFamily::charge(&submission);
        let mut job = InspectJob::new("test", capture, preflights);
        let mut output = None;
        let measured = allocation_counter::measure(|| {
            output = Some(drive_inspection_job(&mut job));
        });
        let (output, work_units, largest_slice) = output.expect("bounded inspection completes");
        let measured_peak = measured_inspection_peak(measured.bytes_max);
        let declared_peak = charge
            .input_bytes()
            .checked_add(measured_peak)
            .expect("measured inspection peak does not overflow");
        let declared_total = charge.total_bytes().expect("checked work charge");
        assert!(
            declared_peak <= declared_total,
            "request {request_id} measured {declared_peak} bytes beyond charge {declared_total}"
        );
        match reserve_inspect_owner_result(&output, &ledger) {
            Ok(reservation) => {
                let result_bytes = reservation.bytes();
                assert!(result_bytes <= nav::producer::OWNER_RESULT_MAX_BYTES);
                assert_eq!(ledger.used_bytes(), result_bytes);
                drop(reservation);
                assert_eq!(ledger.used_bytes(), 0);
                sizes.push(result_bytes);
                eprintln!(
                    "inspection_result_case index={index} result_bytes={result_bytes} \
                     execution_plus_input_peak={declared_peak} charge_bytes={declared_total} \
                     work_units={work_units} largest_slice_ns={}",
                    largest_slice.as_nanos(),
                );
            }
            Err(nav::producer::OwnerResultRefusal::TooLarge { requested, maximum }) => {
                assert_eq!(maximum, nav::producer::OWNER_RESULT_MAX_BYTES);
                assert!(requested > maximum);
                assert_eq!(ledger.used_bytes(), 0);
                too_large_results += 1;
                eprintln!(
                    "inspection_result_case index={index} result=route-too-large \
                     requested_bytes={requested} maximum_bytes={maximum} \
                     execution_plus_input_peak={declared_peak} charge_bytes={declared_total} \
                     work_units={work_units} largest_slice_ns={}",
                    largest_slice.as_nanos(),
                );
            }
            Err(_) => panic!("unexpected owner-result refusal"),
        }
        drop(output);
    }

    assert!(
        !sizes.is_empty(),
        "at least one owner result fits the 64 MiB limit"
    );
    sizes.sort_unstable();
    let nearest_rank = |percent: usize| (sizes.len() * percent).div_ceil(100) - 1;
    let p50 = sizes[nearest_rank(50)];
    let p99 = sizes[nearest_rank(99)];
    let maximum = *sizes.last().expect("nonempty bounded corpus");
    eprintln!(
        "real revision-289 inspection result bytes: samples={} route_too_large={} \
         p50={} p99={} max={}",
        sizes.len(),
        too_large_results,
        p50,
        p99,
        maximum,
    );
}

#[test]
fn inspection_charge_accounts_for_each_bank_fetch_phase_overlap() {
    let from = tile(0, 0, 0);
    let to = tile(4, 0, 1);
    let mut graph = TransportGraph::default();
    graph.at.entry(from).or_default().push(0);
    graph
        .edges
        .push(edge(TransportKind::Boat, from, to, 381, vec![(995, 10)]));
    let world = zoned_line_world(
        graph,
        vec![BankStand {
            name: "Bank booth".into(),
            tile: tile(1, 0, 0),
            access: BankAccess::Booth { op: 2 },
        }],
    );
    let options = FindOptions {
        allow_bank_fetch: true,
        zones: nav::zones::ZoneExempt::named(&[nav::zones::ZoneKey::Zone(0)])
            .expect("one zone exemption"),
        ..FindOptions::default()
    };
    let capture = capture(
        world,
        from,
        to,
        options,
        2_890_200,
        BankRows {
            origin: Origin::Hint,
            rows: vec![(995, 1)],
        },
        None,
        None,
        None,
    );
    let (submission, preflights) = InspectSubmission::for_capture("test", &capture)
        .expect("all overlapping phases fit the bound");
    let strict = preflights.strict.as_ref().expect("strict phase");
    let missing = preflights.missing.as_ref().expect("missing-item phase");
    let stand = preflights.stand.as_ref().expect("stand phase");
    let stand_fallback = preflights
        .stand_fallback
        .as_ref()
        .expect("first-candidate diagnostic phase");
    let mut stand_options = capture.opts;
    stand_options.allow_bank_fetch = false;
    stand_options.prefer_target_order = true;
    let stand_rows = capture.world.banks().len();
    let expected_stand = nav::router::SearchCursor::preflight(
        nav::router::SearchRequestShape::First {
            target_capacity: stand_rows.checked_mul(4).expect("stand goal bound"),
            fallback_capacity: 0,
            options: stand_options,
            avoid_capacity: capture.avoid.len(),
            max_settles: stand_budget(&capture),
        },
        &capture.lease,
        &capture.state,
    )
    .expect("target-ordered bank access preflight");
    assert_eq!(*stand, expected_stand);
    let expected_stand_fallback = nav::router::SearchCursor::preflight(
        nav::router::SearchRequestShape::Single {
            options: stand_options,
            avoid_capacity: capture.avoid.len(),
            max_settles: stand_budget(&capture),
        },
        &capture.lease,
        &capture.state,
    )
    .expect("first-candidate diagnostic preflight");
    assert_eq!(*stand_fallback, expected_stand_fallback);
    let post = preflights.post.as_ref().expect("post-route phase");
    let blockers_with_bank = preflights
        .blockers_with_bank
        .as_ref()
        .expect("bank blocker phase");
    let blockers_without_bank = preflights
        .blockers_without_bank
        .as_ref()
        .expect("post-bank blocker phase");
    let search_peak = |preflight: &nav::router::SearchPreflight| {
        preflight
            .input_bytes()
            .checked_add(preflight.reservation().future_layout_bytes)
            .and_then(|bytes| bytes.checked_add(preflight.reservation().peak_bytes))
            .expect("search input, future and peak")
    };
    let strict_peak = search_peak(strict);
    let missing_peak = search_peak(missing);
    let stand_peak = search_peak(stand);
    let stand_fallback_peak = search_peak(stand_fallback);
    let post_peak = search_peak(post);
    let blocker_with_peak = search_peak(blockers_with_bank);
    let blocker_without_peak = search_peak(blockers_without_bank);
    let missing_reservation = missing.reservation();
    let missing_count = missing_reservation.max_missing_requests;
    assert!(missing_count > 0, "fixture has one missing-item transport");
    let plan_item_rows = missing_reservation.max_plan_item_rows;
    let carry_plus_worn = missing_reservation
        .max_missing_carry_ids
        .checked_add(missing_reservation.max_missing_worn_ids)
        .expect("per-lease plan item count");
    assert_eq!(
        plan_item_rows,
        carry_plus_worn.min(missing_count),
        "planner rows use U_eff, not the full missing-diagnosis count"
    );
    assert_eq!(plan_item_rows, preflights.max_plan_item_rows);
    assert!(missing_count >= nav::router::NODE_BUDGET);
    assert!(
        missing_count > plan_item_rows,
        "M remains the diagnosis bound rather than the planner row count"
    );
    assert_eq!(strict_budget(&capture), nav::router::NODE_BUDGET);
    assert_eq!(stand_budget(&capture), nav::router::NODE_BUDGET);
    assert_eq!(post_budget(&capture), nav::router::NODE_BUDGET);

    let missing_result = search_result_payload_bytes(missing_reservation.result_bytes)
        .expect("retained missing search output");

    let planning_peak =
        PlanningRowsCursor::heap_bound(capture.bank.len(), plan_item_rows).expect("planning peak");
    let planning_rows = PlanningRowsCursor::rows_heap_bound(capture.bank.len(), plan_item_rows)
        .expect("planning rows");
    let bank_rows = capture
        .bank
        .len()
        .checked_add(plan_item_rows)
        .expect("hint rows plus U_eff");
    assert_eq!(
        bank_rows,
        capture.bank.len() + plan_item_rows,
        "captured bank rows remain independent of U_eff"
    );
    let bank_cursor =
        BankFetchCursor::heap_bound(bank_rows, plan_item_rows, stand_rows).expect("bank cursor");
    let bank_plan = BankFetchPlan::heap_bound(plan_item_rows).expect("bank plan");
    let wear_choices = plan_item_rows.min(missing_reservation.max_missing_worn_ids);
    let inventory_new = plan_item_rows;
    let growth = capture
        .state
        .growth_bound(inventory_new, wear_choices)
        .expect("bounded bank-state growth");
    let state_growth = growth
        .inventory_new_bytes
        .checked_add(growth.worn_new_bytes)
        .expect("bounded state growth");
    assert!(state_growth > 0, "the bank plan grows the captured state");

    let blocker_result = search_result_payload_bytes(blockers_with_bank.reservation().result_bytes)
        .expect("bank blocker result")
        .max(
            search_result_payload_bytes(blockers_without_bank.reservation().result_bytes)
                .expect("post-bank blocker result"),
        );
    let detail_bytes = capture
        .world
        .graph
        .zones
        .as_ref()
        .expect("zone detail table")
        .blocking_detail_bytes_bound();
    let phase_a = strict_peak
        .max(missing_peak)
        .max(blocker_with_peak.max(blocker_without_peak) + state_growth)
        .max(blocker_result + detail_bytes + state_growth);
    let phase_b = (planning_peak + missing_result)
        .max(planning_rows + bank_cursor + missing_result)
        .max(bank_plan + state_growth);
    let phase_c = (stand_peak.max(stand_fallback_peak) + bank_cursor).max(post_peak + state_growth);
    let expected_phase_peak = phase_a.max(phase_b).max(phase_c);
    assert_eq!(submission.phase_peak_bytes, expected_phase_peak);

    let route_payload = search_result_payload_bytes(strict.reservation().result_bytes)
        .expect("strict route result")
        .max(
            search_result_payload_bytes(post.reservation().result_bytes)
                .expect("post route result"),
        );
    let expected_result = std::mem::size_of::<api::work::JobSettlement<InspectWorkFamily>>()
        + route_payload.max(detail_bytes);
    assert_eq!(submission.result_bytes, expected_result);
    let charge = InspectWorkFamily::charge(&submission);
    assert_eq!(
        charge.total_bytes(),
        Some(submission.input_bytes + expected_phase_peak.max(expected_result))
    );
    assert!(charge
        .total_bytes()
        .is_some_and(|bytes| bytes <= MAX_INSPECT_CHARGE));
}
