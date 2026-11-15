//! Interface steps for content that is not a dialogue page: the numeric
//! count dialog (`p_countdialog`) and a named interface button.
//!
//! Both steps send one request, and only when the prompt is on screen.
//! They wait for the host's dispatch receipt, never repeat a request, and
//! count their waits in observed game ticks (one per `PLAYER_INFO`). The
//! step's own `settle` proves the content outcome.

use super::super::compile::{
    CompileContext, CompileError, StepContext, StepOutcome, StepPlan, StepRun,
};
use crate::interact::InteractReq;
use crate::native::{
    defer_budget, ActionContext, ActionError, ActionHandle, InteractionDispatch, NativeActions,
    NativeMachine,
};
use api::quest_progress::EvidenceStamp;
use api::snapshot::WidgetView;
use serde::Deserialize;
use std::sync::Arc;
use std::task::Poll;

/// Interface `buttontype=pause` (client `button_type` 6): its press resumes a
/// paused script (`p_pausebutton`), so the client sends `RESUME_PAUSEBUTTON`
/// (`api/src/interact.rs` `continue_dialog` requires it). Any other button
/// sends `IF_BUTTON`.
const BUTTON_TYPE_PAUSE: i32 = 6;

/// Observed ticks from an accepted count answer to the closed dialog. The
/// host writes the packet and then clears the client's `dialog_input_open` in
/// the same dispatch pass (`api/src/interact/driver.rs` `answer_count`,
/// `count_dialog_submitted`), so the next observation shows it closed. One
/// more tick covers a pass whose frame is published late.
const COUNT_CLOSE_TICKS: u64 = 2;

/// Answer the open count dialog with a number.
#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "path-schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub(super) struct AnswerCountArgs {
    /// The number typed into the dialog. The client's own answer is a
    /// non-negative 32-bit integer; the content decides what it means.
    #[cfg_attr(feature = "path-schema", schemars(range(min = 0)))]
    value: i32,
    /// Observed ticks the step waits for the dialog to open. Size it from the
    /// content delay before `p_countdialog`, and count `p_delay(n)` as n + 1.
    #[cfg_attr(feature = "path-schema", schemars(range(min = 1)))]
    max_ticks: u32,
    /// Content citation for the dialog and the chosen value.
    #[cfg_attr(feature = "path-schema", schemars(regex(pattern = r"\S")))]
    source: String,
}

/// Press one interface component once it is shown.
#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "path-schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub(super) struct IfButtonArgs {
    /// Component id from the content interface pack (`interface.pack`, the
    /// `<interface>:<component>` row). A `buttontype=pause` component sends the
    /// resume press instead of `IF_BUTTON`.
    #[cfg_attr(feature = "path-schema", schemars(range(min = 1)))]
    component: i32,
    /// Observed ticks the step waits for the component to be shown: on an open
    /// interface, with no hidden layer above it. Size it from the content
    /// delay before the script unhides it, and count `p_delay(n)` as n + 1.
    #[cfg_attr(feature = "path-schema", schemars(range(min = 1)))]
    max_ticks: u32,
    /// Content citation for the component and the script that handles it.
    #[cfg_attr(feature = "path-schema", schemars(regex(pattern = r"\S")))]
    source: String,
}

pub(super) fn compile_answer_count(
    args: AnswerCountArgs,
    _cx: &CompileContext<'_>,
) -> Result<Arc<dyn StepPlan>, CompileError> {
    if args.value < 0 || args.max_ticks == 0 {
        return Err(CompileError::code("invalid-args"));
    }
    if args.source.trim().is_empty() {
        return Err(CompileError::code("source-required"));
    }
    Ok(Arc::new(PromptPlan {
        op: Op::Count(args.value),
        max_ticks: u64::from(args.max_ticks),
    }))
}

pub(super) fn compile_if_button(
    args: IfButtonArgs,
    _cx: &CompileContext<'_>,
) -> Result<Arc<dyn StepPlan>, CompileError> {
    if args.component < 1 || args.max_ticks == 0 {
        return Err(CompileError::code("invalid-args"));
    }
    if args.source.trim().is_empty() {
        return Err(CompileError::code("source-required"));
    }
    Ok(Arc::new(PromptPlan {
        op: Op::Button(args.component),
        max_ticks: u64::from(args.max_ticks),
    }))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Op {
    Count(i32),
    Button(i32),
}

impl Op {
    fn what(self) -> &'static str {
        match self {
            Op::Count(_) => "count answer",
            Op::Button(_) => "interface button",
        }
    }
}

struct PromptPlan {
    op: Op,
    max_ticks: u64,
}

impl StepPlan for PromptPlan {
    fn begin(&self, cx: &mut StepContext<'_, '_>) -> Result<Box<dyn StepRun>, ActionError> {
        Ok(Box::new(PromptRun {
            action: Some(cx.tick.actions.begin::<Prompt>(
                PromptArgs {
                    op: self.op,
                    max_ticks: self.max_ticks,
                },
                &mut cx.tick.cx,
            )?),
        }))
    }
}

struct PromptRun {
    action: Option<ActionHandle<Prompt>>,
}

