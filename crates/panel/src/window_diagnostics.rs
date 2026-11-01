//! Bounded observations for the one native panel window and its shared event loop.
//!
//! No event history is retained. The recorder keeps one window's exact counters,
//! latest surface states, and one sample baseline, emitting at most one sample
//! interval per second plus a final partial interval and lifecycle/error rows.

use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};
use winit::window::WindowId;

use super::{PanelError, RedrawMode};

const SCHEMA: &str = "render-s5-window-churn-v2";
const PUMP_ID: &str = "ApplicationHandler::about_to_wait";
const MAIN_VIEW_SURFACE_ID: &str = "main_view";
const RAIL_SURFACE_ID: &str = "sidecar_rail";
const SAMPLE_INTERVAL_NS: u64 = 1_000_000_000;

/// Surfaces built by the app callback for one UI frame. `None` means the
/// callback did not report that surface; `Some(false)` is an explicit absence.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct UiPaintObservation {
    pub(super) main_view_built: Option<bool>,
    pub(super) rail_present: Option<bool>,
    pub(super) rail_built: Option<bool>,
}

impl UiPaintObservation {
    pub(super) fn note_main_view_build(&mut self, built: bool) {
        self.main_view_built = Some(built);
    }

    pub(super) fn note_rail_presence(&mut self, present: bool) {
        self.rail_present = Some(present);
        if !present {
            self.rail_built = Some(false);
        }
    }

