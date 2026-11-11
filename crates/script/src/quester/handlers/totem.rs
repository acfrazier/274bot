//! Tribal Totem security-door combination (`x:totem:solve_combination`).
//!
//! Interface layout is content `tribal_door2` (pack id 716). The word itself
//! is Path data so a revision that only changes the answer changes the
//! document. Content echoes each dial letter after every click
//! (`quest_totem.rs2` `tribal_door_combo`: `if_settext(tribal_door2:com_43..46,
//! enum(alphabet, $stage))`).
use crate::interact::InteractReq;
use crate::native::{
    ActionContext, ActionError, ActionHandle, NativeActions, NativeMachine, WalkReceipt,
    WalkRetryState,
};
use crate::quester::compile::{
    CompileContext, CompileError, StepContext, StepOutcome, StepPlan, StepRun,
};
use crate::quester::families::reach;
use crate::quester::prereq::{reach::PrereqReach, walk::PrereqWalk};
use api::WorldTile;
use serde::Deserialize;
use std::sync::Arc;
use std::task::Poll;
use std::time::Duration;

/// Content `pack/interface.pack`: `716=tribal_door2`.
const DOOR_UI: i32 = 716;
/// `tribal_door2:com_43`..`com_46` (pack 760..763).
const DIAL_TEXT: [i32; 4] = [760, 761, 762, 763];
/// Down arrows `com_47,49,51,53` (pack 764,766,768,770).
const DIAL_DOWN: [i32; 4] = [764, 766, 768, 770];
/// Up arrows `com_48,50,52,54` (pack 765,767,769,771).
const DIAL_UP: [i32; 4] = [765, 767, 769, 771];
/// Confirm `com_57` (pack 774). `if_close` runs before the word is tested.
const CONFIRM: i32 = 774;
const LETTERS: i32 = 26;
const CORRECT: &str = "combination seems correct";
const UI_WAIT: Duration = Duration::from_secs(8);
const DIAL_WAIT: Duration = Duration::from_secs(3);
const CONFIRM_WAIT: Duration = Duration::from_secs(8);

#[derive(Deserialize)]
#[cfg_attr(feature = "path-schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub(super) struct AnchorArg {
    tile: [i32; 3],
    #[serde(default)]
    source: String,
}

#[derive(Deserialize)]
#[cfg_attr(feature = "path-schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub(super) struct Args {
    /// Four-letter word the security door accepts (`KURT` on 289).
    combination: String,
    /// Combo-door loc config name (`combodoor`).
    door: String,
    anchor: AnchorArg,
    #[serde(default)]
    radius: i32,
}

pub(super) const STEPS: &[crate::quester::compile::StepHandler] =
    &[crate::quester::compile::step!(
        "x:totem:solve_combination",
        1,
        Explicit,
        Args,
        compile
    )];

pub(super) const FACTS: &[crate::quester::compile::PredicateHandler] = &[];

fn compile(args: Args, cx: &CompileContext<'_>) -> Result<Arc<dyn StepPlan>, CompileError> {
    let word = args.combination.trim().to_ascii_uppercase();
    if word.len() != 4 || !word.chars().all(|c| c.is_ascii_uppercase()) {
        return Err(CompileError::code("invalid-args"));
    }
    if args.anchor.source.trim().is_empty() {
        return Err(CompileError::code("tile-source-required"));
    }
    let tile = args.anchor.tile;
    if !(0..=16383).contains(&tile[0])
        || !(0..=16383).contains(&tile[1])
        || !(0..=3).contains(&tile[2])
    {
        return Err(CompileError::code("invalid-tile"));
    }
    let loc = cx
        .selected
        .loc_by_config(&args.door)
        .ok_or_else(|| CompileError::code("unresolved-loc"))?;
    if !loc.ops.iter().any(|op| op.eq_ignore_ascii_case("Open")) {
        return Err(CompileError::code("unavailable-op"));
    }
    Ok(Arc::new(Plan {
        word: Arc::from(word),
        door_id: loc.id,
        door_name: Arc::from(args.door),
        tile: WorldTile {
            x: tile[0],
            z: tile[1],
            level: tile[2],
        },
        radius: args.radius.max(1),
    }))
}

