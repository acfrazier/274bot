use super::fight_field::choose_fight_field_npc;
use super::*;
use api::snapshot::{NpcView, WorldTile};

fn npc(index: usize, distance: i32, size: i32) -> NpcView {
    NpcView {
        index,
        r#type: Some(1),
        name: Some("Npc".into()),
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

fn receipt_for(index: i32) -> FightFieldScriptReceipt {
    FightFieldScriptReceipt {
        index,
        size: 2,
        ..FightFieldScriptReceipt::default()
    }
}

#[test]
fn chooses_nearest_size_ge_1_not_array_first() {
    let rows = [npc(1, 0, 0), npc(3, 8, 1), npc(11, 1, 2)];
    let chosen = choose_fight_field_npc(&rows, None).expect("nearest size>=1");
    assert_eq!(chosen.index, 11);
    assert_eq!(chosen.size, 2);
}

#[test]
fn receipt_index_selects_that_row_not_nearest() {
    let rows = [npc(1, 0, 0), npc(3, 8, 1), npc(11, 1, 2)];
    let receipt = receipt_for(3);
    let chosen = choose_fight_field_npc(&rows, Some(&receipt)).expect("receipt index 3");
    assert_eq!(chosen.index, 3);
    let nearest = receipt_for(11);
    let chosen = choose_fight_field_npc(&rows, Some(&nearest)).expect("receipt index 11");
    assert_eq!(chosen.index, 11);
}
