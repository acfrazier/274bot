use super::*;

fn held(id: i32, name: &str, slot: i32, count: i32, op: Option<&str>) -> ItemView {
    ItemView {
        def: def(id, name),
        container: ItemContainer::Inventory,
        action_family: ItemActionFamily::Held,
        slot,
        count,
        actions: op.into_iter().map(|op| Some(op.into())).collect(),
        component_id: 3214,
    }
}

fn accept_last(ledger: &mut Option<Box<ledger::Ledger>>, tick: u64, accepted: bool) {
    let authority = ledger.as_ref().unwrap().outbox.last().unwrap().authority();
    ledger.as_mut().unwrap().complete_interaction(
        &authority,
        crate::native::InteractionReceipt {
            request_id: authority.request_id().get(),
            evidence: EvidenceStamp {
                run: authority.run(),
                tick,
                sequence: tick,
            },
            accepted,
            chat_since: 0,
        },
    );
}

fn continuation(options: dialogue::DialogueOptions) -> dialogue::DialogueArgs {
    dialogue::DialogueArgs {
        target: dialogue::DialogueTarget::Continuation,
        options,
    }
}

#[test]
fn strict_line_rules_select_current_page_before_fixed_and_preferred_choices() {
    let mut snapshot = ready();
    snapshot.seed_chat_modal(100, vec!["What is the correct incantation?".into()]);
    snapshot.seed_chat_options(
        vec![
            api::snapshot::ChatOptionView {
                component_id: 101,
                text: "Wrong words".into(),
            },
            api::snapshot::ChatOptionView {
                component_id: 102,
                text: "Carlem Aber Camerinthum Purchai Gabindo".into(),
            },
        ],
        -1,
    );
    let mut ledger = None;
    let handle = with_tick(&snapshot, &mut ledger, 1, |tick| {
        tick.actions
            .begin::<dialogue::Dialogue>(
                continuation(dialogue::DialogueOptions {
                    prefer: Arc::from([Arc::from("Wrong words")]),
                    choose: Some(1),
                    line_rules: Arc::from([dialogue::LineRule {
                        when_line: Arc::from("correct incantation"),
                        choose: Arc::from("Carlem Aber Camerinthum Purchai Gabindo"),
                    }]),
                    strict: true,
                }),
                &mut tick.cx,
            )
            .unwrap()
    });
    assert!(with_tick(&snapshot, &mut ledger, 2, |tick| tick
        .actions
        .poll(&handle, &mut tick.cx))
    .is_pending());
    assert!(matches!(
        emitted(&ledger),
        InteractReq::Answer { option: 2 }
    ));
}

#[test]
fn strict_dialogue_rejects_unmatched_ambiguous_and_out_of_range_answers() {
    for (options, prefer, choose) in [
        (vec!["No", "Later"], vec!["Yes"], None),
        (vec!["Yes please", "Yes certainly"], vec!["Yes"], None),
        (vec!["Yes", "No"], vec![], Some(3)),
        (vec!["Yes", "No"], vec![], None),
    ] {
        let mut snapshot = ready();
        snapshot.seed_chat_modal(100, vec!["A question".into()]);
        snapshot.seed_chat_options(
            options
                .into_iter()
                .enumerate()
                .map(|(index, text)| api::snapshot::ChatOptionView {
                    component_id: 101 + index as i32,
                    text: text.into(),
                })
                .collect(),
            -1,
        );
        let mut ledger = None;
        let unconfigured = prefer.is_empty() && choose.is_none();
        let handle = with_tick(&snapshot, &mut ledger, 1, |tick| {
            tick.actions
                .begin::<dialogue::Dialogue>(
                    continuation(dialogue::DialogueOptions {
                        prefer: prefer.into_iter().map(Arc::from).collect(),
                        choose,
                        strict: true,
                        ..Default::default()
                    }),
                    &mut tick.cx,
                )
                .unwrap()
        });
        let result = with_tick(&snapshot, &mut ledger, 2, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        });
        if unconfigured {
            assert!(
                matches!(result, Poll::Ready(Err(ActionError::Failed(reason)))
                if reason.as_ref() == "dialogue menu requires an explicit answer rule")
            );
        } else {
            assert!(matches!(
                result,
                Poll::Ready(Ok(crate::dialogue_outcome::DialogueOutcome::Failed))
            ));
        }
        assert!(
            ledger
                .as_ref()
                .is_none_or(|ledger| ledger.outbox.is_empty()),
            "a rejected choice must send no answer"
        );
    }
}

#[test]
fn continuation_waits_boundedly_without_talking_or_walking() {
    let snapshot = ready();
    let mut ledger = None;
    let handle = with_tick(&snapshot, &mut ledger, 1, |tick| {
        tick.actions
            .begin::<dialogue::Dialogue>(continuation(Default::default()), &mut tick.cx)
            .unwrap()
    });
    assert!(with_tick(&snapshot, &mut ledger, 2, |tick| tick
        .actions
        .poll(&handle, &mut tick.cx))
    .is_pending());
    assert!(matches!(
        with_tick(&snapshot, &mut ledger, 15, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        }),
        Poll::Ready(Err(ActionError::Failed(reason)))
            if reason.as_ref() == "expected dialogue did not open"
    ));
    assert!(ledger
        .as_ref()
        .is_none_or(|ledger| ledger.outbox.is_empty()));
}

#[test]
fn dialogue_drains_a_page_after_six_closed_game_ticks() {
    let mut snapshot = ready();
    snapshot.seed_chat_modal(100, vec!["First page".into()]);
    snapshot.seed_chat_options(vec![], 101);
    let mut ledger = None;
    let handle = with_tick(&snapshot, &mut ledger, 1, |tick| {
        tick.actions
            .begin::<dialogue::Dialogue>(continuation(Default::default()), &mut tick.cx)
            .unwrap()
    });
    assert!(with_tick(&snapshot, &mut ledger, 2, |tick| tick
        .actions
        .poll(&handle, &mut tick.cx))
    .is_pending());
    snapshot.seed_chat_modal(-1, vec![]);
    snapshot.seed_chat_options(vec![], -1);
    for game_tick in 3..=8 {
        assert!(with_tick(&snapshot, &mut ledger, game_tick, |tick| tick
            .actions
            .poll(&handle, &mut tick.cx))
        .is_pending());
    }
    snapshot.seed_chat_modal(100, vec!["Final reward page".into()]);
    snapshot.seed_chat_options(vec![], 101);
    ledger.as_mut().unwrap().outbox.clear();
    assert!(with_tick(&snapshot, &mut ledger, 9, |tick| tick
        .actions
        .poll(&handle, &mut tick.cx))
    .is_pending());
    assert!(matches!(
        emitted(&ledger),
        InteractReq::ContinueDialog { component_id: None }
    ));
}

