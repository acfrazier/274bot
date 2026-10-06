//! Shared background PNG+JSON evidence writer for the live harness.
//!
//! Live-cell evidence captures used to PNG-encode and write files inside the
//! slot's observe hook: `write_teller_png` built a renderer, rasterised a
//! frame, encoded the PNG and wrote it on the slot thread, adding ~90 ms
//! frame hitches that looked like product stalls. This module moves the
//! encode and the file writes to one background thread per cell.
//!
//! The slot thread does only what needs the client: render (or take an
//! already-rendered frame) into native pixmap words, then [`EvidenceWriter::submit`]
//! hands the pixels plus the JSON receipt to the writer. The writer converts,
//! encodes, writes the PNG and its sidecar, fsyncs both, and records the
//! outcome where the hook polls it. At cell end the existing `capture_written`
//! poll loops are the bounded flush: they only observe completion once the
//! files are durable.
//!
//! When the bounded queue is full, `submit` blocks briefly (bounded by
//! [`WriterConfig::submit_wait`]) and then drops with a counted error rather
//! than stalling the slot: a dropped terminal capture must fail the cell
//! loudly like any other capture error, never stall it silently. Drops,
//! failures and writes are counted ([`EvidenceWriter::stats`]) and logged.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// How many captures may wait for the background thread.
pub const DEFAULT_QUEUE_BOUND: usize = 8;
/// How long `submit` waits for queue space before it drops with an error.
pub const DEFAULT_SUBMIT_WAIT: Duration = Duration::from_secs(2);
/// Default bound for [`EvidenceWriter::flush`] and test-thread captures.
pub const DEFAULT_FLUSH_WAIT: Duration = Duration::from_secs(30);

/// PNG colour layout. Every site uses RGBA except `web_cut_live`, whose
/// historic captures are RGB; the writer preserves each site's layout so
/// file bytes stay identical.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PngColor {
    Rgba,
    Rgb,
}

/// Records a PNG failure under the site's own receipt key (bank core uses
/// `png_error`, the mage teller uses `image.error`).
pub type ReceiptErrorPatch = Box<dyn Fn(&mut serde_json::Value, &str) + Send>;

pub struct EvidenceSidecar {
    pub path: PathBuf,
    pub receipt: serde_json::Value,
    pub patch_error: Option<ReceiptErrorPatch>,
}

/// One capture handed to the background thread.
pub struct EvidenceRequest {
    pub png_path: PathBuf,
    pub width: u32,
    pub height: u32,
    /// Native pixmap words, row-major. Converted off the slot thread.
    pub pixels: Vec<i32>,
    pub color: PngColor,
    pub sidecar: Option<EvidenceSidecar>,
}

/// The finished work for one capture. `error` is `None` only when the PNG
/// (and the sidecar, when present) are written and fsynced.
#[derive(Debug, Clone)]
pub struct EvidenceOutcome {
    pub width: u32,
    pub height: u32,
    pub png_bytes: u64,
    pub convert_ms: f64,
    pub encode_ms: f64,
    pub write_ms: f64,
    pub sync_ms: f64,
    pub error: Option<String>,
}

/// Queue/drop counters. `dropped` counts queue-full submits; `failed` counts
/// captures the worker could not write.
#[derive(Debug, Clone, Copy, Default)]
pub struct WriterStats {
    pub submitted: u64,
    pub dropped: u64,
    pub failed: u64,
    pub written: u64,
}

/// Writer tuning. `bound == 0` drops every submit after `submit_wait`.
#[derive(Clone)]
pub struct WriterConfig {
    pub bound: usize,
    pub submit_wait: Duration,
    /// Test-only gate the worker calls with the job id just before
    /// `write_capture`. Absent in non-test builds, so it costs nothing there.
    #[cfg(test)]
    pub before_write: Option<Arc<dyn Fn(u64) + Send + Sync>>,
}

impl std::fmt::Debug for WriterConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WriterConfig")
            .field("bound", &self.bound)
            .field("submit_wait", &self.submit_wait)
            .finish()
    }
}

impl Default for WriterConfig {
    fn default() -> Self {
        Self {
            bound: DEFAULT_QUEUE_BOUND,
            submit_wait: DEFAULT_SUBMIT_WAIT,
            #[cfg(test)]
            before_write: None,
        }
    }
}

