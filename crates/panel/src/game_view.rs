//! Game Image: RGBA8 765×503 texture filled from the mailbox's taken
//! `FrameOutput`s.
//!
//! The client paints a 765×503 applet (`client::APPLET_W/H`); the panel keeps
//! the texture at exactly that size and lets the Image widget scale it.
//!
//! Two consume paths (the shared-device seam): the CPU backend's `PixMap`
//! frames upload into the panel-owned texture; the GPU backend's `Texture`
//! frames register the client's frame-texture view directly with the ImGui
//! renderer — the client renders on the panel's injected device, so the
//! frame never round-trips through the CPU. `frame_pixels` keeps the
//! read-back fallback for consumers that still want CPU pixels.

use crate::window::Gpu;
use client::graphics::PixMap;
use client::render::backend::{FrameOutput, TextureHandle};
use dear_imgui_rs::TextureId;

/// Applet draw size the client always paints (never DPI-scaled).
pub const APPLET_W: u32 = 765;
pub const APPLET_H: u32 = 503;

/// The panel window's GPU calls the game-view texture paths need, abstracted
/// so the consume path is unit-testable without a full ImGui renderer (the
/// `Texture`-frame tests drive a recording registrar on a headless device).
pub trait FrameGpu {
    fn device(&self) -> &wgpu::Device;
    fn queue(&self) -> &wgpu::Queue;
    fn register_texture(&mut self, texture: &wgpu::Texture, view: &wgpu::TextureView) -> TextureId;
    fn unregister_texture(&mut self, tex_id: TextureId);
}

impl FrameGpu for Gpu<'_> {
    fn device(&self) -> &wgpu::Device {
        Gpu::device(self)
    }
    fn queue(&self) -> &wgpu::Queue {
        Gpu::queue(self)
    }
    fn register_texture(&mut self, texture: &wgpu::Texture, view: &wgpu::TextureView) -> TextureId {
        Gpu::register_texture(self, texture, view)
    }
    fn unregister_texture(&mut self, tex_id: TextureId) {
        Gpu::unregister_texture(self, tex_id)
    }
}

/// RGBA8 game texture registered with the ImGui renderer. Texture data is
/// always APPLET_W×APPLET_H; only the Image widget display size scales.
pub struct GameView {
    pub tex_id: TextureId,
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    rgba: Vec<u8>,
    /// Which texture `tex_id` currently names: the panel-owned `texture`
    /// (the CPU/upload path) or a client's frame texture (the shared GPU
    /// path). wgpu handles compare by id, so re-registration happens only
    /// on a mode switch or a new client frame texture (a slot restart).
    bound: Bound,
}

/// Which texture the view's `tex_id` names. `Client` holds the client's
/// frame texture (a cheap clone) so the direct-bind path detects a new
/// client texture without re-registering every frame.
#[derive(Clone, PartialEq)]
enum Bound {
    /// The panel-owned `texture`, registered by [`GameView::init`] (and
    /// re-registered after a GPU-frame spell).
    Owned,
    /// A client's frame texture (the shared-device seam), registered
    /// directly into the ImGui renderer.
    Client(wgpu::Texture),
}

