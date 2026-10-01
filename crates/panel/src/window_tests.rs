use super::work_area::{
    cocoa_visible_frame_to_physical, fit_frame_to_work_area, win32_work_rect_to_physical, WorkArea,
};
use super::*;

/// Headless wgpu device/queue for the renderer-backed test. `None` when
/// no adapter exists (headless CI) — the texture test then skips.
fn headless_device() -> Option<(wgpu::Device, wgpu::Queue)> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter = block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        compatible_surface: None,
        force_fallback_adapter: false,
    }))
    .ok()?;
    let (device, queue) = block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("274 panel window test"),
        required_features: wgpu::Features::empty(),
        required_limits: wgpu::Limits::default(),
        experimental_features: wgpu::ExperimentalFeatures::default(),
        memory_hints: wgpu::MemoryHints::default(),
        trace: wgpu::Trace::default(),
    }))
    .ok()?;
    Some((device, queue))
}

/// The `Gpu` texture-id manager must delegate register/update/unregister
/// to the renderer's texture store (the surface `game_view` and later
/// screenshot tasks rely on).
#[test]
fn gpu_texture_register_update_unregister_delegates_to_renderer_store() {
    let Some((device, queue)) = headless_device() else {
        return; // no adapter: nothing to delegate to
    };
    let _guard = crate::test_support::imgui_context_guard();
    let mut context = imgui::Context::create();
    let mut renderer = imgui_wgpu::WgpuRenderer::new(
        imgui_wgpu::WgpuInitInfo::new(
            device.clone(),
            queue.clone(),
            wgpu::TextureFormat::Bgra8UnormSrgb,
        ),
        &mut context,
    )
    .expect("WgpuRenderer init on headless device");

    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("274 gpu test texture"),
        size: wgpu::Extent3d {
            width: 2,
            height: 2,
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

    let tex_id = {
        let mut gpu = Gpu::new(&device, &queue, &mut renderer);
        gpu.register_texture(&texture, &view)
    };
    assert!(!tex_id.is_null(), "register returns a live texture id");
    assert!(
        renderer.texture_manager().contains_texture(tex_id),
        "register lands in the renderer texture store"
    );

    let second_view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    let updated = {
        let mut gpu = Gpu::new(&device, &queue, &mut renderer);
        gpu.update_texture_view(tex_id, &second_view)
    };
    assert!(
        updated,
        "update_texture_view on a registered id reports true"
    );

    let missing = {
        let mut gpu = Gpu::new(&device, &queue, &mut renderer);
        gpu.update_texture_view(TextureId::from(0u64), &second_view)
    };
    assert!(!missing, "update on an unknown id reports false");

    {
        let mut gpu = Gpu::new(&device, &queue, &mut renderer);
        gpu.unregister_texture(tex_id);
    }
    assert!(
        !renderer.texture_manager().contains_texture(tex_id),
        "unregister removes the texture from the renderer store"
    );
}

/// Docking defaults: passthru central node, no-chrome host window.
/// `app::runner_config` overrides both.
#[test]
fn docking_config_defaults_passthru_no_chrome() {
    let d = DockingConfig::default();
    assert!(d.enable);
    assert!(d.auto_dockspace);
    assert!(d.dockspace_flags.contains(DockFlags::PASSTHRU_CENTRAL_NODE));
    assert!(d.host_window_flags.contains(WindowFlags::NO_TITLE_BAR));
    assert!(d.host_window_flags.contains(WindowFlags::NO_RESIZE));
    assert_eq!(d.host_window_name, "DockSpaceHost");
}

/// The shot readback normalizes `Bgra8*` captures to RGBA (the PNG
/// color order); `Rgba8*` bytes pass through untouched.
#[test]
fn to_rgba_swaps_bgra_rows_and_leaves_rgba() {
    let bgra = [0u8, 1, 2, 3, 10, 11, 12, 13];
    assert_eq!(
        to_rgba(&bgra, wgpu::TextureFormat::Bgra8Unorm),
        vec![2, 1, 0, 3, 12, 11, 10, 13]
    );
    assert_eq!(
        to_rgba(&bgra, wgpu::TextureFormat::Bgra8UnormSrgb),
        vec![2, 1, 0, 3, 12, 11, 10, 13]
    );
    assert_eq!(
        to_rgba(&bgra, wgpu::TextureFormat::Rgba8UnormSrgb),
        bgra.to_vec()
    );
}

/// Staging rows must be padded to the wgpu copy alignment.
#[test]
fn align_up_pads_to_copy_bytes_per_row() {
    assert_eq!(align_up(4, 256), 256);
    assert_eq!(align_up(256, 256), 256);
    assert_eq!(align_up(257, 256), 512);
    assert_eq!(align_up(0, 256), 0);
}

#[test]
fn shot_state_reports_each_owned_capture_stage() {
    let mut shots = ShotState::default();
    assert_eq!(shots.status("gnome_chop"), ShotStatus::Missing);

    shots.enqueue("gnome_chop".into(), "{\"scene\":2}".into());
    assert_eq!(shots.status("gnome_chop"), ShotStatus::Requested);

    assert_eq!(shots.promote_requests(), 1);
    assert_eq!(shots.status("gnome_chop"), ShotStatus::ReadbackPending);

    shots.wanted.clear();
    shots.done.push(ShotCapture {
        label: "gnome_chop".into(),
        snapshot_json: "{\"scene\":2}".into(),
        width: 1,
        height: 1,
        rgba: vec![0, 0, 0, 255],
        #[cfg(feature = "render-diagnostics")]
        pixel_roi: None,
    });
    assert_eq!(shots.status("gnome_chop"), ShotStatus::WritePending);

    shots.done.clear();
    shots.mark_written("gnome_chop");
    assert_eq!(shots.status("gnome_chop"), ShotStatus::Written);

    shots.mark_failed("other", "readback did not complete");
    assert_eq!(
        shots.status("other"),
        ShotStatus::Failed("readback did not complete".into())
    );
}

#[test]
fn promote_ready_keeps_unready_and_second_actor_queued() {
    let mut shots = ShotState::default();
    shots.enqueue_for_actor(
        "air-alice".into(),
        "{\"actor\":\"alice\"}".into(),
        "alice".into(),
    );
    shots.enqueue_for_actor("air-bob".into(), "{\"actor\":\"bob\"}".into(), "bob".into());

    assert_eq!(shots.promote_ready(Some("alice"), None), 0);
    assert_eq!(
        shots.requests.len(),
        2,
        "unready selected buffer is not captured"
    );
    assert!(shots.wanted.is_empty());

    assert_eq!(shots.promote_ready(Some("alice"), Some("bob")), 0);
    assert_eq!(
        shots.requests.len(),
        2,
        "a presented buffer that is not the focused actor is not associated"
    );

    assert_eq!(shots.promote_ready(Some("alice"), Some("alice")), 1);
    assert_eq!(shots.wanted.len(), 1);
    assert_eq!(shots.wanted[0].0, "air-alice");
    assert_eq!(shots.wanted[0].1, "{\"actor\":\"alice\"}");
    assert_eq!(shots.requests.len(), 1);
    assert_eq!(shots.requests[0].actor.as_deref(), Some("bob"));
    assert_eq!(shots.status("air-alice"), ShotStatus::ReadbackPending);
    assert_eq!(shots.status("air-bob"), ShotStatus::Requested);

    shots.wanted.clear();
    shots.mark_written("air-alice");
    assert_eq!(shots.promote_ready(Some("bob"), Some("bob")), 1);
    assert_eq!(shots.wanted[0].0, "air-bob");
    assert!(shots.requests.is_empty());
}

#[test]
fn fail_labels_drops_queued_actors_and_keeps_the_error() {
    let mut shots = ShotState::default();
    shots.enqueue_for_actor("air-alice".into(), "{}".into(), "alice".into());
    shots.enqueue_for_actor("air-bob".into(), "{}".into(), "bob".into());
    shots.promote_ready(Some("alice"), Some("alice"));
    shots.fail_labels(
        &["air-alice".into(), "air-bob".into()],
        "terminal shot was not written within 10s",
    );
    assert!(shots.requests.is_empty());
    assert!(shots.wanted.is_empty());
    assert_eq!(
        shots.status("air-alice"),
        ShotStatus::Failed("terminal shot was not written within 10s".into())
    );
    assert_eq!(
        shots.status("air-bob"),
        ShotStatus::Failed("terminal shot was not written within 10s".into())
    );
}

#[test]
fn missing_mapped_readback_is_retained_as_a_capture_failure() {
    let mut shots = ShotState::default();
    record_readback_outcomes(&mut shots, &["gnome_chop".into()], Vec::new());
    assert_eq!(
        shots.status("gnome_chop"),
        ShotStatus::Failed("readback did not complete".into())
    );
}

#[test]
fn surface_skip_diagnostic_names_only_readback_pending_captures() {
    let shots = Mutex::new(ShotState::default());
    shots
        .lock()
        .unwrap()
        .enqueue("still-requested".into(), "{}".into());
    assert_eq!(deferred_readback_message(&shots, "occluded"), None);

    {
        let mut shots = shots.lock().unwrap();
        shots.promote_requests();
        shots.enqueue("not-promoted".into(), "{}".into());
    }
    assert_eq!(
        deferred_readback_message(&shots, "occluded").as_deref(),
        Some("[panel] capture readback deferred by surface occluded: still-requested")
    );
}

const OCCLUDED_TEST_PX: u32 = 64;
const MAGENTA: [u8; 4] = [255, 0, 255, 255];
const RED: [u8; 4] = [255, 0, 0, 255];
const GREEN: [u8; 4] = [0, 255, 0, 255];
const GREEN_CLEAR: wgpu::Color = wgpu::Color {
    r: 0.0,
    g: 1.0,
    b: 0.0,
    a: 1.0,
};
/// The known imgui primitive in the capture regressions: one filled rect
/// over capture pixels `(8, 8)..(40, 40)` in fully saturated red. The
/// saturated 0.0/1.0 components survive the imgui gamma path unchanged
/// (it is the identity at both ends), so the expected bytes hold for a
/// linear and an sRGB render target alike — which is why the assertions
/// stay on this primitive and the clear color, not on antialiased edges,
/// the window background, or other mid-tone pixels.
const RECT_PX: (u32, u32, u32, u32) = (8, 8, 40, 40);
const RECT_FILL: [f32; 4] = [1.0, 0.0, 0.0, 1.0];

fn test_texture(
    device: &wgpu::Device,
    format: wgpu::TextureFormat,
    usage: wgpu::TextureUsages,
) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("274 panel occluded capture test"),
        size: wgpu::Extent3d {
            width: OCCLUDED_TEST_PX,
            height: OCCLUDED_TEST_PX,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage,
        view_formats: &[],
    })
}

