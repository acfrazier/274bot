//! Live oracle for the random-event Guardian cells (`guardian_event_*`,
//! `guardian_skill_*`).
//!
//! A cell starts a real activity, then summons one random event through the
//! engine's own random-event machinery: the content's `::macro_event` debug
//! cheat for the general events, or the `::skill_macro_event` test cheat
//! (engine patch [`SKILL_EVENT_PATCH`]) that calls the content's own skill
//! spawn procs. The event is therefore a real event owned by the player.
//!
//! The fixture sends only that trigger. Everything after it is observed: the
//! host Guardian's hold (fed by the live pump), the player's tile, HP and
//! chat, the event NPC, and the activity's skill XP. Each cell names one
//! [`Oracle`]: the Guardian's documented behaviour for that event class
//! (`crates/host/src/random.rs`, `crates/host/src/random/guardian.rs` and the
//! Unreleased CHANGELOG "Random-event guardian" entry). Every cell also
//! requires no death, no failed-event teleport, a released hold and fresh
//! activity XP after the event resolved (the run continues).
//!
//! The judgement is a pure function of per-game-tick [`Frame`]s, so the oracle is
//! unit-tested in both directions without a client.

use api::obj_names::ObjNames;
use api::random::{trapped_area, RandomKind};
use api::snapshot::{ActorKind, GameSnapshot};
use serde_json::{json, Value};

/// The engine test patch the skill-event cells need (repo-relative).
pub const SKILL_EVENT_PATCH: &str =
    "tools/engine-patches/289/0002-content-skill-macro-event-cheat.patch";

/// The content's busy reply to a debug cheat (`_test/scripts/engine/debug.rs2`
/// `[label,please_finish]`): the cheat did not run; send it again.
const BUSY_REPLY: &str = "Please finish what you are doing first.";
/// `player/scripts/death.rs2:27`.
pub(crate) const DEATH_MESSAGE: &str = "Oh dear you are dead!";
/// Genie lamp (`macro_genilamp`), the Guardian's `LAMP_OBJ`.
const LAMP_ID: i32 = 2528;
/// Strange box (`macro_cube`), the Guardian's `STRANGE_BOX_OBJ`.
const STRANGE_BOX_ID: i32 = 3062;
/// The vault's default lamp skill (`vault` `default_lamp_skill`).
const STRENGTH_STAT: i32 = 2;
const HITPOINTS_STAT: i32 = 3;

/// Game ticks an installed `::skill_macro_event` takes to answer. The
/// debugproc answers on the game tick it runs (a busy player gets the busy reply
/// instead); silence means the engine has no such debugproc
/// (`ClientCheatHandler.ts` returns without a reply).
const ACK_TICKS: u32 = 10;
/// Game ticks from an accepted send to the event's first observable sign.
/// The general spawner adds the NPC on the same game tick
/// (`macro_events.rs2` `~macro_event_general_spawn`); the old man's trap
/// events teleport after `p_delay(1)`; the strange box arrives after two game ticks.
const ONSET_TICKS: u32 = 20;
/// Busy replies tolerated before the fixture gives up.
const MAX_SENDS: u32 = 20;
/// Game ticks the Guardian has to raise its hold once it can see the event:
/// it acts on the first fresh game tick that detects it (`guardian.rs`
/// `tick_with_live_policies`). Five allows for the NPC packet landing a game tick
/// after the targeting update.
const HOLD_RISE_TICKS: u32 = 5;
/// A dialogue event must be engaged (Talk-to, or the walk to range that
/// precedes it) within this many game ticks of the NPC appearing.
const DIALOG_ENGAGE_TICKS: u32 = 15;
/// The Load script must own the seeded box for this many observed game ticks
/// before native Guardian may claim it.
const STRANGE_BOX_SCRIPT_OWNED_TICKS: u32 = 5;
/// Game-tick deadline for the seeded box to be claimed, consumed and released.
const STRANGE_BOX_HANDOFF_TICKS: u32 = 80;
/// Tiles of path slack before a move toward the attacker's start counts as
/// walking back: a flee route around an obstacle may bend briefly.
const WALK_BACK_SLACK: i32 = 2;
/// Consecutive game ticks an event NPC must be absent before it counts as gone
/// (a view-edge flicker is not a despawn).
const GONE_TICKS: u32 = 3;
/// A tile jump larger than this between two observed game ticks is a teleport.
/// The content's failed-event punishment (`[queue,macro_event_fail_teleport]`)
/// moves the player to a far random location.
const TELEPORT_TILES: i32 = 40;
/// Endure events: game ticks after the onset before the event counts as
/// survived, enough for the gas rock's four rolls and explosion
/// (`macro_event_gas.rs2`) and five whirlpool re-clicks.
const ENDURE_TICKS: u32 = 30;
/// Bounded trace of key events in the receipt.
const MAX_TRACE: usize = 160;

/// How the event appears in the cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Trigger {
    /// `::macro_event <id>` (content `_test` cheat; ids from
    /// `macro_events.constant`: 1 swarm .. 8 maze).
    General(i32),
    /// `::skill_macro_event <id>` (engine patch [`SKILL_EVENT_PATCH`]).
    Skill(i32),
    /// The event item is seeded into the account before the Load script starts.
    Seeded,
}

impl Trigger {
    /// The chat command or seed description for this cell.
    pub fn command(self) -> String {
        match self {
            Self::General(id) => format!("~macro_event {id}"),
            Self::Skill(id) => format!("~skill_macro_event {id}"),
            Self::Seeded => "pre-seeded account state".to_string(),
        }
    }

    /// The patched cheat's answer prefix; `None` for built-in or seeded cases.
    fn ack_prefix(self) -> Option<String> {
        match self {
            Self::General(_) | Self::Seeded => None,
            Self::Skill(id) => Some(format!("skill_macro_event {id}:")),
        }
    }
}

