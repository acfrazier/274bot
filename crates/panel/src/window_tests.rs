use super::ime::{
    physical_ime_set_data, submit_imgui_ime_area, take_test_callback_count, take_test_submissions,
};
use super::work_area::{
    cocoa_visible_frame_to_physical, fit_frame_to_work_area, win32_work_rect_to_physical, WorkArea,
};
use super::*;
use crate::test_support::headless_gpu;
use crate::theme::PANEL_FONT_SIZE;

#[test]
fn panel_adapter_request_options_uses_the_platform_default() {
    let expected = if cfg!(target_os = "linux") {
        wgpu::PowerPreference::None
    } else {
        wgpu::PowerPreference::LowPower
    };
    let options = panel_adapter_request_options(None, None);

    assert_eq!(options.power_preference, expected);
    assert!(options.compatible_surface.is_none());
    assert!(!options.force_fallback_adapter);
}

#[test]
fn panel_adapter_request_options_override_wins_over_the_platform_default() {
    for preference in [
        wgpu::PowerPreference::None,
        wgpu::PowerPreference::LowPower,
        wgpu::PowerPreference::HighPerformance,
    ] {
        assert_eq!(
            panel_adapter_request_options(None, Some(preference)).power_preference,
            preference,
        );
    }
}

#[test]
fn panel_render_pump_deadline_preempts_each_sleep_mode() {
    let now = std::time::Instant::now();
    let frame_due = now + std::time::Duration::from_secs(1);
    let render_due = now + std::time::Duration::from_millis(50);

    assert_eq!(
        render_deadline_control_flow(RedrawMode::Poll, frame_due, Some(render_due), None),
        ControlFlow::Poll,
        "Poll pumps every loop turn rather than sleeping past the render deadline"
    );
    assert_eq!(
        render_deadline_control_flow(RedrawMode::Wait, frame_due, Some(render_due), None),
        ControlFlow::WaitUntil(render_due)
    );
    assert_eq!(
        render_deadline_control_flow(
            RedrawMode::WaitUntil { fps: 60.0 },
            frame_due,
            Some(render_due),
            None,
        ),
        ControlFlow::WaitUntil(render_due)
    );
    assert_eq!(
        render_deadline_control_flow(
            RedrawMode::WaitUntil { fps: 60.0 },
            render_due,
            Some(frame_due),
            None,
        ),
        ControlFlow::WaitUntil(render_due),
        "the earlier panel frame still wins"
    );
    assert_eq!(
        render_deadline_control_flow(RedrawMode::Wait, frame_due, None, None),
        ControlFlow::Wait
    );
}

#[test]
fn recorder_deadline_wakes_wait_without_capture_or_render_work() {
    let now = Instant::now();
    let record_due = now + Duration::from_millis(500);
    let later = record_due + Duration::from_secs(1);
    for render_due in [None, Some(later)] {
        assert_eq!(
            render_deadline_control_flow(RedrawMode::Wait, later, render_due, Some(record_due)),
            ControlFlow::WaitUntil(record_due)
        );
        assert_eq!(
            render_deadline_control_flow(
                RedrawMode::WaitUntil { fps: 1.0 },
                later,
                render_due,
                Some(record_due),
            ),
            ControlFlow::WaitUntil(record_due)
        );
    }
    assert_eq!(
        render_deadline_control_flow(RedrawMode::Wait, later, Some(now), Some(record_due)),
        ControlFlow::WaitUntil(now),
        "recorder cadence must not postpone a render deadline"
    );
    assert_eq!(
        render_deadline_control_flow(RedrawMode::Poll, later, None, Some(record_due)),
        ControlFlow::Poll
    );
}

#[test]
fn legacy_recording_does_not_add_a_window_deadline() {
    let mut shots = ShotState::default();
    shots.set_ui_work_deadline(Some(Instant::now()));
    assert_eq!(shots.ui_work_deadline, None);
    assert!(!shots.capture_pending());
}

#[cfg(feature = "render-test-hooks")]
#[test]
fn gated_ui_work_deadline_is_scalar_and_does_not_arm_readback() {
    const CHILD: &str = "PANEL_CAPTURE_FIXTURE_CHILD";
    if std::env::var_os(CHILD).is_none() {
        run_capture_fixture_child(
            "window::tests::gated_ui_work_deadline_is_scalar_and_does_not_arm_readback",
            false,
        );
        return;
    }
    let _fixture = capture_process_fixture(true, false);
    let mut shots = ShotState::default();
    let now = Instant::now();
    let bytes = shots.metadata_bytes();
    shots.set_ui_work_deadline(Some(now));
    assert_eq!(shots.ui_work_deadline, Some(now));
    assert!(!shots.capture_pending());
    assert_eq!(shots.metadata_bytes(), bytes);
    assert_eq!(std::mem::size_of::<Option<Instant>>(), 16);
    shots.set_ui_work_deadline(None);
    assert_eq!(shots.ui_work_deadline, None);
    println!("render fixture child completed");
}

