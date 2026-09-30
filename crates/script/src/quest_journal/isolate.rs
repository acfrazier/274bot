//! Owned-root quest-journal machine: `api.questJournalBegin` plus one awaited
//! `api.questJournalRun`.
//!
//! One token per isolate over the posted quest row and the paired main-modal
//! texts. The JS wrapper only validates begin/run arguments and awaits the
//! machine. Rust reads the isolate scene, emits the one `if-button` and
//! `close-modal`, and owns the token, acquisition, close and frozen clock.
//!
//! The pair is the only occupancy fact. The closed start is the explicit
//! `{ root: -1, texts: [] }` the host posted; an omitted pair is not free,
//! `main_modal_id` is not a second closed definition, and the closed pair is
//! never acquired.
//!
//! One `if-button` per token, one `close-modal` per token. Begin of another
//! name cancels the live token and emits no verb for it; the same name is
//! `busy`. Reset and a generation bump abort the token. Pause and hold freeze
//! this machine's own clock, so a frozen call emits no verb and does not burn
//! the acquisition window. There is no Stop arm and nothing is enqueued from
//! `onStop`.

use crate::machine::{AbortReason, Begin, Cx, Family, Step};
use crate::observed::{self, QuestTab};
use crate::scene_query;
use crate::shim::InteractReq;
use crate::task_clock::InstantTaskClock;
use serde::Deserialize;
use serde_json::{json, Value};
use std::cell::RefCell;

/// Frozen observation window: the click has to produce a paired post whose
/// root is not `-1`, and a dispatched close has to produce an explicit closed
/// pair, inside this window. Otherwise the token is `modal-timeout`. This is
/// the journal machine's own window — it is not
/// `modals::CLOSE_TIMEOUT_MS` — and the public timeout is `modal-timeout`, not
/// `timeout`.
pub(crate) const ACQUIRE_TIMEOUT_MS: u64 = 3_000;
const COMPAT_PAINT_QUIET_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

#[derive(Clone, Copy, PartialEq, Eq)]
struct CompatJournalKey {
    token: u64,
    generation: u64,
}

struct CompatJournalLeaseState {
    key: CompatJournalKey,
    since: std::time::Instant,
}

/// Shared Rust-only quiet state for a compatibility journal machine. The
/// isolate thread is the producer and the host reads it without a JS/shim
/// round trip. Both token and work generation fence every transition.
pub(crate) struct CompatJournalLease {
    state: parking_lot::Mutex<Option<CompatJournalLeaseState>>,
    active: std::sync::atomic::AtomicBool,
    producer_generation: std::sync::atomic::AtomicU64,
    host_generation: std::sync::atomic::AtomicU64,
}

impl Default for CompatJournalLease {
    fn default() -> Self {
        Self {
            state: parking_lot::Mutex::new(None),
            active: std::sync::atomic::AtomicBool::new(false),
            producer_generation: std::sync::atomic::AtomicU64::new(0),
            host_generation: std::sync::atomic::AtomicU64::new(0),
        }
    }
}

impl CompatJournalLease {
    pub(crate) fn set_generation(&self, generation: u64) {
        self.producer_generation
            .store(generation, std::sync::atomic::Ordering::Release);
    }

    pub(crate) fn set_host_generation(&self, generation: u64) {
        self.host_generation
            .store(generation, std::sync::atomic::Ordering::Release);
    }

    fn generation(&self) -> u64 {
        self.producer_generation
            .load(std::sync::atomic::Ordering::Acquire)
    }

    pub(crate) fn acquire(&self, token: u64, generation: u64, since: std::time::Instant) {
        if self.generation() != generation
            || self
                .host_generation
                .load(std::sync::atomic::Ordering::Acquire)
                != generation
        {
            return;
        }
        let mut state = self.state.lock();
        if self.generation() != generation
            || self
                .host_generation
                .load(std::sync::atomic::Ordering::Acquire)
                != generation
        {
            return;
        }
        *state = Some(CompatJournalLeaseState {
            key: CompatJournalKey { token, generation },
            since,
        });
        self.active
            .store(true, std::sync::atomic::Ordering::Release);
    }

    pub(crate) fn clear(&self, token: u64, generation: u64) {
        let mut state = self.state.lock();
        if state
            .as_ref()
            .is_some_and(|active| active.key == (CompatJournalKey { token, generation }))
        {
            *state = None;
            self.active
                .store(false, std::sync::atomic::Ordering::Release);
        }
    }

