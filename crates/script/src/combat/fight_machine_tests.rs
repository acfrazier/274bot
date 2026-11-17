//! Guardian Fight slice (design V11D 5): the exact-actor target and the escape
//! reserve in the shared Combat core. The event actor is one NPC slot and one
//! content type; the fight hands over to Flee, instead of attacking on trust,
//! when its Hitpoints cannot pay for the walk out.
use super::*;
use crate::combat::request::EscapeReserve;
use crate::combat::schedule::OpKind;

/// The Hitpoints stat index.
const HITPOINTS: usize = 3;

/// A top-tier event: it hits for 14 at most, one attack every four ticks.
const TOP_TIER: EscapeReserve = EscapeReserve {
    hit: 14,
    interval: 4,
};

fn exact_request(scene: &Scene) -> CombatRequest {
    let npc = &scene.npcs[0];
    CombatRequest {
        target: Target::ExactNpc {
            index: npc.index as u16,
            npc_type: npc.r#type.unwrap() as i32,
        },
        tactic: Tactic::Open,
        fallback: Fallback::Abort,
        ..CombatRequest::default()
    }
}

fn attacks_in(batch: &DrainedBatch) -> Vec<i32> {
    (0..batch.len())
        .filter_map(|row| match batch.get(row) {
            Some(HostEffect::Interaction(InteractReq::Npc {
                action,
                index: Some(index),
                ..
            })) if action == "Attack" => Some(*index),
            _ => None,
        })
        .collect()
}

/// Poll up to `limit` ticks from `from` and say how the fight ended and which
/// NPC slots it clicked Attack on. `None` means it was still running.
fn run(
    harness: &mut Harness,
    scene: &Scene,
    from: u64,
    limit: u64,
) -> (Option<CombatEnd>, Vec<i32>) {
    let mut clicked = Vec::new();
    for tick in from..from + limit {
        match harness.poll(&scene.snapshot, tick) {
            Poll::Pending => clicked.extend(attacks_in(&harness.drain(None))),
            Poll::Ready(Ok(report)) => {
                clicked.extend(attacks_in(&harness.drain(None)));
                return (Some(report.end), clicked);
            }
            Poll::Ready(Err(error)) => panic!("combat failed: {error:?}"),
        }
    }
    (None, clicked)
}

/// A second troll of the same type stands next to the player, the event actor
/// three tiles away. A nearest-type pick would take the neighbour. The exact
/// target takes only its own slot.
#[test]
fn exact_npc_attacks_its_own_slot_and_not_a_nearer_neighbour_of_the_same_type() {
    if !api::test_support::game_data_available() {
        return;
    }
    let mut scene = Scene::new("macro_rivertrollguardian_1");
    let actor_tile = tile(
        scene.local.player.network.x + 3,
        scene.local.player.network.z,
    );
    scene.npcs[0].tile = actor_tile;
    scene.npcs[0].network = actor_tile;
    scene.npcs[0].distance = 3;
    let mut neighbour = scene.npcs[0].clone();
    neighbour.index = 9;
    neighbour.tile = tile(
        scene.local.player.network.x + 1,
        scene.local.player.network.z,
    );
    neighbour.network = neighbour.tile;
    neighbour.distance = 1;
    scene.npcs.push(neighbour);
    scene.refresh();

    let mut harness = Harness::new(&scene, exact_request(&scene));
    let (_, clicked) = run(&mut harness, &scene, 1, 3);
    assert!(
        clicked.contains(&7),
        "the exact actor is attacked: {clicked:?}"
    );
    assert!(
        !clicked.contains(&9),
        "the nearer neighbour of the same type is never attacked: {clicked:?}"
    );

    // The same scene under the ordinary nearest pick shows what the exact
    // target prevents.
    let mut nearest = Harness::new(&scene, scene.request());
    let (_, clicked) = run(&mut nearest, &scene, 1, 3);
    assert!(
        clicked.contains(&9),
        "the nearest pick takes the neighbour: {clicked:?}"
    );
}

