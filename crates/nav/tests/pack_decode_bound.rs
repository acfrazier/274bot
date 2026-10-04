//! PACK-25: a truncated large-grid/pack header must fail before allocating
//! the claimed payload. A counting global allocator proves no multi-megabyte
//! allocation happens: without the `cells > remaining` bound the decoders
//! would attempt a 256 MiB (grid) / 1 GiB (pack) `vec![0u8; cells]`, which
//! still returns `Truncated` afterwards, so an error-only assertion cannot
//! tell the bound apart.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

use nav::pack::{decode, decode_grid, PackError};

/// Largest single allocation observed since the last reset.
static MAX_SINGLE: AtomicUsize = AtomicUsize::new(0);

struct CountingAlloc;

// SAFETY: the allocator records only `layout.size()` in an atomic peak and
// never retains, offsets, or dereferences the pointer; allocation itself is
// delegated to `System`, so alignment and ownership invariants are unchanged.
unsafe impl GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let mut peak = MAX_SINGLE.load(Ordering::Relaxed);
        while layout.size() > peak {
            match MAX_SINGLE.compare_exchange_weak(
                peak,
                layout.size(),
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(observed) => peak = observed,
            }
        }
        // SAFETY: `layout` is the caller's live allocation request, forwarded
        // unchanged to the system allocator.
        System.alloc(layout)
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: `ptr`/`layout` are the pair `System.alloc` returned for this
        // request, forwarded unchanged to the matching deallocation.
        System.dealloc(ptr, layout)
    }
}

#[global_allocator]
static GLOBAL: CountingAlloc = CountingAlloc;

/// Whole-world grid cap from `pack.rs` (`MAX_GRID`). Kept as a literal
/// because the constant is private and this test must not widen product
/// visibility.
const GRID_CAP: u32 = 16384;
/// No single allocation during a truncated-header decode may approach the
/// claimed payload (256 MiB grid walk bytes / 1 GiB pack walk bytes).
const SINGLE_ALLOC_LIMIT: usize = 8 << 20;

fn grid_header(width: u32, height: u32) -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"274N");
    bytes.push(1);
    for value in [0i32, 0, 0] {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    bytes.extend_from_slice(&width.to_le_bytes());
    bytes.extend_from_slice(&height.to_le_bytes());
    bytes
}

fn pack_header(width: u32, height: u32) -> Vec<u8> {
    let mut bytes = b"274V".to_vec();
    bytes.push(nav::pack::VERSION);
    bytes.push(0); // no quest-family binding
    for value in [0i32, 0, 0] {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    bytes.extend_from_slice(&width.to_le_bytes());
    bytes.extend_from_slice(&height.to_le_bytes());
    bytes
}

#[test]
fn truncated_large_headers_fail_before_allocation() {
    MAX_SINGLE.store(0, Ordering::Relaxed);
    let grid = grid_header(GRID_CAP, GRID_CAP);
    assert!(matches!(decode_grid(&grid), Err(PackError::Truncated)));
    assert!(
        MAX_SINGLE.load(Ordering::Relaxed) < SINGLE_ALLOC_LIMIT,
        "truncated {GRID_CAP}x{GRID_CAP} grid allocated {} bytes in one block",
        MAX_SINGLE.load(Ordering::Relaxed)
    );

    MAX_SINGLE.store(0, Ordering::Relaxed);
    let pack = pack_header(GRID_CAP, GRID_CAP);
    assert!(matches!(decode(&pack), Err(PackError::Truncated)));
    assert!(
        MAX_SINGLE.load(Ordering::Relaxed) < SINGLE_ALLOC_LIMIT,
        "truncated {GRID_CAP}x{GRID_CAP} pack allocated {} bytes in one block",
        MAX_SINGLE.load(Ordering::Relaxed)
    );
}
