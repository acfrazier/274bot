//! Native stack facts for the current test worker.
//!
//! The OS-reported stack interval is authoritative. Platform maps are only
//! matched against that interval; anonymous mappings are never guessed.

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
    /// Bytes the OS identifies as committed or backed; never just residency.
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
    let map_attribution = inspect_linux_proc_maps(stack_low_u64, stack_high_u64);
    let reserved_bytes = match map_attribution {
        ProcMapsAttribution::Unique => Some(
            u64::try_from(stack_size)
                .map_err(|_| "pthread stack size does not fit u64".to_owned())?,
        ),
        ProcMapsAttribution::Unavailable | ProcMapsAttribution::Ambiguous => None,
    };

    Ok(StackSample {
        tid,
        stack_low_address: Some(stack_low_u64),
        stack_high_address_exclusive: Some(stack_high_u64),
        reserved_bytes,
        committed_bytes: None,
        resident_bytes: count_resident_bytes(stack_low_u64, stack_high_u64),
        guard_bytes,
        measurement_source: match map_attribution {
            ProcMapsAttribution::Unique => {
                "linux:SYS_gettid+pthread_getattr_np/pthread_attr_getstack+/proc/self/maps;proc-maps=unique-covering-pthread-interval;reserved=pthread-stack-interval-confirmed-by-maps;guard=pthread_attr_getguardsize-reported;mincore-resident-only;commit-unavailable"
            }
            ProcMapsAttribution::Unavailable => {
                "linux:SYS_gettid+pthread_getattr_np/pthread_attr_getstack+/proc/self/maps;proc-maps=unavailable;reserved=unresolved;guard=pthread_attr_getguardsize-reported;mincore-resident-only;commit-unavailable"
            }
            ProcMapsAttribution::Ambiguous => {
                "linux:SYS_gettid+pthread_getattr_np/pthread_attr_getstack+/proc/self/maps;proc-maps=ambiguous;reserved=unresolved;guard=pthread_attr_getguardsize-reported;mincore-resident-only;commit-unavailable"
            }
        },
    })
}

#[cfg(target_os = "linux")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ProcMapsAttribution {
    Unavailable,
    Ambiguous,
    Unique,
}

#[cfg(target_os = "linux")]
fn inspect_linux_proc_maps(stack_low: u64, stack_high: u64) -> ProcMapsAttribution {
    use std::fs::File;
    use std::io::{BufRead, BufReader};

    let file = match File::open("/proc/self/maps") {
        Ok(file) => file,
        Err(_) => return ProcMapsAttribution::Unavailable,
    };
    let mut reader = BufReader::new(file);
    let mut line = String::new();
    let mut matching_regions = 0_u32;
    let mut covering_region = false;

    loop {
        line.clear();
        let bytes_read = match reader.read_line(&mut line) {
            Ok(bytes_read) => bytes_read,
            Err(_) => return ProcMapsAttribution::Unavailable,
        };
        if bytes_read == 0 {
            break;
        }

        let mut fields = line.split_whitespace();
        let Some(range) = fields.next() else {
            continue;
        };
        let Some((low, high)) = range.split_once('-') else {
            return ProcMapsAttribution::Ambiguous;
        };
        let Some(low) = u64::from_str_radix(low, 16).ok() else {
            return ProcMapsAttribution::Ambiguous;
        };
        let Some(high) = u64::from_str_radix(high, 16).ok() else {
            return ProcMapsAttribution::Ambiguous;
        };
        if low >= high {
            return ProcMapsAttribution::Ambiguous;
        }
        if low < stack_high && stack_low < high {
            matching_regions += 1;
            covering_region = low <= stack_low && stack_high <= high;
        }
    }

    if matching_regions == 1 && covering_region {
        ProcMapsAttribution::Unique
    } else if matching_regions == 0 {
        ProcMapsAttribution::Unavailable
    } else {
        ProcMapsAttribution::Ambiguous
    }
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
    let vmmap = capture_macos_vmmap(stack_low_u64, stack_high_u64);

    Ok(StackSample {
        tid,
        stack_low_address: Some(stack_low_u64),
        stack_high_address_exclusive: Some(stack_high_u64),
        reserved_bytes: vmmap.reserved_bytes,
        committed_bytes: vmmap.committed_bytes,
        resident_bytes: count_resident_bytes(stack_low_u64, stack_high_u64),
        guard_bytes: vmmap.guard_bytes,
        measurement_source: vmmap.measurement_source(),
    })
}

