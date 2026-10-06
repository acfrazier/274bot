//! Opt-in headed recording for `--live script_<name>` runs.
//!
//! When `HEADED_RECORD=1`, the panel periodically stages one extra whole-
//! window readback through the same render-pass capture path as the
//! scenario shots (no second screenshot implementation) and pipes the raw
//! frames to an `ffmpeg` background process as H.264. Off by default with
//! zero cost when off: the per-frame check is a single disabled branch.
//!
//! The frame loop never waits on disk or encoding: frames cross a bounded
//! channel with `try_send`, so a slow encoder drops a frame (counted)
//! instead of stalling the UI. The mp4 is finalized on PASS, FAIL and
//! window close, and a sidecar `.txt` maps video time to scenario step
//! and runner tick.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{sync_channel, Receiver, SyncSender};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

/// Env switch: `HEADED_RECORD=1` (also `true`/`yes`/`on`) enables recording.
pub const RECORD_ENV: &str = "HEADED_RECORD";
/// Env override for the capture cadence; frames per second, default 2.
pub const RECORD_FPS_ENV: &str = "HEADED_RECORD_FPS";
/// Env override for the ffmpeg binary (absolute path or `PATH` name).
pub const RECORD_FFMPEG_ENV: &str = "HEADED_RECORD_FFMPEG";
/// Homebrew install path the brief pins; first candidate when no override.
pub const RECORD_FFMPEG_HOMEBREW: &str = "/opt/homebrew/bin/ffmpeg";
/// Default capture cadence.
pub const DEFAULT_FPS: f64 = 2.0;
/// Fastest allowed cadence: bounds GPU readback cost and file size.
pub const MAX_FPS: f64 = 10.0;
/// Slowest allowed cadence; smaller positive values clamp here.
pub const MIN_FPS: f64 = 0.1;
/// Frames the encoder may lag before the UI thread drops instead of
/// stalling. Bounds recording memory to `8 × frame bytes`.
pub const RECORD_QUEUE: usize = 8;
/// Downscale target: the widest output width, never upscaling. Keeps the
/// default 2 fps run well under 500 MB/hour of mostly-static UI.
pub const RECORD_MAX_WIDTH: u32 = 1280;
/// Whole-window readback label for recording frames. Distinct from every
/// scenario shot label so the render pass can route the capture to the
/// recorder instead of the PNG ledger.
pub(crate) const RECORD_LABEL: &str = "__headed_record_frame";
/// Output names next to the terminal shot in the session's shot dir.
pub const RECORD_MP4_NAME: &str = "record.mp4";
pub const RECORD_SIDECAR_NAME: &str = "record.txt";

/// Enabled switch + cadence, read once from the environment.
#[derive(Debug, Clone)]
pub struct HeadedRecordConfig {
    /// Recording requested (`HEADED_RECORD=1` and friends).
    pub enabled: bool,
    /// Captures per second (`HEADED_RECORD_FPS`, default 2).
    pub fps: f64,
}

impl HeadedRecordConfig {
    /// Read the switch and cadence from the process environment.
    pub fn from_env() -> Self {
        Self {
            enabled: parse_enabled(std::env::var(RECORD_ENV).ok().as_deref()),
            fps: parse_fps(std::env::var(RECORD_FPS_ENV).ok().as_deref()),
        }
    }

    /// Per-frame capture interval for the pacer.
    pub fn interval(&self) -> Duration {
        interval_for_fps(self.fps)
    }
}

/// `None` (unset) is off. Whitespace-tolerant; only explicit truthy words
/// enable, everything else stays off.
pub fn parse_enabled(raw: Option<&str>) -> bool {
    matches!(
        raw.map(str::trim)
            .map(|s| s.to_ascii_lowercase())
            .as_deref(),
        Some("1" | "true" | "yes" | "on")
    )
}

