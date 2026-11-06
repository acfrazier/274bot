use super::*;
use api::selected::{FactKey, QuestGate};
use client::config::if_type::{ButtonType, ComponentType};
use client::config::{IfType, IfTypeMut};
use client::io::ServerProt;
use nav_planner::quest_gates::QuestGates;
use std::sync::Arc;

use crate::traveller::CrossingPass;

const REFUSAL: &str = "The barrier refuses to open.";
const PASS: &str = "You pass through the barrier.";

fn gated_shantay_edge() -> TransportEdge {
    let mut edge = shantay_edge();
    edge.quest_gates = Some(
        QuestGates::new([
            QuestGate::Complete(FactKey::new("druidspirit")),
            QuestGate::Complete(FactKey::new("priestperil")),
        ])
        .expect("valid gates"),
    );
    edge
}

fn start_sent_hop(
    edge: TransportEdge,
    snapshot: &GameSnapshot,
    options: &TravelOptions<'_>,
) -> FollowRun {
    let route = Route {
        legs: vec![Leg::Transport {
            edge: Box::new(edge.clone()),
        }],
        dest: edge.to,
        ticks: 1.0,
    };
    let mut run = FollowRun::start(route, options);
    let leg = run.next_leg().expect("one transport leg").into_transport();
    run.transport = Some(TransportHop {
        leg,
        to: edge.to,
        ticks_waited: super::super::TickCount::sent_at(snapshot.tick()),
        sent_tile: Some(edge.at),
        arrival_footprint: None,
        tries: 1,
        troll: false,
        npc_index: None,
        npc_recovery: super::super::NpcRecovery::default(),
        open_sent_tick: None,
        door_retry: super::super::DoorRetry::default(),
        occupied: super::super::OccupiedRetry::default(),
        chat_seq: super::super::chat_seq(snapshot),
        refusal_watch: super::super::gate_refusal_watch(snapshot, &edge, options),
        pass_watch: super::super::gate_pass_watch(snapshot, &edge, options),
        dialog_page: None,
        approach: None,
    });
    run
}

fn plant_message_dialog(client: &mut Client, message: &str) {
    plant_message_dialog_lines(client, &[message]);
}

fn plant_message_dialog_lines(client: &mut Client, messages: &[&str]) {
    let root = 100;
    let continue_id = 101;
    let mut children = Vec::with_capacity(messages.len() + 1);
    client.set_iface(
        continue_id,
        IfType {
            id: continue_id as i32,
            layer_id: root,
            ..Default::default()
        },
    );
    client.set_iface_mut(
        continue_id,
        IfTypeMut {
            button_type: ButtonType::BUTTON_CONTINUE,
            text: "Click here to continue".into(),
            ..Default::default()
        },
    );
    for (index, message) in messages.iter().enumerate() {
        let message_id = 102 + index;
        client.set_iface(
            message_id,
            IfType {
                id: message_id as i32,
                layer_id: root,
                r#type: ComponentType::TYPE_TEXT,
                ..Default::default()
            },
        );
        client.set_iface_mut(
            message_id,
            IfTypeMut {
                text: (*message).into(),
                ..Default::default()
            },
        );
        children.push(message_id as i32);
    }
    children.push(continue_id as i32);
    client.set_iface(
        root as usize,
        IfType {
            id: root,
            layer_id: root,
            children: Some(children),
            ..Default::default()
        },
    );
    client.chat_modal_id = root;
    client.bump_gens(ServerProt::IF_OPENCHAT);
}

