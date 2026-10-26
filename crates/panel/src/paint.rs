//! Script-paint overlay for the Game Image.
//!
//! Structured Paint is drawn on the Game window draw list over the client's
//! chatbox rect — never a second ImGui window and never pixels on the
//! 765×503 game texture. Canvas ops rasterize to a small cached transparent
//! texture over the applet, using the same native font as measureText.
//!
//! The style and the layout live in `frontend-core`
//! (`scripts::paint_style`, `scripts::PaintLayout`). This module only
//! draws: it measures one cell width, asks the layout for the frame's
//! applet-space rectangles, maps each one with `min + v * s`, draws
//! fills, borders and text on the draw list, and hit-tests the same
//! rectangles. Every label takes the body font size on the draw list,
//! so no text is scaled twice on a scaled display.

use dear_imgui_rs::{DrawListMut, MouseButton, TextureId, Ui};
use frontend_core::scripts::paint_style as style;
use frontend_core::scripts::{
    clipped_prefix, map_applet_rect, CanvasCache, CanvasOps, CanvasRaster, PaintLayout, PaintRect,
    PaintView,
};

use crate::game_view::{FrameGpu, APPLET_H, APPLET_W};
use crate::theme::PANEL_FONT_SIZE;

/// Applet-space chatbox rect `(x, y, w, h)` the client reserves for game
/// chat on the 765×503 stage.
pub const CHATBOX: [f32; 4] = style::PANEL_RECT;

/// One script-paint interaction the overlay detected this frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PaintFrameHit {
    /// The `index`-th advertised paint button; the core decides what it
    /// does ([`frontend_core::scripts::PaintView::activate`]).
    Button(usize),
    /// Persistent strip/rail/tabs selection (`paint_select`).
    Select { key: String, name: String },
}

/// Map the applet-space chatbox onto the Game Image's display rect
/// (`min` = the Image widget's top-left corner, `size` = its display
/// size, which is the native 765×503 in single-bot mode).
pub fn chatbox_rect(min: [f32; 2], size: [f32; 2]) -> [f32; 4] {
    let sx = size[0] / APPLET_W as f32;
    let sy = size[1] / APPLET_H as f32;
    [
        min[0] + CHATBOX[0] * sx,
        min[1] + CHATBOX[1] * sy,
        CHATBOX[2] * sx,
        CHATBOX[3] * sy,
    ]
}

/// Uniform scale for structured paint text/padding/hit targets from the
/// Game Image display size. `fit_applet` keeps aspect so sx == sy; `min`
/// avoids stretching glyphs if a non-matching rect is passed.
pub fn paint_uniform_scale(size: [f32; 2]) -> f32 {
    (size[0] / APPLET_W as f32)
        .min(size[1] / APPLET_H as f32)
        .max(0.01)
}

/// The measured cell width in applet pixels: the width of "0" at the
/// body font size. The layout counts every text width in these cells.
fn cell_width(ui: &Ui) -> f32 {
    ui.current_font()
        .calc_text_size(PANEL_FONT_SIZE, f32::MAX, 0.0, "0")[0]
}

/// An sRGB palette colour as the draw list's float colour.
fn rgba(colour: [u8; 4]) -> [f32; 4] {
    [
        colour[0] as f32 / 255.0,
        colour[1] as f32 / 255.0,
        colour[2] as f32 / 255.0,
        colour[3] as f32 / 255.0,
    ]
}

/// An opaque sRGB colour as the draw list's float colour.
fn rgb(colour: [u8; 3]) -> [f32; 4] {
    [
        colour[0] as f32 / 255.0,
        colour[1] as f32 / 255.0,
        colour[2] as f32 / 255.0,
        1.0,
    ]
}

/// Map one applet-space rect to screen corners `[min_x, min_y, max_x,
/// max_y]` at the Game Image's display rect.
fn screen_rect(rect: PaintRect, min: [f32; 2], s: f32) -> [f32; 4] {
    [
        min[0] + rect.x() * s,
        min[1] + rect.y() * s,
        min[0] + rect.right() * s,
        min[1] + rect.bottom() * s,
    ]
}

/// Map one applet-space point to a screen point.
fn screen_point(x: f32, y: f32, min: [f32; 2], s: f32) -> [f32; 2] {
    [min[0] + x * s, min[1] + y * s]
}

fn fill_rect(dl: &DrawListMut<'_>, rect: [f32; 4], colour: [f32; 4]) {
    dl.add_rect([rect[0], rect[1]], [rect[2], rect[3]], colour)
        .filled(true)
        .build();
}

fn border_rect(dl: &DrawListMut<'_>, rect: [f32; 4], colour: [f32; 4], thickness: f32) {
    dl.add_rect([rect[0], rect[1]], [rect[2], rect[3]], colour)
        .thickness(thickness)
        .build();
}

fn hovered(ui: &Ui, rect: [f32; 4]) -> bool {
    ui.is_mouse_hovering_rect([rect[0], rect[1]], [rect[2], rect[3]])
}

/// One frame's text painter: the draw list, the body font size and the
/// screen cell width every paint text item shares.
struct Painter<'u> {
    ui: &'u Ui,
    dl: &'u DrawListMut<'u>,
    font_sz: f32,
    cell_screen: f32,
}

impl Painter<'_> {
    /// Draw `text` at screen point `pos`, clipped to `cells` cells. A
    /// clipped text draws its prefix and then "…" in the last cell, so
    /// a reader always sees that text was cut. `bold` draws the text
    /// twice, one physical pixel apart (the panel has no bold face).
    /// Returns the drawn width in screen pixels.
    fn text(&self, pos: [f32; 2], colour: [f32; 4], text: &str, cells: usize, bold: bool) -> f32 {
        let (prefix, ellipsis) = clipped_prefix(text, cells);
        if prefix.is_empty() && !ellipsis {
            return 0.0;
        }
        let font = self.ui.current_font();
        let mut drawn = 0usize;
        if !prefix.is_empty() {
            self.dl
                .add_text_with_font(font, self.font_sz, pos, colour, prefix, 0.0, None);
            if bold {
                self.dl.add_text_with_font(
                    font,
                    self.font_sz,
                    [pos[0] + 1.0, pos[1]],
                    colour,
                    prefix,
                    0.0,
                    None,
                );
            }
            drawn = prefix.chars().count();
        }
        if ellipsis {
            let at = [pos[0] + drawn as f32 * self.cell_screen, pos[1]];
            self.dl
                .add_text_with_font(font, self.font_sz, at, colour, "…", 0.0, None);
            if bold {
                self.dl.add_text_with_font(
                    font,
                    self.font_sz,
                    [at[0] + 1.0, at[1]],
                    colour,
                    "…",
                    0.0,
                    None,
                );
            }
            drawn += 1;
        }
        drawn as f32 * self.cell_screen + if bold { 1.0 } else { 0.0 }
    }
}

/// One chrome entry a test can find again: which band and name a hit
/// rect belongs to. Production frames keep no such list. They build a
/// selection's store key only on the click itself.
#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ChromeHit {
    Strip(usize),
    Tabs { band: usize, name: usize },
    Rail(usize),
}

struct CanvasGpu {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    tex_id: TextureId,
    w: u32,
    h: u32,
}

/// Cached script-paint overlay for the focused slot.
pub struct PaintOverlay {
    /// `true` = title-only (rs2b0t `paint:collapsed`). View-local: reset
    /// when the paint goes away.
    collapsed: bool,
    /// The frame's layout, refilled each frame. Its buffers are reused,
    /// so a steady frame allocates nothing.
    layout: PaintLayout,
    /// Whether the pointer was inside the drawn paint panel on the
    /// last frame. The Game panes read this after the overlay: a
    /// press inside the panel belongs to the paint, so its mouse
    /// buttons are not also streamed to the game under it.
    pointer_inside_panel: bool,
    /// The paint's title + rows submitted this frame (empty while
    /// collapsed or when no paint is showing). The GPU-less tests assert
    /// on this mirror of the window text. Its strings are refilled in
    /// place, so the mirror itself allocates nothing on a steady frame.
    #[cfg(test)]
    lines: Vec<String>,
    #[cfg(test)]
    lines_len: usize,
    /// Advertised button labels drawn this frame (empty while collapsed).
    #[cfg(test)]
    button_labels: Vec<String>,
    #[cfg(test)]
    button_labels_len: usize,
    /// Screen-space button rects `(min, max)` for GPU-less click tests.
    #[cfg(test)]
    button_hits: Vec<[f32; 4]>,
    /// Chrome hit targets for GPU-less click tests.
    #[cfg(test)]
    chrome_hits: Vec<(ChromeHit, [f32; 4])>,
    #[cfg(test)]
    strip_status_label_rect: Option<[f32; 4]>,
    #[cfg(test)]
    strip_brand_label_rect: Option<[f32; 4]>,
    /// Screen-space canvas dest `(x, y, w, h)` this frame, if any.
    canvas_dest: Option<[f32; 4]>,
    /// Applet-space dirty rect this frame, if any.
    canvas_dirty: Option<[i32; 4]>,
    last_ops: CanvasCache,
    last_dirty: Option<[i32; 4]>,
    canvas_gpu: Option<CanvasGpu>,
    #[cfg(test)]
    rasterize_calls: u32,
    #[cfg(test)]
    upload_calls: u32,
}

/// Refill `slots` entry `len` with `text`, reusing its buffer, and
/// count it. A frame ends by truncating to the count it wrote.
#[cfg(test)]
fn mirror_text(slots: &mut Vec<String>, len: &mut usize, text: &str) {
    match slots.get_mut(*len) {
        Some(slot) => {
            if slot != text {
                slot.clear();
                slot.push_str(text);
            }
        }
        None => slots.push(text.to_owned()),
    }
    *len += 1;
}

impl PaintOverlay {
    pub fn new() -> Self {
        Self {
            collapsed: false,
            layout: PaintLayout::new(),
            pointer_inside_panel: false,
            #[cfg(test)]
            lines: Vec::new(),
            #[cfg(test)]
            lines_len: 0,
            #[cfg(test)]
            button_labels: Vec::new(),
            #[cfg(test)]
            button_labels_len: 0,
            #[cfg(test)]
            button_hits: Vec::new(),
            #[cfg(test)]
            chrome_hits: Vec::new(),
            #[cfg(test)]
            strip_status_label_rect: None,
            #[cfg(test)]
            strip_brand_label_rect: None,
            canvas_dest: None,
            canvas_dirty: None,
            last_ops: CanvasCache::default(),
            last_dirty: None,
            canvas_gpu: None,
            #[cfg(test)]
            rasterize_calls: 0,
            #[cfg(test)]
            upload_calls: 0,
        }
    }

    /// Close this frame's test mirrors at the counts the frame wrote.
    #[cfg(test)]
    fn finish_mirrors(&mut self) {
        self.lines.truncate(self.lines_len);
        self.button_labels.truncate(self.button_labels_len);
    }

    /// Unregister the cached canvas texture (Stop / owner teardown).
    pub fn release_canvas(&mut self, gpu: &mut dyn FrameGpu) {
        if let Some(cached) = self.canvas_gpu.take() {
            gpu.unregister_texture(cached.tex_id);
        }
        self.last_ops.clear();
        self.last_dirty = None;
        self.canvas_dest = None;
        self.canvas_dirty = None;
    }

    #[cfg(test)]
    pub fn canvas_gpu_alive(&self) -> bool {
        self.canvas_gpu.is_some()
    }

    #[cfg(test)]
    pub fn rasterize_calls(&self) -> u32 {
        self.rasterize_calls
    }

    #[cfg(test)]
    pub fn upload_calls(&self) -> u32 {
        self.upload_calls
    }

    /// Whether the pointer was inside the drawn paint panel on the
    /// last [`frame`](Self::frame) call. A press that starts while
    /// this is true is the paint's (a button, the toggle, a tab, or
    /// the panel itself) and must not reach the game. A release is
    /// not decided here. It follows the press that started it.
    pub fn pointer_inside_panel(&self) -> bool {
        self.pointer_inside_panel
    }

