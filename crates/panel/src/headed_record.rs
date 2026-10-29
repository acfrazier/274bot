//! Opt-in headed recording for any panel session.
//!
//! When `HEADED_RECORD=1`, the panel periodically stages one extra whole-
//! window readback through the same render-pass capture path as the
//! scenario shots (no second screenshot implementation) and pipes the raw
//! frames to an `ffmpeg` background process as H.264. A `--live` run and
//! an interactive session record the same way; only the sidecar's
//! scenario name differs (`interactive` without a live runner). Off by
//! default with zero cost when off: the per-frame check is a single
//! disabled branch.
//!
//! The frame loop never waits on disk or encoding: frames cross a bounded
//! channel with `try_send`, so a slow encoder drops a frame (counted)
//! instead of stalling the UI. The mp4 is finalized on PASS, FAIL and
//! window close, and a sidecar `.txt` maps video time to scenario step
//! and runner tick. A second sidecar, `settings.txt`, records one line
//! per saved-setting state change (pending, failed, cleared, saved) as
//! it happens, so a headed check can verify the save states by script.

use std::fs::File;
use std::io::{BufWriter, Write as _};
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
/// Settings-state sidecar next to `record.txt`: one line per state change.
pub const RECORD_SETTINGS_NAME: &str = "settings.txt";

const SETTINGS_INCOMPLETE_MARKER: &str =
    "# incomplete: one or more setting transitions were dropped from a bounded event queue";

/// Send a recorder diagnostic through the host log's category, level,
/// session sink, and redaction.
pub(crate) fn log_diagnostic(level: api::hostlog::Level, message: impl std::fmt::Display) {
    api::host_log!(api::hostlog::Category::Lifecycle, level, "{message}");
}

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

/// One saved-setting state in the settings sidecar. `Pending`, `Failed`
/// and `Saved` are the states the settings owners report for a write
/// (frontend-core's preference writes and the panel's own preference
/// members). `Cleared` is reported directly by an owner that removes a
/// retained failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingState {
    /// An accepted write has not reached its terminal result.
    Pending,
    /// A write failed; the line's error column says why.
    Failed,
    /// A retained failure was cleared.
    Cleared,
    /// A write became durable.
    Saved,
}

impl SettingState {
    /// The state's word in a settings sidecar line.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Failed => "failed",
            Self::Cleared => "cleared",
            Self::Saved => "saved",
        }
    }
}

/// Settings names and error text are shown to the operator elsewhere and
/// may carry quotes or newlines; keep the one-line-per-change format
/// parseable.
pub fn sanitize_settings_text(text: &str) -> String {
    text.replace('"', "'").replace(['\n', '\r'], " ")
}

/// One settings sidecar line: seconds since the recording armed, the
/// setting, its new state, and the error text (`failed` lines only;
/// empty otherwise).
pub fn format_settings_line(
    t_secs: f64,
    setting: &str,
    state: SettingState,
    error: &str,
) -> String {
    let error = if state == SettingState::Failed {
        error
    } else {
        ""
    };
    format!(
        "t={t_secs:.3} setting=\"{}\" state={} error=\"{}\"",
        sanitize_settings_text(setting),
        state.as_str(),
        sanitize_settings_text(error)
    )
}

/// Settings sidecar header: names the scenario, documents the columns.
pub fn format_settings_header(scenario: &str) -> String {
    format!(
        "# headed recording settings scenario={scenario}\n\
         # t=<seconds since armed> setting=\"<setting>\" \
         state=pending|failed|cleared|saved error=\"<error or empty>\""
    )
}

/// The last sidecar state of each setting seen, so the sidecar writes
/// one line per change and none for a repeated report of the same state.
/// Bounded by the number of distinct settings; built only while a
/// recording is armed.
#[derive(Debug, Default)]
pub struct SettingsTracker {
    last: Vec<(String, SettingState, String)>,
}

