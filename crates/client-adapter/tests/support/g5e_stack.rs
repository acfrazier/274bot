//! Native stack facts for the current test worker.
//!
//! The OS-reported stack interval is authoritative; this module never searches
//! anonymous mappings to guess which mapping belongs to the current thread.

use std::ffi::c_void;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct StackSample {
    /// Kernel-assigned native thread ID; never a process ID or `pthread_t`.
    pub(crate) tid: u64,
    /// OS-reported bounds for the current thread's native stack interval.
    pub(crate) stack_low_address: Option<u64>,
    /// Exclusive upper bound for the current thread's native stack interval.
    pub(crate) stack_high_address_exclusive: Option<u64>,
    /// Bytes in the native stack interval; not a resident-byte measurement.
    pub(crate) reserved_bytes: Option<u64>,
    /// Bytes the OS identifies as committed; never substituted with residency.
    pub(crate) committed_bytes: Option<u64>,
    /// Bytes within the native interval whose pages `mincore` reports resident.
    pub(crate) resident_bytes: Option<u64>,
    /// OS-reported guard bytes; `None` when the current guard cannot be verified.
    pub(crate) guard_bytes: Option<u64>,
    /// Names the native APIs and semantics used for the non-null fields.
    pub(crate) measurement_source: &'static str,
}