/// Unset/empty/unparseable/non-finite/non-positive cadences fall back to
/// [`DEFAULT_FPS`]; the rest clamps to `[MIN_FPS, MAX_FPS]`.
pub fn parse_fps(raw: Option<&str>) -> f64 {
    let Some(text) = raw.map(str::trim).filter(|s| !s.is_empty()) else {
        return DEFAULT_FPS;
    };
    match text.parse::<f64>() {
        Ok(fps) if fps.is_finite() && fps > 0.0 => fps.clamp(MIN_FPS, MAX_FPS),
        _ => DEFAULT_FPS,
    }
}

/// Capture interval for a cadence (pure so tests pin the pacing math).
pub fn interval_for_fps(fps: f64) -> Duration {
    Duration::from_secs_f64(1.0 / fps)
}

/// Frame pacer: first poll captures immediately (frame 0 shows the run
/// start), then one capture per interval. A long stall advances the
/// deadline from now, so the loop never bursts to catch up.
#[derive(Debug)]
pub struct RecordPacer {
    interval: Duration,
    next: Option<Instant>,
}

impl RecordPacer {
    /// Pacer for `interval` (see [`HeadedRecordConfig::interval`]).
    pub fn new(interval: Duration) -> Self {
        Self {
            interval,
            next: None,
        }
    }

    /// True when a frame is due at `now` (and re-arms the deadline).
    pub fn poll(&mut self, now: Instant) -> bool {
        match self.next {
            None => {
                self.next = Some(now + self.interval);
                true
            }
            Some(next) if now >= next => {
                self.next = Some(now + self.interval);
                true
            }
            _ => false,
        }
    }
}

/// Scenario position for one recorded frame (owned so the sidecar line
/// outlives the runner lock).
#[derive(Debug, Clone)]
pub struct StepInfo {
    /// Current step index (0-based; `total` while proving/done).
    pub step: usize,
    /// Total run steps.
    pub total: usize,
    /// Current step (or phase) name.
    pub name: String,
    /// Runner dirty-snapshot increments across the run.
    pub tick: u32,
}

/// One decoded whole-window frame ready for the encoder.
#[derive(Debug)]
pub struct RecordFrame {
    /// Capture width in pixels (must match the armed dims).
    pub width: u32,
    /// Capture height in pixels.
    pub height: u32,
    /// RGBA8 bytes, `4 × width × height`.
    pub rgba: Vec<u8>,
}

/// One sidecar line: video time → scenario step + runner tick.
pub fn format_sidecar_line(
    t_secs: f64,
    frame: u64,
    step: usize,
    total: usize,
    tick: u32,
    name: &str,
) -> String {
    format!(
        "t={t_secs:.3} frame={frame} step={step}/{total} tick={tick} name=\"{}\"",
        sanitize_record_name(name)
    )
}

/// Sidecar header: names the scenario and cadence, documents the columns.
pub fn format_sidecar_header(scenario: &str, fps: f64) -> String {
    format!(
        "# headed recording scenario={scenario} fps={fps}\n\
         # t=frame/fps frame=<n> step=<i>/<total> tick=<runner ticks> name=\"<step>\""
    )
}

/// The live recording: spawned ffmpeg plus the bounded frame queue.
/// `thread` is empty until the first staged readback fixes the pipe
/// geometry; `pending_receiver` waits for that spawn. Unit tests build
/// one without either.
struct ActiveRecording {
    sender: SyncSender<Vec<u8>>,
    pending_receiver: Option<Receiver<Vec<u8>>>,
    thread: Option<JoinHandle<EncoderOutcome>>,
    mp4_path: PathBuf,
    txt_path: PathBuf,
    fps: f64,
    width: u32,
    height: u32,
    lines: Vec<String>,
    queued: u64,
    dropped: u64,
    size_mismatch: u64,
}
/// Step names are code-authored, but keep the quoted column parseable even
/// if one ever carries a quote.
pub fn sanitize_record_name(name: &str) -> String {
    name.replace('"', "'")
}

