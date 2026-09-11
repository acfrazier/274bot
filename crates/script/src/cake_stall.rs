//! Rust-owned one-attempt Baker stall sequencing.
//! The API crate owns target/restock predicates; JavaScript only supplies raw
//! observations, dispatches returned verbs and invokes the onSteal callback.

use api::cake_stall::{
    counts_as_stall_food, needs_cake_restock, select_baker_stall, StallLoc, BAKER_STALL,
};
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

#[derive(Debug)]
struct OwnedLoc {
    id: i32,
    name: Option<String>,
    tile: Tile,
    distance: i32,
    actions: Vec<String>,
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
    inv: Vec<(String, i32)>,
    facts_valid: bool,
    locs: Vec<OwnedLoc>,
    locked_out_until: Option<i64>,
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
        let Some(loc) = selected_stall(&obs.locs) else {
            return self.done("no-progress", false);
        };
        self.before = carried(&obs.inv);
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
                let gained = carried(&obs.inv) > self.before;
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
                if carried(&obs.inv) > self.before {
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

fn carried(inv: &[(String, i32)]) -> i32 {
    inv.iter()
        .filter(|(name, _)| counts_as_stall_food(name))
        .fold(0_i32, |sum, (_, count)| sum.saturating_add(*count))
}

fn pack_full(obs: &Observation) -> bool {
    obs.inv_size > 0 && obs.inv.len() >= obs.inv_size as usize
}

fn at_goal(obs: &Observation, fill_to: Option<i32>) -> bool {
    pack_full(obs) || fill_to.is_some_and(|target| carried(&obs.inv) >= target)
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

fn selected_stall(locs: &[OwnedLoc]) -> Option<SelectedLoc> {
    let action_refs: Vec<Vec<&str>> = locs
        .iter()
        .map(|loc| loc.actions.iter().map(String::as_str).collect())
        .collect();
    let views: Vec<StallLoc<'_>> = locs
        .iter()
        .zip(action_refs.iter())
        .map(|(loc, actions)| StallLoc {
            id: loc.id,
            name: loc.name.as_deref(),
            x: loc.tile.x,
            z: loc.tile.z,
            level: loc.tile.level,
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

fn read_tile(value: Option<&Value>) -> Option<Tile> {
    let value = value?;
    Some(Tile {
        x: value.get("x")?.as_i64()? as i32,
        z: value.get("z")?.as_i64()? as i32,
        level: value.get("level")?.as_i64()? as i32,
    })
}

fn selected_facts(value: Option<&Value>) -> bool {
    let Some(value) = value.filter(|value| !value.is_null()) else {
        return false;
    };
    value.get("loc_id").and_then(Value::as_i64) == Some(i64::from(BAKER_STALL.loc_id))
        && value.get("name").and_then(Value::as_str) == Some(BAKER_STALL.name)
        && value.get("op").and_then(Value::as_str) == Some(BAKER_STALL.op)
        && read_tile(value.get("stall"))
            == Some(Tile {
                x: BAKER_STALL.stall.x,
                z: BAKER_STALL.stall.z,
                level: BAKER_STALL.stall.level,
            })
        && read_tile(value.get("stand"))
            == Some(Tile {
                x: BAKER_STALL.stand.x,
                z: BAKER_STALL.stand.z,
                level: BAKER_STALL.stand.level,
            })
}

fn read_observation(value: Option<&Value>) -> Observation {
    let value = value.unwrap_or(&Value::Null);
    let inv = value
        .get("inv")
        .and_then(Value::as_array)
        .map(|rows| {
            rows.iter()
                .filter_map(|row| {
                    Some((
                        row.get("name")?.as_str()?.to_string(),
                        row.get("count").and_then(Value::as_i64).unwrap_or(0) as i32,
                    ))
                })
                .collect()
        })
        .unwrap_or_default();
    let locs = value
        .get("locs")
        .and_then(Value::as_array)
        .map(|rows| {
            rows.iter()
                .filter_map(|row| {
                    Some(OwnedLoc {
                        id: row.get("id")?.as_i64()? as i32,
                        name: row.get("name").and_then(Value::as_str).map(str::to_string),
                        tile: read_tile(Some(row))?,
                        distance: row
                            .get("distance")
                            .and_then(Value::as_i64)
                            .and_then(|distance| i32::try_from(distance).ok())
                            .unwrap_or(i32::MAX),
                        actions: row
                            .get("actions")
                            .and_then(Value::as_array)
                            .map(|actions| {
                                actions
                                    .iter()
                                    .filter_map(Value::as_str)
                                    .map(str::to_string)
                                    .collect()
                            })
                            .unwrap_or_default(),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    Observation {
        ingame: value
            .get("ingame")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        tick: value.get("tick").and_then(Value::as_i64).unwrap_or(0),
        here: read_tile(value.get("here")),
        in_combat: value
            .get("in_combat")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        abort: value.get("abort").and_then(Value::as_bool).unwrap_or(false),
        should_eat: value
            .get("should_eat")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        inv_size: value.get("inv_size").and_then(Value::as_i64).unwrap_or(0) as i32,
        inv,
        facts_valid: selected_facts(value.get("baker_stall")),
        locs,
        locked_out_until: value.get("locked_out_until").and_then(Value::as_i64),
    }
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
    match input.get("op").and_then(Value::as_str).unwrap_or("") {
        "count" => json!(carried(&read_observation(input.get("observation")).inv)),
        "needs_restock" => {
            let obs = read_observation(input.get("observation"));
            json!(needs_cake_restock(
                carried(&obs.inv),
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
                &read_observation(input.get("observation")),
            )
        }),
        "next" => RUNTIME.with(|runtime| {
            runtime.borrow_mut().next(
                input.get("token").and_then(Value::as_u64).unwrap_or(0),
                &read_observation(input.get("observation")),
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
            inv: vec![],
            facts_valid: true,
            locs: vec![OwnedLoc {
                id: BAKER_STALL.loc_id,
                name: Some(BAKER_STALL.name.into()),
                tile: Tile {
                    x: BAKER_STALL.stall.x,
                    z: BAKER_STALL.stall.z,
                    level: BAKER_STALL.stall.level,
                },
                distance: 2,
                actions: vec![BAKER_STALL.op.into()],
            }],
            locked_out_until: None,
        }
    }

    #[test]
    fn api_predicates_own_count_and_restock() {
        let mut obs = observation();
        obs.inv.push(("Chocolate cake".into(), 1));
        assert_eq!(carried(&obs.inv), 0);
        assert!(needs_cake_restock(
            carried(&obs.inv),
            Some(1),
            pack_full(&obs)
        ));
        obs.inv.push(("Chocolate slice".into(), 2));
        assert_eq!(carried(&obs.inv), 2);
        assert!(!needs_cake_restock(
            carried(&obs.inv),
            Some(1),
            pack_full(&obs)
        ));
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
