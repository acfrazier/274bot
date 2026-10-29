//! M3: render contention at N heads (contract §23 §2.5 / §21 measurements),
//! supplemental Linux evidence for the T4 track. Mac FLOOR stays the
//! qualification gate; nothing here is a FLOOR number.
//!
//! N bots log in to Engine A through the production `Play` path, each slot on
//! its own thread with a drawing head (the per-frame hook applies
//! `set_draw(true)`, as the panel's focus switch does). A consumer thread
//! takes every mailbox on its UI period and runs the independent render pump
//! on that same thread. Unset `M3_FAIR_PROBE` retains the legacy full/focus1
//! cadence and grouping. With the explicit hook probe, head 0 is the focused
//! selection in both policies while `full` sets every cadence to full-rate and
//! `focus1` sets only head 0 to full-rate.
//! Per cell the harness writes a JSON summary and per-second samples:
//! paints per head, host raster time per paint (the `performance-profile`
//! slot timers, which `memory-profile` enables), slot frame-interval
//! histograms, process CPU, RSS and tracked GPU bytes. Raster time is
//! lossless: every row carries the raw raster and paint deltas between two
//! raster publication boundaries (see [`RasterPublications`]), so the
//! per-second deltas sum exactly to the cell totals. Run one backend and
//! one N per process:
//!
//! ```sh
//! isohome env LIVE=1 BOT_LIVE_NAME_PREFIX=m3 M3_N=4 M3_BACKEND=gpu \
//!   M3_OUT=/path/to/evidence ENGINE_DIR=... NAV_PACK=... \
//!   cargo test --release -p host-play --features memory-profile \
//!   --test m3_render_contention -- --ignored --nocapture
//! ```
//!
//! `M3_BACKEND=cpu` also needs `BOT_CPU=1`. Optional: `M3_WARMUP_S` (10),
//! `M3_OBSERVE_S` (30), `M3_POLICIES` (`full,focus1`), `M3_PORT` (44594),
//! `M3_HTTP_PORT` (1080).
//!
//! The hook-enabled probe is opt-in and never supplies TTL or quantum defaults:
//! set `M3_FAIR_PROBE=1`, `M3_GRANT_TTL_MS`, and `M3_SCENE_QUANTUM_MS`, and
//! build with `--features memory-profile,render-test-hooks`. Its receipts are
//! labelled PROVISIONAL and are not production qualification.
#![cfg(feature = "memory-profile")]

use std::collections::HashMap;
use std::io::Write as _;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering::Relaxed};
use std::sync::{Arc, LazyLock, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use host::render_head;
use host::{FrameBuf, SlotInput};
use host_play::{
    current_resident_bytes, mint_live_names, run_with_template, sample_process, Play,
    ProfileOptions, SharedClientTemplate,
};
use serde_json::{json, Value};
use vault::{Profile, ProfileSettings, RasterMode};

/// Frame-interval histogram buckets: 1 ms ceilings 0..249, then 250+.
const BUCKETS: usize = 251;
/// The panel's frame-consumer period (its UI loop at ~60 Hz).
const CONSUMER_PERIOD: Duration = Duration::from_millis(16);

fn env_or<T: std::str::FromStr>(key: &str, default: T) -> T {
    std::env::var(key)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
}

fn required(key: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| panic!("{key} is required"))
}

#[derive(Clone, Copy)]
struct FairProbeInputs {
    grant_ttl_ms: u64,
    scene_quantum_ms: u64,
}

fn fair_probe_inputs() -> Option<FairProbeInputs> {
    let enabled = match std::env::var("M3_FAIR_PROBE") {
        Ok(value) if value == "1" => true,
        Ok(value) if value == "0" => false,
        Ok(value) => panic!("M3_FAIR_PROBE must be 0 or 1, got {value}"),
        Err(_) => false,
    };
    if !enabled {
        return None;
    }
    let grant_ttl_ms = required("M3_GRANT_TTL_MS")
        .parse::<u64>()
        .expect("M3_GRANT_TTL_MS must be a positive integer number of milliseconds");
    let scene_quantum_ms = required("M3_SCENE_QUANTUM_MS")
        .parse::<u64>()
        .expect("M3_SCENE_QUANTUM_MS must be a positive integer number of milliseconds");
    assert!(grant_ttl_ms > 0, "M3_GRANT_TTL_MS must be positive");
    assert!(scene_quantum_ms > 0, "M3_SCENE_QUANTUM_MS must be positive");
    Some(FairProbeInputs {
        grant_ttl_ms,
        scene_quantum_ms,
    })
}

fn configure_fair_probe(inputs: Option<FairProbeInputs>) {
    let Some(inputs) = inputs else {
        return;
    };
    #[cfg(feature = "render-test-hooks")]
    render_head::test_hooks::configure_process(render_head::test_hooks::ProcessFixture {
        grant_ttl: Duration::from_millis(inputs.grant_ttl_ms),
        quantum: Duration::from_millis(inputs.scene_quantum_ms),
    })
    .expect("configure M3 render process before its first use");
    #[cfg(not(feature = "render-test-hooks"))]
    panic!(
        "M3_FAIR_PROBE=1 requires host-play --features render-test-hooks; inputs were {:?} ms TTL and {:?} ms quantum",
        inputs.grant_ttl_ms, inputs.scene_quantum_ms
    );
}

fn add_probe_receipt(receipt: &mut Value, inputs: Option<FairProbeInputs>) {
    if let Some(inputs) = inputs {
        receipt["render_probe"] = json!({
            "mode": "hook-enabled",
            "qualification": "PROVISIONAL",
            "inputs_ms": {
                "M3_GRANT_TTL_MS": inputs.grant_ttl_ms,
                "M3_SCENE_QUANTUM_MS": inputs.scene_quantum_ms,
            },
        });
    }
}

#[derive(Clone, Copy)]
struct GrantLatencyDurations {
    publication_to_notice: Duration,
    notice_to_wake: Duration,
    publication_to_wake: Duration,
    wake_to_consume: Duration,
    publication_to_consume: Duration,
}

fn checked_grant_latency_durations(
    published_at: Instant,
    notified_at: Instant,
    woke_at: Instant,
    consumed_at: Instant,
) -> Option<GrantLatencyDurations> {
    Some(GrantLatencyDurations {
        publication_to_notice: notified_at.checked_duration_since(published_at)?,
        notice_to_wake: woke_at.checked_duration_since(notified_at)?,
        publication_to_wake: woke_at.checked_duration_since(published_at)?,
        wake_to_consume: consumed_at.checked_duration_since(woke_at)?,
        publication_to_consume: consumed_at.checked_duration_since(published_at)?,
    })
}

