//! Process RSS / CPU sample for live harnesses and the panel resource card.

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
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        (0, 0.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sample_process_rss_is_nonzero_on_this_host() {
        let (rss, cpu) = sample_process();
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        {
            assert!(rss > 0, "rss={rss} cpu={cpu}");
            assert!(cpu >= 0.0);
        }
        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
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
}
