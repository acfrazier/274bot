//! The operator preferences document, as the front ends read and write it
//! (T3.3): a handle over the process local-state service's preferences
//! domain (the T5.2 I-state-port service), opened by the core
//! ([`crate::OperatorSession::open_process_state`]).
//!
//! Operator preference changes go through their commands
//! (`SetNavPreference`, `SetDangerLevel`, `SetSessionLog`,
//! `SetMapBakeChoice`, `SetQuesterPaths`, `SetDraw`); a front end writes
//! its own presentation state (window layout, last focus) through
//! [`UiPreferences::update`].

use std::io;
use std::path::PathBuf;
use std::sync::Arc;

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

    /// The current document (read again only when it is stale).
    pub fn document(&self) -> io::Result<Arc<Map<String, Value>>> {
        self.inner.document()
    }

    /// One top-level preference; `None` when it is absent or the store
    /// cannot be read.
    pub fn value(&self, key: &str) -> Option<Value> {
        self.inner.value(key)
    }

    /// Read the file again now, whatever its freshness.
    pub fn reload(&self) -> io::Result<Arc<Map<String, Value>>> {
        self.inner.reload()
    }

    /// Read the current document, let `merge` change it, and write each
    /// changed top-level preference durably (retried on a conflicting
    /// writer).
    pub fn update(
        &self,
        merge: impl FnMut(&mut Map<String, Value>) -> io::Result<()>,
    ) -> io::Result<()> {
        self.inner.update(merge)
    }

    /// Write one top-level preference.
    pub fn set(&self, key: &str, value: Value) -> io::Result<()> {
        self.inner.set(key, value)
    }
}

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
