//! Panel-owned winit + wgpu + dear-imgui window loop.
//!
//! Instance → window → surface → surface-compatible adapter → device/queue
//! → surface config → imgui context (theme/ini) → platform + renderer,
//! then a per-frame `prepare_frame` → UI body → `prepare_render_with_ui`
//! → render pass → present cycle driven by an `ApplicationHandler` event
//! loop. The window/GPU stack is rebuilt on render errors.

use std::mem;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use dear_imgui_rs as imgui;
use dear_imgui_rs::{ConfigFlags, DockFlags, Id, TextureId, WindowFlags};
use dear_imgui_wgpu as imgui_wgpu;
use dear_imgui_winit as imgui_winit;
use pollster::block_on;
use thiserror::Error;
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::{Window, WindowId};

/// Panel window-loop error.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum PanelError {
    #[error("server profile: {0}")]
    ServerProfile(String),
    #[error("event loop error: {0}")]
    EventLoop(#[from] winit::error::EventLoopError),
    #[error("window creation failed: {0}")]
    WindowCreation(#[source] winit::error::OsError),
    #[error("WGPU surface creation failed: {0}")]
    SurfaceCreation(#[source] wgpu::CreateSurfaceError),
    #[error("no suitable WGPU adapter found: {0}")]
    AdapterUnavailable(#[source] wgpu::RequestAdapterError),
    #[error("WGPU device request failed: {0}")]
    DeviceRequest(#[source] wgpu::RequestDeviceError),
    #[error("WGPU renderer initialization failed: {0}")]
    RendererInit(#[source] imgui_wgpu::RendererError),
    #[error("WGPU renderer frame preparation failed: {0}")]
    FramePrepare(#[source] imgui_wgpu::RendererError),
    #[error("WGPU renderer draw failed: {0}")]
    Render(#[source] imgui_wgpu::RendererError),
    #[error("WGPU surface validation failed while acquiring the next frame")]
    SurfaceValidation,
}

/// A completed whole-window capture: RGBA8 pixels plus the scenario's
/// sidecar JSON, ready for the panel frame to write (`window.rs` never
/// touches the filesystem — the loop side stays pure).
#[derive(Debug)]
pub struct ShotCapture {
    pub label: String,
    pub snapshot_json: String,
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// Observable ownership stage for one labeled capture. Terminal harnesses
/// use this ledger rather than a per-frame write count, so a completed shot
/// remains completed while a separate proof gate is still pending.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ShotStatus {
    Missing,
    Requested,
    ReadbackPending,
    WritePending,
    Written,
    Failed(String),
}

/// One labeled capture request. Pair terminal shots carry the actor that
/// must be the selected visible main actor before this job is promoted
/// into whole-window readback.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShotRequest {
    pub label: String,
    pub snapshot_json: String,
    pub actor: Option<String>,
}

/// Whole-window capture coordination between the scenario sink (slot
/// thread), the UI body (per-frame drain), the render pass (readback),
/// and the panel frame (file write).
#[derive(Default)]
pub struct ShotState {
    /// Requests pushed by the scenario sink / pair terminal path, drained
    /// by the UI body.
    pub requests: Vec<ShotRequest>,
    /// The UI body's per-frame drain: what the next render pass captures.
    pub wanted: Vec<(String, String)>,
    /// Captures completed by the render pass, consumed by the UI body.
    pub done: Vec<ShotCapture>,
    /// Labels whose PNG and matching snapshot sidecar both wrote.
    written: Vec<String>,
    /// Terminal failures retained for the harness and diagnostics.
    failed: Vec<(String, String)>,
}

impl ShotState {
    pub fn enqueue(&mut self, label: String, snapshot_json: String) {
        self.enqueue_request(ShotRequest {
            label,
            snapshot_json,
            actor: None,
        });
    }

    pub fn enqueue_for_actor(&mut self, label: String, snapshot_json: String, actor: String) {
        self.enqueue_request(ShotRequest {
            label,
            snapshot_json,
            actor: Some(actor),
        });
    }

    fn enqueue_request(&mut self, request: ShotRequest) {
        self.written.retain(|written| written != &request.label);
        self.failed.retain(|(failed, _)| failed != &request.label);
        self.requests.push(request);
    }

    /// Promote untagged jobs. Actor-tagged pair jobs stay queued until
    /// [`Self::promote_ready`] sees that actor selected and presented.
    pub fn promote_requests(&mut self) -> usize {
        self.promote_ready(None, None)
    }

    /// Promote at most one actor-tagged job, and only when that actor is
    /// both focused and already presented in the game view. Untagged jobs
    /// promote together only when no actor-tagged job is pending, so two
    /// pair actors never share one unready whole-window readback.
    pub fn promote_ready(&mut self, focused: Option<&str>, presented: Option<&str>) -> usize {
        let requests = mem::take(&mut self.requests);
        let has_actor = requests.iter().any(|request| request.actor.is_some());
        let mut count = 0;
        let mut kept = Vec::new();
        let mut promoted_actor = false;
        for request in requests {
            match request.actor.as_deref() {
                None if !has_actor => {
                    self.wanted.push((request.label, request.snapshot_json));
                    count += 1;
                }
                None => kept.push(request),
                Some(actor)
                    if !promoted_actor && focused == Some(actor) && presented == Some(actor) =>
                {
                    self.wanted.push((request.label, request.snapshot_json));
                    count += 1;
                    promoted_actor = true;
                }
                Some(_) => kept.push(request),
            }
        }
        self.requests = kept;
        count
    }

    /// First actor-tagged job still waiting to be promoted.
    pub fn pending_actor_request(&self) -> Option<&ShotRequest> {
        self.requests.iter().find(|request| request.actor.is_some())
    }

    pub fn capture_in_flight(&self) -> bool {
        !self.wanted.is_empty() || !self.done.is_empty()
    }

    /// Replace the sidecar on a still-queued actor job so the readback
    /// carries the current scene2 snapshot, not the JSON frozen at enqueue.
    pub fn refresh_actor_sidecar(&mut self, actor: &str, snapshot_json: String) {
        if let Some(request) = self
            .requests
            .iter_mut()
            .find(|request| request.actor.as_deref() == Some(actor))
        {
            request.snapshot_json = snapshot_json;
        }
    }

    /// Drop pending work for `labels` and retain a terminal failure so a
    /// later unready buffer cannot write after the drain/error.
    pub fn fail_labels(&mut self, labels: &[String], error: &str) {
        self.requests
            .retain(|request| !labels.iter().any(|label| label == &request.label));
        self.wanted
            .retain(|(label, _)| !labels.iter().any(|wanted| wanted == label));
        self.done
            .retain(|capture| !labels.iter().any(|label| label == &capture.label));
        for label in labels {
            self.mark_failed(label, error);
        }
    }

    pub fn mark_written(&mut self, label: &str) {
        self.failed.retain(|(failed, _)| failed != label);
        if !self.written.iter().any(|written| written == label) {
            self.written.push(label.to_string());
        }
    }

    pub fn mark_failed(&mut self, label: &str, error: &str) {
        self.written.retain(|written| written != label);
        self.failed.retain(|(failed, _)| failed != label);
        self.failed.push((label.to_string(), error.to_string()));
    }

    pub fn status(&self, label: &str) -> ShotStatus {
        if let Some((_, error)) = self.failed.iter().rev().find(|(failed, _)| failed == label) {
            return ShotStatus::Failed(error.clone());
        }
        if self.written.iter().any(|written| written == label) {
            return ShotStatus::Written;
        }
        if self.done.iter().any(|capture| capture.label == label) {
            return ShotStatus::WritePending;
        }
        if self.wanted.iter().any(|(wanted, _)| wanted == label) {
            return ShotStatus::ReadbackPending;
        }
        if self
            .requests
            .iter()
            .any(|requested| requested.label == label)
        {
            return ShotStatus::Requested;
        }
        ShotStatus::Missing
    }
}

fn deferred_readback_message(shots: &Mutex<ShotState>, surface: &str) -> Option<String> {
    let shots = shots.lock().unwrap();
    if shots.wanted.is_empty() {
        return None;
    }
    let labels = shots
        .wanted
        .iter()
        .map(|(label, _)| label.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    Some(format!(
        "[panel] capture readback deferred by surface {surface}: {labels}"
    ))
}

fn report_deferred_readback(shots: &Mutex<ShotState>, surface: &str) {
    if std::env::var("BOT_DEBUG").as_deref() == Ok("1") {
        if let Some(message) = deferred_readback_message(shots, surface) {
            eprintln!("{message}");
        }
    }
}

/// Redraw behavior for the event loop.
#[derive(Clone, Copy, Debug)]
pub enum RedrawMode {
    /// Always redraw (ControlFlow::Poll)
    Poll,
    /// On-demand redraw (ControlFlow::Wait)
    Wait,
    /// Redraw at most `fps` per second using WaitUntil
    WaitUntil { fps: f32 },
}

/// Simple built-in theme applied before the `on_style` callback.
#[derive(Clone, Copy, Debug)]
pub enum Theme {
    Dark,
    Light,
    Classic,
}

/// Docking configuration.
#[derive(Clone, Copy, Debug)]
pub struct DockingConfig {
    /// Enable ImGui docking (sets `ConfigFlags::DOCKING_ENABLE`)
    pub enable: bool,
    /// Automatically create a fullscreen host window + dockspace over main viewport
    pub auto_dockspace: bool,
    /// Flags used for the created dockspace
    pub dockspace_flags: DockFlags,
    /// Host window flags (for the fullscreen dockspace host)
    pub host_window_flags: WindowFlags,
    /// Optional host window name (useful to persist ini settings)
    pub host_window_name: &'static str,
}

impl Default for DockingConfig {
    fn default() -> Self {
        Self {
            enable: true,
            auto_dockspace: true,
            dockspace_flags: DockFlags::PASSTHRU_CENTRAL_NODE,
            host_window_flags: WindowFlags::NO_TITLE_BAR
                | WindowFlags::NO_RESIZE
                | WindowFlags::NO_MOVE
                | WindowFlags::NO_COLLAPSE
                | WindowFlags::NO_BRING_TO_FRONT_ON_FOCUS
                | WindowFlags::NO_NAV_FOCUS,
            host_window_name: "DockSpaceHost",
        }
    }
}

/// Panel runner configuration.
pub struct PanelConfig {
    pub window_title: String,
    pub window_size: (f64, f64),
    pub present_mode: wgpu::PresentMode,
    pub clear_color: [f32; 4],
    pub docking: DockingConfig,
    pub ini_filename: Option<PathBuf>,
    pub restore_previous_geometry: bool,
    pub redraw: RedrawMode,
    /// Optional override for `Io::config_flags` in addition to the docking
    /// flag. If `Some`, it is merged with the docking flag; if `None`, only
    /// docking is applied.
    pub io_config_flags: Option<ConfigFlags>,
    /// Optional built-in theme to apply at startup (before `on_style`)
    pub theme: Option<Theme>,
}

impl Default for PanelConfig {
    fn default() -> Self {
        Self {
            window_title: "bot".into(),
            window_size: (1280.0, 720.0),
            present_mode: wgpu::PresentMode::Fifo,
            clear_color: [0.1, 0.2, 0.3, 1.0],
            docking: DockingConfig::default(),
            ini_filename: None,
            restore_previous_geometry: true,
            redraw: RedrawMode::Poll,
            io_config_flags: None,
            theme: None,
        }
    }
}

/// GPU access for the UI body: device/queue plus external-texture
/// registration into the ImGui renderer's texture store.
/// `device()`/`queue()` are the seam later tasks use; the texture-id
/// calls delegate straight to the renderer.
pub struct Gpu<'a> {
    device: &'a wgpu::Device,
    queue: &'a wgpu::Queue,
    renderer: &'a mut imgui_wgpu::WgpuRenderer,
}

impl<'a> Gpu<'a> {
    pub(crate) fn new(
        device: &'a wgpu::Device,
        queue: &'a wgpu::Queue,
        renderer: &'a mut imgui_wgpu::WgpuRenderer,
    ) -> Self {
        Self {
            device,
            queue,
            renderer,
        }
    }

    /// Access the WGPU device
    pub fn device(&self) -> &wgpu::Device {
        self.device
    }

    /// Access the default WGPU queue
    pub fn queue(&self) -> &wgpu::Queue {
        self.queue
    }

    /// Register an external texture + view and obtain an ImGui texture id.
    pub fn register_texture(
        &mut self,
        texture: &wgpu::Texture,
        view: &wgpu::TextureView,
    ) -> TextureId {
        self.renderer.register_external_texture(texture, view)
    }

    /// Update the view for an existing registered texture
    pub fn update_texture_view(&mut self, tex_id: TextureId, view: &wgpu::TextureView) -> bool {
        self.renderer.update_external_texture_view(tex_id, view)
    }

    /// Unregister a previously registered texture
    pub fn unregister_texture(&mut self, tex_id: TextureId) {
        self.renderer.unregister_texture(tex_id)
    }
}

struct ImguiState {
    context: imgui::Context,
    platform: imgui_winit::WinitPlatform,
    renderer: imgui_wgpu::WgpuRenderer,
}

struct AppWindow {
    // Kept alive to ensure the surface outlives its instance on all backends.
    #[allow(dead_code)]
    instance: wgpu::Instance,
    device: wgpu::Device,
    queue: wgpu::Queue,
    window: Arc<Window>,
    surface_desc: wgpu::SurfaceConfiguration,
    surface: wgpu::Surface<'static>,
    imgui: ImguiState,
    clear_color: wgpu::Color,
    /// Present for backends whose surface textures cannot be `COPY_SRC`:
    /// the imgui pass renders into this private offscreen texture (same
    /// format as the surface, so the present blit is a legal texture
    /// copy), the shot readback copies it, and it is blitted to the
    /// surface for present. Occluded capture reuses this target when it
    /// already exists; COPY_SRC backends allocate a one-off instead of
    /// keeping a second framebuffer on every slot.
    offscreen: Option<wgpu::Texture>,
}

impl AppWindow {
    fn new(
        event_loop: &ActiveEventLoop,
        cfg: &PanelConfig,
        lifecycle: &mut Lifecycle,
    ) -> Result<Self, PanelError> {
        // WGPU instance and window
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::PRIMARY,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });

        let window = {
            let size = LogicalSize::new(cfg.window_size.0, cfg.window_size.1);
            Arc::new(
                event_loop
                    .create_window(
                        Window::default_attributes()
                            .with_title(cfg.window_title.clone())
                            .with_inner_size(size),
                    )
                    .map_err(PanelError::WindowCreation)?,
            )
        };

        let surface = instance
            .create_surface(window.clone())
            .map_err(PanelError::SurfaceCreation)?;

        let adapter = block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: Some(&surface),
            force_fallback_adapter: false,
        }))
        .map_err(PanelError::AdapterUnavailable)?;

        let device_desc = wgpu::DeviceDescriptor {
            label: None,
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::default(),
            memory_hints: wgpu::MemoryHints::default(),
            ..Default::default()
        };
        let (device, queue) =
            block_on(adapter.request_device(&device_desc)).map_err(PanelError::DeviceRequest)?;

        if std::env::var("BOT_DEBUG").as_deref() == Ok("1") {
            let info = adapter.get_info();
            eprintln!(
                "[panel] adapter name={} backend={:?} device_type={:?} vendor={:#x} device={:#x}",
                info.name, info.backend, info.device_type, info.vendor, info.device
            );
        }

        // Surface config. Whole-window shots (the 377 harness pattern)
        // copy the just-rendered frame back before present, so the
        // surface asks for `COPY_SRC`. Some backends reject that on a
        // surface texture: then the loop renders into a private offscreen
        // texture (see `offscreen`) and the surface only needs `COPY_DST`
        // for the present blit.
        let physical_size = window.inner_size();
        let caps = surface.get_capabilities(&adapter);
        let preferred_srgb = [
            wgpu::TextureFormat::Bgra8UnormSrgb,
            wgpu::TextureFormat::Rgba8UnormSrgb,
        ];
        let format = preferred_srgb
            .iter()
            .cloned()
            .find(|f| caps.formats.contains(f))
            .unwrap_or(caps.formats[0]);

        let surface_copyable = adapter
            .get_texture_format_features(format)
            .allowed_usages
            .contains(wgpu::TextureUsages::COPY_SRC);
        let surface_usage = if surface_copyable {
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC
        } else {
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_DST
        };

        let surface_desc = wgpu::SurfaceConfiguration {
            usage: surface_usage,
            format,
            width: physical_size.width,
            height: physical_size.height,
            present_mode: cfg.present_mode,
            alpha_mode: wgpu::CompositeAlphaMode::Auto,
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };

        surface.configure(&device, &surface_desc);

        let offscreen = if surface_copyable {
            None
        } else {
            Some(make_offscreen(&device, &surface_desc))
        };

        if let Some(cb) = lifecycle.on_gpu_init.as_mut() {
            cb(&window, &device, &queue, &surface_desc);
        }

        // ImGui setup: ini before fonts; `restore_previous_geometry: false`
        // disables the ini file entirely.
        let mut context = imgui::Context::create();
        if !cfg.restore_previous_geometry {
            let _ = context.set_ini_filename(None::<String>);
        } else if let Some(p) = &cfg.ini_filename {
            let _ = context.set_ini_filename(Some(p.clone()));
        } else {
            let _ = context.set_ini_filename(None::<String>);
        }

        // Theme before the user style tweak
        if let Some(theme) = cfg.theme {
            apply_theme(&mut context, theme);
        }
        if let Some(cb) = lifecycle.on_style.as_mut() {
            cb(&mut context);
        }

        // The rail draws U+2059 (⁙) and U+2717 (✗) as text, which the
        // default Latin-1 font cannot render: merge an embedded DejaVu
        // Sans so they rasterize. Fail loudly rather than draw '?' again.
        let (quincunx, ballot_x, folds, fa) = add_glyph_font(&mut context);
        assert!(
            quincunx && ballot_x && folds && fa,
            "merged glyph font must cover rail DejaVu and file-dialog FA codepoints"
        );

        let mut platform = imgui_winit::WinitPlatform::new(&mut context);
        platform.attach_window(&window, imgui_winit::HiDpiMode::Default, &mut context);

        let init_info =
            imgui_wgpu::WgpuInitInfo::new(device.clone(), queue.clone(), surface_desc.format);
        let mut renderer = imgui_wgpu::WgpuRenderer::new(init_info, &mut context)
            .map_err(PanelError::RendererInit)?;
        renderer.set_gamma_mode(imgui_wgpu::GammaMode::Auto);

        // Configure IO flags & docking (never enable multi-viewport here)
        {
            let io = context.io_mut();
            let mut flags = io.config_flags();
            if cfg.docking.enable {
                flags.insert(ConfigFlags::DOCKING_ENABLE);
            }
            if let Some(extra) = &cfg.io_config_flags {
                let merged = flags.bits() | extra.bits();
                flags = ConfigFlags::from_bits_retain(merged);
            }
            io.set_config_flags(flags);
            io.set_config_windows_resize_from_edges(false);
            io.set_config_docking_always_tab_bar(false);
        }

        let imgui = ImguiState {
            context,
            platform,
            renderer,
        };

        Ok(Self {
            instance,
            device,
            queue,
            window,
            surface_desc,
            surface,
            imgui,
            clear_color: wgpu::Color {
                r: cfg.clear_color[0] as f64,
                g: cfg.clear_color[1] as f64,
                b: cfg.clear_color[2] as f64,
                a: cfg.clear_color[3] as f64,
            },
            offscreen,
        })
    }

    fn resize(&mut self, new_size: winit::dpi::PhysicalSize<u32>) {
        if new_size.width > 0 && new_size.height > 0 {
            self.surface_desc.width = new_size.width;
            self.surface_desc.height = new_size.height;
            self.surface.configure(&self.device, &self.surface_desc);
            if self.offscreen.is_some() {
                self.offscreen = Some(make_offscreen(&self.device, &self.surface_desc));
            }
        }
    }

    fn render<F>(
        &mut self,
        gui: &mut F,
        docking: &DockingConfig,
        shots: &Mutex<ShotState>,
    ) -> Result<(), PanelError>
    where
        F: FnMut(&imgui::Ui, &mut Gpu),
    {
        let _profile_frame = client::profiling::UI_FRAME.start();
        self.imgui
            .platform
            .prepare_frame(&self.window, &mut self.imgui.context);
        let ui = self.imgui.context.frame();

        // Optional fullscreen dockspace host. Off for the panel (we own the
        // game-left / panel-right split in `app::dock_host`), kept for parity
        // with the original scaffold.
        if docking.enable && docking.auto_dockspace {
            let viewport = ui.main_viewport();
            // Host window always covering the main viewport
            ui.set_next_window_viewport(viewport.id());
            let pos = viewport.pos();
            let size = viewport.size();
            // NO_BACKGROUND if passthru central node
            let current_flags = DockFlags::from_bits_retain(docking.dockspace_flags.bits());
            let mut win_flags = docking.host_window_flags;
            if current_flags.contains(DockFlags::PASSTHRU_CENTRAL_NODE) {
                win_flags |= WindowFlags::NO_BACKGROUND;
            }
            ui.window(docking.host_window_name)
                .flags(win_flags)
                .position([pos[0], pos[1]], imgui::Condition::Always)
                .size([size[0], size[1]], imgui::Condition::Always)
                .build(|| {
                    let ds_flags = DockFlags::from_bits_retain(current_flags.bits());
                    let _ = ui.dockspace_over_main_viewport_with_flags(Id::from(0u32), ds_flags);
                });
        }

        let mut gpu = Gpu::new(&self.device, &self.queue, &mut self.imgui.renderer);

        // Call the UI body
        gui(ui, &mut gpu);

        // Keep OS cursor/IME state in sync with Dear ImGui's per-frame intent.
        self.imgui.platform.prepare_render_with_ui(ui, &self.window);

        let draw_data = self.imgui.context.render();

        // Acquire the swapchain image as late as possible to reduce time holding it.
        let (frame, reconfigure_after_present) = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame) => (frame, false),
            wgpu::CurrentSurfaceTexture::Suboptimal(frame) => (frame, true),
            wgpu::CurrentSurfaceTexture::Lost => {
                report_deferred_readback(shots, "lost");
                self.surface.configure(&self.device, &self.surface_desc);
                return Ok(());
            }
            wgpu::CurrentSurfaceTexture::Outdated => {
                report_deferred_readback(shots, "outdated");
                self.surface.configure(&self.device, &self.surface_desc);
                return Ok(());
            }
            wgpu::CurrentSurfaceTexture::Timeout => {
                report_deferred_readback(shots, "timeout");
                return Ok(());
            }
            wgpu::CurrentSurfaceTexture::Occluded => {
                // Presentation is blocked, but a promoted capture can still
                // rasterize the current UI into a COPY_SRC target and
                // complete without a present. Unpromoted jobs stay queued.
                let submission = submit_acquired_frame(
                    &self.device,
                    &self.queue,
                    &mut self.imgui.renderer,
                    draw_data,
                    self.clear_color,
                    &self.surface_desc,
                    self.offscreen.as_ref(),
                    AcquiredFrame::Occluded,
                    shots,
                )?;
                // No image to present: the staged readbacks complete here.
                if let FrameSubmission::Submitted(readbacks) = submission {
                    if !readbacks.is_empty() && std::env::var("BOT_DEBUG").as_deref() == Ok("1") {
                        eprintln!(
                            "[panel] capture offscreen readback submitted on occluded surface: {}",
                            readbacks.len()
                        );
                    }
                    complete_readbacks(&self.device, self.surface_desc.format, readbacks, shots);
                }
                return Ok(());
            }
            wgpu::CurrentSurfaceTexture::Validation => {
                return Err(PanelError::SurfaceValidation);
            }
        };

        let submission = submit_acquired_frame(
            &self.device,
            &self.queue,
            &mut self.imgui.renderer,
            draw_data,
            self.clear_color,
            &self.surface_desc,
            self.offscreen.as_ref(),
            AcquiredFrame::Presentable(&frame.texture),
            shots,
        )?;
        // Submit → present → map, in that order: mapping/failed-capture
        // bookkeeping stays after the present, so a slow capture map never
        // holds the drawable (the original visible-frame ordering).
        frame.present();
        if let FrameSubmission::Submitted(readbacks) = submission {
            complete_readbacks(&self.device, self.surface_desc.format, readbacks, shots);
        }
        if reconfigure_after_present {
            self.surface.configure(&self.device, &self.surface_desc);
        }
        Ok(())
    }
}

