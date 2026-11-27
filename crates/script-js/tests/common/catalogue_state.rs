use std::path::{Path, PathBuf};
use std::sync::Arc;

use api::local_state::CatalogueReferenceStore;
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
