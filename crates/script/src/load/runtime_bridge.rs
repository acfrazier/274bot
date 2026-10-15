//! One process-owned Tokio bridge for rustyscript's isolate-local futures.
//! No product CPU job may be submitted to this runtime.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::LazyLock;
use std::time::Duration;

use tokio::runtime::{Builder, Runtime};

use super::isolate::LoadIsolateError;

static BRIDGE: LazyLock<Result<Runtime, String>> = LazyLock::new(|| {
    // Tokio exposes a single naming hook for both pools, so distinguish by
    // spawn order: worker threads spawn at build, blocking threads lazily
    // afterwards. Both names stay within the Linux 15-char task-name limit.
    Builder::new_multi_thread()
        .worker_threads(1)
        .max_blocking_threads(1)
        .thread_keep_alive(Duration::from_millis(50))
        .enable_all()
        .thread_name_fn(|| {
            static NEXT_ID: AtomicUsize = AtomicUsize::new(0);
            if NEXT_ID.fetch_add(1, Ordering::SeqCst) == 0 {
                "script-bridge".to_string()
            } else {
                "script-blk".to_string()
            }
        })
        .build()
        .map_err(|error| error.to_string())
});

/// Return a borrowed handle. A failed first construction stays failed: retrying
/// could leave a previous worker running and silently violate the census.
pub(super) fn handle() -> Result<tokio::runtime::Handle, LoadIsolateError> {
    super::platform::require_v8_platform()?;
    BRIDGE
        .as_ref()
        .map(|runtime| runtime.handle().clone())
        .map_err(|error| LoadIsolateError::Bridge(error.clone()))
}
