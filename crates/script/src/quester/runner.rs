//! §3.2 tick loop: colour-only stage, watchdog, death, Stop/resume, no provision.
use super::compile::{CompiledPath, CompiledStep, PredicateContext, StepContext, StepRun};
use super::death::DeathLatch;
use super::progress::{colour_stage, LiveEvidence};
use super::provision::Provisioner;
use super::select::{select, sequence_for_stage};
use super::watchdog::{Watchdog, WatchdogAction};
use crate::native::{
    Interrupt, NativeOutput, NativePhase, NativeTick, Script, ScriptFailure, ScriptFlow,
    ScriptStatus, StatusField, StatusValue, StopReason,
};
use crate::CompiledId;
use api::quest_facts::QuestCatalog;
use api::quest_progress::{EvidenceProvider, EvidenceStamp};
use api::selected::{FactKey, RunKey, Truth};
use std::sync::Arc;
use std::task::Poll;

pub struct Quester {
    run: RunKey,
    path: Arc<CompiledPath>,
    quests: Arc<QuestCatalog>,
    evidence: Arc<LiveEvidence>,
    stage: Option<FactKey>,
    seq_index: usize,
    step_index: usize,
    step: Option<Box<dyn StepRun>>,
    advances: bool,
    in_prelude: bool,
    settling: bool,
    settle_ticks: u8,
    chat_since: u64,
    needs_read: bool,
    deaths: u8,
    attempts: u8,
    fail_streak: u8,
    parked: bool,
    journal_opened: bool,
    dirty: bool,
    watchdog: Watchdog,
    death: DeathLatch,
    provisioner: Provisioner,
}

impl Quester {
    pub fn new(run: RunKey, path: Arc<CompiledPath>, quests: Arc<QuestCatalog>) -> Self {
        let stamp = EvidenceStamp {
            run,
            tick: 0,
            sequence: 0,
        };
        Self {
            run,
            path,
            quests,
            evidence: Arc::new(LiveEvidence {
                colours: Vec::new(),
                varps: Vec::new(),
                stamp,
            }),
            stage: None,
            seq_index: 0,
            step_index: 0,
            step: None,
            advances: false,
            in_prelude: false,
            settling: false,
            settle_ticks: 0,
            chat_since: 0,
            needs_read: true,
            deaths: 0,
            attempts: 0,
            fail_streak: 0,
            parked: false,
            journal_opened: false,
            dirty: true,
            watchdog: Watchdog::default(),
            death: DeathLatch::default(),
            provisioner: Provisioner::None,
        }
    }

    pub fn journal_opened(&self) -> bool {
        self.journal_opened
    }

    pub fn deaths(&self) -> u8 {
        self.deaths
    }

    pub fn stage(&self) -> Option<&FactKey> {
        self.stage.as_ref()
    }

    fn publish(&mut self, output: &mut dyn NativeOutput) {
        if !self.dirty {
            return;
        }
        self.dirty = false;
        let stage = self
            .stage
            .as_ref()
            .map(|s| StatusValue::Text(Arc::from(s.0.as_ref())))
            .unwrap_or(StatusValue::Text(Arc::from("unknown")));
        output.status(ScriptStatus {
            run: self.run,
            card: CompiledId("Quester"),
            phase: if self.parked {
                NativePhase::Blocked
            } else {
                NativePhase::Working
            },
            active_settings: 1,
            pending_settings: None,
            fields: Arc::from([
                StatusField {
                    key: "quest",
                    label: "Quest",
                    value: StatusValue::Text(Arc::clone(&self.path.display_name)),
                },
                StatusField {
                    key: "stage",
                    label: "Stage",
                    value: stage,
                },
                StatusField {
                    key: "deaths",
                    label: "Deaths",
                    value: StatusValue::Integer(i64::from(self.deaths)),
                },
            ]),
            failure: None,
        });
    }

    fn cancel_step(&mut self, tick: &mut NativeTick<'_>) {
        if let Some(mut step) = self.step.take() {
            step.cancel(tick.actions);
        }
        self.advances = false;
        self.attempts = 0;
        self.settling = false;
        self.settle_ticks = 0;
    }

    fn current_step(&self) -> Option<&CompiledStep> {
        if self.in_prelude {
            self.path.prelude.get(self.step_index)
        } else {
            self.path
                .sequences
                .get(self.seq_index)?
                .steps
                .get(self.step_index)
        }
    }

    fn read_stage(&mut self, tick: &NativeTick<'_>, retarget: bool) {
        let stamp = tick.cx.evidence();
        self.evidence = Arc::new(LiveEvidence::from_snapshot(
            tick.cx.snapshot(),
            &self.quests,
            stamp,
        ));
        if let Some(stage) = colour_stage(&self.path, &self.quests, tick.cx.snapshot()) {
            if self.stage.as_ref() != Some(&stage) {
                self.stage = Some(stage.clone());
                if retarget {
                    self.seq_index = sequence_for_stage(&self.path, stage.0.as_ref()).unwrap_or(0);
                    self.step_index = 0;
                }
                self.dirty = true;
            }
        }
        self.needs_read = false;
    }

