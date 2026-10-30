use std::collections::{BTreeMap, HashSet};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use api::interact::{self, Interactions};
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
use serde_json::{json, Map, Value};
use script::native::{ScriptStatus, StatusValue};
use vault::{Profile, ProfileSettings};

#[path = "support/loopback_proxy.rs"]
mod loopback_proxy;
use loopback_proxy::{LoopbackProxy, ProxyObservation};

const ENGINE_HOST: &str = "127.0.0.1";
const ENGINE_GAME_PORT: u16 = 47_594;
const ENGINE_HTTP_PORT: u16 = 3_080;
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
            json!(if matches!(self.resource, ResourceKind::Copper | ResourceKind::Iron) {
                "Best tier"
            } else {
                "Nearest"
            }),
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
    WaitLogout,
    WaitRelogin,
    Seed,
    Verify,
    Ready,
}

struct SetupState {
    account: String,
    scenario: Scenario,
    commands: Vec<String>,
    next_command: usize,
    phase: SetupPhase,
    error: Option<String>,
}

impl SetupState {
    fn new(account: String, scenario: Scenario) -> Self {
        let mut commands = vec![
            "setstat attack 1".to_owned(),
            "setstat defence 99".to_owned(),
            "setstat hitpoints 99".to_owned(),
            format!("setstat {} {}", scenario.skill.to_ascii_lowercase(), scenario.level),
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
        }
    }

