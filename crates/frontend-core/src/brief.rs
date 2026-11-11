//! Brief notices: one short line for the operator, the whole text in the log.
//!
//! A notice the panel and the TUI show on one line is at most [`LINE_MAX`]
//! characters. When the producer has more to say (the zones that block a
//! route, every failed script card), the extra text is *detail*: it goes to
//! the in-app log once, as packed lines under the store's cap
//! ([`api::hostlog::pack`]), and the shown line ends with [`LOG_POINTER`].
//! The pointer is present only when the detail really reached the log ring:
//! a sink is installed and the category is on the slot-log allowlist
//! ([`api::hostlog::logged`]). Session file logging is off by default and has
//! no say in it.
//!
//! One layer writes each detail: the layer that first holds it. Every other
//! layer carries only the short line, which is at most [`LINE_MAX`]
//! characters and has no line break, so a second [`Brief::from_text`] over a
//! published line changes nothing and logs nothing.

use std::borrow::Cow;

use api::hostlog::{self, Category, Emit, Level};

pub use api::hostlog::{LINE_CHARS as LINE_MAX, SUMMARY_CHARS as SUMMARY_MAX};

/// The end of a shown line whose detail is in the log.
pub const LOG_POINTER: &str = " (see log)";

/// Where a brief's detail is logged.
#[derive(Debug, Clone, Copy)]
pub struct LogTarget<'a> {
    /// Routing category. It must be a slot-log category for the detail to
    /// reach the in-app ring.
    pub category: Category,
    /// Severity of each packed line.
    pub level: Level,
    /// The bot the detail belongs to, when it has one.
    pub slot: Option<&'a str>,
    /// Prefix of each packed line.
    pub head: &'a str,
}

impl<'a> LogTarget<'a> {
    /// A command refusal or failure for `slot` (`Lifecycle`, `info`).
    pub fn command(slot: Option<&'a str>) -> Self {
        Self {
            category: Category::Lifecycle,
            level: Level::Info,
            slot,
            head: "detail: ",
        }
    }

    /// A walk refusal (`NavEvent`, `info`).
    pub fn walk(slot: Option<&'a str>) -> Self {
        Self {
            category: Category::NavEvent,
            level: Level::Info,
            slot,
            head: "walk detail: ",
        }
    }

    /// Script load failures (`ScriptLifecycle`, `warn`).
    pub fn script_load() -> Self {
        Self {
            category: Category::ScriptLifecycle,
            level: Level::Warn,
            slot: None,
            head: "load failure: ",
        }
    }

    /// A script banner no other producer owns (`ScriptLifecycle`, `info`):
    /// the last guard of [`crate::Scripts`]'s `show`.
    pub fn script_notice() -> Self {
        Self {
            category: Category::ScriptLifecycle,
            level: Level::Info,
            slot: None,
            head: "notice: ",
        }
    }

    fn emit(&self) -> Emit<'a> {
        Emit {
            category: self.category,
            level: self.level,
            slot: self.slot,
            always_stderr: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Detail {
    None,
    /// Items to log when the brief is published.
    Items(Vec<String>),
    /// The producer already logged the detail (a walk refusal: the host
    /// writes it once per bot).
    LoggedByProducer,
}

/// A short line for the operator and, optionally, the detail behind it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Brief {
    summary: String,
    detail: Detail,
}

impl Brief {
    /// A brief whose summary is the first line of `summary`, cut to
    /// [`SUMMARY_MAX`] characters.
    pub fn new(summary: impl AsRef<str>) -> Self {
        let (first, _) = split_first_line(summary.as_ref());
        Self {
            summary: hostlog::cut_chars(first, SUMMARY_MAX).into_owned(),
            detail: Detail::None,
        }
    }

    /// The guard for a text nobody shortened. A single line of at most
    /// [`LINE_MAX`] characters (a published brief, a short refusal) is kept
    /// as it is, with no detail. Anything else shows its first line cut to
    /// [`SUMMARY_MAX`] characters and keeps the whole text as detail.
    pub fn from_text(text: &str) -> Self {
        if is_one_line(text) {
            return Self {
                summary: text.to_owned(),
                detail: Detail::None,
            };
        }
        Self::new(text).with_detail([text.to_owned()])
    }

    /// Add detail items, logged once by [`Self::publish`]. No items leaves
    /// the brief as it was.
    #[must_use]
    pub fn with_detail(mut self, items: impl IntoIterator<Item = String>) -> Self {
        let items: Vec<String> = items.into_iter().collect();
        if !items.is_empty() {
            self.detail = Detail::Items(items);
        }
        self
    }

    /// The producer already logged the detail; [`Self::publish`] writes
    /// nothing and still points at the log.
    #[must_use]
    pub fn logged_by_producer(mut self) -> Self {
        self.detail = Detail::LoggedByProducer;
        self
    }

    /// The summary, with no pointer.
    pub fn summary(&self) -> &str {
        &self.summary
    }

    /// Whether the brief has detail to log (or already logged).
    pub fn has_detail(&self) -> bool {
        !matches!(self.detail, Detail::None)
    }

    /// Write the detail to the log, once, and return the line to show: the
    /// summary, then [`LOG_POINTER`] when the detail reached the in-app log
    /// ring. At most [`LINE_MAX`] characters, one line.
    pub fn publish(self, target: &LogTarget<'_>) -> String {
        let Self {
            mut summary,
            detail,
        } = self;
        let pointed = match detail {
            Detail::None => false,
            Detail::Items(items) => {
                hostlog::emit_packed(target.emit(), target.head, &items);
                true
            }
            Detail::LoggedByProducer => true,
        };
        if pointed && hostlog::logged(target.category) {
            summary.push_str(LOG_POINTER);
        }
        summary
    }
}

/// Whether `text` is one line that fits [`LINE_MAX`] characters.
fn is_one_line(text: &str) -> bool {
    !text.contains(['\n', '\r']) && text.chars().nth(LINE_MAX).is_none()
}

/// The first line of `text` that has text, and whether anything follows it.
fn split_first_line(text: &str) -> (&str, bool) {
    let text = text.trim_start_matches(['\n', '\r']);
    let first = text.split(['\n', '\r']).next().unwrap_or("");
    (
        first,
        first.len() < text.trim_end_matches(['\n', '\r']).len(),
    )
}

/// `text` as one shown line: its first line, at most [`LINE_MAX`]
/// characters, ending with `…` when anything was left out. Borrowed when the
/// text already is one short line. Pure: it logs nothing and never changes
/// the stored text, so a caller's equality checks keep working.
pub fn notice_line(text: &str) -> Cow<'_, str> {
    one_line(text, LINE_MAX)
}

/// The first line of `text` with text, at most `max` characters, ending with
/// `…` when a later line or part of the first line was left out.
pub(crate) fn one_line(text: &str, max: usize) -> Cow<'_, str> {
    let (first, more_lines) = split_first_line(text);
    let line = hostlog::cut_chars(first, if more_lines { max - 1 } else { max });
    if !more_lines || line.ends_with('…') {
        return line;
    }
    let mut owned = line.into_owned();
    owned.push('…');
    Cow::Owned(owned)
}

#[cfg(test)]
#[path = "brief_tests.rs"]
mod tests;
