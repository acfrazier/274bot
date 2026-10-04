//! Shared prayer toggle and observed-varp sweep core.
//!
//! Isolate and native callers share one sweep state machine while owning
//! their own capacity, clock, admission and receipt handling. Fresh
//! observations settle pending bits; candidates commit only after acceptance,
//! and batch callers commit only the accepted receipt prefix.

use crate::native::{ActionContext, ActionError, ActionHandle, NativeMachine, NativeTick};
use crate::shim::InteractReq;
use api::game_data::SelectedGameData;
use api::prayer::{OnArg, PrayerObservation, PRAYER_COUNT, TOGGLE_MS};
use api::quest_progress::EvidenceStamp;
use std::sync::Arc;
use std::task::Poll;
use std::time::Duration;

/// Prayer toggles successfully raised by Combat and therefore eligible for
/// cancellation cleanup. Bits use selected overlay varp order.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RaisedPrayers(u16);

impl RaisedPrayers {
    pub const fn empty() -> Self {
        Self(0)
    }

    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    pub fn merge(&mut self, other: Self) {
        self.0 |= other.0;
    }

    /// Mark an accepted raise as owned. Ownership rules live in the callers.
    pub(crate) fn accepted(&mut self, varp: i32, on: bool, displaced: u16) {
        let Some(bit) = Self::bit(varp) else {
            return;
        };
        if on {
            self.0 = (self.0 & !displaced) | bit;
        } else {
            self.0 &= !bit;
        }
    }

    #[cfg(feature = "test-hooks")]
    #[doc(hidden)]
    pub fn from_test_varps(varps: &[i32]) -> Self {
        let mut raised = Self::empty();
        for &varp in varps {
            raised.accepted(varp, true, 0);
        }
        raised
    }

    pub fn contains(self, varp: i32) -> bool {
        Self::bit(varp).is_some_and(|bit| self.0 & bit != 0)
    }

    pub fn remove(&mut self, varp: i32) {
        if let Some(bit) = Self::bit(varp) {
            self.0 &= !bit;
        }
    }

    pub(crate) const fn mask(self) -> u16 {
        self.0
    }

    fn bit(varp: i32) -> Option<u16> {
        let index = varp.checked_sub(api::prayer::PRAYER_VARP0)?;
        if !(0..PRAYER_COUNT as i32).contains(&index) {
            return None;
        }
        Some(1_u16 << index)
    }
}

/// One prayer varp's requested state, shared by toggle and clear operations.
/// Its only caller is the `load` isolate's prayer machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(not(feature = "load"), allow(dead_code))]
pub(crate) struct PrayerToggle {
    varp: i32,
    want: OnArg,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(not(feature = "load"), allow(dead_code))]
pub(crate) enum ToggleProgress {
    Matched,
    Pending,
    TimedOut,
}

#[cfg_attr(not(feature = "load"), allow(dead_code))]
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

/// One candidate off-click. [`PrayerSweep::candidates`] only proposes it;
/// callers commit it with [`PrayerSweep::accepted`] after admission and,
/// where receipts exist, after its dispatch receipt is accepted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SweepClick {
    pub(crate) index: usize,
    pub(crate) varp: i32,
    pub(crate) button_com: i32,
}

/// The completion counters for one observed prayer sweep.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PrayerSweepReport {
    pub clicked: u32,
    pub timed_out: u32,
}

/// Ordered off-click sweep shared by native and isolate callers.
///
/// A candidate does not advance the cursor. `accepted` advances it and marks
/// that varp pending only after the caller knows the click was accepted. For
/// batches, callers accept only the accepted receipt prefix. Timeout masks are
/// caller-owned so each independently-dispatched row can expire on its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct PrayerSweep {
    cursor: u8,
    pending: u32,
    report: PrayerSweepReport,
}

