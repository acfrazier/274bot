//! One slot-thread owner for the account's observed condition.
//!
//! Poison evidence is consumed only at PLAYER_INFO edges. NPC_INFO can
//! complete a proven line's source later in the same observation window.

use std::sync::Arc;

use api::condition::{
    BotCondition, ConditionError, Exposure, PoisonEvent, PoisonMemory, PoisonState, SessionMark,
};
use api::host_log;
use api::hostlog::{Category, Level};
use api::quest_progress::EvidenceStamp;
use api::snapshot::{GameSnapshot, SnapshotView};
use host::DrainResult;
use script::combat::frame::Frame;
use script::combat::tables::CombatTables;
use script::combat::threats::ThreatSet;

const POISON_LINE: &str = "You have been poisoned!";
const DRINK_LINE: &str = "You drink some of your antipoison potion.";
const FRUIT_LINE: &str = "You eat the fruit.";
// content/scripts/skill_combat/scripts/poison.rs2:72-75: normal timer, 30 ticks.
const CLEAN_TICKS: u8 = 30;

#[derive(Default)]
struct SettleWindow {
    clean: u8,
    accepted: bool,
}

/// Evidence retained from the edge, not from a later consumer's threat set.
struct PendingLatch {
    since: u16,
    hit_at_edge: bool,
}

pub(crate) struct SlotCondition {
    fact: BotCondition,
    memory: PoisonMemory,
    tables: Option<Arc<CombatTables>>,
    inv_gen: Option<u64>,
    last_tick: u32,
    chat_seen: i32,
    chat_visible: i32,
    hit_seen: [i32; 4],
    latch: Option<PendingLatch>,
    window: SettleWindow,
    q_unknown: u8,
    complete: bool,
    warned: u8,
}

const _: () = {
    assert!(std::mem::size_of::<SlotCondition>() == 104);
    assert!(std::mem::size_of::<SlotCondition>() <= 160);
};

impl Default for SlotCondition {
    fn default() -> Self {
        Self {
            fact: BotCondition::default(),
            memory: PoisonMemory::default(),
            tables: None,
            inv_gen: None,
            last_tick: 0,
            chat_seen: 0,
            chat_visible: 0,
            hit_seen: [-1; 4],
            latch: None,
            window: SettleWindow::default(),
            q_unknown: u8::MAX,
            complete: false,
            warned: 0,
        }
    }
}

impl SlotCondition {
    pub(crate) fn session_started(&mut self, tick: u32, tables: Option<(Arc<CombatTables>, u8)>) {
        *self = Self::default();
        self.last_tick = tick;
        self.memory = self
            .memory
            .transition(PoisonEvent::Reconnect { tick: tick as u16 }, None);
        self.fact.poison = self.memory.state;
        if let Some((tables, q)) = tables {
            self.tables = Some(tables);
            self.q_unknown = q;
        }
    }

    pub(crate) fn session_ended(&mut self) {
        self.fact.session = SessionMark::Boundary;
        self.complete = false;
        self.latch = None;
    }

    pub(crate) fn current(&self) -> Option<&BotCondition> {
        (self.complete && self.fact.session == SessionMark::Live).then_some(&self.fact)
    }

    pub(crate) fn published(&self) -> BotCondition {
        let mut fact = self.fact;
        if self.current().is_none() {
            fact.session = SessionMark::Boundary;
        }
        fact
    }

    pub(crate) fn unknown_pulse(&self) -> u8 {
        self.q_unknown
    }

    /// A host-dispatched click inside the current clean window. A packet send
    /// alone does not prove a drink and never clears known poison.
    pub(crate) fn accepted_click(&mut self) {
        self.window.accepted = true;
    }

    fn transition(&mut self, event: PoisonEvent) {
        self.memory = self.memory.transition(event, None);
    }

    fn incomplete(&mut self, error: ConditionError) {
        self.complete = false;
        let bit = match error {
            ConditionError::NoLocalPlayer => 1,
            ConditionError::StatsUnavailable => 2,
            ConditionError::InventoryUnavailable => 4,
        };
        if self.warned & bit == 0 {
            self.warned |= bit;
            host_log!(
                Category::NavEvent,
                Level::Warn,
                "condition unavailable: {error}"
            );
        }
    }

