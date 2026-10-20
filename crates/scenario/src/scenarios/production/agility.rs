use super::*;
use api::snapshot::LocView;
use std::sync::Mutex;
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
/// Search radius the radius cell injects, in tiles.
const GNOME_COURSE_RADIUS: i32 = 10;
/// The card's own default `searchRadius` (`AgilityBot.ts:34`).
const GNOME_DEFAULT_SEARCH_RADIUS: i32 = 20;
/// AgilityBot `find` keeps a loc within `searchRadius` Chebyshev tiles of the
/// player, measured to the loc origin (`AgilityBot.ts:141-145`,
/// `ClientAdapter.ts:984`). The longest course gap is the lap wrap: the pipe
/// exit (2484,3437) to the Log balance loc 2295 at (2474,3435) is 10. With 8
/// the script re-syncs to the pipe loc 154 at (2484,3435), which
/// `gnome_course.rs2` `[oploc1,loc_154]` refuses from the north, and every
/// reposition face is blocked or inside the fenced pipe pen. 10 is the
/// smallest radius that completes a lap and stays below the default 20. The
/// witness at the start of the cell (`GNOME_RADIUS_WITNESS_TILE`) covers the
/// other side of that boundary: an obstacle 11 tiles away stays untouched.
const GNOME_COURSE_RADIUS_INJECT: &[ScriptSettingInject] = &[ScriptSettingInject {
    id: "searchRadius",
    value: ScriptInjectValue::Num(GNOME_COURSE_RADIUS as f64),
}];
/// Where the radius cell starts the bot, so that the search radius is the only
/// thing between the bot and the course.
/// - `TravelToCourse` walks the bot back only when it stands more than 20
///   tiles from the course start (`AgilityBot.ts:19-25,113-120`). This tile is
///   10 away, so the bot keeps searching where it stands.
/// - The loc search lists only the player's plane (`ClientAdapter.ts:953`).
///   Level 0 holds these locs named like a course step, each at the origin
///   tile the client lists (`content/maps/m38_53.jm2`, mapsquare origin
///   2432,3392): Log balance 2295 at (2474,3435), raw plane 1 over a bridge
///   tile so plane 0 (`:6606`, `:4339`); Obstacle net 2285 at
///   (2471|2473|2475,3425) (`:5568`, `:5642`, `:5729`) and 2286 at
///   (2483|2485|2487,3426) (`:5993`, `:6055`, `:6107`); Obstacle pipe 154 at
///   (2484,3431|3435) (`:6029`, `:6030`) and 4058 at (2487,3431|3435) (`:6110`,
///   `:6111`; the 289 cache names it like 154). The tree branches and the rope
///   are on planes 1 and 2.
/// - The nearest of those to this tile are the Log balance (11 south) and pipe
///   154 at (2484,3435) (10 east, 11 south). Radius 10 sees none of them and
///   every radius from 11 up sees the log. Together with the lap, which needs
///   10, the cell passes for radius 10 only.
/// - The tile is open ground (`m38_53.jm2:2744`). The nav pack bake-r2-a calls
///   it walkable and routes it to the course start in one 13-game-tick walk leg,
///   and the engine accepts a loc click from anywhere in the scene (engine
///   `OpLocHandler.ts:20-33`), so a bot that does click the log walks off.
pub(crate) const GNOME_RADIUS_WITNESS_TILE: WorldTile = WorldTile {
    x: 2474,
    z: 3446,
    level: 0,
};
/// Observed game ticks (`GameSnapshot::tick`, one per PLAYER_INFO) the witness
/// must see pass without the bot acting. A bot that ignores the radius clicks
/// the Log balance in its first `loop()`. `onStart` parks one `delayUntil`
/// (`AgilityBot.ts:50`, `crates/script/src/shim/execution.js:73-82`), so the
/// first `loop()` runs one game tick after the isolate's first game tick. Its
/// click leaves in that same game tick
/// (`crates/script/src/load/isolate/thread.rs:1057-1094`).
/// The engine applies a click in the game tick it receives it, and that tick's
/// PLAYER_INFO already shows the first step (engine `World.ts:352-378,397-408`).
/// That is 3 game ticks after the isolate's first game tick; 12 is four times
/// that, so it also covers an isolate that needs up to 9 game ticks (5.4 s) to
/// come up after Start.
pub(crate) const GNOME_RADIUS_WITNESS_GAME_TICKS: u32 = 12;
/// Receipt name of the radius witness step.
pub(crate) const GNOME_RADIUS_WITNESS_EVIDENCE: &str =
    "gnome_course_log_balance_left_alone_beyond_the_search_radius";

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
    gnome_course_variant("gnome_course", None, false)
}

