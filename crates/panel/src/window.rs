//! Panel-owned winit + wgpu + dear-imgui window loop.
//!
//! Instance → window → surface on the UI thread, then the surface-compatible
//! adapter → device/queue on a worker while the event loop keeps pumping,
//! then surface config → imgui context (theme/ini) → platform + renderer,
//! then a per-frame `prepare_frame` → UI body → `prepare_render_with_ui`
//! → render pass → present cycle driven by an `ApplicationHandler` event
//! loop. The window/GPU stack is rebuilt on render errors. An OS-destroyed
//! window (`WindowEvent::Destroyed` for the current window, or
//! `CloseRequested`) drops the stack and exits: winit's X11 backend unwraps
//! `XGetGeometry` (`error_kind: Drawable`, X11 `BadDrawable`) if a later
//! redraw in the same pump still holds the `Window`. Per-frame sizing uses
//! ImGui's cached display size instead of `Window::inner_size`.

use std::fmt;
use std::mem;
use std::num::NonZeroUsize;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, TryRecvError};
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

use crate::headed_record::RECORD_LABEL;
use dear_imgui_rs as imgui;
use dear_imgui_rs::{BackendFlags, ConfigFlags, DockFlags, Id, TextureId, WindowFlags};
use dear_imgui_wgpu as imgui_wgpu;
use dear_imgui_winit as imgui_winit;
use frontend_core::surface::{FrameBuf, FrameStamp};
use pollster::block_on;
use thiserror::Error;
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::{Window, WindowId};

use client::render::backend::{
    begin_window_readback, CaptureGeometry, ConsumerFrame, PendingWindowReadback, ReadbackBudget,
    ReadbackError, ReadbackPermit,
};

#[path = "work_area.rs"]
mod work_area;
pub(crate) use work_area::fit_window_to_work_area;
#[cfg(test)]
pub(crate) use work_area::{fit_window_in, FitTarget, WorkArea};

#[path = "ime.rs"]
mod ime;
use ime::attach_platform;

#[cfg(feature = "render-diagnostics")]
#[path = "window_diagnostics.rs"]
mod window_diagnostics;
#[cfg(feature = "render-diagnostics")]
use window_diagnostics::{UiPaintObservation, WindowDiagnostics};

/// What the panel window needs from the graphics stack, appended to the
/// startup errors a missing driver produces. `BOT_CPU=1` selects the game
/// view's CPU rasterizer; the window itself always presents through wgpu.
#[cfg(target_os = "linux")]
const GRAPHICS_HELP: &str = "the panel window needs Vulkan: the Vulkan loader \
(libvulkan.so.1) and a Vulkan driver (ICD). Install your GPU's Vulkan driver, or \
mesa-vulkan-drivers for Mesa lavapipe software rendering. BOT_CPU=1 does not help: it only \
draws the game view on the CPU";
#[cfg(windows)]
const GRAPHICS_HELP: &str = "the panel window needs a Direct3D 12 or Vulkan driver; update \
the GPU driver. BOT_CPU=1 does not help: it only draws the game view on the CPU";
#[cfg(target_os = "macos")]
const GRAPHICS_HELP: &str = "the panel window needs Metal. BOT_CPU=1 does not help: it only \
draws the game view on the CPU";
#[cfg(not(any(target_os = "linux", windows, target_os = "macos")))]
const GRAPHICS_HELP: &str = "the panel window needs a Vulkan driver (ICD). BOT_CPU=1 does not \
help: it only draws the game view on the CPU";