#[test]
fn dialogue_waits_for_delayed_work_page_but_animation_cannot_rearm_forever() {
    for final_page in [true, false] {
        let mut snapshot = ready();
        snapshot.seed_chat_modal(100, vec![]);
        snapshot.seed_local_player(local_player(tile(5, 5)));
        let mut ledger = None;
        let handle = with_tick(&snapshot, &mut ledger, 1, |tick| {
            tick.actions
                .begin::<dialogue::Dialogue>(continuation(Default::default()), &mut tick.cx)
                .unwrap()
        });
        snapshot.seed_chat_modal(-1, vec![]);
        let mut player = local_player(tile(5, 5));
        player.player.actor.animation = 898;
        snapshot.seed_local_player(player);
        assert!(with_tick(&snapshot, &mut ledger, 2, |tick| tick
            .actions
            .poll(&handle, &mut tick.cx))
        .is_pending());
        assert!(with_tick(&snapshot, &mut ledger, 10, |tick| tick
            .actions
            .poll(&handle, &mut tick.cx))
        .is_pending());
        if final_page {
            snapshot.seed_chat_modal(100, vec!["The anvil work is finished.".into()]);
            snapshot.seed_chat_options(vec![], 101);
            assert!(with_tick(&snapshot, &mut ledger, 14, |tick| tick
                .actions
                .poll(&handle, &mut tick.cx))
            .is_pending());
            assert!(matches!(
                emitted(&ledger),
                InteractReq::ContinueDialog { component_id: None }
            ));
        } else {
            for game_tick in [18, 26, 34] {
                assert!(with_tick(&snapshot, &mut ledger, game_tick, |tick| tick
                    .actions
                    .poll(&handle, &mut tick.cx))
                .is_pending());
            }
            assert!(matches!(
                with_tick(&snapshot, &mut ledger, 42, |tick| tick
                    .actions
                    .poll(&handle, &mut tick.cx)),
                Poll::Ready(Ok(crate::dialogue_outcome::DialogueOutcome::Completed))
            ));
        }
    }
}

#[test]
fn held_operation_uses_exact_inventory_slot_and_requires_acceptance() {
    for accepted in [true, false] {
        let mut snapshot = ready();
        snapshot.seed_inventory(
            vec![
                held(1, "IOU", 2, 1, Some("Read")),
                held(2, "IOU", 7, 1, Some("Read")),
            ],
            28,
        );
        let mut args = reach_args(
            reach::ReachKind::Held {
                id: 2,
                obj: Arc::from("IOU"),
            },
            false,
        );
        args.op = Arc::from("Read");
        args.anchor = None;
        let mut ledger = None;
        let handle = with_tick(&snapshot, &mut ledger, 1, |tick| {
            tick.actions
                .begin::<reach::Reach>(args, &mut tick.cx)
                .unwrap()
        });
        assert!(
            matches!(emitted(&ledger), InteractReq::Held { name, action, slot: Some(7), .. } if name == "IOU" && action == "Read")
        );
        assert!(with_tick(&snapshot, &mut ledger, 2, |tick| tick
            .actions
            .poll(&handle, &mut tick.cx))
        .is_pending());
        accept_last(&mut ledger, 3, accepted);
        let result = with_tick(&snapshot, &mut ledger, 3, |tick| {
            tick.actions.poll(&handle, &mut tick.cx)
        });
        if accepted {
            assert!(matches!(result, Poll::Ready(Ok(true))));
        } else {
            assert!(matches!(result, Poll::Ready(Err(ActionError::Failed(_)))));
        }
    }
}

#[test]
fn held_operation_refuses_an_unobserved_item_action() {
    let mut snapshot = ready();
    snapshot.seed_inventory(vec![held(1, "IOU", 7, 1, Some("Drop"))], 28);
    let mut args = reach_args(
        reach::ReachKind::Held {
            id: 1,
            obj: Arc::from("IOU"),
        },
        false,
    );
    args.op = Arc::from("Read");
    let mut ledger = None;
    assert!(matches!(
        with_tick(&snapshot, &mut ledger, 1, |tick| tick
            .actions
            .begin::<reach::Reach>(args, &mut tick.cx)),
        Err(ActionError::Unavailable(_))
    ));
    assert!(ledger
        .as_ref()
        .is_none_or(|ledger| ledger.outbox.is_empty()));
}

#[test]
fn loc_interaction_drains_shared_strict_dialogue_before_success() {
    compile_context_test(|compile| {
        let plan = compile_interact(
            test_args::<InteractArgs>(serde_json::json!({
                "target": {"loc": "priestperiltempledoorl"},
                "op": "Knock-at",
                "radius": 8,
                "dialogue": {"prefer": ["Yes"], "strict": true}
            })),
            compile,
        )
        .unwrap();
        let mut snapshot = ready();
        snapshot.seed_locs(vec![loc(
            resolve_loc(compile, "priestperiltempledoorl").unwrap(),
            "Door",
            "Knock-at",
        )]);
        let mut ledger = None;
        let mut run = with_tick(&snapshot, &mut ledger, 1, |tick| {
            with_step(tick, |cx| plan.begin(cx).unwrap())
        });
        assert!(
            with_tick(&snapshot, &mut ledger, 2, |tick| with_step(tick, |cx| run
                .poll(cx)))
            .is_pending()
        );
        snapshot.seed_chat_modal(100, vec!["Shall I operate this?".into()]);
        snapshot.seed_chat_options(
            vec![api::snapshot::ChatOptionView {
                component_id: 101,
                text: "Yes".into(),
            }],
            -1,
        );
        assert!(
            with_tick(&snapshot, &mut ledger, 3, |tick| with_step(tick, |cx| run
                .poll(cx)))
            .is_pending()
        );
        assert!(
            with_tick(&snapshot, &mut ledger, 4, |tick| with_step(tick, |cx| run
                .poll(cx)))
            .is_pending()
        );
        assert!(matches!(
            emitted(&ledger),
            InteractReq::Answer { option: 1 }
        ));
    });
}

