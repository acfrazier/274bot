//! Host resource metrics for the panel. Formatters and the 1 Hz sampler live
//! in `host-play` so the TUI can show the same view without a second sample.

pub use host_play::{
    background_ack_text, cpu_from_delta, format_background, format_bots, format_rss,
    format_rss_caption, metric_text, sample_process, traffic_from_delta, traffic_from_samples,
    Metric, ResourceSampler, ResourceView,
};

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
    fn rss_caption_mentions_peak_on_macos_and_windows() {
        let s = format_rss_caption(1024);
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        assert!(s.contains("peak"), "{s}");
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        assert!(!s.contains("peak"), "{s}");
    }
}
