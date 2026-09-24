//! Rust-owned bank access openers: frozen `Bank.openNearestAccess` (the
//! `bank_access` [`crate::machine`] family) and `Bank.openNpcAccess`
//! (`bank_npc_access`), for bank rows whose access is a chest, a named loc
//! behind an opener, or a banker's conversation.
//!
//! The default booth access (`Bank booth` / `Use-quickly`) keeps the host's
//! approach-checked booth open ([`crate::bank_open`]); every other object
//! access is the frozen `openNearest` loop over the named loc (interact,
//! object-dialog continue, step beside it, retry), after an optional
//! `openFirst` loc. The NPC opener talks to the banker and answers the
//! access option. Both end with the frozen `openedReady` item-list wait.
//! Log lines go to the caller's `log` through the callback path.

use crate::bank_op::BankView;
use crate::bank_open::{BankOpen, BankOpenArgs};
use crate::dialog::{CHOICE_TICKS, CONTINUE_TICKS, PAGE_ACK_MS};
use crate::machine::{Begin, Call, Cx, Family, Reply, Step};
use crate::observed::{self, Scene, Text};
use crate::shim::InteractReq;
use api::snapshot::WorldTile;
use serde::Deserialize;
use serde_json::json;

/// Frozen `openedReady` item-list wait.
pub const READY_MS: u64 = 4_000;
/// Frozen first loc/opener interact wait.
pub const INTERACT_MS: u64 = 8_000;
/// Frozen retry interact wait once beside the loc.
pub const ADJACENT_MS: u64 = 4_000;
/// Frozen step-beside walk bound.
pub const STEP_WALK_MS: u64 = 15_000;
/// Frozen wait for the booth walk (the shim's default-booth path).
pub const BOOTH_WALK_MS: u64 = 60_000;
/// Frozen object-dialogue continue → bank open wait.
pub const DIALOG_OPEN_MS: u64 = 4_000;
/// Frozen banker dialogue open wait.
pub const NPC_DIALOG_MS: u64 = 6_000;
/// Frozen wait for the bank after the banker dialogue.
pub const NPC_OPEN_MS: u64 = 3_000;

const LOG: usize = 0;

/// The chat facts the openers read.
struct Chat {
    tick: u64,
    modal: i32,
    can_continue: bool,
    options: Vec<String>,
}

impl Chat {
    fn now() -> Self {
        observed::with(|scene| {
            let session = scene.since_login();
            Self {
                tick: scene.tick().unwrap_or(0),
                modal: session.chat_modal_id().unwrap_or(-1),
                can_continue: session.chat_continue().unwrap_or(false),
                options: session.chat_options().cloned().unwrap_or_default(),
            }
        })
    }

    /// `ChatDialog.isOpen()`.
    fn open(&self) -> bool {
        self.modal != -1
    }
}

fn posted_tick() -> u64 {
    observed::with(|scene| scene.tick().unwrap_or(0))
}

/// One `ChatDialog.continue()` / `chooseOption()` press: the verb, the page
/// acknowledgement (bounded) and the frozen settle ticks.
struct Press {
    choice: bool,
    ack_modal: i32,
    due: Option<u64>,
}

impl Press {
    fn send(req: InteractReq, choice: bool, chat: &Chat, cx: &mut Cx<'_>) -> Self {
        cx.clock().arm(PAGE_ACK_MS);
        cx.emit(req);
        Self {
            choice,
            ack_modal: chat.modal,
            due: None,
        }
    }

    fn poll(&mut self, cx: &mut Cx<'_>) -> Option<bool> {
        let chat = Chat::now();
        if let Some(due) = self.due {
            return (chat.tick >= due).then_some(true);
        }
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
            return None;
        }
        cx.clock().bound_reached().then_some(false)
    }
}

/// Frozen `openedReady`: a closed bank is false; an open one waits up to
/// 4 s for its item list, logs when it never came, and answers open.
struct Ready {
    armed: bool,
}

enum Readied {
    Wait,
    Done(bool, Option<String>),
}

impl Ready {
    fn step(&mut self, cx: &mut Cx<'_>) -> Readied {
        let view = BankView::now();
        if !view.open {
            return Readied::Done(false, None);
        }
        if view.ready() {
            return Readied::Done(true, None);
        }
        if !self.armed {
            self.armed = true;
            cx.clock().arm(READY_MS);
        }
        if !cx.clock().bound_reached() {
            return Readied::Wait;
        }
        Readied::Done(
            true,
            Some("bank: opened but the item list never arrived".into()),
        )
    }
}

/// A named loc as `Locs.query()` sees it.
#[derive(Clone, Debug)]
struct Loc {
    id: i32,
    tile: WorldTile,
    distance: i32,
    actions: Vec<Text>,
}

fn present(action: &Text) -> bool {
    !action.is_empty() && &**action != "hidden"
}