impl GameView {
    /// Create the 765×503 RGBA8 texture and register it. Call once from the
    /// first frame, when `gpu.device()` is live.
    pub fn init(gpu: &mut impl FrameGpu) -> Self {
        let texture = gpu.device().create_texture(&wgpu::TextureDescriptor {
            label: Some("274 game image"),
            size: wgpu::Extent3d {
                width: APPLET_W,
                height: APPLET_H,
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
        Self {
            tex_id,
            texture,
            view,
            rgba: vec![0u8; (APPLET_W * APPLET_H * 4) as usize],
            bound: Bound::Owned,
        }
    }

    /// Route a taken frame to the view: `PixMap` (CPU backend) frames
    /// upload into the panel-owned texture; `Texture` (GPU backend) frames
    /// bind the client's frame-texture view directly. The GPU arm never
    /// reads the frame back — the client and panel share one device.
    pub fn present(&mut self, gpu: &mut impl FrameGpu, frame: FrameOutput) {
        match frame {
            FrameOutput::PixMap(pix) => self.upload(gpu, &pixmap_pixels(&pix)),
            FrameOutput::Texture(handle) => self.bind(gpu, &handle),
        }
    }

    /// Direct-bind a client GPU frame (the shared-device seam): register
    /// the client's frame-texture view with the ImGui renderer instead of
    /// reading it back, and make the Image draw it. Re-registers only when
    /// the underlying client texture changes — a client reuses one
    /// `frame_texture` per backend, so the per-frame case is a no-op. A
    /// transition unregisters the texture the view last drew, so the
    /// renderer's texture cache stays bounded across slot restarts.
    pub fn bind(&mut self, gpu: &mut impl FrameGpu, handle: &TextureHandle) {
        let texture = handle.view.texture();
        if matches!(&self.bound, Bound::Client(held) if held == texture) {
            return;
        }
        gpu.unregister_texture(self.tex_id);
        self.tex_id = gpu.register_texture(texture, &handle.view);
        self.bound = Bound::Client(texture.clone());
    }

    /// Upload packed `0x00RRGGBB` pixels (a `FrameBuf` snapshot) into the
    /// texture. `Gpu` exposes no per-texture sampler choice, so
    /// the Image samples with the renderer's default rather than a pixelated
    /// one. Reuses an RGBA scratch buffer so Poll-rate frames don't allocate.
    pub fn upload(&mut self, gpu: &mut impl FrameGpu, pixels: &[u32]) {
        if self.bound != Bound::Owned {
            gpu.unregister_texture(self.tex_id);
            self.tex_id = gpu.register_texture(&self.texture, &self.view);
            self.bound = Bound::Owned;
        }
        let n = (APPLET_W * APPLET_H) as usize;
        self.rgba.resize(n * 4, 0);
        expand_rgba(&pixels[..n.min(pixels.len())], &mut self.rgba);
        gpu.queue().write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &self.rgba,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(APPLET_W * 4),
                rows_per_image: Some(APPLET_H),
            },
            wgpu::Extent3d {
                width: APPLET_W,
                height: APPLET_H,
                depth_or_array_layers: 1,
            },
        );
    }
}

/// Pack a `PixMap` frame (the CPU backend) into `0x00RRGGBB` pixels,
/// shape-gated like the old `FrameBuf::snapshot`.
fn pixmap_pixels(pix: &PixMap) -> Vec<u32> {
    let n = (APPLET_W * APPLET_H) as usize;
    let mut out = Vec::with_capacity(n);
    if pix.width == APPLET_W as i32 && pix.height == APPLET_H as i32 && pix.pixels.len() >= n {
        for src in pix.pixels.iter().take(n) {
            out.push(*src as u32);
        }
    }
    out
}

/// Pack a taken [`FrameOutput`] into `0x00RRGGBB` pixels. The `PixMap` arm
/// (the CPU backend) packs the applet directly; the `Texture` arm reads the
/// wgpu frame back — the pre-seam fallback. The panel's consume path
/// ([`GameView::present`]) binds `Texture` frames directly instead, so this
/// is the CPU-pixel path for consumers that still need it.
pub fn frame_pixels(frame: FrameOutput) -> Vec<u32> {
    match frame {
        FrameOutput::PixMap(pix) => pixmap_pixels(&pix),
        FrameOutput::Texture(handle) => {
            let n = (handle.width as usize) * (handle.height as usize);
            handle
                .read_back()
                .into_iter()
                .take(n)
                .map(|p| p as u32)
                .collect()
        }
    }
}

/// Expand packed `0x00RRGGBB` pixels into RGBA8 bytes with alpha 255.
fn expand_rgba(src: &[u32], dst: &mut [u8]) {
    for (i, p) in src.iter().enumerate() {
        dst[i * 4] = ((*p >> 16) & 0xff) as u8;
        dst[i * 4 + 1] = ((*p >> 8) & 0xff) as u8;
        dst[i * 4 + 2] = (*p & 0xff) as u8;
        dst[i * 4 + 3] = 255;
    }
}

#[cfg(test)]
mod tests {
    use super::{expand_rgba, frame_pixels, FrameGpu, GameView, APPLET_H, APPLET_W};
    use client::graphics::PixMap;
    use client::render::backend::{FrameOutput, TextureHandle};
    use dear_imgui_rs::TextureId;