    pub(super) fn note_rail_build(&mut self, built: bool) {
        self.rail_built = Some(built);
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Counters {
    about_to_wait_count: u64,
    max_pump_interval_ns: Option<u64>,
    ui_frame_count: u64,
    main_view_paint_count: u64,
    rail_present_frame_count: u64,
    rail_paint_count: u64,
    native_paint_count: u64,
    native_paint_sequence: Option<u64>,
    submitted_main_view_paint_count: u64,
    submitted_rail_paint_count: u64,
}

#[derive(Clone, Copy, Debug)]
struct SampleBaseline {
    monotonic_ns: u64,
    counters: Counters,
    surfaces: UiPaintObservation,
}

#[derive(Debug)]
struct WindowStats {
    identity: String,
    redraw_mode: &'static str,
    clock_origin_unix_ns: Option<u64>,
    counters: Counters,
    last_pump_ns: Option<u64>,
    last_pump_interval_ns: Option<u64>,
    last_ui_frame_ns: Option<u64>,
    last_ui_paints: UiPaintObservation,
    last_native_paint_ui_frame: Option<u64>,
    baseline: SampleBaseline,
}

impl WindowStats {
    fn new(
        identity: String,
        redraw_mode: &'static str,
        clock_origin_unix_ns: Option<u64>,
        now_ns: u64,
    ) -> Self {
        let counters = Counters::default();
        let surfaces = UiPaintObservation::default();
        Self {
            identity,
            redraw_mode,
            clock_origin_unix_ns,
            counters,
            last_pump_ns: None,
            last_pump_interval_ns: None,
            last_ui_frame_ns: None,
            last_ui_paints: surfaces,
            last_native_paint_ui_frame: None,
            baseline: SampleBaseline {
                monotonic_ns: now_ns,
                counters,
                surfaces,
            },
        }
    }

    fn scope_row(&self, event_order: u64, elapsed_ns: u64) -> Value {
        json!({
            "schema": SCHEMA,
            "record": "scope",
            "event_order": event_order,
            "elapsed_ns": elapsed_ns,
            "clock_origin_unix_ns": self.clock_origin_unix_ns,
            "status": "observed",
            "window_id": self.identity,
            "pump_id": PUMP_ID,
            "surfaces": [
                { "surface_id": MAIN_VIEW_SURFACE_ID, "role": "main_view" },
                { "surface_id": RAIL_SURFACE_ID, "role": "sidecar_rail" },
            ],
        })
    }

    fn pump_sample_row(
        &self,
        event_order: u64,
        start_ns: u64,
        end_ns: u64,
    ) -> Value {
        let start = self.baseline.counters;
        let end = self.counters;
        json!({
            "schema": SCHEMA,
            "record": "sample",
            "series": "window_pump",
            "event_order": event_order,
            "window_id": self.identity,
            "pump_id": PUMP_ID,
            "start_ns": start_ns,
            "end_ns": end_ns,
            "status": "success",
            "clock_origin_unix_ns": self.clock_origin_unix_ns,
            "redraw_mode": self.redraw_mode,
            "about_to_wait_count_start": start.about_to_wait_count,
            "about_to_wait_count_end": end.about_to_wait_count,
            "max_interval_ns_start": start.max_pump_interval_ns,
            "max_interval_ns_end": end.max_pump_interval_ns,
            "last_interval_ns": self.last_pump_interval_ns,
            "last_pump_monotonic_ns": self.last_pump_ns,
            "native_paint_count_start": start.native_paint_count,
            "native_paint_count_end": end.native_paint_count,
            "native_sequence_start": start.native_paint_sequence,
            "native_sequence_end": end.native_paint_sequence,
        })
    }

    fn ui_paint_sample_row(
        &self,
        event_order: u64,
        start_ns: u64,
        end_ns: u64,
        role: &'static str,
    ) -> Value {
        let (
            surface_id,
            built_start,
            built_end,
            submitted_start,
            submitted_end,
            rail_present_start,
            rail_present_end,
            rail_present_count_start,
            rail_present_count_end,
            last_built_start,
            last_built_end,
        ) = if role == "main_view" {
            (
                MAIN_VIEW_SURFACE_ID,
                self.baseline.counters.main_view_paint_count,
                self.counters.main_view_paint_count,
                self.baseline.counters.submitted_main_view_paint_count,
                self.counters.submitted_main_view_paint_count,
                None,
                None,
                None,
                None,
                self.baseline.surfaces.main_view_built,
                self.last_ui_paints.main_view_built,
            )
        } else {
            (
                RAIL_SURFACE_ID,
                self.baseline.counters.rail_paint_count,
                self.counters.rail_paint_count,
                self.baseline.counters.submitted_rail_paint_count,
                self.counters.submitted_rail_paint_count,
                self.baseline.surfaces.rail_present,
                self.last_ui_paints.rail_present,
                Some(self.baseline.counters.rail_present_frame_count),
                Some(self.counters.rail_present_frame_count),
                self.baseline.surfaces.rail_built,
                self.last_ui_paints.rail_built,
            )
        };
        json!({
            "schema": SCHEMA,
            "record": "sample",
            "series": "ui_paint",
            "event_order": event_order,
            "window_id": self.identity,
            "surface_id": surface_id,
            "role": role,
            "start_ns": start_ns,
            "end_ns": end_ns,
            "clock_origin_unix_ns": self.clock_origin_unix_ns,
            "status": "success",
            "built_paint_count_start": built_start,
            "built_paint_count_end": built_end,
            "submitted_paint_count_start": submitted_start,
            "submitted_paint_count_end": submitted_end,
            "rail_present_start": rail_present_start,
            "rail_present_end": rail_present_end,
            "rail_present_frame_count_start": rail_present_count_start,
            "rail_present_frame_count_end": rail_present_count_end,
            "last_built_start": last_built_start,
            "last_built_end": last_built_end,
            "frame_count_start": self.baseline.counters.ui_frame_count,
            "frame_count_end": self.counters.ui_frame_count,
            "native_paint_count_start": self.baseline.counters.native_paint_count,
            "native_paint_count_end": self.counters.native_paint_count,
            "native_sequence_start": self.baseline.counters.native_paint_sequence,
            "native_sequence_end": self.counters.native_paint_sequence,
            "last_ui_frame_monotonic_ns": self.last_ui_frame_ns,
            "last_submitted_ui_frame_count": self.last_native_paint_ui_frame,
        })
    }
}

/// One event-loop instance. It observes only the current native window; when
/// that window closes its bounded summary is emitted before the next window
/// generation can replace it.
pub(super) struct WindowDiagnostics {
    origin: Instant,
    clock_origin_unix_ns: Option<u64>,
    event_order: u64,
    active: Option<WindowStats>,
}

impl WindowDiagnostics {
    pub(super) fn new() -> Self {
        let before = Instant::now();
        let wall_origin = SystemTime::now();
        let after = Instant::now();
        let origin = before + after.duration_since(before) / 2;
        let clock_origin_unix_ns = wall_origin
            .duration_since(UNIX_EPOCH)
            .ok()
            .map(duration_ns);
        Self {
            origin,
            clock_origin_unix_ns,
            event_order: 0,
            active: None,
        }
    }

    fn elapsed_ns_at(&self, now: Instant) -> u64 {
        duration_ns(now.saturating_duration_since(self.origin))
    }

    fn next_event_order(&mut self) -> u64 {
        self.event_order = self.event_order.saturating_add(1);
        self.event_order
    }

    pub(super) fn window_created(&mut self, window_id: WindowId, mode: RedrawMode) {
        let identity = format!("{window_id:?}");
        let now_ns = self.elapsed_ns_at(Instant::now());
        let rows = self.record_window_created_at(identity, redraw_mode_name(mode), now_ns);
        for row in rows {
            self.emit(row);
        }
    }

    pub(super) fn record_window_created_at(
        &mut self,
        identity: String,
        redraw_mode: &'static str,
        now_ns: u64,
    ) -> [Value; 2] {
        self.active = Some(WindowStats::new(
            identity,
            redraw_mode,
            self.clock_origin_unix_ns,
            now_ns,
        ));
        let birth_order = self.next_event_order();
        let scope_order = self.next_event_order();
        let stats = self.active.as_ref().expect("window just recorded");
        [
            json!({
                "schema": SCHEMA,
                "record": "window_lifecycle",
                "kind": "birth",
                "event_order": birth_order,
                "elapsed_ns": now_ns,
                "clock_origin_unix_ns": stats.clock_origin_unix_ns,
                "window_id": stats.identity,
                "status": "success",
                "redraw_mode": stats.redraw_mode,
            }),
            stats.scope_row(scope_order, now_ns),
        ]
    }

    pub(super) fn about_to_wait(&mut self, now: Instant) {
        let now_ns = self.elapsed_ns_at(now);
        if let Some(rows) = self.record_pump_at(now_ns) {
            self.emit_samples(rows);
        }
    }

    pub(super) fn record_pump_at(&mut self, now_ns: u64) -> Option<[Option<Value>; 3]> {
        let due = {
            let stats = self.active.as_mut()?;
            stats.counters.about_to_wait_count =
                stats.counters.about_to_wait_count.saturating_add(1);
            let interval = stats
                .last_pump_ns
                .map(|last| now_ns.saturating_sub(last));
            stats.last_pump_interval_ns = interval;
            if let Some(interval_ns) = interval {
                stats.counters.max_pump_interval_ns = Some(
                    stats
                        .counters
                        .max_pump_interval_ns
                        .map_or(interval_ns, |max| max.max(interval_ns)),
                );
            }
            stats.last_pump_ns = Some(now_ns);
            now_ns.saturating_sub(stats.baseline.monotonic_ns) >= SAMPLE_INTERVAL_NS
        };
        due.then(|| self.sample_rows_at(now_ns))
    }

    pub(super) fn record_ui_frame(&mut self, paints: UiPaintObservation, now: Instant) {
        let now_ns = self.elapsed_ns_at(now);
        self.record_ui_frame_at(paints, now_ns);
    }

    pub(super) fn record_ui_frame_at(&mut self, paints: UiPaintObservation, now_ns: u64) {
        let Some(stats) = self.active.as_mut() else {
            return;
        };
        stats.counters.ui_frame_count = stats.counters.ui_frame_count.saturating_add(1);
        stats.last_ui_frame_ns = Some(now_ns);
        if paints.main_view_built == Some(true) {
            stats.counters.main_view_paint_count =
                stats.counters.main_view_paint_count.saturating_add(1);
        }
        if paints.rail_present == Some(true) {
            stats.counters.rail_present_frame_count =
                stats.counters.rail_present_frame_count.saturating_add(1);
        }
        if paints.rail_built == Some(true) {
            stats.counters.rail_paint_count = stats.counters.rail_paint_count.saturating_add(1);
        }
        stats.last_ui_paints = paints;
    }

    pub(super) fn submitted_native_paint(&mut self, sequence: Option<u64>, now: Instant) {
        let now_ns = self.elapsed_ns_at(now);
        if let Some(rows) = self.record_native_paint_at(sequence, now_ns) {
            self.emit_samples(rows);
        }
    }

    pub(super) fn record_native_paint_at(
        &mut self,
        sequence: Option<u64>,
        now_ns: u64,
    ) -> Option<[Option<Value>; 3]> {
        let due = {
            let stats = self.active.as_mut()?;
            stats.counters.native_paint_count =
                stats.counters.native_paint_count.saturating_add(1);
            stats.counters.native_paint_sequence = sequence;
            stats.last_native_paint_ui_frame = Some(stats.counters.ui_frame_count);
            if stats.last_ui_paints.main_view_built == Some(true) {
                stats.counters.submitted_main_view_paint_count = stats
                    .counters
                    .submitted_main_view_paint_count
                    .saturating_add(1);
            }
            if stats.last_ui_paints.rail_built == Some(true) {
                stats.counters.submitted_rail_paint_count = stats
                    .counters
                    .submitted_rail_paint_count
                    .saturating_add(1);
            }
            now_ns.saturating_sub(stats.baseline.monotonic_ns) >= SAMPLE_INTERVAL_NS
        };
        due.then(|| self.sample_rows_at(now_ns))
    }

    pub(super) fn error(&mut self, kind: &'static str) {
        let now_ns = self.elapsed_ns_at(Instant::now());
        let row = self.record_error_at(kind, now_ns);
        self.emit(row);
    }

    pub(super) fn record_error_at(&mut self, kind: &'static str, now_ns: u64) -> Value {
        let event_order = self.next_event_order();
        let window_id = self.active.as_ref().map(|stats| stats.identity.as_str());
        json!({
            "schema": SCHEMA,
            "record": "window_lifecycle",
            "kind": "error",
            "event_order": event_order,
            "elapsed_ns": now_ns,
            "clock_origin_unix_ns": self.clock_origin_unix_ns,
            "window_id": window_id,
            "status": "error",
            "error_kind": kind,
        })
    }

    pub(super) fn close(&mut self, reason: &'static str) {
        let now_ns = self.elapsed_ns_at(Instant::now());
        if self.active.is_some() {
            let rows = self.sample_rows_at(now_ns);
            self.emit_samples(rows);
            if let Some(row) = self.record_close_at(reason, now_ns) {
                self.emit(row);
            }
        }
    }

    pub(super) fn record_close_at(&mut self, reason: &'static str, now_ns: u64) -> Option<Value> {
        let stats = self.active.take()?;
        let event_order = self.next_event_order();
        Some(json!({
            "schema": SCHEMA,
            "record": "window_lifecycle",
            "kind": "close",
            "event_order": event_order,
            "elapsed_ns": now_ns,
            "clock_origin_unix_ns": stats.clock_origin_unix_ns,
            "window_id": stats.identity,
            "status": "closed",
            "reason": reason,
        }))
    }

    fn sample_rows_at(&mut self, end_ns: u64) -> [Option<Value>; 3] {
        let start = self.active.as_ref().expect("sample requires an active window");
        let start_counters = start.baseline.counters;
        let end_counters = start.counters;
        let include_pump = end_counters.about_to_wait_count > start_counters.about_to_wait_count;
        let ui_frame_progress = end_counters.ui_frame_count > start_counters.ui_frame_count;
        let include_main = ui_frame_progress && start.last_ui_paints.main_view_built.is_some();
        let include_rail = ui_frame_progress && start.last_ui_paints.rail_present.is_some();
        let start_ns = start.baseline.monotonic_ns;
        let orders = [
            include_pump.then(|| self.next_event_order()),
            include_main.then(|| self.next_event_order()),
            include_rail.then(|| self.next_event_order()),
        ];
        let stats = self.active.as_mut().expect("sample requires an active window");
        let rows = [
            orders[0].map(|order| stats.pump_sample_row(order, start_ns, end_ns)),
            orders[1].map(|order| {
                stats.ui_paint_sample_row(order, start_ns, end_ns, "main_view")
            }),
            orders[2].map(|order| {
                stats.ui_paint_sample_row(order, start_ns, end_ns, "sidecar_rail")
            }),
        ];
        stats.baseline = SampleBaseline {
            monotonic_ns: end_ns,
            counters: stats.counters,
            surfaces: stats.last_ui_paints,
        };
        rows
    }

    fn emit_samples(&self, rows: [Option<Value>; 3]) {
        for row in rows.into_iter().flatten() {
            self.emit(row);
        }
    }

    fn emit(&self, row: Value) {
        eprintln!("{row}");
    }
}

pub(super) fn error_kind(error: &PanelError) -> &'static str {
    match error {
        PanelError::WindowCreation(_) => "native_window_creation",
        PanelError::SurfaceCreation(_) => "surface_creation",
        PanelError::AdapterUnavailable(_) => "adapter_unavailable",
        PanelError::DeviceRequest(_) => "device_request",
        PanelError::GpuThread(_) => "gpu_thread_start",
        PanelError::GpuInitLost => "gpu_init_lost",
        PanelError::RendererInit(_) => "renderer_init",
        PanelError::FramePrepare(_) => "frame_prepare",
        PanelError::Render(_) => "render",
        PanelError::SurfaceValidation => "surface_validation",
        PanelError::EventLoop(_) => "event_loop",
        PanelError::LiveHarnessFailure(_) => "live_harness",
        PanelError::MemoryRunFailure(_) => "memory_run",
        PanelError::StartupFailure(_) => "startup",
    }
}

fn redraw_mode_name(mode: RedrawMode) -> &'static str {
    match mode {
        RedrawMode::Poll => "Poll",
        RedrawMode::Wait => "Wait",
        RedrawMode::WaitUntil { .. } => "WaitUntil",
    }
}

fn duration_ns(duration: Duration) -> u64 {
    duration.as_nanos().min(u64::MAX as u128) as u64
}
