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
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, OnceLock};

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
}

static SINK: OnceLock<&'static dyn Sink> = OnceLock::new();

/// Install the process sink. The first install wins; later calls return
/// false (one structured log per process).
pub fn install_sink(sink: &'static dyn Sink) -> bool {
    SINK.set(sink).is_ok()
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
    logged(category) || debug_enabled()
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
    let sink = SINK.get().filter(|_| emit.category.slot_log());
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

/// Writes one debug line to stderr. A failed write (for example a full disk
/// behind a redirected stderr) is dropped: `eprintln!` would panic instead, and
/// callers log while they hold shared locks, so one failed write would poison
/// them for every slot.
#[cfg(not(test))]
fn write_stderr(line: fmt::Arguments<'_>) {
    write_line(&mut std::io::stderr().lock(), line);
}

fn write_line(out: &mut impl std::io::Write, line: fmt::Arguments<'_>) {
    // A logging failure has nowhere better to go than the sink that failed.
    let _ = writeln!(out, "{line}");
}

#[cfg(test)]
thread_local! {
    /// Stderr lines this test thread emitted.
    static STDERR: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

#[cfg(test)]
fn write_stderr(line: fmt::Arguments<'_>) {
    STDERR.with(|lines| lines.borrow_mut().push(line.to_string()));
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
