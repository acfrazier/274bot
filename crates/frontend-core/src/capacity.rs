//! Shared host-capacity warning for the panel, the TUI and the log.

pub use host::{CapacityWarning, RenderPolicyLabel};

/// One process-wide warning: the host cannot service the fleet on time.
/// Panel, TUI and the session log all show [`Self::notice_text`].
#[derive(Debug, Clone, PartialEq)]
pub struct CapacityNotice {
    warning: CapacityWarning,
    text: String,
}

impl CapacityNotice {
    /// Format the shared line from a host snapshot.
    pub fn from_warning(warning: CapacityWarning) -> Self {
        let p95 = warning.p95_delay.as_secs_f64();
        let tick = f64::from(warning.tick_ms) / 1000.0;
        let text = format!(
            "Host is falling behind: {} of {} bots late (p95 delay {p95:.2}s, {tick:.2} s/tick, render {}). Run fewer bots or turn off render-all.",
            warning.late_bots,
            warning.active_bots,
            warning.render.as_str(),
        );
        Self { warning, text }
    }

    /// The line both front ends and the log show.
    pub fn notice_text(&self) -> &str {
        &self.text
    }

    /// Counts and timings the line was built from.
    pub fn warning(&self) -> CapacityWarning {
        self.warning
    }
}

impl From<CapacityWarning> for CapacityNotice {
    fn from(warning: CapacityWarning) -> Self {
        Self::from_warning(warning)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn notice_names_counts_delay_tick_rate_render_and_the_hint() {
        let notice = CapacityNotice::from_warning(CapacityWarning {
            late_bots: 9,
            active_bots: 90,
            threshold: 9,
            p95_delay: Duration::from_millis(820),
            tick_ms: 750,
            render: RenderPolicyLabel::RenderAll,
        });
        let text = notice.notice_text();
        assert!(text.contains("9 of 90 bots late"), "{text}");
        assert!(text.contains("p95 delay 0.82s"), "{text}");
        assert!(text.contains("0.75 s/tick"), "{text}");
        assert!(text.contains("render render-all"), "{text}");
        assert!(
            text.contains("Run fewer bots or turn off render-all"),
            "{text}"
        );
        assert_eq!(notice.warning().late_bots, 9);
        assert_eq!(notice.warning().render, RenderPolicyLabel::RenderAll);
    }
}