/// Panel application or window-loop error.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum PanelError {
    #[error("server profile: {0}")]
    ServerProfile(String),
    #[error("event loop error: {0}")]
    EventLoop(#[from] winit::error::EventLoopError),
    #[error("live harness failed: {0}")]
    LiveHarnessFailure(String),
    #[error("memory run failed: {0}")]
    MemoryRunFailure(String),
    #[error("startup failed: {0}")]
    StartupFailure(String),
    #[error("window creation failed: {0}")]
    WindowCreation(#[source] winit::error::OsError),
    #[error("WGPU surface creation failed: {0}; {help}", help = GRAPHICS_HELP)]
    SurfaceCreation(#[source] wgpu::CreateSurfaceError),
    #[error("no suitable WGPU adapter found: {0}; {help}", help = GRAPHICS_HELP)]
    AdapterUnavailable(#[source] wgpu::RequestAdapterError),
    #[error("WGPU device request failed: {0}; {help}", help = GRAPHICS_HELP)]
    DeviceRequest(#[source] wgpu::RequestDeviceError),
    #[error("GPU initialization thread could not start: {0}")]
    GpuThread(#[source] std::io::Error),
    #[error("GPU initialization thread ended without an adapter or device")]
    GpuInitLost,
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
pub struct ShotCapture {
    pub label: String,
    pub snapshot_json: String,
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
    /// Paint sequence whose complete window texture was copied.
    pub paint_sequence: u64,
    /// Frame authority held for an actor-tagged window capture.
    pub actor_stamp: Option<FrameStamp>,
    /// Copied from the readback job; never the current LIVE camera.
    #[cfg(feature = "render-diagnostics")]
    pub pixel_roi: Option<client::render::diagnostics::PixelRoiShotBind>,
    /// Keeps the process readback slot charged through output handoff.
    pub(crate) readback_permit: Option<ReadbackPermit>,
}

impl fmt::Debug for ShotCapture {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut capture = formatter.debug_struct("ShotCapture");
        capture
            .field("label", &self.label)
            .field("snapshot_json", &self.snapshot_json)
            .field("width", &self.width)
            .field("height", &self.height)
            .field("rgba", &self.rgba)
            .field("paint_sequence", &self.paint_sequence)
            .field("actor_stamp", &self.actor_stamp);
        #[cfg(feature = "render-diagnostics")]
        capture.field("pixel_roi", &self.pixel_roi);
        capture
            .field("readback_permit_held", &self.readback_permit.is_some())
            .finish()
    }
}

impl ShotCapture {
    fn metadata_bytes(&self) -> usize {
        let mut bytes = self.snapshot_json.capacity();
        if self.actor_stamp.is_some() {
            bytes = bytes.saturating_add(mem::size_of::<FrameStamp>());
        }
        #[cfg(feature = "render-diagnostics")]
        if let Some(pixel_roi) = self.pixel_roi.as_ref() {
            bytes = bytes.saturating_add(pixel_roi_metadata_bytes(pixel_roi));
        }
        bytes
    }
}
fn discard_capture_for_failure(capture: ShotCapture) -> String {
    let ShotCapture {
        label,
        snapshot_json,
        width: _,
        height: _,
        rgba,
        paint_sequence: _,
        actor_stamp: _,
        #[cfg(feature = "render-diagnostics")]
        pixel_roi,
        readback_permit,
    } = capture;
    drop(snapshot_json);
    drop(rgba);
    #[cfg(feature = "render-diagnostics")]
    drop(pixel_roi);
    drop(readback_permit);
    label
}

/// Identity and retained pixels accepted together for one actor capture.
pub struct ActorCaptureFence {
    stamp: FrameStamp,
    mailbox: Weak<FrameBuf>,
    consumer: Option<ConsumerFrame>,
    staged: bool,
}

impl ActorCaptureFence {
    pub fn new(
        stamp: FrameStamp,
        mailbox: Weak<FrameBuf>,
        consumer: Option<ConsumerFrame>,
    ) -> Result<Self, CaptureError> {
        if !stamp.has_valid_scene() {
            return Err(CaptureError::InvalidScene);
        }
        match (stamp.present_token(), consumer.as_ref()) {
            (None, None) => {}
            (Some(token), Some(frame))
                if frame.present_token() == token && frame.has_valid_scene() => {}
            (Some(_), Some(_)) => return Err(CaptureError::StaleHead),
            _ => return Err(CaptureError::StaleHead),
        }
        let fence = Self {
            stamp,
            mailbox,
            consumer,
            staged: false,
        };
        fence.validate_head()?;
        Ok(fence)
    }

    pub const fn stamp(&self) -> FrameStamp {
        self.stamp
    }

    pub fn consumer_frame(&self) -> Option<&ConsumerFrame> {
        self.consumer.as_ref()
    }

    fn validate_head(&self) -> Result<(), CaptureError> {
        self.mailbox
            .upgrade()
            .filter(|mailbox| mailbox.head() == Some(self.stamp.head()))
            .map(|_| ())
            .ok_or(CaptureError::StaleHead)
    }

    fn validate_before_staging(&self) -> Result<(), CaptureError> {
        if self.staged {
            return Err(CaptureError::StaleHead);
        }
        self.validate_head()?;
        match (self.stamp.present_token(), self.consumer.as_ref()) {
            (None, None) => Ok(()),
            (Some(token), Some(frame))
                if frame.present_token() == token && frame.has_valid_scene() =>
            {
                Ok(())
            }
            _ => Err(CaptureError::StaleHead),
        }
    }

    fn validate_after_staging(&self) -> Result<(), CaptureError> {
        if !self.staged || self.consumer.is_some() {
            return Err(CaptureError::StaleHead);
        }
        self.validate_head()
    }

    fn release_consumer_after_staging(&mut self) {
        self.staged = true;
        drop(self.consumer.take());
    }

    fn metadata_bytes(&self) -> usize {
        mem::size_of::<Self>()
    }
}

struct AcceptedActorCapture {
    actor: String,
    fence: ActorCaptureFence,
}

struct CaptureStamp {
    paint_sequence: u64,
    #[cfg(feature = "render-diagnostics")]
    pixel_roi: Option<client::render::diagnostics::PixelRoiShotBind>,
}

impl CaptureStamp {
    fn metadata_bytes(&self) -> usize {
        mem::size_of::<Self>().saturating_add({
            #[cfg(feature = "render-diagnostics")]
            {
                self.pixel_roi.as_ref().map_or(0, pixel_roi_metadata_bytes)
            }
            #[cfg(not(feature = "render-diagnostics"))]
            {
                0
            }
        })
    }
}

#[cfg(feature = "render-diagnostics")]
fn pixel_roi_metadata_bytes(bind: &client::render::diagnostics::PixelRoiShotBind) -> usize {
    bind.meta
        .upload
        .as_ref()
        .map_or(0, |upload| upload.slot.capacity())
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
    FailedCapture(CaptureError),
}

/// Capture refusal at the bounded panel boundary.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Error)]
pub enum CaptureError {
    #[error("capture queue is full")]
    QueueFull,
    #[error("capture exceeds the configured byte envelope")]
    TooLarge,
    #[error("the process readback slot is busy")]
    Busy,
    #[error("capture deadline expired")]
    Expired,
    #[error("capture deadline overflows Instant")]
    DeadlineOverflow,
    #[error("capture policy contains an invalid zero duration")]
    InvalidPolicy,
    #[error("capture request is absent")]
    MissingActorRequest,
    #[error("capture belongs to a stale head")]
    StaleHead,
    #[error("capture requires a valid scene")]
    InvalidScene,
    #[error("retained consumer refused capture: {0:?}")]
    Readback(ReadbackError),
    #[error("capture output failed")]
    Io,
    #[error("capture device operation failed")]
    Device,
    #[error("capture requires explicitly enabled fair rendering")]
    Inactive,
    #[error("capture policy is not configured")]
    PolicyMissing,
    #[error("window paint sequence exhausted")]
    SequenceExhausted,
    #[error("capture request has no explicit deadline")]
    DeadlineMissing,
}

#[derive(Clone, Copy)]
struct CapturePolicy {
    metadata_limit: NonZeroUsize,
    readback_timeout: Duration,
}

struct ActiveCapture {
    label: Option<String>,
    snapshot_json: String,
    deadline: Instant,
    paint_sequence: u64,
    recording: bool,
    actor_capture: bool,
    stamp: Option<CaptureStamp>,
}
/// One labeled capture request. Pair terminal shots carry the actor that
/// must be the selected visible main actor before this job is promoted
/// into whole-window readback.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShotRequest {
    pub label: String,
    pub snapshot_json: String,
    pub actor: Option<String>,
    deadline: Option<Instant>,
}

/// Whole-window capture coordination between the scenario sink (slot
/// thread), the UI body (per-frame drain), the render pass (readback),
/// and the panel frame (file write).
#[derive(Default)]
pub struct ShotState {
    capture_policy: Option<CapturePolicy>,
    wanted_deadlines: Vec<Option<Instant>>,
    active_capture: Option<ActiveCapture>,
    actor_capture: Option<AcceptedActorCapture>,
    actor_display_confirmed: bool,
    actor_display_error: Option<CaptureError>,
    record_deadline: Option<Instant>,
    /// Next recorder or benchmark UI poll, distinct from readback completion.
    ui_work_deadline: Option<Instant>,
    record_drops: u64,
    /// Requests pushed by the scenario sink / pair terminal path, drained
    /// by the UI body.
    pub requests: Vec<ShotRequest>,
    /// The UI body's per-frame drain: what the next render pass captures.
    pub wanted: Vec<(String, String)>,
    /// Captures completed by the render pass, consumed by the UI body.
    pub done: Vec<ShotCapture>,
    /// Most recent terminal outcomes (unbounded in legacy mode).
    terminal: Vec<TerminalStatus>,
    /// Active asynchronous write text, charged until a terminal outcome.
    write_pending_label: Option<String>,
    write_pending_bytes: usize,
    /// The panel retains its run directory after individual writes finish.
    output_directory_bytes: usize,
    #[cfg(feature = "render-diagnostics")]
    write_pending_diagnostic_charged: bool,
    /// Headed recording (`HEADED_RECORD=1`): set by the UI body when the
    /// frame pacer fires. The next render pass stages one extra readback
    /// from the same just-rendered texture; the capture lands in
    /// `record_done`, never in the PNG ledger.
    record_armed: bool,
    /// Completed recording frames, consumed by the recorder pump.
    pub record_done: Vec<ShotCapture>,
}

struct TerminalStatus {
    label: String,
    outcome: TerminalOutcome,
}

enum TerminalOutcome {
    Written,
    Failed(String),
    CaptureFailed(CaptureError),
}

impl ShotState {
    /// Enable bounded capture only after fair rendering has been explicitly
    /// enabled and a capture policy has been configured.
    pub fn configure_capture_policy(
        &mut self,
        metadata_limit: NonZeroUsize,
        readback_timeout: Duration,
    ) -> Result<(), CaptureError> {
        if !render_fair_capture_enabled() {
            return Err(CaptureError::Inactive);
        }
        if readback_timeout.is_zero() {
            return Err(CaptureError::InvalidPolicy);
        }
        if self.capture_policy.is_some()
            || !self.requests.is_empty()
            || !self.wanted.is_empty()
            || self.active_capture.is_some()
            || self.actor_capture.is_some()
            || !self.done.is_empty()
            || self.write_pending_label.is_some()
            || !self.record_done.is_empty()
            || !self.terminal.is_empty()
            || self.record_armed
            || self.record_deadline.is_some()
        {
            return Err(CaptureError::Busy);
        }
        self.capture_policy = Some(CapturePolicy {
            metadata_limit,
            readback_timeout,
        });
        Ok(())
    }

    pub fn capture_deadline(&self, now: Instant) -> Result<Instant, CaptureError> {
        if !render_fair_capture_enabled() {
            return Err(CaptureError::Inactive);
        }
        let policy = self.capture_policy.ok_or(CaptureError::PolicyMissing)?;
        if policy.readback_timeout.is_zero() {
            return Err(CaptureError::InvalidPolicy);
        }
        now.checked_add(policy.readback_timeout)
            .ok_or(CaptureError::DeadlineOverflow)
    }

    /// Serialize an untagged request only inside its complete byte allowance.
    pub(crate) fn enqueue_bounded_snapshot(
        &mut self,
        label: &str,
        build: impl FnOnce(usize) -> Result<String, CaptureError>,
    ) -> Result<(), CaptureError> {
        self.enqueue_bounded_request(label, None, build)
    }

    /// Actor snapshots are rebuilt after a real stamped frame is accepted.
    /// Do not allocate a speculative JSON snapshot while waiting for that frame.
    pub(crate) fn enqueue_bounded_actor_request(
        &mut self,
        label: &str,
        actor: &str,
    ) -> Result<(), CaptureError> {
        self.enqueue_bounded_request(label, Some(actor), |_| Ok(String::new()))
    }

    fn enqueue_bounded_request(
        &mut self,
        label: &str,
        actor: Option<&str>,
        build: impl FnOnce(usize) -> Result<String, CaptureError>,
    ) -> Result<(), CaptureError> {
        let deadline = self.capture_deadline(Instant::now())?;
        if self.requests.len() >= CAPTURE_QUEUE {
            return Err(CaptureError::QueueFull);
        }
        if label.len() > CAPTURE_NAME_BYTES
            || actor.is_some_and(|actor| actor.len() > CAPTURE_NAME_BYTES)
        {
            return Err(CaptureError::TooLarge);
        }
        let policy = self.capture_policy.ok_or(CaptureError::PolicyMissing)?;
        let remaining = policy
            .metadata_limit
            .get()
            .checked_sub(self.metadata_bytes())
            .ok_or(CaptureError::TooLarge)?;
        let label = copy_text_with_capacity(label, remaining)?;
        let remaining = remaining
            .checked_sub(label.capacity())
            .ok_or(CaptureError::TooLarge)?;
        let actor = actor
            .map(|actor| copy_text_with_capacity(actor, remaining))
            .transpose()?;
        let maximum = remaining
            .checked_sub(actor.as_ref().map_or(0, String::capacity))
            .ok_or(CaptureError::TooLarge)?;
        let snapshot_json = build(maximum)?;
        if snapshot_json.capacity() > maximum {
            return Err(CaptureError::TooLarge);
        }
        self.enqueue_request(ShotRequest {
            label,
            snapshot_json,
            actor,
            deadline: Some(deadline),
        })
    }

    /// Replace the retained directory under the metadata lock. During the
    /// builder call the old and new paths overlap only within the configured
    /// metadata allowance.
    pub(crate) fn replace_output_directory(
        &mut self,
        directory: &mut Option<std::path::PathBuf>,
        build: impl FnOnce(usize) -> Result<std::path::PathBuf, CaptureError>,
    ) -> Result<(), CaptureError> {
        if !render_fair_capture_enabled() {
            return Err(CaptureError::Inactive);
        }
        let policy = self.capture_policy.ok_or(CaptureError::PolicyMissing)?;
        let maximum = policy
            .metadata_limit
            .get()
            .checked_sub(self.metadata_bytes())
            .ok_or(CaptureError::TooLarge)?;
        let path = build(maximum)?;
        if path.capacity() > maximum {
            return Err(CaptureError::TooLarge);
        }
        let bytes = path.capacity();
        *directory = Some(path);
        self.output_directory_bytes = bytes;
        Ok(())
    }

    pub fn enqueue(&mut self, label: String, snapshot_json: String) -> Result<(), CaptureError> {
        self.enqueue_request(ShotRequest {
            label,
            snapshot_json,
            actor: None,
            deadline: None,
        })
    }

    pub fn enqueue_with_deadline(
        &mut self,
        label: String,
        snapshot_json: String,
        deadline: Instant,
    ) -> Result<(), CaptureError> {
        self.enqueue_request(ShotRequest {
            label,
            snapshot_json,
            actor: None,
            deadline: Some(deadline),
        })
    }

    pub fn enqueue_for_actor(
        &mut self,
        label: String,
        snapshot_json: String,
        actor: String,
    ) -> Result<(), CaptureError> {
        self.enqueue_request(ShotRequest {
            label,
            snapshot_json,
            actor: Some(actor),
            deadline: None,
        })
    }

    pub fn enqueue_for_actor_with_deadline(
        &mut self,
        label: String,
        snapshot_json: String,
        actor: String,
        deadline: Instant,
    ) -> Result<(), CaptureError> {
        self.enqueue_request(ShotRequest {
            label,
            snapshot_json,
            actor: Some(actor),
            deadline: Some(deadline),
        })
    }

    fn enqueue_request(&mut self, request: ShotRequest) -> Result<(), CaptureError> {
        if render_fair_capture_enabled() {
            let policy = self.capture_policy.ok_or(CaptureError::PolicyMissing)?;
            if request.deadline.is_none() {
                return Err(CaptureError::DeadlineMissing);
            }
            if request
                .deadline
                .is_some_and(|deadline| deadline <= Instant::now())
            {
                return Err(CaptureError::Expired);
            }
            if self.requests.len() >= CAPTURE_QUEUE {
                return Err(CaptureError::QueueFull);
            }
            if request.label.len() > CAPTURE_NAME_BYTES
                || request
                    .actor
                    .as_ref()
                    .is_some_and(|actor| actor.len() > CAPTURE_NAME_BYTES)
                || self
                    .metadata_bytes()
                    .checked_add(request.metadata_bytes())
                    .is_none_or(|bytes| bytes > policy.metadata_limit.get())
            {
                return Err(CaptureError::TooLarge);
            }
            self.requests
                .try_reserve_exact(1)
                .map_err(|_| CaptureError::QueueFull)?;
        }
        self.terminal.retain(|status| status.label != request.label);
        self.requests.push(request);
        Ok(())
    }

    /// Promote untagged jobs. Actor-tagged jobs require an accepted stamped
    /// frame through [`Self::accept_actor_frame`].
    pub fn promote_requests(&mut self) -> usize {
        self.promote_ready(None, None)
    }

    /// Promote untagged jobs. The legacy path retains its actor-name
    /// readiness behavior; the gated path requires an accepted frame fence.
    pub fn promote_ready(&mut self, focused: Option<&str>, presented: Option<&str>) -> usize {
        if render_fair_capture_enabled() {
            if self.actor_capture.is_some() {
                return 0;
            }
            let has_actor = self.requests.iter().any(|request| request.actor.is_some());
            let mut count = 0;
            let mut index = 0;
            while index < self.requests.len() {
                let promote = self.requests[index].actor.is_none() && !has_actor;
                if !promote
                    || self.wanted.len() >= CAPTURE_QUEUE
                    || self.wanted.try_reserve_exact(1).is_err()
                    || self.wanted_deadlines.try_reserve_exact(1).is_err()
                {
                    index += 1;
                    continue;
                }
                let request = self.requests.remove(index);
                self.wanted.push((request.label, request.snapshot_json));
                self.wanted_deadlines.push(request.deadline);
                count += 1;
            }
            return count;
        }
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

    /// First actor-tagged job still waiting for a presented frame.
    pub fn pending_actor_request(&self) -> Option<&ShotRequest> {
        self.requests.iter().find(|request| request.actor.is_some())
    }

    /// Accept the selected actor's exact presented frame with a bounded
    /// snapshot builder. It receives the remaining byte allowance while the
    /// queued JSON and frame fence are still charged; returned capacity must
    /// fit within that allowance.
    pub fn accept_actor_frame(
        &mut self,
        actor: &str,
        build_snapshot: impl FnOnce(usize) -> Result<String, CaptureError>,
        fence: ActorCaptureFence,
    ) -> Result<(), CaptureError> {
        if !render_fair_capture_enabled() {
            return Err(CaptureError::Inactive);
        }
        let policy = self.capture_policy.ok_or(CaptureError::PolicyMissing)?;
        if actor.len() > CAPTURE_NAME_BYTES {
            return Err(CaptureError::TooLarge);
        }
        let index = self
            .requests
            .iter()
            .position(|request| request.actor.as_deref() == Some(actor))
            .ok_or(CaptureError::MissingActorRequest)?;
        if self.actor_capture.is_some()
            || !self.wanted.is_empty()
            || self.active_capture.is_some()
            || !self.done.is_empty()
            || self.write_pending_label.is_some()
            || !self.record_done.is_empty()
        {
            return Err(CaptureError::Busy);
        }
        fence.validate_before_staging()?;
        let request = &self.requests[index];
        let deadline = request.deadline.ok_or(CaptureError::DeadlineMissing)?;
        if deadline <= Instant::now() {
            return Err(CaptureError::Expired);
        }
        if self.wanted.len() >= CAPTURE_QUEUE
            || self.wanted.try_reserve_exact(1).is_err()
            || self.wanted_deadlines.try_reserve_exact(1).is_err()
        {
            return Err(CaptureError::QueueFull);
        }
        let used_with_fence = self
            .metadata_bytes()
            .checked_add(fence.metadata_bytes())
            .ok_or(CaptureError::TooLarge)?;
        let snapshot_allowance = policy
            .metadata_limit
            .get()
            .checked_sub(used_with_fence)
            .ok_or(CaptureError::TooLarge)?;
        let snapshot_json = build_snapshot(snapshot_allowance)?;
        if snapshot_json.capacity() > snapshot_allowance {
            return Err(CaptureError::TooLarge);
        }
        let new_total = self
            .metadata_bytes()
            .checked_sub(request.snapshot_json.capacity())
            .and_then(|bytes| bytes.checked_add(snapshot_json.capacity()))
            .and_then(|bytes| bytes.checked_add(fence.metadata_bytes()))
            .ok_or(CaptureError::TooLarge)?;
        if new_total > policy.metadata_limit.get() {
            return Err(CaptureError::TooLarge);
        }
        let request = self.requests.remove(index);
        let actor = request.actor.ok_or(CaptureError::MissingActorRequest)?;
        self.wanted.push((request.label, snapshot_json));
        self.wanted_deadlines.push(request.deadline);
        self.actor_capture = Some(AcceptedActorCapture { actor, fence });
        self.actor_display_confirmed = false;
        self.actor_display_error = None;
        Ok(())
    }

    /// The consumer frame the UI must bind for this accepted actor shot.
    pub fn actor_consumer_frame(&self, actor: &str) -> Option<&ConsumerFrame> {
        self.actor_capture
            .as_ref()
            .filter(|accepted| accepted.actor == actor)
            .and_then(|accepted| accepted.fence.consumer_frame())
    }

    /// Confirm that the accepted actor's exact frame was submitted to the
    /// focused ImGui image during this UI paint.
    pub fn confirm_actor_display(
        &mut self,
        actor: &str,
        stamp: Option<FrameStamp>,
    ) -> Result<(), CaptureError> {
        self.actor_display_confirmed = false;
        self.actor_display_error = Some(CaptureError::StaleHead);
        let accepted = self
            .actor_capture
            .as_ref()
            .ok_or(CaptureError::MissingActorRequest)?;
        if accepted.actor != actor {
            return Err(CaptureError::StaleHead);
        }
        let stamp = stamp.ok_or(CaptureError::StaleHead)?;
        if !stamp.has_valid_scene() {
            self.actor_display_error = Some(CaptureError::InvalidScene);
            return Err(CaptureError::InvalidScene);
        }
        if stamp != accepted.fence.stamp {
            return Err(CaptureError::StaleHead);
        }
        self.actor_display_confirmed = true;
        self.actor_display_error = None;
        Ok(())
    }

    fn reset_actor_display_confirmation(&mut self) {
        self.actor_display_confirmed = false;
        self.actor_display_error = None;
    }

    fn validate_accepted_actor_for_staging(&self) -> Result<(), CaptureError> {
        if !self.actor_display_confirmed {
            return Err(self.actor_display_error.unwrap_or(CaptureError::StaleHead));
        }
        self.actor_capture
            .as_ref()
            .ok_or(CaptureError::StaleHead)?
            .fence
            .validate_before_staging()
    }

    fn validate_active_actor_for_staging(&self) -> Result<(), CaptureError> {
        let active = self
            .active_capture
            .as_ref()
            .ok_or(CaptureError::StaleHead)?;
        if active.actor_capture {
            self.validate_accepted_actor_for_staging()
        } else {
            Ok(())
        }
    }

    fn release_actor_consumer_after_staging(&mut self) -> Result<(), CaptureError> {
        let active = self
            .active_capture
            .as_ref()
            .ok_or(CaptureError::StaleHead)?;
        if active.actor_capture {
            self.actor_capture
                .as_mut()
                .ok_or(CaptureError::StaleHead)?
                .fence
                .release_consumer_after_staging();
        }
        Ok(())
    }

    pub fn holds_actor_frame(&self, actor: &str) -> bool {
        self.actor_capture
            .as_ref()
            .is_some_and(|accepted| accepted.actor == actor && !accepted.fence.staged)
    }

    pub fn capture_in_flight(&self) -> bool {
        !self.wanted.is_empty()
            || !self.done.is_empty()
            || self.active_capture.is_some()
            || self.actor_capture.is_some()
            || self.write_pending_label.is_some()
    }

    pub fn set_ui_work_deadline(&mut self, deadline: Option<Instant>) {
        self.ui_work_deadline = if render_fair_capture_enabled() {
            deadline
        } else {
            None
        };
    }

    /// Arm a legacy recording frame. Gated capture requires an explicit
    /// caller-owned deadline through `arm_record_with_deadline`.
    pub fn arm_record(&mut self) -> Result<(), CaptureError> {
        if render_fair_capture_enabled() {
            return Err(if self.capture_policy.is_some() {
                CaptureError::DeadlineMissing
            } else {
                CaptureError::PolicyMissing
            });
        }
        self.record_armed = true;
        Ok(())
    }

    pub fn arm_record_with_deadline(&mut self, deadline: Instant) -> Result<(), CaptureError> {
        if !render_fair_capture_enabled() {
            return Err(CaptureError::Inactive);
        }
        if self.capture_policy.is_none() {
            return Err(CaptureError::PolicyMissing);
        }
        if deadline <= Instant::now() {
            return Err(CaptureError::Expired);
        }
        self.record_armed = true;
        self.record_deadline = Some(deadline);
        Ok(())
    }

    /// Take completed recording frames for the recorder pump.
    pub fn take_record(&mut self) -> Vec<ShotCapture> {
        std::mem::take(&mut self.record_done)
    }

    /// Drain asynchronous recorder refusals so the recorder can count each
    /// not-yet-accepted opportunity.
    pub fn take_record_drops(&mut self) -> u64 {
        mem::take(&mut self.record_drops)
    }

    /// True when the next render pass must stage a readback.
    pub fn capture_pending(&self) -> bool {
        !self.wanted.is_empty() || self.record_armed
    }

    /// Replace the sidecar on a still-queued actor job so the readback
    /// carries the current scene2 snapshot, not the JSON frozen at enqueue.
    pub fn refresh_actor_sidecar(
        &mut self,
        actor: &str,
        snapshot_json: String,
    ) -> Result<(), CaptureError> {
        let Some(index) = self
            .requests
            .iter()
            .position(|request| request.actor.as_deref() == Some(actor))
        else {
            return Ok(());
        };
        if render_fair_capture_enabled() {
            let policy = self.capture_policy.ok_or(CaptureError::PolicyMissing)?;
            let old_bytes = self.requests[index].snapshot_json.capacity();
            let new_total = self
                .metadata_bytes()
                .saturating_sub(old_bytes)
                .checked_add(snapshot_json.capacity())
                .ok_or(CaptureError::TooLarge)?;
            if snapshot_json.capacity() > policy.metadata_limit.get()
                || new_total > policy.metadata_limit.get()
            {
                return Err(CaptureError::TooLarge);
            }
        }
        self.requests[index].snapshot_json = snapshot_json;
        Ok(())
    }

    /// Drop pending work for `labels` and retain a terminal failure so a
    /// later unready buffer cannot write after the drain/error.
    pub fn fail_labels(&mut self, labels: &[impl AsRef<str>], error: &str) {
        self.cancel_capture_requests(|candidate| {
            labels.iter().any(|label| label.as_ref() == candidate)
        });
        for label in labels {
            self.mark_failed(label.as_ref(), error);
        }
    }

    pub(crate) fn fail_capture_label(
        &mut self,
        label: &str,
        error: CaptureError,
    ) -> Result<(), CaptureError> {
        self.cancel_capture_requests(|candidate| candidate == label);
        self.mark_failed_capture(label, error)
    }

    fn cancel_capture_requests(&mut self, matches_label: impl Fn(&str) -> bool) {
        let failed_actor_capture = self.actor_capture.is_some()
            && (self.wanted.iter().any(|(wanted, _)| matches_label(wanted))
                || self
                    .active_capture
                    .as_ref()
                    .and_then(|active| active.label.as_ref())
                    .is_some_and(|active| matches_label(active)));
        self.requests
            .retain(|request| !matches_label(&request.label));
        self.wanted_deadlines.resize(self.wanted.len(), None);
        let mut index = 0;
        while index < self.wanted.len() {
            if matches_label(&self.wanted[index].0) {
                self.wanted.remove(index);
                self.wanted_deadlines.remove(index);
            } else {
                index += 1;
            }
        }
        self.done.retain(|capture| !matches_label(&capture.label));
        if self
            .active_capture
            .as_ref()
            .and_then(|active| active.label.as_ref())
            .is_some_and(|active| matches_label(active))
        {
            self.active_capture = None;
        }
        if failed_actor_capture {
            self.actor_capture = None;
            self.reset_actor_display_confirmation();
        }
    }

    pub fn mark_written(&mut self, label: &str) {
        if !render_fair_capture_enabled() {
            self.clear_write_pending(label);
            self.terminal.retain(|status| status.label != label);
            self.terminal.push(TerminalStatus {
                label: label.to_string(),
                outcome: TerminalOutcome::Written,
            });
            self.trim_terminal_metadata();
            return;
        }
        let Some(_) = self.capture_policy else {
            self.clear_write_pending(label);
            return;
        };
        let pending_label = self.take_write_pending_label(label);
        let bytes = pending_label.as_ref().map_or(label.len(), String::capacity);
        let Ok(available) = self.admit_terminal_label(label, bytes) else {
            return;
        };
        let owned_label = match pending_label {
            Some(label) => label,
            None => match copy_text_with_capacity(label, available) {
                Ok(label) => label,
                Err(_) => return,
            },
        };
        self.terminal.push(TerminalStatus {
            label: owned_label,
            outcome: TerminalOutcome::Written,
        });
    }

    pub fn mark_failed(&mut self, label: &str, error: &str) {
        if !render_fair_capture_enabled() {
            self.clear_write_pending(label);
            self.terminal.retain(|status| status.label != label);
            self.terminal.push(TerminalStatus {
                label: label.to_string(),
                outcome: TerminalOutcome::Failed(error.to_string()),
            });
            self.trim_terminal_metadata();
            return;
        }
        let Some(policy) = self.capture_policy else {
            self.clear_write_pending(label);
            return;
        };
        let pending_label = self.take_write_pending_label(label);
        let bytes = pending_label.as_ref().map_or(label.len(), String::capacity);
        let Ok(available) = self.admit_terminal_label(label, bytes) else {
            return;
        };
        let owned_label = match pending_label {
            Some(label) => label,
            None => match copy_text_with_capacity(label, available) {
                Ok(label) => label,
                Err(_) => return,
            },
        };
        let available = policy
            .metadata_limit
            .get()
            .saturating_sub(self.metadata_bytes().saturating_add(owned_label.capacity()));
        let bounded_error = truncate_utf8(error, available);
        let owned_error = match copy_text_with_capacity(bounded_error, available) {
            Ok(error) => error,
            Err(_) => return,
        };
        self.terminal.push(TerminalStatus {
            label: owned_label,
            outcome: TerminalOutcome::Failed(owned_error),
        });
    }
    pub fn fail_actor_frame(
        &mut self,
        actor: &str,
        error: CaptureError,
    ) -> Result<(), CaptureError> {
        if !render_fair_capture_enabled() {
            return Err(CaptureError::Inactive);
        }
        if self.capture_policy.is_none() {
            return Err(CaptureError::PolicyMissing);
        }
        let index = self
            .requests
            .iter()
            .position(|request| request.actor.as_deref() == Some(actor))
            .ok_or(CaptureError::MissingActorRequest)?;
        let request = self.requests.remove(index);
        if let Err(error) = self.mark_failed_capture(&request.label, error) {
            self.requests.insert(index, request);
            return Err(error);
        }
        Ok(())
    }

    pub(crate) fn mark_failed_capture(
        &mut self,
        label: &str,
        error: CaptureError,
    ) -> Result<(), CaptureError> {
        self.capture_policy.ok_or(CaptureError::PolicyMissing)?;
        let pending_label = self.take_write_pending_label(label);
        let bytes = pending_label.as_ref().map_or(label.len(), String::capacity);
        let available = self.admit_terminal_label(label, bytes)?;
        let owned_label = match pending_label {
            Some(label) => label,
            None => copy_text_with_capacity(label, available)?,
        };
        self.terminal.push(TerminalStatus {
            label: owned_label,
            outcome: TerminalOutcome::CaptureFailed(error),
        });
        Ok(())
    }

    /// Evict old terminal owners before allocating their replacement. Reserving
    /// after the queue check also keeps the terminal vector at eight entries.
    fn admit_terminal_label(&mut self, label: &str, bytes: usize) -> Result<usize, CaptureError> {
        let policy = self.capture_policy.ok_or(CaptureError::PolicyMissing)?;
        if label.len() > CAPTURE_NAME_BYTES || bytes > policy.metadata_limit.get() {
            return Err(CaptureError::TooLarge);
        }
        self.terminal.retain(|status| status.label != label);
        while self.terminal.len() >= CAPTURE_QUEUE
            || self
                .metadata_bytes()
                .checked_add(bytes)
                .and_then(|total| {
                    total.checked_add(
                        self.terminal
                            .len()
                            .saturating_add(1)
                            .saturating_sub(self.terminal.capacity())
                            .saturating_mul(mem::size_of::<TerminalStatus>()),
                    )
                })
                .is_none_or(|total| total > policy.metadata_limit.get())
        {
            if self.terminal.is_empty() {
                return Err(CaptureError::TooLarge);
            }
            self.terminal.remove(0);
        }
        self.terminal
            .try_reserve_exact(1)
            .map_err(|_| CaptureError::QueueFull)?;
        policy
            .metadata_limit
            .get()
            .checked_sub(self.metadata_bytes())
            .filter(|available| *available >= bytes)
            .ok_or(CaptureError::TooLarge)
    }

    pub fn status(&self, label: &str) -> ShotStatus {
        if let Some(status) = self
            .terminal
            .iter()
            .rev()
            .find(|status| status.label == label)
        {
            return match &status.outcome {
                TerminalOutcome::Written => ShotStatus::Written,
                TerminalOutcome::Failed(error) => ShotStatus::Failed(error.clone()),
                TerminalOutcome::CaptureFailed(error) => ShotStatus::FailedCapture(*error),
            };
        }
        if self.write_pending_label.as_deref() == Some(label)
            || self.done.iter().any(|capture| capture.label == label)
        {
            return ShotStatus::WritePending;
        }
        if self
            .active_capture
            .as_ref()
            .is_some_and(|active| !active.recording && active.label.as_deref() == Some(label))
            || self.wanted.iter().any(|(wanted, _)| wanted == label)
        {
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

    fn clear_write_pending(&mut self, label: &str) {
        if self.write_pending_label.as_deref() == Some(label) {
            self.write_pending_label = None;
            self.write_pending_bytes = 0;
            #[cfg(feature = "render-diagnostics")]
            {
                self.write_pending_diagnostic_charged = false;
            }
        }
    }

    fn take_write_pending_label(&mut self, label: &str) -> Option<String> {
        if self.write_pending_label.as_deref() == Some(label) {
            self.write_pending_bytes = 0;
            #[cfg(feature = "render-diagnostics")]
            {
                self.write_pending_diagnostic_charged = false;
            }
            self.write_pending_label.take()
        } else {
            None
        }
    }

    #[cfg(feature = "render-diagnostics")]
    /// Clone the pending writer label only when its actual capacity fits the
    /// remaining budget.
    pub(crate) fn try_clone_pending_diagnostic_label(
        &mut self,
        label: &str,
    ) -> Result<String, CaptureError> {
        if !render_fair_capture_enabled() {
            return Err(CaptureError::Inactive);
        }
        let policy = self.capture_policy.ok_or(CaptureError::PolicyMissing)?;
        if self.write_pending_label.as_deref() != Some(label) {
            return Err(CaptureError::StaleHead);
        }
        if self.write_pending_diagnostic_charged {
            return Err(CaptureError::Busy);
        }
        if label.len() > CAPTURE_NAME_BYTES {
            return Err(CaptureError::TooLarge);
        }
        let requested_total = self
            .metadata_bytes()
            .checked_add(label.len())
            .ok_or(CaptureError::TooLarge)?;
        if requested_total > policy.metadata_limit.get() {
            return Err(CaptureError::TooLarge);
        }
        let mut diagnostic_label = String::new();
        diagnostic_label
            .try_reserve_exact(label.len())
            .map_err(|_| CaptureError::QueueFull)?;
        let actual_total = self
            .metadata_bytes()
            .checked_add(diagnostic_label.capacity())
            .ok_or(CaptureError::TooLarge)?;
        if actual_total > policy.metadata_limit.get() {
            return Err(CaptureError::TooLarge);
        }
        diagnostic_label.push_str(label);
        self.write_pending_bytes = self
            .write_pending_bytes
            .checked_add(diagnostic_label.capacity())
            .ok_or(CaptureError::TooLarge)?;
        self.write_pending_diagnostic_charged = true;
        Ok(diagnostic_label)
    }

    fn record_capture_failure_owned(
        &mut self,
        label: &mut String,
        error: CaptureError,
    ) -> Result<(), CaptureError> {
        self.admit_terminal_label(label, label.capacity())?;
        self.terminal.push(TerminalStatus {
            label: mem::take(label),
            outcome: TerminalOutcome::CaptureFailed(error),
        });
        Ok(())
    }

    fn screenshot_done_depth(&self) -> usize {
        self.done
            .len()
            .saturating_add(usize::from(self.write_pending_label.is_some()))
    }

    pub(crate) fn take_done_for_write(&mut self) -> Option<ShotCapture> {
        if self.write_pending_label.is_some() || self.done.is_empty() {
            return None;
        }
        let capture = self.done.remove(0);
        let mut pending_label = String::new();
        if pending_label
            .try_reserve_exact(capture.label.len())
            .is_err()
        {
            let mut label = discard_capture_for_failure(capture);
            let _ = self.record_capture_failure_owned(&mut label, CaptureError::QueueFull);
            return None;
        }
        pending_label.push_str(&capture.label);
        let writer_charge = capture
            .metadata_bytes()
            .saturating_add(capture.label.capacity())
            .saturating_add(pending_label.capacity());
        if self.capture_policy.is_some_and(|policy| {
            self.metadata_bytes().saturating_add(writer_charge) > policy.metadata_limit.get()
        }) {
            drop(pending_label);
            let mut label = discard_capture_for_failure(capture);
            let _ = self.record_capture_failure_owned(&mut label, CaptureError::TooLarge);
            return None;
        }
        self.write_pending_bytes = writer_charge;
        self.write_pending_label = Some(pending_label);
        #[cfg(feature = "render-diagnostics")]
        {
            self.write_pending_diagnostic_charged = false;
        }
        Some(capture)
    }

    /// Bound all three path owners before the pure builder copies any path.
    /// The directory remains charged between writes; the two output paths
    /// stay in the pending-writer charge until durable completion.
    pub(crate) fn with_pending_output_paths<T>(
        &mut self,
        label: &str,
        build: impl FnOnce(usize) -> Result<(T, usize, usize), CaptureError>,
    ) -> Result<T, CaptureError> {
        if !render_fair_capture_enabled() {
            return Err(CaptureError::Inactive);
        }
        let policy = self.capture_policy.ok_or(CaptureError::PolicyMissing)?;
        if self.write_pending_label.as_deref() != Some(label) {
            return Err(CaptureError::StaleHead);
        }
        let used_without_directory = self
            .metadata_bytes()
            .checked_sub(self.output_directory_bytes)
            .ok_or(CaptureError::TooLarge)?;
        let available = policy
            .metadata_limit
            .get()
            .checked_sub(used_without_directory)
            .ok_or(CaptureError::TooLarge)?;
        let path_limit = available / 3;
        if path_limit == 0 {
            return Err(CaptureError::TooLarge);
        }
        let (paths, directory_bytes, output_bytes) = build(path_limit)?;
        if directory_bytes > path_limit || output_bytes > path_limit * 2 {
            return Err(CaptureError::TooLarge);
        }
        self.write_pending_bytes = self
            .write_pending_bytes
            .checked_add(output_bytes)
            .ok_or(CaptureError::TooLarge)?;
        self.output_directory_bytes = directory_bytes;
        Ok(paths)
    }

    fn reserve_next_readback(
        &mut self,
        geometry: CaptureGeometry,
        now: Instant,
        paint_sequence: u64,
    ) -> Result<Option<ReadbackPermit>, CaptureError> {
        if !render_fair_capture_enabled() {
            return Err(CaptureError::Inactive);
        }
        if self.capture_policy.is_none() {
            return Err(CaptureError::PolicyMissing);
        }
        if self.active_capture.is_some() {
            return Ok(None);
        }
        let recording = self.wanted.is_empty() && self.record_armed;
        if self.wanted.is_empty() && !recording {
            return Ok(None);
        }
        let actor_capture = !recording && self.actor_capture.is_some();
        if actor_capture {
            if let Err(error) = self.validate_accepted_actor_for_staging() {
                let (mut label, _) = self.wanted.remove(0);
                self.wanted_deadlines.remove(0);
                self.actor_capture = None;
                self.reset_actor_display_confirmation();
                self.record_capture_failure_owned(&mut label, error)?;
                return Err(error);
            }
        }
        let deadline = if recording {
            self.record_deadline.ok_or(CaptureError::DeadlineMissing)?
        } else {
            self.wanted_deadlines.resize(self.wanted.len(), None);
            self.wanted_deadlines[0].ok_or(CaptureError::DeadlineMissing)?
        };
        if deadline <= now {
            if recording {
                self.record_armed = false;
                self.record_deadline = None;
                self.record_drops = self.record_drops.saturating_add(1);
            } else {
                let (mut label, _) = self.wanted.remove(0);
                self.wanted_deadlines.remove(0);
                if actor_capture {
                    self.actor_capture = None;
                    self.reset_actor_display_confirmation();
                }
                self.record_capture_failure_owned(&mut label, CaptureError::Expired)?;
            }
            return Err(CaptureError::Expired);
        }
        if recording && self.record_done.len() >= CAPTURE_QUEUE {
            self.record_armed = false;
            self.record_deadline = None;
            self.record_drops = self.record_drops.saturating_add(1);
            return Err(CaptureError::QueueFull);
        }
        if !recording && self.screenshot_done_depth() >= CAPTURE_QUEUE {
            return Ok(None);
        }
        let queue_reservation = if recording {
            self.record_done.try_reserve_exact(1)
        } else {
            self.done.try_reserve_exact(1)
        };
        if queue_reservation.is_err() {
            if recording {
                self.record_armed = false;
                self.record_deadline = None;
                self.record_drops = self.record_drops.saturating_add(1);
                return Err(CaptureError::QueueFull);
            }
            let (mut label, _) = self.wanted.remove(0);
            self.wanted_deadlines.remove(0);
            if actor_capture {
                self.actor_capture = None;
                self.reset_actor_display_confirmation();
            }
            self.record_capture_failure_owned(&mut label, CaptureError::QueueFull)?;
            return Err(CaptureError::QueueFull);
        }
        let permit = match ReadbackBudget::process().try_acquire(geometry, deadline) {
            Ok(permit) => permit,
            Err(error) => {
                let error = map_readback_error(error);
                if recording {
                    self.record_armed = false;
                    self.record_deadline = None;
                    self.record_drops = self.record_drops.saturating_add(1);
                    #[cfg(feature = "render-test-hooks")]
                    if error == CaptureError::Busy {
                        test_hooks::note_recording_busy();
                    }
                    return Err(error);
                }
                if error == CaptureError::Busy {
                    return Ok(None);
                }
                let (mut label, _) = self.wanted.remove(0);
                self.wanted_deadlines.remove(0);
                if actor_capture {
                    self.actor_capture = None;
                    self.reset_actor_display_confirmation();
                }
                self.record_capture_failure_owned(&mut label, error)?;
                return Err(error);
            }
        };
        let (label, snapshot_json) = if recording {
            self.record_armed = false;
            self.record_deadline = None;
            (Some(RECORD_LABEL.to_string()), String::new())
        } else {
            let (label, snapshot) = self.wanted.remove(0);
            self.wanted_deadlines.remove(0);
            (Some(label), snapshot)
        };
        self.active_capture = Some(ActiveCapture {
            label,
            snapshot_json,
            deadline,
            paint_sequence,
            recording,
            actor_capture,
            stamp: None,
        });
        Ok(Some(permit))
    }

    fn attach_capture_stamp(&mut self, stamp: CaptureStamp) -> Result<(), CaptureError> {
        if !render_fair_capture_enabled() {
            return Err(CaptureError::Inactive);
        }
        let policy = self.capture_policy.ok_or(CaptureError::PolicyMissing)?;
        let active = self
            .active_capture
            .as_ref()
            .ok_or(CaptureError::StaleHead)?;
        if active.stamp.is_some() {
            return Err(CaptureError::Busy);
        }
        let new_total = self
            .metadata_bytes()
            .checked_add(stamp.metadata_bytes())
            .ok_or(CaptureError::TooLarge)?;
        if new_total > policy.metadata_limit.get() {
            return Err(CaptureError::TooLarge);
        }
        self.active_capture
            .as_mut()
            .ok_or(CaptureError::StaleHead)?
            .stamp = Some(stamp);
        Ok(())
    }

    fn refuse_capture_sequence(&mut self) {
        if !self.wanted.is_empty() {
            self.wanted_deadlines.resize(self.wanted.len(), None);
            let (mut label, _) = self.wanted.remove(0);
            self.wanted_deadlines.remove(0);
            if self.actor_capture.is_some() {
                self.actor_capture = None;
                self.reset_actor_display_confirmation();
            }
            let _ = self.record_capture_failure_owned(&mut label, CaptureError::SequenceExhausted);
        } else if self.record_armed {
            self.record_armed = false;
            self.record_deadline = None;
            self.record_drops = self.record_drops.saturating_add(1);
        }
    }

    fn complete_active(
        &mut self,
        width: u32,
        height: u32,
        rgba: Vec<u8>,
        permit: ReadbackPermit,
    ) -> Result<(), CaptureError> {
        let Some(active) = self.active_capture.as_ref() else {
            return Err(CaptureError::StaleHead);
        };
        let Some(stamp) = active.stamp.as_ref() else {
            return Err(CaptureError::StaleHead);
        };
        let validation = if active.paint_sequence != stamp.paint_sequence {
            Err(CaptureError::StaleHead)
        } else if active.deadline <= Instant::now() {
            Err(CaptureError::Expired)
        } else if active.actor_capture {
            self.actor_capture
                .as_ref()
                .ok_or(CaptureError::StaleHead)
                .and_then(|accepted| accepted.fence.validate_after_staging())
        } else {
            Ok(())
        };
        if let Err(error) = validation {
            self.fail_active(error);
            return Err(error);
        }
        if active.recording && self.record_done.len() >= CAPTURE_QUEUE {
            self.fail_active(CaptureError::QueueFull);
            return Err(CaptureError::QueueFull);
        }
        if !active.recording && self.screenshot_done_depth() >= CAPTURE_QUEUE {
            self.fail_active(CaptureError::QueueFull);
            return Err(CaptureError::QueueFull);
        }
        let capture_stamp = match self
            .active_capture
            .as_mut()
            .and_then(|active| active.stamp.take())
        {
            Some(stamp) => stamp,
            None => {
                self.fail_active(CaptureError::StaleHead);
                return Err(CaptureError::StaleHead);
            }
        };
        let active = self.active_capture.take().ok_or(CaptureError::StaleHead)?;
        let actor_stamp = if active.actor_capture {
            self.actor_capture
                .take()
                .map(|accepted| accepted.fence.stamp)
        } else {
            None
        };
        if active.actor_capture {
            self.reset_actor_display_confirmation();
        }
        let capture = ShotCapture {
            label: active.label.unwrap_or_default(),
            snapshot_json: active.snapshot_json,
            width,
            height,
            rgba,
            paint_sequence: capture_stamp.paint_sequence,
            actor_stamp,
            #[cfg(feature = "render-diagnostics")]
            pixel_roi: capture_stamp.pixel_roi,
            readback_permit: Some(permit),
        };
        if active.recording {
            self.record_done.push(capture);
        } else {
            self.done.push(capture);
        }
        Ok(())
    }

    fn fail_active(&mut self, error: CaptureError) {
        let Some(active) = self.active_capture.take() else {
            return;
        };
        if active.actor_capture {
            self.actor_capture = None;
            self.reset_actor_display_confirmation();
        }
        if active.recording {
            self.record_drops = self.record_drops.saturating_add(1);
        } else if let Some(mut label) = active.label {
            if render_fair_capture_enabled()
                && self.record_capture_failure_owned(&mut label, error).is_ok()
            {
                return;
            }
            self.mark_failed(&label, capture_error_message(error));
        }
    }

    fn metadata_bytes(&self) -> usize {
        let mut total = 0_usize;
        for request in &self.requests {
            total = total.saturating_add(request.metadata_bytes());
        }
        for (label, snapshot) in &self.wanted {
            total = total.saturating_add(label.capacity().saturating_add(snapshot.capacity()));
        }
        if let Some(accepted) = &self.actor_capture {
            total = total
                .saturating_add(accepted.actor.capacity())
                .saturating_add(accepted.fence.metadata_bytes());
        }
        for capture in &self.done {
            total = total
                .saturating_add(capture.label.capacity())
                .saturating_add(capture.metadata_bytes());
        }
        for capture in &self.record_done {
            total = total
                .saturating_add(capture.label.capacity())
                .saturating_add(capture.metadata_bytes());
        }
        total = total.saturating_add(
            self.terminal
                .capacity()
                .saturating_mul(mem::size_of::<TerminalStatus>()),
        );
        for status in &self.terminal {
            total = total
                .saturating_add(status.label.capacity())
                .saturating_add(match &status.outcome {
                    TerminalOutcome::Failed(error) => error.capacity(),
                    TerminalOutcome::Written | TerminalOutcome::CaptureFailed(_) => 0,
                });
        }
        if let Some(active) = &self.active_capture {
            total = total.saturating_add(active.metadata_bytes());
        }
        total
            .saturating_add(self.write_pending_bytes)
            .saturating_add(self.output_directory_bytes)
    }

    fn trim_terminal_metadata(&mut self) {
        let Some(policy) = self.capture_policy else {
            return;
        };
        while self.metadata_bytes() > policy.metadata_limit.get() {
            if !self.terminal.is_empty() {
                self.terminal.remove(0);
            } else {
                break;
            }
        }
    }
}

impl ShotRequest {
    fn metadata_bytes(&self) -> usize {
        self.label
            .capacity()
            .saturating_add(self.snapshot_json.capacity())
            .saturating_add(self.actor.as_ref().map_or(0, String::capacity))
    }
}

impl ActiveCapture {
    fn metadata_bytes(&self) -> usize {
        self.label
            .as_ref()
            .map_or(0, String::capacity)
            .saturating_add(self.snapshot_json.capacity())
            .saturating_add(self.stamp.as_ref().map_or(0, CaptureStamp::metadata_bytes))
    }
}

pub(crate) fn render_fair_capture_enabled() -> bool {
    #[cfg(any(feature = "render-diagnostics", feature = "render-test-hooks"))]
    {
        frontend_core::surface::render_fair_enabled()
    }
    #[cfg(not(any(feature = "render-diagnostics", feature = "render-test-hooks")))]
    {
        false
    }
}

fn map_readback_error(error: ReadbackError) -> CaptureError {
    match error {
        ReadbackError::Busy => CaptureError::Busy,
        ReadbackError::StalePresent | ReadbackError::Retired => CaptureError::StaleHead,
        ReadbackError::InvalidScene => CaptureError::InvalidScene,
        ReadbackError::Timeout => CaptureError::Expired,
        ReadbackError::Device => CaptureError::Device,
        ReadbackError::Ungated | ReadbackError::Limit | ReadbackError::IdentityExhausted => {
            CaptureError::TooLarge
        }
    }
}

fn capture_error_message(error: CaptureError) -> &'static str {
    match error {
        CaptureError::QueueFull => "capture queue is full",
        CaptureError::TooLarge => "capture exceeds the configured byte envelope",
        CaptureError::Busy => "process readback slot is busy",
        CaptureError::Expired => "capture deadline expired",
        CaptureError::DeadlineOverflow => "capture deadline overflows Instant",
        CaptureError::InvalidPolicy => "capture policy contains an invalid zero duration",
        CaptureError::MissingActorRequest => "capture request is absent",
        CaptureError::StaleHead => "capture belongs to a stale head",
        CaptureError::InvalidScene => "capture requires a valid scene",
        CaptureError::Readback(_) => "retained consumer refused capture",
        CaptureError::Io => "capture output failed",
        CaptureError::Device => "capture device operation failed",
        CaptureError::Inactive => "fair rendering is not active",
        CaptureError::PolicyMissing => "capture policy is not configured",
        CaptureError::SequenceExhausted => "window paint sequence exhausted",
        CaptureError::DeadlineMissing => "capture request has no explicit deadline",
    }
}

fn truncate_utf8(text: &str, max_bytes: usize) -> &str {
    let mut end = text.len().min(max_bytes);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}
fn copy_text_with_capacity(text: &str, max_capacity: usize) -> Result<String, CaptureError> {
    if text.len() > max_capacity {
        return Err(CaptureError::TooLarge);
    }
    let mut owned = String::new();
    owned
        .try_reserve_exact(text.len())
        .map_err(|_| CaptureError::QueueFull)?;
    if owned.capacity() > max_capacity {
        return Err(CaptureError::TooLarge);
    }
    owned.push_str(text);
    Ok(owned)
}

const CAPTURE_QUEUE: usize = 8;
pub(crate) const CAPTURE_NAME_BYTES: usize = 256;

#[cfg(feature = "render-test-hooks")]
pub mod test_hooks {
    use std::sync::atomic::{AtomicU64, Ordering};

    static SUBMITTED_PAINTS: AtomicU64 = AtomicU64::new(0);
    static RECORDING_BUSY_DROPS: AtomicU64 = AtomicU64::new(0);

    /// Actual submitted window paints, not redraw requests or hook callbacks.
    pub fn submitted_paints() -> u64 {
        SUBMITTED_PAINTS.load(Ordering::Acquire)
    }

    /// Actual recording opportunities refused by the process readback budget.
    pub fn recording_busy_drops() -> u64 {
        RECORDING_BUSY_DROPS.load(Ordering::Acquire)
    }

    pub(super) fn note_recording_busy() {
        let _ = RECORDING_BUSY_DROPS.fetch_update(Ordering::AcqRel, Ordering::Acquire, |count| {
            count.checked_add(1)
        });
    }

    pub(super) fn note_submitted_paint() {
        let _ = SUBMITTED_PAINTS.fetch_update(Ordering::AcqRel, Ordering::Acquire, |count| {
            count.checked_add(1)
        });
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
    #[cfg(feature = "render-diagnostics")]
    window_paints: UiPaintObservation,
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
            #[cfg(feature = "render-diagnostics")]
            window_paints: UiPaintObservation::default(),
        }
    }

    #[cfg(feature = "render-diagnostics")]
    pub(crate) fn note_main_view_build(&mut self, built: bool) {
        self.window_paints.note_main_view_build(built);
    }

    #[cfg(feature = "render-diagnostics")]
    pub(crate) fn note_rail_presence(&mut self, present: bool) {
        self.window_paints.note_rail_presence(present);
    }

    #[cfg(feature = "render-diagnostics")]
    pub(crate) fn note_rail_build(&mut self, built: bool) {
        self.window_paints.note_rail_build(built);
    }

    #[cfg(feature = "render-diagnostics")]
    fn window_paint_observation(&self) -> UiPaintObservation {
        self.window_paints
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
        let tex_id = self.renderer.register_external_texture(texture, view);
        #[cfg(test)]
        TEXTURE_LEDGER.with_borrow_mut(|ledger| ledger.registered.push(tex_id.id()));
        tex_id
    }

    /// Update the view for an existing registered texture
    pub fn update_texture_view(&mut self, tex_id: TextureId, view: &wgpu::TextureView) -> bool {
        self.renderer.update_external_texture_view(tex_id, view)
    }

    /// Unregister a previously registered texture
    pub fn unregister_texture(&mut self, tex_id: TextureId) {
        #[cfg(test)]
        TEXTURE_LEDGER.with_borrow_mut(|ledger| ledger.unregistered.push(tex_id.id()));
        self.renderer.unregister_texture(tex_id)
    }
}

/// Texture ids that [`Gpu`] registered and unregistered on this thread.
#[cfg(test)]
#[derive(Debug, Clone, Default)]
pub(crate) struct TextureLedger {
    pub(crate) registered: Vec<u64>,
    pub(crate) unregistered: Vec<u64>,
}

#[cfg(test)]
thread_local! {
    static TEXTURE_LEDGER: std::cell::RefCell<TextureLedger> =
        std::cell::RefCell::new(TextureLedger::default());
}

/// A copy of this thread's [`TextureLedger`].
#[cfg(test)]
pub(crate) fn texture_ledger() -> TextureLedger {
    TEXTURE_LEDGER.with_borrow(TextureLedger::clone)
}

struct ImguiState {
    context: imgui::Context,
    platform: imgui_winit::WinitPlatform,
    renderer: imgui_wgpu::WgpuRenderer,
    base_style: imgui::Style,
}

struct AppWindow {
    // Kept alive to ensure the surface outlives its instance on all backends.
    #[expect(
        dead_code,
        reason = "kept alive so the surface outlives its instance on all backends"
    )]
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
    /// One nonblocking process-budgeted readback owned through mapping.
    pending_capture: Option<PendingCapture>,
    /// Monotonic identity of composed window paints; never sampled at
    /// completion time.
    paint_sequence: u64,
}

impl Drop for AppWindow {
    fn drop(&mut self) {
        // Fields drop in declaration order (Window before ImguiState).
        // The callback holds only a Weak and upgrades it per submission, so
        // expiry cannot dangle. Clear first even if other Window Arcs survive.
        ime::clear_window(&self.window);
    }
}

/// How often the loop checks a pending GPU bring-up. Only runs until the
/// device exists.
const GPU_INIT_POLL: Duration = Duration::from_millis(50);

/// GPU bring-up in flight: the OS window exists and the event loop keeps
/// pumping while a worker requests the adapter and device. A cold NVIDIA
/// D3D12 device on a hybrid laptop has taken minutes here; blocking the UI
/// thread on it made Windows mark the window Not Responding after ~5 s.
struct PendingGpu {
    window: Arc<Window>,
    ready: mpsc::Receiver<Result<GpuParts, PanelError>>,
}

/// What the worker hands back to the UI thread.
struct GpuParts {
    instance: wgpu::Instance,
    surface: wgpu::Surface<'static>,
    adapter: wgpu::Adapter,
    device: wgpu::Device,
    queue: wgpu::Queue,
}

/// Backends to try, in order. `WGPU_BACKEND` pins one set. On Windows,
/// Vulkan first: enumerating D3D12 adapters loads every GPU's D3D12 driver,
/// and on a hybrid laptop a cold NVIDIA D3D12 driver held that for ~140 s
/// even when the Intel GPU was then chosen, while Vulkan answered in 0.3 s.
/// D3D12 stays the fallback for GPUs without a Vulkan driver.
pub(crate) fn backend_attempts() -> Vec<wgpu::Backends> {
    if let Some(backends) = wgpu::Backends::from_env() {
        return vec![backends];
    }
    if cfg!(windows) {
        vec![wgpu::Backends::VULKAN, wgpu::Backends::DX12]
    } else {
        vec![wgpu::Backends::PRIMARY]
    }
}

/// One instance per attempt; the wgpu debug flags follow the environment.
pub(crate) fn instance_for(backends: wgpu::Backends) -> wgpu::Instance {
    wgpu::Instance::new(
        wgpu::InstanceDescriptor {
            backends,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        }
        .with_env(),
    )
}

/// The first attempt that can create a surface for `window`, plus the
/// attempts after it. Runs on the UI thread (macOS requires it there).
fn first_surface(
    window: &Arc<Window>,
) -> Result<(wgpu::Instance, wgpu::Surface<'static>, Vec<wgpu::Backends>), PanelError> {
    let mut attempts = backend_attempts();
    loop {
        let instance = instance_for(attempts.remove(0));
        match instance.create_surface(window.clone()) {
            Ok(surface) => return Ok((instance, surface, attempts)),
            Err(error) if attempts.is_empty() => return Err(PanelError::SurfaceCreation(error)),
            Err(_) => {}
        }
    }
}

/// Adapter and device for `surface` on one backend.
fn adapter_and_device(
    instance: &wgpu::Instance,
    surface: &wgpu::Surface<'static>,
    override_preference: Option<wgpu::PowerPreference>,
) -> Result<(wgpu::Adapter, wgpu::Device, wgpu::Queue), PanelError> {
    let request_options = panel_adapter_request_options(Some(surface), override_preference);
    let adapter = block_on(instance.request_adapter(&request_options))
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
    Ok((adapter, device, queue))
}

fn panel_power_preference(
    override_preference: Option<wgpu::PowerPreference>,
) -> wgpu::PowerPreference {
    override_preference.unwrap_or(if cfg!(target_os = "linux") {
        // Keep the display system's adapter order. A low-power sort can
        // put an integrated GPU ahead of the compositor's discrete GPU,
        // even when Vulkan advertises surface support for both.
        wgpu::PowerPreference::None
    } else {
        wgpu::PowerPreference::LowPower
    })
}

fn panel_adapter_request_options<'a>(
    compatible_surface: Option<&'a wgpu::Surface<'a>>,
    override_preference: Option<wgpu::PowerPreference>,
) -> wgpu::RequestAdapterOptions<'a, 'a> {
    wgpu::RequestAdapterOptions {
        power_preference: panel_power_preference(override_preference),
        compatible_surface,
        force_fallback_adapter: false,
    }
}

/// Adapter and device for `surface`, off the UI thread. When the current
/// backend yields no usable adapter or device, the next attempt gets a
/// fresh instance and surface here (only Windows has a next attempt, and a
/// Win32 window accepts a surface from any thread). Linux keeps the system's
/// adapter order. Other platforms prefer low power to avoid the discrete
/// GPU's cold start on hybrid laptops. `WGPU_POWER_PREF` overrides both.
fn request_gpu(
    instance: wgpu::Instance,
    surface: wgpu::Surface<'static>,
    window: Arc<Window>,
    fallbacks: Vec<wgpu::Backends>,
) -> Result<GpuParts, PanelError> {
    let override_preference = wgpu::PowerPreference::from_env();
    let mut fallbacks = fallbacks.into_iter();
    let mut attempt = (instance, surface);
    loop {
        let error = match adapter_and_device(&attempt.0, &attempt.1, override_preference) {
            Ok((adapter, device, queue)) => {
                let (instance, surface) = attempt;
                return Ok(GpuParts {
                    instance,
                    surface,
                    adapter,
                    device,
                    queue,
                });
            }
            Err(error) => error,
        };
        let next = fallbacks.by_ref().find_map(|backends| {
            let candidate = instance_for(backends);
            let candidate_surface = candidate.create_surface(window.clone()).ok()?;
            Some((candidate, candidate_surface))
        });
        match next {
            Some(next) => attempt = next,
            None => return Err(error),
        }
    }
}

/// Clamp a requested size into a work area, in matching units. A request
/// larger than the area comes back as the area; a fitting size is
/// untouched. A degenerate area (non-positive) keeps the request floored
/// at 1 px, so a bogus monitor report never collapses the window to zero.
pub fn clamp_inner_size(requested: (f64, f64), work_area: (f64, f64)) -> (f64, f64) {
    (
        clamp_dim(requested.0, work_area.0),
        clamp_dim(requested.1, work_area.1),
    )
}

fn clamp_dim(requested: f64, max: f64) -> f64 {
    if max > 0.0 {
        // `min`/`max` (not `f64::clamp`) so NaN degrades to a finite bound
        // instead of panicking.
        requested.min(max).max(1.0)
    } else {
        requested.max(1.0)
    }
}

/// Clamp a window outer position so the frame stays inside the work area,
/// in matching units. A frame larger than the area pins to the area
/// origin, so its top-left — title bar and close box — stays reachable.
/// A secondary monitor's offset origin is honored through `area_origin`.
pub fn clamp_outer_position(
    pos: (f64, f64),
    size: (f64, f64),
    area_origin: (f64, f64),
    area_size: (f64, f64),
) -> (f64, f64) {
    (
        clamp_pos(pos.0, size.0, area_origin.0, area_size.0),
        clamp_pos(pos.1, size.1, area_origin.1, area_size.1),
    )
}

fn clamp_pos(pos: f64, size: f64, origin: f64, area: f64) -> f64 {
    if size >= area {
        return origin;
    }
    let lo = origin;
    let hi = (origin + area - size).max(lo);
    // `min`/`max` (not `f64::clamp`) so a NaN position settles on a bound
    // instead of panicking.
    pos.min(hi).max(lo)
}

/// Fit a freshly created panel window into the monitor work area: shrink
/// an oversized launch size and pull an off-screen origin back so the
/// title bar stays reachable. Same provider and glue as rail grow. The
/// layout's own minimum sizes are untouched — only the OS frame is
/// constrained. Best-effort: an unknown work area leaves the window as
/// created.
fn fit_new_window(window: &Window, event_loop: &ActiveEventLoop) {
    let scale = window.scale_factor();
    let logical: LogicalSize<f64> = window.inner_size().to_logical(scale);
    work_area::fit_window(window, (logical.width, logical.height), Some(event_loop));
}

impl AppWindow {
    fn apply_dpi_scale(&mut self, dpi: f32) {
        let base_style = self.imgui.base_style.clone();
        *self.imgui.context.style_mut() = base_style;
        crate::app::apply_ui_scale(self.imgui.context.style_mut(), dpi);
        // RENDERER_HAS_TEXTURES makes ImGui lazily bake the new font size on
        // the next frame; build() would not eagerly rebuild this atlas.
    }

    /// Create the OS window and surface, then hand adapter/device creation
    /// to a worker. [`AppWindow::finish`] completes the stack on the UI
    /// thread once the worker answers.
    fn start(
        event_loop: &ActiveEventLoop,
        cfg: &PanelConfig,
        #[cfg(feature = "render-diagnostics")] diagnostics: &mut WindowDiagnostics,
    ) -> Result<PendingGpu, PanelError> {
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
        #[cfg(feature = "render-diagnostics")]
        diagnostics.window_created(window.id(), cfg.redraw);
        // Launch fit (M-003): shrink a launch size that exceeds this
        // work area, and pull a centered or off-screen origin back inside
        // it, before GPU bring-up shows the frame.
        fit_new_window(&window, event_loop);
        #[cfg(feature = "memory-profile")]
        frontend_core::bench::mark_startup(frontend_core::bench::StartupMark::WindowCreated);

        let (instance, surface, fallbacks) = first_surface(&window)?;

        let (tx, ready) = mpsc::sync_channel(1);
        let worker_window = window.clone();
        std::thread::Builder::new()
            .name("panel-gpu-init".into())
            .spawn(move || {
                // The receiver is gone only when the loop already exited.
                let _ = tx.send(request_gpu(instance, surface, worker_window, fallbacks));
            })
            .map_err(PanelError::GpuThread)?;
        Ok(PendingGpu { window, ready })
    }

    fn finish(
        window: Arc<Window>,
        parts: GpuParts,
        cfg: &PanelConfig,
        lifecycle: &mut Lifecycle,
    ) -> Result<Self, PanelError> {
        let GpuParts {
            instance,
            surface,
            adapter,
            device,
            queue,
        } = parts;

        // One-time structured identity for the memory-profile run record. It
        // writes no log line; the debug print below stays under BOT_DEBUG.
        #[cfg(feature = "memory-profile")]
        {
            let info = adapter.get_info();
            frontend_core::bench::record_adapter(
                frontend_core::bench::AdapterIdentity::new(
                    info.name,
                    format!("{:?}", info.backend),
                    format!("{:?}", info.device_type),
                )
                .with_driver(info.driver, info.driver_info)
                .with_pci(info.vendor, info.device, info.device_pci_bus_id),
            );
        }

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
        #[expect(
            clippy::disallowed_methods,
            reason = "the product sets its own ini policy just below"
        )]
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
        let base_style = context.style().clone();
        crate::app::apply_ui_scale(context.style_mut(), window.scale_factor() as f32);

        // Keep one bundled base font at its logical 14 px size. FontScaleDpi
        // makes Dear ImGui rasterize it at the monitor's physical scale.
        // Dear ImGui 1.92 builds its atlas as a renderer-managed texture.
        // Advertise that capability before the initial build; new monitor
        // scales are baked lazily by ImGui without entering the legacy path.
        {
            let io = context.io_mut();
            let mut flags = io.backend_flags();
            flags.insert(BackendFlags::RENDERER_HAS_TEXTURES);
            io.set_backend_flags(flags);
        }
        let glyphs_present = add_panel_font(&mut context);
        assert!(
            glyphs_present,
            "embedded 3270 font must cover the panel's drawn glyphs"
        );
        let _ = context.font_atlas_mut().build();

        let platform = attach_platform(&mut context, &window);

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
            io.set_config_dpi_scale_fonts(false);

            io.set_config_docking_always_tab_bar(false);
        }

        let imgui = ImguiState {
            context,
            platform,
            renderer,
            base_style,
        };

        #[cfg(feature = "memory-profile")]
        frontend_core::bench::mark_startup(frontend_core::bench::StartupMark::GpuReady);

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
            pending_capture: None,
            paint_sequence: 0,
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

    fn poll_pending_capture(&mut self, shots: &Mutex<ShotState>) -> bool {
        let result = match self.pending_capture.as_mut() {
            Some(capture) => capture.pending.poll(Instant::now()),
            None => return false,
        };
        match result {
            Ok(None) => false,
            Ok(Some(completed)) => {
                let Some(_capture) = self.pending_capture.take() else {
                    return false;
                };
                let (rgba, geometry, permit) = completed.into_parts();
                let mut shots = shots.lock().unwrap();
                let completion =
                    shots.complete_active(geometry.width(), geometry.height(), rgba, permit);
                if let Err(error) = completion {
                    shots.fail_active(error);
                }
                true
            }
            Err(error) => {
                self.pending_capture.take();
                shots.lock().unwrap().fail_active(map_readback_error(error));
                true
            }
        }
    }

    fn render<F>(
        &mut self,
        gui: &mut F,
        docking: &DockingConfig,
        shots: &Mutex<ShotState>,
        #[cfg(feature = "render-diagnostics")] diagnostics: &mut WindowDiagnostics,
    ) -> Result<bool, PanelError>
    where
        F: FnMut(&imgui::Ui, &mut Gpu) -> bool,
    {
        let _profile_frame = client::profiling::UI_FRAME.start();
        #[cfg(feature = "memory-profile")]
        let memory_profile_frame = frontend_core::bench::PanelFrameTimer::start();
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

        if render_fair_capture_enabled() {
            shots.lock().unwrap().reset_actor_display_confirmation();
        }
        let exit_requested = gui(ui, &mut gpu);
        #[cfg(feature = "render-diagnostics")]
        diagnostics.record_ui_frame(gpu.window_paint_observation(), Instant::now());
        // Keep OS cursor/IME state in sync with Dear ImGui's per-frame intent.
        self.imgui.platform.prepare_render_with_ui(ui, &self.window);
        let draw_data = self.imgui.context.render();
        if exit_requested {
            return Ok(true);
        }

        let next_paint_sequence = if render_fair_capture_enabled() {
            self.paint_sequence.checked_add(1)
        } else {
            None
        };

        // Acquire the swapchain image as late as possible to reduce time holding it.
        let (frame, reconfigure_after_present) = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame) => (frame, false),
            wgpu::CurrentSurfaceTexture::Suboptimal(frame) => (frame, true),
            wgpu::CurrentSurfaceTexture::Lost => {
                report_deferred_readback(shots, "lost");
                self.surface.configure(&self.device, &self.surface_desc);
                return Ok(exit_requested);
            }
            wgpu::CurrentSurfaceTexture::Outdated => {
                report_deferred_readback(shots, "outdated");
                self.surface.configure(&self.device, &self.surface_desc);
                return Ok(exit_requested);
            }
            wgpu::CurrentSurfaceTexture::Timeout => {
                report_deferred_readback(shots, "timeout");
                return Ok(exit_requested);
            }
            wgpu::CurrentSurfaceTexture::Occluded => {
                // An occluded frame renders only for a promoted capture.
                let submission = submit_acquired_frame_current(
                    &self.device,
                    &self.queue,
                    &mut self.imgui.renderer,
                    draw_data,
                    self.clear_color,
                    &self.surface_desc,
                    self.offscreen.as_ref(),
                    AcquiredFrame::Occluded,
                    shots,
                    &mut self.pending_capture,
                    next_paint_sequence,
                )?;
                let submitted = !matches!(&submission, &FrameSubmission::Skipped);
                match submission {
                    FrameSubmission::Submitted(readbacks) => {
                        if !readbacks.is_empty() && std::env::var("BOT_DEBUG").as_deref() == Ok("1")
                        {
                            eprintln!(
                                "[panel] capture offscreen readback submitted on occluded surface: {}",
                                readbacks.len()
                            );
                        }
                        complete_readbacks(
                            &self.device,
                            self.surface_desc.format,
                            readbacks,
                            shots,
                        );
                    }
                    FrameSubmission::BoundedSubmitted | FrameSubmission::Skipped => {}
                }
                if submitted {
                    if let Some(sequence) = next_paint_sequence {
                        self.paint_sequence = sequence;
                        #[cfg(feature = "render-test-hooks")]
                        test_hooks::note_submitted_paint();
                    }
                }
                return Ok(exit_requested);
            }
            wgpu::CurrentSurfaceTexture::Validation => {
                return Err(PanelError::SurfaceValidation);
            }
        };

        let submission = submit_acquired_frame_current(
            &self.device,
            &self.queue,
            &mut self.imgui.renderer,
            draw_data,
            self.clear_color,
            &self.surface_desc,
            self.offscreen.as_ref(),
            AcquiredFrame::Presentable(&frame.texture),
            shots,
            &mut self.pending_capture,
            next_paint_sequence,
        )?;
        let submitted = !matches!(&submission, &FrameSubmission::Skipped);
        // Submit → present → map, in that order: the legacy map bookkeeping
        // stays after present so it never holds the drawable.
        frame.present();
        #[cfg(feature = "memory-profile")]
        memory_profile_frame.presented();
        if let FrameSubmission::Submitted(readbacks) = submission {
            complete_readbacks(&self.device, self.surface_desc.format, readbacks, shots);
        }
        if submitted {
            if let Some(sequence) = next_paint_sequence {
                self.paint_sequence = sequence;
                #[cfg(feature = "render-test-hooks")]
                test_hooks::note_submitted_paint();
            }
            #[cfg(feature = "render-diagnostics")]
            diagnostics.submitted_native_paint(next_paint_sequence, Instant::now());
        }
        if reconfigure_after_present {
            self.surface.configure(&self.device, &self.surface_desc);
        }
        Ok(exit_requested)
    }
}