pub(crate) fn gnome_course_radius_scenario() -> Scenario {
    gnome_course_variant(
        "gnome_course_radius",
        Some(GNOME_COURSE_RADIUS_INJECT),
        true,
    )
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
///
/// With `radius_witness` the bot starts off the course instead, 11 tiles from
/// the Log balance, and must leave it alone (`GnomeRadiusWitness`) before the
/// cell teleports it to the course start for the lap. `StatXpGain` baselines
/// are taken when the Start step begins (`runner.rs` `begin_step`), whatever
/// the position, and the witness refuses any Agility XP, so every XP watch
/// keeps its meaning.
fn gnome_course_variant(
    name: &'static str,
    inject: Option<&'static [ScriptSettingInject]>,
    radius_witness: bool,
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
    let start_arm = Proof::ArrivedNear {
        x: start.x,
        z: start.z,
        level: start.level,
        radius: 2,
    };
    if radius_witness {
        let off_course = GNOME_RADIUS_WITNESS_TILE;
        // The first tele keeps the budget of the plain cell's: both leave the
        // seed's landing tile, so the arrival may wait for a scene load.
        steps.push(gnome_tele_step(
            "tele off the course, 11 tiles from the Log balance, before Start",
            off_course,
            Proof::Arrived {
                x: off_course.x,
                z: off_course.z,
                level: off_course.level,
            },
            200,
        ));
        steps.push(start_catalog_step());
        steps.push(gnome_radius_witness_step());
        // Arrival shows on the PLAYER_INFO of the tick that applies the cheat,
        // so the standing post-Start watch budget is only a miss detector.
        steps.push(gnome_tele_step(
            "tele to the gnome course start after the witness",
            start,
            start_arm,
            SCRIPT_GOLD_WATCH_TICKS,
        ));
    } else {
        steps.push(gnome_tele_step(
            "tele to the gnome course start before Start",
            start,
            start_arm,
            200,
        ));
        steps.push(start_catalog_step());
    }
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

/// A cheat teleport to `tile`, done when `arm` holds.
fn gnome_tele_step(name: &'static str, tile: WorldTile, arm: Proof, budget_ticks: u32) -> Step {
    Step {
        name,
        kind: StepKind::Perform {
            send: Box::new(move |c, _| {
                cheat(c, &tele_args(tile.level, tile.x, tile.z));
                true
            }),
        },
        wait: Wait { arm, budget_ticks },
    }
}

/// Obstacle names of the card's default course, lower case and each once
/// (`AgilityBot.ts:30`). `find` compares the lower-cased loc name
/// (`AgilityBot.ts:144`).
const GNOME_COURSE_NAMES: [&str; 5] = [
    "log balance",
    "obstacle net",
    "tree branch",
    "balancing rope",
    "obstacle pipe",
];

/// Selected `gnome_log_balance1`, the first obstacle
/// (`content/pack/loc.pack:2296`).
const GNOME_LOG_BALANCE_ID: i32 = 2295;

/// Whether the bot's own obstacle search could take `loc`: a course name and at
/// least one action (`AgilityBot.ts:144`; `presentOps` drops null and `hidden`,
/// `Interactable.ts:32-34`). The distance test is the caller's.
fn is_gnome_course_obstacle(loc: &LocView) -> bool {
    loc.name.as_deref().is_some_and(|name| {
        GNOME_COURSE_NAMES
            .iter()
            .any(|course| name.eq_ignore_ascii_case(course))
    }) && loc
        .actions
        .iter()
        .flatten()
        .any(|action| action != "hidden")
}

/// What the radius witness has seen of one slot.
#[derive(Default)]
struct GnomeRadiusWitness {
    /// Agility XP on the first game tick the bot stood on the witness tile;
    /// `None` until then.
    xp: Option<i32>,
    /// Latest observed `GameSnapshot::tick`. The runner calls `ready` on every
    /// client frame, so repeated observations of the same game tick do not
    /// count as new observations.
    last_observed_game_tick: Option<u32>,
    /// Consecutive judged game ticks on which every condition held.
    still_game_ticks: u32,
    /// The bot walked off the tile or earned XP after the watch began, so it
    /// acted on an obstacle outside its radius. Never cleared: a bot that
    /// walks back onto the tile is still refused.
    refused: bool,
}

impl GnomeRadiusWitness {
    /// One verdict per observed game tick. Accepts after
    /// `GNOME_RADIUS_WITNESS_GAME_TICKS` consecutive game ticks on which the
    /// bot stood on the witness tile and earned no Agility XP, the scene listed
    /// the Log balance beyond the injected radius but within the default one,
    /// and no loc the bot's own search could take was inside the injected radius.
    /// Distances are Chebyshev from the player tile to the loc origin tile
    /// (`ClientAdapter.ts:984`), computed here from the tiles.
    fn observe(&mut self, snapshot: &GameSnapshot) -> bool {
        if self.refused {
            return false;
        }
        let position = snapshot
            .tile()
            .filter(|_| snapshot.ingame() && snapshot.scene_state() == api::interact::SCENE_READY);
        let Some((x, z, level)) = position else {
            self.still_game_ticks = 0;
            return false;
        };
        if self.last_observed_game_tick == Some(snapshot.tick()) {
            return self.still_game_ticks >= GNOME_RADIUS_WITNESS_GAME_TICKS;
        }
        self.last_observed_game_tick = Some(snapshot.tick());
        let tile = GNOME_RADIUS_WITNESS_TILE;
        if (x, z, level) != (tile.x, tile.z, tile.level) {
            // Before the first game tick on the tile this is the teleport still
            // landing. After it, the bot has walked away.
            self.refused = self.xp.is_some();
            self.still_game_ticks = 0;
            return false;
        }
        let Some(xp) = snapshot
            .stats()
            .iter()
            .find(|stat| stat.index == AGILITY_STAT)
            .map(|stat| stat.xp)
        else {
            self.still_game_ticks = 0;
            return false;
        };
        let baseline = *self.xp.get_or_insert(xp);
        if xp > baseline {
            self.refused = true;
            self.still_game_ticks = 0;
            return false;
        }
        let distance = |loc: &LocView| (loc.tile.x - x).abs().max((loc.tile.z - z).abs());
        let log = snapshot
            .locs()
            .iter()
            .filter(|loc| loc.tile.level == level)
            .find(|loc| loc.id == GNOME_LOG_BALANCE_ID)
            .map(distance);
        let nearest = snapshot
            .locs()
            .iter()
            .filter(|loc| loc.tile.level == level && is_gnome_course_obstacle(loc))
            .map(distance)
            .min();
        let log_beyond_radius = log.is_some_and(|d| d <= GNOME_DEFAULT_SEARCH_RADIUS)
            && nearest.is_some_and(|d| d > GNOME_COURSE_RADIUS);
        self.still_game_ticks = if log_beyond_radius {
            self.still_game_ticks + 1
        } else {
            0
        };
        self.still_game_ticks >= GNOME_RADIUS_WITNESS_GAME_TICKS
    }
}

/// The witness step, right after Start: the bot must leave the Log balance
/// alone for `GNOME_RADIUS_WITNESS_GAME_TICKS` observed game ticks (one per
/// `PLAYER_INFO`). A bot that ignores the radius walks off the tile, which
/// refuses the witness for good, so the step can then only time out and fail.
/// The earlier success path took about 15 dirty snapshots while 12 game ticks
/// elapsed; those snapshots reflect client-cycle polling frequency, not the
/// witness clock.
/// The standing post-Start watch budget only runs out when the witness was
/// refused or its conditions never held. `Await` ignores the arm; it is the
/// placeholder the other `Await` steps use.
fn gnome_radius_witness_step() -> Step {
    let witness = Mutex::new(GnomeRadiusWitness::default());
    Step {
        name: "watch the bot leave the Log balance alone beyond its search radius",
        kind: StepKind::Await {
            evidence: GNOME_RADIUS_WITNESS_EVIDENCE,
            ready: Box::new(move |snapshot| {
                witness
                    .lock()
                    .expect("gnome radius witness mutex")
                    .observe(snapshot)
            }),
        },
        wait: Wait {
            arm: Proof::Stat { id: 16, min: 0 },
            budget_ticks: SCRIPT_GOLD_WATCH_TICKS,
        },
    }
}
