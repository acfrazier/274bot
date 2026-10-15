//! Shared host-capacity warning for the panel, the TUI and the log.

use std::fmt::Write as _;

pub use host::{CapacityWarning, RenderPolicyLabel};

/// One process-wide warning: this machine is not giving the bots CPU time
/// when they need it. Panel, TUI and the session log all show
/// [`Self::notice_text`]. Built only when the warning's figures change.
#[derive(Debug, PartialEq)]
pub struct CapacityNotice {
    warning: CapacityWarning,
    text: String,
}

impl CapacityNotice {
    /// Format the shared line from a host snapshot. A figure with no
    /// samples is left out; the render-all hint shows only while every bot
    /// renders.
    pub fn from_warning(warning: CapacityWarning) -> Self {
        let mut text = format!(
            "Host is falling behind: {} of {} bots late (",
            warning.late_bots(),
            warning.active_bots()
        );
        if let Some(p95) = warning.p95_delay() {
            let _ = write!(text, "p95 delay {:.2}s, ", p95.as_secs_f64());
        }
        if let Some(tick) = warning.tick_period() {
            let _ = write!(text, "{:.2} s/tick, ", tick.as_secs_f64());
        }
        let _ = write!(text, "render {}). ", warning.render().as_str());
        text.push_str(match warning.render() {
            RenderPolicyLabel::RenderAll => "Run fewer bots or turn off render-all.",
            RenderPolicyLabel::OnlySelected | RenderPolicyLabel::Headless => "Run fewer bots.",
        });
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn notice_names_counts_delay_tick_rate_render_and_the_hint() {
        let notice = CapacityNotice::from_warning(CapacityWarning::new(
            9,
            90,
            9,
            Some(Duration::from_millis(820)),
            Some(Duration::from_millis(750)),
            RenderPolicyLabel::RenderAll,
        ));
        assert_eq!(
            notice.notice_text(),
            "Host is falling behind: 9 of 90 bots late (p95 delay 0.82s, 0.75 s/tick, \
             render render-all). Run fewer bots or turn off render-all."
        );
        assert_eq!(notice.warning().late_bots(), 9);
        assert_eq!(notice.warning().render(), RenderPolicyLabel::RenderAll);
    }

    #[test]
    fn missing_figures_are_left_out_and_render_all_is_hinted_only_when_on() {
        let headless = CapacityNotice::from_warning(CapacityWarning::new(
            3,
            20,
            3,
            None,
            None,
            RenderPolicyLabel::Headless,
        ));
        assert_eq!(
            headless.notice_text(),
            "Host is falling behind: 3 of 20 bots late (render headless). Run fewer bots."
        );
        let selected = CapacityNotice::from_warning(CapacityWarning::new(
            3,
            20,
            3,
            Some(Duration::from_millis(700)),
            None,
            RenderPolicyLabel::OnlySelected,
        ));
        assert!(
            !selected.notice_text().contains("render-all"),
            "{}",
            selected.notice_text()
        );
    }
}
