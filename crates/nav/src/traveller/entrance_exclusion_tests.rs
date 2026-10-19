use super::*;
use crate::world_state::{EntranceRequirement, WorldState};

const FORBIDDEN_HELD: i32 = 4242;
const FORBIDDEN_WORN: i32 = 4343;

fn exclusion_route(forbidden_held: bool, forbidden_worn: bool) -> Route {
    let mut edge = ladder_edge();
    if forbidden_held {
        edge.forbidden_held.push(FORBIDDEN_HELD);
    }
    if forbidden_worn {
        edge.forbidden_worn.push(FORBIDDEN_WORN);
    }
    Route {
        dest: edge.to,
        legs: vec![Leg::Transport {
            edge: Box::new(edge),
        }],
        ticks: 2.0,
    }
}

fn observe_empty_inventory_and_equipment(c: &mut Client) {
    // A posted zero-count slot makes the snapshot's inventory observation ready.
    plant_inv_item(c, FORBIDDEN_HELD);
    clear_inventory_item(c);
    // A posted zero-count equipment slot makes the equipment observation ready.
    plant_equipment_item(c, FORBIDDEN_WORN);
    clear_equipment_item(c);
}

fn expected_tile(scene: (i32, i32)) -> WorldTile {
    WorldTile {
        x: 3200 + scene.0,
        z: 3200 + scene.1,
        level: 0,
    }
}

fn assert_no_transport_send(rec: &FollowRec, expected_walks: &[(i32, i32)]) {
    assert_eq!(
        rec.walked, expected_walks,
        "no new approach or reapproach walk"
    );
    assert_eq!(rec.loc_ops, 0, "no loc operation");
    assert_eq!(rec.loc_uses, 0, "no held-item-on-loc operation");
    assert_eq!(rec.npc_ops, 0, "no NPC operation");
    assert_eq!(rec.held_ops, 0, "no held-item operation");
    assert_eq!(rec.if_buttons, 0, "no interface button operation");
    assert_eq!(rec.pause_buttons, 0, "no pause-button operation");
    assert!(rec.sink.steps.is_empty(), "no protocol operation");
    assert!(rec.sink.strings.is_empty(), "no cheat or text operation");
}

fn assert_refused(outcome: Option<TravelOutcome>, at: WorldTile, requirement: EntranceRequirement) {
    assert_eq!(
        outcome,
        Some(TravelOutcome::EntranceRefused {
            at,
            leg: 0,
            requirement,
        })
    );
}

#[test]
fn follow_refuses_newly_held_exclusion_before_transport_send() {
    let mut c = scene_client();
    observe_empty_inventory_and_equipment(&mut c);
    plant_ladder(&mut c, Some("Climb"));
    let route = exclusion_route(true, false);
    let edge = match &route.legs[0] {
        Leg::Transport { edge } => edge.as_ref(),
        Leg::Walk { .. } => unreachable!(),
    };
    let entrance = edge.at;
    let mut snap = snap_at(&mut c, 2, 1);
    assert_eq!(
        WorldState::from_snapshot(&snap).exclusion_failure(edge),
        None,
        "the route starts from observed empty inventory and equipment"
    );
    let mut rec = FollowRec {
        route: Some((2, 1)),
        ..FollowRec::default()
    };
    let mut traveller = Traveller::new();
    let mut options = TravelOptions::default();

    assert!(traveller
        .follow(&mut rec, &snap, route.clone(), &mut options)
        .is_none());
    assert_eq!(rec.walked.len(), 1, "the legal route starts its approach");
    assert_eq!(rec.loc_ops, 0);
    let approach = rec.walked[0];

    plant_inv_item(&mut c, FORBIDDEN_HELD);
    plant_player(&mut c, approach.0, approach.1);
    bump_rebuild(&mut c, &mut snap);
    assert_eq!(
        WorldState::from_snapshot(&snap).exclusion_failure(edge),
        Some(EntranceRequirement::ForbiddenHeld {
            id: FORBIDDEN_HELD,
            at: entrance,
        }),
        "the rebuilt snapshot observes the newly held forbidden item"
    );
    assert_refused(
        traveller.follow(&mut rec, &snap, route, &mut options),
        expected_tile(approach),
        EntranceRequirement::ForbiddenHeld {
            id: FORBIDDEN_HELD,
            at: entrance,
        },
    );
    assert_no_transport_send(&rec, &[approach]);
}

