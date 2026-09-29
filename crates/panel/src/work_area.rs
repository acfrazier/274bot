//! Monitor work-area query and the launch / rail-grow fit glue (M-003).
//!
//! One provider feeds both [`fit_window`] paths. macOS and Windows subtract
//! the menu bar / Dock / taskbar; Linux falls back to the full monitor
//! rectangle (documented on [`work_area_for`]).

use winit::dpi::{PhysicalPosition, PhysicalSize};
use winit::event_loop::ActiveEventLoop;
use winit::window::Window;

use super::{clamp_inner_size, clamp_outer_position};

/// Physical-pixel work area in winit's top-left coordinate space.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WorkArea {
    pub origin: (f64, f64),
    pub size: (f64, f64),
}

/// Inner size (physical) and outer position after fitting a frame into a
/// [`WorkArea`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FrameFit {
    pub inner_size: (f64, f64),
    pub outer_position: (f64, f64),
}

/// Fit `requested_inner` into `work`, using the current outer frame so the
/// position clamp sees the title bar / borders and the inner request is the
/// clamped outer size minus that chrome.
pub fn fit_frame_to_work_area(
    work: WorkArea,
    outer_pos: (f64, f64),
    outer_size: (f64, f64),
    inner_size: (f64, f64),
    requested_inner: (f64, f64),
) -> FrameFit {
    let chrome_w = (outer_size.0 - inner_size.0).max(0.0);
    let chrome_h = (outer_size.1 - inner_size.1).max(0.0);
    let requested_outer = (requested_inner.0 + chrome_w, requested_inner.1 + chrome_h);
    let outer = clamp_inner_size(requested_outer, work.size);
    let inner = ((outer.0 - chrome_w).max(1.0), (outer.1 - chrome_h).max(1.0));
    FrameFit {
        inner_size: inner,
        outer_position: clamp_outer_position(outer_pos, outer, work.origin, work.size),
    }
}

/// Convert an AppKit `visibleFrame` (points, bottom-left origin, y up) into
/// winit physical pixels (top-left, y down). `main_height` is the height of
/// the menu-bar screen in points — the same reference winit uses
/// (`CGMainDisplayID` / `NSScreen.screens[0].frame`).
#[cfg_attr(not(any(test, target_os = "macos")), allow(dead_code))]
pub fn cocoa_visible_frame_to_physical(
    origin: (f64, f64),
    size: (f64, f64),
    main_height: f64,
    scale: f64,
) -> WorkArea {
    let y = main_height - size.1 - origin.1;
    let scale = if scale.is_finite() && scale > 0.0 {
        scale
    } else {
        1.0
    };
    WorkArea {
        origin: (origin.0 * scale, y * scale),
        size: (size.0 * scale, size.1 * scale),
    }
}

/// Convert a Win32 `RECT` (`rcWork`) into winit physical pixels. Win32
/// screen coordinates are already top-left physical.
#[cfg_attr(not(any(test, windows)), allow(dead_code))]
pub fn win32_work_rect_to_physical(left: i32, top: i32, right: i32, bottom: i32) -> WorkArea {
    WorkArea {
        origin: (f64::from(left), f64::from(top)),
        size: (
            f64::from(right.saturating_sub(left)),
            f64::from(bottom.saturating_sub(top)),
        ),
    }
}

/// Work area of the monitor that currently hosts `window`, in winit physical
/// pixels (top-left origin).
///
/// - macOS: `NSScreen.visibleFrame` of the window's screen, flipped into
///   winit's coordinate space and scaled by `backingScaleFactor`.
/// - Windows: `MonitorFromWindow` + `GetMonitorInfoW` `rcWork`.
/// - Linux: X11 `_NET_WORKAREA` is one virtual-desktop rectangle (not a
///   per-monitor strut) and Wayland has no work-area query in winit 0.30, so
///   this falls back to the full monitor rect. Menu bar / Dock / taskbar
///   insets are therefore a macOS and Windows behavior.
fn work_area_for(window: &Window, event_loop: Option<&ActiveEventLoop>) -> Option<WorkArea> {
    platform_work_area(window).or_else(|| monitor_rect(window, event_loop))
}

fn monitor_rect(window: &Window, event_loop: Option<&ActiveEventLoop>) -> Option<WorkArea> {
    let monitor = window
        .current_monitor()
        .or_else(|| event_loop.and_then(|el| el.primary_monitor()))
        .or_else(|| event_loop.and_then(|el| el.available_monitors().next()))?;
    let pos = monitor.position();
    let size = monitor.size();
    Some(WorkArea {
        origin: (f64::from(pos.x), f64::from(pos.y)),
        size: (f64::from(size.width), f64::from(size.height)),
    })
}

#[cfg(target_os = "macos")]
fn platform_work_area(window: &Window) -> Option<WorkArea> {
    macos_visible_frame(window)
}

#[cfg(windows)]
fn platform_work_area(window: &Window) -> Option<WorkArea> {
    windows_rc_work(window)
}

#[cfg(not(any(target_os = "macos", windows)))]
fn platform_work_area(_window: &Window) -> Option<WorkArea> {
    None
}