#[test]
fn stale_refusal_is_ignored_and_fresh_mesbox_pages_close_before_terminal() {
    super::n1_tests::install_log_capture();
    let log_slot = "prereq-hop-test-refusal";
    api::hostlog::bind_slot(log_slot);
    let mut client = scene_client();
    let mut snapshot = snap_at(&mut client, 0, 1);
    client.add_chat(0, REFUSAL, "");
    bump_rebuild(&mut client, &mut snapshot);

    let edge = gated_shantay_edge();
    let crossing = edge.crossing_id().expect("directed crossing identity");
    let gates = edge
        .quest_gates
        .as_ref()
        .expect("gate fixture")
        .shared_gates();
    let mut options = TravelOptions {
        close_enough: 0,
        ..TravelOptions::default()
    };
    options.refusal_texts = Arc::from([
        (crossing, Arc::<str>::from("A different refusal literal.")),
        (crossing, Arc::<str>::from(REFUSAL)),
    ]);
    let mut run = start_sent_hop(edge, &snapshot, &options);
    let mut driver = FollowRec::default();

    plant_message_dialog(&mut client, "An unrelated page after the send.");
    bump_rebuild(&mut client, &mut snapshot);
    assert!(matches!(
        run.poll_transport(&mut driver, &snapshot, &mut options, &mut None),
        Poll::Watching
    ));
    assert_eq!(driver.pause_buttons, 1);

    client.chat_modal_id = -1;
    bump_rebuild(&mut client, &mut snapshot);
    assert!(matches!(
        run.poll_transport(&mut driver, &snapshot, &mut options, &mut None),
        Poll::Watching
    ));

    client.add_chat(0, REFUSAL, "");
    plant_message_dialog(&mut client, REFUSAL);
    bump_rebuild(&mut client, &mut snapshot);
    let matched_line = snapshot
        .chat_lines()
        .iter()
        .filter(|line| line.text == REFUSAL)
        .max_by_key(|line| line.sequence)
        .expect("fresh refusal line");
    let matched_tick = u64::from(snapshot.tick());
    let matched_sequence = u64::try_from(matched_line.sequence).expect("nonnegative sequence");
    assert!(matches!(
        run.poll_transport(&mut driver, &snapshot, &mut options, &mut None),
        Poll::Watching
    ));
    assert_eq!(driver.pause_buttons, 2, "refusal page is acknowledged");

    client.add_chat(0, "The guard sends you on your way.", "");
    plant_message_dialog(&mut client, "A following mesbox page.");
    bump_rebuild(&mut client, &mut snapshot);
    assert!(matches!(
        run.poll_transport(&mut driver, &snapshot, &mut options, &mut None),
        Poll::Watching
    ));
    assert_eq!(driver.pause_buttons, 3, "the next page is acknowledged too");

    client.chat_modal_id = -1;
    bump_rebuild(&mut client, &mut snapshot);
    match run.poll_transport(&mut driver, &snapshot, &mut options, &mut None) {
        Poll::Terminal(TravelOutcome::GateRefused {
            leg,
            crossing: observed_crossing,
            gates: observed_gates,
            matched,
            ..
        }) => {
            assert_eq!(leg, 0);
            assert_eq!(observed_crossing, crossing);
            assert_eq!(observed_gates.as_ref(), gates.as_ref());
            assert_eq!(matched.text, 1, "ordinal is local to this crossing");
            assert_eq!(matched.tick, matched_tick);
            assert_eq!(matched.sequence, matched_sequence);
        }
        other => panic!("expected acknowledged GateRefused, got {other:?}"),
    }
    assert_eq!(
        super::n1_tests::captured_messages(log_slot),
        vec![
            format!(
                "prereq hop-wait=refusal-page crossing={crossing:?} since=2 at=3 count=1 bound=3 end=page hop_budget=60"
            ),
            format!(
                "prereq hop-wait=refusal-page crossing={crossing:?} since=3 at=5 count=2 bound=3 end=page hop_budget=60"
            ),
            format!(
                "prereq hop-wait=refusal-page crossing={crossing:?} since=5 at=6 count=1 bound=3 end=page hop_budget=60"
            ),
            format!(
                "prereq hop-wait=refusal-close crossing={crossing:?} since=6 at=7 count=1 bound=3 end=closed hop_budget=60"
            ),
        ]
    );
}

#[test]
fn changed_refusal_pages_close_without_a_new_chat_ring_message() {
    let mut client = scene_client();
    let mut snapshot = snap_at(&mut client, 0, 1);
    client.add_chat(0, "An existing ordinary chat line.", "");
    plant_message_dialog(&mut client, REFUSAL);
    bump_rebuild(&mut client, &mut snapshot);
    let observed_sequence = super::super::chat_seq(&snapshot);
    let edge = gated_shantay_edge();
    let crossing = edge.crossing_id().expect("directed crossing identity");
    let mut options = TravelOptions {
        close_enough: 0,
        refusal_texts: Arc::from([(crossing, Arc::<str>::from(REFUSAL))]),
        ..TravelOptions::default()
    };
    let mut run = start_sent_hop(edge, &snapshot, &options);
    let mut driver = FollowRec::default();
    assert!(matches!(
        run.poll_transport(&mut driver, &snapshot, &mut options, &mut None),
        Poll::Watching
    ));
    assert_eq!(driver.pause_buttons, 0, "the send-time page is stale");

    for (index, page) in ["STOP!", "Can't I go through here?", REFUSAL]
        .into_iter()
        .enumerate()
    {
        if page == REFUSAL {
            plant_message_dialog_lines(&mut client, &["The barrier refuses", "to open."]);
        } else {
            plant_message_dialog(&mut client, page);
        }
        bump_rebuild(&mut client, &mut snapshot);
        assert_eq!(super::super::chat_seq(&snapshot), observed_sequence);
        assert!(matches!(
            run.poll_transport(&mut driver, &snapshot, &mut options, &mut None),
            Poll::Watching
        ));
        assert_eq!(driver.pause_buttons, index + 1);
    }
    let matched_tick = u64::from(snapshot.tick());
    client.chat_modal_id = -1;
    bump_rebuild(&mut client, &mut snapshot);
    let Poll::Terminal(TravelOutcome::GateRefused { matched, .. }) =
        run.poll_transport(&mut driver, &snapshot, &mut options, &mut None)
    else {
        panic!("acknowledged refusal pages need no ordinary chat message");
    };
    assert_eq!(matched.text, 0);
    assert_eq!(matched.tick, matched_tick);
    assert_eq!(
        matched.sequence,
        u64::try_from(observed_sequence).expect("observed chat sequence")
    );
}

