//! Script-paint overlay for the Game Image.
//!
//! Structured Paint is drawn on the Game window draw list over the client's
//! chatbox rect — never a second ImGui window and never pixels on the
//! 765×503 game texture. Canvas ops rasterize to a small cached transparent
//! texture over the applet, using the same native font as measureText.

use dear_imgui_rs::{MouseButton, StyleVar, TextureId, Ui};
use script::canvas::{self, CanvasOp};
use script::shim::ScriptPaint;

use crate::game_view::{FrameGpu, APPLET_H, APPLET_W};
use crate::theme::{ACCENT, BG_DEEP, TEXT, TEXT_DIM};

/// Applet-space chatbox rect `(x, y, w, h)` the client reserves for game
/// chat on the 765×503 stage.
pub const CHATBOX: [f32; 4] = [8.0, 345.0, 506.0, 150.0];

/// Native structured-paint chrome at 1× (applet display = 765×503).
const PAD_X: f32 = 6.0;
const HEADER_Y: f32 = 2.0;
const LINE_STEP: f32 = 14.0;
const ROW_H_FLOOR: f32 = 16.0;

/// Source paint chrome at 1× applet space (see frozen `Paint.ts`).
const TITLE_H_1X: f32 = 20.0;
const TAB_H_1X: f32 = 18.0;
const TAB_INSET_1X: f32 = 3.0;
const TAB_TRAIL_1X: f32 = 2.0;
const FOOTER_LINE_1X: f32 = 16.0;
const RAIL_W_1X: f32 = 72.0;
const STRIP_TAB_PAD_1X: f32 = 14.0;
const STATUS_FG: [f32; 4] = [0.435, 0.878, 0.545, 1.0];

/// One script-paint interaction the overlay detected this frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PaintFrameHit {
    /// One-shot paint button (`paintClick`).
    Button(String),
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

fn text_width(ui: &Ui, font_sz: f32, text: &str) -> f32 {
    ui.current_font().calc_text_size(font_sz, f32::MAX, 0.0, text)[0]
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
    /// The paint's title + rows submitted this frame (empty while
    /// collapsed or when no paint is showing). The GPU-less tests assert
    /// on this mirror of the window text.
    lines: Vec<String>,
    /// Advertised button labels drawn this frame (empty while collapsed).
    button_labels: Vec<String>,
    /// Screen-space button rects `(min, max)` for GPU-less click tests.
    button_hits: Vec<[f32; 4]>,
    /// Chrome hit targets: `(store key, advertised name, rect)`.
    chrome_hits: Vec<(String, String, [f32; 4])>,
    /// Screen-space canvas dest `(x, y, w, h)` this frame, if any.
    canvas_dest: Option<[f32; 4]>,
    /// Applet-space dirty rect this frame, if any.
    canvas_dirty: Option<[i32; 4]>,
    last_ops: Vec<CanvasOp>,
    last_dirty: Option<[i32; 4]>,
    canvas_gpu: Option<CanvasGpu>,
    #[cfg(test)]
    rasterize_calls: u32,
    #[cfg(test)]
    upload_calls: u32,
}

