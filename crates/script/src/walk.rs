//! Frozen `Traversal.walkResilient` as a step machine, and the one native
//! walk both this family and `walk-hops` issue.
//!
//! Frozen (`Traversal.ts` 102–209, `walkLadder.ts` 47–86):
//! - `isArrived` first, then a baked `walkTo`.
//! - `EventSignal.pending()` (`hold` / `ours`) returns false.
//! - `timeoutMs` is each baked walk's bound, default 90_000 (`Traversal.ts:107`).
//! - `attempts` is `maxPasses` (`Traversal.ts:108`). The no-progress stop
//!   runs only when it is set (`Traversal.ts:149`). Undefined attempts does
//!   not mean one-shot: frozen keeps escalating (scene / unstick / verify).
//!   This host has no scene walker, so a no-progress baked failure ends.
//! - A closer Chebyshev (`walkChebyshev`) resets no-progress and bakes again
//!   (`walkLadder.ts:55–59`) — the wolf-pit fall.

use crate::machine::{Begin, Call, Cx, Family, Reply, Step};
use crate::observed;
use crate::shim::InteractReq;
use crate::walk_wait;
use api::snapshot::WorldTile;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::VecDeque;

/// Frozen `opts.timeoutMs ?? 90000` (`Traversal.ts:107`).
const BAKED_TIMEOUT_MS: u64 = 90_000;

#[derive(Clone, Copy, Debug, Deserialize)]
struct Tile {
    x: i32,
    z: i32,
    #[serde(default)]
    level: i32,
}

impl Tile {
    fn world(self) -> WorldTile {
        WorldTile {
            x: self.x,
            z: self.z,
            level: self.level,
        }
    }
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WalkResilientOpts {
    #[serde(default)]
    radius: i32,
    #[serde(default)]
    attempts: Option<u32>,
    #[serde(default)]
    timeout_ms: Option<u64>,
    #[serde(default)]
    use_teleport_catalog: bool,
}

#[derive(Deserialize)]
pub(crate) struct WalkResilientArgs {
    tile: Tile,
    #[serde(default)]
    opts: WalkResilientOpts,
}

/// Frozen `walkChebyshev`: Chebyshev on the plane, never close across a floor.
fn walk_chebyshev(a: WorldTile, b: WorldTile) -> i32 {
    let xz = (a.x - b.x).abs().max((a.z - b.z).abs());
    if a.level == b.level {
        xz
    } else {
        1_000_000 + xz
    }
}

fn here() -> Option<WorldTile> {
    observed::with(|scene| {
        scene.since_login().here().map(|t| WorldTile {
            x: t.x,
            z: t.z,
            level: t.level,
        })
    })
}

/// Frozen `EventSignal.pending()`: guardian hold or a detected owned random.
fn interrupted() -> bool {
    observed::with(|scene| {
        let session = scene.since_login();
        session.hold().unwrap_or(false) || session.ours().unwrap_or(false)
    })
}

/// Frozen `isArrived` over the cached reach view.
fn arrived(dest: WorldTile, radius: i32) -> bool {
    let Some(here) = here() else {
        return false;
    };
    crate::load::reach_query::with_view(|view| api::query::is_arrived(here, dest, radius, || view))
}

/// One native walk: arrival check, then the settled outcome. Frozen
/// `walkResilient`'s baked `walkTo`; `walk-hops` and `walk-resilient` share it.
pub(crate) struct Walk {
    token: u64,
}

impl Walk {
    /// `Err(arrived)` when no walk was needed (or no tile is posted).
    pub(crate) fn begin(
        dest: WorldTile,
        radius: i32,
        timeout_ms: u64,
        allow_teleports: bool,
        cx: &mut Cx<'_>,
    ) -> Result<Self, bool> {
        let Some(here) = here() else {
            return Err(false);
        };
        // Frozen `walkResilient` asks `isArrived` first: the one arrival
        // rule the walk wait and the host follow share.
        let arrived = crate::load::reach_query::with_view(|view| {
            api::query::is_arrived(here, dest, radius, || view)
        });
        if arrived {
            return Err(true);
        }
        let token = walk_wait::dispatch(&json!({
            "op": "begin",
            "x": dest.x,
            "z": dest.z,
            "level": dest.level,
            "radius": radius,
            "allow_teleports": allow_teleports,
        }))
        .as_u64()
        .unwrap_or(0);
        cx.emit(if radius > 0 {
            InteractReq::WalkNear {
                x: dest.x,
                z: dest.z,
                level: dest.level,
                radius,
                allow_teleports,
                allow_wilderness: true,
                allow_bank_fetch: true,
                request_id: token,
            }
        } else {
            InteractReq::Walk {
                x: dest.x,
                z: dest.z,
                level: dest.level,
                allow_teleports,
                allow_wilderness: true,
                allow_bank_fetch: true,
                request_id: token,
            }
        });
        cx.clock().arm(timeout_ms);
        Ok(Self { token })
    }

