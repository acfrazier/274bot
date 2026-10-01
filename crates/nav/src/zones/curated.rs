//! Baked zone groups and hazards whose source is curated rather than NPC
//! acquisition data. Membership is resolved against the derived spawn rows.

use crate::router::AvoidRect;

pub(crate) struct GroupSpec {
    pub id: &'static str,
    pub label: &'static str,
    pub rect: AvoidRect,
    pub npc_ids: &'static [i32],
}

pub(crate) const GROUPS: &[GroupSpec] = &[
    GroupSpec {
        id: "white-wolf-mountain",
        label: "White Wolf Mountain",
        rect: AvoidRect {
            min_x: 2828,
            max_x: 2878,
            min_z: 3436,
            max_z: 3538,
            level: None,
        },
        npc_ids: &[96, 97, 125, 141, 142],
    },
    GroupSpec {
        id: "draynor-jail-guards",
        label: "Draynor jail guards",
        rect: AvoidRect {
            min_x: 3096,
            max_x: 3140,
            min_z: 3224,
            max_z: 3262,
            level: Some(0),
        },
        npc_ids: &[917],
    },
    GroupSpec {
        id: "death-plateau-throwers",
        label: "Death Plateau thrower trolls",
        rect: AvoidRect {
            min_x: 2843,
            max_x: 2878,
            min_z: 3590,
            max_z: 3608,
            level: Some(0),
        },
        npc_ids: &[1101, 1102, 1103, 1104, 1105],
    },
];

pub(crate) struct HazardSpec {
    pub id: &'static str,
    pub label: &'static str,
    pub rect: AvoidRect,
    pub level: u8,
}

pub(crate) const HAZARDS: &[HazardSpec] = &[HazardSpec {
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
}];