    pub(crate) fn revoke(&self) {
        *self.state.lock() = None;
        self.active
            .store(false, std::sync::atomic::Ordering::Release);
    }

    pub(crate) fn live(&self, generation: u64, now: std::time::Instant) -> bool {
        if !self.active.load(std::sync::atomic::Ordering::Acquire) {
            return false;
        }
        let mut state = self.state.lock();
        let Some(active) = state.as_ref() else {
            self.active
                .store(false, std::sync::atomic::Ordering::Release);
            return false;
        };
        if active.key.generation != generation
            || self.generation() != generation
            || self
                .host_generation
                .load(std::sync::atomic::Ordering::Acquire)
                != generation
            || now.saturating_duration_since(active.since) >= COMPAT_PAINT_QUIET_TIMEOUT
        {
            *state = None;
            self.active
                .store(false, std::sync::atomic::Ordering::Release);
            return false;
        }
        true
    }
}

thread_local! {
    static RUNTIME: RefCell<JournalRuntime> = const { RefCell::new(JournalRuntime::new()) };
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    /// No token: a refused call leaves this in place.
    Idle,
    /// Begin admitted the token; the machine has not emitted its click yet.
    Ready,
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
    compat_lease: Option<std::sync::Arc<CompatJournalLease>>,
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
            compat_lease: None,
        }
    }

    fn bind_compat(&mut self, lease: std::sync::Arc<CompatJournalLease>) {
        self.abort();
        self.compat_lease = Some(lease);
    }

    fn current_generation(&self, requested: u64) -> u64 {
        self.compat_lease
            .as_ref()
            .map_or(requested, |lease| lease.generation())
    }

    fn arm_compat(&self) {
        if let Some(lease) = &self.compat_lease {
            lease.acquire(self.token, self.generation, std::time::Instant::now());
        }
    }

    fn clear_compat(&self) {
        if let Some(lease) = &self.compat_lease {
            lease.clear(self.token, self.generation);
        }
    }

    fn revoke_compat(&self) {
        if let Some(lease) = &self.compat_lease {
            lease.revoke();
        }
    }

    fn compat_live(&self) -> bool {
        self.compat_lease
            .as_ref()
            .is_none_or(|lease| lease.live(self.generation, std::time::Instant::now()))
    }

    fn frozen(&self) -> bool {
        self.clock.frozen()
    }

    fn abort(&mut self) {
        self.clear_compat();
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

    /// A lookup refusal. Does not bump the token or drop a live journal.
    fn refuse(&self, reason: &str) -> Value {
        json!({ "kind": "aborted", "token": self.token, "reason": reason })
    }

    /// A missing or internally inconsistent pair while an observation is in
    /// flight is not a terminal. Acquisition and closing each arm their own
    /// frozen window, so a permanently unusable page eventually releases the
    /// token as `modal-timeout`.
    fn observation_unavailable(&mut self) -> Value {
        if self.frozen() || !self.clock.bound_reached() {
            json!({ "kind": "wait", "token": self.token })
        } else {
            self.aborted("modal-timeout")
        }
    }

    /// The stored acquired pair is still the live pair, tags and order
    /// included. A generation bump is checked before this.
    fn still_owned(&self, root: i32, texts: &[String]) -> bool {
        root == self.root && texts == self.texts.as_slice()
    }

    fn begin(&mut self, input: &Value) -> Value {
        let name = input.get("name").and_then(Value::as_str).unwrap_or("");
        let Some(requested_generation) = input.get("generation").and_then(Value::as_u64) else {
            // The wrapper refuses a missing generation; a direct call still
            // has to be a refusal, never a click on a guessed target.
            return self.refuse("snapshot-unavailable");
        };
        let folded = scene_query::fold_ascii(scene_query::js_trim(name));
        let lookup = observed::with(|scene| {
            let Some(tab) = scene.latest().quest_statuses() else {
                return Err("snapshot-unavailable");
            };
            match tab {
                QuestTab::Unbound => Err("quest-tab-unbound"),
                QuestTab::Bound(rows) => {
                    let Some(pair) = scene.latest().main_modal_texts() else {
                        return Err("snapshot-unavailable");
                    };
                    let Some(sequence) = scene.tick() else {
                        return Err("snapshot-unavailable");
                    };
                    let Some(row) = scene_query::first_quest_row(rows, &folded) else {
                        return Err("unknown-quest");
                    };
                    let Some(component_id) = row.component_id else {
                        return Err("snapshot-unavailable");
                    };
                    Ok((component_id, pair.root, pair.texts.clone(), sequence))
                }
            }
        });
        let (component_id, root, texts, sequence) = match lookup {
            Ok(hit) => hit,
            Err(reason) => return self.refuse(reason),
        };
        // One token. The same name is busy and is not cancelled: a second
        // click is the steal the owned-root contract forbids. Another name
        // cancels the live token and emits no verb for it.
        if self.phase != Phase::Idle {
            if self.name == folded {
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
            // A frozen begin does not admit a token or arm the window.
            return self.aborted("frozen");
        }
        let generation = self.current_generation(requested_generation);
        self.token = self.token.wrapping_add(1);
        self.phase = Phase::Ready;
        self.name = folded;
        self.component_id = component_id;
        self.generation = generation;
        self.root = -1;
        self.texts.clear();
        self.sequence = sequence;
        self.clock.deadline = None;
        self.arm_compat();
        json!({ "kind": "token", "token": self.token })
    }

    fn next(&mut self, token: u64, generation: u64) -> Value {
        if token != self.token || self.phase == Phase::Idle {
            return self.aborted("stale");
        }
        if generation != self.generation {
            return self.aborted("stale");
        }
        if !self.compat_live() {
            return self.aborted("modal-timeout");
        }
        let Some((root, texts)) = posted_pair() else {
            return if self.phase == Phase::AwaitingAcquire {
                self.observation_unavailable()
            } else {
                self.refuse("snapshot-unavailable")
            };
        };
        if root == -1 && !texts.is_empty() {
            return if self.phase == Phase::AwaitingAcquire {
                self.observation_unavailable()
            } else {
                self.refuse("snapshot-unavailable")
            };
        }
        if self.frozen() {
            // Frozen: no verb and no burn. `bound_reached` reads the frozen
            // instant, so the window does not advance.
            return json!({ "kind": "wait", "token": self.token });
        }
        let sequence = posted_sequence().unwrap_or(0);
        match self.phase {
            Phase::Ready => json!({ "kind": "wait", "token": self.token }),
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

    fn close(&mut self, token: u64, generation: u64) -> Value {
        if token != self.token || self.phase == Phase::Idle {
            return self.aborted("stale");
        }
        if generation != self.generation {
            return self.aborted("stale");
        }
        if !self.compat_live() {
            return self.aborted("modal-timeout");
        }
        let Some((root, texts)) = posted_pair() else {
            return if self.phase == Phase::Closing {
                self.observation_unavailable()
            } else {
                self.refuse("snapshot-unavailable")
            };
        };
        if root == -1 && !texts.is_empty() {
            return if self.phase == Phase::Closing {
                self.observation_unavailable()
            } else {
                self.refuse("snapshot-unavailable")
            };
        }
        if self.frozen() {
            return json!({ "kind": "wait", "token": self.token });
        }
        let sequence = posted_sequence().unwrap_or(0);
        match self.phase {
            Phase::Ready => self.aborted("stale"),
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
                    self.clock.arm(ACQUIRE_TIMEOUT_MS);
                    json!({ "kind": "close-modal", "token": self.token })
                } else {
                    self.aborted("stale")
                }
            }
            Phase::Closing => {
                if root == -1 && texts.is_empty() {
                    // Preserve both observations: the lines/root page the
                    // caller asked for and the later page that proved closed.
                    let token = self.token;
                    let lines = self.texts.clone();
                    let root = self.root;
                    let as_of_sequence = self.sequence;
                    self.abort();
                    json!({
                        "kind": "done",
                        "token": token,
                        "lines": lines,
                        "root": root,
                        "as_of_sequence": as_of_sequence,
                        "closed_as_of_sequence": sequence,
                    })
                } else if self.still_owned(root, &texts) {
                    // Still open. A second close does not emit another verb,
                    // but a permanently open modal cannot retain the token.
                    self.observation_unavailable()
                } else {
                    self.aborted("stale")
                }
            }
        }
    }
}

#[derive(Deserialize)]
pub(crate) struct QuestJournalArgs {
    token: u64,
}

/// Drives one admitted journal token from its click through the observed close.
pub(crate) struct QuestJournal {
    token: u64,
}

impl QuestJournal {
    fn stale(&self) -> Value {
        json!({ "kind": "aborted", "token": self.token, "reason": "stale" })
    }
}

impl Family for QuestJournal {
    const NAME: &'static str = "quest-journal";
    const EXCLUSIVE: bool = true;
    const KICK_ON_START: bool = true;
    type Args = QuestJournalArgs;
    type Output = Value;

    fn begin(args: QuestJournalArgs, _cx: &mut Cx<'_>) -> Begin<Self> {
        let live = RUNTIME.with(|rt| {
            let rt = rt.borrow();
            rt.phase != Phase::Idle && rt.token == args.token
        });
        if live {
            Begin::Run(Self { token: args.token })
        } else {
            Begin::Refuse("stale".into())
        }
    }

    fn step(&mut self, cx: &mut Cx<'_>) -> Step<Value> {
        loop {
            let phase = RUNTIME.with(|rt| {
                let rt = rt.borrow();
                (rt.phase != Phase::Idle && rt.token == self.token).then_some(rt.phase)
            });
            let Some(phase) = phase else {
                return Step::Done(self.stale());
            };
            match phase {
                Phase::Idle => return Step::Done(self.stale()),
                Phase::Ready => {
                    let component_id = RUNTIME.with(|rt| {
                        let mut rt = rt.borrow_mut();
                        if rt.frozen() {
                            return Ok(None);
                        }
                        if !rt.compat_live() {
                            return Err(rt.aborted("modal-timeout"));
                        }
                        rt.phase = Phase::AwaitingAcquire;
                        rt.clock.arm(ACQUIRE_TIMEOUT_MS);
                        Ok(Some(rt.component_id))
                    });
                    let component_id = match component_id {
                        Ok(Some(component_id)) => component_id,
                        Ok(None) => return Step::Wait,
                        Err(done) => return Step::Done(done),
                    };
                    cx.emit(InteractReq::IfButton { component_id });
                    return Step::Wait;
                }
                Phase::AwaitingAcquire => {
                    let step = RUNTIME.with(|rt| {
                        let mut rt = rt.borrow_mut();
                        let generation = rt.generation;
                        rt.next(self.token, generation)
                    });
                    match step.get("kind").and_then(Value::as_str) {
                        Some("wait") => return Step::Wait,
                        Some("done") => continue,
                        _ => return Step::Done(step),
                    }
                }
                Phase::Acquired | Phase::Closing => {
                    let step = RUNTIME.with(|rt| {
                        let mut rt = rt.borrow_mut();
                        let generation = rt.generation;
                        rt.close(self.token, generation)
                    });
                    match step.get("kind").and_then(Value::as_str) {
                        Some("close-modal") => {
                            cx.emit(InteractReq::CloseModal);
                            return Step::Wait;
                        }
                        Some("wait") => return Step::Wait,
                        _ => return Step::Done(step),
                    }
                }
            }
        }
    }

    fn abort(&mut self, why: AbortReason) {
        // A newer run continues the same owned token. Reset and termination
        // kill it; a newer begin already changed the runtime token.
        if why == AbortReason::Superseded {
            return;
        }
        RUNTIME.with(|rt| {
            let mut rt = rt.borrow_mut();
            if rt.phase != Phase::Idle && rt.token == self.token {
                rt.abort();
            }
        });
    }
}

/// The paired main-modal walk this call reads from the isolate scene.
fn posted_pair() -> Option<(i32, Vec<String>)> {
    observed::with(|scene| {
        scene
            .latest()
            .main_modal_texts()
            .map(|pair| (pair.root, pair.texts.clone()))
    })
}

/// The sequence stamped on the answer: the last posted tick once the scene
/// holds a pair (the pair itself may have been carried by an earlier post).
fn posted_sequence() -> Option<u64> {
    observed::with(|scene| scene.latest().main_modal_texts().and(scene.tick()))
}

pub(crate) fn on_pause() {
    RUNTIME.with(|rt| {
        let held = rt.borrow().clock.held;
        rt.borrow_mut().clock.set_freeze(true, held);
    });
}

pub(crate) fn on_resume() {
    RUNTIME.with(|rt| {
        let held = rt.borrow().clock.held;
        rt.borrow_mut().clock.set_freeze(false, held);
    });
}

pub(crate) fn on_hold(held: bool) {
    RUNTIME.with(|rt| {
        let paused = rt.borrow().clock.paused;
        rt.borrow_mut().clock.set_freeze(paused, held);
    });
}

pub(crate) fn bind_compat_runtime(lease: std::sync::Arc<CompatJournalLease>) {
    RUNTIME.with(|rt| rt.borrow_mut().bind_compat(lease));
}

pub(crate) fn on_stop() {
    RUNTIME.with(|rt| {
        let mut rt = rt.borrow_mut();
        rt.abort();
        rt.revoke_compat();
    });
}

pub(crate) fn on_reset() {
    RUNTIME.with(|rt| {
        let mut rt = rt.borrow_mut();
        rt.abort();
        rt.revoke_compat();
    });
}

pub(crate) fn dispatch(input: &Value) -> Value {
    match input.get("op").and_then(Value::as_str).unwrap_or("") {
        "begin" => RUNTIME.with(|rt| rt.borrow_mut().begin(input)),
        _ => json!({ "kind": "notImpl", "reason": "unknown quest journal op" }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::observed::{ModalTexts, QuestStatusRow, QuestTab};

    fn cook_row() -> QuestStatusRow {
        QuestStatusRow {
            name: "Cook's Assistant".into(),
            status: "notStarted".into(),
            component_id: Some(1234),
        }
    }

    fn waterfall_row() -> QuestStatusRow {
        QuestStatusRow {
            name: "Waterfall Quest".into(),
            status: "notStarted".into(),
            component_id: Some(42),
        }
    }

    fn post_tab(tick: u64, root: i32, texts: &[&str], rows: Vec<QuestStatusRow>) {
        observed::post(tick, |post| {
            post.quest_statuses(QuestTab::Bound(rows));
            post.main_modal_texts(ModalTexts {
                root,
                texts: texts.iter().map(|text| (*text).to_string()).collect(),
            });
        });
    }

    fn reset_closed() {
        on_reset();
        observed::on_reset();
        post_tab(7, -1, &[], vec![cook_row(), waterfall_row()]);
    }

    fn begin(name: &str, generation: u64) -> Value {
        let out = dispatch(&json!({
            "op": "begin",
            "name": name,
            "generation": generation,
        }));
        if out.get("kind").and_then(Value::as_str) == Some("token") {
            RUNTIME.with(|rt| {
                let mut rt = rt.borrow_mut();
                rt.phase = Phase::AwaitingAcquire;
                rt.clock.arm(ACQUIRE_TIMEOUT_MS);
            });
        }
        out
    }

    fn call(op: &str, token: u64, generation: u64) -> Value {
        RUNTIME.with(|rt| {
            let mut rt = rt.borrow_mut();
            match op {
                "next" => rt.next(token, generation),
                "close" => rt.close(token, generation),
                _ => json!({ "kind": "notImpl" }),
            }
        })
    }

    #[test]
    fn a_generation_mismatch_is_stale_without_a_verb() {
        reset_closed();
        let begin = begin("Cook's Assistant", 4);
        assert_eq!(begin["kind"], "token");
        let token = begin["token"].as_u64().unwrap();
        post_tab(
            8,
            77,
            &["@dre@The Cook's Quest"],
            vec![cook_row(), waterfall_row()],
        );
        let step = call("next", token, 5);
        assert_eq!(step["kind"], "aborted");
        assert_eq!(step["reason"], "stale");
        // The token died with the mismatch.
        let after = call("next", token, 5);
        assert_eq!(after["reason"], "stale");
    }

    #[test]
    fn same_name_is_busy_and_another_name_cancels_before_the_occupied_refusal() {
        reset_closed();
        let first = begin("Cook's Assistant", 4);
        let token = first["token"].as_u64().unwrap();
        let busy = begin("Cook's Assistant", 4);
        assert_eq!(busy["kind"], "aborted");
        assert_eq!(busy["reason"], "busy");

        // Another name cancels the old token first, then refuses the
        // occupied pair: the old token is dead and no verb was returned.
        post_tab(
            8,
            77,
            &["@dre@The Cook's Quest"],
            vec![cook_row(), waterfall_row()],
        );
        let step = begin("Waterfall Quest", 4);
        assert_eq!(step["kind"], "aborted");
        assert_eq!(step["reason"], "main-modal-occupied");
        let cancelled = call("next", token, 4);
        assert_eq!(
            cancelled["reason"], "stale",
            "the cancelled token is not live: {cancelled}"
        );
        post_tab(9, -1, &[], vec![cook_row(), waterfall_row()]);
        let restarted = begin("Waterfall Quest", 4);
        assert_eq!(restarted["kind"], "token");
    }

    #[test]
    fn the_owned_close_succeeds_only_on_the_explicit_closed_pair() {
        reset_closed();
        let token = begin("Cook's Assistant", 1)["token"].as_u64().unwrap();
        post_tab(
            8,
            77,
            &["@dre@The Cook's Quest"],
            vec![cook_row(), waterfall_row()],
        );
        let acquired = call("next", token, 1);
        assert_eq!(acquired["kind"], "done");
        let emitted = call("close", token, 1);
        assert_eq!(emitted["kind"], "close-modal");
        // A second close while the pair still stands is wait, not a verb.
        let again = call("close", token, 1);
        assert_eq!(again["kind"], "wait");
        post_tab(11, -1, &[], vec![cook_row(), waterfall_row()]);
        let closed = call("close", token, 1);
        assert_eq!(closed["kind"], "done");
        assert_eq!(closed["as_of_sequence"], 8);
        assert_eq!(closed["closed_as_of_sequence"], 11);
    }

    #[test]
    fn the_posted_pair_is_read_from_the_scene() {
        reset_closed();
        let token = begin("Cook's Assistant", 1)["token"].as_u64().unwrap();
        post_tab(
            8,
            77,
            &["@dre@The Cook's Quest"],
            vec![cook_row(), waterfall_row()],
        );
        let acquired = call("next", token, 1);
        assert_eq!(acquired["kind"], "done", "{acquired}");
        assert_eq!(acquired["root"], 77);
        assert_eq!(acquired["lines"], json!(["@dre@The Cook's Quest"]));
        assert_eq!(acquired["as_of_sequence"], 8);
    }

    #[test]
    fn a_refused_begin_does_not_abort_a_live_token() {
        reset_closed();
        let token = begin("Cook's Assistant", 1)["token"].as_u64().unwrap();
        post_tab(
            8,
            77,
            &["@dre@The Cook's Quest"],
            vec![cook_row(), waterfall_row()],
        );
        assert_eq!(call("next", token, 1)["kind"], "done");
        let unknown = begin("Missing Quest", 1);
        assert_eq!(unknown["kind"], "aborted");
        assert_eq!(unknown["reason"], "unknown-quest");
        let closing = call("close", token, 1);
        assert_eq!(closing["kind"], "close-modal", "{closing}");
    }
    #[test]
    fn close_before_acquisition_aborts_without_a_close_verb() {
        reset_closed();
        let token = begin("Cook's Assistant", 1)["token"].as_u64().unwrap();
        let early = call("close", token, 1);
        assert_eq!(early["kind"], "aborted", "{early}");
        assert_eq!(early["reason"], "stale", "{early}");
        let dead = call("next", token, 1);
        assert_eq!(dead["kind"], "aborted", "{dead}");
        assert_eq!(dead["reason"], "stale", "{dead}");
    }

    #[test]
    fn a_replacement_root_is_stale_and_is_never_closed() {
        reset_closed();
        let token = begin("Cook's Assistant", 1)["token"].as_u64().unwrap();
        post_tab(
            8,
            77,
            &["@dre@The Cook's Quest"],
            vec![cook_row(), waterfall_row()],
        );
        assert_eq!(call("next", token, 1)["kind"], "done");
        assert_eq!(call("close", token, 1)["kind"], "close-modal");

        post_tab(
            9,
            3824,
            &["@dre@Some Other Quest"],
            vec![cook_row(), waterfall_row()],
        );
        let replaced = call("close", token, 1);
        assert_eq!(replaced["kind"], "aborted", "{replaced}");
        assert_eq!(replaced["reason"], "stale", "{replaced}");
        assert_eq!(call("next", token, 1)["reason"], "stale");
    }

    #[test]
    fn a_wrong_token_on_an_unusable_pair_kills_the_live_token_as_stale() {
        reset_closed();
        let token = begin("Cook's Assistant", 1)["token"].as_u64().unwrap();
        post_tab(
            8,
            -1,
            &["orphaned modal text"],
            vec![cook_row(), waterfall_row()],
        );
        let wrong = call("next", token + 5, 1);
        assert_eq!(wrong["kind"], "aborted", "{wrong}");
        assert_eq!(wrong["reason"], "stale", "{wrong}");
        assert_eq!(call("close", token + 5, 1)["reason"], "stale");
        assert_eq!(call("next", token, 1)["reason"], "stale");

        post_tab(
            9,
            77,
            &["@dre@The Cook's Quest"],
            vec![cook_row(), waterfall_row()],
        );
        assert_eq!(call("next", token, 1)["reason"], "stale");
    }
    #[test]
    fn compat_quiet_lease_is_keyed_and_wall_bounded() {
        let lease = CompatJournalLease::default();
        let started = std::time::Instant::now();

        lease.set_generation(9);
        lease.set_host_generation(9);
        lease.acquire(4, 9, started);
        assert!(lease.live(9, started + std::time::Duration::from_secs(9)));

        // A stale producer cannot clear a replacement token in the same
        // work generation.
        lease.acquire(5, 9, started);
        lease.clear(4, 9);
        assert!(lease.live(9, started + std::time::Duration::from_secs(1)));

        // The generation fence revokes a lease as soon as the host advances
        // the isolate's work, even if the old producer runs afterward.
        assert!(!lease.live(10, started + std::time::Duration::from_secs(1)));

        lease.set_generation(10);
        lease.set_host_generation(10);
        lease.acquire(6, 10, started);
        assert!(!lease.live(10, started + std::time::Duration::from_secs(10)));
    }

    #[test]
    fn compat_quiet_lease_tracks_close_cancel_reset_and_timeout() {
        reset_closed();
        let lease = std::sync::Arc::new(CompatJournalLease::default());
        lease.set_generation(7);
        lease.set_host_generation(7);
        bind_compat_runtime(std::sync::Arc::clone(&lease));

        let first = begin("Cook's Assistant", 0)["token"].as_u64().unwrap();
        assert!(
            lease.live(7, std::time::Instant::now()),
            "the lease is armed before the first if-button"
        );
        post_tab(
            8,
            77,
            &["@dre@The Cook's Quest"],
            vec![cook_row(), waterfall_row()],
        );
        assert_eq!(call("next", first, 7)["kind"], "done");
        assert_eq!(call("close", first, 7)["kind"], "close-modal");
        assert!(
            lease.live(7, std::time::Instant::now()),
            "the lease remains live until the close is observed"
        );
        post_tab(9, -1, &[], vec![cook_row(), waterfall_row()]);
        assert_eq!(call("close", first, 7)["kind"], "done");
        assert!(!lease.live(7, std::time::Instant::now()));

        // A replacement begin cancels the old producer and installs a new
        // key. Its stale cleanup cannot clear that replacement.
        let old = begin("Cook's Assistant", 0)["token"].as_u64().unwrap();
        let replacement = begin("Waterfall Quest", 0)["token"].as_u64().unwrap();
        assert_ne!(old, replacement);
        lease.clear(old, 7);
        assert!(lease.live(7, std::time::Instant::now()));

        on_reset();
        assert!(!lease.live(7, std::time::Instant::now()));

        let timed_out = begin("Cook's Assistant", 0)["token"].as_u64().unwrap();
        RUNTIME.with(|rt| {
            rt.borrow_mut().clock.deadline =
                Some(std::time::Instant::now() - std::time::Duration::from_millis(1));
        });
        assert_eq!(call("next", timed_out, 7)["reason"], "modal-timeout");
        let wall_expired = begin("Cook's Assistant", 0)["token"].as_u64().unwrap();
        let wall_started = std::time::Instant::now();
        assert!(lease.live(7, wall_started));
        assert!(!lease.live(7, wall_started + std::time::Duration::from_secs(10),));
        assert_eq!(call("next", wall_expired, 7)["reason"], "modal-timeout");

        let stopped = begin("Cook's Assistant", 0)["token"].as_u64().unwrap();
        assert!(lease.live(7, std::time::Instant::now()));
        on_stop();
        assert!(!lease.live(7, std::time::Instant::now()));
        let _ = stopped;
    }
}
