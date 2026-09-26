//! The hunt families (fence F09) on the step-machine host.
//!
//! Each hunt module (`hunt_fight`, `hunt_lair`, `hunt_leave`, `hunt_key`,
//! `hunt_cell`, `hunt_bank`) owns its policy as an effect stepper: given
//! the site, the script's current host values and the last reply, `next`
//! returns one effect. This module runs that stepper as a
//! [`crate::machine`] family: it reads the script's getters and the site's
//! own `inArea` predicate through [`Cx::ask`], emits the game ops and walks
//! itself, calls the script's async hooks (`eatOnce`, `armSpecial`,
//! `sustain`, a bank run's own `leave`) through [`Step::Call`], waits out
//! delays in ticks, and runs a nested hunt (leave, key), teleport or bank
//! open as a child in the same row. JavaScript starts a family and awaits
//! one completion.
//!
//! - **Sessions** (`hunt-fight`, `hunt-hold`, `hunt-retreat`,
//!   `hunt-walkspot`, `hunt-enter`): the Task classes. The site crosses
//!   once, at [`begin_session`], and the class keeps the token; `validate`
//!   is a synchronous helper and each `execute` is one machine run on the
//!   token. After ResetSession the token's policy state starts fresh on
//!   its next use (no JS re-bind); a run the stepper aborts leaves a fresh
//!   token behind the same way.
//! - **Runs** (`hunt-leave`, `hunt-key`, `hunt-cell`, `hunt-bank`): one
//!   token per run, ended with it.
//!
//! Every hunt family holds the same [`HOOKS`], so a child calls the
//! script by the parent's hook indices.

mod acquire;
mod teleport_out;
mod wait;

pub(crate) use acquire::Acquire;
#[allow(unused_imports)]
pub(crate) use teleport_out::TeleportOutArgs;
pub(crate) use teleport_out::{escape_shortfall, TeleportOut};
pub(crate) use wait::WaitFed;
#[allow(unused_imports)]
pub(crate) use wait::WaitFedArgs;

use crate::hunt_fight::{Area, Tile};
use crate::machine::{AbortReason, Begin, Call, Cx, Ended, Family, Reply, Step, Thrown};
use crate::observed;
use crate::shim::InteractReq;
use serde::Deserialize;
use serde_json::{json, Value};
use std::any::Any;
use std::cell::RefCell;
use std::collections::HashMap;

/// The script hooks every hunt family holds, by `hooks` key.
pub(crate) const HOOKS: &[&str] = &[
    "log",
    "vlog",
    "setStatus",
    "eatOnce",
    "armSpecial",
    "countBurial",
    "setSafespotIndex",
    "setTarget",
    "sustain",
    "pickWeapon",
    "parkFor",
    "countBankTrip",
    "leave",
    "died",
    "targetIdx",
    "hpFraction",
    "panicHp",
    "retreatHp",
    "hasFood",
    "needEat",
    "style",
    "safespotIndex",
    "buryBones",
    "boneName",
    "shieldReady",
    "parked",
    "leaveByWalk",
    "foodName",
    "foodWithdraw",
    "weaponName",
    "ammoName",
    "spellName",
    "keepExtra",
    "inArea",
    "cond",
    "countKill",
];

