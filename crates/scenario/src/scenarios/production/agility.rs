use super::*;
pub(crate) const AGILITY_STAT: i32 = 16;
const GNOME_START: WorldTile = WorldTile {
    x: 2474,
    z: 3436,
    level: 0,
};
/// Log Walk-across dest is coord z-7 from selected gnome_course.rs2.
const GNOME_AFTER_LOG: WorldTile = WorldTile {
    x: 2474,
    z: 3429,
    level: 0,
};
/// Climb-down lands at packed 0_38_53_55_28.
const GNOME_GROUND_RETURN: WorldTile = WorldTile {
    x: 2487,
    z: 3420,
    level: 0,
};
const GNOME_PIPE: WorldTile = WorldTile {
    x: 2484,
    z: 3431,
    level: 0,
};

const WILDY_START: WorldTile = WorldTile {
    x: 2998,
    z: 3916,
    level: 0,
};
/// North of the selected inner Gate at (2998,3931). Radius 2 cannot include
/// the gate tile, so a ridge click without the world crossing fails closed.
const WILDY_AFTER_RIDGE: WorldTile = WorldTile {
    x: 2998,
    z: 3934,
    level: 0,
};
/// Selected m46_61 loc 2288 at (3004,3938); rs2 lands at loc z+9.
const WILDY_PIPE_DEST: WorldTile = WorldTile {
    x: 3004,
    z: 3947,
    level: 0,
};
/// Selected m46_61 loc 2283 at (3005,3952); rs2 lands five north of its stand.
const WILDY_ROPE_DEST: WorldTile = WorldTile {
    x: 3005,
    z: 3958,
    level: 0,
};
/// Selected m46_61 loc 2311 at (3001,3960); the sixth jump lands x-5.
const WILDY_STONE_DEST: WorldTile = WorldTile {
    x: 2996,
    z: 3960,
    level: 0,
};
/// Selected m46_61 loc 2297 is raw plane 1 over a LinkBelow bridge; the
/// player remains on observed scene plane 0 while the moves land x-7.
const WILDY_LOG_DEST: WorldTile = WorldTile {
    x: 2994,
    z: 3945,
    level: 0,
};
/// Centre tile of the selected three-wide rocks 2328; rs2 lands three south.
const WILDY_ROCKS_DEST: WorldTile = WorldTile {
    x: 2994,
    z: 3933,
    level: 0,
};

const BRIMHAVEN_ENTRANCE: WorldTile = WorldTile {
    x: 2809,
    z: 3194,
    level: 0,
};
/// Selected ladder 3617 Climb-Down destination, also arena platform 24.
const BRIMHAVEN_LADDER_LANDING: WorldTile = WorldTile {
    x: 2805,
    z: 9590,
    level: 3,
};
const AGILITY_TICKET_ID: i32 = 2996;
const AGILITY_ARENA_VARP: i32 = 309;
const GNOME_COURSE_RADIUS_INJECT: &[ScriptSettingInject] = &[ScriptSettingInject {
    id: "searchRadius",
    value: ScriptInjectValue::Num(8.0),
}];

const WILDY_AGILITY_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "acquireFoodAtStart",
        value: ScriptInjectValue::Bool(false),
    },
    ScriptSettingInject {
        id: "minFood",
        value: ScriptInjectValue::Num(0.0),
    },
];

const BRIMHAVEN_AGILITY_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "stealRestock",
        value: ScriptInjectValue::Bool(false),
    },
    ScriptSettingInject {
        id: "bankAtTickets",
        value: ScriptInjectValue::Num(1000.0),
    },
];
pub(crate) fn gnome_course_scenario() -> Scenario {
    gnome_course_variant("gnome_course", None)
}

pub(crate) fn gnome_course_radius_scenario() -> Scenario {
    gnome_course_variant("gnome_course_radius", Some(GNOME_COURSE_RADIUS_INJECT))
}