/// One pending staging readback; the copy is recorded in the encoder and
/// the bytes land after submit + poll.
struct ShotReadback {
    label: String,
    snapshot_json: String,
    buffer: wgpu::Buffer,
    width: u32,
    height: u32,
}

fn record_readback_outcomes(
    shots: &mut ShotState,
    attempted: &[String],
    captures: Vec<ShotCapture>,
) {
    for label in attempted {
        if !captures.iter().any(|capture| &capture.label == label) {
            shots.mark_failed(label, "readback did not complete");
        }
    }
    shots.done.extend(captures);
}

/// One frame's acquired surface state, normalized from
/// [`wgpu::CurrentSurfaceTexture`] so both draw paths share one production
/// entry point: [`AcquiredFrame::Presentable`] for `Success`/`Suboptimal`,
/// [`AcquiredFrame::Occluded`] for a surface that cannot present.
enum AcquiredFrame<'a> {
    /// A presentable swapchain image: the UI blits onto it, and the caller
    /// presents it after submission.
    Presentable(&'a wgpu::Texture),
    /// The surface is occluded: no image exists this frame, so a promoted
    /// capture rasterizes offscreen and nothing is blitted or presented.
    Occluded,
}

/// What one acquired frame recorded. `Skipped` is the occluded no-op (no
/// promoted capture): no target allocated, no draw, no readback, no
/// present. `Submitted` carries the staged readbacks that
/// [`complete_readbacks`] maps once the caller has presented — or, with no
/// image to present, immediately.
enum FrameSubmission {
    Skipped,
    Submitted(Vec<ShotReadback>),
}