/// Indices into [`HOOKS`].
pub(crate) mod hook {
    pub(crate) const LOG: usize = 0;
    pub(crate) const VLOG: usize = 1;
    pub(crate) const SET_STATUS: usize = 2;
    pub(crate) const EAT_ONCE: usize = 3;
    pub(crate) const ARM_SPECIAL: usize = 4;
    pub(crate) const COUNT_BURIAL: usize = 5;
    pub(crate) const SET_SAFESPOT: usize = 6;
    pub(crate) const SET_TARGET: usize = 7;
    pub(crate) const SUSTAIN: usize = 8;
    pub(crate) const PICK_WEAPON: usize = 9;
    pub(crate) const PARK_FOR: usize = 10;
    pub(crate) const COUNT_BANK_TRIP: usize = 11;
    pub(crate) const LEAVE: usize = 12;
    pub(crate) const DIED: usize = 13;
    pub(crate) const TARGET_IDX: usize = 14;
    pub(crate) const HP_FRACTION: usize = 15;
    pub(crate) const PANIC_HP: usize = 16;
    pub(crate) const RETREAT_HP: usize = 17;
    pub(crate) const HAS_FOOD: usize = 18;
    pub(crate) const NEED_EAT: usize = 19;
    pub(crate) const STYLE: usize = 20;
    pub(crate) const SAFESPOT_INDEX: usize = 21;
    pub(crate) const BURY_BONES: usize = 22;
    pub(crate) const BONE_NAME: usize = 23;
    pub(crate) const SHIELD_READY: usize = 24;
    pub(crate) const PARKED: usize = 25;
    pub(crate) const LEAVE_BY_WALK: usize = 26;
    pub(crate) const FOOD_NAME: usize = 27;
    pub(crate) const FOOD_WITHDRAW: usize = 28;
    pub(crate) const WEAPON_NAME: usize = 29;
    pub(crate) const AMMO_NAME: usize = 30;
    pub(crate) const SPELL_NAME: usize = 31;
    pub(crate) const KEEP_EXTRA: usize = 32;
    pub(crate) const IN_AREA: usize = 33;
    pub(crate) const COND: usize = 34;
    pub(crate) const COUNT_KILL: usize = 35;
}

/// Effects one step may take before it yields the tick.
const EFFECTS_PER_STEP: usize = 64;

/// Predicate answers one site keeps; past this the cache starts over.
const AREA_CACHE: usize = 8192;

/// The script side a hunt reads: a machine step ([`Cx::ask`]) or a
/// synchronous helper (`load/hunt_v8.rs`).
pub(crate) trait Host {
    /// The hook was present (not `undefined`/`null`).
    fn has(&mut self, hook: usize) -> bool;
    /// Call it now. `Err` has already recorded the throw or the stop.
    fn ask(&mut self, hook: usize, args: &[Value]) -> Result<Value, Ended>;
}

struct CxHost<'c, 'a>(&'c mut Cx<'a>);

impl Host for CxHost<'_, '_> {
    fn has(&mut self, hook: usize) -> bool {
        self.0.has(hook)
    }

    fn ask(&mut self, hook: usize, args: &[Value]) -> Result<Value, Ended> {
        self.0.ask(hook, args)
    }
}

/// JS truthiness of a settled value.
pub(crate) fn truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|n| n != 0.0 && !n.is_nan()),
        Value::String(s) => !s.is_empty(),
        Value::Array(_) | Value::Object(_) => true,
    }
}

/// `!!hook()`, or `absent` when the script has no such hook.
pub(crate) fn flag(host: &mut dyn Host, hook: usize, absent: bool) -> Result<bool, Ended> {
    if !host.has(hook) {
        return Ok(absent);
    }
    host.ask(hook, &[]).map(|v| truthy(&v))
}

/// `hook() === true`, false when absent.
pub(crate) fn strict_true(host: &mut dyn Host, hook: usize) -> Result<bool, Ended> {
    if !host.has(hook) {
        return Ok(false);
    }
    host.ask(hook, &[]).map(|v| v == Value::Bool(true))
}

/// A number answer; `absent` when the hook is absent or not a number.
pub(crate) fn number(host: &mut dyn Host, hook: usize, absent: f64) -> Result<f64, Ended> {
    if !host.has(hook) {
        return Ok(absent);
    }
    host.ask(hook, &[]).map(|v| v.as_f64().unwrap_or(absent))
}

/// A string answer; `absent` when the hook is absent or not a string.
pub(crate) fn text(host: &mut dyn Host, hook: usize, absent: &str) -> Result<String, Ended> {
    if !host.has(hook) {
        return Ok(absent.to_string());
    }
    host.ask(hook, &[]).map(|v| {
        v.as_str()
            .map_or_else(|| absent.to_string(), str::to_string)
    })
}