#[test]
fn exact_loc_selection_skips_unreachable_same_id_decoys() {
    let mut snapshot = ready();
    let mut wanted = loc(1000, "Roots", "Search");
    wanted.tile = tile(5, 5);
    wanted.distance = 2;
    let mut decoy = wanted.clone();
    decoy.tile = tile(6, 5);
    decoy.distance = 1;
    snapshot.seed_locs(vec![decoy, wanted]);
    let reach_view = wall_door_reach_view();
    let mut ledger = None;
    with_tick_reach(&snapshot, &reach_view, &mut ledger, 1, |tick| {
        assert_eq!(
            reach::nearest_loc(&tick.cx, Some(1000), None, Some("Search"), 6, None, true)
                .unwrap()
                .tile,
            tile(5, 5)
        );
        assert!(reach::nearest_loc(
            &tick.cx,
            Some(1000),
            None,
            Some("Search"),
            6,
            Some(tile(6, 5)),
            true
        )
        .is_none());
    });
    with_tick(&snapshot, &mut ledger, 2, |tick| {
        assert!(
            reach::nearest_loc(&tick.cx, Some(1000), None, Some("Search"), 6, None, true).is_none(),
            "unknown reach must not select a resource candidate"
        );
    });
}

#[test]
fn interact_until_waits_for_work_then_rearms_and_completes_at_actual_count() {
    let mut snapshot = ready();
    snapshot.seed_local_player(local_player(tile(5, 5)));
    let mut resource = loc(1000, "Rock", "Mine");
    resource.tile = tile(5, 5);
    resource.distance = 1;
    snapshot.seed_locs(vec![resource]);
    let plan = InteractPlan {
        kind: reach::ReachKind::Loc {
            id: Some(1000),
            name: None,
        },
        op: Arc::from("Mine"),
        tile: None,
        radius: 6,
        wait_if_missing: false,
        settle_ms: Some(60_000),
        ambiguous: false,
        default_dialogue: true,
        dialogue_options: None,
        until: Some((1, s2::QuantityPlan::Fixed(2))),
        target_tile: None,
        reachable_only: false,
    };
    let reach_view = wall_door_reach_view();
    let mut ledger = None;
    let mut run = with_tick_reach(&snapshot, &reach_view, &mut ledger, 1, |tick| {
        with_step(tick, |cx| plan.begin(cx).unwrap())
    });
    for game_tick in [2, 3] {
        assert!(
            with_tick_reach(&snapshot, &reach_view, &mut ledger, game_tick, |tick| {
                with_step(tick, |cx| run.poll(cx))
            })
            .is_pending()
        );
    }
    let first_request = ledger.as_ref().unwrap().outbox.last().unwrap().request_id;
    snapshot.seed_inventory(vec![held(1, "Ore", 0, 1, None)], 28);
    let mut player = local_player(tile(5, 5));
    player.player.actor.animation = 625;
    snapshot.seed_local_player(player);
    assert!(
        with_tick_reach(&snapshot, &reach_view, &mut ledger, 4, |tick| with_step(
            tick,
            |cx| run.poll(cx)
        ))
        .is_pending()
    );
    assert_eq!(
        ledger.as_ref().unwrap().outbox.last().unwrap().request_id,
        first_request,
        "do not re-click while mining is active"
    );
    snapshot.seed_local_player(local_player(tile(5, 5)));
    assert!(
        with_tick_reach(&snapshot, &reach_view, &mut ledger, 5, |tick| with_step(
            tick,
            |cx| run.poll(cx)
        ))
        .is_pending()
    );
    assert_ne!(
        ledger.as_ref().unwrap().outbox.last().unwrap().request_id,
        first_request,
        "progress followed by idle must rearm promptly"
    );
    snapshot.seed_inventory(vec![held(1, "Ore", 0, 2, None)], 28);
    assert!(
        with_tick_reach(&snapshot, &reach_view, &mut ledger, 6, |tick| {
            with_step(tick, |cx| run.poll(cx))
        })
        .is_pending()
    );
    assert!(matches!(
        with_tick_reach(&snapshot, &reach_view, &mut ledger, 7, |tick| with_step(
            tick,
            |cx| run.poll(cx)
        )),
        Poll::Ready(Ok(_))
    ));
}

#[test]
fn ground_use_on_selects_exact_tile_and_id_not_nearest_decoy() {
    compile_context_test(|compile| {
        let source_id = resolve_obj(compile, "shears").unwrap();
        let target_id = resolve_obj(compile, "wool").unwrap();
        let plan = compile_use_on(
            test_args::<UseOnArgs>(serde_json::json!({
                "item": "shears",
                "target": {"ground": "wool", "tile": [5, 5, 0], "source": "unit fixture"},
                "radius": 8
            })),
            compile,
        )
        .unwrap();
        let mut snapshot = ready();
        snapshot.seed_inventory(vec![held(source_id, "Shears", 7, 1, None)], 28);
        snapshot.seed_ground_items(vec![
            GroundItemView {
                def: def(target_id, "Wool"),
                count: 1,
                actions: vec![],
                tile: tile(6, 5),
                distance: 1,
            },
            GroundItemView {
                def: def(target_id + 1, "Wool"),
                count: 1,
                actions: vec![],
                tile: tile(5, 5),
                distance: 1,
            },
            GroundItemView {
                def: def(target_id, "Wool"),
                count: 1,
                actions: vec![],
                tile: tile(5, 5),
                distance: 2,
            },
        ]);
        let mut ledger = None;
        let mut run = with_tick(&snapshot, &mut ledger, 1, |tick| {
            with_step(tick, |cx| plan.begin(cx).unwrap())
        });
        assert!(
            with_tick(&snapshot, &mut ledger, 2, |tick| with_step(tick, |cx| run
                .poll(cx)))
            .is_pending()
        );
        assert!(
            matches!(emitted(&ledger), InteractReq::UseOn { kind, x: 5, z: 5, source_item_id: Some(source), source_item_slot: Some(7), target_item_id: Some(target), target_item_slot: None, .. } if kind == "obj" && *source == source_id && *target == target_id)
        );
    });
}

