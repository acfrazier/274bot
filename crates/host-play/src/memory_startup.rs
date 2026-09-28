//! Process-relative startup timeline for the memory-profile harness.
//!
//! Every timestamp is nanoseconds from the process epoch a frontend `main`
//! records with [`mark_process_start`] (first use when a caller never did),
//! held in atomics: no per-frame allocation, no logging, and a first-wins
//! store that costs one relaxed load once a milestone has been reached. The
//! receipt runner combines `process_epoch_unix_ms` with its own launch epoch
//! to place these milestones against process creation.
//!
//! The panel frame histogram only times inside `Window::render`. The render
//! gap tracker times the space between render callbacks, which is where an OS
//! "Not Responding" interval lives: the UI thread is elsewhere and not
//! pumping messages.

use parking_lot::Mutex;
use serde_json::{json, Map, Value};
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering::Relaxed};
use std::sync::LazyLock;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

/// Windows marks a window unresponsive after roughly five seconds without
/// message pumping; a render gap that long is a candidate hang interval.
const OS_HANG_GAP_NS: u64 = 5_000_000_000;

struct ProcessStart {
    at: Instant,
    unix_ms: u64,
}

static PROCESS_START: LazyLock<ProcessStart> = LazyLock::new(|| ProcessStart {
    at: Instant::now(),
    unix_ms: SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis() as u64),
});

fn process_start() -> &'static ProcessStart {
    &PROCESS_START
}

/// Record the process epoch. Frontend `main`s call this first so startup
/// milestones count from process entry; a later first use is only a fallback.
pub fn mark_process_start() {
    LazyLock::force(&PROCESS_START);
}

/// Nanoseconds from the process epoch to `at`, never zero (zero means unset).
pub(crate) fn since_process_start_ns(at: Instant) -> u64 {
    u64::try_from(at.saturating_duration_since(process_start().at).as_nanos())
        .unwrap_or(u64::MAX)
        .max(1)
}

/// Startup milestones, each stored at its first occurrence.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StartupMark {
    /// The OS window exists.
    WindowCreated,
    /// Adapter, device, surface and the ImGui renderer are ready.
    GpuReady,
    /// The first render callback began.
    FirstRenderStart,
    /// The first swapchain image was presented.
    FirstFramePresented,
    /// Any client slot ran its first per-frame hook.
    FirstClientFrame,
}

const MARKS: usize = 5;

struct StartupMarks([AtomicU64; MARKS]);

impl StartupMarks {
    const fn new() -> Self {
        Self([const { AtomicU64::new(0) }; MARKS])
    }

    /// First occurrence wins; later calls are one relaxed load.
    fn mark(&self, mark: StartupMark, ns: u64) {
        let slot = &self.0[mark as usize];
        if slot.load(Relaxed) == 0 {
            let _ = slot.compare_exchange(0, ns.max(1), Relaxed, Relaxed);
        }
    }

    fn reached(&self, mark: StartupMark) -> bool {
        self.0[mark as usize].load(Relaxed) != 0
    }

    fn seconds(&self, mark: StartupMark) -> Option<f64> {
        match self.0[mark as usize].load(Relaxed) {
            0 => None,
            ns => Some(ns as f64 / 1e9),
        }
    }
}

static MARKS_SEEN: StartupMarks = StartupMarks::new();

/// Record a startup milestone now (first occurrence only). Per-frame callers
/// pay one relaxed load once the milestone is reached: the clock is read only
/// while it is still unset.
pub fn mark_startup(mark: StartupMark) {
    if !MARKS_SEEN.reached(mark) {
        MARKS_SEEN.mark(mark, since_process_start_ns(Instant::now()));
    }
}

/// Time between the end of one render callback and the start of the next.
struct RenderGapTracker {
    /// Process-relative end of the previous render; zero before the first.
    last_end_ns: AtomicU64,
    gaps: AtomicU64,
    max_ns: AtomicU64,
    /// Process-relative time the longest gap ended.
    max_at_ns: AtomicU64,
    /// Longest gap since the last sample took it.
    interval_max_ns: AtomicU64,
    over_hang: AtomicU64,
}

impl RenderGapTracker {
    const fn new() -> Self {
        Self {
            last_end_ns: AtomicU64::new(0),
            gaps: AtomicU64::new(0),
            max_ns: AtomicU64::new(0),
            max_at_ns: AtomicU64::new(0),
            interval_max_ns: AtomicU64::new(0),
            over_hang: AtomicU64::new(0),
        }
    }

