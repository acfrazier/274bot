//! Rust-owned one-attempt bank approach/open sequencing.
//! JavaScript supplies snapshot-shaped observations and dispatches returned
//! existing interaction verbs; Rust owns identity, phases and deadlines.

use serde_json::{json, Value};
use std::cell::RefCell;
use std::time::{Duration, Instant};

pub const WALK_BOUND_MS: u64 = 60_000;
pub const BANK_READY_MS: u64 = 5_000;
const ACCESS_RADIUS: i32 = 1;

thread_local! {
    static RUNTIME: RefCell<BankOpenRuntime> = const { RefCell::new(BankOpenRuntime::new()) };
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Tile {
    x: i32,
    z: i32,
    level: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Booth {
    tile: Tile,
    id: i32,
    distance: i32,
    name: Option<String>,
    action: Option<String>,
    actions: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Booth,
    Nearest,
    NearestWorld,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Idle,
    WaitStand,
    WaitSelected,
    WaitNearest,
    WaitReady,
    WaitFresh,
}

struct Observation {
    ingame: bool,
    here: Option<Tile>,
    bank_open: bool,
    bank_loaded: bool,
    bank_generation: u64,
    nearest_booth: Option<Booth>,
    locs: Vec<Booth>,
    has_booth_stands: bool,
}

struct BankOpenRuntime {
    paused: bool,
    held: bool,
    frozen_at: Option<Instant>,
    token: u64,
    phase: Phase,
    mode: Mode,
    stand: Option<Tile>,
    selected: Option<Booth>,
    wanted_name: Option<String>,
    wanted_action: Option<String>,
    open_generation: u64,
    deadline: Option<Instant>,
}

impl BankOpenRuntime {
    const fn new() -> Self {
        Self {
            paused: false,
            held: false,
            frozen_at: None,
            token: 0,
            phase: Phase::Idle,
            mode: Mode::Booth,
            stand: None,
            selected: None,
            wanted_name: None,
            wanted_action: None,
            open_generation: 0,
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

    fn abort(&mut self) {
        self.token = self.token.wrapping_add(1);
        self.phase = Phase::Idle;
        self.stand = None;
        self.selected = None;
        self.wanted_name = None;
        self.wanted_action = None;
        self.deadline = None;
    }

    fn done(&mut self, ok: bool, reason: &str) -> Value {
        self.phase = Phase::Idle;
        self.deadline = None;
        json!({"kind": "done", "token": self.token, "ok": ok, "reason": reason})
    }

    fn wait_ready(&mut self) -> Value {
        self.phase = Phase::WaitReady;
        self.deadline = Some(self.now() + Duration::from_millis(BANK_READY_MS));
        json!({"kind": "wait", "token": self.token})
    }

    fn walk_near(&mut self, phase: Phase, tile: Tile) -> Value {
        self.phase = phase;
        self.deadline = Some(self.now() + Duration::from_millis(WALK_BOUND_MS));
        json!({
            "kind": "walk-near",
            "token": self.token,
            "x": tile.x,
            "z": tile.z,
            "level": tile.level,
            "radius": ACCESS_RADIUS,
            "allow_teleports": false,
        })
    }

    fn open(&mut self, booth: Booth, obs: &Observation) -> Value {
        self.open_generation = obs.bank_generation;
        self.phase = Phase::WaitFresh;
        self.deadline = Some(self.now() + Duration::from_millis(BANK_READY_MS));
        let mut out = json!({
            "kind": "open-booth",
            "token": self.token,
            "x": booth.tile.x,
            "z": booth.tile.z,
            "level": booth.tile.level,
            "id": booth.id,
        });
        if let Some(name) = booth.name {
            out["name"] = json!(name);
        }
        if let Some(action) = booth.action {
            out["action"] = json!(action);
        }
        out
    }

    fn begin(
        &mut self,
        mode: Mode,
        stand: Option<Tile>,
        stand_invalid: bool,
        wanted_name: Option<String>,
        wanted_action: Option<String>,
        obs: &Observation,
    ) -> Value {
        self.abort();
        self.mode = mode;
        self.stand = stand;
        self.wanted_name = wanted_name;
        self.wanted_action = wanted_action;
        if !obs.ingame {
            return self.done(false, "missing-facts");
        }
        if obs.bank_open {
            if obs.bank_loaded {
                return self.done(true, "ready");
            }
            return self.wait_ready();
        }
        if stand_invalid {
            return self.done(false, "invalid-stand");
        }
        match mode {
            Mode::Booth => {
                if let Some(stand) = stand {
                    if !near(obs.here, stand) {
                        return self.walk_near(Phase::WaitStand, stand);
                    }
                }
                self.select_and_open(obs)
            }
            Mode::Nearest => {
                let Some(selected) = self.select_named(obs) else {
                    return self.done(false, "missing-booth");
                };
                if near(obs.here, selected.tile) {
                    return self.open(selected, obs);
                }
                self.selected = Some(selected.clone());
                self.walk_near(Phase::WaitSelected, selected.tile)
            }
            Mode::NearestWorld => {
                if let Some(booth) = obs.nearest_booth.clone() {
                    if near(obs.here, booth.tile) {
                        return self.open(unnamed(booth), obs);
                    }
                }
                if obs.here.is_none() || !obs.has_booth_stands {
                    return self.done(false, "missing-bank");
                }
                self.phase = Phase::WaitNearest;
                self.deadline = Some(self.now() + Duration::from_millis(WALK_BOUND_MS));
                json!({"kind": "walk-nearest-bank", "token": self.token})
            }
        }
    }

    fn select_named(&self, obs: &Observation) -> Option<Booth> {
        let name = self.wanted_name.as_deref()?;
        let action = self.wanted_action.as_deref()?;
        let mut selected = obs
            .locs
            .iter()
            .filter(|row| {
                row.id >= 0
                    && row
                        .name
                        .as_deref()
                        .is_some_and(|got| got.eq_ignore_ascii_case(name))
                    && row
                        .actions
                        .iter()
                        .any(|got| got.eq_ignore_ascii_case(action))
                    && obs.here.is_none_or(|here| here.level == row.tile.level)
            })
            .min_by_key(|row| row.distance)
            .cloned()?;
        selected.name = Some(name.to_string());
        selected.action = Some(action.to_string());
        Some(selected)
    }

    fn select_and_open(&mut self, obs: &Observation) -> Value {
        let selected = if self.wanted_name.is_some() || self.wanted_action.is_some() {
            self.select_named(obs)
        } else {
            obs.nearest_booth.clone().map(unnamed)
        };
        let Some(selected) = selected.filter(|row| row.id >= 0) else {
            return self.done(false, "missing-booth");
        };
        self.open(selected, obs)
    }

    fn selected_still_present(&self, obs: &Observation) -> bool {
        let Some(selected) = self.selected.as_ref() else {
            return false;
        };
        obs.locs.iter().any(|row| {
            row.id == selected.id
                && row.tile == selected.tile
                && same_optional_text(row.name.as_deref(), selected.name.as_deref())
                && selected.action.as_deref().is_some_and(|action| {
                    row.actions
                        .iter()
                        .any(|got| got.eq_ignore_ascii_case(action))
                })
                && obs.here.is_none_or(|here| here.level == row.tile.level)
        })
    }

    fn next(&mut self, token: u64, obs: &Observation) -> Value {
        if token != self.token || self.phase == Phase::Idle {
            return json!({"kind": "aborted", "token": self.token});
        }
        if self.frozen() {
            return json!({"kind": "wait", "token": self.token});
        }
        if !obs.ingame {
            return self.done(false, "aborted");
        }
        match self.phase {
            Phase::Idle => json!({"kind": "aborted", "token": self.token}),
            Phase::WaitReady if obs.bank_open && obs.bank_loaded => self.done(true, "ready"),
            Phase::WaitFresh
                if obs.bank_open
                    && obs.bank_loaded
                    && obs.bank_generation > self.open_generation =>
            {
                self.done(true, "ready")
            }
            Phase::WaitStand if obs.bank_open => {
                if obs.bank_loaded {
                    self.done(true, "ready")
                } else {
                    self.wait_ready()
                }
            }
            Phase::WaitStand if self.stand.is_some_and(|stand| near(obs.here, stand)) => {
                self.select_and_open(obs)
            }
            Phase::WaitSelected if obs.bank_open => {
                if obs.bank_loaded {
                    self.done(true, "ready")
                } else {
                    self.wait_ready()
                }
            }
            Phase::WaitSelected
                if self
                    .selected
                    .as_ref()
                    .is_some_and(|selected| near(obs.here, selected.tile)) =>
            {
                if !self.selected_still_present(obs) {
                    self.done(false, "identity-lost")
                } else {
                    let selected = self.selected.take().expect("selected booth");
                    self.open(selected, obs)
                }
            }
            Phase::WaitNearest if obs.bank_open => {
                if obs.bank_loaded {
                    self.done(true, "ready")
                } else {
                    self.wait_ready()
                }
            }
            Phase::WaitNearest => {
                if let Some(booth) = obs.nearest_booth.clone() {
                    if near(obs.here, booth.tile) {
                        return self.open(unnamed(booth), obs);
                    }
                }
                if self.expired() {
                    self.done(false, "walk-timeout")
                } else {
                    json!({"kind": "wait", "token": self.token})
                }
            }
            Phase::WaitReady | Phase::WaitFresh | Phase::WaitStand | Phase::WaitSelected => {
                if self.expired() {
                    self.done(false, "timeout")
                } else {
                    json!({"kind": "wait", "token": self.token})
                }
            }
        }
    }

    fn expired(&self) -> bool {
        self.deadline.is_some_and(|deadline| self.now() >= deadline)
    }
}

fn unnamed(mut booth: Booth) -> Booth {
    booth.name = None;
    booth.action = None;
    booth
}

fn near(here: Option<Tile>, target: Tile) -> bool {
    here.is_some_and(|here| distance(Some(here), target) <= ACCESS_RADIUS)
}

fn distance(here: Option<Tile>, target: Tile) -> i32 {
    let Some(here) = here else {
        return i32::MAX;
    };
    if here.level != target.level {
        return i32::MAX;
    }
    i32::try_from(here.x.abs_diff(target.x).max(here.z.abs_diff(target.z))).unwrap_or(i32::MAX)
}

fn same_optional_text(a: Option<&str>, b: Option<&str>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => a.eq_ignore_ascii_case(b),
        (None, None) => true,
        _ => false,
    }
}

fn parse_mode(input: &Value) -> Mode {
    match input
        .get("mode")
        .and_then(Value::as_str)
        .unwrap_or("open-booth")
    {
        "open-nearest" => Mode::Nearest,
        "open-nearest-world" => Mode::NearestWorld,
        _ => Mode::Booth,
    }
}

fn read_tile(value: Option<&Value>) -> Option<Tile> {
    let value = value?;
    if value.is_null() {
        return None;
    }
    Some(Tile {
        x: i32::try_from(value.get("x")?.as_i64()?).ok()?,
        z: i32::try_from(value.get("z")?.as_i64()?).ok()?,
        level: i32::try_from(value.get("level")?.as_i64()?).ok()?,
    })
}

fn read_booth(value: &Value, action_field: &str) -> Option<Booth> {
    Some(Booth {
        tile: read_tile(Some(value))?,
        id: value.get("id")?.as_i64()? as i32,
        distance: value
            .get("distance")
            .and_then(Value::as_i64)
            .and_then(|distance| i32::try_from(distance).ok())
            .unwrap_or(i32::MAX),
        name: value
            .get("name")
            .and_then(Value::as_str)
            .map(str::to_string),
        action: value
            .get(action_field)
            .and_then(Value::as_str)
            .map(str::to_string),
        actions: value
            .get("actions")
            .and_then(Value::as_array)
            .map(|actions| {
                actions
                    .iter()
                    .filter_map(Value::as_str)
                    .filter(|action| !action.is_empty() && *action != "hidden")
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default(),
    })
}

fn read_observation(value: Option<&Value>) -> Observation {
    let value = value.unwrap_or(&Value::Null);
    let nearest_booth = value
        .get("nearest_booth")
        .and_then(|row| read_booth(row, "op"));
    let locs = value
        .get("locs")
        .and_then(Value::as_array)
        .map(|rows| {
            rows.iter()
                .filter_map(|row| {
                    let booth = read_booth(row, "action").or_else(|| read_booth(row, "op"))?;
                    Some(booth)
                })
                .collect()
        })
        .unwrap_or_default();
    Observation {
        ingame: value
            .get("ingame")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        here: read_tile(value.get("here")),
        bank_open: value
            .get("bank_open")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        bank_loaded: value
            .get("bank_loaded")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        bank_generation: value
            .get("bank_generation")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        nearest_booth,
        locs,
        has_booth_stands: value
            .get("banks")
            .and_then(Value::as_array)
            .is_some_and(|rows| {
                rows.iter()
                    .any(|row| row.get("kind").and_then(Value::as_str) == Some("booth"))
            }),
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
    RUNTIME.with(|runtime| runtime.borrow_mut().abort());
}

pub fn dispatch(input: &Value) -> Value {
    RUNTIME.with(|runtime| {
        let mut runtime = runtime.borrow_mut();
        match input.get("op").and_then(Value::as_str).unwrap_or("") {
            "begin" => {
                let stand_value = input.get("stand");
                let stand = read_tile(stand_value);
                runtime.begin(
                    parse_mode(input),
                    stand,
                    stand_value.is_some_and(|value| !value.is_null()) && stand.is_none(),
                    input
                        .get("booth_name")
                        .and_then(Value::as_str)
                        .map(str::to_string),
                    input
                        .get("booth_action")
                        .and_then(Value::as_str)
                        .map(str::to_string),
                    &read_observation(input.get("observation")),
                )
            }
            "next" => runtime.next(
                input.get("token").and_then(Value::as_u64).unwrap_or(0),
                &read_observation(input.get("observation")),
            ),
            "current_token" => json!(runtime.token),
            _ => json!({"kind": "done", "ok": false, "reason": "unknown-op"}),
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn obs() -> Observation {
        Observation {
            ingame: true,
            here: Some(Tile {
                x: 3010,
                z: 3352,
                level: 0,
            }),
            bank_open: false,
            bank_loaded: false,
            bank_generation: 7,
            nearest_booth: None,
            locs: vec![Booth {
                tile: Tile {
                    x: 3011,
                    z: 3354,
                    level: 0,
                },
                id: 2213,
                distance: 2,
                name: Some("Bank booth".into()),
                action: Some("Use-quickly".into()),
                actions: vec!["Use-quickly".into()],
            }],
            has_booth_stands: true,
        }
    }

    #[test]
    fn walk_and_ready_bounds_are_native() {
        assert_eq!(WALK_BOUND_MS, 60_000);
        assert_eq!(BANK_READY_MS, 5_000);
    }

    #[test]
    fn out_of_range_stand_fails_closed() {
        let result = dispatch(&json!({
            "op": "begin",
            "mode": "open-booth",
            "stand": {"x": 2_147_483_648_i64, "z": 3355, "level": 0},
            "observation": {"ingame": true, "here": {"x": 3010, "z": 3352, "level": 0}},
        }));
        assert_eq!(result["kind"], "done");
        assert_eq!(result["ok"], false);
        assert_eq!(result["reason"], "invalid-stand");
    }

    #[test]
    fn timeout_and_reset_fail_closed_without_another_command() {
        let mut runtime = BankOpenRuntime::new();
        let begin = runtime.begin(
            Mode::Nearest,
            None,
            false,
            Some("Bank booth".into()),
            Some("Use-quickly".into()),
            &obs(),
        );
        let token = begin["token"].as_u64().unwrap();
        runtime.deadline = Some(Instant::now() - Duration::from_millis(1));
        assert_eq!(runtime.next(token, &obs())["kind"], "done");

        let begin = runtime.begin(
            Mode::Nearest,
            None,
            false,
            Some("Bank booth".into()),
            Some("Use-quickly".into()),
            &obs(),
        );
        let stale = begin["token"].as_u64().unwrap();
        runtime.abort();
        assert_eq!(runtime.next(stale, &obs())["kind"], "aborted");
    }
}
