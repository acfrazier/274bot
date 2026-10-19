//! M3: render contention at N heads (contract §23 §2.5 / §21 measurements),
//! supplemental Linux evidence for the T4 track. Mac FLOOR stays the
//! qualification gate; nothing here is a FLOOR number.
//!
//! N bots log in to Engine A through the production `Play` path, each slot on
//! its own thread with a drawing head (the per-frame hook applies
//! `set_draw(true)`, as the panel's focus switch does). A consumer thread
//! takes every mailbox at the panel's ~60 Hz, as the panel does. Two render
//! policies run in one session:
//!
//! - `full`: every head focused (full rate, a paint on every 20 ms frame);
//! - `focus1`: head 0 focused, every other head background (1 fps watch).
//!
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
}

fn mark(heads: &[HeadProbe], taken: &AtomicU64) -> Mark {
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
    raster_ms: Vec<f64>,
    published_raster_ns: u64,
    published_paints: u64,
    histogram: Vec<u64>,
}

fn summarize(heads: &[HeadProbe], focused: &[bool], from: &Mark, to: &Mark) -> Value {
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
        let group = groups
            .entry(if focused[i] { "focused" } else { "background" })
            .or_default();
        group.paint_fps.push(paints as f64 / secs);
        group.raster_ms.extend(raster_ms);
        group.published_raster_ns += published_raster_ns;
        group.published_paints += published_paints;
        if group.histogram.is_empty() {
            group.histogram = vec![0; BUCKETS];
        }
        for (sum, count) in group.histogram.iter_mut().zip(&histogram) {
            *sum += count;
        }
        rows.push(json!({
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
        }));
    }
    let mean = |v: &[f64]| (!v.is_empty()).then(|| v.iter().sum::<f64>() / v.len() as f64);
    let max = |v: &[f64]| v.iter().copied().reduce(f64::max);
    let groups: serde_json::Map<String, Value> = groups
        .into_iter()
        .map(|(name, group)| {
            (
                name.to_string(),
                json!({
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
                }),
            )
        })
        .collect();
    let gpu = client::profiling::gpu_bytes();
    // GPU scene submissions leased from the one scene workspace in this
    // interval, and how long those that found it held waited for their
    // grant (the wrap's retained wait). The max is the process lifetime's.
    let (ws_from, ws_to) = (from.workspace, to.workspace);
    let waited = ws_to.waited() - ws_from.waited();
    let wait = ws_to.wait() - ws_from.wait();
    json!({
        "observe_s": secs,
        "from_t_s": since_epoch(from.at),
        "to_t_s": since_epoch(to.at),
        "process_cpu_percent": (to.cpu_s - from.cpu_s) / secs * 100.0,
        "rss_mib": current_resident_bytes().map(|b| b as f64 / 1048576.0),
        "gpu_tracked_mib": (gpu.buffers + gpu.textures) as f64 / 1048576.0,
        "gpu_peak_tracked_mib": gpu.peak as f64 / 1048576.0,
        "frames_taken_by_consumer": to.taken - from.taken,
        "scene_workspace": {
            "leased": ws_to.leased() - ws_from.leased(),
            "waited": waited,
            "wait_ms_total": wait.as_secs_f64() * 1e3,
            "wait_us_mean": (waited > 0).then(|| wait.as_secs_f64() * 1e6 / waited as f64),
            "max_wait_ms_process": ws_to.max_wait().as_secs_f64() * 1e3,
            "unleased": ws_to.unleased() - ws_from.unleased(),
        },
        "groups": groups,
        "heads": rows,
    })
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

    // The panel's frame consumer: take every mailbox at ~60 Hz, and run the
    // independent render pump on the same loop, as the panel window does.
    let stop = Arc::new(AtomicBool::new(false));
    let taken = Arc::new(AtomicU64::new(0));
    let consumer = {
        let mailboxes: Vec<Arc<FrameBuf>> = heads.iter().map(|h| Arc::clone(&h.mailbox)).collect();
        let (stop, taken) = (Arc::clone(&stop), Arc::clone(&taken));
        thread::spawn(move || {
            while !stop.load(Relaxed) {
                for mailbox in &mailboxes {
                    if mailbox.take().is_some() {
                        taken.fetch_add(1, Relaxed);
                    }
                }
                render_head::pump(Instant::now());
                thread::sleep(CONSUMER_PERIOD);
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
    std::fs::write(
        out.join(format!("{backend}-n{n}-census.json")),
        serde_json::to_string_pretty(&json!({
            "n": n,
            "backend": backend,
            "git_rev": rev,
            "static_bytes_per_world": static_world_bytes(),
            "phases": phases,
        }))
        .unwrap(),
    )
    .unwrap();

    for policy in &policies {
        let focused: Vec<bool> = match policy.as_str() {
            "full" => vec![true; n],
            "focus1" => (0..n).map(|i| i == 0).collect(),
            other => panic!("unknown M3 policy {other}"),
        };
        for (head, &focus) in heads.iter().zip(&focused) {
            head.input.set_full_rate(focus);
        }
        thread::sleep(warmup);
        let cell = format!("{backend}-n{n}-{policy}");
        let mut samples = std::fs::File::create(out.join(format!("{cell}.samples.jsonl"))).unwrap();
        let start = mark(&heads, &taken);
        let mut last = start.clone();
        while last.at.duration_since(start.at) < observe {
            thread::sleep(Duration::from_secs(1));
            let now = mark(&heads, &taken);
            let ingame = play.statuses().iter().filter(|s| s.ingame).count();
            let mut sample = summarize(&heads, &focused, &last, &now);
            sample["t_s"] = json!(now.at.duration_since(start.at).as_secs_f64());
            sample["ingame"] = json!(ingame);
            writeln!(samples, "{sample}").unwrap();
            last = now;
        }
        let ingame = play.statuses().iter().filter(|s| s.ingame).count();
        let mut summary = summarize(&heads, &focused, &start, &last);
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
    let reattach = live_reattach(&play, &heads, &draw, warmup, &census);
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
    json!({
        "painted_while_detached": painted_while_off,
        "repainted_after_s": repainted_after_s,
        "worlds_detached": worlds_detached,
        "worlds_reattached": worlds_reattached,
        "heads": rows,
    })
}
