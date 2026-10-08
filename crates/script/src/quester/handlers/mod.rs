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

quest_handlers!(totem);