#[cfg(target_os = "macos")]
fn macos_visible_frame(window: &Window) -> Option<WorkArea> {
    use objc2::rc::Retained;
    use objc2_app_kit::{NSScreen, NSView};
    use objc2_foundation::MainThreadMarker;
    use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};

    let mtm = MainThreadMarker::new()?;
    let handle = window.window_handle().ok()?;
    let RawWindowHandle::AppKit(appkit) = handle.as_raw() else {
        return None;
    };
    let ns_view = appkit.ns_view.as_ptr().cast::<NSView>();
    // SAFETY: winit's AppKit handle `ns_view` is the live content view for
    // this `Window`, valid for `window`'s lifetime. `retain` takes our own
    // +1 so the `Retained` can outlive the handle borrow. The panel event
    // loop runs on the main thread (`MainThreadMarker::new` succeeded).
    let view = unsafe { Retained::retain(ns_view) }?;
    let ns_window = view.window()?;
    let screen = ns_window.screen()?;
    let visible = screen.visibleFrame();
    let scale = screen.backingScaleFactor();
    let screens = NSScreen::screens(mtm);
    if screens.count() == 0 {
        return None;
    }
    // SAFETY: `count() > 0`, so index 0 is in range. AppKit's `screens[0]`
    // is the menu-bar display, the same origin winit uses for the y-flip
    // (`CGMainDisplayID`).
    let main = unsafe { screens.objectAtIndex(0) };
    let main_height = main.frame().size.height;
    Some(cocoa_visible_frame_to_physical(
        (visible.origin.x, visible.origin.y),
        (visible.size.width, visible.size.height),
        main_height,
        scale,
    ))
}

#[cfg(windows)]
fn windows_rc_work(window: &Window) -> Option<WorkArea> {
    use std::mem;

    use windows_sys::Win32::Foundation::HWND;
    use windows_sys::Win32::Graphics::Gdi::{
        GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST,
    };
    use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};

    let handle = window.window_handle().ok()?;
    let RawWindowHandle::Win32(win32) = handle.as_raw() else {
        return None;
    };
    let hwnd = win32.hwnd.get() as HWND;
    // SAFETY: `hwnd` is the live Win32 handle for this `Window` (non-null
    // `HasWindowHandle`). `MonitorFromWindow` only reads it.
    let hmonitor = unsafe { MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST) };
    if hmonitor == 0 {
        return None;
    }
    // SAFETY: `MONITORINFO` is a C struct of integers; the all-zero bit
    // pattern is valid. `cbSize` is written before `GetMonitorInfoW` reads it.
    let mut info: MONITORINFO = unsafe { mem::zeroed() };
    info.cbSize = mem::size_of::<MONITORINFO>() as u32;
    // SAFETY: `info` is a `MONITORINFO` with `cbSize` set to its size;
    // `hmonitor` came from `MonitorFromWindow` and is non-null.
    // `GetMonitorInfoW` only writes that struct.
    let ok = unsafe { GetMonitorInfoW(hmonitor, &mut info) };
    if ok == 0 {
        return None;
    }
    let rc = info.rcWork;
    if rc.right <= rc.left || rc.bottom <= rc.top {
        return None;
    }
    Some(win32_work_rect_to_physical(
        rc.left, rc.top, rc.right, rc.bottom,
    ))
}

/// Clamp `window` so its outer frame sits in the work area: shrink the inner
/// size and move the origin. Best-effort: an unknown work area or scale
/// leaves the window as-is.
pub(super) fn fit_window(
    window: &Window,
    requested_logical_inner: (f64, f64),
    event_loop: Option<&ActiveEventLoop>,
) {
    let Some(work) = work_area_for(window, event_loop) else {
        return;
    };
    let scale = window.scale_factor();
    if !scale.is_finite() || scale <= 0.0 {
        return;
    }
    let inner = window.inner_size();
    let outer = window.outer_size();
    let inner_phys = (f64::from(inner.width), f64::from(inner.height));
    let outer_phys = (f64::from(outer.width), f64::from(outer.height));
    let pos = window
        .outer_position()
        .ok()
        .map(|p| (f64::from(p.x), f64::from(p.y)));
    let requested_phys = (
        requested_logical_inner.0 * scale,
        requested_logical_inner.1 * scale,
    );
    let fit = fit_frame_to_work_area(
        work,
        pos.unwrap_or((work.origin.0, work.origin.1)),
        outer_phys,
        inner_phys,
        requested_phys,
    );
    apply_fit(window, fit, inner_phys, pos);
}

/// Rail-grow entry: same provider and glue as launch, without an event loop.
pub(crate) fn fit_window_to_work_area(window: &Window, requested_logical_inner: (f64, f64)) {
    fit_window(window, requested_logical_inner, None);
}

fn apply_fit(window: &Window, fit: FrameFit, inner: (f64, f64), pos: Option<(f64, f64)>) {
    if (fit.inner_size.0 - inner.0).abs() > 1.0 || (fit.inner_size.1 - inner.1).abs() > 1.0 {
        let _ = window.request_inner_size(PhysicalSize::new(
            fit.inner_size.0.round().max(1.0),
            fit.inner_size.1.round().max(1.0),
        ));
    }
    let Some(at) = pos else {
        return;
    };
    if (fit.outer_position.0 - at.0).abs() > 1.0 || (fit.outer_position.1 - at.1).abs() > 1.0 {
        window.set_outer_position(PhysicalPosition::new(
            fit.outer_position.0.round() as i32,
            fit.outer_position.1.round() as i32,
        ));
    }
}
