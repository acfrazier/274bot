//! Rust-owned DeathRecovery: the death latch and the recovery run.
//!
//! - The latch reads the posted game chat from the isolate scene: only a
//!   *new* death line (a seq past the first observation) latches; seeded,
//!   old and duplicate chat does not. `validate` is one typed helper
//!   (`load/bank_tasks_v8.rs`) that observes, fires the caller's `onDeath`
//!   on a new latch, and answers due.
//! - The `death_recovery` [`crate::machine`] family is frozen `execute`:
//!   wait for the respawn (bounded 20 s), three ticks, then the caller's
//!   `walkBack` or a walk to the anchor, and clear the latch with
//!   `onRecovered`. Position at the anchor is not recovery until the run
//!   finished, so a death seen before the respawn cannot clear it.

use crate::load::reach_query::arrived;
use crate::machine::{self, Begin, Call, Cx, Family, Reply, Step};
use crate::observed::{self, ChatLine};
use crate::shim::InteractReq;
use api::snapshot::WorldTile as Tile;
use serde::Deserialize;
use serde_json::Value;
use std::cell::RefCell;

pub const WALK_BOUND_MS: u64 = 60_000;
pub const RESPAWN_WAIT_MS: u64 = 20_000;
pub const RESPAWN_TICKS: u64 = 3;
pub const DEFAULT_RADIUS: i32 = 6;

/// The chat latch, kept across runs and reset with the session.
struct Latch {
    latched: bool,
    baseline: bool,
    last_seq: i32,
}

thread_local! {
    static LATCH: RefCell<Latch> = const {
        RefCell::new(Latch {
            latched: false,
            baseline: false,
            last_seq: 0,
        })
    };
}

impl Latch {
    /// Observe the posted chat; `true` when a new death line latched now.
    fn observe(&mut self, lines: &[ChatLine]) -> bool {
        let max_seq = lines
            .iter()
            .map(|line| line.seq)
            .max()
            .unwrap_or(self.last_seq);
        if !self.baseline {
            self.last_seq = max_seq;
            self.baseline = true;
            return false;
        }
        let mut newest = self.last_seq;
        let mut death = false;
        for line in lines.iter().filter(|line| line.seq > self.last_seq) {
            newest = newest.max(line.seq);
            death |= is_death_line(&line.text);
        }
        self.last_seq = newest;
        if death && !self.latched {
            self.latched = true;
            return true;
        }
        false
    }
}

/// Frozen `/oh dear.*you are dead/i`.
fn is_death_line(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    let Some(at) = lower.find("oh dear") else {
        return false;
    };
    lower[at + "oh dear".len()..].contains("you are dead")
}

/// `DeathRecovery.validate`'s Rust half: observe the posted chat, and
/// answer `(due, died_now)`. Due while latched and no recovery runs.
pub(crate) fn validate() -> (bool, bool) {
    let died = observed::with(|scene| {
        let lines = scene
            .since_login()
            .chat_lines()
            .map(Vec::as_slice)
            .unwrap_or_default();
        LATCH.with(|latch| latch.borrow_mut().observe(lines))
    });
    let latched = LATCH.with(|latch| latch.borrow().latched);
    (latched && !machine::live(DeathRecovery::NAME), died)
}

pub fn on_reset() {
    LATCH.with(|latch| {
        *latch.borrow_mut() = Latch {
            latched: false,
            baseline: false,
            last_seq: 0,
        }
    });
}

fn latched() -> bool {
    LATCH.with(|latch| latch.borrow().latched)
}

#[derive(Deserialize)]
pub(crate) struct RecoveryArgs {
    #[serde(default)]
    anchor: Value,
    #[serde(default)]
    radius: Value,
}

const WALK_BACK: usize = 0;
const ON_RECOVERED: usize = 1;

enum Phase {
    Respawn,
    Ticks(u64),
    WalkBack { asked: bool },
    Walk,
    Walking,
    Recovered { asked: bool },
}

/// One awaited frozen `DeathRecovery.execute`.
pub(crate) struct DeathRecovery {
    anchor: Option<Tile>,
    radius: i32,
    phase: Phase,
}

fn posted_tick() -> u64 {
    observed::with(|scene| scene.tick().unwrap_or(0))
}

fn read_anchor(value: &Value) -> Option<Tile> {
    let tile = value.get("tile").unwrap_or(value);
    Some(Tile {
        x: i32::try_from(tile.get("x")?.as_i64()?).ok()?,
        z: i32::try_from(tile.get("z")?.as_i64()?).ok()?,
        level: tile
            .get("level")
            .and_then(Value::as_i64)
            .and_then(|l| i32::try_from(l).ok())
            .unwrap_or(0),
    })
}

