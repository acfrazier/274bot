use std::sync::{Arc, LazyLock, Mutex, Weak};

use client::config::{Cache, ObjType};
use client::dash3d::store::ModelStore;
use client::graphics::{Pix32, Pix3DDraw};
use parking_lot::RwLock;

static CLIENT_CACHE: LazyLock<RwLock<Option<Weak<Cache>>>> = LazyLock::new(|| RwLock::new(None));
static ITEM_ICON_PIX3D: LazyLock<Mutex<Pix3DDraw>> =
    LazyLock::new(|| Mutex::new(Pix3DDraw::default()));

pub(super) fn publish(cache: &Arc<Cache>) {
    let mut current = CLIENT_CACHE.write();
    if current
        .as_ref()
        .is_some_and(|current| current.as_ptr() == Arc::as_ptr(cache))
    {
        return;
    }
    *current = Some(Arc::downgrade(cache));
}

/// Shared immutable item pixels for a synchronous ClientAdapter read.
#[doc(hidden)]
pub struct ItemIconPixels(Arc<Pix32>);

impl ItemIconPixels {
    pub fn width(&self) -> i32 {
        self.0.wi
    }

    pub fn height(&self) -> i32 {
        self.0.hi
    }

    pub fn data(&self) -> &[i32] {
        &self.0.data
    }
}

/// The process-wide client cache used by native ClientAdapter readers.
#[doc(hidden)]
#[derive(Clone)]
pub struct ClientAdapterCache(Arc<Cache>);

impl ClientAdapterCache {
    /// Find the shared count-one item sprite, rendering it into the canonical
    /// ModelStore only on a miss. Each caller gets the immutable shared Arc.
    pub fn item_icon_pixels(&self, id: i32) -> Option<ItemIconPixels> {
        if id < 0 {
            return None;
        }
        if let Some(sprite) = cached_item_sprite(id) {
            return Some(ItemIconPixels(sprite));
        }

        let mut pix3d = ITEM_ICON_PIX3D
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if ObjType::get_sprite(&self.0, &mut pix3d, id, 0, 1).is_none() {
            // ObjType's cert-template early return can precede its raster-state
            // restoration. This scratch renderer is private to the icon path.
            *pix3d = Pix3DDraw::default();
            return None;
        }
        drop(pix3d);

        cached_item_sprite(id).map(ItemIconPixels)
    }
}

/// Upgrade the process-wide cache handle; GameSnapshot refreshes it from the
/// same Client whose facts are posted to the isolate.
#[doc(hidden)]
pub fn client_adapter_cache() -> Option<ClientAdapterCache> {
    CLIENT_CACHE
        .read()
        .as_ref()
        .and_then(Weak::upgrade)
        .map(ClientAdapterCache)
}

fn cached_item_sprite(id: i32) -> Option<Arc<Pix32>> {
    let mut store = ModelStore::instance().lock().ok()?;
    let sprite = store.sprite(id)?;
    (sprite.ohi == 1 || sprite.ohi == -1).then_some(sprite)
}

#[cfg(feature = "test-support")]
#[doc(hidden)]
pub fn set_client_adapter_cache_for_tests(cache: Arc<Cache>) {
    publish(&cache);
}
#[cfg(feature = "test-support")]
#[doc(hidden)]
pub fn clear_client_adapter_cache_for_tests(cache: &Arc<Cache>) {
    let mut current = CLIENT_CACHE.write();
    if current
        .as_ref()
        .is_some_and(|current| current.as_ptr() == Arc::as_ptr(cache))
    {
        *current = None;
    }
}