struct Plan {
    word: Arc<str>,
    door_id: i32,
    door_name: Arc<str>,
    tile: WorldTile,
    radius: i32,
}

impl StepPlan for Plan {
    fn begin(&self, _cx: &mut StepContext<'_, '_>) -> Result<Box<dyn StepRun>, ActionError> {
        Ok(Box::new(Run {
            word: Arc::clone(&self.word),
            door_id: self.door_id,
            door_name: Arc::clone(&self.door_name),
            tile: self.tile,
            radius: self.radius,
            walk: None,
            walk_retry: WalkRetryState::default(),
            reach: None,
            combo: None,
            opened: false,
        }))
    }

    fn settle_timeout(&self) -> Duration {
        Duration::from_secs(12)
    }
}

struct Run {
    word: Arc<str>,
    door_id: i32,
    door_name: Arc<str>,
    tile: WorldTile,
    radius: i32,
    walk: Option<PrereqWalk>,
    walk_retry: WalkRetryState,
    reach: Option<PrereqReach>,
    combo: Option<ActionHandle<Combo>>,
    opened: bool,
}

impl StepRun for Run {
    fn poll(&mut self, cx: &mut StepContext<'_, '_>) -> Poll<Result<StepOutcome, ActionError>> {
        if let Some(walk) = self.walk.as_mut() {
            match walk.poll(cx.tick.actions, &mut cx.tick.cx, cx.tick.state) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(Err(error)) => return Poll::Ready(Err(error)),
                Poll::Ready(Ok(receipt)) => {
                    self.walk = None;
                    if self
                        .walk_retry
                        .observe_approach_failure(&receipt, cx.tick.cx.evidence().tick)
                    {
                        return Poll::Pending;
                    }
                    walk_arrival(receipt)?;
                    self.walk_retry.reset();
                }
            }
        }
        if door_ui_open(&cx.tick.cx) {
            self.opened = true;
        }
        if !self.opened {
            let here = cx.tick.cx.snapshot().here();
            if !here.is_some_and(|obs| reach::within(obs.value, self.tile, self.radius)) {
                let tick = cx.tick.cx.evidence().tick;
                if self.walk_retry.waiting_for_next_tick(tick) {
                    return Poll::Pending;
                }
                let mut request = reach::walk_request(
                    self.tile,
                    self.radius.max(1) as u16,
                    None,
                    cx.required_after,
                );
                // The gate above is the authored anchor radius; any
                // standable tile inside it is close enough to open the door.
                request.arrival = nav_planner::contract::ArrivalKind::Area;
                match crate::native::defer_budget(PrereqWalk::begin(
                    request,
                    cx.tick.actions,
                    &mut cx.tick.cx,
                )) {
                    Poll::Pending => {
                        self.walk_retry.defer_until_next_tick(tick);
                        return Poll::Pending;
                    }
                    Poll::Ready(Err(error)) => return Poll::Ready(Err(error)),
                    Poll::Ready(Ok(handle)) => {
                        self.walk_retry.action_submitted();
                        self.walk = Some(handle);
                    }
                }
                return Poll::Pending;
            }
            self.walk_retry.reset();
            if self.reach.is_none() {
                self.reach = Some(PrereqReach::begin(
                    reach::ReachArgs {
                        kind: reach::ReachKind::Loc {
                            id: Some(self.door_id),
                            name: Some(Arc::clone(&self.door_name)),
                        },
                        op: Arc::from("Open"),
                        anchor: Some(self.tile),
                        radius: self.radius,
                        wait_if_missing: true,
                        target_tile: None,
                        reachable_only: false,
                    },
                    cx.tick.actions,
                    &mut cx.tick.cx,
                )?);
                return Poll::Pending;
            }
        }
        if let Some(handle) = &mut self.reach {
            match handle.poll(cx.tick.actions, &mut cx.tick.cx, cx.tick.state) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(Err(error)) => return Poll::Ready(Err(error)),
                Poll::Ready(Ok(false)) => {
                    return Poll::Ready(Err(ActionError::Failed(Arc::from(
                        "totem security door not reached",
                    ))));
                }
                Poll::Ready(Ok(true)) => {
                    self.reach = None;
                    self.opened = true;
                }
            }
        }
        if self.combo.is_none() {
            self.combo = Some(cx.tick.actions.begin::<Combo>(
                ComboArgs {
                    word: Arc::clone(&self.word),
                },
                &mut cx.tick.cx,
            )?);
            return Poll::Pending;
        }
        match cx
            .tick
            .actions
            .poll(self.combo.as_ref().expect("combo"), &mut cx.tick.cx)
        {
            Poll::Pending => Poll::Pending,
            Poll::Ready(Ok(())) => Poll::Ready(Ok(StepOutcome {
                progress: None,
                evidence: cx.tick.cx.evidence(),
                receipt: None,
            })),
            Poll::Ready(Err(error)) => Poll::Ready(Err(error)),
        }
    }

    fn cancel(&mut self, _actions: &mut NativeActions) {
        if let Some(walk) = self.walk.as_mut() {
            walk.cancel();
        }
        if let Some(reach) = self.reach.as_mut() {
            reach.cancel();
        }
        self.walk = None;
        self.reach = None;
        self.combo = None;
    }
}

