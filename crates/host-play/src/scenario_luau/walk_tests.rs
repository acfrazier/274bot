//! Failure modes for the revision-289 walk family and settled closures:
//! - a route that L8c needs (courtyard, north road across z=3264, south
//!   past the old courtyard box, East Ardougne door room to the south bank)
//!   is refused, or the old courtyard box is still the only ground;
//! - a source or target in a square with a zone trigger, on another level,
//!   with a radius above 1, or an actor already in combat is admitted;
//! - a door other than the contracted loc, tile and Open option is admitted;
//! - a walk root closes on the PLAYER_INFO that first shows the arrival,
//!   before the next cycle's map-zone trigger ran, or keeps an old arrival
//!   after the player moved away, or closes when the last send opened a door;
//! - a settling bank close closes on its first record, a non-settling Bury
//!   waits needlessly, or a record shown on the send's own PLAYER_INFO
//!   counts as fresh.

use api::snapshot::{
    ActorView, ChatLineView, GameSnapshot, LocalPlayerView, PlayerView, WorldTile,
};

use super::{WalkRoot, WalkSend};
use crate::scenario_luau::actions::ActionRoot;
use crate::scenario_luau::contracts::{select, FileAction, Heads, R289_CONTENT, R289_ENGINE};
use crate::scenario_luau::plan::{in_walk_ground, walk_door, WalkContract};

const R289: Heads<'static> = Heads {
    revision: 289,
    engine: R289_ENGINE,
    content: R289_CONTENT,
};

fn tile(x: i32, z: i32, level: i32) -> WorldTile {
    WorldTile { x, z, level }
}

fn snapshot(at: WorldTile, player_info: u64, moving: bool, in_combat: bool) -> GameSnapshot {
    let actor = ActorView {
        name: Some("walk-fixture".into()),
        actions: Vec::new(),
        tile: at,
        distance: 0,
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
        moving,
        running: false,
        in_combat,
    };
    let mut snapshot = GameSnapshot::new();
    snapshot.seed_ingame(2);
    snapshot.seed_tile(at);
    snapshot.seed_player_info_generation(player_info);
    snapshot.seed_local_player(LocalPlayerView {
        player: PlayerView {
            index: 0,
            network: at,
            actor,
            combat_level: 3,
            skill_level: 1,
            headicons: 0,
            weapon: None,
        },
        energy: 100,
        weight: 0,
        animation_update: None,
    });
    snapshot
}

#[test]
fn the_tier_one_routes_lie_in_the_audited_ground_and_others_do_not() {
    let contract = WalkContract;
    for (from, to) in [
        (tile(3220, 3220, 0), tile(3220, 3216, 0)),
        (tile(3220, 3216, 0), tile(3220, 3212, 0)),
        (tile(3220, 3220, 0), tile(3220, 3264, 0)),
        (tile(2671, 3316, 0), tile(2655, 3286, 0)),
    ] {
        assert_eq!(
            contract.admits(&snapshot(from, 1, false, false), to, 0),
            Ok(()),
            "{from:?} -> {to:?}"
        );
    }
    let courtyard = snapshot(tile(3220, 3220, 0), 1, false, false);
    for target in [
        // Al Kharid square 52_50 has [zone] triggers; Draynor manor 48_52 too.
        tile(3330, 3210, 0),
        tile(3100, 3350, 0),
        tile(3220, 3218, 1),
    ] {
        assert!(
            contract.admits(&courtyard, target, 0).is_err(),
            "{target:?}"
        );
    }
    assert!(contract.admits(&courtyard, tile(3220, 3216, 0), 2).is_err());
    let fighting = snapshot(tile(3220, 3220, 0), 1, false, true);
    assert!(contract.admits(&fighting, tile(3220, 3216, 0), 0).is_err());
    assert!(!in_walk_ground(tile(3330, 3210, 0)));
}