impl Family for DeathRecovery {
    const NAME: &'static str = "death_recovery";
    const CALLBACKS: &'static [&'static str] = &["walkBack", "onRecovered"];
    type Args = RecoveryArgs;
    type Output = Value;

    fn begin(args: RecoveryArgs, cx: &mut Cx<'_>) -> Begin<Self> {
        let anchor = read_anchor(&args.anchor);
        if !latched() || (anchor.is_none() && !cx.has(WALK_BACK)) {
            return Begin::Done(Value::Null);
        }
        cx.clock().arm(RESPAWN_WAIT_MS);
        Begin::Run(Self {
            anchor,
            radius: args
                .radius
                .as_i64()
                .and_then(|r| i32::try_from(r).ok())
                .unwrap_or(DEFAULT_RADIUS),
            phase: Phase::Respawn,
        })
    }

    fn step(&mut self, cx: &mut Cx<'_>) -> Step<Value> {
        loop {
            match self.phase {
                Phase::Respawn => {
                    let respawned = observed::with(|scene| {
                        let session = scene.since_login();
                        session.ingame().unwrap_or(false) && session.here().is_some()
                    });
                    if !respawned && !cx.clock().bound_reached() {
                        return Step::Wait;
                    }
                    // Frozen: the respawn wait's result is not read.
                    self.phase = Phase::Ticks(posted_tick().saturating_add(RESPAWN_TICKS));
                }
                Phase::Ticks(due) => {
                    if posted_tick() < due {
                        return Step::Wait;
                    }
                    self.phase = if cx.has(WALK_BACK) {
                        Phase::WalkBack { asked: false }
                    } else {
                        Phase::Walk
                    };
                }
                Phase::WalkBack { asked: false } => {
                    self.phase = Phase::WalkBack { asked: true };
                    return Step::Call(Call {
                        hook: WALK_BACK,
                        args: Vec::new(),
                    });
                }
                Phase::WalkBack { asked: true } => {
                    if let Some(Reply::Threw(thrown)) = cx.reply() {
                        return Step::Fail(thrown);
                    }
                    // The script's walkBack finishing is the recovery.
                    self.phase = Phase::Recovered { asked: false };
                }
                Phase::Walk => {
                    let Some(anchor) = self.anchor else {
                        return Step::Done(Value::Null);
                    };
                    if arrived(anchor, self.radius) {
                        self.phase = Phase::Recovered { asked: false };
                        continue;
                    }
                    cx.clock().arm(WALK_BOUND_MS);
                    cx.emit(InteractReq::WalkNear {
                        x: anchor.x,
                        z: anchor.z,
                        level: anchor.level,
                        radius: self.radius,
                        allow_teleports: false,
                        allow_wilderness: true,
                        allow_bank_fetch: true,
                        request_id: 0,
                    });
                    self.phase = Phase::Walking;
                    return Step::Wait;
                }
                Phase::Walking => {
                    let Some(anchor) = self.anchor else {
                        return Step::Done(Value::Null);
                    };
                    if arrived(anchor, self.radius) {
                        self.phase = Phase::Recovered { asked: false };
                    } else if cx.clock().bound_reached() {
                        // Still latched: the next validate runs it again.
                        return Step::Done(Value::Null);
                    } else {
                        return Step::Wait;
                    }
                }
                Phase::Recovered { asked: false } => {
                    LATCH.with(|latch| latch.borrow_mut().latched = false);
                    if !cx.has(ON_RECOVERED) {
                        return Step::Done(Value::Null);
                    }
                    self.phase = Phase::Recovered { asked: true };
                    return Step::Call(Call {
                        hook: ON_RECOVERED,
                        args: Vec::new(),
                    });
                }
                Phase::Recovered { asked: true } => {
                    if let Some(Reply::Threw(thrown)) = cx.reply() {
                        return Step::Fail(thrown);
                    }
                    return Step::Done(Value::Null);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::rc::Rc;

    fn line(seq: i32, text: &str) -> ChatLine {
        ChatLine {
            seq,
            text: Rc::from(text),
        }
    }

    #[test]
    fn death_line_is_the_frozen_pattern() {
        assert!(is_death_line("Oh dear, you are dead!"));
        assert!(is_death_line("OH DEAR YOU ARE DEAD"));
        assert!(!is_death_line("Welcome to RuneScape"));
        assert!(!is_death_line("you are dead"));
    }

    #[test]
    fn seeded_duplicate_and_old_chat_do_not_latch() {
        let mut latch = Latch {
            latched: false,
            baseline: false,
            last_seq: 0,
        };
        let death = [line(4, "Oh dear, you are dead!"), line(3, "Welcome")];
        assert!(!latch.observe(&death), "the seeded ring does not latch");
        assert!(!latch.observe(&death), "a duplicate post does not latch");
        assert!(
            !latch.observe(&[line(3, "Welcome")]),
            "old chat does not latch"
        );
        let fresh = [line(5, "Oh dear, you are dead!"), line(4, "Welcome")];
        assert!(latch.observe(&fresh));
        assert!(latch.latched);
        assert!(!latch.observe(&fresh), "the same death seq fires once");
    }
}
