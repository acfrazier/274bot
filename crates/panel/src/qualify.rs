//! `panel-play --qualify`: a release-build, machine-readable placement
//! contract for lab gates. The run is the `--smoke` login (the `render_smoke`
//! scenario), but it reports explicit `[qualify] key=value` markers on stdout
//! (adapter, game backend, first frame presented, scene ready, hold
//! complete), holds the window a fixed time and exits with a JSON receipt.
//! Nothing here runs unless `--qualify` armed the process-wide [`Recorder`]:
//! every hook is one relaxed atomic load otherwise.
//!
//! String values on marker lines (`name`, `reason`, `path`) are JSON-quoted,
//! so a line stays one line and a parser can split on spaces outside quotes.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use serde::Serialize;

/// Receipt JSON schema version.
pub const RECEIPT_SCHEMA: u32 = 1;
/// `--qualify-hold` default, in seconds.
pub const DEFAULT_HOLD_S: u64 = 5;
/// `--qualify-hold` upper bound, in seconds.
pub const MAX_HOLD_S: u64 = 60;
/// File name of the default receipt inside the run's shot directory.
pub const RECEIPT_FILE: &str = "qualify-receipt.json";
/// Label of the whole-window shot the `render_smoke` scenario takes.
pub const SCENE2_LABEL: &str = "scene2";

/// What `--qualify` was asked to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QualifyConfig {
    /// Seconds the window stays up after the scene is ready.
    pub hold_s: u64,
    /// Explicit receipt path; `None` writes `<shot dir>/qualify-receipt.json`.
    pub receipt: Option<PathBuf>,
}

impl Default for QualifyConfig {
    fn default() -> Self {
        Self {
            hold_s: DEFAULT_HOLD_S,
            receipt: None,
        }
    }
}

/// The clocks the run is judged by (the `--smoke` constants, passed in so
/// this module stays independent of the live harness).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Timing {
    /// How long the focused slot must hold scene 2 before the world counts
    /// as rendered (and the scene2 shot is released).
    pub settle: Duration,
    /// Whole-run ceiling.
    pub deadline: Duration,
    /// Longest the hold waits for the scene2 shot after the hold elapsed.
    pub shot_drain: Duration,
}

/// Which path produced the game frame the panel presented.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GameBackend {
    /// `FrameOutput::Texture`: the client's GPU frame bound directly.
    Gpu,
    /// `FrameOutput::PixMap`: CPU pixels uploaded.
    Cpu,
}

impl GameBackend {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Gpu => "gpu",
            Self::Cpu => "cpu",
        }
    }

    pub fn of(frame: &client::render::backend::FrameOutput) -> Self {
        match frame {
            client::render::backend::FrameOutput::Texture(_) => Self::Gpu,
            client::render::backend::FrameOutput::PixMap(_) => Self::Cpu,
        }
    }
}

/// The wgpu adapter the panel window runs on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AdapterReport {
    pub name: String,
    pub backend: String,
    pub device_type: String,
    pub vendor: u32,
    pub device: u32,
    pub driver: String,
    pub driver_info: String,
}

impl AdapterReport {
    pub fn from_info(info: &wgpu::AdapterInfo) -> Self {
        Self {
            name: info.name.clone(),
            backend: format!("{:?}", info.backend),
            device_type: format!("{:?}", info.device_type),
            vendor: info.vendor,
            device: info.device,
            driver: info.driver.clone(),
            driver_info: info.driver_info.clone(),
        }
    }
}

/// The markers, in the receipt's `markers` list by observation order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Marker {
    Adapter,
    FirstFramePresented,
    Backend,
    SceneReady,
    HoldComplete,
}

impl Marker {
    pub fn name(self) -> &'static str {
        match self {
            Self::Adapter => "adapter",
            Self::FirstFramePresented => "first_frame_presented",
            Self::Backend => "backend",
            Self::SceneReady => "scene_ready",
            Self::HoldComplete => "hold_complete",
        }
    }
}

/// The JSON receipt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct QualifyReceipt {
    pub schema: u32,
    pub ok: bool,
    pub failure: Option<String>,
    pub adapter: Option<AdapterReport>,
    pub game_backend: Option<&'static str>,
    pub first_frame_presented_ms: Option<u64>,
    pub scene_ready_ms: Option<u64>,
    pub hold_s: u64,
    pub held_ms: Option<u64>,
    pub elapsed_ms: u64,
    pub scene2_png: Option<String>,
    pub markers: Vec<&'static str>,
}

