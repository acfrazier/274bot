//! Host logging facade: one routing point for every host log line.
//!
//! Host code (host, host-play, script, nav) logs through [`host_log!`](crate::host_log)
//! with a [`Category`]. The category decides where the line goes:
//!
//! - an **allowlisted** category ([`Category::slot_log`]) always reaches the
//!   installed [`Sink`] (the operator's structured slot log in
//!   `frontend-core`), in release builds too;
//! - every category is echoed to stderr under `BOT_DEBUG=1` (or
//!   [`set_debug`]), as the scattered `eprintln!` sites did before;
//! - per-frame/render and per-tick trace categories are stderr-only and
//!   cost nothing (not even formatting) while debug is off.
//!
//! Slot threads bind their slot name once ([`bind_slot`]) and publish the
//! game tick they observe ([`set_tick`]); a line logged on that thread picks
//! both up unless the call names a slot explicitly. Any thread can read a
//! slot's current tick ([`slot_tick`]), so lines recorded off the slot
//! thread carry it too.
//!
//! Every registered secret ([`register_secret`], the account passwords) is
//! redacted ([`redact`]) before a line reaches stderr or any sink.

use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::sync::{Arc, LazyLock, OnceLock};
use std::thread;
use std::time::Duration;

use parking_lot::{Mutex, RwLock};

/// Severity, ordered so a minimum-level filter is `level >= min`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum Level {
    Debug,
    Info,
    Warn,
    Error,
}

impl Level {
    pub const ALL: [Level; 4] = [Level::Debug, Level::Info, Level::Warn, Level::Error];

    /// Fixed-width (5) label for columns and files.
    pub const fn label(self) -> &'static str {
        match self {
            Level::Debug => "debug",
            Level::Info => "info",
            Level::Warn => "warn",
            Level::Error => "error",
        }
    }
}

/// Who produced a log line (the operator-facing source filter).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum Source {
    Script,
    Login,
    Nav,
    Bank,
    Watchdog,
    Host,
}

impl Source {
    pub const ALL: [Source; 6] = [
        Source::Script,
        Source::Login,
        Source::Nav,
        Source::Bank,
        Source::Watchdog,
        Source::Host,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Source::Script => "script",
            Source::Login => "login",
            Source::Nav => "nav",
            Source::Bank => "bank",
            Source::Watchdog => "watchdog",
            Source::Host => "host",
        }
    }

    /// Bit for a [`Source`] set mask.
    pub const fn bit(self) -> u8 {
        1 << self as u8
    }
}

/// Routing category of a host log line. Adding a site means picking one of
/// these, not choosing between `eprintln!` and a UI hook.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Category {
    /// Slot worker lifecycle: thread up, world/handshake, spawn failures,
    /// welcome-screen notices, mainland hop.
    Lifecycle,
    /// Login and startup phases, login responses, queue permits.
    Login,
    /// Script start/compile/failure.
    ScriptLifecycle,
    /// Script watchdog: hung loops and restarts.
    Watchdog,
    /// Random-event handling (maze, lamp).
    RandomEvent,
    /// Route-level navigation events: transport dialogues, choices, gliders,
    /// essence entry, door/walk-through sends.
    NavEvent,
    /// Bank operations a script requested (withdraw, count, note, close).
    BankOp,
    /// Per-frame render/loop timing (hitches, window summaries).
    FrameStats,
    /// Per-tick navigation dumps (follow/walk/transport/troll state,
    /// walk-arm worker detail).
    NavTrace,
    /// Per-request interaction echoes (interact, held item, wear/unequip).
    InteractTrace,
    /// Script-side stderr traces (fire machine state).
    ScriptTrace,
    /// Stderr echo of a fact the slot log already receives through the
    /// operator session (status transitions, staged script lines), so it is
    /// never stored twice.
    Echo,
}

impl Category {
    /// The slot-log allowlist: these categories reach the installed sink in
    /// every build. Everything else is stderr-only under debug.
    pub const fn slot_log(self) -> bool {
        match self {
            Category::Lifecycle
            | Category::Login
            | Category::ScriptLifecycle
            | Category::Watchdog
            | Category::RandomEvent
            | Category::NavEvent
            | Category::BankOp => true,
            Category::FrameStats
            | Category::NavTrace
            | Category::InteractTrace
            | Category::ScriptTrace
            | Category::Echo => false,
        }
    }

    pub const fn source(self) -> Source {
        match self {
            Category::Lifecycle | Category::RandomEvent | Category::FrameStats | Category::Echo => {
                Source::Host
            }
            Category::Login => Source::Login,
            Category::ScriptLifecycle | Category::InteractTrace | Category::ScriptTrace => {
                Source::Script
            }
            Category::Watchdog => Source::Watchdog,
            Category::NavEvent | Category::NavTrace => Source::Nav,
            Category::BankOp => Source::Bank,
        }
    }