    /// A render callback began at `now_ns`. The first render has no gap: the
    /// wait before it is the `FirstRenderStart` milestone.
    fn render_started(&self, now_ns: u64) {
        let last_end = self.last_end_ns.load(Relaxed);
        if last_end == 0 {
            return;
        }
        let gap = now_ns.saturating_sub(last_end);
        self.gaps.fetch_add(1, Relaxed);
        self.interval_max_ns.fetch_max(gap, Relaxed);
        if gap > self.max_ns.fetch_max(gap, Relaxed) {
            self.max_at_ns.store(now_ns, Relaxed);
        }
        if gap >= OS_HANG_GAP_NS {
            self.over_hang.fetch_add(1, Relaxed);
        }
    }

    fn render_ended(&self, now_ns: u64) {
        self.last_end_ns.store(now_ns.max(1), Relaxed);
    }

    /// Longest gap since the previous call; `None` before any gap exists.
    fn take_interval_max_ns(&self) -> Option<u64> {
        let seen = self.gaps.load(Relaxed) > 0;
        let max = self.interval_max_ns.swap(0, Relaxed);
        seen.then_some(max)
    }
}

static RENDER_GAPS: RenderGapTracker = RenderGapTracker::new();

/// A render callback began now.
pub(crate) fn render_started(at: Instant) {
    let ns = since_process_start_ns(at);
    MARKS_SEEN.mark(StartupMark::FirstRenderStart, ns);
    RENDER_GAPS.render_started(ns);
}

/// A render callback ended now; `presented` when its image reached the
/// swapchain (a lost, outdated or occluded surface presents nothing).
pub(crate) fn render_ended(at: Instant, presented: bool) {
    let ns = since_process_start_ns(at);
    if presented {
        MARKS_SEEN.mark(StartupMark::FirstFramePresented, ns);
    }
    RENDER_GAPS.render_ended(ns);
}

/// Structured wgpu adapter identity. Vendor and device are backend ids
/// (PCI ids on Vulkan/DX12/Metal); the strings are the driver's own.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdapterRecord {
    pub name: String,
    pub backend: String,
    pub device_type: String,
    pub driver: String,
    pub driver_info: String,
    pub vendor: u32,
    pub device: u32,
    pub pci_bus_id: String,
}

impl AdapterRecord {
    pub fn to_json(&self) -> Value {
        json!({
            "name": self.name,
            "backend": self.backend,
            "device_type": self.device_type,
            "driver": self.driver,
            "driver_info": self.driver_info,
            "vendor": format!("{:#06x}", self.vendor),
            "device": format!("{:#06x}", self.device),
            "pci_bus_id": self.pci_bus_id,
        })
    }
}

/// The latest adapter the panel selected and how many times it selected one.
/// A device-loss rebuild selects again; the latest is the one measured, and
/// a count above one shows the GPU stack was rebuilt.
struct AdapterSlot {
    latest: Mutex<Option<AdapterRecord>>,
    selections: AtomicU32,
}

impl AdapterSlot {
    const fn new() -> Self {
        Self {
            latest: Mutex::new(None),
            selections: AtomicU32::new(0),
        }
    }

    fn record(&self, record: AdapterRecord) {
        *self.latest.lock() = Some(record);
        self.selections.fetch_add(1, Relaxed);
    }

    fn snapshot(&self) -> (Option<AdapterRecord>, u32) {
        (self.latest.lock().clone(), self.selections.load(Relaxed))
    }
}

static ADAPTER: AdapterSlot = AdapterSlot::new();

/// Record the adapter the panel selected. One-time metadata for the run
/// record; it writes no log line.
pub fn record_adapter(record: AdapterRecord) {
    ADAPTER.record(record);
}

