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

use std::fmt::Write as _;
use std::fs::File;
use std::io::{BufWriter, Write as _};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{sync_channel, Receiver, SyncSender, TryRecvError};
use std::sync::{Arc, Mutex};
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
    /// Recording requested by the existing environment switch.
    pub enabled: bool,
    /// Captures per second (`HEADED_RECORD_FPS`, default 2).
    pub fps: f64,
    /// Optional caller-supplied readback deadline; production has no implicit
    /// readback timeout before slice 5 measures one.
    readback_timeout: Option<Duration>,
    /// Optional caller-supplied finalization deadline; no default is chosen
    /// before slice 5 measures one.
    finish_timeout: Option<Duration>,
}

impl HeadedRecordConfig {
    /// Read the switch and cadence from the process environment.
    pub fn from_env() -> Self {
        let (readback_timeout, finish_timeout) = if crate::window::render_fair_capture_enabled() {
            (
                fixture_timeout(
                    std::env::var("T44_CAPTURE_READBACK_TIMEOUT_MS")
                        .ok()
                        .as_deref(),
                ),
                fixture_timeout(
                    std::env::var("T44_RECORD_FINISH_TIMEOUT_MS")
                        .ok()
                        .as_deref(),
                ),
            )
        } else {
            (None, None)
        };
        Self {
            enabled: parse_enabled(std::env::var(RECORD_ENV).ok().as_deref()),
            fps: parse_fps(std::env::var(RECORD_FPS_ENV).ok().as_deref()),
            readback_timeout,
            finish_timeout,
        }
    }

    /// Explicit fixture deadline; production has no implicit timeout yet.
    #[cfg(any(test, feature = "render-test-hooks"))]
    pub fn with_finish_timeout(mut self, timeout: Duration) -> Self {
        self.finish_timeout = Some(timeout);
        self
    }

    /// Explicit fixture timeout; production has no implicit readback deadline.
    #[cfg(any(test, feature = "render-test-hooks"))]
    pub fn with_readback_timeout(mut self, timeout: Duration) -> Self {
        self.readback_timeout = Some(timeout);
        self
    }

    /// Compute a fixture-configured deadline without substituting a default.
    pub fn readback_deadline(&self, now: Instant) -> Option<Instant> {
        self.readback_timeout
            .and_then(|timeout| now.checked_add(timeout))
    }

    fn validate_fixture_policy(&self, now: Instant) -> Result<(), RecordError> {
        for timeout in [self.readback_timeout, self.finish_timeout] {
            let timeout = timeout.ok_or(RecordError::PolicyMissing)?;
            if timeout.is_zero() || now.checked_add(timeout).is_none() {
                return Err(RecordError::InvalidPolicy);
            }
        }
        Ok(())
    }

    /// Per-frame capture interval for the pacer.
    pub fn interval(&self) -> Duration {
        interval_for_fps(self.fps)
    }
}

fn fixture_timeout(raw: Option<&str>) -> Option<Duration> {
    raw?.trim()
        .parse::<u64>()
        .ok()
        .filter(|milliseconds| *milliseconds > 0)
        .map(Duration::from_millis)
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

/// The origin identity carried with a whole-window recording frame.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RecordingStamp {
    /// Sequence assigned to the composed window paint copied by the readback.
    Window { paint_sequence: u64 },
}

/// One owned pixel allocation accepted by the encoder.
#[derive(Debug)]
pub enum EncoderPixels {
    /// GPU/window readback already in RGBA8 order.
    GpuRgba(Vec<u8>),
    /// CPU-owned packed pixels converted to RGBA in place.
    CpuRgba(Vec<i32>),
}

impl EncoderPixels {
    /// Borrow the original pixel allocation as RGBA bytes.
    pub fn rgba_bytes(&self) -> &[u8] {
        match self {
            Self::GpuRgba(bytes) => bytes,
            Self::CpuRgba(pixels) => bytemuck::cast_slice(pixels),
        }
    }

    /// Convert a CPU pixel allocation in place to opaque RGBA byte order.
    pub fn cpu_rgba(mut pixels: Vec<i32>) -> Self {
        for pixel in &mut pixels {
            let packed = *pixel as u32;
            *pixel = i32::from_ne_bytes([
                (packed >> 16) as u8,
                (packed >> 8) as u8,
                packed as u8,
                u8::MAX,
            ]);
        }
        Self::CpuRgba(pixels)
    }
}

/// Bounded UTF-8 text held by a queued recording frame.
#[derive(Clone, Debug)]
pub struct BoundedText<const N: usize> {
    bytes: [u8; N],
    len: usize,
}

impl<const N: usize> Default for BoundedText<N> {
    fn default() -> Self {
        Self {
            bytes: [0; N],
            len: 0,
        }
    }
}

impl<const N: usize> BoundedText<N> {
    fn as_str(&self) -> &str {
        std::str::from_utf8(&self.bytes[..self.len])
            .expect("BoundedText only accepts complete UTF-8 strings")
    }
}

impl<const N: usize> std::fmt::Write for BoundedText<N> {
    fn write_str(&mut self, text: &str) -> std::fmt::Result {
        let end = self.len.checked_add(text.len()).ok_or(std::fmt::Error)?;
        let target = self.bytes.get_mut(self.len..end).ok_or(std::fmt::Error)?;
        target.copy_from_slice(text.as_bytes());
        self.len = end;
        Ok(())
    }
}

/// One whole-window frame ready for the bounded encoder queue.
pub struct RecordFrame {
    /// Capture width in pixels.
    pub width: u32,
    /// Capture height in pixels.
    pub height: u32,
    /// One owned raw allocation, moved into the encoder queue.
    pub pixels: EncoderPixels,
    /// Exact UI paint copied by this frame.
    pub stamp: RecordingStamp,
    /// Charged until the raw frame enters the bounded encoder queue or drops.
    readback_permit: Option<client::render::backend::ReadbackPermit>,
}

impl std::fmt::Debug for RecordFrame {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RecordFrame")
            .field("width", &self.width)
            .field("height", &self.height)
            .field("pixels", &self.pixels)
            .field("stamp", &self.stamp)
            .field("readback_permit_held", &self.readback_permit.is_some())
            .finish()
    }
}

impl RecordFrame {
    /// Build a whole-window frame from the completed readback allocation.
    pub fn window_rgba(
        width: u32,
        height: u32,
        rgba: Vec<u8>,
        paint_sequence: u64,
        readback_permit: Option<client::render::backend::ReadbackPermit>,
    ) -> Self {
        Self {
            width,
            height,
            pixels: EncoderPixels::GpuRgba(rgba),
            stamp: RecordingStamp::Window { paint_sequence },
            readback_permit,
        }
    }
}

struct EncoderFrame {
    width: u32,
    height: u32,
    pixels: EncoderPixels,
    sidecar_line: BoundedText<4096>,
    stamp: RecordingStamp,
}

/// Metadata refusal from the bounded recording/capture path.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RecordError {
    /// A name, output path, or sidecar line exceeds its explicit byte envelope.
    MetadataTooLarge,
    /// The raw frame geometry is outside the shared readback product envelope.
    Geometry,
    /// Another recording process still owns the recorder permit.
    RecorderBusy,
    /// No usable encoder binary is available.
    EncoderUnavailable,
    /// A recorder worker could not be created.
    WorkerStart,
    /// An experimental run supplied no capture or finalization policy.
    PolicyMissing,
    /// An explicit capture duration is zero or cannot form an Instant.
    InvalidPolicy,
}

impl std::fmt::Display for RecordError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::MetadataTooLarge => "recording metadata exceeds its byte limit",
            Self::Geometry => "recording frame dimensions exceed the supported limit",
            Self::RecorderBusy => "another recording is still active",
            Self::EncoderUnavailable => "the recording encoder is unavailable",
            Self::WorkerStart => "a recording worker could not start",
            Self::PolicyMissing => "the capture policy is missing",
            Self::InvalidPolicy => "the capture duration is invalid",
        })
    }
}

fn write_escaped_name<const N: usize>(output: &mut BoundedText<N>, name: &str) -> std::fmt::Result {
    for character in name.chars() {
        match character {
            '"' => output.write_str("\\\"")?,
            '\\' => output.write_str("\\\\")?,
            '\u{0}'..='\u{1f}' => write!(output, "\\u{:04x}", u32::from(character))?,
            other => output.write_char(other)?,
        }
    }
    Ok(())
}

fn bounded_sidecar_line(
    t_secs: f64,
    frame: u64,
    step: usize,
    total: usize,
    tick: u32,
    stamp: RecordingStamp,
    name: &str,
) -> Result<BoundedText<4096>, RecordError> {
    if name.len() > 256 {
        return Err(RecordError::MetadataTooLarge);
    }
    let mut line = BoundedText::default();
    let RecordingStamp::Window { paint_sequence } = stamp;
    write!(
        line,
        "t={t_secs:.3} frame={frame} step={step}/{total} tick={tick} \
         stamp=window:{paint_sequence} name=\""
    )
    .map_err(|_| RecordError::MetadataTooLarge)?;
    write_escaped_name(&mut line, name).map_err(|_| RecordError::MetadataTooLarge)?;
    line.write_char('"')
        .map_err(|_| RecordError::MetadataTooLarge)?;
    Ok(line)
}