    /// Stderr prefix tag.
    pub const fn tag(self) -> &'static str {
        match self {
            Category::Lifecycle => "host",
            Category::Login => "login",
            Category::ScriptLifecycle => "script",
            Category::Watchdog => "watchdog",
            Category::RandomEvent => "random",
            Category::NavEvent => "nav",
            Category::BankOp => "bank",
            Category::FrameStats => "frame",
            Category::NavTrace => "nav-trace",
            Category::InteractTrace => "interact",
            Category::ScriptTrace => "script-trace",
            Category::Echo => "echo",
        }
    }
}

/// One routed line, borrowed for the duration of [`Sink::record`].
#[derive(Debug, Clone, Copy)]
pub struct Record<'a> {
    pub slot: Option<&'a str>,
    pub tick: Option<u32>,
    pub source: Source,
    pub level: Level,
    pub message: &'a str,
}

/// Receiver of allowlisted lines (the structured log store). Called on the
/// logging thread; it must not block on I/O.
pub trait Sink: Send + Sync {
    fn record(&self, record: &Record<'_>);

    /// Opt this test sink into NavTrace records. The sink must retain only
    /// required records or use a capped buffer; production routing is unchanged.
    #[cfg(feature = "test-support")]
    fn test_nav_trace_enabled(&self) -> bool {
        false
    }
}

/// Maximum number of stderr lines reserved, queued or being written.
pub const STDERR_QUEUE_LINES: usize = 256;
/// Maximum bytes in one stderr line, excluding its newline.
pub const STDERR_LINE_BYTES: usize = 1_024;
/// Most lines one log entry may become. A longer entry is refused whole and
/// counted, never cut.
pub const STDERR_ENTRY_LINES: usize = 16;
/// Width of the `[i/n] ` marker that numbers the lines of a split entry
/// (`[16/16] ` at most).
const STDERR_MARKER_BYTES: usize = 8;
/// Bytes of an entry that one numbered line carries.
const STDERR_PIECE_BYTES: usize = STDERR_LINE_BYTES - STDERR_MARKER_BYTES;
/// Largest entry in bytes. A split at character boundaries leaves every
/// piece but the last at least `STDERR_PIECE_BYTES - 3` long, so an entry of
/// this size needs at most [`STDERR_ENTRY_LINES`] lines.
const STDERR_ENTRY_BYTES: usize = STDERR_ENTRY_LINES * (STDERR_PIECE_BYTES - 3);
/// How long process shutdown waits for the writer to drain accepted entries.
const STDERR_SHUTDOWN_WAIT: Duration = Duration::from_millis(250);
/// A numbered, packed log line (at most [`LINE_BYTES`]) plus its `[tag slot] `
/// head always fits one stderr line, so a packed line is never split.
const _: () = assert!(LINE_BYTES * 2 <= STDERR_LINE_BYTES);

/// One log entry ready for the writer: one stderr line, or up to
/// [`STDERR_ENTRY_LINES`] numbered lines joined by `\n`. The writer delivers
/// or drops it as a whole, so the lines of one entry stay together.
struct Entry {
    text: String,
    lines: usize,
}

/// The shared counters of one stderr queue.
struct Counters {
    /// The line cap: [`STDERR_QUEUE_LINES`] in the product.
    cap: usize,
    /// Lines reserved by producers, queued, or being written. Never above
    /// [`STDERR_QUEUE_LINES`], so the line storage never exceeds
    /// `STDERR_QUEUE_LINES * STDERR_LINE_BYTES`.
    queued: AtomicUsize,
    /// Entries reserved, queued or being written.
    pending: AtomicUsize,
    /// Entries refused, failed, or lost at exit.
    dropped: AtomicU64,
}

impl Counters {
    fn new(cap: usize) -> Self {
        Self {
            cap,
            queued: AtomicUsize::new(0),
            pending: AtomicUsize::new(0),
            dropped: AtomicU64::new(0),
        }
    }

    /// Add `extra` lines if the queue still has room for all of them.
    fn try_add_lines(&self, extra: usize) -> bool {
        let mut queued = self.queued.load(Ordering::Acquire);
        loop {
            if queued + extra > self.cap {
                return false;
            }
            match self.queued.compare_exchange_weak(
                queued,
                queued + extra,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => return true,
                Err(actual) => queued = actual,
            }
        }
    }

