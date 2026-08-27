//! Schematic nav-path overlay for the Game Image.
//!
//! The 274 client renders a perspective 3D view, so there is no exact
//! tile→pixel transform. The overlay treats the Image as a flat tile grid
//! centred on the focused slot's observed tile and rebuilds its polyline
//! vertices at the 1 s raster cadence (or immediately when a new route
//! arms) instead of every 50 fps UI frame.

use std::time::{Duration, Instant};

use api::snapshot::WorldTile;
use dear_imgui_rs::Ui;
use nav::router::{Leg, Route};
use nav::tile::Tile;

use crate::queue_card::{queue_ahead_label, queue_k_of_n, QUEUE_CARD_TITLE};
use crate::session::Session;
use crate::theme::ACCENT;

/// How often the overlay recomputes its vertices: the watch-only raster
/// cadence (50 ticks × 20 ms).
const REFRESH: Duration = Duration::from_millis(1000);

/// The Image is treated as a flat tile grid centred on the player. The
/// client culls world rendering past ±26 tiles from the camera, so the
/// grid is 52 tiles wide, aspect-corrected to the 765×503 applet.
const VIEW_TILES_W: i32 = 52;
const VIEW_TILES_H: i32 = 34;

/// Map a world tile to image-local pixels: the inverse of
/// [`host::map_image_to_applet`] conceptually (applet pixel → world tile is
/// the forward direction; here a tile lands on the centre of its cell in a
/// `tiles_w`×`tiles_h` grid whose top-left tile is `origin`).
pub fn tile_to_local(
    t: Tile,
    origin: Tile,
    tiles_w: i32,
    tiles_h: i32,
    img_w: f32,
    img_h: f32,
) -> [f32; 2] {
    let cell_w = img_w / tiles_w.max(1) as f32;
    let cell_h = img_h / tiles_h.max(1) as f32;
    [
        (t.x - origin.x) as f32 * cell_w + cell_w / 2.0,
        (t.z - origin.z) as f32 * cell_h + cell_h / 2.0,
    ]
}

/// Whether the cached polyline is stale: the route generation moved (a new
/// arm / focus switch) or `REFRESH` has elapsed since the last rebuild.
fn needs_refresh(cached_gen: u64, gen: u64, since: Duration) -> bool {
    gen != cached_gen || since >= REFRESH
}

/// Whether a world tile sits on the same cell as a panel tile.
fn same_tile(w: WorldTile, t: Tile) -> bool {
    w.x == t.x && w.z == t.z && w.level == t.level
}

/// The panel tile of a world tile (structurally identical fields).
fn tile_from(w: WorldTile) -> Tile {
    Tile {
        x: w.x,
        z: w.z,
        level: w.level,
    }
}

/// The tiles still ahead on a whole-world `Route`, front to back. Walk
/// legs contribute all their tiles; a transport leg contributes its `at`
/// and `to` so the polyline stays connected across the crossing. When
/// `here` is given (the player's observed tile), legs already traversed
/// are skipped exactly as the follow run skips them, and the current walk
/// leg is trimmed to the tiles from `here` onward so the line shrinks as
/// the player walks, not only at leg end. Empty when nothing is armed or
/// every leg is done.
fn remaining_route_tiles(route: &Route, here: Option<Tile>) -> Vec<Tile> {
    let mut leg = 0;
    if let Some(here) = here {
        while leg < route.legs.len() {
            let done = match &route.legs[leg] {
                Leg::Walk { tiles } => tiles.last().is_some_and(|last| same_tile(*last, here)),
                Leg::Transport { edge } => same_tile(edge.to, here),
            };
            if !done {
                break;
            }
            leg += 1;
        }
    }
    let mut out = Vec::new();
    for (i, l) in route.legs.iter().enumerate().skip(leg) {
        match l {
            Leg::Walk { tiles } => {
                if i == leg {
                    if let Some(here) = here {
                        if let Some(pos) = tiles.iter().position(|t| same_tile(*t, here)) {
                            out.extend(tiles[pos..].iter().map(|t| tile_from(*t)));
                            continue;
                        }
                    }
                }
                out.extend(tiles.iter().map(|t| tile_from(*t)));
            }
            Leg::Transport { edge } => {
                out.push(tile_from(edge.at));
                out.push(tile_from(edge.to));
            }
        }
    }
    // A transport's `to` is the next walk leg's first tile: drop the
    // duplicate crossing tile so the line does not double back.
    out.dedup();
    out
}