#[test]
fn follow_refuses_newly_worn_exclusion_before_transport_send() {
    let mut c = scene_client();
    observe_empty_inventory_and_equipment(&mut c);
    plant_ladder(&mut c, Some("Climb"));
    let route = exclusion_route(false, true);
    let edge = match &route.legs[0] {
        Leg::Transport { edge } => edge.as_ref(),
        Leg::Walk { .. } => unreachable!(),
    };
    let entrance = edge.at;
    let mut snap = snap_at(&mut c, 2, 1);
    assert_eq!(
        WorldState::from_snapshot(&snap).exclusion_failure(edge),
        None
    );
    let mut rec = FollowRec {
        route: Some((2, 1)),
        ..FollowRec::default()
    };
    let mut traveller = Traveller::new();
    let mut options = TravelOptions::default();

    assert!(traveller
        .follow(&mut rec, &snap, route.clone(), &mut options)
        .is_none());
    assert_eq!(rec.walked.len(), 1, "the legal route starts its approach");
    assert_eq!(rec.loc_ops, 0);
    let approach = rec.walked[0];

    plant_equipment_item(&mut c, FORBIDDEN_WORN);
    plant_player(&mut c, approach.0, approach.1);
    bump_rebuild(&mut c, &mut snap);
    assert_eq!(
        WorldState::from_snapshot(&snap).exclusion_failure(edge),
        Some(EntranceRequirement::ForbiddenWorn {
            id: FORBIDDEN_WORN,
            at: entrance,
        }),
        "the rebuilt snapshot observes the newly worn forbidden item"
    );
    assert_refused(
        traveller.follow(&mut rec, &snap, route, &mut options),
        expected_tile(approach),
        EntranceRequirement::ForbiddenWorn {
            id: FORBIDDEN_WORN,
            at: entrance,
        },
    );
    assert_no_transport_send(&rec, &[approach]);
}

#[test]
fn follow_refuses_unknown_inventory_instead_of_assuming_empty() {
    let mut c = scene_client();
    plant_ladder(&mut c, Some("Climb"));
    let route = exclusion_route(true, false);
    let edge = match &route.legs[0] {
        Leg::Transport { edge } => edge.as_ref(),
        Leg::Walk { .. } => unreachable!(),
    };
    let entrance = edge.at;
    let snap = snap_at(&mut c, 2, 1);
    assert_eq!(
        WorldState::from_snapshot(&snap).exclusion_failure(edge),
        Some(EntranceRequirement::InventoryNotObserved { at: entrance })
    );
    let mut rec = FollowRec {
        route: Some((2, 1)),
        ..FollowRec::default()
    };
    let mut traveller = Traveller::new();
    let mut options = TravelOptions::default();

    assert_refused(
        traveller.follow(&mut rec, &snap, route, &mut options),
        expected_tile((2, 1)),
        EntranceRequirement::InventoryNotObserved { at: entrance },
    );
    assert_no_transport_send(&rec, &[]);
}

#[test]
fn follow_refuses_unknown_equipment_instead_of_assuming_empty() {
    let mut c = scene_client();
    plant_ladder(&mut c, Some("Climb"));
    let route = exclusion_route(false, true);
    let edge = match &route.legs[0] {
        Leg::Transport { edge } => edge.as_ref(),
        Leg::Walk { .. } => unreachable!(),
    };
    let entrance = edge.at;
    let snap = snap_at(&mut c, 2, 1);
    assert_eq!(
        WorldState::from_snapshot(&snap).exclusion_failure(edge),
        Some(EntranceRequirement::EquipmentNotObserved { at: entrance })
    );
    let mut rec = FollowRec {
        route: Some((2, 1)),
        ..FollowRec::default()
    };
    let mut traveller = Traveller::new();
    let mut options = TravelOptions::default();

    assert_refused(
        traveller.follow(&mut rec, &snap, route, &mut options),
        expected_tile((2, 1)),
        EntranceRequirement::EquipmentNotObserved { at: entrance },
    );
    assert_no_transport_send(&rec, &[]);
}

#[test]
fn follow_allows_observed_empty_exclusions_to_send_transport() {
    let mut c = scene_client();
    observe_empty_inventory_and_equipment(&mut c);
    plant_ladder(&mut c, Some("Climb"));
    let route = exclusion_route(true, true);
    let edge = match &route.legs[0] {
        Leg::Transport { edge } => edge.as_ref(),
        Leg::Walk { .. } => unreachable!(),
    };
    let snap = snap_at(&mut c, 2, 3);
    assert_eq!(
        WorldState::from_snapshot(&snap).exclusion_failure(edge),
        None
    );
    let mut rec = FollowRec {
        route: Some((2, 3)),
        ..FollowRec::default()
    };
    let mut traveller = Traveller::new();
    let mut options = TravelOptions::default();

    assert!(traveller
        .follow(&mut rec, &snap, route, &mut options)
        .is_none());
    assert_eq!(
        rec.loc_ops, 1,
        "observed empty state permits the loc operation"
    );
    assert!(rec.walked.is_empty());
}
