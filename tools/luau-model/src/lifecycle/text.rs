//! Printed traces: each step in the terms of LD1 and of the product's
//! functions.

use super::{Action, Alloc, Binding, Intent, Lifecycle, Op, Ret, Site, Wake, Who};
use std::fmt::Write as _;

impl Action {
    pub fn text(self) -> String {
        match self {
            Action::Start(Who::Operator) => {
                "control thread: Start (a Load entry runs inside it)".into()
            }
            Action::Start(Who::Runner) => "role pump: the runner's planned Start".into(),
            Action::Pause => "control thread: Pause (sets a flag; no entry)".into(),
            Action::Resume => "control thread: Resume (clears the flag; no entry)".into(),
            Action::Stop => "control thread: Stop (operator)".into(),
            Action::Remove => "control thread: Stop (removal of the slot)".into(),
            Action::DropSlot => "slot thread: raw Drop of the slot".into(),
            Action::Dispatch(Wake::Tick) => {
                "slot thread: dispatch of a game tick (held when the host holds the frame)".into()
            }
            Action::Dispatch(Wake::Evidence) => "slot thread: dispatch of an evidence wake".into(),
            Action::Dispatch(Wake::SeatPage) => "slot thread: dispatch of a seat page".into(),
            Action::Observe => {
                "slot thread: observe step (poll_ready, the stop completion, the log drain)".into()
            }
            Action::Restart => "slot thread: the progress watchdog restarts the run".into(),
            Action::Admit => "seat: drains the rows that the run queued".into(),
            Action::ClientDown => "client: down (the slot gates itself Paused)".into(),
            Action::ClientUp => "client: up".into(),
            Action::HoldOn => "host: a hold starts".into(),
            Action::HoldOff => "host: the hold ends".into(),
            Action::Arrive => {
                "seat: the completion of the awaited request arrives on the page".into()
            }
            Action::SessionReset => "host: a session reset ends the session's seat work".into(),
            Action::Advance => "clock: one time unit passes".into(),
            Action::Next => "slot thread: the dispatch goes on to its Paint entry".into(),
            Action::Vm(op) => op.text(),
            Action::Fence => "runner: one try-lock attempt publishes the entry fence".into(),
            Action::FenceWait => {
                "runner: waits for the busy slot mutex under the runner lock".into()
            }
            Action::Revoke => "runner: revokes the dock and its entries".into(),
            Action::Post(Intent::Start) => "runner: posts a Start intent".into(),
            Action::Post(Intent::Stop { serial }) => {
                format!("runner: posts a Stop intent for execution #{serial}")
            }
            Action::Pump => "role pump: executes the posted intent, outside the runner lock".into(),
            Action::Take => "runner: takes the queued extension call".into(),
            Action::Complete => "runner: completes the call that it took".into(),
            Action::FinalGate => "runner: the final gate reads the ledger and the slot".into(),
        }
    }
}

impl Op {
    pub fn text(self) -> String {
        match self {
            Op::Gap => "script: runs operations with no check point".into(),
            Op::Point => {
                "script: reaches an interrupt (call, return or back-edge); the check runs".into()
            }
            Op::Call(Binding::Row) => "script: calls an action binding".into(),
            Op::Call(Binding::Log) => "script: calls a log binding".into(),
            Op::Call(Binding::Wait(wait)) => {
                format!("script: calls a waiting binding ({wait:?}) and yields")
            }
            Op::Call(Binding::Stop) => "script: calls api.stop".into(),
            Op::Call(Binding::Ext { awaited }) => format!(
                "script: calls an extension function ({})",
                if awaited { "awaited" } else { "request" }
            ),
            Op::Return(Ret::Plain) => "script: the entry function returns".into(),
            Op::Return(Ret::Table { hook, paint }) => {
                format!("script: the file returns its table (onStop {hook}, onPaint {paint})")
            }
            Op::Return(Ret::Other) => "script: the file returns a value that is not a table".into(),
            Op::Fail => "script: an error leaves the entry".into(),
            Op::Catch => "script: pcall swallows the error".into(),
            Op::Alloc(Alloc::OverS) => "script: allocates until used bytes reach S".into(),
            Op::Alloc(Alloc::Refuse(Site::Host)) => {
                "script: the allocator refuses a request; the resume returns MemoryError".into()
            }
            Op::Alloc(Alloc::Refuse(Site::Binding)) => {
                "script: the allocator refuses a request inside a binding".into()
            }
            Op::Alloc(Alloc::Refuse(Site::Guard)) => {
                "script: the allocator refuses a request inside a guarded pcall".into()
            }
            Op::Alloc(Alloc::Lost) => "script: a binding gets the replaced error of mlua".into(),
            Op::Free => "script: drops its large values".into(),
            Op::Panic => "run: an internal failure (panic)".into(),
        }
    }
}

/// Print a trace step by step, with the events and the state after each.
pub fn describe_trace(model: &Lifecycle, trace: &[Action]) -> String {
    let mut state = model.initial_state();
    let mut out = String::new();
    for (index, action) in trace.iter().enumerate() {
        match model.step_detail(&state, action) {
            Ok((next, events)) => {
                let _ = writeln!(out, "{:>3}. {}", index + 1, action.text());
                if !events.is_empty() {
                    let _ = writeln!(out, "       events: {events:?}");
                }
                let _ = writeln!(out, "       state:  {}", next.summary());
                state = next;
            }
            Err(violation) => {
                let _ = writeln!(
                    out,
                    "{:>3}. {}\n       VIOLATION {}: {}",
                    index + 1,
                    action.text(),
                    violation.name,
                    violation.detail
                );
                break;
            }
        }
    }
    out
}