fn fill_solid(queue: &wgpu::Queue, texture: &wgpu::Texture, px: [u8; 4]) {
    let width = texture.width();
    let height = texture.height();
    let mut data = vec![0u8; (width * height * 4) as usize];
    for chunk in data.as_chunks_mut::<4>().0 {
        chunk.copy_from_slice(&px);
    }
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        &data,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(width * 4),
            rows_per_image: Some(height),
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
}

fn read_texture_rgba(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    format: wgpu::TextureFormat,
) -> Vec<u8> {
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("274 panel occluded capture probe"),
    });
    let copies = readback(
        device,
        texture,
        &mut encoder,
        &[("probe".into(), String::new())],
    );
    queue.submit(Some(encoder.finish()));
    map_readbacks(device, format, copies)
        .into_iter()
        .next()
        .expect("probe readback")
        .rgba
}

/// A real imgui renderer on the headless adapter. Only a missing
/// adapter may skip a GPU assertion; a renderer that cannot initialize
/// on an adapter that does exist is a failure, not a silent pass.
fn headless_capture_renderer(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    format: wgpu::TextureFormat,
) -> (imgui::Context, imgui_wgpu::WgpuRenderer) {
    let mut context = imgui::Context::create();
    context
        .io_mut()
        .set_display_size([OCCLUDED_TEST_PX as f32, OCCLUDED_TEST_PX as f32]);
    let renderer = imgui_wgpu::WgpuRenderer::new(
        imgui_wgpu::WgpuInitInfo::new(device.clone(), queue.clone(), format),
        &mut context,
    )
    .expect("WgpuRenderer init on a device whose adapter exists");
    (context, renderer)
}