    fn frame(&mut self, client: &mut Client, account: &str, hold: bool) {
        if account != self.account || hold || self.error.is_some() || self.phase == SetupPhase::Ready {
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
                    self.phase = SetupPhase::WaitLogout;
                } else {
                    self.error = Some("tutorial completion logout was refused".into());
                }
            }
            SetupPhase::WaitLogout => {
                if !client.ingame {
                    self.phase = SetupPhase::WaitRelogin;
                }
            }
            SetupPhase::WaitRelogin => {
                if client.ingame && client.scene_state == 2 {
                    self.phase = SetupPhase::Seed;
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
    start_tick: Option<u32>,
    baseline: Option<FrameObservation>,
    trace: TickEngineTrace,
    statuses: Vec<StatusObservation>,
    transport: Vec<ProxyObservation>,
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
    hook: Arc<TickEngineTestHook>,
    proxy: Option<Arc<LoopbackProxy>>,
    play: Option<host_play::Play>,
    start_tick: Option<u32>,
    baseline: Option<FrameObservation>,
    statuses: Vec<StatusObservation>,
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
            Some(Arc::new(LoopbackProxy::start(SocketAddr::from((
                [127, 0, 0, 1],
                ENGINE_GAME_PORT,
            )))?))
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
            return Err("tick-engine cases require the assigned loopback Engine L profile".into());
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
            return Err("selected Engine L navigation template has no world".into());
        }
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
        let hook = TickEngineTestHook::new(account.clone());
        hook.install();
        if let Some(proxy) = &proxy {
            hook.set_transport_controller(Arc::clone(proxy));
        }
        let frame_setup = Arc::clone(&setup);
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
            hook,
            proxy,
            play: Some(play),
            start_tick: None,
            baseline: None,
            statuses: Vec::new(),
            shutdown: false,
        })
    }

    fn start(&mut self) -> Result<(), String> {
        if self.start_tick.is_some() {
            return Err("Gatherer Start was already submitted".into());
        }
        let password = self.password.take().ok_or("live password was already consumed")?;
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
            .map_err(|error| format!("spawn Engine L account: {error}"))?;
        self.play_mut().focus(&self.account);
        self.wait_setup_ready()?;
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
        self.play_mut()
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
                let setup = self.setup.lock().map_err(|_| "tick-engine setup poisoned")?;
                if let Some(error) = &setup.error {
                    return Err(error.clone());
                }
                if setup.phase == SetupPhase::Ready {
                    return Ok(());
                }
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(format!(
                    "per-account tutorial/seed setup did not reach its real target within {SETUP_DEADLINE:?}"
                ));
            }
            after = self
                .hook
                .wait_for_frame(after, remaining.min(FRAME_HANG_GUARD))
                .ok_or_else(|| "live slot stopped producing host frames during setup".to_owned())?;
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
            start_tick: self.start_tick,
            baseline: self.baseline.clone(),
            trace: self.hook.trace(),
            statuses: self.statuses.clone(),
            transport: self
                .proxy
                .as_ref()
                .map_or_else(Vec::new, |proxy| proxy.observations()),
        }
    }

    fn print_evidence(&self) {
        println!("TICK-MANIP evidence {:#?}", self.evidence());
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
    run.print_evidence();
    let evidence = run.evidence();
    let persisted = persist_evidence(&evidence);
    let cleanup = run.shutdown();
    let mut failures = Vec::new();
    if let Err(error) = result {
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

fn persist_evidence(evidence: &RunEvidence) -> Result<PathBuf, String> {
    let root = required_path("LIVE_EVIDENCE_DIR")?;
    let epoch = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("evidence clock: {error}"))?
        .as_nanos();
    let directory = root.join(format!("{}_{}", evidence.case, epoch));
    std::fs::create_dir_all(&directory)
        .map_err(|error| format!("create evidence directory {}: {error}", directory.display()))?;
    let path = directory.join("tick_engine.txt");
    std::fs::write(&path, format!("{evidence:#?}\n"))
        .map_err(|error| format!("write evidence {}: {error}", path.display()))?;
    println!("TICK-MANIP evidence_path={}", path.display());
    Ok(path)
}

fn require_live_config() -> Result<(), String> {
    if std::env::var("LIVE").as_deref() != Ok("1") {
        return Err("tick-engine live tests require LIVE=1; opted-in cases never silently skip".into());
    }
    let prefix = std::env::var("BOT_LIVE_NAME_PREFIX")
        .map_err(|_| "BOT_LIVE_NAME_PREFIX is required for fresh Engine L accounts".to_owned())?;
    if prefix.is_empty() || prefix.len() > 4 || !prefix.is_ascii() || prefix == "live" {
        return Err("BOT_LIVE_NAME_PREFIX must be an owned 1-4 character ASCII prefix, not the default".into());
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
    let value = std::env::var(name).map_err(|_| format!("{name} is required for tick-engine live qualification"))?;
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
                    pair[0] == ClientProt289::MOVE_OPCLICK.id
                        && (pair[1] == ClientProt289::OPLOC1.id
                            || pair[1] == ClientProt289::OPNPC1.id)
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

fn has_members_line(trace: &TickEngineTrace, through_tick: u64) -> bool {
    trace
        .frames
        .iter()
        .filter(|frame| u64::from(frame.game_tick) <= through_tick)
        .flat_map(|frame| frame.chat.iter())
        .any(|line| {
            let text = line.text.to_ascii_lowercase();
            text.contains("member") && (text.contains("fletch") || text.contains("skill"))
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
    let expected = [ClientProt289::MOVE_OPCLICK.id, target_opcode];
    if observed.as_slice() != expected.as_slice() {
        return Err(run.failure(format!(
            "checked click at tick {tick} encoded {observed:?}, expected {expected:?}"
        )));
    }
    Ok(())
}

fn assert_roll(run: &LiveRun, tick: u64) -> Result<(), String> {
    let trace = run.hook.trace();
    if fresh_animation_on_edge(&trace, tick).is_none_or(|(sequence, _)| sequence != ROLL_ANIMATION) {
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
        .ok_or_else(|| run.failure("native Gatherer status disappeared before the case completed"))?;
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
    if status.fields.iter().any(|field| field.key == "tick_refused") {
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
    let (Some(baseline), Some(_start_tick)) = (evidence.baseline.as_ref(), evidence.start_tick) else {
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
                request: dispatch
                    .request_debug
                    .as_deref()
                    .map(|request| request.split([' ', '{']).next().unwrap_or(request).to_owned()),
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
#[ignore = "requires LIVE=1, GATHERER_ENGINE_DIR/NAV_PACK/CATALOG_ROOT, LIVE_EVIDENCE_DIR and Engine L 127.0.0.1:47594/3080"]
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
                ClientProt289::OPHELDU.id,
                ClientProt289::IF_BUTTON.id,
                ClientProt289::MOVE_OPCLICK.id,
                ClientProt289::OPLOC1.id,
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
        let close_baseline = run.baseline.as_ref().expect("started run has baseline").clone();
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
            return Err(run.failure("clean river accepted Cast animation 622 instead of a landed arm"));
        }
        assert_log_unchanged(run)?;
        assert_running(run)
    })
    .unwrap_or_else(|error| panic!("{error}"));
}

#[test]
#[ignore = "requires LIVE=1, GATHERER_ENGINE_DIR/NAV_PACK/CATALOG_ROOT, LIVE_EVIDENCE_DIR and Engine L 127.0.0.1:47594/3080"]
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
            &[ClientProt289::OPHELDU.id, ClientProt289::IF_BUTTON.id],
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
        assert_click_target(run, &trace, origin + 2, ClientProt289::OPNPC1.id)?;
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
#[ignore = "requires LIVE=1, GATHERER_ENGINE_DIR/NAV_PACK/CATALOG_ROOT, LIVE_EVIDENCE_DIR and Engine L 127.0.0.1:47594/3080"]
fn tick_engine_stale_menu_guard_late_case_two() {
    let scenario = Scenario::river("tick-stale-menu")
        .with_mutation(DispatchMutation::CancelAfterPackets(1))
        .with_openchat_suppression(4);
    run_case(scenario, |run| {
        let trace = run.hook.trace();
        let first = first_knife(&trace).map_err(|error| run.failure(error))?;
        let origin = first.receipt.dispatch_tick;
        assert_batch(run, &first, 1, 1)?;
        assert_knife_wire_order(run, &trace, &first, &[ClientProt289::OPHELDU.id])?;
        let baseline = run.baseline.as_ref().expect("started run has baseline").clone();
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
                && statuses.last().is_some_and(|status| status.tick_slips.unwrap_or(0) >= 1)
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
                ClientProt289::OPHELDU.id,
                ClientProt289::IF_BUTTON.id,
                ClientProt289::MOVE_OPCLICK.id,
                ClientProt289::OPLOC1.id,
            ],
        )?;
        if !frames_in_tick(&trace, origin + 1)
            .iter()
            .any(|frame| frame.suppressed_openchat && frame.actual_chat_modal_root == 8_880)
        {
            return Err(run.failure("stale-menu guard did not filter the real 8880 root before script polling"));
        }
        assert_click(run, &trace, origin + 2)?;
        require_verdict_duration(run, "tick_slips", 4)?;
        assert_log_unchanged(run)?;
        assert_running(run)
    })
    .unwrap_or_else(|error| panic!("{error}"));
}

#[test]
#[ignore = "requires LIVE=1, GATHERER_ENGINE_DIR/NAV_PACK/CATALOG_ROOT, LIVE_EVIDENCE_DIR and Engine L 127.0.0.1:47594/3080"]
fn tick_engine_split_opener_late_case_one() {
    let scenario = Scenario::river("tick-split-opener")
        .with_mutation(DispatchMutation::Transport(TransportMutation::SplitAfterFirstPacket));
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
                && statuses.last().is_some_and(|status| status.tick_slips.unwrap_or(0) >= 1)
        })?;
        let trace = run.hook.trace();
        assert_click(run, &trace, origin + 2)?;
        assert_roll(run, origin + 4)?;
        require_verdict_duration(run, "tick_slips", 4)?;
        if trace
            .wire
            .iter()
            .any(|wire| wire.opcode == ClientProt289::RESUME_PAUSEBUTTON.id)
        {
            return Err(run.failure("split-opener branch unexpectedly emitted a pause press"));
        }
        assert_held_batch(
            run,
            first.receipt.request_id,
            &[
                ClientProt289::OPHELDU.id,
                ClientProt289::IF_BUTTON.id,
                ClientProt289::MOVE_OPCLICK.id,
                ClientProt289::OPLOC1.id,
            ],
            origin as u32 + 1,
        )?;
        assert_log_unchanged(run)?;
        assert_running(run)
    })
    .unwrap_or_else(|error| panic!("{error}"));
}

