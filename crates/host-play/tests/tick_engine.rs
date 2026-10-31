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
const OAK_LOG_ID: i32 = 1_521;
const OAK_LOC_ID: i32 = 1_281;
const RIVER_LOC_ID: i32 = 2_027;
const FLETCHING_ANIMATION: i32 = 1_248;
const CAST_ANIMATION: i32 = 622;
const ROLL_ANIMATION: i32 = 623;
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
const COPPER_BUDGET: u32 = 4;
const HOST_REFUSAL_BUDGET: u32 = 1;
const MAKE_WITHOUT_CLICK_BUDGET: u32 = 3;
const LONE_OPENER_BUDGET: u32 = 2;
const FREE_WORLD_BUDGET: u32 = 2;
const NET_REQUIRED_CYCLES: i64 = 20;
// Ending-family net cycles have no roll signal and count as unobserved; six
// edges per cycle bound the arm, checked click and next-cycle observation.
const NET_EDGES_PER_CYCLE: u32 = 6;
const NET_TICK_BUDGET: u32 = NET_REQUIRED_CYCLES as u32 * NET_EDGES_PER_CYCLE;

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
    require_free_world: bool,
    require_knife_arm: bool,
    mutations: Vec<DispatchMutation>,
    use_proxy: bool,
    suppress_openchat_span: Option<u32>,
    pre_batch_user_events: usize,
}

impl Scenario {
    fn river(name: &'static str) -> Self {
        let start = tile(2_501, 3_506, 0);
        Self {
            name,
            skill: "Fishing",
            skill_index: 10,
            level: 20,
            resource: ResourceKind::River,
            target: start,
            tool: item("fly_fishing_rod", FLY_ROD_ID, 1),
            extra_items: vec![item("feather", FEATHER_ID, 200)],
            products: vec![TROUT_ID, SALMON_ID],
            required_loc: Some((RIVER_LOC_ID, start)),
            require_free_world: false,
            require_knife_arm: true,
            mutations: Vec::new(),
            use_proxy: false,
            suppress_openchat_span: None,
            pre_batch_user_events: 0,
        }
    }

    fn net(name: &'static str) -> Self {
        Self {
            name,
            skill: "Fishing",
            skill_index: 10,
            level: 15,
            resource: ResourceKind::Net,
            target: tile(3_267, 3_148, 0),
            tool: item("net", NET_ID, 1),
            extra_items: Vec::new(),
            products: vec![SHRIMP_ID, ANCHOVY_ID],
            required_loc: None,
            require_free_world: false,
            require_knife_arm: true,
            mutations: Vec::new(),
            use_proxy: false,
            suppress_openchat_span: None,
            pre_batch_user_events: 0,
        }
    }

    fn copper(name: &'static str) -> Result<Self, String> {
        Ok(Self {
            name,
            skill: "Mining",
            skill_index: 14,
            level: 99,
            resource: ResourceKind::Copper,
            target: mining_start()?,
            tool: item("bronze_pickaxe", BRONZE_PICKAXE_ID, 1),
            extra_items: Vec::new(),
            products: vec![COPPER_ORE_ID],
            required_loc: None,
            require_free_world: false,
            require_knife_arm: true,
            mutations: Vec::new(),
            use_proxy: false,
            suppress_openchat_span: None,
            pre_batch_user_events: 0,
        })
    }

    fn iron(name: &'static str) -> Result<Self, String> {
        Ok(Self {
            name,
            skill: "Mining",
            skill_index: 14,
            level: 55,
            resource: ResourceKind::Iron,
            target: mining_start()?,
            tool: item("mithril_pickaxe", MITHRIL_PICKAXE_ID, 1),
            extra_items: Vec::new(),
            products: vec![IRON_ORE_ID],
            required_loc: None,
            require_free_world: false,
            require_knife_arm: true,
            mutations: Vec::new(),
            use_proxy: false,
            suppress_openchat_span: None,
            pre_batch_user_events: 0,
        })
    }

    fn oak(name: &'static str) -> Self {
        Self {
            name,
            skill: "Woodcutting",
            skill_index: 8,
            level: 15,
            resource: ResourceKind::Oak,
            target: tile(3_017, 3_170, 0),
            tool: item("bronze_axe", BRONZE_AXE_ID, 1),
            extra_items: Vec::new(),
            products: vec![OAK_LOG_ID],
            required_loc: Some((OAK_LOC_ID, tile(3_017, 3_170, 0))),
            require_free_world: true,
            require_knife_arm: false,
            mutations: Vec::new(),
            use_proxy: false,
            suppress_openchat_span: None,
            pre_batch_user_events: 0,
        }
    }

