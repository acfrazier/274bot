//! Per-isolate result of the script-installed `EventSignal.setInterrupt` callback.
//! The callback runs once at the bounded tick boundary; Rust walk machines read
//! this bit without calling back into JS from their policy.

use std::cell::Cell;

thread_local! {
    static INTERRUPTED: Cell<bool> = const { Cell::new(false) };
}

pub(crate) fn set(interrupted: bool) {
    INTERRUPTED.with(|value| value.set(interrupted));
}

pub(crate) fn pending() -> bool {
    INTERRUPTED.with(Cell::get)
}

pub(crate) fn clear() {
    set(false);
}
