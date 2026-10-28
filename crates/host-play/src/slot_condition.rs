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
use api::interact::{Driver, PacketCheckpoint};
use api::quest_progress::EvidenceStamp;
use api::snapshot::{GameSnapshot, SnapshotView};
use client::client::Client;
use client::io::{map_client_prot, ClientProt};
use host::DrainResult;
use script::combat::frame::Frame;
use script::combat::tables::CombatTables;
use script::combat::threats::ThreatSet;

const POISON_LINE: &str = "You have been poisoned!";
const DRINK_LINE: &str = "You drink some of your antipoison potion.";
const FRUIT_LINE: &str = "You eat the fruit.";
// poison.rs2:29,72-75: after the last dispatched interaction, retain 30
// ticks to the fire, one queued-damage tick (World.ts:701-704;
// Player.ts:903-911), and the existing observation margin.
const CLEAN_TICKS: u8 = (30 + 1 + script::combat::risk::consts::LAG) as u8;

// Content operations, uses and interface actions can run or resume a script.
// MoveClickHandler.ts:11-56 only rejects a delay or changes the path.
const INTERACTIONS: &[ClientProt] = &[
    ClientProt::OPOBJ1,
    ClientProt::OPOBJ2,
    ClientProt::OPOBJ3,
    ClientProt::OPOBJ4,
    ClientProt::OPOBJ5,
    ClientProt::OPOBJT,
    ClientProt::OPOBJU,
    ClientProt::OPNPC1,
    ClientProt::OPNPC2,
    ClientProt::OPNPC3,
    ClientProt::OPNPC4,
    ClientProt::OPNPC5,
    ClientProt::OPNPCT,
    ClientProt::OPNPCU,
    ClientProt::OPLOC1,
    ClientProt::OPLOC2,
    ClientProt::OPLOC3,
    ClientProt::OPLOC4,
    ClientProt::OPLOC5,
    ClientProt::OPLOCT,
    ClientProt::OPLOCU,
    ClientProt::OPPLAYER1,
    ClientProt::OPPLAYER2,
    ClientProt::OPPLAYER3,
    ClientProt::OPPLAYER4,
    ClientProt::OPPLAYER5,
    ClientProt::OPPLAYERT,
    ClientProt::OPPLAYERU,
    ClientProt::OPHELD1,
    ClientProt::OPHELD2,
    ClientProt::OPHELD3,
    ClientProt::OPHELD4,
    ClientProt::OPHELD5,
    ClientProt::OPHELDT,
    ClientProt::OPHELDU,
    ClientProt::INV_BUTTON1,
    ClientProt::INV_BUTTON2,
    ClientProt::INV_BUTTON3,
    ClientProt::INV_BUTTON4,
    ClientProt::INV_BUTTON5,
    ClientProt::INV_BUTTOND,
    ClientProt::IF_BUTTON,
    ClientProt::RESUME_PAUSEBUTTON,
    ClientProt::RESUME_P_COUNTDIALOG,
    ClientProt::CLOSE_MODAL,
    ClientProt::TUT_CLICKSIDE,
];

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

#[derive(Clone, Copy)]
enum PendingFruit {
    None,
    Chat {
        tick: u16,
    },
    CountLoss {
        tick: u16,
    },
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
    pending_fruit: PendingFruit,
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
            pending_fruit: PendingFruit::None,
            window: SettleWindow::default(),
            q_unknown: u8::MAX,
            complete: false,
            warned: 0,
        }
    }
}