    /// Draw the focused slot's paint over the Game Image. `min`/`size`
    /// are the Image widget's display rect. No-op without a paint. Returns
    /// the advertised button id clicked this frame, if any.
    pub fn frame(
        &mut self,
        ui: &Ui,
        gpu: Option<&mut dyn FrameGpu>,
        paint: Option<PaintView<'_>>,
        min: [f32; 2],
        size: [f32; 2],
    ) -> Option<(PaintFrameHit, u64)> {
        #[cfg(test)]
        {
            self.lines_len = 0;
            self.button_labels_len = 0;
            self.button_hits.clear();
            self.chrome_hits.clear();
            self.strip_status_label_rect = None;
            self.strip_brand_label_rect = None;
        }
        self.canvas_dest = None;
        self.canvas_dirty = None;
        self.pointer_inside_panel = false;

        let structured = paint.filter(|p| {
            p.title().is_some()
                || !p.lines().is_empty()
                || p.buttons().len() != 0
                || p.strip().is_some()
                || p.rail().is_some_and(|r| !r.names().is_empty())
                || p.footer().is_some()
                || p.tabs().len() != 0
        });
        let canvas_ops = paint.map(|p| p.canvas()).filter(|ops| !ops.is_empty());

        if structured.is_none() && canvas_ops.is_none() {
            self.collapsed = false;
            if let Some(gpu) = gpu {
                self.release_canvas(gpu);
            } else {
                self.last_ops.clear();
            }
            #[cfg(test)]
            self.finish_mirrors();
            return None;
        }

        if let Some(canvas_ops) = canvas_ops {
            let ops_unchanged = self.last_ops.same_as(canvas_ops);
            let dirty = if ops_unchanged {
                self.last_dirty
            } else {
                canvas_ops.dirty_bounds().inspect(|arr| {
                    self.last_dirty = Some(*arr);
                })
            };
            if let Some([dx, dy, dw, dh]) = dirty {
                self.canvas_dirty = Some([dx, dy, dw, dh]);
                let dest = map_applet_rect(min, size, dx, dy, dw, dh);
                self.canvas_dest = Some(dest);
                if let Some(gpu) = gpu {
                    let reuse_tex = ops_unchanged && self.canvas_gpu.is_some();
                    if !reuse_tex {
                        if let Some(raster) = canvas_ops.rasterize() {
                            #[cfg(test)]
                            {
                                self.rasterize_calls = self.rasterize_calls.saturating_add(1);
                            }
                            self.sync_canvas_texture(gpu, &raster, canvas_ops);
                        }
                    }
                    if let Some(cached) = &self.canvas_gpu {
                        let dl = ui.get_window_draw_list();
                        dl.add_image(
                            cached.tex_id,
                            [dest[0], dest[1]],
                            [dest[0] + dest[2], dest[1] + dest[3]],
                            [0.0, 0.0],
                            [1.0, 1.0],
                            [1.0, 1.0, 1.0, 1.0],
                        );
                    }
                } else if !ops_unchanged {
                    self.last_ops.store(canvas_ops);
                }
            } else if let Some(gpu) = gpu {
                self.release_canvas(gpu);
            } else {
                self.last_ops.clear();
                self.last_dirty = None;
            }
        } else {
            if let Some(gpu) = gpu {
                self.release_canvas(gpu);
            } else {
                self.last_ops.clear();
                self.last_dirty = None;
            }
        }

        let Some(paint) = structured else {
            #[cfg(test)]
            self.finish_mirrors();
            return None;
        };
        let s = paint_uniform_scale(size);
        let font_sz = (PANEL_FONT_SIZE * s).max(1.0);
        let cell_w = cell_width(ui);
        let cell_screen = cell_w * s;
        let accent = rgb(paint.accent_rgb().unwrap_or(style::DEFAULT_ACCENT));
        self.layout.compute(paint, self.collapsed, cell_w);
        // Collapse acts on mouse down, only inside the toggle square.
        // A click flips the state and this same frame draws the result.
        let toggle_rect = screen_rect(self.layout.toggle(), min, s);
        let toggle_hovered = hovered(ui, toggle_rect);
        if toggle_hovered && ui.is_mouse_clicked(MouseButton::Left) {
            self.collapsed = !self.collapsed;
            self.layout.compute(paint, self.collapsed, cell_w);
        }
        let clicked = ui.is_mouse_clicked(MouseButton::Left);
        let dl = ui.get_window_draw_list();
        let painter = Painter {
            ui,
            dl: &dl,
            font_sz,
            cell_screen,
        };
        let panel_rect = screen_rect(self.layout.panel(), min, s);
        self.pointer_inside_panel = hovered(ui, panel_rect);
        let _clip = dl.push_clip_rect(
            [panel_rect[0], panel_rect[1]],
            [panel_rect[2], panel_rect[3]],
            true,
        );
        fill_rect(&dl, panel_rect, rgba(style::BG));
        let title_rect = screen_rect(self.layout.title_bar(), min, s);
        fill_rect(&dl, title_rect, rgba(style::TITLE_BG));
        border_rect(&dl, panel_rect, rgba(style::BORDER), s);
        border_rect(&dl, title_rect, rgba(style::BORDER), s);
        let title_text_y = self.layout.title_bar().y() + style::TITLE_TEXT_DY;
        let mut chrome_click: Option<PaintFrameHit> = None;
        let mut button_click: Option<usize> = None;

        if let Some(strip) = paint.strip() {
            for at in 0..self.layout.strip_tabs().len() {
                let item = self.layout.strip_tabs()[at];
                let name = &strip.names()[item.name_index()];
                let rect = screen_rect(item.rect(), min, s);
                let active = strip.selected() == name.as_str();
                let over = hovered(ui, rect);
                if active {
                    fill_rect(&dl, rect, rgba(style::WIDGET_HOT));
                } else if over {
                    fill_rect(&dl, rect, rgba(style::WIDGET));
                }
                let origin = item.text_origin();
                painter.text(
                    screen_point(origin[0], origin[1], min, s),
                    if active { accent } else { rgba(style::FG_DIM) },
                    name,
                    item.cells(),
                    true,
                );
                #[cfg(test)]
                self.chrome_hits
                    .push((ChromeHit::Strip(item.name_index()), rect));
                if over && clicked {
                    chrome_click = Some(PaintFrameHit::Select {
                        key: format!("strip:{}", strip.id()),
                        name: name.clone(),
                    });
                }
            }
            if let (Some(status), Some(slot)) = (strip.status(), self.layout.strip_status()) {
                let pos = screen_point(slot.rect().x(), title_text_y, min, s);
                let _drawn = painter.text(pos, rgba(style::STATUS), status, slot.cells(), false);
                #[cfg(test)]
                if _drawn > 0.0 {
                    self.strip_status_label_rect =
                        Some([pos[0], pos[1], pos[0] + _drawn, pos[1] + font_sz]);
                }
            }
            let brand = strip.brand().or_else(|| paint.title()).unwrap_or("");
            if let Some(slot) = self.layout.strip_brand() {
                let pos = screen_point(slot.rect().x(), title_text_y, min, s);
                let _drawn = painter.text(pos, accent, brand, slot.cells(), true);
                #[cfg(test)]
                {
                    mirror_text(&mut self.lines, &mut self.lines_len, brand);
                    self.strip_brand_label_rect =
                        Some([pos[0], pos[1], pos[0] + _drawn, pos[1] + font_sz]);
                }
            }
        } else if let Some(title) = paint.title() {
            painter.text(
                screen_point(
                    self.layout.title_bar().x() + style::PAD,
                    title_text_y,
                    min,
                    s,
                ),
                accent,
                title,
                self.layout.title_cells(),
                true,
            );
        }
        painter.text(
            screen_point(
                self.layout.toggle().x() + style::TOGGLE_TEXT_DX,
                title_text_y,
                min,
                s,
            ),
            if toggle_hovered {
                rgba(style::FG)
            } else {
                rgba(style::FG_DIM)
            },
            if self.collapsed { "+" } else { "-" },
            usize::MAX,
            false,
        );

        if !self.collapsed {
            #[cfg(test)]
            {
                if paint.strip().is_none() {
                    if let Some(title) = paint.title() {
                        mirror_text(&mut self.lines, &mut self.lines_len, title);
                    }
                }
                for row in paint.lines() {
                    mirror_text(&mut self.lines, &mut self.lines_len, row);
                }
                for button in paint.buttons() {
                    mirror_text(
                        &mut self.button_labels,
                        &mut self.button_labels_len,
                        button.label(),
                    );
                }
            }
            for at in 0..self.layout.tabs().len() {
                let item = self.layout.tabs()[at];
                let Some(band) = paint.tabs().nth(item.band_index()) else {
                    continue;
                };
                let name = &band.names()[item.name_index()];
                let rect = screen_rect(item.rect(), min, s);
                let active = band.selected() == name.as_str();
                let over = hovered(ui, rect);
                if active {
                    fill_rect(&dl, rect, rgba(style::WIDGET_HOT));
                    fill_rect(
                        &dl,
                        [
                            rect[0],
                            rect[3] - style::TAB_UNDERLINE_H * s,
                            rect[2],
                            rect[3],
                        ],
                        accent,
                    );
                } else if over {
                    fill_rect(&dl, rect, rgba(style::WIDGET));
                }
                let origin = item.text_origin();
                painter.text(
                    screen_point(origin[0], origin[1], min, s),
                    if active {
                        rgba(style::WHITE)
                    } else {
                        rgba(style::FG_DIM)
                    },
                    name,
                    item.cells(),
                    false,
                );
                #[cfg(test)]
                self.chrome_hits.push((
                    ChromeHit::Tabs {
                        band: item.band_index(),
                        name: item.name_index(),
                    },
                    rect,
                ));
                if over && clicked {
                    chrome_click = Some(PaintFrameHit::Select {
                        key: format!("tabs:{}", band.id()),
                        name: name.clone(),
                    });
                }
            }
            if let Some(rail) = paint.rail() {
                for at in 0..self.layout.rail().len() {
                    let item = self.layout.rail()[at];
                    let name = &rail.names()[item.name_index()];
                    let rect = screen_rect(item.rect(), min, s);
                    let active = rail.selected() == name.as_str();
                    let over = hovered(ui, rect);
                    if active {
                        fill_rect(&dl, rect, rgba(style::WIDGET_HOT));
                        fill_rect(
                            &dl,
                            [
                                rect[0],
                                rect[1],
                                rect[0] + style::RAIL_MARKER_W * s,
                                rect[3],
                            ],
                            accent,
                        );
                    } else if over {
                        fill_rect(&dl, rect, rgba(style::WIDGET));
                    }
                    let origin = item.text_origin();
                    painter.text(
                        screen_point(origin[0], origin[1], min, s),
                        if active {
                            rgba(style::WHITE)
                        } else {
                            rgba(style::FG_DIM)
                        },
                        name,
                        item.cells(),
                        false,
                    );
                    #[cfg(test)]
                    self.chrome_hits
                        .push((ChromeHit::Rail(item.name_index()), rect));
                    if over && clicked {
                        chrome_click = Some(PaintFrameHit::Select {
                            key: format!("rail:{}", rail.id()),
                            name: name.clone(),
                        });
                    }
                }
            }
            for at in 0..self.layout.rows().len() {
                let item = self.layout.rows()[at];
                painter.text(
                    screen_point(item.rect().x(), item.rect().y() + style::TEXT_DY, min, s),
                    rgba(style::FG),
                    &paint.lines()[item.index()],
                    item.cells(),
                    false,
                );
            }
            // Buttons act on mouse down only, inside their rect. A host
            // control the run does not take now draws with a dim label
            // and reports no click.
            for at in 0..self.layout.buttons().len() {
                let item = self.layout.buttons()[at];
                let Some(button) = paint.buttons().nth(item.index()) else {
                    continue;
                };
                let rect = screen_rect(item.rect(), min, s);
                let over = hovered(ui, rect);
                fill_rect(
                    &dl,
                    rect,
                    if button.enabled() && over {
                        rgba(style::WIDGET_HOT)
                    } else {
                        rgba(style::WIDGET)
                    },
                );
                border_rect(&dl, rect, rgba(style::BORDER), s);
                painter.text(
                    screen_point(
                        item.rect().x() + style::BUTTON_TEXT_DX,
                        item.rect().y() + style::BUTTON_TEXT_DY,
                        min,
                        s,
                    ),
                    if button.enabled() {
                        rgba(style::FG)
                    } else {
                        rgba(style::FG_DIM)
                    },
                    button.label(),
                    item.label_cells(),
                    false,
                );
                #[cfg(test)]
                self.button_hits.push(rect);
                if button.enabled() && over && clicked {
                    button_click = Some(item.index());
                }
            }
            if let (Some(footer), Some(place)) = (paint.footer(), self.layout.footer()) {
                let origin = place.origin();
                painter.text(
                    screen_point(origin[0], origin[1], min, s),
                    rgba(style::FG_DIM),
                    footer,
                    place.cells(),
                    false,
                );
            }
        }
        #[cfg(test)]
        self.finish_mirrors();
        if let Some(hit) = chrome_click {
            Some((hit, paint.generation()))
        } else {
            button_click.map(|index| (PaintFrameHit::Button(index), paint.generation()))
        }
    }

    fn sync_canvas_texture(
        &mut self,
        gpu: &mut dyn FrameGpu,
        raster: &CanvasRaster,
        ops: CanvasOps<'_>,
    ) {
        let reuse = self.last_ops.same_as(ops)
            && self
                .canvas_gpu
                .as_ref()
                .is_some_and(|c| c.w == raster.width() && c.h == raster.height());
        if reuse {
            return;
        }
        self.last_ops.store(ops);
        if self
            .canvas_gpu
            .as_ref()
            .is_some_and(|c| c.w != raster.width() || c.h != raster.height())
        {
            if let Some(old) = self.canvas_gpu.take() {
                gpu.unregister_texture(old.tex_id);
            }
        }
        if self.canvas_gpu.is_none() {
            let texture = gpu.device().create_texture(&wgpu::TextureDescriptor {
                label: Some("274 script canvas"),
                size: wgpu::Extent3d {
                    width: raster.width().max(1),
                    height: raster.height().max(1),
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            let tex_id = gpu.register_texture(&texture, &view);
            self.canvas_gpu = Some(CanvasGpu {
                texture,
                view,
                tex_id,
                w: raster.width().max(1),
                h: raster.height().max(1),
            });
        }
        let cached = self.canvas_gpu.as_ref().expect("canvas texture");
        gpu.queue().write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &cached.texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            raster.rgba(),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4 * raster.width().max(1)),
                rows_per_image: Some(raster.height().max(1)),
            },
            wgpu::Extent3d {
                width: raster.width().max(1),
                height: raster.height().max(1),
                depth_or_array_layers: 1,
            },
        );
        #[cfg(test)]
        {
            self.upload_calls = self.upload_calls.saturating_add(1);
        }
        let _ = &cached.view;
    }
}

