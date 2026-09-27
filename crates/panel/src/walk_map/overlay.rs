//! Viewport-local grid/collision/reach overlay. One RGBA image, never per-tile quads.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use api::snapshot::WorldTile;
use nav::map::spatial::{GameTile, View};
use nav::paint::{collision_at, reached};
use nav::world::NavWorld;

/// Overlay texture ceiling from design §5.2.
pub const OVERLAY_MAX_W: u32 = 768;
pub const OVERLAY_MAX_H: u32 = 512;
/// Design §5.3: 768×512 RGBA = 1.5 MiB CPU and 1.5 MiB GPU while a layer is on.
pub const OVERLAY_BYTE_CAP: usize = (OVERLAY_MAX_W as usize) * (OVERLAY_MAX_H as usize) * 4;
/// Dots and cell fill need at least this many overlay pixels per world tile.
pub const MIN_CELL_PPT: f64 = 4.0;
/// NSEW text is drawn only at this screen density.
pub const NSEW_PPT: f64 = 16.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OverlayLayers {
    pub grid: bool,
    pub collision_fill: bool,
    pub reach: bool,
    pub nsew: bool,
    pub path: bool,
    pub flood: bool,
}

impl OverlayLayers {
    pub fn any(self) -> bool {
        self.grid || self.collision_fill || self.reach || self.nsew || self.path || self.flood
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverlayFit {
    Ready { w: u32, h: u32 },
    ZoomIn,
}

/// Size the overlay from the view's world span. The 4 px/tile rule uses this
/// overlay's effective resolution, not only the window's logical scale.
pub fn overlay_fit(view: View) -> OverlayFit {
    let world_w = view.east - view.west;
    let world_h = view.north - view.south;
    if !(world_w.is_finite() && world_h.is_finite()) || world_w <= 0.0 || world_h <= 0.0 {
        return OverlayFit::ZoomIn;
    }
    let need_w = (world_w * MIN_CELL_PPT).ceil();
    let need_h = (world_h * MIN_CELL_PPT).ceil();
    let w = need_w.min(f64::from(OVERLAY_MAX_W));
    let h = need_h.min(f64::from(OVERLAY_MAX_H));
    if w / world_w + f64::EPSILON < MIN_CELL_PPT || h / world_h + f64::EPSILON < MIN_CELL_PPT {
        return OverlayFit::ZoomIn;
    }
    OverlayFit::Ready {
        w: w.max(1.0) as u32,
        h: h.max(1.0) as u32,
    }
}

#[derive(Debug, Clone, Copy)]
pub struct OverlayColors {
    pub grid: [u8; 4],
    pub collision: [u8; 4],
    pub path: [u8; 4],
    pub transport: [u8; 4],
    pub flood_a: [u8; 4],
    pub flood_b: [u8; 4],
    pub unreached: [u8; 4],
    pub nsew: [u8; 4],
}

impl Default for OverlayColors {
    fn default() -> Self {
        Self {
            grid: [255, 176, 0, 180],
            collision: [0, 128, 255, 200],
            path: [255, 0, 0, 220],
            transport: [0, 255, 0, 220],
            flood_a: [0, 0, 255, 160],
            flood_b: [200, 40, 240, 160],
            unreached: [200, 40, 240, 160],
            nsew: [220, 220, 220, 220],
        }
    }
}

pub struct OverlayPaint<'a> {
    pub world: &'a NavWorld,
    pub layers: OverlayLayers,
    pub colors: OverlayColors,
    pub path: &'a [(WorldTile, bool)],
    pub floods: &'a [Arc<HashSet<WorldTile>>],
    pub reach: Option<&'a [u64]>,
}

/// Rasterize visible cells into `rgba` (`w*h*4`). Transparent until a layer paints.
pub fn rasterize(rgba: &mut [u8], w: u32, h: u32, view: View, paint: OverlayPaint<'_>) {
    let n = (w as usize).saturating_mul(h as usize).saturating_mul(4);
    if rgba.len() < n || w == 0 || h == 0 {
        return;
    }
    rgba[..n].fill(0);
    if !paint.layers.any() {
        return;
    }
    let path_at: HashMap<WorldTile, bool> = if paint.layers.path {
        paint.path.iter().copied().collect()
    } else {
        HashMap::new()
    };
    let x0 = view.west.floor() as i32;
    let x1 = view.east.ceil() as i32;
    let z0 = view.south.floor() as i32;
    let z1 = view.north.ceil() as i32;
    for z in z0..z1 {
        for x in x0..x1 {
            let tile = GameTile {
                x,
                z,
                plane: view.plane,
            };
            let wt = WorldTile {
                x,
                z,
                level: i32::from(view.plane),
            };
            let color = cell_color(wt, paint.world, &paint, &path_at);
            if color[3] != 0 {
                fill_tile(rgba, w, h, view, tile, color, &paint);
            }
            if paint.layers.nsew {
                stroke_faces(rgba, w, h, view, wt, paint.world, paint.colors.nsew);
            }
        }
    }
}

fn cell_color(
    wt: WorldTile,
    world: &NavWorld,
    paint: &OverlayPaint<'_>,
    path_at: &HashMap<WorldTile, bool>,
) -> [u8; 4] {
    if paint.layers.path {
        if let Some(&transport) = path_at.get(&wt) {
            return if transport {
                paint.colors.transport
            } else {
                paint.colors.path
            };
        }
    }
    if paint.layers.flood {
        if let Some(id) = paint
            .floods
            .iter()
            .position(|set| set.contains(&wt))
            .map(|i| i as u32)
        {
            return if id == 0 {
                paint.colors.flood_a
            } else {
                paint.colors.flood_b
            };
        }
    }
    if paint.layers.reach {
        if let Some(bits) = paint.reach {
            if !reached(bits, &world.collision, wt) && world.collision.standable(wt) {
                return paint.colors.unreached;
            }
        }
    }
    if paint.layers.collision_fill && collision_at(&world.collision, wt).blocked {
        return paint.colors.collision;
    }
    if paint.layers.grid && world.collision.walkable(wt) {
        return paint.colors.grid;
    }
    [0, 0, 0, 0]
}

fn fill_tile(
    rgba: &mut [u8],
    w: u32,
    h: u32,
    view: View,
    tile: GameTile,
    color: [u8; 4],
    paint: &OverlayPaint<'_>,
) {
    let Some((x0, y0, x1, y1)) = tile_pixels(view, w, h, tile.x, tile.z, tile.x + 1, tile.z + 1)
    else {
        return;
    };
    if paint.layers.grid && color == paint.colors.grid {
        let cx = (x0 + x1) / 2;
        let cy = (y0 + y1) / 2;
        let r = ((x1 - x0).min(y1 - y0) / 4).max(1);
        fill_rect(rgba, w, h, [cx - r, cy - r, cx + r, cy + r], color);
        return;
    }
    fill_rect(rgba, w, h, [x0, y0, x1, y1], color);
}

fn stroke_faces(
    rgba: &mut [u8],
    w: u32,
    h: u32,
    view: View,
    wt: WorldTile,
    world: &NavWorld,
    color: [u8; 4],
) {
    let faces = collision_at(&world.collision, wt);
    let Some((x0, y0, x1, y1)) = tile_pixels(view, w, h, wt.x, wt.z, wt.x + 1, wt.z + 1) else {
        return;
    };
    if faces.n {
        fill_rect(rgba, w, h, [x0, y0, x1, y0 + 1], color);
    }
    if faces.s {
        fill_rect(rgba, w, h, [x0, y1 - 1, x1, y1], color);
    }
    if faces.w {
        fill_rect(rgba, w, h, [x0, y0, x0 + 1, y1], color);
    }
    if faces.e {
        fill_rect(rgba, w, h, [x1 - 1, y0, x1, y1], color);
    }
}

fn tile_pixels(
    view: View,
    w: u32,
    h: u32,
    x0: i32,
    z0: i32,
    x1: i32,
    z1: i32,
) -> Option<(i32, i32, i32, i32)> {
    let world_w = view.east - view.west;
    let world_h = view.north - view.south;
    if world_w <= 0.0 || world_h <= 0.0 {
        return None;
    }
    let sx = f64::from(w) / world_w;
    let sy = f64::from(h) / world_h;
    let px0 = ((f64::from(x0) - view.west) * sx).floor() as i32;
    let px1 = ((f64::from(x1) - view.west) * sx).ceil() as i32;
    let py0 = ((view.north - f64::from(z1)) * sy).floor() as i32;
    let py1 = ((view.north - f64::from(z0)) * sy).ceil() as i32;
    Some((px0.min(px1), py0.min(py1), px0.max(px1), py0.max(py1)))
}

fn fill_rect(rgba: &mut [u8], w: u32, h: u32, rect: [i32; 4], color: [u8; 4]) {
    let [x0, y0, x1, y1] = rect;
    let x0 = x0.max(0) as u32;
    let y0 = y0.max(0) as u32;
    let x1 = (x1.max(0) as u32).min(w);
    let y1 = (y1.max(0) as u32).min(h);
    if x0 >= x1 || y0 >= y1 {
        return;
    }
    for y in y0..y1 {
        let row = (y as usize) * (w as usize) * 4;
        for x in x0..x1 {
            let i = row + (x as usize) * 4;
            rgba[i..i + 4].copy_from_slice(&color);
        }
    }
}

#[cfg(test)]
pub fn pixel(rgba: &[u8], w: u32, x: u32, y: u32) -> [u8; 4] {
    let i = (y as usize * w as usize + x as usize) * 4;
    rgba.get(i..i + 4)
        .and_then(|s| s.try_into().ok())
        .unwrap_or([0; 4])
}

#[cfg(test)]
mod rasterize_rules {
    use super::*;
    use client::dash3d::CollisionFlag;
    use nav::collision::WorldCollision;
    use nav::paint::{flood_components, reached};
    use nav::transport::{TransportEdge, TransportGraph, TransportKind};
    use nav::world::NavWorld;

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