impl PrayerSweep {
    pub(crate) const fn new() -> Self {
        Self {
            cursor: 0,
            pending: 0,
            report: PrayerSweepReport {
                clicked: 0,
                timed_out: 0,
            },
        }
    }

    fn bit(varp: i32) -> Option<u32> {
        let index = varp.checked_sub(api::prayer::PRAYER_VARP0)?;
        if !(0..PRAYER_COUNT as i32).contains(&index) {
            return None;
        }
        Some(1_u32 << index)
    }

    /// Settle each admitted click independently. `timed_out` and the returned
    /// mask use one bit per overlay varp offset from `PRAYER_VARP0`. A matching
    /// off observation wins; a timeout advances the best-effort sweep.
    pub(crate) fn observe(&mut self, observation: &PrayerObservation, timed_out: u32) -> u32 {
        let mut settled = 0;
        for index in 0..PRAYER_COUNT {
            let bit = 1_u32 << index;
            if self.pending & bit == 0 {
                continue;
            }
            let varp = api::prayer::PRAYER_VARP0 + index as i32;
            if observation.is_off(varp) {
                self.pending &= !bit;
                settled |= bit;
            } else if timed_out & bit != 0 {
                self.pending &= !bit;
                self.report.timed_out = self.report.timed_out.saturating_add(1);
                settled |= bit;
            }
        }
        settled
    }

    /// Active off-click candidates in selected order. The iterator is
    /// allocation-free; callers may take a capacity-limited prefix, then call
    /// `accepted` for its host/receipt-accepted prefix only.
    pub(crate) fn candidates<'a>(
        &'a self,
        data: &'a SelectedGameData,
        observation: &'a PrayerObservation,
    ) -> impl Iterator<Item = SweepClick> + 'a {
        data.prayers()
            .iter()
            .enumerate()
            .skip(usize::from(self.cursor))
            .take(PRAYER_COUNT.saturating_sub(usize::from(self.cursor)))
            .filter(move |(index, row)| {
                *index < PRAYER_COUNT
                    && Self::bit(row.varp).is_some_and(|bit| self.pending & bit == 0)
                    && observation.is_on(row.varp)
            })
            .map(|(index, row)| SweepClick {
                index,
                varp: row.varp,
                button_com: row.button_com,
            })
    }

    /// Commit one row of an accepted prefix. False means the candidate is
    /// stale, out of order, already pending, or not an overlay varp.
    pub(crate) fn accepted(&mut self, click: SweepClick) -> bool {
        let Some(bit) = Self::bit(click.varp) else {
            return false;
        };
        if click.index < usize::from(self.cursor)
            || click.index >= PRAYER_COUNT
            || self.pending & bit != 0
        {
            return false;
        }
        self.cursor = (click.index + 1) as u8;
        self.pending |= bit;
        self.report.clicked = self.report.clicked.saturating_add(1);
        true
    }

    /// One pending bit per overlay varp offset from `PRAYER_VARP0`.
    pub(crate) const fn pending_mask(&self) -> u32 {
        self.pending
    }

    pub(crate) const fn report(&self) -> PrayerSweepReport {
        self.report
    }
}

/// Result of admitting shared prayer hygiene. Scoped callers clear only
/// accepted Combat raises; broad callers may clear all active prayers.
#[allow(
    clippy::large_enum_variant,
    reason = "Hygiene is a return value, not stored beside Combat."
)]
pub enum Hygiene {
    Clean,
    Started(ActionHandle<ClearPrayers>),
    Deferred,
    Failed(ActionError),
}

/// Filter applied by `ClearPrayers`: `None` clears every observed prayer, while
/// `Some` clears only the accepted Combat raises in that ownership mask.
pub struct ClearPrayersArgs {
    data: Arc<SelectedGameData>,
    owned: Option<RaisedPrayers>,
}

impl ClearPrayersArgs {
    pub fn broad(data: Arc<SelectedGameData>) -> Self {
        Self { data, owned: None }
    }