impl Default for PaintOverlay {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use dear_imgui_rs::FramePrepareOptions;

    use super::{
        chatbox_rect, paint_uniform_scale, ChromeHit, PaintFrameHit, PaintOverlay, CHATBOX,
    };
    #[cfg(feature = "js")]
    use crate::test_support::headless_gpu;
    #[cfg(feature = "js")]
    use frontend_core::test_support::paint::caption_raster;
    use frontend_core::test_support::paint::TestPaint;

    fn paint(title: Option<&str>, lines: &[&str]) -> TestPaint {
        TestPaint::new(title, lines)
    }

    fn paint_with_button(title: Option<&str>, lines: &[&str], id: &str, label: &str) -> TestPaint {
        paint(title, lines).with_button(id, label)
    }

    fn prepare_frame(ctx: &mut dear_imgui_rs::Context) {
        ctx.prepare_frame(
            FramePrepareOptions::new([900.0, 700.0], 1.0 / 60.0).renderer_has_textures(),
        );
    }

    #[test]
    fn chatbox_rect_maps_the_game_image_to_the_client_chatbox() {
        // Native 765×503 blit: the chatbox keeps its applet-space spot.
        assert_eq!(
            chatbox_rect([10.0, 20.0], [765.0, 503.0]),
            [18.0, 365.0, 506.0, 150.0]
        );
        // A scaled blit (grid mode) scales the chatbox with the image.
        let [x, y, w, h] = chatbox_rect([0.0, 0.0], [382.5, 251.5]);
        assert!((x - CHATBOX[0] * 0.5).abs() < 0.01);
        assert!((y - CHATBOX[1] * 0.5).abs() < 0.01);
        assert!((w - CHATBOX[2] * 0.5).abs() < 0.01);
        assert!((h - CHATBOX[3] * 0.5).abs() < 0.01);
    }

    #[test]
    fn paint_window_shows_title_and_lines_over_the_chatbox() {
        let _guard = crate::test_support::imgui_context_guard();
        let mut ctx = crate::test_support::imgui_context();
        let mut overlay = PaintOverlay::new();
        let p = paint(Some("BoneBurier"), &["a row", "second row"]);
        prepare_frame(&mut ctx);
        {
            let ui = ctx.frame();
            let _ = ui
                .window("Game")
                .position([0.0, 0.0], dear_imgui_rs::Condition::Always)
                .size([900.0, 700.0], dear_imgui_rs::Condition::Always)
                .build(|| {
                    let paint_frame = p.frame();
                    overlay.frame(
                        ui,
                        None,
                        Some(paint_frame.view()),
                        [10.0, 20.0],
                        [765.0, 503.0],
                    );
                });
        }
        ctx.render();
        assert_eq!(
            overlay.lines,
            vec![
                "BoneBurier".to_string(),
                "a row".to_string(),
                "second row".to_string()
            ],
            "the paint title and rows are in the overlay text"
        );
    }

    #[test]
    fn paint_window_hides_without_a_paint() {
        let _guard = crate::test_support::imgui_context_guard();
        let mut ctx = crate::test_support::imgui_context();
        let mut overlay = PaintOverlay::new();
        prepare_frame(&mut ctx);
        {
            let ui = ctx.frame();
            overlay.frame(ui, None, None, [10.0, 20.0], [765.0, 503.0]);
        }
        ctx.render();
        assert!(overlay.lines.is_empty(), "no paint -> no overlay text");
    }

    /// One frame with the overlay up; `mouse` + `left_down` simulate the
    /// pointer for the `–`/`+` title-row click.
    fn paint_click_frame(
        ctx: &mut dear_imgui_rs::Context,
        overlay: &mut PaintOverlay,
        p: &TestPaint,
        mouse: Option<[f32; 2]>,
        left_down: bool,
    ) {
        prepare_frame(ctx);
        if let Some(m) = mouse {
            ctx.io_mut().add_mouse_pos_event(m);
        }
        ctx.io_mut()
            .add_mouse_button_event(dear_imgui_rs::MouseButton::Left, left_down);
        {
            let ui = ctx.frame();
            let _ = ui
                .window("Game")
                .position([0.0, 0.0], dear_imgui_rs::Condition::Always)
                .size([900.0, 700.0], dear_imgui_rs::Condition::Always)
                .build(|| {
                    let paint_frame = p.frame();
                    overlay.frame(
                        ui,
                        None,
                        Some(paint_frame.view()),
                        [10.0, 20.0],
                        [765.0, 503.0],
                    );
                });
        }
        ctx.render();
    }

    /// The toggle square's centre for a frame at `[10, 20]` on a native
    /// 765x503 blit: the chatbox is `[18, 365, 506, 150]`, so the 20 px
    /// square at its top right is `[504, 365, 524, 385]`.
    const TOGGLE_CENTER: [f32; 2] = [514.0, 375.0];

    #[test]
    fn collapse_click_hides_body_and_shrinks_to_title_height() {
        let _guard = crate::test_support::imgui_context_guard();
        let mut ctx = crate::test_support::imgui_context();
        let mut overlay = PaintOverlay::new();
        let p = paint(Some("BoneBurier"), &["a row", "second row"]);
        // A press on the title text does not collapse the paint.
        let title = [30.0, 380.0];
        paint_click_frame(&mut ctx, &mut overlay, &p, Some(title), false);
        assert!(!overlay.collapsed, "expanded by default");
        paint_click_frame(&mut ctx, &mut overlay, &p, Some(title), true);
        assert!(
            !overlay.collapsed,
            "only the toggle square collapses the paint"
        );
        paint_click_frame(&mut ctx, &mut overlay, &p, Some(title), false);
        // The `-` toggle square collapses to title-only.
        paint_click_frame(&mut ctx, &mut overlay, &p, Some(TOGGLE_CENTER), false);
        paint_click_frame(&mut ctx, &mut overlay, &p, Some(TOGGLE_CENTER), true);
        assert!(overlay.collapsed, "`-` collapses to title-only");
        paint_click_frame(&mut ctx, &mut overlay, &p, Some(TOGGLE_CENTER), false);
        assert!(
            overlay.lines.is_empty(),
            "collapsed: body rows are not shown"
        );
        assert!(overlay.collapsed);
        // The `+` toggle square expands again.
        paint_click_frame(&mut ctx, &mut overlay, &p, Some(TOGGLE_CENTER), true);
        assert!(!overlay.collapsed, "`+` expands back");
        assert_eq!(
            overlay.lines,
            vec![
                "BoneBurier".to_string(),
                "a row".to_string(),
                "second row".to_string()
            ]
        );
    }

    #[test]
    fn advertised_button_is_a_real_control_and_collapsed_hides_it() {
        let _guard = crate::test_support::imgui_context_guard();
        let mut ctx = crate::test_support::imgui_context();
        let mut overlay = PaintOverlay::new();
        let p = paint_with_button(Some("NatureCrafter"), &["status"], "gobank", "Go bank");
        prepare_frame(&mut ctx);
        {
            let ui = ctx.frame();
            let _ = ui
                .window("Game")
                .position([0.0, 0.0], dear_imgui_rs::Condition::Always)
                .size([900.0, 700.0], dear_imgui_rs::Condition::Always)
                .build(|| {
                    let paint_frame = p.frame();
                    let clicked = overlay.frame(
                        ui,
                        None,
                        Some(paint_frame.view()),
                        [10.0, 20.0],
                        [765.0, 503.0],
                    );
                    assert!(clicked.is_none(), "no click this frame");
                });
        }
        ctx.render();
        assert_eq!(overlay.button_labels, vec!["Go bank".to_string()]);
        assert_eq!(
            overlay.button_hits.len(),
            1,
            "real ImGui button was laid out"
        );
        let hit = overlay.button_hits[0];
        let center = [(hit[0] + hit[2]) / 2.0, (hit[1] + hit[3]) / 2.0];
        prepare_frame(&mut ctx);
        ctx.io_mut().add_mouse_pos_event(center);
        ctx.io_mut()
            .add_mouse_button_event(dear_imgui_rs::MouseButton::Left, false);
        {
            let ui = ctx.frame();
            let _ = ui
                .window("Game")
                .position([0.0, 0.0], dear_imgui_rs::Condition::Always)
                .size([900.0, 700.0], dear_imgui_rs::Condition::Always)
                .build(|| {
                    let paint_frame = p.frame();
                    let _ = overlay.frame(
                        ui,
                        None,
                        Some(paint_frame.view()),
                        [10.0, 20.0],
                        [765.0, 503.0],
                    );
                });
        }
        ctx.render();
        prepare_frame(&mut ctx);
        ctx.io_mut().add_mouse_pos_event(center);
        ctx.io_mut()
            .add_mouse_button_event(dear_imgui_rs::MouseButton::Left, true);
        let mut clicked = None;
        {
            let ui = ctx.frame();
            let _ = ui
                .window("Game")
                .position([0.0, 0.0], dear_imgui_rs::Condition::Always)
                .size([900.0, 700.0], dear_imgui_rs::Condition::Always)
                .build(|| {
                    let paint_frame = p.frame();
                    clicked = overlay.frame(
                        ui,
                        None,
                        Some(paint_frame.view()),
                        [10.0, 20.0],
                        [765.0, 503.0],
                    );
                });
        }
        ctx.render();
        assert_eq!(
            clicked,
            Some((PaintFrameHit::Button(0), p.generation())),
            "clicking the button dispatches its index with the rendered generation"
        );
        assert!(!overlay.collapsed, "button click is not collapse");
        paint_click_frame(&mut ctx, &mut overlay, &p, Some(center), false);
        paint_click_frame(&mut ctx, &mut overlay, &p, Some(TOGGLE_CENTER), false);
        paint_click_frame(&mut ctx, &mut overlay, &p, Some(TOGGLE_CENTER), true);
        assert!(overlay.collapsed, "toggle click still collapses");
        assert!(
            overlay.button_labels.is_empty(),
            "collapsed hides paint widgets"
        );
        paint_click_frame(&mut ctx, &mut overlay, &p, Some(TOGGLE_CENTER), false);
        paint_click_frame(&mut ctx, &mut overlay, &p, Some(TOGGLE_CENTER), true);
        assert!(!overlay.collapsed);
        assert_eq!(overlay.button_labels, vec!["Go bank".to_string()]);
    }

    #[cfg(feature = "js")]
    fn canvas_banner() -> TestPaint {
        paint(None, &[]).with_canvas_banner()
    }

    #[cfg(feature = "js")]
    #[test]
    fn canvas_dirty_rect_maps_native_and_half_blit() {
        let _guard = crate::test_support::imgui_context_guard();
        let mut ctx = crate::test_support::imgui_context();
        let mut overlay = PaintOverlay::new();
        let p = canvas_banner();
        prepare_frame(&mut ctx);
        {
            let ui = ctx.frame();
            let paint_frame = p.frame();
            overlay.frame(
                ui,
                None,
                Some(paint_frame.view()),
                [10.0, 20.0],
                [765.0, 503.0],
            );
        }
        ctx.render();
        assert_eq!(overlay.canvas_dirty, Some([6, 6, 400, 50]));
        assert_eq!(overlay.canvas_dest, Some([16.0, 26.0, 400.0, 50.0]));
        prepare_frame(&mut ctx);
        {
            let ui = ctx.frame();
            let paint_frame = p.frame();
            overlay.frame(
                ui,
                None,
                Some(paint_frame.view()),
                [0.0, 0.0],
                [382.5, 251.5],
            );
        }
        ctx.render();
        let dest = overlay.canvas_dest.expect("scaled dest");
        assert!((dest[0] - 3.0).abs() < 0.01);
        assert!((dest[1] - 3.0).abs() < 0.01);
        assert!((dest[2] - 200.0).abs() < 0.01);
        assert!((dest[3] - 25.0).abs() < 0.01);
    }

    #[cfg(feature = "js")]
    #[test]
    fn canvas_only_frame_does_not_populate_chatbox() {
        let _guard = crate::test_support::imgui_context_guard();
        let mut ctx = crate::test_support::imgui_context();
        let mut overlay = PaintOverlay::new();
        let p = canvas_banner();
        prepare_frame(&mut ctx);
        {
            let ui = ctx.frame();
            let paint_frame = p.frame();
            overlay.frame(
                ui,
                None,
                Some(paint_frame.view()),
                [10.0, 20.0],
                [765.0, 503.0],
            );
        }
        ctx.render();
        assert!(overlay.lines.is_empty(), "canvas-only has no chatbox lines");
        assert!(overlay.button_labels.is_empty());
        assert!(overlay.canvas_dest.is_some());
    }