#[test]
#[ignore = "requires LIVE=1, GATHERER_ENGINE_DIR/NAV_PACK/CATALOG_ROOT, LIVE_EVIDENCE_DIR and Engine L 127.0.0.1:47594/3080"]
fn tick_engine_plain_late_river_and_mithril_iron() {
    let river = Scenario::river("tick-late-river")
        .with_mutation(DispatchMutation::Transport(TransportMutation::HoldThroughNextEdge));
    run_case(river, |run| {
        let first = first_knife(&run.hook.trace()).map_err(|error| run.failure(error))?;
        let origin = first.receipt.dispatch_tick;
        assert_batch(run, &first, 3, 4)?;
        let baseline = run.baseline.as_ref().expect("started run has baseline").clone();
        run.wait_until(origin as u32, LATE_RIVER_BUDGET, |trace, statuses| {
            !edge_has_openchat(trace, origin + 1)
                && frames_in_tick(trace, origin + 1).iter().all(|frame| {
                    frame.main_modal_opened_observation
                        == baseline.main_modal_opened_observation
                        && frame.main_modal_closed_observation
                            == baseline.main_modal_closed_observation
                })
                && fresh_animation_on_edge(trace, origin + 2).is_some_and(|(sequence, _)| {
                    sequence == FLETCHING_ANIMATION || sequence == CAST_ANIMATION
                })
                && click_dispatches_at(trace, origin + 2).len() == 1
                && fresh_animation_on_edge(trace, origin + 5)
                    .is_some_and(|(sequence, _)| sequence == ROLL_ANIMATION)
                && knife_dispatches(trace).iter().any(|dispatch| {
                    dispatch.receipt.dispatch_tick == origin + 5
                })
                && statuses.last().is_some_and(|status| status.tick_slips.unwrap_or(0) >= 1)
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
                ClientProt289::OPHELDU.id,
                ClientProt289::IF_BUTTON.id,
                ClientProt289::MOVE_OPCLICK.id,
                ClientProt289::OPLOC1.id,
            ],
            origin as u32 + 1,
        )?;
        assert_log_unchanged(run)?;
        assert_running(run)
    })
    .unwrap_or_else(|error| panic!("{error}"));

    let iron = Scenario::iron("tick-late-iron")
        .unwrap_or_else(|error| panic!("{error}"))
        .with_mutation(DispatchMutation::Transport(TransportMutation::HoldThroughNextEdge));
    run_case(iron, |run| {
        let first = first_knife(&run.hook.trace()).map_err(|error| run.failure(error))?;
        let origin = first.receipt.dispatch_tick;
        assert_batch(run, &first, 3, 4)?;
        run.wait_until(origin as u32, LATE_IRON_BUDGET, |trace, statuses| {
            fresh_animation_on_edge(trace, origin + 2).is_some_and(|(sequence, _)| {
                sequence == MITHRIL_PICK_ANIMATION || sequence == FLETCHING_ANIMATION
            }) && has_chat_in_tick(trace, origin + 2, |line| {
                line.to_ascii_lowercase().contains("swing your pick at the rock")
            }) && click_dispatches_at(trace, origin + 2).len() == 1
                && fresh_animation_on_edge(trace, origin + 5)
                    .is_some_and(|(sequence, _)| sequence == ROLL_ANIMATION)
                && statuses.last().is_some_and(|status| status.tick_slips.unwrap_or(0) >= 1)
        })?;
        let trace = run.hook.trace();
        assert_click(run, &trace, origin + 2)?;
        assert_roll(run, origin + 5)?;
        require_verdict_duration(run, "tick_slips", 5)?;
        if knife_dispatches(&trace)
            .iter()
            .any(|dispatch| dispatch.receipt.dispatch_tick < origin + 5 && dispatch.receipt.dispatch_tick != origin)
        {
            return Err(run.failure("iron/mithril branch emitted a second batch before R+1+4"));
        }
        assert_held_batch(
            run,
            first.receipt.request_id,
            &[
                ClientProt289::OPHELDU.id,
                ClientProt289::IF_BUTTON.id,
                ClientProt289::MOVE_OPCLICK.id,
                ClientProt289::OPLOC1.id,
            ],
            origin as u32 + 1,
        )?;
        assert_log_unchanged(run)?;
        assert_running(run)
    })
    .unwrap_or_else(|error| panic!("{error}"));
}

