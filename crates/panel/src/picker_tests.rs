use api::snapshot::WorldTile;
use client::dash3d::CollisionFlag;
use nav::collision::WorldCollision;
use nav::tile::Tile;
use nav::transport::{TransportEdge, TransportGraph, TransportKind};
use nav::world::NavWorld;

use std::sync::Arc;

use super::{
    available_levels, click_to_tile, decode_sidecar_file, drop_flags_sidecar, ensure_flags_sidecar,
    flags_content_hash_count, flags_sidecar_for, flags_sidecar_state, pack, pack_map_tiles, pan_by,
    picker_map_window, reach_bitset, reset_flags_content_hash_count, right_align_x,
    set_navflags_binding, set_pack, set_reach_binding, sidecar_for_grid, snap, walkto_canvas_flags,
    walkto_footer_labels, walkto_window_flags, FlagSidecar, FlagsSidecarState, PackView,
};
use crate::nav_settings::NavSettings;
use crate::session::Session;
use dear_imgui_rs::WindowFlags;
use nav::router::{Leg, Route};
use std::sync::Mutex as StdMutex;

/// Process-global flags binding/hash counters; serialize tests that touch them.
static FLAGS_TEST_LOCK: StdMutex<()> = StdMutex::new(());
/// Process-global reach binding; serialize tests that touch it.
static REACH_TEST_LOCK: StdMutex<()> = StdMutex::new(());

/// A `w`×`h` all-walkable level-0 world at (0,0).
fn open_world(w: usize, h: usize) -> NavWorld {
    NavWorld::from_parts(
        WorldCollision {
            origin: WorldTile {
                x: 0,
                z: 0,
                level: 0,
            },
            width: w,
            height: h,
            walk: vec![0u8; w * h],
            blocked: vec![0u64; (w * h).div_ceil(64)],
            flags: None,
        },
        TransportGraph::default(),
        Vec::new(),
    )
}

#[test]
fn snap_click_to_nearest_walkable() {
    let w = open_world(3, 3);
    let t = snap(&w, 1.4, 1.4, 0).unwrap();
    assert_eq!(
        t,
        Tile {
            x: 1,
            z: 1,
            level: 0
        }
    );
}

#[test]
fn snap_wall_click_lands_on_nearest_walkable() {
    // The world's x=2 tile is a wall; (1,0) wins the Chebyshev/Manhattan
    // tie over (3,0) by iteration order.
    let mut flags = vec![0u32; 5];
    flags[2] = CollisionFlag::WALK_BLOCK_FLAGS as u32;
    let (walk, blocked) = nav::collision::pack_walk(&flags);
    let w = NavWorld::from_parts(
        WorldCollision {
            origin: WorldTile {
                x: 0,
                z: 0,
                level: 0,
            },
            width: 5,
            height: 1,
            walk,
            blocked,
            flags: None,
        },
        TransportGraph::default(),
        Vec::new(),
    );
    let t = snap(&w, 2.2, 0.1, 0).unwrap();
    assert_eq!(
        t,
        Tile {
            x: 1,
            z: 0,
            level: 0
        }
    );
}

#[test]
fn snap_returns_none_on_level_without_walkables() {
    let w = open_world(3, 3);
    assert_eq!(snap(&w, 1.4, 1.4, 1), None);
}

#[test]
fn available_levels_lists_planes_with_content() {
    let w = open_world(3, 3);
    // An all-walkable single-plane world lists the ground plane only.
    assert_eq!(available_levels(&w), vec![0]);
    // A 4-plane world with a stamped level-1 plane lists it too.
    let mut flags = vec![0u32; 4 * 9];
    flags[9 + 4] = CollisionFlag::WALK_SCENERY as u32;
    let (walk, blocked) = nav::collision::pack_walk(&flags);
    let w2 = NavWorld::from_parts(
        WorldCollision {
            origin: WorldTile {
                x: 0,
                z: 0,
                level: 0,
            },
            width: 3,
            height: 3,
            walk,
            blocked,
            flags: None,
        },
        TransportGraph::default(),
        Vec::new(),
    );
    assert_eq!(available_levels(&w2), vec![0, 1]);
}