    #[cfg(feature = "js")]
    #[test]
    fn structured_plus_canvas_keeps_chatbox_out_of_applet_dest() {
        let _guard = crate::test_support::imgui_context_guard();
        let mut ctx = crate::test_support::imgui_context();
        let mut overlay = PaintOverlay::new();
        let p = paint(Some("BoneBurier"), &["a row"]).with_canvas_banner();
        prepare_frame(&mut ctx);
        {
            let ui = ctx.frame();
            let paint_frame = p.frame();
            overlay.frame(
                ui,
                None,
                Some(paint_frame.view()),
                [10.0, 20.0],
                [765.0, 503.0],
            );
        }
        ctx.render();
        assert_eq!(
            overlay.lines,
            vec!["BoneBurier".to_string(), "a row".to_string()]
        );
        let dest = overlay.canvas_dest.expect("canvas dest");
        let chat = chatbox_rect([10.0, 20.0], [765.0, 503.0]);
        assert!(
            dest[1] + dest[3] < chat[1],
            "canvas banner stays in applet space, not the chatbox ({dest:?} vs {chat:?})"
        );
    }
    #[cfg(not(feature = "js"))]
    #[test]
    fn structured_paint_without_canvas_keeps_chatbox_rows() {
        let _guard = crate::test_support::imgui_context_guard();
        let mut ctx = crate::test_support::imgui_context();
        let mut overlay = PaintOverlay::new();
        let p = paint(Some("BoneBurier"), &["a row"]);
        prepare_frame(&mut ctx);
        {
            let ui = ctx.frame();
            let paint_frame = p.frame();
            overlay.frame(
                ui,
                None,
                Some(paint_frame.view()),
                [10.0, 20.0],
                [765.0, 503.0],
            );
        }
        ctx.render();
        assert_eq!(
            overlay.lines,
            vec!["BoneBurier".to_string(), "a row".to_string()]
        );
        assert!(overlay.canvas_dest.is_none());
    }

    #[cfg(feature = "js")]
    #[test]
    fn measured_fillrect_raster_covers_glyphs() {
        let text = "BoneBurier (external)  buried 0";
        let raster = caption_raster(text);
        let width = raster.text_width();
        assert!(raster.x() <= 6 && raster.y() <= 6);
        assert!(raster.x() + raster.width() as i32 >= 6 + (width + 12.0).round() as i32);
        let mut painted = 0usize;
        for px in raster.rgba().as_chunks::<4>().0 {
            if px[3] > 0 {
                painted += 1;
            }
        }
        assert!(
            painted > 50,
            "glyphs and rect must land in the dirty pixmap"
        );
    }

    #[cfg(feature = "js")]
    struct RecordingGpu {
        device: wgpu::Device,
        queue: wgpu::Queue,
        registered: Vec<u64>,
        unregistered: Vec<u64>,
    }

    #[cfg(feature = "js")]
    impl RecordingGpu {
        fn new(device: wgpu::Device, queue: wgpu::Queue) -> Self {
            Self {
                device,
                queue,
                registered: Vec::new(),
                unregistered: Vec::new(),
            }
        }
    }

    #[cfg(feature = "js")]
    impl crate::game_view::FrameGpu for RecordingGpu {
        fn device(&self) -> &wgpu::Device {
            &self.device
        }
        fn queue(&self) -> &wgpu::Queue {
            &self.queue
        }
        fn register_texture(
            &mut self,
            _texture: &wgpu::Texture,
            _view: &wgpu::TextureView,
        ) -> dear_imgui_rs::TextureId {
            let id = self.registered.len() as u64 + 1;
            self.registered.push(id);
            dear_imgui_rs::TextureId::new(id)
        }
        fn unregister_texture(&mut self, tex_id: dear_imgui_rs::TextureId) {
            self.unregistered.push(tex_id.id());
        }
    }

    #[cfg(feature = "js")]
    fn gpu_frame(
        ctx: &mut dear_imgui_rs::Context,
        overlay: &mut PaintOverlay,
        gpu: &mut RecordingGpu,
        p: &TestPaint,
        min: [f32; 2],
        size: [f32; 2],
    ) {
        prepare_frame(ctx);
        {
            let ui = ctx.frame();
            let paint_frame = p.frame();
            overlay.frame(ui, Some(gpu), Some(paint_frame.view()), min, size);
        }
        ctx.render();
    }

    #[cfg(feature = "js")]
    #[test]
    fn canvas_gpu_cache_skips_unchanged_and_releases() {
        let Some((device, queue)) = headless_gpu() else {
            return;
        };
        let _guard = crate::test_support::imgui_context_guard();
        let mut ctx = crate::test_support::imgui_context();
        let mut overlay = PaintOverlay::new();
        let mut gpu = RecordingGpu::new(device, queue);
        let p = canvas_banner();
        gpu_frame(
            &mut ctx,
            &mut overlay,
            &mut gpu,
            &p,
            [10.0, 20.0],
            [765.0, 503.0],
        );
        assert_eq!(overlay.rasterize_calls(), 1);
        assert_eq!(overlay.upload_calls(), 1);
        assert_eq!(gpu.registered.len(), 1);
        assert!(overlay.canvas_gpu_alive());
        gpu_frame(
            &mut ctx,
            &mut overlay,
            &mut gpu,
            &p,
            [10.0, 20.0],
            [765.0, 503.0],
        );
        assert_eq!(
            overlay.rasterize_calls(),
            1,
            "identical ops must not reraster"
        );
        assert_eq!(overlay.upload_calls(), 1, "identical ops must not reupload");
        assert!(gpu.unregistered.is_empty());
        gpu_frame(
            &mut ctx,
            &mut overlay,
            &mut gpu,
            &p,
            [0.0, 0.0],
            [382.5, 251.5],
        );
        assert_eq!(
            overlay.rasterize_calls(),
            1,
            "scale change remaps dest without reraster"
        );
        let dest = overlay.canvas_dest.expect("scaled dest");
        assert!((dest[2] - 200.0).abs() < 0.01);
        overlay.release_canvas(&mut gpu);
        assert!(!overlay.canvas_gpu_alive());
        assert_eq!(gpu.unregistered.len(), 1);
        gpu_frame(
            &mut ctx,
            &mut overlay,
            &mut gpu,
            &p,
            [10.0, 20.0],
            [765.0, 503.0],
        );
        assert_eq!(overlay.rasterize_calls(), 2);
        let empty = paint(None, &[]);
        gpu_frame(
            &mut ctx,
            &mut overlay,
            &mut gpu,
            &empty,
            [10.0, 20.0],
            [765.0, 503.0],
        );
        assert!(!overlay.canvas_gpu_alive());
        assert_eq!(gpu.unregistered.len(), 2);
    }

    #[test]
    fn paint_uniform_scale_is_aspect_min_not_axis_stretch() {
        assert!((paint_uniform_scale([765.0, 503.0]) - 1.0).abs() < 0.001);
        assert!((paint_uniform_scale([382.5, 251.5]) - 0.5).abs() < 0.001);
        // Non-matching rect: do not stretch text on the long axis.
        assert!((paint_uniform_scale([765.0, 251.5]) - 0.5).abs() < 0.001);
        assert!((paint_uniform_scale([382.5, 503.0]) - 0.5).abs() < 0.001);
    }

    #[test]
    fn structured_paint_hit_targets_scale_with_grid_cell() {
        let _guard = crate::test_support::imgui_context_guard();
        let mut ctx = crate::test_support::imgui_context();
        let mut overlay = PaintOverlay::new();
        let p = paint_with_button(Some("NatureCrafter"), &["status"], "gobank", "Go bank");
        prepare_frame(&mut ctx);
        {
            let ui = ctx.frame();
            let _ = ui
                .window("Game")
                .position([0.0, 0.0], dear_imgui_rs::Condition::Always)
                .size([900.0, 700.0], dear_imgui_rs::Condition::Always)
                .build(|| {
                    let paint_frame = p.frame();
                    overlay.frame(
                        ui,
                        None,
                        Some(paint_frame.view()),
                        [10.0, 20.0],
                        [765.0, 503.0],
                    );
                });
        }
        ctx.render();
        let native_hit = overlay.button_hits[0];
        let native_chat = chatbox_rect([10.0, 20.0], [765.0, 503.0]);
        assert!(
            (native_hit[0] - (native_chat[0] + 8.0)).abs() < 0.5,
            "native inset anchors the hit left edge"
        );
        assert!(
            (native_hit[3] - native_hit[1] - 16.0).abs() < 0.5,
            "the compact button is 16 px high at native scale"
        );
        assert!(
            native_hit[2] < native_chat[0] + native_chat[2] - 100.0,
            "the compact button does not span the body width"
        );

        prepare_frame(&mut ctx);
        {
            let ui = ctx.frame();
            let _ = ui
                .window("Game")
                .position([0.0, 0.0], dear_imgui_rs::Condition::Always)
                .size([900.0, 700.0], dear_imgui_rs::Condition::Always)
                .build(|| {
                    let paint_frame = p.frame();
                    overlay.frame(
                        ui,
                        None,
                        Some(paint_frame.view()),
                        [0.0, 0.0],
                        [382.5, 251.5],
                    );
                });
        }
        ctx.render();
        let half_hit = overlay.button_hits[0];
        let half_chat = chatbox_rect([0.0, 0.0], [382.5, 251.5]);
        let s = paint_uniform_scale([382.5, 251.5]);
        assert!((s - 0.5).abs() < 0.001);
        assert!(
            (half_hit[0] - (half_chat[0] + 8.0 * s)).abs() < 0.5,
            "half-scale inset left ({half_hit:?} vs chat {half_chat:?})"
        );
        let native_w = native_hit[2] - native_hit[0];
        let half_w = half_hit[2] - half_hit[0];
        assert!(
            (half_w - native_w * s).abs() < 1.0,
            "button hit width scales with the cell ({half_w} vs {native_w}*{s})"
        );
        let native_h = native_hit[3] - native_hit[1];
        let half_h = half_hit[3] - half_hit[1];
        assert!(
            (half_h - native_h * s).abs() < 1.0,
            "button hit height scales with the cell ({half_h} vs {native_h}*{s})"
        );
        // Hit stays inside the scaled chatbox.
        assert!(half_hit[0] >= half_chat[0] - 0.5);
        assert!(half_hit[2] <= half_chat[0] + half_chat[2] + 0.5);
        assert!(half_hit[1] >= half_chat[1] - 0.5);
        assert!(half_hit[3] <= half_chat[1] + half_chat[3] + 0.5);
    }

    /// Button labels must match the body text size at every display
    /// scale. The body rows take a physical size on the draw list. A
    /// pushed font size takes a logical size because Dear ImGui
    /// multiplies it by FontScaleDpi. This test reads the glyph
    /// triangles that the frame emitted and compares their heights.
    #[test]
    fn paint_button_labels_match_body_text_at_display_scale_2() {
        let _guard = crate::test_support::imgui_context_guard();
        for scale in [1.0_f32, 1.5, 2.0] {
            let mut ctx = crate::test_support::imgui_context();
            crate::app::apply_ui_scale(ctx.style_mut(), scale);
            let mut overlay = PaintOverlay::new();
            let p = paint_with_button(Some("H"), &["HHH"], "go", "HHH");
            let display = [900.0 * scale, 700.0 * scale];
            let size = [765.0 * scale, 503.0 * scale];
            ctx.prepare_frame(
                FramePrepareOptions::new(display, 1.0 / 60.0).renderer_has_textures(),
            );
            {
                let ui = ctx.frame();
                let _ = ui
                    .window("Game")
                    .position([0.0, 0.0], dear_imgui_rs::Condition::Always)
                    .size(display, dear_imgui_rs::Condition::Always)
                    .build(|| {
                        let paint_frame = p.frame();
                        overlay.frame(ui, None, Some(paint_frame.view()), [0.0, 0.0], size);
                    });
            }
            let button = overlay
                .button_hits
                .first()
                .copied()
                .expect("the button was laid out");
            let chat = chatbox_rect([0.0, 0.0], size);
            let inside = |rect: [f32; 4], cx: f32, cy: f32| {
                cx >= rect[0] && cx <= rect[2] && cy >= rect[1] && cy <= rect[3]
            };
            let chat_rect = [chat[0], chat[1], chat[0] + chat[2], chat[1] + chat[3]];
            let draw_data = ctx.render();
            let (mut body_h, mut label_h) = (0.0_f32, 0.0_f32);
            for list in draw_data.draw_lists() {
                let verts = list.vtx_buffer();
                for tri in list.idx_buffer().as_chunks::<3>().0 {
                    let vs = [
                        &verts[tri[0] as usize],
                        &verts[tri[1] as usize],
                        &verts[tri[2] as usize],
                    ];
                    let u_range = vs.iter().map(|v| v.uv[0]).fold(f32::NEG_INFINITY, f32::max)
                        - vs.iter().map(|v| v.uv[0]).fold(f32::INFINITY, f32::min);
                    let v_range = vs.iter().map(|v| v.uv[1]).fold(f32::NEG_INFINITY, f32::max)
                        - vs.iter().map(|v| v.uv[1]).fold(f32::INFINITY, f32::min);
                    if u_range <= 0.0 || v_range <= 0.0 {
                        continue;
                    }
                    let min_x = vs.iter().map(|v| v.pos[0]).fold(f32::INFINITY, f32::min);
                    let max_x = vs
                        .iter()
                        .map(|v| v.pos[0])
                        .fold(f32::NEG_INFINITY, f32::max);
                    let min_y = vs.iter().map(|v| v.pos[1]).fold(f32::INFINITY, f32::min);
                    let max_y = vs
                        .iter()
                        .map(|v| v.pos[1])
                        .fold(f32::NEG_INFINITY, f32::max);
                    let (cx, cy) = ((min_x + max_x) * 0.5, (min_y + max_y) * 0.5);
                    let h = max_y - min_y;
                    if inside(button, cx, cy) {
                        label_h = label_h.max(h);
                    } else if inside(chat_rect, cx, cy) {
                        body_h = body_h.max(h);
                    }
                }
            }
            assert!(
                body_h > 5.0 * scale,
                "body glyphs appear in the draw list at scale {scale}"
            );
            assert!(
                (label_h - body_h).abs() <= 1.5,
                "at scale {scale} the label glyph height {label_h} differs from the body glyph height {body_h}"
            );
        }
    }