    pub fn owned(data: Arc<SelectedGameData>, owned: RaisedPrayers) -> Self {
        Self {
            data,
            owned: Some(owned),
        }
    }
}

/// Clear only accepted Combat raises. The machine remains scoped to this mask
/// across observations and retries; no displaced prayer is restored.
pub fn begin_clear_owned_prayers(
    selected: &Arc<SelectedGameData>,
    owned: RaisedPrayers,
    tick: &mut NativeTick<'_>,
) -> Hygiene {
    if owned.is_empty() {
        return Hygiene::Clean;
    }
    start_clear_prayers(ClearPrayersArgs::owned(Arc::clone(selected), owned), tick)
}

fn start_clear_prayers(args: ClearPrayersArgs, tick: &mut NativeTick<'_>) -> Hygiene {
    match tick.actions.begin::<ClearPrayers>(args, &mut tick.cx) {
        Ok(handle) => Hygiene::Started(handle),
        Err(
            ActionError::Busy
            | ActionError::Held
            | ActionError::Stale
            | ActionError::Cancelled
            | ActionError::BudgetExhausted,
        ) => Hygiene::Deferred,
        Err(error) => Hygiene::Failed(error),
    }
}

/// Native observed-varp clear operation. Every emitted tick owns one compact
/// batch of up to five independent off-clicks; receipts commit only their
/// accepted prefix, and each accepted varp has its own timeout.
pub struct ClearPrayers {
    data: Arc<SelectedGameData>,
    owned: Option<RaisedPrayers>,
    sweep: PrayerSweep,
    last_observation: Option<EvidenceStamp>,
    last_emit: Option<EvidenceStamp>,
    pending_first_id: Option<u64>,
    pending_clicks: [Option<SweepClick>; 5],
    pending_len: u8,
    pending_since: Duration,
    deadlines: [Duration; PRAYER_COUNT],
}

impl ClearPrayers {
    fn commit_receipts(&mut self, cx: &ActionContext<'_>) -> Result<bool, ActionError> {
        let Some(first_id) = self.pending_first_id else {
            return Ok(true);
        };
        let len = usize::from(self.pending_len);
        let mut accepted = [false; 5];
        for (offset, slot) in accepted.iter_mut().enumerate().take(len) {
            let Some(request_id) = first_id.checked_add(offset as u64) else {
                return Err(ActionError::Stale);
            };
            let Some(receipt) = cx.interaction_receipt(request_id) else {
                return Ok(false);
            };
            *slot = receipt.accepted;
        }

        let mut prefix = true;
        let deadline = self
            .pending_since
            .saturating_add(Duration::from_millis(TOGGLE_MS));
        for (offset, &was_accepted) in accepted.iter().enumerate().take(len) {
            if !was_accepted {
                prefix = false;
                continue;
            }
            if !prefix {
                continue;
            }
            let Some(click) = self.pending_clicks[offset] else {
                return Err(ActionError::Stale);
            };
            if !self.sweep.accepted(click) {
                return Err(ActionError::Stale);
            }
            let Some(bit) = PrayerSweep::bit(click.varp) else {
                return Err(ActionError::Stale);
            };
            self.deadlines[bit.trailing_zeros() as usize] = deadline;
        }

        self.pending_first_id = None;
        self.pending_clicks = [None; 5];
        self.pending_len = 0;
        self.pending_since = Duration::ZERO;
        Ok(true)
    }

    fn timed_out(&self, now: Duration) -> u32 {
        let pending = self.sweep.pending_mask();
        let mut timed_out = 0;
        for index in 0..PRAYER_COUNT {
            let bit = 1_u32 << index;
            if pending & bit != 0 && now >= self.deadlines[index] {
                timed_out |= bit;
            }
        }
        timed_out
    }
}

impl NativeMachine for ClearPrayers {
    type Args = ClearPrayersArgs;
    type Output = PrayerSweepReport;