/// What the Guardian is documented to do for the event's class.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Oracle {
    /// Hostile event NPC (`EVADE_NAMES`): the hold rises, the bot flees past
    /// the NPC's content `maxrange + 1` from its start tile, never walks back
    /// toward it while it still targets the bot (hits or misses), and the
    /// hold is released only after the escape (`guardian.rs`
    /// `refresh_evade` / `step_evade`).
    Evade { npc: &'static str },
    /// Talking event (`DIALOG_NAMES`): the Guardian walks to and talks to
    /// the NPC, drains the dialogue, the NPC leaves, the reward arrives.
    Dialog { npc: &'static str, reward: Reward },
    /// Strange plant: probe/Pick until the fruit is held.
    Pick,
    /// Strange box: open and answer the cube until the box is consumed.
    StrangeBox,
    /// The Load script owns the seeded box for an observed window, then
    /// native Guardian claims and solves that same box.
    StrangeBoxHandoff,
    /// Mime or Maze: solve the trap and come back.
    Trap(TrapKind),
    /// Lost pickaxe / axe head: take the head, reattach, re-wield.
    LostTool { tool_ids: &'static [i32] },
    /// Big fish knocks the fishing tool to the ground: take it back.
    LostGear { gear_id: i32 },
    /// No Guardian act is documented for this event (whirlpool, gas rock,
    /// ent): the bot must survive it, keep its tool and keep working.
    Endure { tool_ids: &'static [i32] },
}

/// The dialogue event's content reward.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reward {
    /// Genie: a lamp, then the Guardian's lamp auto rubs it (vault default
    /// on, skill Strength).
    Lamp,
    /// Any of these item ids arrives (drunken dwarf: kebab or beer).
    AnyOf(&'static [i32]),
    /// Some new item arrives (mysterious old man's random present).
    NewItem,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrapKind {
    Mime,
    Maze,
}

impl TrapKind {
    fn kind(self) -> RandomKind {
        match self {
            Self::Mime => RandomKind::Mime,
            Self::Maze => RandomKind::Maze,
        }
    }
}

/// One live cell's event definition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RandomEventCell {
    /// Display name of the event.
    pub event: &'static str,
    pub trigger: Trigger,
    pub oracle: Oracle,
    /// The activity's stat: fresh XP after the event proves the run goes on.
    pub skill: i32,
}

/// The oracle's answer for one observation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    Pending,
    Pass,
    Fail(String),
    /// The engine lacks the test patch the trigger needs.
    Unavailable(String),
}

/// One NPC the oracle cares about (only NPCs named like the event).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FrameNpc {
    pub index: usize,
    pub type_id: Option<usize>,
    pub tile: (i32, i32),
    pub targets_local: bool,
}

/// The per-game-tick facts the oracle judges. Built from the live snapshot by
/// [`Frame::from_snapshot`]; unit tests build them directly.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Frame {
    pub tick: u32,
    pub ready: bool,
    pub tile: Option<(i32, i32, i32)>,
    pub hold: bool,
    pub hp: Option<i32>,
    pub skill_xp: Option<i32>,
    pub strength_xp: Option<i32>,
    /// Newest first, `(sequence, text)`.
    pub chat: Vec<(i32, String)>,
    pub npcs: Vec<FrameNpc>,
    /// Inventory plus equipment, `(id, count)`, summed per id.
    pub held: Vec<(i32, i32)>,
    pub fruit: bool,
    /// A chat continue page or a main/chat modal is open.
    pub modal: bool,
    /// Any NPC targeting the local player, `(name, Chebyshev distance)`, for
    /// HP-drop trace rows.
    pub targeting_npcs: Vec<(String, i32)>,
    /// The NPC the local player itself targets (auto-retaliate), if any.
    pub local_target: Option<String>,
}

impl Frame {
    pub(crate) fn from_snapshot(
        snap: &GameSnapshot,
        cell: &RandomEventCell,
        hold: bool,
        names: Option<&ObjNames>,
    ) -> Self {
        let stat = |id: i32| snap.stats().iter().find(|s| s.index == id);
        let self_slot = snap.self_slot();
        let npc_name = event_npc_name(cell.oracle);
        let npcs = npc_name
            .map(|want| {
                snap.npcs()
                    .iter()
                    .filter(|npc| {
                        npc.name
                            .as_deref()
                            .is_some_and(|name| name.eq_ignore_ascii_case(want))
                    })
                    .map(|npc| FrameNpc {
                        index: npc.index,
                        type_id: npc.r#type,
                        tile: (npc.tile.x, npc.tile.z),
                        targets_local: self_slot >= 0
                            && npc.target.is_some_and(|target| {
                                target.kind == ActorKind::Player
                                    && target.index == self_slot as usize
                            }),
                    })
                    .collect()
            })
            .unwrap_or_default();
        let mut held: Vec<(i32, i32)> = Vec::new();
        for item in snap.inventory().iter().chain(snap.equipment().iter()) {
            match held.iter_mut().find(|(id, _)| *id == item.def.id) {
                Some((_, count)) => *count += item.count,
                None => held.push((item.def.id, item.count)),
            }
        }
        let fruit = matches!(cell.oracle, Oracle::Pick)
            && crate::proof::Proof::Item {
                name: "Strange fruit",
                count: 1,
            }
            .check(snap, names);
        let modals = snap.modals();
        let here = snap.tile();
        let targeting_npcs = snap
            .npcs()
            .iter()
            .filter(|npc| {
                self_slot >= 0
                    && npc.target.is_some_and(|target| {
                        target.kind == ActorKind::Player && target.index == self_slot as usize
                    })
            })
            .map(|npc| {
                let distance = here.map_or(-1, |(x, z, _)| cheb((x, z), (npc.tile.x, npc.tile.z)));
                (npc.name.clone().unwrap_or_default(), distance)
            })
            .collect();
        let local_target = snap
            .local_player()
            .and_then(|local| local.player.actor.target)
            .filter(|target| target.kind == ActorKind::Npc)
            .and_then(|target| snap.npcs().iter().find(|npc| npc.index == target.index))
            .map(|npc| npc.name.clone().unwrap_or_default());
        Self {
            targeting_npcs,
            local_target,
            modal: snap.chat_continue_component_id() != -1
                || modals.main != -1
                || modals.chat != -1,
            tick: snap.tick(),
            ready: snap.ingame() && snap.scene_state() == 2,
            tile: snap.tile(),
            hold,
            hp: stat(HITPOINTS_STAT).map(|s| s.effective),
            skill_xp: stat(cell.skill).map(|s| s.xp),
            strength_xp: stat(STRENGTH_STAT).map(|s| s.xp),
            chat: snap
                .chat_lines()
                .iter()
                .map(|line| (line.sequence, line.text.clone()))
                .collect(),
            npcs,
            held,
            fruit,
        }
    }