/// A whole-number answer, `None` for `null`/absent/non-numbers.
pub(crate) fn index(host: &mut dyn Host, hook: usize) -> Result<Option<i32>, Ended> {
    if !host.has(hook) {
        return Ok(None);
    }
    host.ask(hook, &[]).map(|v| {
        v.as_i64()
            .or_else(|| v.as_f64().filter(|n| n.fract() == 0.0).map(|n| n as i64))
            .and_then(|n| i32::try_from(n).ok())
    })
}

/// A list of strings (non-strings skipped), empty when absent.
pub(crate) fn names(host: &mut dyn Host, hook: usize) -> Result<Vec<String>, Ended> {
    if !host.has(hook) {
        return Ok(Vec::new());
    }
    host.ask(hook, &[]).map(|v| {
        v.as_array()
            .map(|rows| {
                rows.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default()
    })
}

/// One hunt stepper as a family.
pub(crate) trait Kind: 'static {
    const NAME: &'static str;
    /// A Task class: the token outlives each run.
    const SESSION: bool;
    /// A run settles `true`/`false` (else `undefined`).
    const BOOLEAN: bool;
    /// Scene `walk-to` is acknowledged `{ queued: true }`.
    const WALK_TO_ACK: bool = false;
    /// World walks allow the wilderness and bank fetch (Hold, WalkToSpot).
    const WORLD_WALK: bool = false;
    /// The area predicate is asked for every scene npc too (Fight).
    const NPC_AREA: bool = false;
    type Proj: 'static;

    fn parse(site: &Value) -> Self::Proj;
    fn area(proj: &mut Self::Proj) -> &mut Area;
    /// Read the script's current host values into `proj`.
    fn refresh(_proj: &mut Self::Proj, _host: &mut dyn Host) -> Result<(), Ended> {
        Ok(())
    }
    fn mint() -> u64;
    /// A token whose state ResetSession dropped starts fresh.
    fn ensure(token: u64);
    /// Replace the token's state with a fresh one.
    fn renew(token: u64);
    fn next(token: u64, proj: &Self::Proj, reply: Option<&Value>) -> Value;
    fn end(_token: u64) {}
    fn validate(_token: u64, _proj: &Self::Proj) -> bool {
        false
    }
}

type SessionSites = HashMap<(&'static str, u64), Box<dyn Any>>;

thread_local! {
    /// Session sites by family and token, parsed once at begin. They
    /// outlive ResetSession, as the Task instances holding the tokens do.
    static SITES: RefCell<SessionSites> = RefCell::new(HashMap::new());
}

/// A Task class's begin: mint the token and keep its site.
pub(crate) fn begin_session<K: Kind>(site: &Value) -> u64 {
    let token = K::mint();
    let proj: Box<dyn Any> = Box::new(K::parse(site));
    SITES.with(|sites| sites.borrow_mut().insert((K::NAME, token), proj));
    token
}

fn take_site<K: Kind>(token: u64) -> Option<K::Proj> {
    SITES
        .with(|sites| sites.borrow_mut().remove(&(K::NAME, token)))
        .and_then(|proj| proj.downcast::<K::Proj>().ok())
        .map(|proj| *proj)
}

fn put_site<K: Kind>(token: u64, proj: K::Proj) {
    let proj: Box<dyn Any> = Box::new(proj);
    SITES.with(|sites| sites.borrow_mut().insert((K::NAME, token), proj));
}

/// Stop: the sites and the fee proof go with the isolate thread's scripts.
pub(crate) fn on_stop() {
    SITES.with(|sites| sites.borrow_mut().clear());
    crate::hunt_lair::on_stop();
}

/// A session's synchronous read (`validate`, `blocksLoot`): refresh the
/// host values, then `read` the policy. `None`: no such token.
pub(crate) fn with_session<K: Kind, R>(
    token: u64,
    host: &mut dyn Host,
    read: impl FnOnce(u64, &K::Proj) -> R,
) -> Option<Result<R, Ended>> {
    let mut proj = take_site::<K>(token)?;
    K::ensure(token);
    let out = prepare::<K>(&mut proj, host).map(|()| read(token, &proj));
    put_site::<K>(token, proj);
    Some(out)
}

/// Host values, then the area answers for the tiles this step reads.
fn prepare<K: Kind>(proj: &mut K::Proj, host: &mut dyn Host) -> Result<(), Ended> {
    K::refresh(proj, host)?;
    fill_area(K::area(proj), host, K::NPC_AREA)
}

fn fill_area(area: &mut Area, host: &mut dyn Host, npcs: bool) -> Result<(), Ended> {
    if !host.has(hook::IN_AREA) {
        area.known = None;
        return Ok(());
    }
    let tiles: Vec<Tile> = observed::with(|scene| {
        let session = scene.since_login();
        let mut tiles: Vec<Tile> = session.here().map(Tile::from).into_iter().collect();
        if npcs {
            for n in session.npcs().map(Vec::as_slice).unwrap_or_default() {
                tiles.push(Tile {
                    x: n.x,
                    z: n.z,
                    level: n.level,
                });
                tiles.push(Tile {
                    x: n.nx,
                    z: n.nz,
                    level: n.level,
                });
            }
        }
        tiles
    });
    let known = area.known.get_or_insert_with(HashMap::new);
    if known.len() > AREA_CACHE {
        known.clear();
    }
    for tile in tiles {
        if known.contains_key(&tile) {
            continue;
        }
        let point = json!({ "x": tile.x, "z": tile.z, "level": tile.level });
        let inside = truthy(&host.ask(hook::IN_AREA, &[point])?);
        known.insert(tile, inside);
    }
    Ok(())
}

/// Start arguments: a session's token, or a run's site (and bank opts).
#[derive(Deserialize)]
pub(crate) struct HuntArgs {
    #[serde(default)]
    token: Option<u64>,
    #[serde(default)]
    site: Option<Value>,
}

/// What a [`Step::Call`] this row made is for.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Waiting {
    Nothing,
    /// `eatOnce()`: the stepper reads `{ eatOk }`.
    Eat,
    /// `armSpecial()` / `sustain()`: awaited, the value unused.
    Plain,
    /// A bank run's own `leave()`: `{ left }`.
    Leave,
}

/// A nested machine the row runs before its stepper resumes.
enum Child {
    Leave(Box<Hunt<crate::hunt_leave::Leave>>),
    Key(Box<Hunt<crate::hunt_key::Key>>),
    Bank(Box<Hunt<crate::hunt_bank::Bank>>),
    Cell(Box<Hunt<crate::hunt_cell::Cell>>),
    Teleport(crate::teleport::Teleport),
    BankOpen(crate::bank_open::BankOpen),
}

/// One hunt row.
pub(crate) struct Hunt<K: Kind> {
    token: u64,
    proj: Option<K::Proj>,
    /// The run's site, for a child hunt to parse.
    site: Value,
    reply: Option<Value>,
    /// Ticks still to wait before the next effect.
    sleep: u32,
    waiting: Waiting,
    child: Option<Child>,
    ended: bool,
}

impl<K: Kind> Family for Hunt<K> {
    const NAME: &'static str = K::NAME;
    const CALLBACKS: &'static [&'static str] = HOOKS;
    /// The first effects join the caller's tick, as the awaited JS loop's did.
    const KICK_ON_START: bool = true;
    type Args = HuntArgs;
    type Output = Value;

    fn begin(args: HuntArgs, _cx: &mut Cx<'_>) -> Begin<Self> {
        if K::SESSION {
            let Some(token) = args.token else {
                return Begin::Refuse("missing token".into());
            };
            let Some(proj) = take_site::<K>(token) else {
                return Begin::Refuse("unknown token".into());
            };
            K::ensure(token);
            return Begin::Run(Self::new(token, proj, Value::Null));
        }
        let Some(site) = args.site.filter(Value::is_object) else {
            return Begin::Refuse("missing site".into());
        };
        Begin::Run(Self::run(site))
    }

    fn step(&mut self, cx: &mut Cx<'_>) -> Step<Value> {
        match self.drive(cx) {
            Ok(step) => step,
            // The host ends the row; the stepper's state goes with it.
            Err(Ended) => {
                self.abort(AbortReason::Terminated);
                Step::Wait
            }
        }
    }

    fn abort(&mut self, why: AbortReason) {
        if let Some(child) = self.child.as_mut() {
            match child {
                Child::Leave(hunt) => hunt.abort(why),
                Child::Key(hunt) => hunt.abort(why),
                Child::Bank(hunt) => hunt.abort(why),
                Child::Cell(hunt) => hunt.abort(why),
                Child::Teleport(_) | Child::BankOpen(_) => {}
            }
        }
        self.finish();
    }
}

