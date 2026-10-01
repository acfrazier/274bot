//! Panel's single-window, physical-pixel IME bridge.
use std::cell::RefCell;
use std::sync::{Arc, Weak};

use dear_imgui_rs as imgui;
use dear_imgui_winit as imgui_winit;
use winit::window::Window;

// The panel has one ImGui context/window on the winit event-loop thread.
// Never rely on attach_window's raw userdata: CreateContext already installs
// a default IME callback, so winit may leave that userdata null.
thread_local! {
    static IME_WINDOW: RefCell<Weak<Window>> = const { RefCell::new(Weak::new()) };
}

/// Shared by window bring-up and the real-frame integration regression.
pub(super) fn attach_platform(
    context: &mut imgui::Context,
    window: &Arc<Window>,
) -> imgui_winit::WinitPlatform {
    let mut platform = imgui_winit::WinitPlatform::new(context);
    platform.attach_window(window, imgui_winit::HiDpiMode::Locked(1.0), context);
    install_physical_ime_callback(context, window);
    platform
}

/// Clear before AppWindow's fields drop. A stale owner cannot clear a replacement.
pub(super) fn clear_window(window: &Arc<Window>) {
    IME_WINDOW.with(|slot| {
        if slot.borrow().as_ptr() == Arc::as_ptr(window) {
            *slot.borrow_mut() = Weak::new();
        }
    });
}

#[cfg(test)]
thread_local! {
    static TEST_SUBMISSIONS: std::cell::RefCell<Vec<(winit::dpi::Position, winit::dpi::Size)>> = const { std::cell::RefCell::new(Vec::new()) };
    static TEST_CALLBACK_COUNT: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
pub(super) fn take_test_submissions() -> Vec<(winit::dpi::Position, winit::dpi::Size)> {
    TEST_SUBMISSIONS.with(|submissions| std::mem::take(&mut *submissions.borrow_mut()))
}

#[cfg(test)]
pub(super) fn take_test_callback_count() -> usize {
    TEST_CALLBACK_COUNT.with(|count| count.replace(0))
}

/// Submit ImGui's platform IME area to the window. ImGui coordinates and the
/// line height are physical pixels under `HiDpiMode::Locked(1.0)`. Winit's
/// physical `Position` and `Size` use integer pixels, so round when adapting.
pub(super) fn submit_imgui_ime_area(
    input_pos: [f32; 2],
    viewport_pos: [f32; 2],
    input_line_height: f32,
    submit: impl FnOnce(winit::dpi::Position, winit::dpi::Size),
) {
    let position = winit::dpi::PhysicalPosition::new(
        (input_pos[0] - viewport_pos[0]).round() as i32,
        (input_pos[1] - viewport_pos[1]).round() as i32,
    );
    let line_height = if input_line_height > 0.0 {
        input_line_height.round() as u32
    } else {
        16
    };
    submit(
        position.into(),
        winit::dpi::PhysicalSize::new(line_height, line_height).into(),
    );
}

/// dear-imgui-rs has no safe IME-data accessor or callback setter.
/// Windows' default handler can use PlatformHandleRaw, but macOS has no such
/// route; use one cross-platform physical-pixel bridge without raw userdata.
fn install_physical_ime_callback(context: &mut imgui::Context, window: &Arc<Window>) {
    IME_WINDOW.with(|slot| *slot.borrow_mut() = Arc::downgrade(window));
    let platform_io = context.platform_io_mut();
    let raw = platform_io.as_raw_mut();
    // SAFETY: `platform_io_mut()` returns the unique, live PlatformIO for this
    // context. This field's ABI matches `physical_ime_set_data`; the function
    // remains available for the context's lifetime. No window pointer is stored.
    unsafe {
        (*raw).Platform_SetImeDataFn = Some(physical_ime_set_data);
    }
}

pub(super) unsafe extern "C" fn physical_ime_set_data(
    context: *mut imgui::sys::ImGuiContext,
    viewport: *mut imgui::sys::ImGuiViewport,
    data: *mut imgui::sys::ImGuiPlatformImeData,
) {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        if context.is_null() || viewport.is_null() || data.is_null() {
            return;
        }
        // SAFETY: Dear ImGui calls this hook synchronously from EndFrame on
        // the UI thread, with live ImGui-owned viewport and IME data pointers.
        // Both are null-checked above and read only during this invocation.
        let (input_pos, viewport_pos, line_height, wants_input) = unsafe {
            let ime = &*data;
            let viewport = &*viewport;
            (
                [ime.InputPos.x, ime.InputPos.y],
                [viewport.Pos.x, viewport.Pos.y],
                ime.InputLineHeight,
                ime.WantVisible || ime.WantTextInput,
            )
        };
        if !wants_input {
            return;
        }
        #[cfg(test)]
        TEST_CALLBACK_COUNT.with(|count| count.set(count.get() + 1));
        // Upgrade owns the Window for the submission; expiry is harmless even
        // if the Window is dropped before the ImGui context. No raw dereference.
        let Some(window) = IME_WINDOW.with(|slot| slot.borrow().upgrade()) else {
            return;
        };
        submit_imgui_ime_area(input_pos, viewport_pos, line_height, |position, size| {
            #[cfg(test)]
            TEST_SUBMISSIONS.with(|submissions| {
                submissions.borrow_mut().push((position, size));
            });
            window.set_ime_cursor_area(position, size);
        });
    }));
    if result.is_err() {
        eprintln!("panel: panic in physical IME callback");
        std::process::abort();
    }
}