    #[test]
    fn expand_rgba_splits_0x00rrggbb() {
        let mut dst = [0u8; 4];
        expand_rgba(&[0x00aa_bbcc], &mut dst);
        assert_eq!(dst, [0xaa, 0xbb, 0xcc, 255]);
    }

    #[test]
    fn expand_rgba_writes_alpha_for_every_pixel() {
        let mut dst = [0u8; 8];
        expand_rgba(&[0x00aa_bbcc, 0x0011_2233], &mut dst);
        assert_eq!(dst, [0xaa, 0xbb, 0xcc, 255, 0x11, 0x22, 0x33, 255]);
    }

    #[test]
    fn frame_pixels_packs_a_full_applet_pixmap() {
        let mut pix = vec![0i32; (APPLET_W * APPLET_H) as usize];
        pix[0] = 0x00aa_bbcc;
        pix[1] = 0x0011_2233;
        let out = frame_pixels(FrameOutput::PixMap(PixMap {
            width: APPLET_W as i32,
            height: APPLET_H as i32,
            pixels: pix,
        }));
        assert_eq!(out.len(), (APPLET_W * APPLET_H) as usize);
        assert_eq!(out[0], 0x00aa_bbcc);
        assert_eq!(out[1], 0x0011_2233);
    }

    #[test]
    fn frame_pixels_of_wrong_shape_pixmap_is_empty() {
        let out = frame_pixels(FrameOutput::PixMap(PixMap {
            width: 2,
            height: 2,
            pixels: vec![1, 2, 3],
        }));
        assert!(out.is_empty());
    }