    fn sample(
        view: View,
        world: &NavWorld,
        layers: OverlayLayers,
        paint_path: &[(WorldTile, bool)],
        floods: &[Arc<HashSet<WorldTile>>],
        reach: Option<&[u64]>,
    ) -> (Vec<u8>, u32, u32) {
        let OverlayFit::Ready { w, h } = overlay_fit(view) else {
            panic!("view must fit overlay");
        };
        let mut rgba = vec![0u8; (w * h * 4) as usize];
        rasterize(
            &mut rgba,
            w,
            h,
            view,
            OverlayPaint {
                world,
                layers,
                colors: OverlayColors::default(),
                path: paint_path,
                floods,
                reach,
            },
        );
        (rgba, w, h)
    }

    fn at_tile(rgba: &[u8], w: u32, h: u32, view: View, x: i32, z: i32) -> [u8; 4] {
        let sx = (f64::from(x) + 0.5 - view.west) / (view.east - view.west) * f64::from(w);
        let sy = (view.north - (f64::from(z) + 0.5)) / (view.north - view.south) * f64::from(h);
        pixel(rgba, w, (sx as u32).min(w - 1), (sy as u32).min(h - 1))
    }

    #[test]
    fn collision_only_in_viewport() {
        let world = bake_world(5, 1, &[(2, 0, CollisionFlag::WR_GRND as u32)]);
        let view = View {
            west: 1.0,
            south: 0.0,
            east: 3.0,
            north: 1.0,
            pixels_per_tile: 16.0,
            plane: 0,
            max_lod: 0,
        };
        let layers = OverlayLayers {
            grid: false,
            collision_fill: true,
            reach: false,
            nsew: false,
            path: false,
            flood: false,
        };
        let (rgba, w, h) = sample(view, &world, layers, &[], &[], None);
        let colors = OverlayColors::default();
        assert_eq!(at_tile(&rgba, w, h, view, 2, 0), colors.collision);
        assert_eq!(at_tile(&rgba, w, h, view, 1, 0), [0, 0, 0, 0]);
    }

