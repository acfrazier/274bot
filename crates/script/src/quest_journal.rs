//! Owned-root quest-journal machine: `api.questJournalBegin` / `Next` / `Close`.
//!
//! One token per isolate over the posted quest row and the paired main-modal
//! texts. The JS wrapper owns the call arguments, the posted tab and the row
//! selection. The pair and its tick are read from the isolate scene; the
//! wrapper's echo of the same page is only read before the scene has carried
//! the pair this session. This machine owns the token, the generation
//! captured at Begin, the frozen clock, acquisition, the owned-root refusal
//! and the one close.
//!
//! The pair is the only occupancy fact. The closed start is the explicit
//! `{ root: -1, texts: [] }` the host posted; an omitted pair is not free,
//! `main_modal_id` is not a second closed definition, and the closed pair is
//! never acquired.
//!
//! One `if-button` per token, one `close-modal` per token. Begin of another
//! name cancels the live token and emits no close for it; the same name is
//! `busy`. Reset and a generation bump abort the token. Pause and hold freeze
//! this machine's own clock, so a frozen call emits no verb and does not burn
//! the acquisition window. There is no Stop arm and nothing is enqueued from
//! `onStop`.

use crate::observed;
use crate::task_clock::InstantTaskClock;
use serde_json::{json, Value};
use std::cell::RefCell;

/// Frozen acquisition window: the click has to produce a paired post whose
/// root is not `-1` inside this window, or the token is `modal-timeout`. This
/// is the journal machine's own window — it is not `modals::CLOSE_TIMEOUT_MS`
/// and the public timeout is `modal-timeout`, not `timeout`.
pub const ACQUIRE_TIMEOUT_MS: u64 = 3_000;

thread_local! {
    static RUNTIME: RefCell<JournalRuntime> = const { RefCell::new(JournalRuntime::new()) };
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    /// No token: a refused call leaves this in place.
    Idle,
    /// The click was returned; the paired post has not acquired yet.
    AwaitingAcquire,
    /// The pair this token opened: root, texts and its sequence are stored.
    Acquired,
    /// One `close-modal` was returned; the closed pair has not landed yet.
    Closing,
}

struct JournalRuntime {
    clock: InstantTaskClock,
    token: u64,
    phase: Phase,
    /// A–Z folded name of the live token (the wrapper's selection key).
    name: String,
    component_id: i32,
    generation: u64,
    root: i32,
    texts: Vec<String>,
    sequence: u64,
}

impl JournalRuntime {
    const fn new() -> Self {
        Self {
            clock: InstantTaskClock::new(),
            token: 0,
            phase: Phase::Idle,
            name: String::new(),
            component_id: 0,
            generation: 0,
            root: -1,
            texts: Vec::new(),
            sequence: 0,
        }
    }

    fn frozen(&self) -> bool {
        self.clock.frozen()
    }

    fn abort(&mut self) {
        self.token = self.token.wrapping_add(1);
        self.phase = Phase::Idle;
        self.name.clear();
        self.component_id = 0;
        self.generation = 0;
        self.root = -1;
        self.texts.clear();
        self.sequence = 0;
        self.clock.deadline = None;
    }

    fn aborted(&mut self, reason: &str) -> Value {
        self.abort();
        json!({ "kind": "aborted", "token": self.token, "reason": reason })
    }

    /// The stored acquired pair is still the live pair, tags and order
    /// included. A generation bump is checked before this.
    fn still_owned(&self, root: i32, texts: &[String]) -> bool {
        root == self.root && texts == self.texts.as_slice()
    }