    /// `Some(arrived)` once the walk settled or timed out. A timeout stops
    /// the host follow (as a returned frozen `walkResilient` has stopped
    /// its walker), so its next walk packet cannot cancel a later click.
    pub(crate) fn step(&self, cx: &mut Cx<'_>) -> Option<bool> {
        let settled = walk_wait::dispatch(&json!({ "op": "settled", "token": self.token }))
            .as_bool()
            .unwrap_or(false);
        if settled {
            return Some(
                walk_wait::dispatch(&json!({ "op": "value", "token": self.token }))
                    .as_bool()
                    .unwrap_or(false),
            );
        }
        if !cx.clock().bound_reached() {
            return None;
        }
        self.abort(cx);
        Some(false)
    }

    pub(crate) fn abort(&self, cx: &mut Cx<'_>) {
        cx.emit(InteractReq::AbortWalk {
            request_id: self.token,
        });
    }
}

enum Phase {
    Start,
    Walking(Walk),
    Delay,
}

/// Frozen `Traversal.walkResilient(dest, opts)`.
pub(crate) struct WalkResilient {
    dest: WorldTile,
    radius: i32,
    timeout_ms: u64,
    attempts: Option<u32>,
    allow_teleports: bool,
    phase: Phase,
    best_dist: i32,
    no_progress: u32,
    logger: VecDeque<String>,
    result: Option<bool>,
    waiting: bool,
}

impl Family for WalkResilient {
    const NAME: &'static str = "walk-resilient";
    const CALLBACKS: &'static [&'static str] = &["log"];
    /// The first baked walk goes out in the caller's turn.
    const KICK_ON_START: bool = true;
    /// Frozen `log(...)` is not awaited.
    const AWAIT_CALLBACKS: bool = false;
    type Args = WalkResilientArgs;
    type Output = bool;

    fn begin(args: WalkResilientArgs, _cx: &mut Cx<'_>) -> Begin<Self> {
        if interrupted() {
            return Begin::Done(false);
        }
        let dest = args.tile.world();
        let radius = args.opts.radius;
        let Some(here) = here() else {
            return Begin::Done(false);
        };
        if arrived(dest, radius) {
            return Begin::Done(true);
        }
        Begin::Run(Self {
            dest,
            radius,
            timeout_ms: args.opts.timeout_ms.unwrap_or(BAKED_TIMEOUT_MS),
            attempts: args.opts.attempts,
            allow_teleports: args.opts.use_teleport_catalog,
            phase: Phase::Start,
            best_dist: walk_chebyshev(here, dest),
            no_progress: 0,
            logger: VecDeque::new(),
            result: None,
            waiting: false,
        })
    }

    fn step(&mut self, cx: &mut Cx<'_>) -> Step<bool> {
        if let Some(Reply::Threw(thrown)) = cx.reply() {
            return Step::Fail(thrown);
        }
        loop {
            if let Some(line) = self.logger.pop_front() {
                if cx.has(LOG) {
                    return Step::Call(Call {
                        hook: LOG,
                        args: vec![json!(line)],
                    });
                }
                continue;
            }
            if let Some(result) = self.result {
                return Step::Done(result);
            }
            if std::mem::take(&mut self.waiting) {
                return Step::Wait;
            }
            match self.advance(cx) {
                Some(result) => self.result = Some(result),
                None if self.logger.is_empty() => return Step::Wait,
                None => self.waiting = true,
            }
        }
    }
}