/// Window-space vertices for the focused walk arm's remaining route, or
/// empty when nothing is armed. The schematic grid is anchored on the
/// focused slot's observed tile; before the first position report the
/// route's own first tile (the tile it was routed from) anchors instead.
fn points_for(session: &Session, min: [f32; 2], size: [f32; 2]) -> Vec<[f32; 2]> {
    let Some(name) = session.focused_name() else {
        return Vec::new();
    };
    let Some(arm) = session.travellers.lock().unwrap().get(&name).cloned() else {
        return Vec::new();
    };
    let here = session.focused_tile().map(|(x, z)| Tile { x, z, level: 0 });
    let route = arm.lock().unwrap().route.clone();
    let Some(route) = route else {
        return Vec::new();
    };
    let tiles = remaining_route_tiles(&route, here);
    if tiles.is_empty() {
        return Vec::new();
    }
    let anchor = here.unwrap_or(tiles[0]);
    let origin = Tile {
        x: anchor.x - VIEW_TILES_W / 2,
        z: anchor.z - VIEW_TILES_H / 2,
        level: anchor.level,
    };
    tiles
        .iter()
        .map(|t| {
            let local = tile_to_local(*t, origin, VIEW_TILES_W, VIEW_TILES_H, size[0], size[1]);
            [min[0] + local[0], min[1] + local[1]]
        })
        .collect()
}

/// Amber queue-card lines for a focused FIFO place: title, `k of n`,
/// ahead label. Empty when the slot is not queued, so the card disappears
/// the moment the grant lands (`logging in…`).
fn queue_card_lines(queue: Option<(i32, i32)>) -> Vec<String> {
    match queue {
        Some((k, n)) => vec![
            QUEUE_CARD_TITLE.to_string(),
            queue_k_of_n(k, n).unwrap_or_default(),
            queue_ahead_label(k.max(1) as u32),
        ],
        None => Vec::new(),
    }
}

/// Draw the queue card as a dark amber-bordered block over the Image,
/// using the same WindowDrawList as the nav polyline. `min` is the Image's
/// top-left corner. Width is an estimate (no font measurement at this
/// layer), so the block only needs to be legible, not pixel-perfect.
fn draw_queue_card(ui: &Ui, min: [f32; 2], lines: &[String]) {
    const PAD: f32 = 8.0;
    const LINE_H: f32 = 15.0;
    let top = [min[0] + PAD, min[1] + PAD];
    let width = lines
        .iter()
        .map(|l| l.len() as f32 * 7.0)
        .fold(PAD * 2.0, f32::max);
    let height = PAD * 2.0 + LINE_H * (lines.len() as f32 - 1.0) + 13.0;
    let dl = ui.get_window_draw_list();
    dl.add_rect(
        [top[0] - PAD, top[1] - PAD],
        [top[0] + width, top[1] + height],
        [0.0, 0.0, 0.0, 0.6],
    )
    .filled(true)
    .build();
    dl.add_rect(
        [top[0] - PAD, top[1] - PAD],
        [top[0] + width, top[1] + height],
        ACCENT,
    )
    .thickness(1.0)
    .build();
    for (i, line) in lines.iter().enumerate() {
        dl.add_text([top[0], top[1] + i as f32 * LINE_H], ACCENT, line);
    }
}

/// Queue card over the focused slot's cell (MultiBox grid mode). No-op
/// when the focused slot is not queued, so the card disappears the moment
/// the grant lands.
pub fn draw_focused_queue_card(ui: &Ui, session: &Session, min: [f32; 2]) {
    let lines = queue_card_lines(session.queue_place());
    if !lines.is_empty() {
        draw_queue_card(ui, min, &lines);
    }
}

/// Cached overlay polyline: window-space vertices rebuilt only when the
/// route generation changes (rising edge on a new arm / focus switch) or
/// [`REFRESH`] elapsed, so an unchanged route does not recompute or
/// re-allocate vertices every 50 fps UI frame.
pub struct PathOverlay {
    points: Vec<[f32; 2]>,
    gen: u64,
    last: Instant,
    /// Cached focused FIFO place; the queue card must appear immediately
    /// on enqueue, so the tuple (not the 1 s raster cadence) is the gen.
    queue: Option<(i32, i32)>,
    queue_lines: Vec<String>,
}

impl PathOverlay {
    pub fn new() -> Self {
        Self {
            points: Vec::new(),
            gen: 0,
            last: Instant::now(),
            queue: None,
            queue_lines: Vec::new(),
        }
    }

    /// The cached window-space vertices (empty while nothing is armed).
    pub fn points(&self) -> &[[f32; 2]] {
        &self.points
    }