    fn count(&self, id: i32) -> i32 {
        self.held
            .iter()
            .filter(|(held, _)| *held == id)
            .map(|(_, count)| *count)
            .sum()
    }

    fn holds_any(&self, ids: &[i32]) -> bool {
        ids.iter().any(|id| self.count(*id) > 0)
    }

    /// Chat lines newer than `seq`, newest first.
    fn chat_after(&self, seq: i32) -> impl Iterator<Item = &(i32, String)> {
        self.chat.iter().take_while(move |(line, _)| *line > seq)
    }

    fn head(&self) -> i32 {
        self.chat.first().map(|(seq, _)| *seq).unwrap_or(0)
    }
}

/// The NPC name an oracle tracks, if it tracks one.
fn event_npc_name(oracle: Oracle) -> Option<&'static str> {
    match oracle {
        Oracle::Evade { npc } | Oracle::Dialog { npc, .. } => Some(npc),
        Oracle::Pick => Some("Strange plant"),
        _ => None,
    }
}

fn cheb(a: (i32, i32), b: (i32, i32)) -> i32 {
    (a.0 - b.0).abs().max((a.1 - b.1).abs())
}

/// The engine's chase stop for an op-trigger NPC: Chebyshev distance from
/// the NPC's start past `maxrange + 1`, or the corner exactly at
/// `maxrange + 1` (`engine/src/engine/entity/Npc.ts:661-670`).
fn outside_chase_range(player: (i32, i32), start: (i32, i32), maxrange: i32) -> bool {
    let dx = (player.0 - start.0).abs();
    let dz = (player.1 - start.1).abs();
    let stop = maxrange + 1;
    dx.max(dz) > stop || (dx == stop && dz == stop)
}

/// The latched hostile NPC.
#[derive(Debug, Clone, Copy)]
struct Attacker {
    index: usize,
    type_id: Option<usize>,
    start: (i32, i32),
    maxrange: Option<i32>,
}

/// One cell's episode state.
pub struct RandomEventEpisode {
    cell: RandomEventCell,
    baseline: Option<Frame>,
    // Trigger.
    sends: u32,
    send_tick: Option<u32>,
    send_seq: i32,
    acked: bool,
    // Ticks.
    last: Option<Frame>,
    onset: Option<u32>,
    hold_rise: Option<u32>,
    hold_fall: Option<u32>,
    hold_ticks: u32,
    min_hp: Option<i32>,
    // Evade.
    attacker: Option<Attacker>,
    first_target: Option<u32>,
    max_distance: i32,
    escape: Option<(u32, i32)>,
    returned: Option<u32>,
    retargeted: bool,
    targeted_ticks: u32,
    // NPC presence (evade / dialog / pick).
    npc_seen: Option<u32>,
    absent_ticks: u32,
    npc_gone: Option<u32>,
    // Items and traps.
    item_seen: Option<u32>,
    item_cleared: Option<u32>,
    /// The first observed frame proving the seeded box stayed script-owned.
    script_owned: Option<u32>,
    /// Whether the box was still present when native Guardian first held.
    box_at_hold: Option<bool>,
    lost: Option<u32>,
    restored: Option<u32>,
    reward: Option<u32>,
    trap_entered: Option<u32>,
    trap_left: Option<u32>,
    trap_hold: bool,
    // Outcome.
    resolved: Option<(u32, i32)>,
    resumed: Option<u32>,
    trace: Vec<Value>,
    game_data: Option<std::sync::Arc<api::game_data::SelectedGameData>>,
    #[cfg(test)]
    maxrange_override: Option<i32>,
}

impl RandomEventEpisode {
    pub fn new(cell: RandomEventCell) -> Self {
        Self {
            cell,
            baseline: None,
            sends: 0,
            send_tick: None,
            send_seq: 0,
            acked: false,
            last: None,
            onset: None,
            hold_rise: None,
            hold_fall: None,
            hold_ticks: 0,
            min_hp: None,
            attacker: None,
            first_target: None,
            max_distance: 0,
            escape: None,
            returned: None,
            retargeted: false,
            targeted_ticks: 0,
            npc_seen: None,
            absent_ticks: 0,
            npc_gone: None,
            item_seen: None,
            script_owned: None,
            box_at_hold: None,
            item_cleared: None,
            lost: None,
            restored: None,
            reward: None,
            trap_entered: None,
            trap_left: None,
            trap_hold: false,
            resolved: None,
            resumed: None,
            trace: Vec::new(),
            game_data: None,
            #[cfg(test)]
            maxrange_override: None,
        }
    }

