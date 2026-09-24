//! Private prayer set/clear machine. Public results stay HelperResult;
//! this Step keeps the existing autocast `{kind,token,ok,reason}` shape.

use crate::observed::{self, Scene};
use api::game_data::SelectedGameData;
use api::prayer::{
    active, available, lookup, matches_on, max, on_is_truthy, points, OnArg, PrayerObservation,
    PRAYER_COUNT, TOGGLE_MS,
};
use serde_json::{json, Value};
use std::cell::RefCell;
use std::time::{Duration, Instant};

thread_local! {
    static RUNTIME: RefCell<PrayerRuntime> = const { RefCell::new(PrayerRuntime::new()) };
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Idle,
    WaitToggle,
    WaitClear,
}

/// Prayer points/max and the overlay varps, read from the isolate scene. A
/// logout forgets the session: only pages posted since login count. A varp
/// row the last varps page did not carry stays unobserved.
fn prayer_observation(scene: &Scene) -> PrayerObservation {
    let session = scene.since_login();
    let mut obs = PrayerObservation::empty();
    if let Some(row) = session.stat("prayer") {
        obs.points = row.effective;
        obs.max = row.base;
    }
    for row in session.varps().into_iter().flatten() {
        // Rows outside the 15 overlay varps are ignored by `set_varp`.
        obs.set_varp(row.index, row.value);
    }
    obs
}

struct PrayerRuntime {
    paused: bool,
    held: bool,
    frozen_at: Option<Instant>,
    token: u64,
    phase: Phase,
    want: OnArg,
    varp: i32,
    deadline: Option<Instant>,
    clear_index: usize,
    clicked: u32,
    timed_out: u32,
}

