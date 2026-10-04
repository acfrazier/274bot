use super::TravelEvent;
use api::interact::{Driver, SendReason};
use api::prot::Out;
use api::snapshot::{GameSnapshot, WorldTile};
use client::client::{Client, ClientConfig, ClientPlayer, MiniMenuAction};
use client::config::if_type::{ButtonType, ComponentType};
use client::config::{IfType, IfTypeMut, LocType, NpcType, ObjType};
use client::dash3d::ClientNpc;
use client::io::{ClientStream, ServerProt};
use std::sync::Arc;

use crate::essence::essence_session_for_wizard;
use crate::grid::StepGrid;
use crate::router::{find_on_grid, Leg, Route};
use crate::tile::Tile;
use crate::transport::{DoorDir, TransportEdge, TransportKind, CELLAR_SHIFT, SHANTAY_HENGE_LOC_ID};
use crate::traveller::{
    door_tile, FollowRun, HopFailure, LegPhase, NavStatus, Poll, TransportHop, TravelOptions,
    TravelOutcome, Traveller,
};

#[path = "traveller/npc_hop_tests.rs"]
mod npc_hop_tests;

#[path = "traveller/n1_tests.rs"]
mod n1_tests;

#[test]
fn no_route_ticks_idle() {
    let mut t = Traveller::new();
    let mut r = Rec::default();
    assert_eq!(
        t.tick(
            &mut r,
            Tile {
                x: 0,
                z: 0,
                level: 0
            },
            false
        ),
        NavStatus::Idle
    );
}

#[test]
fn remaining_is_empty_without_route() {
    let t = Traveller::new();
    assert!(t.remaining_walk_tiles(None).is_empty());
}

#[test]
fn remaining_covers_armed_route_tiles() {
    let mut t = Traveller::new();
    t.arm(
        find_on_grid(
            &StepGrid::fixture_open_3x3(),
            Tile {
                x: 0,
                z: 0,
                level: 0,
            },
            Tile {
                x: 2,
                z: 2,
                level: 0,
            },
        )
        .unwrap(),
    );
    let tiles = t.remaining_walk_tiles(None);
    assert_eq!(
        tiles.first(),
        Some(&Tile {
            x: 0,
            z: 0,
            level: 0
        })
    );
    assert_eq!(
        tiles.last(),
        Some(&Tile {
            x: 2,
            z: 2,
            level: 0
        })
    );
}

#[test]
fn remaining_is_empty_when_standing_on_dest() {
    let mut t = Traveller::new();
    t.arm(
        find_on_grid(
            &StepGrid::fixture_open_3x3(),
            Tile {
                x: 0,
                z: 0,
                level: 0,
            },
            Tile {
                x: 2,
                z: 2,
                level: 0,
            },
        )
        .unwrap(),
    );
    assert!(t
        .remaining_walk_tiles(Some(Tile {
            x: 2,
            z: 2,
            level: 0
        }))
        .is_empty());
}

#[test]
fn remaining_skips_completed_legs_and_connects_doors() {
    let mut t = Traveller::new();
    t.arm(
        find_on_grid(
            &StepGrid::fixture_door_corridor(),
            Tile {
                x: 0,
                z: 0,
                level: 0,
            },
            Tile {
                x: 4,
                z: 0,
                level: 0,
            },
        )
        .unwrap(),
    );
    // Standing on the door's from-tile: the first walk leg is done and
    // the door connects straight to the far walk leg (no duplicate
    // crossing tile).
    let tiles = t.remaining_walk_tiles(Some(Tile {
        x: 1,
        z: 0,
        level: 0,
    }));
    let expected = vec![
        Tile {
            x: 1,
            z: 0,
            level: 0,
        },
        Tile {
            x: 3,
            z: 0,
            level: 0,
        },
        Tile {
            x: 4,
            z: 0,
            level: 0,
        },
    ];
    assert_eq!(tiles, expected);
}

#[test]
fn remaining_trims_current_leg_to_here() {
    let mut t = Traveller::new();
    t.arm(
        find_on_grid(
            &StepGrid::fixture_open_1x40(),
            Tile {
                x: 0,
                z: 0,
                level: 0,
            },
            Tile {
                x: 39,
                z: 0,
                level: 0,
            },
        )
        .unwrap(),
    );
    // Mid-leg: the line starts at the player's tile, not the leg start.
    let tiles = t.remaining_walk_tiles(Some(Tile {
        x: 15,
        z: 0,
        level: 0,
    }));
    assert_eq!(
        tiles.first(),
        Some(&Tile {
            x: 15,
            z: 0,
            level: 0
        })
    );
    assert_eq!(tiles.len(), 25);
    assert_eq!(
        tiles.last(),
        Some(&Tile {
            x: 39,
            z: 0,
            level: 0
        })
    );
}

#[test]
fn arm_queues_dest_and_clear_drops_it() {
    let mut t = Traveller::new();
    t.arm(
        find_on_grid(
            &StepGrid::fixture_open_3x3(),
            Tile {
                x: 0,
                z: 0,
                level: 0,
            },
            Tile {
                x: 2,
                z: 2,
                level: 0,
            },
        )
        .unwrap(),
    );
    assert_eq!(
        t.queued(),
        Some(Tile {
            x: 2,
            z: 2,
            level: 0
        })
    );
    t.clear();
    assert_eq!(t.queued(), None);
}

#[test]
fn walk_leg_sends_walk_toward_dest() {
    let mut t = Traveller::new();
    t.arm(
        find_on_grid(
            &StepGrid::fixture_open_3x3(),
            Tile {
                x: 0,
                z: 0,
                level: 0,
            },
            Tile {
                x: 2,
                z: 2,
                level: 0,
            },
        )
        .unwrap(),
    );
    let mut r = Rec {
        route: Some((0, 0)),
        ..Rec::default()
    };
    assert_eq!(
        t.tick(
            &mut r,
            Tile {
                x: 0,
                z: 0,
                level: 0
            },
            false
        ),
        NavStatus::Walking
    );
    assert!(r.walked.is_some());
}

#[test]
fn long_walk_leg_hop_targets_fifteen_ahead() {
    let mut t = Traveller::new();
    t.arm(
        find_on_grid(
            &StepGrid::fixture_open_1x40(),
            Tile {
                x: 0,
                z: 0,
                level: 0,
            },
            Tile {
                x: 39,
                z: 0,
                level: 0,
            },
        )
        .unwrap(),
    );
    let mut r = Rec {
        route: Some((0, 0)),
        ..Rec::default()
    };
    t.tick(
        &mut r,
        Tile {
            x: 0,
            z: 0,
            level: 0,
        },
        false,
    );
    // Far end is 39 away (> 20): hop to a tile ~15 steps ahead.
    let (x, z) = r.walked.expect("walk sent");
    assert!((10..=20).contains(&x), "hop target x was {x}");
    assert_eq!(z, 0);
}

#[test]
fn long_walk_leg_second_hop_stays_ahead() {
    let mut t = Traveller::new();
    t.arm(
        find_on_grid(
            &StepGrid::fixture_open_1x40(),
            Tile {
                x: 0,
                z: 0,
                level: 0,
            },
            Tile {
                x: 39,
                z: 0,
                level: 0,
            },
        )
        .unwrap(),
    );
    let mut r = Rec {
        route: Some((15, 0)),
        ..Rec::default()
    };
    // Second hop from 15 tiles in: the target must stay ahead of
    // `here`, not point back toward the leg start.
    t.tick(
        &mut r,
        Tile {
            x: 15,
            z: 0,
            level: 0,
        },
        false,
    );
    let (x, z) = r.walked.expect("walk sent");
    assert!((25..=35).contains(&x), "second-hop target x was {x}");
    assert_eq!(z, 0);
}

#[test]
fn arrived_on_dest_clears_and_reports_arrived() {
    let mut t = Traveller::new();
    t.arm(
        find_on_grid(
            &StepGrid::fixture_open_3x3(),
            Tile {
                x: 0,
                z: 0,
                level: 0,
            },
            Tile {
                x: 2,
                z: 2,
                level: 0,
            },
        )
        .unwrap(),
    );
    let mut r = Rec::default();
    assert_eq!(
        t.tick(
            &mut r,
            Tile {
                x: 2,
                z: 2,
                level: 0
            },
            false
        ),
        NavStatus::Arrived
    );
    assert_eq!(t.queued(), None);
}

#[test]
fn door_open_walks_without_requiring_op_loc() {
    let mut t = Traveller::new();
    t.arm(
        find_on_grid(
            &StepGrid::fixture_door_corridor(),
            Tile {
                x: 0,
                z: 0,
                level: 0,
            },
            Tile {
                x: 4,
                z: 0,
                level: 0,
            },
        )
        .unwrap(),
    );
    let mut r = Rec {
        route: Some((1, 0)),
        ..Rec::default()
    };
    // skip to door by standing on from-tile; door already open
    assert_eq!(
        t.tick(
            &mut r,
            Tile {
                x: 1,
                z: 0,
                level: 0
            },
            true
        ),
        NavStatus::Door
    );
    assert!(r.walked.is_some(), "open door walks through");
    // locs may be 0: OP_LOC1 on an open loc Closes it
}

#[test]
fn current_door_is_the_armed_door_leg() {
    let mut t = Traveller::new();
    t.arm(
        find_on_grid(
            &StepGrid::fixture_door_corridor(),
            Tile {
                x: 0,
                z: 0,
                level: 0,
            },
            Tile {
                x: 4,
                z: 0,
                level: 0,
            },
        )
        .unwrap(),
    );
    assert_eq!(
        t.current_door(Tile {
            x: 1,
            z: 0,
            level: 0
        }),
        Some((
            Tile {
                x: 2,
                z: 0,
                level: 0
            },
            1530
        ))
    );
    assert_eq!(
        t.current_door(Tile {
            x: 0,
            z: 0,
            level: 0
        }),
        None
    );
}

#[test]
fn door_closed_only_op_loc() {
    let mut t = Traveller::new();
    t.arm(
        find_on_grid(
            &StepGrid::fixture_door_corridor(),
            Tile {
                x: 0,
                z: 0,
                level: 0,
            },
            Tile {
                x: 4,
                z: 0,
                level: 0,
            },
        )
        .unwrap(),
    );
    let mut r = Rec {
        route: Some((1, 0)),
        ..Rec::default()
    };
    assert_eq!(
        t.tick(
            &mut r,
            Tile {
                x: 1,
                z: 0,
                level: 0
            },
            false
        ),
        NavStatus::Door
    );
    assert!(r.locs >= 1);
    assert!(r.walked.is_none());
}

#[test]
fn budget_exceeded_reports_budget_and_clears() {
    let mut t = Traveller::new();
    t.arm(
        find_on_grid(
            &StepGrid::fixture_open_3x3(),
            Tile {
                x: 0,
                z: 0,
                level: 0,
            },
            Tile {
                x: 2,
                z: 2,
                level: 0,
            },
        )
        .unwrap(),
    );
    let mut r = Rec {
        route: Some((0, 0)),
        ..Rec::default()
    };
    let mut status = NavStatus::Walking;
    for _ in 0..61 {
        status = t.tick(
            &mut r,
            Tile {
                x: 0,
                z: 0,
                level: 0,
            },
            false,
        );
    }
    assert_eq!(status, NavStatus::Budget);
    assert_eq!(t.queued(), None);
}

#[test]
fn budget_resets_when_here_advances() {
    let mut t = Traveller::new();
    t.arm(
        find_on_grid(
            &StepGrid::fixture_open_3x3(),
            Tile {
                x: 0,
                z: 0,
                level: 0,
            },
            Tile {
                x: 2,
                z: 2,
                level: 0,
            },
        )
        .unwrap(),
    );
    let mut r = Rec {
        route: Some((0, 0)),
        ..Rec::default()
    };
    let mut status;
    for _ in 0..59 {
        status = t.tick(
            &mut r,
            Tile {
                x: 0,
                z: 0,
                level: 0,
            },
            false,
        );
        assert_eq!(status, NavStatus::Walking);
    }
    // The 60th tick moves off the stuck tile: the clock restarts, so
    // the traveller keeps walking instead of tripping the budget.
    status = t.tick(
        &mut r,
        Tile {
            x: 1,
            z: 0,
            level: 0,
        },
        false,
    );
    assert_eq!(status, NavStatus::Walking);
}

#[test]
fn walk_leg_falls_back_to_next_tile_when_dest_rejected() {
    let mut t = Traveller::new();
    t.arm(
        find_on_grid(
            &StepGrid::fixture_open_3x3(),
            Tile {
                x: 0,
                z: 0,
                level: 0,
            },
            Tile {
                x: 2,
                z: 2,
                level: 0,
            },
        )
        .unwrap(),
    );
    let mut r = Rec {
        route: Some((0, 0)),
        reject_far: true,
        ..Rec::default()
    };
    // The leg far end (2,2) is 2 away and the driver rejects it; the
    // traveller retries the adjacent tile so the hop still goes out.
    assert_eq!(
        t.tick(
            &mut r,
            Tile {
                x: 0,
                z: 0,
                level: 0
            },
            false
        ),
        NavStatus::Walking
    );
    assert!(r.walked.is_some(), "adjacent fallback hop was sent");
}

// --- Task 14: pollable high-level follow (drive legs via interact+settle) ---

/// An attached, ingame client with the scene built at base (3200, 3200).
fn scene_client() -> Client {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("local addr");
    let stream = ClientStream::connect(&addr.ip().to_string(), addr.port()).expect("connect");
    // Keep the listener alive so the connect stays established.
    std::mem::forget(listener);
    let mut c = Client::new(ClientConfig {
        host: "127.0.0.1".into(),
        port: 43594,
        cache_dir: "/tmp".into(),
        members: true,
        lowmem: false,
    });
    c.stream = Some(stream);
    c.ingame = true;
    c.scene_state = 2;
    c.map_build_base_x = 3200;
    c.map_build_base_z = 3200;
    c
}

/// Plant the local player at scene (x, z); the actor's world tile lands
/// on (3200 + x, 3200 + z).
fn plant_player(c: &mut Client, x: i32, z: i32) {
    let mut lp = ClientPlayer::at(x, z);
    lp.entity.x = x * 128 + 64;
    lp.entity.z = z * 128 + 64;
    c.local_player = Some(lp);
}

/// Bump every gen and rebuild every family into a fresh snapshot (tick 1).
fn snap_at(c: &mut Client, x: i32, z: i32) -> GameSnapshot {
    plant_player(c, x, z);
    c.bump_gens(ServerProt::REBUILD_NORMAL);
    let mut snap = GameSnapshot::new();
    snap.rebuild(c);
    snap
}

/// Bump every gen and rebuild into the existing snapshot (tick + 1).
fn bump_rebuild(c: &mut Client, snap: &mut GameSnapshot) {
    c.bump_gens(ServerProt::REBUILD_NORMAL);
    snap.rebuild(c);
}

/// A wall loc (id 1, "Ladder") at scene (3, 4) → world tile (3203, 3204).
fn plant_ladder(c: &mut Client, op: Option<&str>) {
    let typecode = 0x4000_0000 + (1 << 14) + 3 + (4 << 7);
    {
        let cache = Arc::get_mut(&mut c.cache).expect("sole cache owner");
        while cache.locs.len() <= 1 {
            cache.locs.push(LocType::default());
        }
        cache.locs[1] = LocType {
            id: 1,
            name: "Ladder".into(),
            op: vec![op.map(str::to_string), None, None, None, None],
            ..Default::default()
        };
    }
    c.world
        .set_wall(0, 3, 4, 0, 0, 0, typecode, 1 << 6, 0, 0, 0, 0);
}

/// A wall loc at scene (`scene_x`, 0) — the `at` tile of [`door_edge`]
/// or an offset of it — as the closed door (1530, "Open") or the open
/// state (1531, "Close"), mirroring the Catherby range-house door
/// configs.
fn plant_door(c: &mut Client, open: bool, scene_x: i32) {
    plant_door_at(c, open, scene_x, 0);
}

/// An NPC of cache type `type_id` at scene (x, z) → world tile
/// (3200 + x, 3200 + z) in slot 0, with a usable op 1 ("Pay-fare",
/// the cart-driver shape): the live target a `TransportKind::Npc`
/// edge's `loc_id` resolves through.
fn plant_driver_npc(c: &mut Client, type_id: usize, x: i32, z: i32) {
    plant_npc_ops(c, type_id, x, z, "Cart driver", &["Pay-fare"]);
}

/// The essence-wizard shape: an NPC whose op 1 is Talk-to and op 3
/// the direct teleport (no dialog) — a Npc hop with `option: 3`.
fn plant_wizard_npc(c: &mut Client, type_id: usize, x: i32, z: i32) {
    plant_npc_ops(
        c,
        type_id,
        x,
        z,
        "Essence wizard",
        &["Talk-to", "Talk-to", "Teleport"],
    );
}

/// An NPC of cache type `type_id` at scene (x, z) → world tile
/// (3200 + x, 3200 + z) in slot 0, with `name` and the given op
/// labels (one per action slot).
fn plant_npc_ops(c: &mut Client, type_id: usize, x: i32, z: i32, name: &str, ops: &[&str]) {
    {
        let cache = Arc::get_mut(&mut c.cache).expect("sole cache owner");
        while cache.npcs.len() <= type_id {
            cache.npcs.push(NpcType::default());
        }
        let mut op: Vec<Option<String>> = ops.iter().map(|s| Some((*s).to_string())).collect();
        op.resize(5, None);
        cache.npcs[type_id] = NpcType {
            id: type_id as i32,
            name: name.into(),
            op,
            ..Default::default()
        };
    }
    let mut npc = ClientNpc::at(x, z);
    npc.r#type = Some(type_id);
    npc.entity.x = x * 128 + 64;
    npc.entity.z = z * 128 + 64;
    c.npc_count = 1;
    c.npc_ids = vec![0];
    c.npc = vec![Some(Box::new(npc))];
}

/// A chat page with a BUTTON_CONTINUE child — the "Hello Bwana!" page
/// the cart driver's `opnpc1` opens before the fare choice.
fn plant_continue_dialog(c: &mut Client) {
    let root = 100;
    let id = 101;
    c.set_iface(
        id,
        IfType {
            id: id as i32,
            layer_id: root,
            ..Default::default()
        },
    );
    c.set_iface_mut(
        id,
        IfTypeMut {
            button_type: ButtonType::BUTTON_CONTINUE,
            text: "Click here to continue".into(),
            ..Default::default()
        },
    );
    c.set_iface(
        root as usize,
        IfType {
            id: root,
            layer_id: root,
            children: Some(vec![id as i32]),
            ..Default::default()
        },
    );
    c.chat_modal_id = root;
    c.bump_gens(ServerProt::IF_OPENCHAT);
}

/// A chat fare dialog (root 100 with one BUTTON_OK choice button per
/// option): the shape the cart driver's `opnpc1` opens after the
/// interact. `layer_id` on every component is the chat modal id so
/// the press's visibility check holds.
fn plant_choice_dialog(c: &mut Client, options: &[&str]) {
    let root = 100;
    let children: Vec<i32> = (0..options.len()).map(|i| (101 + i) as i32).collect();
    for (i, text) in options.iter().enumerate() {
        let id = 101 + i;
        c.set_iface(
            id,
            IfType {
                id: id as i32,
                layer_id: root,
                ..Default::default()
            },
        );
        c.set_iface_mut(
            id,
            IfTypeMut {
                button_type: ButtonType::BUTTON_OK,
                text: (*text).to_string(),
                ..Default::default()
            },
        );
    }
    c.set_iface(
        root as usize,
        IfType {
            id: root,
            layer_id: root,
            children: Some(children),
            ..Default::default()
        },
    );
    c.chat_modal_id = root;
    c.bump_gens(ServerProt::IF_OPENCHAT);
}

/// `if_openmain(glidermap)` with one dest IF_BUTTON (com_21..=25).
fn plant_glider_map(c: &mut Client, dest_com: i32) {
    c.chat_modal_id = -1;
    c.main_modal_id = super::GLIDER_MAP_ROOT;
    c.set_iface(
        super::GLIDER_MAP_ROOT as usize,
        IfType {
            id: super::GLIDER_MAP_ROOT,
            r#type: ComponentType::TYPE_LAYER,
            width: 512,
            height: 334,
            children: Some(vec![dest_com]),
            child_x: Some(vec![0]),
            child_y: Some(vec![0]),
            ..Default::default()
        },
    );
    c.set_iface(
        dest_com as usize,
        IfType {
            id: dest_com,
            layer_id: super::GLIDER_MAP_ROOT,
            r#type: ComponentType::TYPE_MODEL,
            width: 59,
            height: 81,
            ..Default::default()
        },
    );
    c.set_iface_mut(
        dest_com as usize,
        IfTypeMut {
            button_type: ButtonType::BUTTON_OK,
            ..Default::default()
        },
    );
    c.bump_gens(ServerProt::IF_OPENMAIN);
}

fn plant_door_at(c: &mut Client, open: bool, scene_x: i32, scene_z: i32) {
    let id = if open { 1531 } else { 1530 };
    {
        let cache = Arc::get_mut(&mut c.cache).expect("sole cache owner");
        while cache.locs.len() <= 1531 {
            cache.locs.push(LocType::default());
        }
        cache.locs[1530] = LocType {
            id: 1530,
            name: "Door".into(),
            op: vec![Some("Open".into()), None, None, None, None],
            ..Default::default()
        };
        cache.locs[1531] = LocType {
            id: 1531,
            name: "Door".into(),
            op: vec![Some("Close".into()), None, None, None, None],
            ..Default::default()
        };
    }
    let typecode = 0x4000_0000 + (id << 14) + scene_x + (scene_z << 7);
    c.world
        .set_wall(0, scene_x, scene_z, 0, 0, 0, typecode, 1 << 6, 0, 0, 0, 0);
}

/// A wall loc at scene (`scene_x`, `scene_z`) with `id`/`name`/`op1`:
/// the generic wall-planting shape (the essence exit portal, …).
fn plant_loc(c: &mut Client, id: i32, name: &str, op1: &str, scene_x: i32, scene_z: i32) {
    plant_loc_sized(c, id, name, op1, scene_x, scene_z, 1, 1, 1);
}

#[allow(clippy::too_many_arguments)] // test helper plants loc geometry fields together
fn plant_loc_sized(
    c: &mut Client,
    id: i32,
    name: &str,
    op1: &str,
    scene_x: i32,
    scene_z: i32,
    width: i32,
    length: i32,
    angle: i32,
) {
    {
        let cache = Arc::get_mut(&mut c.cache).expect("sole cache owner");
        while cache.locs.len() <= id as usize {
            cache.locs.push(LocType::default());
        }
        cache.locs[id as usize] = LocType {
            id,
            name: name.into(),
            op: vec![Some(op1.into()), None, None, None, None],
            width,
            length,
            ..Default::default()
        };
    }
    let typecode = 0x4000_0000 + (id << 14) + scene_x + (scene_z << 7);
    c.world.set_wall(
        0,
        scene_x,
        scene_z,
        0,
        0,
        0,
        typecode,
        angle << 6,
        0,
        0,
        0,
        0,
    );
}

/// A level-0 walk leg over the given (x, z) world tiles.
fn walk_leg(tiles: &[(i32, i32)]) -> Leg {
    Leg::Walk {
        tiles: tiles
            .iter()
            .map(|(x, z)| WorldTile {
                x: *x,
                z: *z,
                level: 0,
            })
            .collect(),
    }
}

/// A Catherby-style door edge on the fixture scene: closed loc 1530 at
/// the door tile (3201, 3200) — the edge's `at` — crossing east to
/// (3203, 3200) (`to` 2 tiles away; the far-side tile between is
/// blocked, so the far-side walk-out lands 2 out).
fn door_edge() -> TransportEdge {
    TransportEdge {
        takeoff: None,
        worn_all_req: Vec::new(),
        kind: TransportKind::Door,
        player_delta: None,
        at: WorldTile {
            x: 3201,
            z: 3200,
            level: 0,
        },
        to: WorldTile {
            x: 3203,
            z: 3200,
            level: 0,
        },
        loc_id: 1530,
        option: 1,
        ticks: 1,
        dir: None,
        open_loc_id: None,
        skill_req: vec![],
        item_req: vec![],
        consumed_req: vec![],
        item_returns: vec![],
        quest_req: vec![],
        varp_req: vec![],
        worn_req: vec![],
        members_req: false,
        wildy_cap: None,
        quest_gates: None,
    }
}

fn web_cut_edges() -> [TransportEdge; 2] {
    let mut knife = door_edge();
    knife.at = WorldTile {
        x: 3202,
        z: 3200,
        level: 0,
    };
    knife.to = WorldTile {
        x: 3204,
        z: 3200,
        level: 0,
    };
    knife.loc_id = 733;
    knife.option = 0;
    knife.ticks = 2;
    knife.dir = Some(DoorDir::E);
    knife.open_loc_id = Some(734);
    knife.item_req = vec![(946, 1)];

    let mut slash = knife.clone();
    slash.option = 1;
    slash.item_req.clear();
    slash.worn_req = vec![1321];
    [knife, slash]
}

fn web_route(edge: TransportEdge) -> Route {
    Route {
        dest: edge.to,
        ticks: 2.0,
        legs: vec![Leg::Transport {
            edge: Box::new(edge),
        }],
    }
}

/// A ladder edge standing at (3202, 3204) → (3202, 3205).
fn ladder_edge() -> TransportEdge {
    TransportEdge {
        takeoff: None,
        worn_all_req: Vec::new(),
        kind: TransportKind::Ladder,
        player_delta: None,
        at: WorldTile {
            x: 3202,
            z: 3204,
            level: 0,
        },
        to: WorldTile {
            x: 3202,
            z: 3205,
            level: 0,
        },
        loc_id: 1,
        option: 1,
        ticks: 2,
        dir: None,
        open_loc_id: None,
        skill_req: vec![],
        item_req: vec![],
        consumed_req: vec![],
        item_returns: vec![],
        quest_req: vec![],
        varp_req: vec![],
        worn_req: vec![],
        members_req: false,
        wildy_cap: None,
        quest_gates: None,
    }
}

fn agility_at(loc_id: i32, at: WorldTile) -> TransportEdge {
    TransportEdge {
        takeoff: None,
        worn_all_req: Vec::new(),
        kind: TransportKind::AgilityShortcut,
        player_delta: None,
        at,
        to: WorldTile {
            x: at.x - 5,
            z: at.z,
            level: at.level,
        },
        loc_id,
        option: 1,
        ticks: 2,
        dir: None,
        open_loc_id: None,
        skill_req: vec![],
        item_req: vec![],
        consumed_req: vec![],
        item_returns: vec![],
        quest_req: vec![],
        varp_req: vec![],
        worn_req: vec![],
        members_req: false,
        wildy_cap: None,
        quest_gates: None,
    }
}

#[test]
fn find_transport_loc_matches_inbound_swing_on_rotated_footprint() {
    let mut c = scene_client();
    // Origin (3205,3209), start at (3209,3209): Chebyshev 4 to origin,
    // on the length-6 angle-1 footprint (x=3205..3210).
    plant_loc_sized(&mut c, 2322, "Ropeswing", "Swing-on", 5, 9, 1, 6, 1);
    let snap = snap_at(&mut c, 9, 9);
    let loc = snap.locs().iter().find(|l| l.id == 2322).expect("planted");
    assert_eq!(
        loc.tile,
        WorldTile {
            x: 3205,
            z: 3209,
            level: 0
        }
    );
    assert_eq!(loc.footprint_width, 6);
    assert_eq!(loc.footprint_length, 1);
    let edge = agility_at(
        2322,
        WorldTile {
            x: 3209,
            z: 3209,
            level: 0,
        },
    );
    let found = super::find_transport_loc(&snap, &edge).expect("footprint covers start");
    assert_eq!(found.id, 2322);
    assert_eq!(found.tile, loc.tile);
}

#[test]
fn find_transport_loc_rejects_unrelated_or_far_candidate_at_gap_4() {
    let mut c = scene_client();
    // 1×1 loc four tiles west of at: origin gap 4, footprint does not reach.
    plant_loc_sized(&mut c, 2322, "Ropeswing", "Swing-on", 5, 9, 1, 1, 0);
    plant_loc_sized(&mut c, 1, "Door", "Open", 1, 1, 1, 1, 0);
    let snap = snap_at(&mut c, 9, 9);
    let at = WorldTile {
        x: 3209,
        z: 3209,
        level: 0,
    };
    assert!(
        super::find_transport_loc(&snap, &agility_at(2322, at)).is_none(),
        "1×1 origin 4 away must not match; radius stays 3"
    );
    assert!(
        super::find_transport_loc(&snap, &agility_at(1, at)).is_none(),
        "unrelated far loc must not match"
    );
    let closed = TransportEdge {
        takeoff: None,
        worn_all_req: Vec::new(),
        kind: TransportKind::Door,
        player_delta: None,
        at: WorldTile {
            x: 3201,
            z: 3200,
            level: 0,
        },
        to: WorldTile {
            x: 3203,
            z: 3200,
            level: 0,
        },
        loc_id: 1530,
        option: 1,
        ticks: 1,
        dir: None,
        open_loc_id: Some(1531),
        skill_req: vec![],
        item_req: vec![],
        consumed_req: vec![],
        item_returns: vec![],
        quest_req: vec![],
        varp_req: vec![],
        worn_req: vec![],
        members_req: false,
        wildy_cap: None,
        quest_gates: None,
    };
    assert!(
        super::find_transport_loc(&snap, &closed).is_none(),
        "closed-door id still required; 1×1 decoy is not 1530"
    );
}

/// Edgeville trapdoor: closed 1568 / open 1570, dest loc-baked +6400.
/// Live `p_telejump(movecoord(coord(), 0, 0, 6400))` lands on the
/// adjacent stand, Chebyshev 1 off that dest.
fn trapdoor_edge() -> TransportEdge {
    TransportEdge {
        takeoff: None,
        worn_all_req: Vec::new(),
        kind: TransportKind::Ladder,
        player_delta: None,
        at: WorldTile {
            x: 3202,
            z: 3204,
            level: 0,
        },
        to: WorldTile {
            x: 3202,
            z: 3204 + CELLAR_SHIFT,
            level: 0,
        },
        loc_id: 1568,
        option: 1,
        ticks: 3,
        dir: None,
        open_loc_id: Some(1570),
        skill_req: vec![],
        item_req: vec![],
        consumed_req: vec![],
        item_returns: vec![],
        quest_req: vec![],
        varp_req: vec![],
        worn_req: vec![],
        members_req: false,
        wildy_cap: None,
        quest_gates: None,
    }
}

/// A cart-style Npc edge: the driver NPC (type `loc_id`) at (3201,
/// 3201) carries the player to (3300, 3200).
fn cart_edge() -> TransportEdge {
    TransportEdge {
        takeoff: None,
        worn_all_req: Vec::new(),
        kind: TransportKind::Npc,
        player_delta: None,
        at: WorldTile {
            x: 3201,
            z: 3201,
            level: 0,
        },
        to: WorldTile {
            x: 3300,
            z: 3200,
            level: 0,
        },
        loc_id: 7,
        option: 1,
        ticks: 1,
        dir: None,
        open_loc_id: None,
        skill_req: vec![],
        item_req: vec![],
        consumed_req: vec![],
        item_returns: vec![],
        quest_req: vec![],
        varp_req: vec![],
        worn_req: vec![],
        members_req: false,
        wildy_cap: None,
        quest_gates: None,
    }
}

/// Drive `follow` to a terminal outcome, running `on_tick` (the host's
/// per-tick cadence) and rebuilding the snapshot between polls.
fn drive<D: Driver>(
    t: &mut Traveller,
    d: &mut D,
    c: &mut Client,
    snap: &mut GameSnapshot,
    route: &Route,
    options: &mut TravelOptions<'_>,
    mut on_tick: impl FnMut(&mut Client),
) -> TravelOutcome {
    loop {
        if let Some(outcome) = t.follow(d, snap, route.clone(), options) {
            return outcome;
        }
        on_tick(c);
        bump_rebuild(c, snap);
    }
}