fn pixel(rgba: &[u8], width: u32, x: u32, y: u32) -> [u8; 4] {
    let start = ((y * width + x) * 4) as usize;
    rgba[start..start + 4].try_into().expect("4-byte pixel")
}

/// One imgui frame whose only primitive is the known [`RECT_PX`] rect.
/// It is drawn on the foreground list, so its coordinates are capture
/// pixels with no window offset, and the frame is a real composition
/// (not just the clear color) for the readback to prove.
fn draw_known_rect(context: &mut imgui::Context) -> &mut imgui::DrawData {
    let (x0, y0, x1, y1) = RECT_PX;
    {
        let ui = context.frame();
        ui.get_foreground_draw_list()
            .add_rect([x0 as f32, y0 as f32], [x1 as f32, y1 as f32], RECT_FILL)
            .filled(true)
            .build();
    }
    context.render()
}

/// The panel's production surface-config shape at the capture test size,
/// so an occluded one-off target is sized and formatted like the real
/// one.
fn test_surface_desc(format: wgpu::TextureFormat) -> wgpu::SurfaceConfiguration {
    wgpu::SurfaceConfiguration {
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        format,
        width: OCCLUDED_TEST_PX,
        height: OCCLUDED_TEST_PX,
        present_mode: wgpu::PresentMode::Fifo,
        alpha_mode: wgpu::CompositeAlphaMode::Auto,
        view_formats: vec![],
        desired_maximum_frame_latency: 2,
    }
}

/// A promoted, ready-for-readback `flax_aio` job.
fn promoted_flax_shots() -> Mutex<ShotState> {
    let shots = Mutex::new(ShotState::default());
    {
        let mut shots = shots.lock().unwrap();
        shots.enqueue("flax_aio".into(), "{\"scene\":2}".into());
        assert_eq!(shots.promote_requests(), 1);
    }
    shots
}

/// The production occluded route. `AcquiredFrame::Occluded` with a
/// promoted job and no persistent offscreen (a surface that is its own
/// capture source) must allocate the one-off target, rasterize this
/// frame's composition into it, and complete the capture — without a
/// blit or a present. That is the shape the panel's flax_aio FAIL ran:
/// the original early return (and the old helper, which mapped before
/// returning and could only draw into whatever target its caller had
/// already built) leaves `done` empty, so this fails on the pre-fix
/// behavior. The same call with a persistent offscreen must reuse it and
/// overwrite the stale prefill instead of allocating.
#[test]
fn occluded_acquisition_captures_composed_frame_without_present() {
    let Some((device, queue)) = headless_device() else {
        return; // no adapter: no GPU path to assert
    };
    let _guard = crate::test_support::imgui_context_guard();
    let format = wgpu::TextureFormat::Rgba8Unorm;
    let (mut context, mut renderer) = headless_capture_renderer(&device, &queue, format);
    let desc = test_surface_desc(format);
    let (x0, y0, x1, y1) = RECT_PX;

    let shots = promoted_flax_shots();
    let draw_data = draw_known_rect(&mut context);
    assert!(
        draw_data.total_vtx_count() > 0,
        "the regression needs a nonempty imgui draw primitive"
    );
    let submission = submit_acquired_frame(
        &device,
        &queue,
        &mut renderer,
        draw_data,
        GREEN_CLEAR,
        &desc,
        None,
        AcquiredFrame::Occluded,
        &shots,
    )
    .expect("occluded acquisition submit");
    let FrameSubmission::Submitted(readbacks) = submission else {
        panic!("a promoted occluded capture must submit, not skip");
    };
    assert_eq!(readbacks.len(), 1, "one staging readback per promoted shot");
    assert!(
        shots.lock().unwrap().done.is_empty(),
        "the capture completes after submission, not inside it"
    );
    complete_readbacks(&device, format, readbacks, &shots);

    {
        let shots = shots.lock().unwrap();
        assert_eq!(shots.status("flax_aio"), ShotStatus::WritePending);
        assert_eq!(shots.done.len(), 1);
        let capture = &shots.done[0];
        assert_eq!(capture.label, "flax_aio");
        assert_eq!(capture.snapshot_json, "{\"scene\":2}");
        assert_eq!(
            (capture.width, capture.height),
            (OCCLUDED_TEST_PX, OCCLUDED_TEST_PX),
            "the one-off target takes the surface size"
        );
        assert_eq!(
            pixel(&capture.rgba, capture.width, x0 + 4, y0 + 4),
            RED,
            "the imgui primitive is composited into the captured frame"
        );
        assert_eq!(
            pixel(&capture.rgba, capture.width, x1 + 8, y1 + 8),
            GREEN,
            "outside the primitive the clear color still shows"
        );
    }

    // Persistent offscreen: the same occluded call reuses that target
    // (no second framebuffer) and its stale prefill is gone. Occluded
    // frames carry no image, so nothing is blitted or presented either
    // way; the visible regression below is what writes an image.
    let persistent = test_texture(
        &device,
        format,
        wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::COPY_SRC
            | wgpu::TextureUsages::COPY_DST,
    );
    fill_solid(&queue, &persistent, MAGENTA);
    let shots = promoted_flax_shots();
    let draw_data = draw_known_rect(&mut context);
    let submission = submit_acquired_frame(
        &device,
        &queue,
        &mut renderer,
        draw_data,
        GREEN_CLEAR,
        &desc,
        Some(&persistent),
        AcquiredFrame::Occluded,
        &shots,
    )
    .expect("occluded acquisition submit onto the persistent offscreen");
    let FrameSubmission::Submitted(readbacks) = submission else {
        panic!("a promoted occluded capture must submit, not skip");
    };
    complete_readbacks(&device, format, readbacks, &shots);

    {
        let shots = shots.lock().unwrap();
        assert_eq!(shots.done.len(), 1);
        assert_eq!(
            pixel(&shots.done[0].rgba, OCCLUDED_TEST_PX, x0 + 4, y0 + 4),
            RED
        );
        assert_eq!(
            pixel(&shots.done[0].rgba, OCCLUDED_TEST_PX, x1 + 8, y1 + 8),
            GREEN
        );
    }
    let reused = read_texture_rgba(&device, &queue, &persistent, format);
    assert!(
        !reused.as_chunks::<4>().0.iter().any(|px| px == &MAGENTA),
        "the reused target holds this frame, not the stale prefill"
    );
}

