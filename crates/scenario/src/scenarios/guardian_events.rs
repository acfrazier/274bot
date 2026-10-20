//! Random-event Guardian regression cells: every random event, summoned next
//! to a low-level account doing real work, judged by
//! [`crate::random_episode`].
//!
//! Each cell seeds only the account's own state (stats, tool, position),
//! starts a real activity on real content (the compiled Gatherer fishing at
//! Catherby, mining at Varrock East, chopping west of Lumbridge or oaks at
//! Draynor; the catalog
//! BoneBurier and Thiever cards at Lumbridge), waits for the activity's
//! first XP, then summons one event through the engine's own random-event
//! machinery: `::macro_event <id>` for the eight general events, or the
//! `::skill_macro_event <id>` test cheat (engine patch
//! [`crate::random_episode::SKILL_EVENT_PATCH`]) for the skill events.
//! Operator decision 2026-10-14: a random event always appears beside the
//! player, so how it was triggered must not matter to the host.
//!
//! The account keeps the engine's fresh-account combat stats (combat level
//! 3, 10 Hitpoints), like the tester's fishers: troll, golem, spirit, shade,
//! zombie and watchman tiers scale with combat level
//! (`macro_events.rs2` `~macro_event_combat_level`).

use super::gatherer::GATHERER_START_INJECT;
use super::script_basics::script_live_seed_steps;
use crate::random_episode::{Oracle, RandomEventCell, Reward, TrapKind, Trigger};
use crate::*;

const PRAYER_STAT: i32 = 5;
const WOODCUTTING_STAT: i32 = 8;
const FISHING_STAT: i32 = 10;
const MINING_STAT: i32 = 14;
const THIEVING_STAT: i32 = 17;

const HARPOON_ID: i32 = 311;
const BRONZE_PICKAXE_ID: i32 = 1265;
const BRONZE_AXE_ID: i32 = 1351;
const BONES_ID: i32 = 526;
const KEBAB_ID: i32 = 1971;
const BEER_ID: i32 = 1917;

/// The host-play harpoon cells' Catherby stand (`gatherer_live.rs`
/// `CATHERBY_HARPOON_START`), on the shore within eight tiles of the
/// `0_40_53_rarefish` Cage/Harpoon spots, which carry the whirlpool type
/// (`skill_fishing/configs/fishing.npc:128-146`).
pub(crate) const CATHERBY_HARPOON_START: WorldTile = WorldTile {
    x: 2840,
    z: 3436,
    level: 0,
};
/// Inside the selected-289 Varrock East mine, beside its copper and tin
/// rocks (all of which carry the gas type, `skill_mining/configs/rocks.loc`).
pub(crate) const VARROCK_EAST_MINE_START: WorldTile = WorldTile {
    x: 3287,
    z: 3365,
    level: 0,
};

const FISHING_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "skill",
        value: ScriptInjectValue::Str("Fishing"),
    },
    ScriptSettingInject {
        id: "location",
        value: ScriptInjectValue::Str("Start"),
    },
    ScriptSettingInject {
        id: "fishingMethod",
        value: ScriptInjectValue::Str("fishing.rarefish.op3"),
    },
    ScriptSettingInject {
        id: "radius",
        value: ScriptInjectValue::Num(12.0),
    },
    ScriptSettingInject {
        id: "disposition",
        value: ScriptInjectValue::Str("Power"),
    },
];

/// Normal trees west of Lumbridge castle, for the tree spirit and lost axe
/// cells. The Draynor stand of the `gatherer` cell lies beside the Draynor
/// jail, whose Jail guards (`area_draynor/configs/draynor.npc:152-182`,
/// `huntmode=aggressive_melee`, `huntrange=5`) attacked the combat-level-3
/// account in live round 3 and a diagnostic run, whatever the event.
pub(crate) const LUMBRIDGE_TREES_START: WorldTile = WorldTile {
    x: 3168,
    z: 3226,
    level: 0,
};

