//! The operator log shared by the panel and the TUI.
//!
//! One process-wide [`LogStore`] holds a bounded ring per slot plus a
//! process ring for lines no slot owns. Host code reaches it through the
//! `api::hostlog` routing facade (allowlisted categories only); the operator
//! session adds status transitions and script lines; front ends add their
//! own few sources (audio, vault errors). Front ends read it through a
//! [`LogView`]: a filtered, incrementally refreshed copy whose steady-state
//! refresh is one atomic load.
//!
//! Every message passes through secret redaction before it is stored or
//! written, so passwords never reach a view, a copy, a save or the
//! per-session file.

use std::collections::{HashMap, VecDeque};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Once};
use std::time::SystemTime;

pub use api::hostlog::{Level, Record, Source};
use parking_lot::Mutex;

use crate::log_file::SessionLogFile;

/// Entries kept per slot ring.
pub const SLOT_RING_CAP: usize = 500;
/// Entries kept in the process ring (lines no slot owns).
pub const PROCESS_RING_CAP: usize = 500;
/// Longest stored message in bytes; longer lines keep their head and end
/// in `…`.
pub const MESSAGE_CAP: usize = 512;
/// Rows a [`LogScope::All`] view keeps (the newest across every ring).
pub const ALL_VIEW_CAP: usize = 1_000;
/// Shortest password the store redacts. Shorter secrets would blank out
/// ordinary words; the vault never holds one that short for a real account.
const MIN_SECRET_LEN: usize = 4;
const REDACTED: &str = "***";

/// Local wall time `HH:MM:SS.mmm`, stored inline so rendering allocates
/// nothing.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Clock([u8; 12]);

impl Clock {
    pub fn as_str(&self) -> &str {
        // Only ASCII digits, ':' and '.' are ever written.
        std::str::from_utf8(&self.0).unwrap_or("??:??:??.???")
    }

    fn from_local(t: &LocalTime) -> Self {
        let mut b = *b"00:00:00.000";
        put2(&mut b[0..2], t.hour);
        put2(&mut b[3..5], t.minute);
        put2(&mut b[6..8], t.second);
        b[9] = b'0' + (t.millis / 100 % 10) as u8;
        b[10] = b'0' + (t.millis / 10 % 10) as u8;
        b[11] = b'0' + (t.millis % 10) as u8;
        Clock(b)
    }
}

impl std::fmt::Debug for Clock {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

fn put2(out: &mut [u8], v: u32) {
    out[0] = b'0' + (v / 10 % 10) as u8;
    out[1] = b'0' + (v % 10) as u8;
}

/// One structured log line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogEntry {
    /// Process-wide order; strictly increasing across every ring.
    pub seq: u64,
    pub clock: Clock,
    /// Game tick the slot had observed, when the line came from a slot
    /// thread or a status poll that knows it.
    pub tick: Option<u32>,
    pub level: Level,
    pub source: Source,
    /// `None` for process lines.
    pub slot: Option<Arc<str>>,
    pub message: Box<str>,
}

impl LogEntry {
    /// Heap plus inline bytes this entry holds (the slot name is shared by
    /// every entry of its ring and counted once per ring).
    fn heap_bytes(&self) -> usize {
        self.message.len()
    }

    /// `HH:MM:SS.mmm t=<tick> <slot> <level> <source> <message>`.
    pub fn write_line(&self, out: &mut String) {
        out.push_str(self.clock.as_str());
        match self.tick {
            Some(tick) => {
                let _ = write!(out, " t={tick:<6}");
            }
            None => out.push_str(" t=-     "),
        }
        let _ = write!(
            out,
            " {:<12} {:<5} {:<8} {}",
            self.slot.as_deref().unwrap_or("*"),
            self.level.label(),
            self.source.label(),
            self.message
        );
    }
}

