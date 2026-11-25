//! Process RSS / CPU sample for live harnesses and the panel resource card.
//!
//! [`sample_process`] first field is lifetime peak resident:
//! - macos/linux: `ru_maxrss` (bytes / KiB→bytes)
//! - windows: `PeakWorkingSetSize` from `GetProcessMemoryInfo`
//!
//! CPU second field is cumulative process user+kernel seconds (panel delta).
//! Failure sentinel remains `(0, 0.0)` for public tuple compatibility.

#[cfg(any(target_os = "macos", target_os = "linux"))]
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
    process_cpu_user_kernel().map(|(user, kernel)| user + kernel)
}

/// Process CPU time in seconds: `(user, kernel)`.
///
/// `None` when this OS has no sampler or the sampler fails. Callers that need
/// one total use `user + kernel` only where that sum is already the published
/// value. [`sample_process`] on macOS and Linux keeps its previous four-term
/// sum, because adding this pair is not bit-identical.
///
/// The memory-profile caller is feature-gated. On Windows, [`sample_process`]
/// uses this function too, so it stays available there without that feature.
#[cfg(any(feature = "memory-profile", windows))]
pub(crate) fn process_cpu_user_kernel() -> Option<(f64, f64)> {
    #[cfg(unix)]
    {
        self_rusage().map(|usage| unix_cpu_user_kernel(&usage))
    }
    #[cfg(windows)]
    {
        windows_cpu_user_kernel()
    }
    #[cfg(not(any(unix, windows)))]
    {
        None
    }
}

#[cfg(unix)]
fn self_rusage() -> Option<libc::rusage> {
    // SAFETY: `rusage` is a plain C struct. Zero is a valid initial value.
    let mut usage: libc::rusage = unsafe { std::mem::zeroed() };
    // SAFETY: `usage` is a writable `rusage` for this call. `RUSAGE_SELF` names this process.
    let rc = unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut usage) };
    if rc != 0 {
        None
    } else {
        Some(usage)
    }
}

#[cfg(all(unix, feature = "memory-profile"))]
fn timeval_seconds(value: libc::timeval) -> f64 {
    value.tv_sec as f64 + value.tv_usec as f64 / 1_000_000.0
}

#[cfg(all(unix, feature = "memory-profile"))]
fn unix_cpu_user_kernel(usage: &libc::rusage) -> (f64, f64) {
    (
        timeval_seconds(usage.ru_utime),
        timeval_seconds(usage.ru_stime),
    )
}

/// The previous [`sample_process`] CPU sum. Do not replace it with `user + kernel`.
#[cfg(any(target_os = "macos", target_os = "linux"))]
fn unix_sample_cpu_seconds(usage: &libc::rusage) -> f64 {
    usage.ru_utime.tv_sec as f64
        + usage.ru_utime.tv_usec as f64 / 1e6
        + usage.ru_stime.tv_sec as f64
        + usage.ru_stime.tv_usec as f64 / 1e6
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
        let Some(usage) = self_rusage() else {
            return (0, 0.0);
        };
        let rss = rss_bytes_from_ru_maxrss(usage.ru_maxrss);
        (rss, unix_sample_cpu_seconds(&usage))
    }
    #[cfg(windows)]
    {
        // First field = PeakWorkingSetSize (lifetime peak WS).
        // Either sampler fail → full (0, 0.0) sentinel (panel joint check).
        let peak = windows_memory_counters().and_then(|(_, peak)| nonzero_resident(peak));
        let cpu = windows_cpu_seconds_total();
        combine_windows_sample(peak, cpu)
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
    {
        (0, 0.0)
    }
}

#[cfg(target_os = "macos")]
const TASK_VM_INFO_REV1_COUNT: libc::mach_msg_type_number_t = 38;

#[cfg(target_os = "macos")]
const TASK_VM_INFO_REV3_COUNT: libc::mach_msg_type_number_t = 84;

#[cfg(target_os = "macos")]
fn macos_task_vm_footprint(
    status: libc::kern_return_t,
    count: libc::mach_msg_type_number_t,
    info: &[u64; 42],
) -> (Option<u64>, Option<u64>) {
    if status != libc::KERN_SUCCESS || count < TASK_VM_INFO_REV1_COUNT {
        return (None, None);
    }
    let current = (info[18] > 0).then_some(info[18]);
    let peak = if count >= 44 {
        let peak = info[21] as i64;
        (peak > 0).then_some(peak as u64)
    } else {
        None
    };
    (current, peak)
}