    /// The writer is done with `entry`: release its lines, and count it
    /// when the write failed.
    fn finish(&self, entry: &Entry, write_failed: bool) {
        if write_failed {
            self.dropped.fetch_add(1, Ordering::Relaxed);
        }
        self.queued.fetch_sub(entry.lines, Ordering::AcqRel);
        self.pending.fetch_sub(1, Ordering::AcqRel);
    }
}

/// The lines one producer holds for an entry. Dropping an uncommitted
/// reservation releases them.
struct Reservation<'a> {
    counters: &'a Counters,
    lines: usize,
    committed: bool,
}

impl Reservation<'_> {
    /// Hold `extra` more lines, all or none.
    fn grow(&mut self, extra: usize) -> bool {
        if self.counters.try_add_lines(extra) {
            self.lines += extra;
            true
        } else {
            false
        }
    }

    fn commit(mut self) {
        self.committed = true;
    }
}

impl Drop for Reservation<'_> {
    fn drop(&mut self) {
        if !self.committed {
            self.counters.queued.fetch_sub(self.lines, Ordering::AcqRel);
            self.counters.pending.fetch_sub(1, Ordering::AcqRel);
        }
    }
}

/// The detached writer thread, and the signal that it has ended.
struct Worker {
    handle: thread::JoinHandle<()>,
    /// Disconnects when the thread ends.
    ended: Receiver<()>,
}

/// Process-owned, bounded stderr output. Producers do not wait for queue
/// space, for the sender lock or for the sink; a refused entry is counted.
struct StderrOutput {
    tx: Mutex<Option<SyncSender<Entry>>>,
    counters: Arc<Counters>,
    worker: Mutex<Option<Worker>>,
}

impl StderrOutput {
    fn channel() -> (Self, Receiver<Entry>) {
        Self::channel_with_cap(STDERR_QUEUE_LINES)
    }

    /// A queue of `cap` lines. The product uses [`STDERR_QUEUE_LINES`]; the
    /// conformance tests use a small cap so the cap binds.
    fn channel_with_cap(cap: usize) -> (Self, Receiver<Entry>) {
        let (tx, rx) = mpsc::sync_channel(cap);
        (
            Self {
                tx: Mutex::new(Some(tx)),
                counters: Arc::new(Counters::new(cap)),
                worker: Mutex::new(None),
            },
            rx,
        )
    }

    fn start() -> Self {
        Self::start_with(|rx, counters| run_process_stderr_writer(rx, counters, || {}))
    }

    /// An output whose detached writer thread runs `run`. A thread that
    /// cannot start leaves the queue closed, so every entry is counted.
    fn start_with(run: impl FnOnce(Receiver<Entry>, Arc<Counters>) + Send + 'static) -> Self {
        let (output, rx) = Self::channel();
        let counters = Arc::clone(&output.counters);
        let (ended_tx, ended) = mpsc::sync_channel::<()>(0);
        match thread::Builder::new()
            .name("host-log".into())
            .spawn(move || {
                run(rx, counters);
                drop(ended_tx);
            }) {
            Ok(handle) => {
                *output.worker.lock() = Some(Worker { handle, ended });
                output
            }
            Err(_) => {
                output.tx.lock().take();
                output
            }
        }
    }

    /// A sender for one entry, or `None` when the sender lock is busy or the
    /// queue is closed.
    fn sender(&self) -> Option<SyncSender<Entry>> {
        self.tx.try_lock().and_then(|tx| tx.as_ref().cloned())
    }

    /// The first line of an entry, or `None` when the queue is full. No
    /// allocation happens before this succeeds.
    fn reserve_first(&self) -> Option<Reservation<'_>> {
        if !self.counters.try_add_lines(1) {
            return None;
        }
        self.counters.pending.fetch_add(1, Ordering::AcqRel);
        Some(Reservation {
            counters: &self.counters,
            lines: 1,
            committed: false,
        })
    }

    fn enqueue(&self, line: fmt::Arguments<'_>) {
        let mut steps = Enqueue::new(self);
        while steps.step(line) {}
    }

    fn note_drop(&self) {
        self.counters.dropped.fetch_add(1, Ordering::Relaxed);
    }

    /// Count a refused entry and end its sequence.
    fn refused<'a>(&self) -> EnqueuePhase<'a> {
        self.note_drop();
        EnqueuePhase::Done
    }

    /// Close the queue: no new entry is accepted, and the writer ends once it
    /// has taken what was accepted.
    fn close(&self) {
        self.tx.lock().take();
    }

    /// Give up on the writer: every entry still pending is counted as dropped.
    /// Returns that count.
    fn give_up(&self) -> u64 {
        let lost = self.counters.pending.load(Ordering::Acquire) as u64;
        self.counters.dropped.fetch_add(lost, Ordering::Relaxed);
        lost
    }

    /// Close the queue and wait up to `wait` for the writer to drain it.
    /// Returns the entries given up: with a stalled sink the writer cannot
    /// finish, so the wait ends and every entry still pending is counted as
    /// dropped. Never waits for the sink beyond `wait`.
    fn shutdown(&self, wait: Duration) -> u64 {
        self.close();
        let Some(worker) = self.worker.lock().take() else {
            return 0;
        };
        match worker.ended.recv_timeout(wait) {
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                let _ = worker.handle.join();
                0
            }
            Ok(()) | Err(mpsc::RecvTimeoutError::Timeout) => self.give_up(),
        }
    }

    fn dropped(&self) -> u64 {
        self.counters.dropped.load(Ordering::Relaxed)
    }
}