#[test]
fn watch_grant_wake_latency_is_measured_from_publication() {
    let published_at = Instant::now();
    let notified_at = published_at + Duration::from_millis(7);
    let woke_at = notified_at + Duration::from_millis(11);
    let consumed_at = woke_at + Duration::from_millis(13);
    let measured = checked_grant_latency_durations(published_at, notified_at, woke_at, consumed_at)
        .expect("monotonic grant timestamps");
    assert_eq!(measured.publication_to_notice, Duration::from_millis(7));
    assert_eq!(measured.notice_to_wake, Duration::from_millis(11));
    assert_eq!(measured.publication_to_wake, Duration::from_millis(18));
    assert_eq!(measured.publication_to_consume, Duration::from_millis(31));
    assert_ne!(
        measured.publication_to_consume, measured.wake_to_consume,
        "the measured grant latency must begin at publication, not at wake"
    );
}

/// Slot-thread frame intervals for one head, written by the per-frame hook.
struct SlotFrames {
    last: Mutex<Option<Instant>>,
    frames: AtomicU64,
    histogram: [AtomicU64; BUCKETS],
}

impl SlotFrames {
    fn new() -> Self {
        Self {
            last: Mutex::new(None),
            frames: AtomicU64::new(0),
            histogram: [const { AtomicU64::new(0) }; BUCKETS],
        }
    }

    fn record(&self, now: Instant) {
        let previous = self.last.lock().unwrap().replace(now);
        self.frames.fetch_add(1, Relaxed);
        if let Some(previous) = previous {
            let ms = now.duration_since(previous).as_micros().div_ceil(1000) as usize;
            self.histogram[ms.min(BUCKETS - 1)].fetch_add(1, Relaxed);
        }
    }

    fn histogram(&self) -> Vec<u64> {
        self.histogram.iter().map(|b| b.load(Relaxed)).collect()
    }
}

/// Origin of every `t_s` this harness writes outside a cell's own clock.
static EPOCH: LazyLock<Instant> = LazyLock::new(Instant::now);

fn since_epoch(at: Instant) -> f64 {
    at.duration_since(*EPOCH).as_secs_f64()
}

/// One raster publication boundary: the slot's published cumulative raster
/// time and the number of paints it covers.
#[derive(Clone, Copy, Default)]
struct Publication {
    /// Publications seen so far (0 = none yet).
    seq: u64,
    raster_ns: u64,
    paints: u64,
    /// When the per-frame hook saw it.
    at: Option<Instant>,
}

impl Publication {
    fn json(&self) -> Value {
        json!({
            "seq": self.seq,
            "raster_ns": self.raster_ns,
            "paints": self.paints,
            "t_s": self.at.map(since_epoch),
        })
    }
}

/// Raster time paired exactly with the paints it covers.
///
/// The host publishes a slot's cumulative raster time at most once a second
/// (`SlotLoop::publish_profile`), inside the slot frame: after that frame's
/// paint and before its head's `FrameBuf::publish`. The per-frame hook runs in the
/// next slot frame's observe, before that frame's paint, and on every slot
/// frame. So when the hook first sees a new published total, the mailbox
/// generation is exactly the number of paints that total covers. Both
/// counters are written only by the slot thread, which also runs the hook.
/// A publication with no paint since the last one repeats the last total
/// and the same paint count, so it is not a new boundary.
struct RasterPublications {
    mailbox: Arc<FrameBuf>,
    last: Mutex<Publication>,
}

impl RasterPublications {
    fn new(mailbox: Arc<FrameBuf>) -> Self {
        Self {
            mailbox,
            last: Mutex::new(Publication::default()),
        }
    }

    /// Slot thread, every frame: record a new publication boundary.
    fn observe(&self, name: &str) {
        let Some(published) = host::performance_profile::snapshots()
            .into_iter()
            .find(|t| t.username == name)
            .map(|t| t.raster_ns)
        else {
            return;
        };
        let mut last = self.last.lock().unwrap();
        if published != last.raster_ns {
            *last = Publication {
                seq: last.seq + 1,
                raster_ns: published,
                paints: self.mailbox.generation(),
                at: Some(Instant::now()),
            };
        }
    }

    fn last(&self) -> Publication {
        *self.last.lock().unwrap()
    }
}

/// One census of every slot's retained sim world, taken on the slot thread
/// by the per-frame hook (a `Client` never leaves its slot).
#[derive(Default)]
struct Census {
    phase: Mutex<Option<&'static str>>,
    rows: Mutex<Vec<Value>>,
}

impl Census {
    /// Request `phase` from every slot and wait for `n` rows.
    fn run(&self, phase: &'static str, n: usize) -> Vec<Value> {
        self.rows.lock().unwrap().clear();
        *self.phase.lock().unwrap() = Some(phase);
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            let done = self.rows.lock().unwrap().len();
            if done >= n {
                break;
            }
            assert!(Instant::now() < deadline, "census {phase}: {done}/{n}");
            thread::sleep(Duration::from_millis(50));
        }
        *self.phase.lock().unwrap() = None;
        std::mem::take(&mut *self.rows.lock().unwrap())
    }

    fn observe(&self, client: &client::client::Client, name: &str) {
        let Some(phase) = *self.phase.lock().unwrap() else {
            return;
        };
        if !client.ingame || client.scene_state != 2 {
            return;
        }
        let mut rows = self.rows.lock().unwrap();
        if rows.iter().any(|row| row["head"] == name) {
            return;
        }
        let mut row = world_census(client);
        row["head"] = json!(name);
        row["phase"] = json!(phase);
        rows.push(row);
    }
}

/// Counts of the sim world's per-tile records, split by what reads them.
fn world_census(client: &client::client::Client) -> Value {
    use client::dash3d::ground::{Ground, GroundStamp};
    use client::dash3d::square::Square;
    use client::dash3d::BuildArea;
    let mut squares = 0usize;
    let mut stamps = 0usize;
    let mut grounds = 0usize;
    let mut quick_grounds = 0usize;
    let mut locs = 0usize;
    for level in 0..4 {
        for x in 0..BuildArea::SIZE {
            for z in 0..BuildArea::SIZE {
                let mut cursor = client.world.square(level, x, z);
                while let Some(square) = cursor {
                    squares += 1;
                    stamps += usize::from(square.overlay_stamp.is_some());
                    grounds += usize::from(square.ground.is_some());
                    quick_grounds += usize::from(square.quick_ground.is_some());
                    locs += usize::from(square.wall.is_some())
                        + usize::from(square.decor.is_some())
                        + usize::from(square.ground_decor.is_some());
                    cursor = square.linked_square.as_deref();
                }
            }
        }
    }
    json!({
        "squares": squares,
        "square_bytes": squares * std::mem::size_of::<Square>(),
        "overlay_stamps": stamps,
        "overlay_stamp_bytes": stamps * std::mem::size_of::<GroundStamp>(),
        "ground_meshes": grounds,
        "ground_mesh_inline_bytes": grounds * std::mem::size_of::<Ground>(),
        "quick_grounds": quick_grounds,
        "wall_decor_ground_decor": locs,
        "overlay_pending": client.world.overlay_pending,
        "share_light_pending": client.world.share_light_pending,
        "draw": client.draw,
        "dynamic_sprite_storage_bytes": client.world.dynamic_sprite_storage_bytes(),
    })
}