    /// Rebuild-if-stale, then stroke the amber polyline over the Image.
    /// `min`/`size` are the Image widget's top-left corner and size.
    pub fn frame(&mut self, ui: &Ui, session: &Session, min: [f32; 2], size: [f32; 2]) {
        let gen = session.route_gen();
        let now = Instant::now();
        if needs_refresh(self.gen, gen, now.duration_since(self.last)) {
            self.gen = gen;
            self.last = now;
            self.points = points_for(session, min, size);
        }
        if !self.points.is_empty() {
            ui.get_window_draw_list()
                .add_polyline(self.points.clone(), ACCENT)
                .closed(false)
                .thickness(2.0)
                .build();
        }
        let queue = session.queue_place();
        if queue != self.queue {
            self.queue = queue;
            self.queue_lines = queue_card_lines(queue);
        }
        if !self.queue_lines.is_empty() {
            draw_queue_card(ui, min, &self.queue_lines);
        }
    }
}

impl Default for PathOverlay {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use api::snapshot::WorldTile;
    use client::dash3d::CollisionFlag;
    use nav::collision::WorldCollision;
    use nav::router::{find, Leg};
    use nav::tile::Tile;
    use nav::transport::{TransportEdge, TransportGraph, TransportKind};
    use nav::world::NavWorld;

    use super::{
        needs_refresh, queue_card_lines, remaining_route_tiles, tile_to_local, PathOverlay,
    };
    use crate::session::Session;

    /// A `w`×`h` all-walkable level-0 world at (0,0).
    fn open_world(w: usize, h: usize) -> NavWorld {
        NavWorld {
            collision: WorldCollision {
                origin: WorldTile {
                    x: 0,
                    z: 0,
                    level: 0,
                },
                width: w,
                height: h,
                flags: vec![0u32; w * h],
                walkable: vec![0u32; w * h],
            },
            graph: TransportGraph::default(),
        }
    }

    fn tile(x: i32, z: i32, level: i32) -> WorldTile {
        WorldTile { x, z, level }
    }

    #[test]
    fn overlay_tile_to_image_roundtrip() {
        // tile 0,0 in a 3x3 at image [0,0]-[90,90] → center of cell
        let p = tile_to_local(
            Tile {
                x: 0,
                z: 0,
                level: 0,
            },
            Tile {
                x: 0,
                z: 0,
                level: 0,
            },
            3,
            3,
            90.0,
            90.0,
        );
        assert!(p[0] >= 0.0 && p[0] <= 30.0);
    }

    #[test]
    fn tile_to_local_centers_cells_from_origin() {
        // origin (1,1): tile (2,2) is the image centre of a 3x3 grid.
        let p = tile_to_local(
            Tile {
                x: 2,
                z: 2,
                level: 0,
            },
            Tile {
                x: 1,
                z: 1,
                level: 0,
            },
            3,
            3,
            90.0,
            90.0,
        );
        assert!((p[0] - 45.0).abs() < 0.001);
        assert!((p[1] - 45.0).abs() < 0.001);
    }

    #[test]
    fn tile_to_local_scales_to_image_size() {
        // Cells follow the image aspect: a 2x2 grid in a 100x60 image has
        // 50x30 cells, so tile (1,1) centres at (75,45).
        let p = tile_to_local(
            Tile {
                x: 1,
                z: 1,
                level: 0,
            },
            Tile {
                x: 0,
                z: 0,
                level: 0,
            },
            2,
            2,
            100.0,
            60.0,
        );
        assert!((p[0] - 75.0).abs() < 0.001);
        assert!((p[1] - 45.0).abs() < 0.001);
    }

    #[test]
    fn refresh_is_due_on_new_generation_or_one_second() {
        assert!(needs_refresh(0, 1, Duration::ZERO));
        assert!(needs_refresh(1, 1, Duration::from_secs(1)));
        assert!(!needs_refresh(1, 1, Duration::from_millis(999)));
    }

    #[test]
    fn overlay_builds_points_when_route_armed() {
        let _guard = crate::IMGUI_CTX_TEST_GUARD.lock().unwrap();
        let mut ctx = dear_imgui_rs::Context::create();
        ctx.prepare_frame(
            dear_imgui_rs::FramePrepareOptions::new([900.0, 700.0], 1.0 / 60.0)
                .renderer_has_textures(),
        );
        let ui = ctx.frame();
        let mut s = Session::new();
        s.focus.lock().unwrap().focused = Some("alice".into());
        let world = open_world(3, 3);
        s.arm_walk_on(
            &world,
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
        );
        let mut overlay = PathOverlay::new();
        ui.window("##overlay-test").build(|| {
            overlay.frame(ui, &s, [10.0, 10.0], [90.0, 90.0]);
        });
        ctx.render();
        assert!(!overlay.points().is_empty(), "an armed route must paint");
        assert!(
            s.route_gen() != 0,
            "arming must bump the overlay generation"
        );
    }