#[derive(Debug, Default)]
struct Ring {
    entries: VecDeque<LogEntry>,
    /// Entries evicted from the front since the ring was created.
    dropped: u64,
}

impl Ring {
    fn push(&mut self, entry: LogEntry, cap: usize) {
        if self.entries.len() == cap {
            self.entries.pop_front();
            self.dropped += 1;
        }
        self.entries.push_back(entry);
    }
}

/// Where a view reads from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LogScope {
    /// One slot's ring.
    Slot(String),
    /// Lines no slot owns (vault, catalog, process-wide errors).
    Process,
    /// Every ring, newest [`ALL_VIEW_CAP`] rows in order.
    All,
}

/// Which rows a view shows. Text search is ASCII case-insensitive over the
/// message and the slot name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogFilter {
    pub min_level: Level,
    /// [`Source::bit`] mask; all bits set shows every source.
    pub sources: u8,
    pub text: String,
}

impl Default for LogFilter {
    fn default() -> Self {
        Self {
            min_level: Level::Info,
            sources: ALL_SOURCES,
            text: String::new(),
        }
    }
}

/// Every [`Source`] bit.
pub const ALL_SOURCES: u8 = {
    let mut mask = 0;
    let mut i = 0;
    while i < Source::ALL.len() {
        mask |= Source::ALL[i].bit();
        i += 1;
    }
    mask
};

impl LogFilter {
    pub fn matches(&self, entry: &LogEntry) -> bool {
        entry.level >= self.min_level
            && self.sources & entry.source.bit() != 0
            && (self.text.is_empty()
                || contains_ignore_ascii_case(&entry.message, &self.text)
                || entry
                    .slot
                    .as_deref()
                    .is_some_and(|slot| contains_ignore_ascii_case(slot, &self.text)))
    }

    /// Only this source, or every source when `source` is `None`.
    pub fn set_source(&mut self, source: Option<Source>) {
        self.sources = source.map_or(ALL_SOURCES, Source::bit);
    }

    /// The single selected source, `None` when all (or several) are shown.
    pub fn single_source(&self) -> Option<Source> {
        Source::ALL.into_iter().find(|s| s.bit() == self.sources)
    }
}

fn contains_ignore_ascii_case(haystack: &str, needle: &str) -> bool {
    let (h, n) = (haystack.as_bytes(), needle.as_bytes());
    n.is_empty() || h.windows(n.len()).any(|w| w.eq_ignore_ascii_case(n))
}

/// A front end's filtered copy of one scope. [`LogStore::refresh`] brings it
/// up to date: nothing happens (no lock, no allocation) while the store and
/// the filter are unchanged; new matching lines are appended; a scope or
/// filter change rebuilds.
#[derive(Debug)]
pub struct LogView {
    scope: LogScope,
    filter: LogFilter,
    rows: VecDeque<LogEntry>,
    cap: usize,
    seen_generation: u64,
    last_seq: u64,
    rebuild: bool,
    dropped: u64,
    /// Scratch for merging rings in [`LogScope::All`].
    merge: Vec<(u64, usize, usize)>,
    /// Renderer hint: keep the newest row in view.
    pub follow: bool,
}

impl LogView {
    pub fn new(scope: LogScope) -> Self {
        let cap = scope_cap(&scope);
        Self {
            scope,
            filter: LogFilter::default(),
            rows: VecDeque::new(),
            cap,
            seen_generation: 0,
            last_seq: 0,
            rebuild: true,
            dropped: 0,
            merge: Vec::new(),
            follow: true,
        }
    }

    pub fn scope(&self) -> &LogScope {
        &self.scope
    }

    /// Point the view at `slot` (or the process ring for `None`) unless it
    /// shows everything. Allocates only when the slot actually changes.
    pub fn follow_slot(&mut self, slot: Option<&str>) {
        if self.scope == LogScope::All {
            return;
        }
        let same = match (&self.scope, slot) {
            (LogScope::Slot(current), Some(slot)) => current == slot,
            (LogScope::Process, None) => true,
            _ => false,
        };
        if !same {
            self.set_scope(slot.map_or(LogScope::Process, |s| LogScope::Slot(s.to_string())));
        }
    }

