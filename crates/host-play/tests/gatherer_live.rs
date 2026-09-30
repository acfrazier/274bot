//! Real-Play qualification cells for the stage-1 Gatherer card.
//!
//! The cells use the operator Start path: a loopback profile is bound, a
//! slot is spawned, `Play::script_start` prepares and commits the compiled
//! card, and the worker observes a real Client. Selected logs/ores and dispatch
//! receipts are never injected. Fixture placement is supplied by the engine via
//! `GATHERER_WC_TILE` and `GATHERER_MINE_TILE` (`x,z,level`); the selected
//! catalogue and nav pack are supplied by `GATHERER_CATALOG_ROOT` and
//! `GATHERER_NAV_PACK`.
//!
//! Set `LIVE=1` and run an ignored test with the local 289 engine.  Debug
//! host logging is enabled so the production Driver's `native-packet` lines
//! (including a `CloseModal` + four-drop batch when the fixture presents a
//! modal) are part of the live witness.  Power cells check observed empty
//! slots, not merely dispatch receipts.
//! G2 cells cover moving fishing spots, supply gates, oak respawn and scene
//! boundaries, Auto widening, and an id-seeded gas hazard. Fixture helpers
//! alter locations only; they never inject products or XP.
//! Every live account uses the `g2` prefix and a per-process minted suffix.
//! Each cell uses an isolated HOME and requires absolute engine, nav, catalogue,
//! and observed placement paths.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use api::interact::{self, ActionSpec, Interactions, OpTarget, SendResult};
use api::snapshot::{ActorKind, GameSnapshot, WorldTile};
use host::Pump;
use host_play::{ProfileOptions, ScriptStartHandle, SharedClientTemplate};
use serde_json::{json, Map, Value};
use vault::{Profile, ProfileSettings};

const PREP_DEADLINE: Duration = Duration::from_secs(180);
const POWER_DEADLINE: Duration = Duration::from_secs(1800);
const WEDGE_DEADLINE: Duration = Duration::from_secs(900);
const POLL_INTERVAL: Duration = Duration::from_millis(20);
const REQUIRED_CYCLES: u32 = 2;
const REQUIRED_POST_DROP_GATHERS: u32 = 2;
const LOG_ID: i32 = 1511;
const COPPER_ID: i32 = 436;
const TIN_ID: i32 = 438;
const IRON_ID: i32 = 440;
const COAL_ID: i32 = 453;
const SHRIMP_ID: i32 = 317;
const ANCHOVY_ID: i32 = 321;
const TROUT_ID: i32 = 335;
const SALMON_ID: i32 = 331;
const FEATHERS_ID: i32 = 314;
const OAK_ID: i32 = 1281;
const OAK_STUMP_ID: i32 = 1355;
const OAK_RESPAWN_MAX_TICKS: u32 = 30;
const AUTO_TREE_IDS: &[i32] = &[1276, 1278];
const AUTO_STUMP_ID: i32 = 1342;
const ABSENT_LOC_ID: i32 = 2732;
const GAS_HAZARD_LOC_ID: i32 = 2121;
const IRON_ROCK_IDS: &[i32] = &[2092, 2093];
const MINE_PRODUCTS: &[i32] = &[
    COPPER_ID, TIN_ID, IRON_ID, COAL_ID, 1625, 1627, 1629, 1623, 1621, 1619, 1617,
];
const AUTO_NEXT_TILES: &[(i32, i32)] = &[(2409, 3480), (2417, 3480), (2414, 3478)];
const FISH_NET_START: WorldTile = WorldTile {
    x: 3267,
    z: 3148,
    level: 0,
};
const FISH_BAIT_START: WorldTile = WorldTile {
    x: 3238,
    z: 3252,
    level: 0,
};
const OAK_RESPAWN_START: WorldTile = WorldTile {
    x: 3017,
    z: 3170,
    level: 0,
};
const OAK_RESPAWN_OTHER: WorldTile = WorldTile {
    x: 3024,
    z: 3171,
    level: 0,
};
const OAK_EDGE_START: WorldTile = WorldTile {
    x: 3264,
    z: 3205,
    level: 0,
};
const OAK_EDGE_PREFLIGHT: WorldTile = WorldTile {
    x: 3300,
    z: 3205,
    level: 0,
};
const OAK_EDGE_TARGET: WorldTile = WorldTile {
    x: 3218,
    z: 3205,
    level: 0,
};
const OAK_ABSENT_START: WorldTile = OAK_EDGE_TARGET;
const AUTO_START: WorldTile = WorldTile {
    x: 2394,
    z: 3518,
    level: 0,
};
const GAS_START: WorldTile = WorldTile {
    x: 3294,
    z: 3310,
    level: 0,
};
const OAK_EDGE_SEEDS: &[(i32, i32)] = &[
    (3192, 3213),
    (3203, 3240),
    (3203, 3246),
    (3206, 3263),
    (3241, 3253),
    (3252, 3237),
    (3260, 3223),
];
// This centroid has 45 trees: the required forty nearer exhausted placements
// plus five guards that keep the entire first group exhausted.
const AUTO_FELLED_TILES: &[(i32, i32)] = &[
    (2374, 3503),
    (2379, 3502),
    (2382, 3501),
    (2382, 3511),
    (2385, 3518),
    (2389, 3507),
    (2390, 3504),
    (2392, 3509),
    (2393, 3515),
    (2394, 3518),
    (2395, 3506),
    (2395, 3509),
    (2398, 3506),
    (2400, 3503),
    (2403, 3499),
    (2403, 3518),
    (2407, 3509),
    (2408, 3504),
    (2412, 3481),
    (2421, 3493),
    (2424, 3488),
    (2425, 3498),
    (2426, 3501),
    (2429, 3484),
    (2430, 3488),
    (2430, 3498),
    (2431, 3481),
    (2383, 3525),
    (2390, 3521),
    (2391, 3525),
    (2393, 3526),
    (2395, 3521),
    (2395, 3529),
    (2396, 3527),
    (2398, 3527),
    (2398, 3529),
    (2400, 3520),
    (2407, 3527),
    (2411, 3524),
    (2413, 3525),
    (2415, 3522),
    (2415, 3524),
    (2424, 3527),
    (2427, 3524),
    (2430, 3528),
];
const NORMAL_STUMP_IDS: &[i32] = &[
    1342, 1343, 1345, 1347, 1348, 1349, 1351, 1352, 1353, 1354, 1355, 1358, 3880, 3884, 4819, 4821,
];
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Cell {
    Woodcutting,
    Mining,
    Fishing,
}