    #[test]
    fn overlay_skips_draw_without_route() {
        let _guard = crate::IMGUI_CTX_TEST_GUARD.lock().unwrap();
        let mut ctx = dear_imgui_rs::Context::create();
        ctx.prepare_frame(
            dear_imgui_rs::FramePrepareOptions::new([900.0, 700.0], 1.0 / 60.0)
                .renderer_has_textures(),
        );
        let ui = ctx.frame();
        let s = Session::new();
        s.focus.lock().unwrap().focused = Some("alice".into());
        let mut overlay = PathOverlay::new();
        ui.window("##overlay-test").build(|| {
            overlay.frame(ui, &s, [10.0, 10.0], [90.0, 90.0]);
        });
        ctx.render();
        assert!(overlay.points().is_empty(), "no route -> no polyline");
    }

    #[test]
    fn overlay_refreshes_when_route_changes() {
        let _guard = crate::IMGUI_CTX_TEST_GUARD.lock().unwrap();
        let mut ctx = dear_imgui_rs::Context::create();
        ctx.prepare_frame(
            dear_imgui_rs::FramePrepareOptions::new([900.0, 700.0], 1.0 / 60.0)
                .renderer_has_textures(),
        );
        let mut s = Session::new();
        s.focus.lock().unwrap().focused = Some("alice".into());
        let world = open_world(3, 3);
        s.arm_walk_on(
            &world,
            Tile {
                x: 0,
                z: 0,
                level: 0,
            },
            Tile {
                x: 1,
                z: 1,
                level: 0,
            },
        );
        let mut overlay = PathOverlay::new();
        {
            let ui = ctx.frame();
            ui.window("##overlay-test").build(|| {
                overlay.frame(ui, &s, [0.0, 0.0], [90.0, 90.0]);
            });
        }
        ctx.render();
        let first = overlay.points()[0];
        let last = *overlay.points().last().unwrap();
        // Re-arm a longer route: the new generation must rebuild now. Both
        // routes start on the same tile so the first point is stable, but
        // the line must reach the new dest.
        s.arm_walk_on(
            &world,
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
        );
        ctx.prepare_frame(
            dear_imgui_rs::FramePrepareOptions::new([900.0, 700.0], 1.0 / 60.0)
                .renderer_has_textures(),
        );
        {
            let ui = ctx.frame();
            ui.window("##overlay-test").build(|| {
                overlay.frame(ui, &s, [0.0, 0.0], [90.0, 90.0]);
            });
        }
        ctx.render();
        assert_eq!(overlay.points()[0], first);
        assert_ne!(overlay.points().last().copied(), Some(last));
    }

    #[test]
    fn remaining_route_tiles_maps_walk_legs_front_to_back() {
        // A walk route (0,0)->(2,2) on an open world maps one point per
        // route tile, front to back.
        let world = open_world(3, 3);
        let route = find(&world.collision, &world.graph, tile(0, 0, 0), tile(2, 2, 0)).unwrap();
        let tiles = remaining_route_tiles(&route, None);
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
        let Leg::Walk { tiles: leg } = &route.legs[0] else {
            panic!("walk-only route");
        };
        assert_eq!(tiles.len(), leg.len(), "a walk leg maps one point per tile");
    }

    #[test]
    fn remaining_route_tiles_connects_transport_legs() {
        // A walled tile (2,0) with a door edge across it: the route splits
        // into Walk -> Transport -> Walk and the remaining tiles stay
        // connected across the crossing (at then to, deduped).
        let mut flags = vec![0u32; 5];
        flags[2] = CollisionFlag::WALK_BLOCK_FLAGS as u32;
        let mut graph = TransportGraph::default();
        graph.edges.push(TransportEdge {
            kind: TransportKind::Door,
            at: tile(1, 0, 0),
            to: tile(3, 0, 0),
            loc_id: 1530,
            option: 1,
            ticks: 1,
            dir: None,
            open_loc_id: None,
            skill_req: vec![],
            item_req: vec![],
            quest_req: vec![],
            varp_req: vec![],
        });
        graph.at.entry(tile(1, 0, 0)).or_default().push(0);
        let world = NavWorld {
            collision: WorldCollision {
                origin: tile(0, 0, 0),
                width: 5,
                height: 1,
                walkable: nav::collision::derive_walkable(&flags),
                flags,
            },
            graph,
        };
        let route = find(&world.collision, &world.graph, tile(0, 0, 0), tile(4, 0, 0)).unwrap();
        assert_eq!(
            remaining_route_tiles(&route, None),
            vec![
                Tile {
                    x: 0,
                    z: 0,
                    level: 0
                },
                Tile {
                    x: 1,
                    z: 0,
                    level: 0
                },
                Tile {
                    x: 3,
                    z: 0,
                    level: 0
                },
                Tile {
                    x: 4,
                    z: 0,
                    level: 0
                },
            ]
        );
    }