#[test]
fn click_to_tile_maps_canvas_click_through_centre_and_scale() {
    let w = open_world(3, 3);
    // Canvas centre is the centre tile.
    let t = click_to_tile(&w, (1, 1), 10.0, [50.0, 50.0], [100.0, 100.0], 0).unwrap();
    assert_eq!(
        t,
        Tile {
            x: 1,
            z: 1,
            level: 0
        }
    );
    // 14px right of centre at 10px/tile -> east (higher x).
    let t = click_to_tile(&w, (1, 1), 10.0, [64.0, 50.0], [100.0, 100.0], 0).unwrap();
    assert_eq!(
        t,
        Tile {
            x: 2,
            z: 1,
            level: 0
        }
    );
    // 20px below centre is south (lower z): north is up on the canvas.
    let t = click_to_tile(&w, (1, 1), 10.0, [50.0, 70.0], [100.0, 100.0], 0).unwrap();
    assert_eq!(
        t,
        Tile {
            x: 1,
            z: 0,
            level: 0
        }
    );
}

#[test]
fn pan_by_accumulates_sub_tile_remainder_at_fine_and_coarse_zoom() {
    // 2px/tile: a 3px drag is 1.5 tiles; the whole tile lands now and the
    // half tile stays in the remainder. Sign pins "content follows the
    // cursor": positive mouse dx pans west (smaller centre.x), matching
    // the existing `CENTRE_X -= delta/scale`.
    let ((cx, _cz), rem) = pan_by((3200, 3200), (0.0, 0.0), -3.0, 0.0, 2.0);
    assert_eq!(cx, 3201);
    assert!((rem.0 - 0.5).abs() < 1e-5);
    let ((cx, _), rem) = pan_by((3200, 3200), (0.0, 0.0), 3.0, 0.0, 2.0);
    assert_eq!(cx, 3199);
    assert!((rem.0 + 0.5).abs() < 1e-5);
    // The leftover half tile plus another 1px (0.5 tile) completes a
    // second tile.
    let ((cx2, _), rem2) = pan_by((cx, 3200), rem, 1.0, 0.0, 2.0);
    assert_eq!(cx2, 3198);
    assert!(rem2.0.abs() < 1e-5);
    // 16px/tile: an 8px drag is 0.5 tile — the centre must not stay stuck
    // waiting for a full tile.
    let (c, rem) = pan_by((3200, 3200), (0.0, 0.0), 8.0, 0.0, 16.0);
    assert_eq!(c.0, 3200);
    assert!(rem.0.abs() > 0.4);
}

#[test]
fn walkto_window_flags_capture_wheel_on_the_canvas() {
    let w = walkto_window_flags();
    assert!(
        w.contains(WindowFlags::NO_SCROLLBAR),
        "hide the imgui window scrollbar"
    );
    assert!(
        w.contains(WindowFlags::NO_SCROLL_WITH_MOUSE),
        "wheel over WalkTo must pan the map, not scroll the window"
    );
    let c = walkto_canvas_flags();
    assert!(c.contains(WindowFlags::NO_SCROLLBAR));
    assert!(c.contains(WindowFlags::NO_SCROLL_WITH_MOUSE));
}

#[test]
fn right_align_x_sits_the_cluster_on_the_content_edge() {
    // cursor 12, 400px remaining, 80px cluster → 332.
    assert_eq!(right_align_x(12.0, 400.0, 80.0), 332.0);
}

#[test]
fn walkto_footer_adds_teleport_on_local_engine() {
    assert_eq!(walkto_footer_labels(false), &["recentre", "Walk"]);
    assert_eq!(
        walkto_footer_labels(true),
        &["recentre", "Walk", "Teleport"]
    );
}

#[test]
fn picker_map_window_builds_headless() {
    let _guard = crate::IMGUI_CTX_TEST_GUARD.lock().unwrap();
    let mut ctx = dear_imgui_rs::Context::create();
    ctx.prepare_frame(
        dear_imgui_rs::FramePrepareOptions::new([900.0, 700.0], 1.0 / 60.0).renderer_has_textures(),
    );
    let ui = ctx.frame();
    let mut s = Session::new();
    s.walkto_open = true;
    let mut open = true;
    picker_map_window(ui, &mut s, &open_world(3, 3), &mut open);
    ctx.render();
    assert!(open, "the window must stay open until Walk is confirmed");
}