#[cfg(target_os = "macos")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum VmmapAttribution {
    Unavailable,
    Ambiguous,
    Unique,
}

#[cfg(target_os = "macos")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum VmmapGuardAttribution {
    Unavailable,
    Ambiguous,
    Matched,
}

#[cfg(target_os = "macos")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct VmmapStackEvidence {
    reserved_bytes: Option<u64>,
    committed_bytes: Option<u64>,
    guard_bytes: Option<u64>,
    attribution: VmmapAttribution,
    guard_attribution: VmmapGuardAttribution,
}

#[cfg(target_os = "macos")]
impl VmmapStackEvidence {
    fn unresolved(attribution: VmmapAttribution) -> Self {
        Self {
            reserved_bytes: None,
            committed_bytes: None,
            guard_bytes: None,
            attribution,
            guard_attribution: VmmapGuardAttribution::Unavailable,
        }
    }

    fn measurement_source(self) -> &'static str {
        match (self.attribution, self.guard_attribution) {
            (VmmapAttribution::Unavailable, _) => {
                "macos:pthread_threadid_np+pthread_get_stackaddr_np/pthread_get_stacksize_np;vmmap(-w,-pages,-resident,-dirty,-swapped)=unavailable;reserved=unresolved;committed=unresolved;guard=unresolved;mincore-resident-only"
            }
            (VmmapAttribution::Ambiguous, _) => {
                "macos:pthread_threadid_np+pthread_get_stackaddr_np/pthread_get_stacksize_np;vmmap(-w,-pages,-resident,-dirty,-swapped)=ambiguous;reserved=unresolved;committed=unresolved;guard=unresolved;mincore-resident-only"
            }
            (VmmapAttribution::Unique, VmmapGuardAttribution::Matched) => {
                "macos:pthread_threadid_np+pthread_get_stackaddr_np/pthread_get_stacksize_np;vmmap(-w,-pages,-resident,-dirty,-swapped)=unique-exact-pthread-interval;reserved=vmmap-stack-region-size;committed=vmmap-resident-plus-swapped-pages;guard=adjacent-Stack-Guard-same-vmmap-thread;mincore-resident-only"
            }
            (VmmapAttribution::Unique, VmmapGuardAttribution::Unavailable) => {
                "macos:pthread_threadid_np+pthread_get_stackaddr_np/pthread_get_stacksize_np;vmmap(-w,-pages,-resident,-dirty,-swapped)=unique-exact-pthread-interval;reserved=vmmap-stack-region-size;committed=vmmap-resident-plus-swapped-pages;guard=unresolved-not-reported;mincore-resident-only"
            }
            (VmmapAttribution::Unique, VmmapGuardAttribution::Ambiguous) => {
                "macos:pthread_threadid_np+pthread_get_stackaddr_np/pthread_get_stacksize_np;vmmap(-w,-pages,-resident,-dirty,-swapped)=unique-exact-pthread-interval;reserved=vmmap-stack-region-size;committed=vmmap-resident-plus-swapped-pages;guard=unresolved-ambiguous;mincore-resident-only"
            }
        }
    }
}

#[cfg(target_os = "macos")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum VmmapRegionKind {
    Stack,
    StackGuard,
}

#[cfg(target_os = "macos")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct VmmapRegion {
    kind: VmmapRegionKind,
    low: u64,
    high: u64,
    virtual_pages: u64,
    resident_pages: u64,
    swapped_pages: u64,
    thread_index: Option<u64>,
}

#[cfg(target_os = "macos")]
enum VmmapRow {
    NotStack,
    Malformed(VmmapRegionKind),
    Parsed(VmmapRegion),
}

#[cfg(target_os = "macos")]
fn capture_macos_vmmap(stack_low: u64, stack_high: u64) -> VmmapStackEvidence {
    use std::process::Command;

    let output = match Command::new("/usr/bin/vmmap")
        .args(["-w", "-pages", "-resident", "-dirty", "-swapped"])
        .arg(std::process::id().to_string())
        .output()
    {
        Ok(output) if output.status.success() => output,
        _ => return VmmapStackEvidence::unresolved(VmmapAttribution::Unavailable),
    };
    let text = match std::str::from_utf8(&output.stdout) {
        Ok(text) => text,
        Err(_) => return VmmapStackEvidence::unresolved(VmmapAttribution::Unavailable),
    };

    parse_macos_vmmap(text, stack_low, stack_high)
}