/// Occluded with nothing promoted: the production route must allocate no
/// target, render nothing, and stage no readback — it returns `Skipped`
/// before any GPU work, so a still-queued actor job keeps its request.
/// The old path rendered (and cleared) its target first and only then
/// found `wanted` empty, which both checks below catch: the prefilled
/// target would be overwritten, and the out-of-range surface config
/// would raise a validation error from the one-off allocation.
#[test]
fn occluded_acquisition_without_promoted_wanted_records_no_gpu_work() {
    let Some((device, queue)) = headless_device() else {
        return;
    };
    let _guard = crate::test_support::imgui_context_guard();
    let format = wgpu::TextureFormat::Rgba8Unorm;
    let (mut context, mut renderer) = headless_capture_renderer(&device, &queue, format);

    let target = test_texture(
        &device,
        format,
        wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::COPY_SRC
            | wgpu::TextureUsages::COPY_DST,
    );
    fill_solid(&queue, &target, MAGENTA);

    // Beyond every adapter's max texture dimension: if the skip path
    // built the one-off target from this config, the error scope sees it.
    let mut desc = test_surface_desc(format);
    desc.width = u32::MAX;
    desc.height = u32::MAX;

    let shots = Mutex::new(ShotState::default());
    shots.lock().unwrap().enqueue_for_actor(
        "flax_aio".into(),
        "{\"scene\":1}".into(),
        "bot".into(),
    );

    let draw_data = draw_known_rect(&mut context);
    let allocation_scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let submission = submit_acquired_frame(
        &device,
        &queue,
        &mut renderer,
        draw_data,
        GREEN_CLEAR,
        &desc,
        None,
        AcquiredFrame::Occluded,
        &shots,
    )
    .expect("an occluded frame with nothing promoted is not an error");
    let allocation_error = block_on(allocation_scope.pop());
    assert!(
        matches!(submission, FrameSubmission::Skipped),
        "an occluded frame with nothing promoted records no GPU work"
    );
    assert!(
        allocation_error.is_none(),
        "the skipped frame must not allocate a capture target: {allocation_error:?}"
    );

    let draw_data = draw_known_rect(&mut context);
    let submission = submit_acquired_frame(
        &device,
        &queue,
        &mut renderer,
        draw_data,
        GREEN_CLEAR,
        &desc,
        Some(&target),
        AcquiredFrame::Occluded,
        &shots,
    )
    .expect("an occluded frame with nothing promoted is not an error");
    assert!(matches!(submission, FrameSubmission::Skipped));
    let prefilled = read_texture_rgba(&device, &queue, &target, format);
    assert!(
        prefilled.as_chunks::<4>().0.iter().all(|px| px == &MAGENTA),
        "no render pass ran over the caller's target"
    );

    let shots = shots.lock().unwrap();
    assert_eq!(shots.status("flax_aio"), ShotStatus::Requested);
    assert!(shots.wanted.is_empty());
    assert!(shots.done.is_empty());
    assert_eq!(shots.requests.len(), 1);
}