#[test]
fn held_use_on_uses_canonical_host_kind_and_drains_dialogue_before_product_success() {
    compile_context_test(|compile| {
        let source_id = resolve_obj(compile, "shears").unwrap();
        let target_id = resolve_obj(compile, "wool").unwrap();
        let plan = compile_use_on(
            test_args::<UseOnArgs>(serde_json::json!({
                "item": "shears",
                "target": {"item": "wool"},
                "product": "wool",
                "dialogue": "continue"
            })),
            compile,
        )
        .unwrap();
        let mut snapshot = ready();
        snapshot.seed_inventory(
            vec![
                held(source_id, "Shears", 7, 1, None),
                held(target_id, "Wool", 3, 1, None),
            ],
            28,
        );
        let mut ledger = None;
        let mut run = with_tick(&snapshot, &mut ledger, 1, |tick| {
            with_step(tick, |cx| plan.begin(cx).unwrap())
        });
        assert!(
            with_tick(&snapshot, &mut ledger, 2, |tick| with_step(tick, |cx| run
                .poll(cx)))
            .is_pending()
        );
        assert!(
            matches!(emitted(&ledger), InteractReq::UseOn { kind, target_item_id: Some(id), target_item_slot: Some(3), .. } if kind == "inv" && *id == target_id)
        );
        accept_last(&mut ledger, 3, true);
        snapshot.seed_chat_modal(100, vec!["The work is complete.".into()]);
        snapshot.seed_chat_options(vec![], 101);
        assert!(
            with_tick(&snapshot, &mut ledger, 3, |tick| with_step(tick, |cx| run
                .poll(cx)))
            .is_pending()
        );
        assert!(
            with_tick(&snapshot, &mut ledger, 4, |tick| with_step(tick, |cx| run
                .poll(cx)))
            .is_pending()
        );
        assert!(
            matches!(
                emitted(&ledger),
                InteractReq::ContinueDialog { component_id: None }
            ),
            "product observation cannot abandon an owned page"
        );
    });
}

#[test]
fn extension_args_reject_invalid_target_provenance_and_dialogue_rules() {
    compile_context_test(|compile| {
        for args in [
            serde_json::json!({"npc":"fred_the_farmer","continue_only":true}),
            serde_json::json!({"continue_only":true,"choose":0}),
            serde_json::json!({"continue_only":true,"line_rules":[{"when_line":"","choose":"Yes"}]}),
        ] {
            assert!(
                crate::quester::compile::decode_args::<TalkArgs>(&args)
                    .and_then(|args| compile_talk(args, compile))
                    .is_err(),
                "{args}"
            );
        }
        for args in [
            serde_json::json!({"target":{"held":"shears","tile":[5,5,0],"source":"fixture"},"op":"Read"}),
            serde_json::json!({"target":{"loc":"hopper_lumbridge","tile":[5,5,0]},"op":"Use"}),
            serde_json::json!({"target":{"held":"shears"},"op":"Read","reachable_only":true}),
            serde_json::json!({"target":{"loc":"hopper_lumbridge"},"op":"Use","until":{"obj":"wool","qty":0}}),
        ] {
            assert!(
                crate::quester::compile::decode_args::<InteractArgs>(&args)
                    .and_then(|args| compile_interact(args, compile))
                    .is_err(),
                "{args}"
            );
        }
        let args = serde_json::json!({"item":"shears","target":{"ground":"wool","tile":[5,5,0]}});
        assert!(crate::quester::compile::decode_args::<UseOnArgs>(&args)
            .and_then(|args| compile_use_on(args, compile))
            .is_err());
    });
}

#[test]
fn exact_ground_predicate_distinguishes_death_plateau_spawn_from_pedestal() {
    compile_context_test(|compile| {
        let id = resolve_obj(compile, "death_cannonball_blue").unwrap();
        let predicate = compile_ground_item_near(
            test_args::<GroundArg>(serde_json::json!({
                "obj": "death_cannonball_blue", "radius": 8, "at": [2894, 3562, 0]
            })),
            compile,
        )
        .unwrap();
        let mut snapshot = ready();
        snapshot.seed_local_player(local_player(tile(2894, 3563)));
        let spawn = GroundItemView {
            def: def(id, "Blue stone"),
            count: 1,
            actions: vec![],
            tile: tile(2893, 3564),
            distance: 1,
        };
        let evaluate = |snapshot: &GameSnapshot| {
            with_tick(snapshot, &mut None, 1, |tick| {
                with_step(tick, |cx| {
                    predicate.evaluate(&PredicateContext {
                        cx: &cx.tick.cx,
                        quests: cx.quests,
                        progress: cx.progress,
                        required_after: cx.required_after,
                        chat_since: 0,
                        outcome: None,
                        bank: cx.bank,
                    })
                })
            })
        };
        snapshot.seed_ground_items(vec![spawn.clone()]);
        assert_eq!(evaluate(&snapshot), Truth::False);
        snapshot.seed_ground_items(vec![
            spawn.clone(),
            GroundItemView {
                def: def(id + 1, "Blue stone"),
                tile: tile(2894, 3562),
                ..spawn.clone()
            },
        ]);
        assert_eq!(
            evaluate(&snapshot),
            Truth::False,
            "same display is not the selected item"
        );
        snapshot.seed_ground_items(vec![GroundItemView {
            tile: tile(2894, 3562),
            ..spawn
        }]);
        assert_eq!(evaluate(&snapshot), Truth::True);
    });
}

#[test]
fn acquisition_child_identity_is_borrowed_until_the_child_settles() {
    let snapshot = ready();
    let mut run = policy_s2_recipe_run(Box::new(WaitRun {
        until: Arc::new(AllPlan { items: vec![] }),
        deadline_tick: 10,
        chat_since: 0,
    }));
    assert_eq!(run.child_step_id().unwrap().0.as_ref(), "policy-child");
    let identity = run.child_step_id().unwrap().0.as_ptr();
    assert_eq!(run.child_step_id().unwrap().0.as_ptr(), identity);
    assert!(with_tick(&snapshot, &mut None, 1, |tick| {
        with_step(tick, |cx| run.poll(cx))
    })
    .is_pending());
    assert_eq!(run.child_step_id().unwrap().0.as_ptr(), identity);
    assert!(run.in_flight_outcome().is_some());
    assert!(matches!(
        with_tick(&snapshot, &mut None, 2, |tick| {
            with_step(tick, |cx| run.poll(cx))
        }),
        Poll::Ready(Ok(_))
    ));
    assert!(run.child_step_id().is_none());
}