    #[test]
    fn queue_card_lines_match_rs2b0t_copy() {
        assert_eq!(queue_card_lines(None), Vec::<String>::new());
        let lines = queue_card_lines(Some((1, 2)));
        assert_eq!(lines[0], "AUTO-LOGIN QUEUE");
        assert_eq!(lines[1], "1 of 2");
        assert_eq!(lines[2], "0 bots in front");
        let lines = queue_card_lines(Some((2, 2)));
        assert_eq!(lines[2], "1 bot in front");
    }

    #[test]
    fn overlay_draws_queue_card_when_focused_slot_is_queued() {
        let _guard = crate::IMGUI_CTX_TEST_GUARD.lock().unwrap();
        let mut ctx = dear_imgui_rs::Context::create();
        ctx.prepare_frame(
            dear_imgui_rs::FramePrepareOptions::new([900.0, 700.0], 1.0 / 60.0)
                .renderer_has_textures(),
        );
        let ui = ctx.frame();
        let mut s = Session::new();
        s.focus.lock().unwrap().focused = Some("alice".into());
        s.statuses.push(host_play::SlotStatus {
            username: "alice".into(),
            queue_position: 1,
            queue_total: 2,
            ..host_play::SlotStatus::default()
        });
        let mut overlay = PathOverlay::new();
        ui.window("##overlay-queue-test").build(|| {
            overlay.frame(ui, &s, [10.0, 10.0], [90.0, 90.0]);
        });
        ctx.render();
        assert_eq!(
            overlay.queue_lines,
            vec![
                "AUTO-LOGIN QUEUE".to_string(),
                "1 of 2".to_string(),
                "0 bots in front".to_string()
            ]
        );
    }

    #[test]
    fn overlay_queue_card_follows_fifo_head_when_focus_already_granted() {
        let _guard = crate::IMGUI_CTX_TEST_GUARD.lock().unwrap();
        let mut ctx = dear_imgui_rs::Context::create();
        ctx.prepare_frame(
            dear_imgui_rs::FramePrepareOptions::new([900.0, 700.0], 1.0 / 60.0)
                .renderer_has_textures(),
        );
        let ui = ctx.frame();
        let mut s = Session::new();
        s.focus.lock().unwrap().focused = Some("s00".into());
        s.statuses.push(host_play::SlotStatus {
            username: "s00".into(),
            ..host_play::SlotStatus::default()
        });
        s.statuses.push(host_play::SlotStatus {
            username: "s01".into(),
            queue_position: 1,
            queue_total: 49,
            ..host_play::SlotStatus::default()
        });
        let mut overlay = PathOverlay::new();
        ui.window("##overlay-queue-fifo-test").build(|| {
            overlay.frame(ui, &s, [10.0, 10.0], [90.0, 90.0]);
        });
        ctx.render();
        assert_eq!(
            overlay.queue_lines[1], "1 of 49",
            "after the focused slot grants, the card steps k of n for the next queued member"
        );
    }

    #[test]
    fn overlay_skips_queue_card_when_not_queued() {
        let _guard = crate::IMGUI_CTX_TEST_GUARD.lock().unwrap();
        let mut ctx = dear_imgui_rs::Context::create();
        ctx.prepare_frame(
            dear_imgui_rs::FramePrepareOptions::new([900.0, 700.0], 1.0 / 60.0)
                .renderer_has_textures(),
        );
        let ui = ctx.frame();
        let s = Session::new();
        s.focus.lock().unwrap().focused = Some("alice".into());
        let mut overlay = PathOverlay::new();
        ui.window("##overlay-queue-test").build(|| {
            overlay.frame(ui, &s, [10.0, 10.0], [90.0, 90.0]);
        });
        ctx.render();
        assert!(overlay.queue_lines.is_empty(), "no queue -> no card");
    }
}