fn picker_click_frame(
    ctx: &mut dear_imgui_rs::Context,
    session: &mut Session,
    world: &NavWorld,
    mouse: [f32; 2],
    left_down: bool,
) {
    ctx.prepare_frame(
        dear_imgui_rs::FramePrepareOptions::new([900.0, 700.0], 1.0 / 60.0).renderer_has_textures(),
    );
    ctx.io_mut().add_mouse_pos_event(mouse);
    ctx.io_mut()
        .add_mouse_button_event(dear_imgui_rs::MouseButton::Left, left_down);
    {
        let ui = ctx.frame();
        let mut open = true;
        picker_map_window(ui, session, world, &mut open);
    }
    ctx.render();
}

#[test]
fn picker_click_selects_a_walkable_tile() {
    let _guard = crate::IMGUI_CTX_TEST_GUARD.lock().unwrap();
    super::note_closed();
    let mut ctx = dear_imgui_rs::Context::create();
    let world = open_world(3, 3);
    let mut s = Session::new();
    s.walkto_open = true;
    // FirstUseEver WalkTo is 720×560 at the default imgui origin; the
    // canvas sits under the toolbar. A click in the window interior
    // must set picker_sel — not pan, not miss the hit target.
    let mouse = [360.0, 320.0];
    picker_click_frame(&mut ctx, &mut s, &world, mouse, false);
    picker_click_frame(&mut ctx, &mut s, &world, mouse, true);
    picker_click_frame(&mut ctx, &mut s, &world, mouse, false);
    assert!(
        s.picker_sel.is_some(),
        "click on the WalkTo canvas must snap a tile, got {:?}",
        s.picker_sel
    );
}

/// A `w`×`h` level-0 bake at (0,0) with the given per-tile flags OR'd in.
fn bake_world(w: usize, h: usize, extras: &[(i32, i32, u32)]) -> NavWorld {
    let mut flags = vec![0u32; w * h];
    for &(x, z, f) in extras {
        flags[z as usize * w + x as usize] |= f;
    }
    let (walk, blocked) = nav::collision::pack_walk(&flags);
    NavWorld::from_parts(
        WorldCollision {
            origin: WorldTile {
                x: 0,
                z: 0,
                level: 0,
            },
            width: w,
            height: h,
            walk,
            blocked,
            flags: None,
        },
        TransportGraph::default(),
        Vec::new(),
    )
}

/// A 7×7 bake: a 3×3 open corner plus an isolated open tile moated by
/// WR_GRND; everything else blocked ground.
fn disconnected_world() -> NavWorld {
    let mut extras = Vec::new();
    for z in 0..7 {
        for x in 0..7 {
            let open = (x < 3 && z < 3) || (x == 5 && z == 5);
            if !open {
                extras.push((x, z, CollisionFlag::WR_GRND as u32));
            }
        }
    }
    bake_world(7, 7, &extras)
}

#[test]
fn pack_map_collision_only_in_viewport() {
    // A 5-wide bake with a WR_GRND wall at x=2; the small view covers
    // the wall and its open neighbour.
    let world = bake_world(5, 1, &[(2, 0, CollisionFlag::WR_GRND as u32)]);
    let view = PackView {
        x0: 1,
        z0: 0,
        width: 2,
        height: 1,
    };
    let layers = NavSettings {
        collision_fill: true,
        ..Default::default()
    };
    let tiles = pack_map_tiles(&world, view, None, None, None, &layers, 0);
    let in_view = |t: super::Tile| {
        t.x >= view.x0
            && t.x < view.x0 + view.width
            && t.z >= view.z0
            && t.z < view.z0 + view.height
    };
    assert!(tiles.iter().all(|t| in_view(t.tile)));
    assert!(tiles.iter().any(|t| t.blocked));
}

