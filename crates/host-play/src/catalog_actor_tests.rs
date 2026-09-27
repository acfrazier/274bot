use super::actor::choose_actor_observation_npc;
use super::*;
use api::snapshot::{NpcView, WorldTile};

fn npc(index: usize, distance: i32, size: i32, name: &str) -> NpcView {
    NpcView {
        index,
        r#type: Some(1),
        name: Some(name.into()),
        actions: vec![Some("Attack".into())],
        tile: WorldTile {
            x: 3201,
            z: 3205,
            level: 0,
        },
        distance,
        animation: 0,
        pose_animation: 0,
        orientation: 0,
        target_orientation: 0,
        overhead_text: None,
        spot_animation: -1,
        health: 5,
        total_health: 5,
        face_entity: -1,
        target: None,
        moving: false,
        running: false,
        in_combat: false,
        level: 2,
        size,
        network: WorldTile {
            x: 3205,
            z: 3201,
            level: 0,
        },
        x: 3201,
        z: 3205,
        yaw: 0,
    }
}

fn receipt_for(index: i32) -> ActorObservationScriptReceipt {
    ActorObservationScriptReceipt {
        npc: ActorObservationNpcFact {
            index,
            name: Some("Near".into()),
            size: 2,
            ..ActorObservationNpcFact::default()
        },
        ..ActorObservationScriptReceipt::default()
    }
}

#[test]
fn chooses_nearest_size_ge_1_not_array_first() {
    let rows = [
        npc(1, 0, 0, "Zero"),
        npc(3, 8, 1, "Far"),
        npc(11, 1, 2, "Near"),
    ];
    let chosen = choose_actor_observation_npc(&rows, None).expect("nearest size>=1");
    assert_eq!(chosen.index, 11);
    assert_eq!(chosen.name.as_deref(), Some("Near"));
    assert_eq!(chosen.size, 2);
}

#[test]
fn receipt_index_selects_that_row_not_nearest() {
    let rows = [
        npc(1, 0, 0, "Zero"),
        npc(3, 8, 1, "Far"),
        npc(11, 1, 2, "Near"),
    ];
    let receipt = receipt_for(3);
    let chosen = choose_actor_observation_npc(&rows, Some(&receipt)).expect("receipt index 3");
    assert_eq!(chosen.index, 3);
    assert_eq!(chosen.name.as_deref(), Some("Far"));
    let nearest = receipt_for(11);
    let chosen = choose_actor_observation_npc(&rows, Some(&nearest)).expect("receipt index 11");
    assert_eq!(chosen.index, 11);
}