#[cfg(target_os = "macos")]
fn parse_macos_vmmap(text: &str, stack_low: u64, stack_high: u64) -> VmmapStackEvidence {
    let mut page_size = None;
    for line in text.lines() {
        if let Some(value) = line.strip_prefix("VM page size: ") {
            let Some((size, unit)) = value.split_once(' ') else {
                return VmmapStackEvidence::unresolved(VmmapAttribution::Unavailable);
            };
            if unit != "bytes" {
                return VmmapStackEvidence::unresolved(VmmapAttribution::Unavailable);
            }
            let Ok(size) = size.parse::<u64>() else {
                return VmmapStackEvidence::unresolved(VmmapAttribution::Unavailable);
            };
            if size == 0 || page_size.is_some_and(|prior| prior != size) {
                return VmmapStackEvidence::unresolved(VmmapAttribution::Ambiguous);
            }
            page_size = Some(size);
        }
    }
    let Some(page_size) = page_size else {
        return VmmapStackEvidence::unresolved(VmmapAttribution::Unavailable);
    };

    let mut overlapping_stack_count = 0_u32;
    let mut stack_region = None;
    for line in text.lines() {
        match parse_vmmap_region(line) {
            VmmapRow::Parsed(region) if region.kind == VmmapRegionKind::Stack => {
                let Some(region_size) = region.high.checked_sub(region.low) else {
                    return VmmapStackEvidence::unresolved(VmmapAttribution::Ambiguous);
                };
                let Some(reported_size) = region.virtual_pages.checked_mul(page_size) else {
                    return VmmapStackEvidence::unresolved(VmmapAttribution::Ambiguous);
                };
                if region_size != reported_size {
                    return VmmapStackEvidence::unresolved(VmmapAttribution::Ambiguous);
                }
                if region.low < stack_high && stack_low < region.high {
                    overlapping_stack_count += 1;
                    if region.low == stack_low && region.high == stack_high {
                        stack_region = Some(region);
                    }
                }
            }
            VmmapRow::Malformed(VmmapRegionKind::Stack) => {
                return VmmapStackEvidence::unresolved(VmmapAttribution::Ambiguous);
            }
            _ => {}
        }
    }

    if overlapping_stack_count == 0 {
        return VmmapStackEvidence::unresolved(VmmapAttribution::Unavailable);
    }
    let Some(stack_region) = stack_region.filter(|_| overlapping_stack_count == 1) else {
        return VmmapStackEvidence::unresolved(VmmapAttribution::Ambiguous);
    };
    let reserved_bytes = stack_high - stack_low;
    let Some(committed_pages) = stack_region
        .resident_pages
        .checked_add(stack_region.swapped_pages)
        .filter(|pages| *pages <= stack_region.virtual_pages)
    else {
        return VmmapStackEvidence::unresolved(VmmapAttribution::Ambiguous);
    };
    let Some(committed_bytes) = committed_pages.checked_mul(page_size) else {
        return VmmapStackEvidence::unresolved(VmmapAttribution::Ambiguous);
    };

    let mut adjacent_guard_count = 0_u32;
    let mut matching_guard = None;
    let mut malformed_guard = false;
    for line in text.lines() {
        match parse_vmmap_region(line) {
            VmmapRow::Parsed(region) if region.kind == VmmapRegionKind::StackGuard => {
                let Some(region_size) = region.high.checked_sub(region.low) else {
                    malformed_guard = true;
                    continue;
                };
                let Some(reported_size) = region.virtual_pages.checked_mul(page_size) else {
                    malformed_guard = true;
                    continue;
                };
                if region_size != reported_size {
                    malformed_guard = true;
                    continue;
                }
                if region.high == stack_low {
                    adjacent_guard_count += 1;
                    if stack_region.thread_index.is_some()
                        && region.thread_index == stack_region.thread_index
                    {
                        matching_guard = Some(region);
                    }
                }
            }
            VmmapRow::Malformed(VmmapRegionKind::StackGuard) => malformed_guard = true,
            _ => {}
        }
    }

    let (guard_bytes, guard_attribution) = if malformed_guard || adjacent_guard_count > 1 {
        (None, VmmapGuardAttribution::Ambiguous)
    } else if let Some(guard) = matching_guard.filter(|_| adjacent_guard_count == 1) {
        (Some(guard.high - guard.low), VmmapGuardAttribution::Matched)
    } else if adjacent_guard_count == 1 {
        (None, VmmapGuardAttribution::Ambiguous)
    } else {
        (None, VmmapGuardAttribution::Unavailable)
    };

    VmmapStackEvidence {
        reserved_bytes: Some(reserved_bytes),
        committed_bytes: Some(committed_bytes),
        guard_bytes,
        attribution: VmmapAttribution::Unique,
        guard_attribution,
    }
}