    fn begin(args: Self::Args, _cx: &mut ActionContext<'_>) -> Result<Self, ActionError> {
        Ok(Self {
            data: args.data,
            owned: args.owned,
            sweep: PrayerSweep::new(),
            last_observation: None,
            last_emit: None,
            pending_first_id: None,
            pending_clicks: [None; 5],
            pending_len: 0,
            pending_since: Duration::ZERO,
            deadlines: [Duration::ZERO; PRAYER_COUNT],
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

        let receipts_ready = match self.commit_receipts(cx) {
            Ok(ready) => ready,
            Err(error) => return Poll::Ready(Err(error)),
        };
        let now = cx.active_now();
        let settled = self.sweep.observe(&observation, self.timed_out(now));
        for index in 0..PRAYER_COUNT {
            if settled & (1_u32 << index) != 0 {
                self.deadlines[index] = Duration::ZERO;
            }
        }
        if !receipts_ready {
            return Poll::Pending;
        }

        let mut clicks = [None; 5];
        let mut rows = std::array::from_fn(|_| None);
        let mut len = 0;
        for click in self
            .sweep
            .candidates(&self.data, &observation)
            .filter(|click| self.owned.is_none_or(|owned| owned.contains(click.varp)))
            .take(clicks.len())
        {
            clicks[len] = Some(click);
            rows[len] = Some(InteractReq::IfButton {
                component_id: click.button_com,
            });
            len += 1;
        }
        if len == 0 {
            if self.sweep.pending_mask() != 0 {
                return Poll::Pending;
            }
            if self.owned.is_some_and(|owned| {
                self.data
                    .prayers()
                    .iter()
                    .any(|row| owned.contains(row.varp) && !observation.varp_observed(row.varp))
            }) {
                return Poll::Pending;
            }
            let report = self.sweep.report();
            return if report.timed_out == 0 {
                Poll::Ready(Ok(report))
            } else {
                Poll::Ready(Err(ActionError::Failed(
                    "prayer cleanup did not settle".into(),
                )))
            };
        }
        if self
            .last_emit
            .is_some_and(|emitted| emitted.run == stamp.run && emitted.tick == stamp.tick)
        {
            return Poll::Pending;
        }

        match cx.emit_batch(rows) {
            Ok(first_id) => {
                self.pending_first_id = Some(first_id);
                self.pending_clicks = clicks;
                self.pending_len = len as u8;
                self.pending_since = now;
                self.last_emit = Some(stamp);
                Poll::Pending
            }
            Err(ActionError::BudgetExhausted) => Poll::Pending,
            Err(error) => Poll::Ready(Err(error)),
        }
    }

    fn cancel(&mut self) {
        // Revoking the NativeActions owner is sufficient; cleanup has no
        // compensating click to emit.
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native::{ledger, HostEffect, InteractionReceipt};
    use crate::quester::families::tests::with_tick;
    use api::game_data::SelectedGameData;
    use api::quest_progress::EvidenceStamp;
    use api::selected::ClientRevision;
    use api::snapshot::{GameSnapshot, VarpView};

    fn selected_data() -> Arc<SelectedGameData> {
        api::game_data::for_revision(ClientRevision::R289).expect("selected prayer data")
    }

    fn seed_prayers(
        snapshot: &mut GameSnapshot,
        data: &SelectedGameData,
        active: usize,
        off: &[usize],
    ) {
        snapshot.seed_varps(
            data.prayers()
                .iter()
                .enumerate()
                .map(|(index, row)| VarpView {
                    index: row.varp,
                    value: if index < active && !off.contains(&index) {
                        1
                    } else {
                        0
                    },
                })
                .collect(),
        );
    }

    fn expected_buttons(data: &SelectedGameData, start: usize, end: usize) -> Vec<InteractReq> {
        data.prayers()[start..end]
            .iter()
            .map(|row| InteractReq::IfButton {
                component_id: row.button_com,
            })
            .collect()
    }

    fn assert_batch(ledger: &Option<Box<ledger::Ledger>>, len: usize) {
        let outbox = &ledger.as_ref().expect("action ledger").outbox;
        assert_eq!(outbox.len(), len);
        let first = outbox[0].request_id.get();
        for (offset, action) in outbox.iter().enumerate() {
            assert_eq!(action.batch, first);
            assert_eq!(action.request_id.get(), first + offset as u64);
        }
    }

    fn dispatch(
        ledger: &mut Option<Box<ledger::Ledger>>,
        accepted: &[bool],
        tick: u64,
    ) -> Vec<InteractReq> {
        let ledger = ledger.as_mut().expect("action ledger");
        assert_eq!(ledger.outbox.len(), accepted.len());
        let mut requests = Vec::with_capacity(accepted.len());
        for &accepted in accepted {
            let action = ledger.outbox.remove(0);
            ledger.complete_interaction(
                &action.authority(),
                InteractionReceipt {
                    request_id: action.request_id.get(),
                    evidence: EvidenceStamp {
                        run: action.run(),
                        tick,
                        sequence: tick,
                    },
                    accepted,
                    chat_since: 0,
                },
            );
            match action.effect {
                HostEffect::Interaction(request) => requests.push(request),
                HostEffect::Walk(_) | HostEffect::BankPick(_) => {
                    panic!("prayer clear only emits interactions")
                }
            }
        }
        requests
    }

    #[test]
    fn sweep_settles_bits_independently_and_advances_after_timeout() {
        let data = selected_data();
        let mut observation = PrayerObservation::empty();
        for row in &data.prayers()[..3] {
            observation.set_varp(row.varp, 1);
        }

        let mut sweep = PrayerSweep::new();
        let mut candidates = sweep.candidates(&data, &observation);
        let first = candidates.next().expect("first active prayer");
        let second = candidates.next().expect("second active prayer");
        drop(candidates);
        assert_eq!(first.index, 0);
        assert_eq!(second.index, 1);
        assert!(sweep.accepted(first));
        assert!(sweep.accepted(second));
        assert_eq!(sweep.pending_mask(), 0b11);

        observation.set_varp(first.varp, 0);
        let settled = sweep.observe(&observation, 0b11);
        assert_eq!(settled, 0b11);
        assert_eq!(sweep.pending_mask(), 0);
        assert_eq!(sweep.report().timed_out, 1, "observed-off wins for bit 0");
        assert_eq!(
            sweep
                .candidates(&data, &observation)
                .next()
                .map(|click| click.index),
            Some(2),
            "a timed-out still-on row does not block later clear candidates"
        );
    }

    #[test]
    fn policy_s2_scoped_clear_waits_for_missing_owned_row_and_preserves_user() {
        let data = selected_data();
        let skin = data.prayer_by_name("Thick Skin").unwrap();
        let protect = data.prayer_by_name("Protect from Melee").unwrap();
        let mut owned = RaisedPrayers::empty();
        owned.accepted(protect.varp, true, 0);
        let mut snapshot = GameSnapshot::new();
        snapshot.seed_ingame(2);
        snapshot.seed_varps(vec![VarpView {
            index: skin.varp,
            value: 1,
        }]);
        let mut ledger = None;
        let handle = with_tick(&snapshot, &mut ledger, 0, |tick| {
            tick.actions
                .begin::<ClearPrayers>(
                    ClearPrayersArgs::owned(Arc::clone(&data), owned),
                    &mut tick.cx,
                )
                .unwrap()
        });
        assert!(with_tick(&snapshot, &mut ledger, 1, |tick| {
            tick.actions.poll(&handle, &mut tick.cx).is_pending()
        }));
        assert!(ledger.as_ref().unwrap().outbox.is_empty());
        snapshot.seed_varps(vec![
            VarpView {
                index: skin.varp,
                value: 1,
            },
            VarpView {
                index: protect.varp,
                value: 1,
            },
        ]);
        assert!(with_tick(&snapshot, &mut ledger, 2, |tick| {
            tick.actions.poll(&handle, &mut tick.cx).is_pending()
        }));
        assert_eq!(
            dispatch(&mut ledger, &[true], 2),
            vec![InteractReq::IfButton {
                component_id: protect.button_com
            }]
        );
        snapshot.seed_varps(vec![
            VarpView {
                index: skin.varp,
                value: 1,
            },
            VarpView {
                index: protect.varp,
                value: 0,
            },
        ]);
        assert!(matches!(
            with_tick(&snapshot, &mut ledger, 3, |tick| {
                tick.actions.poll(&handle, &mut tick.cx)
            }),
            Poll::Ready(Ok(PrayerSweepReport {
                clicked: 1,
                timed_out: 0
            }))
        ));
        assert!(ledger.as_ref().unwrap().outbox.is_empty());
    }

    #[test]
    fn clear_batches_ordered_offs_keep_failed_suffix_and_report_timeout() {
        let data = selected_data();
        assert!(data.prayers().len() >= 6);
        let mut snapshot = GameSnapshot::new();
        snapshot.seed_ingame(2);
        seed_prayers(&mut snapshot, &data, 6, &[]);

        let mut ledger = None;
        let handle = with_tick(&snapshot, &mut ledger, 0, |tick| {
            tick.actions
                .begin::<ClearPrayers>(ClearPrayersArgs::broad(Arc::clone(&data)), &mut tick.cx)
                .expect("begin prayer clear")
        });

        assert!(with_tick(&snapshot, &mut ledger, 1, |tick| {
            tick.actions.poll(&handle, &mut tick.cx).is_pending()
        }));
        assert_batch(&ledger, 5);
        assert_eq!(
            dispatch(&mut ledger, &[true, true, false, false, false], 1),
            expected_buttons(&data, 0, 5)
        );

        // The false receipt starts a fail-stop suffix. Its first row remains
        // the cursor, and the next owned batch retries that row before any
        // later active prayer; the accepted prefix is not clicked twice.
        assert!(with_tick(&snapshot, &mut ledger, 2, |tick| {
            tick.actions.poll(&handle, &mut tick.cx).is_pending()
        }));
        assert_batch(&ledger, 4);
        assert_eq!(
            dispatch(&mut ledger, &[true, true, true, true], 2),
            expected_buttons(&data, 2, 6)
        );

        assert!(with_tick(&snapshot, &mut ledger, 3, |tick| {
            tick.actions.poll(&handle, &mut tick.cx).is_pending()
        }));
        assert!(ledger.as_ref().unwrap().outbox.is_empty());

        // One accepted bit settles independently while the others remain
        // pending. At tick 5 the older row 1 expires; rows 2–5 do not yet.
        seed_prayers(&mut snapshot, &data, 6, &[0]);
        assert!(with_tick(&snapshot, &mut ledger, 4, |tick| {
            tick.actions.poll(&handle, &mut tick.cx).is_pending()
        }));
        assert!(ledger.as_ref().unwrap().outbox.is_empty());

        assert!(with_tick(&snapshot, &mut ledger, 5, |tick| {
            tick.actions.poll(&handle, &mut tick.cx).is_pending()
        }));
        assert!(
            ledger.as_ref().unwrap().outbox.is_empty(),
            "an accepted off-click is not retried after its observation timeout"
        );

        seed_prayers(&mut snapshot, &data, 6, &[0, 1, 2, 3, 4, 5]);
        assert!(
            matches!(
                with_tick(&snapshot, &mut ledger, 6, |tick| {
                    tick.actions.poll(&handle, &mut tick.cx)
                }),
                Poll::Ready(Err(ActionError::Failed(_)))
            ),
            "a timed-out clear fails even when the other components later settle"
        );
        assert!(ledger.as_ref().unwrap().outbox.is_empty());
    }
}