fn copper_history(
    scenario: Scenario,
    expect_held_join: bool,
) -> Result<RunEvidence, String> {
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
                ClientProt289::OPHELDU.id,
                ClientProt289::IF_BUTTON.id,
                ClientProt289::MOVE_OPCLICK.id,
                ClientProt289::OPLOC1.id,
            ],
        )?;
        run.wait_until(origin as u32, COPPER_BUDGET, |trace, statuses| {
            fresh_animation_on_edge(trace, origin + 1)
                .is_some_and(|(sequence, _)| sequence == -1)
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
            return Err(run.failure("copper delayed cycle did not emit its second-rock batch at R+4"));
        }
        assert_batch(run, &batches[1], 3, 4)?;
        assert_knife_wire_order(
            run,
            &trace,
            &batches[1],
            &[
                ClientProt289::OPHELDU.id,
                ClientProt289::IF_BUTTON.id,
                ClientProt289::MOVE_OPCLICK.id,
                ClientProt289::OPLOC1.id,
            ],
        )?;
        if run.product_gain(&trace) <= 0 && run.skill_xp_gain(&trace) <= 0 {
            return Err(run.failure("copper case showed no real ore or Mining XP"));
        }
        if batches.iter().any(|dispatch| dispatch.receipt.terminal.is_some()) {
            return Err(run.failure("copper host receipt recorded a refused KnifeArm"));
        }
        if expect_held_join {
            let observations = run
                .proxy
                .as_ref()
                .map_or_else(Vec::new, |proxy| proxy.observations());
            let expected_opcodes = vec![
                ClientProt289::OPHELDU.id,
                ClientProt289::IF_BUTTON.id,
                ClientProt289::MOVE_OPCLICK.id,
                ClientProt289::OPLOC1.id,
            ];
            let complete = observations.first().is_some_and(|observation| {
                let expected_bytes: usize = observation.packet_lengths.iter().sum();
                observation.request_ids
                    == vec![
                        batches[0].receipt.request_id,
                        batches[1].receipt.request_id,
                    ]
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
#[ignore = "requires LIVE=1, GATHERER_ENGINE_DIR/NAV_PACK/CATALOG_ROOT, LIVE_EVIDENCE_DIR and Engine L 127.0.0.1:47594/3080"]
fn tick_engine_copper_success_and_delayed_first_batch_history() {
    let normal = copper_history(Scenario::copper("tick-copper-normal").unwrap(), false)
        .unwrap_or_else(|error| panic!("{error}"));
    let held = copper_history(
        Scenario::copper("tick-copper-held")
            .unwrap()
            .with_mutation(DispatchMutation::Transport(
                TransportMutation::HoldUntilNextDispatch,
            )),
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
#[ignore = "requires LIVE=1, GATHERER_ENGINE_DIR/NAV_PACK/CATALOG_ROOT, LIVE_EVIDENCE_DIR and Engine L 127.0.0.1:47594/3080"]
fn tick_engine_removed_target_refuses_prefix_zero_and_continues() {
    let scenario = Scenario::river("tick-target-removed").with_mutation(
        DispatchMutation::RemoveLoc {
            id: RIVER_LOC_ID,
            tile: tile(2_501, 3_506, 0),
        },
    );
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
                && statuses.last().is_some_and(|status| status.tick_lost.unwrap_or(0) >= 1)
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
#[ignore = "requires LIVE=1, GATHERER_ENGINE_DIR/NAV_PACK/CATALOG_ROOT, LIVE_EVIDENCE_DIR and Engine L 127.0.0.1:47594/3080"]
fn tick_engine_accepted_make_without_click() {
    let scenario = Scenario::river("tick-make-without-click")
        .with_mutation(DispatchMutation::Transport(
            TransportMutation::DropSuffixAfterPacket(2),
        ));
    run_case(scenario, |run| {
        let first = first_knife(&run.hook.trace()).map_err(|error| run.failure(error))?;
        let origin = first.receipt.dispatch_tick;
        assert_batch(run, &first, 2, 2)?;
        assert_knife_wire_order(
            run,
            &run.hook.trace(),
            &first,
            &[ClientProt289::OPHELDU.id, ClientProt289::IF_BUTTON.id],
        )?;
        run.wait_until(origin as u32, MAKE_WITHOUT_CLICK_BUDGET, |trace, statuses| {
            fresh_animation_on_edge(trace, origin + 1)
                .is_some_and(|(sequence, _)| sequence == FLETCHING_ANIMATION)
                && !has_act_line(trace, origin + 1)
                && click_dispatches_at(trace, origin + 1).len() == 1
                && fresh_animation_on_edge(trace, origin + 3)
                    .is_some_and(|(sequence, _)| sequence == ROLL_ANIMATION)
                && statuses.last().is_some_and(|status| status.tick_cycle == Some(3))
        })?;
        let trace = run.hook.trace();
        assert_click(run, &trace, origin + 1)?;
        assert_roll(run, origin + 3)?;
        if has_act_line(&trace, origin + 1) {
            return Err(run.failure("accepted Make without the click unexpectedly produced the act line"));
        }
        assert_log_unchanged(run)?;
        assert_running(run)
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
            &[ClientProt289::OPHELDU.id],
        )?;
        let baseline = run.baseline.as_ref().expect("started run has baseline").clone();
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
            wire.opcode == ClientProt289::IF_BUTTON.id || wire.opcode == ClientProt289::CLOSE_MODAL.id
        }) {
            return Err(run.failure("lone opener emitted IF_BUTTON or host CLOSE_MODAL"));
        }
        if suppress {
            if !frames_in_tick(&trace, origin + 1)
                .iter()
                .any(|frame| frame.suppressed_openchat && frame.actual_chat_modal_root == 8_880)
            {
                return Err(run.failure("suppressed history did not filter a real IF_OPENCHAT snapshot"));
            }
        } else if !edge_has_openchat(&trace, origin + 1) {
            return Err(run.failure("unsuppressed lone opener never exposed the real 8880 root"));
        }
        assert_log_unchanged(run)?;
        assert_running(run)
    })
}

#[test]
#[ignore = "requires LIVE=1, GATHERER_ENGINE_DIR/NAV_PACK/CATALOG_ROOT, LIVE_EVIDENCE_DIR and Engine L 127.0.0.1:47594/3080"]
fn tick_engine_lone_opener_with_and_without_snapshot_suppression() {
    let visible = lone_opener_history("tick-lone-opener", false)
        .unwrap_or_else(|error| panic!("{error}"));
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
#[ignore = "requires LIVE=1, GATHERER_ENGINE_DIR/NAV_PACK/CATALOG_ROOT, LIVE_EVIDENCE_DIR and selected free-world local world.json"]
fn tick_engine_free_world_disables_members_only_knife_arm() {
    run_case(Scenario::oak("tick-free-world-oak"), |run| {
        let start = run.start_tick.ok_or_else(|| run.failure("missing pre-Start tick"))?;
        run.wait_until(start, FREE_WORLD_BUDGET, |trace, statuses| {
            has_members_line(trace, u64::from(start + FREE_WORLD_BUDGET))
                && checked_click_pairs(trace, u64::from(start + 1)) == 1
                && statuses
                    .last()
                    .and_then(|status| status.tick_method.as_deref())
                    == Some("Knife (off: members-only-arm)")
        })?;
        let trace = run.hook.trace();
        if trace.dispatches.iter().any(is_knife_arm) {
            return Err(run.failure("free-world content emitted a members-only KnifeArm"));
        }
        if trace.wire.iter().any(|wire| {
            wire.opcode == ClientProt289::OPHELDU.id || wire.opcode == ClientProt289::IF_BUTTON.id
        }) {
            return Err(run.failure("free-world content opened a Knife/Make menu"));
        }
        if trace
            .frames
            .iter()
            .any(|frame| frame.actual_chat_modal_root == 8_880)
        {
            return Err(run.failure("free-world content opened an interface menu"));
        }
        if !has_members_line(&trace, u64::from(start + FREE_WORLD_BUDGET)) {
            return Err(run.failure("free-world run did not expose the real members-only message"));
        }
        if checked_click_pairs(&trace, u64::from(start + 1)) != 1 {
            return Err(run.failure("free-world run did not emit exactly one checked click"));
        }
        if run.latest_tick_method() != Some("Knife (off: members-only-arm)") {
            return Err(run.failure("tick_method did not report Knife (off: members-only-arm)"));
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
