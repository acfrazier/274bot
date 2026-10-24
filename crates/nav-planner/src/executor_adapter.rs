//! Raw forwarding surface for `nav-executor` only. Not a semantic view: it
//! exposes router storage types (`Leg`, `CompactRouteLeg`) and transport
//! edge predicates. `script` must not name this module. The I-exec-closure
//! endpoint removes this module when the executor uses the closed contract.

use crate::contract::RoutePlanLease;
use crate::router::{CompactRouteLeg, Leg};
use crate::transport::TransportEdge;

pub fn plan_leg_count(plan: &RoutePlanLease) -> usize {
    plan.leg_count()
}

pub fn plan_retained_leg(plan: &RoutePlanLease, index: usize) -> Option<CompactRouteLeg<'_>> {
    plan.retained_leg(index)
}

pub fn plan_owned_leg(plan: &RoutePlanLease, index: usize) -> Option<Leg> {
    plan.owned_leg(index)
}

pub fn is_slashable_web(edge: &TransportEdge) -> bool {
    edge.is_slashable_web()
}

pub fn is_dialogue_door_loc(loc_id: i32) -> bool {
    crate::transport::is_dialogue_door_loc(loc_id)
}

pub fn select_web_action<'a>(
    candidates: impl Iterator<Item = &'a TransportEdge>,
    edge: &TransportEdge,
    qualifies: impl FnMut(&TransportEdge) -> bool,
) -> Option<&'a TransportEdge> {
    crate::transport::select_web_action(candidates, edge, qualifies)
}

pub const CELLAR_SHIFT: i32 = crate::transport::CELLAR_SHIFT;
pub const SHANTAY_HENGE_LOC_ID: i32 = crate::transport::SHANTAY_HENGE_LOC_ID;
pub const AL_KHARID_TOLL_LEFT_LOC_ID: i32 = crate::transport::AL_KHARID_TOLL_LEFT_LOC_ID;
pub const AL_KHARID_TOLL_RIGHT_LOC_ID: i32 = crate::transport::AL_KHARID_TOLL_RIGHT_LOC_ID;
pub const ARDOUGNE_WILDERNESS_LEVER_LOC_ID: i32 =
    crate::transport::ARDOUGNE_WILDERNESS_LEVER_LOC_ID;

#[cfg(test)]
#[path = "executor_adapter_tests.rs"]
mod tests;