/// Fixed world and head draw-scratch sizes, separated by their actual owner.
fn static_world_bytes() -> Value {
    use client::core::world::{MAX_DYNAMIC_SPRITES, MAX_OCCLUDERS, OCCLUDER_LEVELS};
    use client::dash3d::occlude::Occlude;
    use client::dash3d::BuildArea;
    let side = BuildArea::SIZE as usize + 1;
    json!({
        "occluders": OCCLUDER_LEVELS * MAX_OCCLUDERS * std::mem::size_of::<Option<Occlude>>(),
        "occlusion_cycle_zeroed": 0,
        "dynamic_sprite_slots": 0,
        "head_occlusion_cycle_bytes": OCCLUDER_LEVELS * side * side * std::mem::size_of::<i32>(),
        "dynamic_sprite_capacity_max_bytes": MAX_DYNAMIC_SPRITES * std::mem::size_of::<Option<usize>>(),
        "square_size": std::mem::size_of::<client::dash3d::square::Square>(),
    })
}

/// One head's channels and counters.
struct HeadProbe {
    name: String,
    input: Arc<SlotInput>,
    mailbox: Arc<FrameBuf>,
    frames: Arc<SlotFrames>,
    raster: Arc<RasterPublications>,
}

/// Cumulative counters at one instant. `paints` is the mailbox count now;
/// `published` is the last raster publication boundary, whose own paint
/// count pairs with its raster total.
#[derive(Clone)]
struct Mark {
    at: Instant,
    cpu_s: f64,
    paints: Vec<u64>,
    published: Vec<Publication>,
    frames: Vec<u64>,
    histograms: Vec<Vec<u64>>,
    taken: u64,
    workspace: render_head::WorkspaceStats,
    pump: Option<PumpSnapshot>,
    #[cfg(feature = "render-test-hooks")]
    fair: Option<Vec<Option<render_head::HeadFairStats>>>,
    #[cfg(feature = "render-test-hooks")]
    head_identities: Option<Vec<Option<host::render::HeadIdentity>>>,
}

fn mark(heads: &[HeadProbe], taken: &AtomicU64, pump: Option<&PumpProbe>) -> Mark {
    Mark {
        at: Instant::now(),
        cpu_s: sample_process().1,
        paints: heads.iter().map(|h| h.mailbox.generation()).collect(),
        published: heads.iter().map(|h| h.raster.last()).collect(),
        frames: heads
            .iter()
            .map(|h| h.frames.frames.load(Relaxed))
            .collect(),
        histograms: heads.iter().map(|h| h.frames.histogram()).collect(),
        taken: taken.load(Relaxed),
        workspace: render_head::workspace_stats(),
        pump: pump.map(PumpProbe::snapshot),
        #[cfg(feature = "render-test-hooks")]
        fair: pump
            .is_some()
            .then(|| heads.iter().map(|h| h.mailbox.fair_stats()).collect()),
        #[cfg(feature = "render-test-hooks")]
        head_identities: pump
            .is_some()
            .then(|| heads.iter().map(|h| h.mailbox.head()).collect()),
    }
}

/// Actual intervals between calls to the independent render pump.
struct PumpProbe {
    last: Mutex<Option<Instant>>,
    calls: AtomicU64,
    intervals: AtomicU64,
    interval_ns: AtomicU64,
    histogram: [AtomicU64; BUCKETS],
}

#[derive(Clone, Copy)]
struct PumpSnapshot {
    calls: u64,
    intervals: u64,
    interval_ns: u64,
    histogram: [u64; BUCKETS],
}

impl PumpProbe {
    fn new() -> Self {
        Self {
            last: Mutex::new(None),
            calls: AtomicU64::new(0),
            intervals: AtomicU64::new(0),
            interval_ns: AtomicU64::new(0),
            histogram: [const { AtomicU64::new(0) }; BUCKETS],
        }
    }

    fn record(&self, now: Instant) {
        let previous = self.last.lock().unwrap().replace(now);
        self.calls.fetch_add(1, Relaxed);
        let Some(previous) = previous else {
            return;
        };
        let interval = now.saturating_duration_since(previous);
        self.intervals.fetch_add(1, Relaxed);
        self.interval_ns.fetch_add(
            u64::try_from(interval.as_nanos()).unwrap_or(u64::MAX),
            Relaxed,
        );
        let ms = interval.as_micros().div_ceil(1000);
        let bucket = usize::try_from(ms).unwrap_or(usize::MAX).min(BUCKETS - 1);
        self.histogram[bucket].fetch_add(1, Relaxed);
    }

    fn snapshot(&self) -> PumpSnapshot {
        PumpSnapshot {
            calls: self.calls.load(Relaxed),
            intervals: self.intervals.load(Relaxed),
            interval_ns: self.interval_ns.load(Relaxed),
            histogram: std::array::from_fn(|i| self.histogram[i].load(Relaxed)),
        }
    }
}

/// Quantile (ms ceiling) of a 1 ms histogram.
fn quantile(histogram: &[u64], q: f64) -> Option<usize> {
    let total: u64 = histogram.iter().sum();
    if total == 0 {
        return None;
    }
    let rank = ((total as f64) * q).ceil().max(1.0) as u64;
    let mut seen = 0;
    histogram.iter().position(|&count| {
        seen += count;
        seen >= rank
    })
}

fn delta(a: &[u64], b: &[u64]) -> Vec<u64> {
    a.iter().zip(b).map(|(x, y)| y - x).collect()
}

/// Per-group (focused / background) accumulation for one interval.
#[derive(Default)]
struct Group {
    paint_fps: Vec<f64>,
    composite_fps: Vec<f64>,
    scene_fps: Vec<f64>,
    raster_ms: Vec<f64>,
    published_raster_ns: u64,
    published_paints: u64,
    histogram: Vec<u64>,
}

fn pump_metrics(from: Option<PumpSnapshot>, to: Option<PumpSnapshot>) -> Option<Value> {
    let (from, to) = (from?, to?);
    let intervals = to.intervals.saturating_sub(from.intervals);
    let interval_ns = to.interval_ns.saturating_sub(from.interval_ns);
    let histogram = std::array::from_fn(|i| to.histogram[i].saturating_sub(from.histogram[i]));
    Some(json!({
        "schedule": "earlier-of-returned-deadline-and-consumer-period",
        "consumer_period_ms": CONSUMER_PERIOD.as_millis(),
        "pump_calls": to.calls.saturating_sub(from.calls),
        "actual_intervals": intervals,
        "actual_interval_ms_mean": (intervals > 0)
            .then(|| interval_ns as f64 / intervals as f64 / 1e6),
        "actual_interval_ms_p50": quantile(&histogram, 0.50),
        "actual_interval_ms_p95": quantile(&histogram, 0.95),
        "actual_interval_ms_p99": quantile(&histogram, 0.99),
        "actual_interval_ms_max": histogram.iter().rposition(|&count| count > 0),
    }))
}