    #[test]
    fn paints_the_selected_plane() {
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
        let view = View {
            west: 0.0,
            south: 0.0,
            east: 3.0,
            north: 3.0,
            pixels_per_tile: 16.0,
            plane: 1,
            max_lod: 0,
        };
        let layers = OverlayLayers {
            grid: false,
            collision_fill: true,
            reach: false,
            nsew: false,
            path: false,
            flood: false,
        };
        let (rgba, w, h) = sample(view, &world, layers, &[], &[], None);
        let colors = OverlayColors::default();
        assert_eq!(at_tile(&rgba, w, h, view, 1, 1), colors.collision);
        let ground = View { plane: 0, ..view };
        let (rgba0, w0, h0) = sample(ground, &world, layers, &[], &[], None);
        assert_eq!(at_tile(&rgba0, w0, h0, ground, 1, 1), [0, 0, 0, 0]);
    }

    #[test]
    fn flood_marks_two_components() {
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
        let floods: Vec<Arc<HashSet<WorldTile>>> =
            flood_components(&world.collision, &[player, dest])
                .into_iter()
                .map(Arc::new)
                .collect();
        assert_eq!(floods.len(), 2);
        let view = View {
            west: 0.0,
            south: 0.0,
            east: 7.0,
            north: 7.0,
            pixels_per_tile: 16.0,
            plane: 0,
            max_lod: 0,
        };
        let layers = OverlayLayers {
            grid: false,
            collision_fill: false,
            reach: false,
            nsew: false,
            path: false,
            flood: true,
        };
        let (rgba, w, h) = sample(view, &world, layers, &[], &floods, None);
        let colors = OverlayColors::default();
        assert_eq!(at_tile(&rgba, w, h, view, 0, 0), colors.flood_a);
        assert_eq!(at_tile(&rgba, w, h, view, 5, 5), colors.flood_b);
        assert_eq!(at_tile(&rgba, w, h, view, 3, 3), [0, 0, 0, 0]);
    }

