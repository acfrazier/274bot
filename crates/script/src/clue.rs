//! Isolate-owned clue-session machine skeleton: `api.clue.begin` / `next`.
//!
//! One token per isolate over the landed held-step identify. The JS wrapper
//! owns the page reads — the posted `snapshot.inv` `(id, count)` page, the
//! posted `hold || ours` signal, the wrapper's `lifecycleGeneration` — and
//! hands them in; this machine owns the token, the generation captured at
//! begin, the frozen clock, the identify call, the callback kinds and the
//! idle end state.
//!
//! `api::clue_logic::identify_step` over the current posted page and the
//! selected `trails()` family is the only membership decision, in its own
//! order: `missing-selected-data`, then `family-unavailable:trails`, then
//! `none-held`, then the landed row. A family absence is not `none-held`. A
//! `none-held` begin is refused with no live token — the caller begins again
//! after the pickup — and a live session that loses its held membership
//! errors `none-held` and aborts.
//!
//! This is the envelope, not the dispatcher: no search, dig, talk, guardian,
//! puzzle, deposit, retry or return-grind. A held step of any type, the
//! packed 3554 `access: "constrained"` clue included, is identified and then
//! idled: no action and no walk. Yield keeps the token live, so it is not
//! trail completion, and this machine never returns `status: "done"`, never
//! restores gear, and never emits the exact `'clue solved'` string.
//!
//! One token per isolate. A second begin, reset and stop abort the live token
//! and emit no verb for it. Pause and hold freeze this machine's own clock,
//! so a frozen call emits no callback and does not advance the session.
//! `on_snapshot` is fan-out only: begin and next read the page the wrapper
//! hands in at call time, so nothing is cached here.

use crate::isolate_fb::SnapshotReader;
use crate::task_clock::InstantTaskClock;
use api::clue_logic::identify_step;
use api::game_data::{SelectedGameData, TrailMembershipRow};
use serde_json::{json, Value};
use std::cell::RefCell;

/// No selected pin. The same public token the landed V8 held-step wrapper
/// publishes: the machine refuses with it rather than calling a page empty.
const MISSING_SELECTED_DATA: &str = "missing-selected-data";

/// The token is not this machine's live one.
const STALE: &str = "stale";

/// The live session was aborted out from under the token by the generation
/// bump (reset / stop). The token is dead and nothing is emitted for it.
const ABORTED: &str = "aborted";

thread_local! {
    static RUNTIME: RefCell<ClueRuntime> = const { RefCell::new(ClueRuntime::new()) };
}

/// The live session's phase. `Idle` is the only phase without a token, so a
/// refused begin leaves it in place.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    /// No token.
    Idle,
    /// A held step was identified and the fresh `enabled` read is unanswered.
    Gate,
    /// Enabled: the progress status line is not posted yet.
    Reporting,
    /// Progress posted: idle while the same step stays held. No action, no
    /// walk, and no second callback for this step.
    Steady,
}

struct ClueRuntime {
    clock: InstantTaskClock,
    token: u64,
    phase: Phase,
    /// The wrapper's `lifecycleGeneration` captured at begin. Nothing else
    /// about the live session is captured: `enabled`, the page and the
    /// `hold || ours` signal are re-read at call time.
    generation: u64,
    /// The membership row this phase belongs to: a different held row
    /// re-arms the gate, so the enabled read is never reused across steps.
    step_id: i32,
}

impl ClueRuntime {
    const fn new() -> Self {
        Self {
            clock: InstantTaskClock::new(),
            token: 0,
            phase: Phase::Idle,
            generation: 0,
            step_id: 0,
        }
    }

    /// Abort keeps the pause/hold freeze (`InstantTaskClock` contract: abort
    /// clears the deadline only) and emits nothing.
    fn abort(&mut self) {
        self.token = self.token.wrapping_add(1);
        self.phase = Phase::Idle;
        self.generation = 0;
        self.step_id = 0;
        self.clock.deadline = None;
    }

    fn aborted(&mut self, reason: &str) -> Value {
        self.abort();
        json!({ "kind": "aborted", "token": self.token, "reason": reason })
    }

    fn emit(&self, kind: &str) -> Value {
        json!({ "kind": kind, "token": self.token })
    }