    pub fn set_scope(&mut self, scope: LogScope) {
        if self.scope != scope {
            self.cap = scope_cap(&scope);
            self.scope = scope;
            self.rebuild = true;
        }
    }

    pub fn filter(&self) -> &LogFilter {
        &self.filter
    }

    /// Edit the filter; a change rebuilds on the next refresh.
    pub fn edit_filter(&mut self, edit: impl FnOnce(&mut LogFilter)) {
        let before = self.filter.clone();
        edit(&mut self.filter);
        if self.filter != before {
            self.rebuild = true;
        }
    }

    pub fn rows(&self) -> &VecDeque<LogEntry> {
        &self.rows
    }

    pub fn len(&self) -> usize {
        self.rows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// Lines the scope's ring(s) evicted (oldest first) since they started.
    pub fn dropped(&self) -> u64 {
        self.dropped
    }

    /// The visible rows as text, one [`LogEntry::write_line`] per row (Copy
    /// and Save log…).
    pub fn to_text(&self) -> String {
        let mut out = String::with_capacity(self.rows.len() * 96);
        for row in &self.rows {
            row.write_line(&mut out);
            out.push('\n');
        }
        out
    }

    fn push_row(&mut self, entry: &LogEntry) {
        if self.rows.len() == self.cap {
            self.rows.pop_front();
        }
        self.rows.push_back(entry.clone());
    }
}

fn scope_cap(scope: &LogScope) -> usize {
    match scope {
        LogScope::Slot(_) => SLOT_RING_CAP,
        LogScope::Process => PROCESS_RING_CAP,
        LogScope::All => ALL_VIEW_CAP,
    }
}

#[derive(Default)]
struct Inner {
    next_seq: u64,
    process: Ring,
    slots: HashMap<Arc<str>, Ring>,
    secrets: Vec<Box<str>>,
    file: Option<SessionLogFile>,
    /// Reused line buffer for the session file.
    line: String,
}

/// Process-wide structured log. See the module docs.
pub struct LogStore {
    inner: Mutex<Inner>,
    /// Bumped after every push; a view compares it before locking.
    generation: AtomicU64,
}

impl Default for LogStore {
    fn default() -> Self {
        Self::new()
    }
}

static GLOBAL: LazyLock<LogStore> = LazyLock::new(LogStore::new);
static INSTALL: Once = Once::new();

/// The process store, installed as the `api::hostlog` sink on first use.
pub fn global() -> &'static LogStore {
    let store = &*GLOBAL;
    INSTALL.call_once(|| {
        api::hostlog::install_sink(store);
    });
    store
}

impl api::hostlog::Sink for LogStore {
    fn record(&self, record: &Record<'_>) {
        self.push(record);
    }
}