impl SettingsTracker {
    /// The lines one reported transition writes. An owner-reported
    /// `Cleared` state is written before the next state. The recorder does
    /// not infer a clear from a pending or saved write.
    pub fn lines_for(
        &mut self,
        t_secs: f64,
        setting: &str,
        state: SettingState,
        error: &str,
    ) -> [Option<String>; 2] {
        let error = if state == SettingState::Failed {
            error
        } else {
            ""
        };
        let previous = self
            .last
            .iter()
            .find(|(name, _, _)| name == setting)
            .map(|(_, state, error)| (*state, error.as_str()));
        if previous == Some((state, error)) {
            return [None, None];
        }
        let cleared = (state == SettingState::Cleared)
            .then(|| format_settings_line(t_secs, setting, SettingState::Cleared, ""));
        let line = (state != SettingState::Cleared)
            .then(|| format_settings_line(t_secs, setting, state, error));
        match self.last.iter_mut().find(|(name, _, _)| name == setting) {
            Some(entry) => {
                entry.1 = state;
                entry.2.clear();
                entry.2.push_str(error);
            }
            None => self
                .last
                .push((setting.to_owned(), state, error.to_owned())),
        }
        [cleared, line]
    }
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
    /// When the recording armed; settings lines count seconds from here.
    started: Instant,
    /// The settings sidecar, appended on change only and flushed per
    /// line. `None` when the file could not be opened (warned once).
    settings_file: Option<BufWriter<File>>,
    /// Last settings state per setting, for change-only lines.
    settings: SettingsTracker,
    /// The settings queue dropped one or more transitions.
    settings_incomplete: bool,
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
            started: Instant::now(),
            settings_file: None,
            settings: SettingsTracker::default(),
            settings_incomplete: false,
            queued: 0,
            dropped: 0,
            size_mismatch: 0,
        }
    }
}

