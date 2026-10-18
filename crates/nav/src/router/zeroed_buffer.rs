use super::SearchCursorError;
use std::ffi::c_void;
use std::mem::{align_of, size_of};
use std::ops::{Deref, DerefMut};
use std::ptr::NonNull;
use std::slice;

const MAPPED_THRESHOLD: usize = 64 * 1024;
const ADMISSION_PAGE_SIZE: usize = 16 * 1024;

const _: () = assert!(size_of::<usize>() == 8);

mod sealed {
    pub trait Sealed {}
}

pub(super) trait ZeroedElement: sealed::Sealed + Copy + Send + Sync {
    const ZERO: Self;
}

impl sealed::Sealed for super::resources::PackedSearchSlot {}
impl ZeroedElement for super::resources::PackedSearchSlot {
    const ZERO: Self = ([0; 3], [0; 8], 0, [0; 3], [0; 3]);
}

impl sealed::Sealed for [u8; 3] {}
impl ZeroedElement for [u8; 3] {
    const ZERO: Self = [0; 3];
}

enum Backing<T: ZeroedElement> {
    Vec(Vec<T>),
    Mapping { bytes: usize },
}

/// An owned, zero-filled slice backed by a fallible Vec or an anonymous mapping.
pub(super) struct ZeroedBuffer<T: ZeroedElement> {
    // This common pointer keeps every slice access independent of the backing kind.
    ptr: NonNull<T>,
    len: usize,
    backing: Backing<T>,
}

impl<T: ZeroedElement> ZeroedBuffer<T> {
    pub(super) fn empty() -> Self {
        let mut values = Vec::new();
        let ptr = vec_ptr(&mut values);
        Self {
            ptr,
            len: 0,
            backing: Backing::Vec(values),
        }
    }

    pub(super) fn try_new(len: usize) -> Result<Self, SearchCursorError> {
        if len == 0 {
            return Err(SearchCursorError::Allocation);
        }
        let payload_bytes = len
            .checked_mul(size_of::<T>())
            .ok_or(SearchCursorError::Allocation)?;
        if payload_bytes < MAPPED_THRESHOLD {
            let mut values = Vec::new();
            values
                .try_reserve_exact(len)
                .map_err(|_| SearchCursorError::Allocation)?;
            values.resize(len, T::ZERO);
            let ptr = vec_ptr(&mut values);
            return Ok(Self {
                ptr,
                len,
                backing: Backing::Vec(values),
            });
        }

        let (ptr, bytes) = map_anonymous::<T>(len)?;
        Ok(Self {
            ptr,
            len,
            backing: Backing::Mapping { bytes },
        })
    }

    pub(super) fn try_grow(
        &mut self,
        new_len: usize,
        initialized_prefix: usize,
    ) -> Result<(), SearchCursorError> {
        let old_len = self.len;
        if initialized_prefix > old_len || old_len > new_len {
            return Err(SearchCursorError::Allocation);
        }
        let new_payload_bytes = new_len
            .checked_mul(size_of::<T>())
            .ok_or(SearchCursorError::Allocation)?;

        if self.is_mapped() || new_payload_bytes >= MAPPED_THRESHOLD {
            let mut replacement = Self::try_new(new_len)?;
            replacement[..initialized_prefix].copy_from_slice(&self[..initialized_prefix]);
            std::mem::swap(self, &mut replacement);
            return Ok(());
        }

        let ptr = {
            let Backing::Vec(values) = &mut self.backing else {
                return Err(SearchCursorError::Allocation);
            };
            values
                .try_reserve_exact(new_len - old_len)
                .map_err(|_| SearchCursorError::Allocation)?;
            values.resize(new_len, T::ZERO);
            values[initialized_prefix..old_len].fill(T::ZERO);
            vec_ptr(values)
        };
        self.ptr = ptr;
        self.len = new_len;
        Ok(())
    }