/// `Locs.query().name(name).where(filter).nearest()`.
fn nearest_loc(scene: &Scene, name: &str, filter: impl Fn(&Loc) -> bool) -> Option<Loc> {
    let wanted = name.trim().to_lowercase();
    scene
        .since_login()
        .locs()?
        .iter()
        .filter(|row| {
            row.name
                .as_deref()
                .is_some_and(|got| got.trim().to_lowercase() == wanted)
        })
        .map(|row| Loc {
            id: row.id,
            tile: WorldTile {
                x: row.x,
                z: row.z,
                level: row.level,
            },
            distance: row.distance,
            actions: row.actions.iter().filter(|a| present(a)).cloned().collect(),
        })
        .filter(|loc| filter(loc))
        .min_by_key(|loc| loc.distance)
}

/// Frozen `locWithAction(name, op)`.
fn loc_with_action(name: &str, op: &str) -> Option<Loc> {
    let op = op.to_lowercase();
    observed::with(|scene| {
        nearest_loc(scene, name, |loc| {
            loc.actions.iter().any(|action| action.to_lowercase() == op)
        })
    })
}

/// A named loc with any action.
fn usable_loc(name: &str, adjacent: bool) -> Option<Loc> {
    observed::with(|scene| {
        nearest_loc(scene, name, |loc| {
            !loc.actions.is_empty() && (!adjacent || loc.distance <= 1)
        })
    })
}

/// Frozen `openNearest`'s `pick`: the wanted op, else a Use/Bank op, else
/// the first.
fn pick(actions: &[Text], op: &str) -> Option<Text> {
    actions
        .iter()
        .find(|action| action.eq_ignore_ascii_case(op))
        .or_else(|| {
            actions.iter().find(|action| {
                let lower = action.to_ascii_lowercase();
                lower.starts_with("use") || lower.starts_with("bank")
            })
        })
        .or_else(|| actions.first())
        .cloned()
}

fn interact(loc: &Loc, action: &str) -> InteractReq {
    InteractReq::Loc {
        x: loc.tile.x,
        z: loc.tile.z,
        level: loc.tile.level,
        action: action.to_string(),
        id: Some(loc.id),
    }
}

fn walk_near(tile: WorldTile, radius: i32) -> InteractReq {
    InteractReq::WalkNear {
        x: tile.x,
        z: tile.z,
        level: tile.level,
        radius,
        allow_teleports: false,
        allow_wilderness: true,
        allow_bank_fetch: true,
        request_id: 0,
    }
}

/// Frozen `bankStand`: the reachable side neighbour of the loc nearest the
/// player (Chebyshev), in east/west/north/south order on ties.
fn bank_stand(loc: WorldTile) -> Option<WorldTile> {
    let here = observed::with(|scene| scene.latest().here());
    let options = api::query::SceneReachOptions {
        max_steps: Some(400),
        adjacent_ok: false,
    };
    let mut reachable: Vec<WorldTile> = [(1, 0), (-1, 0), (0, 1), (0, -1)]
        .into_iter()
        .map(|(dx, dz)| WorldTile {
            x: loc.x + dx,
            z: loc.z + dz,
            level: loc.level,
        })
        .filter(|tile| crate::load::reach_query::with_view(|view| view.can_reach(*tile, &options)))
        .collect();
    if let Some(here) = here {
        reachable.sort_by_key(|t| (t.x - here.x).abs().max((t.z - here.z).abs()));
    }
    reachable.into_iter().next()
}

#[derive(Clone, Deserialize)]
pub(crate) struct Opener {
    name: String,
    op: String,
}

/// `BankObjectAccess`: the shim passes `open_first`, a bank destination
/// row carries `openFirst`.
#[derive(Clone, Deserialize)]
pub(crate) struct AccessArgs {
    name: String,
    op: String,
    #[serde(default, alias = "openFirst")]
    open_first: Option<Opener>,
}

/// Where a failed object-dialogue continue leaves the `openNearest` loop.
#[derive(Clone)]
enum AfterContinue {
    /// The first interact's branch: step beside the loc next.
    Approach(Loc),
    /// The retry's branch: the next attempt.
    Next,
}

enum Phase {
    Start,
    BoothWalk(WorldTile),
    BoothOpen(Box<BankOpen>),
    OpenerFind(u32),
    OpenerWait(u32),
    OpenerDelay(u32, u64),
    OpenerGate,
    NearFind(u32),
    NearWait { attempt: u32, loc: Loc, first: bool },
    NearApproach(u32, Loc),
    NearWalk(u32, Option<Loc>, WorldTile),
    Continue(u32, Press, AfterContinue),
    ContinueOpen(u32, AfterContinue),
    Ready(Ready),
    Finish(bool),
}

/// One awaited frozen `Bank.openNearestAccess(access, log)`.
pub(crate) struct BankAccess {
    name: String,
    op: String,
    open_first: Option<Opener>,
    phase: Phase,
    /// A log line waiting for the caller's `log` hook.
    line: Option<String>,
    log_hook: Option<usize>,
}

const NEAREST_ATTEMPTS: u32 = 6;
const OPENER_ATTEMPTS: u32 = 3;

impl BankAccess {
    /// One `openNearestAccess`; `log_hook` is the caller's `log` hook, if any.
    pub(crate) fn new(args: AccessArgs, log_hook: Option<usize>) -> Self {
        Self {
            name: args.name,
            op: args.op,
            open_first: args.open_first,
            phase: Phase::Start,
            line: None,
            log_hook,
        }
    }