impl<K: Kind> Hunt<K> {
    fn new(token: u64, proj: K::Proj, site: Value) -> Self {
        Self {
            token,
            proj: Some(proj),
            site,
            reply: None,
            sleep: 0,
            waiting: Waiting::Nothing,
            child: None,
            ended: false,
        }
    }

    /// A one-token run over `site`.
    fn run(site: Value) -> Self {
        let proj = K::parse(&site);
        Self::new(K::mint(), proj, site)
    }

    /// End the token (a run) or hand the site back (a session), once.
    fn finish(&mut self) {
        if std::mem::replace(&mut self.ended, true) {
            return;
        }
        if K::SESSION {
            if let Some(proj) = self.proj.take() {
                put_site::<K>(self.token, proj);
            }
        } else {
            K::end(self.token);
        }
    }

    fn done(&mut self, value: Option<bool>) -> Step<Value> {
        self.finish();
        Step::Done(if K::BOOLEAN {
            Value::Bool(value.unwrap_or(false))
        } else {
            Value::Null
        })
    }

    fn fail(&mut self, thrown: Thrown) -> Step<Value> {
        self.finish();
        Step::Fail(thrown)
    }

    fn drive(&mut self, cx: &mut Cx<'_>) -> Result<Step<Value>, Ended> {
        if let Some(step) = self.step_child(cx)? {
            return Ok(step);
        }
        match std::mem::replace(&mut self.waiting, Waiting::Nothing) {
            Waiting::Nothing => {}
            waiting => match cx.reply() {
                Some(Reply::Threw(thrown)) => return Ok(self.fail(thrown)),
                Some(Reply::Value(value)) => match waiting {
                    Waiting::Eat => self.reply = Some(json!({ "eatOk": truthy(&value) })),
                    Waiting::Leave => {
                        self.reply = Some(json!({ "left": value == Value::Bool(true) }));
                    }
                    Waiting::Plain | Waiting::Nothing => {}
                },
                None => {}
            },
        }
        if self.sleep > 0 {
            self.sleep -= 1;
            return Ok(Step::Wait);
        }
        for _ in 0..EFFECTS_PER_STEP {
            if let Some(step) = self.step_child(cx)? {
                return Ok(step);
            }
            let proj = self.proj.as_mut().expect("a live hunt row holds its site");
            prepare::<K>(proj, &mut CxHost(cx))?;
            let effect = K::next(self.token, proj, self.reply.take().as_ref());
            if let Some(step) = self.apply(&effect, cx)? {
                return Ok(step);
            }
        }
        Ok(Step::Wait)
    }