    fn begin(&mut self, input: &Value) -> Value {
        let name = input.get("name").and_then(Value::as_str).unwrap_or("");
        let component_id = input
            .get("component_id")
            .and_then(Value::as_i64)
            .and_then(|id| i32::try_from(id).ok());
        let generation = input.get("generation").and_then(Value::as_u64);
        let sequence = posted_sequence(input);
        let (Some(component_id), Some(generation), Some(sequence)) =
            (component_id, generation, sequence)
        else {
            // The wrapper refuses these before the machine; a direct call
            // still has to be a refusal, never a click on a guessed target.
            return self.aborted("snapshot-unavailable");
        };
        let Some((root, texts)) = posted_pair(input) else {
            return self.aborted("snapshot-unavailable");
        };
        // One token. The same name is busy and is not cancelled: a second
        // click is the steal the owned-root contract forbids. Another name
        // cancels the live token and emits no verb for it.
        if self.phase != Phase::Idle {
            if self.name == name {
                return json!({
                    "kind": "aborted",
                    "token": self.token,
                    "reason": "busy",
                });
            }
            self.abort();
        }
        // Only the explicit closed pair is a free modal.
        if root != -1 {
            return self.aborted("main-modal-occupied");
        }
        if !texts.is_empty() {
            return self.aborted("snapshot-unavailable");
        }
        if self.frozen() {
            // A frozen begin does not click and does not arm the window: a
            // click that was never returned is not timed out later.
            return self.aborted("frozen");
        }
        self.token = self.token.wrapping_add(1);
        self.phase = Phase::AwaitingAcquire;
        self.name = name.to_string();
        self.component_id = component_id;
        self.generation = generation;
        self.root = -1;
        self.texts.clear();
        self.sequence = sequence;
        self.clock.arm(ACQUIRE_TIMEOUT_MS);
        json!({
            "kind": "if-button",
            "token": self.token,
            "component_id": component_id,
        })
    }

    fn next(&mut self, input: &Value) -> Value {
        let Some(token) = input.get("token").and_then(Value::as_u64) else {
            return self.aborted("stale");
        };
        if token != self.token || self.phase == Phase::Idle {
            return self.aborted("stale");
        }
        if input.get("generation").and_then(Value::as_u64) != Some(self.generation) {
            return self.aborted("stale");
        }
        let Some((root, texts)) = posted_pair(input) else {
            return json!({
                "kind": "aborted",
                "token": self.token,
                "reason": "snapshot-unavailable",
            });
        };
        if root == -1 && !texts.is_empty() {
            // Not the closed pair and not a modal: the pair is unusable.
            return json!({
                "kind": "aborted",
                "token": self.token,
                "reason": "snapshot-unavailable",
            });
        }
        if self.frozen() {
            // Frozen: no verb and no burn. `bound_reached` reads the frozen
            // instant, so the window does not advance.
            return json!({ "kind": "wait", "token": self.token });
        }
        let sequence = posted_sequence(input).unwrap_or(0);
        match self.phase {
            Phase::Idle => self.aborted("stale"),
            Phase::AwaitingAcquire => {
                if root != -1 {
                    // The first later pair whose root is not -1 is this
                    // token's journal. Empty lines on a positive root are a
                    // real walk, not a reason to keep waiting.
                    self.root = root;
                    self.texts = texts;
                    self.sequence = sequence;
                    self.phase = Phase::Acquired;
                    self.clock.deadline = None;
                    return json!({
                        "kind": "done",
                        "token": self.token,
                        "lines": self.texts,
                        "root": self.root,
                        "as_of_sequence": sequence,
                    });
                }
                if self.clock.bound_reached() {
                    return self.aborted("modal-timeout");
                }
                // A still-closed pair is wait, never empty lines.
                json!({ "kind": "wait", "token": self.token })
            }
            Phase::Acquired | Phase::Closing => {
                if self.still_owned(root, &texts) {
                    // The live pair was just re-checked at this tick, so the
                    // answer is as of this observation: the acquired texts,
                    // the acquired root, and the page tick that carried them.
                    json!({
                        "kind": "done",
                        "token": self.token,
                        "lines": self.texts,
                        "root": self.root,
                        "as_of_sequence": sequence,
                    })
                } else {
                    // A replacement root, or the same root with different
                    // texts, is not this token's journal.
                    self.aborted("stale")
                }
            }
        }
    }

