//! Shared host-capacity warning for the panel, the TUI and the log.

use std::fmt::Write as _;
use std::time::Duration;

/// How the front end is drawing, for the warning line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderPolicyLabel {
    /// Panel default: only the focused bot rasters.
    OnlySelected,
    /// Every wall member rasters (render-all).
    RenderAll,
    /// TUI / headless: no game raster.
    Headless,
}

impl RenderPolicyLabel {
    /// End-user label in the warning line.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::OnlySelected => "only selected",
            Self::RenderAll => "render-all",
            Self::Headless => "headless",
        }
    }

    pub(crate) fn from_host(label: host::RenderPolicyLabel) -> Self {
        match label {
            host::RenderPolicyLabel::OnlySelected => Self::OnlySelected,
            host::RenderPolicyLabel::RenderAll => Self::RenderAll,
            host::RenderPolicyLabel::Headless => Self::Headless,
        }
    }

    pub(crate) fn to_host(self) -> host::RenderPolicyLabel {
        match self {
            Self::OnlySelected => host::RenderPolicyLabel::OnlySelected,
            Self::RenderAll => host::RenderPolicyLabel::RenderAll,
            Self::Headless => host::RenderPolicyLabel::Headless,
        }
    }
}

/// Snapshot of the host being behind: the figures the warning line shows.
/// Rounded to 10 ms by the host that measured them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CapacityWarning {
    late_bots: u16,
    active_bots: u16,
    threshold: u16,
    p95_delay: Option<Duration>,
    tick_period: Option<Duration>,
    render: RenderPolicyLabel,
}

impl CapacityWarning {
    pub(crate) fn from_host(warning: host::CapacityWarning) -> Self {
        Self {
            late_bots: warning.late_bots(),
            active_bots: warning.active_bots(),
            threshold: warning.threshold(),
            p95_delay: warning.p95_delay(),
            tick_period: warning.tick_period(),
            render: RenderPolicyLabel::from_host(warning.render()),
        }
    }

    /// A warning snapshot as the host would build it (durations rounded to
    /// 10 ms), for tests.
    #[cfg(any(test, feature = "test-support"))]
    pub fn new(
        late_bots: u16,
        active_bots: u16,
        threshold: u16,
        p95_delay: Option<Duration>,
        tick_period: Option<Duration>,
        render: RenderPolicyLabel,
    ) -> Self {
        Self::from_host(host::CapacityWarning::new(
            late_bots,
            active_bots,
            threshold,
            p95_delay,
            tick_period,
            render.to_host(),
        ))
    }

    /// This snapshot as the host's, for tests that feed the session a
    /// host evaluation.
    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn to_host(self) -> host::CapacityWarning {
        host::CapacityWarning::new(
            self.late_bots,
            self.active_bots,
            self.threshold,
            self.p95_delay,
            self.tick_period,
            self.render.to_host(),
        )
    }

    /// Active bots with several late services in the recent window.
    pub fn late_bots(&self) -> u16 {
        self.late_bots
    }

    /// Active bots (parked, paused, suspended and quiet bots excluded).
    pub fn active_bots(&self) -> u16 {
        self.active_bots
    }

    /// How many late bots trip the warning.
    pub fn threshold(&self) -> u16 {
        self.threshold
    }

    /// p95 of the recent long scheduling delays; `None` when there were
    /// none.
    pub fn p95_delay(&self) -> Option<Duration> {
        self.p95_delay
    }

    /// Median time per game tick the active bots processed; `None`
    /// without ticks.
    pub fn tick_period(&self) -> Option<Duration> {
        self.tick_period
    }

    /// What the front end is rendering.
    pub fn render(&self) -> RenderPolicyLabel {
        self.render
    }
}

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

    #[test]
    fn every_render_policy_label_converts_to_the_same_host_label_and_back() {
        for (label, host_label) in [
            (
                RenderPolicyLabel::OnlySelected,
                host::RenderPolicyLabel::OnlySelected,
            ),
            (
                RenderPolicyLabel::RenderAll,
                host::RenderPolicyLabel::RenderAll,
            ),
            (
                RenderPolicyLabel::Headless,
                host::RenderPolicyLabel::Headless,
            ),
        ] {
            assert_eq!(label.to_host(), host_label);
            assert_eq!(RenderPolicyLabel::from_host(host_label), label);
            assert_eq!(label.as_str(), host_label.as_str());
        }
    }

    #[test]
    fn a_host_warning_keeps_every_figure() {
        let host_warning = host::CapacityWarning::new(
            7,
            64,
            6,
            Some(Duration::from_millis(1230)),
            Some(Duration::from_millis(640)),
            host::RenderPolicyLabel::RenderAll,
        );
        let warning = CapacityWarning::from_host(host_warning);
        assert_eq!(warning.late_bots(), host_warning.late_bots());
        assert_eq!(warning.active_bots(), host_warning.active_bots());
        assert_eq!(warning.threshold(), host_warning.threshold());
        assert_eq!(warning.p95_delay(), host_warning.p95_delay());
        assert_eq!(warning.tick_period(), host_warning.tick_period());
        assert_eq!(warning.render(), RenderPolicyLabel::RenderAll);
        assert_eq!(
            (
                warning.late_bots(),
                warning.active_bots(),
                warning.threshold(),
                warning.p95_delay(),
                warning.tick_period(),
            ),
            (
                7,
                64,
                6,
                Some(Duration::from_millis(1230)),
                Some(Duration::from_millis(640)),
            )
        );
        assert_eq!(warning.to_host(), host_warning);

        let bare = CapacityWarning::from_host(host::CapacityWarning::new(
            3,
            20,
            3,
            None,
            None,
            host::RenderPolicyLabel::Headless,
        ));
        assert_eq!((bare.p95_delay(), bare.tick_period()), (None, None));
        assert_eq!(bare.render(), RenderPolicyLabel::Headless);
    }

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