/// Captures native stack facts for the thread that calls this function.
pub(crate) fn capture_current_thread_stack() -> Result<StackSample, String> {
    #[cfg(target_os = "linux")]
    {
        capture_linux()
    }
    #[cfg(target_os = "macos")]
    {
        capture_macos()
    }
    #[cfg(target_os = "windows")]
    {
        capture_windows()
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        Err("current-thread stack measurement is unsupported on this target".to_owned())
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn count_resident_bytes(stack_low: u64, stack_high: u64) -> Option<u64> {
    const CHUNK_PAGES: usize = 256;

    if stack_low >= stack_high {
        return None;
    }

    // SAFETY: sysconf is a read-only query and _SC_PAGESIZE is valid here.
    let page_size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
    let page_size = u64::try_from(page_size).ok().filter(|size| *size != 0)?;
    let first_page = (stack_low / page_size).checked_mul(page_size)?;
    let end_page = stack_high.div_ceil(page_size).checked_mul(page_size)?;

    let mut remaining_pages = (end_page - first_page) / page_size;
    let mut address = first_page;
    let mut resident_bytes = 0_u64;
    let mut residency = [0 as libc::c_char; CHUNK_PAGES];

    while remaining_pages != 0 {
        let pages = remaining_pages.min(CHUNK_PAGES as u64);
        let byte_count = pages.checked_mul(page_size)?;
        let byte_count = usize::try_from(byte_count).ok()?;
        let address_usize = usize::try_from(address).ok()?;

        // SAFETY: mincore requires page alignment. Query every page covering
        // the OS stack interval; boundary pages are clipped when counted.
        // The buffer has one byte for every queried page.
        let result = unsafe {
            libc::mincore(
                address_usize as *mut c_void,
                byte_count,
                residency.as_mut_ptr().cast(),
            )
        };
        if result != 0 {
            return None;
        }

        for (index, status) in residency.iter().take(pages as usize).enumerate() {
            if *status as u8 & 1 != 0 {
                let page_low = address.checked_add((index as u64).checked_mul(page_size)?)?;
                let page_high = page_low.checked_add(page_size)?;
                let covered = page_high.min(stack_high) - page_low.max(stack_low);
                resident_bytes = resident_bytes.checked_add(covered)?;
            }
        }
        address = address.checked_add(byte_count as u64)?;
        remaining_pages -= pages;
    }

    Some(resident_bytes)
}

#[cfg(target_os = "linux")]
fn capture_linux() -> Result<StackSample, String> {
    use std::mem::MaybeUninit;

    let raw_tid =
        // SAFETY: SYS_gettid takes no additional arguments and has no side effects.
        unsafe { libc::syscall(libc::SYS_gettid) };
    let tid = u64::try_from(raw_tid)
        .ok()
        .filter(|tid| *tid != 0)
        .ok_or_else(|| format!("SYS_gettid returned invalid thread ID {raw_tid}"))?;

    let pthread =
        // SAFETY: pthread_self returns the calling thread's pthread identifier.
        unsafe { libc::pthread_self() };
    let mut attributes = MaybeUninit::<libc::pthread_attr_t>::uninit();
    // SAFETY: pthread_getattr_np initializes `attributes` for the live calling
    // thread when it returns zero.
    let result = unsafe { libc::pthread_getattr_np(pthread, attributes.as_mut_ptr()) };
    if result != 0 {
        return Err(format!("pthread_getattr_np failed with {result}"));
    }
    // SAFETY: pthread_getattr_np succeeded and initialized the attributes.
    let attributes = unsafe { attributes.assume_init() };
    let attributes = PthreadAttributes(attributes);

    let mut stack_address = std::ptr::null_mut();
    let mut stack_size = 0;
    // SAFETY: `attributes` is initialized and the output pointers are valid.
    let result =
        unsafe { libc::pthread_attr_getstack(&attributes.0, &mut stack_address, &mut stack_size) };
    if result != 0 {
        return Err(format!("pthread_attr_getstack failed with {result}"));
    }
    if stack_address.is_null() || stack_size == 0 {
        return Err("pthread_attr_getstack returned an empty stack interval".to_owned());
    }

    let stack_low = stack_address as usize;
    let stack_high = stack_low
        .checked_add(stack_size)
        .ok_or_else(|| "pthread stack interval overflowed address space".to_owned())?;
    ensure_local_address_is_in_stack(stack_low, stack_high)?;

    let mut guard_size = 0;
    // SAFETY: `attributes` is initialized and the output pointer is valid.
    let result = unsafe { libc::pthread_attr_getguardsize(&attributes.0, &mut guard_size) };
    let guard_bytes = if result == 0 && guard_size <= stack_size {
        Some(guard_size as u64)
    } else {
        None
    };

    let stack_low_u64 = u64::try_from(stack_low)
        .map_err(|_| "pthread stack lower bound does not fit u64".to_owned())?;
    let stack_high_u64 = u64::try_from(stack_high)
        .map_err(|_| "pthread stack upper bound does not fit u64".to_owned())?;
    let reserved_bytes =
        u64::try_from(stack_size).map_err(|_| "pthread stack size does not fit u64".to_owned())?;

    Ok(StackSample {
        tid,
        stack_low_address: Some(stack_low_u64),
        stack_high_address_exclusive: Some(stack_high_u64),
        reserved_bytes: Some(reserved_bytes),
        committed_bytes: None,
        resident_bytes: count_resident_bytes(stack_low_u64, stack_high_u64),
        guard_bytes,
        measurement_source: "linux:SYS_gettid+pthread_getattr_np/pthread_attr_getstack;reserved=pthread-stack-interval-size;guard=pthread_attr_getguardsize-reported;mincore-resident-only;commit-unavailable",
    })
}

#[cfg(target_os = "linux")]
struct PthreadAttributes(libc::pthread_attr_t);

#[cfg(target_os = "linux")]
impl Drop for PthreadAttributes {
    fn drop(&mut self) {
        // SAFETY: this value was initialized by pthread_getattr_np and is
        // destroyed exactly once when the capture finishes.
        let _result = unsafe { libc::pthread_attr_destroy(&mut self.0) };
    }
}

#[cfg(target_os = "macos")]
fn capture_macos() -> Result<StackSample, String> {
    let pthread =
        // SAFETY: pthread_self returns the calling thread's pthread identifier.
        unsafe { libc::pthread_self() };
    let mut tid = 0_u64;
    // SAFETY: `pthread` identifies the calling thread and `tid` is writable.
    let result = unsafe { libc::pthread_threadid_np(pthread, &mut tid) };
    if result != 0 || tid == 0 {
        return Err(format!("pthread_threadid_np failed with {result}"));
    }

    // SAFETY: both functions query stack properties for the calling thread.
    let stack_high = unsafe { libc::pthread_get_stackaddr_np(pthread) as usize };
    // SAFETY: pthread_get_stacksize_np queries the calling thread's stack size.
    let stack_size = unsafe { libc::pthread_get_stacksize_np(pthread) };
    if stack_high == 0 || stack_size == 0 {
        return Err("pthread stack API returned an empty stack interval".to_owned());
    }
    let stack_low = stack_high
        .checked_sub(stack_size)
        .ok_or_else(|| "pthread stack interval underflowed address space".to_owned())?;
    ensure_local_address_is_in_stack(stack_low, stack_high)?;

    let stack_low_u64 = u64::try_from(stack_low)
        .map_err(|_| "pthread stack lower bound does not fit u64".to_owned())?;
    let stack_high_u64 = u64::try_from(stack_high)
        .map_err(|_| "pthread stack upper bound does not fit u64".to_owned())?;
    let reserved_bytes =
        u64::try_from(stack_size).map_err(|_| "pthread stack size does not fit u64".to_owned())?;

    Ok(StackSample {
        tid,
        stack_low_address: Some(stack_low_u64),
        stack_high_address_exclusive: Some(stack_high_u64),
        reserved_bytes: Some(reserved_bytes),
        committed_bytes: None,
        resident_bytes: count_resident_bytes(stack_low_u64, stack_high_u64),
        guard_bytes: None,
        measurement_source: "macos:pthread_threadid_np+pthread_get_stackaddr_np/pthread_get_stacksize_np;reserved=pthread-stack-interval-size;mincore-resident-only;commit-unavailable;guard-unavailable",
    })
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn ensure_local_address_is_in_stack(stack_low: usize, stack_high: usize) -> Result<(), String> {
    let marker = 0_u8;
    let marker_address = std::ptr::addr_of!(marker) as usize;
    if marker_address < stack_low || marker_address >= stack_high {
        return Err("native stack interval does not contain the calling thread's local".to_owned());
    }
    Ok(())
}

#[cfg(target_os = "windows")]
fn capture_windows() -> Result<StackSample, String> {
    const MEM_COMMIT: u32 = 0x1000;
    const MEM_RESERVE: u32 = 0x2000;
    const PAGE_GUARD: u32 = 0x0100;
    const PAGE_NOACCESS: u32 = 0x0001;

    let tid =
        // SAFETY: GetCurrentThreadId is a read-only query of the calling thread.
        unsafe { GetCurrentThreadId() };
    if tid == 0 {
        return Err("GetCurrentThreadId returned zero".to_owned());
    }

    let mut current_low_limit = 0_usize;
    let mut stack_high = 0_usize;
    // SAFETY: both pointers are writable and GetCurrentThreadStackLimits fills
    // them with boundaries for the calling thread.
    unsafe { GetCurrentThreadStackLimits(&mut current_low_limit, &mut stack_high) };
    if current_low_limit == 0 || stack_high <= current_low_limit {
        return Err("GetCurrentThreadStackLimits returned invalid bounds".to_owned());
    }

    let marker = 0_u8;
    let marker_address = std::ptr::addr_of!(marker) as usize;
    if marker_address < current_low_limit || marker_address >= stack_high {
        return Err(
            "GetCurrentThreadStackLimits does not contain the calling thread's local".to_owned(),
        );
    }
    let marker_info = query_memory(marker_address)?;
    let allocation_base = marker_info.allocation_base as usize;
    if allocation_base == 0 || allocation_base > current_low_limit || stack_high <= allocation_base
    {
        return Err("VirtualQuery returned an invalid stack allocation base".to_owned());
    }

    let mut cursor = allocation_base;
    let mut reserved_bytes = 0_u64;
    let mut committed_bytes = 0_u64;
    let mut guard_bytes = 0_u64;
    while cursor < stack_high {
        let information = query_memory(cursor)?;
        if information.allocation_base as usize != allocation_base {
            return Err("VirtualQuery left the current thread's stack allocation".to_owned());
        }
        let region_base = information.base_address as usize;
        let region_end = region_base
            .checked_add(information.region_size)
            .ok_or_else(|| "VirtualQuery region overflowed address space".to_owned())?;
        if region_base > cursor || region_end <= cursor {
            return Err("VirtualQuery returned a non-covering stack region".to_owned());
        }
        let covered_end = region_end.min(stack_high);
        let covered_bytes = u64::try_from(covered_end - cursor)
            .map_err(|_| "VirtualQuery region size does not fit u64".to_owned())?;

        match information.state {
            MEM_COMMIT => {
                committed_bytes = committed_bytes
                    .checked_add(covered_bytes)
                    .ok_or_else(|| "committed stack byte count overflowed".to_owned())?;
                let base_protection = information.protect & 0xff;
                if information.protect & PAGE_GUARD != 0 || base_protection == PAGE_NOACCESS {
                    guard_bytes = guard_bytes
                        .checked_add(covered_bytes)
                        .ok_or_else(|| "stack guard byte count overflowed".to_owned())?;
                }
            }
            MEM_RESERVE => {}
            state => {
                return Err(format!(
                    "VirtualQuery returned unexpected stack state {state:#x}"
                ))
            }
        }
        reserved_bytes = reserved_bytes
            .checked_add(covered_bytes)
            .ok_or_else(|| "reserved stack byte count overflowed".to_owned())?;
        cursor = covered_end;
    }

    let stack_low_u64 = u64::try_from(allocation_base)
        .map_err(|_| "stack allocation base does not fit u64".to_owned())?;
    let stack_high_u64 =
        u64::try_from(stack_high).map_err(|_| "stack upper bound does not fit u64".to_owned())?;
    let address_span = stack_high_u64
        .checked_sub(stack_low_u64)
        .ok_or_else(|| "stack allocation bounds are reversed".to_owned())?;
    if reserved_bytes != address_span || committed_bytes == 0 || guard_bytes > committed_bytes {
        return Err("VirtualQuery stack totals do not match the observed allocation".to_owned());
    }

    Ok(StackSample {
        tid: u64::from(tid),
        stack_low_address: Some(stack_low_u64),
        stack_high_address_exclusive: Some(stack_high_u64),
        reserved_bytes: Some(reserved_bytes),
        committed_bytes: Some(committed_bytes),
        resident_bytes: None,
        guard_bytes: Some(guard_bytes),
        measurement_source: "windows:GetCurrentThreadId+GetCurrentThreadStackLimits+VirtualQuery;reserved=MEM_RESERVE+MEM_COMMIT-allocation;commit=MEM_COMMIT;guard=committed-PAGE_GUARD-or-PAGE_NOACCESS;resident-unavailable",
    })
}

#[cfg(target_os = "windows")]
#[repr(C)]
struct MemoryBasicInformation {
    base_address: *mut c_void,
    allocation_base: *mut c_void,
    _allocation_protect: u32,
    _partition_id: u16,
    region_size: usize,
    state: u32,
    protect: u32,
    _memory_type: u32,
}

#[cfg(target_os = "windows")]
fn query_memory(address: usize) -> Result<MemoryBasicInformation, String> {
    use std::mem::{size_of, MaybeUninit};

    let mut information = MaybeUninit::<MemoryBasicInformation>::zeroed();
    // SAFETY: `information` points to a writable MEMORY_BASIC_INFORMATION
    // buffer of the exact size requested by VirtualQuery.
    let returned = unsafe {
        VirtualQuery(
            address as *const c_void,
            information.as_mut_ptr(),
            size_of::<MemoryBasicInformation>(),
        )
    };
    if returned < size_of::<MemoryBasicInformation>() {
        return Err(format!("VirtualQuery failed for address {address:#x}"));
    }
    // SAFETY: VirtualQuery returned at least the complete structure size.
    Ok(unsafe { information.assume_init() })
}

#[cfg(target_os = "windows")]
#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetCurrentThreadId() -> u32;
    fn GetCurrentThreadStackLimits(low_limit: *mut usize, high_limit: *mut usize);
    fn VirtualQuery(
        address: *const c_void,
        information: *mut MemoryBasicInformation,
        length: usize,
    ) -> usize;
}

#[cfg(all(
    test,
    any(target_os = "linux", target_os = "macos", target_os = "windows")
))]
mod tests {
    use super::capture_current_thread_stack;
    use std::thread;