    /// Step the running child; `Some` when this step is its.
    fn step_child(&mut self, cx: &mut Cx<'_>) -> Result<Option<Step<Value>>, Ended> {
        let Some(child) = self.child.as_mut() else {
            return Ok(None);
        };
        let (step, key) = match child {
            Child::Leave(hunt) => (hunt.drive(cx)?, "left"),
            Child::Key(hunt) => (hunt.drive(cx)?, "held"),
            Child::Bank(hunt) => (hunt.drive(cx)?, "banked"),
            Child::Cell(hunt) => (hunt.drive(cx)?, "celled"),
            Child::Teleport(teleport) => (map_bool(teleport.step(cx)), "teleported"),
            Child::BankOpen(open) => (map_bool(open.step(cx)), "opened"),
        };
        match step {
            Step::Done(value) => {
                self.child = None;
                self.reply = Some(json!({ key: value == Value::Bool(true) }));
                Ok(None)
            }
            Step::Fail(thrown) => {
                self.child = None;
                Ok(Some(self.fail(thrown)))
            }
            step => Ok(Some(step)),
        }
    }

    /// Carry out one effect; `Some` ends this step.
    fn apply(&mut self, effect: &Value, cx: &mut Cx<'_>) -> Result<Option<Step<Value>>, Ended> {
        notes(effect, cx)?;
        let kind = effect.get("kind").and_then(Value::as_str).unwrap_or("");
        let ack = json!({ "queued": true });
        match kind {
            "yield" => {
                let value = effect.get("value").and_then(Value::as_bool) == Some(true);
                return Ok(Some(self.done(Some(value))));
            }
            "aborted" => {
                // The stepper will not run this token again: a session
                // continues on a fresh one, as the old JS re-bind did.
                if K::SESSION {
                    K::renew(self.token);
                }
                return Ok(Some(self.done(None)));
            }
            "wait" => return Ok(Some(Step::Wait)),
            "delay-ticks" => {
                let n = effect.get("n").and_then(Value::as_u64).unwrap_or(1).max(1);
                self.sleep = u32::try_from(n - 1).unwrap_or(u32::MAX);
                return Ok(Some(Step::Wait));
            }
            "log" => notify(cx, hook::LOG, &[message(effect)])?,
            "vlog" => notify(cx, hook::VLOG, &[message(effect)])?,
            "status" => notify(cx, hook::SET_STATUS, &[message(effect)])?,
            "set-safespot" => notify(cx, hook::SET_SAFESPOT, &[field(effect, "index")])?,
            "set-target" => notify(cx, hook::SET_TARGET, &[field(effect, "index")])?,
            "count-burial" => notify(cx, hook::COUNT_BURIAL, &[])?,
            "count-bank-trip" => notify(cx, hook::COUNT_BANK_TRIP, &[])?,
            "park" => notify(cx, hook::PARK_FOR, &[field(effect, "reason")])?,
            "pick-weapon" => {
                let names = effect.get("names").cloned().unwrap_or_else(|| json!([]));
                notify(cx, hook::PICK_WEAPON, &[names])?;
            }
            // Frozen `await host.eatOnce()`: a host without it throws.
            "eat" => return Ok(Some(self.call(hook::EAT_ONCE, Waiting::Eat))),
            "arm-special" if cx.has(hook::ARM_SPECIAL) => {
                return Ok(Some(self.call(hook::ARM_SPECIAL, Waiting::Plain)));
            }
            "sustain" if cx.has(hook::SUSTAIN) => {
                return Ok(Some(self.call(hook::SUSTAIN, Waiting::Plain)));
            }
            "arm-special" | "sustain" => {}
            "bury" => {
                cx.emit(InteractReq::Held {
                    name: string(effect, "name"),
                    action: "Bury".into(),
                });
                self.reply = Some(json!({ "buried": true }));
            }
            "walk-to" => {
                cx.emit(InteractReq::WalkTo {
                    x: int(effect, "x"),
                    z: int(effect, "z"),
                    level: int(effect, "level"),
                });
                if K::WALK_TO_ACK {
                    self.reply = Some(ack);
                }
            }
            "walk" | "walk-near" => {
                let radius = if kind == "walk" {
                    0
                } else {
                    int(effect, "radius")
                };
                self.walk(effect, kind == "walk", radius, cx);
            }
            "continue" => cx.emit(InteractReq::ContinueDialog),
            "answer" => cx.emit(InteractReq::Answer {
                option: int(effect, "option"),
            }),
            "close-modal" => cx.emit(InteractReq::CloseModal),
            "teleport" => {
                let name = string(effect, "name");
                let args = serde_json::from_value(json!({ "name": name }))
                    .expect("teleport arguments are a name");
                match <crate::teleport::Teleport as Family>::begin(args, cx) {
                    Begin::Run(teleport) => {
                        self.child = Some(Child::Teleport(teleport));
                        return Ok(Some(Step::Wait));
                    }
                    Begin::Done(ok) => self.reply = Some(json!({ "teleported": ok })),
                    Begin::Refuse(_) => self.reply = Some(json!({ "teleported": false })),
                }
            }
            "bank-open" => {
                let args = serde_json::from_value(json!({
                    "mode": "open-nearest",
                    "stand": null,
                    "booth_name": "Bank booth",
                    "booth_action": "Use-quickly",
                }))
                .expect("bank open arguments");
                match <crate::bank_open::BankOpen as Family>::begin(args, cx) {
                    Begin::Run(open) => {
                        self.child = Some(Child::BankOpen(open));
                        return Ok(Some(Step::Wait));
                    }
                    Begin::Done(ok) => self.reply = Some(json!({ "opened": ok })),
                    Begin::Refuse(_) => self.reply = Some(json!({ "opened": false })),
                }
            }
            // A bank run's own `opts.leave`, else the leave family.
            "leave" if cx.has(hook::LEAVE) => {
                return Ok(Some(self.call(hook::LEAVE, Waiting::Leave)));
            }
            "leave" => self.child = Some(Child::Leave(Box::new(Hunt::run(self.site.clone())))),
            "key" => self.child = Some(Child::Key(Box::new(Hunt::run(self.site.clone())))),
            _ => match game_op(kind, effect) {
                Some(op) => {
                    cx.emit(op);
                    self.reply = Some(ack);
                }
                None => {
                    return Ok(Some(
                        self.fail(Thrown::new(format!("not impl: hunt effect {kind:?}"))),
                    ));
                }
            },
        }
        Ok(None)
    }