    fn is_mapped(&self) -> bool {
        matches!(&self.backing, Backing::Mapping { .. })
    }
}

impl<T: ZeroedElement> Deref for ZeroedBuffer<T> {
    type Target = [T];

    fn deref(&self) -> &Self::Target {
        // SAFETY: `ptr` identifies `len` live elements in the owned Vec or mapping. The Vec
        // pointer is refreshed after any growth that can reallocate. The mapping starts at zero,
        // and the sealed ZeroedElement implementations are all-zero-valid. Shared access is read-only.
        unsafe { slice::from_raw_parts(self.ptr.as_ptr(), self.len) }
    }
}

impl<T: ZeroedElement> DerefMut for ZeroedBuffer<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        // SAFETY: `ptr` identifies `len` live elements in the exclusively borrowed Vec or mapping.
        // The Vec pointer is refreshed after any growth that can reallocate. The mapping starts at
        // zero, and the sealed ZeroedElement implementations are all-zero-valid.
        unsafe { slice::from_raw_parts_mut(self.ptr.as_ptr(), self.len) }
    }
}

// SAFETY: ZeroedBuffer owns its allocation, and moving the owner preserves the allocation address.
// ZeroedElement requires Send, and safe access cannot outlive or alias the owner's mutable borrow.
unsafe impl<T: ZeroedElement> Send for ZeroedBuffer<T> {}

// SAFETY: ZeroedBuffer owns its allocation. ZeroedElement requires Sync, and shared access exposes
// only shared references; mutable access requires an exclusive borrow of the owner.
unsafe impl<T: ZeroedElement> Sync for ZeroedBuffer<T> {}

impl<T: ZeroedElement> Drop for ZeroedBuffer<T> {
    fn drop(&mut self) {
        if let Backing::Mapping { bytes } = &self.backing {
            let _ = unmap_owned(self.ptr, *bytes);
        }
    }
}

fn vec_ptr<T>(values: &mut [T]) -> NonNull<T> {
    NonNull::from(values).cast()
}

fn map_anonymous<T: ZeroedElement>(len: usize) -> Result<(NonNull<T>, usize), SearchCursorError> {
    let page_size = system_page_size::<T>()?;
    let mapped_bytes = mapping_length::<T>(len, page_size).ok_or(SearchCursorError::Allocation)?;

    #[cfg(test)]
    if MAPPING_FAILURE.with(|failure| failure.get()) {
        return Err(SearchCursorError::Allocation);
    }

    #[cfg(test)]
    MMAP_CALLS.with(|calls| calls.set(calls.get().saturating_add(1)));

    // SAFETY: The Darwin FFI declaration matches 64-bit size_t, int, and off_t. The flags request
    // a private anonymous read/write mapping, and mapping_length checked a nonzero valid extent.
    let raw = unsafe {
        mmap(
            std::ptr::null_mut(),
            mapped_bytes,
            PROT_READ | PROT_WRITE,
            MAP_PRIVATE | MAP_ANON,
            -1,
            0,
        )
    };
    if raw == (-1isize as *mut c_void) {
        return Err(SearchCursorError::Allocation);
    }
    let Some(ptr) = NonNull::new(raw.cast::<T>()) else {
        // SAFETY: A null return that is not MAP_FAILED is still a successful
        // mapping. It cannot back a Rust slice, so release its exact extent.
        let _ = unsafe { munmap(raw, mapped_bytes) };
        return Err(SearchCursorError::Allocation);
    };
    if (ptr.as_ptr() as usize) & (align_of::<T>() - 1) != 0 {
        let _ = unmap_owned(ptr, mapped_bytes);
        return Err(SearchCursorError::Allocation);
    }
    Ok((ptr, mapped_bytes))
}