    /// Drive the opener; it reads its own log calls' replies.
    pub(crate) fn run(&mut self, cx: &mut Cx<'_>) -> Step<bool> {
        loop {
            if let Some(step) = log_first(&mut self.line, self.log_hook, cx) {
                return step;
            }
            if let Some(step) = self.decide(cx) {
                return step;
            }
        }
    }

    fn default_booth(&self) -> bool {
        self.open_first.is_none()
            && self.name.eq_ignore_ascii_case("bank booth")
            && self.op.eq_ignore_ascii_case("use-quickly")
    }

    fn say(&mut self, line: String, next: Phase) {
        self.line = Some(line);
        self.phase = next;
    }

    /// One decision; `None` keeps deciding in this step.
    fn decide(&mut self, cx: &mut Cx<'_>) -> Option<Step<bool>> {
        let phase = std::mem::replace(&mut self.phase, Phase::Finish(false));
        match phase {
            Phase::Start => {
                if BankView::now().open {
                    self.phase = Phase::Ready(Ready { armed: false });
                } else if self.default_booth() {
                    let booth =
                        observed::with(|scene| scene.since_login().nearest_booth().cloned());
                    let Some(booth) = booth else {
                        let name = self.name.clone();
                        self.say(
                            format!("no usable '{name}' in the scene"),
                            Phase::Finish(false),
                        );
                        return None;
                    };
                    let tile = WorldTile {
                        x: booth.tile.x,
                        z: booth.tile.z,
                        level: booth.tile.level,
                    };
                    if crate::load::reach_query::arrived(tile, 1) {
                        return self.open_booth(cx);
                    }
                    cx.clock().arm(BOOTH_WALK_MS);
                    cx.emit(walk_near(tile, 1));
                    self.phase = Phase::BoothWalk(tile);
                    return Some(Step::Wait);
                } else if self.open_first.is_some()
                    && loc_with_action(&self.name, &self.op).is_none()
                {
                    self.phase = Phase::OpenerFind(0);
                } else {
                    self.phase = Phase::NearFind(0);
                }
            }
            Phase::BoothWalk(tile) => {
                if crate::load::reach_query::arrived(tile, 1) {
                    return self.open_booth(cx);
                }
                if cx.clock().bound_reached() {
                    return Some(Step::Done(false));
                }
                self.phase = Phase::BoothWalk(tile);
                return Some(Step::Wait);
            }
            Phase::BoothOpen(mut open) => match Family::step(open.as_mut(), cx) {
                Step::Done(ok) => return Some(Step::Done(ok)),
                _ => {
                    self.phase = Phase::BoothOpen(open);
                    return Some(Step::Wait);
                }
            },
            Phase::OpenerFind(attempt) => {
                if attempt >= OPENER_ATTEMPTS || BankView::now().open {
                    self.phase = Phase::OpenerGate;
                    return None;
                }
                let opener = self.open_first.as_ref().expect("opener phase");
                let (name, op) = (opener.name.clone(), opener.op.clone());
                match loc_with_action(&name, &op) {
                    None => {
                        let until = posted_tick().saturating_add(1);
                        self.say(
                            format!("no '{name}' with '{op}' in the scene"),
                            Phase::OpenerDelay(attempt, until),
                        );
                    }
                    Some(closed) => {
                        let action = closed
                            .actions
                            .iter()
                            .find(|a| a.eq_ignore_ascii_case(&op))
                            .map_or(op.clone(), |a| a.to_string());
                        cx.clock().arm(INTERACT_MS);
                        cx.emit(interact(&closed, &action));
                        self.say(
                            format!("opening '{name}' before banking"),
                            Phase::OpenerWait(attempt),
                        );
                    }
                }
            }
            Phase::OpenerWait(attempt) => {
                if BankView::now().open || loc_with_action(&self.name, &self.op).is_some() {
                    self.phase = Phase::OpenerGate;
                } else if cx.clock().bound_reached() {
                    self.phase = Phase::OpenerFind(attempt + 1);
                } else {
                    self.phase = Phase::OpenerWait(attempt);
                    return Some(Step::Wait);
                }
            }
            Phase::OpenerDelay(attempt, until) => {
                if posted_tick() < until {
                    self.phase = Phase::OpenerDelay(attempt, until);
                    return Some(Step::Wait);
                }
                self.phase = Phase::OpenerFind(attempt + 1);
            }
            Phase::OpenerGate => {
                if BankView::now().open {
                    self.phase = Phase::Ready(Ready { armed: false });
                } else if loc_with_action(&self.name, &self.op).is_none() {
                    let name = self.name.clone();
                    self.say(
                        format!("'{name}' never became usable"),
                        Phase::Finish(false),
                    );
                } else {
                    self.phase = Phase::NearFind(0);
                }
            }
            Phase::NearFind(attempt) => {
                if attempt >= NEAREST_ATTEMPTS || BankView::now().open {
                    self.phase = Phase::Ready(Ready { armed: false });
                    return None;
                }
                let Some(loc) = usable_loc(&self.name, false) else {
                    let name = self.name.clone();
                    self.say(
                        format!("no usable '{name}' in the scene"),
                        Phase::Finish(false),
                    );
                    return None;
                };
                match pick(&loc.actions, &self.op) {
                    Some(action) => {
                        cx.clock().arm(INTERACT_MS);
                        cx.emit(interact(&loc, &action));
                        self.phase = Phase::NearWait {
                            attempt,
                            loc,
                            first: true,
                        };
                        return Some(Step::Wait);
                    }
                    None => self.phase = Phase::NearApproach(attempt, loc),
                }
            }
            Phase::NearWait {
                attempt,
                loc,
                first,
            } => {
                let open = BankView::now().open;
                let chat = Chat::now();
                if chat.can_continue {
                    let after = if first {
                        AfterContinue::Approach(loc)
                    } else {
                        AfterContinue::Next
                    };
                    let press = Press::send(InteractReq::ContinueDialog, false, &chat, cx);
                    self.phase = Phase::Continue(attempt, press, after);
                    return Some(Step::Wait);
                }
                if open {
                    self.phase = Phase::Ready(Ready { armed: false });
                } else if cx.clock().bound_reached() {
                    self.phase = if first {
                        Phase::NearApproach(attempt, loc)
                    } else {
                        Phase::NearFind(attempt + 1)
                    };
                } else {
                    self.phase = Phase::NearWait {
                        attempt,
                        loc,
                        first,
                    };
                    return Some(Step::Wait);
                }
            }
            Phase::NearApproach(attempt, loc) => {
                if loc.distance <= 1 {
                    // Already beside it: no walk, straight to the retry.
                    return self.retry_beside(attempt, Some(loc), cx);
                }
                let name = self.name.clone();
                let (line, target) = match bank_stand(loc.tile) {
                    Some(stand) => (
                        format!(
                            "booth didn't open — stepping to the bank counter at ({}, {})",
                            stand.x, stand.z
                        ),
                        stand,
                    ),
                    None => (
                        format!("no reachable tile beside '{name}' yet — closing in"),
                        loc.tile,
                    ),
                };
                cx.clock().arm(STEP_WALK_MS);
                cx.emit(walk_near(target, 1));
                self.say(line, Phase::NearWalk(attempt, Some(loc), target));
            }
            Phase::NearWalk(attempt, loc, target) => {
                if crate::load::reach_query::arrived(target, 1) || cx.clock().bound_reached() {
                    return self.retry_beside(attempt, loc, cx);
                }
                self.phase = Phase::NearWalk(attempt, loc, target);
                return Some(Step::Wait);
            }
            Phase::Continue(attempt, mut press, after) => match press.poll(cx) {
                None => {
                    self.phase = Phase::Continue(attempt, press, after);
                    return Some(Step::Wait);
                }
                Some(true) => {
                    cx.clock().arm(DIALOG_OPEN_MS);
                    self.phase = Phase::ContinueOpen(attempt, after);
                }
                Some(false) => {
                    let next = after_continue(attempt, after);
                    self.say("bank: failed to Continue the object dialogue".into(), next);
                }
            },
            Phase::ContinueOpen(attempt, after) => {
                if BankView::now().open {
                    self.phase = Phase::Ready(Ready { armed: false });
                } else if cx.clock().bound_reached() {
                    let next = after_continue(attempt, after);
                    self.say(
                        "bank: object dialogue completed without opening the bank".into(),
                        next,
                    );
                } else {
                    self.phase = Phase::ContinueOpen(attempt, after);
                    return Some(Step::Wait);
                }
            }
            Phase::Ready(mut ready) => match ready.step(cx) {
                Readied::Wait => {
                    self.phase = Phase::Ready(ready);
                    return Some(Step::Wait);
                }
                Readied::Done(ok, line) => match line {
                    Some(line) => self.say(line, Phase::Finish(ok)),
                    None => return Some(Step::Done(ok)),
                },
            },
            Phase::Finish(ok) => return Some(Step::Done(ok)),
        }
        None
    }

