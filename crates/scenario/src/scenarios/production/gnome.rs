use super::*;
pub(crate) const WOODCUTTING_STAT: i32 = 8;
pub(crate) const MAGIC_LOGS_ID: i32 = 1513;
const NOTED_MAGIC_LOGS_ID: i32 = 1514;
pub(crate) const UNSTRUNG_MAGIC_SHORTBOW_ID: i32 = 72;
pub(crate) const UNSTRUNG_MAGIC_LONGBOW_ID: i32 = 70;
pub(crate) const MAGIC_SHORTBOW_ID: i32 = 861;
const MAGIC_LONGBOW_ID: i32 = 859;
const NOTED_UNSTRUNG_MAGIC_SHORTBOW_ID: i32 = 73;
const NOTED_UNSTRUNG_MAGIC_LONGBOW_ID: i32 = 71;
pub(crate) const RUNE_AXE_ID: i32 = 1359;
const MAGIC_TREE_ID: i32 = 1306;
/// One Rune axe plus 26 unstackable Knives leaves one product slot. The
/// frozen Gnome script preserves Knife as a tool during gear prep and bank trips.
pub(crate) const GNOME_BALLAST_KNIVES: i32 = 26;
const GNOME_SOUTH_BANK_MAGIC_STAND: WorldTile = WorldTile {
    x: 2433,
    z: 3409,
    level: 0,
};
const GNOME_SOUTH_BANK_MAGIC_TREE: WorldTile = WorldTile {
    x: 2432,
    z: 3410,
    level: 0,
};
pub(crate) const GNOME_BANK_STAND: WorldTile = WorldTile {
    x: 2445,
    z: 3425,
    level: 1,
};
pub(crate) const GNOME_BANK_STAIR_SOUTH: WorldTile = WorldTile {
    x: 2444,
    z: 3416,
    level: 0,
};
const GNOME_CHOP_INJECT: &[ScriptSettingInject] = &[ScriptSettingInject {
    id: "fletchLogs",
    value: ScriptInjectValue::Bool(false),
}];

