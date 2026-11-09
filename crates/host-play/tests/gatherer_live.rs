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
//! boundaries, and Auto widening. Gas and random-event fixtures fail closed
//! because the content has no deterministic account-only trigger. G3 cells
//! cover banked-tool/supply trips, cost-ranked bank selection, and deposit returns.
//! The fixed harpoon Bank cell requires two positive deposit trips and fresh
//! fishing yields after each return, and conserves the inventory-only harpoon.
//! Its first trip also verifies that a seeded casket, spare net and beer are
//! deposited.
//! Its Auto sibling exercises the operator Fishing 76/radius 40/Catherby Start
//! bag and the panel's friendly "Raw tuna / Raw swordfish" label, without seeded
//! fish, resources, NPCs or an incidental-drop gate. Initial level/tool fixtures
//! precede the progression baseline; proof gains come from the running script.
//! G4a cells verify death recovery, including terminal return refusals and
//! watchdog recreation. Gas and random-event identity stay explicit live-proof
//! limits because their real content outcomes are chance-gated.
//! Fixtures seed only account-owned state; they never inject scene locations,
//! products or XP.
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
use host_play::evidence_writer::{
    EvidenceJob, EvidenceRequest, EvidenceSidecar, EvidenceWriter, PngColor,
};
use host_play::{ProfileOptions, ScriptStartHandle, SharedClientTemplate, SlotArm};
#[cfg(feature = "live-harness")]
use scenario::{RunnerStatus, ScenarioRunner};
use serde_json::{json, Map, Value};
use vault::{Profile, ProfileSettings};
#[cfg(all(feature = "live-harness", feature = "test-support"))]
#[path = "support/gatherer_tick_live.rs"]
mod gatherer_tick_live;

const PREP_DEADLINE: Duration = Duration::from_secs(180);
const POWER_DEADLINE: Duration = Duration::from_secs(1800);
const WEDGE_DEADLINE: Duration = Duration::from_secs(900);
const POLL_INTERVAL: Duration = Duration::from_millis(20);
const CLEAN_LOGOUT_DEADLINE: Duration = Duration::from_secs(30);
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
const SPARE_NET_ID: i32 = 303;
const BEER_ID: i32 = 1917;
const OAK_ID: i32 = 1281;
const MAPLE_LOG_ID: i32 = 1517;
const WILLOW_LOG_ID: i32 = 1519;
const OAK_STUMP_ID: i32 = 1355;
const OAK_RESPAWN_MAX_TICKS: u32 = 30;
const AUTO_TREE_IDS: &[i32] = &[1276, 1278];
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
const DEATH_RETURN_REFUSED_TARGET: WorldTile = WorldTile {
    x: 3204,
    z: 3525,
    level: 0,
};
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
const ARDOUGNE_GUILD_START: WorldTile = WorldTile {
    x: 2661,
    z: 3306,
    level: 0,
};

const FISHING_GUILD_START: WorldTile = WorldTile {
    x: 2611,
    z: 3399,
    level: 0,
};
const DRAYNOR_FISH_SITE_START: WorldTile = WorldTile {
    x: 3086,
    z: 3229,
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
const DRAYNOR_OAK_TARGET: WorldTile = WorldTile {
    x: 3106,
    z: 3247,
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
const OAK_NO_PLACEMENT_START: WorldTile = WorldTile {
    x: 2809,
    z: 3441,
    level: 0,
};
const OAK_NO_PLACEMENT_BANK_BOOTH: WorldTile = WorldTile {
    x: 2809,
    z: 3442,
    level: 0,
};
const AUTO_START: WorldTile = WorldTile {
    x: 3295,
    z: 3298,
    level: 0,
};
const GAS_START: WorldTile = WorldTile {
    x: 3294,
    z: 3310,
    level: 0,
};
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
            (Self::Fishing, LiveCase::SiteDraynor) => "gatherer_fish_draynor_site_bank_return",
            (Self::Woodcutting, LiveCase::Site) => "gatherer_wc_willow_site",
            (Self::Mining, LiveCase::Site) => "gatherer_mine_copper_tin_site",
            (Self::Mining, LiveCase::Power) => "gatherer_mine_tier_power",
            (Self::Woodcutting, LiveCase::CancelBeforeDrain) => "gatherer_cancel_before_drain_live",
            (Self::Woodcutting, LiveCase::DeathDuringDrop) => "gatherer_death_during_drop_live",
            (Self::Woodcutting, LiveCase::RunKeyChangeDuringDrop) => {
                "gatherer_run_key_change_during_drop_live"
            }
            (Self::Fishing, LiveCase::FishNet) => "gatherer_fish_net",
            (Self::Fishing, LiveCase::FishSpotMoves) => "gatherer_fish_catherby_spot_moves",
            (Self::Fishing, LiveCase::FishBaitGate) => "gatherer_fish_bait_gate",
            (Self::Woodcutting, LiveCase::OakRespawn) => "gatherer_wc_respawn",
            (Self::Woodcutting, LiveCase::OakNearEdge) => "gatherer_wc_near_edge",
            (Self::Woodcutting, LiveCase::LocationAuto) => "gatherer_location_modes",
            (Self::Woodcutting, LiveCase::OakNoPlacementArea) => "gatherer_wc_area_no_placement",
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
            (Self::Fishing, LiveCase::FishBankSoak) => "gatherer_fish_bank_soak",
            (Self::Fishing, LiveCase::FishGuildApproach) => "gatherer_fish_guild_ardougne",
            (Self::Woodcutting, LiveCase::CoinRunes) => "gatherer_coin_runes",
            (Self::Woodcutting, LiveCase::CoinRunesEmpty) => "gatherer_coin_runes_empty_stock",
            (Self::Woodcutting, LiveCase::PowerToBank) => "gatherer_power_to_bank_live",
            (Self::Woodcutting, LiveCase::PauseResumeOtherPlane) => {
                "gatherer_pause_resume_other_plane_live"
            }
            (Self::Woodcutting, LiveCase::ReconnectReturn) => "gatherer_reconnect_return_live",
            (Self::Woodcutting, LiveCase::BankTripRelog) => "gatherer_bank_trip_relog_live",
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
            LiveCase::Site if matches!(self, Self::Fishing) => "GATHERER_FISH_SITE_START_TILE",
            LiveCase::CoinRunes | LiveCase::CoinRunesEmpty => "GATHERER_COIN_RUNES_TILE",
            LiveCase::PowerToBank => "GATHERER_POWER_TO_BANK_TILE",
            LiveCase::PauseResumeOtherPlane => "GATHERER_RETURN_PAUSE_TILE",
            LiveCase::ReconnectReturn | LiveCase::BankTripRelog => "GATHERER_RECONNECT_RETURN_TILE",
            LiveCase::GasHazard => "GATHERER_GAS_TILE",
            LiveCase::OakNoPlacementArea => "GATHERER_WC_NO_PLACEMENT_TILE",
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
            (
                Self::Fishing,
                LiveCase::FishNet | LiveCase::FishSpotMoves | LiveCase::SiteDraynor,
            ) => "net",
            (Self::Fishing, LiveCase::FishBaitGate | LiveCase::FishBait) => "fly_fishing_rod",
            (Self::Fishing, LiveCase::FishHarpoonBank | LiveCase::FishHarpoonBankAuto) => "harpoon",
            (Self::Fishing, LiveCase::FishBankSoak) => "net",
            (Self::Fishing, LiveCase::Site | LiveCase::FishGuildApproach) => "harpoon",
            (Self::Mining, LiveCase::BankCost | LiveCase::Site) => "bronze_pickaxe",
            (Self::Mining, _) => "steel_pickaxe",
            (Self::Woodcutting, LiveCase::WoodcuttingBankUnwieldable) => "rune_axe",
            _ => "bronze_axe",
        }
    }

    const fn default_tool_id(self, case: LiveCase) -> i32 {
        match (self, case) {
            (
                Self::Fishing,
                LiveCase::FishNet | LiveCase::FishSpotMoves | LiveCase::SiteDraynor,
            ) => 303,
            (Self::Fishing, LiveCase::FishBaitGate | LiveCase::FishBait) => 309,
            (Self::Fishing, LiveCase::FishHarpoonBank | LiveCase::FishHarpoonBankAuto) => 311,
            (Self::Fishing, LiveCase::FishBankSoak) => 303,
            (Self::Fishing, LiveCase::Site | LiveCase::FishGuildApproach) => 311,
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
            (Self::Fishing, LiveCase::SiteDraynor) => 15,
            (Self::Fishing, LiveCase::FishSpotMoves) => 76,
            (Self::Fishing, LiveCase::FishGuildApproach) => 68,
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
                | LiveCase::OakNoPlacementArea
                | LiveCase::WoodcuttingBank
                | LiveCase::BankCostFirstGoal
                | LiveCase::BankCostNoCandidate
                | LiveCase::WoodcuttingBankUnwieldable
                | LiveCase::PauseResumeOtherPlane
                | LiveCase::ReconnectReturn
                | LiveCase::BankTripRelog,
            ) => 15,
            (Self::Fishing, LiveCase::FishBaitGate | LiveCase::FishBait) => 20,
            (Self::Woodcutting, LiveCase::BankCostSeersMaple) => 45,
            (Self::Fishing, LiveCase::FishHarpoonBank) => 99,
            (Self::Fishing, LiveCase::FishHarpoonBankAuto) => 76,
            (Self::Fishing, LiveCase::FishBankSoak) => 40,
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
            (
                Self::Fishing,
                LiveCase::SiteDraynor
                | LiveCase::FishNet
                | LiveCase::FishSpotMoves
                | LiveCase::FishBankSoak,
            ) => &[SHRIMP_ID, ANCHOVY_ID],
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
                | LiveCase::OakNoPlacementArea
                | LiveCase::WoodcuttingBank
                | LiveCase::BankCostFirstGoal
                | LiveCase::BankCostNoCandidate
                | LiveCase::WoodcuttingBankUnwieldable
                | LiveCase::PauseResumeOtherPlane
                | LiveCase::ReconnectReturn
                | LiveCase::BankTripRelog,
            ) => &[1521],
            (Self::Fishing, LiveCase::FishBaitGate | LiveCase::FishBait) => &[TROUT_ID, SALMON_ID],
            (Self::Fishing, LiveCase::FishHarpoonBank | LiveCase::FishHarpoonBankAuto) => {
                &[TUNA_ID, SWORDFISH_ID]
            }
            (Self::Fishing, LiveCase::FishGuildApproach) => &[TUNA_ID, SWORDFISH_ID],
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
                    | LiveCase::OakNoPlacementArea
                    | LiveCase::WoodcuttingBank
                    | LiveCase::WoodcuttingBankUnwieldable
                    | LiveCase::BankCostFirstGoal
                    | LiveCase::BankCostNoCandidate
                    | LiveCase::PauseResumeOtherPlane
                    | LiveCase::ReconnectReturn
                    | LiveCase::BankTripRelog
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
            (_, LiveCase::Site | LiveCase::SiteDraynor) => None,
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
            (Self::Woodcutting, LiveCase::ReconnectReturn | LiveCase::BankTripRelog) => {
                Some("GATHERER_RECONNECT_RETURN_LEVEL")
            }
            (Self::Woodcutting, _) => Some("GATHERER_WC_LEVEL"),
            (Self::Mining, _) => Some("GATHERER_MINE_LEVEL"),
            (Self::Fishing, LiveCase::FishHarpoonBankAuto | LiveCase::FishGuildApproach) => None,
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
                | LiveCase::ReconnectReturn
                | LiveCase::BankTripRelog,
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
                | LiveCase::ReconnectReturn
                | LiveCase::BankTripRelog,
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
                    LiveCase::FishGuildApproach => "fishing.rarefish.op3",
                    LiveCase::Site => "fishing.harpoon.tool_311.products_359_371",
                    LiveCase::SiteDraynor => "fishing.saltfish.op1",
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
                LiveCase::OakRespawn
                | LiveCase::LocationAuto
                | LiveCase::BankCost
                | LiveCase::FishSpotMoves
                | LiveCase::FishBankSoak => 32,
                LiveCase::FishHarpoonBankAuto => 40,
                LiveCase::OakNearEdge => 64,
                LiveCase::OakNoPlacementArea => 2,
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
                    | LiveCase::FishBankSoak
                    | LiveCase::PauseResumeOtherPlane
                    | LiveCase::ReconnectReturn
                    | LiveCase::BankTripRelog
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
                | LiveCase::FishBankSoak
                | LiveCase::PowerToBank
                | LiveCase::PauseResumeOtherPlane
                | LiveCase::ReconnectReturn
                | LiveCase::BankTripRelog
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
        if case == LiveCase::FishSpotMoves {
            bag.insert("location".into(), json!("Site"));
            bag.insert("site".into(), json!("fishing.catherby"));
            bag.insert("disposition".into(), json!("Bank"));
            bag.insert("bank".into(), json!("Nearest"));
        }
        if case.is_named_site() {
            bag.insert("location".into(), json!("Site"));
            bag.insert("site".into(), json!(self.site_id_for_case(case)));
            bag.insert("disposition".into(), json!("Bank"));
            bag.insert("bank".into(), json!("Nearest"));
        }
        if case == LiveCase::FishBankSoak {
            // The user's 0.2.0.2 configuration: the Catherby site, net
            // fishing, Bank disposition, nearest bank, radius 32.
            bag.insert("location".into(), json!("Site"));
            bag.insert("site".into(), json!(self.site_id()));
        }
        if case == LiveCase::FishGuildApproach {
            bag.insert("location".into(), json!("Site"));
            bag.insert("site".into(), json!("fishing.fishing_guild"));
            bag.insert("disposition".into(), json!("Power"));
        }
        if case.bank_tool_only() || case.is_death_recovery() {
            bag.insert("buyTools".into(), json!(false));
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

    const fn site_id_for_case(self, case: LiveCase) -> &'static str {
        match (self, case) {
            (Self::Fishing, LiveCase::SiteDraynor) => "fishing.draynor_village",
            _ => self.site_id(),
        }
    }

    const fn site_bank_for_case(self, case: LiveCase) -> &'static str {
        match (self, case) {
            (Self::Fishing, LiveCase::SiteDraynor) => "Draynor",
            _ => self.site_bank(),
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
    SiteDraynor,
    CancelBeforeDrain,
    DeathDuringDrop,
    RunKeyChangeDuringDrop,
    RandomEvent,
    DeathReturn,
    DeathRespawnRegion,
    DeathNoStock,
    DeathReturnRefused,
    // Run by the `live-probe` watchdog cells; the unit tests construct both.
    DeathWatchdogPending,
    DeathWatchdogProving,
    FishNet,
    FishSpotMoves,
    FishBaitGate,
    OakRespawn,
    OakNearEdge,
    LocationAuto,
    OakNoPlacementArea,
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
    /// Back-to-back Catherby bank trips at the user's rate: the pack is
    /// refilled with raw shrimp after every return so each trip banks
    /// immediately; a blocked run is recorded and restarted.
    FishBankSoak,
    FishGuildApproach,
    CoinRunes,
    CoinRunesEmpty,
    PowerToBank,
    PauseResumeOtherPlane,
    ReconnectReturn,
    /// A bank trip whose connection is cut at an exact trip step
    /// (`GATHERER_RELOG_STEP`, `GATHERER_RELOG_FAULT`): the trip must restart
    /// in the new session and finish without a new Start.
    BankTripRelog,
    MazeRandom,
}

