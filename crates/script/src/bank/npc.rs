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
                        None if chat.can_continue => (InteractReq::ContinueDialog, false),
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