/// Production draw/capture path for one acquired frame: rasterizes the
/// current UI draw data (this frame's composition, never a copy of a
/// previous one) into `target`, stages the promoted shots from that same
/// texture, and blits onto the presentable image when the frame has one —
/// all in a single submission.
///
/// Target selection: the persistent offscreen when the backend keeps one
/// (the blit then carries the frame to the surface); otherwise the acquired
/// image itself; otherwise — occluded, with nothing presentable — a one-off
/// offscreen allocated here and dropped with this frame, so `COPY_SRC`
/// backends never keep a second always-on framebuffer.
///
/// Mapping the staged bytes is deliberately not part of this function: the
/// caller presents first and then calls [`complete_readbacks`], so a slow
/// capture map never holds the drawable. An occluded frame passes no
/// present image, never blits, and never presents; with nothing promoted it
/// returns [`FrameSubmission::Skipped`] before any allocation, draw, or
/// readback.
fn submit_acquired_frame(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    renderer: &mut imgui_wgpu::WgpuRenderer,
    draw_data: &mut imgui::DrawData,
    clear_color: wgpu::Color,
    desc: &wgpu::SurfaceConfiguration,
    offscreen: Option<&wgpu::Texture>,
    acquired: AcquiredFrame<'_>,
    shots: &Mutex<ShotState>,
) -> Result<FrameSubmission, PanelError> {
    let present_dest = match acquired {
        AcquiredFrame::Presentable(image) => Some(image),
        AcquiredFrame::Occluded => None,
    };

    // The promoted-wanted check runs before any GPU work: an occluded frame
    // with nothing promoted allocates no target and stages no readback
    // (unpromoted jobs stay queued for the next visible frame).
    if present_dest.is_none() && shots.lock().unwrap().wanted.is_empty() {
        return Ok(FrameSubmission::Skipped);
    }

    let one_off;
    let target: &wgpu::Texture = match offscreen {
        Some(existing) => existing,
        None => match present_dest {
            Some(image) => image,
            None => {
                one_off = make_offscreen(device, desc);
                &one_off
            }
        },
    };
    // An offscreen target reaches the surface only through the blit; a
    // surface that is its own target needs none.
    let blit_dest = present_dest.filter(|_| offscreen.is_some());

    let readbacks = submit_ui_frame(
        device,
        queue,
        renderer,
        draw_data,
        clear_color,
        target,
        blit_dest,
        shots,
    )?;
    Ok(FrameSubmission::Submitted(readbacks))
}