    fn begin(&mut self, selected: Option<&SelectedGameData>, input: &Value) -> Value {
        // A second begin aborts the live token first and emits nothing for
        // it: begin has no continue kind, and a refused begin is still a
        // refusal, not a resumed session.
        self.abort();
        let row = match identify(selected, input) {
            Ok(row) => row,
            Err(reason) => return self.aborted(reason),
        };
        self.token = self.token.wrapping_add(1);
        self.phase = Phase::Gate;
        // Strictly captured: `next` requires the same generation to be
        // posted again, so a wrapper that stops sending it fails closed.
        self.generation = input.get("generation").and_then(Value::as_u64).unwrap_or(0);
        self.step_id = row.id;
        json!({ "kind": "token", "token": self.token })
    }

    fn next(&mut self, selected: Option<&SelectedGameData>, input: &Value) -> Value {
        let Some(token) = input.get("token").and_then(Value::as_u64) else {
            return self.aborted(STALE);
        };
        if token != self.token || self.phase == Phase::Idle {
            return self.aborted(STALE);
        }
        if input.get("generation").and_then(Value::as_u64) != Some(self.generation) {
            // Reset, stop and a second begin abort silently; the first thing
            // the dead token hears about it is this error, never a kind.
            return self.aborted(ABORTED);
        }
        if self.clock.frozen() {
            // Frozen: no callback, no verb and no burn. `resume` is not
            // consumed, so the gate is still unanswered after the thaw.
            return self.emit("wait");
        }
        if input.get("hold").and_then(Value::as_bool).unwrap_or(false) {
            // The posted `hold || ours` cooperative interrupt. The token
            // lives and the step is not trail completion.
            return self.emit("yield");
        }
        let row = match identify(selected, input) {
            Ok(row) => row,
            Err(reason) => return self.aborted(reason),
        };
        if self.step_id != row.id {
            // A different step is held: the previous progress and its
            // `enabled` answer belong to the old row, so this call re-asks
            // and a `resume` it carried is not the new step's answer.
            self.step_id = row.id;
            self.phase = Phase::Gate;
            return self.emit("callback.enabled");
        }
        match self.phase {
            Phase::Idle => self.aborted(STALE),
            Phase::Gate => match input.get("resume").and_then(Value::as_bool) {
                // The callback return. False means do not execute: the
                // session idles with its token live, and the following gate
                // re-reads instead of replaying this answer.
                Some(false) => self.emit("wait"),
                Some(true) => {
                    self.phase = Phase::Reporting;
                    json!({
                        "kind": "callback.log",
                        "token": self.token,
                        "message": progress(row),
                    })
                }
                None => self.emit("callback.enabled"),
            },
            Phase::Reporting => {
                self.phase = Phase::Steady;
                json!({
                    "kind": "callback.setStatus",
                    "token": self.token,
                    "message": status(row),
                })
            }
            Phase::Steady => self.emit("wait"),
        }
    }
}

/// The landed identify over the wrapper's page and the selected family. The
/// family token and `none-held` are the helper's own; only the missing pin is
/// named here.
fn identify<'a>(
    selected: Option<&'a SelectedGameData>,
    input: &Value,
) -> Result<&'a TrailMembershipRow, &'static str> {
    let Some(data) = selected else {
        return Err(MISSING_SELECTED_DATA);
    };
    let held = posted_page(input);
    identify_step(&held, data.trails())
}

/// The posted `(id, count)` page in posted order, as the wrapper marshals it.
/// A missing, non-array, or malformed entry is skipped the same way a row
/// that is not an `i32` pair cannot be held — never a second page and never a
/// snapshot error. Only a positive count holds, and only a membership row
/// wins; that stays in the landed helper.
fn posted_page(input: &Value) -> Vec<(i32, i32)> {
    let Some(rows) = input.get("held").and_then(Value::as_array) else {
        return Vec::new();
    };
    let mut held = Vec::with_capacity(rows.len());
    for row in rows {
        let Some(pair) = row.as_array() else {
            continue;
        };
        let (Some(id), Some(count)) = (pair.first().and_then(i32_of), pair.get(1).and_then(i32_of))
        else {
            continue;
        };
        held.push((id, count));
    }
    held
}

/// The wrapper writes page numbers as JSON integers.
fn i32_of(value: &Value) -> Option<i32> {
    i32::try_from(value.as_i64()?).ok()
}

