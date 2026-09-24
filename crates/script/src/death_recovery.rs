//! Rust-owned DeathRecovery: the death latch and the recovery run.
//!
//! - The latch reads the posted game chat from the isolate scene: only a
//!   *new* death line (a seq past the first observation) latches; seeded,
//!   old and duplicate chat does not.
//! - `validate` is one typed helper (`load/bank_tasks_v8.rs`): it observes
//!   ([`observe`]), fires the caller's `onDeath` on a new latch, and, as
//!   frozen `validate` on every pass while the death is latched (and no
//!   run is live), clears the latch and fires `onRecovered` when the posted
//!   tile is within Chebyshev `radius` of the anchor ([`near`],
//!   [`recover`]); otherwise it answers due and `execute` runs (again). A
//!   death beside the anchor therefore recovers at once, with no run.
//! - The `death_recovery` [`crate::machine`] family is frozen `execute`:
//!   wait for the respawn (bounded 20 s), three ticks, then the caller's
//!   `walkBack` or a walk to the anchor. Neither result is read.

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

impl Latch {
    const fn new() -> Self {
        Self {
            latched: false,
            baseline: false,
            last_seq: 0,
        }
    }
}

thread_local! {
    static LATCH: RefCell<Latch> = const { RefCell::new(Latch::new()) };
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

/// What `validate` read: whether a new death latched now, and the posted
/// tile to check against the anchor (latched, no run live, a tile posted).
pub(crate) struct Observed {
    pub(crate) died: bool,
    pub(crate) check_from: Option<Tile>,
}

/// `DeathRecovery.validate`'s first half: observe the posted chat.
pub(crate) fn observe() -> Observed {
    let (died, here) = observed::with(|scene| {
        let session = scene.since_login();
        let lines = session.chat_lines().map(Vec::as_slice).unwrap_or_default();
        let died = LATCH.with(|latch| latch.borrow_mut().observe(lines));
        let here = session.here().map(|t| Tile {
            x: t.x,
            z: t.z,
            level: t.level,
        });
        (died, here)
    });
    let checkable = LATCH.with(|latch| {
        let latch = latch.borrow();
        latch.latched
    }) && !machine::live(DeathRecovery::NAME);
    Observed {
        died,
        check_from: here.filter(|_| checkable),
    }
}

/// Frozen `near(home, anchor, radius)`: the anchor's level strictly equal,
/// then `Math.abs(home.x - anchor.x) <= r` and the same for `z`, over the
/// caller's numbers (`None` level: not a number, never equal).
pub(crate) fn near(home: Tile, level: Option<f64>, x: f64, z: f64, radius: f64) -> bool {
    level == Some(f64::from(home.level))
        && (f64::from(home.x) - x).abs() <= radius
        && (f64::from(home.z) - z).abs() <= radius
}

/// Frozen `this.died = false` (the caller then fires `onRecovered`).
pub(crate) fn recover() {
    LATCH.with(|latch| latch.borrow_mut().latched = false);
}

/// `return this.died`, while no recovery run is live.
pub(crate) fn due() -> bool {
    latched() && !machine::live(DeathRecovery::NAME)
}

pub fn on_reset() {
    LATCH.with(|latch| *latch.borrow_mut() = Latch::new());
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

enum Phase {
    Respawn,
    Ticks(u64),
    WalkBack { asked: bool },
    Walk,
    Walking,
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
    /// Frozen awaits `walkBack`. `onRecovered` belongs to `validate`.
    const CALLBACKS: &'static [&'static str] = &["walkBack"];
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
                    // Frozen ignores walkBack's result: validate decides.
                    return Step::Done(Value::Null);
                }
                Phase::Walk => {
                    let Some(anchor) = self.anchor else {
                        return Step::Done(Value::Null);
                    };
                    // Frozen `walkResilient` returns at once when arrived.
                    if arrived(anchor, self.radius) {
                        return Step::Done(Value::Null);
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
                    // The walk's own result is not read: validate decides.
                    if arrived(anchor, self.radius) || cx.clock().bound_reached() {
                        return Step::Done(Value::Null);
                    }
                    return Step::Wait;
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
        let mut latch = Latch::new();
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

    struct NoJs;

    impl machine::Js for NoJs {
        fn queue_len(&mut self) -> usize {
            0
        }

        fn call(
            &mut self,
            _hook: Option<&crate::load::callback_v8::HeldCallback>,
            _args: &[Value],
        ) -> machine::Called {
            panic!("no hooks were passed");
        }

        fn poll(&mut self, _pending: &machine::Pending) -> Option<machine::Reply> {
            panic!("no hooks were passed");
        }

        fn claimed(&mut self) -> bool {
            false
        }
    }

    /// Frozen reads no result from the 20 s respawn wait: when it lapses
    /// with no respawn posted, three ticks later the walk still starts.
    #[test]
    fn a_lapsed_respawn_wait_still_walks_after_three_ticks() {
        machine::on_reset();
        observed::on_reset();
        on_reset();
        LATCH.with(|latch| latch.borrow_mut().latched = true);
        observed::post(1, |post| {
            post.session(false);
        });
        let started = machine::start(
            DeathRecovery::NAME,
            serde_json::json!({ "anchor": { "x": 3235, "z": 3295, "level": 0 }, "radius": 3 }),
            Vec::new(),
            0,
        );
        let machine::Started::Running(handle) = started else {
            panic!("expected a running row, got {started:?}");
        };
        machine::step(&mut NoJs);
        assert!(
            machine::merge_ops(Vec::new()).is_empty(),
            "waiting to respawn"
        );
        machine::tests::expire_deadlines();
        machine::step(&mut NoJs);
        for n in 2..=4 {
            observed::post(n, |post| {
                post.session(true).here(observed::Tile {
                    x: 3222,
                    z: 3218,
                    level: 0,
                });
            });
            machine::step(&mut NoJs);
            let ops = machine::merge_ops(Vec::new());
            if n < 4 {
                assert!(ops.is_empty(), "tick {n}: inside the three ticks");
            } else {
                assert!(
                    matches!(
                        ops.as_slice(),
                        [InteractReq::WalkNear {
                            x: 3235,
                            z: 3295,
                            radius: 3,
                            ..
                        }]
                    ),
                    "{ops:?}"
                );
            }
        }
        assert_eq!(machine::take(handle), machine::Take::Pending);
    }

    /// A scene with a wall between rows 9 and 10 (walk_wait's fixture).
    fn walled_view(here: Tile) -> api::query::ReachQueryView {
        use client::dash3d::CollisionFlag;
        let mut scene = api::snapshot::SceneView {
            available: true,
            base_x: 2810,
            base_z: 3546,
            level: 0,
            width: 20,
            height: 20,
            collision_flags: vec![0; 400],
        };
        for lx in 0..20 {
            scene.collision_flags[lx * 20 + 9] |= CollisionFlag::W_N;
            scene.collision_flags[lx * 20 + 10] |= CollisionFlag::W_S;
        }
        let flood = api::query::SceneQuery::new(&scene, Some(here)).flood_reach();
        api::query::pack_reach_query(&scene, flood.as_ref())
    }

    fn post_at(tick: u64, here: Tile, view: &api::query::ReachQueryView) {
        use crate::isolate_fb::{encode_snapshot, ReachViewInput, SnapshotReader, TileInput};
        let mut input = crate::isolate_fb::tests::empty_input(tick);
        input.here = Some(TileInput {
            x: here.x,
            z: here.z,
            level: here.level,
        });
        input.reach = ReachViewInput {
            available: view.available,
            base_x: view.base_x,
            base_z: view.base_z,
            level: view.level,
            width: view.width,
            height: view.height,
            walkable: &view.walkable,
            reachable: &view.reachable,
            reachable_adj: &view.reachable_adj,
            exact_rank: &view.exact_rank,
            adjacent_rank: &view.adjacent_rank,
            step: &view.step,
            canlight: &view.canlight,
            stamp: 1,
        };
        let bytes = encode_snapshot(&input);
        let snap = SnapshotReader::from_bytes(&bytes).expect("snapshot");
        observed::apply(&snap);
        crate::load::reach_query::apply(&snap);
    }

    /// A walk that ends within Chebyshev `radius` of the anchor but not
    /// `isArrived` (across a wall) is recovered by the next validate's
    /// frozen `near`, not walked again.
    #[test]
    fn a_walk_ending_near_but_not_arrived_recovers_at_the_next_validate() {
        machine::on_reset();
        observed::on_reset();
        crate::load::reach_query::on_reset();
        on_reset();
        let here = Tile {
            x: 2820,
            z: 3555,
            level: 0,
        };
        let anchor = Tile {
            x: 2820,
            z: 3557,
            level: 0,
        };
        let view = walled_view(here);
        post_at(1, here, &view);
        LATCH.with(|latch| latch.borrow_mut().latched = true);
        let started = machine::start(
            DeathRecovery::NAME,
            serde_json::json!({ "anchor": { "x": anchor.x, "z": anchor.z, "level": 0 }, "radius": 2 }),
            Vec::new(),
            0,
        );
        let machine::Started::Running(_) = started else {
            panic!("expected a running row, got {started:?}");
        };
        let mut walked = false;
        for tick in 1..=5 {
            post_at(tick, here, &view);
            machine::step(&mut NoJs);
            walked |= machine::merge_ops(Vec::new())
                .iter()
                .any(|op| matches!(op, InteractReq::WalkNear { .. }));
        }
        assert!(
            walked,
            "Chebyshev 2 across the wall is not arrived: it walks"
        );
        assert!(observe().check_from.is_none(), "no position check mid-run");
        machine::tests::expire_deadlines();
        machine::step(&mut NoJs);
        assert!(
            !machine::live(DeathRecovery::NAME),
            "the walk bound ended the run"
        );
        let check = observe().check_from.expect("a finished run is checked");
        assert!(due(), "still latched until validate decides");
        assert!(near(
            check,
            Some(0.0),
            f64::from(anchor.x),
            f64::from(anchor.z),
            2.0
        ));
        recover();
        assert!(!due(), "recovered: execute does not run again");
    }
}