/// The visible path through the same production entry point: a
/// presentable image receives the blit of the freshly rendered offscreen
/// and the capture completes from that same target, with the blocking
/// map left outside the submit call (AppWindow presents in between,
/// which is the ordering this guards). A surface that is its own capture
/// source draws into the acquired image directly, with no blit.
#[test]
fn visible_acquisition_captures_composed_frame_and_writes_the_image() {
    let Some((device, queue)) = headless_device() else {
        return;
    };
    let _guard = crate::test_support::imgui_context_guard();
    let format = wgpu::TextureFormat::Rgba8Unorm;
    let (mut context, mut renderer) = headless_capture_renderer(&device, &queue, format);
    let desc = test_surface_desc(format);
    let (x0, y0, x1, y1) = RECT_PX;

    let offscreen = test_texture(
        &device,
        format,
        wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::COPY_SRC
            | wgpu::TextureUsages::COPY_DST,
    );
    let image = test_texture(
        &device,
        format,
        wgpu::TextureUsages::COPY_SRC | wgpu::TextureUsages::COPY_DST,
    );
    fill_solid(&queue, &offscreen, MAGENTA);
    fill_solid(&queue, &image, RED);

    let shots = promoted_flax_shots();
    let draw_data = draw_known_rect(&mut context);
    let submission = submit_acquired_frame(
        &device,
        &queue,
        &mut renderer,
        draw_data,
        GREEN_CLEAR,
        &desc,
        Some(&offscreen),
        AcquiredFrame::Presentable(&image),
        &shots,
    )
    .expect("visible acquisition submit");
    let FrameSubmission::Submitted(readbacks) = submission else {
        panic!("a visible frame always submits");
    };
    assert!(
        shots.lock().unwrap().done.is_empty(),
        "the caller completes the readbacks after presenting"
    );
    complete_readbacks(&device, format, readbacks, &shots);

    {
        let shots = shots.lock().unwrap();
        assert_eq!(shots.status("flax_aio"), ShotStatus::WritePending);
        assert_eq!(shots.done.len(), 1);
        assert_eq!(
            pixel(&shots.done[0].rgba, OCCLUDED_TEST_PX, x0 + 4, y0 + 4),
            RED
        );
        assert_eq!(
            pixel(&shots.done[0].rgba, OCCLUDED_TEST_PX, x1 + 8, y1 + 8),
            GREEN
        );
    }

    let presented = read_texture_rgba(&device, &queue, &image, format);
    assert_eq!(
        pixel(&presented, OCCLUDED_TEST_PX, x0 + 4, y0 + 4),
        RED,
        "the presentable image receives the composed frame's blit"
    );
    assert_eq!(pixel(&presented, OCCLUDED_TEST_PX, x1 + 8, y1 + 8), GREEN);

    // No persistent offscreen: the acquired image is the render target
    // and the capture source, so the frame lands on it without a blit.
    let direct = test_texture(
        &device,
        format,
        wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::COPY_SRC
            | wgpu::TextureUsages::COPY_DST,
    );
    fill_solid(&queue, &direct, MAGENTA);
    let shots = promoted_flax_shots();
    let draw_data = draw_known_rect(&mut context);
    let submission = submit_acquired_frame(
        &device,
        &queue,
        &mut renderer,
        draw_data,
        GREEN_CLEAR,
        &desc,
        None,
        AcquiredFrame::Presentable(&direct),
        &shots,
    )
    .expect("visible acquisition submit onto the image");
    let FrameSubmission::Submitted(readbacks) = submission else {
        panic!("a visible frame always submits");
    };
    complete_readbacks(&device, format, readbacks, &shots);

    let drawn = read_texture_rgba(&device, &queue, &direct, format);
    assert_eq!(pixel(&drawn, OCCLUDED_TEST_PX, x0 + 4, y0 + 4), RED);
    assert_eq!(pixel(&drawn, OCCLUDED_TEST_PX, x1 + 8, y1 + 8), GREEN);
    assert!(
        !drawn.as_chunks::<4>().0.iter().any(|px| px == &MAGENTA),
        "the acquired image holds this frame, not the stale prefill"
    );
}

/// The only embedded base font must cover every non-ASCII string literal
/// in panel sources, including the file-dialog Nerd Font icons.
#[test]
fn panel_font_covers_non_ascii_source_string_literals() {
    let _guard = crate::test_support::imgui_context_guard();
    let (mut ctx, required_glyphs_present) = panel_font_test_context();
    let source_glyphs = panel_non_ascii_string_glyphs();
    let missing = {
        let ui = ctx.frame();
        let font = ui.current_font();
        source_glyphs
            .into_iter()
            .filter(|(_, glyph)| !font.is_glyph_in_font(*glyph))
            .collect::<Vec<_>>()
    };
    ctx.render();

    let missing_text = missing
        .iter()
        .map(|(path, glyph)| format!("{} U+{:04X}", path.display(), *glyph as u32))
        .collect::<Vec<_>>()
        .join(", ");
    assert!(
        missing.is_empty(),
        "panel source string glyphs missing from the atlas: {missing_text}"
    );
    assert!(
        required_glyphs_present,
        "rail controls and Nerd Font icons must be in the atlas"
    );
}

/// The 3270 cell advance at [`PANEL_FONT_SIZE`] stays within 5% of
/// ProggyClean's old 7 px advance at 13 px, natively (no minimum-advance
/// padding), so the panel's existing horizontal layout still fits.
#[test]
fn panel_font_advance_stays_within_five_percent_of_old_layout() {
    let _guard = crate::test_support::imgui_context_guard();
    let (mut ctx, glyphs_present) = panel_font_test_context();
    assert!(glyphs_present, "panel's required glyphs are in the atlas");

    let (advance, size, line_height) = {
        let ui = ctx.frame();
        let size = ui.current_font_size();
        let line_height = ui.text_line_height();
        let advance = ui
            .current_font()
            .calc_text_size(size, f32::MAX, 0.0, "00000000")[0]
            / 8.0;
        (advance, size, line_height)
    };
    ctx.render();
    assert!(
        (size - PANEL_FONT_SIZE).abs() <= 0.01,
        "font size at 1× must stay {PANEL_FONT_SIZE} px, got {size:.3}"
    );
    assert!(
        (line_height - 14.0).abs() <= 0.01,
        "ImGui line height at the selected size must be 14 px, got {line_height:.3}"
    );
    assert!(
        (advance - 7.0).abs() <= 0.35,
        "3270 cell advance {advance:.3} at size {size:.3} must stay within 5% of ProggyClean's 7 px baseline"
    );
}