#[test]
fn follow_defers_known_thieving_stun_on_distinct_ticks_then_walks() {
    let mut c = scene_client();
    let mut snap = snap_at(&mut c, 0, 0);
    c.local_player.as_mut().unwrap().spotanim_id = 245;
    c.local_player.as_mut().unwrap().spotanim_last_cycle = 100;
    bump_rebuild(&mut c, &mut snap);
    let route = Route {
        legs: vec![walk_leg(&[(3200, 3200), (3200, 3201)])],
        dest: WorldTile {
            x: 3200,
            z: 3201,
            level: 0,
        },
        ticks: 0.5,
    };
    let mut rec = FollowRec {
        route: Some((0, 0)),
        ..Default::default()
    };
    let mut t = Traveller::new();
    let mut options = TravelOptions::default();
    for _ in 0..30 {
        assert!(t
            .follow(&mut rec, &snap, route.clone(), &mut options)
            .is_none());
    }
    assert!(
        rec.walked.is_empty(),
        "duplicate snapshots must not consume the stun wait"
    );
    // The visual may disappear while the server movement lock remains.
    c.local_player.as_mut().unwrap().spotanim_id = -1;
    for _ in 0..10 {
        bump_rebuild(&mut c, &mut snap);
        assert!(t
            .follow(&mut rec, &snap, route.clone(), &mut options)
            .is_none());
        assert!(rec.walked.is_empty());
    }
    bump_rebuild(&mut c, &mut snap);
    assert!(t
        .follow(&mut rec, &snap, route.clone(), &mut options)
        .is_none());
    assert_eq!(rec.walked.len(), 1);
    plant_player(&mut c, 0, 1);
    bump_rebuild(&mut c, &mut snap);
    assert!(matches!(
        t.follow(&mut rec, &snap, route, &mut options),
        Some(TravelOutcome::Arrived { .. })
    ));
}

#[test]
fn stun_recovery_resends_pending_walk_once_and_still_expires() {
    let mut c = scene_client();
    let mut snap = snap_at(&mut c, 0, 0);
    let route = Route {
        legs: vec![walk_leg(&[(3200, 3200), (3200, 3201)])],
        dest: WorldTile {
            x: 3200,
            z: 3201,
            level: 0,
        },
        ticks: 0.5,
    };
    let mut rec = FollowRec {
        route: Some((0, 0)),
        ..Default::default()
    };
    let mut t = Traveller::new();
    let mut options = TravelOptions {
        budget_ticks_per_hop: 3,
        ..Default::default()
    };
    assert!(t
        .follow(&mut rec, &snap, route.clone(), &mut options)
        .is_none());
    assert_eq!(rec.walked.len(), 1);
    c.local_player.as_mut().unwrap().spotanim_id = 245;
    c.local_player.as_mut().unwrap().spotanim_last_cycle = 100;
    bump_rebuild(&mut c, &mut snap);
    for _ in 0..11 {
        assert!(t
            .follow(&mut rec, &snap, route.clone(), &mut options)
            .is_none());
        assert_eq!(rec.walked.len(), 1);
        bump_rebuild(&mut c, &mut snap);
    }
    assert!(t
        .follow(&mut rec, &snap, route.clone(), &mut options)
        .is_none());
    assert_eq!(rec.walked, vec![(0, 1), (0, 1)]);
    // A second animation must not keep extending this route indefinitely.
    c.local_player.as_mut().unwrap().spotanim_last_cycle = 200;
    bump_rebuild(&mut c, &mut snap);
    let mut outcome = None;
    for _ in 0..5 {
        outcome = t.follow(&mut rec, &snap, route.clone(), &mut options);
        if outcome.is_some() {
            break;
        }
        bump_rebuild(&mut c, &mut snap);
    }
    assert!(matches!(outcome, Some(TravelOutcome::Stalled { .. })));
    assert_eq!(rec.walked.len(), 2);
}

#[test]
fn stun_seen_during_guardian_hold_recovers_without_replaying_arrived_walk() {
    for arrived in [false, true] {
        let mut c = scene_client();
        let mut snap = snap_at(&mut c, 0, 0);
        let route = Route {
            legs: vec![walk_leg(&[(3200, 3200), (3200, 3201)])],
            dest: WorldTile {
                x: 3200,
                z: 3201,
                level: 0,
            },
            ticks: 0.5,
        };
        let mut rec = FollowRec {
            route: Some((0, 0)),
            ..Default::default()
        };
        let mut t = Traveller::new();
        let mut options = TravelOptions::default();
        assert!(t
            .follow(&mut rec, &snap, route.clone(), &mut options)
            .is_none());
        c.local_player.as_mut().unwrap().spotanim_id = 245;
        c.local_player.as_mut().unwrap().spotanim_last_cycle = 100;
        bump_rebuild(&mut c, &mut snap);
        c.local_player.as_mut().unwrap().spotanim_id = -1;
        // The host keeps rebuilding snapshots while follow is held/paused.
        for _ in 0..15 {
            bump_rebuild(&mut c, &mut snap);
        }
        if arrived {
            plant_player(&mut c, 0, 1);
            bump_rebuild(&mut c, &mut snap);
        }
        let outcome = t.follow(&mut rec, &snap, route, &mut options);
        if arrived {
            assert!(matches!(outcome, Some(TravelOutcome::Arrived { .. })));
            assert_eq!(rec.walked.len(), 1);
        } else {
            assert!(outcome.is_none());
            assert_eq!(rec.walked.len(), 2);
        }
    }
}

#[test]
fn stun_observation_retains_onset_and_refreshes_only_for_new_stun_packet() {
    let mut c = scene_client();
    let mut snap = snap_at(&mut c, 0, 0);
    c.local_player.as_mut().unwrap().spotanim_id = 244;
    bump_rebuild(&mut c, &mut snap);
    assert_eq!(snap.thieving_stun_tick(), None);
    c.local_player.as_mut().unwrap().spotanim_id = 245;
    c.local_player.as_mut().unwrap().spotanim_last_cycle = 100;
    bump_rebuild(&mut c, &mut snap);
    let first = snap.thieving_stun_tick();
    assert_eq!(first, Some(snap.tick()));
    bump_rebuild(&mut c, &mut snap);
    assert_eq!(snap.thieving_stun_tick(), first);
    c.local_player.as_mut().unwrap().spotanim_id = -1;
    bump_rebuild(&mut c, &mut snap);
    assert_eq!(snap.thieving_stun_tick(), first);
    c.local_player.as_mut().unwrap().spotanim_id = 245;
    c.local_player.as_mut().unwrap().spotanim_last_cycle = 200;
    bump_rebuild(&mut c, &mut snap);
    assert_eq!(snap.thieving_stun_tick(), Some(snap.tick()));
    c.ingame = false;
    bump_rebuild(&mut c, &mut snap);
    assert_eq!(snap.thieving_stun_tick(), None);
}

#[test]
fn follow_walks_a_single_leg_to_arrival() {
    let mut c = scene_client();
    let mut snap = snap_at(&mut c, 0, 0);
    let mut rec = FollowRec {
        route: Some((0, 0)),
        ..FollowRec::default()
    };
    let mut t = Traveller::new();
    let route = Route {
        legs: vec![walk_leg(&[(3200, 3200), (3200, 3201)])],
        dest: WorldTile {
            x: 3200,
            z: 3201,
            level: 0,
        },
        ticks: 0.5, // 1 run step
    };
    let mut options = TravelOptions::default();
    // One tile to the leg end: the run sends one walk and the arrived
    // arm completes the hop once the player steps onto the tile.
    let outcome = drive(
        &mut t,
        &mut rec,
        &mut c,
        &mut snap,
        &route,
        &mut options,
        |c| {
            plant_player(c, 0, 1);
        },
    );
    assert!(matches!(
        outcome,
        TravelOutcome::Arrived { at } if at == WorldTile { x: 3200, z: 3201, level: 0 }
    ));
    assert_eq!(rec.walked.len(), 1, "one walk send");
    assert_eq!(rec.walked, vec![(0, 1)]);
}

#[test]
fn follow_does_not_arrive_short_of_the_last_walk_tile() {
    // The leg's last tile is the destination: the settle must require
    // the player to stand on it exactly. A loose `close_enough` would
    // report Arrived one tile short, and the scenario runner would
    // re-arm the original route (through a door, that walks back).
    let mut c = scene_client();
    let mut snap = snap_at(&mut c, 0, 0);
    let mut rec = FollowRec {
        route: Some((0, 0)),
        ..FollowRec::default()
    };
    let mut t = Traveller::new();
    let route = Route {
        legs: vec![walk_leg(&[(3200, 3200), (3200, 3201), (3200, 3202)])],
        dest: WorldTile {
            x: 3200,
            z: 3202,
            level: 0,
        },
        ticks: 1.0,
    };
    let mut options = TravelOptions::default(); // close_enough = 2

    assert!(t
        .follow(&mut rec, &snap, route.clone(), &mut options)
        .is_none());
    assert_eq!(rec.walked, vec![(0, 2)], "walk aims at the last tile");

    plant_player(&mut c, 0, 1);
    bump_rebuild(&mut c, &mut snap);
    assert!(
        t.follow(&mut rec, &snap, route.clone(), &mut options)
            .is_none(),
        "a walk leg must not finish one tile short of its last tile"
    );

    plant_player(&mut c, 0, 2);
    bump_rebuild(&mut c, &mut snap);
    match t.follow(&mut rec, &snap, route.clone(), &mut options) {
        Some(TravelOutcome::Arrived { at }) => {
            assert_eq!(
                at,
                WorldTile {
                    x: 3200,
                    z: 3202,
                    level: 0
                }
            );
        }
        other => panic!("expected Arrived on the last tile, got {other:?}"),
    }
}

#[test]
fn current_aim_is_none_idle_and_the_walk_hop_aim_in_follow() {
    let mut t = Traveller::new();
    assert_eq!(t.current_aim(), None, "idle traveller has no aim");
    let mut c = scene_client();
    let snap = snap_at(&mut c, 0, 0);
    let mut rec = FollowRec {
        route: Some((0, 0)),
        ..FollowRec::default()
    };
    let route = Route {
        legs: vec![walk_leg(&[(3200, 3200), (3200, 3201), (3200, 3202)])],
        dest: WorldTile {
            x: 3200,
            z: 3202,
            level: 0,
        },
        ticks: 1.0,
    };
    let mut options = TravelOptions::default();
    assert!(
        t.follow(&mut rec, &snap, route.clone(), &mut options)
            .is_none(),
        "the run must be active after the first poll"
    );
    // A short leg aims at its last tile (the same `pick_aim` the walk
    // send used).
    assert_eq!(
        t.current_aim(),
        Some(WorldTile {
            x: 3200,
            z: 3202,
            level: 0
        }),
        "the walk hop's aim is the leg's last tile"
    );
}

#[test]
fn follow_skips_single_tile_walk_legs() {
    let mut c = scene_client();
    let mut snap = snap_at(&mut c, 0, 0);
    let mut rec = FollowRec {
        route: Some((0, 0)),
        ..FollowRec::default()
    };
    let mut t = Traveller::new();
    let route = Route {
        legs: vec![walk_leg(&[(3200, 3200)])],
        dest: WorldTile {
            x: 3200,
            z: 3200,
            level: 0,
        },
        ticks: 0.0, // no steps
    };
    let mut options = TravelOptions::default();
    // A single-tile leg (the find(from == to) shape) is a no-op: no
    // walk is sent, the run arrives immediately.
    let outcome = drive(
        &mut t,
        &mut rec,
        &mut c,
        &mut snap,
        &route,
        &mut options,
        |_| {},
    );
    assert!(matches!(outcome, TravelOutcome::Arrived { .. }));
    assert_eq!(rec.walked.len(), 0, "no walk for a single-tile leg");
}

#[test]
fn follow_transport_leg_interacts_and_arrives() {
    let mut c = scene_client();
    plant_ladder(&mut c, Some("Climb"));
    let mut snap = snap_at(&mut c, 0, 0);
    let mut rec = FollowRec {
        route: Some((0, 0)),
        ..FollowRec::default()
    };
    let mut t = Traveller::new();
    let route = Route {
        legs: vec![Leg::Transport {
            edge: Box::new(ladder_edge()),
        }],
        dest: WorldTile {
            x: 3202,
            z: 3205,
            level: 0,
        },
        ticks: 2.0, // the ladder edge's ticks
    };
    let mut options = TravelOptions::default();
    let outcome = drive(
        &mut t,
        &mut rec,
        &mut c,
        &mut snap,
        &route,
        &mut options,
        |c| {
            plant_player(c, 2, 5);
        },
    );
    assert!(matches!(
        outcome,
        TravelOutcome::Arrived { at } if at == WorldTile { x: 3202, z: 3205, level: 0 }
    ));
    assert_eq!(rec.loc_ops, 1, "one OP_LOC1 interact sent");
}

#[test]
fn follow_blocks_transport_when_worn_all_item_is_only_carried() {
    let mut client = scene_client();
    plant_ladder(&mut client, Some("Climb"));
    plant_inv_item(&mut client, 946);
    let snapshot = snap_at(&mut client, 2, 3);
    assert!(snapshot
        .inv()
        .iter()
        .any(|&(id, count)| id == 946 && count > 0));
    assert!(snapshot.equipment().is_empty());

    let mut edge = ladder_edge();
    edge.worn_all_req = vec![946];
    let route = Route {
        dest: edge.to,
        legs: vec![Leg::Transport {
            edge: Box::new(edge),
        }],
        ticks: 2.0,
    };
    let mut rec = FollowRec {
        route: Some((2, 3)),
        ..FollowRec::default()
    };
    let mut traveller = Traveller::new();
    let outcome = traveller.follow(&mut rec, &snapshot, route, &mut TravelOptions::default());
    assert!(matches!(
        outcome,
        Some(TravelOutcome::Blocked { detail, .. })
            if detail.contains("worn_all_req item 946")
    ));
    assert_eq!(rec.loc_ops, 0, "a carried item does not authorize the hop");
}

#[test]
fn gated_transport_rechecks_evidence_after_approach_before_send() {
    use crate::quest_gates::tests::{family, range, tbwt_evidence, window};
    use crate::quest_gates::QuestGates;
    use api::selected::Truth;

    let proven = tbwt_evidence(range(Some(3), Some(3)));
    let blocked = tbwt_evidence(range(Some(4), Some(4)));
    let unknown = tbwt_evidence(range(Some(3), Some(4)));
    let gate = window("tbwt", "tbwt_main", Some(3), Some(3));
    // `None`: the crossing is allowed. Otherwise the typed terminal's verdict
    // and whether it names the gate as still undecided.
    for (current, refused) in [
        (Some(&blocked), Some((Truth::False, false))),
        (Some(&unknown), Some((Truth::Unknown, true))),
        (None, Some((Truth::Unknown, true))),
        (Some(&proven), None),
    ] {
        let mut c = scene_client();
        plant_ladder(&mut c, Some("Climb"));
        let mut snap = snap_at(&mut c, 2, 1);
        let mut rec = FollowRec {
            route: Some((0, 0)),
            ..FollowRec::default()
        };
        let mut edge = ladder_edge();
        edge.quest_gates = Some(QuestGates::new(family(1), [gate.clone()]).unwrap());
        assert_eq!(
            edge.quest_gates.as_ref().unwrap().test(Some(&proven)),
            Truth::True
        );
        let route = Route {
            dest: edge.to,
            legs: vec![Leg::Transport {
                edge: Box::new(edge),
            }],
            ticks: 2.0,
        };
        let mut traveller = Traveller::new();
        let mut options = TravelOptions {
            quest_evidence: Some(&proven),
            ..Default::default()
        };
        assert!(traveller
            .follow(&mut rec, &snap, route.clone(), &mut options)
            .is_none());
        assert_eq!(rec.walked, vec![(2, 3)]);
        assert_eq!(rec.loc_ops, 0);

        plant_player(&mut c, 2, 3);
        bump_rebuild(&mut c, &mut snap);
        options.quest_evidence = current;
        let outcome = traveller.follow(&mut rec, &snap, route.clone(), &mut options);
        match refused {
            None => {
                assert!(outcome.is_none());
                assert_eq!(rec.loc_ops, 1);
                // Sent: evidence changing now does not abort a crossing that
                // is already under way; its settle poll still arrives.
                options.quest_evidence = Some(&blocked);
                plant_player(&mut c, 2, 5);
                bump_rebuild(&mut c, &mut snap);
                assert!(matches!(
                    traveller.follow(&mut rec, &snap, route, &mut options),
                    Some(TravelOutcome::Arrived { .. })
                ));
                assert_eq!(rec.loc_ops, 1, "a proven gate sends the interact once");
            }
            Some((verdict, undecided)) => {
                assert_eq!(
                    rec.loc_ops, 0,
                    "gate closure must prevent the pending interact"
                );
                let expected: std::sync::Arc<[_]> = if undecided {
                    [gate.clone()].into()
                } else {
                    Vec::new().into()
                };
                assert_eq!(
                    outcome,
                    Some(TravelOutcome::EvidenceUnproven {
                        // Scene (2, 3) on the fixture's 3200 build base.
                        at: WorldTile {
                            x: 3202,
                            z: 3203,
                            level: 0
                        },
                        leg: 0,
                        verdict,
                        unresolved: expected,
                    })
                );
            }
        }
    }
}

/// Closed trapdoor 1568 Open then loc_change to 1570: Climb-down must
/// target the open leaf. Landing is the player's tile +6400 (offset 1
/// from the loc-baked dest). Host WalkNear close_enough 0 still arrives.
#[test]
fn follow_trapdoor_open_then_climb_arrives_cellar_offset() {
    let mut c = scene_client();
    plant_loc(&mut c, 1568, "Trapdoor", "Open", 2, 4);
    let mut snap = snap_at(&mut c, 2, 3);
    let mut rec = FollowRec {
        route: Some((2, 3)),
        ..FollowRec::default()
    };
    let mut t = Traveller::new();
    let edge = trapdoor_edge();
    let dest = edge.to;
    let landing = WorldTile {
        x: 3202,
        z: 3203 + CELLAR_SHIFT,
        level: 0,
    };
    let route = Route {
        legs: vec![Leg::Transport {
            edge: Box::new(edge),
        }],
        dest,
        ticks: 3.0,
    };
    let attempts = std::cell::RefCell::new(Vec::new());
    let mut options = TravelOptions {
        close_enough: 0,
        on_event: Some(Box::new(|e| {
            if let TravelEvent::TransportAttempt { actual_id, .. } = e {
                attempts.borrow_mut().push(actual_id);
            }
        })),
        ..TravelOptions::default()
    };
    assert!(
        t.follow(&mut rec, &snap, route.clone(), &mut options)
            .is_none(),
        "Open on closed 1568"
    );
    assert_eq!(rec.loc_ops, 1, "Open");
    assert_eq!(*attempts.borrow(), vec![1568]);
    plant_loc(&mut c, 1570, "Trapdoor", "Climb-down", 2, 4);
    bump_rebuild(&mut c, &mut snap);
    assert!(
        t.follow(&mut rec, &snap, route.clone(), &mut options)
            .is_none(),
        "Climb-down on open 1570"
    );
    assert_eq!(rec.loc_ops, 2, "Open then Climb-down");
    assert_eq!(*attempts.borrow(), vec![1568, 1570]);
    plant_player(&mut c, 2, 3 + CELLAR_SHIFT);
    bump_rebuild(&mut c, &mut snap);
    match t.follow(&mut rec, &snap, route.clone(), &mut options) {
        Some(TravelOutcome::Arrived { at }) => {
            assert_eq!(at, landing, "player-tile cellar landing, not loc dest");
            assert_ne!(at, dest);
        }
        other => panic!("expected Arrived at cellar offset, got {other:?}"),
    }
    assert_eq!(rec.loc_ops, 2, "no extra interact after climb");
}

/// Already-open leaf 1570 only: first interact is Climb-down, same
/// cellar offset-1 arrive under close_enough 0.
#[test]
fn follow_trapdoor_already_open_climb_arrives_cellar_offset() {
    let mut c = scene_client();
    plant_loc(&mut c, 1570, "Trapdoor", "Climb-down", 2, 4);
    let mut snap = snap_at(&mut c, 2, 3);
    let mut rec = FollowRec {
        route: Some((2, 3)),
        ..FollowRec::default()
    };
    let mut t = Traveller::new();
    let edge = trapdoor_edge();
    let dest = edge.to;
    let landing = WorldTile {
        x: 3202,
        z: 3203 + CELLAR_SHIFT,
        level: 0,
    };
    let route = Route {
        legs: vec![Leg::Transport {
            edge: Box::new(edge),
        }],
        dest,
        ticks: 3.0,
    };
    let attempts = std::cell::RefCell::new(Vec::new());
    let mut options = TravelOptions {
        close_enough: 0,
        on_event: Some(Box::new(|e| {
            if let TravelEvent::TransportAttempt { actual_id, .. } = e {
                attempts.borrow_mut().push(actual_id);
            }
        })),
        ..TravelOptions::default()
    };
    assert!(
        t.follow(&mut rec, &snap, route.clone(), &mut options)
            .is_none(),
        "Climb-down on already-open 1570"
    );
    assert_eq!(rec.loc_ops, 1, "one Climb-down, not Open");
    assert_eq!(*attempts.borrow(), vec![1570]);
    plant_player(&mut c, 2, 3 + CELLAR_SHIFT);
    bump_rebuild(&mut c, &mut snap);
    match t.follow(&mut rec, &snap, route.clone(), &mut options) {
        Some(TravelOutcome::Arrived { at }) => {
            assert_eq!(at, landing, "player-tile cellar landing, not loc dest");
            assert_ne!(at, dest);
        }
        other => panic!("expected Arrived at cellar offset, got {other:?}"),
    }
    assert_eq!(rec.loc_ops, 1, "already-open must not re-click");
}

#[test]
fn follow_vertical_ladder_lands_on_the_adjacent_player_tile() {
    for (from_level, to_level, id, op, width, length, landing_x, landing_z) in [
        (0, 1, 1748, "Climb-up", 1, 1, 3202, 3203),
        (2, 1, 1746, "Climb-down", 1, 1, 3202, 3203),
        (0, 1, 1748, "Climb-up", 2, 2, 3204, 3206),
    ] {
        let mut c = scene_client();
        plant_loc(&mut c, id, "Ladder", op, 2, 4);
        let mut snap = snap_at(&mut c, 2, 3);
        let mut loc = snap.locs()[0].clone();
        loc.tile.level = from_level;
        loc.footprint_width = width;
        loc.footprint_length = length;
        c.minusedlevel = from_level;
        bump_rebuild(&mut c, &mut snap);
        snap.seed_locs(vec![loc]);
        let mut edge = trapdoor_edge();
        edge.at.level = from_level;
        edge.to = WorldTile {
            x: edge.at.x,
            z: edge.at.z,
            level: to_level,
        };
        edge.loc_id = id;
        edge.open_loc_id = None;
        let landing = WorldTile {
            x: landing_x,
            z: landing_z,
            level: to_level,
        };
        let route = Route {
            legs: vec![Leg::Transport {
                edge: Box::new(edge),
            }],
            dest: landing,
            ticks: 3.0,
        };
        let mut rec = FollowRec {
            route: Some((2, 3)),
            ..FollowRec::default()
        };
        let mut t = Traveller::new();
        let mut options = TravelOptions {
            close_enough: 0,
            ..TravelOptions::default()
        };
        assert!(t
            .follow(&mut rec, &snap, route.clone(), &mut options)
            .is_none());
        assert_eq!(rec.loc_ops, 1);
        // The old plane must not settle the hop, even at the same x/z.
        bump_rebuild(&mut c, &mut snap);
        assert!(t
            .follow(&mut rec, &snap, route.clone(), &mut options)
            .is_none());
        c.minusedlevel = to_level;
        // A distant tile on the expected plane is not this ladder's arrival.
        plant_player(&mut c, 2, 1);
        bump_rebuild(&mut c, &mut snap);
        assert!(t
            .follow(&mut rec, &snap, route.clone(), &mut options)
            .is_none());
        plant_player(&mut c, landing_x - 3200, landing_z - 3200);
        bump_rebuild(&mut c, &mut snap);
        assert_eq!(
            t.follow(&mut rec, &snap, route, &mut options),
            Some(TravelOutcome::Arrived { at: landing })
        );
        assert_eq!(rec.loc_ops, 1, "the completed climb must not be sent twice");
    }
}

#[test]
fn follow_cellar_offset_completes_finished_walk_then_uses_keyed_door() {
    let mut c = scene_client();
    c.map_build_base_x = 3114;
    c.map_build_base_z = 9848;
    plant_loc(&mut c, 1755, "Ladder", "Climb-up", 2, 4);
    plant_inv_item(&mut c, 983);
    let mut snap = snap_at(&mut c, 2, 3);
    let mut rec = FollowRec {
        route: Some((2, 3)),
        build_base: Some((3114, 9848)),
        ..FollowRec::default()
    };
    let mut t = Traveller::new();
    let packed_landing = WorldTile {
        x: 3116,
        z: 3452,
        level: 0,
    };
    let offset_landing = WorldTile {
        x: 3116,
        z: 3451,
        level: 0,
    };
    let mut ladder = trapdoor_edge();
    ladder.at = WorldTile {
        x: 3116,
        z: 9852,
        level: 0,
    };
    ladder.to = packed_landing;
    ladder.loc_id = 1755;
    ladder.open_loc_id = None;
    let door_to = WorldTile {
        x: 3115,
        z: 3449,
        level: 0,
    };
    let mut door = door_edge();
    door.at = WorldTile {
        x: 3115,
        z: 3450,
        level: 0,
    };
    door.to = door_to;
    door.loc_id = 1804;
    door.option = 0;
    door.dir = Some(DoorDir::S);
    door.open_loc_id = Some(1535);
    door.item_req = vec![(983, 1)];
    let route = Route {
        legs: vec![
            Leg::Transport {
                edge: Box::new(ladder),
            },
            Leg::Walk {
                tiles: vec![packed_landing, offset_landing],
            },
            Leg::Transport {
                edge: Box::new(door),
            },
        ],
        dest: door_to,
        ticks: 4.5,
    };
    let mut phases = Vec::new();
    let mut options = TravelOptions {
        close_enough: 0,
        on_leg: Some(Box::new(|leg: &Leg, phase: LegPhase| {
            phases.push((leg.clone(), phase));
        })),
        ..TravelOptions::default()
    };

    let first = t.follow(&mut rec, &snap, route.clone(), &mut options);
    assert_eq!(
        first,
        None,
        "Climb-up on cellar ladder; scene={:?} driver_base={:?} loc_ops={}",
        (snap.scene().base_x, snap.scene().base_z),
        rec.build_base,
        rec.loc_ops,
    );
    assert_eq!(rec.loc_ops, 1, "one ladder interact");

    c.map_build_base_z = 3448;
    rec.build_base = Some((3114, 3448));
    plant_loc(&mut c, 1804, "Door", "Open", 1, 2);
    plant_player(&mut c, 2, 3);
    bump_rebuild(&mut c, &mut snap);
    assert!(
        t.follow(&mut rec, &snap, route.clone(), &mut options)
            .is_none(),
        "the offset landing must continue into the keyed door hop"
    );
    let run = t.follow.as_ref().expect("follow still settling the door");
    assert_eq!(run.leg_index, 2, "ladder and completed walk are done");
    assert_eq!(run.hops, 0, "the completed walk consumes no hop");
    assert_eq!(rec.walked, Vec::<(i32, i32)>::new(), "no bogus walk send");
    assert_eq!(rec.loc_uses, 1, "use brass key 983 on door 1804");

    plant_player(&mut c, 1, 1);
    bump_rebuild(&mut c, &mut snap);
    let outcome = t.follow(&mut rec, &snap, route, &mut options);
    assert_eq!(outcome, Some(TravelOutcome::Arrived { at: door_to }));
    drop(options);
    assert_eq!(phases.len(), 6, "start/done for all three legs");
    assert!(matches!(
        &phases[0],
        (Leg::Transport { .. }, LegPhase::Start)
    ));
    assert!(matches!(
        &phases[1],
        (Leg::Transport { .. }, LegPhase::Done)
    ));
    assert!(matches!(&phases[2], (Leg::Walk { .. }, LegPhase::Start)));
    assert!(matches!(&phases[3], (Leg::Walk { .. }, LegPhase::Done)));
    assert!(matches!(
        &phases[4],
        (Leg::Transport { .. }, LegPhase::Start)
    ));
    assert!(matches!(
        &phases[5],
        (Leg::Transport { .. }, LegPhase::Done)
    ));
}

#[test]
fn follow_does_not_complete_unsent_walk_at_endpoint_on_other_level() {
    let mut c = scene_client();
    let snap = snap_at(&mut c, 0, 0);
    let mut rec = FollowRec {
        route: Some((0, 0)),
        ..FollowRec::default()
    };
    let mut t = Traveller::new();
    let here = WorldTile {
        x: 3200,
        z: 3200,
        level: 0,
    };
    let off_level = WorldTile { level: 1, ..here };
    let route = Route {
        legs: vec![Leg::Walk {
            tiles: vec![here, off_level],
        }],
        dest: off_level,
        ticks: 0.5,
    };
    let mut options = TravelOptions::default();

    assert_eq!(
        t.follow(&mut rec, &snap, route, &mut options),
        Some(TravelOutcome::Refused {
            at: here,
            reason: SendReason::LevelMismatch,
        }),
        "matching endpoint coordinates on another level are not complete"
    );
    assert!(rec.walked.is_empty(), "no off-level walk is sent");
}

/// Agility forcemove holds the player after they land on `to`. Completing
/// the hop on the first arrived poll sends the next walk into a locked
/// player (live Yanille ledge: Dropped at 2580,9512 aiming 2580,9501).
/// Packed `edge.ticks` is the anim delay; stay on the hop until it elapses.
#[test]
fn follow_agility_shortcut_waits_packed_ticks_after_landing() {
    let mut c = scene_client();
    plant_ladder(&mut c, Some("Climb"));
    let mut snap = snap_at(&mut c, 1, 4);
    let mut rec = FollowRec {
        route: Some((1, 4)),
        ..FollowRec::default()
    };
    let mut t = Traveller::new();
    let mut edge = ladder_edge();
    edge.kind = TransportKind::AgilityShortcut;
    edge.ticks = 4;
    let dest = edge.to;
    let route = Route {
        legs: vec![Leg::Transport {
            edge: Box::new(edge),
        }],
        dest,
        ticks: 4.0,
    };
    let mut options = TravelOptions::default();
    assert!(
        t.follow(&mut rec, &snap, route.clone(), &mut options)
            .is_none(),
        "approach/interact"
    );
    plant_player(&mut c, 2, 5);
    bump_rebuild(&mut c, &mut snap);
    for i in 0..3 {
        assert!(
            t.follow(&mut rec, &snap, route.clone(), &mut options)
                .is_none(),
            "still on the anim delay after landing poll {i}"
        );
    }
    match t.follow(&mut rec, &snap, route.clone(), &mut options) {
        Some(TravelOutcome::Arrived { at }) => assert_eq!(at, dest),
        other => panic!("expected Arrived after packed ticks, got {other:?}"),
    }
}

