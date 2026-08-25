//! Game Image: RGBA8 765×503 texture filled from the mailbox's taken
//! `FrameOutput`s.
//!
//! The client paints a 765×503 applet (`client::APPLET_W/H`); the panel keeps
//! the texture at exactly that size and lets the Image widget scale it.

use client::render::backend::FrameOutput;
use dear_app::GpuApi;
use dear_imgui_rs::TextureId;

/// Applet draw size the client always paints (never DPI-scaled).
pub const APPLET_W: u32 = 765;
pub const APPLET_H: u32 = 503;

/// RGBA8 game texture registered with the ImGui renderer. Texture data is
/// always APPLET_W×APPLET_H; only the Image widget display size scales.
pub struct GameView {
    pub tex_id: TextureId,
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    rgba: Vec<u8>,
}

impl GameView {
    /// Create the 765×503 RGBA8 texture and register it. Call once from the
    /// first `on_frame`, when `gpu.device()` is live.
    pub fn init(gpu: &mut GpuApi) -> Self {
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
        }
    }

    /// Upload packed `0x00RRGGBB` pixels (a `FrameBuf` snapshot) into the
    /// texture. dear-app's `GpuApi` exposes no per-texture sampler choice, so
    /// the Image samples with the renderer's default rather than a pixelated
    /// one. Reuses an RGBA scratch buffer so Poll-rate frames don't allocate.
    pub fn upload(&mut self, gpu: &GpuApi, pixels: &[u32]) {
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

/// Pack a taken [`FrameOutput`] into `0x00RRGGBB` pixels for
/// [`GameView::upload`]. The `PixMap` arm (the CPU backend) packs the
/// applet directly, shape-gated like the old `FrameBuf::snapshot`; the
/// `Texture` arm (the GPU backend) reads the wgpu frame back through the
/// client's device. The panel has its own wgpu device, so a client-side
/// texture view cannot be bound into the ImGui renderer — the GPU frame
/// lands as a read-back + upload (the campaign's documented deviation; a
/// shared-device seam would replace the read-back with a direct view
/// bind).
pub fn frame_pixels(frame: FrameOutput) -> Vec<u32> {
    match frame {
        FrameOutput::PixMap(pix) => {
            let n = (APPLET_W * APPLET_H) as usize;
            let mut out = Vec::with_capacity(n);
            if pix.width == APPLET_W as i32
                && pix.height == APPLET_H as i32
                && pix.pixels.len() >= n
            {
                for src in pix.pixels.iter().take(n) {
                    out.push(*src as u32);
                }
            }
            out
        }
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
    use super::{expand_rgba, frame_pixels, APPLET_H, APPLET_W};
    use client::graphics::PixMap;
    use client::render::backend::{FrameOutput, TextureHandle};

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

    /// A real 2×2 RGBA8 texture on a headless wgpu device, written with
    /// four known pixels. `None` when no adapter exists (headless CI) —
    /// the read-back test then skips.
    fn test_texture_handle() -> Option<TextureHandle> {
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
        Some(TextureHandle {
            device,
            queue,
            view,
            width: 2,
            height: 2,
        })
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
}