    fn jive_strip_paint(status: &str) -> TestPaint {
        TestPaint::new(Some("JiveCrafting"), &["Overview row"])
            .with_generation(3)
            .with_strip(
                "k",
                &["Statistics", "Options"],
                "Statistics",
                Some(status),
                Some("JiveCrafting"),
            )
    }

    fn label_rects_overlap(a: [f32; 4], b: [f32; 4]) -> bool {
        a[0] < b[2] && b[0] < a[2] && a[1] < b[3] && b[1] < a[3]
    }

    fn frame_strip_fixture(
        ctx: &mut dear_imgui_rs::Context,
        overlay: &mut PaintOverlay,
        p: &TestPaint,
        min: [f32; 2],
        size: [f32; 2],
    ) {
        prepare_frame(ctx);
        {
            let ui = ctx.frame();
            let _ = ui
                .window("Game")
                .position([0.0, 0.0], dear_imgui_rs::Condition::Always)
                .size([900.0, 700.0], dear_imgui_rs::Condition::Always)
                .build(|| {
                    let paint_frame = p.frame();
                    overlay.frame(ui, None, Some(paint_frame.view()), min, size);
                });
        }
        ctx.render();
    }

    #[test]
    fn strip_status_brand_label_rects_disjoint_native_and_half_grid() {
        let _guard = crate::test_support::imgui_context_guard();
        let mut ctx = crate::test_support::imgui_context();
        for status in ["ok", "banking supplies run"] {
            for (min, size) in [([10.0, 20.0], [765.0, 503.0]), ([0.0, 0.0], [382.5, 251.5])] {
                let mut overlay = PaintOverlay::new();
                let p = jive_strip_paint(status);
                frame_strip_fixture(&mut ctx, &mut overlay, &p, min, size);
                let status_rect = overlay.strip_status_label_rect.expect("status label rect");
                let brand_rect = overlay.strip_brand_label_rect.expect("brand label rect");
                assert!(
                    !label_rects_overlap(status_rect, brand_rect),
                    "status {status:?} overlaps brand at scale {size:?}: {status_rect:?} vs {brand_rect:?}"
                );
                let chat = chatbox_rect(min, size);
                let toggle_left = chat[0] + chat[2]
                    - frontend_core::scripts::paint_style::TOGGLE_W * paint_uniform_scale(size);
                assert!(
                    brand_rect[2] <= toggle_left,
                    "brand overlaps collapse control at {size:?}: {brand_rect:?}, toggle starts {toggle_left}"
                );
                for (_kind, hit) in &overlay.chrome_hits {
                    assert!(
                        hit[2] <= brand_rect[0] + 0.5,
                        "tab hit must stay left of brand: {hit:?} brand_x={}",
                        brand_rect[0]
                    );
                }
            }
        }
    }

    #[test]
    fn strip_chrome_click_maps_scaled_hit_to_paint_select() {
        let _guard = crate::test_support::imgui_context_guard();
        let mut ctx = crate::test_support::imgui_context();
        let mut overlay = PaintOverlay::new();
        let p = jive_strip_paint("ok");
        let half = [382.5_f32, 251.5];
        prepare_frame(&mut ctx);
        {
            let ui = ctx.frame();
            let _ = ui
                .window("Game")
                .position([0.0, 0.0], dear_imgui_rs::Condition::Always)
                .size([900.0, 700.0], dear_imgui_rs::Condition::Always)
                .build(|| {
                    let paint_frame = p.frame();
                    overlay.frame(ui, None, Some(paint_frame.view()), [0.0, 0.0], half);
                });
        }
        ctx.render();
        assert!(
            overlay
                .chrome_hits
                .iter()
                .any(|(kind, _)| *kind == ChromeHit::Strip(1)),
            "strip advertises both page targets"
        );
        let hit = overlay
            .chrome_hits
            .iter()
            .find(|(kind, _)| *kind == ChromeHit::Strip(1))
            .map(|(_, r)| *r)
            .expect("Options hit rect");
        let center = [(hit[0] + hit[2]) / 2.0, (hit[1] + hit[3]) / 2.0];
        prepare_frame(&mut ctx);
        ctx.io_mut().add_mouse_pos_event(center);
        ctx.io_mut()
            .add_mouse_button_event(dear_imgui_rs::MouseButton::Left, true);
        let mut clicked = None;
        {
            let ui = ctx.frame();
            let _ = ui
                .window("Game")
                .position([0.0, 0.0], dear_imgui_rs::Condition::Always)
                .size([900.0, 700.0], dear_imgui_rs::Condition::Always)
                .build(|| {
                    let paint_frame = p.frame();
                    clicked = overlay.frame(ui, None, Some(paint_frame.view()), [0.0, 0.0], half);
                });
        }
        ctx.render();
        assert_eq!(
            clicked,
            Some((
                PaintFrameHit::Select {
                    key: "strip:k".into(),
                    name: "Options".into(),
                },
                p.generation()
            ))
        );
    }

    #[test]
    fn collapse_hit_strip_scales_with_grid_cell() {
        let _guard = crate::test_support::imgui_context_guard();
        let mut ctx = crate::test_support::imgui_context();
        let mut overlay = PaintOverlay::new();
        let p = paint(Some("BoneBurier"), &["a row"]);
        let half = [382.5_f32, 251.5];
        let chat = chatbox_rect([0.0, 0.0], half);
        let s = paint_uniform_scale(half);
        // Toggle square centre at half scale: the square is 20 applet
        // px at the title bar's right end.
        prepare_frame(&mut ctx);
        let title = [chat[0] + chat[2] - 10.0 * s, chat[1] + 10.0 * s];
        ctx.io_mut().add_mouse_pos_event(title);
        ctx.io_mut()
            .add_mouse_button_event(dear_imgui_rs::MouseButton::Left, false);
        {
            let ui = ctx.frame();
            let _ = ui
                .window("Game")
                .position([0.0, 0.0], dear_imgui_rs::Condition::Always)
                .size([900.0, 700.0], dear_imgui_rs::Condition::Always)
                .build(|| {
                    let paint_frame = p.frame();
                    overlay.frame(ui, None, Some(paint_frame.view()), [0.0, 0.0], half);
                });
        }
        ctx.render();
        assert!(!overlay.collapsed);
        prepare_frame(&mut ctx);
        ctx.io_mut().add_mouse_pos_event(title);
        ctx.io_mut()
            .add_mouse_button_event(dear_imgui_rs::MouseButton::Left, true);
        {
            let ui = ctx.frame();
            let _ = ui
                .window("Game")
                .position([0.0, 0.0], dear_imgui_rs::Condition::Always)
                .size([900.0, 700.0], dear_imgui_rs::Condition::Always)
                .build(|| {
                    let paint_frame = p.frame();
                    overlay.frame(ui, None, Some(paint_frame.view()), [0.0, 0.0], half);
                });
        }
        ctx.render();
        assert!(
            overlay.collapsed,
            "a click inside the scaled toggle square collapses"
        );
        assert!(overlay.lines.is_empty());
    }

    #[cfg(feature = "js")]
    #[test]
    fn path_op_frame_reuses_texture_on_identical_ops() {
        let Some((device, queue)) = headless_gpu() else {
            return;
        };
        let _guard = crate::test_support::imgui_context_guard();
        let mut ctx = crate::test_support::imgui_context();
        let mut overlay = PaintOverlay::new();
        let mut gpu = RecordingGpu::new(device, queue);
        let p = paint(None, &[]).with_path_stroke();
        gpu_frame(
            &mut ctx,
            &mut overlay,
            &mut gpu,
            &p,
            [10.0, 20.0],
            [765.0, 503.0],
        );
        assert_eq!(overlay.rasterize_calls(), 1);
        assert_eq!(overlay.upload_calls(), 1);
        gpu_frame(
            &mut ctx,
            &mut overlay,
            &mut gpu,
            &p,
            [10.0, 20.0],
            [765.0, 503.0],
        );
        assert_eq!(
            overlay.rasterize_calls(),
            1,
            "identical path ops must not reraster"
        );
        assert_eq!(overlay.upload_calls(), 1);
    }

    // --- Host controls on a helper-generated paint (PAINT-CONTROLS-025) ---

    #[cfg(feature = "js")]
    use frontend_core::scripts::LoadShape;
    use frontend_core::scripts::{PaintFrame, ScriptRunState};

    /// One overlay frame over `frame` with the pointer at `mouse`.
    fn frame_click(
        ctx: &mut dear_imgui_rs::Context,
        overlay: &mut PaintOverlay,
        frame: &PaintFrame,
        mouse: [f32; 2],
        left_down: bool,
    ) -> Option<(PaintFrameHit, u64)> {
        prepare_frame(ctx);
        ctx.io_mut().add_mouse_pos_event(mouse);
        ctx.io_mut()
            .add_mouse_button_event(dear_imgui_rs::MouseButton::Left, left_down);
        let mut hit = None;
        {
            let ui = ctx.frame();
            let _ = ui
                .window("Game")
                .position([0.0, 0.0], dear_imgui_rs::Condition::Always)
                .size([900.0, 700.0], dear_imgui_rs::Condition::Always)
                .build(|| {
                    hit = overlay.frame(ui, None, Some(frame.view()), [10.0, 20.0], [765.0, 503.0]);
                });
        }
        ctx.render();
        hit
    }

    /// Press button `index` of `frame` with the mouse, as an operator does:
    /// lay out, hover, press. Returns the overlay's hit.
    fn press(
        ctx: &mut dear_imgui_rs::Context,
        overlay: &mut PaintOverlay,
        frame: &PaintFrame,
        index: usize,
    ) -> Option<(PaintFrameHit, u64)> {
        let away = [890.0, 690.0];
        let _ = frame_click(ctx, overlay, frame, away, false);
        let hit = overlay.button_hits[index];
        let center = [(hit[0] + hit[2]) / 2.0, (hit[1] + hit[3]) / 2.0];
        let _ = frame_click(ctx, overlay, frame, center, false);
        let pressed = frame_click(ctx, overlay, frame, center, true);
        let _ = frame_click(ctx, overlay, frame, center, false);
        pressed
    }