fn operation_fixture(
    compile: &CompileContext<'_>,
    kind: &str,
    dialogue: Option<serde_json::Value>,
    until: bool,
) -> (GameSnapshot, Arc<dyn StepPlan>, i32) {
    let goal = resolve_obj(compile, "seasoned_sardine").unwrap();
    let mut snapshot = ready();
    snapshot.seed_inventory(
        vec![
            held(
                resolve_obj(compile, "death_iou").unwrap(),
                "IOU",
                0,
                1,
                Some("Read"),
            ),
            held(
                resolve_obj(compile, "doogleleaves").unwrap(),
                "Doogle leaves",
                1,
                3,
                None,
            ),
            held(
                resolve_obj(compile, "raw_sardine").unwrap(),
                "Raw sardine",
                2,
                3,
                None,
            ),
        ],
        28,
    );
    let mut args = match kind {
        "interact" => serde_json::json!({"target":{"held":"death_iou"},"op":"Read"}),
        "use_on" => serde_json::json!({"item":"doogleleaves","target":{"item":"raw_sardine"}}),
        _ => unreachable!(),
    };
    args["settle_ms"] = serde_json::json!(60_000);
    if let Some(dialogue) = dialogue {
        args["dialogue"] = dialogue;
    }
    if until {
        args["until"] = serde_json::json!({"obj":"seasoned_sardine","qty":2});
    }
    let plan = match kind {
        "interact" => compile_interact(test_args::<InteractArgs>(args), compile),
        "use_on" => compile_use_on(test_args::<UseOnArgs>(args), compile),
        _ => unreachable!(),
    }
    .unwrap();
    (snapshot, plan, goal)
}

fn set_operation_count(snapshot: &mut GameSnapshot, id: i32, count: i32) {
    let mut inventory = snapshot.inventory().to_vec();
    inventory.retain(|item| item.def.id != id);
    inventory.push(held(id, "Seasoned sardine", 3, count, None));
    snapshot.seed_inventory(inventory, 28);
}

#[test]
fn use_on_omitted_dialogue_refuses_an_observed_menu_without_answering() {
    compile_context_test(|compile| {
        let (mut snapshot, plan, _) = operation_fixture(compile, "use_on", None, false);
        let mut ledger = None;
        let mut run = with_tick(&snapshot, &mut ledger, 1, |tick| {
            with_step(tick, |cx| plan.begin(cx).unwrap())
        });
        assert!(with_tick(&snapshot, &mut ledger, 2, |tick| {
            with_step(tick, |cx| run.poll(cx))
        })
        .is_pending());
        assert!(matches!(emitted(&ledger), InteractReq::UseOn { .. }));
        accept_last(&mut ledger, 3, true);
        snapshot.seed_chat_modal(100, vec!["Choose what happens next.".into()]);
        snapshot.seed_chat_options(
            vec![
                api::snapshot::ChatOptionView {
                    component_id: 101,
                    text: "First choice".into(),
                },
                api::snapshot::ChatOptionView {
                    component_id: 102,
                    text: "Last choice".into(),
                },
            ],
            -1,
        );
        assert!(with_tick(&snapshot, &mut ledger, 3, |tick| {
            with_step(tick, |cx| run.poll(cx))
        })
        .is_pending());
        assert!(matches!(
            with_tick(&snapshot, &mut ledger, 4, |tick| {
                with_step(tick, |cx| run.poll(cx))
            }),
            Poll::Ready(Err(ActionError::Failed(reason)))
                if reason.as_ref() == "dialogue menu requires an explicit answer rule"
        ));
        assert!(ledger.as_ref().unwrap().outbox.iter().all(|request| {
            !matches!(
                request.effect,
                HostEffect::Interaction(InteractReq::Answer { .. })
            )
        }));
    });
}

#[test]
fn operation_explicit_dialogue_reports_when_an_expected_page_does_not_open() {
    compile_context_test(|compile| {
        for kind in ["interact", "use_on"] {
            for dialogue in [
                serde_json::json!("continue"),
                serde_json::json!({"prefer":["Yes"]}),
            ] {
                for until in [false, true] {
                    let (snapshot, plan, _) =
                        operation_fixture(compile, kind, Some(dialogue.clone()), until);
                    let mut ledger = None;
                    let mut run = with_tick(&snapshot, &mut ledger, 1, |tick| {
                        with_step(tick, |cx| plan.begin(cx).unwrap())
                    });
                    for tick in 2..=4 {
                        assert!(with_tick(&snapshot, &mut ledger, tick, |tick| {
                            with_step(tick, |cx| run.poll(cx))
                        })
                        .is_pending());
                    }
                    assert!(matches!(
                        emitted(&ledger),
                        InteractReq::UseOn { .. } | InteractReq::Held { .. }
                    ));
                    accept_last(&mut ledger, 5, true);
                    let mut outcome = Poll::Pending;
                    for tick in 5..=21 {
                        outcome = with_tick(&snapshot, &mut ledger, tick, |tick| {
                            with_step(tick, |cx| run.poll(cx))
                        });
                        if outcome.is_ready() {
                            break;
                        }
                    }
                    assert!(
                        matches!(
                            outcome,
                            Poll::Ready(Err(ActionError::Failed(reason)))
                                if reason.as_ref() == "expected dialogue did not open"
                        ),
                        "{kind}, until={until}, dialogue={dialogue}"
                    );
                    assert!(ledger.as_ref().unwrap().outbox.iter().all(|request| {
                        matches!(
                            request.effect,
                            HostEffect::Interaction(
                                InteractReq::UseOn { .. } | InteractReq::Held { .. }
                            )
                        )
                    }));
                }
            }
        }
    });
}

#[test]
fn operation_until_requires_explicit_dialogue_again_after_each_accepted_round() {
    compile_context_test(|compile| {
        for kind in ["interact", "use_on"] {
            let (mut snapshot, plan, goal) =
                operation_fixture(compile, kind, Some(serde_json::json!("continue")), true);
            let mut ledger = None;
            let mut run = with_tick(&snapshot, &mut ledger, 1, |tick| {
                with_step(tick, |cx| plan.begin(cx).unwrap())
            });
            for tick in 2..=4 {
                assert!(with_tick(&snapshot, &mut ledger, tick, |tick| {
                    with_step(tick, |cx| run.poll(cx))
                })
                .is_pending());
            }
            accept_last(&mut ledger, 5, true);
            snapshot.seed_chat_modal(100, vec!["First round.".into()]);
            snapshot.seed_chat_options(vec![], 105);
            for tick in 5..=6 {
                assert!(with_tick(&snapshot, &mut ledger, tick, |tick| {
                    with_step(tick, |cx| run.poll(cx))
                })
                .is_pending());
            }
            assert!(matches!(
                emitted(&ledger),
                InteractReq::ContinueDialog { .. }
            ));
            accept_last(&mut ledger, 7, true);
            snapshot.seed_chat_modal(-1, vec![]);
            snapshot.seed_chat_options(vec![], -1);
            set_operation_count(&mut snapshot, goal, 1);
            let mut second_tick = None;
            for tick in 7..=22 {
                assert!(with_tick(&snapshot, &mut ledger, tick, |tick| {
                    with_step(tick, |cx| run.poll(cx))
                })
                .is_pending());
                if matches!(
                    emitted(&ledger),
                    InteractReq::UseOn { .. } | InteractReq::Held { .. }
                ) {
                    second_tick = Some(tick);
                    break;
                }
            }
            let second_tick = second_tick.expect("a drained first round must rearm the operation");
            accept_last(&mut ledger, second_tick + 1, true);
            let mut outcome = Poll::Pending;
            for tick in second_tick + 1..=second_tick + 17 {
                outcome = with_tick(&snapshot, &mut ledger, tick, |tick| {
                    with_step(tick, |cx| run.poll(cx))
                });
                if outcome.is_ready() {
                    break;
                }
            }
            assert!(
                matches!(
                    outcome,
                    Poll::Ready(Err(ActionError::Failed(reason)))
                        if reason.as_ref() == "expected dialogue did not open"
                ),
                "{kind}: the second accepted round still requires a page"
            );
        }
    });
}

