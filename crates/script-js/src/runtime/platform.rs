//! Process-wide V8 initialization. Runtime threads inherit Linux protection-key
//! permissions from the thread that initializes the protected platform.

use std::sync::OnceLock;

use super::isolate::LoadIsolateError;

static PLATFORM: OnceLock<()> = OnceLock::new();

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
pub fn bootstrap_v8_platform() -> Result<(), LoadIsolateError> {
    if PLATFORM.get().is_some() {
        return Ok(());
    }
    if !on_process_main_thread() {
        return Err(LoadIsolateError::PlatformNotMainThread);
    }
    PLATFORM.get_or_init(|| rustyscript::init_platform(1, true));
    Ok(())
}

fn on_process_main_thread() -> bool {
    #[cfg(target_os = "linux")]
    {
        // SAFETY: gettid and getpid take no pointers and have no preconditions.
        unsafe { libc::syscall(libc::SYS_gettid) == i64::from(libc::getpid()) }
    }
    #[cfg(not(target_os = "linux"))]
    {
        // Other supported targets' Rust process entry names its main thread.
        // An unknown name refuses bootstrap rather than initializing too late.
        std::thread::current().name() == Some("main")
    }
}

/// Stable libtest creates sibling workers before any test can initialize V8.
/// Use V8's documented unprotected platform only for unit tests or debug builds
/// with the dev-only test-hooks feature. Neither this function nor its callsite
/// exists in release builds, even when test-hooks is accidentally enabled.
#[cfg(any(test, all(feature = "test-hooks", debug_assertions)))]
pub(super) fn bootstrap_test_v8_platform() {
    PLATFORM.get_or_init(|| {
        let platform = v8::new_unprotected_default_platform(1, true).make_shared();
        deno_core::JsRuntime::init_platform(Some(platform), true);
    });
}

pub(super) fn require_v8_platform() -> Result<(), LoadIsolateError> {
    #[cfg(any(test, all(feature = "test-hooks", debug_assertions)))]
    bootstrap_test_v8_platform();

    if PLATFORM.get().is_some() {
        Ok(())
    } else {
        Err(LoadIsolateError::PlatformNotBootstrapped)
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    #[test]
    fn only_process_main_thread_can_initialize_protected_platform() {
        assert!(!std::thread::spawn(super::on_process_main_thread)
            .join()
            .unwrap());
    }
}
