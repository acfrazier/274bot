use std::path::{Path, PathBuf};
use std::sync::Arc;

use api::local_state::{
    CatalogueComponent, CatalogueReferenceError, CatalogueReferenceStore, FileCardReference,
    StoreVersion,
};
use host_play::local_state::{LocalStateService, StateRoot};
use script_js::JsLibrary;

/// Real local-state storage for one script-js test directory.
pub struct CatalogueState {
    _service: Arc<LocalStateService>,
    references: Arc<dyn CatalogueReferenceStore>,
}

impl CatalogueState {
    pub fn new(dir: &Path) -> Self {
        let service = LocalStateService::open_process(StateRoot::new(dir.join("local-state")));
        let references = service.catalogue_references();
        Self {
            _service: service,
            references,
        }
    }

    pub fn references(&self) -> Arc<dyn CatalogueReferenceStore> {
        Arc::clone(&self.references)
    }

    pub fn library(&self, cache_root: impl Into<PathBuf>) -> JsLibrary {
        JsLibrary::with_cache(self.references(), cache_root.into())
    }
}

pub fn library(dir: &Path, cache_root: impl Into<PathBuf>) -> JsLibrary {
    CatalogueState::new(dir).library(cache_root)
}

/// Forces root writes to use a stale SQL version while all reads use the real service.
pub struct StaleRootSave {
    inner: Arc<dyn CatalogueReferenceStore>,
}

impl StaleRootSave {
    pub fn new(inner: Arc<dyn CatalogueReferenceStore>) -> Self {
        Self { inner }
    }
}

impl CatalogueReferenceStore for StaleRootSave {
    fn file_cards(
        &self,
    ) -> Result<CatalogueComponent<Arc<[FileCardReference]>>, CatalogueReferenceError> {
        self.inner.file_cards()
    }

    fn saved_root(
        &self,
    ) -> Result<CatalogueComponent<Option<PathBuf>>, CatalogueReferenceError> {
        self.inner.saved_root()
    }

    fn import_deferred(&self) -> Result<CatalogueComponent<bool>, CatalogueReferenceError> {
        self.inner.import_deferred()
    }

    fn save_file_cards(
        &self,
        expected: StoreVersion,
        rows: Arc<[FileCardReference]>,
    ) -> Result<StoreVersion, CatalogueReferenceError> {
        self.inner.save_file_cards(expected, rows)
    }

    fn save_root(
        &self,
        _expected: Option<StoreVersion>,
        root: Option<PathBuf>,
    ) -> Result<StoreVersion, CatalogueReferenceError> {
        self.inner.save_root(Some(StoreVersion::INITIAL), root)
    }

    fn save_deferred(
        &self,
        expected: Option<StoreVersion>,
        deferred: bool,
    ) -> Result<StoreVersion, CatalogueReferenceError> {
        self.inner.save_deferred(expected, deferred)
    }
}