/// Rasterize `draw_data` into `target`, drain the promoted `wanted` shots
/// into `copy_texture_to_buffer` staging buffers recorded in that same
/// encoder, optionally blit `target` onto a presentable destination, and
/// submit. Returns the staged readbacks for [`complete_readbacks`] and
/// never blocks on the mapped bytes.
fn submit_ui_frame(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    renderer: &mut imgui_wgpu::WgpuRenderer,
    draw_data: &mut imgui::DrawData,
    clear_color: wgpu::Color,
    target: &wgpu::Texture,
    present_dest: Option<&wgpu::Texture>,
    shots: &Mutex<ShotState>,
) -> Result<Vec<ShotReadback>, PanelError> {
    let view = target.create_view(&wgpu::TextureViewDescriptor::default());
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("Render Encoder"),
    });

    {
        let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Render Pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(clear_color),
                    store: wgpu::StoreOp::Store,
                },
                depth_slice: None,
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });

        renderer.new_frame().map_err(PanelError::FramePrepare)?;
        renderer
            .render_draw_data(draw_data, &mut rpass)
            .map_err(PanelError::Render)?;
    }

    // Whole-window shots: drain the UI body's promoted requests, copy the
    // just-rendered frame into staging buffers (recorded in this encoder),
    // and map the bytes back after submit. No file I/O — the loop side
    // stays pure; `done` holds bytes for the panel.
    let mut readbacks: Vec<ShotReadback> = Vec::new();
    {
        let mut guard = shots.lock().unwrap();
        let wanted = mem::take(&mut guard.wanted);
        if !wanted.is_empty() {
            if std::env::var("BOT_DEBUG").as_deref() == Ok("1") {
                eprintln!("[panel] capture readback requested: {}", wanted.len());
            }
            readbacks = readback(device, target, &mut encoder, &wanted);
        }
    }
    if let Some(dest) = present_dest {
        blit_texture(&mut encoder, target, dest);
    }
    queue.submit(Some(encoder.finish()));
    Ok(readbacks)
}

