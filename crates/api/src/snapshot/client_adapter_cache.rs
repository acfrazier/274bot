#[cfg(feature = "test-support")]
use std::cell::RefCell;
use std::sync::{Arc, LazyLock, Mutex, Weak};

use client::config::{Cache, ObjType};
use client::dash3d::store::ModelStore;
use client::graphics::{Pix32, Pix3DDraw};
use parking_lot::RwLock;

static CLIENT_CACHE: LazyLock<RwLock<Option<Weak<Cache>>>> = LazyLock::new(|| RwLock::new(None));
static ITEM_ICON_PIX3D: LazyLock<Mutex<Pix3DDraw>> =
    LazyLock::new(|| Mutex::new(Pix3DDraw::default()));
#[cfg(feature = "test-support")]
struct ClientCacheTestOverride {
    active: bool,
    cache: Option<Weak<Cache>>,
}

#[cfg(feature = "test-support")]
std::thread_local! {
    static CLIENT_CACHE_TEST_OVERRIDE: RefCell<ClientCacheTestOverride> =
        const { RefCell::new(ClientCacheTestOverride { active: false, cache: None }) };
}

#[cfg(feature = "test-support")]
fn with_test_cache<R>(f: impl FnOnce(&mut Option<Weak<Cache>>) -> R) -> Option<R> {
    CLIENT_CACHE_TEST_OVERRIDE.with(|override_cache| {
        let mut override_cache = override_cache.borrow_mut();
        override_cache.active.then(|| f(&mut override_cache.cache))
    })
}

#[cfg(feature = "test-support")]
#[doc(hidden)]
#[must_use]
pub struct ClientAdapterCacheTestIsolation(std::marker::PhantomData<std::rc::Rc<()>>);

#[cfg(feature = "test-support")]
impl Drop for ClientAdapterCacheTestIsolation {
    fn drop(&mut self) {
        let _ = CLIENT_CACHE_TEST_OVERRIDE.try_with(|override_cache| {
            let mut override_cache = override_cache.borrow_mut();
            override_cache.active = false;
            override_cache.cache = None;
        });
    }
}

#[cfg(feature = "test-support")]
#[doc(hidden)]
pub fn isolate_client_adapter_cache_for_tests() -> ClientAdapterCacheTestIsolation {
    CLIENT_CACHE_TEST_OVERRIDE.with(|override_cache| {
        let mut override_cache = override_cache.borrow_mut();
        assert!(
            !override_cache.active,
            "client adapter cache test isolation cannot be nested"
        );
        override_cache.active = true;
        override_cache.cache = None;
    });
    ClientAdapterCacheTestIsolation(std::marker::PhantomData)
}

fn publish_to(current: &mut Option<Weak<Cache>>, cache: &Arc<Cache>) {
    if current
        .as_ref()
        .is_some_and(|current| current.as_ptr() == Arc::as_ptr(cache))
    {
        return;
    }
    *current = Some(Arc::downgrade(cache));
}

pub(super) fn publish(cache: &Arc<Cache>) {
    #[cfg(feature = "test-support")]
    if with_test_cache(|current| publish_to(current, cache)).is_some() {
        return;
    }
    let mut current = CLIENT_CACHE.write();
    publish_to(&mut current, cache);
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

/// Upgrade the selected client cache handle; GameSnapshot refreshes it from the
/// same Client whose facts are posted to the isolate. Test-support scopes use
/// a per-thread handle so parallel tests cannot replace one another's cache.
#[doc(hidden)]
pub fn client_adapter_cache() -> Option<ClientAdapterCache> {
    #[cfg(feature = "test-support")]
    if let Some(cache) = with_test_cache(|current| current.as_ref().and_then(Weak::upgrade)) {
        return cache.map(ClientAdapterCache);
    }
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
    if with_test_cache(|current| {
        if current
            .as_ref()
            .is_some_and(|current| current.as_ptr() == Arc::as_ptr(cache))
        {
            *current = None;
        }
    })
    .is_some()
    {
        return;
    }
    let mut current = CLIENT_CACHE.write();
    if current
        .as_ref()
        .is_some_and(|current| current.as_ptr() == Arc::as_ptr(cache))
    {
        *current = None;
    }
}
#[cfg(all(test, feature = "test-support"))]
mod tests {
    use super::*;

    #[test]
    fn scoped_cache_publication_does_not_free_a_stale_process_cache() {
        let stale = Arc::new(Cache::default());
        let mut process_cache = CLIENT_CACHE.write();
        let previous = (*process_cache).replace(Arc::downgrade(&stale));
        drop(stale);

        let fresh = Arc::new(Cache::default());
        let _isolation = isolate_client_adapter_cache_for_tests();
        let measured = allocation_counter::measure(|| publish(&fresh));

        let cache_matches =
            client_adapter_cache().is_some_and(|cached| Arc::ptr_eq(&cached.0, &fresh));
        drop(_isolation);
        *process_cache = previous;

        assert_eq!(measured.bytes_total, 0, "{measured:?}");
        assert_eq!(measured.count_total, 0, "{measured:?}");
        assert_eq!(measured.bytes_current, 0, "{measured:?}");
        assert_eq!(measured.count_current, 0, "{measured:?}");
        assert!(cache_matches);
    }
}
