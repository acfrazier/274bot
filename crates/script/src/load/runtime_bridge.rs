//! One process-owned Tokio bridge for rustyscript's isolate-local futures.
//! No product CPU job may be submitted to this runtime.

use std::sync::LazyLock;
use std::time::Duration;

use tokio::runtime::{Builder, Runtime};

use super::isolate::LoadIsolateError;

static BRIDGE: LazyLock<Result<Runtime, String>> = LazyLock::new(|| {
    Builder::new_multi_thread()
        .worker_threads(1)
        .max_blocking_threads(1)
        .thread_keep_alive(Duration::from_millis(50))
        .enable_all()
        .thread_name("script-async-bridge")
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