impl LogStore {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(Inner::default()),
            generation: AtomicU64::new(0),
        }
    }

    /// Store one line: redact secrets, flatten control characters, cap the
    /// length, stamp wall time and order, then forward it to the session
    /// file when one is open. Never blocks on I/O.
    pub fn push(&self, record: &Record<'_>) {
        let local = LocalTime::now();
        let mut inner = self.inner.lock();
        let message = clean_message(record.message, &inner.secrets);
        inner.next_seq += 1;
        let seq = inner.next_seq;
        let slot = record
            .slot
            .map(|name| match inner.slots.get_key_value(name) {
                Some((key, _)) => Arc::clone(key),
                None => Arc::from(name),
            });
        let entry = LogEntry {
            seq,
            clock: Clock::from_local(&local),
            tick: record.tick,
            level: record.level,
            source: record.source,
            slot: slot.clone(),
            message,
        };
        if inner.file.is_some() {
            let Inner { file, line, .. } = &mut *inner;
            line.clear();
            entry.write_line(line);
            if let Some(file) = file.as_ref() {
                file.send(line);
            }
        }
        match slot {
            Some(slot) => inner
                .slots
                .entry(slot)
                .or_default()
                .push(entry, SLOT_RING_CAP),
            None => inner.process.push(entry, PROCESS_RING_CAP),
        }
        drop(inner);
        self.generation.fetch_add(1, Ordering::Release);
    }

    /// A line for `slot` from a front end or the operator session.
    pub fn slot_line(&self, slot: &str, source: Source, level: Level, message: &str) {
        self.push(&Record {
            slot: Some(slot),
            tick: None,
            source,
            level,
            message,
        });
    }

    /// A line no slot owns.
    pub fn process_line(&self, source: Source, level: Level, message: &str) {
        self.push(&Record {
            slot: None,
            tick: None,
            source,
            level,
            message,
        });
    }

    /// Never store `password` in any log path. `username` guards the
    /// harness profiles whose password equals the name (redacting it would
    /// blank every line of that slot); secrets shorter than four bytes would
    /// blank ordinary words and are skipped.
    pub fn register_secret(&self, username: &str, password: &str) {
        if password.len() < MIN_SECRET_LEN || password == username {
            return;
        }
        let mut inner = self.inner.lock();
        if !inner.secrets.iter().any(|s| &**s == password) {
            inner.secrets.push(password.into());
        }
    }

    /// Current store generation; changes after every push.
    pub fn generation(&self) -> u64 {
        self.generation.load(Ordering::Acquire)
    }

    /// Bring `view` up to date. Returns whether its rows changed.
    pub fn refresh(&self, view: &mut LogView) -> bool {
        let generation = self.generation();
        if !view.rebuild && view.seen_generation == generation {
            return false;
        }
        let inner = self.inner.lock();
        let rebuilt = view.rebuild;
        if rebuilt {
            view.rows.clear();
            view.last_seq = 0;
            view.rebuild = false;
        }
        let before = view.rows.back().map(|e| e.seq);
        let after = view.last_seq;
        let single = match &view.scope {
            LogScope::Slot(name) => Some(inner.slots.get(name.as_str())),
            LogScope::Process => Some(Some(&inner.process)),
            LogScope::All => None,
        };
        match single {
            Some(Some(ring)) => {
                view.dropped = ring.dropped;
                append_ring(view, ring, after);
            }
            Some(None) => view.dropped = 0,
            None => {
                let mut rings: Vec<&Ring> = Vec::with_capacity(inner.slots.len() + 1);
                rings.push(&inner.process);
                rings.extend(inner.slots.values());
                view.dropped = rings.iter().map(|r| r.dropped).sum();
                let mut merge = std::mem::take(&mut view.merge);
                merge.clear();
                for (ring_index, ring) in rings.iter().enumerate() {
                    let start = ring.entries.partition_point(|e| e.seq <= after);
                    for i in start..ring.entries.len() {
                        merge.push((ring.entries[i].seq, ring_index, i));
                    }
                }
                merge.sort_unstable_by_key(|&(seq, ..)| seq);
                for &(_, ring_index, i) in &merge {
                    let entry = &rings[ring_index].entries[i];
                    if view.filter.matches(entry) {
                        view.push_row(entry);
                    }
                }
                if let Some(&(seq, ..)) = merge.last() {
                    view.last_seq = seq;
                }
                merge.clear();
                view.merge = merge;
            }
        }
        view.seen_generation = generation;
        rebuilt || view.rows.back().map(|e| e.seq) != before
    }

    /// Bytes held by `slot`'s ring: the ring buffer itself plus message
    /// heap and the shared slot name.
    pub fn slot_bytes(&self, slot: &str) -> usize {
        let inner = self.inner.lock();
        inner.slots.get_key_value(slot).map_or(0, |(name, ring)| {
            ring.entries.capacity() * std::mem::size_of::<LogEntry>()
                + ring.entries.iter().map(LogEntry::heap_bytes).sum::<usize>()
                + name.len()
        })
    }

    /// Entries currently held for `slot` (0 when it never logged).
    pub fn slot_len(&self, slot: &str) -> usize {
        self.inner
            .lock()
            .slots
            .get(slot)
            .map_or(0, |r| r.entries.len())
    }

    /// Turn the per-session file on (`Some`) or off (`None`). Closing drops
    /// the sender; the writer thread flushes what it has and exits on its
    /// own, so neither direction blocks the caller on I/O.
    pub fn set_file(&self, file: Option<SessionLogFile>) {
        let old = std::mem::replace(&mut self.inner.lock().file, file);
        drop(old);
    }

    /// Path of the open session file, if any.
    pub fn file_path(&self) -> Option<PathBuf> {
        self.inner
            .lock()
            .file
            .as_ref()
            .map(|f| f.path().to_path_buf())
    }
}

