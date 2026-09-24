//! Frozen `Traversal.walkResilient` as a step machine, and the one native
//! walk both this family and `walk-hops` issue.
//!
//! Frozen (`Traversal.ts` 102–209, `walkLadder.ts` 47–86):
//! - `await Sustain.run()` at the start of each loop (`Traversal.ts:130`).
//! - `EventSignal.pending()` for an owned random (`ours`) returns false
//!   (`Traversal.ts:131–134`, `walkLadder.ts:48–52`). Guardian `hold` does
//!   not abort: `machine::on_hold` freezes the row and the walk pauses.
//! - `timeoutMs` is each baked `walkTo` bound, default 90_000 (`Traversal.ts:107`).
//! - `attempts` is `maxPasses` (`Traversal.ts:108`); the bound applies only
//!   when set (`Traversal.ts:149`).
//! - A closer Chebyshev rebakes and resets no-progress (`walkLadder.ts:55–59`).
//! - A baked walk that ends short of the destination is not success: frozen
//!   reads `isArrived` on the next loop (`Traversal.ts:118–121`, `141–142`).
//! - No-progress after baked: frozen goes scene → unstick (`Traversal.ts:176–189`,
//!   `walkLadder.ts:73–81`). This host has no scene/unstick walker; the closest
//!   honest equivalent is one pass then backoff (`walkLadder.ts:82–86`).
//! - `UNREACHABLE_PASSES` (3) then verify (`walkLadder.ts:33, 83–84`). No
//!   `WalkExecutor.probeDest` here, so verify is fail-closed as probe-dead
//!   (`walkLadder.ts:66–68`).
//! - Backoff 2–16 ticks (`walkLadder.ts:30–31, 39–40`).
//! - Frozen `WalkExecutor.lastOutcome === 'blocked'` returns true
//!   (`Traversal.ts:172–174`). This host's walk wait is arrived-or-failed;
//!   there is no blocked, so a settled walk is re-checked with `isArrived`.
//! - Frozen `budget` rebakes once with `bigBudget` (`walkLadder.ts:73–75`).
//!   The wait is a bool; there is no budget vs failed, so no big-budget rebake.

use crate::machine::{Begin, Call, Cx, Family, Reply, Step};
use crate::observed;
use crate::shim::InteractReq;
use crate::walk_wait;
use api::snapshot::WorldTile;
use serde::Deserialize;
use serde_json::json;
use std::collections::VecDeque;

/// Frozen `opts.timeoutMs ?? 90000` (`Traversal.ts:107`).
pub(crate) const BAKED_TIMEOUT_MS: u64 = 90_000;
/// Frozen `walkWithHops` / hop approach `attempts: 3`.
pub(crate) const HOP_ATTEMPTS: u32 = 3;
/// Frozen `UNREACHABLE_PASSES` (`walkLadder.ts:33`).
const UNREACHABLE_PASSES: u32 = 3;
const BACKOFF_MIN: u32 = 2;
const BACKOFF_MAX: u32 = 16;

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

/// Frozen `EventSignal.pending()` for an owned random. Guardian `hold`
/// freezes the row (`machine::on_hold`) instead of aborting the walk.
fn interrupted() -> bool {
    observed::with(|scene| scene.since_login().ours().unwrap_or(false))
}

/// Frozen `isArrived` over the cached reach view.
fn arrived(dest: WorldTile, radius: i32) -> bool {
    let Some(here) = here() else {
        return false;
    };
    crate::load::reach_query::with_view(|view| api::query::is_arrived(here, dest, radius, || view))
}

/// Frozen `backoffTicks` (`walkLadder.ts:39–40`).
fn backoff_ticks(no_progress_passes: u32) -> u32 {
    BACKOFF_MAX.min(BACKOFF_MIN + 2 * no_progress_passes.saturating_sub(1))
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

    /// `Some(settled)` once the walk wait settled or timed out. A timeout
    /// stops the host follow (as a returned frozen `walkResilient` has
    /// stopped its walker). The bool is the wait's value, not `isArrived`:
    /// a closest terminal is true here and the ladder re-checks arrival.
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
    NeedWalk,
    Walking(Walk),
    Backoff,
}

/// Frozen walkResilient ladder over baked [`Walk`]s. Shared by `walk-hops`.
pub(crate) struct Resilient {
    dest: WorldTile,
    radius: i32,
    timeout_ms: u64,
    attempts: Option<u32>,
    allow_teleports: bool,
    phase: Phase,
    best_dist: i32,
    no_progress: u32,
    delay_left: u32,
    logs: VecDeque<String>,
}

