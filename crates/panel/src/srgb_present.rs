use dear_imgui_rs::render::DrawData;
use dear_imgui_rs::Context;
use dear_imgui_wgpu::{GammaMode, WgpuInitInfo, WgpuRenderer};
use pollster::block_on;
use std::sync::mpsc;
use std::time::Duration;

const CELL: u32 = 8;
const SWATCHES: [[u8; 4]; 5] = [
    [0, 0, 0, 255],
    [9, 7, 7, 255],
    [116, 89, 42, 255],
    [17, 17, 17, 255],
    [255, 255, 255, 255],
];
const WIDTH: u32 = CELL * SWATCHES.len() as u32;

fn headless_gpu() -> Option<(wgpu::Device, wgpu::Queue, wgpu::AdapterInfo)> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter = block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        compatible_surface: None,
        force_fallback_adapter: false,
    }))
    .ok()?;
    let info = adapter.get_info();
    let (device, queue) = block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("274 panel sRGB present test"),
        required_features: wgpu::Features::empty(),
        required_limits: wgpu::Limits::default(),
        experimental_features: wgpu::ExperimentalFeatures::default(),
        memory_hints: wgpu::MemoryHints::default(),
        trace: wgpu::Trace::default(),
    }))
    .ok()?;
    Some((device, queue, info))
}

fn render_swatches(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    format: wgpu::TextureFormat,
    gamma_mode: GammaMode,
) -> Vec<[u8; 4]> {
    let mut context = Context::create();
    context
        .io_mut()
        .set_display_size([WIDTH as f32, CELL as f32]);

    let mut renderer = WgpuRenderer::new(
        WgpuInitInfo::new(device.clone(), queue.clone(), format),
        &mut context,
    )
    .expect("initialize Dear ImGui WGPU renderer");
    renderer.set_gamma_mode(gamma_mode);

    let mut texels = vec![0; (WIDTH * CELL * 4) as usize];
    for (index, rgba) in SWATCHES.iter().enumerate() {
        for y in 0..CELL {
            for x in 0..CELL {
                let offset = ((y * WIDTH + index as u32 * CELL + x) * 4) as usize;
                texels[offset..offset + 4].copy_from_slice(rgba);
            }
        }
    }

    let source = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("274 panel sRGB swatches"),
        size: wgpu::Extent3d {
            width: WIDTH,
            height: CELL,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &source,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        &texels,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(WIDTH * 4),
            rows_per_image: Some(CELL),
        },
        wgpu::Extent3d {
            width: WIDTH,
            height: CELL,
            depth_or_array_layers: 1,
        },
    );
    let source_view = source.create_view(&wgpu::TextureViewDescriptor::default());
    let texture_id = renderer.register_external_texture(&source, &source_view);

    let draw_data = {
        {
            let ui = context.frame();
            ui.get_foreground_draw_list().add_image(
                texture_id,
                [0.0, 0.0],
                [WIDTH as f32, CELL as f32],
                [0.0, 0.0],
                [1.0, 1.0],
                [1.0, 1.0, 1.0, 1.0],
            );
        }
        context.render()
    };

    let rgba = render_and_read_back(device, queue, &mut renderer, draw_data, format);
    SWATCHES
        .iter()
        .enumerate()
        .map(|(index, _)| {
            let x = index as u32 * CELL + CELL / 2;
            let offset = ((CELL / 2 * WIDTH + x) * 4) as usize;
            rgba[offset..offset + 4].try_into().unwrap()
        })
        .collect()
}

fn render_and_read_back(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    renderer: &mut WgpuRenderer,
    draw_data: &mut DrawData,
    format: wgpu::TextureFormat,
) -> Vec<u8> {
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("274 panel sRGB present target"),
        size: wgpu::Extent3d {
            width: WIDTH,
            height: CELL,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let target_view = target.create_view(&wgpu::TextureViewDescriptor::default());
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("274 panel sRGB present encoder"),
    });
    {
        let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("274 panel sRGB present pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &target_view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
                depth_slice: None,
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        renderer.new_frame().expect("begin Dear ImGui frame");
        renderer
            .render_draw_data(draw_data, &mut render_pass)
            .expect("render Dear ImGui draw data");
    }

    let padded_bytes_per_row = (WIDTH * 4).div_ceil(256) * 256;
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("274 panel sRGB present readback"),
        size: (padded_bytes_per_row * CELL) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: &target,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &readback,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded_bytes_per_row),
                rows_per_image: Some(CELL),
            },
        },
        wgpu::Extent3d {
            width: WIDTH,
            height: CELL,
            depth_or_array_layers: 1,
        },
    );
    queue.submit([encoder.finish()]);

    let slice = readback.slice(..);
    let (sender, receiver) = mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |result| {
        sender.send(result).expect("send map result");
    });
    device
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(Duration::from_secs(10)),
        })
        .expect("wait for sRGB present readback");
    receiver
        .recv_timeout(Duration::from_secs(1))
        .expect("receive map result")
        .expect("map sRGB present readback");

    let mapped = slice.get_mapped_range();
    let mut rgba = Vec::with_capacity((WIDTH * CELL * 4) as usize);
    for y in 0..CELL as usize {
        let start = y * padded_bytes_per_row as usize;
        rgba.extend_from_slice(&mapped[start..start + (WIDTH * 4) as usize]);
    }
    drop(mapped);
    readback.unmap();

    if matches!(
        format,
        wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb
    ) {
        for pixel in rgba.as_chunks_mut::<4>().0 {
            pixel.swap(0, 2);
        }
    }
    rgba
}

#[test]
fn auto_srgb_roundtrip_palette_and_gold() {
    let Some((device, queue, adapter)) = headless_gpu() else {
        eprintln!("SKIP: no WGPU adapter available");
        return;
    };
    eprintln!(
        "adapter name={} backend={:?} device_type={:?}",
        adapter.name, adapter.backend, adapter.device_type
    );

    let _guard = crate::test_support::imgui_context_guard();

    let auto_srgb = render_swatches(
        &device,
        &queue,
        wgpu::TextureFormat::Bgra8UnormSrgb,
        GammaMode::Auto,
    );
    assert_eq!(auto_srgb, SWATCHES, "Auto+sRGB must preserve sRGB codes");

    let auto_unorm = render_swatches(
        &device,
        &queue,
        wgpu::TextureFormat::Rgba8Unorm,
        GammaMode::Auto,
    );
    assert_eq!(auto_unorm, SWATCHES, "Auto+Unorm must remain identity");

    let linear_srgb = render_swatches(
        &device,
        &queue,
        wgpu::TextureFormat::Bgra8UnormSrgb,
        GammaMode::Linear,
    );
    assert_eq!(
        linear_srgb[1],
        [53, 46, 46, 255],
        "Linear+sRGB must retain its existing retint"
    );
}