/// Where one entry is on its way into the queue. Each step is one atomic
/// operation of the producer; `tools/hostlog-model` has the same phases.
enum EnqueuePhase<'a> {
    /// Nothing held.
    Start,
    /// A sender clone is held.
    Cloned(SyncSender<Entry>),
    /// The first line is reserved.
    Reserved(SyncSender<Entry>, Reservation<'a>),
    /// The entry is formatted.
    Formatted(SyncSender<Entry>, Reservation<'a>, Entry),
    /// All the entry's lines are reserved.
    Held(SyncSender<Entry>, Reservation<'a>, Entry),
    Done,
}

/// The producer side of the queue as explicit steps. [`StderrOutput::enqueue`]
/// runs them to the end; the conformance tests step them one at a time and
/// compare each state with the model. No step waits: a refusal drops the
/// entry, counts it, and ends the sequence.
struct Enqueue<'a> {
    output: &'a StderrOutput,
    phase: EnqueuePhase<'a>,
}

impl<'a> Enqueue<'a> {
    fn new(output: &'a StderrOutput) -> Self {
        Self {
            output,
            phase: EnqueuePhase::Start,
        }
    }

    /// Run one step; `line` is the entry, formatted in the `Reserved` step.
    /// Returns false once the entry is finished.
    fn step(&mut self, line: fmt::Arguments<'_>) -> bool {
        let output = self.output;
        self.phase = match std::mem::replace(&mut self.phase, EnqueuePhase::Done) {
            EnqueuePhase::Start => match output.sender() {
                Some(tx) => EnqueuePhase::Cloned(tx),
                None => output.refused(),
            },
            EnqueuePhase::Cloned(tx) => match output.reserve_first() {
                Some(reservation) => EnqueuePhase::Reserved(tx, reservation),
                None => output.refused(),
            },
            EnqueuePhase::Reserved(tx, reservation) => match format_entry(line) {
                Some(entry) => EnqueuePhase::Formatted(tx, reservation, entry),
                None => output.refused(),
            },
            EnqueuePhase::Formatted(tx, mut reservation, entry) => {
                if entry.lines > 1 && !reservation.grow(entry.lines - 1) {
                    output.refused()
                } else {
                    EnqueuePhase::Held(tx, reservation, entry)
                }
            }
            EnqueuePhase::Held(tx, reservation, entry) => {
                if tx.try_send(entry).is_ok() {
                    reservation.commit();
                } else {
                    output.note_drop();
                }
                EnqueuePhase::Done
            }
            EnqueuePhase::Done => EnqueuePhase::Done,
        };
        !matches!(self.phase, EnqueuePhase::Done)
    }
}

static STDERR_OUTPUT: LazyLock<StderrOutput> = LazyLock::new(StderrOutput::start);

fn stderr_output() -> &'static StderrOutput {
    &STDERR_OUTPUT
}

/// Start the process-owned stderr writer before slot workers start.
///
/// The writer is also started lazily if a host logs before process bootstrap.
pub fn start_stderr_writer() {
    let _ = stderr_output();
}

/// Close the stderr queue at process exit and wait a short, fixed time for the
/// writer to write what it accepted. A sink that does not finish in that time
/// (a pipe nobody reads) is abandoned: the entries still pending are counted
/// in [`stderr_dropped`] and returned. A working sink returns 0.
pub fn shutdown_stderr_writer() -> u64 {
    LazyLock::get(&STDERR_OUTPUT).map_or(0, |output| output.shutdown(STDERR_SHUTDOWN_WAIT))
}

/// Number of log entries dropped because the sender lock was busy, the queue
/// was full or closed, the entry was longer than [`STDERR_ENTRY_LINES`] lines,
/// the writer could not start, a stderr write failed, or the process exited
/// before the writer finished. An entry that needs several lines counts once.
pub fn stderr_dropped() -> u64 {
    LazyLock::get(&STDERR_OUTPUT).map_or(0, StderrOutput::dropped)
}

