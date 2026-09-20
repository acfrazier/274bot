//! Rust-owned chat-dialogue sequencing for `openDialogue` / `driveDialog` /
//! `talkThrough`.
//!
//! JavaScript marshals the NPC name, preferred fragments and optional gap,
//! dispatches the returned verbs and reports the bool. Matching, the 120-step
//! cap, open/gap bounds, continue-vs-choice, pause-latch quiet, pending abort
//! and completion stay here. Game actions reuse the existing FlatBuffer
//! `npc` / `continue` / `answer` verbs.

use crate::isolate_fb::SnapshotReader;
use crate::task_clock::InstantTaskClock;
use serde_json::{json, Value};
use std::cell::RefCell;

/// Frozen page-turn quiet. A scene that walks an NPC about wants its caller
/// to name more via `gapMs`.
pub const DIALOG_GAP_MS: u64 = 1_500;
/// Frozen `openDialogue` wait for the first chat page.
pub const DIALOGUE_OPEN_MS: u64 = 8_000;
/// Frozen drive loop bound (`for (let i = 0; i < 120; i++)`).
pub const DRIVE_STEPS: u32 = 120;
/// Frozen `ChatDialog.continue` / `chooseOption` observed-ack wait.
pub const PAGE_ACK_MS: u64 = 3_000;
/// Frozen `delayTicks(1)` after a continue ack.
pub const CONTINUE_TICKS: u64 = 1;
/// Frozen `delayTicks(2)` after a choice ack.
pub const CHOICE_TICKS: u64 = 2;