/// Draynor's oak grove east of the bank (`gatherer_live.rs`
/// `DRAYNOR_OAK_BANK_START`), seven tiles from the nearest oak. The ent cell
/// chops oaks: a normal tree always falls with its first log, so right after
/// the first XP the tree beside the bot is often a stump, which has no ent
/// type (live round 1). An oak falls with a log only one time in eight
/// (`skill_woodcutting/scripts/woodcut.rs2:133-149`).
pub(crate) const DRAYNOR_OAK_START: WorldTile = WorldTile {
    x: 3095,
    z: 3243,
    level: 0,
};

const OAK_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "skill",
        value: ScriptInjectValue::Str("Woodcutting"),
    },
    ScriptSettingInject {
        id: "location",
        value: ScriptInjectValue::Str("Start"),
    },
    ScriptSettingInject {
        id: "woodcuttingResources",
        value: ScriptInjectValue::StrList(&["oak"]),
    },
    ScriptSettingInject {
        id: "radius",
        value: ScriptInjectValue::Num(12.0),
    },
    ScriptSettingInject {
        id: "disposition",
        value: ScriptInjectValue::Str("Power"),
    },
];

const MINING_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "skill",
        value: ScriptInjectValue::Str("Mining"),
    },
    ScriptSettingInject {
        id: "location",
        value: ScriptInjectValue::Str("Start"),
    },
    ScriptSettingInject {
        id: "miningResources",
        value: ScriptInjectValue::StrList(&["copper", "tin"]),
    },
    ScriptSettingInject {
        id: "radius",
        value: ScriptInjectValue::Num(12.0),
    },
    ScriptSettingInject {
        id: "disposition",
        value: ScriptInjectValue::Str("Power"),
    },
];

/// ThievingBot's default target (`ThievingBot.ts:41`), no banking, no loot,
/// and the food loadout the card requires before it starts (round 1: "The
/// saved loadout setting \"\" cannot select a loadout").
const THIEVING_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "loadout",
        value: ScriptInjectValue::Str("Guardian food"),
    },
    ScriptSettingInject {
        id: "target",
        value: ScriptInjectValue::Str("Man"),
    },
    ScriptSettingInject {
        id: "banking",
        value: ScriptInjectValue::Str("None"),
    },
    ScriptSettingInject {
        id: "loot",
        value: ScriptInjectValue::Str(""),
    },
];

/// Posted at catalog Start: the Thiever's food, as the `thiever` cell does.
const THIEVING_LOADOUTS: &[FixtureLoadout] = &[FixtureLoadout {
    name: "Guardian food",
    carry: &[("Lobster", 1)],
}];
const LOBSTER_ID: i32 = 379;

/// The real activity a cell runs while its event arrives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Activity {
    Fishing,
    Mining,
    Woodcutting,
    OakWoodcutting,
    Prayer,
    Thieving,
}

impl Activity {
    pub(crate) const fn stat(self) -> i32 {
        match self {
            Self::Fishing => FISHING_STAT,
            Self::Mining => MINING_STAT,
            Self::Woodcutting | Self::OakWoodcutting => WOODCUTTING_STAT,
            Self::Prayer => PRAYER_STAT,
            Self::Thieving => THIEVING_STAT,
        }
    }