/// Everything a sample row needs from the startup timeline.
fn timeline_fields(
    process_epoch_unix_ms: u64,
    run_started_ns: u64,
    marks: &StartupMarks,
    gaps: &RenderGapTracker,
    interval_max_ns: Option<u64>,
    adapter: (Option<AdapterRecord>, u32),
) -> Map<String, Value> {
    let mut fields = Map::new();
    fields.insert("process_epoch_unix_ms".into(), process_epoch_unix_ms.into());
    fields.insert(
        "run_started_process_s".into(),
        (run_started_ns as f64 / 1e9).into(),
    );
    for (key, mark) in [
        ("startup_window_created_s", StartupMark::WindowCreated),
        ("startup_gpu_ready_s", StartupMark::GpuReady),
        ("startup_first_render_s", StartupMark::FirstRenderStart),
        (
            "startup_first_frame_presented_s",
            StartupMark::FirstFramePresented,
        ),
        (
            "startup_first_client_frame_s",
            StartupMark::FirstClientFrame,
        ),
    ] {
        fields.insert(key.into(), marks.seconds(mark).into());
    }
    let measured = gaps.gaps.load(Relaxed);
    fields.insert("render_gap_samples".into(), measured.into());
    let cumulative = (measured > 0).then(|| gaps.max_ns.load(Relaxed));
    fields.insert("render_gap_max_ns".into(), cumulative.into());
    fields.insert(
        "render_gap_max_at_s".into(),
        cumulative
            .map(|_| gaps.max_at_ns.load(Relaxed) as f64 / 1e9)
            .into(),
    );
    fields.insert("render_gap_interval_max_ns".into(), interval_max_ns.into());
    fields.insert(
        "render_gaps_ge_5s".into(),
        (measured > 0).then(|| gaps.over_hang.load(Relaxed)).into(),
    );
    let (record, selections) = adapter;
    fields.insert(
        "adapter".into(),
        record.as_ref().map_or(Value::Null, AdapterRecord::to_json),
    );
    fields.insert("adapter_selections".into(), selections.into());
    fields
}