    /// Content facts (NPC `maxrange`) from the selected revision's data.
    pub fn with_game_data(
        mut self,
        data: Option<std::sync::Arc<api::game_data::SelectedGameData>>,
    ) -> Self {
        self.game_data = data;
        self
    }

    pub fn cell(&self) -> &RandomEventCell {
        &self.cell
    }

    #[cfg(test)]
    pub(crate) fn with_maxrange(mut self, maxrange: i32) -> Self {
        self.maxrange_override = Some(maxrange);
        self
    }

    fn maxrange(&self, type_id: Option<usize>) -> Option<i32> {
        #[cfg(test)]
        if let Some(maxrange) = self.maxrange_override {
            return Some(maxrange);
        }
        let id = i32::try_from(type_id?).ok()?;
        self.game_data
            .as_ref()?
            .npc_name(id)
            .map(|row| row.maxrange)
    }

    fn note(&mut self, frame: &Frame, what: &str, extra: Value) {
        if self.trace.len() >= MAX_TRACE {
            return;
        }
        let tile = frame.tile.map(|(x, z, level)| json!([x, z, level]));
        self.trace
            .push(json!({"tick": frame.tick, "what": what, "tile": tile, "extra": extra}));
    }

    /// Where the latched attacker is, for HP-drop trace rows.
    fn attacker_fact(&self, frame: &Frame) -> Value {
        let Some(attacker) = self.attacker else {
            return Value::Null;
        };
        let npc = frame.npcs.iter().find(|n| n.index == attacker.index);
        let distance = frame
            .tile
            .zip(npc)
            .map(|((x, z, _), npc)| cheb((x, z), npc.tile));
        json!({
            "present": npc.is_some(),
            "targeting": npc.is_some_and(|n| n.targets_local),
            "distance": distance,
        })
    }

    /// The trigger command to send now, if any. Call before [`Self::observe`]
    /// on the same frame; the runner sends it and the episode records it.
    pub(crate) fn next_send(&mut self, frame: &Frame) -> Option<String> {
        self.baseline.as_ref()?;
        if !frame.ready
            || self.acked
            || self.onset.is_some()
            || matches!(self.cell.trigger, Trigger::Seeded)
        {
            return None;
        }
        let busy_since_send = self.send_tick.is_some()
            && frame
                .chat_after(self.send_seq)
                .any(|(_, text)| text.contains(BUSY_REPLY));
        let due = match self.send_tick {
            None => true,
            Some(_) => busy_since_send,
        };
        if !due || self.sends >= MAX_SENDS {
            return None;
        }
        self.sends += 1;
        self.send_tick = Some(frame.tick);
        self.send_seq = frame.head();
        let command = self.cell.trigger.command();
        self.note(frame, "send", json!(command));
        Some(command)
    }

    /// Judge one observation.
    pub(crate) fn observe(&mut self, frame: Frame) -> Verdict {
        if !frame.ready {
            return Verdict::Pending;
        }
        if self.baseline.is_none() {
            self.note(
                &frame,
                "baseline",
                json!({"xp": frame.skill_xp, "hp": frame.hp}),
            );
            self.baseline = Some(frame.clone());
            self.last = Some(frame.clone());
            if matches!(self.cell.trigger, Trigger::Seeded) {
                if !matches!(self.cell.oracle, Oracle::StrangeBoxHandoff) {
                    return Verdict::Fail(
                        "fixture: seeded trigger requires the Strange Box handoff oracle".into(),
                    );
                }
                if frame.count(STRANGE_BOX_ID) == 0 {
                    return Verdict::Fail(
                        "fixture: the seeded Strange box is absent at the observation baseline"
                            .into(),
                    );
                }
                self.onset = Some(frame.tick);
                self.item_seen = Some(frame.tick);
                self.note(
                    &frame,
                    "seeded_box",
                    json!({"id": STRANGE_BOX_ID, "count": frame.count(STRANGE_BOX_ID)}),
                );
                if frame.hold {
                    self.hold_rise = Some(frame.tick);
                    self.box_at_hold = Some(frame.count(STRANGE_BOX_ID) > 0);
                    self.note(&frame, "hold_rise", Value::Null);
                }
            }
            return Verdict::Pending;
        }
        let fresh = self
            .last
            .as_ref()
            .is_none_or(|last| last.tick != frame.tick);
        let verdict = self.judge(&frame, fresh);
        self.last = Some(frame);
        verdict
    }