#[cfg(feature = "test-support")]
/// Test hook that fills an isolated queue and records one process drop.
#[doc(hidden)]
pub fn test_fill_stderr_queue_and_drop_one_line() -> u64 {
    let global = stderr_output();
    let (output, receiver) = StderrOutput::channel();
    for _ in 0..STDERR_QUEUE_LINES {
        output.enqueue(format_args!("test"));
    }
    output.enqueue(format_args!("overflow"));
    global
        .counters
        .dropped
        .fetch_add(output.dropped(), Ordering::Relaxed);
    drop(output);
    drop(receiver);
    global.dropped()
}

/// Collects one formatted entry and refuses one over [`STDERR_ENTRY_BYTES`].
struct BoundedEntry {
    text: String,
}

impl fmt::Write for BoundedEntry {
    fn write_str(&mut self, value: &str) -> fmt::Result {
        if value.len() > STDERR_ENTRY_BYTES - self.text.len() {
            return Err(fmt::Error);
        }
        self.text.push_str(value);
        Ok(())
    }
}

/// Format one log entry. A text that fits one line stays one line. A longer
/// one is split at character boundaries into lines of at most
/// [`STDERR_LINE_BYTES`] bytes, each numbered `[i/n] `, and nothing is cut.
/// `None`: the text is over [`STDERR_ENTRY_BYTES`] (a formatting error counts
/// the same way), so the caller counts the whole entry as dropped.
fn format_entry(line: fmt::Arguments<'_>) -> Option<Entry> {
    let mut whole = BoundedEntry {
        text: String::with_capacity(STDERR_LINE_BYTES),
    };
    fmt::write(&mut whole, line).ok()?;
    let text = whole.text;
    if text.len() <= STDERR_LINE_BYTES {
        return Some(Entry { text, lines: 1 });
    }
    let lines = split_lines(&text).count();
    let mut out = String::with_capacity(text.len() + lines * (STDERR_MARKER_BYTES + 1));
    for (index, piece) in split_lines(&text).enumerate() {
        if index > 0 {
            out.push('\n');
        }
        out.push('[');
        out.push_str(&(index + 1).to_string());
        out.push('/');
        out.push_str(&lines.to_string());
        out.push_str("] ");
        out.push_str(piece);
    }
    Some(Entry { text: out, lines })
}

/// `text` in pieces of at most [`STDERR_PIECE_BYTES`] bytes, cut only at
/// character boundaries.
fn split_lines(text: &str) -> impl Iterator<Item = &str> {
    let mut rest = text;
    std::iter::from_fn(move || {
        if rest.is_empty() {
            return None;
        }
        let mut end = STDERR_PIECE_BYTES.min(rest.len());
        while !rest.is_char_boundary(end) {
            end -= 1;
        }
        let (piece, tail) = rest.split_at(end);
        rest = tail;
        Some(piece)
    })
}

fn run_stderr_writer(
    rx: Receiver<Entry>,
    counters: Arc<Counters>,
    mut on_wait: impl FnMut(),
    mut write: impl FnMut(&str) -> std::io::Result<()>,
) {
    loop {
        on_wait();
        let Ok(entry) = rx.recv() else {
            break;
        };
        let failed = write(&entry.text).is_err();
        counters.finish(&entry, failed);
    }
}

fn run_process_stderr_writer(rx: Receiver<Entry>, counters: Arc<Counters>, on_wait: impl FnMut()) {
    let stderr = std::io::stderr();
    run_stderr_writer(rx, counters, on_wait, |text| {
        let mut stderr = stderr.lock();
        write_line(&mut stderr, text)
    });
}

static SINK: OnceLock<&'static dyn Sink> = OnceLock::new();

/// Install the process sink. The first install wins; later calls return
/// false (one structured log per process).
pub fn install_sink(sink: &'static dyn Sink) -> bool {
    SINK.set(sink).is_ok()
}

#[inline]
fn routed_sink(sink: &dyn Sink, category: Category) -> Option<&dyn Sink> {
    if category.slot_log() {
        return Some(sink);
    }
    #[cfg(feature = "test-support")]
    if category == Category::NavTrace && sink.test_nav_trace_enabled() {
        return Some(sink);
    }
    None
}

#[inline]
fn selected_sink(category: Category) -> Option<&'static dyn Sink> {
    routed_sink(*SINK.get()?, category)
}

/// Deliver a record straight to the installed sink (front-end sources that
/// already know their slot, level and source), redacted like [`emit`].
pub fn record(record: &Record<'_>) {
    if let Some(sink) = SINK.get() {
        let message = redact(record.message);
        sink.record(&Record {
            message: &message,
            ..*record
        });
    }
}

/// Replacement for every redacted secret.
pub const REDACTED: &str = "***";

static SECRETS: RwLock<Vec<Box<str>>> = RwLock::new(Vec::new());
static HAS_SECRETS: AtomicBool = AtomicBool::new(false);