    fn on_step_boundary(&mut self, tick: &NativeTick<'_>) {
        let mut inv = [(0, 0); 28];
        let mut inv_len = 0usize;
        if let Some(rows) = tick.cx.snapshot().inventory() {
            for item in rows.value.iter().take(28) {
                inv[inv_len] = (item.def.id, item.count);
                inv_len += 1;
            }
        }
        let mut worn = [0; 14];
        let mut worn_len = 0usize;
        if let Some(rows) = tick.cx.snapshot().equipment() {
            for item in rows.value.iter().take(14) {
                worn[worn_len] = item.def.id;
                worn_len += 1;
            }
        }
        let tile = tick.cx.snapshot().here().map(|obs| obs.value);
        let xp = tick
            .cx
            .snapshot()
            .stats()
            .map(|stats| stats.value.iter().map(|s| s.xp).sum())
            .unwrap_or(0);
        if matches!(
            self.watchdog.observe(
                self.stage.as_ref(),
                tile,
                &inv[..inv_len],
                &worn[..worn_len],
                xp,
            ),
            WatchdogAction::Park
        ) {
            self.parked = true;
            self.dirty = true;
        }
    }
}

impl Script for Quester {
    fn tick(&mut self, tick: &mut NativeTick<'_>) -> Result<ScriptFlow, ScriptFailure> {
        if tick.cx.run() != self.run {
            self.run = tick.cx.run();
            self.cancel_step(tick);
            self.needs_read = true;
        }
        if !tick.cx.eligible {
            self.publish(tick.output);
            return Ok(ScriptFlow::Continue);
        }
        if self.parked {
            self.publish(tick.output);
            return Ok(ScriptFlow::Blocked(ScriptFailure {
                code: Arc::from("parked"),
                message: Arc::from("no progress"),
                retryable: true,
            }));
        }
        if self.death.observe(tick.cx.snapshot()) {
            self.cancel_step(tick);
            self.deaths = self.deaths.saturating_add(1);
            self.needs_read = true;
            self.dirty = true;
        }
        if self.needs_read {
            self.read_stage(tick, !self.settling);
            if !self.settling {
                self.step = None;
            }
        }
        if self
            .path
            .sequences
            .get(self.seq_index)
            .is_some_and(|seq| seq.terminal && seq.steps.is_empty())
        {
            self.publish(tick.output);
            return Ok(ScriptFlow::Complete);
        }
        if self.settling {
            if self.needs_read {
                self.read_stage(tick, false);
            }
            let truth = {
                let mut required_after = tick.cx.evidence();
                required_after.sequence = self.chat_since;
                let pred = PredicateContext {
                    cx: &tick.cx,
                    quests: &self.quests,
                    progress: &[],
                    required_after,
                    outcome: None,
                };
                self.current_step()
                    .map(|step| step.settle.evaluate(&pred))
                    .unwrap_or(Truth::False)
            };
            if truth == Truth::True {
                self.settling = false;
                self.settle_ticks = 0;
                self.fail_streak = 0;
                if let Some(stage) = self.stage.as_ref() {
                    self.seq_index =
                        sequence_for_stage(&self.path, stage.0.as_ref()).unwrap_or(self.seq_index);
                    self.step_index = 0;
                }
                self.on_step_boundary(tick);
            } else {
                self.settle_ticks = self.settle_ticks.saturating_add(1);
                if self.settle_ticks >= 40 {
                    self.settling = false;
                    self.fail_streak = self.fail_streak.saturating_add(1);
                    if self.fail_streak >= 5 {
                        self.parked = true;
                    }
                    self.on_step_boundary(tick);
                }
            }
            self.publish(tick.output);
            return Ok(ScriptFlow::Continue);
        }
        if self.step.is_none() {
            let _ = self.provisioner.check();
            let selected = {
                let pred = PredicateContext {
                    cx: &tick.cx,
                    quests: &self.quests,
                    progress: &[],
                    required_after: tick.cx.evidence(),
                    outcome: None,
                };
                select(&self.path, self.seq_index, &pred).map(|sel| {
                    (
                        sel.index,
                        sel.step.advances,
                        sel.prelude,
                        sel.step as *const _,
                    )
                })
            };
            let Some((index, advances, _prelude, ptr)) = selected else {
                self.needs_read = true;
                self.publish(tick.output);
                return Ok(ScriptFlow::Continue);
            };
            self.step_index = index;
            self.advances = advances;
            self.in_prelude = _prelude;
            let step = if _prelude {
                &self.path.prelude[index]
            } else {
                &self.path.sequences[self.seq_index].steps[index]
            };
            debug_assert!(std::ptr::eq(step as *const _, ptr));
            self.chat_since = super::families::reach::last_chat_seq(&tick.cx) as u64;
            let evidence: Arc<dyn EvidenceProvider> = Arc::clone(&self.evidence) as _;
            let required_after = tick.cx.evidence();
            let mut step_cx = StepContext {
                tick,
                quests: &self.quests,
                progress: &[],
                required_after,
                walk_evidence: &evidence,
            };
            match step.plan.begin(&mut step_cx) {
                Ok(run) => {
                    self.step = Some(run);
                    self.dirty = true;
                }
                Err(_) => {
                    self.attempts = self.attempts.saturating_add(1);
                    if self.attempts >= 5 {
                        self.parked = true;
                    }
                }
            }
            self.publish(tick.output);
            return Ok(ScriptFlow::Continue);
        }
        let evidence: Arc<dyn EvidenceProvider> = Arc::clone(&self.evidence) as _;
        let required_after = tick.cx.evidence();
        let poll = {
            let mut step_cx = StepContext {
                tick,
                quests: &self.quests,
                progress: &[],
                required_after,
                walk_evidence: &evidence,
            };
            self.step
                .as_mut()
                .map(|step| step.poll(&mut step_cx))
                .unwrap_or(Poll::Pending)
        };
        match poll {
            Poll::Pending => {}
            Poll::Ready(Ok(_)) => {
                if self.advances {
                    self.needs_read = true;
                }
                self.step = None;
                self.settling = true;
                self.settle_ticks = 0;
            }
            Poll::Ready(Err(_)) => {
                self.step = None;
                self.fail_streak = self.fail_streak.saturating_add(1);
                if self.fail_streak >= 5 {
                    self.parked = true;
                }
                self.on_step_boundary(tick);
            }
        }
        self.publish(tick.output);
        Ok(ScriptFlow::Continue)
    }