    fn judge(&mut self, frame: &Frame, fresh: bool) -> Verdict {
        let baseline_seq = self.baseline.as_ref().map(Frame::head).unwrap_or(0);

        // Hold edges, sampled on every frame.
        if frame.hold
            && self.hold_rise.is_none()
            && (self.send_tick.is_some() || matches!(self.cell.trigger, Trigger::Seeded))
        {
            self.hold_rise = Some(frame.tick);
            if matches!(self.cell.oracle, Oracle::StrangeBoxHandoff) {
                self.box_at_hold = Some(frame.count(STRANGE_BOX_ID) > 0);
            }
            self.note(frame, "hold_rise", Value::Null);
        }
        if matches!(self.cell.oracle, Oracle::StrangeBoxHandoff)
            && self.hold_rise.is_none()
            && self.script_owned.is_none()
            && !frame.hold
            && frame.count(STRANGE_BOX_ID) > 0
            && self.onset.is_some_and(|onset| {
                frame.tick.saturating_sub(onset) >= STRANGE_BOX_SCRIPT_OWNED_TICKS
            })
        {
            self.script_owned = Some(frame.tick);
            self.note(
                frame,
                "script_owned",
                json!({"ticks": frame.tick.saturating_sub(self.onset.unwrap_or(frame.tick))}),
            );
        }
        if let Some(last) = self.last.as_ref() {
            if last.hold && !frame.hold && self.hold_rise.is_some() {
                self.hold_fall = Some(frame.tick);
                let tick = frame.tick;
                self.note(frame, "hold_fall", json!(tick));
            }
        }
        if fresh && frame.hold {
            self.hold_ticks += 1;
        }
        if let Some(hp) = frame.hp {
            self.min_hp = Some(self.min_hp.map_or(hp, |min| min.min(hp)));
        }
        if self.send_tick.is_some() || matches!(self.cell.trigger, Trigger::Seeded) {
            if let Some(last) = self.last.clone() {
                if frame.hp != last.hp {
                    let attacker = self.attacker_fact(frame);
                    self.note(
                        frame,
                        "hp",
                        json!({
                            "hp": frame.hp,
                            "attacker": attacker,
                            "targeting_npcs": frame.targeting_npcs,
                            "local_target": frame.local_target,
                        }),
                    );
                }
                if frame.modal != last.modal {
                    let what = if frame.modal {
                        "modal_open"
                    } else {
                        "modal_closed"
                    };
                    self.note(frame, what, Value::Null);
                }
            }
        }

        // Death and failed-event teleports fail at once, whatever the class.
        if frame
            .chat_after(baseline_seq)
            .any(|(_, text)| text.contains(DEATH_MESSAGE))
            || ((self.send_tick.is_some() || matches!(self.cell.trigger, Trigger::Seeded))
                && frame.hp == Some(0))
        {
            return Verdict::Fail(format!(
                "the bot died at game tick {} at {:?} during the {} event",
                frame.tick, frame.tile, self.cell.event
            ));
        }
        if let (Some(last), Some(now)) = (self.last.as_ref().and_then(|l| l.tile), frame.tile) {
            let jump = cheb((last.0, last.1), (now.0, now.1)) > TELEPORT_TILES || last.2 != now.2;
            let trap_move = matches!(self.cell.oracle, Oracle::Trap(_))
                && (trapped_area(last.0, last.1, last.2).is_some()
                    || trapped_area(now.0, now.1, now.2).is_some());
            if jump
                && !trap_move
                && (self.send_tick.is_some() || matches!(self.cell.trigger, Trigger::Seeded))
            {
                return Verdict::Fail(format!(
                    "teleported from {last:?} to {now:?} at game tick {} (the content's failed-event \
                     teleport, macro_event_fail_teleport): the {} event was not handled",
                    frame.tick, self.cell.event
                ));
            }
        }

        // Trigger acknowledgement (patched cheat) and onset.
        if let Some(prefix) = self.cell.trigger.ack_prefix() {
            if !self.acked {
                if let Some((_, line)) = frame
                    .chat_after(self.send_seq)
                    .find(|(_, text)| text.starts_with(&prefix))
                {
                    let line = line.clone();
                    if line.contains("refused") {
                        return Verdict::Fail(format!(
                            "fixture: the cheat refused the {} event at game tick {}: {line}",
                            self.cell.event, frame.tick
                        ));
                    }
                    self.acked = true;
                    self.note(frame, "ack", json!(line));
                } else if let Some(sent) = self.send_tick {
                    let busy = frame
                        .chat_after(self.send_seq)
                        .any(|(_, text)| text.contains(BUSY_REPLY));
                    if !busy && frame.tick.saturating_sub(sent) > ACK_TICKS {
                        return Verdict::Unavailable(format!(
                            "UNAVAILABLE: engine patch {SKILL_EVENT_PATCH} is not installed: \
                             `{}` got no `{prefix}` answer within {ACK_TICKS} game ticks",
                            self.cell.trigger.command()
                        ));
                    }
                }
            }
            if !self.acked {
                return self.sends_exhausted();
            }
        }
        if self.onset.is_none() {
            if let Some(verdict) = self.detect_onset(frame) {
                return verdict;
            }
        }
        let Some(onset) = self.onset else {
            return self.sends_exhausted();
        };

        if let Some(verdict) = self.judge_class(frame, fresh, onset) {
            return verdict;
        }

        // The run goes on: fresh activity XP after the event resolved.
        if let (Some((_, xp)), None) = (self.resolved, self.resumed) {
            if frame.skill_xp.is_some_and(|now| now > xp) {
                self.resumed = Some(frame.tick);
                self.note(frame, "resumed", json!(frame.skill_xp));
                return Verdict::Pass;
            }
        }
        Verdict::Pending
    }

    fn sends_exhausted(&self) -> Verdict {
        if self.sends >= MAX_SENDS {
            Verdict::Fail(format!(
                "fixture: `{}` was refused as busy {MAX_SENDS} times",
                self.cell.trigger.command()
            ))
        } else {
            Verdict::Pending
        }
    }