struct ComboArgs {
    word: Arc<str>,
}

enum ComboPhase {
    WaitUi,
    Dial {
        index: usize,
        last: Option<char>,
        clicks: u8,
        deadline: Duration,
    },
    Confirm {
        chat_since: i32,
        deadline: Duration,
    },
}

struct Combo {
    word: Arc<str>,
    phase: ComboPhase,
    ui_deadline: Duration,
}

impl NativeMachine for Combo {
    type Args = ComboArgs;
    type Output = ();

    fn begin(args: Self::Args, cx: &mut ActionContext<'_>) -> Result<Self, ActionError> {
        Ok(Self {
            word: args.word,
            phase: ComboPhase::WaitUi,
            ui_deadline: cx.active_now() + UI_WAIT,
        })
    }

    fn poll(&mut self, cx: &mut ActionContext<'_>) -> Poll<Result<(), ActionError>> {
        if saw_correct(cx, 0) {
            return Poll::Ready(Ok(()));
        }
        match self.phase {
            ComboPhase::WaitUi => {
                if door_ui_open(cx) {
                    self.phase = ComboPhase::Dial {
                        index: 0,
                        last: None,
                        clicks: 0,
                        deadline: cx.active_now() + DIAL_WAIT,
                    };
                    return Poll::Pending;
                }
                if cx.active_now() >= self.ui_deadline {
                    return Poll::Ready(Err(ActionError::Failed(Arc::from(
                        "totem security door never raised its lock",
                    ))));
                }
                Poll::Pending
            }
            ComboPhase::Dial {
                index,
                last,
                clicks,
                deadline,
            } => self.poll_dial(cx, index, last, clicks, deadline),
            ComboPhase::Confirm {
                chat_since,
                deadline,
            } => {
                if saw_correct(cx, chat_since) {
                    return Poll::Ready(Ok(()));
                }
                if !door_ui_open(cx) && cx.active_now() >= deadline {
                    return Poll::Ready(Err(ActionError::Failed(Arc::from(
                        "totem security door refused the combination",
                    ))));
                }
                if cx.active_now() >= deadline {
                    return Poll::Ready(Err(ActionError::Failed(Arc::from(
                        "totem security door did not accept the combination",
                    ))));
                }
                Poll::Pending
            }
        }
    }

    fn cancel(&mut self) {}
}

