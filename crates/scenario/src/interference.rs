//! Opt-in measured windows over existing, immutable snapshot observations.

use api::snapshot::{ActorKind, ActorTargetView, GameSnapshot, HitmarksView};
use serde::Serialize;

/// Combat expected by the cell, never inferred from a step name or death message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CombatExpectation {
    /// Combat, damage, targeting and death invalidate this window.
    Quiet,
    /// Exempt all combat only if HP zero is observed before this window closes.
    /// Keep it short: neither hitmarks nor face targets identify the damage source.
    LabelledDeath { label: &'static str },
}

/// Scalar evidence from the snapshot the runner already owns. No new client read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct CombatSample {
    pub session: u64,
    pub tick: u32,
    pub ingame: bool,
    pub scene: i32,
    pub tile: Option<[i32; 3]>,
    pub hitpoints: Option<i32>,
    pub in_combat: bool,
    pub target: Option<ActorTargetView>,
    pub hitmark_cycle: Option<i32>,
    pub hitmarks: Option<HitmarksView>,
}

impl CombatSample {
    fn read(snapshot: &GameSnapshot, session: u64) -> Self {
        let actor = snapshot.local_player().map(|local| &local.player.actor);
        Self {
            session,
            tick: snapshot.tick(),
            ingame: snapshot.ingame(),
            scene: snapshot.scene_state(),
            tile: snapshot.tile().map(|(x, z, level)| [x, z, level]),
            hitpoints: snapshot
                .stats()
                .iter()
                .find(|stat| stat.index == 3)
                .map(|stat| stat.effective),
            in_combat: actor.is_some_and(|actor| actor.in_combat),
            target: actor.and_then(|actor| actor.target),
            hitmark_cycle: snapshot
                .hitmarks()
                .and_then(|hits| hits.marks.iter().map(|hit| hit.cycle).max()),
            hitmarks: snapshot.hitmarks().copied(),
        }
    }

    fn combat_reason(self, previous: Option<Self>) -> Option<&'static str> {
        if self.hitpoints == Some(0) {
            return Some("death");
        }
        if previous.is_some_and(|old| {
            old.session == self.session
                && matches!((old.hitpoints, self.hitpoints), (Some(a), Some(b)) if b < a)
        }) {
            return Some("hitpoints-decreased");
        }
        if previous.is_some_and(|old| {
            old.session == self.session
                && matches!((old.hitmark_cycle, self.hitmark_cycle), (Some(a), Some(b)) if b > a)
        }) {
            return Some("fresh-hitmark");
        }
        if self.in_combat {
            return Some("combat-active");
        }
        self.target.map(|_| "target-active")
    }

    fn changed(self, old: Self) -> bool {
        self.session != old.session
            || self.ingame != old.ingame
            || self.scene != old.scene
            || self.hitpoints != old.hitpoints
            || self.in_combat != old.in_combat
            || self.target != old.target
            || self.hitmark_cycle != old.hitmark_cycle
            || self.hitmarks.map(|hits| hits.marks) != old.hitmarks.map(|hits| hits.marks)
    }
}

/// One named interval, including its actual terminal boundary when interrupted.
#[derive(Debug, Clone, Serialize)]
pub struct MeasuredWindow {
    pub name: &'static str,
    pub expectation: CombatExpectation,
    pub start: CombatSample,
    pub end: Option<CombatSample>,
    pub end_reason: Option<&'static str>,
    pub death_observed: bool,
    pub hit_count: usize,
    pub valid: bool,
}

/// A combat change or session/scene transit, retained even outside a window.
#[derive(Debug, Clone, Serialize)]
pub struct InterferenceEvent {
    pub sample: CombatSample,
    pub previous: Option<CombatSample>,
    pub window: Option<&'static str>,
    /// `transit`, `interference`, `labelled_death`, or `observation`.
    pub classification: &'static str,
    pub reason: &'static str,
    pub transit: bool,
    pub label: Option<&'static str>,
    /// Nearby NPC slots facing this account at this change, not proven attackers.
    pub targeting_npcs: Option<Vec<usize>>,
    pub targeting_npcs_truncated: bool,
}

/// Bounded, typed records for later journal adapters. Overflow is non-pass.
#[derive(Debug, Clone, Default, Serialize)]
pub struct InterferenceEvidence {
    pub windows: Vec<MeasuredWindow>,
    pub events: Vec<InterferenceEvent>,
    pub dropped_events: u64,
    pub invalid_reason: Option<&'static str>,
}

/// Optional owner. Ordinary scenarios keep `None`, with no per-tick allocation.
#[derive(Debug, Default)]
pub struct CombatObserver {
    evidence: InterferenceEvidence,
    previous: Option<CombatSample>,
    active: Option<usize>,
}

impl CombatObserver {
    /// Observe only when explicitly armed. The provider has no sender or mutable client.
    pub fn observe(observer: Option<&mut Self>, snapshot: &GameSnapshot, session: u64) {
        if let Some(observer) = observer {
            let before = observer.evidence.events.len();
            observer.sample(CombatSample::read(snapshot, session));
            if observer.evidence.events.len() != before {
                if let Some(local) = snapshot.local_player() {
                    let target = ActorTargetView {
                        kind: ActorKind::Player,
                        index: local.player.index,
                    };
                    let event = observer.evidence.events.last_mut().expect("new event");
                    let targets = event.targeting_npcs.insert(Vec::new());
                    for npc in snapshot
                        .npcs()
                        .iter()
                        .filter(|npc| npc.target == Some(target))
                    {
                        if targets.len() == crate::evidence::MAX_RECEIPT_EVENTS {
                            event.targeting_npcs_truncated = true;
                            break;
                        }
                        targets.push(npc.index);
                    }
                }
            }
        }
    }