/// What one [`Recorder::tick`] decided.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Pending,
    /// The hold elapsed and the receipt was written.
    Passed,
    /// The run failed (the receipt with `ok: false` is written).
    Failed(String),
}

fn ms(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

fn quoted(value: &str) -> String {
    serde_json::to_string(value).expect("a string always serializes")
}

/// The pure state machine of one qualify run. Every transition takes `now`
/// so tests need no sleeping; marker lines accumulate in `lines` for the
/// [`Recorder`] to print.
#[derive(Debug)]
pub struct QualifyRun {
    config: QualifyConfig,
    timing: Timing,
    started: Instant,
    adapter: Option<AdapterReport>,
    game_backend: Option<GameBackend>,
    game_frames: u64,
    first_frame_ms: Option<u64>,
    /// When the focused slot's current unbroken stretch at `ingame && scene >= 2`
    /// began; any tick outside the world before scene-ready clears it.
    scene2_seen_at: Option<Instant>,
    /// Scene 2 was reached at least once (only to word the stall reason).
    scene2_reached: bool,
    frames_at_scene2: u64,
    scene_ready_at: Option<Instant>,
    scene_ready_ms: Option<u64>,
    held_ms: Option<u64>,
    scene2_png: Option<PathBuf>,
    shot_dir: Option<PathBuf>,
    markers: Vec<Marker>,
    lines: Vec<String>,
    sealed: bool,
}

impl QualifyRun {
    pub fn new(config: QualifyConfig, timing: Timing, now: Instant) -> Self {
        Self {
            config,
            timing,
            started: now,
            adapter: None,
            game_backend: None,
            game_frames: 0,
            first_frame_ms: None,
            scene2_seen_at: None,
            scene2_reached: false,
            frames_at_scene2: 0,
            scene_ready_at: None,
            scene_ready_ms: None,
            held_ms: None,
            scene2_png: None,
            shot_dir: None,
            markers: Vec::new(),
            lines: Vec::new(),
            sealed: false,
        }
    }

    /// Marker lines produced since the last call.
    pub fn take_lines(&mut self) -> Vec<String> {
        std::mem::take(&mut self.lines)
    }

    fn since_start(&self, now: Instant) -> u64 {
        ms(now.saturating_duration_since(self.started))
    }

    fn mark(&mut self, marker: Marker, line: String) {
        self.markers.push(marker);
        self.lines.push(format!("[qualify] {line}"));
    }

    pub fn note_adapter(&mut self, report: AdapterReport) {
        if self.adapter.is_some() {
            return;
        }
        let line = format!(
            "adapter name={} backend={} device_type={} vendor={:#x} device={:#x}",
            quoted(&report.name),
            report.backend,
            report.device_type,
            report.vendor,
            report.device
        );
        self.adapter = Some(report);
        self.mark(Marker::Adapter, line);
    }

    /// A game frame reached the game image. The first one fixes the
    /// `backend` marker; every one counts toward the scene-ready gate.
    pub fn note_game_frame(&mut self, backend: GameBackend) {
        self.game_frames += 1;
        if self.game_backend.is_some() {
            return;
        }
        self.game_backend = Some(backend);
        let surface = self
            .adapter
            .as_ref()
            .map_or("unknown", |adapter| adapter.backend.as_str());
        let line = format!("backend game={} surface={surface}", backend.as_str());
        self.mark(Marker::Backend, line);
    }

    /// The swapchain presented a frame (`Occluded` never reaches here).
    pub fn note_frame_presented(&mut self, now: Instant) {
        if self.first_frame_ms.is_some() {
            return;
        }
        let elapsed = self.since_start(now);
        self.first_frame_ms = Some(elapsed);
        self.mark(
            Marker::FirstFramePresented,
            format!("first_frame_presented elapsed_ms={elapsed}"),
        );
    }

    pub fn note_shot_dir(&mut self, dir: PathBuf) {
        self.shot_dir = Some(dir);
    }

    pub fn note_scene2_png(&mut self, path: PathBuf) {
        self.scene2_png = Some(path);
    }

    /// The focused slot has held scene 2 for the settle window and is
    /// still in the world: the render has had time to rasterize it.
    pub fn scene_settled(&self, ingame: bool, now: Instant) -> bool {
        ingame
            && self
                .scene2_seen_at
                .is_some_and(|seen| now.saturating_duration_since(seen) >= self.timing.settle)
    }

    /// Advance the scene-ready / hold / deadline machine with the focused
    /// slot's status.
    pub fn tick(&mut self, ingame: bool, scene_state: i32, now: Instant) -> Outcome {
        if self.sealed {
            return Outcome::Pending;
        }
        // The whole-run deadline outranks every success below.
        if now.saturating_duration_since(self.started) >= self.timing.deadline {
            return Outcome::Failed(self.stall_reason());
        }
        let in_world = ingame && scene_state >= 2;
        if self.scene_ready_at.is_none() {
            if !in_world {
                self.scene2_seen_at = None;
            } else if self.scene2_seen_at.is_none() {
                self.scene2_seen_at = Some(now);
                self.scene2_reached = true;
                self.frames_at_scene2 = self.game_frames;
            }
        }
        if self.scene_ready_at.is_none()
            && self.first_frame_ms.is_some()
            && self.game_frames > self.frames_at_scene2
            && in_world
            && self.scene_settled(ingame, now)
        {
            let elapsed = self.since_start(now);
            self.scene_ready_at = Some(now);
            self.scene_ready_ms = Some(elapsed);
            let game_backend = self.game_backend.map_or("unknown", GameBackend::as_str);
            self.mark(
                Marker::SceneReady,
                format!(
                    "scene_ready elapsed_ms={elapsed} scene={scene_state} \
                     game_backend={game_backend} hold_s={}",
                    self.config.hold_s
                ),
            );
        }
        if let Some(ready) = self.scene_ready_at {
            let held = now.saturating_duration_since(ready);
            let shot_done = self.scene2_png.is_some() || held >= self.timing.shot_drain;
            if held >= Duration::from_secs(self.config.hold_s) && shot_done {
                self.held_ms = Some(ms(held));
                let elapsed = self.since_start(now);
                self.mark(
                    Marker::HoldComplete,
                    format!("hold_complete elapsed_ms={elapsed} held_ms={}", ms(held)),
                );
                return Outcome::Passed;
            }
        }
        Outcome::Pending
    }

    fn stall_reason(&self) -> String {
        let secs = self.timing.deadline.as_secs();
        let stage = if self.first_frame_ms.is_none() {
            "no frame was presented to the swapchain"
        } else if !self.scene2_reached {
            "the focused slot never reached scene 2"
        } else if self.scene2_seen_at.is_none() {
            "the focused slot did not stay in scene 2 until the render settled"
        } else if self.scene_ready_at.is_none() {
            "no game frame was presented after scene 2"
        } else {
            "the hold did not complete"
        };
        format!("{stage} within {secs}s")
    }

    /// Where the receipt goes: the explicit path, else inside the run's
    /// shot directory. `None` when neither exists (the caller falls back).
    pub fn receipt_path(&self) -> Option<PathBuf> {
        self.config
            .receipt
            .clone()
            .or_else(|| self.shot_dir.as_ref().map(|dir| dir.join(RECEIPT_FILE)))
    }

    /// Close the run: the receipt for `failure` (`None` is a pass) and, for
    /// a failure, the `failed` marker line. Idempotent: a sealed run has
    /// nothing more to report.
    pub fn seal(&mut self, failure: Option<String>, now: Instant) -> Option<QualifyReceipt> {
        if self.sealed {
            return None;
        }
        self.sealed = true;
        if let Some(reason) = failure.as_deref() {
            self.lines
                .push(format!("[qualify] failed reason={}", quoted(reason)));
        }
        Some(QualifyReceipt {
            schema: RECEIPT_SCHEMA,
            ok: failure.is_none(),
            failure,
            adapter: self.adapter.clone(),
            game_backend: self.game_backend.map(GameBackend::as_str),
            first_frame_presented_ms: self.first_frame_ms,
            scene_ready_ms: self.scene_ready_ms,
            hold_s: self.config.hold_s,
            held_ms: self.held_ms,
            elapsed_ms: self.since_start(now),
            scene2_png: self
                .scene2_png
                .as_ref()
                .map(|path| path.display().to_string()),
            markers: self.markers.iter().map(|m| m.name()).collect(),
        })
    }
}

/// Write `receipt` to `path` atomically: a synced temp file in the same
/// directory, then a rename over the destination.
pub fn write_receipt_atomic(path: &Path, receipt: &QualifyReceipt) -> std::io::Result<()> {
    let mut json = serde_json::to_string_pretty(receipt)
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
    json.push('\n');
    let file_name = path
        .file_name()
        .ok_or_else(|| std::io::Error::other("receipt path has no file name"))?;
    let mut temp_name = std::ffi::OsString::from(".");
    temp_name.push(file_name);
    temp_name.push(".tmp");
    let temp = path.with_file_name(temp_name);
    let written = (|| {
        let mut file = std::fs::File::create(&temp)?;
        file.write_all(json.as_bytes())?;
        file.sync_all()?;
        std::fs::rename(&temp, path)
    })();
    if written.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    written
}

fn receipt_line(path: &Path, ok: bool) -> String {
    format!(
        "[qualify] receipt path={} ok={ok}",
        quoted(&path.display().to_string())
    )
}

/// The process-wide holder of the one [`QualifyRun`]. A plain run (no
/// `--qualify`) never arms it, so every hook is one relaxed load.
pub struct Recorder {
    active: AtomicBool,
    presented: AtomicBool,
    run: Mutex<Option<QualifyRun>>,
}

impl Recorder {
    pub const fn new() -> Self {
        Self {
            active: AtomicBool::new(false),
            presented: AtomicBool::new(false),
            run: Mutex::new(None),
        }
    }

    pub fn is_active(&self) -> bool {
        self.active.load(Ordering::Relaxed)
    }

    fn with_run<R>(
        &self,
        out: &mut dyn Write,
        apply: impl FnOnce(&mut QualifyRun) -> R,
    ) -> Option<R> {
        if !self.is_active() {
            return None;
        }
        let mut guard = self.run.lock().unwrap_or_else(PoisonError::into_inner);
        let run = guard.as_mut()?;
        let result = apply(run);
        for line in run.take_lines() {
            // A closed stdout must not take the panel down.
            let _ = writeln!(out, "{line}");
        }
        let _ = out.flush();
        Some(result)
    }

    pub fn arm(&self, config: QualifyConfig, timing: Timing, now: Instant) {
        *self.run.lock().unwrap_or_else(PoisonError::into_inner) =
            Some(QualifyRun::new(config, timing, now));
        self.presented.store(false, Ordering::Relaxed);
        self.active.store(true, Ordering::Relaxed);
    }

    pub fn note_adapter(&self, info: &wgpu::AdapterInfo, out: &mut dyn Write) {
        if self.is_active() {
            let report = AdapterReport::from_info(info);
            self.with_run(out, |run| run.note_adapter(report));
        }
    }

    pub fn note_game_frame(&self, backend: GameBackend, out: &mut dyn Write) {
        self.with_run(out, |run| run.note_game_frame(backend));
    }

    /// True until the first swapchain present was recorded: the window's
    /// per-frame hook only builds a timestamp then.
    pub fn wants_present(&self) -> bool {
        self.is_active() && !self.presented.load(Ordering::Relaxed)
    }

    pub fn note_frame_presented(&self, now: Instant, out: &mut dyn Write) {
        if self
            .with_run(out, |run| run.note_frame_presented(now))
            .is_some()
        {
            self.presented.store(true, Ordering::Relaxed);
        }
    }

    pub fn note_shot_dir(&self, dir: &Path, out: &mut dyn Write) {
        self.with_run(out, |run| run.note_shot_dir(dir.to_path_buf()));
    }

    pub fn note_scene2_png(&self, path: &Path, out: &mut dyn Write) {
        self.with_run(out, |run| run.note_scene2_png(path.to_path_buf()));
    }

    pub fn scene_settled(&self, ingame: bool, now: Instant) -> bool {
        self.run
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .is_some_and(|run| run.scene_settled(ingame, now))
    }

    pub fn has_verdict(&self) -> bool {
        self.run
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .is_some_and(|run| run.sealed)
    }

    /// Advance the run; a terminal outcome has already written the receipt
    /// (a write failure turns a pass into a failure).
    pub fn tick(
        &self,
        ingame: bool,
        scene_state: i32,
        now: Instant,
        out: &mut dyn Write,
        fallback_dir: impl FnOnce() -> PathBuf,
    ) -> Outcome {
        let Some(outcome) = self.with_run(out, |run| run.tick(ingame, scene_state, now)) else {
            return Outcome::Pending;
        };
        match outcome {
            Outcome::Pending => Outcome::Pending,
            Outcome::Passed => match self.finish(None, now, out, fallback_dir) {
                Ok(()) => Outcome::Passed,
                Err(error) => Outcome::Failed(error),
            },
            Outcome::Failed(reason) => {
                let _ = self.finish(Some(reason.clone()), now, out, fallback_dir);
                Outcome::Failed(reason)
            }
        }
    }

    /// Seal the run and write its receipt, printing the `failed` and
    /// `receipt` markers. A run that is already sealed (or never armed) is
    /// left alone, so the first verdict wins.
    pub fn finish(
        &self,
        failure: Option<String>,
        now: Instant,
        out: &mut dyn Write,
        fallback_dir: impl FnOnce() -> PathBuf,
    ) -> Result<(), String> {
        let Some((receipt, target)) = self
            .with_run(out, |run| {
                run.seal(failure, now)
                    .map(|receipt| (receipt, run.receipt_path()))
            })
            .flatten()
        else {
            return Ok(());
        };
        let path = target.unwrap_or_else(|| fallback_dir().join(RECEIPT_FILE));
        let written = write_receipt_atomic(&path, &receipt);
        // `receipt` is announced only for a file that exists. A passing run
        // whose receipt cannot be written fails with that error instead.
        if let Err(error) = written {
            let reason = format!("receipt {}: {error}", path.display());
            if receipt.ok {
                let _ = writeln!(out, "[qualify] failed reason={}", quoted(&reason));
                let _ = out.flush();
            }
            return Err(reason);
        }
        let _ = writeln!(out, "{}", receipt_line(&path, receipt.ok));
        let _ = out.flush();
        Ok(())
    }
}

impl Default for Recorder {
    fn default() -> Self {
        Self::new()
    }
}

static RECORDER: Recorder = Recorder::new();

/// Where an early failure's receipt goes when neither `--qualify-receipt`
/// nor a shot directory exists: a fresh run directory, else the working
/// directory.
fn fallback_receipt_dir() -> PathBuf {
    frontend_core::live::create_shot_run_dir().unwrap_or_else(|error| {
        eprintln!(
            "[panel] shot dir: {error}; writing the qualify receipt to the working directory"
        );
        PathBuf::new()
    })
}

/// Arm the process-wide recorder for a `--qualify` run.
pub fn arm(config: QualifyConfig, timing: Timing) {
    RECORDER.arm(config, timing, Instant::now());
}

/// The panel window's adapter came up.
pub fn note_adapter(adapter: &wgpu::Adapter) {
    if RECORDER.is_active() {
        RECORDER.note_adapter(&adapter.get_info(), &mut std::io::stdout());
    }
}

/// The swapchain presented a frame.
pub fn note_frame_presented() {
    if RECORDER.wants_present() {
        RECORDER.note_frame_presented(Instant::now(), &mut std::io::stdout());
    }
}

/// A game frame was uploaded to the game image.
pub fn note_game_frame(backend: GameBackend) {
    if RECORDER.is_active() {
        RECORDER.note_game_frame(backend, &mut std::io::stdout());
    }
}

pub fn note_shot_dir(dir: &Path) {
    RECORDER.note_shot_dir(dir, &mut std::io::stdout());
}

pub fn note_scene2_png(path: &Path) {
    RECORDER.note_scene2_png(path, &mut std::io::stdout());
}

pub fn scene_settled(ingame: bool, now: Instant) -> bool {
    RECORDER.scene_settled(ingame, now)
}

/// Whether the armed run already has its verdict (receipt written).
pub fn has_verdict() -> bool {
    RECORDER.has_verdict()
}

/// One live-harness tick of the armed run.
pub fn tick(ingame: bool, scene_state: i32) -> Outcome {
    RECORDER.tick(
        ingame,
        scene_state,
        Instant::now(),
        &mut std::io::stdout(),
        fallback_receipt_dir,
    )
}

/// Record a failure the live tick did not see (boot, GPU, window error, or
/// a window closed early). No-op when the run already has its verdict.
pub fn fail(reason: &str) {
    if let Err(error) = RECORDER.finish(
        Some(reason.to_owned()),
        Instant::now(),
        &mut std::io::stdout(),
        fallback_receipt_dir,
    ) {
        eprintln!("[panel] qualify: {error}");
    }
}

#[cfg(test)]
#[path = "qualify_tests.rs"]
mod tests;