/// Never let `secret` reach stderr or a sink. Every non-empty password is
/// registered, however short and even when it equals its username: a short
/// password blanks those characters in messages (safety over readability),
/// and the slot column still names the account, which is not a secret.
pub fn register_secret(secret: &str) {
    if secret.is_empty() {
        return;
    }
    let mut secrets = SECRETS.write();
    if !secrets.iter().any(|s| &**s == secret) {
        secrets.push(secret.into());
        HAS_SECRETS.store(true, Ordering::Release);
    }
}

/// `message` with every occurrence of every registered secret replaced by
/// [`REDACTED`]. Matches are found in the original text (overlapping ones
/// included) and merged before replacing, so one secret containing or
/// overlapping another never leaves part of either behind. Borrowed, with
/// no allocation, when nothing matches.
pub fn redact(message: &str) -> Cow<'_, str> {
    if !HAS_SECRETS.load(Ordering::Acquire) {
        return Cow::Borrowed(message);
    }
    let secrets = SECRETS.read();
    let mut spans: Vec<(usize, usize)> = Vec::new();
    for secret in secrets.iter() {
        let mut from = 0;
        while let Some(at) = message[from..].find(&**secret) {
            let start = from + at;
            spans.push((start, start + secret.len()));
            // Step one character, not the whole match, to catch overlaps.
            from = start + message[start..].chars().next().map_or(1, char::len_utf8);
        }
    }
    drop(secrets);
    if spans.is_empty() {
        return Cow::Borrowed(message);
    }
    spans.sort_unstable();
    let mut out = String::with_capacity(message.len());
    let mut copied = 0;
    let mut i = 0;
    while i < spans.len() {
        let (start, mut end) = spans[i];
        i += 1;
        while i < spans.len() && spans[i].0 <= end {
            end = end.max(spans[i].1);
            i += 1;
        }
        out.push_str(&message[copied..start]);
        out.push_str(REDACTED);
        copied = end;
    }
    out.push_str(&message[copied..]);
    Cow::Owned(out)
}

static DEBUG: AtomicBool = AtomicBool::new(false);

/// Force debug echo on/off (host-play `--debug`); `BOT_DEBUG=1` enables it
/// regardless.
pub fn set_debug(enabled: bool) {
    DEBUG.store(enabled, Ordering::Relaxed);
}

static DEBUG_ENV: LazyLock<bool> =
    LazyLock::new(|| std::env::var("BOT_DEBUG").is_ok_and(|v| v == "1"));

/// Debug echo is on when `BOT_DEBUG=1` (read once) or [`set_debug`] ran.
pub fn debug_enabled() -> bool {
    DEBUG.load(Ordering::Relaxed) || *DEBUG_ENV
}

/// Whether a line in `category` goes anywhere right now. The macro checks
/// this before formatting, so debug-only categories are free while off.
#[inline]
pub fn enabled(category: Category) -> bool {
    selected_sink(category).is_some() || debug_enabled()
}

/// Whether a line in `category` reaches the in-app log ring: a sink is
/// installed and the category is on the slot-log allowlist. Debug stderr
/// and the per-session file do not count: a front end that points the
/// operator at "the log" asks this, so the pointer holds with file logging
/// off (the default).
#[inline]
pub fn logged(category: Category) -> bool {
    category.slot_log() && SINK.get().is_some()
}

/// Longest summary a front end shows for a notice, in characters. The cause
/// and the count fit before the cut.
pub const SUMMARY_CHARS: usize = 100;

/// Longest notice line a front end shows, in characters: a summary, the
/// ` (see log)` pointer and a margin. At four bytes a character this is 448
/// bytes, which one packed log line holds.
pub const LINE_CHARS: usize = 112;

/// `text` cut to at most `max` characters, ending with `…` when it was cut.
/// Never splits a code point. Borrowed when the text already fits.
pub fn cut_chars(text: &str, max: usize) -> Cow<'_, str> {
    let Some((end, _)) = text.char_indices().nth(max) else {
        return Cow::Borrowed(text);
    };
    let keep = max.saturating_sub(1);
    let at = text.char_indices().nth(keep).map_or(end, |(at, _)| at);
    let mut out = String::with_capacity(at + '…'.len_utf8());
    out.push_str(&text[..at]);
    out.push('…');
    Cow::Owned(out)
}

/// Longest message the structured log store keeps, in bytes. The store cuts
/// a longer line and ends it with `…`; [`pack`] keeps every line under it.
pub const LINE_BYTES: usize = 512;

/// Longest packed line, in bytes, head and marker included: [`LINE_BYTES`]
/// minus a margin, because redaction can lengthen a line.
pub const PACK_BYTES: usize = 480;

/// Width of the widest marker this packer writes (`[100/100] `).
const PACK_MARKER_BYTES: usize = "[100/100] ".len();

