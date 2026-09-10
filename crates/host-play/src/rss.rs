//! Process RSS / CPU sample for live harnesses and the panel resource card.
//!
//! [`sample_process`] first field is lifetime peak resident:
//! - macos/linux: `ru_maxrss` (bytes / KiB→bytes)
//! - windows: `PeakWorkingSetSize` from `GetProcessMemoryInfo`
//!
//! CPU second field is cumulative process user+kernel seconds (panel delta).
//! Failure sentinel remains `(0, 0.0)` for public tuple compatibility.

pub fn rss_bytes_from_ru_maxrss(raw: i64) -> u64 {
    if raw < 0 {
        return 0;
    }
    let n = raw as u64;
    #[cfg(target_os = "linux")]
    {
        n.saturating_mul(1024)
    }
    #[cfg(target_os = "macos")]
    {
        n
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        let _ = n;
        0
    }
}

/// Convert GetProcessTimes user/kernel `FILETIME` parts to seconds.
/// Those values are **CPU durations** in 100 ns units (not wall time since 1601).
/// Pure; factored for unit tests without calling the OS.
#[cfg(windows)]
pub(crate) fn filetime_parts_to_seconds(high: u32, low: u32) -> f64 {
    let ticks = ((high as u64) << 32) | (low as u64);
    ticks as f64 / 10_000_000.0
}

/// Combine Windows peak-resident and CPU samples into the public tuple.
/// If **either** sampler fails (`None`), return the joint failure sentinel
/// `(0, 0.0)` so panel treats the whole sample as Error (not mixed zeros).
/// Pure and OS-agnostic so Mac/Linux can unit-test mixed failure without
/// synthetic Win32 API failures. Wired only into the Windows `sample_process`.
#[cfg(any(windows, test))]
pub(crate) fn combine_windows_sample(peak: Option<u64>, cpu: Option<f64>) -> (u64, f64) {
    match (peak, cpu) {
        (Some(p), Some(c)) => (p, c),
        _ => (0, 0.0),
    }
}

/// Map a resident size to `Option`, never `Some(0)`.
#[cfg(windows)]
fn nonzero_resident(n: usize) -> Option<u64> {
    let n = n as u64;
    if n == 0 {
        None
    } else {
        Some(n)
    }
}

/// Raw `(WorkingSetSize, PeakWorkingSetSize)` when `GetProcessMemoryInfo` succeeds.
/// Callers map zeros to `None` / sentinel — do not treat a successful zero as real.
#[cfg(windows)]
fn windows_memory_counters() -> Option<(usize, usize)> {
    use windows_sys::Win32::System::ProcessStatus::{
        GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS,
    };
    use windows_sys::Win32::System::Threading::GetCurrentProcess;

    // GetCurrentProcess returns a pseudo-handle; do not CloseHandle it.
    unsafe {
        let mut pmc: PROCESS_MEMORY_COUNTERS = std::mem::zeroed();
        pmc.cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
        let ok = GetProcessMemoryInfo(GetCurrentProcess(), &mut pmc, pmc.cb);
        if ok == 0 {
            return None;
        }
        Some((pmc.WorkingSetSize, pmc.PeakWorkingSetSize))
    }
}

#[cfg(windows)]
fn windows_cpu_seconds_total() -> Option<f64> {
    windows_cpu_user_kernel().map(|(u, k)| u + k)
}

#[cfg(windows)]
pub(crate) fn windows_cpu_user_kernel() -> Option<(f64, f64)> {
    use windows_sys::Win32::Foundation::FILETIME;
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes};

    // GetCurrentProcess pseudo-handle — do not close.
    unsafe {
        let mut creation: FILETIME = std::mem::zeroed();
        let mut exit: FILETIME = std::mem::zeroed();
        let mut kernel: FILETIME = std::mem::zeroed();
        let mut user: FILETIME = std::mem::zeroed();
        let ok = GetProcessTimes(
            GetCurrentProcess(),
            &mut creation,
            &mut exit,
            &mut kernel,
            &mut user,
        );
        if ok == 0 {
            return None;
        }
        let user_s = filetime_parts_to_seconds(user.dwHighDateTime, user.dwLowDateTime);
        let kernel_s = filetime_parts_to_seconds(kernel.dwHighDateTime, kernel.dwLowDateTime);
        Some((user_s, kernel_s))
    }
}