    const fn start_script(self) -> &'static str {
        match self {
            Self::Fishing | Self::Mining | Self::Woodcutting | Self::OakWoodcutting => "Gatherer",
            Self::Prayer => "BoneBurier",
            Self::Thieving => "Thiever",
        }
    }

    const fn inject(self) -> Option<&'static [ScriptSettingInject]> {
        match self {
            Self::Fishing => Some(FISHING_INJECT),
            Self::Mining => Some(MINING_INJECT),
            Self::Woodcutting => Some(GATHERER_START_INJECT),
            Self::OakWoodcutting => Some(OAK_INJECT),
            Self::Prayer => None,
            Self::Thieving => Some(THIEVING_INJECT),
        }
    }

    const fn start(self) -> WorldTile {
        match self {
            Self::Fishing => CATHERBY_HARPOON_START,
            Self::Mining => VARROCK_EAST_MINE_START,
            Self::Woodcutting => LUMBRIDGE_TREES_START,
            Self::OakWoodcutting => DRAYNOR_OAK_START,
            Self::Prayer | Self::Thieving => MAINLAND_LANDING,
        }
    }

    /// Own-state seed cheats: the skill level and the activity's tool or
    /// stock. Combat stats stay at the fresh account's.
    fn seed(self, client: &mut Client) -> bool {
        let cheats: &[&str] = match self {
            // The host-play harpoon cells' level for tuna and swordfish.
            Self::Fishing => &["setstat fishing 76", "give harpoon 1"],
            Self::Mining => &["setstat mining 1", "give bronze_pickaxe 1"],
            Self::Woodcutting => &["setstat woodcutting 1", "give bronze_axe 1"],
            Self::OakWoodcutting => &["setstat woodcutting 15", "give bronze_axe 1"],
            Self::Prayer => &["give bones 28"],
            Self::Thieving => &["setstat thieving 1", "give lobster 5"],
        };
        cheats.iter().all(|cmd| cheat(client, cmd).is_sent())
    }

    const fn seeded_item(self) -> i32 {
        match self {
            Self::Fishing => HARPOON_ID,
            Self::Mining => BRONZE_PICKAXE_ID,
            Self::Woodcutting | Self::OakWoodcutting => BRONZE_AXE_ID,
            Self::Prayer => BONES_ID,
            Self::Thieving => LOBSTER_ID,
        }
    }

    /// The tool the Gatherer wields for the activity, if any.
    const fn wielded_tool(self) -> Option<i32> {
        match self {
            Self::Mining => Some(BRONZE_PICKAXE_ID),
            Self::Woodcutting | Self::OakWoodcutting => Some(BRONZE_AXE_ID),
            _ => None,
        }
    }
}

/// Observed game-tick cap for the activity's first XP before the trigger
/// (one per `PLAYER_INFO`; see the existing Gatherer watch).
const ACTIVITY_WATCH_TICKS: u32 = 240;
///
/// Observed game-tick cap for the whole episode: event, Guardian handling and
/// first fresh activity XP after it. Client cycles and dirty snapshots do not
/// count. Event content bounds also use server game ticks: Evade has the
/// Swarm's bounded 25-game-tick flee plus return and catch (~120 game ticks);
/// the genie's three warnings are 50 game ticks apart; Maze uses twice the
/// event limit (`host/src/random/maze.rs` `SOLVE_TICKS`). The `maze_owned`
/// cell watches 900 game ticks.
const fn episode_budget(oracle: Oracle) -> u32 {
    match oracle {
        Oracle::Trap(TrapKind::Maze) => 1000,
        Oracle::Trap(TrapKind::Mime) => 700,
        Oracle::Dialog { .. } | Oracle::Pick => 500,
        _ => 400,
    }
}

/// One live cell: real activity, then one summoned event.
pub(crate) fn guardian_event_cell(
    name: &'static str,
    activity: Activity,
    event: &'static str,
    trigger: Trigger,
    oracle: Oracle,
) -> Scenario {
    let start = activity.start();
    let mut steps = script_live_seed_steps();
    steps.push(Step {
        name: "seed the activity's skill and tool or stock",
        kind: StepKind::Perform {
            send: Box::new(move |client, _| activity.seed(client)),
        },
        wait: Wait {
            arm: Proof::ItemId {
                id: activity.seeded_item(),
                count: 1,
            },
            budget_ticks: 120,
        },
    });
    if let Some(tool) = activity.wielded_tool() {
        steps.push(Step {
            name: "wield the seeded tool",
            kind: StepKind::Perform {
                send: Box::new(move |client, snapshot| {
                    matches!(
                        Interactions::new(snapshot, client).wear(tool),
                        SendResult::Sent { .. }
                    )
                }),
            },
            wait: Wait {
                arm: Proof::EquipmentId { id: tool },
                budget_ticks: 120,
            },
        });
    }
    steps.push(drain_advancestat());
    steps.push(Step {
        name: "stand at the real activity site",
        kind: StepKind::Perform {
            send: Box::new(move |client, _| {
                cheat(client, &tele_args(start.level, start.x, start.z)).is_sent()
            }),
        },
        wait: Wait {
            arm: Proof::Arrived {
                x: start.x,
                z: start.z,
                level: start.level,
            },
            budget_ticks: 120,
        },
    });
    steps.push(match activity {
        Activity::Prayer | Activity::Thieving => start_catalog_step(),
        _ => start_compiled_step(),
    });
    steps.push(Step {
        name: "watch the activity's first XP before the event",
        kind: StepKind::Perform {
            send: Box::new(|_, _| true),
        },
        wait: Wait {
            arm: Proof::FreshStatXpGain {
                id: activity.stat(),
                min: 1,
            },
            budget_ticks: ACTIVITY_WATCH_TICKS,
        },
    });
    let budget = episode_budget(oracle);
    steps.push(Step {
        name: "summon the event and observe the Guardian handle it",
        kind: StepKind::ObserveRandomEvent {
            cell: RandomEventCell {
                event,
                trigger,
                oracle,
                skill: activity.stat(),
            },
        },
        wait: Wait {
            arm: Proof::IngameScene2,
            budget_ticks: budget,
        },
    });
    Scenario {
        name,
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: true,
        },
        steps,
        proof: Proof::IngameScene2,
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            require_mainland_base: true,
            // Seed, Start and first XP take about two minutes; the episode
            // budget runs at about two dirties a second.
            deadline: Duration::from_secs(180 + budget as u64 / 2),
            start_script: Some(activity.start_script()),
            script_settings_inject: activity.inject(),
            fixture_loadouts: (activity == Activity::Thieving).then_some(THIEVING_LOADOUTS),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}