fn panel_font_test_context() -> (imgui::Context, bool) {
    let mut ctx = imgui::Context::create();
    let required_glyphs_present = add_panel_font(&mut ctx);
    let _ = ctx.font_atlas_mut().build();
    ctx.io_mut().set_display_size([128.0, 128.0]);
    ctx.io_mut().set_delta_time(1.0 / 60.0);
    (ctx, required_glyphs_present)
}

fn panel_non_ascii_string_glyphs() -> Vec<(std::path::PathBuf, char)> {
    let source_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    collect_panel_rust_files(&source_root, &mut files);
    files.sort();

    let mut glyphs = std::collections::BTreeSet::new();
    for path in files {
        let source = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("reading {}: {error}", path.display()));
        let tokens = source
            .parse::<proc_macro2::TokenStream>()
            .unwrap_or_else(|error| panic!("tokenizing {}: {error}", path.display()));
        let mut literals = Vec::new();
        collect_string_literals(tokens, &mut literals);
        for literal in literals {
            for glyph in literal.chars().filter(|glyph| !glyph.is_ascii()) {
                glyphs.insert((path.clone(), glyph));
            }
        }
    }
    glyphs.into_iter().collect()
}

fn collect_panel_rust_files(directory: &std::path::Path, files: &mut Vec<std::path::PathBuf>) {
    for entry in std::fs::read_dir(directory)
        .unwrap_or_else(|error| panic!("reading {}: {error}", directory.display()))
    {
        let path = entry
            .unwrap_or_else(|error| panic!("reading {} entry: {error}", directory.display()))
            .path();
        if path.is_dir() {
            collect_panel_rust_files(&path, files);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            files.push(path);
        }
    }
}

fn collect_string_literals(tokens: proc_macro2::TokenStream, literals: &mut Vec<String>) {
    for token in tokens {
        match token {
            proc_macro2::TokenTree::Group(group) => {
                collect_string_literals(group.stream(), literals)
            }
            proc_macro2::TokenTree::Literal(literal) => {
                if let Ok(string) = syn::parse_str::<syn::LitStr>(&literal.to_string()) {
                    literals.push(string.value());
                }
            }
            proc_macro2::TokenTree::Ident(_) | proc_macro2::TokenTree::Punct(_) => {}
        }
    }
}

/// M-003: an oversized request clamps to the work area. The default
/// 1120x580 panel is the launch size; nothing in this crate saves OS
/// window geometry.
#[test]
fn oversized_request_clamps_to_the_work_area() {
    assert_eq!(
        clamp_inner_size((1920.0, 1080.0), (1366.0, 768.0)),
        (1366.0, 768.0),
        "a request bigger than the work area must shrink to fit it"
    );
}

/// M-003: a fitting launch size passes through untouched. The default
/// 1120x580 panel on a 1366x768 work area must not move a pixel.
#[test]
fn fitting_launch_geometry_is_untouched_by_the_clamp() {
    assert_eq!(
        clamp_inner_size((1120.0, 580.0), (1366.0, 768.0)),
        (1120.0, 580.0),
        "a window that already fits must keep its exact size"
    );
}

/// M-003: the rail-open need (1120 + 264 = 1384 wide) on a 1366-wide
/// work area clamps the width while the height survives.
#[test]
fn rail_open_overflow_clamps_width_but_keeps_height() {
    assert_eq!(
        clamp_inner_size((1384.0, 580.0), (1366.0, 768.0)),
        (1366.0, 580.0),
        "only the overflowing dimension may shrink"
    );
}

/// M-003: an off-screen position pulls back inside the work area, and a
/// frame larger than the area pins to the origin so the title bar and
/// close box stay reachable.
#[test]
fn off_screen_position_pulls_back_inside_the_work_area() {
    // Off the right/bottom (second monitor unplugged): 1366-1120=246,
    // 768-580=188 is the furthest on-screen top-left.
    assert_eq!(
        clamp_outer_position(
            (3000.0, 500.0),
            (1120.0, 580.0),
            (0.0, 0.0),
            (1366.0, 768.0)
        ),
        (246.0, 188.0),
        "a stranded frame must land fully on-screen"
    );
    // Off the top-left.
    assert_eq!(
        clamp_outer_position(
            (-100.0, -50.0),
            (1120.0, 580.0),
            (0.0, 0.0),
            (1366.0, 768.0)
        ),
        (0.0, 0.0),
        "a negative position must settle at the origin"
    );
    // Already on-screen: untouched.
    assert_eq!(
        clamp_outer_position((100.0, 100.0), (1120.0, 580.0), (0.0, 0.0), (1366.0, 768.0)),
        (100.0, 100.0),
        "an on-screen position must not move"
    );
    // Oversized frame pins to the origin, not to a negative offset.
    assert_eq!(
        clamp_outer_position(
            (100.0, 100.0),
            (2000.0, 1000.0),
            (0.0, 0.0),
            (1366.0, 768.0)
        ),
        (0.0, 0.0),
        "a frame bigger than the area must pin its top-left to the origin"
    );
    // Secondary monitor offset origin is honored.
    assert_eq!(
        clamp_outer_position(
            (1400.0, 100.0),
            (800.0, 600.0),
            (1366.0, 0.0),
            (1920.0, 1080.0)
        ),
        (1400.0, 100.0),
        "an on-screen position on a secondary monitor must not move"
    );
}