#[cfg(feature = "render-test-hooks")]
fn fair_head_metrics(
    previous: Option<render_head::HeadFairStats>,
    current: Option<render_head::HeadFairStats>,
    previous_identity: Option<host::render::HeadIdentity>,
    current_identity: Option<host::render::HeadIdentity>,
    seconds: f64,
) -> Value {
    let Some(current) = current else {
        return Value::Null;
    };
    let previous = previous.filter(|stats| stats.identity() == current.identity());
    let interval_identity = previous.is_some()
        && previous_identity == current_identity
        && current_identity == Some(current.identity());
    let count = |current_value: u64, previous_value: Option<u64>| {
        if interval_identity {
            json!(current_value.saturating_sub(previous_value.unwrap_or_default()))
        } else {
            Value::Null
        }
    };
    let duration_ms = |current_value: Duration, previous_value: Option<Duration>| {
        if interval_identity {
            json!(
                current_value
                    .saturating_sub(previous_value.unwrap_or_default())
                    .as_secs_f64()
                    * 1e3
            )
        } else {
            Value::Null
        }
    };
    let consumes = interval_identity.then(|| {
        current
            .scene_consumes()
            .saturating_sub(previous.unwrap().scene_consumes())
    });
    let scene_fps = (current.backend() == client::render::backend::BackendKind::Gpu)
        .then(|| consumes.map(|count| count as f64 / seconds))
        .flatten();
    let latency_metrics =
        |latency: render_head::GrantLatency,
         previous_latency: Option<render_head::GrantLatency>| {
            let is_new = previous_latency
                .map(|previous| previous.reservation() != latency.reservation())
                .unwrap_or(false);
            let published_at = latency.published_at();
            let notified_at = latency.notified_at();
            let woke_at = latency.woke_at();
            let consumed_at = latency.consumed_at();
            let durations =
                checked_grant_latency_durations(published_at, notified_at, woke_at, consumed_at);
            json!({
                "reservation_id": latency.reservation().reservation_id().get(),
                "new_in_interval": is_new,
                "published_at_t_s": since_epoch(published_at),
                "notified_at_t_s": since_epoch(notified_at),
                "woke_at_t_s": since_epoch(woke_at),
                "consumed_at_t_s": since_epoch(consumed_at),
                "timestamps_monotonic": durations.is_some(),
                "publication_to_notice_ms": durations.map(|value| value.publication_to_notice.as_secs_f64() * 1e3),
                "notice_to_wake_ms": durations.map(|value| value.notice_to_wake.as_secs_f64() * 1e3),
                "publication_to_wake_ms": durations.map(|value| value.publication_to_wake.as_secs_f64() * 1e3),
                "wake_to_consume_ms": durations.map(|value| value.wake_to_consume.as_secs_f64() * 1e3),
                "publication_to_consume_ms": durations.map(|value| value.publication_to_consume.as_secs_f64() * 1e3),
            })
        };
    let last_latency = current.last_grant_latency().map(|latency| {
        latency_metrics(
            latency,
            previous.and_then(|stats| stats.last_grant_latency()),
        )
    });
    let max_latency = current.max_grant_latency().map(|latency| {
        latency_metrics(
            latency,
            previous.and_then(|stats| stats.max_grant_latency()),
        )
    });
    let identity = current.identity();
    json!({
        "slot_incarnation": identity.slot().incarnation().get(),
        "head_generation": identity.generation().get(),
        "interval_identity_valid": interval_identity,
        "due_skips": count(current.due_skips(), previous.map(|stats| stats.due_skips())),
        "max_due_skips": current.max_due_skips(),
        "expiry_events": count(current.expiry_events(), previous.map(|stats| stats.expiry_events())),
        "expiry_streak": current.expiry_streak(),
        "max_expiry_streak": current.max_expiry_streak(),
        "busy_paints": count(current.busy_paints(), previous.map(|stats| stats.busy_paints())),
        "busy_paint_ms": duration_ms(current.busy_paint_time(), previous.map(|stats| stats.busy_paint_time())),
        "granted_paints": count(current.granted_paints(), previous.map(|stats| stats.granted_paints())),
        "granted_paint_ms": duration_ms(current.granted_paint_time(), previous.map(|stats| stats.granted_paint_time())),
        "scene_consumes": consumes,
        "grant_delivery_samples": count(current.grant_delivery_samples(), previous.map(|stats| stats.grant_delivery_samples())),
        "grant_delivery_samples_total": current.grant_delivery_samples(),
        "max_grant_to_consume_ms": current.max_grant_to_consume().as_secs_f64() * 1e3,
        "grant_wake_samples": count(current.grant_wake_samples(), previous.map(|stats| stats.grant_wake_samples())),
        "grant_wake_samples_total": current.grant_wake_samples(),
        "scene_releases": count(current.scene_releases(), previous.map(|stats| stats.scene_releases())),
        "scene_lease_ms": duration_ms(current.scene_lease_time(), previous.map(|stats| stats.scene_lease_time())),
        "max_scene_lease_ms": current.max_scene_lease_time().as_secs_f64() * 1e3,
        "scene_fps": scene_fps,
        "oldest_demand_age_ms": current.oldest_demand_age().as_secs_f64() * 1e3,
        "backend": format!("{:?}", current.backend()),
        "has_valid_scene": current.has_valid_scene(),
        "scene_progress": format!("{:?}", current.scene_progress()),
        "last_grant_latency": last_latency,
        "max_grant_latency": max_latency,
    })
}

#[cfg(feature = "render-test-hooks")]
fn fair_workspace_metrics(
    from: render_head::WorkspaceStats,
    to: render_head::WorkspaceStats,
) -> Value {
    let before = from.faults();
    let after = to.faults();
    let before_totals = before.totals();
    let after_totals = after.totals();
    json!({
        "leased": to.leased().saturating_sub(from.leased()),
        "busy": to.busy().saturating_sub(from.busy()),
        "faults": {
            "lock_contention": after_totals[0].saturating_sub(before_totals[0]),
            "consumed_overdue": after_totals[1].saturating_sub(before_totals[1]),
            "resource_limit": after_totals[2].saturating_sub(before_totals[2]),
            "totals": after_totals,
            "acknowledged_sequence": after.acknowledged_sequence(),
            "latest": after.latest().map(|fault| json!({
                "sequence": fault.sequence(),
                "kind": format!("{:?}", fault.kind()),
                "reservation_id": fault.reservation().reservation_id().get(),
                "active": fault.active(),
                "detected_at_t_s": since_epoch(fault.detected_at()),
            })),
        },
    })
}

#[cfg(feature = "render-test-hooks")]
#[derive(Clone, Copy)]
struct FairOracleHead<'a> {
    name: &'a str,
    status_present: bool,
    live: bool,
    refused: bool,
    attached: bool,
    stats_present: bool,
    identity_matches: bool,
    paints: u64,
    gpu: bool,
    scene_consumes: u64,
    has_valid_scene: bool,
    scene_progress: client::render::backend::SceneProgress,
}

