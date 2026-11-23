//! Pollable route execution over the planner's navigation data.

mod traveller;

pub use traveller::{
    BlockedWhy, CrossingPass, DoorCrossing, DoorCrossings, HopFailure, LegPhase, LocId,
    NpcHopBlockReason, RefusalMatch, TickCount, TransportRefusal, TravelEvent, TravelLegCallback,
    TravelOptions, TravelOutcome, TravelWaitReason, Traveller, WebCutFailure,
};

#[cfg(test)]
mod planner_surface_tests;