#[test]
fn pass_requires_the_exact_direction_message_and_landing() {
    super::n1_tests::install_log_capture();
    let log_slot = "prereq-hop-test-pass";
    api::hostlog::bind_slot(log_slot);
    let mut client = scene_client();
    let mut snapshot = snap_at(&mut client, 0, 1);
    let edge = gated_shantay_edge();
    let crossing = edge.crossing_id().expect("directed crossing identity");
    let mut reverse = edge.clone();
    std::mem::swap(&mut reverse.at, &mut reverse.to);
    let reverse_crossing = reverse.crossing_id().expect("reverse crossing identity");
    let pass_landing = WorldTile {
        x: 3201,
        z: 3210,
        level: 0,
    };
    let mut options = TravelOptions {
        close_enough: 0,
        ..TravelOptions::default()
    };
    options.crossing_passes = Arc::from([
        CrossingPass {
            crossing: reverse_crossing,
            message: Arc::from("You step back through the barrier."),
            landing: pass_landing,
            delay_ticks: 0,
        },
        CrossingPass {
            crossing,
            message: Arc::from(PASS),
            landing: pass_landing,
            delay_ticks: 2,
        },
    ]);
    let mut run = start_sent_hop(edge, &snapshot, &options);
    let mut driver = FollowRec::default();

    client.add_chat(0, "You step back through the barrier.", "");
    plant_player(&mut client, 1, 10);
    bump_rebuild(&mut client, &mut snapshot);
    assert!(matches!(
        run.poll_transport(&mut driver, &snapshot, &mut options, &mut None),
        Poll::Watching
    ));

    client.add_chat(0, PASS, "");
    plant_player(&mut client, 0, 10);
    bump_rebuild(&mut client, &mut snapshot);
    assert!(
        matches!(
            run.poll_transport(&mut driver, &snapshot, &mut options, &mut None),
            Poll::Watching
        ),
        "the right pass message at the packed edge landing is not enough"
    );

    plant_player(&mut client, 1, 10);
    bump_rebuild(&mut client, &mut snapshot);
    assert!(matches!(
        run.poll_transport(&mut driver, &snapshot, &mut options, &mut None),
        Poll::LegDone
    ));
    assert_eq!(
        super::n1_tests::captured_messages(log_slot),
        vec![format!(
            "prereq hop-wait=pass crossing={crossing:?} since=1 at=4 count=3 bound=5 end=pass hop_budget=60"
        )]
    );
}

#[test]
fn pass_timeout_logs_observed_count_and_hop_budget_separately() {
    super::n1_tests::install_log_capture();
    let log_slot = "prereq-hop-test-pass-timeout";
    api::hostlog::bind_slot(log_slot);
    let mut client = scene_client();
    let mut snapshot = snap_at(&mut client, 0, 1);
    let edge = gated_shantay_edge();
    let crossing = edge.crossing_id().expect("directed crossing identity");
    let mut options = TravelOptions {
        close_enough: 0,
        crossing_passes: Arc::from([CrossingPass {
            crossing,
            message: Arc::from(PASS),
            landing: edge.to,
            delay_ticks: 0,
        }]),
        ..TravelOptions::default()
    };
    let mut run = start_sent_hop(edge, &snapshot, &options);
    let mut driver = FollowRec::default();

    let outcome = loop {
        bump_rebuild(&mut client, &mut snapshot);
        match run.poll_transport(&mut driver, &snapshot, &mut options, &mut None) {
            Poll::Terminal(outcome) => break outcome,
            Poll::Watching => {}
            other => panic!("unexpected result before the hop budget: {other:?}"),
        }
    };
    assert!(matches!(outcome, TravelOutcome::Stalled { .. }));
    assert_eq!(
        super::n1_tests::captured_messages(log_slot),
        vec![format!(
            "prereq hop-wait=pass crossing={crossing:?} since=1 at=62 count=61 bound=3 end=timeout hop_budget=60"
        )]
    );
}

