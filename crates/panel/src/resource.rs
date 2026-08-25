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

/// Rate from a byte-counter delta. No slots → Measuring (never fake 0 B/s).
/// Non-positive wall delta → Measuring.
pub fn traffic_from_delta(d_bytes: u64, dt_secs: f64, n_slots: usize) -> Metric {
    if n_slots == 0 || dt_secs <= 0.0 {
        return Metric::Measuring;
    }
    let bps = d_bytes as f64 / dt_secs;
    Metric::Available(format_bps(bps))
}

/// If `n_slots==0` or `dt<=0` → Measuring.
/// If `n_slots != n_prev` or `sum < sum0` → Measuring (re-baseline; do not wrapping_sub).
/// Else rate from `sum.wrapping_sub(sum0)` / dt.
pub fn traffic_from_samples(
    sum: u64,
    sum0: u64,
    dt_secs: f64,
    n_slots: usize,
    n_prev: usize,
) -> Metric {
    if n_slots == 0 || dt_secs <= 0.0 || n_slots != n_prev || sum < sum0 {
        return Metric::Measuring;
    }
    traffic_from_delta(sum.wrapping_sub(sum0), dt_secs, n_slots)
}

/// macos: `format_rss(bytes) + " peak"`; other: `format_rss(bytes)`.
pub fn format_rss_caption(bytes: u64) -> String {
    let base = format_rss(bytes);
    #[cfg(target_os = "macos")]
    {
        format!("{base} peak")
    }
    #[cfg(not(target_os = "macos"))]
    {
        base
    }
}

fn format_bps(bps: f64) -> String {
    let kb = 1024.0;
    let mb = kb * 1024.0;
    if bps < kb {
        format!("{bps:.0} B/s")
    } else if bps < mb {
        format!("{:.1} KB/s", bps / kb)
    } else {
        format!("{:.1} MB/s", bps / mb)
    }
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

/// Sampler lives in `host-play` (Darwin bytes, Linux `ru_maxrss` * 1024);
/// re-exported so `app.rs` keeps `use crate::resource::sample_process`.
pub use host_play::sample_process;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn traffic_measuring_without_slots_or_dt() {
        match traffic_from_delta(0, 1.0, 0) {
            Metric::Measuring => {}
            other => panic!("{other:?}"),
        }
        match traffic_from_delta(100, 0.0, 2) {
            Metric::Measuring => {}
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn traffic_rate_after_two_samples() {
        match traffic_from_delta(2048, 2.0, 2) {
            Metric::Available(s) => {
                assert!(s.contains("/s"), "{s}");
                assert!(
                    !s.starts_with('0') || s.contains("KB") || s.contains("B"),
                    "{s}"
                );
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn traffic_stub_unavailable_is_gone() {
        // traffic_metric deleted; rate path never returns Unavailable("…ClientStream…").
        match traffic_from_delta(0, 1.0, 1) {
            Metric::Available(s) => assert!(!s.contains("ClientStream"), "{s}"),
            Metric::Measuring => {}
            Metric::Unavailable(r) => assert!(!r.contains("ClientStream"), "{r}"),
            Metric::Error(e) => assert!(!e.contains("ClientStream"), "{e}"),
        }
    }

    #[test]
    fn format_bots_and_rss() {
        assert_eq!(format_bots(1, 0), "1 bot (0 running)");
        assert_eq!(format_bots(2, 2), "2 bots (2 running)");
        let s = format_rss(1024);
        assert!(s.contains("KB") || s.contains("B") || s.contains("MB"));
    }

    #[test]
    fn traffic_rebases_when_sum_drops_or_slot_count_changes() {
        match traffic_from_samples(10, 100, 1.0, 1, 2) {
            Metric::Measuring => {}
            other => panic!("{other:?}"),
        }
        match traffic_from_samples(10, 100, 1.0, 2, 2) {
            Metric::Measuring => {}
            other => panic!("{other:?}"),
        }
        match traffic_from_samples(200, 100, 1.0, 2, 2) {
            Metric::Available(s) => assert!(s.contains("/s"), "{s}"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn rss_caption_mentions_peak_on_macos() {
        let s = format_rss_caption(1024);
        #[cfg(target_os = "macos")]
        assert!(s.contains("peak"), "{s}");
        #[cfg(not(target_os = "macos"))]
        assert!(!s.contains("peak"), "{s}");
    }
}