/// Map the staged readbacks and record their outcomes (a failed map still
/// lands in the ledger as a capture failure). Runs after the caller
/// presented the frame, so the blocking map and write-back never hold the
/// drawable; device errors stay fail-closed at the render call site.
fn complete_readbacks(
    device: &wgpu::Device,
    format: wgpu::TextureFormat,
    readbacks: Vec<ShotReadback>,
    shots: &Mutex<ShotState>,
) {
    if readbacks.is_empty() {
        return;
    }
    let attempted = readbacks
        .iter()
        .map(|readback| readback.label.clone())
        .collect::<Vec<_>>();
    let captures = map_readbacks(device, format, readbacks);
    record_readback_outcomes(&mut shots.lock().unwrap(), &attempted, captures);
}

fn blit_texture(encoder: &mut wgpu::CommandEncoder, src: &wgpu::Texture, dest: &wgpu::Texture) {
    encoder.copy_texture_to_texture(
        wgpu::TexelCopyTextureInfo {
            texture: src,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyTextureInfo {
            texture: dest,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::Extent3d {
            width: src.width(),
            height: src.height(),
            depth_or_array_layers: 1,
        },
    );
}

/// Record a `copy_texture_to_buffer` per requested shot into a
/// `MAP_READ | COPY_DST` staging buffer. The copies share this encoder's
/// submission; the bytes land in [`map_readbacks`].
fn readback(
    device: &wgpu::Device,
    source: &wgpu::Texture,
    encoder: &mut wgpu::CommandEncoder,
    jobs: &[(String, String)],
) -> Vec<ShotReadback> {
    let width = source.width();
    let height = source.height();
    let bytes_per_row = 4 * width;
    let padded = align_up(bytes_per_row, wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
    jobs.iter()
        .map(|(label, snapshot_json)| {
            let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("274 panel shot staging"),
                size: padded as u64 * height as u64,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            encoder.copy_texture_to_buffer(
                wgpu::TexelCopyTextureInfo {
                    texture: source,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::TexelCopyBufferInfo {
                    buffer: &buffer,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(padded),
                        rows_per_image: Some(height),
                    },
                },
                wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
            );
            ShotReadback {
                label: label.clone(),
                snapshot_json: snapshot_json.clone(),
                buffer,
                width,
                height,
            }
        })
        .collect()
}

/// Block on the staging copies (poll) and pack the padded rows into
/// RGBA8, normalized from the surface/offscreen format.
fn map_readbacks(
    device: &wgpu::Device,
    format: wgpu::TextureFormat,
    readbacks: Vec<ShotReadback>,
) -> Vec<ShotCapture> {
    readbacks
        .into_iter()
        .filter_map(|rb| {
            let padded = align_up(4 * rb.width, wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
            let slice = rb.buffer.slice(..);
            let mapped = Arc::new(AtomicBool::new(false));
            let flag = Arc::clone(&mapped);
            let label = rb.label.clone();
            slice.map_async(wgpu::MapMode::Read, move |res| match res {
                Ok(()) => flag.store(true, Ordering::Release),
                Err(error) => eprintln!("[panel] shot {label}: map failed: {error}"),
            });
            if let Err(error) = device.poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: None,
            }) {
                eprintln!("[panel] shot {}: poll failed: {error}", rb.label);
            }
            // A failed map (device lost) drops the shot instead of
            // panicking the loop, which recovers GPU state on render
            // errors — the shot is a smoke artifact, not the run.
            if !mapped.load(Ordering::Acquire) {
                eprintln!("[panel] shot {}: readback did not complete", rb.label);
                return None;
            }
            let data = slice.get_mapped_range();
            let mut rgba = Vec::with_capacity((4 * rb.width * rb.height) as usize);
            for row in 0..rb.height as usize {
                let start = row * padded as usize;
                rgba.extend_from_slice(&data[start..start + (4 * rb.width) as usize]);
            }
            drop(data);
            rb.buffer.unmap();
            Some(ShotCapture {
                label: rb.label,
                snapshot_json: rb.snapshot_json,
                width: rb.width,
                height: rb.height,
                rgba: to_rgba(&rgba, format),
            })
        })
        .collect()
}

/// The offscreen capture target for backends whose surface textures
/// cannot be copied, and for a one-off occluded capture when the surface
/// itself is COPY_SRC. Same format as the surface so a present blit
/// (`copy_texture_to_texture`) is a legal copy. Captured bytes are
/// normalized to RGBA in [`to_rgba`].
fn make_offscreen(device: &wgpu::Device, desc: &wgpu::SurfaceConfiguration) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("274 panel shot offscreen"),
        size: wgpu::Extent3d {
            width: desc.width,
            height: desc.height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: desc.format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    })
}

