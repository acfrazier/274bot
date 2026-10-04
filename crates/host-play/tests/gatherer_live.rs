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
//! Mining Power seeds one uncut sapphire to prove incidental-product disposal
//! deterministically. Natural gem rolls are observations, never a pass gate.
//! G2 cells cover moving fishing spots, supply gates, oak respawn and scene
//! boundaries, Auto widening, and an id-seeded gas hazard. G3 cells cover
//! banked-tool/supply trips, cost-ranked bank selection, and deposit returns.
//! The Catherby harpoon Bank cell requires two positive deposit trips and fresh
//! fishing yields after each return, with the inventory-only tool conserved.
//! Its Auto sibling exercises the operator Fishing 76/radius 40/Catherby Start
//! bag and the panel's friendly "Raw tuna / Raw swordfish" label, without seeded
//! fish, resources, NPCs or an incidental-drop gate. Initial level/tool fixtures
//! precede the progression baseline; proof gains come from the running script.
//! G4a cells exercise guardian-owned random-event holds and verified death
//! recovery, including terminal return refusals and watchdog recreation;
//! they require the
//! `BOT_LIVE_NAME_PREFIX=g4a` namespace.
//! Fixture helpers alter locations only; they never inject products or XP.
//! Every live account uses the configured `BOT_LIVE_NAME_PREFIX` and a
//! suffix. Each cell uses an isolated HOME plus absolute paths for
//! `GATHERER_ENGINE_DIR`, `GATHERER_NAV_PACK`, and `GATHERER_CATALOG_ROOT`.
//! G3 bank trips default to proven oak/fishing origins; the first-goal bank
//! cell uses Draynor's live oak grove east of its bank. It does not seed scene
//! locations; only the account's bronze axe is bank-seeded for its real
//! gathering and deposit trip.
//! Bank receipts use the prepared native deposit policy for every skill:
//! products and incidental gifts must reach the bank, leaving only next-run
//! tools/supplies. Initial tool-fetch visits do not count as deposit receipts.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use api::interact::{self, ActionSpec, Interactions, OpTarget, SendResult};
use api::snapshot::{ActorKind, GameSnapshot, WorldTile};
use host::Pump;
use host_play::{ProfileOptions, ScriptStartHandle, SharedClientTemplate};
#[cfg(feature = "live-harness")]
use scenario::{RunnerStatus, ScenarioRunner};
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
const SEEDED_MINING_GEM_ID: i32 = 1623;
const SHRIMP_ID: i32 = 317;
const ANCHOVY_ID: i32 = 321;
const TROUT_ID: i32 = 335;
const SALMON_ID: i32 = 331;
const TUNA_ID: i32 = 359;
const SWORDFISH_ID: i32 = 371;
const FEATHERS_ID: i32 = 314;
const CASKET_ID: i32 = 405;
const OAK_ID: i32 = 1281;
const MAPLE_LOG_ID: i32 = 1517;
const WILLOW_LOG_ID: i32 = 1519;
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
const DEATH_FILLERS: &[(i32, &str)] = &[
    (1073, "adamant_platelegs"),
    (1123, "adamant_platebody"),
    (1161, "adamant_full_helm"),
];
const DEATH_REGION_START: WorldTile = WorldTile {
    x: 3219,
    z: 3208,
    level: 0,
};
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
const CATHERBY_HARPOON_START: WorldTile = WorldTile {
    x: 2840,
    z: 3436,
    level: 0,
};

const OAK_RESPAWN_START: WorldTile = WorldTile {
    x: 3017,
    z: 3170,
    level: 0,
};

