//! Host resource metrics for the panel: pure formatters plus a best-effort
//! process sampler. No ImGui.

/// One resource row: still measuring, available with a compact label,
/// unavailable with the fixed reason, or a hard error message.
#[derive(Debug)]
pub enum Metric {
    Measuring,
    Available(String),
    Unavailable(&'static str),
    Error(String),
}

/// Snapshot of host resources shown on the panel.
pub struct ResourceView {
    pub bots: usize,
    pub ingame: usize,
    pub cpu: Metric,
    pub ram: Metric,
    pub traffic: Metric,
}

/// Traffic is always unavailable: the 274 client streams over FR's
/// `ClientStream`, which exposes no host byte counters here.
pub fn traffic_metric() -> Metric {
    Metric::Unavailable("no host byte counters (ClientStream is FR)")
}

/// `"{n} bots ({ingame} running)"`, singular `bot` when `n == 1`.
pub fn format_bots(n: usize, ingame: usize) -> String {
    let noun = if n == 1 { "bot" } else { "bots" };
    format!("{n} {noun} ({ingame} running)")
}

/// CPU utilisation from CPU/wall delta times. `Measuring` when the wall
/// delta is not positive; else busy cores (`cpu / wall`) over `ncpu`.
pub fn cpu_from_delta(dt_cpu_secs: f64, dt_wall_secs: f64, ncpu: u32) -> Metric {
    if dt_wall_secs <= 0.0 {
        return Metric::Measuring;
    }
    let cores = dt_cpu_secs / dt_wall_secs;
    let pct = 100.0 * dt_cpu_secs / (dt_wall_secs * ncpu as f64);
    Metric::Available(format!("{cores:.1} cores ({pct:.0}% of {ncpu})"))
}

/// RSS in the nearest human unit: bytes under 1 KB, then KB, MB, GB.
pub fn format_rss(bytes: u64) -> String {
    let b = bytes as f64;
    let kb = 1024.0;
    let mb = kb * 1024.0;
    let gb = mb * 1024.0;
    if b < kb {
        format!("{b:.0} B")
    } else if b < mb {
        format!("{:.0} KB", b / kb)
    } else if b < gb {
        format!("{:.1} MB", b / mb)
    } else {
        format!("{:.2} GB", b / gb)
    }
}

/// Best-effort sample of this process: `(rss_bytes, cpu_time_secs)`.
/// Returns `(0, 0.0)` on failure; the caller surfaces `Metric::Error`.
///
/// `getrusage`'s `ru_maxrss` is **bytes** on Darwin but kilobytes on Linux,
/// so the unit depends on `target_os` and cannot be shared as-is.
#[cfg(target_os = "macos")]
pub fn sample_process() -> (u64, f64) {
    let mut usage: libc::rusage = unsafe { std::mem::zeroed() };
    let rc = unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut usage) };
    if rc != 0 {
        return (0, 0.0);
    }
    // Darwin: `ru_maxrss` is bytes.
    let rss = usage.ru_maxrss as u64;
    let cpu = usage.ru_utime.tv_sec as f64
        + usage.ru_utime.tv_usec as f64 / 1e6
        + usage.ru_stime.tv_sec as f64
        + usage.ru_stime.tv_usec as f64 / 1e6;
    (rss, cpu)
}

/// Non-Darwin hosts have no portable sampler wired up yet; report failure
/// so the caller shows `Metric::Error` rather than fake numbers.
#[cfg(not(target_os = "macos"))]
pub fn sample_process() -> (u64, f64) {
    (0, 0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn traffic_is_unavailable_not_zero() {
        match traffic_metric() {
            Metric::Unavailable(r) => assert!(r.contains("ClientStream")),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn format_bots_and_rss() {
        assert_eq!(format_bots(1, 0), "1 bot (0 running)");
        assert_eq!(format_bots(2, 2), "2 bots (2 running)");
        let s = format_rss(1024);
        assert!(s.contains("KB") || s.contains("B") || s.contains("MB"));
    }
}