/// `(scenario name, activity, event, trigger, oracle)` for every cell.
pub(crate) const GUARDIAN_EVENT_CELLS: &[(&str, Activity, &str, Trigger, Oracle)] = &[
    // General events (`macro_events.constant`), while fishing at Catherby.
    (
        "guardian_event_swarm",
        Activity::Fishing,
        "swarm",
        Trigger::General(1),
        Oracle::Evade { npc: "Swarm" },
    ),
    (
        "guardian_event_genie",
        Activity::Fishing,
        "genie",
        Trigger::General(2),
        Oracle::Dialog {
            npc: "Genie",
            reward: Reward::Lamp,
        },
    ),
    (
        "guardian_event_drunken_dwarf",
        Activity::Fishing,
        "drunken dwarf",
        Trigger::General(3),
        Oracle::Dialog {
            npc: "Drunken dwarf",
            reward: Reward::AnyOf(&[KEBAB_ID, BEER_ID]),
        },
    ),
    (
        "guardian_event_old_man",
        Activity::Fishing,
        "mysterious old man",
        Trigger::General(4),
        Oracle::Dialog {
            npc: "Mysterious Old Man",
            reward: Reward::NewItem,
        },
    ),
    (
        "guardian_event_strange_plant",
        Activity::Fishing,
        "strange plant",
        Trigger::General(5),
        Oracle::Pick,
    ),
    (
        "guardian_event_strange_box",
        Activity::Fishing,
        "strange box",
        Trigger::General(6),
        Oracle::StrangeBox,
    ),
    (
        "guardian_event_mime",
        Activity::Fishing,
        "mime",
        Trigger::General(7),
        Oracle::Trap(TrapKind::Mime),
    ),
    (
        "guardian_event_maze",
        Activity::Fishing,
        "maze",
        Trigger::General(8),
        Oracle::Trap(TrapKind::Maze),
    ),
    // Skill events (`::skill_macro_event`), in their own activity.
    (
        "guardian_skill_big_fish",
        Activity::Fishing,
        "big fish",
        Trigger::Skill(1),
        Oracle::LostGear {
            gear_id: HARPOON_ID,
        },
    ),
    (
        "guardian_skill_river_troll",
        Activity::Fishing,
        "river troll",
        Trigger::Skill(2),
        Oracle::Evade { npc: "River troll" },
    ),
    (
        "guardian_skill_whirlpool",
        Activity::Fishing,
        "whirlpool",
        Trigger::Skill(3),
        Oracle::Endure {
            tool_ids: &[HARPOON_ID],
        },
    ),
    (
        "guardian_skill_gas",
        Activity::Mining,
        "gas",
        Trigger::Skill(4),
        Oracle::Endure {
            tool_ids: &[BRONZE_PICKAXE_ID],
        },
    ),
    (
        "guardian_skill_lost_pickaxe",
        Activity::Mining,
        "lost pickaxe",
        Trigger::Skill(5),
        Oracle::LostTool {
            tool_ids: &[BRONZE_PICKAXE_ID],
        },
    ),
    (
        "guardian_skill_rock_golem",
        Activity::Mining,
        "rock golem",
        Trigger::Skill(6),
        Oracle::Evade { npc: "Rock Golem" },
    ),
    (
        "guardian_skill_tree_spirit",
        Activity::Woodcutting,
        "tree spirit",
        Trigger::Skill(7),
        Oracle::Evade { npc: "Tree spirit" },
    ),
    (
        "guardian_skill_ent",
        Activity::OakWoodcutting,
        "ent",
        Trigger::Skill(8),
        Oracle::Endure {
            tool_ids: &[BRONZE_AXE_ID],
        },
    ),
    (
        "guardian_skill_lost_axe",
        Activity::Woodcutting,
        "lost axe",
        Trigger::Skill(9),
        Oracle::LostTool {
            tool_ids: &[BRONZE_AXE_ID],
        },
    ),
    (
        "guardian_skill_shade",
        Activity::Prayer,
        "shade",
        Trigger::Skill(10),
        Oracle::Evade { npc: "Shade" },
    ),
    (
        "guardian_skill_zombie",
        Activity::Prayer,
        "zombie",
        Trigger::Skill(11),
        Oracle::Evade { npc: "Zombie" },
    ),
    (
        "guardian_skill_watchman",
        Activity::Thieving,
        "watchman",
        Trigger::Skill(12),
        Oracle::Evade { npc: "Watchman" },
    ),
];

