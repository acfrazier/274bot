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
//! both up unless the call names a slot explicitly.

use std::cell::{Cell, RefCell};
use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock, OnceLock};

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
/// already know their slot, level and source).
pub fn record(record: &Record<'_>) {
    if let Some(sink) = SINK.get() {
        sink.record(record);
    }
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
    (category.slot_log() && SINK.get().is_some()) || debug_enabled()
}

thread_local! {
    static SLOT: RefCell<Option<Arc<str>>> = const { RefCell::new(None) };
    static TICK: Cell<Option<u32>> = const { Cell::new(None) };
}

/// Bind this thread to `slot`: lines logged here without an explicit slot
/// belong to it. Slot worker threads call this once at start.
pub fn bind_slot(slot: &str) {
    SLOT.with(|s| *s.borrow_mut() = Some(Arc::from(slot)));
    TICK.with(|t| t.set(None));
}

/// Publish the game tick this thread last observed.
#[inline]
pub fn set_tick(tick: u32) {
    TICK.with(|t| t.set(Some(tick)));
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
    let message = match args.as_str() {
        Some(s) => s,
        None => {
            owned = args.to_string();
            owned.as_str()
        }
    };
    let bound = if emit.slot.is_none() {
        SLOT.with(|s| s.borrow().clone())
    } else {
        None
    };
    let slot = emit.slot.or(bound.as_deref());
    if debug {
        match slot {
            Some(slot) => eprintln!("[{} {slot}] {message}", emit.category.tag()),
            None => eprintln!("[{}] {message}", emit.category.tag()),
        }
    }
    if let Some(sink) = sink {
        sink.record(&Record {
            slot,
            tick: TICK.with(Cell::get),
            source: emit.category.source(),
            level: emit.level,
            message,
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