impl LiveCase {
    const fn is_named_site(self) -> bool {
        matches!(self, Self::Site | Self::SiteDraynor)
    }
    const fn name(self) -> &'static str {
        match self {
            Self::Power => "power",
            Self::Site => "named-site-bank-return",
            Self::SiteDraynor => "draynor-fishing-site-bank-return",
            Self::CancelBeforeDrain => "cancel-before-drain",
            Self::DeathDuringDrop => "death-during-drop",
            Self::RunKeyChangeDuringDrop => "run-key-change-during-drop",
            Self::FishNet => "fish-net",
            Self::FishSpotMoves => "catherby-spot-moves",
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
            Self::OakNoPlacementArea => "oak-no-placement-area",
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
            Self::FishBankSoak => "fish-bank-soak",
            Self::FishGuildApproach => "fish-guild-ardougne",
            Self::CoinRunes => "coin-runes-topup",
            Self::CoinRunesEmpty => "coin-runes-empty-stock",
            Self::PowerToBank => "power-to-bank",
            Self::PauseResumeOtherPlane => "pause-resume-other-plane",
            Self::ReconnectReturn => "reconnect-return",
            Self::BankTripRelog => "bank-trip-relog",
            Self::MazeRandom => "maze-random",
        }
    }

    const fn is_fish_harpoon_bank(self) -> bool {
        matches!(self, Self::FishHarpoonBank | Self::FishHarpoonBankAuto)
    }

    /// Cells whose banked items go through the per-trip conservation
    /// receipt (`BankTripReceipt`): every deposit candidate that leaves the
    /// pack must reappear as a bank increase, the Gatherer's deposited tally
    /// must equal the candidates it held, and protected rows must stay.
    const fn is_bank_mode(self) -> bool {
        matches!(
            self,
            Self::WoodcuttingBank
                | Self::Site
                | Self::SiteDraynor
                | Self::FishSpotMoves
                | Self::WoodcuttingBankUnwieldable
                | Self::BankCost
                | Self::BankCostFirstGoal
                | Self::BankCostSeersMaple
                | Self::BankCostNoCandidate
                | Self::FishBait
                | Self::FishHarpoonBank
                | Self::FishHarpoonBankAuto
                | Self::FishBankSoak
                | Self::CoinRunes
                | Self::CoinRunesEmpty
                | Self::PauseResumeOtherPlane
                | Self::ReconnectReturn
                | Self::BankTripRelog
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
                | Self::BankTripRelog
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
    ReseedTutorial,
    WaitReseedConfirm,
    Seed,
    WaitSeed,
    WaitAutoStart,
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

#[derive(Debug, Clone)]
struct UnsettledItem {
    since: Instant,
    /// `Observation::captured_at` of the observation the item left the pack
    /// in: the cohort a genie-lamp wish confirmation must match.
    left_in: Instant,
    /// Loaded-bank counts from the last observation before the item left the
    /// pack. A later increase conserves it when a non-bank cell's supply trip
    /// deposits it.
    bank_before: Option<BTreeMap<i32, i32>>,
}

/// How long an item that left the pack may stay unaccounted (not back in
/// the pack or equipment, not credited in the bank) before the cell fails.
const UNSETTLED_WINDOW: Duration = Duration::from_secs(2);
/// `macro_genilamp` (`content/pack/obj.pack:2529`), the genie lamp the
/// host's random-event handler rubs (`host/src/random.rs` `LAMP_OBJ`).
const GENIE_LAMP_ID: i32 = 2528;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct FishTarget {
    type_id: usize,
    tile: (i32, i32, i32),
}

#[derive(Debug, Clone, serde::Serialize)]
struct FishSpotMove {
    index: usize,
    from: (i32, i32, i32),
    to: (i32, i32, i32),
    tick: u64,
    xp: i32,
    resumed_tick: Option<u64>,
}

#[derive(Debug, Clone, Default)]
struct FishMovementWitness {
    worked: BTreeMap<usize, FishTarget>,
    moves: Vec<FishSpotMove>,
}

impl FishMovementWitness {
    fn observe(
        &mut self,
        npcs: &BTreeMap<usize, FishTarget>,
        working: Option<usize>,
        tick: u64,
        xp: i32,
    ) {
        for (&index, old) in &mut self.worked {
            if let Some(new) = npcs.get(&index).filter(|new| new.type_id == old.type_id) {
                if new.tile != old.tile {
                    self.moves.push(FishSpotMove {
                        index,
                        from: old.tile,
                        to: new.tile,
                        tick,
                        xp,
                        resumed_tick: None,
                    });
                    *old = *new;
                }
            }
        }
        if let Some((index, target)) =
            working.and_then(|index| npcs.get(&index).map(|row| (index, row)))
        {
            self.worked.clear();
            self.worked.insert(index, *target);
            for movement in &mut self.moves {
                if movement.resumed_tick.is_none() && tick > movement.tick && xp > movement.xp {
                    movement.resumed_tick = Some(tick);
                }
            }
        }
    }

    fn complete(&self) -> bool {
        self.moves
            .iter()
            .filter(|row| row.resumed_tick.is_some())
            .count()
            >= 2
    }
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
    auto_next: Vec<WorldTile>,
    seed_locs: Vec<LocSeed>,
    chop_target: Option<WorldTile>,
    bank_seed: Vec<(String, i32)>,
    inventory_seed: Vec<(String, i32)>,
    reserve_runes: Vec<(i32, i32)>,
    bank_cost_air_banks: Vec<(String, WorldTile)>,
    bank_cost_air_nearest: Option<String>,
    bank_cost_origin: Option<WorldTile>,
    bank_cost_gated_candidates: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FixtureStage {
    Start,
    WaitTile,
    WaitStump,
}

#[derive(Debug, Clone)]
enum FixtureTask {
    Chop {
        tile: WorldTile,
        stage: FixtureStage,
    },
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
    fishing_npcs: BTreeMap<usize, FishTarget>,
    fishing_target_index: Option<usize>,
    live_oaks: BTreeSet<(i32, i32, i32)>,
    bank_booths: BTreeSet<(i32, i32, i32)>,
    oak_stumps: BTreeSet<(i32, i32, i32)>,
    normal_trees: BTreeSet<(i32, i32, i32)>,
    normal_stumps: BTreeSet<(i32, i32, i32)>,
    live_iron_tiles: BTreeSet<(i32, i32, i32)>,
    loc_ids: BTreeSet<(i32, i32, i32, i32)>,
    /// When this observation was captured; observations are captured in
    /// order, once per client drain in `frame`.
    captured_at: Instant,
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

#[derive(Debug, Clone)]
struct ReprovisionDepositEvidence {
    session: u64,
    opened_at: Instant,
    observed_at: Instant,
    expected_items: BTreeMap<i32, i32>,
    bank_before: BTreeMap<i32, i32>,
    bank_after: BTreeMap<i32, i32>,
}

impl ReprovisionDepositEvidence {
    fn accounts_for(&self, id: i32, count: i32, left_in: Instant) -> bool {
        self.opened_at <= left_in
            && left_in <= self.observed_at
            && self.expected_items.get(&id).is_some_and(|expected| {
                count > 0
                    && *expected >= count
                    && self
                        .bank_after
                        .get(&id)
                        .copied()
                        .unwrap_or(0)
                        .saturating_sub(self.bank_before.get(&id).copied().unwrap_or(0))
                        >= *expected
            })
    }

    fn as_json(&self) -> Value {
        json!({
            "session": self.session,
            "expected_items": self.expected_items,
            "bank_before": self.bank_before,
            "bank_after": self.bank_after,
        })
    }
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

    fn withdrawal_observed(
        &self,
        inventory_after: &BTreeMap<i32, i32>,
        bank_after: &BTreeMap<i32, i32>,
    ) -> bool {
        inventory_after.iter().any(|(id, count)| {
            let gained = count.saturating_sub(self.inventory_before.get(id).copied().unwrap_or(0));
            gained > 0
                && self
                    .bank_before
                    .get(id)
                    .copied()
                    .unwrap_or(0)
                    .saturating_sub(bank_after.get(id).copied().unwrap_or(0))
                    == gained
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

/// The recorder path that made a sighting of the selected tool. The first
/// sighting of each kind keeps its source, so a receipt names the durable
/// path that supplied the wield proof.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ToolSightingSource {
    Start,
    Frame,
    Status,
}

impl ToolSightingSource {
    fn as_str(self) -> &'static str {
        match self {
            Self::Start => "start",
            Self::Frame => "frame",
            Self::Status => "status",
        }
    }
}

impl Witness {
    /// Stamps one durable sighting of the selected tool. All three recorder
    /// paths (Start, frame snapshots, status rows) share this counter, so
    /// the first pack-unworn sighting and the first worn sighting compare in
    /// observation order. Only the first sighting of each kind keeps its
    /// index: a later frame or row cannot reorder the proof.
    fn record_tool_sighting(&mut self, source: ToolSightingSource, worn: bool) {
        self.tool_sighting_count = self.tool_sighting_count.saturating_add(1);
        let at = self.tool_sighting_count;
        if worn {
            if self.tool_worn_first.is_none() {
                self.tool_worn_first = Some(at);
                self.tool_worn_source = Some(source);
            }
        } else if self.tool_held_not_worn_first.is_none() {
            self.tool_held_not_worn_first = Some(at);
            self.tool_held_not_worn_source = Some(source);
        }
    }

    /// The wield proof: the first pack-unworn sighting was recorded before
    /// the first worn one. A worn-first shape is rejected even if a bare
    /// row or frame follows it.
    fn tool_wield_proved(&self) -> bool {
        self.tool_held_not_worn_first
            .zip(self.tool_worn_first)
            .is_some_and(|(held, worn)| held < worn)
    }

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

#[derive(Debug, Clone, Copy)]
struct SiteReturnRequest {
    request_id: u64,
    anchor: WorldTile,
    preferred_center: WorldTile,
}

#[derive(Debug, Default, Clone, serde::Serialize)]
struct DeathReturnStopWitness {
    allow_wilderness: Option<bool>,
    permission_evidence_missing: bool,
    respawn_tick: Option<u32>,
    post_respawn_tiles: Vec<(u32, (i32, i32, i32))>,
    observed_through_tick: Option<u32>,
    wilderness_entered: bool,
    tile_evidence_missing: bool,
    terminal_detail: Option<String>,
    terminal_tick: Option<u32>,
    terminal_tile: Option<(i32, i32, i32)>,
    terminal_request_id: Option<u64>,
    latest_request_id: Option<u64>,
    request_retired: bool,
    terminal_stopped: bool,
    quiet_through_tick: Option<u32>,
    walking_after_failure: bool,
    navigation_evidence_missing: bool,
}

impl DeathReturnStopWitness {
    fn observe_permissions(&mut self, configured: Option<bool>, global: bool) {
        if let Some(configured) = configured {
            self.allow_wilderness =
                Some(self.allow_wilderness.unwrap_or(false) || configured || global);
        } else {
            self.permission_evidence_missing = true;
        }
    }

    fn observe_tile(
        &mut self,
        tick: u32,
        tile: Option<(i32, i32, i32)>,
        alive: bool,
        in_wilderness: Option<bool>,
    ) {
        if self.respawn_tick.is_none() {
            if !alive
                || !tile.is_some_and(|(x, z, level)| {
                    script::native::death::RESPAWN_SQUARE.contains(WorldTile { x, z, level })
                })
            {
                return;
            }
            self.respawn_tick = Some(tick);
        }
        self.observed_through_tick = Some(tick);
        match (tile, in_wilderness) {
            (Some(tile), Some(in_wilderness)) => {
                self.wilderness_entered |= in_wilderness;
                if self
                    .post_respawn_tiles
                    .last()
                    .is_none_or(|(_, prior)| *prior != tile)
                {
                    self.post_respawn_tiles.push((tick, tile));
                }
                if self.terminal_tick.is_some_and(|terminal| tick > terminal)
                    && self.terminal_tile != Some(tile)
                {
                    self.walking_after_failure = true;
                }
            }
            _ => self.tile_evidence_missing = true,
        }
    }

    fn observe_terminal(
        &mut self,
        tick: u32,
        tile: Option<(i32, i32, i32)>,
        deaths: u8,
        code: &str,
        detail: &str,
    ) {
        if self.terminal_tick.is_none()
            && self.respawn_tick.is_some()
            && deaths == 1
            && code == "return-failed"
            && detail.contains("walk ended with ")
        {
            self.terminal_tick = Some(tick);
            self.terminal_tile = tile;
            self.terminal_detail = Some(detail.to_owned());
            self.tile_evidence_missing |= tile.is_none();
        }
    }

    fn observe_navigation(&mut self, tick: u32, nav: &Value, stopped: bool) {
        let Some(terminal_tick) = self.terminal_tick else {
            return;
        };
        let request_id = nav.get("request_id").and_then(Value::as_u64);
        self.latest_request_id = request_id;
        if let Some(prior) = self.terminal_request_id {
            self.walking_after_failure |= request_id
                .is_some_and(|current| current != 0 && (self.request_retired || current != prior));
        } else {
            self.terminal_request_id = request_id;
        }
        // AbortWalk retires the completed request to zero. Only a later
        // nonzero request is a rearm; zero must still have no active work.
        self.request_retired |= request_id == Some(0);
        self.navigation_evidence_missing |= request_id.is_none();
        self.terminal_stopped = stopped;
        // The terminal frame may still contain the completed walk's cleanup.
        // Later game ticks must contain neither active work nor a new request.
        if tick > terminal_tick {
            for key in [
                "armed",
                "route",
                "route_work_pending",
                "pending",
                "requested",
                "bank_fetch",
                "carry",
            ] {
                match nav.get(key).and_then(Value::as_bool) {
                    Some(active) => self.walking_after_failure |= active,
                    None => self.navigation_evidence_missing = true,
                }
            }
            if stopped && !self.walking_after_failure && !self.navigation_evidence_missing {
                self.quiet_through_tick = Some(tick);
            }
        }
    }

    fn complete(&self) -> bool {
        self.allow_wilderness == Some(false)
            && !self.permission_evidence_missing
            && self.respawn_tick.is_some()
            && !self.post_respawn_tiles.is_empty()
            && !self.wilderness_entered
            && !self.tile_evidence_missing
            && self.terminal_detail.is_some()
            && self.terminal_tile.is_some()
            && self.terminal_request_id.is_some()
            && self.terminal_stopped
            && !self.walking_after_failure
            && !self.navigation_evidence_missing
            && self
                .terminal_tick
                .zip(self.quiet_through_tick)
                .is_some_and(|(terminal, quiet)| {
                    quiet.saturating_sub(terminal) >= 3
                        && self.observed_through_tick.is_some_and(|tick| tick >= quiet)
                })
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
    last_status_block_reason: Option<String>,
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
    gas_site_observed: bool,
    gas_hazard_observed: bool,
    gas_hazard_escaped: bool,
    fish_targets: BTreeSet<FishTarget>,
    moved_spot_reacquired: bool,
    fish_movement: FishMovementWitness,
    fish_status_sequence: Vec<Value>,
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
    /// First sighting of the selected tool worn, in the monotonic order of
    /// [`Witness::record_tool_sighting`].
    tool_worn_first: Option<u64>,
    /// Which recorder path made the first worn sighting.
    tool_worn_source: Option<ToolSightingSource>,
    /// First sighting of the selected tool in the pack and not worn. The
    /// bank-close status row, a frame between the withdrawal and the wear
    /// action, or the seeded Start pack. The bank fixture banks the axe, so
    /// a correct run records this after the withdrawal; the oracle requires
    /// it before the first worn sighting.
    tool_held_not_worn_first: Option<u64>,
    /// Which recorder path made the first held-not-worn sighting.
    tool_held_not_worn_source: Option<ToolSightingSource>,
    /// Monotonic counter shared by every recorder path, so the two first
    /// sightings above compare across status rows, frames and Start.
    tool_sighting_count: u64,
    /// The selected tool was already worn at Start (after seeding). A run
    /// must fetch and wield it, so the oracle rejects this.
    tool_worn_at_start: bool,
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
    bank_open_login_session: Option<u64>,
    bank_closed_observed: bool,
    bank_return_step_seen: bool,
    site_anchor_selected: bool,
    site_initial_walk: bool,
    site_return_area: bool,
    site_walks: Vec<Value>,
    site_return_request: Option<SiteReturnRequest>,
    site_return_arrival: Option<Value>,
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
    death_respawn_tile: Option<(i32, i32, i32)>,
    death_respawn_region_unchanged: bool,
    death_progress_baselines: BTreeMap<u8, (i32, i32, i64)>,
    death_reprovision_deposits: Vec<ReprovisionDepositEvidence>,
    death_recovered: BTreeSet<u8>,
    death_products_after: BTreeSet<u8>,
    death_xp_after: BTreeSet<u8>,
    death_first_recovery_cycles: Option<u32>,
    death_first_recovery_post_drop_gathers: Option<u32>,
    death_haul_between: bool,
    death_return_stop: DeathReturnStopWitness,
    death_no_stock_blocked_running: bool,
    death_watchdog_aged: bool,
    death_watchdog_generation_before: Option<u64>,
    death_watchdog_recreated: bool,
    last_deaths: u8,
    last_recoveries: u8,
    last_recovery_step: u8,
    soak: SoakWitness,
    /// The engine's wish confirmation for a rubbed genie lamp ("Your wish
    /// has been granted!", `xplamp.rs2:106-114`, after `inv_del` of
    /// `macro_genilamp`) not yet matched to a lamp that left the pack.
    /// Stamped by `frame` after its client drain's observation was
    /// captured, so it belongs to that observation; one lamp consumes it.
    last_wish_granted: Option<Instant>,
    /// `BankTripRelog`: the cut was made, at this published trip step.
    relog_fired: bool,
    relog_step: Option<String>,
    /// The client's session counter when the cut was made, and whether a
    /// later session reached the world again.
    relog_session_before: Option<u64>,
    relog_session_advanced: bool,
    relog_offline_observed: bool,
    /// Published trip steps after the new session, without repeats.
    relog_steps_after: Vec<String>,
    /// `BankTripRelog` at Access: the character's tile on the first frame of
    /// the walk to the bank.
    relog_access_origin: Option<(i32, i32, i32)>,
}

/// Bank-trip soak counters (`LiveCase::FishBankSoak`). Trips are summed
/// across restarted runs; every block carries the Gatherer status and the
/// recent client-side bank facts (modal ids, inventory packet generations)
/// so the server event order around the failure is part of the receipt.
#[derive(Debug, Default, Clone)]
struct SoakWitness {
    /// Completed bank trips (the Gatherer's `trips` counter, summed over runs).
    trips: u32,
    /// Trips reported by the current run so far.
    run_trips: u32,
    /// Runs started by the cell (the first Start plus every restart).
    runs: u32,
    /// Blocked runs whose failure code was `bank-deposit-failed`.
    deposit_blocks: u32,
    /// Blocked runs with any other failure code.
    other_blocks: u32,
    /// One row per block: tick, code, message, status and the facts ring.
    blocks: Vec<Value>,
    /// A block was observed on the current run and its restart is pending.
    block_pending: bool,
    /// Run key of the last recorded block, so a repeated status is not a new block.
    blocked_run: Option<String>,
    /// Run key currently publishing status.
    current_run: Option<String>,
    /// Rolling window of the last client-side bank facts rows.
    facts: std::collections::VecDeque<Value>,
    /// Last facts row, for change detection.
    last_facts: Option<Value>,
    /// Shrimp top-ups sent after a return.
    refills: u32,
    /// Accepted host auto-run `set_run(true)` sends (`SlotStatus::run_sends`),
    /// mirrored by the main loop. The run-on orb's server script runs
    /// `if_close` (`player_controls.rs2:34-35`).
    run_sends: u32,
    /// Last status phase, event and bank label as published.
    last_phase: Option<String>,
    last_bank: Option<String>,
    last_event: Option<String>,
    /// Last status tick seen, for the facts rows.
    last_status_tick: Option<u64>,
}

const SOAK_FACTS_WINDOW: usize = 80;
/// Raw shrimp seeded before Start and after each return: the net takes the
/// 28th slot, so the pack is full and the next bank trip is due at once.
const SOAK_SEED_FISH: usize = 27;
const SOAK_REFILL_GAP: Duration = Duration::from_secs(4);

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
    /// The loaded bank counts from the previous observation. A non-bank cell
    /// (a death recovery's supply trip) can deposit a non-product item, and
    /// that deposit must conserve it.
    bank_counts_before: Option<BTreeMap<i32, i32>>,
    bank_config: Option<Arc<script::native::PreparedConfig>>,
    last_observed_dropped: i64,
    last_observed_product_drops: u32,
    witness: Witness,
    error: Option<String>,
    pause_plane_teleport_sent: bool,
    tutorial_reseed: Option<scenario::tutorial::PostRelogTutorial>,
    soak_last_refill: Instant,
    relog: RelogPlan,
    /// The cut is a silent server (`RelogPlan::dead_server`): the main loop
    /// owns the relay and stops forwarding when this is set.
    relog_pause_requested: bool,
    /// `BankTripRelog`: the preparation sent the mainland hop itself.
    relog_prep_hop_sent: bool,
    /// The slot's arm. The preparation relogs a bot that has no script yet,
    /// so the approved scriptless policy parks it after its own logout; the
    /// fixture posts the operator's Login, as a user would.
    login_arm: Option<Arc<SlotArm>>,
}
/// How far the character walks toward the bank before an `Access` cut. The
/// oak fixture (3017,3170) is 75 tiles from the Draynor bank (3092,3243), so
/// 20 tiles leaves the walk well under way and the bank well ahead.
const RELOG_ACCESS_CUT_TILES: i32 = 20;

/// Where a `BankTripRelog` cell cuts the connection, and how.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RelogPlan {
    /// The Gatherer's published trip step at the cut: `Access`, `Open` or `Deposit`.
    step: &'static str,
    /// `false`: an unknown opcode fails the client's own dispatch (the T1
    /// path of `relog_live_tests.rs`) and a fresh login follows. `true`: a
    /// relay stops forwarding the server's bytes, so the client's dead-server
    /// deadline fires and a soft reconnect follows.
    dead_server: bool,
}

impl RelogPlan {
    const DEFAULT: Self = Self {
        step: "Open",
        dead_server: false,
    };
}

fn relog_plan() -> Result<RelogPlan, String> {
    let step = match std::env::var("GATHERER_RELOG_STEP").as_deref() {
        Ok("Open") | Err(_) => "Open",
        Ok("Access") => "Access",
        Ok("Deposit") => "Deposit",
        Ok(other) => {
            return Err(format!(
                "GATHERER_RELOG_STEP must be Access, Open or Deposit, not {other:?}"
            ))
        }
    };
    let dead_server = match std::env::var("GATHERER_RELOG_FAULT").as_deref() {
        Ok("protocol-error") | Err(_) => false,
        Ok("dead-server") => true,
        Ok(other) => {
            return Err(format!(
                "GATHERER_RELOG_FAULT must be protocol-error or dead-server, not {other:?}"
            ))
        }
    };
    Ok(RelogPlan { step, dead_server })
}

/// The published trip steps after the new session include `step`. The status
/// labels "withdrawal confirmed" and "bank closed; validating equipment" last
/// one tick. In one run of twelve the Gatherer's own log shows the close label
/// and the witness did not record it. The Close and Validate steps follow those
/// labels and the Gatherer reaches them only when the withdrawal and the close
/// completed, so the step trail states the same facts.
fn relog_step_seen(steps: &[String], step: &str) -> bool {
    steps.iter().any(|seen| seen == step)
}

/// The trip steps a restarted bank trip must publish, in this order, after
/// the new session (other steps may lie between).
fn relog_steps_in_order(steps: &[String]) -> bool {
    let mut wanted = ["Access", "Open", "Deposit"].into_iter();
    let mut next = wanted.next();
    for step in steps {
        if next == Some(step.as_str()) {
            next = wanted.next();
        }
    }
    next.is_none()
}

/// Host-log lines of one `BankTripRelog` run: the Gatherer's recovery lines,
/// the bank machine's reopen lines and the session lines around the cut.
struct RelogCapture(Mutex<Vec<String>>);

static RELOG_CAPTURE: RelogCapture = RelogCapture(Mutex::new(Vec::new()));

impl api::hostlog::Sink for RelogCapture {
    fn record(&self, record: &api::hostlog::Record<'_>) {
        let message = record.message;
        if [
            "gatherer-bank-",
            "bank-reopen",
            "session exit",
            "relog decision",
            "unexpected logout",
            "handshake ok",
            "login code",
            "native Gatherer",
        ]
        .iter()
        .any(|prefix| message.starts_with(prefix))
        {
            if let Ok(mut lines) = self.0.lock() {
                lines.push(format!("{} {message}", record.slot.unwrap_or("-")));
            }
        }
    }
}

fn relog_log() -> Vec<String> {
    RELOG_CAPTURE
        .0
        .lock()
        .map(|lines| lines.clone())
        .unwrap_or_default()
}

/// The Gatherer logged the restart of its bank trip at `step`.
fn relog_restart_logged(step: &str) -> bool {
    let needle = format!("step={step} ");
    relog_log()
        .iter()
        .any(|line| line.contains("gatherer-bank-trip-restart") && line.contains(&needle))
}

/// Test-only relay between the client and the engine, as in
/// `relog_live_tests.rs`: `pause_live_connection` stops the server-to-client
/// direction of the game connection that is live when it is called; the
/// reconnect passes through. The client also opens update-service
/// connections to the same port, so the game connection is the newest one
/// whose first byte is the login opcode 14 (`client.rs:3530`), not the newest
/// connection.
mod relog_relay {
    use std::io::{Read, Write};
    use std::net::{Shutdown, TcpListener, TcpStream};
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::thread;

    const LOGIN_OPCODE: u8 = 14;

    pub struct Relay {
        pub port: u16,
        game: Arc<AtomicUsize>,
        paused: Arc<AtomicUsize>,
        stop: Arc<AtomicBool>,
    }

    impl Relay {
        pub fn start(upstream: u16) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").expect("bind relay");
            let port = listener.local_addr().unwrap().port();
            let game = Arc::new(AtomicUsize::new(0));
            let paused = Arc::new(AtomicUsize::new(0));
            let stop = Arc::new(AtomicBool::new(false));
            let (game_accept, paused_accept, stop_accept) =
                (Arc::clone(&game), Arc::clone(&paused), Arc::clone(&stop));
            thread::spawn(move || {
                let mut next = 0;
                for incoming in listener.incoming() {
                    if stop_accept.load(Ordering::Relaxed) {
                        return;
                    }
                    let Ok(client) = incoming else { continue };
                    let Ok(server) = TcpStream::connect(("127.0.0.1", upstream)) else {
                        let _ = client.shutdown(Shutdown::Both);
                        continue;
                    };
                    next += 1;
                    let id = next;
                    pump(
                        client.try_clone().unwrap(),
                        server.try_clone().unwrap(),
                        Direction::ToServer {
                            id,
                            game: Arc::clone(&game_accept),
                        },
                    );
                    pump(
                        server,
                        client,
                        Direction::ToClient {
                            id,
                            paused: Arc::clone(&paused_accept),
                        },
                    );
                }
            });
            Self {
                port,
                game,
                paused,
                stop,
            }
        }

        pub fn pause_live_connection(&self) -> usize {
            let id = self.game.load(Ordering::Relaxed);
            self.paused.store(id, Ordering::Relaxed);
            id
        }
    }

    impl Drop for Relay {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::Relaxed);
            let _ = TcpStream::connect(("127.0.0.1", self.port));
        }
    }

    enum Direction {
        /// Client to server: the first byte of a connection names its service.
        ToServer { id: usize, game: Arc<AtomicUsize> },
        /// Server to client: bytes are dropped while this connection is paused.
        ToClient { id: usize, paused: Arc<AtomicUsize> },
    }

    /// Copy `from` to `to`. While the paused id is this connection's, the
    /// client hears nothing until it gives up, and its close ends the other
    /// direction.
    fn pump(mut from: TcpStream, mut to: TcpStream, direction: Direction) {
        thread::spawn(move || {
            let mut buf = [0u8; 16 * 1024];
            let mut first = true;
            loop {
                let n = match from.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => n,
                };
                match &direction {
                    Direction::ToServer { id, game } => {
                        if first && buf[0] == LOGIN_OPCODE {
                            game.store(*id, Ordering::Relaxed);
                        }
                        first = false;
                    }
                    Direction::ToClient { id, paused } => {
                        if paused.load(Ordering::Relaxed) == *id {
                            continue;
                        }
                    }
                }
                if to.write_all(&buf[..n]).is_err() {
                    break;
                }
            }
            let _ = from.shutdown(Shutdown::Both);
            let _ = to.shutdown(Shutdown::Both);
        });
    }
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
            bank_counts_before: None,
            bank_config: None,
            last_observed_dropped: 0,
            last_observed_product_drops: 0,
            witness: Witness::default(),
            error: None,
            pause_plane_teleport_sent: false,
            tutorial_reseed: None,
            soak_last_refill: Instant::now(),
            relog: relog_plan().unwrap_or(RelogPlan::DEFAULT),
            relog_pause_requested: false,
            relog_prep_hop_sent: false,
            login_arm: None,
        }
    }

    /// `BankTripRelog`: cut the connection the first frame the published trip
    /// step is the planned one (`Access`: the walk to the bank is
    /// `RELOG_ACCESS_CUT_TILES` under way; `Open`: before the table loads;
    /// `Deposit`: with the table loaded), then follow the steps the new
    /// session publishes.
    fn relog_frame(&mut self, client: &mut client::client::Client) {
        let step = self
            .witness
            .last_status_bank
            .as_deref()
            .and_then(|bank| bank.rsplit_once("; "))
            .map(|(_, step)| step.to_owned());
        if !self.witness.relog_fired {
            if !client.ingame || step.as_deref() != Some(self.relog.step) {
                return;
            }
            let at_boundary = match self.relog.step {
                "Open" => !self.snapshot.bank_loaded(),
                "Access" => {
                    // Mid-walk: the character has left the tile it stood on
                    // when the walk to the bank began.
                    let Some(here) = self.latest.as_ref().and_then(|row| row.tile) else {
                        return;
                    };
                    let origin = *self.witness.relog_access_origin.get_or_insert(here);
                    here.2 == origin.2
                        && (here.0 - origin.0).abs().max((here.1 - origin.1).abs())
                            >= RELOG_ACCESS_CUT_TILES
                }
                _ => self.snapshot.bank_loaded(),
            };
            if !at_boundary {
                return;
            }
            self.witness.relog_fired = true;
            self.witness.relog_step = step;
            self.witness.relog_session_before = Some(client.gens.session);
            if self.relog.dead_server {
                self.relog_pause_requested = true;
            } else {
                // The decoded opcode DIAG-PROTO-74 saw: 289 size 0, no
                // handler. Dispatched through the client's own seam.
                let mut payload = client::io::Packet::new(vec![]);
                client.ptype = 74;
                client.psize = 0;
                client.handle_packet(74, &mut payload);
            }
            println!(
                "gatherer-bank-relog-cut {}",
                json!({
                    "step": self.witness.relog_step,
                    "dead_server": self.relog.dead_server,
                    "session": client.gens.session,
                    "bank_loaded": self.snapshot.bank_loaded(),
                    "tile": self.latest.as_ref().and_then(|row| row.tile),
                })
            );
            return;
        }
        let before = self.witness.relog_session_before.unwrap_or(0);
        if !client.ingame {
            self.witness.relog_offline_observed = true;
        }
        if client.ingame && client.gens.session > before {
            self.witness.relog_session_advanced = true;
        }
        if self.witness.relog_session_advanced {
            if let Some(step) = step {
                if self.witness.relog_steps_after.last() != Some(&step) {
                    self.witness.relog_steps_after.push(step);
                }
            }
        }
    }

    fn is_real_content_death_reprovision(&self) -> bool {
        self.case.is_death_recovery()
            && self.plan.seed_locs.is_empty()
            && self.plan.oak_tiles.is_empty()
    }

    fn observe_reprovision_bank(&mut self, observation: &Observation) -> Result<(), String> {
        if !self.is_real_content_death_reprovision() || !self.snapshot.bank_loaded() {
            return Ok(());
        }
        let session = self.snapshot.bank_session_generation();
        let bank_after = self.snapshot_bank_counts();
        if let Some(evidence) = self
            .witness
            .death_reprovision_deposits
            .last_mut()
            .filter(|evidence| evidence.session == session)
        {
            evidence.bank_after = bank_after;
            evidence.observed_at = observation.captured_at;
        } else {
            let expected_items = bank_deposit_expectation(
                &self.snapshot_inventory_counts(),
                &self.native_deposit_ids()?,
            );
            self.witness
                .death_reprovision_deposits
                .push(ReprovisionDepositEvidence {
                    session,
                    opened_at: observation.captured_at,
                    observed_at: observation.captured_at,
                    expected_items,
                    bank_before: bank_after.clone(),
                    bank_after,
                });
        }
        Ok(())
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
                self.expected_preflight_build()
                    && latest.build.is_some_and(|build| {
                        self.plan.edge_oak.is_some_and(|tile| {
                            build.outside_gap(tile_key(tile)).is_some_and(|gap| gap > 0)
                                && !latest.live_oaks.contains(&tile_key(tile))
                        }) && latest
                            .tile
                            .and_then(|tile| build.edge_distance(tile))
                            .is_some_and(|distance| distance <= 20)
                    })
            }
            LiveCase::GasHazard => latest.live_iron_tiles.contains(&tile_key(self.target)),
            LiveCase::OakNoPlacementArea => {
                latest.bank_booths.iter().any(|&booth| {
                    tile_distance(booth, self.target).is_some_and(|distance| distance <= 1)
                        && tile_distance(booth, OAK_NO_PLACEMENT_BANK_BOOTH)
                            .is_some_and(|distance| distance <= 1)
                }) && latest
                    .live_oaks
                    .iter()
                    .all(|&oak| tile_distance(oak, self.target).is_none_or(|distance| distance > 2))
            }
            LiveCase::LocationAuto => {
                latest
                    .build
                    .is_some_and(|build| build.contains(tile_key(self.target)))
                    && latest.normal_trees.iter().all(|&tree| {
                        tile_distance(tree, self.target).is_none_or(|distance| distance > 32)
                    })
                    && self.plan.auto_next.iter().any(|tile| {
                        latest.normal_trees.contains(&tile_key(*tile))
                            && tile_distance(tile_key(self.target), *tile)
                                .is_some_and(|distance| (33..=128).contains(&distance))
                    })
            }
            _ => true,
        }
    }

    fn observe_auto_start_area(&mut self) -> Result<(), String> {
        let Some(latest) = self.latest.as_ref() else {
            return Err("Auto fixture has no real-content observation".into());
        };
        if !latest
            .build
            .is_some_and(|build| build.contains(tile_key(self.target)))
        {
            return Err(format!(
                "Auto start {:?} is outside the observed scene",
                tile_key(self.target)
            ));
        }
        if latest
            .normal_trees
            .iter()
            .any(|&tree| tile_distance(tree, self.target).is_some_and(|distance| distance <= 32))
        {
            return Err(format!(
                "Auto start {:?} has a real normal tree inside radius 32",
                tile_key(self.target)
            ));
        }
        let scene = self.snapshot.scene();
        let approach_options = auto_approach_options(scene.width, scene.height);
        let start_query = api::query::SceneQuery::new(scene, Some(self.target));
        if !start_query.walkable(self.target) {
            return Err(format!(
                "Auto start {:?} is not a standable scene tile",
                tile_key(self.target)
            ));
        }
        let next = auto_next_group(self.target, &latest.normal_trees)
            .into_iter()
            .filter(|&tree| start_query.can_reach(tree, &approach_options))
            .collect::<Vec<_>>();
        if next.is_empty() {
            return Err(format!(
                "Auto start {:?} has no reachable real normal tree with an adjacent approach at distance 33..=128",
                tile_key(self.target)
            ));
        }
        self.plan.auto_next = next;
        Ok(())
    }

    fn ready_to_start(&self) -> bool {
        self.phase == Prep::Ready && self.fixture_done && self.fixture_ready()
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
        let fishing_npcs = if self.case == LiveCase::FishSpotMoves {
            self.snapshot
                .npcs()
                .iter()
                .filter_map(|npc| {
                    (npc.r#type == Some(320)).then_some((
                        npc.index,
                        FishTarget {
                            type_id: 320,
                            tile: (npc.tile.x, npc.tile.z, npc.tile.level),
                        },
                    ))
                })
                .collect()
        } else {
            BTreeMap::new()
        };
        let fishing_target_index = fishing_target.and_then(|_| {
            self.snapshot
                .local_player()?
                .player
                .actor
                .target
                .map(|target| target.index)
        });
        let locs = self.snapshot.locs();
        let bank_booths = locs
            .iter()
            .filter(|loc| loc.name.as_deref() == Some("Bank booth"))
            .map(|loc| (loc.tile.x, loc.tile.z, loc.tile.level))
            .collect();
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
            fishing_npcs,
            fishing_target_index,
            live_oaks,
            bank_booths,
            oak_stumps,
            normal_trees,
            normal_stumps,
            live_iron_tiles,
            loc_ids,
            captured_at: Instant::now(),
        }
    }

    fn frame(&mut self, client: &mut client::client::Client, hold: bool) {
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
        if self.case == LiveCase::BankTripRelog {
            self.relog_frame(client);
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
            if self
                .snapshot
                .chat_modal_texts()
                .iter()
                .any(|text| text.contains("Your wish has been granted"))
            {
                self.witness.last_wish_granted = Some(Instant::now());
            }
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
        if self.case == LiveCase::FishBankSoak && self.started {
            self.record_soak_facts(client, &observation);
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
                if let Err(error) = self.advance_fixture(client) {
                    self.error = Some(error);
                }
            }
            if self.case == LiveCase::FishBankSoak {
                if let Err(error) = self.soak_refill(client) {
                    self.error = Some(error);
                }
            }
        } else if self.fixture_task.is_some() && self.phase == Prep::Ready {
            if let Err(error) = self.advance_fixture(client) {
                self.error = Some(error);
            }
        } else if let Err(error) = self.advance_prep(client) {
            self.error = Some(error);
        }
    }

    /// One row of client-side bank facts per change: the modal ids, the bank
    /// withdraw component's inventory packet generations and transmit flag,
    /// the main-modal open/close packet observations, the host's loaded /
    /// session facts and the pack. Printed as `gatherer-bank-facts` lines and
    /// kept in a rolling window attached to every recorded block.
    fn record_soak_facts(&mut self, client: &client::client::Client, observation: &Observation) {
        let bank_com = self.snapshot.bank_component_id();
        let inv_state = (bank_com >= 0)
            .then(|| client.inventory_packet_state(bank_com))
            .flatten();
        let modal = client.main_modal_packet_state();
        let row = json!({
            "main": client.main_modal_id,
            "side": client.side_modal_id,
            "bank_com": bank_com,
            "loaded": self.snapshot.bank_loaded(),
            "session": self.snapshot.bank_session_generation(),
            "bank_rows": self.snapshot.bank().len(),
            "side_rows": self.snapshot.bank_side().len(),
            "inv_gen": inv_state.map(|state| state.generation),
            "inv_full_gen": inv_state.map(|state| state.full_generation),
            "inv_transmitting": inv_state.map(|state| state.transmitting),
            "inv_full_obs": inv_state.map(|state| state.full_observation),
            "modal_gen": modal.generation,
            "modal_opened_obs": modal.opened_observation,
            "modal_closed_obs": modal.closed_observation,
            "pack": observation.inventory.len(),
            "energy": client.runenergy,
            "run_orb": run_orb_echo(client),
            "run_sends": self.witness.soak.run_sends,
            "products": observation.product_count,
            "phase": self.witness.soak.last_phase,
            "bank": self.witness.soak.last_bank,
            "event": self.witness.soak.last_event,
        });
        if self.witness.soak.last_facts.as_ref() == Some(&row) {
            return;
        }
        let stamped = json!({
            "tick": observation.tick,
            "native_tick": self.native_tick,
            "status_tick": self.witness.soak.last_status_tick,
            "tile": observation.tile,
            "facts": row,
        });
        println!("gatherer-bank-facts {stamped}");
        self.witness.soak.last_facts = Some(row);
        let facts = &mut self.witness.soak.facts;
        if facts.len() >= SOAK_FACTS_WINDOW {
            facts.pop_front();
        }
        facts.push_back(stamped);
    }

    /// Refill the pack with raw shrimp once the Gatherer is back from a trip
    /// (its bank label is `Idle` and no bank is open) so the next trip is due
    /// immediately. Only the account's own inventory is seeded.
    fn soak_refill(&mut self, client: &mut client::client::Client) -> Result<(), String> {
        let soak = &self.witness.soak;
        if soak.block_pending
            || self.snapshot.bank_component_id() >= 0
            || self.soak_last_refill.elapsed() < SOAK_REFILL_GAP
        {
            return Ok(());
        }
        let idle = soak
            .last_bank
            .as_deref()
            .is_none_or(|bank| bank == "—" || bank.ends_with("; Idle"));
        let working = soak
            .last_phase
            .as_deref()
            .is_some_and(|phase| matches!(phase, "gathering" | "waiting" | "walking"));
        if !idle || !working {
            return Ok(());
        }
        let Some(latest) = self.latest.as_ref() else {
            return Ok(());
        };
        if !latest.inventory.values().any(|row| row.id == self.tool_id) {
            return Ok(());
        }
        let free = 28usize.saturating_sub(latest.inventory.len());
        if free == 0 {
            return Ok(());
        }
        let (shrimp, anchovies) = if soak_two_products() {
            let free = i32::try_from(free).unwrap_or(i32::MAX);
            (free - free / 2, free / 2)
        } else {
            (i32::try_from(free).unwrap_or(i32::MAX), 0)
        };
        send_cheat(client, &format!("give raw_shrimp {shrimp}"))?;
        if anchovies > 0 {
            send_cheat(client, &format!("give raw_anchovies {anchovies}"))?;
        }
        self.soak_last_refill = Instant::now();
        self.witness.soak.refills = self.witness.soak.refills.saturating_add(1);
        println!(
            "gatherer-bank-soak-refill {}",
            json!({
                "tick": latest.tick,
                "native_tick": self.native_tick,
                "free": free,
                "refills": self.witness.soak.refills,
                "trips": self.witness.soak.trips,
            })
        );
        Ok(())
    }

    /// Sum the Gatherer's per-run `trips` counter across restarted runs and
    /// keep the latest phase / bank label / event for the facts rows.
    fn apply_soak_status(&mut self, status: &script::native::ScriptStatus) {
        let run = format!("{:?}", status.run);
        let trips = integer_field(status, "trips")
            .and_then(|value| u32::try_from(value).ok())
            .unwrap_or(0);
        let tick = self.latest.as_ref().map(|latest| latest.tick);
        let soak = &mut self.witness.soak;
        if soak.current_run.as_deref() != Some(run.as_str()) {
            soak.current_run = Some(run);
            soak.run_trips = 0;
            soak.runs = soak.runs.saturating_add(1);
        }
        if trips > soak.run_trips {
            soak.trips = soak.trips.saturating_add(trips - soak.run_trips);
            soak.run_trips = trips;
            println!(
                "gatherer-bank-soak-trip {}",
                json!({
                    "tick": tick,
                    "trips": soak.trips,
                    "run_trips": trips,
                    "runs": soak.runs,
                    "deposit_blocks": soak.deposit_blocks,
                    "other_blocks": soak.other_blocks,
                })
            );
        }
        soak.last_phase = text_field(status, "phase").map(str::to_owned);
        soak.last_bank = text_field(status, "bank").map(str::to_owned);
        soak.last_event = text_field(status, "last_event").map(str::to_owned);
        soak.last_status_tick = tick.map(u64::from);
    }

    /// Record one block per run with the status and the facts window; the
    /// main loop restarts the run once `block_pending` is set. The block is
    /// the recorded failure of its trip: the trip's conservation receipt
    /// and the items it left unsettled are dropped, so the soak keeps
    /// counting blocks instead of failing on the aborted trip's accounting.
    fn record_soak_block(
        &mut self,
        status: &script::native::ScriptStatus,
        code: &str,
        message: &str,
    ) {
        let run = format!("{:?}", status.run);
        if self.witness.soak.blocked_run.as_deref() == Some(run.as_str()) {
            return;
        }
        let row = json!({
            "tick": self.latest.as_ref().map(|latest| latest.tick),
            "native_tick": self.native_tick,
            "run": run,
            "runs": self.witness.soak.runs,
            "trips_before": self.witness.soak.trips,
            "phase": text_field(status, "phase"),
            "event": text_field(status, "last_event"),
            "bank": text_field(status, "bank"),
            "tile": self.latest.as_ref().and_then(|latest| latest.tile),
            "failure_code": code,
            "failure_message": message,
            "facts": self.witness.soak.facts.iter().cloned().collect::<Vec<_>>(),
        });
        println!("gatherer-bank-soak-block {row}");
        let soak = &mut self.witness.soak;
        if code == "bank-deposit-failed" {
            soak.deposit_blocks = soak.deposit_blocks.saturating_add(1);
        } else {
            soak.other_blocks = soak.other_blocks.saturating_add(1);
        }
        soak.blocks.push(row);
        soak.blocked_run = Some(run);
        soak.block_pending = true;
        self.witness.pending_bank_trip = None;
        self.witness.waiting_for_return_yield = false;
        self.witness.waiting_for_bank_yield = false;
        self.unsettled_items.clear();
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

    fn advance_fixture(&mut self, client: &mut client::client::Client) -> Result<(), String> {
        let Some(FixtureTask::Chop { tile, mut stage }) = self.fixture_task.take() else {
            self.fixture_done = true;
            return Ok(());
        };
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
                    send_cheat(client, &interact::tele_args(stand.level, stand.x, stand.z))?;
                    stage = FixtureStage::WaitTile;
                } else if let Some(loc) = self.snapshot.locs().iter().find(|loc| {
                    loc.id == OAK_ID
                        && loc.tile
                            == (WorldTile {
                                x: tile.x,
                                z: tile.z,
                                level: tile.level,
                            })
                }) {
                    if !matches!(
                        Interactions::new(&self.snapshot, client)
                            .interact(OpTarget::Loc(loc), ActionSpec::Label("Chop down".into()),),
                        SendResult::Sent { .. }
                    ) {
                        return Err(format!("fixture Chop down was refused at {tile:?}"));
                    }
                    stage = FixtureStage::WaitStump;
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
        }
        self.fixture_task = Some(FixtureTask::Chop { tile, stage });
        Ok(())
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
        self.observe_reprovision_bank(&observation)?;
        self.observe_bank_open(self.witness.status_deposited)?;
        self.observe_bank_withdrawal();
        let bank_active = self.bank_conservation_active();
        let drop_baseline = self.last_observed_dropped;
        let product_drop_baseline = self.last_observed_product_drops;
        // Inventory, equipment and bank counts arrive in separate packets.
        // Every banked item settles only after its expected bank increase;
        // protected tools still must reappear in inventory or equipment.
        let bank_counts = if !self.unsettled_items.is_empty() && self.snapshot.bank_loaded() {
            self.snapshot_bank_counts()
        } else {
            BTreeMap::new()
        };
        // A genie lamp the host's own handler rubbed (`host/src/random.rs`
        // `LAMP_OBJ`) leaves the pack by `inv_del` and the same engine
        // script confirms the wish (`xplamp.rs2:106-114`). Only a
        // confirmation from the observation the lamp left in or a later one
        // settles it. `frame` stamps a confirmation after capturing its own
        // observation, so that holds exactly when the stamp is no earlier
        // than the lamp's `left_in`. Each confirmation settles one lamp.
        let mut wish_granted = self.witness.last_wish_granted.take();
        self.unsettled_items.retain(|&(id, count), unsettled| {
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
                && !self
                    .witness
                    .death_reprovision_deposits
                    .iter()
                    .any(|evidence| evidence.accounts_for(id, count, unsettled.left_in))
                && !(!bank_active
                    && unsettled.bank_before.as_ref().is_some_and(|before| {
                        bank_counts
                            .get(&id)
                            .copied()
                            .unwrap_or(0)
                            .saturating_sub(before.get(&id).copied().unwrap_or(0))
                            >= count
                    }))
                && !(id == GENIE_LAMP_ID
                    && wish_granted
                        .take_if(|granted| *granted >= unsettled.left_in)
                        .is_some())
        });
        self.witness.last_wish_granted = wish_granted;
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
                        if bank_active
                            || (self.is_real_content_death_reprovision()
                                && self.snapshot.bank_loaded())
                        {
                            self.unsettled_items.entry((old.id, old.count)).or_insert(
                                UnsettledItem {
                                    since: Instant::now(),
                                    left_in: observation.captured_at,
                                    bank_before: self.bank_counts_before.clone(),
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
                                left_in: observation.captured_at,
                                bank_before: self.bank_counts_before.clone(),
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
            .find(|(_, item)| item.since.elapsed() >= UNSETTLED_WINDOW)
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
            // Stamp this frame's durable sighting. The pack state and the
            // equipment state are exclusive here, so one frame makes at most
            // one sighting and the shared counter keeps their order.
            let worn = self
                .snapshot
                .equipment()
                .iter()
                .any(|item| item.def.id == self.tool_id);
            if worn {
                self.witness
                    .record_tool_sighting(ToolSightingSource::Frame, true);
            } else if self
                .snapshot
                .inventory()
                .iter()
                .any(|item| item.def.id == self.tool_id && item.count > 0)
            {
                self.witness
                    .record_tool_sighting(ToolSightingSource::Frame, false);
            }
            if self.case == LiveCase::WoodcuttingBankUnwieldable
                && self.witness.tool_worn_first.is_some()
            {
                return Err("unwieldable rune axe was equipped".into());
            }
        }
        if self.snapshot.bank_loaded() {
            self.bank_counts_before = Some(self.snapshot_bank_counts());
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
        if self.case == LiveCase::FishSpotMoves {
            self.witness.fish_movement.observe(
                &observation.fishing_npcs,
                observation.fishing_target_index,
                self.native_tick,
                observation.xp,
            );
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
            if observation.live_iron_tiles.contains(&tile_key(self.target))
                && !self.witness.gas_site_observed
            {
                self.witness.gas_site_observed = true;
                self.witness.gas_hazard_tile = Some(tile_key(self.target));
                self.witness.gas_hazard_xp_start = observation.xp;
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

    fn observe_bank_open(&mut self, current_deposited: i64) -> Result<(), String> {
        if !self.bank_conservation_active()
            || !self.snapshot.bank_loaded()
            || self.witness.pending_bank_trip.is_some()
        {
            return Ok(());
        }
        let inventory_before = self.snapshot_inventory_counts();
        let deposit_ids = self.native_deposit_ids()?;
        if self.case == LiveCase::FishHarpoonBank && self.witness.bank_trips.is_empty() {
            for (id, label) in [(SPARE_NET_ID, "spare net"), (BEER_ID, "beer")] {
                if inventory_before.get(&id) != Some(&1) || !deposit_ids.contains(&id) {
                    return Err(format!(
                        "{} did not prepare the seeded {label} as a deposit candidate: inventory={inventory_before:?} deposit_ids={deposit_ids:?}",
                        self.name()
                    ));
                }
            }
        }
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
        self.witness.pending_bank_trip = Some(BankTripReceipt::new(
            self.witness.status_trips.saturating_add(1),
            inventory_before,
            &deposit_ids,
            self.snapshot_bank_counts(),
            current_deposited,
            self.cell.products(self.case),
            seeded_casket_expected,
        ));
        Ok(())
    }

    fn observe_bank_withdrawal(&mut self) {
        if self.witness.bank_withdrawal_confirmed || !self.snapshot.bank_loaded() {
            return;
        }
        let Some(receipt) = self.witness.pending_bank_trip.as_ref() else {
            return;
        };
        self.witness.bank_withdrawal_confirmed = receipt.withdrawal_observed(
            &self.snapshot_inventory_counts(),
            &self.snapshot_bank_counts(),
        );
    }

    fn status_claims_bank_close(&self, status: &script::native::ScriptStatus) -> bool {
        // The acquisition completion event also follows wear-only requests.
        // A missing bank field does not prove that no bank was involved.
        text_field(status, "last_event") == Some("bank closed; validating equipment")
            && (text_field(status, "bank") != Some("—")
                || self.witness.pending_bank_trip.is_some()
                || self.snapshot.bank_loaded())
    }

    fn observe_bank_deposit(
        &mut self,
        status: &script::native::ScriptStatus,
        current_deposited: i64,
    ) -> Result<(), String> {
        if !self.bank_conservation_active() {
            return Ok(());
        }
        self.observe_bank_open(current_deposited)?;
        self.observe_bank_withdrawal();
        let event = text_field(status, "last_event");
        if event == Some("deposit confirmed") && self.witness.pending_bank_trip.is_none() {
            return Err(format!(
                "{} confirmed a deposit without pre-deposit inventory evidence",
                self.name()
            ));
        }

        // The core skips an empty deposit, including its status and event.
        // Its observed pack and bank still prove the zero-item boundary.
        let empty_deposit = self
            .witness
            .pending_bank_trip
            .as_ref()
            .is_some_and(|receipt| receipt.expected_items.is_empty());
        let should_observe_deposit =
            self.witness
                .pending_bank_trip
                .as_ref()
                .is_some_and(|receipt| {
                    !receipt.deposit_verified
                        && (event == Some("deposit confirmed")
                            || receipt.deposit_confirmed
                            || empty_deposit)
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
                    event == Some("deposit confirmed") || empty_deposit,
                );
            result.map_err(|error| format!("{}: {error}", self.name()))?;
        }
        if self.status_claims_bank_close(status) {
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

    fn stats_posted(&self) -> bool {
        let stats = self.snapshot.stats();
        !stats.is_empty() && stats.iter().all(|stat| !stat.used || stat.base > 0)
    }

    fn advance_prep(&mut self, client: &mut client::client::Client) -> Result<(), String> {
        match self.phase {
            Prep::WaitIngame => {
                if self.snapshot.ingame() && self.snapshot.scene_state() == 2 {
                    self.phase = Prep::TutSkip;
                }
            }
            Prep::TutSkip => {
                send_cheat(client, scenario::tutorial::TUTORIAL_SETVAR)?;
                send_cheat(client, scenario::tutorial::TUTORIAL_GETVAR)?;
                self.phase = Prep::WaitTutorial;
            }
            Prep::WaitTutorial => {
                if scenario::tutorial::tutorial_confirmed(&self.snapshot, 0) {
                    self.phase = Prep::Relog;
                }
            }
            Prep::Relog => {
                let ifaces = Arc::clone(&client.ifaces);
                if !interact::logout(client, &ifaces) {
                    return Err("logout interface unavailable during fixture preparation".into());
                }
                if let Some(arm) = &self.login_arm {
                    arm.arm_explicit_login();
                }
                self.phase = Prep::WaitRelog;
            }
            Prep::WaitRelog => {
                if self.snapshot.ingame() && self.snapshot.scene_state() == 2 {
                    if self.case == LiveCase::BankTripRelog && !self.relog_prep_hop_sent {
                        // The product's own hop is off for this cell (see
                        // `run_cell`), so the preparation sends it once here,
                        // where the product would have sent it, before the
                        // planned cut. The cut is never followed by one.
                        interact::mainland_hop(client);
                        self.relog_prep_hop_sent = true;
                    }
                    if self.inventory_tab_available() {
                        self.phase = Prep::ReseedTutorial;
                    }
                }
            }
            Prep::ReseedTutorial => {
                // Post-relog reseed after the kit-close queue has run.
                let baseline = scenario::tutorial::chat_baseline(&self.snapshot);
                send_cheat(client, scenario::tutorial::TUTORIAL_SETVAR)?;
                send_cheat(client, scenario::tutorial::TUTORIAL_GETVAR)?;
                self.tutorial_reseed = Some(scenario::tutorial::PostRelogTutorial::new(baseline));
                self.phase = Prep::WaitReseedConfirm;
            }
            Prep::WaitReseedConfirm => {
                let reseed = self
                    .tutorial_reseed
                    .as_ref()
                    .expect("tutorial reseed armed");
                match reseed.check(&self.snapshot) {
                    Ok(true) => {
                        println!("{}", scenario::tutorial::confirmation_log());
                        self.phase = Prep::Seed;
                    }
                    Ok(false) => {}
                    Err(error) => return Err(error),
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
                    && self.stats_posted()
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
                    } else if self.case == LiveCase::LocationAuto {
                        let scene = self.snapshot.scene();
                        let player_tile =
                            self.snapshot
                                .tile()
                                .map(|(x, z, level)| WorldTile { x, z, level });
                        let scene_query = api::query::SceneQuery::new(scene, player_tile);
                        let approach_options = auto_approach_options(scene.width, scene.height);
                        let reachable = scene_query
                            .flood_reach()
                            .ok_or("Auto fixture has no observed collision reachability")?;
                        let start = self
                            .latest
                            .as_ref()
                            .and_then(|latest| {
                                latest.build.and_then(|build| {
                                    auto_start_candidate(
                                        self.target,
                                        build,
                                        &latest.normal_trees,
                                        |tile| reachable.at(&tile).0,
                                        |tile| scene_query.walkable(tile),
                                        |from, tree| {
                                            api::query::SceneQuery::new(scene, Some(from))
                                                .can_reach(
                                                    tree,
                                                    &approach_options,
                                                )
                                        },
                                    )
                                })
                            })
                            .ok_or_else(|| {
                                format!(
                                    "{} has no reachable standable empty Auto ring beside a real tree with a reachable adjacent approach",
                                    self.name()
                                )
                            })?;
                        self.target = start;
                        if self.near_tile(start, 6) {
                            self.observe_auto_start_area()?;
                            self.phase = Prep::Equip;
                        } else {
                            send_cheat(
                                client,
                                &interact::tele_args(start.level, start.x, start.z),
                            )?;
                            self.phase = Prep::WaitAutoStart;
                        }
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
            Prep::WaitAutoStart => {
                if self.snapshot.ingame()
                    && self.snapshot.scene_state() == 2
                    && self.near_target()
                    && self.item_in_inventory()
                    && self.stats_posted()
                    && self.stat_level() >= self.requested_level
                {
                    self.observe_auto_start_area()?;
                    self.phase = Prep::Equip;
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
        if matches!(
            self.case,
            LiveCase::WoodcuttingBank | LiveCase::WoodcuttingBankUnwieldable
        ) {
            // After seeding, before the script starts. This fixture banks the
            // axe, so a correct Start has it in neither the pack nor the
            // equipment; the wield oracle requires it to be absent from the
            // equipment here and observed unworn later.
            self.witness.tool_worn_at_start = self.item_equipped();
            if self.witness.tool_worn_at_start {
                self.witness
                    .record_tool_sighting(ToolSightingSource::Start, true);
            } else if self.item_in_inventory() {
                self.witness
                    .record_tool_sighting(ToolSightingSource::Start, false);
            }
        }
        let seeded_gem = self.cell == Cell::Mining && self.case == LiveCase::Power;
        let seeded_count: i32 = baseline
            .inventory
            .values()
            .filter(|row| row.id == SEEDED_MINING_GEM_ID)
            .map(|row| row.count)
            .sum();
        let seeded_products = if self.case == LiveCase::FishBankSoak {
            // The soak starts with a full pack of raw shrimp so its first
            // bank trip is due at once.
            SOAK_SEED_FISH as i32
        } else {
            i32::from(seeded_gem)
        };
        if baseline.product_count != seeded_products || (seeded_gem && seeded_count != 1) {
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
        // Match SnapshotView::bank_session: only an in-game decoded modal
        // can prove closure. Never bridge an offline frame or login session.
        if !self.snapshot.ingame() {
            self.witness.bank_open_login_session = None;
        } else {
            let session = self.snapshot.gens().session;
            if self.snapshot.bank_component_id() >= 0 {
                self.witness.bank_open_login_session = Some(session);
            } else {
                self.witness.bank_closed_observed |=
                    self.witness.bank_open_login_session.take() == Some(session);
            }
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
        if let Some(reason) =
            text_field(status, "block_reason").filter(|reason| !reason.trim().is_empty())
        {
            self.witness.last_status_block_reason = Some(reason.to_owned());
        }
        if self.case == LiveCase::FishSpotMoves {
            let phase = text_field(status, "phase");
            let event = text_field(status, "last_event");
            let area = text_field(status, "area");
            let target = text_field(status, "target");
            if self.witness.fish_status_sequence.last().is_none_or(|last| {
                last["phase"].as_str() != phase
                    || last["event"].as_str() != event
                    || last["area"].as_str() != area
                    || last["target"].as_str() != target
            }) {
                let npcs: Vec<_> = self
                    .snapshot
                    .npcs()
                    .iter()
                    .filter(|npc| matches!(npc.r#type, Some(320 | 390 | 403..=406 | 444..=452)))
                    .map(|npc| {
                        json!({
                            "index": npc.index,
                            "type": npc.r#type,
                            "tile": tile_key(npc.tile),
                        })
                    })
                    .collect();
                let row = json!({
                    "tick": self.native_tick,
                    "snapshot_tick": self.latest.as_ref().map(|row| row.tick),
                    "phase": phase,
                    "event": event,
                    "area": area,
                    "target": target,
                    "xp": self.latest.as_ref().map(|row| row.xp),
                    "tile": self.latest.as_ref().and_then(|row| row.tile),
                    "npcs": npcs,
                });
                println!("gatherer-fish-status {row}");
                self.witness.fish_status_sequence.push(row);
            }
        }
        if self.case == LiveCase::FishBankSoak {
            self.apply_soak_status(status);
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
            }
        }
        if self.case == LiveCase::DeathRespawnRegion {
            observe_death_respawn_region(
                &mut self.witness,
                current_recovery_step,
                text_field(status, "last_event"),
                self.latest.as_ref().and_then(|latest| latest.tile),
            );
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
        let bank_step_is_return = self
            .witness
            .last_status_bank
            .as_deref()
            .and_then(|bank| bank.rsplit_once("; ").map(|(_, step)| step == "Return"));
        if let Some(step_is_return) = bank_step_is_return {
            self.witness.bank_loaded_observed |= self.snapshot.bank_loaded();
            if step_is_return {
                record_bank_return_step(&mut self.witness, "Return");
                self.witness.bank_return_after_pause |=
                    self.case == LiveCase::PauseResumeOtherPlane && self.witness.resume_observed;
                self.witness.bank_return_after_reconnect |=
                    self.case == LiveCase::ReconnectReturn && self.witness.reconnect_observed;
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
                    // A bare id is the selected tool in the pack, unworn:
                    // the bank-close row and any row before the wear action.
                    self.witness
                        .record_tool_sighting(ToolSightingSource::Status, false);
                } else if let Some(id) = tool.strip_suffix(" (worn)") {
                    if id == expected {
                        self.witness.tool_selected_observed = true;
                        self.witness
                            .record_tool_sighting(ToolSightingSource::Status, true);
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
            if self.case == LiveCase::LocationAuto
                && auto_search_beyond_start_ring(self.target, area)
            {
                self.witness.auto_widen_seen = true;
            }
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
            if self.status_claims_bank_close(status) {
                self.witness.bank_closed_observed = true;
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

        if !self.witness.bank_selected && self.complete_bank_selection() {
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
            self.witness.bank_selected = true;
            self.witness.bank_selection_elapsed = self
                .witness
                .bank_trip_due_at
                .map(|started| Instant::now().duration_since(started));
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
            if self.case == LiveCase::FishBankSoak {
                self.record_soak_block(status, &code, &message);
                return Ok(());
            }
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
                LiveCase::CoinRunesEmpty => {
                    code == "supply-missing"
                        && status.phase == script::native::NativePhase::Blocked
                        && self.complete_bank_selection()
                        && self.witness.bank_loaded_observed
                        && text_field(status, "bank").is_some_and(|bank| bank.ends_with("; Supply"))
                }
                LiveCase::OakNoPlacementArea => {
                    code == "area-invalid"
                        && message == "no selected method has an accessible placement in the area"
                }
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
                    if let Some(latest) = &self.latest {
                        self.witness.death_return_stop.observe_terminal(
                            latest.tick,
                            latest.tile,
                            self.witness.last_deaths,
                            &code,
                            &message,
                        );
                    }
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

    /// The Woodcutting bank cells' tool fact: the selected axe was first
    /// observed in the pack while not worn, then observed worn, in that
    /// order. The old gate sampled the transient
    /// `last_event = "wielding tool"`, which the product may set and replace
    /// within one game tick; these facts are durable because the bank-close
    /// row and the worn equipment persist. The order comes from one
    /// monotonic sighting index shared by the Start, frame and status
    /// recorders, so the reviewer's worn-row-then-bare-row shape fails as it
    /// did under the old gate. A run that started with the axe already
    /// worn, or never wore it, still fails.
    fn qualifies_wield(&self) -> Result<(), String> {
        if self.case == LiveCase::WoodcuttingBank
            && (self.witness.tool_worn_at_start || !self.witness.tool_wield_proved())
        {
            return Err(format!(
                "{} did not fetch the selected bronze axe unworn and then wear it: {:?}",
                self.name(),
                self.witness
            ));
        }
        if self.case == LiveCase::WoodcuttingBankUnwieldable
            && self.witness.tool_worn_first.is_some()
        {
            return Err(format!(
                "{} incorrectly marked the unwieldable rune axe worn: {:?}",
                self.name(),
                self.witness
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
            LiveCase::FishSpotMoves => {
                if !self.witness.fish_movement.complete()
                    || self.witness.last_status_yielded == 0
                    || self.witness.failure_code.is_some()
                {
                    return Err(format!(
                        "{} did not resume fishing after two real spot moves: {:?}",
                        self.name(),
                        self.witness.fish_movement
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
                if !next_group_beyond_start_ring
                    || !self.witness.auto_widen_seen
                    || !self.witness.auto_selected_next
                    || self.witness.auto_next_gather_tick.is_none()
                    || self.witness.last_status_yielded <= 0
                    || self.witness.last_xp <= self.baseline_xp()
                {
                    return Err(format!(
                        "{} did not widen beyond the empty start ring and gather at the real next group: {:?}",
                        self.name(),
                        self.witness
                    ));
                }
                Ok(())
            }
            LiveCase::OakNoPlacementArea => {
                let no_output = self.witness.last_status_yielded == 0
                    && self.witness.last_status_dropped == 0
                    && !self.witness.script_moved_after_start
                    && self.latest.as_ref().is_some_and(|latest| {
                        latest.product_count == 0 && latest.xp == self.baseline_xp()
                    });
                if !self.witness.native_phases.contains("Blocked")
                    || self.witness.failure_code.as_deref() != Some("area-invalid")
                    || self.witness.native_phases.contains("Waiting")
                    || self.witness.wait_until.is_some()
                    || !no_output
                {
                    return Err(format!(
                        "{} did not reject the area without a placement before moving, waiting, or producing output: {:?}",
                        self.name(),
                        self.witness
                    ));
                }
                Ok(())
            }
            LiveCase::GasHazard => {
                let expected_hazard = Some(tile_key(self.target));
                if !self.witness.gas_site_observed
                    || !self.witness.gas_hazard_observed
                    || self.witness.gas_hazard_tile != expected_hazard
                    || !self.witness.gas_hazard_escaped
                    || !self.witness.gas_hazard_xp_after_escape
                {
                    return Err(format!(
                        "{} did not observe a real iron placement, hazard, escape, and resumed yield: {:?}",
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
                self.qualifies_wield()?;
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
            LiveCase::Site | LiveCase::SiteDraynor => {
                if !site_bank_complete(
                    self.cell,
                    self.case,
                    &self.witness,
                    &self.plan,
                    self.baseline_xp(),
                ) || !self.complete_bank_selection()
                    || self
                        .latest
                        .as_ref()
                        .is_none_or(|latest| latest.product_count == 0)
                    || (self.cell == Cell::Fishing
                        && (!self.item_in_inventory() || self.item_equipped()))
                {
                    return Err(format!("{} did not prove a selected resource anchor, initial approach, nearest bank conservation, Area return and fresh yield: {:?}", self.name(), self.witness));
                }
                Ok(())
            }
            LiveCase::FishGuildApproach => {
                if !fishing_guild_area_reached(self.latest.as_ref().and_then(|latest| latest.tile))
                    || self.witness.last_xp <= self.baseline_xp()
                    || self
                        .latest
                        .as_ref()
                        .is_none_or(|latest| latest.xp <= self.baseline_xp())
                {
                    return Err(format!(
                        "{} did not reach the Fishing Guild fishing area and gain Fishing XP: {:?}",
                        self.name(),
                        self.witness
                    ));
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
                let at_supply = self
                    .witness
                    .last_status_bank
                    .as_deref()
                    .is_some_and(|bank| bank.ends_with("; Supply"));
                if self.witness.failure_code.as_deref() != Some("supply-missing")
                    || !self.witness.native_phases.contains("Blocked")
                    || !self.complete_bank_selection()
                    || !self.witness.bank_loaded_observed
                    || !at_supply
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
            LiveCase::BankTripRelog => {
                let fired_step = self.witness.relog_step.clone().unwrap_or_default();
                if !self.complete_bank_selection()
                    || !self.witness.relog_fired
                    || !self.witness.relog_session_advanced
                    || !relog_steps_in_order(&self.witness.relog_steps_after)
                    || !relog_restart_logged(&fired_step)
                    || !(self.witness.bank_withdrawal_confirmed
                        || relog_step_seen(&self.witness.relog_steps_after, "Close"))
                    || !(self.witness.bank_closed_observed
                        || relog_step_seen(&self.witness.relog_steps_after, "Validate"))
                    || !self.status_integer_seen("trips", 1)
                    || !self.witness.bank_return_step_seen
                    || self.witness.post_bank_yields == 0
                    || self
                        .latest
                        .as_ref()
                        .is_none_or(|latest| latest.xp <= self.baseline_xp())
                {
                    return Err(format!(
                        "{} did not restart its bank trip after the cut and yield: log={:?} {:?}",
                        self.name(),
                        relog_log(),
                        self.witness
                    ));
                }
                Ok(())
            }
            LiveCase::FishBankSoak => {
                let soak = &self.witness.soak;
                let min_trips = soak_min_trips();
                if soak.trips < min_trips {
                    return Err(format!(
                        "{} completed {} bank trips, fewer than the required {min_trips}: {:?}",
                        self.name(),
                        soak.trips,
                        soak
                    ));
                }
                if soak.deposit_blocks > 0 {
                    return Err(format!(
                        "{} blocked on bank deposit {} time(s) in {} trips ({} other blocks): {:?}",
                        self.name(),
                        soak.deposit_blocks,
                        soak.trips,
                        soak.other_blocks,
                        soak.blocks
                            .iter()
                            .map(|block| {
                                json!({
                                    "tick": block["tick"],
                                    "trips_before": block["trips_before"],
                                    "failure_code": block["failure_code"],
                                    "failure_message": block["failure_message"],
                                    "event": block["event"],
                                    "bank": block["bank"],
                                })
                            })
                            .collect::<Vec<_>>()
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
            .filter(|kind| matches!(*kind, "NearShortcut" | "Reachable" | "AirFallback"))
    }

    fn complete_bank_selection(&self) -> bool {
        self.selected_bank_name().is_some()
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
fn observe_death_respawn_region(
    witness: &mut Witness,
    recovery_step: u8,
    last_event: Option<&str>,
    sampled_tile: Option<(i32, i32, i32)>,
) {
    if witness.last_deaths == 0 || witness.death_respawn_tile.is_some() {
        return;
    }
    if recovery_step != 1 || last_event != Some("respawn observed; settling") {
        return;
    }
    let Some(respawn) = sampled_tile else {
        return;
    };
    let respawn_tile = WorldTile {
        x: respawn.0,
        z: respawn.1,
        level: respawn.2,
    };
    witness.death_respawn_tile = Some(respawn);
    witness.death_respawn_region_unchanged = script::native::death::RESPAWN_SQUARE
        .contains(respawn_tile)
        && witness.death_command_tile.is_some_and(|death| {
            death.0.div_euclid(64) == respawn.0.div_euclid(64)
                && death.1.div_euclid(64) == respawn.1.div_euclid(64)
                && death.2 == respawn.2
        });
}

fn auto_search_beyond_start_ring(start: WorldTile, area: &str) -> bool {
    let Some((coords, search)) = area
        .strip_prefix("auto (")
        .and_then(|area| area.split_once(") r32; searched: "))
    else {
        return false;
    };
    let Some((searched, attempts)) = search.split_once("; tried: ") else {
        return false;
    };
    let Some((tried, limit)) = attempts.split_once("; limit: ") else {
        return false;
    };
    if limit != "128" || tried.parse::<usize>().is_err() {
        return false;
    }
    let Ok(searched) = searched.parse::<u32>() else {
        return false;
    };
    let Ok(anchor) = parse_tile("Auto area", coords) else {
        return false;
    };
    let distance = anchor.x.abs_diff(start.x).max(anchor.z.abs_diff(start.z));
    searched <= 128 && anchor.level == start.level && (33..=searched).contains(&distance)
}

fn auto_approach_options(width: i32, height: i32) -> api::query::SceneReachOptions {
    api::query::SceneReachOptions {
        // A tree outside the 32-tile ring can exceed the API's 400-expansion
        // default. Search the observed scene, not an unbounded world region.
        max_steps: Some(
            u32::try_from(width)
                .unwrap_or(0)
                .saturating_mul(u32::try_from(height).unwrap_or(0)),
        ),
        adjacent_ok: true,
    }
}

fn auto_start_candidate(
    origin: WorldTile,
    build: BuildRect,
    normal_trees: &BTreeSet<(i32, i32, i32)>,
    reachable: impl Fn(WorldTile) -> bool,
    standable: impl Fn(WorldTile) -> bool,
    tree_reachable: impl Fn(WorldTile, WorldTile) -> bool,
) -> Option<WorldTile> {
    if build.level != origin.level || build.width <= 0 || build.height <= 0 {
        return None;
    }
    let min_x = build.base_x.max(origin.x.saturating_sub(48));
    let max_x = (build.base_x + build.width - 1).min(origin.x.saturating_add(48));
    let min_z = build.base_z.max(origin.z.saturating_sub(48));
    let max_z = (build.base_z + build.height - 1).min(origin.z.saturating_add(48));
    let mut best: Option<(i32, WorldTile)> = None;
    for x in min_x..=max_x {
        for z in min_z..=max_z {
            let candidate = WorldTile {
                x,
                z,
                level: build.level,
            };
            if !standable(candidate)
                || normal_trees.iter().any(|&tree| {
                    tile_distance(tree, candidate).is_some_and(|distance| distance <= 32)
                })
            {
                continue;
            }
            let distance = tile_distance(tile_key(origin), candidate)?;
            if best.is_some_and(|(best_distance, best_tile)| {
                (distance, candidate.x, candidate.z) >= (best_distance, best_tile.x, best_tile.z)
            }) {
                continue;
            }
            if !reachable(candidate)
                || !normal_trees.iter().any(|&tree| {
                    let tree_tile = WorldTile {
                        x: tree.0,
                        z: tree.1,
                        level: tree.2,
                    };
                    tile_distance(tree, candidate)
                        .is_some_and(|distance| (33..=128).contains(&distance))
                        && tree_reachable(candidate, tree_tile)
                })
            {
                continue;
            }
            best = Some((distance, candidate));
        }
    }
    best.map(|(_, tile)| tile)
}

fn auto_next_group(start: WorldTile, normal_trees: &BTreeSet<(i32, i32, i32)>) -> Vec<WorldTile> {
    normal_trees
        .iter()
        .filter_map(|&tree| {
            tile_distance(tree, start)
                .filter(|distance| (33..=128).contains(distance))
                .map(|_| WorldTile {
                    x: tree.0,
                    z: tree.1,
                    level: tree.2,
                })
        })
        .collect()
}

/// Run on/off from the client's orb pair (the host's own auto-run echo,
/// `host::run_echo`): the off orb visible and the on orb hidden is running.
fn run_orb_echo(client: &client::client::Client) -> Option<bool> {
    let off = client.if_(152)?;
    let on = client.if_(153)?;
    (off.hide != on.hide).then_some(!off.hide)
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

/// Fixed soak deadline (`GATHERER_SOAK_SECS`, default 1800 s), measured from
/// the first Start.
fn soak_duration() -> Duration {
    Duration::from_secs(
        std::env::var("GATHERER_SOAK_SECS")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(1800),
    )
}

/// Bank trips the soak must complete to qualify (`GATHERER_SOAK_MIN_TRIPS`,
/// default 10).
fn soak_min_trips() -> u32 {
    std::env::var("GATHERER_SOAK_MIN_TRIPS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(10)
}

/// `GATHERER_SOAK_TWO_PRODUCTS=1`: a measurement mode, not the
/// qualification soak. The pack is seeded with raw shrimp and raw anchovies
/// so every trip deposits in two clicks: the transfer window a run-threshold
/// crossing can land in is about twice as long, which raises the rate at
/// which an unguarded host's press (`controls:com_5`, `if_close` first) is
/// recorded inside a transfer.
fn soak_two_products() -> bool {
    std::env::var("GATHERER_SOAK_TWO_PRODUCTS").is_ok_and(|value| value == "1")
}

/// Raw shrimp and raw anchovies seeded per fill: all shrimp normally, a
/// two-product split under `GATHERER_SOAK_TWO_PRODUCTS`.
fn soak_seed_split() -> (i32, i32) {
    let total = SOAK_SEED_FISH as i32;
    if soak_two_products() {
        (total - total / 2, total / 2)
    } else {
        (total, 0)
    }
}

/// A capture handed to the background writer, awaiting its outcome.
struct PendingGathererCapture {
    job: EvidenceJob,
}

/// The slot-thread half of the terminal gatherer capture: render (needs the
/// client) into native pixmap words and hand them plus the JSON receipt to
/// the shared background evidence writer for encode and write. The hook
/// polls the job and reports completion only once the files are durable.
fn submit_live_capture(
    client: &mut client::client::Client,
    writer: &EvidenceWriter,
    directory: &Path,
    receipt: &Value,
) -> Result<PendingGathererCapture, String> {
    std::fs::create_dir_all(directory).map_err(|error| error.to_string())?;
    let epoch = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| error.to_string())?
        .as_secs();
    let stem = format!("{epoch}Z_01-final");
    let render_start = Instant::now();
    let mut renderer = client::render::Renderer::new_prefer(client.config.lowmem, false);
    let was_draw = client.draw;
    client.set_draw(true);
    let frame = renderer.mainredraw(client);
    client.set_draw(was_draw);
    let client::render::backend::FrameOutput::PixMap(pixels) = frame else {
        return Err("live capture did not return a CPU PixMap".into());
    };
    println!(
        "capture-render site=gatherer {stem} {}x{} render_ms={:.1}",
        pixels.width,
        pixels.height,
        render_start.elapsed().as_secs_f64() * 1000.0,
    );
    let mut receipt = receipt.clone();
    receipt["frame"] = json!({
        "ingame": client.ingame,
        "scene_state": client.scene_state,
        "renderer": "real Client CpuPix3D framebuffer",
    });
    let job = writer.submit(EvidenceRequest {
        png_path: directory.join(format!("{stem}.png")),
        width: pixels.width as u32,
        height: pixels.height as u32,
        pixels: pixels.pixels,
        color: PngColor::Rgba,
        sidecar: Some(EvidenceSidecar {
            path: directory.join(format!("{stem}.json")),
            receipt,
            patch_error: None,
        }),
    });
    Ok(PendingGathererCapture { job })
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

/// Fishing Guild NPC envelope [2599,3410]–[2612,3426] plus a 2-tile stand pad.
/// The south guild door is outside this box, so a door/nav stall is a failure.
fn fishing_guild_area_reached(tile: Option<(i32, i32, i32)>) -> bool {
    tile.is_some_and(|(x, z, level)| {
        level == 0 && (2597..=2614).contains(&x) && (3408..=3428).contains(&z)
    })
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
        | LiveCase::ReconnectReturn
        | LiveCase::BankTripRelog => plan.bank_seed.push(("bronze_axe".into(), 1)),
        LiveCase::DeathReturn => plan.bank_seed.push(("bronze_axe".into(), 3)),
        LiveCase::DeathNoStock => plan.inventory_seed.push(("bronze_axe".into(), 1)),
        LiveCase::DeathRespawnRegion
        | LiveCase::DeathReturnRefused
        | LiveCase::DeathWatchdogPending
        | LiveCase::DeathWatchdogProving => {
            plan.bank_seed.push(("bronze_axe".into(), 2));
        }
        LiveCase::FishBait => plan.bank_seed.push(("feather".into(), 7)),
        LiveCase::FishHarpoonBank => {
            plan.inventory_seed.push(("casket".into(), 1));
            plan.inventory_seed.push(("net".into(), 1));
            plan.inventory_seed.push(("beer".into(), 1));
        }
        LiveCase::FishBankSoak => {
            let (shrimp, anchovies) = soak_seed_split();
            plan.inventory_seed.push(("raw_shrimp".into(), shrimp));
            if anchovies > 0 {
                plan.inventory_seed
                    .push(("raw_anchovies".into(), anchovies));
            }
        }
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
        LiveCase::RandomEvent => fixture_tile(cell.tile_env(case), DRAYNOR_OAK_TARGET)?,
        LiveCase::MazeRandom => fixture_tile(cell.tile_env(case), OAK_RESPAWN_START)?,
        LiveCase::DeathReturn
        | LiveCase::DeathNoStock
        | LiveCase::DeathWatchdogPending
        | LiveCase::DeathWatchdogProving => fixture_tile(cell.tile_env(case), DRAYNOR_OAK_TARGET)?,
        LiveCase::DeathRespawnRegion => fixture_tile(cell.tile_env(case), DEATH_REGION_START)?,
        LiveCase::DeathReturnRefused => {
            // Use a quiet authored wilderness oak. After death, returning to it
            // crosses the forbidden boundary with allowWilderness still false.
            fixture_tile(cell.tile_env(case), DEATH_RETURN_REFUSED_TARGET)?
        }
        LiveCase::FishNet => fixture_tile("GATHERER_FISH_TILE", FISH_NET_START)?,
        LiveCase::FishSpotMoves => world_tile(2838, 3432),
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
            plan.edge_oak = Some(fixture_tile("GATHERER_WC_EDGE_OAK_TILE", OAK_EDGE_TARGET)?);
            plan.preflight_edge = Some(fixture_tile(
                "GATHERER_WC_EDGE_PREFLIGHT_TILE",
                OAK_EDGE_PREFLIGHT,
            )?);
            fixture_tile("GATHERER_WC_EDGE_START_TILE", OAK_EDGE_START)?
        }
        LiveCase::OakNoPlacementArea => {
            fixture_tile("GATHERER_WC_NO_PLACEMENT_TILE", OAK_NO_PLACEMENT_START)?
        }
        LiveCase::LocationAuto => fixture_tile("GATHERER_AUTO_START_TILE", AUTO_START)?,
        LiveCase::GasHazard => fixture_tile("GATHERER_GAS_TILE", GAS_START)?,
        LiveCase::WoodcuttingBank
        | LiveCase::WoodcuttingBankUnwieldable
        | LiveCase::BankCostNoCandidate
        | LiveCase::CoinRunes
        | LiveCase::CoinRunesEmpty
        | LiveCase::PowerToBank
        | LiveCase::PauseResumeOtherPlane
        | LiveCase::ReconnectReturn
        | LiveCase::BankTripRelog => fixture_tile(cell.tile_env(case), OAK_RESPAWN_START)?,
        LiveCase::FishBait => fixture_tile(cell.tile_env(case), FISH_BAIT_START)?,
        LiveCase::FishHarpoonBank => fixture_tile(cell.tile_env(case), world_tile(2840, 3436))?,
        LiveCase::FishHarpoonBankAuto => CATHERBY_HARPOON_START,
        LiveCase::FishBankSoak => CATHERBY_HARPOON_START,
        LiveCase::FishGuildApproach => ARDOUGNE_GUILD_START,
        LiveCase::Site => match cell {
            Cell::Fishing => fixture_tile(cell.tile_env(case), FISHING_GUILD_START)?,
            Cell::Woodcutting => world_tile(3120, 3267),
            Cell::Mining => world_tile(3253, 3420),
        },
        LiveCase::SiteDraynor => DRAYNOR_FISH_SITE_START,
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
        _ => None,
    };
    if matches!(
        case,
        LiveCase::BankCostFirstGoal
            | LiveCase::BankCostSeersMaple
            | LiveCase::Site
            | LiveCase::SiteDraynor
            | LiveCase::FishGuildApproach
    ) && (!plan.seed_locs.is_empty() || !plan.oak_tiles.is_empty() || task.is_some())
    {
        return Err(
            "real-content bank fixtures must use live scene resources without location seeds"
                .into(),
        );
    }
    Ok((target, plan, task))
}
fn real_content_death_fixture(
    _cell: Cell,
    _case: LiveCase,
) -> Result<(WorldTile, FixturePlan, Option<FixtureTask>), String> {
    let mut plan = FixturePlan::default();
    plan.bank_seed.push(("bronze_axe".into(), 3));
    plan.inventory_seed
        .extend(DEATH_FILLERS.iter().map(|(_, alias)| ((*alias).into(), 1)));
    Ok((DRAYNOR_OAK_BANK_START, plan, None))
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
    cache_dir: Option<PathBuf>,
    nav_flags: Option<PathBuf>,
    temp: &Path,
    game_port: Option<u16>,
) -> Result<(Arc<host_play::ServerProfile>, Arc<SharedClientTemplate>), String> {
    let options = ProfileOptions {
        profile: Some("local-289".into()),
        revision: Some("289".into()),
        host: Some("127.0.0.1".into()),
        asset_host: Some("127.0.0.1".into()),
        port: Some(game_port.unwrap_or_else(|| {
            std::env::var("GATHERER_GAME_PORT")
                .ok()
                .and_then(|value| value.parse().ok())
                .unwrap_or(47594)
        })),
        http_port: Some(
            std::env::var("GATHERER_HTTP_PORT")
                .ok()
                .and_then(|value| value.parse().ok())
                .unwrap_or(3080),
        ),
        cache_dir,
        nav_pack: Some(nav_pack),
        nav_flags,
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

fn macro_event_fixture_blocker(case: LiveCase) -> Option<&'static str> {
    match case {
        LiveCase::GasHazard => Some(
            "gas-hazard requires a deterministic engine-side gas event; account-only state cannot create its loc",
        ),
        LiveCase::RandomEvent => Some(
            "random-event identity requires deterministic owner and foreign events; account-only state cannot force the random macro-event selector",
        ),
        _ => None,
    }
}
include!("../src/clean_logout_guard.inc.rs");
define_clean_logout_guard!(host_play::Play, CLEAN_LOGOUT_DEADLINE);

fn run_cell(cell: Cell, case: LiveCase) -> Result<(), String> {
    run_cell_with_fixture(cell, case, fixture_plan)
}

fn run_cell_with_fixture(
    cell: Cell,
    case: LiveCase,
    fixture_builder: impl FnOnce(
        Cell,
        LiveCase,
    ) -> Result<(WorldTile, FixturePlan, Option<FixtureTask>), String>,
) -> Result<(), String> {
    if std::env::var("LIVE").as_deref() != Ok("1") {
        return Ok(());
    }
    if let Some(reason) = macro_event_fixture_blocker(case) {
        return Err(reason.into());
    }
    if !api::test_support::game_data_available() {
        return Ok(());
    }
    if case.is_named_site() && !cfg!(feature = "live-probe") {
        return Err(
            "Site live cells require host-play/live-probe for read-only Area-arrival evidence"
                .into(),
        );
    }
    if (case == LiveCase::RandomEvent
        || case == LiveCase::BankTripRelog
        || case.is_death_recovery())
        && std::env::var_os("BOT_LIVE_NAME_PREFIX").is_none()
    {
        return Err("G4a live cells require an own BOT_LIVE_NAME_PREFIX".into());
    }
    let cell_name = cell.name_for_case(case);
    let _home = script::IsolatedEnv::enter(cell_name);
    api::hostlog::set_debug(true);
    let Some(nav_pack) = api::test_support::local_asset_path(
        "GATHERER_NAV_PACK",
        "",
        "real navigation pack for the Gatherer live proof",
    ) else {
        return Ok(());
    };
    let Some(engine_dir) = api::test_support::local_asset_path(
        "GATHERER_ENGINE_DIR",
        "",
        "real 289 engine content for the Gatherer live proof",
    ) else {
        return Ok(());
    };
    let Some(catalog_root) = api::test_support::local_asset_path(
        "GATHERER_CATALOG_ROOT",
        "",
        "real frozen script catalog for the Gatherer live proof",
    ) else {
        return Ok(());
    };
    let cache_dir = if std::env::var_os("BOT_CACHE_DIR").is_some() {
        let Some(path) = api::test_support::local_asset_path(
            "BOT_CACHE_DIR",
            "",
            "local Gatherer cache snapshot",
        ) else {
            return Ok(());
        };
        Some(path)
    } else {
        None
    };
    let nav_flags = if std::env::var_os("GATHERER_NAV_FLAGS").is_some() {
        let Some(path) = api::test_support::local_asset_path(
            "GATHERER_NAV_FLAGS",
            "",
            "local Gatherer navigation flags",
        ) else {
            return Ok(());
        };
        Some(path)
    } else {
        None
    };
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
    let (target, mut plan, fixture_task) = fixture_builder(cell, case)?;
    let temp = TempRoot::new(cell_name)?;
    let relog = relog_plan()?;
    let upstream_port = std::env::var("GATHERER_GAME_PORT")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(45594);
    let relay = (case == LiveCase::BankTripRelog && relog.dead_server)
        .then(|| relog_relay::Relay::start(upstream_port));
    if case == LiveCase::BankTripRelog && !api::hostlog::install_sink(&RELOG_CAPTURE) {
        return Err("run this live cell in its own test process".into());
    }
    let (profile, template) = selected_profile(
        nav_pack,
        engine_dir,
        catalog_root,
        cache_dir,
        nav_flags,
        temp.path(),
        relay.as_ref().map(|relay| relay.port),
    )?;
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
    let helper_needed = fixture_task.is_some();
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
        None,
        false,
    )));
    let helper_state = fixture_task.map(|task| {
        Arc::new(Mutex::new(GatherSlot::new(
            cell,
            case,
            target,
            plan.clone(),
            Some(task),
            true,
        )))
    });
    // The preparation relogs each bot before any script runs: each slot's arm
    // is owned here so the fixture can post the Login after its own logout.
    let subject_arm = SlotArm::new(274_289_304, true);
    state.lock().unwrap().login_arm = Some(Arc::clone(&subject_arm));
    let helper_arm = helper_state.as_ref().map(|helper| {
        let arm = SlotArm::new(274_289_305, true);
        helper.lock().unwrap().login_arm = Some(Arc::clone(&arm));
        arm
    });
    let start_handle: Arc<Mutex<Option<ScriptStartHandle>>> = Arc::new(Mutex::new(None));
    let frame_state = Arc::clone(&state);
    let frame_handle = Arc::clone(&start_handle);
    let frame_account = account.clone();
    let frame_helper_account = helper_account.clone();
    let frame_helper_state = helper_state.clone();
    let frame_world = profile.world();
    let capture_request = Arc::new(Mutex::new(None::<(PathBuf, Value)>));
    let capture_result = Arc::new(Mutex::new(None::<Result<(), String>>));
    let frame_capture_request = Arc::clone(&capture_request);
    let frame_capture_result = Arc::clone(&capture_result);
    let evidence = Arc::new(EvidenceWriter::new(
        host_play::evidence_writer::DEFAULT_QUEUE_BOUND,
    ));
    let frame_evidence = Arc::clone(&evidence);
    let frame_capture_job = Arc::new(Mutex::new(None::<PendingGathererCapture>));
    let mut play = host_play::run_with_template(
        Arc::clone(&template),
        // The product re-sends this hop (a teleport to 3220,3220 and
        // `setvar tutorial 1000`) at every new session of a --mainland run
        // (`play_slots.rs:1461,1801-1804`). A relog cell cuts a session on
        // purpose, and the hop after the cut moved the character off its
        // walk. The cell's preparation sends the hop once itself
        // (`Prep::WaitRelog`), before the cut, and the cut is never followed
        // by one.
        case != LiveCase::BankTripRelog,
        vec![],
        |_| (None, None),
        move |client, username, frame| {
            let hold = frame.hold;
            if frame_helper_account.as_deref() == Some(username) {
                if let Some(helper_state) = &frame_helper_state {
                    if let Ok(mut helper) = helper_state.lock() {
                        helper.frame(client, hold);
                        if let Ok(mut main) = frame_state.lock() {
                            main.witness.fixture_chop_observed |=
                                helper.witness.fixture_chop_observed;
                            main.witness
                                .fixture_chop_tiles
                                .extend(helper.witness.fixture_chop_tiles.iter().copied());
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
            let (trigger_recovery_death, death_command_tile) = frame_state
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
                    (trigger_recovery_death, death_command_tile)
                })
                .unwrap_or((false, None));
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
                slot.frame(client, hold);
                if case == LiveCase::DeathReturnRefused
                    && slot.witness.death_recovery_command_sent > 0
                {
                    let tick = slot.latest.as_ref().map_or(0, |latest| latest.tick);
                    let tile = slot.snapshot.tile();
                    let alive = slot.snapshot.ingame()
                        && slot.snapshot.scene_state() == 2
                        && slot.snapshot.stats().iter().any(|stat| {
                            stat.name.eq_ignore_ascii_case("Hitpoints") && stat.effective > 0
                        });
                    let in_wilderness =
                        frame_world
                            .as_ref()
                            .zip(tile)
                            .map(|(world, (x, z, level))| {
                                world.graph.wilderness.contains(WorldTile { x, z, level })
                            });
                    slot.witness
                        .death_return_stop
                        .observe_tile(tick, tile, alive, in_wilderness);
                }
            }
            let capture = frame_capture_request
                .lock()
                .ok()
                .and_then(|mut request| request.take());
            if let Some((directory, receipt)) = capture {
                let result = submit_live_capture(client, &frame_evidence, &directory, &receipt);
                match result {
                    Ok(pending) => {
                        if let Ok(mut job) = frame_capture_job.lock() {
                            *job = Some(pending);
                        }
                    }
                    Err(error) => {
                        if let Ok(mut completion) = frame_capture_result.lock() {
                            *completion = Some(Err(error));
                        }
                    }
                }
            }
            // Completions land here; the driver loop already waits for the
            // result under a 20 s deadline, which is the bounded flush.
            let completed = frame_capture_job.lock().ok().and_then(|mut job| {
                let outcome = frame_evidence.poll(&job.as_ref()?.job)?;
                let result = outcome.error.map_or(Ok(()), Err);
                job.take();
                Some(result)
            });
            if let Some(result) = completed {
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
    play.try_spawn_slot(subject, None, None, Some(subject_arm))?;
    if let (Some(helper_account), Some(helper_password)) =
        (helper_account.as_ref(), helper_password.as_ref())
    {
        play.try_spawn_slot(
            mint_profile(helper_account, helper_password, 2)?,
            None,
            None,
            helper_arm,
        )?;
    }
    let mut logout_guard = (case == LiveCase::BankCostFirstGoal).then(|| CleanLogoutGuard {
        play: &play,
        account: &account,
        label: cell_name,
        attempted: false,
    });
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
    let fish_spot_soak = case == LiveCase::FishSpotMoves
        && std::env::var("GATHERER_FISH_SPOT_SOAK").as_deref() == Ok("1");
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
    let mut relay_paused = false;
    let mut soak_started_at: Option<Instant> = None;
    let mut soak_stop_requested = false;
    let mut soak_stop_at = Instant::now();
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
        let ready_to_start = ready_to_start && helper_done;
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
            let needs_bank_config = {
                let slot = state.lock().map_err(|_| "live state poisoned")?;
                case.is_bank_mode() || slot.is_real_content_death_reprovision()
            };
            let bank_config = if needs_bank_config {
                let preparation = match play.script_prepare_config(
                    script::CompiledId("Gatherer"),
                    1,
                    Arc::new(start_bag.clone()),
                ) {
                    Ok(preparation) => preparation,
                    Err(error) => break Err(format!("{cell_name} bank settings: {error}")),
                };
                let identity = preparation.identity();
                match preparation.join() {
                    Ok(Ok(prepared)) => match prepared.commit(identity) {
                        Ok(config) => Some(config),
                        Err(error) => break Err(format!("{cell_name} bank settings: {error}")),
                    },
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
            if soak_started_at.is_none() {
                soak_started_at = Some(Instant::now());
            }
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
            let identity = preparation.identity();
            let prepared = match preparation.join() {
                Ok(Ok(prepared)) => match prepared.commit(identity) {
                    Ok(config) => config,
                    Err(error) => {
                        break Err(format!("{cell_name} Power to Bank settings: {error}"))
                    }
                },
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
        if case == LiveCase::BankTripRelog && !relay_paused {
            if let Some(relay) = &relay {
                let requested = state
                    .lock()
                    .map_err(|_| "live state poisoned")?
                    .relog_pause_requested;
                if requested {
                    let connection = relay.pause_live_connection();
                    println!(
                        "gatherer-bank-relog-pause {}",
                        json!({"connection": connection})
                    );
                    // Connection 0 means no game connection was seen yet.
                    relay_paused = connection != 0;
                }
            }
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
        if case == LiveCase::FishBankSoak && witness.soak.block_pending {
            if !soak_stop_requested {
                // The user's recovery: stop the blocked run and start again
                // with the same settings. The receipt keeps every block.
                play.script_stop(&account);
                soak_stop_requested = true;
                soak_stop_at = Instant::now();
            } else if current_run.is_none() || soak_stop_at.elapsed() >= Duration::from_secs(10) {
                let runs = {
                    let mut slot = state.lock().map_err(|_| "live state poisoned")?;
                    slot.reset_after_stop_for_restart();
                    slot.witness.status_trips = 0;
                    slot.witness.soak.block_pending = false;
                    slot.witness.soak.run_trips = 0;
                    slot.witness.soak.runs
                };
                if let Err(error) =
                    play.script_start(&account, script::CompiledId("Gatherer"), settings.clone())
                {
                    break Err(format!("{} soak restart failed: {error}", cell_name));
                }
                println!(
                    "{}",
                    json!({
                        "phase": "soak-restart",
                        "cell": cell_name,
                        "runs_before": runs,
                        "deposit_blocks": witness.soak.deposit_blocks,
                        "other_blocks": witness.soak.other_blocks,
                        "trips": witness.soak.trips,
                    })
                );
                soak_stop_requested = false;
            }
        }
        if case == LiveCase::FishBankSoak {
            let run_sends = play
                .statuses()
                .iter()
                .find(|status| status.username == account)
                .map(|status| status.run_sends);
            if let Some(run_sends) = run_sends {
                state
                    .lock()
                    .map_err(|_| "live state poisoned")?
                    .witness
                    .soak
                    .run_sends = run_sends;
            }
        }
        if let Some(status) = play.script_native_status(&account) {
            let mut slot = state.lock().map_err(|_| "live state poisoned")?;
            if let Err(error) = slot.apply_status(&status) {
                slot.error = Some(error);
            }
            if case == LiveCase::DeathNoStock
                && status.phase == script::native::NativePhase::Blocked
                && play.script_state(&account) == script::RunState::Running
                && text_field(&status, "block_reason")
                    .is_some_and(|reason| !reason.trim().is_empty())
            {
                slot.witness.death_no_stock_blocked_running = true;
            }
            #[cfg(feature = "live-probe")]
            if case.is_named_site() {
                let probe = play.manual_click_live_probe(&account);
                let nav = &probe["nav"];
                let returning =
                    text_field(&status, "bank").is_some_and(|bank| bank.ends_with("; Return"));
                let anchor = text_field(&status, "area").and_then(parse_work_anchor);
                if case == LiveCase::SiteDraynor {
                    let player_tile =
                        slot.snapshot
                            .tile()
                            .map(|(x, z, level)| WorldTile { x, z, level });
                    record_draynor_site_return(
                        &mut slot.witness,
                        nav,
                        returning,
                        text_field(&status, "target").unwrap_or_default(),
                        anchor,
                        player_tile,
                    );
                }
                if nav["armed"] == true {
                    slot.witness.site_initial_walk |= !returning
                        && slot.witness.status_trips == 0
                        && slot.witness.last_status_yielded == 0
                        && anchor.is_some_and(|tile| {
                            site_initial_nav_matches(
                                cell,
                                nav,
                                tile,
                                text_field(&status, "phase").unwrap_or_default(),
                                text_field(&status, "target").unwrap_or_default(),
                            )
                        });
                    if case != LiveCase::SiteDraynor
                        && returning
                        && anchor.is_some_and(|tile| {
                            site_return_nav_matches(
                                cell,
                                nav,
                                tile,
                                text_field(&status, "target").unwrap_or_default(),
                            )
                        })
                    {
                        slot.witness.site_return_area = true;
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
        if case == LiveCase::DeathReturnRefused {
            let stopped = play.script_state(&account) == script::RunState::Idle
                && play.script_native_run(&account).is_none()
                && play
                    .script_lifecycle_receipt(&account)
                    .is_some_and(|receipt| receipt.state == script::ScriptTerminalState::Failed);
            #[cfg(feature = "live-probe")]
            let nav = play.manual_click_live_probe(&account)["nav"].clone();
            #[cfg(not(feature = "live-probe"))]
            let nav = Value::Null;
            let mut slot = state.lock().map_err(|_| "live state poisoned")?;
            let tick = slot.latest.as_ref().map_or(0, |latest| latest.tick);
            slot.witness.death_return_stop.observe_permissions(
                settings.get("allowWilderness").and_then(Value::as_bool),
                play.walk_globals().allow_wilderness,
            );
            slot.witness
                .death_return_stop
                .observe_navigation(tick, &nav, stopped);
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
            LiveCase::FishSpotMoves => {
                witness.fish_movement.complete()
                    && (!fish_spot_soak
                        || Instant::now() >= started_at + PREP_DEADLINE + POWER_DEADLINE)
            }
            LiveCase::FishBaitGate | LiveCase::OakNoPlacementArea => witness.failure_code.is_some(),
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
            LiveCase::Site | LiveCase::SiteDraynor => {
                site_bank_complete(cell, case, &witness, &plan, baseline_xp)
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
            LiveCase::FishGuildApproach => latest.as_ref().is_some_and(|latest| {
                fishing_guild_area_reached(latest.tile) && latest.xp > baseline_xp
            }),
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
                        .is_some_and(|bank| bank.ends_with("; Supply"))
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
            LiveCase::BankTripRelog => {
                witness.relog_fired
                    && witness.relog_session_advanced
                    && relog_steps_in_order(&witness.relog_steps_after)
                    && witness.bank_return_step_seen
                    && witness.post_bank_yields >= 1
                    && latest
                        .as_ref()
                        .is_some_and(|latest| latest.xp > baseline_xp)
            }
            LiveCase::FishBankSoak => {
                soak_started_at.is_some_and(|started| started.elapsed() >= soak_duration())
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
        let deadline = if case == LiveCase::FishBankSoak {
            started_at + PREP_DEADLINE + soak_duration() + Duration::from_secs(60)
        } else if phase == Prep::Running {
            if case.is_power() || fish_spot_soak {
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
            "death_respawn_tile": witness.death_respawn_tile,
            "death_respawn_region_unchanged": witness.death_respawn_region_unchanged,
            "death_progress_baselines": witness.death_progress_baselines,
            "deaths": witness.last_deaths,
            "recoveries": witness.last_recoveries,
            "recovery_step": witness.last_recovery_step,
            "death_recovered": witness.death_recovered,
            "death_products_after": witness.death_products_after,
            "death_xp_after": witness.death_xp_after,
            "death_haul_between": witness.death_haul_between,
            "death_return_stop": witness.death_return_stop,
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
            "tool_worn": witness.tool_worn_first.is_some(),
            "tool_held_not_worn": witness.tool_held_not_worn_first.is_some(),
            "tool_worn_at_start": witness.tool_worn_at_start,
            // The wield proof depends on the order of these two sightings, so
            // the receipt carries the first sighting's recorder path and index
            // for each; a passing receipt shows held-before-worn.
            "tool_held_not_worn_source": witness.tool_held_not_worn_source.map(ToolSightingSource::as_str),
            "tool_held_not_worn_seq": witness.tool_held_not_worn_first,
            "tool_worn_source": witness.tool_worn_source.map(ToolSightingSource::as_str),
            "tool_worn_seq": witness.tool_worn_first,
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
    if case == LiveCase::BankTripRelog {
        receipt["bank_relog"] = json!({
            "step": relog.step,
            "dead_server": relog.dead_server,
            "fired": witness.relog_fired,
            "fired_step": witness.relog_step,
            "session_before": witness.relog_session_before,
            "session_advanced": witness.relog_session_advanced,
            "offline_observed": witness.relog_offline_observed,
            "steps_after": witness.relog_steps_after,
            "host_log": relog_log(),
        });
    }
    receipt["failure_sequence"] = json!(witness.failure_sequence);
    if case == LiveCase::FishBankSoak {
        let soak = &witness.soak;
        receipt["soak"] = json!({
            "duration_secs": soak_duration().as_secs(),
            "elapsed_secs": soak_started_at.map(|started| started.elapsed().as_secs()),
            "min_trips": soak_min_trips(),
            "trips": soak.trips,
            "runs": soak.runs,
            "refills": soak.refills,
            "run_sends": soak.run_sends,
            "two_products": soak_two_products(),
            "deposit_blocks": soak.deposit_blocks,
            "other_blocks": soak.other_blocks,
            "blocks": soak.blocks,
            "last_facts": soak.facts,
        });
    }
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
            "death_no_stock": (case == LiveCase::DeathNoStock).then(|| json!({
                "native_phases": witness.native_phases,
                "failure": witness.failure,
                "failure_code": witness.failure_code,
                "block_reason": witness.last_status_block_reason,
                "running_while_blocked": witness.death_no_stock_blocked_running,
                "deaths": witness.last_deaths,
                "recovery_commands": witness.death_recovery_command_sent,
                "bank": witness.last_status_bank,
            })),
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
            "fish_spot_moves": witness.fish_movement.moves,
            "fish_status_sequence": witness.fish_status_sequence,
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
            "auto_real_next_tiles": fixture_plan.auto_next,
            "gas_hazard_tile": witness.gas_hazard_tile,
            "gas_real_site_observed": witness.gas_site_observed,
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
    if case == LiveCase::DeathReturn
        && fixture_plan.seed_locs.is_empty()
        && fixture_plan.oak_tiles.is_empty()
    {
        fixture_receipt["real_content_death_reprovision"] = json!({
            "origin": target,
            "resource": "oak",
            "scene_locations_seeded": fixture_plan.seed_locs.len(),
            "fixture_locations_seeded": fixture_plan.oak_tiles.len(),
            "deposit_evidence": witness
                .death_reprovision_deposits
                .iter()
                .map(ReprovisionDepositEvidence::as_json)
                .collect::<Vec<_>>(),
        });
    }

    if case.is_named_site() {
        receipt["named_site"] = json!({
            "id": cell.site_id_for_case(case),
            "expected_bank": cell.site_bank_for_case(case),
            "selected_resource_anchor": witness.site_anchor_selected,
            "initial_site_walk": witness.site_initial_walk,
            "return_area": witness.site_return_area,
            "return_arrival": witness.site_return_arrival,
            "walks": witness.site_walks,
            "area": witness.last_area,
            "scene_seeds": fixture_plan.seed_locs.len(),
            "start_tile": target,
        });
    }
    if case == LiveCase::FishGuildApproach {
        let (end_tile, end_xp, end_tick, native_tick, baseline_xp, fishing, stats_ready, members) = {
            let slot = state.lock().map_err(|_| "live state poisoned")?;
            let stats = slot.snapshot.stats();
            let fishing = stats
                .iter()
                .find(|stat| stat.name.eq_ignore_ascii_case("fishing"));
            let stats_ready =
                !stats.is_empty() && stats.iter().all(|stat| !stat.used || stat.base > 0);
            (
                slot.latest.as_ref().and_then(|latest| latest.tile),
                slot.latest.as_ref().map(|latest| latest.xp).unwrap_or(0),
                slot.latest.as_ref().map(|latest| latest.tick).unwrap_or(0),
                slot.native_tick,
                slot.baseline_xp(),
                fishing.map(|stat| {
                    json!({
                        "index": stat.index,
                        "base": stat.base,
                        "effective": stat.effective,
                        "xp": stat.xp,
                        "used": stat.used,
                    })
                }),
                stats_ready,
                slot.snapshot.world().members,
            )
        };
        receipt["guild_approach"] = json!({
            "start_tile": [ARDOUGNE_GUILD_START.x, ARDOUGNE_GUILD_START.z, ARDOUGNE_GUILD_START.level],
            "end_tile": end_tile,
            "ticks": end_tick,
            "native_ticks": native_tick,
            "baseline_xp": baseline_xp,
            "xp": end_xp,
            "xp_gained": i64::from(end_xp) - i64::from(baseline_xp),
            "in_guild_area": fishing_guild_area_reached(end_tile),
            "site": "fishing.fishing_guild",
            "method": "fishing.rarefish.op3",
            "scene_seeds": fixture_plan.seed_locs.len(),
            "final_fishing": fishing,
            "final_stats_ready": stats_ready,
            "final_world_members": members,
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
    let clean_logout = if let Some(guard) = logout_guard.as_mut() {
        guard.logout()
    } else {
        play.script_stop(&account);
        Ok(())
    };
    drop(logout_guard);
    play.stop_slot(&account);
    if let Some(helper_account) = &helper_account {
        play.stop_slot(helper_account);
    }
    clean_logout?;
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
fn quester_shared_core_completed(
    native_status: Option<&script::native::ScriptStatus>,
    lifecycle: Option<&script::ScriptLifecycleReceipt>,
    run_state: script::RunState,
) -> Result<bool, String> {
    if let Some(status) = native_status {
        if status.card != script::CompiledId("Quester") {
            return Err(format!(
                "quester_sheep terminal status belongs to {:?}",
                status.card
            ));
        }
    }
    let Some(receipt) = lifecycle else {
        return Ok(false);
    };
    if receipt.state != script::ScriptTerminalState::Completed {
        return Err(format!(
            "quester_sheep native run ended {:?}: {receipt:?}",
            receipt.state
        ));
    }
    Ok(run_state == script::RunState::Idle)
}

#[cfg(feature = "live-harness")]
fn run_quester_sheep_shared_core() -> Result<(), String> {
    if std::env::var("LIVE").as_deref() != Ok("1") {
        return Ok(());
    }
    if !api::test_support::game_data_available() {
        return Ok(());
    }
    const CELL: &str = "g3_quester_sheep_bank_shared_core";
    let _home = script::IsolatedEnv::enter(CELL);
    api::hostlog::set_debug(true);
    let Some(nav_pack) = api::test_support::local_asset_path(
        "GATHERER_NAV_PACK",
        "",
        "real navigation pack for the Gatherer live proof",
    ) else {
        return Ok(());
    };
    let Some(engine_dir) = api::test_support::local_asset_path(
        "GATHERER_ENGINE_DIR",
        "",
        "real 289 engine content for the Gatherer live proof",
    ) else {
        return Ok(());
    };
    let Some(catalog_root) = api::test_support::local_asset_path(
        "GATHERER_CATALOG_ROOT",
        "",
        "real frozen script catalog for the Gatherer live proof",
    ) else {
        return Ok(());
    };
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
    let cache_dir = if std::env::var_os("BOT_CACHE_DIR").is_some() {
        let Some(path) = api::test_support::local_asset_path(
            "BOT_CACHE_DIR",
            "",
            "local Gatherer cache snapshot",
        ) else {
            return Ok(());
        };
        Some(path)
    } else {
        None
    };
    let nav_flags = if std::env::var_os("GATHERER_NAV_FLAGS").is_some() {
        let Some(path) = api::test_support::local_asset_path(
            "GATHERER_NAV_FLAGS",
            "",
            "local Gatherer navigation flags",
        ) else {
            return Ok(());
        };
        Some(path)
    } else {
        None
    };
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
    let (profile, template) = selected_profile(
        nav_pack,
        engine_dir,
        catalog_root,
        cache_dir,
        nav_flags,
        temp.path(),
        None,
    )?;
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
        let lifecycle = play.script_lifecycle_receipt(&account);
        let (runner_status, start_count, error) = {
            let mut slot = state.lock().map_err(|_| "Quester live state poisoned")?;
            if slot.runner.take_relog_login() {
                if let Some(arm) = play.arm(&account) {
                    arm.arm_explicit_login();
                }
            }
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
                match quester_shared_core_completed(
                    native_status.as_deref(),
                    lifecycle.as_ref(),
                    run_state,
                ) {
                    Ok(true) => {
                        println!(
                            "{}",
                            json!({
                                "phase": "witness",
                                "live_case": CELL,
                                "scenario": "quester_sheep",
                                "card": "Quester",
                                "native_phase": native_status.as_ref().map(|status| format!("{:?}", status.phase)),
                                "run_state": format!("{run_state:?}"),
                                "lifecycle": lifecycle,
                                "start_count": start_count,
                            })
                        );
                        break Ok(());
                    }
                    Ok(false) => {}
                    Err(error) => break Err(error),
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
#[ignore = "requires LIVE=1, BOT_LIVE_NAME_PREFIX and Engine A; fishes real Catherby saltfish through two content-timed moves"]
fn gatherer_fish_catherby_spot_moves() {
    run_cell(Cell::Fishing, LiveCase::FishSpotMoves).unwrap();
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
#[ignore = "requires LIVE=1, GATHERER_WC_NO_PLACEMENT_TILE and local 289 engine"]
fn gatherer_wc_area_no_placement() {
    run_cell(Cell::Woodcutting, LiveCase::OakNoPlacementArea).unwrap();
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
#[ignore = "requires LIVE=1, BOT_LIVE_NAME_PREFIX, GATHERER_NAV_PACK, GATHERER_ENGINE_DIR, GATHERER_CATALOG_ROOT and Engine A; back-to-back Catherby bank trips for GATHERER_SOAK_SECS"]
fn gatherer_fish_bank_soak() {
    run_cell(Cell::Fishing, LiveCase::FishBankSoak).unwrap();
}

#[test]
#[ignore = "requires LIVE=1, BOT_LIVE_NAME_PREFIX, GATHERER_NAV_PACK, GATHERER_ENGINE_DIR, GATHERER_CATALOG_ROOT and Engine A"]
fn gatherer_fish_guild_ardougne_approach() {
    run_cell(Cell::Fishing, LiveCase::FishGuildApproach).unwrap();
}

#[test]
#[ignore = "requires LIVE=1, live-probe and local 289 engine"]
fn gatherer_fish_harpoon_bank_site() {
    run_cell(Cell::Fishing, LiveCase::Site).unwrap();
}

#[test]
#[ignore = "requires LIVE=1, live-probe and local 289 Engine A; gathers at Draynor and proves a bank return to its fishing spots"]
fn gatherer_fish_draynor_site_bank_return() {
    run_cell(Cell::Fishing, LiveCase::SiteDraynor).unwrap();
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
#[ignore = "requires LIVE=1, an own BOT_LIVE_NAME_PREFIX and local 289 engine; GATHERER_RELOG_STEP and GATHERER_RELOG_FAULT choose the cut"]
fn gatherer_bank_trip_relog() {
    run_cell(Cell::Woodcutting, LiveCase::BankTripRelog).unwrap();
}

#[test]
fn relog_steps_must_restart_at_access_then_open_then_deposit() {
    let steps = |list: &[&str]| {
        list.iter()
            .map(|step| (*step).to_owned())
            .collect::<Vec<_>>()
    };
    assert!(relog_steps_in_order(&steps(&[
        "Deposit", "Access", "Open", "Deposit", "Withdraw"
    ])));
    assert!(relog_steps_in_order(&steps(&["Access", "Open", "Deposit"])));
    assert!(!relog_steps_in_order(&steps(&[
        "Deposit", "Withdraw", "Close"
    ])));
    assert!(!relog_steps_in_order(&steps(&[
        "Open", "Deposit", "Access"
    ])));
    assert!(!relog_steps_in_order(&[]));
}
#[test]
#[ignore = "requires LIVE=1, an own BOT_LIVE_NAME_PREFIX, GATHERER_WC_TILE, and local 289 engine"]
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
#[ignore = "requires LIVE=1, an own BOT_LIVE_NAME_PREFIX, GATHERER_WC_BANK_TILE, and local 289 engine"]
fn gatherer_death_return() {
    run_cell(Cell::Woodcutting, LiveCase::DeathReturn).unwrap();
}

#[test]
#[ignore = "requires LIVE=1, an own BOT_LIVE_NAME_PREFIX, and local 289 engine"]
fn gatherer_death_reprovision_real_content() {
    run_cell_with_fixture(
        Cell::Woodcutting,
        LiveCase::DeathReturn,
        real_content_death_fixture,
    )
    .unwrap();
}

#[test]
#[ignore = "requires LIVE=1, an own BOT_LIVE_NAME_PREFIX, and local 289 engine; defaults to the Lumbridge respawn-region fixture"]
fn gatherer_death_return_respawn_region() {
    run_cell(Cell::Woodcutting, LiveCase::DeathRespawnRegion).unwrap();
}

#[test]
#[ignore = "requires LIVE=1, an own BOT_LIVE_NAME_PREFIX, and local 289 engine"]
fn gatherer_death_return_no_stock() {
    run_cell(Cell::Woodcutting, LiveCase::DeathNoStock).unwrap();
}

#[test]
#[ignore = "requires LIVE=1, an own BOT_LIVE_NAME_PREFIX, GATHERER_DEATH_RETURN_REFUSE_TILE, and local 289 engine"]
fn gatherer_death_return_refused_stops() {
    run_cell(Cell::Woodcutting, LiveCase::DeathReturnRefused).unwrap();
}

#[cfg(feature = "live-probe")]
#[test]
#[ignore = "requires LIVE=1, an own BOT_LIVE_NAME_PREFIX, GATHERER_WC_BANK_TILE, live-probe, and local 289 engine"]
fn gatherer_death_watchdog_pending5() {
    run_cell(Cell::Woodcutting, LiveCase::DeathWatchdogPending).unwrap();
}

#[cfg(feature = "live-probe")]
#[test]
#[ignore = "requires LIVE=1, an own BOT_LIVE_NAME_PREFIX, GATHERER_WC_BANK_TILE, live-probe, and local 289 engine"]
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
                && witness.death_respawn_tile.is_some()
                && witness.death_respawn_region_unchanged
                && witness.death_recovered.contains(&1)
                && witness.death_products_after.contains(&1)
                && witness.death_xp_after.contains(&1)
                && witness.last_deaths == 1
                && witness.last_recoveries == 1
        }
        LiveCase::DeathNoStock => {
            witness.failure.is_none()
                && witness.failure_code.is_none()
                && witness.native_phases.contains("Blocked")
                && witness.death_no_stock_blocked_running
                && witness
                    .last_status_block_reason
                    .as_deref()
                    .is_some_and(|reason| !reason.trim().is_empty())
                && witness.last_deaths == 1
                && witness.death_recovery_command_sent == 1
                && witness.status_trips == 0
                && witness
                    .last_status_bank
                    .as_deref()
                    .is_some_and(|bank| bank.contains("; access:"))
        }
        LiveCase::DeathReturnRefused => {
            witness.death_return_stop.complete()
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
            let revision = api::selected::ClientRevision::R289;
            let selected = api::game_data::for_revision(revision).unwrap();
            let mut cx = script::native::PrepareContext {
                zones: None,
                revision,
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

    fn open_bank_slot(items: &[(i32, i32)]) -> GatherSlot {
        let (config, mut snapshot) = bank_fixture(items);
        let (_, bank) = bank_fixture(&[(KEBAB_ID, 2)]);
        snapshot.seed_bank_observation(5292, 1, Some(bank.inventory().to_vec()), vec![]);
        let mut slot = GatherSlot::new(
            Cell::Woodcutting,
            LiveCase::WoodcuttingBank,
            DRAYNOR_OAK_BANK_START,
            FixturePlan::default(),
            None,
            false,
        );
        slot.bank_config = Some(config);
        slot.snapshot = snapshot;
        slot
    }

    fn bank_status(
        step: &str,
        event: &str,
        deposited: i64,
        trips: i64,
    ) -> script::native::ScriptStatus {
        use script::native::{NativePhase, ScriptStatus, StatusField, StatusValue};
        ScriptStatus {
            survival: Default::default(),
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
                    value: StatusValue::Text(format!("Draynor; {step}").into()),
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
                StatusField {
                    key: "trips",
                    label: "Trips",
                    value: StatusValue::Integer(trips),
                },
            ]),
        }
    }

    fn unopened_bank_slot() -> GatherSlot {
        let mut slot = open_bank_slot(&[]);
        let (_, snapshot) = bank_fixture(&[]);
        slot.snapshot = snapshot;
        slot
    }

    fn bank_close_status(bank: &str) -> script::native::ScriptStatus {
        let mut status = bank_status("Validate", "bank closed; validating equipment", 0, 0);
        Arc::make_mut(&mut status.fields)
            .iter_mut()
            .find(|field| field.key == "bank")
            .unwrap()
            .value = script::native::StatusValue::Text(bank.into());
        status
    }

    #[test]
    fn bank_close_boundary_accepts_wear_only_without_claiming_a_bank_close() {
        let mut slot = unopened_bank_slot();
        slot.apply_status(&bank_close_status("—")).unwrap();
        assert!(slot.witness.pending_bank_trip.is_none());
        assert!(!slot.witness.bank_closed_observed);
        assert!(!slot.witness.bank_loaded_observed);
        assert_eq!(slot.witness.status_trips, 0);
    }

    #[test]
    fn bank_close_boundary_rejects_a_selected_bank_without_a_receipt() {
        let mut slot = unopened_bank_slot();
        assert!(slot
            .apply_status(&bank_close_status("Draynor; Validate"))
            .is_err());
        assert!(!slot.witness.bank_closed_observed);
    }

    #[test]
    fn bank_close_boundary_rejects_a_missing_bank_field_without_a_receipt() {
        let mut slot = unopened_bank_slot();
        let mut status = bank_close_status("Draynor; Validate");
        Arc::make_mut(&mut status.fields)
            .iter_mut()
            .find(|field| field.key == "bank")
            .unwrap()
            .key = "not_bank";
        assert!(slot.apply_status(&status).is_err());
        assert!(!slot.witness.bank_closed_observed);
    }

    #[test]
    fn bank_close_boundary_rejects_an_unverified_receipt_without_a_selected_bank() {
        let mut slot = open_bank_slot(&[(KEBAB_ID, 4)]);
        slot.observe_running(None, slot.capture()).unwrap();
        let receipt = slot.witness.pending_bank_trip.as_ref().unwrap();
        assert!(!receipt.deposit_verified);
        let (_, snapshot) = bank_fixture(&[(KEBAB_ID, 4)]);
        slot.snapshot = snapshot;
        assert!(slot.apply_status(&bank_close_status("—")).is_err());
        assert!(!slot.witness.bank_closed_observed);
    }

    #[test]
    fn bank_close_boundary_credits_a_verified_bank_receipt() {
        let mut slot = open_bank_slot(&[]);
        slot.observe_running(None, slot.capture()).unwrap();
        slot.apply_status(&bank_close_status("Draynor; Validate"))
            .unwrap();
        let receipt = slot.witness.pending_bank_trip.as_ref().unwrap();
        assert!(receipt.deposit_verified);
        assert!(slot.witness.bank_closed_observed);
    }

    #[test]
    fn bank_open_boundary_accepts_an_empty_supply_deposit_without_a_deposit_stage() {
        let mut slot = open_bank_slot(&[]);
        slot.observe_running(None, slot.capture()).unwrap();
        slot.apply_status(&bank_status("Withdraw", "bank trip due", 0, 0))
            .unwrap();
        slot.apply_status(&bank_status(
            "Wear",
            "bank closed; validating equipment",
            0,
            0,
        ))
        .unwrap();
        slot.apply_status(&bank_status("Idle", "returned to work area", 0, 1))
            .unwrap();
        assert_eq!(slot.witness.status_trips, 1);
        assert!(slot.witness.bank_trips.is_empty());
    }

    #[test]
    fn bank_open_boundary_accepts_only_the_observed_item_count_increase() {
        let mut slot = open_bank_slot(&[(KEBAB_ID, 4)]);
        slot.observe_running(None, slot.capture()).unwrap();
        let receipt = slot.witness.pending_bank_trip.as_ref().unwrap();
        assert_eq!(receipt.expected_items, counts(&[(KEBAB_ID, 4)]));
        assert_eq!(receipt.bank_before, counts(&[(KEBAB_ID, 2)]));
        let (_, bank) = bank_fixture(&[(KEBAB_ID, 6)]);
        slot.snapshot.seed_inventory(vec![], 28);
        slot.snapshot
            .seed_bank_observation(5292, 2, Some(bank.inventory().to_vec()), vec![]);
        slot.apply_status(&bank_status("Supply", "deposit confirmed", 4, 0))
            .unwrap();
        slot.apply_status(&bank_status(
            "Wear",
            "bank closed; validating equipment",
            4,
            0,
        ))
        .unwrap();
        slot.apply_status(&bank_status("Idle", "returned to work area", 4, 1))
            .unwrap();
        assert_eq!(slot.witness.bank_trips.len(), 1);
        assert!(slot.witness.bank_trips[0].deposit_verified);
    }

    #[test]
    fn bank_open_boundary_rejects_items_that_vanish_without_a_bank_increase() {
        let mut slot = open_bank_slot(&[(KEBAB_ID, 4)]);
        slot.observe_running(None, slot.capture()).unwrap();
        slot.snapshot.seed_inventory(vec![], 28);
        slot.apply_status(&bank_status("Supply", "deposit confirmed", 4, 0))
            .unwrap();
        assert!(slot
            .apply_status(&bank_status(
                "Wear",
                "bank closed; validating equipment",
                4,
                0,
            ))
            .is_err());
    }

    fn real_death_slot_at_bank() -> GatherSlot {
        let (config, mut snapshot) = bank_fixture(&[(KEBAB_ID, 2), (BEER_ID, 1), (1521, 3)]);
        let (_, bank) = bank_fixture(&[(KEBAB_ID, 5), (BEER_ID, 7), (1521, 2)]);
        snapshot.seed_bank_observation(5292, 1, Some(bank.inventory().to_vec()), vec![]);
        let (target, plan, task) =
            real_content_death_fixture(Cell::Woodcutting, LiveCase::DeathReturn).unwrap();
        let mut slot = GatherSlot::new(
            Cell::Woodcutting,
            LiveCase::DeathReturn,
            target,
            plan,
            task,
            false,
        );
        slot.bank_config = Some(config);
        slot.snapshot = snapshot;
        slot
    }

    fn reprovision_deposit_observation(
        slot: &mut GatherSlot,
        bank_after: &[(i32, i32)],
    ) -> Observation {
        let before = slot.capture();
        slot.observe_running(None, before.clone()).unwrap();
        let (_, bank) = bank_fixture(bank_after);
        slot.snapshot.seed_inventory(vec![], 28);
        slot.snapshot
            .seed_bank_observation(5292, 2, Some(bank.inventory().to_vec()), vec![]);
        let after = slot.capture();
        slot.observe_running(Some(before), after.clone()).unwrap();
        assert_eq!(slot.unsettled_items.len(), 3);
        assert_eq!(slot.witness.confirmed_drops, 0);
        for item in slot.unsettled_items.values_mut() {
            item.since = Instant::now() - UNSETTLED_WINDOW;
        }
        after
    }

    #[test]
    fn real_death_reprovision_conservation_accepts_observed_per_item_bank_increases() {
        let mut slot = real_death_slot_at_bank();
        let after =
            reprovision_deposit_observation(&mut slot, &[(KEBAB_ID, 7), (BEER_ID, 8), (1521, 5)]);
        slot.observe_running(Some(after), slot.capture()).unwrap();
        assert!(slot.unsettled_items.is_empty());
        let evidence = &slot.witness.death_reprovision_deposits[0];
        assert_eq!(
            evidence.expected_items,
            counts(&[(KEBAB_ID, 2), (BEER_ID, 1), (1521, 3)])
        );
        assert_eq!(
            evidence.bank_before,
            counts(&[(KEBAB_ID, 5), (BEER_ID, 7), (1521, 2)])
        );
        assert_eq!(
            evidence.bank_after,
            counts(&[(KEBAB_ID, 7), (BEER_ID, 8), (1521, 5)])
        );
        assert!(!evidence.accounts_for(KEBAB_ID, 2, evidence.opened_at - Duration::from_secs(1)));
        assert!(!evidence.accounts_for(KEBAB_ID, 2, evidence.observed_at + Duration::from_secs(1)));
    }

    #[test]
    fn real_death_reprovision_conservation_rejects_vanished_missing_or_short_bank_rows() {
        for bank_after in [
            vec![(KEBAB_ID, 5), (BEER_ID, 7), (1521, 2)],
            vec![(KEBAB_ID, 6), (BEER_ID, 8), (1521, 5)],
            vec![(KEBAB_ID, 8), (1521, 5)],
        ] {
            let mut slot = real_death_slot_at_bank();
            let after = reprovision_deposit_observation(&mut slot, &bank_after);
            let error = slot
                .observe_running(Some(after), slot.capture())
                .unwrap_err();
            assert!(error.contains("was not conserved"), "{error}");
            assert!(!slot.unsettled_items.is_empty());
        }
    }

    #[test]
    fn every_real_death_variant_requires_exact_reprovision_bank_increases() {
        for case in [
            LiveCase::DeathReturn,
            LiveCase::DeathRespawnRegion,
            LiveCase::DeathNoStock,
            LiveCase::DeathReturnRefused,
            LiveCase::DeathWatchdogPending,
            LiveCase::DeathWatchdogProving,
        ] {
            let mut slot = real_death_slot_at_bank();
            slot.case = case;
            let after = reprovision_deposit_observation(
                &mut slot,
                &[(KEBAB_ID, 7), (BEER_ID, 8), (1521, 5)],
            );
            slot.observe_running(Some(after), slot.capture()).unwrap();
            assert!(slot.unsettled_items.is_empty(), "{case:?}");
            assert_eq!(slot.witness.death_reprovision_deposits.len(), 1);

            let mut missing = real_death_slot_at_bank();
            missing.case = case;
            let after = reprovision_deposit_observation(
                &mut missing,
                &[(KEBAB_ID, 6), (BEER_ID, 8), (1521, 5)],
            );
            let error = missing
                .observe_running(Some(after), missing.capture())
                .unwrap_err();
            assert!(error.contains("was not conserved"), "{case:?}: {error}");
        }
    }

    fn native_deposit_ids(items: &[(i32, i32)]) -> Arc<[i32]> {
        let (config, snapshot) = bank_fixture(items);
        script::gatherer::test_bank_deposit_ids(&config, &snapshot).unwrap()
    }

    #[test]
    fn woodcutting_status_gate_rejects_kebab_left_after_deposit() {
        if !api::test_support::game_data_available() {
            return;
        }
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
            survival: script::survival::SurvivalReport::default(),
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
        if !api::test_support::game_data_available() {
            return;
        }
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
        if !api::test_support::game_data_available() {
            return;
        }
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

    /// `macro_triffidfruit` (`content/pack/obj.pack:465`), the Strange fruit
    /// that landed in the review's cell b pack: a non-product the Gatherer
    /// banks with the catch.
    const STRANGE_FRUIT_ID: i32 = 464;
    const NET_ID: i32 = 303;

    /// A fish-bank-soak slot at the Deposit step of a trip with the user's
    /// configuration: the net, 26 raw shrimp and one incidental fruit in the
    /// pack, an empty bank open and loaded.
    fn soak_slot_at_deposit() -> GatherSlot {
        use script::native::{NativePhase, ScriptStatus, StatusField, StatusValue};
        let settings = Cell::Fishing.settings(LiveCase::FishBankSoak);
        let config = api::selected::FamilyPreparation::run(|families| {
            let revision = api::selected::ClientRevision::R289;
            let selected = api::game_data::for_revision(revision).unwrap();
            let mut cx = script::native::PrepareContext {
                zones: None,
                revision,
                selected,
                banks: Arc::default(),
                families,
            };
            (script::gatherer::CARD.prepare)(&mut cx, 1, Arc::new(settings)).unwrap()
        })
        .unwrap()
        .join()
        .unwrap();
        let mut slot = GatherSlot::new(
            Cell::Fishing,
            LiveCase::FishBankSoak,
            CATHERBY_HARPOON_START,
            FixturePlan::default(),
            None,
            false,
        );
        slot.bank_config = Some(config);
        slot.started = true;
        let mut pack = vec![(NET_ID, 1)];
        pack.extend(std::iter::repeat_n((SHRIMP_ID, 1), 26));
        pack.push((STRANGE_FRUIT_ID, 1));
        slot.snapshot.seed_inventory(inventory_rows(&pack), 28);
        slot.snapshot
            .seed_bank_observation(5292, 1, Some(vec![]), vec![]);
        let status = ScriptStatus {
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
                    value: StatusValue::Text(
                        "Catherby; Reachable; access:2809,3441,0; Deposit".into(),
                    ),
                },
                StatusField {
                    key: "last_event",
                    label: "Event",
                    value: StatusValue::Text("bank selected".into()),
                },
                StatusField {
                    key: "deposited",
                    label: "Deposited",
                    value: StatusValue::Integer(0),
                },
                StatusField {
                    key: "trips",
                    label: "Trips",
                    value: StatusValue::Integer(0),
                },
            ]),
            survival: script::survival::SurvivalReport::default(),
        };
        slot.apply_status(&status).unwrap();
        let receipt = slot.witness.pending_bank_trip.as_ref().unwrap();
        assert_eq!(
            receipt.expected_items,
            counts(&[(SHRIMP_ID, 26), (STRANGE_FRUIT_ID, 1)]),
            "the net is the protected tool; the catch and the fruit are banked"
        );
        assert_eq!(
            receipt.expected_incidentals,
            counts(&[(STRANGE_FRUIT_ID, 1)])
        );
        assert!(receipt.protected_ids.contains(&NET_ID));
        slot
    }

    fn inventory_rows(items: &[(i32, i32)]) -> Vec<api::snapshot::ItemView> {
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
            .collect()
    }

    fn bank_rows(items: &[(i32, i32)]) -> Vec<api::snapshot::ItemView> {
        inventory_rows(items)
            .into_iter()
            .map(|row| api::snapshot::ItemView {
                container: api::snapshot::ItemContainer::Bank,
                ..row
            })
            .collect()
    }

    fn soak_status(
        step: &str,
        event: &str,
        deposited: i64,
        trips: i64,
    ) -> script::native::ScriptStatus {
        use script::native::{NativePhase, ScriptStatus, StatusField, StatusValue};
        ScriptStatus {
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
                    value: StatusValue::Text(
                        format!("Catherby; Reachable; access:2809,3441,0; {step}").into(),
                    ),
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
                StatusField {
                    key: "trips",
                    label: "Trips",
                    value: StatusValue::Integer(trips),
                },
            ]),
            survival: script::survival::SurvivalReport::default(),
        }
    }

    /// Two frames: the rows leave the pack (the first frame records them
    /// as unsettled), then the next frame settles them against the bank.
    fn observe_pack_change(
        slot: &mut GatherSlot,
        pack_after: &[(i32, i32)],
        bank_after: &[(i32, i32)],
    ) -> Result<(), String> {
        let previous = slot.capture();
        slot.snapshot.seed_inventory(inventory_rows(pack_after), 28);
        slot.snapshot
            .seed_bank_observation(5292, 2, Some(bank_rows(bank_after)), vec![]);
        let observation = slot.capture();
        slot.observe_running(Some(previous), observation.clone())?;
        slot.observe_running(Some(observation.clone()), observation)
    }

    fn coin_runes_empty_slot() -> GatherSlot {
        let mut slot = open_bank_slot(&[]);
        slot.case = LiveCase::CoinRunesEmpty;
        let baseline = slot.capture();
        slot.baseline = Some(baseline.clone());
        slot.latest = Some(baseline);
        slot
    }

    fn coin_runes_empty_status(step: &str, code: &str) -> script::native::ScriptStatus {
        let mut status = soak_status(step, "bank trip due", 0, 0);
        status.phase = script::native::NativePhase::Blocked;
        status.failure = Some(script::native::ScriptFailure {
            code: code.into(),
            message: "no Fire rune in the pack, worn, or in the bank".into(),
        });
        status
    }

    #[test]
    fn coin_runes_empty_accepts_a_loaded_supply_shortage() {
        let mut slot = coin_runes_empty_slot();
        slot.apply_status(&coin_runes_empty_status("Supply", "supply-missing"))
            .unwrap();
        slot.qualifies().unwrap();
    }

    #[test]
    fn coin_runes_empty_rejects_the_retired_withdraw_stage() {
        let mut slot = coin_runes_empty_slot();
        assert!(slot
            .apply_status(&coin_runes_empty_status("Withdraw", "supply-missing"))
            .is_err());
        assert!(slot.qualifies().is_err());
    }

    #[test]
    fn coin_runes_empty_rejects_an_unloaded_bank() {
        let mut slot = coin_runes_empty_slot();
        slot.snapshot = GameSnapshot::new();
        assert!(slot
            .apply_status(&coin_runes_empty_status("Supply", "supply-missing"))
            .is_err());
        assert!(slot.qualifies().is_err());
    }

    #[test]
    fn coin_runes_empty_rejects_an_unselected_bank() {
        let mut slot = coin_runes_empty_slot();
        let mut status = coin_runes_empty_status("Supply", "supply-missing");
        let mut fields = status.fields.to_vec();
        fields
            .iter_mut()
            .find(|field| field.key == "bank")
            .unwrap()
            .value = script::native::StatusValue::Text("—".into());
        status.fields = fields.into();
        assert!(slot.apply_status(&status).is_err());
        assert!(slot.qualifies().is_err());
    }

    #[test]
    fn coin_runes_empty_rejects_a_different_failure() {
        let mut slot = coin_runes_empty_slot();
        assert!(slot
            .apply_status(&coin_runes_empty_status("Supply", "return-failed"))
            .is_err());
        assert!(slot.qualifies().is_err());
    }

    #[test]
    fn coin_runes_empty_rejects_any_trip_yield_or_xp_progress() {
        for (trips, yielded, xp) in [(1, 0, 0), (0, 1, 0), (0, 0, 1)] {
            let mut slot = coin_runes_empty_slot();
            slot.apply_status(&coin_runes_empty_status("Supply", "supply-missing"))
                .unwrap();
            slot.witness.status_trips = trips;
            slot.witness.last_status_yielded = yielded;
            slot.latest.as_mut().unwrap().xp = xp;
            assert!(slot.qualifies().is_err(), "{trips}/{yielded}/{xp}");
        }
    }

    #[test]
    fn bank_selection_observes_the_projection_without_a_retired_event() {
        for kind in ["Reachable", "AirFallback", "NearShortcut"] {
            let mut slot = soak_slot_at_deposit();
            slot.witness.bank_selected = false;
            let mut status = soak_status("Access", "bank trip due", 0, 0);
            let mut fields = status.fields.to_vec();
            fields
                .iter_mut()
                .find(|field| field.key == "bank")
                .unwrap()
                .value = script::native::StatusValue::Text(
                format!("Catherby; {kind}; access:2809,3441,0; Access").into(),
            );
            status.fields = fields.into();
            slot.apply_status(&status).unwrap();
            assert!(slot.complete_bank_selection(), "{kind}");
            assert_eq!(slot.selected_bank_kind(), Some(kind));
        }
    }

    #[test]
    fn bank_selection_rejects_a_retired_event_without_a_selection_projection() {
        let mut slot = soak_slot_at_deposit();
        slot.witness.bank_selected = false;
        let mut status = soak_status("Select", "bank selected", 0, 0);
        let mut fields = status.fields.to_vec();
        fields
            .iter_mut()
            .find(|field| field.key == "bank")
            .unwrap()
            .value = script::native::StatusValue::Text("—".into());
        status.fields = fields.into();
        slot.apply_status(&status).unwrap();
        assert!(!slot.witness.bank_selected);
        assert!(!slot.complete_bank_selection());
    }

    fn slot_before_supply_withdrawal() -> GatherSlot {
        let mut slot = soak_slot_at_deposit();
        slot.snapshot
            .seed_inventory(inventory_rows(&[(NET_ID, 1)]), 28);
        slot.snapshot
            .seed_bank_observation(5292, 1, Some(bank_rows(&[(FEATHERS_ID, 3)])), vec![]);
        slot.witness.pending_bank_trip = None;
        let observation = slot.capture();
        slot.observe_running(None, observation).unwrap();
        slot
    }

    #[test]
    fn bank_withdrawal_requires_matching_pack_and_bank_item_deltas() {
        let mut slot = slot_before_supply_withdrawal();
        observe_pack_change(&mut slot, &[(NET_ID, 1), (FEATHERS_ID, 3)], &[]).unwrap();
        assert!(slot.witness.bank_withdrawal_confirmed);
    }

    #[test]
    fn bank_withdrawal_rejects_an_uncredited_pack_increase() {
        let mut slot = slot_before_supply_withdrawal();
        observe_pack_change(
            &mut slot,
            &[(NET_ID, 1), (FEATHERS_ID, 3)],
            &[(FEATHERS_ID, 3)],
        )
        .unwrap();
        assert!(!slot.witness.bank_withdrawal_confirmed);
    }

    #[test]
    fn bank_withdrawal_rejects_a_different_bank_delta_quantity() {
        let mut slot = slot_before_supply_withdrawal();
        observe_pack_change(
            &mut slot,
            &[(NET_ID, 1), (FEATHERS_ID, 3)],
            &[(FEATHERS_ID, 1)],
        )
        .unwrap();
        assert!(!slot.witness.bank_withdrawal_confirmed);
    }

    #[test]
    fn bank_withdrawal_rejects_a_retired_event_without_item_deltas() {
        let mut slot = slot_before_supply_withdrawal();
        slot.apply_status(&soak_status("Withdraw", "withdrawal confirmed", 0, 0))
            .unwrap();
        assert!(!slot.witness.bank_withdrawal_confirmed);
    }

    fn age_unsettled(slot: &mut GatherSlot) {
        for item in slot.unsettled_items.values_mut() {
            item.since = Instant::now().checked_sub(Duration::from_secs(3)).unwrap();
        }
    }

    /// HOTFIX-0.2.1 review F2: the soak's banked items go through the
    /// per-trip receipt. A correct trip (catch and incidental fruit both
    /// credited in the bank, tally 27, net kept) passes every gate.
    #[test]
    fn fish_bank_soak_oracle_accepts_a_banked_incidental_item() {
        if !api::test_support::game_data_available() {
            return;
        }
        let mut slot = soak_slot_at_deposit();
        observe_pack_change(
            &mut slot,
            &[(NET_ID, 1)],
            &[(SHRIMP_ID, 26), (STRANGE_FRUIT_ID, 1)],
        )
        .unwrap();
        assert!(
            slot.unsettled_items.is_empty(),
            "the fruit and the catch settle against their bank increase: {:?}",
            slot.unsettled_items
        );
        slot.apply_status(&soak_status("Close", "deposit confirmed", 27, 0))
            .unwrap();
        assert!(
            slot.witness
                .pending_bank_trip
                .as_ref()
                .unwrap()
                .deposit_verified
        );
        slot.apply_status(&soak_status(
            "Validate",
            "bank closed; validating equipment",
            27,
            0,
        ))
        .unwrap();
        slot.apply_status(&soak_status("Return", "returning to work area", 27, 1))
            .unwrap();
        assert_eq!(slot.witness.bank_trips.len(), 1);
        assert!(slot.witness.bank_trips[0].deposit_verified);
        assert_eq!(slot.witness.bank_trips[0].deposited_count, 27);
    }

    /// A death recovery's supply trip opens the bank even though the cell is
    /// Power, not bank mode. Depositing a seeded non-product item there is
    /// conservation, not a loss: the item's bank increase settles it.
    #[test]
    fn death_recovery_bank_deposit_conserves_a_seeded_filler() {
        let filler = DEATH_FILLERS[1].0;
        let mut slot = GatherSlot::new(
            Cell::Woodcutting,
            LiveCase::DeathReturn,
            OAK_RESPAWN_START,
            FixturePlan::default(),
            None,
            false,
        );
        slot.snapshot
            .seed_inventory(inventory_rows(&[(LOG_ID, 1), (filler, 1)]), 28);
        slot.snapshot
            .seed_bank_observation(5292, 1, Some(bank_rows(&[])), vec![]);
        let before = slot.capture();
        slot.observe_running(None, before.clone()).unwrap();
        slot.snapshot
            .seed_inventory(inventory_rows(&[(LOG_ID, 1)]), 28);
        slot.snapshot
            .seed_bank_observation(5292, 2, Some(bank_rows(&[(filler, 1)])), vec![]);
        let after = slot.capture();
        slot.observe_running(Some(before), after.clone()).unwrap();
        slot.observe_running(Some(after.clone()), after).unwrap();
        assert!(
            slot.unsettled_items.is_empty(),
            "the deposited filler settles against its bank increase: {:?}",
            slot.unsettled_items
        );
    }

    /// The incidental fruit left the pack but never reached the bank: the
    /// frame oracle keeps it unsettled and the trip is not verified.
    #[test]
    fn fish_bank_soak_oracle_rejects_a_missing_bank_credit() {
        if !api::test_support::game_data_available() {
            return;
        }
        let mut slot = soak_slot_at_deposit();
        observe_pack_change(&mut slot, &[(NET_ID, 1)], &[(SHRIMP_ID, 26)]).unwrap();
        assert_eq!(
            slot.unsettled_items.keys().copied().collect::<Vec<_>>(),
            vec![(STRANGE_FRUIT_ID, 1)]
        );
        age_unsettled(&mut slot);
        let observation = slot.capture();
        let error = slot
            .observe_running(Some(observation.clone()), observation)
            .unwrap_err();
        assert!(
            error.contains("id 464 count 1 was not conserved"),
            "{error}"
        );

        slot.apply_status(&soak_status("Close", "deposit confirmed", 27, 0))
            .unwrap();
        assert!(
            !slot
                .witness
                .pending_bank_trip
                .as_ref()
                .unwrap()
                .deposit_verified
        );
        let error = slot
            .apply_status(&soak_status(
                "Validate",
                "bank closed; validating equipment",
                27,
                0,
            ))
            .unwrap_err();
        assert!(
            error.contains("before its per-trip deposit was verified"),
            "{error}"
        );
    }

    /// The protected net left the pack with the catch: it is not a deposit
    /// candidate, so no bank increase settles it.
    #[test]
    fn fish_bank_soak_oracle_rejects_a_lost_protected_net() {
        if !api::test_support::game_data_available() {
            return;
        }
        let mut slot = soak_slot_at_deposit();
        observe_pack_change(&mut slot, &[], &[(SHRIMP_ID, 26), (STRANGE_FRUIT_ID, 1)]).unwrap();
        assert_eq!(
            slot.unsettled_items.keys().copied().collect::<Vec<_>>(),
            vec![(NET_ID, 1)]
        );
        age_unsettled(&mut slot);
        let observation = slot.capture();
        let error = slot
            .observe_running(Some(observation.clone()), observation)
            .unwrap_err();
        assert!(
            error.contains("id 303 count 1 was not conserved"),
            "{error}"
        );
    }

    /// The Deposit output under-counted the catch (the F1 symptom): the
    /// bank holds everything, but the Gatherer's tally is not the 27 it
    /// held, so the trip is not verified.
    #[test]
    fn fish_bank_soak_oracle_rejects_a_wrong_deposit_tally() {
        if !api::test_support::game_data_available() {
            return;
        }
        let mut slot = soak_slot_at_deposit();
        observe_pack_change(
            &mut slot,
            &[(NET_ID, 1)],
            &[(SHRIMP_ID, 26), (STRANGE_FRUIT_ID, 1)],
        )
        .unwrap();
        slot.apply_status(&soak_status("Close", "deposit confirmed", 1, 0))
            .unwrap();
        let receipt = slot.witness.pending_bank_trip.as_ref().unwrap();
        assert!(!receipt.deposit_count_matches_expected);
        assert!(!receipt.deposit_verified);
        let error = slot
            .apply_status(&soak_status(
                "Validate",
                "bank closed; validating equipment",
                1,
                0,
            ))
            .unwrap_err();
        assert!(
            error.contains("before its per-trip deposit was verified"),
            "{error}"
        );
    }

    /// A recorded block abandons its trip's receipt: the soak keeps counting
    /// blocks instead of failing on the aborted trip's accounting.
    #[test]
    fn fish_bank_soak_block_abandons_the_pending_trip_receipt() {
        if !api::test_support::game_data_available() {
            return;
        }
        let mut slot = soak_slot_at_deposit();
        observe_pack_change(&mut slot, &[(NET_ID, 1)], &[]).unwrap();
        assert_eq!(slot.unsettled_items.len(), 2);
        let status = soak_status("Deposit", "bank selected", 0, 0);
        slot.record_soak_block(
            &status,
            "bank-deposit-failed",
            "bank deposit failed: Failed(\"bank closed before the transfer settled\")",
        );
        assert_eq!(slot.witness.soak.deposit_blocks, 1);
        assert!(slot.witness.soak.block_pending);
        assert!(slot.witness.pending_bank_trip.is_none());
        assert!(slot.unsettled_items.is_empty());
    }

    /// A genie lamp the host rubbed (the live cell `r3nf` of the no-guard
    /// measurement: lamp given on the walk back, `xplamp` opened, "Your
    /// wish has been granted!") leaves the pack by `inv_del`
    /// (`xplamp.rs2:106-114`), not by a deposit. Back from the bank (bank
    /// closed, no trip pending) with the net and `lamps` lamps.
    fn soak_slot_with_lamps(lamps: usize) -> GatherSlot {
        let mut slot = soak_slot_at_deposit();
        slot.snapshot.seed_bank_observation(-1, 3, None, vec![]);
        let mut pack = vec![(NET_ID, 1)];
        pack.extend(std::iter::repeat_n((GENIE_LAMP_ID, 1), lamps));
        slot.snapshot.seed_inventory(inventory_rows(&pack), 28);
        slot.latest = Some(slot.capture());
        slot.witness.pending_bank_trip = None;
        slot
    }

    /// The only lamp leaves the pack: the observation before it and the
    /// observation it left in, not yet run through the oracle.
    fn lamp_leaves(slot: &mut GatherSlot) -> (Observation, Observation) {
        let before = slot.capture();
        slot.snapshot
            .seed_inventory(inventory_rows(&[(NET_ID, 1)]), 28);
        (before, slot.capture())
    }

    /// The lamp stays unaccounted on further observations and fails the
    /// cell at the conservation bound.
    fn assert_lamp_fails_at_the_bound(slot: &mut GatherSlot, observation: &Observation) {
        slot.observe_running(Some(observation.clone()), observation.clone())
            .unwrap();
        assert_eq!(
            slot.unsettled_items.keys().copied().collect::<Vec<_>>(),
            vec![(GENIE_LAMP_ID, 1)]
        );
        age_unsettled(slot);
        let error = slot
            .observe_running(Some(observation.clone()), observation.clone())
            .unwrap_err();
        assert!(
            error.contains("id 2528 count 1 was not conserved"),
            "{error}"
        );
    }

    #[test]
    fn lamp_without_a_wish_fails_at_the_bound() {
        if !api::test_support::game_data_available() {
            return;
        }
        let mut slot = soak_slot_with_lamps(1);
        let (before, left) = lamp_leaves(&mut slot);
        slot.observe_running(Some(before), left.clone()).unwrap();
        assert_lamp_fails_at_the_bound(&mut slot, &left);
    }

    /// `frame` stamps the confirmation after it captured the observation of
    /// the same client drain, before the oracle records the lamp as gone:
    /// the earliest such stamp is the observation's own capture instant.
    #[test]
    fn lamp_settles_on_a_wish_in_the_observation_it_left_in() {
        if !api::test_support::game_data_available() {
            return;
        }
        let mut slot = soak_slot_with_lamps(1);
        let (before, left) = lamp_leaves(&mut slot);
        slot.witness.last_wish_granted = Some(left.captured_at);
        slot.observe_running(Some(before), left.clone()).unwrap();
        assert_eq!(slot.unsettled_items.len(), 1);
        slot.observe_running(Some(left.clone()), left).unwrap();
        assert!(
            slot.unsettled_items.is_empty(),
            "{:?}",
            slot.unsettled_items
        );
        assert_eq!(slot.witness.last_wish_granted, None, "the wish is consumed");
    }

    #[test]
    fn lamp_settles_on_a_wish_after_it_left() {
        if !api::test_support::game_data_available() {
            return;
        }
        let mut slot = soak_slot_with_lamps(1);
        let (before, left) = lamp_leaves(&mut slot);
        slot.observe_running(Some(before), left.clone()).unwrap();
        assert_eq!(slot.unsettled_items.len(), 1);
        let later = slot.capture();
        slot.witness.last_wish_granted = Some(Instant::now());
        slot.observe_running(Some(left), later).unwrap();
        assert!(
            slot.unsettled_items.is_empty(),
            "{:?}",
            slot.unsettled_items
        );
        assert_eq!(slot.witness.last_wish_granted, None, "the wish is consumed");
    }

    /// A wish from any earlier observation, however recent, belongs to
    /// another lamp (HOTFIX-0.2.1 review r2 N1: one second earlier was
    /// accepted by the old two-second window).
    #[test]
    fn lamp_is_not_settled_by_a_recent_earlier_wish() {
        if !api::test_support::game_data_available() {
            return;
        }
        for earlier in [Duration::from_nanos(1), Duration::from_secs(1)] {
            let mut slot = soak_slot_with_lamps(1);
            let (before, left) = lamp_leaves(&mut slot);
            slot.witness.last_wish_granted = Some(left.captured_at.checked_sub(earlier).unwrap());
            slot.observe_running(Some(before), left.clone()).unwrap();
            assert_lamp_fails_at_the_bound(&mut slot, &left);
        }
    }

    #[test]
    fn lamp_is_not_settled_by_an_old_wish() {
        if !api::test_support::game_data_available() {
            return;
        }
        let mut slot = soak_slot_with_lamps(1);
        let (before, left) = lamp_leaves(&mut slot);
        slot.witness.last_wish_granted = Some(
            left.captured_at
                .checked_sub(Duration::from_secs(60))
                .unwrap(),
        );
        slot.observe_running(Some(before), left.clone()).unwrap();
        assert_lamp_fails_at_the_bound(&mut slot, &left);
    }

    /// Two lamps leave one observation apart and one wish arrives with the
    /// second: it settles the first lamp and is consumed, so it cannot also
    /// settle the second, although it belongs to the second's observation.
    #[test]
    fn lamp_is_not_settled_by_a_wish_another_lamp_consumed() {
        if !api::test_support::game_data_available() {
            return;
        }
        let mut slot = soak_slot_with_lamps(2);
        let before = slot.capture();
        slot.snapshot
            .seed_inventory(inventory_rows(&[(NET_ID, 1), (GENIE_LAMP_ID, 1)]), 28);
        let first_left = slot.capture();
        slot.observe_running(Some(before), first_left.clone())
            .unwrap();
        assert_eq!(slot.unsettled_items.len(), 1);
        slot.snapshot
            .seed_inventory(inventory_rows(&[(NET_ID, 1)]), 28);
        let second_left = slot.capture();
        slot.witness.last_wish_granted = Some(second_left.captured_at);
        slot.observe_running(Some(first_left), second_left.clone())
            .unwrap();
        assert_eq!(
            slot.witness.last_wish_granted, None,
            "the first lamp consumed the wish"
        );
        assert_eq!(
            slot.unsettled_items
                .values()
                .map(|item| item.left_in)
                .collect::<Vec<_>>(),
            vec![second_left.captured_at],
            "only the second lamp is outstanding"
        );
        assert_lamp_fails_at_the_bound(&mut slot, &second_left);
    }
}

fn is_mining_gem(id: i32) -> bool {
    MINE_PRODUCTS.contains(&id) && ![COPPER_ID, TIN_ID, IRON_ID, COAL_ID].contains(&id)
}

fn record_bank_return_step(witness: &mut Witness, step: &str) {
    if step == "Return" {
        // Return is persistent runner evidence that Close completed.
        witness.bank_return_step_seen = true;
        witness.bank_closed_observed = true;
    }
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
    let area = area.strip_prefix("site (")?;
    let (coords, radius) = area.split_once(") r")?;
    if radius != "12" {
        return None;
    }
    parse_tile("site area", coords).ok()
}

fn site_nav_destination(nav: &Value) -> Option<WorldTile> {
    let [x, z, level] = nav["requested_destination"].as_array()?.as_slice() else {
        return None;
    };
    Some(WorldTile {
        x: i32::try_from(x.as_i64()?).ok()?,
        z: i32::try_from(z.as_i64()?).ok()?,
        level: i32::try_from(level.as_i64()?).ok()?,
    })
}

fn site_nav_targets_area(nav: &Value, anchor: WorldTile) -> bool {
    site_nav_destination(nav).is_some_and(|goal| {
        goal.level == anchor.level && goal.x.abs_diff(anchor.x).max(goal.z.abs_diff(anchor.z)) <= 12
    })
}

fn site_observation_nav_matches(cell: Cell, nav: &Value, anchor: WorldTile) -> bool {
    nav["armed"] == true
        && nav["requested"] == true
        && nav["request_id"].as_u64().is_some_and(|id| id > 0)
        && nav["arrival"] == "Area"
        && nav["requested_radius"] == if cell == Cell::Fishing { 0 } else { 1 }
        && site_nav_targets_area(nav, anchor)
}

fn site_initial_nav_matches(
    cell: Cell,
    nav: &Value,
    anchor: WorldTile,
    phase: &str,
    target: &str,
) -> bool {
    site_observation_nav_matches(cell, nav, anchor)
        && phase
            == if cell == Cell::Fishing {
                "surveying"
            } else {
                "walking"
            }
        && !target.is_empty()
        && target != "—"
}

fn site_return_nav_matches(cell: Cell, nav: &Value, anchor: WorldTile, target: &str) -> bool {
    let Some(stand) = target
        .split_once("; observation stand:")
        .and_then(|(_, coords)| parse_tile("Site Return stand", coords).ok())
    else {
        return false;
    };
    site_observation_nav_matches(cell, nav, anchor) && site_nav_destination(nav) == Some(stand)
}

fn site_walk_outcome_destination(nav: &Value) -> Option<WorldTile> {
    let [x, z, level] = nav["destination"].as_array()?.as_slice() else {
        return None;
    };
    Some(WorldTile {
        x: i32::try_from(x.as_i64()?).ok()?,
        z: i32::try_from(z.as_i64()?).ok()?,
        level: i32::try_from(level.as_i64()?).ok()?,
    })
}

fn draynor_site_return_nav_matches(
    nav: &Value,
    request: SiteReturnRequest,
    player_tile: Option<WorldTile>,
) -> bool {
    nav["outcome_request_id"].as_u64() == Some(request.request_id)
        && nav["failed"] == false
        && nav["blocked"] == false
        && nav["reason"] == "None"
        && nav["outcome_radius"] == 0
        && site_walk_outcome_destination(nav).is_some_and(|stand| {
            let tile = (stand.x, stand.z, stand.level);
            tile_distance(tile, request.anchor).is_some_and(|distance| distance <= 12)
                && tile_distance(tile, request.preferred_center)
                    .is_some_and(|distance| distance <= 1)
                && player_tile == Some(stand)
        })
}

fn record_draynor_site_return(
    witness: &mut Witness,
    nav: &Value,
    returning: bool,
    target: &str,
    anchor: Option<WorldTile>,
    player_tile: Option<WorldTile>,
) {
    if returning {
        if let Some(anchor) =
            anchor.filter(|&anchor| site_return_nav_matches(Cell::Fishing, nav, anchor, target))
        {
            if let (Some(request_id), Some(preferred_center)) =
                (nav["request_id"].as_u64(), site_nav_destination(nav))
            {
                witness.site_return_request = Some(SiteReturnRequest {
                    request_id,
                    anchor,
                    preferred_center,
                });
            }
        }
    }
    let Some(request) = witness.site_return_request else {
        return;
    };
    if draynor_site_return_nav_matches(nav, request, player_tile) {
        witness.site_return_area = true;
        witness.site_return_arrival.get_or_insert_with(|| {
            json!({
                "request_id": request.request_id,
                "anchor": request.anchor,
                "preferred_center": request.preferred_center,
                "player": player_tile,
                "nav": nav,
            })
        });
    }
}

fn site_bank_complete(
    cell: Cell,
    case: LiveCase,
    witness: &Witness,
    plan: &FixturePlan,
    baseline_xp: i32,
) -> bool {
    witness.site_anchor_selected
        && (case == LiveCase::SiteDraynor || witness.site_initial_walk)
        && witness.site_return_area
        && witness.bank_return_step_seen
        && witness.bank_arrivals >= 1
        && witness.bank_trips.iter().any(BankTripReceipt::complete)
        && witness.bank_nonzero_roundtrips >= 1
        && witness.post_bank_yields >= 1
        && witness.last_xp > baseline_xp
        && cell
            .products(case)
            .iter()
            .any(|id| witness.products_seen.contains(id))
        && witness.last_status_bank.as_deref().is_some_and(|bank| {
            bank.starts_with(cell.site_bank_for_case(case)) && bank.contains("; Reachable;")
        })
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
fn draynor_oak_lifecycle_requires_bank_close_and_fresh_post_bank_yield() {
    let mut witness = Witness {
        status_trips: 2,
        bank_arrivals: 2,
        status_deposited: 28,
        bank_nonzero_roundtrips: 1,
        post_bank_yields: 2,
        bank_loaded_observed: true,
        bank_withdrawal_confirmed: true,
        last_status_yielded: 29,
        last_xp: 3498,
        last_status_bank: Some("Draynor; Reachable; access:3092,3243,0; Return".into()),
        last_event: Some("returning to work area".into()),
        ..Witness::default()
    };
    witness.products_seen.insert(1521);
    witness.oak_live_tiles.insert((3106, 3247, 0));
    let plan = FixturePlan::default();
    let target = world_tile(3106, 3247);
    let baseline_xp = 2411;

    record_bank_return_step(&mut witness, "Return");
    assert!(witness.bank_closed_observed);
    assert!(real_content_bank_lifecycle_complete(
        LiveCase::BankCostFirstGoal,
        &witness,
        &plan,
        target,
        baseline_xp,
    ));

    let valid_xp = witness.last_xp;
    for xp in [baseline_xp, baseline_xp - 1] {
        witness.last_xp = xp;
        assert!(
            witness.post_bank_yields >= 1,
            "the negative XP case keeps a fresh post-bank yield"
        );
        assert!(
            !real_content_bank_lifecycle_complete(
                LiveCase::BankCostFirstGoal,
                &witness,
                &plan,
                target,
                baseline_xp,
            ),
            "XP at or below the baseline must not pass with a fresh yield"
        );
    }
    witness.last_xp = valid_xp;

    witness.post_bank_yields = 0;
    witness.last_status_yielded = 28;
    witness.last_xp = 3423;
    assert!(
        !real_content_bank_lifecycle_complete(
            LiveCase::BankCostFirstGoal,
            &witness,
            &plan,
            target,
            baseline_xp,
        ),
        "a bank return without a fresh gather must not pass"
    );

    witness.post_bank_yields = 2;
    witness.last_status_yielded = 29;
    witness.last_xp = 3498;
    witness.bank_return_step_seen = false;
    witness.bank_closed_observed = false;
    witness.last_status_bank = Some("Draynor; Reachable; access:3092,3243,0; Close".into());
    witness.last_event = Some("withdrawal confirmed".into());
    record_bank_return_step(&mut witness, "Close");
    assert!(!witness.bank_closed_observed);
    assert!(
        !real_content_bank_lifecycle_complete(
            LiveCase::BankCostFirstGoal,
            &witness,
            &plan,
            target,
            baseline_xp,
        ),
        "a run that did not leave the open-bank Close step must not pass"
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
mod bank_closure_oracle_tests {
    use super::*;

    fn slot() -> GatherSlot {
        let mut slot = GatherSlot::new(
            Cell::Woodcutting,
            LiveCase::BankCostFirstGoal,
            OAK_RESPAWN_START,
            FixturePlan::default(),
            None,
            false,
        );
        slot.snapshot.seed_ingame(2);
        // Keep the existing deposit guard active. Verify a real fixture
        // transfer before testing either the legacy text or decoded closure.
        let mut receipt = BankTripReceipt::new(
            1,
            BTreeMap::from([(1521, 1)]),
            &[1521],
            BTreeMap::new(),
            0,
            &[1521],
            0,
        );
        receipt
            .observe_deposit(
                &BTreeMap::new(),
                &BTreeMap::new(),
                true,
                &BTreeMap::from([(1521, 1)]),
                1,
                true,
            )
            .unwrap();
        assert!(receipt.deposit_verified);
        slot.witness.pending_bank_trip = Some(receipt);
        slot
    }

    fn observe(slot: &mut GatherSlot, component_id: i32, event: &'static str) {
        use script::native::{NativePhase, ScriptStatus, StatusField, StatusValue};
        slot.snapshot
            .seed_bank_observation(component_id, 1, Some(vec![]), vec![]);
        let status = ScriptStatus {
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
                    value: StatusValue::Text(
                        "Draynor; Reachable; access:3092,3245; Validate".into(),
                    ),
                },
                StatusField {
                    key: "last_event",
                    label: "Event",
                    value: StatusValue::Text(event.into()),
                },
            ]),
            survival: script::survival::SurvivalReport::default(),
        };
        slot.apply_status(&status).unwrap();
    }

    #[test]
    fn bank_closure_survives_a_missed_status_pulse() {
        let mut slot = slot();
        observe(&mut slot, 5292, "withdrawal confirmed");
        assert!(!slot.witness.bank_closed_observed);
        observe(&mut slot, -1, "wielding tool");
        assert!(slot.witness.bank_closed_observed);
    }

    #[test]
    fn bank_closure_requires_a_decoded_open_to_closed_transition() {
        let mut slot = slot();
        observe(&mut slot, -1, "bank closed; validating equipment");
        assert!(!slot.witness.bank_closed_observed);
        observe(&mut slot, 5292, "bank closed; validating equipment");
        assert!(!slot.witness.bank_closed_observed);
    }

    #[test]
    fn bank_closure_does_not_bridge_an_offline_frame() {
        let mut slot = slot();
        observe(&mut slot, 5292, "withdrawal confirmed");
        slot.snapshot = GameSnapshot::new();
        observe(&mut slot, -1, "waiting for world");
        assert!(!slot.witness.bank_closed_observed);
        slot.snapshot.seed_ingame(2);
        observe(&mut slot, -1, "wielding tool");
        assert!(!slot.witness.bank_closed_observed);
        observe(&mut slot, 5292, "withdrawal confirmed");
        observe(&mut slot, -1, "wielding tool");
        assert!(slot.witness.bank_closed_observed);
    }

    #[test]
    fn bank_closure_does_not_bridge_login_sessions() {
        let mut slot = slot();
        observe(&mut slot, 5292, "withdrawal confirmed");
        let mut gens = slot.snapshot.gens();
        gens.session += 1;
        slot.snapshot.reset_session(gens);
        slot.snapshot.seed_ingame(2);
        observe(&mut slot, -1, "wielding tool");
        assert!(!slot.witness.bank_closed_observed);
        observe(&mut slot, 5292, "withdrawal confirmed");
        observe(&mut slot, -1, "wielding tool");
        assert!(slot.witness.bank_closed_observed);
    }

    #[test]
    fn bank_closure_never_closes_still_fails_the_unchanged_terminal() {
        let mut slot = slot();
        slot.witness = Witness {
            status_trips: 1,
            status_deposited: 1,
            bank_nonzero_roundtrips: 1,
            post_bank_yields: 1,
            bank_loaded_observed: true,
            bank_withdrawal_confirmed: true,
            last_status_yielded: 1,
            last_xp: 1,
            ..slot.witness
        };
        slot.witness.products_seen.insert(1521);
        slot.witness.oak_live_tiles.insert((
            OAK_RESPAWN_START.x,
            OAK_RESPAWN_START.z,
            OAK_RESPAWN_START.level,
        ));
        for _ in 0..3 {
            observe(&mut slot, 5292, "bank closed; validating equipment");
            assert!(!real_content_bank_lifecycle_complete(
                slot.case,
                &slot.witness,
                &slot.plan,
                slot.target,
                0,
            ));
        }
        observe(&mut slot, -1, "wielding tool");
        assert!(
            real_content_bank_lifecycle_complete(
                slot.case,
                &slot.witness,
                &slot.plan,
                slot.target,
                0,
            ),
            "all other terminal facts were already satisfied before decoded closure"
        );
    }
}

#[cfg(test)]
mod woodcutting_wield_oracle_tests {
    use super::*;

    fn slot_for(case: LiveCase) -> GatherSlot {
        GatherSlot::new(
            Cell::Woodcutting,
            case,
            OAK_RESPAWN_START,
            FixturePlan::default(),
            None,
            false,
        )
    }

    fn tool_status(tool: String) -> script::native::ScriptStatus {
        use script::native::{NativePhase, ScriptStatus, StatusField, StatusValue};
        ScriptStatus {
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
            fields: Arc::from([StatusField {
                key: "tool",
                label: "Tool",
                value: StatusValue::Text(tool.into()),
            }]),
            survival: script::survival::SurvivalReport::default(),
        }
    }

    fn equipment_row(id: i32) -> api::snapshot::ItemView {
        api::snapshot::ItemView {
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
            container: api::snapshot::ItemContainer::Equipment,
            action_family: api::snapshot::ItemActionFamily::Held,
            slot: 3,
            count: 1,
            actions: vec![],
            component_id: -1,
        }
    }

    fn pack_row(id: i32) -> api::snapshot::ItemView {
        api::snapshot::ItemView {
            container: api::snapshot::ItemContainer::Inventory,
            slot: 0,
            ..equipment_row(id)
        }
    }

    /// A constructed passing witness: the first pack-unworn sighting was
    /// stamped before the first worn one, as a status row pair would.
    fn wielded_witness() -> Witness {
        Witness {
            tool_selected_observed: true,
            tool_sighting_count: 2,
            tool_held_not_worn_first: Some(1),
            tool_held_not_worn_source: Some(ToolSightingSource::Status),
            tool_worn_first: Some(2),
            tool_worn_source: Some(ToolSightingSource::Status),
            ..Witness::default()
        }
    }

    /// The failed WORLD-STATE-COST run in witness form: the axe was seen in
    /// the pack and not worn (the bank-close row), then seen worn. No
    /// `last_event = "wielding tool"` sample exists, and none is required.
    #[test]
    fn wield_oracle_accepts_a_fetched_axe_worn_without_an_event_sample() {
        let mut slot = slot_for(LiveCase::WoodcuttingBank);
        slot.witness = wielded_witness();
        assert!(
            slot.witness.status_values.is_empty(),
            "no last_event sample may be required"
        );
        assert!(!slot.witness.tool_worn_at_start);
        assert!(slot.qualifies_wield().is_ok());
    }

    /// An axe that never leaves the pack is a correct refusal.
    #[test]
    fn wield_oracle_rejects_an_axe_never_worn() {
        let mut slot = slot_for(LiveCase::WoodcuttingBank);
        let mut witness = wielded_witness();
        witness.tool_worn_first = None;
        witness.tool_worn_source = None;
        slot.witness = witness;
        let error = slot.qualifies_wield().unwrap_err();
        assert!(error.contains("did not fetch"), "{error}");
    }

    /// A run seeded with the axe already worn did not wield it, even though
    /// the worn state is observed for the whole run.
    #[test]
    fn wield_oracle_rejects_an_axe_already_worn_at_start() {
        let mut slot = slot_for(LiveCase::WoodcuttingBank);
        let mut witness = wielded_witness();
        witness.tool_worn_at_start = true;
        slot.witness = witness;
        let error = slot.qualifies_wield().unwrap_err();
        assert!(error.contains("did not fetch"), "{error}");
    }

    /// A worn sighting with no pack-unworn sighting at all is a refusal:
    /// the durable facts must show the fetch, not only the wear.
    #[test]
    fn wield_oracle_rejects_a_worn_axe_with_no_unworn_sighting() {
        let mut slot = slot_for(LiveCase::WoodcuttingBank);
        slot.witness = Witness {
            tool_selected_observed: true,
            tool_sighting_count: 1,
            tool_worn_first: Some(1),
            tool_worn_source: Some(ToolSightingSource::Status),
            ..Witness::default()
        };
        let error = slot.qualifies_wield().unwrap_err();
        assert!(error.contains("did not fetch"), "{error}");
    }

    /// The reviewer's constructed probe: a worn sighting is stamped first and
    /// a bare pack sighting later. The old oracle refused this run because it
    /// never sampled a `wielding tool` event; the ordered witness refuses it
    /// because the first wear was not preceded by a pack sighting.
    #[test]
    fn wield_oracle_rejects_a_worn_sighting_before_the_unworn_one() {
        let mut slot = slot_for(LiveCase::WoodcuttingBank);
        slot.witness = Witness {
            tool_selected_observed: true,
            tool_sighting_count: 2,
            tool_worn_first: Some(1),
            tool_worn_source: Some(ToolSightingSource::Status),
            tool_held_not_worn_first: Some(2),
            tool_held_not_worn_source: Some(ToolSightingSource::Status),
            ..Witness::default()
        };
        let error = slot.qualifies_wield().unwrap_err();
        assert!(error.contains("did not fetch"), "{error}");
    }

    /// The unwieldable sibling keeps its old rejection unchanged: the rune
    /// axe observed worn always fails, and the same witness without the wear
    /// passes.
    #[test]
    fn wield_oracle_keeps_the_unwieldable_gate() {
        let mut slot = slot_for(LiveCase::WoodcuttingBankUnwieldable);
        slot.witness = wielded_witness();
        let error = slot.qualifies_wield().unwrap_err();
        assert!(error.contains("unwieldable"), "{error}");
        slot.witness.tool_worn_first = None;
        slot.witness.tool_worn_source = None;
        assert!(slot.qualifies_wield().is_ok());
    }

    /// The durable rows the real run delivered: bank close with the bare tool
    /// id, then the worn id. Applying them in order satisfies the oracle
    /// without any transient event sample.
    #[test]
    fn wield_witness_records_the_bank_close_row_then_the_worn_row() {
        let mut slot = slot_for(LiveCase::WoodcuttingBank);
        let tool_id = slot.tool_id;
        slot.apply_status(&tool_status(tool_id.to_string()))
            .unwrap();
        assert!(slot.witness.tool_selected_observed);
        assert_eq!(slot.witness.tool_held_not_worn_first, Some(1));
        assert_eq!(
            slot.witness.tool_held_not_worn_source,
            Some(ToolSightingSource::Status)
        );
        assert_eq!(slot.witness.tool_worn_first, None);
        slot.apply_status(&tool_status(format!("{tool_id} (worn)")))
            .unwrap();
        assert_eq!(slot.witness.tool_worn_first, Some(2));
        assert_eq!(
            slot.witness.tool_worn_source,
            Some(ToolSightingSource::Status)
        );
        assert!(slot.qualifies_wield().is_ok());
    }

    /// The frame recorder, not the status path, supplies both sightings: the
    /// axe in the pack unworn, then the same axe in the equipment. A frame
    /// after the wear cannot reorder the proof, and the oracle accepts.
    #[test]
    fn wield_witness_orders_the_frame_pack_then_equipment() {
        let mut slot = slot_for(LiveCase::WoodcuttingBank);
        let tool_id = slot.tool_id;
        let before = slot.capture();
        slot.snapshot.seed_inventory(vec![pack_row(tool_id)], 28);
        let held = slot.capture();
        slot.observe_running(Some(before), held.clone()).unwrap();
        assert_eq!(slot.witness.tool_held_not_worn_first, Some(1));
        assert_eq!(
            slot.witness.tool_held_not_worn_source,
            Some(ToolSightingSource::Frame)
        );
        assert_eq!(slot.witness.tool_worn_first, None);
        slot.snapshot.seed_inventory(Vec::new(), 28);
        slot.snapshot.seed_equipment(vec![equipment_row(tool_id)]);
        let worn = slot.capture();
        slot.observe_running(Some(held), worn).unwrap();
        assert_eq!(slot.witness.tool_worn_first, Some(2));
        assert_eq!(
            slot.witness.tool_worn_source,
            Some(ToolSightingSource::Frame)
        );
        assert!(slot.qualifies_wield().is_ok());
    }

    /// Start records the seeded state before the script runs: a worn axe sets
    /// the start fact and the existing Start baseline check refuses the run.
    #[test]
    fn wield_witness_records_a_seeded_worn_axe_at_start() {
        let mut slot = slot_for(LiveCase::WoodcuttingBank);
        slot.phase = Prep::Ready;
        slot.latest = Some(slot.capture());
        let tool_id = slot.tool_id;
        slot.snapshot.seed_equipment(vec![equipment_row(tool_id)]);
        let error = slot.mark_started().unwrap_err();
        assert!(
            error.contains("Start baseline lacks its required held/banked tool fixture"),
            "{error}"
        );
        assert!(slot.witness.tool_worn_at_start);
        assert_eq!(slot.witness.tool_worn_first, Some(1));
        assert_eq!(
            slot.witness.tool_worn_source,
            Some(ToolSightingSource::Start)
        );
        assert_eq!(slot.witness.tool_held_not_worn_first, None);
    }
}

#[cfg(test)]
mod fish_spot_move_tests {
    use super::*;

    fn fish(x: i32) -> FishTarget {
        FishTarget {
            type_id: 320,
            tile: (x, 3439, 0),
        }
    }

    #[test]
    fn catherby_moves_fixture_matches_the_report_without_spawning_resources() {
        let settings = Cell::Fishing.settings(LiveCase::FishSpotMoves);
        assert_eq!(settings.get("location"), Some(&json!("Site")));
        assert_eq!(settings.get("site"), Some(&json!("fishing.catherby")));
        assert_eq!(settings.get("radius"), Some(&json!(32)));
        assert_eq!(
            settings.get("fishingMethod"),
            Some(&json!("fishing.saltfish.op1"))
        );
        assert_eq!(settings.get("disposition"), Some(&json!("Bank")));
        assert_eq!(settings.get("bank"), Some(&json!("Nearest")));
        assert!(LiveCase::FishSpotMoves.is_bank_mode());
        let (_, plan, task) = fixture_plan(Cell::Fishing, LiveCase::FishSpotMoves).unwrap();
        assert!(plan.seed_locs.is_empty());
        assert!(task.is_none());
    }

    #[test]
    fn move_oracle_requires_instance_movement_and_fresh_fishing_after_each_move() {
        let mut witness = FishMovementWitness::default();
        witness.observe(
            &BTreeMap::from([(1, fish(2836)), (2, fish(2838))]),
            Some(1),
            10,
            100,
        );
        // Switching targets is not a content move.
        witness.observe(
            &BTreeMap::from([(1, fish(2836)), (2, fish(2838))]),
            Some(2),
            11,
            110,
        );
        assert!(witness.moves.is_empty());
        // The now inactive spot moving is not an interruption either.
        witness.observe(
            &BTreeMap::from([(1, fish(2844)), (2, fish(2838))]),
            Some(2),
            12,
            120,
        );
        assert!(witness.moves.is_empty());
        witness.observe(&BTreeMap::from([(2, fish(2845))]), None, 20, 120);
        assert_eq!(witness.moves.len(), 1);
        witness.observe(&BTreeMap::from([(2, fish(2845))]), Some(2), 21, 120);
        assert!(
            witness.moves[0].resumed_tick.is_none(),
            "retargeting without XP is not progress"
        );
        witness.observe(&BTreeMap::from([(2, fish(2845))]), Some(2), 22, 130);
        assert_eq!(witness.moves[0].resumed_tick, Some(22));
        assert!(!witness.complete(), "one move does not qualify");
        witness.observe(&BTreeMap::from([(2, fish(2837))]), None, 30, 130);
        witness.observe(&BTreeMap::from([(2, fish(2837))]), None, 31, 140);
        assert!(
            !witness.complete(),
            "XP without a fishing target cannot qualify"
        );
        witness.observe(&BTreeMap::from([(2, fish(2837))]), Some(2), 32, 140);
        assert!(witness.complete());
    }
}

#[cfg(test)]
mod fish_guild_ardougne_fixture_tests {
    use super::*;

    #[test]
    fn guild_ardougne_fixture_uses_site_harpoon_without_spawned_npcs() {
        let case = LiveCase::FishGuildApproach;
        let settings = Cell::Fishing.settings(case);
        assert_eq!(settings.get("skill"), Some(&json!("Fishing")));
        assert_eq!(
            settings.get("fishingMethod"),
            Some(&json!("fishing.rarefish.op3"))
        );
        assert_eq!(settings.get("location"), Some(&json!("Site")));
        assert_eq!(settings.get("site"), Some(&json!("fishing.fishing_guild")));
        assert_eq!(settings.get("disposition"), Some(&json!("Power")));
        assert_eq!(Cell::Fishing.default_level(case), 68);
        assert_eq!(Cell::Fishing.level(case), 68);
        assert_eq!(Cell::Fishing.default_tool_alias(case), "harpoon");
        assert_eq!(Cell::Fishing.default_tool_id(case), 311);
        assert_eq!(Cell::Fishing.products(case), &[TUNA_ID, SWORDFISH_ID]);
        let (start, plan, task) = fixture_plan(Cell::Fishing, case).unwrap();
        assert_eq!(start, ARDOUGNE_GUILD_START);
        assert!(!fishing_guild_area_reached(Some((
            start.x,
            start.z,
            start.level
        ))));
        assert!(plan.inventory_seed.is_empty());
        assert!(plan.bank_seed.is_empty());
        assert!(plan.seed_locs.is_empty());
        assert!(task.is_none());
        assert!(fishing_guild_area_reached(Some((2612, 3412, 0))));
        assert!(!fishing_guild_area_reached(Some((2661, 3306, 0))));
        assert!(
            !fishing_guild_area_reached(Some((2611, 3394, 0))),
            "the south guild door must not count as the fishing area"
        );
    }
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
        assert!(plan.auto_next.is_empty());
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
        assert_eq!(
            fixed_plan.inventory_seed,
            vec![
                ("casket".to_owned(), 1),
                ("net".to_owned(), 1),
                ("beer".to_owned(), 1)
            ]
        );
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
mod auto_search_oracle_tests {
    use super::*;

    #[test]
    fn initial_auto_search_beyond_empty_ring_qualifies_without_retry_event() {
        for area in [
            "auto (3292,3347,0) r32; searched: 64; tried: 0; limit: 128",
            "auto (3294,3331,0) r32; searched: 33; tried: 0; limit: 128",
            "auto (3295,3426,0) r32; searched: 128; tried: 1; limit: 128",
        ] {
            assert!(auto_search_beyond_start_ring(AUTO_START, area), "{area}");
        }
    }

    #[test]
    fn auto_search_rejects_unresolved_or_in_ring_anchor() {
        for area in [
            "auto unresolved; searched: 64; tried: 0; limit: 128",
            "start (3292,3347,0) r32; searched: 64; tried: 0; limit: 128",
            "site (3292,3347,0) r32; searched: 64; tried: 0; limit: 128",
            "auto (3295,3298,0) r32; searched: 64; tried: 0; limit: 128",
            "auto (3295,3330,0) r32; searched: 64; tried: 0; limit: 128",
            "auto (3292,3347,1) r32; searched: 64; tried: 0; limit: 128",
        ] {
            assert!(!auto_search_beyond_start_ring(AUTO_START, area), "{area}");
        }
    }

    #[test]
    fn auto_search_rejects_unproven_or_out_of_bounds_search() {
        for area in [
            "auto (3292,3347,0) r32",
            "auto (3292,3347,0) r32; searched: 32; tried: 0; limit: 128",
            "auto (3292,3347,0) r32; searched: 129; tried: 0; limit: 128",
            "auto (3292,3347,0) r32; searched: unknown; tried: 0; limit: 128",
            "auto (3292,3347,0) r32; searched: 64; tried: 0; limit: 256",
            "auto (3292,3347,0) r31; searched: 64; tried: 0; limit: 128",
            "auto (3292,3347,0) r32; searched: 64; tried: unknown; limit: 128",
            "auto (3295,3427,0) r32; searched: 128; tried: 0; limit: 128",
            "auto (3295,3363,0) r32; searched: 64; tried: 0; limit: 128",
        ] {
            assert!(!auto_search_beyond_start_ring(AUTO_START, area), "{area}");
        }
    }
}

#[cfg(test)]
mod gatherer_real_content_fixture_tests {
    use super::*;
    use api::query::{ArrivalProbe, CollisionArrivalProbe, SceneReachOptions};

    fn auto_test_build() -> BuildRect {
        BuildRect {
            base_x: 3240,
            base_z: 3248,
            width: 104,
            height: 104,
            level: 0,
        }
    }

    fn auto_real_scene_trees() -> BTreeSet<(i32, i32, i32)> {
        // Complete ground-plane 1276/1278 placements in this 104x104 scene,
        // from the authored map census m50_50 through m52_52.
        BTreeSet::from([
            tile_key(world_tile(3241, 3268)),
            tile_key(world_tile(3241, 3332)),
            tile_key(world_tile(3243, 3272)),
            tile_key(world_tile(3244, 3314)),
            tile_key(world_tile(3245, 3253)),
            tile_key(world_tile(3245, 3267)),
            tile_key(world_tile(3246, 3256)),
            tile_key(world_tile(3246, 3259)),
            tile_key(world_tile(3248, 3340)),
            tile_key(world_tile(3256, 3326)),
            tile_key(world_tile(3258, 3252)),
            tile_key(world_tile(3260, 3252)),
            tile_key(world_tile(3260, 3326)),
            tile_key(world_tile(3262, 3249)),
            tile_key(world_tile(3267, 3337)),
            tile_key(world_tile(3267, 3340)),
            tile_key(world_tile(3270, 3337)),
            tile_key(world_tile(3270, 3341)),
            tile_key(world_tile(3272, 3344)),
            tile_key(world_tile(3273, 3335)),
            tile_key(world_tile(3273, 3340)),
            tile_key(world_tile(3274, 3347)),
            tile_key(world_tile(3275, 3337)),
            tile_key(world_tile(3275, 3344)),
            tile_key(world_tile(3278, 3342)),
            tile_key(world_tile(3278, 3348)),
            tile_key(world_tile(3279, 3351)),
            tile_key(world_tile(3282, 3336)),
            tile_key(world_tile(3283, 3348)),
            tile_key(world_tile(3290, 3340)),
            tile_key(world_tile(3290, 3343)),
            tile_key(world_tile(3293, 3337)),
            tile_key(world_tile(3294, 3331)),
            tile_key(world_tile(3295, 3351)),
            tile_key(world_tile(3296, 3342)),
            tile_key(world_tile(3298, 3331)),
            tile_key(world_tile(3299, 3341)),
            tile_key(world_tile(3301, 3346)),
            tile_key(world_tile(3305, 3332)),
            tile_key(world_tile(3305, 3340)),
            tile_key(world_tile(3306, 3350)),
            tile_key(world_tile(3309, 3344)),
            tile_key(world_tile(3312, 3349)),
            tile_key(world_tile(3314, 3348)),
            tile_key(world_tile(3315, 3344)),
            tile_key(world_tile(3319, 3349)),
            tile_key(world_tile(3322, 3343)),
            tile_key(world_tile(3329, 3340)),
            tile_key(world_tile(3335, 3330)),
        ])
    }

    #[test]
    fn auto_approach_search_covers_the_scene_instead_of_the_default_400_expansions() {
        let origin = AUTO_START;
        let tree = world_tile(3294, 3331);
        let probe = CollisionArrivalProbe::new(|_| Some(0));
        assert!(!probe.can_reach_from(
            origin,
            tree,
            &SceneReachOptions {
                adjacent_ok: true,
                ..Default::default()
            },
        ));
        assert!(probe.can_reach_from(origin, tree, &auto_approach_options(104, 104)));
        assert!(!probe.can_reach_from(origin, tree, &auto_approach_options(0, 104)));
        assert!(!probe.can_reach_from(origin, tree, &auto_approach_options(104, -1)));
    }

    #[test]
    fn auto_fixture_uses_observed_tree_placements_without_location_seeds() {
        let (target, plan, task) = fixture_plan(Cell::Woodcutting, LiveCase::LocationAuto).unwrap();
        assert_eq!(target, AUTO_START);
        assert!(plan.seed_locs.is_empty());
        assert!(plan.oak_tiles.is_empty());
        assert!(plan.auto_next.is_empty());
        assert!(task.is_none());

        let build = auto_test_build();
        let real_scene_trees = auto_real_scene_trees();
        let nearest_tree = world_tile(3294, 3331);
        assert_eq!(tile_distance(tile_key(nearest_tree), AUTO_START), Some(33));
        let start = auto_start_candidate(
            AUTO_START,
            build,
            &real_scene_trees,
            |_| true,
            |_| true,
            |candidate, tree| candidate == AUTO_START && tree == nearest_tree,
        )
        .expect("the authored nearest tree should have an empty reachable Auto ring");
        assert_eq!(start, AUTO_START);
        assert!(real_scene_trees
            .iter()
            .all(|&tree| { tile_distance(tree, start).is_none_or(|distance| distance > 32) }));
        assert!(!auto_next_group(start, &real_scene_trees).is_empty());
        assert!(auto_start_candidate(
            AUTO_START,
            build,
            &real_scene_trees,
            |_| false,
            |_| true,
            |_, _| true,
        )
        .is_none());
        assert!(auto_start_candidate(
            AUTO_START,
            build,
            &real_scene_trees,
            |_| true,
            |_| true,
            |_, _| false,
        )
        .is_none());
        assert!(auto_start_candidate(
            AUTO_START,
            build,
            &real_scene_trees,
            |_| true,
            |_| false,
            |_, _| true,
        )
        .is_none());
        let land_alternate = auto_start_candidate(
            AUTO_START,
            build,
            &real_scene_trees,
            |_| true,
            |candidate| candidate != AUTO_START,
            |_, _| true,
        )
        .expect("a blocked origin must not qualify merely because its flood includes it");
        assert_ne!(land_alternate, AUTO_START);
        let alternate = auto_start_candidate(
            AUTO_START,
            build,
            &real_scene_trees,
            |tile| tile != start,
            |_| true,
            |_, _| true,
        )
        .expect("an unreachable closest tile must not hide another empty ring");
        assert_ne!(alternate, start);
    }

    #[test]
    fn auto_fixture_candidate_is_reachable_standable_and_beside_authored_trees_in_nav_pack() {
        let Some(pack_path) = std::env::var_os("NAV_PACK") else {
            return;
        };
        let world = nav_planner::world::NavWorld::load_pack(&PathBuf::from(pack_path))
            .expect("the selected real navigation pack must load");
        let collision = &world.collision;
        let build = auto_test_build();
        let scene = api::snapshot::SceneView {
            available: true,
            base_x: build.base_x,
            base_z: build.base_z,
            level: build.level,
            width: build.width,
            height: build.height,
            collision_flags: (0..build.width)
                .flat_map(|x| {
                    (0..build.height).map(move |z| {
                        collision.walkable_word(build.base_x + x, build.base_z + z, build.level)
                            as i32
                    })
                })
                .collect(),
        };
        let origin_query = api::query::SceneQuery::new(&scene, Some(AUTO_START));
        let reachable = origin_query
            .flood_reach()
            .expect("the real scene must admit its normal bounded reach flood");
        let trees = auto_real_scene_trees();
        let options = auto_approach_options(build.width, build.height);
        let start = auto_start_candidate(
            AUTO_START,
            build,
            &trees,
            |tile| reachable.at(&tile).0,
            |tile| origin_query.walkable(tile),
            |from, tree| api::query::SceneQuery::new(&scene, Some(from)).can_reach(tree, &options),
        )
        .expect("the real nav pack must admit a reachable stand beside an authored tree");
        assert!(collision.standable(start));
        assert!(!world.graph.wilderness.contains(start));
        assert!(reachable.at(&start).0);
        assert!(trees
            .iter()
            .all(|&tree| { tile_distance(tree, start).is_none_or(|distance| distance > 32) }));
        assert!(trees.iter().any(|&tree| {
            tile_distance(tree, start).is_some_and(|distance| (33..=128).contains(&distance))
                && api::query::SceneQuery::new(&scene, Some(start)).can_reach(
                    WorldTile {
                        x: tree.0,
                        z: tree.1,
                        level: tree.2,
                    },
                    &options,
                )
        }));
    }

    #[test]
    fn refused_return_fixture_uses_the_authored_wilderness_oak() {
        let (target, plan, task) =
            fixture_plan(Cell::Woodcutting, LiveCase::DeathReturnRefused).unwrap();
        assert_eq!(target, DEATH_RETURN_REFUSED_TARGET);
        assert_eq!(tile_distance((3227, 3533, 0), target), Some(23));
        let settings = Cell::Woodcutting.settings(LiveCase::DeathReturnRefused);
        assert_eq!(settings.get("allowWilderness"), Some(&json!(false)));
        assert!(plan.seed_locs.is_empty());
        assert!(plan.oak_tiles.is_empty());
        assert!(plan
            .inventory_seed
            .iter()
            .any(|(alias, count)| alias == "bronze_axe" && *count == 1));
        assert!(task.is_none());
    }

    #[test]
    fn near_edge_fixture_starts_before_the_outside_oak_loads() {
        let (target, plan, task) = fixture_plan(Cell::Woodcutting, LiveCase::OakNearEdge).unwrap();
        let mut slot = GatherSlot::new(
            Cell::Woodcutting,
            LiveCase::OakNearEdge,
            target,
            plan,
            task,
            false,
        );
        let mut observation = slot.capture();
        observation.tile = Some(tile_key(OAK_EDGE_START));
        observation.build = Some(BuildRect {
            base_x: (OAK_EDGE_PREFLIGHT.x.div_euclid(8) - 6) * 8,
            base_z: (OAK_EDGE_PREFLIGHT.z.div_euclid(8) - 6) * 8,
            width: 104,
            height: 104,
            level: OAK_EDGE_PREFLIGHT.level,
        });
        slot.latest = Some(observation.clone());
        assert!(slot.fixture_ready());
        slot.latest
            .as_mut()
            .unwrap()
            .live_oaks
            .insert(tile_key(OAK_EDGE_TARGET));
        assert!(
            !slot.fixture_ready(),
            "an already loaded oak cannot prove scene-edge approach"
        );
        observation.tile = Some(tile_key(OAK_EDGE_PREFLIGHT));
        slot.latest = Some(observation);
        assert!(
            !slot.fixture_ready(),
            "the initial actor must be within twenty tiles of the edge"
        );
    }

    #[test]
    fn macro_event_cases_use_real_targets_and_fail_closed() {
        let (gas_target, gas_plan, gas_task) =
            fixture_plan(Cell::Mining, LiveCase::GasHazard).unwrap();
        assert_eq!(gas_target, GAS_START);
        assert_eq!(
            Cell::Mining.resources(LiveCase::GasHazard),
            vec!["iron".to_owned()]
        );
        assert!(gas_plan.seed_locs.is_empty());
        assert!(gas_plan.oak_tiles.is_empty());
        assert!(gas_plan.inventory_seed.is_empty());
        assert!(gas_plan.bank_seed.is_empty());
        assert!(gas_task.is_none());
        assert!(macro_event_fixture_blocker(LiveCase::GasHazard).is_some());

        let (random_target, random_plan, random_task) =
            fixture_plan(Cell::Woodcutting, LiveCase::RandomEvent).unwrap();
        assert_eq!(random_target, DRAYNOR_OAK_TARGET);
        assert!(random_plan.seed_locs.is_empty());
        assert!(random_plan.oak_tiles.is_empty());
        assert!(random_plan.inventory_seed.is_empty());
        assert!(random_plan.bank_seed.is_empty());
        assert!(random_task.is_none());
        assert!(macro_event_fixture_blocker(LiveCase::RandomEvent).is_some());
    }

    #[test]
    fn no_placement_oak_fixture_checks_real_catherby_booth_and_no_nearby_oaks() {
        let (target, plan, task) =
            fixture_plan(Cell::Woodcutting, LiveCase::OakNoPlacementArea).unwrap();
        assert_eq!(target, OAK_NO_PLACEMENT_START);
        assert_eq!(
            tile_distance(tile_key(OAK_NO_PLACEMENT_BANK_BOOTH), target),
            Some(1)
        );
        assert!(plan.seed_locs.is_empty());
        assert!(plan.oak_tiles.is_empty());
        assert!(plan.inventory_seed.is_empty());
        assert!(plan.bank_seed.is_empty());
        assert!(task.is_none());
    }

    #[test]
    fn no_placement_verdict_requires_area_invalid_without_gameplay() {
        let fixture = || {
            let (target, plan, task) =
                fixture_plan(Cell::Woodcutting, LiveCase::OakNoPlacementArea).unwrap();
            let mut slot = GatherSlot::new(
                Cell::Woodcutting,
                LiveCase::OakNoPlacementArea,
                target,
                plan,
                task,
                false,
            );
            let observation = slot.capture();
            slot.baseline = Some(observation.clone());
            slot.latest = Some(observation);
            slot.witness.native_phases.insert("Blocked".into());
            slot.witness.failure_code = Some("area-invalid".into());
            slot
        };
        fixture().qualifies().unwrap();
        let invalidators: [fn(&mut GatherSlot); 8] = [
            |slot| slot.witness.failure_code = Some("resource-unavailable".into()),
            |slot| slot.witness.last_status_yielded = 1,
            |slot| slot.witness.last_status_dropped = 1,
            |slot| slot.witness.script_moved_after_start = true,
            |slot| {
                slot.witness.native_phases.insert("Waiting".into());
            },
            |slot| slot.witness.wait_until = Some(1),
            |slot| slot.latest.as_mut().unwrap().xp += 1,
            |slot| slot.latest.as_mut().unwrap().product_count += 1,
        ];
        for invalidate in invalidators {
            let mut slot = fixture();
            invalidate(&mut slot);
            assert!(slot.qualifies().is_err());
        }
    }
}

#[cfg(test)]
mod named_site_fixture_tests {
    use super::*;

    #[test]
    fn named_sites_use_real_content_and_fishing_defaults_to_guild() {
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
            let (start, plan, task) = fixture_plan(cell, LiveCase::Site).unwrap();
            assert_eq!(
                start,
                match cell {
                    Cell::Fishing => FISHING_GUILD_START,
                    Cell::Woodcutting => world_tile(3120, 3267),
                    Cell::Mining => world_tile(3253, 3420),
                }
            );
            assert!(plan.seed_locs.is_empty());
            assert!(plan.oak_tiles.is_empty());
            assert!(plan.inventory_seed.is_empty());
            assert!(plan.bank_seed.is_empty());
            assert!(plan.auto_next.is_empty());
            assert!(task.is_none());
        }
    }

    #[test]
    fn draynor_site_fixture_uses_real_site_and_default_start_without_override() {
        let case = LiveCase::SiteDraynor;
        let settings = Cell::Fishing.settings(case);
        assert_eq!(
            settings.get("site"),
            Some(&json!("fishing.draynor_village"))
        );
        assert_eq!(settings.get("disposition"), Some(&json!("Bank")));
        assert_eq!(settings.get("bank"), Some(&json!("Nearest")));
        assert_eq!(settings.get("radius"), Some(&json!(12)));
        assert_eq!(Cell::Fishing.tool_alias(case), "net");
        assert_eq!(Cell::Fishing.tool_id(case), 303);
        assert_eq!(Cell::Fishing.level(case), 15);
        assert_eq!(Cell::Fishing.products(case), &[SHRIMP_ID, ANCHOVY_ID]);
        assert_eq!(
            settings.get("fishingMethod"),
            Some(&json!("fishing.saltfish.op1"))
        );
        for coordinate in ["x", "z", "level"] {
            assert!(!settings.contains_key(coordinate));
        }
        let (start, plan, task) = fixture_plan(Cell::Fishing, case).unwrap();
        assert_eq!(start, DRAYNOR_FISH_SITE_START);
        assert!(plan.seed_locs.is_empty());
        assert!(plan.inventory_seed.is_empty());
        assert!(plan.bank_seed.is_empty());
        assert!(task.is_none());
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
                site_return_area: true,
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
            assert!(site_bank_complete(cell, LiveCase::Site, &witness, &plan, 0));
            witness.site_anchor_selected = false;
            assert!(!site_bank_complete(
                cell,
                LiveCase::Site,
                &witness,
                &plan,
                0
            ));
            witness.site_anchor_selected = true;
            witness.site_initial_walk = false;
            assert!(!site_bank_complete(
                cell,
                LiveCase::Site,
                &witness,
                &plan,
                0
            ));
            witness.site_initial_walk = true;
            witness.site_return_area = false;
            assert!(!site_bank_complete(
                cell,
                LiveCase::Site,
                &witness,
                &plan,
                0
            ));
            witness.site_return_area = true;
            witness.post_bank_yields = 0;
            assert!(!site_bank_complete(
                cell,
                LiveCase::Site,
                &witness,
                &plan,
                0
            ));
            witness.post_bank_yields = 1;
            witness.bank_trips[0].deposit_verified = false;
            assert!(!site_bank_complete(
                cell,
                LiveCase::Site,
                &witness,
                &plan,
                0
            ));
            witness.bank_trips[0].deposit_verified = true;
            witness.last_status_bank = Some("Different bank; Reachable; Return".into());
            assert!(!site_bank_complete(
                cell,
                LiveCase::Site,
                &witness,
                &plan,
                0
            ));
            witness.last_status_bank = Some(format!("{}; Reachable; Return", cell.site_bank()));
            plan.oak_tiles.push(world_tile(1, 1));
            assert!(!site_bank_complete(
                cell,
                LiveCase::Site,
                &witness,
                &plan,
                0
            ));
        }
    }

    #[test]
    fn draynor_bank_return_completion_does_not_require_an_initial_survey_walk() {
        let cell = Cell::Fishing;
        let case = LiveCase::SiteDraynor;
        let product = cell.products(case)[0];
        let mut trip = BankTripReceipt::new(
            1,
            BTreeMap::from([(cell.default_tool_id(case), 1), (product, 2)]),
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
            site_initial_walk: false,
            site_return_area: true,
            bank_return_step_seen: true,
            bank_arrivals: 1,
            bank_trips: vec![trip],
            bank_nonzero_roundtrips: 1,
            post_bank_yields: 1,
            last_xp: 1,
            products_seen: BTreeSet::from([product]),
            last_status_bank: Some(format!(
                "{}; Reachable; Return",
                cell.site_bank_for_case(case)
            )),
            ..Witness::default()
        };
        let plan = FixturePlan::default();
        assert!(site_bank_complete(cell, case, &witness, &plan, 0));

        witness.site_return_area = false;
        assert!(!site_bank_complete(cell, case, &witness, &plan, 0));
        witness.site_return_area = true;
        witness.bank_trips[0].deposit_verified = false;
        assert!(!site_bank_complete(cell, case, &witness, &plan, 0));
        witness.bank_trips[0].deposit_verified = true;
        witness.post_bank_yields = 0;
        assert!(!site_bank_complete(cell, case, &witness, &plan, 0));
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
        assert_eq!(parse_work_anchor("site (3083,3237,0) r11"), None);
    }

    #[test]
    fn site_walk_goal_is_inside_the_selected_work_area_not_necessarily_its_anchor() {
        let anchor = world_tile(3083, 3237);
        let mut nav = json!({"requested_destination": [3084, 3237, 0]});
        assert!(site_nav_targets_area(&nav, anchor));
        for goal in [
            json!([3096, 3237, 0]),
            json!([3084, 3237, 1]),
            json!([3084, 3237]),
            json!([3084.5, 3237, 0]),
        ] {
            nav["requested_destination"] = goal;
            assert!(!site_nav_targets_area(&nav, anchor));
        }
    }

    fn initial_site_nav() -> Value {
        json!({
            "armed": true,
            "requested": true,
            "request_id": 2,
            "arrival": "Area",
            "requested_radius": 0,
            "requested_destination": [2846, 3437, 0],
        })
    }

    #[test]
    fn initial_site_walk_accepts_the_unloaded_resource_approach() {
        assert!(site_initial_nav_matches(
            Cell::Fishing,
            &initial_site_nav(),
            world_tile(2844, 3429),
            "surveying",
            "0_44_53_rarefish",
        ));
    }

    #[test]
    fn initial_site_walk_accepts_the_loaded_woodcutting_approach() {
        let mut nav = initial_site_nav();
        nav["requested_radius"] = json!(1);
        assert!(site_initial_nav_matches(
            Cell::Woodcutting,
            &nav,
            world_tile(2844, 3429),
            "walking",
            "Oak",
        ));
    }

    #[test]
    fn initial_fishing_approach_rejects_a_radius_one_arrival() {
        let mut nav = initial_site_nav();
        nav["requested_radius"] = json!(1);
        assert!(!site_initial_nav_matches(
            Cell::Fishing,
            &nav,
            world_tile(2844, 3429),
            "surveying",
            "0_44_53_rarefish",
        ));
    }

    #[test]
    fn initial_site_walk_rejects_non_approach_phases() {
        for phase in ["", "banking", "gathering", "blocked"] {
            assert!(!site_initial_nav_matches(
                Cell::Fishing,
                &initial_site_nav(),
                world_tile(2844, 3429),
                phase,
                "0_44_53_rarefish",
            ));
        }
    }

    #[test]
    fn initial_site_walk_rejects_a_missing_resource_target() {
        for phase in ["walking", "surveying"] {
            for target in ["", "—"] {
                assert!(!site_initial_nav_matches(
                    Cell::Fishing,
                    &initial_site_nav(),
                    world_tile(2844, 3429),
                    phase,
                    target,
                ));
            }
        }
    }

    #[test]
    fn initial_site_walk_rejects_an_unarmed_walk() {
        let mut nav = initial_site_nav();
        nav["armed"] = json!(false);
        for phase in ["walking", "surveying"] {
            assert!(!site_initial_nav_matches(
                Cell::Fishing,
                &nav,
                world_tile(2844, 3429),
                phase,
                "0_44_53_rarefish",
            ));
        }
    }

    #[test]
    fn initial_site_walk_rejects_goals_outside_the_selected_site() {
        let mut nav = initial_site_nav();
        for goal in [
            json!([2857, 3429, 0]),
            json!([2846, 3437, 1]),
            json!([2846, 3437]),
        ] {
            nav["requested_destination"] = goal;
            for phase in ["walking", "surveying"] {
                assert!(!site_initial_nav_matches(
                    Cell::Fishing,
                    &nav,
                    world_tile(2844, 3429),
                    phase,
                    "0_44_53_rarefish",
                ));
            }
        }
    }

    #[test]
    fn initial_observation_proof_requires_current_skill_specific_walk() {
        let anchor = world_tile(2844, 3429);
        for cell in [Cell::Fishing, Cell::Woodcutting, Cell::Mining] {
            let phase = if cell == Cell::Fishing {
                "surveying"
            } else {
                "walking"
            };
            let radius = if cell == Cell::Fishing { 0 } else { 1 };
            let nav = json!({
                "armed": true,
                "requested": true,
                "request_id": 2,
                "arrival": "Area",
                "requested_radius": radius,
                "requested_destination": [2846, 3417, 0],
            });
            assert!(site_initial_nav_matches(
                cell, &nav, anchor, phase, "resource"
            ));
            for bad_phase in ["gathering", "waiting", "banking"] {
                assert!(!site_initial_nav_matches(
                    cell, &nav, anchor, bad_phase, "resource"
                ));
            }
            for bad_target in ["", "—"] {
                assert!(!site_initial_nav_matches(
                    cell, &nav, anchor, phase, bad_target
                ));
            }
            for (field, value) in [
                ("armed", json!(false)),
                ("requested", json!(false)),
                ("request_id", json!(0)),
                ("arrival", json!("Reach")),
                ("requested_radius", json!(1 - radius)),
                ("requested_destination", json!([2846, 3417, 1])),
                ("requested_destination", json!([2860, 3417, 0])),
            ] {
                let mut bad = nav.clone();
                bad[field] = value;
                assert!(
                    !site_initial_nav_matches(cell, &bad, anchor, phase, "resource"),
                    "{field}"
                );
            }
        }
    }

    #[test]
    fn fishing_return_proof_requires_exact_area_observation_arrival() {
        let anchor = world_tile(2844, 3429);
        let target = "0_44_53_rarefish; observation stand:2846,3429,0";
        let mut nav = json!({
            "armed": true,
            "requested": true,
            "request_id": 23,
            "arrival": "Area",
            "requested_radius": 0,
            "requested_destination": [2846, 3429, 0],
        });
        assert!(site_return_nav_matches(Cell::Fishing, &nav, anchor, target));
        nav["requested_radius"] = json!(1);
        assert!(!site_return_nav_matches(
            Cell::Fishing,
            &nav,
            anchor,
            target
        ));
    }

    #[test]
    fn draynor_return_proof_requires_work_area_and_settled_player_goal() {
        let anchor = world_tile(3086, 3229);
        let stand = anchor;
        let request = SiteReturnRequest {
            request_id: 23,
            anchor,
            preferred_center: stand,
        };
        let mut nav = json!({
            "outcome_request_id": 23,
            "failed": false,
            "blocked": false,
            "reason": "None",
            "outcome_radius": 0,
            "destination": [stand.x, stand.z, stand.level],
        });
        assert!(draynor_site_return_nav_matches(&nav, request, Some(stand)));

        for rejected in [world_tile(3071, 3216), world_tile(3092, 3242)] {
            nav["destination"] = json!([rejected.x, rejected.z, rejected.level]);
            assert!(!draynor_site_return_nav_matches(
                &nav,
                request,
                Some(rejected)
            ));
        }
        nav["destination"] = json!([stand.x, stand.z, stand.level]);
        assert!(!draynor_site_return_nav_matches(
            &nav,
            request,
            Some(world_tile(3092, 3242)),
        ));
        nav["outcome_request_id"] = json!(22);
        assert!(!draynor_site_return_nav_matches(&nav, request, Some(stand)));
        nav["outcome_request_id"] = json!(23);
        nav["failed"] = json!(true);
        assert!(!draynor_site_return_nav_matches(&nav, request, Some(stand)));
    }

    #[test]
    fn draynor_return_proof_survives_cleared_return_status_and_actual_goal_selection() {
        let anchor = world_tile(3085, 3230);
        let selected = world_tile(3086, 3231);
        let mut witness = Witness::default();
        let requested = json!({
            "armed": true,
            "requested": true,
            "request_id": 23,
            "outcome_request_id": 0,
            "failed": false,
            "arrival": "Area",
            "requested_radius": 0,
            "requested_destination": [anchor.x, anchor.z, anchor.level],
            "destination": [0, 0, 0],
        });
        record_draynor_site_return(
            &mut witness,
            &requested,
            true,
            "saltfish; observation stand:3085,3230,0",
            Some(anchor),
            Some(world_tile(3092, 3243)),
        );
        assert!(!witness.site_return_area);
        let settled = json!({
            "armed": false,
            "requested": false,
            "request_id": 23,
            "outcome_request_id": 23,
            "failed": false,
            "blocked": false,
            "reason": "None",
            "outcome_radius": 0,
            "arrival": "Area",
            "requested_radius": null,
            "requested_destination": null,
            "destination": [selected.x, selected.z, selected.level],
        });
        record_draynor_site_return(
            &mut witness,
            &settled,
            false,
            "saltfish",
            Some(anchor),
            Some(selected),
        );
        assert!(
            witness.site_return_area,
            "the exact correlated settled return must survive status cleanup"
        );
    }

    #[test]
    fn return_proof_binds_current_area_request_to_its_resource_observation_stand() {
        let anchor = world_tile(3083, 3237);
        let target = "willowtree; observation stand:3084,3237,0";
        let mut nav = json!({
            "armed": true,
            "requested": true,
            "request_id": 1,
            "arrival": "Area",
            "requested_radius": 1,
            "requested_destination": [3084, 3237, 0],
            "outcome_radius": 1,
        });
        assert!(site_return_nav_matches(
            Cell::Woodcutting,
            &nav,
            anchor,
            target
        ));
        assert!(!site_return_nav_matches(
            Cell::Woodcutting,
            &nav,
            anchor,
            "willowtree"
        ));
        assert!(!site_return_nav_matches(
            Cell::Woodcutting,
            &nav,
            anchor,
            "willowtree; observation stand:3084,3237,1"
        ));
        assert!(!site_return_nav_matches(
            Cell::Woodcutting,
            &json!({
                "armed": true,
                "requested": true,
                "request_id": 1,
                "arrival": "Area",
                "requested_radius": 1,
                "requested_destination": [3096, 3237, 0],
            }),
            anchor,
            "willowtree; observation stand:3096,3237,0"
        ));
        nav["requested_destination"] = json!([anchor.x, anchor.z, anchor.level]);
        assert!(!site_return_nav_matches(
            Cell::Woodcutting,
            &nav,
            anchor,
            target
        ));
        nav["requested_destination"] = json!([3084, 3237, 0]);
        nav["requested_radius"] = json!(12);
        assert!(
            !site_return_nav_matches(Cell::Woodcutting, &nav, anchor, target),
            "a stale r1 outcome is not a current Return"
        );
        nav["requested_radius"] = json!(1);
        nav["arrival"] = json!("Reach");
        assert!(!site_return_nav_matches(
            Cell::Woodcutting,
            &nav,
            anchor,
            target
        ));
        nav["arrival"] = json!("Area");
        nav["requested"] = json!(false);
        assert!(!site_return_nav_matches(
            Cell::Woodcutting,
            &nav,
            anchor,
            target
        ));
        nav["requested"] = json!(true);
        nav["request_id"] = json!(0);
        assert!(!site_return_nav_matches(
            Cell::Woodcutting,
            &nav,
            anchor,
            target
        ));
        nav["request_id"] = json!(1);
        nav["armed"] = json!(false);
        assert!(!site_return_nav_matches(
            Cell::Woodcutting,
            &nav,
            anchor,
            target
        ));
    }
}

#[cfg(test)]
mod real_content_death_fixture_tests {
    use super::*;

    #[test]
    fn death_reprovision_fixture_seeds_account_only() {
        let (target, plan, task) =
            real_content_death_fixture(Cell::Woodcutting, LiveCase::DeathReturn).unwrap();

        assert_eq!(target, DRAYNOR_OAK_BANK_START);
        assert_eq!(plan.bank_seed, vec![("bronze_axe".into(), 3)]);
        assert_eq!(
            plan.inventory_seed,
            DEATH_FILLERS
                .iter()
                .map(|(_, alias)| ((*alias).into(), 1))
                .collect::<Vec<_>>()
        );
        assert!(plan.seed_locs.is_empty());
        assert!(plan.oak_tiles.is_empty());
        assert!(task.is_none());
    }

    #[test]
    fn death_no_stock_fixture_seeds_only_account_inventory() {
        let case = LiveCase::DeathNoStock;
        let settings = Cell::Woodcutting.settings(case);
        assert_eq!(settings.get("buyTools"), Some(&json!(false)));

        let (target, plan, task) = fixture_plan(Cell::Woodcutting, case).unwrap();
        assert_eq!(target, DRAYNOR_OAK_TARGET);
        assert_eq!(
            plan.inventory_seed,
            std::iter::once(("bronze_axe".to_owned(), 1))
                .chain(
                    DEATH_FILLERS
                        .iter()
                        .map(|(_, alias)| ((*alias).to_owned(), 1))
                )
                .collect::<Vec<_>>()
        );
        assert!(plan.bank_seed.is_empty());
        assert!(plan.seed_locs.is_empty());
        assert!(plan.oak_tiles.is_empty());
        assert!(task.is_none());
    }

    #[test]
    fn death_no_stock_accepts_blocked_running_without_failure() {
        let witness = Witness {
            native_phases: BTreeSet::from(["Blocked".to_owned()]),
            last_status_block_reason: Some("required tool unavailable".into()),
            death_no_stock_blocked_running: true,
            last_deaths: 1,
            death_recovery_command_sent: 1,
            last_status_bank: Some("Draynor; Reachable; access:3092,3243,0; Supply".into()),
            ..Witness::default()
        };
        assert!(interrupts_complete(LiveCase::DeathNoStock, &witness));

        let mut failed = witness.clone();
        failed.failure_code = Some("supply-missing".into());
        assert!(!interrupts_complete(LiveCase::DeathNoStock, &failed));

        let mut stopped = witness;
        stopped.death_no_stock_blocked_running = false;
        assert!(!interrupts_complete(LiveCase::DeathNoStock, &stopped));
    }
}

#[cfg(test)]
mod death_oracle_tests {
    use super::*;

    fn no_stock_witness(bank: &str, trips: i64) -> Witness {
        Witness {
            native_phases: BTreeSet::from(["Blocked".to_owned()]),
            death_no_stock_blocked_running: true,
            last_status_block_reason: Some("missing Bronze axe".into()),
            last_deaths: 1,
            death_recovery_command_sent: 1,
            last_status_bank: Some(bank.into()),
            status_trips: trips,
            ..Witness::default()
        }
    }

    fn death_region_witness(death_tile: (i32, i32, i32)) -> Witness {
        Witness {
            death_recovery_in_area: true,
            death_command_tile: Some(death_tile),
            death_recovered: BTreeSet::from([1]),
            death_products_after: BTreeSet::from([1]),
            death_xp_after: BTreeSet::from([1]),
            last_deaths: 1,
            last_recoveries: 1,
            ..Witness::default()
        }
    }

    #[test]
    fn death_no_stock_accepts_supply_bank_without_formatter_suffix() {
        let witness = no_stock_witness("Draynor; Reachable; access:3092,3243,0; Supply", 0);
        assert!(interrupts_complete(LiveCase::DeathNoStock, &witness));
    }

    #[test]
    fn death_no_stock_rejects_a_repeated_supply_trip() {
        let witness = no_stock_witness("Draynor; Reachable; access:3092,3243,0; Withdraw", 1);
        assert!(!interrupts_complete(LiveCase::DeathNoStock, &witness));
    }

    #[test]
    fn death_no_stock_rejects_missing_bank_access_or_an_error() {
        let mut witness = no_stock_witness("Draynor; Supply", 0);
        assert!(!interrupts_complete(LiveCase::DeathNoStock, &witness));
        witness.last_status_bank = Some("Draynor; Reachable; access:3092,3243,0; Supply".into());
        witness.failure_code = Some("bank-unavailable".into());
        assert!(!interrupts_complete(LiveCase::DeathNoStock, &witness));
    }

    #[test]
    fn death_respawn_region_accepts_a_delayed_same_region_sample() {
        let death_tile = (3219, 3208, 0);
        let respawn_tile = (3224, 3221, 0);
        let mut witness = death_region_witness(death_tile);

        observe_death_respawn_region(
            &mut witness,
            0,
            Some("death observed; inventory unknown"),
            Some(death_tile),
        );
        assert_eq!(witness.death_respawn_tile, None);
        assert!(!interrupts_complete(LiveCase::DeathRespawnRegion, &witness));

        observe_death_respawn_region(
            &mut witness,
            1,
            Some("respawn observed; settling"),
            Some(respawn_tile),
        );
        assert_eq!(witness.death_respawn_tile, Some(respawn_tile));
        assert!(witness.death_respawn_region_unchanged);
        assert!(interrupts_complete(LiveCase::DeathRespawnRegion, &witness));
    }

    #[test]
    fn death_respawn_region_rejects_a_different_region_sample() {
        let mut witness = death_region_witness((3264, 3208, 0));
        observe_death_respawn_region(
            &mut witness,
            1,
            Some("respawn observed; settling"),
            Some((3224, 3221, 0)),
        );
        assert_eq!(witness.death_respawn_tile, Some((3224, 3221, 0)));
        assert!(!witness.death_respawn_region_unchanged);
        assert!(!interrupts_complete(LiveCase::DeathRespawnRegion, &witness));
    }

    #[test]
    fn death_respawn_region_rejects_a_non_respawn_tile() {
        let mut witness = death_region_witness((3219, 3208, 0));
        observe_death_respawn_region(
            &mut witness,
            1,
            Some("respawn observed; settling"),
            Some((3225, 3218, 0)),
        );
        assert_eq!(witness.death_respawn_tile, Some((3225, 3218, 0)));
        assert!(!witness.death_respawn_region_unchanged);
        assert!(!interrupts_complete(LiveCase::DeathRespawnRegion, &witness));
    }
}

#[cfg(all(test, feature = "live-harness"))]
mod quester_shared_core_completion_tests {
    use super::*;
    use script::{RunState, ScriptLifecycleReceipt, ScriptTerminalState};

    fn receipt(state: ScriptTerminalState) -> ScriptLifecycleReceipt {
        ScriptLifecycleReceipt {
            runtime_generation: 1,
            state,
            tick: 100,
            reason: "terminal fixture".into(),
        }
    }

    #[test]
    fn durable_completion_survives_native_status_teardown() {
        assert!(quester_shared_core_completed(
            None,
            Some(&receipt(ScriptTerminalState::Completed)),
            RunState::Idle,
        )
        .unwrap());
    }

    #[test]
    fn absent_completion_and_active_runs_do_not_pass() {
        assert!(!quester_shared_core_completed(None, None, RunState::Idle).unwrap());
        assert!(!quester_shared_core_completed(
            None,
            Some(&receipt(ScriptTerminalState::Completed)),
            RunState::Running,
        )
        .unwrap());
    }

    #[test]
    fn other_terminal_states_do_not_pass() {
        for state in [
            ScriptTerminalState::Failed,
            ScriptTerminalState::Stopped,
            ScriptTerminalState::Cancelled,
        ] {
            assert!(
                quester_shared_core_completed(None, Some(&receipt(state)), RunState::Idle).is_err()
            );
        }
    }

    #[test]
    fn another_cards_status_does_not_pass() {
        let status = script::native::ScriptStatus {
            survival: Default::default(),
            run: api::selected::RunKey {
                slot: 1,
                run: 1,
                session: 1,
            },
            card: script::CompiledId("Gatherer"),
            phase: script::native::NativePhase::Complete,
            active_settings: 1,
            pending_settings: None,
            failure: None,
            fields: Arc::from([]),
        };
        assert!(quester_shared_core_completed(
            Some(&status),
            Some(&receipt(ScriptTerminalState::Completed)),
            RunState::Idle,
        )
        .is_err());
    }
}

#[cfg(test)]
mod death_return_stop_oracle_tests {
    use super::*;

    fn quiet_nav(request_id: u64) -> Value {
        json!({
            "request_id": request_id,
            "armed": false,
            "route": false,
            "route_work_pending": false,
            "pending": false,
            "requested": false,
            "bank_fetch": false,
            "carry": false,
        })
    }

    fn stopped_return(detail: &str) -> Witness {
        let mut stop = DeathReturnStopWitness::default();
        stop.observe_permissions(Some(false), false);
        stop.observe_tile(50, Some((3222, 3218, 0)), true, Some(false));
        stop.observe_tile(99, Some((3092, 3243, 0)), true, Some(false));
        stop.observe_terminal(100, Some((3092, 3243, 0)), 1, "return-failed", detail);
        stop.observe_navigation(100, &quiet_nav(42), true);
        stop.observe_tile(103, Some((3092, 3243, 0)), true, Some(false));
        stop.observe_navigation(103, &quiet_nav(42), true);
        Witness {
            death_return_stop: stop,
            last_deaths: 1,
            death_recovery_command_sent: 1,
            ..Witness::default()
        }
    }

    #[test]
    fn no_path_after_death_passes_and_keeps_the_walk_detail() {
        let witness = stopped_return("walk ended with NoPath");
        assert!(interrupts_complete(LiveCase::DeathReturnRefused, &witness));
        assert_eq!(
            witness.death_return_stop.terminal_detail.as_deref(),
            Some("walk ended with NoPath")
        );
        assert!(interrupts_complete(
            LiveCase::DeathReturnRefused,
            &stopped_return("walk ended with Refused")
        ));
    }

    #[test]
    fn request_retirement_after_terminal_is_cleanup_not_a_walk() {
        let mut witness = stopped_return("walk ended with NoPath");
        witness
            .death_return_stop
            .observe_navigation(104, &quiet_nav(0), true);
        witness
            .death_return_stop
            .observe_tile(104, Some((3092, 3243, 0)), true, Some(false));
        assert!(interrupts_complete(LiveCase::DeathReturnRefused, &witness));
        witness
            .death_return_stop
            .observe_navigation(105, &quiet_nav(42), true);
        assert!(!interrupts_complete(LiveCase::DeathReturnRefused, &witness));

        let mut active_retired = stopped_return("walk ended with NoPath");
        let mut nav = quiet_nav(0);
        nav["armed"] = json!(true);
        active_retired
            .death_return_stop
            .observe_navigation(104, &nav, true);
        assert!(!interrupts_complete(
            LiveCase::DeathReturnRefused,
            &active_retired
        ));
    }

    #[test]
    fn entering_wilderness_after_respawn_fails() {
        let mut witness = stopped_return("walk ended with NoPath");
        witness
            .death_return_stop
            .observe_tile(60, Some((3204, 3525, 0)), true, Some(true));
        assert!(!interrupts_complete(LiveCase::DeathReturnRefused, &witness));
    }

    #[test]
    fn continuing_or_rearming_walk_after_terminal_failure_fails() {
        for key in [
            "armed",
            "route",
            "route_work_pending",
            "pending",
            "requested",
            "bank_fetch",
            "carry",
        ] {
            let mut witness = stopped_return("walk ended with NoPath");
            let mut nav = quiet_nav(42);
            nav[key] = json!(true);
            witness
                .death_return_stop
                .observe_navigation(104, &nav, true);
            assert!(
                !interrupts_complete(LiveCase::DeathReturnRefused, &witness),
                "active {key} must fail"
            );
        }
        let mut rearmed = stopped_return("walk ended with NoPath");
        rearmed
            .death_return_stop
            .observe_navigation(104, &quiet_nav(43), true);
        assert!(!interrupts_complete(LiveCase::DeathReturnRefused, &rearmed));
        let mut moving = stopped_return("walk ended with NoPath");
        moving
            .death_return_stop
            .observe_tile(104, Some((3093, 3243, 0)), true, Some(false));
        assert!(!interrupts_complete(LiveCase::DeathReturnRefused, &moving));
    }

    #[test]
    fn missing_or_contradictory_safety_evidence_fails() {
        let mut granted = stopped_return("walk ended with NoPath");
        granted
            .death_return_stop
            .observe_permissions(Some(false), true);
        granted
            .death_return_stop
            .observe_permissions(Some(false), false);
        assert!(!interrupts_complete(LiveCase::DeathReturnRefused, &granted));

        let mut no_permission = stopped_return("walk ended with NoPath");
        no_permission
            .death_return_stop
            .observe_permissions(None, false);
        assert!(!interrupts_complete(
            LiveCase::DeathReturnRefused,
            &no_permission
        ));

        let mut no_tile = stopped_return("walk ended with NoPath");
        no_tile
            .death_return_stop
            .observe_tile(104, None, true, None);
        assert!(!interrupts_complete(LiveCase::DeathReturnRefused, &no_tile));

        let mut no_nav = stopped_return("walk ended with NoPath");
        no_nav
            .death_return_stop
            .observe_navigation(104, &Value::Null, true);
        assert!(!interrupts_complete(LiveCase::DeathReturnRefused, &no_nav));

        let mut still_running = stopped_return("walk ended with NoPath");
        still_running
            .death_return_stop
            .observe_navigation(104, &quiet_nav(42), false);
        assert!(!interrupts_complete(
            LiveCase::DeathReturnRefused,
            &still_running
        ));
    }

    #[test]
    fn terminal_requires_the_death_and_three_fresh_quiet_ticks() {
        let mut witness = stopped_return("walk ended with NoPath");
        witness.death_return_stop.quiet_through_tick = Some(102);
        assert!(!interrupts_complete(LiveCase::DeathReturnRefused, &witness));
        witness.death_return_stop.quiet_through_tick = Some(103);
        witness.last_deaths = 0;
        assert!(!interrupts_complete(LiveCase::DeathReturnRefused, &witness));

        let mut premature = DeathReturnStopWitness::default();
        premature.observe_tile(50, Some((3222, 3218, 0)), true, Some(false));
        premature.observe_terminal(
            100,
            Some((3092, 3243, 0)),
            0,
            "return-failed",
            "walk ended with NoPath",
        );
        assert!(premature.terminal_detail.is_none());
        premature.observe_terminal(
            100,
            Some((3092, 3243, 0)),
            1,
            "supply-missing",
            "walk ended with NoPath",
        );
        assert!(premature.terminal_detail.is_none());
    }
}