/// M-003: degenerate inputs never collapse the window or panic. Zero and
/// negative sizes floor at 1 px, a bogus (non-positive) work area keeps
/// the request, and NaN degrades to a finite bound.
#[test]
fn clamp_never_collapses_or_panics_on_degenerate_input() {
    assert_eq!(
        clamp_inner_size((0.0, -4.0), (1366.0, 768.0)),
        (1.0, 1.0),
        "non-positive requests must floor, never vanish"
    );
    assert_eq!(
        clamp_inner_size((1120.0, 580.0), (0.0, 0.0)),
        (1120.0, 580.0),
        "a bogus work area must keep the request, never zero it"
    );
    assert_eq!(
        clamp_inner_size((f64::NAN, 580.0), (1366.0, 768.0)),
        (1366.0, 580.0),
        "NaN width must degrade to the bound, never panic"
    );
    let (x, y) = clamp_outer_position(
        (f64::NAN, f64::NAN),
        (100.0, 100.0),
        (0.0, 0.0),
        (1366.0, 768.0),
    );
    assert!(
        (0.0..=1266.0).contains(&x) && (0.0..=668.0).contains(&y),
        "NaN position must settle inside the area, got ({x}, {y})"
    );
}

/// M-003 glue: a centered 1120-wide launch grown to the 1384 rail need on a
/// 1366-wide work area must shrink *and* move. Size-only clamp leaves the
/// origin at 123 and hangs 123 px off the right edge — the BreakMac failure.
/// These helpers did not exist at `a110ce92b`; the old rail path never
/// called `clamp_outer_position`.
#[test]
fn rail_grow_on_centered_launch_repositions_onto_the_work_area() {
    let work = WorkArea {
        origin: (0.0, 0.0),
        size: (1366.0, 768.0),
    };
    let fit = fit_frame_to_work_area(
        work,
        (123.0, 94.0),
        (1120.0, 580.0),
        (1120.0, 580.0),
        (1384.0, 580.0),
    );
    assert_eq!(
        fit.inner_size,
        (1366.0, 580.0),
        "rail grow must shrink to the work-area width"
    );
    assert_eq!(
        fit.outer_position,
        (0.0, 94.0),
        "rail grow must slide the centered origin to the work-area left, not keep x=123"
    );
}

/// M-003 glue: a menu-bar inset is a work area whose origin is *below* the
/// monitor origin. An oversized frame must pin to that origin, not (0, 0)
/// (which puts the title bar under the menu bar).
#[test]
fn menu_bar_inset_pins_oversized_frame_to_the_work_origin() {
    let work = WorkArea {
        origin: (0.0, 34.0),
        size: (1366.0, 734.0),
    };
    let fit = fit_frame_to_work_area(
        work,
        (100.0, 0.0),
        (2000.0, 1000.0),
        (2000.0, 972.0),
        (2000.0, 972.0),
    );
    assert_eq!(
        fit.outer_position,
        (0.0, 34.0),
        "title bar must land on the work-area origin, not behind the menu bar"
    );
}

/// M-003 glue: a bottom taskbar shrinks the work-area height. A frame that
/// would sit on the taskbar is pulled up.
#[test]
fn bottom_taskbar_pulls_a_frame_up_into_the_work_area() {
    let work = WorkArea {
        origin: (0.0, 0.0),
        size: (1366.0, 728.0),
    };
    let fit = fit_frame_to_work_area(
        work,
        (100.0, 200.0),
        (800.0, 600.0),
        (800.0, 572.0),
        (800.0, 572.0),
    );
    assert_eq!(
        fit.outer_position,
        (100.0, 128.0),
        "200+600 overshoots a 728-tall work area; y must be 728-600=128"
    );
}

/// M-003 glue: a left taskbar (or left Dock) shifts the work origin. A
/// frame at x=0, or an oversized frame, pins to that origin, not the
/// monitor's 0.
#[test]
fn left_taskbar_pins_to_the_work_origin_not_the_monitor_origin() {
    let work = WorkArea {
        origin: (48.0, 0.0),
        size: (1318.0, 768.0),
    };
    let shifted = fit_frame_to_work_area(
        work,
        (0.0, 50.0),
        (800.0, 600.0),
        (800.0, 572.0),
        (800.0, 572.0),
    );
    assert_eq!(
        shifted.outer_position,
        (48.0, 50.0),
        "a frame over the left taskbar must slide to the work origin"
    );
    let oversized = fit_frame_to_work_area(
        work,
        (100.0, 50.0),
        (2000.0, 600.0),
        (2000.0, 572.0),
        (2000.0, 572.0),
    );
    assert_eq!(
        oversized.outer_position,
        (48.0, 50.0),
        "an oversized frame must pin x to the work origin, not 0"
    );
}

/// M-003 glue: a secondary monitor with a negative origin is a real
/// layout. The clamp must pin to that origin, not jump to 0.
#[test]
fn negative_origin_secondary_monitor_stays_on_that_display() {
    let work = WorkArea {
        origin: (-1920.0, 0.0),
        size: (1920.0, 1080.0),
    };
    let fit = fit_frame_to_work_area(
        work,
        (-2000.0, 40.0),
        (800.0, 600.0),
        (800.0, 572.0),
        (800.0, 572.0),
    );
    assert_eq!(
        fit.outer_position,
        (-1920.0, 40.0),
        "a frame left of a negative-origin display must pin to -1920, not 0"
    );
}

