//! Script-paint overlay for the Game Image.
//!
//! Structured Paint is drawn on the Game window draw list over the client's
//! chatbox rect — never a second ImGui window and never pixels on the
//! 765×503 game texture. Canvas ops rasterize to a small cached transparent
//! texture over the applet, using the same native font as measureText.

use dear_imgui_rs::{MouseButton, TextureId, Ui};
use script::canvas::{self, CanvasOp};
use script::shim::ScriptPaint;

use crate::game_view::{FrameGpu, APPLET_H, APPLET_W};
use crate::theme::{ACCENT, BG_DEEP, TEXT};

/// Applet-space chatbox rect `(x, y, w, h)` the client reserves for game
/// chat on the 765×503 stage.
pub const CHATBOX: [f32; 4] = [8.0, 345.0, 506.0, 150.0];

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
    ) -> Option<(String, u64)> {
        self.lines.clear();
        self.button_labels.clear();
        self.button_hits.clear();
        self.canvas_dest = None;
        self.canvas_dirty = None;

        let structured =
            paint.filter(|p| p.title.is_some() || !p.lines.is_empty() || !p.buttons.is_empty());
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
        let [x, y, w, h] = chatbox_rect(min, size);
        let row_h = ui.frame_height().max(16.0);
        let height = if self.collapsed { row_h } else { h };
        if ui.is_mouse_hovering_rect([x, y], [x + w, y + row_h])
            && ui.is_mouse_clicked(MouseButton::Left)
        {
            self.collapsed = !self.collapsed;
        }
        let dl = ui.get_window_draw_list();
        dl.add_rect(
            [x, y],
            [x + w, y + height],
            [BG_DEEP[0], BG_DEEP[1], BG_DEEP[2], 0.92],
        )
        .filled(true)
        .build();
        dl.add_rect([x, y], [x + w, y + height], ACCENT)
            .thickness(1.0)
            .build();
        let glyph = if self.collapsed { "+" } else { "–" };
        let header = match &paint.title {
            Some(t) => format!("{glyph} {t}"),
            None => glyph.to_string(),
        };
        dl.add_text([x + 6.0, y + 2.0], ACCENT, &header);
        let mut clicked = None;
        if !self.collapsed {
            if let Some(title) = &paint.title {
                self.lines.push(title.clone());
            }
            let mut ty = y + row_h;
            for row in &paint.lines {
                self.lines.push(row.clone());
                if ty + 14.0 <= y + height {
                    dl.add_text([x + 6.0, ty], TEXT, row);
                    ty += 14.0;
                }
            }
            for btn in &paint.buttons {
                self.button_labels.push(btn.label.clone());
                if ty + row_h <= y + height {
                    ui.set_cursor_screen_pos([x + 6.0, ty]);
                    let hit = [x + 6.0, ty, x + w - 6.0, ty + row_h];
                    self.button_hits.push(hit);
                    let pressed = ui.button(format!("{}##paint-{}", btn.label, btn.id));
                    let hovered = ui.is_mouse_hovering_rect([hit[0], hit[1]], [hit[2], hit[3]]);
                    if pressed || (hovered && ui.is_mouse_clicked(MouseButton::Left)) {
                        clicked = Some((btn.id.clone(), paint.generation));
                    }
                    ty += row_h;
                }
            }
        }
        clicked
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

    use super::{chatbox_rect, PaintOverlay, CHATBOX};

    fn paint(title: Option<&str>, lines: &[&str]) -> ScriptPaint {
        ScriptPaint {
            title: title.map(str::to_string),
            accent: None,
            lines: lines.iter().map(|l| l.to_string()).collect(),
            buttons: Vec::new(),
            generation: 0,
            canvas: Vec::new(),
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
            Some(("gobank".to_string(), p.generation)),
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
        p.canvas = vec![script::canvas::CanvasOp::FillRect {
            x: 6,
            y: 6,
            w: 400,
            h: 50,
            color: script::canvas::pack_rgba(0, 0, 0, 178),
        }];
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
}