/// The `Gpu` texture-id manager must delegate register/update/unregister
/// to the renderer's texture store (the surface `game_view` and later
/// screenshot tasks rely on).
#[test]
fn gpu_texture_register_update_unregister_delegates_to_renderer_store() {
    let Some((device, queue)) = headless_gpu() else {
        return; // no adapter: nothing to delegate to
    };
    let _guard = crate::test_support::imgui_context_guard();
    let mut context = crate::test_support::imgui_context();
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

#[test]
fn ime_cursor_area_uses_imgui_physical_coordinates_at_fractional_scales() {
    for (scale, (x, y), height) in [
        (1.25_f32, (50, 30), 20),
        (1.5, (60, 36), 24),
        (1.75, (71, 43), 28),
        (2.0, (81, 49), 32),
    ] {
        let viewport_pos = [4.2 * scale, 5.4 * scale];
        let input_pos = [44.5 * scale, 29.7 * scale];
        let input_line_height = 16.0 * scale;
        let mut submitted = None;

        submit_imgui_ime_area(
            input_pos,
            viewport_pos,
            input_line_height,
            |position, size| submitted = Some((position, size)),
        );

        let expected = (
            winit::dpi::Position::Physical(winit::dpi::PhysicalPosition::new(x, y)),
            winit::dpi::Size::Physical(winit::dpi::PhysicalSize::new(height, height)),
        );
        assert_eq!(submitted, Some(expected), "wrong IME area at {scale}×");
    }
}

#[test]
fn physical_ime_callback_ignores_null_imgui_arguments() {
    take_test_submissions();
    take_test_callback_count();
    // SAFETY: null callback arguments are explicitly checked before dereference.
    unsafe {
        physical_ime_set_data(
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        );
    }
    assert!(
        take_test_submissions().is_empty(),
        "null arguments must not submit"
    );
    assert_eq!(
        take_test_callback_count(),
        0,
        "null data must not be consumed"
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

    shots
        .enqueue("gnome_chop".into(), "{\"scene\":2}".into())
        .expect("capture request is accepted");
    assert_eq!(shots.status("gnome_chop"), ShotStatus::Requested);

    assert_eq!(shots.promote_requests(), 1);
    assert_eq!(shots.status("gnome_chop"), ShotStatus::ReadbackPending);

    shots.wanted.clear();
    shots.done.push(ShotCapture {
        label: "gnome_chop".into(),
        snapshot_json: "{\"scene\":2}".into(),
        paint_sequence: 0,
        width: 1,
        height: 1,
        rgba: vec![0, 0, 0, 255],
        actor_stamp: None,
        readback_permit: None,
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
    shots
        .enqueue_for_actor(
            "air-alice".into(),
            "{\"actor\":\"alice\"}".into(),
            "alice".into(),
        )
        .expect("actor shot is accepted");
    shots
        .enqueue_for_actor("air-bob".into(), "{\"actor\":\"bob\"}".into(), "bob".into())
        .expect("actor shot is accepted");

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
    shots
        .enqueue_for_actor("air-alice".into(), "{}".into(), "alice".into())
        .expect("actor shot is accepted");
    shots
        .enqueue_for_actor("air-bob".into(), "{}".into(), "bob".into())
        .expect("actor shot is accepted");
    shots.promote_ready(Some("alice"), Some("alice"));
    shots.fail_labels(
        &["air-alice", "air-bob"],
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
        .enqueue("still-requested".into(), "{}".into())
        .expect("capture request is accepted");
    assert_eq!(deferred_readback_message(&shots, "occluded"), None);

    {
        let mut shots = shots.lock().unwrap();
        shots.promote_requests();
        shots
            .enqueue("not-promoted".into(), "{}".into())
            .expect("capture request is accepted");
    }
    assert_eq!(
        deferred_readback_message(&shots, "occluded").as_deref(),
        Some("[panel] capture readback deferred by surface occluded: still-requested")
    );
}

#[cfg(feature = "render-test-hooks")]
#[test]
fn many_untagged_shots_cannot_multiply_staging() {
    const CHILD: &str = "PANEL_CAPTURE_FIXTURE_CHILD";
    if std::env::var_os(CHILD).is_none() {
        run_capture_fixture_child(
            "window::tests::many_untagged_shots_cannot_multiply_staging",
            false,
        );
        return;
    }

    let _fixture = capture_process_fixture(true, false);
    let mut shots = ShotState::default();
    shots
        .configure_capture_policy(
            std::num::NonZeroUsize::new(16_384).unwrap(),
            std::time::Duration::from_secs(30),
        )
        .expect("the explicit process fixture enables bounded capture");
    let deadline = shots
        .capture_deadline(std::time::Instant::now())
        .expect("the fixture supplies an explicit capture deadline");
    for index in 0..CAPTURE_QUEUE {
        shots
            .enqueue_with_deadline(format!("shot-{index}"), "{\"scene\":2}".into(), deadline)
            .expect("each bounded request fits the fixture");
    }
    assert_eq!(
        shots.enqueue_with_deadline("overflow".into(), "{}".into(), deadline),
        Err(CaptureError::QueueFull),
        "the ninth request is refused explicitly"
    );
    assert_eq!(shots.promote_requests(), CAPTURE_QUEUE);

    let geometry = client::render::backend::CaptureGeometry::new(64, 64);
    let permit = shots
        .reserve_next_readback(geometry, std::time::Instant::now(), 1)
        .expect("the first job receives the process permit")
        .expect("one active readback is admitted");
    assert!(shots.active_capture.is_some());
    assert_eq!(shots.wanted.len(), CAPTURE_QUEUE - 1);
    assert!(shots
        .reserve_next_readback(geometry, std::time::Instant::now(), 2)
        .expect("an active job blocks another admission")
        .is_none());
    assert!(matches!(
        ReadbackBudget::process().try_acquire(geometry, deadline),
        Err(ReadbackError::Busy)
    ));
    shots.fail_active(CaptureError::Device);
    drop(permit);
    assert_eq!(
        shots.status("shot-0"),
        ShotStatus::FailedCapture(CaptureError::Device)
    );
    assert_eq!(shots.wanted.len(), CAPTURE_QUEUE - 1);
    println!("render fixture child completed");
}

#[cfg(feature = "render-test-hooks")]
#[test]
fn busy_recording_counts_typed_refusal_and_reuses_slot() {
    const CHILD: &str = "PANEL_CAPTURE_FIXTURE_CHILD";
    if std::env::var_os(CHILD).is_none() {
        run_capture_fixture_child(
            "window::tests::busy_recording_counts_typed_refusal_and_reuses_slot",
            false,
        );
        return;
    }
    let _fixture = capture_process_fixture(true, false);
    let mut shots = ShotState::default();
    shots
        .configure_capture_policy(
            std::num::NonZeroUsize::new(16_384).unwrap(),
            Duration::from_secs(30),
        )
        .unwrap();
    let now = Instant::now();
    let deadline = shots.capture_deadline(now).unwrap();
    let geometry = client::render::backend::CaptureGeometry::new(64, 64);
    let occupied = ReadbackBudget::process()
        .try_acquire(geometry, deadline)
        .unwrap();
    let busy_before = test_hooks::recording_busy_drops();
    shots.arm_record_with_deadline(deadline).unwrap();
    assert!(matches!(
        shots.reserve_next_readback(geometry, now, 1),
        Err(CaptureError::Busy)
    ));
    assert!(!shots.record_armed);
    assert_eq!(shots.take_record_drops(), 1);
    assert_eq!(test_hooks::recording_busy_drops(), busy_before + 1);
    drop(occupied);
    shots.arm_record_with_deadline(deadline).unwrap();
    let permit = shots
        .reserve_next_readback(geometry, now, 2)
        .unwrap()
        .expect("the next recording uses the released process slot");
    assert!(shots.active_capture.as_ref().unwrap().recording);
    shots.fail_active(CaptureError::Device);
    drop(permit);
    println!("render fixture child completed");
}

#[cfg(all(feature = "render-test-hooks", feature = "render-diagnostics"))]
#[test]
fn writer_diagnostic_label_capacity_is_admitted_before_copy() {
    const CHILD: &str = "PANEL_CAPTURE_FIXTURE_CHILD";
    if std::env::var_os(CHILD).is_none() {
        run_capture_fixture_child(
            "window::tests::writer_diagnostic_label_capacity_is_admitted_before_copy",
            false,
        );
        return;
    }

    let _fixture = capture_process_fixture(true, false);
    let mut shots = ShotState::default();
    shots
        .configure_capture_policy(
            std::num::NonZeroUsize::new(16_384).unwrap(),
            std::time::Duration::from_secs(30),
        )
        .expect("the explicit process fixture enables bounded capture");
    let mut snapshot_json = String::with_capacity(256);
    snapshot_json.push_str("{}");
    let snapshot_capacity = snapshot_json.capacity();
    shots.done.push(ShotCapture {
        label: "writer-label".into(),
        snapshot_json,
        width: 1,
        height: 1,
        rgba: Vec::new(),
        paint_sequence: 1,
        actor_stamp: None,
        #[cfg(feature = "render-diagnostics")]
        pixel_roi: None,
        readback_permit: None,
    });
    let capture = shots
        .take_done_for_write()
        .expect("the completed capture transfers to the writer");
    assert_eq!(capture.snapshot_json.capacity(), snapshot_capacity);
    let writer_charge = shots.write_pending_bytes;
    let policy = shots.capture_policy.unwrap();
    let before_diagnostic = shots.metadata_bytes();
    let denied_limit = before_diagnostic
        .checked_add("writer-label".len() - 1)
        .expect("the fixture's diagnostic limit does not overflow");
    shots.capture_policy = Some(CapturePolicy {
        metadata_limit: std::num::NonZeroUsize::new(denied_limit).unwrap(),
        readback_timeout: policy.readback_timeout,
    });
    assert_eq!(
        shots.try_clone_pending_diagnostic_label("writer-label"),
        Err(CaptureError::TooLarge),
        "diagnostic text is refused before the clone exceeds the byte allowance"
    );
    assert_eq!(shots.metadata_bytes(), before_diagnostic);
    assert!(!shots.write_pending_diagnostic_charged);
    shots.capture_policy = Some(policy);
    let diagnostic_label = shots
        .try_clone_pending_diagnostic_label("writer-label")
        .expect("diagnostic text is admitted before cloning");
    assert_eq!(diagnostic_label.as_str(), "writer-label");
    assert_eq!(
        shots.write_pending_bytes,
        writer_charge + diagnostic_label.capacity(),
        "the third label allocation is charged by its actual capacity"
    );
    assert!(shots.metadata_bytes() <= 16_384);
    assert!(shots.write_pending_diagnostic_charged);

    drop(capture);
    drop(diagnostic_label);
    shots.mark_written("writer-label");
    assert_eq!(shots.status("writer-label"), ShotStatus::Written);
    assert_eq!(shots.write_pending_bytes, 0);
    assert!(!shots.write_pending_diagnostic_charged);
    println!("render fixture child completed");
}

#[cfg(feature = "render-test-hooks")]
#[test]
fn writer_output_paths_share_the_explicit_metadata_allowance() {
    const CHILD: &str = "PANEL_CAPTURE_FIXTURE_CHILD";
    if std::env::var_os(CHILD).is_none() {
        run_capture_fixture_child(
            "window::tests::writer_output_paths_share_the_explicit_metadata_allowance",
            false,
        );
        return;
    }

    let _fixture = capture_process_fixture(true, false);
    let mut shots = ShotState::default();
    shots
        .configure_capture_policy(
            std::num::NonZeroUsize::new(16_384).unwrap(),
            std::time::Duration::from_secs(30),
        )
        .expect("the fixture supplies the metadata allowance");
    shots.done.push(ShotCapture {
        label: "writer-paths".into(),
        snapshot_json: "{}".into(),
        width: 1,
        height: 1,
        rgba: Vec::new(),
        paint_sequence: 1,
        actor_stamp: None,
        #[cfg(feature = "render-diagnostics")]
        pixel_roi: None,
        readback_permit: None,
    });
    let capture = shots.take_done_for_write().expect("pending writer");
    let writer_bytes = shots.write_pending_bytes;
    let policy = shots.capture_policy.unwrap();
    let before_paths = shots.metadata_bytes();
    shots.capture_policy = Some(CapturePolicy {
        metadata_limit: std::num::NonZeroUsize::new(before_paths + 2).unwrap(),
        readback_timeout: policy.readback_timeout,
    });
    assert_eq!(
        shots.with_pending_output_paths::<()>("writer-paths", |_| {
            panic!("path construction must not start without byte admission")
        }),
        Err(CaptureError::TooLarge)
    );
    assert_eq!(shots.metadata_bytes(), before_paths);
    shots.capture_policy = Some(policy);

    let directory = std::path::PathBuf::from("shots");
    let directory_bytes = directory.capacity();
    let paths = shots
        .with_pending_output_paths("writer-paths", |maximum| {
            let paths = frontend_core::live::shot_output_paths(
                &directory,
                "writer-paths",
                std::time::UNIX_EPOCH,
                maximum,
            )
            .map_err(|_| CaptureError::TooLarge)?;
            let output_bytes = paths.0.capacity() + paths.1.capacity();
            Ok((paths, directory_bytes, output_bytes))
        })
        .expect("the admitted path owners fit");
    assert_eq!(shots.output_directory_bytes, directory_bytes);
    assert_eq!(
        shots.write_pending_bytes,
        writer_bytes + paths.0.capacity() + paths.1.capacity()
    );
    assert!(shots.metadata_bytes() <= policy.metadata_limit.get());
    drop(paths);
    drop(capture);
    shots.mark_written("writer-paths");
    assert_eq!(shots.write_pending_bytes, 0);
    assert_eq!(shots.output_directory_bytes, directory_bytes);
    let terminal_bytes = shots.terminal.capacity() * std::mem::size_of::<TerminalStatus>()
        + shots
            .terminal
            .iter()
            .map(|status| status.label.capacity())
            .sum::<usize>();
    assert_eq!(shots.metadata_bytes(), directory_bytes + terminal_bytes);
    assert_eq!(
        shots.with_pending_output_paths::<()>("absent", |_| {
            panic!("an absent writer must not allocate output paths")
        }),
        Err(CaptureError::StaleHead)
    );
    println!("render fixture child completed");
}

#[cfg(feature = "render-test-hooks")]
#[test]
fn borrowed_capture_requests_admit_before_copy_and_defer_actor_json() {
    const CHILD: &str = "PANEL_CAPTURE_FIXTURE_CHILD";
    if std::env::var_os(CHILD).is_none() {
        run_capture_fixture_child(
            "window::tests::borrowed_capture_requests_admit_before_copy_and_defer_actor_json",
            false,
        );
        return;
    }
    let _fixture = capture_process_fixture(true, false);
    let mut shots = ShotState::default();
    assert_eq!(
        shots.enqueue_bounded_snapshot("absent-policy", |_| {
            panic!("missing policy must refuse before serialization")
        }),
        Err(CaptureError::PolicyMissing)
    );
    shots
        .configure_capture_policy(
            std::num::NonZeroUsize::new(16_384).unwrap(),
            std::time::Duration::from_secs(30),
        )
        .unwrap();
    let oversized = "x".repeat(CAPTURE_NAME_BYTES + 1);
    assert_eq!(
        shots.enqueue_bounded_snapshot(&oversized, |_| {
            panic!("oversized label must refuse before serialization")
        }),
        Err(CaptureError::TooLarge)
    );
    assert_eq!(
        shots.enqueue_bounded_actor_request("actor", &oversized),
        Err(CaptureError::TooLarge)
    );
    assert!(shots.requests.is_empty());
    shots
        .enqueue_bounded_snapshot("plain", |maximum| {
            assert!(maximum >= 2);
            Ok("{}".into())
        })
        .unwrap();
    shots
        .enqueue_bounded_actor_request("actor", "alice")
        .unwrap();
    assert_eq!(shots.requests[0].snapshot_json, "{}");
    assert_eq!(shots.requests[1].actor.as_deref(), Some("alice"));
    assert!(shots.requests[1].snapshot_json.is_empty());
    assert_eq!(shots.requests[1].snapshot_json.capacity(), 0);
    while shots.requests.len() < CAPTURE_QUEUE {
        shots
            .enqueue_bounded_actor_request(&format!("actor-{}", shots.requests.len()), "alice")
            .unwrap();
    }
    let before = shots.metadata_bytes();
    assert_eq!(
        shots.enqueue_bounded_snapshot("full", |_| {
            panic!("full queue must refuse before serialization")
        }),
        Err(CaptureError::QueueFull)
    );
    assert_eq!(shots.metadata_bytes(), before);
    assert!(before <= 16_384);
    println!("render fixture child completed");
}

#[cfg(feature = "render-test-hooks")]
#[test]
fn directory_replacement_and_typed_cancellation_keep_actual_metadata_charges() {
    const CHILD: &str = "PANEL_CAPTURE_FIXTURE_CHILD";
    if std::env::var_os(CHILD).is_none() {
        run_capture_fixture_child(
            "window::tests::directory_replacement_and_typed_cancellation_keep_actual_metadata_charges",
            false,
        );
        return;
    }
    let _fixture = capture_process_fixture(true, false);
    let mut shots = ShotState::default();
    shots
        .configure_capture_policy(
            std::num::NonZeroUsize::new(16_384).unwrap(),
            std::time::Duration::from_secs(30),
        )
        .unwrap();
    let mut directory = None;
    shots
        .replace_output_directory(&mut directory, |maximum| {
            assert!(maximum >= 5);
            frontend_core::live::record_output_path(std::path::Path::new(""), "shots")
                .map_err(|_| CaptureError::TooLarge)
        })
        .unwrap();
    let initial_bytes = directory.as_ref().unwrap().capacity();
    assert_eq!(shots.output_directory_bytes, initial_bytes);
    assert_eq!(shots.metadata_bytes(), initial_bytes);
    let policy = shots.capture_policy.unwrap();
    shots.capture_policy = Some(CapturePolicy {
        metadata_limit: std::num::NonZeroUsize::new(initial_bytes).unwrap(),
        readback_timeout: policy.readback_timeout,
    });
    assert_eq!(
        shots.replace_output_directory(&mut directory, |maximum| {
            assert_eq!(maximum, 0, "the old directory is still charged");
            Err(CaptureError::TooLarge)
        }),
        Err(CaptureError::TooLarge)
    );
    assert_eq!(directory.as_deref(), Some(std::path::Path::new("shots")));
    assert_eq!(shots.metadata_bytes(), initial_bytes);
    shots.capture_policy = Some(policy);
    let expected_allowance = policy.metadata_limit.get() - shots.metadata_bytes();
    shots
        .replace_output_directory(&mut directory, |maximum| {
            assert_eq!(maximum, expected_allowance);
            frontend_core::live::record_output_path(std::path::Path::new(""), "second")
                .map_err(|_| CaptureError::TooLarge)
        })
        .unwrap();
    let final_bytes = directory.as_ref().unwrap().capacity();
    assert_eq!(shots.metadata_bytes(), final_bytes);
    shots
        .enqueue_bounded_actor_request("cancelled", "alice")
        .unwrap();
    shots
        .fail_capture_label("cancelled", CaptureError::InvalidScene)
        .unwrap();
    assert!(shots.requests.is_empty());
    assert_eq!(
        shots.status("cancelled"),
        ShotStatus::FailedCapture(CaptureError::InvalidScene)
    );
    assert_eq!(shots.output_directory_bytes, final_bytes);
    assert!(shots.metadata_bytes() <= policy.metadata_limit.get());
    println!("render fixture child completed");
}

#[cfg(feature = "render-test-hooks")]
fn bounded_window_fixed_layout_receipt() {
    use std::mem::size_of;
    println!(
        "bounded_window_fixed_layout_receipt={}",
        serde_json::json!({
            "target_arch": std::env::consts::ARCH,
            "target_os": std::env::consts::OS,
            "pointer_bytes": size_of::<usize>(),
            "debug_assertions": cfg!(debug_assertions),
            "layouts": {
                "shot_state": size_of::<ShotState>(),
                "shot_request": size_of::<ShotRequest>(),
                "shot_capture": size_of::<ShotCapture>(),
                "active_capture": size_of::<ActiveCapture>(),
                "capture_policy": size_of::<CapturePolicy>(),
                "terminal_status": size_of::<TerminalStatus>(),
                "terminal_outcome": size_of::<TerminalOutcome>(),
                "frame_stamp": size_of::<FrameStamp>(),
                "ui_work_deadline_bytes_once_per_process": size_of::<Option<Instant>>(),
                "recording_busy_counter_bytes_once_per_process": size_of::<std::sync::atomic::AtomicU64>(),
                "borrowed_pair_label_stack": size_of::<[[u8; CAPTURE_NAME_BYTES]; 2]>(),
            },
            "container_entry_limit": CAPTURE_QUEUE,
            "scope": "Fixed named envelopes, not allocator-live totals. Variable owned JSON, names, directory and writer paths require the configured capture metadata allowance."
        })
    );
}

#[cfg(feature = "render-test-hooks")]
#[test]
fn terminal_capture_admission_precedes_label_and_queue_growth() {
    const CHILD: &str = "PANEL_CAPTURE_FIXTURE_CHILD";
    if std::env::var_os(CHILD).is_none() {
        run_capture_fixture_child(
            "window::tests::terminal_capture_admission_precedes_label_and_queue_growth",
            false,
        );
        return;
    }
    let _fixture = capture_process_fixture(true, false);
    let mut shots = ShotState::default();
    shots
        .configure_capture_policy(
            std::num::NonZeroUsize::new(std::mem::size_of::<TerminalStatus>() - 1).unwrap(),
            std::time::Duration::from_secs(30),
        )
        .unwrap();
    assert_eq!(
        shots.mark_failed_capture("x", CaptureError::Io),
        Err(CaptureError::TooLarge)
    );
    assert_eq!(shots.terminal.capacity(), 0);
    assert_eq!(shots.metadata_bytes(), 0);
    let policy = shots.capture_policy.unwrap();
    shots.capture_policy = Some(CapturePolicy {
        metadata_limit: std::num::NonZeroUsize::new(16_384).unwrap(),
        readback_timeout: policy.readback_timeout,
    });
    for index in 0..CAPTURE_QUEUE {
        shots
            .mark_failed_capture(&format!("label-{index:02}"), CaptureError::Io)
            .unwrap();
    }
    assert_eq!(shots.terminal.len(), CAPTURE_QUEUE);
    assert_eq!(shots.terminal.capacity(), CAPTURE_QUEUE);
    let storage = shots.terminal.as_ptr();
    for index in CAPTURE_QUEUE..4 * CAPTURE_QUEUE {
        shots
            .mark_failed_capture(&format!("label-{index:02}"), CaptureError::Io)
            .unwrap();
        assert_eq!(shots.terminal.len(), CAPTURE_QUEUE);
        assert_eq!(shots.terminal.capacity(), CAPTURE_QUEUE);
        assert_eq!(shots.terminal.as_ptr(), storage);
        assert!(shots.metadata_bytes() <= shots.capture_policy.unwrap().metadata_limit.get());
    }
    assert_eq!(
        shots.status("label-31"),
        ShotStatus::FailedCapture(CaptureError::Io)
    );
    bounded_window_fixed_layout_receipt();
    println!("render fixture child completed");
}

#[cfg(all(feature = "render-test-hooks", feature = "render-diagnostics"))]
#[test]
fn diagnostic_roi_and_capture_stamp_are_atomic() {
    const CHILD: &str = "PANEL_CAPTURE_FIXTURE_CHILD";
    if std::env::var_os(CHILD).is_none() {
        run_capture_fixture_child(
            "window::tests::diagnostic_roi_and_capture_stamp_are_atomic",
            false,
        );
        return;
    }

    let _fixture = capture_process_fixture(true, false);
    let mut shots = ShotState::default();
    shots
        .configure_capture_policy(
            std::num::NonZeroUsize::new(16_384).unwrap(),
            std::time::Duration::from_secs(30),
        )
        .expect("the explicit process fixture enables bounded capture");
    let deadline = shots
        .capture_deadline(std::time::Instant::now())
        .expect("the fixture supplies an explicit capture deadline");
    shots
        .enqueue_with_deadline("roi-shot".into(), "{}".into(), deadline)
        .expect("the shot request is admitted");
    assert_eq!(shots.promote_requests(), 1);
    let permit = shots
        .reserve_next_readback(
            client::render::backend::CaptureGeometry::new(64, 64),
            std::time::Instant::now(),
            1,
        )
        .expect("the process permit admits one ROI shot")
        .expect("the ROI shot is staged");
    let current_bytes = shots.metadata_bytes();
    let mut slot = String::with_capacity(128);
    slot.push_str("panel-slot");
    let slot_len = slot.len();
    let slot_capacity = slot.capacity();
    assert!(slot_capacity > slot_len);
    let mut metadata = client::render::diagnostics::PixelRoiMeta::produced(
        7,
        client::render::diagnostics::PixelRoiCam {
            cycle: 1,
            eye_x: 2,
            eye_y: 3,
            eye_z: 4,
            yaw: 5,
            pitch: 6,
            origin_x: 7,
            origin_z: 8,
            trace_frame: 9,
        },
        client::render::diagnostics::PackedHist::EMPTY,
        client::render::diagnostics::PackedHist::EMPTY,
    );
    metadata.upload = Some(client::render::diagnostics::PixelRoiUpload {
        frame_id: 7,
        packed: client::render::diagnostics::PackedHist::EMPTY,
        rgba: client::render::diagnostics::PackedHist::EMPTY,
        packed_fp: 0,
        slot,
        generation: 1,
    });
    let stamp = CaptureStamp {
        paint_sequence: 1,
        pixel_roi: Some(client::render::diagnostics::PixelRoiShotBind {
            meta: metadata,
            image: client::render::diagnostics::PixelRoiImage {
                logical_min: [0.0, 0.0],
                logical_size: [1.0, 1.0],
                fb_scale: [1.0, 1.0],
                domain: client::render::diagnostics::PixelRoiCoordDomain::LogicalImGui,
            },
        }),
    };
    assert_eq!(
        stamp.metadata_bytes(),
        std::mem::size_of::<CaptureStamp>() + slot_capacity
    );
    let policy = shots.capture_policy.unwrap();
    let text_only_limit = current_bytes
        .checked_add(std::mem::size_of::<CaptureStamp>())
        .and_then(|bytes| bytes.checked_add(slot_len))
        .unwrap();
    shots.capture_policy = Some(CapturePolicy {
        metadata_limit: std::num::NonZeroUsize::new(text_only_limit).unwrap(),
        readback_timeout: policy.readback_timeout,
    });
    assert_eq!(
        shots.attach_capture_stamp(stamp),
        Err(CaptureError::TooLarge),
        "ROI String capacity, not just its UTF-8 length, must fit the job"
    );
    assert!(
        shots
            .active_capture
            .as_ref()
            .is_some_and(|active| active.stamp.is_none()),
        "a rejected ROI cannot leave a partial capture stamp"
    );
    shots.fail_active(CaptureError::TooLarge);
    drop(permit);
    assert_eq!(
        shots.status("roi-shot"),
        ShotStatus::FailedCapture(CaptureError::TooLarge)
    );
    println!("render fixture child completed");
}

#[cfg(feature = "render-test-hooks")]
#[test]
#[ignore = "requires an explicitly configured headless GPU for retained-frame proof"]
fn actor_window_copy_fences_the_stamped_head() {
    const CHILD: &str = "PANEL_CAPTURE_FIXTURE_CHILD";
    if std::env::var_os(CHILD).is_none() {
        run_capture_fixture_child(
            "window::tests::actor_window_copy_fences_the_stamped_head",
            true,
        );
        return;
    }

    let mut fixture = capture_process_fixture(false, true);
    assert_eq!(
        fixture.backend_kind(),
        client::render::backend::BackendKind::Gpu,
        "the actor proof must exercise the GPU backend rather than CPU fallback"
    );
    let mut client = capture_scene_client();
    assert!(fixture.paint(&mut client));
    let first = fixture
        .output()
        .take_presented()
        .expect("the first paint publishes its genuine frame");
    let first_stamp = *first.stamp();
    let client::render::backend::FrameOutput::Texture(handle) = first.into_output() else {
        panic!("GPU paint publishes a texture handle");
    };
    let registration = handle
        .try_register_consumer()
        .expect("the head installs its retained consumer");

    assert!(fixture.paint(&mut client));
    let presented = fixture
        .output()
        .take_presented()
        .expect("the retained consumer receives a later genuine paint");
    let stamp = *presented.stamp();
    drop(presented.into_output());
    let consumer = registration
        .try_take_frame()
        .expect("the consumer offer can be polled")
        .expect("the later paint offers the retained frame");
    assert_eq!(
        stamp.present_token(),
        Some(consumer.present_token()),
        "the mailbox stamp and retained pixels identify one presentation"
    );
    assert!(stamp.has_valid_scene());
    assert_eq!(stamp.head(), first_stamp.head());
    assert!(stamp.mailbox_sequence() > first_stamp.mailbox_sequence());

    assert!(fixture.paint(&mut client));
    let newer = fixture
        .output()
        .take_presented()
        .expect("the next paint publishes a newer mailbox frame");
    let newer_stamp = *newer.stamp();
    drop(newer.into_output());
    assert!(newer_stamp.mailbox_sequence() > stamp.mailbox_sequence());

    let mut shots = ShotState::default();
    shots
        .configure_capture_policy(
            std::num::NonZeroUsize::new(16_384).unwrap(),
            std::time::Duration::from_secs(30),
        )
        .expect("the explicit process fixture enables bounded capture");
    let deadline = shots
        .capture_deadline(std::time::Instant::now())
        .expect("the fixture supplies an explicit capture deadline");
    shots
        .enqueue_for_actor_with_deadline(
            "actor-window".into(),
            "{}".into(),
            "alice".into(),
            deadline,
        )
        .expect("the actor request is admitted with its deadline");
    let mailbox = std::sync::Arc::downgrade(fixture.output());
    let fence = ActorCaptureFence::new(stamp, mailbox, Some(consumer))
        .expect("the authentic retained frame creates a matching fence");
    const SNAPSHOT_JSON: &str = "{\"scene\":2,\"accepted\":true}";
    let expected_allowance = shots
        .capture_policy
        .unwrap()
        .metadata_limit
        .get()
        .checked_sub(
            shots
                .metadata_bytes()
                .checked_add(fence.metadata_bytes())
                .unwrap(),
        )
        .unwrap();
    shots
        .accept_actor_frame(
            "alice",
            |allowance| {
                assert_eq!(allowance, expected_allowance);
                let mut snapshot_json = String::new();
                snapshot_json
                    .try_reserve_exact(SNAPSHOT_JSON.len())
                    .map_err(|_| CaptureError::QueueFull)?;
                if snapshot_json.capacity() > allowance {
                    return Err(CaptureError::TooLarge);
                }
                snapshot_json.push_str(SNAPSHOT_JSON);
                Ok(snapshot_json)
            },
            fence,
        )
        .expect("the exact frame and bounded sidecar are accepted together");
    assert_eq!(shots.wanted[0].1, SNAPSHOT_JSON);
    assert!(shots.holds_actor_frame("alice"));
    assert_eq!(
        shots
            .actor_consumer_frame("alice")
            .map(|frame| frame.present_token()),
        stamp.present_token(),
        "the accepted old frame remains held after a newer mailbox publication"
    );
    assert_eq!(
        shots.confirm_actor_display("alice", Some(first_stamp)),
        Err(CaptureError::StaleHead),
        "an earlier real presentation cannot confirm the accepted frame"
    );
    shots
        .confirm_actor_display("alice", Some(stamp))
        .expect("the exact accepted frame confirms its focused display");

    let permit = shots
        .reserve_next_readback(
            client::render::backend::CaptureGeometry::new(64, 64),
            std::time::Instant::now(),
            1,
        )
        .expect("confirmed actor work may reserve one readback")
        .expect("the actor readback is admitted");
    shots
        .validate_active_actor_for_staging()
        .expect("the actor fence remains valid before staging");
    assert!(shots.holds_actor_frame("alice"));
    shots.fail_active(CaptureError::Device);
    drop(permit);
    assert_eq!(
        shots.status("actor-window"),
        ShotStatus::FailedCapture(CaptureError::Device)
    );
    println!("render fixture child completed");
}

#[cfg(feature = "render-test-hooks")]
fn run_capture_fixture_child(test_name: &str, include_ignored: bool) {
    const CHILD: &str = "PANEL_CAPTURE_FIXTURE_CHILD";
    let mut command =
        std::process::Command::new(std::env::current_exe().expect("current panel test executable"));
    if include_ignored {
        command.arg("--ignored");
    }
    let output = command
        .args(["--exact", test_name, "--nocapture"])
        .env(CHILD, "1")
        .output()
        .expect("run the isolated capture fixture test process");
    assert!(
        output.status.success(),
        "capture fixture child failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("render fixture child completed"),
        "exact test filter did not run the capture fixture child:\n{}",
        String::from_utf8_lossy(&output.stdout),
    );
}

#[cfg(feature = "render-test-hooks")]
fn capture_process_fixture(
    prefer_cpu: bool,
    retained_consumer: bool,
) -> host::render_head::test_hooks::CaptureFixture {
    host::render_head::test_hooks::configure_process(
        host::render_head::test_hooks::ProcessFixture {
            grant_ttl: std::time::Duration::from_millis(20),
            quantum: std::time::Duration::from_millis(40),
        },
    )
    .expect("configure this child process before host initialization");
    if retained_consumer {
        client::render::backend::configure_retained_present_budget(
            std::num::NonZeroUsize::new(1).unwrap(),
        )
        .expect("one retained present is enough for this isolated head");
    }
    host::render_head::test_hooks::CaptureFixture::new(
        api::identity::SlotKey::from_incarnation(501).unwrap(),
        prefer_cpu,
        false,
    )
    .expect("construct the process fixture head")
}

#[cfg(feature = "render-test-hooks")]
fn capture_scene_client() -> client::client::Client {
    use client::client::{ClientConfig, ClientRevision};
    use client::dash3d::TerrainOverlayShape;

    let mut client = client::client::Client::new_with_revision(
        ClientConfig {
            host: "127.0.0.1".into(),
            port: 43594,
            cache_dir: "/nonexistent-window-capture-fixture".into(),
            members: true,
            lowmem: false,
        },
        ClientRevision::R289,
    );
    client.set_draw(true);
    client.ingame = true;
    client.scene_state = 2;
    client.world.fill_base_level(0);
    for x in 8..24 {
        for z in 8..40 {
            let hsl = ((x * 7 + z * 13) % 60) * 1024 + 2624;
            client.world.set_ground(
                0,
                x,
                z,
                TerrainOverlayShape::PLAIN,
                0,
                -1,
                0,
                0,
                0,
                0,
                hsl,
                hsl,
                hsl,
                hsl,
                0,
                0,
                0,
                0,
                0x00ff00,
                0,
            );
        }
    }
    client.cam_x = 16 * 128;
    client.cam_z = 4 * 128;
    client.cam_y = -600;
    client.cam_pitch = 200;
    client.cam_yaw = 0;
    client
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
    let mut context = crate::test_support::imgui_context();
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
#[derive(Debug)]
struct DpiLayoutFrame {
    text_vertex_bounds: [f32; 4],
    text_row_origin_y: Vec<f32>,
    text_row_bounds: Vec<[f32; 4]>,
    status_bounds: Vec<[f32; 4]>,
    rail_item_bounds: Vec<[f32; 4]>,
}
/// First-glyph bearing used to recover the emitted row origin from draw-list vertices.
fn baked_glyph_y0(ui: &imgui::Ui, codepoint: char) -> f32 {
    let font = ui.current_font();
    // SAFETY: the current baked font and its glyph buffer remain owned by
    // this live ImGui context throughout the frame.
    unsafe {
        let baked = (*font.raw()).LastBaked;
        assert!(!baked.is_null(), "current font has a baked size");
        let glyphs = &(*baked).Glyphs;
        assert!(glyphs.Size > 0 && !glyphs.Data.is_null());
        std::slice::from_raw_parts(glyphs.Data, glyphs.Size as usize)
            .iter()
            .find(|glyph| glyph.Codepoint() == codepoint as u32)
            .unwrap_or_else(|| panic!("current font contains {codepoint:?}"))
            .Y0
    }
}

fn draw_dpi_layout_frame(scale: f32) -> DpiLayoutFrame {
    let mut context = crate::test_support::imgui_context();
    context
        .io_mut()
        .set_backend_flags(imgui::BackendFlags::RENDERER_HAS_TEXTURES);
    crate::app::apply_ui_scale(context.style_mut(), scale);
    assert!(add_panel_font(&mut context));
    let _ = context.font_atlas_mut().build();
    context
        .io_mut()
        .set_display_size([320.0 * scale, 180.0 * scale]);
    context.io_mut().set_display_framebuffer_scale([1.0, 1.0]);
    context.io_mut().set_delta_time(1.0 / 60.0);

    let mut text_row_glyphs = Vec::new();
    let mut text_row_bounds = Vec::new();
    let mut rail_item_bounds = Vec::new();
    let mut status_colour = [0.0; 4];
    {
        let ui = context.frame();
        ui.window("##dpi-text")
            .position([0.0, 0.0], imgui::Condition::Always)
            .size([300.0 * scale, 80.0 * scale], imgui::Condition::Always)
            .flags(
                imgui::WindowFlags::NO_TITLE_BAR
                    | imgui::WindowFlags::NO_RESIZE
                    | imgui::WindowFlags::NO_MOVE
                    | imgui::WindowFlags::NO_SAVED_SETTINGS
                    | imgui::WindowFlags::NO_BACKGROUND,
            )
            .build(|| {
                for row in ["first rail row", "second rail row", "third rail row"] {
                    let first_glyph = row.chars().find(|c| !c.is_whitespace()).unwrap();
                    ui.text(row);
                    let y0 = baked_glyph_y0(ui, first_glyph);
                    let min = ui.item_rect_min();
                    let max = ui.item_rect_max();
                    text_row_bounds.push([min[0], min[1], max[0], max[1]]);
                    let glyph_count = row.chars().filter(|c| !c.is_whitespace()).count();
                    text_row_glyphs.push((glyph_count, y0));
                }
            });
        ui.window("##dpi-rail")
            .position([0.0, 80.0 * scale], imgui::Condition::Always)
            .size([300.0 * scale, 80.0 * scale], imgui::Condition::Always)
            .flags(
                imgui::WindowFlags::NO_TITLE_BAR
                    | imgui::WindowFlags::NO_RESIZE
                    | imgui::WindowFlags::NO_MOVE
                    | imgui::WindowFlags::NO_SAVED_SETTINGS
                    | imgui::WindowFlags::NO_BACKGROUND,
            )
            .build(|| {
                (rail_item_bounds, status_colour) = crate::app::draw_test_rail_cap(
                    ui,
                    crate::theme::scale_px(ui, crate::rail::RAIL_W),
                );
            });
    }
    let draw_data = context.render();
    let vertices: Vec<_> = draw_data
        .draw_lists()
        .flat_map(|list| list.vtx_buffer().iter())
        .collect();
    let is_colour = |vertex: &imgui::DrawVert, expected: [f32; 4]| {
        imgui::Color::from_imgui_u32(vertex.col)
            .to_array()
            .into_iter()
            .zip(expected)
            .all(|(actual, expected)| (actual - expected).abs() <= 1.0 / 255.0)
    };
    let text_vertices: Vec<_> = vertices
        .iter()
        .filter(|vertex| is_colour(vertex, [1.0, 1.0, 1.0, 1.0]))
        .collect();
    let mut text_row_origin_y = Vec::with_capacity(text_row_glyphs.len());
    let mut next_text_vertex = 0;
    for (glyph_count, glyph_y0) in text_row_glyphs {
        let row_vertices = text_vertices
            .get(next_text_vertex..next_text_vertex + glyph_count * 4)
            .expect("all text-row glyphs appear in the draw list");
        text_row_origin_y.push(row_vertices[0].pos[1] - glyph_y0);
        next_text_vertex += row_vertices.len();
    }
    let text_vertex_bounds = text_vertices.iter().fold(
        [
            f32::INFINITY,
            f32::INFINITY,
            f32::NEG_INFINITY,
            f32::NEG_INFINITY,
        ],
        |[min_x, min_y, max_x, max_y], vertex| {
            [
                min_x.min(vertex.pos[0]),
                min_y.min(vertex.pos[1]),
                max_x.max(vertex.pos[0]),
                max_y.max(vertex.pos[1]),
            ]
        },
    );
    let status_vertices: Vec<_> = vertices
        .iter()
        .filter(|vertex| is_colour(vertex, status_colour))
        .map(|vertex| [vertex.pos[0], vertex.pos[1]])
        .collect();
    let (status_quads, status_remainder) = status_vertices.as_slice().as_chunks::<4>();
    assert!(
        status_remainder.is_empty(),
        "status vertices must form complete rectangle quads"
    );
    let mut status_bounds: Vec<_> = status_quads
        .iter()
        .map(|quad| {
            let (mut min_x, mut min_y) = (f32::INFINITY, f32::INFINITY);
            let (mut max_x, mut max_y) = (f32::NEG_INFINITY, f32::NEG_INFINITY);
            for [x, y] in quad {
                min_x = min_x.min(*x);
                min_y = min_y.min(*y);
                max_x = max_x.max(*x);
                max_y = max_y.max(*y);
            }
            [min_x, min_y, max_x, max_y]
        })
        .collect();
    status_bounds.sort_by(|left, right| {
        left[1]
            .total_cmp(&right[1])
            .then_with(|| left[0].total_cmp(&right[0]))
    });
    DpiLayoutFrame {
        text_row_origin_y,
        text_vertex_bounds,
        text_row_bounds,
        status_bounds,
        rail_item_bounds,
    }
}

#[test]
fn dpi_layout_draw_list_keeps_text_rows_and_rail_items_pixel_aligned() {
    let _guard = crate::test_support::imgui_context_guard();
    let reference = draw_dpi_layout_frame(1.0);
    assert_eq!(
        reference.rail_item_bounds.len(),
        5,
        "status, mark, name, fold, remove"
    );
    for scale in [1.0, 1.25, 1.5, 1.75, 2.0] {
        let frame = if scale == 1.0 {
            DpiLayoutFrame {
                text_row_origin_y: reference.text_row_origin_y.clone(),
                text_vertex_bounds: reference.text_vertex_bounds,
                text_row_bounds: reference.text_row_bounds.clone(),
                status_bounds: reference.status_bounds.clone(),
                rail_item_bounds: reference.rail_item_bounds.clone(),
            }
        } else {
            draw_dpi_layout_frame(scale)
        };
        assert_eq!(
            frame.text_row_origin_y.len(),
            reference.text_row_origin_y.len()
        );
        for (origin, baseline) in frame
            .text_row_origin_y
            .iter()
            .zip(&reference.text_row_origin_y)
        {
            assert!(
                (*origin - origin.round()).abs() < 0.001,
                "text row origin must be a physical pixel at {scale}×: {origin}"
            );
            assert!(
                (origin - baseline * scale).abs() <= 1.0,
                "text row origin at {scale}× must match 1× within one pixel: {origin} vs {baseline}"
            );
        }
        assert_eq!(frame.text_row_bounds.len(), reference.text_row_bounds.len());
        for (actual, baseline) in frame.text_row_bounds.iter().zip(&reference.text_row_bounds) {
            assert!(
                (actual[0] - baseline[0] * scale).abs() <= 1.0
                    && (actual[1] - baseline[1] * scale).abs() <= 1.0
                    && ((actual[3] - actual[1]) - (baseline[3] - baseline[1]) * scale).abs()
                        <= 1.0,
                "text row origin/height at {scale}× must match 1× within one pixel: {actual:?} vs {baseline:?}"
            );
        }
        for rows in frame.text_row_bounds.windows(2) {
            assert!(
                rows[0][3] <= rows[1][1],
                "text rows must not overlap at {scale}×: {rows:?}"
            );
        }
        let [min_x, min_y, max_x, max_y] = frame.text_vertex_bounds;
        assert!(
            min_x >= 0.0 && min_y >= 0.0 && max_x <= 300.0 * scale && max_y <= 160.0 * scale,
            "text must remain visible within the window at {scale}×: {:?}",
            frame.text_vertex_bounds
        );
        assert_eq!(
            frame.status_bounds.len(),
            5,
            "five marker squares are drawn"
        );
        for (square, baseline) in frame.status_bounds.iter().zip(&reference.status_bounds) {
            assert!(
                square
                    .iter()
                    .all(|edge| (*edge - edge.round()).abs() < 0.001),
                "rail status square edges are physical pixels at {scale}×: {square:?}"
            );
            assert!(
                square
                    .iter()
                    .zip(baseline)
                    .all(|(actual, baseline)| (actual - baseline * scale).abs() <= 1.0),
                "status square at {scale}× must match 1× within one pixel: {square:?} vs {baseline:?}; all {:?} vs {:?}; text rows {:?} vs {:?}; rail items {:?} vs {:?}",
                frame.status_bounds,
                reference.status_bounds,
                frame.text_row_bounds,
                reference.text_row_bounds,
                frame.rail_item_bounds,
                reference.rail_item_bounds
            );
        }
        assert_eq!(frame.rail_item_bounds.len(), 5);
        for (item, baseline) in frame
            .rail_item_bounds
            .iter()
            .zip(&reference.rail_item_bounds)
        {
            assert!(
                (item[1] - item[1].round()).abs() < 0.001,
                "every rail item origin must be a physical pixel at {scale}×: {item:?}"
            );
            assert!(
                item.iter()
                    .zip(baseline)
                    .all(|(actual, baseline)| (actual - baseline * scale).abs() <= 1.0),
                "rail item at {scale}× must match 1× within one pixel: {item:?} vs {baseline:?}"
            );
        }
        assert!(
            frame.rail_item_bounds[0][2] <= frame.rail_item_bounds[1][0]
                && frame.rail_item_bounds[1][2] <= frame.rail_item_bounds[2][0]
                && frame.rail_item_bounds[2][2] <= frame.rail_item_bounds[3][0],
            "rail marker, name, fold and remove items must not overlap at {scale}×: {:?}",
            frame.rail_item_bounds
        );
        assert!(
            frame.text_row_bounds.last().unwrap()[3] <= frame.rail_item_bounds[0][1],
            "rail cap must not overlap the preceding text rows at {scale}×"
        );
    }
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
        shots
            .enqueue("flax_aio".into(), "{\"scene\":2}".into())
            .expect("capture request is accepted");
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
    let Some((device, queue)) = headless_gpu() else {
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
    let Some((device, queue)) = headless_gpu() else {
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
    shots
        .lock()
        .unwrap()
        .enqueue_for_actor("flax_aio".into(), "{\"scene\":1}".into(), "bot".into())
        .expect("actor shot is accepted");

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
    let Some((device, queue)) = headless_gpu() else {
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
    let mut ctx = crate::test_support::imgui_context();
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

#[test]
fn record_arm_flags_a_capture_without_touching_shot_wanted() {
    let mut shots = ShotState::default();
    assert!(!shots.capture_pending());
    shots
        .arm_record()
        .expect("legacy recording arm remains available");
    assert!(shots.capture_pending());
    assert!(
        shots.wanted.is_empty(),
        "the recording lane never enters the PNG promotion queue"
    );
    // Idempotent: repeated pacer ticks arm one readback, not one per tick.
    shots
        .arm_record()
        .expect("repeated legacy recording arm remains available");
    assert!(shots.capture_pending());
}

#[test]
fn record_readback_outcomes_route_to_record_done_not_the_png_ledger() {
    use crate::headed_record::RECORD_LABEL;

    fn capture(label: &str) -> ShotCapture {
        ShotCapture {
            label: label.into(),
            snapshot_json: String::new(),
            paint_sequence: 0,
            width: 2,
            height: 2,
            rgba: vec![0; 16],
            actor_stamp: None,
            readback_permit: None,
            #[cfg(feature = "render-diagnostics")]
            pixel_roi: None,
        }
    }

    let mut shots = ShotState::default();
    record_readback_outcomes(
        &mut shots,
        &[RECORD_LABEL.to_string(), "quester_path".to_string()],
        vec![capture(RECORD_LABEL), capture("quester_path")],
    );
    assert_eq!(shots.done.len(), 1);
    assert_eq!(shots.done[0].label, "quester_path");
    assert_eq!(shots.record_done.len(), 1);
    assert_eq!(shots.record_done[0].label, RECORD_LABEL);
    assert_eq!(
        shots.status("quester_path"),
        ShotStatus::WritePending,
        "ordinary shots keep their ledger stage"
    );
    assert_eq!(
        shots.status(RECORD_LABEL),
        ShotStatus::Missing,
        "recording frames never enter the PNG ledger"
    );
    assert_eq!(shots.take_record().len(), 1);
    assert!(shots.take_record().is_empty());
}

#[test]
fn lost_record_frame_leaves_no_png_failure_behind() {
    use crate::headed_record::RECORD_LABEL;

    let mut shots = ShotState::default();
    // The staged copy mapped nothing (device lost): the frame is simply
    // absent from the video and the pacer stages the next one.
    record_readback_outcomes(&mut shots, &[RECORD_LABEL.to_string()], vec![]);
    assert!(shots.done.is_empty());
    assert!(shots.record_done.is_empty());
    assert_eq!(shots.status(RECORD_LABEL), ShotStatus::Missing);
}

#[cfg(feature = "render-diagnostics")]
#[test]
fn window_diagnostic_server_profile_errors_keep_their_category() {
    use super::window_diagnostics::{error_kind, WindowDiagnostics};

    let kind = error_kind(&PanelError::ServerProfile("invalid local world".to_owned()));
    assert_eq!(kind, "server_profile");
    let mut diagnostics = WindowDiagnostics::new();
    let error = diagnostics.record_error_at(kind, 25);
    assert_eq!(error["record"], "window_lifecycle");
    assert_eq!(error["kind"], "error");
    assert_eq!(error["status"], "error");
    assert_eq!(error["error_kind"], "server_profile");
    assert!(error["window_id"].is_null());
}

#[cfg(feature = "render-diagnostics")]
#[test]
fn window_diagnostic_lifecycle_rows_keep_identity_order_status_and_scope() {
    use super::window_diagnostics::WindowDiagnostics;

    let mut diagnostics = WindowDiagnostics::new();
    let [birth, scope] =
        diagnostics.record_window_created_at("WindowId(17)".to_owned(), "Wait", 10);
    assert_eq!(birth["schema"], "render-s5-window-churn-v2");
    assert_eq!(birth["record"], "window_lifecycle");
    assert_eq!(birth["kind"], "birth");
    assert_eq!(birth["event_order"], 1);
    assert_eq!(birth["elapsed_ns"], 10);
    let clock_origin = birth["clock_origin_unix_ns"]
        .as_u64()
        .expect("the system clock has a Unix-epoch origin");
    assert_eq!(birth["window_id"], "WindowId(17)");
    assert_eq!(birth["status"], "success");
    assert_eq!(birth["redraw_mode"], "Wait");

    assert_eq!(scope["schema"], "render-s5-window-churn-v2");
    assert_eq!(scope["record"], "scope");
    assert_eq!(scope["event_order"], 2);
    assert_eq!(scope["elapsed_ns"], 10);
    assert_eq!(scope["clock_origin_unix_ns"], clock_origin);
    assert_eq!(scope["status"], "observed");
    assert_eq!(scope["window_id"], "WindowId(17)");
    assert_eq!(scope["pump_id"], "ApplicationHandler::about_to_wait");
    assert_eq!(
        scope["surfaces"],
        serde_json::json!([
            { "surface_id": "main_view", "role": "main_view" },
            { "surface_id": "sidecar_rail", "role": "sidecar_rail" }
        ])
    );

    let error = diagnostics.record_error_at("surface_creation", 25);
    assert_eq!(error["schema"], "render-s5-window-churn-v2");
    assert_eq!(error["record"], "window_lifecycle");
    assert_eq!(error["kind"], "error");
    assert_eq!(error["event_order"], 3);
    assert_eq!(error["elapsed_ns"], 25);
    assert_eq!(error["clock_origin_unix_ns"], clock_origin);
    assert_eq!(error["window_id"], "WindowId(17)");
    assert_eq!(error["status"], "error");
    assert_eq!(error["error_kind"], "surface_creation");

    let close = diagnostics
        .record_close_at("close_requested", 30)
        .expect("an active window has one close row");
    assert_eq!(close["schema"], "render-s5-window-churn-v2");
    assert_eq!(close["record"], "window_lifecycle");
    assert_eq!(close["kind"], "close");
    assert_eq!(close["event_order"], 4);
    assert_eq!(close["elapsed_ns"], 30);
    assert_eq!(close["clock_origin_unix_ns"], clock_origin);
    assert_eq!(close["window_id"], "WindowId(17)");
    assert_eq!(close["status"], "closed");
    assert_eq!(close["reason"], "close_requested");
    assert!(diagnostics.record_close_at("duplicate", 31).is_none());

    let encoded = serde_json::to_string(&[birth, scope, error, close])
        .expect("lifecycle observations are JSON serializable");
    let decoded: serde_json::Value =
        serde_json::from_str(&encoded).expect("serialized rows remain structured JSON");
    assert_eq!(decoded.as_array().unwrap().len(), 4);
}

#[cfg(feature = "render-diagnostics")]
#[test]
fn window_diagnostic_samples_separate_built_and_submitted_surfaces() {
    use super::window_diagnostics::{UiPaintObservation, WindowDiagnostics};

    let mut diagnostics = WindowDiagnostics::new();
    let [birth, _scope] =
        diagnostics.record_window_created_at("WindowId(17)".to_owned(), "WaitUntil", 0);
    let clock_origin = birth["clock_origin_unix_ns"]
        .as_u64()
        .expect("the system clock has a Unix-epoch origin");
    assert_eq!(birth["redraw_mode"], "WaitUntil");
    let mut paints = UiPaintObservation::default();
    paints.note_main_view_build(true);
    paints.note_rail_presence(true);
    paints.note_rail_build(true);
    assert_eq!(paints.main_view_built, Some(true));
    assert_eq!(paints.rail_present, Some(true));
    assert_eq!(paints.rail_built, Some(true));

    diagnostics.record_ui_frame_at(paints, 100);
    assert!(diagnostics.record_native_paint_at(Some(1), 200).is_none());
    diagnostics.record_ui_frame_at(paints, 1_000_000_100);
    let rows = diagnostics
        .record_native_paint_at(Some(2), 1_000_000_200)
        .expect("one second and two native paints yield bounded samples");
    assert!(rows[0].is_none());
    let main = rows[1].as_ref().expect("main-view paint has its own row");
    let rail = rows[2].as_ref().expect("rail paint has its own row");

    assert_eq!(main["schema"], "render-s5-window-churn-v2");
    assert_eq!(main["record"], "sample");
    assert_eq!(main["series"], "ui_paint");
    assert_eq!(main["event_order"], 3);
    assert_eq!(main["window_id"], "WindowId(17)");
    assert_eq!(main["surface_id"], "main_view");
    assert_eq!(main["role"], "main_view");
    assert_eq!(main["start_ns"], 0);
    assert_eq!(main["end_ns"], 1_000_000_200_u64);
    assert_eq!(main["clock_origin_unix_ns"], clock_origin);
    assert_eq!(main["status"], "success");
    assert_eq!(main["built_paint_count_start"], 0);
    assert_eq!(main["built_paint_count_end"], 2);
    assert_eq!(main["submitted_paint_count_start"], 0);
    assert_eq!(main["submitted_paint_count_end"], 2);
    assert_eq!(main["rail_present_start"], serde_json::Value::Null);
    assert_eq!(main["rail_present_end"], serde_json::Value::Null);
    assert_eq!(
        main["rail_present_frame_count_start"],
        serde_json::Value::Null
    );
    assert_eq!(
        main["rail_present_frame_count_end"],
        serde_json::Value::Null
    );
    assert_eq!(main["last_built_start"], serde_json::Value::Null);
    assert_eq!(main["last_built_end"], true);
    assert_eq!(main["frame_count_start"], 0);
    assert_eq!(main["frame_count_end"], 2);
    assert_eq!(main["native_paint_count_start"], 0);
    assert_eq!(main["native_paint_count_end"], 2);
    assert_eq!(main["native_sequence_start"], serde_json::Value::Null);
    assert_eq!(main["native_sequence_end"], 2);
    assert_eq!(main["last_ui_frame_monotonic_ns"], 1_000_000_100_u64);
    assert_eq!(main["last_submitted_ui_frame_count"], 2);

    assert_eq!(rail["schema"], "render-s5-window-churn-v2");
    assert_eq!(rail["record"], "sample");
    assert_eq!(rail["series"], "ui_paint");
    assert_eq!(rail["event_order"], 4);
    assert_eq!(rail["window_id"], "WindowId(17)");
    assert_eq!(rail["surface_id"], "sidecar_rail");
    assert_eq!(rail["role"], "sidecar_rail");
    assert_eq!(rail["start_ns"], 0);
    assert_eq!(rail["end_ns"], 1_000_000_200_u64);
    assert_eq!(rail["clock_origin_unix_ns"], clock_origin);
    assert_eq!(rail["status"], "success");
    assert_eq!(rail["built_paint_count_start"], 0);
    assert_eq!(rail["built_paint_count_end"], 2);
    assert_eq!(rail["submitted_paint_count_start"], 0);
    assert_eq!(rail["submitted_paint_count_end"], 2);
    assert_eq!(rail["rail_present_start"], serde_json::Value::Null);
    assert_eq!(rail["rail_present_end"], true);
    assert_eq!(rail["rail_present_frame_count_start"], 0);
    assert_eq!(rail["rail_present_frame_count_end"], 2);
    assert_eq!(rail["last_built_start"], serde_json::Value::Null);
    assert_eq!(rail["last_built_end"], true);
    assert_eq!(rail["frame_count_start"], 0);
    assert_eq!(rail["frame_count_end"], 2);
    assert_eq!(rail["native_paint_count_start"], 0);
    assert_eq!(rail["native_paint_count_end"], 2);
    assert_eq!(rail["native_sequence_start"], serde_json::Value::Null);
    assert_eq!(rail["native_sequence_end"], 2);
    assert_eq!(rail["last_ui_frame_monotonic_ns"], 1_000_000_100_u64);
    assert_eq!(rail["last_submitted_ui_frame_count"], 2);
    let close = diagnostics
        .record_close_at("test_complete", 1_000_000_201)
        .expect("the final lifecycle row follows both paint samples");
    assert_eq!(close["event_order"], 5);
    assert_eq!(close["elapsed_ns"], 1_000_000_201_u64);
    assert_eq!(close["clock_origin_unix_ns"], clock_origin);
    assert_eq!(close["status"], "closed");

    let encoded = serde_json::to_string(&rows).expect("samples remain JSON serializable");
    let decoded: serde_json::Value =
        serde_json::from_str(&encoded).expect("serialized samples remain structured JSON");
    assert_eq!(decoded.as_array().unwrap().len(), 3);
}

#[cfg(feature = "render-diagnostics")]
#[test]
fn window_diagnostic_pump_samples_report_real_pump_interval_and_progress() {
    use super::window_diagnostics::WindowDiagnostics;

    let mut diagnostics = WindowDiagnostics::new();
    let [birth, _scope] =
        diagnostics.record_window_created_at("WindowId(22)".to_owned(), "Poll", 0);
    let clock_origin = birth["clock_origin_unix_ns"]
        .as_u64()
        .expect("the system clock has a Unix-epoch origin");
    assert!(diagnostics.record_pump_at(10).is_none());
    assert!(diagnostics.record_pump_at(30).is_none());
    assert!(diagnostics.record_pump_at(75).is_none());
    let rows = diagnostics
        .record_pump_at(1_000_000_075)
        .expect("one second of pump progress yields a sample");
    let pump = rows[0].as_ref().expect("pump has a distinct sample");
    assert!(rows[1].is_none());
    assert!(rows[2].is_none());
    assert_eq!(pump["schema"], "render-s5-window-churn-v2");
    assert_eq!(pump["record"], "sample");
    assert_eq!(pump["series"], "window_pump");
    assert_eq!(pump["event_order"], 3);
    assert_eq!(pump["window_id"], "WindowId(22)");
    assert_eq!(pump["pump_id"], "ApplicationHandler::about_to_wait");
    assert_eq!(pump["start_ns"], 0);
    assert_eq!(pump["end_ns"], 1_000_000_075_u64);
    assert_eq!(pump["clock_origin_unix_ns"], clock_origin);
    assert_eq!(pump["status"], "success");
    assert_eq!(pump["redraw_mode"], "Poll");
    assert_eq!(pump["about_to_wait_count_start"], 0);
    assert_eq!(pump["about_to_wait_count_end"], 4);
    assert_eq!(pump["max_interval_ns_start"], serde_json::Value::Null);
    assert_eq!(pump["max_interval_ns_end"], 1_000_000_000_u64);
    assert_eq!(pump["last_interval_ns"], 1_000_000_000_u64);
    assert_eq!(pump["last_pump_monotonic_ns"], 1_000_000_075_u64);
    assert_eq!(pump["native_paint_count_start"], 0);
    assert_eq!(pump["native_paint_count_end"], 0);
    assert_eq!(pump["native_sequence_start"], serde_json::Value::Null);
    assert_eq!(pump["native_sequence_end"], serde_json::Value::Null);
}

#[cfg(feature = "render-diagnostics")]
#[test]
fn window_diagnostic_absent_rail_is_an_explicit_observation() {
    use super::window_diagnostics::{UiPaintObservation, WindowDiagnostics};

    let mut diagnostics = WindowDiagnostics::new();
    let _ = diagnostics.record_window_created_at("WindowId(23)".to_owned(), "Wait", 0);
    let mut paints = UiPaintObservation::default();
    paints.note_main_view_build(true);
    paints.note_rail_presence(false);
    assert_eq!(paints.rail_present, Some(false));
    assert_eq!(paints.rail_built, Some(false));
    diagnostics.record_ui_frame_at(paints, 1_000_000_000);
    let rows = diagnostics
        .record_native_paint_at(None, 1_000_000_100)
        .expect("the sampled UI frame progresses");
    let rail = rows[2]
        .as_ref()
        .expect("absence is distinct from no observation");
    assert_eq!(rail["rail_present_start"], serde_json::Value::Null);
    assert_eq!(rail["rail_present_end"], false);
    assert_eq!(rail["rail_present_frame_count_start"], 0);
    assert_eq!(rail["rail_present_frame_count_end"], 0);
    assert_eq!(rail["last_built_start"], serde_json::Value::Null);
    assert_eq!(rail["last_built_end"], false);
    assert_eq!(rail["built_paint_count_start"], 0);
    assert_eq!(rail["built_paint_count_end"], 0);
    assert_eq!(rail["submitted_paint_count_start"], 0);
    assert_eq!(rail["submitted_paint_count_end"], 0);
    assert_eq!(rail["frame_count_start"], 0);
    assert_eq!(rail["frame_count_end"], 1);
    assert_eq!(rail["native_paint_count_start"], 0);
    assert_eq!(rail["native_paint_count_end"], 1);
    assert_eq!(rail["native_sequence_end"], serde_json::Value::Null);
}

#[cfg(not(feature = "render-diagnostics"))]
#[test]
fn noninstrumented_window_layout_matches_the_frozen_pre_diagnostics_shape() {
    use std::mem::size_of;

    struct PreDiagnosticsGpu<'a> {
        device: &'a wgpu::Device,
        queue: &'a wgpu::Queue,
        renderer: &'a mut imgui_wgpu::WgpuRenderer,
    }

    struct PreDiagnosticsAppWindow {
        instance: wgpu::Instance,
        device: wgpu::Device,
        queue: wgpu::Queue,
        window: std::sync::Arc<Window>,
        surface_desc: wgpu::SurfaceConfiguration,
        surface: wgpu::Surface<'static>,
        imgui: ImguiState,
        clear_color: wgpu::Color,
        offscreen: Option<wgpu::Texture>,
        pending_capture: Option<PendingCapture>,
        paint_sequence: u64,
    }

    struct PreDiagnosticsApp<F> {
        cfg: PanelConfig,
        window: Option<AppWindow>,
        pending: Option<PendingGpu>,
        lifecycle: Lifecycle,
        ui_frame: F,
        on_gpu_shutdown: Box<dyn FnMut(&mut Gpu)>,
        shots: std::sync::Arc<std::sync::Mutex<ShotState>>,
        last_wake: Instant,
        failure: Option<PanelError>,
    }

    type LayoutFrame = for<'a> fn(&imgui::Ui, &mut Gpu<'a>) -> bool;

    assert_eq!(size_of::<Gpu>(), size_of::<PreDiagnosticsGpu<'_>>());
    assert_eq!(size_of::<AppWindow>(), size_of::<PreDiagnosticsAppWindow>());
    assert_eq!(
        size_of::<App<LayoutFrame>>(),
        size_of::<PreDiagnosticsApp<LayoutFrame>>()
    );

    assert_eq!(
        [
            std::mem::offset_of!(Gpu<'_>, device),
            std::mem::offset_of!(Gpu<'_>, queue),
            std::mem::offset_of!(Gpu<'_>, renderer),
        ],
        [
            std::mem::offset_of!(PreDiagnosticsGpu<'_>, device),
            std::mem::offset_of!(PreDiagnosticsGpu<'_>, queue),
            std::mem::offset_of!(PreDiagnosticsGpu<'_>, renderer),
        ]
    );
    assert_eq!(
        [
            std::mem::offset_of!(AppWindow, instance),
            std::mem::offset_of!(AppWindow, device),
            std::mem::offset_of!(AppWindow, queue),
            std::mem::offset_of!(AppWindow, window),
            std::mem::offset_of!(AppWindow, surface_desc),
            std::mem::offset_of!(AppWindow, surface),
            std::mem::offset_of!(AppWindow, imgui),
            std::mem::offset_of!(AppWindow, clear_color),
            std::mem::offset_of!(AppWindow, offscreen),
            std::mem::offset_of!(AppWindow, pending_capture),
            std::mem::offset_of!(AppWindow, paint_sequence),
        ],
        [
            std::mem::offset_of!(PreDiagnosticsAppWindow, instance),
            std::mem::offset_of!(PreDiagnosticsAppWindow, device),
            std::mem::offset_of!(PreDiagnosticsAppWindow, queue),
            std::mem::offset_of!(PreDiagnosticsAppWindow, window),
            std::mem::offset_of!(PreDiagnosticsAppWindow, surface_desc),
            std::mem::offset_of!(PreDiagnosticsAppWindow, surface),
            std::mem::offset_of!(PreDiagnosticsAppWindow, imgui),
            std::mem::offset_of!(PreDiagnosticsAppWindow, clear_color),
            std::mem::offset_of!(PreDiagnosticsAppWindow, offscreen),
            std::mem::offset_of!(PreDiagnosticsAppWindow, pending_capture),
            std::mem::offset_of!(PreDiagnosticsAppWindow, paint_sequence),
        ]
    );
    assert_eq!(
        [
            std::mem::offset_of!(App<LayoutFrame>, cfg),
            std::mem::offset_of!(App<LayoutFrame>, window),
            std::mem::offset_of!(App<LayoutFrame>, pending),
            std::mem::offset_of!(App<LayoutFrame>, lifecycle),
            std::mem::offset_of!(App<LayoutFrame>, ui_frame),
            std::mem::offset_of!(App<LayoutFrame>, on_gpu_shutdown),
            std::mem::offset_of!(App<LayoutFrame>, shots),
            std::mem::offset_of!(App<LayoutFrame>, last_wake),
            std::mem::offset_of!(App<LayoutFrame>, failure),
        ],
        [
            std::mem::offset_of!(PreDiagnosticsApp<LayoutFrame>, cfg),
            std::mem::offset_of!(PreDiagnosticsApp<LayoutFrame>, window),
            std::mem::offset_of!(PreDiagnosticsApp<LayoutFrame>, pending),
            std::mem::offset_of!(PreDiagnosticsApp<LayoutFrame>, lifecycle),
            std::mem::offset_of!(PreDiagnosticsApp<LayoutFrame>, ui_frame),
            std::mem::offset_of!(PreDiagnosticsApp<LayoutFrame>, on_gpu_shutdown),
            std::mem::offset_of!(PreDiagnosticsApp<LayoutFrame>, shots),
            std::mem::offset_of!(PreDiagnosticsApp<LayoutFrame>, last_wake),
            std::mem::offset_of!(PreDiagnosticsApp<LayoutFrame>, failure),
        ]
    );
}

#[cfg(feature = "render-diagnostics")]
#[test]
fn window_diagnostic_preserves_present_but_unbuilt_surface_states() {
    use super::window_diagnostics::{UiPaintObservation, WindowDiagnostics};

    let mut diagnostics = WindowDiagnostics::new();
    let _ = diagnostics.record_window_created_at("WindowId(24)".to_owned(), "WaitUntil", 0);
    let mut paints = UiPaintObservation::default();
    paints.note_main_view_build(false);
    paints.note_rail_presence(true);
    paints.note_rail_build(false);
    assert_eq!(paints.main_view_built, Some(false));
    assert_eq!(paints.rail_present, Some(true));
    assert_eq!(paints.rail_built, Some(false));
    diagnostics.record_ui_frame_at(paints, 1_000_000_000);
    let rows = diagnostics
        .record_native_paint_at(None, 1_000_000_100)
        .expect("a real UI frame progresses even without a surface build");
    let main = rows[1]
        .as_ref()
        .expect("the main view has a distinct observation");
    let rail = rows[2]
        .as_ref()
        .expect("the present rail has a distinct observation");
    assert_eq!(main["last_built_start"], serde_json::Value::Null);
    assert_eq!(main["last_built_end"], false);
    assert_eq!(main["built_paint_count_end"], 0);
    assert_eq!(main["submitted_paint_count_end"], 0);
    assert_eq!(rail["rail_present_start"], serde_json::Value::Null);
    assert_eq!(rail["rail_present_end"], true);
    assert_eq!(rail["rail_present_frame_count_end"], 1);
    assert_eq!(rail["last_built_start"], serde_json::Value::Null);
    assert_eq!(rail["last_built_end"], false);
    assert_eq!(rail["built_paint_count_end"], 0);
    assert_eq!(rail["submitted_paint_count_end"], 0);
}
