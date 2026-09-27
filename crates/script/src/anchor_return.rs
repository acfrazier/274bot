//! Frozen `createReturnToAnchorTask(...).execute` (`Anchor.ts:101–127`) as
//! the `return-to-anchor` [`crate::machine`] family.
//!
//! JavaScript sets the status, resolves the camp tile and awaits one run.
//! Rust owns the order of the frozen legs:
//! - inside the arrive disk (`Tile.distanceTo <= arriveRadius`) nothing
//!   walks (`Anchor.ts:106–109`);
//! - past `longRangeTiles` (when positive) one `walkResilient(anchor,
//!   { radius: arriveRadius, timeoutMs })` leg runs first, and ends the task
//!   when it lands in the disk (`Anchor.ts:110–120`);
//! - with `obstacles` the approach is `walkOpening(anchor, arriveRadius,
//!   obstacles)` (`Anchor.ts:121–124`);
//! - otherwise one baked `Traversal.walkTo(anchor, { radius: arriveRadius,
//!   timeoutMs })` (`Anchor.ts:125`), `timeoutMs` defaulting to 90 s
//!   (`Anchor.ts:89`).
//!
//! The resilient and walkTo legs log with the frozen two-space prefix;
//! walkOpening logs bare (`Anchor.ts:103, 114, 122, 125`). Every walking
//! tick pumps the `sustain` hook, as frozen `WalkExecutor.followPath` and the
//! ladder run `Sustain.run()` each iteration (`WalkExecutor.ts:853`,
//! `Traversal.ts:130`). Frozen `execute` resolves `void`.

use crate::machine::{Begin, Call, Cx, Family, Reply, Step};
use crate::walk::{here, interrupted, Resilient, Walk, WalkOpening};
use api::snapshot::WorldTile;
use serde::Deserialize;
use serde_json::json;
use std::collections::VecDeque;

/// Frozen `opts.arriveRadius ?? 8` (`Anchor.ts:88`).
const ARRIVE_RADIUS: i32 = 8;
/// Frozen `opts.timeoutMs ?? 90_000` (`Anchor.ts:89`).
const TIMEOUT_MS: u64 = 90_000;
const LOG: usize = 0;
const SUSTAIN: usize = 1;

#[derive(Clone, Copy, Deserialize)]
struct Tile {
    x: i32,
    z: i32,
    #[serde(default)]
    level: i32,
}

fn default_arrive_radius() -> i32 {
    ARRIVE_RADIUS
}

fn default_timeout_ms() -> u64 {
    TIMEOUT_MS
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ReturnToAnchorArgs {
    anchor: Tile,
    #[serde(default = "default_arrive_radius")]
    arrive_radius: i32,
    #[serde(default = "default_timeout_ms")]
    timeout_ms: u64,
    #[serde(default)]
    obstacles: Vec<String>,
    #[serde(default)]
    long_range_tiles: i32,
}

/// Frozen `Tile.distanceTo` (`Tile.ts:18–25`): Chebyshev, far off another
/// level.
fn distance_to(a: WorldTile, b: WorldTile) -> i32 {
    let xz = (a.x - b.x).abs().max((a.z - b.z).abs());
    if a.level == b.level {
        xz
    } else {
        1_000_000 + xz
    }
}

enum Leg {
    Start,
    Long(Resilient),
    Opening(WalkOpening),
    WalkTo(Walk),
}

/// One frozen return-to-anchor `execute`.
pub(crate) struct ReturnToAnchor {
    anchor: WorldTile,
    arrive_radius: i32,
    timeout_ms: u64,
    obstacles: Vec<String>,
    long_range_tiles: i32,
    leg: Leg,
    logs: VecDeque<String>,
    done: bool,
    /// This tick's advance already ended waiting (a log was written after).
    waiting: bool,
    pumped: bool,
}

impl Family for ReturnToAnchor {
    const NAME: &'static str = "return-to-anchor";
    const CALLBACKS: &'static [&'static str] = &["log", "sustain"];
    /// Frozen `host.log?.(m)` is not awaited; `await Sustain.run()` is.
    const SYNC_HOOKS: &'static [usize] = &[LOG];
    /// The first walk goes out in the caller's turn.
    const KICK_ON_START: bool = true;
    type Args = ReturnToAnchorArgs;
    type Output = ();

    fn begin(args: ReturnToAnchorArgs, _cx: &mut Cx<'_>) -> Begin<Self> {
        let anchor = WorldTile {
            x: args.anchor.x,
            z: args.anchor.z,
            level: args.anchor.level,
        };
        // Already inside the arrive disk: do not micro-walk the pin.
        if here().is_some_and(|me| distance_to(anchor, me) <= args.arrive_radius) {
            return Begin::Done(());
        }
        Begin::Run(Self {
            anchor,
            arrive_radius: args.arrive_radius,
            timeout_ms: args.timeout_ms,
            // Frozen `.map(s => s.trim().toLowerCase()).filter(Boolean)`.
            obstacles: args
                .obstacles
                .iter()
                .map(|name| name.trim().to_lowercase())
                .filter(|name| !name.is_empty())
                .collect(),
            long_range_tiles: args.long_range_tiles,
            leg: Leg::Start,
            logs: VecDeque::new(),
            done: false,
            waiting: false,
            pumped: false,
        })
    }

    fn step(&mut self, cx: &mut Cx<'_>) -> Step<()> {
        if let Some(Reply::Threw(thrown)) = cx.reply() {
            return Step::Fail(thrown);
        }
        loop {
            if let Some(line) = self.logs.pop_front() {
                if cx.has(LOG) {
                    return Step::Call(Call {
                        hook: LOG,
                        args: vec![json!(line)],
                    });
                }
                continue;
            }
            if self.done {
                return Step::Done(());
            }
            if std::mem::take(&mut self.waiting) {
                self.pumped = false;
                return Step::Wait;
            }
            if !self.pumped && cx.has(SUSTAIN) {
                self.pumped = true;
                return Step::Call(Call {
                    hook: SUSTAIN,
                    args: Vec::new(),
                });
            }
            if self.advance(cx) {
                self.done = true;
            } else if self.logs.is_empty() {
                self.pumped = false;
                return Step::Wait;
            } else {
                self.waiting = true;
            }
        }
    }
}