/// Resolve the ffmpeg binary: explicit `HEADED_RECORD_FFMPEG` override,
/// then the Homebrew path, then `PATH`. Returns the display string; the
/// spawn itself is the existence check, so a missing binary fails once at
/// arm time with one warning.
fn ffmpeg_candidate() -> String {
    if let Ok(custom) = std::env::var(RECORD_FFMPEG_ENV) {
        if !custom.trim().is_empty() {
            return custom;
        }
    }
    if Path::new(RECORD_FFMPEG_HOMEBREW).exists() {
        return RECORD_FFMPEG_HOMEBREW.to_string();
    }
    "ffmpeg".to_string()
}

/// Outcome of the background encoder thread.
#[derive(Debug)]
struct EncoderOutcome {
    frames_written: u64,
    error: Option<String>,
}

impl ActiveRecording {
    /// Queue one frame; false (counted) when the encoder lags or died.
    /// Never blocks: the UI/slot frame loop stays off disk and off encode.
    fn push(&mut self, bytes: Vec<u8>) -> bool {
        match self.sender.try_send(bytes) {
            Ok(()) => {
                self.queued += 1;
                true
            }
            Err(_) => {
                self.dropped += 1;
                false
            }
        }
    }

    #[cfg(test)]
    fn for_test(sender: SyncSender<Vec<u8>>, fps: f64) -> Self {
        Self {
            sender,
            pending_receiver: None,
            thread: None,
            mp4_path: PathBuf::from("record.mp4"),
            txt_path: PathBuf::from("record.txt"),
            fps,
            width: 64,
            height: 64,
            lines: Vec::new(),
            queued: 0,
            dropped: 0,
            size_mismatch: 0,
        }
    }
}

/// Opt-in headed recorder. Created per panel run from the environment;
/// armed lazily once the live run's shot dir exists. Drop finalizes the
/// file, so window close always leaves a playable mp4.
pub struct HeadedRecord {
    config: HeadedRecordConfig,
    pacer: RecordPacer,
    active: Option<ActiveRecording>,
    /// Latched once `finish` runs: a terminal run never re-arms.
    done: bool,
}

impl HeadedRecord {
    /// Read the switch/cadence from the environment (off by default).
    pub fn from_env() -> Self {
        let config = HeadedRecordConfig::from_env();
        let interval = config.interval();
        Self {
            config,
            pacer: RecordPacer::new(interval),
            active: None,
            done: false,
        }
    }

    /// Recording requested and neither armed nor finished.
    pub fn needs_arm(&self) -> bool {
        self.config.enabled && self.active.is_none() && !self.done
    }

    /// True once ffmpeg is spawned (frames are flowing or due).
    pub fn is_active(&self) -> bool {
        self.active.is_some()
    }

    /// Stage `<shot_dir>/record.mp4` for recording. One warning and a
    /// permanent disable when the ffmpeg binary is missing; the run
    /// continues. The encoder itself spawns on the first staged readback,
    /// which fixes the rawvideo geometry.
    pub fn arm(&mut self, shot_dir: &Path, scenario: &str) {
        if !self.needs_arm() {
            return;
        }
        let mp4_path = shot_dir.join(RECORD_MP4_NAME);
        let txt_path = shot_dir.join(RECORD_SIDECAR_NAME);
        let fps = self.config.fps;
        // Only resolve the binary here so a missing ffmpeg warns once now;
        // the encoder itself spawns on the first staged readback, which
        // fixes the rawvideo geometry (dims are unknown until then).
        let binary = ffmpeg_candidate();
        if which_ffmpeg(&binary).is_none() {
            eprintln!(
                "[panel] headed record: ffmpeg not found ({binary}); \
                 continuing without recording"
            );
            self.config.enabled = false;
            return;
        }
        eprintln!(
            "[panel] headed record: {scenario} -> {} @ {fps} fps",
            mp4_path.display()
        );
        let (sender, receiver) = sync_channel::<Vec<u8>>(RECORD_QUEUE);
        let lines = vec![format_sidecar_header(scenario, fps)];
        self.active = Some(ActiveRecording {
            sender,
            pending_receiver: Some(receiver),
            thread: None,
            mp4_path,
            txt_path,
            fps,
            width: 0,
            height: 0,
            lines,
            queued: 0,
            dropped: 0,
            size_mismatch: 0,
        });
    }