    fn open_booth(&mut self, cx: &mut Cx<'_>) -> Option<Step<bool>> {
        match <BankOpen as Family>::begin(BankOpenArgs::nearest_booth(), cx) {
            Begin::Run(open) => {
                self.phase = Phase::BoothOpen(Box::new(open));
                Some(Step::Wait)
            }
            Begin::Done(ok) => Some(Step::Done(ok)),
            Begin::Refuse(_) => Some(Step::Done(false)),
        }
    }

    /// Beside the loc: interact with the adjacent one (or the one found).
    fn retry_beside(
        &mut self,
        attempt: u32,
        found: Option<Loc>,
        cx: &mut Cx<'_>,
    ) -> Option<Step<bool>> {
        let Some(loc) = usable_loc(&self.name, true).or(found) else {
            self.phase = Phase::NearFind(attempt + 1);
            return None;
        };
        match pick(&loc.actions, &self.op) {
            Some(action) => {
                cx.clock().arm(ADJACENT_MS);
                cx.emit(interact(&loc, &action));
                self.phase = Phase::NearWait {
                    attempt,
                    loc,
                    first: false,
                };
                Some(Step::Wait)
            }
            None => {
                self.phase = Phase::NearFind(attempt + 1);
                None
            }
        }
    }
}

fn after_continue(attempt: u32, after: AfterContinue) -> Phase {
    match after {
        AfterContinue::Approach(loc) => Phase::NearApproach(attempt, loc),
        AfterContinue::Next => Phase::NearFind(attempt + 1),
    }
}

