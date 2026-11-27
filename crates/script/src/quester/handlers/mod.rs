//! Typed quest-specific procedures, registered once per quest module.
//!
//! Each quest module exports `STEPS: &[StepHandler]` and
//! `FACTS: &[PredicateHandler]` (an empty slice if the module has no facts).
//! Rows use `crate::step!` or `crate::fact!` with a typed compile callback.
//! Argument types derive `Deserialize` and derive `JsonSchema` with
//! `path-schema` enabled. Add the module name to `quest_handlers!` below.
//! Dispatch, argument schemas, advances policy and handler ABI share these rows.
use super::compile::{PredicateHandler, StepHandler};

macro_rules! quest_handlers {
    () => {
        pub fn steps() -> impl Iterator<Item = &'static StepHandler> {
            std::iter::empty()
        }

        pub fn predicates() -> impl Iterator<Item = &'static PredicateHandler> {
            std::iter::empty()
        }
    };
    ($($quest:ident),+ $(,)?) => {
        $(mod $quest;)+

        pub fn steps() -> impl Iterator<Item = &'static StepHandler> {
            [$($quest::STEPS),+].into_iter().flatten()
        }

        pub fn predicates() -> impl Iterator<Item = &'static PredicateHandler> {
            [$($quest::FACTS),+].into_iter().flatten()
        }
    };
}

quest_handlers!(mm, totem, upass);

pub use upass::GridRun;
mod upass_store;

/// The grid step's saved-route port, for the host's SQLite tests.
#[cfg(feature = "test-hooks")]
pub use upass_store::probe as upass_route_probe;

/// The grid step's fakes, for the runner-level tests of its run memory
/// (`quester::runner::ordered_runner_tests`).
#[cfg(test)]
pub(in crate::quester) use upass::{
    plan_for_test,
    tests::{leaked_slot, retained_route, seed_retained_route, FakeService, Files},
};