/// Unequippable knife: the packed web hop is `option` 0 + `item_req`
/// knife, so follow must `oplocu` (USEHELD_ONLOC), never Slash.
#[test]
fn follow_web_knife_edge_uses_the_held_knife_on_the_loc() {
    let mut c = scene_client();
    plant_loc(&mut c, 733, "Web", "Slash", 1, 0);
    plant_inv_item(&mut c, 946);
    let mut snap = snap_at(&mut c, 0, 0);
    let mut rec = FollowRec {
        route: Some((0, 0)),
        ..FollowRec::default()
    };
    let mut t = Traveller::new();
    let mut edge = door_edge();
    edge.loc_id = 733;
    edge.option = 0;
    edge.item_req = vec![(946, 1)];
    edge.open_loc_id = Some(734);
    let dest = edge.to;
    let route = Route {
        legs: vec![Leg::Transport {
            edge: Box::new(edge),
        }],
        dest,
        ticks: 2.0,
    };
    let mut options = TravelOptions::default();
    let outcome = drive(
        &mut t,
        &mut rec,
        &mut c,
        &mut snap,
        &route,
        &mut options,
        |c| {
            plant_player(c, 3, 0);
        },
    );
    assert!(
        matches!(outcome, TravelOutcome::Arrived { at } if at == dest),
        "got {outcome:?}"
    );
    assert_eq!(rec.loc_ops, 0, "Slash oploc1 is the worn-blade hop");
    assert_eq!(rec.loc_uses, 1, "oplocu the knife on the web");
}

#[test]
fn follow_web_reselects_knife_after_planned_slash_blade_is_unequipped() {
    let mut c = scene_client();
    plant_loc(&mut c, 733, "Web", "Slash", 2, 0);
    plant_inv_item(&mut c, 946);
    plant_equipment_item(&mut c, 1321);
    let mut snap = snap_at(&mut c, 0, 0);
    assert_eq!(snap.equipment()[0].def.id, 1321);
    assert!(snap.inv().iter().any(|&(id, count)| id == 946 && count > 0));

    let edges = web_cut_edges();
    let route = web_route(edges[1].clone());
    let mut rec = FollowRec {
        route: Some((0, 0)),
        ..FollowRec::default()
    };
    let mut traveller = Traveller::new();
    let mut options = TravelOptions {
        close_enough: 0,
        edges: Some(&edges),
        ..TravelOptions::default()
    };

    assert!(traveller
        .follow(&mut rec, &snap, route.clone(), &mut options)
        .is_none());
    assert_eq!(rec.walked.len(), 1, "one approach walk before the cut");
    let stand = rec.walked[0];

    clear_equipment_item(&mut c);
    plant_player(&mut c, stand.0, stand.1);
    bump_rebuild(&mut c, &mut snap);
    assert!(snap.equipment().is_empty());
    assert!(traveller
        .follow(&mut rec, &snap, route, &mut options)
        .is_none());
    assert_eq!(rec.loc_uses, 1, "the current knife replaces planned Slash");
    assert_eq!(rec.loc_ops, 0, "do not send the stale Slash op");
}

#[test]
fn follow_web_reselects_slash_after_planned_knife_is_removed() {
    let mut c = scene_client();
    plant_loc(&mut c, 733, "Web", "Slash", 2, 0);
    plant_inv_item(&mut c, 946);
    let mut snap = snap_at(&mut c, 0, 0);
    let edges = web_cut_edges();
    let route = web_route(edges[0].clone());
    let mut rec = FollowRec {
        route: Some((0, 0)),
        ..FollowRec::default()
    };
    let mut traveller = Traveller::new();
    let mut options = TravelOptions {
        close_enough: 0,
        edges: Some(&edges),
        ..TravelOptions::default()
    };

    assert!(traveller
        .follow(&mut rec, &snap, route.clone(), &mut options)
        .is_none());
    assert_eq!(rec.walked.len(), 1, "one approach walk before the cut");
    let stand = rec.walked[0];

    clear_inventory_item(&mut c);
    plant_equipment_item(&mut c, 1321);
    plant_player(&mut c, stand.0, stand.1);
    bump_rebuild(&mut c, &mut snap);
    assert!(snap.inv().iter().all(|&(id, _)| id != 946));
    assert!(snap.equipment().iter().any(|item| item.def.id == 1321));
    assert!(traveller
        .follow(&mut rec, &snap, route, &mut options)
        .is_none());
    assert_eq!(rec.loc_ops, 1, "the current slash blade replaces oplocu");
    assert_eq!(rec.loc_uses, 0, "do not use the stale knife edge");
}

#[test]
fn follow_web_without_a_qualifying_action_fails_without_sending() {
    let mut c = scene_client();
    plant_loc(&mut c, 733, "Web", "Slash", 2, 0);
    plant_inv_item(&mut c, 946);
    plant_equipment_item(&mut c, 1321);
    let mut snap = snap_at(&mut c, 0, 0);
    let edges = web_cut_edges();
    let route = web_route(edges[1].clone());
    let mut rec = FollowRec {
        route: Some((0, 0)),
        ..FollowRec::default()
    };
    let mut traveller = Traveller::new();
    let mut options = TravelOptions {
        close_enough: 0,
        edges: Some(&edges),
        ..TravelOptions::default()
    };

    assert!(traveller
        .follow(&mut rec, &snap, route.clone(), &mut options)
        .is_none());
    clear_inventory_item(&mut c);
    clear_equipment_item(&mut c);
    let stand = rec.walked[0];
    plant_player(&mut c, stand.0, stand.1);
    bump_rebuild(&mut c, &mut snap);

    match traveller.follow(&mut rec, &snap, route, &mut options) {
        Some(TravelOutcome::Blocked { detail, .. }) => {
            assert!(
                detail.contains("neither a worn slash blade nor a carried knife qualifies"),
                "explicit failure detail: {detail}"
            );
        }
        other => panic!("expected an explicit blocked outcome, got {other:?}"),
    }
    assert_eq!(rec.loc_ops, 0, "no doomed Slash is sent");
    assert_eq!(rec.loc_uses, 0, "no missing knife is used");
}

#[test]
fn follow_web_slash_during_approach_walks_through_without_an_op() {
    let mut c = scene_client();
    plant_loc(&mut c, 733, "Web", "Slash", 2, 0);
    plant_inv_item(&mut c, 946);
    let mut snap = snap_at(&mut c, 0, 0);
    let edges = web_cut_edges();
    let route = web_route(edges[0].clone());
    let dest = route.dest;
    let mut rec = FollowRec {
        route: Some((0, 0)),
        ..FollowRec::default()
    };
    let mut traveller = Traveller::new();
    let mut options = TravelOptions {
        close_enough: 0,
        edges: Some(&edges),
        ..TravelOptions::default()
    };

    assert!(traveller
        .follow(&mut rec, &snap, route.clone(), &mut options)
        .is_none());
    let stand = rec.walked[0];
    plant_loc(&mut c, 734, "Web", "Slash", 2, 0);
    c.add_chat(0, "You fail to cut through it.", "");
    plant_player(&mut c, stand.0, stand.1);
    bump_rebuild(&mut c, &mut snap);
    assert!(traveller
        .follow(&mut rec, &snap, route.clone(), &mut options)
        .is_none());
    assert_eq!(rec.loc_ops, 0);
    assert_eq!(rec.loc_uses, 0, "the approach completion sends no op");

    bump_rebuild(&mut c, &mut snap);
    assert!(traveller
        .follow(&mut rec, &snap, route.clone(), &mut options)
        .is_none());
    assert!(rec.walked.contains(&(4, 0)), "walk through the open web");
    assert_eq!(rec.loc_ops, 0);
    assert_eq!(rec.loc_uses, 0);
    plant_player(&mut c, 4, 0);
    bump_rebuild(&mut c, &mut snap);
    assert_eq!(
        traveller.follow(&mut rec, &snap, route, &mut options),
        Some(TravelOutcome::Arrived { at: dest })
    );
}

#[test]
fn follow_web_retries_each_observed_cut_failure_and_accepts_the_slashed_loc() {
    let mut c = scene_client();
    plant_loc(&mut c, 733, "Web", "Slash", 1, 0);
    plant_inv_item(&mut c, 946);
    let mut snap = snap_at(&mut c, 0, 0);
    let mut rec = FollowRec {
        route: Some((0, 0)),
        ..FollowRec::default()
    };
    let mut traveller = Traveller::new();
    let mut edge = door_edge();
    edge.loc_id = 733;
    edge.option = 0;
    edge.item_req = vec![(946, 1)];
    edge.open_loc_id = Some(734);
    edge.dir = Some(DoorDir::E);
    let edges = [edge.clone()];
    let dest = edge.to;
    let route = Route {
        legs: vec![Leg::Transport {
            edge: Box::new(edge),
        }],
        dest,
        ticks: 2.0,
    };
    let mut options = TravelOptions {
        edges: Some(&edges),
        ..TravelOptions::default()
    };

    assert!(traveller
        .follow(&mut rec, &snap, route.clone(), &mut options)
        .is_none());
    assert_eq!(rec.loc_uses, 1, "the initial cut uses the held knife");

    c.add_chat(0, "You fail to cut through it.", "");
    bump_rebuild(&mut c, &mut snap);
    assert!(traveller
        .follow(&mut rec, &snap, route.clone(), &mut options)
        .is_none());
    assert_eq!(
        rec.loc_uses, 2,
        "one observed 50% failure immediately permits one retry"
    );

    bump_rebuild(&mut c, &mut snap);
    assert!(traveller
        .follow(&mut rec, &snap, route.clone(), &mut options)
        .is_none());
    assert_eq!(
        rec.loc_uses, 2,
        "the same failure message must not trigger multiple retries"
    );

    c.add_chat(0, "You fail to cut through it.", "");
    bump_rebuild(&mut c, &mut snap);
    assert!(traveller
        .follow(&mut rec, &snap, route.clone(), &mut options)
        .is_none());
    assert_eq!(rec.loc_uses, 3, "each fresh failure permits one retry");

    plant_loc(&mut c, 734, "Web", "Slash", 1, 0);
    bump_rebuild(&mut c, &mut snap);
    assert!(traveller
        .follow(&mut rec, &snap, route.clone(), &mut options)
        .is_none());
    assert_eq!(
        rec.loc_uses, 3,
        "loc_change to 734 succeeds without reclicking"
    );
    assert!(
        rec.walked.contains(&(3, 0)),
        "the follower walks through the slashed web"
    );

    plant_player(&mut c, 3, 0);
    bump_rebuild(&mut c, &mut snap);
    assert_eq!(
        traveller.follow(&mut rec, &snap, route, &mut options),
        Some(TravelOutcome::Arrived { at: dest })
    );
}

#[test]
fn follow_web_does_not_retry_from_elapsed_time_without_a_failure_message() {
    let mut c = scene_client();
    plant_loc(&mut c, 733, "Web", "Slash", 1, 0);
    plant_inv_item(&mut c, 946);
    let mut snap = snap_at(&mut c, 0, 0);
    let mut rec = FollowRec {
        route: Some((0, 0)),
        ..FollowRec::default()
    };
    let mut traveller = Traveller::new();
    let mut edge = door_edge();
    edge.loc_id = 733;
    edge.option = 0;
    edge.item_req = vec![(946, 1)];
    edge.open_loc_id = Some(734);
    edge.dir = Some(DoorDir::E);
    let edges = [edge.clone()];
    let route = Route {
        legs: vec![Leg::Transport {
            edge: Box::new(edge.clone()),
        }],
        dest: edge.to,
        ticks: 2.0,
    };
    let mut options = TravelOptions {
        budget_ticks_per_hop: 1,
        edges: Some(&edges),
        ..TravelOptions::default()
    };

    assert!(traveller
        .follow(&mut rec, &snap, route.clone(), &mut options)
        .is_none());
    assert_eq!(rec.loc_uses, 1);
    let outcome = loop {
        bump_rebuild(&mut c, &mut snap);
        if let Some(outcome) = traveller.follow(&mut rec, &snap, route.clone(), &mut options) {
            break outcome;
        }
        assert!(snap.tick() < 8, "web did not settle within the hop budget");
    };
    assert!(
        matches!(outcome, TravelOutcome::Stalled { .. }),
        "without an observed fail message, wait for the ordinary hop timeout: {outcome:?}"
    );
    assert_eq!(
        rec.loc_uses, 1,
        "the 50% retry is event-driven, never an automatic timed door troll"
    );
}

#[test]
fn follow_npc_edge_sends_op_npc_and_arrives() {
    // Task 2: a `TransportKind::Npc` edge (cart, essence wizard,
    // Elkoy) must interact with the driver NPC — an `OP_NPC1` on the
    // type-id match within 3 of `at` — never a loc op, and arrive at
    // `edge.to`. The player stands beside the driver on an operable
    // cardinal tile, so the cart interaction is sent immediately.
    let mut c = scene_client();
    plant_driver_npc(&mut c, 7, 1, 1);
    let mut snap = snap_at(&mut c, 1, 2);
    let mut rec = FollowRec {
        route: Some((1, 2)),
        ..FollowRec::default()
    };
    let mut t = Traveller::new();
    let route = Route {
        legs: vec![Leg::Transport {
            edge: Box::new(cart_edge()),
        }],
        dest: WorldTile {
            x: 3300,
            z: 3200,
            level: 0,
        },
        ticks: 1.0, // the cart edge's ticks
    };
    let mut options = TravelOptions::default();
    let outcome = drive(
        &mut t,
        &mut rec,
        &mut c,
        &mut snap,
        &route,
        &mut options,
        |c| {
            plant_player(c, 100, 0);
        },
    );
    assert!(matches!(
        outcome,
        TravelOutcome::Arrived { at } if at == WorldTile { x: 3300, z: 3200, level: 0 }
    ));
    assert_eq!(rec.npc_ops, 1, "one OP_NPC1 interact sent for the Npc edge");
    assert_eq!(rec.loc_ops, 0, "an Npc edge never sends OP_LOC1");
}

#[test]
fn follow_boat_edge_ops_the_seaman_npc_not_a_loc() {
    // Port Sarim → Musa is `TransportKind::Boat` with loc_id = npc 378.
    // Looking it up as a loc is the live "loc 378 not within 3 tiles"
    // fail; the hop must OP_NPC1 the sailor.
    let mut c = scene_client();
    plant_npc_ops(&mut c, 378, 1, 1, "Seaman Thresnor", &["Talk-to"]);
    let mut snap = snap_at(&mut c, 1, 2);
    let mut rec = FollowRec {
        route: Some((1, 2)),
        ..FollowRec::default()
    };
    let mut t = Traveller::new();
    let mut edge = cart_edge();
    edge.kind = TransportKind::Boat;
    edge.loc_id = 378;
    let route = Route {
        legs: vec![Leg::Transport {
            edge: Box::new(edge),
        }],
        dest: WorldTile {
            x: 3300,
            z: 3200,
            level: 0,
        },
        ticks: 9.0,
    };
    let mut options = TravelOptions::default();
    let outcome = drive(
        &mut t,
        &mut rec,
        &mut c,
        &mut snap,
        &route,
        &mut options,
        |c| {
            plant_player(c, 100, 0);
        },
    );
    assert!(matches!(outcome, TravelOutcome::Arrived { .. }));
    assert_eq!(rec.npc_ops, 1, "Talk-to the seaman");
    assert_eq!(rec.loc_ops, 0, "a Boat edge is not a loc");
}

#[test]
fn follow_disembark_plank_ops_the_boat_side_loc() {
    // After set_sail the player is on the Musa deck (2956,3143,1).
    // The Boat edge must not swallow the gangplank: Cross loc 2082
    // at (2956,3144,1) lands on the dock (2956,3146,0).
    let mut c = scene_client();
    plant_loc(&mut c, 2082, "Gangplank", "Cross", 1, 1);
    let mut snap = snap_at(&mut c, 1, 1);
    let mut rec = FollowRec {
        route: Some((1, 1)),
        ..FollowRec::default()
    };
    let mut t = Traveller::new();
    let dock = WorldTile {
        x: 3201,
        z: 3203,
        level: 0,
    };
    let edge = TransportEdge {
        takeoff: None,
        worn_all_req: Vec::new(),
        kind: TransportKind::Ladder,
        player_delta: None,
        at: WorldTile {
            x: 3201,
            z: 3201,
            level: 0,
        },
        to: dock,
        loc_id: 2082,
        option: 1,
        ticks: 2,
        dir: None,
        open_loc_id: None,
        skill_req: vec![],
        item_req: vec![],
        consumed_req: vec![],
        item_returns: vec![],
        quest_req: vec![],
        varp_req: vec![],
        worn_req: vec![],
        members_req: false,
        wildy_cap: None,
        quest_gates: None,
    };
    let route = Route {
        legs: vec![Leg::Transport {
            edge: Box::new(edge),
        }],
        dest: dock,
        ticks: 2.0,
    };
    let mut options = TravelOptions::default();
    let outcome = drive(
        &mut t,
        &mut rec,
        &mut c,
        &mut snap,
        &route,
        &mut options,
        |c| {
            plant_player(c, 1, 3);
        },
    );
    assert!(matches!(outcome, TravelOutcome::Arrived { .. }));
    assert_eq!(rec.loc_ops, 1, "Cross the boat-side gangplank");
    assert_eq!(rec.npc_ops, 0);
}

#[test]
fn dest_dialog_choice_indexes_spirit_tree_siblings() {
    let village = WorldTile {
        x: 2542,
        z: 3169,
        level: 0,
    };
    let varrock = WorldTile {
        x: 2542,
        z: 3168,
        level: 0,
    };
    let tree = |to| TransportEdge {
        takeoff: None,
        worn_all_req: Vec::new(),
        kind: TransportKind::SpiritTree,
        player_delta: None,
        at: WorldTile {
            x: 2461,
            z: 3444,
            level: 0,
        },
        to,
        loc_id: 1293,
        option: 1,
        ticks: 1,
        dir: None,
        open_loc_id: None,
        skill_req: vec![],
        item_req: vec![],
        consumed_req: vec![],
        item_returns: vec![],
        quest_req: vec![],
        varp_req: vec![],
        worn_req: vec![],
        members_req: false,
        wildy_cap: None,
        quest_gates: None,
    };
    let packed = [tree(village), tree(varrock)];
    let leg = Leg::Transport {
        edge: Box::new(packed[1].clone()),
    };
    assert_eq!(
        super::dest_dialog_choice(&leg, None, Some(&packed)),
        2,
        "the second dest of the same tree is choice 2"
    );
}

#[test]
fn dest_map_component_is_the_glidermap_button_for_the_pad() {
    let mut edge = cart_edge();
    edge.kind = TransportKind::Glider;
    edge.loc_id = 170;
    edge.to = WorldTile {
        x: 2971,
        z: 2969,
        level: 0,
    };
    assert_eq!(
        super::dest_map_component(&edge),
        Some(824),
        "gandius is glidermap:com_21"
    );
    edge.kind = TransportKind::Boat;
    assert_eq!(
        super::dest_map_component(&edge),
        None,
        "ship_journey has no dest IF_BUTTON"
    );
}

#[test]
fn follow_glider_answers_talk_then_presses_the_map_dest() {
    // Gnome Air: Talk-to the pilot, first chat choice ("Can you take
    // me on the glider?"), then IF_BUTTON the packed dest on
    // glidermap — never dest-index chat and never Close Window.
    let mut c = scene_client();
    plant_npc_ops(&mut c, 170, 1, 1, "Gnome pilot", &["Talk-to"]);
    let mut snap = snap_at(&mut c, 1, 2);
    let mut rec = FollowRec {
        route: Some((1, 2)),
        ..FollowRec::default()
    };
    let mut t = Traveller::new();
    let gandius = WorldTile {
        x: 2971,
        z: 2969,
        level: 0,
    };
    let mut edge = cart_edge();
    edge.kind = TransportKind::Glider;
    edge.loc_id = 170;
    edge.to = gandius;
    let route = Route {
        legs: vec![Leg::Transport {
            edge: Box::new(edge),
        }],
        dest: gandius,
        ticks: 4.0,
    };
    let mut options = TravelOptions::default();
    assert!(t
        .follow(&mut rec, &snap, route.clone(), &mut options)
        .is_none());
    assert_eq!(rec.npc_ops, 1, "Talk-to the pilot");
    plant_choice_dialog(
        &mut c,
        &[
            "Can you take me on the glider?",
            "Why are gliders better than other transport?",
            "Sorry, I don't want anything now.",
        ],
    );
    bump_rebuild(&mut c, &mut snap);
    assert!(t
        .follow(&mut rec, &snap, route.clone(), &mut options)
        .is_none());
    assert_eq!(
        rec.if_button_components,
        vec![101],
        "first chat choice, not a dest index"
    );
    plant_glider_map(&mut c, 824);
    bump_rebuild(&mut c, &mut snap);
    assert!(t
        .follow(&mut rec, &snap, route.clone(), &mut options)
        .is_none());
    assert_eq!(
        rec.if_button_components,
        vec![101, 824],
        "glidermap dest is IF_BUTTON 824 (gandius)"
    );
    plant_player(&mut c, -229, -231); // world (2971, 2969)
    bump_rebuild(&mut c, &mut snap);
    match t.follow(&mut rec, &snap, route.clone(), &mut options) {
        Some(TravelOutcome::Arrived { at }) => assert_eq!(at, gandius),
        other => panic!("expected Arrived, got {other:?}"),
    }
}

/// `gnome_glider.rs2` lands at `map_findsquare($dest, 0, 1,
/// lineofwalk)` — chebyshev 1, never the pad exactly when the pad is
/// occupied. Live Kar-Hewo Expired on (3285,3211) aiming (3284,3211)
/// because the hop used the runner's exact `close_enough`.
#[test]
fn follow_glider_arrives_within_map_findsquare_radius_1() {
    let mut c = scene_client();
    plant_npc_ops(&mut c, 170, 1, 1, "Gnome pilot", &["Talk-to"]);
    let mut snap = snap_at(&mut c, 1, 2);
    let mut rec = FollowRec {
        route: Some((1, 2)),
        ..FollowRec::default()
    };
    let mut t = Traveller::new();
    let pad = WorldTile {
        x: 3284,
        z: 3211,
        level: 0,
    };
    let mut edge = cart_edge();
    edge.kind = TransportKind::Glider;
    edge.loc_id = 170;
    edge.to = pad;
    let route = Route {
        legs: vec![Leg::Transport {
            edge: Box::new(edge),
        }],
        dest: pad,
        ticks: 4.0,
    };
    let mut options = TravelOptions {
        close_enough: 0,
        ..TravelOptions::default()
    };
    assert!(t
        .follow(&mut rec, &snap, route.clone(), &mut options)
        .is_none());
    plant_choice_dialog(
        &mut c,
        &[
            "Can you take me on the glider?",
            "Why are gliders better than other transport?",
            "Sorry, I don't want anything now.",
        ],
    );
    bump_rebuild(&mut c, &mut snap);
    assert!(t
        .follow(&mut rec, &snap, route.clone(), &mut options)
        .is_none());
    plant_glider_map(&mut c, 828);
    bump_rebuild(&mut c, &mut snap);
    assert!(t
        .follow(&mut rec, &snap, route.clone(), &mut options)
        .is_none());
    plant_player(&mut c, 85, 11); // world (3285, 3211), cheb 1 of the pad
    bump_rebuild(&mut c, &mut snap);
    match t.follow(&mut rec, &snap, route.clone(), &mut options) {
        Some(TravelOutcome::Arrived { at }) => assert_eq!(
            at,
            WorldTile {
                x: 3285,
                z: 3211,
                level: 0
            }
        ),
        other => panic!("expected Arrived within radius 1, got {other:?}"),
    }
}

#[test]
fn follow_spirit_tree_answers_gate_then_second_dest() {
    // Adult spirit tree (`spirit_tree.rs2` ent): mesbox continues,
    // then "No thanks" / "Where can I go?", then the 3 dests.
    // Dest-index 1 on the gate page returns without a telejump.
    let mut c = scene_client();
    plant_loc(&mut c, 1293, "Spirit tree", "Talk-to", 1, 1);
    let mut snap = snap_at(&mut c, 1, 1);
    let mut rec = FollowRec {
        route: Some((1, 1)),
        ..FollowRec::default()
    };
    let mut t = Traveller::new();
    let at = WorldTile {
        x: 3201,
        z: 3201,
        level: 0,
    };
    let village = WorldTile {
        x: 2542,
        z: 3169,
        level: 0,
    };
    let varrock = WorldTile {
        x: 3179,
        z: 3507,
        level: 0,
    };
    let khazard = WorldTile {
        x: 2555,
        z: 3259,
        level: 0,
    };
    let tree = |to| TransportEdge {
        takeoff: None,
        worn_all_req: Vec::new(),
        kind: TransportKind::SpiritTree,
        player_delta: None,
        at,
        to,
        loc_id: 1293,
        option: 1,
        ticks: 1,
        dir: None,
        open_loc_id: None,
        skill_req: vec![],
        item_req: vec![],
        consumed_req: vec![],
        item_returns: vec![],
        quest_req: vec![],
        varp_req: vec![],
        worn_req: vec![],
        members_req: false,
        wildy_cap: None,
        quest_gates: None,
    };
    let packed = [tree(village), tree(varrock), tree(khazard)];
    let route = Route {
        legs: vec![Leg::Transport {
            edge: Box::new(packed[1].clone()),
        }],
        dest: varrock,
        ticks: 1.0,
    };
    let mut options = TravelOptions {
        edges: Some(&packed),
        ..TravelOptions::default()
    };
    assert!(t
        .follow(&mut rec, &snap, route.clone(), &mut options)
        .is_none());
    assert_eq!(rec.loc_ops, 1, "oploc1 the tree");
    plant_continue_dialog(&mut c);
    bump_rebuild(&mut c, &mut snap);
    assert!(t
        .follow(&mut rec, &snap, route.clone(), &mut options)
        .is_none());
    assert_eq!(rec.pause_buttons, 1, "mesbox continue");
    plant_choice_dialog(&mut c, &["No thanks, old tree.", "Where can I go?"]);
    bump_rebuild(&mut c, &mut snap);
    assert!(t
        .follow(&mut rec, &snap, route.clone(), &mut options)
        .is_none());
    assert_eq!(
        rec.if_button_components,
        vec![102],
        "gate answers Where can I go? (choice 2), not No thanks"
    );
    plant_choice_dialog(
        &mut c,
        &[
            "Tree Gnome Village.",
            "Forest north of Varrock.",
            "Battlefield of Khazard.",
        ],
    );
    bump_rebuild(&mut c, &mut snap);
    assert!(t
        .follow(&mut rec, &snap, route.clone(), &mut options)
        .is_none());
    assert_eq!(
        rec.if_button_components,
        vec![102, 102],
        "dest page answers Forest north of Varrock (choice 2)"
    );
    plant_player(&mut c, -21, 307);
    bump_rebuild(&mut c, &mut snap);
    match t.follow(&mut rec, &snap, route.clone(), &mut options) {
        Some(TravelOutcome::Arrived { at }) => {
            assert_eq!(at, varrock);
        }
        other => panic!("expected Arrived, got {other:?}"),
    }
    assert_eq!(rec.if_buttons, 2, "gate and dest, never re-pressed");
}

#[test]
fn follow_approaches_an_npc_before_interacting() {
    // The game only accepts an interact from adjacent: starting 3
    // tiles away from the driver NPC, `follow` must first walk to the
    // nearest standable tile within chebyshev 1 of `at` and only then
    // send the NPC interact, exactly like the ladder approach.
    let mut c = scene_client();
    plant_driver_npc(&mut c, 7, 1, 1);
    let mut snap = snap_at(&mut c, 1, 4); // cheb 3 south of the driver
    let mut rec = FollowRec {
        route: Some((1, 4)),
        ..FollowRec::default()
    };
    let mut t = Traveller::new();
    let route = Route {
        legs: vec![Leg::Transport {
            edge: Box::new(cart_edge()),
        }],
        dest: WorldTile {
            x: 3300,
            z: 3200,
            level: 0,
        },
        ticks: 1.0,
    };
    let mut options = TravelOptions::default();
    // Poll 1: the hop walks to the adjacent standable tile, never the
    // interact (the click would be dropped from 3 tiles away).
    assert!(t
        .follow(&mut rec, &snap, route.clone(), &mut options)
        .is_none());
    assert_eq!(rec.walked, vec![(1, 2)], "the approach walk goes out first");
    assert_eq!(rec.npc_ops, 0, "no OP_NPC1 before the player is adjacent");
    // The player steps onto the approach tile: the hop sends `op_npc`.
    plant_player(&mut c, 1, 2);
    bump_rebuild(&mut c, &mut snap);
    assert!(t
        .follow(&mut rec, &snap, route.clone(), &mut options)
        .is_none());
    assert_eq!(rec.npc_ops, 1, "one OP_NPC1 once adjacent");
    plant_player(&mut c, 100, 0);
    bump_rebuild(&mut c, &mut snap);
    assert!(matches!(
        t.follow(&mut rec, &snap, route.clone(), &mut options),
        Some(TravelOutcome::Arrived { .. })
    ));
}

#[test]
fn follow_sits_out_a_refused_npc_approach_right_after_a_scene_change() {
    // Frozen WalkExecutor.ts:1178-1184: with nothing clicked yet the scene
    // may still be loading, so a refused click is retried two ticks later
    // (CANDIDATE_SETTLE_TRIES per walk) instead of failing the follow.
    let mut c = scene_client();
    plant_driver_npc(&mut c, 7, 1, 1);
    let mut snap = snap_at(&mut c, 1, 4);
    let mut rec = FollowRec {
        route: Some((1, 4)),
        reject_to: Some((1, 2)),
        ..FollowRec::default()
    };
    let mut t = Traveller::new();
    let route = Route {
        legs: vec![Leg::Transport {
            edge: Box::new(cart_edge()),
        }],
        dest: WorldTile {
            x: 3300,
            z: 3200,
            level: 0,
        },
        ticks: 1.0,
    };
    let mut options = TravelOptions::default();
    assert!(
        t.follow(&mut rec, &snap, route.clone(), &mut options)
            .is_none(),
        "a refused approach right after the rebuild is not terminal"
    );
    assert!(rec.walked.is_empty());
    // The scene settles: the client accepts the same approach click.
    rec.reject_to = None;
    bump_rebuild(&mut c, &mut snap);
    assert!(t
        .follow(&mut rec, &snap, route.clone(), &mut options)
        .is_none());
    assert!(
        rec.walked.is_empty(),
        "the retry waits the frozen two ticks"
    );
    bump_rebuild(&mut c, &mut snap);
    assert!(t
        .follow(&mut rec, &snap, route.clone(), &mut options)
        .is_none());
    assert_eq!(
        rec.walked,
        vec![(1, 2)],
        "the approach goes out once settled"
    );
}

#[test]
fn follow_npc_approach_refused_past_the_settle_budget_ends_refused() {
    let mut c = scene_client();
    plant_driver_npc(&mut c, 7, 1, 1);
    let mut snap = snap_at(&mut c, 1, 4);
    let mut rec = FollowRec {
        route: Some((1, 4)),
        reject_to: Some((1, 2)),
        ..FollowRec::default()
    };
    let mut t = Traveller::new();
    let route = Route {
        legs: vec![Leg::Transport {
            edge: Box::new(cart_edge()),
        }],
        dest: WorldTile {
            x: 3300,
            z: 3200,
            level: 0,
        },
        ticks: 1.0,
    };
    let mut options = TravelOptions::default();
    let mut outcome = None;
    for _ in 0..20 {
        outcome = t.follow(&mut rec, &snap, route.clone(), &mut options);
        if outcome.is_some() {
            break;
        }
        bump_rebuild(&mut c, &mut snap);
    }
    assert!(
        matches!(
            outcome,
            Some(TravelOutcome::Refused {
                reason: SendReason::Unreachable,
                ..
            })
        ),
        "three sat-out refusals, then the fourth ends the follow: {outcome:?}"
    );
}