/// The payload never falls below this, however long the head.
const PACK_MIN_PAYLOAD: usize = 64;

/// `items` as log lines that each fit [`PACK_BYTES`], loses nothing.
///
/// The items are flattened (`\n`, `\r` and `\t` become one space, as the
/// store does), joined with `"; "` and cut into lines. A line ends after a
/// `"; "` or `", "` when one fits, so an item (and a comma-separated part of
/// one) stays whole where it can; text longer than a line without either is
/// cut on a `char` boundary, never inside a code point. The
/// payloads of all lines, in order and without their markers, concatenate to
/// exactly the flattened join. Every line of a split result starts with
/// `[i/n] `; one line has no marker.
pub fn pack<S: AsRef<str>>(items: &[S]) -> Vec<String> {
    pack_after_head(items, 0)
}

fn pack_after_head<S: AsRef<str>>(items: &[S], head_bytes: usize) -> Vec<String> {
    let limit = PACK_BYTES
        .saturating_sub(PACK_MARKER_BYTES + head_bytes)
        .max(PACK_MIN_PAYLOAD);
    let mut flat = String::new();
    for (index, item) in items.iter().enumerate() {
        if index > 0 {
            flat.push_str("; ");
        }
        flat.extend(item.as_ref().chars().map(|c| {
            if matches!(c, '\n' | '\r' | '\t') {
                ' '
            } else {
                c
            }
        }));
    }
    let mut chunks: Vec<&str> = Vec::new();
    let mut rest = flat.as_str();
    while !rest.is_empty() {
        let end = if rest.len() <= limit {
            rest.len()
        } else {
            let mut cut = limit;
            while !rest.is_char_boundary(cut) {
                cut -= 1;
            }
            // End the line after the last `"; "` or `", "` that fits, so an
            // item stays whole where the text has any.
            rest.as_bytes()[..cut]
                .windows(2)
                .rposition(|pair| matches!(pair, [b';' | b',', b' ']))
                .map_or(cut, |at| at + 2)
        };
        let (chunk, tail) = rest.split_at(end);
        chunks.push(chunk);
        rest = tail;
    }
    let count = chunks.len();
    if count <= 1 {
        return chunks.into_iter().map(str::to_owned).collect();
    }
    chunks
        .into_iter()
        .enumerate()
        .map(|(index, chunk)| format!("[{}/{count}] {chunk}", index + 1))
        .collect()
}

/// One [`emit`] per line of [`pack`]`(items)`, each prefixed with `head`, all
/// in the same category, level and slot, in order. Nothing is written
/// when the category goes nowhere.
pub fn emit_packed<S: AsRef<str>>(to: Emit<'_>, head: &str, items: &[S]) {
    if !(to.always_stderr || enabled(to.category)) {
        return;
    }
    for line in pack_after_head(items, head.len()) {
        emit(to, format_args!("{head}{line}"));
    }
}

/// A slot's published tick; [`NO_TICK`] until its thread observes one.
type TickCell = Arc<AtomicU64>;
const NO_TICK: u64 = u64::MAX;

/// Current tick per slot name, shared with every thread. Bounded by the
/// slot names this process ever bound.
static TICKS: LazyLock<Mutex<HashMap<Arc<str>, TickCell>>> = LazyLock::new(Mutex::default);

struct Bound {
    slot: Arc<str>,
    tick: TickCell,
}

thread_local! {
    static SLOT: RefCell<Option<Bound>> = const { RefCell::new(None) };
}

/// Bind this thread to `slot`: lines logged here without an explicit slot
/// belong to it. Slot worker threads call this once at start; the slot's
/// tick is unknown until the next [`set_tick`].
pub fn bind_slot(slot: &str) {
    let (slot, tick) = {
        let mut ticks = TICKS.lock();
        match ticks.get_key_value(slot) {
            Some((name, tick)) => (Arc::clone(name), Arc::clone(tick)),
            None => {
                let name: Arc<str> = Arc::from(slot);
                let tick = Arc::new(AtomicU64::new(NO_TICK));
                ticks.insert(Arc::clone(&name), Arc::clone(&tick));
                (name, tick)
            }
        }
    };
    tick.store(NO_TICK, Ordering::Relaxed);
    SLOT.with(|s| *s.borrow_mut() = Some(Bound { slot, tick }));
}

/// Publish the game tick this slot thread observes (one atomic store).
#[inline]
pub fn set_tick(tick: u32) {
    SLOT.with(|s| {
        if let Some(bound) = s.borrow().as_ref() {
            bound.tick.store(u64::from(tick), Ordering::Relaxed);
        }
    });
}

