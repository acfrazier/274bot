use std::collections::{BTreeMap, HashSet};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use api::interact;
use api::snapshot::{GameSnapshot, WorldTile};
use client::client::Client;
use client::io::ClientProt289;
use host_play::tick_engine_test_support::{
    DispatchMutation, DispatchObservation, FrameObservation, PollKind, TickEngineTestHook,
    TickEngineTrace, TransportMutation, WireObservation,
};
use host_play::{
    ProfileClass, ProfileOptions, SharedClientTemplate, WorldMembersFact, WorldMembersSource,
};
use script::native::{ScriptStatus, StatusValue};
use serde_json::{json, Map, Value};
use vault::{Profile, ProfileSettings};

#[path = "support/loopback_proxy.rs"]
mod loopback_proxy;
use loopback_proxy::{LoopbackProxy, ProxyObservation};
#[path = "support/engine_packet_log.rs"]
mod engine_packet_log;
use engine_packet_log::{EnginePacket, EnginePacketLog};
#[path = "support/guard_opener.rs"]
mod guard_opener;
#[path = "support/tick_startup.rs"]
mod tick_startup;
use guard_opener::GuardOpenerState;
#[path = "support/drop_history.rs"]
mod drop_history;

const ENGINE_HOST: &str = "127.0.0.1";
const ENGINE_GAME_PORT: u16 = 48_594;
const ENGINE_HTTP_PORT: u16 = 4_080;
const LOG_ID: i32 = 1_511;
const KNIFE_ID: i32 = 946;
const LOGS_ID: i32 = 1_511;
const FLY_ROD_ID: i32 = 309;
const NET_ID: i32 = 303;
const FEATHER_ID: i32 = 314;
const BRONZE_PICKAXE_ID: i32 = 1_265;
const MITHRIL_PICKAXE_ID: i32 = 1_273;
const BRONZE_AXE_ID: i32 = 1_351;
const COPPER_ORE_ID: i32 = 436;
const IRON_ORE_ID: i32 = 440;
const SHRIMP_ID: i32 = 317;
const ANCHOVY_ID: i32 = 321;
const TROUT_ID: i32 = 335;
const SALMON_ID: i32 = 331;
const FRESHFISH_NPC_ID: i32 = 309;
const FRESHFISH_NPC_TILE: (i32, i32, i32) = (2_382, 3_414, 0);
// This KnifeClick uses movement mode 2 for MOVE_OPCLICK, then sends OPNPC1.
const NPC_KNIFE_WIRE_OPCODES: [u8; 4] = [
    ClientProt289::OPHELDU.id as u8,
    ClientProt289::IF_BUTTON.id as u8,
    ClientProt289::MOVE_OPCLICK.id as u8,
    ClientProt289::OPNPC1.id as u8,
];
const OAK_LOG_ID: i32 = 1_521;
const OAK_LOC_ID: i32 = 1_281;
const RIVER_LOC_ID: i32 = 2_027;
const FLETCHING_ANIMATION: i32 = 1_248;
const CAST_ANIMATION: i32 = 622;
const ROLL_ANIMATION: i32 = 623;
const BRONZE_PICK_ANIMATION: i32 = 625;
const MITHRIL_PICK_ANIMATION: i32 = 629;

// Start submits the first native action on the next PLAYER_INFO, allowing one
// additional edge for the worker's Start handoff. Case budgets below begin at
// a real dispatcher receipt, never a guessed Start or setup tick.
const STARTUP_EDGE_BUDGET: u32 = 2;
const CLEAN_RIVER_BUDGET: u32 = 3;
const STALE_MENU_BUDGET: u32 = 4;
const SPLIT_OPENER_BUDGET: u32 = 4;
const LATE_RIVER_BUDGET: u32 = 5;
const LATE_IRON_BUDGET: u32 = 5;
const NATURAL_IRON_ROLL_BUDGET: u32 = 600;
const IRON_ROLL_OPPORTUNITY_LIMIT: usize = 32;
const COPPER_BUDGET: u32 = 4;
const HOST_REFUSAL_BUDGET: u32 = 1;
const MAKE_WITHOUT_CLICK_BUDGET: u32 = 3;
// After the first-budget click, freshfish's expired-delay branch rearms, then
// casts at d=3 and writes its act line at d=2 (freshfish.rs2:37-50). The last
// before-PLAYER_INFO line can first appear in the preceding wake.
const LONE_OPENER_BUDGET: u32 = 4;
const FREE_WORLD_BUDGET: u32 = 2;
const NET_REQUIRED_CYCLES: i64 = 20;
// Ending-family net cycles have no roll signal and count as unobserved; six
// edges per cycle bound the arm, checked click and next-cycle observation.
const NET_EDGES_PER_CYCLE: u32 = 6;
const NET_TICK_BUDGET: u32 = NET_REQUIRED_CYCLES as u32 * NET_EDGES_PER_CYCLE;
// freshfish.rs2's first roll is four ticks after the NPC click. After each
// roll, the expired-delay branch rearms for +4 on the next tick, so later
// rolls are five ticks apart. Twenty Fishing-20 trout rolls fit in 99 ticks;
// the 100-tick bound includes one observation edge. At level 20 the authored
// trout success range is 32..192, and STAT_RANDOM accepts 63 of 256 values,
// giving about 99.6% chance of at least one catch in twenty independent rolls.
const FRESHFISH_MAX_ROLLS: u32 = 20;
const FRESHFISH_ROLL_INTERVAL_TICKS: u32 = 5;
const FRESHFISH_TICK_BUDGET: u32 = FRESHFISH_MAX_ROLLS * FRESHFISH_ROLL_INTERVAL_TICKS;

const FRAME_HANG_GUARD: Duration = Duration::from_secs(5);
const SETUP_DEADLINE: Duration = Duration::from_secs(180);
const LOGOUT_DEADLINE: Duration = Duration::from_secs(30);
static LIVE_RUN_LOCK: Mutex<()> = Mutex::new(());

#[derive(Debug, Clone)]
struct SeedItem {
    alias: &'static str,
    id: i32,
    count: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ResourceKind {
    River,
    NpcRiver,
    Net,
    Copper,
    Iron,
    Oak,
}

#[derive(Debug, Clone)]
struct Scenario {
    name: &'static str,
    skill: &'static str,
    skill_index: i32,
    level: i32,
    resource: ResourceKind,
    target: WorldTile,
    tool: SeedItem,
    extra_items: Vec<SeedItem>,
    products: Vec<i32>,
    required_loc: Option<(i32, WorldTile)>,
    required_npc: Option<(i32, WorldTile)>,
    require_free_world: bool,
    require_knife_arm: bool,
    mutations: Vec<DispatchMutation>,
    use_proxy: bool,
    suppress_openchat_span: Option<u32>,
    pre_batch_user_events: usize,
    pre_batch_guard_opener: bool,
}

impl Scenario {
    fn river(name: &'static str) -> Self {
        let spot = tile(2_504, 3_498, 0);
        Self {
            name,
            skill: "Fishing",
            skill_index: 10,
            level: 20,
            resource: ResourceKind::River,
            target: tile(2_503, 3_498, 0),
            tool: item("fly_fishing_rod", FLY_ROD_ID, 1),
            extra_items: vec![item("feather", FEATHER_ID, 200)],
            products: vec![TROUT_ID, SALMON_ID],
            required_loc: Some((RIVER_LOC_ID, spot)),
            required_npc: None,
            require_free_world: false,
            require_knife_arm: true,
            mutations: Vec::new(),
            use_proxy: false,
            suppress_openchat_span: None,
            pre_batch_user_events: 0,
            pre_batch_guard_opener: false,
        }
    }

    fn npc_river(name: &'static str) -> Self {
        let npc_tile = tile(
            FRESHFISH_NPC_TILE.0,
            FRESHFISH_NPC_TILE.1,
            FRESHFISH_NPC_TILE.2,
        );
        Self {
            name,
            skill: "Fishing",
            skill_index: 10,
            level: 20,
            resource: ResourceKind::NpcRiver,
            // The Gnome Stronghold freshfish spawn is adjacent to this bank
            // tile, and the setup teleports only the player's own character.
            target: tile(npc_tile.x, npc_tile.z + 1, npc_tile.level),
            tool: item("fly_fishing_rod", FLY_ROD_ID, 1),
            extra_items: vec![item("feather", FEATHER_ID, 200)],
            products: vec![TROUT_ID],
            required_loc: None,
            required_npc: Some((FRESHFISH_NPC_ID, npc_tile)),
            require_free_world: false,
            require_knife_arm: true,
            mutations: Vec::new(),
            use_proxy: false,
            suppress_openchat_span: None,
            pre_batch_user_events: 0,
            pre_batch_guard_opener: false,
        }
    }

    fn net(name: &'static str) -> Self {
        Self {
            name,
            skill: "Fishing",
            skill_index: 10,
            level: 15,
            resource: ResourceKind::Net,
            target: tile(3_267, 3_149, 0),
            tool: item("net", NET_ID, 1),
            extra_items: Vec::new(),
            products: vec![SHRIMP_ID, ANCHOVY_ID],
            required_loc: None,
            required_npc: None,
            require_free_world: false,
            require_knife_arm: true,
            mutations: Vec::new(),
            use_proxy: false,
            suppress_openchat_span: None,
            pre_batch_user_events: 0,
            pre_batch_guard_opener: false,
        }
    }
    fn copper(name: &'static str) -> Result<Self, String> {
        Ok(Self {
            name,
            skill: "Mining",
            skill_index: 14,
            level: 99,
            resource: ResourceKind::Copper,
            target: tile(3_287, 3_364, 0),
            tool: item("bronze_pickaxe", BRONZE_PICKAXE_ID, 1),
            extra_items: Vec::new(),
            products: vec![COPPER_ORE_ID],
            required_loc: Some((2_090, tile(3_286, 3_364, 0))),
            required_npc: None,
            require_free_world: false,
            require_knife_arm: true,
            mutations: Vec::new(),
            use_proxy: false,
            suppress_openchat_span: None,
            pre_batch_user_events: 0,
            pre_batch_guard_opener: false,
        })
    }

    fn iron(name: &'static str) -> Result<Self, String> {
        Ok(Self {
            name,
            skill: "Mining",
            skill_index: 14,
            // The real mithril tool starts at 21; this cell needs a failed roll.
            level: 21,
            resource: ResourceKind::Iron,
            target: mining_start()?,
            tool: item("mithril_pickaxe", MITHRIL_PICKAXE_ID, 1),
            extra_items: Vec::new(),
            products: vec![IRON_ORE_ID],
            required_loc: None,
            required_npc: None,
            require_free_world: false,
            // The first successful rock may reach an Idle replacement before a KnifeArm.
            require_knife_arm: false,
            mutations: Vec::new(),
            use_proxy: false,
            suppress_openchat_span: None,
            pre_batch_user_events: 0,
            pre_batch_guard_opener: false,
        })
    }

    fn oak(name: &'static str) -> Self {
        Self {
            name,
            skill: "Woodcutting",
            skill_index: 8,
            level: 15,
            resource: ResourceKind::Oak,
            target: tile(3_016, 3_170, 0),
            tool: item("bronze_axe", BRONZE_AXE_ID, 1),
            extra_items: Vec::new(),
            products: vec![OAK_LOG_ID],
            required_loc: Some((OAK_LOC_ID, tile(3_017, 3_170, 0))),
            required_npc: None,
            require_free_world: true,
            require_knife_arm: true,
            mutations: Vec::new(),
            use_proxy: false,
            suppress_openchat_span: None,
            pre_batch_user_events: 0,
            pre_batch_guard_opener: false,
        }
    }
    fn with_mutation(mut self, mutation: DispatchMutation) -> Self {
        self.use_proxy |= matches!(
            &mutation,
            DispatchMutation::Transport(_) | DispatchMutation::HoldOnFreshMiningRoll { .. }
        );
        self.mutations.push(mutation);
        self
    }

    fn with_openchat_suppression(mut self, edges_from_start: u32) -> Self {
        self.suppress_openchat_span = Some(edges_from_start);
        self
    }

    fn with_pre_batch_user_events(mut self, count: usize) -> Self {
        self.pre_batch_user_events = count;
        self
    }

    fn with_guard_opener(mut self) -> Self {
        self.pre_batch_guard_opener = true;
        self
    }

    fn seed_items(&self) -> Vec<SeedItem> {
        let mut items = vec![
            item("knife", KNIFE_ID, 1),
            item("logs", LOGS_ID, 1),
            self.tool.clone(),
        ];
        items.extend(self.extra_items.iter().cloned());
        items
    }

