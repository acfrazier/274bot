//! M-279 NPC bank access policy, shared by the compat and native adapters.

use crate::shim::InteractReq;

pub(crate) const READY_MS: u64 = 4_000;
pub(crate) const PAGE_ACK_MS: u64 = 3_000;
pub(crate) const CONTINUE_TICKS: u64 = 1;
pub(crate) const CHOICE_TICKS: u64 = 2;
pub(crate) const NPC_DIALOG_MS: u64 = 6_000;
pub(crate) const NPC_OPEN_MS: u64 = 3_000;
const NPC_ATTEMPTS: u8 = 3;
const NPC_PRESSES: u8 = 12;

#[derive(Clone, Copy)]
pub(crate) enum NpcOp<'a> {
    Name(&'a str),
    Index(i32),
}

pub(crate) struct Intent<'a> {
    pub name: &'a str,
    pub op: NpcOp<'a>,
    pub choose: &'a str,
}

pub(crate) struct Banker {
    pub name: String,
    pub action: String,
    pub index: i32,
}

#[derive(Clone, Copy)]
pub(crate) struct Chat {
    pub tick: u64,
    pub modal: i32,
    pub can_continue: bool,
}

impl Chat {
    fn open(self) -> bool {
        self.modal != -1
    }
}

pub(crate) trait Context {
    type Error;
    fn bank_open(&self) -> bool;
    fn bank_loaded(&self) -> bool;
    fn chat(&self) -> Chat;
    fn banker(&self, name: &str, op: NpcOp<'_>) -> Option<Banker>;
    fn choice(&self, choose: &str) -> Option<i32>;
    fn arm(&mut self, millis: u64);
    fn expired(&mut self) -> bool;
    fn emit(&mut self, request: InteractReq) -> Result<(), Self::Error>;
}

pub(crate) enum Note {
    NoBanker,
    NoDialogue,
    NotOpened,
    Unloaded,
}

impl Note {
    #[cfg(feature = "load")]
    pub fn message(self, name: &str) -> String {
        match self {
            Self::NoBanker => format!("no '{name}' in the scene to bank with"),
            Self::NoDialogue => format!("'{name}' never opened a dialogue"),
            Self::NotOpened => format!("could not get {name} to open the bank"),
            Self::Unloaded => "bank: opened but the item list never arrived".into(),
        }
    }
}

pub(crate) enum Step {
    Wait,
    Note(Note),
    Done(bool),
}

#[derive(Clone, Copy)]
enum Phase {
    Attempt,
    WaitDialog,
    Delay,
    Drive,
    Press,
    Settle,
    Ready,
    ReadyWait,
    Finish(bool),
}

/// The adapters retain the intent and supply their existing pause-aware clock.
/// Sequencing contains neither isolate state nor a second native dialogue loop.
#[derive(Clone, Copy)]
pub(crate) struct NpcAccess {
    phase: Phase,
    attempt: u8,
    presses: u8,
    choice: bool,
    ack_modal: i32,
    due: Option<u64>,
}

impl NpcAccess {
    pub fn new() -> Self {
        Self {
            phase: Phase::Attempt,
            attempt: 0,
            presses: 0,
            choice: false,
            ack_modal: -1,
            due: None,
        }
    }