impl SlotCondition {
    pub(crate) fn session_started(
        &mut self,
        tick: u32,
        chat_sequence: i32,
        tables: Option<(Arc<CombatTables>, u8)>,
    ) {
        *self = Self::default();
        self.last_tick = tick;
        // Reconnect retains the client's ring. Ignore every pre-boundary line,
        // including lines the old snapshot had not rebuilt or consumed.
        self.chat_seen = chat_sequence;
        self.chat_visible = chat_sequence;
        self.memory = self
            .memory
            .transition(PoisonEvent::Reconnect { tick: tick as u16 });
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
        self.pending_fruit = PendingFruit::None;
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

    /// Restart only the post-login window, never an established poison fact.
    fn accepted_click(&mut self) {
        if matches!(self.memory.state, PoisonState::Unknown { .. }) {
            self.window.clean = 0;
            self.window.accepted = true;
        }
    }

    pub(crate) fn dispatch_checkpoint(&self, client: &Client) -> Option<PacketCheckpoint> {
        (client.ingame && matches!(self.memory.state, PoisonState::Unknown { .. }))
            .then(|| PacketCheckpoint::capture(client))
    }

    /// Observe actual packets queued by the slot's host dispatch, not requests
    /// that preconditions may refuse or resolve to movement alone.
    pub(crate) fn observe_dispatch(&mut self, client: &Client, checkpoint: PacketCheckpoint) {
        let revision = client.revision();
        let mut interaction = false;
        let mut movement = false;
        let complete = client.trace_packets(checkpoint, &mut |opcode| {
            interaction |= INTERACTIONS
                .iter()
                .any(|&packet| map_client_prot(revision, packet).id == i32::from(opcode));
            movement |= [
                ClientProt::MOVE_GAMECLICK,
                ClientProt::MOVE_MINIMAPCLICK,
                ClientProt::MOVE_OPCLICK,
            ]
            .iter()
            .any(|&packet| map_client_prot(revision, packet).id == i32::from(opcode));
        });
        if !complete {
            // An incomplete trace cannot prove that the run stayed clean.
            self.window = SettleWindow::default();
        } else if interaction {
            self.accepted_click();
        } else if movement {
            self.window.accepted = true;
        }
    }

    fn transition(&mut self, event: PoisonEvent) {
        self.memory = self.memory.transition(event);
    }

    // strange_fruit.rs2:6-12 removes the fruit and sends its line in one
    // server tick. The row has no p_delay (consume_effects.rs2:166-168,
    // consume_messages.dbrow:713-719). World.ts:1108-1116 sends PLAYER_INFO
    // before inventory. NetworkPlayer.ts:191-227 writes each packet at once.
    // A drain can hold multiple PLAYER_INFO packets, so pair signals from this
    // or the immediately previous observed edge, not by a fixed tick delta.
    fn fruit_origin(
        &mut self,
        chat: bool,
        count_loss: bool,
        previous_tick: u16,
        now: u16,
    ) -> Option<u16> {
        let pending = self.pending_fruit;
        let pending_tick = match pending {
            PendingFruit::None => None,
            PendingFruit::Chat { tick } | PendingFruit::CountLoss { tick } => Some(tick),
        };
        let pending = pending_tick
            .filter(|tick| *tick == previous_tick || *tick == now)
            .map(|_| pending);
        if pending.is_none() {
            self.pending_fruit = PendingFruit::None;
        }

        match (pending, chat, count_loss) {
            (Some(PendingFruit::Chat { tick }), false, true)
            | (Some(PendingFruit::CountLoss { tick }), true, false) => {
                self.pending_fruit = PendingFruit::None;
                Some(tick)
            }
            (_, true, true) => {
                self.pending_fruit = PendingFruit::None;
                Some(now)
            }
            (_, true, false) => {
                self.pending_fruit = PendingFruit::Chat { tick: now };
                None
            }
            (_, false, true) => {
                self.pending_fruit = PendingFruit::CountLoss { tick: now };
                None
            }
            _ => None,
        }
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
        if self.tables.is_none() {
            // Without selected content, zero food/cures would falsely mean
            // "none held". An unpublished fact keeps both summaries unknown.
            self.complete = false;
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
        let inv_gen = snapshot
            .inventory_packet_generation()
            .expect("validated inventory packet");
        let derived = if self.inv_gen != Some(inv_gen) {
            match script::survival::condition::derive(
                snapshot,
                self.tables.as_deref().expect("validated selected tables"),
            ) {
                Ok(derived) => Some(derived),
                Err(error) => {
                    self.incomplete(error);
                    return;
                }
            }
        } else {
            None
        };
        let tick = snapshot.tick();
        let previous_tick = self.last_tick as u16;
        let now = tick as u16;
        let k = tick.saturating_sub(self.last_tick);
        let prev_complete = self.complete;
        self.last_tick = tick;
        let fruit_count_loss =
            derived.is_some_and(|fact| fact.cures.strange_fruit < self.fact.cures.strange_fruit);
        let mut poison_signal = false;
        let mut hits = [None; 4];
        let mut hit_at_edge = false;
        if let Some(origin) = self.fruit_origin(false, fruit_count_loss, previous_tick, now) {
            // strange_fruit.rs2:10 sets %poison to min(%poison, -1). Its
            // negative timer proves that no future immunity fire can occur,
            // so the fruit earns zero lower-bound immunity credit.
            self.transition(PoisonEvent::ProvenFruit { origin });
            self.latch = None;
        }
        if drain.player_info {
            // Close only a latch opened by an earlier edge, before new evidence.
            if self.latch.take().is_some() {
                self.transition(PoisonEvent::LatchComplete { q: self.q_unknown });
            }
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
            for line in snapshot.chat_lines().iter().rev().filter(|line| {
                line.sequence > previous_seen && line.type_ == 0 && line.username.is_none()
            }) {
                if line.text == POISON_LINE {
                    if matches!(self.pending_fruit, PendingFruit::Chat { .. }) {
                        self.pending_fruit = PendingFruit::None;
                    }
                    poison_signal = true;
                    self.transition(PoisonEvent::PoisonChat { tick: now });
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
                } else if line.text == FRUIT_LINE {
                    if let Some(origin) = self.fruit_origin(true, false, previous_tick, now) {
                        // strange_fruit.rs2:10 sets %poison to min(%poison, -1).
                        // That negative timer proves no future immunity fire, so
                        // fruit grants zero lower-bound immunity credit.
                        self.transition(PoisonEvent::ProvenFruit { origin });
                        self.latch = None;
                    }
                }
            }
            self.chat_seen = snapshot
                .chat_lines()
                .first()
                .map_or(self.chat_seen, |line| line.sequence.max(self.chat_seen));
        }
        if drain.player_info {
            // Kind 2 reveals the fire's value. A same-edge ordinary hit may
            // follow it server-side, so process it afterward and fail closed.
            for poison in [true, false] {
                for &(kind, value) in hits.iter().flatten() {
                    if (kind == 2) != poison {
                        continue;
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
                // The final possible mark tick stays Unknown. Clear only on
                // the following edge, after the inclusive observation bound.
                if self.window.clean > CLEAN_TICKS
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
        if let Some(derived) = derived {
            self.fact.food = derived.food;
            self.fact.cures = derived.cures;
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