#[test]
fn use_on_omitted_dialogue_does_not_require_a_page() {
    compile_context_test(|compile| {
        for until in [false, true] {
            let (mut snapshot, plan, goal) = operation_fixture(compile, "use_on", None, until);
            let mut ledger = None;
            let mut run = with_tick(&snapshot, &mut ledger, 1, |tick| {
                with_step(tick, |cx| plan.begin(cx).unwrap())
            });
            assert!(with_tick(&snapshot, &mut ledger, 2, |tick| {
                with_step(tick, |cx| run.poll(cx))
            })
            .is_pending());
            accept_last(&mut ledger, 3, true);
            if until {
                set_operation_count(&mut snapshot, goal, 2);
            }
            assert!(with_tick(&snapshot, &mut ledger, 3, |tick| {
                with_step(tick, |cx| run.poll(cx))
            })
            .is_pending());
            assert!(matches!(
                with_tick(&snapshot, &mut ledger, 4, |tick| {
                    with_step(tick, |cx| run.poll(cx))
                }),
                Poll::Ready(Ok(_))
            ));
            assert!(matches!(emitted(&ledger), InteractReq::UseOn { .. }));
        }
    });
}

#[test]
fn use_on_until_drains_an_initial_continue_only_page_with_the_shared_driver() {
    compile_context_test(|compile| {
        let (mut snapshot, plan, _) = operation_fixture(compile, "use_on", None, true);
        snapshot.seed_chat_modal(-1, vec!["An existing page.".into()]);
        snapshot.seed_chat_options(vec![], 105);
        let mut ledger = None;
        let mut run = with_tick(&snapshot, &mut ledger, 1, |tick| {
            with_step(tick, |cx| plan.begin(cx).unwrap())
        });
        assert!(with_tick(&snapshot, &mut ledger, 2, |tick| {
            with_step(tick, |cx| run.poll(cx))
        })
        .is_pending());
        assert!(
            ledger
                .as_ref()
                .is_none_or(|ledger| ledger.outbox.is_empty()),
            "begin the shared dialogue driver before dispatching the next count round"
        );
        assert!(with_tick(&snapshot, &mut ledger, 3, |tick| {
            with_step(tick, |cx| run.poll(cx))
        })
        .is_pending());
        assert!(matches!(
            emitted(&ledger),
            InteractReq::ContinueDialog { .. }
        ));
        accept_last(&mut ledger, 4, true);
        snapshot.seed_chat_modal(-1, vec![]);
        snapshot.seed_chat_options(vec![], -1);
        let mut dispatched = false;
        for tick in 4..=16 {
            assert!(with_tick(&snapshot, &mut ledger, tick, |tick| {
                with_step(tick, |cx| run.poll(cx))
            })
            .is_pending());
            if matches!(emitted(&ledger), InteractReq::UseOn { .. }) {
                dispatched = true;
                break;
            }
        }
        assert!(
            dispatched,
            "continue acknowledgement and the shared close gap must precede UseOn"
        );
    });
}

#[test]
fn interact_until_reports_rejected_reach_without_waiting_for_settlement() {
    let mut snapshot = ready();
    snapshot.seed_local_player(local_player(tile(5, 5)));
    let mut target = loc(1000, "Door", "Open");
    target.tile = tile(5, 5);
    target.distance = 1;
    snapshot.seed_locs(vec![target]);
    let plan = InteractPlan {
        kind: reach::ReachKind::Loc {
            id: Some(1000),
            name: None,
        },
        op: Arc::from("Open"),
        tile: None,
        radius: 6,
        wait_if_missing: false,
        settle_ms: Some(60_000),
        ambiguous: false,
        default_dialogue: true,
        dialogue_options: None,
        until: Some((1, s2::QuantityPlan::Fixed(2))),
        target_tile: None,
        reachable_only: false,
    };
    let reach_view = wall_door_reach_view();
    let mut ledger = None;
    let mut run = with_tick_reach(&snapshot, &reach_view, &mut ledger, 1, |tick| {
        with_step(tick, |cx| plan.begin(cx).unwrap())
    });
    assert!(
        with_tick_reach(&snapshot, &reach_view, &mut ledger, 2, |tick| {
            with_step(tick, |cx| run.poll(cx))
        })
        .is_pending()
    );
    assert!(matches!(
        emitted(&ledger),
        InteractReq::Loc { id: Some(1000), .. }
    ));
    accept_last(&mut ledger, 3, false);
    snapshot.seed_locs(vec![]);
    assert!(matches!(
        with_tick_reach(&snapshot, &reach_view, &mut ledger, 3, |tick| {
            with_step(tick, |cx| run.poll(cx))
        }),
        Poll::Ready(Err(ActionError::Failed(reason)))
            if reason.as_ref() == "interact target not reached"
    ));
}

#[test]
fn held_interact_compile_error_preserves_the_authored_alias() {
    compile_context_test(|compile| {
        let alias = "missing_held_alias";
        let error = compile_interact(
            test_args::<InteractArgs>(serde_json::json!({"target":{"held":alias},"op":"Read"})),
            compile,
        )
        .err()
        .unwrap();
        assert_eq!(error.code.as_ref(), "unresolved-obj");
        assert_eq!(error.detail.as_deref(), Some(alias));
    });
}