    #[cfg(feature = "js")]
    /// A panel session over a real play whose `names` each run a real Load
    /// script and publish the paint-controls helper's frame.
    fn helper_session(names: &[&str]) -> crate::session::Session {
        let mut s = crate::session::Session::new();
        s.core.set_spawn_workers(false);
        let mut play = frontend_core::test_support::empty_play(&host_play::PlayOptions {
            host: "127.0.0.1".into(),
            transport: client::Transport::Tcp,
            port: 43594,
            cache_dir: "/tmp".into(),
            lowmem: true,
            mainland: false,
        });
        for (uid, name) in names.iter().enumerate() {
            play.attach_arm(name, host_play::SlotArm::new(uid as i32 + 1, false));
            frontend_core::test_support::tui::start_script_source(
                &play,
                name,
                "export function tick(api) { api._n = (api._n||0)+1 }".into(),
                LoadShape::NativeTick,
            )
            .expect("start");
            let guard = std::time::Instant::now() + std::time::Duration::from_secs(120);
            while frontend_core::test_support::tui::play_script_state(&play, name)
                != ScriptRunState::Running
            {
                assert!(std::time::Instant::now() < guard, "{name} never ran");
                play.pump_script_lifecycle(name);
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            paint(Some("Helper"), &["status"])
                .with_host_controls()
                .with_generation(70 + uid as u64)
                .publish(&play, name);
        }
        s.core.set_play(Some(play));
        s.core.poll();
        s
    }
    #[cfg(not(feature = "js"))]
    /// A panel session over real compiled Gatherer runs with TestPaint
    /// frames that advertise the native host controls.
    fn native_helper_session(
        names: &[&str],
    ) -> (crate::session::Session, crate::test_support::TestDir) {
        use frontend_core::scripts::CompiledCardId;
        use frontend_core::test_support::scripts::Assignment;

        crate::test_support::IsolatedStores::ensure_thread();
        let dir = crate::test_support::TestDir::new("panel-paint-native-controls");
        let mut vault =
            crate::test_support::VaultFixture::create(&dir.join("v.vault"), "test-passphrase-01");
        for (index, name) in names.iter().enumerate() {
            vault.put(crate::test_support::FixtureProfile::new(name, "pw").uid(index as i32 + 1));
        }

        let mut s = crate::session::Session::new();
        frontend_core::test_support::session::install_vault(&mut s.core, vault);
        s.persist_ui = false;
        let mut play = frontend_core::test_support::empty_play(&host_play::PlayOptions {
            host: "127.0.0.1".into(),
            transport: client::Transport::Tcp,
            port: 43594,
            cache_dir: "/tmp".into(),
            lowmem: true,
            mainland: false,
        });
        for (uid, name) in names.iter().enumerate() {
            play.attach_arm(name, host_play::SlotArm::new(uid as i32 + 1, false));
            s.core.fleet_mut().add(name);
        }
        s.core.set_play(Some(play));
        let data =
            api::game_data::for_revision(api::selected::ClientRevision::R289).expect("R289 data");
        s.core
            .play_mut_for_test()
            .expect("native play")
            .bind_script_test_data(data);

        let card = CompiledCardId::lookup("Gatherer").expect("compiled Gatherer card");
        for name in names {
            s.persist_successful_assignment(name, Assignment::compiled(card));
        }
        s.core.flush_writes();
        for name in names {
            s.script_start_profile(name)
                .expect("compiled Gatherer Start");
            let guard = std::time::Instant::now() + std::time::Duration::from_secs(120);
            while s.core.script_state(name) != ScriptRunState::Running {
                assert!(std::time::Instant::now() < guard, "{name} never ran");
                s.core
                    .play_for_test()
                    .expect("native play")
                    .pump_script_lifecycle(name);
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
        }
        for (uid, name) in names.iter().enumerate() {
            paint(Some("Helper"), &["status"])
                .with_host_controls()
                .with_generation(70 + uid as u64)
                .publish(s.core.play_for_test().expect("native play"), name);
        }
        s.core.poll();
        (s, dir)
    }

    /// The overlay draws Pause and Stop from the displayed run, a click
    /// pauses that run through the command seam, the retained frame then
    /// draws Resume and resumes it without another script tick, and Stop
    /// ends it (the paths `app.rs` runs for a hit).
    #[test]
    fn clicked_paint_controls_pause_resume_and_stop_the_displayed_run() {
        let _guard = crate::test_support::imgui_context_guard();
        let mut ctx = crate::test_support::imgui_context();
        let mut overlay = PaintOverlay::new();
        #[cfg(feature = "js")]
        let mut s = helper_session(&["alice"]);
        #[cfg(not(feature = "js"))]
        let (mut s, _dir) = native_helper_session(&["alice"]);
        s.select("alice");

        let frame = s.core.script_paint("alice").expect("paint");
        let hit = press(&mut ctx, &mut overlay, &frame, 0);
        assert_eq!(overlay.button_labels, vec!["Pause", "Stop"]);
        assert_eq!(
            hit.as_ref().map(|(hit, _)| hit),
            Some(&PaintFrameHit::Button(0))
        );
        s.script_paint_activate(&frame, 0);
        assert_eq!(s.core.script_state("alice"), ScriptRunState::Paused);

        let paused = s.core.script_paint("alice").expect("retained paint");
        let hit = press(&mut ctx, &mut overlay, &paused, 0);
        assert_eq!(overlay.button_labels, vec!["Resume", "Stop"]);
        assert_eq!(
            hit.as_ref().map(|(hit, _)| hit),
            Some(&PaintFrameHit::Button(0))
        );
        s.script_paint_activate(&paused, 0);
        assert_eq!(s.core.script_state("alice"), ScriptRunState::Running);

        let running = s.core.script_paint("alice").expect("paint");
        let hit = press(&mut ctx, &mut overlay, &running, 1);
        assert_eq!(
            hit.as_ref().map(|(hit, _)| hit),
            Some(&PaintFrameHit::Button(1))
        );
        s.script_paint_activate(&running, 1);
        let guard = std::time::Instant::now() + std::time::Duration::from_secs(120);
        while s.core.script_state("alice") != ScriptRunState::Idle {
            assert!(std::time::Instant::now() < guard, "Stop never settled");
            s.core
                .play_for_test()
                .unwrap()
                .pump_script_lifecycle("alice");
            s.core.poll();
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    /// A disabled host control draws but reports no click.
    #[test]
    fn a_stopping_run_draws_its_controls_disabled_and_reports_no_click() {
        let _guard = crate::test_support::imgui_context_guard();
        let mut ctx = crate::test_support::imgui_context();
        let mut overlay = PaintOverlay::new();
        let frame = paint(Some("Helper"), &["status"])
            .with_host_controls()
            .frame_for_run("alice", ScriptRunState::Stopping);
        assert_eq!(press(&mut ctx, &mut overlay, &frame, 0), None);
        assert_eq!(press(&mut ctx, &mut overlay, &frame, 1), None);
        assert_eq!(overlay.button_labels, vec!["Pause", "Stop"]);
    }

    /// Pause and Stop stay drawn when text rows fill the dock. The
    /// rows stop early to leave room for the host controls.
    #[test]
    fn host_controls_stay_visible_when_rows_fill_the_dock() {
        let _guard = crate::test_support::imgui_context_guard();
        let mut ctx = crate::test_support::imgui_context();
        let mut overlay = PaintOverlay::new();
        let frame = TestPaint::new(
            Some("JiveCrafting"),
            &[
                "Crafting level 40",
                "XP gained 1200",
                "Items made 36",
                "Runtime 0:05:01",
                "Status walking to the bank",
                "Supplies 12",
                "Trips 3",
                "Level up in 400 XP",
                "Current item leather body",
                "Bank trips left 2",
                "Overflow row one",
                "Overflow row two",
            ],
        )
        .with_strip(
            "k",
            &["Statistics", "Options"],
            "Statistics",
            Some("ok"),
            Some("JiveCrafting"),
        )
        .with_rail("r", &["Overview", "Supplies"], "Overview")
        .with_footer("by Jive")
        .with_host_controls()
        .frame_for_run("alice", ScriptRunState::Running);
        let _ = frame_click(&mut ctx, &mut overlay, &frame, [890.0, 690.0], false);
        assert_eq!(overlay.button_labels, vec!["Pause", "Stop"]);
        assert_eq!(
            overlay.button_hits.len(),
            2,
            "Pause and Stop are both drawn with a full dock. Hits {:?}",
            overlay.button_hits
        );
        let chat = chatbox_rect([10.0, 20.0], [765.0, 503.0]);
        let footer_top = chat[1] + chat[3] - frontend_core::scripts::paint_style::FOOTER_H;
        for hit in &overlay.button_hits {
            assert!(
                hit[0] >= chat[0] && hit[2] <= chat[0] + chat[2] && hit[1] >= chat[1],
                "the control stays inside the dock. Hit {hit:?} dock {chat:?}"
            );
            assert!(
                hit[3] <= footer_top + 0.5,
                "the control stays above the footer row. Hit {hit:?} footer top {footer_top}"
            );
        }
        assert!(
            (overlay.button_hits[1][1] - overlay.button_hits[0][1]).abs() < 0.5
                && overlay.button_hits[1][0] > overlay.button_hits[0][2],
            "Stop sits beside Pause on the same row"
        );
        let hit = press(&mut ctx, &mut overlay, &frame, 1);
        assert_eq!(
            hit.as_ref().map(|(hit, _)| hit),
            Some(&PaintFrameHit::Button(1)),
            "the drawn Stop control reports its click"
        );
    }

    /// One frame in the Game panes' input order: the Game Image item,
    /// then the paint overlay, then the capture decision the panes
    /// make (the gate and anchor read at the Image; the frame's left
    /// click reaches the game only while the pointer is outside the
    /// paint panel). Returns the paint hit, whether the click would
    /// be forwarded to the game, the Image's top-left corner, and
    /// whether the pointer is inside the panel.
    fn pane_capture_frame(
        ctx: &mut dear_imgui_rs::Context,
        overlay: &mut PaintOverlay,
        frame: &PaintFrame,
        mouse: [f32; 2],
        left_down: bool,
    ) -> (Option<(PaintFrameHit, u64)>, bool, [f32; 2], bool) {
        prepare_frame(ctx);
        ctx.io_mut().add_mouse_pos_event(mouse);
        ctx.io_mut()
            .add_mouse_button_event(dear_imgui_rs::MouseButton::Left, left_down);
        let mut out = (None, false, [0.0; 2], false);
        {
            let ui = ctx.frame();
            let _ = ui
                .window("Game")
                .position([0.0, 0.0], dear_imgui_rs::Condition::Always)
                .size([900.0, 700.0], dear_imgui_rs::Condition::Always)
                .build(|| {
                    ui.image(dear_imgui_rs::TextureId::new(0), [765.0, 503.0]);
                    let min = ui.item_rect_min();
                    let image_hovered = ui.is_item_hovered();
                    let hit = overlay.frame(ui, None, Some(frame.view()), min, [765.0, 503.0]);
                    let inside = overlay.pointer_inside_panel();
                    let forwarded = image_hovered
                        && ui.is_mouse_clicked(dear_imgui_rs::MouseButton::Left)
                        && !inside;
                    out = (hit, forwarded, min, inside);
                });
        }
        ctx.render();
        out
    }

    /// Hover then press `point` in the pane order, as an operator's
    /// click arrives. Returns the press frame's pane decision.
    fn pane_press(
        ctx: &mut dear_imgui_rs::Context,
        overlay: &mut PaintOverlay,
        frame: &PaintFrame,
        point: [f32; 2],
    ) -> (Option<(PaintFrameHit, u64)>, bool, [f32; 2], bool) {
        let _ = pane_capture_frame(ctx, overlay, frame, point, false);
        let pressed = pane_capture_frame(ctx, overlay, frame, point, true);
        let _ = pane_capture_frame(ctx, overlay, frame, point, false);
        pressed
    }

    /// M1: with input capture on, a press on Pause, Stop, a strip tab
    /// or the toggle — anywhere inside the paint panel — is the
    /// paint's alone. The same press outside the panel still reaches
    /// the game.
    #[test]
    fn a_press_inside_the_paint_never_clicks_the_game() {
        let _guard = crate::test_support::imgui_context_guard();
        let mut ctx = crate::test_support::imgui_context();
        let mut overlay = PaintOverlay::new();
        let frame = paint(Some("Helper"), &["status"])
            .with_strip(
                "k",
                &["Statistics", "Options"],
                "Statistics",
                Some("ok"),
                Some("Helper"),
            )
            .with_host_controls()
            .frame_for_run("alice", ScriptRunState::Running);
        let away = [890.0, 690.0];
        let _ = pane_capture_frame(&mut ctx, &mut overlay, &frame, away, false);
        let (_, _, min, _) = pane_capture_frame(&mut ctx, &mut overlay, &frame, away, false);
        let buttons = overlay.button_hits.clone();
        assert_eq!(buttons.len(), 2, "Pause and Stop are drawn");
        let centre = |rect: [f32; 4]| [(rect[0] + rect[2]) / 2.0, (rect[1] + rect[3]) / 2.0];

        // A press on the game, outside the panel, is forwarded.
        let outside = [min[0] + 300.0, min[1] + 100.0];
        let (hit, forwarded, _, inside) = pane_press(&mut ctx, &mut overlay, &frame, outside);
        assert_eq!(hit, None, "no paint control is pressed outside");
        assert!(!inside, "the pointer is outside the panel");
        assert!(forwarded, "a press outside the paint reaches the game");

        // Pause and Stop belong to the paint alone.
        for (index, rect) in buttons.iter().enumerate() {
            let (hit, forwarded, _, inside) =
                pane_press(&mut ctx, &mut overlay, &frame, centre(*rect));
            assert_eq!(
                hit.as_ref().map(|(hit, _)| hit),
                Some(&PaintFrameHit::Button(index)),
                "the press hits the paint control"
            );
            assert!(inside, "the control is inside the panel");
            assert!(!forwarded, "the press never also clicks the game");
        }

        // A strip tab belongs to the paint alone.
        let tab = overlay
            .chrome_hits
            .iter()
            .find(|(kind, _)| *kind == ChromeHit::Strip(1))
            .map(|(_, rect)| *rect)
            .expect("Options tab rect");
        let (hit, forwarded, _, inside) = pane_press(&mut ctx, &mut overlay, &frame, centre(tab));
        assert_eq!(
            hit.as_ref().map(|(hit, _)| hit),
            Some(&PaintFrameHit::Select {
                key: "strip:k".into(),
                name: "Options".into(),
            }),
            "the press selects the tab"
        );
        assert!(inside && !forwarded, "the tab press stays in the paint");

        // The panel body itself, away from every control, also holds
        // the press back from the game under it.
        let body = [min[0] + 400.0, min[1] + 430.0];
        let (hit, forwarded, _, inside) = pane_press(&mut ctx, &mut overlay, &frame, body);
        assert_eq!(hit, None, "no control is pressed on the panel body");
        assert!(
            inside && !forwarded,
            "the panel body press stays in the paint"
        );

        // The toggle square collapses the paint and forwards nothing.
        let toggle = [min[0] + 504.0, min[1] + 355.0];
        let (_, forwarded, _, inside) = pane_press(&mut ctx, &mut overlay, &frame, toggle);
        assert!(overlay.collapsed, "the toggle press collapses the paint");
        assert!(inside && !forwarded, "the toggle press stays in the paint");
    }

    use client::client::GameShell;
    use frontend_core::surface::{InputEv, InputSender, SlotInput};
    use frontend_core::{GameInputEvent, MapDesk};

    use crate::input_capture::{
        discard_unconsumed_native_capture, stream_capture, MouseButtons, MouseFrame, MouseTarget,
        PaintMouseGate,
    };

    /// This fixture uses the input order shared by both Game panes.
    /// It draws the Game Image, then the paint overlay. It stages
    /// image movement, then routes button edges through the shared
    /// gate. The staged events pass through `InputSender` and
    /// `SlotInput` into `GameShell`. This matches the focused slot's
    /// consumer path.
    ///
    /// Keyboard ownership runs at the frame owner, outside both
    /// panes, and this fixture does not model it. A left press can
    /// change keyboard availability and release a drag through that
    /// separate path. The gate tests cover the left-button rule.
    ///
    /// Scale 1.0 is the single-bot pane's native blit. Scale 0.5 is a
    /// focused grid cell's fitted blit.
    struct PaneCapture {
        ctx: dear_imgui_rs::Context,
        overlay: PaintOverlay,
        frame: PaintFrame,
        map: MapDesk,
        gate: PaintMouseGate,
        input: std::sync::Arc<SlotInput>,
        tx: InputSender,
        shell: GameShell,
        scale: f32,
        min: [f32; 2],
        last_image_hovered: bool,
        show_paint: bool,
    }

    impl PaneCapture {
        fn new(scale: f32) -> Self {
            discard_unconsumed_native_capture();
            let input = SlotInput::new();
            input.set_enabled(true);
            let tx = InputSender::connect(&input);
            let mut map = MapDesk::default();
            map.set_input_sender(Some(tx.duplicate()));
            let frame = paint(Some("Helper"), &["status"])
                .with_strip("s", &["One", "Two"], "One", Some("ok"), Some("Helper"))
                .with_rail("r", &["First", "Second"], "First")
                .with_tab_band("t", &["Alpha", "Beta"], "Alpha")
                .with_host_controls()
                .frame_for_run("alice", ScriptRunState::Running);
            let mut capture = Self {
                ctx: crate::test_support::imgui_context(),
                overlay: PaintOverlay::new(),
                frame,
                map,
                gate: PaintMouseGate::default(),
                input,
                tx,
                shell: GameShell::new(),
                scale,
                min: [0.0; 2],
                last_image_hovered: false,
                show_paint: true,
            };
            // The product Game window is docked and cannot undock.
            // Do not let this GPU-less standalone window steal a
            // Game Image press as an ImGui window-move gesture.
            capture
                .ctx
                .io_mut()
                .set_config_windows_move_from_title_bar_only(true);
            capture
                .ctx
                .io_mut()
                .set_config_flags(dear_imgui_rs::ConfigFlags::DOCKING_ENABLE);
            capture.step([890.0, 690.0], false, false);
            capture.step([890.0, 690.0], false, false);
            capture
        }

        /// An applet-space point on the scaled Game Image.
        fn point(&self, x: f32, y: f32) -> [f32; 2] {
            [self.min[0] + x * self.scale, self.min[1] + y * self.scale]
        }

        /// One frame with the pointer at `mouse` and the given
        /// physical button states. Returns the events staged for the
        /// game this frame.
        fn step(&mut self, mouse: [f32; 2], left: bool, right: bool) -> Vec<InputEv> {
            self.ctx.prepare_frame(
                FramePrepareOptions::new([900.0, 700.0], 1.0 / 60.0).renderer_has_textures(),
            );
            self.ctx.io_mut().add_mouse_pos_event(mouse);
            self.ctx
                .io_mut()
                .add_mouse_button_event(dear_imgui_rs::MouseButton::Left, left);
            self.ctx
                .io_mut()
                .add_mouse_button_event(dear_imgui_rs::MouseButton::Right, right);
            let mut image_hovered = false;
            self.map.begin_game_input_frame();
            {
                let ui = self.ctx.frame();
                let dock = ui.dockspace_over_main_viewport_with_flags(
                    dear_imgui_rs::Id::from(1u32),
                    dear_imgui_rs::DockNodeFlags::NO_UNDOCKING,
                );
                ui.set_next_window_dock_id(dock);
                let _ = ui
                    .window("Game")
                    .flags(dear_imgui_rs::WindowFlags::NO_MOVE)
                    .position([0.0, 0.0], dear_imgui_rs::Condition::Always)
                    .size([900.0, 700.0], dear_imgui_rs::Condition::Always)
                    .build(|| {
                        let size = [765.0 * self.scale, 503.0 * self.scale];
                        ui.image(dear_imgui_rs::TextureId::new(0), size);
                        self.min = ui.item_rect_min();
                        image_hovered = ui.is_item_hovered();
                        let view = if self.show_paint {
                            Some(self.frame.view())
                        } else {
                            None
                        };
                        let _ = self.overlay.frame(ui, None, view, self.min, size);
                        let inside = self.overlay.pointer_inside_panel();
                        let buttons = MouseButtons {
                            left_down: ui.is_mouse_clicked(dear_imgui_rs::MouseButton::Left),
                            right_down: ui.is_mouse_clicked(dear_imgui_rs::MouseButton::Right),
                            left_up: ui.is_mouse_released(dear_imgui_rs::MouseButton::Left),
                            right_up: ui.is_mouse_released(dear_imgui_rs::MouseButton::Right),
                        };
                        let applet_point = if image_hovered {
                            frontend_core::surface::map_image_to_applet(
                                mouse[0] - self.min[0],
                                mouse[1] - self.min[1],
                                size[0],
                                size[1],
                            )
                        } else {
                            None
                        };
                        if image_hovered {
                            stream_capture(
                                &mut self.map,
                                mouse[0] - self.min[0],
                                mouse[1] - self.min[1],
                                size[0],
                                size[1],
                                false,
                                false,
                                false,
                                false,
                                &[],
                            );
                        }
                        let sender = if image_hovered
                            && !inside
                            && applet_point.is_some()
                            && (buttons.left_down || buttons.right_down)
                        {
                            self.map.duplicate_input_sender()
                        } else {
                            None
                        };
                        let target = Some(MouseTarget {
                            slot: "alice",
                            input: &self.input,
                        });
                        self.gate.route(
                            &mut self.map,
                            MouseFrame {
                                target,
                                sender,
                                image_hovered,
                                pointer_in_paint: inside,
                                applet_point,
                                buttons,
                            },
                            |slot, input| {
                                slot == "alice" && std::sync::Arc::ptr_eq(input, &self.input)
                            },
                        );
                    });
            }
            self.ctx.render();
            self.last_image_hovered = image_hovered;
            let events: Vec<InputEv> = self
                .map
                .staged_game_input_for_test()
                .into_iter()
                .filter_map(|(event, _)| match event {
                    GameInputEvent::Captured(event) => Some(event),
                    GameInputEvent::Wire(_) => None,
                })
                .collect();
            for (event, target) in self.map.staged_game_input_for_test() {
                if let GameInputEvent::Captured(event) = event {
                    assert!(target.unwrap_or(&self.tx).send(event));
                }
            }
            self.input.drain(&mut self.shell);
            events
        }

        /// Click the game once and let the window-focus change
        /// settle, so the measured drag starts from a steady state.
        fn settle(&mut self, outside: [f32; 2]) {
            self.step(outside, false, false);
            self.step(outside, true, false);
            self.step(outside, false, false);
            self.step(outside, false, false);
            self.step(outside, false, false);
            assert_eq!(self.shell.mouse_button, 0, "the settling click ended");
        }
    }

    /// A press that starts on the game, then releases inside the
    /// paint, still sends its release to the game. The game does not
    /// stay held after the physical button goes up.
    #[test]
    fn a_game_press_released_inside_the_paint_releases_the_game() {
        let _guard = crate::test_support::imgui_context_guard();
        for scale in [1.0, 0.5] {
            for (name, left, right, held) in [("left", true, false, 1), ("right", false, true, 2)] {
                let mut capture = PaneCapture::new(scale);
                let outside = capture.point(300.0, 100.0);
                let inside = capture.point(400.0, 430.0);
                capture.settle(outside);
                let events = capture.step(outside, left, right);
                assert!(
                    events
                        .iter()
                        .any(|e| matches!(e, InputEv::Down { button, .. } if *button == held)),
                    "scale {scale} {name}: the press reaches the game. Events {events:?}"
                );
                assert_eq!(
                    capture.shell.mouse_button, held,
                    "scale {scale} {name}: the game holds the button"
                );
                let held_events = capture.step(inside, left, right);
                assert_eq!(
                    capture.shell.mouse_button, held,
                    "scale {scale} {name}: the drag over the paint stays held. Events {held_events:?}"
                );
                let events = capture.step(inside, false, false);
                assert!(
                    events.iter().any(|e| matches!(e, InputEv::Up)),
                    "scale {scale} {name}: the release inside the paint reaches the game. Events {events:?}"
                );
                assert_eq!(
                    capture.shell.mouse_button, 0,
                    "scale {scale} {name}: the physical release ends the hold"
                );
            }
        }
    }

    /// A game press keeps its release after the pointer leaves the Game Image.
    #[test]
    fn a_game_press_released_outside_the_image_releases_the_game() {
        let _guard = crate::test_support::imgui_context_guard();
        for scale in [1.0, 0.5] {
            for (name, left, right, held) in [("left", true, false, 1), ("right", false, true, 2)] {
                let mut capture = PaneCapture::new(scale);
                let game = capture.point(300.0, 100.0);
                let outside_image = capture.point(800.0, 600.0);
                capture.settle(game);

                let events = capture.step(game, left, right);
                assert!(
                    events.iter().any(
                        |event| matches!(event, InputEv::Down { button, .. } if *button == held)
                    ),
                    "scale {scale} {name}: the press reaches the game. Events {events:?}"
                );
                assert_eq!(capture.shell.mouse_button, held);

                capture.step(outside_image, left, right);
                assert!(
                    !capture.last_image_hovered,
                    "scale {scale} {name}: the held pointer is outside the Game Image"
                );
                let events = capture.step(outside_image, false, false);
                assert!(
                    !capture.last_image_hovered,
                    "scale {scale} {name}: the release is outside the Game Image"
                );
                assert!(
                    events.iter().any(|event| matches!(event, InputEv::Up)),
                    "scale {scale} {name}: the off-image release reaches the game. Events {events:?}"
                );
                assert_eq!(
                    capture.shell.mouse_button, 0,
                    "scale {scale} {name}: the physical release ends the hold"
                );
            }
        }
    }

    /// A press that starts on the paint sends nothing to the game,
    /// and its release over the game sends nothing either. The game
    /// gets no release for a press it never saw.
    #[test]
    fn a_paint_press_released_over_the_game_sends_nothing_to_the_game() {
        let _guard = crate::test_support::imgui_context_guard();
        for scale in [1.0, 0.5] {
            for (name, left, right) in [("left", true, false), ("right", false, true)] {
                let mut capture = PaneCapture::new(scale);
                let outside = capture.point(300.0, 100.0);
                let inside = capture.point(400.0, 430.0);
                capture.settle(outside);
                capture.step(inside, false, false);
                let events = capture.step(inside, left, right);
                assert!(
                    !events.iter().any(|e| matches!(e, InputEv::Down { .. })),
                    "scale {scale} {name}: the paint press sends no Down. Events {events:?}"
                );
                assert_eq!(capture.shell.mouse_button, 0);
                capture.step(outside, left, right);
                let events = capture.step(outside, false, false);
                assert!(
                    !events.iter().any(|e| matches!(e, InputEv::Up)),
                    "scale {scale} {name}: the release over the game sends no Up. Events {events:?}"
                );
                assert_eq!(
                    capture.shell.mouse_button, 0,
                    "scale {scale} {name}: the game never held the button"
                );
            }
        }
    }

    /// Stop clears the paint. A game press that is still held when
    /// the paint goes away keeps its release afterwards.
    #[test]
    fn a_game_press_keeps_its_release_after_the_paint_goes_away() {
        let _guard = crate::test_support::imgui_context_guard();
        for scale in [1.0, 0.5] {
            let mut capture = PaneCapture::new(scale);
            let outside = capture.point(300.0, 100.0);
            let inside = capture.point(400.0, 430.0);
            capture.settle(outside);
            capture.step(outside, false, true);
            assert_eq!(
                capture.shell.mouse_button, 2,
                "scale {scale}: the game holds the right button"
            );
            capture.show_paint = false;
            capture.step(inside, false, true);
            let events = capture.step(inside, false, false);
            assert!(
                events.iter().any(|e| matches!(e, InputEv::Up)),
                "scale {scale}: the release reaches the game after the paint went away. Events {events:?}"
            );
            assert_eq!(capture.shell.mouse_button, 0);
        }
    }

    /// One overlay frame over `frame` at an explicit display and
    /// blit size, with an optional pointer. Returns the hit.
    fn frame_at_scale(
        ctx: &mut dear_imgui_rs::Context,
        overlay: &mut PaintOverlay,
        frame: &PaintFrame,
        display: [f32; 2],
        size: [f32; 2],
        mouse: Option<[f32; 2]>,
        left_down: bool,
    ) -> Option<(PaintFrameHit, u64)> {
        ctx.prepare_frame(FramePrepareOptions::new(display, 1.0 / 60.0).renderer_has_textures());
        if let Some(mouse) = mouse {
            ctx.io_mut().add_mouse_pos_event(mouse);
        }
        ctx.io_mut()
            .add_mouse_button_event(dear_imgui_rs::MouseButton::Left, left_down);
        let mut hit = None;
        {
            let ui = ctx.frame();
            let _ = ui
                .window("Game")
                .position([0.0, 0.0], dear_imgui_rs::Condition::Always)
                .size(display, dear_imgui_rs::Condition::Always)
                .build(|| {
                    hit = overlay.frame(ui, None, Some(frame.view()), [0.0, 0.0], size);
                });
        }
        ctx.render();
        hit
    }

    /// M2: a paint whose tab bands fill the dock (5, 6 and the
    /// isolate's limit of 16) still draws Pause and Stop inside the
    /// chatbox at scales 1.0, 1.5 and 2.0, and Pause reports its
    /// press.
    #[test]
    fn host_controls_stay_visible_with_many_tab_bands_at_each_scale() {
        let _guard = crate::test_support::imgui_context_guard();
        for bands in [5usize, 6, 16] {
            for scale in [1.0_f32, 1.5, 2.0] {
                let mut ctx = crate::test_support::imgui_context();
                crate::app::apply_ui_scale(ctx.style_mut(), scale);
                let mut overlay = PaintOverlay::new();
                let mut fixture = paint(Some("T"), &["row one", "row two"]);
                for band in 0..bands {
                    fixture = fixture.with_tab_band(&format!("b{band}"), &["A", "B"], "A");
                }
                let frame = fixture
                    .with_host_controls()
                    .frame_for_run("alice", ScriptRunState::Running);
                let display = [900.0 * scale, 700.0 * scale];
                let size = [765.0 * scale, 503.0 * scale];
                let _ = frame_at_scale(&mut ctx, &mut overlay, &frame, display, size, None, false);
                assert_eq!(
                    overlay.button_hits.len(),
                    2,
                    "bands {bands} scale {scale}: Pause and Stop are both drawn"
                );
                let chat = chatbox_rect([0.0, 0.0], size);
                let top = overlay.button_hits[0][1];
                for rect in &overlay.button_hits {
                    assert!(
                        rect[0] >= chat[0] - 0.5
                            && rect[2] <= chat[0] + chat[2] + 0.5
                            && rect[1] >= chat[1] - 0.5
                            && rect[3] <= chat[1] + chat[3] + 0.5,
                        "bands {bands} scale {scale}: the control stays in the chatbox: {rect:?} chat {chat:?}"
                    );
                    assert!(
                        (rect[3] - rect[1] - 16.0 * scale).abs() < 0.5,
                        "bands {bands} scale {scale}: the control is 16 px high: {rect:?}"
                    );
                    assert_eq!(rect[1], top, "bands {bands} scale {scale}: one row");
                }
                let pause = overlay.button_hits[0];
                let centre = [(pause[0] + pause[2]) / 2.0, (pause[1] + pause[3]) / 2.0];
                let _ = frame_at_scale(
                    &mut ctx,
                    &mut overlay,
                    &frame,
                    display,
                    size,
                    Some(centre),
                    false,
                );
                let hit = frame_at_scale(
                    &mut ctx,
                    &mut overlay,
                    &frame,
                    display,
                    size,
                    Some(centre),
                    true,
                );
                assert_eq!(
                    hit.as_ref().map(|(hit, _)| hit),
                    Some(&PaintFrameHit::Button(0)),
                    "bands {bands} scale {scale}: the drawn Pause reports its press"
                );
                let _ = frame_at_scale(
                    &mut ctx,
                    &mut overlay,
                    &frame,
                    display,
                    size,
                    Some(centre),
                    false,
                );
            }
        }
    }

    /// One overlay frame over `frame` at the native blit in the corner.
    fn frame_native(
        ctx: &mut dear_imgui_rs::Context,
        overlay: &mut PaintOverlay,
        frame: &PaintFrame,
    ) {
        prepare_frame(ctx);
        {
            let ui = ctx.frame();
            let _ = ui
                .window("Game")
                .position([0.0, 0.0], dear_imgui_rs::Condition::Always)
                .size([900.0, 700.0], dear_imgui_rs::Condition::Always)
                .build(|| {
                    overlay.frame(ui, None, Some(frame.view()), [0.0, 0.0], [765.0, 503.0]);
                });
        }
        ctx.render();
    }

    /// Design test 2: Pause and Stop share one compact row at scale 1.
    /// The exact width formula (label cells x cell width + 18) is
    /// pinned in the frontend-core layout test with a fixed cell
    /// width. This test pins what the renderer draws.
    #[test]
    fn pause_and_stop_share_one_compact_row() {
        let _guard = crate::test_support::imgui_context_guard();
        let mut ctx = crate::test_support::imgui_context();
        let mut overlay = PaintOverlay::new();
        let frame = paint(Some("Helper"), &["status"])
            .with_host_controls()
            .frame_for_run("alice", ScriptRunState::Running);
        frame_native(&mut ctx, &mut overlay, &frame);
        assert_eq!(overlay.button_labels, vec!["Pause", "Stop"]);
        assert_eq!(overlay.button_hits.len(), 2);
        let pause = overlay.button_hits[0];
        let stop = overlay.button_hits[1];
        assert!(
            (pause[3] - pause[1] - 16.0).abs() < 0.5 && (stop[3] - stop[1] - 16.0).abs() < 0.5,
            "both controls are 16 px high: {pause:?} {stop:?}"
        );
        assert!(
            (pause[1] - stop[1]).abs() < 0.5,
            "both controls share one row top: {pause:?} {stop:?}"
        );
        assert!(
            (pause[0] - 16.0).abs() < 0.5,
            "Pause left = 16 (the dock at 8 plus the 8 px inset): {pause:?}"
        );
        let pause_w = pause[2] - pause[0];
        assert!(
            (48.0..=58.0).contains(&pause_w),
            "Pause width is 5 cells + 18 at the panel face's cell width: {pause_w}"
        );
        assert!(
            (stop[0] - pause[2] - 6.0).abs() < 0.5,
            "Stop left = Pause right + 6: {pause:?} {stop:?}"
        );
    }

    /// The sRGB bytes of the first glyph vertex colour inside `area`,
    /// read back from the frame's draw lists. Filled rects use the
    /// atlas white pixel, so their triangles have no UV range and do
    /// not count as glyphs.
    fn glyph_colours(ctx: &mut dear_imgui_rs::Context, area: [f32; 4]) -> Vec<[i32; 3]> {
        let draw_data = ctx.render();
        let mut colours = Vec::new();
        for list in draw_data.draw_lists() {
            let verts = list.vtx_buffer();
            for tri in list.idx_buffer().as_chunks::<3>().0 {
                let vs = [
                    &verts[tri[0] as usize],
                    &verts[tri[1] as usize],
                    &verts[tri[2] as usize],
                ];
                let u_range = vs.iter().map(|v| v.uv[0]).fold(f32::NEG_INFINITY, f32::max)
                    - vs.iter().map(|v| v.uv[0]).fold(f32::INFINITY, f32::min);
                let v_range = vs.iter().map(|v| v.uv[1]).fold(f32::NEG_INFINITY, f32::max)
                    - vs.iter().map(|v| v.uv[1]).fold(f32::INFINITY, f32::min);
                if u_range <= 0.0 || v_range <= 0.0 {
                    continue;
                }
                let cx = vs.iter().map(|v| v.pos[0]).sum::<f32>() / 3.0;
                let cy = vs.iter().map(|v| v.pos[1]).sum::<f32>() / 3.0;
                if cx >= area[0] && cx < area[2] && cy >= area[1] && cy < area[3] {
                    let col = vs[0].col;
                    colours.push([
                        (col & 0xff) as i32,
                        ((col >> 8) & 0xff) as i32,
                        ((col >> 16) & 0xff) as i32,
                    ]);
                }
            }
        }
        colours
    }

    /// Design test 5: the title text takes the script's accent. A paint
    /// without an accent uses rs2b0t's default (122, 208, 255).
    #[test]
    fn paint_title_uses_the_script_accent() {
        let _guard = crate::test_support::imgui_context_guard();
        for (accent, want) in [(Some("#5be05b"), [91, 224, 91]), (None, [122, 208, 255])] {
            let mut ctx = crate::test_support::imgui_context();
            let mut overlay = PaintOverlay::new();
            let mut p = paint(Some("ChickenKiller"), &["Runtime 0:01"]);
            if let Some(accent) = accent {
                p = p.with_accent(accent);
            }
            prepare_frame(&mut ctx);
            {
                let ui = ctx.frame();
                let _ = ui
                    .window("Game")
                    .position([0.0, 0.0], dear_imgui_rs::Condition::Always)
                    .size([900.0, 700.0], dear_imgui_rs::Condition::Always)
                    .build(|| {
                        let paint_frame = p.frame();
                        overlay.frame(
                            ui,
                            None,
                            Some(paint_frame.view()),
                            [0.0, 0.0],
                            [765.0, 503.0],
                        );
                    });
            }
            // The title bar at the native blit, left of the toggle.
            let chat = chatbox_rect([0.0, 0.0], [765.0, 503.0]);
            let area = [chat[0], chat[1], chat[0] + chat[2] - 20.0, chat[1] + 20.0];
            let colours = glyph_colours(&mut ctx, area);
            assert!(
                !colours.is_empty(),
                "title glyphs appear in the draw list for accent {accent:?}"
            );
            for colour in &colours {
                for ch in 0..3 {
                    assert!(
                        (colour[ch] - want[ch]).abs() <= 2,
                        "accent {accent:?}: title glyph colour {colour:?}, want {want:?}"
                    );
                }
            }
        }
    }

    /// Design test 6: a steady paint frame allocates nothing. The
    /// layout and every test mirror reuse their buffers, and no string
    /// is built per frame.
    #[test]
    fn steady_paint_frame_allocates_nothing() {
        let _guard = crate::test_support::imgui_context_guard();
        let mut ctx = crate::test_support::imgui_context();
        let mut overlay = PaintOverlay::new();
        let frame = TestPaint::new(
            Some("JiveCrafting"),
            &["Crafting level 40", "XP gained 1200", "Runtime 0:05:01"],
        )
        .with_accent("#e05be0")
        .with_strip(
            "k",
            &["Statistics", "Options"],
            "Statistics",
            Some("walking to the bank"),
            Some("JiveCrafting"),
        )
        .with_rail("r", &["Overview", "Supplies"], "Overview")
        .with_footer("by Jive")
        .with_host_controls()
        .frame_for_run("alice", ScriptRunState::Running);
        // Warm frames grow the reused buffers once.
        frame_native(&mut ctx, &mut overlay, &frame);
        frame_native(&mut ctx, &mut overlay, &frame);
        let mut delta = usize::MAX;
        prepare_frame(&mut ctx);
        {
            let ui = ctx.frame();
            let _ = ui
                .window("Game")
                .position([0.0, 0.0], dear_imgui_rs::Condition::Always)
                .size([900.0, 700.0], dear_imgui_rs::Condition::Always)
                .build(|| {
                    let before = crate::test_alloc::alloc_calls();
                    overlay.frame(ui, None, Some(frame.view()), [0.0, 0.0], [765.0, 503.0]);
                    delta = crate::test_alloc::alloc_calls() - before;
                });
        }
        ctx.render();
        assert_eq!(delta, 0, "allocation calls during a steady paint frame");
    }

    /// A click captured on alice's paint never controls bob once the
    /// focus moved to bob before the click was applied.
    #[test]
    fn a_paint_control_captured_for_one_profile_never_controls_the_new_focus() {
        #[cfg(feature = "js")]
        let mut s = helper_session(&["alice", "bob"]);
        #[cfg(not(feature = "js"))]
        let (mut s, _dir) = native_helper_session(&["alice", "bob"]);
        s.select("alice");
        let alice = s.core.script_paint("alice").expect("paint");
        s.select("bob");
        s.script_paint_activate(&alice, 0);
        s.script_paint_activate(&alice, 1);
        assert_eq!(s.core.script_state("alice"), ScriptRunState::Running);
        assert_eq!(s.core.script_state("bob"), ScriptRunState::Running);
    }
}
