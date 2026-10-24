//! Shared curated hazard facts for runtime risk and release zone generation.

use crate::router::AvoidRect;

pub struct HazardSpec {
    pub id: &'static str,
    pub label: &'static str,
    pub rect: AvoidRect,
    pub level: u8,
    pub damage: Option<u8>,
}

pub const HAZARDS: &[HazardSpec] = &[HazardSpec {
    id: "ikov-lava-bridge",
    label: "Temple of Ikov lava bridge (20 damage unless weight < 0)",
    rect: AvoidRect {
        min_x: 2648,
        max_x: 2650,
        min_z: 9828,
        max_z: 9829,
        level: Some(0),
    },
    level: 0,
    damage: Some(20),
}];

/// Fixed hazard damage by curated zone kind id; absent facts stay unknown.
pub fn hazard_damage(id: &str) -> Option<u8> {
    HAZARDS
        .iter()
        .find(|hazard| hazard.id == id)
        .and_then(|hazard| hazard.damage)
}