#[test]
fn operation_until_goal_observed_on_the_deadline_poll_wins_over_timeout() {
    compile_context_test(|compile| {
        for kind in ["interact", "use_on"] {
            let (mut snapshot, plan, goal) = operation_fixture(compile, kind, None, true);
            let mut ledger = None;
            let mut run = with_tick(&snapshot, &mut ledger, 1, |tick| {
                with_step(tick, |cx| plan.begin(cx).unwrap())
            });
            for tick in 2..=4 {
                assert!(with_tick(&snapshot, &mut ledger, tick, |tick| {
                    with_step(tick, |cx| run.poll(cx))
                })
                .is_pending());
            }
            accept_last(&mut ledger, 5, true);
            assert!(with_tick(&snapshot, &mut ledger, 5, |tick| {
                with_step(tick, |cx| run.poll(cx))
            })
            .is_pending());
            set_operation_count(&mut snapshot, goal, 2);
            assert!(
                matches!(
                    with_tick(&snapshot, &mut ledger, 102, |tick| {
                        with_step(tick, |cx| run.poll(cx))
                    }),
                    Poll::Ready(Ok(_))
                ),
                "{kind}: an observed goal at the 60-second deadline is not a timeout"
            );
        }
    });
}

#[test]
fn use_on_until_drains_post_use_page_after_pre_dispatch_drain() {
    compile_context_test(|compile| {
        for initial_page in [false, true] {
            let (mut snapshot, plan, goal) = operation_fixture(compile, "use_on", None, true);
            if initial_page {
                snapshot.seed_chat_modal(-1, vec!["An existing page.".into()]);
                snapshot.seed_chat_options(vec![], 105);
            }
            let mut ledger = None;
            let mut run = with_tick(&snapshot, &mut ledger, 1, |tick| {
                with_step(tick, |cx| plan.begin(cx).unwrap())
            });
            let mut dispatched = None;
            let mut continued = false;
            for tick in 2..=20 {
                assert!(with_tick(&snapshot, &mut ledger, tick, |tick| {
                    with_step(tick, |cx| run.poll(cx))
                })
                .is_pending());
                if matches!(emitted_if_any(&ledger), Some(InteractReq::UseOn { .. })) {
                    dispatched = Some(tick);
                    break;
                }
                if matches!(
                    emitted_if_any(&ledger),
                    Some(InteractReq::ContinueDialog { .. })
                ) && !continued
                {
                    continued = true;
                    accept_last(&mut ledger, tick + 1, true);
                    snapshot.seed_chat_modal(-1, vec![]);
                    snapshot.seed_chat_options(vec![], -1);
                }
            }
            assert_eq!(continued, initial_page);
            let tick = dispatched.expect("UseOn dispatched");
            accept_last(&mut ledger, tick + 1, true);
            snapshot.seed_chat_modal(-1, vec!["You season the sardine.".into()]);
            snapshot.seed_chat_options(vec![], 105);
            set_operation_count(&mut snapshot, goal, 2);
            assert!(
                with_tick(&snapshot, &mut ledger, tick + 1, |tick| {
                    with_step(tick, |cx| run.poll(cx))
                })
                .is_pending(),
                "initial_page={initial_page}: post-use page must prevent settlement"
            );
            assert!(with_tick(&snapshot, &mut ledger, tick + 2, |tick| {
                with_step(tick, |cx| run.poll(cx))
            })
            .is_pending());
            assert!(matches!(
                emitted(&ledger),
                InteractReq::ContinueDialog { .. }
            ));
            accept_last(&mut ledger, tick + 3, true);
            snapshot.seed_chat_modal(-1, vec![]);
            snapshot.seed_chat_options(vec![], -1);
            let mut result = Poll::Pending;
            for tick in tick + 3..=tick + 16 {
                result = with_tick(&snapshot, &mut ledger, tick, |tick| {
                    with_step(tick, |cx| run.poll(cx))
                });
                if result.is_ready() {
                    break;
                }
            }
            assert!(matches!(result, Poll::Ready(Ok(_))));
        }
    });
}

fn emitted_if_any(ledger: &Option<Box<ledger::Ledger>>) -> Option<&InteractReq> {
    ledger
        .as_ref()?
        .outbox
        .last()
        .and_then(|request| match &request.effect {
            HostEffect::Interaction(request) => Some(request),
            _ => None,
        })
}

fn seed_operation_menu(snapshot: &mut GameSnapshot) {
    snapshot.seed_chat_modal(100, vec!["Choose what happens next.".into()]);
    snapshot.seed_chat_options(
        vec![
            api::snapshot::ChatOptionView {
                component_id: 101,
                text: "First choice".into(),
            },
            api::snapshot::ChatOptionView {
                component_id: 102,
                text: "Last choice".into(),
            },
        ],
        -1,
    );
}

#[test]
fn operation_omission_drains_a_continue_page_after_acceptance_without_settling_open() {
    compile_context_test(|compile| {
        for kind in ["interact", "use_on"] {
            for until in [false, true] {
                for root in [-1, 100] {
                    let (mut snapshot, plan, goal) = operation_fixture(compile, kind, None, until);
                    let mut ledger = None;
                    let mut run = with_tick(&snapshot, &mut ledger, 1, |tick| {
                        with_step(tick, |cx| plan.begin(cx).unwrap())
                    });
                    assert!(with_tick(&snapshot, &mut ledger, 2, |tick| {
                        with_step(tick, |cx| run.poll(cx))
                    })
                    .is_pending());
                    accept_last(&mut ledger, 3, true);
                    assert!(
                        with_tick(&snapshot, &mut ledger, 3, |tick| {
                            with_step(tick, |cx| run.poll(cx))
                        })
                        .is_pending(),
                        "{kind}: acceptance alone cannot prove a closed post-action page"
                    );
                    snapshot.seed_chat_modal(root, vec!["The action opened this page.".into()]);
                    snapshot.seed_chat_options(vec![], 105);
                    if until {
                        set_operation_count(&mut snapshot, goal, 2);
                    }
                    for tick in 4..=5 {
                        assert!(
                            with_tick(&snapshot, &mut ledger, tick, |tick| {
                                with_step(tick, |cx| run.poll(cx))
                            })
                            .is_pending(),
                            "{kind}: an open page must prevent settlement"
                        );
                    }
                    assert!(matches!(
                        emitted(&ledger),
                        InteractReq::ContinueDialog { .. }
                    ));
                    accept_last(&mut ledger, 6, true);
                    snapshot.seed_chat_modal(-1, vec![]);
                    snapshot.seed_chat_options(vec![], -1);
                    let mut result = Poll::Pending;
                    for tick in 6..=20 {
                        result = with_tick(&snapshot, &mut ledger, tick, |tick| {
                            with_step(tick, |cx| run.poll(cx))
                        });
                        if result.is_ready() {
                            break;
                        }
                    }
                    assert!(
                        matches!(result, Poll::Ready(Ok(_))),
                        "{kind}, until={until}"
                    );
                }
            }
        }
    });
}