impl PrayerRuntime {
    const fn new() -> Self {
        Self {
            paused: false,
            held: false,
            frozen_at: None,
            token: 0,
            phase: Phase::Idle,
            want: OnArg::Undefined,
            varp: -1,
            deadline: None,
            clear_index: 0,
            clicked: 0,
            timed_out: 0,
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
        self.want = OnArg::Undefined;
        self.varp = -1;
        self.deadline = None;
        self.clear_index = 0;
        self.clicked = 0;
        self.timed_out = 0;
    }

    fn done(&mut self, ok: bool, reason: &str, value: Value) -> Value {
        self.phase = Phase::Idle;
        self.deadline = None;
        let mut out = json!({
            "kind": "done",
            "token": self.token,
            "ok": ok,
            "reason": reason,
        });
        if !value.is_null() {
            out["value"] = value;
        }
        out
    }

    fn command_click(&mut self, phase: Phase, component_id: i32) -> Value {
        self.phase = phase;
        self.deadline = Some(self.now() + Duration::from_millis(TOGGLE_MS));
        json!({
            "kind": "if-button",
            "token": self.token,
            "component_id": component_id,
        })
    }

    fn begin_set(
        &mut self,
        data: Option<&SelectedGameData>,
        name: &str,
        on: OnArg,
        obs: &PrayerObservation,
    ) -> Value {
        self.abort();
        let Some(data) = data else {
            return self.done(false, "unknown-prayer", Value::Null);
        };
        let Some(row) = lookup(data, name) else {
            return self.done(false, "unknown-prayer", Value::Null);
        };
        if matches_on(active(data, name, obs), on) {
            return self.done(true, "matched", json!(true));
        }
        if on_is_truthy(on) && !available(data, name, obs) {
            return self.done(false, "unavailable", Value::Null);
        }
        self.want = on;
        self.varp = row.varp;
        self.command_click(Phase::WaitToggle, row.button_com)
    }

    fn begin_clear(&mut self, data: Option<&SelectedGameData>, obs: &PrayerObservation) -> Value {
        self.abort();
        self.phase = Phase::WaitClear;
        self.clear_index = 0;
        self.clicked = 0;
        self.timed_out = 0;
        self.advance_clear(data, obs)
    }

    fn advance_clear(&mut self, data: Option<&SelectedGameData>, obs: &PrayerObservation) -> Value {
        let Some(data) = data else {
            return self.clear_done();
        };
        let rows = data.prayers();
        while self.clear_index < rows.len() && self.clear_index < PRAYER_COUNT {
            let row = &rows[self.clear_index];
            self.clear_index += 1;
            if obs.is_on(row.varp) {
                self.clicked = self.clicked.saturating_add(1);
                self.varp = row.varp;
                return self.command_click(Phase::WaitClear, row.button_com);
            }
        }
        self.clear_done()
    }

    fn clear_done(&mut self) -> Value {
        let value = json!({
            "clicked": self.clicked,
            "timed_out": self.timed_out,
        });
        self.done(true, "cleared", value)
    }

    fn timed_out(&self) -> bool {
        self.deadline.is_some_and(|deadline| self.now() >= deadline)
    }

    fn next(
        &mut self,
        token: u64,
        data: Option<&SelectedGameData>,
        obs: &PrayerObservation,
    ) -> Value {
        if token != self.token || self.phase == Phase::Idle {
            return json!({"kind": "aborted", "token": self.token});
        }
        if self.frozen() {
            return json!({"kind": "wait", "token": self.token});
        }
        match self.phase {
            Phase::Idle => json!({"kind": "aborted", "token": self.token}),
            Phase::WaitToggle
                if match self.want {
                    OnArg::Bool(true) => obs.is_on(self.varp),
                    OnArg::Bool(false) => obs.is_off(self.varp),
                    OnArg::Undefined | OnArg::Other { .. } => false,
                } =>
            {
                self.done(true, "toggled", json!(true))
            }
            Phase::WaitToggle if self.timed_out() => {
                self.done(false, "toggle-timeout", Value::Null)
            }
            Phase::WaitClear if obs.is_off(self.varp) => self.advance_clear(data, obs),
            Phase::WaitClear if self.timed_out() => {
                self.timed_out = self.timed_out.saturating_add(1);
                self.advance_clear(data, obs)
            }
            _ => json!({"kind": "wait", "token": self.token}),
        }
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

pub fn configure(_data: Option<&SelectedGameData>) {}

fn parse_on(input: &Value) -> OnArg {
    let on = input.get("on").unwrap_or(&Value::Null);
    match on.get("kind").and_then(Value::as_str) {
        Some("boolean") => OnArg::Bool(on.get("value").and_then(Value::as_bool).unwrap_or(false)),
        Some("other") => OnArg::Other {
            truthy: on.get("truthy").and_then(Value::as_bool).unwrap_or(false),
        },
        _ => OnArg::Undefined,
    }
}

fn helper_ok(value: Value) -> Value {
    json!({ "ok": true, "value": value })
}

fn query(data: Option<&SelectedGameData>, obs: &PrayerObservation, input: &Value) -> Value {
    match input.get("op").and_then(Value::as_str).unwrap_or("") {
        "points" => helper_ok(json!(points(obs))),
        "max" => helper_ok(json!(max(obs))),
        "full" => helper_ok(json!(api::prayer::full(obs))),
        "known" => {
            let name = input.get("name").and_then(Value::as_str).unwrap_or("");
            helper_ok(json!(
                data.is_some_and(|data| api::prayer::known(data, name))
            ))
        }
        "available" => {
            let name = input.get("name").and_then(Value::as_str).unwrap_or("");
            helper_ok(json!(data.is_some_and(|data| available(data, name, obs))))
        }
        "active" => {
            let name = input.get("name").and_then(Value::as_str).unwrap_or("");
            helper_ok(json!(data.is_some_and(|data| active(data, name, obs))))
        }
        _ => json!({ "ok": false, "error": "unknown-op" }),
    }
}

pub fn dispatch(data: Option<&SelectedGameData>, input: &Value) -> Value {
    let obs = observed::with(prayer_observation);
    match input.get("op").and_then(Value::as_str).unwrap_or("") {
        "points" | "max" | "full" | "known" | "available" | "active" => query(data, &obs, input),
        "begin-set" => RUNTIME.with(|runtime| {
            runtime.borrow_mut().begin_set(
                data,
                input.get("name").and_then(Value::as_str).unwrap_or(""),
                parse_on(input),
                &obs,
            )
        }),
        "begin-clear" => RUNTIME.with(|runtime| runtime.borrow_mut().begin_clear(data, &obs)),
        "next" => RUNTIME.with(|runtime| {
            runtime.borrow_mut().next(
                input.get("token").and_then(Value::as_u64).unwrap_or(0),
                data,
                &obs,
            )
        }),
        "current_token" => RUNTIME.with(|runtime| json!(runtime.borrow().token)),
        _ => json!({ "ok": false, "error": "unknown-op" }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use client::io::ClientRevision;
    use std::thread;
    use std::time::Duration;

    fn data(rev: ClientRevision) -> std::sync::Arc<SelectedGameData> {
        api::game_data::for_revision(rev).expect("selected data")
    }

    /// Stand in for a post carrying the prayer stat and one more overlay
    /// varp; earlier varp rows stay on the page.
    fn set_obs(points: i32, max: i32, varp: i32, value: i32) {
        let mut varps = observed::with(|scene| scene.latest().varps().cloned().unwrap_or_default());
        varps.retain(|row| row.index != varp);
        varps.push(observed::VarpRow { index: varp, value });
        observed::post(0, |post| {
            post.stats(vec![observed::StatRow {
                name: "prayer".into(),
                effective: points,
                base: max,
                ..observed::StatRow::default()
            }])
            .varps(varps);
        });
    }

    fn reset_tls() {
        on_reset();
        observed::on_reset();
        on_resume();
        on_hold(false);
    }

    #[test]
    fn matching_set_does_not_click_and_unknown_is_error() {
        reset_tls();
        let data = data(ClientRevision::R274);
        set_obs(43, 43, 97, 1);
        let matched = dispatch(
            Some(data.as_ref()),
            &json!({"op":"begin-set","name":"Protect from Melee","on":{"kind":"boolean","value":true}}),
        );
        assert_eq!(matched["kind"], "done");
        assert_eq!(matched["ok"], true);
        assert_eq!(matched["reason"], "matched");

        let unknown = dispatch(
            Some(data.as_ref()),
            &json!({"op":"begin-set","name":"Nope","on":{"kind":"boolean","value":true}}),
        );
        assert_eq!(unknown["kind"], "done");
        assert_eq!(unknown["ok"], false);
        assert_eq!(unknown["reason"], "unknown-prayer");
        reset_tls();
    }

    #[test]
    fn unavailable_on_does_not_click_off_ignores_available() {
        reset_tls();
        let data = data(ClientRevision::R289);
        set_obs(0, 1, 97, 0);
        let unavailable = dispatch(
            Some(data.as_ref()),
            &json!({"op":"begin-set","name":"Protect from Melee","on":{"kind":"boolean","value":true}}),
        );
        assert_eq!(unavailable["ok"], false);
        assert_eq!(unavailable["reason"], "unavailable");
        assert_ne!(unavailable["kind"], "if-button");

        set_obs(0, 1, 97, 1);
        let off = dispatch(
            Some(data.as_ref()),
            &json!({"op":"begin-set","name":"Protect from Melee","on":{"kind":"boolean","value":false}}),
        );
        assert_eq!(off["kind"], "if-button");
        assert_eq!(off["component_id"], 5623);
        reset_tls();
    }

    #[test]
    fn omitted_on_clicks_then_times_out() {
        reset_tls();
        let data = data(ClientRevision::R274);
        set_obs(43, 43, 97, 0);
        let begin = dispatch(
            Some(data.as_ref()),
            &json!({"op":"begin-set","name":"Protect from Melee","on":{"kind":"undefined"}}),
        );
        assert_eq!(begin["kind"], "if-button");
        assert_eq!(begin["component_id"], 5623);
        let token = begin["token"].as_u64().expect("token");
        thread::sleep(Duration::from_millis(TOGGLE_MS + 50));
        let done = dispatch(Some(data.as_ref()), &json!({"op":"next","token":token}));
        assert_eq!(done["kind"], "done");
        assert_eq!(done["ok"], false);
        assert_eq!(done["reason"], "toggle-timeout");
        reset_tls();
    }

    #[test]
    fn pause_hold_freeze_deadline_and_reset_aborts_token() {
        reset_tls();
        let data = data(ClientRevision::R289);
        set_obs(43, 43, 97, 0);
        let begin = dispatch(
            Some(data.as_ref()),
            &json!({"op":"begin-set","name":"Protect from Melee","on":{"kind":"boolean","value":true}}),
        );
        let token = begin["token"].as_u64().expect("token");
        on_pause();
        thread::sleep(Duration::from_millis(TOGGLE_MS + 50));
        let paused = dispatch(Some(data.as_ref()), &json!({"op":"next","token":token}));
        assert_eq!(paused["kind"], "wait");
        on_resume();
        on_hold(true);
        thread::sleep(Duration::from_millis(TOGGLE_MS + 50));
        let held = dispatch(Some(data.as_ref()), &json!({"op":"next","token":token}));
        assert_eq!(held["kind"], "wait");
        on_hold(false);
        on_reset();
        let stale = dispatch(Some(data.as_ref()), &json!({"op":"next","token":token}));
        assert_eq!(stale["kind"], "aborted");
        reset_tls();
    }

    #[test]
    fn clear_continues_after_timeout_with_counts() {
        reset_tls();
        let data = data(ClientRevision::R274);
        set_obs(43, 43, 96, 1);
        set_obs(43, 43, 97, 1);
        let begin = dispatch(Some(data.as_ref()), &json!({"op":"begin-clear"}));
        assert_eq!(begin["kind"], "if-button");
        assert_eq!(begin["component_id"], 5622);
        let token = begin["token"].as_u64().expect("token");
        thread::sleep(Duration::from_millis(TOGGLE_MS + 50));
        let second = dispatch(Some(data.as_ref()), &json!({"op":"next","token":token}));
        assert_eq!(second["kind"], "if-button");
        assert_eq!(second["component_id"], 5623);
        set_obs(43, 43, 97, 0);
        let done = dispatch(Some(data.as_ref()), &json!({"op":"next","token":token}));
        assert_eq!(done["kind"], "done");
        assert_eq!(done["ok"], true);
        assert_eq!(done["value"]["clicked"], 2);
        assert_eq!(done["value"]["timed_out"], 1);
        reset_tls();
    }
}
