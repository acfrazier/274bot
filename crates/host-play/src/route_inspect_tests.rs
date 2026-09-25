    use super::*;
    use api::snapshot::WorldTile;
    use client::config::{Cache, LocType, NpcType, ObjType};
    use nav::collision::{pack_walk, WorldCollision};
    use nav::pack::{BankAccess, BankStand};
    use nav::transport::{TransportEdge, TransportGraph};
    use nav::world::NavWorld;
    use std::collections::HashMap;
    use std::time::Duration;

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
        item_req: Vec<(i32, i32)>,
    ) -> TransportEdge {
        TransportEdge {
            kind,
            at,
            to,
            loc_id,
            option: 1,
            ticks: 2,
            dir: None,
            open_loc_id: None,
            skill_req: vec![],
            item_req,
            quest_req: vec![],
            varp_req: vec![],
            worn_req: vec![],
            members_req: false,
            wildy_cap: None,
        }
    }

    fn world_with(graph: TransportGraph, banks: Vec<BankStand>) -> Arc<NavWorld> {
        Arc::new(NavWorld::from_parts(open_world(8, 8), graph, banks))
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
            avoid: Vec::new(),
            request_id,
            invalid_args: false,
        }
    }

    fn wait_latest(navs: &Arc<Mutex<HashMap<String, NavBot>>>, id: u64) -> InspectTerminal {
        let start = std::time::Instant::now();
        loop {
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
                start.elapsed() < Duration::from_secs(2),
                "inspect {id} did not publish"
            );
            thread::yield_now();
        }
    }

    fn wait_idle(navs: &Arc<Mutex<HashMap<String, NavBot>>>) {
        let start = std::time::Instant::now();
        loop {
            {
                let all = navs.lock().unwrap();
                if let Some(bot) = all.get("p") {
                    if !bot.inspect.executing
                        && bot.inspect.pending.is_none()
                        && bot.inspect.worker.is_none()
                    {
                        return;
                    }
                }
            }
            assert!(
                start.elapsed() < Duration::from_secs(2),
                "inspect worker did not idle"
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
                vec![],
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
    fn bank_false_item_gate_is_nopath() {
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
            world,
            state: WorldState::empty(),
            bank: vec![],
            from: at,
            to,
            opts: FindOptions {
                allow_wilderness: true,
                ..FindOptions::default()
            },
            avoid: vec![],
            cache: None,
            obj_names: None,
            test_barrier: None,
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
            world,
            state: WorldState::empty(),
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
            test_barrier: None,
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
            world,
            state: WorldState::empty(),
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
            test_barrier: None,
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
        assert_eq!(term.hops[0].kind, "boat");
        assert!(term.ticks > 0.0);
    }

    #[test]
    fn delayed_calculate_barrier_is_nonblocking() {
        clear_barrier();
        let barrier = InspectBarrier::new();
        install_barrier(Arc::clone(&barrier));
        let world = world_with(TransportGraph::default(), vec![]);
        let navs = Arc::new(Mutex::new(HashMap::new()));
        queue_inspect(
            &navs,
            "p",
            &Some(world),
            Some(WorldState::empty()),
            vec![],
            None,
            None,
            req(tile(0, 0, 0), tile(2, 2, 1), 11),
        );
        barrier.wait_entered();
        {
            let all = navs.lock().unwrap();
            let bot = all.get("p").unwrap();
            assert!(bot.inspect.worker.is_some());
            assert_eq!(bot.inspect.running_id, 11);
        }
        barrier.release();
        let term = wait_latest(&navs, 11);
        assert!(!term.ok);
        clear_barrier();
    }

    #[test]
    fn reset_keeps_inspect_worker() {
        clear_barrier();
        let barrier = InspectBarrier::new();
        install_barrier(Arc::clone(&barrier));
        let world = world_with(TransportGraph::default(), vec![]);
        let navs = Arc::new(Mutex::new(HashMap::new()));
        queue_inspect(
            &navs,
            "p",
            &Some(Arc::clone(&world)),
            Some(WorldState::empty()),
            vec![],
            None,
            None,
            req(tile(0, 0, 0), tile(2, 2, 0), 21),
        );
        barrier.wait_entered();
        {
            let mut all = navs.lock().unwrap();
            reset_inspect(all.get_mut("p").unwrap());
            let bot = all.get("p").unwrap();
            assert!(bot.inspect.worker.is_some(), "reset must leave the worker");
            assert!(bot.inspect.latest.is_none());
        }
        queue_inspect(
            &navs,
            "p",
            &Some(world),
            Some(WorldState::empty()),
            vec![],
            None,
            None,
            req(tile(0, 0, 0), tile(3, 3, 0), 22),
        );
        {
            let all = navs.lock().unwrap();
            let bot = all.get("p").unwrap();
            assert!(bot.inspect.worker.is_some());
            assert_eq!(bot.inspect.pending_id, 22);
        }
        barrier.release();
        let _ = wait_latest(&navs, 22);
        clear_barrier();
    }

    #[test]
    fn spawn_failed_publishes_and_frees_token() {
        force_next_spawn_fail();
        let world = world_with(TransportGraph::default(), vec![]);
        let navs = Arc::new(Mutex::new(HashMap::new()));
        queue_inspect(
            &navs,
            "p",
            &Some(world),
            Some(WorldState::empty()),
            vec![],
            None,
            None,
            req(tile(0, 0, 0), tile(1, 1, 0), 31),
        );
        let all = navs.lock().unwrap();
        let bot = all.get("p").unwrap();
        assert!(bot.inspect.worker.is_none());
        assert_eq!(bot.inspect.latest.as_ref().unwrap().reason, "spawn-failed");
        assert_eq!(bot.inspect.latest.as_ref().unwrap().request_id, 31);
    }

    #[test]
    fn pending_replace_publishes_stale_for_b() {
        clear_barrier();
        let barrier = InspectBarrier::new();
        install_barrier(Arc::clone(&barrier));
        let world = world_with(TransportGraph::default(), vec![]);
        let navs = Arc::new(Mutex::new(HashMap::new()));
        queue_inspect(
            &navs,
            "p",
            &Some(Arc::clone(&world)),
            Some(WorldState::empty()),
            vec![],
            None,
            None,
            req(tile(0, 0, 0), tile(1, 1, 0), 41),
        );
        barrier.wait_entered();
        queue_inspect(
            &navs,
            "p",
            &Some(Arc::clone(&world)),
            Some(WorldState::empty()),
            vec![],
            None,
            None,
            req(tile(0, 0, 0), tile(2, 2, 0), 42),
        );
        queue_inspect(
            &navs,
            "p",
            &Some(world),
            Some(WorldState::empty()),
            vec![],
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
        barrier.release();
        clear_barrier();
    }

    #[test]
    fn ring_holds_unacked_prev_until_apply_ack() {
        let mut nav = InspectNav::default();
        nav.publish(InspectTerminal::refusal(1, 0, "NoPath"));
        nav.publish(InspectTerminal::refusal(2, 0, "NoPath"));
        nav.publish(InspectTerminal::refusal(3, 0, "NoPath"));
        assert_eq!(nav.latest.as_ref().unwrap().request_id, 2);
        assert_eq!(nav.prev.as_ref().unwrap().request_id, 1);
        assert_eq!(nav.held.as_ref().unwrap().request_id, 3);
        nav.apply_ack(nav.prev.as_ref().unwrap().seq, 0);
        assert_eq!(nav.latest.as_ref().unwrap().request_id, 3);
        assert_eq!(nav.prev.as_ref().unwrap().request_id, 2);
        assert!(nav.held.is_none());
    }

    #[test]
    fn invalid_args_and_missing_graph() {
        let navs = Arc::new(Mutex::new(HashMap::new()));
        let mut bad = req(tile(0, 0, 9), tile(1, 1, 0), 51);
        bad.invalid_args = true;
        queue_inspect(&navs, "p", &None, None, vec![], None, None, bad);
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
            vec![],
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
    fn abort_script_walk_leaves_inspect_worker() {
        clear_barrier();
        let barrier = InspectBarrier::new();
        install_barrier(Arc::clone(&barrier));
        let world = world_with(TransportGraph::default(), vec![]);
        let navs = Arc::new(Mutex::new(HashMap::new()));
        queue_inspect(
            &navs,
            "p",
            &Some(world),
            Some(WorldState::empty()),
            vec![],
            None,
            None,
            req(tile(0, 0, 0), tile(2, 2, 0), 61),
        );
        barrier.wait_entered();
        crate::abort_script_walk(&navs, "p");
        {
            let all = navs.lock().unwrap();
            let bot = all.get("p").unwrap();
            assert!(bot.inspect.worker.is_some());
            assert_eq!(bot.inspect.running_id, 61);
            assert!(bot.route_worker.is_none());
        }
        barrier.release();
        let term = wait_latest(&navs, 61);
        assert_eq!(term.request_id, 61);
        clear_barrier();
    }

    #[allow(clippy::too_many_arguments)] // test capture packs world/tiles/budget fields
    fn capture(
        world: Arc<NavWorld>,
        from: WorldTile,
        to: WorldTile,
        opts: FindOptions,
        request_id: u64,
        bank: Vec<(i32, i32)>,
        strict: Option<usize>,
        stand: Option<usize>,
        post: Option<usize>,
    ) -> InspectCapture {
        InspectCapture {
            request_id,
            generation: 0,
            world,
            state: WorldState::empty(),
            bank,
            from,
            to,
            opts,
            avoid: vec![],
            cache: None,
            obj_names: None,
            test_barrier: None,
            search_budget_strict: strict,
            search_budget_stand: stand,
            search_budget_post: post,
        }
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
            vec![],
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
            vec![],
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
            vec![(995, 10)],
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
            vec![(995, 10)],
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
                vec![],
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
            vec![],
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
            vec![],
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
                vec![],
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
            vec![],
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
            vec![],
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
            vec![],
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
        clear_barrier();
        let barrier = InspectBarrier::new();
        install_barrier(Arc::clone(&barrier));
        let world = world_with(TransportGraph::default(), vec![]);
        let navs = Arc::new(Mutex::new(HashMap::new()));
        queue_inspect(
            &navs,
            "p",
            &Some(Arc::clone(&world)),
            Some(WorldState::empty()),
            vec![],
            None,
            None,
            req(tile(0, 0, 0), tile(1, 1, 0), 81),
        );
        barrier.wait_entered();
        queue_inspect(
            &navs,
            "p",
            &Some(world),
            Some(WorldState::empty()),
            vec![],
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
        barrier.release();
        let _ = wait_latest(&navs, 81);
        clear_barrier();
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
                script::shim::InteractReq::InspectAck { seq, generation } => {
                    Some((seq, generation))
                }
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
        clear_barrier();
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
        let barrier = InspectBarrier::new();
        install_barrier(Arc::clone(&barrier));
        queue_inspect(
            &navs,
            "p",
            &Some(Arc::clone(&world)),
            Some(WorldState::empty()),
            vec![],
            None,
            None,
            req(tile(0, 0, 0), tile(1, 1, 0), a),
        );
        barrier.wait_entered();
        queue_inspect(
            &navs,
            "p",
            &Some(Arc::clone(&world)),
            Some(WorldState::empty()),
            vec![],
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
            vec![],
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
            assert!(bot.inspect.worker.is_some());
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
        barrier.release();
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
        assert!(navs
            .lock()
            .unwrap()
            .get("p")
            .unwrap()
            .inspect
            .worker
            .is_none());

        clear_barrier();
        let sustain = InspectBarrier::new();
        install_barrier(Arc::clone(&sustain));
        let d = inspect_begin_at(&iso, tile(0, 0, 0), tile(4, 4, 0));
        queue_inspect(
            &navs,
            "p",
            &Some(Arc::clone(&world)),
            Some(WorldState::empty()),
            vec![],
            None,
            None,
            req(tile(0, 0, 0), tile(4, 4, 0), d),
        );
        sustain.wait_entered();
        let e = inspect_begin_at(&iso, tile(0, 0, 0), tile(5, 5, 0));
        queue_inspect(
            &navs,
            "p",
            &Some(Arc::clone(&world)),
            Some(WorldState::empty()),
            vec![],
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
        sustain.release();
        let _ = wait_latest(&navs, d);
        let _ = wait_latest(&navs, e);
        post_and_apply(&iso, &navs, 3);
        assert!(inspect_settled(&iso, d));
        assert!(inspect_settled(&iso, e));

        clear_barrier();
        let f = inspect_begin_at(&iso, tile(0, 0, 0), tile(1, 2, 0));
        queue_inspect(
            &navs,
            "p",
            &Some(Arc::clone(&world)),
            Some(WorldState::empty()),
            vec![],
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
            vec![],
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
            vec![],
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
            vec![],
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

        clear_barrier();
        let reset_barrier = InspectBarrier::new();
        install_barrier(Arc::clone(&reset_barrier));
        let live = inspect_begin_at(&iso, tile(0, 0, 0), tile(7, 7, 0));
        queue_inspect(
            &navs,
            "p",
            &Some(world),
            Some(WorldState::empty()),
            vec![],
            None,
            None,
            req(tile(0, 0, 0), tile(7, 7, 0), live),
        );
        reset_barrier.wait_entered();
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
            assert!(bot.inspect.worker.is_some());
            assert_eq!(
                Arc::strong_count(bot.inspect.worker.as_ref().unwrap()),
                2,
                "live worker token stays after reset"
            );
            bot.inspect.apply_ack(stale_ack.0, stale_ack.1);
            assert_eq!(bot.inspect.observed_seq, 0);
            assert!(bot.inspect.latest.is_none());
        }
        reset_barrier.release();
        clear_barrier();
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
            vec![],
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
                vec![],
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
            vec![],
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
                vec![],
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
                vec![],
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
            vec![],
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