/// Write a queued log line (when the caller gave `log`) before deciding.
fn log_first(
    line: &mut Option<String>,
    hook: Option<usize>,
    cx: &mut Cx<'_>,
) -> Option<Step<bool>> {
    if let Some(Reply::Threw(thrown)) = cx.reply() {
        return Some(Step::Fail(thrown));
    }
    let line = line.take()?;
    let hook = hook.filter(|hook| cx.has(*hook))?;
    Some(Step::Call(Call {
        hook,
        args: vec![json!(line)],
    }))
}

impl Family for BankAccess {
    const NAME: &'static str = "bank_access";
    /// One bank open at a time, whichever surface started it: a newer
    /// open (this, `openNpcAccess`, `openBooth` …) supersedes this row.
    const EXCLUSIVE: bool = true;
    const EXCLUSIVE_GROUP: &'static str = BankOpen::NAME;
    const CALLBACKS: &'static [&'static str] = &["log"];
    /// Frozen calls `log?.()` without awaiting it.
    const AWAIT_CALLBACKS: bool = false;
    /// The first verb joins the caller's tick, after any log line.
    const KICK_ON_START: bool = true;
    type Args = AccessArgs;
    type Output = bool;

    fn begin(args: AccessArgs, _cx: &mut Cx<'_>) -> Begin<Self> {
        Begin::Run(Self::new(args, Some(LOG)))
    }

    fn step(&mut self, cx: &mut Cx<'_>) -> Step<bool> {
        self.run(cx)
    }
}

#[derive(Clone, Deserialize)]
pub(crate) struct NpcAccessArgs {
    name: String,
    op: String,
    choose: String,
}

enum NpcPhase {
    Attempt(u32),
    WaitDialog(u32),
    Delay(u32, u64),
    Drive(u32, u32),
    Press(u32, u32, Press),
    Settle(u32),
    AfterLoop,
    Ready(Ready),
    Finish(bool),
}

const NPC_ATTEMPTS: u32 = 3;
const NPC_PRESSES: u32 = 12;

/// One awaited frozen `Bank.openNpcAccess(access, log)`.
pub(crate) struct NpcAccess {
    name: String,
    op: String,
    choose: String,
    phase: NpcPhase,
    line: Option<String>,
    log_hook: Option<usize>,
}

struct Banker {
    name: String,
    action: String,
    index: i32,
}

/// `Npcs.query().name(name).action(op).nearest()`.
fn banker(name: &str, op: &str) -> Option<Banker> {
    let wanted = name.trim().to_lowercase();
    let op = op.to_lowercase();
    observed::with(|scene| {
        scene
            .since_login()
            .npcs()?
            .iter()
            .filter(|npc| {
                npc.name
                    .as_deref()
                    .is_some_and(|got| got.trim().to_lowercase() == wanted)
            })
            .filter_map(|npc| {
                let action = npc
                    .actions
                    .iter()
                    .find(|a| present(a) && a.to_lowercase() == op)?;
                Some((
                    npc.distance,
                    Banker {
                        name: npc.name_or_empty().to_string(),
                        action: action.to_string(),
                        index: npc.index,
                    },
                ))
            })
            .min_by_key(|(distance, _)| *distance)
            .map(|(_, banker)| banker)
    })
}

impl NpcAccess {
    /// One `openNpcAccess`; `log_hook` is the caller's `log` hook, if any.
    pub(crate) fn new(args: NpcAccessArgs, log_hook: Option<usize>) -> Self {
        Self {
            name: args.name,
            op: args.op,
            choose: args.choose,
            phase: NpcPhase::Attempt(0),
            line: None,
            log_hook,
        }
    }

    /// Drive the opener; it reads its own log calls' replies.
    pub(crate) fn run(&mut self, cx: &mut Cx<'_>) -> Step<bool> {
        loop {
            if let Some(step) = log_first(&mut self.line, self.log_hook, cx) {
                return step;
            }
            if let Some(step) = self.decide(cx) {
                return step;
            }
        }
    }

    fn say(&mut self, line: String, next: NpcPhase) {
        self.line = Some(line);
        self.phase = next;
    }