/// Add the startup timeline to one sample row. Takes the render-gap interval
/// maximum, so call it exactly once per row.
pub(crate) fn add_sample_fields(row: &mut Value, run_started: Instant) {
    let fields = timeline_fields(
        process_start().unix_ms,
        since_process_start_ns(run_started),
        &MARKS_SEEN,
        &RENDER_GAPS,
        RENDER_GAPS.take_interval_max_ns(),
        ADAPTER.snapshot(),
    );
    for (key, value) in fields {
        row[key] = value;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MS: u64 = 1_000_000;
    const SECOND: u64 = 1_000 * MS;

    /// The first render has no preceding callback, so it measures no gap; the
    /// wait before it is its own milestone. Every later gap runs from the end
    /// of one render to the start of the next, so time spent *inside* a slow
    /// render never counts as a gap.
    #[test]
    fn gap_is_measured_between_renders_not_inside_them() {
        let gaps = RenderGapTracker::new();
        gaps.render_started(10 * MS);
        gaps.render_ended(10 * MS + 400 * MS);
        assert_eq!(
            gaps.gaps.load(Relaxed),
            0,
            "no gap before the second render"
        );
        assert_eq!(gaps.take_interval_max_ns(), None);

        gaps.render_started(500 * MS);
        assert_eq!(gaps.max_ns.load(Relaxed), 90 * MS);
        assert_eq!(gaps.max_at_ns.load(Relaxed), 500 * MS);
    }

    /// An interval maximum belongs to one sample; the cumulative maximum, when
    /// it ended, and the count of gaps at or beyond the OS hang threshold
    /// survive it so a startup hang is still visible after the run recovers.
    #[test]
    fn a_hang_sized_gap_outlives_the_sample_interval_that_saw_it() {
        let gaps = RenderGapTracker::new();
        gaps.render_started(SECOND);
        gaps.render_ended(SECOND + 5 * MS);
        gaps.render_started(SECOND + 25 * MS); // 20 ms gap
        gaps.render_ended(SECOND + 30 * MS);
        gaps.render_started(SECOND + 30 * MS + 6 * SECOND); // 6 s gap
        gaps.render_ended(8 * SECOND);
        assert_eq!(gaps.take_interval_max_ns(), Some(6 * SECOND));

        gaps.render_started(8 * SECOND + 16 * MS); // 16 ms gap
        assert_eq!(
            gaps.take_interval_max_ns(),
            Some(16 * MS),
            "the next sample sees only its own gaps"
        );
        assert_eq!(gaps.max_ns.load(Relaxed), 6 * SECOND);
        assert_eq!(gaps.max_at_ns.load(Relaxed), 7 * SECOND + 30 * MS);
        assert_eq!(gaps.over_hang.load(Relaxed), 1);

        gaps.render_ended(9 * SECOND);
        gaps.render_started(9 * SECOND + OS_HANG_GAP_NS);
        assert_eq!(
            gaps.over_hang.load(Relaxed),
            2,
            "a gap of exactly the threshold counts"
        );
    }

    #[test]
    fn a_milestone_keeps_its_first_occurrence() {
        let marks = StartupMarks::new();
        assert_eq!(marks.seconds(StartupMark::FirstClientFrame), None);
        marks.mark(StartupMark::FirstClientFrame, 3 * SECOND);
        marks.mark(StartupMark::FirstClientFrame, 9 * SECOND);
        assert_eq!(marks.seconds(StartupMark::FirstClientFrame), Some(3.0));
        assert_eq!(
            marks.seconds(StartupMark::FirstFramePresented),
            None,
            "milestones are independent"
        );
    }

    /// A later adapter selection means the GPU stack was rebuilt; the record
    /// reports the adapter in use now and that it changed.
    #[test]
    fn a_rebuilt_gpu_stack_reports_the_latest_adapter_and_the_rebuild() {
        let adapter = |name: &str| AdapterRecord {
            name: name.into(),
            backend: "Vulkan".into(),
            device_type: "DiscreteGpu".into(),
            driver: "driver".into(),
            driver_info: "1.2.3".into(),
            vendor: 0x10de,
            device: 0x2684,
            pci_bus_id: "0000:01:00.0".into(),
        };
        let slot = AdapterSlot::new();
        assert_eq!(slot.snapshot(), (None, 0));
        slot.record(adapter("first"));
        slot.record(adapter("second"));
        let (latest, selections) = slot.snapshot();
        assert_eq!(latest.map(|record| record.name), Some("second".into()));
        assert_eq!(selections, 2);
    }

    /// The row a Windows panel would write after a slow startup: process
    /// epoch, milestones, the hang-sized gap with when it ended, and the
    /// adapter. A headless run writes nulls, never zeros, for the panel-only
    /// fields.
    #[test]
    fn sample_fields_carry_the_startup_timeline_and_null_when_headless() {
        let marks = StartupMarks::new();
        marks.mark(StartupMark::WindowCreated, 400 * MS);
        marks.mark(StartupMark::FirstFramePresented, 1_100 * MS);
        marks.mark(StartupMark::FirstClientFrame, 381 * SECOND);
        let gaps = RenderGapTracker::new();
        gaps.render_started(1_000 * MS);
        gaps.render_ended(1_050 * MS);
        gaps.render_started(1_050 * MS + 7 * SECOND);
        let adapter = AdapterRecord {
            name: "Test GPU".into(),
            backend: "Dx12".into(),
            device_type: "IntegratedGpu".into(),
            driver: "drv".into(),
            driver_info: "info".into(),
            vendor: 0x8086,
            device: 0x46a6,
            pci_bus_id: String::new(),
        };
        let panel = timeline_fields(
            1_700_000_000_123,
            380 * SECOND,
            &marks,
            &gaps,
            gaps.take_interval_max_ns(),
            (Some(adapter), 1),
        );
        assert_eq!(panel["process_epoch_unix_ms"], 1_700_000_000_123_u64);
        assert_eq!(panel["run_started_process_s"], 380.0);
        assert_eq!(panel["startup_window_created_s"], 0.4);
        assert_eq!(panel["startup_first_frame_presented_s"], 1.1);
        assert_eq!(panel["startup_first_client_frame_s"], 381.0);
        assert!(panel["startup_gpu_ready_s"].is_null(), "unreached is null");
        assert_eq!(panel["render_gap_max_ns"], 7 * SECOND);
        assert_eq!(panel["render_gap_max_at_s"], 8.05);
        assert_eq!(panel["render_gap_interval_max_ns"], 7 * SECOND);
        assert_eq!(panel["render_gaps_ge_5s"], 1);
        assert_eq!(panel["adapter"]["name"], "Test GPU");
        assert_eq!(panel["adapter"]["backend"], "Dx12");
        assert_eq!(panel["adapter"]["vendor"], "0x8086");
        assert_eq!(panel["adapter_selections"], 1);

        let headless = timeline_fields(
            1,
            2 * SECOND,
            &StartupMarks::new(),
            &RenderGapTracker::new(),
            None,
            (None, 0),
        );
        for key in [
            "startup_window_created_s",
            "startup_first_render_s",
            "startup_first_frame_presented_s",
            "render_gap_max_ns",
            "render_gap_max_at_s",
            "render_gap_interval_max_ns",
            "render_gaps_ge_5s",
            "adapter",
        ] {
            assert!(headless[key].is_null(), "{key} must be null headless");
        }
        assert_eq!(headless["render_gap_samples"], 0);
        assert_eq!(headless["adapter_selections"], 0);
    }
}
