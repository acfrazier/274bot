//! Process start and stop for the front-end binaries, and process-wide
//! facts the front ends read.

/// Whether host debug diagnostics are on (`BOT_DEBUG`).
pub fn debug_enabled() -> bool {
    host::debug_enabled()
}

/// Why the process could not start.
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct StartError(String);

/// Start the protected V8 platform. Call first, on the process main thread,
/// before any script or slot work. With the memory benchmark it also marks
/// the process start.
pub fn bootstrap() -> Result<(), StartError> {
    script::registry::install_cards(&script::sherlock::CARDS)
        .map_err(|error| StartError(error.to_string()))?;
    script::bootstrap_v8_platform().map_err(|error| StartError(error.to_string()))?;
    #[cfg(feature = "memory-profile")]
    host_play::memory::mark_process_start();
    Ok(())
}

/// Raise the open-file soft limit before any slot starts (a Finder-launched
/// macOS app gets 256). The session log records the result when it opens.
pub fn raise_fd_limit() {
    let _ = host_play::fd_limit::raise_at_startup();
}

/// Why background work did not stop in order.
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct ShutdownError(String);

/// Stop the shared background workers, if any started, waiting up to their
/// fixed deadline.
pub fn shutdown_background_work() -> Result<(), ShutdownError> {
    script::work_admission::WorkService::shutdown_if_started()
        .map_err(|error| ShutdownError(error.to_string()))
}

/// The memory benchmark's global allocator for the front-end binaries:
/// counts requested Rust bytes (or forwards to the system allocator under
/// `memory-profile-no-alloc`). Install with `#[global_allocator]`.
#[cfg(feature = "memory-profile")]
pub struct BenchmarkAllocator;

/// The value to install as the benchmark's global allocator.
#[cfg(feature = "memory-profile")]
pub const BENCHMARK_ALLOCATOR: BenchmarkAllocator = BenchmarkAllocator;

// SAFETY: every method forwards its arguments unchanged to host-play's
// benchmark allocator, which upholds the `GlobalAlloc` contract; this type
// adds no state and no work, so the contract (layout validity, pointer
// provenance, dealloc with the allocating layout) holds exactly as for the
// forwarded allocator.
#[cfg(feature = "memory-profile")]
unsafe impl std::alloc::GlobalAlloc for BenchmarkAllocator {
    #[inline]
    unsafe fn alloc(&self, layout: std::alloc::Layout) -> *mut u8 {
        // SAFETY: the caller's `alloc` contract is passed through unchanged.
        unsafe { host_play::memory::BENCHMARK_ALLOCATOR.alloc(layout) }
    }

    #[inline]
    unsafe fn dealloc(&self, ptr: *mut u8, layout: std::alloc::Layout) {
        // SAFETY: `ptr` came from this allocator, which is the forwarded one,
        // with this `layout`.
        unsafe { host_play::memory::BENCHMARK_ALLOCATOR.dealloc(ptr, layout) }
    }

    #[inline]
    unsafe fn alloc_zeroed(&self, layout: std::alloc::Layout) -> *mut u8 {
        // SAFETY: the caller's `alloc_zeroed` contract is passed through.
        unsafe { host_play::memory::BENCHMARK_ALLOCATOR.alloc_zeroed(layout) }
    }

    #[inline]
    unsafe fn realloc(&self, ptr: *mut u8, layout: std::alloc::Layout, new_size: usize) -> *mut u8 {
        // SAFETY: `ptr`/`layout` came from the forwarded allocator and the
        // caller's `realloc` contract is passed through unchanged.
        unsafe { host_play::memory::BENCHMARK_ALLOCATOR.realloc(ptr, layout, new_size) }
    }
}

#[cfg(all(test, feature = "memory-profile"))]
mod tests {
    use std::alloc::{GlobalAlloc, Layout};

    use super::BENCHMARK_ALLOCATOR;

    /// The forwarding allocator round-trips allocate, zero, grow and free
    /// with the layouts the contract requires.
    #[test]
    fn the_benchmark_allocator_forwards_every_operation() {
        let layout = Layout::from_size_align(64, 16).unwrap();
        // SAFETY: a non-zero-size layout; every pointer is freed with the
        // layout it was allocated (or reallocated) with.
        unsafe {
            let ptr = BENCHMARK_ALLOCATOR.alloc(layout);
            assert!(!ptr.is_null());
            assert_eq!(ptr as usize % 16, 0);
            BENCHMARK_ALLOCATOR.dealloc(ptr, layout);

            let zeroed = BENCHMARK_ALLOCATOR.alloc_zeroed(layout);
            assert!(!zeroed.is_null());
            assert!(std::slice::from_raw_parts(zeroed, 64)
                .iter()
                .all(|b| *b == 0));
            *zeroed = 7;
            let grown = BENCHMARK_ALLOCATOR.realloc(zeroed, layout, 128);
            assert!(!grown.is_null());
            assert_eq!(*grown, 7);
            BENCHMARK_ALLOCATOR.dealloc(grown, Layout::from_size_align(128, 16).unwrap());
        }
    }
}