struct JobSlot {
    outcome: parking_lot::Mutex<Option<EvidenceOutcome>>,
    ready: parking_lot::Condvar,
}

/// Handle to one submitted capture. Poll it from the observe hook; the
/// outcome is present only after the files are durable.
#[derive(Clone)]
pub struct EvidenceJob {
    id: u64,
    slot: Arc<JobSlot>,
}

impl EvidenceJob {
    fn completed(id: u64, outcome: EvidenceOutcome) -> Self {
        Self {
            id,
            slot: Arc::new(JobSlot {
                outcome: parking_lot::Mutex::new(Some(outcome)),
                ready: parking_lot::Condvar::new(),
            }),
        }
    }

    fn pending(id: u64) -> Self {
        Self {
            id,
            slot: Arc::new(JobSlot {
                outcome: parking_lot::Mutex::new(None),
                ready: parking_lot::Condvar::new(),
            }),
        }
    }

    pub fn id(&self) -> u64 {
        self.id
    }
}

struct QueuedJob {
    job: EvidenceJob,
    request: EvidenceRequest,
}

struct WriterState {
    queue: VecDeque<QueuedJob>,
    /// Jobs popped by the worker but not yet finished.
    active: usize,
    next_id: u64,
    /// False once [`EvidenceWriter`] is dropped; the worker drains first.
    alive: bool,
    stats: WriterStats,
}

struct WriterInner {
    state: parking_lot::Mutex<WriterState>,
    changed: parking_lot::Condvar,
    config: WriterConfig,
}

/// One background PNG+JSON evidence writer. Share one per cell across its
/// slots; the worker processes captures FIFO.
pub struct EvidenceWriter {
    inner: Arc<WriterInner>,
    thread: parking_lot::Mutex<Option<JoinHandle<()>>>,
}

impl EvidenceWriter {
    pub fn new(bound: usize) -> Self {
        Self::with_config(WriterConfig {
            bound,
            ..WriterConfig::default()
        })
    }

    pub fn with_config(config: WriterConfig) -> Self {
        let inner = Arc::new(WriterInner {
            state: parking_lot::Mutex::new(WriterState {
                queue: VecDeque::new(),
                active: 0,
                next_id: 0,
                alive: true,
                stats: WriterStats::default(),
            }),
            changed: parking_lot::Condvar::new(),
            config,
        });
        let worker_inner = Arc::clone(&inner);
        let fallback_inner = Arc::clone(&inner);
        let thread = std::thread::Builder::new()
            .name("evidence-writer".into())
            .spawn(move || worker_loop(worker_inner))
            .unwrap_or_else(|_| {
                // The harness runs with threads available; a spawn failure
                // must still leave a usable writer whose submits fail
                // loudly instead of hanging the slot. Fall back to a thread
                // that only marks the writer dead.
                std::thread::Builder::new()
                    .name("evidence-writer-dead".into())
                    .spawn(move || {
                        fallback_inner.state.lock().alive = false;
                        fallback_inner.changed.notify_all();
                    })
                    .expect("evidence writer fallback thread spawns")
            });
        Self {
            inner,
            thread: parking_lot::Mutex::new(Some(thread)),
        }
    }

    /// Hand pixels plus the JSON receipt to the background thread. Only
    /// locks briefly; blocks at most `submit_wait` when the queue is full,
    /// then drops with a counted error outcome.
    pub fn submit(&self, request: EvidenceRequest) -> EvidenceJob {
        let mut state = self.inner.state.lock();
        let deadline = Instant::now() + self.inner.config.submit_wait;
        while state.alive && state.queue.len() >= self.inner.config.bound {
            let now = Instant::now();
            if now >= deadline {
                break;
            }
            self.inner.changed.wait_for(&mut state, deadline - now);
        }
        if !state.alive || state.queue.len() >= self.inner.config.bound {
            state.stats.submitted += 1;
            state.stats.dropped += 1;
            let dropped = state.stats.dropped;
            let id = state.next_id;
            state.next_id += 1;
            drop(state);
            let message = format!(
                "evidence queue full (bound {}); capture {} dropped (dropped={dropped})",
                self.inner.config.bound,
                request.png_path.display(),
            );
            println!(
                "evidence-dropped png={} dropped={dropped}",
                request.png_path.display()
            );
            return EvidenceJob::completed(
                id,
                EvidenceOutcome {
                    width: request.width,
                    height: request.height,
                    png_bytes: 0,
                    convert_ms: 0.0,
                    encode_ms: 0.0,
                    write_ms: 0.0,
                    sync_ms: 0.0,
                    error: Some(message),
                },
            );
        }
        let id = state.next_id;
        state.next_id += 1;
        state.stats.submitted += 1;
        let job = EvidenceJob::pending(id);
        state.queue.push_back(QueuedJob {
            job: job.clone(),
            request,
        });
        drop(state);
        self.inner.changed.notify_one();
        job
    }