const LOG: usize = 0;

impl WalkResilient {
    fn advance(&mut self, cx: &mut Cx<'_>) -> Option<bool> {
        if interrupted() {
            if let Phase::Walking(walk) = &self.phase {
                walk.abort(cx);
            }
            self.logger
                .push_back("walk interrupted by a runtime event — yielding to the runtime".into());
            return Some(false);
        }
        if arrived(self.dest, self.radius) {
            return Some(true);
        }
        if let Some(max) = self.attempts {
            if self.no_progress >= max {
                self.logger.push_back(format!(
                    "walkResilient: {max} passes made no progress — stopping (bounded caller)"
                ));
                return Some(false);
            }
        }
        match std::mem::replace(&mut self.phase, Phase::Start) {
            Phase::Start | Phase::Delay => self.kick_walk(cx),
            Phase::Walking(walk) => match walk.step(cx) {
                None => {
                    self.phase = Phase::Walking(walk);
                    None
                }
                Some(true) => Some(true),
                Some(false) => self.after_baked(cx),
            },
        }
    }

    fn kick_walk(&mut self, cx: &mut Cx<'_>) -> Option<bool> {
        match Walk::begin(
            self.dest,
            self.radius,
            self.timeout_ms,
            self.allow_teleports,
            cx,
        ) {
            Ok(walk) => {
                self.phase = Phase::Walking(walk);
                None
            }
            Err(done) => Some(done),
        }
    }

