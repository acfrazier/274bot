//! Rust-owned Baker stall target/restock policy and one-attempt sequencing.
//! `api::cake_stall` owns the posted pins and row types; this module owns
//! selection, restock and stall-food predicates. Snapshot deltas feed a
//! compact native projection; JavaScript supplies callbacks and dispatches
//! returned verbs.

use crate::isolate_fb::SnapshotReader;
use api::cake_stall::{BakerStall, StallLoc, BAKER_STALL, CAKE_ITEM_NAMES};
use api::snapshot::WorldTile;
use serde_json::{json, Value};
use std::cell::RefCell;
use std::time::{Duration, Instant};

pub const WALK_BOUND_MS: u64 = 60_000;
pub const STEAL_RESOLVE_MS: u64 = 2_400;

thread_local! {
    static RUNTIME: RefCell<CakeStallRuntime> = const { RefCell::new(CakeStallRuntime::new()) };
    static NATIVE_OBSERVATION: RefCell<NativeObservation> = const { RefCell::new(NativeObservation::new()) };
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
    const fn new() -> Self {
        Self {
            ingame: false,
            tick: 0,
            here: None,
            in_combat: false,
            inv_size: 0,
            inv_len: 0,
            carried: 0,
            stall: None,
        }
    }

    fn update(&mut self, snap: &SnapshotReader<'_>) {
        self.tick = i64::try_from(snap.tick()).unwrap_or(i64::MAX);
        if snap.has_ingame() {
            if !snap.ingame() {
                *self = Self::new();
                return;
            }
            self.ingame = true;
        }
        if snap.has_here() {
            self.here = snap.here().map(|tile| Tile {
                x: tile.x(),
                z: tile.z(),
                level: tile.level(),
            });
        }
        if snap.has_in_combat() {
            self.in_combat = snap.in_combat();
        }
        if snap.has_inv_size() {
            self.inv_size = snap.inv_size();
        }
        if snap.has_inv() {
            let inv = snap.inv();
            self.inv_len = inv.len();
            self.carried = sum_carried(
                inv.iter()
                    .filter_map(|row| row.name().map(|name| (name, row.count()))),
            );
        }
        if snap.has_locs() {
            self.stall = selected_stall(snap);
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

fn selected_stall(snap: &SnapshotReader<'_>) -> Option<SelectedLoc> {
    let locs = snap.locs();
    let action_refs: Vec<Vec<&str>> = locs.iter().map(|loc| loc.actions()).collect();
    let views: Vec<StallLoc<'_>> = locs
        .iter()
        .zip(action_refs.iter())
        .map(|(loc, actions)| StallLoc {
            id: loc.id(),
            name: loc.name(),
            x: loc.x(),
            z: loc.z(),
            level: loc.level(),
            distance: loc.distance(),
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
    NATIVE_OBSERVATION.with(|observation| {
        *observation.borrow_mut() = NativeObservation::new();
    });
}

pub fn on_snapshot(snap: &SnapshotReader<'_>) {
    NATIVE_OBSERVATION.with(|observation| observation.borrow_mut().update(snap));
}

pub fn dispatch(input: &Value) -> Value {
    NATIVE_OBSERVATION.with(|native| {
        let obs = native.borrow().with_callbacks(input);
        match input.get("op").and_then(Value::as_str).unwrap_or("") {
            "count" => json!(obs.carried),
            "needs_restock" => json!(needs_cake_restock(
                obs.carried,
                input
                    .get("target")
                    .and_then(Value::as_i64)
                    .map(|n| n as i32),
                pack_full(&obs),
            )),
            "begin" => RUNTIME.with(|runtime| {
                runtime.borrow_mut().begin(
                    input
                        .get("fill_to")
                        .and_then(Value::as_i64)
                        .map(|n| n as i32),
                    &obs,
                )
            }),
            "next" => RUNTIME.with(|runtime| {
                runtime.borrow_mut().next(
                    input.get("token").and_then(Value::as_u64).unwrap_or(0),
                    &obs,
                )
            }),
            "current_token" => RUNTIME.with(|runtime| json!(runtime.borrow().token)),
            _ => json!({"kind": "done", "result": "no-progress", "stole": false}),
        }
    })
}

/// Neighborhood around the selected stall tile used to reject the other stall.
pub const TARGET_RADIUS: i32 = 3;

fn names_match(actual: Option<&str>, want: &str) -> bool {
    actual
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .is_some_and(|name| name.eq_ignore_ascii_case(want.trim()))
}

fn has_op(actions: &[&str], want: &str) -> bool {
    let want = want.trim();
    actions.iter().any(|action| {
        let action = action.trim();
        !action.is_empty() && action != "hidden" && action.eq_ignore_ascii_case(want)
    })
}

fn chebyshev(a: WorldTile, x: i32, z: i32, level: i32) -> i32 {
    if a.level != level {
        return i32::MAX;
    }
    (a.x - x).abs().max((a.z - z).abs())
}

/// Pick the matching Baker stall: name, steal op, selected loc id, and
/// neighborhood of the posted stall tile. Closest *qualifying* row wins.
/// Missing facts yield no target.
pub fn select_baker_stall<'a>(
    facts: Option<&BakerStall>,
    locs: &'a [StallLoc<'a>],
) -> Option<&'a StallLoc<'a>> {
    let facts = facts?;
    locs.iter()
        .filter(|loc| {
            loc.id == facts.loc_id
                && names_match(loc.name, facts.name)
                && has_op(loc.actions, facts.op)
                && chebyshev(facts.stall, loc.x, loc.z, loc.level) <= TARGET_RADIUS
        })
        .min_by_key(|loc| loc.distance)
}

/// Restock is owed only when the pack can still take stall food.
pub fn needs_cake_restock(carried: i32, target: Option<i32>, pack_full: bool) -> bool {
    if pack_full {
        return false;
    }
    carried < target.unwrap_or(1)
}

/// Exact stall-food names; chocolate cake does not match chocolate slice.
pub fn counts_as_stall_food(name: &str) -> bool {
    CAKE_ITEM_NAMES
        .iter()
        .any(|want| name.trim().eq_ignore_ascii_case(want))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn loc<'a>(
        id: i32,
        name: Option<&'a str>,
        x: i32,
        z: i32,
        distance: i32,
        actions: &'a [&'a str],
    ) -> StallLoc<'a> {
        StallLoc {
            id,
            name,
            x,
            z,
            level: 0,
            distance,
            actions,
        }
    }

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

    #[test]
    fn closer_door_does_not_beat_farther_qualifying_stall() {
        let steal = ["Steal from"];
        let open = ["Open"];
        let locs = [
            loc(1530, Some("Door"), 2668, 3312, 0, &open),
            loc(2561, Some("Baker's stall"), 2667, 3310, 2, &steal),
        ];
        let hit = select_baker_stall(Some(&BAKER_STALL), &locs).expect("stall");
        assert_eq!(hit.id, 2561);
        assert_eq!((hit.x, hit.z), (2667, 3310));
    }

    #[test]
    fn other_same_id_stall_is_not_the_selected_target() {
        let steal = ["Steal from"];
        let locs = [
            loc(2561, Some("Baker's stall"), 2655, 3311, 1, &steal),
            loc(2561, Some("Baker's stall"), 2667, 3310, 11, &steal),
        ];
        let hit = select_baker_stall(Some(&BAKER_STALL), &locs).expect("market stall");
        assert_eq!((hit.x, hit.z, hit.distance), (2667, 3310, 11));
    }

    #[test]
    fn depleted_or_wrong_op_or_missing_facts_select_nothing() {
        let examine = ["Examine"];
        let steal = ["Steal from"];
        let depleted = [loc(2561, Some("Baker's stall"), 2667, 3310, 1, &examine)];
        assert!(select_baker_stall(Some(&BAKER_STALL), &depleted).is_none());
        let wrong_name = [loc(2561, Some("Bakery stall"), 2667, 3310, 1, &steal)];
        assert!(select_baker_stall(Some(&BAKER_STALL), &wrong_name).is_none());
        let wrong_id = [loc(99, Some("Baker's stall"), 2667, 3310, 1, &steal)];
        assert!(select_baker_stall(Some(&BAKER_STALL), &wrong_id).is_none());
        assert!(select_baker_stall(None, &depleted).is_none());
        assert!(select_baker_stall(Some(&BAKER_STALL), &[]).is_none());
    }

    #[test]
    fn restock_honors_capacity_and_requested_count() {
        assert!(needs_cake_restock(0, Some(1), false));
        assert!(!needs_cake_restock(0, Some(1), true));
        assert!(!needs_cake_restock(0, Some(0), false));
        assert!(!needs_cake_restock(4, Some(4), false));
        assert!(needs_cake_restock(0, None, false));
        assert!(counts_as_stall_food("chocolate slice"));
        assert!(counts_as_stall_food("Cake"));
        assert!(!counts_as_stall_food("Chocolate cake"));
        assert!(!counts_as_stall_food("Bread roll"));
    }
}