    /// Non-blocking outcome check for the observe hook.
    pub fn poll(&self, job: &EvidenceJob) -> Option<EvidenceOutcome> {
        job.slot.outcome.lock().clone()
    }

    /// Bounded wait for one capture, for test-thread callers.
    pub fn wait(&self, job: &EvidenceJob, timeout: Duration) -> Option<EvidenceOutcome> {
        let mut outcome = job.slot.outcome.lock();
        let deadline = Instant::now() + timeout;
        while outcome.is_none() {
            let now = Instant::now();
            if now >= deadline {
                break;
            }
            job.slot.ready.wait_for(&mut outcome, deadline - now);
        }
        outcome.clone()
    }

    /// Bounded wait until the queue is empty and no capture is in flight.
    /// True when everything submitted so far is durable.
    pub fn flush(&self, timeout: Duration) -> bool {
        let mut state = self.inner.state.lock();
        let deadline = Instant::now() + timeout;
        while state.alive && !(state.queue.is_empty() && state.active == 0) {
            let now = Instant::now();
            if now >= deadline {
                return false;
            }
            self.inner.changed.wait_for(&mut state, deadline - now);
        }
        state.queue.is_empty() && state.active == 0
    }
    pub fn stats(&self) -> WriterStats {
        self.inner.state.lock().stats
    }
}

impl Drop for EvidenceWriter {
    fn drop(&mut self) {
        {
            let mut state = self.inner.state.lock();
            state.alive = false;
        }
        self.inner.changed.notify_all();
        if let Some(thread) = self.thread.lock().take() {
            let _ = thread.join();
        }
    }
}

fn worker_loop(inner: Arc<WriterInner>) {
    loop {
        let queued = {
            let mut state = inner.state.lock();
            while state.alive && state.queue.is_empty() {
                inner.changed.wait(&mut state);
            }
            if state.queue.is_empty() {
                // Shutdown: the writer drains the queue before it exits.
                state.alive = false;
                inner.changed.notify_all();
                return;
            }
            state.active += 1;
            state.queue.pop_front().expect("queue checked non-empty")
        };
        // A freed slot must wake a submitter blocked on a full queue;
        // without this the submit stalls until `submit_wait`.
        inner.changed.notify_all();
        let QueuedJob { job, request } = queued;
        #[cfg(test)]
        if let Some(hook) = inner.config.before_write.clone() {
            hook(job.id);
        }
        let outcome = write_capture(job.id, &request);
        {
            let mut state = inner.state.lock();
            state.active -= 1;
            if outcome.error.is_some() {
                state.stats.failed += 1;
            } else {
                state.stats.written += 1;
            }
            *job.slot.outcome.lock() = Some(outcome.clone());
            job.slot.ready.notify_all();
            // Wake flush waiters (and any submitter rechecking state):
            // completion changes `active`/stats observed under `changed`.
            inner.changed.notify_all();
        }
        if outcome.error.is_some() {
            println!(
                "evidence-failed id={} png={} error={}",
                job.id,
                request.png_path.display(),
                outcome.error.as_deref().unwrap_or("unknown"),
            );
        } else {
            println!(
                "evidence-written id={} png={} {}x{} bytes={} convert_ms={:.1} encode_ms={:.1} write_ms={:.1} sync_ms={:.1}",
                job.id,
                request.png_path.display(),
                outcome.width,
                outcome.height,
                outcome.png_bytes,
                outcome.convert_ms,
                outcome.encode_ms,
                outcome.write_ms,
                outcome.sync_ms,
            );
        }
    }
}