/// Packed `at` is the pier spawn, not a leash. Live Musa customs
/// wander; talking from cheb-1 of the spawn while the officer is
/// four tiles away is `I can't reach that!`. Approach the live tile.
#[test]
fn follow_npc_edge_approaches_the_live_tile_when_the_driver_wandered() {
    let mut c = scene_client();
    plant_driver_npc(&mut c, 7, 5, 1); // world (3205, 3201), 4 tiles east of packed at
    let mut snap = snap_at(&mut c, 1, 1); // packed cart_edge.at
    let mut rec = FollowRec {
        route: Some((1, 1)),
        ..FollowRec::default()
    };
    let mut t = Traveller::new();
    let route = Route {
        legs: vec![Leg::Transport {
            edge: Box::new(cart_edge()),
        }],
        dest: WorldTile {
            x: 3300,
            z: 3200,
            level: 0,
        },
        ticks: 1.0,
    };
    let mut options = TravelOptions::default();
    assert!(t
        .follow(&mut rec, &snap, route.clone(), &mut options)
        .is_none());
    assert_eq!(rec.npc_ops, 0, "do not Talk-to from the packed spawn");
    assert!(
        !rec.walked.is_empty(),
        "walk toward the live NPC, got {:?}",
        rec.walked
    );
    plant_player(&mut c, 4, 1);
    bump_rebuild(&mut c, &mut snap);
    assert!(t
        .follow(&mut rec, &snap, route.clone(), &mut options)
        .is_none());
    assert_eq!(rec.npc_ops, 1, "Talk-to once adjacent to the wanderer");
    // The cart carries the player to `edge.to`: the run arrives.
    plant_player(&mut c, 100, 0);
    bump_rebuild(&mut c, &mut snap);
    match t.follow(&mut rec, &snap, route.clone(), &mut options) {
        Some(TravelOutcome::Arrived { at }) => {
            assert_eq!(
                at,
                WorldTile {
                    x: 3300,
                    z: 3200,
                    level: 0
                }
            );
        }
        other => panic!("expected Arrived, got {other:?}"),
    }
}

#[test]
fn follow_reapproaches_when_the_tracked_npc_network_tile_moves_after_approach() {
    let mut c = scene_client();
    plant_driver_npc(&mut c, 7, 2, 1);
    let mut snap = snap_at(&mut c, 0, 1);
    let mut t = Traveller::new();
    let route = Route {
        legs: vec![Leg::Transport {
            edge: Box::new(cart_edge()),
        }],
        dest: WorldTile {
            x: 3300,
            z: 3200,
            level: 0,
        },
        ticks: 1.0,
    };
    let walks = std::cell::RefCell::new(Vec::new());
    let transports = std::cell::Cell::new(0);
    let mut options = TravelOptions {
        on_event: Some(Box::new(|event| match event {
            TravelEvent::WalkAttempt { aim, .. } => walks.borrow_mut().push(aim),
            TravelEvent::TransportAttempt { .. } => transports.set(transports.get() + 1),
            _ => {}
        })),
        ..TravelOptions::default()
    };

    assert!(t
        .follow(&mut c, &snap, route.clone(), &mut options)
        .is_none());
    assert_eq!(
        walks.borrow().as_slice(),
        &[WorldTile {
            x: 3201,
            z: 3201,
            level: 0
        }]
    );
    assert_eq!(transports.get(), 0);

    // The player reaches the old approach stand while the same NPC slot's
    // route head moves three tiles away. Keep the rendered tile stale to
    // exercise the packet target (`NpcView.network`), not interpolation.
    plant_player(&mut c, 1, 1);
    let npc = c.npc[0].as_mut().unwrap();
    npc.entity.route_x[0] = 4;
    npc.entity.route_z[0] = 1;
    bump_rebuild(&mut c, &mut snap);
    assert_eq!(snap.npcs()[0].index, 0);
    assert_eq!(
        snap.npcs()[0].tile,
        WorldTile {
            x: 3202,
            z: 3201,
            level: 0
        }
    );
    assert_eq!(
        snap.npcs()[0].network,
        WorldTile {
            x: 3204,
            z: 3201,
            level: 0
        }
    );

    assert!(t
        .follow(&mut c, &snap, route.clone(), &mut options)
        .is_none());
    assert_eq!(
        transports.get(),
        0,
        "do not send OP_NPC from the stale stand"
    );
    assert_eq!(
        walks.borrow().as_slice(),
        &[
            WorldTile {
                x: 3201,
                z: 3201,
                level: 0
            },
            WorldTile {
                x: 3203,
                z: 3201,
                level: 0
            },
        ],
        "re-approach the tracked NPC's current network tile"
    );
}

#[test]
fn follow_moving_tracked_npc_stalls_within_one_approach_budget() {
    let mut c = scene_client();
    plant_driver_npc(&mut c, 7, 2, 1);
    let mut snap = snap_at(&mut c, 0, 1);
    let mut t = Traveller::new();
    let route = Route {
        legs: vec![Leg::Transport {
            edge: Box::new(cart_edge()),
        }],
        dest: WorldTile {
            x: 3300,
            z: 3200,
            level: 0,
        },
        ticks: 1.0,
    };
    let walks = std::cell::RefCell::new(Vec::new());
    let transports = std::cell::Cell::new(0);
    let budget = 6;
    let mut options = TravelOptions {
        budget_ticks_per_hop: budget,
        on_event: Some(Box::new(|event| match event {
            TravelEvent::WalkAttempt { aim, .. } => walks.borrow_mut().push(aim),
            TravelEvent::TransportAttempt { .. } => transports.set(transports.get() + 1),
            _ => {}
        })),
        ..TravelOptions::default()
    };

    let mut outcome = None;
    let mut polls = 0;
    for poll in 1..=budget + 3 {
        polls = poll;
        outcome = t.follow(&mut c, &snap, route.clone(), &mut options);
        if outcome.is_some() {
            break;
        }

        // Settle every accepted approach, then move the same NPC slot two
        // tiles farther away before the next poll. A per-re-arm clock would
        // chase forever; the hop-wide clock must still expire.
        let aim = walks
            .borrow()
            .last()
            .copied()
            .expect("each chase poll arms an approach");
        plant_player(&mut c, aim.x - 3200, aim.z - 3200);
        c.npc[0].as_mut().unwrap().entity.route_x[0] += 2;
        bump_rebuild(&mut c, &mut snap);
    }

    assert!(
        matches!(outcome, Some(TravelOutcome::Stalled { .. })),
        "the moving target must stall, got {outcome:?}"
    );
    assert!(
        polls <= budget + 2,
        "the cumulative approach clock ended on poll {polls}, budget {budget}"
    );
    assert_eq!(
        transports.get(),
        0,
        "never interact while the tracked NPC keeps moving"
    );
}

#[test]
fn follow_reapproaches_tracked_npc_beyond_the_initial_search_radius() {
    let mut c = scene_client();
    // The selected slot begins seven tiles from edge.at, inside the initial
    // eight-tile candidate radius.
    plant_driver_npc(&mut c, 7, 8, 1);
    let mut snap = snap_at(&mut c, 0, 1);
    let mut t = Traveller::new();
    let route = Route {
        legs: vec![Leg::Transport {
            edge: Box::new(cart_edge()),
        }],
        dest: WorldTile {
            x: 3300,
            z: 3200,
            level: 0,
        },
        ticks: 1.0,
    };
    let walks = std::cell::RefCell::new(Vec::new());
    let transports = std::cell::Cell::new(0);
    let mut options = TravelOptions {
        on_event: Some(Box::new(|event| match event {
            TravelEvent::WalkAttempt { aim, .. } => walks.borrow_mut().push(aim),
            TravelEvent::TransportAttempt { .. } => transports.set(transports.get() + 1),
            _ => {}
        })),
        ..TravelOptions::default()
    };

    assert!(t
        .follow(&mut c, &snap, route.clone(), &mut options)
        .is_none());
    plant_player(&mut c, 7, 1);
    // The same index and type wanders eleven tiles from edge.at. It remains
    // the tracked instance even though a fresh candidate search would omit it.
    c.npc[0].as_mut().unwrap().entity.route_x[0] = 12;
    bump_rebuild(&mut c, &mut snap);
    assert!(t
        .follow(&mut c, &snap, route.clone(), &mut options)
        .is_none());

    assert_eq!(
        walks.borrow().as_slice(),
        &[
            WorldTile {
                x: 3207,
                z: 3201,
                level: 0,
            },
            WorldTile {
                x: 3211,
                z: 3201,
                level: 0,
            },
        ],
        "re-approach the selected slot outside the initial search radius"
    );
    assert_eq!(transports.get(), 0);
}

#[test]
fn follow_revalidates_after_a_wanderer_temporarily_has_no_standable_ring() {
    let mut c = scene_client();
    plant_driver_npc(&mut c, 7, 2, 1);
    let mut snap = snap_at(&mut c, 0, 1);
    let mut t = Traveller::new();
    let route = Route {
        legs: vec![Leg::Transport {
            edge: Box::new(cart_edge()),
        }],
        dest: WorldTile {
            x: 3300,
            z: 3200,
            level: 0,
        },
        ticks: 1.0,
    };
    let walks = std::cell::RefCell::new(Vec::new());
    let transports = std::cell::Cell::new(0);
    let mut options = TravelOptions {
        on_event: Some(Box::new(|event| match event {
            TravelEvent::WalkAttempt { aim, .. } => walks.borrow_mut().push(aim),
            TravelEvent::TransportAttempt { .. } => transports.set(transports.get() + 1),
            _ => {}
        })),
        ..TravelOptions::default()
    };

    assert!(t
        .follow(&mut c, &snap, route.clone(), &mut options)
        .is_none());
    assert_eq!(walks.borrow().len(), 1);

    // The player settles the first approach while the NPC moves into a
    // completely blocked ring. No replacement walk can be sent this poll.
    plant_player(&mut c, 1, 1);
    {
        let npc = c.npc[0].as_mut().unwrap();
        npc.entity.route_x[0] = 5;
        npc.entity.route_z[0] = 5;
    }
    for x in 4..=6 {
        for z in 4..=6 {
            c.collision[0].flags[x][z] |= client::dash3d::CollisionFlag::SQ_BLOCKED;
        }
    }
    bump_rebuild(&mut c, &mut snap);
    assert!(t
        .follow(&mut c, &snap, route.clone(), &mut options)
        .is_none());
    assert_eq!(walks.borrow().len(), 1);

    // Once the tracked slot returns to an approachable tile, the next poll
    // must revalidate immediately rather than waiting beside a walk never sent.
    {
        let npc = c.npc[0].as_mut().unwrap();
        npc.entity.route_x[0] = 4;
        npc.entity.route_z[0] = 1;
    }
    bump_rebuild(&mut c, &mut snap);
    assert!(t
        .follow(&mut c, &snap, route.clone(), &mut options)
        .is_none());
    assert_eq!(
        walks.borrow().last(),
        Some(&WorldTile {
            x: 3203,
            z: 3201,
            level: 0,
        })
    );
    assert_eq!(transports.get(), 0);
}

#[test]
fn follow_npc_interaction_refreshes_the_reach_chat_watermark_when_sent() {
    let mut c = scene_client();
    plant_driver_npc(&mut c, 7, 2, 1);
    let mut snap = snap_at(&mut c, 0, 1);
    let mut rec = FollowRec {
        route: Some((0, 1)),
        ..FollowRec::default()
    };
    let mut t = Traveller::new();
    let route = Route {
        legs: vec![Leg::Transport {
            edge: Box::new(cart_edge()),
        }],
        dest: WorldTile {
            x: 3300,
            z: 3200,
            level: 0,
        },
        ticks: 1.0,
    };
    let mut options = TravelOptions::default();

    assert!(t
        .follow(&mut rec, &snap, route.clone(), &mut options)
        .is_none());
    assert_eq!(rec.npc_ops, 0);

    // This line predates the OP_NPC send. It must be behind the watermark
    // captured when the player reaches the approach stand and Talk-to goes out.
    c.add_chat(0, "I can't reach that!", "");
    plant_player(&mut c, 1, 1);
    bump_rebuild(&mut c, &mut snap);
    assert!(t
        .follow(&mut rec, &snap, route.clone(), &mut options)
        .is_none());
    assert_eq!(rec.npc_ops, 1);

    bump_rebuild(&mut c, &mut snap);
    assert!(
        t.follow(&mut rec, &snap, route.clone(), &mut options)
            .is_none(),
        "a reach line from before the interaction must not abort the hop"
    );
    plant_player(&mut c, 100, 0);
    bump_rebuild(&mut c, &mut snap);
    assert!(matches!(
        t.follow(&mut rec, &snap, route, &mut options),
        Some(TravelOutcome::Arrived { .. })
    ));
}

#[test]
fn follow_npc_edge_blocks_when_the_driver_is_out_of_scene() {
    // No NPC of the edge's type within search radius of `at`: the hop waits out
    // its loc budget and reports `Blocked`, never a loc-shaped lookup
    // against a phantom loc id.
    let mut c = scene_client();
    // No driver planted: the only scene npcs are none.
    let mut snap = snap_at(&mut c, 1, 1);
    let mut rec = FollowRec {
        route: Some((1, 1)),
        ..FollowRec::default()
    };
    let mut t = Traveller::new();
    let route = Route {
        legs: vec![Leg::Transport {
            edge: Box::new(cart_edge()),
        }],
        dest: WorldTile {
            x: 3300,
            z: 3200,
            level: 0,
        },
        ticks: 1.0,
    };
    let mut options = TravelOptions {
        budget_ticks_per_hop: 2,
        ..TravelOptions::default()
    };
    let mut outcome = None;
    for _ in 0..4 {
        if let Some(o) = t.follow(&mut rec, &snap, route.clone(), &mut options) {
            outcome = Some(o);
            break;
        }
        bump_rebuild(&mut c, &mut snap);
    }
    match outcome {
        Some(TravelOutcome::Blocked { detail, .. }) => {
            assert!(
                detail.contains("npc"),
                "the block names the missing NPC, got: {detail}"
            );
        }
        other => panic!("expected Blocked for a missing driver, got {other:?}"),
    }
}

#[test]
fn follow_npc_edge_answers_the_fare_dialog_before_arriving() {
    // The live cart drivers' `opnpc1` opens a chat dialog instead of
    // riding immediately: a "Hello!" page with a Continue button, then
    // the fare page ("Is that Ok?" + a "Yes please…" choice). After
    // the NPC interact, the hop must press Continue (each page), then
    // press the edge's choice (option 1 = the "Yes please" button)
    // exactly once, and settle `arrived(edge.to)`.
    let mut c = scene_client();
    plant_driver_npc(&mut c, 7, 1, 1);
    let mut snap = snap_at(&mut c, 1, 2);
    let mut rec = FollowRec {
        route: Some((1, 2)),
        ..FollowRec::default()
    };
    let mut t = Traveller::new();
    let route = Route {
        legs: vec![Leg::Transport {
            edge: Box::new(cart_edge()),
        }],
        dest: WorldTile {
            x: 3300,
            z: 3200,
            level: 0,
        },
        ticks: 1.0,
    };
    let mut options = TravelOptions::default();
    // Poll 1: the hop sends the NPC interact; no dialog is up yet.
    assert!(t
        .follow(&mut rec, &snap, route.clone(), &mut options)
        .is_none());
    assert_eq!(rec.npc_ops, 1, "the NPC interact went out");
    assert_eq!(rec.if_buttons, 0, "no answer before the dialog opens");
    assert_eq!(rec.pause_buttons, 0);

    // The driver opens the "Hello!" page with a Continue button.
    plant_continue_dialog(&mut c);
    bump_rebuild(&mut c, &mut snap);
    assert!(t
        .follow(&mut rec, &snap, route.clone(), &mut options)
        .is_none());
    assert_eq!(rec.pause_buttons, 1, "the hop presses Continue once");
    assert_eq!(rec.if_buttons, 0, "no choice press on the Continue page");

    // The fare page replaces it: the hop presses the edge's choice.
    plant_choice_dialog(
        &mut c,
        &["Yes please, I'd like to go to Brimhaven.", "No thanks."],
    );
    bump_rebuild(&mut c, &mut snap);
    assert!(t
        .follow(&mut rec, &snap, route.clone(), &mut options)
        .is_none());
    assert_eq!(rec.if_buttons, 1, "one IF_BUTTON press for the fare choice");
    // Still waiting — the choice is never re-pressed.
    bump_rebuild(&mut c, &mut snap);
    assert!(t
        .follow(&mut rec, &snap, route.clone(), &mut options)
        .is_none());
    assert_eq!(rec.if_buttons, 1, "the dialog is answered exactly once");

    // The post-choice "Great!" page still needs a Continue before the
    // ride leaves.
    plant_continue_dialog(&mut c);
    bump_rebuild(&mut c, &mut snap);
    assert!(t
        .follow(&mut rec, &snap, route.clone(), &mut options)
        .is_none());
    assert_eq!(
        rec.pause_buttons, 2,
        "the post-choice page is continued before the ride"
    );

    // The cart carries the player to `edge.to`: the run arrives.
    plant_player(&mut c, 100, 0);
    bump_rebuild(&mut c, &mut snap);
    match t.follow(&mut rec, &snap, route.clone(), &mut options) {
        Some(TravelOutcome::Arrived { at }) => {
            assert_eq!(
                at,
                WorldTile {
                    x: 3300,
                    z: 3200,
                    level: 0
                }
            );
        }
        other => panic!("expected Arrived, got {other:?}"),
    }
    assert_eq!(rec.npc_ops, 1, "one OP_NPC interact");
    assert_eq!(rec.loc_ops, 0, "an Npc edge never sends OP_LOC1");
}

/// Scene origin that places the real Ranging Guild stands inside the
/// 104-tile fixture scene without inventing hop offsets.
const RANGING_SCENE_BASE: (i32, i32) = (2650, 3430);
const RANGING_OUTSIDE: WorldTile = WorldTile {
    x: 2657,
    z: 3439,
    level: 0,
};
const RANGING_INSIDE: WorldTile = WorldTile {
    x: 2659,
    z: 3437,
    level: 0,
};
const RANGING_LOC: WorldTile = WorldTile {
    x: 2658,
    z: 3438,
    level: 0,
};
const SHANTAY_NORTH_SCENE_BASE: (i32, i32) = (3296, 3104);
const SHANTAY_NORTH_AT: WorldTile = WorldTile {
    x: 3302,
    z: 3116,
    level: 0,
};
const SHANTAY_NORTH_TO: WorldTile = WorldTile {
    x: 3304,
    z: 3115,
    level: 0,
};

fn scene_of(base: (i32, i32), tile: WorldTile) -> (i32, i32) {
    (tile.x - base.0, tile.z - base.1)
}

fn rangingguild_enter_edge() -> TransportEdge {
    TransportEdge {
        takeoff: None,
        worn_all_req: Vec::new(),
        kind: TransportKind::Door,
        player_delta: None,
        at: RANGING_OUTSIDE,
        to: RANGING_INSIDE,
        loc_id: 2514,
        option: 1,
        ticks: 1,
        dir: None,
        open_loc_id: None,
        skill_req: vec![(4, 40)],
        item_req: vec![],
        consumed_req: vec![],
        item_returns: vec![],
        quest_req: vec![],
        varp_req: vec![],
        worn_req: vec![],
        members_req: false,
        wildy_cap: None,
        quest_gates: None,
    }
}

fn rangingguild_exit_edge() -> TransportEdge {
    TransportEdge {
        takeoff: None,
        skill_req: vec![],
        at: RANGING_INSIDE,
        to: RANGING_OUTSIDE,
        ..rangingguild_enter_edge()
    }
}

fn shantay_north_short_edge() -> TransportEdge {
    TransportEdge {
        takeoff: None,
        worn_all_req: Vec::new(),
        kind: TransportKind::Door,
        player_delta: None,
        at: SHANTAY_NORTH_AT,
        to: SHANTAY_NORTH_TO,
        loc_id: SHANTAY_HENGE_LOC_ID,
        option: 1,
        ticks: 3,
        dir: None,
        open_loc_id: None,
        skill_req: vec![],
        item_req: vec![],
        consumed_req: vec![(1854, 1)],
        item_returns: vec![],
        quest_req: vec![],
        varp_req: vec![],
        worn_req: vec![],
        members_req: false,
        wildy_cap: None,
        quest_gates: None,
    }
}

fn follow_still_pending<D: Driver>(
    t: &mut Traveller,
    d: &mut D,
    snap: &GameSnapshot,
    route: &Route,
    options: &mut TravelOptions<'_>,
    label: &str,
) {
    assert!(
        t.follow(d, snap, route.clone(), options).is_none(),
        "{label} must stay pending"
    );
}

/// The Shantay henge gated hop on the fixture scene: loc 4031 at the
/// edge's `at` (3201, 3201) — the live target the `op_loc` resolves
/// through — `to` the `[queue,shantay_pass_enter]` landing.
fn shantay_edge() -> TransportEdge {
    TransportEdge {
        takeoff: None,
        worn_all_req: Vec::new(),
        kind: TransportKind::Door,
        player_delta: None,
        at: WorldTile {
            x: 3201,
            z: 3201,
            level: 0,
        },
        to: WorldTile {
            x: 3200,
            z: 3210,
            level: 0,
        },
        loc_id: SHANTAY_HENGE_LOC_ID,
        option: 1,
        ticks: 3,
        dir: None,
        open_loc_id: None,
        skill_req: vec![],
        item_req: vec![],
        consumed_req: vec![(1854, 1)],
        item_returns: vec![],
        quest_req: vec![],
        varp_req: vec![],
        worn_req: vec![],
        members_req: false,
        wildy_cap: None,
        quest_gates: None,
    }
}

#[test]
fn follow_shantay_door_edge_drives_the_pass_handover_dialog_before_arriving() {
    // The Shantay henge's gated `oploc1` (loc 4031) shows the pass
    // handover before consuming the pass and teleporting:
    // `~chatnpc("Can I see your Shantay Desert Pass please.")`,
    // `~objbox(...)`, and `~chatplayer(...)` — each a
    // `p_pausebutton` chat modal. A Door hop must press those
    // continue pages (one per poll) and then settle `arrived(to)`,
    // like the Npc ride dialogs.
    let mut c = scene_client();
    plant_loc(
        &mut c,
        SHANTAY_HENGE_LOC_ID,
        "Shantay pass henge doorway",
        "Go-through",
        1,
        1,
    );
    let mut snap = snap_at(&mut c, 0, 1);
    let mut rec = FollowRec {
        route: Some((0, 1)),
        ..FollowRec::default()
    };
    let mut t = Traveller::new();
    let route = Route {
        legs: vec![Leg::Transport {
            edge: Box::new(shantay_edge()),
        }],
        dest: WorldTile {
            x: 3200,
            z: 3210,
            level: 0,
        },
        ticks: 1.0,
    };
    let mut options = TravelOptions::default();
    // Poll 1: the hop sends the loc interact; no dialog is up yet.
    assert!(t
        .follow(&mut rec, &snap, route.clone(), &mut options)
        .is_none());
    assert_eq!(rec.loc_ops, 1, "the OP_LOC1 interact went out");
    assert_eq!(rec.pause_buttons, 0, "no continue before the dialog opens");

    // The chatnpc page: the hop presses Continue once per page.
    plant_continue_dialog(&mut c);
    bump_rebuild(&mut c, &mut snap);
    assert!(t
        .follow(&mut rec, &snap, route.clone(), &mut options)
        .is_none());
    assert_eq!(rec.pause_buttons, 1, "the chatnpc page is continued");

    // The objbox page.
    plant_continue_dialog(&mut c);
    bump_rebuild(&mut c, &mut snap);
    assert!(t
        .follow(&mut rec, &snap, route.clone(), &mut options)
        .is_none());
    assert_eq!(rec.pause_buttons, 2, "the objbox page is continued");

    // The chatplayer page.
    plant_continue_dialog(&mut c);
    bump_rebuild(&mut c, &mut snap);
    assert!(t
        .follow(&mut rec, &snap, route.clone(), &mut options)
        .is_none());
    assert_eq!(rec.pause_buttons, 3, "the chatplayer page is continued");

    // The branch consumes the pass and teleports the player to `to`;
    // the chatplayer page closed on the press.
    c.chat_modal_id = -1;
    plant_player(&mut c, 0, 10);
    bump_rebuild(&mut c, &mut snap);
    match t.follow(&mut rec, &snap, route.clone(), &mut options) {
        Some(TravelOutcome::Arrived { at }) => {
            assert_eq!(
                at,
                WorldTile {
                    x: 3200,
                    z: 3210,
                    level: 0
                }
            );
        }
        other => panic!("expected Arrived, got {other:?}"),
    }
    assert_eq!(rec.loc_ops, 1, "one OP_LOC1 interact");
    assert_eq!(
        rec.pause_buttons, 3,
        "the three handover pages were pressed"
    );
}

const ALKHARID_TOLL_LOC_ID: i32 = 2882;
const COINS_OBJ: i32 = 995;
/// Not obj 995 — the no-coins follow test must fail if the follower hard-codes coins.
const TEST_TOLL_COIN_OBJ: i32 = 4242;

fn alkharid_toll_edge() -> TransportEdge {
    TransportEdge {
        takeoff: None,
        worn_all_req: Vec::new(),
        kind: TransportKind::Door,
        player_delta: None,
        at: WorldTile {
            x: 3202,
            z: 3201,
            level: 0,
        },
        to: WorldTile {
            x: 3203,
            z: 3201,
            level: 0,
        },
        loc_id: ALKHARID_TOLL_LOC_ID,
        option: 1,
        ticks: 1,
        dir: Some(DoorDir::E),
        open_loc_id: Some(1562),
        skill_req: vec![],
        item_req: vec![],
        consumed_req: vec![(COINS_OBJ, 10)],
        item_returns: vec![],
        quest_req: vec![],
        varp_req: vec![],
        worn_req: vec![],
        members_req: false,
        wildy_cap: None,
        quest_gates: None,
    }
}

fn alkharid_toll_route() -> Route {
    let edge = alkharid_toll_edge();
    Route {
        dest: edge.to,
        ticks: 1.0,
        legs: vec![Leg::Transport {
            edge: Box::new(edge),
        }],
    }
}

fn alkharid_toll_follow_start(
    with_coins: bool,
) -> (Client, GameSnapshot, FollowRec, Traveller, Route) {
    let mut c = scene_client();
    plant_loc(&mut c, ALKHARID_TOLL_LOC_ID, "Gate", "Open", 2, 1);
    if with_coins {
        plant_inv_stack(&mut c, COINS_OBJ, 10);
    }
    let snap = snap_at(&mut c, 1, 1);
    if with_coins {
        assert_eq!(snap.inv_count(COINS_OBJ), 10);
    } else {
        assert_eq!(snap.inv_count(COINS_OBJ), 0);
    }
    let rec = FollowRec {
        route: Some((1, 1)),
        ..FollowRec::default()
    };
    (c, snap, rec, Traveller::new(), alkharid_toll_route())
}

/// The Al Kharid toll's `oploc1` opens the border-guard chat, then
/// `p_choice3` with the pay option last. A Door hop must continue the
/// talk pages and answer "Yes, ok." — never the first "walk around"
/// refuse — then settle `arrived(to)`.
#[test]
fn follow_alkharid_toll_answers_yes_ok_from_content_then_arrives() {
    let (mut c, mut snap, mut rec, mut t, route) = alkharid_toll_follow_start(true);
    let mut options = TravelOptions {
        close_enough: 0,
        ..TravelOptions::default()
    };
    assert!(t
        .follow(&mut rec, &snap, route.clone(), &mut options)
        .is_none());
    assert_eq!(rec.loc_ops, 1, "the OP_LOC1 Open went out");
    assert_eq!(rec.if_buttons, 0, "no answer before the dialog opens");

    plant_continue_dialog(&mut c);
    bump_rebuild(&mut c, &mut snap);
    assert!(t
        .follow(&mut rec, &snap, route.clone(), &mut options)
        .is_none());
    assert_eq!(rec.pause_buttons, 1, "the chatplayer page is continued");

    plant_continue_dialog(&mut c);
    bump_rebuild(&mut c, &mut snap);
    assert!(t
        .follow(&mut rec, &snap, route.clone(), &mut options)
        .is_none());
    assert_eq!(rec.pause_buttons, 2, "the chatnpc toll page is continued");

    plant_choice_dialog(
        &mut c,
        &[
            "No thank you, I'll walk around.",
            "Who does my money go to?",
            "Yes, ok.",
        ],
    );
    bump_rebuild(&mut c, &mut snap);
    assert!(t
        .follow(&mut rec, &snap, route.clone(), &mut options)
        .is_none());
    assert_eq!(rec.if_buttons, 1, "the pay choice is pressed");
    assert_eq!(
        rec.if_button_components,
        vec![103],
        "Yes, ok. is the third option, not the refuse branch"
    );

    plant_continue_dialog(&mut c);
    bump_rebuild(&mut c, &mut snap);
    assert!(t
        .follow(&mut rec, &snap, route.clone(), &mut options)
        .is_none());
    assert_eq!(
        rec.pause_buttons, 3,
        "the post-pay chatplayer page is continued"
    );

    c.chat_modal_id = -1;
    plant_player(&mut c, 3, 1);
    bump_rebuild(&mut c, &mut snap);
    match t.follow(&mut rec, &snap, route, &mut options) {
        Some(TravelOutcome::Arrived { at }) => {
            assert_eq!(
                at,
                WorldTile {
                    x: 3203,
                    z: 3201,
                    level: 0
                }
            );
        }
        other => panic!("expected Arrived, got {other:?}"),
    }
}

/// With the pay page up and fewer than 10 coins, the hop refuses instead of
/// answering "Yes, ok." — the engine would only then say there isn't enough.
#[test]
fn follow_alkharid_toll_refuses_without_coins_instead_of_paying() {
    let mut edge = alkharid_toll_edge();
    edge.consumed_req = vec![(TEST_TOLL_COIN_OBJ, 10)];
    let route = Route {
        dest: edge.to,
        ticks: 1.0,
        legs: vec![Leg::Transport {
            edge: Box::new(edge),
        }],
    };
    let mut c = scene_client();
    plant_loc(&mut c, ALKHARID_TOLL_LOC_ID, "Gate", "Open", 2, 1);
    let mut snap = snap_at(&mut c, 1, 1);
    assert_eq!(snap.inv_count(TEST_TOLL_COIN_OBJ), 0);
    let mut rec = FollowRec {
        route: Some((1, 1)),
        ..FollowRec::default()
    };
    let mut t = Traveller::new();
    let mut options = TravelOptions {
        close_enough: 0,
        ..TravelOptions::default()
    };
    assert!(t
        .follow(&mut rec, &snap, route.clone(), &mut options)
        .is_none());
    assert_eq!(rec.loc_ops, 1);

    plant_choice_dialog(
        &mut c,
        &[
            "No thank you, I'll walk around.",
            "Who does my money go to?",
            "Yes, ok.",
        ],
    );
    bump_rebuild(&mut c, &mut snap);
    match t.follow(&mut rec, &snap, route, &mut options) {
        Some(TravelOutcome::Blocked { detail, .. }) => {
            assert!(
                detail.contains(&TEST_TOLL_COIN_OBJ.to_string()) && detail.contains("10"),
                "refusal must name the missing coins, got {detail}"
            );
        }
        other => panic!("expected Blocked for missing coins, got {other:?}"),
    }
    assert_eq!(
        rec.if_buttons, 0,
        "the pay option is not answered without coins"
    );
}

/// An unrecognized option page is not guessed. Choice 1 would be the
/// "walk around" refuse on the real toll page.
#[test]
fn follow_alkharid_toll_does_not_answer_unknown_pages() {
    let (mut c, mut snap, mut rec, mut t, route) = alkharid_toll_follow_start(true);
    let mut options = TravelOptions {
        close_enough: 0,
        ..TravelOptions::default()
    };
    assert!(t
        .follow(&mut rec, &snap, route.clone(), &mut options)
        .is_none());
    assert_eq!(rec.loc_ops, 1);

    plant_choice_dialog(&mut c, &["An unexpected route.", "No thanks."]);
    bump_rebuild(&mut c, &mut snap);
    match t.follow(&mut rec, &snap, route, &mut options) {
        Some(TravelOutcome::Blocked { detail, .. }) => {
            assert!(
                detail.contains("unrecognized"),
                "refusal must name the unknown page, got {detail}"
            );
        }
        other => panic!("expected Blocked for unknown dialogue, got {other:?}"),
    }
    assert_eq!(rec.if_buttons, 0, "unknown pages are not answered");
}