pub fn sample_process() -> (u64, f64) {
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    {
        let mut usage: libc::rusage = unsafe { std::mem::zeroed() };
        let rc = unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut usage) };
        if rc != 0 {
            return (0, 0.0);
        }
        let rss = rss_bytes_from_ru_maxrss(usage.ru_maxrss);
        let cpu = usage.ru_utime.tv_sec as f64
            + usage.ru_utime.tv_usec as f64 / 1e6
            + usage.ru_stime.tv_sec as f64
            + usage.ru_stime.tv_usec as f64 / 1e6;
        (rss, cpu)
    }
    #[cfg(windows)]
    {
        // First field = PeakWorkingSetSize (lifetime peak WS).
        // Either sampler fail → full (0, 0.0) sentinel (panel joint check).
        let peak = windows_memory_counters()
            .and_then(|(_, peak)| nonzero_resident(peak));
        let cpu = windows_cpu_seconds_total();
        combine_windows_sample(peak, cpu)
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
    {
        (0, 0.0)
    }
}

/// Current resident set size in bytes.
///
/// Distinct from [`sample_process`]'s first field, which is lifetime peak
/// (`ru_maxrss` / Windows `PeakWorkingSetSize`). Returns `None` on unsupported
/// OS or sample failure — never `Some(0)`.
pub fn current_resident_bytes() -> Option<u64> {
    #[cfg(target_os = "macos")]
    {
        // libc::mach_task_self is deprecated in favor of mach2; stay on
        // libc with the rest of host-play and suppress the warn.
        #[allow(deprecated)]
        unsafe {
            let mut info: libc::mach_task_basic_info = std::mem::zeroed();
            let mut count = libc::MACH_TASK_BASIC_INFO_COUNT;
            let kr = libc::task_info(
                libc::mach_task_self(),
                libc::MACH_TASK_BASIC_INFO,
                &mut info as *mut _ as libc::task_info_t,
                &mut count,
            );
            if kr != libc::KERN_SUCCESS {
                return None;
            }
            let n = info.resident_size as u64;
            if n == 0 {
                None
            } else {
                Some(n)
            }
        }
    }
    #[cfg(target_os = "linux")]
    {
        let text = std::fs::read_to_string("/proc/self/statm").ok()?;
        let mut parts = text.split_whitespace();
        let _total_pages = parts.next()?;
        let resident_pages: u64 = parts.next()?.parse().ok()?;
        if resident_pages == 0 {
            return None;
        }
        let page = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
        if page <= 0 {
            return None;
        }
        Some(resident_pages.saturating_mul(page as u64))
    }
    #[cfg(windows)]
    {
        windows_memory_counters().and_then(|(current, _)| nonzero_resident(current))
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
    {
        None
    }
}

/// Count unique ESTABLISHED TCP names in `lsof -nP -iTCP` stdout.
/// One socket can appear twice (reader + writer fd); the NAME column
/// (`host:port->host:port`) is the connection.
pub fn parse_lsof_established(stdout: &str) -> usize {
    use std::collections::HashSet;
    let mut names = HashSet::new();
    for line in stdout.lines() {
        if !line.contains("ESTABLISHED") {
            continue;
        }
        let Some(tcp) = line.find("TCP ") else {
            continue;
        };
        let rest = &line[tcp + 4..];
        if let Some(name) = rest.split_whitespace().next() {
            names.insert(name);
        }
    }
    names.len()
}

/// Established TCP from this pid to `host:port`, via `lsof`. `None` if
/// `lsof` is missing. Empty match is `Some(0)` (`lsof` exits 1).
///
/// **Windows:** no TCP-count backend in this hop (`lsof` is unix-only).
/// Always `None` — do not shell out or fabricate zero.
pub fn count_tcp_to(host: &str, port: u16) -> Option<usize> {
    #[cfg(windows)]
    {
        let _ = (host, port);
        None
    }
    #[cfg(not(windows))]
    {
        let pid = std::process::id().to_string();
        let spec = format!("-iTCP@{host}:{port}");
        let output = std::process::Command::new("lsof")
            .args(["-nP", "-a", "-p", &pid, &spec])
            .output()
            .ok()?;
        Some(parse_lsof_established(&String::from_utf8_lossy(
            &output.stdout,
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_memory_resident_is_some_and_independent_of_peak() {
        let current = current_resident_bytes();
        let (peak, _) = sample_process();
        #[cfg(any(target_os = "macos", target_os = "linux", windows))]
        {
            let cur = current.expect("current_resident_bytes should be Some on this host");
            assert!(cur > 0, "current={cur}");
            assert!(peak > 0, "peak={peak}");
            // Independently sampled: both are real process metrics.
            // Current can be ≤ peak; they must not be forced equal by
            // replacing peak with current in sample_process.
            let _ = (cur, peak);
        }
        #[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
        {
            assert_eq!(current, None);
            assert_eq!(peak, 0);
        }
    }

    #[test]
    fn sample_process_rss_is_nonzero_on_this_host() {
        let (rss, cpu) = sample_process();
        #[cfg(any(target_os = "macos", target_os = "linux", windows))]
        {
            assert!(rss > 0, "rss={rss} cpu={cpu}");
            assert!(cpu >= 0.0);
        }
        #[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
        {
            assert_eq!((rss, cpu), (0, 0.0));
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn darwin_ru_maxrss_is_bytes() {
        assert_eq!(rss_bytes_from_ru_maxrss(4096), 4096);
        assert_eq!(rss_bytes_from_ru_maxrss(-1), 0);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_ru_maxrss_is_kilobytes() {
        assert_eq!(rss_bytes_from_ru_maxrss(2), 2048);
        assert_eq!(rss_bytes_from_ru_maxrss(-1), 0);
    }

    #[cfg(windows)]
    #[test]
    fn filetime_parts_to_seconds_controlled() {
        assert_eq!(filetime_parts_to_seconds(0, 0), 0.0);
        let one = filetime_parts_to_seconds(0, 10_000_000);
        assert!((one - 1.0).abs() < 1e-12, "one={one}");
        let half = filetime_parts_to_seconds(0, 5_000_000);
        assert!((half - 0.5).abs() < 1e-12, "half={half}");
        // high dword alone: 2^32 ticks / 1e7 seconds
        let high = filetime_parts_to_seconds(1, 0);
        let expect = (1u64 << 32) as f64 / 10_000_000.0;
        assert!((high - expect).abs() < 1e-9, "high={high} expect={expect}");
    }

    #[cfg(windows)]
    #[test]
    fn nonzero_resident_rejects_zero() {
        assert_eq!(nonzero_resident(0), None);
        assert_eq!(nonzero_resident(1), Some(1));
        assert_eq!(nonzero_resident(4096), Some(4096));
    }

    #[test]
    fn combine_windows_sample_joint_sentinel_on_any_failure() {
        assert_eq!(combine_windows_sample(Some(4096), Some(1.5)), (4096, 1.5));
        // Peak ok, CPU fail → full sentinel (not positive peak + 0.0 CPU).
        assert_eq!(combine_windows_sample(Some(4096), None), (0, 0.0));
        // Memory fail, CPU ok → full sentinel (not 0 peak + positive CPU).
        assert_eq!(combine_windows_sample(None, Some(2.0)), (0, 0.0));
        assert_eq!(combine_windows_sample(None, None), (0, 0.0));
        // Zero peak is already mapped to None by nonzero_resident before combine.
        assert_eq!(combine_windows_sample(None, Some(0.0)), (0, 0.0));
    }

    #[cfg(windows)]
    #[test]
    fn windows_working_set_current_and_peak_nonzero() {
        let current = current_resident_bytes().expect("WorkingSetSize");
        let (peak, cpu) = sample_process();
        assert!(current > 0, "current ws={current}");
        assert!(peak > 0, "peak ws={peak}");
        assert!(
            peak >= current,
            "peak WS should be ≥ current WS: peak={peak} current={current}"
        );
        assert!(cpu >= 0.0, "cpu={cpu}");
    }

    #[cfg(windows)]
    #[test]
    fn windows_cpu_user_kernel_monotonic() {
        let before = windows_cpu_user_kernel().expect("GetProcessTimes");
        // Touch a little user time.
        let mut x = 0u64;
        for i in 0..50_000u64 {
            x = x.wrapping_add(i);
        }
        std::hint::black_box(x);
        let after = windows_cpu_user_kernel().expect("GetProcessTimes");
        assert!(before.0 >= 0.0 && before.1 >= 0.0);
        assert!(
            after.0 >= before.0 && after.1 >= before.1,
            "before={before:?} after={after:?}"
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_count_tcp_to_is_unsupported() {
        assert_eq!(count_tcp_to("127.0.0.1", 1), None);
    }

    #[test]
    fn lsof_established_counts_unique_sockets_not_fds() {
        let two_fds_one_socket = "\
COMMAND   PID USER   FD   TYPE DEVICE SIZE/OFF NODE NAME
rss_ladder 1 acf   10u  IPv4 0x1      0t0  TCP 127.0.0.1:50000->127.0.0.1:43594 (ESTABLISHED)
rss_ladder 1 acf   11u  IPv4 0x1      0t0  TCP 127.0.0.1:50000->127.0.0.1:43594 (ESTABLISHED)
rss_ladder 1 acf   12u  IPv4 0x2      0t0  TCP 127.0.0.1:50001->127.0.0.1:43594 (ESTABLISHED)
rss_ladder 1 acf   13u  IPv4 0x3      0t0  TCP 127.0.0.1:50002->127.0.0.1:43594 (CLOSE_WAIT)
";
        assert_eq!(parse_lsof_established(two_fds_one_socket), 2);
        assert_eq!(parse_lsof_established(""), 0);
        assert_eq!(parse_lsof_established("COMMAND PID\n"), 0);
    }
}