fn write_capture(id: u64, request: &EvidenceRequest) -> EvidenceOutcome {
    let pixel_count = (request.width as usize)
        .checked_mul(request.height as usize)
        .filter(|count| *count > 0)
        .unwrap_or(0);
    if pixel_count == 0 || request.pixels.len() < pixel_count {
        let error = format!(
            "id={id}: rendered frame has invalid dimensions/data: {}x{}, {} pixels",
            request.width,
            request.height,
            request.pixels.len()
        );
        return finish_with_sidecar(request, error, 0, 0.0, 0.0, 0.0, 0.0);
    }
    let convert_start = Instant::now();
    let bytes_per_pixel = match request.color {
        PngColor::Rgba => 4,
        PngColor::Rgb => 3,
    };
    let mut raw = Vec::with_capacity(pixel_count * bytes_per_pixel);
    for pixel in request.pixels.iter().take(pixel_count) {
        let red = ((pixel >> 16) & 0xff) as u8;
        let green = ((pixel >> 8) & 0xff) as u8;
        let blue = (pixel & 0xff) as u8;
        match request.color {
            PngColor::Rgba => raw.extend_from_slice(&[red, green, blue, u8::MAX]),
            PngColor::Rgb => raw.extend_from_slice(&[red, green, blue]),
        }
    }
    let convert_ms = convert_start.elapsed().as_secs_f64() * 1000.0;
    let encode_start = Instant::now();
    let mut encoded = Vec::new();
    let encode_error = {
        let mut encoder = png::Encoder::new(&mut encoded, request.width, request.height);
        encoder.set_color(match request.color {
            PngColor::Rgba => png::ColorType::Rgba,
            PngColor::Rgb => png::ColorType::Rgb,
        });
        encoder.set_depth(png::BitDepth::Eight);
        match encoder.write_header() {
            Ok(mut writer) => writer
                .write_image_data(&raw)
                .err()
                .map(|error| error.to_string()),
            Err(error) => Some(error.to_string()),
        }
    };
    let encode_ms = encode_start.elapsed().as_secs_f64() * 1000.0;
    if let Some(error) = encode_error {
        let error = format!("id={id}: encode {}: {error}", request.png_path.display());
        return finish_with_sidecar(request, error, 0, convert_ms, encode_ms, 0.0, 0.0);
    }
    let write_start = Instant::now();
    let write_error = std::fs::write(&request.png_path, &encoded)
        .err()
        .map(|error| error.to_string());
    let write_ms = write_start.elapsed().as_secs_f64() * 1000.0;
    if let Some(error) = write_error {
        let _ = std::fs::remove_file(&request.png_path);
        let error = format!("id={id}: write {}: {error}", request.png_path.display());
        return finish_with_sidecar(request, error, 0, convert_ms, encode_ms, write_ms, 0.0);
    }
    let sync_start = Instant::now();
    let sync_error = std::fs::File::open(&request.png_path)
        .and_then(|file| file.sync_all())
        .err()
        .map(|error| error.to_string());
    let sync_ms = sync_start.elapsed().as_secs_f64() * 1000.0;
    if let Some(error) = sync_error {
        let error = format!("id={id}: fsync {}: {error}", request.png_path.display());
        return finish_with_sidecar(
            request,
            error,
            encoded.len() as u64,
            convert_ms,
            encode_ms,
            write_ms,
            sync_ms,
        );
    }
    finish_with_sidecar(
        request,
        String::new(),
        encoded.len() as u64,
        convert_ms,
        encode_ms,
        write_ms,
        sync_ms,
    )
}

