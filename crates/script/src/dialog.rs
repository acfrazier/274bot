//! Rust-owned chat-dialogue sequencing for `openDialogue` / `driveDialog` /
//! `talkThrough`, the `dialog` [`crate::machine`] family.
//!
//! JavaScript passes the NPC name, preferred fragments, optional gap and the
//! caller's `log`, and awaits the bool. Matching, the 120-step cap,
//! open/gap bounds, continue-vs-choice, pause-latch quiet, pending abort,
//! the frozen log lines and completion stay here. Game actions reuse the
//! existing FlatBuffer `npc` / `continue` / `answer` verbs.

use crate::machine::{Begin, Call, Cx, Family, Reply, Step};
use crate::observed::{self, Ops, Scene, Text};
use crate::shim::InteractReq;
use serde::Deserialize;
use serde_json::json;

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

#[derive(Debug, Clone, PartialEq, Eq)]
struct Npc {
    name: Text,
    actions: Ops,
    distance: i32,
    index: i32,
}

/// The posted facts this module decides from, read from the isolate scene.
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
    /// A logout forgets the session: only pages posted since login count.
    /// The tick is always the last posted one.
    fn from_scene(scene: &Scene) -> Self {
        let session = scene.since_login();
        Self {
            ingame: session.ingame().unwrap_or(false),
            tick: scene.tick().unwrap_or(0),
            hold: session.hold().unwrap_or(false),
            ours: session.ours().unwrap_or(false),
            chat_modal_id: session.chat_modal_id().unwrap_or(-1),
            chat_continue: session.chat_continue().unwrap_or(false),
            // Empty texts stay: the 1-based Answer index is the posted slot.
            chat_options: session.chat_options().cloned().unwrap_or_default(),
            bank_open: session.bank_open().unwrap_or(false),
            npcs: session
                .npcs()
                .map(|rows| {
                    rows.iter()
                        // Shared with the scene: no per-call string copies.
                        // `talk_op` never matches an empty or `hidden` slot.
                        .map(|npc| Npc {
                            name: npc.name.clone().unwrap_or_default(),
                            actions: npc.actions.clone(),
                            distance: npc.distance,
                            index: npc.index,
                        })
                        .collect()
                })
                .unwrap_or_default(),
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

/// The caller's `log` hook.
const LOG: usize = 0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Kind {
    Open,
    Drive,
    Talk,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    WaitOpen,
    Drive,
    WaitContinueAck,
    WaitContinueTick,
    WaitChoiceAck,
    WaitChoiceTicks,
    WaitGap,
}

/// What a step does once its log line was written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum After {
    Wait,
    Done(bool),
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DialogArgs {
    kind: Kind,
    #[serde(default)]
    npc: String,
    #[serde(default)]
    prefer: Vec<String>,
    /// A whole non-negative number; anything else keeps the frozen 1.5 s.
    #[serde(default)]
    gap_ms: serde_json::Value,
}

/// One frozen `openDialogue` / `driveDialog` / `talkThrough`.
pub(crate) struct Dialog {
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
    /// [`crate::reach::pending_posts`] when the dialogue began.
    pending_mark: u64,
    /// A log line to write before `after`.
    log: Option<String>,
    after: Option<After>,
}

impl Family for Dialog {
    const NAME: &'static str = "dialog";
    /// One dialogue at a time: a new one replaces the one in flight.
    const EXCLUSIVE: bool = true;
    const CALLBACKS: &'static [&'static str] = &["log"];
    /// A begin that settles with a log line writes it in the caller's tick.
    const KICK_ON_START: bool = true;
    type Args = DialogArgs;
    type Output = bool;

    fn begin(args: DialogArgs, cx: &mut Cx<'_>) -> Begin<Self> {
        let obs = observed::with(NativeObservation::from_scene);
        if !obs.ingame || obs.pending() {
            return Begin::Done(false);
        }
        let mut dialog = Self {
            phase: Phase::Drive,
            kind: args.kind,
            npc_name: args.npc.trim().to_string(),
            npc_action: String::new(),
            npc_index: -1,
            prefer: args.prefer,
            gap_ms: args
                .gap_ms
                .as_u64()
                .or_else(|| {
                    args.gap_ms
                        .as_f64()
                        .filter(|n| *n >= 0.0 && n.fract() == 0.0)
                        .map(|n| n as u64)
                })
                .unwrap_or(DIALOG_GAP_MS),
            steps: 0,
            due_tick: 0,
            ack_modal_id: -1,
            pending_mark: crate::reach::pending_posts(),
            log: None,
            after: None,
        };
        match dialog.start(&obs, cx) {
            // A queued log line is written by the first step (the start kick).
            Step::Done(ok) if dialog.log.is_none() => Begin::Done(ok),
            _ => Begin::Run(dialog),
        }
    }

    fn step(&mut self, cx: &mut Cx<'_>) -> Step<bool> {
        if let Some(Reply::Threw(thrown)) = cx.reply() {
            return Step::Fail(thrown);
        }
        if let Some(line) = self.log.take() {
            return Step::Call(Call {
                hook: LOG,
                args: vec![json!(line)],
            });
        }
        if let Some(after) = self.after.take() {
            return match after {
                After::Wait => Step::Wait,
                After::Done(ok) => Step::Done(ok),
            };
        }
        let obs = observed::with(NativeObservation::from_scene);
        if !obs.ingame || crate::reach::pending_posts() != self.pending_mark || obs.pending() {
            return Step::Done(false);
        }
        let step = match self.phase {
            Phase::WaitOpen => self.wait_open(&obs, cx),
            Phase::WaitContinueAck => self.wait_continue_ack(&obs, cx),
            Phase::WaitChoiceAck => self.wait_choice_ack(&obs, cx),
            Phase::WaitContinueTick | Phase::WaitChoiceTicks => {
                if obs.tick >= self.due_tick {
                    self.phase = Phase::Drive;
                    self.drive_step(&obs, cx)
                } else {
                    Step::Wait
                }
            }
            Phase::WaitGap => self.wait_gap(&obs, cx),
            Phase::Drive => self.drive_step(&obs, cx),
        };
        match self.log.take() {
            Some(line) => Step::Call(Call {
                hook: LOG,
                args: vec![json!(line)],
            }),
            None => step,
        }
    }
}

impl Dialog {
    /// Queue `line` for the caller's `log` (when it gave one), then
    /// `after`; without a line this is `after` itself.
    fn then(&mut self, line: Option<String>, after: After, cx: &Cx<'_>) -> Step<bool> {
        if let Some(line) = line.filter(|_| cx.has(LOG)) {
            self.log = Some(line);
            self.after = Some(after);
            return Step::Wait;
        }
        match after {
            After::Wait => Step::Wait,
            After::Done(ok) => Step::Done(ok),
        }
    }

    fn start(&mut self, obs: &NativeObservation, cx: &mut Cx<'_>) -> Step<bool> {
        match self.kind {
            Kind::Drive => self.drive_step(obs, cx),
            Kind::Open | Kind::Talk => {
                if obs.dialog_ready() {
                    if self.kind == Kind::Open {
                        return Step::Done(true);
                    }
                    return self.drive_step(obs, cx);
                }
                if obs.bank_open {
                    return Step::Done(self.kind == Kind::Talk);
                }
                let Some(npc) = talk_target(&obs.npcs, &self.npc_name) else {
                    let line = format!("no '{}' nearby to talk to", self.npc_name);
                    return self.then(Some(line), After::Done(false), cx);
                };
                self.npc_name = npc.name;
                self.npc_action = npc.action;
                self.npc_index = npc.index;
                self.phase = Phase::WaitOpen;
                cx.clock().arm(DIALOGUE_OPEN_MS);
                cx.emit(InteractReq::Npc {
                    name: self.npc_name.clone(),
                    action: self.npc_action.clone(),
                    index: Some(self.npc_index),
                });
                Step::Wait
            }
        }
    }

    fn wait_open(&mut self, obs: &NativeObservation, cx: &mut Cx<'_>) -> Step<bool> {
        if obs.dialog_ready() {
            if self.kind == Kind::Open {
                return Step::Done(true);
            }
            self.phase = Phase::Drive;
            return self.drive_step(obs, cx);
        }
        if obs.bank_open {
            return Step::Done(self.kind == Kind::Talk);
        }
        if cx.clock().bound_reached() {
            let line = format!("'{}' never opened a dialogue", self.npc_name);
            return self.then(Some(line), After::Done(false), cx);
        }
        Step::Wait
    }

    fn continue_acked(&self, obs: &NativeObservation) -> bool {
        obs.chat_modal_id != self.ack_modal_id || !obs.chat_continue
    }

    fn choice_acked(&self, obs: &NativeObservation) -> bool {
        obs.chat_modal_id != self.ack_modal_id || obs.chat_continue
    }

    fn wait_continue_ack(&mut self, obs: &NativeObservation, cx: &mut Cx<'_>) -> Step<bool> {
        if self.continue_acked(obs) {
            self.phase = Phase::WaitContinueTick;
            self.due_tick = obs.tick.saturating_add(CONTINUE_TICKS);
            cx.clock().deadline = None;
            return Step::Wait;
        }
        if cx.clock().bound_reached() {
            return Step::Done(false);
        }
        Step::Wait
    }

    fn wait_choice_ack(&mut self, obs: &NativeObservation, cx: &mut Cx<'_>) -> Step<bool> {
        if self.choice_acked(obs) {
            self.phase = Phase::WaitChoiceTicks;
            self.due_tick = obs.tick.saturating_add(CHOICE_TICKS);
            cx.clock().deadline = None;
            return Step::Wait;
        }
        if cx.clock().bound_reached() {
            return Step::Done(false);
        }
        Step::Wait
    }

    fn wait_gap(&mut self, obs: &NativeObservation, cx: &mut Cx<'_>) -> Step<bool> {
        if obs.dialog_ready() {
            self.phase = Phase::Drive;
            cx.clock().deadline = None;
            return self.drive_step(obs, cx);
        }
        if obs.bank_open || cx.clock().bound_reached() {
            return Step::Done(!obs.is_open());
        }
        Step::Wait
    }

    fn drive_step(&mut self, obs: &NativeObservation, cx: &mut Cx<'_>) -> Step<bool> {
        self.phase = Phase::Drive;
        if self.steps >= DRIVE_STEPS {
            return Step::Done(!obs.is_open());
        }
        if !obs.dialog_ready() {
            if obs.bank_open {
                return Step::Done(true);
            }
            self.phase = Phase::WaitGap;
            cx.clock().arm(self.gap_ms);
            return if cx.clock().bound_reached() {
                Step::Done(!obs.is_open())
            } else {
                Step::Wait
            };
        }
        if obs.chat_continue {
            self.steps += 1;
            self.ack_modal_id = obs.chat_modal_id;
            self.phase = Phase::WaitContinueAck;
            cx.clock().arm(PAGE_ACK_MS);
            cx.emit(InteractReq::ContinueDialog);
            return Step::Wait;
        }
        if !obs.options().is_empty() {
            let (option, line) = choose_option(obs.options(), &self.prefer);
            self.steps += 1;
            self.ack_modal_id = obs.chat_modal_id;
            self.phase = Phase::WaitChoiceAck;
            cx.clock().arm(PAGE_ACK_MS);
            cx.emit(InteractReq::Answer { option });
            return self.then(line, After::Wait, cx);
        }
        // Chat is up but the continue id is hidden (pause latch) and no
        // choices are posted: stay quiet one tick, never Talk-to or re-press.
        self.steps += 1;
        self.phase = Phase::WaitContinueTick;
        self.due_tick = obs.tick.saturating_add(CONTINUE_TICKS);
        cx.clock().deadline = None;
        Step::Wait
    }
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
                    name: npc.name.to_string(),
                    action: action.to_string(),
                    index: npc.index,
                },
            ))
        })
        .min_by_key(|(distance, _)| *distance)
        .map(|(_, target)| target)
}