    fn interrupt(&mut self, event: Interrupt) {
        match event {
            Interrupt::Resume | Interrupt::SessionReady => {
                self.needs_read = true;
                self.step = None;
                self.settling = false;
                self.dirty = true;
            }
            Interrupt::SessionEnded => {
                self.step = None;
                self.settling = false;
                self.needs_read = true;
            }
            Interrupt::Pause | Interrupt::Hold(_) => {}
        }
    }

    fn on_stop(&mut self, _reason: StopReason) {
        self.step = None;
        self.settling = false;
        self.dirty = true;
    }

    fn retry(&mut self) -> Result<(), ScriptFailure> {
        self.parked = false;
        self.needs_read = true;
        self.fail_streak = 0;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quester_struct_fits_the_per_bot_budget() {
        let bytes = std::mem::size_of::<Quester>();
        eprintln!("Quester size_of={bytes}");
        assert!(bytes < 4096, "Quester is {bytes} bytes");
    }

    #[test]
    fn message_settle_uses_step_begin_chat_not_tick_sequence() {
        use super::super::families::tests::with_tick;
        use api::snapshot::{ChatLineView, GameSnapshot, QuestStatusView};
        let data = api::game_data::for_revision(api::selected::ClientRevision::R289).unwrap();
        let quests = Arc::new(QuestCatalog::from_identity(data.quest_identity()).unwrap());
        let mut document = super::super::compile::decode_cook().unwrap();
        let step = &mut document.roles[0].sequences[0].steps[0];
        step.kind = "wait".into();
        step.args = serde_json::json!({"max_ticks": 10});
        step.advances = false;
        step.settle = super::super::path::PredicateDocument::Fact {
            kind: "message".into(),
            version: 1,
            args: serde_json::json!({"any": ["you put the grain in the hopper"]}),
        };
        let path =
            super::super::compile::compile_path(&document, b"message-regression", &data, &quests)
                .unwrap();
        let run = RunKey {
            slot: 1,
            run: 1,
            session: 1,
        };
        let mut script = Quester::new(run, path, quests);
        let mut s = GameSnapshot::new();
        s.seed_ingame(2);
        s.seed_quest_statuses(
            vec![QuestStatusView {
                name: "Cook's Assistant".into(),
                component_id: 0,
                colour: 0xff0000,
            }],
            true,
        );
        s.seed_chat_lines(vec![ChatLineView {
            type_: 0,
            username: None,
            text: "you put the grain in the hopper".into(),
            sequence: 2,
        }]);
        let mut ledger = None;
        with_tick(&s, &mut ledger, 5000, |t| script.tick(t).unwrap());
        with_tick(&s, &mut ledger, 5001, |t| script.tick(t).unwrap());
        with_tick(&s, &mut ledger, 5002, |t| script.tick(t).unwrap());
        assert!(script.settling, "old matching chat must not settle");
        s.seed_chat_lines(vec![ChatLineView {
            type_: 0,
            username: None,
            text: "You put the grain in the hopper.".into(),
            sequence: 3,
        }]);
        with_tick(&s, &mut ledger, 5003, |t| script.tick(t).unwrap());
        assert!(
            !script.settling,
            "a new sequence 3 message settles at tick 5003"
        );
    }
}