    pub(crate) fn observe_frame(
        &mut self,
        snapshot: &GameSnapshot,
        drain: &DrainResult,
        guardian_hold: bool,
    ) {
        if !snapshot.ingame() {
            self.session_ended();
            return;
        }
        if snapshot.local_player().is_none() {
            self.incomplete(ConditionError::NoLocalPlayer);
            return;
        }
        let mut hp_rows = snapshot.stats().iter().filter(|stat| stat.index == 3);
        let Some(hp) = hp_rows.next() else {
            self.incomplete(ConditionError::StatsUnavailable);
            return;
        };
        if hp_rows.next().is_some()
            || !hp.used
            || u8::try_from(hp.effective).is_err()
            || u8::try_from(hp.base).is_err()
        {
            self.incomplete(ConditionError::StatsUnavailable);
            return;
        }
        if snapshot.inventory_size() <= 0 || snapshot.inventory_packet_generation().is_none() {
            self.incomplete(ConditionError::InventoryUnavailable);
            return;
        }
        let tick = snapshot.tick();
        let now = tick as u16;
        let k = tick.saturating_sub(self.last_tick);
        let prev_complete = self.complete;
        self.last_tick = tick;
        let mut poison_signal = false;
        if drain.player_info {
            // Close only a latch opened by an earlier edge, before new evidence.
            if self.latch.take().is_some() {
                self.transition(PoisonEvent::LatchComplete { q: self.q_unknown });
            }
            let mut hits = [None; 4];
            let mut hit_at_edge = false;
            if let Some(marks) = snapshot.hitmarks() {
                for (slot, mark) in marks.marks.iter().enumerate() {
                    if mark.cycle <= marks.loop_cycle || mark.cycle == self.hit_seen[slot] {
                        continue;
                    }
                    self.hit_seen[slot] = mark.cycle;
                    hit_at_edge |= matches!(mark.kind, 0 | 1);
                    poison_signal |= mark.kind == 2;
                    hits[slot] = Some((mark.kind, mark.value));
                }
            }
            // The ring is newest first. Every new line has the same conservative
            // origin, but its oldest poison/drink ordering still matters.
            let previous_seen = self.chat_seen;
            for line in snapshot
                .chat_lines()
                .iter()
                .rev()
                .filter(|line| line.sequence > previous_seen)
            {
                if line.text == POISON_LINE {
                    poison_signal = true;
                    self.transition(PoisonEvent::PoisonChat { tick: now });
                    if let PoisonState::Onset {
                        q, latched: false, ..
                    } = &mut self.memory.state
                    {
                        *q = self.q_unknown;
                    }
                    let proven = line.sequence <= self.chat_visible || !drain.npc_rows_coherent;
                    let can_latch =
                        proven && k == 1 && prev_complete && self.tables.is_some() && hit_at_edge;
                    if can_latch && matches!(self.memory.state, PoisonState::Onset { .. }) {
                        self.latch = Some(PendingLatch {
                            since: now,
                            hit_at_edge,
                        });
                    } else {
                        self.transition(PoisonEvent::LatchComplete { q: self.q_unknown });
                    }
                } else if line.text == DRINK_LINE {
                    // anti_poison.rs2:5-19, poison.rs2:25-28: both families
                    // prove at least ordinary immunity. Backlogs use the first tick.
                    let origin = tick.saturating_sub(k.max(1).saturating_sub(1)) as u16;
                    self.transition(PoisonEvent::ProvenDrink { origin });
                    self.latch = None;
                } else if line.text == FRUIT_LINE
                    && snapshot
                        .inventory()
                        .iter()
                        .filter(|item| item.def.id == 464)
                        .map(|item| item.count)
                        .sum::<i32>()
                        < i32::from(self.fact.cures.strange_fruit)
                {
                    // strange_fruit.rs2:6-12 and consume_messages.dbrow:713-718.
                    // The line must accompany an observed fruit loss. Its single
                    // negative timer value proves no future immunity fires.
                    let origin = tick.saturating_sub(k.max(1).saturating_sub(1)) as u16;
                    self.transition(PoisonEvent::ProvenFruit { origin });
                    self.latch = None;
                }
            }
            self.chat_seen = snapshot
                .chat_lines()
                .first()
                .map_or(self.chat_seen, |line| line.sequence.max(self.chat_seen));
            // Kind 2 reveals the fire's value. A same-edge ordinary hit may
            // follow it server-side, so process it afterward and fail closed.
            for poison in [true, false] {
                for &(kind, value) in hits.iter().flatten() {
                    if (kind == 2) != poison {
                        continue;
                    }
                    if kind == 2 && matches!(self.memory.state, PoisonState::Immune { .. }) {
                        host_log!(
                            Category::NavEvent,
                            Level::Warn,
                            "poison mark contradicts credited immunity"
                        );
                    }
                    self.transition(PoisonEvent::Hitmark {
                        kind: kind.clamp(0, u16::MAX as i32) as u16,
                        value: value.clamp(0, u8::MAX as i32) as u8,
                        tick: now,
                    });
                    if kind == 2 || kind >= 3 {
                        self.latch = None;
                    }
                }
            }
            self.transition(PoisonEvent::Tick { tick: now });
        }
        if drain.npc_rows_coherent {
            if let Some(latch) = self.latch.take() {
                let q = if latch.since == now && latch.hit_at_edge {
                    self.line_pulse(snapshot)
                } else {
                    self.q_unknown
                };
                self.transition(PoisonEvent::LatchComplete { q });
            }
        }
        if snapshot.modals().main != -1 || snapshot.modals().chat != -1 || guardian_hold {
            self.window = SettleWindow::default();
        }
        if matches!(self.memory.state, PoisonState::Unknown { .. }) && drain.player_info {
            if k == 1
                && prev_complete
                && snapshot.modals().main == -1
                && snapshot.modals().chat == -1
                && !guardian_hold
                && !poison_signal
            {
                self.window.clean = self.window.clean.saturating_add(1);
                if self.window.clean >= CLEAN_TICKS
                    && self.window.accepted
                    && !self.memory.session_invalid
                {
                    self.memory.state = PoisonState::Clear;
                }
            } else {
                self.window = SettleWindow::default();
            }
        } else if !matches!(self.memory.state, PoisonState::Unknown { .. }) {
            self.window = SettleWindow::default();
        }
        if hp.effective <= 0 {
            self.transition(PoisonEvent::DeathObserved);
            self.fact.session = SessionMark::Dead;
        } else {
            self.transition(PoisonEvent::RespawnObserved);
            self.fact.session = SessionMark::Live;
        }
        self.fact.hp = hp.effective.clamp(0, u8::MAX as i32) as u8;
        self.fact.hp_max = hp.base.clamp(0, u8::MAX as i32) as u8;
        self.fact.hp_known = true;
        let inv_gen = snapshot
            .inventory_packet_generation()
            .expect("validated inventory packet");
        if self.inv_gen != Some(inv_gen) {
            if let Some(tables) = self.tables.as_deref() {
                match script::survival::condition::derive(snapshot, tables) {
                    Ok(derived) => {
                        self.fact.food = derived.food;
                        self.fact.cures = derived.cures;
                    }
                    Err(error) => {
                        self.incomplete(error);
                        return;
                    }
                }
            }
            self.inv_gen = Some(inv_gen);
        }
        self.fact.poison = self.memory.state;
        self.fact.tick = tick;
        self.complete = true;
        self.chat_visible = snapshot
            .chat_lines()
            .first()
            .map_or(self.chat_visible, |line| line.sequence);
    }

    fn line_pulse(&self, snapshot: &GameSnapshot) -> u8 {
        let Some(tables) = self.tables.as_deref() else {
            return self.q_unknown;
        };
        let stamp = EvidenceStamp {
            run: api::selected::RunKey {
                slot: 0,
                run: 0,
                session: 0,
            },
            tick: u64::from(snapshot.tick()),
            sequence: u64::from(snapshot.tick()),
        };
        let Some(frame) = Frame::borrow(SnapshotView::new(Some(snapshot), stamp)) else {
            return self.q_unknown;
        };
        let mut threats = ThreatSet::default();
        threats.observe(&frame, tables, frame.tick);
        match script::survival::math::exposure(&threats, tables, frame.tick) {
            Exposure::Attributable { max_pulse } if max_pulse > 0 => max_pulse,
            Exposure::None | Exposure::Attributable { .. } | Exposure::Unknown => self.q_unknown,
        }
    }
}

#[cfg(test)]
#[path = "slot_condition_tests.rs"]
mod tests;
