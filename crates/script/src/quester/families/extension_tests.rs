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
                    answer_once: false,
                    chat_only: false,
                    gap_ticks: None,
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
fn oddenstein_handin_mes_lines_rearm_the_authored_closed_chat_gap() {
    // Sourced from content
    // (area_draynor/scripts/professor_oddenstein.rs2 hand-in): `if_close`
    // at T, then four `mes` lines at T, T+2, T+5, T+8 (`p_delay` 1/2/2/1)
    // before the next chat page opens at T+10, with no inventory, position
    // or animation change in between. Each `mes` line is a game chat
    // message (`type_ == 0`, `username == None`), observed here with a
    // one-tick lag. The authored four-tick gap is re-armed by those lines.
    let mut snapshot = ready();
    snapshot.seed_chat_modal(100, vec!["Have you found anything yet?".into()]);
    snapshot.seed_chat_options(vec![], 101);
    let mut ledger = None;
    let handle = with_tick(&snapshot, &mut ledger, 1, |tick| {
        tick.actions
            .begin::<dialogue::Dialogue>(
                continuation(dialogue::DialogueOptions {
                    gap_ticks: Some(4),
                    ..Default::default()
                }),
                &mut tick.cx,
            )
            .unwrap()
    });
    assert!(with_tick(&snapshot, &mut ledger, 2, |tick| tick
        .actions
        .poll(&handle, &mut tick.cx))
    .is_pending());
    // T = 3 is `if_close`. Engine `mes` ticks are 3, 5, 8, 11; each line is
    // first observable one tick later.
    let line = |sequence| ChatLineView {
        type_: 0,
        username: None,
        text: "The machine hums and shakes.".into(),
        sequence,
    };
    snapshot.seed_chat_modal(-1, vec![]);
    snapshot.seed_chat_options(vec![], -1);
    snapshot.seed_chat_lines(vec![line(1)]);
    for game_tick in [3, 4, 5] {
        assert!(with_tick(&snapshot, &mut ledger, game_tick, |tick| tick
            .actions
            .poll(&handle, &mut tick.cx))
        .is_pending());
    }
    snapshot.seed_chat_lines(vec![line(1), line(2)]);
    for game_tick in [6, 7, 8] {
        assert!(with_tick(&snapshot, &mut ledger, game_tick, |tick| tick
            .actions
            .poll(&handle, &mut tick.cx))
        .is_pending());
    }
    snapshot.seed_chat_lines(vec![line(1), line(2), line(3)]);
    for game_tick in [9, 10, 11] {
        assert!(with_tick(&snapshot, &mut ledger, game_tick, |tick| tick
            .actions
            .poll(&handle, &mut tick.cx))
        .is_pending());
    }
    snapshot.seed_chat_lines(vec![line(1), line(2), line(3), line(4)]);
    assert!(with_tick(&snapshot, &mut ledger, 12, |tick| tick
        .actions
        .poll(&handle, &mut tick.cx))
    .is_pending());
    // T+10 = 13 is the next chat page. It must be drained, not left open.
    snapshot.seed_chat_modal(100, vec!["Ernest thanks you.".into()]);
    snapshot.seed_chat_options(vec![], 101);
    assert!(with_tick(&snapshot, &mut ledger, 13, |tick| tick
        .actions
        .poll(&handle, &mut tick.cx))
    .is_pending());
    assert_eq!(
        ledger
            .as_ref()
            .unwrap()
            .outbox
            .iter()
            .filter(|request| matches!(
                &request.effect,
                HostEffect::Interaction(InteractReq::ContinueDialog { .. })
            ))
            .count(),
        2,
        "the reward page after the mes lines must be continued"
    );
    snapshot.seed_chat_modal(-1, vec![]);
    snapshot.seed_chat_options(vec![], -1);
    // On close at tick 14, the four-tick quiet gap completes at tick 18.
    for game_tick in [14, 15, 16, 17] {
        assert!(with_tick(&snapshot, &mut ledger, game_tick, |tick| tick
            .actions
            .poll(&handle, &mut tick.cx))
        .is_pending());
    }
    assert!(matches!(
        with_tick(&snapshot, &mut ledger, 18, |tick| tick
            .actions
            .poll(&handle, &mut tick.cx)),
        Poll::Ready(Ok(crate::dialogue_outcome::DialogueOutcome::Completed))
    ));
}

#[test]
fn plain_conversation_end_completes_one_quiet_tick_after_close() {
    // The closed chat is observed at tick 3, and the default one-tick quiet
    // gap completes at tick 4.
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
    assert!(with_tick(&snapshot, &mut ledger, 3, |tick| tick
        .actions
        .poll(&handle, &mut tick.cx))
    .is_pending());
    assert!(matches!(
        with_tick(&snapshot, &mut ledger, 4, |tick| tick
            .actions
            .poll(&handle, &mut tick.cx)),
        Poll::Ready(Ok(crate::dialogue_outcome::DialogueOutcome::Completed))
    ));
}