#[test]
fn pack_map_paints_the_selected_plane() {
    // A 3x3 two-plane world: level 0 is open, level 1 carries a WR_GRND
    // wall at (1,1). Painting must read the passed level, not the bake's
    // origin plane.
    let mut flags = vec![0u32; 2 * 9];
    flags[9 + 3 + 1] = CollisionFlag::WR_GRND as u32;
    let (walk, blocked) = nav::collision::pack_walk(&flags);
    let world = NavWorld::from_parts(
        WorldCollision {
            origin: WorldTile {
                x: 0,
                z: 0,
                level: 0,
            },
            width: 3,
            height: 3,
            walk,
            blocked,
            flags: None,
        },
        TransportGraph::default(),
        Vec::new(),
    );
    let view = PackView {
        x0: 0,
        z0: 0,
        width: 3,
        height: 3,
    };
    let layers = NavSettings {
        collision_fill: true,
        ..Default::default()
    };
    let tiles = pack_map_tiles(&world, view, None, None, None, &layers, 1);
    assert!(
        tiles.iter().all(|p| p.tile.level == 1),
        "paints live on the selected plane, not the bake's origin plane"
    );
    assert!(
        tiles
            .iter()
            .any(|p| p.blocked && p.tile.x == 1 && p.tile.z == 1),
        "the level-1 wall blocks its own plane"
    );
}

#[test]
fn pack_map_flood_marks_two_components() {
    let layers = NavSettings {
        component_flood: true,
        ..Default::default()
    };
    let tiles = pack_map_tiles(
        &disconnected_world(),
        PackView {
            x0: 0,
            z0: 0,
            width: 7,
            height: 7,
        },
        None,
        Some(WorldTile {
            x: 0,
            z: 0,
            level: 0,
        }),
        Some(WorldTile {
            x: 5,
            z: 5,
            level: 0,
        }),
        &layers,
        0,
    );
    let ids: std::collections::HashSet<_> = tiles.iter().filter_map(|t| t.flood).collect();
    assert_eq!(ids.len(), 2, "the player and dest components both flood");
}

#[test]
fn pack_map_marks_walkable_unreached_puddle() {
    let _guard = REACH_TEST_LOCK.lock().unwrap();
    set_reach_binding(
        None,
        WorldTile {
            x: 0,
            z: 0,
            level: 0,
        },
        0,
        0,
        false,
    );
    // 5×5 with a sealed 1×1 courtyard at (2,2) (all W_* faces) and one
    // door edge outside it: the reach BFS floods the open ground but
    // never the courtyard, so only the courtyard tile paints
    // walkable-unreached.
    let base = bake_world(5, 5, &[(2, 2, CollisionFlag::WALK_BLOCK_FLAGS as u32)]);
    let banks = base.banks().to_vec();
    let world = NavWorld::from_parts(
        base.collision,
        TransportGraph {
            edges: vec![TransportEdge {
                kind: TransportKind::Door,
                at: WorldTile {
                    x: 0,
                    z: 0,
                    level: 0,
                },
                to: WorldTile {
                    x: 4,
                    z: 4,
                    level: 0,
                },
                loc_id: 1530,
                option: 1,
                ticks: 1,
                dir: None,
                open_loc_id: None,
                skill_req: vec![],
                item_req: vec![],
                quest_req: vec![],
                varp_req: vec![],
                worn_req: vec![],
                members_req: false,
                wildy_cap: None,
            }],
            ..Default::default()
        },
        banks,
    );
    let view = PackView {
        x0: 0,
        z0: 0,
        width: 5,
        height: 5,
    };
    let tiles = pack_map_tiles(&world, view, None, None, None, &NavSettings::default(), 0);
    let courtyard = tiles
        .iter()
        .find(|t| t.tile.x == 2 && t.tile.z == 2)
        .expect("the sealed courtyard paints unreached");
    assert!(
        courtyard.unreached,
        "the puddle floor is walkable-unreached"
    );
    assert!(
        !courtyard.blocked && !courtyard.path && courtyard.flood.is_none(),
        "the puddle is a plain unreached tile"
    );
    assert!(
        tiles
            .iter()
            .all(|t| (t.tile.x == 2 && t.tile.z == 2) || !t.unreached),
        "only the sealed courtyard is unreached"
    );
    assert!(
        !tiles.iter().any(|t| t.tile.x == 0 && t.tile.z == 0),
        "reached ground stays off the reach layer"
    );
}