    /// The event's first observable sign after the trigger.
    fn detect_onset(&mut self, frame: &Frame) -> Option<Verdict> {
        let sent = self.send_tick?;
        let baseline = self.baseline.as_ref()?;
        let present = match self.cell.oracle {
            Oracle::Evade { .. } => frame.npcs.iter().any(|npc| npc.targets_local),
            Oracle::Dialog { .. } | Oracle::Pick => frame
                .tile
                .is_some_and(|(x, z, _)| frame.npcs.iter().any(|npc| cheb((x, z), npc.tile) <= 15)),
            Oracle::StrangeBox | Oracle::StrangeBoxHandoff => {
                frame.count(STRANGE_BOX_ID) > baseline.count(STRANGE_BOX_ID)
            }
            Oracle::Trap(trap) => frame
                .tile
                .is_some_and(|(x, z, level)| trapped_area(x, z, level) == Some(trap.kind())),
            Oracle::LostTool { tool_ids } => !frame.holds_any(tool_ids),
            Oracle::LostGear { gear_id } => frame.count(gear_id) < baseline.count(gear_id),
            // The patched cheat's answer is the onset (no client-visible
            // owner tell for the changed loc / fishing spot).
            Oracle::Endure { .. } => self.acked,
        };
        if present {
            self.onset = Some(frame.tick);
            self.note(frame, "onset", Value::Null);
            return None;
        }
        let busy = frame
            .chat_after(self.send_seq)
            .any(|(_, text)| text.contains(BUSY_REPLY));
        if !busy && frame.tick.saturating_sub(sent) > ONSET_TICKS {
            return Some(Verdict::Fail(format!(
                "the {} event never appeared within {ONSET_TICKS} game ticks of `{}` (sent at game \
                 tick {sent}, {} sends)",
                self.cell.event,
                self.cell.trigger.command(),
                self.sends
            )));
        }
        None
    }

    /// Class-specific checks. `Some` ends the episode; `None` continues.
    fn judge_class(&mut self, frame: &Frame, fresh: bool, onset: u32) -> Option<Verdict> {
        match self.cell.oracle {
            Oracle::Evade { npc } => self.judge_evade(frame, fresh, npc),
            Oracle::Dialog { npc, reward } => self.judge_dialog(frame, fresh, onset, npc, reward),
            Oracle::Pick => {
                self.track_npc(frame, fresh);
                if frame.fruit && self.reward.is_none() {
                    self.reward = Some(frame.tick);
                    self.note(frame, "fruit", Value::Null);
                }
                self.resolve_when(frame, self.reward.is_some() && !frame.hold);
                None
            }
            Oracle::StrangeBox => {
                if frame.count(STRANGE_BOX_ID) > 0 && self.item_seen.is_none() {
                    self.item_seen = Some(frame.tick);
                }
                if self.item_seen.is_some()
                    && frame.count(STRANGE_BOX_ID) == 0
                    && self.item_cleared.is_none()
                {
                    self.item_cleared = Some(frame.tick);
                    self.note(frame, "box_consumed", Value::Null);
                }
                if let Some(verdict) = self.require_hold(frame, onset, DIALOG_ENGAGE_TICKS) {
                    return Some(verdict);
                }
                self.resolve_when(frame, self.item_cleared.is_some() && !frame.hold);
                None
            }
            Oracle::StrangeBoxHandoff => self.judge_strange_box_handoff(frame, onset),
            Oracle::Trap(trap) => {
                let inside = frame
                    .tile
                    .is_some_and(|(x, z, level)| trapped_area(x, z, level) == Some(trap.kind()));
                if inside {
                    self.trap_entered.get_or_insert(frame.tick);
                    self.trap_hold |= frame.hold;
                } else if self.trap_entered.is_some() && self.trap_left.is_none() {
                    self.trap_left = Some(frame.tick);
                    self.note(frame, "trap_left", Value::Null);
                }
                if let Some(verdict) = self.require_hold(frame, onset, HOLD_RISE_TICKS) {
                    return Some(verdict);
                }
                self.resolve_when(
                    frame,
                    self.trap_left.is_some() && self.trap_hold && !frame.hold,
                );
                None
            }
            Oracle::LostTool { tool_ids } => {
                self.lost.get_or_insert(onset);
                if self.restored.is_none() && frame.holds_any(tool_ids) {
                    self.restored = Some(frame.tick);
                    self.note(frame, "tool_restored", Value::Null);
                }
                if let Some(verdict) = self.require_hold(frame, onset, HOLD_RISE_TICKS) {
                    return Some(verdict);
                }
                self.resolve_when(frame, self.restored.is_some() && !frame.hold);
                None
            }
            Oracle::LostGear { gear_id } => {
                self.lost.get_or_insert(onset);
                let base = self
                    .baseline
                    .as_ref()
                    .map_or(1, |b| b.count(gear_id))
                    .max(1);
                if self.restored.is_none() && frame.count(gear_id) >= base {
                    self.restored = Some(frame.tick);
                    self.note(frame, "gear_restored", Value::Null);
                }
                self.resolve_when(frame, self.restored.is_some() && !frame.hold);
                None
            }
            Oracle::Endure { tool_ids } => {
                if !frame.holds_any(tool_ids) && self.lost.is_none() {
                    self.lost = Some(frame.tick);
                    self.note(frame, "tool_lost", Value::Null);
                }
                if self.lost.is_some() && frame.holds_any(tool_ids) && self.restored.is_none() {
                    self.restored = Some(frame.tick);
                    self.note(frame, "tool_restored", Value::Null);
                }
                let survived = frame.tick.saturating_sub(onset) >= ENDURE_TICKS;
                let tool_ok = frame.holds_any(tool_ids);
                self.resolve_when(frame, survived && tool_ok && !frame.hold);
                None
            }
        }
    }