/// One pending staging readback; the copy is recorded in the encoder and
/// the bytes land after submit + poll.
struct PendingCapture {
    pending: PendingWindowReadback,
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
        if label == RECORD_LABEL {
            // Recording frames have no PNG ledger entry: a lost frame is
            // simply absent from the video, and the pacer stages the next.
            continue;
        }
        if !captures.iter().any(|capture| &capture.label == label) {
            shots.mark_failed(label, "readback did not complete");
        }
    }
    let (record, shots_done): (Vec<ShotCapture>, Vec<ShotCapture>) = captures
        .into_iter()
        .partition(|capture| capture.label == RECORD_LABEL);
    shots.done.extend(shots_done);
    shots.record_done.extend(record);
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
    BoundedSubmitted,
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
    // (unpromoted jobs stay queued for the next visible frame; an armed
    // recording frame likewise waits for the next visible frame).
    if present_dest.is_none() && !shots.lock().unwrap().capture_pending() {
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

#[allow(clippy::too_many_arguments)] // bounded frame submit packs render and capture owners
fn submit_acquired_frame_bounded(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    renderer: &mut imgui_wgpu::WgpuRenderer,
    draw_data: &mut imgui::DrawData,
    clear_color: wgpu::Color,
    desc: &wgpu::SurfaceConfiguration,
    offscreen: Option<&wgpu::Texture>,
    acquired: AcquiredFrame<'_>,
    shots: &Mutex<ShotState>,
    pending: &mut Option<PendingCapture>,
    paint_sequence: Option<u64>,
) -> Result<FrameSubmission, PanelError> {
    let present_dest = match acquired {
        AcquiredFrame::Presentable(image) => Some(image),
        AcquiredFrame::Occluded => None,
    };
    if present_dest.is_none() && !shots.lock().unwrap().capture_pending() {
        return Ok(FrameSubmission::Skipped);
    }
    let one_off;
    let target = match offscreen {
        Some(existing) => existing,
        None => match present_dest {
            Some(image) => image,
            None => {
                one_off = make_offscreen(device, desc);
                &one_off
            }
        },
    };
    let blit_dest = present_dest.filter(|_| offscreen.is_some());
    let mut encoder = encode_ui_frame(device, renderer, draw_data, clear_color, target)?;
    if let Some(dest) = blit_dest {
        blit_texture(&mut encoder, target, dest);
    }
    queue.submit(Some(encoder.finish()));

    if pending.is_none() {
        if let Some(paint_sequence) = paint_sequence {
            let geometry = CaptureGeometry::new(desc.width, desc.height);
            let reservation = {
                shots.lock().unwrap().reserve_next_readback(
                    geometry,
                    Instant::now(),
                    paint_sequence,
                )
            };
            if let Ok(Some(permit)) = reservation {
                let staging_check = shots.lock().unwrap().validate_active_actor_for_staging();
                if let Err(error) = staging_check {
                    drop(permit);
                    shots.lock().unwrap().fail_active(error);
                } else {
                    #[cfg(feature = "render-diagnostics")]
                    let pixel_roi = client::render::diagnostics::snapshot_shot_bind();
                    let stamp = CaptureStamp {
                        paint_sequence,
                        #[cfg(feature = "render-diagnostics")]
                        pixel_roi,
                    };
                    let attach_result = shots.lock().unwrap().attach_capture_stamp(stamp);
                    if let Err(error) = attach_result {
                        drop(permit);
                        shots.lock().unwrap().fail_active(error);
                    } else {
                        match begin_window_readback(device, queue, target, permit) {
                            Ok(readback) => {
                                let release_result =
                                    shots.lock().unwrap().release_actor_consumer_after_staging();
                                if let Err(error) = release_result {
                                    drop(readback);
                                    shots.lock().unwrap().fail_active(error);
                                } else {
                                    *pending = Some(PendingCapture { pending: readback });
                                }
                            }
                            Err((error, permit)) => {
                                drop(permit);
                                shots.lock().unwrap().fail_active(map_readback_error(error));
                            }
                        }
                    }
                }
            }
        } else {
            shots.lock().unwrap().refuse_capture_sequence();
        }
    }
    Ok(FrameSubmission::BoundedSubmitted)
}

#[allow(clippy::too_many_arguments)] // dispatches current panel draw/capture mode
fn submit_acquired_frame_current(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    renderer: &mut imgui_wgpu::WgpuRenderer,
    draw_data: &mut imgui::DrawData,
    clear_color: wgpu::Color,
    desc: &wgpu::SurfaceConfiguration,
    offscreen: Option<&wgpu::Texture>,
    acquired: AcquiredFrame<'_>,
    shots: &Mutex<ShotState>,
    pending: &mut Option<PendingCapture>,
    paint_sequence: Option<u64>,
) -> Result<FrameSubmission, PanelError> {
    if render_fair_capture_enabled() {
        submit_acquired_frame_bounded(
            device,
            queue,
            renderer,
            draw_data,
            clear_color,
            desc,
            offscreen,
            acquired,
            shots,
            pending,
            paint_sequence,
        )
    } else {
        submit_acquired_frame(
            device,
            queue,
            renderer,
            draw_data,
            clear_color,
            desc,
            offscreen,
            acquired,
            shots,
        )
    }
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
    let mut encoder = encode_ui_frame(device, renderer, draw_data, clear_color, target)?;
    let mut readbacks = Vec::new();
    {
        let mut guard = shots.lock().unwrap();
        let mut wanted = mem::take(&mut guard.wanted);
        if mem::replace(&mut guard.record_armed, false) {
            wanted.push((RECORD_LABEL.to_string(), String::new()));
        }
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

#[allow(clippy::too_many_arguments)] // renderer draw packs frame state and target
fn encode_ui_frame(
    device: &wgpu::Device,
    renderer: &mut imgui_wgpu::WgpuRenderer,
    draw_data: &mut imgui::DrawData,
    clear_color: wgpu::Color,
    target: &wgpu::Texture,
) -> Result<wgpu::CommandEncoder, PanelError> {
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
    Ok(encoder)
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
                paint_sequence: 0,
                actor_stamp: None,
                readback_permit: None,
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

/// Subset of 3270 Nerd Font Regular: panel text, rail controls and Nerd
/// Fonts' Font Awesome icons. Recreate the asset with this exact command:
/// `python3 -m fontTools.subset /path/to/nf-3270NerdFont-Regular.ttf --output-file=crates/panel/assets/3270NerdFont-Regular-subset.ttf '--unicodes=U+0020-007E,U+00A0-00FF,U+2013-2014,U+2026,U+2192,U+2194,U+2212,U+2264,U+2265,U+2582-2585,U+2605,U+2715,U+2717,U+F005,U+F006,U+F015,U+F019,U+F054,U+F07B,U+F108,U+F15B,U+F15C' --no-ignore-missing-unicodes`
const PANEL_FONT_BYTES: &[u8] = include_bytes!("../assets/3270NerdFont-Regular-subset.ttf");

/// Non-ASCII codepoints the panel renders as text (status is drawn geometry).
const PANEL_REQUIRED_GLYPHS: &[char] = &[
    '\u{2013}', '\u{2014}', '\u{2026}', '\u{2192}', '\u{2194}', '\u{2212}', '\u{2264}', '\u{2265}',
    '\u{2582}', '\u{2583}', '\u{2584}', '\u{2585}', '\u{2605}', '\u{2715}', '\u{2717}', '\u{f005}',
    '\u{f006}', '\u{f015}', '\u{f019}', '\u{f054}', '\u{f07b}', '\u{f108}', '\u{f15b}', '\u{f15c}',
];

/// Add the 3270 subset as the atlas's only base font at
/// [`crate::theme::PANEL_FONT_SIZE`]. Dear ImGui 1.92 owns density scaling;
/// this remains a 14 logical px size at every monitor scale.
pub(crate) fn add_panel_font(ctx: &mut imgui::Context) -> bool {
    let mut fonts = ctx.fonts();
    let font = fonts
        .add_font_from_memory_ttf(
            PANEL_FONT_BYTES,
            crate::theme::PANEL_FONT_SIZE,
            Some(&imgui::FontConfig::new().name("3270 Nerd Font Regular")),
            None,
        )
        .expect("embedded 3270 Nerd Font Regular subset is a valid TTF");
    PANEL_REQUIRED_GLYPHS
        .iter()
        .all(|&glyph| font.is_glyph_in_font(glyph))
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

/// Run the owner's GPU shutdown hook with a handle on `renderer`, before
/// the renderer drops, so that the owner can unregister its textures.
pub(crate) fn release_renderer(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    renderer: &mut imgui_wgpu::WgpuRenderer,
    release: &mut dyn FnMut(&mut Gpu),
) {
    let mut gpu = Gpu::new(device, queue, renderer);
    release(&mut gpu);
}

struct App<F>
where
    F: FnMut(&imgui::Ui, &mut Gpu) -> bool + 'static,
{
    cfg: PanelConfig,
    window: Option<AppWindow>,
    /// The window exists but its adapter/device are still being requested.
    pending: Option<PendingGpu>,
    lifecycle: Lifecycle,
    ui_frame: F,
    on_gpu_shutdown: Box<dyn FnMut(&mut Gpu)>,
    /// Whole-window shot coordination shared with the UI body (the
    /// scenario sink's requests) and the render readback.
    shots: Arc<Mutex<ShotState>>,
    last_wake: Instant,
    #[cfg(feature = "render-diagnostics")]
    diagnostics: WindowDiagnostics,
    /// Why the loop stopped without a window (creation or rebuild failed);
    /// [`run`] returns it, so the process exits non-zero.
    failure: Option<PanelError>,
}

impl<F> App<F>
where
    F: FnMut(&imgui::Ui, &mut Gpu) -> bool + 'static,
{
    fn new(
        cfg: PanelConfig,
        on_style: impl FnMut(&mut imgui::Context) + 'static,
        on_gpu_init: impl FnMut(&Arc<Window>, &wgpu::Device, &wgpu::Queue, &wgpu::SurfaceConfiguration)
            + 'static,
        shots: Arc<Mutex<ShotState>>,
        on_gpu_shutdown: impl FnMut(&mut Gpu) + 'static,
        ui_frame: F,
    ) -> Self {
        Self {
            cfg,
            window: None,
            pending: None,
            lifecycle: Lifecycle {
                on_style: Some(Box::new(on_style)),
                on_gpu_init: Some(Box::new(on_gpu_init)),
            },
            ui_frame,
            on_gpu_shutdown: Box::new(on_gpu_shutdown),
            shots,
            last_wake: Instant::now(),
            #[cfg(feature = "render-diagnostics")]
            diagnostics: WindowDiagnostics::new(),
            failure: None,
        }
    }

    fn drop_window(&mut self) {
        if let Some(mut window) = self.window.take() {
            release_renderer(
                &window.device,
                &window.queue,
                &mut window.imgui.renderer,
                &mut self.on_gpu_shutdown,
            );
        }
    }

    fn close_window(&mut self, event_loop: &ActiveEventLoop, _reason: &'static str) {
        self.drop_window();
        self.pending = None;
        #[cfg(feature = "render-diagnostics")]
        self.diagnostics.close(_reason);
        event_loop.exit();
    }

    /// Create the window and start its GPU bring-up; the loop keeps pumping
    /// until [`App::poll_gpu`] completes it.
    fn start_gpu(&mut self, event_loop: &ActiveEventLoop) {
        #[cfg(feature = "render-diagnostics")]
        let start = AppWindow::start(event_loop, &self.cfg, &mut self.diagnostics);
        #[cfg(not(feature = "render-diagnostics"))]
        let start = AppWindow::start(event_loop, &self.cfg);
        match start {
            Ok(pending) => self.pending = Some(pending),
            Err(e) => {
                #[cfg(feature = "render-diagnostics")]
                {
                    self.diagnostics.error(window_diagnostics::error_kind(&e));
                    self.diagnostics.close("window_start_failed");
                }
                self.failure = Some(e);
                event_loop.exit();
            }
        }
    }

    /// Finish a pending bring-up once the worker answers. Returns whether
    /// it is still pending.
    fn poll_gpu(&mut self, event_loop: &ActiveEventLoop) -> bool {
        let Some(pending) = self.pending.take() else {
            return false;
        };
        let result = match pending.ready.try_recv() {
            Ok(Ok(parts)) => {
                AppWindow::finish(pending.window, parts, &self.cfg, &mut self.lifecycle)
            }
            Ok(Err(e)) => Err(e),
            Err(TryRecvError::Empty) => {
                self.pending = Some(pending);
                return true;
            }
            Err(TryRecvError::Disconnected) => Err(PanelError::GpuInitLost),
        };
        match result {
            Ok(window) => {
                window.window.request_redraw();
                self.window = Some(window);
            }
            Err(e) => {
                #[cfg(feature = "render-diagnostics")]
                {
                    self.diagnostics.error(window_diagnostics::error_kind(&e));
                    self.diagnostics.close("gpu_initialization_failed");
                }
                self.failure = Some(e);
                event_loop.exit();
            }
        }
        false
    }
}

fn current_os_window_id(
    window: &Option<AppWindow>,
    pending: &Option<PendingGpu>,
) -> Option<WindowId> {
    window
        .as_ref()
        .map(|w| w.window.id())
        .or_else(|| pending.as_ref().map(|p| p.window.id()))
}

impl<F> ApplicationHandler for App<F>
where
    F: FnMut(&imgui::Ui, &mut Gpu) -> bool + 'static,
{
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if event_loop.exiting() {
            return;
        }
        if self.window.is_none() && self.pending.is_none() {
            self.start_gpu(event_loop);
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
            // OS-destroy of *this* window (or a close request) drops the
            // stack before `exit`. winit still delivers `RedrawRequested`
            // later in this pump; dropping `AppWindow` makes that a no-op
            // so X11 `inner_size` cannot unwrap BadDrawable after DestroyNotify.
            // A GPU rebuild drops the old winit window, which also emits
            // `Destroyed` — ignore ids that are not the current window.
            WindowEvent::Destroyed => {
                if current_os_window_id(&self.window, &self.pending) != Some(window_id) {
                    return;
                }
                self.close_window(event_loop, "destroyed");
            }
            WindowEvent::CloseRequested => {
                self.close_window(event_loop, "close_requested");
            }
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

                    #[cfg(feature = "render-diagnostics")]
                    let render = window.render(
                        &mut self.ui_frame,
                        &self.cfg.docking,
                        self.shots.as_ref(),
                        &mut self.diagnostics,
                    );
                    #[cfg(not(feature = "render-diagnostics"))]
                    let render =
                        window.render(&mut self.ui_frame, &self.cfg.docking, self.shots.as_ref());
                    match render {
                        Ok(true) => event_loop.exit(),
                        Ok(false) if matches!(self.cfg.redraw, RedrawMode::Poll) => {
                            window.window.request_redraw();
                        }
                        Ok(false) => {}
                        Err(e) => {
                            #[cfg(feature = "render-diagnostics")]
                            self.diagnostics.error(window_diagnostics::error_kind(&e));
                            eprintln!(
                                "Render error: {e}; attempting to recover by recreating GPU state"
                            );
                            need_recreate = true;
                        }
                    }
                }

                if need_recreate && !event_loop.exiting() {
                    // Release owner textures while the old renderer is live,
                    // then rebuild the same GPU/window stack as startup.
                    self.drop_window();
                    #[cfg(feature = "render-diagnostics")]
                    self.diagnostics.close("render_error");
                    self.start_gpu(event_loop);
                }
            }
            _ => {
                let Some(window) = self.window.as_mut() else {
                    // Close/destroy already dropped the stack. Other events
                    // before GPU bring-up finishes are ignored.
                    return;
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
                    // extra-deliver. The frame owner handles releases
                    // before hover-gating new presses in `capture_keys`.
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
                        window.apply_dpi_scale(window.window.scale_factor() as f32);
                        let new_size = window.window.inner_size();
                        window.resize(new_size);
                        window.window.request_redraw();
                    }
                    _ => {}
                }
            }
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if event_loop.exiting() {
            #[cfg(feature = "render-diagnostics")]
            self.diagnostics.about_to_wait(Instant::now());
            return;
        }
        // The independent render pump runs on every loop turn, and the loop
        // wakes for its next deadline, so a render grant whose head stops
        // consuming expires on time even when no frame is drawn. It never
        // renders on this thread.
        let pump_now = Instant::now();
        #[cfg(feature = "render-diagnostics")]
        self.diagnostics.about_to_wait(pump_now);
        let render_due = frontend_core::surface::pump_render(pump_now);
        if self.poll_gpu(event_loop) {
            event_loop.set_control_flow(ControlFlow::WaitUntil(earliest(
                Instant::now() + GPU_INIT_POLL,
                render_due,
            )));
            return;
        }
        let mut readback_pending = false;
        let mut capture_queued = false;
        let mut ui_due = None;
        if render_fair_capture_enabled() {
            if let Some(window) = self.window.as_mut() {
                if window.poll_pending_capture(self.shots.as_ref()) {
                    window.window.request_redraw();
                }
                readback_pending = window.pending_capture.is_some();
            }
            let shots = self.shots.lock().unwrap();
            capture_queued = shots.capture_pending();
            ui_due = shots.ui_work_deadline;
            drop(shots);
            if ui_due.is_some_and(|due| due <= pump_now) {
                if let Some(window) = &self.window {
                    window.window.request_redraw();
                }
            }
        }
        match self.cfg.redraw {
            RedrawMode::Poll => {
                event_loop.set_control_flow(render_deadline_control_flow(
                    self.cfg.redraw,
                    pump_now,
                    render_due,
                    ui_due,
                ));
                if let Some(window) = &self.window {
                    window.window.request_redraw();
                }
            }
            RedrawMode::Wait => {
                if capture_queued && !readback_pending {
                    if let Some(window) = &self.window {
                        window.window.request_redraw();
                    }
                }
                event_loop.set_control_flow(if capture_queued || readback_pending {
                    ControlFlow::Poll
                } else {
                    render_deadline_control_flow(self.cfg.redraw, pump_now, render_due, ui_due)
                });
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
                event_loop.set_control_flow(if readback_pending {
                    ControlFlow::Poll
                } else {
                    render_deadline_control_flow(self.cfg.redraw, next_wake, render_due, ui_due)
                });
            }
        }
    }
}

/// The earlier of the window's own wake and the render pump's deadline.
fn earliest(wake: Instant, render_due: Option<Instant>) -> Instant {
    render_due.map_or(wake, |due| due.min(wake))
}

/// Poll pumps every loop turn; sleeping modes wake for whichever independent
/// deadline comes first.
fn render_deadline_control_flow(
    redraw: RedrawMode,
    panel_due: Instant,
    render_due: Option<Instant>,
    ui_due: Option<Instant>,
) -> ControlFlow {
    let next_due = match (render_due, ui_due) {
        (Some(render), Some(ui)) => Some(render.min(ui)),
        (render, ui) => render.or(ui),
    };
    match redraw {
        RedrawMode::Poll => ControlFlow::Poll,
        RedrawMode::Wait => next_due.map_or(ControlFlow::Wait, ControlFlow::WaitUntil),
        RedrawMode::WaitUntil { .. } => ControlFlow::WaitUntil(earliest(panel_due, next_due)),
    }
}

/// Run the panel window loop. `ui_frame` is called every frame with the Ui
/// and `Gpu` handle, and returns `true` to end the loop after that frame.
/// `on_gpu_init` fires once the device/queue/surface exist. `on_gpu_shutdown`
/// runs before the renderer is dropped or rebuilt, while its GPU handle is
/// still live. `shots` coordinates whole-window captures.
pub fn run<F>(
    cfg: PanelConfig,
    on_style: impl FnMut(&mut imgui::Context) + 'static,
    on_gpu_init: impl FnMut(&Arc<Window>, &wgpu::Device, &wgpu::Queue, &wgpu::SurfaceConfiguration)
        + 'static,
    on_gpu_shutdown: impl FnMut(&mut Gpu) + 'static,
    shots: Arc<Mutex<ShotState>>,
    ui_frame: F,
) -> Result<(), PanelError>
where
    F: FnMut(&imgui::Ui, &mut Gpu) -> bool + 'static,
{
    #[cfg(feature = "render-diagnostics")]
    let event_loop = match EventLoop::new() {
        Ok(event_loop) => event_loop,
        Err(error) => {
            let mut diagnostics = WindowDiagnostics::new();
            diagnostics.error("event_loop");
            return Err(error.into());
        }
    };
    #[cfg(not(feature = "render-diagnostics"))]
    let event_loop = EventLoop::new()?;
    match cfg.redraw {
        RedrawMode::Poll => event_loop.set_control_flow(ControlFlow::Poll),
        RedrawMode::Wait => event_loop.set_control_flow(ControlFlow::Wait),
        RedrawMode::WaitUntil { fps } => {
            let frame = Duration::from_secs_f32(1.0f32 / fps.max(1.0));
            event_loop.set_control_flow(ControlFlow::WaitUntil(Instant::now() + frame));
        }
    }

    let mut app = App::new(cfg, on_style, on_gpu_init, shots, on_gpu_shutdown, ui_frame);
    let run_result = event_loop.run_app(&mut app);
    app.drop_window();
    app.pending = None;
    #[cfg(feature = "render-diagnostics")]
    if run_result.is_err() {
        app.diagnostics.error("event_loop");
    }
    #[cfg(feature = "render-diagnostics")]
    app.diagnostics.close("event_loop_returned");
    run_result?;
    // A window that could not be created (or rebuilt after a GPU error)
    // ends the loop; report it so the process exits non-zero.
    match app.failure.take() {
        Some(error) => Err(error),
        None => Ok(()),
    }
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