/// Opt-in headed recorder. Created per panel run from the environment;
/// armed lazily once the session's shot dir exists. Drop finalizes the
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

    /// Whether recording was requested (`HEADED_RECORD=1`), armed or not.
    pub fn enabled(&self) -> bool {
        self.config.enabled
    }

    /// Give up recording for this run without arming (the shot dir could
    /// not be created): no further arm attempts.
    pub fn abandon(&mut self) {
        self.config.enabled = false;
        self.done = true;
    }

    /// Record one saved-setting transition in the settings sidecar. A
    /// no-op before arming and after finish. Writes and flushes only when
    /// the setting's state changed, so a frame with no settings change
    /// neither allocates nor touches the disk.
    pub fn note_setting(&mut self, setting: &str, state: SettingState, error: &str) {
        let Some(active) = self.active.as_mut() else {
            return;
        };
        let t_secs = active.started.elapsed().as_secs_f64();
        let lines = active.settings.lines_for(t_secs, setting, state, error);
        if lines.iter().all(Option::is_none) {
            return;
        }
        let Some(writer) = active.settings_file.as_mut() else {
            return;
        };
        let mut written = true;
        for line in lines.into_iter().flatten() {
            if writeln!(writer, "{line}").is_err() {
                written = false;
                break;
            }
        }
        if written && writer.flush().is_err() {
            written = false;
        }
        if !written {
            log_diagnostic(
                api::hostlog::Level::Warn,
                "headed record: settings sidecar write failed; closing it",
            );
            active.settings_file = None;
        }
    }

    /// Mark the settings sidecar incomplete after an event queue overflow.
    /// Called only when evidence was lost, never on an idle frame.
    pub fn note_settings_incomplete(&mut self) {
        let Some(active) = self.active.as_mut() else {
            return;
        };
        if active.settings_incomplete {
            return;
        }
        active.settings_incomplete = true;
        let write_failed = active.settings_file.as_mut().is_some_and(|writer| {
            writeln!(writer, "{SETTINGS_INCOMPLETE_MARKER}")
                .and_then(|_| writer.flush())
                .is_err()
        });
        if write_failed {
            log_diagnostic(
                api::hostlog::Level::Warn,
                "headed record: settings sidecar incomplete marker write failed; closing it",
            );
            active.settings_file = None;
        }
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
            log_diagnostic(
                api::hostlog::Level::Warn,
                format_args!(
                    "headed record: ffmpeg not found ({binary}); continuing without recording"
                ),
            );
            self.config.enabled = false;
            return;
        }
        log_diagnostic(
            api::hostlog::Level::Info,
            format_args!(
                "headed record: {scenario} -> {} @ {fps} fps",
                mp4_path.display()
            ),
        );
        let (sender, receiver) = sync_channel::<Vec<u8>>(RECORD_QUEUE);
        let lines = vec![format_sidecar_header(scenario, fps)];
        let settings_path = shot_dir.join(RECORD_SETTINGS_NAME);
        let settings_file = match File::create(&settings_path) {
            Ok(file) => {
                let mut writer = BufWriter::new(file);
                let header = format_settings_header(scenario);
                match writeln!(writer, "{header}").and_then(|_| writer.flush()) {
                    Ok(()) => Some(writer),
                    Err(error) => {
                        log_diagnostic(
                            api::hostlog::Level::Warn,
                            format_args!(
                                "headed record: settings sidecar {}: {error}",
                                settings_path.display()
                            ),
                        );
                        None
                    }
                }
            }
            Err(error) => {
                log_diagnostic(
                    api::hostlog::Level::Warn,
                    format_args!(
                        "headed record: settings sidecar {}: {error}",
                        settings_path.display()
                    ),
                );
                None
            }
        };
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
            started: Instant::now(),
            settings_file,
            settings: SettingsTracker::default(),
            settings_incomplete: false,
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
        // Settings lines flush as they are written; this last flush only
        // covers a header-only sidecar.
        if let Some(writer) = active.settings_file.as_mut() {
            let _ = writer.flush();
        }
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
            log_diagnostic(
                api::hostlog::Level::Warn,
                format_args!(
                    "headed record: sidecar {}: {error}",
                    active.txt_path.display()
                ),
            );
        }
        if let Some(error) = outcome.error.as_deref() {
            log_diagnostic(
                api::hostlog::Level::Error,
                format_args!("headed record: encode error: {error}"),
            );
        }
        log_diagnostic(
            api::hostlog::Level::Info,
            format_args!(
                "headed record: {} ({} frames, {} dropped, {} resized)",
                active.mp4_path.display(),
                active.queued,
                active.dropped,
                active.size_mismatch
            ),
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
        log_diagnostic(
            api::hostlog::Level::Error,
            "headed record: encoder queue missing; stopping",
        );
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
            log_diagnostic(
                api::hostlog::Level::Error,
                format_args!("headed record: ffmpeg spawn failed: {error}; stopping"),
            );
            return;
        }
    };
    let stdin: ChildStdin = match child.stdin.take() {
        Some(stdin) => stdin,
        None => {
            log_diagnostic(
                api::hostlog::Level::Error,
                "headed record: ffmpeg stdin unavailable; stopping",
            );
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

    /// T4.1 characterization (§23 §2.7 recording): today's recording takes
    /// whole-window readbacks, not per-head outputs. It accepts only frames
    /// matching the armed pipe geometry, queues them without blocking, and
    /// never asks the render path for a new frame.
    #[test]
    fn pump_queues_only_frames_matching_the_armed_geometry() {
        let (sender, receiver) = sync_channel::<Vec<u8>>(RECORD_QUEUE);
        let mut record = HeadedRecord {
            config: HeadedRecordConfig {
                enabled: true,
                fps: 2.0,
            },
            pacer: RecordPacer::new(Duration::from_millis(500)),
            active: Some(ActiveRecording::for_test(sender, 2.0)),
            done: false,
        };
        let frame = |width: u32, height: u32, fill: u8| RecordFrame {
            width,
            height,
            rgba: vec![fill; 4 * width as usize * height as usize],
        };
        let step = StepInfo {
            step: 1,
            total: 2,
            name: "walk".into(),
            tick: 7,
        };
        // 32×128 carries exactly the armed 64×64 byte count: only the
        // geometry check can refuse it.
        record.pump_frames(
            vec![frame(64, 64, 1), frame(32, 128, 2), frame(64, 64, 3)],
            &step,
        );
        let mut short = frame(64, 64, 4);
        short.rgba.pop();
        record.pump_frames(vec![short], &step);
        let active = record.active.as_ref().expect("still armed");
        assert_eq!(active.queued, 2, "only same-geometry frames queue");
        assert_eq!(active.size_mismatch, 2, "resized and short frames drop");
        assert_eq!(active.lines.len(), 2, "one sidecar line per queued frame");
        assert_eq!(receiver.try_recv().unwrap()[0], 1);
        assert_eq!(receiver.try_recv().unwrap()[0], 3);
        assert!(receiver.try_recv().is_err());
    }

    #[test]
    fn settings_line_names_setting_state_and_error() {
        assert_eq!(
            format_settings_line(1.5, "capture", SettingState::Pending, ""),
            "t=1.500 setting=\"capture\" state=pending error=\"\""
        );
        assert_eq!(
            format_settings_line(2.0, "capture", SettingState::Saved, "ignored"),
            "t=2.000 setting=\"capture\" state=saved error=\"\"",
            "only a failed line carries error text"
        );
        assert_eq!(
            format_settings_line(
                7.25,
                "navigation preference",
                SettingState::Failed,
                "preferences is busy"
            ),
            "t=7.250 setting=\"navigation preference\" state=failed \
             error=\"preferences is busy\""
        );
        assert_eq!(
            format_settings_line(9.0, "capture", SettingState::Cleared, ""),
            "t=9.000 setting=\"capture\" state=cleared error=\"\""
        );
    }

    #[test]
    fn settings_line_keeps_one_line_per_change_parseable() {
        assert_eq!(
            format_settings_line(
                0.5,
                "capture",
                SettingState::Failed,
                "say \"hi\"\nthen\rstop"
            ),
            "t=0.500 setting=\"capture\" state=failed error=\"say 'hi' then stop\""
        );
    }

    #[test]
    fn settings_header_documents_the_columns() {
        let header = format_settings_header("interactive");
        assert!(header.contains("scenario=interactive"), "{header}");
        assert!(
            header.contains("state=pending|failed|cleared|saved"),
            "{header}"
        );
    }

    #[test]
    fn settings_tracker_writes_one_line_per_change() {
        let mut tracker = SettingsTracker::default();
        assert_eq!(
            tracker.lines_for(1.0, "capture", SettingState::Pending, ""),
            [
                None,
                Some("t=1.000 setting=\"capture\" state=pending error=\"\"".to_string())
            ]
        );
        // The same state reported again is not a change.
        assert_eq!(
            tracker.lines_for(1.5, "capture", SettingState::Pending, ""),
            [None, None]
        );
        assert_eq!(
            tracker.lines_for(2.0, "capture", SettingState::Saved, ""),
            [
                None,
                Some("t=2.000 setting=\"capture\" state=saved error=\"\"".to_string())
            ]
        );
        // Another setting tracks independently.
        assert_eq!(
            tracker.lines_for(2.5, "sidecar_50", SettingState::Pending, ""),
            [
                None,
                Some("t=2.500 setting=\"sidecar_50\" state=pending error=\"\"".to_string())
            ]
        );
    }

    #[test]
    fn settings_tracker_failed_then_saved_does_not_invent_a_clear() {
        let mut tracker = SettingsTracker::default();
        tracker.lines_for(1.0, "capture", SettingState::Pending, "");
        tracker.lines_for(6.0, "capture", SettingState::Failed, "preferences is busy");
        assert_eq!(
            tracker.lines_for(9.0, "capture", SettingState::Saved, ""),
            [
                None,
                Some("t=9.000 setting=\"capture\" state=saved error=\"\"".to_string())
            ],
            "a saved report does not prove that the owner cleared its failure"
        );
    }

    #[test]
    fn settings_tracker_failed_then_pending_does_not_invent_a_clear() {
        let mut tracker = SettingsTracker::default();
        tracker.lines_for(1.0, "capture", SettingState::Failed, "disk full");
        assert_eq!(
            tracker.lines_for(3.0, "capture", SettingState::Pending, ""),
            [
                None,
                Some("t=3.000 setting=\"capture\" state=pending error=\"\"".to_string())
            ],
            "an accepted retry does not prove that the owner cleared its failure"
        );
    }

    #[test]
    fn settings_tracker_cleared_reports_and_repeated_failures() {
        let mut tracker = SettingsTracker::default();
        // An explicit owner clear remains evidence if recording starts
        // after the failure entered the owner's state.
        assert_eq!(
            tracker.lines_for(1.0, "capture", SettingState::Cleared, ""),
            [
                Some("t=1.000 setting=\"capture\" state=cleared error=\"\"".to_string()),
                None
            ]
        );
        assert_eq!(
            tracker.lines_for(1.5, "capture", SettingState::Cleared, ""),
            [None, None],
            "a repeated explicit clear is not a new transition"
        );
        tracker.lines_for(2.0, "capture", SettingState::Failed, "one");
        // A second failure with new text is a change of its own.
        assert_eq!(
            tracker.lines_for(3.0, "capture", SettingState::Failed, "two"),
            [
                None,
                Some("t=3.000 setting=\"capture\" state=failed error=\"two\"".to_string())
            ]
        );
        assert_eq!(
            tracker.lines_for(4.0, "capture", SettingState::Failed, "two"),
            [None, None],
            "the same failure reported again is not a change"
        );
        // An owner that clears the failure (expiry, a dismiss) writes one
        // cleared line and no state line after it.
        assert_eq!(
            tracker.lines_for(20.0, "capture", SettingState::Cleared, ""),
            [
                Some("t=20.000 setting=\"capture\" state=cleared error=\"\"".to_string()),
                None
            ]
        );
        assert_eq!(
            tracker.lines_for(21.0, "capture", SettingState::Pending, ""),
            [
                None,
                Some("t=21.000 setting=\"capture\" state=pending error=\"\"".to_string())
            ],
            "expiry clears the retained failure once; a later retry adds no inferred clear"
        );
    }

    #[test]
    fn recorder_diagnostics_reach_the_redacted_host_log() {
        use frontend_core::log::{global, LogScope, LogView};

        let log = global();
        let secret = "headed_record_log_secret";
        api::hostlog::register_secret(secret);
        let message = format!("headed record diagnostic route {secret}");
        log_diagnostic(api::hostlog::Level::Warn, &message);

        let mut view = LogView::new(LogScope::Process);
        view.edit_filter(|filter| filter.set_text("headed record diagnostic route"));
        log.refresh(&mut view);
        assert_eq!(view.len(), 1, "{}", view.to_text());
        let row = &view.rows()[0];
        assert_eq!(row.level(), api::hostlog::Level::Warn);
        assert_eq!(row.source(), api::hostlog::Source::Host);
        assert!(row.message().contains(api::hostlog::REDACTED));
        assert!(!row.message().contains(secret));
    }

    #[test]
    fn overflow_writes_an_incomplete_settings_sidecar_marker_once() {
        let dir = crate::test_support::TestDir::new("record-settings-incomplete");
        let path = dir.join(RECORD_SETTINGS_NAME);
        let file = File::create(&path).expect("settings sidecar");
        let (sender, _receiver) = sync_channel::<Vec<u8>>(RECORD_QUEUE);
        let mut record = HeadedRecord {
            config: HeadedRecordConfig {
                enabled: true,
                fps: DEFAULT_FPS,
            },
            pacer: RecordPacer::new(Duration::from_millis(500)),
            active: Some(ActiveRecording::for_test(sender, DEFAULT_FPS)),
            done: false,
        };
        record.active.as_mut().unwrap().settings_file = Some(BufWriter::new(file));

        record.note_settings_incomplete();
        record.note_settings_incomplete();

        assert_eq!(
            std::fs::read_to_string(path).expect("read settings sidecar"),
            format!("{SETTINGS_INCOMPLETE_MARKER}\n")
        );
    }
}