#[cfg(target_os = "macos")]
fn parse_vmmap_region(line: &str) -> VmmapRow {
    let trimmed = line.trim_start();
    let Some((kind, row)) = trimmed.split_once("  ") else {
        return VmmapRow::NotStack;
    };
    let kind = match kind {
        "Stack" => VmmapRegionKind::Stack,
        "Stack Guard" => VmmapRegionKind::StackGuard,
        _ => return VmmapRow::NotStack,
    };
    let Some(range) = row.split_whitespace().next() else {
        return VmmapRow::NotStack;
    };
    if !range.contains('-') {
        return VmmapRow::NotStack;
    }
    let Some((low, high)) = range.split_once('-') else {
        return VmmapRow::Malformed(kind);
    };
    let Some(low) = parse_vmmap_hex_address(low) else {
        return VmmapRow::Malformed(kind);
    };
    let Some(high) = parse_vmmap_hex_address(high) else {
        return VmmapRow::Malformed(kind);
    };
    if low >= high {
        return VmmapRow::Malformed(kind);
    }

    let Some(open_bracket) = row.find('[') else {
        return VmmapRow::Malformed(kind);
    };
    let Some(close_bracket) = row[open_bracket + 1..]
        .find(']')
        .map(|offset| open_bracket + 1 + offset)
    else {
        return VmmapRow::Malformed(kind);
    };
    let mut sizes = row[open_bracket + 1..close_bracket].split_whitespace();
    let Some(virtual_pages) = sizes.next().and_then(|size| size.parse::<u64>().ok()) else {
        return VmmapRow::Malformed(kind);
    };
    let Some(resident_pages) = sizes.next().and_then(|size| size.parse::<u64>().ok()) else {
        return VmmapRow::Malformed(kind);
    };
    let Some(_dirty_pages) = sizes.next().and_then(|size| size.parse::<u64>().ok()) else {
        return VmmapRow::Malformed(kind);
    };
    let Some(swapped_pages) = sizes.next().and_then(|size| size.parse::<u64>().ok()) else {
        return VmmapRow::Malformed(kind);
    };
    if sizes.next().is_some() {
        return VmmapRow::Malformed(kind);
    }

    let mut suffix_words = row[close_bracket + 1..].split_whitespace();
    let mut thread_index = None;
    while let Some(word) = suffix_words.next() {
        if word == "thread" {
            thread_index = suffix_words
                .next()
                .and_then(|index| index.parse::<u64>().ok());
            break;
        }
    }

    VmmapRow::Parsed(VmmapRegion {
        kind,
        low,
        high,
        virtual_pages,
        resident_pages,
        swapped_pages,
        thread_index,
    })
}