/// Reciprocal stand→teleport Door hops (Ranging 2514) are Chebyshev 2.
/// Default `close_enough` 2 must not complete from the origin stand,
/// a forcemove wait, or an adjacent/wrong-side tile; only the packed
/// landing finishes. WalkNear's explicit 0 must stay exact-`to` too.
#[test]
fn follow_rangingguild_dir_none_completes_only_on_the_teleport_landing() {
    assert_eq!(TravelOptions::default().close_enough, 2);
    let loc = scene_of(RANGING_SCENE_BASE, RANGING_LOC);
    for (label, mut options, edge, origin, dest) in [
        (
            "default enter",
            TravelOptions::default(),
            rangingguild_enter_edge(),
            RANGING_OUTSIDE,
            RANGING_INSIDE,
        ),
        (
            "explicit0 enter",
            TravelOptions {
                close_enough: 0,
                ..TravelOptions::default()
            },
            rangingguild_enter_edge(),
            RANGING_OUTSIDE,
            RANGING_INSIDE,
        ),
        (
            "default exit",
            TravelOptions::default(),
            rangingguild_exit_edge(),
            RANGING_INSIDE,
            RANGING_OUTSIDE,
        ),
        (
            "explicit0 exit",
            TravelOptions {
                close_enough: 0,
                ..TravelOptions::default()
            },
            rangingguild_exit_edge(),
            RANGING_INSIDE,
            RANGING_OUTSIDE,
        ),
    ] {
        let mut c = scene_client();
        c.map_build_base_x = RANGING_SCENE_BASE.0;
        c.map_build_base_z = RANGING_SCENE_BASE.1;
        plant_loc(&mut c, 2514, "Guild door", "Open", loc.0, loc.1);
        let start = scene_of(RANGING_SCENE_BASE, origin);
        let mut snap = snap_at(&mut c, start.0, start.1);
        let mut rec = FollowRec {
            route: Some(start),
            build_base: Some(RANGING_SCENE_BASE),
            ..FollowRec::default()
        };
        let mut t = Traveller::new();
        let route = Route {
            legs: vec![Leg::Transport {
                edge: Box::new(edge.clone()),
            }],
            dest,
            ticks: 1.0,
        };

        follow_still_pending(
            &mut t,
            &mut rec,
            &snap,
            &route,
            &mut options,
            &format!("{label} first poll"),
        );
        assert_eq!(
            rec.loc_ops, 1,
            "{label} must send Open from the origin stand"
        );

        bump_rebuild(&mut c, &mut snap);
        follow_still_pending(
            &mut t,
            &mut rec,
            &snap,
            &route,
            &mut options,
            &format!("{label} origin wait"),
        );

        let force = scene_of(RANGING_SCENE_BASE, origin);
        plant_player(&mut c, force.0, force.1);
        bump_rebuild(&mut c, &mut snap);
        follow_still_pending(
            &mut t,
            &mut rec,
            &snap,
            &route,
            &mut options,
            &format!("{label} forcemove still on at"),
        );

        for (tile, why) in [
            (
                WorldTile {
                    x: origin.x + 1,
                    z: origin.z,
                    level: 0,
                },
                "adjacent +x",
            ),
            (
                WorldTile {
                    x: origin.x,
                    z: origin.z - 1,
                    level: 0,
                },
                "adjacent -z",
            ),
            (
                WorldTile {
                    x: dest.x,
                    z: origin.z,
                    level: 0,
                },
                "wrong-side dest x / origin z",
            ),
        ] {
            let (sx, sz) = scene_of(RANGING_SCENE_BASE, tile);
            plant_player(&mut c, sx, sz);
            bump_rebuild(&mut c, &mut snap);
            follow_still_pending(
                &mut t,
                &mut rec,
                &snap,
                &route,
                &mut options,
                &format!("{label} {why} {tile:?}"),
            );
        }

        let (dx, dz) = scene_of(RANGING_SCENE_BASE, dest);
        plant_player(&mut c, dx, dz);
        bump_rebuild(&mut c, &mut snap);
        match t.follow(&mut rec, &snap, route, &mut options) {
            Some(TravelOutcome::Arrived { at }) => {
                assert_eq!(at, dest, "{label} landing");
            }
            other => panic!("{label} expected Arrived on exact to, got {other:?}"),
        }
        assert_eq!(rec.loc_ops, 1, "{label} one Open");
    }
}

/// Sealed ranging 2514 hop with the door loc absent: a nearby unrelated
/// Close-action loc must not satisfy [`edge_loc_open`] and skip Open.
#[test]
fn follow_rangingguild_missing_loc_nearby_close_does_not_skip_open() {
    let mut c = scene_client();
    c.map_build_base_x = RANGING_SCENE_BASE.0;
    c.map_build_base_z = RANGING_SCENE_BASE.1;
    // 2514 not planted — only an unrelated swing door one tile off `at`.
    let nearby = scene_of(RANGING_SCENE_BASE, RANGING_LOC);
    plant_loc(&mut c, 1531, "Unrelated door", "Close", nearby.0, nearby.1);
    let start = scene_of(RANGING_SCENE_BASE, RANGING_OUTSIDE);
    let mut snap = snap_at(&mut c, start.0, start.1);
    let mut rec = FollowRec {
        route: Some(start),
        build_base: Some(RANGING_SCENE_BASE),
        ..FollowRec::default()
    };
    let mut t = Traveller::new();
    let route = Route {
        legs: vec![Leg::Transport {
            edge: Box::new(rangingguild_enter_edge()),
        }],
        dest: RANGING_INSIDE,
        ticks: 1.0,
    };
    let mut options = TravelOptions {
        budget_ticks_per_hop: 3,
        ..TravelOptions::default()
    };
    assert!(
        t.follow(&mut rec, &snap, route.clone(), &mut options)
            .is_none(),
        "first poll must stay pending, not walk through on a nearby Close"
    );
    assert_eq!(
        rec.loc_ops, 0,
        "must not OP_LOC1 Open when 2514 is missing (no skip-open fallback)"
    );
    assert!(
        rec.walked.is_empty(),
        "must not walk toward `to` on a nearby Close misread"
    );

    let mut outcome = None;
    for _ in 0..8 {
        if let Some(o) = t.follow(&mut rec, &snap, route.clone(), &mut options) {
            outcome = Some(o);
            break;
        }
        bump_rebuild(&mut c, &mut snap);
    }
    match outcome {
        Some(TravelOutcome::Blocked { detail, .. }) => {
            assert!(
                detail.contains("2514"),
                "blocked waiting for the packed door loc, got: {detail}"
            );
        }
        other => panic!("expected Blocked for missing 2514, got {other:?}"),
    }
    assert_eq!(
        rec.loc_ops, 0,
        "never sent Open — nearby Close is not the ranging door"
    );
}

/// Shantay north is another Door+dir=None Cheb-2 teleport. Default-2
/// must not finish at the origin loc; only (3304,3115) completes.
#[test]
fn follow_shantay_north_dir_none_does_not_complete_from_origin() {
    let mut c = scene_client();
    c.map_build_base_x = SHANTAY_NORTH_SCENE_BASE.0;
    c.map_build_base_z = SHANTAY_NORTH_SCENE_BASE.1;
    let at = scene_of(SHANTAY_NORTH_SCENE_BASE, SHANTAY_NORTH_AT);
    plant_loc(
        &mut c,
        SHANTAY_HENGE_LOC_ID,
        "Shantay pass henge doorway",
        "Go-through",
        at.0,
        at.1,
    );
    let mut snap = snap_at(&mut c, at.0, at.1);
    let mut rec = FollowRec {
        route: Some(at),
        build_base: Some(SHANTAY_NORTH_SCENE_BASE),
        ..FollowRec::default()
    };
    let mut t = Traveller::new();
    let route = Route {
        legs: vec![Leg::Transport {
            edge: Box::new(shantay_north_short_edge()),
        }],
        dest: SHANTAY_NORTH_TO,
        ticks: 1.0,
    };
    let mut options = TravelOptions::default();
    follow_still_pending(&mut t, &mut rec, &snap, &route, &mut options, "open");
    assert_eq!(rec.loc_ops, 1);
    bump_rebuild(&mut c, &mut snap);
    follow_still_pending(&mut t, &mut rec, &snap, &route, &mut options, "origin");
    let adj = scene_of(
        SHANTAY_NORTH_SCENE_BASE,
        WorldTile {
            x: 3303,
            z: 3116,
            level: 0,
        },
    );
    plant_player(&mut c, adj.0, adj.1);
    bump_rebuild(&mut c, &mut snap);
    follow_still_pending(
        &mut t,
        &mut rec,
        &snap,
        &route,
        &mut options,
        "adjacent origin",
    );
    let to = scene_of(SHANTAY_NORTH_SCENE_BASE, SHANTAY_NORTH_TO);
    plant_player(&mut c, to.0, to.1);
    bump_rebuild(&mut c, &mut snap);
    match t.follow(&mut rec, &snap, route, &mut options) {
        Some(TravelOutcome::Arrived { at }) => assert_eq!(at, SHANTAY_NORTH_TO),
        other => panic!("expected Arrived at north landing, got {other:?}"),
    }
}

/// Far Door+dir=None teleports (Zanaris / wilderness levers) must keep
/// runner close_enough: a Cheb-1 landing still completes under default 2.
#[test]
fn follow_far_dir_none_door_keeps_close_enough_tolerance() {
    let mut c = scene_client();
    plant_loc(&mut c, 2409, "Door", "Open", 1, 1);
    let mut snap = snap_at(&mut c, 1, 1);
    let mut rec = FollowRec {
        route: Some((1, 1)),
        ..FollowRec::default()
    };
    let dest = WorldTile {
        x: 3200,
        z: 3300,
        level: 0,
    };
    let edge = TransportEdge {
        takeoff: None,
        worn_all_req: Vec::new(),
        kind: TransportKind::Door,
        player_delta: None,
        at: WorldTile {
            x: 3201,
            z: 3201,
            level: 0,
        },
        to: dest,
        loc_id: 2409,
        option: 1,
        ticks: 4,
        dir: None,
        open_loc_id: None,
        skill_req: vec![],
        item_req: vec![],
        consumed_req: vec![],
        item_returns: vec![],
        quest_req: vec![],
        varp_req: vec![],
        worn_req: vec![],
        members_req: false,
        wildy_cap: None,
        quest_gates: None,
    };
    let mut t = Traveller::new();
    let route = Route {
        legs: vec![Leg::Transport {
            edge: Box::new(edge),
        }],
        dest,
        ticks: 1.0,
    };
    let mut options = TravelOptions::default();
    follow_still_pending(
        &mut t,
        &mut rec,
        &snap,
        &route,
        &mut options,
        "zanaris-style open",
    );
    assert_eq!(rec.loc_ops, 1);
    plant_player(&mut c, 1, 100);
    bump_rebuild(&mut c, &mut snap);
    match t.follow(&mut rec, &snap, route, &mut options) {
        Some(TravelOutcome::Arrived { at }) => {
            assert_eq!(
                at,
                WorldTile {
                    x: 3201,
                    z: 3300,
                    level: 0
                }
            );
        }
        other => panic!("far dir=None must accept close_enough 2, got {other:?}"),
    }
}

#[test]
fn follow_reverse_straight_door_waits_for_crossing_then_arrives_on_loc_tile() {
    let mut c = scene_client();
    plant_door(&mut c, false, 1);
    let mut snap = snap_at(&mut c, 2, 0);
    let mut rec = FollowRec {
        route: Some((2, 0)),
        ..FollowRec::default()
    };
    let mut edge = door_edge();
    edge.to = edge.at;
    edge.dir = Some(DoorDir::W);
    edge.open_loc_id = Some(1531);
    let dest = edge.to;
    let route = Route {
        legs: vec![Leg::Transport {
            edge: Box::new(edge),
        }],
        dest,
        ticks: 1.0,
    };
    let mut traveller = Traveller::new();
    let mut options = TravelOptions::default();
    follow_still_pending(
        &mut traveller,
        &mut rec,
        &snap,
        &route,
        &mut options,
        "reverse Open",
    );
    assert_eq!(rec.loc_ops, 1);
    bump_rebuild(&mut c, &mut snap);
    assert!(
        traveller
            .follow(&mut rec, &snap, route.clone(), &mut options)
            .is_none(),
        "adjacency on the pocket side must not count as crossing"
    );
    plant_player(&mut c, 1, 0);
    bump_rebuild(&mut c, &mut snap);
    assert!(
        matches!(
            traveller.follow(&mut rec, &snap, route, &mut options),
            Some(TravelOutcome::Arrived { at }) if at == dest
        ),
        "reverse crossing lands on the original loc tile"
    );
}

/// Cardinal doors keep tolerant dest-adjacent completion under default 2.
#[test]
fn follow_cardinal_door_still_arrives_adjacent_to_to_under_default_close_enough() {
    let mut c = scene_client();
    plant_door(&mut c, false, 1);
    let mut snap = snap_at(&mut c, 0, 0);
    let mut rec = FollowRec {
        route: Some((0, 0)),
        ..FollowRec::default()
    };
    let mut edge = door_edge();
    edge.dir = Some(DoorDir::E);
    edge.open_loc_id = Some(1531);
    let dest = edge.to;
    let mut t = Traveller::new();
    let route = Route {
        legs: vec![Leg::Transport {
            edge: Box::new(edge),
        }],
        dest,
        ticks: 1.0,
    };
    let mut options = TravelOptions::default();
    follow_still_pending(
        &mut t,
        &mut rec,
        &snap,
        &route,
        &mut options,
        "cardinal open",
    );
    assert_eq!(rec.loc_ops, 1);
    plant_player(&mut c, 3, 1);
    bump_rebuild(&mut c, &mut snap);
    match t.follow(&mut rec, &snap, route, &mut options) {
        Some(TravelOutcome::Arrived { at }) => {
            assert_eq!(
                at,
                WorldTile {
                    x: 3203,
                    z: 3201,
                    level: 0
                }
            );
        }
        other => {
            panic!("cardinal Door must keep tolerant dest-adjacent arrival, got {other:?}")
        }
    }
}

#[test]
fn follow_npc_edge_answers_choice_one_not_the_op_index() {
    // An Npc hop's `option` is the NPC op index (essence's opnpc3
    // teleport; the cart drivers' Talk-to op 1) — never the dialog
    // choice. A hop whose op is 3 but whose dialog puts the ride
    // first must answer the FIRST choice, and the fake must record
    // the op-3 interact as an npc op. (Regressions for the
    // `edge.option` conflation and the OP_NPC1-only matcher.)
    let mut c = scene_client();
    plant_wizard_npc(&mut c, 7, 1, 1);
    let mut snap = snap_at(&mut c, 1, 2);
    let mut rec = FollowRec {
        route: Some((1, 2)),
        ..FollowRec::default()
    };
    let mut t = Traveller::new();
    let edge = TransportEdge {
        takeoff: None,
        worn_all_req: Vec::new(),
        option: 3, // essence-style op index, not a dialog choice
        ..cart_edge()
    };
    let route = Route {
        legs: vec![Leg::Transport {
            edge: Box::new(edge),
        }],
        dest: WorldTile {
            x: 3300,
            z: 3200,
            level: 0,
        },
        ticks: 1.0,
    };
    let mut options = TravelOptions::default();
    // Poll 1: the hop sends the OP_NPC3 interact.
    assert!(t
        .follow(&mut rec, &snap, route.clone(), &mut options)
        .is_none());
    assert_eq!(
        rec.npc_ops, 1,
        "an op-3 interact counts as an npc op (matcher covers OP_NPC2..=5)"
    );
    assert_eq!(rec.if_buttons, 0, "no dialog answer before it opens");

    // The driver opens a two-choice dialog: the hop answers the FIRST
    // choice (component 101), never the op index (there is no choice
    // 3 on the page).
    plant_choice_dialog(&mut c, &["Yes please.", "Not now, thanks."]);
    bump_rebuild(&mut c, &mut snap);
    assert!(t
        .follow(&mut rec, &snap, route.clone(), &mut options)
        .is_none());
    assert_eq!(rec.if_buttons, 1, "the ride choice is pressed");
    assert_eq!(
        rec.if_button_components,
        vec![101],
        "the first chat option is the ride, independent of the op index"
    );

    // The hop carries the player over: the run arrives.
    plant_player(&mut c, 100, 0);
    bump_rebuild(&mut c, &mut snap);
    assert!(matches!(
        t.follow(&mut rec, &snap, route.clone(), &mut options),
        Some(TravelOutcome::Arrived { at })
            if at == WorldTile {
                x: 3300,
                z: 3200,
                level: 0
            }
    ));
}

#[test]
fn follow_essence_entry_latches_the_session_on_arrival() {
    // A wizard entry hop (`opnpc4` on Aubury) that teleports the
    // player into a random mine tile: the hop completes on any tile
    // inside the enclosed mine (never the pad exactly), and the
    // traveller records the wizard so the mine exit loc can only
    // return to him.
    let mut c = scene_client();
    plant_npc_ops(
        &mut c,
        553,
        1,
        1,
        "Aubury",
        &["Talk-to", "Talk-to", "Talk-to", "Teleport"],
    );
    let mut snap = snap_at(&mut c, 1, 2);
    let mut rec = FollowRec {
        route: Some((1, 2)),
        ..FollowRec::default()
    };
    let mut t = Traveller::new();
    let edge = TransportEdge {
        takeoff: None,
        worn_all_req: Vec::new(),
        kind: TransportKind::Npc,
        player_delta: None,
        at: WorldTile {
            x: 3201,
            z: 3201,
            level: 0,
        },
        to: WorldTile {
            x: 2912,
            z: 4833,
            level: 0, // the mine pad
        },
        loc_id: 553,
        option: 4, // Aubury's `[opnpc4,aubury]`
        ticks: 5,  // OP_BASE + the portal p_delay(4)
        ..cart_edge()
    };
    let route = Route {
        legs: vec![Leg::Transport {
            edge: Box::new(edge),
        }],
        dest: WorldTile {
            x: 2912,
            z: 4833,
            level: 0,
        },
        ticks: 5.0,
    };
    let mut options = TravelOptions::default();
    assert!(t.essence().is_none(), "no session before the entry");
    // Poll 1: the hop sends the OP_NPC4 interact.
    assert!(t
        .follow(&mut rec, &snap, route.clone(), &mut options)
        .is_none());
    assert_eq!(rec.npc_ops, 1, "one OP_NPC4 interact sent");
    // The wizard teleports the player into the mine — a tile off the
    // pad, still inside the enclosure: the hop completes.
    plant_player(&mut c, -288, 1633); // world (2912, 4833)
    bump_rebuild(&mut c, &mut snap);
    assert!(matches!(
        t.follow(&mut rec, &snap, route.clone(), &mut options),
        Some(TravelOutcome::Arrived { at })
            if at == WorldTile {
                x: 2912,
                z: 4833,
                level: 0
            }
    ));
    let session = t.essence().expect("the entry latches the session");
    assert_eq!(session.wizard_npc, 553);
    assert_eq!(
        session.return_tile,
        WorldTile {
            x: 3253,
            z: 3401,
            level: 0
        },
        "the exit portal may only return to Aubury's anchor"
    );
}

#[test]
fn follow_essence_entry_accepts_any_mine_landing() {
    // The entry settle must not demand the pad exactly: the live
    // teleport lands at a random `essence_mine_teleports` coord, so a
    // landing several tiles off the pad still completes the hop.
    let mut c = scene_client();
    plant_npc_ops(
        &mut c,
        553,
        1,
        1,
        "Aubury",
        &["Talk-to", "Talk-to", "Talk-to", "Teleport"],
    );
    let mut snap = snap_at(&mut c, 1, 2);
    let mut rec = FollowRec {
        route: Some((1, 2)),
        ..FollowRec::default()
    };
    let mut t = Traveller::new();
    let edge = TransportEdge {
        takeoff: None,
        worn_all_req: Vec::new(),
        kind: TransportKind::Npc,
        player_delta: None,
        at: WorldTile {
            x: 3201,
            z: 3201,
            level: 0,
        },
        to: WorldTile {
            x: 2912,
            z: 4833,
            level: 0, // the mine pad
        },
        loc_id: 553,
        option: 4,
        ticks: 5,
        ..cart_edge()
    };
    let route = Route {
        legs: vec![Leg::Transport {
            edge: Box::new(edge),
        }],
        dest: WorldTile {
            x: 2912,
            z: 4833,
            level: 0,
        },
        ticks: 5.0,
    };
    let mut options = TravelOptions::default();
    assert!(t
        .follow(&mut rec, &snap, route.clone(), &mut options)
        .is_none());
    // A far landing (m45_75 local (55,46) → (2935, 4846)), 23 tiles
    // from the pad: still inside the enclosed mine, still arrived.
    plant_player(&mut c, -265, 1646); // world (2935, 4846)
    bump_rebuild(&mut c, &mut snap);
    assert!(matches!(
        t.follow(&mut rec, &snap, route.clone(), &mut options),
        Some(TravelOutcome::Arrived { at })
            if at == WorldTile {
                x: 2935,
                z: 4846,
                level: 0
            }
    ));
    assert!(
        t.essence().is_some(),
        "any mine landing latches the session"
    );
}

#[test]
fn follow_essence_exit_arrives_within_the_landing_radius() {
    // The exit portal (`oploc1`, loc 2492) teleports to a random tile
    // within chebyshev 2 of the entry wizard's anchor: the hop accepts
    // any landing in that radius, never an exact tile, and never
    // re-latches the session (only the entry does).
    let mut c = scene_client();
    plant_loc(&mut c, 2492, "Portal", "Enter", 1, 1);
    let mut snap = snap_at(&mut c, 1, 1);
    let mut rec = FollowRec {
        route: Some((1, 1)),
        ..FollowRec::default()
    };
    let mut t = Traveller::new();
    let edge = TransportEdge {
        takeoff: None,
        worn_all_req: Vec::new(),
        kind: TransportKind::EssenceExit,
        player_delta: None,
        at: WorldTile {
            x: 3201,
            z: 3201,
            level: 0,
        },
        to: WorldTile {
            x: 3200,
            z: 3205,
            level: 0, // the wizard's anchor
        },
        loc_id: 2492,
        option: 1,
        ticks: 2,
        ..cart_edge()
    };
    let route = Route {
        legs: vec![Leg::Transport {
            edge: Box::new(edge),
        }],
        dest: WorldTile {
            x: 3200,
            z: 3205,
            level: 0,
        },
        ticks: 2.0,
    };
    let mut options = TravelOptions::default();
    // Poll 1: the hop sends the OP_LOC1 on the portal.
    assert!(t
        .follow(&mut rec, &snap, route.clone(), &mut options)
        .is_none());
    assert_eq!(rec.loc_ops, 1, "one OP_LOC1 on the exit portal");
    assert_eq!(rec.npc_ops, 0, "the exit is a loc, never an npc op");
    // The portal drops the player a tile off the anchor: still arrived.
    plant_player(&mut c, 1, 5); // world (3201, 3205), cheb 1 from `to`
    bump_rebuild(&mut c, &mut snap);
    match t.follow(&mut rec, &snap, route.clone(), &mut options) {
        Some(TravelOutcome::Arrived { at }) => {
            assert_eq!(
                at,
                WorldTile {
                    x: 3201,
                    z: 3205,
                    level: 0
                }
            );
        }
        other => panic!("expected Arrived, got {other:?}"),
    }
    assert_eq!(t.essence(), None, "an exit hop never latches the session");
}

// --- Task 5: packed Teleport execute (never the WalkTo `::tele` cheat) ---

/// A packed jewellery Teleport edge: the charged dueling ring (obj
/// 2552, `opheld4` Rub) carries the player to the Al Kharid Duel
/// Arena. `at` is the any-tile placeholder — never indexed.
fn ring_edge() -> TransportEdge {
    TransportEdge {
        takeoff: None,
        worn_all_req: Vec::new(),
        kind: TransportKind::Teleport,
        player_delta: None,
        at: WorldTile {
            x: 0,
            z: 0,
            level: 0,
        },
        to: WorldTile {
            x: 3315,
            z: 3235,
            level: 0,
        },
        loc_id: 2552,
        option: 4, // Rub (opheld4)
        ticks: 2,  // OP_BASE + the rub p_delay(1)
        dir: None,
        open_loc_id: None,
        skill_req: vec![],
        item_req: vec![],
        consumed_req: vec![(2552, 1)],
        item_returns: vec![(2551, 1)],
        quest_req: vec![],
        varp_req: vec![],
        worn_req: vec![],
        members_req: false,
        wildy_cap: None,
        quest_gates: None,
    }
}

/// A packed glory-style jewellery edge (obj 1712, `opheld4` Rub): the
/// shape every dest of the four-location glory group shares. The
/// `to` names the landing (default Edgeville, `switch_int($choice)`
/// case 1); the group's sibling edges share `loc_id` + option and
/// differ only in `to`, exactly as the bake emits them.
fn glory_edge() -> TransportEdge {
    TransportEdge {
        takeoff: None,
        worn_all_req: Vec::new(),
        kind: TransportKind::Teleport,
        player_delta: None,
        at: WorldTile {
            x: 0,
            z: 0,
            level: 0,
        },
        to: WorldTile {
            x: 3087,
            z: 3496,
            level: 0, // Edgeville (case 1)
        },
        loc_id: 1712,
        option: 4, // Rub (opheld4)
        ticks: 2,  // OP_BASE + the rub p_delay(1)
        dir: None,
        open_loc_id: None,
        skill_req: vec![],
        item_req: vec![],
        consumed_req: vec![(1712, 1)],
        item_returns: vec![(1711, 1)],
        quest_req: vec![],
        varp_req: vec![],
        worn_req: vec![],
        members_req: false,
        wildy_cap: None,
        quest_gates: None,
    }
}

/// A packed spell Teleport edge: Varrock (Magic 25, fire/air/law
/// runes). `loc_id` 0 = "a spell button, not a loc/obj use"; the
/// traveller resolves the spellbook button from the landing tile.
fn varrock_spell_edge() -> TransportEdge {
    TransportEdge {
        takeoff: None,
        worn_all_req: Vec::new(),
        kind: TransportKind::Teleport,
        player_delta: None,
        at: WorldTile {
            x: 0,
            z: 0,
            level: 0,
        },
        to: WorldTile {
            x: 3213,
            z: 3424,
            level: 0,
        },
        loc_id: 0, // a spell button, not a loc/obj use
        option: 0,
        ticks: 3, // OP_BASE + the cast p_delay(2)
        dir: None,
        open_loc_id: None,
        skill_req: vec![(6, 25)],
        item_req: vec![],
        consumed_req: vec![(554, 1), (556, 3), (563, 1)],
        item_returns: vec![],
        quest_req: vec![],
        varp_req: vec![],
        worn_req: vec![],
        members_req: false,
        wildy_cap: None,
        quest_gates: None,
    }
}

/// A packed spell Teleport edge: Lumbridge (Magic 31, earth/air/law
/// runes). `loc_id` 0 = "a spell button, not a loc/obj use"; the
/// traveller resolves the spellbook button from the landing tile.
fn lumbridge_spell_edge() -> TransportEdge {
    TransportEdge {
        takeoff: None,
        worn_all_req: Vec::new(),
        kind: TransportKind::Teleport,
        player_delta: None,
        at: WorldTile {
            x: 0,
            z: 0,
            level: 0,
        },
        to: WorldTile {
            x: 3221,
            z: 3218,
            level: 0,
        },
        loc_id: 0, // a spell button, not a loc/obj use
        option: 0,
        ticks: 3, // OP_BASE + the cast p_delay(2)
        dir: None,
        open_loc_id: None,
        skill_req: vec![(6, 31)],
        item_req: vec![],
        consumed_req: vec![(557, 1), (556, 3), (563, 1)],
        item_returns: vec![],
        quest_req: vec![],
        varp_req: vec![],
        worn_req: vec![],
        members_req: false,
        wildy_cap: None,
        quest_gates: None,
    }
}

/// The inventory tab (side 3) with a TYPE_INV child carrying one
/// charged obj `obj_id` (stored `obj_id + 1`), whose def offers the
/// Rub op in slot 4: the container the jewellery-rub arm reads the
/// packed item from.
fn plant_inv_item(c: &mut Client, obj_id: i32) {
    plant_inv_stack(c, obj_id, 1);
}

/// Inventory tab (side 3) carrying `count` of `obj_id` (stored `obj_id + 1`).
fn plant_inv_stack(c: &mut Client, obj_id: i32, count: i32) {
    {
        let cache = Arc::get_mut(&mut c.cache).expect("sole cache owner");
        while cache.objs.len() <= obj_id as usize {
            cache.objs.push(ObjType::default());
        }
        cache.objs[obj_id as usize] = ObjType {
            id: obj_id,
            iop: [None, None, None, Some("Rub".into()), None],
            ..Default::default()
        };
    }
    c.side_icon[3] = 300;
    c.set_iface(
        300,
        IfType {
            id: 300,
            layer_id: 300,
            children: Some(vec![301]),
            ..Default::default()
        },
    );
    c.set_iface(
        301,
        IfType {
            id: 301,
            layer_id: 300,
            r#type: ComponentType::TYPE_INV,
            obj_ops: true,
            ..Default::default()
        },
    );
    c.set_iface_mut(
        301,
        IfTypeMut {
            link_obj_type: Some(vec![obj_id + 1]),
            link_obj_number: Some(vec![count]),
            ..Default::default()
        },
    );
}

fn clear_inventory_item(c: &mut Client) {
    c.set_iface_mut(
        301,
        IfTypeMut {
            link_obj_type: Some(vec![0]),
            link_obj_number: Some(vec![0]),
            ..Default::default()
        },
    );
}

fn plant_equipment_item(c: &mut Client, obj_id: i32) {
    {
        let cache = Arc::get_mut(&mut c.cache).expect("sole cache owner");
        while cache.objs.len() <= obj_id as usize {
            cache.objs.push(ObjType::default());
        }
        cache.objs[obj_id as usize] = ObjType {
            id: obj_id,
            ..Default::default()
        };
    }
    c.side_icon[4] = 400;
    c.set_iface(
        400,
        IfType {
            id: 400,
            layer_id: 400,
            children: Some(vec![401]),
            ..Default::default()
        },
    );
    c.set_iface(
        401,
        IfType {
            id: 401,
            layer_id: 400,
            r#type: ComponentType::TYPE_INV,
            ..Default::default()
        },
    );
    c.set_iface_mut(
        401,
        IfTypeMut {
            link_obj_type: Some(vec![obj_id + 1]),
            link_obj_number: Some(vec![1]),
            ..Default::default()
        },
    );
}

fn clear_equipment_item(c: &mut Client) {
    c.set_iface_mut(
        401,
        IfTypeMut {
            link_obj_type: Some(vec![0]),
            link_obj_number: Some(vec![0]),
            ..Default::default()
        },
    );
}

/// Same charged obj as [`plant_inv_item`], but side tab 3 stays unbound
/// (`side_icon[3] == -1`). `rebuild_inv` still finds the TYPE_INV via its
/// first-with-items fallback (so `WorldState` / `Proof::Item` pass); the
/// jewellery arm reads `inventory()`, which needs the bound tab.
fn plant_inv_item_unbound_tab(c: &mut Client, obj_id: i32) {
    plant_inv_item(c, obj_id);
    c.side_icon[3] = -1;
}

/// The magic tab (side 6) with the Lumbridge spellbook button: the
/// live button the spell arm presses when the loaded tree carries the
/// 2004 button text.
fn plant_spell_button(c: &mut Client) {
    c.side_icon[6] = 500;
    c.set_iface(
        500,
        IfType {
            id: 500,
            layer_id: 500,
            children: Some(vec![501]),
            ..Default::default()
        },
    );
    c.set_iface(
        501,
        IfType {
            id: 501,
            layer_id: 500,
            button_text: "Cast @gre@Lumbridge teleport".into(),
            ..Default::default()
        },
    );
    c.set_iface_mut(
        501,
        IfTypeMut {
            button_type: ButtonType::BUTTON_OK,
            ..Default::default()
        },
    );
}