/// The PNG wants RGBA; `Bgra8*` textures store their R/B bytes swapped.
fn to_rgba(bytes: &[u8], format: wgpu::TextureFormat) -> Vec<u8> {
    if matches!(
        format,
        wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb
    ) {
        let mut rgba = bytes.to_vec();
        for px in rgba.as_chunks_mut::<4>().0 {
            px.swap(0, 2);
        }
        rgba
    } else {
        bytes.to_vec()
    }
}

fn align_up(n: u32, align: u32) -> u32 {
    n.div_ceil(align) * align
}

/// DejaVu Sans 2.37 (SIL OFL), embedded so the rail's non-Latin-1 glyphs
/// render. The default atlas font (ProggyClean) covers Latin-1 only;
/// `U+2059` (⁙) and `U+2717` (✗) need a second font source.
const GLYPH_FONT_BYTES: &[u8] = include_bytes!("../assets/DejaVuSans.ttf");

/// Font Awesome 6 Free Solid (SIL OFL). File-dialog chrome only; rail
/// glyphs stay on DejaVu. Same `merge_mode` path as [`GLYPH_FONT_BYTES`].
const FA_FONT_BYTES: &[u8] = include_bytes!("../assets/fa-solid-900.ttf");

/// The two codepoints the rail draws beyond the default font's Latin-1,
/// as a Dear ImGui `(start, end)` pair list: `U+2059` (status quincunx)
/// and `U+2717` (remove), NUL-terminated.
const GLYPH_FONT_RANGES: [u32; 7] = [0x2059, 0x2059, 0x2582, 0x2585, 0x2717, 0x2717, 0];