const GNOME_FLETCH_INJECT: &[ScriptSettingInject] = &[ScriptSettingInject {
    id: "fletchLogs",
    value: ScriptInjectValue::Bool(true),
}];
/// One free product slot at the south-bank Magic tree. fletchLogs off. Seed WC 75, Rune
/// axe 1359, and retained nonproduct Knife ballast. The script chops one
/// magic log 1513 with Woodcutting XP, deposits it at the upstairs gnome
/// booth, returns to ground and chops again. This qualifies the resource
/// cycle, not ordinary 28-slot throughput. Death recovery stays out.
pub(crate) fn gnome_chop_scenario() -> Scenario {
    let stand = GNOME_SOUTH_BANK_MAGIC_STAND;
    let first_xp = Proof::StatXpGain {
        id: WOODCUTTING_STAT,
        min: 1,
    };
    let logs = Proof::ItemId {
        id: MAGIC_LOGS_ID,
        count: 1,
    };
    let mut steps = script_live_seed_steps();
    steps.push(Step {
        name: "seed woodcutting, rune axe, 26-Knife ballast and the south-bank Magic tree before Start",
        kind: StepKind::Perform {
            send: Box::new(move |c, _| {
                cheat(c, "~clearinv");
                cheat(c, "setstat woodcutting 75");
                cheat(c, "give rune_axe 1");
                cheat(c, &format!("give knife {GNOME_BALLAST_KNIVES}"));
                cheat(c, &tele_args(stand.level, stand.x, stand.z));
                true
            }),
        },
        wait: Wait {
            arm: Proof::ArrivedNear {
                x: stand.x,
                z: stand.z,
                level: stand.level,
                radius: 1,
            },
            budget_ticks: 200,
        },
    });
    for (step_name, arm) in [
        (
            "confirm Woodcutting 75 before Start",
            Proof::Stat {
                id: WOODCUTTING_STAT,
                min: 75,
            },
        ),
        (
            "confirm Rune axe 1359 before Start",
            Proof::ItemId {
                id: RUNE_AXE_ID,
                count: 1,
            },
        ),
        (
            "confirm exactly one Rune axe before Start",
            Proof::ItemIdAtMost {
                id: RUNE_AXE_ID,
                count: 1,
            },
        ),
        (
            "confirm exact south-bank Magic tree and Chop down action before Start",
            Proof::LocActionNear {
                id: MAGIC_TREE_ID,
                x: GNOME_SOUTH_BANK_MAGIC_TREE.x,
                z: GNOME_SOUTH_BANK_MAGIC_TREE.z,
                level: GNOME_SOUTH_BANK_MAGIC_TREE.level,
                radius: 0,
                action: "Chop down",
                present: true,
            },
        ),
        (
            "confirm 26 retained nonproduct Knives before Start",
            Proof::ItemId {
                id: KNIFE_ID,
                count: GNOME_BALLAST_KNIVES,
            },
        ),
        (
            "confirm exactly 26 retained nonproduct Knives before Start",
            Proof::ItemIdAtMost {
                id: KNIFE_ID,
                count: GNOME_BALLAST_KNIVES,
            },
        ),
        (
            "confirm no seeded magic logs in pack before Start",
            Proof::ItemIdAtMost {
                id: MAGIC_LOGS_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded unstrung magic shortbow in pack before Start",
            Proof::ItemIdAtMost {
                id: UNSTRUNG_MAGIC_SHORTBOW_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded unstrung magic longbow in pack before Start",
            Proof::ItemIdAtMost {
                id: UNSTRUNG_MAGIC_LONGBOW_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded noted magic logs in pack before Start",
            Proof::ItemIdAtMost {
                id: NOTED_MAGIC_LOGS_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded noted unstrung magic shortbow in pack before Start",
            Proof::ItemIdAtMost {
                id: NOTED_UNSTRUNG_MAGIC_SHORTBOW_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded noted unstrung magic longbow in pack before Start",
            Proof::ItemIdAtMost {
                id: NOTED_UNSTRUNG_MAGIC_LONGBOW_ID,
                count: 0,
            },
        ),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    steps.push(start_catalog_step());
    for (step_name, arm) in [
        (
            "watch Woodcutting XP from a Magic tree after Start",
            first_xp,
        ),
        ("watch exact Magic logs 1513 chopped after Start", logs),
        (
            "watch arrival at the upstairs gnome booth after the log fill",
            Proof::ArrivedNear {
                x: GNOME_BANK_STAND.x,
                z: GNOME_BANK_STAND.z,
                level: GNOME_BANK_STAND.level,
                radius: 8,
            },
        ),
        (
            "watch script-chopped magic logs enter a fresh gnome bank",
            Proof::BankItemId {
                id: MAGIC_LOGS_ID,
                count: 1,
            },
        ),
        (
            "watch the pack empty of magic logs after deposit",
            Proof::ItemIdAtMost {
                id: MAGIC_LOGS_ID,
                count: 0,
            },
        ),
        (
            "watch ground return at the gnome bank stairs after deposit",
            Proof::ArrivedNear {
                x: GNOME_BANK_STAIR_SOUTH.x,
                z: GNOME_BANK_STAIR_SOUTH.z,
                level: GNOME_BANK_STAIR_SOUTH.level,
                radius: 30,
            },
        ),
        (
            "watch the gnome log bank close after deposit",
            Proof::BankClosed,
        ),
        ("watch another exact Magic logs 1513 after return", logs),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    Scenario {
        name: "gnome_chop",
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: true,
        },
        steps,
        proof: logs,
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            require_mainland_base: true,
            deadline: SCRIPT_GOLD_DEADLINE,
            start_script: Some("GnomeMagicChopper"),
            script_settings_inject: Some(GNOME_CHOP_INJECT),
            terminal_shot: Some("gnome_chop"),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}

struct GnomeFletchSpec {
    name: &'static str,
    fletching: i32,
    fletching_max: Option<i32>,
    product_id: i32,
}

pub(crate) fn gnome_fletch_short_scenario() -> Scenario {
    gnome_fletch_variant(GnomeFletchSpec {
        name: "gnome_fletch_short",
        fletching: 80,
        fletching_max: Some(84),
        product_id: UNSTRUNG_MAGIC_SHORTBOW_ID,
    })
}

pub(crate) fn gnome_fletch_long_scenario() -> Scenario {
    gnome_fletch_variant(GnomeFletchSpec {
        name: "gnome_fletch_long",
        fletching: 85,
        fletching_max: None,
        product_id: UNSTRUNG_MAGIC_LONGBOW_ID,
    })
}

/// fletchLogs on. Seed WC 75, Fletching 80/85, Rune axe, and 26 retained
/// Knives so one script-chopped log fills the pack. The script consumes it
/// into exact unstrung 72/70 with Fletching XP, deposits the bow upstairs,
/// returns to ground and chops again. This is cycle qualification, not
/// ordinary capacity proof. Missing Knife is a stop, not a pass; strung
/// 861/859 are not the unstrung product.
fn gnome_fletch_variant(spec: GnomeFletchSpec) -> Scenario {
    let GnomeFletchSpec {
        name,
        fletching,
        fletching_max,
        product_id,
    } = spec;
    let stand = GNOME_SOUTH_BANK_MAGIC_STAND;
    let first_wc = Proof::StatXpGain {
        id: WOODCUTTING_STAT,
        min: 1,
    };
    let first_fletch = Proof::StatXpGain {
        id: FLETCHING_STAT,
        min: 1,
    };
    let logs = Proof::ItemId {
        id: MAGIC_LOGS_ID,
        count: 1,
    };
    let product = Proof::ItemId {
        id: product_id,
        count: 1,
    };
    let mut steps = script_live_seed_steps();
    steps.push(Step {
        name:
            "seed woodcutting, fletching, rune axe, 26-Knife ballast and the south-bank Magic tree before Start",
        kind: StepKind::Perform {
            send: Box::new(move |c, _| {
                cheat(c, "~clearinv");
                cheat(c, "setstat woodcutting 75");
                cheat(c, &format!("setstat fletching {fletching}"));
                cheat(c, "give rune_axe 1");
                cheat(c, &format!("give knife {GNOME_BALLAST_KNIVES}"));
                cheat(c, &tele_args(stand.level, stand.x, stand.z));
                true
            }),
        },
        wait: Wait {
            arm: Proof::ArrivedNear {
                x: stand.x,
                z: stand.z,
                level: stand.level,
                radius: 1,
            },
            budget_ticks: 200,
        },
    });
    let mut seed_arms = vec![
        (
            "confirm Woodcutting 75 before Start",
            Proof::Stat {
                id: WOODCUTTING_STAT,
                min: 75,
            },
        ),
        (
            "confirm prepared Fletching before Start",
            Proof::Stat {
                id: FLETCHING_STAT,
                min: fletching,
            },
        ),
        (
            "confirm Rune axe 1359 before Start",
            Proof::ItemId {
                id: RUNE_AXE_ID,
                count: 1,
            },
        ),
        (
            "confirm exactly one Rune axe before Start",
            Proof::ItemIdAtMost {
                id: RUNE_AXE_ID,
                count: 1,
            },
        ),
        (
            "confirm exact south-bank Magic tree and Chop down action before Start",
            Proof::LocActionNear {
                id: MAGIC_TREE_ID,
                x: GNOME_SOUTH_BANK_MAGIC_TREE.x,
                z: GNOME_SOUTH_BANK_MAGIC_TREE.z,
                level: GNOME_SOUTH_BANK_MAGIC_TREE.level,
                radius: 0,
                action: "Chop down",
                present: true,
            },
        ),
        (
            "confirm 26 retained nonproduct Knives before Start",
            Proof::ItemId {
                id: KNIFE_ID,
                count: GNOME_BALLAST_KNIVES,
            },
        ),
        (
            "confirm exactly 26 retained nonproduct Knives before Start",
            Proof::ItemIdAtMost {
                id: KNIFE_ID,
                count: GNOME_BALLAST_KNIVES,
            },
        ),
        (
            "confirm no seeded magic logs in pack before Start",
            Proof::ItemIdAtMost {
                id: MAGIC_LOGS_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded unstrung product in pack before Start",
            Proof::ItemIdAtMost {
                id: product_id,
                count: 0,
            },
        ),
        (
            "confirm no seeded strung magic shortbow in pack before Start",
            Proof::ItemIdAtMost {
                id: MAGIC_SHORTBOW_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded strung magic longbow in pack before Start",
            Proof::ItemIdAtMost {
                id: MAGIC_LONGBOW_ID,
                count: 0,
            },
        ),
    ];
    if let Some(max) = fletching_max {
        seed_arms.insert(
            2,
            (
                "confirm Fletching below longbow 85 before Start",
                Proof::StatAtMost {
                    id: FLETCHING_STAT,
                    max,
                },
            ),
        );
    }
    for (step_name, arm) in seed_arms {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    steps.push(start_catalog_step());
    // The two Magic-tree roll arms get MAGIC_TREE_CHOP_WATCH_TICKS (dirty
    // increments sized to the 17/256-per-4-ticks roll, not 150×600ms).
    // Other arms keep the ordinary gold watch.
    let chop = MAGIC_TREE_CHOP_WATCH_TICKS;
    let gold = SCRIPT_GOLD_WATCH_TICKS;
    for (step_name, arm, budget_ticks) in [
        (
            "watch Woodcutting XP from a Magic tree after Start",
            first_wc,
            chop,
        ),
        (
            "watch exact Magic logs 1513 chopped after Start",
            logs,
            gold,
        ),
        ("watch Fletching XP after Start", first_fletch, gold),
        (
            "watch exact unstrung magic bow after logs are consumed",
            product,
            gold,
        ),
        (
            "watch script-fletched bows enter a fresh gnome bank",
            Proof::BankItemId {
                id: product_id,
                count: 1,
            },
            gold,
        ),
        (
            "watch the pack empty of unstrung bows after deposit",
            Proof::ItemIdAtMost {
                id: product_id,
                count: 0,
            },
            gold,
        ),
        (
            "watch ground return at the gnome bank stairs after deposit",
            Proof::ArrivedNear {
                x: GNOME_BANK_STAIR_SOUTH.x,
                z: GNOME_BANK_STAIR_SOUTH.z,
                level: GNOME_BANK_STAIR_SOUTH.level,
                radius: 30,
            },
            gold,
        ),
        (
            "watch the gnome fletch bank close after deposit",
            Proof::BankClosed,
            gold,
        ),
        (
            "watch another exact Magic logs 1513 after return",
            logs,
            chop,
        ),
    ] {
        steps.push(Step {
            name: step_name,
            kind: StepKind::Perform {
                send: Box::new(|_, _| true),
            },
            wait: Wait { arm, budget_ticks },
        });
    }
    Scenario {
        name,
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: true,
        },
        steps,
        proof: logs,
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            require_mainland_base: true,
            deadline: MAGIC_TREE_CHOP_DEADLINE,
            start_script: Some("GnomeMagicChopper"),
            script_settings_inject: Some(GNOME_FLETCH_INJECT),
            terminal_shot: Some(name),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}