    /// True when a capture is due this frame (only while armed). Advances
    /// the pacer, so the caller must stage the readback when true.
    pub fn is_due(&mut self, now: Instant) -> bool {
        if self.active.is_none() {
            return false;
        }
        self.pacer.poll(now)
    }

    /// Move staged readback frames into the encoder queue, appending one
    /// sidecar line per queued frame. Frames whose dims differ from the
    /// first (window resized mid-run) are dropped and counted: the
    /// rawvideo pipe has fixed geometry.
    pub fn pump_frames(&mut self, frames: Vec<RecordFrame>, step: &StepInfo) {
        let Some(active) = self.active.as_mut() else {
            return;
        };
        for frame in frames {
            if active.width == 0 {
                // First staged readback fixes the pipe geometry: spawn the
                // encoder now so a missing/resized window never writes a
                // header the frames cannot honor.
                active.width = frame.width;
                active.height = frame.height;
                let receiver = active.pending_receiver.take();
                spawn_encoder(active, receiver);
                if active.thread.is_none() {
                    // Spawn failed (already warned): stop accepting frames.
                    self.config.enabled = false;
                    self.active = None;
                    return;
                }
            }
            if frame.width != active.width || frame.height != active.height {
                active.size_mismatch += 1;
                continue;
            }
            let expected = 4 * active.width as usize * active.height as usize;
            if frame.rgba.len() != expected {
                active.size_mismatch += 1;
                continue;
            }
            // Video time only advances for frames the encoder receives, so
            // the line is appended only when the queue accepts the frame.
            let t_secs = active.queued as f64 / active.fps;
            let frame_index = active.queued;
            if active.push(frame.rgba) {
                active.lines.push(format_sidecar_line(
                    t_secs,
                    frame_index,
                    step.step,
                    step.total,
                    step.tick,
                    &step.name,
                ));
            }
        }
    }

    /// Close the pipe, wait for ffmpeg's moov trailer, write the sidecar,
    /// and print the summary. Idempotent: PASS/FAIL calls it, Drop covers
    /// window close.
    pub fn finish(&mut self) {
        let Some(mut active) = self.active.take() else {
            return;
        };
        self.done = true;
        let Some(thread) = active.thread.take() else {
            // Armed but no frame ever staged (instant exit): the encoder
            // never spawned, so there is no file to finalize.
            return;
        };
        // Dropping the last sender disconnects the encoder loop, which
        // then closes stdin so ffmpeg writes a playable trailer.
        drop(active.pending_receiver.take());
        let sender = std::mem::replace(&mut active.sender, sync_channel::<Vec<u8>>(0).0);
        drop(sender);
        let outcome = thread.join().unwrap_or(EncoderOutcome {
            frames_written: 0,
            error: Some("encoder thread panicked".to_string()),
        });
        let mut trailer = format!(
            "# end frames={} dropped={} size_mismatch={} encoder_wrote={}",
            active.queued, active.dropped, active.size_mismatch, outcome.frames_written
        );
        if let Some(error) = outcome.error.as_deref() {
            trailer.push_str(&format!(" encode_error={error}"));
        }
        active.lines.push(trailer);
        let sidecar = active.lines.join("\n") + "\n";
        if let Err(error) = std::fs::write(&active.txt_path, sidecar) {
            eprintln!(
                "[panel] headed record: sidecar {}: {error}",
                active.txt_path.display()
            );
        }
        if let Some(error) = outcome.error.as_deref() {
            eprintln!("[panel] headed record: encode error: {error}");
        }
        eprintln!(
            "[panel] headed record: {} ({} frames, {} dropped, {} resized)",
            active.mp4_path.display(),
            active.queued,
            active.dropped,
            active.size_mismatch
        );
    }
}

impl Drop for HeadedRecord {
    fn drop(&mut self) {
        self.finish();
    }
}