    fn with_mutation(mut self, mutation: DispatchMutation) -> Self {
        self.use_proxy |= matches!(&mutation, DispatchMutation::Transport(_));
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
        if matches!(self.resource, ResourceKind::River | ResourceKind::Net) {
            settings.insert(
                "fishingMethod".into(),
                json!(if self.resource == ResourceKind::River {
                    "fishing.loc_2027.op1"
                } else {
                    "fishing.saltfish.op1"
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
}

impl SetupState {
    fn new(account: String, scenario: Scenario) -> Self {
        let mut commands = vec![
            "setstat attack 1".to_owned(),
            "setstat defence 99".to_owned(),
            "setstat hitpoints 99".to_owned(),
            format!(
                "setstat {} {}",
                scenario.skill.to_ascii_lowercase(),
                scenario.level
            ),
        ];
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
        }
    }

    fn frame(&mut self, client: &mut Client, account: &str, hold: bool) {
        if account != self.account
            || hold
            || self.error.is_some()
            || self.phase == SetupPhase::Ready
        {
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
                    self.phase = SetupPhase::Ready;
                }
            }
            SetupPhase::Ready => {}
        }
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
        at_target && level_ready && items_ready && loc_ready
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
    start_tick: Option<u32>,
    baseline: Option<FrameObservation>,
    trace: TickEngineTrace,
    statuses: Vec<StatusObservation>,
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
    if !snapshot.ingame()
        || snapshot.scene_state() != 2
        || snapshot.tick() == hook.observed_game_tick()
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
    let direction = if source_x <= 100 { 1 } else { -1 };
    let mut returned_moves = 0;
    for offset in 1..=requested {
        let target_x = source_x + direction * offset as i32;
        if !client.tryMove(
            source_x, source_z, target_x, source_z, true, 0, 0, 0, 0, 0, 0,
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
    hook: Arc<TickEngineTestHook>,
    proxy: Option<Arc<LoopbackProxy>>,
    play: Option<host_play::Play>,
    start_tick: Option<u32>,
    baseline: Option<FrameObservation>,
    statuses: Vec<StatusObservation>,
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
        let setup = Arc::new(Mutex::new(SetupState::new(
            account.clone(),
            scenario.clone(),
        )));
        let foreign_events = Arc::new(Mutex::new(ForeignEventState {
            requested: scenario.pre_batch_user_events,
            ..ForeignEventState::default()
        }));
        let hook = TickEngineTestHook::new(account.clone());
        hook.install();
        if let Some(proxy) = &proxy {
            hook.set_transport_controller(proxy.clone());
        }
        let frame_setup = Arc::clone(&setup);
        let frame_foreign_events = Arc::clone(&foreign_events);
        let frame_hook = Arc::clone(&hook);
        let frame_account = account.clone();
        let play = match host_play::run_with_template(
            Arc::clone(&template),
            true,
            vec![],
            |_| (None, None),
            move |client, username, frame| {
                if username == frame_account {
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
            hook,
            proxy,
            play: Some(play),
            start_tick: None,
            baseline: None,
            statuses: Vec::new(),
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
            self.hook
                .suppress_openchat_through_tick(baseline.game_tick.wrapping_add(span));
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
        let Some(status) = self.play().script_native_status(&self.account) else {
            return;
        };
        let host_frame = trace
            .frames
            .last()
            .map_or_else(|| self.hook.observed_host_frame(), |frame| frame.host_frame);
        if self
            .statuses
            .last()
            .is_some_and(|previous| previous.host_frame == host_frame)
        {
            return;
        }
        let game_tick = trace
            .frames
            .last()
            .map_or_else(|| self.hook.observed_game_tick(), |frame| frame.game_tick);
        self.statuses
            .push(StatusObservation::capture(&status, host_frame, game_tick));
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
            start_tick: self.start_tick,
            baseline: self.baseline.clone(),
            trace: self.hook.trace(),
            statuses: self.statuses.clone(),
            transport: self
                .proxy
                .as_ref()
                .map_or_else(Vec::new, |proxy| proxy.observations()),
            engine_packets: self.engine_log.as_ref().map(|log| log.read(&self.account)),
        }
    }

    fn failure(&self, message: impl std::fmt::Display) -> String {
        format!(
            "tick-engine case {} failed: {message}\nlast evidence: {:#?}",
            self.scenario.name,
            self.evidence()
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
    println!("TICK-MANIP evidence {evidence:#?}");
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
    if matches!(
        evidence.case,
        "tick-make-without-click" | "tick-make-proxy-drop"
    ) {
        assert_engine_accepted_make_prefix(packets)?;
    }
    Ok(())
}

fn engine_accepted_user_event(packet: &EnginePacket) -> bool {
    packet.outcome == "accepted" && packet.handler_accepted && packet.charged_user_event
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
    println!("TICK-MANIP Engine T accepted Knife opener and Make before the absent terminal click");
    Ok(())
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
    let mut by_tick = BTreeMap::<u64, Vec<&EnginePacket>>::new();
    for packet in packets {
        by_tick.entry(packet.tick).or_default().push(packet);
    }
    let accepted = engine_accepted_user_event;
    for (tick, rows) in by_tick {
        let movement: Vec<_> = rows
            .iter()
            .copied()
            .filter(|packet| packet.opcode == ClientProt289::MOVE_GAMECLICK.id as u8)
            .collect();
        let opener: Vec<_> = rows
            .iter()
            .copied()
            .filter(|packet| packet.opcode == ClientProt289::OPHELDU.id as u8)
            .collect();
        let make: Vec<_> = rows
            .iter()
            .copied()
            .filter(|packet| packet.opcode == ClientProt289::IF_BUTTON.id as u8)
            .collect();
        let deferred_move: Vec<_> = rows
            .iter()
            .copied()
            .filter(|packet| packet.opcode == ClientProt289::MOVE_OPCLICK.id as u8)
            .collect();
        let deferred_loc: Vec<_> = rows
            .iter()
            .copied()
            .filter(|packet| packet.opcode == ClientProt289::OPLOC1.id as u8)
            .collect();
        if movement.len() != 3
            || opener.len() != 1
            || make.len() != 1
            || deferred_move.len() != 1
            || deferred_loc.len() != 1
            || !movement.iter().all(|packet| accepted(packet))
            || !accepted(opener[0])
            || !accepted(make[0])
            || deferred_move[0].outcome != "deferred"
            || deferred_loc[0].outcome != "deferred"
            || rows
                .iter()
                .filter(|packet| packet.charged_user_event)
                .count()
                != 5
            || movement
                .iter()
                .any(|packet| packet.sequence >= opener[0].sequence)
            || opener[0].sequence >= make[0].sequence
            || make[0].sequence >= deferred_move[0].sequence
            || deferred_move[0].sequence >= deferred_loc[0].sequence
        {
            continue;
        }
        println!(
            "TICK-MANIP deferred terminal row proved by Engine T tick={tick}; accepted moves=3, accepted Knife/Make=2, deferred click/loc=2"
        );
        return Ok(());
    }
    Err(format!(
        "Engine T log has no tick with three accepted MOVE_GAMECLICK events before accepted Knife/Make and deferred MOVE_OPCLICK/OPLOC1: {packets:?}"
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
        "start_tick": evidence.start_tick,
        "product_ids": evidence.product_ids,
        "baseline_snapshot": baseline,
        "frames": frames,
        "dispatch_receipts": dispatches,
        "encrypted_client_wire": wire,
        "native_status_history": statuses,
        "proxy_transport_history": transport,
        "engine_packet_history": engine_packets,
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
    let default = tile(3_253, 3_420, 0);
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

fn checked_click_pairs(trace: &TickEngineTrace, tick: u64) -> usize {
    let knife_ids: HashSet<u64> = trace
        .dispatches
        .iter()
        .filter(|dispatch| is_knife_arm(dispatch))
        .map(|dispatch| dispatch.receipt.request_id)
        .collect();
    let mut groups: BTreeMap<Option<u64>, Vec<u8>> = BTreeMap::new();
    for wire in trace.wire.iter().filter(|wire| {
        u64::from(wire.game_tick) == tick
            && !wire
                .request_id
                .is_some_and(|request_id| knife_ids.contains(&request_id))
    }) {
        groups.entry(wire.request_id).or_default().push(wire.opcode);
    }
    groups
        .values()
        .map(|opcodes| {
            opcodes
                .windows(2)
                .filter(|pair| {
                    pair[0] == ClientProt289::MOVE_OPCLICK.id as u8
                        && (pair[1] == ClientProt289::OPLOC1.id as u8
                            || pair[1] == ClientProt289::OPNPC1.id as u8)
                })
                .count()
        })
        .sum()
}

fn click_dispatches_at(trace: &TickEngineTrace, tick: u64) -> Vec<DispatchObservation> {
    trace
        .dispatches
        .iter()
        .filter(|dispatch| {
            dispatch.receipt.dispatch_tick == tick
                && !is_knife_arm(dispatch)
                && dispatch.receipt.wire_events == 2
                && dispatch.receipt.terminal.is_none()
        })
        .cloned()
        .collect()
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
    frames_in_tick(trace, tick)
        .into_iter()
        .flat_map(|frame| frame.chat.iter())
        .any(|line| predicate(&line.text))
}

fn has_act_line(trace: &TickEngineTrace, tick: u64) -> bool {
    has_chat_in_tick(trace, tick, |line| {
        line.to_ascii_lowercase()
            .contains("you attempt to catch a fish")
    })
}

fn has_fletching_members_line(trace: &TickEngineTrace, from_tick: u64, through_tick: u64) -> bool {
    trace
        .frames
        .iter()
        .filter(|frame| {
            let tick = u64::from(frame.game_tick);
            tick >= from_tick && tick <= through_tick
        })
        .flat_map(|frame| frame.chat.iter())
        .any(|line| {
            line.text.trim().eq_ignore_ascii_case(
                "You need to be on a members' world to gain experience in Fletching.",
            )
        })
}

fn has_copper_success(trace: &TickEngineTrace, from_tick: u64, through_tick: u64) -> bool {
    trace
        .frames
        .iter()
        .filter(|frame| {
            let tick = u64::from(frame.game_tick);
            tick >= from_tick && tick <= through_tick
        })
        .flat_map(|frame| frame.chat.iter())
        .any(|line| {
            let text = line.text.to_ascii_lowercase();
            text.contains("copper") && text.contains("mine")
        })
}

fn edge_has_openchat(trace: &TickEngineTrace, tick: u64) -> bool {
    frames_in_tick(trace, tick)
        .into_iter()
        .any(|frame| frame.actual_chat_modal_root == 8_880)
}

fn close_packet_seen(trace: &TickEngineTrace, tick: u64, baseline: &FrameObservation) -> bool {
    frames_in_tick(trace, tick).into_iter().any(|frame| {
        frame.server_if_close_packets > baseline.server_if_close_packets
            && frame.main_modal_closed_observation > baseline.main_modal_closed_observation
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
    let observed: Vec<u8> = wires_for_request(trace, dispatch.receipt.request_id)
        .into_iter()
        .map(|wire| wire.opcode)
        .collect();
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

fn assert_proxy_drop_after_make(run: &LiveRun, request_id: u64) -> Result<(), String> {
    let observations = run
        .proxy
        .as_ref()
        .map_or_else(Vec::new, |proxy| proxy.observations());
    let Some(observation) = observations.first() else {
        return Err(run.failure("Make suffix drop has no completed proxy observation"));
    };
    let wires = wires_for_request(&run.hook.trace(), request_id);
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
        return Err(run.failure(format!(
            "proxy did not forward only the two-packet Make prefix and drop its byte suffix: {observation:?}"
        )));
    }
    Ok(())
}

fn assert_make_without_click_outcome(run: &mut LiveRun, origin: u64) -> Result<(), String> {
    run.wait_until(
        origin as u32,
        MAKE_WITHOUT_CLICK_BUDGET,
        |trace, statuses| {
            fresh_animation_on_edge(trace, origin + 1)
                .is_some_and(|(sequence, _)| sequence == FLETCHING_ANIMATION)
                && !has_act_line(trace, origin + 1)
                && click_dispatches_at(trace, origin + 1).len() == 1
                && fresh_animation_on_edge(trace, origin + 3)
                    .is_some_and(|(sequence, _)| sequence == ROLL_ANIMATION)
                && statuses
                    .last()
                    .is_some_and(|status| status.tick_cycle == Some(3))
        },
    )?;
    let trace = run.hook.trace();
    assert_click(run, &trace, origin + 1)?;
    assert_roll(run, origin + 3)?;
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
    let receipts = click_dispatches_at(trace, tick);
    if count != 1 || receipts.len() != 1 {
        return Err(run.failure(format!(
            "expected one checked click at tick {tick}, observed {count} encoded packet pairs and {} receipts",
            receipts.len()
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
    let dispatch = click_dispatches_at(trace, tick)
        .into_iter()
        .next()
        .ok_or_else(|| run.failure(format!("checked click receipt disappeared at tick {tick}")))?;
    let observed: Vec<u8> = wires_for_request(trace, dispatch.receipt.request_id)
        .into_iter()
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

fn require_verdict_duration(run: &LiveRun, counter: &str, duration: i64) -> Result<(), String> {
    if run.counter(counter).unwrap_or(0) < 1 || run.counter("tick_cycle") != Some(duration) {
        return Err(run.failure(format!(
            "expected {counter} with cycle duration {duration}; counter={:?} tick_cycle={:?}",
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
    statuses: Vec<PrefixStatus>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PrefixFrame {
    offset: u32,
    tick_edge: bool,
    animation: Option<(i32, u32)>,
    chat: Vec<(String, PollKind)>,
    log_count: i32,
    product_counts: Vec<(i32, i32)>,
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
    offset: u32,
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
        .map(|dispatch| dispatch.receipt.dispatch_tick as u32)
    else {
        return PrefixTrace::default();
    };
    let baseline_xp: BTreeMap<i32, i32> = baseline.skill_xp.iter().copied().collect();
    let frames = evidence
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
                    .map(|line| (line.text.clone(), line.poll))
                    .collect(),
                log_count: frame.log_count,
                product_counts: frame.inventory_counts.clone(),
                skill_xp: frame
                    .skill_xp
                    .iter()
                    .map(|(index, xp)| (*index, xp - baseline_xp.get(index).copied().unwrap_or(0)))
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
    let dispatches = evidence
        .trace
        .dispatches
        .iter()
        .filter_map(|dispatch| {
            let offset = (dispatch.receipt.dispatch_tick as u32).wrapping_sub(origin);
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
        .filter_map(|packet| {
            let offset = packet.game_tick.wrapping_sub(origin);
            (offset < end_offset).then_some(PrefixWire {
                offset,
                opcode: packet.opcode,
                length: packet.encoded_length,
            })
        })
        .collect();
    let statuses = evidence
        .statuses
        .iter()
        .filter_map(|status| {
            let offset = status.game_tick.wrapping_sub(origin);
            (offset < end_offset).then(|| PrefixStatus {
                offset,
                phase: status.phase.clone(),
                method: status.method.clone(),
                tick_method: status.tick_method.clone(),
                target: status.target.clone(),
                last_event: status.last_event.clone(),
                tick_cycle: status.tick_cycle,
                tick_verified: status.tick_verified,
                tick_unobserved: status.tick_unobserved,
                tick_slips: status.tick_slips,
                tick_lost: status.tick_lost,
                tick_delayed: status.tick_delayed,
                tick_paused: status.tick_paused,
                tick_fallbacks: status.tick_fallbacks,
                tick_fallback_failed: status.tick_fallback_failed,
                tick_events: status.tick_events,
            })
        })
        .collect();
    PrefixTrace {
        frames,
        dispatches,
        wire,
        statuses,
    }
}

#[test]
#[ignore = "requires LIVE=1, GATHERER_ENGINE_DIR/NAV_PACK/CATALOG_ROOT, LIVE_EVIDENCE_DIR and Engine T 127.0.0.1:48594/4080"]
fn tick_engine_clean_continuing_river() {
    run_case(Scenario::river("tick-clean-river"), |run| {
        let trace = run.hook.trace();
        let dispatch = first_knife(&trace).map_err(|error| run.failure(error))?;
        let origin = dispatch.receipt.dispatch_tick;
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
fn tick_engine_clean_ending_net_twenty_cycles() {
    run_case(Scenario::net("tick-clean-net"), |run| {
        let trace = run.hook.trace();
        let dispatch = first_knife(&trace).map_err(|error| run.failure(error))?;
        let origin = dispatch.receipt.dispatch_tick;
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
    let scenario = Scenario::river("tick-stale-menu")
        .with_mutation(DispatchMutation::CancelAfterPackets(1))
        .with_openchat_suppression(4);
    run_case(scenario, |run| {
        let trace = run.hook.trace();
        let first = first_knife(&trace).map_err(|error| run.failure(error))?;
        let origin = first.receipt.dispatch_tick;
        assert_batch(run, &first, 1, 1)?;
        assert_knife_wire_order(run, &trace, &first, &[ClientProt289::OPHELDU.id as u8])?;
        let baseline = run
            .baseline
            .as_ref()
            .expect("started run has baseline")
            .clone();
        run.wait_until(origin as u32, STALE_MENU_BUDGET, |trace, statuses| {
            let batches = knife_dispatches(trace);
            batches.len() >= 2
                && batches[1].receipt.dispatch_tick == origin + 1
                && batches[1].receipt.dispatched_prefix == 3
                && batches[1].receipt.wire_events == 4
                && close_packet_seen(trace, origin + 1, &baseline)
                && no_fresh_animation_on_edge(trace, origin + 1)
                && fresh_animation_on_edge(trace, origin + 2)
                    .is_some_and(|(sequence, _)| sequence == FLETCHING_ANIMATION)
                && has_act_line(trace, origin + 2)
                && fresh_animation_on_edge(trace, origin + 4)
                    .is_some_and(|(sequence, _)| sequence == ROLL_ANIMATION)
                && statuses
                    .last()
                    .is_some_and(|status| status.tick_slips.unwrap_or(0) >= 1)
        })?;
        let trace = run.hook.trace();
        let batches = knife_dispatches(&trace);
        let second = batches
            .get(1)
            .ok_or_else(|| run.failure("stale-menu next-window batch receipt disappeared"))?;
        assert_batch(run, second, 3, 4)?;
        assert_knife_wire_order(
            run,
            &trace,
            second,
            &[
                ClientProt289::OPHELDU.id as u8,
                ClientProt289::IF_BUTTON.id as u8,
                ClientProt289::MOVE_OPCLICK.id as u8,
                ClientProt289::OPLOC1.id as u8,
            ],
        )?;
        if !frames_in_tick(&trace, origin + 1)
            .iter()
            .any(|frame| frame.suppressed_openchat && frame.actual_chat_modal_root == 8_880)
        {
            return Err(run.failure(
                "stale-menu guard did not filter the real 8880 root before script polling",
            ));
        }
        assert_click(run, &trace, origin + 2)?;
        require_verdict_duration(run, "tick_slips", 4)?;
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
        let origin = first.receipt.dispatch_tick;
        assert_batch(run, &first, 3, 4)?;
        run.wait_until(origin as u32, SPLIT_OPENER_BUDGET, |trace, statuses| {
            edge_has_openchat(trace, origin + 1)
                && fresh_animation_on_edge(trace, origin + 2)
                    .is_some_and(|(sequence, _)| sequence == FLETCHING_ANIMATION)
                && has_act_line(trace, origin + 2)
                && click_dispatches_at(trace, origin + 2).len() == 1
                && fresh_animation_on_edge(trace, origin + 4)
                    .is_some_and(|(sequence, _)| sequence == ROLL_ANIMATION)
                && statuses
                    .last()
                    .is_some_and(|status| status.tick_slips.unwrap_or(0) >= 1)
        })?;
        let trace = run.hook.trace();
        assert_click(run, &trace, origin + 2)?;
        assert_roll(run, origin + 4)?;
        require_verdict_duration(run, "tick_slips", 4)?;
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
        let origin = first.receipt.dispatch_tick;
        assert_batch(run, &first, 3, 4)?;
        // The first window may include interface packets before PLAYER_INFO.
        run.wait_until(origin as u32, LATE_RIVER_BUDGET, |trace, statuses| {
            fresh_animation_on_edge(trace, origin + 2).is_some_and(|(sequence, _)| {
                sequence == FLETCHING_ANIMATION || sequence == CAST_ANIMATION
            }) && click_dispatches_at(trace, origin + 2).len() == 1
                && fresh_animation_on_edge(trace, origin + 5)
                    .is_some_and(|(sequence, _)| sequence == ROLL_ANIMATION)
                && knife_dispatches(trace)
                    .iter()
                    .any(|dispatch| dispatch.receipt.dispatch_tick == origin + 5)
                && statuses
                    .last()
                    .is_some_and(|status| status.tick_slips.unwrap_or(0) >= 1)
        })?;
        let trace = run.hook.trace();
        assert_click(run, &trace, origin + 2)?;
        assert_roll(run, origin + 5)?;
        require_verdict_duration(run, "tick_slips", 5)?;
        if knife_dispatches(&trace)
            .iter()
            .filter(|dispatch| dispatch.receipt.dispatch_tick < origin + 5)
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
        .with_mutation(DispatchMutation::Transport(
            TransportMutation::HoldThroughNextEdge,
        ));
    run_case(iron, |run| {
        let first = first_knife(&run.hook.trace()).map_err(|error| run.failure(error))?;
        let origin = first.receipt.dispatch_tick;
        assert_batch(run, &first, 3, 4)?;
        run.wait_until(origin as u32, LATE_IRON_BUDGET, |trace, statuses| {
            fresh_animation_on_edge(trace, origin + 2).is_some_and(|(sequence, _)| {
                sequence == MITHRIL_PICK_ANIMATION || sequence == FLETCHING_ANIMATION
            }) && has_chat_in_tick(trace, origin + 2, |line| {
                line.to_ascii_lowercase()
                    .contains("swing your pick at the rock")
            }) && click_dispatches_at(trace, origin + 2).len() == 1
                && fresh_animation_on_edge(trace, origin + 5)
                    .is_some_and(|(sequence, _)| sequence == ROLL_ANIMATION)
                && statuses
                    .last()
                    .is_some_and(|status| status.tick_slips.unwrap_or(0) >= 1)
        })?;
        let trace = run.hook.trace();
        assert_click(run, &trace, origin + 2)?;
        assert_roll(run, origin + 5)?;
        require_verdict_duration(run, "tick_slips", 5)?;
        if knife_dispatches(&trace).iter().any(|dispatch| {
            dispatch.receipt.dispatch_tick < origin + 5 && dispatch.receipt.dispatch_tick != origin
        }) {
            return Err(run.failure("iron/mithril branch emitted a second batch before R+1+4"));
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

fn copper_history(scenario: Scenario, expect_held_join: bool) -> Result<RunEvidence, String> {
    run_case(scenario, |run| {
        let initial_trace = run.hook.trace();
        let first = first_knife(&initial_trace).map_err(|error| run.failure(error))?;
        let origin = first.receipt.dispatch_tick;
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
                    dispatch.receipt.dispatch_tick == origin + 4
                        && dispatch.request_debug != first.request_debug
                })
                && fresh_animation_on_edge(trace, origin + 4)
                    .is_some_and(|(sequence, _)| sequence == ROLL_ANIMATION)
                && statuses.last().is_some_and(|status| {
                    status.tick_delayed.unwrap_or(0) >= 1 && status.tick_cycle == Some(4)
                })
        })?;
        let trace = run.hook.trace();
        let batches = knife_dispatches(&trace);
        if batches.len() < 2 || batches[1].receipt.dispatch_tick != origin + 4 {
            return Err(
                run.failure("copper delayed cycle did not emit its second-rock batch at R+4")
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
                    && observation.release_trigger_tick == Some(origin as u32 + 4)
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
        assert_running(run)
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
    if normal_prefix != held_prefix {
        panic!(
            "copper host trace diverged before the second edge\nnormal={normal_prefix:#?}\nheld={held_prefix:#?}"
        );
    }
}

#[test]
#[ignore = "requires LIVE=1, GATHERER_ENGINE_DIR/NAV_PACK/CATALOG_ROOT, LIVE_EVIDENCE_DIR and Engine T 127.0.0.1:48594/4080"]
fn tick_engine_removed_target_refuses_prefix_zero_and_continues() {
    let scenario =
        Scenario::river("tick-target-removed").with_mutation(DispatchMutation::RemoveLoc {
            id: RIVER_LOC_ID,
            tile: tile(2_501, 3_506, 0),
        });
    run_case(scenario, |run| {
        let first = first_knife(&run.hook.trace()).map_err(|error| run.failure(error))?;
        let origin = first.receipt.dispatch_tick;
        run.wait_until(origin as u32, HOST_REFUSAL_BUDGET, |trace, statuses| {
            trace.dispatches.iter().any(|dispatch| {
                is_knife_arm(dispatch)
                    && dispatch.receipt.dispatch_tick == origin
                    && dispatch.mutation_applied
                    && dispatch.receipt.dispatched_prefix == 0
                    && dispatch.receipt.wire_events == 0
                    && format!("{:?}", dispatch.receipt.terminal).contains("StaleTarget")
            }) && click_dispatches_at(trace, origin + 1).len() == 1
                && statuses
                    .last()
                    .is_some_and(|status| status.tick_lost.unwrap_or(0) >= 1)
        })?;
        let trace = run.hook.trace();
        let refused = trace
            .dispatches
            .iter()
            .find(|dispatch| is_knife_arm(dispatch) && dispatch.receipt.dispatch_tick == origin)
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
        require_verdict_duration(run, "tick_lost", 1)?;
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
        let origin = first.receipt.dispatch_tick;
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
        let origin = first.receipt.dispatch_tick;
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
                && click_dispatches_at(trace, origin + 1).len() == 1
                && fresh_animation_on_edge(trace, origin + 3)
                    .is_some_and(|(sequence, _)| sequence == ROLL_ANIMATION)
                && statuses.last().is_some_and(|status| {
                    status.tick_slips == Some(initial_slips + 1)
                        && status.tick_fallbacks == Some(initial_fallbacks + 1)
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
            || final_status.tick_fallbacks != Some(initial_fallbacks + 1)
            || final_status.tick_fallback_failed != Some(initial_fallback_failed)
        {
            return Err(run.failure(format!(
                "deferred terminal-row cycle did not end as exactly one slip and one fallback: initial={initial:?} final={final_status:?}"
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
        let origin = first.receipt.dispatch_tick;
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
        assert_make_without_click_outcome(run, origin)?;
        assert_proxy_drop_after_make(run, first.receipt.request_id)
    })
    .unwrap_or_else(|error| panic!("{error}"));
}

fn lone_opener_history(name: &'static str, suppress: bool) -> Result<RunEvidence, String> {
    let mut scenario = Scenario::river(name).with_mutation(DispatchMutation::CancelAfterPackets(1));
    if suppress {
        scenario = scenario.with_openchat_suppression(4);
    }
    run_case(scenario, |run| {
        let first = first_knife(&run.hook.trace()).map_err(|error| run.failure(error))?;
        let origin = first.receipt.dispatch_tick;
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
            click_dispatches_at(trace, origin + 1).len() == 1
                && close_packet_seen(trace, origin + 1, &baseline)
                && has_act_line(trace, origin + 1)
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
    if visible_prefix != hidden_prefix {
        panic!(
            "lone-opener host trace changed when IF_OPENCHAT was snapshot-suppressed\nvisible={visible_prefix:#?}\nhidden={hidden_prefix:#?}"
        );
    }
}

#[test]
#[ignore = "requires LIVE=1, GATHERER_ENGINE_DIR/NAV_PACK/CATALOG_ROOT, LIVE_EVIDENCE_DIR, selected local free-world world.json, and Engine T node.members=false phase"]
fn tick_engine_free_world_disables_members_only_knife_arm() {
    run_case(Scenario::oak("tick-free-world-oak"), |run| {
        let start = run.start_tick.ok_or_else(|| run.failure("missing pre-Start tick"))?;
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

    #[test]
    fn river_setup_starts_on_a_real_standable_tile() {
        let pack = PathBuf::from(std::env::var_os("NAV_PACK").expect("NAV_PACK is required"));
        let world = nav_planner::world::NavWorld::load_pack(&pack).unwrap();
        let scenario = Scenario::river("tick-setup");
        let neighbours = [(-1, 0), (1, 0), (0, -1), (0, 1)].map(|(dx, dz)| {
            let candidate = tile(scenario.target.x + dx, scenario.target.z + dz, 0);
            (candidate, world.collision.standable(candidate))
        });
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
        let mut accessible = Vec::new();
        for spot in spots.iter() {
            for (dx, dz) in [(-1, 0), (1, 0), (0, -1), (0, 1)] {
                let stand = tile(spot.origin.x + dx, spot.origin.z + dz, spot.origin.level);
                if world.collision.standable(stand) {
                    accessible.push((spot.origin, stand));
                }
            }
        }
        assert!(
            world.collision.standable(scenario.target),
            "river player start {:?} is not standable; neighbours={neighbours:?}; accessible fixed spots={accessible:?}",
            scenario.target
        );
        assert_ne!(
            scenario.target,
            scenario.required_loc.expect("river has a fixed loc").1
        );
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
            start_tick: Some(7),
            baseline: None,
            trace: TickEngineTrace {
                frames: Vec::new(),
                dispatches: Vec::new(),
                wire,
            },
            statuses: Vec::new(),
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

    #[test]
    fn engine_correlation_accepts_byte_preserving_make_suffix_drop() {
        let request = 11;
        let make_opcodes = vec![
            ClientProt289::OPHELDU.id as u8,
            ClientProt289::IF_BUTTON.id as u8,
            ClientProt289::MOVE_OPCLICK.id as u8,
            ClientProt289::OPLOC1.id as u8,
        ];
        let make_lengths = vec![6, 4, 7, 5];
        let mut wire_packets: Vec<_> = make_opcodes
            .iter()
            .zip(&make_lengths)
            .map(|(opcode, length)| wire(*opcode, request, *length))
            .collect();
        let click_opcodes = [
            ClientProt289::MOVE_OPCLICK.id as u8,
            ClientProt289::OPNPC1.id as u8,
        ];
        wire_packets.extend(
            click_opcodes
                .iter()
                .map(|opcode| wire(*opcode, request + 1, 4)),
        );
        let mut engine_packets = vec![packet(make_opcodes[0], 1), packet(make_opcodes[1], 2)];
        engine_packets.extend(
            click_opcodes
                .iter()
                .enumerate()
                .map(|(index, opcode)| packet(*opcode, index as u64 + 3)),
        );
        let transport = vec![drop_observation(
            request,
            make_opcodes,
            make_lengths,
            10,
            12,
        )];
        assert!(
            correlate_engine_packets(&evidence(wire_packets, transport, engine_packets,)).is_ok()
        );
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
        assert!(
            correlate_engine_packets(&evidence(wire_packets, transport, engine_packets,)).is_err()
        );
    }
}