    #[test]
    fn marks_walkable_unreached_puddle() {
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
        let bits = nav::paint::bake_reach(&world.collision, &world.graph);
        let courtyard = WorldTile {
            x: 2,
            z: 2,
            level: 0,
        };
        assert!(
            !reached(&bits, &world.collision, courtyard) && world.collision.standable(courtyard)
        );
        let view = View {
            west: 0.0,
            south: 0.0,
            east: 5.0,
            north: 5.0,
            pixels_per_tile: 16.0,
            plane: 0,
            max_lod: 0,
        };
        let layers = OverlayLayers {
            grid: false,
            collision_fill: false,
            reach: true,
            nsew: false,
            path: false,
            flood: false,
        };
        let (rgba, w, h) = sample(view, &world, layers, &[], &[], Some(&bits));
        let colors = OverlayColors::default();
        assert_eq!(at_tile(&rgba, w, h, view, 2, 2), colors.unreached);
        assert_eq!(at_tile(&rgba, w, h, view, 0, 0), [0, 0, 0, 0]);
    }

    #[test]
    fn marks_remaining_path_and_transport() {
        let world = bake_world(5, 1, &[]);
        let view = View {
            west: 0.0,
            south: 0.0,
            east: 5.0,
            north: 1.0,
            pixels_per_tile: 16.0,
            plane: 0,
            max_lod: 0,
        };
        let path = [
            (
                WorldTile {
                    x: 0,
                    z: 0,
                    level: 0,
                },
                false,
            ),
            (
                WorldTile {
                    x: 1,
                    z: 0,
                    level: 0,
                },
                true,
            ),
        ];
        let layers = OverlayLayers {
            grid: false,
            collision_fill: false,
            reach: false,
            nsew: false,
            path: true,
            flood: false,
        };
        let (rgba, w, h) = sample(view, &world, layers, &path, &[], None);
        let colors = OverlayColors::default();
        assert_eq!(at_tile(&rgba, w, h, view, 0, 0), colors.path);
        assert_eq!(at_tile(&rgba, w, h, view, 1, 0), colors.transport);
        assert_eq!(at_tile(&rgba, w, h, view, 2, 0), [0, 0, 0, 0]);
    }
}
