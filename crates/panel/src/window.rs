//! Panel-owned winit + wgpu + dear-imgui window loop (dear-app re-roll).
//!
//! Replaces `dear-app`'s scaffold: instance → window → surface →
//! surface-compatible adapter → device/queue → surface config → imgui
//! context (theme/ini) → platform + renderer, then a per-frame
//! `prepare_frame` → UI body → `prepare_render_with_ui` → render pass →
//! present cycle driven by an `ApplicationHandler` event loop. The
//! window/GPU stack is rebuilt on render errors exactly like the original.

use std::path::PathBuf;
use std::sync::Arc;
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

/// Panel window-loop error (dear-app `DearAppError` equivalent).
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum PanelError {
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

/// Redraw behavior for the event loop (dear-app `RedrawMode` equivalent).
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

/// Docking configuration (dear-app `DockingConfig` equivalent).
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

/// Panel runner configuration (dear-app `RunnerConfig` equivalent).
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
            window_title: "274bot".into(),
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
/// registration into the ImGui renderer's texture store (dear-app `GpuApi`
/// equivalent). `device()`/`queue()` are the seam later tasks use; the
/// texture-id calls delegate straight to the renderer.
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

        // Surface config
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

        let surface_desc = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: physical_size.width,
            height: physical_size.height,
            present_mode: cfg.present_mode,
            alpha_mode: wgpu::CompositeAlphaMode::Auto,
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };

        surface.configure(&device, &surface_desc);

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
        })
    }

    fn resize(&mut self, new_size: winit::dpi::PhysicalSize<u32>) {
        if new_size.width > 0 && new_size.height > 0 {
            self.surface_desc.width = new_size.width;
            self.surface_desc.height = new_size.height;
            self.surface.configure(&self.device, &self.surface_desc);
        }
    }

    fn render<F>(&mut self, gui: &mut F, docking: &DockingConfig) -> Result<(), PanelError>
    where
        F: FnMut(&imgui::Ui, &mut Gpu),
    {
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
            wgpu::CurrentSurfaceTexture::Lost | wgpu::CurrentSurfaceTexture::Outdated => {
                self.surface.configure(&self.device, &self.surface_desc);
                return Ok(());
            }
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                return Ok(());
            }
            wgpu::CurrentSurfaceTexture::Validation => {
                return Err(PanelError::SurfaceValidation);
            }
        };

        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Render Encoder"),
            });

        {
            let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Render Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(self.clear_color),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });

            self.imgui
                .renderer
                .new_frame()
                .map_err(PanelError::FramePrepare)?;
            self.imgui
                .renderer
                .render_draw_data(draw_data, &mut rpass)
                .map_err(PanelError::Render)?;
        }

        self.queue.submit(Some(encoder.finish()));
        frame.present();
        if reconfigure_after_present {
            self.surface.configure(&self.device, &self.surface_desc);
        }
        Ok(())
    }
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

                    if let Err(e) = window.render(&mut self.ui_frame, &self.cfg.docking) {
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
/// exist (the loop owns the window now).
pub fn run<F>(
    cfg: PanelConfig,
    on_style: impl FnMut(&mut imgui::Context) + 'static,
    on_gpu_init: impl FnMut(&Arc<Window>, &wgpu::Device, &wgpu::Queue, &wgpu::SurfaceConfiguration)
        + 'static,
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

    let mut app = App::new(cfg, on_style, on_gpu_init, ui_frame);
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

    /// Docking defaults mirror dear-app's: passthru central node, no-chrome
    /// host window. `app::runner_config` overrides both.
    #[test]
    fn docking_config_defaults_match_dear_app() {
        let d = DockingConfig::default();
        assert!(d.enable);
        assert!(d.auto_dockspace);
        assert!(d.dockspace_flags.contains(DockFlags::PASSTHRU_CENTRAL_NODE));
        assert!(d.host_window_flags.contains(WindowFlags::NO_TITLE_BAR));
        assert!(d.host_window_flags.contains(WindowFlags::NO_RESIZE));
        assert_eq!(d.host_window_name, "DockSpaceHost");
    }
}