fn bounded_sidecar_header(scenario: &str, fps: f64) -> Result<BoundedText<4096>, RecordError> {
    if scenario.len() > 256 {
        return Err(RecordError::MetadataTooLarge);
    }
    let mut header = BoundedText::default();
    header
        .write_str("# headed recording scenario=\"")
        .map_err(|_| RecordError::MetadataTooLarge)?;
    write_escaped_name(&mut header, scenario).map_err(|_| RecordError::MetadataTooLarge)?;
    write!(header, "\" fps={fps}\n# t=frame/fps frame=<n> step=<i>/<total> tick=<runner ticks> stamp=window:<paint sequence> name=\"<step>\"")
        .map_err(|_| RecordError::MetadataTooLarge)?;
    Ok(header)
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
    legacy_sidecar_line(t_secs, frame, step, total, tick, name)
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

fn bounded_settings_line(
    t_secs: f64,
    setting: &str,
    state: SettingState,
    error: &str,
) -> Result<BoundedText<4096>, RecordError> {
    if setting.len() > 256 || error.len() > 256 {
        return Err(RecordError::MetadataTooLarge);
    }
    let error = if state == SettingState::Failed {
        error
    } else {
        ""
    };
    let mut line = BoundedText::default();
    write!(line, "t={t_secs:.3} setting=\"").map_err(|_| RecordError::MetadataTooLarge)?;
    write_escaped_name(&mut line, setting).map_err(|_| RecordError::MetadataTooLarge)?;
    write!(line, "\" state={} error=\"", state.as_str())
        .map_err(|_| RecordError::MetadataTooLarge)?;
    write_escaped_name(&mut line, error).map_err(|_| RecordError::MetadataTooLarge)?;
    line.write_char('"')
        .map_err(|_| RecordError::MetadataTooLarge)?;
    Ok(line)
}

/// Keep legacy setting text parseable by replacing quotes and line breaks.
pub fn sanitize_settings_text(text: &str) -> String {
    text.replace('"', "'").replace(['\n', '\r'], " ")
}

/// One settings sidecar line with legacy-compatible escaped metadata.
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

fn bounded_settings_header(scenario: &str) -> Result<BoundedText<4096>, RecordError> {
    if scenario.len() > 256 {
        return Err(RecordError::MetadataTooLarge);
    }
    let mut header = BoundedText::default();
    header
        .write_str("# headed recording settings scenario=\"")
        .map_err(|_| RecordError::MetadataTooLarge)?;
    write_escaped_name(&mut header, scenario).map_err(|_| RecordError::MetadataTooLarge)?;
    header
        .write_str(
            "\"\n# t=<seconds since armed> setting=\"<setting>\" \
             state=pending|failed|cleared|saved error=\"<error or empty>\"",
        )
        .map_err(|_| RecordError::MetadataTooLarge)?;
    Ok(header)
}

/// Settings sidecar header: names the scenario, documents the columns.
pub fn format_settings_header(scenario: &str) -> String {
    format!(
        "# headed recording settings scenario={scenario}\n\
         # t=<seconds since armed> setting=\"<setting>\" \
         state=pending|failed|cleared|saved error=\"<error or empty>\""
    )
}

#[derive(Clone, Debug)]
struct SettingEntry {
    name: BoundedText<256>,
    state: SettingState,
    error: BoundedText<256>,
}

/// Bounded last-state table for the settings sidecar.
#[derive(Debug)]
pub struct SettingsTracker {
    last: [Option<SettingEntry>; RECORD_QUEUE],
    len: usize,
}

impl Default for SettingsTracker {
    fn default() -> Self {
        Self {
            last: std::array::from_fn(|_| None),
            len: 0,
        }
    }
}

impl SettingsTracker {
    /// The lines one reported transition writes. An owner-reported
    /// `Cleared` state is written before the next state. A bounded overflow
    /// is reported rather than growing the table with recording duration.
    pub fn lines_for(
        &mut self,
        t_secs: f64,
        setting: &str,
        state: SettingState,
        error: &str,
    ) -> Result<[Option<BoundedText<4096>>; 2], RecordError> {
        let error = if state == SettingState::Failed {
            error
        } else {
            ""
        };
        if setting.len() > 256 || error.len() > 256 {
            return Err(RecordError::MetadataTooLarge);
        }
        let slot = self.last[..self.len].iter().position(|entry| {
            entry
                .as_ref()
                .is_some_and(|entry| entry.name.as_str() == setting)
        });
        if let Some(index) = slot {
            let previous = self.last[index].as_ref().expect("matched slot");
            if previous.state == state && previous.error.as_str() == error {
                return Ok([None, None]);
            }
        } else if self.len == RECORD_QUEUE {
            return Err(RecordError::MetadataTooLarge);
        }
        let cleared = if state == SettingState::Cleared {
            Some(bounded_settings_line(
                t_secs,
                setting,
                SettingState::Cleared,
                "",
            )?)
        } else {
            None
        };
        let line = if state != SettingState::Cleared {
            Some(bounded_settings_line(t_secs, setting, state, error)?)
        } else {
            None
        };
        let mut name = BoundedText::default();
        name.write_str(setting)
            .map_err(|_| RecordError::MetadataTooLarge)?;
        let mut error_text = BoundedText::default();
        error_text
            .write_str(error)
            .map_err(|_| RecordError::MetadataTooLarge)?;
        let entry = SettingEntry {
            name,
            state,
            error: error_text,
        };
        if let Some(index) = slot {
            self.last[index] = Some(entry);
        } else {
            self.last[self.len] = Some(entry);
            self.len += 1;
        }
        Ok([cleared, line])
    }
}

/// One frame or settings transition in the shared bounded writer queue.
enum EncoderMessage {
    Frame(EncoderFrame),
    Setting(BoundedText<4096>),
}

enum EncoderControl {
    Finish { deadline: Option<Instant> },
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum WriterReady {
    Ready,
    Failed,
}

#[derive(Clone, Copy, Debug)]
struct WriterFrameInfo {
    width: u32,
    height: u32,
}

/// Nonblocking UI-visible state of the recorder owner.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RecordFinish {
    Idle,
    Finalizing,
    Complete,
    TimedOut,
    Failed,
}

enum RecordFailure {
    Io(std::io::Error),
    InvalidFrame,
    InvalidMetadata,
    WriterChannelClosed,
    WriterPanicked,
}

struct LegacyActiveRecording {
    sender: SyncSender<Vec<u8>>,
    pending_receiver: Option<Receiver<Vec<u8>>>,
    thread: Option<JoinHandle<LegacyEncoderOutcome>>,
    mp4_path: PathBuf,
    txt_path: PathBuf,
    fps: f64,
    width: u32,
    height: u32,
    lines: Vec<String>,
    started: Instant,
    settings_file: Option<BufWriter<File>>,
    settings: SettingsTracker,
    settings_incomplete: bool,
    queued: u64,
    dropped: u64,
    size_mismatch: u64,
}

impl LegacyActiveRecording {
    fn push(&mut self, bytes: Vec<u8>) -> bool {
        if self.sender.try_send(bytes).is_ok() {
            self.queued = self.queued.saturating_add(1);
            true
        } else {
            self.dropped = self.dropped.saturating_add(1);
            false
        }
    }
}

struct LegacyEncoderOutcome {
    frames_written: u64,
    error: Option<String>,
}

struct RecorderPermit {
    occupied: &'static AtomicBool,
}

static RECORDER_OCCUPIED: AtomicBool = AtomicBool::new(false);

impl RecorderPermit {
    fn try_acquire() -> Result<Self, RecordError> {
        RECORDER_OCCUPIED
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map(|_| Self {
                occupied: &RECORDER_OCCUPIED,
            })
            .map_err(|_| RecordError::RecorderBusy)
    }
}

impl Drop for RecorderPermit {
    fn drop(&mut self) {
        self.occupied.store(false, Ordering::Release);
    }
}

struct ActiveRecording {
    sender: Arc<Mutex<Option<SyncSender<EncoderMessage>>>>,
    control: SyncSender<EncoderControl>,
    supervisor: Option<JoinHandle<()>>,
    outcome: Arc<Mutex<RecordFinish>>,
    mp4_path: Arc<PathBuf>,
    fps: f64,
    width: u32,
    height: u32,
    started: Instant,
    settings: SettingsTracker,
    settings_incomplete: Arc<AtomicBool>,
    failure: Arc<AtomicBool>,
    finish_timeout: Option<Duration>,
    queued: u64,
    dropped: u64,
    size_mismatch: u64,
}

impl ActiveRecording {
    /// Queue one already-validated frame without waiting for the writer.
    fn push(&mut self, frame: EncoderFrame) -> bool {
        let sent = match self.sender.try_lock() {
            Ok(sender) => sender
                .as_ref()
                .is_some_and(|sender| sender.try_send(EncoderMessage::Frame(frame)).is_ok()),
            Err(_) => false,
        };
        if sent {
            self.queued = self.queued.saturating_add(1);
        } else {
            self.dropped = self.dropped.saturating_add(1);
        }
        sent
    }

    fn push_setting(&mut self, line: BoundedText<4096>) -> bool {
        if self.width == 0 {
            self.settings_incomplete.store(true, Ordering::Release);
            return false;
        }
        let sent = match self.sender.try_lock() {
            Ok(sender) => sender
                .as_ref()
                .is_some_and(|sender| sender.try_send(EncoderMessage::Setting(line)).is_ok()),
            Err(_) => false,
        };
        if !sent {
            self.settings_incomplete.store(true, Ordering::Release);
        }
        sent
    }

    #[cfg(test)]
    fn for_test(sender: SyncSender<EncoderMessage>, fps: f64) -> Self {
        let (control, control_receiver) = sync_channel(1);
        drop(control_receiver);
        Self {
            sender: Arc::new(Mutex::new(Some(sender))),
            control,
            supervisor: None,
            outcome: Arc::new(Mutex::new(RecordFinish::Idle)),
            mp4_path: Arc::new(PathBuf::from(RECORD_MP4_NAME)),
            fps,
            width: 64,
            height: 64,
            started: Instant::now(),
            settings: SettingsTracker::default(),
            settings_incomplete: Arc::new(AtomicBool::new(false)),
            failure: Arc::new(AtomicBool::new(false)),
            finish_timeout: None,
            queued: 0,
            dropped: 0,
            size_mismatch: 0,
        }
    }
}

struct FinalizingRecording {
    outcome: Arc<Mutex<RecordFinish>>,
    supervisor: Option<JoinHandle<()>>,
    mp4_path: Arc<PathBuf>,
    queued: u64,
    dropped: u64,
    size_mismatch: u64,
    reported: Option<RecordFinish>,
}

/// Resolve the ffmpeg binary: explicit `HEADED_RECORD_FFMPEG` override,
/// then the Homebrew path, then `PATH`.
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

fn render_fair_capture_enabled() -> bool {
    #[cfg(any(feature = "render-diagnostics", feature = "render-test-hooks"))]
    {
        frontend_core::surface::render_fair_enabled()
    }
    #[cfg(not(any(feature = "render-diagnostics", feature = "render-test-hooks")))]
    {
        false
    }
}

/// Opt-in headed recorder. Created per panel run when fair rendering is
/// explicitly enabled; Drop requests termination without joining.
pub struct HeadedRecord {
    config: HeadedRecordConfig,
    pacer: RecordPacer,
    gated: bool,
    active: Option<ActiveRecording>,
    legacy_active: Option<LegacyActiveRecording>,
    finalizing: Option<FinalizingRecording>,
    done: bool,
}

impl HeadedRecord {
    /// Read the switch/cadence from the environment. Explicitly enabled fair
    /// rendering selects the bounded asynchronous recording pipeline.
    pub fn from_env() -> Self {
        let config = HeadedRecordConfig::from_env();
        let interval = config.interval();
        Self {
            config,
            pacer: RecordPacer::new(interval),
            gated: render_fair_capture_enabled(),
            active: None,
            legacy_active: None,
            finalizing: None,
            done: false,
        }
    }

    /// Recording requested and neither armed nor finished.
    pub fn needs_arm(&self) -> bool {
        self.config.enabled
            && self.active.is_none()
            && self.legacy_active.is_none()
            && self.finalizing.is_none()
            && !self.done
    }

    /// True while accepting frames.
    pub fn is_active(&self) -> bool {
        self.active.is_some() || self.legacy_active.is_some()
    }

    /// Whether recording was requested and passed process activation.
    pub fn enabled(&self) -> bool {
        self.config.enabled
    }

    /// Deadline for a capture opportunity using configured policy.
    pub fn readback_deadline(&self, now: Instant) -> Option<Instant> {
        self.config.readback_deadline(now)
    }

    /// Keep a sleeping UI on the configured capture cadence. Finalization
    /// polls at that same cadence without waiting for the supervisor.
    /// Gate-off recording keeps its existing event-loop behavior.
    pub fn next_ui_deadline(&self, now: Instant) -> Option<Instant> {
        if !self.gated {
            None
        } else if self.is_active() {
            Some(self.pacer.next.unwrap_or(now))
        } else if self.is_finalizing() {
            now.checked_add(self.pacer.interval)
        } else {
            None
        }
    }

    /// Give up recording for this run without arming.
    pub fn abandon(&mut self) {
        self.config.enabled = false;
        self.done = true;
    }

    /// Queue one saved-setting transition without performing file I/O.
    pub fn note_setting(&mut self, setting: &str, state: SettingState, error: &str) {
        if self.gated {
            let Some(active) = self.active.as_mut() else {
                return;
            };
            let t_secs = active.started.elapsed().as_secs_f64();
            let lines = match active.settings.lines_for(t_secs, setting, state, error) {
                Ok(lines) => lines,
                Err(_) => {
                    active.settings_incomplete.store(true, Ordering::Release);
                    return;
                }
            };
            for line in lines.into_iter().flatten() {
                active.push_setting(line);
            }
            return;
        }
        let Some(active) = self.legacy_active.as_mut() else {
            return;
        };
        let t_secs = active.started.elapsed().as_secs_f64();
        let lines = match active.settings.lines_for(t_secs, setting, state, error) {
            Ok(lines) => lines,
            Err(_) => {
                active.settings_incomplete = true;
                return;
            }
        };
        let Some(writer) = active.settings_file.as_mut() else {
            return;
        };
        for line in lines.into_iter().flatten() {
            if writeln!(writer, "{}", line.as_str()).is_err() {
                active.settings_file = None;
                return;
            }
        }
        if writer.flush().is_err() {
            active.settings_file = None;
        }
    }

    /// True after a finish request has transferred ownership to the supervisor.
    pub fn is_finalizing(&self) -> bool {
        self.finalizing.is_some()
    }

    /// Count recorder opportunities refused before a readback was accepted.
    pub fn note_capture_drops(&mut self, count: u64) {
        if let Some(active) = self.active.as_mut() {
            active.dropped = active.dropped.saturating_add(count);
        }
    }

    pub fn note_capture_drop(&mut self) {
        self.note_capture_drops(1);
    }

    /// Mark the settings sidecar incomplete after an event queue overflow.
    pub fn note_settings_incomplete(&mut self) {
        if self.gated {
            if let Some(active) = self.active.as_ref() {
                active.settings_incomplete.store(true, Ordering::Release);
            }
            return;
        }
        let Some(active) = self.legacy_active.as_mut() else {
            return;
        };
        if active.settings_incomplete {
            return;
        }
        active.settings_incomplete = true;
        if active.settings_file.as_mut().is_some_and(|writer| {
            writeln!(writer, "{SETTINGS_INCOMPLETE_MARKER}")
                .and_then(|_| writer.flush())
                .is_err()
        }) {
            active.settings_file = None;
        }
    }

    /// Stage the writer/supervisor pair after validating bounded metadata.
    /// No encoder or file work is performed on the UI thread.
    pub fn arm(&mut self, shot_dir: &Path, scenario: &str) -> Result<(), RecordError> {
        if !self.needs_arm() {
            return Ok(());
        }
        if !self.gated {
            self.arm_legacy(shot_dir, scenario);
            return Ok(());
        }
        let result = self.arm_inner(shot_dir, scenario);
        if result.is_err() {
            self.config.enabled = false;
            self.done = true;
        }
        result
    }

    fn arm_legacy(&mut self, shot_dir: &Path, scenario: &str) {
        let mp4_path = shot_dir.join(RECORD_MP4_NAME);
        let txt_path = shot_dir.join(RECORD_SIDECAR_NAME);
        let fps = self.config.fps;
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
        let (sender, receiver) = sync_channel::<Vec<u8>>(RECORD_QUEUE);
        self.legacy_active = Some(LegacyActiveRecording {
            sender,
            pending_receiver: Some(receiver),
            thread: None,
            mp4_path,
            txt_path,
            fps,
            width: 0,
            height: 0,
            lines: vec![format_sidecar_header(scenario, fps)],
            started: Instant::now(),
            settings_file,
            settings: SettingsTracker::default(),
            settings_incomplete: false,
            queued: 0,
            dropped: 0,
            size_mismatch: 0,
        });
    }

    fn arm_inner(&mut self, shot_dir: &Path, scenario: &str) -> Result<(), RecordError> {
        self.config.validate_fixture_policy(Instant::now())?;
        let fps = self.config.fps;
        let header = bounded_sidecar_header(scenario, self.config.fps)?;
        let settings_header = bounded_settings_header(scenario)?;
        let mp4_path = Arc::new(checked_child_path(shot_dir, RECORD_MP4_NAME)?);
        let sidecar_path = Arc::new(checked_child_path(shot_dir, RECORD_SIDECAR_NAME)?);
        let settings_path = Arc::new(checked_child_path(shot_dir, RECORD_SETTINGS_NAME)?);
        let binary_name = ffmpeg_candidate();
        let binary = which_ffmpeg(&binary_name).ok_or(RecordError::EncoderUnavailable)?;
        let permit = RecorderPermit::try_acquire()?;
        let (frame_sender, frame_receiver) = sync_channel::<EncoderMessage>(RECORD_QUEUE);
        let (control_sender, control_receiver) = sync_channel::<EncoderControl>(1);
        let sender = Arc::new(Mutex::new(Some(frame_sender)));
        let supervisor_sender = Arc::clone(&sender);
        let outcome = Arc::new(Mutex::new(RecordFinish::Idle));
        let supervisor_outcome = Arc::clone(&outcome);
        let settings_incomplete = Arc::new(AtomicBool::new(false));
        let supervisor_settings_incomplete = Arc::clone(&settings_incomplete);
        let failure = Arc::new(AtomicBool::new(false));
        let supervisor_failure = Arc::clone(&failure);
        let finish_timeout = self.config.finish_timeout;
        let task_mp4_path = Arc::clone(&mp4_path);
        let task_sidecar_path = sidecar_path;
        let task_settings_path = settings_path;
        let supervisor = thread::Builder::new()
            .name("headed-record-supervisor".to_string())
            .spawn(move || {
                run_supervisor(
                    binary,
                    task_mp4_path,
                    task_sidecar_path,
                    task_settings_path,
                    fps,
                    header,
                    settings_header,
                    frame_receiver,
                    control_receiver,
                    supervisor_sender,
                    supervisor_outcome,
                    supervisor_settings_incomplete,
                    supervisor_failure,
                    permit,
                )
            })
            .map_err(|_| RecordError::WorkerStart)?;
        log_diagnostic(
            api::hostlog::Level::Info,
            format_args!(
                "headed record: {scenario} -> {} @ {fps} fps",
                mp4_path.display()
            ),
        );
        self.active = Some(ActiveRecording {
            sender,
            control: control_sender,
            supervisor: Some(supervisor),
            outcome,
            mp4_path,
            fps,
            width: 0,
            height: 0,
            started: Instant::now(),
            settings: SettingsTracker::default(),
            settings_incomplete,
            failure,
            finish_timeout,
            queued: 0,
            dropped: 0,
            size_mismatch: 0,
        });
        Ok(())
    }

    /// True when a capture is due this frame (only while armed).
    pub fn is_due(&mut self, now: Instant) -> bool {
        self.is_active() && self.pacer.poll(now)
    }

    /// Move readback frames into the selected pipeline.
    pub fn pump_frames(
        &mut self,
        frames: Vec<RecordFrame>,
        step: &StepInfo,
    ) -> Result<(), RecordError> {
        if self.gated {
            self.pump_gated_frames(frames, step)
        } else {
            self.pump_legacy_frames(frames, step);
            Ok(())
        }
    }

    fn pump_gated_frames(
        &mut self,
        frames: Vec<RecordFrame>,
        step: &StepInfo,
    ) -> Result<(), RecordError> {
        let Some(active) = self.active.as_mut() else {
            return Ok(());
        };
        for frame in frames {
            let Some(expected_bytes) = checked_raw_frame_bytes(frame.width, frame.height) else {
                active.size_mismatch = active.size_mismatch.saturating_add(1);
                continue;
            };
            if frame.pixels.rgba_bytes().len() != expected_bytes {
                active.size_mismatch = active.size_mismatch.saturating_add(1);
                continue;
            }
            if active.width == 0 {
                active.width = frame.width;
                active.height = frame.height;
            }
            if frame.width != active.width || frame.height != active.height {
                active.size_mismatch = active.size_mismatch.saturating_add(1);
                continue;
            }
            let frame_index = active.queued;
            let sidecar_line = match bounded_sidecar_line(
                frame_index as f64 / active.fps,
                frame_index,
                step.step,
                step.total,
                step.tick,
                frame.stamp,
                &step.name,
            ) {
                Ok(line) => line,
                Err(error) => {
                    active.failure.store(true, Ordering::Release);
                    return Err(error);
                }
            };
            active.push(EncoderFrame {
                width: frame.width,
                height: frame.height,
                pixels: frame.pixels,
                sidecar_line,
                stamp: frame.stamp,
            });
            // Only now has this raw image transferred to one of the eight
            // queue slots (or been destroyed by explicit queue refusal).
            drop(frame.readback_permit);
        }
        Ok(())
    }

    fn pump_legacy_frames(&mut self, frames: Vec<RecordFrame>, step: &StepInfo) {
        let mut spawn_failed = false;
        if let Some(active) = self.legacy_active.as_mut() {
            for frame in frames {
                if active.width == 0 {
                    active.width = frame.width;
                    active.height = frame.height;
                    let receiver = active.pending_receiver.take();
                    spawn_legacy_encoder(active, receiver);
                    if active.thread.is_none() {
                        spawn_failed = true;
                        break;
                    }
                }
                if frame.width != active.width || frame.height != active.height {
                    active.size_mismatch = active.size_mismatch.saturating_add(1);
                    continue;
                }
                let expected = 4 * active.width as usize * active.height as usize;
                let bytes = match frame.pixels {
                    EncoderPixels::GpuRgba(bytes) => bytes,
                    EncoderPixels::CpuRgba(_) => {
                        active.size_mismatch = active.size_mismatch.saturating_add(1);
                        continue;
                    }
                };
                if bytes.len() != expected {
                    active.size_mismatch = active.size_mismatch.saturating_add(1);
                    continue;
                }
                let frame_index = active.queued;
                let t_secs = frame_index as f64 / active.fps;
                if active.push(bytes) {
                    active.lines.push(legacy_sidecar_line(
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
        if spawn_failed {
            self.config.enabled = false;
            self.legacy_active = None;
        }
    }

    /// Request finalization without waiting on a writer, process, or disk.
    pub fn request_finish(&mut self, now: Instant) {
        if !self.gated {
            if self.done {
                return;
            }
            self.done = true;
            self.finish_legacy();
            return;
        }
        if self.done {
            return;
        }
        self.done = true;
        let Some(mut active) = self.active.take() else {
            return;
        };
        // A missing/unrepresentable policy is an immediate closed failure,
        // never an unbounded supervisor lifetime or an inferred timeout.
        let deadline = Some(
            active
                .finish_timeout
                .and_then(|timeout| now.checked_add(timeout))
                .unwrap_or(now),
        );
        {
            let mut outcome = active
                .outcome
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            if *outcome == RecordFinish::Idle {
                *outcome = RecordFinish::Finalizing;
            }
        }
        let _ = active.control.try_send(EncoderControl::Finish { deadline });
        self.finalizing = Some(FinalizingRecording {
            outcome: active.outcome,
            supervisor: active.supervisor.take(),
            mp4_path: active.mp4_path,
            queued: active.queued,
            dropped: active.dropped,
            size_mismatch: active.size_mismatch,
            reported: None,
        });
    }

    fn finish_legacy(&mut self) {
        let Some(mut active) = self.legacy_active.take() else {
            return;
        };
        if let Some(writer) = active.settings_file.as_mut() {
            let _ = writer.flush();
        }
        let Some(thread) = active.thread.take() else {
            return;
        };
        drop(active.pending_receiver.take());
        drop(active.sender);
        let outcome = thread.join().unwrap_or_else(|_| LegacyEncoderOutcome {
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

    /// Legacy compatibility: the gate-off pipeline keeps synchronous
    /// finalization; the fair-render pipeline only requests supervisor shutdown.
    pub fn finish(&mut self) {
        self.request_finish(Instant::now());
    }

    /// Read completion state and join only after the supervisor has exited.
    pub fn poll_finish(&mut self, _now: Instant) -> RecordFinish {
        let Some(finalizing) = self.finalizing.as_mut() else {
            return if !self.gated && self.done {
                RecordFinish::Complete
            } else {
                RecordFinish::Idle
            };
        };
        let mut status = *finalizing
            .outcome
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let worker_finished = finalizing
            .supervisor
            .as_ref()
            .is_none_or(JoinHandle::is_finished);
        if !worker_finished && status != RecordFinish::TimedOut {
            status = RecordFinish::Finalizing;
        }
        if matches!(
            status,
            RecordFinish::Complete | RecordFinish::TimedOut | RecordFinish::Failed
        ) && finalizing.reported != Some(status)
        {
            let level = if status == RecordFinish::Complete {
                api::hostlog::Level::Info
            } else {
                api::hostlog::Level::Error
            };
            log_diagnostic(
                level,
                format_args!(
                    "headed record: {} ({status:?}, {} queued, {} dropped, {} resized)",
                    finalizing.mp4_path.display(),
                    finalizing.queued,
                    finalizing.dropped,
                    finalizing.size_mismatch
                ),
            );
            finalizing.reported = Some(status);
        }
        if worker_finished {
            if let Some(supervisor) = finalizing.supervisor.take() {
                if supervisor.join().is_err() && status != RecordFinish::TimedOut {
                    status = RecordFinish::Failed;
                    *finalizing
                        .outcome
                        .lock()
                        .unwrap_or_else(|error| error.into_inner()) = status;
                }
            }
            self.finalizing = None;
        }
        status
    }
}

impl Drop for HeadedRecord {
    fn drop(&mut self) {
        self.request_finish(Instant::now());
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

fn legacy_sidecar_line(
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

/// Preserve the original gate-off sidecar escaping behavior.
pub fn sanitize_record_name(name: &str) -> String {
    name.replace('"', "'")
}

fn spawn_legacy_encoder(active: &mut LegacyActiveRecording, receiver: Option<Receiver<Vec<u8>>>) {
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
    let mut child = match Command::new(&binary)
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
    let stdin = match child.stdin.take() {
        Some(stdin) => stdin,
        None => {
            log_diagnostic(
                api::hostlog::Level::Error,
                "headed record: ffmpeg stdin unavailable; stopping",
            );
            return;
        }
    };
    active.thread = Some(thread::spawn(move || {
        run_legacy_encoder(stdin, receiver, child)
    }));
}

fn run_legacy_encoder(
    mut stdin: ChildStdin,
    receiver: Receiver<Vec<u8>>,
    mut child: Child,
) -> LegacyEncoderOutcome {
    let mut frames_written = 0u64;
    let mut error = None;
    while let Ok(bytes) = receiver.recv() {
        if let Err(io) = stdin.write_all(&bytes) {
            error = Some(format!("stdin write: {io}"));
            break;
        }
        frames_written = frames_written.saturating_add(1);
    }
    drop(stdin);
    match child.wait() {
        Ok(status) if status.success() => {}
        Ok(status) => error = error.or(Some(format!("ffmpeg exit {status}"))),
        Err(io) => error = Some(format!("ffmpeg wait: {io}")),
    }
    LegacyEncoderOutcome {
        frames_written,
        error,
    }
}

fn checked_child_path(directory: &Path, file: &str) -> Result<PathBuf, RecordError> {
    frontend_core::live::record_output_path(directory, file)
        .map_err(|_| RecordError::MetadataTooLarge)
}

fn checked_raw_frame_bytes(width: u32, height: u32) -> Option<usize> {
    let width = usize::try_from(width).ok()?;
    let height = usize::try_from(height).ok()?;
    let pixels = width.checked_mul(height)?;
    if pixels == 0 || pixels > client::render::backend::RECORD_MAX_RAW_PIXELS {
        return None;
    }
    let raw_bytes = pixels.checked_mul(4)?;
    if raw_bytes > client::render::backend::READBACK_MAX_STAGE_BYTES {
        return None;
    }
    let raw_row = width.checked_mul(4)?;
    let alignment = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT as usize;
    let padded_row = raw_row.checked_add(alignment - 1)?.checked_div(alignment)? * alignment;
    let stage_bytes = padded_row.checked_mul(height)?;
    (stage_bytes <= client::render::backend::READBACK_MAX_STAGE_BYTES).then_some(raw_bytes)
}

fn set_finish_status(outcome: &Mutex<RecordFinish>, status: RecordFinish) {
    *outcome.lock().unwrap_or_else(|error| error.into_inner()) = status;
}

#[cfg(feature = "render-diagnostics")]
fn record_finish_completion(disconnected_at: Option<Instant>) {
    if let Some(disconnected_at) = disconnected_at {
        log_diagnostic(
            api::hostlog::Level::Info,
            format_args!(
                "render timing: record_disconnect_to_completion_ns={}",
                disconnected_at.elapsed().as_nanos()
            ),
        );
    }
}

fn close_frame_sender(sender: &Mutex<Option<SyncSender<EncoderMessage>>>) {
    sender
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .take();
}

fn spawn_ffmpeg(
    binary: &Path,
    output: &Path,
    fps: f64,
    width: u32,
    height: u32,
) -> std::io::Result<Child> {
    let size = format!("{width}x{height}");
    let fps = format!("{fps}");
    let scale = format!("scale='min({},iw)':-2", RECORD_MAX_WIDTH);
    Command::new(binary)
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
        .arg(output)
        .stdin(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
}

fn write_encoder_frame(
    stdin: &mut ChildStdin,
    sidecar: &mut File,
    frame: EncoderFrame,
) -> Result<(), RecordFailure> {
    let expected =
        checked_raw_frame_bytes(frame.width, frame.height).ok_or(RecordFailure::InvalidFrame)?;
    let pixels = frame.pixels.rgba_bytes();
    if pixels.len() != expected {
        return Err(RecordFailure::InvalidFrame);
    }
    let RecordingStamp::Window { paint_sequence } = frame.stamp;
    let mut stamp = BoundedText::<32>::default();
    write!(stamp, "stamp=window:{paint_sequence}").map_err(|_| RecordFailure::InvalidMetadata)?;
    if !frame.sidecar_line.as_str().contains(stamp.as_str()) {
        return Err(RecordFailure::InvalidMetadata);
    }
    stdin.write_all(pixels).map_err(RecordFailure::Io)?;
    sidecar
        .write_all(frame.sidecar_line.as_str().as_bytes())
        .and_then(|_| sidecar.write_all(b"\n"))
        .map_err(RecordFailure::Io)
}

#[expect(
    clippy::too_many_arguments,
    reason = "The writer receives independently owned channels and bounded sidecar metadata"
)]
fn run_writer(
    stdin_receiver: Receiver<ChildStdin>,
    receiver: Receiver<EncoderMessage>,
    first_frame_ready: SyncSender<WriterFrameInfo>,
    ready: SyncSender<WriterReady>,
    sidecar_path: &Path,
    settings_path: &Path,
    header: BoundedText<4096>,
    settings_header: BoundedText<4096>,
    settings_incomplete: &AtomicBool,
) -> Result<EncoderOutcome, RecordFailure> {
    let files = (|| {
        if let Some(parent) = sidecar_path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            std::fs::create_dir_all(parent).map_err(RecordFailure::Io)?;
        }
        let mut sidecar = File::create(sidecar_path).map_err(RecordFailure::Io)?;
        let mut settings = File::create(settings_path).map_err(RecordFailure::Io)?;
        sidecar
            .write_all(header.as_str().as_bytes())
            .and_then(|_| sidecar.write_all(b"\n"))
            .map_err(RecordFailure::Io)?;
        settings
            .write_all(settings_header.as_str().as_bytes())
            .and_then(|_| settings.write_all(b"\n"))
            .map_err(RecordFailure::Io)?;
        Ok::<_, RecordFailure>((sidecar, settings))
    })();
    let (mut sidecar, mut settings) = match files {
        Ok(files) => {
            if ready.send(WriterReady::Ready).is_err() {
                return Err(RecordFailure::WriterChannelClosed);
            }
            files
        }
        Err(error) => {
            let _ = ready.send(WriterReady::Failed);
            return Err(error);
        }
    };

    let mut stdin = None;
    let mut frames_written = 0u64;
    while let Ok(message) = receiver.recv() {
        match message {
            EncoderMessage::Frame(frame) => {
                if stdin.is_none() {
                    let info = WriterFrameInfo {
                        width: frame.width,
                        height: frame.height,
                    };
                    first_frame_ready
                        .send(info)
                        .map_err(|_| RecordFailure::WriterChannelClosed)?;
                    let Ok(input) = stdin_receiver.recv() else {
                        break;
                    };
                    stdin = Some(input);
                }
                write_encoder_frame(
                    stdin
                        .as_mut()
                        .expect("first frame admits the encoder stdin"),
                    &mut sidecar,
                    frame,
                )?;
                frames_written = frames_written.saturating_add(1);
            }
            EncoderMessage::Setting(line) => {
                settings
                    .write_all(line.as_str().as_bytes())
                    .and_then(|_| settings.write_all(b"\n"))
                    .map_err(RecordFailure::Io)?;
            }
        }
    }
    if settings_incomplete.load(Ordering::Acquire) {
        settings
            .write_all(SETTINGS_INCOMPLETE_MARKER.as_bytes())
            .and_then(|_| settings.write_all(b"\n"))
            .map_err(RecordFailure::Io)?;
    }
    sidecar.flush().map_err(RecordFailure::Io)?;
    settings.flush().map_err(RecordFailure::Io)?;
    drop(stdin);
    Ok(EncoderOutcome { frames_written })
}

#[derive(Debug)]
struct EncoderOutcome {
    frames_written: u64,
}

#[expect(
    clippy::too_many_arguments,
    reason = "The supervisor owns the recorder channels, shared outcomes, metadata and admission permit"
)]
fn run_supervisor(
    binary: PathBuf,
    mp4_path: Arc<PathBuf>,
    sidecar_path: Arc<PathBuf>,
    settings_path: Arc<PathBuf>,
    fps: f64,
    header: BoundedText<4096>,
    settings_header: BoundedText<4096>,
    receiver: Receiver<EncoderMessage>,
    control: Receiver<EncoderControl>,
    sender: Arc<Mutex<Option<SyncSender<EncoderMessage>>>>,
    outcome: Arc<Mutex<RecordFinish>>,
    settings_incomplete: Arc<AtomicBool>,
    forced_failure: Arc<AtomicBool>,
    _permit: RecorderPermit,
) {
    let (ready_sender, ready_receiver) = sync_channel(1);
    let (first_frame_sender, first_frame_receiver) = sync_channel(1);
    let (stdin_sender, stdin_receiver) = sync_channel(1);
    let writer_sidecar_path = Arc::clone(&sidecar_path);
    let writer_settings_path = Arc::clone(&settings_path);
    let writer_settings_incomplete = Arc::clone(&settings_incomplete);
    let writer = match thread::Builder::new()
        .name("headed-record-writer".to_string())
        .spawn(move || {
            run_writer(
                stdin_receiver,
                receiver,
                first_frame_sender,
                ready_sender,
                &writer_sidecar_path,
                &writer_settings_path,
                header,
                settings_header,
                &writer_settings_incomplete,
            )
        }) {
        Ok(writer) => writer,
        Err(error) => {
            close_frame_sender(&sender);
            log_diagnostic(
                api::hostlog::Level::Error,
                format_args!("headed record: writer spawn failed: {error}"),
            );
            set_finish_status(&outcome, RecordFinish::Failed);
            return;
        }
    };
    let mut writer = Some(writer);

    let mut stdin_sender = Some(stdin_sender);
    let mut writer_ready = false;
    let mut frame_info = None;
    let mut deadline = None;
    let mut timed_out = false;
    let mut failed = false;
    let mut finish_received = false;
    let mut child: Option<Child> = None;
    let mut child_status: Option<ExitStatus> = None;
    let mut writer_result = None;
    #[cfg(feature = "render-diagnostics")]
    let mut disconnected_at = None;

    loop {
        if !finish_received {
            match control.try_recv() {
                Ok(EncoderControl::Finish {
                    deadline: finish_deadline,
                }) => {
                    deadline = finish_deadline;
                    finish_received = true;
                    // Include disconnect work and supervisor poll granularity.
                    #[cfg(feature = "render-diagnostics")]
                    {
                        disconnected_at = Some(Instant::now());
                    }
                    close_frame_sender(&sender);
                }
                Err(TryRecvError::Disconnected) => {
                    finish_received = true;
                    #[cfg(feature = "render-diagnostics")]
                    {
                        disconnected_at = Some(Instant::now());
                    }
                    close_frame_sender(&sender);
                }
                Err(TryRecvError::Empty) => {}
            }
        }
        if !timed_out && deadline.is_some_and(|deadline| Instant::now() >= deadline) {
            timed_out = true;
            set_finish_status(&outcome, RecordFinish::TimedOut);
            close_frame_sender(&sender);
            stdin_sender.take();
            if let Some(child) = child.as_mut() {
                let _ = child.kill();
            }
        }

        if !writer_ready {
            match ready_receiver.try_recv() {
                Ok(WriterReady::Ready) => writer_ready = true,
                Ok(WriterReady::Failed) => {
                    failed = true;
                    close_frame_sender(&sender);
                    stdin_sender.take();
                    if !timed_out {
                        set_finish_status(&outcome, RecordFinish::Failed);
                    }
                }
                Err(TryRecvError::Disconnected)
                    if writer.as_ref().is_none_or(JoinHandle::is_finished) =>
                {
                    failed = true;
                    close_frame_sender(&sender);
                    stdin_sender.take();
                    if !timed_out {
                        set_finish_status(&outcome, RecordFinish::Failed);
                    }
                }
                Err(TryRecvError::Disconnected | TryRecvError::Empty) => {}
            }
        }
        if frame_info.is_none() {
            if let Ok(info) = first_frame_receiver.try_recv() {
                frame_info = Some(info);
            }
        }
        if writer_ready && frame_info.is_some() && child.is_none() && !timed_out && !failed {
            let info = frame_info.take().expect("checked first-frame metadata");
            match spawn_ffmpeg(&binary, &mp4_path, fps, info.width, info.height) {
                Ok(mut spawned_child) => {
                    let Some(stdin) = spawned_child.stdin.take() else {
                        failed = true;
                        close_frame_sender(&sender);
                        stdin_sender.take();
                        let _ = spawned_child.kill();
                        child = Some(spawned_child);
                        if !timed_out {
                            set_finish_status(&outcome, RecordFinish::Failed);
                        }
                        continue;
                    };
                    let sent = stdin_sender
                        .take()
                        .is_some_and(|sender| sender.send(stdin).is_ok());
                    if !sent {
                        failed = true;
                        close_frame_sender(&sender);
                        let _ = spawned_child.kill();
                        if !timed_out {
                            set_finish_status(&outcome, RecordFinish::Failed);
                        }
                    }
                    child = Some(spawned_child);
                }
                Err(error) => {
                    log_diagnostic(
                        api::hostlog::Level::Error,
                        format_args!("headed record: ffmpeg spawn failed: {error}"),
                    );
                    failed = true;
                    close_frame_sender(&sender);
                    stdin_sender.take();
                    if !timed_out {
                        set_finish_status(&outcome, RecordFinish::Failed);
                    }
                }
            }
        }

        if let Some(child) = child.as_mut() {
            if child_status.is_none() {
                match child.try_wait() {
                    Ok(status) => child_status = status,
                    Err(error) => {
                        if !failed && !timed_out {
                            log_diagnostic(
                                api::hostlog::Level::Error,
                                format_args!("headed record: ffmpeg status failed: {error}"),
                            );
                            set_finish_status(&outcome, RecordFinish::Failed);
                        }
                        failed = true;
                        close_frame_sender(&sender);
                        let _ = child.kill();
                    }
                }
            }
        }

        if let Some(writer) = writer.take_if(|writer| writer.is_finished()) {
            writer_result = Some(match writer.join() {
                Ok(result) => result,
                Err(_) => Err(RecordFailure::WriterPanicked),
            });
        }
        if let Some(Err(error)) = writer_result.as_ref() {
            if !timed_out {
                log_diagnostic(
                    api::hostlog::Level::Error,
                    format_args!(
                        "headed record: writer failed: {}",
                        display_record_failure(error)
                    ),
                );
                if !failed {
                    set_finish_status(&outcome, RecordFinish::Failed);
                }
            }
            failed = true;
            close_frame_sender(&sender);
            stdin_sender.take();
            if let Some(child) = child.as_mut() {
                let _ = child.kill();
            }
        }
        if let Some(status) = child_status {
            if !status.success() && !failed && !timed_out {
                set_finish_status(&outcome, RecordFinish::Failed);
                failed = true;
                close_frame_sender(&sender);
            }
        }

        if let Some(writer_result) = writer_result.as_ref() {
            if child.is_none() {
                if !timed_out {
                    let completed =
                        writer_result.is_ok() && !failed && !forced_failure.load(Ordering::Acquire);
                    #[cfg(feature = "render-diagnostics")]
                    if completed {
                        record_finish_completion(disconnected_at);
                    }
                    set_finish_status(
                        &outcome,
                        if completed {
                            RecordFinish::Complete
                        } else {
                            RecordFinish::Failed
                        },
                    );
                }
                return;
            }
            if child_status.is_some() {
                if !timed_out {
                    let completed = child_status.is_some_and(|status| status.success())
                        && writer_result.is_ok()
                        && !failed
                        && !forced_failure.load(Ordering::Acquire);
                    #[cfg(feature = "render-diagnostics")]
                    if completed {
                        record_finish_completion(disconnected_at);
                    }
                    set_finish_status(
                        &outcome,
                        if completed {
                            RecordFinish::Complete
                        } else {
                            RecordFinish::Failed
                        },
                    );
                }
                if let Ok(encoder) = writer_result {
                    log_diagnostic(
                        api::hostlog::Level::Info,
                        format_args!(
                            "headed record: encoder wrote {} frames",
                            encoder.frames_written
                        ),
                    );
                }
                return;
            }
        }
        thread::park_timeout(Duration::from_millis(10));
    }
}

fn display_record_failure(error: &RecordFailure) -> String {
    match error {
        RecordFailure::Io(error) => error.to_string(),
        RecordFailure::InvalidFrame => "invalid frame geometry".to_string(),
        RecordFailure::InvalidMetadata => "invalid sidecar stamp".to_string(),
        RecordFailure::WriterChannelClosed => "recorder writer channel closed".to_string(),
        RecordFailure::WriterPanicked => "recorder writer panicked".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(all(feature = "render-test-hooks", unix))]
    fn bounded_recorder_fixed_layout_receipt() {
        use std::mem::size_of;
        let line_payload = 13 * 4096;
        let tracker_payload = RECORD_QUEUE * 2 * 256;
        let path_payload = 3 * 4096;
        println!(
            "bounded_recorder_fixed_layout_receipt={}",
            serde_json::json!({
                "target_arch": std::env::consts::ARCH,
                "target_os": std::env::consts::OS,
                "pointer_bytes": size_of::<usize>(),
                "debug_assertions": cfg!(debug_assertions),
                "layouts": {
                    "bounded_text_4096": size_of::<BoundedText<4096>>(),
                    "bounded_text_256": size_of::<BoundedText<256>>(),
                    "encoder_pixels": size_of::<EncoderPixels>(),
                    "record_frame": size_of::<RecordFrame>(),
                    "recording_stamp": size_of::<RecordingStamp>(),
                    "encoder_frame": size_of::<EncoderFrame>(),
                    "encoder_message": size_of::<EncoderMessage>(),
                    "encoder_control": size_of::<EncoderControl>(),
                    "writer_ready": size_of::<WriterReady>(),
                    "writer_frame_info": size_of::<WriterFrameInfo>(),
                    "setting_entry": size_of::<SettingEntry>(),
                    "optional_setting_entry": size_of::<Option<SettingEntry>>(),
                    "settings_tracker": size_of::<SettingsTracker>(),
                    "settings_producer_pair": size_of::<[Option<BoundedText<4096>>; 2]>(),
                    "active_recording": size_of::<ActiveRecording>(),
                    "finalizing_recording": size_of::<FinalizingRecording>(),
                    "headed_record": size_of::<HeadedRecord>(),
                    "recorder_permit": size_of::<RecorderPermit>(),
                    "record_finish": size_of::<RecordFinish>(),
                    "record_failure": size_of::<RecordFailure>(),
                    "frame_sender": size_of::<SyncSender<EncoderMessage>>(),
                    "frame_receiver": size_of::<Receiver<EncoderMessage>>(),
                    "shared_frame_sender_state": size_of::<Mutex<Option<SyncSender<EncoderMessage>>>>(),
                    "shared_finish_state": size_of::<Mutex<RecordFinish>>(),
                    "shared_path_handle": size_of::<Arc<PathBuf>>(),
                    "shared_path_allocation_without_payload": 2 * size_of::<usize>() + size_of::<PathBuf>(),
                },
                "recorder_text_and_path_payload_upper": {
                    "line_and_header_buffers": 13,
                    "line_and_header_bytes": line_payload,
                    "settings_tracker_text_bytes": tracker_payload,
                    "distinct_path_buffers": 3,
                    "path_bytes": path_payload,
                    "total_bytes": line_payload + tracker_payload + path_payload,
                },
                "scope": "Recorder payload only. Fixed envelope layouts include their inline text; do not add that payload twice. Still-shot JSON, labels and paths use the separate explicit capture metadata allowance. Pixel allocations, channel internals, thread resources and allocator overhead are separate domains."
            })
        );
    }

    fn encoder_frame(fill: u8, paint_sequence: u64) -> EncoderFrame {
        let stamp = RecordingStamp::Window { paint_sequence };
        EncoderFrame {
            width: 64,
            height: 64,
            pixels: EncoderPixels::GpuRgba(vec![fill; 64 * 64 * 4]),
            sidecar_line: bounded_sidecar_line(0.0, 0, 0, 1, 0, stamp, "test").unwrap(),
            stamp,
        }
    }

    fn settings_lines(
        tracker: &mut SettingsTracker,
        t_secs: f64,
        setting: &str,
        state: SettingState,
        error: &str,
    ) -> [Option<String>; 2] {
        tracker
            .lines_for(t_secs, setting, state, error)
            .unwrap()
            .map(|line| line.map(|line| line.as_str().to_owned()))
    }

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
    fn fixture_readback_deadline_is_explicit_and_overflow_checked() {
        let now = Instant::now();
        let unset = HeadedRecordConfig {
            enabled: true,
            fps: DEFAULT_FPS,
            readback_timeout: None,
            finish_timeout: None,
        };
        assert_eq!(unset.readback_deadline(now), None);
        assert_eq!(
            unset
                .clone()
                .with_readback_timeout(Duration::from_secs(3))
                .readback_deadline(now),
            Some(now + Duration::from_secs(3))
        );
        assert_eq!(
            unset
                .with_readback_timeout(Duration::MAX)
                .readback_deadline(now),
            None
        );
    }

    #[test]
    fn fixture_record_policy_requires_both_finite_positive_deadlines() {
        let now = Instant::now();
        assert_eq!(fixture_timeout(None), None);
        assert_eq!(fixture_timeout(Some("0")), None);
        assert_eq!(fixture_timeout(Some("bad")), None);
        assert_eq!(
            fixture_timeout(Some(" 17 ")),
            Some(Duration::from_millis(17))
        );
        let mut config = HeadedRecordConfig {
            enabled: true,
            fps: DEFAULT_FPS,
            readback_timeout: None,
            finish_timeout: None,
        };
        assert_eq!(
            config.validate_fixture_policy(now),
            Err(RecordError::PolicyMissing)
        );
        config.readback_timeout = Some(Duration::from_secs(2));
        assert_eq!(
            config.validate_fixture_policy(now),
            Err(RecordError::PolicyMissing)
        );
        config.finish_timeout = Some(Duration::MAX);
        assert_eq!(
            config.validate_fixture_policy(now),
            Err(RecordError::InvalidPolicy)
        );
        config.finish_timeout = Some(Duration::ZERO);
        assert_eq!(
            config.validate_fixture_policy(now),
            Err(RecordError::InvalidPolicy)
        );
        config.finish_timeout = Some(Duration::from_secs(3));
        assert_eq!(config.validate_fixture_policy(now), Ok(()));
    }

    #[test]
    fn cpu_recording_conversion_keeps_one_owned_pixel_allocation() {
        let pixels = vec![0x0022_3344, 0x00ab_cdef];
        let capacity = pixels.capacity();
        let pointer = pixels.as_ptr();
        let converted = EncoderPixels::cpu_rgba(pixels);
        assert_eq!(
            converted.rgba_bytes(),
            [0x22, 0x33, 0x44, 255, 0xab, 0xcd, 0xef, 255]
        );
        let EncoderPixels::CpuRgba(pixels) = converted else {
            panic!("CPU conversion must retain the original allocation");
        };
        assert_eq!(pixels.as_ptr(), pointer);
        assert_eq!(pixels.capacity(), capacity);
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
    fn gated_recording_deadline_tracks_pacer_and_finalization() {
        let (sender, _receiver) = sync_channel::<EncoderMessage>(RECORD_QUEUE);
        let mut record = HeadedRecord {
            config: HeadedRecordConfig {
                enabled: true,
                fps: 2.0,
                readback_timeout: None,
                finish_timeout: None,
            },
            pacer: RecordPacer::new(Duration::from_millis(500)),
            gated: false,
            active: Some(ActiveRecording::for_test(sender, 2.0)),
            legacy_active: None,
            finalizing: None,
            done: false,
        };
        let now = Instant::now();
        assert_eq!(record.next_ui_deadline(now), None);
        record.gated = true;
        assert_eq!(record.next_ui_deadline(now), Some(now));
        assert!(record.is_due(now));
        let due = now + Duration::from_millis(500);
        assert_eq!(record.next_ui_deadline(now), Some(due));
        assert!(!record.is_due(now + Duration::from_millis(1)));
        assert_eq!(record.next_ui_deadline(now), Some(due));
        let late = now + Duration::from_secs(10);
        assert!(record.is_due(late));
        assert_eq!(
            record.next_ui_deadline(late),
            Some(late + Duration::from_millis(500))
        );
        record.request_finish(late);
        assert!(!record.is_active());
        assert!(record.is_finalizing());
        assert_eq!(
            record.next_ui_deadline(late),
            Some(late + Duration::from_millis(500))
        );
        record.finalizing = None;
        assert_eq!(record.next_ui_deadline(late), None);
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
        let (sender, receiver) = sync_channel::<EncoderMessage>(1);
        let mut active = ActiveRecording::for_test(sender, 2.0);
        assert!(active.push(encoder_frame(1, 1)));
        assert!(!active.push(encoder_frame(2, 2)));
        assert!(!active.push(encoder_frame(3, 3)));
        assert_eq!(active.queued, 1);
        assert_eq!(active.dropped, 2, "full queue drops, never blocks");
        let EncoderMessage::Frame(frame) = receiver.recv().unwrap() else {
            panic!("frame queue contains a frame");
        };
        assert_eq!(frame.pixels.rgba_bytes()[0], 1);
    }

    #[test]
    fn dead_encoder_counts_as_drops() {
        let (sender, receiver) = sync_channel::<EncoderMessage>(4);
        drop(receiver);
        let mut active = ActiveRecording::for_test(sender, 2.0);
        assert!(!active.push(encoder_frame(1, 1)));
        assert_eq!(active.queued, 0);
        assert_eq!(active.dropped, 1);
    }

    /// Bounded recording preserves fixed geometry and drops malformed frames.
    #[test]
    fn pump_queues_only_frames_matching_the_armed_geometry() {
        let (sender, receiver) = sync_channel::<EncoderMessage>(RECORD_QUEUE);
        let mut record = HeadedRecord {
            config: HeadedRecordConfig {
                enabled: true,
                fps: 2.0,
                readback_timeout: None,
                finish_timeout: None,
            },
            pacer: RecordPacer::new(Duration::from_millis(500)),
            gated: true,
            active: Some(ActiveRecording::for_test(sender, 2.0)),
            legacy_active: None,
            finalizing: None,
            done: false,
        };
        let frame = |width: u32, height: u32, fill: u8| {
            RecordFrame::window_rgba(
                width,
                height,
                vec![fill; 4 * width as usize * height as usize],
                u64::from(fill),
                None,
            )
        };
        let step = StepInfo {
            step: 1,
            total: 2,
            name: "walk".into(),
            tick: 7,
        };
        // 32×128 carries exactly the armed 64×64 byte count: geometry alone
        // refuses it.
        record
            .pump_frames(
                vec![frame(64, 64, 1), frame(32, 128, 2), frame(64, 64, 3)],
                &step,
            )
            .unwrap();
        let mut short = frame(64, 64, 4);
        if let EncoderPixels::GpuRgba(bytes) = &mut short.pixels {
            bytes.pop();
        }
        record.pump_frames(vec![short], &step).unwrap();
        let active = record.active.as_ref().expect("still armed");
        assert_eq!(active.queued, 2, "only same-geometry frames queue");
        assert_eq!(active.size_mismatch, 2, "resized and short frames drop");
        for expected in [1, 3] {
            let EncoderMessage::Frame(frame) = receiver.try_recv().unwrap() else {
                panic!("frame queue contains a frame");
            };
            assert_eq!(frame.pixels.rgba_bytes()[0], expected);
            assert!(frame.sidecar_line.as_str().contains("stamp=window:"));
        }
        assert!(receiver.try_recv().is_err());
    }

    #[test]
    fn readback_permit_transfers_only_after_encoder_admission_or_raw_drop() {
        use client::render::backend::{CaptureGeometry, ReadbackBudget, ReadbackError};

        let budget = ReadbackBudget::process();
        let geometry = CaptureGeometry::new(64, 64);
        let deadline = Instant::now() + Duration::from_secs(60);
        let (sender, receiver) = sync_channel::<EncoderMessage>(1);
        let mut record = HeadedRecord {
            config: HeadedRecordConfig {
                enabled: true,
                fps: 2.0,
                readback_timeout: Some(Duration::from_secs(60)),
                finish_timeout: Some(Duration::from_secs(60)),
            },
            pacer: RecordPacer::new(Duration::from_millis(500)),
            gated: true,
            active: Some(ActiveRecording::for_test(sender, 2.0)),
            legacy_active: None,
            finalizing: None,
            done: false,
        };
        let step = StepInfo {
            step: 0,
            total: 1,
            name: "encoder handoff".into(),
            tick: 0,
        };
        for sequence in [1, 2] {
            let permit = budget
                .try_acquire(geometry, deadline)
                .expect("previous raw frame transferred or dropped");
            let frame = RecordFrame::window_rgba(
                64,
                64,
                vec![sequence as u8; 64 * 64 * 4],
                sequence,
                Some(permit),
            );
            assert!(matches!(
                budget.try_acquire(geometry, deadline),
                Err(ReadbackError::Busy)
            ));
            record.pump_frames(vec![frame], &step).unwrap();
        }
        let active = record.active.as_ref().unwrap();
        assert_eq!(active.queued, 1);
        assert_eq!(
            active.dropped, 1,
            "full queue destroys the second raw frame"
        );
        let released = budget
            .try_acquire(geometry, deadline)
            .expect("both bounded acceptance and refusal release the handoff permit");
        let EncoderMessage::Frame(frame) = receiver.try_recv().unwrap() else {
            panic!("the admitted frame remains owned by the encoder queue");
        };
        assert_eq!(frame.pixels.rgba_bytes()[0], 1);
        assert!(receiver.try_recv().is_err());
        drop(released);
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
            settings_lines(&mut tracker, 1.0, "capture", SettingState::Pending, ""),
            [
                None,
                Some("t=1.000 setting=\"capture\" state=pending error=\"\"".to_string())
            ]
        );
        // The same state reported again is not a change.
        assert_eq!(
            settings_lines(&mut tracker, 1.5, "capture", SettingState::Pending, ""),
            [None, None]
        );
        assert_eq!(
            settings_lines(&mut tracker, 2.0, "capture", SettingState::Saved, ""),
            [
                None,
                Some("t=2.000 setting=\"capture\" state=saved error=\"\"".to_string())
            ]
        );
        // Another setting tracks independently.
        assert_eq!(
            settings_lines(&mut tracker, 2.5, "sidecar_50", SettingState::Pending, ""),
            [
                None,
                Some("t=2.500 setting=\"sidecar_50\" state=pending error=\"\"".to_string())
            ]
        );
    }

    #[test]
    fn settings_tracker_failed_then_saved_does_not_invent_a_clear() {
        let mut tracker = SettingsTracker::default();
        settings_lines(&mut tracker, 1.0, "capture", SettingState::Pending, "");
        settings_lines(
            &mut tracker,
            6.0,
            "capture",
            SettingState::Failed,
            "preferences is busy",
        );
        assert_eq!(
            settings_lines(&mut tracker, 9.0, "capture", SettingState::Saved, ""),
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
        settings_lines(
            &mut tracker,
            1.0,
            "capture",
            SettingState::Failed,
            "disk full",
        );
        assert_eq!(
            settings_lines(&mut tracker, 3.0, "capture", SettingState::Pending, ""),
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
            settings_lines(&mut tracker, 1.0, "capture", SettingState::Cleared, ""),
            [
                Some("t=1.000 setting=\"capture\" state=cleared error=\"\"".to_string()),
                None
            ]
        );
        assert_eq!(
            settings_lines(&mut tracker, 1.5, "capture", SettingState::Cleared, ""),
            [None, None],
            "a repeated explicit clear is not a new transition"
        );
        settings_lines(&mut tracker, 2.0, "capture", SettingState::Failed, "one");
        // A second failure with new text is a change of its own.
        assert_eq!(
            settings_lines(&mut tracker, 3.0, "capture", SettingState::Failed, "two"),
            [
                None,
                Some("t=3.000 setting=\"capture\" state=failed error=\"two\"".to_string())
            ]
        );
        assert_eq!(
            settings_lines(&mut tracker, 4.0, "capture", SettingState::Failed, "two"),
            [None, None],
            "the same failure reported again is not a change"
        );
        // An owner that clears the failure (expiry, a dismiss) writes one
        // cleared line and no state line after it.
        assert_eq!(
            settings_lines(&mut tracker, 20.0, "capture", SettingState::Cleared, ""),
            [
                Some("t=20.000 setting=\"capture\" state=cleared error=\"\"".to_string()),
                None
            ]
        );
        assert_eq!(
            settings_lines(&mut tracker, 21.0, "capture", SettingState::Pending, ""),
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
        let (sender, receiver) = sync_channel::<Vec<u8>>(RECORD_QUEUE);
        let mut record = HeadedRecord {
            config: HeadedRecordConfig {
                enabled: true,
                fps: DEFAULT_FPS,
                readback_timeout: None,
                finish_timeout: None,
            },
            pacer: RecordPacer::new(Duration::from_millis(500)),
            gated: false,
            active: None,
            legacy_active: Some(LegacyActiveRecording {
                sender,
                pending_receiver: Some(receiver),
                thread: None,
                mp4_path: PathBuf::new(),
                txt_path: PathBuf::new(),
                fps: DEFAULT_FPS,
                width: 0,
                height: 0,
                lines: Vec::new(),
                started: Instant::now(),
                settings_file: Some(BufWriter::new(file)),
                settings: SettingsTracker::default(),
                settings_incomplete: false,
                queued: 0,
                dropped: 0,
                size_mismatch: 0,
            }),
            finalizing: None,
            done: false,
        };

        record.note_settings_incomplete();
        record.note_settings_incomplete();

        assert_eq!(
            std::fs::read_to_string(path).expect("read settings sidecar"),
            format!("{SETTINGS_INCOMPLETE_MARKER}\n")
        );
    }

    #[cfg(all(feature = "render-test-hooks", unix))]
    fn run_isolated_recorder_fixture(mode: &str) {
        use std::os::unix::fs::PermissionsExt;

        let directory = crate::test_support::TestDir::new("headed-record-fixture");
        let encoder = directory.join("fake-ffmpeg");
        let marker = directory.join("encoder.started");
        let script = if mode == "encoder-block" {
            "#!/bin/sh\n: > \"$RECORDER_FIXTURE_MARKER\"\nexec /bin/sleep 3\n"
        } else if mode == "encoder-exits" {
            "#!/bin/sh\nexit 23\n"
        } else {
            "#!/bin/sh\noutput=\nfor argument do output=$argument; done\ntest -d \"${output%/*}\" || exit 17\n: > \"$output\" || exit 18\nexec /bin/cat >/dev/null\n"
        };
        std::fs::write(&encoder, script).expect("write deterministic fake encoder");
        let mut permissions = std::fs::metadata(&encoder)
            .expect("inspect fake encoder")
            .permissions();
        permissions.set_mode(0o700);
        std::fs::set_permissions(&encoder, permissions).expect("make fake encoder executable");

        let mut child =
            Command::new(std::env::current_exe().expect("current recorder test executable"))
                .args([
                    "--ignored",
                    "--exact",
                    "--nocapture",
                    "headed_record::tests::recorder_fixture_child",
                ])
                .env("RECORDER_CHILD_MODE", mode)
                .env("RECORDER_FIXTURE_DIR", directory.as_os_str())
                .env("RECORDER_FIXTURE_MARKER", marker.as_os_str())
                .env(RECORD_ENV, "1")
                .env(RECORD_FPS_ENV, "2")
                .env(RECORD_FFMPEG_ENV, encoder.as_os_str())
                .env("T44_CAPTURE_READBACK_TIMEOUT_MS", "5000")
                .env(
                    "T44_RECORD_FINISH_TIMEOUT_MS",
                    if matches!(mode, "encoder-block" | "sidecar-block" | "queue") {
                        "100"
                    } else {
                        "5000"
                    },
                )
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .expect("spawn isolated recorder fixture process");
        let child_deadline = Instant::now() + Duration::from_secs(15);
        let mut timed_out = false;
        loop {
            match child.try_wait() {
                Ok(Some(_)) => break,
                Ok(None) if Instant::now() < child_deadline => thread::yield_now(),
                _ => {
                    timed_out = true;
                    let _ = child.kill();
                    break;
                }
            }
        }
        let output = child
            .wait_with_output()
            .expect("collect isolated recorder fixture output");
        assert!(
            !timed_out,
            "recorder fixture child exceeded its bounded test deadline:\n{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            output.status.success(),
            "recorder fixture child failed:\n{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            String::from_utf8_lossy(&output.stdout).contains("recorder fixture child completed"),
            "exact recorder fixture child did not run:\n{}",
            String::from_utf8_lossy(&output.stdout)
        );
    }

    #[cfg(all(feature = "render-test-hooks", unix))]
    fn configured_fixture_record() -> HeadedRecord {
        let record = HeadedRecord::from_env();
        assert!(record.gated, "the child configured the render fixture gate");
        assert!(record.enabled(), "the explicit recorder fixture is enabled");
        record
    }

    #[cfg(all(feature = "render-test-hooks", unix))]
    fn wait_for_path(path: &Path, timeout: Duration) {
        let deadline = Instant::now() + timeout;
        while !path.exists() {
            assert!(
                Instant::now() < deadline,
                "fixture process did not create {}",
                path.display()
            );
            thread::yield_now();
        }
    }

    #[cfg(all(feature = "render-test-hooks", unix))]
    fn wait_for_finish_status(
        record: &mut HeadedRecord,
        expected: RecordFinish,
        timeout: Duration,
    ) -> RecordFinish {
        let deadline = Instant::now() + timeout;
        loop {
            let status = record.poll_finish(Instant::now());
            if status == expected {
                return status;
            }
            assert!(
                Instant::now() < deadline,
                "recorder did not report {expected:?}; last status was {status:?}"
            );
            thread::yield_now();
        }
    }

    #[cfg(all(feature = "render-test-hooks", unix))]
    fn wait_for_finalized(record: &mut HeadedRecord, timeout: Duration) -> RecordFinish {
        let deadline = Instant::now() + timeout;
        loop {
            let status = record.poll_finish(Instant::now());
            if !record.is_finalizing() {
                return status;
            }
            assert!(
                Instant::now() < deadline,
                "supervisor/writer did not join after fixture release; status {status:?}"
            );
            thread::yield_now();
        }
    }

    #[cfg(all(feature = "render-test-hooks", unix))]
    fn offer_fixture_frame(
        record: &mut HeadedRecord,
        step: &StepInfo,
        width: u32,
        height: u32,
        sequence: u64,
    ) -> bool {
        let queued_before = record.active.as_ref().expect("record is armed").queued;
        let pixels = vec![sequence as u8; width as usize * height as usize * 4];
        record
            .pump_frames(
                vec![RecordFrame::window_rgba(
                    width, height, pixels, sequence, None,
                )],
                step,
            )
            .expect("bounded recorder frame metadata");
        record.active.as_ref().expect("record remains armed").queued > queued_before
    }

    #[cfg(all(feature = "render-test-hooks", unix))]
    fn create_fifo(path: &Path) {
        let result = Command::new("mkfifo")
            .arg(path)
            .status()
            .expect("run mkfifo for a deterministic blocked sidecar fixture");
        assert!(result.success(), "mkfifo failed for {}", path.display());
    }

    #[cfg(all(feature = "render-test-hooks", unix))]
    fn release_fifo(path: &Path) -> File {
        std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)
            .expect("open FIFO read/write to release the blocked writer")
    }

    #[cfg(all(feature = "render-test-hooks", unix))]
    #[test]
    fn capture_metadata_refuses_before_copy_and_sidecar_format_stays_bounded() {
        run_isolated_recorder_fixture("metadata");
    }

    #[cfg(all(feature = "render-test-hooks", unix))]
    #[test]
    fn sidecar_memory_is_constant_and_lines_match_written_frames() {
        run_isolated_recorder_fixture("sidecar-memory");
    }

    #[cfg(all(feature = "render-test-hooks", unix))]
    #[test]
    fn blocked_encoder_write_cannot_block_finish_control() {
        run_isolated_recorder_fixture("encoder-block");
    }

    #[cfg(all(feature = "render-test-hooks", unix))]
    #[test]
    fn recorder_failure_before_finish_reaches_the_live_exit() {
        run_isolated_recorder_fixture("encoder-exits");
    }

    #[cfg(all(feature = "render-test-hooks", unix))]
    #[test]
    fn blocked_sidecar_writer_refuses_rearm_without_memory_growth() {
        run_isolated_recorder_fixture("sidecar-block");
    }

    #[cfg(all(feature = "render-test-hooks", unix))]
    #[test]
    fn raw_and_padded_geometry_limits_refuse_before_allocation() {
        run_isolated_recorder_fixture("geometry");
    }

    #[cfg(all(feature = "render-test-hooks", unix))]
    #[test]
    fn record_queue_saturation_preserves_ticks_and_ui() {
        run_isolated_recorder_fixture("queue");
        bounded_recorder_fixed_layout_receipt();
    }

    #[cfg(all(feature = "render-test-hooks", unix))]
    #[test]
    fn legacy_render_fixture_gate_off_keeps_all_three_outputs() {
        run_isolated_recorder_fixture("legacy");
    }

    #[cfg(all(feature = "render-test-hooks", unix))]
    #[test]
    fn retired_play_keeps_polling_recording_completion() {
        run_isolated_recorder_fixture("retired-play");
    }

    #[cfg(all(feature = "render-test-hooks", unix))]
    #[test]
    #[ignore = "invoked in an isolated process by recorder acceptance tests"]
    fn recorder_fixture_child() {
        let mode = std::env::var("RECORDER_CHILD_MODE").expect("recorder fixture mode");
        if mode != "legacy" {
            host::render_head::test_hooks::configure_process(
                host::render_head::test_hooks::ProcessFixture {
                    grant_ttl: Duration::from_millis(20),
                    quantum: Duration::from_millis(40),
                },
            )
            .expect("configure the render fixture before recorder construction");
            host::render_head::test_hooks::with_process_render_budget(|| ());
        }
        let fixture_dir =
            PathBuf::from(std::env::var_os("RECORDER_FIXTURE_DIR").expect("fixture directory"));
        match mode.as_str() {
            "metadata" => {
                let worst_name = "\u{0001}".repeat(256);
                let stamp = RecordingStamp::Window {
                    paint_sequence: u64::MAX,
                };
                let line = bounded_sidecar_line(
                    18_446_744_073_709_551_615.0,
                    u64::MAX,
                    usize::MAX,
                    usize::MAX,
                    u32::MAX,
                    stamp,
                    &worst_name,
                )
                .expect("maximum UTF-8 name remains within the fixed line");
                let derivation = 256 * 6 + 16 * 20 + 512;
                assert!(line.as_str().len() <= derivation);
                assert_eq!(line.as_str().matches("\\u0001").count(), 256);
                assert!(
                    bounded_sidecar_header(&worst_name, DEFAULT_FPS)
                        .expect("maximum escaped header")
                        .as_str()
                        .len()
                        <= 4096
                );
                let maximum_setting =
                    bounded_settings_line(0.0, &worst_name, SettingState::Failed, &worst_name)
                        .expect("both maximum escaped settings fields remain bounded");
                assert_eq!(maximum_setting.as_str().len(), 3112);
                let settings_derivation = 2 * 256 * 6 + 16 * 20 + 512;
                assert!(maximum_setting.as_str().len() <= settings_derivation);
                assert!(
                    bounded_settings_header(&worst_name)
                        .expect("maximum escaped settings header")
                        .as_str()
                        .len()
                        <= 4096
                );

                let too_long_name = "é".repeat(129);
                assert!(matches!(
                    bounded_sidecar_header(&too_long_name, DEFAULT_FPS),
                    Err(RecordError::MetadataTooLarge)
                ));
                assert!(matches!(
                    bounded_settings_header(&too_long_name),
                    Err(RecordError::MetadataTooLarge)
                ));
                assert!(matches!(
                    bounded_sidecar_line(0.0, 0, 0, 1, 0, stamp, &too_long_name),
                    Err(RecordError::MetadataTooLarge)
                ));

                let output_dir = fixture_dir.join("not-created-for-refused-metadata");
                let mut oversized_name = configured_fixture_record();
                assert_eq!(
                    oversized_name.arm(&output_dir, &"n".repeat(257)),
                    Err(RecordError::MetadataTooLarge)
                );
                assert!(!output_dir.exists(), "name refusal precedes output setup");

                let exact_base = PathBuf::from("x".repeat(4096 - RECORD_SIDECAR_NAME.len() - 1));
                let exact_path = checked_child_path(&exact_base, RECORD_SIDECAR_NAME)
                    .expect("the full 4096-byte output path is accepted");
                assert_eq!(exact_path.as_os_str().len(), 4096);
                let oversized_base = PathBuf::from("x".repeat(4096));
                assert_eq!(
                    checked_child_path(&oversized_base, RECORD_SIDECAR_NAME),
                    Err(RecordError::MetadataTooLarge)
                );
                let mut oversized_path = configured_fixture_record();
                assert_eq!(
                    oversized_path.arm(&oversized_base, "scenario"),
                    Err(RecordError::MetadataTooLarge)
                );
                assert!(
                    !oversized_base.exists(),
                    "path refusal does not create output directories"
                );
            }
            "retired-play" => {
                let output_dir = fixture_dir.join("retired play recording");
                let mut record = configured_fixture_record();
                record
                    .arm(&output_dir, "retired play")
                    .expect("arm through the gated recorder path");
                assert!(offer_fixture_frame(
                    &mut record,
                    &StepInfo {
                        step: 0,
                        total: 1,
                        name: "last benchmark frame".into(),
                        tick: 1,
                    },
                    2,
                    2,
                    1,
                ));
                record.request_finish(Instant::now());
                let deadline = Instant::now() + Duration::from_secs(5);
                while !record
                    .finalizing
                    .as_ref()
                    .unwrap()
                    .supervisor
                    .as_ref()
                    .unwrap()
                    .is_finished()
                {
                    assert!(Instant::now() < deadline, "recorder did not finish");
                    thread::yield_now();
                }
                crate::app::tests::recording_exit_with_retired_play(record);
            }
            "sidecar-memory" => {
                let output_dir = fixture_dir.join("new output directory/recording");
                assert!(!output_dir.exists());
                let mut record = configured_fixture_record();
                record
                    .arm(&output_dir, "sidecar memory")
                    .expect("arm through the gated recorder path");
                let step = StepInfo {
                    step: 3,
                    total: 9,
                    name: "owned frame".to_string(),
                    tick: 77,
                };
                let mut accepted_sequences = Vec::new();
                assert!(offer_fixture_frame(&mut record, &step, 2, 2, 10));
                accepted_sequences.push(10);
                record.note_setting("capture", SettingState::Pending, "");
                assert!(!record
                    .active
                    .as_ref()
                    .expect("record is active")
                    .settings_incomplete
                    .load(Ordering::Acquire));
                for sequence in 11..267 {
                    if offer_fixture_frame(&mut record, &step, 2, 2, sequence) {
                        accepted_sequences.push(sequence);
                    }
                }
                let queued = record.active.as_ref().expect("record is active").queued;
                assert_eq!(accepted_sequences.len(), queued as usize);
                assert!(queued >= RECORD_QUEUE as u64 - 1);
                record.request_finish(Instant::now());
                assert_eq!(
                    wait_for_finalized(&mut record, Duration::from_secs(5)),
                    RecordFinish::Complete
                );

                let sidecar = std::fs::read_to_string(output_dir.join(RECORD_SIDECAR_NAME))
                    .expect("writer-created recording sidecar");
                let lines: Vec<_> = sidecar.lines().collect();
                assert_eq!(lines.len(), accepted_sequences.len() + 2);
                for (frame_index, (line, sequence)) in
                    lines[2..].iter().zip(accepted_sequences.iter()).enumerate()
                {
                    assert!(line.contains(&format!("frame={frame_index} ")), "{line}");
                    assert!(line.contains(&format!("stamp=window:{sequence}")), "{line}");
                    assert!(line.contains("step=3/9 tick=77"), "{line}");
                }
                let settings = std::fs::read_to_string(output_dir.join(RECORD_SETTINGS_NAME))
                    .expect("writer-created settings sidecar");
                assert!(settings.contains("state=pending error=\"\""), "{settings}");
                assert_eq!(
                    std::fs::metadata(output_dir.join(RECORD_MP4_NAME))
                        .expect("fake encoder created output after writer setup")
                        .len(),
                    0,
                    "fake process proves output directory existed before encoder startup"
                );
            }
            "encoder-exits" => {
                let output_dir = fixture_dir.join("failed encoder");
                let mut record = configured_fixture_record();
                record
                    .arm(&output_dir, "failed encoder")
                    .expect("arm failing encoder");
                let step = StepInfo {
                    step: 0,
                    total: 1,
                    name: "failed encoder".to_string(),
                    tick: 0,
                };
                assert!(offer_fixture_frame(&mut record, &step, 2, 2, 1));
                let deadline = Instant::now() + Duration::from_secs(5);
                loop {
                    let active = record.active.as_ref().expect("recorder remains active");
                    if active
                        .supervisor
                        .as_ref()
                        .is_some_and(JoinHandle::is_finished)
                    {
                        assert_eq!(*active.outcome.lock().unwrap(), RecordFinish::Failed);
                        break;
                    }
                    assert!(
                        Instant::now() < deadline,
                        "failing encoder did not terminate"
                    );
                    thread::yield_now();
                }
                let failure = Mutex::new(None);
                assert!(crate::app::finish_recording_for_exit(&mut record, &failure));
                assert!(failure
                    .lock()
                    .unwrap()
                    .as_ref()
                    .is_some_and(|error| error.to_string().contains("Failed")));
                assert!(!record.is_finalizing(), "the exited supervisor was joined");
                drop(RecorderPermit::try_acquire().expect("failed encoder releases permit"));
            }
            "encoder-block" => {
                let output_dir = fixture_dir.join("new output directory/blocked encoder");
                let mut record = configured_fixture_record();
                record
                    .arm(&output_dir, "blocked encoder")
                    .expect("arm blocked-pipe fixture");
                let width = 1984;
                let height = 2016;
                let pixel_bytes = checked_raw_frame_bytes(width, height)
                    .expect("large in-envelope frame avoids arbitrary test-only caps");
                let step = StepInfo {
                    step: 0,
                    total: 1,
                    name: "blocked pipe".to_string(),
                    tick: 0,
                };
                record
                    .pump_frames(
                        vec![RecordFrame::window_rgba(
                            width,
                            height,
                            vec![0x5a; pixel_bytes],
                            1,
                            None,
                        )],
                        &step,
                    )
                    .expect("queue maximum-bound recording fixture frame");
                let marker =
                    PathBuf::from(std::env::var_os("RECORDER_FIXTURE_MARKER").expect("marker"));
                wait_for_path(&marker, Duration::from_secs(5));
                record.request_finish(Instant::now());
                assert_eq!(
                    wait_for_finalized(&mut record, Duration::from_secs(5)),
                    RecordFinish::TimedOut,
                    "finish control kills the non-reading owned encoder process"
                );
                assert!(
                    !record.is_finalizing(),
                    "supervisor reaped the child and joined"
                );
                drop(RecorderPermit::try_acquire().expect("reaped recorder releases permit"));
            }
            "sidecar-block" | "queue" => {
                let output_dir = fixture_dir.join(format!("{mode} output"));
                std::fs::create_dir_all(&output_dir).expect("create FIFO fixture directory");
                let sidecar_path = output_dir.join(RECORD_SIDECAR_NAME);
                create_fifo(&sidecar_path);
                let mut record = configured_fixture_record();
                record
                    .arm(&output_dir, "blocked sidecar")
                    .expect("arm blocked-sidecar fixture");
                let step = StepInfo {
                    step: 1,
                    total: 2,
                    name: "queue saturation".to_string(),
                    tick: 42,
                };
                let frame_count = if mode == "queue" {
                    RECORD_QUEUE * 8
                } else {
                    RECORD_QUEUE + 1
                };
                for sequence in 0..frame_count {
                    assert_eq!(
                        offer_fixture_frame(&mut record, &step, 1, 1, sequence as u64,),
                        sequence < RECORD_QUEUE,
                        "the blocked writer accepts only the eight bounded queue slots"
                    );
                }
                let active = record.active.as_ref().expect("record remains active");
                assert_eq!(active.queued, RECORD_QUEUE as u64);
                assert_eq!(
                    active.dropped,
                    (frame_count - RECORD_QUEUE) as u64,
                    "full try-send queue drops without growing another backlog"
                );

                if mode == "queue" {
                    // This recorder-local assertion covers the queue and pacer; it does not
                    // stand in for the parent's live panel/UI tick proof.
                    let start = Instant::now();
                    assert!(record.is_due(start));
                    assert!(!record.is_due(start + Duration::from_millis(499)));
                    assert!(record.is_due(start + Duration::from_millis(500)));
                    assert_eq!(
                        record.active.as_ref().expect("record active").queued,
                        RECORD_QUEUE as u64,
                        "recording opportunities remain bounded after overflow"
                    );
                }
                record.request_finish(Instant::now());
                assert_eq!(
                    wait_for_finish_status(
                        &mut record,
                        RecordFinish::TimedOut,
                        Duration::from_secs(5)
                    ),
                    RecordFinish::TimedOut
                );
                assert!(
                    record.is_finalizing(),
                    "timed-out writer remains quarantined while the FIFO is blocked"
                );
                let failure = Mutex::new(None);
                assert!(crate::app::finish_recording_for_exit(&mut record, &failure));
                assert!(failure
                    .lock()
                    .unwrap()
                    .as_ref()
                    .is_some_and(|error| error.to_string().contains("TimedOut")));
                assert!(
                    record.is_finalizing(),
                    "live exit does not join the blocked writer"
                );
                if mode == "sidecar-block" {
                    let mut retry = configured_fixture_record();
                    assert_eq!(
                        retry.arm(&output_dir.join("retry"), "retry"),
                        Err(RecordError::RecorderBusy),
                        "a blocked sidecar writer retains the process-wide permit"
                    );
                }
                let fifo_reader = release_fifo(&sidecar_path);
                assert_eq!(
                    wait_for_finalized(&mut record, Duration::from_secs(5)),
                    RecordFinish::TimedOut
                );
                drop(fifo_reader);
                drop(RecorderPermit::try_acquire().expect("released writer frees permit"));
            }
            "geometry" => {
                assert_eq!(checked_raw_frame_bytes(0, 64), None);
                assert_eq!(checked_raw_frame_bytes(64, 0), None);
                assert_eq!(
                    checked_raw_frame_bytes(1984, 2016),
                    Some(15_998_976),
                    "the accepted case stays below both raw and padded-byte limits"
                );
                assert_eq!(
                    checked_raw_frame_bytes(2000, 2000),
                    None,
                    "raw bytes fit at 16 MB but padded rows exceed the staging ceiling"
                );
                assert_eq!(
                    checked_raw_frame_bytes(2001, 2000),
                    None,
                    "raw pixels/bytes refuse before a frame vector is constructed"
                );
                assert_eq!(
                    checked_raw_frame_bytes(1, 4_000_000),
                    None,
                    "narrow/tall geometry refuses from padded staging bytes"
                );
                assert_eq!(
                    checked_raw_frame_bytes(u32::MAX, u32::MAX),
                    None,
                    "pathological dimensions refuse before allocation"
                );
            }
            "legacy" => {
                let output_dir = fixture_dir.join("legacy output");
                std::fs::create_dir_all(&output_dir).expect("create legacy output directory");
                let mut record = HeadedRecord::from_env();
                assert!(
                    !record.gated,
                    "no fixture gate was configured in this child"
                );
                assert_eq!(record.config.readback_timeout, None);
                assert_eq!(record.config.finish_timeout, None);
                record
                    .arm(&output_dir, "legacy scenario")
                    .expect("legacy gate-off path remains unchanged");
                record.note_setting("capture", SettingState::Pending, "");
                let step = StepInfo {
                    step: 2,
                    total: 3,
                    name: "legacy step".to_string(),
                    tick: 9,
                };
                record
                    .pump_frames(
                        vec![RecordFrame::window_rgba(2, 2, vec![0x2a; 16], 9, None)],
                        &step,
                    )
                    .expect("legacy frame path");
                record.finish();
                for filename in [RECORD_MP4_NAME, RECORD_SIDECAR_NAME, RECORD_SETTINGS_NAME] {
                    assert!(
                        output_dir.join(filename).is_file(),
                        "gate-off legacy recording retains {filename}"
                    );
                }
                assert!(
                    std::fs::read_to_string(output_dir.join(RECORD_SIDECAR_NAME))
                        .expect("legacy sidecar")
                        .contains("legacy step")
                );
                assert!(
                    std::fs::read_to_string(output_dir.join(RECORD_SETTINGS_NAME))
                        .expect("legacy settings sidecar")
                        .contains("state=pending")
                );
            }
            _ => panic!("unknown recorder fixture mode {mode:?}"),
        }
        println!("recorder fixture child completed: {mode}");
    }
}
