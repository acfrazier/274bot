//! Rust-owned death latch and recovery sequencing.
//! JS keeps the frozen DeathRecovery ABI and script callbacks. Host latches a
//! *new* posted game-chat death line; seeded/old/duplicate chat does not.

use serde_json::{json, Value};
use std::cell::RefCell;

pub const WALK_BOUND_MS: u64 = 60_000;
pub const RESPAWN_WAIT_MS: u64 = 20_000;
pub const RESPAWN_TICKS: i32 = 3;
pub const DEFAULT_RADIUS: i32 = 6;

thread_local! {
    static RUNTIME: RefCell<DeathRecoveryRuntime> = RefCell::new(DeathRecoveryRuntime::new());
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Idle,
    WaitRespawn,
    WaitTicks,
    Walk,
    WalkBack,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Tile {
    x: i32,
    z: i32,
    level: i32,
}

pub struct DeathRecoveryRuntime {
    paused: bool,
    held: bool,
    token: u64,
    phase: Phase,
    latched: bool,
    baseline: bool,
    last_seq: i32,
    fire_on_death: bool,
    fire_on_recovered: bool,
    has_walk_back: bool,
    anchor: Option<Tile>,
    radius: i32,
}

impl DeathRecoveryRuntime {
    pub fn new() -> Self {
        Self {
            paused: false,
            held: false,
            token: 0,
            phase: Phase::Idle,
            latched: false,
            baseline: false,
            last_seq: 0,
            fire_on_death: false,
            fire_on_recovered: false,
            has_walk_back: false,
            anchor: None,
            radius: DEFAULT_RADIUS,
        }
    }

    fn frozen(&self) -> bool {
        self.paused || self.held
    }

    fn apply_freeze_flags(&mut self, paused: bool, held: bool) {
        self.paused = paused;
        self.held = held;
    }

    pub fn set_paused(&mut self, paused: bool) {
        self.apply_freeze_flags(paused, self.held);
    }

    pub fn set_held(&mut self, held: bool) {
        self.apply_freeze_flags(self.paused, held);
    }

    pub fn abort(&mut self) {
        self.token = self.token.wrapping_add(1);
        self.phase = Phase::Idle;
        self.latched = false;
        self.baseline = false;
        self.last_seq = 0;
        self.fire_on_death = false;
        self.fire_on_recovered = false;
        self.has_walk_back = false;
        self.anchor = None;
    }

    fn observe_chat(&mut self, lines: &[(i32, &str)]) {
        let max_seq = lines
            .iter()
            .map(|(seq, _)| *seq)
            .max()
            .unwrap_or(self.last_seq);
        if !self.baseline {
            self.last_seq = max_seq;
            self.baseline = true;
            return;
        }
        let mut newest = self.last_seq;
        let mut death = false;
        for (seq, text) in lines {
            if *seq <= self.last_seq {
                continue;
            }
            newest = newest.max(*seq);
            if is_death_line(text) {
                death = true;
            }
        }
        self.last_seq = newest;
        if death && !self.latched {
            self.latched = true;
            self.fire_on_death = true;
            self.fire_on_recovered = false;
        }
    }

    fn take_flags(&mut self) -> (bool, bool) {
        let death = self.fire_on_death;
        let recovered = self.fire_on_recovered;
        self.fire_on_death = false;
        self.fire_on_recovered = false;
        (death, recovered)
    }

    fn recover_now(&mut self) {
        self.latched = false;
        self.phase = Phase::Idle;
        self.fire_on_recovered = true;
        self.has_walk_back = false;
    }

    fn validate(&mut self, obs: &Observation) -> Value {
        self.observe_chat(&obs.chat_lines);
        if let Some(anchor) = obs.anchor {
            self.anchor = Some(anchor);
        }
        if let Some(radius) = obs.radius {
            self.radius = radius;
        }
        // Position at the configured anchor is not recovery until execute has
        // waited and the walk/walkBack actually finished. Seeded-at-anchor
        // therefore cannot clear the latch.
        let due = self.latched && self.phase == Phase::Idle;
        let (on_death, on_recovered) = self.take_flags();
        json!({
            "due": due,
            "latched": self.latched,
            "fire_on_death": on_death,
            "fire_on_recovered": on_recovered,
            "token": self.token,
        })
    }

    fn begin(&mut self, has_walk_back: bool, anchor: Option<Tile>, radius: Option<i32>) -> Value {
        self.token = self.token.wrapping_add(1);
        if let Some(anchor) = anchor {
            self.anchor = Some(anchor);
        }
        if let Some(radius) = radius {
            self.radius = radius;
        }
        self.has_walk_back = has_walk_back;
        if !self.latched {
            self.phase = Phase::Idle;
            return json!({
                "kind": "fail",
                "token": self.token,
                "reason": "not latched",
            });
        }
        if self.anchor.is_none() && !has_walk_back {
            self.phase = Phase::Idle;
            return json!({
                "kind": "fail",
                "token": self.token,
                "reason": "missing anchor",
            });
        }
        self.phase = Phase::WaitRespawn;
        json!({"kind": "started", "token": self.token})
    }

    fn next(&mut self, token: u64, obs: &Observation) -> Value {
        self.observe_chat(&obs.chat_lines);
        if token != self.token || self.phase == Phase::Idle {
            return json!({"kind": "aborted", "token": self.token});
        }
        if self.frozen() {
            return json!({
                "kind": "wait",
                "token": self.token,
                "timeout_ms": RESPAWN_WAIT_MS,
            });
        }
        match self.phase {
            Phase::Idle => json!({"kind": "aborted", "token": self.token}),
            Phase::WaitRespawn => json!({
                "kind": "wait-respawn",
                "token": self.token,
                "timeout_ms": RESPAWN_WAIT_MS,
            }),
            Phase::WaitTicks => json!({
                "kind": "wait-ticks",
                "token": self.token,
                "ticks": RESPAWN_TICKS,
            }),
            Phase::Walk => {
                let Some(anchor) = self.anchor else {
                    return self.fail_now();
                };
                if obs.here.is_some_and(|here| near(here, anchor, self.radius)) {
                    return self.succeed_now();
                }
                json!({
                    "kind": "walk-near",
                    "token": self.token,
                    "x": anchor.x,
                    "z": anchor.z,
                    "level": anchor.level,
                    "radius": self.radius,
                    "timeout_ms": WALK_BOUND_MS,
                })
            }
            Phase::WalkBack => json!({
                "kind": "walk_back",
                "token": self.token,
            }),
        }
    }

    fn ack_respawn(&mut self, token: u64) -> Value {
        if token != self.token || self.phase != Phase::WaitRespawn {
            return json!({"kind": "aborted", "token": self.token});
        }
        self.phase = Phase::WaitTicks;
        json!({"kind": "ok", "token": self.token})
    }

    fn ack_ticks(&mut self, token: u64) -> Value {
        if token != self.token || self.phase != Phase::WaitTicks {
            return json!({"kind": "aborted", "token": self.token});
        }
        self.phase = if self.has_walk_back {
            Phase::WalkBack
        } else {
            Phase::Walk
        };
        json!({"kind": "ok", "token": self.token})
    }

    fn ack_walk_back(&mut self, token: u64) -> Value {
        if token != self.token || self.phase != Phase::WalkBack {
            return json!({"kind": "aborted", "token": self.token});
        }
        // Script-owned walkBack completion is the recovery outcome.
        self.succeed_now()
    }

    fn succeed_now(&mut self) -> Value {
        self.recover_now();
        let (on_death, on_recovered) = self.take_flags();
        json!({
            "kind": "ok",
            "token": self.token,
            "fire_on_death": on_death,
            "fire_on_recovered": on_recovered,
        })
    }

    fn fail_now(&mut self) -> Value {
        self.phase = Phase::Idle;
        json!({"kind": "fail", "token": self.token})
    }

    fn finish_fail(&mut self, token: u64) -> Value {
        if token != self.token {
            return json!({"kind": "aborted", "token": self.token});
        }
        self.fail_now()
    }
}

struct Observation<'a> {
    here: Option<Tile>,
    chat_lines: Vec<(i32, &'a str)>,
    anchor: Option<Tile>,
    radius: Option<i32>,
}

fn is_death_line(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    let Some(at) = lower.find("oh dear") else {
        return false;
    };
    lower[at + "oh dear".len()..].contains("you are dead")
}

fn near(a: Tile, b: Tile, radius: i32) -> bool {
    a.level == b.level && (a.x - b.x).abs().max((a.z - b.z).abs()) <= radius
}

pub fn on_pause() {
    RUNTIME.with(|runtime| runtime.borrow_mut().set_paused(true));
}

pub fn on_resume() {
    RUNTIME.with(|runtime| runtime.borrow_mut().set_paused(false));
}

pub fn on_hold(held: bool) {
    RUNTIME.with(|runtime| runtime.borrow_mut().set_held(held));
}

pub fn on_reset() {
    RUNTIME.with(|runtime| runtime.borrow_mut().abort());
}

pub fn dispatch(input: &Value) -> Value {
    RUNTIME.with(|runtime| dispatch_with(&mut runtime.borrow_mut(), input))
}

fn dispatch_with(runtime: &mut DeathRecoveryRuntime, input: &Value) -> Value {
    match input.get("op").and_then(Value::as_str).unwrap_or("") {
        "validate" => runtime.validate(&read_observation(input)),
        "begin" => runtime.begin(
            input["has_walk_back"].as_bool().unwrap_or(false),
            read_tile(input.get("anchor")),
            input
                .get("radius")
                .and_then(Value::as_i64)
                .map(|n| n as i32),
        ),
        "next" => {
            let token = input["token"].as_u64().unwrap_or(0);
            runtime.next(token, &read_observation(input))
        }
        "ack_respawn" => runtime.ack_respawn(input["token"].as_u64().unwrap_or(0)),
        "ack_ticks" => runtime.ack_ticks(input["token"].as_u64().unwrap_or(0)),
        "ack_walk_back" => runtime.ack_walk_back(input["token"].as_u64().unwrap_or(0)),
        "fail" => runtime.finish_fail(input["token"].as_u64().unwrap_or(0)),
        "current_token" => json!(runtime.token),
        _ => json!({"kind": "fail", "reason": "unknown op"}),
    }
}

fn read_tile(value: Option<&Value>) -> Option<Tile> {
    let value = value?;
    if value.is_null() {
        return None;
    }
    let tile = value.get("tile").unwrap_or(value);
    Some(Tile {
        x: tile.get("x")?.as_i64()? as i32,
        z: tile.get("z")?.as_i64()? as i32,
        level: tile.get("level").and_then(Value::as_i64).unwrap_or(0) as i32,
    })
}

fn read_observation(input: &Value) -> Observation<'_> {
    let chat_lines = input
        .get("chat_lines")
        .and_then(Value::as_array)
        .map(|rows| {
            rows.iter()
                .filter_map(|row| {
                    Some((row.get("seq")?.as_i64()? as i32, row.get("text")?.as_str()?))
                })
                .collect()
        })
        .unwrap_or_default();
    Observation {
        here: read_tile(input.get("here")),
        chat_lines,
        anchor: read_tile(input.get("anchor")),
        radius: input
            .get("radius")
            .and_then(Value::as_i64)
            .map(|n| n as i32),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(seq: i32, text: &'static str) -> (i32, &'static str) {
        (seq, text)
    }

    fn field() -> Tile {
        Tile {
            x: 3235,
            z: 3295,
            level: 0,
        }
    }

    fn lumbridge() -> Tile {
        Tile {
            x: 3222,
            z: 3218,
            level: 0,
        }
    }

    fn anchor() -> Tile {
        Tile {
            x: 3235,
            z: 3295,
            level: 0,
        }
    }

    fn obs<'a>(here: Tile, lines: &'a [(i32, &'a str)]) -> Observation<'a> {
        Observation {
            here: Some(here),
            chat_lines: lines.to_vec(),
            anchor: Some(anchor()),
            radius: Some(3),
        }
    }

    #[test]
    fn bounds_match_frozen_contract() {
        assert_eq!(WALK_BOUND_MS, 60_000);
        assert_eq!(RESPAWN_WAIT_MS, 20_000);
        assert_eq!(RESPAWN_TICKS, 3);
        assert_eq!(DEFAULT_RADIUS, 6);
        assert!(is_death_line("Oh dear, you are dead!"));
        assert!(is_death_line("OH DEAR YOU ARE DEAD"));
        assert!(!is_death_line("Welcome to RuneScape"));
        assert!(!is_death_line("you are dead"));
    }

    #[test]
    fn seeded_duplicate_and_old_chat_do_not_latch() {
        let mut runtime = DeathRecoveryRuntime::new();
        let welcome = [line(1, "Welcome to RuneScape")];
        let death = [
            line(4, "Oh dear, you are dead!"),
            line(3, "Welcome to RuneScape"),
        ];
        runtime.observe_chat(&death);
        assert!(!runtime.latched, "seeded ring must not latch");
        runtime.observe_chat(&death);
        assert!(!runtime.latched, "duplicate snapshot must not latch");
        runtime.observe_chat(&welcome);
        assert!(!runtime.latched, "old chat must not latch");
        let fresh = [
            line(5, "Oh dear, you are dead!"),
            line(4, "Welcome to RuneScape"),
        ];
        runtime.observe_chat(&fresh);
        assert!(runtime.latched);
        assert!(runtime.fire_on_death);
        runtime.fire_on_death = false;
        runtime.observe_chat(&fresh);
        assert!(runtime.latched);
        assert!(
            !runtime.fire_on_death,
            "duplicate death seq must not re-trigger"
        );
    }

    #[test]
    fn validate_false_until_new_death_and_recovers_once_at_anchor() {
        let mut runtime = DeathRecoveryRuntime::new();
        let first = runtime.validate(&obs(field(), &[line(1, "Welcome to RuneScape")]));
        assert_eq!(first["due"], false);
        assert_eq!(first["fire_on_death"], false);
        let death = runtime.validate(&obs(
            lumbridge(),
            &[
                line(2, "Oh dear, you are dead!"),
                line(1, "Welcome to RuneScape"),
            ],
        ));
        assert_eq!(death["due"], true);
        assert_eq!(death["fire_on_death"], true);
        let again = runtime.validate(&obs(
            lumbridge(),
            &[
                line(2, "Oh dear, you are dead!"),
                line(1, "Welcome to RuneScape"),
            ],
        ));
        assert_eq!(again["due"], true);
        assert_eq!(again["fire_on_death"], false);
        let started = runtime.begin(false, Some(anchor()), Some(3));
        let token = started["token"].as_u64().unwrap();
        assert_eq!(started["kind"], "started");
        let wait = runtime.next(token, &obs(lumbridge(), &[]));
        assert_eq!(wait["kind"], "wait-respawn");
        assert_eq!(wait["timeout_ms"], RESPAWN_WAIT_MS);
        assert_eq!(runtime.ack_respawn(token)["kind"], "ok");
        let ticks = runtime.next(token, &obs(lumbridge(), &[]));
        assert_eq!(ticks["kind"], "wait-ticks");
        assert_eq!(ticks["ticks"], RESPAWN_TICKS);
        assert_eq!(runtime.ack_ticks(token)["kind"], "ok");
        let walk = runtime.next(token, &obs(lumbridge(), &[]));
        assert_eq!(walk["kind"], "walk-near");
        assert_eq!(walk["x"], 3235);
        assert_eq!(walk["radius"], 3);
        assert_eq!(walk["timeout_ms"], WALK_BOUND_MS);
        let still = runtime.next(token, &obs(lumbridge(), &[]));
        assert_eq!(still["kind"], "walk-near", "queued walk is not recovery");
        let done = runtime.next(token, &obs(anchor(), &[]));
        assert_eq!(done["kind"], "ok");
        assert_eq!(done["fire_on_recovered"], true);
        let idle = runtime.validate(&obs(anchor(), &[line(2, "Oh dear, you are dead!")]));
        assert_eq!(idle["due"], false);
        assert_eq!(idle["fire_on_recovered"], false);
    }

    #[test]
    fn walk_back_is_invoked_instead_of_default_walk() {
        let mut runtime = DeathRecoveryRuntime::new();
        runtime.observe_chat(&[line(1, "hello")]);
        runtime.observe_chat(&[line(2, "Oh dear, you are dead!")]);
        let started = runtime.begin(true, Some(anchor()), Some(6));
        let token = started["token"].as_u64().unwrap();
        runtime.ack_respawn(token);
        runtime.ack_ticks(token);
        let step = runtime.next(token, &obs(lumbridge(), &[]));
        assert_eq!(step["kind"], "walk_back");
        let done = runtime.ack_walk_back(token);
        assert_eq!(done["kind"], "ok");
        assert_eq!(done["fire_on_recovered"], true);
        assert!(!runtime.latched);
    }

    #[test]
    fn pause_freezes_sends_and_reset_drops_stale_token() {
        let mut runtime = DeathRecoveryRuntime::new();
        runtime.observe_chat(&[line(1, "hello")]);
        runtime.observe_chat(&[line(2, "Oh dear, you are dead!")]);
        let started = runtime.begin(false, Some(anchor()), Some(3));
        let token = started["token"].as_u64().unwrap();
        runtime.ack_respawn(token);
        runtime.ack_ticks(token);
        runtime.set_paused(true);
        let frozen = runtime.next(token, &obs(lumbridge(), &[]));
        assert_eq!(frozen["kind"], "wait");
        runtime.set_paused(false);
        let walk = runtime.next(token, &obs(lumbridge(), &[]));
        assert_eq!(walk["kind"], "walk-near");
        runtime.abort();
        assert_eq!(runtime.ack_ticks(token)["kind"], "aborted");
        assert_eq!(
            runtime.next(token, &obs(anchor(), &[]))["kind"],
            "aborted",
            "stale result after session replacement"
        );
        assert!(!runtime.latched);
    }

    #[test]
    fn respawn_timeout_still_advances_to_ticks() {
        let mut runtime = DeathRecoveryRuntime::new();
        runtime.observe_chat(&[line(1, "hello")]);
        runtime.observe_chat(&[line(2, "Oh dear, you are dead!")]);
        let started = runtime.begin(false, Some(anchor()), Some(3));
        let token = started["token"].as_u64().unwrap();
        let wait = runtime.next(
            token,
            &Observation {
                here: None,
                chat_lines: vec![],
                anchor: Some(anchor()),
                radius: Some(3),
            },
        );
        assert_eq!(wait["kind"], "wait-respawn");
        assert_eq!(runtime.ack_respawn(token)["kind"], "ok");
        assert_eq!(
            runtime.next(token, &obs(lumbridge(), &[]))["kind"],
            "wait-ticks"
        );
    }
}