/// M-003 glue: scale 2.0 is physical = logical × 2. A 34 pt menu bar is 68
/// physical px; the 1384 logical rail need is 2768 physical and must clamp
/// to the 2732-wide work area and move the centered origin.
#[test]
fn scale_2_work_area_converts_logical_insets_and_repositions_rail_grow() {
    let work = cocoa_visible_frame_to_physical((0.0, 0.0), (1366.0, 734.0), 768.0, 2.0);
    assert_eq!(
        work,
        WorkArea {
            origin: (0.0, 68.0),
            size: (2732.0, 1468.0),
        },
        "visibleFrame under a 34 pt menu bar at scale 2 is origin y=68, not the full monitor"
    );
    let fit = fit_frame_to_work_area(
        work,
        (246.0, 188.0),
        (2240.0, 1216.0),
        (2240.0, 1160.0),
        (2768.0, 1160.0),
    );
    assert_eq!(
        fit.inner_size,
        (2732.0, 1160.0),
        "physical rail width must clamp to the scaled work-area width"
    );
    assert_eq!(
        fit.outer_position,
        (0.0, 188.0),
        "centered origin 246 must slide to the scaled work-area left"
    );
}

/// M-003 glue: position clamp uses the outer size, so a title-bar chrome
/// of 28 px at the bottom of a menu-bar-inset area does not hang the frame
/// past the work area. Using the inner size would leave y 28 px too low.
#[test]
fn position_clamp_uses_outer_size_not_inner() {
    let work = WorkArea {
        origin: (0.0, 34.0),
        size: (1366.0, 734.0),
    };
    let fit = fit_frame_to_work_area(
        work,
        (100.0, 200.0),
        (1120.0, 608.0),
        (1120.0, 580.0),
        (1120.0, 580.0),
    );
    assert_eq!(
        fit.outer_position,
        (100.0, 160.0),
        "outer 608 against work bottom 768 must land at y=160, not inner-based y=188"
    );
}

/// M-003 provider: AppKit visibleFrame (bottom-left points) becomes winit
/// physical top-left, including a left Dock + menu bar.
#[test]
fn cocoa_visible_frame_flips_and_scales_into_winit_space() {
    assert_eq!(
        cocoa_visible_frame_to_physical((0.0, 0.0), (1512.0, 948.0), 982.0, 2.0),
        WorkArea {
            origin: (0.0, 68.0),
            size: (3024.0, 1896.0),
        },
        "1512x948 visibleFrame on a 982-tall main display at scale 2 is a 34 pt (68 px) top inset"
    );
    assert_eq!(
        cocoa_visible_frame_to_physical((80.0, 0.0), (1286.0, 734.0), 768.0, 1.0),
        WorkArea {
            origin: (80.0, 34.0),
            size: (1286.0, 734.0),
        },
        "a left Dock plus menu bar must keep both insets after the y-flip"
    );
}

/// M-003 provider: Win32 `rcWork` is already top-left physical. Passing the
/// taskbar-subtracted rect (not `rcMonitor`) is what the Windows FFI does.
#[test]
fn win32_rc_work_keeps_taskbar_insets() {
    assert_eq!(
        win32_work_rect_to_physical(0, 0, 1920, 1040),
        WorkArea {
            origin: (0.0, 0.0),
            size: (1920.0, 1040.0),
        },
        "a bottom taskbar rcWork is shorter than rcMonitor"
    );
    assert_eq!(
        win32_work_rect_to_physical(40, 0, 1920, 1080),
        WorkArea {
            origin: (40.0, 0.0),
            size: (1880.0, 1080.0),
        },
        "a left taskbar rcWork origin is not the monitor origin"
    );
    assert_eq!(
        win32_work_rect_to_physical(-1920, 0, 0, 1080),
        WorkArea {
            origin: (-1920.0, 0.0),
            size: (1920.0, 1080.0),
        },
        "a secondary monitor's rcWork keeps the negative origin"
    );
}

/// M-003 launch fit on the Windows 1366x768 guest (48 px taskbar):
/// `CW_USEDEFAULT` put the frame's bottom at 775, under the taskbar. The
/// launch fit asks for the window's own size, so it only moves the frame up
/// to the `rcWork` bottom; it must not resize it.
#[test]
fn launch_fit_moves_a_frame_off_the_taskbar_without_resizing() {
    let frame = crate::test_support::FakeFrame::new(1.0, (16, 39), (1120, 580), (156, 156));
    let work = win32_work_rect_to_physical(0, 0, 1366, 720);
    fit_window_in(&frame, Some(work), (1120.0, 580.0));
    assert_eq!(frame.outer_rect(), (156, 101, 1292, 720));
    assert_eq!(
        frame.resizes(),
        0,
        "a fitting launch size is not re-requested"
    );
}

/// M-003 N3: with no work area (Wayland before the surface enters an output,
/// or a window AppKit has not placed on a screen), the rail grow still asks
/// for the unclamped need, as the panel did before M-003, instead of
/// dropping the resize.
#[test]
fn unknown_work_area_still_requests_the_unclamped_rail_need() {
    let frame = crate::test_support::FakeFrame::new(2.0, (0, 56), (2240, 1160), (100, 100));
    fit_window_in(&frame, None, (1384.0, 580.0));
    assert_eq!(frame.inner(), (2768, 1160), "logical 1384x580 at scale 2");
    assert_eq!(frame.resizes(), 1);
    assert_eq!(frame.moves(), 0, "no work area to move into");
}
