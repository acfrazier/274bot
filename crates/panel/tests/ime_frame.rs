//! Runs on the OS main thread: macOS cannot create winit windows in libtest workers.
//! Compile the private production bridge directly, without a public test-only API.
#[path = "../src/ime.rs"]
mod ime;

use std::sync::Arc;

use dear_imgui_rs as imgui;
use winit::application::ApplicationHandler;
use winit::dpi::{PhysicalPosition, PhysicalSize, Position, Size};
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::window::{Window, WindowId};

#[derive(Default)]
struct ImeFrame {
    completed: bool,
    areas: Vec<(Position, Size)>,
    cleared_areas: Vec<(Position, Size)>,
    expired_areas: Vec<(Position, Size)>,
    window_expired: bool,
    cleared_callbacks: usize,
    expired_callbacks: usize,
}

fn draw_input(context: &mut imgui::Context, text: &mut String, x: f32, focus: bool) {
    context.io_mut().set_display_size([800.0, 600.0]);
    context.io_mut().set_delta_time(1.0 / 60.0);
    let ui = context.frame();
    ui.window("ime")
        .position([x, 20.0], imgui::Condition::Always)
        .size([300.0, 150.0], imgui::Condition::Always)
        .build(|| {
            if focus {
                ui.set_keyboard_focus_here();
            }
            ui.input_text("##field", text).build();
        });
    context.render(); // EndFrame invokes the actual production callback.
}

impl ApplicationHandler for ImeFrame {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.completed {
            return;
        }
        let window = Arc::new(
            event_loop
                .create_window(
                    Window::default_attributes()
                        .with_visible(false)
                        .with_title("panel IME regression"),
                )
                .expect("hidden IME test window"),
        );
        let mut context = imgui::Context::create();
        let _ = context.set_ini_filename(None::<String>);
        // Exact platform bring-up used by AppWindow::finish: new -> attach Locked(1) -> install.
        let mut platform = ime::attach_platform(&mut context, &window);
        let _ = context.font_atlas_mut().build();

        let mut text = String::from("abc");
        for frame in 0..4 {
            platform.prepare_frame(&window, &mut context);
            draw_input(&mut context, &mut text, 20.0, frame == 0);
            self.areas.extend(ime::take_test_submissions());
        }
        ime::take_test_callback_count();

        // AppWindow::drop clears the association even if another Arc survives.
        ime::clear_window(&window);
        draw_input(&mut context, &mut text, 30.0, false);
        self.cleared_areas = ime::take_test_submissions();
        self.cleared_callbacks = ime::take_test_callback_count();

        // Also prove expiry alone is safe with the context still alive. Move the
        // focused input again so EndFrame has changed IME data to dispatch.
        let _platform = ime::attach_platform(&mut context, &window);
        let weak_window = Arc::downgrade(&window);
        drop(window);
        self.window_expired = weak_window.upgrade().is_none();
        draw_input(&mut context, &mut text, 40.0, false);
        self.expired_areas = ime::take_test_submissions();
        self.expired_callbacks = ime::take_test_callback_count();
        drop(context);
        self.completed = true;
        event_loop.exit();
    }

    fn window_event(&mut self, _: &ActiveEventLoop, _: WindowId, _: WindowEvent) {}
}

fn main() {
    #[cfg(target_os = "linux")]
    if std::env::var_os("DISPLAY").is_none() && std::env::var_os("WAYLAND_DISPLAY").is_none() {
        println!("ime_frame skipped: no desktop display; run under Xvfb to exercise winit");
        return;
    }
    let event_loop = EventLoop::new().expect("IME test event loop");
    let mut app = ImeFrame::default();
    event_loop
        .run_app(&mut app)
        .expect("IME test event loop run");
    // Assert outside the macOS delegate's non-unwinding FFI boundary.
    assert!(
        app.completed,
        "event loop must execute the IME frame regression"
    );
    assert_eq!(
        app.areas,
        vec![(
            Position::Physical(PhysicalPosition::new(52, 50)),
            Size::Physical(PhysicalSize::new(13, 13)),
        )],
        "focused InputText must submit its physical caret through production install",
    );
    assert_eq!(
        app.cleared_callbacks, 1,
        "EndFrame must dispatch after clear"
    );
    assert_eq!(
        app.expired_callbacks, 1,
        "EndFrame must dispatch after expiry"
    );
    assert!(
        app.cleared_areas.is_empty(),
        "cleared window must not receive IME areas"
    );
    assert!(
        app.window_expired,
        "the bridge must not keep a Window alive"
    );
    assert!(
        app.expired_areas.is_empty(),
        "expired window must not receive IME areas"
    );
    println!(
        "focused InputText submitted {:?}; cleared/expired window submissions suppressed",
        app.areas
    );
}
