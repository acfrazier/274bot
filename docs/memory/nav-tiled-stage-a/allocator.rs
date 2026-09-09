use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicU64, Ordering};
static ALLOCS: AtomicU64 = AtomicU64::new(0);
static FREES: AtomicU64 = AtomicU64::new(0);
static REQUESTED: AtomicU64 = AtomicU64::new(0);
static RELEASED: AtomicU64 = AtomicU64::new(0);
struct Counting;
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        let p = System.alloc(l);
        if !p.is_null() {
            ALLOCS.fetch_add(1, Ordering::Relaxed);
            REQUESTED.fetch_add(l.size() as u64, Ordering::Relaxed);
        }
        p
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        FREES.fetch_add(1, Ordering::Relaxed);
        RELEASED.fetch_add(l.size() as u64, Ordering::Relaxed);
        System.dealloc(p, l);
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
        let q = System.realloc(p, l, n);
        if !q.is_null() {
            ALLOCS.fetch_add(1, Ordering::Relaxed);
            FREES.fetch_add(1, Ordering::Relaxed);
            REQUESTED.fetch_add(n as u64, Ordering::Relaxed);
            RELEASED.fetch_add(l.size() as u64, Ordering::Relaxed);
        }
        q
    }
}
#[cfg(feature = "stage-counting")]
#[global_allocator]
static ALLOCATOR: Counting = Counting;
#[cfg(not(feature = "stage-counting"))]
#[global_allocator]
static ALLOCATOR: System = System;
fn counts() -> [u64; 4] {
    [
        ALLOCS.load(Ordering::Relaxed),
        FREES.load(Ordering::Relaxed),
        REQUESTED.load(Ordering::Relaxed),
        RELEASED.load(Ordering::Relaxed),
    ]
}
