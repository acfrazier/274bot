#[path = "catalogue_store.rs"]
mod catalogue_store;

use std::path::PathBuf;
use std::sync::Arc;

use api::local_state::CatalogueReferenceStore;
use script_js::JsLibrary;

/// Shared in-memory reference storage for one script-js test.
pub struct CatalogueState {
    references: Arc<dyn CatalogueReferenceStore>,
}

impl Default for CatalogueState {
    fn default() -> Self {
        Self {
            references: Arc::new(catalogue_store::MemoryCatalogueReferences::default()),
        }
    }
}

impl CatalogueState {
    pub fn references(&self) -> Arc<dyn CatalogueReferenceStore> {
        Arc::clone(&self.references)
    }

    pub fn library(&self, cache_root: impl Into<PathBuf>) -> JsLibrary {
        JsLibrary::with_cache(self.references(), cache_root.into())
    }
}

pub fn library(cache_root: impl Into<PathBuf>) -> JsLibrary {
    CatalogueState::default().library(cache_root)
}
