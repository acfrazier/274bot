//! Benchmark-only counters. Separate domains — not additive with RSS.
//!
//! Isolate/pool wiring that updates these is a later hop. Until then,
//! callers should treat absence of a real sample as JSON null, not zero.

#![cfg(feature = "memory-profile")]

use std::sync::atomic::{AtomicU64, Ordering::Relaxed};

pub(crate) static SNAPSHOT_BYTES: AtomicU64 = AtomicU64::new(0);
pub(crate) static SNAPSHOT_CAPACITY: AtomicU64 = AtomicU64::new(0);
pub(crate) static HEAP_USED: AtomicU64 = AtomicU64::new(0);
pub(crate) static HEAP_TOTAL: AtomicU64 = AtomicU64::new(0);

/// `(snapshot_inflight_bytes, snapshot_inflight_capacity, v8_used, v8_total)`.
///
/// Returns the raw atomics. Host sample builders must not pretend an
/// unwired counter is a measurement (prefer JSON null over these zeros
/// until isolate hooks land).
pub fn sample() -> (u64, u64, u64, u64) {
    (
        SNAPSHOT_BYTES.load(Relaxed),
        SNAPSHOT_CAPACITY.load(Relaxed),
        HEAP_USED.load(Relaxed),
        HEAP_TOTAL.load(Relaxed),
    )
}