/// Complete one capture: patch the receipt's error key on failure, write
/// and fsync the sidecar when present, then report. An empty `error` means
/// the PNG (and sidecar) are durable.
fn finish_with_sidecar(
    request: &EvidenceRequest,
    error: String,
    png_bytes: u64,
    convert_ms: f64,
    encode_ms: f64,
    write_ms: f64,
    sync_ms: f64,
) -> EvidenceOutcome {
    let failed = !error.is_empty();
    if let Some(sidecar) = request.sidecar.as_ref() {
        // Sites whose historic receipts record the PNG error (bank core,
        // mage teller) still land their sidecar on failure, patched. Sites
        // that wrote nothing on an encode failure pass no patch and keep
        // that shape: no sidecar, just the error.
        if failed && sidecar.patch_error.is_none() {
            return EvidenceOutcome {
                width: request.width,
                height: request.height,
                png_bytes,
                convert_ms,
                encode_ms,
                write_ms,
                sync_ms,
                error: Some(error),
            };
        }
        let mut receipt = sidecar.receipt.clone();
        if failed {
            if let Some(patch) = sidecar.patch_error.as_ref() {
                patch(&mut receipt, &error);
            }
        }
        let sidecar_error = match serde_json::to_vec_pretty(&receipt) {
            Ok(bytes) => std::fs::write(&sidecar.path, &bytes)
                .err()
                .map(|error| error.to_string())
                .or_else(|| {
                    std::fs::File::open(&sidecar.path)
                        .and_then(|file| file.sync_all())
                        .err()
                        .map(|error| error.to_string())
                }),
            Err(error) => Some(error.to_string()),
        };
        if let Some(sidecar_error) = sidecar_error {
            let detail = if failed {
                format!(
                    "{error} (sidecar {}: {sidecar_error})",
                    sidecar.path.display()
                )
            } else {
                format!("write {}: {sidecar_error}", sidecar.path.display())
            };
            return EvidenceOutcome {
                width: request.width,
                height: request.height,
                png_bytes,
                convert_ms,
                encode_ms,
                write_ms,
                sync_ms,
                error: Some(detail),
            };
        }
    }
    EvidenceOutcome {
        width: request.width,
        height: request.height,
        png_bytes,
        convert_ms,
        encode_ms,
        write_ms,
        sync_ms,
        error: if failed { Some(error) } else { None },
    }
}