fn unmap_owned<T>(ptr: NonNull<T>, bytes: usize) -> std::os::raw::c_int {
    #[cfg(test)]
    MUNMAP_CALLS.with(|calls| calls.set(calls.get().saturating_add(1)));

    // SAFETY: Callers provide the non-null pointer and exact byte extent of a successful mmap.
    // The owner invokes this once, either while rejecting an invalid alignment or on Drop.
    let result = unsafe { munmap(ptr.as_ptr().cast::<c_void>(), bytes) };

    #[cfg(test)]
    if result == 0 {
        MUNMAP_SUCCESSES.with(|successes| successes.set(successes.get().saturating_add(1)));
    }
    result
}

fn system_page_size<T: ZeroedElement>() -> Result<usize, SearchCursorError> {
    // SAFETY: getpagesize takes no arguments and returns the host page size as a C int.
    let page_size = unsafe { getpagesize() };
    let page_size = usize::try_from(page_size).map_err(|_| SearchCursorError::Allocation)?;
    if valid_page_size::<T>(page_size) {
        Ok(page_size)
    } else {
        Err(SearchCursorError::Allocation)
    }
}

fn valid_page_size<T: ZeroedElement>(page_size: usize) -> bool {
    page_size != 0
        && page_size <= ADMISSION_PAGE_SIZE
        && page_size.is_power_of_two()
        && page_size >= align_of::<T>()
}

fn mapping_length<T: ZeroedElement>(len: usize, page_size: usize) -> Option<usize> {
    if len == 0 || !valid_page_size::<T>(page_size) {
        return None;
    }
    let payload_bytes = len.checked_mul(size_of::<T>())?;
    if payload_bytes == 0 || payload_bytes > isize::MAX as usize {
        return None;
    }
    let mapped_bytes = round_up(payload_bytes, page_size)?;
    (mapped_bytes <= isize::MAX as usize).then_some(mapped_bytes)
}

fn round_up(value: usize, alignment: usize) -> Option<usize> {
    if value == 0 || alignment == 0 || !alignment.is_power_of_two() {
        return None;
    }
    value
        .checked_add(alignment - 1)
        .map(|rounded| rounded & !(alignment - 1))
}

pub(super) fn storage_bound<T: ZeroedElement>(len: usize) -> Option<usize> {
    let payload_bytes = len.checked_mul(size_of::<T>())?;
    if payload_bytes < MAPPED_THRESHOLD {
        Some(payload_bytes)
    } else {
        round_up(payload_bytes, ADMISSION_PAGE_SIZE)
    }
}