#[test]
fn authored_gap_waits_for_harolds_reopened_page() {
    // Harold Blurberry's death-guard equipment-room script
    // (`death_guard_equiproom.rs2:289-297`) closes chat before ten closed
    // ticks of delay and then opens another page without a re-arm signal.
    let mut snapshot = ready();
    snapshot.seed_chat_modal(100, vec!["Continue?".into()]);
    snapshot.seed_chat_options(vec![], 101);
    let mut ledger = None;
    let handle = with_tick(&snapshot, &mut ledger, 1, |tick| {
        tick.actions
            .begin::<dialogue::Dialogue>(
                continuation(dialogue::DialogueOptions {
                    gap_ticks: Some(10),
                    ..Default::default()
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
        InteractReq::ContinueDialog { component_id: None }
    ));
    snapshot.seed_chat_modal(-1, vec![]);
    snapshot.seed_chat_options(vec![], -1);
    assert!(with_tick(&snapshot, &mut ledger, 3, |tick| tick
        .actions
        .poll(&handle, &mut tick.cx))
    .is_pending());
    for game_tick in 4..=12 {
        assert!(with_tick(&snapshot, &mut ledger, game_tick, |tick| tick
            .actions
            .poll(&handle, &mut tick.cx))
        .is_pending());
    }
    snapshot.seed_chat_modal(100, vec!["The next page.".into()]);
    snapshot.seed_chat_options(vec![], 101);
    assert!(with_tick(&snapshot, &mut ledger, 13, |tick| tick
        .actions
        .poll(&handle, &mut tick.cx))
    .is_pending());
    assert!(matches!(
        emitted(&ledger),
        InteractReq::ContinueDialog { component_id: None }
    ));
}

/// The compiled talk options of the shipped step `step` in the Path `json`,
/// whether a top-level or an acquisition step.
fn shipped_talk_options(json: &str, step: &str) -> dialogue::DialogueOptions {
    let document: crate::quester::path::PathDocument = serde_json::from_str(json).unwrap();
    let acquisitions = document
        .quest
        .iter()
        .flat_map(|quest| quest.acquire.values())
        .flatten();
    let args = document
        .roles
        .iter()
        .flat_map(|role| &role.sequences)
        .flat_map(|sequence| &sequence.steps)
        .chain(acquisitions)
        .find(|candidate| candidate.id.0.as_ref() == step)
        .map(|found| found.args.clone())
        .unwrap_or_else(|| panic!("no step {step}"));
    let args: TalkArgs = serde_json::from_value(args).unwrap();
    compile_dialogue_options(DialogueOptionsDocument {
        prefer: args.prefer,
        choose: args.choose,
        line_rules: args.line_rules,
        strict: args.strict,
        answer_once: args.answer_once,
        gap_ticks: args.gap_ticks,
    })
    .unwrap()
}

fn shipped_death_talk_options(step: &str) -> dialogue::DialogueOptions {
    shipped_talk_options(crate::quester::compile::DEATH_JSON, step)
}

fn chat_page(snapshot: &mut GameSnapshot, root: i32, text: &str) {
    snapshot.seed_chat_modal(root, vec![text.into()]);
    snapshot.seed_chat_options(vec![], root + 1);
}

fn chat_closed(snapshot: &mut GameSnapshot) {
    snapshot.seed_chat_modal(-1, vec![]);
    snapshot.seed_chat_options(vec![], -1);
}

/// Harold's Blurberry route with a Blurberry Special carried, driven with
/// `options` from the first objbox page: after the ale (`ale`) the chat
/// closes for four ticks (`death_guard_equiproom.rs2:318-335`) and reopens
/// on the stage-50 menu; "Can I buy you a drink?" runs `harold_drink2`
/// (`:302-304`), which takes the Blurberry Special, closes the chat for ten
/// ticks (`:280-297`, no re-arm signal) and opens Harold's last page.
/// Returns whether the driver still owned the conversation and continued
/// that last page.
fn drives_harolds_last_page(options: dialogue::DialogueOptions, ale: bool) -> bool {
    let mut snapshot = ready();
    let mut ledger = None;
    let mut tick = 1;
    let first = if ale {
        "You give Harold an Asgarnian Ale."
    } else {
        "Hi."
    };
    chat_page(&mut snapshot, 300, first);
    let handle = with_tick(&snapshot, &mut ledger, tick, |t| {
        t.actions
            .begin::<dialogue::Dialogue>(continuation(options), &mut t.cx)
            .unwrap()
    });
    let mut step = |snapshot: &GameSnapshot, ledger: &mut Option<Box<ledger::Ledger>>| {
        tick += 1;
        with_tick(snapshot, ledger, tick, |t| {
            t.actions.poll(&handle, &mut t.cx)
        })
        .is_pending()
    };
    assert!(step(&snapshot, &mut ledger));
    assert!(matches!(
        emitted(&ledger),
        InteractReq::ContinueDialog { .. }
    ));
    if ale {
        chat_closed(&mut snapshot);
        for _ in 0..4 {
            if !step(&snapshot, &mut ledger) {
                return false;
            }
        }
        chat_page(&mut snapshot, 100, "Arrh. That hit the spot!");
        assert!(step(&snapshot, &mut ledger));
        assert!(matches!(
            emitted(&ledger),
            InteractReq::ContinueDialog { .. }
        ));
    }
    snapshot.seed_chat_modal(200, vec!["Select an Option".into()]);
    snapshot.seed_chat_options(
        [
            "Where were you when you last had the combination?",
            "Would you like to gamble?",
            "Can I buy you a drink?",
        ]
        .into_iter()
        .zip(201..)
        .map(|(text, component_id)| api::snapshot::ChatOptionView {
            component_id,
            text: text.into(),
        })
        .collect(),
        -1,
    );
    assert!(step(&snapshot, &mut ledger));
    assert!(matches!(
        emitted(&ledger),
        InteractReq::Answer { option: 3 }
    ));
    chat_page(&mut snapshot, 300, "You give Harold a Blurberry Special.");
    assert!(step(&snapshot, &mut ledger));
    chat_closed(&mut snapshot);
    for _ in 0..10 {
        if !step(&snapshot, &mut ledger) {
            return false;
        }
    }
    chat_page(&mut snapshot, 100, "Now THAT hit the spot!");
    let continued = step(&snapshot, &mut ledger);
    assert!(matches!(
        emitted(&ledger),
        InteractReq::ContinueDialog { .. }
    ));
    continued
}

#[test]
fn harold_steps_keep_the_blurberry_route_through_harolds_last_page() {
    // `give-ale-harold` reaches the Blurberry chain when the cocktail is
    // already carried; the ale's own four-tick value would end it early.
    let ale = shipped_death_talk_options("give-ale-harold");
    assert!(drives_harolds_last_page(ale.clone(), true));
    assert!(!drives_harolds_last_page(
        dialogue::DialogueOptions {
            gap_ticks: Some(4),
            ..ale
        },
        true
    ));
    assert!(drives_harolds_last_page(
        shipped_death_talk_options("give-blurberry-harold"),
        false
    ));
}

/// A resumed Traiborn hand-in, driven with `options` from "Give 'em here
/// then." with three bones held: each bone closes the chat, takes the bone
/// and sends a message, then waits `p_delay(1)` (`traiborn.rs2:22-29`), and
/// short of the 25th the loop ends on the "That's all of them." page
/// (`:191-199`). Returns whether the driver continued that page.
fn drives_traiborn_bone_loop(options: dialogue::DialogueOptions) -> bool {
    let bones = |count: i32| vec![held(526, "Bones", 0, 1, None); count as usize];
    let given = |count: i32| {
        (1..=count)
            .map(|sequence| api::snapshot::ChatLineView {
                type_: 0,
                username: None,
                text: "You give Traiborn a set of bones.".into(),
                sequence,
            })
            .collect::<Vec<_>>()
    };
    let mut snapshot = ready();
    snapshot.seed_inventory(bones(3), 28);
    chat_page(&mut snapshot, 100, "Give 'em here then.");
    let mut ledger = None;
    let mut tick = 1;
    let handle = with_tick(&snapshot, &mut ledger, tick, |t| {
        t.actions
            .begin::<dialogue::Dialogue>(continuation(options), &mut t.cx)
            .unwrap()
    });
    let mut step = |snapshot: &GameSnapshot, ledger: &mut Option<Box<ledger::Ledger>>| {
        tick += 1;
        with_tick(snapshot, ledger, tick, |t| {
            t.actions.poll(&handle, &mut t.cx)
        })
        .is_pending()
    };
    assert!(step(&snapshot, &mut ledger));
    chat_closed(&mut snapshot);
    for given_so_far in 1..=3 {
        // The bone and its message arrive with the close, then every two ticks.
        snapshot.seed_inventory(bones(3 - given_so_far), 28);
        snapshot.seed_chat_lines(given(given_so_far));
        for _ in 0..2 {
            if !step(&snapshot, &mut ledger) {
                return false;
            }
        }
    }
    chat_page(&mut snapshot, 110, "That's all of them.");
    let continued = step(&snapshot, &mut ledger);
    assert!(matches!(
        emitted(&ledger),
        InteractReq::ContinueDialog { .. }
    ));
    continued
}

#[test]
fn ask_traiborn_keeps_a_resumed_bone_hand_in_through_its_last_page() {
    // `ask-traiborn` runs at ritual progress 3-27 (`traiborn.rs2:1-8`).
    let ask = shipped_talk_options(crate::quester::compile::DEMON_JSON, "ask-traiborn");
    assert!(drives_traiborn_bone_loop(ask.clone()));
    assert!(!drives_traiborn_bone_loop(dialogue::DialogueOptions {
        gap_ticks: None,
        ..ask
    }));
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
        accept_last(&mut ledger, 3, true);
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
    // The posted flood starts on the live player tile.
    snapshot.seed_local_player(local_player(tile(5, 5)));
    let mut wanted = loc(1000, "Roots", "Search");
    wanted.tile = tile(5, 5);
    wanted.distance = 2;
    let mut decoy = wanted.clone();
    decoy.tile = tile(6, 5);
    decoy.distance = 1;
    snapshot.seed_locs(vec![decoy, wanted]);
    let reach_view = wall_door_reach_view();
    let mut ledger = None;
    let none = reach::Avoid::default();
    let area = reach::Area::new(None, 6);
    with_tick_reach(&snapshot, &reach_view, &mut ledger, 1, |tick| {
        assert_eq!(
            reach::choose_loc(
                &tick.cx,
                Some(1000),
                None,
                Some("Search"),
                area,
                None,
                &none
            )
            .select(true)
            .unwrap()
            .tile,
            tile(5, 5)
        );
        assert!(reach::choose_loc(
            &tick.cx,
            Some(1000),
            None,
            Some("Search"),
            area,
            Some(tile(6, 5)),
            &none
        )
        .select(true)
        .is_none());
    });
    with_tick(&snapshot, &mut ledger, 2, |tick| {
        assert!(
            reach::choose_loc(
                &tick.cx,
                Some(1000),
                None,
                Some("Search"),
                area,
                None,
                &none
            )
            .select(true)
            .is_none(),
            "unknown reach must not select a resource candidate"
        );
    });
}

#[test]
fn interact_until_rearms_after_adjacent_instant_progress() {
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
        wait_until: None,
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
    accept_last(&mut ledger, 4, true);
    snapshot.seed_inventory(vec![held(1, "Ore", 0, 1, None)], 28);
    let mut player = local_player(tile(5, 5));
    player.player.actor.animation = 625;
    snapshot.seed_local_player(player);
    assert!(
        with_tick_reach(&snapshot, &reach_view, &mut ledger, 4, |tick| {
            with_step(tick, |cx| run.poll(cx))
        })
        .is_pending()
    );
    assert_eq!(
        ledger.as_ref().unwrap().outbox.last().unwrap().request_id,
        first_request,
        "do not re-click while mining is active"
    );

    snapshot.seed_local_player(local_player(tile(5, 5)));
    assert!(
        with_tick_reach(&snapshot, &reach_view, &mut ledger, 5, |tick| {
            with_step(tick, |cx| run.poll(cx))
        })
        .is_pending()
    );
    assert_ne!(
        ledger.as_ref().unwrap().outbox.last().unwrap().request_id,
        first_request,
        "an adjacent instant target can rearm after a fresh idle tick"
    );

    snapshot.seed_inventory(vec![held(1, "Ore", 0, 2, None)], 28);
    let mut player = local_player(tile(5, 5));
    player.player.actor.animation = 625;
    snapshot.seed_local_player(player);
    accept_last(&mut ledger, 6, true);
    assert!(matches!(
        with_tick_reach(&snapshot, &reach_view, &mut ledger, 6, |tick| {
            with_step(tick, |cx| run.poll(cx))
        }),
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
                        pairs: cx.tick.pairs,
                        quests: cx.quests,
                        progress: cx.progress,
                        required_after: cx.required_after,
                        chat_since: 0,
                        outcome: None,
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
        park_on_timeout: false,
    }));
    assert_eq!(run.child_step_id().unwrap().0.as_ref(), "policy-child");
    let identity = run.child_step_id().unwrap().0.as_ptr();
    assert_eq!(run.child_step_id().unwrap().0.as_ptr(), identity);
    // The child's Ready tick also evaluates its settle and ends the pass on
    // the same snapshot (TICK-FIX #5, E-Q15).
    assert!(matches!(
        with_tick(&snapshot, &mut None, 1, |tick| {
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
    operation_fixture_with_settle(compile, kind, dialogue, until, 60_000)
}

fn operation_fixture_with_settle(
    compile: &CompileContext<'_>,
    kind: &str,
    dialogue: Option<serde_json::Value>,
    until: bool,
    settle_ms: u64,
) -> (GameSnapshot, Arc<dyn StepPlan>, i32) {
    let goal = resolve_obj(compile, "seasoned_sardine").unwrap();
    let mut snapshot = ready();
    snapshot.seed_local_player(local_player(tile(5, 5)));
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
    args["settle_ms"] = serde_json::json!(settle_ms);
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
        // The adopted page is driven on its adopt tick (TICK-FIX #14).
        assert!(matches!(
            with_tick(&snapshot, &mut ledger, 3, |tick| {
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
        let (snapshot, plan, _) = operation_fixture(compile, "use_on", None, false);
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
        assert!(matches!(
            with_tick(&snapshot, &mut ledger, 4, |tick| {
                with_step(tick, |cx| run.poll(cx))
            }),
            Poll::Ready(Ok(_))
        ));
        assert!(matches!(emitted(&ledger), InteractReq::UseOn { .. }));
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
        // The shared driver is begun and continues the page on the same poll
        // (TICK-FIX #14), before any count round.
        assert!(with_tick(&snapshot, &mut ledger, 2, |tick| {
            with_step(tick, |cx| run.poll(cx))
        })
        .is_pending());
        assert!(matches!(
            emitted(&ledger),
            InteractReq::ContinueDialog { .. }
        ));
        assert_eq!(ledger.as_ref().unwrap().outbox.len(), 1);
        accept_last(&mut ledger, 3, true);
        snapshot.seed_chat_modal(-1, vec![]);
        snapshot.seed_chat_options(vec![], -1);
        let mut dispatched = false;
        for tick in 3..=16 {
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
        wait_until: None,
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
fn use_on_until_leaves_post_use_page_for_the_next_owner() {
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
                matches!(
                    with_tick(&snapshot, &mut ledger, tick + 1, |tick| {
                        with_step(tick, |cx| run.poll(cx))
                    }),
                    Poll::Ready(Ok(_))
                ),
                "initial_page={initial_page}: a reached until completes without draining the post-use page"
            );
            assert!(
                matches!(emitted(&ledger), InteractReq::UseOn { .. }),
                "only a pre-dispatch page may be continued"
            );
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
fn operation_omission_leaves_pages_after_reached_until_but_drains_without_until() {
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
                        assert!(
                            matches!(
                                with_tick(&snapshot, &mut ledger, 4, |tick| {
                                    with_step(tick, |cx| run.poll(cx))
                                }),
                                Poll::Ready(Ok(_))
                            ),
                            "{kind}: reached until leaves the open page for the next owner"
                        );
                        assert!(ledger.as_ref().unwrap().outbox.iter().all(|request| {
                            !matches!(
                                request.effect,
                                HostEffect::Interaction(
                                    InteractReq::ContinueDialog { .. } | InteractReq::Answer { .. }
                                )
                            )
                        }));
                        continue;
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
                if until {
                    assert!(
                        matches!(
                            with_tick(&snapshot, &mut ledger, 4, |tick| {
                                with_step(tick, |cx| run.poll(cx))
                            }),
                            Poll::Ready(Ok(_))
                        ),
                        "{kind}: reached until leaves a menu for the next owner"
                    );
                    assert!(ledger.as_ref().unwrap().outbox.iter().all(|request| {
                        !matches!(
                            request.effect,
                            HostEffect::Interaction(
                                InteractReq::ContinueDialog { .. } | InteractReq::Answer { .. }
                            )
                        )
                    }));
                    continue;
                }
                // Adopted and refused on the tick the menu is observed
                // (TICK-FIX #14).
                assert!(
                    matches!(
                        with_tick(&snapshot, &mut ledger, 4, |tick| {
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
fn operation_omission_waits_for_arrival_and_drains_distant_loc_page() {
    compile_context_test(|compile| {
        let plan = compile_interact(
            test_args::<InteractArgs>(serde_json::json!({
                "target": {"loc": "priestperiltempledoorl"},
                "op": "Knock-at",
                "radius": 8
            })),
            compile,
        )
        .unwrap();
        let mut snapshot = ready();
        let mut target = loc(
            resolve_loc(compile, "priestperiltempledoorl").unwrap(),
            "Door",
            "Knock-at",
        );
        target.distance = 8;
        let destination = target.tile;
        let start = tile(destination.x - 8, destination.z);
        snapshot.seed_locs(vec![target]);
        snapshot.seed_local_player(local_player(start));
        let mut ledger = None;
        let mut run = with_tick(&snapshot, &mut ledger, 1, |tick| {
            with_step(tick, |cx| plan.begin(cx).unwrap())
        });
        assert!(with_tick(&snapshot, &mut ledger, 2, |tick| {
            with_step(tick, |cx| run.poll(cx))
        })
        .is_pending());
        assert!(matches!(emitted(&ledger), InteractReq::Loc { .. }));
        accept_last(&mut ledger, 3, true);
        for tick in 3..=6 {
            let mut player = local_player(tile(start.x + (tick as i32 - 2) * 2, start.z));
            player.player.actor.moving = true;
            snapshot.seed_local_player(player);
            assert!(
                with_tick(&snapshot, &mut ledger, tick, |tick| {
                    with_step(tick, |cx| run.poll(cx))
                })
                .is_pending(),
                "a Loc eight tiles away must not settle before arrival"
            );
            assert_eq!(ledger.as_ref().unwrap().outbox.len(), 1);
        }
        snapshot.seed_local_player(local_player(destination));
        snapshot.seed_chat_modal(100, vec!["The arrival script opened this page.".into()]);
        snapshot.seed_chat_options(vec![], 105);
        for tick in 7..=8 {
            assert!(with_tick(&snapshot, &mut ledger, tick, |tick| {
                with_step(tick, |cx| run.poll(cx))
            })
            .is_pending());
        }
        assert!(matches!(
            emitted(&ledger),
            InteractReq::ContinueDialog { .. }
        ));
        accept_last(&mut ledger, 9, true);
        snapshot.seed_chat_modal(-1, vec![]);
        snapshot.seed_chat_options(vec![], -1);
        let mut result = Poll::Pending;
        for tick in 9..=20 {
            result = with_tick(&snapshot, &mut ledger, tick, |tick| {
                with_step(tick, |cx| run.poll(cx))
            });
            if result.is_ready() {
                break;
            }
        }
        assert!(matches!(result, Poll::Ready(Ok(_))));
        assert!(matches!(
            emitted(&ledger),
            InteractReq::ContinueDialog { .. }
        ));
    });
}

#[test]
fn operation_omission_waits_for_fresh_idle_player_before_no_page() {
    compile_context_test(|compile| {
        for kind in ["interact", "use_on"] {
            let (mut snapshot, plan, _) = operation_fixture(compile, kind, None, false);
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
            for (tick, moving, animation) in [(4, true, -1), (5, false, 123)] {
                let active_now = Duration::from_millis(1_800 + tick - 3);
                let mut player = local_player(tile(5, 5));
                player.player.actor.moving = moving;
                player.player.actor.animation = animation;
                snapshot.seed_local_player(player);
                assert!(
                    with_tick(&snapshot, &mut ledger, tick, |tick| {
                        tick.cx.active_now = active_now;
                        with_step(tick, |cx| run.poll(cx))
                    })
                    .is_pending(),
                    "{kind}: movement or primary animation cannot prove completion"
                );
            }
            snapshot.seed_local_player(local_player(tile(5, 5)));
            assert!(matches!(
                with_tick(&snapshot, &mut ledger, 6, |tick| {
                    tick.cx.active_now = Duration::from_millis(1_803);
                    with_step(tick, |cx| run.poll(cx))
                }),
                Poll::Ready(Ok(_))
            ));
            assert_eq!(ledger.as_ref().unwrap().outbox.len(), 1);
        }
    });
}

#[test]
fn interact_omission_settle_timeout_remains_active_during_perpetual_activity() {
    compile_context_test(|compile| {
        for (moving, animation) in [(true, -1), (false, 123)] {
            let (mut snapshot, plan, _) = operation_fixture(compile, "interact", None, false);
            let mut ledger = None;
            let mut run = with_tick(&snapshot, &mut ledger, 1, |tick| {
                with_step(tick, |cx| plan.begin(cx).unwrap())
            });
            assert!(with_tick(&snapshot, &mut ledger, 2, |tick| {
                tick.cx.active_now = Duration::ZERO;
                with_step(tick, |cx| run.poll(cx))
            })
            .is_pending());
            accept_last(&mut ledger, 3, true);
            assert!(with_tick(&snapshot, &mut ledger, 3, |tick| {
                with_step(tick, |cx| run.poll(cx))
            })
            .is_pending());
            let mut player = local_player(tile(5, 5));
            player.player.actor.moving = moving;
            player.player.actor.animation = animation;
            snapshot.seed_local_player(player);
            assert!(
                matches!(
                    with_tick(&snapshot, &mut ledger, 4, |tick| {
                        tick.cx.active_now = Duration::from_millis(60_001);
                        with_step(tick, |cx| run.poll(cx))
                    }),
                    Poll::Ready(Err(ActionError::Failed(reason)))
                        if reason.as_ref() == "interact settle timeout"
                ),
                "moving={moving}, animation={animation}: activity must not bypass the settle deadline"
            );
        }
    });
}

#[test]
fn interact_omission_requires_scene_activity_before_fresh_idle_completion() {
    compile_context_test(|compile| {
        let plan = compile_interact(
            test_args::<InteractArgs>(serde_json::json!({
                "target": {"loc": "priestperiltempledoorl"},
                "op": "Knock-at",
                "radius": 8
            })),
            compile,
        )
        .unwrap();
        let mut snapshot = ready();
        let mut target = loc(
            resolve_loc(compile, "priestperiltempledoorl").unwrap(),
            "Door",
            "Knock-at",
        );
        target.distance = 8;
        let destination = target.tile;
        let start = tile(destination.x - 8, destination.z);
        snapshot.seed_locs(vec![target]);
        snapshot.seed_local_player(local_player(start));
        let mut ledger = None;
        let mut run = with_tick(&snapshot, &mut ledger, 1, |tick| {
            with_step(tick, |cx| plan.begin(cx).unwrap())
        });
        assert!(with_tick(&snapshot, &mut ledger, 2, |tick| {
            with_step(tick, |cx| run.poll(cx))
        })
        .is_pending());
        assert!(matches!(emitted(&ledger), InteractReq::Loc { .. }));
        accept_last(&mut ledger, 3, true);
        assert!(with_tick(&snapshot, &mut ledger, 3, |tick| {
            with_step(tick, |cx| run.poll(cx))
        })
        .is_pending());
        assert!(
            with_tick(&snapshot, &mut ledger, 4, |tick| {
                with_step(tick, |cx| run.poll(cx))
            })
            .is_pending(),
            "a distant scene target must not settle on an idle tick before click activity"
        );
        let mut moving_player = local_player(tile(start.x + 2, start.z));
        moving_player.player.actor.moving = true;
        snapshot.seed_local_player(moving_player);
        assert!(with_tick(&snapshot, &mut ledger, 5, |tick| {
            with_step(tick, |cx| run.poll(cx))
        })
        .is_pending());
        snapshot.seed_local_player(local_player(destination));
        assert!(matches!(
            with_tick(&snapshot, &mut ledger, 6, |tick| {
                with_step(tick, |cx| run.poll(cx))
            }),
            Poll::Ready(Ok(_))
        ));
    });
}

#[test]
fn use_on_omission_completes_adjacent_instant_loc_without_activity() {
    compile_context_test(|compile| {
        let plan = compile_use_on(
            test_args::<UseOnArgs>(serde_json::json!({
                "item": "doogleleaves",
                "target": {"loc": "priestperiltempledoorl"}
            })),
            compile,
        )
        .unwrap();
        let mut snapshot = ready();
        let mut target = loc(
            resolve_loc(compile, "priestperiltempledoorl").unwrap(),
            "Door",
            "Knock-at",
        );
        target.tile = tile(6, 5);
        target.distance = 1;
        snapshot.seed_locs(vec![target]);
        snapshot.seed_local_player(local_player(tile(5, 5)));
        snapshot.seed_inventory(
            vec![held(
                resolve_obj(compile, "doogleleaves").unwrap(),
                "Doogle leaves",
                1,
                3,
                None,
            )],
            28,
        );
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
    });
}

#[test]
fn operation_omission_leaves_unsupported_main_modal_untouched() {
    compile_context_test(|compile| {
        let ui = compile.selected.dialogue_ui().unwrap();
        let unsupported = [ui.scroll_root, ui.book_root].into_iter().max().unwrap() + 100;
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
                snapshot.seed_main_modal(unsupported, vec![]);
                if until {
                    set_operation_count(&mut snapshot, goal, 2);
                }
                assert!(
                    matches!(
                        with_tick(&snapshot, &mut ledger, 4, |tick| {
                            with_step(tick, |cx| run.poll(cx))
                        }),
                        Poll::Ready(Ok(_))
                    ),
                    "{kind}, until={until}: an unrelated main interface is not a dialogue page"
                );
                assert_eq!(ledger.as_ref().unwrap().outbox.len(), 1);
            }
        }
    });
}

#[test]
fn reach_scene_operations_require_accepted_receipt() {
    for kind in [
        reach::ReachKind::Loc {
            id: Some(1000),
            name: None,
        },
        reach::ReachKind::Name {
            name: Arc::from("Door"),
        },
        reach::ReachKind::Npc {
            id: 1000,
            name: Arc::from("Guard"),
        },
    ] {
        for accepted in [false, true] {
            let mut snapshot = ready();
            snapshot.seed_locs(vec![loc(1000, "Door", "Open")]);
            snapshot.seed_npcs(vec![api::snapshot::NpcView {
                index: 42,
                r#type: Some(1000),
                name: Some("Guard".into()),
                actions: vec![Some("Open".into())],
                tile: tile(3167, 3308),
                distance: 3,
                animation: -1,
                animation_frame: 0,
                pose_animation: -1,
                orientation: 0,
                target_orientation: 0,
                overhead_text: None,
                spot_animation: -1,
                spot_animation_stamp: 0,
                health: 10,
                total_health: 10,
                face_entity: -1,
                target: None,
                moving: false,
                running: false,
                in_combat: false,
                level: 3,
                size: 1,
                network: tile(3167, 3308),
                x: 0,
                z: 0,
                yaw: 0,
            }]);
            let mut ledger = None;
            let handle = with_tick(&snapshot, &mut ledger, 1, |tick| {
                tick.actions
                    .begin::<reach::Reach>(
                        reach::ReachArgs {
                            kind: kind.clone(),
                            op: Arc::from("Open"),
                            anchor: None,
                            radius: 8,
                            wait_if_missing: false,
                            target_tile: None,
                            reachable_only: false,
                        },
                        &mut tick.cx,
                    )
                    .unwrap()
            });
            for tick in 2..=4 {
                assert!(
                    with_tick(&snapshot, &mut ledger, tick, |tick| {
                        tick.actions.poll(&handle, &mut tick.cx)
                    })
                    .is_pending(),
                    "scene operations need their own receipt, not an eligible poll"
                );
            }
            accept_last(&mut ledger, 5, accepted);
            assert!(matches!(
                with_tick(&snapshot, &mut ledger, 5, |tick| {
                    tick.actions.poll(&handle, &mut tick.cx)
                }),
                Poll::Ready(Ok(result)) if result == accepted
            ));
        }
    }
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
                // The adopted page is answered (or refused) on the tick the
                // Dialogue begins (TICK-FIX #14).
                let result = with_tick(&snapshot, &mut ledger, 2, |tick| {
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
fn loc_interact_plan(settle_ms: u64, until: Option<(i32, s2::QuantityPlan)>) -> InteractPlan {
    InteractPlan {
        kind: reach::ReachKind::Loc {
            id: Some(1000),
            name: None,
        },
        op: Arc::from("Mine"),
        tile: None,
        radius: 10,
        wait_if_missing: false,
        settle_ms: Some(settle_ms),
        ambiguous: false,
        default_dialogue: true,
        dialogue_options: None,
        until,
        target_tile: None,
        reachable_only: false,
        wait_until: None,
    }
}

#[test]
fn interact_wait_until_gates_initial_and_refused_rounds() {
    use std::sync::atomic::{AtomicU8, Ordering};
    struct Gate(Arc<AtomicU8>);
    impl PredicatePlan for Gate {
        fn evaluate(&self, _: &PredicateContext<'_, '_>) -> Truth {
            match self.0.load(Ordering::Relaxed) {
                0 => Truth::Unknown,
                1 => Truth::False,
                _ => Truth::True,
            }
        }
    }
    let mut snapshot = ready();
    snapshot.seed_local_player(local_player(tile(5, 5)));
    snapshot.seed_inventory(vec![held(1, "Ore", 0, 0, None)], 28);
    let mut resource = loc(1000, "Rock", "Mine");
    resource.tile = tile(5, 5);
    resource.distance = 1;
    snapshot.seed_locs(vec![resource]);
    let gate = Arc::new(AtomicU8::new(0));
    let mut plan = loc_interact_plan(120_000, Some((1, s2::QuantityPlan::Fixed(1))));
    plan.wait_until = Some(Arc::new(Gate(Arc::clone(&gate))));
    let reach_view = super::wall_door_reach_view();
    let mut ledger = None;
    let mut run = with_tick_reach(&snapshot, &reach_view, &mut ledger, 1, |tick| {
        with_step(tick, |cx| plan.begin(cx).unwrap())
    });
    for tick in 2..=3 {
        gate.store((tick - 2) as u8, Ordering::Relaxed);
        assert!(
            with_tick_reach(&snapshot, &reach_view, &mut ledger, tick, |tick| {
                with_step(tick, |cx| run.poll(cx))
            })
            .is_pending()
        );
        assert!(ledger
            .as_ref()
            .is_none_or(|ledger| ledger.outbox.is_empty()));
    }
    gate.store(2, Ordering::Relaxed);
    assert!(
        with_tick_reach(&snapshot, &reach_view, &mut ledger, 4, |tick| {
            with_step(tick, |cx| run.poll(cx))
        })
        .is_pending()
    );
    accept_last(&mut ledger, 5, true);
    let first_request = ledger
        .as_ref()
        .unwrap()
        .outbox
        .last()
        .unwrap()
        .authority()
        .request_id();
    assert!(
        with_tick_reach(&snapshot, &reach_view, &mut ledger, 5, |tick| {
            with_step(tick, |cx| run.poll(cx))
        })
        .is_pending()
    );
    gate.store(1, Ordering::Relaxed);
    for tick in 6..=30 {
        let mut player = local_player(tile(5, 5));
        player.player.actor.animation = if tick == 6 { 1 } else { -1 };
        snapshot.seed_local_player(player);
        assert!(
            with_tick_reach(&snapshot, &reach_view, &mut ledger, tick, |tick| {
                with_step(tick, |cx| run.poll(cx))
            })
            .is_pending()
        );
        assert_eq!(
            ledger.as_ref().unwrap().outbox.len(),
            1,
            "no repeated op while blocked"
        );
        assert_eq!(
            ledger
                .as_ref()
                .unwrap()
                .outbox
                .last()
                .unwrap()
                .authority()
                .request_id(),
            first_request
        );
    }
    gate.store(2, Ordering::Relaxed);
    assert!(
        with_tick_reach(&snapshot, &reach_view, &mut ledger, 31, |tick| {
            with_step(tick, |cx| run.poll(cx))
        })
        .is_pending()
    );
    assert_ne!(
        ledger
            .as_ref()
            .unwrap()
            .outbox
            .last()
            .unwrap()
            .authority()
            .request_id(),
        first_request,
        "same step retries after clearance"
    );
    accept_last(&mut ledger, 32, true);
    snapshot.seed_inventory(vec![held(1, "Ore", 0, 1, None)], 28);
    assert!(matches!(
        with_tick_reach(&snapshot, &reach_view, &mut ledger, 32, |tick| {
            with_step(tick, |cx| run.poll(cx))
        }),
        Poll::Ready(Ok(_))
    ));
}

#[test]
fn interact_reached_until_completes_before_optional_scene_gate() {
    compile_context_test(|compile| {
        // Once the count is reached, omitted dialogue completes immediately even
        // when the player is still moving.
        let (mut snapshot, plan, goal) = operation_fixture(compile, "interact", None, true);
        let mut ledger = None;
        let mut run = with_tick(&snapshot, &mut ledger, 1, |tick| {
            with_step(tick, |cx| plan.begin(cx).unwrap())
        });
        assert!(with_tick(&snapshot, &mut ledger, 2, |tick| {
            tick.cx.active_now = Duration::from_millis(1_000);
            with_step(tick, |cx| run.poll(cx))
        })
        .is_pending());
        accept_last(&mut ledger, 3, true);
        set_operation_count(&mut snapshot, goal, 2);
        let mut moving = local_player(tile(5, 5));
        moving.player.actor.moving = true;
        snapshot.seed_local_player(moving);
        assert!(matches!(
            with_tick(&snapshot, &mut ledger, 3, |tick| {
                tick.cx.active_now = Duration::from_millis(1_600);
                with_step(tick, |cx| run.poll(cx))
            }),
            Poll::Ready(Ok(_))
        ));
        assert_eq!(ledger.as_ref().unwrap().outbox.len(), 1);

        // A scene action that reaches its goal on acceptance also completes on
        // that poll without waiting for movement, animation, or an idle tick.
        let mut snapshot = ready();
        snapshot.seed_local_player(local_player(tile(5, 5)));
        snapshot.seed_inventory(vec![held(1, "Ore", 0, 0, None)], 28);
        let mut resource = loc(1000, "Rock", "Mine");
        resource.tile = tile(5, 5);
        resource.distance = 1;
        snapshot.seed_locs(vec![resource]);
        let plan = loc_interact_plan(5_000, Some((1, s2::QuantityPlan::Fixed(2))));
        let reach_view = super::wall_door_reach_view();
        let mut ledger = None;
        let mut run = with_tick_reach(&snapshot, &reach_view, &mut ledger, 1, |tick| {
            with_step(tick, |cx| plan.begin(cx).unwrap())
        });
        assert!(
            with_tick_reach(&snapshot, &reach_view, &mut ledger, 2, |tick| {
                tick.cx.active_now = Duration::from_millis(1_000);
                with_step(tick, |cx| run.poll(cx))
            })
            .is_pending()
        );
        accept_last(&mut ledger, 3, true);
        snapshot.seed_inventory(vec![held(1, "Ore", 0, 2, None)], 28);
        assert!(matches!(
            with_tick_reach(&snapshot, &reach_view, &mut ledger, 3, |tick| {
                tick.cx.active_now = Duration::from_millis(1_600);
                with_step(tick, |cx| run.poll(cx))
            }),
            Poll::Ready(Ok(_))
        ));
        assert_eq!(ledger.as_ref().unwrap().outbox.len(), 1);
    });
}

#[test]
fn use_on_reached_until_completes_before_optional_gate() {
    compile_context_test(|compile| {
        // Probe C: item use reaches its goal after acceptance while the player
        // remains in motion, without waiting for the omitted-dialogue gate.
        let (mut snapshot, plan, goal) = operation_fixture(compile, "use_on", None, true);
        let mut ledger = None;
        let mut run = with_tick(&snapshot, &mut ledger, 1, |tick| {
            with_step(tick, |cx| plan.begin(cx).unwrap())
        });
        assert!(with_tick(&snapshot, &mut ledger, 2, |tick| {
            tick.cx.active_now = Duration::from_millis(1_000);
            with_step(tick, |cx| run.poll(cx))
        })
        .is_pending());
        accept_last(&mut ledger, 3, true);
        set_operation_count(&mut snapshot, goal, 2);
        let mut moving = local_player(tile(5, 5));
        moving.player.actor.moving = true;
        snapshot.seed_local_player(moving);
        let mut outcome = with_tick(&snapshot, &mut ledger, 3, |tick| {
            tick.cx.active_now = Duration::from_millis(1_600);
            with_step(tick, |cx| run.poll(cx))
        });
        if outcome.is_pending() {
            outcome = with_tick(&snapshot, &mut ledger, 4, |tick| {
                tick.cx.active_now = Duration::from_millis(2_200);
                with_step(tick, |cx| run.poll(cx))
            });
        }
        assert!(
            matches!(outcome, Poll::Ready(Ok(_))),
            "a reached count must bypass the omitted-dialogue gate while the player is moving"
        );
        assert_eq!(ledger.as_ref().unwrap().outbox.len(), 1);
    });
}

#[test]
fn reached_until_never_skips_the_settle_deadline() {
    compile_context_test(|compile| {
        for (kind, timeout) in [
            ("interact", "interact settle timeout"),
            ("use_on", "use_on timeout"),
        ] {
            let (mut snapshot, plan, goal) = operation_fixture(compile, kind, None, true);
            let mut ledger = None;
            let mut run = with_tick(&snapshot, &mut ledger, 1, |tick| {
                with_step(tick, |cx| plan.begin(cx).unwrap())
            });
            assert!(with_tick(&snapshot, &mut ledger, 2, |tick| {
                tick.cx.active_now = Duration::from_millis(1_000);
                with_step(tick, |cx| run.poll(cx))
            })
            .is_pending());
            accept_last(&mut ledger, 3, true);
            let mut moving = local_player(tile(5, 5));
            moving.player.actor.moving = true;
            snapshot.seed_local_player(moving);
            assert!(with_tick(&snapshot, &mut ledger, 3, |tick| {
                tick.cx.active_now = Duration::from_millis(1_600);
                with_step(tick, |cx| run.poll(cx))
            })
            .is_pending());
            set_operation_count(&mut snapshot, goal, 2);
            assert!(
                matches!(
                    with_tick(&snapshot, &mut ledger, 4, |tick| {
                        tick.cx.active_now = Duration::from_millis(61_001);
                        with_step(tick, |cx| run.poll(cx))
                    }),
                    Poll::Ready(Err(ActionError::Failed(reason)))
                        if reason.as_ref() == timeout
                ),
                "{kind}: a reached goal must not extend settle_ms"
            );
            assert_eq!(ledger.as_ref().unwrap().outbox.len(), 1);
        }
    });
}

#[test]
fn adjacent_instant_interact_completes_on_fresh_idle_without_activity() {
    let mut snapshot = ready();
    snapshot.seed_local_player(local_player(tile(5, 5)));
    let mut resource = loc(1000, "Rock", "Mine");
    resource.tile = tile(5, 5);
    resource.distance = 1;
    snapshot.seed_locs(vec![resource]);
    let plan = loc_interact_plan(5_000, None);
    let reach_view = super::wall_door_reach_view();
    let mut ledger = None;
    let mut run = with_tick_reach(&snapshot, &reach_view, &mut ledger, 1, |tick| {
        with_step(tick, |cx| plan.begin(cx).unwrap())
    });
    assert!(
        with_tick_reach(&snapshot, &reach_view, &mut ledger, 2, |tick| {
            tick.cx.active_now = Duration::from_millis(1_000);
            with_step(tick, |cx| run.poll(cx))
        })
        .is_pending()
    );
    accept_last(&mut ledger, 3, true);
    assert!(
        with_tick_reach(&snapshot, &reach_view, &mut ledger, 3, |tick| {
            tick.cx.active_now = Duration::from_millis(1_600);
            with_step(tick, |cx| run.poll(cx))
        })
        .is_pending()
    );
    assert!(matches!(
        with_tick_reach(&snapshot, &reach_view, &mut ledger, 4, |tick| {
            tick.cx.active_now = Duration::from_millis(2_200);
            with_step(tick, |cx| run.poll(cx))
        }),
        Poll::Ready(Ok(_))
    ));
    assert_eq!(ledger.as_ref().unwrap().outbox.len(), 1);
}

#[test]
fn distant_instant_interact_still_requires_activity_then_fresh_idle() {
    let mut snapshot = ready();
    snapshot.seed_local_player(local_player(tile(5, 5)));
    let mut resource = loc(1000, "Rock", "Mine");
    resource.tile = tile(13, 5);
    resource.distance = 8;
    snapshot.seed_locs(vec![resource]);
    let plan = loc_interact_plan(5_000, None);
    let reach_view = super::wall_door_reach_view();
    let mut ledger = None;
    let mut run = with_tick_reach(&snapshot, &reach_view, &mut ledger, 1, |tick| {
        with_step(tick, |cx| plan.begin(cx).unwrap())
    });
    assert!(
        with_tick_reach(&snapshot, &reach_view, &mut ledger, 2, |tick| {
            tick.cx.active_now = Duration::from_millis(1_000);
            with_step(tick, |cx| run.poll(cx))
        })
        .is_pending()
    );
    accept_last(&mut ledger, 3, true);
    assert!(
        with_tick_reach(&snapshot, &reach_view, &mut ledger, 3, |tick| {
            tick.cx.active_now = Duration::from_millis(1_600);
            with_step(tick, |cx| run.poll(cx))
        })
        .is_pending()
    );
    assert!(
        with_tick_reach(&snapshot, &reach_view, &mut ledger, 4, |tick| {
            tick.cx.active_now = Duration::from_millis(2_200);
            with_step(tick, |cx| run.poll(cx))
        })
        .is_pending(),
        "a fresh idle tick alone is insufficient for a distant scene target"
    );
    let mut moving = local_player(tile(5, 5));
    moving.player.actor.moving = true;
    snapshot.seed_local_player(moving);
    assert!(
        with_tick_reach(&snapshot, &reach_view, &mut ledger, 5, |tick| {
            tick.cx.active_now = Duration::from_millis(2_800);
            with_step(tick, |cx| run.poll(cx))
        })
        .is_pending()
    );
    snapshot.seed_local_player(local_player(tile(5, 5)));
    assert!(matches!(
        with_tick_reach(&snapshot, &reach_view, &mut ledger, 6, |tick| {
            tick.cx.active_now = Duration::from_millis(3_400);
            with_step(tick, |cx| run.poll(cx))
        }),
        Poll::Ready(Ok(_))
    ));
    assert_eq!(ledger.as_ref().unwrap().outbox.len(), 1);
}

fn instant_loc_plan(id: i32, op: &str) -> InteractPlan {
    InteractPlan {
        kind: reach::ReachKind::Loc {
            id: Some(id),
            name: None,
        },
        op: Arc::from(op),
        ..loc_interact_plan(20_000, None)
    }
}

fn trapdoor(id: i32, ops: &[&str], at: WorldTile, distance: i32) -> LocView {
    let mut trapdoor = loc(id, "Trapdoor", ops[0]);
    trapdoor.actions = ops.iter().map(|op| Some((*op).into())).collect();
    trapdoor.tile = at;
    trapdoor.distance = distance;
    trapdoor
}

/// Clicks `plan` on tick 2, applies `server` before the receipt is read on
/// tick 3, then polls idle ticks 4..=8. Returns the first ready tick.
fn instant_loc_completion_tick(
    mut snapshot: GameSnapshot,
    plan: &InteractPlan,
    server: impl FnOnce(&mut GameSnapshot),
) -> Option<u64> {
    let reach_view = super::wall_door_reach_view();
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
    assert!(matches!(emitted(&ledger), InteractReq::Loc { .. }));
    server(&mut snapshot);
    accept_last(&mut ledger, 3, true);
    assert!(
        with_tick_reach(&snapshot, &reach_view, &mut ledger, 3, |tick| {
            with_step(tick, |cx| run.poll(cx))
        })
        .is_pending(),
        "the acceptance tick itself is not a fresh idle tick"
    );
    let ready = (4..=8).find(|&tick| {
        match with_tick_reach(&snapshot, &reach_view, &mut ledger, tick, |cx| {
            with_step(cx, |cx| run.poll(cx))
        }) {
            Poll::Pending => false,
            Poll::Ready(result) => {
                assert!(result.is_ok(), "tick {tick}: {:?}", result.err());
                true
            }
        }
    });
    assert_eq!(ledger.as_ref().unwrap().outbox.len(), 1, "no re-click");
    ready
}

#[test]
fn adjacent_loc_transformed_on_acceptance_completes_on_next_fresh_idle() {
    // Trapdoor Open: by the time the receipt is read the server has already
    // replaced `1568 [Open]` with `1570 [Climb-down/Close]`.
    let mut snapshot = ready();
    snapshot.seed_local_player(local_player(tile(5, 5)));
    snapshot.seed_locs(vec![trapdoor(1568, &["Open"], tile(5, 6), 1)]);
    let ready_tick =
        instant_loc_completion_tick(snapshot, &instant_loc_plan(1568, "Open"), |snapshot| {
            snapshot.seed_locs(vec![trapdoor(
                1570,
                &["Climb-down", "Close"],
                tile(5, 6),
                1,
            )]);
        });
    assert_eq!(ready_tick, Some(4));
}

#[test]
fn adjacent_teleport_before_acceptance_completes_on_next_fresh_idle() {
    // Climb-down: the receipt is read after the scene rebuild, in the crypt,
    // where the clicked trapdoor no longer exists.
    let mut snapshot = ready();
    snapshot.seed_local_player(local_player(tile(5, 5)));
    snapshot.seed_locs(vec![trapdoor(
        1570,
        &["Climb-down", "Close"],
        tile(5, 6),
        1,
    )]);
    let ready_tick = instant_loc_completion_tick(
        snapshot,
        &instant_loc_plan(1570, "Climb-down"),
        |snapshot| {
            snapshot.seed_local_player(local_player(tile(5, 6405)));
            snapshot.seed_locs(vec![]);
        },
    );
    assert_eq!(ready_tick, Some(4));
}

#[test]
fn distant_click_never_completes_on_unrelated_toggle_or_teleport() {
    // A click on a target out of range when chosen still needs post-acceptance
    // movement or animation, even if the loc changes or the player teleports.
    for teleport in [false, true] {
        let mut snapshot = ready();
        snapshot.seed_local_player(local_player(tile(5, 5)));
        snapshot.seed_locs(vec![trapdoor(1568, &["Open"], tile(13, 5), 8)]);
        let ready_tick =
            instant_loc_completion_tick(snapshot, &instant_loc_plan(1568, "Open"), |snapshot| {
                snapshot.seed_locs(vec![trapdoor(
                    1570,
                    &["Climb-down", "Close"],
                    tile(13, 5),
                    if teleport { 6400 } else { 8 },
                )]);
                if teleport {
                    snapshot.seed_local_player(local_player(tile(13, 6405)));
                }
            });
        assert_eq!(ready_tick, None, "teleport={teleport}");
    }
}

#[test]
fn interact_until_leaves_a_preexisting_page_for_the_next_owner() {
    compile_context_test(|compile| {
        let (mut snapshot, plan, goal) = operation_fixture(compile, "interact", None, true);
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
        assert!(matches!(emitted(&ledger), InteractReq::Held { .. }));
        accept_last(&mut ledger, 3, true);
        set_operation_count(&mut snapshot, goal, 2);

        assert!(matches!(
            with_tick(&snapshot, &mut ledger, 4, |tick| {
                with_step(tick, |cx| run.poll(cx))
            }),
            Poll::Ready(Ok(_))
        ));
        assert_eq!(ledger.as_ref().unwrap().outbox.len(), 1);
        assert!(matches!(emitted(&ledger), InteractReq::Held { .. }));
    });
}

#[test]
fn interact_until_leaves_a_later_page_during_the_dialogue_gap() {
    compile_context_test(|compile| {
        let (mut snapshot, plan, goal) = operation_fixture(compile, "interact", None, true);
        let mut ledger = None;
        let mut run = with_tick(&snapshot, &mut ledger, 1, |tick| {
            with_step(tick, |cx| plan.begin(cx).unwrap())
        });

        assert!(with_tick(&snapshot, &mut ledger, 2, |tick| {
            with_step(tick, |cx| run.poll(cx))
        })
        .is_pending());
        accept_last(&mut ledger, 3, true);
        snapshot.seed_chat_modal(-1, vec!["First page.".into()]);
        snapshot.seed_chat_options(vec![], 105);
        assert!(with_tick(&snapshot, &mut ledger, 3, |tick| {
            with_step(tick, |cx| run.poll(cx))
        })
        .is_pending());
        assert!(with_tick(&snapshot, &mut ledger, 4, |tick| {
            with_step(tick, |cx| run.poll(cx))
        })
        .is_pending());
        assert!(with_tick(&snapshot, &mut ledger, 5, |tick| {
            with_step(tick, |cx| run.poll(cx))
        })
        .is_pending());
        assert!(matches!(
            emitted(&ledger),
            InteractReq::ContinueDialog { .. }
        ));
        accept_last(&mut ledger, 6, true);
        snapshot.seed_chat_modal(-1, vec![]);
        snapshot.seed_chat_options(vec![], -1);
        for tick in 7..=12 {
            // A scripted `mes` line lands mid-gap and spends one finite
            // chat re-arm (Oddenstein's hand-in keeps the four-tick gap
            // alive the same way), so the later page still opens inside it.
            if tick == 11 {
                snapshot.seed_chat_lines(vec![ChatLineView {
                    type_: 0,
                    username: None,
                    text: "The machine hums and shakes.".into(),
                    sequence: 1,
                }]);
            }
            assert!(with_tick(&snapshot, &mut ledger, tick, |tick| {
                with_step(tick, |cx| run.poll(cx))
            })
            .is_pending());
        }

        snapshot.seed_chat_modal(-1, vec!["Later page.".into()]);
        snapshot.seed_chat_options(vec![], 105);
        set_operation_count(&mut snapshot, goal, 2);
        assert!(matches!(
            with_tick(&snapshot, &mut ledger, 13, |tick| {
                with_step(tick, |cx| run.poll(cx))
            }),
            Poll::Ready(Ok(_))
        ));
        assert_eq!(
            ledger
                .as_ref()
                .unwrap()
                .outbox
                .iter()
                .filter(|request| matches!(
                    &request.effect,
                    HostEffect::Interaction(InteractReq::ContinueDialog { .. })
                ))
                .count(),
            1,
            "the page that opens after the goal belongs to the next owner"
        );
    });
}

#[test]
fn interact_settle_deadline_wins_when_a_page_opens_late() {
    compile_context_test(|compile| {
        let (mut snapshot, plan, goal) =
            operation_fixture_with_settle(compile, "interact", None, true, 3_000);
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
        assert!(with_tick(&snapshot, &mut ledger, 4, |tick| {
            with_step(tick, |cx| run.poll(cx))
        })
        .is_pending());
        set_operation_count(&mut snapshot, goal, 2);
        snapshot.seed_chat_modal(-1, vec!["Late page.".into()]);
        snapshot.seed_chat_options(vec![], 105);

        assert!(matches!(
            with_tick(&snapshot, &mut ledger, 8, |tick| {
                with_step(tick, |cx| run.poll(cx))
            }),
            Poll::Ready(Err(ActionError::Failed(reason)))
                if reason.as_ref() == "interact settle timeout"
        ));
        assert_eq!(ledger.as_ref().unwrap().outbox.len(), 1);
        assert!(matches!(emitted(&ledger), InteractReq::Held { .. }));
    });
}

#[test]
fn interact_settle_deadline_bounds_an_active_dialogue_drain() {
    compile_context_test(|compile| {
        let (mut snapshot, plan, _) =
            operation_fixture_with_settle(compile, "interact", None, true, 3_000);
        let mut ledger = None;
        let mut run = with_tick(&snapshot, &mut ledger, 1, |tick| {
            with_step(tick, |cx| plan.begin(cx).unwrap())
        });

        assert!(with_tick(&snapshot, &mut ledger, 2, |tick| {
            with_step(tick, |cx| run.poll(cx))
        })
        .is_pending());
        accept_last(&mut ledger, 3, true);
        snapshot.seed_chat_modal(-1, vec!["Page to drain.".into()]);
        snapshot.seed_chat_options(vec![], 105);
        assert!(with_tick(&snapshot, &mut ledger, 4, |tick| {
            with_step(tick, |cx| run.poll(cx))
        })
        .is_pending());
        assert!(with_tick(&snapshot, &mut ledger, 5, |tick| {
            with_step(tick, |cx| run.poll(cx))
        })
        .is_pending());
        assert!(with_tick(&snapshot, &mut ledger, 6, |tick| {
            with_step(tick, |cx| run.poll(cx))
        })
        .is_pending());
        assert!(matches!(
            emitted(&ledger),
            InteractReq::ContinueDialog { .. }
        ));

        assert!(matches!(
            with_tick(&snapshot, &mut ledger, 7, |tick| {
                with_step(tick, |cx| run.poll(cx))
            }),
            Poll::Ready(Err(ActionError::Failed(reason)))
                if reason.as_ref() == "interact settle timeout"
        ));
    });
}

#[test]
fn interact_until_completes_while_dialogue_driver_is_active() {
    compile_context_test(|compile| {
        let (mut snapshot, plan, goal) =
            operation_fixture_with_settle(compile, "interact", None, true, 60_000);
        let mut ledger = None;
        let mut run = with_tick(&snapshot, &mut ledger, 1, |tick| {
            with_step(tick, |cx| plan.begin(cx).unwrap())
        });

        assert!(with_tick(&snapshot, &mut ledger, 2, |tick| {
            with_step(tick, |cx| run.poll(cx))
        })
        .is_pending());
        accept_last(&mut ledger, 3, true);
        snapshot.seed_chat_modal(-1, vec!["Page to continue.".into()]);
        snapshot.seed_chat_options(vec![], 105);
        assert!(with_tick(&snapshot, &mut ledger, 4, |tick| {
            with_step(tick, |cx| run.poll(cx))
        })
        .is_pending());
        assert!(with_tick(&snapshot, &mut ledger, 5, |tick| {
            with_step(tick, |cx| run.poll(cx))
        })
        .is_pending());
        assert!(with_tick(&snapshot, &mut ledger, 6, |tick| {
            with_step(tick, |cx| run.poll(cx))
        })
        .is_pending());
        assert!(matches!(
            emitted(&ledger),
            InteractReq::ContinueDialog { .. }
        ));

        accept_last(&mut ledger, 7, true);
        set_operation_count(&mut snapshot, goal, 2);
        assert!(matches!(
            with_tick(&snapshot, &mut ledger, 7, |tick| {
                with_step(tick, |cx| run.poll(cx))
            }),
            Poll::Ready(Ok(_))
        ));
        assert_eq!(
            ledger
                .as_ref()
                .unwrap()
                .outbox
                .iter()
                .filter(|request| matches!(
                    &request.effect,
                    HostEffect::Interaction(InteractReq::ContinueDialog { .. })
                ))
                .count(),
            1,
            "reaching until must not continue a later page"
        );
    });
}

#[test]
fn use_on_settle_deadline_bounds_an_active_dialogue_drain() {
    compile_context_test(|compile| {
        let (mut snapshot, plan, _) =
            operation_fixture_with_settle(compile, "use_on", None, true, 3_000);
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
        snapshot.seed_chat_modal(-1, vec!["Use-on page.".into()]);
        snapshot.seed_chat_options(vec![], 105);
        assert!(with_tick(&snapshot, &mut ledger, 3, |tick| {
            with_step(tick, |cx| run.poll(cx))
        })
        .is_pending());
        assert!(with_tick(&snapshot, &mut ledger, 4, |tick| {
            with_step(tick, |cx| run.poll(cx))
        })
        .is_pending());
        assert!(matches!(
            emitted(&ledger),
            InteractReq::ContinueDialog { .. }
        ));

        assert!(matches!(
            with_tick(&snapshot, &mut ledger, 7, |tick| {
                with_step(tick, |cx| run.poll(cx))
            }),
            Poll::Ready(Err(ActionError::Failed(reason)))
                if reason.as_ref() == "use_on timeout"
        ));
    });
}

#[test]
fn authored_interact_dialogue_progress_outlives_the_default_settle_window() {
    compile_context_test(|compile| {
        let (mut snapshot, plan, _) = operation_fixture_with_settle(
            compile,
            "interact",
            Some(serde_json::json!({"choose": 1})),
            false,
            20_000,
        );
        let mut ledger = None;
        let mut run = with_tick(&snapshot, &mut ledger, 1, |tick| {
            with_step(tick, |cx| plan.begin(cx).unwrap())
        });
        assert!(with_tick(&snapshot, &mut ledger, 2, |tick| {
            with_step(tick, |cx| run.poll(cx))
        })
        .is_pending());
        accept_last(&mut ledger, 3, true);
        for tick in 3..=93 {
            let page = (tick - 3) / 3;
            snapshot.seed_chat_modal(100, vec![format!("Drezel story page {page}")]);
            if page == 14 {
                snapshot.seed_chat_options(
                    vec![api::snapshot::ChatOptionView {
                        component_id: 101,
                        text: "Yes.".into(),
                    }],
                    -1,
                );
            } else {
                snapshot.seed_chat_options(vec![], 105);
            }
            let result = with_tick(&snapshot, &mut ledger, tick, |tick| {
                with_step(tick, |cx| run.poll(cx))
            });
            assert!(result.is_pending(), "advancing page {page} at tick {tick}");
            if !ledger.as_ref().unwrap().outbox.is_empty() {
                accept_last(&mut ledger, tick, true);
            }
        }
        assert!(ledger.as_ref().unwrap().outbox.iter().any(|entry| matches!(
            entry.effect,
            HostEffect::Interaction(InteractReq::Answer { option: 1 })
        )));
        snapshot.seed_chat_modal(-1, vec![]);
        snapshot.seed_chat_options(vec![], -1);
        let mut result = Poll::Pending;
        for tick in 94..=108 {
            result = with_tick(&snapshot, &mut ledger, tick, |tick| {
                with_step(tick, |cx| run.poll(cx))
            });
            if result.is_ready() {
                break;
            }
        }
        assert!(matches!(result, Poll::Ready(Ok(_))));
    });
}

#[test]
fn authored_interact_dialogue_progress_has_a_total_cap() {
    compile_context_test(|compile| {
        let (mut snapshot, plan, _) = operation_fixture_with_settle(
            compile,
            "interact",
            Some(serde_json::json!("continue")),
            false,
            20_000,
        );
        let mut ledger = None;
        let mut run = with_tick(&snapshot, &mut ledger, 1, |tick| {
            with_step(tick, |cx| plan.begin(cx).unwrap())
        });
        assert!(with_tick(&snapshot, &mut ledger, 2, |tick| {
            with_step(tick, |cx| run.poll(cx))
        })
        .is_pending());
        accept_last(&mut ledger, 3, true);
        let mut result = Poll::Pending;
        let mut ended = 0;
        for tick in 3..=210 {
            snapshot.seed_chat_modal(100, vec![format!("Endless page {}", (tick - 3) / 3)]);
            snapshot.seed_chat_options(vec![], 105);
            result = with_tick(&snapshot, &mut ledger, tick, |tick| {
                with_step(tick, |cx| run.poll(cx))
            });
            if result.is_ready() {
                ended = tick;
                break;
            }
            if !ledger.as_ref().unwrap().outbox.is_empty() {
                accept_last(&mut ledger, tick, true);
            }
        }
        assert!(
            matches!(result, Poll::Ready(Err(ActionError::Failed(reason)))
            if reason.as_ref() == "interact settle timeout")
        );
        assert_eq!(
            ended, 202,
            "progress extends inactivity, never the 120-second total cap"
        );
    });
}

#[test]
fn authored_interact_settle_override_still_bounds_missing_dialogue() {
    compile_context_test(|compile| {
        let (snapshot, plan, _) = operation_fixture_with_settle(
            compile,
            "interact",
            Some(serde_json::json!("continue")),
            false,
            6_000,
        );
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
        assert!(matches!(with_tick(&snapshot, &mut ledger, 12, |tick| {
            with_step(tick, |cx| run.poll(cx))
        }), Poll::Ready(Err(ActionError::Failed(reason)))
            if reason.as_ref() == "interact settle timeout"));
    });
}
fn production_menu_client(rows: &[(i32, &str)]) -> client::client::Client {
    use client::client::{Client, ClientConfig};
    use client::config::if_type::{ButtonType, ComponentType, IfType, IfTypeMut};
    use client::io::ServerProt;

    let mut client = Client::new(ClientConfig {
        host: "127.0.0.1".into(),
        port: 43594,
        cache_dir: "/tmp/274bot-no-production-cache".into(),
        members: true,
        lowmem: false,
    });
    let cache = Arc::get_mut(&mut client.cache).unwrap();
    let max_id = rows.iter().map(|(id, _)| *id).max().unwrap_or(0) as usize;
    cache
        .objs
        .resize(max_id + 1, client::config::ObjType::default());
    for (id, name) in rows {
        cache.objs[*id as usize] = client::config::ObjType {
            id: *id,
            name: (*name).into(),
            ..Default::default()
        };
    }

    let button_texts = ["Make 1", "Make 5", "Make 10", "Make X"];
    let mut children = Vec::with_capacity(rows.len() * (button_texts.len() + 1));
    for row_index in 0..rows.len() {
        let row_base = 2110 + row_index as i32 * 20;
        children.push(row_base);
        children.extend((0..button_texts.len()).map(|index| row_base + 10 + index as i32));
    }
    client.set_iface(
        2100,
        IfType {
            id: 2100,
            layer_id: 2100,
            r#type: ComponentType::TYPE_LAYER,
            children: Some(children),
            ..Default::default()
        },
    );
    for (row_index, (object_id, _)) in rows.iter().enumerate() {
        let row_base = 2110 + row_index as i32 * 20;
        client.set_iface(
            row_base as usize,
            IfType {
                id: row_base,
                layer_id: 2100,
                r#type: ComponentType::TYPE_MODEL,
                ..Default::default()
            },
        );
        client.set_iface_mut(
            row_base as usize,
            IfTypeMut {
                model1_type: 4,
                model1_id: *object_id,
                ..Default::default()
            },
        );
        for (index, text) in button_texts.iter().enumerate() {
            let component_id = row_base + 10 + index as i32;
            client.set_iface(
                component_id as usize,
                IfType {
                    id: component_id,
                    layer_id: 2100,
                    r#type: ComponentType::TYPE_TEXT,
                    button_text: (*text).into(),
                    ..Default::default()
                },
            );
            client.set_iface_mut(
                component_id as usize,
                IfTypeMut {
                    button_type: ButtonType::BUTTON_OK,
                    ..Default::default()
                },
            );
        }
    }
    client.chat_modal_id = 2100;
    client.bump_gens(ServerProt::IF_OPENCHAT);
    client
}

fn production_main_make_client(
    item_id: i32,
    item_name: &str,
    actions: &[&str],
) -> client::client::Client {
    use client::client::{Client, ClientConfig};
    use client::config::if_type::{ComponentType, IfType, IfTypeMut};
    use client::io::ServerProt;

    let mut client = Client::new(ClientConfig {
        host: "127.0.0.1".into(),
        port: 43594,
        cache_dir: "/tmp/274bot-no-production-cache".into(),
        members: true,
        lowmem: false,
    });
    let cache = Arc::get_mut(&mut client.cache).unwrap();
    cache
        .objs
        .resize(item_id as usize + 1, client::config::ObjType::default());
    cache.objs[item_id as usize] = client::config::ObjType {
        id: item_id,
        name: item_name.into(),
        ..Default::default()
    };
    client.set_iface(
        2200,
        IfType {
            id: 2200,
            layer_id: 2200,
            r#type: ComponentType::TYPE_LAYER,
            children: Some(vec![2210]),
            ..Default::default()
        },
    );
    let mut operations: [Option<String>; 5] = Default::default();
    for (operation, text) in operations.iter_mut().zip(actions) {
        *operation = Some((*text).into());
    }
    client.set_iface(
        2210,
        IfType {
            id: 2210,
            layer_id: 2200,
            r#type: ComponentType::TYPE_INV,
            iop: operations,
            ..Default::default()
        },
    );
    client.set_iface_mut(
        2210,
        IfTypeMut {
            link_obj_type: Some(vec![item_id + 1]),
            link_obj_number: Some(vec![1]),
            ..Default::default()
        },
    );
    client.main_modal_id = 2200;
    client.bump_gens(ServerProt::IF_OPENMAIN);
    client
}

#[test]
fn make_accepts_selector_only_and_legacy_location_trigger_but_rejects_invalid_authorship() {
    compile_context_test(|compile| {
        let selector = serde_json::json!({
            "product": "unstrung_shortbow",
            "menu": {
                "obj": "shortbow",
                "source": "lostcity-289/content/scripts/skill_fletching/scripts/cut_logs.rs2:18-41"
            },
            "qty": 1
        });
        assert!(
            crate::quester::compile::decode_args::<s2::MakeArgs>(&selector)
                .and_then(|args| s2::compile_make(args, compile))
                .is_ok(),
            "selector-only make follows the preceding use_on step"
        );

        let legacy = serde_json::json!({
            "loc": {"name": "spinningwheel", "op": "Spin"},
            "anchor": {"tile": [3200, 3200, 0], "source": "lostcity-289/content/scripts/skill_crafting/scripts/spinning/spinning.rs2:36-51"},
            "product": "ball_of_wool",
            "menu": {"obj": "wool", "source": "lostcity-289/content/scripts/skill_crafting/scripts/spinning/spinning.rs2:36-51"},
            "qty": 1
        });
        assert!(
            crate::quester::compile::decode_args::<s2::MakeArgs>(&legacy)
                .and_then(|args| s2::compile_make(args, compile))
                .is_ok(),
            "legacy location-triggered make remains authored"
        );

        for invalid in [
            serde_json::json!({
                "loc": {"name": "spinningwheel", "op": "Spin"},
                "product": "ball_of_wool", "qty": 1
            }),
            serde_json::json!({
                "anchor": {"tile": [3200, 3200, 0], "source": "runner fixture"},
                "product": "ball_of_wool", "qty": 1
            }),
            serde_json::json!({"product": "missing-production-item", "qty": 1}),
            serde_json::json!({
                "product": "ball_of_wool",
                "menu": {"obj": "missing-production-menu-item", "source": "runner fixture"},
                "qty": 1
            }),
            serde_json::json!({
                "product": "ball_of_wool",
                "menu": {"obj": "wool", "source": " "},
                "qty": 1
            }),
            serde_json::json!({
                "loc": {"name": "spinningwheel", "op": "Spin"},
                "anchor": {"tile": [3200, 3200, 0], "source": " "},
                "product": "ball_of_wool", "qty": 1
            }),
            serde_json::json!({"product": "ball_of_wool", "qty": 0}),
            serde_json::json!({
                "loc": {"name": "spinningwheel", "op": "Produce"},
                "anchor": {"tile": [3200, 3200, 0], "source": "runner fixture"},
                "product": "ball_of_wool", "qty": 1
            }),
        ] {
            assert!(
                crate::quester::compile::decode_args::<s2::MakeArgs>(&invalid)
                    .and_then(|args| s2::compile_make(args, compile))
                    .is_err(),
                "{invalid}"
            );
        }
    });
}

#[test]
fn item_on_item_use_on_emits_the_selected_inventory_target() {
    compile_context_test(|compile| {
        let source_id = resolve_obj(compile, "knife").unwrap();
        let target_id = resolve_obj(compile, "logs").unwrap();
        let plan = compile_use_on(
            test_args::<UseOnArgs>(serde_json::json!({
                "item": "knife",
                "target": {"item": "logs"}
            })),
            compile,
        )
        .unwrap();
        let mut snapshot = ready();
        snapshot.seed_local_player(local_player(tile(5, 5)));
        snapshot.seed_inventory(
            vec![
                held(source_id, "Knife", 4, 1, None),
                held(target_id, "Logs", 9, 3, None),
            ],
            28,
        );
        let mut ledger = None;
        let mut run = with_tick(&snapshot, &mut ledger, 1, |tick| {
            with_step(tick, |cx| plan.begin(cx).unwrap())
        });
        assert!(with_tick(&snapshot, &mut ledger, 2, |tick| {
            with_step(tick, |cx| run.poll(cx))
        })
        .is_pending());
        assert!(matches!(
            emitted(&ledger),
            InteractReq::UseOn {
                kind,
                source_item_id: Some(source),
                source_item_slot: Some(4),
                target_item_id: Some(target),
                target_item_slot: Some(9),
                ..
            } if kind == "inv" && *source == source_id && *target == target_id
        ));
        accept_last(&mut ledger, 3, true);
        let product_id = resolve_obj(compile, "unstrung_shortbow").unwrap();
        let menu_id = resolve_obj(compile, "shortbow").unwrap();
        let decoy_id = resolve_obj(compile, "longbow").unwrap();
        let make = s2::compile_make(
            test_args::<s2::MakeArgs>(serde_json::json!({
                "product": "unstrung_shortbow",
                "menu": {
                    "obj": "shortbow",
                    "source": "lostcity-289/content/scripts/skill_fletching/scripts/cut_logs.rs2:18-41"
                },
                "qty": 10
            })),
            compile,
        )
        .unwrap();
        let client = production_menu_client(&[(decoy_id, "Longbow"), (menu_id, "Shortbow")]);
        snapshot.rebuild_family(&client, api::snapshot::Family::MakeProducts);
        snapshot.rebuild_family(&client, api::snapshot::Family::Modals);
        snapshot.seed_inventory(
            vec![
                held(source_id, "Knife", 4, 1, None),
                held(target_id, "Logs", 9, 3, None),
                held(product_id, "Unstrung shortbow", 12, 5, None),
            ],
            28,
        );
        assert!(with_tick(&snapshot, &mut ledger, 4, |tick| {
            with_step(tick, |cx| run.poll(cx))
        })
        .is_pending());
        assert!(matches!(
            with_tick(&snapshot, &mut ledger, 5, |tick| {
                with_step(tick, |cx| run.poll(cx))
            }),
            Poll::Ready(Ok(_))
        ));
        let mut make_run = with_tick(&snapshot, &mut ledger, 6, |tick| {
            with_step(tick, |cx| make.begin(cx).unwrap())
        });
        assert!(with_tick(&snapshot, &mut ledger, 7, |tick| {
            with_step(tick, |cx| make_run.poll(cx))
        })
        .is_pending());
        assert!(matches!(
            emitted(&ledger),
            InteractReq::IfButton { component_id: 2141 }
        ));
    });
}

#[test]
fn item_on_location_use_on_opens_pottery_product_selection() {
    compile_context_test(|compile| {
        let source_id = resolve_obj(compile, "softclay").unwrap();
        let wheel_id = resolve_loc(compile, "potterywheel").unwrap();
        let plan = compile_use_on(
            test_args::<UseOnArgs>(serde_json::json!({
                "item": "softclay",
                "target": {"loc": "potterywheel"}
            })),
            compile,
        )
        .unwrap();
        let mut snapshot = ready();
        snapshot.seed_local_player(local_player(tile(5, 5)));
        snapshot.seed_inventory(vec![held(source_id, "Soft clay", 6, 1, None)], 28);
        let mut wheel = loc(wheel_id, "Potter's Wheel", "Use");
        wheel.tile = tile(5, 6);
        wheel.distance = 1;
        snapshot.seed_locs(vec![wheel]);
        let mut ledger = None;
        let mut run = with_tick(&snapshot, &mut ledger, 1, |tick| {
            with_step(tick, |cx| plan.begin(cx).unwrap())
        });
        assert!(with_tick(&snapshot, &mut ledger, 2, |tick| {
            with_step(tick, |cx| run.poll(cx))
        })
        .is_pending());
        assert!(matches!(
            emitted(&ledger),
            InteractReq::UseOn {
                kind,
                target_name: Some(name),
                x: 5,
                z: 6,
                source_item_id: Some(source),
                source_item_slot: Some(6),
                ..
            } if kind == "loc" && name == "Potter's Wheel" && *source == source_id
        ));

        accept_last(&mut ledger, 3, true);
        let product_id = resolve_obj(compile, "bowl_unfired").unwrap();
        let menu_id = resolve_obj(compile, "bowl_unfired").unwrap();
        let make = s2::compile_make(
            test_args::<s2::MakeArgs>(serde_json::json!({
                "product": "bowl_unfired",
                "menu": {
                    "obj": "bowl_unfired",
                    "source": "lostcity-289/content/scripts/skill_crafting/scripts/pottery/pottery.rs2:59-66"
                },
                "qty": 5
            })),
            compile,
        )
        .unwrap();
        let client = production_menu_client(&[(menu_id, "Unfired bowl")]);
        snapshot.rebuild_family(&client, api::snapshot::Family::MakeProducts);
        snapshot.rebuild_family(&client, api::snapshot::Family::Modals);
        snapshot.seed_inventory(vec![held(source_id, "Soft clay", 6, 1, None)], 28);
        assert_ne!(source_id, product_id);
        assert!(with_tick(&snapshot, &mut ledger, 4, |tick| {
            with_step(tick, |cx| run.poll(cx))
        })
        .is_pending());
        assert!(matches!(
            with_tick(&snapshot, &mut ledger, 5, |tick| {
                with_step(tick, |cx| run.poll(cx))
            }),
            Poll::Ready(Ok(_))
        ));
        let mut make_run = with_tick(&snapshot, &mut ledger, 6, |tick| {
            with_step(tick, |cx| make.begin(cx).unwrap())
        });
        assert!(with_tick(&snapshot, &mut ledger, 7, |tick| {
            with_step(tick, |cx| make_run.poll(cx))
        })
        .is_pending());
        assert!(matches!(
            emitted(&ledger),
            InteractReq::IfButton { component_id: 2121 }
        ));
    });
}

#[test]
fn item_on_anvil_make_selects_bronze_wire_main_make_row() {
    compile_context_test(|compile| {
        let bar_id = resolve_obj(compile, "bronze_bar").unwrap();
        let product_id = resolve_obj(compile, "bronzecraftwire").unwrap();
        let anvil_id = resolve_loc(compile, "anvil").unwrap();
        let use_on = compile_use_on(
            test_args::<UseOnArgs>(serde_json::json!({
                "item": "bronze_bar",
                "target": {"loc": "anvil"}
            })),
            compile,
        )
        .unwrap();
        let make = s2::compile_make(
            test_args::<s2::MakeArgs>(serde_json::json!({
                "product": "bronzecraftwire",
                "menu": {
                    "obj": "bronze_bar",
                    "source": "C:scripts/skill_smithing/configs/smithing/smithing.dbrow:1321-1330"
                },
                "qty": 1
            })),
            compile,
        )
        .unwrap();

        let mut snapshot = ready();
        snapshot.seed_local_player(local_player(tile(5, 5)));
        snapshot.seed_inventory(vec![held(bar_id, "Bronze bar", 8, 1, None)], 28);
        let mut anvil = loc(anvil_id, "Anvil", "Use");
        anvil.tile = tile(5, 6);
        anvil.distance = 1;
        snapshot.seed_locs(vec![anvil]);
        let mut ledger = None;
        let mut use_on_run = with_tick(&snapshot, &mut ledger, 1, |tick| {
            with_step(tick, |cx| use_on.begin(cx).unwrap())
        });
        assert!(with_tick(&snapshot, &mut ledger, 2, |tick| {
            with_step(tick, |cx| use_on_run.poll(cx))
        })
        .is_pending());
        assert!(matches!(
            emitted(&ledger),
            InteractReq::UseOn {
                kind,
                target_name: Some(name),
                x: 5,
                z: 6,
                source_item_id: Some(source),
                source_item_slot: Some(8),
                ..
            } if kind == "loc" && name == "Anvil" && *source == bar_id
        ));
        accept_last(&mut ledger, 3, true);

        let menu_client = production_main_make_client(bar_id, "Bronze bar", &["Make 1"]);
        snapshot.rebuild_family(&menu_client, api::snapshot::Family::MainMake);
        snapshot.rebuild_family(&menu_client, api::snapshot::Family::Modals);
        assert_ne!(bar_id, product_id);
        assert!(with_tick(&snapshot, &mut ledger, 4, |tick| {
            with_step(tick, |cx| use_on_run.poll(cx))
        })
        .is_pending());
        assert!(matches!(
            with_tick(&snapshot, &mut ledger, 5, |tick| {
                with_step(tick, |cx| use_on_run.poll(cx))
            }),
            Poll::Ready(Ok(_))
        ));
        let mut make_run = with_tick(&snapshot, &mut ledger, 6, |tick| {
            with_step(tick, |cx| make.begin(cx).unwrap())
        });
        assert!(with_tick(&snapshot, &mut ledger, 7, |tick| {
            with_step(tick, |cx| make_run.poll(cx))
        })
        .is_pending());
        assert!(matches!(
            emitted(&ledger),
            InteractReq::MakePanel {
                id,
                slot: 0,
                component,
                operation: 1,
            } if *id == bar_id && *component == 2210
        ));
    });
}

#[test]
fn loc_make_starts_menu_clock_after_multi_tick_approach() {
    // The accepted click follows approach arrival. World processes client input on
    // a later tick (E engine/src/engine/World.ts:350-355), then NetworkPlayer
    // writes the interface after that tick packet (E engine/src/engine/NetworkPlayer.ts:171-177).
    // A snapshot on that tick can still lack the menu; it is observable on the next.
    compile_context_test(|compile| {
        let wheel_id = resolve_loc(compile, "spinningwheel").unwrap();
        let wool_id = resolve_obj(compile, "wool").unwrap();
        let anchor = tile(3200, 3200);
        let plan = s2::compile_make(
            test_args::<s2::MakeArgs>(serde_json::json!({
                "loc": {"name": "spinningwheel", "op": "Spin"},
                "anchor": {"tile": [3200, 3200, 0], "source": "C:scripts/skill_crafting/scripts/spinning/spinning.rs2:36-51"},
                "product": "ball_of_wool",
                "menu": {
                    "obj": "wool",
                    "source": "C:scripts/skill_crafting/scripts/spinning/spinning.rs2:36-51"
                },
                "qty": 1
            })),
            compile,
        )
        .unwrap();
        let mut snapshot = ready();
        snapshot.seed_local_player(local_player(tile(3195, 3200)));
        let mut wheel = loc(wheel_id, "Spinning wheel", "Spin");
        wheel.tile = anchor;
        wheel.distance = 5;
        snapshot.seed_locs(vec![wheel]);
        snapshot.seed_inventory(Vec::new(), 28);

        let mut ledger = None;
        let mut run = with_tick(&snapshot, &mut ledger, 1, |tick| {
            with_step(tick, |cx| plan.begin(cx).unwrap())
        });
        assert!(with_tick(&snapshot, &mut ledger, 2, |tick| {
            with_step(tick, |cx| run.poll(cx))
        })
        .is_pending());
        assert!(
            matches!(
                &ledger.as_ref().unwrap().outbox.last().unwrap().effect,
                crate::native::HostEffect::Walk(request)
                    if request.target == anchor && request.radius == 1
            ),
            "the loc op must wait for its multi-tick approach"
        );

        for tick in 3..=5 {
            assert!(with_tick(&snapshot, &mut ledger, tick, |native| {
                with_step(native, |cx| run.poll(cx))
            })
            .is_pending());
        }
        snapshot.seed_local_player(local_player(anchor));
        let mut wheel = loc(wheel_id, "Spinning wheel", "Spin");
        wheel.tile = anchor;
        wheel.distance = 0;
        snapshot.seed_locs(vec![wheel]);
        let (request_id, run_key) = {
            let action = ledger
                .as_ref()
                .unwrap()
                .outbox
                .iter()
                .rev()
                .find(|action| matches!(&action.effect, crate::native::HostEffect::Walk(_)))
                .unwrap();
            (action.request_id.get(), action.run())
        };
        ledger.as_mut().unwrap().walk = Some(crate::native::WalkReceipt {
            goal: None,
            request_id,
            evidence: api::quest_progress::EvidenceStamp {
                run: run_key,
                tick: 6,
                sequence: 6,
            },
            end: crate::native::WalkEnd::Arrived,
            blocked: None,
            detail: None,
            refusal: None,
            assessment: None,
            escape: None,
            prereq_allow: None,
        });
        assert!(with_tick(&snapshot, &mut ledger, 6, |tick| {
            with_step(tick, |cx| run.poll(cx))
        })
        .is_pending());
        assert!(matches!(
            &ledger.as_ref().unwrap().outbox.last().unwrap().effect,
            crate::native::HostEffect::Interaction(InteractReq::Loc {
                id: Some(id),
                ..
            }) if *id == wheel_id
        ));

        accept_last(&mut ledger, 7, true);
        assert!(with_tick(&snapshot, &mut ledger, 7, |tick| {
            with_step(tick, |cx| run.poll(cx))
        })
        .is_pending());
        assert!(with_tick(&snapshot, &mut ledger, 8, |tick| {
            with_step(tick, |cx| run.poll(cx))
        })
        .is_pending());
        let menu_client = production_menu_client(&[(wool_id, "Wool")]);
        snapshot.rebuild_family(&menu_client, api::snapshot::Family::MakeProducts);
        snapshot.rebuild_family(&menu_client, api::snapshot::Family::Modals);
        assert!(with_tick(&snapshot, &mut ledger, 9, |tick| {
            with_step(tick, |cx| run.poll(cx))
        })
        .is_pending());
        assert!(matches!(emitted(&ledger), InteractReq::IfButton { .. }));
    });
}

#[test]
fn make_targets_item_yield_using_process_count_from_content() {
    compile_context_test(|compile| {
        let bar_id = resolve_obj(compile, "bronze_bar").unwrap();
        let heads_id = resolve_obj(compile, "bronze_arrowheads").unwrap();
        let plan = s2::compile_make(
            test_args::<s2::MakeArgs>(serde_json::json!({
                "product": "bronze_arrowheads",
                "menu": {
                    "obj": "bronze_bar",
                    "source": "C:scripts/skill_smithing/configs/smithing/smithing.inv:30-35"
                },
                "qty": 7
            })),
            compile,
        )
        .unwrap();
        let menu_client = production_menu_client(&[(bar_id, "Bronze bar")]);
        let mut snapshot = ready();
        snapshot.seed_inventory(
            vec![
                held(bar_id, "Bronze bar", 8, 1, None),
                held(heads_id, "Bronze arrowheads", 9, 0, None),
            ],
            28,
        );
        snapshot.rebuild_family(&menu_client, api::snapshot::Family::MakeProducts);
        snapshot.rebuild_family(&menu_client, api::snapshot::Family::Modals);
        let mut ledger = None;
        let mut run = with_tick(&snapshot, &mut ledger, 1, |tick| {
            with_step(tick, |cx| plan.begin(cx).unwrap())
        });
        assert!(with_tick(&snapshot, &mut ledger, 2, |tick| {
            with_step(tick, |cx| run.poll(cx))
        })
        .is_pending());
        assert!(matches!(
            emitted(&ledger),
            InteractReq::IfButton { component_id: 2120 }
        ));
    });
}

#[test]
fn main_make_retriggers_item_on_loc_after_each_closed_batch() {
    use client::io::ServerProt;

    compile_context_test(|compile| {
        let bar_id = resolve_obj(compile, "bronze_bar").unwrap();
        let wire_id = resolve_obj(compile, "bronzecraftwire").unwrap();
        let anvil_id = resolve_loc(compile, "anvil").unwrap();
        let make = s2::compile_make(
            test_args::<s2::MakeArgs>(serde_json::json!({
                "loc": {"name": "anvil"},
                "anchor": {
                    "tile": [6, 7, 0],
                    "source": "unit fixture",
                },
                "product": "bronzecraftwire",
                "menu": {
                    "obj": "bronze_bar",
                    "source": "C:scripts/skill_smithing/configs/smithing/smithing.inv:30-35"
                },
                "qty": 7
            })),
            compile,
        )
        .unwrap();
        let mut menu_client =
            production_main_make_client(bar_id, "Bronze bar", &["Make 1", "Make 5", "Make 10"]);
        let mut snapshot = ready();
        snapshot.seed_local_player(local_player(tile(5, 6)));
        snapshot.seed_inventory(
            vec![
                held(bar_id, "Bronze bar", 8, 10, None),
                held(wire_id, "Bronze wire", 9, 0, None),
            ],
            28,
        );
        let mut anvil = loc(anvil_id, "Anvil", "Use");
        anvil.tile = tile(5, 6);
        anvil.distance = 0;
        snapshot.seed_locs(vec![anvil]);
        let mut ledger = None;
        let mut run = with_tick(&snapshot, &mut ledger, 1, |tick| {
            with_step(tick, |cx| make.begin(cx).unwrap())
        });

        assert!(with_tick(&snapshot, &mut ledger, 2, |tick| {
            with_step(tick, |cx| run.poll(cx))
        })
        .is_pending());
        assert!(matches!(
            emitted(&ledger),
            InteractReq::UseOn {
                kind,
                x,
                z,
                source_item_id: Some(id),
                target_item_id: Some(target),
                ..
            } if kind == "loc"
                && (*x, *z) == (5, 6)
                && *id == bar_id
                && *target == anvil_id
        ));
        accept_last(&mut ledger, 3, true);
        assert!(with_tick(&snapshot, &mut ledger, 3, |tick| {
            with_step(tick, |cx| run.poll(cx))
        })
        .is_pending());

        menu_client.main_modal_id = 2200;
        menu_client.bump_gens(ServerProt::IF_OPENMAIN);
        assert!(snapshot.rebuild_family(&menu_client, api::snapshot::Family::MainMake));
        snapshot.rebuild_family(&menu_client, api::snapshot::Family::Modals);
        assert!(with_tick(&snapshot, &mut ledger, 4, |tick| {
            with_step(tick, |cx| run.poll(cx))
        })
        .is_pending());
        assert!(matches!(
            emitted(&ledger),
            InteractReq::MakePanel { operation: 2, .. }
        ));
        accept_last(&mut ledger, 5, true);
        // C:scripts/skill_smithing/scripts/smithing/smithing.rs2:213-214 closes the menu per attempt.
        menu_client.main_modal_id = -1;
        menu_client.bump_gens(ServerProt::IF_CLOSE);
        assert!(snapshot.rebuild_family(&menu_client, api::snapshot::Family::MainMake));
        snapshot.rebuild_family(&menu_client, api::snapshot::Family::Modals);
        assert!(with_tick(&snapshot, &mut ledger, 6, |tick| {
            with_step(tick, |cx| run.poll(cx))
        })
        .is_pending());

        snapshot.seed_inventory(
            vec![
                held(bar_id, "Bronze bar", 8, 5, None),
                held(wire_id, "Bronze wire", 9, 5, None),
            ],
            28,
        );
        for tick in 7..=10 {
            assert!(with_tick(&snapshot, &mut ledger, tick, |native| {
                with_step(native, |cx| run.poll(cx))
            })
            .is_pending());
        }
        assert!(with_tick(&snapshot, &mut ledger, 11, |tick| {
            with_step(tick, |cx| run.poll(cx))
        })
        .is_pending());
        assert!(matches!(
            emitted(&ledger),
            InteractReq::UseOn {
                kind,
                x,
                z,
                source_item_id: Some(id),
                target_item_id: Some(target),
                ..
            } if kind == "loc"
                && (*x, *z) == (5, 6)
                && *id == bar_id
                && *target == anvil_id
        ));

        accept_last(&mut ledger, 12, true);
        assert!(with_tick(&snapshot, &mut ledger, 12, |tick| {
            with_step(tick, |cx| run.poll(cx))
        })
        .is_pending());
        menu_client.main_modal_id = 2200;
        menu_client.bump_gens(ServerProt::IF_OPENMAIN);
        assert!(snapshot.rebuild_family(&menu_client, api::snapshot::Family::MainMake));
        snapshot.rebuild_family(&menu_client, api::snapshot::Family::Modals);
        assert!(with_tick(&snapshot, &mut ledger, 13, |tick| {
            with_step(tick, |cx| run.poll(cx))
        })
        .is_pending());
        assert!(matches!(
            emitted(&ledger),
            InteractReq::MakePanel { operation: 1, .. }
        ));

        accept_last(&mut ledger, 14, true);
        menu_client.main_modal_id = -1;
        menu_client.bump_gens(ServerProt::IF_CLOSE);
        assert!(snapshot.rebuild_family(&menu_client, api::snapshot::Family::MainMake));
        snapshot.rebuild_family(&menu_client, api::snapshot::Family::Modals);
        assert!(with_tick(&snapshot, &mut ledger, 15, |tick| {
            with_step(tick, |cx| run.poll(cx))
        })
        .is_pending());
        snapshot.seed_inventory(
            vec![
                held(bar_id, "Bronze bar", 8, 4, None),
                held(wire_id, "Bronze wire", 9, 6, None),
            ],
            28,
        );
        for tick in 16..=19 {
            assert!(with_tick(&snapshot, &mut ledger, tick, |native| {
                with_step(native, |cx| run.poll(cx))
            })
            .is_pending());
        }
        assert!(with_tick(&snapshot, &mut ledger, 20, |tick| {
            with_step(tick, |cx| run.poll(cx))
        })
        .is_pending());
        assert!(matches!(
            emitted(&ledger),
            InteractReq::UseOn {
                kind,
                x,
                z,
                source_item_id: Some(id),
                target_item_id: Some(target),
                ..
            } if kind == "loc"
                && (*x, *z) == (5, 6)
                && *id == bar_id
                && *target == anvil_id
        ));

        accept_last(&mut ledger, 21, true);
        assert!(with_tick(&snapshot, &mut ledger, 21, |tick| {
            with_step(tick, |cx| run.poll(cx))
        })
        .is_pending());
        menu_client.main_modal_id = 2200;
        menu_client.bump_gens(ServerProt::IF_OPENMAIN);
        assert!(snapshot.rebuild_family(&menu_client, api::snapshot::Family::MainMake));
        snapshot.rebuild_family(&menu_client, api::snapshot::Family::Modals);
        assert!(with_tick(&snapshot, &mut ledger, 22, |tick| {
            with_step(tick, |cx| run.poll(cx))
        })
        .is_pending());
        assert!(matches!(
            emitted(&ledger),
            InteractReq::MakePanel { operation: 1, .. }
        ));

        accept_last(&mut ledger, 23, true);
        menu_client.main_modal_id = -1;
        menu_client.bump_gens(ServerProt::IF_CLOSE);
        assert!(snapshot.rebuild_family(&menu_client, api::snapshot::Family::MainMake));
        snapshot.rebuild_family(&menu_client, api::snapshot::Family::Modals);
        assert!(with_tick(&snapshot, &mut ledger, 24, |tick| {
            with_step(tick, |cx| run.poll(cx))
        })
        .is_pending());
        snapshot.seed_inventory(
            vec![
                held(bar_id, "Bronze bar", 8, 3, None),
                held(wire_id, "Bronze wire", 9, 7, None),
            ],
            28,
        );
        for tick in 25..=27 {
            assert!(with_tick(&snapshot, &mut ledger, tick, |native| {
                with_step(native, |cx| run.poll(cx))
            })
            .is_pending());
        }
        assert!(matches!(
            with_tick(&snapshot, &mut ledger, 28, |tick| {
                with_step(tick, |cx| run.poll(cx))
            }),
            Poll::Ready(Ok(_))
        ));
    });
}