    fn close(&mut self, input: &Value) -> Value {
        let Some(token) = input.get("token").and_then(Value::as_u64) else {
            return self.aborted("stale");
        };
        if token != self.token || self.phase == Phase::Idle {
            return self.aborted("stale");
        }
        if input.get("generation").and_then(Value::as_u64) != Some(self.generation) {
            return self.aborted("stale");
        }
        let Some((root, texts)) = posted_pair(input) else {
            return json!({
                "kind": "aborted",
                "token": self.token,
                "reason": "snapshot-unavailable",
            });
        };
        if root == -1 && !texts.is_empty() {
            return json!({
                "kind": "aborted",
                "token": self.token,
                "reason": "snapshot-unavailable",
            });
        }
        if self.frozen() {
            return json!({ "kind": "wait", "token": self.token });
        }
        let sequence = posted_sequence(input).unwrap_or(0);
        match self.phase {
            Phase::Idle => self.aborted("stale"),
            Phase::AwaitingAcquire => {
                // Close before a positive root is acquired emits nothing and
                // is not a close of `main_modal_id`.
                self.aborted("stale")
            }
            Phase::Acquired => {
                if self.still_owned(root, &texts) {
                    // One close-modal, only while the latest pair is still
                    // the acquired root and the acquired texts.
                    self.phase = Phase::Closing;
                    json!({ "kind": "close-modal", "token": self.token })
                } else {
                    self.aborted("stale")
                }
            }
            Phase::Closing => {
                if root == -1 && texts.is_empty() {
                    // The only close success: a later explicit closed pair.
                    let token = self.token;
                    self.abort();
                    json!({
                        "kind": "done",
                        "token": token,
                        "as_of_sequence": sequence,
                    })
                } else if self.still_owned(root, &texts) {
                    // Still open. A second close does not emit another verb.
                    json!({ "kind": "wait", "token": self.token })
                } else {
                    self.aborted("stale")
                }
            }
        }
    }
}

/// The paired main-modal walk this call reads: the pair the host posted to
/// the isolate scene, or the wrapper's echo before the scene carried one.
fn posted_pair(input: &Value) -> Option<(i32, Vec<String>)> {
    observed::with(|scene| {
        scene
            .latest()
            .main_modal_texts()
            .map(|pair| (pair.root, pair.texts.clone()))
    })
    .or_else(|| read_pair(input))
}

/// The tick that carried [`posted_pair`]'s pair.
fn posted_sequence(input: &Value) -> Option<u64> {
    observed::with(|scene| scene.latest().main_modal_texts().and(scene.tick()))
        .or_else(|| input.get("sequence").and_then(Value::as_u64))
}

/// The paired object the materializer wrote, as echoed by the wrapper.
/// `None` is unusable: a root that is not an integer, or texts that are not
/// strings. An omitted pair never reaches here — the wrapper refuses it.
fn read_pair(input: &Value) -> Option<(i32, Vec<String>)> {
    let root = i32::try_from(input.get("root")?.as_i64()?).ok()?;
    let rows = input.get("texts")?.as_array()?;
    let mut texts = Vec::with_capacity(rows.len());
    for row in rows {
        texts.push(row.as_str()?.to_string());
    }
    Some((root, texts))
}

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