/// Progress line for the identified step: landed alias, role and id only.
/// Never an invented coord, npc or answer, and never `'clue solved'`.
fn progress(row: &TrailMembershipRow) -> String {
    format!("clue step held: {} {} [{}]", row.role, row.alias, row.id)
}

/// Status line for the identified step. Trail completion — `done`, restore,
/// then the exact `'clue solved'` string — is a later slice, not this one.
fn status(row: &TrailMembershipRow) -> String {
    format!("clue: {}", row.alias)
}

/// The posted snapshot is not this machine's page: begin and next read the
/// page the wrapper hands in at call time, so nothing is cached here.
/// Registered so the machine's hook set matches the isolate's fan-out.
pub fn on_snapshot(_snap: &SnapshotReader<'_>) {}

pub fn on_pause() {
    RUNTIME.with(|rt| {
        let held = rt.borrow().clock.held;
        rt.borrow_mut().clock.set_freeze(true, held);
    });
}

pub fn on_resume() {
    RUNTIME.with(|rt| {
        let held = rt.borrow().clock.held;
        rt.borrow_mut().clock.set_freeze(false, held);
    });
}

pub fn on_hold(held: bool) {
    RUNTIME.with(|rt| {
        let paused = rt.borrow().clock.paused;
        rt.borrow_mut().clock.set_freeze(paused, held);
    });
}

pub fn on_reset() {
    RUNTIME.with(|rt| rt.borrow_mut().abort());
}

/// The machine's only entry point: the selected pin comes from the native
/// registration's captured `game_data`, and the payload is the wrapper's
/// marshalled call — never a host wire.
pub fn dispatch(selected: Option<&SelectedGameData>, input: &Value) -> Value {
    match input.get("op").and_then(Value::as_str).unwrap_or("") {
        "begin" => RUNTIME.with(|rt| rt.borrow_mut().begin(selected, input)),
        "next" => RUNTIME.with(|rt| rt.borrow_mut().next(selected, input)),
        _ => json!({ "kind": "notImpl", "reason": "unknown clue op" }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use client::io::ClientRevision;
    use std::sync::Arc;

    const CASKET: i32 = 3531;
    const CLUE: i32 = 3554;

    fn selected() -> Arc<SelectedGameData> {
        api::game_data::for_revision(ClientRevision::R274).expect("selected data")
    }

    /// A held id that is not a membership row: a challenge-answer row, which
    /// stays unread.
    fn unrelated(data: &SelectedGameData) -> i32 {
        data.trails()
            .expect("trails")
            .challenge_answers
            .first()
            .expect("challenge row")
            .id
    }

    fn payload(op: &str, token: Option<u64>, held: Value, extra: Value) -> Value {
        let mut call = json!({ "op": op, "generation": 4, "held": held });
        if let Some(token) = token {
            call["token"] = json!(token);
        }
        for (key, value) in extra.as_object().expect("extra") {
            call[key] = value.clone();
        }
        call
    }

    fn begin(data: &SelectedGameData, held: Value) -> Value {
        dispatch(Some(data), &payload("begin", None, held, json!({})))
    }

    fn call(data: &SelectedGameData, token: u64, held: Value, extra: Value) -> Value {
        dispatch(Some(data), &payload("next", Some(token), held, extra))
    }

    fn token_of(step: &Value) -> u64 {
        step["token"].as_u64().expect("token")
    }

    #[test]
    fn a_frozen_clock_emits_wait_and_keeps_the_gate_unanswered() {
        on_reset();
        let data = selected();
        let opened = begin(&data, json!([[CASKET, 1]]));
        assert_eq!(opened["kind"], "token");
        let token = token_of(&opened);
        let gate = call(&data, token, json!([[CASKET, 1]]), json!({}));
        assert_eq!(gate["kind"], "callback.enabled", "{gate}");

        // Paused: no callback, no verb, and the answer is not consumed. The
        // freeze wins over the posted interrupt carried in the same call.
        on_pause();
        let paused = call(
            &data,
            token,
            json!([[CASKET, 1]]),
            json!({ "resume": true, "hold": true }),
        );
        assert_eq!(paused["kind"], "wait", "{paused}");
        assert!(paused.get("message").is_none(), "{paused}");
        on_resume();
        let after_pause = call(&data, token, json!([[CASKET, 1]]), json!({}));
        assert_eq!(
            after_pause["kind"], "callback.enabled",
            "a frozen call burns nothing: {after_pause}"
        );

        // A held clock is the same wait.
        on_hold(true);
        let held_clock = call(
            &data,
            token,
            json!([[CASKET, 1]]),
            json!({ "resume": true }),
        );
        assert_eq!(held_clock["kind"], "wait", "{held_clock}");
        on_hold(false);

        // The posted `hold || ours` signal without a frozen clock: yield, and
        // the token survives it.
        let yielded = call(&data, token, json!([[CASKET, 1]]), json!({ "hold": true }));
        assert_eq!(yielded["kind"], "yield", "{yielded}");
        assert_eq!(token_of(&yielded), token, "{yielded}");

        // The gate was still open, so the enabled answer lands now.
        let enabled = call(
            &data,
            token,
            json!([[CASKET, 1]]),
            json!({ "resume": true }),
        );
        assert_eq!(enabled["kind"], "callback.log", "{enabled}");
    }

    #[test]
    fn a_generation_bump_aborts_the_session_without_a_verb() {
        on_reset();
        let data = selected();
        let opened = begin(&data, json!([[CLUE, 1]]));
        let token = token_of(&opened);

        let bumped = dispatch(
            Some(&data),
            &payload(
                "next",
                Some(token),
                json!([[CLUE, 1]]),
                json!({ "generation": 5 }),
            ),
        );
        assert_eq!(bumped["kind"], "aborted", "{bumped}");
        assert_eq!(bumped["reason"], "aborted", "{bumped}");
        for kind in [
            "wait",
            "yield",
            "callback.enabled",
            "callback.log",
            "callback.setStatus",
        ] {
            assert_ne!(bumped["kind"], kind, "{bumped}");
        }
        // The token died with the bump: the same call on the old generation
        // is stale, not a resumed session.
        let after = call(&data, token, json!([[CLUE, 1]]), json!({}));
        assert_eq!(after["kind"], "aborted", "{after}");
        assert_eq!(after["reason"], "stale", "{after}");
    }

    #[test]
    fn enabled_is_re_read_at_every_gate_and_never_captured() {
        on_reset();
        let data = selected();
        // Extra begin keys are ignored, not captured: no `enabled`, `resume`,
        // hold flag or page identity is frozen into the session.
        let opened = dispatch(
            Some(&data),
            &payload(
                "begin",
                None,
                json!([[CLUE, 1]]),
                json!({ "enabled": false, "resume": false, "hold": true }),
            ),
        );
        assert_eq!(opened["kind"], "token", "{opened}");
        let token = token_of(&opened);
        assert!(
            opened.get("kind").is_some() && opened.get("reason").is_none(),
            "begin never returns a continue kind and never a reason: {opened}"
        );

        let first = call(&data, token, json!([[CLUE, 1]]), json!({}));
        assert_eq!(first["kind"], "callback.enabled", "{first}");
        // False means do not execute: an idle continue with the token live.
        let denied = call(&data, token, json!([[CLUE, 1]]), json!({ "resume": false }));
        assert_eq!(denied["kind"], "wait", "{denied}");
        assert_eq!(token_of(&denied), token, "{denied}");
        // The next tick asks again rather than replaying the captured answer.
        let again = call(&data, token, json!([[CLUE, 1]]), json!({}));
        assert_eq!(again["kind"], "callback.enabled", "{again}");
        // And a later true is honored, because nothing was captured at begin.
        let enabled = call(&data, token, json!([[CLUE, 1]]), json!({ "resume": true }));
        assert_eq!(enabled["kind"], "callback.log", "{enabled}");
        let message = enabled["message"].as_str().unwrap_or("");
        assert!(message.contains("trail_clue_hard_sextant028"), "{enabled}");
        assert!(message.contains("3554"), "{enabled}");

        // The constrained 3554 row keeps the token and then idles.
        let steady = call(&data, token, json!([[CLUE, 1]]), json!({}));
        assert_eq!(steady["kind"], "callback.setStatus", "{steady}");
        assert!(
            steady["message"]
                .as_str()
                .unwrap_or("")
                .contains("trail_clue_hard_sextant028"),
            "{steady}"
        );
        for _ in 0..2 {
            let idle = call(&data, token, json!([[CLUE, 1]]), json!({}));
            assert_eq!(idle["kind"], "wait", "{idle}");
            assert_eq!(token_of(&idle), token, "{idle}");
        }
    }

    #[test]
    fn a_new_held_step_re_arms_the_gate() {
        on_reset();
        let data = selected();
        let token = token_of(&begin(&data, json!([[CASKET, 1]])));
        assert_eq!(
            call(&data, token, json!([[CASKET, 1]]), json!({}))["kind"],
            "callback.enabled"
        );
        assert_eq!(
            call(
                &data,
                token,
                json!([[CASKET, 1]]),
                json!({ "resume": true })
            )["kind"],
            "callback.log"
        );
        // The clue replaces the casket: the enabled read is not reused.
        let re_armed = call(&data, token, json!([[CLUE, 1]]), json!({ "resume": true }));
        assert_eq!(re_armed["kind"], "callback.enabled", "{re_armed}");
        assert_eq!(token_of(&re_armed), token, "{re_armed}");
    }

    #[test]
    fn none_held_is_a_refusal_and_never_a_live_token_or_a_done() {
        on_reset();
        let data = selected();
        let challenge = unrelated(&data);
        for held in [json!([]), json!([[challenge, 1]]), json!([[CASKET, 0]])] {
            let refused = begin(&data, held.clone());
            assert_eq!(refused["kind"], "aborted", "{held} {refused}");
            assert_eq!(refused["reason"], "none-held", "{held} {refused}");
            // No live token: the refusal's own number is not a session.
            let after = call(&data, token_of(&refused), held.clone(), json!({}));
            assert_eq!(after["reason"], "stale", "{held} {after}");
        }

        // A live session that loses its held membership errors `none-held`
        // and aborts — it does not quietly become done.
        let token = token_of(&begin(&data, json!([[CASKET, 1]])));
        let lost = call(&data, token, json!([[challenge, 1]]), json!({}));
        assert_eq!(lost["kind"], "aborted", "{lost}");
        assert_eq!(lost["reason"], "none-held", "{lost}");
        let after = call(&data, token, json!([[CASKET, 1]]), json!({}));
        assert_eq!(after["reason"], "stale", "{after}");
    }

    #[test]
    fn a_missing_selected_pin_is_not_none_held() {
        on_reset();
        let refused = dispatch(
            None,
            &payload("begin", None, json!([[CASKET, 1]]), json!({})),
        );
        assert_eq!(refused["kind"], "aborted", "{refused}");
        assert_eq!(refused["reason"], "missing-selected-data", "{refused}");
        // The live token's view of a vanished pin is the same refusal.
        let data = selected();
        let token = token_of(&begin(&data, json!([[CASKET, 1]])));
        let gone = dispatch(
            None,
            &payload("next", Some(token), json!([[CASKET, 1]]), json!({})),
        );
        assert_eq!(gone["kind"], "aborted", "{gone}");
        assert_eq!(gone["reason"], "missing-selected-data", "{gone}");
    }

    #[test]
    fn the_exact_clue_solved_string_is_never_emitted() {
        on_reset();
        let data = selected();
        let token = token_of(&begin(&data, json!([[CASKET, 1]])));
        let steps = vec![
            call(&data, token, json!([[CASKET, 1]]), json!({})),
            call(
                &data,
                token,
                json!([[CASKET, 1]]),
                json!({ "resume": true }),
            ),
            call(&data, token, json!([[CASKET, 1]]), json!({})),
            call(&data, token, json!([[CASKET, 1]]), json!({})),
            call(&data, token, json!([[CASKET, 1]]), json!({ "hold": true })),
        ];
        for step in &steps {
            let text = step.to_string();
            assert!(!text.contains("clue solved"), "{step}");
            assert!(!text.contains("ownsEquipment"), "{step}");
            assert!(
                step["status"].is_null(),
                "the machine emits kinds, not the public status: {step}"
            );
            assert_ne!(step["kind"], "done", "{step}");
        }
        assert_eq!(
            steps
                .iter()
                .map(|step| step["kind"].clone())
                .collect::<Vec<_>>(),
            vec![
                json!("callback.enabled"),
                json!("callback.log"),
                json!("callback.setStatus"),
                json!("wait"),
                json!("yield"),
            ],
            "{steps:?}"
        );
    }

    #[test]
    fn an_unknown_op_is_not_impl() {
        on_reset();
        let data = selected();
        let step = dispatch(Some(&data), &json!({ "op": "solve" }));
        assert_eq!(step["kind"], "notImpl", "{step}");
    }
}
