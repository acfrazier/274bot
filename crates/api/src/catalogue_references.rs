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
    /// A legacy source or durable value was refused. Nothing was overwritten.
    #[error("{path}: {detail}")]
    Refused {
        /// Source or database path that failed.
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
}