    /// Frozen `advance` after a baked `walkTo`: closer → bake again;
    /// otherwise escalate. This host has no scene/unstick/verify, so a
    /// no-progress failure either counts toward `attempts` or ends.
    fn after_baked(&mut self, cx: &mut Cx<'_>) -> Option<bool> {
        if arrived(self.dest, self.radius) {
            return Some(true);
        }
        if interrupted() {
            self.logger
                .push_back("walk interrupted by a runtime event — yielding to the runtime".into());
            return Some(false);
        }
        let Some(here) = here() else {
            return Some(false);
        };
        let cur = walk_chebyshev(here, self.dest);
        if cur < self.best_dist {
            self.best_dist = cur;
            self.no_progress = 0;
            return self.kick_walk(cx);
        }
        match self.attempts {
            None => Some(false),
            Some(max) => {
                self.no_progress += 1;
                if self.no_progress >= max {
                    self.logger.push_back(format!(
                        "walkResilient: {max} passes made no progress — stopping (bounded caller)"
                    ));
                    Some(false)
                } else {
                    self.phase = Phase::Delay;
                    None
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::load::callback_v8::HeldCallback;
    use crate::machine::{self, Called, Outcome, Pending, Started, Take};
    use crate::observed::{self, WalkOutcome};
    use crate::walk_wait;

    struct NoJs;

    impl machine::Js for NoJs {
        fn queue_len(&mut self) -> usize {
            0
        }

        fn call(&mut self, _hook: Option<&HeldCallback>, _args: &[Value]) -> Called {
            panic!("no hook is held");
        }

        fn poll(&mut self, _pending: &Pending) -> Option<Reply> {
            panic!("no hook is held");
        }

        fn claimed(&mut self) -> bool {
            false
        }
    }

    fn reset() {
        observed::on_reset();
        machine::on_reset();
        walk_wait::on_reset();
    }

    fn post_here(x: i32, z: i32) {
        observed::post(1, |post| {
            post.session(true).here(observed::Tile { x, z, level: 0 });
        });
    }

    fn start(attempts: Option<u32>) -> machine::Handle {
        let mut args = json!({ "tile": { "x": 10, "z": 0, "level": 0 }, "opts": { "radius": 0 } });
        if let Some(attempts) = attempts {
            args["opts"]["attempts"] = json!(attempts);
        }
        let Started::Running(h) = machine::start("walk-resilient", args, Vec::new(), 0) else {
            panic!("walk-resilient runs");
        };
        h
    }

    fn walk_token() -> u64 {
        match machine::merge_ops(Vec::new()).as_slice() {
            [InteractReq::Walk {
                x: 10,
                z: 0,
                request_id,
                ..
            }] => *request_id,
            other => panic!("expected the walk, got {other:?}"),
        }
    }

    #[test]
    fn already_arrived_is_done_without_a_walk() {
        reset();
        post_here(10, 0);
        let args = json!({ "tile": { "x": 10, "z": 0, "level": 0 }, "opts": { "radius": 0 } });
        match machine::start("walk-resilient", args, Vec::new(), 0) {
            Started::Settled(Outcome::Done(Value::Bool(true))) => {}
            other => panic!("arrived begin is done true, got {other:?}"),
        }
        assert!(machine::merge_ops(Vec::new()).is_empty());
    }

    #[test]
    fn interrupt_at_start_is_done_false() {
        reset();
        observed::post(1, |post| {
            post.session(true)
                .here(observed::Tile {
                    x: 0,
                    z: 0,
                    level: 0,
                })
                .ours(true);
        });
        let args = json!({ "tile": { "x": 10, "z": 0, "level": 0 }, "opts": { "radius": 0 } });
        match machine::start("walk-resilient", args, Vec::new(), 0) {
            Started::Settled(Outcome::Done(Value::Bool(false))) => {}
            other => panic!("pending EventSignal is done false, got {other:?}"),
        }
    }

    #[test]
    fn nopath_without_attempts_ends() {
        reset();
        post_here(0, 0);
        let h = start(None);
        machine::step(&mut NoJs);
        let token = walk_token();
        observed::post(2, |post| {
            post.session(true)
                .here(observed::Tile {
                    x: 0,
                    z: 0,
                    level: 0,
                })
                .walk_outcome(WalkOutcome {
                    seq: 1,
                    generation: 0,
                    request_id: token,
                    failed: true,
                    tile: observed::Tile {
                        x: 10,
                        z: 0,
                        level: 0,
                    },
                    radius: 0,
                    allow_teleports: false,
                });
        });
        machine::step(&mut NoJs);
        assert!(
            machine::merge_ops(Vec::new()).is_empty(),
            "undefined attempts does not bake again after no progress"
        );
        assert_eq!(machine::take(h), Take::Settled(Outcome::Done(json!(false))));
    }

    #[test]
    fn closer_tile_rebakes_after_nopath() {
        reset();
        post_here(0, 0);
        let h = start(Some(3));
        machine::step(&mut NoJs);
        let token = walk_token();
        observed::post(2, |post| {
            post.session(true)
                .here(observed::Tile {
                    x: 5,
                    z: 0,
                    level: 0,
                })
                .walk_outcome(WalkOutcome {
                    seq: 1,
                    generation: 0,
                    request_id: token,
                    failed: true,
                    tile: observed::Tile {
                        x: 10,
                        z: 0,
                        level: 0,
                    },
                    radius: 0,
                    allow_teleports: false,
                });
        });
        machine::step(&mut NoJs);
        let retry = match machine::merge_ops(Vec::new()).as_slice() {
            [InteractReq::Walk {
                x: 10,
                z: 0,
                request_id,
                ..
            }] => *request_id,
            other => panic!("progress rebakes, got {other:?}"),
        };
        assert_ne!(retry, 0);
        assert_ne!(retry, token);
        assert_eq!(machine::take(h), Take::Pending);
    }

    #[test]
    fn a_timed_out_walk_stops_the_host_follow_and_fails() {
        reset();
        post_here(0, 0);
        let h = start(None);
        machine::step(&mut NoJs);
        let walk_token = walk_token();
        assert_ne!(walk_token, 0);
        machine::step(&mut NoJs);
        assert!(machine::merge_ops(Vec::new()).is_empty(), "still walking");
        machine::tests::expire_deadlines();
        machine::step(&mut NoJs);
        assert_eq!(
            machine::merge_ops(Vec::new()),
            vec![InteractReq::AbortWalk {
                request_id: walk_token
            }],
        );
        assert_eq!(machine::take(h), Take::Settled(Outcome::Done(json!(false))));
    }
}