    pub fn step<C: Context>(&mut self, intent: Intent<'_>, cx: &mut C) -> Result<Step, C::Error> {
        loop {
            match self.phase {
                Phase::Attempt => {
                    if self.attempt >= NPC_ATTEMPTS || cx.bank_open() {
                        self.phase = Phase::Ready;
                        if !cx.bank_open() {
                            return Ok(Step::Note(Note::NotOpened));
                        }
                        continue;
                    }
                    let chat = cx.chat();
                    if chat.open() || chat.can_continue {
                        self.presses = 0;
                        self.phase = Phase::Drive;
                        continue;
                    }
                    let Some(banker) = cx.banker(intent.name, intent.op) else {
                        self.due = Some(chat.tick.saturating_add(1));
                        self.phase = Phase::Delay;
                        return Ok(Step::Note(Note::NoBanker));
                    };
                    cx.arm(NPC_DIALOG_MS);
                    cx.emit(InteractReq::Npc {
                        name: banker.name,
                        action: banker.action,
                        index: Some(banker.index),
                    })?;
                    self.phase = Phase::WaitDialog;
                    return Ok(Step::Wait);
                }
                Phase::WaitDialog => {
                    let chat = cx.chat();
                    if chat.open() || chat.can_continue || cx.bank_open() {
                        self.presses = 0;
                        self.phase = Phase::Drive;
                    } else if cx.expired() {
                        self.attempt += 1;
                        self.phase = Phase::Attempt;
                        return Ok(Step::Note(Note::NoDialogue));
                    } else {
                        return Ok(Step::Wait);
                    }
                }
                Phase::Delay => {
                    if self.due.is_some_and(|due| cx.chat().tick < due) {
                        return Ok(Step::Wait);
                    }
                    self.phase = Phase::Attempt;
                    self.attempt += 1;
                    self.due = None;
                }
                Phase::Drive => {
                    if self.presses >= NPC_PRESSES || cx.bank_open() {
                        cx.arm(NPC_OPEN_MS);
                        self.phase = Phase::Settle;
                        continue;
                    }
                    let chat = cx.chat();
                    let option = (!intent.choose.is_empty())
                        .then(|| cx.choice(intent.choose))
                        .flatten();
                    let (request, choice) = match option {
                        Some(option) => (InteractReq::Answer { option }, true),
                        None if chat.can_continue => {
                            (InteractReq::ContinueDialog { component_id: None }, false)
                        }
                        None => {
                            cx.arm(NPC_OPEN_MS);
                            self.phase = Phase::Settle;
                            continue;
                        }
                    };
                    cx.arm(PAGE_ACK_MS);
                    cx.emit(request)?;
                    self.choice = choice;
                    self.ack_modal = chat.modal;
                    self.due = None;
                    self.phase = Phase::Press;
                    return Ok(Step::Wait);
                }
                Phase::Press => {
                    let chat = cx.chat();
                    if let Some(due) = self.due {
                        if chat.tick < due {
                            return Ok(Step::Wait);
                        }
                    } else {
                        let acked = chat.modal != self.ack_modal
                            || if self.choice {
                                chat.can_continue
                            } else {
                                !chat.can_continue
                            };
                        if acked {
                            let ticks = if self.choice {
                                CHOICE_TICKS
                            } else {
                                CONTINUE_TICKS
                            };
                            self.due = Some(chat.tick.saturating_add(ticks));
                            return Ok(Step::Wait);
                        }
                        if !cx.expired() {
                            return Ok(Step::Wait);
                        }
                    }
                    self.presses += 1;
                    self.phase = Phase::Drive;
                }
                Phase::Settle => {
                    if !cx.bank_open() && !cx.expired() {
                        return Ok(Step::Wait);
                    }
                    self.attempt += 1;
                    self.phase = Phase::Attempt;
                }
                Phase::Ready => {
                    if !cx.bank_open() {
                        self.phase = Phase::Finish(false);
                        continue;
                    }
                    if cx.bank_loaded() {
                        self.phase = Phase::Finish(true);
                        continue;
                    }
                    cx.arm(READY_MS);
                    self.phase = Phase::ReadyWait;
                    return Ok(Step::Wait);
                }
                Phase::ReadyWait => {
                    if !cx.bank_open() {
                        self.phase = Phase::Finish(false);
                    } else if cx.bank_loaded() {
                        self.phase = Phase::Finish(true);
                    } else if cx.expired() {
                        self.phase = Phase::Finish(true);
                        return Ok(Step::Note(Note::Unloaded));
                    } else {
                        return Ok(Step::Wait);
                    }
                }
                Phase::Finish(open) => return Ok(Step::Done(open)),
            }
        }
    }
}

pub(crate) fn name_matches(actual: &str, wanted: &str) -> bool {
    let actual = actual.trim();
    let wanted = wanted.trim();
    if actual.is_ascii() && wanted.is_ascii() {
        actual.eq_ignore_ascii_case(wanted)
    } else {
        actual
            .chars()
            .flat_map(char::to_lowercase)
            .eq(wanted.chars().flat_map(char::to_lowercase))
    }
}

pub(crate) fn action_matches(actual: &str, index: usize, wanted: NpcOp<'_>) -> bool {
    if actual.is_empty() || actual == "hidden" {
        return false;
    }
    match wanted {
        NpcOp::Name(name) => name_matches(actual, name),
        NpcOp::Index(op) => usize::try_from(op).is_ok_and(|op| index + 1 == op),
    }
}

pub(crate) fn option_matches(actual: &str, wanted: &str) -> bool {
    if wanted.is_empty() {
        return false;
    }
    if actual.is_ascii() && wanted.is_ascii() {
        return actual
            .as_bytes()
            .windows(wanted.len())
            .any(|window| window.eq_ignore_ascii_case(wanted.as_bytes()));
    }
    actual.char_indices().any(|(at, _)| {
        let mut actual = actual[at..].chars().flat_map(char::to_lowercase);
        wanted
            .chars()
            .flat_map(char::to_lowercase)
            .all(|wanted| actual.next() == Some(wanted))
    })
}

/// The banker both adapters pick from their scene rows: named `name`,
/// carrying `op` (by label or by packed 1-based index), nearest first.
/// A row with no posted name reads as empty, so it can only match an
/// empty wanted name; adapters never ask for one.
pub(crate) fn banker_scan<'a, R: crate::bank::ops::NpcRow>(
    rows: &'a [R],
    name: &str,
    op: NpcOp<'_>,
) -> Option<(&'a R, &'a str)> {
    rows.iter()
        .filter(|row| name_matches(row.name().unwrap_or_default(), name))
        .filter_map(|row| {
            let action = (1..=row.op_len()).find_map(|slot| {
                let label = row.op_label(slot)?;
                let index = usize::try_from(slot).ok()?.checked_sub(1)?;
                action_matches(label, index, op).then_some(label)
            })?;
            Some((row, action))
        })
        .min_by_key(|(row, _)| row.distance())
}

