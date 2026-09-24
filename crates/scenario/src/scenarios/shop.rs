use super::{combat::*, production::*};
use crate::*;
mod buyout;
pub(crate) use buyout::*;

const LUMBRIDGE_BANK: WorldTile = WorldTile {
    x: 3092,
    z: 3245,
    level: 0,
};
pub(crate) const FALADOR_TELE_LAND: WorldTile = WorldTile {
    x: 2965,
    z: 3378,
    level: 0,
};
pub(super) const VARROCK_ANVIL: WorldTile = WorldTile {
    x: 3188,
    z: 3425,
    level: 0,
};


const AIO_TELEPORT_INJECT: &[ScriptSettingInject] = &[];
const AIO_TELEPORT_FALADOR_INJECT: &[ScriptSettingInject] = &[ScriptSettingInject {
    id: "teleportName",
    value: ScriptInjectValue::Str("falador"),
}];
const AIO_TELEPORT_NO_STAFF_INJECT: &[ScriptSettingInject] = &[ScriptSettingInject {
    id: "useStaffRunes",
    value: ScriptInjectValue::Bool(false),
}];
struct AioTeleportPlan {
    name: &'static str,
    inject: &'static [ScriptSettingInject],
    magic: i32,
    staff_id: Option<i32>,
    staff_alias: Option<&'static str>,
    pack_air: bool,
    pack_fire: bool,
    landing: WorldTile,
    restock: WorldTile,
}

pub(crate) fn aio_teleport_scenario() -> Scenario {
    aio_teleport_variant(AioTeleportPlan {
        name: "aio_teleport",
        inject: AIO_TELEPORT_INJECT,
        magic: 25,
        staff_id: Some(STAFF_OF_AIR_ID),
        staff_alias: Some("staff_of_air"),
        pack_air: false,
        pack_fire: true,
        landing: VARROCK_TELE_LAND,
        restock: VARROCK_EAST_BANK,
    })
}

pub(crate) fn aio_teleport_falador_scenario() -> Scenario {
    aio_teleport_variant(AioTeleportPlan {
        name: "aio_teleport_falador",
        inject: AIO_TELEPORT_FALADOR_INJECT,
        magic: 37,
        staff_id: Some(STAFF_OF_WATER_ID),
        staff_alias: Some("staff_of_water"),
        pack_air: true,
        pack_fire: false,
        landing: FALADOR_TELE_LAND,
        restock: FALADOR_WEST_BANK,
    })
}

pub(crate) fn aio_teleport_no_staff_scenario() -> Scenario {
    aio_teleport_variant(AioTeleportPlan {
        name: "aio_teleport_no_staff",
        inject: AIO_TELEPORT_NO_STAFF_INJECT,
        magic: 25,
        staff_id: None,
        staff_alias: None,
        pack_air: true,
        pack_fire: true,
        landing: VARROCK_TELE_LAND,
        restock: VARROCK_EAST_BANK,
    })
}

/// Pack two laws so the default 1000-law withdraw never runs inside 180s.
/// Falador also packs Air: water staff covers Water, not Air.
fn aio_teleport_variant(plan: AioTeleportPlan) -> Scenario {
    let AioTeleportPlan {
        name,
        inject,
        magic,
        staff_id,
        staff_alias,
        pack_air,
        pack_fire,
        landing,
        restock,
    } = plan;
    let first_xp = Proof::StatXpGain {
        id: MAGIC_STAT,
        min: 1,
    };
    let further_xp = Proof::FreshStatXpGain {
        id: MAGIC_STAT,
        min: 1,
    };
    let land = Proof::ArrivedNear {
        x: landing.x,
        z: landing.z,
        level: landing.level,
        radius: 8,
    };
    let mut steps = script_live_seed_steps();
    steps.push(Step {
        name: "seed Magic, packed laws, and Lumbridge bank before Start",
        kind: StepKind::Perform {
            send: Box::new(move |c, _| {
                cheat(c, "~clearinv");
                cheat(c, &format!("setstat magic {magic}"));
                cheat(c, "give lawrune 2");
                if pack_air {
                    cheat(c, "give airrune 20");
                }
                if pack_fire {
                    cheat(c, "give firerune 20");
                }
                if let Some(alias) = staff_alias {
                    cheat(c, &format!("give {alias} 1"));
                }
                cheat(c, "givebank lawrune 200");
                cheat(
                    c,
                    &tele_args(LUMBRIDGE_BANK.level, LUMBRIDGE_BANK.x, LUMBRIDGE_BANK.z),
                );
                true
            }),
        },
        wait: Wait {
            arm: Proof::ArrivedNear {
                x: LUMBRIDGE_BANK.x,
                z: LUMBRIDGE_BANK.z,
                level: LUMBRIDGE_BANK.level,
                radius: 8,
            },
            budget_ticks: 200,
        },
    });
    steps.push(bank_fletcher_watch(
        "confirm Magic before Start",
        Proof::Stat {
            id: MAGIC_STAT,
            min: magic,
        },
    ));
    steps.push(bank_fletcher_watch(
        "confirm packed laws before Start",
        Proof::ItemId {
            id: LAW_RUNE_ID,
            count: 2,
        },
    ));
    if let Some(id) = staff_id {
        steps.push(wear_combat_item_step(
            "wield and acknowledge the covering staff before Start",
            id,
        ));
    } else {
        steps.push(bank_fletcher_watch(
            "confirm no covering air staff before Start",
            Proof::ItemIdAtMost {
                id: STAFF_OF_AIR_ID,
                count: 0,
            },
        ));
    }
    steps.push(tanner_open_seed_bank(
        "open and acknowledge the banked law restock",
        Proof::BankItemId {
            id: LAW_RUNE_ID,
            count: 200,
        },
    ));
    steps.push(bank_fletcher_close_seed_bank());
    steps.push(start_catalog_step());
    for (step_name, arm) in [
        ("watch Magic XP from Game.teleport after Start", first_xp),
        (
            "watch arrival at the selected teleport land after Start",
            land,
        ),
        (
            "watch a packed law consumed after Start",
            Proof::ItemIdAtMost {
                id: LAW_RUNE_ID,
                count: 1,
            },
        ),
        (
            "watch law restock at the destination bank after Start",
            Proof::BankItemId {
                id: LAW_RUNE_ID,
                count: 1,
            },
        ),
        ("watch the teleport bank close", Proof::BankClosed),
        ("watch a further Magic XP after restock", further_xp),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    let _ = restock;
    Scenario {
        name,
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: true,
        },
        steps,
        proof: further_xp,
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            require_mainland_base: true,
            deadline: SCRIPT_GOLD_DEADLINE,
            start_script: Some("AIO Teleport"),
            script_settings_inject: if inject.is_empty() {
                None
            } else {
                Some(inject)
            },
            terminal_shot: Some(name),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}

