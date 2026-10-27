//! Pollable route execution over the planner's navigation data.

mod traveller;

pub use traveller::{
    BlockedWhy, HopFailure, LegPhase, NpcHopBlockReason, TransportRefusal, TravelEvent,
    TravelLegCallback, TravelOptions, TravelOutcome, Traveller, WebCutFailure,
};

#[cfg(test)]
mod planner_surface_tests;
