//! Process-wide V8 initialization. Runtime threads inherit Linux protection-key
//! permissions from the thread that initializes the protected platform.

use std::sync::Once;

use super::isolate::LoadIsolateError;

static PLATFORM: Once = Once::new();

/// Bootstrap the protected V8 platform once for this process.
///
/// Call this on the process's main thread, before creating any slot, worker or
/// control thread that may create a JavaScript runtime (including validation
/// and script restarts). All V8-using threads must descend from that thread
/// **after** initialization: on Linux, memory-protection-key permissions are
/// per-thread and are not retroactively applied to existing sibling threads.
/// A once-only call on the first bot's thread does not satisfy this contract.
///
/// Repeated calls do nothing; they cannot repair an incorrect first caller or
/// change an already initialized platform. Embedders must use this entrypoint
/// before any other V8 initialization. The platform lives until process exit.
pub fn bootstrap_v8_platform() {
    PLATFORM.call_once(|| rustyscript::init_platform(1, true));
}

/// Stable libtest creates sibling workers before any test can initialize V8.
/// Use V8's documented unprotected platform only for unit tests or debug builds
/// with the dev-only test-hooks feature. Neither this function nor its callsite
/// exists in release builds, even when test-hooks is accidentally enabled.
#[cfg(any(test, all(feature = "test-hooks", debug_assertions)))]
pub(super) fn bootstrap_test_v8_platform() {
    PLATFORM.call_once(|| {
        let platform = v8::new_unprotected_default_platform(1, true).make_shared();
        deno_core::JsRuntime::init_platform(Some(platform), true);
    });
}

pub(super) fn require_v8_platform() -> Result<(), LoadIsolateError> {
    #[cfg(any(test, all(feature = "test-hooks", debug_assertions)))]
    bootstrap_test_v8_platform();

    if PLATFORM.is_completed() {
        Ok(())
    } else {
        Err(LoadIsolateError::PlatformNotBootstrapped)
    }
}
