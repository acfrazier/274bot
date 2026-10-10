//! Raise the process's open-file limit at startup.
//!
//! Every bot holds a game socket plus a few local descriptors (wake pipes,
//! the script isolate, cache files). macOS gives an app launched from Finder
//! a soft limit of 256, so a fleet of ~70 bots ran out ("Too many open
//! files") while the hard limit allowed far more. The binaries raise the
//! soft limit once, before any slot starts.

/// Soft limit the binaries ask for: far above any fleet the host runs, and
/// still bounded so a huge hard limit doesn't become the soft one.
pub const TARGET: u64 = 65_536;

/// The limit before and after [`raise_open_file_limit`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OpenFileLimit {
    pub before: u64,
    pub after: u64,
}

/// The soft limit to set, or `None` when `soft` is already high enough.
/// `per_process` is the kernel's per-process ceiling where it's separate
/// from the hard limit (macOS `kern.maxfilesperproc`).
pub fn soft_target(soft: u64, hard: u64, per_process: Option<u64>) -> Option<u64> {
    let mut want = TARGET.min(hard);
    if let Some(cap) = per_process {
        want = want.min(cap);
    }
    (want > soft).then_some(want)
}

/// Raise the soft `RLIMIT_NOFILE` toward [`TARGET`]. `Ok(None)` when it's
/// already at least that high (or on platforms without the limit).
#[cfg(unix)]
pub fn raise_open_file_limit() -> std::io::Result<Option<OpenFileLimit>> {
    let mut lim = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    // SAFETY: `lim` is a valid, writable rlimit for getrlimit to fill.
    if unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut lim) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    // rlim_t is u64 on every Unix target the host ships (macOS, Linux x64).
    let before: u64 = lim.rlim_cur;
    let Some(want) = soft_target(before, lim.rlim_max, per_process_cap()) else {
        return Ok(None);
    };
    let set = |soft: u64| {
        let next = libc::rlimit {
            rlim_cur: soft,
            rlim_max: lim.rlim_max,
        };
        // SAFETY: `next` is a valid rlimit; setrlimit only reads it.
        (unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &next) } == 0)
            .then_some(soft)
            .ok_or_else(std::io::Error::last_os_error)
    };
    let after = match set(want) {
        Ok(after) => after,
        // Older macOS refuses a soft limit above OPEN_MAX (10240).
        #[cfg(target_os = "macos")]
        Err(e) if want > 10_240 && before < 10_240 => set(10_240).map_err(|_| e)?,
        Err(e) => return Err(e),
    };
    Ok(Some(OpenFileLimit { before, after }))
}

#[cfg(not(unix))]
pub fn raise_open_file_limit() -> std::io::Result<Option<OpenFileLimit>> {
    Ok(None)
}

/// One line for the log describing what [`raise_open_file_limit`] did.
pub fn describe(result: &std::io::Result<Option<OpenFileLimit>>) -> Option<String> {
    match result {
        Ok(Some(l)) => Some(format!(
            "open-file limit raised from {} to {}",
            l.before, l.after
        )),
        Ok(None) => None,
        Err(e) => Some(format!("open-file limit could not be raised: {e}")),
    }
}

#[cfg(target_os = "macos")]
fn per_process_cap() -> Option<u64> {
    let mut value: libc::c_int = 0;
    let mut len = std::mem::size_of::<libc::c_int>();
    // SAFETY: the name is NUL-terminated; `value`/`len` describe a writable
    // c_int, which is the type of kern.maxfilesperproc.
    let rc = unsafe {
        libc::sysctlbyname(
            c"kern.maxfilesperproc".as_ptr(),
            (&mut value as *mut libc::c_int).cast(),
            &mut len,
            std::ptr::null_mut(),
            0,
        )
    };
    (rc == 0 && value > 0).then_some(value as u64)
}

#[cfg(all(unix, not(target_os = "macos")))]
fn per_process_cap() -> Option<u64> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raises_a_low_soft_limit_to_the_target_within_the_hard_limit() {
        // The Finder default on macOS: 256 soft, unlimited hard.
        assert_eq!(soft_target(256, u64::MAX, Some(245_760)), Some(TARGET));
        // A hard limit below the target caps it.
        assert_eq!(soft_target(1024, 4096, None), Some(4096));
        // So does the kernel's per-process ceiling.
        assert_eq!(soft_target(256, u64::MAX, Some(10_240)), Some(10_240));
    }

    #[test]
    fn leaves_an_already_high_soft_limit_alone() {
        assert_eq!(soft_target(TARGET, u64::MAX, None), None);
        assert_eq!(soft_target(1_048_576, u64::MAX, None), None);
        // Never lowers: hard limit at the current soft limit.
        assert_eq!(soft_target(4096, 4096, None), None);
    }

    #[cfg(unix)]
    #[test]
    fn the_process_limit_ends_at_least_as_high_as_it_started() {
        let mut lim = libc::rlimit {
            rlim_cur: 0,
            rlim_max: 0,
        };
        // SAFETY: valid writable rlimit.
        assert_eq!(unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut lim) }, 0);
        let start: u64 = lim.rlim_cur;
        let result = raise_open_file_limit().expect("raise");
        // SAFETY: as above.
        assert_eq!(unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut lim) }, 0);
        let now: u64 = lim.rlim_cur;
        assert!(now >= start);
        if let Some(l) = result {
            assert_eq!((l.before, l.after), (start, now));
        }
    }
}