    fn decide(&mut self, cx: &mut Cx<'_>) -> Option<Step<bool>> {
        let phase = std::mem::replace(&mut self.phase, NpcPhase::Finish(false));
        match phase {
            NpcPhase::Attempt(attempt) => {
                if attempt >= NPC_ATTEMPTS || BankView::now().open {
                    self.phase = NpcPhase::AfterLoop;
                    return None;
                }
                let chat = Chat::now();
                if chat.open() || chat.can_continue {
                    self.phase = NpcPhase::Drive(attempt, 0);
                    return None;
                }
                match banker(&self.name, &self.op) {
                    None => {
                        let name = self.name.clone();
                        let until = posted_tick().saturating_add(1);
                        self.say(
                            format!("no '{name}' in the scene to bank with"),
                            NpcPhase::Delay(attempt, until),
                        );
                    }
                    Some(banker) => {
                        cx.clock().arm(NPC_DIALOG_MS);
                        cx.emit(InteractReq::Npc {
                            name: banker.name,
                            action: banker.action,
                            index: Some(banker.index),
                        });
                        self.phase = NpcPhase::WaitDialog(attempt);
                        return Some(Step::Wait);
                    }
                }
            }
            NpcPhase::WaitDialog(attempt) => {
                let chat = Chat::now();
                if chat.open() || chat.can_continue || BankView::now().open {
                    self.phase = NpcPhase::Drive(attempt, 0);
                } else if cx.clock().bound_reached() {
                    let name = self.name.clone();
                    self.say(
                        format!("'{name}' never opened a dialogue"),
                        NpcPhase::Attempt(attempt + 1),
                    );
                } else {
                    self.phase = NpcPhase::WaitDialog(attempt);
                    return Some(Step::Wait);
                }
            }
            NpcPhase::Delay(attempt, until) => {
                if posted_tick() < until {
                    self.phase = NpcPhase::Delay(attempt, until);
                    return Some(Step::Wait);
                }
                self.phase = NpcPhase::Attempt(attempt + 1);
            }
            NpcPhase::Drive(attempt, presses) => {
                if presses >= NPC_PRESSES || BankView::now().open {
                    cx.clock().arm(NPC_OPEN_MS);
                    self.phase = NpcPhase::Settle(attempt);
                    return None;
                }
                let chat = Chat::now();
                let choose = self.choose.to_lowercase();
                let option = chat
                    .options
                    .iter()
                    .position(|option| option.to_lowercase().contains(&choose));
                let press = match option {
                    Some(index) => Press::send(
                        InteractReq::Answer {
                            option: i32::try_from(index + 1).unwrap_or(i32::MAX),
                        },
                        true,
                        &chat,
                        cx,
                    ),
                    None if chat.can_continue => {
                        Press::send(InteractReq::ContinueDialog, false, &chat, cx)
                    }
                    None => {
                        cx.clock().arm(NPC_OPEN_MS);
                        self.phase = NpcPhase::Settle(attempt);
                        return None;
                    }
                };
                self.phase = NpcPhase::Press(attempt, presses, press);
                return Some(Step::Wait);
            }
            NpcPhase::Press(attempt, presses, mut press) => {
                if press.poll(cx).is_none() {
                    self.phase = NpcPhase::Press(attempt, presses, press);
                    return Some(Step::Wait);
                }
                self.phase = NpcPhase::Drive(attempt, presses + 1);
            }
            NpcPhase::Settle(attempt) => {
                if BankView::now().open || cx.clock().bound_reached() {
                    self.phase = NpcPhase::Attempt(attempt + 1);
                } else {
                    self.phase = NpcPhase::Settle(attempt);
                    return Some(Step::Wait);
                }
            }
            NpcPhase::AfterLoop => {
                let next = NpcPhase::Ready(Ready { armed: false });
                if BankView::now().open {
                    self.phase = next;
                } else {
                    let name = self.name.clone();
                    self.say(format!("could not get {name} to open the bank"), next);
                }
            }
            NpcPhase::Ready(mut ready) => match ready.step(cx) {
                Readied::Wait => {
                    self.phase = NpcPhase::Ready(ready);
                    return Some(Step::Wait);
                }
                Readied::Done(ok, line) => match line {
                    Some(line) => self.say(line, NpcPhase::Finish(ok)),
                    None => return Some(Step::Done(ok)),
                },
            },
            NpcPhase::Finish(ok) => return Some(Step::Done(ok)),
        }
        None
    }
}

