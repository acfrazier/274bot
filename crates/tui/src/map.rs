//! TUI world-map widget. Consumes the shared [`NavWorld`], the player's
//! tile, an optional routed [`Route`], the operator's [`MapView`] and
//! catalogue POIs; paints the walkable dot field, remaining-walk polyline,
//! here marker, POI glyphs and selection crosshair. Keyboard: arrows/hjkl
//! pan, `+`/`-` zoom, `/` search, Enter selects/arms through the shared map
//! model, and Esc clears or closes the map.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::widgets::Widget;

use api::snapshot::WorldTile;
use host_play::walk_map::Catalogue;
use nav::map::poi::{PoiKind, PoiRecord};
use nav::paint::remaining_path_tiles;
use nav::router::Route;
use nav::tile::Tile;
use nav::world::NavWorld;

/// Zoom steps in tiles per cell (coarse to fine): the TUI twin of the
/// picker's pixel `ZOOMS`. A cell covers this many world tiles per side.
pub const ZOOMS: [usize; 4] = [8, 4, 2, 1];

/// Default view centre while the player tile is unknown: the Lumbridge
/// courtyard, same default as the headed picker.
pub const DEFAULT_CENTRE: (i32, i32) = (3220, 3220);

/// Glyphs for the semantic terminal map. Collision dots are deliberately
/// simple: terminal cells are navigation diagnostics, not terrain artwork.
const WALKABLE_GLYPH: &str = ".";
const BLOCKED_GLYPH: &str = "#";
const HERE_GLYPH: &str = "@";
const PATH_GLYPH: &str = "*";
const SELECTION_GLYPH: &str = "+";
const OBSERVED_GLYPH: &str = "N";

/// Optional map layers. The dot layer is the useful default; collision and
/// reach are explicit diagnostics and never trigger a provider load here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MapLayers {
    pub dots: bool,
    pub collision: bool,
    pub reach: bool,
}

impl Default for MapLayers {
    fn default() -> Self {
        Self {
            dots: true,
            collision: false,
            reach: false,
        }
    }
}

/// Compact observed-NPC marker. The TUI never retains snapshot NPC tables.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ObservedMark {
    pub x: i32,
    pub z: i32,
    pub level: i32,
}

/// One keyboard outcome from [`Map::on_key`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MapAction {
    /// The walk was confirmed: walk to the selected tile.
    Walk(Tile),
    /// The view state changed (pan/zoom/selection) but nothing walked.
    Moved,
    /// The key was ignored.
    Ignored,
}

/// Mutable map view state kept across frames: pan offset, zoom step, and
/// the operator's selection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MapView {
    /// Pan offset in tiles from the follow centre; zero re-centres the
    /// view on the player tile.
    pub pan: (i32, i32),
    /// Index into [`ZOOMS`] (tiles per cell); higher zooms in.
    pub zoom: usize,
    /// Navigation plane shown by this view. Plane selection is independent
    /// of raster LOD (the TUI has no raster path).
    pub plane: u8,
    /// Optional semantic terminal layers.
    pub layers: MapLayers,
    /// The operator's selected tile (`+` crosshair); `None` once Esc
    /// clears it.
    pub selection: Option<Tile>,
}

impl MapView {
    /// A fresh view: centred on the player, finest zoom, no selection.
    pub fn new() -> Self {
        Self {
            pan: (0, 0),
            zoom: ZOOMS.len() - 1,
            plane: 0,
            layers: MapLayers::default(),
            selection: None,
        }
    }
}

impl Default for MapView {
    fn default() -> Self {
        Self::new()
    }
}

pub struct Map<'a, F> {
    pub world: &'a NavWorld,
    /// The player's observed tile; `None` before the first snapshot.
    pub here: Option<WorldTile>,
    /// The armed route whose remaining tiles paint `*` (optional).
    pub route: Option<&'a Route>,
    /// Small shared catalogue projection. The TUI never owns one copy per
    /// bot and never decodes terrain images.
    pub pois: &'a [PoiRecord],
    pub catalogue: Option<&'a Catalogue>,
    pub selected_poi: Option<usize>,
    /// Focused-bot live NPC services; dropped when the map closes.
    pub observed: &'a [ObservedMark],
    pub view: &'a mut MapView,
    /// Walk-confirm hook: called with the selected tile.
    pub walk: F,
}