impl ReturnToAnchor {
    /// `true` once the frozen `execute` returned.
    fn advance(&mut self, cx: &mut Cx<'_>) -> bool {
        match std::mem::replace(&mut self.leg, Leg::Start) {
            Leg::Start => {
                let far = here().is_some_and(|me| {
                    self.long_range_tiles > 0
                        && distance_to(self.anchor, me) > self.long_range_tiles
                });
                if !far {
                    return self.approach(cx);
                }
                let long = Resilient::new(
                    self.anchor,
                    self.arrive_radius,
                    self.timeout_ms,
                    None,
                    false,
                );
                match long.start(cx) {
                    Ok(long) => {
                        self.leg = Leg::Long(long);
                        false
                    }
                    Err(_) => self.after_long(cx),
                }
            }
            Leg::Long(mut long) => {
                let out = long.step(cx);
                while let Some(line) = long.pop_log() {
                    self.logs.push_back(format!("  {line}"));
                }
                if out.is_none() {
                    self.leg = Leg::Long(long);
                    return false;
                }
                self.after_long(cx)
            }
            Leg::Opening(mut opening) => {
                let out = opening.advance(cx);
                while let Some(line) = opening.pop_log() {
                    self.logs.push_back(line);
                }
                if out.is_none() {
                    self.leg = Leg::Opening(opening);
                    return false;
                }
                true
            }
            Leg::WalkTo(walk) => {
                // Frozen `followPath` yields to a pending event
                // (`WalkExecutor.ts:845–847, 360–363`).
                if interrupted() {
                    walk.abort(cx);
                    self.logs
                        .push_back("  walk interrupted — a random event is being handled".into());
                    return true;
                }
                if walk.step(cx).is_none() {
                    self.leg = Leg::WalkTo(walk);
                    return false;
                }
                true
            }
        }
    }

    /// Frozen `Anchor.ts:116–119`: the long leg that landed in the disk ends
    /// the task; otherwise the local approach follows.
    fn after_long(&mut self, cx: &mut Cx<'_>) -> bool {
        if here().is_some_and(|me| distance_to(self.anchor, me) <= self.arrive_radius) {
            return true;
        }
        self.approach(cx)
    }