/// The slot of the event actor holds another type after a reuse: the actor is
/// gone, and nothing clicks Attack on the new occupant.
#[test]
fn exact_npc_is_gone_when_its_slot_is_reused_by_another_type() {
    if !api::test_support::game_data_available() {
        return;
    }
    let mut scene = Scene::new("macro_rivertrollguardian_1");
    let mut harness = Harness::new(&scene, exact_request(&scene));
    attack(harness.pending(&scene, 1));
    scene.install();
    scene.face_us();
    scene.refresh();
    assert!(harness.pending(&scene, 2).is_none());

    let other = scene
        .data
        .npc_by_config("macro_rivertrollguardian_2")
        .unwrap()
        .id;
    scene.npcs[0].r#type = Some(other as usize);
    scene.refresh();
    let (end, clicked) = run(&mut harness, &scene, 3, 6);
    assert_eq!(end, Some(CombatEnd::TargetGone));
    assert!(
        clicked.is_empty(),
        "the reused slot is not attacked: {clicked:?}"
    );
}

/// A type the content does not let a player attack cannot be an exact target.
#[test]
fn exact_npc_of_an_unattackable_type_is_refused_when_the_fight_begins() {
    if !api::test_support::game_data_available() {
        return;
    }
    let scene = Scene::new("hans");
    assert!(
        Harness::try_new(&scene, exact_request(&scene), 0).is_err(),
        "Hans cannot be fought"
    );
}

/// With no unreserved food, a fight whose Hitpoints sit above the line keeps
/// attacking, and at the line it hands over before another attack. The line for
/// a top-tier event on a seven-step walk is 71 Hitpoints (the design example).
#[test]
fn an_escape_reserve_hands_over_at_the_line_when_no_food_remains() {
    if !api::test_support::game_data_available() {
        return;
    }
    for (hitpoints, hands_over) in [(72, false), (71, true)] {
        let mut scene = Scene::new("macro_rivertrollguardian_1");
        scene.stat(HITPOINTS, hitpoints, 99);
        scene.refresh();
        let mut request = exact_request(&scene);
        request.escape_reserve = Some(TOP_TIER);
        let mut harness = Harness::new(&scene, request);
        harness.machine.set_escape_walk(7);
        let (end, clicked) = run(&mut harness, &scene, 1, 4);
        if hands_over {
            assert_eq!(
                end,
                Some(CombatEnd::Aborted(AbortReason::Unprotected(
                    Unprotected::NoFood
                ))),
                "{hitpoints} Hitpoints cannot pay for the walk"
            );
            assert!(
                clicked.is_empty(),
                "no attack goes out below the line: {clicked:?}"
            );
        } else {
            assert_eq!(end, None, "{hitpoints} Hitpoints still fight");
            assert!(
                clicked.contains(&7),
                "the fight attacks above the line: {clicked:?}"
            );
        }
    }
}

/// A longer walk raises the line: the same Hitpoints that fight on a seven-step
/// walk hand over on an eleven-step one (the line is 85).
#[test]
fn a_longer_walk_raises_the_line() {
    if !api::test_support::game_data_available() {
        return;
    }
    for (walk, hands_over) in [(7, false), (11, true)] {
        let mut scene = Scene::new("macro_rivertrollguardian_1");
        scene.stat(HITPOINTS, 80, 99);
        scene.refresh();
        let mut request = exact_request(&scene);
        request.escape_reserve = Some(TOP_TIER);
        let mut harness = Harness::new(&scene, request);
        harness.machine.set_escape_walk(walk);
        let (end, _) = run(&mut harness, &scene, 1, 4);
        assert_eq!(end.is_some(), hands_over, "walk {walk}: {end:?}");
    }
}

