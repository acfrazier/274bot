//! Rust-owned Baker stall target/restock policy and one-attempt sequencing.
//! `api::cake_stall` owns the posted pins and row types; this module owns
//! selection, restock and stall-food predicates. The posted facts are read
//! from the isolate scene at call time; JavaScript supplies callbacks and
//! dispatches returned verbs.

use super::{counts_as_stall_food, needs_cake_restock, select_baker_stall};
use crate::observed::{self, Scene, SceneRow};
use api::cake_stall::{StallLoc, BAKER_STALL};
use serde_json::{json, Value};
use std::cell::RefCell;
use std::time::{Duration, Instant};

pub const WALK_BOUND_MS: u64 = 60_000;
pub const STEAL_RESOLVE_MS: u64 = 2_400;
/// Frozen `RESET_AFTER_REFUSALS`: consecutive steals that gained nothing and
/// were not caught (the stall owner was watching) before the other stand is
/// tried.
pub const RESET_AFTER_REFUSALS: u32 = 3;
/// Frozen `LOCKOUT_TICKS`: how long a steal refused for recent combat waits
/// before the next attempt.
pub const LOCKOUT_TICKS: i64 = 10;
/// Frozen `LOCKOUT_RE`: the server refuses stall steals within ten ticks of
/// combat with this line. It is neither a catch nor a watched-stand refusal.
const LOCKOUT_LINE: &str = "can't steal from the market stall during combat";

