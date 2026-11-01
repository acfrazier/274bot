//! Pollable route execution over the planner's navigation data.

mod traveller;

pub use traveller::{
    BlockedWhy, CrossingPass, HopFailure, LegPhase, NpcHopBlockReason, RefusalMatch,
    TransportRefusal, TravelEvent, TravelLegCallback, TravelOptions, TravelOutcome, Traveller,
    WebCutFailure,
};

#[cfg(test)]
mod planner_surface_tests;