#[test]
fn follow_jewellery_teleport_rubs_the_packed_item_and_arrives() {
    // A `TransportKind::Teleport` edge with a charged obj id and the
    // `opheld4` Rub op: the hop must interact the inventory item (an
    // OP_HELD4 press), answer the destination choice the rub opens
    // (the dueling ring asks "Where would you like to teleport to?"
    // with the arena first and "Nowhere." last), and settle the
    // landing within the packed `to`'s scatter radius — never the
    // WalkTo `::tele` cheat.
    let mut c = scene_client();
    plant_inv_item(&mut c, 2552);
    let mut snap = snap_at(&mut c, 0, 0);
    let mut rec = FollowRec {
        route: Some((0, 0)),
        ..FollowRec::default()
    };
    let mut t = Traveller::new();
    let route = Route {
        legs: vec![Leg::Transport {
            edge: Box::new(ring_edge()),
        }],
        dest: WorldTile {
            x: 3315,
            z: 3235,
            level: 0,
        },
        ticks: 2.0,
    };
    // The packed single-dest rub group: the ring's only sibling edge,
    // so the derived dialog choice is 1 (the arena).
    let mut options = TravelOptions {
        teleports: Some(&[ring_edge()]),
        ..TravelOptions::default()
    };
    // Poll 1: the hop interacts the packed item — the OP_HELD4 Rub.
    assert!(t
        .follow(&mut rec, &snap, route.clone(), &mut options)
        .is_none());
    assert_eq!(rec.held_ops, 1, "one OP_HELD4 rub sent");
    assert_eq!(rec.loc_ops, 0, "a teleport edge never sends OP_LOC1");
    assert_eq!(rec.npc_ops, 0, "a teleport edge never sends OP_NPC");
    assert!(
        rec.sink.strings.is_empty(),
        "the packed op is never the ::tele cheat"
    );
    // The rub opens the destination choice: answer the first (the
    // arena), exactly once.
    plant_choice_dialog(&mut c, &["Al Kharid Duel Arena.", "Nowhere."]);
    bump_rebuild(&mut c, &mut snap);
    assert!(t
        .follow(&mut rec, &snap, route.clone(), &mut options)
        .is_none());
    assert_eq!(rec.if_buttons, 1, "the destination choice is answered");
    bump_rebuild(&mut c, &mut snap);
    assert!(t
        .follow(&mut rec, &snap, route.clone(), &mut options)
        .is_none());
    assert_eq!(rec.if_buttons, 1, "the choice is never re-pressed");
    // The ring lands the player a tile off the packed landing (the
    // `map_findsquare` scatter): the hop accepts the radius.
    plant_player(&mut c, 113, 35); // world (3313, 3235), cheb 2 from `to`
    bump_rebuild(&mut c, &mut snap);
    match t.follow(&mut rec, &snap, route.clone(), &mut options) {
        Some(TravelOutcome::Arrived { at }) => {
            assert_eq!(
                at,
                WorldTile {
                    x: 3313,
                    z: 3235,
                    level: 0
                }
            );
        }
        other => panic!("expected Arrived, got {other:?}"),
    }
    assert_eq!(rec.held_ops, 1, "one rub total");
    assert_eq!(rec.loc_ops, 0);
    assert!(rec.sink.strings.is_empty());
}

#[test]
fn follow_jewellery_teleport_waits_when_the_inv_tab_is_unbound() {
    // Live `nav_tele` after mainlandAccount: `inv()` sees the cheated
    // ring (TYPE_INV fallback) so the router packs the rub, but
    // `inventory()` is empty until a Relog binds side tab 3. The hop
    // must Wait, never OP_HELD, and name the unbound charged-item control
    // when the bounded wait ends.
    let mut c = scene_client();
    plant_inv_item_unbound_tab(&mut c, 2552);
    let mut snap = snap_at(&mut c, 0, 0);
    assert!(
        snap.inv().iter().any(|&(id, n)| id == 2552 && n >= 1),
        "inv() fallback still carries the ring"
    );
    assert!(
        snap.inventory().is_empty(),
        "inventory() stays empty while tab 3 is unbound"
    );
    let mut rec = FollowRec {
        route: Some((0, 0)),
        ..FollowRec::default()
    };
    let mut t = Traveller::new();
    let route = Route {
        legs: vec![Leg::Transport {
            edge: Box::new(ring_edge()),
        }],
        dest: WorldTile {
            x: 3315,
            z: 3235,
            level: 0,
        },
        ticks: 2.0,
    };
    let mut options = TravelOptions {
        teleports: Some(&[ring_edge()]),
        budget_ticks_per_hop: 2,
        ..TravelOptions::default()
    };
    assert!(t
        .follow(&mut rec, &snap, route.clone(), &mut options)
        .is_none());
    bump_rebuild(&mut c, &mut snap);
    assert!(t
        .follow(&mut rec, &snap, route.clone(), &mut options)
        .is_none());
    bump_rebuild(&mut c, &mut snap);
    match t.follow(&mut rec, &snap, route, &mut options) {
        Some(TravelOutcome::Blocked { at, detail, .. }) => {
            assert_eq!(
                at,
                WorldTile {
                    x: 3200,
                    z: 3200,
                    level: 0
                }
            );
            assert!(
                detail.contains("charged teleport item 2552")
                    && detail.contains("inventory control"),
                "blocked detail: {detail}"
            );
        }
        other => panic!("expected Blocked after Wait budget, got {other:?}"),
    }
    assert_eq!(rec.held_ops, 0, "an unbound inv tab never sends OP_HELD");
    assert!(rec.sink.strings.is_empty());
}

#[test]
fn follow_jewellery_teleport_answers_the_second_dest_choice() {
    // A packed multi-destination jewellery rub (the glory's four
    // locations share one opheld4 op, differing only in `to`, in the
    // script's `switch_int($choice)` order): executing the SECOND
    // landing must answer dialog choice 2 — never the constant first
    // choice (which would teleport to Edgeville). The choice is the
    // 1-based index of the edge's `to` among the packed same-`loc_id`
    // rub edges.
    let mut c = scene_client();
    plant_inv_item(&mut c, 1712);
    let mut snap = snap_at(&mut c, 0, 0);
    let mut rec = FollowRec {
        route: Some((0, 0)),
        ..FollowRec::default()
    };
    let mut t = Traveller::new();
    let karamja = WorldTile {
        x: 2918,
        z: 3176,
        level: 0, // `0_45_49_38_40` (case 2)
    };
    let glory = [
        TransportEdge {
            takeoff: None,
            worn_all_req: Vec::new(),
            to: WorldTile {
                x: 3087,
                z: 3496,
                level: 0,
            }, // Edgeville (case 1)
            ..glory_edge()
        },
        TransportEdge {
            takeoff: None,
            worn_all_req: Vec::new(),
            to: karamja,
            ..glory_edge()
        },
    ];
    let route = Route {
        legs: vec![Leg::Transport {
            edge: Box::new(glory[1].clone()),
        }],
        dest: karamja,
        ticks: 2.0,
    };
    let mut options = TravelOptions {
        teleports: Some(&glory),
        ..TravelOptions::default()
    };
    // Poll 1: the hop interacts the packed item — the OP_HELD4 Rub.
    assert!(t
        .follow(&mut rec, &snap, route.clone(), &mut options)
        .is_none());
    assert_eq!(rec.held_ops, 1, "one OP_HELD4 rub sent");
    // The rub opens the four-location destination choice: the hop
    // presses the SECOND option (Karamja), exactly once.
    plant_choice_dialog(
        &mut c,
        &[
            "Edgeville.",
            "Karamja.",
            "Draynor Village.",
            "Al Kharid.",
            "Nowhere.",
        ],
    );
    bump_rebuild(&mut c, &mut snap);
    assert!(t
        .follow(&mut rec, &snap, route.clone(), &mut options)
        .is_none());
    assert_eq!(
        rec.if_button_components,
        vec![102],
        "the second destination answers choice 2, not 1"
    );
    bump_rebuild(&mut c, &mut snap);
    assert!(t
        .follow(&mut rec, &snap, route.clone(), &mut options)
        .is_none());
    assert_eq!(rec.if_buttons, 1, "the choice is never re-pressed");
    // The glory lands the player at the packed Karamja landing.
    plant_player(&mut c, -282, -24); // world (2918, 3176)
    bump_rebuild(&mut c, &mut snap);
    match t.follow(&mut rec, &snap, route.clone(), &mut options) {
        Some(TravelOutcome::Arrived { at }) => {
            assert_eq!(
                at,
                WorldTile {
                    x: 2918,
                    z: 3176,
                    level: 0
                }
            );
        }
        other => panic!("expected Arrived, got {other:?}"),
    }
    assert_eq!(rec.held_ops, 1, "one rub total");
    assert_eq!(rec.loc_ops, 0);
    assert!(rec.sink.strings.is_empty());
}

#[test]
fn follow_spell_teleport_presses_the_spellbook_button_and_arrives() {
    // A `TransportKind::Teleport` spell edge names no widget on the
    // wire (`loc_id` 0): the hop resolves the standard spell the
    // packed landing identifies and presses its magic-tab button — the
    // 2004 component id when the loaded scene carries no spellbook
    // text — never `::tele`.
    let mut c = scene_client();
    n1_tests::spell_supply(&mut c, &varrock_spell_edge());
    let mut snap = snap_at(&mut c, 0, 0);
    let mut rec = FollowRec {
        route: Some((0, 0)),
        ..FollowRec::default()
    };
    let mut t = Traveller::new();
    let route = Route {
        legs: vec![Leg::Transport {
            edge: Box::new(varrock_spell_edge()),
        }],
        dest: WorldTile {
            x: 3213,
            z: 3424,
            level: 0,
        },
        ticks: 3.0,
    };
    let mut options = TravelOptions::default();
    assert!(t
        .follow(&mut rec, &snap, route.clone(), &mut options)
        .is_none());
    assert_eq!(
        rec.if_button_components,
        vec![1164],
        "the fallback 2004 Varrock spellbook button is pressed"
    );
    assert_eq!(rec.held_ops, 0, "a spell is a button, never a held op");
    assert!(
        rec.sink.strings.is_empty(),
        "the spell cast is never the ::tele cheat"
    );
    // The spell lands the player at the packed landing: the hop
    // arrives within the scatter radius.
    plant_player(&mut c, 13, 224); // world (3213, 3424)
    bump_rebuild(&mut c, &mut snap);
    assert!(matches!(
        t.follow(&mut rec, &snap, route.clone(), &mut options),
        Some(TravelOutcome::Arrived { at })
            if at == WorldTile {
                x: 3213,
                z: 3424,
                level: 0
            }
    ));
    assert!(rec.sink.strings.is_empty());
}

#[test]
fn follow_refused_spell_teleport_does_not_resend() {
    // The server refuses a wilderness teleport with a mes, not "I can't
    // reach that", so the hop never matches an arm. The traveller must
    // press the spell once, wait out the hop budget, and Stall — never
    // recast. A later find from the same tile cannot pick that teleport
    // again (the packed cap is the same gate the server applied).
    let mut c = scene_client();
    n1_tests::spell_supply(&mut c, &varrock_spell_edge());
    let mut snap = snap_at(&mut c, 0, 0);
    let mut rec = FollowRec {
        route: Some((0, 0)),
        ..FollowRec::default()
    };
    let mut t = Traveller::new();
    let dest = WorldTile {
        x: 3213,
        z: 3424,
        level: 0,
    };
    let route = Route {
        legs: vec![Leg::Transport {
            edge: Box::new(varrock_spell_edge()),
        }],
        dest,
        ticks: 3.0,
    };
    let mut options = TravelOptions {
        budget_ticks_per_hop: 3,
        ..TravelOptions::default()
    };
    let outcome = drive(
        &mut t,
        &mut rec,
        &mut c,
        &mut snap,
        &route,
        &mut options,
        |_| {},
    );
    assert!(
        matches!(
            outcome,
            TravelOutcome::Stalled {
                aiming,
                why: HopFailure::Dropped,
                tries: 1,
                ..
            } if aiming == dest
        ),
        "{outcome:?}"
    );
    assert_eq!(
        rec.if_button_components,
        vec![1164],
        "the spell is pressed exactly once; a budget lapse must not recast"
    );
    assert_eq!(rec.held_ops, 0, "a spell is a button, never a held op");
    assert!(
        rec.sink.strings.is_empty(),
        "the spell cast is never the ::tele cheat"
    );
}

#[test]
fn follow_spell_teleport_presses_the_live_spellbook_button_by_text() {
    // When the magic tab's tree carries the 2004 spellbook button
    // text, the hop presses the live button (a gated IF_BUTTON)
    // instead of the baked fallback id. The planted tree spells
    // Lumbridge, so the edge must be the Lumbridge standard spell
    // (the `widget_search` match keys on the landing's dest word).
    let mut c = scene_client();
    n1_tests::spell_supply(&mut c, &lumbridge_spell_edge());
    plant_spell_button(&mut c);
    let mut snap = snap_at(&mut c, 0, 0);
    let mut rec = FollowRec {
        route: Some((0, 0)),
        ..FollowRec::default()
    };
    let mut t = Traveller::new();
    let route = Route {
        legs: vec![Leg::Transport {
            edge: Box::new(lumbridge_spell_edge()),
        }],
        dest: WorldTile {
            x: 3221,
            z: 3218,
            level: 0,
        },
        ticks: 3.0,
    };
    let mut options = TravelOptions::default();
    assert!(t
        .follow(&mut rec, &snap, route.clone(), &mut options)
        .is_none());
    assert_eq!(
        rec.if_button_components,
        vec![501],
        "the live spellbook button is pressed by text"
    );
    assert!(rec.sink.strings.is_empty());
    plant_player(&mut c, 21, 18); // world (3221, 3218)
    bump_rebuild(&mut c, &mut snap);
    assert!(matches!(
        t.follow(&mut rec, &snap, route.clone(), &mut options),
        Some(TravelOutcome::Arrived { .. })
    ));
}

#[test]
fn follow_essence_entry_does_not_latch_for_a_cart_driver() {
    // A non-wizard Npc hop (the cart driver) arrives like always but
    // records no session: only the essence-mine wizards latch.
    let mut c = scene_client();
    plant_driver_npc(&mut c, 7, 1, 1);
    let mut snap = snap_at(&mut c, 1, 2);
    let mut rec = FollowRec {
        route: Some((1, 2)),
        ..FollowRec::default()
    };
    let mut t = Traveller::new();
    let route = Route {
        legs: vec![Leg::Transport {
            edge: Box::new(cart_edge()),
        }],
        dest: WorldTile {
            x: 3300,
            z: 3200,
            level: 0,
        },
        ticks: 1.0,
    };
    let mut options = TravelOptions::default();
    assert!(t
        .follow(&mut rec, &snap, route.clone(), &mut options)
        .is_none());
    plant_player(&mut c, 100, 0);
    bump_rebuild(&mut c, &mut snap);
    assert!(matches!(
        t.follow(&mut rec, &snap, route.clone(), &mut options),
        Some(TravelOutcome::Arrived { .. })
    ));
    assert_eq!(t.essence(), None, "a cart ride never latches a session");
}

#[test]
fn clear_preserves_the_latched_essence_session() {
    // The mine latch survives route teardown: `clear()` drops the
    // armed route and follow run but keeps the session, so a route
    // out of the mine can be armed after the entry follow's run ends
    // (the live twin relies on this between the entry and exit Follow
    // steps — the exit route re-arms with the traveller's latch).
    let mut t = Traveller::new();
    t.set_essence(essence_session_for_wizard(553));
    t.arm(
        find_on_grid(
            &StepGrid::fixture_open_3x3(),
            Tile {
                x: 0,
                z: 0,
                level: 0,
            },
            Tile {
                x: 2,
                z: 2,
                level: 0,
            },
        )
        .unwrap(),
    );
    assert!(t.queued().is_some(), "route armed before clear");
    t.clear();
    assert_eq!(t.queued(), None, "clear drops the route");
    assert_eq!(
        t.essence(),
        essence_session_for_wizard(553),
        "clear keeps the mine latch"
    );
}

#[test]
fn stun_recovery_rearms_transport_approach_before_interacting() {
    let mut c = scene_client();
    plant_ladder(&mut c, Some("Climb"));
    let mut snap = snap_at(&mut c, 2, 1);
    let mut rec = FollowRec {
        route: Some((0, 0)),
        ..Default::default()
    };
    let mut t = Traveller::new();
    let route = Route {
        legs: vec![Leg::Transport {
            edge: Box::new(ladder_edge()),
        }],
        dest: WorldTile {
            x: 3202,
            z: 3205,
            level: 0,
        },
        ticks: 2.0,
    };
    let mut options = TravelOptions::default();
    assert!(t
        .follow(&mut rec, &snap, route.clone(), &mut options)
        .is_none());
    c.local_player.as_mut().unwrap().spotanim_id = 245;
    c.local_player.as_mut().unwrap().spotanim_last_cycle = 100;
    bump_rebuild(&mut c, &mut snap);
    for _ in 0..11 {
        assert!(t
            .follow(&mut rec, &snap, route.clone(), &mut options)
            .is_none());
        assert_eq!(rec.walked.len(), 1);
        assert_eq!(rec.loc_ops, 0);
        bump_rebuild(&mut c, &mut snap);
    }
    assert!(t
        .follow(&mut rec, &snap, route.clone(), &mut options)
        .is_none());
    assert_eq!(rec.walked, vec![(2, 3), (2, 3)]);
    assert_eq!(rec.loc_ops, 0);
    plant_player(&mut c, 2, 3);
    bump_rebuild(&mut c, &mut snap);
    assert!(t
        .follow(&mut rec, &snap, route.clone(), &mut options)
        .is_none());
    assert_eq!(rec.loc_ops, 1);
    plant_player(&mut c, 2, 5);
    bump_rebuild(&mut c, &mut snap);
    assert!(matches!(
        t.follow(&mut rec, &snap, route, &mut options),
        Some(TravelOutcome::Arrived { .. })
    ));
}

#[test]
fn disconnect_during_stun_wait_does_not_wait_for_another_game_tick() {
    let mut c = scene_client();
    let mut snap = snap_at(&mut c, 0, 0);
    let mut rec = FollowRec {
        route: Some((0, 0)),
        ..Default::default()
    };
    let mut t = Traveller::new();
    let mut options = TravelOptions::default();
    let route = Route {
        legs: vec![walk_leg(&[(3200, 3200), (3200, 3201)])],
        dest: WorldTile {
            x: 3200,
            z: 3201,
            level: 0,
        },
        ticks: 0.5,
    };
    assert!(t
        .follow(&mut rec, &snap, route.clone(), &mut options)
        .is_none());
    c.local_player.as_mut().unwrap().spotanim_id = 245;
    bump_rebuild(&mut c, &mut snap);
    assert!(t
        .follow(&mut rec, &snap, route.clone(), &mut options)
        .is_none());
    c.stream = None;
    c.ingame = false;
    bump_rebuild(&mut c, &mut snap);
    assert!(t.follow(&mut rec, &snap, route, &mut options).is_some());
    assert_eq!(rec.walked.len(), 1);
}

#[test]
fn follow_approaches_a_transport_loc_before_interacting() {
    // The router arms a transport leg from an adjacent take-off. This
    // test starts 3 tiles south of the ladder (a follow that still has
    // to close the last gap). The game only accepts an
    // `op_loc` from adjacent. The player starts 3 tiles south of the
    // ladder loc: `follow` must first walk to the nearest standable
    // tile within chebyshev 1 of `at` (here (3202, 3203)) and only
    // then send `op_loc`; once adjacent it interacts and settles
    // `arrived(edge.to)`.
    let mut c = scene_client();
    plant_ladder(&mut c, Some("Climb"));
    let mut snap = snap_at(&mut c, 2, 1); // cheb 3 from the ladder's `at`
    let mut rec = FollowRec {
        route: Some((0, 0)),
        ..FollowRec::default()
    };
    let mut t = Traveller::new();
    let route = Route {
        legs: vec![Leg::Transport {
            edge: Box::new(ladder_edge()),
        }],
        dest: WorldTile {
            x: 3202,
            z: 3205,
            level: 0,
        },
        ticks: 2.0, // the ladder edge's ticks
    };
    let events = std::cell::RefCell::new(Vec::new());
    let mut options = TravelOptions {
        on_event: Some(Box::new(|e| events.borrow_mut().push(e))),
        ..Default::default()
    };
    // Poll 1: the hop walks to the adjacent standable tile, never the
    // interact (the click would be dropped from 3 tiles away).
    assert!(t
        .follow(&mut rec, &snap, route.clone(), &mut options)
        .is_none());
    assert_eq!(rec.walked, vec![(2, 3)], "the approach walk goes out first");
    assert_eq!(rec.loc_ops, 0, "no OP_LOC1 before the player is adjacent");
    // The player steps onto the approach tile: the hop sends `op_loc`.
    plant_player(&mut c, 2, 3);
    bump_rebuild(&mut c, &mut snap);
    assert!(t
        .follow(&mut rec, &snap, route.clone(), &mut options)
        .is_none());
    assert_eq!(rec.loc_ops, 1, "one OP_LOC1 once adjacent");
    // The ladder carries the player to `edge.to`: the run arrives.
    plant_player(&mut c, 2, 5);
    bump_rebuild(&mut c, &mut snap);
    match t.follow(&mut rec, &snap, route.clone(), &mut options) {
        Some(TravelOutcome::Arrived { at }) => {
            assert_eq!(
                at,
                WorldTile {
                    x: 3202,
                    z: 3205,
                    level: 0
                }
            );
        }
        other => panic!("expected Arrived, got {other:?}"),
    }
    let events = events.borrow();
    assert!(events
        .iter()
        .any(|e| matches!(e, TravelEvent::TransportState { approach: true, .. })));
    assert!(events.iter().any(|e| matches!(
        e,
        TravelEvent::TransportAttempt {
            actual_id: 1,
            option: 1,
            refusal: None,
            ..
        }
    )));
    assert_eq!(
        rec.walked.len(),
        1,
        "diagnostics must not add movement sends"
    );
    assert_eq!(rec.loc_ops, 1, "diagnostics must not add transport sends");
}

#[test]
fn follow_pinned_takeoff_rejects_a_nearby_operable_stand() {
    let mut c = scene_client();
    plant_loc(&mut c, 1, "Ladder", "Climb", 2, 4);
    let mut snap = snap_at(&mut c, 2, 0);
    // Exact takeoffs require the real footprint predicate, not the legacy
    // wall fixture's unknown-geometry/radius fallback.
    let mut footprint = snap.locs()[0].clone();
    footprint.shape = 10;
    snap.seed_locs(vec![footprint.clone()]);
    let mut rec = FollowRec {
        route: Some((0, 0)),
        ..FollowRec::default()
    };
    let mut t = Traveller::new();
    let mut edge = ladder_edge();
    let required = WorldTile {
        x: 3202,
        z: 3203,
        level: 0,
    };
    edge.takeoff = Some(required);
    let route = Route {
        legs: vec![Leg::Transport {
            edge: Box::new(edge),
        }],
        dest: WorldTile {
            x: 3202,
            z: 3205,
            level: 0,
        },
        ticks: 2.0,
    };

    assert!(t
        .follow(
            &mut rec,
            &snap,
            route.clone(),
            &mut TravelOptions::default()
        )
        .is_none());
    assert_eq!(rec.walked, vec![(2, 3)]);
    assert_eq!(rec.loc_ops, 0);

    // A different stand is still adjacent to the loc, but it is not the
    // content-required starting tile and must never trigger the op.
    plant_player(&mut c, 1, 4);
    bump_rebuild(&mut c, &mut snap);
    snap.seed_locs(vec![footprint.clone()]);
    assert_eq!(
        api::query::loc_approach::can_operate_from(
            &footprint,
            snap.scene(),
            WorldTile {
                x: 3201,
                z: 3204,
                level: 0
            },
        ),
        Some(true),
        "the substitute really is another operable stand"
    );
    assert!(t
        .follow(
            &mut rec,
            &snap,
            route.clone(),
            &mut TravelOptions::default()
        )
        .is_none());
    assert_eq!(rec.loc_ops, 0);

    plant_player(&mut c, 2, 3);
    bump_rebuild(&mut c, &mut snap);
    snap.seed_locs(vec![footprint.clone()]);
    assert!(t
        .follow(
            &mut rec,
            &snap,
            route.clone(),
            &mut TravelOptions::default()
        )
        .is_none());
    assert_eq!(rec.loc_ops, 1);

    plant_player(&mut c, 2, 4);
    bump_rebuild(&mut c, &mut snap);
    snap.seed_locs(vec![footprint.clone()]);
    assert!(t
        .follow(
            &mut rec,
            &snap,
            route.clone(),
            &mut TravelOptions::default()
        )
        .is_none());
    plant_player(&mut c, 2, 5);
    bump_rebuild(&mut c, &mut snap);
    snap.seed_locs(vec![footprint]);
    assert!(matches!(
        t.follow(&mut rec, &snap, route, &mut TravelOptions::default()),
        Some(TravelOutcome::Arrived { .. })
    ));
}

#[test]
fn follow_auto_troll_keeps_the_spent_cheap_hop_budget() {
    // A tick-perfect closer can outlast the cheap hop. The automatic
    // fallback may attempt recovery, but cannot spend a second wait budget
    // before reporting that the unchanged position's send was dropped.
    let mut c = scene_client();
    plant_door(&mut c, false, 1);
    let mut snap = snap_at(&mut c, 0, 0);
    let mut rec = FollowRec {
        route: Some((0, 0)),
        ..FollowRec::default()
    };
    let mut t = Traveller::new();
    let route = Route {
        legs: vec![Leg::Transport {
            edge: Box::new(TransportEdge {
                open_loc_id: Some(1531),
                ..door_edge()
            }),
        }],
        dest: WorldTile {
            x: 3203,
            z: 3200,
            level: 0,
        },
        ticks: 1.0,
    };
    // Default options: the fallback must engage automatically.
    let mut options = TravelOptions {
        budget_ticks_per_hop: 3,
        close_enough: 1,
        ..TravelOptions::default()
    };

    let mut tick = 0u32;
    let mut crossed = false;
    loop {
        match t.follow(&mut rec, &snap, route.clone(), &mut options) {
            Some(TravelOutcome::Stalled { at, why, .. }) => {
                assert_eq!(
                    at,
                    WorldTile {
                        x: 3200,
                        z: 3200,
                        level: 0
                    }
                );
                assert_eq!(why, HopFailure::Dropped);
                break;
            }
            Some(other) => panic!("expected bounded Stalled, got {other:?}"),
            None => {}
        }
        tick += 1;
        assert!(tick <= 5, "the fallback extended the spent hop budget");
        // The closer slams the door shut each tick: alternate the
        // door's open/closed state, and only move the player once a
        // walk was actually sent (the troll's same-tick walk).
        let open = tick % 2 == 1;
        plant_door(&mut c, open, 1);
        if open && !crossed && !rec.walked.is_empty() {
            crossed = true;
            plant_player(&mut c, 3, 0);
        }
        bump_rebuild(&mut c, &mut snap);
    }
    assert!(!crossed, "the expired hop cannot wait for another crossing");
}

#[test]
fn follow_troll_walks_an_open_door_without_closing_it() {
    // OP_LOC1 on an open door is Close. Exercise the troll with remaining
    // budget; it must walk through an already-open door without clicking it.
    let mut c = scene_client();
    plant_door(&mut c, false, 1);
    let mut snap = snap_at(&mut c, 0, 0);
    let mut rec = FollowRec {
        route: Some((0, 0)),
        ..FollowRec::default()
    };
    let mut t = Traveller::new();
    let route = Route {
        legs: vec![Leg::Transport {
            edge: Box::new(TransportEdge {
                open_loc_id: Some(1531),
                ..door_edge()
            }),
        }],
        dest: WorldTile {
            x: 3203,
            z: 3200,
            level: 0,
        },
        ticks: 1.0,
    };
    let mut options = TravelOptions {
        budget_ticks_per_hop: 2,
        close_enough: 1,
        ..TravelOptions::default()
    };

    // Arm the cheap send, then exercise the recovery before its deadline.
    assert!(t
        .follow(&mut rec, &snap, route.clone(), &mut options)
        .is_none());
    let ops_after_cheap = rec.loc_ops;
    assert!(ops_after_cheap >= 1, "cheap hop sent OP_LOC1");
    t.follow.as_mut().unwrap().transport.as_mut().unwrap().troll = true;

    plant_door(&mut c, true, 1);
    bump_rebuild(&mut c, &mut snap);
    let ops_before_open = rec.loc_ops;
    assert!(t
        .follow(&mut rec, &snap, route.clone(), &mut options)
        .is_none());
    assert_eq!(
        rec.loc_ops, ops_before_open,
        "troll must not OP_LOC1 an open door (that Closes it)"
    );
    assert!(
        rec.walked.contains(&(3, 0)),
        "troll walks through the open door"
    );
}

#[test]
fn troll_open_door_progress_does_not_reverse_to_approach() {
    // A far-side target takes several snapshots to reach. Recovery must
    // not replace the forward walk with an approach once outside radius1.
    for dir in [None, Some(DoorDir::E)] {
        let mut c = scene_client();
        plant_door(&mut c, true, 1);
        let mut snap = snap_at(&mut c, 0, 0);
        let mut edge = door_edge();
        edge.dir = dir;
        edge.open_loc_id = Some(1531);
        edge.to.x = 3205;
        let route = Route {
            legs: vec![Leg::Transport {
                edge: Box::new(edge.clone()),
            }],
            dest: edge.to,
            ticks: 1.0,
        };
        let mut options = TravelOptions {
            close_enough: 0,
            ..TravelOptions::default()
        };
        let mut run = FollowRun::start(route, &options);
        let leg = run.legs.pop_front().unwrap();
        run.transport = Some(TransportHop {
            leg,
            to: edge.to,
            ticks_waited: 0,
            sent_tile: None,
            arrival_footprint: None,
            tries: 0,
            troll: true,
            npc_index: None,
            npc_recovery: super::NpcRecovery::default(),
            open_sent_tick: None,
            chat_seq: 0,
            dialog_page: None,
            approach: None,
        });
        let mut rec = FollowRec {
            route: Some((0, 0)),
            ..FollowRec::default()
        };
        for x in [0, 2, 3, 4] {
            plant_player(&mut c, x, 0);
            bump_rebuild(&mut c, &mut snap);
            let before = rec.walked.len();
            assert!(matches!(
                run.poll_transport(&mut rec, &snap, &mut options, &mut None),
                Poll::Watching
            ));
            assert_eq!(
                &rec.walked[before..],
                &[(5, 0)],
                "forward progress at x={x}, dir={dir:?}"
            );
            assert_eq!(rec.loc_ops, 0);
        }
        plant_player(&mut c, 5, 0);
        bump_rebuild(&mut c, &mut snap);
        assert!(matches!(
            run.poll_transport(&mut rec, &snap, &mut options, &mut None),
            Poll::LegDone
        ));
    }
}