fn append_ring(view: &mut LogView, ring: &Ring, after: u64) {
    let start = ring.entries.partition_point(|e| e.seq <= after);
    for entry in ring.entries.range(start..) {
        if view.filter.matches(entry) {
            view.push_row(entry);
        }
    }
    if let Some(last) = ring.entries.back() {
        view.last_seq = view.last_seq.max(last.seq);
    }
}

/// Redact secrets, flatten newlines/tabs to spaces and cap the length.
fn clean_message(message: &str, secrets: &[Box<str>]) -> Box<str> {
    let mut owned: Option<String> = None;
    for secret in secrets {
        let current = owned.as_deref().unwrap_or(message);
        if current.contains(&**secret) {
            owned = Some(current.replace(&**secret, REDACTED));
        }
    }
    let current = owned.as_deref().unwrap_or(message);
    let needs_flatten = current.contains(['\n', '\r', '\t']);
    if !needs_flatten && current.len() <= MESSAGE_CAP {
        return match owned {
            Some(s) => s.into_boxed_str(),
            None => current.into(),
        };
    }
    let mut out = String::with_capacity(current.len().min(MESSAGE_CAP));
    for ch in current.chars() {
        let ch = if matches!(ch, '\n' | '\r' | '\t') {
            ' '
        } else {
            ch
        };
        if out.len() + ch.len_utf8() > MESSAGE_CAP - '…'.len_utf8() {
            out.push('…');
            break;
        }
        out.push(ch);
    }
    out.into_boxed_str()
}

/// Map a script line (isolate `this.log`, lifecycle errors staged by the
/// slot) onto the log's source/level.
pub fn classify_script_line(line: &str) -> (Source, Level) {
    if line.starts_with("watchdog") {
        return (Source::Watchdog, Level::Warn);
    }
    if line.starts_with("slow tick") || line.starts_with("skipped stale ticks") {
        return (Source::Script, Level::Debug);
    }
    let error = line.starts_with("script panic")
        || line.starts_with("script stopped")
        || line.starts_with("script requested stop")
        || line.contains("Error:")
        || line.contains(" threw")
        || line.contains("error:");
    (
        Source::Script,
        if error { Level::Error } else { Level::Info },
    )
}

/// Local calendar time from the OS clock (one libc/Win32 call).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct LocalTime {
    pub year: u32,
    pub month: u32,
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
    pub second: u32,
    pub millis: u32,
}

impl LocalTime {
    pub(crate) fn now() -> Self {
        local_now()
    }

    /// `YYYYmmdd-HHMMSS` for file names.
    pub(crate) fn stamp(&self) -> String {
        format!(
            "{:04}{:02}{:02}-{:02}{:02}{:02}",
            self.year, self.month, self.day, self.hour, self.minute, self.second
        )
    }
}