impl<'a, F: FnMut(Tile)> Map<'a, F> {
    /// New map over the packed collision world with the view state `view`;
    /// `walk` receives the tile a confirmed selection walks to.
    pub fn new(world: &'a NavWorld, view: &'a mut MapView, walk: F) -> Self {
        Self {
            world,
            here: None,
            route: None,
            pois: &[],
            catalogue: None,
            selected_poi: None,
            observed: &[],
            view,
            walk,
        }
    }

    /// Attach the shared POI projection. Records remain compact and
    /// revision-bound; no name/coordinate roster is invented in this widget.
    pub fn pois(mut self, pois: &'a [PoiRecord]) -> Self {
        self.pois = pois;
        self
    }

    pub fn catalogue(mut self, catalogue: &'a Catalogue) -> Self {
        self.catalogue = Some(catalogue);
        self
    }

    pub fn selected_poi(mut self, selected: Option<usize>) -> Self {
        self.selected_poi = selected;
        self
    }

    pub fn observed(mut self, observed: &'a [ObservedMark]) -> Self {
        self.observed = observed;
        self
    }

    /// Set the player's observed tile (the `@` marker).
    pub fn here(mut self, here: WorldTile) -> Self {
        self.here = Some(here);
        self
    }

    /// Attach the armed route whose remaining tiles paint `*`.
    pub fn route(mut self, route: &'a Route) -> Self {
        self.route = Some(route);
        self
    }

    /// The tile the view centres on: the player tile (or the picker
    /// default) plus the pan offset, on the explicitly selected plane.
    pub fn centre(&self) -> WorldTile {
        let (bx, bz) = match self.here {
            Some(h) => (h.x, h.z),
            None => (DEFAULT_CENTRE.0, DEFAULT_CENTRE.1),
        };
        WorldTile {
            x: bx + self.view.pan.0,
            z: bz + self.view.pan.1,
            level: i32::from(self.view.plane),
        }
    }

    /// Handle one key event. Pan keys are scoped to Map focus (h/l are not
    /// global loadout shortcuts), Enter selects the centre or confirms a
    /// walk, Esc clears the selection, and +/- changes terminal zoom.
    pub fn on_key(&mut self, key: KeyEvent) -> MapAction {
        match key.code {
            KeyCode::Left | KeyCode::Char('h') => self.pan_by(-1, 0),
            KeyCode::Right | KeyCode::Char('l') => self.pan_by(1, 0),
            KeyCode::Up | KeyCode::Char('k') => self.pan_by(0, 1),
            KeyCode::Down | KeyCode::Char('j') => self.pan_by(0, -1),
            KeyCode::Enter => self.confirm(),
            KeyCode::Esc => {
                self.view.selection = None;
                MapAction::Moved
            }
            KeyCode::Char('+') | KeyCode::Char('=') => {
                if self.view.zoom < ZOOMS.len() - 1 {
                    self.view.zoom += 1;
                }
                MapAction::Moved
            }
            KeyCode::Char('-') => {
                if self.view.zoom > 0 {
                    self.view.zoom -= 1;
                }
                MapAction::Moved
            }
            _ => MapAction::Ignored,
        }
    }

    /// Pan the view by one zoom step: the camera follows the key (Right
    /// east, Up north, Left west, Down south — +x east, +z north), the
    /// same orientation as the headed picker.
    fn pan_by(&mut self, dx: i32, dz: i32) -> MapAction {
        let step = ZOOMS[self.view.zoom.min(ZOOMS.len() - 1)] as i32;
        self.view.pan.0 += dx * step;
        self.view.pan.1 += dz * step;
        MapAction::Moved
    }