    /// Frozen `Anchor.ts:121–125`.
    fn approach(&mut self, cx: &mut Cx<'_>) -> bool {
        if !self.obstacles.is_empty() {
            self.leg = Leg::Opening(WalkOpening::new(
                self.anchor,
                self.arrive_radius,
                std::mem::take(&mut self.obstacles),
            ));
            return self.advance(cx);
        }
        match Walk::begin(self.anchor, self.arrive_radius, self.timeout_ms, false, cx) {
            Ok(walk) => {
                self.leg = Leg::WalkTo(walk);
                false
            }
            Err(_) => true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::machine::{self, Outcome, Started, Take};
    use crate::shim::InteractReq;
    use crate::walk::tests::{
        door_loc, post_here, post_walk_outcome, post_walled_scene, reset, NoJs,
    };
    use serde_json::{json, Value};

    const ANCHOR: WorldTile = WorldTile {
        x: 10,
        z: 0,
        level: 0,
    };

    fn start(opts: Value) -> Started {
        let mut args = json!({ "anchor": { "x": ANCHOR.x, "z": ANCHOR.z, "level": 0 } });
        for (key, value) in opts.as_object().expect("opts object") {
            args[key] = value.clone();
        }
        machine::start("return-to-anchor", args, Vec::new(), 0)
    }

    fn running(opts: Value) -> machine::Handle {
        match start(opts) {
            Started::Running(h) => h,
            other => panic!("return-to-anchor runs, got {other:?}"),
        }
    }

    /// The one walk this step sent toward the anchor: `(radius, token)`.
    fn anchor_walk() -> (i32, u64) {
        match machine::merge_ops(Vec::new()).as_slice() {
            [InteractReq::WalkNear {
                x: 10,
                z: 0,
                radius,
                request_id,
                ..
            }] => (*radius, *request_id),
            other => panic!("expected the walk to the anchor, got {other:?}"),
        }
    }

    fn fail(seq: u64, token: u64, x: i32, z: i32) {
        post_walk_outcome(seq, token, WorldTile { x, z, level: 0 }, ANCHOR, 2, true);
    }

    #[test]
    fn inside_the_arrive_disk_does_not_walk() {
        reset();
        post_here(4, 0);
        assert!(matches!(
            start(json!({})),
            Started::Settled(Outcome::Done(Value::Null))
        ));
        assert!(machine::merge_ops(Vec::new()).is_empty());
    }

    #[test]
    fn past_long_range_the_resilient_leg_walks_first() {
        reset();
        post_here(-40, 0);
        let h = running(json!({ "arriveRadius": 2, "longRangeTiles": 30 }));
        machine::step(&mut NoJs);
        let (radius, token) = anchor_walk();
        assert_eq!(radius, 2);
        fail(2, token, -40, 0);
        machine::step(&mut NoJs);
        assert_eq!(
            machine::merge_ops(Vec::new()),
            vec![InteractReq::WalkTo {
                x: 8,
                z: 0,
                level: 0
            }],
            "the long leg is walkResilient: a failed baked walk takes its scene step"
        );
        assert_eq!(machine::take(h), Take::Pending);
    }

    #[test]
    fn a_long_leg_that_lands_in_the_disk_ends_the_task() {
        reset();
        post_here(-40, 0);
        let h = running(json!({ "arriveRadius": 2, "longRangeTiles": 30 }));
        machine::step(&mut NoJs);
        let (_, token) = anchor_walk();
        post_walk_outcome(
            2,
            token,
            WorldTile {
                x: 9,
                z: 0,
                level: 0,
            },
            ANCHOR,
            2,
            false,
        );
        machine::step(&mut NoJs);
        assert!(
            machine::merge_ops(Vec::new()).is_empty(),
            "no local approach"
        );
        assert_eq!(machine::take(h), Take::Settled(Outcome::Done(Value::Null)));
    }

    #[test]
    fn obstacles_take_the_walk_opening_approach() {
        reset();
        post_walled_scene(vec![door_loc(1530, 2, 0)]);
        let h = running(json!({ "arriveRadius": 0, "obstacles": [" DOOR ", ""] }));
        machine::step(&mut NoJs);
        let token = match machine::merge_ops(Vec::new()).as_slice() {
            [InteractReq::Walk {
                x: 10,
                z: 0,
                request_id,
                ..
            }] => *request_id,
            other => panic!("expected the first walkOpening segment, got {other:?}"),
        };
        post_walk_outcome(
            2,
            token,
            WorldTile {
                x: 0,
                z: 0,
                level: 0,
            },
            ANCHOR,
            0,
            true,
        );
        machine::step(&mut NoJs);
        assert!(
            matches!(
                machine::merge_ops(Vec::new()).as_slice(),
                [InteractReq::WalkNear {
                    x: 2,
                    z: 0,
                    radius: 1,
                    ..
                }]
            ),
            "the trimmed, lower-cased obstacle names the door walkOpening walks to"
        );
        assert_eq!(machine::take(h), Take::Pending);
    }

    #[test]
    fn the_plain_approach_is_one_walk_to_with_the_frozen_90_s_bound() {
        reset();
        post_here(0, 0);
        let h = running(json!({ "arriveRadius": 2 }));
        machine::step(&mut NoJs);
        let (radius, token) = anchor_walk();
        assert_eq!(radius, 2);
        machine::age(h, 60_001);
        machine::step(&mut NoJs);
        assert!(
            machine::merge_ops(Vec::new()).is_empty(),
            "still walking after 60 s"
        );
        assert_eq!(machine::take(h), Take::Pending);
        machine::age(h, 30_000);
        machine::step(&mut NoJs);
        assert_eq!(
            machine::merge_ops(Vec::new()),
            vec![InteractReq::AbortWalk { request_id: token }]
        );
        assert_eq!(machine::take(h), Take::Settled(Outcome::Done(Value::Null)));
    }

    #[test]
    fn a_failed_plain_approach_ends_without_the_ladder() {
        reset();
        post_here(0, 0);
        let h = running(json!({ "arriveRadius": 2 }));
        machine::step(&mut NoJs);
        let (_, token) = anchor_walk();
        fail(2, token, 0, 0);
        machine::step(&mut NoJs);
        assert!(
            machine::merge_ops(Vec::new()).is_empty(),
            "frozen walkTo has no scene step"
        );
        assert_eq!(machine::take(h), Take::Settled(Outcome::Done(Value::Null)));
    }
}