/// Current footprint and lifetime footprint peak from one macOS task-info read.
/// A kernel that returns only REV1 or REV2 supplies no footprint peak.
#[cfg(target_os = "macos")]
pub fn current_footprint_and_peak_bytes() -> (Option<u64>, Option<u64>) {
    // mach/task_info.h: REV3 has 84 integer_t cells. phys_footprint starts
    // at cell 36 (u64 index 18), ledger_phys_footprint_peak at cell 42 (index 21).
    let mut info = [0_u64; 42];
    let mut count = TASK_VM_INFO_REV3_COUNT;
    #[expect(
        deprecated,
        reason = "libc's Mach task accessor avoids an extra FFI dependency"
    )]
    // SAFETY: mach_task_self() names this process. task_info writes at most
    // count integer_t cells into the aligned 42-u64 buffer. The parser checks
    // the returned count before reading either complete footprint field.
    let kr = unsafe {
        libc::task_info(
            libc::mach_task_self(),
            22, // TASK_VM_INFO
            info.as_mut_ptr().cast::<libc::integer_t>(),
            &mut count,
        )
    };
    macos_task_vm_footprint(kr, count, &info)
}

#[cfg(any(target_os = "linux", test))]
fn read_linux_rss(
    read_status: impl FnOnce() -> std::io::Result<String>,
) -> (Option<u64>, Option<u64>) {
    fn parse_kib(value: &str) -> Option<u64> {
        let mut parts = value.split_ascii_whitespace();
        let kib: u64 = parts.next()?.parse().ok()?;
        if parts.next()? != "kB" {
            return None;
        }
        kib.checked_mul(1024).filter(|bytes| *bytes > 0)
    }

    let Ok(status) = read_status() else {
        return (None, None);
    };
    let mut current = None;
    let mut peak = None;
    for line in status.lines() {
        if let Some(value) = line.strip_prefix("VmRSS:") {
            current = parse_kib(value);
        } else if let Some(value) = line.strip_prefix("VmHWM:") {
            peak = parse_kib(value);
        }
        if current.is_some() && peak.is_some() {
            break;
        }
    }
    (current, peak)
}

/// Current RSS and lifetime RSS high-water mark from one Linux status read.
#[cfg(target_os = "linux")]
pub fn current_rss_and_peak_bytes() -> (Option<u64>, Option<u64>) {
    // Linux fs/proc/task_mmu.c task_mem derives VmRSS and VmHWM from the
    // same exact counter sum, with VmHWM at least VmRSS.
    read_linux_rss(|| std::fs::read_to_string("/proc/self/status"))
}