#[cfg(feature = "render-test-hooks")]
fn m3_fair_oracle(
    expected_heads: usize,
    heads: &[FairOracleHead<'_>],
    fault_totals: [u64; 3],
) -> Result<(), String> {
    if heads.len() != expected_heads {
        return Err(format!(
            "expected {expected_heads} heads but observed {}",
            heads.len()
        ));
    }
    if fault_totals.iter().any(|&count| count > 0) {
        return Err(format!("render fault totals are nonzero: {fault_totals:?}"));
    }
    for head in heads {
        if !head.status_present {
            return Err(format!("{}: missing status row", head.name));
        }
        if !head.live {
            return Err(format!("{}: stopped or not in a live scene", head.name));
        }
        if head.refused {
            return Err(format!("{}: render head was refused", head.name));
        }
        if !head.attached {
            return Err(format!("{}: render head is not attached", head.name));
        }
        if !head.stats_present {
            return Err(format!("{}: fair stats are missing", head.name));
        }
        if !head.identity_matches {
            return Err(format!(
                "{}: fair stats do not match the live head",
                head.name
            ));
        }
        if head.paints == 0 {
            return Err(format!("{}: head stopped painting", head.name));
        }
        if head.gpu && head.paints > 0 && head.scene_consumes == 0 {
            return Err(format!(
                "{}: head painted but never consumed a scene",
                head.name
            ));
        }
        if !head.gpu
            && (!head.has_valid_scene
                || head.scene_progress == client::render::backend::SceneProgress::NoScene)
        {
            return Err(format!(
                "{}: CPU head has no live scene progress",
                head.name
            ));
        }
    }
    Ok(())
}

#[cfg(feature = "render-test-hooks")]
#[test]
fn m3_oracle_rejects_paints_without_scene() {
    let head = FairOracleHead {
        name: "paint-only",
        status_present: true,
        live: true,
        refused: false,
        attached: true,
        stats_present: true,
        identity_matches: true,
        paints: 1,
        gpu: true,
        scene_consumes: 0,
        has_valid_scene: false,
        scene_progress: client::render::backend::SceneProgress::NoScene,
    };
    let error = m3_fair_oracle(1, &[head], [0; 3]).unwrap_err();
    assert!(
        error.contains("painted but never consumed a scene"),
        "{error}"
    );
}

#[cfg(feature = "render-test-hooks")]
#[test]
fn m3_oracle_rejects_missing_refused_and_stopped_heads() {
    let head = FairOracleHead {
        name: "fixture",
        status_present: true,
        live: true,
        refused: false,
        attached: true,
        stats_present: true,
        identity_matches: true,
        paints: 1,
        gpu: true,
        scene_consumes: 1,
        has_valid_scene: true,
        scene_progress: client::render::backend::SceneProgress::Rebuilt,
    };
    assert!(m3_fair_oracle(1, &[], [0; 3]).is_err());
    assert!(m3_fair_oracle(
        1,
        &[FairOracleHead {
            refused: true,
            ..head
        }],
        [0; 3]
    )
    .unwrap_err()
    .contains("refused"));
    assert!(m3_fair_oracle(
        1,
        &[FairOracleHead {
            live: false,
            ..head
        }],
        [0; 3]
    )
    .unwrap_err()
    .contains("stopped"));
}

#[cfg(feature = "render-test-hooks")]
fn assert_fair_interval(
    cell: &str,
    heads: &[HeadProbe],
    statuses: &[host_play::SlotStatus],
    from: &Mark,
    to: &Mark,
) {
    let evidence: Vec<_> = heads
        .iter()
        .enumerate()
        .map(|(i, head)| {
            let status = statuses.iter().find(|status| status.username == head.name);
            let previous = from
                .fair
                .as_ref()
                .and_then(|stats| stats.get(i).copied().flatten());
            let current = to
                .fair
                .as_ref()
                .and_then(|stats| stats.get(i).copied().flatten());
            let previous_identity = from
                .head_identities
                .as_ref()
                .and_then(|identities| identities.get(i).copied().flatten());
            let current_identity = to
                .head_identities
                .as_ref()
                .and_then(|identities| identities.get(i).copied().flatten());
            let identity_matches = match (previous, current) {
                (Some(previous), Some(current)) => {
                    previous.identity() == current.identity()
                        && previous_identity == current_identity
                        && current_identity == Some(current.identity())
                }
                _ => false,
            };
            let scene_consumes = if identity_matches {
                current
                    .map(|stats| {
                        stats
                            .scene_consumes()
                            .saturating_sub(previous.unwrap().scene_consumes())
                    })
                    .unwrap_or_default()
            } else {
                0
            };
            let stats = current;
            FairOracleHead {
                name: &head.name,
                status_present: status.is_some(),
                live: status.is_some_and(|status| {
                    status.ingame && status.scene_state == 2 && status.worker_terminal.is_none()
                }),
                attached: current_identity.is_some(),
                stats_present: stats.is_some(),
                identity_matches,
                paints: to.paints[i].saturating_sub(from.paints[i]),
                gpu: stats.is_some_and(|stats| {
                    stats.backend() == client::render::backend::BackendKind::Gpu
                }),
                scene_consumes,
                has_valid_scene: stats.is_some_and(|stats| stats.has_valid_scene()),
                scene_progress: stats
                    .map_or(client::render::backend::SceneProgress::NoScene, |stats| {
                        stats.scene_progress()
                    }),
            }
        })
        .collect();
    let fault_totals = to.workspace.faults().totals();
    m3_fair_oracle(heads.len(), &evidence, fault_totals)
        .unwrap_or_else(|error| panic!("{cell}: {error}"));
}

fn summarize(
    heads: &[HeadProbe],
    focused: &[bool],
    full_rate: &[bool],
    from: &Mark,
    to: &Mark,
    probe: Option<FairProbeInputs>,
) -> Value {
    let secs = to.at.duration_since(from.at).as_secs_f64();
    let mut rows = Vec::new();
    let mut groups: HashMap<&str, Group> = HashMap::new();
    for (i, head) in heads.iter().enumerate() {
        let paints = to.paints[i] - from.paints[i];
        let (pub_from, pub_to) = (from.published[i], to.published[i]);
        let published_raster_ns = pub_to.raster_ns - pub_from.raster_ns;
        let published_paints = pub_to.paints - pub_from.paints;
        let frames = to.frames[i] - from.frames[i];
        let histogram = delta(&from.histograms[i], &to.histograms[i]);
        let raster_ms = (published_paints > 0)
            .then(|| published_raster_ns as f64 / published_paints as f64 / 1e6);
        let fair = {
            #[cfg(feature = "render-test-hooks")]
            {
                if probe.is_some() {
                    fair_head_metrics(
                        from.fair
                            .as_ref()
                            .and_then(|stats| stats.get(i).copied().flatten()),
                        to.fair
                            .as_ref()
                            .and_then(|stats| stats.get(i).copied().flatten()),
                        from.head_identities
                            .as_ref()
                            .and_then(|identities| identities.get(i).copied().flatten()),
                        to.head_identities
                            .as_ref()
                            .and_then(|identities| identities.get(i).copied().flatten()),
                        secs,
                    )
                } else {
                    Value::Null
                }
            }
            #[cfg(not(feature = "render-test-hooks"))]
            {
                let _ = probe;
                Value::Null
            }
        };
        let group = groups
            .entry(if focused[i] { "focused" } else { "background" })
            .or_default();
        group.paint_fps.push(paints as f64 / secs);
        if probe.is_some() {
            group.composite_fps.push(paints as f64 / secs);
            if let Some(scene_fps) = fair.get("scene_fps").and_then(Value::as_f64) {
                group.scene_fps.push(scene_fps);
            }
        }
        group.raster_ms.extend(raster_ms);
        group.published_raster_ns += published_raster_ns;
        group.published_paints += published_paints;
        if group.histogram.is_empty() {
            group.histogram = vec![0; BUCKETS];
        }
        for (sum, count) in group.histogram.iter_mut().zip(&histogram) {
            *sum += count;
        }
        let mut row = json!({
            "head": head.name,
            "focused": focused[i],
            "paints": paints,
            "paint_fps": paints as f64 / secs,
            "raster_ms_per_paint": raster_ms,
            "raster_publication": {
                "from": pub_from.json(),
                "to": pub_to.json(),
                "raster_ns": published_raster_ns,
                "paints": published_paints,
            },
            "slot_frames": frames,
            "frame_interval_ms_p50": quantile(&histogram, 0.50),
            "frame_interval_ms_p95": quantile(&histogram, 0.95),
            "frame_interval_ms_p99": quantile(&histogram, 0.99),
            "frame_interval_ms_max": histogram.iter().rposition(|&c| c > 0),
        });
        if probe.is_some() {
            row["actual_focused"] = json!(focused[i]);
            row["full_rate"] = json!(full_rate[i]);
            row["composite_fps"] = json!(paints as f64 / secs);
            row["scene_fps"] = fair.get("scene_fps").cloned().unwrap_or(Value::Null);
            row["fair"] = fair;
        }
        rows.push(row);
    }
    let mean = |v: &[f64]| (!v.is_empty()).then(|| v.iter().sum::<f64>() / v.len() as f64);
    let max = |v: &[f64]| v.iter().copied().reduce(f64::max);
    let groups: serde_json::Map<String, Value> = groups
        .into_iter()
        .map(|(name, group)| {
            let mut row = json!({
                "heads": group.paint_fps.len(),
                "paint_fps_mean": mean(&group.paint_fps),
                "paint_fps_min": group.paint_fps.iter().copied().reduce(f64::min),
                "raster_ms_per_paint_mean": mean(&group.raster_ms),
                "raster_ms_per_paint_max": max(&group.raster_ms),
                "published_raster_ns": group.published_raster_ns,
                "published_paints": group.published_paints,
                "raster_ms_per_paint_pooled": (group.published_paints > 0).then(|| {
                    group.published_raster_ns as f64 / group.published_paints as f64 / 1e6
                }),
                "frame_interval_ms_p50": quantile(&group.histogram, 0.50),
                "frame_interval_ms_p95": quantile(&group.histogram, 0.95),
                "frame_interval_ms_p99": quantile(&group.histogram, 0.99),
            });
            if probe.is_some() {
                row["composite_fps_mean"] = json!(mean(&group.composite_fps));
                row["scene_fps_mean"] = json!(mean(&group.scene_fps));
            }
            (name.to_string(), row)
        })
        .collect();
    let gpu = client::profiling::gpu_bytes();
    let (ws_from, ws_to) = (from.workspace, to.workspace);
    let waited = ws_to.waited() - ws_from.waited();
    let wait = ws_to.wait() - ws_from.wait();
    let legacy_workspace = json!({
        "leased": ws_to.leased() - ws_from.leased(),
        "waited": waited,
        "wait_ms_total": wait.as_secs_f64() * 1e3,
        "wait_us_mean": (waited > 0).then(|| wait.as_secs_f64() * 1e6 / waited as f64),
        "max_wait_ms_process": ws_to.max_wait().as_secs_f64() * 1e3,
        "unleased": ws_to.unleased() - ws_from.unleased(),
    });
    #[cfg(feature = "render-test-hooks")]
    let scene_workspace = if probe.is_some() {
        fair_workspace_metrics(ws_from, ws_to)
    } else {
        legacy_workspace
    };
    #[cfg(not(feature = "render-test-hooks"))]
    let scene_workspace = legacy_workspace;
    let mut summary = json!({
        "observe_s": secs,
        "from_t_s": since_epoch(from.at),
        "to_t_s": since_epoch(to.at),
        "process_cpu_percent": (to.cpu_s - from.cpu_s) / secs * 100.0,
        "rss_mib": current_resident_bytes().map(|b| b as f64 / 1048576.0),
        "gpu_tracked_mib": (gpu.buffers + gpu.textures) as f64 / 1048576.0,
        "gpu_peak_tracked_mib": gpu.peak as f64 / 1048576.0,
        "frames_taken_by_consumer": to.taken - from.taken,
        "scene_workspace": scene_workspace,
        "groups": groups,
        "heads": rows,
    });
    if probe.is_some() {
        summary["independent_pump"] = pump_metrics(from.pump, to.pump).unwrap_or(Value::Null);
        add_probe_receipt(&mut summary, probe);
    }
    summary
}

fn wait_ingame(play: &Play, want: usize, timeout: Duration) {
    let deadline = Instant::now() + timeout;
    loop {
        let ready = play
            .statuses()
            .iter()
            .filter(|s| s.ingame && s.scene_state == 2)
            .count();
        if ready >= want {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "{ready}/{want} heads in game with a scene after {timeout:?}"
        );
        thread::sleep(Duration::from_millis(250));
    }
}

#[test]
#[ignore = "LIVE=1 Engine A measurement; logs in its own BOT_LIVE_NAME_PREFIX accounts"]
fn live_m3_render_contention() {
    assert_eq!(std::env::var("LIVE").as_deref(), Ok("1"));
    let n: usize = required("M3_N").parse().expect("M3_N");
    let backend = required("M3_BACKEND");
    let cpu = match backend.as_str() {
        "cpu" => true,
        "gpu" => false,
        other => panic!("M3_BACKEND must be cpu or gpu, got {other}"),
    };
    assert_eq!(
        std::env::var("BOT_CPU").as_deref() == Ok("1"),
        cpu,
        "BOT_CPU=1 exactly for the cpu backend"
    );
    let probe = fair_probe_inputs();
    configure_fair_probe(probe);

    let out = PathBuf::from(required("M3_OUT"));
    std::fs::create_dir_all(&out).unwrap();
    let warmup = Duration::from_secs(env_or("M3_WARMUP_S", 10));
    let observe = Duration::from_secs(env_or("M3_OBSERVE_S", 30));
    let policies: Vec<String> = std::env::var("M3_POLICIES")
        .unwrap_or_else(|_| "full,focus1".into())
        .split(',')
        .map(str::to_string)
        .collect();
    client::profiling::enable();
    LazyLock::force(&EPOCH);

    let options = ProfileOptions {
        profile: Some("local-289".into()),
        revision: Some("289".into()),
        host: Some("127.0.0.1".into()),
        asset_host: Some("127.0.0.1".into()),
        port: Some(env_or("M3_PORT", 44594)),
        http_port: Some(env_or("M3_HTTP_PORT", 1080)),
        engine_dir: Some(required("ENGINE_DIR").into()),
        nav_pack: Some(required("NAV_PACK").into()),
        ..ProfileOptions::default()
    };
    let bound = options
        .resolve(None)
        .expect("resolve")
        .bind()
        .expect("bind");
    let names = mint_live_names(n);
    eprintln!("m3 accounts: {names:?}");
    let profiles: Vec<Profile> = names
        .iter()
        .enumerate()
        .map(|(i, name)| Profile {
            username: name.clone(),
            password: name.clone().into(),
            uid: 274_300_000 + i as i32,
            settings: ProfileSettings {
                auto_login: true,
                lowmem: true,
                raster: if cpu {
                    RasterMode::Cpu
                } else {
                    RasterMode::Gpu
                },
                ..ProfileSettings::default()
            },
        })
        .collect();
    let heads: Vec<HeadProbe> = names
        .iter()
        .map(|name| {
            let input = SlotInput::new();
            input.set_prefer_cpu(cpu);
            let mailbox = FrameBuf::new();
            HeadProbe {
                name: name.clone(),
                input,
                raster: Arc::new(RasterPublications::new(Arc::clone(&mailbox))),
                mailbox,
                frames: Arc::new(SlotFrames::new()),
            }
        })
        .collect();
    let channels: HashMap<String, (Arc<SlotInput>, Arc<FrameBuf>)> = heads
        .iter()
        .map(|h| {
            (
                h.name.clone(),
                (Arc::clone(&h.input), Arc::clone(&h.mailbox)),
            )
        })
        .collect();
    type HookProbes = HashMap<String, (Arc<SlotFrames>, Arc<RasterPublications>)>;
    let hook_probes: Arc<HookProbes> = Arc::new(
        heads
            .iter()
            .map(|h| {
                (
                    h.name.clone(),
                    (Arc::clone(&h.frames), Arc::clone(&h.raster)),
                )
            })
            .collect(),
    );
    // Heads stay detached (headless) until the census below has run.
    let draw = Arc::new(AtomicBool::new(false));
    let census = Arc::new(Census::default());
    let template = SharedClientTemplate::load(bound).expect("template");
    let play = {
        let (draw, census) = (Arc::clone(&draw), Arc::clone(&census));
        run_with_template(
            template,
            false,
            profiles,
            move |name| {
                let (input, mailbox) = &channels[name];
                (Some(Arc::clone(input)), Some(Arc::clone(mailbox)))
            },
            move |client, name, _| {
                client.set_draw(draw.load(Relaxed));
                if let Some((stats, raster)) = hook_probes.get(name) {
                    stats.record(Instant::now());
                    raster.observe(name);
                }
                census.observe(client, name);
            },
        )
        .expect("play")
    };

    // The panel's frame consumer takes each mailbox on its UI period. In the
    // explicit probe mode, its independent render pump wakes at the earlier
    // of the service's returned deadline and the next consumer-period tick.
    let stop = Arc::new(AtomicBool::new(false));
    let taken = Arc::new(AtomicU64::new(0));
    let pump_probe = probe.map(|_| Arc::new(PumpProbe::new()));
    let consumer = {
        let mailboxes: Vec<Arc<FrameBuf>> = heads.iter().map(|h| Arc::clone(&h.mailbox)).collect();
        let (stop, taken) = (Arc::clone(&stop), Arc::clone(&taken));
        let pump_probe = pump_probe.as_ref().map(Arc::clone);
        thread::spawn(move || {
            if let Some(pump_probe) = pump_probe {
                let mut next_consumer = Instant::now();
                let mut next_pump = next_consumer;
                while !stop.load(Relaxed) {
                    let mut pump_due = false;
                    if Instant::now() >= next_consumer {
                        for mailbox in &mailboxes {
                            if mailbox.take().is_some() {
                                taken.fetch_add(1, Relaxed);
                            }
                        }
                        next_consumer = Instant::now() + CONSUMER_PERIOD;
                        pump_due = true;
                    }
                    if Instant::now() >= next_pump {
                        pump_due = true;
                    }
                    if pump_due {
                        let pump_at = Instant::now();
                        pump_probe.record(pump_at);
                        next_pump = render_head::pump(pump_at)
                            .map(|deadline| deadline.instant())
                            .unwrap_or(pump_at + CONSUMER_PERIOD);
                    }
                    if stop.load(Relaxed) {
                        break;
                    }
                    let wake_at = next_consumer.min(next_pump);
                    let now = Instant::now();
                    if let Some(timeout) = wake_at.checked_duration_since(now) {
                        thread::park_timeout(timeout);
                    } else {
                        thread::yield_now();
                    }
                }
            } else {
                while !stop.load(Relaxed) {
                    for mailbox in &mailboxes {
                        if mailbox.take().is_some() {
                            taken.fetch_add(1, Relaxed);
                        }
                    }
                    render_head::pump(Instant::now());
                    thread::sleep(CONSUMER_PERIOD);
                }
            }
        })
    };

    let login_started = Instant::now();
    wait_ingame(&play, n, Duration::from_secs(60 + 30 * n as u64));
    eprintln!(
        "m3: {n} bots in game after {:.1}s",
        login_started.elapsed().as_secs_f64()
    );
    let rev = std::env::var("M3_GIT_REV").unwrap_or_default();

    // M1 evidence: the sim world each bot retains with no head, then with a
    // drawing head, on the real scenes these accounts loaded.
    let mut phases = serde_json::Map::new();
    for phase in ["headless", "headed"] {
        draw.store(phase == "headed", Relaxed);
        thread::sleep(warmup);
        let rows = census.run(phase, n);
        let gpu = client::profiling::gpu_bytes();
        phases.insert(
            phase.into(),
            json!({
                "rss_mib": current_resident_bytes().map(|b| b as f64 / 1048576.0),
                "gpu_tracked_mib": (gpu.buffers + gpu.textures) as f64 / 1048576.0,
                "worlds": rows,
            }),
        );
    }
    let mut census_receipt = json!({
        "n": n,
        "backend": backend,
        "git_rev": rev,
        "static_bytes_per_world": static_world_bytes(),
        "phases": phases,
    });
    add_probe_receipt(&mut census_receipt, probe);
    std::fs::write(
        out.join(format!("{backend}-n{n}-census.json")),
        serde_json::to_string_pretty(&census_receipt).unwrap(),
    )
    .unwrap();

    for policy in &policies {
        let full_rate: Vec<bool> = match policy.as_str() {
            "full" => vec![true; n],
            "focus1" => (0..n).map(|i| i == 0).collect(),
            other => panic!("unknown M3 policy {other}"),
        };
        let focused: Vec<bool> = if probe.is_some() {
            (0..n).map(|i| i == 0).collect()
        } else {
            full_rate.clone()
        };
        for ((head, &full_rate), &focused) in heads.iter().zip(&full_rate).zip(&focused) {
            #[cfg(feature = "render-test-hooks")]
            if probe.is_some() {
                head.input.set_render_focused(focused);
            }
            head.input.set_full_rate(full_rate);
        }
        thread::sleep(warmup);
        let cell = format!("{backend}-n{n}-{policy}");
        let mut samples = std::fs::File::create(out.join(format!("{cell}.samples.jsonl"))).unwrap();
        let start = mark(&heads, &taken, pump_probe.as_deref());
        let mut last = start.clone();
        while last.at.duration_since(start.at) < observe {
            thread::sleep(Duration::from_secs(1));
            let now = mark(&heads, &taken, pump_probe.as_deref());
            let statuses = play.statuses();
            let ingame = statuses.iter().filter(|status| status.ingame).count();
            if probe.is_some() {
                #[cfg(feature = "render-test-hooks")]
                assert_fair_interval(&cell, &heads, &statuses, &last, &now);
                #[cfg(not(feature = "render-test-hooks"))]
                unreachable!("the probe setup rejects missing render-test-hooks");
            }
            let mut sample = summarize(&heads, &focused, &full_rate, &last, &now, probe);
            sample["t_s"] = json!(now.at.duration_since(start.at).as_secs_f64());
            sample["ingame"] = json!(ingame);
            writeln!(samples, "{sample}").unwrap();
            last = now;
        }
        let statuses = play.statuses();
        let ingame = statuses.iter().filter(|status| status.ingame).count();
        if probe.is_some() {
            #[cfg(feature = "render-test-hooks")]
            assert_fair_interval(&cell, &heads, &statuses, &start, &last);
            #[cfg(not(feature = "render-test-hooks"))]
            unreachable!("the probe setup rejects missing render-test-hooks");
        }
        let mut summary = summarize(&heads, &focused, &full_rate, &start, &last, probe);
        summary["cell"] = json!(cell);
        summary["n"] = json!(n);
        summary["backend"] = json!(backend);
        summary["policy"] = json!(policy);
        summary["ingame_at_end"] = json!(ingame);
        summary["git_rev"] = json!(rev);
        summary["warmup_s"] = json!(warmup.as_secs());
        std::fs::write(
            out.join(format!("{cell}.json")),
            serde_json::to_string_pretty(&summary).unwrap(),
        )
        .unwrap();
        eprintln!("m3 {cell}: {}", summary["groups"]);
        assert_eq!(ingame, n, "{cell}: every head stayed in game");
    }

    // T4.2: live head detach and reattach on the logged-in sessions.
    let reattach = live_reattach(&play, &heads, &draw, warmup, &census, probe);
    std::fs::write(
        out.join(format!("{backend}-n{n}-reattach.json")),
        serde_json::to_string_pretty(&reattach).unwrap(),
    )
    .unwrap();
    eprintln!("m3 {backend}-n{n}-reattach: {reattach}");

    for head in &heads {
        if let Some(arm) = play.arm(&head.name) {
            arm.request_logout();
        }
    }
    let deadline = Instant::now() + Duration::from_secs(60);
    while play.statuses().iter().any(|s| s.ingame) && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(250));
    }
    stop.store(true, Relaxed);
    consumer.join().unwrap();
}