#[test]
fn refusal_page_timeout_logs_observed_count_and_hop_budget_separately() {
    super::n1_tests::install_log_capture();
    let log_slot = "prereq-hop-test-refusal-page-timeout";
    api::hostlog::bind_slot(log_slot);
    let mut client = scene_client();
    let mut snapshot = snap_at(&mut client, 0, 1);
    let edge = gated_shantay_edge();
    let crossing = edge.crossing_id().expect("directed crossing identity");
    let mut options = TravelOptions {
        close_enough: 0,
        refusal_texts: Arc::from([(crossing, Arc::<str>::from(REFUSAL))]),
        ..TravelOptions::default()
    };
    let mut run = start_sent_hop(edge, &snapshot, &options);
    let mut driver = FollowRec::default();

    let outcome = loop {
        bump_rebuild(&mut client, &mut snapshot);
        match run.poll_transport(&mut driver, &snapshot, &mut options, &mut None) {
            Poll::Terminal(outcome) => break outcome,
            Poll::Watching => {}
            other => panic!("unexpected result before the hop budget: {other:?}"),
        }
    };
    assert!(matches!(outcome, TravelOutcome::Stalled { .. }));
    assert_eq!(
        super::n1_tests::captured_messages(log_slot),
        vec![format!(
            "prereq hop-wait=refusal-page crossing={crossing:?} since=1 at=62 count=61 bound=3 end=timeout hop_budget=60"
        )]
    );
}

#[test]
fn refusal_close_timeout_logs_from_the_terminal_continue() {
    super::n1_tests::install_log_capture();
    let log_slot = "prereq-hop-test-refusal-close-timeout";
    api::hostlog::bind_slot(log_slot);
    let mut client = scene_client();
    let mut snapshot = snap_at(&mut client, 0, 1);
    let edge = gated_shantay_edge();
    let crossing = edge.crossing_id().expect("directed crossing identity");
    let mut options = TravelOptions {
        close_enough: 0,
        refusal_texts: Arc::from([(crossing, Arc::<str>::from(REFUSAL))]),
        ..TravelOptions::default()
    };
    let mut run = start_sent_hop(edge, &snapshot, &options);
    let mut driver = FollowRec::default();

    client.add_chat(0, REFUSAL, "");
    plant_message_dialog(&mut client, REFUSAL);
    bump_rebuild(&mut client, &mut snapshot);
    assert!(matches!(
        run.poll_transport(&mut driver, &snapshot, &mut options, &mut None),
        Poll::Watching
    ));
    assert_eq!(
        driver.pause_buttons, 1,
        "the terminal refusal page is acknowledged"
    );

    let outcome = loop {
        bump_rebuild(&mut client, &mut snapshot);
        match run.poll_transport(&mut driver, &snapshot, &mut options, &mut None) {
            Poll::Terminal(outcome) => break outcome,
            Poll::Watching => {}
            other => panic!("unexpected result before the hop budget: {other:?}"),
        }
    };
    assert!(matches!(outcome, TravelOutcome::Stalled { .. }));
    assert_eq!(
        super::n1_tests::captured_messages(log_slot),
        vec![
            format!(
                "prereq hop-wait=refusal-page crossing={crossing:?} since=1 at=2 count=1 bound=3 end=page hop_budget=60"
            ),
            format!(
                "prereq hop-wait=refusal-close crossing={crossing:?} since=2 at=62 count=60 bound=3 end=timeout hop_budget=60"
            ),
        ]
    );
}

#[test]
fn unmatched_direction_pass_keeps_generic_arrival_policy() {
    let mut client = scene_client();
    let mut snapshot = snap_at(&mut client, 0, 1);
    let edge = gated_shantay_edge();
    let mut reverse = edge.clone();
    std::mem::swap(&mut reverse.at, &mut reverse.to);
    let reverse_crossing = reverse.crossing_id().expect("reverse crossing identity");
    let mut options = TravelOptions {
        close_enough: 0,
        crossing_passes: Arc::from([CrossingPass {
            crossing: reverse_crossing,
            message: Arc::from("You step back through the barrier."),
            landing: edge.to,
            delay_ticks: 0,
        }]),
        ..TravelOptions::default()
    };
    let mut run = start_sent_hop(edge, &snapshot, &options);
    let mut driver = FollowRec::default();

    client.add_chat(0, "You step back through the barrier.", "");
    plant_player(&mut client, 0, 10);
    bump_rebuild(&mut client, &mut snapshot);
    assert!(matches!(
        run.poll_transport(&mut driver, &snapshot, &mut options, &mut None),
        Poll::LegDone
    ));
}