    fn settings(&self) -> Map<String, Value> {
        let mut settings = Map::new();
        settings.insert("skill".into(), json!(self.skill));
        settings.insert("tickMethod".into(), json!("Knife"));
        settings.insert(
            "woodcuttingResources".into(),
            json!(if self.resource == ResourceKind::Oak {
                vec!["oak"]
            } else {
                Vec::<&str>::new()
            }),
        );
        settings.insert(
            "miningResources".into(),
            json!(match self.resource {
                ResourceKind::Copper => vec!["copper"],
                ResourceKind::Iron => vec!["iron"],
                _ => Vec::<&str>::new(),
            }),
        );
        if matches!(
            self.resource,
            ResourceKind::River | ResourceKind::NpcRiver | ResourceKind::Net
        ) {
            settings.insert(
                "fishingMethod".into(),
                json!(match self.resource {
                    ResourceKind::River => "fishing.loc_2027.op1",
                    ResourceKind::NpcRiver => "fishing.freshfish.op1",
                    ResourceKind::Net => "fishing.saltfish.op1",
                    _ => unreachable!("non-fishing resource matched"),
                }),
            );
        }
        settings.insert(
            "targetPreference".into(),
            json!(
                if matches!(self.resource, ResourceKind::Copper | ResourceKind::Iron) {
                    "Best tier"
                } else {
                    "Nearest"
                }
            ),
        );
        settings.insert("location".into(), json!("Start"));
        settings.insert("radius".into(), json!(12));
        settings.insert("disposition".into(), json!("Power"));
        settings
    }
}

fn item(alias: &'static str, id: i32, count: i32) -> SeedItem {
    SeedItem { alias, id, count }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SetupPhase {
    WaitIngame,
    TutorialSetvar,
    TutorialGetvar,
    WaitTutorial,
    LogoutTutorial,
    WaitRelogin,
    ReseedTutorial,
    WaitReseedTutorial,
    Seed,
    Verify,
    WaitNetPosition,
    Ready,
}

#[derive(Debug, Clone, Default)]
struct ForeignEventState {
    requested: usize,
    armed: bool,
    sent: bool,
    emitted: usize,
    client_tick: Option<u32>,
    host_frame: Option<u64>,
    error: Option<String>,
}

struct SetupState {
    account: String,
    scenario: Scenario,
    commands: Vec<String>,
    next_command: usize,
    phase: SetupPhase,
    error: Option<String>,
    logout_session: Option<u64>,
    tutorial_reseed: Option<scenario::tutorial::PostRelogTutorial>,
    npc_pump: host::Pump,
    npc_snapshot: GameSnapshot,
    npc_observation_generation: Option<u64>,
    npc_observations: Vec<interact::server_ordered::ExactNpcReference>,
}

impl SetupState {
    fn new(account: String, scenario: Scenario) -> Self {
        let mut commands = Vec::new();
        if scenario.resource != ResourceKind::NpcRiver {
            commands.extend([
                "setstat attack 1".to_owned(),
                "setstat defence 99".to_owned(),
                "setstat hitpoints 99".to_owned(),
            ]);
        }
        commands.push(format!(
            "setstat {} {}",
            scenario.skill.to_ascii_lowercase(),
            scenario.level
        ));
        commands.extend(
            scenario
                .seed_items()
                .iter()
                .map(|item| format!("give {} {}", item.alias, item.count)),
        );
        commands.push(interact::tele_args(
            scenario.target.level,
            scenario.target.x,
            scenario.target.z,
        ));
        Self {
            account,
            scenario,
            commands,
            next_command: 0,
            phase: SetupPhase::WaitIngame,
            error: None,
            logout_session: None,
            tutorial_reseed: None,
            npc_pump: host::Pump::default(),
            npc_snapshot: GameSnapshot::new(),
            npc_observation_generation: None,
            npc_observations: Vec::new(),
        }
    }

    fn frame(&mut self, client: &mut Client, account: &str, hold: bool) {
        if account != self.account || hold || self.error.is_some() {
            return;
        }
        if self.phase == SetupPhase::Ready {
            if self.scenario.resource == ResourceKind::NpcRiver {
                self.observe_npc(client);
            }
            return;
        }
        let mut snapshot = GameSnapshot::new();
        snapshot.rebuild(client);
        match self.phase {
            SetupPhase::WaitIngame => {
                if client.ingame && client.scene_state == 2 {
                    self.phase = SetupPhase::TutorialSetvar;
                }
            }
            SetupPhase::TutorialSetvar => {
                if interact::cheat(client, scenario::tutorial::TUTORIAL_SETVAR).is_sent() {
                    self.phase = SetupPhase::TutorialGetvar;
                } else {
                    self.error = Some("tutorial setvar command was refused".into());
                }
            }
            SetupPhase::TutorialGetvar => {
                if interact::cheat(client, scenario::tutorial::TUTORIAL_GETVAR).is_sent() {
                    self.phase = SetupPhase::WaitTutorial;
                } else {
                    self.error = Some("tutorial getvar command was refused".into());
                }
            }
            SetupPhase::WaitTutorial => {
                if scenario::tutorial::tutorial_confirmed(&snapshot, 0) {
                    self.phase = SetupPhase::LogoutTutorial;
                }
            }
            SetupPhase::LogoutTutorial => {
                let ifaces = Arc::clone(&client.ifaces);
                if interact::logout(client, &ifaces) {
                    self.logout_session = Some(client.gens.session);
                    self.phase = SetupPhase::WaitRelogin;
                } else {
                    self.error = Some("tutorial completion logout was refused".into());
                }
            }
            SetupPhase::WaitRelogin => {
                if client.ingame
                    && client.scene_state == 2
                    && self
                        .logout_session
                        .is_some_and(|session| session != client.gens.session)
                {
                    self.phase = SetupPhase::ReseedTutorial;
                }
            }
            SetupPhase::ReseedTutorial => {
                let reseed = scenario::tutorial::PostRelogTutorial::new(
                    scenario::tutorial::chat_baseline(&snapshot),
                );
                reseed.send_reseed(client);
                self.tutorial_reseed = Some(reseed);
                self.phase = SetupPhase::WaitReseedTutorial;
            }
            SetupPhase::WaitReseedTutorial => {
                match self
                    .tutorial_reseed
                    .as_ref()
                    .expect("tutorial reseed armed")
                    .check(&snapshot)
                {
                    Ok(true) => {
                        println!("{}", scenario::tutorial::confirmation_log());
                        self.phase = SetupPhase::Seed;
                    }
                    Ok(false) => {}
                    Err(error) => self.error = Some(error),
                }
            }
            SetupPhase::Seed => {
                if let Some(command) = self.commands.get(self.next_command) {
                    if interact::cheat(client, command).is_sent() {
                        self.next_command += 1;
                        if self.next_command == self.commands.len() {
                            self.phase = SetupPhase::Verify;
                        }
                    } else {
                        self.error = Some(format!("engine refused setup cheat {command:?}"));
                    }
                }
            }
            SetupPhase::Verify => {
                if self.ready_in_snapshot(&snapshot) {
                    if self.scenario.resource == ResourceKind::Net {
                        if let Err(error) = self.approach_net(client, &snapshot) {
                            self.error = Some(error);
                        }
                    } else {
                        self.observe_npc(client);
                        self.phase = SetupPhase::Ready;
                    }
                }
            }
            SetupPhase::WaitNetPosition => {
                if self.ready_in_snapshot(&snapshot) {
                    self.phase = SetupPhase::Ready;
                }
            }
            SetupPhase::Ready => {}
        }
    }

    fn observe_npc(&mut self, client: &Client) {
        let Some((type_id, _)) = self.scenario.required_npc else {
            return;
        };
        // Match the production host's retained publication stamp, not a fresh
        // snapshot's default stamp. Keep received-packet cadence separate.
        let drain = self.npc_pump.drain_client(client);
        host::publish_snapshot(&mut self.npc_snapshot, client, drain);
        let snapshot = &self.npc_snapshot;
        let generation = client.gens.npc_info;
        if self.npc_observation_generation == Some(generation) {
            return;
        }
        self.npc_observation_generation = Some(generation);
        // Record each observed row, not the first NPC sharing the display
        // spawn. Gatherer may choose another real spot after normal movement.
        // from_snapshot uses NpcView.network: build base plus route[0], not
        // the rendered tile (snapshot/decode.rs:1388).
        for npc in snapshot
            .npcs()
            .iter()
            .filter(|npc| npc.r#type.and_then(|id| i32::try_from(id).ok()) == Some(type_id))
        {
            if let Ok(reference) =
                interact::server_ordered::ExactNpcReference::from_snapshot(snapshot, npc, 1)
            {
                self.npc_observations.push(reference);
            }
        }
    }

    fn approach_net(&mut self, client: &mut Client, snapshot: &GameSnapshot) -> Result<(), String> {
        let (x, z, level) = snapshot.tile().ok_or("Net setup has no player tile")?;
        let here = tile(x, z, level);
        let npc = snapshot
            .npcs()
            .iter()
            .filter(|npc| npc.r#type == Some(330))
            .min_by_key(|npc| api::query::chebyshev_to(here, npc.network))
            .ok_or("Net setup has no ordinary small-net spot")?;
        let (base_x, base_z) = snapshot.base().ok_or("Net setup has no scene base")?;
        // Spots move during normal content. Complete a real path to the
        // currently observed ordinary spot before the measured Start window.
        if !client.tryMove(
            x - base_x,
            z - base_z,
            npc.network.x - base_x,
            npc.network.z - base_z,
            false,
            npc.size,
            npc.size,
            0,
            0,
            0,
            0,
        ) {
            return Err("Net setup could not path to the observed ordinary spot".into());
        }
        self.scenario.target = tile(
            client.minimap_flag_x + base_x,
            client.minimap_flag_z + base_z,
            level,
        );
        self.phase = SetupPhase::WaitNetPosition;
        Ok(())
    }

    fn ready_in_snapshot(&self, snapshot: &GameSnapshot) -> bool {
        let at_target = snapshot.tile().is_some_and(|(x, z, level)| {
            (x, z, level)
                == (
                    self.scenario.target.x,
                    self.scenario.target.z,
                    self.scenario.target.level,
                )
        });
        let level_ready = snapshot
            .stats()
            .iter()
            .find(|stat| stat.index == self.scenario.skill_index)
            .is_some_and(|stat| stat.base >= self.scenario.level);
        let items_ready = self
            .scenario
            .seed_items()
            .iter()
            .all(|item| snapshot.inv_count(item.id) >= item.count);
        let loc_ready = self.scenario.required_loc.is_none_or(|(id, target)| {
            snapshot
                .locs()
                .iter()
                .any(|loc| loc.id == id && loc.tile == target)
        });
        let npc_ready = match self.scenario.resource {
            ResourceKind::Net => snapshot.npcs().iter().any(|npc| {
                npc.r#type == Some(330)
                    && (self.phase != SetupPhase::WaitNetPosition
                        || (npc.network.level == self.scenario.target.level
                            && (npc.network.x - self.scenario.target.x).abs()
                                + (npc.network.z - self.scenario.target.z).abs()
                                == 1))
            }),
            ResourceKind::NpcRiver => {
                self.scenario.required_npc.is_some_and(|(type_id, target)| {
                    snapshot.tile().is_some_and(|(x, z, level)| {
                        level == target.level
                            && snapshot.npcs().iter().any(|npc| {
                                npc.r#type.and_then(|id| i32::try_from(id).ok()) == Some(type_id)
                                    && npc.network == target
                                    && (x - target.x).abs() + (z - target.z).abs() == 1
                                    && npc.actions.first().is_some_and(Option::is_some)
                            })
                    })
                })
            }
            _ => true,
        };
        at_target && level_ready && items_ready && loc_ready && npc_ready
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct StatusObservation {
    host_frame: u64,
    game_tick: u32,
    phase: String,
    method: Option<String>,
    tick_method: Option<String>,
    target: Option<String>,
    last_event: Option<String>,
    tick_cycle: Option<i64>,
    tick_verified: Option<i64>,
    tick_unobserved: Option<i64>,
    tick_slips: Option<i64>,
    tick_lost: Option<i64>,
    tick_delayed: Option<i64>,
    tick_paused: Option<i64>,
    tick_fallbacks: Option<i64>,
    tick_fallback_failed: Option<i64>,
    tick_events: Option<i64>,
}

impl StatusObservation {
    fn capture(status: &ScriptStatus, host_frame: u64, game_tick: u32) -> Self {
        Self {
            host_frame,
            game_tick,
            phase: format!("{:?}", status.phase),
            method: text_field(status, "method"),
            tick_method: text_field(status, "tick_method"),
            target: text_field(status, "target"),
            last_event: text_field(status, "last_event"),
            tick_cycle: integer_field(status, "tick_cycle"),
            tick_verified: integer_field(status, "tick_verified"),
            tick_unobserved: integer_field(status, "tick_unobserved"),
            tick_slips: integer_field(status, "tick_slips"),
            tick_lost: integer_field(status, "tick_lost"),
            tick_delayed: integer_field(status, "tick_delayed"),
            tick_paused: integer_field(status, "tick_paused"),
            tick_fallbacks: integer_field(status, "tick_fallbacks"),
            tick_fallback_failed: integer_field(status, "tick_fallback_failed"),
            tick_events: integer_field(status, "tick_events"),
        }
    }
}

fn text_field(status: &ScriptStatus, key: &str) -> Option<String> {
    status
        .fields
        .iter()
        .find(|field| field.key == key)
        .and_then(|field| match &field.value {
            StatusValue::Text(value) => Some(value.to_string()),
            _ => None,
        })
}

fn integer_field(status: &ScriptStatus, key: &str) -> Option<i64> {
    status
        .fields
        .iter()
        .find(|field| field.key == key)
        .and_then(|field| match &field.value {
            StatusValue::Integer(value) => Some(*value),
            _ => None,
        })
}

#[derive(Debug, Clone)]
struct RunEvidence {
    case: &'static str,
    account: String,
    profile: String,
    world_members: String,
    product_ids: Vec<i32>,
    pre_batch_user_events: ForeignEventState,
    pre_batch_guard_opener: GuardOpenerState,
    start_tick: Option<u32>,
    baseline: Option<FrameObservation>,
    npc_observations: Vec<interact::server_ordered::ExactNpcReference>,
    trace: TickEngineTrace,
    statuses: Vec<StatusObservation>,
    drop_history: Option<drop_history::DropHistory>,
    transport: Vec<ProxyObservation>,
    engine_packets: Option<Result<Vec<EnginePacket>, String>>,
}

fn send_pre_batch_user_events(
    client: &mut Client,
    hold: bool,
    hook: &TickEngineTestHook,
    state: &Arc<Mutex<ForeignEventState>>,
) {
    if hold {
        return;
    }
    let unavailable = {
        let state = state
            .lock()
            .expect("tick-engine foreign event state poisoned");
        !state.armed || state.sent || state.error.is_some()
    };
    if unavailable {
        return;
    }
    let mut snapshot = GameSnapshot::new();
    snapshot.rebuild(client);
    let Some((previous_tick, previous_animation)) = hook.observed_client_frame() else {
        return;
    };
    if !snapshot.ingame()
        || snapshot.scene_state() != 2
        || snapshot.tick() == previous_tick
        // This river case needs foreign events immediately before the first
        // Knife batch, not before Start's earlier ordinary native click.
        || client.local_animation_update().is_none_or(|update| {
            update.sequence != ROLL_ANIMATION
                || previous_animation.is_some_and(|previous| previous.serial == update.serial)
        })
    {
        return;
    }
    let client_tick = snapshot.tick();
    let host_frame = hook.observed_host_frame();
    let requested = {
        let mut state = state
            .lock()
            .expect("tick-engine foreign event state poisoned");
        if !state.armed || state.sent || state.error.is_some() {
            return;
        }
        state.armed = false;
        state.requested
    };
    if requested == 0 {
        return;
    }
    let Some((source_x, source_z)) = snapshot.tile().and_then(|(world_x, world_z, _)| {
        snapshot
            .base()
            .map(|(base_x, base_z)| (world_x - base_x, world_z - base_z))
    }) else {
        let mut state = state
            .lock()
            .expect("tick-engine foreign event state poisoned");
        state.client_tick = Some(client_tick);
        state.host_frame = Some(host_frame);
        state.error = Some("real edge had no local-player tile or scene base".into());
        return;
    };
    let mut returned_moves = 0;
    for _ in 0..requested {
        // Real same-tile walks charge user events without moving the actor
        // into the river or depending on neighboring terrain being passable.
        if !client.tryMove(
            source_x, source_z, source_x, source_z, false, 0, 0, 0, 0, 0, 0,
        ) {
            break;
        }
        returned_moves += 1;
    }
    let observed_moves = hook
        .trace()
        .wire
        .iter()
        .filter(|wire| {
            wire.host_frame == host_frame
                && wire.request_id.is_none()
                && wire.opcode == ClientProt289::MOVE_GAMECLICK.id as u8
        })
        .count();
    let mut state = state
        .lock()
        .expect("tick-engine foreign event state poisoned");
    state.client_tick = Some(client_tick);
    state.host_frame = Some(host_frame);
    state.emitted = observed_moves;
    state.sent = returned_moves == requested && observed_moves == requested;
    if !state.sent {
        state.error = Some(format!(
            "requested {requested} MOVE_GAMECLICK events; tryMove returned {returned_moves}, real wire recorded {observed_moves}"
        ));
    }
}

struct TempRoot(PathBuf);

impl TempRoot {
    fn new(label: &str) -> Result<Self, String> {
        let serial = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| format!("clock: {error}"))?
            .as_nanos();
        let path = std::env::temp_dir().join(format!("274bot-tick-engine-{label}-{serial}"));
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

struct LiveRun {
    scenario: Scenario,
    _isolated_home: script::IsolatedEnv,
    _temp: TempRoot,
    account: String,
    password: Option<String>,
    profile_identity: String,
    world_members: String,
    setup: Arc<Mutex<SetupState>>,
    foreign_events: Arc<Mutex<ForeignEventState>>,
    guard_opener: Arc<Mutex<GuardOpenerState>>,
    hook: Arc<TickEngineTestHook>,
    proxy: Option<Arc<LoopbackProxy>>,
    play: Option<host_play::Play>,
    start_tick: Option<u32>,
    first_arm_budget: u32,
    baseline: Option<FrameObservation>,
    statuses: Vec<StatusObservation>,
    drop_history: Option<Arc<Mutex<drop_history::DropHistory>>>,
    engine_log: Option<EnginePacketLog>,
    shutdown: bool,
}

impl LiveRun {
    fn new(scenario: Scenario) -> Result<Self, String> {
        require_live_config()?;
        let isolated_home = script::IsolatedEnv::enter(scenario.name);
        api::hostlog::set_debug(true);
        let nav_pack = required_file("GATHERER_NAV_PACK")?;
        let engine_dir = required_dir("GATHERER_ENGINE_DIR")?;
        let catalog_root = required_dir("GATHERER_CATALOG_ROOT")?;
        let temp = TempRoot::new(scenario.name)?;
        let proxy = if scenario.use_proxy {
            Some(Arc::new(
                LoopbackProxy::start(SocketAddr::from(([127, 0, 0, 1], ENGINE_GAME_PORT)))
                    .map_err(|error| format!("start Engine T loopback proxy: {error}"))?,
            ))
        } else {
            None
        };
        let game_port = proxy
            .as_ref()
            .map_or(ENGINE_GAME_PORT, |proxy| proxy.address().port());
        let options = ProfileOptions {
            profile: Some("local-289".into()),
            revision: Some("289".into()),
            host: Some(ENGINE_HOST.into()),
            asset_host: Some(ENGINE_HOST.into()),
            port: Some(game_port),
            http_port: Some(ENGINE_HTTP_PORT),
            cache_dir: std::env::var_os("BOT_CACHE_DIR").map(PathBuf::from),
            nav_pack: Some(nav_pack),
            nav_flags: std::env::var_os("GATHERER_NAV_FLAGS").map(PathBuf::from),
            engine_dir: Some(engine_dir),
            vault_path: Some(temp.path().join("vault")),
            catalog_root: Some(catalog_root),
            ..ProfileOptions::default()
        };
        let profile = options.resolve(None)?.bind()?;
        if profile.profile_class() != ProfileClass::Local
            || profile.client().game_host() != ENGINE_HOST
            || profile.client().asset_host() != ENGINE_HOST
            || profile.client().asset_port() != ENGINE_HTTP_PORT
            || profile.client().game_port() != game_port
        {
            return Err("tick-engine cases require the assigned loopback Engine T profile".into());
        }
        let world_members = format!("{:?}", profile.world_members());
        if scenario.require_free_world
            && !matches!(
                profile.world_members(),
                WorldMembersFact::Known {
                    members: false,
                    source: WorldMembersSource::LocalWorldJson { .. }
                }
            )
        {
            return Err(format!(
                "free-world case requires selected local world.json with members=false; actual bound fact is {:?}",
                profile.world_members()
            ));
        }
        let template = SharedClientTemplate::load(Arc::clone(&profile))?;
        if template.world().is_none() {
            return Err("selected Engine T navigation template has no world".into());
        }
        let first_arm_budget = tick_startup::first_arm_budget(
            profile
                .game_data()
                .ok_or("Engine T profile has no selected data")?,
            match scenario.resource {
                ResourceKind::River => "fishing.loc_2027.op1",
                ResourceKind::NpcRiver => "fishing.freshfish.op1",
                ResourceKind::Net => "fishing.saltfish.op1",
                ResourceKind::Copper => "mining.copper",
                ResourceKind::Iron => "mining.iron",
                ResourceKind::Oak => "woodcutting.oak",
            },
            scenario.tool.id,
            STARTUP_EDGE_BUDGET,
        )?;
        // Asset initialization is not a live host frame or a cadence observation.
        // Complete it before the slot's fixed setup deadline starts.
        let mut bootstrap = template.prepare_client(0, true)?;
        bootstrap.set_draw(false);
        bootstrap.maininit();
        if bootstrap.error_loading {
            return Err("Engine T client asset initialization failed before live setup".into());
        }
        drop(bootstrap);
        let profile_identity = format!(
            "{} game={}:{} asset={}:{} nav={}",
            profile.label(),
            profile.client().game_host(),
            profile.client().game_port(),
            profile.client().asset_host(),
            profile.client().asset_port(),
            profile.nav_pack().display()
        );
        let names = host_play::mint_live_names(1);
        let account = names
            .first()
            .cloned()
            .ok_or("live account mint returned no name")?;
        let (minted_account, password) = host_play::mint_live_entries(&names)
            .into_iter()
            .next()
            .ok_or("live credential mint returned no account")?;
        if minted_account != account {
            return Err("live credential mint changed the account name".into());
        }
        if scenario.resource == ResourceKind::NpcRiver && !account.starts_with("tmx") {
            return Err(format!(
                "NPC fishing live proof requires BOT_LIVE_NAME_PREFIX=tmx; minted {account}"
            ));
        }
        let setup = Arc::new(Mutex::new(SetupState::new(
            account.clone(),
            scenario.clone(),
        )));
        let foreign_events = Arc::new(Mutex::new(ForeignEventState {
            requested: scenario.pre_batch_user_events,
            ..ForeignEventState::default()
        }));
        let guard_opener = Arc::new(Mutex::new(GuardOpenerState::default()));
        let hook = TickEngineTestHook::new(account.clone());
        let drop_history = (scenario.name == "tick-make-proxy-drop")
            .then(|| Arc::new(Mutex::new(drop_history::DropHistory::default())));
        hook.install();
        if let Some(proxy) = &proxy {
            hook.set_transport_controller(proxy.clone());
        }
        let frame_setup = Arc::clone(&setup);
        let frame_foreign_events = Arc::clone(&foreign_events);
        let frame_guard_opener = Arc::clone(&guard_opener);
        let frame_drop_history = drop_history.as_ref().map(Arc::clone);
        let frame_hook = Arc::clone(&hook);
        let frame_account = account.clone();
        let play = match host_play::run_with_template(
            Arc::clone(&template),
            true,
            vec![],
            |_| (None, None),
            move |client, username, frame| {
                if username == frame_account {
                    if let Some(history) = &frame_drop_history {
                        history
                            .lock()
                            .expect("tick-engine drop history poisoned")
                            .observe_client(client, frame_hook.observed_host_frame());
                    }
                    frame_setup
                        .lock()
                        .expect("tick-engine setup poisoned")
                        .frame(client, username, frame.hold);
                    send_pre_batch_user_events(
                        client,
                        frame.hold,
                        &frame_hook,
                        &frame_foreign_events,
                    );
                    guard_opener::frame(client, frame.hold, &frame_hook, &frame_guard_opener);
                }
            },
        ) {
            Ok(play) => play,
            Err(error) => {
                hook.uninstall();
                return Err(error);
            }
        };
        Ok(Self {
            scenario,
            _isolated_home: isolated_home,
            _temp: temp,
            account,
            password: Some(password),
            profile_identity,
            world_members,
            setup,
            foreign_events,
            guard_opener,
            hook,
            proxy,
            play: Some(play),
            start_tick: None,
            first_arm_budget,
            baseline: None,
            statuses: Vec::new(),
            drop_history,
            engine_log: None,
            shutdown: false,
        })
    }

    fn start(&mut self) -> Result<(), String> {
        if self.start_tick.is_some() {
            return Err("Gatherer Start was already submitted".into());
        }
        let password = self
            .password
            .take()
            .ok_or("live password was already consumed")?;
        let uid = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| format!("uid clock: {error}"))?
            .as_millis() as i32;
        let profile = Profile {
            username: self.account.clone(),
            password: password.into(),
            uid: uid.saturating_add(1),
            settings: ProfileSettings::default(),
        };
        self.play_mut()
            .try_spawn_slot(profile, None, None, None)
            .map_err(|error| format!("spawn Engine T account: {error}"))?;
        self.play
            .as_mut()
            .expect("live Play exists until shutdown")
            .focus(&self.account);
        self.wait_setup_ready()?;
        if self.drop_history.is_some() {
            drop_history::clear_host_log()?;
        }
        self.scenario.target = self
            .setup
            .lock()
            .map_err(|_| "tick-engine setup poisoned")?
            .scenario
            .target;
        self.engine_log = Some(EnginePacketLog::capture(required_file(
            "TICK_ENGINE_PACKET_LOG",
        )?)?);
        let after = self.hook.observed_host_frame();
        self.hook.arm_measurement(LOG_ID, &self.scenario.products);
        if self.hook.wait_for_frame(after, FRAME_HANG_GUARD).is_none() {
            return Err("no host frame after arming tick-engine evidence".into());
        }
        let trace = self.hook.trace();
        let baseline = trace
            .frames
            .last()
            .cloned()
            .ok_or("tick-engine hook did not capture the pre-Start baseline")?;
        self.start_tick = Some(baseline.game_tick);
        self.baseline = Some(baseline.clone());
        if let Some(span) = self.scenario.suppress_openchat_span {
            // Setup/Start precedes the first native roll. Keep the bounded
            // filter alive through the first-arm deadline and the case span.
            self.hook.suppress_openchat_through_tick(
                baseline
                    .game_tick
                    .wrapping_add(self.first_arm_budget)
                    .wrapping_add(span),
            );
        }
        for mutation in self.scenario.mutations.iter().cloned() {
            self.hook.queue_dispatch_mutation(mutation);
        }
        println!(
            "TICK-MANIP start case={} account={} profile={} world_members={} baseline_tick={} target={:?} seed_items={:?}",
            self.scenario.name,
            self.account,
            self.profile_identity,
            self.world_members,
            baseline.game_tick,
            self.scenario.target,
            self.scenario.seed_items()
        );
        if self.scenario.pre_batch_user_events > 0 {
            self.foreign_events
                .lock()
                .map_err(|_| "tick-engine foreign event state poisoned")?
                .armed = true;
        }
        if self.scenario.pre_batch_guard_opener {
            self.guard_opener
                .lock()
                .map_err(|_| "tick-engine guard opener state poisoned")?
                .armed = true;
        }
        self.play
            .as_mut()
            .expect("live Play exists until shutdown")
            .script_start(
                &self.account,
                script::CompiledId("Gatherer"),
                self.scenario.settings(),
            )
            .map_err(|error| format!("Gatherer operator Start failed: {error}"))?;
        if self.scenario.require_knife_arm {
            self.wait_until(baseline.game_tick, STARTUP_EDGE_BUDGET, |trace, _| {
                tick_startup::native_click_within(trace, baseline.game_tick, STARTUP_EDGE_BUDGET)
            })?;
            self.wait_until(baseline.game_tick, self.first_arm_budget, |trace, _| {
                trace.dispatches.iter().any(is_knife_arm)
            })?;
        } else {
            self.wait_until(baseline.game_tick, STARTUP_EDGE_BUDGET, |_, statuses| {
                statuses.iter().any(|status| status.tick_method.is_some())
            })?;
        }
        let foreign_events = self
            .foreign_events
            .lock()
            .map_err(|_| "tick-engine foreign event state poisoned")?
            .clone();
        if foreign_events.requested > 0
            && (!foreign_events.sent
                || foreign_events.emitted != foreign_events.requested
                || foreign_events.error.is_some())
        {
            return Err(self.failure(format!(
                "pre-batch user events did not reach the real Client wire: {foreign_events:?}"
            )));
        }
        if self.play().script_state(&self.account) != script::RunState::Running {
            return Err(self.failure("Gatherer native loop did not reach Running after Start"));
        }
        Ok(())
    }

    fn wait_setup_ready(&self) -> Result<(), String> {
        let deadline = Instant::now() + SETUP_DEADLINE;
        let mut login_posted = false;
        let mut after = self.hook.observed_host_frame();
        loop {
            {
                let setup = self
                    .setup
                    .lock()
                    .map_err(|_| "tick-engine setup poisoned")?;
                if let Some(error) = &setup.error {
                    return Err(error.clone());
                }
                // The fixture logs its bot out to complete the tutorial. Post a real
                // one-shot Login once after that logout; an unlatched slot with
                // auto-login would also reconnect by itself.
                if setup.phase == SetupPhase::WaitRelogin && !login_posted {
                    login_posted = true;
                    if let Some(arm) = self.play().arm(&self.account) {
                        arm.arm_explicit_login();
                    }
                }
                if setup.phase == SetupPhase::Ready {
                    return Ok(());
                }
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            let frame = if remaining.is_zero() {
                None
            } else {
                self.hook.wait_for_frame(after, remaining)
            };
            let Some(frame) = frame else {
                let setup = self
                    .setup
                    .lock()
                    .map_err(|_| "tick-engine setup poisoned")?;
                return Err(format!(
                    "live setup did not reach Ready within {SETUP_DEADLINE:?}; phase={:?} command={}/{} last_host_frame={} connected={}",
                    setup.phase,
                    setup.next_command,
                    setup.commands.len(),
                    self.hook.observed_host_frame(),
                    self.play().slot_connected(&self.account)
                ));
            };
            after = frame;
        }
    }

    fn wait_until(
        &mut self,
        origin_tick: u32,
        tick_budget: u32,
        mut satisfied: impl FnMut(&TickEngineTrace, &[StatusObservation]) -> bool,
    ) -> Result<(), String> {
        let mut after_host_frame = self.hook.observed_host_frame();
        loop {
            let trace = self.hook.trace();
            self.capture_status(&trace);
            if satisfied(&trace, &self.statuses) {
                return Ok(());
            }
            if let Some(frame) = trace.frames.last() {
                let elapsed = frame.game_tick.wrapping_sub(origin_tick);
                if elapsed > tick_budget {
                    return Err(self.failure(format!(
                        "content tick budget exhausted: origin={origin_tick} current={} budget={tick_budget}",
                        frame.game_tick
                    )));
                }
            }
            after_host_frame = self
                .hook
                .wait_for_frame(after_host_frame, FRAME_HANG_GUARD)
                .ok_or_else(|| {
                    self.failure(format!(
                        "host-frame hang guard expired while waiting within {tick_budget} game ticks"
                    ))
                })?;
        }
    }

    fn capture_status(&mut self, trace: &TickEngineTrace) {
        let host_frame = trace
            .frames
            .last()
            .map_or_else(|| self.hook.observed_host_frame(), |frame| frame.host_frame);
        if let Some(history) = &self.drop_history {
            let slot_statuses = self.play().statuses();
            let host_log = drop_history::host_log_for(&self.account);
            let mut history = history.lock().expect("tick-engine drop history poisoned");
            if let Some(status) = slot_statuses
                .iter()
                .find(|status| status.username == self.account)
            {
                history.observe_slot(status, host_frame);
            }
            history.replace_host_log(host_log);
        }
        let statuses = &mut self.statuses;
        self.hook
            .drain_native_statuses(|host_frame, game_tick, status| {
                statuses.push(StatusObservation::capture(status, host_frame, game_tick));
            });
    }

    fn evidence(&self) -> RunEvidence {
        RunEvidence {
            case: self.scenario.name,
            account: self.account.clone(),
            profile: self.profile_identity.clone(),
            world_members: self.world_members.clone(),
            product_ids: self.scenario.products.clone(),
            pre_batch_user_events: self
                .foreign_events
                .lock()
                .expect("tick-engine foreign event state poisoned")
                .clone(),
            pre_batch_guard_opener: self
                .guard_opener
                .lock()
                .expect("tick-engine guard opener state poisoned")
                .clone(),
            start_tick: self.start_tick,
            baseline: self.baseline.clone(),
            npc_observations: self
                .setup
                .lock()
                .expect("tick-engine setup poisoned")
                .npc_observations
                .clone(),
            trace: self.hook.trace(),
            statuses: self.statuses.clone(),
            drop_history: self.drop_history.as_ref().map(|history| {
                history
                    .lock()
                    .expect("tick-engine drop history poisoned")
                    .clone()
            }),
            transport: self
                .proxy
                .as_ref()
                .map_or_else(Vec::new, |proxy| proxy.observations()),
            engine_packets: self.engine_log.as_ref().map(|log| log.read(&self.account)),
        }
    }

    fn failure(&self, message: impl std::fmt::Display) -> String {
        let trace = self.hook.trace();
        format!(
            "tick-engine case {} failed: {message}\nlast frame: {:?}\nlast status: {:?}",
            self.scenario.name,
            trace.frames.last(),
            self.statuses.last()
        )
    }

    fn play(&self) -> &host_play::Play {
        self.play.as_ref().expect("live Play exists until shutdown")
    }

    fn play_mut(&mut self) -> &mut host_play::Play {
        self.play.as_mut().expect("live Play exists until shutdown")
    }

    fn counter(&self, key: &str) -> Option<i64> {
        let status = self.statuses.last()?;
        match key {
            "tick_cycle" => status.tick_cycle,
            "tick_verified" => status.tick_verified,
            "tick_unobserved" => status.tick_unobserved,
            "tick_slips" => status.tick_slips,
            "tick_lost" => status.tick_lost,
            "tick_delayed" => status.tick_delayed,
            "tick_paused" => status.tick_paused,
            "tick_fallbacks" => status.tick_fallbacks,
            "tick_fallback_failed" => status.tick_fallback_failed,
            "tick_events" => status.tick_events,
            _ => None,
        }
    }

    fn product_count(&self, frame: &FrameObservation) -> i32 {
        frame
            .inventory_counts
            .iter()
            .filter(|(id, _)| self.scenario.products.contains(id))
            .map(|(_, count)| *count)
            .sum()
    }

    fn product_gain(&self, trace: &TickEngineTrace) -> i32 {
        let baseline = self
            .baseline
            .as_ref()
            .map_or(0, |frame| self.product_count(frame));
        trace
            .frames
            .last()
            .map_or(0, |frame| self.product_count(frame) - baseline)
    }

    fn skill_xp_gain(&self, trace: &TickEngineTrace) -> i32 {
        let baseline = self
            .baseline
            .as_ref()
            .and_then(|frame| skill_xp(frame, self.scenario.skill_index))
            .unwrap_or(0);
        trace
            .frames
            .last()
            .and_then(|frame| skill_xp(frame, self.scenario.skill_index))
            .unwrap_or(baseline)
            - baseline
    }

    fn logs_unchanged(&self, trace: &TickEngineTrace) -> bool {
        let Some(baseline) = &self.baseline else {
            return false;
        };
        trace
            .frames
            .iter()
            .all(|frame| frame.log_count == baseline.log_count)
    }

    fn shutdown(&mut self) -> Result<(), String> {
        if self.shutdown {
            return Ok(());
        }
        let Some(play) = self.play.as_mut() else {
            self.hook.uninstall();
            self.shutdown = true;
            return Ok(());
        };
        play.script_stop(&self.account);
        if let Some(arm) = play.arm(&self.account) {
            arm.request_logout();
        }
        play.wake(&self.account);
        let deadline = Instant::now() + LOGOUT_DEADLINE;
        let mut after = self.hook.observed_host_frame();
        while play.slot_connected(&self.account) && Instant::now() < deadline {
            let remaining = deadline.saturating_duration_since(Instant::now());
            match self
                .hook
                .wait_for_frame(after, remaining.min(FRAME_HANG_GUARD))
            {
                Some(frame) => after = frame,
                None => break,
            }
        }
        let still_connected = play.slot_connected(&self.account);
        play.stop_slot(&self.account);
        self.hook.uninstall();
        self.play.take();
        let proxy_result = self
            .proxy
            .as_ref()
            .map_or(Ok(()), |proxy| proxy.ensure_healthy());
        self.proxy.take();
        self.shutdown = true;
        if still_connected {
            return Err(format!(
                "account {} did not log out within {LOGOUT_DEADLINE:?}",
                self.account
            ));
        }
        proxy_result
    }
}

impl Drop for LiveRun {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}

fn run_case(
    scenario: Scenario,
    assertion: impl FnOnce(&mut LiveRun) -> Result<(), String>,
) -> Result<RunEvidence, String> {
    let _serial = LIVE_RUN_LOCK
        .lock()
        .map_err(|_| "tick-engine live-run lock poisoned")?;
    if scenario.name == "tick-make-proxy-drop" {
        drop_history::begin_case()?;
    }
    let mut run = LiveRun::new(scenario)?;
    let result = run.start().and_then(|()| assertion(&mut run));
    let account = run.account.clone();
    run.play_mut().script_stop(&account);
    let mut evidence = run.evidence();
    let cleanup = run.shutdown();
    // The engine log below includes logout. Keep the Client's packet stream
    // through that same boundary, rather than compare it with a pre-logout copy.
    evidence.trace = run.hook.trace();
    evidence.engine_packets = run.engine_log.as_ref().map(|log| log.read(&run.account));
    let correlation = correlate_engine_packets(&evidence);
    println!(
        "TICK-MANIP evidence case={} account={} frames={} dispatches={} wire_packets={}",
        evidence.case,
        evidence.account,
        evidence.trace.frames.len(),
        evidence.trace.dispatches.len(),
        evidence.trace.wire.len()
    );
    let persisted = persist_evidence(&evidence);
    let mut failures = Vec::new();
    if let Err(error) = result {
        failures.push(error);
    }
    if let Err(error) = correlation {
        failures.push(error);
    }
    if let Err(error) = persisted {
        failures.push(error);
    }
    if let Err(error) = cleanup {
        failures.push(format!("cleanup also failed: {error}"));
    }
    if failures.is_empty() {
        Ok(evidence)
    } else {
        Err(failures.join("; "))
    }
}

fn correlate_engine_packets(evidence: &RunEvidence) -> Result<(), String> {
    let packets = evidence
        .engine_packets
        .as_ref()
        .ok_or("Engine T packet capture was not armed")?
        .as_ref()
        .map_err(|error| format!("Engine T packet capture failed: {error}"))?;
    if evidence.case == "tick-make-proxy-drop" {
        return correlate_dropped_cipher_suffix(evidence, packets);
    }
    let relevant = |opcode| {
        [
            ClientProt289::OPHELDU.id as u8,
            ClientProt289::IF_BUTTON.id as u8,
            ClientProt289::MOVE_GAMECLICK.id as u8,
            ClientProt289::MOVE_OPCLICK.id as u8,
            ClientProt289::OPLOC1.id as u8,
            ClientProt289::OPNPC1.id as u8,
        ]
        .contains(&opcode)
    };
    let mut dropped_packets = HashSet::<(u64, usize)>::new();
    for observation in evidence
        .transport
        .iter()
        .filter(|observation| observation.dropped_bytes > 0)
    {
        let host_packets = wires_for_request(&evidence.trace, observation.request_id);
        let total_bytes: usize = observation.packet_lengths.iter().sum();
        let mut forwarded_count = 0;
        let mut forwarded_bytes = 0;
        while forwarded_count < observation.packet_lengths.len()
            && forwarded_bytes < observation.forwarded_bytes
        {
            forwarded_bytes += observation.packet_lengths[forwarded_count];
            forwarded_count += 1;
        }
        if !observation.mutation.starts_with("DropSuffixAfterPacket(")
            || observation.request_ids.len() != 1
            || observation.request_ids.first() != Some(&observation.request_id)
            || observation.packet_requests.len() != observation.packet_opcodes.len()
            || observation.packet_opcodes.len() != observation.packet_lengths.len()
            || host_packets.len() != observation.packet_opcodes.len()
            || forwarded_bytes != observation.forwarded_bytes
            || forwarded_count == 0
            || forwarded_count == observation.packet_lengths.len()
            || observation.dropped_bytes
                != observation.packet_lengths[forwarded_count..]
                    .iter()
                    .sum::<usize>()
            || observation.forwarded_bytes + observation.dropped_bytes != total_bytes
        {
            return Err(format!(
                "proxy did not report a complete byte-preserving dropped suffix: {observation:?}"
            ));
        }
        for (index, (wire, (request_id, packet_index))) in host_packets
            .iter()
            .zip(&observation.packet_requests)
            .enumerate()
        {
            if *request_id != observation.request_id
                || *packet_index != index + 1
                || wire.opcode != observation.packet_opcodes[index]
                || wire.encoded_length != observation.packet_lengths[index]
            {
                return Err(format!(
                    "proxy packet accounting differs from host wire evidence at index {index}: {observation:?}"
                ));
            }
            if index >= forwarded_count {
                dropped_packets.insert((*request_id, *packet_index));
            }
        }
    }
    let mut packet_indexes = BTreeMap::<u64, usize>::new();
    let transmitted: Vec<_> = evidence
        .trace
        .wire
        .iter()
        .filter_map(|wire| {
            let dropped = wire.request_id.is_some_and(|request_id| {
                let packet_index = packet_indexes.entry(request_id).or_default();
                *packet_index += 1;
                dropped_packets.contains(&(request_id, *packet_index))
            });
            (relevant(wire.opcode) && !dropped).then_some(wire.opcode)
        })
        .collect();
    let decoded: Vec<_> = packets
        .iter()
        .filter(|packet| relevant(packet.opcode))
        .map(|packet| packet.opcode)
        .collect();
    if transmitted.is_empty() || transmitted != decoded {
        return Err(format!(
            "Engine T decoded packet order differs from the encrypted Client stream after proxy byte drops: transmitted={transmitted:?} decoded={decoded:?}"
        ));
    }
    let mut accepted_by_engine_tick = BTreeMap::<u64, u32>::new();
    for packet in packets.iter().filter(|packet| packet.charged_user_event) {
        *accepted_by_engine_tick.entry(packet.tick).or_default() += 1;
    }
    println!(
        "TICK-MANIP server_account={} accepted_user_events_by_engine_tick={accepted_by_engine_tick:?}; engine ticks and Client observation ticks are distinct clocks; host dispatch receipts are not server acknowledgements",
        evidence.account
    );
    if evidence.pre_batch_user_events.requested > 0 {
        assert_deferred_terminal_row(evidence, packets)?;
    }
    if evidence.case == "tick-stale-menu" {
        let user_events = guard_carrier_user_events(evidence)?;
        assert_engine_stale_guard(packets)?;
        println!("TICK-MANIP stale-menu carrier: ordinary opener plus unchanged native prefix=3 wire=4; total user events={user_events}");
    }
    if matches!(
        evidence.case,
        "tick-make-without-click" | "tick-make-proxy-drop"
    ) {
        assert_engine_accepted_make_prefix(packets)?;
    }
    if evidence.case == "tick-npc-knife" {
        assert_engine_npc_knife_sequence(packets, &evidence.account)?;
    }
    Ok(())
}

fn correlate_dropped_cipher_suffix(
    evidence: &RunEvidence,
    packets: &[EnginePacket],
) -> Result<(), String> {
    let observations = evidence
        .transport
        .iter()
        .filter(|observation| observation.dropped_bytes > 0)
        .count();
    if observations != 1 {
        return Err(format!(
            "Make suffix-drop case needs exactly one dropped proxy observation, got {}",
            observations
        ));
    }
    let observation = evidence
        .transport
        .iter()
        .find(|observation| observation.dropped_bytes > 0)
        .expect("one dropped proxy observation was counted");
    validate_proxy_drop_after_make(&evidence.trace, &evidence.transport, observation.request_id)?;
    assert_engine_accepted_make_prefix(packets)?;
    let make_index = packets
        .iter()
        .position(|packet| packet.opcode == ClientProt289::OPHELDU.id as u8)
        .and_then(|opener_index| {
            packets
                .iter()
                .enumerate()
                .skip(opener_index + 1)
                .find(|(_, packet)| packet.opcode == ClientProt289::IF_BUTTON.id as u8)
                .map(|(index, _)| index)
        })
        .ok_or("Engine T accepted-Make prefix has no Make packet")?;
    let make = &packets[make_index];
    let unknown = packets
        .get(make_index + 1)
        .ok_or("Engine T did not record the post-drop ciphertext as an unknown opcode")?;
    if unknown.sequence <= make.sequence
        || unknown.tick < make.tick
        || unknown.name != "UNKNOWN"
        || unknown.outcome != "refused"
        || unknown.reason != "unknown-opcode"
        || unknown.handler_accepted
        || unknown.charged_user_event
    {
        return Err(format!(
            "Engine T did not close the cipher-desynchronized stream at the first post-Make packet: make={make:?} next={unknown:?}"
        ));
    }
    println!(
        "TICK-MANIP Engine T accepted the opener and Make, then closed the dropped-cipher stream at unknown-opcode sequence={}",
        unknown.sequence
    );
    validate_drop_relogin_evidence(evidence, observation.request_id)?;
    Ok(())
}

fn validate_proxy_drop_after_make(
    trace: &TickEngineTrace,
    observations: &[ProxyObservation],
    request_id: u64,
) -> Result<(), String> {
    let Some(receipt) = trace
        .dispatches
        .iter()
        .find(|dispatch| dispatch.receipt.request_id == request_id)
        .map(|dispatch| dispatch.receipt)
    else {
        return Err("Make suffix drop has no full native host receipt".into());
    };
    if receipt.row_count != 3
        || receipt.dispatched_prefix != 3
        || receipt.wire_events != 4
        || receipt.terminal.is_some()
        || !matches!(
            receipt.rows[0],
            interact::server_ordered::RowDispatch::HostQueued { wire_events: 1 }
        )
        || !matches!(
            receipt.rows[1],
            interact::server_ordered::RowDispatch::HostQueued { wire_events: 1 }
        )
        || !matches!(
            receipt.rows[2],
            interact::server_ordered::RowDispatch::HostQueued { wire_events: 2 }
        )
    {
        return Err(format!(
            "Make suffix drop requires the full native prefix=3/wire=4 host receipt: {receipt:?}"
        ));
    }
    let Some(observation) = observations.first() else {
        return Err("Make suffix drop has no completed proxy observation".into());
    };
    let wires = wires_for_request(trace, request_id);
    let packet_lengths: Vec<_> = wires.iter().map(|wire| wire.encoded_length).collect();
    let packet_opcodes: Vec<_> = wires.iter().map(|wire| wire.opcode).collect();
    let expected_packets: Vec<_> = (1..=wires.len())
        .map(|packet_index| (request_id, packet_index))
        .collect();
    let forwarded_bytes: usize = packet_lengths.iter().take(2).sum();
    let dropped_bytes: usize = packet_lengths.iter().skip(2).sum();
    if observations.len() != 1
        || observation.request_id != request_id
        || observation.request_ids != vec![request_id]
        || observation.mutation != "DropSuffixAfterPacket(2)"
        || packet_opcodes.as_slice()
            != [
                ClientProt289::OPHELDU.id as u8,
                ClientProt289::IF_BUTTON.id as u8,
                ClientProt289::MOVE_OPCLICK.id as u8,
                ClientProt289::OPLOC1.id as u8,
            ]
        || observation.packet_requests != expected_packets
        || observation.packet_opcodes != packet_opcodes
        || observation.packet_lengths != packet_lengths
        || observation.forwarded_bytes != forwarded_bytes
        || observation.dropped_bytes != dropped_bytes
        || observation.forwarded_bytes + observation.dropped_bytes
            != packet_lengths.iter().sum::<usize>()
    {
        return Err(format!(
            "proxy did not forward only the two-packet Make prefix and drop its byte suffix: {observation:?}"
        ));
    }
    Ok(())
}

fn engine_accepted_user_event(packet: &EnginePacket) -> bool {
    // A charged packet may suspend its content script while the network handler
    // accepts it. Content success still needs the case's real animation/chat oracle.
    packet.handler_accepted && packet.charged_user_event
}

fn assert_engine_accepted_make_prefix(packets: &[EnginePacket]) -> Result<(), String> {
    let Some(opener_index) = packets
        .iter()
        .position(|packet| packet.opcode == ClientProt289::OPHELDU.id as u8)
    else {
        return Err("Engine T log has no Knife opener for accepted-Make case".into());
    };
    let opener = &packets[opener_index];
    let make = packets
        .iter()
        .skip(opener_index + 1)
        .find(|packet| packet.opcode == ClientProt289::IF_BUTTON.id as u8);
    if !make.is_some_and(|make| {
        make.tick == opener.tick
            && make.sequence > opener.sequence
            && engine_accepted_user_event(opener)
            && engine_accepted_user_event(make)
    }) {
        return Err(format!(
            "Engine T did not accept the Make prefix in one tick: opener={opener:?} make={make:?}"
        ));
    }
    println!("TICK-MANIP Engine T accepted Knife opener and Make in one engine tick");
    Ok(())
}

fn assert_engine_npc_knife_sequence(packets: &[EnginePacket], account: &str) -> Result<(), String> {
    if !account.starts_with("tmx") {
        return Err(format!(
            "NPC Knife Engine T correlation requires the minted tmx account, got {account}"
        ));
    }
    let relevant: Vec<_> = packets
        .iter()
        .filter(|packet| NPC_KNIFE_WIRE_OPCODES.contains(&packet.opcode))
        .collect();
    for rows in relevant.windows(NPC_KNIFE_WIRE_OPCODES.len()) {
        if rows
            .iter()
            .map(|packet| packet.opcode)
            .eq(NPC_KNIFE_WIRE_OPCODES)
            && rows
                .iter()
                .all(|packet| packet.username == account && packet.handler_accepted)
            && engine_accepted_user_event(rows[0])
            && engine_accepted_user_event(rows[1])
            && rows
                .windows(2)
                .all(|pair| pair[0].sequence < pair[1].sequence)
        {
            return Ok(());
        }
    }
    Err(format!(
        "Engine T did not accept the ordered NPC Knife action for {account}: {packets:?}"
    ))
}

fn assert_deferred_terminal_row(
    evidence: &RunEvidence,
    packets: &[EnginePacket],
) -> Result<(), String> {
    if evidence.pre_batch_user_events.requested != 3 {
        return Err(format!(
            "deferred terminal-row case expected three foreign events, requested {}",
            evidence.pre_batch_user_events.requested
        ));
    }
    // The rate limit stops decoding after five charged user events. The
    // untouched terminal bytes are decoded on the next tick, not logged as
    // fictitious deferred packets in the first tick.
    let relevant: Vec<_> = packets
        .iter()
        .filter(|packet| {
            [
                ClientProt289::MOVE_GAMECLICK.id as u8,
                ClientProt289::OPHELDU.id as u8,
                ClientProt289::IF_BUTTON.id as u8,
                ClientProt289::MOVE_OPCLICK.id as u8,
                ClientProt289::OPLOC1.id as u8,
            ]
            .contains(&packet.opcode)
        })
        .collect();
    let expected = [
        ClientProt289::MOVE_GAMECLICK.id as u8,
        ClientProt289::MOVE_GAMECLICK.id as u8,
        ClientProt289::MOVE_GAMECLICK.id as u8,
        ClientProt289::OPHELDU.id as u8,
        ClientProt289::IF_BUTTON.id as u8,
        ClientProt289::MOVE_OPCLICK.id as u8,
        ClientProt289::OPLOC1.id as u8,
    ];
    for rows in relevant.windows(expected.len()) {
        let tick = rows[0].tick;
        if rows.iter().map(|packet| packet.opcode).ne(expected)
            || !rows.iter().all(|packet| engine_accepted_user_event(packet))
            || !rows[..5].iter().all(|packet| packet.tick == tick)
            || !rows[5..].iter().all(|packet| packet.tick == tick + 1)
            || !rows
                .windows(2)
                .all(|pair| pair[0].sequence < pair[1].sequence)
            || packets
                .iter()
                .filter(|packet| packet.tick == tick && packet.charged_user_event)
                .count()
                != 5
        {
            continue;
        }
        println!(
            "TICK-MANIP Engine T tick={tick}: three foreign moves and Knife/Make charged; original terminal MOVE_OPCLICK/OPLOC1 decoded and accepted at tick={}",
            tick + 1
        );
        return Ok(());
    }
    Err(format!(
        "Engine T log has no five-event prefix followed by its terminal pair decoded on the next tick: {packets:?}"
    ))
}

fn evidence_json(evidence: &RunEvidence) -> Value {
    let frame_json = |frame: &FrameObservation| {
        let chat = frame
            .chat
            .iter()
            .map(|line| {
                json!({
                    "game_tick": line.game_tick,
                    "sequence": line.sequence,
                    "text": line.text,
                    "poll_kind": match line.poll {
                        PollKind::Edge => "edge",
                        PollKind::Wake => "wake",
                    },
                })
            })
            .collect::<Vec<_>>();
        let inventory = frame
            .inventory_counts
            .iter()
            .map(|(id, count)| json!({"id": id, "count": count}))
            .collect::<Vec<_>>();
        let products = evidence
            .product_ids
            .iter()
            .map(|id| {
                json!({
                    "id": id,
                    "count": frame
                        .inventory_counts
                        .iter()
                        .find(|(item_id, _)| item_id == id)
                        .map_or(0, |(_, count)| *count),
                })
            })
            .collect::<Vec<_>>();
        let skill_xp = frame
            .skill_xp
            .iter()
            .map(|(index, xp)| json!({"skill_index": index, "xp": xp}))
            .collect::<Vec<_>>();
        json!({
            "host_frame": frame.host_frame,
            "game_tick": frame.game_tick,
            "client_snapshot_tick": frame.client_tick,
            "tick_edge": frame.tick_edge,
            "snapshot": {
                "tile": frame.player_tile.map(|tile| {
                    json!({"x": tile.x, "z": tile.z, "level": tile.level})
                }),
                "animation": frame.animation.map(|animation| {
                    json!({"sequence": animation.sequence, "serial": animation.serial})
                }),
                "chat_lines": chat,
                "inventory_counts": inventory,
                "product_counts": products,
                "log_count": frame.log_count,
                "skill_xp": skill_xp,
                "chat_modal_root": frame.chat_modal_root,
                "actual_chat_modal_root": frame.actual_chat_modal_root,
                "server_if_close_packets": frame.server_if_close_packets,
                "iface_generation": frame.iface_generation,
                "main_modal_opened_observation": frame.main_modal_opened_observation,
                "main_modal_closed_observation": frame.main_modal_closed_observation,
                "suppressed_openchat": frame.suppressed_openchat,
            },
        })
    };
    let frames = evidence
        .trace
        .frames
        .iter()
        .map(&frame_json)
        .collect::<Vec<_>>();
    let baseline = evidence.baseline.as_ref().map(frame_json);
    let dispatches = evidence
        .trace
        .dispatches
        .iter()
        .map(|dispatch| {
            let receipt = dispatch.receipt;
            json!({
                "native_dispatch_tick": dispatch.game_tick,
                "request_id": receipt.request_id,
                "dispatch_tick": receipt.dispatch_tick,
                "row_count": receipt.row_count,
                "dispatched_prefix": receipt.dispatched_prefix,
                "wire_events": receipt.wire_events,
                "terminal": receipt.terminal.map(|terminal| format!("{terminal:?}")),
                "rows": receipt.rows.iter().map(|row| format!("{row:?}")).collect::<Vec<_>>(),
                "request_debug": dispatch.request_debug,
                "mutation_applied": dispatch.mutation_applied,
            })
        })
        .collect::<Vec<_>>();
    let wire = evidence
        .trace
        .wire
        .iter()
        .map(|packet| {
            json!({
                "host_frame": packet.host_frame,
                "game_tick": packet.game_tick,
                "request_id": packet.request_id,
                "opcode": packet.opcode,
                "encoded_length": packet.encoded_length,
            })
        })
        .collect::<Vec<_>>();
    let statuses = evidence
        .statuses
        .iter()
        .map(|status| {
            json!({
                "host_frame": status.host_frame,
                "game_tick": status.game_tick,
                "phase": status.phase,
                "method": status.method,
                "tick_method": status.tick_method,
                "target": status.target,
                "last_event": status.last_event,
                "tick_cycle": status.tick_cycle,
                "tick_verified": status.tick_verified,
                "tick_unobserved": status.tick_unobserved,
                "tick_slips": status.tick_slips,
                "tick_lost": status.tick_lost,
                "tick_delayed": status.tick_delayed,
                "tick_paused": status.tick_paused,
                "tick_fallbacks": status.tick_fallbacks,
                "tick_fallback_failed": status.tick_fallback_failed,
                "tick_events": status.tick_events,
            })
        })
        .collect::<Vec<_>>();
    let transport = evidence
        .transport
        .iter()
        .map(|observation| {
            json!({
                "request_id": observation.request_id,
                "request_ids": observation.request_ids,
                "mutation": observation.mutation,
                "start_game_tick": observation.start_game_tick,
                "start_host_frame": observation.start_host_frame,
                "packet_requests": observation.packet_requests,
                "packet_opcodes": observation.packet_opcodes,
                "packet_lengths": observation.packet_lengths,
                "forwarded_bytes": observation.forwarded_bytes,
                "dropped_bytes": observation.dropped_bytes,
                "release_edge_tick": observation.release_edge_tick,
                "release_trigger_tick": observation.release_trigger_tick,
                "release_edge_frame": observation.release_edge_frame,
                "released_host_frame": observation.released_host_frame,
            })
        })
        .collect::<Vec<_>>();
    let engine_packets = match &evidence.engine_packets {
        Some(Ok(packets)) => json!({
            "status": "captured",
            "packets": packets.iter().map(|packet| {
                json!({
                    "sequence": packet.sequence,
                    "tick": packet.tick,
                    "username": packet.username,
                    "opcode": packet.opcode,
                    "name": packet.name,
                    "outcome": packet.outcome,
                    "reason": packet.reason,
                    "handler_accepted": packet.handler_accepted,
                    "charged_user_event": packet.charged_user_event,
                    "scripts": packet.scripts.iter().map(|script| {
                        json!({
                            "outcome": script.outcome,
                            "reason": script.reason,
                            "script": script.script,
                            "state": script.state,
                        })
                    }).collect::<Vec<_>>(),
                })
            }).collect::<Vec<_>>(),
        }),
        Some(Err(error)) => json!({"status": "error", "error": error}),
        None => json!({"status": "not_armed"}),
    };
    let drop_history = evidence
        .drop_history
        .as_ref()
        .map_or(Value::Null, drop_history::DropHistory::json);
    json!({
        "case": evidence.case,
        "account": evidence.account,
        "profile": evidence.profile,
        "world_members": evidence.world_members,
        "pre_batch_user_events": {
            "requested": evidence.pre_batch_user_events.requested,
            "armed": evidence.pre_batch_user_events.armed,
            "sent": evidence.pre_batch_user_events.sent,
            "emitted": evidence.pre_batch_user_events.emitted,
            "client_tick": evidence.pre_batch_user_events.client_tick,
            "host_frame": evidence.pre_batch_user_events.host_frame,
            "error": evidence.pre_batch_user_events.error,
        },
        "pre_batch_guard_opener": {
            "armed": evidence.pre_batch_guard_opener.armed,
            "sent": evidence.pre_batch_guard_opener.sent,
            "client_tick": evidence.pre_batch_guard_opener.client_tick,
            "host_frame": evidence.pre_batch_guard_opener.host_frame,
            "encoded_length": evidence.pre_batch_guard_opener.encoded_length,
            "error": evidence.pre_batch_guard_opener.error,
            "total_user_events_with_first_native_batch": guard_carrier_user_events(evidence).ok(),
        },
        "start_tick": evidence.start_tick,
        "product_ids": evidence.product_ids,
        "baseline_snapshot": baseline,
        "observed_npc_references": evidence.npc_observations.iter()
            .map(|reference| format!("{reference:?}")).collect::<Vec<_>>(),
        "frames": frames,
        "dispatch_receipts": dispatches,
        "encrypted_client_wire": wire,
        "native_status_history": statuses,
        "proxy_transport_history": transport,
        "engine_packet_history": engine_packets,
        "drop_lifecycle_history": drop_history,
        "interpretation": {
            "host_dispatch_receipts_are_not_server_acknowledgements": true,
            "client_game_ticks_and_engine_ticks_are_distinct_clocks": true,
        },
    })
}

fn persist_evidence(evidence: &RunEvidence) -> Result<PathBuf, String> {
    let root = required_path("LIVE_EVIDENCE_DIR")?;
    let epoch = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("evidence clock: {error}"))?
        .as_nanos();
    let directory = root.join(format!("{}_{}", evidence.case, epoch));
    std::fs::create_dir_all(&directory)
        .map_err(|error| format!("create evidence directory {}: {error}", directory.display()))?;
    let path = directory.join("tick_engine.json");
    let bytes = serde_json::to_vec_pretty(&evidence_json(evidence))
        .map_err(|error| format!("serialize evidence JSON: {error}"))?;
    std::fs::write(&path, bytes)
        .map_err(|error| format!("write evidence {}: {error}", path.display()))?;
    println!("TICK-MANIP evidence_path={}", path.display());
    Ok(path)
}

fn require_live_config() -> Result<(), String> {
    if std::env::var("LIVE").as_deref() != Ok("1") {
        return Err(
            "tick-engine live tests require LIVE=1; opted-in cases never silently skip".into(),
        );
    }
    let prefix = std::env::var("BOT_LIVE_NAME_PREFIX")
        .map_err(|_| "BOT_LIVE_NAME_PREFIX=tmx is required for Engine T accounts".to_owned())?;
    if prefix != "tmx" {
        return Err("S8 is assigned Engine T and requires BOT_LIVE_NAME_PREFIX=tmx".into());
    }
    let evidence_dir = required_path("LIVE_EVIDENCE_DIR")?;
    if evidence_dir.exists() && !evidence_dir.is_dir() {
        return Err(format!(
            "LIVE_EVIDENCE_DIR is not a directory: {}",
            evidence_dir.display()
        ));
    }
    Ok(())
}

fn required_file(name: &str) -> Result<PathBuf, String> {
    let path = required_path(name)?;
    if !path.is_file() {
        return Err(format!("{name} is not a file: {}", path.display()));
    }
    Ok(path)
}

fn required_dir(name: &str) -> Result<PathBuf, String> {
    let path = required_path(name)?;
    if !path.is_dir() {
        return Err(format!("{name} is not a directory: {}", path.display()));
    }
    Ok(path)
}

fn required_path(name: &str) -> Result<PathBuf, String> {
    let value = std::env::var(name)
        .map_err(|_| format!("{name} is required for tick-engine live qualification"))?;
    let path = PathBuf::from(value);
    if !path.is_absolute() {
        return Err(format!("{name} must be absolute: {}", path.display()));
    }
    Ok(path)
}

fn mining_start() -> Result<WorldTile, String> {
    let default = tile(3_284, 3_368, 0);
    match std::env::var("GATHERER_MINE_TILE") {
        Ok(raw) => parse_tile("GATHERER_MINE_TILE", &raw),
        Err(std::env::VarError::NotPresent) => Ok(default),
        Err(error) => Err(format!("GATHERER_MINE_TILE is not valid UTF-8: {error}")),
    }
}

fn parse_tile(name: &str, raw: &str) -> Result<WorldTile, String> {
    let mut parts = raw.split(',').map(str::trim);
    let x = parts
        .next()
        .ok_or_else(|| format!("{name} must be x,z,level"))?
        .parse::<i32>()
        .map_err(|error| format!("{name} has invalid x: {error}"))?;
    let z = parts
        .next()
        .ok_or_else(|| format!("{name} must be x,z,level"))?
        .parse::<i32>()
        .map_err(|error| format!("{name} has invalid z: {error}"))?;
    let level = parts
        .next()
        .ok_or_else(|| format!("{name} must be x,z,level"))?
        .parse::<i32>()
        .map_err(|error| format!("{name} has invalid level: {error}"))?;
    if parts.next().is_some() {
        return Err(format!("{name} must be x,z,level"));
    }
    Ok(tile(x, z, level))
}

fn tile(x: i32, z: i32, level: i32) -> WorldTile {
    WorldTile { x, z, level }
}

fn skill_xp(frame: &FrameObservation, skill_index: i32) -> Option<i32> {
    frame
        .skill_xp
        .iter()
        .find(|(index, _)| *index == skill_index)
        .map(|(_, xp)| *xp)
}

fn inventory_product_count(frame: &FrameObservation, product_ids: &[i32]) -> i32 {
    frame
        .inventory_counts
        .iter()
        .filter(|(id, _)| product_ids.contains(id))
        .map(|(_, count)| *count)
        .sum()
}

fn fishing_gain_observed(
    baseline: &FrameObservation,
    observed: &FrameObservation,
    product_ids: &[i32],
    skill_index: i32,
) -> bool {
    let fish_gained = inventory_product_count(observed, product_ids)
        > inventory_product_count(baseline, product_ids);
    let xp_gained = skill_xp(baseline, skill_index)
        .zip(skill_xp(observed, skill_index))
        .is_some_and(|(before, after)| after > before);
    fish_gained || xp_gained
}

fn is_knife_arm(dispatch: &DispatchObservation) -> bool {
    dispatch
        .request_debug
        .as_deref()
        .is_some_and(|request| request.contains("KnifeArm"))
}

fn first_knife(trace: &TickEngineTrace) -> Result<DispatchObservation, String> {
    trace
        .dispatches
        .iter()
        .find(|dispatch| is_knife_arm(dispatch))
        .cloned()
        .ok_or_else(|| "no real KnifeArm receipt in the trace".to_owned())
}

fn knife_dispatches(trace: &TickEngineTrace) -> Vec<DispatchObservation> {
    trace
        .dispatches
        .iter()
        .filter(|dispatch| is_knife_arm(dispatch))
        .cloned()
        .collect()
}

fn wires_for_request(trace: &TickEngineTrace, request_id: u64) -> Vec<WireObservation> {
    trace
        .wire
        .iter()
        .filter(|wire| wire.request_id == Some(request_id))
        .copied()
        .collect()
}

fn request_wire_opcodes(trace: &TickEngineTrace, dispatch: &DispatchObservation) -> Vec<u8> {
    wires_for_request(trace, dispatch.receipt.request_id)
        .into_iter()
        .map(|wire| wire.opcode)
        .collect()
}

fn npc_freshfish_knife_dispatch_matches(
    dispatch: &DispatchObservation,
    observed: &[interact::server_ordered::ExactNpcReference],
) -> bool {
    let exact_request = dispatch.request_debug.as_deref().is_some_and(|request| {
        request.contains("KnifeArm {")
            && request.contains("Continuing {")
            && observed
                .iter()
                .any(|reference| request.contains(&format!("Npc({reference:?})")))
            && !request.contains("Loc(")
    });
    let receipt = dispatch.receipt;
    exact_request
        && !dispatch.mutation_applied
        && receipt.row_count == 3
        && receipt.dispatched_prefix == 3
        && receipt.wire_events == 4
        && receipt.terminal.is_none()
        && matches!(
            receipt.rows[0],
            interact::server_ordered::RowDispatch::HostQueued { wire_events: 1 }
        )
        && matches!(
            receipt.rows[1],
            interact::server_ordered::RowDispatch::HostQueued { wire_events: 1 }
        )
        && matches!(
            receipt.rows[2],
            interact::server_ordered::RowDispatch::HostQueued { wire_events: 2 }
        )
}

fn npc_knife_wire_order_matches(trace: &TickEngineTrace, dispatch: &DispatchObservation) -> bool {
    request_wire_opcodes(trace, dispatch) == NPC_KNIFE_WIRE_OPCODES
}

fn checked_click_pair_completions(
    trace: &TickEngineTrace,
) -> impl Iterator<Item = &WireObservation> {
    // Ordinary Interact completions are not ServerOrderedDispatch receipts.
    // Their two real unowned Client packets must share one native tick and
    // host frame. Engine T correlation separately establishes acceptance.
    let mut previous: Option<&WireObservation> = None;
    trace
        .wire
        .iter()
        .filter(|wire| wire.request_id.is_none())
        .filter(move |&wire| {
            let completes_pair = previous.is_some_and(|movement| {
                movement.host_frame == wire.host_frame
                    && movement.game_tick == wire.game_tick
                    && movement.opcode == ClientProt289::MOVE_OPCLICK.id as u8
                    && (wire.opcode == ClientProt289::OPLOC1.id as u8
                        || wire.opcode == ClientProt289::OPNPC1.id as u8)
            });
            previous = Some(wire);
            completes_pair
        })
}

fn checked_click_pairs(trace: &TickEngineTrace, tick: u64) -> usize {
    checked_click_pair_completions(trace)
        .filter(|wire| u64::from(wire.game_tick) == tick)
        .count()
}

fn frames_in_tick(trace: &TickEngineTrace, tick: u64) -> Vec<&FrameObservation> {
    trace
        .frames
        .iter()
        .filter(|frame| u64::from(frame.game_tick) == tick)
        .collect()
}

fn edge_frame(trace: &TickEngineTrace, tick: u64) -> Option<&FrameObservation> {
    trace
        .frames
        .iter()
        .find(|frame| u64::from(frame.game_tick) == tick && frame.tick_edge)
}

fn fresh_animation_on_edge(trace: &TickEngineTrace, tick: u64) -> Option<(i32, u32)> {
    let edge = edge_frame(trace, tick)?;
    let previous = trace
        .frames
        .iter()
        .filter(|frame| frame.host_frame < edge.host_frame)
        .max_by_key(|frame| frame.host_frame)
        .and_then(|frame| frame.animation);
    let current = edge.animation?;
    if previous.is_some_and(|previous| previous.serial == current.serial) {
        return None;
    }
    Some((current.sequence, current.serial))
}

fn no_fresh_animation_on_edge(trace: &TickEngineTrace, tick: u64) -> bool {
    let Some(edge) = edge_frame(trace, tick) else {
        return false;
    };
    let previous = trace
        .frames
        .iter()
        .filter(|frame| frame.host_frame < edge.host_frame)
        .max_by_key(|frame| frame.host_frame)
        .and_then(|frame| frame.animation);
    match (previous, edge.animation) {
        (None, None) => true,
        (Some(previous), Some(current)) => previous.serial == current.serial,
        (None, Some(_)) | (Some(_), None) => false,
    }
}

fn has_chat_in_tick(trace: &TickEngineTrace, tick: u64, predicate: impl Fn(&str) -> bool) -> bool {
    has_chat_in_range(trace, tick, tick, predicate)
}

fn has_chat_in_range(
    trace: &TickEngineTrace,
    from_tick: u64,
    through_tick: u64,
    predicate: impl Fn(&str) -> bool,
) -> bool {
    trace
        .frames
        .iter()
        .flat_map(|frame| &frame.chat)
        .any(|line| {
            // Chat precedes PLAYER_INFO: an edge bounds it to [t, t + 1],
            // while a wake attributes it exactly to t + 1 (facts r5 §1.5).
            let observed = u64::from(line.game_tick);
            let first = line.first_attributable_tick();
            let last = observed + 1;
            first <= through_tick && last >= from_tick && predicate(&line.text)
        })
}

fn has_act_line(trace: &TickEngineTrace, tick: u64) -> bool {
    has_chat_in_tick(trace, tick, |line| {
        line.to_ascii_lowercase()
            .contains("you attempt to catch a fish")
    })
}

fn has_fletching_members_line(trace: &TickEngineTrace, from_tick: u64, through_tick: u64) -> bool {
    has_chat_in_range(trace, from_tick, through_tick, |line| {
        line.trim().eq_ignore_ascii_case(
            "You need to be on a members' world to gain experience in Fletching.",
        )
    })
}

fn has_copper_success(trace: &TickEngineTrace, from_tick: u64, through_tick: u64) -> bool {
    has_chat_in_range(trace, from_tick, through_tick, |line| {
        let text = line.to_ascii_lowercase();
        text.contains("copper") && text.contains("mine")
    })
}

fn edge_has_openchat(trace: &TickEngineTrace, tick: u64) -> bool {
    frames_in_tick(trace, tick)
        .into_iter()
        .any(|frame| frame.actual_chat_modal_root == 8_880)
}

fn close_packet_seen(trace: &TickEngineTrace, tick: u64, baseline: &FrameObservation) -> bool {
    let mut previous = baseline.server_if_close_packets;
    trace.frames.iter().any(|frame| {
        let fresh = frame.server_if_close_packets > previous;
        previous = frame.server_if_close_packets;
        let observed = u64::from(frame.game_tick);
        // IF_CLOSE follows PLAYER_INFO: a wake is exact, an edge is [t - 1, t].
        fresh
            && frame.server_if_close_packets > baseline.server_if_close_packets
            && (tick == observed || frame.tick_edge && observed.checked_sub(1) == Some(tick))
    })
}

fn iron_roll_on_edge(trace: &TickEngineTrace, tick: u64) -> bool {
    fresh_animation_on_edge(trace, tick).is_some_and(|(sequence, _)| {
        sequence == MITHRIL_PICK_ANIMATION
            || sequence == -1
                && has_chat_in_tick(trace, tick, |line| {
                    line.contains("You manage to mine some iron.")
                })
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum IronPrecondition {
    Waiting {
        opportunities: usize,
    },
    Selected {
        request_id: u64,
        roll_tick: u64,
        opportunities: usize,
    },
    NoEligibleFailure {
        opportunities: usize,
    },
    GuardExhausted {
        opportunities: usize,
    },
    SelectionMissing {
        roll_tick: u64,
        opportunities: usize,
    },
}

fn iron_precondition(
    trace: &TickEngineTrace,
    selected_request: Option<u64>,
    start: u32,
    current: u32,
) -> IronPrecondition {
    let mut opportunities = 0;
    let mut previous_tick = None;
    for frame in &trace.frames {
        let tick = u64::from(frame.game_tick);
        if !frame.tick_edge
            || frame.game_tick < start
            || frame.game_tick.saturating_sub(start) > NATURAL_IRON_ROLL_BUDGET
            || previous_tick == Some(tick)
            || !iron_roll_on_edge(trace, tick)
            || has_chat_in_tick(trace, tick, |line| {
                line.contains("You swing your pick at the rock.")
            })
        {
            continue;
        }
        previous_tick = Some(tick);
        opportunities += 1;
        let success = has_chat_in_tick(trace, tick, |line| {
            line.contains("You manage to mine some iron.")
        });
        if !success {
            if let Some(dispatch) = trace.dispatches.iter().find(|dispatch| {
                Some(dispatch.receipt.request_id) == selected_request && dispatch.game_tick == tick
            }) {
                return IronPrecondition::Selected {
                    request_id: dispatch.receipt.request_id,
                    roll_tick: tick,
                    opportunities,
                };
            }
            // The receipt may still be in flight in this same host window.
            if u64::from(current) > tick {
                return IronPrecondition::SelectionMissing {
                    roll_tick: tick,
                    opportunities,
                };
            }
            return IronPrecondition::Waiting { opportunities };
        }
        if opportunities == IRON_ROLL_OPPORTUNITY_LIMIT {
            return IronPrecondition::NoEligibleFailure { opportunities };
        }
    }
    if current.saturating_sub(start) >= NATURAL_IRON_ROLL_BUDGET {
        IronPrecondition::GuardExhausted { opportunities }
    } else {
        IronPrecondition::Waiting { opportunities }
    }
}

fn validate_late_iron_history(
    trace: &TickEngineTrace,
    statuses: &[StatusObservation],
    origin: u64,
) -> Result<(), &'static str> {
    if !fresh_animation_on_edge(trace, origin + 2).is_some_and(|(sequence, _)| {
        sequence == MITHRIL_PICK_ANIMATION || sequence == FLETCHING_ANIMATION
    }) || !has_chat_in_tick(trace, origin + 2, |line| {
        line.contains("You swing your pick at the rock.")
    }) || checked_click_pairs(trace, origin + 2) != 1
        || !iron_roll_on_edge(trace, origin + 5)
    {
        return Err("late-acceptance failed: missing R+2 act/click or real R+5 iron roll");
    }
    let before = statuses
        .iter()
        .rev()
        .find(|status| u64::from(status.game_tick) <= origin + 1)
        .ok_or("late-acceptance failed: missing pre-landing status")?;
    let after = statuses
        .last()
        .ok_or("late-acceptance failed: missing final status")?;
    if after.tick_slips != before.tick_slips.map(|count| count + 1)
        || after.tick_verified != before.tick_verified
        || after.tick_unobserved != before.tick_unobserved
        || after.tick_cycle != before.tick_cycle
    {
        return Err("late-acceptance failed: expected one Slip and unchanged prior cycle totals");
    }
    if knife_dispatches(trace)
        .iter()
        .any(|dispatch| dispatch.game_tick > origin && dispatch.game_tick < origin + 5)
    {
        return Err("late-acceptance failed: second batch before R+5");
    }
    if trace.frames.first().is_some_and(|baseline| {
        trace
            .frames
            .iter()
            .any(|frame| frame.log_count != baseline.log_count)
    }) {
        return Err("late-acceptance failed: raw log count changed");
    }
    Ok(())
}

fn iface_moved_in_window(trace: &TickEngineTrace, tick: u64, baseline: &FrameObservation) -> bool {
    let mut previous = baseline.iface_generation;
    trace.frames.iter().any(|frame| {
        let fresh = frame.iface_generation != previous;
        previous = frame.iface_generation;
        let observed = u64::from(frame.game_tick);
        // facts-r5:137: interface packets occur on either side of PLAYER_INFO.
        fresh
            && if frame.tick_edge {
                observed.abs_diff(tick) <= 1
            } else {
                observed == tick || observed + 1 == tick
            }
    })
}

fn guard_carrier_user_events(evidence: &RunEvidence) -> Result<usize, String> {
    let opener = &evidence.pre_batch_guard_opener;
    let first = first_knife(&evidence.trace)?;
    let index = evidence
        .trace
        .wire
        .iter()
        .position(|wire| wire.request_id == Some(first.receipt.request_id))
        .ok_or("stale-menu carrier has no native packets")?;
    let start = index.checked_sub(1).ok_or("ordinary opener is absent")?;
    let carrier = evidence
        .trace
        .wire
        .get(start..index + 4)
        .ok_or("stale-menu native batch is incomplete")?;
    let expected = [
        ClientProt289::OPHELDU.id as u8,
        ClientProt289::OPHELDU.id as u8,
        ClientProt289::IF_BUTTON.id as u8,
        ClientProt289::MOVE_OPCLICK.id as u8,
        ClientProt289::OPLOC1.id as u8,
    ];
    if !opener.sent
        || opener.error.is_some()
        || opener.host_frame != Some(carrier[0].host_frame)
        || opener.encoded_length != Some(carrier[0].encoded_length)
        || evidence
            .trace
            .wire
            .iter()
            .filter(|wire| {
                wire.request_id.is_none() && wire.opcode == ClientProt289::OPHELDU.id as u8
            })
            .count()
            != 1
        || carrier[0].request_id.is_some()
        || carrier.iter().map(|wire| wire.opcode).ne(expected)
        || !carrier[1..].iter().all(|wire| {
            wire.request_id == Some(first.receipt.request_id)
                && wire.host_frame == carrier[0].host_frame
        })
        || first.receipt.dispatched_prefix != 3
        || first.receipt.wire_events != 4
        || first.receipt.terminal.is_some()
    {
        return Err(format!(
            "stale-menu carrier is not one ordinary opener followed by the unchanged full native batch: opener={opener:?} receipt={:?} wire={carrier:?}",
            first.receipt
        ));
    }
    Ok(carrier.len())
}

fn assert_engine_stale_guard(packets: &[EnginePacket]) -> Result<(), String> {
    let expected = [200, 200, 86, 67, 10];
    let relevant: Vec<_> = packets
        .iter()
        .filter(|packet| expected.contains(&packet.opcode))
        .collect();
    for rows in relevant.windows(expected.len()) {
        if rows.iter().map(|packet| packet.opcode).eq(expected)
            && rows.iter().all(|packet| packet.tick == rows[0].tick)
            && rows[..3]
                .iter()
                .all(|packet| engine_accepted_user_event(packet))
            && rows[3..]
                .iter()
                .all(|packet| !packet.handler_accepted && !packet.charged_user_event)
            && rows
                .windows(2)
                .all(|pair| pair[0].sequence < pair[1].sequence)
        {
            println!(
                "TICK-MANIP stale-menu guard Engine T tick={} (host R+1): two accepted openers, accepted Make, refused terminal pair; chat.rs2:830-834; Fletching must follow at R+2 and roll at R+4",
                rows[0].tick
            );
            return Ok(());
        }
    }
    Err(format!(
        "Engine T did not decode the five-event stale-menu guard carrier in one tick: {packets:?}"
    ))
}

fn stale_guard_landing(trace: &TickEngineTrace, origin: u64, baseline: &FrameObservation) -> bool {
    close_packet_seen(trace, origin + 1, baseline)
        && iface_moved_in_window(trace, origin + 1, baseline)
        && no_fresh_animation_on_edge(trace, origin + 1)
        && fresh_animation_on_edge(trace, origin + 2)
            .is_some_and(|(sequence, _)| sequence == FLETCHING_ANIMATION)
        && !has_act_line(trace, origin + 2)
        && checked_click_pairs(trace, origin + 2) == 1
        && fresh_animation_on_edge(trace, origin + 4)
            .is_some_and(|(sequence, _)| sequence == ROLL_ANIMATION)
        && knife_dispatches(trace)
            .iter()
            .filter(|dispatch| dispatch.game_tick < origin + 4)
            .count()
            == 1
        && trace
            .frames
            .iter()
            .all(|frame| frame.log_count == baseline.log_count)
        && !trace.wire.iter().any(|wire| {
            wire.opcode == ClientProt289::CLOSE_MODAL.id as u8
                || wire.opcode == ClientProt289::IF_BUTTON.id as u8 && wire.request_id.is_none()
        })
}

fn assert_batch(
    run: &LiveRun,
    dispatch: &DispatchObservation,
    prefix: u8,
    wire_events: u8,
) -> Result<(), String> {
    if dispatch.receipt.dispatched_prefix != prefix || dispatch.receipt.wire_events != wire_events {
        return Err(run.failure(format!(
            "KnifeArm receipt mismatch: expected prefix={prefix} wire={wire_events}; got {:?}",
            dispatch.receipt
        )));
    }
    Ok(())
}

fn assert_knife_wire_order(
    run: &LiveRun,
    trace: &TickEngineTrace,
    dispatch: &DispatchObservation,
    expected: &[u8],
) -> Result<(), String> {
    let observed = request_wire_opcodes(trace, dispatch);
    if observed != expected {
        return Err(run.failure(format!(
            "request {} encoded opcodes {observed:?}, expected {expected:?}",
            dispatch.receipt.request_id
        )));
    }
    Ok(())
}

fn assert_held_batch(
    run: &LiveRun,
    expected_request_id: u64,
    expected_opcodes: &[u8],
    expected_edge_tick: u32,
) -> Result<(), String> {
    let observations = run
        .proxy
        .as_ref()
        .map_or_else(Vec::new, |proxy| proxy.observations());
    let Some(observation) = observations.first() else {
        return Err(run.failure("held batch has no completed real proxy observation"));
    };
    let all_packet_bytes: usize = observation.packet_lengths.iter().sum();
    if observations.len() != 1
        || observation.request_id != expected_request_id
        || observation.request_ids != vec![expected_request_id]
        || observation.packet_opcodes != expected_opcodes
        || observation.release_edge_tick != Some(expected_edge_tick)
        || observation.release_trigger_tick.is_some()
        || observation
            .released_host_frame
            .zip(observation.release_edge_frame)
            .is_none_or(|(released, edge)| released <= edge)
        || observation.dropped_bytes != 0
        || observation.forwarded_bytes != all_packet_bytes
    {
        return Err(run.failure(format!(
            "real encrypted batch was not held through the requested edge: {observation:?}"
        )));
    }
    Ok(())
}

fn assert_proxy_drop_after_make(
    run: &mut LiveRun,
    request_id: u64,
    origin: u64,
) -> Result<(), String> {
    let proxy = run
        .proxy
        .as_ref()
        .map(Arc::clone)
        .ok_or_else(|| run.failure("suffix-drop case has no loopback proxy"))?;
    let deadline = Instant::now() + LOGOUT_DEADLINE;
    let mut after_host_frame = run.hook.observed_host_frame();
    loop {
        let trace = run.hook.trace();
        run.capture_status(&trace);
        proxy
            .ensure_healthy()
            .map_err(|error| run.failure(format!("suffix-drop proxy failed: {error}")))?;
        let observations = proxy.observations();
        if observations
            .iter()
            .any(|observation| observation.request_id == request_id)
        {
            validate_proxy_drop_after_make(&trace, &observations, request_id)
                .map_err(|error| run.failure(error))?;
            return assert_drop_relogin(run, request_id, origin, deadline);
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(run.failure(format!(
                "Make suffix drop had no completed proxy observation within {LOGOUT_DEADLINE:?}: trace={trace:?}"
            )));
        }
        after_host_frame = run
            .hook
            .wait_for_frame(after_host_frame, remaining.min(FRAME_HANG_GUARD))
            .ok_or_else(|| {
                run.failure(format!(
                    "host-frame hang guard expired while awaiting the Make suffix-drop proxy observation within {LOGOUT_DEADLINE:?}"
                ))
            })?;
    }
}

#[derive(Debug, Clone)]
struct DropReloginBaseline {
    batch_host_frame: u64,
    initial_session: u64,
    initial_slot: drop_history::SlotObservation,
    initial_verified: i64,
    baseline_host_frame: u64,
}

fn drop_relogin_baseline(
    history: &drop_history::DropHistory,
    trace: &TickEngineTrace,
    statuses: &[StatusObservation],
    request_id: u64,
    baseline_host_frame: u64,
) -> Result<DropReloginBaseline, String> {
    let batch_host_frame = wires_for_request(trace, request_id)
        .first()
        .map(|wire| wire.host_frame)
        .ok_or("suffix-drop request has no encrypted wire frames")?;
    let initial_session = history
        .client_sessions
        .iter()
        .filter(|observation| {
            observation.host_frame <= batch_host_frame
                && observation.ingame
                && observation.scene_state == 2
        })
        .max_by_key(|observation| observation.host_frame)
        .map(|observation| observation.session)
        .ok_or("suffix-drop request has no ready client-session baseline")?;
    let initial_slot = history
        .slot_status
        .iter()
        .filter(|observation| {
            observation.host_frame <= batch_host_frame
                && observation.connected
                && observation.ingame
                && observation.scene_state == 2
        })
        .max_by_key(|observation| observation.host_frame)
        .cloned()
        .ok_or("suffix-drop request has no ready slot-status baseline")?;
    let initial_verified = statuses
        .iter()
        .filter(|status| status.host_frame <= batch_host_frame)
        .max_by_key(|status| status.host_frame)
        .and_then(|status| status.tick_verified)
        .ok_or("suffix-drop request has no tick_verified baseline")?;
    Ok(DropReloginBaseline {
        batch_host_frame,
        initial_session,
        initial_slot,
        initial_verified,
        baseline_host_frame,
    })
}

fn suffix_drop_relogin_ready(
    history: &drop_history::DropHistory,
    trace: &TickEngineTrace,
    statuses: &[StatusObservation],
    baseline: &DropReloginBaseline,
    origin: u64,
) -> Result<Option<u64>, String> {
    let first_new_session_frame = history
        .client_sessions
        .iter()
        .filter(|observation| {
            observation.host_frame > baseline.batch_host_frame
                && observation.session != baseline.initial_session
        })
        .map(|observation| observation.host_frame)
        .min();
    let session_ready = history.client_sessions.iter().find(|observation| {
        observation.host_frame > baseline.batch_host_frame
            && observation.session != baseline.initial_session
            && observation.ingame
            && observation.scene_state == 2
    });
    let disconnect_boundary = history.slot_status.iter().any(|observation| {
        observation.host_frame > baseline.batch_host_frame && !observation.connected
    });
    let ready_slot = history.slot_status.iter().any(|observation| {
        observation.host_frame > baseline.batch_host_frame
            && observation.stream_epoch != baseline.initial_slot.stream_epoch
            && observation.connected
            && observation.ingame
            && observation.scene_state == 2
            && observation.startup_phase == host_play::StartupPhase::Ready
    });
    let read_eof = history.host_log.iter().position(|observation| {
        observation
            .message
            .starts_with("session exit reason=read_eof")
    });
    let reconnect_attempt = read_eof.and_then(|exit_index| {
        history
            .host_log
            .iter()
            .enumerate()
            .find(|(index, observation)| {
                *index > exit_index
                    && observation
                        .message
                        .starts_with("relog decision attempt reconnect=true")
            })
            .map(|(index, _)| index)
    });
    let handshake_after_reconnect = reconnect_attempt.is_some_and(|attempt_index| {
        history
            .host_log
            .iter()
            .enumerate()
            .any(|(index, observation)| {
                index > attempt_index && observation.message.starts_with("handshake ok")
            })
    });
    let old_session_verified_increment = statuses.iter().any(|status| {
        status.host_frame >= baseline.baseline_host_frame
            && first_new_session_frame
                .is_none_or(|new_session_frame| status.host_frame < new_session_frame)
            && status.tick_verified != Some(baseline.initial_verified)
    });
    let roll_tick = origin.wrapping_add(3) as u32;
    let old_session_roll = trace.frames.iter().any(|edge| {
        if edge.host_frame < baseline.baseline_host_frame
            || first_new_session_frame
                .is_some_and(|new_session_frame| edge.host_frame >= new_session_frame)
            || edge.game_tick != roll_tick
            || !edge.tick_edge
        {
            return false;
        }
        let Some(current) = edge.animation else {
            return false;
        };
        current.sequence == ROLL_ANIMATION
            && trace
                .frames
                .iter()
                .filter(|frame| frame.host_frame < edge.host_frame)
                .max_by_key(|frame| frame.host_frame)
                .and_then(|frame| frame.animation)
                .is_none_or(|previous| previous.serial != current.serial)
    });
    if old_session_verified_increment || old_session_roll {
        return Err(format!(
            "dropped suffix produced an old-session R+3 success: verified_increment={old_session_verified_increment} fresh_roll={old_session_roll} roll_tick={roll_tick}"
        ));
    }
    if read_eof.is_some()
        && reconnect_attempt.is_some()
        && handshake_after_reconnect
        && disconnect_boundary
        && ready_slot
    {
        return Ok(session_ready.map(|observation| observation.session));
    }
    Ok(None)
}

fn validate_drop_relogin_evidence(evidence: &RunEvidence, request_id: u64) -> Result<(), String> {
    let history = evidence
        .drop_history
        .as_ref()
        .ok_or("suffix-drop evidence has no lifecycle history")?;
    let wires = wires_for_request(&evidence.trace, request_id);
    let origin = wires
        .first()
        .map(|wire| u64::from(wire.game_tick))
        .ok_or("suffix-drop evidence has no encrypted wire frames")?;
    let batch_host_frame = wires[0].host_frame;
    let baseline_host_frame = evidence
        .baseline
        .as_ref()
        .map_or(batch_host_frame, |baseline| baseline.host_frame);
    let baseline = drop_relogin_baseline(
        history,
        &evidence.trace,
        &evidence.statuses,
        request_id,
        baseline_host_frame,
    )?;
    if baseline.initial_verified != 0 {
        return Err(format!(
            "first dropped Make request began after an unexpected verified cycle: tick_verified={}",
            baseline.initial_verified
        ));
    }
    if suffix_drop_relogin_ready(
        history,
        &evidence.trace,
        &evidence.statuses,
        &baseline,
        origin,
    )?
    .is_none()
    {
        return Err(format!(
            "suffix-drop evidence did not observe read_eof, disconnect, reconnect, handshake, a new ready session, and a Ready slot: history={history:?}"
        ));
    }
    Ok(())
}

fn assert_drop_relogin(
    run: &mut LiveRun,
    request_id: u64,
    origin: u64,
    deadline: Instant,
) -> Result<(), String> {
    let history = run
        .drop_history
        .as_ref()
        .map(Arc::clone)
        .ok_or("suffix-drop case has no lifecycle recorder")?;
    let baseline_host_frame = run.baseline.as_ref().map_or_else(
        || run.hook.observed_host_frame(),
        |baseline| baseline.host_frame,
    );
    let trace = run.hook.trace();
    run.capture_status(&trace);
    let history_snapshot = history
        .lock()
        .expect("tick-engine drop history poisoned")
        .clone();
    let baseline = drop_relogin_baseline(
        &history_snapshot,
        &trace,
        &run.statuses,
        request_id,
        baseline_host_frame,
    )
    .map_err(|error| run.failure(error))?;
    if baseline.initial_verified != 0 {
        return Err(run.failure(format!(
            "first dropped Make request began after an unexpected verified cycle: tick_verified={}",
            baseline.initial_verified
        )));
    }
    let mut after_host_frame = run.hook.observed_host_frame();
    loop {
        let trace = run.hook.trace();
        run.capture_status(&trace);
        let history_snapshot = history
            .lock()
            .expect("tick-engine drop history poisoned")
            .clone();
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(run.failure(format!(
                "suffix-drop session did not observe read_eof, reconnect, handshake and Ready within {LOGOUT_DEADLINE:?}: initial_session={} initial_slot={:?} history={history_snapshot:?}",
                baseline.initial_session,
                baseline.initial_slot
            )));
        }
        if let Some(new_session) =
            suffix_drop_relogin_ready(&history_snapshot, &trace, &run.statuses, &baseline, origin)
                .map_err(|error| run.failure(error))?
        {
            println!(
                "TICK-MANIP dropped suffix closed session={} at read_eof, then session={new_session} reached Ready within {LOGOUT_DEADLINE:?}; tick_verified stayed {} with no old-session R+3 roll",
                baseline.initial_session,
                baseline.initial_verified
            );
            return Ok(());
        }
        after_host_frame = run
            .hook
            .wait_for_frame(after_host_frame, remaining.min(FRAME_HANG_GUARD))
            .ok_or_else(|| {
                run.failure(format!(
                    "host-frame hang guard expired while awaiting suffix-drop relog within {LOGOUT_DEADLINE:?}"
                ))
            })?;
    }
}

fn assert_make_without_click_outcome(run: &mut LiveRun, origin: u64) -> Result<(), String> {
    run.wait_until(
        origin as u32,
        MAKE_WITHOUT_CLICK_BUDGET,
        |trace, statuses| {
            fresh_animation_on_edge(trace, origin + 1)
                .is_some_and(|(sequence, _)| sequence == FLETCHING_ANIMATION)
                && !has_act_line(trace, origin + 1)
                && checked_click_pairs(trace, origin + 1) == 1
                && fresh_animation_on_edge(trace, origin + 3)
                    .is_some_and(|(sequence, _)| sequence == ROLL_ANIMATION)
                && statuses.last().is_some_and(|status| {
                    status.tick_slips == Some(1)
                        && status.tick_verified == Some(0)
                        && status.tick_unobserved == Some(0)
                        && status.tick_cycle == Some(-1)
                })
        },
    )?;
    let trace = run.hook.trace();
    assert_click(run, &trace, origin + 1)?;
    assert_roll(run, origin + 3)?;
    require_first_nonverified_verdict(run, "tick_slips")?;
    if has_act_line(&trace, origin + 1) {
        return Err(
            run.failure("accepted Make without the click unexpectedly produced the act line")
        );
    }
    assert_log_unchanged(run)?;
    assert_running(run)
}

fn assert_pre_batch_user_events(
    run: &LiveRun,
    dispatch: &DispatchObservation,
) -> Result<(), String> {
    let events = run
        .foreign_events
        .lock()
        .map_err(|_| run.failure("tick-engine foreign event state poisoned"))?
        .clone();
    let Some(host_frame) = events.host_frame else {
        return Err(run.failure(format!(
            "pre-batch user-event driver has no host-frame evidence: {events:?}"
        )));
    };
    let trace = run.hook.trace();
    let movement_positions: Vec<_> = trace
        .wire
        .iter()
        .enumerate()
        .filter_map(|(index, wire)| {
            (wire.host_frame == host_frame
                && wire.request_id.is_none()
                && wire.opcode == ClientProt289::MOVE_GAMECLICK.id as u8)
                .then_some(index)
        })
        .collect();
    let opener_position = trace.wire.iter().position(|wire| {
        wire.request_id == Some(dispatch.receipt.request_id)
            && wire.opcode == ClientProt289::OPHELDU.id as u8
    });
    if events.requested != 3
        || !events.sent
        || events.emitted != 3
        || events.error.is_some()
        || movement_positions.len() != 3
        || opener_position.is_none()
        || opener_position.is_some_and(|opener| {
            movement_positions
                .last()
                .is_some_and(|position| *position >= opener)
        })
        || dispatch.receipt.dispatched_prefix != 3
        || dispatch.receipt.wire_events != 4
        || dispatch.receipt.terminal.is_some()
    {
        return Err(run.failure(format!(
            "three real movement events did not precede the full KnifeArm wire: events={events:?} movement_positions={movement_positions:?} opener_position={opener_position:?} receipt={:?}",
            dispatch.receipt
        )));
    }
    Ok(())
}

fn assert_click(run: &LiveRun, trace: &TickEngineTrace, tick: u64) -> Result<(), String> {
    let count = checked_click_pairs(trace, tick);
    if count != 1 {
        return Err(run.failure(format!(
            "expected one ordinary checked click at tick {tick}, observed {count} encoded packet pairs"
        )));
    }
    Ok(())
}

fn assert_click_target(
    run: &LiveRun,
    trace: &TickEngineTrace,
    tick: u64,
    target_opcode: u8,
) -> Result<(), String> {
    assert_click(run, trace, tick)?;
    let observed: Vec<u8> = trace
        .wire
        .iter()
        .filter(|wire| {
            u64::from(wire.game_tick) == tick
                && wire.request_id.is_none()
                && [
                    ClientProt289::MOVE_OPCLICK.id as u8,
                    ClientProt289::OPLOC1.id as u8,
                    ClientProt289::OPNPC1.id as u8,
                ]
                .contains(&wire.opcode)
        })
        .map(|wire| wire.opcode)
        .collect();
    let expected = [ClientProt289::MOVE_OPCLICK.id as u8, target_opcode];
    if observed.as_slice() != expected.as_slice() {
        return Err(run.failure(format!(
            "checked click at tick {tick} encoded {observed:?}, expected {expected:?}"
        )));
    }
    Ok(())
}

fn assert_roll(run: &LiveRun, tick: u64) -> Result<(), String> {
    let trace = run.hook.trace();
    if fresh_animation_on_edge(&trace, tick).is_none_or(|(sequence, _)| sequence != ROLL_ANIMATION)
    {
        return Err(run.failure(format!(
            "expected fresh roll animation {ROLL_ANIMATION} at edge {tick}"
        )));
    }
    Ok(())
}

fn first_nonverified_verdict_present(
    counter: Option<i64>,
    last_cycle: Option<i64>,
    verified: Option<i64>,
    unobserved: Option<i64>,
) -> bool {
    // DESIGN r5:114: only a verified or unobserved cycle updates tick_cycle.
    counter.is_some_and(|count| count > 0)
        && last_cycle == Some(-1)
        && verified == Some(0)
        && unobserved == Some(0)
}

fn require_first_nonverified_verdict(run: &LiveRun, counter: &str) -> Result<(), String> {
    if !first_nonverified_verdict_present(
        run.counter(counter),
        run.counter("tick_cycle"),
        run.counter("tick_verified"),
        run.counter("tick_unobserved"),
    ) {
        return Err(run.failure(format!(
            "expected first {counter} without a verified/unobserved cycle; counter={:?} tick_cycle={:?}",
            run.counter(counter),
            run.counter("tick_cycle")
        )));
    }
    Ok(())
}

fn assert_log_unchanged(run: &LiveRun) -> Result<(), String> {
    if !run.logs_unchanged(&run.hook.trace()) {
        return Err(run.failure("raw-log count changed during a case that must conserve logs"));
    }
    Ok(())
}

fn assert_running(run: &LiveRun) -> Result<(), String> {
    if run.play().script_state(&run.account) != script::RunState::Running {
        return Err(run.failure("native Gatherer loop stopped before the case completed"));
    }
    let status = run
        .play()
        .script_native_status(&run.account)
        .ok_or_else(|| {
            run.failure("native Gatherer status disappeared before the case completed")
        })?;
    for key in ["method", "tick_method", "target", "last_event"] {
        if text_field(&status, key).is_none() {
            return Err(run.failure(format!(
                "native status is missing required Text field {key:?}"
            )));
        }
    }
    for key in [
        "tick_cycle",
        "tick_verified",
        "tick_unobserved",
        "tick_slips",
        "tick_lost",
        "tick_delayed",
        "tick_paused",
        "tick_fallbacks",
        "tick_fallback_failed",
        "tick_events",
    ] {
        if integer_field(&status, key).is_none() {
            return Err(run.failure(format!(
                "native status is missing required Integer field {key:?}"
            )));
        }
    }
    if status
        .fields
        .iter()
        .any(|field| field.key == "tick_refused")
    {
        return Err(run.failure("native status must not add a tick_refused counter"));
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct PrefixTrace {
    frames: Vec<PrefixFrame>,
    dispatches: Vec<PrefixDispatch>,
    wire: Vec<PrefixWire>,
    statuses: Vec<(u32, PrefixStatus)>,
    status_events: Vec<(u32, String)>,
    counter_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PrefixFrame {
    offset: u32,
    tick_edge: bool,
    animation: Option<(i32, u32)>,
    chat: Vec<(String, PollKind)>,
    log_count: i32,
    skill_xp: Vec<(i32, i32)>,
    if_close_delta: u64,
    main_modal_opened_delta: u64,
    main_modal_closed_delta: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PrefixDispatch {
    offset: u32,
    request: Option<String>,
    prefix: u8,
    wire_events: u8,
    terminal: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PrefixWire {
    offset: u32,
    opcode: u8,
    length: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PrefixStatus {
    phase: String,
    method: Option<String>,
    tick_method: Option<String>,
    target: Option<String>,
    tick_cycle: Option<i64>,
    tick_verified: Option<i64>,
    tick_unobserved: Option<i64>,
    tick_slips: Option<i64>,
    tick_lost: Option<i64>,
    tick_delayed: Option<i64>,
    tick_paused: Option<i64>,
    tick_fallbacks: Option<i64>,
    tick_fallback_failed: Option<i64>,
}

// Compare host control, not chance-gated gathering yield. Keep all raw yield
// and XP in the evidence; Fletching XP, logs, control chat and every emission
// remain part of this comparison.
fn chance_gather_success(line: &str) -> bool {
    line.starts_with("You catch ")
        || line.starts_with("You manage to mine some ")
        || matches!(line, "You get some logs." | "You get some oak logs.")
}

fn prefix_receipt_counter_error(
    evidence: &RunEvidence,
    origin: u32,
    end_offset: u32,
) -> Option<String> {
    let mut receipts = Vec::new();
    let mut total = 0_i64;
    for dispatch in &evidence.trace.dispatches {
        if (dispatch.game_tick as u32).wrapping_sub(origin) >= end_offset
            || dispatch.receipt.wire_events == 0
        {
            continue;
        }
        let mut count = 0;
        let mut last_wire_frame = 0;
        for packet in &evidence.trace.wire {
            if packet.request_id == Some(dispatch.receipt.request_id) {
                count += 1;
                last_wire_frame = last_wire_frame.max(packet.host_frame);
            }
        }
        if count != usize::from(dispatch.receipt.wire_events) {
            return Some(format!(
                "prefix receipt {} lacks its actual wire events",
                dispatch.receipt.request_id
            ));
        }
        // Ordered dispatch runs inside the next host observation. Wire records
        // name the last completed frame; native status names the completed one.
        let completed_frame = last_wire_frame + 1;
        let events = i64::from(dispatch.receipt.wire_events);
        receipts.push((completed_frame, events));
        total += events;
    }
    // The documented counter also includes two events per real checked
    // technique click. These have ordinary Interact, not server-ordered,
    // completions; use the same exact-frame packet proof as the click oracle.
    for packet in checked_click_pair_completions(&evidence.trace) {
        if packet.game_tick.wrapping_sub(origin) < end_offset {
            receipts.push((packet.host_frame + 1, 2));
            total += 2;
        }
    }
    receipts.sort_unstable_by_key(|&(frame, _)| frame);
    let mut next_receipt = 0;
    let mut emitted = 0_i64;
    let mut last_counter = None;
    for status in &evidence.statuses {
        if status.game_tick.wrapping_sub(origin) >= end_offset {
            continue;
        }
        while let Some(&(frame, events)) = receipts.get(next_receipt) {
            if frame > status.host_frame {
                break;
            }
            emitted += events;
            next_receipt += 1;
        }
        let Some(counter) = status.tick_events else {
            return Some("prefix status lacks its host event counter".into());
        };
        if !(0..=emitted).contains(&counter) {
            return Some(format!("prefix event counter {counter} exceeds {emitted} emitted receipt events at frame {}", status.host_frame));
        }
        last_counter = Some(counter);
    }
    if last_counter != Some(total) {
        return Some(format!(
            "prefix boundary counter {last_counter:?} differs from {total} receipt events"
        ));
    }
    None
}

fn normalized_prefix(evidence: &RunEvidence, end_offset: u32) -> PrefixTrace {
    let (Some(baseline), Some(_start_tick)) = (evidence.baseline.as_ref(), evidence.start_tick)
    else {
        return PrefixTrace::default();
    };
    let Some(origin) = evidence
        .trace
        .dispatches
        .iter()
        .find(|dispatch| is_knife_arm(dispatch))
        .map(|dispatch| dispatch.game_tick as u32)
    else {
        return PrefixTrace::default();
    };
    let mut frames: Vec<_> = evidence
        .trace
        .frames
        .iter()
        .filter_map(|frame| {
            let offset = frame.game_tick.wrapping_sub(origin);
            (offset < end_offset).then(|| PrefixFrame {
                offset,
                tick_edge: frame.tick_edge,
                animation: frame.animation.map(|animation| {
                    (
                        animation.sequence,
                        animation
                            .serial
                            .wrapping_sub(baseline.animation.map_or(0, |old| old.serial)),
                    )
                }),
                chat: frame
                    .chat
                    .iter()
                    .filter(|line| !chance_gather_success(&line.text))
                    .filter(|line| {
                        line.first_attributable_tick()
                            .checked_sub(u64::from(origin))
                            .is_some_and(|offset| offset < u64::from(end_offset))
                    })
                    .map(|line| (line.text.clone(), line.poll))
                    .collect(),
                log_count: frame.log_count,
                skill_xp: frame
                    .skill_xp
                    .iter()
                    .filter(|(index, _)| ![8, 10, 14].contains(index))
                    .map(|(index, xp)| {
                        let baseline_xp = skill_xp(baseline, *index).unwrap_or(0);
                        (*index, xp - baseline_xp)
                    })
                    .collect(),
                if_close_delta: frame
                    .server_if_close_packets
                    .saturating_sub(baseline.server_if_close_packets),
                main_modal_opened_delta: frame
                    .main_modal_opened_observation
                    .saturating_sub(baseline.main_modal_opened_observation),
                main_modal_closed_delta: frame
                    .main_modal_closed_observation
                    .saturating_sub(baseline.main_modal_closed_observation),
            })
        })
        .collect();
    // Wake frequency is not a content signal. Preserve edges and every state
    // transition, but do not compare the number of unchanged host polls.
    frames.dedup();
    let dispatches = evidence
        .trace
        .dispatches
        .iter()
        .filter_map(|dispatch| {
            let offset = (dispatch.game_tick as u32).wrapping_sub(origin);
            (offset < end_offset).then(|| PrefixDispatch {
                offset,
                request: dispatch.request_debug.as_deref().map(|request| {
                    request
                        .split([' ', '{'])
                        .next()
                        .unwrap_or(request)
                        .to_owned()
                }),
                prefix: dispatch.receipt.dispatched_prefix,
                wire_events: dispatch.receipt.wire_events,
                terminal: format!("{:?}", dispatch.receipt.terminal),
            })
        })
        .collect();
    let wire = evidence
        .trace
        .wire
        .iter()
        // Client frame upkeep, not a host action (client.rs:12873-12879).
        // Raw evidence still retains every such packet and its actual window.
        .filter(|packet| {
            packet.request_id.is_some()
                || packet.opcode != ClientProt289::ANTICHEAT_CYCLELOGIC7.id as u8
        })
        .filter_map(|packet| {
            let offset = packet.game_tick.wrapping_sub(origin);
            (offset < end_offset).then_some(PrefixWire {
                offset,
                opcode: packet.opcode,
                length: packet.encoded_length,
            })
        })
        .collect();
    let mut statuses: Vec<_> = evidence
        .statuses
        .iter()
        .filter_map(|status| {
            let offset = status.game_tick.wrapping_sub(origin);
            (offset < end_offset).then(|| {
                (
                    offset,
                    PrefixStatus {
                        phase: status.phase.clone(),
                        method: status.method.clone(),
                        tick_method: status.tick_method.clone(),
                        target: status.target.clone(),
                        tick_cycle: status.tick_cycle,
                        tick_verified: status.tick_verified,
                        tick_unobserved: status.tick_unobserved,
                        tick_slips: status.tick_slips,
                        tick_lost: status.tick_lost,
                        tick_delayed: status.tick_delayed,
                        tick_paused: status.tick_paused,
                        tick_fallbacks: status.tick_fallbacks,
                        tick_fallback_failed: status.tick_fallback_failed,
                    },
                )
            })
        })
        .collect();
    statuses.dedup_by(|next, previous| next.1 == previous.1);
    let mut status_events: Vec<_> = evidence
        .statuses
        .iter()
        .filter_map(|status| {
            let offset = status.game_tick.wrapping_sub(origin);
            let event = status.last_event.as_deref()?;
            (offset < end_offset && event != "gathering").then(|| (offset, event.to_owned()))
        })
        .collect();
    status_events.dedup();
    PrefixTrace {
        frames,
        dispatches,
        wire,
        statuses,
        status_events,
        counter_error: prefix_receipt_counter_error(evidence, origin, end_offset),
    }
}

fn prefix_difference(left: &PrefixTrace, right: &PrefixTrace) -> Option<String> {
    if let Some(error) = &left.counter_error {
        return Some(format!("left status receipt counter: {error}"));
    }
    if let Some(error) = &right.counter_error {
        return Some(format!("right status receipt counter: {error}"));
    }
    fn first_difference<T: PartialEq + std::fmt::Debug>(
        name: &str,
        left: &[T],
        right: &[T],
    ) -> Option<String> {
        if left == right {
            return None;
        }
        let index = left
            .iter()
            .zip(right)
            .position(|(left, right)| left != right)
            .unwrap_or(left.len().min(right.len()));
        Some(format!(
            "{name}: lengths {} vs {}; first difference at {index}: {:?} vs {:?}",
            left.len(),
            right.len(),
            left.get(index),
            right.get(index)
        ))
    }
    first_difference("frames", &left.frames, &right.frames)
        .or_else(|| first_difference("dispatches", &left.dispatches, &right.dispatches))
        .or_else(|| first_difference("wire", &left.wire, &right.wire))
        .or_else(|| first_difference("statuses", &left.statuses, &right.statuses))
        .or_else(|| first_difference("status events", &left.status_events, &right.status_events))
}

#[test]
#[ignore = "requires LIVE=1, GATHERER_ENGINE_DIR/NAV_PACK/CATALOG_ROOT, LIVE_EVIDENCE_DIR and Engine T 127.0.0.1:48594/4080"]
fn tick_engine_clean_continuing_river() {
    run_case(Scenario::river("tick-clean-river"), |run| {
        let trace = run.hook.trace();
        let dispatch = first_knife(&trace).map_err(|error| run.failure(error))?;
        let origin = dispatch.game_tick;
        assert_batch(run, &dispatch, 3, 4)?;
        assert_knife_wire_order(
            run,
            &trace,
            &dispatch,
            &[
                ClientProt289::OPHELDU.id as u8,
                ClientProt289::IF_BUTTON.id as u8,
                ClientProt289::MOVE_OPCLICK.id as u8,
                ClientProt289::OPLOC1.id as u8,
            ],
        )?;
        if !dispatch.request_debug.as_deref().is_some_and(|request| {
            request.contains("Continuing")
                && request.contains("Loc(")
                && request.contains("2027")
                && !request.contains("Npc(")
        }) {
            return Err(run.failure("clean continuing request was not the actual loc_2027 target"));
        }
        let close_baseline = run
            .baseline
            .as_ref()
            .expect("started run has baseline")
            .clone();
        run.wait_until(origin as u32, CLEAN_RIVER_BUDGET, |trace, statuses| {
            fresh_animation_on_edge(trace, origin + 1)
                .is_some_and(|(sequence, _)| sequence == FLETCHING_ANIMATION)
                && has_act_line(trace, origin + 1)
                && fresh_animation_on_edge(trace, origin + 3)
                    .is_some_and(|(sequence, _)| sequence == ROLL_ANIMATION)
                && close_packet_seen(trace, origin + 1, &close_baseline)
                && statuses.last().is_some_and(|status| {
                    status.tick_verified.unwrap_or(0) >= 1
                        && status.tick_cycle == Some(CLEAN_RIVER_BUDGET as i64)
                })
        })?;
        let trace = run.hook.trace();
        if frames_in_tick(&trace, origin + 1)
            .iter()
            .any(|frame| frame.actual_chat_modal_root == 8_880)
        {
            return Err(run.failure("clean river landing window observed IF_OPENCHAT"));
        }
        if fresh_animation_on_edge(&trace, origin + 1)
            .is_some_and(|(sequence, _)| sequence == CAST_ANIMATION)
        {
            return Err(
                run.failure("clean river accepted Cast animation 622 instead of a landed arm")
            );
        }
        assert_log_unchanged(run)?;
        assert_running(run)
    })
    .unwrap_or_else(|error| panic!("{error}"));
}

#[test]
#[ignore = "requires LIVE=1, GATHERER_ENGINE_DIR/NAV_PACK/CATALOG_ROOT, LIVE_EVIDENCE_DIR and Engine T 127.0.0.1:48594/4080"]
fn tick_engine_builtin_gatherer_npc_knife() {
    run_case(Scenario::npc_river("tick-npc-knife"), |run| {
        let trace = run.hook.trace();
        let dispatch = first_knife(&trace).map_err(|error| run.failure(error))?;
        let observed = run
            .setup
            .lock()
            .map_err(|_| run.failure("tick-engine setup poisoned"))?
            .npc_observations
            .clone();
        if !npc_freshfish_knife_dispatch_matches(&dispatch, &observed) {
            return Err(run.failure(format!(
                "expected a full KnifeArm Continuing receipt for freshfish NPC 309 op1: {dispatch:?}"
            )));
        }
        assert_knife_wire_order(run, &trace, &dispatch, &NPC_KNIFE_WIRE_OPCODES)?;

        let origin = dispatch.game_tick as u32;
        let baseline = run
            .baseline
            .as_ref()
            .expect("started run has baseline")
            .clone();
        let products = run.scenario.products.clone();
        let skill_index = run.scenario.skill_index;
        run.wait_until(origin, FRESHFISH_TICK_BUDGET, |trace, _| {
            trace.frames.iter().any(|frame| {
                frame.game_tick.wrapping_sub(origin) <= FRESHFISH_TICK_BUDGET
                    && fishing_gain_observed(&baseline, frame, &products, skill_index)
            })
        })?;
        assert_running(run)
    })
    .unwrap_or_else(|error| panic!("{error}"));
}

#[test]
#[ignore = "requires LIVE=1, GATHERER_ENGINE_DIR/NAV_PACK/CATALOG_ROOT, LIVE_EVIDENCE_DIR and Engine T 127.0.0.1:48594/4080"]
fn tick_engine_clean_ending_net_twenty_cycles() {
    run_case(Scenario::net("tick-clean-net"), |run| {
        let trace = run.hook.trace();
        let dispatch = first_knife(&trace).map_err(|error| run.failure(error))?;
        let origin = dispatch.game_tick;
        assert_batch(run, &dispatch, 2, 2)?;
        assert_knife_wire_order(
            run,
            &trace,
            &dispatch,
            &[
                ClientProt289::OPHELDU.id as u8,
                ClientProt289::IF_BUTTON.id as u8,
            ],
        )?;
        if !dispatch
            .request_debug
            .as_deref()
            .is_some_and(|request| request.contains("Ending"))
        {
            return Err(run.failure("net KnifeArm did not use the Ending shape"));
        }
        run.wait_until(origin as u32, NET_TICK_BUDGET, |_, statuses| {
            statuses
                .last()
                .is_some_and(|status| status.tick_unobserved.unwrap_or(0) >= NET_REQUIRED_CYCLES)
        })?;
        let trace = run.hook.trace();
        assert_click_target(run, &trace, origin + 2, ClientProt289::OPNPC1.id as u8)?;
        if run.product_gain(&trace) <= 0 && run.skill_xp_gain(&trace) <= 0 {
            return Err(run.failure("ending net produced neither a real fish nor Fishing XP"));
        }
        if run.counter("tick_unobserved").unwrap_or(0) < NET_REQUIRED_CYCLES {
            return Err(run.failure(format!(
                "ending net completed fewer than {NET_REQUIRED_CYCLES} unobserved cycles"
            )));
        }
        if run.counter("tick_verified") != Some(0) {
            return Err(run.failure("ending net unexpectedly claimed a verified roll signal"));
        }
        assert_log_unchanged(run)?;
        assert_running(run)
    })
    .unwrap_or_else(|error| panic!("{error}"));
}

#[test]
#[ignore = "requires LIVE=1, GATHERER_ENGINE_DIR/NAV_PACK/CATALOG_ROOT, LIVE_EVIDENCE_DIR and Engine T 127.0.0.1:48594/4080"]
fn tick_engine_stale_menu_guard_late_case_two() {
    // Both real Client encoders append to the same normal out-buffer flush.
    // The extra ordinary opener is not part of the native dispatch receipt.
    let scenario = Scenario::river("tick-stale-menu").with_guard_opener();
    run_case(scenario, |run| {
        let first = first_knife(&run.hook.trace()).map_err(|error| run.failure(error))?;
        let origin = first.game_tick;
        assert_batch(run, &first, 3, 4)?;
        let baseline = run
            .baseline
            .as_ref()
            .expect("started run has baseline")
            .clone();
        run.wait_until(origin as u32, STALE_MENU_BUDGET, |trace, statuses| {
            stale_guard_landing(trace, origin, &baseline)
                && statuses.last().is_some_and(|status| {
                    status.tick_slips == Some(1)
                        && status.tick_verified == Some(0)
                        && status.tick_unobserved == Some(0)
                        && status.tick_cycle == Some(-1)
                })
        })?;
        guard_carrier_user_events(&run.evidence()).map_err(|error| run.failure(error))?;
        assert_click(run, &run.hook.trace(), origin + 2)?;
        require_first_nonverified_verdict(run, "tick_slips")?;
        assert_log_unchanged(run)?;
        assert_running(run)
    })
    .unwrap_or_else(|error| panic!("{error}"));
}

#[test]
#[ignore = "requires LIVE=1, GATHERER_ENGINE_DIR/NAV_PACK/CATALOG_ROOT, LIVE_EVIDENCE_DIR and Engine T 127.0.0.1:48594/4080"]
fn tick_engine_split_opener_late_case_one() {
    let scenario = Scenario::river("tick-split-opener").with_mutation(DispatchMutation::Transport(
        TransportMutation::SplitAfterFirstPacket,
    ));
    run_case(scenario, |run| {
        let trace = run.hook.trace();
        let first = first_knife(&trace).map_err(|error| run.failure(error))?;
        let origin = first.game_tick;
        assert_batch(run, &first, 3, 4)?;
        run.wait_until(origin as u32, SPLIT_OPENER_BUDGET, |trace, statuses| {
            edge_has_openchat(trace, origin + 1)
                && fresh_animation_on_edge(trace, origin + 2)
                    .is_some_and(|(sequence, _)| sequence == FLETCHING_ANIMATION)
                && has_act_line(trace, origin + 2)
                && checked_click_pairs(trace, origin + 2) == 1
                && fresh_animation_on_edge(trace, origin + 4)
                    .is_some_and(|(sequence, _)| sequence == ROLL_ANIMATION)
                && statuses
                    .last()
                    .is_some_and(|status| status.tick_slips.unwrap_or(0) >= 1)
        })?;
        let trace = run.hook.trace();
        assert_click(run, &trace, origin + 2)?;
        assert_roll(run, origin + 4)?;
        require_first_nonverified_verdict(run, "tick_slips")?;
        if trace
            .wire
            .iter()
            .any(|wire| wire.opcode == ClientProt289::RESUME_PAUSEBUTTON.id as u8)
        {
            return Err(run.failure("split-opener branch unexpectedly emitted a pause press"));
        }
        assert_held_batch(
            run,
            first.receipt.request_id,
            &[
                ClientProt289::OPHELDU.id as u8,
                ClientProt289::IF_BUTTON.id as u8,
                ClientProt289::MOVE_OPCLICK.id as u8,
                ClientProt289::OPLOC1.id as u8,
            ],
            origin as u32 + 1,
        )?;
        assert_log_unchanged(run)?;
        assert_running(run)
    })
    .unwrap_or_else(|error| panic!("{error}"));
}

#[test]
#[ignore = "requires LIVE=1, GATHERER_ENGINE_DIR/NAV_PACK/CATALOG_ROOT, LIVE_EVIDENCE_DIR and Engine T 127.0.0.1:48594/4080"]
fn tick_engine_plain_late_river_and_mithril_iron() {
    let river = Scenario::river("tick-late-river").with_mutation(DispatchMutation::Transport(
        TransportMutation::HoldThroughNextEdge,
    ));
    run_case(river, |run| {
        let first = first_knife(&run.hook.trace()).map_err(|error| run.failure(error))?;
        let origin = first.game_tick;
        assert_batch(run, &first, 3, 4)?;
        // The first window may include interface packets before PLAYER_INFO.
        run.wait_until(origin as u32, LATE_RIVER_BUDGET, |trace, statuses| {
            fresh_animation_on_edge(trace, origin + 2).is_some_and(|(sequence, _)| {
                sequence == FLETCHING_ANIMATION || sequence == CAST_ANIMATION
            }) && checked_click_pairs(trace, origin + 2) == 1
                && fresh_animation_on_edge(trace, origin + 5)
                    .is_some_and(|(sequence, _)| sequence == ROLL_ANIMATION)
                && knife_dispatches(trace)
                    .iter()
                    .any(|dispatch| dispatch.game_tick == origin + 5)
                && statuses
                    .last()
                    .is_some_and(|status| status.tick_slips.unwrap_or(0) >= 1)
        })?;
        let trace = run.hook.trace();
        assert_click(run, &trace, origin + 2)?;
        assert_roll(run, origin + 5)?;
        require_first_nonverified_verdict(run, "tick_slips")?;
        if knife_dispatches(&trace)
            .iter()
            .filter(|dispatch| dispatch.game_tick < origin + 5)
            .count()
            != 1
        {
            return Err(run.failure("river late batch emitted a replacement before its R+5 roll"));
        }
        assert_held_batch(
            run,
            first.receipt.request_id,
            &[
                ClientProt289::OPHELDU.id as u8,
                ClientProt289::IF_BUTTON.id as u8,
                ClientProt289::MOVE_OPCLICK.id as u8,
                ClientProt289::OPLOC1.id as u8,
            ],
            origin as u32 + 1,
        )?;
        assert_log_unchanged(run)?;
        assert_running(run)
    })
    .unwrap_or_else(|error| panic!("{error}"));

    let iron = Scenario::iron("tick-late-iron")
        .unwrap_or_else(|error| panic!("{error}"))
        .with_mutation(DispatchMutation::HoldOnFreshMiningRoll {
            sequence: MITHRIL_PICK_ANIMATION,
        });
    run_case(iron, |run| {
        let native_start = run
            .baseline
            .as_ref()
            .map(|baseline| baseline.game_tick)
            .ok_or_else(|| run.failure("missing iron setup baseline"))?;
        run.wait_until(native_start, STARTUP_EDGE_BUDGET, |trace, _| {
            tick_startup::native_click_within(trace, native_start, STARTUP_EDGE_BUDGET)
        })?;
        let selection = Arc::clone(&run.hook);
        let start = run.hook.observed_game_tick();
        let mut precondition = IronPrecondition::Waiting { opportunities: 0 };
        run.wait_until(start, NATURAL_IRON_ROLL_BUDGET, |trace, _| {
            let current = trace.frames.last().map_or(start, |frame| frame.game_tick);
            precondition = iron_precondition(trace, selection.fresh_roll_request_id(), start, current);
            !matches!(precondition, IronPrecondition::Waiting { .. })
        })?;
        let (request_id, selected_tick, opportunities) = match precondition {
            IronPrecondition::Selected { request_id, roll_tick, opportunities } => {
                (request_id, roll_tick, opportunities)
            }
            IronPrecondition::NoEligibleFailure { opportunities } => return Err(run.failure(format!(
                "chance precondition: no eligible failed roll in {opportunities} observed opportunities; p <= 4.07e-8, not a late-acceptance result"
            ))),
            IronPrecondition::GuardExhausted { opportunities } => return Err(run.failure(format!(
                "scheduling/target precondition: only {opportunities} observed opportunities by the fixed {NATURAL_IRON_ROLL_BUDGET}-edge guard; not a chance or late-acceptance result"
            ))),
            IronPrecondition::SelectionMissing { roll_tick, opportunities } => return Err(run.failure(format!(
                "scheduling precondition: eligible failed roll at {roll_tick}, opportunity {opportunities}, has no selected held request"
            ))),
            IronPrecondition::Waiting { .. } => unreachable!("wait stopped on a terminal precondition"),
        };
        println!("TICK-MANIP iron precondition selected real failed roll R={selected_tick}, request={request_id}, observed_opportunities={opportunities}; fixed_guard={NATURAL_IRON_ROLL_BUDGET}, limit={IRON_ROLL_OPPORTUNITY_LIMIT}");
        let first = run.hook.trace().dispatches.into_iter()
            .find(|dispatch| dispatch.receipt.request_id == request_id)
            .ok_or_else(|| run.failure("selected Mining-roll receipt disappeared"))?;
        assert_batch(run, &first, 3, 4)?;
        debug_assert_eq!(first.game_tick, selected_tick);
        let release_edge_tick = selected_tick as u32 + 1;
        let held_proxy = run
            .proxy
            .as_ref()
            .cloned()
            .ok_or_else(|| run.failure("iron roll hold requires the real loopback proxy"))?;
        run.wait_until(selected_tick as u32, LATE_IRON_BUDGET, |_, _| {
            held_proxy.observations().iter().any(|observation| {
                observation.request_id == request_id
                    && observation.release_edge_tick == Some(release_edge_tick)
            })
        })?;
        assert_held_batch(
            run,
            request_id,
            &[
                ClientProt289::OPHELDU.id as u8,
                ClientProt289::IF_BUTTON.id as u8,
                ClientProt289::MOVE_OPCLICK.id as u8,
                ClientProt289::OPLOC1.id as u8,
            ],
            release_edge_tick,
        )?;
        let origin = first.game_tick;
        run.wait_until(origin as u32, LATE_IRON_BUDGET, |trace, statuses| {
            validate_late_iron_history(trace, statuses, origin).is_ok()
        }).map_err(|error| format!("late-acceptance failed after its selected-roll precondition: {error}"))?;
        let trace = run.hook.trace();
        assert_click(run, &trace, origin + 2)?;
        validate_late_iron_history(&trace, &run.statuses, origin)
            .map_err(|error| run.failure(error))?;
        println!("TICK-MANIP iron late-acceptance passed independently of precondition selection: R+2 act/click, R+5 real roll, one Slip, unchanged prior cycle totals");
        assert_log_unchanged(run)?;
        assert_running(run)
    })
    .unwrap_or_else(|error| panic!("{error}"));
}

fn copper_history(scenario: Scenario, expect_held_join: bool) -> Result<RunEvidence, String> {
    run_case(scenario, |run| {
        let initial_trace = run.hook.trace();
        let first = first_knife(&initial_trace).map_err(|error| run.failure(error))?;
        let origin = first.game_tick;
        assert_batch(run, &first, 3, 4)?;
        assert_knife_wire_order(
            run,
            &initial_trace,
            &first,
            &[
                ClientProt289::OPHELDU.id as u8,
                ClientProt289::IF_BUTTON.id as u8,
                ClientProt289::MOVE_OPCLICK.id as u8,
                ClientProt289::OPLOC1.id as u8,
            ],
        )?;
        run.wait_until(origin as u32, COPPER_BUDGET, |trace, statuses| {
            fresh_animation_on_edge(trace, origin + 1).is_some_and(|(sequence, _)| sequence == -1)
                && has_copper_success(trace, origin, origin + 1)
                && knife_dispatches(trace).iter().any(|dispatch| {
                    dispatch.game_tick == origin + 1
                        && dispatch.request_debug != first.request_debug
                })
                && statuses
                    .last()
                    .is_some_and(|status| status.tick_delayed.unwrap_or(0) >= 1)
                && if expect_held_join {
                    edge_frame(trace, origin + 2).is_some()
                } else {
                    fresh_animation_on_edge(trace, origin + 4)
                        .is_some_and(|(sequence, _)| sequence == BRONZE_PICK_ANIMATION)
                        && statuses
                            .last()
                            .is_some_and(|status| status.tick_cycle == Some(3))
                }
        })?;
        let trace = run.hook.trace();
        let batches = knife_dispatches(&trace);
        if batches.len() < 2 || batches[1].game_tick != origin + 1 {
            return Err(
                run.failure("copper delayed cycle did not emit its second-rock batch at R+1")
            );
        }
        assert_batch(run, &batches[1], 3, 4)?;
        assert_knife_wire_order(
            run,
            &trace,
            &batches[1],
            &[
                ClientProt289::OPHELDU.id as u8,
                ClientProt289::IF_BUTTON.id as u8,
                ClientProt289::MOVE_OPCLICK.id as u8,
                ClientProt289::OPLOC1.id as u8,
            ],
        )?;
        if run.product_gain(&trace) <= 0 && run.skill_xp_gain(&trace) <= 0 {
            return Err(run.failure("copper case showed no real ore or Mining XP"));
        }
        if batches
            .iter()
            .any(|dispatch| dispatch.receipt.terminal.is_some())
        {
            return Err(run.failure("copper host receipt recorded a refused KnifeArm"));
        }
        if expect_held_join {
            let observations = run
                .proxy
                .as_ref()
                .map_or_else(Vec::new, |proxy| proxy.observations());
            let expected_opcodes = [
                ClientProt289::OPHELDU.id as u8,
                ClientProt289::IF_BUTTON.id as u8,
                ClientProt289::MOVE_OPCLICK.id as u8,
                ClientProt289::OPLOC1.id as u8,
            ];
            let complete = observations.first().is_some_and(|observation| {
                let expected_bytes: usize = observation.packet_lengths.iter().sum();
                observation.request_ids
                    == vec![batches[0].receipt.request_id, batches[1].receipt.request_id]
                    && observation.packet_opcodes == expected_opcodes.repeat(2)
                    && observation.release_edge_tick.is_none()
                    && observation.release_trigger_tick == Some(origin as u32 + 1)
                    && observation.released_host_frame.is_some()
                    && observation.dropped_bytes == 0
                    && observation.forwarded_bytes == expected_bytes
            });
            if observations.len() != 1 || !complete {
                return Err(run.failure(
                    "held copper gate did not forward both real batch streams after the second dispatch completed",
                ));
            }
        }
        assert_log_unchanged(run)?;
        assert_running(run)?;
        // The measured four-edge history is complete. A normal-rock success
        // can still own protected access through its following edge (SPEC
        // section 4 / facts 3.3), so stop new work and drain those two windows
        // before the ordinary logout button. This is cleanup, not an
        // extension of COPPER_BUDGET or a new recovery action.
        run.play().script_stop(&run.account);
        let stopped_at = run.hook.observed_game_tick();
        run.wait_until(stopped_at, 2, |trace, _| {
            edge_frame(trace, u64::from(stopped_at) + 2).is_some()
        })?;
        // SPEC 11.3(6) requires only the common prefix through the second
        // edge for the held history; later engine outcomes remain evidence.
        // Its queued Fletching can consume the log during this cleanup.
        if !expect_held_join {
            assert_log_unchanged(run)?;
        }
        Ok(())
    })
}

#[test]
#[ignore = "requires LIVE=1, GATHERER_ENGINE_DIR/NAV_PACK/CATALOG_ROOT, LIVE_EVIDENCE_DIR and Engine T 127.0.0.1:48594/4080"]
fn tick_engine_copper_success_and_delayed_first_batch_history() {
    let normal = copper_history(Scenario::copper("tick-copper-normal").unwrap(), false)
        .unwrap_or_else(|error| panic!("{error}"));
    let held =
        copper_history(
            Scenario::copper("tick-copper-held").unwrap().with_mutation(
                DispatchMutation::Transport(TransportMutation::HoldUntilNextDispatch),
            ),
            true,
        )
        .unwrap_or_else(|error| panic!("{error}"));
    let normal_prefix = normalized_prefix(&normal, 2);
    let held_prefix = normalized_prefix(&held, 2);
    if let Some(difference) = prefix_difference(&normal_prefix, &held_prefix) {
        panic!("copper host trace diverged before the second edge: {difference}");
    }
}

#[test]
#[ignore = "requires LIVE=1, GATHERER_ENGINE_DIR/NAV_PACK/CATALOG_ROOT, LIVE_EVIDENCE_DIR and Engine T 127.0.0.1:48594/4080"]
fn tick_engine_removed_target_refuses_prefix_zero_and_continues() {
    let scenario = Scenario::river("tick-target-removed");
    let (id, target) = scenario.required_loc.expect("river has a fixed loc");
    let scenario = scenario.with_mutation(DispatchMutation::RemoveLoc { id, tile: target });
    run_case(scenario, |run| {
        let first = first_knife(&run.hook.trace()).map_err(|error| run.failure(error))?;
        let origin = first.game_tick;
        run.wait_until(origin as u32, HOST_REFUSAL_BUDGET, |trace, statuses| {
            trace.dispatches.iter().any(|dispatch| {
                is_knife_arm(dispatch)
                    && dispatch.game_tick == origin
                    && dispatch.mutation_applied
                    && dispatch.receipt.dispatched_prefix == 0
                    && dispatch.receipt.wire_events == 0
                    && format!("{:?}", dispatch.receipt.terminal).contains("StaleTarget")
            }) && checked_click_pairs(trace, origin + 1) == 1
                && statuses
                    .last()
                    .is_some_and(|status| status.tick_lost.unwrap_or(0) >= 1)
        })?;
        let trace = run.hook.trace();
        let refused = trace
            .dispatches
            .iter()
            .find(|dispatch| is_knife_arm(dispatch) && dispatch.game_tick == origin)
            .ok_or_else(|| run.failure("removed target receipt disappeared"))?;
        if refused.receipt.dispatched_prefix != 0
            || refused.receipt.wire_events != 0
            || !refused.mutation_applied
            || !format!("{:?}", refused.receipt.terminal).contains("StaleTarget")
        {
            return Err(run.failure(format!(
                "target removal was not a real prefix-zero refusal: {:?}",
                refused.receipt
            )));
        }
        if !wires_for_request(&trace, refused.receipt.request_id).is_empty() {
            return Err(run.failure("host-refused target removal emitted a Client packet"));
        }
        assert_click(run, &trace, origin + 1)?;
        require_first_nonverified_verdict(run, "tick_lost")?;
        assert_running(run)
    })
    .unwrap_or_else(|error| panic!("{error}"));
}

#[test]
#[ignore = "requires LIVE=1, GATHERER_ENGINE_DIR/NAV_PACK/CATALOG_ROOT, LIVE_EVIDENCE_DIR and Engine T 127.0.0.1:48594/4080"]
fn tick_engine_accepted_make_without_click() {
    let scenario = Scenario::river("tick-make-without-click")
        .with_mutation(DispatchMutation::CancelAfterPackets(2));
    run_case(scenario, |run| {
        let first = first_knife(&run.hook.trace()).map_err(|error| run.failure(error))?;
        let origin = first.game_tick;
        assert_batch(run, &first, 2, 2)?;
        assert_knife_wire_order(
            run,
            &run.hook.trace(),
            &first,
            &[
                ClientProt289::OPHELDU.id as u8,
                ClientProt289::IF_BUTTON.id as u8,
            ],
        )?;
        assert_make_without_click_outcome(run, origin)
    })
    .unwrap_or_else(|error| panic!("{error}"));
}

#[test]
#[ignore = "requires LIVE=1, GATHERER_ENGINE_DIR/NAV_PACK/CATALOG_ROOT, LIVE_EVIDENCE_DIR and Engine T 127.0.0.1:48594/4080"]
fn tick_engine_deferred_terminal_row_is_a_slip_after_three_foreign_events() {
    let scenario = Scenario::river("tick-deferred-terminal-row").with_pre_batch_user_events(3);
    run_case(scenario, |run| {
        let first = first_knife(&run.hook.trace()).map_err(|error| run.failure(error))?;
        let origin = first.game_tick;
        assert_batch(run, &first, 3, 4)?;
        assert_knife_wire_order(
            run,
            &run.hook.trace(),
            &first,
            &[
                ClientProt289::OPHELDU.id as u8,
                ClientProt289::IF_BUTTON.id as u8,
                ClientProt289::MOVE_OPCLICK.id as u8,
                ClientProt289::OPLOC1.id as u8,
            ],
        )?;
        assert_pre_batch_user_events(run, &first)?;
        let initial = run
            .statuses
            .last()
            .cloned()
            .ok_or_else(|| run.failure("missing pre-cycle status"))?;
        let initial_verified = initial.tick_verified.unwrap_or(0);
        let initial_unobserved = initial.tick_unobserved.unwrap_or(0);
        let initial_slips = initial.tick_slips.unwrap_or(0);
        let initial_lost = initial.tick_lost.unwrap_or(0);
        let initial_delayed = initial.tick_delayed.unwrap_or(0);
        let initial_paused = initial.tick_paused.unwrap_or(0);
        let initial_fallbacks = initial.tick_fallbacks.unwrap_or(0);
        let initial_fallback_failed = initial.tick_fallback_failed.unwrap_or(0);
        run.wait_until(origin as u32, MAKE_WITHOUT_CLICK_BUDGET, |trace, statuses| {
            fresh_animation_on_edge(trace, origin + 1)
                .is_some_and(|(sequence, _)| sequence == FLETCHING_ANIMATION)
                && !has_act_line(trace, origin + 1)
                && checked_click_pairs(trace, origin + 1) == 1
                && fresh_animation_on_edge(trace, origin + 3)
                    .is_some_and(|(sequence, _)| sequence == ROLL_ANIMATION)
                && statuses.last().is_some_and(|status| {
                    status.tick_slips == Some(initial_slips + 1)
                        && status.tick_fallbacks == Some(initial_fallbacks)
                        && status.tick_verified == Some(initial_verified)
                })
        })?;
        let trace = run.hook.trace();
        assert_click(run, &trace, origin + 1)?;
        assert_roll(run, origin + 3)?;
        if has_act_line(&trace, origin + 1) {
            return Err(run.failure("deferred terminal-row landing unexpectedly had an act line"));
        }
        let final_status = run
            .statuses
            .last()
            .ok_or_else(|| run.failure("missing deferred-row terminal status"))?;
        if final_status.tick_verified != Some(initial_verified)
            || final_status.tick_unobserved != Some(initial_unobserved)
            || final_status.tick_slips != Some(initial_slips + 1)
            || final_status.tick_lost != Some(initial_lost)
            || final_status.tick_delayed != Some(initial_delayed)
            || final_status.tick_paused != Some(initial_paused)
            || final_status.tick_fallbacks != Some(initial_fallbacks)
            || final_status.tick_fallback_failed != Some(initial_fallback_failed)
        {
            return Err(run.failure(format!(
                "deferred terminal-row cycle did not end as exactly one slip without a second fallback: initial={initial:?} final={final_status:?}"
            )));
        }
        assert_log_unchanged(run)?;
        assert_running(run)
    })
    .unwrap_or_else(|error| panic!("{error}"));
}

#[test]
#[ignore = "requires LIVE=1, GATHERER_ENGINE_DIR/NAV_PACK/CATALOG_ROOT, LIVE_EVIDENCE_DIR and Engine T 127.0.0.1:48594/4080"]
fn tick_engine_proxy_drop_after_make_keeps_full_host_receipt() {
    let scenario = Scenario::river("tick-make-proxy-drop").with_mutation(
        DispatchMutation::Transport(TransportMutation::DropSuffixAfterPacket(2)),
    );
    run_case(scenario, |run| {
        let first = first_knife(&run.hook.trace()).map_err(|error| run.failure(error))?;
        let origin = first.game_tick;
        assert_batch(run, &first, 3, 4)?;
        assert_knife_wire_order(
            run,
            &run.hook.trace(),
            &first,
            &[
                ClientProt289::OPHELDU.id as u8,
                ClientProt289::IF_BUTTON.id as u8,
                ClientProt289::MOVE_OPCLICK.id as u8,
                ClientProt289::OPLOC1.id as u8,
            ],
        )?;
        assert_proxy_drop_after_make(run, first.receipt.request_id, origin)
    })
    .unwrap_or_else(|error| panic!("{error}"));
}

fn lone_opener_continues(
    trace: &TickEngineTrace,
    origin: u64,
    baseline: &FrameObservation,
) -> bool {
    checked_click_pairs(trace, origin + 1) == 1
        && close_packet_seen(trace, origin + 1, baseline)
        && (origin + 1..=origin + u64::from(LONE_OPENER_BUDGET))
            .any(|tick| has_act_line(trace, tick))
}

fn lone_opener_history(name: &'static str, suppress: bool) -> Result<RunEvidence, String> {
    let mut scenario = Scenario::river(name).with_mutation(DispatchMutation::CancelAfterPackets(1));
    if suppress {
        scenario = scenario.with_openchat_suppression(4);
    }
    run_case(scenario, |run| {
        let first = first_knife(&run.hook.trace()).map_err(|error| run.failure(error))?;
        let origin = first.game_tick;
        assert_batch(run, &first, 1, 1)?;
        assert_knife_wire_order(
            run,
            &run.hook.trace(),
            &first,
            &[ClientProt289::OPHELDU.id as u8],
        )?;
        let baseline = run
            .baseline
            .as_ref()
            .expect("started run has baseline")
            .clone();
        run.wait_until(origin as u32, LONE_OPENER_BUDGET, |trace, statuses| {
            lone_opener_continues(trace, origin, &baseline)
                // Compare three complete windows, not whichever partial wake
                // happened to deliver the act line first in each account.
                && edge_frame(trace, origin + 3).is_some()
                && statuses
                    .last()
                    .is_some_and(|status| status.tick_method.is_some())
        })?;
        let trace = run.hook.trace();
        assert_click(run, &trace, origin + 1)?;
        if trace.wire.iter().any(|wire| {
            wire.opcode == ClientProt289::IF_BUTTON.id as u8
                || wire.opcode == ClientProt289::CLOSE_MODAL.id as u8
        }) {
            return Err(run.failure("lone opener emitted IF_BUTTON or host CLOSE_MODAL"));
        }
        if suppress {
            if !frames_in_tick(&trace, origin + 1)
                .iter()
                .any(|frame| frame.suppressed_openchat && frame.actual_chat_modal_root == 8_880)
            {
                return Err(
                    run.failure("suppressed history did not filter a real IF_OPENCHAT snapshot")
                );
            }
        } else if !edge_has_openchat(&trace, origin + 1) {
            return Err(run.failure("unsuppressed lone opener never exposed the real 8880 root"));
        }
        assert_log_unchanged(run)?;
        assert_running(run)
    })
}

#[test]
#[ignore = "requires LIVE=1, GATHERER_ENGINE_DIR/NAV_PACK/CATALOG_ROOT, LIVE_EVIDENCE_DIR and Engine T 127.0.0.1:48594/4080"]
fn tick_engine_lone_opener_with_and_without_snapshot_suppression() {
    let visible =
        lone_opener_history("tick-lone-opener", false).unwrap_or_else(|error| panic!("{error}"));
    let hidden = lone_opener_history("tick-lone-opener-hidden", true)
        .unwrap_or_else(|error| panic!("{error}"));
    let visible_prefix = normalized_prefix(&visible, 3);
    let hidden_prefix = normalized_prefix(&hidden, 3);
    if let Some(difference) = prefix_difference(&visible_prefix, &hidden_prefix) {
        panic!(
            "lone-opener host trace changed when IF_OPENCHAT was snapshot-suppressed: {difference}"
        );
    }
}

#[test]
#[ignore = "requires LIVE=1, GATHERER_ENGINE_DIR/NAV_PACK/CATALOG_ROOT, LIVE_EVIDENCE_DIR, selected local free-world world.json, and Engine T node.members=false phase"]
fn tick_engine_free_world_disables_members_only_knife_arm() {
    run_case(Scenario::oak("tick-free-world-oak"), |run| {
        let start = first_knife(&run.hook.trace())
            .map_err(|error| run.failure(error))?
            .game_tick as u32;
        run.wait_until(start, FREE_WORLD_BUDGET, |trace, statuses| {
            has_fletching_members_line(
                trace,
                u64::from(start + 1),
                u64::from(start + FREE_WORLD_BUDGET),
            ) && checked_click_pairs(trace, u64::from(start + 1)) == 1
                && statuses.last().is_some_and(|status| {
                    status.tick_method.as_deref() == Some("Knife (off: members-only-arm)")
                        && status.tick_fallbacks == Some(1)
                        && status.tick_fallback_failed == Some(0)
                })
        })?;
        let trace = run.hook.trace();
        let knife_dispatches = knife_dispatches(&trace);
        if knife_dispatches.is_empty() {
            return Err(run.failure("free-world Fletching refusal had no real KnifeArm"));
        }
        let knife_wires = wires_for_request(&trace, knife_dispatches[0].receipt.request_id);
        if !knife_wires
            .iter()
            .any(|wire| wire.opcode == ClientProt289::OPHELDU.id as u8)
        {
            return Err(run.failure("free-world KnifeArm did not encode OPHELDU"));
        }
        if trace
            .frames
            .iter()
            .any(|frame| frame.actual_chat_modal_root == 8_880)
        {
            return Err(run.failure("free-world Fletching refusal opened a Make menu"));
        }
        if !has_fletching_members_line(
            &trace,
            u64::from(start + 1),
            u64::from(start + FREE_WORLD_BUDGET),
        ) {
            return Err(run.failure(
                "free-world run did not expose the authored Fletching members-only message",
            ));
        }
        assert_click_target(run, &trace, u64::from(start + 1), ClientProt289::OPLOC1.id as u8)?;
        if run.latest_tick_method() != Some("Knife (off: members-only-arm)") {
            return Err(run.failure("tick_method did not report Knife (off: members-only-arm)"));
        }
        let final_status = run
            .statuses
            .last()
            .ok_or_else(|| run.failure("missing free-world fallback status"))?;
        if final_status.tick_fallbacks != Some(1)
            || final_status.tick_fallback_failed != Some(0)
        {
            return Err(run.failure(format!(
                "free-world refusal did not produce exactly one successful fallback: {final_status:?}"
            )));
        }
        assert_running(run)
    })
    .unwrap_or_else(|error| panic!("{error}"));
}

impl LiveRun {
    fn latest_tick_method(&self) -> Option<&str> {
        self.statuses
            .last()
            .and_then(|status| status.tick_method.as_deref())
    }
}

#[cfg(test)]
mod packet_oracle_tests {
    use super::*;
    use host_play::tick_engine_test_support::ChatObservation;

    fn observed_frame(game_tick: u32, tick_edge: bool, if_close: u64) -> FrameObservation {
        FrameObservation {
            host_frame: u64::from(game_tick) * 10,
            game_tick,
            client_tick: game_tick,
            tick_edge,
            animation: None,
            chat: Vec::new(),
            inventory_counts: Vec::new(),
            log_count: 1,
            skill_xp: Vec::new(),
            player_tile: None,
            chat_modal_root: -1,
            actual_chat_modal_root: -1,
            server_if_close_packets: if_close,
            iface_generation: 0,
            main_modal_opened_observation: 0,
            main_modal_closed_observation: 0,
            suppressed_openchat: false,
        }
    }

    fn prefix_evidence() -> RunEvidence {
        let mut evidence = evidence(
            vec![wire(ClientProt289::OPHELDU.id as u8, 1, 13)],
            vec![],
            vec![],
        );
        evidence.baseline = Some(observed_frame(6, true, 1));
        evidence.trace.dispatches.push(DispatchObservation {
            game_tick: 7,
            receipt: interact::server_ordered::ServerOrderedDispatch::refused(
                1,
                7,
                3,
                0,
                interact::server_ordered::ServerOrderedError::Cancelled,
            ),
            request_debug: Some("KnifeArm".into()),
            mutation_applied: false,
        });
        evidence.trace.frames = vec![observed_frame(7, true, 1), observed_frame(7, false, 1)];
        evidence.statuses.push(StatusObservation {
            host_frame: 70,
            game_tick: 7,
            phase: "Working".into(),
            method: Some("loc_2027".into()),
            tick_method: Some("Knife".into()),
            target: Some("loc_2027".into()),
            last_event: Some("gathering".into()),
            tick_cycle: Some(-1),
            tick_verified: Some(0),
            tick_unobserved: Some(0),
            tick_slips: Some(0),
            tick_lost: Some(0),
            tick_delayed: Some(0),
            tick_paused: Some(0),
            tick_fallbacks: Some(0),
            tick_fallback_failed: Some(0),
            tick_events: Some(0),
        });
        evidence
    }

    fn receipt_status_evidence() -> RunEvidence {
        let mut evidence = prefix_evidence();
        let receipt = &mut evidence.trace.dispatches[0].receipt;
        receipt.rows[0] = interact::server_ordered::RowDispatch::HostQueued { wire_events: 1 };
        receipt.rows[1] = interact::server_ordered::RowDispatch::Refused {
            reason: interact::server_ordered::ServerOrderedError::Cancelled,
        };
        receipt.dispatched_prefix = 1;
        receipt.wire_events = 1;
        evidence.statuses[0].tick_events = Some(1);
        let mut boundary = evidence.statuses[0].clone();
        boundary.host_frame += 10;
        boundary.game_tick += 1;
        evidence.statuses.push(boundary);
        evidence
    }

    #[test]
    fn prefix_status_accepts_generic_pulses_in_both_directions() {
        let mut original = receipt_status_evidence();
        for status in &mut original.statuses {
            status.last_event = Some("tick: armed".into());
        }
        let reference = normalized_prefix(&original, 2);
        assert_eq!(reference.counter_error, None);
        for at in 0..2 {
            let mut pulsed = original.clone();
            let mut pulse = pulsed.statuses[at].clone();
            pulse.host_frame += 1;
            pulse.last_event = Some("gathering".into());
            pulsed.statuses.insert(at + 1, pulse);
            let observed = normalized_prefix(&pulsed, 2);
            assert_eq!(prefix_difference(&reference, &observed), None);
            assert_eq!(prefix_difference(&observed, &reference), None);
            assert_eq!(pulsed.statuses.len(), original.statuses.len() + 1);
        }
    }

    #[test]
    fn prefix_status_accepts_one_tick_receipt_display_lag_in_both_directions() {
        let original = receipt_status_evidence();
        let mut lagged = original.clone();
        lagged.statuses[0].tick_events = Some(0);
        let reference = normalized_prefix(&original, 2);
        let observed = normalized_prefix(&lagged, 2);
        assert_eq!(prefix_difference(&reference, &observed), None);
        assert_eq!(prefix_difference(&observed, &reference), None);
        assert_eq!(lagged.statuses[1].tick_events, Some(1));
    }

    #[test]
    fn prefix_status_rejects_semantic_events_and_counter_errors_in_both_directions() {
        let original = receipt_status_evidence();
        let reference = normalized_prefix(&original, 2);
        assert_eq!(reference.counter_error, None);
        for facet in 0..18 {
            let mut changed = original.clone();
            let status = &mut changed.statuses[0];
            match facet {
                0 => status.phase = "Blocked".into(),
                1 => status.method = Some("other-method".into()),
                2 => status.tick_method = Some("Off".into()),
                3 => status.target = Some("other-target".into()),
                4 => status.tick_cycle = Some(3),
                5 => status.tick_verified = Some(1),
                6 => status.tick_unobserved = Some(1),
                7 => status.tick_slips = Some(1),
                8 => status.tick_lost = Some(1),
                9 => status.tick_delayed = Some(1),
                10 => status.tick_paused = Some(1),
                11 => status.tick_fallbacks = Some(1),
                12 => status.tick_fallback_failed = Some(1),
                13 => status.last_event = Some("tick: unexpected".into()),
                14 => status.tick_events = Some(2),
                15 => changed.statuses[1].tick_events = Some(0),
                16 => status.tick_events = None,
                17 => status.host_frame = 1,
                _ => unreachable!(),
            }
            let observed = normalized_prefix(&changed, 2);
            assert!(
                prefix_difference(&reference, &observed).is_some(),
                "forward facet {facet}"
            );
            assert!(
                prefix_difference(&observed, &reference).is_some(),
                "reverse facet {facet}"
            );
        }
    }

    #[test]
    fn prefix_status_counts_real_ordinary_checked_pairs_without_ordered_receipts() {
        for opcode in [
            ClientProt289::OPLOC1.id as u8,
            ClientProt289::OPNPC1.id as u8,
        ] {
            let mut original = receipt_status_evidence();
            let mut movement = wire(ClientProt289::MOVE_OPCLICK.id as u8, 11, 7);
            movement.request_id = None;
            movement.host_frame = 75;
            movement.game_tick = 8;
            let mut target = movement;
            target.opcode = opcode;
            original.trace.wire.extend([movement, target]);
            original.statuses[1].tick_events = Some(3);
            assert_eq!(original.trace.dispatches.len(), 1);
            assert_eq!(checked_click_pairs(&original.trace, 8), 1);
            let reference = normalized_prefix(&original, 2);
            assert_eq!(reference.counter_error, None);
            assert_eq!(prefix_difference(&reference, &reference), None);
            for facet in 0..6 {
                let mut changed = original.clone();
                match facet {
                    0 => changed.trace.wire[2].host_frame += 1,
                    1 => changed.trace.wire[2].request_id = Some(11),
                    2 => changed.statuses[0].tick_events = Some(3),
                    3 => changed.trace.wire[2].opcode = ClientProt289::IF_BUTTON.id as u8,
                    4 => changed.trace.wire[2].game_tick += 1,
                    5 => {
                        changed.trace.wire.remove(1);
                    }
                    _ => unreachable!(),
                }
                let observed = normalized_prefix(&changed, 2);
                assert!(
                    observed.counter_error.is_some(),
                    "opcode={opcode} facet={facet}"
                );
                assert!(prefix_difference(&reference, &observed).is_some());
                assert!(prefix_difference(&observed, &reference).is_some());
            }
        }
    }

    #[test]
    fn first_nonverified_verdict_does_not_invent_a_last_verified_duration() {
        assert!(first_nonverified_verdict_present(
            Some(1),
            Some(-1),
            Some(0),
            Some(0)
        ));
        for (counter, cycle, verified, unobserved) in [
            (Some(0), Some(-1), Some(0), Some(0)),
            (None, Some(-1), Some(0), Some(0)),
            (Some(1), Some(4), Some(0), Some(0)),
            (Some(1), Some(5), Some(0), Some(0)),
            (Some(1), Some(-1), Some(1), Some(0)),
            (Some(1), Some(-1), Some(0), Some(1)),
        ] {
            assert!(!first_nonverified_verdict_present(
                counter, cycle, verified, unobserved
            ));
        }
    }

    fn stale_guard_evidence() -> RunEvidence {
        let mut evidence = prefix_evidence();
        let receipt = &mut evidence.trace.dispatches[0].receipt;
        receipt.dispatched_prefix = 3;
        receipt.wire_events = 4;
        receipt.terminal = None;
        evidence.pre_batch_guard_opener = GuardOpenerState {
            armed: false,
            sent: true,
            error: None,
            client_tick: Some(7),
            host_frame: Some(70),
            encoded_length: Some(13),
        };
        evidence.trace.wire = [200, 200, 86, 67, 10]
            .into_iter()
            .enumerate()
            .map(|(index, opcode)| {
                let mut wire = wire(opcode, 1, if opcode == 200 { 13 } else { 1 });
                wire.host_frame = 70;
                if index == 0 {
                    wire.request_id = None;
                }
                wire
            })
            .collect();
        for opcode in [67, 10] {
            let mut wire = wire(opcode, 1, 1);
            wire.game_tick = 9;
            wire.host_frame = 90;
            wire.request_id = None;
            evidence.trace.wire.push(wire);
        }
        evidence.trace.frames = (7..=11)
            .map(|tick| {
                let mut frame = observed_frame(tick, true, if tick >= 8 { 2 } else { 1 });
                frame.iface_generation = u64::from(tick >= 8);
                frame.animation = Some(client::client::client::LocalAnimationUpdate {
                    sequence: if (9..=10).contains(&tick) {
                        FLETCHING_ANIMATION
                    } else {
                        ROLL_ANIMATION
                    },
                    serial: if tick < 9 {
                        1
                    } else if tick < 11 {
                        2
                    } else {
                        3
                    },
                });
                frame
            })
            .collect();
        evidence
    }

    fn iron_success_opportunities(count: u32) -> TickEngineTrace {
        let mut trace = prefix_evidence().trace;
        trace.frames.clear();
        for index in 0..count {
            let tick = 10 + index * 4;
            let mut frame = observed_frame(tick, true, 1);
            frame.animation = Some(client::client::client::LocalAnimationUpdate {
                sequence: -1,
                serial: index + 1,
            });
            frame.chat.push(ChatObservation {
                game_tick: tick,
                sequence: i32::try_from(index).unwrap(),
                text: "You manage to mine some iron.".into(),
                poll: PollKind::Edge,
            });
            trace.frames.push(frame);
        }
        trace
    }

    fn late_iron_evidence() -> RunEvidence {
        let mut evidence = prefix_evidence();
        let mut receipt = interact::server_ordered::ServerOrderedDispatch::refused(
            2,
            10,
            3,
            0,
            interact::server_ordered::ServerOrderedError::Cancelled,
        );
        receipt.dispatched_prefix = 3;
        receipt.wire_events = 4;
        receipt.terminal = None;
        for (row, wire_events) in receipt.rows[..3].iter_mut().zip([1, 1, 2]) {
            *row = interact::server_ordered::RowDispatch::HostQueued { wire_events };
        }
        evidence.trace.dispatches.push(DispatchObservation {
            game_tick: 10,
            receipt,
            request_debug: Some("KnifeArm".into()),
            mutation_applied: false,
        });
        evidence.trace.frames = (9..=15)
            .map(|tick| {
                let mut frame = observed_frame(tick, true, 1);
                if tick >= 10 {
                    frame.animation = Some(client::client::client::LocalAnimationUpdate {
                        sequence: if (12..15).contains(&tick) {
                            FLETCHING_ANIMATION
                        } else {
                            MITHRIL_PICK_ANIMATION
                        },
                        serial: if tick < 12 {
                            1
                        } else if tick < 15 {
                            2
                        } else {
                            3
                        },
                    });
                }
                if tick == 12 {
                    frame.chat.push(ChatObservation {
                        game_tick: 12,
                        sequence: 1,
                        text: "You swing your pick at the rock.".into(),
                        poll: PollKind::Edge,
                    });
                }
                frame
            })
            .collect();
        for opcode in [
            ClientProt289::MOVE_OPCLICK.id as u8,
            ClientProt289::OPLOC1.id as u8,
        ] {
            let mut packet = wire(opcode, 2, 7);
            packet.host_frame = 120;
            packet.game_tick = 12;
            packet.request_id = None;
            evidence.trace.wire.push(packet);
        }
        let before = &mut evidence.statuses[0];
        before.game_tick = 11;
        before.host_frame = 110;
        before.tick_cycle = Some(3);
        before.tick_verified = Some(4);
        before.tick_slips = Some(1);
        let mut after = before.clone();
        after.game_tick = 15;
        after.host_frame = 150;
        after.tick_slips = Some(2);
        evidence.statuses.push(after);
        evidence
    }

    #[test]
    fn iron_precondition_reports_thirty_two_successes_as_a_chance_outcome() {
        let trace = iron_success_opportunities(32);
        assert_eq!(
            iron_precondition(&trace, None, 8, 138),
            IronPrecondition::NoEligibleFailure { opportunities: 32 }
        );
    }

    #[test]
    fn iron_precondition_guard_counts_observed_rolls_not_elapsed_edges() {
        for opportunities in [0, 1, 31] {
            let trace = iron_success_opportunities(opportunities);
            assert_eq!(
                iron_precondition(&trace, None, 8, 608),
                IronPrecondition::GuardExhausted {
                    opportunities: opportunities as usize
                }
            );
        }
    }

    #[test]
    fn late_iron_oracle_accepts_selected_failed_roll_and_valid_late_history() {
        let evidence = late_iron_evidence();
        assert_eq!(
            iron_precondition(&evidence.trace, Some(2), 8, 15),
            IronPrecondition::Selected {
                request_id: 2,
                roll_tick: 10,
                opportunities: 1
            }
        );
        assert!(validate_late_iron_history(&evidence.trace, &evidence.statuses, 10).is_ok());
    }

    #[test]
    fn late_iron_acceptance_failure_is_not_a_chance_or_precondition_result() {
        for facet in 0..6 {
            let mut evidence = late_iron_evidence();
            match facet {
                0 => {
                    evidence
                        .trace
                        .frames
                        .last_mut()
                        .unwrap()
                        .animation
                        .as_mut()
                        .unwrap()
                        .sequence = ROLL_ANIMATION
                }
                1 => evidence.trace.wire.retain(|wire| {
                    wire.game_tick != 12 || wire.opcode != ClientProt289::OPLOC1.id as u8
                }),
                2 => evidence.trace.frames[3].chat.clear(),
                3 => evidence.statuses.last_mut().unwrap().tick_verified = Some(5),
                4 => {
                    let mut early = evidence.trace.dispatches[1].clone();
                    early.game_tick = 12;
                    evidence.trace.dispatches.push(early);
                }
                5 => evidence.trace.frames.last_mut().unwrap().log_count = 0,
                _ => unreachable!(),
            }
            assert!(
                validate_late_iron_history(&evidence.trace, &evidence.statuses, 10).is_err(),
                "late-acceptance facet {facet}"
            );
        }
    }

    #[test]
    fn natural_mining_roll_gate_rejects_start_acts_stale_frames_and_successes() {
        use host_play::tick_engine_test_support::fresh_mining_roll;
        let mut evidence = stale_guard_evidence();
        evidence.trace.frames[0].chat.push(ChatObservation {
            game_tick: 7,
            sequence: 7,
            text: "You swing your pick at the rock.".into(),
            poll: PollKind::Edge,
        });
        evidence.trace.dispatches.clear();
        evidence.trace.frames[4]
            .animation
            .as_mut()
            .unwrap()
            .sequence = MITHRIL_PICK_ANIMATION;
        assert!(fresh_mining_roll(
            &evidence.trace,
            None,
            11,
            MITHRIL_PICK_ANIMATION
        ));
        let mut before = evidence.trace.clone();
        let pending = before.frames.pop().unwrap();
        assert!(!fresh_mining_roll(
            &before,
            None,
            11,
            MITHRIL_PICK_ANIMATION
        ));
        assert!(fresh_mining_roll(
            &before,
            Some(&pending),
            11,
            MITHRIL_PICK_ANIMATION
        ));
        assert_eq!(
            before.frames.len(),
            4,
            "selection does not publish an incomplete frame"
        );
        let mut stale_pending = pending.clone();
        stale_pending.tick_edge = false;
        assert!(!fresh_mining_roll(
            &before,
            Some(&stale_pending),
            11,
            MITHRIL_PICK_ANIMATION
        ));
        let mut retained = evidence.trace.clone();
        let mut wake = retained.frames.last().unwrap().clone();
        wake.host_frame += 1;
        wake.tick_edge = false;
        retained.frames.push(wake);
        assert!(fresh_mining_roll(
            &retained,
            None,
            11,
            MITHRIL_PICK_ANIMATION
        ));
        retained
            .frames
            .last_mut()
            .unwrap()
            .animation
            .as_mut()
            .unwrap()
            .serial += 1;
        assert!(!fresh_mining_roll(
            &retained,
            None,
            11,
            MITHRIL_PICK_ANIMATION
        ));
        let mut start_act = evidence.trace.clone();
        start_act.frames[4].chat.push(ChatObservation {
            game_tick: 11,
            sequence: 11,
            text: "You swing your pick at the rock.".into(),
            poll: PollKind::Edge,
        });
        assert!(
            !fresh_mining_roll(&start_act, None, 11, MITHRIL_PICK_ANIMATION),
            "a fresh Mining animation with a start-Act line is not a natural roll"
        );
        for facet in 0..5 {
            let mut trace = evidence.trace.clone();
            match facet {
                0 => trace.frames[4].tick_edge = false,
                1 => trace.frames[4].animation.as_mut().unwrap().serial = 2,
                2 => trace.frames[4].animation.as_mut().unwrap().sequence = -1,
                3 => trace.frames[0].chat.clear(),
                4 => trace.frames[4].chat.push(ChatObservation {
                    game_tick: 11,
                    sequence: 11,
                    text: "You manage to mine some iron.".into(),
                    poll: PollKind::Edge,
                }),
                _ => unreachable!(),
            }
            assert!(
                !fresh_mining_roll(&trace, None, 11, MITHRIL_PICK_ANIMATION),
                "invalid facet {facet}"
            );
        }
    }

    #[test]
    fn iron_roll_accepts_real_pick_or_fast_rock_success_not_fishing_or_a_bare_null() {
        let mut evidence = stale_guard_evidence();
        evidence.trace.frames[4]
            .animation
            .as_mut()
            .unwrap()
            .sequence = MITHRIL_PICK_ANIMATION;
        assert!(iron_roll_on_edge(&evidence.trace, 11));
        evidence.trace.frames[4]
            .animation
            .as_mut()
            .unwrap()
            .sequence = ROLL_ANIMATION;
        assert!(!iron_roll_on_edge(&evidence.trace, 11));
        evidence.trace.frames[4]
            .animation
            .as_mut()
            .unwrap()
            .sequence = -1;
        assert!(!iron_roll_on_edge(&evidence.trace, 11));
        evidence.trace.frames[4].chat.push(ChatObservation {
            game_tick: 11,
            sequence: 11,
            text: "You manage to mine some iron.".into(),
            poll: PollKind::Edge,
        });
        assert!(iron_roll_on_edge(&evidence.trace, 11));
        evidence.trace.frames[4].game_tick = 12;
        assert!(!iron_roll_on_edge(&evidence.trace, 11));
    }

    #[test]
    fn stale_guard_requires_the_five_real_events_and_unchanged_native_receipt() {
        let evidence = stale_guard_evidence();
        assert_eq!(guard_carrier_user_events(&evidence).unwrap(), 5);
        let mut partial = evidence.clone();
        partial.trace.wire.remove(0);
        assert!(guard_carrier_user_events(&partial).is_err());
        let mut forged = evidence.clone();
        forged.trace.dispatches[0].receipt.wire_events = 5;
        assert!(guard_carrier_user_events(&forged).is_err());
        let mut duplicate = evidence;
        duplicate.trace.wire.insert(0, duplicate.trace.wire[0]);
        assert!(guard_carrier_user_events(&duplicate).is_err());
    }

    #[test]
    fn stale_guard_landing_preserves_every_required_observable() {
        let evidence = stale_guard_evidence();
        let baseline = evidence.baseline.as_ref().unwrap();
        assert!(stale_guard_landing(&evidence.trace, 7, baseline));
        for facet in 0..11 {
            let mut trace = evidence.trace.clone();
            match facet {
                0 => trace
                    .frames
                    .iter_mut()
                    .for_each(|frame| frame.server_if_close_packets = 1),
                1 => trace
                    .frames
                    .iter_mut()
                    .for_each(|frame| frame.iface_generation = 0),
                2 => trace.frames[1].animation.as_mut().unwrap().serial = 99,
                3 => trace.frames[2].animation.as_mut().unwrap().sequence = CAST_ANIMATION,
                4 => {
                    let _ = trace.wire.pop();
                }
                5 => trace.frames[4].game_tick += 1,
                6 => trace.frames[2].log_count = 0,
                7 => trace.frames[2].chat.push(ChatObservation {
                    game_tick: 9,
                    sequence: 11,
                    text: "You attempt to catch a fish.".into(),
                    poll: PollKind::Edge,
                }),
                8 => {
                    let mut early = trace.dispatches[0].clone();
                    early.game_tick = 8;
                    trace.dispatches.push(early);
                }
                9 | 10 => {
                    let opcode = if facet == 9 {
                        ClientProt289::IF_BUTTON.id as u8
                    } else {
                        ClientProt289::CLOSE_MODAL.id as u8
                    };
                    let mut press = wire(opcode, 1, 3);
                    press.request_id = None;
                    trace.wire.push(press);
                }
                _ => unreachable!(),
            }
            assert!(
                !stale_guard_landing(&trace, 7, baseline),
                "missing facet {facet}"
            );
        }
    }

    #[test]
    fn stale_guard_engine_oracle_requires_one_tick_and_refused_terminal_packets() {
        let packets: Vec<_> = [200, 200, 86, 67, 10]
            .into_iter()
            .enumerate()
            .map(|(index, opcode)| {
                let mut packet = packet(opcode, index as u64 + 1);
                packet.handler_accepted = index < 3;
                packet.charged_user_event = index < 3;
                packet
            })
            .collect();
        assert_engine_stale_guard(&packets).unwrap();
        for index in 0..packets.len() {
            let mut wrong = packets.clone();
            wrong[index].handler_accepted = !wrong[index].handler_accepted;
            assert!(assert_engine_stale_guard(&wrong).is_err());
        }
        let mut held = packets;
        held[1..].iter_mut().for_each(|packet| packet.tick += 1);
        assert!(assert_engine_stale_guard(&held).is_err());
    }

    #[test]
    fn normalized_prefix_attributes_boundary_chat_before_comparing() {
        let mut absent = prefix_evidence();
        absent.trace.frames.push(observed_frame(8, false, 1));
        let mut present = absent.clone();
        present
            .trace
            .frames
            .last_mut()
            .unwrap()
            .chat
            .push(ChatObservation {
                game_tick: 8,
                sequence: 1,
                text: "You swing your pick at the rock.".into(),
                poll: PollKind::Wake,
            });
        assert_eq!(
            normalized_prefix(&absent, 2),
            normalized_prefix(&present, 2)
        );
        let later = normalized_prefix(&present, 3);
        assert!(later.frames.iter().any(|frame| {
            frame
                .chat
                .iter()
                .any(|(text, _)| text == "You swing your pick at the rock.")
        }));
        assert_ne!(normalized_prefix(&absent, 3), later);
    }

    #[test]
    fn normalized_prefix_still_rejects_earlier_control_chat_differences() {
        let absent = prefix_evidence();
        for poll in [PollKind::Edge, PollKind::Wake] {
            let mut present = absent.clone();
            present.trace.frames[1].chat.push(ChatObservation {
                game_tick: 7,
                sequence: 1,
                text: "You swing your pick at the rock.".into(),
                poll,
            });
            assert!(
                prefix_difference(
                    &normalized_prefix(&absent, 2),
                    &normalized_prefix(&present, 2)
                )
                .is_some(),
                "poll={poll:?}"
            );
        }
    }

    #[test]
    fn normalized_prefix_ignores_only_unchanged_duplicate_polls() {
        let original = prefix_evidence();
        let mut repeated = original.clone();
        repeated.trace.frames.push(repeated.trace.frames[1].clone());
        repeated.trace.frames.last_mut().unwrap().host_frame += 1;
        repeated.statuses.push(repeated.statuses[0].clone());
        repeated.statuses.last_mut().unwrap().host_frame += 1;
        assert_eq!(
            normalized_prefix(&original, 2),
            normalized_prefix(&repeated, 2)
        );
    }

    #[test]
    fn normalized_control_trace_ignores_only_automatic_cyclelogic7_upkeep() {
        let original = prefix_evidence();
        let reference = normalized_prefix(&original, 3);
        let mut upkeep = original.clone();
        let mut packet = wire(ClientProt289::ANTICHEAT_CYCLELOGIC7.id as u8, 1, 1);
        packet.request_id = None;
        packet.game_tick = 8;
        upkeep.trace.wire.push(packet);
        assert_eq!(reference, normalized_prefix(&upkeep, 3));
        upkeep.trace.wire.last_mut().unwrap().game_tick = 9;
        assert_eq!(reference, normalized_prefix(&upkeep, 3));
        let mut action = original;
        action.trace.wire[0].game_tick = 8;
        assert_ne!(
            reference,
            normalized_prefix(&action, 3),
            "host action moved"
        );
    }

    #[test]
    fn normalized_prefix_preserves_edges_state_changes_and_every_emission() {
        let original = prefix_evidence();
        let reference = normalized_prefix(&original, 2);
        let mut changed = original.clone();
        changed.trace.frames[0].tick_edge = false;
        assert_ne!(
            reference,
            normalized_prefix(&changed, 2),
            "edge disappeared"
        );
        let mut changed = original.clone();
        let mut frame = changed.trace.frames[1].clone();
        frame.server_if_close_packets += 1;
        changed.trace.frames.push(frame);
        assert_ne!(
            reference,
            normalized_prefix(&changed, 2),
            "IF_CLOSE disappeared"
        );
        let mut changed = original.clone();
        let mut status = changed.statuses[0].clone();
        status.tick_slips = Some(1);
        changed.statuses.push(status);
        assert_ne!(
            reference,
            normalized_prefix(&changed, 2),
            "verdict disappeared"
        );
        let mut changed = original;
        changed.trace.wire.push(changed.trace.wire[0]);
        assert_ne!(
            reference,
            normalized_prefix(&changed, 2),
            "duplicate packet disappeared"
        );
    }

    #[test]
    fn normalized_prefix_ignores_chance_yield_but_preserves_control_and_fletching() {
        let original = prefix_evidence();
        let reference = normalized_prefix(&original, 2);
        let mut caught = original.clone();
        caught.trace.frames[1].inventory_counts.push((335, 1));
        caught.trace.frames[1].skill_xp.push((10, 50));
        caught.trace.frames[1].chat.push(ChatObservation {
            game_tick: 7,
            sequence: 11,
            text: "You catch a trout.".into(),
            poll: PollKind::Wake,
        });
        assert_eq!(reference, normalized_prefix(&caught, 2));
        caught.trace.frames[1].chat[0].text = "You attempt to catch a fish.".into();
        assert_ne!(
            reference,
            normalized_prefix(&caught, 2),
            "act chat disappeared"
        );
        let mut changed = original.clone();
        changed.trace.frames[1].skill_xp.push((9, 1));
        assert_ne!(
            reference,
            normalized_prefix(&changed, 2),
            "Fletching XP disappeared"
        );
        let mut changed = original;
        changed.trace.frames[1].log_count = 0;
        assert_ne!(
            reference,
            normalized_prefix(&changed, 2),
            "log use disappeared"
        );
    }

    #[test]
    fn chat_attribution_uses_first_seen_poll_position() {
        for poll in [PollKind::Wake, PollKind::Edge] {
            let mut frame = observed_frame(15, poll == PollKind::Edge, 1);
            frame.chat.push(ChatObservation {
                game_tick: 15,
                sequence: 11,
                text: "You attempt to catch a fish.".into(),
                poll,
            });
            let trace = TickEngineTrace {
                frames: vec![frame],
                ..TickEngineTrace::default()
            };
            assert!(has_act_line(&trace, 16), "{poll:?}");
            assert_eq!(has_act_line(&trace, 15), poll == PollKind::Edge);
            assert!(!has_act_line(&trace, 14));
            assert!(!has_act_line(&trace, 17));
        }
    }

    #[test]
    fn chat_only_if_close_needs_a_fresh_packet_not_a_main_modal_transition() {
        let baseline = observed_frame(15, false, 1);
        let trace = TickEngineTrace {
            frames: vec![
                baseline.clone(),
                observed_frame(16, false, 2),
                observed_frame(17, true, 2),
            ],
            ..TickEngineTrace::default()
        };
        assert!(close_packet_seen(&trace, 16, &baseline));
        assert!(!close_packet_seen(&trace, 15, &baseline));
        assert!(!close_packet_seen(&trace, 17, &baseline));

        let mut transition_without_packet = observed_frame(16, false, 1);
        transition_without_packet.main_modal_closed_observation = 1;
        let trace = TickEngineTrace {
            frames: vec![baseline.clone(), transition_without_packet],
            ..TickEngineTrace::default()
        };
        assert!(!close_packet_seen(&trace, 16, &baseline));
    }

    #[test]
    fn if_close_first_seen_on_an_edge_keeps_its_two_tick_interval() {
        let baseline = observed_frame(15, false, 1);
        let trace = TickEngineTrace {
            frames: vec![baseline.clone(), observed_frame(16, true, 2)],
            ..TickEngineTrace::default()
        };
        assert!(close_packet_seen(&trace, 15, &baseline));
        assert!(close_packet_seen(&trace, 16, &baseline));
        assert!(!close_packet_seen(&trace, 14, &baseline));
        assert!(!close_packet_seen(&trace, 17, &baseline));
    }

    #[test]
    fn late_iron_scenario_skips_the_first_arm_wait_but_keeps_knife_method() {
        let scenario =
            Scenario::iron("tick-late-iron").expect("iron setup uses the default mining tile");
        assert!(!scenario.require_knife_arm);
        assert_eq!(scenario.settings()["tickMethod"], json!("Knife"));
        // pickaxes.obj:162-164 permits the real four-tick mithril tool at 21.
        // The late case needs a failed native roll, not S6b's high-yield setup.
        assert_eq!(scenario.level, 21);
        assert_eq!(scenario.tool.id, MITHRIL_PICKAXE_ID);
    }

    #[test]
    fn river_setup_starts_on_a_real_standable_tile() {
        if !api::test_support::game_data_available() {
            return;
        }
        let Some(pack) = api::test_support::local_asset_path(
            "NAV_PACK",
            "",
            "real 289 navigation pack for tick-engine river setup",
        ) else {
            return;
        };
        let world = nav_planner::world::NavWorld::load_pack(&pack).unwrap();
        let scenario = Scenario::river("tick-setup");
        let (_, target) = scenario.required_loc.expect("river has a fixed loc");
        let data = api::game_data::for_revision(client::io::ClientRevision::R289).unwrap();
        let catalog =
            api::selected::FamilyPreparation::run(move |worker| data.prepare_gathering(worker))
                .unwrap()
                .join()
                .unwrap()
                .unwrap();
        let api::selected::Knowledge::Known(spots) =
            &catalog.method("fishing.loc_2027.op1").unwrap().spots
        else {
            panic!("river fixture needs complete fishing placement data");
        };
        assert!(spots.iter().any(|spot| {
            spot.entity == api::selected::EntityId::Loc(RIVER_LOC_ID) && spot.origin == target
        }));
        assert!(world.collision.standable(scenario.target));
        assert_ne!(scenario.target, target);
        assert_eq!(scenario.target.level, target.level);
        assert_eq!(
            (scenario.target.x - target.x)
                .abs()
                .max((scenario.target.z - target.z).abs()),
            1
        );
    }

    #[test]
    fn oak_setup_starts_on_a_real_standable_tile() {
        let Some(pack) = api::test_support::local_asset_path(
            "NAV_PACK",
            "",
            "real 289 navigation pack for tick-engine oak setup",
        ) else {
            return;
        };
        let world = nav_planner::world::NavWorld::load_pack(&pack).unwrap();
        let scenario = Scenario::oak("tick-setup");
        let (_, target) = scenario.required_loc.expect("oak has a fixed loc");
        assert!(world.collision.standable(scenario.target));
        assert_ne!(scenario.target, target);
        assert_eq!(scenario.target.level, target.level);
        assert_eq!(
            (scenario.target.x - target.x)
                .abs()
                .max((scenario.target.z - target.z).abs()),
            1
        );
    }

    #[test]
    fn copper_replacements_share_a_real_standable_cardinal_stand() {
        if !api::test_support::game_data_available() {
            return;
        }
        let Some(pack) = api::test_support::local_asset_path(
            "NAV_PACK",
            "",
            "real 289 navigation pack for tick-engine copper setup",
        ) else {
            return;
        };
        let world = nav_planner::world::NavWorld::load_pack(&pack).unwrap();
        let scenario = Scenario::copper("tick-setup").unwrap();
        let first = scenario.required_loc.unwrap().1;
        let second = tile(3_287, 3_365, 0);
        let data = api::game_data::for_revision(client::io::ClientRevision::R289).unwrap();
        let catalog =
            api::selected::FamilyPreparation::run(move |worker| data.prepare_gathering(worker))
                .unwrap()
                .join()
                .unwrap()
                .unwrap();
        assert!(world.collision.standable(scenario.target));
        for target in [first, second] {
            assert_eq!(scenario.target.level, target.level);
            assert_eq!(
                (scenario.target.x - target.x).abs() + (scenario.target.z - target.z).abs(),
                1
            );
            assert!(catalog.methods().iter().any(|method| match &method.spots {
                api::selected::Knowledge::Known(spots) => spots.iter().any(|spot| {
                    spot.entity == api::selected::EntityId::Loc(2_090) && spot.origin == target
                }),
                _ => false,
            }));
        }
    }

    #[test]
    fn setup_observes_relogin_without_requiring_an_offline_callback() {
        let account = "tick-setup";
        let mut setup = SetupState::new(account.into(), Scenario::river("tick-setup"));
        setup.phase = SetupPhase::WaitRelogin;
        setup.logout_session = Some(10);
        let mut client = Client::new_with_revision(
            client::client::ClientConfig {
                host: "127.0.0.1".into(),
                port: 1,
                cache_dir: String::new(),
                members: true,
                lowmem: true,
            },
            client::client::ClientRevision::R289,
        );
        client.ingame = true;
        client.scene_state = 2;
        client.gens.session = 10;
        setup.frame(&mut client, account, false);
        assert_eq!(setup.phase, SetupPhase::WaitRelogin);
        client.gens.session = 11;
        setup.frame(&mut client, account, true);
        assert_eq!(setup.phase, SetupPhase::WaitRelogin);
        setup.frame(&mut client, account, false);
        assert_eq!(setup.phase, SetupPhase::ReseedTutorial);
        assert!(setup.tutorial_reseed.is_none());
    }

    fn packet(opcode: u8, sequence: u64) -> EnginePacket {
        EnginePacket {
            sequence,
            tick: 7,
            username: "tmx-oracle".into(),
            opcode,
            name: format!("opcode-{opcode}"),
            outcome: "accepted".into(),
            reason: String::new(),
            handler_accepted: true,
            charged_user_event: false,
            scripts: Vec::new(),
        }
    }

    #[test]
    fn lone_opener_restart_follows_the_expired_native_delay_after_its_close() {
        let baseline = observed_frame(15, true, 1);
        let close = observed_frame(16, false, 2);
        let mut restart = observed_frame(18, false, 2);
        restart.chat.push(ChatObservation {
            game_tick: 18,
            sequence: 12,
            text: "You attempt to catch a fish.".into(),
            poll: PollKind::Wake,
        });
        let mut movement = wire(ClientProt289::MOVE_OPCLICK.id as u8, 1, 7);
        movement.request_id = None;
        movement.game_tick = 16;
        let mut target = wire(ClientProt289::OPLOC1.id as u8, 1, 7);
        target.request_id = None;
        target.game_tick = 16;
        let mut trace = TickEngineTrace {
            frames: vec![close, restart],
            wire: vec![movement, target],
            ..TickEngineTrace::default()
        };
        assert!(
            lone_opener_continues(&trace, 15, &baseline),
            "IF_CLOSE belongs to the first-budget click; the native act follows its own delay"
        );
        trace.frames[1].chat[0].game_tick = 19;
        assert!(!lone_opener_continues(&trace, 15, &baseline));
        trace.frames[1].chat[0].game_tick = 18;
        trace.frames[0].game_tick = 17;
        assert!(!lone_opener_continues(&trace, 15, &baseline));
    }

    #[test]
    fn make_prefix_counts_a_charged_opener_whose_dialogue_script_is_suspended() {
        let mut opener = packet(ClientProt289::OPHELDU.id as u8, 1);
        opener.outcome = "deferred".into();
        opener.reason = "script-suspended".into();
        opener.charged_user_event = true;
        opener.scripts.push(engine_packet_log::EngineScript {
            outcome: "deferred".into(),
            reason: "script-suspended".into(),
            script: "[proc,skill_multi3]".into(),
            state: 3,
        });
        let mut make = packet(ClientProt289::IF_BUTTON.id as u8, 2);
        make.charged_user_event = true;
        let mut packets = [opener, make];
        assert_engine_accepted_make_prefix(&packets)
            .expect("the network accepted and charged both packets in one engine tick");
        packets[0].handler_accepted = false;
        assert!(assert_engine_accepted_make_prefix(&packets).is_err());
        packets[0].handler_accepted = true;
        packets[0].charged_user_event = false;
        assert!(assert_engine_accepted_make_prefix(&packets).is_err());
        packets[0].charged_user_event = true;
        packets[1].tick += 1;
        assert!(assert_engine_accepted_make_prefix(&packets).is_err());
    }

    #[test]
    fn deferred_terminal_pair_is_decoded_only_after_the_five_event_tick() {
        let opcodes = [234, 234, 234, 200, 86, 67, 10];
        let mut packets: Vec<_> = opcodes
            .into_iter()
            .enumerate()
            .map(|(index, opcode)| {
                let mut packet = packet(opcode, index as u64 + 1);
                packet.charged_user_event = true;
                packet.tick += u64::from(index >= 5);
                packet
            })
            .collect();
        packets[3].outcome = "deferred".into();
        packets[3].reason = "script-suspended".into();
        let mut evidence = evidence(vec![], vec![], vec![]);
        evidence.pre_batch_user_events.requested = 3;
        assert_deferred_terminal_row(&evidence, &packets).unwrap();
        for index in 0..packets.len() {
            let mut refused = packets.clone();
            refused[index].handler_accepted = false;
            assert!(assert_deferred_terminal_row(&evidence, &refused).is_err());
        }
        let mut early = packets.clone();
        early[5].tick -= 1;
        early[6].tick -= 1;
        early[5].outcome = "deferred".into();
        early[6].outcome = "deferred".into();
        assert!(assert_deferred_terminal_row(&evidence, &early).is_err());
        let mut late = packets.clone();
        late[5].tick += 1;
        late[6].tick += 1;
        assert!(assert_deferred_terminal_row(&evidence, &late).is_err());
        packets.swap(5, 6);
        assert!(assert_deferred_terminal_row(&evidence, &packets).is_err());
    }

    #[test]
    fn ordinary_click_pairs_require_one_real_client_frame_without_an_ordered_receipt() {
        let mut movement = wire(ClientProt289::MOVE_OPCLICK.id as u8, 11, 7);
        movement.request_id = None;
        let mut target = wire(ClientProt289::OPLOC1.id as u8, 11, 7);
        target.request_id = None;
        let mut trace = TickEngineTrace {
            wire: vec![movement, target],
            ..TickEngineTrace::default()
        };
        assert!(trace.dispatches.is_empty());
        assert_eq!(checked_click_pairs(&trace, 7), 1);
        trace.wire[1].host_frame += 1;
        assert_eq!(
            checked_click_pairs(&trace, 7),
            0,
            "packets from separate Client frames are not one ordinary click"
        );
        trace.wire[1].host_frame = trace.wire[0].host_frame;
        trace.wire[1].request_id = Some(11);
        assert_eq!(
            checked_click_pairs(&trace, 7),
            0,
            "an ordered packet cannot complete an ordinary click"
        );
    }

    #[test]
    fn three_stationary_walks_use_the_real_encrypted_client_encoder() {
        let mut client = Client::new_with_revision(
            client::client::ClientConfig {
                host: "127.0.0.1".into(),
                port: 1,
                cache_dir: String::new(),
                members: true,
                lowmem: true,
            },
            client::client::ClientRevision::R289,
        );
        let seed = [1, 2, 3, 4];
        client.out.random = Some(client::io::Isaac::new(&seed));
        client.map_build_base_x = 3_200;
        client.map_build_base_z = 3_200;
        for _ in 0..3 {
            assert!(client.tryMove(5, 5, 5, 5, false, 0, 0, 0, 0, 0, 0));
        }
        let expected = api::prot::Send::move_gameclick(false, 3_205, 3_205, &[]).unwrap();
        let bytes = &client.out.data()[..client.out.pos];
        let mut cipher = client::io::Isaac::new(&seed);
        let mut packets = bytes.chunks_exact(expected.body().len() + 2);
        for _ in 0..3 {
            let packet = packets.next().expect("three real movement packets");
            assert_eq!(
                packet[0].wrapping_sub(cipher.next_int() as u8),
                ClientProt289::MOVE_GAMECLICK.id as u8
            );
            assert_eq!(usize::from(packet[1]), expected.body().len());
            assert_eq!(&packet[2..], expected.body());
        }
        assert!(packets.next().is_none());
        assert!(packets.remainder().is_empty());
    }

    fn wire(opcode: u8, request_id: u64, length: usize) -> WireObservation {
        WireObservation {
            host_frame: 1,
            game_tick: 7,
            request_id: Some(request_id),
            opcode,
            encoded_length: length,
        }
    }

    fn evidence(
        wire: Vec<WireObservation>,
        transport: Vec<ProxyObservation>,
        packets: Vec<EnginePacket>,
    ) -> RunEvidence {
        RunEvidence {
            case: "packet-oracle",
            account: "tmx-oracle".into(),
            profile: "test".into(),
            world_members: "unknown".into(),
            product_ids: Vec::new(),
            pre_batch_user_events: ForeignEventState::default(),
            pre_batch_guard_opener: GuardOpenerState::default(),
            start_tick: Some(7),
            baseline: None,
            npc_observations: Vec::new(),
            trace: TickEngineTrace {
                frames: Vec::new(),
                dispatches: Vec::new(),
                wire,
            },
            statuses: Vec::new(),
            drop_history: None,
            transport,
            engine_packets: Some(Ok(packets)),
        }
    }

    fn drop_observation(
        request_id: u64,
        packet_opcodes: Vec<u8>,
        packet_lengths: Vec<usize>,
        forwarded_bytes: usize,
        dropped_bytes: usize,
    ) -> ProxyObservation {
        ProxyObservation {
            request_id,
            request_ids: vec![request_id],
            mutation: "DropSuffixAfterPacket(2)".into(),
            start_game_tick: 7,
            start_host_frame: 1,
            packet_requests: (1..=packet_opcodes.len())
                .map(|packet_index| (request_id, packet_index))
                .collect(),
            packet_opcodes,
            packet_lengths,
            forwarded_bytes,
            dropped_bytes,
            release_edge_tick: None,
            release_trigger_tick: None,
            release_edge_frame: None,
            released_host_frame: None,
        }
    }

    fn drop_status(host_frame: u64, tick_verified: i64) -> StatusObservation {
        StatusObservation {
            host_frame,
            game_tick: 7,
            phase: "Working".into(),
            method: Some("loc_2027".into()),
            tick_method: Some("Knife".into()),
            target: Some("loc_2027".into()),
            last_event: Some("gathering".into()),
            tick_cycle: Some(-1),
            tick_verified: Some(tick_verified),
            tick_unobserved: Some(0),
            tick_slips: Some(0),
            tick_lost: Some(0),
            tick_delayed: Some(0),
            tick_paused: Some(0),
            tick_fallbacks: Some(0),
            tick_fallback_failed: Some(0),
            tick_events: Some(0),
        }
    }

    fn suffix_drop_evidence(request_id: u64) -> RunEvidence {
        let opcodes = vec![
            ClientProt289::OPHELDU.id as u8,
            ClientProt289::IF_BUTTON.id as u8,
            ClientProt289::MOVE_OPCLICK.id as u8,
            ClientProt289::OPLOC1.id as u8,
        ];
        let lengths = vec![13, 3, 7, 7];
        let wire_packets = opcodes
            .iter()
            .zip(&lengths)
            .map(|(opcode, length)| wire(*opcode, request_id, *length))
            .collect();
        let mut opener = packet(opcodes[0], 1);
        opener.charged_user_event = true;
        let mut make = packet(opcodes[1], 2);
        make.charged_user_event = true;
        let mut unknown = packet(54, 3);
        unknown.tick = 8;
        unknown.name = "UNKNOWN".into();
        unknown.outcome = "refused".into();
        unknown.reason = "unknown-opcode".into();
        unknown.handler_accepted = false;
        let mut evidence = evidence(
            wire_packets,
            vec![drop_observation(request_id, opcodes, lengths, 16, 14)],
            vec![opener, make, unknown],
        );
        evidence.case = "tick-make-proxy-drop";

        let mut receipt = interact::server_ordered::ServerOrderedDispatch::refused(
            request_id,
            7,
            3,
            0,
            interact::server_ordered::ServerOrderedError::Cancelled,
        );
        receipt.rows[0] = interact::server_ordered::RowDispatch::HostQueued { wire_events: 1 };
        receipt.rows[1] = interact::server_ordered::RowDispatch::HostQueued { wire_events: 1 };
        receipt.rows[2] = interact::server_ordered::RowDispatch::HostQueued { wire_events: 2 };
        receipt.dispatched_prefix = 3;
        receipt.wire_events = 4;
        receipt.terminal = None;
        evidence.trace.dispatches.push(DispatchObservation {
            game_tick: 7,
            receipt,
            request_debug: Some("KnifeArm".into()),
            mutation_applied: false,
        });
        evidence.statuses = vec![drop_status(1, 0), drop_status(2, 0)];
        evidence.drop_history = Some(drop_history::DropHistory {
            client_sessions: vec![
                drop_history::ClientSessionObservation {
                    host_frame: 1,
                    session: 100,
                    ingame: true,
                    scene_state: 2,
                },
                drop_history::ClientSessionObservation {
                    host_frame: 3,
                    session: 101,
                    ingame: true,
                    scene_state: 2,
                },
            ],
            slot_status: vec![
                drop_history::SlotObservation {
                    host_frame: 1,
                    connected: true,
                    ingame: true,
                    scene_state: 2,
                    startup_phase: host_play::StartupPhase::Ready,
                    stream_epoch: 4,
                },
                drop_history::SlotObservation {
                    host_frame: 2,
                    connected: false,
                    ingame: false,
                    scene_state: 0,
                    startup_phase: host_play::StartupPhase::Connecting,
                    stream_epoch: 4,
                },
                drop_history::SlotObservation {
                    host_frame: 4,
                    connected: true,
                    ingame: true,
                    scene_state: 2,
                    startup_phase: host_play::StartupPhase::Ready,
                    stream_epoch: 5,
                },
            ],
            host_log: vec![
                drop_history::HostLogObservation {
                    slot: Some("tmx-oracle".into()),
                    tick: Some(7),
                    message: "session exit reason=read_eof".into(),
                },
                drop_history::HostLogObservation {
                    slot: Some("tmx-oracle".into()),
                    tick: Some(7),
                    message: "relog decision attempt reconnect=true".into(),
                },
                drop_history::HostLogObservation {
                    slot: Some("tmx-oracle".into()),
                    tick: Some(7),
                    message: "handshake ok".into(),
                },
            ],
        });
        evidence
    }

    #[test]
    fn engine_correlation_accepts_suffix_drop_only_at_unknown_opcode_boundary() {
        let evidence = suffix_drop_evidence(11);
        assert!(correlate_engine_packets(&evidence).is_ok());
    }

    #[test]
    fn engine_correlation_rejects_old_session_r_plus_three_continuation() {
        let mut evidence = suffix_drop_evidence(12);
        evidence.statuses[1].tick_verified = Some(1);
        let mut roll = observed_frame(10, true, 0);
        roll.host_frame = 2;
        roll.animation = Some(client::client::client::LocalAnimationUpdate {
            serial: 1,
            sequence: ROLL_ANIMATION,
        });
        evidence.trace.frames.push(roll);
        assert!(correlate_engine_packets(&evidence).is_err());
    }

    #[test]
    fn engine_correlation_rejects_disconnect_without_full_host_receipt() {
        let mut evidence = suffix_drop_evidence(13);
        evidence.trace.dispatches[0].receipt.wire_events = 3;
        assert!(correlate_engine_packets(&evidence).is_err());
    }

    #[test]
    fn engine_correlation_rejects_dropped_packets_reported_by_engine() {
        let request = 21;
        let opcodes = vec![
            ClientProt289::OPHELDU.id as u8,
            ClientProt289::IF_BUTTON.id as u8,
            ClientProt289::MOVE_OPCLICK.id as u8,
            ClientProt289::OPLOC1.id as u8,
        ];
        let lengths = vec![13, 3, 7, 7];
        let wire_packets = opcodes
            .iter()
            .zip(&lengths)
            .map(|(opcode, length)| wire(*opcode, request, *length))
            .collect();
        let mut opener = packet(opcodes[0], 1);
        opener.charged_user_event = true;
        let mut make = packet(opcodes[1], 2);
        make.charged_user_event = true;
        let engine_packets = vec![opener, make, packet(opcodes[2], 3), packet(opcodes[3], 4)];
        let transport = vec![drop_observation(request, opcodes, lengths, 16, 14)];
        let mut evidence = evidence(wire_packets, transport, engine_packets);
        evidence.case = "tick-make-proxy-drop";
        assert!(correlate_engine_packets(&evidence).is_err());
    }

    #[test]
    fn engine_correlation_rejects_proxy_dropped_packets_reported_by_engine() {
        let request = 21;
        let opcodes = vec![
            ClientProt289::OPHELDU.id as u8,
            ClientProt289::IF_BUTTON.id as u8,
            ClientProt289::MOVE_OPCLICK.id as u8,
            ClientProt289::OPLOC1.id as u8,
        ];
        let lengths = vec![6, 4, 7, 5];
        let wire_packets = opcodes
            .iter()
            .zip(&lengths)
            .map(|(opcode, length)| wire(*opcode, request, *length))
            .collect();
        let engine_packets = opcodes
            .iter()
            .enumerate()
            .map(|(index, opcode)| packet(*opcode, index as u64 + 1))
            .collect();
        let transport = vec![drop_observation(request, opcodes, lengths, 10, 12)];
        assert!(
            correlate_engine_packets(&evidence(wire_packets, transport, engine_packets,)).is_err()
        );
    }

    #[test]
    fn engine_correlation_rejects_non_packet_aligned_proxy_accounting() {
        let request = 31;
        let opcodes = vec![
            ClientProt289::OPHELDU.id as u8,
            ClientProt289::IF_BUTTON.id as u8,
            ClientProt289::MOVE_OPCLICK.id as u8,
            ClientProt289::OPLOC1.id as u8,
        ];
        let lengths = vec![6, 4, 7, 5];
        let wire_packets = opcodes
            .iter()
            .zip(&lengths)
            .map(|(opcode, length)| wire(*opcode, request, *length))
            .collect();
        let engine_packets = vec![packet(opcodes[0], 1), packet(opcodes[1], 2)];
        let transport = vec![drop_observation(request, opcodes, lengths, 11, 11)];
        let mut evidence = evidence(wire_packets, transport, engine_packets);
        evidence.case = "tick-make-proxy-drop";
        assert!(correlate_engine_packets(&evidence).is_err());
    }

    fn freshfish_dispatch_fixture() -> DispatchObservation {
        let mut receipt = interact::server_ordered::ServerOrderedDispatch::refused(
            41,
            7,
            3,
            0,
            interact::server_ordered::ServerOrderedError::Cancelled,
        );
        receipt.rows[0] = interact::server_ordered::RowDispatch::HostQueued { wire_events: 1 };
        receipt.rows[1] = interact::server_ordered::RowDispatch::HostQueued { wire_events: 1 };
        receipt.rows[2] = interact::server_ordered::RowDispatch::HostQueued { wire_events: 2 };
        receipt.dispatched_prefix = 3;
        receipt.wire_events = 4;
        receipt.terminal = None;
        DispatchObservation {
            game_tick: 7,
            receipt,
            request_debug: Some(
                "KnifeArm { shape: Continuing { click: Npc(ExactNpcReference { index: 3712, type_id: 309, tile: WorldTile { x: 2382, z: 3415, level: 0 }, npc_info_generation: 3, operation: 1 }) } }"
                    .into(),
            ),
            mutation_applied: false,
        }
    }

    fn accepted_freshfish_packets() -> Vec<EnginePacket> {
        NPC_KNIFE_WIRE_OPCODES
            .iter()
            .enumerate()
            .map(|(index, opcode)| {
                let mut row = packet(*opcode, index as u64 + 1);
                row.reason = "handler-accepted".into();
                row.charged_user_event = index < 2;
                row
            })
            .collect()
    }

    #[test]
    fn npc_river_scenario_uses_real_freshfish_and_seeds_only_the_player() {
        let scenario = Scenario::npc_river("tick-npc-knife");
        let settings = scenario.settings();
        assert_eq!(
            settings.get("fishingMethod").and_then(Value::as_str),
            Some("fishing.freshfish.op1")
        );
        assert_eq!(scenario.target, tile(2_382, 3_415, 0));
        assert_eq!(
            scenario.required_npc,
            Some((FRESHFISH_NPC_ID, tile(2_382, 3_414, 0)))
        );
        assert_eq!(scenario.required_loc, None);
        assert_eq!(scenario.products.as_slice(), &[TROUT_ID]);
        let setup = SetupState::new("tmx-oracle".into(), scenario);
        assert_eq!(
            setup.commands,
            vec![
                "setstat fishing 20".to_owned(),
                "give knife 1".to_owned(),
                "give logs 1".to_owned(),
                "give fly_fishing_rod 1".to_owned(),
                "give feather 200".to_owned(),
                interact::tele_args(0, 2_382, 3_415),
            ]
        );
    }

    #[test]
    fn network_coords_npc_observer_and_fixture_readiness_use_server_tiles() {
        let mut client = Client::new_with_revision(
            client::client::ClientConfig {
                host: "127.0.0.1".into(),
                port: 1,
                cache_dir: String::new(),
                members: true,
                lowmem: true,
            },
            client::client::ClientRevision::R289,
        );
        client.ingame = true;
        client.scene_state = 2;
        client.map_build_base_x = 2374;
        client.map_build_base_z = 3410;
        let mut npc = client::dash3d::ClientNpc::at(8, 5);
        npc.r#type = Some(309);
        // The displayed actor is still one tile behind its received path head.
        npc.entity.size = 1;
        npc.entity.x = 8 * 128 + 64;
        npc.entity.z = 4 * 128 + 64;
        client.npc[3712] = Some(Box::new(npc));
        client.npc_ids = vec![3712];
        client.npc_count = 1;
        client.gens.npc = 1;
        client.gens.npc_info = 3;
        let mut setup = SetupState::new("tmx-oracle".into(), Scenario::npc_river("tick-npc-knife"));
        // Seed the same retained session watermark as the host. The NPC
        // family itself is newer and must be republished from the Client.
        let mut watermark = client.gens;
        watermark.npc = 0;
        setup.npc_snapshot.reset_session(watermark);
        setup.observe_npc(&client);
        assert_eq!(setup.npc_snapshot.gens().npc_info, client.gens.npc_info);
        assert_eq!(setup.npc_snapshot.npcs()[0].tile, tile(2382, 3414, 0));
        let first =
            interact::server_ordered::ExactNpcReference::new(3712, 309, tile(2382, 3415, 0), 3, 1)
                .unwrap();
        assert_eq!(setup.npc_observations, vec![first]);
        for scenario in [
            Scenario::npc_river("network-freshfish"),
            Scenario::net("network-net"),
        ] {
            let mut ready = GameSnapshot::new();
            ready.seed_tile(scenario.target);
            ready.seed_inv(
                scenario
                    .seed_items()
                    .iter()
                    .map(|item| (item.id, item.count))
                    .collect(),
            );
            ready.seed_stats(vec![api::snapshot::StatView {
                index: scenario.skill_index,
                name: scenario.skill.into(),
                effective: scenario.level,
                base: scenario.level,
                xp: 0,
                used: true,
            }]);
            let mut npc = setup.npc_snapshot.npcs()[0].clone();
            npc.r#type = Some(if scenario.resource == ResourceKind::Net {
                330
            } else {
                309
            });
            npc.actions = vec![Some("Net".into())];
            npc.network = scenario.required_npc.map_or(
                WorldTile {
                    x: scenario.target.x + 1,
                    ..scenario.target
                },
                |(_, target)| target,
            );
            npc.tile = WorldTile {
                x: scenario.target.x + 10,
                ..scenario.target
            };
            ready.seed_npcs(vec![npc.clone()]);
            let mut ready_setup = SetupState::new("network-setup".into(), scenario);
            ready_setup.phase = SetupPhase::WaitNetPosition;
            assert!(ready_setup.ready_in_snapshot(&ready));
            std::mem::swap(&mut npc.network, &mut npc.tile);
            ready.seed_npcs(vec![npc]);
            assert!(!ready_setup.ready_in_snapshot(&ready));
        }
        setup.observe_npc(&client);
        assert_eq!(
            setup.npc_observations,
            vec![first],
            "no duplicate generation"
        );

        client.npc[3712].as_mut().unwrap().entity.route_z[0] = 6;
        client.gens.npc += 1;
        client.gens.npc_info += 1;
        setup.observe_npc(&client);
        let second = interact::server_ordered::ExactNpcReference::new(
            3712,
            309,
            tile(2382, 3416, 0),
            client.gens.npc_info,
            1,
        )
        .unwrap();
        assert_eq!(setup.npc_observations, vec![first, second]);

        client.npc[3712].as_mut().unwrap().r#type = Some(330);
        client.gens.npc += 1;
        client.gens.npc_info += 1;
        setup.observe_npc(&client);
        assert_eq!(
            setup.npc_observations,
            vec![first, second],
            "different type is not an observation"
        );
    }

    #[test]
    fn npc_knife_receipt_requires_exact_continuing_freshfish_operation() {
        let dispatch = freshfish_dispatch_fixture();
        let observed = [
            interact::server_ordered::ExactNpcReference::new(3712, 309, tile(2382, 3415, 0), 3, 1)
                .unwrap(),
            interact::server_ordered::ExactNpcReference::new(3714, 309, tile(2382, 3414, 0), 3, 1)
                .unwrap(),
        ];
        assert!(npc_freshfish_knife_dispatch_matches(&dispatch, &observed));

        // The real spawn's rendered tile is (2382,3414), but its observed
        // path-head network tile in the retained receipt is (2382,3415).
        // Changing only this request's tile or NPC must fail, even when that
        // field belongs to another observed row. The whole tuple must match.
        for (from, to) in [
            ("index: 3712", "index: 3713"),
            ("index: 3712", "index: 3714"),
            ("type_id: 309", "type_id: 330"),
            ("z: 3415", "z: 3414"),
            ("npc_info_generation: 3", "npc_info_generation: 2"),
            ("operation: 1", "operation: 3"),
            ("Continuing {", "Ending {"),
        ] {
            let mut wrong = dispatch.clone();
            wrong.request_debug = wrong.request_debug.map(|request| request.replace(from, to));
            assert!(
                !npc_freshfish_knife_dispatch_matches(&wrong, &observed),
                "{from} -> {to}"
            );
        }
        assert!(!npc_freshfish_knife_dispatch_matches(&dispatch, &[]));

        let mut partial = dispatch.clone();
        partial.receipt.wire_events = 3;
        assert!(!npc_freshfish_knife_dispatch_matches(&partial, &observed));

        let mut mutated = dispatch;
        mutated.mutation_applied = true;
        assert!(!npc_freshfish_knife_dispatch_matches(&mutated, &observed));
    }

    #[test]
    fn npc_wire_oracle_requires_mode_two_packet_sequence_and_exact_request() {
        let dispatch = freshfish_dispatch_fixture();
        assert_eq!(NPC_KNIFE_WIRE_OPCODES, [200, 86, 67, 252]);
        let trace = TickEngineTrace {
            wire: NPC_KNIFE_WIRE_OPCODES
                .iter()
                .map(|opcode| wire(*opcode, dispatch.receipt.request_id, 1))
                .collect(),
            ..TickEngineTrace::default()
        };
        assert!(npc_knife_wire_order_matches(&trace, &dispatch));

        for index in 0..trace.wire.len() {
            let mut wrong_opcode = trace.clone();
            wrong_opcode.wire[index].opcode = wrong_opcode.wire[index].opcode.wrapping_add(1);
            assert!(!npc_knife_wire_order_matches(&wrong_opcode, &dispatch));
        }
        let mut wrong_request = trace;
        wrong_request.wire[0].request_id = Some(dispatch.receipt.request_id + 1);
        assert!(!npc_knife_wire_order_matches(&wrong_request, &dispatch));
    }

    #[test]
    fn npc_fishing_success_requires_a_real_fish_or_xp_gain() {
        let mut baseline = observed_frame(7, true, 0);
        baseline.inventory_counts = vec![(TROUT_ID, 0)];
        baseline.skill_xp = vec![(10, 1_000)];

        let mut fish = baseline.clone();
        fish.inventory_counts[0].1 = 1;
        assert!(fishing_gain_observed(&baseline, &fish, &[TROUT_ID], 10));

        let mut xp = baseline.clone();
        xp.skill_xp[0].1 += 500;
        assert!(fishing_gain_observed(&baseline, &xp, &[TROUT_ID], 10));

        let mut animation_only = baseline.clone();
        animation_only.animation = Some(client::client::client::LocalAnimationUpdate {
            serial: 1,
            sequence: ROLL_ANIMATION,
        });
        animation_only.chat.push(ChatObservation {
            game_tick: 8,
            sequence: 1,
            text: "You attempt to catch a fish.".into(),
            poll: PollKind::Edge,
        });
        assert!(!fishing_gain_observed(
            &baseline,
            &animation_only,
            &[TROUT_ID],
            10
        ));
    }

    #[test]
    fn npc_engine_correlation_requires_accepted_order_for_the_minted_tmx_account() {
        let request = 41;
        let wire_packets = NPC_KNIFE_WIRE_OPCODES
            .iter()
            .map(|opcode| wire(*opcode, request, 1))
            .collect();
        let accepted = accepted_freshfish_packets();
        let mut evidence = evidence(wire_packets, vec![], accepted.clone());
        evidence.case = "tick-npc-knife";
        assert!(correlate_engine_packets(&evidence).is_ok());

        for index in 0..accepted.len() {
            let mut refused = accepted.clone();
            refused[index].handler_accepted = false;
            refused[index].charged_user_event = false;
            let mut wrong = evidence.clone();
            wrong.engine_packets = Some(Ok(refused));
            assert!(correlate_engine_packets(&wrong).is_err());
        }

        let mut wrong_account = evidence.clone();
        wrong_account.account = "other-oracle".into();
        assert!(correlate_engine_packets(&wrong_account).is_err());

        let mut wrong_packet_account = evidence.clone();
        wrong_packet_account
            .engine_packets
            .as_mut()
            .unwrap()
            .as_mut()
            .unwrap()[2]
            .username = "other-account".into();
        assert!(correlate_engine_packets(&wrong_packet_account).is_err());

        let mut wrong_sequence = evidence.clone();
        let rows = wrong_sequence
            .engine_packets
            .as_mut()
            .unwrap()
            .as_mut()
            .unwrap();
        rows[2].sequence = rows[1].sequence;
        assert!(correlate_engine_packets(&wrong_sequence).is_err());

        let mut wrong_order = evidence;
        let mut reordered = accepted;
        reordered.swap(2, 3);
        wrong_order.engine_packets = Some(Ok(reordered));
        assert!(correlate_engine_packets(&wrong_order).is_err());
    }
}