    fn call(&mut self, hook: usize, waiting: Waiting) -> Step<Value> {
        self.waiting = waiting;
        Step::Call(Call {
            hook,
            args: Vec::new(),
        })
    }

    /// A world walk the walk wait tracks; the stepper reads its token.
    fn walk(&mut self, effect: &Value, exact: bool, radius: i32, cx: &mut Cx<'_>) {
        let (x, z, level) = (int(effect, "x"), int(effect, "z"), int(effect, "level"));
        let token = crate::walk_wait::dispatch(&json!({
            "op": "begin",
            "x": x,
            "z": z,
            "level": level,
            "radius": radius,
            "allow_teleports": false,
        }))
        .as_u64()
        .unwrap_or(0);
        let world = K::WORLD_WALK;
        cx.emit(if exact {
            InteractReq::Walk {
                x,
                z,
                level,
                allow_teleports: false,
                allow_wilderness: world,
                allow_bank_fetch: world,
                request_id: token,
            }
        } else {
            InteractReq::WalkNear {
                x,
                z,
                level,
                radius,
                allow_teleports: false,
                allow_wilderness: world,
                allow_bank_fetch: world,
                request_id: token,
            }
        });
        self.reply = Some(json!({ "queued": true, "walkToken": token }));
    }
}

fn map_bool(step: Step<bool>) -> Step<Value> {
    match step {
        Step::Wait => Step::Wait,
        Step::Call(call) => Step::Call(call),
        Step::Done(ok) => Step::Done(Value::Bool(ok)),
        Step::Fail(thrown) => Step::Fail(thrown),
    }
}

