//! Game Image: RGBA8 765×503 texture filled from `FrameBuf` snapshots.
//!
//! The client paints a 765×503 applet (`client::APPLET_W/H`); the panel keeps
//! the texture at exactly that size and lets the Image widget scale it.

use std::sync::Arc;

use dear_app::GpuApi;
use dear_imgui_rs::TextureId;
use host::FrameBuf;

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

/// Snapshot for the game image: the slot's `FrameBuf` when wired, else a
/// black applet. CPU path only — the mailbox's `PixMap`; Task 4c binds
/// `FrameOutput::Texture` at the call site instead of uploading.
pub fn game_pixels(pixels: &Option<Arc<FrameBuf>>) -> Vec<u32> {
    match pixels {
        Some(p) => p.snapshot(),
        None => vec![0; (APPLET_W * APPLET_H) as usize],
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
    use super::{expand_rgba, game_pixels, APPLET_H, APPLET_W};

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
    fn game_pixels_without_slot_is_black_applet() {
        let black = game_pixels(&None);
        assert_eq!(black.len(), (APPLET_W * APPLET_H) as usize);
        assert!(black.iter().all(|&p| p == 0));
    }
}