    fn judge_strange_box_handoff(&mut self, frame: &Frame, onset: u32) -> Option<Verdict> {
        let count = frame.count(STRANGE_BOX_ID);
        if self.hold_rise.is_none() {
            if count == 0 {
                return Some(Verdict::Fail(
                    "the seeded Strange box disappeared before native Guardian claimed it".into(),
                ));
            }
            if frame.tick.saturating_sub(onset) > STRANGE_BOX_HANDOFF_TICKS {
                return Some(Verdict::Fail(format!(
                    "native Guardian did not claim the seeded Strange box within \
                     {STRANGE_BOX_HANDOFF_TICKS} game ticks"
                )));
            }
            return None;
        }

        let hold_rise = self.hold_rise.unwrap_or(onset);
        let owned_ticks = self
            .script_owned
            .map_or(0, |tick| tick.saturating_sub(onset));
        if owned_ticks < STRANGE_BOX_SCRIPT_OWNED_TICKS {
            return Some(Verdict::Fail(format!(
                "native Guardian claimed before the Load script retained the Strange box for \
                 {STRANGE_BOX_SCRIPT_OWNED_TICKS} observed game ticks (hold at {hold_rise}, \
                 onset at {onset})"
            )));
        }
        if self.box_at_hold != Some(true) {
            return Some(Verdict::Fail(
                "the seeded Strange box was not present when native Guardian claimed it".into(),
            ));
        }
        if count == 0 && self.item_cleared.is_none() {
            self.item_cleared = Some(frame.tick);
            self.note(frame, "box_consumed", Value::Null);
        }
        if !frame.hold {
            if self.item_cleared.is_none() {
                return Some(Verdict::Fail(
                    "native Guardian released the seeded Strange box before solving it".into(),
                ));
            }
            if self.resolved.is_none() {
                if let Some(xp) = frame.skill_xp {
                    self.resolved = Some((frame.tick, xp));
                }
                self.note(frame, "resolved", json!(frame.skill_xp));
            }
            return Some(Verdict::Pass);
        }
        if frame.tick.saturating_sub(onset) > STRANGE_BOX_HANDOFF_TICKS {
            return Some(Verdict::Fail(format!(
                "native Guardian did not consume and release the seeded Strange box within \
                 {STRANGE_BOX_HANDOFF_TICKS} game ticks"
            )));
        }
        None
    }

    fn resolve_when(&mut self, frame: &Frame, done: bool) {
        if done && self.resolved.is_none() {
            if let Some(xp) = frame.skill_xp {
                self.resolved = Some((frame.tick, xp));
                self.note(frame, "resolved", json!(xp));
            }
        }
    }

    /// The Guardian must take the slot within `game_ticks` of the onset.
    fn require_hold(&self, frame: &Frame, onset: u32, game_ticks: u32) -> Option<Verdict> {
        (self.hold_rise.is_none() && frame.tick.saturating_sub(onset) > game_ticks).then(|| {
            Verdict::Fail(format!(
                "the Guardian never held the slot within {game_ticks} game ticks of the {} event \
                 (onset game tick {onset}, now game tick {})",
                self.cell.event, frame.tick
            ))
        })
    }

    /// Track the event NPC's presence; latches `npc_gone` after
    /// [`GONE_TICKS`] consecutive absent game ticks.
    fn track_npc(&mut self, frame: &Frame, fresh: bool) {
        let present = match self.attacker {
            Some(attacker) => frame.npcs.iter().any(|npc| npc.index == attacker.index),
            None => !frame.npcs.is_empty(),
        };
        if present {
            self.npc_seen.get_or_insert(frame.tick);
            self.absent_ticks = 0;
        } else if fresh && self.npc_seen.is_some() {
            self.absent_ticks += 1;
            if self.absent_ticks >= GONE_TICKS && self.npc_gone.is_none() {
                self.npc_gone = Some(frame.tick);
                self.note(frame, "npc_gone", Value::Null);
            }
        }
    }

    fn judge_evade(&mut self, frame: &Frame, fresh: bool, npc: &'static str) -> Option<Verdict> {
        if self.attacker.is_none() {
            if let Some(found) = frame.npcs.iter().find(|n| n.targets_local) {
                let maxrange = self.maxrange(found.type_id);
                self.attacker = Some(Attacker {
                    index: found.index,
                    type_id: found.type_id,
                    start: found.tile,
                    maxrange,
                });
                self.first_target = Some(frame.tick);
                let start = found.tile;
                self.note(
                    frame,
                    "attacker",
                    json!({"index": found.index, "type": found.type_id, "start": [start.0, start.1], "maxrange": maxrange}),
                );
            }
        }
        let attacker = self.attacker?;
        let Some(maxrange) = attacker.maxrange else {
            return Some(Verdict::Fail(format!(
                "oracle: no content maxrange for {npc} type {:?}",
                attacker.type_id
            )));
        };
        self.track_npc(frame, fresh);
        let targeting = frame
            .npcs
            .iter()
            .any(|n| n.index == attacker.index && n.targets_local);
        if targeting && fresh {
            self.targeted_ticks += 1;
            if self.hold_fall.is_some() && !self.retargeted {
                self.retargeted = true;
                let hold = frame.hold;
                self.note(frame, "retargeted_after_release", json!({"hold": hold}));
            }
        }
        let first = self.first_target.unwrap_or(frame.tick);
        if self.hold_rise.is_none() && frame.tick.saturating_sub(first) > HOLD_RISE_TICKS {
            return Some(Verdict::Fail(format!(
                "the Guardian did not take the slot within {HOLD_RISE_TICKS} game ticks of {npc} \
                 targeting the bot (first targeted at game tick {first}, start {:?})",
                attacker.start
            )));
        }
        let (x, z, _) = frame.tile?;
        let distance = cheb((x, z), attacker.start);
        if self.escape.is_none() && outside_chase_range((x, z), attacker.start, maxrange) {
            self.escape = Some((frame.tick, distance));
            self.note(
                frame,
                "escaped",
                json!({"distance": distance, "maxrange": maxrange}),
            );
        }
        if self.hold_rise.is_some() {
            if targeting && distance + WALK_BACK_SLACK < self.max_distance {
                return Some(Verdict::Fail(format!(
                    "walked back toward {npc} while it still targeted the bot: game tick {} at \
                     ({x},{z}) is {distance} tiles from its start {:?}, after reaching {} \
                     (hold {})",
                    frame.tick, attacker.start, self.max_distance, frame.hold
                )));
            }
            self.max_distance = self.max_distance.max(distance);
            if !frame.hold && self.escape.is_none() && targeting {
                return Some(Verdict::Fail(format!(
                    "the Guardian released the hold at game tick {} while {npc} still targeted the \
                     bot inside its chase range ({distance} tiles from start {:?}, maxrange \
                     {maxrange}): the flee gave up",
                    frame.tick, attacker.start
                )));
            }
        }
        // Observation only: the bot coming back inside the chase range after
        // the escape, and whether the attacker was still there.
        if self.escape.is_some()
            && self.returned.is_none()
            && !outside_chase_range((x, z), attacker.start, maxrange)
        {
            self.returned = Some(frame.tick);
            let present = frame.npcs.iter().any(|n| n.index == attacker.index);
            self.note(
                frame,
                "returned_inside_range",
                json!({"attacker_present": present, "attacker_targeting": targeting}),
            );
        }
        // After the Guardian let go, the same attacker must not be hitting
        // the bot: a hit while it targets the bot means the evade ended
        // before the attacker gave up (the bot is left to fight it).
        if self.hold_fall.is_some() && !frame.hold && targeting {
            if let Some(last_hp) = self.last.as_ref().and_then(|last| last.hp) {
                if frame.hp.is_some_and(|hp| hp < last_hp) {
                    return Some(Verdict::Fail(format!(
                        "{npc} hit the bot at game tick {} ({last_hp} -> {:?} HP, at ({x},{z}), {distance} \
                         tiles from its start {:?}) after the Guardian released the hold at game \
                         tick {:?}: the attacker was still targeting the bot",
                        frame.tick, frame.hp, attacker.start, self.hold_fall
                    )));
                }
            }
        }
        let disengaged = self.npc_gone.is_some() || !targeting;
        self.resolve_when(
            frame,
            self.escape.is_some() && !frame.hold && self.hold_rise.is_some() && disengaged,
        );
        None
    }