/// PNG-failure variant: patch the receipt's error key, then write the
/// sidecar so failure receipts keep their historic shape. The slot thread
/// calls this only on the render-failure path (the worker handles its own
/// failures internally); both are failure-only, never a frame hitch.
pub fn write_failure_sidecar(sidecar: EvidenceSidecar, error: &str) -> String {
    let mut receipt = sidecar.receipt;
    if let Some(patch) = sidecar.patch_error {
        patch(&mut receipt, error);
    }
    let message = error.to_owned();
    match serde_json::to_vec_pretty(&receipt) {
        Ok(bytes) => {
            if let Err(error) = std::fs::write(&sidecar.path, &bytes) {
                return format!(
                    "write {}: {error} (capture failed: {message})",
                    sidecar.path.display()
                );
            }
            let _ = std::fs::File::open(&sidecar.path).and_then(|file| file.sync_all());
        }
        Err(error) => {
            return format!(
                "encode {}: {error} (capture failed: {message})",
                sidecar.path.display()
            );
        }
    }
    message
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_pixels(width: u32, height: u32) -> Vec<i32> {
        (0..width * height)
            .map(|index| 0xff00_0000u32 as i32 | index as i32 | (((index * 7) & 0xff) as i32) << 8)
            .collect()
    }

    fn direct_encode(width: u32, height: u32, pixels: &[i32]) -> Vec<u8> {
        let mut raw = Vec::with_capacity(pixels.len() * 4);
        for pixel in pixels {
            raw.extend_from_slice(&[
                ((pixel >> 16) & 0xff) as u8,
                ((pixel >> 8) & 0xff) as u8,
                (pixel & 0xff) as u8,
                u8::MAX,
            ]);
        }
        let mut encoded = Vec::new();
        let mut encoder = png::Encoder::new(&mut encoded, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()
            .expect("header")
            .write_image_data(&raw)
            .expect("pixels");
        encoded
    }

    #[test]
    fn writer_bytes_match_direct_encode_and_sidecar_is_exact() {
        let dir =
            std::env::temp_dir().join(format!("274bot-evidence-writer-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("scratch");
        let png_path = dir.join("01-final.png");
        let json_path = dir.join("01-final.json");
        let receipt =
            serde_json::json!({"scenario": "unit", "png": "01-final.png", "png_error": null});
        let writer = EvidenceWriter::new(8);
        let job = writer.submit(EvidenceRequest {
            png_path: png_path.clone(),
            width: 16,
            height: 9,
            pixels: test_pixels(16, 9),
            color: PngColor::Rgba,
            sidecar: Some(EvidenceSidecar {
                path: json_path.clone(),
                receipt: receipt.clone(),
                patch_error: None,
            }),
        });
        let outcome = writer
            .wait(&job, Duration::from_secs(10))
            .expect("capture completes");
        assert!(outcome.error.is_none(), "unexpected: {:?}", outcome.error);
        assert!(writer.flush(Duration::from_secs(10)));
        let on_disk = std::fs::read(&png_path).expect("png");
        assert_eq!(on_disk, direct_encode(16, 9, &test_pixels(16, 9)));
        let sidecar = std::fs::read(&json_path).expect("json");
        assert_eq!(
            sidecar,
            serde_json::to_vec_pretty(&receipt).expect("receipt")
        );
        let stats = writer.stats();
        assert_eq!(
            (stats.submitted, stats.written, stats.failed, stats.dropped),
            (1, 1, 0, 0)
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn rgb_capture_decodes_with_rgb_layout() {
        let dir = std::env::temp_dir().join(format!("274bot-evidence-rgb-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("scratch");
        let png_path = dir.join("shot.png");
        let writer = EvidenceWriter::new(8);
        let job = writer.submit(EvidenceRequest {
            png_path: png_path.clone(),
            width: 4,
            height: 3,
            pixels: test_pixels(4, 3),
            color: PngColor::Rgb,
            sidecar: None,
        });
        let outcome = writer
            .wait(&job, Duration::from_secs(10))
            .expect("capture completes");
        assert!(outcome.error.is_none(), "unexpected: {:?}", outcome.error);
        let file = std::io::BufReader::new(std::fs::File::open(&png_path).expect("png"));
        let mut decoder = png::Decoder::new(file);
        decoder.set_transformations(png::Transformations::normalize_to_color8());
        let mut reader = decoder.read_info().expect("info");
        let mut pixels = vec![0; reader.output_buffer_size().expect("png buffer size")];
        let info = reader.next_frame(&mut pixels).expect("frame");
        assert_eq!(info.color_type, png::ColorType::Rgb);
        assert_eq!((info.width, info.height), (4, 3));
        assert_eq!(pixels.len(), 4 * 3 * 3);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn invalid_dimensions_fail_without_a_png_and_patch_the_receipt() {
        let dir = std::env::temp_dir().join(format!("274bot-evidence-bad-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("scratch");
        let png_path = dir.join("bad.png");
        let json_path = dir.join("bad.json");
        let writer = EvidenceWriter::new(8);
        let job = writer.submit(EvidenceRequest {
            png_path: png_path.clone(),
            width: 0,
            height: 0,
            pixels: Vec::new(),
            color: PngColor::Rgba,
            sidecar: Some(EvidenceSidecar {
                path: json_path.clone(),
                receipt: serde_json::json!({"png_error": null}),
                patch_error: Some(Box::new(|receipt, error| {
                    receipt["png_error"] = serde_json::Value::String(error.to_owned());
                })),
            }),
        });
        let outcome = writer
            .wait(&job, Duration::from_secs(10))
            .expect("capture completes");
        assert!(outcome.error.is_some(), "invalid frame must fail");
        assert!(!png_path.exists(), "no PNG is left behind on failure");
        let receipt: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&json_path).expect("failure receipt"))
                .expect("receipt parses");
        assert!(
            receipt["png_error"].is_string(),
            "failure receipt carries the patched error: {receipt}"
        );
        let stats = writer.stats();
        assert_eq!((stats.failed, stats.written), (1, 0));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn full_queue_drops_with_a_counted_error() {
        let writer = EvidenceWriter::with_config(WriterConfig {
            bound: 0,
            submit_wait: Duration::from_millis(1),
            before_write: None,
        });
        let job = writer.submit(EvidenceRequest {
            png_path: PathBuf::from("never.png"),
            width: 2,
            height: 2,
            pixels: test_pixels(2, 2),
            color: PngColor::Rgba,
            sidecar: None,
        });
        let outcome = writer.poll(&job).expect("drop completes inline");
        assert!(outcome.error.is_some(), "drop must carry an error");
        assert_eq!(writer.stats().dropped, 1);
    }

    #[test]
    fn full_queue_submit_wakes_when_worker_pops() {
        use std::collections::HashSet;

        struct WriteGate {
            entered: std::sync::Mutex<HashSet<u64>>,
            entered_cv: std::sync::Condvar,
            released: std::sync::Mutex<HashSet<u64>>,
            released_cv: std::sync::Condvar,
        }

        impl WriteGate {
            fn hook(&self, id: u64) {
                {
                    let mut entered = self.entered.lock().expect("entered");
                    entered.insert(id);
                    self.entered_cv.notify_all();
                }
                let mut released = self.released.lock().expect("released");
                while !released.contains(&id) {
                    released = self.released_cv.wait(released).expect("wait release");
                }
            }

            fn wait_entered(&self, id: u64, timeout: Duration) {
                let deadline = Instant::now() + timeout;
                let mut entered = self.entered.lock().expect("entered");
                while !entered.contains(&id) {
                    let now = Instant::now();
                    assert!(now < deadline, "job {id} never entered write gate");
                    let (guard, wait) = self
                        .entered_cv
                        .wait_timeout(entered, deadline - now)
                        .expect("wait entered");
                    entered = guard;
                    assert!(
                        entered.contains(&id) || !wait.timed_out(),
                        "job {id} never entered write gate"
                    );
                }
            }

            fn release(&self, id: u64) {
                self.released.lock().expect("released").insert(id);
                self.released_cv.notify_all();
            }
        }

        let dir =
            std::env::temp_dir().join(format!("274bot-evidence-wakeup-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("scratch");
        let gate = Arc::new(WriteGate {
            entered: std::sync::Mutex::new(HashSet::new()),
            entered_cv: std::sync::Condvar::new(),
            released: std::sync::Mutex::new(HashSet::new()),
            released_cv: std::sync::Condvar::new(),
        });
        let hook_gate = Arc::clone(&gate);
        let hook: Arc<dyn Fn(u64) + Send + Sync> = Arc::new(move |id| hook_gate.hook(id));
        let writer = Arc::new(EvidenceWriter::with_config(WriterConfig {
            bound: 1,
            submit_wait: Duration::from_secs(10),
            before_write: Some(hook),
        }));
        let request = |name: &str| EvidenceRequest {
            png_path: dir.join(name),
            width: 2,
            height: 2,
            pixels: test_pixels(2, 2),
            color: PngColor::Rgba,
            sidecar: None,
        };

        // 1. Submit A and wait until the worker gates on it.
        let job_a = writer.submit(request("w0.png"));
        let id_a = job_a.id();
        gate.wait_entered(id_a, Duration::from_secs(10));

        // 2. B queues behind the gated A.
        let job_b = writer.submit(request("w1.png"));
        let id_b = job_b.id();
        assert!(writer.poll(&job_b).is_none(), "B must queue, not drop");

        // 3. C blocks on the full queue.
        let (tx, rx) = std::sync::mpsc::channel();
        let writer_c = Arc::clone(&writer);
        let request_c = request("w2.png");
        std::thread::spawn(move || {
            let job_c = writer_c.submit(request_c);
            tx.send(job_c).expect("send C");
        });
        // Let the C thread park inside `submit` while the queue is full.
        std::thread::sleep(Duration::from_millis(300));

        // 4. Release A; the worker completes it, pops B, and gates on B.
        gate.release(id_a);
        gate.wait_entered(id_b, Duration::from_secs(10));

        // 5. The pop must have woken C even though B is still gated.
        let job_c = rx
            .recv_timeout(Duration::from_secs(3))
            .expect("submit blocked on a full queue must enqueue once the worker pops");
        assert!(
            writer.poll(&job_c).is_none(),
            "woken submit must be pending, not dropped"
        );
        assert_eq!(writer.stats().dropped, 0);

        // 6. Drain.
        let id_c = job_c.id();
        gate.release(id_b);
        gate.release(id_c);
        for job in [&job_a, &job_b, &job_c] {
            let outcome = writer
                .wait(job, Duration::from_secs(10))
                .expect("capture completes");
            assert!(outcome.error.is_none(), "unexpected: {:?}", outcome.error);
        }
        assert!(writer.flush(Duration::from_secs(10)));
        assert_eq!(writer.stats().dropped, 0);
        std::fs::remove_dir_all(&dir).ok();
    }
}