impl Combo {
    fn poll_dial(
        &mut self,
        cx: &mut ActionContext<'_>,
        index: usize,
        last: Option<char>,
        clicks: u8,
        deadline: Duration,
    ) -> Poll<Result<(), ActionError>> {
        if !door_ui_open(cx) {
            return Poll::Ready(Err(ActionError::Failed(Arc::from(
                "totem lock interface closed",
            ))));
        }
        let Some(target) = self.word.chars().nth(index) else {
            return Poll::Ready(Err(ActionError::Failed(Arc::from(
                "totem combination is not four letters",
            ))));
        };
        let Some(current) = dial_letter(cx, index) else {
            if cx.active_now() >= deadline {
                return Poll::Ready(Err(ActionError::Failed(Arc::from(
                    "totem dial is not readable",
                ))));
            }
            return Poll::Pending;
        };
        if current == target {
            let next = index + 1;
            if next >= self.word.len() {
                let chat_since = reach::last_chat_seq(cx);
                if let Err(error) = cx.emit(InteractReq::IfButton {
                    component_id: CONFIRM,
                }) {
                    return budget_retry(error);
                }
                self.phase = ComboPhase::Confirm {
                    chat_since,
                    deadline: cx.active_now() + CONFIRM_WAIT,
                };
                return Poll::Pending;
            }
            self.phase = ComboPhase::Dial {
                index: next,
                last: None,
                clicks: 0,
                deadline: cx.active_now() + DIAL_WAIT,
            };
            return Poll::Pending;
        }
        if last == Some(current) && cx.active_now() < deadline {
            return Poll::Pending;
        }
        if clicks > LETTERS as u8 {
            return Poll::Ready(Err(ActionError::Failed(Arc::from(
                "totem dial did not reach its letter",
            ))));
        }
        if let Err(error) = cx.emit(InteractReq::IfButton {
            component_id: dial_button(index, current, target),
        }) {
            return budget_retry(error);
        }
        self.phase = ComboPhase::Dial {
            index,
            last: Some(current),
            clicks: clicks.saturating_add(1),
            deadline: cx.active_now() + DIAL_WAIT,
        };
        Poll::Pending
    }
}

/// A dial or confirm click finds this observed tick's one event already spent
/// (`ledger.rs` `TickBudget::event`), for example when a fresh snapshot in the
/// same tick already shows the letter that the last click echoed
/// (`quest_totem.rs2` `tribal_door_combo` ends in `if_settext`). Phase state
/// is only committed after a successful emit, so waiting one tick repeats the
/// same decision; any other emit error still ends the step.
fn budget_retry(error: ActionError) -> Poll<Result<(), ActionError>> {
    crate::native::defer_budget(Err(error))
}

fn door_ui_open(cx: &ActionContext<'_>) -> bool {
    cx.snapshot()
        .main_modal()
        .is_some_and(|modal| modal.value.root == DOOR_UI)
}

fn dial_letter(cx: &ActionContext<'_>, index: usize) -> Option<char> {
    let id = *DIAL_TEXT.get(index)?;
    let widgets = cx.snapshot().widgets()?;
    let text = widgets
        .value
        .iter()
        .find(|widget| widget.component_id == id)?
        .text
        .as_deref()?;
    let letter = text.trim().chars().next()?.to_ascii_uppercase();
    letter.is_ascii_uppercase().then_some(letter)
}

fn saw_correct(cx: &ActionContext<'_>, since: i32) -> bool {
    cx.snapshot().chat_lines(since).is_some_and(|lines| {
        lines.value.iter().any(|line| {
            line.username.is_none()
                && line.type_ == 0
                && line.text.to_ascii_lowercase().contains(CORRECT)
        })
    })
}

fn walk_arrival(receipt: WalkReceipt) -> Result<(), ActionError> {
    receipt.into_arrival().map(|_| ())
}