/// A synchronous notification; skipped when the script has no such hook
/// (the frozen `host.log?.(m)`).
fn notify(cx: &mut Cx<'_>, hook: usize, args: &[Value]) -> Result<(), Ended> {
    if cx.has(hook) {
        cx.ask(hook, args)?;
    }
    Ok(())
}

/// The host calls a stepper queued ahead of `effect` (status and log
/// lines, `countKill`), in order.
fn notes(effect: &Value, cx: &mut Cx<'_>) -> Result<(), Ended> {
    let Some(notes) = effect.get("notes").and_then(Value::as_array) else {
        return Ok(());
    };
    for note in notes {
        match note.get("kind").and_then(Value::as_str) {
            Some("count-kill") => notify(cx, hook::COUNT_KILL, &[])?,
            Some("status") => notify(cx, hook::SET_STATUS, &[message(note)])?,
            Some("vlog") => notify(cx, hook::VLOG, &[message(note)])?,
            _ => notify(cx, hook::LOG, &[message(note)])?,
        }
    }
    Ok(())
}

fn field(effect: &Value, key: &str) -> Value {
    effect.get(key).cloned().unwrap_or(Value::Null)
}

fn message(effect: &Value) -> Value {
    field(effect, "message")
}

fn string(effect: &Value, key: &str) -> String {
    effect
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn int(effect: &Value, key: &str) -> i32 {
    effect
        .get(key)
        .and_then(Value::as_i64)
        .and_then(|n| i32::try_from(n).ok())
        .unwrap_or(0)
}

fn opt_int(effect: &Value, key: &str) -> Option<i32> {
    effect
        .get(key)
        .and_then(Value::as_i64)
        .and_then(|n| i32::try_from(n).ok())
}

/// The one-op effects the steppers acknowledge `{ queued: true }`.
fn game_op(kind: &str, effect: &Value) -> Option<InteractReq> {
    Some(match kind {
        "npc" => InteractReq::Npc {
            name: string(effect, "name"),
            action: string(effect, "action"),
            index: opt_int(effect, "index"),
        },
        "loc" => InteractReq::Loc {
            x: int(effect, "x"),
            z: int(effect, "z"),
            level: int(effect, "level"),
            action: string(effect, "action"),
            id: opt_int(effect, "id"),
        },
        "use-on" => InteractReq::UseOn {
            name: string(effect, "name"),
            kind: "loc".into(),
            target_name: None,
            x: int(effect, "x"),
            z: int(effect, "z"),
            level: int(effect, "level"),
            index: None,
            source_item_id: opt_int(effect, "id"),
            source_item_slot: opt_int(effect, "slot"),
            target_item_id: None,
            target_item_slot: None,
        },
        "obj" => InteractReq::Obj {
            x: int(effect, "x"),
            z: int(effect, "z"),
            level: int(effect, "level"),
            name: effect
                .get("name")
                .and_then(Value::as_str)
                .map(str::to_string),
            action: string(effect, "action"),
        },
        "deposit" => InteractReq::Deposit {
            name: string(effect, "name"),
        },
        "withdraw" => InteractReq::Withdraw {
            name: string(effect, "name"),
            action: string(effect, "action"),
        },
        "withdraw-x" => InteractReq::WithdrawX {
            name: string(effect, "name"),
            count: int(effect, "count"),
            bank_item_id: int(effect, "bank_item_id"),
            lands_as_id: int(effect, "lands_as_id"),
            action: string(effect, "action"),
            bank_generation: effect
                .get("bank_generation")
                .and_then(Value::as_u64)
                .unwrap_or(0),
        },
        "wear" => InteractReq::Wear {
            name: string(effect, "name"),
        },
        "held" => InteractReq::Held {
            name: string(effect, "name"),
            action: string(effect, "action"),
        },
        "close" => InteractReq::Close,
        _ => return None,
    })
}