/// Food the fight may spend keeps it going below the line: the ordinary bite
/// tiers decide when to eat, and the hand-over comes only once the food is
/// gone.
#[test]
fn food_that_remains_keeps_the_fight_going_below_the_line() {
    if !api::test_support::game_data_available() {
        return;
    }
    let mut scene = Scene::new("macro_rivertrollguardian_1");
    scene.stat(HITPOINTS, 60, 99);
    scene.inventory.push(scene.held("lobster", 0));
    scene.refresh();
    let mut request = exact_request(&scene);
    request.escape_reserve = Some(TOP_TIER);
    let mut harness = Harness::new(&scene, request);
    harness.machine.set_escape_walk(7);
    let (end, clicked) = run(&mut harness, &scene, 1, 4);
    assert_eq!(end, None, "60 Hitpoints with food still fight");
    assert!(clicked.contains(&7), "the fight attacks: {clicked:?}");
}

/// An escape no one priced is not an escape the fight may bet on: with an
/// unknown walk the fight attacks nothing and hands over.
#[test]
fn an_unpriced_escape_never_attacks_on_trust() {
    if !api::test_support::game_data_available() {
        return;
    }
    let scene = Scene::new("macro_rivertrollguardian_1");
    let mut request = exact_request(&scene);
    request.escape_reserve = Some(TOP_TIER);
    let mut harness = Harness::new(&scene, request);
    let (end, clicked) = run(&mut harness, &scene, 1, 4);
    assert_eq!(
        end,
        Some(CombatEnd::Aborted(AbortReason::Unprotected(
            Unprotected::NoFood
        )))
    );
    assert!(
        clicked.is_empty(),
        "no attack on an unpriced walk: {clicked:?}"
    );
}

/// The reserve is opt-in. The same low Hitpoints and no food, with no
/// reserve, fight on as every ordinary fight does.
#[test]
fn a_request_without_a_reserve_keeps_the_ordinary_lines() {
    if !api::test_support::game_data_available() {
        return;
    }
    let mut scene = Scene::new("macro_rivertrollguardian_1");
    scene.stat(HITPOINTS, 60, 99);
    scene.refresh();
    let mut harness = Harness::new(&scene, exact_request(&scene));
    let (end, clicked) = run(&mut harness, &scene, 1, 4);
    assert_eq!(end, None);
    assert!(clicked.contains(&7), "{clicked:?}");
}

/// An admitted bite the machine has not seen settle is what the Guardian's
/// upkeep guard inherits: it must not eat again for a bite already sent, and
/// it must know the lock that bite admitted.
#[test]
fn the_machine_reports_the_bite_it_admitted_for_the_hand_over() {
    if !api::test_support::game_data_available() {
        return;
    }
    let mut scene = Scene::new("macro_rivertrollguardian_1");
    scene.stat(HITPOINTS, 99, 99);
    scene.refresh();
    let mut request = exact_request(&scene);
    request.escape_reserve = Some(TOP_TIER);
    let mut harness = Harness::new(&scene, request);
    harness.machine.set_escape_walk(7);
    attack(harness.pending(&scene, 1));
    scene.install();
    scene.refresh();
    assert!(harness.pending(&scene, 2).is_none());
    assert!(harness.machine.pending_bite().is_none(), "nothing sent yet");
    // The troll hits us under the eat line of its tier (2 * 2 + 1) with food in
    // the pack: the machine bites.
    scene.stat(HITPOINTS, 4, 99);
    scene.face_us();
    scene.inventory.push(scene.held("lobster", 0));
    scene.refresh();
    if let Poll::Ready(result) = harness.poll(&scene.snapshot, 3) {
        panic!("the fight ended instead of biting: {result:?}");
    }
    let batch = harness.drain(None);
    let rows = row_names(&batch);
    assert!(rows.contains("Eat"), "low Hitpoints eat first: {rows}");
    let bite = harness
        .machine
        .pending_bite()
        .expect("the admitted bite is reported until it settles");
    let lobster = scene.data.item_by_alias("lobster").unwrap().id;
    assert_eq!(bite.id, lobster);
    assert!(
        harness.machine.handoff_schedule().pending(OpKind::Eat),
        "the schedule keeps the bite pending"
    );
}
