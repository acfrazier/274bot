//! Shared prayer toggle and observed-varp sweep core.
//!
//! Both the isolate prayer family and native combat use these state rules.
//! Callers own their clock and admission surface: observations commit once
//! per fresh frame, while a sweep click commits only after its request is
//! admitted.

use crate::native::{ActionContext, ActionError, NativeMachine};
use crate::shim::InteractReq;
use api::game_data::SelectedGameData;
use api::prayer::{OnArg, PrayerObservation, PRAYER_COUNT, TOGGLE_MS};
use api::quest_progress::EvidenceStamp;
use std::sync::Arc;
use std::task::Poll;
use std::time::Duration;

/// One prayer varp's requested state, shared by toggle and clear operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PrayerToggle {
    varp: i32,
    want: OnArg,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ToggleProgress {
    Matched,
    Pending,
    TimedOut,
}

impl PrayerToggle {
    pub(crate) const fn new(varp: i32, want: OnArg) -> Self {
        Self { varp, want }
    }

    /// Apply the shared observed-varp-before-timeout rule.
    pub(crate) fn progress(
        self,
        observation: &PrayerObservation,
        timed_out: bool,
    ) -> ToggleProgress {
        let matched = match self.want {
            OnArg::Bool(true) => observation.is_on(self.varp),
            OnArg::Bool(false) => observation.is_off(self.varp),
            OnArg::Undefined | OnArg::Other { .. } => false,
        };
        if matched {
            ToggleProgress::Matched
        } else if timed_out {
            ToggleProgress::TimedOut
        } else {
            ToggleProgress::Pending
        }
    }
}

/// An admitted off-click candidate. Constructed by [`PrayerSweep::next`],
/// then committed with [`PrayerSweep::emitted`] only after host admission.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SweepClick {
    pub(crate) index: usize,
    pub(crate) varp: i32,
    pub(crate) button_com: i32,
}

/// The completion counters for one observed prayer sweep.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct PrayerSweepReport {
    pub(crate) clicked: u32,
    pub(crate) timed_out: u32,
}

/// What the next sweep step can do without changing operation-owned state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SweepDecision {
    Wait,
    Click(SweepClick),
    Done(PrayerSweepReport),
}

/// Ordered off-click sweep. Observation transitions may be committed at a
/// fresh observation even on a no-op tick; the cursor and click counter move
/// only when [`Self::emitted`] confirms a request was admitted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct PrayerSweep {
    cursor: usize,
    pending: Option<PrayerToggle>,
    report: PrayerSweepReport,
}

impl PrayerSweep {
    pub(crate) const fn new() -> Self {
        Self {
            cursor: 0,
            pending: None,
            report: PrayerSweepReport {
                clicked: 0,
                timed_out: 0,
            },
        }
    }

    /// Commit observed settle or timeout for the preceding admitted click.
    /// Returns true exactly when a pending click was cleared.
    pub(crate) fn observe(
        &mut self,
        observation: &PrayerObservation,
        timed_out: bool,
    ) -> bool {
        let Some(pending) = self.pending else {
            return false;
        };
        match pending.progress(observation, timed_out) {
            ToggleProgress::Matched => {
                self.pending = None;
                true
            }
            ToggleProgress::TimedOut => {
                self.pending = None;
                self.report.timed_out = self.report.timed_out.saturating_add(1);
                true
            }
            ToggleProgress::Pending => false,
        }
    }

    /// Return the next operation without advancing the cursor. Missing
    /// selected data means there are no rows to inspect, matching the legacy
    /// family's settled-empty behavior.
    pub(crate) fn next(
        &self,
        data: Option<&SelectedGameData>,
        observation: &PrayerObservation,
    ) -> SweepDecision {
        if self.pending.is_some() {
            return SweepDecision::Wait;
        }
        if let Some(data) = data {
            for (index, row) in data
                .prayers()
                .iter()
                .enumerate()
                .skip(self.cursor)
                .take(PRAYER_COUNT.saturating_sub(self.cursor))
            {
                if observation.is_on(row.varp) {
                    return SweepDecision::Click(SweepClick {
                        index,
                        varp: row.varp,
                        button_com: row.button_com,
                    });
                }
            }
        }
        SweepDecision::Done(self.report)
    }

    /// Commit a click only after its caller's emit/admission succeeded.
    pub(crate) fn emitted(&mut self, click: SweepClick) {
        debug_assert!(self.pending.is_none());
        debug_assert!(click.index >= self.cursor);
        debug_assert!(click.index < PRAYER_COUNT);
        if self.pending.is_some() || click.index < self.cursor || click.index >= PRAYER_COUNT {
            return;
        }
        self.cursor = click.index + 1;
        self.pending = Some(PrayerToggle::new(click.varp, OnArg::Bool(false)));
        self.report.clicked = self.report.clicked.saturating_add(1);
    }
}

/// Native observed-varp clear operation, usable with `NativeActions::begin`
/// and `poll`. The selected table is shared by Arc, not copied per tick.
pub(crate) struct ClearPrayers {
    data: Arc<SelectedGameData>,
    sweep: PrayerSweep,
    last_observation: Option<EvidenceStamp>,
    last_emit: Option<EvidenceStamp>,
    deadline: Option<Duration>,
}

impl NativeMachine for ClearPrayers {
    type Args = Arc<SelectedGameData>;
    type Output = PrayerSweepReport;

    fn begin(data: Self::Args, _cx: &mut ActionContext<'_>) -> Result<Self, ActionError> {
        Ok(Self {
            data,
            sweep: PrayerSweep::new(),
            last_observation: None,
            last_emit: None,
            deadline: None,
        })
    }

    fn poll(&mut self, cx: &mut ActionContext<'_>) -> Poll<Result<Self::Output, ActionError>> {
        let Some(varps) = cx.snapshot().varps() else {
            // An absent getter is not evidence that all prayers are off.
            return Poll::Pending;
        };
        let stamp = varps.stamp;
        if self.last_observation == Some(stamp) {
            return Poll::Pending;
        }
        // Observation accounting commits independently of whether this fresh
        // frame produces an admitted operation.
        self.last_observation = Some(stamp);

        let mut observation = PrayerObservation::empty();
        for varp in varps.value {
            observation.set_varp(varp.index, varp.value);
        }
        let timed_out = self.deadline.is_some_and(|deadline| cx.active_now() >= deadline);
        if self.sweep.observe(&observation, timed_out) {
            self.deadline = None;
        }

        match self.sweep.next(Some(&self.data), &observation) {
            SweepDecision::Wait => Poll::Pending,
            SweepDecision::Done(report) => Poll::Ready(Ok(report)),
            SweepDecision::Click(click) => {
                if self.last_emit.is_some_and(|emitted| {
                    emitted.run == stamp.run && emitted.tick == stamp.tick
                }) {
                    return Poll::Pending;
                }
                match cx.emit(InteractReq::IfButton {
                    component_id: click.button_com,
                }) {
                    Ok(_) => {
                        self.sweep.emitted(click);
                        self.last_emit = Some(stamp);
                        self.deadline = Some(
                            cx.active_now()
                                .saturating_add(Duration::from_millis(TOGGLE_MS)),
                        );
                        Poll::Pending
                    }
                    Err(ActionError::BudgetExhausted) => Poll::Pending,
                    Err(error) => Poll::Ready(Err(error)),
                }
            }
        }
    }

    fn cancel(&mut self) {
        // Revoking the NativeActions owner is sufficient; cleanup has no
        // compensating click to emit.
    }
}
