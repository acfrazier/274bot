//! The operator preferences document, as the front ends read and write it
//! (T3.3): a handle over the process local-state service's preferences
//! domain (the T5.2 I-state-port service), opened by the core
//! ([`crate::OperatorSession::open_process_state`]).
//!
//! Operator commands submit changed-member intent to the process writer.
//! Reads use its immutable cache. Neither operation waits for storage.

#[cfg(any(test, feature = "test-support"))]
use std::io;
use std::path::PathBuf;
use std::sync::Arc;

pub use host_play::local_state::{PreferenceError, PreferencePatch, PreferenceSnapshot, Update};
use serde_json::{Map, Value};

/// A handle on the preferences document. [`Self::share`] gives another
/// handle on the same document (no copy of it).
pub struct UiPreferences {
    inner: host_play::local_state::Preferences,
}

impl UiPreferences {
    pub(crate) fn new(inner: host_play::local_state::Preferences) -> Self {
        Self { inner }
    }

    pub(crate) fn inner(&self) -> &host_play::local_state::Preferences {
        &self.inner
    }

    /// Another handle on the same document.
    pub fn share(&self) -> Self {
        Self::new(self.inner.clone())
    }

    /// The preferences file.
    pub fn path(&self) -> PathBuf {
        self.inner.path()
    }

    /// The cached document, or its cold/refused state. This never waits.
    pub fn document(&self) -> Result<Arc<Map<String, Value>>, PreferenceError> {
        self.inner.document()
    }

    /// The cached document with its durable version (the applied-version
    /// key of the core's reconciliation), or its cold/refused state. This
    /// never waits.
    pub fn snapshot(&self) -> Result<PreferenceSnapshot, PreferenceError> {
        self.inner.snapshot()
    }

    /// One top-level preference; `None` when it is absent or the store
    /// cannot be read.
    pub fn value(&self, key: &str) -> Option<Value> {
        self.inner.value(key)
    }

    /// Whether the process writer published a readable preferences snapshot.
    pub fn ready(&self) -> bool {
        self.document().is_ok()
    }

    /// Submit only the members this action changed, without waiting.
    /// The process service owns accepted intent until terminal settlement.
    pub fn begin_patch(&self, patch: PreferencePatch) -> Result<Update, PreferenceError> {
        self.inner.begin_patch(patch)
    }

    /// Submit one top-level member without waiting.
    pub(crate) fn begin_set(&self, key: &str, value: Value) -> Result<Update, PreferenceError> {
        self.inner.begin_set(key, value)
    }

    /// Fixture-only readback. Product readers use the cached document.
    #[cfg(any(test, feature = "test-support"))]
    pub fn reload(&self) -> io::Result<Arc<Map<String, Value>>> {
        self.inner.reload()
    }

    /// Fixture-only synchronous mutation.
    #[cfg(any(test, feature = "test-support"))]
    pub fn update(
        &self,
        merge: impl FnMut(&mut Map<String, Value>) -> io::Result<()> + Send + 'static,
    ) -> io::Result<()> {
        self.inner.update(merge)
    }

    /// Fixture-only synchronous member replacement.
    #[cfg(any(test, feature = "test-support"))]
    pub fn set(&self, key: &str, value: Value) -> io::Result<()> {
        self.inner.set(key, value)
    }
}

/// Test seam: the terminal of a preference write that may still be
/// queued. The writer thread settles every queued write (durable,
/// refused, or cancelled when its service stops), so this waits for that
/// settlement; [`WRITE_HANG_GUARD`] only ends a test whose writer hung.
#[cfg(any(test, feature = "test-support"))]
pub(crate) fn settled_for_test(write: Result<Update, PreferenceError>) -> io::Result<()> {
    match write.map_err(io::Error::other)? {
        Update::Durable => Ok(()),
        Update::Queued(mut queued) => match queued.wait(WRITE_HANG_GUARD) {
            Some(result) => result.map_err(io::Error::other),
            None => Err(io::Error::new(
                io::ErrorKind::TimedOut,
                format!("the preference write was still queued after {WRITE_HANG_GUARD:?}"),
            )),
        },
    }
}

/// How long a test waits for one step of a queued preference write
/// before it calls the writer hung. Not a pass condition: a loaded host
/// delays the step (round 3 saw more than 5 s on a busy disk), and the
/// step ends by itself; ten minutes is far above any such delay.
#[cfg(any(test, feature = "test-support"))]
const WRITE_HANG_GUARD: std::time::Duration = std::time::Duration::from_secs(600);

#[cfg(any(test, feature = "test-support"))]
impl UiPreferences {
    /// Fixture seam: the preferences of `service` (a test's own process
    /// service, shared with its loadouts or a peer writer).
    pub fn of_service_for_test(service: &Arc<host_play::local_state::LocalStateService>) -> Self {
        Self::new(service.preferences())
    }

    /// Fixture seam: preferences of a process local-state service opened
    /// over `root` (a test's scratch state directory).
    pub fn open_for_test(root: PathBuf) -> Self {
        Self::new(
            host_play::local_state::LocalStateService::open_process(
                host_play::local_state::StateRoot::new(root),
            )
            .preferences(),
        )
    }
}