    fn invalidate(&mut self, reason: &'static str) {
        self.evidence.invalid_reason.get_or_insert(reason);
        if let Some(index) = self.active {
            self.evidence.windows[index].valid = false;
        }
    }

    fn push(&mut self, event: InterferenceEvent) {
        if self.evidence.events.len() < crate::evidence::MAX_RECEIPT_EVENTS {
            self.evidence.events.push(event);
        } else {
            self.evidence.dropped_events = self.evidence.dropped_events.saturating_add(1);
            self.invalidate("combat-evidence-overflow");
        }
    }

    fn sample(&mut self, sample: CombatSample) {
        let previous = self.previous.replace(sample);
        if previous.is_some_and(|old| !sample.changed(old)) {
            return;
        }
        let transit = previous.is_some_and(|old| {
            old.session != sample.session
                || old.ingame != sample.ingame
                || old.scene != sample.scene
        });
        if let Some(index) = self.active {
            let window = &mut self.evidence.windows[index];
            window.death_observed |= sample.hitpoints == Some(0);
            if let Some(hits) = sample.hitmarks {
                window.hit_count += hits
                    .marks
                    .iter()
                    .enumerate()
                    .filter(|(index, hit)| {
                        hit.cycle > hits.loop_cycle
                            && !previous
                                .filter(|old| old.session == sample.session)
                                .and_then(|old| old.hitmarks)
                                .is_some_and(|old| old.marks[*index] == **hit)
                    })
                    .count();
            }
        }
        let reason = sample.combat_reason(previous);
        let window = self.active.map(|index| &self.evidence.windows[index]);
        let label = window.and_then(|window| match window.expectation {
            CombatExpectation::LabelledDeath { label } => Some(label),
            CombatExpectation::Quiet => None,
        });
        let name = window.map(|window| window.name);
        let classification = if reason.is_some() {
            if label.is_some() {
                "labelled_death"
            } else if name.is_some() {
                "interference"
            } else {
                "transit"
            }
        } else {
            "observation"
        };
        self.push(InterferenceEvent {
            sample,
            previous,
            window: name,
            classification,
            reason: reason.unwrap_or(if transit {
                "session-or-scene-transit"
            } else {
                "combat-state"
            }),
            transit,
            label,
            targeting_npcs: None,
            targeting_npcs_truncated: false,
        });
        if classification == "interference" {
            self.invalidate(reason.expect("interference has a combat reason"));
        }
        if name.is_some() && previous.is_some_and(|old| old.session != sample.session) {
            self.invalidate("measured-session-changed");
        }
    }

    pub(crate) fn begin(
        &mut self,
        name: &'static str,
        expectation: CombatExpectation,
    ) -> Result<(), &'static str> {
        if name.is_empty()
            || self.active.is_some()
            || self
                .evidence
                .windows
                .iter()
                .any(|window| window.name == name)
            || matches!(expectation, CombatExpectation::LabelledDeath { label: "" })
        {
            return Err("invalid measured-window Begin or duplicate name");
        }
        let start = self
            .previous
            .ok_or("measured-window Begin has no observation")?;
        if !start.ingame || start.scene != 2 || start.hitpoints.is_none() || start.tile.is_none() {
            return Err("measured-window Begin has no ready combat observation");
        }
        if self.evidence.windows.len() == crate::evidence::MAX_RECEIPT_EVENTS {
            return Err("measured-window limit exceeded");
        }
        self.active = Some(self.evidence.windows.len());
        self.evidence.windows.push(MeasuredWindow {
            name,
            expectation,
            start,
            end: None,
            end_reason: None,
            death_observed: false,
            hit_count: 0,
            valid: true,
        });
        // A fight that started in transit still overlaps the newly measured interval.
        self.previous = None;
        self.sample(start);
        Ok(())
    }

    pub(crate) fn end(&mut self, name: &'static str) -> Result<(), &'static str> {
        let index = self.active.ok_or("measured-window End without Begin")?;
        if self.evidence.windows[index].name != name {
            return Err("measured-window End does not match Begin");
        }
        self.close("end");
        Ok(())
    }

    fn close(&mut self, reason: &'static str) {
        if let Some(index) = self.active {
            let window = &mut self.evidence.windows[index];
            window.end = self.previous;
            window.end_reason = Some(reason);
            window.valid &= reason == "end";
            let mut unexempted = None;
            if matches!(window.expectation, CombatExpectation::LabelledDeath { .. })
                && !window.death_observed
            {
                for event in &mut self.evidence.events {
                    if event.window == Some(window.name) && event.classification == "labelled_death"
                    {
                        event.classification = "interference";
                        unexempted.get_or_insert(event.reason);
                    }
                }
            }
            if let Some(reason) = unexempted {
                self.invalidate(reason);
            }
            self.active = None;
        }
    }

    /// Close a cut-short window without confusing an incomplete proof with interference.
    pub(crate) fn terminal(&mut self) -> bool {
        let open = self.active.is_some();
        self.close("terminal");
        open
    }

    pub(crate) fn invalid_reason(&self) -> Option<&'static str> {
        self.evidence.invalid_reason
    }

    pub(crate) fn into_evidence(self) -> InterferenceEvidence {
        self.evidence
    }
}

#[cfg(test)]
#[path = "interference_tests.rs"]
mod tests;