const DRAYNOR_OAK_BANK_START: WorldTile = WorldTile {
    x: 3095,
    z: 3243,
    level: 0,
};
const SEERS_MAPLE_BANK_START: WorldTile = WorldTile {
    x: 2727,
    z: 3501,
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
            (Self::Fishing, LiveCase::Site) => "gatherer_fish_harpoon_bank_site",
            (Self::Woodcutting, LiveCase::Site) => "gatherer_wc_willow_site",
            (Self::Mining, LiveCase::Site) => "gatherer_mine_copper_tin_site",
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
            (Self::Woodcutting, LiveCase::WoodcuttingBank) => "gatherer_wc_bank",
            (Self::Woodcutting, LiveCase::WoodcuttingBankUnwieldable) => {
                "gatherer_wc_bank_unwieldable"
            }
            (Self::Mining, LiveCase::BankCost) => "gatherer_mining_bank_cost_walk_ranked",
            (Self::Woodcutting, LiveCase::BankCostFirstGoal) => "gatherer_bank_cost_t1",
            (Self::Woodcutting, LiveCase::BankCostSeersMaple) => "gatherer_bank_cost_seers_maple",
            (Self::Woodcutting, LiveCase::BankCostNoCandidate) => "gatherer_bank_cost_no_candidate",
            (Self::Fishing, LiveCase::FishBait) => "gatherer_fish_bait",
            (Self::Fishing, LiveCase::FishHarpoonBank) => "gatherer_fish_harpoon_bank",
            (Self::Fishing, LiveCase::FishHarpoonBankAuto) => "gatherer_fish_harpoon_bank_auto",
            (Self::Woodcutting, LiveCase::CoinRunes) => "gatherer_coin_runes",
            (Self::Woodcutting, LiveCase::CoinRunesEmpty) => "gatherer_coin_runes_empty_stock",
            (Self::Woodcutting, LiveCase::PowerToBank) => "gatherer_power_to_bank_live",
            (Self::Woodcutting, LiveCase::PauseResumeOtherPlane) => {
                "gatherer_pause_resume_other_plane_live"
            }
            (Self::Woodcutting, LiveCase::ReconnectReturn) => "gatherer_reconnect_return_live",
            (Self::Woodcutting, LiveCase::RandomEvent) => "gatherer_random",
            (Self::Woodcutting, LiveCase::MazeRandom) => "gatherer_maze_random",
            (Self::Woodcutting, LiveCase::DeathReturn) => "gatherer_death_return",
            (Self::Woodcutting, LiveCase::DeathRespawnRegion) => {
                "gatherer_death_return_respawn_region"
            }
            (Self::Woodcutting, LiveCase::DeathNoStock) => "gatherer_death_return_no_stock",
            (Self::Woodcutting, LiveCase::DeathReturnRefused) => {
                "gatherer_death_return_refused_stops"
            }
            (Self::Woodcutting, LiveCase::DeathWatchdogPending) => {
                "gatherer_death_watchdog_pending5"
            }
            (Self::Woodcutting, LiveCase::DeathWatchdogProving) => {
                "gatherer_death_watchdog_proving"
            }
            _ => "gatherer_invalid_fixture",
        }
    }

    const fn tile_env(self, case: LiveCase) -> &'static str {
        match case {
            LiveCase::FishNet | LiveCase::FishBaitGate => "GATHERER_FISH_TILE",
            LiveCase::OakRespawn => "GATHERER_WC_RESPAWN_START_TILE",
            LiveCase::OakNearEdge => "GATHERER_WC_EDGE_START_TILE",
            LiveCase::LocationAuto => "GATHERER_AUTO_START_TILE",
            LiveCase::WoodcuttingBank => "GATHERER_WC_BANK_TILE",
            LiveCase::WoodcuttingBankUnwieldable => "GATHERER_WC_BANK_TILE",
            LiveCase::BankCostNoCandidate => "GATHERER_BANK_NO_CANDIDATE_TILE",
            LiveCase::FishBait => "GATHERER_FISH_BAIT_TILE",
            LiveCase::FishHarpoonBank => "GATHERER_FISH_HARPOON_TILE",
            LiveCase::CoinRunes | LiveCase::CoinRunesEmpty => "GATHERER_COIN_RUNES_TILE",
            LiveCase::PowerToBank => "GATHERER_POWER_TO_BANK_TILE",
            LiveCase::PauseResumeOtherPlane => "GATHERER_RETURN_PAUSE_TILE",
            LiveCase::ReconnectReturn => "GATHERER_RECONNECT_RETURN_TILE",
            LiveCase::GasHazard => "GATHERER_GAS_TILE",
            LiveCase::OakAbsentArea => "GATHERER_WC_ABSENT_TILE",
            LiveCase::RandomEvent | LiveCase::MazeRandom => "GATHERER_WC_TILE",
            LiveCase::DeathReturn
            | LiveCase::DeathNoStock
            | LiveCase::DeathWatchdogPending
            | LiveCase::DeathWatchdogProving => "GATHERER_WC_BANK_TILE",
            LiveCase::DeathRespawnRegion => "GATHERER_DEATH_REGION_TILE",
            LiveCase::DeathReturnRefused => "GATHERER_DEATH_RETURN_REFUSE_TILE",
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
            (Self::Fishing, LiveCase::FishBaitGate | LiveCase::FishBait) => "fly_fishing_rod",
            (Self::Fishing, LiveCase::FishHarpoonBank | LiveCase::FishHarpoonBankAuto) => "harpoon",
            (Self::Fishing, LiveCase::Site) => "harpoon",
            (Self::Mining, LiveCase::BankCost | LiveCase::Site) => "bronze_pickaxe",
            (Self::Mining, _) => "steel_pickaxe",
            (Self::Woodcutting, LiveCase::WoodcuttingBankUnwieldable) => "rune_axe",
            _ => "bronze_axe",
        }
    }

    const fn default_tool_id(self, case: LiveCase) -> i32 {
        match (self, case) {
            (Self::Fishing, LiveCase::FishNet) => 303,
            (Self::Fishing, LiveCase::FishBaitGate | LiveCase::FishBait) => 309,
            (Self::Fishing, LiveCase::FishHarpoonBank | LiveCase::FishHarpoonBankAuto) => 311,
            (Self::Fishing, LiveCase::Site) => 311,
            (Self::Mining, LiveCase::BankCost | LiveCase::Site) => 1265,
            (Self::Mining, _) => 1269,
            (Self::Woodcutting, LiveCase::WoodcuttingBankUnwieldable) => 1359,
            _ => 1351,
        }
    }

    const fn default_level(self, case: LiveCase) -> i32 {
        match (self, case) {
            (Self::Woodcutting, LiveCase::Site) => 30,
            (Self::Mining, LiveCase::Site) => 1,
            (Self::Fishing, LiveCase::Site) => 76,
            (
                Self::Woodcutting,
                LiveCase::DeathReturn
                | LiveCase::DeathRespawnRegion
                | LiveCase::DeathNoStock
                | LiveCase::DeathReturnRefused
                | LiveCase::DeathWatchdogPending
                | LiveCase::DeathWatchdogProving,
            ) => 15,
            (
                Self::Woodcutting,
                LiveCase::OakRespawn
                | LiveCase::OakNearEdge
                | LiveCase::OakAbsentArea
                | LiveCase::WoodcuttingBank
                | LiveCase::BankCostFirstGoal
                | LiveCase::BankCostNoCandidate
                | LiveCase::WoodcuttingBankUnwieldable
                | LiveCase::PauseResumeOtherPlane
                | LiveCase::ReconnectReturn,
            ) => 15,
            (Self::Fishing, LiveCase::FishBaitGate | LiveCase::FishBait) => 20,
            (Self::Woodcutting, LiveCase::BankCostSeersMaple) => 45,
            (Self::Fishing, LiveCase::FishHarpoonBank) => 99,
            (Self::Fishing, LiveCase::FishHarpoonBankAuto) => 76,
            (Self::Mining, LiveCase::BankCost) => 1,
            (Self::Mining, _) => 30,
            _ => 1,
        }
    }

    const fn products(self, case: LiveCase) -> &'static [i32] {
        match (self, case) {
            (Self::Woodcutting, LiveCase::Site) => &[WILLOW_LOG_ID],
            (Self::Mining, LiveCase::Site) => &[COPPER_ID, TIN_ID],
            (Self::Fishing, LiveCase::Site) => &[TUNA_ID, SWORDFISH_ID],
            (Self::Woodcutting, LiveCase::BankCostSeersMaple) => &[MAPLE_LOG_ID],
            (
                Self::Woodcutting,
                LiveCase::DeathReturn
                | LiveCase::DeathRespawnRegion
                | LiveCase::DeathNoStock
                | LiveCase::DeathReturnRefused
                | LiveCase::DeathWatchdogPending
                | LiveCase::DeathWatchdogProving,
            ) => &[1521],
            (
                Self::Woodcutting,
                LiveCase::OakRespawn
                | LiveCase::OakNearEdge
                | LiveCase::OakAbsentArea
                | LiveCase::WoodcuttingBank
                | LiveCase::BankCostFirstGoal
                | LiveCase::BankCostNoCandidate
                | LiveCase::WoodcuttingBankUnwieldable
                | LiveCase::PauseResumeOtherPlane
                | LiveCase::ReconnectReturn,
            ) => &[1521],
            (Self::Fishing, LiveCase::FishNet) => &[SHRIMP_ID, ANCHOVY_ID],
            (Self::Fishing, LiveCase::FishBaitGate | LiveCase::FishBait) => &[TROUT_ID, SALMON_ID],
            (Self::Fishing, LiveCase::FishHarpoonBank | LiveCase::FishHarpoonBankAuto) => {
                &[TUNA_ID, SWORDFISH_ID]
            }
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
            Self::Woodcutting if case == LiveCase::Site => vec!["willow".into()],
            Self::Mining if case == LiveCase::Site => vec!["copper".into(), "tin".into()],
            Self::Woodcutting if case == LiveCase::BankCostSeersMaple => vec!["maple".into()],
            Self::Woodcutting => vec![if matches!(
                case,
                LiveCase::OakRespawn
                    | LiveCase::OakNearEdge
                    | LiveCase::OakAbsentArea
                    | LiveCase::WoodcuttingBank
                    | LiveCase::WoodcuttingBankUnwieldable
                    | LiveCase::BankCostFirstGoal
                    | LiveCase::BankCostNoCandidate
                    | LiveCase::PauseResumeOtherPlane
                    | LiveCase::ReconnectReturn
                    | LiveCase::DeathReturn
                    | LiveCase::DeathRespawnRegion
                    | LiveCase::DeathNoStock
                    | LiveCase::DeathReturnRefused
                    | LiveCase::DeathWatchdogPending
                    | LiveCase::DeathWatchdogProving
            ) {
                "oak"
            } else {
                "normal"
            }
            .into()],
            Self::Mining if case == LiveCase::BankCost => {
                vec!["copper".into(), "tin".into()]
            }
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
            (_, LiveCase::Site) => None,
            (Self::Woodcutting, LiveCase::BankCostSeersMaple) => None,
            (Self::Woodcutting, LiveCase::OakRespawn | LiveCase::OakNearEdge) => {
                Some("GATHERER_OAK_LEVEL")
            }
            (
                Self::Woodcutting,
                LiveCase::DeathReturn
                | LiveCase::DeathRespawnRegion
                | LiveCase::DeathNoStock
                | LiveCase::DeathReturnRefused
                | LiveCase::DeathWatchdogPending
                | LiveCase::DeathWatchdogProving,
            ) => Some("GATHERER_WC_BANK_LEVEL"),
            (
                Self::Woodcutting,
                LiveCase::WoodcuttingBank | LiveCase::WoodcuttingBankUnwieldable,
            ) => Some("GATHERER_WC_BANK_LEVEL"),
            (Self::Mining, LiveCase::BankCost) => Some("GATHERER_BANK_COST_LEVEL"),
            (Self::Woodcutting, LiveCase::BankCostFirstGoal) => Some("GATHERER_BANK_T1_LEVEL"),
            (Self::Woodcutting, LiveCase::BankCostNoCandidate) => {
                Some("GATHERER_BANK_NO_CANDIDATE_LEVEL")
            }
            (Self::Woodcutting, LiveCase::CoinRunes | LiveCase::CoinRunesEmpty) => None,
            (Self::Woodcutting, LiveCase::PauseResumeOtherPlane) => {
                Some("GATHERER_RETURN_PAUSE_LEVEL")
            }
            (Self::Woodcutting, LiveCase::ReconnectReturn) => {
                Some("GATHERER_RECONNECT_RETURN_LEVEL")
            }
            (Self::Woodcutting, _) => Some("GATHERER_WC_LEVEL"),
            (Self::Mining, _) => Some("GATHERER_MINE_LEVEL"),
            (Self::Fishing, LiveCase::FishHarpoonBankAuto) => None,
            (Self::Fishing, _) => Some("GATHERER_FISH_LEVEL"),
        };
        env.and_then(|name| std::env::var(name).ok())
            .and_then(|value| value.parse().ok())
            .unwrap_or_else(|| self.default_level(case))
    }

    fn tool_alias(self, case: LiveCase) -> String {
        match (self, case) {
            (Self::Woodcutting, LiveCase::WoodcuttingBankUnwieldable) => {
                std::env::var("GATHERER_WC_BANK_BETTER_TOOL")
                    .unwrap_or_else(|_| self.default_tool_alias(case).into())
            }
            (
                Self::Woodcutting,
                LiveCase::DeathReturn
                | LiveCase::DeathRespawnRegion
                | LiveCase::DeathNoStock
                | LiveCase::DeathReturnRefused
                | LiveCase::DeathWatchdogPending
                | LiveCase::DeathWatchdogProving,
            ) => std::env::var("GATHERER_WC_BANK_TOOL")
                .unwrap_or_else(|_| self.default_tool_alias(case).into()),
            (
                Self::Woodcutting,
                LiveCase::WoodcuttingBank
                | LiveCase::BankCostFirstGoal
                | LiveCase::BankCostNoCandidate
                | LiveCase::PauseResumeOtherPlane
                | LiveCase::ReconnectReturn,
            ) => std::env::var("GATHERER_WC_BANK_TOOL")
                .unwrap_or_else(|_| self.default_tool_alias(case).into()),
            (Self::Woodcutting, LiveCase::BankCostSeersMaple) => {
                self.default_tool_alias(case).into()
            }
            (Self::Woodcutting, LiveCase::CoinRunes | LiveCase::CoinRunesEmpty) => {
                std::env::var("GATHERER_COIN_TOOL")
                    .unwrap_or_else(|_| self.default_tool_alias(case).into())
            }
            (Self::Woodcutting, _) => std::env::var("GATHERER_WC_TOOL")
                .unwrap_or_else(|_| self.default_tool_alias(case).into()),
            (Self::Mining, LiveCase::BankCost) => self.default_tool_alias(case).into(),
            (Self::Mining, _) => std::env::var("GATHERER_MINE_TOOL")
                .unwrap_or_else(|_| self.default_tool_alias(case).into()),
            _ => self.default_tool_alias(case).into(),
        }
    }

    fn tool_id(self, case: LiveCase) -> i32 {
        let env = match (self, case) {
            (Self::Woodcutting, LiveCase::WoodcuttingBankUnwieldable) => {
                Some("GATHERER_WC_BANK_BETTER_TOOL_ID")
            }
            (
                Self::Woodcutting,
                LiveCase::DeathReturn
                | LiveCase::DeathRespawnRegion
                | LiveCase::DeathNoStock
                | LiveCase::DeathReturnRefused
                | LiveCase::DeathWatchdogPending
                | LiveCase::DeathWatchdogProving,
            ) => Some("GATHERER_WC_BANK_TOOL_ID"),
            (
                Self::Woodcutting,
                LiveCase::WoodcuttingBank
                | LiveCase::BankCostFirstGoal
                | LiveCase::BankCostNoCandidate
                | LiveCase::PauseResumeOtherPlane
                | LiveCase::ReconnectReturn,
            ) => Some("GATHERER_WC_BANK_TOOL_ID"),
            (Self::Woodcutting, LiveCase::BankCostSeersMaple) => None,
            (Self::Woodcutting, LiveCase::CoinRunes | LiveCase::CoinRunesEmpty) => {
                Some("GATHERER_COIN_TOOL_ID")
            }
            (Self::Woodcutting, _) => Some("GATHERER_WC_TOOL_ID"),
            (Self::Mining, LiveCase::BankCost) => None,
            (Self::Mining, _) => Some("GATHERER_MINE_TOOL_ID"),
            (Self::Fishing, _) => None,
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
                json!(match case {
                    LiveCase::FishBaitGate | LiveCase::FishBait => "fishing.freshfish.op1",
                    LiveCase::FishHarpoonBank => "fishing.rarefish.op3",
                    LiveCase::FishHarpoonBankAuto => "fishing.rarefish.op3",
                    LiveCase::Site => "fishing.harpoon.tool_311.products_359_371",
                    _ => "fishing.saltfish.op1",
                }),
            );
        }
        bag.insert("targetPreference".into(), json!(self.target_preference()));
        bag.insert(
            "location".into(),
            json!(
                if matches!(case, LiveCase::LocationAuto | LiveCase::FishHarpoonBankAuto) {
                    "Auto"
                } else {
                    "Start"
                }
            ),
        );
        bag.insert(
            "radius".into(),
            json!(match case {
                LiveCase::OakRespawn | LiveCase::LocationAuto | LiveCase::BankCost => 32,
                LiveCase::FishHarpoonBankAuto => 40,
                LiveCase::OakNearEdge => 64,
                LiveCase::OakAbsentArea => 2,
                LiveCase::GasHazard => 12,
                _ => 12,
            }),
        );
        // Older G1/G2 fixtures remain explicit Power cases.
        bag.insert(
            "disposition".into(),
            json!(if matches!(
                case,
                LiveCase::WoodcuttingBank
                    | LiveCase::WoodcuttingBankUnwieldable
                    | LiveCase::BankCost
                    | LiveCase::BankCostFirstGoal
                    | LiveCase::BankCostSeersMaple
                    | LiveCase::BankCostNoCandidate
                    | LiveCase::FishBait
                    | LiveCase::FishHarpoonBank
                    | LiveCase::FishHarpoonBankAuto
                    | LiveCase::CoinRunes
                    | LiveCase::CoinRunesEmpty
                    | LiveCase::PauseResumeOtherPlane
                    | LiveCase::ReconnectReturn
            ) {
                "Bank"
            } else {
                "Power"
            }),
        );
        if matches!(
            case,
            LiveCase::WoodcuttingBank
                | LiveCase::WoodcuttingBankUnwieldable
                | LiveCase::BankCost
                | LiveCase::BankCostFirstGoal
                | LiveCase::BankCostSeersMaple
                | LiveCase::BankCostNoCandidate
                | LiveCase::FishBait
                | LiveCase::FishHarpoonBank
                | LiveCase::FishHarpoonBankAuto
                | LiveCase::CoinRunes
                | LiveCase::CoinRunesEmpty
                | LiveCase::PowerToBank
                | LiveCase::PauseResumeOtherPlane
                | LiveCase::ReconnectReturn
                | LiveCase::DeathReturn
                | LiveCase::DeathRespawnRegion
                | LiveCase::DeathNoStock
                | LiveCase::DeathReturnRefused
                | LiveCase::DeathWatchdogPending
                | LiveCase::DeathWatchdogProving
        ) {
            bag.insert(
                "bank".into(),
                json!(match case {
                    LiveCase::BankCostNoCandidate => "Zanaris",
                    LiveCase::BankCostSeersMaple => "Seers",
                    LiveCase::FishHarpoonBankAuto => "Catherby",
                    _ => "Nearest",
                }),
            );
            bag.insert(
                "useMageBank".into(),
                json!(case == LiveCase::BankCost && BANK_COST_FIXTURE_INTENT.use_mage_bank),
            );
            bag.insert(
                "useZanarisBank".into(),
                json!(case == LiveCase::BankCost && BANK_COST_FIXTURE_INTENT.use_zanaris_bank),
            );
        }
        bag.insert("allowTeleports".into(), json!(false));
        bag.insert(
            "allowWilderness".into(),
            json!(case == LiveCase::BankCost && BANK_COST_FIXTURE_INTENT.allow_wilderness),
        );
        bag.insert(
            "deathPolicy".into(),
            json!(if case.is_death_recovery() {
                "Recover"
            } else {
                "Stop"
            }),
        );
        bag.insert("maxDeaths".into(), json!(2));
        if case == LiveCase::FishBait {
            bag.insert("baitTarget".into(), json!(3));
        }
        if matches!(case, LiveCase::CoinRunes | LiveCase::CoinRunesEmpty) {
            bag.insert("coinTarget".into(), json!(100));
            bag.insert(
                "reserveTeleport".into(),
                json!(std::env::var("GATHERER_COIN_RESERVE_TELEPORT")
                    .unwrap_or_else(|_| "Trollheim".into())),
            );
            bag.insert("reserveCasts".into(), json!(10));
        }
        if case == LiveCase::Site {
            bag.insert("location".into(), json!("Site"));
            bag.insert("site".into(), json!(self.site_id()));
            bag.insert("disposition".into(), json!("Bank"));
            bag.insert("bank".into(), json!("Nearest"));
        }
        bag
    }

    const fn site_id(self) -> &'static str {
        match self {
            Self::Woodcutting => "woodcutting.draynor",
            Self::Mining => "mining.varrock_east.se",
            Self::Fishing => "fishing.catherby",
        }
    }

    const fn site_bank(self) -> &'static str {
        match self {
            Self::Woodcutting => "Draynor",
            Self::Mining => "Varrock East",
            Self::Fishing => "Catherby",
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct BankCostFixtureIntent {
    use_mage_bank: bool,
    use_zanaris_bank: bool,
    allow_wilderness: bool,
}

const BANK_COST_FIXTURE_INTENT: BankCostFixtureIntent = BankCostFixtureIntent {
    use_mage_bank: true,
    use_zanaris_bank: false,
    allow_wilderness: true,
};

impl BankCostFixtureIntent {
    fn bank_preferences(self) -> api::named_banks::BankPreferences {
        api::named_banks::BankPreferences {
            use_mage_bank: self.use_mage_bank,
            use_zanaris_bank: self.use_zanaris_bank,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LiveCase {
    Power,
    Site,
    CancelBeforeDrain,
    DeathDuringDrop,
    RunKeyChangeDuringDrop,
    RandomEvent,
    DeathReturn,
    DeathRespawnRegion,
    DeathNoStock,
    DeathReturnRefused,
    // Constructed only by the `live-probe` watchdog cells.
    #[cfg_attr(not(feature = "live-probe"), allow(dead_code))]
    DeathWatchdogPending,
    #[cfg_attr(not(feature = "live-probe"), allow(dead_code))]
    DeathWatchdogProving,
    FishNet,
    FishBaitGate,
    OakRespawn,
    OakNearEdge,
    LocationAuto,
    OakAbsentArea,
    GasHazard,
    WoodcuttingBank,
    WoodcuttingBankUnwieldable,
    BankCost,
    BankCostFirstGoal,
    BankCostSeersMaple,
    BankCostNoCandidate,
    FishBait,
    FishHarpoonBank,
    FishHarpoonBankAuto,
    CoinRunes,
    CoinRunesEmpty,
    PowerToBank,
    PauseResumeOtherPlane,
    ReconnectReturn,
    MazeRandom,
}

impl LiveCase {
    const fn name(self) -> &'static str {
        match self {
            Self::Power => "power",
            Self::Site => "named-site-bank-return",
            Self::CancelBeforeDrain => "cancel-before-drain",
            Self::DeathDuringDrop => "death-during-drop",
            Self::RunKeyChangeDuringDrop => "run-key-change-during-drop",
            Self::FishNet => "fish-net",
            Self::RandomEvent => "random-event",
            Self::DeathReturn => "death-return",
            Self::DeathRespawnRegion => "death-return-respawn-region",
            Self::DeathNoStock => "death-return-no-stock",
            Self::DeathReturnRefused => "death-return-refused-stop",
            Self::DeathWatchdogPending => "death-watchdog-pending5",
            Self::DeathWatchdogProving => "death-watchdog-proving",
            Self::FishBaitGate => "fish-bait-gate",
            Self::OakRespawn => "oak-respawn",
            Self::OakNearEdge => "oak-near-edge",
            Self::LocationAuto => "location-auto",
            Self::OakAbsentArea => "oak-absent-area",
            Self::GasHazard => "gas-hazard",
            Self::WoodcuttingBank => "woodcutting-bank",
            Self::WoodcuttingBankUnwieldable => "woodcutting-bank-unwieldable",
            Self::BankCost => "mining-bank-cost-walk-ranked",
            Self::BankCostFirstGoal => "bank-cost-first-goal-reachable",
            Self::BankCostSeersMaple => "seers-maple-bank-lifecycle",
            Self::BankCostNoCandidate => "bank-cost-no-candidate",
            Self::FishBait => "fish-bait-bank",
            Self::FishHarpoonBank => "fish-harpoon-bank",
            Self::FishHarpoonBankAuto => "fish-harpoon-bank-auto",
            Self::CoinRunes => "coin-runes-topup",
            Self::CoinRunesEmpty => "coin-runes-empty-stock",
            Self::PowerToBank => "power-to-bank",
            Self::PauseResumeOtherPlane => "pause-resume-other-plane",
            Self::ReconnectReturn => "reconnect-return",
            Self::MazeRandom => "maze-random",
        }
    }

    const fn is_fish_harpoon_bank(self) -> bool {
        matches!(self, Self::FishHarpoonBank | Self::FishHarpoonBankAuto)
    }

    const fn is_bank_mode(self) -> bool {
        matches!(
            self,
            Self::WoodcuttingBank
                | Self::Site
                | Self::WoodcuttingBankUnwieldable
                | Self::BankCost
                | Self::BankCostFirstGoal
                | Self::BankCostSeersMaple
                | Self::BankCostNoCandidate
                | Self::FishBait
                | Self::FishHarpoonBank
                | Self::FishHarpoonBankAuto
                | Self::CoinRunes
                | Self::CoinRunesEmpty
                | Self::PauseResumeOtherPlane
                | Self::ReconnectReturn
        )
    }

    const fn requires_seeded_casket(self) -> bool {
        matches!(self, Self::FishHarpoonBank)
    }
    const fn is_death_recovery(self) -> bool {
        matches!(
            self,
            Self::DeathReturn
                | Self::DeathRespawnRegion
                | Self::DeathNoStock
                | Self::DeathReturnRefused
                | Self::DeathWatchdogPending
                | Self::DeathWatchdogProving
        )
    }

    #[cfg(feature = "live-probe")]
    const fn is_death_watchdog(self) -> bool {
        matches!(
            self,
            Self::DeathWatchdogPending | Self::DeathWatchdogProving
        )
    }

    const fn is_power(self) -> bool {
        matches!(
            self,
            Self::Power
                | Self::FishNet
                | Self::WoodcuttingBank
                | Self::WoodcuttingBankUnwieldable
                | Self::PowerToBank
                | Self::DeathReturn
        )
    }

    const fn bank_tool_only(self) -> bool {
        matches!(
            self,
            Self::WoodcuttingBank
                | Self::WoodcuttingBankUnwieldable
                | Self::BankCost
                | Self::BankCostFirstGoal
                | Self::BankCostSeersMaple
                | Self::BankCostNoCandidate
                | Self::PauseResumeOtherPlane
                | Self::ReconnectReturn
        )
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

#[derive(Debug, Clone, Copy)]
struct UnsettledItem {
    since: Instant,
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
    bank_seed: Vec<(String, i32)>,
    inventory_seed: Vec<(String, i32)>,
    reserve_runes: Vec<(i32, i32)>,
    inject_after_progress: Option<LocSeed>,
    bank_cost_air_banks: Vec<(String, WorldTile)>,
    bank_cost_air_nearest: Option<String>,
    bank_cost_origin: Option<WorldTile>,
    bank_cost_gated_candidates: Vec<String>,
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

fn aggregate_item_counts(items: impl IntoIterator<Item = (i32, i32)>) -> BTreeMap<i32, i32> {
    let mut counts = BTreeMap::new();
    for (id, count) in items {
        if id >= 0 && count > 0 {
            *counts.entry(id).or_default() += count;
        }
    }
    counts
}

fn bank_deposit_expectation(
    inventory: &BTreeMap<i32, i32>,
    deposit_ids: &[i32],
) -> BTreeMap<i32, i32> {
    inventory
        .iter()
        .filter(|(id, count)| **count > 0 && deposit_ids.contains(id))
        .map(|(&id, &count)| (id, count))
        .collect()
}

fn bank_conservation_satisfied(
    expected_items: &BTreeMap<i32, i32>,
    expected_products: &BTreeMap<i32, i32>,
    inventory_after: &BTreeMap<i32, i32>,
    bank_before: &BTreeMap<i32, i32>,
    bank_after: &BTreeMap<i32, i32>,
    deposited_count: i64,
    unneeded_after: &BTreeMap<i32, i32>,
) -> bool {
    let expected_deposit_count = expected_items
        .values()
        .map(|count| i64::from(*count))
        .sum::<i64>();
    deposited_count == expected_deposit_count
        && expected_items.iter().all(|(&id, &expected)| {
            inventory_after.get(&id).copied().unwrap_or(0) == 0
                && bank_after
                    .get(&id)
                    .copied()
                    .unwrap_or(0)
                    .saturating_sub(bank_before.get(&id).copied().unwrap_or(0))
                    >= expected
        })
        && expected_products.iter().all(|(&id, &expected)| {
            let in_inventory = inventory_after.get(&id).copied().unwrap_or(0);
            let in_bank = bank_after
                .get(&id)
                .copied()
                .unwrap_or(0)
                .saturating_sub(bank_before.get(&id).copied().unwrap_or(0));
            in_inventory.saturating_add(in_bank) >= expected
        })
        && unneeded_after.is_empty()
}

#[derive(Debug, Clone, Default)]
struct BankTripReceipt {
    trip: i64,
    inventory_before: BTreeMap<i32, i32>,
    protected_ids: BTreeSet<i32>,
    expected_items: BTreeMap<i32, i32>,
    expected_products: BTreeMap<i32, i32>,
    expected_incidentals: BTreeMap<i32, i32>,
    bank_before: BTreeMap<i32, i32>,
    bank_after: BTreeMap<i32, i32>,
    deposited_before: i64,
    deposited_after: i64,
    deposited_count: i64,
    seeded_casket_expected: i32,
    deposit_confirmed: bool,
    expected_items_after: BTreeMap<i32, i32>,
    unneeded_items_after: BTreeMap<i32, i32>,
    expected_items_empty: bool,
    inventory_only_needed: bool,
    products_conserved: bool,
    bank_loaded: bool,
    bank_increased: bool,
    deposit_count_matches_expected: bool,
    deposit_verified: bool,
    seeded_casket_banked: bool,
    positive_return: bool,
    returned: bool,
    post_bank_yield: bool,
}

impl BankTripReceipt {
    fn new(
        trip: i64,
        inventory_before: BTreeMap<i32, i32>,
        deposit_ids: &[i32],
        bank_before: BTreeMap<i32, i32>,
        deposited_before: i64,
        product_ids: &[i32],
        seeded_casket_expected: i32,
    ) -> Self {
        let expected_items = bank_deposit_expectation(&inventory_before, deposit_ids);
        let protected_ids = inventory_before
            .keys()
            .filter(|id| !deposit_ids.contains(id))
            .copied()
            .collect();
        let expected_products: BTreeMap<_, _> = inventory_before
            .iter()
            .filter(|(id, count)| **count > 0 && product_ids.contains(*id))
            .map(|(&id, &count)| (id, count))
            .collect();
        let expected_incidentals: BTreeMap<_, _> = expected_items
            .iter()
            .filter(|(id, _)| !product_ids.contains(*id))
            .map(|(&id, &count)| (id, count))
            .collect();
        Self {
            trip,
            inventory_before,
            protected_ids,
            expected_items,
            expected_products,
            expected_incidentals,
            bank_before,
            deposited_before,
            seeded_casket_expected,
            ..Self::default()
        }
    }

    fn expected_item_count(&self) -> i64 {
        self.expected_items
            .values()
            .map(|count| i64::from(*count))
            .sum()
    }

    fn is_deposit_trip(&self) -> bool {
        self.deposit_verified && self.deposited_count > 0
    }

    fn expected_product_count(&self) -> i64 {
        self.expected_products
            .values()
            .map(|count| i64::from(*count))
            .sum()
    }

    fn bank_increased_for(&self, id: i32, bank_after: &BTreeMap<i32, i32>) -> bool {
        self.expected_items.get(&id).is_some_and(|expected| {
            let before = self.bank_before.get(&id).copied().unwrap_or(0);
            let after = bank_after.get(&id).copied().unwrap_or(0);
            after.saturating_sub(before) >= *expected
        })
    }

    fn observe_deposit(
        &mut self,
        inventory_after: &BTreeMap<i32, i32>,
        unneeded_after: &BTreeMap<i32, i32>,
        bank_loaded: bool,
        bank_after: &BTreeMap<i32, i32>,
        deposited_after: i64,
        deposit_confirmed: bool,
    ) -> Result<(), String> {
        self.deposit_confirmed |= deposit_confirmed;
        if !self.deposit_confirmed {
            return Ok(());
        }

        self.expected_items_after = self
            .expected_items
            .keys()
            .filter_map(|id| {
                let count = inventory_after.get(id).copied().unwrap_or(0);
                (count > 0).then_some((*id, count))
            })
            .collect();
        self.unneeded_items_after = unneeded_after.clone();
        self.expected_items_empty = self.expected_items_after.is_empty();
        self.inventory_only_needed = self.unneeded_items_after.is_empty();
        self.deposited_after = deposited_after;
        self.deposited_count = deposited_after.saturating_sub(self.deposited_before);
        self.deposit_count_matches_expected = self.deposited_count == self.expected_item_count();

        if bank_loaded {
            self.bank_loaded = true;
            self.bank_after = bank_after.clone();
            self.bank_increased = self
                .expected_items
                .keys()
                .all(|id| self.bank_increased_for(*id, bank_after));
            self.products_conserved = self.expected_products.iter().all(|(&id, &expected)| {
                let inventory_count = inventory_after.get(&id).copied().unwrap_or(0);
                let bank_count = bank_after
                    .get(&id)
                    .copied()
                    .unwrap_or(0)
                    .saturating_sub(self.bank_before.get(&id).copied().unwrap_or(0));
                inventory_count.saturating_add(bank_count) >= expected
            });
        }
        let casket_delta = self
            .bank_after
            .get(&CASKET_ID)
            .copied()
            .unwrap_or(0)
            .saturating_sub(self.bank_before.get(&CASKET_ID).copied().unwrap_or(0));
        self.seeded_casket_banked = self.seeded_casket_expected > 0
            && self
                .expected_items_after
                .get(&CASKET_ID)
                .copied()
                .unwrap_or(0)
                == 0
            && casket_delta >= self.seeded_casket_expected;
        self.deposit_verified = self.deposit_confirmed
            && self.bank_loaded
            && bank_conservation_satisfied(
                &self.expected_items,
                &self.expected_products,
                inventory_after,
                &self.bank_before,
                &self.bank_after,
                self.deposited_count,
                &self.unneeded_items_after,
            );

        if !self.expected_items_empty {
            if self.seeded_casket_expected > 0 && self.expected_items_after.contains_key(&CASKET_ID)
            {
                return Err(format!(
                    "seeded casket {CASKET_ID} remained in inventory after deposit: {:?}",
                    self.expected_items_after
                ));
            }
            return Err(format!(
                "deposit left expected item IDs in inventory: {:?}",
                self.expected_items_after
            ));
        }
        if !self.inventory_only_needed {
            return Err(format!(
                "bank trip left non-needed inventory IDs after deposit: {:?}",
                self.unneeded_items_after
            ));
        }
        Ok(())
    }

    fn record_return(
        &mut self,
        trip: i64,
        deposited_after: i64,
        require_positive: bool,
    ) -> Result<(), String> {
        self.trip = trip;
        self.deposited_after = deposited_after;
        self.deposited_count = deposited_after.saturating_sub(self.deposited_before);
        self.deposit_count_matches_expected = self.deposited_count == self.expected_item_count();
        self.positive_return = self.deposited_count > 0 && self.expected_product_count() > 0;
        self.returned = true;
        if !self.deposit_verified
            || !self.deposit_count_matches_expected
            || (require_positive && !self.positive_return)
            || (self.seeded_casket_expected > 0 && !self.seeded_casket_banked)
        {
            return Err(format!(
                "bank trip {trip} returned without a verified conservation receipt: {self:?}"
            ));
        }
        Ok(())
    }

    fn as_json(&self) -> Value {
        json!({
            "trip": self.trip,
            "inventory_before": &self.inventory_before,
            "needed_ids": &self.protected_ids,
            "expected_items": &self.expected_items,
            "expected_products": &self.expected_products,
            "expected_incidentals": &self.expected_incidentals,
            "bank_before": &self.bank_before,
            "bank_after": &self.bank_after,
            "deposited_before": self.deposited_before,
            "deposited_after": self.deposited_after,
            "deposited_count": self.deposited_count,
            "expected_item_count": self.expected_item_count(),
            "deposit_confirmed": self.deposit_confirmed,
            "expected_items_after": &self.expected_items_after,
            "unneeded_items_after": &self.unneeded_items_after,
            "expected_items_empty": self.expected_items_empty,
            "inventory_only_needed": self.inventory_only_needed,
            "products_conserved": self.products_conserved,
            "bank_loaded": self.bank_loaded,
            "bank_increased": self.bank_increased,
            "deposit_count_matches_expected": self.deposit_count_matches_expected,
            "deposit_verified": self.deposit_verified,
            "seeded_casket_expected": self.seeded_casket_expected,
            "seeded_casket_banked": self.seeded_casket_banked,
            "positive_return": self.positive_return,
            "returned": self.returned,
            "post_bank_yield": self.post_bank_yield,
        })
    }

    fn complete(&self) -> bool {
        self.deposit_verified
            && self.positive_return
            && self.returned
            && self.post_bank_yield
            && self.expected_product_count() > 0
    }
}

impl Witness {
    fn fish_bank_complete(&self, seeded_casket_required: bool) -> bool {
        (!seeded_casket_required
            || (self.seeded_casket_baseline_verified
                && self.seeded_casket_baseline_count == 1
                && self
                    .bank_trips
                    .first()
                    .is_some_and(|trip| trip.seeded_casket_banked)))
            && self.bank_trips.len() >= 2
            && self
                .bank_trips
                .iter()
                .take(2)
                .all(BankTripReceipt::complete)
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
    natural_gems_seen: BTreeSet<i32>,
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
    status_trips: i64,
    status_deposited: i64,
    bank_trip_due_at: Option<Instant>,
    bank_selection_elapsed: Option<Duration>,
    bank_selected: bool,
    bank_withdrawal_confirmed: bool,
    bank_arrivals: u32,
    post_bank_yields: u32,
    waiting_for_bank_yield: bool,
    waiting_for_return_yield: bool,
    tool_worn_observed: bool,
    coin_seed_count: i32,
    coin_balance_exact: bool,
    pause_observed: bool,
    resume_observed: bool,
    reconnect_observed: bool,
    reconnect_offline_observed: bool,
    last_status_event: Option<String>,
    last_status_bank: Option<String>,
    bank_status_sequence: Vec<Value>,
    failure_sequence: Vec<Value>,
    bait_exact: bool,
    reserve_runes_intact: bool,
    bank_nonzero_roundtrips: u32,
    tool_selected_observed: bool,
    bank_loaded_observed: bool,
    bank_closed_observed: bool,
    bank_return_step_seen: bool,
    site_anchor_selected: bool,
    site_initial_walk: bool,
    site_return_area_r1: bool,
    site_walks: Vec<Value>,
    bank_return_after_pause: bool,
    bank_return_after_reconnect: bool,
    other_plane_observed: bool,
    power_to_bank_edit_requested: bool,
    power_to_bank_edit_pending: bool,
    power_to_bank_edit_drop_baseline: u32,
    power_to_bank_edit_slots: u32,
    power_to_bank_applied: bool,
    last_return_deposited: i64,
    seeded_casket_baseline_count: i32,
    seeded_casket_baseline_verified: bool,
    natural_casket_first_observed: Option<(u32, i32)>,
    pending_bank_trip: Option<BankTripReceipt>,
    bank_trips: Vec<BankTripReceipt>,
    failure: Option<String>,
    random_owned_command_sent: bool,
    random_owned_event_seen: bool,
    random_owned_hold_frames: u32,
    random_hold_baseline: Option<(i64, i64, i32, i32)>,
    random_hold_unchanged: bool,
    random_owned_released: bool,
    random_release_yielded: i64,
    random_release_xp: i32,
    random_fresh_yield: bool,
    random_owned_revalidated: bool,
    random_foreign_command_sent: bool,
    random_foreign_event_seen: bool,
    random_foreign_not_held: bool,
    random_foreign_yield_baseline: i64,
    random_foreign_xp_baseline: i32,
    /// `~maze` (the content's `[debugproc,maze]`) went out after the
    /// first gathered yield.
    maze_command_sent: bool,
    /// The subject stood on the Maze square after the command.
    maze_entered: bool,
    /// The subject left the Maze square again; yield/xp at that edge.
    maze_exited: bool,
    maze_exit_yielded: i64,
    maze_exit_xp: i32,
    /// Fresh Gatherer yield and xp after leaving the Maze.
    maze_resumed: bool,
    /// The Gatherer blocked while still on the Maze square.
    maze_blocked_inside: bool,
    /// After that block, the slot took the terminal Stop: Idle, no
    /// native run, and a `Failed` lifecycle receipt.
    maze_stopped: bool,
    random_foreign_fresh_yield: bool,
    death_recovery_command_sent: u32,
    death_command_while_paused: bool,
    death_chat_observed_while_paused: bool,
    death_recovery_in_area: bool,
    death_command_tile: Option<(i32, i32, i32)>,
    death_respawn_region_unchanged: bool,
    death_progress_baselines: BTreeMap<u8, (i32, i32, i64)>,
    death_recovered: BTreeSet<u8>,
    death_products_after: BTreeSet<u8>,
    death_xp_after: BTreeSet<u8>,
    death_first_recovery_cycles: Option<u32>,
    death_first_recovery_post_drop_gathers: Option<u32>,
    death_haul_between: bool,
    death_return_refused: bool,
    death_return_stopped: bool,
    death_watchdog_aged: bool,
    death_watchdog_generation_before: Option<u64>,
    death_watchdog_recreated: bool,
    last_deaths: u8,
    last_recoveries: u8,
    last_recovery_step: u8,
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
    unsettled_items: BTreeMap<(i32, i32), UnsettledItem>,
    bank_config: Option<Arc<script::native::PreparedConfig>>,
    last_observed_dropped: i64,
    last_observed_product_drops: u32,
    witness: Witness,
    error: Option<String>,
    pause_plane_teleport_sent: bool,
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
            bank_config: None,
            last_observed_dropped: 0,
            last_observed_product_drops: 0,
            witness: Witness::default(),
            error: None,
            pause_plane_teleport_sent: false,
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
        if self.case == LiveCase::PauseResumeOtherPlane
            && observation
                .tile
                .is_some_and(|(_, _, level)| level != self.target.level)
        {
            self.witness.other_plane_observed = true;
        }
        if self.case == LiveCase::ReconnectReturn && !self.snapshot.ingame() {
            self.witness.reconnect_offline_observed = true;
        }
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
        self.record_random_event(hold, &observation);
        self.record_maze(&observation);
        if self.case == LiveCase::DeathReturn
            && self.witness.pause_observed
            && self.witness.death_command_while_paused
            && self.snapshot.chat_lines().iter().any(|line| {
                let text = line.text.to_ascii_lowercase();
                text.contains("oh dear") && text.contains("you are dead")
            })
        {
            self.witness.death_chat_observed_while_paused = true;
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
    fn record_maze(&mut self, observation: &Observation) {
        if self.case != LiveCase::MazeRandom || !self.witness.maze_command_sent {
            return;
        }
        let Some((x, z, level)) = observation.tile else {
            return;
        };
        if api::random::trapped_area(x, z, level) == Some(api::random::RandomKind::Maze) {
            if !self.witness.maze_entered {
                println!(
                    "{}",
                    json!({"phase": "maze-entered", "tile": [x, z, level], "xp": observation.xp})
                );
            }
            self.witness.maze_entered = true;
        } else if self.witness.maze_entered && !self.witness.maze_exited {
            self.witness.maze_exited = true;
            self.witness.maze_exit_yielded = self.witness.last_status_yielded;
            self.witness.maze_exit_xp = observation.xp;
            println!(
                "{}",
                json!({"phase": "maze-exited", "tile": [x, z, level], "xp": observation.xp})
            );
        }
    }

    fn record_random_event(&mut self, hold: bool, observation: &Observation) {
        if self.case != LiveCase::RandomEvent {
            return;
        }
        let local_index = self.snapshot.local_player().map(|local| local.player.index);
        let event_target = self
            .snapshot
            .npcs()
            .iter()
            .find(|npc| {
                npc.name
                    .as_deref()
                    .is_some_and(|name| name.eq_ignore_ascii_case("Mysterious Old Man"))
            })
            .and_then(|npc| npc.target)
            .filter(|target| target.kind == ActorKind::Player);
        if let Some(target) = event_target {
            if self.witness.random_owned_command_sent
                && local_index.is_some_and(|index| target.index == index)
            {
                self.witness.random_owned_event_seen = true;
                if hold {
                    let current = (
                        self.witness.last_status_yielded,
                        self.witness.last_status_dropped,
                        observation.xp,
                        observation.product_count,
                    );
                    if let Some(baseline) = self.witness.random_hold_baseline {
                        self.witness.random_hold_unchanged &= baseline == current;
                    } else {
                        self.witness.random_hold_baseline = Some(current);
                        self.witness.random_hold_unchanged = true;
                    }
                    self.witness.random_owned_hold_frames =
                        self.witness.random_owned_hold_frames.saturating_add(1);
                }
            } else if self.witness.random_foreign_command_sent
                && local_index.is_some_and(|index| target.index != index)
            {
                if !self.witness.random_foreign_event_seen {
                    self.witness.random_foreign_not_held = true;
                }
                self.witness.random_foreign_event_seen = true;
                self.witness.random_foreign_not_held &= !hold;
            }
        }
        if self.witness.random_owned_hold_frames > 0 && !hold && !self.witness.random_owned_released
        {
            self.witness.random_owned_released = true;
            self.witness.random_release_yielded = self.witness.last_status_yielded;
            self.witness.random_release_xp = observation.xp;
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
        let bank_active = self.bank_conservation_active();
        let drop_baseline = self.last_observed_dropped;
        let product_drop_baseline = self.last_observed_product_drops;
        // Inventory, equipment and bank counts arrive in separate packets.
        // Every banked item settles only after its expected bank increase;
        // protected tools still must reappear in inventory or equipment.
        let bank_counts = if !self.unsettled_items.is_empty()
            && self.witness.pending_bank_trip.is_some()
            && self.snapshot.bank_loaded()
        {
            self.snapshot_bank_counts()
        } else {
            BTreeMap::new()
        };
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
                && !(bank_active
                    && self
                        .witness
                        .pending_bank_trip
                        .as_ref()
                        .is_some_and(|receipt| {
                            receipt.expected_items.contains_key(&id)
                                && receipt.bank_increased_for(id, &bank_counts)
                        }))
        });
        self.record_g2_observation(previous.as_ref(), &observation);
        if self.case == LiveCase::FishHarpoonBank
            && self
                .witness
                .bank_trips
                .first()
                .is_some_and(|trip| trip.seeded_casket_banked)
        {
            let casket_count = observation
                .inventory
                .values()
                .filter(|item| item.id == CASKET_ID)
                .map(|item| item.count)
                .sum::<i32>();
            if casket_count > 0 {
                self.witness
                    .natural_casket_first_observed
                    .get_or_insert((observation.tick, casket_count));
            }
        }
        if let Some(previous) = previous {
            if self.cell == Cell::Mining {
                for item in observation
                    .inventory
                    .values()
                    .filter(|item| is_mining_gem(item.id))
                {
                    let count = |rows: &BTreeMap<i32, InvRow>| {
                        rows.values()
                            .filter(|row| row.id == item.id)
                            .map(|row| row.count)
                            .sum::<i32>()
                    };
                    if count(&observation.inventory) > count(&previous.inventory) {
                        self.witness.natural_gems_seen.insert(item.id);
                    }
                }
            }
            for (slot, old) in &previous.inventory {
                if !observation.inventory.contains_key(slot) {
                    if self.cell.products(self.case).contains(&old.id) {
                        if bank_active {
                            self.unsettled_items.entry((old.id, old.count)).or_insert(
                                UnsettledItem {
                                    since: Instant::now(),
                                },
                            );
                        } else {
                            self.witness.confirmed_drops =
                                self.witness.confirmed_drops.saturating_add(1);
                            self.witness.dropped_product_ids.insert(old.id);
                        }
                    } else if !self
                        .snapshot
                        .equipment()
                        .iter()
                        .any(|item| item.def.id == old.id && item.count == old.count)
                    {
                        self.unsettled_items
                            .entry((old.id, old.count))
                            .or_insert(UnsettledItem {
                                since: Instant::now(),
                            });
                    }
                }
            }
            let pending_drop_count = i64::try_from(self.unsettled_items.len()).unwrap_or(i64::MAX);
            let observed_drops = self
                .witness
                .last_status_dropped
                .saturating_sub(drop_baseline);
            let product_drops = self
                .witness
                .confirmed_drops
                .saturating_sub(product_drop_baseline);
            if !bank_active
                && pending_drop_count > 0
                && observed_drops.saturating_sub(i64::from(product_drops)) >= pending_drop_count
            {
                self.unsettled_items.clear();
            }
            self.last_observed_dropped = self.witness.last_status_dropped;
            self.last_observed_product_drops = self.witness.confirmed_drops;
        }
        if let Some((&(id, count), _)) = self
            .unsettled_items
            .iter()
            .find(|(_, item)| item.since.elapsed() >= Duration::from_secs(2))
        {
            return Err(format!(
                "{}: non-product id {id} count {count} was not conserved",
                self.name()
            ));
        }
        self.witness.max_product_count = self
            .witness
            .max_product_count
            .max(observation.product_count);
        self.witness.last_xp = observation.xp;
        if self.case.is_death_recovery() {
            for (&death, &(baseline_xp, baseline_products, _)) in
                &self.witness.death_progress_baselines
            {
                if observation.product_count > baseline_products {
                    self.witness.death_products_after.insert(death);
                }
                if observation.xp > baseline_xp {
                    self.witness.death_xp_after.insert(death);
                }
            }
        }
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
        let inventory_count = |id: i32| {
            observation
                .inventory
                .values()
                .filter(|item| item.id == id)
                .map(|item| item.count)
                .sum::<i32>()
        };
        if self.case == LiveCase::FishBait && self.witness.bank_arrivals > 0 {
            self.witness.bait_exact |= inventory_count(FEATHERS_ID) == 3;
        }
        if self.case == LiveCase::CoinRunes && self.witness.bank_arrivals > 0 {
            self.witness.coin_balance_exact = inventory_count(995) == 100;
            let baseline = self
                .baseline
                .as_ref()
                .ok_or("missing coin-runes Start baseline")?;
            self.witness.reserve_runes_intact &= self.plan.reserve_runes.iter().all(|(id, _)| {
                baseline
                    .inventory
                    .values()
                    .filter(|item| item.id == *id)
                    .map(|item| item.count)
                    .sum::<i32>()
                    == inventory_count(*id)
            });
        }
        if matches!(
            self.case,
            LiveCase::WoodcuttingBank | LiveCase::WoodcuttingBankUnwieldable
        ) {
            self.witness.tool_worn_observed |= self
                .snapshot
                .equipment()
                .iter()
                .any(|item| item.def.id == self.tool_id);
            if self.case == LiveCase::WoodcuttingBankUnwieldable && self.witness.tool_worn_observed
            {
                return Err("unwieldable rune axe was equipped".into());
            }
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

    fn snapshot_inventory_counts(&self) -> BTreeMap<i32, i32> {
        aggregate_item_counts(
            self.snapshot
                .inventory()
                .iter()
                .map(|item| (item.def.id, item.count)),
        )
    }

    fn snapshot_bank_counts(&self) -> BTreeMap<i32, i32> {
        aggregate_item_counts(
            self.snapshot
                .bank()
                .iter()
                .map(|item| (item.def.id, item.count)),
        )
    }

    fn bank_conservation_active(&self) -> bool {
        self.case.is_bank_mode()
            || (self.case == LiveCase::PowerToBank && self.witness.power_to_bank_applied)
    }

    fn native_deposit_ids(&self) -> Result<Arc<[i32]>, String> {
        let config = self
            .bank_config
            .as_deref()
            .ok_or_else(|| format!("{} has no prepared Gatherer bank settings", self.name()))?;
        script::gatherer::test_bank_deposit_ids(config, &self.snapshot)
            .ok_or_else(|| format!("{} bank settings are not a Gatherer config", self.name()))
    }

    fn observe_bank_deposit(
        &mut self,
        status: &script::native::ScriptStatus,
        current_deposited: i64,
    ) -> Result<(), String> {
        if !self.bank_conservation_active() {
            return Ok(());
        }
        let bank = text_field(status, "bank");
        let bank_step = bank.and_then(|bank| bank.rsplit_once("; ").map(|(_, step)| step));
        let event = text_field(status, "last_event");
        if bank_step == Some("Deposit")
            && self.snapshot.bank_loaded()
            && self.witness.pending_bank_trip.is_none()
        {
            let inventory_before = self.snapshot_inventory_counts();
            let deposit_ids = self.native_deposit_ids()?;
            let seeded_casket_expected =
                if self.case.requires_seeded_casket() && self.witness.bank_trips.is_empty() {
                    self.witness.seeded_casket_baseline_count
                } else {
                    0
                };
            if seeded_casket_expected > 0
                && inventory_before.get(&CASKET_ID).copied().unwrap_or(0) < seeded_casket_expected
            {
                return Err(format!(
                    "{} lost its seeded casket before the first bank deposit: {:?}",
                    self.name(),
                    inventory_before
                ));
            }
            let receipt = BankTripReceipt::new(
                self.witness.status_trips.saturating_add(1),
                inventory_before,
                &deposit_ids,
                self.snapshot_bank_counts(),
                current_deposited,
                self.cell.products(self.case),
                seeded_casket_expected,
            );
            self.witness.pending_bank_trip = Some(receipt);
        }
        if event == Some("deposit confirmed") && self.witness.pending_bank_trip.is_none() {
            return Err(format!(
                "{} confirmed a deposit without pre-deposit inventory evidence",
                self.name()
            ));
        }

        let should_observe_deposit =
            self.witness
                .pending_bank_trip
                .as_ref()
                .is_some_and(|receipt| {
                    !receipt.deposit_verified
                        && (event == Some("deposit confirmed") || receipt.deposit_confirmed)
                });
        if should_observe_deposit {
            let inventory_after = self.snapshot_inventory_counts();
            let deposit_ids = self.native_deposit_ids()?;
            let unneeded_after = bank_deposit_expectation(&inventory_after, &deposit_ids);
            let bank_loaded = self.snapshot.bank_loaded();
            let bank_after = if bank_loaded {
                self.snapshot_bank_counts()
            } else {
                BTreeMap::new()
            };
            let result = self
                .witness
                .pending_bank_trip
                .as_mut()
                .expect("pending receipt was just observed")
                .observe_deposit(
                    &inventory_after,
                    &unneeded_after,
                    bank_loaded,
                    &bank_after,
                    current_deposited,
                    event == Some("deposit confirmed"),
                );
            result.map_err(|error| format!("{}: {error}", self.name()))?;
        }
        if event == Some("bank closed; validating equipment") {
            let receipt = self.witness.pending_bank_trip.as_ref();
            if !receipt.is_some_and(|receipt| receipt.deposit_verified) {
                return Err(format!(
                    "{} closed the bank before its per-trip deposit was verified: {receipt:?}",
                    self.name()
                ));
            }
        }
        Ok(())
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
                if !self.case.bank_tool_only()
                    && (!self.fixture_helper || self.case == LiveCase::OakRespawn)
                    && !self.item_in_inventory()
                {
                    send_cheat(client, &format!("give {} 1", self.tool_alias))?;
                }
                for (alias, count) in &self.plan.inventory_seed {
                    send_cheat(client, &format!("give {alias} {count}"))?;
                }
                for (alias, count) in &self.plan.bank_seed {
                    send_cheat(client, &format!("givebank {alias} {count}"))?;
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
                        || self.case.bank_tool_only()
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
                    } else if self.case.bank_tool_only()
                        || matches!(self.cell, Cell::Fishing | Cell::Mining)
                    {
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
        let seeded_gem = self.cell == Cell::Mining && self.case == LiveCase::Power;
        let seeded_count: i32 = baseline
            .inventory
            .values()
            .filter(|row| row.id == SEEDED_MINING_GEM_ID)
            .map(|row| row.count)
            .sum();
        if baseline.product_count != i32::from(seeded_gem) || (seeded_gem && seeded_count != 1) {
            return Err(format!(
                "{} Start baseline must contain only its deterministic gem fixture (mining Power: {seeded_gem}); products={} sapphire={seeded_count}",
                self.name(), baseline.product_count
            ));
        }
        if self.case.requires_seeded_casket() {
            let casket_count = baseline
                .inventory
                .values()
                .filter(|item| item.id == CASKET_ID)
                .map(|item| item.count)
                .sum::<i32>();
            if casket_count != 1 {
                return Err(format!(
                    "{} Start baseline must contain exactly one seeded casket {CASKET_ID}, found {casket_count}",
                    self.name()
                ));
            }
            self.witness.seeded_casket_baseline_count = casket_count;
            self.witness.seeded_casket_baseline_verified = true;
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
            Cell::Woodcutting if self.case.bank_tool_only() => {
                !self.item_in_inventory() && !self.item_equipped()
            }
            Cell::Woodcutting => self.item_equipped(),
            Cell::Mining if self.case == LiveCase::BankCost => {
                !self.item_in_inventory() && !self.item_equipped()
            }
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
        self.witness.coin_seed_count = baseline
            .inventory
            .values()
            .filter(|item| item.id == 995)
            .map(|item| item.count)
            .sum();
        self.witness.last_return_deposited = self.witness.status_deposited;
        if self.case == LiveCase::CoinRunes {
            let exact_reserve = self.plan.reserve_runes.iter().all(|(id, count)| {
                baseline
                    .inventory
                    .values()
                    .filter(|item| item.id == *id)
                    .map(|item| item.count)
                    .sum::<i32>()
                    == *count
            });
            if self.plan.reserve_runes.is_empty() || !exact_reserve {
                return Err(format!(
                    "{} Start baseline lacks exactly one selected teleport cast",
                    self.name()
                ));
            }
            self.witness.reserve_runes_intact = true;
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
        if self.bank_conservation_active() {
            let phase = text_field(status, "phase");
            let event = text_field(status, "last_event");
            let bank = text_field(status, "bank");
            if self.witness.bank_status_sequence.last().is_none_or(|last| {
                last["phase"].as_str() != phase
                    || last["event"].as_str() != event
                    || last["bank"].as_str() != bank
            }) {
                let row = json!({
                    "tick": self.latest.as_ref().map(|row| row.tick),
                    "phase": phase,
                    "event": event,
                    "bank": bank,
                    "tool": text_field(status, "tool"),
                    "trips": integer_field(status, "trips"),
                    "deposited": integer_field(status, "deposited"),
                    "yielded": integer_field(status, "yielded"),
                    "xp": self.latest.as_ref().map(|row| row.xp),
                    "tile": self.latest.as_ref().and_then(|row| row.tile),
                    "products": self.latest.as_ref().map(|row| row.product_count),
                });
                println!("gatherer-bank-status {row}");
                self.witness.bank_status_sequence.push(row);
            }
        }
        let previous_yielded = self.witness.last_status_yielded;
        let current_yielded = integer_field(status, "yielded").unwrap_or(previous_yielded);
        let current_trips = integer_field(status, "trips").unwrap_or(self.witness.status_trips);
        let current_deposited =
            integer_field(status, "deposited").unwrap_or(self.witness.status_deposited);
        if self.bank_conservation_active() {
            self.observe_bank_deposit(status, current_deposited)?;
        }
        let previous_deaths = self.witness.last_deaths;
        let previous_recoveries = self.witness.last_recoveries;
        let current_deaths = integer_field(status, "deaths")
            .and_then(|value| u8::try_from(value).ok())
            .unwrap_or(previous_deaths);
        let current_recoveries = integer_field(status, "recoveries")
            .and_then(|value| u8::try_from(value).ok())
            .unwrap_or(previous_recoveries);
        let current_recovery_step = integer_field(status, "recovery_step")
            .and_then(|value| u8::try_from(value).ok())
            .unwrap_or(self.witness.last_recovery_step);
        if current_trips > self.witness.status_trips && self.bank_conservation_active() {
            if current_trips != self.witness.status_trips.saturating_add(1) {
                return Err(format!(
                    "{} skipped a bank trip receipt: {} -> {}",
                    self.name(),
                    self.witness.status_trips,
                    current_trips
                ));
            }
            let Some(mut receipt) = self.witness.pending_bank_trip.take() else {
                return Err(format!(
                    "{} returned from banking without a pre-deposit receipt",
                    self.name()
                ));
            };
            if let Err(error) = receipt.record_return(
                current_trips,
                current_deposited,
                self.case.is_fish_harpoon_bank(),
            ) {
                self.witness.pending_bank_trip = Some(receipt);
                return Err(format!("{}: {error}", self.name()));
            }
            // A supply-only visit verifies its empty deposit boundary, but is
            // not a completed deposit trip and must not produce such a receipt.
            if receipt.is_deposit_trip() {
                self.witness.bank_trips.push(receipt);
            }
            self.witness.bank_arrivals = self
                .witness
                .bank_arrivals
                .saturating_add((current_trips - self.witness.status_trips) as u32);
            self.witness.waiting_for_return_yield = true;
            self.witness.waiting_for_bank_yield =
                current_deposited > self.witness.last_return_deposited;
            self.witness.last_return_deposited = current_deposited;
        }
        if self.witness.waiting_for_return_yield && current_yielded > previous_yielded {
            self.witness.post_bank_yields = self.witness.post_bank_yields.saturating_add(1);
            if self.witness.waiting_for_bank_yield {
                self.witness.bank_nonzero_roundtrips =
                    self.witness.bank_nonzero_roundtrips.saturating_add(1);
            }
            if self.bank_conservation_active() && self.witness.waiting_for_bank_yield {
                let Some(receipt) = self
                    .witness
                    .bank_trips
                    .iter_mut()
                    .rev()
                    .find(|receipt| !receipt.post_bank_yield)
                else {
                    return Err(format!(
                        "{} observed a post-bank yield without a matching trip receipt",
                        self.name()
                    ));
                };
                receipt.post_bank_yield = true;
            }
            self.witness.waiting_for_return_yield = false;
            self.witness.waiting_for_bank_yield = false;
        }
        self.witness.status_trips = current_trips;
        self.witness.status_deposited = current_deposited;
        self.witness.last_status_yielded = current_yielded;
        self.witness.last_status_dropped =
            integer_field(status, "dropped").unwrap_or(self.witness.last_status_dropped);
        self.witness.last_deaths = current_deaths;
        self.witness.last_recoveries = current_recoveries;
        self.witness.last_recovery_step = current_recovery_step;
        if current_deaths > previous_deaths {
            if let Some(latest) = self.latest.as_ref() {
                self.witness.death_progress_baselines.insert(
                    current_deaths,
                    (latest.xp, latest.product_count, current_yielded),
                );
                self.witness.death_recovery_in_area |= latest
                    .tile
                    .and_then(|tile| tile_distance(tile, self.target))
                    .is_some_and(|distance| distance <= 12);
                if self.case == LiveCase::DeathRespawnRegion {
                    self.witness.death_respawn_region_unchanged = self
                        .witness
                        .death_command_tile
                        .zip(latest.tile)
                        .is_some_and(|(death, respawn)| {
                            let respawn_tile = WorldTile {
                                x: respawn.0,
                                z: respawn.1,
                                level: respawn.2,
                            };
                            script::native::death::RESPAWN_SQUARE.contains(respawn_tile)
                                && death.0.div_euclid(64) == respawn.0.div_euclid(64)
                                && death.1.div_euclid(64) == respawn.1.div_euclid(64)
                                && death.2 == respawn.2
                        });
                }
            }
        }
        if self.case == LiveCase::MazeRandom
            && self.witness.maze_exited
            && current_yielded > self.witness.maze_exit_yielded
            && self
                .latest
                .as_ref()
                .is_some_and(|latest| latest.xp > self.witness.maze_exit_xp)
        {
            self.witness.maze_resumed = true;
        }
        if self.case == LiveCase::RandomEvent {
            if self.witness.random_owned_released
                && current_yielded > self.witness.random_release_yielded
                && self
                    .latest
                    .as_ref()
                    .is_some_and(|latest| latest.xp > self.witness.random_release_xp)
            {
                self.witness.random_fresh_yield = true;
                self.witness.random_owned_revalidated = text_field(status, "method")
                    .is_some_and(|method| !method.is_empty())
                    && text_field(status, "target").is_some_and(|target| !target.is_empty());
            }
            if self.witness.random_foreign_event_seen
                && current_yielded > self.witness.random_foreign_yield_baseline
                && self
                    .latest
                    .as_ref()
                    .is_some_and(|latest| latest.xp > self.witness.random_foreign_xp_baseline)
            {
                self.witness.random_foreign_fresh_yield = true;
            }
        }
        self.witness.last_status_bank = text_field(status, "bank").map(str::to_owned);
        if let Some(bank) = self.witness.last_status_bank.as_deref() {
            if let Some((_, step)) = bank.rsplit_once("; ") {
                self.witness.bank_loaded_observed |= self.snapshot.bank_loaded();
                self.witness.bank_return_step_seen |= step == "Return";
                if step == "Return" {
                    self.witness.bank_return_after_pause |= self.case
                        == LiveCase::PauseResumeOtherPlane
                        && self.witness.resume_observed;
                    self.witness.bank_return_after_reconnect |=
                        self.case == LiveCase::ReconnectReturn && self.witness.reconnect_observed;
                }
            }
        }
        if matches!(
            self.case,
            LiveCase::WoodcuttingBank | LiveCase::WoodcuttingBankUnwieldable
        ) {
            if let Some(tool) = text_field(status, "tool") {
                let expected = self.tool_id.to_string();
                if tool == expected {
                    self.witness.tool_selected_observed = true;
                } else if let Some(id) = tool.strip_suffix(" (worn)") {
                    if id == expected {
                        self.witness.tool_selected_observed = true;
                        self.witness.tool_worn_observed = true;
                        if self.case == LiveCase::WoodcuttingBankUnwieldable {
                            return Err("unwieldable rune axe status says worn".into());
                        }
                    }
                }
            }
        }
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
            if event == "bank trip due" {
                if self.witness.bank_trip_due_at.is_none() {
                    self.witness.bank_trip_due_at = Some(Instant::now());
                }
                if self.case == LiveCase::BankCost && self.plan.bank_cost_air_nearest.is_none() {
                    if let Some((x, z, level)) = self.snapshot.tile() {
                        let from = WorldTile { x, z, level };
                        self.plan.bank_cost_origin = Some(from);
                        self.plan.bank_cost_air_nearest = self
                            .plan
                            .bank_cost_air_banks
                            .iter()
                            .enumerate()
                            .min_by_key(|(index, (_, tile))| {
                                (bank_air_distance(from, *tile), *index)
                            })
                            .map(|(_, (name, _))| name.clone());
                    }
                }
            }
            if event == "bank selected" && self.witness.bank_selection_elapsed.is_none() {
                self.witness.bank_selection_elapsed = self
                    .witness
                    .bank_trip_due_at
                    .map(|started| Instant::now().duration_since(started));
            }
            self.witness.last_status_event = Some(event.to_owned());
            if let Some(death) = event
                .strip_prefix("recovered after death ")
                .and_then(|value| value.parse::<u8>().ok())
            {
                self.witness.death_recovered.insert(death);
                if death == 1 {
                    self.witness.death_first_recovery_cycles = Some(self.witness.cycles);
                    self.witness.death_first_recovery_post_drop_gathers =
                        Some(self.witness.post_drop_gathers);
                }
            }
            if event == "bank selected" {
                self.witness.bank_selected = true;
                if self.case == LiveCase::PowerToBank {
                    let completed = self
                        .witness
                        .confirmed_drops
                        .saturating_sub(self.witness.power_to_bank_edit_drop_baseline)
                        >= self.witness.power_to_bank_edit_slots;
                    if !self.witness.power_to_bank_edit_requested
                        || !self.witness.power_to_bank_applied
                        || !completed
                    {
                        return Err(format!(
                            "bank selection preempted the boundary-applied Power to Bank batch: {:?}",
                            self.witness
                        ));
                    }
                }
            }
            if event == "withdrawal confirmed" {
                self.witness.bank_withdrawal_confirmed = true;
            }
            if event == "bank closed; validating equipment" {
                self.witness.bank_closed_observed = true;
            }
            if self.case == LiveCase::FishBait
                && event == "withdrawal confirmed"
                && integer_field(status, "bait") == Some(3)
            {
                self.witness.bait_exact = true;
            }
            if self.case == LiveCase::WoodcuttingBankUnwieldable && event == "wielding tool" {
                return Err("unwieldable rune axe wear action was started".into());
            }
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
            if self.case.is_fish_harpoon_bank() {
                let row = json!({
                    "tick": self.latest.as_ref().map(|latest| latest.tick),
                    "phase": text_field(status, "phase"),
                    "event": text_field(status, "last_event"),
                    "bank": text_field(status, "bank"),
                    "tile": self.latest.as_ref().and_then(|latest| latest.tile),
                    "failure_code": &code,
                    "failure_message": &message,
                });
                if self.witness.failure_sequence.last().is_none_or(|last| {
                    last["phase"].as_str() != row["phase"].as_str()
                        || last["event"].as_str() != row["event"].as_str()
                        || last["bank"].as_str() != row["bank"].as_str()
                        || last["failure_code"].as_str() != Some(code.as_str())
                        || last["failure_message"].as_str() != Some(message.as_str())
                }) {
                    println!("gatherer-bank-failure {row}");
                    self.witness.failure_sequence.push(row);
                }
            }
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
                LiveCase::DeathNoStock => code == "supply-missing",
                LiveCase::MazeRandom => {
                    let inside = self
                        .latest
                        .as_ref()
                        .and_then(|latest| latest.tile)
                        .is_some_and(|(x, z, level)| {
                            api::random::trapped_area(x, z, level)
                                == Some(api::random::RandomKind::Maze)
                        });
                    self.witness.maze_blocked_inside |= code == "random-trapped" && inside;
                    code == "random-trapped" && inside
                }
                LiveCase::DeathReturnRefused => {
                    self.witness.death_return_refused |=
                        code == "return-failed" && message.contains("walk ended with Refused");
                    code == "return-failed"
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
        if self.case == LiveCase::DeathReturn
            && self
                .witness
                .death_first_recovery_cycles
                .is_some_and(|cycles| {
                    self.witness.cycles > cycles
                        && self
                            .witness
                            .death_first_recovery_post_drop_gathers
                            .is_some_and(|gathers| self.witness.post_drop_gathers > gathers)
                })
        {
            self.witness.death_haul_between = true;
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
            LiveCase::MazeRandom => {
                if maze_random_complete(&self.witness) {
                    Ok(())
                } else {
                    Err(format!(
                        "{} maze proof incomplete: {:?}",
                        self.name(),
                        self.witness
                    ))
                }
            }
            LiveCase::RandomEvent
            | LiveCase::DeathReturn
            | LiveCase::DeathRespawnRegion
            | LiveCase::DeathNoStock
            | LiveCase::DeathReturnRefused
            | LiveCase::DeathWatchdogPending
            | LiveCase::DeathWatchdogProving => {
                if interrupts_complete(self.case, &self.witness) {
                    Ok(())
                } else {
                    Err(format!(
                        "{} interrupt proof incomplete: {:?}",
                        self.name(),
                        self.witness
                    ))
                }
            }
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
            LiveCase::WoodcuttingBank | LiveCase::WoodcuttingBankUnwieldable => {
                let required_trips = 3;
                if !self.complete_bank_selection()
                    || !self.witness.tool_selected_observed
                    || self.witness.bank_nonzero_roundtrips < 2
                    || self.witness.post_bank_yields < required_trips
                    || !self.status_integer_seen("trips", required_trips as i64)
                    || !self.witness.bank_loaded_observed
                    || !self.witness.bank_closed_observed
                    || !self.witness.bank_withdrawal_confirmed
                    || self.witness.last_status_yielded <= 0
                    || self
                        .latest
                        .as_ref()
                        .is_none_or(|latest| latest.xp <= self.baseline_xp())
                {
                    return Err(format!(
                        "{} did not complete the tool-only trip and two positive bank trips with fresh yields after returns: {:?}",
                        self.name(),
                        self.witness
                    ));
                }
                if self.case == LiveCase::WoodcuttingBank
                    && (!self.witness.tool_worn_observed
                        || !self.status_seen("last_event", "wielding tool"))
                {
                    return Err(format!(
                        "{} did not wield the selected bronze axe: {:?}",
                        self.name(),
                        self.witness
                    ));
                }
                if self.case == LiveCase::WoodcuttingBankUnwieldable
                    && self.witness.tool_worn_observed
                {
                    return Err(format!(
                        "{} incorrectly marked the unwieldable rune axe worn: {:?}",
                        self.name(),
                        self.witness
                    ));
                }
                Ok(())
            }
            LiveCase::BankCost => {
                let selected = self.selected_bank_name();
                let expected_air_nearest = self.plan.bank_cost_air_nearest.as_deref();
                let selected_is_gate_disabled = selected.is_some_and(|selected| {
                    self.plan
                        .bank_cost_gated_candidates
                        .iter()
                        .any(|candidate| candidate == selected)
                });
                let has_required_gates = ["Zanaris", "Fishing Guild"].into_iter().all(|required| {
                    self.plan
                        .bank_cost_gated_candidates
                        .iter()
                        .any(|candidate| candidate.as_str() == required)
                });
                let mining_area_selected = self
                    .witness
                    .last_area
                    .as_deref()
                    .is_some_and(|area| area != "—");
                if self.cell != Cell::Mining
                    || !mining_area_selected
                    || self.plan.bank_cost_origin != Some(world_tile(3016, 9840))
                    || !self.complete_bank_selection()
                    || self.selected_bank_kind() != Some("Reachable")
                    || selected.is_none()
                    || expected_air_nearest.is_none()
                    || selected == expected_air_nearest
                    || selected_is_gate_disabled
                    || !has_required_gates
                    || !self.witness.bank_loaded_observed
                {
                    return Err(format!(
                        "{} did not admit the Dwarven Mine mining area and open a reachable cost winner distinct from the eligible air-nearest bank while excluding disabled Zanaris/Fishing Guild: {:?}",
                        self.name(),
                        self.witness
                    ));
                }
                Ok(())
            }
            LiveCase::BankCostFirstGoal | LiveCase::BankCostSeersMaple => {
                let (expected_bank, resource) = match self.case {
                    LiveCase::BankCostFirstGoal => ("Draynor", "oak"),
                    LiveCase::BankCostSeersMaple => ("Seers", "maple"),
                    _ => unreachable!("real-content bank case"),
                };
                if !real_content_bank_lifecycle_complete(
                    self.case,
                    &self.witness,
                    &self.plan,
                    self.target,
                    self.baseline_xp(),
                ) || !self.complete_bank_selection()
                    || self.selected_bank_name() != Some(expected_bank)
                    || self.selected_bank_kind() != Some("Reachable")
                    || self
                        .latest
                        .as_ref()
                        .is_none_or(|latest| latest.xp <= self.baseline_xp())
                {
                    return Err(format!(
                        "{} did not gather real {resource} and complete the reachable {expected_bank} bank, deposit, return, and fresh-yield lifecycle without seeded scene locations: {:?}",
                        self.name(),
                        self.witness
                    ));
                }
                Ok(())
            }
            LiveCase::BankCostNoCandidate => {
                if self.witness.failure_code.as_deref() != Some("bank-unavailable")
                    || self.witness.bank_selected
                    || self.witness.bank_loaded_observed
                    || self.witness.status_trips != 0
                    || self.witness.last_status_yielded != 0
                    || self
                        .latest
                        .as_ref()
                        .is_none_or(|latest| latest.xp != self.baseline_xp())
                {
                    return Err(format!(
                        "{} did not fail closed without a viable bank candidate: {:?}",
                        self.name(),
                        self.witness
                    ));
                }
                Ok(())
            }
            LiveCase::FishBait => {
                if !self.complete_bank_selection()
                    || !self.witness.bait_exact
                    || !self.witness.bank_withdrawal_confirmed
                    || !self.witness.bank_closed_observed
                    || !self.status_integer_seen("trips", 1)
                    || self.witness.post_bank_yields == 0
                    || self.witness.last_status_yielded <= 0
                    || self
                        .latest
                        .as_ref()
                        .is_none_or(|latest| latest.xp <= self.baseline_xp())
                {
                    return Err(format!(
                        "{} did not withdraw exactly three bait and resume fishing after banking: {:?}",
                        self.name(),
                        self.witness
                    ));
                }
                Ok(())
            }
            LiveCase::Site => {
                if !site_bank_complete(self.cell, &self.witness, &self.plan, self.baseline_xp())
                    || !self.complete_bank_selection()
                    || self
                        .latest
                        .as_ref()
                        .is_none_or(|latest| latest.product_count == 0)
                    || (self.cell == Cell::Fishing
                        && (!self.item_in_inventory() || self.item_equipped()))
                {
                    return Err(format!("{} did not prove a selected resource anchor, initial approach, nearest bank conservation, Area r1 Return and fresh yield: {:?}", self.name(), self.witness));
                }
                Ok(())
            }
            LiveCase::FishHarpoonBank | LiveCase::FishHarpoonBankAuto => {
                let seeded_casket_required = self.case.requires_seeded_casket();
                if !self.complete_bank_selection()
                    || !self.witness.bank_loaded_observed
                    || !self.witness.bank_closed_observed
                    || !self.witness.fish_bank_complete(seeded_casket_required)
                    || !self.item_in_inventory()
                    || self.item_equipped()
                    || self.latest.as_ref().is_none_or(|latest| {
                        latest.product_count == 0 || latest.xp <= self.baseline_xp()
                    })
                {
                    let casket_requirement = if seeded_casket_required {
                        " and bank its seeded casket"
                    } else {
                        ""
                    };
                    return Err(format!(
                        "{} did not complete two verified positive fish-bank-fish returns{} with the harpoon in inventory: {:?}",
                        self.name(),
                        casket_requirement,
                        self.witness
                    ));
                }
                Ok(())
            }
            LiveCase::CoinRunes => {
                if !self.complete_bank_selection()
                    || !self.witness.bank_withdrawal_confirmed
                    || !self.witness.coin_balance_exact
                    || !self.witness.reserve_runes_intact
                    || !self.status_integer_seen("trips", 1)
                    || !self.witness.bank_closed_observed
                    || self.witness.last_status_yielded <= 0
                    || self
                        .latest
                        .as_ref()
                        .is_none_or(|latest| latest.xp <= self.baseline_xp())
                {
                    return Err(format!(
                        "{} did not top up to the exact coin target while preserving reserve runes and yielding: {:?}",
                        self.name(),
                        self.witness
                    ));
                }
                Ok(())
            }
            LiveCase::CoinRunesEmpty => {
                let at_withdraw = self
                    .witness
                    .last_status_bank
                    .as_deref()
                    .is_some_and(|bank| bank.ends_with("; Withdraw"));
                if self.witness.failure_code.as_deref() != Some("supply-missing")
                    || !self.witness.bank_selected
                    || !self.witness.bank_loaded_observed
                    || !at_withdraw
                    || self.witness.status_trips != 0
                    || self.witness.last_status_yielded != 0
                    || self
                        .latest
                        .as_ref()
                        .is_none_or(|latest| latest.xp != self.baseline_xp())
                {
                    return Err(format!(
                        "{} did not block at the loaded bank when reserve runes were unavailable: {:?}",
                        self.name(),
                        self.witness
                    ));
                }
                Ok(())
            }
            LiveCase::PowerToBank => {
                if !self.complete_bank_selection()
                    || !self.witness.power_to_bank_edit_requested
                    || !self.witness.power_to_bank_applied
                    || self.witness.power_to_bank_edit_slots == 0
                    || self
                        .witness
                        .confirmed_drops
                        .saturating_sub(self.witness.power_to_bank_edit_drop_baseline)
                        < self.witness.power_to_bank_edit_slots
                    || !self.witness.bank_closed_observed
                    || self.witness.bank_nonzero_roundtrips == 0
                    || self.witness.post_bank_yields == 0
                    || self
                        .latest
                        .as_ref()
                        .is_none_or(|latest| latest.xp <= self.baseline_xp())
                {
                    return Err(format!(
                        "{} did not apply Power to Bank at a completed batch boundary and resume after deposit: {:?}",
                        self.name(),
                        self.witness
                    ));
                }
                Ok(())
            }
            LiveCase::PauseResumeOtherPlane => {
                if !self.complete_bank_selection()
                    || !self.witness.bank_withdrawal_confirmed
                    || !self.witness.bank_closed_observed
                    || !self.status_integer_seen("trips", 1)
                    || !self.witness.pause_observed
                    || !self.witness.other_plane_observed
                    || !self.witness.resume_observed
                    || !self.witness.bank_return_step_seen
                    || !self.witness.bank_return_after_pause
                    || self.witness.post_bank_yields == 0
                    || self.latest.as_ref().is_none_or(|latest| {
                        latest.tile.is_none_or(|tile| tile.2 != self.target.level)
                            || latest.xp <= self.baseline_xp()
                    })
                {
                    return Err(format!(
                        "{} did not pause, observe another plane, resume its bank return and yield: {:?}",
                        self.name(),
                        self.witness
                    ));
                }
                Ok(())
            }
            LiveCase::ReconnectReturn => {
                if !self.complete_bank_selection()
                    || !self.witness.bank_withdrawal_confirmed
                    || !self.witness.bank_closed_observed
                    || !self.status_integer_seen("trips", 1)
                    || !self.witness.reconnect_offline_observed
                    || !self.witness.reconnect_observed
                    || !self.witness.bank_return_step_seen
                    || !self.witness.bank_return_after_reconnect
                    || self.witness.post_bank_yields == 0
                    || self
                        .latest
                        .as_ref()
                        .is_none_or(|latest| latest.xp <= self.baseline_xp())
                {
                    return Err(format!(
                        "{} did not preserve the bank-return lifecycle across reconnect and yield: {:?}",
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
    fn status_seen(&self, key: &str, expected: &str) -> bool {
        self.witness
            .status_values
            .get(key)
            .is_some_and(|values| values.contains(expected))
    }

    fn status_integer_seen(&self, key: &str, expected: i64) -> bool {
        self.witness.status_values.get(key).is_some_and(|values| {
            values
                .iter()
                .any(|value| value.parse::<i64>().ok() == Some(expected))
        })
    }

    fn selected_bank_name(&self) -> Option<&str> {
        self.witness
            .last_status_bank
            .as_deref()
            .and_then(|bank| bank.split(';').next())
            .map(str::trim)
            .filter(|name| !name.is_empty() && *name != "—")
    }

    fn selected_bank_kind(&self) -> Option<&str> {
        self.witness
            .last_status_bank
            .as_deref()
            .and_then(|bank| bank.split(';').nth(1))
            .map(str::trim)
            .filter(|kind| !kind.is_empty())
    }

    fn complete_bank_selection(&self) -> bool {
        self.witness.bank_selected
            && self.selected_bank_name().is_some()
            && self.selected_bank_kind().is_some()
            && self
                .witness
                .last_status_bank
                .as_deref()
                .is_some_and(|bank| bank.contains("; access:"))
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

fn save_live_capture(
    client: &mut client::client::Client,
    directory: &Path,
    receipt: &Value,
) -> Result<(), String> {
    std::fs::create_dir_all(directory).map_err(|error| error.to_string())?;
    let epoch = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| error.to_string())?
        .as_secs();
    let stem = format!("{epoch}Z_01-final");
    let mut renderer = client::render::Renderer::new_prefer(client.config.lowmem, false);
    let was_draw = client.draw;
    client.set_draw(true);
    let frame = renderer.mainredraw(client);
    client.set_draw(was_draw);
    let client::render::backend::FrameOutput::PixMap(pixels) = frame else {
        return Err("live capture did not return a CPU PixMap".into());
    };
    let mut rgba = Vec::with_capacity(pixels.pixels.len() * 4);
    for pixel in &pixels.pixels {
        rgba.extend_from_slice(&[
            ((pixel >> 16) & 0xff) as u8,
            ((pixel >> 8) & 0xff) as u8,
            (pixel & 0xff) as u8,
            u8::MAX,
        ]);
    }
    let file = std::fs::File::create(directory.join(format!("{stem}.png")))
        .map_err(|error| error.to_string())?;
    let mut encoder = png::Encoder::new(file, pixels.width as u32, pixels.height as u32);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder
        .write_header()
        .map_err(|error| error.to_string())?
        .write_image_data(&rgba)
        .map_err(|error| error.to_string())?;
    let mut receipt = receipt.clone();
    receipt["frame"] = json!({
        "ingame": client.ingame,
        "scene_state": client.scene_state,
        "renderer": "real Client CpuPix3D framebuffer",
    });
    std::fs::write(
        directory.join(format!("{stem}.json")),
        serde_json::to_vec_pretty(&receipt).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())
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

fn bank_air_distance(from: WorldTile, to: WorldTile) -> i64 {
    let dx = i64::from(from.x) - i64::from(to.x);
    let dz = i64::from(from.z) - i64::from(to.z);
    dx * dx + dz * dz
}

fn configure_bank_cost_plan(
    template: &SharedClientTemplate,
    plan: &mut FixturePlan,
) -> Result<(), String> {
    let world = template
        .world()
        .ok_or("BankCost selected template has no navigation world")?;
    if world.named_bank_facts().is_none() {
        let data = template
            .profile()
            .game_data()
            .ok_or("BankCost selected profile has no game data")?;
        world.bind_named_bank_facts(&data)?;
    }
    let facts = world
        .named_bank_facts()
        .ok_or("BankCost selected template has no bound named-bank facts")?;
    let preferences = BANK_COST_FIXTURE_INTENT.bank_preferences();
    let eligible = |bank: &api::named_banks::NamedBank| {
        bank.eligible(|skill| (skill == 10).then_some(1), |_| false, preferences)
    };
    for bank in facts.banks().iter().filter(|bank| eligible(bank)) {
        plan.bank_cost_air_banks
            .push((bank.name.to_owned(), bank.air_tile()));
    }
    for bank in facts.banks().iter().filter(|bank| {
        bank.routable
            && bank.definition.is_some_and(|definition| {
                (definition.skill.is_some()
                    || definition.setting.is_some()
                    || definition.quest.is_some())
                    && !eligible(bank)
            })
    }) {
        plan.bank_cost_gated_candidates.push(bank.name.to_owned());
    }
    if plan.bank_cost_air_banks.is_empty() {
        return Err("BankCost selected navigation data has no eligible named banks".into());
    }
    let missing_gates: Vec<_> = ["Zanaris", "Fishing Guild"]
        .into_iter()
        .filter(|required| {
            !plan
                .bank_cost_gated_candidates
                .iter()
                .any(|candidate| candidate.as_str() == *required)
        })
        .collect();
    if !missing_gates.is_empty() {
        return Err(format!(
            "BankCost selected navigation data does not expose disabled Zanaris and Fishing Guild candidates: {missing_gates:?}"
        ));
    }
    Ok(())
}

fn fixture_plan(
    cell: Cell,
    case: LiveCase,
) -> Result<(WorldTile, FixturePlan, Option<FixtureTask>), String> {
    let mut plan = FixturePlan::default();
    match case {
        LiveCase::Power if cell == Cell::Mining => {
            plan.inventory_seed.push(("uncut_sapphire".into(), 1));
        }
        LiveCase::WoodcuttingBank => plan.bank_seed.push(("bronze_axe".into(), 1)),
        LiveCase::WoodcuttingBankUnwieldable => {
            plan.bank_seed.push(("bronze_axe".into(), 1));
            plan.bank_seed.push(("rune_axe".into(), 1));
        }
        LiveCase::BankCost => plan.bank_seed.push(("bronze_pickaxe".into(), 1)),
        LiveCase::BankCostFirstGoal
        | LiveCase::BankCostSeersMaple
        | LiveCase::PauseResumeOtherPlane
        | LiveCase::ReconnectReturn => plan.bank_seed.push(("bronze_axe".into(), 1)),
        LiveCase::DeathReturn => plan.bank_seed.push(("bronze_axe".into(), 3)),
        LiveCase::DeathNoStock => plan.inventory_seed.push(("bronze_axe".into(), 1)),
        LiveCase::DeathRespawnRegion
        | LiveCase::DeathReturnRefused
        | LiveCase::DeathWatchdogPending
        | LiveCase::DeathWatchdogProving => {
            plan.bank_seed.push(("bronze_axe".into(), 2));
        }
        LiveCase::FishBait => plan.bank_seed.push(("feather".into(), 7)),
        LiveCase::FishHarpoonBank => plan.inventory_seed.push(("casket".into(), 1)),
        LiveCase::CoinRunes => {
            plan.bank_seed.push(("coins".into(), 1_000));
            plan.inventory_seed.push(("coins".into(), 40));
        }
        LiveCase::CoinRunesEmpty => plan.bank_seed.push(("coins".into(), 1_000)),
        _ => {}
    }
    if case == LiveCase::DeathReturnRefused {
        // Start with a disposable carried tool, so only the post-death return
        // crosses the forbidden wilderness boundary.
        plan.inventory_seed.push(("bronze_axe".into(), 1));
    }
    if case.is_death_recovery() {
        plan.inventory_seed
            .extend(DEATH_FILLERS.iter().map(|(_, alias)| ((*alias).into(), 1)));
    }
    let target = match case {
        LiveCase::RandomEvent | LiveCase::MazeRandom => {
            fixture_tile(cell.tile_env(case), OAK_RESPAWN_START)?
        }
        LiveCase::DeathReturn
        | LiveCase::DeathNoStock
        | LiveCase::DeathWatchdogPending
        | LiveCase::DeathWatchdogProving => {
            let tile = fixture_tile(cell.tile_env(case), OAK_RESPAWN_START)?;
            plan.oak_tiles.push(tile);
            plan.seed_locs.push(seed(tile, "oaktree", OAK_ID));
            tile
        }
        LiveCase::DeathRespawnRegion => {
            let tile = fixture_tile(cell.tile_env(case), DEATH_REGION_START)?;
            plan.oak_tiles.push(tile);
            plan.seed_locs.push(seed(tile, "oaktree", OAK_ID));
            tile
        }
        LiveCase::DeathReturnRefused => {
            // Authored oak in m48_55; its radius-12 return remains north of
            // the wilderness boundary while allowWilderness stays false.
            let tile = fixture_tile(cell.tile_env(case), world_tile(3101, 3535))?;
            plan.oak_tiles.push(tile);
            plan.seed_locs.push(seed(tile, "oaktree", OAK_ID));
            tile
        }
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
        LiveCase::WoodcuttingBank
        | LiveCase::WoodcuttingBankUnwieldable
        | LiveCase::BankCostNoCandidate
        | LiveCase::CoinRunes
        | LiveCase::CoinRunesEmpty
        | LiveCase::PowerToBank
        | LiveCase::PauseResumeOtherPlane
        | LiveCase::ReconnectReturn => fixture_tile(cell.tile_env(case), OAK_RESPAWN_START)?,
        LiveCase::FishBait => fixture_tile(cell.tile_env(case), FISH_BAIT_START)?,
        LiveCase::FishHarpoonBank => fixture_tile(cell.tile_env(case), world_tile(2840, 3436))?,
        LiveCase::FishHarpoonBankAuto => CATHERBY_HARPOON_START,
        LiveCase::Site => match cell {
            Cell::Fishing => CATHERBY_HARPOON_START,
            Cell::Woodcutting => world_tile(3120, 3267),
            Cell::Mining => world_tile(3253, 3420),
        },
        LiveCase::BankCost => world_tile(3016, 9840),
        LiveCase::BankCostFirstGoal => DRAYNOR_OAK_BANK_START,
        LiveCase::BankCostSeersMaple => SEERS_MAPLE_BANK_START,
        _ => parse_tile(cell.tile_env(case), &required(cell.tile_env(case))?)?,
    };
    let task = match case {
        LiveCase::OakRespawn => plan.chop_target.map(|tile| FixtureTask::Chop {
            tile,
            stage: FixtureStage::Start,
        }),
        LiveCase::OakNearEdge
        | LiveCase::OakAbsentArea
        | LiveCase::LocationAuto
        | LiveCase::DeathReturn
        | LiveCase::DeathRespawnRegion
        | LiveCase::DeathNoStock
        | LiveCase::DeathReturnRefused
        | LiveCase::DeathWatchdogPending
        | LiveCase::DeathWatchdogProving => Some(FixtureTask::Seed {
            seeds: plan.seed_locs.clone(),
            next: 0,
            return_to: target,
            stage: FixtureStage::Start,
        }),
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
    if matches!(
        case,
        LiveCase::BankCostFirstGoal | LiveCase::BankCostSeersMaple | LiveCase::Site
    ) && (!plan.seed_locs.is_empty() || !plan.oak_tiles.is_empty() || task.is_some())
    {
        return Err(
            "real-content bank fixtures must use live scene resources without location seeds"
                .into(),
        );
    }
    Ok((target, plan, task))
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
        cache_dir: std::env::var_os("BOT_CACHE_DIR").map(PathBuf::from),
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
    if case == LiveCase::Site && !cfg!(feature = "live-probe") {
        return Err(
            "Site live cells require host-play/live-probe for read-only Area-arrival evidence"
                .into(),
        );
    }
    if (case == LiveCase::RandomEvent || case.is_death_recovery())
        && std::env::var("BOT_LIVE_NAME_PREFIX").as_deref() != Ok("g4a")
    {
        return Err("G4a live cells require BOT_LIVE_NAME_PREFIX=g4a".into());
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
    let (target, mut plan, fixture_task) = fixture_plan(cell, case)?;
    let temp = TempRoot::new(cell_name)?;
    let (profile, template) = selected_profile(nav_pack, engine_dir, catalog_root, temp.path())?;
    if case == LiveCase::BankCost {
        configure_bank_cost_plan(&template, &mut plan)?;
    }
    if matches!(case, LiveCase::CoinRunes | LiveCase::CoinRunesEmpty) {
        let reserve_name =
            std::env::var("GATHERER_COIN_RESERVE_TELEPORT").unwrap_or_else(|_| "Trollheim".into());
        let data = profile
            .game_data()
            .ok_or("selected 289 teleport facts unavailable")?;
        let teleport = data
            .teleport(&reserve_name)
            .filter(|teleport| teleport.available())
            .ok_or_else(|| format!("selected cache has no available teleport {reserve_name:?}"))?;
        if teleport.runes.is_empty() {
            return Err(format!(
                "selected teleport {reserve_name:?} has no rune requirements"
            ));
        }
        for rune in &teleport.runes {
            if rune.alias.is_empty() || rune.count <= 0 {
                return Err(format!(
                    "selected teleport {reserve_name:?} has invalid rune fact {:?}",
                    rune
                ));
            }
            if case == LiveCase::CoinRunes {
                let reserve_count = rune
                    .count
                    .checked_mul(10)
                    .ok_or("reserve rune seed count overflow")?;
                plan.bank_seed.push((rune.alias.clone(), reserve_count));
                plan.inventory_seed.push((rune.alias.clone(), rune.count));
                plan.reserve_runes.push((rune.id, rune.count));
            }
        }
    }
    let helper_needed = fixture_task.is_some() || case == LiveCase::RandomEvent;
    let names = host_play::mint_live_names(if helper_needed { 2 } else { 1 });
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
    let helper_state = if case == LiveCase::RandomEvent {
        Some(Arc::new(Mutex::new(GatherSlot::new(
            cell,
            case,
            target,
            plan.clone(),
            None,
            true,
        ))))
    } else {
        fixture_task.map(|task| {
            Arc::new(Mutex::new(GatherSlot::new(
                cell,
                case,
                target,
                plan.clone(),
                Some(task),
                true,
            )))
        })
    };
    let start_handle: Arc<Mutex<Option<ScriptStartHandle>>> = Arc::new(Mutex::new(None));
    let frame_state = Arc::clone(&state);
    let frame_handle = Arc::clone(&start_handle);
    let frame_account = account.clone();
    let frame_helper_account = helper_account.clone();
    let frame_helper_state = helper_state.clone();
    let frame_helper_needs_progress = case == LiveCase::GasHazard;
    let capture_request = Arc::new(Mutex::new(None::<(PathBuf, Value)>));
    let capture_result = Arc::new(Mutex::new(None::<Result<(), String>>));
    let frame_capture_request = Arc::clone(&capture_request);
    let frame_capture_result = Arc::clone(&capture_result);
    let mut play = host_play::run_with_template(
        Arc::clone(&template),
        true,
        vec![],
        |_| (None, None),
        move |client, username, frame| {
            let hold = frame.hold;
            if frame_helper_account.as_deref() == Some(username) {
                let progress = if frame_helper_needs_progress {
                    frame_state.lock().ok().map(|slot| slot.fixture_progress())
                } else {
                    None
                };
                if let Some(helper_state) = &frame_helper_state {
                    if let Ok(mut helper) = helper_state.lock() {
                        let spawn_foreign_event = frame_state.lock().ok().is_some_and(|main| {
                            main.case == LiveCase::RandomEvent
                                && main.witness.random_owned_released
                                && main.witness.random_fresh_yield
                                && !main.witness.random_foreign_command_sent
                        });
                        if !hold
                            && spawn_foreign_event
                            && helper.phase == Prep::Ready
                            && helper.snapshot.ingame()
                            && interact::cheat(client, "~macro_event 4").is_sent()
                        {
                            if let Ok(mut main) = frame_state.lock() {
                                main.witness.random_foreign_command_sent = true;
                                main.witness.random_foreign_yield_baseline =
                                    main.witness.last_status_yielded;
                                main.witness.random_foreign_xp_baseline = main.witness.last_xp;
                            }
                        }
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
            let (spawn_owned_event, trigger_recovery_death, death_command_tile) = frame_state
                .lock()
                .ok()
                .map(|slot| {
                    let initial_progress = slot.witness.last_status_yielded > 0
                        && slot.witness.last_xp > slot.baseline_xp();
                    let death_command_tile = slot.latest.as_ref().and_then(|latest| latest.tile);
                    let death_at_target = death_command_tile
                        .and_then(|tile| tile_distance(tile, slot.target))
                        .is_some_and(|distance| distance <= 12);
                    let trigger_recovery_death = !hold
                        && slot.case.is_death_recovery()
                        && death_at_target
                        && (slot.case != LiveCase::DeathReturn
                            || slot.witness.death_recovery_command_sent > 0
                            || slot.witness.pause_observed)
                        && if slot.case == LiveCase::DeathReturn
                            && slot.witness.death_recovery_command_sent == 1
                        {
                            slot.witness.death_haul_between
                        } else {
                            slot.witness.death_recovery_command_sent == 0 && initial_progress
                        };
                    (
                        !hold
                            && slot.case == LiveCase::RandomEvent
                            && initial_progress
                            && !slot.witness.random_owned_command_sent,
                        trigger_recovery_death,
                        death_command_tile,
                    )
                })
                .unwrap_or((false, false, None));
            if spawn_owned_event && interact::cheat(client, "~macro_event 4").is_sent() {
                if let Ok(mut slot) = frame_state.lock() {
                    slot.witness.random_owned_command_sent = true;
                }
            }
            // The content's `[debugproc,maze]`: the natural event's teleport,
            // reward and briefing, without waiting for the random-event RNG.
            let send_maze = frame_state.lock().ok().is_some_and(|slot| {
                !hold
                    && slot.case == LiveCase::MazeRandom
                    && slot.witness.last_status_yielded > 0
                    && slot.witness.last_xp > slot.baseline_xp()
                    && !slot.witness.maze_command_sent
            });
            if send_maze && interact::cheat(client, "~maze").is_sent() {
                if let Ok(mut slot) = frame_state.lock() {
                    slot.witness.maze_command_sent = true;
                }
                println!("{}", json!({"phase": "maze-command", "command": "~maze"}));
            }
            if trigger_recovery_death && interact::cheat(client, "~death").is_sent() {
                if let Ok(mut slot) = frame_state.lock() {
                    slot.witness.death_command_while_paused |= slot.case == LiveCase::DeathReturn
                        && slot.witness.death_recovery_command_sent == 0
                        && slot.witness.pause_observed;
                    slot.witness.death_recovery_command_sent =
                        slot.witness.death_recovery_command_sent.saturating_add(1);
                    slot.witness.death_command_tile = death_command_tile;
                }
            }
            let plane_teleport = frame_state.lock().ok().and_then(|slot| {
                (slot.case == LiveCase::PauseResumeOtherPlane
                    && slot.witness.pause_observed
                    && !slot.pause_plane_teleport_sent)
                    .then_some(slot.target)
            });
            if let Some(target) = plane_teleport {
                if send_cheat(
                    client,
                    &interact::tele_args(target.level.saturating_add(1), target.x, target.z),
                )
                .is_ok()
                {
                    if let Ok(mut slot) = frame_state.lock() {
                        slot.pause_plane_teleport_sent = true;
                    }
                }
            }
            if let Ok(mut slot) = frame_state.lock() {
                slot.frame(client, hold, None);
            }
            let capture = frame_capture_request
                .lock()
                .ok()
                .and_then(|mut request| request.take());
            if let Some((directory, receipt)) = capture {
                let result = save_live_capture(client, &directory, &receipt);
                if let Ok(mut completion) = frame_capture_result.lock() {
                    *completion = Some(result);
                }
            }
        },
    )?;
    if let Ok(mut handle) = start_handle.lock() {
        *handle = Some(play.script_start_handle());
    }
    let mut subject = mint_profile(&account, &password, 1)?;
    // `GATHERER_MAZE_RANDOM_EVENTS=off`: the operator's random-event toggle
    // is off, so no guardian solves the Maze and the Gatherer must block
    // inside it instead of staying frozen.
    if case == LiveCase::MazeRandom
        && std::env::var("GATHERER_MAZE_RANDOM_EVENTS").as_deref() == Ok("off")
    {
        subject.settings.random_events = false;
    }
    play.try_spawn_slot(subject, None, None, None)?;
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
    let mut identity = json!({
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
        "real_content_trip": (case == LiveCase::BankCostFirstGoal).then(|| json!({
            "resource": "oak",
            "camp": "Draynor Village east of the bank",
            "scene_locations_seeded": plan.seed_locs.len(),
            "fixture_locations_seeded": plan.oak_tiles.len(),
            "expected_bank": "Draynor",
            "expected_selection_kind": "Reachable",
            "five_second_timeout_expected": false,
        })),
        "bank_cost_fixture_intent": (case == LiveCase::BankCost).then(|| json!({
            "use_mage_bank": BANK_COST_FIXTURE_INTENT.use_mage_bank,
            "allow_wilderness": BANK_COST_FIXTURE_INTENT.allow_wilderness,
            "use_zanaris_bank": BANK_COST_FIXTURE_INTENT.use_zanaris_bank,
            "prerequisite": "selected nav facts expose eligible banks and explicitly disable Zanaris plus Fishing Guild",
        })),
        "bank_cost_oracle": (case == LiveCase::BankCost).then(|| json!({
            "eligible_air_candidates": plan.bank_cost_air_banks,
            "gate_disabled_candidates": plan.bank_cost_gated_candidates,
            "air_nearest_is_measured_at": "bank trip due using the observed live tile",
        })),
        "driver_trace": "enabled; native-packet account/tick/count lines are the packet witness",
    });
    if case == LiveCase::BankCostSeersMaple {
        identity["non_catalog_booth_trip"] = json!({
            "origin": target,
            "resource": "maple",
            "scene_locations_seeded": plan.seed_locs.len(),
            "oak_tiles_seeded": plan.oak_tiles.len(),
            "expected_bank": "Seers",
            "expected_selection_kind": "Reachable",
        });
    }
    println!("{identity}");
    let started_at = Instant::now();
    let mut restart_requested = false;
    let mut death_stop_requested = false;
    let mut prior_run = None;
    let mut reported = (0, 0, 0);
    let mut oak_all_stumps_prepared = false;
    let mut power_to_bank_edit_requested = false;
    let mut power_to_bank_edit_revision = None;
    let mut pause_requested = false;
    let mut resume_requested = false;
    let mut reconnect_logout_requested = false;
    let mut reconnect_login_armed = false;
    let result = loop {
        let lifecycle_error = play.script_last_error(&account);
        let (phase, start_requested, ready_to_start, slot_error, witness, baseline_xp) = {
            let slot = state.lock().map_err(|_| "live state poisoned")?;
            (
                slot.phase,
                slot.start_requested,
                slot.ready_to_start(),
                slot.error.clone(),
                slot.witness.clone(),
                slot.baseline_xp(),
            )
        };
        let (mut helper_done, helper_error) = if let Some(helper_state) = &helper_state {
            let helper = helper_state.lock().map_err(|_| "fixture state poisoned")?;
            (
                helper.fixture_done
                    && (case != LiveCase::RandomEvent || helper.phase == Prep::Ready),
                helper.error.clone(),
            )
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
            let bank_config = if case.is_bank_mode() {
                let preparation = match play.script_prepare_config(
                    script::CompiledId("Gatherer"),
                    1,
                    Arc::new(start_bag.clone()),
                ) {
                    Ok(preparation) => preparation,
                    Err(error) => break Err(format!("{cell_name} bank settings: {error}")),
                };
                match preparation.join() {
                    Ok(Ok(prepared)) => Some(prepared),
                    Ok(Err(error)) => break Err(format!("{cell_name} bank settings: {error}")),
                    Err(_) => break Err(format!("{cell_name} bank settings preparation panicked")),
                }
            } else {
                None
            };
            if let Err(error) =
                play.script_start(&account, script::CompiledId("Gatherer"), start_bag)
            {
                break Err(format!("{} Start failed: {error}", cell_name));
            }
            if let Some(bank_config) = bank_config {
                state.lock().map_err(|_| "live state poisoned")?.bank_config = Some(bank_config);
            }
            println!("{}", json!({"phase": "start", "cell": cell_name}));
        }
        let current_run = play.script_native_run(&account);
        if case == LiveCase::PowerToBank
            && !power_to_bank_edit_requested
            && witness.awaiting_drop
            && witness.cycle_product_slots > 0
            && witness.confirmed_drops > witness.cycle_confirmed_start
            && witness
                .confirmed_drops
                .saturating_sub(witness.cycle_confirmed_start)
                < witness.cycle_product_slots
        {
            let Some(run) = current_run else {
                break Err(format!(
                    "{cell_name} has a full batch but no native run to configure"
                ));
            };
            let Some(revision) = play.script_native_settings_revision(&account) else {
                break Err(format!("{cell_name} has no native settings revision"));
            };
            let Some(next_revision) = revision.checked_add(1) else {
                break Err(format!("{cell_name} native settings revision exhausted"));
            };
            let mut bank_settings = settings.clone();
            bank_settings.insert("disposition".into(), json!("Bank"));
            let preparation = match play.script_prepare_config(
                script::CompiledId("Gatherer"),
                next_revision,
                Arc::new(bank_settings),
            ) {
                Ok(preparation) => preparation,
                Err(error) => break Err(format!("{cell_name} Power to Bank preparation: {error}")),
            };
            let prepared = match preparation.join() {
                Ok(Ok(prepared)) => prepared,
                Ok(Err(error)) => {
                    break Err(format!("{cell_name} Power to Bank settings: {error}"))
                }
                Err(_) => break Err(format!("{cell_name} Power to Bank preparation panicked")),
            };
            let bank_config = Arc::clone(&prepared);
            let delivery = play.script_configure_compiled(&account, prepared, run);
            if !matches!(delivery, script::CompiledDelivery::PendingBoundary) {
                break Err(format!(
                    "{cell_name} Power to Bank update was not queued for a batch boundary: {delivery:?}"
                ));
            }
            let mut slot = state.lock().map_err(|_| "live state poisoned")?;
            slot.bank_config = Some(bank_config);
            slot.witness.power_to_bank_edit_requested = true;
            slot.witness.power_to_bank_edit_pending = true;
            slot.witness.power_to_bank_edit_drop_baseline = witness.cycle_confirmed_start;
            slot.witness.power_to_bank_edit_slots = witness.cycle_product_slots;
            power_to_bank_edit_revision = Some(next_revision);
            power_to_bank_edit_requested = true;
            let settings_status = play
                .script_native_status(&account)
                .map(|status| (status.active_settings, status.pending_settings));
            println!(
                "{}",
                json!({
                    "phase": "configure",
                    "cell": cell_name,
                    "live_case": case.name(),
                    "settings_revision": next_revision,
                    "batch_slots": witness.cycle_product_slots,
                    "settings_status": settings_status,
                })
            );
        }
        if let Some(revision) = power_to_bank_edit_revision {
            if play
                .script_native_status(&account)
                .is_some_and(|status| status.active_settings == revision)
            {
                let mut slot = state.lock().map_err(|_| "live state poisoned")?;
                let complete = slot
                    .witness
                    .confirmed_drops
                    .saturating_sub(slot.witness.power_to_bank_edit_drop_baseline)
                    >= slot.witness.power_to_bank_edit_slots;
                slot.witness.power_to_bank_applied = true;
                slot.witness.power_to_bank_edit_pending = false;
                if !complete {
                    slot.error = Some(
                        "Power to Bank settings activated before the active drop batch completed"
                            .into(),
                    );
                }
                power_to_bank_edit_revision = None;
            }
        }
        if case == LiveCase::DeathReturn
            && witness.last_status_yielded > 0
            && witness.last_xp > baseline_xp
            && witness.death_recovery_command_sent == 0
            && !pause_requested
        {
            let Some(run) = current_run else {
                break Err(format!(
                    "{cell_name} lost its run before death-watermark pause"
                ));
            };
            if !play.script_native_pause(&account, run, true) {
                break Err(format!(
                    "{cell_name} could not pause before death watermark gap"
                ));
            }
            pause_requested = true;
        }
        if case == LiveCase::DeathReturn
            && pause_requested
            && !witness.pause_observed
            && play.script_state(&account) == script::RunState::Paused
        {
            state
                .lock()
                .map_err(|_| "live state poisoned")?
                .witness
                .pause_observed = true;
        }
        if case == LiveCase::DeathReturn
            && pause_requested
            && witness.death_command_while_paused
            && witness.death_chat_observed_while_paused
            && !resume_requested
        {
            let Some(run) = current_run else {
                break Err(format!(
                    "{cell_name} lost its run before death-watermark resume"
                ));
            };
            if !play.script_native_pause(&account, run, false) {
                break Err(format!(
                    "{cell_name} could not resume after death watermark gap"
                ));
            }
            resume_requested = true;
        }
        if case == LiveCase::DeathReturn
            && resume_requested
            && !witness.resume_observed
            && play.script_state(&account) == script::RunState::Running
        {
            state
                .lock()
                .map_err(|_| "live state poisoned")?
                .witness
                .resume_observed = true;
        }
        if case == LiveCase::PauseResumeOtherPlane
            && witness.bank_return_step_seen
            && !pause_requested
        {
            if current_run.is_none() {
                break Err(format!("{cell_name} lost its run before pause"));
            }
            play.script_pause(&account);
            pause_requested = true;
        }
        if case == LiveCase::PauseResumeOtherPlane
            && pause_requested
            && play.script_state(&account) == script::RunState::Paused
        {
            let mut slot = state.lock().map_err(|_| "live state poisoned")?;
            slot.witness.pause_observed = true;
        }
        if case == LiveCase::PauseResumeOtherPlane
            && pause_requested
            && witness.other_plane_observed
            && !resume_requested
        {
            play.script_resume(&account);
            resume_requested = true;
        }
        if case == LiveCase::PauseResumeOtherPlane
            && resume_requested
            && play.script_state(&account) == script::RunState::Running
        {
            let mut slot = state.lock().map_err(|_| "live state poisoned")?;
            slot.witness.resume_observed = true;
        }
        if case == LiveCase::ReconnectReturn
            && witness.bank_return_step_seen
            && !reconnect_logout_requested
        {
            let Some(arm) = play.arm(&account) else {
                break Err(format!("{cell_name} has no reconnect slot arm"));
            };
            arm.request_logout();
            reconnect_logout_requested = true;
        }
        if case == LiveCase::ReconnectReturn
            && reconnect_logout_requested
            && witness.reconnect_offline_observed
            && !reconnect_login_armed
        {
            let Some(arm) = play.arm(&account) else {
                break Err(format!("{cell_name} lost its slot arm while offline"));
            };
            arm.arm_explicit_login();
            reconnect_login_armed = true;
        }
        if case == LiveCase::ReconnectReturn
            && reconnect_login_armed
            && witness.reconnect_offline_observed
            && current_run.is_some()
            && play.script_state(&account) == script::RunState::Running
        {
            let logged_in = state
                .lock()
                .map_err(|_| "live state poisoned")?
                .snapshot
                .ingame();
            if logged_in {
                state
                    .lock()
                    .map_err(|_| "live state poisoned")?
                    .witness
                    .reconnect_observed = true;
            }
        }
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
            #[cfg(feature = "live-probe")]
            if case == LiveCase::Site {
                let probe = play.manual_click_live_probe(&account);
                let nav = &probe["nav"];
                let returning =
                    text_field(&status, "bank").is_some_and(|bank| bank.ends_with("; Return"));
                let anchor = text_field(&status, "area").and_then(parse_work_anchor);
                if nav["armed"] == true {
                    slot.witness.site_initial_walk |= !returning
                        && slot.witness.status_trips == 0
                        && slot.witness.last_status_yielded == 0
                        && anchor.is_some_and(|tile| site_nav_targets_anchor(nav, tile));
                    if returning && anchor.is_some_and(|tile| site_return_nav_matches(nav, tile)) {
                        slot.witness.site_return_area_r1 = true;
                    }
                    if slot
                        .witness
                        .site_walks
                        .last()
                        .is_none_or(|prior| prior["nav"]["request_id"] != nav["request_id"])
                    {
                        slot.witness.site_walks.push(json!({"returning": returning, "nav": nav, "target": text_field(&status, "target")}));
                    }
                }
                if !slot.witness.site_anchor_selected {
                    if let Some(area) = anchor {
                        slot.witness.site_anchor_selected =
                            slot.bank_config.as_ref().is_some_and(|config| {
                                script::gatherer::test_site_anchor_selected(config, area)
                            });
                    }
                }
            }
            if case == LiveCase::DeathReturnRefused && slot.witness.death_return_refused {
                slot.witness.death_return_stopped |= play.script_state(&account)
                    == script::RunState::Idle
                    && play.script_native_run(&account).is_none()
                    && play
                        .script_lifecycle_receipt(&account)
                        .is_some_and(|receipt| {
                            receipt.state == script::ScriptTerminalState::Failed
                        });
            }
            if case == LiveCase::MazeRandom
                && slot.witness.maze_blocked_inside
                && !slot.witness.maze_stopped
                && play.script_state(&account) == script::RunState::Idle
                && play.script_native_run(&account).is_none()
                && play
                    .script_lifecycle_receipt(&account)
                    .is_some_and(|receipt| receipt.state == script::ScriptTerminalState::Failed)
            {
                slot.witness.maze_stopped = true;
                println!(
                    "{}",
                    json!({"phase": "maze-stopped", "state": "Idle", "receipt": "Failed"})
                );
            }
        }
        let (
            witness,
            latest,
            product_capacity,
            baseline_xp,
            bank_selection_complete,
            harpoon_in_inventory,
            harpoon_equipped,
        ) = {
            let slot = state.lock().map_err(|_| "live state poisoned")?;
            (
                slot.witness.clone(),
                slot.latest.clone(),
                (!case.is_fish_harpoon_bank()).then(|| slot.cycle_product_capacity()),
                slot.baseline_xp(),
                slot.complete_bank_selection(),
                slot.item_in_inventory(),
                slot.item_equipped(),
            )
        };
        #[cfg(feature = "live-probe")]
        if case.is_death_watchdog() {
            let retained_step = if case == LiveCase::DeathWatchdogPending {
                5
            } else {
                6
            };
            if witness.last_deaths == 1
                && witness.last_recovery_step == retained_step
                && !witness.death_watchdog_aged
            {
                let probe = play.manual_click_live_probe(&account);
                if probe["script"]["watchdog_state"].as_str() == Some("Armed") {
                    let Some(generation) = probe["script"]["runtime_generation"].as_u64() else {
                        break Err(format!("{cell_name} has no runtime generation to age"));
                    };
                    let Some((x, z, level)) = latest.as_ref().and_then(|latest| latest.tile) else {
                        break Err(format!("{cell_name} has no tile to age the watchdog at"));
                    };
                    let xp: Vec<i32> = state
                        .lock()
                        .map_err(|_| "live state poisoned")?
                        .snapshot
                        .stats()
                        .iter()
                        .map(|stat| stat.xp)
                        .collect();
                    if let Err(error) = play.manual_click_live_age_gameplay_with_xp(
                        &account,
                        WorldTile { x, z, level },
                        &xp,
                    ) {
                        break Err(format!("{cell_name} watchdog age failed: {error}"));
                    }
                    let mut slot = state.lock().map_err(|_| "live state poisoned")?;
                    slot.witness.death_watchdog_aged = true;
                    slot.witness.death_watchdog_generation_before = Some(generation);
                }
            }
            if witness.death_watchdog_aged {
                let probe = play.manual_click_live_probe(&account);
                if witness
                    .death_watchdog_generation_before
                    .zip(probe["script"]["runtime_generation"].as_u64())
                    .is_some_and(|(before, after)| {
                        after > before
                            && witness.last_deaths == 1
                            && witness.last_recovery_step == retained_step
                    })
                {
                    state
                        .lock()
                        .map_err(|_| "live state poisoned")?
                        .witness
                        .death_watchdog_recreated = true;
                }
            }
        }
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
            LiveCase::Power => power_complete(cell, &witness),
            LiveCase::FishNet => {
                witness.cycles >= REQUIRED_CYCLES
                    && witness.post_drop_gathers >= REQUIRED_POST_DROP_GATHERS
                    && witness.moved_spot_reacquired
                    && witness.fish_targets.len() >= 2
            }
            LiveCase::FishBaitGate | LiveCase::OakAbsentArea => witness.failure_code.is_some(),
            LiveCase::MazeRandom => maze_random_complete(&witness),
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
            LiveCase::RandomEvent
            | LiveCase::DeathReturn
            | LiveCase::DeathRespawnRegion
            | LiveCase::DeathNoStock
            | LiveCase::DeathReturnRefused
            | LiveCase::DeathWatchdogPending
            | LiveCase::DeathWatchdogProving => interrupts_complete(case, &witness),
            LiveCase::CancelBeforeDrain => witness.stopped_before_drain,
            LiveCase::DeathDuringDrop => witness.death_restart_gathered,
            LiveCase::RunKeyChangeDuringDrop => {
                witness.run_key_changed && witness.confirmed_drops >= 28
            }
            LiveCase::WoodcuttingBank | LiveCase::WoodcuttingBankUnwieldable => {
                witness.status_trips >= 3
                    && witness.bank_nonzero_roundtrips >= 2
                    && witness.post_bank_yields >= 3
                    && witness.last_status_yielded > 0
                    && witness.bank_closed_observed
            }
            LiveCase::BankCost => witness.bank_loaded_observed,
            LiveCase::Site => {
                site_bank_complete(cell, &witness, &plan, baseline_xp)
                    && bank_selection_complete
                    && latest
                        .as_ref()
                        .is_some_and(|latest| latest.product_count > 0 && latest.xp > baseline_xp)
                    && (cell != Cell::Fishing || (harpoon_in_inventory && !harpoon_equipped))
            }
            LiveCase::BankCostFirstGoal | LiveCase::BankCostSeersMaple => {
                real_content_bank_lifecycle_complete(case, &witness, &plan, target, baseline_xp)
            }
            LiveCase::BankCostNoCandidate => {
                witness.failure_code.as_deref() == Some("bank-unavailable")
            }
            LiveCase::FishBait => {
                witness.status_trips >= 1
                    && witness.bait_exact
                    && witness.post_bank_yields >= 1
                    && witness.last_status_yielded > 0
            }
            LiveCase::FishHarpoonBank | LiveCase::FishHarpoonBankAuto => {
                witness.fish_bank_complete(case.requires_seeded_casket())
                    && bank_selection_complete
                    && witness.bank_loaded_observed
                    && witness.bank_closed_observed
                    && harpoon_in_inventory
                    && !harpoon_equipped
                    && latest
                        .as_ref()
                        .is_some_and(|latest| latest.product_count > 0 && latest.xp > baseline_xp)
            }
            LiveCase::CoinRunes => {
                witness.status_trips >= 1
                    && witness.bank_withdrawal_confirmed
                    && witness.coin_balance_exact
                    && witness.reserve_runes_intact
                    && witness.post_bank_yields >= 1
                    && witness.last_status_yielded > 0
            }
            LiveCase::CoinRunesEmpty => {
                witness.failure_code.as_deref() == Some("supply-missing")
                    && witness
                        .last_status_bank
                        .as_deref()
                        .is_some_and(|bank| bank.ends_with("; Withdraw"))
            }
            LiveCase::PowerToBank => {
                witness.power_to_bank_applied
                    && witness.status_trips >= 1
                    && witness.bank_nonzero_roundtrips >= 1
                    && witness.post_bank_yields >= 1
                    && witness.last_status_yielded > 0
            }
            LiveCase::PauseResumeOtherPlane => {
                witness.pause_observed
                    && witness.other_plane_observed
                    && witness.resume_observed
                    && witness.bank_return_after_pause
                    && witness.post_bank_yields >= 1
                    && latest.as_ref().is_some_and(|latest| {
                        latest.tile.is_some_and(|tile| tile.2 == target.level)
                            && latest.xp > baseline_xp
                    })
            }
            LiveCase::ReconnectReturn => {
                witness.reconnect_offline_observed
                    && witness.reconnect_observed
                    && witness.bank_return_after_reconnect
                    && witness.post_bank_yields >= 1
                    && latest
                        .as_ref()
                        .is_some_and(|latest| latest.xp > baseline_xp)
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
    let (witness, product_capacity, fixture_plan) = {
        let slot = state.lock().map_err(|_| "live state poisoned")?;
        (
            slot.witness.clone(),
            (!case.is_fish_harpoon_bank()).then(|| slot.cycle_product_capacity()),
            slot.plan.clone(),
        )
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
    let g4a_receipt = json!({
            "random_owned_event_seen": witness.random_owned_event_seen,
            "random_owned_hold_frames": witness.random_owned_hold_frames,
            "random_hold_unchanged": witness.random_hold_unchanged,
            "random_owned_released": witness.random_owned_released,
            "random_fresh_yield": witness.random_fresh_yield,
            "random_owned_revalidated": witness.random_owned_revalidated,
            "random_foreign_event_seen": witness.random_foreign_event_seen,
            "random_foreign_not_held": witness.random_foreign_not_held,
            "random_foreign_fresh_yield": witness.random_foreign_fresh_yield,
            "death_recovery_command_sent": witness.death_recovery_command_sent,
            "death_command_while_paused": witness.death_command_while_paused,
            "death_chat_observed_while_paused": witness.death_chat_observed_while_paused,
            "pause_observed": witness.pause_observed,
            "resume_observed": witness.resume_observed,
            "death_recovery_in_area": witness.death_recovery_in_area,
            "death_command_tile": witness.death_command_tile,
            "death_respawn_region_unchanged": witness.death_respawn_region_unchanged,
            "death_progress_baselines": witness.death_progress_baselines,
            "deaths": witness.last_deaths,
            "recoveries": witness.last_recoveries,
            "recovery_step": witness.last_recovery_step,
            "death_recovered": witness.death_recovered,
            "death_products_after": witness.death_products_after,
            "death_xp_after": witness.death_xp_after,
            "death_haul_between": witness.death_haul_between,
            "death_return_refused": witness.death_return_refused,
            "death_return_stopped": witness.death_return_stopped,
            "death_watchdog_aged": witness.death_watchdog_aged,
            "death_watchdog_generation_before": witness.death_watchdog_generation_before,
            "death_watchdog_recreated": witness.death_watchdog_recreated,
    });
    receipt["interrupts"] = g4a_receipt;
    let mut bank_receipt = json!({
            "bank": witness.last_status_bank,
            "bank_trips": witness.status_trips,
            "deposited": witness.status_deposited,
            "nonzero_deposit_returns": witness.bank_nonzero_roundtrips,
            "fish_bank_complete": witness.fish_bank_complete(case.requires_seeded_casket()),
            "bank_trip_receipts": witness
                .bank_trips
                .iter()
                .map(BankTripReceipt::as_json)
                .collect::<Vec<_>>(),
            "pending_bank_trip": witness
                .pending_bank_trip
                .as_ref()
                .map(BankTripReceipt::as_json),
            "post_bank_yields": witness.post_bank_yields,
            "bank_loaded": witness.bank_loaded_observed,
            "bank_closed": witness.bank_closed_observed,
            "tool_selected": witness.tool_selected_observed,
            "tool_worn": witness.tool_worn_observed,
            "bait_exact": witness.bait_exact,
            "coin_balance_exact": witness.coin_balance_exact,
            "reserve_runes_intact": witness.reserve_runes_intact,
            "pending_boundary": witness.power_to_bank_edit_pending,
            "power_to_bank_applied": witness.power_to_bank_applied,
            "other_plane_observed": witness.other_plane_observed,
            "return_after_pause": witness.bank_return_after_pause,
            "reconnect_offline": witness.reconnect_offline_observed,
            "return_after_reconnect": witness.bank_return_after_reconnect,
            "failure_code": witness.failure_code,
            "failure_message": witness.failure_message,
            "first_goal_trip": (case == LiveCase::BankCostFirstGoal).then(|| json!({
                "expected_bank": "Draynor",
                "expected_selection_kind": "Reachable",
                "five_second_timeout_expected": false,
                "selection_elapsed_ms": witness.bank_selection_elapsed.map(|elapsed| elapsed.as_millis()),
            })),
    });
    if case == LiveCase::BankCostSeersMaple {
        bank_receipt["non_catalog_booth_trip"] = json!({
            "origin": target,
            "resource": "maple",
            "expected_bank": "Seers",
            "expected_selection_kind": "Reachable",
            "selection_elapsed_ms": witness.bank_selection_elapsed.map(|elapsed| elapsed.as_millis()),
            "bank_arrivals": witness.bank_arrivals,
            "return_step_seen": witness.bank_return_step_seen,
        });
    }
    receipt["bank_trip"] = bank_receipt;
    receipt["bank_status_sequence"] = json!(witness.bank_status_sequence);
    receipt["failure_sequence"] = json!(witness.failure_sequence);
    receipt["final_tick"] = json!(state
        .lock()
        .map_err(|_| "live state poisoned")?
        .latest
        .as_ref()
        .map(|row| row.tick));
    {
        let slot = state.lock().map_err(|_| "live state poisoned")?;
        receipt["final_inventory"] = json!(slot
            .snapshot
            .inventory()
            .iter()
            .map(|item| json!({"id": item.def.id, "slot": item.slot, "count": item.count}))
            .collect::<Vec<_>>());
        receipt["final_equipment"] = json!(slot
            .snapshot
            .equipment()
            .iter()
            .map(|item| json!({"id": item.def.id, "slot": item.slot, "count": item.count}))
            .collect::<Vec<_>>());
    }
    let mut fixture_receipt = json!({
            "bank_selection_elapsed_ms": witness
                .bank_selection_elapsed
                .map(|elapsed| elapsed.as_millis()),
            "bank_cost_oracle": (case == LiveCase::BankCost).then(|| json!({
                "origin": fixture_plan.bank_cost_origin,
                "air_nearest": fixture_plan.bank_cost_air_nearest,
                "gate_disabled_candidates": fixture_plan.bank_cost_gated_candidates,
            })),
            "real_content_trip": (case == LiveCase::BankCostFirstGoal).then(|| json!({
                "origin": target,
                "resource": "oak",
                "nearby_live_oak_tiles": witness
                    .oak_live_tiles
                    .iter()
                    .filter(|&&tile| tile_distance(tile, target).is_some_and(|distance| distance <= 12))
                    .copied()
                    .collect::<Vec<_>>(),
                "scene_locations_seeded": fixture_plan.seed_locs.len(),
                "fixture_locations_seeded": fixture_plan.oak_tiles.len(),
            })),
            "fixture_chop_observed": witness.fixture_chop_observed,
            "seeded_casket": {
                "required": case.requires_seeded_casket(),
                "alias": "casket",
                "id": CASKET_ID,
                "start_baseline_count": witness.seeded_casket_baseline_count,
                "baseline_verified": witness.seeded_casket_baseline_verified,
                "banked_on_first_trip": witness
                    .bank_trips
                    .first()
                    .is_some_and(|trip| trip.seeded_casket_banked),
                "natural_first_observed_after_seeded_bank": witness.natural_casket_first_observed,
            },
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
            "seeded_mining_gem": (cell == Cell::Mining && case == LiveCase::Power).then(|| json!({
                "id": SEEDED_MINING_GEM_ID,
                "dropped": witness.dropped_product_ids.contains(&SEEDED_MINING_GEM_ID),
            })),
            "natural_gems": {
                "status": if witness.natural_gems_seen.is_empty() { "not_exercised" } else { "observed" },
                "ids": witness.natural_gems_seen,
                "dropped_ids": witness.natural_gems_seen.intersection(&witness.dropped_product_ids).copied().collect::<Vec<_>>(),
            },
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
    if case == LiveCase::BankCostSeersMaple {
        fixture_receipt["non_catalog_booth_trip"] = json!({
            "origin": target,
            "resource": "maple",
            "resource_product_id": MAPLE_LOG_ID,
            "resource_product_seen": witness.products_seen.contains(&MAPLE_LOG_ID),
            "expected_bank": "Seers",
            "expected_selection_kind": "Reachable",
            "scene_locations_seeded": fixture_plan.seed_locs.len(),
            "oak_tiles_seeded": fixture_plan.oak_tiles.len(),
            "bank_arrivals": witness.bank_arrivals,
            "return_step_seen": witness.bank_return_step_seen,
            "bank_trips": witness.status_trips,
            "bank_loaded": witness.bank_loaded_observed,
            "bank_closed": witness.bank_closed_observed,
            "deposited": witness.status_deposited,
            "nonzero_deposit_returns": witness.bank_nonzero_roundtrips,
            "post_bank_yields": witness.post_bank_yields,
            "yielded": witness.last_status_yielded,
            "xp": witness.last_xp,
        });
    }
    if case == LiveCase::Site {
        receipt["named_site"] = json!({
            "id": cell.site_id(),
            "expected_bank": cell.site_bank(),
            "selected_resource_anchor": witness.site_anchor_selected,
            "initial_site_walk": witness.site_initial_walk,
            "return_area_r1": witness.site_return_area_r1,
            "walks": witness.site_walks,
            "area": witness.last_area,
            "scene_seeds": fixture_plan.seed_locs.len(),
        });
    }
    receipt.as_object_mut().expect("receipt object").append(
        fixture_receipt
            .as_object_mut()
            .expect("fixture receipt object"),
    );
    println!("{receipt}");
    if let Some(root) = std::env::var_os("LIVE_EVIDENCE_DIR") {
        let epoch = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| error.to_string())?
            .as_secs();
        let directory = PathBuf::from(root).join(format!("{cell_name}_{account}_{epoch}Z"));
        *capture_request
            .lock()
            .map_err(|_| "capture request poisoned")? = Some((directory.clone(), receipt));
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            if let Some(completion) = capture_result
                .lock()
                .map_err(|_| "capture result poisoned")?
                .take()
            {
                completion?;
                println!("live-evidence={}", directory.display());
                break;
            }
            if Instant::now() >= deadline {
                return Err("final live evidence capture timed out".into());
            }
            std::thread::sleep(POLL_INTERVAL);
        }
    }
    play.script_stop(&account);
    play.stop_slot(&account);
    if let Some(helper_account) = &helper_account {
        play.stop_slot(helper_account);
    }
    result?;
    let qualification = state.lock().map_err(|_| "live state poisoned")?.qualifies();
    qualification
}

#[cfg(feature = "live-harness")]
struct QuesterSharedCoreState {
    runner: ScenarioRunner,
    account: String,
    snapshot: GameSnapshot,
    pump: Pump,
    start_handle: Option<ScriptStartHandle>,
    start_settings: Map<String, Value>,
    start_count: u32,
    error: Option<String>,
}

#[cfg(feature = "live-harness")]
impl QuesterSharedCoreState {
    fn frame(&mut self, client: &mut client::client::Client, hold: bool) {
        let drain = self.pump.drain_client(client);
        host::publish_snapshot(&mut self.snapshot, client, drain);
        if self.runner.on_start_script() && self.start_count == 0 {
            let Some(handle) = self.start_handle.as_ref() else {
                self.error = Some("Quester Start reached before ScriptStartHandle install".into());
                return;
            };
            match handle.start_compiled(
                &self.account,
                script::CompiledId("Quester"),
                self.start_settings.clone(),
            ) {
                Ok(()) => self.start_count = 1,
                Err(error) => {
                    self.error = Some(format!("Quester compiled Start failed: {error}"));
                    return;
                }
            }
        }
        if !matches!(
            self.runner.status(),
            RunnerStatus::Passed | RunnerStatus::Failed(_)
        ) {
            self.runner.tick_with_hold(client, hold);
        }
    }
}

#[cfg(feature = "live-harness")]
fn run_quester_sheep_shared_core() -> Result<(), String> {
    if std::env::var("LIVE").as_deref() != Ok("1") {
        return Ok(());
    }
    const CELL: &str = "g3_quester_sheep_bank_shared_core";
    let _home = script::IsolatedEnv::enter(CELL);
    api::hostlog::set_debug(true);
    let nav_pack = PathBuf::from(required("GATHERER_NAV_PACK")?);
    let engine_dir = PathBuf::from(required("GATHERER_ENGINE_DIR")?);
    let catalog_root = PathBuf::from(required("GATHERER_CATALOG_ROOT")?);
    for (name, path, is_file) in [
        ("GATHERER_NAV_PACK", &nav_pack, true),
        ("GATHERER_ENGINE_DIR", &engine_dir, false),
        ("GATHERER_CATALOG_ROOT", &catalog_root, false),
    ] {
        if !path.is_absolute() {
            return Err(format!(
                "{name} must be an absolute path: {}",
                path.display()
            ));
        }
        if is_file && !path.is_file() {
            return Err(format!("{name} is not a file: {}", path.display()));
        }
        if !is_file && !path.is_dir() {
            return Err(format!("{name} is not a directory: {}", path.display()));
        }
    }
    let mut scenario =
        scenario::get("quester_sheep").ok_or("scenario registry has no quester_sheep")?;
    if scenario.settings.start_script != Some("Quester") {
        return Err(format!(
            "quester_sheep starts {:?}, not Quester",
            scenario.settings.start_script
        ));
    }
    let start_settings = scenario::settings_inject_map(scenario.settings.script_settings_inject)
        .ok_or("quester_sheep has no compiled Start settings")?;
    if start_settings.len() != 1 || start_settings.get("quests") != Some(&json!(["sheep"])) {
        return Err(format!(
            "quester_sheep must Start Quester with exactly {{\"quests\":[\"sheep\"]}}, got {start_settings:?}"
        ));
    }
    let deadline = scenario.settings.deadline;
    let mainland = scenario.seed.mainland;
    scenario.settings.nav.engine_speed_ms = None;
    let temp = TempRoot::new(CELL)?;
    let (profile, template) = selected_profile(nav_pack, engine_dir, catalog_root, temp.path())?;
    let names = host_play::mint_live_names(1);
    let account = names
        .first()
        .cloned()
        .ok_or("failed to mint Quester live account")?;
    let password = host_play::mint_live_entries(&names)
        .first()
        .map(|(_, password)| password.clone())
        .ok_or("failed to mint Quester live credential")?;
    let mut runner = ScenarioRunner::with_world(scenario, template.world());
    runner.set_map_members(profile.map_members());
    runner.set_live_names(&names);
    runner.set_shot_sink(Box::new(|_, _| {}));
    let state = Arc::new(Mutex::new(QuesterSharedCoreState {
        runner,
        account: account.clone(),
        snapshot: GameSnapshot::new(),
        pump: Pump::new(),
        start_handle: None,
        start_settings: start_settings.clone(),
        start_count: 0,
        error: None,
    }));
    let frame_state = Arc::clone(&state);
    let frame_account = account.clone();
    let mut play = host_play::run_with_template(
        Arc::clone(&template),
        mainland,
        vec![],
        |_| (None, None),
        move |client, username, hold| {
            if username == frame_account {
                frame_state.lock().unwrap().frame(client, hold.hold);
            }
        },
    )?;
    {
        let mut slot = state.lock().map_err(|_| "Quester live state poisoned")?;
        slot.runner.set_obj_names(play.obj_names());
        slot.start_handle = Some(play.script_start_handle());
    }
    play.try_spawn_slot(mint_profile(&account, &password, 1)?, None, None, None)?;
    play.focus(&account);
    println!(
        "{}",
        json!({
            "phase": "identity",
            "live_case": CELL,
            "profile": profile.label(),
            "game_port": profile.client().game_port(),
            "nav_pack": profile.nav_pack(),
            "account": account,
            "card": "Quester",
            "settings": start_settings,
            "scenario": "quester_sheep",
        })
    );
    let outer_deadline = Instant::now() + deadline + Duration::from_secs(15);
    let result = loop {
        let script_error = play.script_last_error(&account);
        let native_status = play.script_native_status(&account);
        let run_state = play.script_state(&account);
        let (runner_status, start_count, error) = {
            let mut slot = state.lock().map_err(|_| "Quester live state poisoned")?;
            if run_state == script::RunState::Running {
                slot.runner.observe_script_running();
            }
            (slot.runner.status(), slot.start_count, slot.error.clone())
        };
        if let Some(error) = error {
            break Err(error);
        }
        if let Some(error) = script_error {
            break Err(format!("Quester lifecycle error: {error}"));
        }
        match runner_status {
            RunnerStatus::Failed(error) => {
                break Err(format!("quester_sheep scenario failed: {error}"));
            }
            RunnerStatus::Passed => {
                if start_count != 1 {
                    break Err(format!(
                        "quester_sheep passed after {start_count} compiled Starts, expected one"
                    ));
                }
                if let Some(status) = native_status.as_ref() {
                    if status.card != script::CompiledId("Quester") {
                        break Err(format!(
                            "quester_sheep terminal status belongs to {:?}",
                            status.card
                        ));
                    }
                    if status.phase == script::native::NativePhase::Complete {
                        println!(
                            "{}",
                            json!({
                                "phase": "witness",
                                "live_case": CELL,
                                "scenario": "quester_sheep",
                                "card": status.card.0,
                                "native_phase": format!("{:?}", status.phase),
                                "start_count": start_count,
                            })
                        );
                        break Ok(());
                    }
                }
            }
            RunnerStatus::Seeding | RunnerStatus::Running { .. } => {}
        }
        if Instant::now() >= outer_deadline {
            let tile = state
                .lock()
                .map_err(|_| "Quester live state poisoned")?
                .snapshot
                .tile();
            break Err(format!(
                "quester_sheep timed out: runner={runner_status:?} native_status={native_status:?} tile={tile:?}"
            ));
        }
        std::thread::sleep(POLL_INTERVAL);
    };
    play.stop_slot(&account);
    result
}

#[cfg(feature = "live-harness")]
#[test]
#[ignore = "requires LIVE=1, Gatherer live engine/nav/catalog settings and local 289 engine"]
fn g3_quester_sheep_bank_shared_core() {
    run_quester_sheep_shared_core().unwrap();
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

#[test]
#[ignore = "requires LIVE=1 and local 289 engine; defaults to the validated oak fixture"]
fn gatherer_wc_bank_trip_lifecycle() {
    run_cell(Cell::Woodcutting, LiveCase::WoodcuttingBank).unwrap();
}

#[test]
#[ignore = "requires LIVE=1 and local 289 engine; defaults to the validated oak fixture"]
fn gatherer_wc_bank_trip_unwieldable_tool() {
    run_cell(Cell::Woodcutting, LiveCase::WoodcuttingBankUnwieldable).unwrap();
}

#[test]
#[ignore = "requires LIVE=1, GATHERER_NAV_PACK/GATHERER_ENGINE_DIR/GATHERER_CATALOG_ROOT and a local 289 engine"]
fn gatherer_mining_bank_cost_walk_ranked() {
    run_cell(Cell::Mining, LiveCase::BankCost).unwrap();
}

#[test]
#[ignore = "requires LIVE=1, GATHERER_NAV_PACK/GATHERER_ENGINE_DIR/GATHERER_CATALOG_ROOT, and local 289 engine; uses Draynor's actual oak grove and nearby bank"]
fn gatherer_bank_cost_air_fallback() {
    run_cell(Cell::Woodcutting, LiveCase::BankCostFirstGoal).unwrap();
}

#[test]
#[ignore = "requires LIVE=1, BOT_LIVE_NAME_PREFIX, GATHERER_NAV_PACK, GATHERER_ENGINE_DIR, GATHERER_CATALOG_ROOT and a local 289 engine; gathers real Seers maples and requires Seers"]
fn gatherer_bank_non_catalog_seers_maples_lifecycle() {
    run_cell(Cell::Woodcutting, LiveCase::BankCostSeersMaple).unwrap();
}

#[test]
#[ignore = "requires LIVE=1 and local 289 engine; defaults to the validated oak fixture"]
fn gatherer_bank_cost_no_candidate() {
    run_cell(Cell::Woodcutting, LiveCase::BankCostNoCandidate).unwrap();
}

#[test]
#[ignore = "requires LIVE=1 and local 289 engine; defaults to the validated fish fixture"]
fn gatherer_fish_bait_bank() {
    run_cell(Cell::Fishing, LiveCase::FishBait).unwrap();
}

#[test]
#[ignore = "requires LIVE=1 and local 289 engine; defaults to Catherby with a harpoon"]
fn gatherer_fish_harpoon_bank() {
    run_cell(Cell::Fishing, LiveCase::FishHarpoonBank).unwrap();
}

#[test]
#[ignore = "requires LIVE=1 and local 289 engine"]
fn gatherer_fish_harpoon_bank_auto() {
    run_cell(Cell::Fishing, LiveCase::FishHarpoonBankAuto).unwrap();
}

#[test]
#[ignore = "requires LIVE=1, live-probe and local 289 engine"]
fn gatherer_fish_harpoon_bank_site() {
    run_cell(Cell::Fishing, LiveCase::Site).unwrap();
}

#[test]
#[ignore = "requires LIVE=1, live-probe and local 289 engine"]
fn gatherer_wc_willow_site() {
    run_cell(Cell::Woodcutting, LiveCase::Site).unwrap();
}

#[test]
#[ignore = "requires LIVE=1, live-probe and local 289 engine"]
fn gatherer_mine_copper_tin_site() {
    run_cell(Cell::Mining, LiveCase::Site).unwrap();
}

#[test]
#[ignore = "requires LIVE=1 and local 289 engine; defaults to the validated oak fixture"]
fn gatherer_coin_runes_bank() {
    run_cell(Cell::Woodcutting, LiveCase::CoinRunes).unwrap();
}

#[test]
#[ignore = "requires LIVE=1 and local 289 engine; defaults to the validated oak fixture"]
fn gatherer_coin_runes_empty_stock() {
    run_cell(Cell::Woodcutting, LiveCase::CoinRunesEmpty).unwrap();
}

#[test]
#[ignore = "requires LIVE=1 and local 289 engine; defaults to the validated oak fixture"]
fn gatherer_power_to_bank_boundary() {
    run_cell(Cell::Woodcutting, LiveCase::PowerToBank).unwrap();
}

#[test]
#[ignore = "requires LIVE=1 and local 289 engine; defaults to the validated oak fixture"]
fn gatherer_bank_pause_resume_other_plane() {
    run_cell(Cell::Woodcutting, LiveCase::PauseResumeOtherPlane).unwrap();
}

#[test]
#[ignore = "requires LIVE=1 and local 289 engine; defaults to the validated oak fixture"]
fn gatherer_bank_reconnect_return() {
    run_cell(Cell::Woodcutting, LiveCase::ReconnectReturn).unwrap();
}
#[test]
#[ignore = "requires LIVE=1, BOT_LIVE_NAME_PREFIX=g4a, GATHERER_WC_TILE, and local 289 engine"]
fn gatherer_random() {
    run_cell(Cell::Woodcutting, LiveCase::RandomEvent).unwrap();
}

#[test]
#[ignore = "requires LIVE=1, GATHERER_WC_TILE, and local 289 engine"]
fn gatherer_maze_random() {
    run_cell(Cell::Woodcutting, LiveCase::MazeRandom).unwrap();
}

/// The Maze teleport was observed, and the Gatherer either resumed
/// gathering after the host's solver walked it out, or blocked while
/// still trapped inside and took the terminal Stop — never a held
/// `Working` run inside the Maze.
fn maze_random_complete(witness: &Witness) -> bool {
    witness.maze_command_sent
        && witness.maze_entered
        && ((witness.maze_exited && witness.maze_resumed)
            || (witness.maze_blocked_inside && witness.maze_stopped))
}

#[test]
#[ignore = "requires LIVE=1, BOT_LIVE_NAME_PREFIX=g4a, GATHERER_WC_BANK_TILE, and local 289 engine"]
fn gatherer_death_return() {
    run_cell(Cell::Woodcutting, LiveCase::DeathReturn).unwrap();
}

#[test]
#[ignore = "requires LIVE=1, BOT_LIVE_NAME_PREFIX=g4a, and local 289 engine; defaults to the Lumbridge respawn-region fixture"]
fn gatherer_death_return_respawn_region() {
    run_cell(Cell::Woodcutting, LiveCase::DeathRespawnRegion).unwrap();
}

#[test]
#[ignore = "requires LIVE=1, BOT_LIVE_NAME_PREFIX=g4a, and local 289 engine"]
fn gatherer_death_return_no_stock() {
    run_cell(Cell::Woodcutting, LiveCase::DeathNoStock).unwrap();
}

#[test]
#[ignore = "requires LIVE=1, BOT_LIVE_NAME_PREFIX=g4a, GATHERER_DEATH_RETURN_REFUSE_TILE, and local 289 engine"]
fn gatherer_death_return_refused_stops() {
    run_cell(Cell::Woodcutting, LiveCase::DeathReturnRefused).unwrap();
}

#[cfg(feature = "live-probe")]
#[test]
#[ignore = "requires LIVE=1, BOT_LIVE_NAME_PREFIX=g4a, GATHERER_WC_BANK_TILE, live-probe, and local 289 engine"]
fn gatherer_death_watchdog_pending5() {
    run_cell(Cell::Woodcutting, LiveCase::DeathWatchdogPending).unwrap();
}

#[cfg(feature = "live-probe")]
#[test]
#[ignore = "requires LIVE=1, BOT_LIVE_NAME_PREFIX=g4a, GATHERER_WC_BANK_TILE, live-probe, and local 289 engine"]
fn gatherer_death_watchdog_proving() {
    run_cell(Cell::Woodcutting, LiveCase::DeathWatchdogProving).unwrap();
}

fn interrupts_complete(case: LiveCase, witness: &Witness) -> bool {
    match case {
        LiveCase::RandomEvent => {
            witness.random_owned_event_seen
                && witness.random_owned_hold_frames > 0
                && witness.random_hold_unchanged
                && witness.random_owned_released
                && witness.random_fresh_yield
                && witness.random_owned_revalidated
                && witness.random_foreign_event_seen
                && witness.random_foreign_not_held
                && witness.random_foreign_fresh_yield
        }
        LiveCase::DeathReturn => {
            witness.death_recovered.contains(&1)
                && witness.death_recovered.contains(&2)
                && witness.death_products_after.contains(&1)
                && witness.death_products_after.contains(&2)
                && witness.death_xp_after.contains(&1)
                && witness.death_xp_after.contains(&2)
                && witness.death_haul_between
                && witness.death_recovery_command_sent >= 2
                && witness.death_command_while_paused
                && witness.death_chat_observed_while_paused
                && witness.pause_observed
                && witness.resume_observed
                && witness.last_deaths == 2
                && witness.last_recoveries == 2
        }
        LiveCase::DeathRespawnRegion => {
            witness.death_recovery_in_area
                && witness.death_respawn_region_unchanged
                && witness.death_recovered.contains(&1)
                && witness.death_products_after.contains(&1)
                && witness.death_xp_after.contains(&1)
                && witness.last_deaths == 1
                && witness.last_recoveries == 1
        }
        LiveCase::DeathNoStock => {
            witness.failure_code.as_deref() == Some("supply-missing")
                && witness.last_deaths == 1
                && witness.death_recovery_command_sent == 1
                && witness
                    .last_status_bank
                    .as_deref()
                    .is_some_and(|bank| bank.ends_with("; Withdraw"))
        }
        LiveCase::DeathReturnRefused => {
            witness.death_return_refused
                && witness.death_return_stopped
                && witness.last_deaths == 1
                && witness.death_recovery_command_sent == 1
        }
        LiveCase::DeathWatchdogPending | LiveCase::DeathWatchdogProving => {
            witness.death_watchdog_aged
                && witness.death_watchdog_generation_before.is_some()
                && witness.death_watchdog_recreated
                && witness.death_recovery_command_sent == 1
                && witness.death_recovered.contains(&1)
                && witness.last_deaths == 1
                && witness.last_recoveries == 1
                && witness.death_products_after.contains(&1)
                && witness.death_xp_after.contains(&1)
        }
        _ => false,
    }
}

#[cfg(test)]
mod bank_conservation_tests {
    use super::*;

    const HARPOON_ID: i32 = 311;
    const KEBAB_ID: i32 = 1971;

    fn counts(items: &[(i32, i32)]) -> BTreeMap<i32, i32> {
        aggregate_item_counts(items.iter().copied())
    }

    fn bank_fixture(items: &[(i32, i32)]) -> (Arc<script::native::PreparedConfig>, GameSnapshot) {
        let config = api::selected::FamilyPreparation::run(|families| {
            let selected =
                api::game_data::for_revision(api::selected::ClientRevision::R289).unwrap();
            let mut cx = script::native::PrepareContext {
                pin: selected.selected_pin().unwrap(),
                selected,
                banks: Arc::default(),
                families,
            };
            let mut settings = script::native::SettingsBag::new();
            settings.insert("disposition".into(), json!("Bank"));
            (script::gatherer::CARD.prepare)(&mut cx, 1, Arc::new(settings)).unwrap()
        })
        .unwrap()
        .join()
        .unwrap();
        let mut snapshot = GameSnapshot::new();
        snapshot.seed_inventory(
            items
                .iter()
                .enumerate()
                .map(|(slot, &(id, count))| api::snapshot::ItemView {
                    def: api::ItemDefView {
                        id,
                        name: None,
                        stackable: false,
                        members: false,
                        base_value: 0,
                        noted: false,
                        certificate_link: -1,
                        certificate_template: -1,
                    },
                    container: api::snapshot::ItemContainer::Inventory,
                    action_family: api::snapshot::ItemActionFamily::Held,
                    slot: slot as i32,
                    count,
                    actions: vec![],
                    component_id: -1,
                })
                .collect(),
            28,
        );
        (config, snapshot)
    }

    fn native_deposit_ids(items: &[(i32, i32)]) -> Arc<[i32]> {
        let (config, snapshot) = bank_fixture(items);
        script::gatherer::test_bank_deposit_ids(&config, &snapshot).unwrap()
    }

    #[test]
    fn woodcutting_status_gate_rejects_kebab_left_after_deposit() {
        use script::native::{NativePhase, ScriptStatus, StatusField, StatusValue};
        let (config, snapshot) = bank_fixture(&[(LOG_ID, 4), (KEBAB_ID, 1)]);
        let mut slot = GatherSlot::new(
            Cell::Woodcutting,
            LiveCase::WoodcuttingBank,
            SEERS_MAPLE_BANK_START,
            FixturePlan::default(),
            None,
            false,
        );
        slot.bank_config = Some(config);
        slot.snapshot = snapshot;
        slot.snapshot
            .seed_bank_observation(5292, 1, Some(vec![]), vec![]);
        let status = |event: &'static str, deposited| ScriptStatus {
            run: api::selected::RunKey {
                slot: 1,
                run: 1,
                session: 1,
            },
            card: script::CompiledId("Gatherer"),
            phase: NativePhase::Working,
            active_settings: 1,
            pending_settings: None,
            failure: None,
            fields: Arc::from([
                StatusField {
                    key: "bank",
                    label: "Bank",
                    value: StatusValue::Text("Seers; Deposit".into()),
                },
                StatusField {
                    key: "last_event",
                    label: "Event",
                    value: StatusValue::Text(event.into()),
                },
                StatusField {
                    key: "deposited",
                    label: "Deposited",
                    value: StatusValue::Integer(deposited),
                },
            ]),
        };
        slot.apply_status(&status("bank opened", 0)).unwrap();
        let rows = slot.snapshot.inventory().to_vec();
        slot.snapshot.seed_inventory(vec![rows[1].clone()], 28);
        slot.snapshot
            .seed_bank_observation(5292, 2, Some(vec![rows[0].clone()]), vec![]);
        let result = slot.apply_status(&status("deposit confirmed", 4));
        assert!(
            result.is_err(),
            "a completed WC deposit must not leave a kebab: {result:?}"
        );
    }

    #[test]
    fn tool_fetch_first_visit_is_not_a_deposit_trip() {
        let empty = BTreeMap::new();
        let deposit_ids = native_deposit_ids(&[]);
        assert!(deposit_ids.is_empty());
        let mut receipt = BankTripReceipt::new(
            1,
            empty.clone(),
            &deposit_ids,
            empty.clone(),
            0,
            &[LOG_ID],
            0,
        );
        receipt
            .observe_deposit(&empty, &empty, true, &empty, 0, true)
            .unwrap();
        receipt.record_return(1, 0, false).unwrap();
        assert!(!receipt.is_deposit_trip());
    }

    #[test]
    fn woodcutting_bank_conserves_products_and_incidental_kebab() {
        let inventory_before = counts(&[(LOG_ID, 4), (KEBAB_ID, 1)]);
        let deposit_ids = native_deposit_ids(&[(LOG_ID, 4), (KEBAB_ID, 1)]);
        let expected_items = bank_deposit_expectation(&inventory_before, &deposit_ids);
        let expected_products = counts(&[(LOG_ID, 4)]);
        let expected_incidentals = counts(&[(KEBAB_ID, 1)]);
        let empty_inventory = BTreeMap::new();
        let empty_bank = BTreeMap::new();
        let bank_after = counts(&[(LOG_ID, 4), (KEBAB_ID, 1)]);

        let mut receipt = BankTripReceipt::new(
            1,
            inventory_before.clone(),
            &deposit_ids,
            empty_bank.clone(),
            0,
            &[LOG_ID],
            0,
        );
        assert_eq!(receipt.expected_incidentals, expected_incidentals);
        receipt
            .observe_deposit(
                &empty_inventory,
                &empty_inventory,
                true,
                &bank_after,
                5,
                true,
            )
            .unwrap();
        assert!(receipt.deposit_verified);
        assert_eq!(receipt.as_json()["expected_incidentals"]["1971"], json!(1));

        assert!(bank_conservation_satisfied(
            &expected_items,
            &expected_products,
            &empty_inventory,
            &empty_bank,
            &bank_after,
            5,
            &empty_inventory,
        ));
        let product_lost_bank = counts(&[(KEBAB_ID, 1)]);
        assert!(!bank_conservation_satisfied(
            &expected_items,
            &expected_products,
            &empty_inventory,
            &empty_bank,
            &product_lost_bank,
            5,
            &empty_inventory,
        ));

        let kebab_left = counts(&[(KEBAB_ID, 1)]);
        let kebab_left_unneeded = bank_deposit_expectation(&kebab_left, &deposit_ids);
        let product_only_bank = counts(&[(LOG_ID, 4)]);
        assert!(!bank_conservation_satisfied(
            &expected_items,
            &expected_products,
            &kebab_left,
            &empty_bank,
            &product_only_bank,
            4,
            &kebab_left_unneeded,
        ));
    }

    #[test]
    fn harpoon_bank_receipts_follow_seeded_fixture_and_incidental_slots() {
        let seeded_baseline = counts(&[(HARPOON_ID, 1), (CASKET_ID, 1)]);
        assert_eq!(seeded_baseline.get(&CASKET_ID), Some(&1));

        let deposit_ids = [CASKET_ID, SWORDFISH_ID];
        let first_inventory = counts(&[(HARPOON_ID, 1), (CASKET_ID, 1), (SWORDFISH_ID, 26)]);
        let empty_bank = BTreeMap::new();
        let first_bank = counts(&[(CASKET_ID, 1), (SWORDFISH_ID, 26)]);
        let carried_tool = counts(&[(HARPOON_ID, 1)]);
        let mut first = BankTripReceipt::new(
            1,
            first_inventory.clone(),
            &deposit_ids,
            empty_bank,
            0,
            &[SWORDFISH_ID],
            1,
        );
        assert_eq!(first.expected_product_count(), 26);
        assert_eq!(first.expected_incidentals.get(&CASKET_ID), Some(&1));
        assert!(!first.bank_increased_for(CASKET_ID, &BTreeMap::new()));
        assert!(first.bank_increased_for(CASKET_ID, &first_bank));
        assert!(!first.bank_increased_for(HARPOON_ID, &carried_tool));
        let mut existing_casket = first.clone();
        existing_casket.bank_before = counts(&[(CASKET_ID, 1)]);
        assert!(!existing_casket.bank_increased_for(CASKET_ID, &first_bank));
        assert!(existing_casket.bank_increased_for(CASKET_ID, &counts(&[(CASKET_ID, 2)])));
        first
            .observe_deposit(&carried_tool, &BTreeMap::new(), true, &first_bank, 27, true)
            .unwrap();
        first.record_return(1, 27, true).unwrap();
        first.post_bank_yield = true;
        assert!(first.complete());

        let second_inventory = counts(&[(HARPOON_ID, 1), (SWORDFISH_ID, 27)]);
        let second_bank = counts(&[(CASKET_ID, 1), (SWORDFISH_ID, 53)]);
        let mut second = BankTripReceipt::new(
            2,
            second_inventory,
            &deposit_ids,
            first_bank.clone(),
            27,
            &[SWORDFISH_ID],
            0,
        );
        assert_eq!(second.expected_product_count(), 27);
        second
            .observe_deposit(
                &carried_tool,
                &BTreeMap::new(),
                true,
                &second_bank,
                54,
                true,
            )
            .unwrap();
        second.record_return(2, 54, true).unwrap();
        second.post_bank_yield = true;
        assert!(second.complete());
        assert_ne!(first.expected_products, second.expected_products);

        // A later casket slot lowers the second catch, while the two actual
        // deposits still contain 54 items.
        let legacy_start = counts(&[(HARPOON_ID, 1)]);
        let legacy_capacity = 28 - legacy_start.len() as u32;
        let old_first_pack = counts(&[(HARPOON_ID, 1), (SWORDFISH_ID, 27)]);
        let old_second_pack = counts(&[(HARPOON_ID, 1), (SWORDFISH_ID, 26), (CASKET_ID, 1)]);
        let old_first_expected = bank_deposit_expectation(&old_first_pack, &deposit_ids);
        let old_second_expected = bank_deposit_expectation(&old_second_pack, &deposit_ids);
        let old_fish_deposited = i64::from(
            old_first_expected.get(&SWORDFISH_ID).copied().unwrap_or(0)
                + old_second_expected.get(&SWORDFISH_ID).copied().unwrap_or(0),
        );
        let old_required = i64::from(legacy_capacity * 2);
        let actual_item_deposits = old_first_expected
            .values()
            .map(|count| i64::from(*count))
            .sum::<i64>()
            + old_second_expected
                .values()
                .map(|count| i64::from(*count))
                .sum::<i64>();
        assert_eq!(old_required, 54);
        assert_eq!(old_fish_deposited, 53);
        assert!(old_fish_deposited < old_required);
        assert_eq!(actual_item_deposits, 54);

        let mut product_only_baseline = BankTripReceipt::new(
            1,
            first_inventory,
            &deposit_ids,
            BTreeMap::new(),
            0,
            &[SWORDFISH_ID],
            1,
        );
        let left_in_pack = counts(&[(HARPOON_ID, 1), (CASKET_ID, 1)]);
        let product_only_bank = counts(&[(SWORDFISH_ID, 26)]);
        let error = product_only_baseline
            .observe_deposit(
                &left_in_pack,
                &bank_deposit_expectation(&left_in_pack, &deposit_ids),
                true,
                &product_only_bank,
                26,
                true,
            )
            .unwrap_err();
        assert!(error.contains("seeded casket"));
        assert!(!product_only_baseline.seeded_casket_banked);
    }
}

fn is_mining_gem(id: i32) -> bool {
    MINE_PRODUCTS.contains(&id) && ![COPPER_ID, TIN_ID, IRON_ID, COAL_ID].contains(&id)
}

fn real_content_bank_lifecycle_complete(
    case: LiveCase,
    witness: &Witness,
    plan: &FixturePlan,
    target: WorldTile,
    baseline_xp: i32,
) -> bool {
    let (product_id, bank_prefix) = match case {
        LiveCase::BankCostFirstGoal => (1521, "Draynor; Reachable;"),
        LiveCase::BankCostSeersMaple => (MAPLE_LOG_ID, "Seers; Reachable;"),
        _ => return false,
    };
    witness.status_trips >= 1
        && (case != LiveCase::BankCostSeersMaple
            || (witness.bank_return_step_seen && witness.bank_arrivals >= 1))
        && witness.status_deposited > 0
        && witness.bank_nonzero_roundtrips >= 1
        && witness.post_bank_yields >= 1
        && witness.bank_loaded_observed
        && witness.bank_closed_observed
        && witness.bank_withdrawal_confirmed
        && witness.last_status_yielded > 0
        && witness.products_seen.contains(&product_id)
        && plan.seed_locs.is_empty()
        && plan.oak_tiles.is_empty()
        && (case != LiveCase::BankCostFirstGoal
            || witness
                .oak_live_tiles
                .iter()
                .any(|&tile| tile_distance(tile, target).is_some_and(|distance| distance <= 12)))
        && witness
            .last_status_bank
            .as_deref()
            .is_some_and(|bank| bank.starts_with(bank_prefix) && bank.contains("; access:"))
        && witness.last_xp > baseline_xp
}

fn parse_work_anchor(area: &str) -> Option<WorldTile> {
    let coords = area.strip_prefix("site (")?.split_once(')')?.0;
    parse_tile("site area", coords).ok()
}

fn site_nav_targets_anchor(nav: &Value, anchor: WorldTile) -> bool {
    nav["requested_destination"] == json!([anchor.x, anchor.z, anchor.level])
}

fn site_return_nav_matches(nav: &Value, anchor: WorldTile) -> bool {
    nav["armed"] == true
        && nav["arrival"] == "Area"
        && nav["requested_radius"] == 1
        && site_nav_targets_anchor(nav, anchor)
}

fn site_bank_complete(cell: Cell, witness: &Witness, plan: &FixturePlan, baseline_xp: i32) -> bool {
    witness.site_anchor_selected
        && witness.site_initial_walk
        && witness.site_return_area_r1
        && witness.bank_return_step_seen
        && witness.bank_arrivals >= 1
        && witness.bank_trips.iter().any(BankTripReceipt::complete)
        && witness.bank_nonzero_roundtrips >= 1
        && witness.post_bank_yields >= 1
        && witness.last_xp > baseline_xp
        && cell
            .products(LiveCase::Site)
            .iter()
            .any(|id| witness.products_seen.contains(id))
        && witness
            .last_status_bank
            .as_deref()
            .is_some_and(|bank| bank.starts_with(cell.site_bank()) && bank.contains("; Reachable;"))
        && plan.seed_locs.is_empty()
        && plan.oak_tiles.is_empty()
}

fn power_complete(cell: Cell, witness: &Witness) -> bool {
    witness.cycles >= REQUIRED_CYCLES
        && witness.post_drop_gathers >= REQUIRED_POST_DROP_GATHERS
        && (cell != Cell::Mining || witness.dropped_product_ids.contains(&SEEDED_MINING_GEM_ID))
}

#[test]
fn seers_maple_lifecycle_requires_the_named_bank_and_a_returned_yield() {
    let mut witness = Witness {
        status_trips: 1,
        bank_arrivals: 1,
        bank_return_step_seen: true,
        status_deposited: 1,
        bank_nonzero_roundtrips: 1,
        post_bank_yields: 1,
        bank_loaded_observed: true,
        bank_closed_observed: true,
        bank_withdrawal_confirmed: true,
        last_status_yielded: 1,
        last_xp: 1,
        last_status_bank: Some("Seers; Reachable; access:2724,3493".into()),
        ..Witness::default()
    };
    witness.products_seen.insert(MAPLE_LOG_ID);
    let plan = FixturePlan::default();
    let target = SEERS_MAPLE_BANK_START;

    assert!(real_content_bank_lifecycle_complete(
        LiveCase::BankCostSeersMaple,
        &witness,
        &plan,
        target,
        0,
    ));

    witness.bank_return_step_seen = false;
    assert!(
        !real_content_bank_lifecycle_complete(
            LiveCase::BankCostSeersMaple,
            &witness,
            &plan,
            target,
            0,
        ),
        "a deposit-time trip count cannot substitute for the bank return step"
    );
    witness.bank_return_step_seen = true;

    witness.last_status_bank = Some("Edgeville; Reachable; access:3094,3490".into());
    assert!(
        !real_content_bank_lifecycle_complete(
            LiveCase::BankCostSeersMaple,
            &witness,
            &plan,
            target,
            0,
        ),
        "a different reachable bank cannot satisfy the Seers-only fixture"
    );

    witness.last_status_bank = Some("Seers; Reachable; access:2724,3493".into());
    witness.bank_arrivals = 0;
    assert!(
        !real_content_bank_lifecycle_complete(
            LiveCase::BankCostSeersMaple,
            &witness,
            &plan,
            target,
            0,
        ),
        "a deposit-time trip count cannot substitute for an observed resource-stand return"
    );

    witness.bank_arrivals = 1;
    witness.post_bank_yields = 0;
    assert!(
        !real_content_bank_lifecycle_complete(
            LiveCase::BankCostSeersMaple,
            &witness,
            &plan,
            target,
            0,
        ),
        "returning without a fresh post-bank yield is incomplete"
    );

    witness.post_bank_yields = 1;
    witness.products_seen.clear();
    assert!(
        !real_content_bank_lifecycle_complete(
            LiveCase::BankCostSeersMaple,
            &witness,
            &plan,
            target,
            0,
        ),
        "a return yield must include a maple-log resource product"
    );

    witness.products_seen.insert(MAPLE_LOG_ID);
    let mut seeded_scene = FixturePlan::default();
    seeded_scene
        .seed_locs
        .push(seed(world_tile(1, 1), "oaktree", OAK_ID));
    assert!(
        !real_content_bank_lifecycle_complete(
            LiveCase::BankCostSeersMaple,
            &witness,
            &seeded_scene,
            target,
            0,
        ),
        "scene-seeded resources cannot satisfy the real-content fixture"
    );
    let mut seeded_oak_tiles = FixturePlan::default();
    seeded_oak_tiles.oak_tiles.push(world_tile(1, 1));
    assert!(
        !real_content_bank_lifecycle_complete(
            LiveCase::BankCostSeersMaple,
            &witness,
            &seeded_oak_tiles,
            target,
            0,
        ),
        "the oak fixture tile list must also remain empty"
    );
}

#[test]
fn mining_power_requires_seeded_gem_disposal_not_a_random_drop() {
    let mut witness = Witness {
        cycles: REQUIRED_CYCLES,
        post_drop_gathers: REQUIRED_POST_DROP_GATHERS,
        ..Witness::default()
    };
    witness.dropped_product_ids.insert(TIN_ID);
    assert!(!power_complete(Cell::Mining, &witness));
    witness.dropped_product_ids.insert(1621);
    assert!(
        !power_complete(Cell::Mining, &witness),
        "an incidental emerald cannot substitute for the seeded sapphire"
    );
    witness.dropped_product_ids.insert(SEEDED_MINING_GEM_ID);
    assert!(witness.natural_gems_seen.is_empty());
    assert!(
        power_complete(Cell::Mining, &witness),
        "no natural gem roll is required"
    );
    witness.post_drop_gathers = 0;
    assert!(
        !power_complete(Cell::Mining, &witness),
        "seeded disposal cannot substitute for renewed gathering"
    );
}

#[cfg(test)]
mod fish_harpoon_auto_fixture_tests {
    use super::*;

    #[test]
    fn auto_harpoon_fixture_matches_operator_settings_without_rng_seeds() {
        let case = LiveCase::FishHarpoonBankAuto;
        let settings = Cell::Fishing.settings(case);
        assert_eq!(settings.get("skill"), Some(&json!("Fishing")));
        assert_eq!(
            settings.get("fishingMethod"),
            Some(&json!("fishing.rarefish.op3"))
        );
        assert_eq!(settings.get("location"), Some(&json!("Auto")));
        assert_eq!(settings.get("radius"), Some(&json!(40)));
        assert_eq!(settings.get("disposition"), Some(&json!("Bank")));
        assert_eq!(settings.get("bank"), Some(&json!("Catherby")));
        assert_eq!(Cell::Fishing.default_level(case), 76);
        assert_eq!(Cell::Fishing.level(case), 76);
        assert_eq!(Cell::Fishing.default_tool_alias(case), "harpoon");
        assert_eq!(Cell::Fishing.default_tool_id(case), 311);
        assert_eq!(Cell::Fishing.products(case), &[TUNA_ID, SWORDFISH_ID]);

        let (target, plan, task) = fixture_plan(Cell::Fishing, case).unwrap();
        assert_eq!(target, CATHERBY_HARPOON_START);
        assert!(plan.inventory_seed.is_empty());
        assert!(plan.bank_seed.is_empty());
        assert!(plan.seed_locs.is_empty());
        assert!(plan.auto_felled.is_empty());
        assert!(plan.auto_next.is_empty());
        assert!(plan.inject_after_progress.is_none());
        assert!(task.is_none());

        let fixed = LiveCase::FishHarpoonBank;
        let fixed_settings = Cell::Fishing.settings(fixed);
        assert_eq!(fixed_settings.get("location"), Some(&json!("Start")));
        assert_eq!(fixed_settings.get("radius"), Some(&json!(12)));
        assert_eq!(
            fixed_settings.get("fishingMethod"),
            Some(&json!("fishing.rarefish.op3"))
        );
        assert_eq!(Cell::Fishing.default_level(fixed), 99);
        let (_, fixed_plan, _) = fixture_plan(Cell::Fishing, fixed).unwrap();
        assert_eq!(fixed_plan.inventory_seed, vec![("casket".to_owned(), 1)]);
    }

    #[test]
    fn auto_harpoon_completion_does_not_require_incidental_casket() {
        let positive_trip = || {
            let mut trip = BankTripReceipt::new(
                1,
                BTreeMap::from([(311, 1), (SWORDFISH_ID, 2)]),
                &[SWORDFISH_ID],
                BTreeMap::new(),
                0,
                &[SWORDFISH_ID],
                0,
            );
            trip.deposit_verified = true;
            trip.positive_return = true;
            trip.returned = true;
            trip.post_bank_yield = true;
            trip
        };
        let witness = Witness {
            bank_trips: vec![positive_trip(), positive_trip()],
            ..Witness::default()
        };
        assert!(witness.fish_bank_complete(false));
        assert!(!witness.fish_bank_complete(true));
    }
}

#[cfg(test)]
mod named_site_fixture_tests {
    use super::*;

    #[test]
    fn site_fixtures_use_named_real_content_without_coordinate_settings() {
        for cell in [Cell::Fishing, Cell::Woodcutting, Cell::Mining] {
            let settings = cell.settings(LiveCase::Site);
            assert_eq!(settings.get("location"), Some(&json!("Site")));
            assert_eq!(settings.get("site"), Some(&json!(cell.site_id())));
            assert_eq!(settings.get("radius"), Some(&json!(12)));
            assert_eq!(settings.get("disposition"), Some(&json!("Bank")));
            assert_eq!(settings.get("bank"), Some(&json!("Nearest")));
            for coordinate in ["x", "z", "level"] {
                assert!(!settings.contains_key(coordinate));
            }
            let (_, plan, task) = fixture_plan(cell, LiveCase::Site).unwrap();
            assert!(plan.seed_locs.is_empty());
            assert!(plan.oak_tiles.is_empty());
            assert!(plan.inventory_seed.is_empty());
            assert!(plan.bank_seed.is_empty());
            assert!(plan.auto_felled.is_empty());
            assert!(plan.auto_next.is_empty());
            assert!(plan.inject_after_progress.is_none());
            assert!(task.is_none());
        }
    }

    #[test]
    fn site_completion_requires_real_anchor_walk_area_return_and_fresh_yield() {
        for cell in [Cell::Fishing, Cell::Woodcutting, Cell::Mining] {
            let product = cell.products(LiveCase::Site)[0];
            let mut trip = BankTripReceipt::new(
                1,
                BTreeMap::from([(cell.default_tool_id(LiveCase::Site), 1), (product, 2)]),
                &[product],
                BTreeMap::new(),
                0,
                &[product],
                0,
            );
            trip.deposit_verified = true;
            trip.positive_return = true;
            trip.returned = true;
            trip.post_bank_yield = true;
            let mut witness = Witness {
                site_anchor_selected: true,
                site_initial_walk: true,
                site_return_area_r1: true,
                bank_return_step_seen: true,
                bank_arrivals: 1,
                bank_trips: vec![trip],
                bank_nonzero_roundtrips: 1,
                post_bank_yields: 1,
                last_xp: 1,
                products_seen: BTreeSet::from([product]),
                last_status_bank: Some(format!("{}; Reachable; Return", cell.site_bank())),
                ..Witness::default()
            };
            let mut plan = FixturePlan::default();
            assert!(site_bank_complete(cell, &witness, &plan, 0));
            witness.site_anchor_selected = false;
            assert!(!site_bank_complete(cell, &witness, &plan, 0));
            witness.site_anchor_selected = true;
            witness.site_initial_walk = false;
            assert!(!site_bank_complete(cell, &witness, &plan, 0));
            witness.site_initial_walk = true;
            witness.site_return_area_r1 = false;
            assert!(!site_bank_complete(cell, &witness, &plan, 0));
            witness.site_return_area_r1 = true;
            witness.post_bank_yields = 0;
            assert!(!site_bank_complete(cell, &witness, &plan, 0));
            witness.post_bank_yields = 1;
            witness.bank_trips[0].deposit_verified = false;
            assert!(!site_bank_complete(cell, &witness, &plan, 0));
            witness.bank_trips[0].deposit_verified = true;
            witness.last_status_bank = Some("Different bank; Reachable; Return".into());
            assert!(!site_bank_complete(cell, &witness, &plan, 0));
            witness.last_status_bank = Some(format!("{}; Reachable; Return", cell.site_bank()));
            plan.oak_tiles.push(world_tile(1, 1));
            assert!(!site_bank_complete(cell, &witness, &plan, 0));
        }
    }

    #[test]
    fn site_work_anchor_parser_only_accepts_resolved_site_areas() {
        assert_eq!(
            parse_work_anchor("site (3083,3237,0) r12"),
            Some(world_tile(3083, 3237))
        );
        assert_eq!(parse_work_anchor("site unresolved"), None);
        assert_eq!(parse_work_anchor("auto (3083,3237,0) r12"), None);
        assert_eq!(parse_work_anchor("site (bad,3237,0) r12"), None);
    }

    #[test]
    fn return_proof_requires_current_area_request_at_the_retained_anchor() {
        let anchor = world_tile(3083, 3237);
        let mut nav = json!({
            "armed": true,
            "arrival": "Area",
            "requested_radius": 1,
            "requested_destination": [anchor.x, anchor.z, anchor.level],
            "outcome_radius": 1,
        });
        assert!(site_return_nav_matches(&nav, anchor));
        nav["requested_destination"] = json!([3084, 3237, 0]);
        assert!(!site_return_nav_matches(&nav, anchor));
        nav["requested_destination"] = json!([anchor.x, anchor.z, anchor.level]);
        nav["requested_radius"] = json!(12);
        assert!(
            !site_return_nav_matches(&nav, anchor),
            "a stale r1 outcome is not a current Return"
        );
        nav["requested_radius"] = json!(1);
        nav["arrival"] = json!("Reach");
        assert!(!site_return_nav_matches(&nav, anchor));
        nav["arrival"] = json!("Area");
        nav["armed"] = json!(false);
        assert!(!site_return_nav_matches(&nav, anchor));
    }
}