pub fn dispatch(input: &Value) -> Value {
    match input.get("op").and_then(Value::as_str).unwrap_or("") {
        "begin" => RUNTIME.with(|rt| rt.borrow_mut().begin(input)),
        "next" => RUNTIME.with(|rt| rt.borrow_mut().next(input)),
        "close" => RUNTIME.with(|rt| rt.borrow_mut().close(input)),
        _ => json!({ "kind": "notImpl", "reason": "unknown quest journal op" }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn begin(name: &str, generation: u64) -> Value {
        dispatch(&json!({
            "op": "begin",
            "name": name,
            "component_id": 1234,
            "sequence": 7,
            "generation": generation,
            "root": -1,
            "texts": [],
        }))
    }

    fn call(
        op: &str,
        token: u64,
        generation: u64,
        root: i32,
        texts: Value,
        sequence: u64,
    ) -> Value {
        dispatch(&json!({
            "op": op,
            "token": token,
            "generation": generation,
            "root": root,
            "texts": texts,
            "sequence": sequence,
        }))
    }

    #[test]
    fn a_generation_mismatch_is_stale_without_a_verb() {
        on_reset();
        let begin = begin("Cook's Assistant", 4);
        assert_eq!(begin["kind"], "if-button");
        let token = begin["token"].as_u64().unwrap();
        let step = call("next", token, 5, 77, json!(["@dre@The Cook's Quest"]), 8);
        assert_eq!(step["kind"], "aborted");
        assert_eq!(step["reason"], "stale");
        // The token died with the mismatch.
        let after = call("next", token, 5, 77, json!(["@dre@The Cook's Quest"]), 9);
        assert_eq!(after["reason"], "stale");
    }

    #[test]
    fn same_name_is_busy_and_another_name_cancels_before_the_occupied_refusal() {
        on_reset();
        let first = begin("Cook's Assistant", 4);
        let token = first["token"].as_u64().unwrap();
        let busy = begin("Cook's Assistant", 4);
        assert_eq!(busy["kind"], "aborted");
        assert_eq!(busy["reason"], "busy");

        // Another name cancels the old token first, then refuses the
        // occupied pair: the old token is dead and no verb was returned.
        let mut occupied = json!({
            "op": "begin",
            "name": "Waterfall Quest",
            "component_id": 42,
            "sequence": 7,
            "generation": 4,
            "root": 77,
            "texts": ["@dre@The Cook's Quest"],
        });
        let step = dispatch(&occupied);
        assert_eq!(step["kind"], "aborted");
        assert_eq!(step["reason"], "main-modal-occupied");
        let cancelled = call("next", token, 4, 77, json!(["@dre@The Cook's Quest"]), 7);
        assert_eq!(
            cancelled["reason"], "stale",
            "the cancelled token is not live: {cancelled}"
        );
        occupied["texts"] = json!([]);
        occupied["root"] = json!(-1);
        let restarted = dispatch(&occupied);
        assert_eq!(restarted["kind"], "if-button");
    }

    #[test]
    fn the_owned_close_succeeds_only_on_the_explicit_closed_pair() {
        on_reset();
        let token = begin("Cook's Assistant", 1)["token"].as_u64().unwrap();
        let acquired = call("next", token, 1, 77, json!(["@dre@The Cook's Quest"]), 8);
        assert_eq!(acquired["kind"], "done");
        let emitted = call("close", token, 1, 77, json!(["@dre@The Cook's Quest"]), 9);
        assert_eq!(emitted["kind"], "close-modal");
        // A second close while the pair still stands is wait, not a verb.
        let again = call("close", token, 1, 77, json!(["@dre@The Cook's Quest"]), 10);
        assert_eq!(again["kind"], "wait");
        let closed = call("close", token, 1, -1, json!([]), 11);
        assert_eq!(closed["kind"], "done");
        assert_eq!(closed["as_of_sequence"], 11);
    }

    #[test]
    fn the_posted_pair_is_read_from_the_scene_over_a_stale_echo() {
        on_reset();
        observed::on_reset();
        let post = |tick: u64, root: i32, texts: &[&str]| {
            observed::post(tick, |post| {
                post.main_modal_texts(observed::ModalTexts {
                    root,
                    texts: texts.iter().map(|text| (*text).to_string()).collect(),
                });
            });
        };
        post(7, -1, &[]);
        let token = begin("Cook's Assistant", 1)["token"].as_u64().unwrap();
        post(8, 77, &["@dre@The Cook's Quest"]);
        // The echo still carries the closed pair from an older page.
        let acquired = call("next", token, 1, -1, json!([]), 7);
        assert_eq!(acquired["kind"], "done", "{acquired}");
        assert_eq!(acquired["root"], 77);
        assert_eq!(acquired["lines"], json!(["@dre@The Cook's Quest"]));
        assert_eq!(acquired["as_of_sequence"], 8);
    }
}
