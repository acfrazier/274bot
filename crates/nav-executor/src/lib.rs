//! Pollable route execution over the planner's navigation data.

mod traveller;

pub use traveller::{HopFailure, LegPhase, TravelEvent, TravelOptions, TravelOutcome, Traveller};

#[cfg(test)]
mod planner_surface_tests;