/// Shortest rotation on a 26-letter wheel: forward (up) when that is ≤ 13 steps.
fn dial_button(index: usize, current: char, target: char) -> i32 {
    let forward = (target as i32 - current as i32).rem_euclid(LETTERS);
    if forward * 2 <= LETTERS {
        DIAL_UP[index]
    } else {
        DIAL_DOWN[index]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::quester::families::tests::{local_player, post_walk_receipt, with_tick};
    use api::snapshot::GameSnapshot;

    /// The door approach is gated on `reach::within` against the authored
    /// anchor and radius, so it must accept any standable tile in that area
    /// rather than demand a path onto the door tile.
    #[test]
    fn door_approach_walks_the_anchor_area() {
        if !api::test_support::game_data_available() {
            return;
        }
        let door = WorldTile {
            x: 2658,
            z: 3322,
            level: 0,
        };
        let plan = super::Plan {
            word: Arc::from("KURT"),
            door_id: 716,
            door_name: Arc::from("combodoor"),
            tile: door,
            radius: 2,
        };
        let mut snapshot = GameSnapshot::new();
        snapshot.seed_ingame(2);
        snapshot.seed_local_player(local_player(WorldTile {
            x: 2640,
            z: 3322,
            level: 0,
        }));
        let mut ledger = None;
        let mut run = with_tick(&snapshot, &mut ledger, 1, |tick| {
            with_step(tick, |cx| plan.begin(cx).unwrap())
        });
        assert!(with_tick(&snapshot, &mut ledger, 2, |tick| {
            with_step(tick, |cx| run.poll(cx))
        })
        .is_pending());
        let crate::native::HostEffect::Walk(request) =
            &ledger.as_ref().unwrap().outbox.last().unwrap().effect
        else {
            panic!("the door approach must emit a native walk");
        };
        assert_eq!(request.target, door);
        assert_eq!(request.radius, 2);
        assert_eq!(request.loc_id, None);
        assert_eq!(
            request.arrival,
            nav_planner::contract::ArrivalKind::Area,
            "the authored door radius accepts any standable tile inside it"
        );
    }

    #[test]
    fn dial_picks_the_shorter_rotation() {
        assert_eq!(dial_button(0, 'A', 'B'), DIAL_UP[0], "one step forward");
        assert_eq!(
            dial_button(1, 'A', 'N'),
            DIAL_UP[1],
            "13 forward ties to up"
        );
        assert_eq!(
            dial_button(2, 'A', 'O'),
            DIAL_DOWN[2],
            "14 forward is 12 back"
        );
        assert_eq!(dial_button(3, 'A', 'Z'), DIAL_DOWN[3], "wrap-around down");
        assert_eq!(dial_button(0, 'K', 'U'), DIAL_UP[0], "KURT second letter");
        assert_eq!(
            dial_button(0, 'K', 'A'),
            DIAL_DOWN[0],
            "ten steps back beats sixteen forward"
        );
    }

    fn with_step<R>(
        tick: &mut crate::native::NativeTick<'_>,
        f: impl FnOnce(&mut crate::quester::compile::StepContext<'_, '_>) -> R,
    ) -> R {
        let quests = api::quest_facts::QuestCatalog::empty();
        let banks = Arc::new(api::named_banks::NamedBankFacts::empty());
        let choices = crate::quester::choices::QuestChoices::default();
        let required_after = tick.cx.evidence();
        f(&mut crate::quester::compile::StepContext {
            tick,
            quests: &quests,
            progress: &[],
            required_after,
            banks: &banks,
            choices: &choices,
        })
    }

    #[test]
    fn totem_door_retries_a_failed_walk_on_the_next_tick() {
        if !api::test_support::game_data_available() {
            return;
        }
        let tile = WorldTile {
            x: 12,
            z: 34,
            level: 0,
        };
        let mut snapshot = GameSnapshot::new();
        snapshot.seed_ingame(2);
        snapshot.seed_local_player(local_player(WorldTile {
            x: 30,
            z: 40,
            level: 0,
        }));
        let mut ledger = None;
        let plan = Plan {
            word: Arc::from("KURT"),
            door_id: 41,
            door_name: Arc::from("Tribal door"),
            tile,
            radius: 1,
        };
        let mut run = with_tick(&snapshot, &mut ledger, 1, |tick| {
            with_step(tick, |cx| plan.begin(cx).unwrap())
        });
        assert!(with_tick(&snapshot, &mut ledger, 2, |tick| {
            with_step(tick, |cx| run.poll(cx))
        })
        .is_pending());
        post_walk_receipt(
            &mut ledger,
            2,
            crate::native::WalkEnd::NoPath,
            Some("totem approach failed"),
        );
        assert!(with_tick(&snapshot, &mut ledger, 2, |tick| {
            with_step(tick, |cx| run.poll(cx))
        })
        .is_pending());
        assert!(with_tick(&snapshot, &mut ledger, 3, |tick| {
            with_step(tick, |cx| run.poll(cx))
        })
        .is_pending());
    
        let retry = ledger.as_ref().unwrap().outbox.last().unwrap();
        let crate::native::HostEffect::Walk(request) = &retry.effect else {
            panic!("the next attempt should be a fresh totem approach walk");
        };
        assert_eq!(request.target, tile);
        assert_eq!(request.required_after.tick, 3);
    }

    fn dial_widget(component_id: i32, letter: &str) -> api::snapshot::WidgetView {
        use api::snapshot::{WidgetKind, WidgetRoot};
        api::snapshot::WidgetView {
            kind: WidgetKind::Widget,
            component_id,
            layer_id: component_id,
            parent_id: DOOR_UI,
            root_component_id: DOOR_UI,
            root: WidgetRoot::Main,
            type_: 4,
            button_type: 0,
            client_code: 0,
            x: 0,
            y: 0,
            width: 1,
            height: 1,
            scroll_height: 0,
            inventory_ready: false,
            scroll_position: 0,
            hidden: false,
            text: Some(letter.to_owned()),
            alternate_text: None,
            button_text: None,
            target_verb: None,
            target_base: None,
            target_mask: 0,
            model_type: 0,
            model_id: -1,
            alternate_model_type: 0,
            alternate_model_id: -1,
            scripts: None,
            script_comparators: None,
            script_operands: None,
            varp_bindings: Vec::new(),
            colour: 0,
            actions: Vec::new(),
            items: Vec::new(),
        }
    }

    fn lock_snapshot(letters: [&str; 4]) -> GameSnapshot {
        let mut snapshot = GameSnapshot::new();
        snapshot.seed_ingame(2);
        snapshot.seed_main_modal(
            DOOR_UI,
            DIAL_TEXT
                .iter()
                .zip(letters)
                .map(|(id, letter)| dial_widget(*id, letter))
                .collect(),
        );
        snapshot
    }

    /// One click spends the observed tick's only event. When the snapshot of
    /// that same tick already shows the echoed letter, the next dial's click
    /// has no event left: it waits for the next tick and then goes out, and
    /// the step does not fail with `BudgetExhausted` (totem:4 receipt, the
    /// third dial, after 17 clicks).
    #[test]
    fn dial_click_waits_for_the_next_tick_when_the_event_is_spent() {
        if !api::test_support::game_data_available() {
            return;
        }
        let before = lock_snapshot(["A", "A", "A", "A"]);
        let echoed = lock_snapshot(["B", "A", "A", "A"]);
        let mut ledger = None;
        let handle = crate::quester::families::tests::with_tick_snapshots(
            &before,
            &echoed,
            &mut ledger,
            1,
            |tick, echoed| {
                let handle = tick
                    .actions
                    .begin::<Combo>(
                        ComboArgs {
                            word: Arc::from("BBAA"),
                        },
                        &mut tick.cx,
                    )
                    .unwrap();
                // The lock opens, then the first dial's click is sent.
                assert!(tick.actions.poll(&handle, &mut tick.cx).is_pending());
                assert!(tick.actions.poll(&handle, &mut tick.cx).is_pending());
                assert_eq!(tick.cx.ledger.as_ref().unwrap().outbox.len(), 1);
                tick.cx.snapshot =
                    api::snapshot::SnapshotView::new(Some(echoed), tick.cx.evidence());
                // The first dial now reads its letter; the second dial's click
                // has no event left in this tick.
                assert!(tick.actions.poll(&handle, &mut tick.cx).is_pending());
                assert!(
                    tick.actions.poll(&handle, &mut tick.cx).is_pending(),
                    "a spent tick event defers the click and does not fail the step"
                );
                assert_eq!(tick.cx.ledger.as_ref().unwrap().outbox.len(), 1);
                handle
            },
        );
        assert!(
            crate::quester::families::tests::with_tick(&echoed, &mut ledger, 2, |tick| {
                tick.actions.poll(&handle, &mut tick.cx)
            })
            .is_pending()
        );
        let outbox = &ledger.as_ref().unwrap().outbox;
        assert_eq!(outbox.len(), 2, "the deferred click goes out on the next tick");
        let crate::native::HostEffect::Interaction(InteractReq::IfButton { component_id }) =
            &outbox.last().unwrap().effect
        else {
            panic!("the next tick sends the second dial's button");
        };
        assert_eq!(*component_id, DIAL_UP[1]);
    }
}