#[test]
fn troll_probes_crossing_after_open_before_snapshot_catches_up() {
    let mut c = scene_client();
    plant_door(&mut c, false, 1);
    let mut snap = snap_at(&mut c, 1, 0);
    let mut edge = door_edge();
    edge.to.x = 3202;
    edge.dir = Some(DoorDir::E);
    edge.open_loc_id = Some(1531);
    let route = Route {
        legs: vec![Leg::Transport {
            edge: Box::new(edge.clone()),
        }],
        dest: edge.to,
        ticks: 1.0,
    };
    let mut options = TravelOptions {
        close_enough: 0,
        ..TravelOptions::default()
    };
    let mut run = FollowRun::start(route, &options);
    run.transport = Some(TransportHop {
        leg: run.legs.pop_front().unwrap(),
        to: edge.to,
        ticks_waited: 0,
        sent_tile: None,
        arrival_footprint: None,
        tries: 0,
        troll: true,
        npc_index: None,
        npc_recovery: super::NpcRecovery::default(),
        open_sent_tick: None,
        chat_seq: 0,
        dialog_page: None,
        approach: None,
    });
    let mut rec = FollowRec {
        route: Some((1, 0)),
        ..FollowRec::default()
    };
    assert!(matches!(
        run.poll_transport(&mut rec, &snap, &mut options, &mut None),
        Poll::Watching
    ));
    assert_eq!(rec.loc_ops, 1);
    // Server has opened; the delivered snapshot still shows closed.
    bump_rebuild(&mut c, &mut snap);
    assert!(matches!(
        run.poll_transport(&mut rec, &snap, &mut options, &mut None),
        Poll::Watching
    ));
    assert_eq!(
        rec.loc_ops, 1,
        "do not replace the crossing with another Open approach"
    );
    assert_eq!(
        rec.sink.steps,
        vec![client::io::ClientProt::MOVE_GAMECLICK.id, 5, 0, 3202, 3200]
    );
    // Submission does not report arrival. A closed server door can reject it;
    // the existing retry budget remains active until a position update.
    plant_player(&mut c, 2, 0);
    bump_rebuild(&mut c, &mut snap);
    assert!(matches!(
        run.poll_transport(&mut rec, &snap, &mut options, &mut None),
        Poll::LegDone
    ));
}

/// Tenzing-shaped closed door at scene (6,6) — world (3206,3206), clear
/// of the collision map's blocked border — with its wall on the east
/// edge (3745 is angle 2), crossed along `dir` to `to_x`. Starts the
/// cheap hop from scene (`from_x`, 6): one Open, no step.
fn tenzing_door_hop(
    dir: DoorDir,
    from_x: i32,
    to_x: i32,
) -> (
    Client,
    GameSnapshot,
    FollowRec,
    Traveller,
    Route,
    TravelOptions<'static>,
) {
    let mut c = scene_client();
    plant_door_at(&mut c, false, 6, 6);
    c.collision[0].add_wall(6, 6, 0, 2, false);
    let snap = snap_at(&mut c, from_x, 6);
    let mut edge = door_edge();
    edge.at = WorldTile {
        x: 3206,
        z: 3206,
        level: 0,
    };
    edge.to = WorldTile {
        x: 3200 + to_x,
        z: 3206,
        level: 0,
    };
    edge.dir = Some(dir);
    let route = Route {
        legs: vec![Leg::Transport {
            edge: Box::new(edge.clone()),
        }],
        dest: edge.to,
        ticks: 1.0,
    };
    let mut options = TravelOptions {
        budget_ticks_per_hop: 60,
        close_enough: 0,
        ..TravelOptions::default()
    };
    let mut rec = FollowRec {
        route: Some((from_x, 6)),
        ..FollowRec::default()
    };
    let mut t = Traveller::new();
    assert!(t
        .follow(&mut rec, &snap, route.clone(), &mut options)
        .is_none());
    assert_eq!(rec.loc_ops, 1, "the cheap hop sends Open");
    assert!(rec.sink.steps.is_empty() && rec.walked.is_empty());
    (c, snap, rec, t, route, options)
}

/// Tenzing's hut door 3745 (`open_and_close_door2`, wall on the east
/// edge of its loc tile): Open from the outside teleports the player
/// onto the loc tile `at` and swaps the door for an inviswall for three
/// ticks, so the loc never reads open. The cheap hop must take the
/// wall-clear step to `to` instead of sitting out its budget until the
/// troll (live: 60 idle ticks at (2822,3555) before every entry in the
/// ClimbingBoots walk and teleport cells).
#[test]
fn cheap_door_hop_steps_through_when_open_lands_on_the_door_tile() {
    let (mut c, mut snap, mut rec, mut t, route, mut options) = tenzing_door_hop(DoorDir::W, 7, 5);
    // The door script put the player on `at`; the loc still reads closed.
    plant_player(&mut c, 6, 6);
    rec.route = Some((6, 6));
    bump_rebuild(&mut c, &mut snap);
    assert!(t
        .follow(&mut rec, &snap, route.clone(), &mut options)
        .is_none());
    assert_eq!(
        rec.loc_ops, 1,
        "no second Open: from `at` it sends the player back out"
    );
    assert_eq!(
        rec.sink.steps,
        vec![client::io::ClientProt::MOVE_GAMECLICK.id, 5, 0, 3205, 3206],
        "the step to `to` goes out on the tick the player lands on `at`"
    );

    plant_player(&mut c, 5, 6);
    bump_rebuild(&mut c, &mut snap);
    assert!(matches!(
        t.follow(&mut rec, &snap, route.clone(), &mut options),
        Some(TravelOutcome::Arrived { at }) if at.x == 3205 && at.z == 3206
    ));
}

/// The same door left the other way (edge E, `to` across the closed
/// wall): the server walks the player onto `at` before the queued Open
/// fires. A step packet there would cancel that Open, so the cheap hop
/// sends nothing while the wall still blocks `at` → `to`.
#[test]
fn cheap_door_hop_does_not_step_into_a_closed_wall_from_the_door_tile() {
    let (mut c, mut snap, mut rec, mut t, route, mut options) = tenzing_door_hop(DoorDir::E, 5, 7);
    plant_player(&mut c, 6, 6);
    rec.route = Some((6, 6));
    bump_rebuild(&mut c, &mut snap);
    assert!(t
        .follow(&mut rec, &snap, route.clone(), &mut options)
        .is_none());
    assert_eq!(rec.loc_ops, 1);
    assert!(rec.sink.steps.is_empty(), "no step into the closed wall");
    assert!(rec.walked.is_empty());
}

#[test]
fn troll_does_not_reopen_a_door_behind_the_walker() {
    for (dir, player, target) in [
        (DoorDir::E, (6, 4), (8, 4)),
        (DoorDir::W, (2, 4), (0, 4)),
        (DoorDir::N, (4, 6), (4, 8)),
        (DoorDir::S, (4, 2), (4, 0)),
    ] {
        let mut c = scene_client();
        plant_door_at(&mut c, false, 4, 4);
        let mut snap = snap_at(&mut c, player.0, player.1);
        let mut edge = door_edge();
        edge.at = WorldTile {
            x: 3204,
            z: 3204,
            level: 0,
        };
        edge.dir = Some(dir);
        edge.open_loc_id = Some(1531);
        edge.to = WorldTile {
            x: 3200 + target.0,
            z: 3200 + target.1,
            level: 0,
        };
        let route = Route {
            legs: vec![Leg::Transport {
                edge: Box::new(edge.clone()),
            }],
            dest: edge.to,
            ticks: 1.0,
        };
        let mut options = TravelOptions {
            close_enough: 0,
            ..TravelOptions::default()
        };
        let mut run = FollowRun::start(route, &options);
        let leg = run.legs.pop_front().unwrap();
        run.transport = Some(TransportHop {
            leg,
            to: edge.to,
            ticks_waited: 0,
            sent_tile: None,
            arrival_footprint: None,
            tries: 0,
            troll: true,
            npc_index: None,
            npc_recovery: super::NpcRecovery::default(),
            open_sent_tick: None,
            chat_seq: 0,
            dialog_page: None,
            approach: None,
        });
        let mut rec = FollowRec {
            route: Some((0, 0)),
            ..FollowRec::default()
        };
        assert!(matches!(
            run.poll_transport(&mut rec, &snap, &mut options, &mut None),
            Poll::Watching
        ));
        assert_eq!(rec.walked, vec![target], "direction {dir:?}");
        assert_eq!(rec.loc_ops, 0);
        plant_player(&mut c, target.0, target.1);
        bump_rebuild(&mut c, &mut snap);
        assert!(matches!(
            run.poll_transport(&mut rec, &snap, &mut options, &mut None),
            Poll::LegDone
        ));
        assert_eq!(
            rec.walked,
            vec![target],
            "arrival must not send another action"
        );
        assert_eq!(rec.loc_ops, 0);
    }
}

#[test]
fn follow_troll_finds_an_offset_door_loc_within_radius() {
    // The door loc's live tile is offset from the edge's derived `at`
    // by +1 in x, so the exact-tile lookup (`l.tile == at`) never
    // finds it and the troll blocks while the walker stands still.
    // The troll must search by id within radius 3 of `at` — the same
    // shape as `find_transport_loc` — to find the offset loc, re-open
    // it, and walk through on the open tick instead of `Blocked`.
    let mut c = scene_client();
    plant_door(&mut c, false, 2); // offset from door_edge()'s at (3201, 3200)
    let mut snap = snap_at(&mut c, 0, 0);
    let mut rec = FollowRec {
        route: Some((0, 0)),
        ..FollowRec::default()
    };
    let mut t = Traveller::new();
    let route = Route {
        legs: vec![Leg::Transport {
            edge: Box::new(TransportEdge {
                open_loc_id: Some(1531),
                ..door_edge()
            }),
        }],
        dest: WorldTile {
            x: 3203,
            z: 3200,
            level: 0,
        },
        ticks: 1.0,
    };
    let mut options = TravelOptions {
        budget_ticks_per_hop: 3,
        close_enough: 1,
        ..TravelOptions::default()
    };

    // This lookup fixture starts recovery with unspent budget. Deadline
    // escalation must not grant a fresh budget; that is covered separately.
    assert!(t
        .follow(&mut rec, &snap, route.clone(), &mut options)
        .is_none());
    t.follow.as_mut().unwrap().transport.as_mut().unwrap().troll = true;

    let mut tick = 0u32;
    let mut crossed = false;
    loop {
        match t.follow(&mut rec, &snap, route.clone(), &mut options) {
            Some(TravelOutcome::Arrived { at }) => {
                assert_eq!(
                    at,
                    WorldTile {
                        x: 3203,
                        z: 3200,
                        level: 0
                    }
                );
                break;
            }
            Some(other) => panic!("expected Arrived, got {other:?}"),
            None => {}
        }
        tick += 1;
        assert!(tick < 200, "the troll never crossed the offset door");
        // A tick-perfect closer slams the door each tick; the player
        // only crosses once a walk was actually sent (the troll's
        // same-tick walk), exactly like the on-at troll test.
        let open = tick % 2 == 1;
        plant_door(&mut c, open, 2);
        if open && !crossed && !rec.walked.is_empty() {
            crossed = true;
            plant_player(&mut c, 3, 0);
        }
        bump_rebuild(&mut c, &mut snap);
    }
    assert!(crossed, "the troll's same-tick walk never crossed the door");
    assert!(
        rec.loc_ops >= 2,
        "the troll must re-open the offset door after the cheap hop lapses, got {} loc ops",
        rec.loc_ops
    );
    assert!(
        rec.walked.contains(&(3, 0)),
        "the troll walks through the open door in the same tick"
    );
}

#[test]
fn follow_walks_through_an_open_leaf_without_op_loc() {
    // The open leaf (1531) is planted at `edge.at` and the edge
    // carries its id: `follow` must not interact (OP_LOC1 on an open
    // leaf Closes it) — it walks straight through and arrives on
    // `edge.to` with no op_loc at all.
    let mut c = scene_client();
    plant_door(&mut c, true, 1);
    let mut snap = snap_at(&mut c, 0, 0);
    let mut rec = FollowRec {
        route: Some((0, 0)),
        ..FollowRec::default()
    };
    let mut t = Traveller::new();
    let route = Route {
        legs: vec![Leg::Transport {
            edge: Box::new(TransportEdge {
                open_loc_id: Some(1531),
                ..door_edge()
            }),
        }],
        dest: WorldTile {
            x: 3203,
            z: 3200,
            level: 0,
        },
        ticks: 1.0,
    };
    let mut options = TravelOptions::default();
    let outcome = drive(
        &mut t,
        &mut rec,
        &mut c,
        &mut snap,
        &route,
        &mut options,
        |c| {
            plant_player(c, 3, 0);
        },
    );
    assert!(matches!(
        outcome,
        TravelOutcome::Arrived { at } if at == WorldTile { x: 3203, z: 3200, level: 0 }
    ));
    assert_eq!(rec.loc_ops, 0, "no OP_LOC1 through an open leaf");
    assert!(
        rec.walked.contains(&(3, 0)),
        "walk straight through the open door"
    );
}

#[test]
fn follow_walks_through_a_close_leaf_without_op_loc() {
    // The door's config carries no `open_loc_id`: the open leaf is
    // still recognized by the closed id being absent from `edge.at`
    // while a same-tile loc offers the "Close" op.
    let mut c = scene_client();
    plant_door(&mut c, true, 1);
    let mut snap = snap_at(&mut c, 0, 0);
    let mut rec = FollowRec {
        route: Some((0, 0)),
        ..FollowRec::default()
    };
    let mut t = Traveller::new();
    let route = Route {
        legs: vec![Leg::Transport {
            edge: Box::new(door_edge()), // open_loc_id: None
        }],
        dest: WorldTile {
            x: 3203,
            z: 3200,
            level: 0,
        },
        ticks: 1.0,
    };
    let mut options = TravelOptions::default();
    let outcome = drive(
        &mut t,
        &mut rec,
        &mut c,
        &mut snap,
        &route,
        &mut options,
        |c| {
            plant_player(c, 3, 0);
        },
    );
    assert!(matches!(
        outcome,
        TravelOutcome::Arrived { at } if at == WorldTile { x: 3203, z: 3200, level: 0 }
    ));
    assert_eq!(rec.loc_ops, 0, "no OP_LOC1 through a close leaf");
    assert!(
        rec.walked.contains(&(3, 0)),
        "walk straight through the open door"
    );
}

#[test]
fn follow_cheap_hop_walks_when_the_open_leaf_is_offset() {
    // Live Catherby: closed 1530 is derived at (2816,3438) but the
    // open 1531 sits a tile north. `edge_loc_open` used exact `at`, so
    // after Open the hop kept waiting (open=false) until the troll
    // budget. Cheap hop must see the offset open leaf and walk.
    let mut c = scene_client();
    plant_door_at(&mut c, false, 1, 0);
    let mut snap = snap_at(&mut c, 0, 0);
    let mut rec = FollowRec {
        route: Some((0, 0)),
        ..FollowRec::default()
    };
    let mut t = Traveller::new();
    let edge = TransportEdge {
        takeoff: None,
        worn_all_req: Vec::new(),
        open_loc_id: Some(1531),
        dir: Some(DoorDir::E),
        ..door_edge()
    };
    let route = Route {
        legs: vec![Leg::Transport {
            edge: Box::new(edge),
        }],
        dest: WorldTile {
            x: 3203,
            z: 3200,
            level: 0,
        },
        ticks: 1.0,
    };
    let mut options = TravelOptions::default();

    assert!(t
        .follow(&mut rec, &snap, route.clone(), &mut options)
        .is_none());
    assert_eq!(rec.loc_ops, 1, "cheap hop Opens the closed door");

    c.world.del_wall(0, 1, 0);
    plant_door_at(&mut c, true, 1, 1); // open leaf at (3201, 3201)
    bump_rebuild(&mut c, &mut snap);
    let walks = rec.walked.len();
    assert!(t
        .follow(&mut rec, &snap, route.clone(), &mut options)
        .is_none());
    assert!(
        rec.walked.len() > walks,
        "cheap hop must walk through once the offset open leaf appears"
    );
}

#[test]
fn follow_fails_fast_when_the_game_reports_cant_reach() {
    // The client pathfind failed right after the interact: the game
    // says "I can't reach that!" (a chat line new since the hop
    // started) and the hop must fail fast with the unreachable
    // signal — never sit out the settle budget. The player starts
    // adjacent to the ladder, so the interact is the hop's first send
    // (a further start would walk the approach first).
    let mut c = scene_client();
    plant_ladder(&mut c, Some("Climb"));
    let mut snap = snap_at(&mut c, 2, 3);
    let mut rec = FollowRec {
        route: Some((0, 0)),
        ..FollowRec::default()
    };
    let mut t = Traveller::new();
    let route = Route {
        legs: vec![Leg::Transport {
            edge: Box::new(ladder_edge()),
        }],
        dest: WorldTile {
            x: 3202,
            z: 3205,
            level: 0,
        },
        ticks: 2.0, // the ladder edge's ticks
    };
    // `close_enough: 1` keeps the player outside the arrive arm's
    // radius of `edge.to` while adjacent to `at` (the far side is
    // 2 tiles past the ladder), so the can't-reach line is the only
    // arm that can fire.
    let mut options = TravelOptions {
        close_enough: 1,
        ..TravelOptions::default()
    };
    // Poll 1: the interact is sent and the hop starts watching.
    assert!(t
        .follow(&mut rec, &snap, route.clone(), &mut options)
        .is_none());
    assert_eq!(rec.loc_ops, 1, "one OP_LOC1 interact sent");
    // One tick later the game reports the pathfind failed.
    c.add_chat(0, "I can't reach that!", "");
    bump_rebuild(&mut c, &mut snap);
    match t.follow(&mut rec, &snap, route.clone(), &mut options) {
        Some(TravelOutcome::Refused {
            at,
            reason: SendReason::Unreachable,
        }) => {
            assert_eq!(
                at,
                WorldTile {
                    x: 3202,
                    z: 3203,
                    level: 0
                }
            );
        }
        other => panic!("expected Refused(Unreachable), got {other:?}"),
    }
}

#[test]
fn follow_lumbridge_stairs_approaches_operable_side_on_real_289_pack() {
    let Some(world) = crate::world::NavWorld::load_default_pack_or_skip() else {
        return;
    };
    let edge = world
        .graph
        .edges
        .iter()
        .find(|edge| {
            edge.loc_id == 1738
                && edge.at
                    == WorldTile {
                        x: 3204,
                        z: 3207,
                        level: 0,
                    }
        })
        .expect("real Lumbridge south staircase")
        .clone();
    let mut c = scene_client();
    plant_loc_sized(&mut c, 1738, "Staircase", "Climb-up", 4, 7, 2, 2, 0);
    for x in 0..104 {
        for z in 0..104 {
            c.collision[0].flags[x][z] =
                world
                    .collision
                    .walkable_word(3200 + x as i32, 3200 + z as i32, 0) as i32;
        }
    }
    let mut snap = snap_at(&mut c, 6, 9);
    let mut loc = snap
        .locs()
        .iter()
        .find(|loc| loc.id == 1738)
        .unwrap()
        .clone();
    loc.shape = 10;
    loc.footprint_width = 2;
    loc.footprint_length = 2;
    snap.seed_locs(vec![loc.clone()]);
    let from = WorldTile {
        x: 3205,
        z: 3206,
        level: 0,
    };
    assert_eq!(
        api::query::loc_approach::can_operate_from(&loc, snap.scene(), from),
        Some(false),
        "the south castle wall separates the Chebyshev-near stand from the stairs"
    );
    let near = WorldTile {
        x: 3206,
        z: 3209,
        level: 0,
    };
    let flood = api::query::SceneQuery::new(snap.scene(), Some(near))
        .flood_reach()
        .unwrap();
    let approach = api::query::loc_approach::booth_approach(&loc, snap.scene(), near, &flood)
        .unwrap()
        .dest
        .expect("inside the castle, an operable stair side is reachable");
    eprintln!("real stairs near={near:?} loc={loc:?} approach={approach:?}");
    assert_eq!(
        api::query::loc_approach::can_operate_from(&loc, snap.scene(), approach),
        Some(true)
    );
    let route = Route {
        legs: vec![Leg::Transport {
            edge: Box::new(edge.clone()),
        }],
        dest: edge.to,
        ticks: edge.ticks as f64,
    };
    let mut rec = FollowRec {
        route: Some((6, 9)),
        ..FollowRec::default()
    };
    let mut traveller = Traveller::new();
    let mut options = TravelOptions {
        close_enough: 0,
        ..TravelOptions::default()
    };
    assert!(traveller
        .follow(&mut rec, &snap, route.clone(), &mut options)
        .is_none());
    assert_eq!(
        rec.loc_ops, 0,
        "do not send a stairs op through the castle wall"
    );
    assert_eq!(
        rec.walked.last(),
        Some(&(approach.x - 3200, approach.z - 3200))
    );
    // A near-anchor tile still behind the wall must not finish the approach.
    plant_player(&mut c, 5, 6);
    bump_rebuild(&mut c, &mut snap);
    snap.seed_locs(vec![loc.clone()]);
    assert!(traveller
        .follow(&mut rec, &snap, route.clone(), &mut options)
        .is_none());
    assert_eq!(rec.loc_ops, 0);
    plant_player(&mut c, approach.x - 3200, approach.z - 3200);
    bump_rebuild(&mut c, &mut snap);
    snap.seed_locs(vec![loc]);
    assert!(traveller
        .follow(&mut rec, &snap, route.clone(), &mut options)
        .is_none());
    assert_eq!(
        rec.loc_ops, 1,
        "the operable footprint stand can send Climb-up"
    );
    c.minusedlevel = 1;
    plant_player(&mut c, edge.to.x - 3200, edge.to.z - 3200);
    bump_rebuild(&mut c, &mut snap);
    assert!(matches!(
        traveller.follow(&mut rec, &snap, route, &mut options),
        Some(TravelOutcome::Arrived { at }) if at == edge.to
    ));
}

#[test]
fn door_tile_is_the_edge_at() {
    // A door with `to` 2 tiles away: the door's own tile is the
    // edge's `at` (the loc tile), never the midpoint of `at`/`to`.
    let edge = door_edge();
    assert_eq!(door_tile(&edge), edge.at);
    assert_ne!(door_tile(&edge).x, (edge.at.x + edge.to.x) / 2);
}

#[test]
fn follow_stalls_dropped_when_the_player_never_moves() {
    let mut c = scene_client();
    let mut snap = snap_at(&mut c, 0, 0);
    let mut rec = FollowRec {
        route: Some((0, 0)),
        ..FollowRec::default()
    };
    let mut t = Traveller::new();
    let route = Route {
        legs: vec![walk_leg(&[
            (3200, 3200),
            (3200, 3201),
            (3200, 3202),
            (3200, 3203),
            (3200, 3204),
        ])],
        dest: WorldTile {
            x: 3200,
            z: 3204,
            level: 0,
        },
        ticks: 2.0, // 4 run steps at 0.5 ticks each
    };
    let mut options = TravelOptions {
        budget_ticks_per_hop: 3,
        ..TravelOptions::default()
    };
    // The player never leaves the send tile: the hop lapses as Dropped.
    // Budget 3 is below the cancelled-walk recovery window, so the
    // original bound still exhausts without an unbounded retry loop.
    let outcome = drive(
        &mut t,
        &mut rec,
        &mut c,
        &mut snap,
        &route,
        &mut options,
        |_| {},
    );
    assert!(matches!(
        outcome,
        TravelOutcome::Stalled { at, aiming, why: HopFailure::Dropped, .. }
            if at == WorldTile { x: 3200, z: 3200, level: 0 }
                && aiming == WorldTile { x: 3200, z: 3204, level: 0 }
    ));
}

#[test]
fn follow_recovers_cancelled_walk_with_same_aim() {
    // Sent hop cancelled in place (no map flag, still on sent_tile):
    // after five distinct idle game ticks, reissue the same aim.
    let mut c = scene_client();
    let mut snap = snap_at(&mut c, 0, 0);
    let mut rec = FollowRec {
        route: Some((0, 0)),
        ..FollowRec::default()
    };
    let mut t = Traveller::new();
    let route = Route {
        legs: vec![walk_leg(&[
            (3200, 3200),
            (3200, 3201),
            (3200, 3202),
            (3200, 3203),
            (3200, 3204),
        ])],
        dest: WorldTile {
            x: 3200,
            z: 3204,
            level: 0,
        },
        ticks: 2.0,
    };
    let attempts = std::cell::RefCell::new(Vec::new());
    let mut options = TravelOptions {
        budget_ticks_per_hop: 20,
        on_event: Some(Box::new(|ev| {
            if let TravelEvent::WalkAttempt { aim, refusal, .. } = ev {
                attempts.borrow_mut().push((aim, refusal));
            }
        })),
        ..TravelOptions::default()
    };
    assert!(t
        .follow(&mut rec, &snap, route.clone(), &mut options)
        .is_none());
    assert_eq!(rec.walked.len(), 1, "first send");
    let first_aim = attempts.borrow()[0].0;
    // Four more idle ticks: still one send.
    for _ in 0..4 {
        bump_rebuild(&mut c, &mut snap);
        assert!(t
            .follow(&mut rec, &snap, route.clone(), &mut options)
            .is_none());
    }
    assert_eq!(rec.walked.len(), 1, "recovery waits the idle window");
    // Fifth idle tick: same-aim recovery.
    bump_rebuild(&mut c, &mut snap);
    assert!(t
        .follow(&mut rec, &snap, route.clone(), &mut options)
        .is_none());
    assert_eq!(rec.walked.len(), 2, "one recovery reissue");
    assert_eq!(attempts.borrow().len(), 2);
    assert_eq!(
        attempts.borrow()[1].0,
        first_aim,
        "recovery preserves sent aim"
    );
    assert_eq!(attempts.borrow()[1].1, None);
    // Arrival after recovery still completes.
    plant_player(&mut c, 0, 4);
    bump_rebuild(&mut c, &mut snap);
    assert!(matches!(
        t.follow(&mut rec, &snap, route, &mut options),
        Some(TravelOutcome::Arrived { .. })
    ));
}

#[test]
fn follow_does_not_recover_walk_while_moving() {
    let mut c = scene_client();
    let mut snap = snap_at(&mut c, 0, 0);
    let mut rec = FollowRec {
        route: Some((0, 0)),
        ..FollowRec::default()
    };
    let mut t = Traveller::new();
    let route = Route {
        legs: vec![walk_leg(&[
            (3200, 3200),
            (3200, 3201),
            (3200, 3202),
            (3200, 3203),
            (3200, 3204),
        ])],
        dest: WorldTile {
            x: 3200,
            z: 3204,
            level: 0,
        },
        ticks: 2.0,
    };
    let mut options = TravelOptions {
        budget_ticks_per_hop: 20,
        ..TravelOptions::default()
    };
    assert!(t
        .follow(&mut rec, &snap, route.clone(), &mut options)
        .is_none());
    assert_eq!(rec.walked.len(), 1);
    // Movement must not accumulate idle ticks, with or without a map flag.
    c.minimap_flag_x = 12;
    c.minimap_flag_z = 34;
    c.local_player.as_mut().unwrap().entity.route_length = 3;
    for _ in 0..8 {
        bump_rebuild(&mut c, &mut snap);
        assert!(t
            .follow(&mut rec, &snap, route.clone(), &mut options)
            .is_none());
    }
    assert_eq!(rec.walked.len(), 1, "a live walk must not spuriously retry");
    // Flag clears but actor is moving: still no recovery.
    c.minimap_flag_x = 0;
    c.minimap_flag_z = 0;
    for _ in 0..8 {
        bump_rebuild(&mut c, &mut snap);
        assert!(t
            .follow(&mut rec, &snap, route.clone(), &mut options)
            .is_none());
    }
    assert_eq!(rec.walked.len(), 1, "moving must not spuriously retry");
}

#[test]
fn follow_cancelled_walk_idle_ignores_duplicate_snapshot_ticks() {
    let mut c = scene_client();
    let mut snap = snap_at(&mut c, 0, 0);
    let mut rec = FollowRec {
        route: Some((0, 0)),
        ..FollowRec::default()
    };
    let mut t = Traveller::new();
    let route = Route {
        legs: vec![walk_leg(&[
            (3200, 3200),
            (3200, 3201),
            (3200, 3202),
            (3200, 3203),
            (3200, 3204),
        ])],
        dest: WorldTile {
            x: 3200,
            z: 3204,
            level: 0,
        },
        ticks: 2.0,
    };
    let mut options = TravelOptions {
        budget_ticks_per_hop: 30,
        ..TravelOptions::default()
    };
    assert!(t
        .follow(&mut rec, &snap, route.clone(), &mut options)
        .is_none());
    assert_eq!(rec.walked.len(), 1);
    // Many polls on the same snapshot tick must not count as idle ticks.
    for _ in 0..20 {
        assert!(t
            .follow(&mut rec, &snap, route.clone(), &mut options)
            .is_none());
    }
    assert_eq!(
        rec.walked.len(),
        1,
        "duplicate snapshot polls must not trigger recovery"
    );
    // The send-tick polls already credited one distinct idle tick.
    // Three fresh ticks → idle 4; still under the threshold of 5.
    for _ in 0..3 {
        bump_rebuild(&mut c, &mut snap);
        assert!(t
            .follow(&mut rec, &snap, route.clone(), &mut options)
            .is_none());
    }
    assert_eq!(rec.walked.len(), 1);
    bump_rebuild(&mut c, &mut snap);
    assert!(t
        .follow(&mut rec, &snap, route.clone(), &mut options)
        .is_none());
    assert_eq!(rec.walked.len(), 2, "fifth distinct idle tick recovers");
}

#[test]
fn follow_persistent_cancel_after_recovery_exhausts_original_budget() {
    // Repeated recovery must still Drop under the original hop budget
    // (ticks_waited is not reset and recovery does not consume max_hops).
    let mut c = scene_client();
    let mut snap = snap_at(&mut c, 0, 0);
    let mut rec = FollowRec {
        route: Some((0, 0)),
        ..FollowRec::default()
    };
    let mut t = Traveller::new();
    let route = Route {
        legs: vec![walk_leg(&[
            (3200, 3200),
            (3200, 3201),
            (3200, 3202),
            (3200, 3203),
            (3200, 3204),
        ])],
        dest: WorldTile {
            x: 3200,
            z: 3204,
            level: 0,
        },
        ticks: 2.0,
    };
    let mut options = TravelOptions {
        budget_ticks_per_hop: 12,
        max_hops: 1,
        ..TravelOptions::default()
    };
    let outcome = drive(
        &mut t,
        &mut rec,
        &mut c,
        &mut snap,
        &route,
        &mut options,
        |_| {},
    );
    assert!(matches!(
        outcome,
        TravelOutcome::Stalled {
            why: HopFailure::Dropped,
            tries,
            ..
        } if tries == 3
    ));
    assert_eq!(rec.walked.len(), 3, "only two idle windows fit the budget");
}

#[test]
fn follow_recovers_cancelled_walk_after_partial_progress() {
    // Chaos-style: hop progresses partway, map flag clears, player
    // idles off the original sent_tile — same-aim recovery after
    // five distinct idle ticks at the new position.
    let mut c = scene_client();
    let mut snap = snap_at(&mut c, 0, 0);
    let mut rec = FollowRec {
        route: Some((0, 0)),
        ..FollowRec::default()
    };
    let mut t = Traveller::new();
    let route = Route {
        legs: vec![walk_leg(&[
            (3200, 3200),
            (3200, 3201),
            (3200, 3202),
            (3200, 3203),
            (3200, 3204),
            (3200, 3205),
            (3200, 3206),
            (3200, 3207),
        ])],
        dest: WorldTile {
            x: 3200,
            z: 3207,
            level: 0,
        },
        ticks: 3.5,
    };
    let attempts = std::cell::RefCell::new(Vec::new());
    let mut options = TravelOptions {
        budget_ticks_per_hop: 30,
        on_event: Some(Box::new(|ev| {
            if let TravelEvent::WalkAttempt { aim, refusal, .. } = ev {
                attempts.borrow_mut().push((aim, refusal));
            }
        })),
        ..TravelOptions::default()
    };
    assert!(t
        .follow(&mut rec, &snap, route.clone(), &mut options)
        .is_none());
    assert_eq!(rec.walked.len(), 1, "first send");
    let first_aim = attempts.borrow()[0].0;
    // Partial progress away from sent_tile, then stop (no map flag).
    plant_player(&mut c, 0, 2);
    c.minimap_flag_x = 0;
    c.minimap_flag_z = 0;
    c.local_player.as_mut().unwrap().entity.route_length = 0;
    // Four distinct idle ticks at the midway tile: still one send.
    for _ in 0..4 {
        bump_rebuild(&mut c, &mut snap);
        assert!(t
            .follow(&mut rec, &snap, route.clone(), &mut options)
            .is_none());
    }
    assert_eq!(
        rec.walked.len(),
        1,
        "partial-progress idle waits the full window"
    );
    // Fifth idle tick at midway tile: same-aim recovery.
    bump_rebuild(&mut c, &mut snap);
    assert!(t
        .follow(&mut rec, &snap, route.clone(), &mut options)
        .is_none());
    assert_eq!(rec.walked.len(), 2, "one recovery after partial progress");
    assert_eq!(attempts.borrow().len(), 2);
    assert_eq!(
        attempts.borrow()[1].0,
        first_aim,
        "recovery preserves sent aim"
    );
    assert_eq!(attempts.borrow()[1].1, None);
}