#[cfg(test)]
mod tests {
    use super::*;
    use api::snapshot::{NpcView, WorldTile};

    fn npc(index: usize, name: &str, distance: i32, actions: &[&str]) -> NpcView {
        let tile = WorldTile {
            x: 3200,
            z: 3200,
            level: 0,
        };
        NpcView {
            index,
            r#type: None,
            name: Some(name.to_owned()),
            actions: actions
                .iter()
                .map(|action| Some((*action).to_owned()))
                .collect(),
            tile,
            distance,
            animation: -1,
            animation_frame: 0,
            pose_animation: -1,
            orientation: 0,
            target_orientation: 0,
            overhead_text: None,
            spot_animation: -1,
            spot_animation_stamp: -1,
            health: 0,
            total_health: 0,
            face_entity: -1,
            target: None,
            moving: false,
            running: false,
            in_combat: false,
            level: 0,
            size: 1,
            network: tile,
            x: tile.x,
            z: tile.z,
            yaw: 0,
        }
    }

    /// Characterization (T2.2 r2): the shared banker scan is
    /// nearest-first, over rows passing the name and op filters; the
    /// first row wins an equal-distance tie.
    #[test]
    fn banker_scan_picks_the_nearest_named_banker_with_the_op() {
        let rows = [
            npc(1, "Trader", 0, &["Bank"]),
            npc(2, "Banker", 1, &["Talk-to"]),
            npc(3, "Banker", 9, &["Talk-to", "Bank"]),
            npc(4, " banker ", 2, &["Bank"]),
            npc(5, "Banker", 5, &["hidden", "Bank"]),
            npc(6, "Banker", 2, &["Bank"]),
        ];
        let (row, action) = banker_scan(&rows, "Banker", NpcOp::Name("Bank"))
            .expect("a banker with the Bank op is in the scene");
        assert_eq!(row.index, 4, "nearest matching banker, first on a tie");
        assert_eq!(action, "Bank");
        // By packed index the same nearest rule holds; the empty and
        // `hidden` labels never carry an op.
        let (row, _) =
            banker_scan(&rows, "Banker", NpcOp::Index(2)).expect("a banker carries a second op");
        assert_eq!(row.index, 5, "distance 5 beats the distance-9 row");
        assert!(banker_scan(&rows, "Fremennik", NpcOp::Name("Bank")).is_none());
        assert!(banker_scan(&rows, "Banker", NpcOp::Name("Collect")).is_none());
    }
}