/// Cross the south ridge, complete the five selected wilderness obstacles and
/// make real progress through the next pipe. All setup cheats happen before
/// catalog Start; post-Start steps are observation-only.
pub(crate) fn wildy_agility_scenario() -> Scenario {
    let further_xp = Proof::StatXpGain {
        id: AGILITY_STAT,
        min: 598,
    };
    let mut steps = script_live_seed_steps();
    steps.push(Step {
        name: "seed Agility 52 and five Lobsters, then tele south of the ridge",
        kind: StepKind::Perform {
            send: Box::new(|c, _| {
                cheat(c, "advancestat agility 52");
                cheat(c, "setstat hitpoints 40");
                cheat(c, "give lobster 5");
                cheat(
                    c,
                    &tele_args(WILDY_START.level, WILDY_START.x, WILDY_START.z),
                );
                true
            }),
        },
        wait: Wait {
            arm: Proof::ArrivedNear {
                x: WILDY_START.x,
                z: WILDY_START.z,
                level: WILDY_START.level,
                radius: 2,
            },
            budget_ticks: 200,
        },
    });
    steps.push(bank_fletcher_watch(
        "confirm five exact Lobsters before Start",
        Proof::ItemId {
            id: LOBSTER_ID,
            count: 5,
        },
    ));
    steps.push(drain_advancestat());
    steps.push(bank_fletcher_watch(
        "confirm Hitpoints 40 before the wilderness course",
        Proof::Stat {
            id: HITPOINTS_STAT,
            min: 40,
        },
    ));
    steps.push(start_catalog_step());
    for (name, arm) in [
        (
            "watch ridge Agility XP",
            Proof::StatXpGain {
                id: AGILITY_STAT,
                min: 15,
            },
        ),
        (
            "watch the ridge world crossing north of the inner gate",
            Proof::ArrivedNear {
                x: WILDY_AFTER_RIDGE.x,
                z: WILDY_AFTER_RIDGE.z,
                level: WILDY_AFTER_RIDGE.level,
                radius: 2,
            },
        ),
        (
            "watch pipe XP after the ridge",
            Proof::StatXpGain {
                id: AGILITY_STAT,
                min: 27,
            },
        ),
        (
            "watch the selected pipe destination",
            Proof::ArrivedNear {
                x: WILDY_PIPE_DEST.x,
                z: WILDY_PIPE_DEST.z,
                level: WILDY_PIPE_DEST.level,
                radius: 3,
            },
        ),
        (
            "watch ropeswing XP after the pipe",
            Proof::StatXpGain {
                id: AGILITY_STAT,
                min: 47,
            },
        ),
        (
            "watch the selected ropeswing destination",
            Proof::ArrivedNear {
                x: WILDY_ROPE_DEST.x,
                z: WILDY_ROPE_DEST.z,
                level: WILDY_ROPE_DEST.level,
                radius: 3,
            },
        ),
        (
            "watch stepping-stone XP after the ropeswing",
            Proof::StatXpGain {
                id: AGILITY_STAT,
                min: 67,
            },
        ),
        (
            "watch the selected stepping-stone destination",
            Proof::ArrivedNear {
                x: WILDY_STONE_DEST.x,
                z: WILDY_STONE_DEST.z,
                level: WILDY_STONE_DEST.level,
                radius: 3,
            },
        ),
        (
            "watch log XP after the stepping stones",
            Proof::StatXpGain {
                id: AGILITY_STAT,
                min: 87,
            },
        ),
        (
            "watch the selected log destination",
            Proof::ArrivedNear {
                x: WILDY_LOG_DEST.x,
                z: WILDY_LOG_DEST.z,
                level: WILDY_LOG_DEST.level,
                radius: 3,
            },
        ),
        (
            "watch the five-obstacle lap XP bonus",
            Proof::StatXpGain {
                id: AGILITY_STAT,
                min: 586,
            },
        ),
        (
            "watch the selected rocks destination",
            Proof::ArrivedNear {
                x: WILDY_ROCKS_DEST.x,
                z: WILDY_ROCKS_DEST.z,
                level: WILDY_ROCKS_DEST.level,
                radius: 3,
            },
        ),
        (
            "watch the next pipe destination after the full lap",
            Proof::ArrivedNear {
                x: WILDY_PIPE_DEST.x,
                z: WILDY_PIPE_DEST.z,
                level: WILDY_PIPE_DEST.level,
                radius: 3,
            },
        ),
    ] {
        steps.push(bank_fletcher_watch(name, arm));
    }
    Scenario {
        name: "wildy_agility",
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
            start_script: Some("WildyAgility"),
            script_settings_inject: Some(WILDY_AGILITY_INJECT),
            terminal_shot: Some("wildy_agility"),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}

/// Pay and enter naturally, require obstacle XP before the script's first
/// no-ticket Tag, earn a later ticket, then require independently fresh obstacle
/// XP after that ticket.
pub(crate) fn brimhaven_agility_scenario() -> Scenario {
    let first_hop_xp = Proof::StatXpGain {
        id: AGILITY_STAT,
        min: 1,
    };
    let subsequent_xp = Proof::FreshStatXpGain {
        id: AGILITY_STAT,
        min: 1,
    };
    let mut steps = script_live_seed_steps();
    steps.push(Step {
        name: "seed Agility 52, 1000 Coins and ten Lobsters, then tele to the arena entrance",
        kind: StepKind::Perform {
            send: Box::new(|c, _| {
                cheat(c, "advancestat agility 52");
                cheat(c, "give coins 1000");
                cheat(c, "give lobster 10");
                cheat(
                    c,
                    &tele_args(
                        BRIMHAVEN_ENTRANCE.level,
                        BRIMHAVEN_ENTRANCE.x,
                        BRIMHAVEN_ENTRANCE.z,
                    ),
                );
                true
            }),
        },
        wait: Wait {
            arm: Proof::ArrivedNear {
                x: BRIMHAVEN_ENTRANCE.x,
                z: BRIMHAVEN_ENTRANCE.z,
                level: BRIMHAVEN_ENTRANCE.level,
                radius: 2,
            },
            budget_ticks: 200,
        },
    });
    steps.push(drain_advancestat());
    for (name, arm) in [
        (
            "confirm 1000 exact Coins before Start",
            Proof::ItemId {
                id: COINS_ID,
                count: 1000,
            },
        ),
        (
            "confirm ten exact Lobsters before Start",
            Proof::ItemId {
                id: LOBSTER_ID,
                count: 10,
            },
        ),
        (
            "confirm no seeded agility-arena ticket before Start",
            Proof::ItemIdAtMost {
                id: AGILITY_TICKET_ID,
                count: 0,
            },
        ),
    ] {
        steps.push(bank_fletcher_watch(name, arm));
    }
    steps.push(start_catalog_step());
    for (name, arm) in [
        (
            "watch the 200-Coin arena fee",
            Proof::ItemIdAtMost {
                id: COINS_ID,
                count: 800,
            },
        ),
        (
            "watch the arena paid bit",
            Proof::Varp {
                id: AGILITY_ARENA_VARP,
                min: 2,
            },
        ),
        (
            "watch Climb-Down reach the arena ladder platform",
            Proof::ArrivedNear {
                x: BRIMHAVEN_LADDER_LANDING.x,
                z: BRIMHAVEN_LADDER_LANDING.z,
                level: BRIMHAVEN_LADDER_LANDING.level,
                radius: 2,
            },
        ),
        (
            "watch obstacle XP from a real hop before the first Tag",
            first_hop_xp,
        ),
        (
            "watch the first Tag prompt for the next pillar",
            Proof::Chat {
                needle: "tag the next",
            },
        ),
        (
            "watch the first Tag set the tagged bit",
            Proof::Varp {
                id: AGILITY_ARENA_VARP,
                min: 15,
            },
        ),
        (
            "confirm the first Tag grants no ticket",
            Proof::ItemIdAtMost {
                id: AGILITY_TICKET_ID,
                count: 0,
            },
        ),
        (
            "watch a later Tag grant the first ticket",
            Proof::ItemId {
                id: AGILITY_TICKET_ID,
                count: 1,
            },
        ),
        (
            "watch subsequent arena obstacle XP after the ticket",
            subsequent_xp,
        ),
    ] {
        steps.push(bank_fletcher_watch(name, arm));
    }
    Scenario {
        name: "brimhaven_agility",
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: true,
        },
        steps,
        proof: subsequent_xp,
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            require_mainland_base: true,
            deadline: SCRIPT_GOLD_DEADLINE,
            start_script: Some("BrimhavenAgility"),
            script_settings_inject: Some(BRIMHAVEN_AGILITY_INJECT),
            terminal_shot: Some("brimhaven_agility"),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}

/// Complete a natural gnome lap, then cross the log again. Selected 274/289
/// content grants 86.5 XP per lap plus 7.5 for the next log. Stored milestones
/// are on the ground; snapshot levels follow the actual client plane.
fn gnome_course_variant(
    name: &'static str,
    inject: Option<&'static [ScriptSettingInject]>,
) -> Scenario {
    let first_xp = Proof::StatXpGain {
        id: AGILITY_STAT,
        min: 1,
    };
    let further_xp = Proof::StatXpGain {
        id: AGILITY_STAT,
        min: 94,
    };
    let start = GNOME_START;
    let mut steps = script_live_seed_steps();
    steps.push(Step {
        name: "tele to the gnome course start before Start",
        kind: StepKind::Perform {
            send: Box::new(move |c, _| {
                cheat(c, &tele_args(start.level, start.x, start.z));
                true
            }),
        },
        wait: Wait {
            arm: Proof::ArrivedNear {
                x: start.x,
                z: start.z,
                level: start.level,
                radius: 2,
            },
            budget_ticks: 200,
        },
    });
    steps.push(start_catalog_step());
    for (step_name, arm) in [
        ("watch Agility XP from the first obstacle", first_xp),
        (
            "watch the log dest tile after Walk-across",
            Proof::ArrivedNear {
                x: GNOME_AFTER_LOG.x,
                z: GNOME_AFTER_LOG.z,
                level: GNOME_AFTER_LOG.level,
                radius: 3,
            },
        ),
        (
            "watch the selected climb-down ground return",
            Proof::ArrivedNear {
                x: GNOME_GROUND_RETURN.x,
                z: GNOME_GROUND_RETURN.z,
                level: GNOME_GROUND_RETURN.level,
                radius: 3,
            },
        ),
        (
            "watch the obstacle pipe after the ground nets",
            Proof::ArrivedNear {
                x: GNOME_PIPE.x,
                z: GNOME_PIPE.z,
                level: GNOME_PIPE.level,
                radius: 6,
            },
        ),
        (
            "watch further Agility XP at the start of a second lap",
            further_xp,
        ),
        (
            "watch the log dest after second-lap progress",
            Proof::ArrivedNear {
                x: GNOME_AFTER_LOG.x,
                z: GNOME_AFTER_LOG.z,
                level: GNOME_AFTER_LOG.level,
                radius: 3,
            },
        ),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
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
            start_script: Some("GnomeCourse"),
            script_settings_inject: inject,
            terminal_shot: Some(name),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}
