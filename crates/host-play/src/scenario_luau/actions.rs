//! Open File roots for normal non-walk actions and their closure (LD2 §5.1).

use api::snapshot::{GameSnapshot, ItemView, WorldTile};
use client::client::Client;
use script::interact::InteractReq;
use serde_json::{json, Value};

use super::barrier::Barrier;
use super::contracts::{Closure, Contract, Domain, FileAction};

type Slots = Vec<(i32, i32, i32)>;

fn slots(items: &[ItemView]) -> Slots {
    items
        .iter()
        .map(|item| (item.slot, item.def.id, item.count))
        .collect()
}

fn total(slots: &Slots, id: i32) -> i64 {
    slots
        .iter()
        .filter(|(_, object, _)| *object == id)
        .map(|(_, _, count)| i64::from(*count))
        .sum()
}

fn chat_after(sequence: i32, baseline: Option<i32>) -> bool {
    baseline.is_none_or(|baseline| sequence.wrapping_sub(baseline) > 0)
}

/// Resolve the request the normal owner is about to send to exact types.
/// Anything that is not one exact posted row is refused, never guessed.
pub(crate) fn resolve(
    request: &InteractReq,
    snapshot: &GameSnapshot,
) -> Result<FileAction, String> {
    let carried = |id: Option<i32>, slot: Option<i32>, name: &str| -> Result<i32, String> {
        let rows: Vec<&ItemView> = snapshot
            .inventory()
            .iter()
            .filter(|row| {
                id.map_or_else(
                    || {
                        row.def
                            .name
                            .as_deref()
                            .is_some_and(|label| label.eq_ignore_ascii_case(name))
                    },
                    |id| row.def.id == id,
                ) && slot.is_none_or(|slot| row.slot == slot)
            })
            .collect();
        match rows.as_slice() {
            [] => Err("File action item is not carried".into()),
            [first, rest @ ..] if rest.iter().all(|row| row.def.id == first.def.id) => {
                Ok(first.def.id)
            }
            _ => Err("File action item name selects more than one item type".into()),
        }
    };
    let held_op = |row_id: i32, label: &str| -> Result<u8, String> {
        let row = snapshot
            .inventory()
            .iter()
            .find(|row| row.def.id == row_id)
            .ok_or("File held item is not carried")?;
        let ops: Vec<usize> = row
            .actions
            .iter()
            .enumerate()
            .filter(|(_, action)| {
                action
                    .as_deref()
                    .is_some_and(|action| action.eq_ignore_ascii_case(label))
            })
            .map(|(index, _)| index)
            .collect();
        match ops.as_slice() {
            [index] => u8::try_from(index + 1).map_err(|_| "held op out of range".into()),
            _ => Err("File held action does not select exactly one item option".into()),
        }
    };
    match request {
        InteractReq::UseOn {
            name,
            kind,
            target_name,
            source_item_id,
            source_item_slot,
            target_item_id,
            target_item_slot,
            ..
        } if matches!(kind.as_str(), "inv" | "held") => Ok(FileAction::UseOnHeld {
            source: carried(*source_item_id, *source_item_slot, name)?,
            target: carried(
                *target_item_id,
                *target_item_slot,
                target_name.as_deref().unwrap_or_default(),
            )?,
        }),
        InteractReq::Held {
            name,
            action,
            slot,
            target_item_id,
        } => {
            let item = carried(*target_item_id, *slot, name)?;
            Ok(FileAction::HeldOp {
                item,
                op: held_op(item, action)?,
            })
        }
        InteractReq::Unequip { name } => {
            let ids: Vec<i32> = snapshot
                .equipment()
                .iter()
                .filter(|row| {
                    row.def
                        .name
                        .as_deref()
                        .is_some_and(|label| label.eq_ignore_ascii_case(name))
                })
                .map(|row| row.def.id)
                .collect();
            match ids.as_slice() {
                [item] => Ok(FileAction::Unequip { item: *item }),
                _ => Err("File unequip does not select exactly one worn row".into()),
            }
        }
        InteractReq::Loc {
            x,
            z,
            level,
            action,
            id,
        } => {
            // script_interact::dispatch_loc takes the first row at the tile.
            let loc = snapshot
                .locs()
                .iter()
                .find(|loc| {
                    (loc.tile.x, loc.tile.z, loc.tile.level) == (*x, *z, *level)
                        && id.is_none_or(|id| loc.id == id)
                })
                .ok_or("File loc action has no posted loc at its tile")?;
            let ops: Vec<usize> = loc
                .actions
                .iter()
                .enumerate()
                .filter(|(_, label)| {
                    label
                        .as_deref()
                        .is_some_and(|label| label.eq_ignore_ascii_case(action))
                })
                .map(|(index, _)| index)
                .collect();
            let [index] = ops.as_slice() else {
                return Err("File loc action does not select exactly one loc option".into());
            };
            Ok(FileAction::LocOp {
                loc: loc.id,
                tile: WorldTile {
                    x: *x,
                    z: *z,
                    level: *level,
                },
                op: u8::try_from(index + 1).map_err(|_| "loc op out of range")?,
            })
        }
        _ => Err("native action family has no File root support".into()),
    }
}