impl PaintOverlay {
    pub fn new() -> Self {
        Self {
            collapsed: false,
            lines: Vec::new(),
            button_labels: Vec::new(),
            button_hits: Vec::new(),
            chrome_hits: Vec::new(),
            canvas_dest: None,
            canvas_dirty: None,
            last_ops: Vec::new(),
            last_dirty: None,
            canvas_gpu: None,
            #[cfg(test)]
            rasterize_calls: 0,
            #[cfg(test)]
            upload_calls: 0,
        }
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

    /// Draw the focused slot's paint over the Game Image. `min`/`size`
    /// are the Image widget's display rect. No-op without a paint. Returns
    /// the advertised button id clicked this frame, if any.
    pub fn frame(
        &mut self,
        ui: &Ui,
        gpu: Option<&mut dyn FrameGpu>,
        paint: Option<&ScriptPaint>,
        min: [f32; 2],
        size: [f32; 2],
    ) -> Option<(PaintFrameHit, u64)> {
        self.lines.clear();
        self.button_labels.clear();
        self.button_hits.clear();
        self.chrome_hits.clear();
        self.canvas_dest = None;
        self.canvas_dirty = None;

        let structured = paint.filter(|p| {
            p.title.is_some()
                || !p.lines.is_empty()
                || !p.buttons.is_empty()
                || p.strip.is_some()
                || p.rail.as_ref().is_some_and(|r| !r.names.is_empty())
                || p.footer.is_some()
                || !p.tabs.is_empty()
        });
        let canvas_ops = paint.map(|p| p.canvas.as_slice()).unwrap_or(&[]);

        if structured.is_none() && canvas_ops.is_empty() {
            self.collapsed = false;
            if let Some(gpu) = gpu {
                self.release_canvas(gpu);
            } else {
                self.last_ops.clear();
            }
            return None;
        }

        if canvas_ops.is_empty() {
            if let Some(gpu) = gpu {
                self.release_canvas(gpu);
            } else {
                self.last_ops.clear();
                self.last_dirty = None;
            }
        } else {
            let ops_unchanged = self.last_ops.as_slice() == canvas_ops;
            let dirty = if ops_unchanged {
                self.last_dirty
            } else {
                canvas::dirty_bounds(canvas_ops).map(|d| {
                    let arr = [d.x, d.y, d.w, d.h];
                    self.last_dirty = Some(arr);
                    arr
                })
            };
            if let Some([dx, dy, dw, dh]) = dirty {
                self.canvas_dirty = Some([dx, dy, dw, dh]);
                let dest = canvas::map_applet_rect(min, size, dx, dy, dw, dh);
                self.canvas_dest = Some(dest);
                if let Some(gpu) = gpu {
                    let reuse_tex = ops_unchanged && self.canvas_gpu.is_some();
                    if !reuse_tex {
                        if let Some(raster) = canvas::rasterize(canvas_ops) {
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
                    self.last_ops = canvas_ops.to_vec();
                }
            } else if let Some(gpu) = gpu {
                self.release_canvas(gpu);
            } else {
                self.last_ops.clear();
                self.last_dirty = None;
            }
        }

        let Some(paint) = structured else {
            return None;
        };
        let s = paint_uniform_scale(size);
        let [x, y, w, h] = chatbox_rect(min, size);
        let row_h = (ui.frame_height().max(ROW_H_FLOOR) * s).max(1.0);
        let pad = PAD_X * s;
        let header_y = HEADER_Y * s;
        let line_step = (LINE_STEP * s).max(1.0);
        let font_sz = (ui.current_font_size() * s).max(1.0);
        let title_h = (TITLE_H_1X * s).max(1.0);
        let tab_row_h = ((TAB_INSET_1X + TAB_H_1X + TAB_TRAIL_1X) * s).max(1.0);
        let footer_h = if paint.footer.is_some() {
            (FOOTER_LINE_1X * s).max(1.0)
        } else {
            0.0
        };
        let has_strip = paint.strip.is_some();
        let header_h = if has_strip { title_h } else { row_h };
        let height = if self.collapsed { header_h } else { h };
        let toggle = [x + w - header_h, y, x + w, y + header_h];
        if has_strip {
            if ui.is_mouse_hovering_rect([toggle[0], toggle[1]], [toggle[2], toggle[3]])
                && ui.is_mouse_clicked(MouseButton::Left)
            {
                self.collapsed = !self.collapsed;
            }
        } else if ui.is_mouse_hovering_rect([x, y], [x + w, y + row_h])
            && ui.is_mouse_clicked(MouseButton::Left)
        {
            self.collapsed = !self.collapsed;
        }
        let dl = ui.get_window_draw_list();
        let _clip = dl.push_clip_rect([x, y], [x + w, y + height], true);
        dl.add_rect(
            [x, y],
            [x + w, y + height],
            [BG_DEEP[0], BG_DEEP[1], BG_DEEP[2], 0.92],
        )
        .filled(true)
        .build();
        dl.add_rect([x, y], [x + w, y + height], ACCENT)
            .thickness(s)
            .build();
        let font = ui.current_font();
        let glyph = if self.collapsed { "+" } else { "–" };
        let mut chrome_click: Option<(String, String)> = None;
        let mut content_top = y + header_h;
        if let Some(strip) = &paint.strip {
            let mid = y + header_h * 0.5;
            let mut tx = x + pad;
            for name in &strip.names {
                let tw = (text_width(ui, font_sz, name) + STRIP_TAB_PAD_1X * s).max(1.0);
                let box_y = y + 2.0 * s;
                let box_h = (header_h - 4.0 * s).max(1.0);
                let hit = [tx, box_y, tx + tw, box_y + box_h];
                let active = strip.selected == *name;
                let hovered = ui.is_mouse_hovering_rect([hit[0], hit[1]], [hit[2], hit[3]]);
                if active || hovered {
                    dl.add_rect([hit[0], hit[1]], [hit[2], hit[3]], [0.28, 0.28, 0.34, 0.95])
                        .filled(true)
                        .build();
                }
                dl.add_text_with_font(
                    font,
                    font_sz,
                    [tx + 7.0 * s, mid - font_sz * 0.35],
                    if active { ACCENT } else { TEXT_DIM },
                    name,
                    0.0,
                    None,
                );
                let key = format!("strip:{}", strip.id);
                self.chrome_hits
                    .push((key.clone(), name.clone(), hit));
                if hovered && ui.is_mouse_clicked(MouseButton::Left) {
                    chrome_click = Some((key, name.clone()));
                }
                tx += tw + 2.0 * s;
            }
            if let Some(status) = &strip.status {
                let status_w = text_width(ui, font_sz, status);
                dl.add_text_with_font(
                    font,
                    font_sz,
                    [toggle[0] - status_w - pad, mid - font_sz * 0.35],
                    STATUS_FG,
                    status,
                    0.0,
                    None,
                );
            }
            let brand = strip
                .brand
                .as_deref()
                .or(paint.title.as_deref())
                .unwrap_or("");
            if !brand.is_empty() {
                let brand_w = text_width(ui, font_sz, brand);
                dl.add_text_with_font(
                    font,
                    font_sz,
                    [toggle[0] - brand_w - pad * 2.0, mid - font_sz * 0.35],
                    ACCENT,
                    brand,
                    0.0,
                    None,
                );
                self.lines.push(brand.to_string());
            }
            dl.add_text_with_font(
                font,
                font_sz,
                [toggle[0] + 7.0 * s, y + header_y],
                TEXT_DIM,
                glyph,
                0.0,
                None,
            );
        } else {
            let header = match &paint.title {
                Some(t) => format!("{glyph} {t}"),
                None => glyph.to_string(),
            };
            dl.add_text_with_font(
                font,
                font_sz,
                [x + pad, y + header_y],
                ACCENT,
                &header,
                0.0,
                None,
            );
        }
        let mut button_click: Option<String> = None;
        if !self.collapsed {
            if !has_strip {
                if let Some(title) = &paint.title {
                    self.lines.push(title.clone());
                }
            }
            for band in &paint.tabs {
                if band.names.is_empty() {
                    continue;
                }
                let ty = content_top + TAB_INSET_1X * s;
                let mut tx = x + 4.0 * s;
                for name in &band.names {
                    let tw = (text_width(ui, font_sz, name) + STRIP_TAB_PAD_1X * s).max(1.0);
                    let hit = [tx, ty, tx + tw, ty + TAB_H_1X * s];
                    let active = band.selected == *name;
                    let hovered = ui.is_mouse_hovering_rect([hit[0], hit[1]], [hit[2], hit[3]]);
                    if active || hovered {
                        dl.add_rect([hit[0], hit[1]], [hit[2], hit[3]], [0.28, 0.28, 0.34, 0.95])
                            .filled(true)
                            .build();
                    }
                    dl.add_text_with_font(
                        font,
                        font_sz,
                        [tx + 7.0 * s, ty + TAB_H_1X * s * 0.45],
                        if active { ACCENT } else { TEXT_DIM },
                        name,
                        0.0,
                        None,
                    );
                    let key = format!("tabs:{}", band.id);
                    self.chrome_hits
                        .push((key.clone(), name.clone(), hit));
                    if hovered && ui.is_mouse_clicked(MouseButton::Left) {
                        chrome_click = Some((key, name.clone()));
                    }
                    tx += tw + 2.0 * s;
                }
                content_top += tab_row_h;
            }
            let content_bottom = y + height - footer_h;
            let rail_w = paint
                .rail
                .as_ref()
                .filter(|r| !r.names.is_empty())
                .map(|_| (RAIL_W_1X * s).max(1.0))
                .unwrap_or(0.0);
            let body_left = x + pad + rail_w;
            let body_w = (w - pad * 2.0 - rail_w).max(1.0);
            if let Some(rail) = paint.rail.as_ref() {
                if !rail.names.is_empty() {
                    let rail_body_h = (content_bottom - content_top).max(1.0);
                    let slot_h = (rail_body_h / rail.names.len() as f32).max(line_step);
                    for (i, name) in rail.names.iter().enumerate() {
                        let ry = content_top + slot_h * i as f32;
                        let hit = [
                            x + 2.0 * s,
                            ry,
                            x + rail_w - 2.0 * s,
                            (ry + slot_h).min(content_bottom),
                        ];
                        let active = rail.selected == *name;
                        let hovered = ui.is_mouse_hovering_rect([hit[0], hit[1]], [hit[2], hit[3]]);
                        if active || hovered {
                            dl.add_rect([hit[0], hit[1]], [hit[2], hit[3]], [0.28, 0.28, 0.34, 0.95])
                                .filled(true)
                                .build();
                        }
                        if active {
                            dl.add_rect([hit[0], hit[1]], [hit[0] + 2.0 * s, hit[3]], ACCENT)
                                .filled(true)
                                .build();
                        }
                        dl.add_text_with_font(
                            font,
                            font_sz,
                            [hit[0] + 6.0 * s, ry + slot_h * 0.35],
                            if active { TEXT } else { TEXT_DIM },
                            name,
                            0.0,
                            None,
                        );
                        let key = format!("rail:{}", rail.id);
                        self.chrome_hits
                            .push((key.clone(), name.clone(), hit));
                        if hovered && ui.is_mouse_clicked(MouseButton::Left) {
                            chrome_click = Some((key, name.clone()));
                        }
                    }
                }
            }
            let mut ty = content_top;
            for row in &paint.lines {
                self.lines.push(row.clone());
                if ty + line_step <= content_bottom {
                    dl.add_text_with_font(font, font_sz, [body_left, ty], TEXT, row, 0.0, None);
                    ty += line_step;
                }
            }
            let _font = ui.push_font_with_size(None, font_sz);
            let fp = ui.clone_style().frame_padding();
            let _pad = ui.push_style_var(StyleVar::FramePadding([
                (fp[0] * s).max(0.0),
                (fp[1] * s).max(0.0),
            ]));
            for btn in &paint.buttons {
                self.button_labels.push(btn.label.clone());
                if ty + row_h <= content_bottom {
                    ui.set_cursor_screen_pos([body_left, ty]);
                    let hit = [body_left, ty, body_left + body_w, ty + row_h];
                    self.button_hits.push(hit);
                    let pressed = ui.button_with_size(
                        format!("{}##paint-{}", btn.label, btn.id),
                        [body_w, row_h],
                    );
                    let hovered = ui.is_mouse_hovering_rect([hit[0], hit[1]], [hit[2], hit[3]]);
                    if pressed || (hovered && ui.is_mouse_clicked(MouseButton::Left)) {
                        button_click = Some(btn.id.clone());
                    }
                    ty += row_h;
                }
            }
            if let Some(footer) = &paint.footer {
                let footer_w = text_width(ui, font_sz, footer);
                dl.add_text_with_font(
                    font,
                    font_sz,
                    [x + w - pad - footer_w, content_bottom + footer_h * 0.25],
                    TEXT_DIM,
                    footer,
                    0.0,
                    None,
                );
            }
        }
        if chrome_click.is_some() {
            chrome_click.map(|(key, name)| {
                (
                    PaintFrameHit::Select { key, name },
                    paint.generation,
                )
            })
        } else {
            button_click.map(|id| (PaintFrameHit::Button(id), paint.generation))
        }
    }

    fn sync_canvas_texture(
        &mut self,
        gpu: &mut dyn FrameGpu,
        raster: &canvas::Raster,
        ops: &[CanvasOp],
    ) {
        let reuse = self.last_ops.as_slice() == ops
            && self
                .canvas_gpu
                .as_ref()
                .is_some_and(|c| c.w == raster.w && c.h == raster.h);
        if reuse {
            return;
        }
        self.last_ops = ops.to_vec();
        if self
            .canvas_gpu
            .as_ref()
            .is_some_and(|c| c.w != raster.w || c.h != raster.h)
        {
            if let Some(old) = self.canvas_gpu.take() {
                gpu.unregister_texture(old.tex_id);
            }
        }
        if self.canvas_gpu.is_none() {
            let texture = gpu.device().create_texture(&wgpu::TextureDescriptor {
                label: Some("274 script canvas"),
                size: wgpu::Extent3d {
                    width: raster.w.max(1),
                    height: raster.h.max(1),
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
                w: raster.w.max(1),
                h: raster.h.max(1),
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
            &raster.rgba,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4 * raster.w.max(1)),
                rows_per_image: Some(raster.h.max(1)),
            },
            wgpu::Extent3d {
                width: raster.w.max(1),
                height: raster.h.max(1),
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
    use script::shim::ScriptPaint;

    use super::{
        chatbox_rect, paint_uniform_scale, PaintFrameHit, PaintOverlay, CHATBOX, PAD_X,
    };

    fn paint(title: Option<&str>, lines: &[&str]) -> ScriptPaint {
        ScriptPaint {
            title: title.map(str::to_string),
            accent: None,
            lines: lines.iter().map(|l| l.to_string()).collect(),
            buttons: Vec::new(),
            generation: 0,
            canvas: Vec::new(),
            ..Default::default()
        }
    }

    fn paint_with_button(
        title: Option<&str>,
        lines: &[&str],
        id: &str,
        label: &str,
    ) -> ScriptPaint {
        let mut p = paint(title, lines);
        p.buttons.push(script::shim::ScriptPaintButton {
            id: id.to_string(),
            label: label.to_string(),
        });
        p
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
        let _guard = crate::IMGUI_CTX_TEST_GUARD.lock().unwrap();
        let mut ctx = dear_imgui_rs::Context::create();
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
                    overlay.frame(ui, None, Some(&p), [10.0, 20.0], [765.0, 503.0]);
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
        let _guard = crate::IMGUI_CTX_TEST_GUARD.lock().unwrap();
        let mut ctx = dear_imgui_rs::Context::create();
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
        p: &ScriptPaint,
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
                    overlay.frame(ui, None, Some(p), [10.0, 20.0], [765.0, 503.0]);
                });
        }
        ctx.render();
    }

    #[test]
    fn collapse_click_hides_body_and_shrinks_to_title_height() {
        let _guard = crate::IMGUI_CTX_TEST_GUARD.lock().unwrap();
        let mut ctx = dear_imgui_rs::Context::create();
        let mut overlay = PaintOverlay::new();
        let p = paint(Some("BoneBurier"), &["a row", "second row"]);
        // The title row spans the chatbox top strip: hover it, then press.
        let title = [30.0, 380.0];
        paint_click_frame(&mut ctx, &mut overlay, &p, Some(title), false);
        assert!(!overlay.collapsed, "expanded by default");
        paint_click_frame(&mut ctx, &mut overlay, &p, Some(title), true);
        assert!(overlay.collapsed, "`–` collapses to title-only");
        paint_click_frame(&mut ctx, &mut overlay, &p, Some(title), false);
        assert!(
            overlay.lines.is_empty(),
            "collapsed: body rows are not shown"
        );
        assert!(overlay.collapsed);
        // The `+` bar expands again.
        paint_click_frame(&mut ctx, &mut overlay, &p, Some(title), true);
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
        let _guard = crate::IMGUI_CTX_TEST_GUARD.lock().unwrap();
        let mut ctx = dear_imgui_rs::Context::create();
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
                    let clicked = overlay.frame(ui, None, Some(&p), [10.0, 20.0], [765.0, 503.0]);
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
                    let _ = overlay.frame(ui, None, Some(&p), [10.0, 20.0], [765.0, 503.0]);
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
                    clicked = overlay.frame(ui, None, Some(&p), [10.0, 20.0], [765.0, 503.0]);
                });
        }
        ctx.render();
        assert_eq!(
            clicked,
            Some((
                PaintFrameHit::Button("gobank".to_string()),
                p.generation
            )),
            "clicking the button dispatches the advertised id with the rendered generation"
        );
        assert!(!overlay.collapsed, "button click is not collapse");
        paint_click_frame(&mut ctx, &mut overlay, &p, Some(center), false);
        let title = [30.0, 380.0];
        paint_click_frame(&mut ctx, &mut overlay, &p, Some(title), true);
        assert!(overlay.collapsed, "title click still collapses");
        assert!(
            overlay.button_labels.is_empty(),
            "collapsed hides paint widgets"
        );
        paint_click_frame(&mut ctx, &mut overlay, &p, Some(title), false);
        paint_click_frame(&mut ctx, &mut overlay, &p, Some(title), true);
        assert!(!overlay.collapsed);
        assert_eq!(overlay.button_labels, vec!["Go bank".to_string()]);
    }

    fn canvas_banner() -> ScriptPaint {
        let mut p = paint(None, &[]);
        p.canvas = vec![script::canvas::CanvasOp::fill_rect(
            6,
            6,
            400,
            50,
            script::canvas::pack_rgba(0, 0, 0, 178),
        )];
        p
    }

    #[test]
    fn canvas_dirty_rect_maps_native_and_half_blit() {
        let _guard = crate::IMGUI_CTX_TEST_GUARD.lock().unwrap();
        let mut ctx = dear_imgui_rs::Context::create();
        let mut overlay = PaintOverlay::new();
        let p = canvas_banner();
        prepare_frame(&mut ctx);
        {
            let ui = ctx.frame();
            overlay.frame(ui, None, Some(&p), [10.0, 20.0], [765.0, 503.0]);
        }
        ctx.render();
        assert_eq!(overlay.canvas_dirty, Some([6, 6, 400, 50]));
        assert_eq!(overlay.canvas_dest, Some([16.0, 26.0, 400.0, 50.0]));
        prepare_frame(&mut ctx);
        {
            let ui = ctx.frame();
            overlay.frame(ui, None, Some(&p), [0.0, 0.0], [382.5, 251.5]);
        }
        ctx.render();
        let dest = overlay.canvas_dest.expect("scaled dest");
        assert!((dest[0] - 3.0).abs() < 0.01);
        assert!((dest[1] - 3.0).abs() < 0.01);
        assert!((dest[2] - 200.0).abs() < 0.01);
        assert!((dest[3] - 25.0).abs() < 0.01);
    }

    #[test]
    fn canvas_only_frame_does_not_populate_chatbox() {
        let _guard = crate::IMGUI_CTX_TEST_GUARD.lock().unwrap();
        let mut ctx = dear_imgui_rs::Context::create();
        let mut overlay = PaintOverlay::new();
        let p = canvas_banner();
        prepare_frame(&mut ctx);
        {
            let ui = ctx.frame();
            overlay.frame(ui, None, Some(&p), [10.0, 20.0], [765.0, 503.0]);
        }
        ctx.render();
        assert!(overlay.lines.is_empty(), "canvas-only has no chatbox lines");
        assert!(overlay.button_labels.is_empty());
        assert!(overlay.canvas_dest.is_some());
    }

    #[test]
    fn structured_plus_canvas_keeps_chatbox_out_of_applet_dest() {
        let _guard = crate::IMGUI_CTX_TEST_GUARD.lock().unwrap();
        let mut ctx = dear_imgui_rs::Context::create();
        let mut overlay = PaintOverlay::new();
        let mut p = paint(Some("BoneBurier"), &["a row"]);
        p.canvas = canvas_banner().canvas;
        prepare_frame(&mut ctx);
        {
            let ui = ctx.frame();
            overlay.frame(ui, None, Some(&p), [10.0, 20.0], [765.0, 503.0]);
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

    #[test]
    fn measured_fillrect_raster_covers_glyphs() {
        script::canvas::begin();
        script::canvas::set_style("font", "12px monospace");
        let text = "BoneBurier (external)  buried 0";
        let width = script::canvas::measure_text(text).unwrap();
        script::canvas::set_style("fillStyle", "rgba(0, 0, 0, 0.6)");
        script::canvas::fill_rect(6.0, 6.0, width + 12.0, 24.0);
        script::canvas::set_style("fillStyle", "#ffb15b");
        script::canvas::fill_text(text, 12.0, 22.0);
        let taken = script::canvas::take();
        let raster = script::canvas::rasterize(&taken.ops).expect("raster");
        assert!(raster.x <= 6 && raster.y <= 6);
        assert!(raster.x + raster.w as i32 >= 6 + (width + 12.0).round() as i32);
        let mut painted = 0usize;
        for px in raster.rgba.chunks_exact(4) {
            if px[3] > 0 {
                painted += 1;
            }
        }
        assert!(
            painted > 50,
            "glyphs and rect must land in the dirty pixmap"
        );
    }

    struct RecordingGpu {
        device: wgpu::Device,
        queue: wgpu::Queue,
        registered: Vec<u64>,
        unregistered: Vec<u64>,
    }

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

    fn headless_gpu() -> Option<(wgpu::Device, wgpu::Queue)> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: None,
            force_fallback_adapter: false,
        }))
        .ok()?;
        pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("274 paint canvas test"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::default(),
            experimental_features: wgpu::ExperimentalFeatures::default(),
            memory_hints: wgpu::MemoryHints::default(),
            trace: wgpu::Trace::default(),
        }))
        .ok()
    }

    fn gpu_frame(
        ctx: &mut dear_imgui_rs::Context,
        overlay: &mut PaintOverlay,
        gpu: &mut RecordingGpu,
        p: &ScriptPaint,
        min: [f32; 2],
        size: [f32; 2],
    ) {
        prepare_frame(ctx);
        {
            let ui = ctx.frame();
            overlay.frame(ui, Some(gpu), Some(p), min, size);
        }
        ctx.render();
    }

    #[test]
    fn canvas_gpu_cache_skips_unchanged_and_releases() {
        let Some((device, queue)) = headless_gpu() else {
            return;
        };
        let _guard = crate::IMGUI_CTX_TEST_GUARD.lock().unwrap();
        let mut ctx = dear_imgui_rs::Context::create();
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
        let _guard = crate::IMGUI_CTX_TEST_GUARD.lock().unwrap();
        let mut ctx = dear_imgui_rs::Context::create();
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
                    overlay.frame(ui, None, Some(&p), [10.0, 20.0], [765.0, 503.0]);
                });
        }
        ctx.render();
        let native_hit = overlay.button_hits[0];
        let native_chat = chatbox_rect([10.0, 20.0], [765.0, 503.0]);
        assert!(
            (native_hit[0] - (native_chat[0] + PAD_X)).abs() < 0.5,
            "native pad anchors the hit left edge"
        );
        assert!(
            (native_hit[2] - (native_chat[0] + native_chat[2] - PAD_X)).abs() < 0.5,
            "native pad anchors the hit right edge"
        );

        prepare_frame(&mut ctx);
        {
            let ui = ctx.frame();
            let _ = ui
                .window("Game")
                .position([0.0, 0.0], dear_imgui_rs::Condition::Always)
                .size([900.0, 700.0], dear_imgui_rs::Condition::Always)
                .build(|| {
                    overlay.frame(ui, None, Some(&p), [0.0, 0.0], [382.5, 251.5]);
                });
        }
        ctx.render();
        let half_hit = overlay.button_hits[0];
        let half_chat = chatbox_rect([0.0, 0.0], [382.5, 251.5]);
        let s = paint_uniform_scale([382.5, 251.5]);
        assert!((s - 0.5).abs() < 0.001);
        assert!(
            (half_hit[0] - (half_chat[0] + PAD_X * s)).abs() < 0.5,
            "half-scale pad left ({half_hit:?} vs chat {half_chat:?})"
        );
        assert!(
            (half_hit[2] - (half_chat[0] + half_chat[2] - PAD_X * s)).abs() < 0.5,
            "half-scale pad right"
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

    fn jive_strip_paint() -> ScriptPaint {
        ScriptPaint {
            title: Some("JiveCrafting".into()),
            generation: 3,
            strip: Some(script::shim::PaintChromeBand {
                id: "k".into(),
                names: vec!["Statistics".into(), "Options".into()],
                selected: "Statistics".into(),
                status: Some("ok".into()),
                brand: Some("JiveCrafting".into()),
            }),
            lines: vec!["Overview row".into()],
            ..Default::default()
        }
    }

    #[test]
    fn strip_chrome_click_maps_scaled_hit_to_paint_select() {
        let _guard = crate::IMGUI_CTX_TEST_GUARD.lock().unwrap();
        let mut ctx = dear_imgui_rs::Context::create();
        let mut overlay = PaintOverlay::new();
        let p = jive_strip_paint();
        let half = [382.5_f32, 251.5];
        prepare_frame(&mut ctx);
        {
            let ui = ctx.frame();
            let _ = ui
                .window("Game")
                .position([0.0, 0.0], dear_imgui_rs::Condition::Always)
                .size([900.0, 700.0], dear_imgui_rs::Condition::Always)
                .build(|| {
                    overlay.frame(ui, None, Some(&p), [0.0, 0.0], half);
                });
        }
        ctx.render();
        assert!(
            overlay.chrome_hits.iter().any(|(k, n, _)| k == "strip:k" && n == "Options"),
            "strip advertises both page targets"
        );
        let hit = overlay
            .chrome_hits
            .iter()
            .find(|(_, n, _)| n == "Options")
            .map(|(_, _, r)| *r)
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
                    clicked = overlay.frame(ui, None, Some(&p), [0.0, 0.0], half);
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
                p.generation
            ))
        );
    }

    #[test]
    fn collapse_hit_strip_scales_with_grid_cell() {
        let _guard = crate::IMGUI_CTX_TEST_GUARD.lock().unwrap();
        let mut ctx = dear_imgui_rs::Context::create();
        let mut overlay = PaintOverlay::new();
        let p = paint(Some("BoneBurier"), &["a row"]);
        let half = [382.5_f32, 251.5];
        let chat = chatbox_rect([0.0, 0.0], half);
        let s = paint_uniform_scale(half);
        // Title strip center at half scale (row_h ≈ frame_height * s).
        prepare_frame(&mut ctx);
        let title = [chat[0] + chat[2] * 0.5, chat[1] + 4.0 * s];
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
                    overlay.frame(ui, None, Some(&p), [0.0, 0.0], half);
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
                    overlay.frame(ui, None, Some(&p), [0.0, 0.0], half);
                });
        }
        ctx.render();
        assert!(
            overlay.collapsed,
            "title click inside the scaled strip collapses"
        );
        assert!(overlay.lines.is_empty());
    }

    #[test]
    fn path_op_frame_reuses_texture_on_identical_ops() {
        let Some((device, queue)) = headless_gpu() else {
            return;
        };
        let _guard = crate::IMGUI_CTX_TEST_GUARD.lock().unwrap();
        let mut ctx = dear_imgui_rs::Context::create();
        let mut overlay = PaintOverlay::new();
        let mut gpu = RecordingGpu::new(device, queue);
        let mut p = paint(None, &[]);
        p.canvas = vec![script::canvas::CanvasOp::StrokePath {
            segs: vec![
                script::canvas::PathSeg::MoveTo { x: 40.0, y: 40.0 },
                script::canvas::PathSeg::LineTo { x: 80.0, y: 90.0 },
            ],
            color: script::canvas::pack_rgba(255, 224, 64, 140),
            line_width: 1.5,
            line_join: script::canvas::LineJoinKind::Round,
            extras: script::canvas::DrawExtras::default(),
        }];
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
}