/// The tick `slot`'s thread last published, from any thread.
pub fn slot_tick(slot: &str) -> Option<u32> {
    let tick = TICKS.lock().get(slot)?.load(Ordering::Relaxed);
    u32::try_from(tick).ok()
}

fn bound_tick(bound: &Bound) -> Option<u32> {
    u32::try_from(bound.tick.load(Ordering::Relaxed)).ok()
}

/// Where a line goes regardless of debug (`always_stderr` keeps the few
/// historically unconditional stderr lines on stderr).
#[derive(Debug, Clone, Copy)]
pub struct Emit<'a> {
    pub category: Category,
    pub level: Level,
    pub slot: Option<&'a str>,
    pub always_stderr: bool,
}

/// Macro back end: format once, echo to stderr under debug, route
/// allowlisted categories to the sink.
pub fn emit(emit: Emit<'_>, args: fmt::Arguments<'_>) {
    let debug = emit.always_stderr || debug_enabled();
    let sink = selected_sink(emit.category);
    if !debug && sink.is_none() {
        return;
    }
    let owned;
    let raw = match args.as_str() {
        Some(s) => s,
        None => {
            owned = args.to_string();
            owned.as_str()
        }
    };
    let message = redact(raw);
    let message = &*message;
    // The bound slot, and its tick when the line belongs to it.
    let (bound, bound_tick) = SLOT.with(|s| match s.borrow().as_ref() {
        Some(b) if emit.slot.is_none_or(|slot| slot == &*b.slot) => {
            (Some(Arc::clone(&b.slot)), bound_tick(b))
        }
        _ => (None, None),
    });
    let slot = emit.slot.or(bound.as_deref());
    if debug {
        match slot {
            Some(slot) => write_stderr(format_args!("[{} {slot}] {message}", emit.category.tag())),
            None => write_stderr(format_args!("[{}] {message}", emit.category.tag())),
        }
    }
    if let Some(sink) = sink {
        sink.record(&Record {
            slot,
            tick: bound_tick.or_else(|| slot.and_then(slot_tick)),
            source: emit.category.source(),
            level: emit.level,
            message,
        });
    }
}

/// Enqueue one bounded stderr line. Only the worker performs I/O.
#[cfg(not(test))]
fn write_stderr(line: fmt::Arguments<'_>) {
    stderr_output().enqueue(line);
}

fn write_line(out: &mut impl std::io::Write, line: &str) -> std::io::Result<()> {
    writeln!(out, "{line}")
}

#[cfg(test)]
thread_local! {
    /// Stderr lines this test thread emitted.
    static STDERR: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

#[cfg(test)]
fn write_stderr(line: fmt::Arguments<'_>) {
    // A refused (oversized) entry reaches no sink, as in production.
    if let Some(entry) = format_entry(line) {
        STDERR.with(|lines| {
            lines
                .borrow_mut()
                .extend(entry.text.lines().map(str::to_owned))
        });
    }
}

/// Log one host line through the routing facade.
///
/// ```ignore
/// host_log!(Category::Login, Level::Info, "startup phase {phase:?}");
/// host_log!(Category::Lifecycle, Level::Warn, slot = name, "maininit failed");
/// host_log!(stderr; Category::Lifecycle, Level::Error, "{error}");
/// ```
///
/// Arguments are only evaluated when the category goes somewhere.
#[macro_export]
macro_rules! host_log {
    (stderr; $cat:expr, $level:expr, slot = $slot:expr, $($arg:tt)+) => {
        $crate::hostlog::emit(
            $crate::hostlog::Emit { category: $cat, level: $level, slot: Some($slot), always_stderr: true },
            format_args!($($arg)+),
        )
    };
    (stderr; $cat:expr, $level:expr, $($arg:tt)+) => {
        $crate::hostlog::emit(
            $crate::hostlog::Emit { category: $cat, level: $level, slot: None, always_stderr: true },
            format_args!($($arg)+),
        )
    };
    ($cat:expr, $level:expr, slot = $slot:expr, $($arg:tt)+) => {{
        let category: $crate::hostlog::Category = $cat;
        if $crate::hostlog::enabled(category) {
            $crate::hostlog::emit(
                $crate::hostlog::Emit { category, level: $level, slot: Some($slot), always_stderr: false },
                format_args!($($arg)+),
            )
        }
    }};
    ($cat:expr, $level:expr, $($arg:tt)+) => {{
        let category: $crate::hostlog::Category = $cat;
        if $crate::hostlog::enabled(category) {
            $crate::hostlog::emit(
                $crate::hostlog::Emit { category, level: $level, slot: None, always_stderr: false },
                format_args!($($arg)+),
            )
        }
    }};
}

#[cfg(test)]
#[path = "hostlog_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "hostlog_conformance_tests.rs"]
mod conformance_tests;