thread_local! {
    static RUNTIME: RefCell<CakeStallRuntime> = const { RefCell::new(CakeStallRuntime::new()) };
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Tile {
    x: i32,
    z: i32,
    level: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SelectedLoc {
    id: i32,
    x: i32,
    z: i32,
    level: i32,
}

struct Observation {
    ingame: bool,
    tick: i64,
    here: Option<Tile>,
    in_combat: bool,
    abort: bool,
    should_eat: bool,
    inv_size: i32,
    inv_len: usize,
    carried: i32,
    facts_valid: bool,
    stall: Option<SelectedLoc>,
    locked_out_until: Option<i64>,
    chat_max_seq: i32,
    lockout_seq: Option<i32>,
}

/// The posted facts this module decides from, read from the isolate scene.
struct NativeObservation {
    ingame: bool,
    tick: i64,
    here: Option<Tile>,
    in_combat: bool,
    inv_size: i32,
    inv_len: usize,
    carried: i32,
    stall: Option<SelectedLoc>,
    chat_max_seq: i32,
    lockout_seq: Option<i32>,
}

impl NativeObservation {
    /// A logout forgets the session: only pages posted since login count.
    /// The stall is picked only for the ops that walk to or steal from it.
    fn from_scene(scene: &Scene, pick_stall: bool) -> Self {
        let session = scene.since_login();
        let inv = session.inv();
        let lines = session.chat_lines();
        Self {
            ingame: session.ingame().unwrap_or(false),
            tick: scene
                .session_tick()
                .map_or(0, |tick| i64::try_from(tick).unwrap_or(i64::MAX)),
            here: session.here().map(|tile| Tile {
                x: tile.x,
                z: tile.z,
                level: tile.level,
            }),
            in_combat: session.in_combat().unwrap_or(false),
            inv_size: session.inv_size().unwrap_or(0),
            inv_len: inv.map_or(0, Vec::len),
            carried: inv.map_or(0, |rows| {
                sum_carried(
                    rows.iter()
                        .filter_map(|row| row.name.as_deref().map(|name| (name, row.count))),
                )
            }),
            stall: if pick_stall {
                session.locs().and_then(|locs| selected_stall(locs))
            } else {
                None
            },
            chat_max_seq: lines.map_or(-1, |lines| {
                lines.iter().map(|line| line.seq).max().unwrap_or(-1)
            }),
            lockout_seq: lines.and_then(|lines| {
                lines
                    .iter()
                    .filter(|line| contains_ascii_ci(&line.text, LOCKOUT_LINE))
                    .map(|line| line.seq)
                    .max()
            }),
        }
    }

    fn with_callbacks(&self, input: &Value) -> Observation {
        Observation {
            ingame: self.ingame,
            tick: self.tick,
            here: self.here,
            in_combat: self.in_combat,
            abort: input.get("abort").and_then(Value::as_bool).unwrap_or(false),
            should_eat: input
                .get("should_eat")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            inv_size: self.inv_size,
            inv_len: self.inv_len,
            carried: self.carried,
            facts_valid: input
                .get("facts_valid")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            stall: self.stall,
            locked_out_until: input.get("locked_out_until").and_then(Value::as_i64),
            chat_max_seq: self.chat_max_seq,
            lockout_seq: self.lockout_seq,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Idle,
    NeedInitialLockout,
    WaitStand,
    NeedAfterWalkCallbacks { arrived: bool },
    NeedAfterWalkLockout { arrived: bool },
    WaitSteal,
    NeedStealResult,
    AfterOnSteal,
    AfterOnReset,
}

struct CakeStallRuntime {
    paused: bool,
    held: bool,
    frozen_at: Option<Instant>,
    token: u64,
    phase: Phase,
    fill_to: Option<i32>,
    before: i32,
    deadline: Option<Instant>,
    /// The stand steals are made from, kept across passes the way the
    /// frozen `stealCakes` loop keeps it until it returns anything but a
    /// retry: `false` is the main stand, `true` the alternate.
    alt_stand: bool,
    /// Consecutive refused steals from the current stand.
    refusals: u32,
    /// Frozen `selfLockout`: the tick a steal refused for recent combat may
    /// be retried from, kept across passes like the stand.
    self_lockout_until: i64,
    /// Newest chat seq when the current steal was sent; only later lines
    /// resolve it.
    mark_seq: i32,
}

impl CakeStallRuntime {
    const fn new() -> Self {
        Self {
            paused: false,
            held: false,
            frozen_at: None,
            token: 0,
            phase: Phase::Idle,
            fill_to: None,
            before: 0,
            deadline: None,
            alt_stand: false,
            refusals: 0,
            self_lockout_until: 0,
            mark_seq: -1,
        }
    }

    fn frozen(&self) -> bool {
        self.paused || self.held
    }

    fn now(&self) -> Instant {
        self.frozen_at.unwrap_or_else(Instant::now)
    }

    fn set_freeze(&mut self, paused: bool, held: bool) {
        let was_frozen = self.frozen();
        self.paused = paused;
        self.held = held;
        let frozen = self.frozen();
        if !was_frozen && frozen {
            self.frozen_at = Some(Instant::now());
        } else if was_frozen && !frozen {
            if let Some(at) = self.frozen_at.take() {
                if let Some(deadline) = self.deadline.as_mut() {
                    *deadline += Instant::now().saturating_duration_since(at);
                }
            }
        }
    }

    fn abort_runtime(&mut self) {
        self.token = self.token.wrapping_add(1);
        self.phase = Phase::Idle;
        self.fill_to = None;
        self.before = 0;
        self.deadline = None;
    }

    fn stand(&self) -> Tile {
        let stand = if self.alt_stand {
            BAKER_STALL.stand_alt
        } else {
            BAKER_STALL.stand
        };
        Tile {
            x: stand.x,
            z: stand.z,
            level: stand.level,
        }
    }

    /// A session boundary or a pass that ended on anything but a retry: the
    /// next steal starts from the main stand with no refusals counted and no
    /// self-imposed lockout, as a fresh frozen `stealCakes` call does.
    fn reset_stand(&mut self) {
        self.alt_stand = false;
        self.refusals = 0;
        self.self_lockout_until = 0;
    }

    fn done(&mut self, result: &str, stole: bool) -> Value {
        self.phase = Phase::Idle;
        self.deadline = None;
        if result != "no-progress" {
            self.reset_stand();
        }
        json!({
            "kind": "done",
            "token": self.token,
            "result": result,
            "stole": stole,
        })
    }

    fn begin(&mut self, fill_to: Option<i32>, obs: &Observation) -> Value {
        self.abort_runtime();
        self.fill_to = fill_to;
        if let Some(result) = self.callback_gate_result(obs) {
            return self.done(result, false);
        }
        self.phase = Phase::NeedInitialLockout;
        self.observe(false, true)
    }

    fn callback_gate_result(&self, obs: &Observation) -> Option<&'static str> {
        if !obs.ingame || obs.abort || obs.should_eat {
            return Some("aborted");
        }
        if obs.in_combat {
            return Some("combat");
        }
        if at_goal(obs, self.fill_to) {
            return Some("stocked");
        }
        None
    }

    fn observe(&self, callbacks: bool, lockout: bool) -> Value {
        json!({
            "kind": "observe",
            "token": self.token,
            "callbacks": callbacks,
            "lockout": lockout,
        })
    }

    fn start_after_lockout(&mut self, obs: &Observation, arrived: Option<bool>) -> Value {
        let until = obs
            .locked_out_until
            .unwrap_or(0)
            .max(self.self_lockout_until);
        if obs.tick < until {
            return self.done("no-progress", false);
        }
        if arrived == Some(false) || !obs.facts_valid {
            return self.done("no-progress", false);
        }
        let stand = self.stand();
        if obs.here == Some(stand) {
            return self.start_steal(obs);
        }
        self.phase = Phase::WaitStand;
        self.deadline = Some(self.now() + Duration::from_millis(WALK_BOUND_MS));
        json!({
            "kind": "walk-to",
            "token": self.token,
            "x": stand.x,
            "z": stand.z,
            "level": stand.level,
        })
    }

    fn start_steal(&mut self, obs: &Observation) -> Value {
        let Some(loc) = obs.stall else {
            return self.done("no-progress", false);
        };
        self.before = obs.carried;
        self.mark_seq = obs.chat_max_seq;
        self.phase = Phase::WaitSteal;
        self.deadline = Some(self.now() + Duration::from_millis(STEAL_RESOLVE_MS));
        json!({
            "kind": "loc",
            "token": self.token,
            "x": loc.x,
            "z": loc.z,
            "level": loc.level,
            "id": loc.id,
            "action": BAKER_STALL.op,
        })
    }

    fn next(&mut self, token: u64, obs: &Observation) -> Value {
        if token != self.token || self.phase == Phase::Idle {
            return json!({"kind": "aborted", "token": self.token});
        }
        if self.frozen() {
            return json!({"kind": "wait", "token": self.token});
        }
        match self.phase {
            Phase::Idle => json!({"kind": "aborted", "token": self.token}),
            Phase::NeedInitialLockout => self.start_after_lockout(obs, None),
            Phase::WaitStand => {
                if !obs.ingame {
                    return self.done("aborted", false);
                }
                let arrived = obs.here == Some(self.stand());
                if arrived || self.expired() {
                    self.phase = Phase::NeedAfterWalkCallbacks { arrived };
                    self.observe(true, false)
                } else {
                    json!({"kind": "wait", "token": self.token})
                }
            }
            Phase::NeedAfterWalkCallbacks { arrived } => {
                if let Some(result) = self.callback_gate_result(obs) {
                    return self.done(result, false);
                }
                self.phase = Phase::NeedAfterWalkLockout { arrived };
                self.observe(false, true)
            }
            Phase::NeedAfterWalkLockout { arrived } => self.start_after_lockout(obs, Some(arrived)),
            Phase::WaitSteal => {
                let gained = obs.carried > self.before;
                if !obs.ingame
                    || obs.abort
                    || obs.should_eat
                    || obs.in_combat
                    || gained
                    || self.lockout_seen(obs)
                    || self.expired()
                {
                    self.phase = Phase::NeedStealResult;
                    self.observe(true, false)
                } else {
                    json!({"kind": "wait", "token": self.token})
                }
            }
            Phase::NeedStealResult => {
                if !obs.ingame || obs.abort || obs.should_eat {
                    return self.done("aborted", false);
                }
                if obs.carried > self.before {
                    self.refusals = 0;
                    self.phase = Phase::AfterOnSteal;
                    return json!({"kind": "on-steal", "token": self.token});
                }
                if obs.in_combat {
                    return self.done("combat", false);
                }
                if at_goal(obs, self.fill_to) {
                    return self.done("stocked", false);
                }
                // Refused for recent combat: wait the lockout out, and do not
                // count it against the stand.
                if self.lockout_seen(obs) {
                    self.self_lockout_until = obs.tick.saturating_add(LOCKOUT_TICKS);
                    return self.done("no-progress", false);
                }
                // Nothing gained and not caught: the owner was watching.
                self.refusals += 1;
                if self.refusals >= RESET_AFTER_REFUSALS {
                    let refusals = self.refusals;
                    self.alt_stand = !self.alt_stand;
                    self.refusals = 0;
                    self.phase = Phase::AfterOnReset;
                    let stand = self.stand();
                    return json!({
                        "kind": "on-reset",
                        "token": self.token,
                        "status": "watched — swapping stands",
                        "log": format!(
                            "{refusals} refused steals — swapping to the stand at ({},{})",
                            stand.x, stand.z
                        ),
                    });
                }
                self.done("no-progress", false)
            }
            Phase::AfterOnReset => self.done("no-progress", false),
            Phase::AfterOnSteal => {
                if !obs.ingame {
                    return self.done("aborted", true);
                }
                if obs.in_combat {
                    return self.done("combat", true);
                }
                if at_goal(obs, self.fill_to) {
                    return self.done("stocked", true);
                }
                self.done("no-progress", true)
            }
        }
    }

    fn expired(&self) -> bool {
        self.deadline.is_some_and(|deadline| self.now() >= deadline)
    }

    fn lockout_seen(&self, obs: &Observation) -> bool {
        obs.lockout_seq.is_some_and(|seq| seq > self.mark_seq)
    }
}

/// ASCII case-insensitive substring test, without the copy.
fn contains_ascii_ci(hay: &str, needle: &str) -> bool {
    hay.as_bytes()
        .windows(needle.len())
        .any(|window| window.eq_ignore_ascii_case(needle.as_bytes()))
}

fn sum_carried<'a>(inv: impl IntoIterator<Item = (&'a str, i32)>) -> i32 {
    inv.into_iter()
        .filter(|(name, _)| counts_as_stall_food(name))
        .fold(0_i32, |sum, (_, count)| sum.saturating_add(count))
}

fn pack_full(obs: &Observation) -> bool {
    obs.inv_size > 0 && obs.inv_len >= obs.inv_size as usize
}

fn at_goal(obs: &Observation, fill_to: Option<i32>) -> bool {
    pack_full(obs) || fill_to.is_some_and(|target| obs.carried >= target)
}

fn selected_stall(locs: &[SceneRow]) -> Option<SelectedLoc> {
    // `select_baker_stall` only takes the pinned loc id, so only those rows
    // are viewed.
    let pinned: Vec<&SceneRow> = locs
        .iter()
        .filter(|loc| loc.id == BAKER_STALL.loc_id)
        .collect();
    let action_refs: Vec<Vec<&str>> = pinned
        .iter()
        .map(|loc| loc.actions.iter().map(|action| &**action).collect())
        .collect();
    let views: Vec<StallLoc<'_>> = pinned
        .iter()
        .zip(action_refs.iter())
        .map(|(loc, actions)| StallLoc {
            id: loc.id,
            name: loc.name.as_deref(),
            x: loc.x,
            z: loc.z,
            level: loc.level,
            distance: loc.distance,
            actions,
        })
        .collect();
    select_baker_stall(Some(&BAKER_STALL), &views).map(|loc| SelectedLoc {
        id: loc.id,
        x: loc.x,
        z: loc.z,
        level: loc.level,
    })
}

pub fn on_pause() {
    RUNTIME.with(|runtime| {
        let held = runtime.borrow().held;
        runtime.borrow_mut().set_freeze(true, held);
    });
}

pub fn on_resume() {
    RUNTIME.with(|runtime| {
        let held = runtime.borrow().held;
        runtime.borrow_mut().set_freeze(false, held);
    });
}

pub fn on_hold(held: bool) {
    RUNTIME.with(|runtime| {
        let paused = runtime.borrow().paused;
        runtime.borrow_mut().set_freeze(paused, held);
    });
}

pub fn on_reset() {
    RUNTIME.with(|runtime| {
        let mut runtime = runtime.borrow_mut();
        runtime.abort_runtime();
        runtime.reset_stand();
    });
}

pub fn dispatch(input: &Value) -> Value {
    let observe = |pick_stall: bool| {
        observed::with(|scene| NativeObservation::from_scene(scene, pick_stall))
            .with_callbacks(input)
    };
    match input.get("op").and_then(Value::as_str).unwrap_or("") {
        "count" => json!(observe(false).carried),
        "needs_restock" => {
            let obs = observe(false);
            json!(needs_cake_restock(
                obs.carried,
                input
                    .get("target")
                    .and_then(Value::as_i64)
                    .map(|n| n as i32),
                pack_full(&obs),
            ))
        }
        "begin" => RUNTIME.with(|runtime| {
            runtime.borrow_mut().begin(
                input
                    .get("fill_to")
                    .and_then(Value::as_i64)
                    .map(|n| n as i32),
                &observe(true),
            )
        }),
        "next" => RUNTIME.with(|runtime| {
            runtime.borrow_mut().next(
                input.get("token").and_then(Value::as_u64).unwrap_or(0),
                &observe(true),
            )
        }),
        "current_token" => RUNTIME.with(|runtime| json!(runtime.borrow().token)),
        _ => json!({"kind": "done", "result": "no-progress", "stole": false}),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn observation() -> Observation {
        Observation {
            ingame: true,
            tick: 1,
            here: Some(Tile {
                x: BAKER_STALL.stand.x,
                z: BAKER_STALL.stand.z,
                level: BAKER_STALL.stand.level,
            }),
            in_combat: false,
            abort: false,
            should_eat: false,
            inv_size: 28,
            inv_len: 0,
            carried: 0,
            facts_valid: true,
            stall: Some(SelectedLoc {
                id: BAKER_STALL.loc_id,
                x: BAKER_STALL.stall.x,
                z: BAKER_STALL.stall.z,
                level: BAKER_STALL.stall.level,
            }),
            locked_out_until: None,
            chat_max_seq: -1,
            lockout_seq: None,
        }
    }

    #[test]
    fn native_predicates_own_count_and_restock() {
        let mut obs = observation();
        assert_eq!(
            sum_carried([("Chocolate cake", 1), ("Chocolate slice", 2)]),
            2
        );
        assert!(needs_cake_restock(obs.carried, Some(1), pack_full(&obs)));
        obs.carried = 2;
        assert!(!needs_cake_restock(obs.carried, Some(1), pack_full(&obs)));
    }

    #[test]
    fn native_bounds_timeout_and_reset_are_fail_closed() {
        assert_eq!(WALK_BOUND_MS, 60_000);
        assert_eq!(STEAL_RESOLVE_MS, 2_400);
        let mut runtime = CakeStallRuntime::new();
        let begin = runtime.begin(Some(28), &observation());
        let token = begin["token"].as_u64().unwrap();
        assert_eq!(runtime.next(token, &observation())["kind"], "loc");
        runtime.deadline = Some(Instant::now() - Duration::from_millis(1));
        assert_eq!(runtime.next(token, &observation())["kind"], "observe");
        assert_eq!(runtime.next(token, &observation())["result"], "no-progress");
        let begin = runtime.begin(Some(28), &observation());
        let stale = begin["token"].as_u64().unwrap();
        runtime.abort_runtime();
        assert_eq!(runtime.next(stale, &observation())["kind"], "aborted");
    }

    /// One steal pass from `obs`: begin, clear the lockout, steal, and let
    /// the resolve window expire with nothing gained.
    fn refused_pass(runtime: &mut CakeStallRuntime, obs: &Observation) -> Value {
        let token = runtime.begin(Some(28), obs)["token"].as_u64().unwrap();
        let steal = runtime.next(token, obs);
        assert_eq!(steal["kind"], "loc", "{steal}");
        runtime.deadline = Some(Instant::now() - Duration::from_millis(1));
        assert_eq!(runtime.next(token, obs)["kind"], "observe");
        runtime.next(token, obs)
    }

    fn at(stand: api::snapshot::WorldTile) -> Observation {
        Observation {
            here: Some(Tile {
                x: stand.x,
                z: stand.z,
                level: stand.level,
            }),
            ..observation()
        }
    }

    #[test]
    fn three_refused_steals_move_to_the_other_stand() {
        let mut runtime = CakeStallRuntime::new();
        let main = at(BAKER_STALL.stand);
        for _ in 1..RESET_AFTER_REFUSALS {
            assert_eq!(refused_pass(&mut runtime, &main)["result"], "no-progress");
        }
        let reset = refused_pass(&mut runtime, &main);
        assert_eq!(reset["kind"], "on-reset", "{reset}");
        assert!(
            reset["log"].as_str().unwrap().contains("(2669,3310)"),
            "{reset}"
        );
        let token = reset["token"].as_u64().unwrap();
        assert_eq!(runtime.next(token, &main)["result"], "no-progress");

        // The next pass does not steal from the watched stand again.
        let token = runtime.begin(Some(28), &main)["token"].as_u64().unwrap();
        let walk = runtime.next(token, &main);
        assert_eq!(walk["kind"], "walk-to", "{walk}");
        assert_eq!(
            (walk["x"].as_i64(), walk["z"].as_i64()),
            (Some(2669), Some(3310))
        );
        let alt = at(BAKER_STALL.stand_alt);
        assert_eq!(runtime.next(token, &alt)["kind"], "observe");
        assert_eq!(runtime.next(token, &alt)["kind"], "observe");
        assert_eq!(runtime.next(token, &alt)["kind"], "loc");
    }

    #[test]
    fn a_steal_clears_the_count_and_combat_returns_to_the_main_stand() {
        let mut runtime = CakeStallRuntime::new();
        let main = at(BAKER_STALL.stand);
        for _ in 1..RESET_AFTER_REFUSALS {
            refused_pass(&mut runtime, &main);
        }
        // A steal that gains food clears the refusals.
        let token = runtime.begin(Some(28), &main)["token"].as_u64().unwrap();
        assert_eq!(runtime.next(token, &main)["kind"], "loc");
        let gained = Observation {
            carried: 1,
            ..at(BAKER_STALL.stand)
        };
        assert_eq!(runtime.next(token, &gained)["kind"], "observe");
        assert_eq!(runtime.next(token, &gained)["kind"], "on-steal");
        assert_eq!(runtime.next(token, &gained)["result"], "no-progress");
        for _ in 1..RESET_AFTER_REFUSALS {
            assert_eq!(refused_pass(&mut runtime, &main)["result"], "no-progress");
        }
        assert_eq!(refused_pass(&mut runtime, &main)["kind"], "on-reset");

        // Caught while on the alternate stand: the next pass starts over
        // from the main stand, as a fresh frozen `stealCakes` call does.
        let alt = at(BAKER_STALL.stand_alt);
        let token = runtime.begin(Some(28), &alt)["token"].as_u64().unwrap();
        assert_eq!(runtime.next(token, &alt)["kind"], "loc");
        let caught = Observation {
            in_combat: true,
            ..at(BAKER_STALL.stand_alt)
        };
        assert_eq!(runtime.next(token, &caught)["kind"], "observe");
        assert_eq!(runtime.next(token, &caught)["result"], "combat");
        let token = runtime.begin(Some(28), &alt)["token"].as_u64().unwrap();
        let walk = runtime.next(token, &alt);
        assert_eq!(walk["kind"], "walk-to", "{walk}");
        assert_eq!(
            (walk["x"].as_i64(), walk["z"].as_i64()),
            (Some(2668), Some(3312))
        );
    }

    /// The server's "can't steal ... during combat" line resolves the steal
    /// at once, holds the next steal for `LOCKOUT_TICKS` from the same stand,
    /// and is not a watched-stand refusal (frozen `classifySteal` 'lockout').
    #[test]
    fn combat_lockout_line_waits_ten_ticks_and_is_not_a_refusal() {
        let mut runtime = CakeStallRuntime::new();
        let main = at(BAKER_STALL.stand);
        for _ in 1..RESET_AFTER_REFUSALS {
            assert_eq!(refused_pass(&mut runtime, &main)["result"], "no-progress");
        }
        let locked_at = |tick: i64| Observation {
            tick,
            chat_max_seq: 0,
            lockout_seq: Some(0),
            ..at(BAKER_STALL.stand)
        };

        let token = runtime.begin(Some(28), &main)["token"].as_u64().unwrap();
        assert_eq!(runtime.next(token, &main)["kind"], "loc");
        // The line resolves the steal before its resolve window runs out.
        assert_eq!(runtime.next(token, &locked_at(5))["kind"], "observe");
        assert_eq!(runtime.next(token, &locked_at(5))["result"], "no-progress");

        for tick in 5..5 + LOCKOUT_TICKS {
            let token = runtime.begin(Some(28), &locked_at(tick))["token"]
                .as_u64()
                .unwrap();
            let held = runtime.next(token, &locked_at(tick));
            assert_eq!(held["result"], "no-progress", "tick {tick}: {held}");
        }

        // Same stand after the lockout; the old line does not resolve it.
        let after = locked_at(5 + LOCKOUT_TICKS);
        let token = runtime.begin(Some(28), &after)["token"].as_u64().unwrap();
        assert_eq!(runtime.next(token, &after)["kind"], "loc");
        assert_eq!(runtime.next(token, &after)["kind"], "wait");
        // Only now is the third refusal counted.
        runtime.deadline = Some(Instant::now() - Duration::from_millis(1));
        assert_eq!(runtime.next(token, &after)["kind"], "observe");
        assert_eq!(runtime.next(token, &after)["kind"], "on-reset");
    }
}