impl Resilient {
    pub(crate) fn new(
        dest: WorldTile,
        radius: i32,
        timeout_ms: u64,
        attempts: Option<u32>,
        allow_teleports: bool,
    ) -> Self {
        let best_dist = here().map(|h| walk_chebyshev(h, dest)).unwrap_or(i32::MAX);
        Self {
            dest,
            radius,
            timeout_ms,
            attempts,
            allow_teleports,
            phase: Phase::NeedWalk,
            best_dist,
            no_progress: 0,
            delay_left: 0,
            logs: VecDeque::new(),
        }
    }

    /// Issue the first baked walk. `Err(done)` if no walk was needed.
    pub(crate) fn start(mut self, cx: &mut Cx<'_>) -> Result<Self, bool> {
        if interrupted() {
            self.logs
                .push_back("walk interrupted by a runtime event — yielding to the runtime".into());
            return Err(false);
        }
        let Some(_) = here() else {
            return Err(false);
        };
        if arrived(self.dest, self.radius) {
            return Err(true);
        }
        match self.kick_walk(cx) {
            None => Ok(self),
            Some(done) => Err(done),
        }
    }

    pub(crate) fn pop_log(&mut self) -> Option<String> {
        self.logs.pop_front()
    }

    /// `Some(result)` once the ladder ended; `None` waits.
    pub(crate) fn step(&mut self, cx: &mut Cx<'_>) -> Option<bool> {
        // Frozen: EventSignal.pending before isArrived (Traversal.ts:131,
        // walkLadder.ts:48–52).
        if interrupted() {
            if let Phase::Walking(walk) = &self.phase {
                walk.abort(cx);
            }
            self.logs
                .push_back("walk interrupted by a runtime event — yielding to the runtime".into());
            return Some(false);
        }
        if arrived(self.dest, self.radius) {
            return Some(true);
        }
        if let Some(max) = self.attempts {
            if self.no_progress >= max {
                self.logs.push_back(format!(
                    "walkResilient: {max} passes made no progress — stopping (bounded caller)"
                ));
                return Some(false);
            }
        }
        match std::mem::replace(&mut self.phase, Phase::NeedWalk) {
            Phase::NeedWalk => self.kick_walk(cx),
            Phase::Backoff => {
                if self.delay_left > 1 {
                    self.delay_left -= 1;
                    self.phase = Phase::Backoff;
                    None
                } else {
                    self.delay_left = 0;
                    self.kick_walk(cx)
                }
            }
            Phase::Walking(walk) => match walk.step(cx) {
                None => {
                    self.phase = Phase::Walking(walk);
                    None
                }
                Some(_) => self.after_baked(cx),
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

    fn after_baked(&mut self, cx: &mut Cx<'_>) -> Option<bool> {
        // Frozen next-loop order: pending, then withinRadius (N5/N6).
        if interrupted() {
            self.logs
                .push_back("walk interrupted by a runtime event — yielding to the runtime".into());
            return Some(false);
        }
        if arrived(self.dest, self.radius) {
            return Some(true);
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
        // Frozen: baked → scene → unstick, then passes++. No scene/unstick
        // on this host (Traversal.ts:176–189); count one pass.
        self.no_progress += 1;
        if let Some(max) = self.attempts {
            if self.no_progress >= max {
                self.logs.push_back(format!(
                    "walkResilient: {max} passes made no progress — stopping (bounded caller)"
                ));
                return Some(false);
            }
        }
        if self.no_progress >= UNREACHABLE_PASSES {
            // Frozen verify then probe-dead → unreachable (walkLadder.ts:66–68,
            // 83–84). No probeDest: fail-closed as dead.
            self.logs.push_back(format!(
                "walkResilient: ({},{},{}) unreachable from here — stopping (best {} tiles)",
                self.dest.x, self.dest.z, self.dest.level, self.best_dist
            ));
            return Some(false);
        }
        self.delay_left = backoff_ticks(self.no_progress);
        self.phase = Phase::Backoff;
        None
    }
}

/// Frozen `Traversal.walkResilient(dest, opts)`.
pub(crate) struct WalkResilient {
    drive: Resilient,
    pumped: bool,
    result: Option<bool>,
    waiting: bool,
}

impl Family for WalkResilient {
    const NAME: &'static str = "walk-resilient";
    const CALLBACKS: &'static [&'static str] = &["log", "sustain"];
    /// Frozen `log(...)` is not awaited; `await Sustain.run()` is.
    const SYNC_HOOKS: &'static [usize] = &[LOG];
    /// The first baked walk goes out in the caller's turn.
    const KICK_ON_START: bool = true;
    type Args = WalkResilientArgs;
    type Output = bool;

    fn begin(args: WalkResilientArgs, _cx: &mut Cx<'_>) -> Begin<Self> {
        if interrupted() {
            return Begin::Done(false);
        }
        let dest = args.tile.world();
        let radius = args.opts.radius;
        let Some(_) = here() else {
            return Begin::Done(false);
        };
        if arrived(dest, radius) {
            return Begin::Done(true);
        }
        Begin::Run(Self {
            drive: Resilient::new(
                dest,
                radius,
                args.opts.timeout_ms.unwrap_or(BAKED_TIMEOUT_MS),
                args.opts.attempts,
                args.opts.use_teleport_catalog,
            ),
            pumped: false,
            result: None,
            waiting: false,
        })
    }

    fn step(&mut self, cx: &mut Cx<'_>) -> Step<bool> {
        if let Some(Reply::Threw(thrown)) = cx.reply() {
            return Step::Fail(thrown);
        }
        loop {
            if let Some(line) = self.drive.pop_log() {
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
                self.pumped = false;
                return Step::Wait;
            }
            // Frozen `await Sustain.run()` (`Traversal.ts:130`); hunt-wait-fed
            // pumps the same hook so cards that eat during walks keep eating.
            if !self.pumped && cx.has(SUSTAIN) {
                self.pumped = true;
                return Step::Call(Call {
                    hook: SUSTAIN,
                    args: Vec::new(),
                });
            }
            match self.drive.step(cx) {
                Some(result) => self.result = Some(result),
                None if self.drive.logs.is_empty() => {
                    self.pumped = false;
                    return Step::Wait;
                }
                None => self.waiting = true,
            }
        }
    }
}

const LOG: usize = 0;
const SUSTAIN: usize = 1;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::load::callback_v8::HeldCallback;
    use crate::machine::{self, Called, Outcome, Pending, Started, Take};
    use crate::observed::{self, WalkOutcome};
    use crate::walk_wait;
    use serde_json::Value;

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
        machine::on_hold(false);
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

    fn fail_walk(seq: u64, token: u64, x: i32, z: i32, failed: bool) {
        observed::post(seq, |post| {
            post.session(true)
                .here(observed::Tile { x, z, level: 0 })
                .walk_outcome(WalkOutcome {
                    seq,
                    generation: 0,
                    request_id: token,
                    failed,
                    tile: observed::Tile {
                        x: 10,
                        z: 0,
                        level: 0,
                    },
                    radius: 0,
                    allow_teleports: false,
                });
        });
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
    fn nopath_without_attempts_backoffs_instead_of_ending() {
        reset();
        post_here(0, 0);
        let h = start(None);
        machine::step(&mut NoJs);
        let token = walk_token();
        fail_walk(1, token, 0, 0, true);
        machine::step(&mut NoJs);
        assert!(
            machine::merge_ops(Vec::new()).is_empty(),
            "undefined attempts backoffs (walkLadder.ts:86), does not one-shot"
        );
        assert_eq!(machine::take(h), Take::Pending);
    }

    #[test]
    fn three_no_progress_passes_stop_as_unreachable() {
        reset();
        post_here(0, 0);
        let h = start(None);
        for (seq, pass) in (1..).zip(0..UNREACHABLE_PASSES) {
            machine::step(&mut NoJs);
            let token = walk_token();
            fail_walk(seq, token, 0, 0, true);
            machine::step(&mut NoJs);
            if pass + 1 < UNREACHABLE_PASSES {
                let ticks = backoff_ticks(pass + 1);
                for _ in 0..ticks {
                    machine::step(&mut NoJs);
                }
            }
        }
        assert_eq!(machine::take(h), Take::Settled(Outcome::Done(json!(false))));
    }

    #[test]
    fn closer_tile_rebakes_after_nopath() {
        reset();
        post_here(0, 0);
        let h = start(Some(3));
        machine::step(&mut NoJs);
        let token = walk_token();
        fail_walk(1, token, 5, 0, true);
        machine::step(&mut NoJs);
        let retry = walk_token();
        assert_ne!(retry, 0);
        assert_ne!(retry, token);
        assert_eq!(machine::take(h), Take::Pending);
    }

    #[test]
    fn closest_terminal_without_arrival_is_not_success() {
        reset();
        post_here(0, 0);
        let h = start(None);
        machine::step(&mut NoJs);
        let token = walk_token();
        fail_walk(1, token, 0, 0, false);
        machine::step(&mut NoJs);
        assert_eq!(
            machine::take(h),
            Take::Pending,
            "N1: isArrived, not wait true"
        );
    }

    #[test]
    fn ours_after_nopath_returns_false() {
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
                .ours(true)
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
        assert_eq!(machine::take(h), Take::Settled(Outcome::Done(json!(false))));
    }

    #[test]
    fn guardian_hold_pauses_the_row() {
        reset();
        post_here(0, 0);
        let h = start(None);
        machine::step(&mut NoJs);
        let _token = walk_token();
        machine::on_hold(true);
        machine::step(&mut NoJs);
        assert!(machine::merge_ops(Vec::new()).is_empty());
        assert_eq!(machine::take(h), Take::Pending);
        machine::on_hold(false);
    }

    #[test]
    fn a_timed_out_walk_stops_the_host_follow() {
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
        assert_eq!(machine::take(h), Take::Pending);
    }
}