#[derive(Debug)]
struct SendState {
    tick: u64,
    chat_baseline: Option<i32>,
    carried: Slots,
    worn: Slots,
    stats: Vec<(i32, i32, i32, i32)>,
}

/// A paired same-item positive control owed by a silent guard closure.
#[derive(Debug, Clone)]
pub(super) struct PairedControl {
    pub(super) item: i32,
    pub(super) skill: i32,
    pub(super) level: i32,
    pub(super) silent_root: u64,
}

pub(super) struct ActionRoot {
    pub(super) request: u64,
    pub(super) session: u64,
    pub(super) action: FileAction,
    pub(super) contract: Contract,
    sent: Option<SendState>,
    terminal_seen: Option<i32>,
    /// An admitted silence window that started before this root held.
    pub(super) window_held: bool,
    barrier: Option<Barrier>,
}

impl ActionRoot {
    pub(super) fn new(request: u64, session: u64, action: FileAction, contract: Contract) -> Self {
        Self {
            request,
            session,
            action,
            contract,
            sent: None,
            terminal_seen: None,
            window_held: false,
            barrier: None,
        }
    }

    pub(super) fn sent(&self) -> bool {
        self.sent.is_some()
    }

    /// Capture the pre-send state of the frame that sends this root.
    pub(super) fn note_send(&mut self, snapshot: &GameSnapshot) {
        self.sent = Some(SendState {
            tick: u64::from(snapshot.tick()),
            chat_baseline: snapshot.chat_lines().first().map(|line| line.sequence),
            carried: slots(snapshot.inventory()),
            worn: slots(snapshot.equipment()),
            stats: snapshot
                .stats()
                .iter()
                .map(|stat| (stat.index, stat.base, stat.effective, stat.xp))
                .collect(),
        });
    }

    fn fresh_message(&self, snapshot: &GameSnapshot, text: &str) -> Option<i32> {
        let sent = self.sent.as_ref()?;
        snapshot
            .chat_lines()
            .iter()
            .find(|line| {
                line.type_ == 0
                    && chat_after(line.sequence, sent.chat_baseline)
                    && line.text == text
            })
            .map(|line| line.sequence)
    }

    /// The fresh post-send terminal record of a `Terminal` contract, if present.
    pub(super) fn terminal_evidence(&self, snapshot: &GameSnapshot) -> Option<Value> {
        let Closure::Terminal(terminal) = &self.contract.closure else {
            return None;
        };
        let sent = self.sent.as_ref()?;
        if snapshot.gens().session != self.session {
            return None;
        }
        if let Some(text) = &terminal.message {
            if let Some(sequence) = self.fresh_message(snapshot, text) {
                return Some(json!({"terminal":"message","text":text,"sequence":sequence}));
            }
        }
        for domain in terminal.inventories {
            let (before, now) = match domain {
                Domain::Carried => (&sent.carried, slots(snapshot.inventory())),
                Domain::Worn => (&sent.worn, slots(snapshot.equipment())),
            };
            for item in &terminal.items {
                let (was, is) = (total(before, *item), total(&now, *item));
                if was != is {
                    return Some(json!({"terminal":domain,"item":item,"before":was,"after":is}));
                }
            }
        }
        None
    }