#[test]
fn operation_omission_refuses_a_menu_after_acceptance_without_answering() {
    compile_context_test(|compile| {
        for kind in ["interact", "use_on"] {
            for until in [false, true] {
                let (mut snapshot, plan, goal) = operation_fixture(compile, kind, None, until);
                let mut ledger = None;
                let mut run = with_tick(&snapshot, &mut ledger, 1, |tick| {
                    with_step(tick, |cx| plan.begin(cx).unwrap())
                });
                assert!(with_tick(&snapshot, &mut ledger, 2, |tick| {
                    with_step(tick, |cx| run.poll(cx))
                })
                .is_pending());
                accept_last(&mut ledger, 3, true);
                assert!(with_tick(&snapshot, &mut ledger, 3, |tick| {
                    with_step(tick, |cx| run.poll(cx))
                })
                .is_pending());
                seed_operation_menu(&mut snapshot);
                if until {
                    set_operation_count(&mut snapshot, goal, 2);
                }
                assert!(with_tick(&snapshot, &mut ledger, 4, |tick| {
                    with_step(tick, |cx| run.poll(cx))
                })
                .is_pending());
                assert!(
                    matches!(
                        with_tick(&snapshot, &mut ledger, 5, |tick| {
                            with_step(tick, |cx| run.poll(cx))
                        }),
                        Poll::Ready(Err(ActionError::Failed(reason)))
                            if reason.as_ref() == "dialogue menu requires an explicit answer rule"
                    ),
                    "{kind}, until={until}"
                );
                assert!(ledger.as_ref().unwrap().outbox.iter().all(|request| {
                    !matches!(
                        request.effect,
                        HostEffect::Interaction(InteractReq::Answer { .. })
                    )
                }));
            }
        }
    });
}

#[test]
fn operation_omission_no_page_settles_on_fresh_evidence_not_a_timer() {
    compile_context_test(|compile| {
        for kind in ["interact", "use_on"] {
            let (snapshot, plan, _) = operation_fixture(compile, kind, None, false);
            let mut ledger = None;
            let mut run = with_tick(&snapshot, &mut ledger, 1, |tick| {
                with_step(tick, |cx| plan.begin(cx).unwrap())
            });
            assert!(with_tick(&snapshot, &mut ledger, 2, |tick| {
                with_step(tick, |cx| run.poll(cx))
            })
            .is_pending());
            accept_last(&mut ledger, 3, true);
            for _ in 0..3 {
                assert!(
                    with_tick(&snapshot, &mut ledger, 3, |tick| {
                        with_step(tick, |cx| run.poll(cx))
                    })
                    .is_pending(),
                    "{kind}: duplicate acceptance polls are not fresh page evidence"
                );
            }
            assert!(
                matches!(
                    with_tick(&snapshot, &mut ledger, 4, |tick| {
                        tick.cx.active_now = Duration::from_millis(1_801);
                        with_step(tick, |cx| run.poll(cx))
                    }),
                    Poll::Ready(Ok(_))
                ),
                "{kind}: a fresh closed observation settles without a fixed delay"
            );
            assert_eq!(ledger.as_ref().unwrap().outbox.len(), 1);
        }
    });
}

#[test]
fn adopted_talk_menu_uses_only_authored_answers_without_talk_to() {
    compile_context_test(|compile| {
        for continuation_only in [false, true] {
            for options in [
                serde_json::json!({}),
                serde_json::json!({"prefer":["Missing answer"]}),
                serde_json::json!({"prefer":["First choice"]}),
                serde_json::json!({"choose":1}),
                serde_json::json!({"line_rules":[{"when_line":"what happens next","choose":"First choice"}]}),
            ] {
                let configured = options.get("choose").is_some()
                    || options.get("line_rules").is_some()
                    || options["prefer"] == serde_json::json!(["First choice"]);
                let mut args = options;
                if continuation_only {
                    args["continue_only"] = serde_json::json!(true);
                } else {
                    args["npc"] = serde_json::json!("fred_the_farmer");
                }
                let plan = compile_talk(test_args::<TalkArgs>(args), compile).unwrap();
                let mut snapshot = ready();
                seed_operation_menu(&mut snapshot);
                let mut ledger = None;
                let mut run = with_tick(&snapshot, &mut ledger, 1, |tick| {
                    with_step(tick, |cx| plan.begin(cx).unwrap())
                });
                assert!(with_tick(&snapshot, &mut ledger, 2, |tick| {
                    with_step(tick, |cx| run.poll(cx))
                })
                .is_pending());
                let result = with_tick(&snapshot, &mut ledger, 3, |tick| {
                    with_step(tick, |cx| run.poll(cx))
                });
                if configured {
                    assert!(result.is_pending());
                    assert!(matches!(
                        emitted(&ledger),
                        InteractReq::Answer { option: 1 }
                    ));
                } else {
                    assert!(matches!(result, Poll::Ready(Err(ActionError::Failed(_)))));
                    assert!(
                        ledger
                            .as_ref()
                            .is_none_or(|ledger| ledger.outbox.is_empty()),
                        "an adopted page must not guess or send Talk-to"
                    );
                }
            }
        }
    });
}

#[test]
fn operation_explicit_none_never_adopts_chat_after_acceptance() {
    compile_context_test(|compile| {
        for kind in ["interact", "use_on"] {
            for menu in [false, true] {
                let (mut snapshot, plan, _) =
                    operation_fixture(compile, kind, Some(serde_json::json!("none")), false);
                let mut ledger = None;
                let mut run = with_tick(&snapshot, &mut ledger, 1, |tick| {
                    with_step(tick, |cx| plan.begin(cx).unwrap())
                });
                assert!(with_tick(&snapshot, &mut ledger, 2, |tick| {
                    with_step(tick, |cx| run.poll(cx))
                })
                .is_pending());
                accept_last(&mut ledger, 3, true);
                if menu {
                    seed_operation_menu(&mut snapshot);
                } else {
                    snapshot.seed_chat_modal(-1, vec!["Leave this page alone.".into()]);
                    snapshot.seed_chat_options(vec![], 105);
                }
                assert!(
                    matches!(
                        with_tick(&snapshot, &mut ledger, 3, |tick| {
                            with_step(tick, |cx| run.poll(cx))
                        }),
                        Poll::Ready(Ok(_))
                    ),
                    "{kind}: explicit none settles at acceptance without touching a page"
                );
                assert_eq!(ledger.as_ref().unwrap().outbox.len(), 1);
            }
        }
    });
}