    /// Enter: with a selection, confirm Walk; without one, select the
    /// view-centre tile as the requested destination. Blocked ground is
    /// allowed — Walk still needs the shared radius-16 snap on the model.
    fn confirm(&mut self) -> MapAction {
        if let Some(sel) = self.view.selection {
            (self.walk)(sel);
            return MapAction::Walk(sel);
        }
        let c = self.centre();
        if !(0..4).contains(&c.level) {
            return MapAction::Moved;
        }
        self.view.selection = Some(Tile {
            x: c.x,
            z: c.z,
            level: c.level,
        });
        MapAction::Moved
    }
}

impl<'a, F: FnMut(Tile)> Widget for Map<'a, F> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let step = ZOOMS[self.view.zoom.min(ZOOMS.len() - 1)] as i32;
        let c = self.centre();
        let (w, h) = (area.width as i32, area.height as i32);
        // Iterate only the visible bounded window. At coarse zoom, collision
        // mode uses explicit mixed cells rather than claiming “walkable if
        // any underlying tile happened to overwrite the cell”.
        let half_x = (w * step) / 2;
        let half_z = (h * step) / 2;
        let x_lo = (c.x - half_x - step).max(self.world.collision.origin.x);
        let z_lo = (c.z - half_z - step).max(self.world.collision.origin.z);
        let x_hi = (c.x + half_x + step)
            .min(self.world.collision.origin.x + self.world.collision.width as i32);
        let z_hi = (c.z + half_z + step)
            .min(self.world.collision.origin.z + self.world.collision.height as i32);
        for z in z_lo..z_hi {
            for x in x_lo..x_hi {
                let Some((col, row)) = cell_of(x, z, (c.x, c.z), step, area) else {
                    continue;
                };
                let walkable = self.world.collision.walkable(WorldTile {
                    x,
                    z,
                    level: c.level,
                });
                let glyph = if self.view.layers.collision && step > 1 {
                    if walkable {
                        WALKABLE_GLYPH
                    } else {
                        BLOCKED_GLYPH
                    }
                } else if walkable {
                    WALKABLE_GLYPH
                } else {
                    " "
                };
                if self.view.layers.dots || self.view.layers.collision {
                    put(buf, area, col, row, glyph);
                }
            }
        }

        if let Some(route) = self.route {
            for pt in remaining_path_tiles(route, self.here) {
                if pt.tile.level != c.level {
                    continue;
                }
                if let Some((col, row)) = cell_of(pt.tile.x, pt.tile.z, (c.x, c.z), step, area) {
                    put(buf, area, col, row, PATH_GLYPH);
                }
            }
        }

        // Shared catalogue POIs are semantic glyphs, not a frozen destination
        // list. Full names and provenance stay in the inspector/list pane.
        if let Some(catalogue) = self.catalogue {
            for entry in catalogue.entries() {
                let anchor = entry.anchor();
                if anchor.level != i32::from(self.view.plane) {
                    continue;
                }
                let Some((col, row)) = cell_of(anchor.x, anchor.z, (c.x, c.z), step, area) else {
                    continue;
                };
                put(
                    buf,
                    area,
                    col,
                    row,
                    if self.selected_poi == Some(entry.index()) {
                        "*"
                    } else {
                        poi_glyph(entry.kind())
                    },
                );
            }
        } else {
            for (index, poi) in self.pois.iter().enumerate() {
                if poi.effective_plane != self.view.plane {
                    continue;
                }
                let Some((col, row)) = cell_of(
                    poi.display.x.floor() as i32,
                    poi.display.z.floor() as i32,
                    (c.x, c.z),
                    step,
                    area,
                ) else {
                    continue;
                };
                put(
                    buf,
                    area,
                    col,
                    row,
                    if self.selected_poi == Some(index) {
                        "*"
                    } else {
                        poi_glyph(poi.kind)
                    },
                );
            }
        }

        for mark in self.observed {
            if mark.level != c.level {
                continue;
            }
            let Some((col, row)) = cell_of(mark.x, mark.z, (c.x, c.z), step, area) else {
                continue;
            };
            put(buf, area, col, row, OBSERVED_GLYPH);
        }

        if let Some(sel) = self.view.selection {
            if sel.level == c.level {
                if let Some((col, row)) = cell_of(sel.x, sel.z, (c.x, c.z), step, area) {
                    put(buf, area, col, row, SELECTION_GLYPH);
                }
            }
        }
        if let Some(here) = self.here {
            if here.level == c.level {
                if let Some((col, row)) = cell_of(here.x, here.z, (c.x, c.z), step, area) {
                    put(buf, area, col, row, HERE_GLYPH);
                }
            }
        }
    }
}