thread_local! {
    static RUNTIME: RefCell<DialogRuntime> = const { RefCell::new(DialogRuntime::new()) };
    static NATIVE_OBSERVATION: RefCell<NativeObservation> =
        const { RefCell::new(NativeObservation::new()) };
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Npc {
    name: String,
    actions: Vec<String>,
    distance: i32,
    index: i32,
}

#[derive(Clone)]
struct NativeObservation {
    ingame: bool,
    tick: u64,
    hold: bool,
    ours: bool,
    chat_modal_id: i32,
    chat_continue: bool,
    chat_options: Vec<String>,
    bank_open: bool,
    npcs: Vec<Npc>,
}

impl NativeObservation {
    const fn new() -> Self {
        Self {
            ingame: false,
            tick: 0,
            hold: false,
            ours: false,
            chat_modal_id: -1,
            chat_continue: false,
            chat_options: Vec::new(),
            bank_open: false,
            npcs: Vec::new(),
        }
    }

    fn update(&mut self, snap: &SnapshotReader<'_>) {
        self.tick = snap.tick();
        if snap.has_ingame() {
            if !snap.ingame() {
                *self = Self::new();
                self.tick = snap.tick();
                return;
            }
            self.ingame = true;
        }
        if snap.has_hold() {
            self.hold = snap.hold();
        }
        if snap.has_ours() {
            self.ours = snap.ours();
        }
        if snap.has_chat_modal_id() {
            self.chat_modal_id = snap.chat_modal_id();
        }
        if snap.has_chat_continue() {
            self.chat_continue = snap.chat_continue();
        }
        if snap.has_chat_options() {
            // Keep empty texts: the 1-based Answer index is the posted slot.
            self.chat_options = snap
                .chat_options()
                .iter()
                .map(|row| row.text().to_string())
                .collect();
        }
        if snap.has_bank_open() {
            self.bank_open = snap.bank_open();
        }
        if snap.has_npcs() {
            self.npcs = snap
                .npcs()
                .iter()
                .map(|npc| Npc {
                    name: npc.name().unwrap_or_default().to_string(),
                    actions: npc
                        .actions()
                        .iter()
                        .filter(|action| !action.is_empty() && **action != "hidden")
                        .map(|action| (*action).to_string())
                        .collect(),
                    distance: npc.distance(),
                    index: npc.index(),
                })
                .collect();
        }
    }

    fn pending(&self) -> bool {
        self.hold || self.ours
    }

    fn is_open(&self) -> bool {
        self.chat_modal_id != -1
    }

    fn dialog_ready(&self) -> bool {
        self.is_open() || self.chat_continue
    }

    fn options(&self) -> &[String] {
        &self.chat_options
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Open,
    Drive,
    Talk,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Idle,
    WaitOpen,
    Drive,
    WaitContinueAck,
    WaitContinueTick,
    WaitChoiceAck,
    WaitChoiceTicks,
    WaitGap,
}

struct DialogRuntime {
    clock: InstantTaskClock,
    token: u64,
    phase: Phase,
    kind: Kind,
    npc_name: String,
    npc_action: String,
    npc_index: i32,
    prefer: Vec<String>,
    gap_ms: u64,
    steps: u32,
    due_tick: u64,
    ack_modal_id: i32,
    interrupted: bool,
}

impl DialogRuntime {
    const fn new() -> Self {
        Self {
            clock: InstantTaskClock::new(),
            token: 0,
            phase: Phase::Idle,
            kind: Kind::Drive,
            npc_name: String::new(),
            npc_action: String::new(),
            npc_index: -1,
            prefer: Vec::new(),
            gap_ms: DIALOG_GAP_MS,
            steps: 0,
            due_tick: 0,
            ack_modal_id: -1,
            interrupted: false,
        }
    }

    fn frozen(&self) -> bool {
        self.clock.frozen()
    }

    fn set_freeze(&mut self, paused: bool, held: bool) {
        self.clock.set_freeze(paused, held);
    }

    fn arm(&mut self, window: u64) {
        self.clock.arm(window);
    }

    fn bound_reached(&self) -> bool {
        self.clock.bound_reached()
    }

    fn abort_runtime(&mut self) {
        self.token = self.token.wrapping_add(1);
        self.phase = Phase::Idle;
        self.npc_name.clear();
        self.npc_action.clear();
        self.npc_index = -1;
        self.prefer.clear();
        self.gap_ms = DIALOG_GAP_MS;
        self.steps = 0;
        self.due_tick = 0;
        self.ack_modal_id = -1;
        self.interrupted = false;
        self.clock.deadline = None;
    }

    fn done(&mut self, result: bool, reason: &str, log: Option<String>) -> Value {
        let token = self.token;
        self.phase = Phase::Idle;
        self.clock.deadline = None;
        with_log(
            json!({
                "kind": "done",
                "token": token,
                "result": result,
                "reason": reason,
            }),
            log,
        )
    }

    fn wait(&self) -> Value {
        json!({ "kind": "wait", "token": self.token })
    }

    fn npc_verb(&self) -> Value {
        json!({
            "kind": "npc",
            "token": self.token,
            "name": self.npc_name,
            "action": self.npc_action,
            "index": self.npc_index,
        })
    }

    fn continue_verb(&self) -> Value {
        json!({
            "kind": "ops",
            "token": self.token,
            "ops": [{ "op": "continue" }],
        })
    }

    fn answer_verb(&self, option: i32, log: Option<String>) -> Value {
        with_log(
            json!({
                "kind": "ops",
                "token": self.token,
                "ops": [{ "op": "answer", "option": option }],
            }),
            log,
        )
    }
}

pub fn on_snapshot(snap: &SnapshotReader<'_>) {
    NATIVE_OBSERVATION.with(|obs| {
        let mut obs = obs.borrow_mut();
        obs.update(snap);
        RUNTIME.with(|rt| {
            let mut rt = rt.borrow_mut();
            if rt.phase != Phase::Idle && obs.pending() {
                rt.interrupted = true;
            }
        });
    });
}

pub fn on_pause() {
    RUNTIME.with(|rt| {
        let held = rt.borrow().clock.held;
        rt.borrow_mut().set_freeze(true, held);
    });
}

pub fn on_resume() {
    RUNTIME.with(|rt| {
        let held = rt.borrow().clock.held;
        rt.borrow_mut().set_freeze(false, held);
    });
}

pub fn on_hold(held: bool) {
    RUNTIME.with(|rt| {
        let paused = rt.borrow().clock.paused;
        rt.borrow_mut().set_freeze(paused, held);
    });
}

pub fn on_reset() {
    RUNTIME.with(|rt| rt.borrow_mut().abort_runtime());
    NATIVE_OBSERVATION.with(|obs| *obs.borrow_mut() = NativeObservation::new());
}

pub fn dispatch(input: &Value) -> Value {
    match input.get("op").and_then(Value::as_str).unwrap_or("") {
        "begin" => begin(input),
        "next" => next(input.get("token").and_then(Value::as_u64).unwrap_or(0)),
        _ => json!({ "kind": "notImpl", "reason": "unknown op" }),
    }
}

fn begin(input: &Value) -> Value {
    let kind = match input.get("kind").and_then(Value::as_str).unwrap_or("") {
        "open" => Kind::Open,
        "drive" => Kind::Drive,
        "talk" => Kind::Talk,
        _ => return json!({ "kind": "notImpl", "reason": "unknown dialog op" }),
    };
    let npc_name = input
        .get("npc")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    let prefer = read_prefer(input.get("prefer"));
    let gap_ms = input
        .get("gapMs")
        .and_then(Value::as_u64)
        .unwrap_or(DIALOG_GAP_MS);
    NATIVE_OBSERVATION.with(|o| {
        let obs = o.borrow();
        RUNTIME.with(|rt| {
            let mut rt = rt.borrow_mut();
            rt.abort_runtime();
            rt.kind = kind;
            rt.npc_name = npc_name;
            rt.prefer = prefer;
            rt.gap_ms = gap_ms;
            if !obs.ingame {
                return json!({ "kind": "aborted", "reason": "not ingame" });
            }
            if obs.pending() {
                rt.interrupted = true;
                return rt.done(false, "pending", None);
            }
            start(&mut rt, &obs)
        })
    })
}

fn start(rt: &mut DialogRuntime, obs: &NativeObservation) -> Value {
    match rt.kind {
        Kind::Drive => {
            rt.phase = Phase::Drive;
            drive_step(rt, obs)
        }
        Kind::Open | Kind::Talk => {
            if obs.dialog_ready() {
                if rt.kind == Kind::Open {
                    return rt.done(true, "already-open", None);
                }
                rt.phase = Phase::Drive;
                return drive_step(rt, obs);
            }
            if obs.bank_open {
                return rt.done(rt.kind == Kind::Talk, "bank-open", None);
            }
            let Some(npc) = talk_target(&obs.npcs, &rt.npc_name) else {
                let name = rt.npc_name.clone();
                return rt.done(
                    false,
                    "no-npc",
                    Some(format!("no '{name}' nearby to talk to")),
                );
            };
            rt.npc_name = npc.name;
            rt.npc_action = npc.action;
            rt.npc_index = npc.index;
            rt.phase = Phase::WaitOpen;
            rt.arm(DIALOGUE_OPEN_MS);
            rt.npc_verb()
        }
    }
}

fn next(token: u64) -> Value {
    NATIVE_OBSERVATION.with(|o| {
        let obs = o.borrow();
        RUNTIME.with(|rt| {
            let mut rt = rt.borrow_mut();
            if token != rt.token || rt.phase == Phase::Idle {
                return json!({ "kind": "aborted", "token": rt.token });
            }
            if rt.frozen() {
                return rt.wait();
            }
            if !obs.ingame {
                return json!({ "kind": "aborted", "token": rt.token });
            }
            if rt.interrupted || obs.pending() {
                return rt.done(false, "pending", None);
            }
            match rt.phase {
                Phase::Idle => json!({ "kind": "aborted", "token": rt.token }),
                Phase::WaitOpen => wait_open(&mut rt, &obs),
                Phase::WaitContinueAck => wait_continue_ack(&mut rt, &obs),
                Phase::WaitChoiceAck => wait_choice_ack(&mut rt, &obs),
                Phase::WaitContinueTick | Phase::WaitChoiceTicks => {
                    if obs.tick >= rt.due_tick {
                        rt.phase = Phase::Drive;
                        drive_step(&mut rt, &obs)
                    } else {
                        rt.wait()
                    }
                }
                Phase::WaitGap => wait_gap(&mut rt, &obs),
                Phase::Drive => drive_step(&mut rt, &obs),
            }
        })
    })
}

fn wait_open(rt: &mut DialogRuntime, obs: &NativeObservation) -> Value {
    if obs.dialog_ready() {
        if rt.kind == Kind::Open {
            return rt.done(true, "opened", None);
        }
        rt.phase = Phase::Drive;
        return drive_step(rt, obs);
    }
    if obs.bank_open {
        return rt.done(rt.kind == Kind::Talk, "bank-open", None);
    }
    if rt.bound_reached() {
        let name = rt.npc_name.clone();
        return rt.done(
            false,
            "open-timeout",
            Some(format!("'{name}' never opened a dialogue")),
        );
    }
    rt.wait()
}

fn continue_acked(rt: &DialogRuntime, obs: &NativeObservation) -> bool {
    obs.chat_modal_id != rt.ack_modal_id || !obs.chat_continue
}

fn choice_acked(rt: &DialogRuntime, obs: &NativeObservation) -> bool {
    obs.chat_modal_id != rt.ack_modal_id || obs.chat_continue
}

fn wait_continue_ack(rt: &mut DialogRuntime, obs: &NativeObservation) -> Value {
    if continue_acked(rt, obs) {
        rt.phase = Phase::WaitContinueTick;
        rt.due_tick = obs.tick.saturating_add(CONTINUE_TICKS);
        rt.clock.deadline = None;
        return rt.wait();
    }
    if rt.bound_reached() {
        return rt.done(false, "continue-ack-timeout", None);
    }
    rt.wait()
}

fn wait_choice_ack(rt: &mut DialogRuntime, obs: &NativeObservation) -> Value {
    if choice_acked(rt, obs) {
        rt.phase = Phase::WaitChoiceTicks;
        rt.due_tick = obs.tick.saturating_add(CHOICE_TICKS);
        rt.clock.deadline = None;
        return rt.wait();
    }
    if rt.bound_reached() {
        return rt.done(false, "choice-ack-timeout", None);
    }
    rt.wait()
}

fn wait_gap(rt: &mut DialogRuntime, obs: &NativeObservation) -> Value {
    if obs.dialog_ready() {
        rt.phase = Phase::Drive;
        rt.clock.deadline = None;
        return drive_step(rt, obs);
    }
    if obs.bank_open || rt.bound_reached() {
        return rt.done(!obs.is_open(), "gap", None);
    }
    rt.wait()
}

fn drive_step(rt: &mut DialogRuntime, obs: &NativeObservation) -> Value {
    if rt.steps >= DRIVE_STEPS {
        return rt.done(!obs.is_open(), "step-cap", None);
    }
    if !obs.dialog_ready() {
        if obs.bank_open {
            return rt.done(true, "bank-open", None);
        }
        rt.phase = Phase::WaitGap;
        rt.arm(rt.gap_ms);
        return if rt.bound_reached() {
            rt.done(!obs.is_open(), "gap", None)
        } else {
            rt.wait()
        };
    }
    if obs.chat_continue {
        rt.steps += 1;
        rt.ack_modal_id = obs.chat_modal_id;
        rt.phase = Phase::WaitContinueAck;
        rt.arm(PAGE_ACK_MS);
        return rt.continue_verb();
    }
    if !obs.options().is_empty() {
        let (option, log) = choose_option(obs.options(), &rt.prefer);
        rt.steps += 1;
        rt.ack_modal_id = obs.chat_modal_id;
        rt.phase = Phase::WaitChoiceAck;
        rt.arm(PAGE_ACK_MS);
        return rt.answer_verb(option, log);
    }
    // Chat is up but the continue id is hidden (pause latch) and no
    // choices are posted: stay quiet one tick, never Talk-to or re-press.
    rt.steps += 1;
    rt.phase = Phase::WaitContinueTick;
    rt.due_tick = obs.tick.saturating_add(CONTINUE_TICKS);
    rt.clock.deadline = None;
    rt.wait()
}

struct TalkTarget {
    name: String,
    action: String,
    index: i32,
}

fn talk_target(npcs: &[Npc], wanted: &str) -> Option<TalkTarget> {
    let want = wanted.trim().to_ascii_lowercase();
    if want.is_empty() {
        return None;
    }
    npcs.iter()
        .filter_map(|npc| {
            if npc.name.trim().to_ascii_lowercase() != want {
                return None;
            }
            let action = talk_op(&npc.actions)?;
            Some((
                npc.distance,
                TalkTarget {
                    name: npc.name.clone(),
                    action: action.to_string(),
                    index: npc.index,
                },
            ))
        })
        .min_by_key(|(distance, _)| *distance)
        .map(|(_, target)| target)
}

fn talk_op(actions: &[String]) -> Option<&str> {
    actions.iter().find_map(|action| {
        action
            .get(..4)
            .is_some_and(|head| head.eq_ignore_ascii_case("talk"))
            .then_some(action.as_str())
    })
}

fn pick_preferred(options: &[String], prefer: &[String]) -> Option<usize> {
    for fragment in prefer {
        let want = fragment.to_ascii_lowercase();
        if want.is_empty() {
            continue;
        }
        if let Some(index) = options
            .iter()
            .position(|option| option.to_ascii_lowercase().contains(&want))
        {
            return Some(index);
        }
    }
    None
}

fn choose_option(options: &[String], prefer: &[String]) -> (i32, Option<String>) {
    if let Some(index) = pick_preferred(options, prefer) {
        return ((index + 1) as i32, None);
    }
    let last = options.len() as i32;
    (
        last,
        Some(format!(
            "WARN: no preferred option in [{}] — taking the last",
            options.join(" | ")
        )),
    )
}

fn read_prefer(value: Option<&Value>) -> Vec<String> {
    match value {
        Some(Value::Array(rows)) => rows
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect(),
        _ => Vec::new(),
    }
}

fn with_log(mut value: Value, log: Option<String>) -> Value {
    if let Some(log) = log {
        value["log"] = json!(log);
    }
    value
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn npc(name: &str, actions: &[&str], distance: i32, index: i32) -> Npc {
        Npc {
            name: name.into(),
            actions: actions.iter().map(|action| (*action).to_string()).collect(),
            distance,
            index,
        }
    }

    fn obs(npcs: Vec<Npc>) -> NativeObservation {
        NativeObservation {
            ingame: true,
            tick: 1,
            hold: false,
            ours: false,
            chat_modal_id: -1,
            chat_continue: false,
            chat_options: Vec::new(),
            bank_open: false,
            npcs,
        }
    }

    #[test]
    fn frozen_bounds_match_the_canonical_primitives() {
        assert_eq!(DIALOG_GAP_MS, 1_500);
        assert_eq!(DIALOGUE_OPEN_MS, 8_000);
        assert_eq!(DRIVE_STEPS, 120);
        assert_eq!(PAGE_ACK_MS, 3_000);
        assert_eq!(CONTINUE_TICKS, 1);
        assert_eq!(CHOICE_TICKS, 2);
    }

    #[test]
    fn talk_op_is_the_first_talk_action_and_skips_hidden() {
        assert_eq!(
            talk_op(&["Examine".into(), "Talk-to".into()]),
            Some("Talk-to")
        );
        assert_eq!(talk_op(&["Talk".into()]), Some("Talk"));
        assert_eq!(talk_op(&["hidden".into(), "Attack".into()]), None);
        assert_eq!(talk_op(&["Bank".into()]), None);
        assert_eq!(
            talk_op(&["abcéxxxx".into()]),
            None,
            "a mid-character byte index must not panic or match"
        );
        assert_eq!(talk_op(&["talké".into()]), Some("talké"));
    }

    #[test]
    fn pick_preferred_is_first_fragment_substring_then_last() {
        let opts = [
            "Hello, what are you doing out here?".into(),
            "I'd like to access my bank account, please.".into(),
        ];
        assert_eq!(pick_preferred(&opts, &["access my bank".into()]), Some(1));
        let with_empty = [
            "".into(),
            "I'd like to access my bank account, please.".into(),
        ];
        assert_eq!(
            choose_option(&with_empty, &["access my bank".into()]).0,
            2,
            "an empty earlier slot keeps the posted 1-based index"
        );
        let (option, log) = choose_option(&opts, &["no such".into()]);
        assert_eq!(option, 2);
        assert!(log.as_deref().unwrap().contains("taking the last"));
    }

    #[test]
    fn nearest_talk_target_wins_and_missing_talk_is_absent() {
        let npcs = vec![
            npc("Gundai", &["Talk-to"], 3, 4),
            npc("Gundai", &["Talk-to"], 1, 9),
            npc("Banker", &["Bank"], 0, 2),
        ];
        let hit = talk_target(&npcs, "gundai").unwrap();
        assert_eq!(hit.index, 9);
        assert_eq!(hit.action, "Talk-to");
        assert!(talk_target(&npcs, "Banker").is_none());
        assert!(talk_target(&npcs, "").is_none());
    }

    #[test]
    fn pending_and_reset_fail_closed_without_another_talk() {
        on_reset();
        let mut observation = obs(vec![npc("Gundai", &["Talk-to"], 1, 3)]);
        observation.ours = true;
        NATIVE_OBSERVATION.with(|slot| *slot.borrow_mut() = observation.clone());
        let pending = dispatch(&json!({
            "op": "begin",
            "kind": "talk",
            "npc": "Gundai",
            "prefer": ["access my bank"],
        }));
        assert_eq!(pending["result"], false);
        assert_eq!(pending["reason"], "pending");

        observation.ours = false;
        NATIVE_OBSERVATION.with(|slot| *slot.borrow_mut() = observation.clone());
        let begin = dispatch(&json!({
            "op": "begin",
            "kind": "talk",
            "npc": "Gundai",
            "prefer": ["access my bank"],
        }));
        assert_eq!(begin["kind"], "npc");
        assert_eq!(begin["action"], "Talk-to");
        let token = begin["token"].as_u64().unwrap();
        RUNTIME.with(|rt| rt.borrow_mut().interrupted = true);
        let stopped = dispatch(&json!({ "op": "next", "token": token }));
        assert_eq!(stopped["kind"], "done");
        assert_eq!(stopped["result"], false);
        assert_eq!(stopped["reason"], "pending");

        NATIVE_OBSERVATION.with(|slot| *slot.borrow_mut() = observation);
        let again = dispatch(&json!({
            "op": "begin",
            "kind": "talk",
            "npc": "Gundai",
            "prefer": ["access my bank"],
        }));
        let stale = again["token"].as_u64().unwrap();
        on_reset();
        assert_eq!(
            dispatch(&json!({ "op": "next", "token": stale }))["kind"],
            "aborted"
        );
    }

    #[test]
    fn pause_does_not_continue_or_choose() {
        let mut observation = obs(vec![npc("Gundai", &["Talk-to"], 1, 3)]);
        observation.chat_modal_id = 968;
        observation.chat_continue = true;
        let mut runtime = DialogRuntime::new();
        runtime.kind = Kind::Talk;
        runtime.npc_name = "Gundai".into();
        runtime.prefer = vec!["access my bank".into()];
        runtime.phase = Phase::Drive;
        runtime.set_freeze(true, false);
        assert_eq!(runtime.wait()["kind"], "wait");
        runtime.set_freeze(false, false);
        let step = drive_step(&mut runtime, &observation);
        assert_eq!(step["kind"], "ops");
        assert_eq!(step["ops"][0]["op"], "continue");
    }

    #[test]
    fn same_continue_page_does_not_re_emit_and_ack_timeout_fails() {
        let mut observation = obs(vec![npc("Gundai", &["Talk-to"], 1, 3)]);
        observation.chat_modal_id = 968;
        observation.chat_continue = true;
        let mut runtime = DialogRuntime::new();
        runtime.kind = Kind::Talk;
        runtime.phase = Phase::Drive;
        let first = drive_step(&mut runtime, &observation);
        assert_eq!(first["ops"][0]["op"], "continue");
        assert_eq!(runtime.phase, Phase::WaitContinueAck);
        observation.tick = 5;
        assert_eq!(
            wait_continue_ack(&mut runtime, &observation)["kind"],
            "wait"
        );
        runtime.clock.deadline =
            Some(runtime.clock.now() - Duration::from_millis(1));
        let timed = wait_continue_ack(&mut runtime, &observation);
        assert_eq!(timed["kind"], "done");
        assert_eq!(timed["result"], false);
        assert_eq!(timed["reason"], "continue-ack-timeout");
    }
}