#[test]
fn pack_map_marks_remaining_path_in_view() {
    let world = bake_world(5, 1, &[]);
    let tiles: Vec<WorldTile> = (0..5).map(|x| WorldTile { x, z: 0, level: 0 }).collect();
    let route = Route {
        dest: tiles[4],
        legs: vec![Leg::Walk {
            tiles: tiles.clone(),
        }],
        ticks: 0.0,
    };
    let layers = NavSettings {
        show_nav_path: true,
        ..Default::default()
    };
    let view = PackView {
        x0: 0,
        z0: 0,
        width: 5,
        height: 1,
    };
    let marks = pack_map_tiles(&world, view, Some(&route), None, None, &layers, 0);
    let path: Vec<i32> = marks.iter().filter(|t| t.path).map(|t| t.tile.x).collect();
    assert_eq!(path, vec![0, 1, 2, 3, 4]);
    assert!(
        marks.iter().all(|t| !t.transport),
        "a walk-only route has no hops"
    );
}

#[test]
fn pack_map_flood_report_line_reports_each_arm() {
    let player = WorldTile {
        x: 0,
        z: 0,
        level: 0,
    };
    let dest = WorldTile {
        x: 5,
        z: 5,
        level: 0,
    };
    let first = super::flood_report_line(None, 1, player, dest, 9, 1).unwrap();
    assert_eq!(first, "nav-flood: player 9 dest 1");
    // A second arm with the same tiles still reports: the arm
    // generation is part of the report key.
    let second =
        super::flood_report_line(Some((1, player, dest, 9, 1)), 2, player, dest, 9, 1).unwrap();
    assert_eq!(second, first);
    // The same arm and sizes do not re-report; a size change does.
    assert!(
        super::flood_report_line(Some((2, player, dest, 9, 1)), 2, player, dest, 9, 1).is_none()
    );
    assert!(
        super::flood_report_line(Some((2, player, dest, 9, 1)), 2, player, dest, 9, 9).is_some()
    );
}

#[test]
fn pack_map_flood_report_sizes_from_cached_sets() {
    let world = disconnected_world();
    let player = WorldTile {
        x: 0,
        z: 0,
        level: 0,
    };
    let dest = WorldTile {
        x: 5,
        z: 5,
        level: 0,
    };
    let comps = super::flood_sets_for(&world, &[player, dest]);
    assert_eq!(
        super::flood_report_sizes(&comps, dest),
        (9, 1),
        "the 3x3 corner and the isolated dest tile"
    );
    // Connected seeds (the dest inside the player's flood) share one size.
    let inside = WorldTile {
        x: 1,
        z: 1,
        level: 0,
    };
    let comps = super::flood_sets_for(&world, &[player, inside]);
    assert_eq!(super::flood_report_sizes(&comps, inside), (9, 9));
}

#[test]
fn sidecar_file_roundtrips_and_rejects_garbage() {
    let path = std::env::temp_dir().join(format!(
        "274bot-panel-sidecar-{}.navflags",
        std::process::id()
    ));
    let flags = vec![
        CollisionFlag::W_N as u32 | CollisionFlag::WR_GRND as u32,
        CollisionFlag::W_E as u32,
        0,
        CollisionFlag::WALK_SCENERY as u32,
    ];
    let bytes = nav::pack::encode_flags_sidecar(
        WorldTile {
            x: 3200,
            z: 3200,
            level: 0,
        },
        1,
        1,
        &flags,
    );
    std::fs::write(&path, &bytes).unwrap();
    let s = decode_sidecar_file(&path).expect("valid sidecar decodes");
    assert_eq!(
        s.origin,
        WorldTile {
            x: 3200,
            z: 3200,
            level: 0
        }
    );
    assert_eq!((s.width, s.height), (1, 1));
    assert_eq!(&*s.flags, &flags[..]);
    // Garbage and missing files fall back to the walk word, never panic.
    std::fs::write(&path, b"not a sidecar").unwrap();
    assert!(decode_sidecar_file(&path).is_none());
    std::fs::remove_file(&path).ok();
    assert!(decode_sidecar_file(&path).is_none());
}