/// The buffer cell (col, row) whose tile range contains world `(x, z)`;
/// `None` when the tile is outside the view. +x is east (right), +z is
/// north (up) — the same orientation as the headed picker.
fn cell_of(x: i32, z: i32, centre: (i32, i32), step: i32, area: Rect) -> Option<(usize, usize)> {
    let w = area.width as i32;
    let h = area.height as i32;
    let col = w / 2 + (x - centre.0) / step;
    let row = h / 2 - (z - centre.1) / step;
    if col < 0 || row < 0 || col >= w || row >= h {
        return None;
    }
    Some((col as usize, row as usize))
}

/// Write `glyph` into the buffer cell unless it is outside `area`.
fn put(buf: &mut Buffer, area: Rect, col: usize, row: usize, glyph: &str) {
    if col < area.width as usize && row < area.height as usize {
        buf[(area.x + col as u16, area.y + row as u16)].set_symbol(glyph);
    }
}

fn poi_glyph(kind: PoiKind) -> &'static str {
    match kind {
        PoiKind::Bank => "B",
        PoiKind::Transport => "T",
        PoiKind::Teleport => "X",
        _ => "·",
    }
}

#[cfg(test)]
mod tests {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    use nav::grid::StepGrid;
    use nav::router::find;
    use nav::tile::Tile;

    use super::{Map, MapAction, MapView, ZOOMS};

    fn wtile(x: i32, z: i32, level: i32) -> api::snapshot::WorldTile {
        api::snapshot::WorldTile { x, z, level }
    }