    /// Whether the level precondition of a silent guard held at send.
    pub(super) fn stat_at_send(&self, skill: i32) -> Option<i32> {
        self.sent
            .as_ref()?
            .stats
            .iter()
            .find(|stat| stat.0 == skill)
            .map(|stat| stat.2)
    }

    /// Drive the NoTrigger or silent-guard closure. Returns the closure
    /// certificate once the barrier and closure watch are complete.
    pub(super) fn poll_window(
        &mut self,
        client: &mut Client,
        snapshot: &GameSnapshot,
        varp: &str,
    ) -> Result<Option<Value>, String> {
        let Some(sent) = &self.sent else {
            return Ok(None);
        };
        if snapshot.gens().session != self.session {
            return Err("action root changed sessions before closure".into());
        }
        let allowed: Vec<String> = match &self.contract.closure {
            Closure::Terminal(_) => return Ok(None),
            Closure::NoTrigger { message } => {
                if self.terminal_seen.is_none() {
                    self.terminal_seen = self.fresh_message(snapshot, message);
                    if self.terminal_seen.is_none() {
                        return Ok(None);
                    }
                }
                vec![message.clone(), "Nothing interesting happens.".into()]
            }
            Closure::SilentGuard { skill, level, .. } => {
                if self
                    .stat_at_send(*skill)
                    .is_none_or(|effective| effective >= *level)
                {
                    return Err("silent guard precondition did not hold at send".into());
                }
                if !self.window_held {
                    return Ok(None);
                }
                Vec::new()
            }
        };
        let barrier = match &mut self.barrier {
            Some(barrier) => barrier,
            slot @ None => {
                *slot = Some(Barrier::start(client, snapshot, varp)?);
                return Ok(None);
            }
        };
        if !barrier.poll(client, snapshot)? {
            return Ok(None);
        }
        let boundary = barrier
            .boundary()
            .ok_or("barrier closed without a boundary")?;
        // Closure watch: no product effect from send through the applied reply 2.
        for line in snapshot.chat_lines() {
            if line.type_ != 0
                || !chat_after(line.sequence, sent.chat_baseline)
                || line.sequence.wrapping_sub(boundary) > 0
                || barrier.is_control(line.sequence)
                || allowed.iter().any(|text| *text == line.text)
            {
                continue;
            }
            return Err(format!(
                "unexpected game message inside the closure window: {:?}",
                line.text
            ));
        }
        if slots(snapshot.inventory()) != sent.carried || slots(snapshot.equipment()) != sent.worn {
            return Err("carried or worn inventory changed inside the closure window".into());
        }
        if let Closure::SilentGuard { skill, level, .. } = &self.contract.closure {
            let now = snapshot
                .stats()
                .iter()
                .find(|stat| stat.index == *skill)
                .map(|stat| (stat.index, stat.base, stat.effective, stat.xp));
            if now != sent.stats.iter().copied().find(|stat| stat.0 == *skill)
                || now.is_none_or(|stat| stat.2 >= *level)
            {
                return Err(
                    "silent guard skill or precondition changed inside the closure window".into(),
                );
            }
        }
        Ok(Some(json!({
            "root": self.request,
            "disposition": "EffectClosed",
            "route": "NoEffectWindow",
            "certificate": "ClosedNoPendingEffect",
            "branch_cause": "Unattributed",
            "contract": self.contract.id,
            "citations": self.contract.citations,
            "terminal_record_sequence": self.terminal_seen,
            "send_tick": sent.tick,
            "closing_tick": snapshot.tick(),
            "barrier": barrier.record(),
        })))
    }
}