/// Probe the binary without spawning a run: absolute paths need to exist;
/// bare names must resolve on `PATH`.
fn which_ffmpeg(binary: &str) -> Option<PathBuf> {
    let path = Path::new(binary);
    if path.components().count() > 1 {
        return path.is_file().then(|| path.to_path_buf());
    }
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths)
            .map(|dir| dir.join(binary))
            .find(|candidate| candidate.is_file())
    })
}

/// Start the background encoder on the first frame's geometry. The render
/// loop never touches this thread except through the bounded queue.
fn spawn_encoder(active: &mut ActiveRecording, receiver: Option<Receiver<Vec<u8>>>) {
    let Some(receiver) = receiver else {
        eprintln!("[panel] headed record: encoder queue missing; stopping");
        return;
    };
    let binary = ffmpeg_candidate();
    let size = format!("{}x{}", active.width, active.height);
    let fps = format!("{}", active.fps);
    let scale = format!("scale='min({},iw)':-2", RECORD_MAX_WIDTH);
    let mut child: Child = match Command::new(&binary)
        .args([
            "-hide_banner",
            "-loglevel",
            "warning",
            "-y",
            "-f",
            "rawvideo",
            "-pix_fmt",
            "rgba",
            "-s",
            &size,
            "-framerate",
            &fps,
            "-i",
            "pipe:0",
            "-vf",
            &scale,
            "-c:v",
            "libx264",
            "-preset",
            "veryfast",
            "-crf",
            "23",
            "-pix_fmt",
            "yuv420p",
            "-movflags",
            "+faststart",
        ])
        .arg(&active.mp4_path)
        .stdin(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
    {
        Ok(child) => child,
        Err(error) => {
            eprintln!("[panel] headed record: ffmpeg spawn failed: {error}; stopping");
            return;
        }
    };
    let stdin: ChildStdin = match child.stdin.take() {
        Some(stdin) => stdin,
        None => {
            eprintln!("[panel] headed record: ffmpeg stdin unavailable; stopping");
            return;
        }
    };
    active.thread = Some(thread::spawn(move || run_encoder(stdin, receiver, child)));
}

/// Background encode: never on the UI/slot threads. Ends when the last
/// sender drops (run finalizes), then waits for the trailer write.
fn run_encoder(
    mut stdin: ChildStdin,
    receiver: Receiver<Vec<u8>>,
    mut child: Child,
) -> EncoderOutcome {
    let mut frames_written = 0u64;
    let mut error = None;
    while let Ok(bytes) = receiver.recv() {
        if let Err(io) = stdin.write_all(&bytes) {
            error = Some(format!("stdin write: {io}"));
            break;
        }
        frames_written += 1;
    }
    drop(stdin);
    match child.wait() {
        Ok(status) if status.success() => {}
        Ok(status) => error = error.or(Some(format!("ffmpeg exit {status}"))),
        Err(io) => error = Some(format!("ffmpeg wait: {io}")),
    }
    EncoderOutcome {
        frames_written,
        error,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_switch_parses_truthy_words_only() {
        assert!(!parse_enabled(None));
        assert!(!parse_enabled(Some("")));
        assert!(!parse_enabled(Some("0")));
        assert!(!parse_enabled(Some("false")));
        assert!(!parse_enabled(Some("no")));
        assert!(parse_enabled(Some("1")));
        assert!(parse_enabled(Some("true")));
        assert!(parse_enabled(Some("TRUE")));
        assert!(parse_enabled(Some(" yes ")));
        assert!(parse_enabled(Some("on")));
    }

    #[test]
    fn record_fps_defaults_and_clamps() {
        assert_eq!(parse_fps(None), DEFAULT_FPS);
        assert_eq!(parse_fps(Some("")), DEFAULT_FPS);
        assert_eq!(parse_fps(Some("2")), 2.0);
        assert_eq!(parse_fps(Some("0.5")), 0.5);
        assert_eq!(parse_fps(Some("0")), DEFAULT_FPS);
        assert_eq!(parse_fps(Some("-3")), DEFAULT_FPS);
        assert_eq!(parse_fps(Some("abc")), DEFAULT_FPS);
        assert_eq!(parse_fps(Some("inf")), DEFAULT_FPS);
        assert_eq!(parse_fps(Some("NaN")), DEFAULT_FPS);
        assert_eq!(parse_fps(Some("100")), MAX_FPS);
        assert_eq!(parse_fps(Some("0.01")), MIN_FPS);
    }

    #[test]
    fn record_interval_matches_cadence() {
        assert_eq!(interval_for_fps(2.0), Duration::from_millis(500));
        assert_eq!(interval_for_fps(1.0), Duration::from_secs(1));
    }

    #[test]
    fn pacer_captures_first_frame_then_gates() {
        let mut pacer = RecordPacer::new(Duration::from_millis(500));
        let start = Instant::now();
        assert!(pacer.poll(start), "frame 0 captures the run start");
        assert!(!pacer.poll(start), "same instant is gated");
        assert!(!pacer.poll(start + Duration::from_millis(499)));
        assert!(pacer.poll(start + Duration::from_millis(500)));
        assert!(!pacer.poll(start + Duration::from_millis(501)));
    }

    #[test]
    fn pacer_never_bursts_after_a_stall() {
        let mut pacer = RecordPacer::new(Duration::from_millis(500));
        let start = Instant::now();
        assert!(pacer.poll(start));
        // A 10 s stall yields one frame, then gates again from now.
        let late = start + Duration::from_secs(10);
        assert!(pacer.poll(late));
        assert!(!pacer.poll(late));
        assert!(!pacer.poll(late + Duration::from_millis(499)));
        assert!(pacer.poll(late + Duration::from_millis(500)));
    }

    #[test]
    fn sidecar_line_maps_video_time_to_step_and_tick() {
        assert_eq!(
            format_sidecar_line(0.0, 0, 0, 12, 0, "seeding"),
            "t=0.000 frame=0 step=0/12 tick=0 name=\"seeding\""
        );
        assert_eq!(
            format_sidecar_line(12.5, 25, 3, 12, 1234, "take the pot"),
            "t=12.500 frame=25 step=3/12 tick=1234 name=\"take the pot\""
        );
        assert_eq!(
            format_sidecar_line(0.5, 1, 12, 12, 99, "done"),
            "t=0.500 frame=1 step=12/12 tick=99 name=\"done\""
        );
    }

    #[test]
    fn sidecar_name_keeps_the_quoted_column_parseable() {
        assert_eq!(sanitize_record_name("take \"the\" pot"), "take 'the' pot");
        assert_eq!(
            format_sidecar_line(0.0, 0, 1, 2, 3, "say \"hi\""),
            "t=0.000 frame=0 step=1/2 tick=3 name=\"say 'hi'\""
        );
    }

    #[test]
    fn sidecar_header_names_scenario_and_cadence() {
        let header = format_sidecar_header("quester_path", 2.0);
        assert!(header.contains("scenario=quester_path"), "{header}");
        assert!(header.contains("fps=2"), "{header}");
        assert!(header.contains("tick=<runner ticks>"), "{header}");
    }

    #[test]
    fn full_queue_drops_instead_of_stalling() {
        let (sender, receiver) = sync_channel::<Vec<u8>>(1);
        let mut active = ActiveRecording::for_test(sender, 2.0);
        active.push(vec![1]);
        active.push(vec![2]);
        active.push(vec![3]);
        assert_eq!(active.queued, 1);
        assert_eq!(active.dropped, 2, "full queue drops, never blocks");
        assert_eq!(receiver.recv().unwrap(), vec![1]);
    }

    #[test]
    fn dead_encoder_counts_as_drops() {
        let (sender, receiver) = sync_channel::<Vec<u8>>(4);
        drop(receiver);
        let mut active = ActiveRecording::for_test(sender, 2.0);
        active.push(vec![1]);
        assert_eq!(active.queued, 0);
        assert_eq!(active.dropped, 1);
    }
}
