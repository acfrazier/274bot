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
    /// Copied from the readback job; never the current LIVE camera.
    #[cfg(feature = "render-diagnostics")]
    pub pixel_roi: Option<client::render::diagnostics::PixelRoiShotBind>,
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
        // dear-imgui-sys disables OS clipboard funcs; DefaultImpl is a local
        // buffer only. Install arboard so password-manager paste reaches
        // InputText (##cred-pass still allows paste). Cmd/Ctrl-V mapping stays
        // with dear-imgui-winit + ConfigMacOSXBehaviors.
        let _clipboard_ok = crate::clipboard::install_native_clipboard(&mut context);
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
    #[cfg(feature = "render-diagnostics")]
    pixel_roi: Option<client::render::diagnostics::PixelRoiShotBind>,
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
#[allow(clippy::too_many_arguments)] // frame submit packs device/queue/renderer/shot handles
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
#[allow(clippy::too_many_arguments)] // ui frame submit packs device/queue/renderer/shot handles
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
    #[cfg(feature = "render-diagnostics")]
    let pixel_roi = client::render::diagnostics::snapshot_shot_bind();
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
                #[cfg(feature = "render-diagnostics")]
                pixel_roi: pixel_roi.clone(),
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
                #[cfg(feature = "render-diagnostics")]
                pixel_roi: rb.pixel_roi,
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
                        crate::input_capture::add_shifted_key_event(
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
#[path = "window_tests.rs"]
mod tests;