/// FA PUA for Scripts file-dialog buttons (home, download, chevron,
/// folder, file/file-lines, desktop). Tiny pairs, NUL-terminated.
const FA_FONT_RANGES: [u32; 13] = [
    0xf015, 0xf015, 0xf019, 0xf019, 0xf054, 0xf054, 0xf07b, 0xf07b, 0xf15b, 0xf15c, 0xf390, 0xf390,
    0,
];

/// Merge the embedded DejaVu Sans into the atlas's default font so the
/// rail's `U+2059` and `U+2717` glyphs rasterize (they render as `?` in
/// the Latin-1 default font). `merge_mode` keeps Latin-1 on the default
/// font — only the ranged codepoints fall through to DejaVu, at the
/// default font's reference size (`size_pixels: 0.0`; an explicit size
/// would trip imgui's merge/implicit-reference-size assert). Returns the
/// two codepoints' presence in the merged font; the unit test pins it.
fn add_glyph_font(ctx: &mut imgui::Context) -> (bool, bool, bool, bool) {
    let mut fonts = ctx.fonts();
    fonts.add_font_default(None);
    let _dejavu = fonts
        .add_font_from_memory_ttf(
            GLYPH_FONT_BYTES,
            0.0,
            Some(
                &imgui::FontConfig::new()
                    .merge_mode(true)
                    .name("dejavu-sans (status/remove glyphs)"),
            ),
            Some(&GLYPH_FONT_RANGES),
        )
        .expect("embedded DejaVu Sans is a valid TTF");
    let merged = fonts
        .add_font_from_memory_ttf(
            FA_FONT_BYTES,
            0.0,
            Some(
                &imgui::FontConfig::new()
                    .merge_mode(true)
                    .name("fa-solid (file-dialog glyphs)"),
            ),
            Some(&FA_FONT_RANGES),
        )
        .expect("embedded Font Awesome Free Solid is a valid TTF");
    let fa = fa_dialog_glyphs_in(merged);
    (
        merged.is_glyph_in_font('\u{2059}'),
        merged.is_glyph_in_font('\u{2717}'),
        merged.is_glyph_in_font('\u{2582}') && merged.is_glyph_in_font('\u{2585}'),
        fa,
    )
}

fn fa_dialog_glyphs_in(font: &imgui::Font) -> bool {
    [
        crate::script_picker::GLYPH_HOME,
        crate::script_picker::GLYPH_DESKTOP,
        crate::script_picker::GLYPH_DOCUMENTS,
        crate::script_picker::GLYPH_DOWNLOADS,
        crate::script_picker::GLYPH_FOLDER,
        crate::script_picker::GLYPH_FILE,
        crate::script_picker::GLYPH_CHEVRON,
    ]
    .into_iter()
    .all(|s| s.chars().all(|c| font.is_glyph_in_font(c)))
}

/// Lifecycle callbacks: style tweak after the theme, and the GPU-init hook
/// that hands the live `Arc<Window>` to the panel (rail resize path).
type StyleCallback = Box<dyn FnMut(&mut imgui::Context)>;
type GpuInitCallback =
    Box<dyn FnMut(&Arc<Window>, &wgpu::Device, &wgpu::Queue, &wgpu::SurfaceConfiguration)>;

struct Lifecycle {
    on_style: Option<StyleCallback>,
    on_gpu_init: Option<GpuInitCallback>,
}

struct App<F>
where
    F: FnMut(&imgui::Ui, &mut Gpu) + 'static,
{
    cfg: PanelConfig,
    window: Option<AppWindow>,
    lifecycle: Lifecycle,
    ui_frame: F,
    /// Whole-window shot coordination shared with the UI body (the
    /// scenario sink's requests) and the render readback.
    shots: Arc<Mutex<ShotState>>,
    last_wake: Instant,
}

impl<F> App<F>
where
    F: FnMut(&imgui::Ui, &mut Gpu) + 'static,
{
    fn new(
        cfg: PanelConfig,
        on_style: impl FnMut(&mut imgui::Context) + 'static,
        on_gpu_init: impl FnMut(&Arc<Window>, &wgpu::Device, &wgpu::Queue, &wgpu::SurfaceConfiguration)
            + 'static,
        shots: Arc<Mutex<ShotState>>,
        ui_frame: F,
    ) -> Self {
        Self {
            cfg,
            window: None,
            lifecycle: Lifecycle {
                on_style: Some(Box::new(on_style)),
                on_gpu_init: Some(Box::new(on_gpu_init)),
            },
            ui_frame,
            shots,
            last_wake: Instant::now(),
        }
    }
}