    const REQUESTED_STACK_BYTES: usize = 2 * 1024 * 1024;

    #[test]
    fn current_worker_sample_matches_native_stack_and_requested_size() {
        let parent_tid = capture_current_thread_stack()
            .expect("capture parent thread stack")
            .tid;
        let (sample, local_address) = thread::Builder::new()
            .stack_size(REQUESTED_STACK_BYTES)
            .spawn(|| {
                let mut stack_local = 0_u8;
                let stack_local_pointer = std::ptr::addr_of_mut!(stack_local);
                // SAFETY: the pointer addresses the live local for this scope.
                unsafe { stack_local_pointer.write_volatile(0x5a) };
                let local_address = stack_local_pointer as usize as u64;
                #[cfg(any(target_os = "linux", target_os = "macos"))]
                assert_eq!(
                    super::count_resident_bytes(local_address, local_address + 1),
                    Some(1),
                    "a touched partial page contributes only its stack-interval bytes"
                );
                let sample = capture_current_thread_stack().expect("capture worker stack");
                std::hint::black_box(stack_local);
                (sample, local_address)
            })
            .expect("spawn worker with an explicit stack request")
            .join()
            .expect("stack probe worker did not panic");

        assert_ne!(
            sample.tid, parent_tid,
            "worker TID must differ from caller TID"
        );
        assert_ne!(sample.tid, 0, "native thread IDs are positive");
        let stack_low = sample.stack_low_address.expect("native stack lower bound");
        let stack_high = sample
            .stack_high_address_exclusive
            .expect("native stack upper bound");
        assert!(stack_low < stack_high, "stack bounds must be increasing");
        assert!(
            stack_low <= local_address && local_address < stack_high,
            "known worker-local address must lie within native stack bounds"
        );
        let reserved = sample
            .reserved_bytes
            .expect("native reserved stack interval");
        assert_eq!(reserved, stack_high - stack_low);
        assert!(
            reserved >= REQUESTED_STACK_BYTES as u64,
            "native reservation must cover the explicit Builder stack-size request"
        );

        #[cfg(any(target_os = "linux", target_os = "macos"))]
        {
            assert_eq!(
                sample.committed_bytes, None,
                "commit is not exposed per stack"
            );
            let resident = sample
                .resident_bytes
                .expect("mincore residency query for native stack interval");
            assert!(resident > 0, "the touched worker stack has resident pages");
            assert!(resident <= reserved);
            assert!(sample.measurement_source.contains("mincore-resident-only"));
        }

        #[cfg(target_os = "linux")]
        {
            let guard = sample.guard_bytes.expect("pthread guard-size attribute");
            assert!(guard <= reserved);
            assert!(sample
                .measurement_source
                .contains("pthread_attr_getguardsize"));
            assert!(sample.measurement_source.contains("SYS_gettid"));
            assert!(sample.measurement_source.contains("pthread_attr_getstack"));
        }

        #[cfg(target_os = "macos")]
        {
            assert_eq!(
                sample.guard_bytes, None,
                "current-thread guard is not verified"
            );
            assert!(sample.measurement_source.contains("pthread_threadid_np"));
            assert!(sample.measurement_source.contains("guard-unavailable"));
            assert!(sample
                .measurement_source
                .contains("pthread_get_stackaddr_np"));
            assert!(sample
                .measurement_source
                .contains("pthread_get_stacksize_np"));
        }

        #[cfg(target_os = "windows")]
        {
            let committed = sample.committed_bytes.expect("VirtualQuery commit total");
            let guard = sample.guard_bytes.expect("VirtualQuery guard total");
            assert!(committed > 0 && committed <= reserved);
            assert!(guard > 0 && guard <= committed);
            assert_eq!(
                sample.resident_bytes, None,
                "resident pages are unavailable"
            );
            assert!(sample.measurement_source.contains("VirtualQuery"));
            assert!(sample.measurement_source.contains("MEM_COMMIT"));
            assert!(sample.measurement_source.contains("GetCurrentThreadId"));
            assert!(sample
                .measurement_source
                .contains("GetCurrentThreadStackLimits"));
        }
    }
}