impl Family for NpcAccess {
    const NAME: &'static str = "bank_npc_access";
    /// One bank open at a time, whichever surface started it: a newer
    /// open (this, `openNpcAccess`, `openBooth` …) supersedes this row.
    const EXCLUSIVE: bool = true;
    const EXCLUSIVE_GROUP: &'static str = BankOpen::NAME;
    const CALLBACKS: &'static [&'static str] = &["log"];
    /// Frozen calls `log?.()` without awaiting it.
    const AWAIT_CALLBACKS: bool = false;
    /// The first verb joins the caller's tick, after any log line.
    const KICK_ON_START: bool = true;
    type Args = NpcAccessArgs;
    type Output = bool;

    fn begin(args: NpcAccessArgs, _cx: &mut Cx<'_>) -> Begin<Self> {
        Begin::Run(Self::new(args, Some(LOG)))
    }

    fn step(&mut self, cx: &mut Cx<'_>) -> Step<bool> {
        self.run(cx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::load::callback_v8::HeldCallback;
    use crate::machine::{self, Called, Js, Outcome, Pending, Started, Take};
    use crate::observed::{EntityRow, Ops, SceneRow, Tile};
    use serde_json::Value;
    use std::rc::Rc;

    /// No `log` hook is passed: lines are skipped, verbs are what is seen.
    struct NoJs;

    impl Js for NoJs {
        fn queue_len(&mut self) -> usize {
            0
        }

        fn call(&mut self, _hook: Option<&HeldCallback>, _args: &[Value]) -> Called {
            panic!("no log hook was passed");
        }

        fn poll(&mut self, _pending: &Pending) -> Option<Reply> {
            panic!("no log hook was passed");
        }

        fn claimed(&mut self) -> bool {
            false
        }
    }

    fn ops(actions: &[&str]) -> Ops {
        actions.iter().map(|action| Rc::from(*action)).collect()
    }

    fn loc(id: i32, name: &str, x: i32, z: i32, distance: i32, actions: &[&str]) -> SceneRow {
        SceneRow {
            id,
            name: Some(Rc::from(name)),
            x,
            z,
            level: 0,
            distance,
            actions: ops(actions),
        }
    }

    fn banker() -> EntityRow {
        EntityRow {
            index: 5,
            id: 494,
            name: Some(Rc::from("Banker")),
            distance: 2,
            actions: ops(&["Talk-to", "Bank"]),
            ..EntityRow::default()
        }
    }

    fn at(x: i32, z: i32) -> Tile {
        Tile { x, z, level: 0 }
    }

    fn start(family: &str, args: Value) -> machine::Handle {
        match machine::start(family, args, Vec::new(), 0) {
            Started::Running(handle) => handle,
            other => panic!("expected a running row, got {other:?}"),
        }
    }

    fn tick() -> Vec<InteractReq> {
        machine::step(&mut NoJs);
        machine::merge_ops(Vec::new())
    }

    fn reset() {
        machine::on_reset();
        observed::on_reset();
    }

    fn interact(loc: &SceneRow, action: &str) -> InteractReq {
        InteractReq::Loc {
            x: loc.x,
            z: loc.z,
            level: 0,
            action: action.into(),
            id: Some(loc.id),
        }
    }

    fn talk() -> InteractReq {
        InteractReq::Npc {
            name: "Banker".into(),
            action: "Bank".into(),
            index: Some(5),
        }
    }

    fn done(handle: machine::Handle, ok: bool) {
        assert_eq!(
            machine::take(handle),
            Take::Settled(Outcome::Done(Value::Bool(ok)))
        );
    }

    /// `openFirst`: the closed loc is opened first, then the named access
    /// loc (a different id on the same tile) is used.
    #[test]
    fn open_first_opens_the_closer_then_banks_at_the_opened_loc() {
        reset();
        let closed = loc(1, "Closed chest", 3382, 3269, 1, &["Open"]);
        let open = loc(2, "Open chest", 3382, 3269, 1, &["Bank"]);
        observed::post(1, |post| {
            post.session(true)
                .here(at(3382, 3268))
                .locs(vec![closed.clone()]);
        });
        let handle = start(
            "bank_access",
            json!({ "name": "Open chest", "op": "Bank",
                    "open_first": { "name": "Closed chest", "op": "Open" } }),
        );
        assert_eq!(tick(), vec![interact(&closed, "Open")]);
        observed::post(2, |post| {
            post.locs(vec![open.clone()]);
        });
        assert_eq!(tick(), vec![interact(&open, "Bank")]);
        observed::post(3, |post| {
            post.bank_open(true).bank_loaded(true);
        });
        assert!(tick().is_empty());
        done(handle, true);
    }

    /// No opener in the scene: three one-tick attempts, then `false`.
    #[test]
    fn open_first_gives_up_after_three_attempts() {
        reset();
        observed::post(1, |post| {
            post.session(true).here(at(3382, 3268)).locs(Vec::new());
        });
        let handle = start(
            "bank_access",
            json!({ "name": "Open chest", "op": "Bank",
                    "open_first": { "name": "Closed chest", "op": "Open" } }),
        );
        for n in 1..=3 {
            observed::post(n, |_| {});
            assert!(tick().is_empty());
            assert_eq!(machine::take(handle), Take::Pending, "attempt {n}");
        }
        observed::post(4, |_| {});
        assert!(tick().is_empty());
        done(handle, false);
    }

    /// An object dialogue (Continue) after the interact is pressed through,
    /// and the bank opening after it answers true.
    #[test]
    fn an_object_dialogue_is_continued_into_the_bank() {
        reset();
        let chest = loc(2693, "Shantay chest", 3309, 3120, 1, &["Open"]);
        observed::post(1, |post| {
            post.session(true)
                .here(at(3308, 3120))
                .locs(vec![chest.clone()]);
        });
        let handle = start(
            "bank_access",
            json!({ "name": "Shantay chest", "op": "Open" }),
        );
        assert_eq!(tick(), vec![interact(&chest, "Open")]);
        observed::post(2, |post| {
            post.chat_modal_id(4882).chat_continue(true);
        });
        assert_eq!(tick(), vec![InteractReq::ContinueDialog]);
        observed::post(3, |post| {
            post.chat_modal_id(-1).chat_continue(false);
        });
        assert!(tick().is_empty(), "acknowledged: one settle tick");
        observed::post(4, |post| {
            post.bank_open(true).bank_loaded(true);
        });
        assert!(tick().is_empty());
        done(handle, true);
    }

    /// A far loc that did not open: step beside it (no reach view: the loc
    /// tile, radius 1), then interact again from there.
    #[test]
    fn an_unopened_far_loc_is_retried_from_beside_it() {
        reset();
        let far = loc(2693, "Shantay chest", 3303, 3120, 3, &["Open"]);
        observed::post(1, |post| {
            post.session(true)
                .here(at(3300, 3120))
                .locs(vec![far.clone()]);
        });
        let handle = start(
            "bank_access",
            json!({ "name": "Shantay chest", "op": "Open" }),
        );
        assert_eq!(tick(), vec![interact(&far, "Open")]);
        machine::tests::expire_deadlines();
        assert_eq!(
            tick(),
            vec![InteractReq::WalkNear {
                x: 3303,
                z: 3120,
                level: 0,
                radius: 1,
                allow_teleports: false,
                allow_wilderness: true,
                allow_bank_fetch: true,
                request_id: 0,
            }]
        );
        let beside = loc(2693, "Shantay chest", 3303, 3120, 1, &["Open"]);
        observed::post(2, |post| {
            post.here(at(3302, 3120)).locs(vec![beside.clone()]);
        });
        assert_eq!(tick(), vec![interact(&beside, "Open")]);
        assert_eq!(machine::take(handle), Take::Pending);
    }

    /// Frozen `openedReady`: an open bank whose list never posts still
    /// answers open once the 4 s wait lapses.
    #[test]
    fn opened_ready_answers_open_after_its_wait() {
        reset();
        observed::post(1, |post| {
            post.session(true)
                .here(at(3308, 3120))
                .bank_open(true)
                .bank_loaded(false);
        });
        let handle = start(
            "bank_access",
            json!({ "name": "Shantay chest", "op": "Open" }),
        );
        assert!(tick().is_empty());
        assert_eq!(machine::take(handle), Take::Pending, "inside the wait");
        machine::tests::expire_deadlines();
        assert!(tick().is_empty());
        done(handle, true);
    }

    /// The banker: absent for one attempt, then talked to; a silent talk is
    /// retried; the dialogue is continued, then the access option answered.
    #[test]
    fn npc_access_retries_then_continues_and_answers_the_option() {
        reset();
        observed::post(1, |post| {
            post.session(true).here(at(2852, 2952)).npcs(Vec::new());
        });
        let handle = start(
            "bank_npc_access",
            json!({ "name": "Banker", "op": "Bank", "choose": "access my bank" }),
        );
        assert!(tick().is_empty(), "no banker: one tick");
        observed::post(2, |post| {
            post.npcs(vec![banker()]);
        });
        assert_eq!(tick(), vec![talk()]);
        machine::tests::expire_deadlines();
        assert_eq!(tick(), vec![talk()], "no dialogue opened: the next attempt");
        observed::post(3, |post| {
            post.chat_modal_id(1).chat_continue(true);
        });
        assert_eq!(tick(), vec![InteractReq::ContinueDialog]);
        observed::post(4, |post| {
            post.chat_modal_id(2)
                .chat_continue(false)
                .chat_options(vec![
                    "Nothing thanks".into(),
                    "I'd like to access my bank account".into(),
                ]);
        });
        assert!(tick().is_empty(), "acknowledged: one settle tick");
        observed::post(5, |_| {});
        assert_eq!(tick(), vec![InteractReq::Answer { option: 2 }]);
        assert_eq!(machine::take(handle), Take::Pending);
    }

    /// No banker for all three attempts: `false`.
    #[test]
    fn npc_access_gives_up_after_three_attempts() {
        reset();
        observed::post(1, |post| {
            post.session(true).here(at(2852, 2952)).npcs(Vec::new());
        });
        let handle = start(
            "bank_npc_access",
            json!({ "name": "Banker", "op": "Bank", "choose": "access" }),
        );
        for n in 1..=3 {
            observed::post(n, |_| {});
            assert!(tick().is_empty());
            assert_eq!(machine::take(handle), Take::Pending, "attempt {n}");
        }
        observed::post(4, |_| {});
        assert!(tick().is_empty());
        done(handle, false);
    }

    /// Every bank open shares one exclusive group: a newer `Bank.openBooth`
    /// supersedes a running access opener, as it did the shim's
    /// `openBooth` it used to call.
    #[test]
    fn a_newer_bank_open_supersedes_an_access_opener() {
        reset();
        let chest = loc(2693, "Shantay chest", 3309, 3120, 1, &["Open"]);
        observed::post(1, |post| {
            post.session(true)
                .here(at(3308, 3120))
                .locs(vec![chest.clone()]);
        });
        let access = start(
            "bank_access",
            json!({ "name": "Shantay chest", "op": "Open" }),
        );
        assert_eq!(tick(), vec![interact(&chest, "Open")]);
        observed::post(2, |post| {
            post.bank_open(true).bank_loaded(false);
        });
        let _open = start("bank_open", json!({ "mode": "open-booth" }));
        assert_eq!(
            machine::take(access),
            Take::Settled(Outcome::Aborted(machine::AbortReason::Superseded))
        );
    }
}
