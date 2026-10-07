//! Tribal Totem security-door combination (`x:totem:solve_combination`).
//!
//! Interface layout is content `tribal_door2` (pack id 716). The word itself
//! is Path data so a revision that only changes the answer changes the
//! document. AIO reads the echoed letter after every click
//! (`defs/tribaltotem.ts` `setDial`).
use crate::native::walk::Walk;
use crate::native::{ActionContext, ActionError, ActionHandle, NativeActions, NativeMachine, WalkReceipt};
use crate::quester::compile::{
    CompileContext, CompileError, StepContext, StepOutcome, StepPlan, StepRun,
};
use crate::quester::families::reach;
use crate::shim::InteractReq;
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
    walk: Option<ActionHandle<Walk>>,
    reach: Option<ActionHandle<reach::Reach>>,
    combo: Option<ActionHandle<Combo>>,
    opened: bool,
}

impl StepRun for Run {
    fn poll(&mut self, cx: &mut StepContext<'_, '_>) -> Poll<Result<StepOutcome, ActionError>> {
        if let Some(handle) = &self.walk {
            match cx.tick.actions.poll(handle, &mut cx.tick.cx) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(Err(error)) => return Poll::Ready(Err(error)),
                Poll::Ready(Ok(receipt)) => {
                    walk_arrival(receipt)?;
                    self.walk = None;
                }
            }
        }
        if door_ui_open(&cx.tick.cx) {
            self.opened = true;
        }
        if !self.opened {
            let here = cx.tick.cx.snapshot().here();
            if !here.is_some_and(|obs| reach::within(obs.value, self.tile, self.radius)) {
                self.walk = Some(cx.tick.actions.begin::<Walk>(
                    reach::walk_request(
                        self.tile,
                        self.radius.max(1) as u16,
                        None,
                        cx.required_after,
                    ),
                    &mut cx.tick.cx,
                )?);
                return Poll::Pending;
            }
            if self.reach.is_none() {
                self.reach = Some(cx.tick.actions.begin::<reach::Reach>(
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
                    &mut cx.tick.cx,
                )?);
                return Poll::Pending;
            }
        }
        if let Some(handle) = &self.reach {
            match cx.tick.actions.poll(handle, &mut cx.tick.cx) {
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
                    return Poll::Ready(Err(error));
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
            return Poll::Ready(Err(error));
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
    use super::{dial_button, DIAL_DOWN, DIAL_UP};

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
}
