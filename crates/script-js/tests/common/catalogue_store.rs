//! In-memory catalogue port for JS consumer tests. SQL and legacy imports are
//! tested by the host and front ends that own persistent state.

use std::path::PathBuf;
use std::sync::Arc;

use api::local_state::{
    CatalogueComponent, CatalogueReferenceError, CatalogueReferenceStore, CatalogueReferences,
    FileCardReference, StoreVersion,
};
use parking_lot::Mutex;

pub struct MemoryCatalogueReferences(Mutex<CatalogueReferences>);

impl Default for MemoryCatalogueReferences {
    fn default() -> Self {
        Self(Mutex::new(CatalogueReferences {
            file_cards: CatalogueComponent {
                version: StoreVersion::INITIAL,
                value: Arc::default(),
            },
            saved_root: CatalogueComponent {
                version: StoreVersion::INITIAL,
                value: None,
            },
            import_deferred: CatalogueComponent {
                version: StoreVersion::INITIAL,
                value: false,
            },
        }))
    }
}

impl MemoryCatalogueReferences {
    fn replace<T>(
        component: &mut CatalogueComponent<T>,
        expected: Option<StoreVersion>,
        value: T,
    ) -> Result<StoreVersion, CatalogueReferenceError> {
        if expected.is_some_and(|version| version != component.version) {
            return Err(CatalogueReferenceError::Conflict);
        }
        let version = component.version.checked_next().ok_or_else(|| {
            CatalogueReferenceError::Unavailable("catalogue version space exhausted".into())
        })?;
        *component = CatalogueComponent { version, value };
        Ok(version)
    }
}

impl CatalogueReferenceStore for MemoryCatalogueReferences {
    fn file_cards(
        &self,
    ) -> Result<CatalogueComponent<Arc<[FileCardReference]>>, CatalogueReferenceError> {
        Ok(self.0.lock().file_cards.clone())
    }

    fn saved_root(&self) -> Result<CatalogueComponent<Option<PathBuf>>, CatalogueReferenceError> {
        Ok(self.0.lock().saved_root.clone())
    }

    fn import_deferred(&self) -> Result<CatalogueComponent<bool>, CatalogueReferenceError> {
        Ok(self.0.lock().import_deferred.clone())
    }

    fn save_file_cards(
        &self,
        expected: StoreVersion,
        rows: Arc<[FileCardReference]>,
    ) -> Result<StoreVersion, CatalogueReferenceError> {
        Self::replace(&mut self.0.lock().file_cards, Some(expected), rows)
    }

    fn save_root(
        &self,
        expected: Option<StoreVersion>,
        root: Option<PathBuf>,
    ) -> Result<StoreVersion, CatalogueReferenceError> {
        Self::replace(&mut self.0.lock().saved_root, expected, root)
    }

    fn save_deferred(
        &self,
        expected: Option<StoreVersion>,
        deferred: bool,
    ) -> Result<StoreVersion, CatalogueReferenceError> {
        Self::replace(&mut self.0.lock().import_deferred, expected, deferred)
    }
}