#[test]
fn sidecar_flags_only_apply_to_the_matching_grid() {
    let s = FlagSidecar {
        origin: WorldTile {
            x: 3200,
            z: 3200,
            level: 0,
        },
        width: 64,
        height: 64,
        flags: vec![0u32; 4 * 64 * 64].into(),
    };
    assert!(
        sidecar_for_grid(
            &s,
            WorldTile {
                x: 3200,
                z: 3200,
                level: 0
            },
            64,
            64
        )
        .is_some(),
        "a matching grid header applies the sidecar"
    );
    assert!(
        sidecar_for_grid(
            &s,
            WorldTile {
                x: 0,
                z: 0,
                level: 0
            },
            64,
            64
        )
        .is_none(),
        "a foreign origin must not paint from a stale sidecar"
    );
    assert!(sidecar_for_grid(
        &s,
        WorldTile {
            x: 3200,
            z: 3200,
            level: 0
        },
        64,
        65
    )
    .is_none());
}

#[test]
fn picker_pack_uses_injected_arc_not_a_second_decode() {
    let world = Arc::new(bake_world(1, 1, &[]));
    set_pack(Some(Arc::clone(&world)));
    let p = pack().expect("injected");
    assert!(
        Arc::ptr_eq(&p, &world),
        "pack() must hand back the injected Arc, not a re-decode"
    );
    // Leave the global detached so parallel tests cannot observe it.
    set_pack(None);
    assert!(pack().is_none(), "set_pack(None) must detach the world");
}

#[test]
fn flags_sidecar_requires_exact_identity_and_drops_on_toggle_off() {
    let _guard = FLAGS_TEST_LOCK.lock().unwrap();
    let path = std::env::temp_dir().join(format!(
        "274bot-panel-flags-identity-{}.navflags",
        std::process::id()
    ));
    let origin = WorldTile {
        x: 3200,
        z: 3200,
        level: 0,
    };
    let flags = vec![CollisionFlag::W_N as u32, 0, 0, 0];
    let bytes = nav::pack::encode_flags_sidecar(origin, 1, 1, &flags);
    std::fs::write(&path, &bytes).unwrap();
    let digest = nav::manifest::hash_bytes(&bytes);

    reset_flags_content_hash_count();
    set_navflags_binding(path.clone(), None, false);
    ensure_flags_sidecar();
    assert_eq!(
        flags_sidecar_state(),
        FlagsSidecarState::Refused("flags identity is unknown")
    );
    assert!(flags_sidecar_for(origin, 1, 1).is_none());
    assert_eq!(flags_content_hash_count(), 0);

    drop_flags_sidecar();
    set_navflags_binding(path.clone(), Some("00".repeat(32)), false);
    ensure_flags_sidecar();
    assert_eq!(
        flags_sidecar_state(),
        FlagsSidecarState::Refused("flags sidecar hash mismatch")
    );
    assert!(flags_sidecar_for(origin, 1, 1).is_none());
    assert_eq!(flags_content_hash_count(), 1);

    drop_flags_sidecar();
    set_navflags_binding(path.clone(), Some(digest), false);
    ensure_flags_sidecar();
    assert_eq!(flags_sidecar_state(), FlagsSidecarState::Applied);
    assert_eq!(
        flags_sidecar_for(origin, 1, 1).as_deref(),
        Some(flags.as_slice())
    );
    assert_eq!(flags_content_hash_count(), 2);
    drop_flags_sidecar();
    assert_eq!(flags_sidecar_state(), FlagsSidecarState::Unloaded);
    assert!(flags_sidecar_for(origin, 1, 1).is_none());
    let _ = std::fs::remove_file(path);
}