impl<F> ApplicationHandler for App<F>
where
    F: FnMut(&imgui::Ui, &mut Gpu) + 'static,
{
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_none() {
            match AppWindow::new(event_loop, &self.cfg, &mut self.lifecycle) {
                Ok(window) => {
                    self.window = Some(window);
                    if let Some(w) = self.window.as_ref() {
                        w.window.request_redraw();
                    }
                }
                Err(e) => {
                    eprintln!("Failed to create window: {e}");
                    event_loop.exit();
                }
            }
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        // We may recreate the window/gpu stack on fatal GPU errors, so we avoid
        // holding a mutable borrow of self.window across the whole match.
        match event {
            WindowEvent::RedrawRequested => {
                // Render and, on fatal errors, attempt a full GPU/window rebuild.
                let mut need_recreate = false;
                if let Some(window) = self.window.as_mut() {
                    let full_event: winit::event::Event<()> = winit::event::Event::WindowEvent {
                        window_id,
                        event: event.clone(),
                    };
                    window.imgui.platform.handle_event(
                        &mut window.imgui.context,
                        &window.window,
                        &full_event,
                    );

                    if let Err(e) =
                        window.render(&mut self.ui_frame, &self.cfg.docking, self.shots.as_ref())
                    {
                        eprintln!(
                            "Render error: {e}; attempting to recover by recreating GPU state"
                        );
                        need_recreate = true;
                    } else if matches!(self.cfg.redraw, RedrawMode::Poll) {
                        window.window.request_redraw();
                    }
                }

                if need_recreate {
                    // Drop the existing window and try to rebuild the whole stack.
                    let old_window = self.window.take();
                    match AppWindow::new(event_loop, &self.cfg, &mut self.lifecycle) {
                        Ok(window) => {
                            self.window = Some(window);
                            if let Some(window) = self.window.as_mut() {
                                window.window.request_redraw();
                            }
                        }
                        Err(e) => {
                            eprintln!("Failed to recreate window after GPU error: {e}");
                            let _ = old_window;
                            event_loop.exit();
                        }
                    }
                }
            }
            _ => {
                let window = match self.window.as_mut() {
                    Some(window) => window,
                    None => return,
                };

                let full_event: winit::event::Event<()> = winit::event::Event::WindowEvent {
                    window_id,
                    event: event.clone(),
                };
                window.imgui.platform.handle_event(
                    &mut window.imgui.context,
                    &window.window,
                    &full_event,
                );
                if let WindowEvent::KeyboardInput { event, .. } = &event {
                    // The backend queues event.text for text widgets, but
                    // shifted punctuation has no ImGui key identity there.
                    // Add only the missing key lifecycle and record the
                    // produced/named capture ch in event order: text must
                    // not be queued a second time, and key-repeat must not
                    // extra-deliver.
                    if !event.repeat {
                        crate::app::add_shifted_key_event(
                            window.imgui.context.io_mut(),
                            &event.logical_key,
                            event.location,
                            event.state == winit::event::ElementState::Pressed,
                        );
                    }
                }

                match event {
                    WindowEvent::Resized(physical_size) => {
                        window.resize(physical_size);
                        window.window.request_redraw();
                    }
                    WindowEvent::ScaleFactorChanged { .. } => {
                        let new_size = window.window.inner_size();
                        window.resize(new_size);
                        window.window.request_redraw();
                    }
                    WindowEvent::CloseRequested => event_loop.exit(),
                    _ => {}
                }
            }
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        match self.cfg.redraw {
            RedrawMode::Poll => {
                event_loop.set_control_flow(ControlFlow::Poll);
                if let Some(window) = &self.window {
                    window.window.request_redraw();
                }
            }
            RedrawMode::Wait => {
                event_loop.set_control_flow(ControlFlow::Wait);
            }
            RedrawMode::WaitUntil { fps } => {
                let frame = Duration::from_secs_f32(1.0f32 / fps.max(1.0));
                let now = Instant::now();
                let mut next_wake = self.last_wake + frame;
                if now >= next_wake {
                    self.last_wake = now;
                    next_wake = self.last_wake + frame;
                    if let Some(window) = &self.window {
                        window.window.request_redraw();
                    }
                }
                event_loop.set_control_flow(ControlFlow::WaitUntil(next_wake));
            }
        }
    }
}

/// Run the panel window loop. `ui_frame` is called every frame with the Ui
/// and the `Gpu` handle; `on_gpu_init` fires once the device/queue/surface
/// exist (the loop owns the window now). `shots` coordinates whole-window
/// captures between the UI body (requests) and the render pass (readback).
pub fn run<F>(
    cfg: PanelConfig,
    on_style: impl FnMut(&mut imgui::Context) + 'static,
    on_gpu_init: impl FnMut(&Arc<Window>, &wgpu::Device, &wgpu::Queue, &wgpu::SurfaceConfiguration)
        + 'static,
    shots: Arc<Mutex<ShotState>>,
    ui_frame: F,
) -> Result<(), PanelError>
where
    F: FnMut(&imgui::Ui, &mut Gpu) + 'static,
{
    let event_loop = EventLoop::new()?;
    match cfg.redraw {
        RedrawMode::Poll => event_loop.set_control_flow(ControlFlow::Poll),
        RedrawMode::Wait => event_loop.set_control_flow(ControlFlow::Wait),
        RedrawMode::WaitUntil { fps } => {
            let frame = Duration::from_secs_f32(1.0f32 / fps.max(1.0));
            event_loop.set_control_flow(ControlFlow::WaitUntil(Instant::now() + frame));
        }
    }

    let mut app = App::new(cfg, on_style, on_gpu_init, shots, ui_frame);
    event_loop.run_app(&mut app)?;
    Ok(())
}

fn apply_theme(ctx: &mut imgui::Context, theme: Theme) {
    let preset = match theme {
        Theme::Dark => imgui::ThemePreset::Dark,
        Theme::Light => imgui::ThemePreset::Light,
        Theme::Classic => imgui::ThemePreset::Classic,
    };
    let t = imgui::Theme {
        preset,
        ..Default::default()
    };
    t.apply_to_context(ctx);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::IMGUI_CTX_TEST_GUARD;

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
        let _guard = IMGUI_CTX_TEST_GUARD.lock().unwrap();
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
        for chunk in data.chunks_exact_mut(4) {
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
        let _guard = IMGUI_CTX_TEST_GUARD.lock().unwrap();
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
            !reused.chunks_exact(4).any(|px| px == MAGENTA),
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
        let _guard = IMGUI_CTX_TEST_GUARD.lock().unwrap();
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
            prefilled.chunks_exact(4).all(|px| px == MAGENTA),
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
        let _guard = IMGUI_CTX_TEST_GUARD.lock().unwrap();
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
            !drawn.chunks_exact(4).any(|px| px == MAGENTA),
            "the acquired image holds this frame, not the stale prefill"
        );
    }

    /// The merged glyph font must cover the two non-Latin-1 codepoints
    /// the rail draws as text: U+2059 (⁙ status dot) and U+2717 (✗
    /// remove). Without them the panel would render `?` again.
    #[test]
    fn glyph_font_merges_status_and_remove_codepoints() {
        let _guard = IMGUI_CTX_TEST_GUARD.lock().unwrap();
        let mut ctx = imgui::Context::create();
        let (quincunx, ballot_x, folds, fa) = add_glyph_font(&mut ctx);
        assert!(
            quincunx,
            "U+2059 (status dot) must resolve in the merged font"
        );
        assert!(ballot_x, "U+2717 (remove) must resolve in the merged font");
        assert!(folds, "U+2582/U+2585 (fold/unfold) must resolve");
        assert!(
            fa,
            "FA Free Solid PUA (home/desktop/docs/downloads/folder/file/chevron) must resolve"
        );
    }
}