#[cfg(target_os = "macos")]
fn parse_vmmap_hex_address(address: &str) -> Option<u64> {
    u64::from_str_radix(address.strip_prefix("0x").unwrap_or(address), 16).ok()
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
        let reserved = sample.reserved_bytes;
        if let Some(reserved) = reserved {
            assert_eq!(reserved, stack_high - stack_low);
            assert!(
                reserved >= REQUESTED_STACK_BYTES as u64,
                "native reservation must cover the explicit Builder stack-size request"
            );
        }

        #[cfg(any(target_os = "linux", target_os = "macos"))]
        {
            let resident = sample
                .resident_bytes
                .expect("mincore residency query for native stack interval");
            assert!(resident > 0, "the touched worker stack has resident pages");
            if let Some(reserved) = reserved {
                assert!(resident <= reserved);
            }
            assert!(sample.measurement_source.contains("mincore-resident-only"));
        }

        #[cfg(target_os = "linux")]
        {
            assert_eq!(
                sample.committed_bytes, None,
                "commit is not exposed per stack"
            );
            let guard = sample.guard_bytes.expect("pthread guard-size attribute");
            if let Some(reserved) = reserved {
                assert!(guard <= reserved);
                assert!(sample
                    .measurement_source
                    .contains("proc-maps=unique-covering-pthread-interval"));
            } else {
                assert!(
                    sample.measurement_source.contains("proc-maps=unavailable")
                        || sample.measurement_source.contains("proc-maps=ambiguous")
                );
            }
            assert!(sample
                .measurement_source
                .contains("pthread_attr_getguardsize"));
            assert!(sample.measurement_source.contains("SYS_gettid"));
            assert!(sample.measurement_source.contains("pthread_attr_getstack"));
        }

        #[cfg(target_os = "macos")]
        {
            let source = sample.measurement_source;
            assert!(source.contains("pthread_threadid_np"));
            assert!(source.contains("pthread_get_stackaddr_np"));
            assert!(source.contains("pthread_get_stacksize_np"));
            assert!(source.contains("vmmap("));
            assert!(source.contains("mincore-resident-only"));
            if source.contains("vmmap(-w,-pages,-resident,-dirty,-swapped)=unique-") {
                let reserved = reserved.expect("uniquely matched vmmap stack reservation");
                let committed = sample.committed_bytes.expect("vmmap-backed stack bytes");
                assert!(committed <= reserved);
                if let Some(guard) = sample.guard_bytes {
                    assert!(guard > 0);
                    assert!(guard <= reserved);
                    assert!(source.contains("guard=adjacent-Stack-Guard-same-vmmap-thread"));
                } else {
                    assert!(source.contains("guard=unresolved"));
                }
            } else {
                assert!(
                    source.contains("vmmap(-w,-pages,-resident,-dirty,-swapped)=unavailable")
                        || source.contains("vmmap(-w,-pages,-resident,-dirty,-swapped)=ambiguous")
                );
                assert_eq!(reserved, None);
                assert_eq!(sample.committed_bytes, None);
                assert_eq!(sample.guard_bytes, None);
            }
        }

        #[cfg(target_os = "windows")]
        {
            let reserved = reserved.expect("VirtualQuery reserve total");
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
    #[cfg(target_os = "macos")]
    #[test]
    fn vmmap_stack_evidence_requires_a_unique_exact_pthread_interval() {
        const VM_MAP: &str = "\
VM page size: 4096 bytes
REGION TYPE                    START - END         [ VSIZE RSDNT DIRTY SWAP]
Stack Guard                    f000-10000          [ 1 0 0 0] ---/rwx SM=NUL stack guard for thread 7
Stack                          10000-90000         [ 128 3 3 1] rw-/rwx SM=COW thread 7
Stack                          a0000-b0000         [ 16 1 1 0] rw-/rwx SM=COW thread 8
";

        let evidence = super::parse_macos_vmmap(VM_MAP, 0x10000, 0x90000);
        assert_eq!(evidence.attribution, super::VmmapAttribution::Unique);
        assert_eq!(evidence.reserved_bytes, Some(0x80000));
        assert_eq!(evidence.committed_bytes, Some(4 * 4096));
        assert_eq!(evidence.guard_bytes, Some(4096));
        assert_eq!(
            evidence.guard_attribution,
            super::VmmapGuardAttribution::Matched
        );

        let duplicate = format!(
            "{VM_MAP}Stack                          10000-90000         [ 128 3 3 1] rw-/rwx SM=COW thread 9\n"
        );
        let evidence = super::parse_macos_vmmap(&duplicate, 0x10000, 0x90000);
        assert_eq!(evidence.attribution, super::VmmapAttribution::Ambiguous);
        assert_eq!(evidence.reserved_bytes, None);
        assert_eq!(evidence.committed_bytes, None);
        assert_eq!(evidence.guard_bytes, None);

        let evidence = super::parse_macos_vmmap(VM_MAP, 0x11000, 0x90000);
        assert_eq!(evidence.attribution, super::VmmapAttribution::Ambiguous);
        assert_eq!(evidence.reserved_bytes, None);
        assert_eq!(evidence.committed_bytes, None);
        assert_eq!(evidence.guard_bytes, None);

        let evidence = super::parse_macos_vmmap("VM page size: 4096 bytes\n", 0x10000, 0x90000);
        assert_eq!(evidence.attribution, super::VmmapAttribution::Unavailable);
        assert_eq!(evidence.reserved_bytes, None);
        assert_eq!(evidence.committed_bytes, None);
        assert_eq!(evidence.guard_bytes, None);
    }
}