    /// A real headless wgpu device/queue on this machine's adapter
    /// (`None` when no adapter exists — the GPU tests then skip).
    fn headless_gpu() -> Option<(wgpu::Device, wgpu::Queue)> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: None,
            force_fallback_adapter: false,
        }))
        .ok()?;
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("274 panel test"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::default(),
            experimental_features: wgpu::ExperimentalFeatures::default(),
            memory_hints: wgpu::MemoryHints::default(),
            trace: wgpu::Trace::default(),
        }))
        .ok()?;
        Some((device, queue))
    }

    /// A real 2×2 RGBA8 texture on the given device, written with four
    /// known pixels, as a client `TextureHandle` would arrive from
    /// `FrameOutput::Texture`.
    fn frame_handle(device: &wgpu::Device, queue: &wgpu::Queue) -> TextureHandle {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("274 panel test frame"),
            size: wgpu::Extent3d {
                width: 2,
                height: 2,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_DST
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &[
                255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255,
            ],
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(2 * 4),
                rows_per_image: Some(2),
            },
            wgpu::Extent3d {
                width: 2,
                height: 2,
                depth_or_array_layers: 1,
            },
        );
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        TextureHandle {
            device: device.clone(),
            queue: queue.clone(),
            view,
            width: 2,
            height: 2,
        }
    }

    /// A real 2×2 RGBA8 texture on a headless wgpu device, written with
    /// four known pixels. `None` when no adapter exists (headless CI) —
    /// the read-back test then skips.
    fn test_texture_handle() -> Option<TextureHandle> {
        let (device, queue) = headless_gpu()?;
        Some(frame_handle(&device, &queue))
    }

    #[test]
    fn frame_pixels_reads_a_texture_frame_back() {
        let Some(handle) = test_texture_handle() else {
            return; // no adapter: headless CI has nothing to read back
        };
        let pixels = frame_pixels(FrameOutput::Texture(handle));
        assert_eq!(
            pixels,
            vec![0x00ff_0000, 0x0000_ff00, 0x0000_00ff, 0x00ff_ffff]
        );
    }

    /// A recording [`FrameGpu`] on a real headless device: every
    /// `register_texture`/`unregister_texture` records the id (and the
    /// registered texture), so the direct-bind path is testable without a
    /// full ImGui renderer.
    struct RecordingGpu {
        device: wgpu::Device,
        queue: wgpu::Queue,
        /// `(assigned id, registered texture)` in registration order.
        registered: Vec<(u64, wgpu::Texture)>,
        /// Unregistered ids, in order.
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
        fn last_registered(&self) -> Option<&wgpu::Texture> {
            self.registered.last().map(|(_, t)| t)
        }
    }

    impl FrameGpu for RecordingGpu {
        fn device(&self) -> &wgpu::Device {
            &self.device
        }
        fn queue(&self) -> &wgpu::Queue {
            &self.queue
        }
        fn register_texture(
            &mut self,
            texture: &wgpu::Texture,
            _view: &wgpu::TextureView,
        ) -> TextureId {
            let id = self.registered.len() as u64 + 1;
            self.registered.push((id, texture.clone()));
            TextureId::new(id)
        }
        fn unregister_texture(&mut self, tex_id: TextureId) {
            self.unregistered.push(tex_id.id());
        }
    }

    /// The shared-device seam: a `Texture` frame's consume path registers
    /// the client's frame-texture view directly with the renderer — the
    /// `read_back` fallback is never on this path. A real 2×2 client
    /// texture (same device the panel's own texture lives on, as in
    /// production) plus a recording registrar prove the routing: init
    /// registers the panel texture; the first `Texture` frame registers
    /// the client's view and adopts its id; a repeat frame of the same
    /// client texture is a no-op; a new client texture re-registers and
    /// unregisters the old one; a `PixMap` frame switches back to the
    /// panel-owned texture.
    #[test]
    fn texture_frame_binds_the_client_view_directly() {
        let Some((device, queue)) = headless_gpu() else {
            return; // no adapter: headless CI has nothing to bind
        };
        let mut gpu = RecordingGpu::new(device.clone(), queue.clone());
        let mut view = GameView::init(&mut gpu);
        assert_eq!(
            gpu.registered.len(),
            1,
            "init registers the panel-owned texture"
        );
        assert_eq!(view.tex_id.id(), gpu.registered[0].0);

        // The `Texture` consume path: bind the client's view directly (no
        // `read_back`), and the Image draws the client's frame texture.
        // The transition unregisters the panel texture init registered.
        let handle = frame_handle(&device, &queue);
        let client_texture = handle.view.texture().clone();
        view.present(&mut gpu, FrameOutput::Texture(handle));
        assert_eq!(
            gpu.registered.len(),
            2,
            "the Texture frame must register the client's view"
        );
        assert_eq!(
            view.tex_id.id(),
            2,
            "the Image must now draw the client's frame texture"
        );
        assert_eq!(
            gpu.last_registered(),
            Some(&client_texture),
            "the registered texture must be the client's frame texture"
        );
        assert_eq!(
            gpu.unregistered,
            vec![1],
            "switching to the client frame must unregister the panel texture"
        );

        // The client reuses one `frame_texture` per backend: a re-taken
        // frame of the same texture must not re-register.
        let re_take = TextureHandle {
            device: device.clone(),
            queue: queue.clone(),
            view: client_texture.create_view(&Default::default()),
            width: 2,
            height: 2,
        };
        view.present(&mut gpu, FrameOutput::Texture(re_take));
        assert_eq!(
            gpu.registered.len(),
            2,
            "re-binding the same client frame texture is a no-op"
        );

        // A new client texture (a slot restart) re-registers and
        // unregisters the previous client frame.
        let fresh = frame_handle(&device, &queue);
        view.present(&mut gpu, FrameOutput::Texture(fresh));
        assert_eq!(
            gpu.registered.len(),
            3,
            "a new client frame texture re-registers"
        );
        assert_eq!(view.tex_id.id(), 3);
        assert_eq!(
            gpu.unregistered,
            vec![1, 2],
            "the old client frame must be unregistered"
        );

        // Back to the CPU path: a `PixMap` frame re-registers the
        // panel-owned texture (unregistering the client frame) and uploads
        // into it.
        let mut pix = vec![0i32; (APPLET_W * APPLET_H) as usize];
        pix[0] = 0x00aa_bbcc;
        view.present(
            &mut gpu,
            FrameOutput::PixMap(PixMap {
                width: APPLET_W as i32,
                height: APPLET_H as i32,
                pixels: pix,
            }),
        );
        assert_eq!(
            gpu.registered.len(),
            4,
            "the PixMap frame must re-register the panel texture"
        );
        assert_eq!(
            view.tex_id.id(),
            4,
            "the Image must draw the panel's texture again"
        );
        assert_eq!(
            gpu.last_registered(),
            Some(&view.texture),
            "the registered texture must be the panel-owned texture"
        );
        assert_eq!(
            gpu.unregistered,
            vec![1, 2, 3],
            "every replaced texture must be unregistered (bounded renderer cache)"
        );
    }
}