impl Cell {
    const fn name_for_case(self, case: LiveCase) -> &'static str {
        match (self, case) {
            (Self::Woodcutting, LiveCase::Power) => "gatherer_wc_power",
            (Self::Mining, LiveCase::Power) => "gatherer_mine_tier_power",
            (Self::Woodcutting, LiveCase::CancelBeforeDrain) => "gatherer_cancel_before_drain_live",
            (Self::Woodcutting, LiveCase::DeathDuringDrop) => "gatherer_death_during_drop_live",
            (Self::Woodcutting, LiveCase::RunKeyChangeDuringDrop) => {
                "gatherer_run_key_change_during_drop_live"
            }
            (Self::Fishing, LiveCase::FishNet) => "gatherer_fish_net",
            (Self::Fishing, LiveCase::FishBaitGate) => "gatherer_fish_bait_gate",
            (Self::Woodcutting, LiveCase::OakRespawn) => "gatherer_wc_respawn",
            (Self::Woodcutting, LiveCase::OakNearEdge) => "gatherer_wc_near_edge",
            (Self::Woodcutting, LiveCase::LocationAuto) => "gatherer_location_modes",
            (Self::Woodcutting, LiveCase::OakAbsentArea) => "gatherer_wc_absent",
            (Self::Mining, LiveCase::GasHazard) => "gatherer_mine_gas_hazard",
            _ => "gatherer_invalid_fixture",
        }
    }

    const fn tile_env(self, case: LiveCase) -> &'static str {
        match case {
            LiveCase::FishNet | LiveCase::FishBaitGate => "GATHERER_FISH_TILE",
            LiveCase::OakRespawn => "GATHERER_WC_RESPAWN_START_TILE",
            LiveCase::OakNearEdge => "GATHERER_WC_EDGE_START_TILE",
            LiveCase::LocationAuto => "GATHERER_AUTO_START_TILE",
            LiveCase::GasHazard => "GATHERER_GAS_TILE",
            LiveCase::OakAbsentArea => "GATHERER_WC_ABSENT_TILE",
            _ => match self {
                Self::Woodcutting => "GATHERER_WC_TILE",
                Self::Mining => "GATHERER_MINE_TILE",
                Self::Fishing => "GATHERER_FISH_TILE",
            },
        }
    }

    const fn skill_name(self) -> &'static str {
        match self {
            Self::Woodcutting => "woodcutting",
            Self::Mining => "mining",
            Self::Fishing => "fishing",
        }
    }

    const fn setting_skill(self) -> &'static str {
        match self {
            Self::Woodcutting => "Woodcutting",
            Self::Mining => "Mining",
            Self::Fishing => "Fishing",
        }
    }

    const fn default_tool_alias(self, case: LiveCase) -> &'static str {
        match (self, case) {
            (Self::Fishing, LiveCase::FishNet) => "net",
            (Self::Fishing, LiveCase::FishBaitGate) => "fly_fishing_rod",
            (Self::Mining, _) => "steel_pickaxe",
            _ => "bronze_axe",
        }
    }

    const fn default_tool_id(self, case: LiveCase) -> i32 {
        match (self, case) {
            (Self::Fishing, LiveCase::FishNet) => 303,
            (Self::Fishing, LiveCase::FishBaitGate) => 309,
            (Self::Mining, _) => 1269,
            _ => 1351,
        }
    }

    const fn default_level(self, case: LiveCase) -> i32 {
        match (self, case) {
            (
                Self::Woodcutting,
                LiveCase::OakRespawn | LiveCase::OakNearEdge | LiveCase::OakAbsentArea,
            ) => 15,
            (Self::Fishing, LiveCase::FishBaitGate) => 20,
            (Self::Mining, _) => 30,
            _ => 1,
        }
    }

    const fn products(self, case: LiveCase) -> &'static [i32] {
        match (self, case) {
            (
                Self::Woodcutting,
                LiveCase::OakRespawn | LiveCase::OakNearEdge | LiveCase::OakAbsentArea,
            ) => &[1521],
            (Self::Fishing, LiveCase::FishNet) => &[SHRIMP_ID, ANCHOVY_ID],
            (Self::Fishing, LiveCase::FishBaitGate) => &[TROUT_ID, SALMON_ID],
            (Self::Mining, _) => MINE_PRODUCTS,
            _ => &[LOG_ID],
        }
    }

    const fn target_preference(self) -> &'static str {
        match self {
            Self::Mining => "Best tier",
            _ => "Nearest",
        }
    }

    fn resources(self, case: LiveCase) -> Vec<String> {
        match self {
            Self::Woodcutting => vec![if matches!(
                case,
                LiveCase::OakRespawn | LiveCase::OakNearEdge | LiveCase::OakAbsentArea
            ) {
                "oak"
            } else {
                "normal"
            }
            .into()],
            Self::Mining if case == LiveCase::GasHazard => vec!["iron".into()],
            Self::Mining => std::env::var("GATHERER_MINE_RESOURCES")
                .unwrap_or_else(|_| "iron,coal".into())
                .split(',')
                .map(str::trim)
                .filter(|resource| !resource.is_empty())
                .map(ToOwned::to_owned)
                .collect(),
            Self::Fishing => Vec::new(),
        }
    }

    fn level(self, case: LiveCase) -> i32 {
        let env = match (self, case) {
            (Self::Woodcutting, LiveCase::OakRespawn) => Some("GATHERER_OAK_LEVEL"),
            (Self::Woodcutting, LiveCase::OakNearEdge) => Some("GATHERER_OAK_LEVEL"),
            (Self::Woodcutting, _) => Some("GATHERER_WC_LEVEL"),
            (Self::Mining, _) => Some("GATHERER_MINE_LEVEL"),
            (Self::Fishing, _) => Some("GATHERER_FISH_LEVEL"),
        };
        env.and_then(|name| std::env::var(name).ok())
            .and_then(|value| value.parse().ok())
            .unwrap_or_else(|| self.default_level(case))
    }

    fn tool_alias(self, case: LiveCase) -> String {
        match (self, case) {
            (Self::Woodcutting, _) => std::env::var("GATHERER_WC_TOOL")
                .unwrap_or_else(|_| self.default_tool_alias(case).into()),
            (Self::Mining, _) => std::env::var("GATHERER_MINE_TOOL")
                .unwrap_or_else(|_| self.default_tool_alias(case).into()),
            _ => self.default_tool_alias(case).into(),
        }
    }

    fn tool_id(self, case: LiveCase) -> i32 {
        let env = match self {
            Self::Woodcutting => Some("GATHERER_WC_TOOL_ID"),
            Self::Mining => Some("GATHERER_MINE_TOOL_ID"),
            Self::Fishing => None,
        };
        env.and_then(|name| std::env::var(name).ok())
            .and_then(|value| value.parse().ok())
            .unwrap_or_else(|| self.default_tool_id(case))
    }

    fn settings(self, case: LiveCase) -> Map<String, Value> {
        let mut bag = Map::new();
        bag.insert("skill".into(), json!(self.setting_skill()));
        bag.insert(
            "woodcuttingResources".into(),
            json!(self
                .resources(case)
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>()),
        );
        bag.insert(
            "miningResources".into(),
            json!(if self == Self::Mining {
                self.resources(case)
            } else {
                Vec::<String>::new()
            }),
        );
        if self == Self::Fishing {
            bag.insert(
                "fishingMethod".into(),
                json!(if case == LiveCase::FishBaitGate {
                    "fishing.freshfish.op1"
                } else {
                    "fishing.saltfish.op1"
                }),
            );
        }
        bag.insert("targetPreference".into(), json!(self.target_preference()));
        bag.insert(
            "location".into(),
            json!(if case == LiveCase::LocationAuto {
                "Auto"
            } else {
                "Start"
            }),
        );
        bag.insert(
            "radius".into(),
            json!(match case {
                LiveCase::OakRespawn | LiveCase::LocationAuto => 32,
                LiveCase::OakNearEdge => 64,
                LiveCase::OakAbsentArea => 2,
                LiveCase::GasHazard => 12,
                _ => 12,
            }),
        );
        bag.insert("disposition".into(), json!("Power"));
        bag.insert("allowTeleports".into(), json!(false));
        bag.insert("allowWilderness".into(), json!(false));
        bag.insert("deathPolicy".into(), json!("Stop"));
        bag.insert("maxDeaths".into(), json!(2));
        bag
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LiveCase {
    Power,
    CancelBeforeDrain,
    DeathDuringDrop,
    RunKeyChangeDuringDrop,
    FishNet,
    FishBaitGate,
    OakRespawn,
    OakNearEdge,
    LocationAuto,
    OakAbsentArea,
    GasHazard,
}

impl LiveCase {
    const fn name(self) -> &'static str {
        match self {
            Self::Power => "power",
            Self::CancelBeforeDrain => "cancel-before-drain",
            Self::DeathDuringDrop => "death-during-drop",
            Self::RunKeyChangeDuringDrop => "run-key-change-during-drop",
            Self::FishNet => "fish-net",
            Self::FishBaitGate => "fish-bait-gate",
            Self::OakRespawn => "oak-respawn",
            Self::OakNearEdge => "oak-near-edge",
            Self::LocationAuto => "location-auto",
            Self::OakAbsentArea => "oak-absent-area",
            Self::GasHazard => "gas-hazard",
        }
    }

    const fn is_power(self) -> bool {
        matches!(self, Self::Power | Self::FishNet)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Prep {
    WaitIngame,
    TutSkip,
    WaitTutorial,
    Relog,
    WaitRelog,
    Seed,
    WaitSeed,
    WaitEdgeStart,
    Equip,
    WaitEquip,
    OpenBank,
    WaitBank,
    WaitBankClose,
    ReturnFromBank,
    Ready,
    Running,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct InvRow {
    id: i32,
    count: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct FishTarget {
    type_id: usize,
    tile: (i32, i32, i32),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct BuildRect {
    base_x: i32,
    base_z: i32,
    width: i32,
    height: i32,
    level: i32,
}

impl BuildRect {
    fn contains(self, tile: (i32, i32, i32)) -> bool {
        tile.2 == self.level
            && tile.0 >= self.base_x
            && tile.0 < self.base_x + self.width
            && tile.1 >= self.base_z
            && tile.1 < self.base_z + self.height
    }

    fn edge_distance(self, tile: (i32, i32, i32)) -> Option<i32> {
        self.contains(tile).then(|| {
            (tile.0 - self.base_x)
                .min(self.base_x + self.width - 1 - tile.0)
                .min(tile.1 - self.base_z)
                .min(self.base_z + self.height - 1 - tile.1)
        })
    }

    fn outside_gap(self, tile: (i32, i32, i32)) -> Option<i32> {
        if tile.2 != self.level || self.contains(tile) {
            return None;
        }
        let dx = if tile.0 < self.base_x {
            self.base_x - tile.0
        } else if tile.0 >= self.base_x + self.width {
            tile.0 - (self.base_x + self.width - 1)
        } else {
            0
        };
        let dz = if tile.1 < self.base_z {
            self.base_z - tile.1
        } else if tile.1 >= self.base_z + self.height {
            tile.1 - (self.base_z + self.height - 1)
        } else {
            0
        };
        Some(dx.max(dz))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct LocSeed {
    tile: WorldTile,
    alias: &'static str,
    id: i32,
}

#[derive(Debug, Default, Clone)]
struct FixturePlan {
    oak_tiles: Vec<WorldTile>,
    first_oak: Option<WorldTile>,
    other_oak: Option<WorldTile>,
    edge_oak: Option<WorldTile>,
    preflight_edge: Option<WorldTile>,
    auto_felled: Vec<WorldTile>,
    auto_next: Vec<WorldTile>,
    seed_locs: Vec<LocSeed>,
    chop_target: Option<WorldTile>,
    inject_after_progress: Option<LocSeed>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FixtureStage {
    Start,
    WaitTile,
    Place,
    WaitLoc,
    WaitStump,
    Return,
    WaitReturn,
}

#[derive(Debug, Clone)]
enum FixtureTask {
    Seed {
        seeds: Vec<LocSeed>,
        next: usize,
        return_to: WorldTile,
        stage: FixtureStage,
    },
    Chop {
        tile: WorldTile,
        stage: FixtureStage,
    },
    InjectAfterProgress {
        seed: LocSeed,
        tile: Option<WorldTile>,
        return_to: Option<WorldTile>,
        stage: FixtureStage,
    },
}
#[derive(Debug, Default, Clone, Copy)]
struct FixtureProgress {
    baseline_xp: i32,
    xp: i32,
    product_count: i32,
    tile: Option<(i32, i32, i32)>,
    nearest_live_iron: Option<WorldTile>,
}

#[derive(Debug, Clone)]
struct Observation {
    inventory: BTreeMap<i32, InvRow>,
    product_count: i32,
    xp: i32,
    tick: u32,
    tile: Option<(i32, i32, i32)>,
    build: Option<BuildRect>,
    fishing_target: Option<FishTarget>,
    live_oaks: BTreeSet<(i32, i32, i32)>,
    oak_stumps: BTreeSet<(i32, i32, i32)>,
    normal_trees: BTreeSet<(i32, i32, i32)>,
    normal_stumps: BTreeSet<(i32, i32, i32)>,
    live_iron_tiles: BTreeSet<(i32, i32, i32)>,
    loc_ids: BTreeSet<(i32, i32, i32, i32)>,
}

impl Observation {
    fn has_loc(&self, id: i32, tile: WorldTile) -> bool {
        self.loc_ids.contains(&(id, tile.x, tile.z, tile.level))
    }
}

#[derive(Debug, Default, Clone)]
struct Witness {
    cycles: u32,
    post_drop_gathers: u32,
    confirmed_drops: u32,
    full_pack_seen: u32,
    max_product_count: i32,
    last_xp: i32,
    last_status_yielded: i64,
    last_status_dropped: i64,
    awaiting_drop: bool,
    awaiting_post_drop_gather: bool,
    cycle_drop_start: u32,
    cycle_confirmed_start: u32,
    cycle_product_slots: u32,
    cycle_xp_start: i32,
    methods: BTreeSet<String>,
    targets: BTreeSet<String>,
    status_values: BTreeMap<String, BTreeSet<String>>,
    native_phases: BTreeSet<String>,
    failure_code: Option<String>,
    failure_message: Option<String>,
    products_seen: BTreeSet<i32>,
    dropped_product_ids: BTreeSet<i32>,
    fish_products_seen: BTreeSet<i32>,
    first_regrown_tile: Option<(i32, i32, i32)>,
    edge_fresh_xp: bool,
    gas_hazard_tile: Option<(i32, i32, i32)>,
    gas_hazard_xp_after_escape: bool,
    fixture_chop_observed: bool,
    fixture_chop_tiles: BTreeSet<(i32, i32, i32)>,
    fixture_seeds_observed: usize,
    fixture_seed_tiles: BTreeSet<(i32, i32, i32, i32)>,
    last_area: Option<String>,
    last_event: Option<String>,
    first_fish_target: Option<FishTarget>,
    fish_move_xp: Option<i32>,
    fish_move_tile: Option<(i32, i32, i32)>,
    oak_other_gather_tick: Option<u32>,
    all_oaks_depleted_at_wait: bool,
    edge_initial_gap: Option<i32>,
    gas_hazard_xp_start: i32,
    gas_hazard_start_tile: Option<(i32, i32, i32)>,
    edge_initial_distance: Option<i32>,
    auto_selected_next: bool,
    gas_hazard_seeded: bool,
    gas_hazard_observed: bool,
    gas_hazard_escaped: bool,
    fish_targets: BTreeSet<FishTarget>,
    moved_spot_reacquired: bool,
    preflight_build_observed: bool,
    fish_target_seen: bool,
    no_fish_xp: bool,
    player_tiles: BTreeSet<(i32, i32, i32)>,
    oak_live_tiles: BTreeSet<(i32, i32, i32)>,
    oak_stump_tiles: BTreeSet<(i32, i32, i32)>,
    normal_stump_tiles: BTreeSet<(i32, i32, i32)>,
    initial_build: Option<BuildRect>,
    initial_area: Option<String>,
    script_moved_after_start: bool,
    first_wait_tick: Option<u32>,
    wait_until: Option<u64>,
    first_wait_native_tick: Option<u64>,
    first_regrown_tick: Option<u32>,
    first_regrown_approach_tick: Option<u32>,
    first_regrown_gather_tick: Option<u32>,
    first_edge_oak_loaded_tick: Option<u32>,
    first_edge_oak_approach_tick: Option<u32>,
    auto_widen_seen: bool,
    auto_next_gather_tick: Option<u32>,
    wedge_triggered: bool,
    stopped_before_drain: bool,
    death_command_sent: bool,
    death_blocked: bool,
    death_restart_tile: Option<(i32, i32, i32)>,
    death_restart_xp: i32,
    death_restart_gathered: bool,
    modal_command_sent: bool,
    modal_observed: bool,
    run_key_changed: bool,
    banked_unusable_tool: Option<i32>,
    failure: Option<String>,
}

struct GatherSlot {
    cell: Cell,
    case: LiveCase,
    fixture_helper: bool,
    plan: FixturePlan,
    fixture_task: Option<FixtureTask>,
    fixture_done: bool,
    target: WorldTile,
    tool_id: i32,
    tool_alias: String,
    requested_level: i32,
    phase: Prep,
    snapshot: GameSnapshot,
    pump: Pump,
    native_tick: u64,
    last_action: Instant,
    started: bool,
    start_requested: bool,
    baseline: Option<Observation>,
    latest: Option<Observation>,
    unsettled_items: BTreeMap<(i32, i32), Instant>,
    witness: Witness,
    error: Option<String>,
}

impl GatherSlot {
    fn new(
        cell: Cell,
        case: LiveCase,
        target: WorldTile,
        plan: FixturePlan,
        fixture_task: Option<FixtureTask>,
        fixture_helper: bool,
    ) -> Self {
        Self {
            cell,
            case,
            plan,
            fixture_helper,
            fixture_done: fixture_task.is_none(),
            fixture_task,
            target,
            tool_id: cell.tool_id(case),
            tool_alias: cell.tool_alias(case),
            requested_level: if fixture_helper && case == LiveCase::OakRespawn {
                99
            } else {
                cell.level(case)
            },
            phase: Prep::WaitIngame,
            snapshot: GameSnapshot::new(),
            pump: Pump::new(),
            native_tick: 0,
            last_action: Instant::now(),
            started: false,
            start_requested: false,
            baseline: None,
            latest: None,
            unsettled_items: BTreeMap::new(),
            witness: Witness::default(),
            error: None,
        }
    }
    fn fixture_progress(&self) -> FixtureProgress {
        let latest = self.latest.as_ref();
        let nearest_live_iron = latest.and_then(|row| {
            let (x, z, level) = row.tile?;
            row.live_iron_tiles
                .iter()
                .filter(|tile| tile.2 == level)
                .min_by_key(|tile| (x - tile.0).abs().max((z - tile.1).abs()))
                .map(|&(x, z, level)| WorldTile { x, z, level })
        });
        FixtureProgress {
            baseline_xp: self.baseline.as_ref().map_or(0, |row| row.xp),
            xp: latest.map_or(0, |row| row.xp),
            product_count: latest.map_or(0, |row| row.product_count),
            tile: latest.and_then(|row| row.tile),
            nearest_live_iron,
        }
    }

    fn fixture_ready(&self) -> bool {
        let Some(latest) = self.latest.as_ref() else {
            return false;
        };
        match self.case {
            LiveCase::OakRespawn => {
                self.plan
                    .first_oak
                    .zip(self.plan.other_oak)
                    .is_some_and(|(first, other)| {
                        latest.has_loc(OAK_STUMP_ID, first)
                            && latest.live_oaks.contains(&tile_key(other))
                    })
            }
            LiveCase::OakNearEdge => {
                (self.witness.preflight_build_observed || self.expected_preflight_build())
                    && self
                        .plan
                        .seed_locs
                        .iter()
                        .find(|seed| seed.id == ABSENT_LOC_ID)
                        .is_some_and(|seed| latest.has_loc(seed.id, seed.tile))
            }
            LiveCase::OakAbsentArea => self
                .plan
                .seed_locs
                .iter()
                .any(|seed| seed.id == ABSENT_LOC_ID && latest.has_loc(seed.id, seed.tile)),
            LiveCase::LocationAuto => {
                self.plan
                    .auto_felled
                    .iter()
                    .all(|tile| latest.has_loc(AUTO_STUMP_ID, *tile))
                    && self
                        .plan
                        .auto_next
                        .iter()
                        .any(|tile| latest.normal_trees.contains(&tile_key(*tile)))
            }
            _ => true,
        }
    }

    fn ready_to_start(&self) -> bool {
        self.phase == Prep::Ready
            && if self.case == LiveCase::GasHazard {
                true
            } else {
                self.fixture_done && self.fixture_ready()
            }
    }

    fn name(&self) -> &'static str {
        self.cell.name_for_case(self.case)
    }

    fn publish(&mut self, client: &client::client::Client) -> Observation {
        let drain = self.pump.drain_client(client);
        // Match the worker's PLAYER_INFO clock across login/session resets;
        // GameSnapshot::tick resets at login and is not the native evidence clock.
        if host::should_emit_tick(drain.player_info) {
            self.native_tick = self.native_tick.wrapping_add(1);
        }
        host::publish_snapshot(&mut self.snapshot, client, drain);
        self.capture()
    }

    fn capture(&self) -> Observation {
        let inventory = self
            .snapshot
            .inventory()
            .iter()
            .filter(|item| item.slot >= 0 && item.def.id >= 0 && item.count > 0)
            .map(|item| {
                (
                    item.slot,
                    InvRow {
                        id: item.def.id,
                        count: item.count,
                    },
                )
            })
            .collect::<BTreeMap<_, _>>();
        let product_count = inventory
            .values()
            .filter(|row| self.cell.products(self.case).contains(&row.id))
            .map(|row| row.count.max(0))
            .sum();
        let xp = self
            .snapshot
            .stats()
            .iter()
            .find(|stat| stat.name.eq_ignore_ascii_case(self.cell.skill_name()))
            .map_or(0, |stat| stat.xp);
        let scene = self.snapshot.scene();
        let build = scene.available.then_some(BuildRect {
            base_x: self.snapshot.world().map_base_x,
            base_z: self.snapshot.world().map_base_z,
            width: scene.width,
            height: scene.height,
            level: scene.level,
        });
        let fishing_target = if self.cell == Cell::Fishing {
            let action = if self.case == LiveCase::FishBaitGate {
                "Lure"
            } else {
                "Net"
            };
            self.snapshot
                .local_player()
                .and_then(|local| local.player.actor.target)
                .filter(|target| target.kind == ActorKind::Npc)
                .and_then(|target| {
                    self.snapshot
                        .npcs()
                        .iter()
                        .find(|npc| npc.index == target.index)
                })
                .filter(|npc| {
                    npc.actions
                        .iter()
                        .flatten()
                        .any(|candidate| candidate.eq_ignore_ascii_case(action))
                })
                .and_then(|npc| {
                    npc.r#type.map(|type_id| FishTarget {
                        type_id,
                        tile: (npc.tile.x, npc.tile.z, npc.tile.level),
                    })
                })
        } else {
            None
        };
        let locs = self.snapshot.locs();
        let live_oaks = locs
            .iter()
            .filter(|loc| loc.id == OAK_ID)
            .map(|loc| (loc.tile.x, loc.tile.z, loc.tile.level))
            .collect();
        let oak_stumps = locs
            .iter()
            .filter(|loc| loc.id == OAK_STUMP_ID)
            .map(|loc| (loc.tile.x, loc.tile.z, loc.tile.level))
            .collect();
        let normal_trees = locs
            .iter()
            .filter(|loc| AUTO_TREE_IDS.contains(&loc.id))
            .map(|loc| (loc.tile.x, loc.tile.z, loc.tile.level))
            .collect();
        let normal_stumps = locs
            .iter()
            .filter(|loc| NORMAL_STUMP_IDS.contains(&loc.id))
            .map(|loc| (loc.tile.x, loc.tile.z, loc.tile.level))
            .collect();
        let live_iron_tiles = locs
            .iter()
            .filter(|loc| IRON_ROCK_IDS.contains(&loc.id))
            .map(|loc| (loc.tile.x, loc.tile.z, loc.tile.level))
            .collect();
        let loc_ids = locs
            .iter()
            .map(|loc| (loc.id, loc.tile.x, loc.tile.z, loc.tile.level))
            .collect();
        Observation {
            inventory,
            product_count,
            xp,
            tick: self.snapshot.tick(),
            tile: self.snapshot.tile(),
            build,
            fishing_target,
            live_oaks,
            oak_stumps,
            normal_trees,
            normal_stumps,
            live_iron_tiles,
            loc_ids,
        }
    }

    fn frame(
        &mut self,
        client: &mut client::client::Client,
        hold: bool,
        progress: Option<&FixtureProgress>,
    ) {
        let previous_chat_root = self.snapshot.modals().chat;
        let observation = self.publish(client);
        if self.started && self.snapshot.modals().chat != previous_chat_root {
            println!(
                "gatherer-chat {}",
                json!({
                    "previous_root": previous_chat_root,
                    "root": self.snapshot.modals().chat,
                    "texts": self.snapshot.chat_modal_texts(),
                    "xp": observation.xp,
                })
            );
        }
        if self.started && self.witness.modal_command_sent && self.snapshot.modals().main >= 0 {
            self.witness.modal_observed = true;
        }
        let previous = self.latest.replace(observation.clone());
        if self.case == LiveCase::OakNearEdge && self.expected_preflight_build() {
            self.witness.preflight_build_observed = true;
        }
        if hold {
            return;
        }
        if self.started {
            if let Err(error) = self.observe_running(previous, observation) {
                self.error = Some(error);
                return;
            }
            if self.fixture_task.is_some() {
                if let Err(error) = self.advance_fixture_with_progress(client, progress) {
                    self.error = Some(error);
                }
            }
        } else if self.fixture_task.is_some() && self.phase == Prep::Ready {
            if let Err(error) = self.advance_fixture_with_progress(client, progress) {
                self.error = Some(error);
            }
        } else if let Err(error) = self.advance_prep(client) {
            self.error = Some(error);
        }
    }

    fn advance_fixture(
        &mut self,
        client: &mut client::client::Client,
        progress: &FixtureProgress,
    ) -> Result<(), String> {
        let Some(task) = self.fixture_task.take() else {
            self.fixture_done = true;
            return Ok(());
        };
        let next_task = match task {
            FixtureTask::Seed {
                seeds,
                mut next,
                return_to,
                mut stage,
            } => {
                if next >= seeds.len()
                    && matches!(
                        stage,
                        FixtureStage::Start
                            | FixtureStage::WaitTile
                            | FixtureStage::Place
                            | FixtureStage::WaitLoc
                    )
                {
                    stage = FixtureStage::Return;
                }
                let active_seed = if matches!(
                    stage,
                    FixtureStage::Start
                        | FixtureStage::WaitTile
                        | FixtureStage::Place
                        | FixtureStage::WaitLoc
                ) {
                    Some(
                        *seeds
                            .get(next)
                            .ok_or("fixture seed index escaped its plan")?,
                    )
                } else {
                    None
                };
                let visit_tile = active_seed.map(|seed| {
                    if !self.fixture_helper && self.case == LiveCase::OakNearEdge {
                        // Observe the seeded northern zone without crossing the
                        // western rebuild threshold that this cell must retain.
                        WorldTile {
                            x: return_to.x,
                            ..seed.tile
                        }
                    } else {
                        seed.tile
                    }
                });
                match stage {
                    FixtureStage::Start => {
                        let visit = visit_tile.expect("seed stage has a visit tile");
                        if self.snapshot.tile() == Some((visit.x, visit.z, visit.level)) {
                            stage = FixtureStage::Place;
                        } else {
                            send_cheat(
                                client,
                                &interact::tele_args(visit.level, visit.x, visit.z),
                            )?;
                            stage = FixtureStage::WaitTile;
                        }
                    }
                    FixtureStage::WaitTile => {
                        let visit = visit_tile.expect("seed stage has a visit tile");
                        if self.snapshot.tile() == Some((visit.x, visit.z, visit.level)) {
                            stage = FixtureStage::Place;
                        }
                    }
                    FixtureStage::Place => {
                        let seed = active_seed.expect("seed stage has an active seed");
                        if self.fixture_helper {
                            // Refresh even an earlier run's still-visible stump;
                            // its old expiry must not invalidate this run's setup.
                            send_cheat(client, &format!("~loc {}", seed.alias))?;
                            stage = FixtureStage::WaitLoc;
                        } else if self
                            .latest
                            .as_ref()
                            .is_some_and(|row| row.has_loc(seed.id, seed.tile))
                        {
                            next += 1;
                            self.witness.fixture_seeds_observed = next;
                            stage = FixtureStage::Start;
                        } else {
                            stage = FixtureStage::WaitLoc;
                        }
                    }
                    FixtureStage::WaitLoc => {
                        let seed = active_seed.expect("seed stage has an active seed");
                        if self
                            .latest
                            .as_ref()
                            .is_some_and(|row| row.has_loc(seed.id, seed.tile))
                        {
                            next += 1;
                            self.witness.fixture_seeds_observed = next;
                            stage = FixtureStage::Start;
                        }
                    }
                    FixtureStage::Return => {
                        if self.snapshot.tile() == Some((return_to.x, return_to.z, return_to.level))
                        {
                            self.fixture_done = true;
                            self.fixture_task = None;
                            return Ok(());
                        }
                        send_cheat(
                            client,
                            &interact::tele_args(return_to.level, return_to.x, return_to.z),
                        )?;
                        stage = FixtureStage::WaitReturn;
                    }
                    FixtureStage::WaitReturn => {
                        if self.snapshot.tile() == Some((return_to.x, return_to.z, return_to.level))
                        {
                            self.fixture_done = true;
                            self.fixture_task = None;
                            return Ok(());
                        }
                    }
                    FixtureStage::WaitStump => {
                        return Err("seed actor entered the oak-fell wait state".into());
                    }
                }
                FixtureTask::Seed {
                    seeds,
                    next,
                    return_to,
                    stage,
                }
            }
            FixtureTask::Chop { tile, mut stage } => {
                if self.snapshot.inventory().len() == 28 {
                    if self.last_action.elapsed() >= Duration::from_millis(600) {
                        if let Some(log) = self
                            .snapshot
                            .inventory()
                            .iter()
                            .find(|item| item.def.id == 1521 && item.count > 0)
                        {
                            Interactions::new(&self.snapshot, client)
                                .interact(OpTarget::Item(log), ActionSpec::Label("Drop".into()));
                            self.last_action = Instant::now();
                        }
                    }
                    self.fixture_task = Some(FixtureTask::Chop {
                        tile,
                        stage: FixtureStage::Start,
                    });
                    return Ok(());
                }
                let stand = WorldTile {
                    x: tile.x - 1,
                    ..tile
                };
                match stage {
                    FixtureStage::Start => {
                        if self
                            .latest
                            .as_ref()
                            .is_some_and(|row| row.has_loc(OAK_STUMP_ID, tile))
                        {
                            self.witness.fixture_chop_observed = true;
                            self.witness.fixture_chop_tiles.insert(tile_key(tile));
                            self.fixture_done = true;
                            self.fixture_task = None;
                            return Ok(());
                        }
                        if self.snapshot.tile() != Some((stand.x, stand.z, stand.level)) {
                            send_cheat(
                                client,
                                &interact::tele_args(stand.level, stand.x, stand.z),
                            )?;
                            stage = FixtureStage::WaitTile;
                        } else {
                            let loc = self.snapshot.locs().iter().find(|loc| {
                                loc.id == OAK_ID
                                    && loc.tile
                                        == (WorldTile {
                                            x: tile.x,
                                            z: tile.z,
                                            level: tile.level,
                                        })
                            });
                            if let Some(loc) = loc {
                                if !matches!(
                                    Interactions::new(&self.snapshot, client).interact(
                                        OpTarget::Loc(loc),
                                        ActionSpec::Label("Chop down".into()),
                                    ),
                                    SendResult::Sent { .. }
                                ) {
                                    return Err(format!(
                                        "fixture Chop down was refused at {tile:?}"
                                    ));
                                }
                                stage = FixtureStage::WaitStump;
                            }
                        }
                    }
                    FixtureStage::WaitTile => {
                        if self.snapshot.tile() == Some((stand.x, stand.z, stand.level)) {
                            stage = FixtureStage::Start;
                        }
                    }
                    FixtureStage::WaitStump => {
                        if self
                            .latest
                            .as_ref()
                            .is_some_and(|row| row.has_loc(OAK_STUMP_ID, tile))
                        {
                            self.witness.fixture_chop_observed = true;
                            self.witness.fixture_chop_tiles.insert(tile_key(tile));
                            self.fixture_done = true;
                            self.fixture_task = None;
                            return Ok(());
                        }
                    }
                    _ => return Err("oak feller entered an invalid fixture state".into()),
                }
                FixtureTask::Chop { tile, stage }
            }
            FixtureTask::InjectAfterProgress {
                seed,
                mut tile,
                mut return_to,
                mut stage,
            } => match stage {
                FixtureStage::Start => {
                    if progress.xp <= progress.baseline_xp || progress.product_count <= 0 {
                        FixtureTask::InjectAfterProgress {
                            seed,
                            tile,
                            return_to,
                            stage,
                        }
                    } else if let Some(target) = progress.nearest_live_iron {
                        tile = Some(target);
                        return_to = progress.tile.map(|(x, z, level)| WorldTile { x, z, level });
                        if self.snapshot.tile() == Some((target.x, target.z, target.level)) {
                            stage = FixtureStage::Place;
                            FixtureTask::InjectAfterProgress {
                                seed,
                                tile,
                                return_to,
                                stage,
                            }
                        } else {
                            send_cheat(
                                client,
                                &interact::tele_args(target.level, target.x, target.z),
                            )?;
                            stage = FixtureStage::WaitTile;
                            FixtureTask::InjectAfterProgress {
                                seed,
                                tile,
                                return_to,
                                stage,
                            }
                        }
                    } else {
                        FixtureTask::InjectAfterProgress {
                            seed,
                            tile,
                            return_to,
                            stage,
                        }
                    }
                }
                FixtureStage::WaitTile => {
                    if let Some(target) = tile {
                        if self.snapshot.tile() == Some((target.x, target.z, target.level)) {
                            stage = FixtureStage::Place;
                        }
                    }
                    FixtureTask::InjectAfterProgress {
                        seed,
                        tile,
                        return_to,
                        stage,
                    }
                }
                FixtureStage::Place => {
                    if let Some(target) = tile {
                        if progress.nearest_live_iron == Some(target) {
                            send_cheat(client, &format!("~loc {}", seed.alias))?;
                            self.plan.inject_after_progress = Some(LocSeed {
                                tile: target,
                                ..seed
                            });
                            stage = FixtureStage::WaitLoc;
                        } else {
                            tile = None;
                            return_to = None;
                            stage = FixtureStage::Start;
                        }
                    }
                    FixtureTask::InjectAfterProgress {
                        seed,
                        tile,
                        return_to,
                        stage,
                    }
                }
                FixtureStage::WaitLoc => {
                    if tile.is_some_and(|target| {
                        self.latest
                            .as_ref()
                            .is_some_and(|row| row.has_loc(seed.id, target))
                    }) {
                        if return_to.is_some() {
                            stage = FixtureStage::Return;
                        } else {
                            self.fixture_done = true;
                            self.fixture_task = None;
                            return Ok(());
                        }
                    }
                    FixtureTask::InjectAfterProgress {
                        seed,
                        tile,
                        return_to,
                        stage,
                    }
                }
                FixtureStage::Return => {
                    let home = return_to.ok_or("hazard injector lost its return tile")?;
                    if self.snapshot.tile() == Some((home.x, home.z, home.level)) {
                        self.fixture_done = true;
                        self.fixture_task = None;
                        return Ok(());
                    }
                    send_cheat(client, &interact::tele_args(home.level, home.x, home.z))?;
                    stage = FixtureStage::WaitReturn;
                    FixtureTask::InjectAfterProgress {
                        seed,
                        tile,
                        return_to,
                        stage,
                    }
                }
                FixtureStage::WaitReturn => {
                    if return_to.is_some_and(|home| {
                        self.snapshot.tile() == Some((home.x, home.z, home.level))
                    }) {
                        self.fixture_done = true;
                        self.fixture_task = None;
                        return Ok(());
                    }
                    FixtureTask::InjectAfterProgress {
                        seed,
                        tile,
                        return_to,
                        stage,
                    }
                }
                FixtureStage::WaitStump => {
                    return Err("hazard injector entered the oak-fell wait state".into());
                }
            },
        };
        self.fixture_task = Some(next_task);
        Ok(())
    }

    fn advance_fixture_with_progress(
        &mut self,
        client: &mut client::client::Client,
        progress: Option<&FixtureProgress>,
    ) -> Result<(), String> {
        if let Some(progress) = progress {
            self.advance_fixture(client, progress)
        } else {
            let progress = self.fixture_progress();
            self.advance_fixture(client, &progress)
        }
    }

    fn pre_drain_wedge(&self) -> bool {
        self.started
            && self.witness.awaiting_drop
            && !self.witness.wedge_triggered
            && !self.case.is_power()
    }

    fn mark_stop_wedge(&mut self) {
        self.witness.wedge_triggered = true;
        self.witness.stopped_before_drain = true;
    }

    fn mark_death_wedge(&mut self) {
        self.witness.wedge_triggered = true;
        self.witness.death_command_sent = true;
    }

    fn reset_after_stop_for_restart(&mut self) {
        self.started = true;
        self.phase = Prep::Running;
        self.start_requested = false;
        self.witness.awaiting_drop = false;
    }

    fn observe_running(
        &mut self,
        previous: Option<Observation>,
        observation: Observation,
    ) -> Result<(), String> {
        if self
            .witness
            .banked_unusable_tool
            .is_some_and(|id| observation.inventory.values().any(|row| row.id == id))
        {
            return Err("Gatherer fetched the unusable banked pickaxe".into());
        }
        // Inventory and equipment arrive in separate packets. A removed tool
        // must reappear in an observed container within the settlement bound.
        self.unsettled_items.retain(|&(id, count), _| {
            !observation
                .inventory
                .values()
                .any(|item| item.id == id && item.count == count)
                && !self
                    .snapshot
                    .equipment()
                    .iter()
                    .any(|item| item.def.id == id && item.count == count)
        });
        if let Some((&(id, count), _)) = self
            .unsettled_items
            .iter()
            .find(|(_, since)| since.elapsed() >= Duration::from_secs(2))
        {
            return Err(format!(
                "{}: non-product id {id} count {count} was not conserved",
                self.name()
            ));
        }
        self.record_g2_observation(previous.as_ref(), &observation);
        if let Some(previous) = previous {
            for (slot, old) in &previous.inventory {
                if self.cell.products(self.case).contains(&old.id) {
                    if !observation.inventory.contains_key(slot) {
                        self.witness.confirmed_drops =
                            self.witness.confirmed_drops.saturating_add(1);
                        self.witness.dropped_product_ids.insert(old.id);
                    }
                } else if !observation.inventory.contains_key(slot)
                    && !self
                        .snapshot
                        .equipment()
                        .iter()
                        .any(|item| item.def.id == old.id && item.count == old.count)
                {
                    self.unsettled_items
                        .entry((old.id, old.count))
                        .or_insert_with(Instant::now);
                }
            }
        }
        self.witness.max_product_count = self
            .witness
            .max_product_count
            .max(observation.product_count);
        self.witness.last_xp = observation.xp;
        if observation.inventory.len() == 28
            && observation.product_count > 0
            && !self.witness.awaiting_drop
        {
            self.witness.full_pack_seen = self.witness.full_pack_seen.saturating_add(1);
            self.witness.awaiting_drop = true;
            self.witness.cycle_drop_start = self.witness.last_status_dropped.max(0) as u32;
            self.witness.cycle_confirmed_start = self.witness.confirmed_drops;
            self.witness.cycle_product_slots = observation.product_count as u32;
            self.witness.cycle_xp_start = observation.xp;
        }
        self.latest = Some(observation);
        Ok(())
    }

    fn record_g2_observation(&mut self, previous: Option<&Observation>, observation: &Observation) {
        let baseline_xp = self.baseline_xp();
        if let Some(tile) = observation.tile {
            self.witness.player_tiles.insert(tile);
            if self.started && self.baseline.as_ref().and_then(|row| row.tile) != Some(tile) {
                self.witness.script_moved_after_start = true;
            }
        }

        for seed in &self.plan.seed_locs {
            if observation.has_loc(seed.id, seed.tile)
                && self.witness.fixture_seed_tiles.insert((
                    seed.id,
                    seed.tile.x,
                    seed.tile.z,
                    seed.tile.level,
                ))
            {
                self.witness.fixture_seeds_observed =
                    self.witness.fixture_seeds_observed.saturating_add(1);
            }
        }

        if self
            .plan
            .first_oak
            .is_some_and(|tile| observation.has_loc(OAK_STUMP_ID, tile))
        {
            self.witness.fixture_chop_observed = true;
        }
        if self.witness.all_oaks_depleted_at_wait && self.witness.first_regrown_tile.is_none() {
            let first_regrown = observation
                .live_oaks
                .iter()
                .filter(|&&tile| self.witness.oak_stump_tiles.contains(&tile))
                .min_by_key(|&&tile| {
                    let target = WorldTile {
                        x: tile.0,
                        z: tile.1,
                        level: tile.2,
                    };
                    observation
                        .tile
                        .and_then(|here| tile_distance(here, target))
                        .unwrap_or(i32::MAX)
                });
            if let Some(&tile) = first_regrown {
                self.witness.first_regrown_tile = Some(tile);
                self.witness.first_regrown_tick = Some(observation.tick);
            }
        }
        if self.witness.first_regrown_approach_tick.is_none() {
            if let (Some(tile), Some(regrown), Some(previous)) = (
                self.witness.first_regrown_tile,
                self.witness.first_regrown_tick,
                previous,
            ) {
                let target = WorldTile {
                    x: tile.0,
                    z: tile.1,
                    level: tile.2,
                };
                let approached = previous
                    .tile
                    .and_then(|here| tile_distance(here, target))
                    .zip(
                        observation
                            .tile
                            .and_then(|here| tile_distance(here, target)),
                    )
                    .is_some_and(|(old, new)| new < old);
                if approached && observation.tick >= regrown {
                    self.witness.first_regrown_approach_tick = Some(observation.tick);
                }
            }
        }
        self.witness
            .oak_live_tiles
            .extend(observation.live_oaks.iter().copied());
        self.witness
            .oak_stump_tiles
            .extend(observation.oak_stumps.iter().copied());
        self.witness
            .normal_stump_tiles
            .extend(observation.normal_stumps.iter().copied());

        for item in observation.inventory.values().filter(|row| row.count > 0) {
            if self.cell.products(self.case).contains(&item.id) {
                self.witness.products_seen.insert(item.id);
                if self.cell == Cell::Fishing {
                    self.witness.fish_products_seen.insert(item.id);
                }
            }
        }
        if let Some(target) = observation.fishing_target {
            self.witness.fish_target_seen = true;
            self.witness.fish_targets.insert(target);
            if self.case == LiveCase::FishNet {
                if let Some(first) = self.witness.first_fish_target {
                    if target != first {
                        if let Some(xp) = self.witness.fish_move_xp {
                            if observation.xp > xp {
                                self.witness.moved_spot_reacquired = true;
                            }
                        } else {
                            self.witness.fish_move_xp = Some(observation.xp);
                            self.witness.fish_move_tile = observation.tile;
                        }
                    }
                } else {
                    self.witness.first_fish_target = Some(target);
                }
            }
        }

        if self.case == LiveCase::FishBaitGate {
            self.witness.no_fish_xp = self.baseline.as_ref().is_some_and(|baseline| {
                observation.xp == baseline.xp && observation.product_count == 0
            });
        }

        if self.case == LiveCase::OakRespawn {
            if let (Some(previous), Some(other)) = (previous, self.plan.other_oak) {
                let product_grew = observation.product_count > previous.product_count;
                let near_other = observation
                    .tile
                    .and_then(|tile| tile_distance(tile, other))
                    .is_some_and(|distance| distance <= 1);
                if product_grew && near_other && self.witness.oak_other_gather_tick.is_none() {
                    self.witness.oak_other_gather_tick = Some(observation.tick);
                }
            }
        }

        if self.case == LiveCase::OakNearEdge {
            if let Some(edge) = self.plan.edge_oak {
                let key = tile_key(edge);
                if observation.live_oaks.contains(&key)
                    && self.witness.first_edge_oak_loaded_tick.is_none()
                {
                    self.witness.first_edge_oak_loaded_tick = Some(observation.tick);
                }
                if let (Some(previous), Some(here)) = (previous, observation.tile) {
                    if let Some(previous_tile) = previous.tile {
                        let old_distance = tile_distance(previous_tile, edge);
                        let new_distance = tile_distance(here, edge);
                        if matches!((old_distance, new_distance), (Some(old), Some(new)) if new < old)
                            && self.witness.first_edge_oak_approach_tick.is_none()
                        {
                            self.witness.first_edge_oak_approach_tick = Some(observation.tick);
                        }
                    }
                }
                if observation.xp > baseline_xp
                    && observation
                        .tile
                        .and_then(|tile| tile_distance(tile, edge))
                        .is_some_and(|distance| distance <= 1)
                {
                    self.witness.edge_fresh_xp = true;
                }
            }
        }

        if self.case == LiveCase::LocationAuto
            && self.witness.auto_widen_seen
            && observation.tile.is_some_and(|tile| {
                self.plan.auto_next.iter().any(|target| {
                    tile_distance(tile, *target).is_some_and(|distance| distance <= 6)
                })
            })
        {
            self.witness.auto_selected_next = true;
            if previous.is_some_and(|old| observation.product_count > old.product_count) {
                self.witness
                    .auto_next_gather_tick
                    .get_or_insert(observation.tick);
            }
        }

        if self.case == LiveCase::GasHazard {
            if let Some(hazard) = self.plan.inject_after_progress {
                if observation.has_loc(hazard.id, hazard.tile) && !self.witness.gas_hazard_seeded {
                    self.witness.gas_hazard_seeded = true;
                    self.witness.gas_hazard_tile = Some(tile_key(hazard.tile));
                    self.witness.gas_hazard_xp_start = observation.xp;
                }
            }
            if self.witness.gas_hazard_observed {
                if self.witness.gas_hazard_start_tile.is_none() {
                    self.witness.gas_hazard_start_tile = observation.tile;
                } else if let (Some(start), Some(current), Some(hazard)) = (
                    self.witness.gas_hazard_start_tile,
                    observation.tile,
                    self.witness.gas_hazard_tile,
                ) {
                    let distance = |tile: (i32, i32, i32)| {
                        if tile.2 != hazard.2 {
                            i32::MAX
                        } else {
                            (tile.0 - hazard.0).abs().max((tile.1 - hazard.1).abs())
                        }
                    };
                    if current != start && distance(current) > distance(start) {
                        self.witness.gas_hazard_escaped = true;
                    }
                }
            }
            if self.witness.gas_hazard_escaped && observation.xp > self.witness.gas_hazard_xp_start
            {
                self.witness.gas_hazard_xp_after_escape = true;
            }
        }
    }

    fn cycle_product_capacity(&self) -> u32 {
        28 - self.baseline.as_ref().map_or(0, |baseline| {
            baseline
                .inventory
                .values()
                .filter(|row| !self.cell.products(self.case).contains(&row.id))
                .count() as u32
        })
    }

    fn chat_has(&self, needle: &str) -> bool {
        let needle = needle.to_ascii_lowercase();
        self.snapshot
            .chat_lines()
            .iter()
            .any(|line| line.text.to_ascii_lowercase().contains(&needle))
            || self
                .snapshot
                .chat_modal_texts()
                .iter()
                .any(|text| text.to_ascii_lowercase().contains(&needle))
    }

    fn inventory_tab_available(&self) -> bool {
        self.snapshot
            .side_tabs()
            .iter()
            .any(|tab| tab.index == 3 && tab.available)
    }

    fn item_in_inventory(&self) -> bool {
        self.snapshot
            .inventory()
            .iter()
            .any(|item| item.def.id == self.tool_id && item.count > 0)
    }

    fn item_equipped(&self) -> bool {
        self.snapshot
            .equipment()
            .iter()
            .any(|item| item.def.id == self.tool_id && item.count > 0)
    }

    fn near_target(&self) -> bool {
        self.near_tile(self.target, 6)
    }

    fn near_tile(&self, target: WorldTile, distance: i32) -> bool {
        self.snapshot.tile().is_some_and(|tile| {
            tile.2 == target.level
                && (tile.0 - target.x).abs().max((tile.1 - target.z).abs()) <= distance
        })
    }

    fn prep_tile(&self) -> WorldTile {
        if self.case == LiveCase::OakNearEdge && !self.fixture_helper {
            self.plan
                .preflight_edge
                .expect("near-edge fixture has a preflight tile")
        } else {
            self.target
        }
    }

    fn expected_preflight_build(&self) -> bool {
        let Some(tile) = self.plan.preflight_edge else {
            return false;
        };
        let Some(build) = self.latest.as_ref().and_then(|row| row.build) else {
            return false;
        };
        build.base_x == (tile.x.div_euclid(8) - 6) * 8
            && build.base_z == (tile.z.div_euclid(8) - 6) * 8
            && build.width == 104
            && build.height == 104
            && build.level == tile.level
    }

    fn stat_level(&self) -> i32 {
        self.snapshot
            .stats()
            .iter()
            .find(|stat| stat.name.eq_ignore_ascii_case(self.cell.skill_name()))
            .map_or(0, |stat| stat.base.max(stat.effective))
    }

    fn advance_prep(&mut self, client: &mut client::client::Client) -> Result<(), String> {
        match self.phase {
            Prep::WaitIngame => {
                if self.snapshot.ingame() && self.snapshot.scene_state() == 2 {
                    self.phase = Prep::TutSkip;
                }
            }
            Prep::TutSkip => {
                send_cheat(client, "setvar tutorial 1000")?;
                send_cheat(client, "getvar tutorial")?;
                self.phase = Prep::WaitTutorial;
            }
            Prep::WaitTutorial => {
                if self.chat_has("get tutorial: 1000") {
                    self.phase = Prep::Relog;
                }
            }
            Prep::Relog => {
                let ifaces = Arc::clone(&client.ifaces);
                if !interact::logout(client, &ifaces) {
                    return Err("logout interface unavailable during fixture preparation".into());
                }
                self.phase = Prep::WaitRelog;
            }
            Prep::WaitRelog => {
                if self.snapshot.ingame()
                    && self.snapshot.scene_state() == 2
                    && self.inventory_tab_available()
                {
                    self.phase = Prep::Seed;
                }
            }
            Prep::Seed => {
                send_cheat(
                    client,
                    &format!(
                        "setstat {} {}",
                        self.cell.skill_name(),
                        self.requested_level
                    ),
                )?;
                send_cheat(
                    client,
                    if self.cell == Cell::Mining {
                        "setstat attack 1"
                    } else {
                        "setstat attack 5"
                    },
                )?;
                // Isolate gathering from the mine's aggressive scorpions.
                send_cheat(client, "setstat defence 99")?;
                if (!self.fixture_helper || self.case == LiveCase::OakRespawn)
                    && !self.item_in_inventory()
                {
                    send_cheat(client, &format!("give {} 1", self.tool_alias))?;
                }
                send_cheat(client, "setstat hitpoints 99")?;
                let tile = self.prep_tile();
                send_cheat(client, &interact::tele_args(tile.level, tile.x, tile.z))?;
                self.phase = Prep::WaitSeed;
            }
            Prep::WaitSeed => {
                let prep_tile = self.prep_tile();
                if self.snapshot.ingame()
                    && self.snapshot.scene_state() == 2
                    && self.near_tile(prep_tile, 6)
                    && (self.fixture_helper && self.case != LiveCase::OakRespawn
                        || self.item_in_inventory())
                    && self.stat_level() >= self.requested_level
                {
                    if self.fixture_helper {
                        self.phase = if self.case == LiveCase::OakRespawn {
                            Prep::Equip
                        } else {
                            Prep::Ready
                        };
                    } else if self.case == LiveCase::OakNearEdge {
                        if !self.expected_preflight_build() {
                            return Err(format!(
                                "{} preflight build rectangle was not observed: {:?}",
                                self.name(),
                                self.latest.as_ref().and_then(|row| row.build)
                            ));
                        }
                        send_cheat(
                            client,
                            &interact::tele_args(self.target.level, self.target.x, self.target.z),
                        )?;
                        self.phase = Prep::WaitEdgeStart;
                    } else if self.cell == Cell::Mining && self.case == LiveCase::Power {
                        send_cheat(client, "givebank rune_pickaxe 1")?;
                        send_cheat(client, &interact::tele_args(0, 2809, 3441))?;
                        self.phase = Prep::OpenBank;
                    } else if matches!(self.cell, Cell::Fishing | Cell::Mining) {
                        self.phase = Prep::Ready;
                    } else {
                        self.phase = Prep::Equip;
                    }
                }
            }
            Prep::WaitEdgeStart => {
                if !self.expected_preflight_build() {
                    return Err(format!(
                        "{} build rectangle changed during near-edge placement",
                        self.name()
                    ));
                }
                if self.snapshot.ingame()
                    && self.snapshot.scene_state() == 2
                    && self.near_target()
                    && (self.fixture_helper || self.item_in_inventory())
                    && self.stat_level() >= self.requested_level
                {
                    self.phase = Prep::Equip;
                }
            }
            Prep::Equip => {
                if self.item_equipped() {
                    self.phase = Prep::Ready;
                } else if self.last_action.elapsed() >= Duration::from_millis(400) {
                    match Interactions::new(&self.snapshot, client).wear(self.tool_id) {
                        SendResult::Sent { .. } | SendResult::Refused { .. } => {
                            self.last_action = Instant::now();
                        }
                    }
                    self.phase = Prep::WaitEquip;
                }
            }
            Prep::WaitEquip => {
                if self.item_equipped() {
                    self.phase = Prep::Ready;
                } else if self.last_action.elapsed() >= Duration::from_millis(800) {
                    self.phase = Prep::Equip;
                }
            }
            Prep::OpenBank => {
                if self.snapshot.tile() == Some((2809, 3441, 0))
                    && self.last_action.elapsed() >= Duration::from_millis(800)
                {
                    if let Some(loc) = self.snapshot.locs().iter().find(|loc| {
                        loc.tile.x == 2809
                            && loc.tile.z == 3442
                            && loc.tile.level == 0
                            && loc.name.as_deref() == Some("Bank booth")
                    }) {
                        if matches!(
                            Interactions::new(&self.snapshot, client)
                                .open_booth_at(loc.tile, loc.id),
                            SendResult::Sent { .. }
                        ) {
                            self.phase = Prep::WaitBank;
                            self.last_action = Instant::now();
                        }
                    }
                }
            }
            Prep::WaitBank => {
                if self.snapshot.bank_loaded() {
                    if let Some(row) = self.snapshot.bank().iter().find(|row| {
                        row.def.name.as_deref() == Some("Rune pickaxe") && row.count == 1
                    }) {
                        self.witness.banked_unusable_tool = Some(row.def.id);
                        if matches!(
                            Interactions::new(&self.snapshot, client).close_modal(),
                            SendResult::Sent { .. }
                        ) {
                            self.phase = Prep::WaitBankClose;
                        }
                    }
                }
            }
            Prep::WaitBankClose => {
                if !self.snapshot.bank_loaded() && self.snapshot.bank_component_id() < 0 {
                    send_cheat(
                        client,
                        &interact::tele_args(self.target.level, self.target.x, self.target.z),
                    )?;
                    self.phase = Prep::ReturnFromBank;
                }
            }
            Prep::ReturnFromBank => {
                if self.near_target() && self.snapshot.scene_state() == 2 {
                    self.phase = Prep::Ready;
                }
            }
            Prep::Ready | Prep::Running => {}
        }
        Ok(())
    }

    fn mark_started(&mut self) -> Result<Map<String, Value>, String> {
        if self.phase != Prep::Ready {
            return Err(format!(
                "{} Start reached {:?}, not Ready",
                self.name(),
                self.phase
            ));
        }
        let baseline = self.latest.clone().ok_or("missing Start baseline")?;
        if baseline.product_count != 0 {
            return Err(format!(
                "{} Start baseline already has {} products",
                self.name(),
                baseline.product_count
            ));
        }
        if self.case == LiveCase::FishBaitGate
            && baseline
                .inventory
                .values()
                .any(|row| row.id == FEATHERS_ID && row.count > 0)
        {
            return Err("bait-gate fixture unexpectedly contains feathers".into());
        }
        let tool_ready = match self.cell {
            Cell::Woodcutting => self.item_equipped(),
            Cell::Mining if self.case == LiveCase::Power => {
                self.item_in_inventory() && self.witness.banked_unusable_tool.is_some()
            }
            Cell::Mining => self.item_in_inventory() || self.item_equipped(),
            Cell::Fishing => self.item_in_inventory(),
        };
        if !tool_ready {
            return Err(format!(
                "{} Start baseline lacks its required held/banked tool fixture",
                self.name()
            ));
        }
        if self.case == LiveCase::OakNearEdge {
            if let Some(build) = baseline.build {
                self.witness.initial_build = Some(build);
                self.witness.edge_initial_gap = self
                    .plan
                    .edge_oak
                    .and_then(|tile| build.outside_gap(tile_key(tile)));
                self.witness.edge_initial_distance =
                    baseline.tile.and_then(|tile| build.edge_distance(tile));
            }
        }
        self.baseline = Some(baseline);
        self.phase = Prep::Running;
        self.started = true;
        self.start_requested = false;
        Ok(self.cell.settings(self.case))
    }

    fn apply_status(&mut self, status: &script::native::ScriptStatus) -> Result<(), String> {
        if status.card != script::CompiledId("Gatherer") {
            return Err(format!(
                "unexpected card in {} status: {:?}",
                self.name(),
                status.card
            ));
        }
        let native_phase = format!("{:?}", status.phase);
        self.witness.native_phases.insert(native_phase.clone());
        for field in status.fields.iter() {
            let value = match &field.value {
                script::native::StatusValue::Text(value) => value.to_string(),
                script::native::StatusValue::Integer(value) => value.to_string(),
                script::native::StatusValue::Tile(tile) => {
                    format!("{},{},{}", tile.x, tile.z, tile.level)
                }
                script::native::StatusValue::Truth(value) => format!("{value:?}"),
                script::native::StatusValue::Quest(value) => format!("{value:?}"),
            };
            self.witness
                .status_values
                .entry(field.key.to_string())
                .or_default()
                .insert(value.clone());
            if field.key == "method" {
                self.witness.methods.insert(value.clone());
            } else if field.key == "target" {
                self.witness.targets.insert(value);
            }
        }
        self.witness.last_status_yielded =
            integer_field(status, "yielded").unwrap_or(self.witness.last_status_yielded);
        self.witness.last_status_dropped =
            integer_field(status, "dropped").unwrap_or(self.witness.last_status_dropped);
        if let Some(area) = text_field(status, "area") {
            self.witness
                .initial_area
                .get_or_insert_with(|| area.to_owned());
            self.witness.last_area = Some(area.to_owned());
            if let Some((_, deadline)) = area.split_once("; wait_until: ") {
                self.witness.wait_until = deadline.trim().parse().ok();
            }
        }
        if let Some(event) = text_field(status, "last_event") {
            self.witness.last_event = Some(event.to_owned());
            if self.case == LiveCase::LocationAuto && event == "widening Auto within 128 tiles" {
                self.witness.auto_widen_seen = true;
            }
            if self.case == LiveCase::OakNearEdge
                && event == "approaching target"
                && text_field(status, "phase") == Some("walking")
            {
                if let Some(latest) = self.latest.as_ref() {
                    // A build can load along the queued route before the next
                    // observed player position; retain the native walk request.
                    self.witness
                        .first_edge_oak_approach_tick
                        .get_or_insert(latest.tick);
                }
            }
            if self.case == LiveCase::GasHazard
                && (event.contains("hazard observed") || event == "walking away from hazard")
            {
                self.witness.gas_hazard_observed = true;
                if self.witness.gas_hazard_start_tile.is_none() {
                    self.witness.gas_hazard_start_tile =
                        self.latest.as_ref().and_then(|row| row.tile);
                }
            }
            if self.case == LiveCase::OakNearEdge
                && native_phase == "Waiting"
                && event == "waiting for a resource"
            {
                if let Some(latest) = self.latest.as_ref() {
                    self.witness.first_wait_tick.get_or_insert(latest.tick);
                }
            }
            if self.case == LiveCase::OakRespawn
                && native_phase == "Waiting"
                && event == "waiting for a resource"
            {
                if let Some(latest) = self.latest.as_ref() {
                    let all_felled = self.plan.oak_tiles.iter().all(|tile| {
                        latest.oak_stumps.contains(&tile_key(*tile))
                            && !latest.live_oaks.contains(&tile_key(*tile))
                    });
                    if all_felled {
                        self.witness.all_oaks_depleted_at_wait = true;
                        self.witness.first_wait_tick.get_or_insert(latest.tick);
                        self.witness
                            .first_wait_native_tick
                            .get_or_insert(self.native_tick);
                    }
                }
            }
            if self.case == LiveCase::OakRespawn
                && event == "gathering"
                && self.witness.first_regrown_gather_tick.is_none()
            {
                if let (Some(latest), Some(tile)) =
                    (self.latest.as_ref(), self.witness.first_regrown_tile)
                {
                    let target = WorldTile {
                        x: tile.0,
                        z: tile.1,
                        level: tile.2,
                    };
                    if latest
                        .tile
                        .and_then(|here| tile_distance(here, target))
                        .is_some_and(|distance| distance <= 1)
                    {
                        self.witness.first_regrown_gather_tick = Some(latest.tick);
                    }
                }
            }
        }

        if self.case == LiveCase::FishBaitGate {
            self.witness.no_fish_xp = self.baseline.as_ref().is_some_and(|baseline| {
                self.latest.as_ref().is_some_and(|latest| {
                    latest.xp == baseline.xp
                        && latest.product_count == 0
                        && self.witness.last_status_yielded == 0
                        && !self.witness.script_moved_after_start
                })
            });
        }
        if let Some(failure) = &status.failure {
            let code = failure.code.to_string();
            let message = failure.message.to_string();
            self.witness.failure = Some(format!("{code}: {message}"));
            self.witness.failure_code = Some(code.clone());
            self.witness.failure_message = Some(message.clone());
            if self.case == LiveCase::DeathDuringDrop && code == "died" {
                self.witness.death_blocked = true;
                return Ok(());
            }
            let expected_block = match self.case {
                LiveCase::FishBaitGate => code == "supply-missing",
                LiveCase::OakAbsentArea => {
                    code == "resource-unavailable" && message.starts_with("resource-unavailable")
                }
                _ => false,
            };
            if expected_block {
                return Ok(());
            }
            return Err(format!("{} script failure: {failure:?}", self.name()));
        }
        if self.witness.death_restart_tile.is_some()
            && self.witness.last_status_yielded > 0
            && self.witness.last_xp > self.witness.death_restart_xp
        {
            self.witness.death_restart_gathered = true;
        }
        if self.witness.awaiting_drop {
            let empty = self
                .latest
                .as_ref()
                .is_some_and(|observation| observation.product_count == 0);
            let dropped = self.witness.last_status_dropped.max(0) as u32;
            if empty
                && dropped > self.witness.cycle_drop_start
                && self
                    .witness
                    .confirmed_drops
                    .saturating_sub(self.witness.cycle_confirmed_start)
                    >= self.witness.cycle_product_slots
            {
                self.witness.cycles = self.witness.cycles.saturating_add(1);
                self.witness.awaiting_drop = false;
                self.witness.awaiting_post_drop_gather = true;
                self.witness.cycle_xp_start = self
                    .latest
                    .as_ref()
                    .map_or(self.witness.cycle_xp_start, |observation| observation.xp);
            }
        }
        if self.witness.awaiting_post_drop_gather {
            if let Some(observation) = self.latest.as_ref() {
                if observation.product_count > 0 && observation.xp > self.witness.cycle_xp_start {
                    self.witness.post_drop_gathers =
                        self.witness.post_drop_gathers.saturating_add(1);
                    self.witness.awaiting_post_drop_gather = false;
                    self.witness.cycle_xp_start = observation.xp;
                }
            }
        }
        Ok(())
    }

    fn qualifies_power_cycles(&self) -> Result<(), String> {
        if self.witness.cycles < REQUIRED_CYCLES {
            return Err(format!(
                "{} only completed {} full cycles (full packs {}, confirmed drops {})",
                self.name(),
                self.witness.cycles,
                self.witness.full_pack_seen,
                self.witness.confirmed_drops
            ));
        }
        if self.witness.post_drop_gathers < REQUIRED_POST_DROP_GATHERS {
            return Err(format!(
                "{} only observed {} post-disposal gathers",
                self.name(),
                self.witness.post_drop_gathers
            ));
        }
        if self.witness.confirmed_drops < REQUIRED_CYCLES * self.cycle_product_capacity() {
            return Err(format!(
                "{} confirmed only {} empty product slots",
                self.name(),
                self.witness.confirmed_drops
            ));
        }
        if self.witness.last_status_yielded <= 0 || self.witness.last_xp <= self.baseline_xp() {
            return Err(format!(
                "{} had no observed yield/XP: yielded={} xp={}",
                self.name(),
                self.witness.last_status_yielded,
                self.witness.last_xp
            ));
        }
        if self.cell == Cell::Mining
            && self.witness.methods.len() < 2
            && self.cell.resources(self.case).len() >= 2
        {
            return Err(format!(
                "{} did not show tier preference/depletion fallback; methods={:?} targets={:?}",
                self.name(),
                self.witness.methods,
                self.witness.targets
            ));
        }
        Ok(())
    }

    fn qualifies(&self) -> Result<(), String> {
        match self.case {
            LiveCase::Power => {
                if !self.witness.modal_observed {
                    return Err("power fixture did not observe its disposal modal".into());
                }
                self.qualifies_power_cycles()
            }
            LiveCase::FishNet => {
                self.qualifies_power_cycles()?;
                if !self.witness.moved_spot_reacquired
                    || self.witness.fish_targets.len() < 2
                    || self.witness.fish_products_seen.is_empty()
                {
                    return Err(format!(
                        "{} did not observe a moved fishing spot reacquired with a fish yield: {:?}",
                        self.name(),
                        self.witness
                    ));
                }
                Ok(())
            }
            LiveCase::FishBaitGate => {
                let no_output = self.witness.last_status_yielded == 0
                    && self.witness.last_status_dropped == 0
                    && !self.witness.fish_target_seen
                    && self.witness.products_seen.is_empty()
                    && !self.witness.script_moved_after_start
                    && self.latest.as_ref().is_some_and(|latest| {
                        latest.product_count == 0 && latest.xp == self.baseline_xp()
                    });
                if !self.witness.native_phases.contains("Blocked")
                    || self.witness.failure_code.as_deref() != Some("supply-missing")
                    || !self.witness.no_fish_xp
                    || !no_output
                {
                    return Err(format!(
                        "{} did not Blocked(supply-missing) before fishing: {:?}",
                        self.name(),
                        self.witness
                    ));
                }
                Ok(())
            }
            LiveCase::OakRespawn => {
                let expected_wait_until = self
                    .witness
                    .first_wait_native_tick
                    .map(|tick| tick + u64::from(OAK_RESPAWN_MAX_TICKS));
                let regrowth_gathered = self
                    .witness
                    .first_regrown_tick
                    .zip(self.witness.first_regrown_gather_tick)
                    .is_some_and(|(regrown, gathered)| gathered >= regrown);
                let regrowth_reselected_immediately = self
                    .witness
                    .first_regrown_tick
                    .zip(self.witness.first_regrown_approach_tick)
                    .is_some_and(|(regrown, approach)| {
                        approach >= regrown && approach - regrown <= 4
                    })
                    || self
                        .witness
                        .first_regrown_tick
                        .zip(self.witness.first_regrown_gather_tick)
                        .is_some_and(|(regrown, gathered)| {
                            gathered >= regrown && gathered - regrown <= 4
                        });
                let regrown_known_stump = self
                    .witness
                    .first_regrown_tile
                    .is_some_and(|tile| self.witness.oak_stump_tiles.contains(&tile));
                let both_oaks_felled =
                    self.plan.first_oak.is_some_and(|tile| {
                        self.witness.fixture_chop_tiles.contains(&tile_key(tile))
                    }) && self.plan.other_oak.is_some_and(|tile| {
                        self.witness.fixture_chop_tiles.contains(&tile_key(tile))
                    });
                if !self.witness.fixture_chop_observed
                    || !both_oaks_felled
                    || self.witness.oak_other_gather_tick.is_none()
                    || !self.witness.all_oaks_depleted_at_wait
                    || !self.witness.native_phases.contains("Waiting")
                    || self.witness.wait_until != expected_wait_until
                    || !regrown_known_stump
                    || !regrowth_gathered
                    || !regrowth_reselected_immediately
                {
                    return Err(format!(
                        "{} did not prove other-oak yield, respawn-bounded all-stump wait, and immediate regrown-oak gather: {:?}",
                        self.name(),
                        self.witness
                    ));
                }
                Ok(())
            }
            LiveCase::OakNearEdge => {
                let approach_preceded_load = self
                    .witness
                    .first_edge_oak_approach_tick
                    .zip(self.witness.first_edge_oak_loaded_tick)
                    .is_some_and(|(approach, loaded)| approach <= loaded);
                let no_wait_before_load = self.witness.first_wait_tick.is_none_or(|wait| {
                    self.witness
                        .first_edge_oak_loaded_tick
                        .is_some_and(|loaded| loaded <= wait)
                });
                if !self.witness.preflight_build_observed
                    || !self.witness.edge_initial_gap.is_some_and(|gap| gap > 0)
                    || !self
                        .witness
                        .edge_initial_distance
                        .is_some_and(|distance| distance <= 20)
                    || !approach_preceded_load
                    || !no_wait_before_load
                    || !self.witness.edge_fresh_xp
                {
                    return Err(format!(
                        "{} did not approach and load the eligible edge oak before waiting, then gather: {:?}",
                        self.name(),
                        self.witness
                    ));
                }
                Ok(())
            }
            LiveCase::LocationAuto => {
                let next_group_beyond_start_ring = self.plan.auto_next.iter().any(|tile| {
                    tile_distance(tile_key(self.target), *tile)
                        .is_some_and(|distance| (33..=128).contains(&distance))
                });
                if self.witness.fixture_seeds_observed < self.plan.seed_locs.len()
                    || !next_group_beyond_start_ring
                    || !self.witness.auto_widen_seen
                    || !self.witness.auto_selected_next
                    || self.witness.auto_next_gather_tick.is_none()
                    || self.witness.last_status_yielded <= 0
                    || self.witness.last_xp <= self.baseline_xp()
                {
                    return Err(format!(
                        "{} did not seed all felled Auto placements, widen, and gather at the next group: {:?}",
                        self.name(),
                        self.witness
                    ));
                }
                Ok(())
            }
            LiveCase::OakAbsentArea => {
                let absent_seen = self
                    .witness
                    .status_values
                    .get("absent")
                    .is_some_and(|values| {
                        values
                            .iter()
                            .any(|value| value.parse::<i64>().is_ok_and(|count| count > 0))
                    });
                let no_output = self.witness.last_status_yielded == 0
                    && self.witness.last_status_dropped == 0
                    && !self.witness.script_moved_after_start
                    && self.latest.as_ref().is_some_and(|latest| {
                        latest.product_count == 0 && latest.xp == self.baseline_xp()
                    });
                if !self.witness.native_phases.contains("Blocked")
                    || self.witness.failure_code.as_deref() != Some("resource-unavailable")
                    || !absent_seen
                    || self.witness.native_phases.contains("Waiting")
                    || self.witness.wait_until.is_some()
                    || !no_output
                {
                    return Err(format!(
                        "{} did not fail promptly on the observed absent-only area: {:?}",
                        self.name(),
                        self.witness
                    ));
                }
                Ok(())
            }
            LiveCase::GasHazard => {
                let expected_hazard = self
                    .plan
                    .inject_after_progress
                    .map(|seed| tile_key(seed.tile));
                if !self.witness.gas_hazard_seeded
                    || !self.witness.gas_hazard_observed
                    || self.witness.gas_hazard_tile != expected_hazard
                    || !self.witness.gas_hazard_escaped
                    || !self.witness.gas_hazard_xp_after_escape
                {
                    return Err(format!(
                        "{} did not observe the seeded gas hazard, escape, and resume yield: {:?}",
                        self.name(),
                        self.witness
                    ));
                }
                Ok(())
            }
            LiveCase::CancelBeforeDrain => {
                if !self.witness.stopped_before_drain || self.witness.confirmed_drops != 0 {
                    return Err(format!(
                        "{} cancellation did not clear the pre-drain drop outbox: {:?}",
                        self.name(),
                        self.witness
                    ));
                }
                Ok(())
            }
            LiveCase::DeathDuringDrop => {
                if !self.witness.death_command_sent
                    || !self.witness.death_blocked
                    || !self.witness.death_restart_gathered
                {
                    return Err(format!(
                        "{} death-during-drop did not settle Blocked(died): {:?}",
                        self.name(),
                        self.witness
                    ));
                }
                Ok(())
            }
            LiveCase::RunKeyChangeDuringDrop => {
                if !self.witness.run_key_changed
                    || !self.witness.stopped_before_drain
                    || self.witness.confirmed_drops < 28
                {
                    return Err(format!(
                        "{} run-key wedge did not stop before drain and restart: {:?}",
                        self.name(),
                        self.witness
                    ));
                }
                Ok(())
            }
        }
    }

    fn baseline_xp(&self) -> i32 {
        self.baseline
            .as_ref()
            .map_or(0, |observation| observation.xp)
    }
}

fn integer_field(status: &script::native::ScriptStatus, key: &str) -> Option<i64> {
    status
        .fields
        .iter()
        .find(|field| field.key == key)
        .and_then(|field| match &field.value {
            script::native::StatusValue::Integer(value) => Some(*value),
            _ => None,
        })
}

fn text_field<'a>(status: &'a script::native::ScriptStatus, key: &str) -> Option<&'a str> {
    status
        .fields
        .iter()
        .find(|field| field.key == key)
        .and_then(|field| match &field.value {
            script::native::StatusValue::Text(value) => Some(value.as_ref()),
            _ => None,
        })
}

fn tile_key(tile: WorldTile) -> (i32, i32, i32) {
    (tile.x, tile.z, tile.level)
}
fn tile_distance(tile: (i32, i32, i32), target: WorldTile) -> Option<i32> {
    (tile.2 == target.level).then(|| (tile.0 - target.x).abs().max((tile.1 - target.z).abs()))
}

fn send_cheat(client: &mut client::client::Client, command: &str) -> Result<(), String> {
    if interact::cheat(client, command).is_sent() {
        Ok(())
    } else {
        Err(format!("fixture command refused: {command}"))
    }
}

struct TempRoot(PathBuf);

impl TempRoot {
    fn new(label: &str) -> Result<Self, String> {
        let serial = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| format!("clock: {error}"))?
            .as_nanos();
        let path = std::env::temp_dir().join(format!("274bot-gatherer-{label}-{serial}"));
        std::fs::create_dir_all(&path)
            .map_err(|error| format!("create {}: {error}", path.display()))?;
        Ok(Self(path))
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempRoot {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn required(name: &str) -> Result<String, String> {
    std::env::var(name).map_err(|_| format!("{name} is required for Gatherer live qualification"))
}

fn parse_tile(name: &str, raw: &str) -> Result<WorldTile, String> {
    let mut parts = raw.split(',').map(str::trim);
    let x = parts
        .next()
        .ok_or_else(|| format!("{name} must be x,z,level"))?
        .parse()
        .map_err(|_| format!("{name} has invalid x: {raw}"))?;
    let z = parts
        .next()
        .ok_or_else(|| format!("{name} must be x,z,level"))?
        .parse()
        .map_err(|_| format!("{name} has invalid z: {raw}"))?;
    let level = parts
        .next()
        .ok_or_else(|| format!("{name} must be x,z,level"))?
        .parse()
        .map_err(|_| format!("{name} has invalid level: {raw}"))?;
    if parts.next().is_some() {
        return Err(format!("{name} must be x,z,level: {raw}"));
    }
    Ok(WorldTile { x, z, level })
}
fn fixture_tile(name: &str, default: WorldTile) -> Result<WorldTile, String> {
    match std::env::var(name) {
        Ok(raw) => parse_tile(name, &raw),
        Err(std::env::VarError::NotPresent) => Ok(default),
        Err(error) => Err(format!("{name} is not valid UTF-8: {error}")),
    }
}

fn seed(tile: WorldTile, alias: &'static str, id: i32) -> LocSeed {
    LocSeed { tile, alias, id }
}

fn world_tile(x: i32, z: i32) -> WorldTile {
    WorldTile { x, z, level: 0 }
}

fn fixture_plan(
    cell: Cell,
    case: LiveCase,
) -> Result<(WorldTile, FixturePlan, Option<FixtureTask>), String> {
    let mut plan = FixturePlan::default();
    let target = match case {
        LiveCase::FishNet => fixture_tile("GATHERER_FISH_TILE", FISH_NET_START)?,
        LiveCase::FishBaitGate => fixture_tile("GATHERER_FISH_TILE", FISH_BAIT_START)?,
        LiveCase::OakRespawn => {
            let first = fixture_tile("GATHERER_WC_RESPAWN_TARGET_TILE", OAK_RESPAWN_START)?;
            let other = fixture_tile("GATHERER_WC_RESPAWN_OTHER_TILE", OAK_RESPAWN_OTHER)?;
            plan.oak_tiles = vec![first, other];
            plan.first_oak = Some(first);
            plan.other_oak = Some(other);
            plan.chop_target = Some(first);
            fixture_tile("GATHERER_WC_RESPAWN_START_TILE", first)?
        }
        LiveCase::OakNearEdge => {
            let edge = fixture_tile("GATHERER_WC_EDGE_OAK_TILE", OAK_EDGE_TARGET)?;
            plan.edge_oak = Some(edge);
            plan.preflight_edge = Some(fixture_tile(
                "GATHERER_WC_EDGE_PREFLIGHT_TILE",
                OAK_EDGE_PREFLIGHT,
            )?);
            plan.oak_tiles.push(edge);
            // Earlier absent/edge cells may have replaced this same placement.
            plan.seed_locs.push(seed(edge, "oaktree", OAK_ID));
            for &(x, z) in OAK_EDGE_SEEDS {
                let tile = world_tile(x, z);
                if tile == world_tile(3252, 3237) {
                    plan.seed_locs.push(seed(tile, "fire", ABSENT_LOC_ID));
                } else {
                    plan.oak_tiles.push(tile);
                    plan.seed_locs
                        .push(seed(tile, "evergreen_large_stump", OAK_STUMP_ID));
                }
            }
            fixture_tile("GATHERER_WC_EDGE_START_TILE", OAK_EDGE_START)?
        }
        LiveCase::OakAbsentArea => {
            let tile = fixture_tile("GATHERER_WC_ABSENT_TILE", OAK_ABSENT_START)?;
            plan.oak_tiles.push(tile);
            plan.seed_locs.push(seed(tile, "fire", ABSENT_LOC_ID));
            tile
        }
        LiveCase::LocationAuto => {
            plan.auto_felled = AUTO_FELLED_TILES
                .iter()
                .map(|&(x, z)| world_tile(x, z))
                .collect();
            plan.auto_next = if std::env::var_os("GATHERER_AUTO_NEXT_TILE").is_some() {
                vec![fixture_tile(
                    "GATHERER_AUTO_NEXT_TILE",
                    world_tile(2409, 3480),
                )?]
            } else {
                AUTO_NEXT_TILES
                    .iter()
                    .map(|&(x, z)| world_tile(x, z))
                    .collect()
            };
            plan.seed_locs = plan
                .auto_felled
                .iter()
                .copied()
                .map(|tile| seed(tile, "treestump2", AUTO_STUMP_ID))
                .collect();
            fixture_tile("GATHERER_AUTO_START_TILE", AUTO_START)?
        }
        LiveCase::GasHazard => {
            let tile = fixture_tile("GATHERER_GAS_TILE", GAS_START)?;
            let hazard = seed(tile, "macro_ironrock1", GAS_HAZARD_LOC_ID);
            plan.inject_after_progress = Some(hazard);
            tile
        }
        _ => parse_tile(cell.tile_env(case), &required(cell.tile_env(case))?)?,
    };
    let task = match case {
        LiveCase::OakRespawn => plan.chop_target.map(|tile| FixtureTask::Chop {
            tile,
            stage: FixtureStage::Start,
        }),
        LiveCase::OakNearEdge | LiveCase::OakAbsentArea | LiveCase::LocationAuto => {
            Some(FixtureTask::Seed {
                seeds: plan.seed_locs.clone(),
                next: 0,
                return_to: target,
                stage: FixtureStage::Start,
            })
        }
        LiveCase::GasHazard => {
            plan.inject_after_progress
                .map(|seed| FixtureTask::InjectAfterProgress {
                    seed,
                    tile: None,
                    return_to: None,
                    stage: FixtureStage::Start,
                })
        }
        _ => None,
    };
    Ok((target, plan, task))
}
fn mint_g2_names(n: usize) -> Vec<String> {
    host_play::mint_live_names(n)
        .into_iter()
        .map(|name| {
            let suffix = name
                .strip_prefix("live")
                .expect("host-play live name prefix");
            format!("g2{suffix}")
        })
        .collect()
}

fn mint_profile(account: &str, password: &str, offset: i32) -> Result<Profile, String> {
    let uid = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("clock: {error}"))?
        .as_millis()
        .checked_rem(i32::MAX as u128)
        .ok_or("uid clock overflow")? as i32;
    Ok(Profile {
        username: account.to_owned(),
        password: password.to_owned().into(),
        uid: uid.saturating_add(offset),
        settings: ProfileSettings::default(),
    })
}

fn selected_profile(
    nav_pack: PathBuf,
    engine_dir: PathBuf,
    catalog_root: PathBuf,
    temp: &Path,
) -> Result<(Arc<host_play::ServerProfile>, Arc<SharedClientTemplate>), String> {
    let options = ProfileOptions {
        profile: Some("local-289".into()),
        revision: Some("289".into()),
        host: Some("127.0.0.1".into()),
        asset_host: Some("127.0.0.1".into()),
        port: Some(
            std::env::var("GATHERER_GAME_PORT")
                .ok()
                .and_then(|value| value.parse().ok())
                .unwrap_or(45594),
        ),
        http_port: Some(
            std::env::var("GATHERER_HTTP_PORT")
                .ok()
                .and_then(|value| value.parse().ok())
                .unwrap_or(2080),
        ),
        nav_pack: Some(nav_pack),
        nav_flags: std::env::var_os("GATHERER_NAV_FLAGS").map(PathBuf::from),
        engine_dir: Some(engine_dir),
        vault_path: Some(temp.join("vault")),
        catalog_root: Some(catalog_root),
        ..ProfileOptions::default()
    };
    let profile = options.resolve(None)?.bind()?;
    if profile.profile_class() != host_play::ProfileClass::Local
        || profile.client().game_host() != "127.0.0.1"
    {
        return Err("Gatherer live qualification requires a loopback local profile".into());
    }
    let template = SharedClientTemplate::load(Arc::clone(&profile))?;
    if template.world().is_none() {
        return Err("selected template has no navigation world".into());
    }
    Ok((profile, template))
}

fn run_cell(cell: Cell, case: LiveCase) -> Result<(), String> {
    if std::env::var("LIVE").as_deref() != Ok("1") {
        return Ok(());
    }
    let cell_name = cell.name_for_case(case);
    let _home = script::IsolatedEnv::enter(cell_name);
    api::hostlog::set_debug(true);
    let nav_pack = PathBuf::from(required("GATHERER_NAV_PACK")?);
    let engine_dir = PathBuf::from(required("GATHERER_ENGINE_DIR")?);
    let catalog_root = PathBuf::from(required("GATHERER_CATALOG_ROOT")?);
    for (name, path) in [
        ("GATHERER_NAV_PACK", &nav_pack),
        ("GATHERER_ENGINE_DIR", &engine_dir),
        ("GATHERER_CATALOG_ROOT", &catalog_root),
    ] {
        if !path.is_absolute() {
            return Err(format!(
                "{name} must be an absolute path: {}",
                path.display()
            ));
        }
    }
    if !nav_pack.is_file() {
        return Err(format!(
            "GATHERER_NAV_PACK is not a file: {}",
            nav_pack.display()
        ));
    }
    if !engine_dir.is_dir() {
        return Err(format!(
            "GATHERER_ENGINE_DIR is not a directory: {}",
            engine_dir.display()
        ));
    }
    if !catalog_root.is_dir() {
        return Err(format!(
            "GATHERER_CATALOG_ROOT is not a directory: {}",
            catalog_root.display()
        ));
    }
    let (target, plan, fixture_task) = fixture_plan(cell, case)?;
    let temp = TempRoot::new(cell_name)?;
    let (profile, template) = selected_profile(nav_pack, engine_dir, catalog_root, temp.path())?;
    let helper_needed = fixture_task.is_some();
    let names = mint_g2_names(if helper_needed { 2 } else { 1 });
    let credentials = host_play::mint_live_entries(&names);
    let account = names.first().cloned().ok_or("failed to mint account")?;
    let password = credentials
        .first()
        .map(|entry| entry.1.clone())
        .ok_or("failed to mint password")?;
    let helper_account = helper_needed.then(|| names[1].clone());
    let helper_password = helper_needed.then(|| credentials[1].1.clone());
    let state = Arc::new(Mutex::new(GatherSlot::new(
        cell,
        case,
        target,
        plan.clone(),
        // The engine streams dynamic loc changes only in its nearby zone window.
        // Visit each seeded zone before Start without rebuilding the 104-tile scene,
        // so all forty exhausted placements are genuinely observed by the subject.
        matches!(case, LiveCase::LocationAuto | LiveCase::OakNearEdge).then(|| FixtureTask::Seed {
            seeds: plan
                .seed_locs
                .iter()
                .copied()
                .filter(|seed| case == LiveCase::LocationAuto || seed.id == ABSENT_LOC_ID)
                .collect(),
            next: 0,
            return_to: target,
            stage: FixtureStage::Start,
        }),
        false,
    )));
    let helper_state = fixture_task.map(|task| {
        Arc::new(Mutex::new(GatherSlot::new(
            cell,
            case,
            target,
            plan,
            Some(task),
            true,
        )))
    });
    let start_handle: Arc<Mutex<Option<ScriptStartHandle>>> = Arc::new(Mutex::new(None));
    let frame_state = Arc::clone(&state);
    let frame_handle = Arc::clone(&start_handle);
    let frame_account = account.clone();
    let frame_helper_account = helper_account.clone();
    let frame_helper_state = helper_state.clone();
    let frame_helper_needs_progress = case == LiveCase::GasHazard;
    let mut play = host_play::run_with_template(
        Arc::clone(&template),
        true,
        vec![],
        |_| (None, None),
        move |client, username, hold| {
            if frame_helper_account.as_deref() == Some(username) {
                let progress = if frame_helper_needs_progress {
                    frame_state.lock().ok().map(|slot| slot.fixture_progress())
                } else {
                    None
                };
                if let Some(helper_state) = &frame_helper_state {
                    if let Ok(mut helper) = helper_state.lock() {
                        helper.frame(client, hold, progress.as_ref());
                        if let Ok(mut main) = frame_state.lock() {
                            if helper.case == LiveCase::GasHazard {
                                main.plan.inject_after_progress = helper.plan.inject_after_progress;
                            }
                            main.witness.fixture_chop_observed |=
                                helper.witness.fixture_chop_observed;
                            main.witness
                                .fixture_chop_tiles
                                .extend(helper.witness.fixture_chop_tiles.iter().copied());
                            main.witness.fixture_seeds_observed = main
                                .witness
                                .fixture_seeds_observed
                                .max(helper.witness.fixture_seeds_observed);
                        }
                    }
                }
                return;
            }
            if username != frame_account {
                return;
            }
            let (do_wedge, open_modal) = frame_state
                .lock()
                .ok()
                .map(|slot| {
                    (
                        !hold && slot.pre_drain_wedge(),
                        !hold
                            && slot.case == LiveCase::Power
                            && slot.witness.awaiting_drop
                            && !slot.witness.modal_command_sent,
                    )
                })
                .unwrap_or((false, false));
            if open_modal && interact::cheat(client, "openmain book4").is_sent() {
                if let Ok(mut slot) = frame_state.lock() {
                    slot.witness.modal_command_sent = true;
                }
            }
            if do_wedge {
                let mode = frame_state.lock().ok().map(|slot| slot.case);
                match mode {
                    Some(LiveCase::CancelBeforeDrain | LiveCase::RunKeyChangeDuringDrop) => {
                        if let Some(handle) =
                            frame_handle.lock().ok().and_then(|guard| guard.clone())
                        {
                            if handle.stop(&frame_account).is_ok() {
                                if let Ok(mut slot) = frame_state.lock() {
                                    slot.mark_stop_wedge();
                                }
                            }
                        }
                    }
                    Some(LiveCase::DeathDuringDrop)
                        if interact::cheat(client, "~death").is_sent() =>
                    {
                        if let Ok(mut slot) = frame_state.lock() {
                            slot.mark_death_wedge();
                        }
                    }
                    _ => {}
                }
            }
            if let Ok(mut slot) = frame_state.lock() {
                slot.frame(client, hold, None);
            }
        },
    )?;
    if let Ok(mut handle) = start_handle.lock() {
        *handle = Some(play.script_start_handle());
    }
    play.try_spawn_slot(mint_profile(&account, &password, 1)?, None, None, None)?;
    if let (Some(helper_account), Some(helper_password)) =
        (helper_account.as_ref(), helper_password.as_ref())
    {
        play.try_spawn_slot(
            mint_profile(helper_account, helper_password, 2)?,
            None,
            None,
            None,
        )?;
    }
    let settings = cell.settings(case);
    println!(
        "{}",
        json!({
            "phase": "identity",
            "cell": cell_name,
            "live_case": case.name(),
            "profile": profile.label(),
            "game_port": profile.client().game_port(),
            "nav_pack": profile.nav_pack(),
            "account": account,
            "fixture_account": helper_account.as_deref(),
            "target": {"x": target.x, "z": target.z, "level": target.level},
            "settings": settings,
            "driver_trace": "enabled; native-packet account/tick/count lines are the packet witness",
        })
    );
    let started_at = Instant::now();
    let mut restart_requested = false;
    let mut death_stop_requested = false;
    let mut prior_run = None;
    let mut reported = (0, 0, 0);
    let mut oak_all_stumps_prepared = false;
    let result = loop {
        let lifecycle_error = play.script_last_error(&account);
        let (phase, start_requested, ready_to_start, slot_error, witness) = {
            let slot = state.lock().map_err(|_| "live state poisoned")?;
            (
                slot.phase,
                slot.start_requested,
                slot.ready_to_start(),
                slot.error.clone(),
                slot.witness.clone(),
            )
        };
        let (mut helper_done, helper_error) = if let Some(helper_state) = &helper_state {
            let helper = helper_state.lock().map_err(|_| "fixture state poisoned")?;
            (helper.fixture_done, helper.error.clone())
        } else {
            (true, None)
        };
        if let Some(error) = helper_error {
            break Err(format!("{cell_name} fixture helper: {error}"));
        }
        if case == LiveCase::OakRespawn
            && witness.oak_other_gather_tick.is_some()
            && !oak_all_stumps_prepared
        {
            let (all_stumps, both_chopped, next_chop) =
                {
                    let slot = state.lock().map_err(|_| "live state poisoned")?;
                    let latest = slot.latest.as_ref();
                    let all_stumps = latest.is_some_and(|latest| {
                        slot.plan.oak_tiles.iter().all(|tile| {
                            let key = tile_key(*tile);
                            latest.oak_stumps.contains(&key) && !latest.live_oaks.contains(&key)
                        })
                    });
                    let both_chopped = slot.plan.first_oak.zip(slot.plan.other_oak).is_some_and(
                        |(first, other)| {
                            slot.witness.fixture_chop_tiles.contains(&tile_key(first))
                                && slot.witness.fixture_chop_tiles.contains(&tile_key(other))
                        },
                    );
                    let next_chop = latest.and_then(|latest| {
                        slot.plan.oak_tiles.iter().copied().find(|tile| {
                            let key = tile_key(*tile);
                            (latest.oak_stumps.contains(&key) || latest.live_oaks.contains(&key))
                                && (!slot.witness.fixture_chop_tiles.contains(&key)
                                    || latest.live_oaks.contains(&key))
                        })
                    });
                    (all_stumps, both_chopped, next_chop)
                };
            if all_stumps && both_chopped {
                oak_all_stumps_prepared = true;
            } else if helper_done {
                if let (Some(helper_state), Some(tile)) = (helper_state.as_ref(), next_chop) {
                    let mut helper = helper_state.lock().map_err(|_| "fixture state poisoned")?;
                    helper.fixture_done = false;
                    helper.fixture_task = Some(FixtureTask::Chop {
                        tile,
                        stage: FixtureStage::Start,
                    });
                    helper_done = false;
                }
            }
        }
        let ready_to_start = ready_to_start && (case == LiveCase::GasHazard || helper_done);
        if let Some(error) = lifecycle_error {
            let death_receipt = case == LiveCase::DeathDuringDrop && witness.death_command_sent;
            if !death_receipt {
                break Err(format!("{} lifecycle error: {error}", cell_name));
            }
        }
        if let Some(error) = slot_error {
            break Err(error);
        }
        if ready_to_start && !start_requested {
            let start_bag = {
                let mut slot = state.lock().map_err(|_| "live state poisoned")?;
                slot.start_requested = true;
                match slot.mark_started() {
                    Ok(settings) => settings,
                    Err(error) => break Err(error),
                }
            };
            if let Err(error) =
                play.script_start(&account, script::CompiledId("Gatherer"), start_bag)
            {
                break Err(format!("{} Start failed: {error}", cell_name));
            }
            println!("{}", json!({"phase": "start", "cell": cell_name}));
        }
        let current_run = play.script_native_run(&account);
        if case == LiveCase::DeathDuringDrop && witness.death_blocked {
            if !death_stop_requested {
                play.script_stop(&account);
                death_stop_requested = true;
            } else if !restart_requested && current_run.is_none() {
                let mut slot = state.lock().map_err(|_| "live state poisoned")?;
                let respawned = slot.snapshot.ingame()
                    && slot.snapshot.scene_state() == 2
                    && slot.snapshot.tile().is_some_and(|(x, z, level)| {
                        level == 0 && (x - 3222).abs().max((z - 3218).abs()) < 12
                    })
                    && slot.snapshot.stats().iter().any(|stat| {
                        stat.name.eq_ignore_ascii_case("Hitpoints") && stat.effective > 0
                    });
                if respawned {
                    slot.witness.death_restart_tile = slot.snapshot.tile();
                    slot.witness.death_restart_xp = slot.latest.as_ref().map_or(0, |row| row.xp);
                    slot.witness.last_status_yielded = 0;
                    slot.reset_after_stop_for_restart();
                    // The castle respawn courtyard has no admitted normal tree
                    // within twelve tiles; this fresh Start selects a wider area.
                    let mut restart_settings = settings.clone();
                    restart_settings.insert("radius".into(), json!(32));
                    println!(
                        "{}",
                        json!({
                            "phase": "death-restart",
                            "tile": slot.witness.death_restart_tile,
                            "xp": slot.witness.death_restart_xp,
                            "radius": 32,
                        })
                    );
                    drop(slot);
                    if let Err(error) = play.script_start(
                        &account,
                        script::CompiledId("Gatherer"),
                        restart_settings,
                    ) {
                        break Err(format!("{} death restart failed: {error}", cell_name));
                    }
                    restart_requested = true;
                }
            }
        }
        if case == LiveCase::RunKeyChangeDuringDrop && !witness.wedge_triggered {
            if let Some(run) = current_run {
                prior_run = Some(run);
            }
        }
        if case == LiveCase::RunKeyChangeDuringDrop
            && witness.stopped_before_drain
            && !restart_requested
            && current_run.is_none()
        {
            if let Err(error) =
                play.script_start(&account, script::CompiledId("Gatherer"), settings.clone())
            {
                break Err(format!("{} restart failed: {error}", cell_name));
            }
            restart_requested = true;
            if let Ok(mut slot) = state.lock() {
                slot.reset_after_stop_for_restart();
            }
        }
        if case == LiveCase::RunKeyChangeDuringDrop
            && witness.stopped_before_drain
            && restart_requested
        {
            if let (Some(old), Some(current)) = (prior_run, current_run) {
                if old != current {
                    if let Ok(mut slot) = state.lock() {
                        slot.witness.run_key_changed = true;
                    }
                }
            }
        }
        if let Some(status) = play.script_native_status(&account) {
            let mut slot = state.lock().map_err(|_| "live state poisoned")?;
            if let Err(error) = slot.apply_status(&status) {
                slot.error = Some(error);
            }
        }
        let (witness, latest, product_capacity, baseline_xp) = {
            let slot = state.lock().map_err(|_| "live state poisoned")?;
            (
                slot.witness.clone(),
                slot.latest.clone(),
                slot.cycle_product_capacity(),
                slot.baseline_xp(),
            )
        };
        if reported
            != (
                witness.cycles,
                witness.post_drop_gathers,
                witness.confirmed_drops,
            )
        {
            reported = (
                witness.cycles,
                witness.post_drop_gathers,
                witness.confirmed_drops,
            );
            println!(
                "{}",
                json!({
                    "phase": "progress",
                    "cell": cell_name,
                    "live_case": case.name(),
                    "cycles": witness.cycles,
                    "post_drop_gathers": witness.post_drop_gathers,
                    "confirmed_drops": witness.confirmed_drops,
                    "product_capacity": product_capacity,
                    "yielded": witness.last_status_yielded,
                    "dropped": witness.last_status_dropped,
                    "xp": witness.last_xp,
                    "methods": witness.methods,
                })
            );
        }
        let done = match case {
            LiveCase::Power => {
                witness.cycles >= REQUIRED_CYCLES
                    && witness.post_drop_gathers >= REQUIRED_POST_DROP_GATHERS
                    && (cell != Cell::Mining
                        || witness
                            .dropped_product_ids
                            .iter()
                            .any(|id| matches!(id, 1623 | 1621 | 1619 | 1617)))
            }
            LiveCase::FishNet => {
                witness.cycles >= REQUIRED_CYCLES
                    && witness.post_drop_gathers >= REQUIRED_POST_DROP_GATHERS
                    && witness.moved_spot_reacquired
                    && witness.fish_targets.len() >= 2
            }
            LiveCase::FishBaitGate | LiveCase::OakAbsentArea => witness.failure_code.is_some(),
            LiveCase::OakRespawn => {
                witness.all_oaks_depleted_at_wait && witness.first_regrown_gather_tick.is_some()
            }
            LiveCase::OakNearEdge => {
                witness.first_edge_oak_loaded_tick.is_some() && witness.edge_fresh_xp
            }
            LiveCase::LocationAuto => {
                witness.auto_next_gather_tick.is_some()
                    && witness.last_status_yielded > 0
                    && witness.last_xp > baseline_xp
            }
            LiveCase::GasHazard => witness.gas_hazard_xp_after_escape,
            LiveCase::CancelBeforeDrain => witness.stopped_before_drain,
            LiveCase::DeathDuringDrop => witness.death_restart_gathered,
            LiveCase::RunKeyChangeDuringDrop => {
                witness.run_key_changed && witness.confirmed_drops >= 28
            }
        };
        if done
            && state
                .lock()
                .map_err(|_| "live state poisoned")?
                .unsettled_items
                .is_empty()
        {
            break Ok(());
        }
        let deadline = if phase == Prep::Running {
            if case.is_power() {
                started_at + PREP_DEADLINE + POWER_DEADLINE
            } else {
                started_at + PREP_DEADLINE + WEDGE_DEADLINE
            }
        } else {
            started_at + PREP_DEADLINE
        };
        if Instant::now() >= deadline {
            if let Some(helper_state) = &helper_state {
                let helper = helper_state.lock().map_err(|_| "fixture state poisoned")?;
                eprintln!(
                    "fixture-timeout phase={:?} task={:?} tile={:?} inventory={:?} equipment={:?} chat={:?}",
                    helper.phase,
                    helper.fixture_task,
                    helper.snapshot.tile(),
                    helper.snapshot.inventory(),
                    helper.snapshot.equipment(),
                    helper.snapshot.chat_modal_texts(),
                );
            }
            break Err(format!(
                "{} timeout: phase={phase:?} witness={witness:?} latest={latest:?}",
                cell_name
            ));
        }
        std::thread::sleep(POLL_INTERVAL);
    };
    let (witness, product_capacity) = {
        let slot = state.lock().map_err(|_| "live state poisoned")?;
        (slot.witness.clone(), slot.cycle_product_capacity())
    };
    let mut receipt = json!({
            "phase": "witness",
            "cell": cell_name,
            "live_case": case.name(),
            "cycles": witness.cycles,
            "post_drop_gathers": witness.post_drop_gathers,
            "confirmed_drops": witness.confirmed_drops,
            "full_pack_seen": witness.full_pack_seen,
            "product_capacity": product_capacity,
            "stopped_before_drain": witness.stopped_before_drain,
            "death_command_sent": witness.death_command_sent,
            "death_blocked": witness.death_blocked,
            "death_restart_tile": witness.death_restart_tile,
            "death_restart_gathered": witness.death_restart_gathered,
            "run_key_changed": witness.run_key_changed,
            "modal_command_sent": witness.modal_command_sent,
            "banked_unusable_tool": witness.banked_unusable_tool,
            "yielded": witness.last_status_yielded,
            "dropped": witness.last_status_dropped,
            "xp": witness.last_xp,
            "methods": witness.methods,
            "targets": witness.targets,
            "failure": witness.failure,
            "native_phases": witness.native_phases,
            "last_area": witness.last_area,
            "last_event": witness.last_event,
    });
    let mut fixture_receipt = json!({
            "fixture_chop_observed": witness.fixture_chop_observed,
            "fixture_chop_tiles": witness.fixture_chop_tiles,
            "fixture_seeds_observed": witness.fixture_seeds_observed,
            "fish_targets": witness
                .fish_targets
                .iter()
                .map(|target| (target.type_id, target.tile))
                .collect::<Vec<_>>(),
            "fish_products_seen": witness.fish_products_seen,
            "products_seen": witness.products_seen,
            "dropped_product_ids": witness.dropped_product_ids,
            "moved_spot_reacquired": witness.moved_spot_reacquired,
            "no_fish_xp": witness.no_fish_xp,
            "all_oaks_depleted_at_wait": witness.all_oaks_depleted_at_wait,
            "oak_other_gather_tick": witness.oak_other_gather_tick,
            "first_wait_tick": witness.first_wait_tick,
            "wait_until": witness.wait_until,
            "first_wait_native_tick": witness.first_wait_native_tick,
            "first_regrown_tile": witness.first_regrown_tile,
            "first_regrown_tick": witness.first_regrown_tick,
            "first_regrown_approach_tick": witness.first_regrown_approach_tick,
            "first_regrown_gather_tick": witness.first_regrown_gather_tick,
            "edge_initial_gap": witness.edge_initial_gap,
            "edge_initial_distance": witness.edge_initial_distance,
            "first_edge_oak_approach_tick": witness.first_edge_oak_approach_tick,
            "first_edge_oak_loaded_tick": witness.first_edge_oak_loaded_tick,
            "edge_fresh_xp": witness.edge_fresh_xp,
            "auto_widen_seen": witness.auto_widen_seen,
            "auto_selected_next": witness.auto_selected_next,
            "auto_next_gather_tick": witness.auto_next_gather_tick,
            "gas_hazard_tile": witness.gas_hazard_tile,
            "gas_hazard_observed": witness.gas_hazard_observed,
            "gas_hazard_escaped": witness.gas_hazard_escaped,
            "gas_hazard_xp_after_escape": witness.gas_hazard_xp_after_escape,
            "driver_trace": "host debug enabled; inspect native-packet account/tick/count lines",
    });
    receipt.as_object_mut().expect("receipt object").append(
        fixture_receipt
            .as_object_mut()
            .expect("fixture receipt object"),
    );
    println!("{receipt}");
    play.script_stop(&account);
    play.stop_slot(&account);
    if let Some(helper_account) = &helper_account {
        play.stop_slot(helper_account);
    }
    result?;
    let qualification = state.lock().map_err(|_| "live state poisoned")?.qualifies();
    qualification
}

#[test]
#[ignore = "requires LIVE=1, GATHERER_* fixture settings and local 289 engine"]
fn gatherer_wc_power() {
    run_cell(Cell::Woodcutting, LiveCase::Power).unwrap();
}

#[test]
#[ignore = "requires LIVE=1, GATHERER_* fixture settings and local 289 engine"]
fn gatherer_mine_tier_power() {
    run_cell(Cell::Mining, LiveCase::Power).unwrap();
}

#[test]
#[ignore = "requires LIVE=1, GATHERER_WC_TILE and local 289 engine"]
fn gatherer_cancel_before_drain_live() {
    run_cell(Cell::Woodcutting, LiveCase::CancelBeforeDrain).unwrap();
}

#[test]
#[ignore = "requires LIVE=1, GATHERER_WC_TILE and local 289 engine"]
fn gatherer_death_during_drop_live() {
    run_cell(Cell::Woodcutting, LiveCase::DeathDuringDrop).unwrap();
}

#[test]
#[ignore = "requires LIVE=1, GATHERER_WC_TILE and local 289 engine"]
fn gatherer_run_key_change_during_drop_live() {
    run_cell(Cell::Woodcutting, LiveCase::RunKeyChangeDuringDrop).unwrap();
}
#[test]
#[ignore = "requires LIVE=1, GATHERER_FISH_TILE and local 289 engine"]
fn gatherer_fish_net() {
    run_cell(Cell::Fishing, LiveCase::FishNet).unwrap();
}

#[test]
#[ignore = "requires LIVE=1, GATHERER_FISH_TILE and local 289 engine"]
fn gatherer_fish_bait_gate() {
    run_cell(Cell::Fishing, LiveCase::FishBaitGate).unwrap();
}

#[test]
#[ignore = "requires LIVE=1, GATHERER_WC_RESPAWN_*_TILE and local 289 engine"]
fn gatherer_wc_respawn() {
    run_cell(Cell::Woodcutting, LiveCase::OakRespawn).unwrap();
}

#[test]
#[ignore = "requires LIVE=1, GATHERER_WC_EDGE_*_TILE and local 289 engine"]
fn gatherer_wc_near_edge() {
    run_cell(Cell::Woodcutting, LiveCase::OakNearEdge).unwrap();
}

#[test]
#[ignore = "requires LIVE=1, GATHERER_AUTO_START_TILE and local 289 engine"]
fn gatherer_location_modes() {
    run_cell(Cell::Woodcutting, LiveCase::LocationAuto).unwrap();
}

#[test]
#[ignore = "requires LIVE=1, GATHERER_WC_ABSENT_TILE and local 289 engine"]
fn gatherer_wc_absent() {
    run_cell(Cell::Woodcutting, LiveCase::OakAbsentArea).unwrap();
}

#[test]
#[ignore = "requires LIVE=1, GATHERER_GAS_TILE and local 289 engine"]
fn gatherer_mine_gas_hazard() {
    run_cell(Cell::Mining, LiveCase::GasHazard).unwrap();
}