#[cfg(test)]
thread_local! {
    static MAPPING_FAILURE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static MMAP_CALLS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static MUNMAP_CALLS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static MUNMAP_SUCCESSES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
pub(super) fn with_mapping_failure<R>(f: impl FnOnce() -> R) -> R {
    struct RestoreFailure(bool);

    impl Drop for RestoreFailure {
        fn drop(&mut self) {
            MAPPING_FAILURE.with(|failure| failure.set(self.0));
        }
    }

    let previous = MAPPING_FAILURE.with(|failure| failure.replace(true));
    let _restore = RestoreFailure(previous);
    f()
}

#[cfg(test)]
#[derive(Clone, Copy)]
struct SyscallCounts {
    mmap: usize,
    munmap: usize,
    successful_munmap: usize,
}

#[cfg(test)]
fn syscall_counts() -> SyscallCounts {
    SyscallCounts {
        mmap: MMAP_CALLS.with(|calls| calls.get()),
        munmap: MUNMAP_CALLS.with(|calls| calls.get()),
        successful_munmap: MUNMAP_SUCCESSES.with(|successes| successes.get()),
    }
}

#[cfg(test)]
const TASK_VM_INFO_FLAVOR: std::os::raw::c_int = 22;
#[cfg(test)]
const TASK_VM_INFO_REV1_COUNT: u32 = 38;

#[cfg(test)]
pub(super) fn current_phys_footprint() -> u64 {
    let mut info = [0u64; 19];
    let mut count = TASK_VM_INFO_REV1_COUNT;

    // SAFETY: MACH_TASK_SELF is the current task port. task_info writes REV1's 38 integer_t
    // cells into the aligned 19-u64 buffer, and count gives the exact capacity in integer_t cells.
    let result = unsafe {
        task_info(
            MACH_TASK_SELF,
            TASK_VM_INFO_FLAVOR,
            info.as_mut_ptr().cast::<std::os::raw::c_int>(),
            &mut count,
        )
    };
    assert_eq!(result, 0, "task_info(TASK_VM_INFO) failed");
    assert_eq!(count, TASK_VM_INFO_REV1_COUNT);
    info[18]
}

const PROT_READ: std::os::raw::c_int = 0x1;
const PROT_WRITE: std::os::raw::c_int = 0x2;
const MAP_PRIVATE: std::os::raw::c_int = 0x2;
const MAP_ANON: std::os::raw::c_int = 0x1000;

// SAFETY: These declarations match the 64-bit Darwin System ABI. The caller supplies valid
// mappings and buffers to mmap, munmap, and task_info, and getpagesize has no arguments.
#[link(name = "System")]
unsafe extern "C" {
    fn getpagesize() -> std::os::raw::c_int;
    fn mmap(
        address: *mut c_void,
        length: usize,
        protection: std::os::raw::c_int,
        flags: std::os::raw::c_int,
        descriptor: std::os::raw::c_int,
        offset: i64,
    ) -> *mut c_void;
    fn munmap(address: *mut c_void, length: usize) -> std::os::raw::c_int;
    #[cfg(test)]
    #[link_name = "mach_task_self_"]
    static MACH_TASK_SELF: u32;
    #[cfg(test)]
    fn task_info(
        target_task: u32,
        flavor: std::os::raw::c_int,
        task_info_out: *mut std::os::raw::c_int,
        task_info_out_count: *mut u32,
    ) -> std::os::raw::c_int;
}

#[cfg(test)]
mod tests {
    use super::super::resources::PackedSearchSlot;
    use super::*;
    use std::fmt::Debug;

    fn mapped_len<T: ZeroedElement>() -> usize {
        MAPPED_THRESHOLD / size_of::<T>() + 1
    }

    fn assert_fresh_mapping_is_zero<T>()
    where
        T: ZeroedElement + PartialEq + Debug,
    {
        let values = ZeroedBuffer::<T>::try_new(mapped_len::<T>()).unwrap();
        assert!(values.is_mapped());
        assert!(values.iter().all(|value| *value == T::ZERO));
    }

    #[test]
    fn fresh_mapped_buffers_are_zero_and_mutable_at_both_ends() {
        assert_fresh_mapping_is_zero::<PackedSearchSlot>();
        assert_fresh_mapping_is_zero::<[u8; 3]>();

        let mut slots =
            ZeroedBuffer::<PackedSearchSlot>::try_new(mapped_len::<PackedSearchSlot>()).unwrap();
        let first = ([1, 2, 3], [4; 8], 5, [6; 3], [7; 3]);
        let last = ([8, 9, 10], [11; 8], 12, [13; 3], [14; 3]);
        {
            let values: &mut [PackedSearchSlot] = &mut slots;
            values[0] = first;
            let last_index = values.len() - 1;
            values[last_index] = last;
        }
        assert_eq!(slots[0], first);
        assert_eq!(slots[slots.len() - 1], last);

        let mut links = ZeroedBuffer::<[u8; 3]>::try_new(mapped_len::<[u8; 3]>()).unwrap();
        let first_link = [1, 2, 3];
        let last_link = [4, 5, 6];
        {
            let values: &mut [[u8; 3]] = &mut links;
            values[0] = first_link;
            let last_index = values.len() - 1;
            values[last_index] = last_link;
        }
        assert_eq!(links[0], first_link);
        assert_eq!(links[links.len() - 1], last_link);
    }

    #[test]
    fn mapped_owner_moves_across_a_thread() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<ZeroedBuffer<[u8; 3]>>();

        let mut values = ZeroedBuffer::<[u8; 3]>::try_new(mapped_len::<[u8; 3]>()).unwrap();
        values[0] = [1, 2, 3];
        let last_index = values.len() - 1;
        values[last_index] = [4, 5, 6];

        let values = std::thread::spawn(move || {
            assert_eq!(values[0], [1, 2, 3]);
            assert_eq!(values[values.len() - 1], [4, 5, 6]);
            values
        })
        .join()
        .unwrap();
        assert_eq!(values[0], [1, 2, 3]);
        assert_eq!(values[values.len() - 1], [4, 5, 6]);
    }

    #[test]
    #[ignore = "run alone on macOS for native phys_footprint measurement"]
    fn mapped_lazy_commit_releases_phys_footprint_after_drop() {
        const RECORDS: usize = 4_000_002;
        const MAPPED_PAYLOAD: usize = RECORDS * size_of::<PackedSearchSlot>();
        const TOUCH_LEN: usize = (16 * 1024) / size_of::<PackedSearchSlot>();
        const TOUCH_PAYLOAD: usize = TOUCH_LEN * size_of::<PackedSearchSlot>();

        let baseline = current_phys_footprint();
        let mut values = ZeroedBuffer::<PackedSearchSlot>::try_new(RECORDS).unwrap();
        assert!(values.is_mapped());
        let after_map = current_phys_footprint();
        assert!(
            after_map.saturating_sub(baseline) < MAPPED_PAYLOAD / 4,
            "mapping {MAPPED_PAYLOAD} bytes grew phys_footprint by {} bytes",
            after_map.saturating_sub(baseline)
        );

        for (index, slot) in values[..TOUCH_LEN].iter_mut().enumerate() {
            *slot = ([index as i32 + 1; 3], [0xa5; 8], 1, [0x5a; 3], [0xc3; 3]);
            std::hint::black_box(slot);
        }
        let after_touch = current_phys_footprint();
        let touched_growth = after_touch.saturating_sub(baseline);
        assert!(TOUCH_PAYLOAD <= 16 * 1024);
        assert!(
            touched_growth > 0 && touched_growth < MAPPED_PAYLOAD / 4,
            "writing {TOUCH_PAYLOAD} bytes grew phys_footprint by {touched_growth} bytes"
        );

        drop(values);
        let after_drop = current_phys_footprint();
        eprintln!(
            "MAPPED_PHYS_FOOTPRINT payload={MAPPED_PAYLOAD} touched={TOUCH_PAYLOAD} baseline={baseline} after_map={after_map} after_touch={after_touch} after_drop={after_drop}"
        );
        assert!(
            after_drop < after_touch,
            "phys_footprint did not fall after munmap: {after_touch} to {after_drop}"
        );
    }
    #[test]
    fn mapping_rejects_zero_overflow_and_unsupported_pages() {
        assert!(matches!(
            ZeroedBuffer::<PackedSearchSlot>::try_new(0),
            Err(SearchCursorError::Allocation)
        ));
        assert_eq!(mapping_length::<PackedSearchSlot>(0, 4096), None);
        assert_eq!(mapping_length::<PackedSearchSlot>(usize::MAX, 4096), None);
        assert_eq!(
            mapping_length::<PackedSearchSlot>(1, ADMISSION_PAGE_SIZE * 2),
            None
        );
        assert_eq!(mapping_length::<PackedSearchSlot>(1, 0), None);
    }

    #[test]
    fn storage_bound_rounds_only_mapped_payloads() {
        assert_eq!(storage_bound::<PackedSearchSlot>(0), Some(0));
        assert_eq!(storage_bound::<PackedSearchSlot>(2340), Some(65_520));
        assert_eq!(storage_bound::<PackedSearchSlot>(2341), Some(81_920));
        assert_eq!(
            storage_bound::<PackedSearchSlot>(4_000_002),
            Some(112_001_024)
        );
        assert_eq!(storage_bound::<PackedSearchSlot>(usize::MAX), None);
    }

    #[test]
    fn growth_preserves_only_the_initialized_prefix_and_zeroes_the_tail() {
        let mut small = ZeroedBuffer::<[u8; 3]>::try_new(4).unwrap();
        small.copy_from_slice(&[[1; 3], [2; 3], [3; 3], [4; 3]]);
        small.try_grow(8, 2).unwrap();
        assert_eq!(&small[..2], &[[1; 3], [2; 3]]);
        assert!(small[2..].iter().all(|value| *value == [0; 3]));

        let small_len = MAPPED_THRESHOLD / size_of::<[u8; 3]>();
        let mut crossing = ZeroedBuffer::<[u8; 3]>::try_new(small_len).unwrap();
        crossing[0] = [9; 3];
        crossing[1] = [8; 3];
        crossing[2] = [7; 3];
        let mapped_len = small_len + 1;
        crossing.try_grow(mapped_len, 2).unwrap();
        assert!(crossing.is_mapped());
        assert_eq!(&crossing[..2], &[[9; 3], [8; 3]]);
        assert!(crossing[2..].iter().all(|value| *value == [0; 3]));

        let mut mapped = ZeroedBuffer::<[u8; 3]>::try_new(mapped_len).unwrap();
        mapped[0] = [6; 3];
        mapped[1] = [5; 3];
        mapped[2] = [4; 3];
        mapped.try_grow(mapped_len + 1, 1).unwrap();
        assert_eq!(mapped[0], [6; 3]);
        assert!(mapped[1..].iter().all(|value| *value == [0; 3]));
    }

    #[test]
    fn invalid_growth_is_rejected_without_changing_the_buffer() {
        let mut values = ZeroedBuffer::<[u8; 3]>::try_new(3).unwrap();
        values.copy_from_slice(&[[1; 3], [2; 3], [3; 3]]);
        let original = values.to_vec();
        assert!(matches!(
            values.try_grow(2, 1),
            Err(SearchCursorError::Allocation)
        ));
        assert!(matches!(
            values.try_grow(4, 4),
            Err(SearchCursorError::Allocation)
        ));
        assert_eq!(&*values, original.as_slice());
    }

    #[test]
    fn injected_mapping_failure_returns_allocation_and_keeps_old_buffer() {
        let mut values = ZeroedBuffer::<[u8; 3]>::try_new(4).unwrap();
        values.copy_from_slice(&[[1; 3], [2; 3], [3; 3], [4; 3]]);
        let original = values.to_vec();
        let mapped_len = mapped_len::<[u8; 3]>();
        let before = syscall_counts();

        with_mapping_failure(|| {
            assert!(ZeroedBuffer::<[u8; 3]>::try_new(1).is_ok());
            assert!(matches!(
                ZeroedBuffer::<[u8; 3]>::try_new(mapped_len),
                Err(SearchCursorError::Allocation)
            ));
            assert!(matches!(
                values.try_grow(mapped_len, 1),
                Err(SearchCursorError::Allocation)
            ));
        });

        assert_eq!(&*values, original.as_slice());
        let after = syscall_counts();
        assert_eq!(after.mmap - before.mmap, 0);
        assert_eq!(after.munmap - before.munmap, 0);
    }

    #[test]
    fn mapped_owner_calls_successful_munmap_once_on_drop() {
        let before = syscall_counts();
        let values = ZeroedBuffer::<[u8; 3]>::try_new(mapped_len::<[u8; 3]>()).unwrap();
        let after_map = syscall_counts();
        assert_eq!(after_map.mmap - before.mmap, 1);

        drop(values);
        let after_drop = syscall_counts();
        assert_eq!(after_drop.munmap - before.munmap, 1);
        assert_eq!(after_drop.successful_munmap - before.successful_munmap, 1);
    }
}