/// Live head detach and reattach (T4.2, ADR 0001). Draw off retires every
/// head: its mailbox stops accepting frames and nothing paints. Draw on
/// attaches a new generation of the same slot over the same connection and
/// login: the stream epoch is unchanged and its byte counters keep growing.
fn live_reattach(
    play: &Play,
    heads: &[HeadProbe],
    draw: &AtomicBool,
    settle: Duration,
    census: &Census,
    probe: Option<FairProbeInputs>,
) -> Value {
    let session = |name: &str| {
        play.statuses()
            .into_iter()
            .find(|status| status.username == name)
            .expect("a status row per head")
    };
    let before: Vec<_> = heads
        .iter()
        .map(|head| {
            let status = session(&head.name);
            let attached = head.mailbox.head().expect("a drawing slot has a head");
            (attached, status)
        })
        .collect();

    draw.store(false, Relaxed);
    thread::sleep(settle);
    let detached: Vec<bool> = heads.iter().map(|h| h.mailbox.head().is_none()).collect();
    let paints_off: Vec<u64> = heads.iter().map(|h| h.mailbox.generation()).collect();
    thread::sleep(Duration::from_secs(2));
    let painted_while_off: Vec<u64> = heads
        .iter()
        .zip(&paints_off)
        .map(|(h, &paints)| h.mailbox.generation() - paints)
        .collect();
    let worlds_detached = census.run("detached", heads.len());
    assert!(
        worlds_detached
            .iter()
            .all(|world| world["dynamic_sprite_storage_bytes"] == 0),
        "detached worlds release transient draw-sprite storage"
    );

    draw.store(true, Relaxed);
    let reattach_from = Instant::now();
    let deadline = reattach_from + Duration::from_secs(30);
    while heads
        .iter()
        .zip(&paints_off)
        .any(|(h, &paints)| h.mailbox.generation() <= paints)
    {
        assert!(
            Instant::now() < deadline,
            "every head paints again after reattach"
        );
        thread::sleep(Duration::from_millis(50));
    }
    let repainted_after_s = reattach_from.elapsed().as_secs_f64();
    thread::sleep(Duration::from_secs(1));
    let worlds_reattached = census.run("reattached", heads.len());

    let mut rows = Vec::new();
    for ((head, (first, status)), &detached) in heads.iter().zip(&before).zip(&detached) {
        let now = session(&head.name);
        let second = head.mailbox.head().expect("draw on reattaches a head");
        let row = json!({
            "head": head.name,
            "slot": first.slot().incarnation().get(),
            "generation_before": first.generation().get(),
            "generation_after": second.generation().get(),
            "detached": detached,
            "same_slot": second.slot() == first.slot(),
            "new_generation": second.generation() != first.generation(),
            "stream_epoch_before": status.stream_epoch,
            "stream_epoch_after": now.stream_epoch,
            "same_login": now.login_started == status.login_started,
            "bytes_in_before": status.bytes_in,
            "bytes_in_after": now.bytes_in,
            "ingame": now.ingame,
            "scene_state": now.scene_state,
        });
        assert!(detached, "{}: draw off retires the head", head.name);
        assert_eq!(second.slot(), first.slot(), "{}: same slot", head.name);
        assert_ne!(
            second.generation(),
            first.generation(),
            "{}: reattach is a new head generation",
            head.name
        );
        assert_eq!(
            now.stream_epoch, status.stream_epoch,
            "{}: attach never reconnects (ADR 0001)",
            head.name
        );
        assert_eq!(
            now.login_started, status.login_started,
            "{}: no new login",
            head.name
        );
        assert!(
            now.bytes_in >= status.bytes_in,
            "{}: the same stream",
            head.name
        );
        assert!(
            now.ingame && now.scene_state == 2,
            "{}: still in game",
            head.name
        );
        rows.push(row);
    }
    assert!(
        painted_while_off.iter().all(|&paints| paints == 0),
        "no head paints while detached: {painted_while_off:?}"
    );
    let mut receipt = json!({
        "painted_while_detached": painted_while_off,
        "repainted_after_s": repainted_after_s,
        "worlds_detached": worlds_detached,
        "worlds_reattached": worlds_reattached,
        "heads": rows,
    });
    add_probe_receipt(&mut receipt, probe);
    receipt
}