    fn judge_dialog(
        &mut self,
        frame: &Frame,
        fresh: bool,
        onset: u32,
        npc: &'static str,
        reward: Reward,
    ) -> Option<Verdict> {
        self.track_npc(frame, fresh);
        if self.hold_rise.is_none() && frame.tick.saturating_sub(onset) > DIALOG_ENGAGE_TICKS {
            return Some(Verdict::Fail(format!(
                "the Guardian never engaged the {npc} within {DIALOG_ENGAGE_TICKS} game ticks of it \
                 appearing (onset game tick {onset})"
            )));
        }
        let baseline = self.baseline.clone()?;
        let got = match reward {
            Reward::Lamp => {
                if frame.count(LAMP_ID) > 0 && self.item_seen.is_none() {
                    self.item_seen = Some(frame.tick);
                    self.note(frame, "lamp", Value::Null);
                }
                self.item_seen.is_some()
                    && frame.count(LAMP_ID) == 0
                    && frame.strength_xp > baseline.strength_xp
            }
            Reward::AnyOf(ids) => ids.iter().any(|id| frame.count(*id) > baseline.count(*id)),
            Reward::NewItem => frame
                .held
                .iter()
                .any(|(id, count)| *count > baseline.count(*id) && *id != LAMP_ID),
        };
        if got && self.reward.is_none() {
            self.reward = Some(frame.tick);
            self.note(frame, "reward", Value::Null);
        }
        self.resolve_when(
            frame,
            self.reward.is_some() && self.npc_gone.is_some() && !frame.hold,
        );
        None
    }

    /// Compact receipt for the terminal evidence and the step-budget message.
    pub fn receipt(&self) -> Value {
        json!({
            "event": self.cell.event,
            "trigger": self.cell.trigger.command(),
            "oracle": format!("{:?}", self.cell.oracle),
            "sends": self.sends,
            "acked": self.acked,
            "onset": self.onset,
            "hold_rise": self.hold_rise,
            "script_owned": self.script_owned,
            "box_at_hold": self.box_at_hold,
            "hold_fall": self.hold_fall,
            "hold_ticks": self.hold_ticks,
            "min_hp": self.min_hp,
            "attacker": self.attacker.map(|a| json!({"index": a.index, "type": a.type_id, "start": [a.start.0, a.start.1], "maxrange": a.maxrange})),
            "first_target": self.first_target,
            "targeted_ticks": self.targeted_ticks,
            "max_distance": self.max_distance,
            "escape": self.escape.map(|(tick, distance)| json!({"tick": tick, "distance": distance})),
            "returned_inside_range": self.returned,
            "npc_gone": self.npc_gone,
            "reward": self.reward,
            "trap": {"entered": self.trap_entered, "left": self.trap_left, "held": self.trap_hold},
            "lost": self.lost,
            "restored": self.restored,
            "box_consumed": self.item_cleared,
            "resolved": self.resolved.map(|(tick, xp)| json!({"tick": tick, "xp": xp})),
            "resumed": self.resumed,
            "trace": self.trace,
        })
    }

    /// One-line summary for a step-budget failure.
    pub fn summary(&self) -> String {
        format!(
            "{} episode [onset={:?},hold_rise={:?},hold_fall={:?},escape={:?},npc_gone={:?},reward={:?},lost={:?},restored={:?},trap={:?}/{:?},resolved={:?},resumed={:?}]",
            self.cell.event,
            self.onset,
            self.hold_rise,
            self.hold_fall,
            self.escape,
            self.npc_gone,
            self.reward,
            self.lost,
            self.restored,
            self.trap_entered,
            self.trap_left,
            self.resolved,
            self.resumed
        )
    }
}

#[cfg(test)]
#[path = "random_episode_tests.rs"]
mod tests;
