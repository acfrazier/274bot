//! Rust-owned PeriodicBank due-evaluation, clocks and run sequencing.
//! JS marshals caller options/callbacks and executes the returned existing
//! bank/navigation operations. Do not clone foreign `Banking.bankNearest`.

use serde_json::{json, Value};
use std::cell::RefCell;
use std::time::{Duration, Instant};

pub const WALK_BOUND_MS: u64 = 60_000;
pub const BANK_WAIT_MS: u64 = 4_000;
pub const FAILURE_BACKOFF_MS: u64 = 180_000;
pub const ACCESS_RADIUS: i32 = 1;
pub const RETURN_RADIUS: i32 = 6;

thread_local! {
    static RUNTIME: RefCell<PeriodicBankRuntime> = RefCell::new(PeriodicBankRuntime::new());
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BankStrategy {
    Off,
    Items,
    Time,
    Either,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Idle,
    Access,
    Open,
    WaitReady,
    Deposit,
    AfterDeposit,
    Close,
    Return,
}

#[derive(Debug, Clone, Copy)]
struct Tile {
    x: i32,
    z: i32,
    level: i32,
}

#[derive(Debug, Clone, Copy)]
struct Booth {
    x: i32,
    z: i32,
    level: i32,
    id: i32,
}

pub struct PeriodicBankRuntime {
    last_success: Instant,
    suppress_until: Option<Instant>,
    paused: bool,
    held: bool,
    frozen_at: Option<Instant>,
    token: u64,
    phase: Phase,
    dest: Option<Tile>,
    return_to: Option<Tile>,
    has_after_deposit: bool,
    open_generation: u64,
}

impl PeriodicBankRuntime {
    pub fn new() -> Self {
        let now = Instant::now();
        Self {
            last_success: now,
            suppress_until: None,
            paused: false,
            held: false,
            frozen_at: None,
            token: 0,
            phase: Phase::Idle,
            dest: None,
            return_to: None,
            has_after_deposit: false,
            open_generation: 0,
        }
    }

    fn frozen(&self) -> bool {
        self.paused || self.held
    }

    fn now(&self) -> Instant {
        self.frozen_at.unwrap_or_else(Instant::now)
    }

    fn apply_freeze_flags(&mut self, paused: bool, held: bool) {
        let was = self.frozen();
        self.paused = paused;
        self.held = held;
        let now_frozen = self.frozen();
        if !was && now_frozen {
            self.frozen_at = Some(Instant::now());
        } else if was && !now_frozen {
            if let Some(at) = self.frozen_at.take() {
                let dt = Instant::now().saturating_duration_since(at);
                self.last_success += dt;
                if let Some(until) = self.suppress_until.as_mut() {
                    *until += dt;
                }
            }
        }
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
        self.dest = None;
        self.return_to = None;
        self.has_after_deposit = false;
    }

    pub fn validate(
        &self,
        strategy: BankStrategy,
        in_combat: bool,
        loot_count: i32,
        items_threshold: i32,
        minutes_threshold: f64,
    ) -> bool {
        if strategy == BankStrategy::Off || in_combat {
            return false;
        }
        let now = self.now();
        if self.suppress_until.is_some_and(|until| now < until) {
            return false;
        }
        should_bank_now(
            strategy,
            loot_count,
            items_threshold,
            minutes_since(now, self.last_success),
            minutes_threshold,
        )
    }

    fn begin(
        &mut self,
        dest: Option<Tile>,
        return_to: Option<Tile>,
        has_after_deposit: bool,
        npc_access: bool,
        bank_generation: u64,
    ) -> Value {
        self.token = self.token.wrapping_add(1);
        if npc_access {
            self.phase = Phase::Idle;
            return json!({"kind": "fail", "token": self.token, "reason": "missing access"});
        }
        self.dest = dest;
        self.return_to = return_to;
        self.has_after_deposit = has_after_deposit;
        self.open_generation = bank_generation;
        self.phase = Phase::Access;
        json!({"kind": "started", "token": self.token})
    }

    fn next(&mut self, token: u64, obs: &Observation) -> Value {
        if token != self.token || self.phase == Phase::Idle {
            return json!({"kind": "aborted", "token": self.token});
        }
        if self.frozen() {
            return json!({"kind": "wait", "token": self.token, "timeout_ms": BANK_WAIT_MS});
        }
        loop {
            match self.phase {
                Phase::Idle => return json!({"kind": "aborted", "token": self.token}),
                Phase::Access => {
                    if snapshot_ready(obs) {
                        self.phase = Phase::Deposit;
                        continue;
                    }
                    let Some(here) = obs.here else {
                        return self.fail_now();
                    };
                    if let Some(dest) = self.dest {
                        if chebyshev(here, dest) <= ACCESS_RADIUS {
                            self.phase = Phase::Open;
                            continue;
                        }
                        return json!({
                            "kind": "walk-near",
                            "token": self.token,
                            "x": dest.x,
                            "z": dest.z,
                            "level": dest.level,
                            "radius": ACCESS_RADIUS,
                            "timeout_ms": WALK_BOUND_MS,
                        });
                    }
                    if let Some(booth) = obs.nearest_booth {
                        if chebyshev(here, booth.tile()) <= ACCESS_RADIUS {
                            self.phase = Phase::Open;
                            continue;
                        }
                    }
                    if obs.nearest_booth.is_some() || obs.has_booth_stands {
                        return json!({
                            "kind": "walk-nearest-bank",
                            "token": self.token,
                            "timeout_ms": WALK_BOUND_MS,
                        });
                    }
                    return self.fail_now();
                }
                Phase::Open => {
                    if snapshot_ready(obs) {
                        self.phase = Phase::Deposit;
                        continue;
                    }
                    let Some(booth) = obs.nearest_booth else {
                        return self.fail_now();
                    };
                    if let Some(dest) = self.dest {
                        if chebyshev(dest, booth.tile()) > ACCESS_RADIUS {
                            return self.fail_now();
                        }
                    }
                    self.open_generation = obs.bank_generation;
                    self.phase = Phase::WaitReady;
                    let mut body = json!({
                        "kind": "open-booth",
                        "token": self.token,
                        "x": booth.x,
                        "z": booth.z,
                        "level": booth.level,
                        "id": booth.id,
                        "timeout_ms": BANK_WAIT_MS,
                    });
                    if let Some(name) = obs.booth_name.as_ref() {
                        body["name"] = json!(name);
                    }
                    if let Some(action) = obs.booth_action.as_ref() {
                        body["action"] = json!(action);
                    }
                    return body;
                }
                Phase::WaitReady => {
                    if snapshot_ready(obs) && obs.bank_generation > self.open_generation {
                        self.phase = Phase::Deposit;
                        continue;
                    }
                    return json!({"kind": "wait", "token": self.token, "timeout_ms": BANK_WAIT_MS});
                }
                Phase::Deposit => {
                    return json!({"kind": "deposit", "token": self.token});
                }
                Phase::AfterDeposit => {
                    return json!({"kind": "after_deposit", "token": self.token});
                }
                Phase::Close => {
                    if !obs.bank_open {
                        self.phase = Phase::Return;
                        continue;
                    }
                    return json!({"kind": "close", "token": self.token});
                }
                Phase::Return => {
                    let Some(ret) = self.return_to else {
                        return self.succeed_now();
                    };
                    let Some(here) = obs.here else {
                        return self.fail_now();
                    };
                    if chebyshev(here, ret) <= RETURN_RADIUS {
                        return self.succeed_now();
                    }
                    return json!({
                        "kind": "return",
                        "token": self.token,
                        "x": ret.x,
                        "z": ret.z,
                        "level": ret.level,
                        "radius": RETURN_RADIUS,
                        "timeout_ms": WALK_BOUND_MS,
                    });
                }
            }
        }
    }

    fn note_deposit(&mut self, token: u64) -> Value {
        if token != self.token || self.phase != Phase::Deposit {
            return json!({"kind": "aborted", "token": self.token});
        }
        self.phase = if self.has_after_deposit {
            Phase::AfterDeposit
        } else {
            Phase::Close
        };
        json!({"kind": "ok", "token": self.token})
    }

    fn ack_after_deposit(&mut self, token: u64) -> Value {
        if token != self.token || self.phase != Phase::AfterDeposit {
            return json!({"kind": "aborted", "token": self.token});
        }
        self.phase = Phase::Close;
        json!({"kind": "ok", "token": self.token})
    }

    fn succeed_now(&mut self) -> Value {
        self.last_success = self.now();
        self.suppress_until = None;
        self.phase = Phase::Idle;
        json!({"kind": "ok", "token": self.token})
    }

    fn fail_now(&mut self) -> Value {
        let now = self.now();
        self.last_success = now;
        self.suppress_until = Some(now + Duration::from_millis(FAILURE_BACKOFF_MS));
        self.phase = Phase::Idle;
        json!({"kind": "fail", "token": self.token})
    }

    fn finish_fail(&mut self, token: u64) -> Value {
        if token != self.token {
            return json!({"kind": "aborted", "token": self.token});
        }
        self.fail_now()
    }

    fn finish_ok(&mut self, token: u64) -> Value {
        if token != self.token {
            return json!({"kind": "aborted", "token": self.token});
        }
        self.succeed_now()
    }
}

struct Observation {
    here: Option<Tile>,
    bank_open: bool,
    bank_loaded: bool,
    bank_generation: u64,
    nearest_booth: Option<Booth>,
    booth_name: Option<String>,
    booth_action: Option<String>,
    has_booth_stands: bool,
}

impl Booth {
    fn tile(self) -> Tile {
        Tile {
            x: self.x,
            z: self.z,
            level: self.level,
        }
    }
}

fn snapshot_ready(obs: &Observation) -> bool {
    obs.bank_open && obs.bank_loaded
}

fn chebyshev(a: Tile, b: Tile) -> i32 {
    if a.level != b.level {
        return i32::MAX;
    }
    (a.x - b.x).abs().max((a.z - b.z).abs())
}

fn minutes_since(now: Instant, then: Instant) -> f64 {
    now.saturating_duration_since(then).as_secs_f64() / 60.0
}

/// Parse frozen labels, the shim token `loot`, and the source token `items`.
/// Those two tokens are not treated as identical strings; both map to Items.
pub fn parse_bank_strategy(raw: &str) -> BankStrategy {
    match raw.trim().to_ascii_lowercase().as_str() {
        "loot count" | "loot" | "items" => BankStrategy::Items,
        "time" => BankStrategy::Time,
        "either" => BankStrategy::Either,
        _ => BankStrategy::Off,
    }
}

pub fn should_bank_now(
    strategy: BankStrategy,
    loot_count: i32,
    items_threshold: i32,
    minutes_since_last_bank: f64,
    minutes_threshold: f64,
) -> bool {
    if loot_count <= 0 {
        return false;
    }
    let by_items = loot_count >= items_threshold;
    let by_time = minutes_since_last_bank >= minutes_threshold;
    match strategy {
        BankStrategy::Off => false,
        BankStrategy::Items => by_items,
        BankStrategy::Time => by_time,
        BankStrategy::Either => by_items || by_time,
    }
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

fn dispatch_with(runtime: &mut PeriodicBankRuntime, input: &Value) -> Value {
    match input.get("op").and_then(Value::as_str).unwrap_or("") {
        "validate" => {
            let strategy = parse_bank_strategy(input["strategy"].as_str().unwrap_or("off"));
            json!(runtime.validate(
                strategy,
                input["in_combat"].as_bool().unwrap_or(false),
                input["loot_count"].as_i64().unwrap_or(0) as i32,
                input["items_threshold"].as_i64().unwrap_or(15) as i32,
                input["minutes_threshold"].as_f64().unwrap_or(10.0),
            ))
        }
        "begin" => runtime.begin(
            read_tile(input.get("destination")),
            read_tile(input.get("return_to")),
            input["has_after_deposit"].as_bool().unwrap_or(false),
            input["npc_access"].as_bool().unwrap_or(false),
            input["bank_generation"].as_u64().unwrap_or(0),
        ),
        "next" => {
            let token = input["token"].as_u64().unwrap_or(0);
            runtime.next(token, &read_observation(input))
        }
        "note_deposit" => runtime.note_deposit(input["token"].as_u64().unwrap_or(0)),
        "ack_after_deposit" => runtime.ack_after_deposit(input["token"].as_u64().unwrap_or(0)),
        "fail" => runtime.finish_fail(input["token"].as_u64().unwrap_or(0)),
        "ok" => runtime.finish_ok(input["token"].as_u64().unwrap_or(0)),
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

fn read_observation(input: &Value) -> Observation {
    let booth = input.get("nearest_booth").and_then(|row| {
        if row.is_null() {
            return None;
        }
        Some(Booth {
            x: row.get("x")?.as_i64()? as i32,
            z: row.get("z")?.as_i64()? as i32,
            level: row.get("level").and_then(Value::as_i64).unwrap_or(0) as i32,
            id: row.get("id")?.as_i64()? as i32,
        })
    });
    Observation {
        here: read_tile(input.get("here")),
        bank_open: input["bank_open"].as_bool().unwrap_or(false),
        bank_loaded: input["bank_loaded"].as_bool().unwrap_or(false),
        bank_generation: input["bank_generation"].as_u64().unwrap_or(0),
        nearest_booth: booth,
        booth_name: input
            .get("booth_name")
            .and_then(Value::as_str)
            .map(str::to_string),
        booth_action: input
            .get("booth_action")
            .and_then(Value::as_str)
            .map(str::to_string),
        has_booth_stands: input["has_booth_stands"].as_bool().unwrap_or(false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_and_tokens_map_to_items_without_equating_loot_and_items_strings() {
        assert_eq!(WALK_BOUND_MS, 60_000);
        assert_eq!(BANK_WAIT_MS, 4_000);
        assert_eq!(FAILURE_BACKOFF_MS, 180_000);
        assert_ne!("loot", "items");
        assert_eq!(parse_bank_strategy("Loot count"), BankStrategy::Items);
        assert_eq!(parse_bank_strategy("loot"), BankStrategy::Items);
        assert_eq!(parse_bank_strategy("items"), BankStrategy::Items);
        assert_eq!(parse_bank_strategy("Off"), BankStrategy::Off);
        assert_eq!(parse_bank_strategy("Time"), BankStrategy::Time);
        assert_eq!(parse_bank_strategy("Either"), BankStrategy::Either);
        assert_eq!(parse_bank_strategy("unknown"), BankStrategy::Off);
    }

    #[test]
    fn frozen_should_bank_now_contract() {
        assert!(!should_bank_now(BankStrategy::Items, 0, 1, 100.0, 1.0));
        assert!(should_bank_now(BankStrategy::Items, 15, 15, 0.0, 10.0));
        assert!(!should_bank_now(BankStrategy::Items, 14, 15, 100.0, 1.0));
        assert!(should_bank_now(BankStrategy::Time, 1, 99, 10.0, 10.0));
        assert!(!should_bank_now(BankStrategy::Time, 1, 1, 9.9, 10.0));
        assert!(should_bank_now(BankStrategy::Either, 15, 15, 0.0, 10.0));
        assert!(should_bank_now(BankStrategy::Either, 1, 15, 10.0, 10.0));
        assert!(!should_bank_now(BankStrategy::Off, 27, 1, 100.0, 1.0));
    }

    #[test]
    fn combat_and_off_never_due() {
        let runtime = PeriodicBankRuntime::new();
        assert!(!runtime.validate(BankStrategy::Items, true, 20, 15, 10.0));
        assert!(!runtime.validate(BankStrategy::Off, false, 20, 15, 10.0));
        assert!(runtime.validate(BankStrategy::Items, false, 15, 15, 10.0));
    }

    #[test]
    fn failure_backoff_blocks_retry_and_pause_freezes_elapsed() {
        let mut runtime = PeriodicBankRuntime::new();
        runtime.finish_fail(runtime.token);
        assert!(
            !runtime.validate(BankStrategy::Items, false, 20, 15, 10.0),
            "180s backoff after a failed run"
        );
        runtime.last_success = runtime.now() - Duration::from_secs(11 * 60);
        runtime.suppress_until = None;
        assert!(runtime.validate(BankStrategy::Time, false, 1, 99, 10.0));
        runtime.set_paused(true);
        std::thread::sleep(Duration::from_millis(30));
        let paused_minutes = minutes_since(runtime.now(), runtime.last_success);
        runtime.set_paused(false);
        let resumed_minutes = minutes_since(runtime.now(), runtime.last_success);
        assert!(
            (paused_minutes - resumed_minutes).abs() < 0.002,
            "Pause must not count as banking minutes: paused={paused_minutes} resumed={resumed_minutes}"
        );
    }

    #[test]
    fn supplied_destination_does_not_fall_back_and_reset_drops_callback_phase() {
        let mut runtime = PeriodicBankRuntime::new();
        let dest = Tile {
            x: 2725,
            z: 3490,
            level: 0,
        };
        let started = runtime.begin(Some(dest), Some(dest), true, false, 3);
        let token = started["token"].as_u64().unwrap();
        let walk = runtime.next(
            token,
            &Observation {
                here: Some(Tile {
                    x: 2600,
                    z: 3400,
                    level: 0,
                }),
                bank_open: false,
                bank_loaded: false,
                bank_generation: 3,
                nearest_booth: Some(Booth {
                    x: 2655,
                    z: 3286,
                    level: 0,
                    id: 2213,
                }),
                booth_name: None,
                booth_action: None,
                has_booth_stands: true,
            },
        );
        assert_eq!(walk["kind"], "walk-near");
        assert_eq!(walk["x"], dest.x);
        assert_eq!(walk["z"], dest.z);
        let far = runtime.next(
            token,
            &Observation {
                here: Some(dest),
                bank_open: false,
                bank_loaded: false,
                bank_generation: 3,
                nearest_booth: Some(Booth {
                    x: 2655,
                    z: 3286,
                    level: 0,
                    id: 2213,
                }),
                booth_name: None,
                booth_action: None,
                has_booth_stands: true,
            },
        );
        assert_eq!(far["kind"], "fail", "a different loc is missing access");
        let started = runtime.begin(Some(dest), None, true, false, 4);
        let token = started["token"].as_u64().unwrap();
        let open = runtime.next(
            token,
            &Observation {
                here: Some(dest),
                bank_open: false,
                bank_loaded: false,
                bank_generation: 4,
                nearest_booth: Some(Booth {
                    x: dest.x,
                    z: dest.z,
                    level: 0,
                    id: 2213,
                }),
                booth_name: Some("Bank booth".into()),
                booth_action: Some("Use-quickly".into()),
                has_booth_stands: true,
            },
        );
        assert_eq!(open["kind"], "open-booth");
        assert_eq!(open["id"], 2213);
        runtime.phase = Phase::AfterDeposit;
        runtime.abort();
        assert_eq!(
            runtime.ack_after_deposit(token)["kind"],
            "aborted",
            "session replacement must drop an old afterDeposit ack"
        );
    }

    #[test]
    fn npc_access_is_explicit_missing_access() {
        let mut runtime = PeriodicBankRuntime::new();
        let out = runtime.begin(None, None, false, true, 0);
        assert_eq!(out["kind"], "fail");
        assert_eq!(out["reason"], "missing access");
    }
}