/// Build the named cell from [`GUARDIAN_EVENT_CELLS`].
pub(crate) fn guardian_cell(name: &str) -> Scenario {
    let (name, activity, event, trigger, oracle) = *GUARDIAN_EVENT_CELLS
        .iter()
        .find(|cell| cell.0 == name)
        .unwrap_or_else(|| panic!("{name} is not a guardian event cell"));
    guardian_event_cell(name, activity, event, trigger, oracle)
}

macro_rules! guardian_cell_factories {
    ($($fn_name:ident => $cell:literal),* $(,)?) => {
        $(
            pub(crate) fn $fn_name() -> Scenario {
                guardian_cell($cell)
            }
        )*
    };
}

guardian_cell_factories! {
    guardian_event_swarm_scenario => "guardian_event_swarm",
    guardian_event_genie_scenario => "guardian_event_genie",
    guardian_event_drunken_dwarf_scenario => "guardian_event_drunken_dwarf",
    guardian_event_old_man_scenario => "guardian_event_old_man",
    guardian_event_strange_plant_scenario => "guardian_event_strange_plant",
    guardian_event_strange_box_scenario => "guardian_event_strange_box",
    guardian_event_mime_scenario => "guardian_event_mime",
    guardian_event_maze_scenario => "guardian_event_maze",
    guardian_skill_big_fish_scenario => "guardian_skill_big_fish",
    guardian_skill_river_troll_scenario => "guardian_skill_river_troll",
    guardian_skill_whirlpool_scenario => "guardian_skill_whirlpool",
    guardian_skill_gas_scenario => "guardian_skill_gas",
    guardian_skill_lost_pickaxe_scenario => "guardian_skill_lost_pickaxe",
    guardian_skill_rock_golem_scenario => "guardian_skill_rock_golem",
    guardian_skill_tree_spirit_scenario => "guardian_skill_tree_spirit",
    guardian_skill_ent_scenario => "guardian_skill_ent",
    guardian_skill_lost_axe_scenario => "guardian_skill_lost_axe",
    guardian_skill_shade_scenario => "guardian_skill_shade",
    guardian_skill_zombie_scenario => "guardian_skill_zombie",
    guardian_skill_watchman_scenario => "guardian_skill_watchman",
}

#[cfg(test)]
#[path = "guardian_events_tests.rs"]
mod tests;