#[test]
fn only_the_contracted_door_open_is_a_route_continuation() {
    let door = tile(2669, 3316, 0);
    assert!(walk_door(1530, door, 1));
    assert!(!walk_door(1530, door, 2));
    assert!(!walk_door(1531, door, 1));
    assert!(!walk_door(1530, tile(2669, 3317, 0), 1));
}

fn walk_root(send: WalkSend) -> WalkRoot {
    WalkRoot {
        request: 7,
        route_pending: false,
        session: 0,
        target: send.aim,
        radius: 0,
        send: Some(send),
        subject: false,
        arrived_at: None,
        painted_route: None,
    }
}

#[test]
fn a_walk_closes_one_player_info_after_its_settled_arrival() {
    let aim = tile(3220, 3264, 0);
    let send = WalkSend {
        nav_request: 7,
        aim,
        sent_from: tile(3220, 3220, 0),
        raw_player_info: 10,
        commit: 1,
        door: None,
    };
    let mut root = walk_root(send);
    // Still walking, and on the send's own PLAYER_INFO: no arrival.
    assert!(root
        .settled_closure(&snapshot(aim, 20, true, false))
        .is_none());
    assert!(root
        .settled_closure(&snapshot(aim, 10, false, false))
        .is_none());
    // First arrival frame: the map-zone trigger has not yet run.
    assert!(root
        .settled_closure(&snapshot(aim, 21, false, false))
        .is_none());
    // Leaving the aim resets the arrival; it must be shown afresh.
    assert!(root
        .settled_closure(&snapshot(tile(3220, 3263, 0), 22, false, false))
        .is_none());
    assert!(root
        .settled_closure(&snapshot(aim, 23, false, false))
        .is_none());
    let (closed, first) = root
        .settled_closure(&snapshot(aim, 24, false, false))
        .expect("the next PLAYER_INFO closes the root");
    assert_eq!((closed.commit, first), (1, 23));

    let mut door = walk_root(WalkSend {
        door: Some(1530),
        ..send
    });
    for player_info in [21, 22, 23] {
        assert!(door
            .settled_closure(&snapshot(aim, player_info, false, false))
            .is_none());
    }
}

fn bank_snapshot(open: bool, player_info: u64) -> GameSnapshot {
    let mut snapshot = snapshot(tile(3253, 3419, 0), player_info, false, false);
    if open {
        snapshot.seed_bank_observation(5382, 1, Some(Vec::new()), Vec::new());
    }
    snapshot
}

#[test]
fn a_settling_close_waits_one_player_info_and_bury_does_not() {
    let open = bank_snapshot(true, 10);
    let contract = select(R289, FileAction::CloseModal, &open).unwrap();
    let mut close = ActionRoot::new(1, 0, FileAction::CloseModal, contract, true);
    close.note_send(&open);
    // A closed bank on the send's own PLAYER_INFO is not a post-send record.
    assert!(close.terminal_closure(&bank_snapshot(false, 10)).is_none());
    assert!(close.terminal_closure(&bank_snapshot(false, 11)).is_none());
    let closed = close
        .terminal_closure(&bank_snapshot(false, 12))
        .expect("the queued reorganize cycle has run");
    assert_eq!(closed["first_terminal_player_info"], 11);

    let mut sent = bank_snapshot(false, 10);
    sent.seed_chat_lines(vec![ChatLineView {
        sequence: 4,
        type_: 0,
        username: None,
        text: "You dig a hole in the ground...".into(),
    }]);
    let bury = FileAction::HeldOp { item: 526, op: 1 };
    let contract = select(R289, bury, &sent).unwrap();
    let mut root = ActionRoot::new(2, 0, bury, contract, true);
    root.note_send(&sent);
    let mut buried = bank_snapshot(false, 11);
    buried.seed_chat_lines(vec![ChatLineView {
        sequence: 5,
        type_: 0,
        username: None,
        text: "You bury the bones.".into(),
    }]);
    assert!(root.terminal_closure(&buried).is_some());
}