    fn tile(x: i32, z: i32, level: i32) -> Tile {
        Tile { x, z, level }
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    /// Render `map` into a `w × h` TestBackend and return the buffer text.
    fn render(map: Map<'_, impl FnMut(Tile)>, w: u16, h: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
        terminal
            .draw(|frame| frame.render_widget(map, frame.area()))
            .unwrap();
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect()
    }

    #[test]
    fn here_at_centre_draws_the_at_marker() {
        let world = nav::world::NavWorld::from_grid(&StepGrid::fixture_open_3x3());
        let mut view = MapView::new();
        let text = render(
            Map::new(&world, &mut view, |_| {}).here(wtile(1, 1, 0)),
            9,
            9,
        );
        let buf_text = text;
        // 9×9, centre tile (1,1) → cell (4,4): `@` wins over the dot.
        assert_eq!(&buf_text[4 * 9 + 4..4 * 9 + 5], "@");
        // Walkable neighbours keep the dot glyph.
        assert_eq!(&buf_text[3 * 9 + 3..3 * 9 + 4], ".");
        assert_eq!(&buf_text[5 * 9 + 5..5 * 9 + 6], ".");
        // Blocked (off-grid) cells stay blank.
        assert_eq!(&buf_text[0..1], " ");
    }

    #[test]
    fn route_paints_stars_and_advancing_here_drops_the_first() {
        let world = nav::world::NavWorld::from_grid(&StepGrid::fixture_rect_at(
            Tile {
                x: 0,
                z: 0,
                level: 0,
            },
            5,
            1,
        ));
        let route = find(
            &world.collision,
            &world.graph,
            wtile(0, 0, 0),
            wtile(4, 0, 0),
        )
        .expect("open corridor routes");

        let mut view = MapView::new();
        let first = render(
            Map::new(&world, &mut view, |_| {})
                .here(wtile(0, 0, 0))
                .route(&route),
            9,
            5,
        );
        let stars = first.chars().filter(|c| *c == '*').count();
        assert_eq!(stars, 4, "walk tiles 1..4 paint `*`");
        assert_eq!(&first[2 * 9 + 4..2 * 9 + 5], "@", "here at the origin");

        let second = render(
            Map::new(&world, &mut view, |_| {})
                .here(wtile(1, 0, 0))
                .route(&route),
            9,
            5,
        );
        // The view re-centres on the moved player, so the first `*` tile
        // (1,0) is now the `@` marker at the centre column.
        assert_eq!(
            &second[2 * 9 + 4..2 * 9 + 5],
            "@",
            "the consumed first `*` tile is now here"
        );
        assert_eq!(
            second.chars().filter(|c| *c == '*').count(),
            stars - 1,
            "advancing here consumes the first `*`"
        );
    }

    #[test]
    fn arrows_and_hjkl_pan_the_view() {
        let world = nav::world::NavWorld::from_grid(&StepGrid::fixture_open_3x3());
        let mut view = MapView::new();
        let mut map = Map::new(&world, &mut view, |_| {}).here(wtile(1, 1, 0));

        assert_eq!(map.on_key(key(KeyCode::Right)), MapAction::Moved);
        assert_eq!(map.view.pan, (1, 0));
        assert_eq!(map.on_key(key(KeyCode::Char('h'))), MapAction::Moved);
        assert_eq!(map.view.pan, (0, 0));
        // North-up camera: j/down is south (centre -z), k/up is north
        // (centre +z), mirroring Right = east.
        assert_eq!(map.on_key(key(KeyCode::Char('j'))), MapAction::Moved);
        assert_eq!(map.view.pan, (0, -1));
        assert_eq!(map.on_key(key(KeyCode::Char('k'))), MapAction::Moved);
        assert_eq!(map.view.pan, (0, 0));
        assert_eq!(map.on_key(key(KeyCode::Char('l'))), MapAction::Moved);
        assert_eq!(map.view.pan, (1, 0));
        assert_eq!(map.on_key(key(KeyCode::Up)), MapAction::Moved);
        assert_eq!(map.view.pan, (1, 1));
        assert_eq!(map.on_key(key(KeyCode::Down)), MapAction::Moved);
        assert_eq!(map.view.pan, (1, 0));

        // A panned view shifts the here marker: centre east one tile puts
        // the player one cell west of centre.
        let text = render(
            Map::new(&world, &mut view, |_| {}).here(wtile(1, 1, 0)),
            9,
            9,
        );
        assert_eq!(
            &text[4 * 9 + 3..4 * 9 + 4],
            "@",
            "pan right moves the player left"
        );

        // Pan north (centre +z): on the north-up buffer the player tile
        // sinks one row.
        let mut map = Map::new(&world, &mut view, |_| {}).here(wtile(1, 1, 0));
        assert_eq!(map.on_key(key(KeyCode::Char('h'))), MapAction::Moved);
        assert_eq!(map.view.pan, (0, 0));
        assert_eq!(map.on_key(key(KeyCode::Char('k'))), MapAction::Moved);
        assert_eq!(map.view.pan, (0, 1));
        let text = render(
            Map::new(&world, &mut view, |_| {}).here(wtile(1, 1, 0)),
            9,
            9,
        );
        assert_eq!(
            &text[5 * 9 + 4..5 * 9 + 5],
            "@",
            "pan north moves the player down the buffer"
        );
    }

    #[test]
    fn enter_selects_the_centre_tile_and_plus_paints_it() {
        let world = nav::world::NavWorld::from_grid(&StepGrid::fixture_open_3x3());
        let mut view = MapView::new();
        let mut map = Map::new(&world, &mut view, |_| {}).here(wtile(1, 1, 0));

        assert_eq!(map.on_key(key(KeyCode::Right)), MapAction::Moved);
        assert_eq!(map.on_key(key(KeyCode::Enter)), MapAction::Moved);
        assert_eq!(
            map.view.selection,
            Some(tile(2, 1, 0)),
            "Enter selects the panned centre tile"
        );
        let text = render(
            Map::new(&world, &mut view, |_| {}).here(wtile(1, 1, 0)),
            9,
            9,
        );
        assert_eq!(
            &text[4 * 9 + 4..4 * 9 + 5],
            "+",
            "the selection crosshair paints"
        );
        assert_eq!(
            &text[4 * 9 + 3..4 * 9 + 4],
            "@",
            "the here marker stays on the player"
        );
    }

    #[test]
    fn enter_selects_the_requested_centre_even_when_blocked() {
        let world = nav::world::NavWorld::from_grid(&StepGrid::fixture_open_3x3());
        let mut view = MapView::new();
        view.pan = (2, 2); // centre (3,3): off the 3x3 grid at (0,0).
        let mut map = Map::new(&world, &mut view, |_| {}).here(wtile(1, 1, 0));
        assert_eq!(map.on_key(key(KeyCode::Enter)), MapAction::Moved);
        assert_eq!(
            map.view.selection,
            Some(tile(3, 3, 0)),
            "selection is destination-only; Walk snap lives in MapModel"
        );
    }

    #[test]
    fn enter_confirms_walk_to_the_selection_through_the_hook() {
        let world = nav::world::NavWorld::from_grid(&StepGrid::fixture_open_3x3());
        let mut view = MapView::new();
        view.selection = Some(tile(2, 2, 0));
        let mut walked: Vec<Tile> = Vec::new();
        let mut map = Map::new(&world, &mut view, |t| walked.push(t));
        assert_eq!(
            map.on_key(key(KeyCode::Enter)),
            MapAction::Walk(tile(2, 2, 0))
        );
        assert_eq!(
            walked,
            vec![tile(2, 2, 0)],
            "the hook receives the selection"
        );
        assert_eq!(
            view.selection,
            Some(tile(2, 2, 0)),
            "the selection stays until Esc clears it"
        );
    }

    #[test]
    fn esc_clears_the_selection() {
        let world = nav::world::NavWorld::from_grid(&StepGrid::fixture_open_3x3());
        let mut view = MapView::new();
        view.selection = Some(tile(2, 2, 0));
        let mut map = Map::new(&world, &mut view, |_| {});
        assert_eq!(map.on_key(key(KeyCode::Esc)), MapAction::Moved);
        assert_eq!(view.selection, None);
    }

    #[test]
    fn zoom_keys_cycle_the_tiles_per_cell_steps() {
        let world = nav::world::NavWorld::from_grid(&StepGrid::fixture_open_3x3());
        let mut view = MapView::new();
        let mut map = Map::new(&world, &mut view, |_| {});
        let finest = ZOOMS.len() - 1;
        assert_eq!(
            map.view.zoom, finest,
            "a fresh view starts at the finest zoom"
        );
        for _ in 0..finest {
            assert_eq!(map.on_key(key(KeyCode::Char('-'))), MapAction::Moved);
        }
        assert_eq!(map.view.zoom, 0);
        assert_eq!(
            map.on_key(key(KeyCode::Char('-'))),
            MapAction::Moved,
            "clamped"
        );
        assert_eq!(map.view.zoom, 0);
        for _ in 0..finest {
            assert_eq!(map.on_key(key(KeyCode::Char('+'))), MapAction::Moved);
        }
        assert_eq!(map.view.zoom, finest);
        assert_eq!(
            map.on_key(key(KeyCode::Char('='))),
            MapAction::Moved,
            "clamped"
        );
        assert_eq!(map.view.zoom, finest);
    }

    #[test]
    fn unknown_keys_are_ignored() {
        let world = nav::world::NavWorld::from_grid(&StepGrid::fixture_open_3x3());
        let mut view = MapView::new();
        let mut map = Map::new(&world, &mut view, |_| {});
        assert_eq!(map.on_key(key(KeyCode::F(1))), MapAction::Ignored);
        assert_eq!(view.pan, (0, 0));
        assert_eq!(view.selection, None);
    }
}