fn talk_op(actions: &[Text]) -> Option<&str> {
    actions.iter().find_map(|action| {
        action
            .get(..4)
            .is_some_and(|head| head.eq_ignore_ascii_case("talk"))
            .then_some(&**action)
    })
}

/// Frozen `pickPreferred`: the first fragment (in order) that some option
/// contains, case-insensitively; a matched empty option is no pick.
pub(crate) fn pick_preferred(options: &[String], prefer: &[String]) -> Option<usize> {
    prefer.iter().find_map(|fragment| {
        let want = fragment.to_lowercase();
        options
            .iter()
            .position(|option| option.to_lowercase().contains(&want))
            .filter(|&index| !options[index].is_empty())
    })
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::machine::{self, AbortReason, Called, Handle, Outcome, Pending, Started, Take};

    fn npc(name: &str, actions: &[&str], distance: i32, index: i32) -> Npc {
        Npc {
            name: name.into(),
            actions: actions.iter().map(|action| Text::from(*action)).collect(),
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

    /// Stand in for a decoded post carrying `obs`.
    fn post(obs: &NativeObservation) {
        observed::post(obs.tick, |post| {
            post.session(obs.ingame)
                .hold(obs.hold)
                .ours(obs.ours)
                .chat_modal_id(obs.chat_modal_id)
                .chat_continue(obs.chat_continue)
                .chat_options(obs.chat_options.clone())
                .bank_open(obs.bank_open)
                .npcs(
                    obs.npcs
                        .iter()
                        .map(|npc| observed::EntityRow {
                            index: npc.index,
                            name: Some(npc.name.clone()),
                            distance: npc.distance,
                            actions: npc.actions.clone(),
                            ..observed::EntityRow::default()
                        })
                        .collect(),
                );
        });
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
        assert_eq!(
            pick_preferred(&opts, &["".into()]),
            Some(0),
            "frozen: an empty fragment is contained by every option"
        );
        assert_eq!(
            pick_preferred(&with_empty, &["".into(), "BANK".into()]),
            Some(1),
            "a matched empty option is no pick: the next fragment decides"
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

    /// No script callbacks: no `log` hook is held here.
    struct NoJs;

    impl machine::Js for NoJs {
        fn queue_len(&mut self) -> usize {
            0
        }

        fn call(
            &mut self,
            _hook: Option<&crate::load::callback_v8::HeldCallback>,
            _args: &[serde_json::Value],
        ) -> Called {
            panic!("no log hook is held");
        }

        fn poll(&mut self, _pending: &Pending) -> Option<Reply> {
            panic!("no log hook is held");
        }

        fn claimed(&mut self) -> bool {
            false
        }
    }

    fn start(kind: &str) -> Started {
        machine::start(
            "dialog",
            json!({ "kind": kind, "npc": "Gundai", "prefer": ["access my bank"] }),
            Vec::new(),
            0,
        )
    }

    fn running(started: Started) -> Handle {
        match started {
            Started::Running(handle) => handle,
            other => panic!("expected a running dialogue, got {other:?}"),
        }
    }

    fn tick() -> Vec<InteractReq> {
        machine::step(&mut NoJs);
        machine::merge_ops(Vec::new())
    }

    fn ops() -> Vec<InteractReq> {
        machine::merge_ops(Vec::new())
    }

    #[test]
    fn pending_and_reset_fail_closed_without_another_talk() {
        let mut observation = obs(vec![npc("Gundai", &["Talk-to"], 1, 3)]);
        observation.ours = true;
        post(&observation);
        assert_eq!(
            start("talk"),
            Started::Settled(Outcome::Done(json!(false))),
            "a pending interrupt refuses to talk"
        );
        assert!(ops().is_empty());

        observation.ours = false;
        post(&observation);
        let talk = running(start("talk"));
        assert_eq!(
            ops(),
            vec![InteractReq::Npc {
                name: "Gundai".into(),
                action: "Talk-to".into(),
                index: Some(3),
            }]
        );
        observation.ours = true;
        post(&observation);
        assert!(tick().is_empty());
        assert_eq!(
            machine::take(talk),
            Take::Settled(Outcome::Done(json!(false)))
        );

        observation.ours = false;
        post(&observation);
        let again = running(start("talk"));
        ops();
        machine::on_reset();
        assert!(tick().is_empty());
        assert_eq!(
            machine::take(again),
            Take::Settled(Outcome::Aborted(AbortReason::Reset))
        );
    }

    #[test]
    fn pause_does_not_continue_or_choose() {
        // Chat up with the continue id hidden: the drive stays quiet a tick.
        let mut observation = obs(vec![npc("Gundai", &["Talk-to"], 1, 3)]);
        observation.chat_modal_id = 968;
        post(&observation);
        let drive = running(start("drive"));
        assert!(ops().is_empty());
        machine::on_pause();
        observation.tick = 2;
        observation.chat_continue = true;
        post(&observation);
        assert!(
            tick().is_empty(),
            "a paused drive neither continues nor chooses"
        );
        machine::on_resume();
        assert_eq!(tick(), vec![InteractReq::ContinueDialog]);
        assert_eq!(machine::take(drive), Take::Pending);
    }

    #[test]
    fn same_continue_page_does_not_re_emit_and_ack_timeout_fails() {
        let mut observation = obs(vec![npc("Gundai", &["Talk-to"], 1, 3)]);
        observation.chat_modal_id = 968;
        observation.chat_continue = true;
        post(&observation);
        let drive = running(start("drive"));
        assert_eq!(ops(), vec![InteractReq::ContinueDialog]);
        observation.tick = 5;
        post(&observation);
        assert!(tick().is_empty(), "the same page is not continued twice");
        assert_eq!(machine::take(drive), Take::Pending);
        machine::tests::expire_deadlines();
        assert!(tick().is_empty());
        assert_eq!(
            machine::take(drive),
            Take::Settled(Outcome::Done(json!(false)))
        );
    }
}