#[cfg(unix)]
fn local_now() -> LocalTime {
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = now.as_secs() as libc::time_t;
    // SAFETY: `localtime_r` writes only into the `tm` we own and reads the
    // `time_t` we pass; it is the re-entrant variant.
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    let ok = unsafe { !libc::localtime_r(&secs, &mut tm).is_null() };
    if !ok {
        return utc_from(now);
    }
    LocalTime {
        year: (tm.tm_year + 1900) as u32,
        month: (tm.tm_mon + 1) as u32,
        day: tm.tm_mday as u32,
        hour: tm.tm_hour as u32,
        minute: tm.tm_min as u32,
        second: tm.tm_sec as u32,
        millis: now.subsec_millis(),
    }
}

#[cfg(windows)]
fn local_now() -> LocalTime {
    #[repr(C)]
    struct SystemTime16 {
        year: u16,
        month: u16,
        day_of_week: u16,
        day: u16,
        hour: u16,
        minute: u16,
        second: u16,
        millis: u16,
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn GetLocalTime(time: *mut SystemTime16);
    }
    let mut t = SystemTime16 {
        year: 0,
        month: 0,
        day_of_week: 0,
        day: 0,
        hour: 0,
        minute: 0,
        second: 0,
        millis: 0,
    };
    // SAFETY: GetLocalTime fills the SYSTEMTIME we own and cannot fail.
    unsafe { GetLocalTime(&mut t) };
    LocalTime {
        year: t.year.into(),
        month: t.month.into(),
        day: t.day.into(),
        hour: t.hour.into(),
        minute: t.minute.into(),
        second: t.second.into(),
        millis: t.millis.into(),
    }
}

#[cfg(not(any(unix, windows)))]
fn local_now() -> LocalTime {
    utc_from(
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default(),
    )
}

/// UTC fallback when the local conversion is unavailable.
#[cfg_attr(windows, allow(dead_code))]
fn utc_from(since_epoch: std::time::Duration) -> LocalTime {
    let secs = since_epoch.as_secs();
    let days = secs / 86_400;
    let rem = secs % 86_400;
    // Civil-from-days (Howard Hinnant), proleptic Gregorian.
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = (yoe + era * 400 + i64::from(month <= 2)) as u32;
    LocalTime {
        year,
        month,
        day,
        hour: (rem / 3_600) as u32,
        minute: (rem / 60 % 60) as u32,
        second: (rem % 60) as u32,
        millis: since_epoch.subsec_millis(),
    }
}

/// `~/.274bot/logs`.
pub fn logs_dir() -> PathBuf {
    script::bot_file("logs")
}

/// Default Save log… target: `~/.274bot/logs/<label>-<stamp>.log`.
pub fn default_save_path(label: &str) -> PathBuf {
    let safe: String = label
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    logs_dir().join(format!("{safe}-{}.log", LocalTime::now().stamp()))
}

/// A Save log… in flight on its own thread; poll it from the UI.
#[derive(Debug, Clone, Default)]
pub struct SaveTicket(Arc<Mutex<Option<Result<PathBuf, String>>>>);

impl SaveTicket {
    /// The outcome once the write finished; `None` while it runs.
    pub fn poll(&self) -> Option<Result<PathBuf, String>> {
        self.0.lock().take()
    }
}

/// Write `text` to `path` off the caller's thread (private file and
/// directory permissions).
pub fn save_text(path: PathBuf, text: String) -> SaveTicket {
    let ticket = SaveTicket::default();
    let out = ticket.clone();
    std::thread::spawn(move || {
        let result = write_text(&path, &text)
            .map(|()| path.clone())
            .map_err(|e| format!("save log {}: {e}", path.display()));
        *out.0.lock() = Some(result);
    });
    ticket
}

fn write_text(path: &Path, text: &str) -> std::io::Result<()> {
    vault::write_private_file(path, text.as_bytes())
}

#[cfg(test)]
#[path = "log_tests.rs"]
mod tests;