#[test]
fn bundled_flags_decode_without_content_hash_and_reuse_sidecar() {
    let _guard = FLAGS_TEST_LOCK.lock().unwrap();
    let path = std::env::temp_dir().join(format!(
        "274bot-panel-flags-bundled-{}.navflags",
        std::process::id()
    ));
    let origin = WorldTile {
        x: 3200,
        z: 3200,
        level: 0,
    };
    let flags = vec![CollisionFlag::W_N as u32, 0, 0, 0];
    let bytes = nav::pack::encode_flags_sidecar(origin, 1, 1, &flags);
    std::fs::write(&path, &bytes).unwrap();
    // Digest is still bound (identity names flags) but must not be rehashed.
    let digest = nav::manifest::hash_bytes(&bytes);

    reset_flags_content_hash_count();
    set_navflags_binding(path.clone(), Some(digest.clone()), true);
    ensure_flags_sidecar();
    assert_eq!(flags_sidecar_state(), FlagsSidecarState::Applied);
    assert_eq!(
        flags_sidecar_for(origin, 1, 1).as_deref(),
        Some(flags.as_slice())
    );
    assert_eq!(
        flags_content_hash_count(),
        0,
        "trusted bundled flags must not content-hash on first paint"
    );

    // Per-session reuse: second ensure is a no-op and still does not hash.
    ensure_flags_sidecar();
    assert_eq!(flags_content_hash_count(), 0);
    assert_eq!(
        flags_sidecar_for(origin, 1, 1).as_deref(),
        Some(flags.as_slice())
    );

    // Geometry mismatch still refuses paint application without a rehash.
    assert!(flags_sidecar_for(origin, 2, 1).is_none());
    assert_eq!(flags_content_hash_count(), 0);

    drop_flags_sidecar();
    // Explicit same-path override remains external and must hash.
    set_navflags_binding(path.clone(), Some(digest), false);
    ensure_flags_sidecar();
    assert_eq!(flags_sidecar_state(), FlagsSidecarState::Applied);
    assert_eq!(flags_content_hash_count(), 1);

    drop_flags_sidecar();
    set_navflags_binding(path.clone(), Some("00".repeat(32)), false);
    ensure_flags_sidecar();
    assert_eq!(
        flags_sidecar_state(),
        FlagsSidecarState::Refused("flags sidecar hash mismatch")
    );
    assert_eq!(flags_content_hash_count(), 2);

    // Malformed header refused after trusted bind without hashing.
    drop_flags_sidecar();
    std::fs::write(&path, b"not-a-flags-sidecar").unwrap();
    set_navflags_binding(path.clone(), Some("ab".repeat(32)), true);
    ensure_flags_sidecar();
    assert_eq!(
        flags_sidecar_state(),
        FlagsSidecarState::Refused("flags sidecar is unreadable")
    );
    assert_eq!(flags_content_hash_count(), 2);

    let _ = std::fs::remove_file(path);
}

#[test]
fn bundled_reach_is_shared_without_flood_on_first_or_second_paint() {
    let _guard = REACH_TEST_LOCK.lock().unwrap();
    let mut world = bake_world(3, 3, &[]);
    world.collision.origin.x = 7777;
    let origin = world.collision.origin;
    let width = world.collision.width;
    let height = world.collision.height;
    let expected: Arc<[u64]> = nav::paint::bake_reach(&world.collision, &world.graph).into();
    nav::paint::reset_bake_reach_calls();
    set_reach_binding(Some(Arc::clone(&expected)), origin, width, height, true);
    let first = reach_bitset(&world).expect("bundled bits");
    let second = reach_bitset(&world).expect("slot reuse");
    assert!(Arc::ptr_eq(&first, &second));
    assert_eq!(&*first, &*expected);
    assert_eq!(
        nav::paint::bake_reach_calls(),
        0,
        "bundled first/second paint must not flood"
    );
    let mut other = open_world(3, 3);
    other.collision.origin.x = 99;
    assert!(
        reach_bitset(&other).is_none(),
        "same-size different origin must not reuse bundled bits"
    );
    assert_eq!(nav::paint::bake_reach_calls(), 0);
    set_reach_binding(None, origin, 0, 0, false);
}

#[test]
fn external_reach_floods_once_and_reuses_the_arc() {
    let _guard = REACH_TEST_LOCK.lock().unwrap();
    set_reach_binding(
        None,
        WorldTile {
            x: 0,
            z: 0,
            level: 0,
        },
        0,
        0,
        false,
    );
    let world = bake_world(3, 3, &[]);
    nav::paint::reset_bake_reach_calls();
    let first = reach_bitset(&world).expect("external bake");
    let second = reach_bitset(&world).expect("cached");
    assert!(Arc::ptr_eq(&first, &second));
    assert_eq!(nav::paint::bake_reach_calls(), 1);
    set_pack(None);
    let _ = reach_bitset(&world);
    assert_eq!(
        nav::paint::bake_reach_calls(),
        2,
        "set_pack drops the computed cache"
    );
}