#[test]
fn follow_partial_progress_tile_change_resets_stall_idle() {
    let mut c = scene_client();
    let mut snap = snap_at(&mut c, 0, 0);
    let mut rec = FollowRec {
        route: Some((0, 0)),
        ..FollowRec::default()
    };
    let mut t = Traveller::new();
    let route = Route {
        legs: vec![walk_leg(&[
            (3200, 3200),
            (3200, 3201),
            (3200, 3202),
            (3200, 3203),
            (3200, 3204),
            (3200, 3205),
            (3200, 3206),
            (3200, 3207),
        ])],
        dest: WorldTile {
            x: 3200,
            z: 3207,
            level: 0,
        },
        ticks: 3.5,
    };
    let mut options = TravelOptions {
        budget_ticks_per_hop: 40,
        ..TravelOptions::default()
    };
    assert!(t
        .follow(&mut rec, &snap, route.clone(), &mut options)
        .is_none());
    assert_eq!(rec.walked.len(), 1);
    // Idle three ticks at z=2, then move to z=3 — idle window resets.
    plant_player(&mut c, 0, 2);
    for _ in 0..3 {
        bump_rebuild(&mut c, &mut snap);
        assert!(t
            .follow(&mut rec, &snap, route.clone(), &mut options)
            .is_none());
    }
    assert_eq!(rec.walked.len(), 1);
    plant_player(&mut c, 0, 3);
    for _ in 0..4 {
        bump_rebuild(&mut c, &mut snap);
        assert!(t
            .follow(&mut rec, &snap, route.clone(), &mut options)
            .is_none());
    }
    assert_eq!(
        rec.walked.len(),
        1,
        "tile change must clear prior idle credits"
    );
    bump_rebuild(&mut c, &mut snap);
    assert!(t
        .follow(&mut rec, &snap, route.clone(), &mut options)
        .is_none());
    assert_eq!(
        rec.walked.len(),
        2,
        "fifth distinct idle tick at new tile recovers"
    );
}

#[test]
fn follow_partial_progress_cancel_still_exhausts_original_budget() {
    // Progress off sent_tile and repeated recovery must still Drop/Expire
    // under the original hop budget.
    let mut c = scene_client();
    let mut snap = snap_at(&mut c, 0, 0);
    let mut rec = FollowRec {
        route: Some((0, 0)),
        ..FollowRec::default()
    };
    let mut t = Traveller::new();
    let route = Route {
        legs: vec![walk_leg(&[
            (3200, 3200),
            (3200, 3201),
            (3200, 3202),
            (3200, 3203),
            (3200, 3204),
            (3200, 3205),
            (3200, 3206),
            (3200, 3207),
        ])],
        dest: WorldTile {
            x: 3200,
            z: 3207,
            level: 0,
        },
        ticks: 3.5,
    };
    let mut options = TravelOptions {
        budget_ticks_per_hop: 12,
        max_hops: 1,
        ..TravelOptions::default()
    };
    assert!(t
        .follow(&mut rec, &snap, route.clone(), &mut options)
        .is_none());
    assert_eq!(rec.walked.len(), 1);
    // Move partway and stay put until budget + recovery settle.
    plant_player(&mut c, 0, 2);
    let outcome = drive(
        &mut t,
        &mut rec,
        &mut c,
        &mut snap,
        &route,
        &mut options,
        |c| {
            plant_player(c, 0, 2);
        },
    );
    assert!(matches!(
        outcome,
        TravelOutcome::Stalled {
            tries,
            ..
        } if tries == 3
    ));
    assert_eq!(rec.walked.len(), 3, "only two idle windows fit the budget");
}

#[test]
fn follow_stalls_expired_when_progress_stalls() {
    let mut c = scene_client();
    let mut snap = snap_at(&mut c, 0, 0);
    let mut rec = FollowRec {
        route: Some((0, 0)),
        ..FollowRec::default()
    };
    let mut t = Traveller::new();
    let route = Route {
        legs: vec![walk_leg(&[
            (3200, 3200),
            (3200, 3201),
            (3200, 3202),
            (3200, 3203),
            (3200, 3204),
            (3200, 3205),
            (3200, 3206),
            (3200, 3207),
        ])],
        dest: WorldTile {
            x: 3200,
            z: 3207,
            level: 0,
        },
        ticks: 3.5, // 7 run steps at 0.5 ticks each
    };
    let mut options = TravelOptions {
        budget_ticks_per_hop: 3,
        ..TravelOptions::default()
    };
    // The player creeps one tile per tick toward the far leg end but
    // never arrives within the tiny hop budget: the hop lapses as
    // Expired (progress was made).
    let mut z = 0;
    let outcome = drive(
        &mut t,
        &mut rec,
        &mut c,
        &mut snap,
        &route,
        &mut options,
        |c| {
            z += 1;
            plant_player(c, 0, z);
        },
    );
    assert!(matches!(
        outcome,
        TravelOutcome::Stalled {
            why: HopFailure::Expired,
            ..
        }
    ));
}

#[test]
fn follow_retries_a_near_tile_when_the_far_hop_is_unreachable() {
    let mut c = scene_client();
    let snap = snap_at(&mut c, 0, 0);
    let mut rec = FollowRec {
        route: Some((0, 0)),
        reject_far: true,
        ..FollowRec::default()
    };
    let mut t = Traveller::new();
    let route = Route {
        legs: vec![walk_leg(&[
            (3200, 3200),
            (3200, 3201),
            (3200, 3202),
            (3200, 3203),
            (3200, 3204),
        ])],
        dest: WorldTile {
            x: 3200,
            z: 3204,
            level: 0,
        },
        ticks: 2.0,
    };
    let mut options = TravelOptions::default();
    // The driver rejects a multi-tile hop: follow walks the index
    // back to the adjacent tile (the v1 traveller's fallback).
    assert!(t
        .follow(&mut rec, &snap, route.clone(), &mut options)
        .is_none());
    assert_eq!(
        rec.walked.last().copied(),
        Some((0, 1)),
        "the adjacent tile is the fallback hop"
    );
}

#[test]
fn follow_gives_up_after_max_hops() {
    let mut c = scene_client();
    let mut snap = snap_at(&mut c, 0, 0);
    let mut rec = FollowRec {
        route: Some((0, 0)),
        ..FollowRec::default()
    };
    let mut t = Traveller::new();
    let tiles: Vec<(i32, i32)> = (0..40).map(|z| (3200, 3200 + z)).collect();
    let route = Route {
        legs: vec![walk_leg(&tiles)],
        dest: WorldTile {
            x: 3200,
            z: 3239,
            level: 0,
        },
        ticks: 19.5, // 39 run steps at 0.5 ticks each
    };
    let mut options = TravelOptions {
        close_enough: 200,
        max_hops: 2,
        ..TravelOptions::default()
    };
    // A loose close-enough matches every poll, so each call starts a
    // fresh hop until the hop cap trips `GaveUp`.
    let outcome = drive(
        &mut t,
        &mut rec,
        &mut c,
        &mut snap,
        &route,
        &mut options,
        |_| {},
    );
    assert!(matches!(outcome, TravelOutcome::GaveUp { hops: 2, .. }));
}

#[test]
fn follow_reports_leg_phases() {
    let mut c = scene_client();
    let mut snap = snap_at(&mut c, 0, 0);
    let mut rec = FollowRec {
        route: Some((0, 0)),
        ..FollowRec::default()
    };
    let mut t = Traveller::new();
    let route = Route {
        legs: vec![walk_leg(&[(3200, 3200), (3200, 3201), (3200, 3202)])],
        dest: WorldTile {
            x: 3200,
            z: 3202,
            level: 0,
        },
        ticks: 1.0, // 2 run steps at 0.5 ticks each
    };
    let mut phases = Vec::new();
    let mut options = TravelOptions {
        on_leg: Some(Box::new(|leg: &Leg, phase: LegPhase| {
            phases.push((leg.clone(), phase));
        })),
        ..TravelOptions::default()
    };
    let outcome = drive(
        &mut t,
        &mut rec,
        &mut c,
        &mut snap,
        &route,
        &mut options,
        |c| {
            plant_player(c, 0, 2);
        },
    );
    assert!(matches!(outcome, TravelOutcome::Arrived { .. }));
    // `options` borrows `phases` through the on_leg callback; dropping
    // it ends the borrow so the recording can be asserted.
    drop(options);
    assert_eq!(phases.len(), 2, "start + done for one leg");
    assert!(matches!(&phases[0], (Leg::Walk { .. }, LegPhase::Start)));
    assert!(matches!(&phases[1], (Leg::Walk { .. }, LegPhase::Done)));
}

#[test]
fn follow_blocks_when_the_transport_loc_is_missing() {
    // The player starts adjacent to the edge's `at`, so the run reaches
    // the loc lookup without an approach walk; loc id 99 is never
    // planted in the scene.
    let mut c = scene_client();
    let mut snap = snap_at(&mut c, 2, 3);
    let mut rec = FollowRec {
        route: Some((0, 0)),
        ..FollowRec::default()
    };
    let mut t = Traveller::new();
    // Loc id 99 is never planted in the scene.
    let edge = TransportEdge {
        takeoff: None,
        worn_all_req: Vec::new(),
        loc_id: 99,
        ..ladder_edge()
    };
    let route = Route {
        legs: vec![Leg::Transport {
            edge: Box::new(edge),
        }],
        dest: WorldTile {
            x: 3202,
            z: 3205,
            level: 0,
        },
        ticks: 2.0, // the ladder edge's ticks
    };
    let mut options = TravelOptions {
        budget_ticks_per_hop: 3,
        ..TravelOptions::default()
    };
    // The loc never appears: after the loc-wait budget, the leg blocks.
    let outcome = drive(
        &mut t,
        &mut rec,
        &mut c,
        &mut snap,
        &route,
        &mut options,
        |_| {},
    );
    assert!(matches!(
        outcome,
        TravelOutcome::Blocked { leg: 0, detail, .. } if detail.contains("99")
    ));
}

#[test]
fn follow_drives_walk_then_transport_legs_in_order() {
    let mut c = scene_client();
    plant_ladder(&mut c, Some("Climb"));
    let mut snap = snap_at(&mut c, 0, 0);
    let mut rec = FollowRec {
        route: Some((0, 0)),
        ..FollowRec::default()
    };
    let mut t = Traveller::new();
    let route = Route {
        legs: vec![
            // The walk leg ends adjacent to the ladder's `at`, so the
            // transport leg interacts on its first poll (no approach
            // walk) and the send counts stay per-leg.
            walk_leg(&[
                (3200, 3200),
                (3200, 3201),
                (3200, 3202),
                (3200, 3203),
                (3201, 3203),
            ]),
            Leg::Transport {
                edge: Box::new(ladder_edge()),
            },
        ],
        dest: WorldTile {
            x: 3202,
            z: 3205,
            level: 0,
        },
        ticks: 3.0, // 2 run steps (1.0) + the ladder edge's 2 ticks
    };
    let mut phases = Vec::new();
    let mut options = TravelOptions {
        on_leg: Some(Box::new(|leg: &Leg, phase: LegPhase| {
            phases.push((leg.clone(), phase));
        })),
        ..TravelOptions::default()
    };
    let mut step = 0;
    let outcome = drive(
        &mut t,
        &mut rec,
        &mut c,
        &mut snap,
        &route,
        &mut options,
        |c| {
            step += 1;
            match step {
                1 => plant_player(c, 1, 3), // reach the walk leg's end (adjacent to the ladder)
                _ => plant_player(c, 2, 5), // cross the transport to edge.to
            }
        },
    );
    assert!(matches!(
        outcome,
        TravelOutcome::Arrived { at } if at == WorldTile { x: 3202, z: 3205, level: 0 }
    ));
    assert_eq!(rec.walked.len(), 1, "one walk for the walk leg");
    assert_eq!(rec.loc_ops, 1, "one OP_LOC1 for the transport leg");
    // `options` borrows `phases` through the on_leg callback; dropping
    // it ends the borrow so the recording can be asserted.
    drop(options);
    assert_eq!(phases.len(), 4, "start/done per leg");
    assert!(matches!(&phases[0], (Leg::Walk { .. }, LegPhase::Start)));
    assert!(matches!(&phases[1], (Leg::Walk { .. }, LegPhase::Done)));
    assert!(matches!(
        &phases[2],
        (Leg::Transport { .. }, LegPhase::Start)
    ));
    assert!(matches!(
        &phases[3],
        (Leg::Transport { .. }, LegPhase::Done)
    ));
}

/// Recording driver for the follow tests: the build base matches the
/// fixture scene (3200, 3200), so loc scene coords translate like the
/// live client and `Interactions`' in-scene check passes. Walks are
/// recorded scene-relative (`dx, dz` from the route origin).
#[derive(Default)]
struct FollowRec {
    walked: Vec<(i32, i32)>,
    loc_ops: usize,
    /// `USEHELD_ONLOC` (oplocu): the web-knife arm.
    loc_uses: usize,
    npc_ops: usize,
    /// Held-item ops (OP_HELD1..=5): the jewellery rub arm's press.
    held_ops: usize,
    if_buttons: usize,
    /// The component ids pressed via IF_BUTTON, in order (the dialog
    /// ride-choice arm asserts *which* choice was answered).
    if_button_components: Vec<i32>,
    pause_buttons: usize,
    reject_far: bool,
    /// A scene tile the client cannot path onto (blocked live).
    reject_to: Option<(i32, i32)>,
    route: Option<(i32, i32)>,
    build_base: Option<(i32, i32)>,
    sink: Sink,
}

impl Driver for FollowRec {
    fn set_menu(&mut self, _slot: i32, action: i32, _a: i32, _b: i32, _c: i32) {
        match action {
            MiniMenuAction::OP_LOC1 => self.loc_ops += 1,
            MiniMenuAction::USEHELD_ONLOC => self.loc_uses += 1,
            MiniMenuAction::OP_NPC1
            | MiniMenuAction::OP_NPC2
            | MiniMenuAction::OP_NPC3
            | MiniMenuAction::OP_NPC4
            | MiniMenuAction::OP_NPC5 => self.npc_ops += 1,
            MiniMenuAction::OP_HELD1
            | MiniMenuAction::OP_HELD2
            | MiniMenuAction::OP_HELD3
            | MiniMenuAction::OP_HELD4
            | MiniMenuAction::OP_HELD5 => self.held_ops += 1,
            MiniMenuAction::IF_BUTTON => {
                self.if_buttons += 1;
                self.if_button_components.push(_c);
            }
            MiniMenuAction::PAUSE_BUTTON => self.pause_buttons += 1,
            _ => {}
        }
    }

    fn do_action(&mut self, _slot: i32) -> bool {
        true
    }

    fn try_move(
        &mut self,
        src_x: i32,
        src_z: i32,
        dx: i32,
        dz: i32,
        _try_nearest: bool,
        _loc_width: i32,
        _loc_length: i32,
        _loc_angle: i32,
        _loc_shape: i32,
        _forceapproach: i32,
        _ty: i32,
    ) -> bool {
        if self.reject_far && (src_x - dx).abs().max((src_z - dz).abs()) > 1 {
            return false;
        }
        if self.reject_to == Some((dx, dz)) {
            return false;
        }
        self.walked.push((dx, dz));
        true
    }

    fn local_route(&self) -> Option<(i32, i32)> {
        self.route
    }

    fn build_base(&self) -> (i32, i32) {
        self.build_base.unwrap_or((3200, 3200))
    }

    fn loc_typecode(&self, _scene_x: i32, _scene_z: i32) -> Option<i32> {
        None
    }

    fn out(&mut self) -> &mut dyn Out {
        &mut self.sink
    }

    fn login(&mut self, _username: &str, _password: &str, _reconnect: bool) -> bool {
        true
    }
}

#[test]
fn follow_ends_blocked_when_the_adjacent_route_end_refuses_every_click() {
    // Frozen `'blocked'` (WalkExecutor.ts:1039-1040, 1092-1094): no click of
    // this follow was accepted and the route's last tile, one step away, is
    // blocked live. After the frozen five idle ticks the follow ends as
    // close as reachable instead of watching forever.
    let mut c = scene_client();
    let mut snap = snap_at(&mut c, 0, 0);
    let mut rec = FollowRec {
        route: Some((0, 0)),
        reject_to: Some((0, 1)),
        ..FollowRec::default()
    };
    let mut t = Traveller::new();
    let route = Route {
        legs: vec![walk_leg(&[(3200, 3200), (3200, 3201)])],
        dest: WorldTile {
            x: 3200,
            z: 3201,
            level: 0,
        },
        ticks: 0.5,
    };
    let mut options = TravelOptions::default();
    // Repeated polls of one tick count once.
    for _ in 0..3 {
        assert!(t
            .follow(&mut rec, &snap, route.clone(), &mut options)
            .is_none());
    }
    for _ in 0..3 {
        bump_rebuild(&mut c, &mut snap);
        assert!(
            t.follow(&mut rec, &snap, route.clone(), &mut options)
                .is_none(),
            "inside the stall window the follow keeps watching"
        );
    }
    bump_rebuild(&mut c, &mut snap);
    assert_eq!(
        t.follow(&mut rec, &snap, route.clone(), &mut options),
        Some(TravelOutcome::Stalled {
            at: WorldTile {
                x: 3200,
                z: 3200,
                level: 0
            },
            aiming: WorldTile {
                x: 3200,
                z: 3201,
                level: 0
            },
            why: HopFailure::EndBlocked,
            tries: 0,
        }),
        "the fifth distinct refused tick ends the follow"
    );
    assert!(rec.walked.is_empty(), "no walk was ever accepted");
}

#[test]
fn follow_waits_across_polls_for_the_hop_to_arrive() {
    // Regression: a hop spanning several ticks must return `None` from
    // each still-waiting poll (the host re-polls next tick), never a
    // stall caused by re-polling the same snapshot within one call.
    let mut c = scene_client();
    let mut snap = snap_at(&mut c, 0, 0);
    let mut rec = FollowRec {
        route: Some((0, 0)),
        ..FollowRec::default()
    };
    let mut t = Traveller::new();
    let route = Route {
        legs: vec![walk_leg(&[
            (3200, 3200),
            (3200, 3201),
            (3200, 3202),
            (3200, 3203),
            (3200, 3204),
            (3200, 3205),
        ])],
        dest: WorldTile {
            x: 3200,
            z: 3205,
            level: 0,
        },
        ticks: 2.5, // 5 run steps at 0.5 ticks each
    };
    let mut options = TravelOptions {
        budget_ticks_per_hop: 10,
        ..TravelOptions::default()
    };
    // Poll 1: the run sends the hop's walk.
    assert!(t
        .follow(&mut rec, &snap, route.clone(), &mut options)
        .is_none());
    // Poll 2: the player is mid-leg — the settle must still be watching
    // (the call returns `None`), never a stall from an intra-call spin.
    plant_player(&mut c, 0, 2);
    bump_rebuild(&mut c, &mut snap);
    assert!(
        t.follow(&mut rec, &snap, route.clone(), &mut options)
            .is_none(),
        "a mid-walk poll must not stall"
    );
    // Poll 3: the player reaches the leg end — the run arrives.
    plant_player(&mut c, 0, 5);
    bump_rebuild(&mut c, &mut snap);
    match t.follow(&mut rec, &snap, route.clone(), &mut options) {
        Some(TravelOutcome::Arrived { at }) => {
            assert_eq!(
                at,
                WorldTile {
                    x: 3200,
                    z: 3205,
                    level: 0
                }
            )
        }
        other => panic!("expected Arrived, got {other:?}"),
    }
}

#[test]
fn follow_steps_a_long_walk_leg_hop_by_hop() {
    // A leg longer than close_enough needs several hops: the player
    // advances toward the end between polls and each matched hop arms
    // the next one until the last tile is reached.
    let mut c = scene_client();
    let mut snap = snap_at(&mut c, 0, 0);
    let mut rec = FollowRec {
        route: Some((0, 0)),
        ..FollowRec::default()
    };
    let mut t = Traveller::new();
    let tiles: Vec<(i32, i32)> = (0..40).map(|z| (3200, 3200 + z)).collect();
    let route = Route {
        legs: vec![walk_leg(&tiles)],
        dest: WorldTile {
            x: 3200,
            z: 3239,
            level: 0,
        },
        ticks: 19.5, // 39 run steps at 0.5 ticks each
    };
    let mut options = TravelOptions {
        close_enough: 3,
        ..TravelOptions::default()
    };
    let mut z = 0;
    loop {
        if let Some(outcome) = t.follow(&mut rec, &snap, route.clone(), &mut options) {
            assert!(matches!(outcome, TravelOutcome::Arrived { .. }));
            break;
        }
        z = (z + 15).min(39);
        plant_player(&mut c, 0, z);
        bump_rebuild(&mut c, &mut snap);
    }
    assert!(
        rec.walked.len() >= 2,
        "a long leg needs several hops, got {}",
        rec.walked.len()
    );
}

#[test]
fn follow_click_ahead_sends_the_next_hop_before_exact_stand() {
    // Mid-hop: standing Chebyshev 2 from the current aim must send
    // the next walk on the same poll (no extra tick of standing).
    let mut c = scene_client();
    let mut snap = snap_at(&mut c, 0, 0);
    let mut rec = FollowRec {
        route: Some((0, 0)),
        ..FollowRec::default()
    };
    let mut t = Traveller::new();
    let tiles: Vec<(i32, i32)> = (0..40).map(|z| (3200, 3200 + z)).collect();
    let route = Route {
        legs: vec![walk_leg(&tiles)],
        dest: WorldTile {
            x: 3200,
            z: 3239,
            level: 0,
        },
        ticks: 19.5,
    };
    let mut options = TravelOptions {
        close_enough: 0,
        ..TravelOptions::default()
    };
    assert!(t
        .follow(&mut rec, &snap, route.clone(), &mut options)
        .is_none());
    let first = rec.walked.len();
    assert_eq!(first, 1, "the opening hop went out");
    // 15-tile aim from (3200,3200) is z=15; stand at z=13 (cheb 2).
    plant_player(&mut c, 0, 13);
    bump_rebuild(&mut c, &mut snap);
    assert!(t
        .follow(&mut rec, &snap, route.clone(), &mut options)
        .is_none());
    assert!(
        rec.walked.len() > first,
        "click-ahead must send the next hop at cheb 2, got {}",
        rec.walked.len()
    );
}

#[test]
fn pick_aim_in_scene_clips_a_hop_that_would_leave_the_104() {
    let tiles: Vec<WorldTile> = (0..120)
        .map(|z| WorldTile {
            x: 3200,
            z: 3200 + z,
            level: 0,
        })
        .collect();
    let here = WorldTile {
        x: 3200,
        z: 3290,
        level: 0,
    };
    let scene = api::snapshot::SceneView {
        available: true,
        base_x: 3200,
        base_z: 3200,
        level: 0,
        width: 104,
        height: 104,
        collision_flags: vec![],
    };
    let (aim, _) = super::pick_aim_in_scene(&tiles, here, 0, &scene);
    let lz = aim.z - scene.base_z;
    assert!(
        (0..104).contains(&lz),
        "clipped aim must stay in the loaded scene, lz={lz}"
    );
    assert!(aim.z > here.z, "still aims forward");
}

#[test]
fn level_change_transport_requires_proximity_to_to() {
    // Regression: a level-changing transport completes only when the
    // player is within `close_enough` of `to` on the destination level
    // — never merely for standing anywhere on that level.
    let mut c = scene_client();
    c.minusedlevel = 1;
    let snap = snap_at(&mut c, 100, 100);
    let edge = TransportEdge {
        takeoff: None,
        worn_all_req: Vec::new(),
        kind: TransportKind::Ladder,
        player_delta: None,
        at: WorldTile {
            x: 3202,
            z: 3204,
            level: 0,
        },
        to: WorldTile {
            x: 3202,
            z: 3205,
            level: 1,
        },
        loc_id: 1,
        option: 1,
        ticks: 2,
        dir: None,
        open_loc_id: None,
        skill_req: vec![],
        item_req: vec![],
        consumed_req: vec![],
        item_returns: vec![],
        quest_req: vec![],
        varp_req: vec![],
        worn_req: vec![],
        members_req: false,
        wildy_cap: None,
        quest_gates: None,
    };
    let route = Route {
        legs: vec![Leg::Transport {
            edge: Box::new(edge),
        }],
        dest: WorldTile {
            x: 3202,
            z: 3205,
            level: 1,
        },
        ticks: 2.0, // the ladder edge's ticks
    };
    let mut options = TravelOptions::default();
    let mut run = FollowRun::start(route, &options);
    let leg = run.legs.pop_front().expect("the transport leg");
    let mut rec = FollowRec::default();
    run.transport = Some(TransportHop {
        leg,
        to: WorldTile {
            x: 3202,
            z: 3205,
            level: 1,
        },
        arrival_footprint: None,
        ticks_waited: 0,
        sent_tile: Some(WorldTile {
            x: 3200,
            z: 3200,
            level: 0,
        }),
        tries: 0,
        troll: false,
        npc_index: None,
        npc_recovery: super::NpcRecovery::default(),
        open_sent_tick: None,
        chat_seq: 0,
        dialog_page: None,
        approach: None,
    });
    // The player is on the destination level (1) but far from `to`:
    // the hop must still be watching.
    let mut no_session = None;
    assert!(matches!(
        run.poll_transport(&mut rec, &snap, &mut options, &mut no_session),
        Poll::Watching
    ));
    // Within close_enough of `to` on the destination level: the hop
    // completes the leg.
    plant_player(&mut c, 2, 5);
    c.bump_gens(ServerProt::REBUILD_NORMAL);
    let mut snap = GameSnapshot::new();
    snap.rebuild(&c);
    assert!(matches!(
        run.poll_transport(&mut rec, &snap, &mut options, &mut no_session),
        Poll::LegDone
    ));
}

#[test]
fn horizontal_climb_proves_translated_takeoff_without_widening_arrival() {
    let mut c = scene_client();
    let snap = snap_at(&mut c, 11, 33);
    let anchor = WorldTile {
        x: 3202,
        z: 3204,
        level: 0,
    };
    let baked_to = WorldTile {
        x: 3212,
        z: 3234,
        level: 0,
    };
    let landing = WorldTile {
        x: 3210,
        z: 3233,
        level: 0,
    };
    let edge = TransportEdge {
        takeoff: None,
        worn_all_req: Vec::new(),
        kind: TransportKind::Ladder,
        player_delta: Some(WorldTile {
            x: 10,
            z: 30,
            level: 0,
        }),
        at: anchor,
        to: baked_to,
        loc_id: 1,
        option: 1,
        ticks: 2,
        dir: None,
        open_loc_id: None,
        skill_req: vec![],
        item_req: vec![],
        consumed_req: vec![],
        item_returns: vec![],
        quest_req: vec![],
        varp_req: vec![],
        worn_req: vec![],
        members_req: false,
        wildy_cap: None,
        quest_gates: None,
    };
    let mut options = TravelOptions {
        close_enough: 0,
        ..TravelOptions::default()
    };
    let mut run = FollowRun::start(
        Route {
            legs: vec![Leg::Transport {
                edge: Box::new(edge),
            }],
            dest: landing,
            ticks: 2.0,
        },
        &options,
    );
    let leg = run.legs.pop_front().unwrap();
    run.transport = Some(TransportHop {
        leg,
        to: baked_to,
        arrival_footprint: None,
        ticks_waited: 0,
        sent_tile: Some(WorldTile {
            x: 3200,
            z: 3203,
            level: 0,
        }),
        tries: 0,
        troll: false,
        npc_index: None,
        npc_recovery: super::NpcRecovery::default(),
        open_sent_tick: None,
        chat_seq: 0,
        dialog_page: None,
        approach: None,
    });
    let mut rec = FollowRec::default();
    let mut no_session = None;
    // A tile beside either predicted landing is not an arrival at radius 0.
    assert!(matches!(
        run.poll_transport(&mut rec, &snap, &mut options, &mut no_session),
        Poll::Watching
    ));
    // The route's nominal/planned `to` is not proof of this relative jump.
    let snap = snap_at(&mut c, 12, 34);
    assert!(matches!(
        run.poll_transport(&mut rec, &snap, &mut options, &mut no_session),
        Poll::Watching
    ));
    let snap = snap_at(&mut c, 10, 33);
    assert!(matches!(
        run.poll_transport(&mut rec, &snap, &mut options, &mut no_session),
        Poll::LegDone
    ));
}

/// Recording driver: captures the last walk target and counts OP_LOC1
/// interactions. `route` stands in for the local player tile so
/// `api::walk` finds a route origin. `reject_far` mirrors the live
/// client rejecting a tryMove shot of more than one tile.
#[derive(Default)]
struct Rec {
    walked: Option<(i32, i32)>,
    locs: usize,
    route: Option<(i32, i32)>,
    reject_far: bool,
    sink: Sink,
}

impl Driver for Rec {
    fn set_menu(&mut self, _slot: i32, action: i32, _a: i32, _b: i32, _c: i32) {
        if action == MiniMenuAction::OP_LOC1 {
            self.locs += 1;
        }
    }

    fn do_action(&mut self, _slot: i32) -> bool {
        true
    }

    fn try_move(
        &mut self,
        src_x: i32,
        src_z: i32,
        dx: i32,
        dz: i32,
        _try_nearest: bool,
        _loc_width: i32,
        _loc_length: i32,
        _loc_angle: i32,
        _loc_shape: i32,
        _forceapproach: i32,
        _ty: i32,
    ) -> bool {
        if self.reject_far && (src_x - dx).abs().max((src_z - dz).abs()) > 1 {
            return false;
        }
        self.walked = Some((dx, dz));
        true
    }

    fn local_route(&self) -> Option<(i32, i32)> {
        self.route
    }

    fn build_base(&self) -> (i32, i32) {
        (0, 0)
    }

    fn loc_typecode(&self, _scene_x: i32, _scene_z: i32) -> Option<i32> {
        None
    }

    fn out(&mut self) -> &mut dyn Out {
        &mut self.sink
    }

    fn login(&mut self, _username: &str, _password: &str, _reconnect: bool) -> bool {
        true
    }
}

/// Minimal outbound sink: records the `pjstr` payloads (the `::tele`
/// cheat writes its body through `pjstr`, so a non-empty `strings`
/// proves a cheat was queued — the teleport-arm tests assert empty).
#[derive(Default)]
struct Sink {
    strings: Vec<String>,
    steps: Vec<i32>,
}

impl Out for Sink {
    fn p1_enc(&mut self, opcode: i32) {
        self.steps.push(opcode);
    }
    fn p1(&mut self, value: i32) {
        self.steps.push(value);
    }
    fn p2(&mut self, value: i32) {
        self.steps.push(value);
    }
    fn p4(&mut self, _value: i32) {}
    fn pjstr(&mut self, s: &str) {
        self.strings.push(s.to_string());
    }
}
