//! Root-wide catalogue references, with independent component versions.

use std::path::PathBuf;
use std::sync::Arc;

use crate::local_state::StoreVersion;

/// Largest legacy or canonical file-card list, in bytes.
pub const MAX_CATALOGUE_FILE_BYTES: usize = 4 * 1024 * 1024;
/// Most file references admitted by the smallest legacy `{name,path}` records.
pub const MAX_CATALOGUE_FILE_ROWS: usize = (MAX_CATALOGUE_FILE_BYTES - 1) / 22;
/// Largest persisted catalogue root, in UTF-8 bytes.
pub const MAX_CATALOGUE_ROOT_BYTES: usize = 8 * 1024;

/// A file-card identity. Source text and compiled code remain outside the store.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct FileCardReference {
    /// Picker name, preserved even when another path has the same name.
    pub name: String,
    /// Verbatim source path. Consumers check trust before opening it.
    pub path: String,
}

/// One independently versioned reference component.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CatalogueComponent<T> {
    /// Durable version of this component only.
    pub version: StoreVersion,
    /// Immutable component value.
    pub value: T,
}

/// The three typed reference values stored for one operator root.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CatalogueReferences {
    /// Ordered file-card references, including unresolved identities.
    pub file_cards: CatalogueComponent<Arc<[FileCardReference]>>,
    /// Persisted selection, independent of environment and runtime overrides.
    pub saved_root: CatalogueComponent<Option<PathBuf>>,
    /// Whether the operator deferred the first-run catalogue prompt.
    pub import_deferred: CatalogueComponent<bool>,
}

/// Why a reference component could not be loaded or saved.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum CatalogueReferenceError {
    /// A legacy source file could not be read or parsed. Nothing was overwritten.
    #[error("{path}: {detail}")]
    LegacySourceRefused {
        /// Legacy source path that failed.
        path: PathBuf,
        /// Cause and operator remedy.
        detail: Arc<str>,
    },
    /// A durable value or state database operation was refused. Nothing was overwritten.
    #[error("{path}: {detail}")]
    Refused {
        /// Database path that failed.
        path: PathBuf,
        /// Cause and operator remedy.
        detail: Arc<str>,
    },
    /// Another writer changed this component since its read.
    #[error("catalogue references changed in another operation; reload and retry")]
    Conflict,
    /// The bounded owner queue could not complete this request.
    #[error("catalogue references: {0}")]
    Unavailable(Arc<str>),
}

enum PendingCatalogueOpState<T> {
    Ready(Option<Result<T, CatalogueReferenceError>>),
    Receiver {
        rx: std::sync::mpsc::Receiver<Result<T, CatalogueReferenceError>>,
        disconnected: &'static str,
    },
}

/// Non-blocking handle for one queued catalogue reference operation.
///
/// Polling never blocks the caller thread.
pub struct PendingCatalogueOp<T> {
    state: PendingCatalogueOpState<T>,
}

impl<T> PendingCatalogueOp<T> {
    /// Wrap a result that is already settled.
    pub fn ready(result: Result<T, CatalogueReferenceError>) -> Self {
        Self {
            state: PendingCatalogueOpState::Ready(Some(result)),
        }
    }

    /// Build a handle backed by a non-blocking single-result channel.
    pub fn from_receiver(
        rx: std::sync::mpsc::Receiver<Result<T, CatalogueReferenceError>>,
        disconnected: &'static str,
    ) -> Self {
        Self {
            state: PendingCatalogueOpState::Receiver { rx, disconnected },
        }
    }

    /// Return the settled result if complete, or `None` while still pending.
    pub fn poll(&mut self) -> Option<Result<T, CatalogueReferenceError>> {
        match &mut self.state {
            PendingCatalogueOpState::Ready(result) => result.take(),
            PendingCatalogueOpState::Receiver { rx, disconnected } => match rx.try_recv() {
                Ok(result) => {
                    self.state = PendingCatalogueOpState::Ready(None);
                    Some(result)
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => None,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    let detail = *disconnected;
                    self.state = PendingCatalogueOpState::Ready(None);
                    Some(Err(CatalogueReferenceError::Unavailable(Arc::from(detail))))
                }
            },
        }
    }
}

/// Host-owned durable operations. Each read and edit addresses one component.
///
/// `None` on explicit root/defer commands permits a deliberate choice after a
/// failed private legacy read. It does not import, rename, or overwrite that source.
/// File-card edits always require a successful read of their own component.
pub trait CatalogueReferenceStore: Send + Sync {
    /// Hydrate ordered file references before restoring any source.
    fn file_cards(
        &self,
    ) -> Result<CatalogueComponent<Arc<[FileCardReference]>>, CatalogueReferenceError>;
    /// Hydrate the persisted root, without environment or runtime overrides.
    fn saved_root(&self) -> Result<CatalogueComponent<Option<PathBuf>>, CatalogueReferenceError>;
    /// Hydrate the first-run defer choice before deciding whether to prompt.
    fn import_deferred(&self) -> Result<CatalogueComponent<bool>, CatalogueReferenceError>;
    /// Replace the file component against its own durable version.
    fn save_file_cards(
        &self,
        expected: StoreVersion,
        rows: Arc<[FileCardReference]>,
    ) -> Result<StoreVersion, CatalogueReferenceError>;
    /// Persist an explicit successful selection, or clear that selection.
    fn save_root(
        &self,
        expected: Option<StoreVersion>,
        root: Option<PathBuf>,
    ) -> Result<StoreVersion, CatalogueReferenceError>;
    /// Persist an explicit defer or clear choice.
    fn save_deferred(
        &self,
        expected: Option<StoreVersion>,
        deferred: bool,
    ) -> Result<StoreVersion, CatalogueReferenceError>;
    /// Queue a non-blocking read of ordered file references on the owner.
    fn submit_file_cards(
        &self,
    ) -> PendingCatalogueOp<CatalogueComponent<Arc<[FileCardReference]>>> {
        PendingCatalogueOp::ready(self.file_cards())
    }
    /// Queue a non-blocking replacement of the file component on the owner.
    fn submit_save_file_cards(
        &self,
        expected: StoreVersion,
        rows: Arc<[FileCardReference]>,
    ) -> PendingCatalogueOp<StoreVersion> {
        PendingCatalogueOp::ready(self.save_file_cards(expected, rows))
    }
    /// Queue a non-blocking root save on the owner.
    fn submit_save_root(
        &self,
        expected: Option<StoreVersion>,
        root: Option<PathBuf>,
    ) -> PendingCatalogueOp<StoreVersion> {
        PendingCatalogueOp::ready(self.save_root(expected, root))
    }
    /// Queue a non-blocking defer save on the owner.
    fn submit_save_deferred(
        &self,
        expected: Option<StoreVersion>,
        deferred: bool,
    ) -> PendingCatalogueOp<StoreVersion> {
        PendingCatalogueOp::ready(self.save_deferred(expected, deferred))
    }
}
