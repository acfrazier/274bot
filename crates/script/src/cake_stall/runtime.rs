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
}

impl NativeObservation {
    /// A logout forgets the session: only pages posted since login count.
    /// The stall is picked only for the ops that walk to or steal from it.
    fn from_scene(scene: &Scene, pick_stall: bool) -> Self {
        let session = scene.since_login();
        let inv = session.inv();
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

    fn done(&mut self, result: &str, stole: bool) -> Value {
        self.phase = Phase::Idle;
        self.deadline = None;
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
        if obs.tick < obs.locked_out_until.unwrap_or(0) {
            return self.done("no-progress", false);
        }
        if arrived == Some(false) || !obs.facts_valid {
            return self.done("no-progress", false);
        }
        if on_stand(obs.here) {
            return self.start_steal(obs);
        }
        self.phase = Phase::WaitStand;
        self.deadline = Some(self.now() + Duration::from_millis(WALK_BOUND_MS));
        json!({
            "kind": "walk-to",
            "token": self.token,
            "x": BAKER_STALL.stand.x,
            "z": BAKER_STALL.stand.z,
            "level": BAKER_STALL.stand.level,
        })
    }

    fn start_steal(&mut self, obs: &Observation) -> Value {
        let Some(loc) = obs.stall else {
            return self.done("no-progress", false);
        };
        self.before = obs.carried;
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
                let arrived = on_stand(obs.here);
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
                    self.phase = Phase::AfterOnSteal;
                    return json!({"kind": "on-steal", "token": self.token});
                }
                if obs.in_combat {
                    return self.done("combat", false);
                }
                if at_goal(obs, self.fill_to) {
                    return self.done("stocked", false);
                }
                self.done("no-progress", false)
            }
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

fn on_stand(here: Option<Tile>) -> bool {
    here.is_some_and(|here| {
        let stand = Tile {
            x: BAKER_STALL.stand.x,
            z: BAKER_STALL.stand.z,
            level: BAKER_STALL.stand.level,
        };
        let alt = Tile {
            x: BAKER_STALL.stand_alt.x,
            z: BAKER_STALL.stand_alt.z,
            level: BAKER_STALL.stand_alt.level,
        };
        here == stand || here == alt
    })
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
    RUNTIME.with(|runtime| runtime.borrow_mut().abort_runtime());
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
}