/// Current process memory in bytes: physical footprint on macOS, resident
/// pages on Linux, and working set on Windows.
///
/// Distinct from [`sample_process`]'s first field, which is lifetime peak
/// (`ru_maxrss` / Windows `PeakWorkingSetSize`). Returns `None` on unsupported
/// OS or sample failure — never `Some(0)`.
pub fn current_resident_bytes() -> Option<u64> {
    #[cfg(target_os = "macos")]
    {
        current_footprint_and_peak_bytes().0
    }
    #[cfg(target_os = "linux")]
    {
        current_rss_and_peak_bytes().0
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

    const LINUX_STATUS_FIXTURE: &str = "\
Name:\talice
State:\tR (running)
VmPeak:\t   65536 kB
VmSize:\t   32768 kB
VmHWM:\t    8192 kB
VmRSS:\t    6144 kB
RssAnon:\t   4096 kB
RssFile:\t   2048 kB
Threads:\t4
";

    #[test]
    fn linux_rss_pair_uses_one_status_read() {
        let mut reads = 0;
        let pair = read_linux_rss(|| {
            reads += 1;
            Ok(LINUX_STATUS_FIXTURE.to_owned())
        });
        assert_eq!(reads, 1);
        assert_eq!(pair, (Some(6144 * 1024), Some(8192 * 1024)));
    }

    #[test]
    fn linux_rss_status_preserves_missing_and_failed_values() {
        for (text, expected) in [
            ("VmRSS:\t128 kB\n", (Some(128 * 1024), None)),
            ("VmHWM:\t256 kB\n", (None, Some(256 * 1024))),
            ("VmRSS:\t0 kB\nVmHWM:\t256 kB\n", (None, Some(256 * 1024))),
            ("VmRSS:\t128 B\nVmHWM:\tinvalid kB\n", (None, None)),
        ] {
            assert_eq!(read_linux_rss(|| Ok(text.to_owned())), expected, "{text}");
        }
        assert_eq!(
            read_linux_rss(|| Err(std::io::Error::other("status read failed"))),
            (None, None)
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_status_rss_peak_covers_the_same_samples_current_value() {
        let (current, peak) = current_rss_and_peak_bytes();
        let current = current.expect("VmRSS");
        let peak = peak.expect("VmHWM");
        assert!(
            current > 0 && peak >= current,
            "current={current} peak={peak}"
        );
    }

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

    #[cfg(target_os = "macos")]
    #[test]
    fn task_vm_phys_footprint_rejects_failed_or_short_reads() {
        let mut info = [0_u64; 42];
        info[18] = 4096;
        info[21] = 8192;
        assert_eq!(
            macos_task_vm_footprint(libc::KERN_SUCCESS, TASK_VM_INFO_REV3_COUNT, &info),
            (Some(4096), Some(8192))
        );
        assert_eq!(
            macos_task_vm_footprint(libc::KERN_FAILURE, TASK_VM_INFO_REV3_COUNT, &info),
            (None, None)
        );
        for count in [TASK_VM_INFO_REV1_COUNT, 42, 43] {
            assert_eq!(
                macos_task_vm_footprint(libc::KERN_SUCCESS, count, &info),
                (Some(4096), None),
                "the peak needs both integer_t cells: count={count}"
            );
        }
        assert_eq!(
            macos_task_vm_footprint(libc::KERN_SUCCESS, 44, &info),
            (Some(4096), Some(8192))
        );
        assert_eq!(
            macos_task_vm_footprint(libc::KERN_SUCCESS, TASK_VM_INFO_REV1_COUNT - 1, &info),
            (None, None)
        );
        info[18] = 0;
        info[21] = u64::MAX;
        assert_eq!(
            macos_task_vm_footprint(libc::KERN_SUCCESS, TASK_VM_INFO_REV3_COUNT, &info),
            (None, None)
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_current_memory_matches_map_footprint() {
        // The map_footprint example reads ri_phys_footprint from RUSAGE_INFO_V4.
        // SAFETY: rusage_info_v4 is a plain C output struct; zero is a valid
        // initial value for its fields.
        let mut info: libc::rusage_info_v4 = unsafe { std::mem::zeroed() };
        let readout_before = current_resident_bytes().expect("task_info before rusage");
        // SAFETY: The current PID identifies the same process as the readout.
        // proc_pid_rusage writes RUSAGE_INFO_V4 into the valid output buffer.
        let rc = unsafe {
            libc::proc_pid_rusage(
                libc::getpid(),
                libc::RUSAGE_INFO_V4,
                (&mut info as *mut libc::rusage_info_v4).cast::<libc::rusage_info_t>(),
            )
        };
        assert_eq!(rc, 0, "proc_pid_rusage(RUSAGE_INFO_V4) failed");
        let example = info.ri_phys_footprint;
        let readout_after = current_resident_bytes().expect("task_info after rusage");
        println!(
            "readout_before_bytes={readout_before} example_ri_phys_footprint_bytes={example} readout_after_bytes={readout_after}"
        );
        assert!(example > 0, "example footprint was zero");
        const SLACK: u64 = 256 * 1024;
        let lower = readout_before.min(readout_after).saturating_sub(SLACK);
        let upper = readout_before.max(readout_after).saturating_add(SLACK);
        assert!(
            (lower..=upper).contains(&example),
            "readout_before={readout_before} example={example} readout_after={readout_after}: different memory metrics"
        );
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

    #[cfg(all(
        feature = "memory-profile",
        any(target_os = "macos", target_os = "linux")
    ))]
    #[test]
    #[ignore = "timing self-test of the process CPU counter: a loaded machine (CI, parallel suites) can starve the busy child; run it explicitly before a CPU-share measurement"]
    fn process_cpu_counter_busy_loop_child() {
        const CHILD: &str = "274BOT_PROCESS_CPU_COUNTER_SELFTEST_CHILD";
        if std::env::var_os(CHILD).is_some() {
            let before = process_cpu_user_kernel().expect("getrusage before busy loop");
            let started = std::time::Instant::now();
            let mut value = 0u64;
            while started.elapsed() < std::time::Duration::from_secs(2) {
                value = value.wrapping_add(1);
                std::hint::black_box(value);
            }
            let wall_seconds = started.elapsed().as_secs_f64();
            let after = process_cpu_user_kernel().expect("getrusage after busy loop");
            let cpu_seconds = (after.0 + after.1) - (before.0 + before.1);
            let cpu_cores = cpu_seconds / wall_seconds;
            eprintln!(
                "PROCESS_CPU_BUSY_LOOP cores={cpu_cores:.3}, cpu={cpu_seconds:.3}s, wall={wall_seconds:.3}s"
            );
            assert!(
                (cpu_cores - 1.0).abs() <= 0.5,
                "busy child used {cpu_cores:.3} CPU cores; expected 1.0 ±0.5"
            );
            return;
        }

        let output = crate::test_process::output(
            std::process::Command::new(crate::test_process::executable())
                .arg("process_cpu_counter_busy_loop_child")
                .arg("--nocapture")
                .arg("--ignored")
                .arg("--test-threads=1")
                .env(CHILD, "1"),
        )
        .expect("spawn isolated busy-loop child");
        assert!(
            output.status.success(),
            "busy-loop child failed: stdout={} stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        let stderr = String::from_utf8_lossy(&output.stderr);
        let report = stderr
            .lines()
            .find(|line| line.contains("PROCESS_CPU_BUSY_LOOP"))
            .unwrap_or("PROCESS_CPU_BUSY_LOOP result not printed");
        println!("{report}");
    }
}