impl StepRun for PromptRun {
    fn poll(&mut self, cx: &mut StepContext<'_, '_>) -> Poll<Result<StepOutcome, ActionError>> {
        let Some(handle) = &self.action else {
            return Poll::Ready(Err(ActionError::Cancelled));
        };
        cx.tick.actions.poll(handle, &mut cx.tick.cx).map(|result| {
            result.map(|evidence| StepOutcome {
                progress: None,
                evidence,
                receipt: None,
            })
        })
    }

    fn cancel(&mut self, actions: &mut NativeActions) {
        if let Some(handle) = self.action.take() {
            actions.cancel(handle);
        }
    }
}

struct PromptArgs {
    op: Op,
    max_ticks: u64,
}

/// The prompt machine: wait until the prompt is shown, send one request,
/// then wait for the host's receipt.
struct Prompt {
    op: Op,
    /// The observed tick at which the wait for the prompt ends. Its last tick
    /// still counts, as for the `wait` step.
    deadline_tick: u64,
    request: Option<u64>,
}

impl NativeMachine for Prompt {
    type Args = PromptArgs;
    type Output = EvidenceStamp;

    fn begin(args: PromptArgs, cx: &mut ActionContext<'_>) -> Result<Self, ActionError> {
        Ok(Self {
            op: args.op,
            deadline_tick: cx.evidence().tick.saturating_add(args.max_ticks),
            request: None,
        })
    }

    fn poll(&mut self, cx: &mut ActionContext<'_>) -> Poll<Result<EvidenceStamp, ActionError>> {
        let tick = cx.evidence().tick;
        let Some(request) = self.request else {
            return self.send_when_shown(cx, tick);
        };
        match cx.interaction_dispatch(request) {
            InteractionDispatch::Queued => Poll::Pending,
            InteractionDispatch::Unsent => Poll::Ready(Err(failure(
                "the host dropped the",
                self.op,
                "before it was sent",
            ))),
            InteractionDispatch::Settled(receipt) if !receipt.accepted => Poll::Ready(Err(
                failure("the host refused the", self.op, "(see the host log)"),
            )),
            InteractionDispatch::Settled(receipt) => match self.op {
                Op::Button(_) => Poll::Ready(Ok(cx.evidence())),
                Op::Count(_) => {
                    let closed = cx
                        .snapshot()
                        .count_dialog_open()
                        .is_some_and(|open| !open.value);
                    if closed {
                        Poll::Ready(Ok(cx.evidence()))
                    } else if tick > receipt.evidence.tick.saturating_add(COUNT_CLOSE_TICKS) {
                        Poll::Ready(Err(failure(
                            "the count dialog stayed open after the",
                            self.op,
                            "was accepted",
                        )))
                    } else {
                        Poll::Pending
                    }
                }
            },
        }
    }

    fn cancel(&mut self) {}
}

impl Prompt {
    fn send_when_shown(
        &mut self,
        cx: &mut ActionContext<'_>,
        tick: u64,
    ) -> Poll<Result<EvidenceStamp, ActionError>> {
        let Some(request) = self.shown_request(cx) else {
            if tick >= self.deadline_tick {
                return Poll::Ready(Err(failure(
                    "no prompt appeared for the",
                    self.op,
                    "within the authored ticks",
                )));
            }
            return Poll::Pending;
        };
        match defer_budget(cx.emit(request)) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(Err(error)) => Poll::Ready(Err(error)),
            Poll::Ready(Ok(id)) => {
                self.request = Some(id);
                Poll::Pending
            }
        }
    }

    /// The request to send now, or `None` while the prompt is not shown.
    fn shown_request(&self, cx: &ActionContext<'_>) -> Option<InteractReq> {
        match self.op {
            Op::Count(value) => cx
                .snapshot()
                .count_dialog_open()
                .is_some_and(|open| open.value)
                .then_some(InteractReq::AnswerCount { value }),
            Op::Button(component_id) => {
                let widgets = cx.snapshot().widgets()?.value;
                let widget = shown_component(widgets, component_id)?;
                Some(if widget.button_type == BUTTON_TYPE_PAUSE {
                    InteractReq::ContinueDialog {
                        component_id: Some(component_id),
                    }
                } else {
                    InteractReq::IfButton { component_id }
                })
            }
        }
    }
}

fn failure(before: &str, op: Op, after: &str) -> ActionError {
    ActionError::Failed(Arc::from(format!("{before} {} {after}", op.what())))
}

/// The component, when it and every layer above it are not hidden. The widget
/// list holds only components reachable from an open interface root, so a
/// component that is listed is on an open interface. A hidden layer hides its
/// children without marking each child itself.
fn shown_component(widgets: &[WidgetView], component_id: i32) -> Option<&WidgetView> {
    let widget = widgets
        .iter()
        .find(|widget| widget.component_id == component_id)?;
    let mut node = widget;
    for _ in 0..widgets.len() {
        if node.hidden {
            return None;
        }
        if node.parent_id == -1 {
            return Some(widget);
        }
        node = widgets
            .iter()
            .find(|parent| parent.component_id == node.parent_id)?;
    }
    None
}

#[cfg(test)]
#[path = "prompt_tests.rs"]
mod tests;
