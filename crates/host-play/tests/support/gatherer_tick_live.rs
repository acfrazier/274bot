//! S6 Engine L tick probe and qualification cells, deliberately ignored by default.
//! Run one cell at a time:
//! ```sh
//! cargo test -p host-play --test gatherer_live --features live-harness,test-support \
//!   -- --ignored --exact gatherer_tick_live::probe_tick_landing --nocapture
//! ```
//! Replace the final test name for a cell.
//! Live execution requires LIVE=1, BOT_CPU=1, a 1-4 character BOT_LIVE_NAME_PREFIX,
//! GATHERER_GAME_PORT=47594, GATHERER_HTTP_PORT=3080, and absolute
//! GATHERER_NAV_PACK, GATHERER_ENGINE_DIR, GATHERER_CATALOG_ROOT, and
//! GATHERER_TICK_EVIDENCE paths. The evidence destination must not already exist.

use super::{mint_profile, required, selected_profile, TempRoot};
use api::interact;
use api::snapshot::{GameSnapshot, WorldTile};
use client::client::Client;
use client::io::ClientProt289;
use host_play::tick_engine_test_support::{
    ChatObservation, DispatchObservation, FrameObservation, PollKind, TickEngineTestHook,
    WireObservation,
};
use host_play::WorldMembersFact;
use script::native::{ScriptStatus, StatusValue};
use serde_json::{json, Map, Value};
use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[path = "tick_startup.rs"]
mod tick_startup;

const ENGINE_HOST: &str = "127.0.0.1";
const ENGINE_GAME_PORT: u16 = 47_594;
const ENGINE_HTTP_PORT: u16 = 3_080;
const EXPECTED_TICK_MS: u64 = 600;
const KIT_KNIFE_ID: i32 = 946;
const LOG_ID: i32 = 1_511;
const FLY_ROD_ID: i32 = 309;
const FEATHER_ID: i32 = 314;
const NET_ID: i32 = 303;
const MITHRIL_PICK_ID: i32 = 1_273;
const RUNE_PICK_ID: i32 = 1_275;
const IRON_AXE_ID: i32 = 1_349;
const OAK_LOC_ID: i32 = 1_281;
const RIVER_LOC_ID: i32 = 2_027;
const OAK_TILE: WorldTile = WorldTile {
    x: 3_017,
    z: 3_170,
    level: 0,
};
const OAK_START_TILE: WorldTile = WorldTile {
    x: 3_016,
    ..OAK_TILE
};
const RIVER_TILE: WorldTile = WorldTile {
    x: 2_504,
    z: 3_498,
    level: 0,
};
const RIVER_START_TILE: WorldTile = WorldTile {
    x: 2_503,
    ..RIVER_TILE
};
const NET_TILE: WorldTile = WorldTile {
    x: 3_267,
    z: 3_149,
    level: 0,
};
const IRON_TILE: WorldTile = WorldTile {
    x: 3_284,
    z: 3_368,
    level: 0,
};
const MINING_IRON_ROCKS: &[i32] = &[2_092, 2_093];
const REQUIRED_CYCLES: u32 = 60;
const PROBE_CYCLES: u32 = 300;
const PROBE_TICK_BUDGET: u32 = 2_400;
const CELL_TICK_BUDGET: u32 = 800;
const RUNE_XP_TICK_BUDGET: u32 = 60;
const FIXED_DEADLINE: Duration = Duration::from_secs(1_800);
const FRAME_HANG_GUARD: Duration = Duration::from_secs(5);
const SETUP_DEADLINE: Duration = Duration::from_secs(180);
const LOGOUT_DEADLINE: Duration = Duration::from_secs(30);
const STARTUP_TICK_BUDGET: u32 = 2;

const VERDICT_COUNTERS: [(&str, &str); 6] = [
    ("Verified", "tick_verified"),
    ("Unobserved", "tick_unobserved"),
    ("Slip", "tick_slips"),
    ("Lost", "tick_lost"),
    ("Delayed", "tick_delayed"),
    ("Paused", "tick_paused"),
];

static LIVE_RUN_LOCK: Mutex<()> = Mutex::new(());

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Case {
    Probe,
    Fly,
    Net,
    Iron,
    Oak,
    Rune,
}

impl Case {
    fn name(self) -> &'static str {
        match self {
            Self::Probe => "probe_tick_landing",
            Self::Fly => "gatherer_fly_tick",
            Self::Net => "gatherer_net_tick",
            Self::Iron => "gatherer_mine_iron_tick",
            Self::Oak => "gatherer_oak_tick",
            Self::Rune => "gatherer_tick_refused_rune",
        }
    }

    fn skill(self) -> &'static str {
        match self {
            Self::Probe | Self::Fly | Self::Net => "Fishing",
            Self::Iron | Self::Rune => "Mining",
            Self::Oak => "Woodcutting",
        }
    }

    fn skill_index(self) -> i32 {
        match self {
            Self::Probe | Self::Fly | Self::Net => 10,
            Self::Iron | Self::Rune => 14,
            Self::Oak => 8,
        }
    }

    fn level(self) -> i32 {
        match self {
            Self::Probe | Self::Fly => 20,
            Self::Net => 1,
            // Accepted S6b correction: the real mithril tool requires Mining 21.
            Self::Iron => 21,
            Self::Oak => 15,
            // The rune pickaxe's own level gate is 41; this is a real usable
            // iron-rock action whose catalogue row is specifically no-gain.
            Self::Rune => 41,
        }
    }

    fn target(self) -> WorldTile {
        match self {
            Self::Probe | Self::Fly => RIVER_START_TILE,
            Self::Net => NET_TILE,
            Self::Iron | Self::Rune => IRON_TILE,
            Self::Oak => OAK_START_TILE,
        }
    }

    fn tool(self) -> (&'static str, i32) {
        match self {
            Self::Probe | Self::Fly => ("fly_fishing_rod", FLY_ROD_ID),
            Self::Net => ("net", NET_ID),
            Self::Iron => ("mithril_pickaxe", MITHRIL_PICK_ID),
            Self::Oak => ("iron_axe", IRON_AXE_ID),
            Self::Rune => ("rune_pickaxe", RUNE_PICK_ID),
        }
    }

    fn inventory_ids(self) -> Vec<i32> {
        let mut ids = vec![KIT_KNIFE_ID, LOG_ID, self.tool().1];
        match self {
            Self::Probe | Self::Fly => ids.extend([FEATHER_ID, 335, 331]),
            Self::Net => ids.extend([317, 321]),
            Self::Iron | Self::Rune => ids.push(440),
            Self::Oak => ids.push(1_521),
        }
        ids.sort_unstable();
        ids.dedup();
        ids
    }

    fn target_loc(self) -> Option<(i32, WorldTile)> {
        match self {
            Self::Probe | Self::Fly => Some((RIVER_LOC_ID, RIVER_TILE)),
            Self::Oak => Some((OAK_LOC_ID, OAK_TILE)),
            _ => None,
        }
    }

    fn required_cycles(self) -> u32 {
        match self {
            Self::Probe => PROBE_CYCLES,
            Self::Fly | Self::Net | Self::Iron | Self::Oak => REQUIRED_CYCLES,
            Self::Rune => 0,
        }
    }

    fn tick_budget(self) -> u32 {
        match self {
            Self::Probe => PROBE_TICK_BUDGET,
            Self::Fly | Self::Net | Self::Iron | Self::Oak => CELL_TICK_BUDGET,
            Self::Rune => RUNE_XP_TICK_BUDGET,
        }
    }

    fn is_mining(self) -> bool {
        matches!(self, Self::Iron | Self::Rune)
    }

    fn is_rune(self) -> bool {
        self == Self::Rune
    }

    fn settings(self) -> Map<String, Value> {
        let mut settings = Map::new();
        settings.insert("skill".into(), json!(self.skill()));
        settings.insert("tickMethod".into(), json!("Knife"));
        settings.insert(
            "woodcuttingResources".into(),
            json!(if self == Self::Oak {
                vec!["oak"]
            } else {
                Vec::<&str>::new()
            }),
        );
        settings.insert(
            "miningResources".into(),
            json!(if self.is_mining() {
                vec!["iron"]
            } else {
                Vec::<&str>::new()
            }),
        );
        if matches!(self, Self::Probe | Self::Fly | Self::Net) {
            settings.insert(
                "fishingMethod".into(),
                json!(if matches!(self, Self::Probe | Self::Fly) {
                    "fishing.loc_2027.op1"
                } else {
                    "fishing.saltfish.op1"
                }),
            );
        }
        settings.insert(
            "targetPreference".into(),
            json!(if self.is_mining() {
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

    fn seeds(self) -> Vec<(&'static str, i32, i32)> {
        let mut items = vec![("knife", KIT_KNIFE_ID, 1), ("logs", LOG_ID, 1)];
        let (alias, tool) = self.tool();
        items.push((alias, tool, 1));
        match self {
            Self::Probe | Self::Fly => items.push(("feather", FEATHER_ID, 1_200)),
            _ => {}
        }
        items
    }
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

struct SetupState {
    account: String,
    case: Case,
    phase: SetupPhase,
    commands: Vec<String>,
    next_command: usize,
    error: Option<String>,
    logout_session: Option<u64>,
    tutorial_reseed: Option<scenario::tutorial::PostRelogTutorial>,
}

impl SetupState {
    fn new(account: String, case: Case) -> Self {
        let mut commands = vec![
            "setstat attack 1".to_owned(),
            "setstat defence 99".to_owned(),
            "setstat hitpoints 99".to_owned(),
            format!("setstat {} {}", case.skill().to_lowercase(), case.level()),
        ];
        if matches!(case, Case::Probe | Case::Fly) {
            commands.push("setstat fletching 1".to_owned());
        }
        for (alias, _, count) in case.seeds() {
            commands.push(format!("give {alias} {count}"));
        }
        let tile = case.target();
        commands.push(interact::tele_args(tile.level, tile.x, tile.z));
        Self {
            account,
            case,
            phase: SetupPhase::WaitIngame,
            commands,
            next_command: 0,
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
                        self.error = Some(format!(
                            "engine refused own-account setup cheat {command:?}"
                        ));
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
        let target = self.case.target();
        let at_target = snapshot
            .tile()
            .is_some_and(|(x, z, level)| (x, z, level) == (target.x, target.z, target.level));
        let level_ready = snapshot
            .stats()
            .iter()
            .find(|stat| stat.index == self.case.skill_index())
            .is_some_and(|stat| stat.base >= self.case.level());
        let items_ready = self
            .case
            .seeds()
            .iter()
            .all(|(_, id, count)| snapshot.inv_count(*id) >= *count);
        let loc_ready = self.case.target_loc().is_none_or(|(id, tile)| {
            snapshot
                .locs()
                .iter()
                .any(|loc| loc.id == id && loc.tile == tile)
        });
        let iron_rocks_present = !self.case.is_mining()
            || snapshot
                .locs()
                .iter()
                .any(|loc| MINING_IRON_ROCKS.contains(&loc.id));
        at_target && level_ready && items_ready && loc_ready && iron_rocks_present
    }
}

#[derive(Debug, Clone)]
struct StatusObservation {
    host_frame: u64,
    observed_tick: u32,
    phase: String,
    method: Option<String>,
    tick_method: Option<String>,
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
    xp: Option<i64>,
}

impl StatusObservation {
    fn capture(status: &ScriptStatus, host_frame: u64, observed_tick: u32) -> Self {
        Self {
            host_frame,
            observed_tick,
            phase: format!("{:?}", status.phase),
            method: text_field(status, "method"),
            tick_method: text_field(status, "tick_method"),
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
            xp: integer_field(status, "xp"),
        }
    }

    fn completed_cycles(&self) -> Result<u32, String> {
        VERDICT_COUNTERS.iter().try_fold(0_u32, |total, (_, key)| {
            let count = self
                .counter(key)
                .ok_or_else(|| format!("Gatherer status missing required integer field {key}"))?;
            let count =
                u32::try_from(count).map_err(|_| format!("Gatherer status {key} was negative"))?;
            total
                .checked_add(count)
                .ok_or_else(|| "Gatherer verdict counter overflow".to_owned())
        })
    }

    fn counter(&self, key: &str) -> Option<i64> {
        match key {
            "tick_verified" => self.tick_verified,
            "tick_unobserved" => self.tick_unobserved,
            "tick_slips" => self.tick_slips,
            "tick_lost" => self.tick_lost,
            "tick_delayed" => self.tick_delayed,
            "tick_paused" => self.tick_paused,
            "tick_events" => self.tick_events,
            _ => None,
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

struct LiveRun {
    _isolated_home: script::IsolatedEnv,
    _temp: TempRoot,
    case: Case,
    account: String,
    password: Option<String>,
    profile_identity: String,
    world_members: String,
    setup: Arc<Mutex<SetupState>>,
    hook: Arc<TickEngineTestHook>,
    play: Option<host_play::Play>,
    output: PathBuf,
    started_tick: Option<u32>,
    first_arm_budget: u32,
    baseline: Option<FrameObservation>,
    statuses: Vec<StatusObservation>,
    logout_requested: bool,
    logout_still_connected: Option<bool>,
    shutdown: bool,
}

impl LiveRun {
    fn new(case: Case) -> Result<Self, String> {
        if std::env::var("LIVE").as_deref() != Ok("1") {
            return Err("set LIVE=1 when explicitly running this ignored Engine L cell".into());
        }
        if std::env::var("BOT_CPU").as_deref() != Ok("1") {
            return Err("the live tick cells require BOT_CPU=1".into());
        }
        let prefix = required("BOT_LIVE_NAME_PREFIX")?;
        let prefix_chars = prefix.chars().count();
        if !(1..=4).contains(&prefix_chars) {
            return Err("BOT_LIVE_NAME_PREFIX must contain 1-4 characters".into());
        }
        if std::env::var("GATHERER_GAME_PORT").as_deref() != Ok("47594")
            || std::env::var("GATHERER_HTTP_PORT").as_deref() != Ok("3080")
        {
            return Err("the assigned Engine L cell requires GATHERER_GAME_PORT=47594 and GATHERER_HTTP_PORT=3080".into());
        }
        let output = PathBuf::from(required("GATHERER_TICK_EVIDENCE")?);
        if !output.is_absolute() {
            return Err("GATHERER_TICK_EVIDENCE must be an absolute path".into());
        }
        if output.exists() {
            return Err(format!(
                "refusing to overwrite tick evidence {}",
                output.display()
            ));
        }
        let output_parent = output
            .parent()
            .ok_or("GATHERER_TICK_EVIDENCE has no parent directory")?;
        std::fs::create_dir_all(output_parent).map_err(|error| {
            format!(
                "create evidence directory {}: {error}",
                output_parent.display()
            )
        })?;
        let _isolated_home = script::IsolatedEnv::enter(case.name());
        api::hostlog::set_debug(true);
        let _temp = TempRoot::new(case.name())?;
        let nav_pack = absolute_path("GATHERER_NAV_PACK")?;
        let engine_dir = absolute_path("GATHERER_ENGINE_DIR")?;
        let catalog_root = absolute_path("GATHERER_CATALOG_ROOT")?;
        let (server_profile, template) =
            selected_profile(nav_pack, engine_dir, catalog_root, _temp.path(), None)?;
        if server_profile.profile_class() != host_play::ProfileClass::Local
            || server_profile.client().game_host() != ENGINE_HOST
            || server_profile.client().asset_host() != ENGINE_HOST
            || server_profile.client().game_port() != ENGINE_GAME_PORT
            || server_profile.client().asset_port() != ENGINE_HTTP_PORT
        {
            return Err("live tick cells require the assigned local Engine L loopback profile on 47594/3080".into());
        }
        if !matches!(
            server_profile.world_members(),
            WorldMembersFact::Known { members: true, .. }
        ) {
            return Err(format!(
                "Gatherer Knife arm requires an observed members world; got {:?}",
                server_profile.world_members()
            ));
        }
        if template.world().is_none() {
            return Err("selected Engine L navigation template has no world".into());
        }
        let first_arm_budget = tick_startup::first_arm_budget(
            server_profile
                .game_data()
                .ok_or("Engine L profile has no selected data")?,
            match case {
                Case::Probe | Case::Fly => "fishing.loc_2027.op1",
                Case::Net => "fishing.saltfish.op1",
                Case::Iron | Case::Rune => "mining.iron",
                Case::Oak => "woodcutting.oak",
            },
            case.tool().1,
            STARTUP_TICK_BUDGET,
        )?;
        // Asset initialization is not a live host frame or a cadence observation.
        // Complete it before the slot's fixed setup deadline starts.
        let mut bootstrap = template.prepare_client(0, true)?;
        bootstrap.set_draw(false);
        bootstrap.maininit();
        if bootstrap.error_loading {
            return Err("Engine L client asset initialization failed before live setup".into());
        }
        drop(bootstrap);
        let profile_identity = format!(
            "{} game={}:{} asset={}:{} nav={}",
            server_profile.label(),
            server_profile.client().game_host(),
            server_profile.client().game_port(),
            server_profile.client().asset_host(),
            server_profile.client().asset_port(),
            server_profile.nav_pack().display()
        );
        let world_members = format!("{:?}", server_profile.world_members());
        let names = host_play::mint_live_names(1);
        let account = names
            .first()
            .cloned()
            .ok_or("live account mint returned no name")?;
        if !account.starts_with(&prefix) {
            return Err("minted live account did not retain BOT_LIVE_NAME_PREFIX".into());
        }
        let (minted_account, password) = host_play::mint_live_entries(&names)
            .into_iter()
            .next()
            .ok_or("live credential mint returned no account")?;
        if minted_account != account {
            return Err("live credential mint changed the account name".into());
        }
        let setup = Arc::new(Mutex::new(SetupState::new(account.clone(), case)));
        let hook = TickEngineTestHook::new(account.clone());
        hook.install();
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
                        .expect("live tick setup poisoned")
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
            _isolated_home,
            _temp,
            case,
            account,
            password: Some(password),
            profile_identity,
            world_members,
            setup,
            hook,
            play: Some(play),
            output,
            started_tick: None,
            first_arm_budget,
            baseline: None,
            statuses: Vec::new(),
            logout_requested: false,
            logout_still_connected: None,
            shutdown: false,
        })
    }

    fn play(&self) -> &host_play::Play {
        self.play.as_ref().expect("live tick Play missing")
    }

    fn play_mut(&mut self) -> &mut host_play::Play {
        self.play.as_mut().expect("live tick Play missing")
    }

    fn start(&mut self) -> Result<(), String> {
        let password = self
            .password
            .take()
            .ok_or("live password was already consumed")?;
        let profile = mint_profile(&self.account, &password, 0)?;
        self.play_mut()
            .try_spawn_slot(profile, None, None, None)
            .map_err(|error| format!("spawn Engine L account: {error}"))?;
        self.play
            .as_mut()
            .expect("live tick Play missing")
            .focus(&self.account);
        self.wait_setup_ready()?;
        let after = self.hook.observed_host_frame();
        self.hook
            .arm_measurement(LOG_ID, &self.case.inventory_ids());
        if self.hook.wait_for_frame(after, FRAME_HANG_GUARD).is_none() {
            return Err("no host frame after arming live tick evidence".into());
        }
        let baseline = self
            .hook
            .trace()
            .frames
            .last()
            .cloned()
            .ok_or("live tick hook did not capture its pre-Start baseline")?;
        self.started_tick = Some(baseline.game_tick);
        self.baseline = Some(baseline.clone());
        println!(
            "TICK-MANIP start case={} account={} profile={} world_members={} baseline_tick={} target={:?} seed_items={:?}",
            self.case.name(), self.account, self.profile_identity, self.world_members,
            baseline.game_tick, self.case.target(), self.case.seeds()
        );
        self.play
            .as_mut()
            .expect("live tick Play missing")
            .script_start(
                &self.account,
                script::CompiledId("Gatherer"),
                self.case.settings(),
            )
            .map_err(|error| format!("Gatherer operator Start failed: {error}"))?;
        self.wait_started()
    }

    fn wait_setup_ready(&self) -> Result<(), String> {
        let deadline = Instant::now() + SETUP_DEADLINE;
        let mut after = self.hook.observed_host_frame();
        loop {
            {
                let setup = self.setup.lock().map_err(|_| "live tick setup poisoned")?;
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
                let setup = self.setup.lock().map_err(|_| "live tick setup poisoned")?;
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

    fn wait_started(&mut self) -> Result<(), String> {
        let origin = self
            .started_tick
            .ok_or("Gatherer Start has no baseline tick")?;
        let mut after = self.hook.observed_host_frame();
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            self.capture_status();
            let trace = self.hook.trace();
            let current = self.hook.observed_game_tick();
            if !self.case.is_rune()
                && current.wrapping_sub(origin) > STARTUP_TICK_BUDGET
                && !tick_startup::native_click_within(&trace, origin, STARTUP_TICK_BUDGET)
            {
                return Err(format!("Gatherer Start did not emit its native click within {STARTUP_TICK_BUDGET} ticks"));
            }
            if self.case.is_rune() {
                if let Some(status) = self.statuses.iter().find(|status| {
                    status.tick_method.as_deref() == Some("Knife (off: no-gain)")
                        && status.observed_tick.wrapping_sub(origin) <= 1
                }) {
                    if trace.dispatches.iter().any(is_knife_arm) {
                        return Err(
                            "no-gain Rune cell emitted a KnifeArm instead of refusing it".into(),
                        );
                    }
                    if self.play().script_state(&self.account) != script::RunState::Running {
                        return Err(
                            "Gatherer did not remain Running during the Rune no-gain check".into(),
                        );
                    }
                    println!(
                        "TICK-MANIP rune_no_gain tick={} tick_method={:?}",
                        status.observed_tick, status.tick_method
                    );
                    return Ok(());
                }
            } else if trace.dispatches.iter().any(is_knife_arm) {
                if self.play().script_state(&self.account) != script::RunState::Running {
                    return Err("Gatherer native loop did not reach Running after Start".into());
                }
                return Ok(());
            }
            if current.wrapping_sub(origin) > self.first_arm_budget {
                return Err(format!(
                    "Gatherer Start did not reach its first arm within the selected native-roll bound of {} ticks",
                    self.first_arm_budget
                ));
            }
            if Instant::now() >= deadline {
                return Err("Gatherer Start exceeded its bounded 30s startup deadline".into());
            }
            after = self
                .hook
                .wait_for_frame(after, FRAME_HANG_GUARD)
                .ok_or_else(|| {
                    "live slot stopped producing frames while waiting for Gatherer Start".to_owned()
                })?;
        }
    }

    fn measure(&mut self) -> Result<(), String> {
        let start_tick = self.started_tick.ok_or("live tick run did not start")?;
        let deadline = Instant::now() + FIXED_DEADLINE;
        let required = self.case.required_cycles();
        let mut after = self.hook.observed_host_frame();
        loop {
            self.capture_status();
            if let Some(status) = self.statuses.last() {
                if self.case.is_rune() {
                    if self.xp_gain() > 0 {
                        let trace = self.hook.trace();
                        if trace.dispatches.iter().any(is_knife_arm) {
                            return Err("Rune no-gain cell emitted KnifeArm while gathering".into());
                        }
                        return Ok(());
                    }
                } else if status.completed_cycles()? >= required && self.xp_gain() > 0 {
                    return Ok(());
                }
            }
            let elapsed_ticks = self.hook.observed_game_tick().wrapping_sub(start_tick);
            if elapsed_ticks > self.case.tick_budget() {
                return Err(format!(
                    "{} did not reach its required cycles/XP by tick {} (budget {})",
                    self.case.name(),
                    self.hook.observed_game_tick(),
                    self.case.tick_budget()
                ));
            }
            if Instant::now() >= deadline {
                return Err(format!(
                    "{} exceeded its fixed bounded 1800s deadline",
                    self.case.name()
                ));
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            after = self
                .hook
                .wait_for_frame(after, remaining.min(FRAME_HANG_GUARD))
                .ok_or_else(|| {
                    format!(
                        "{} stopped producing live frames before its bounded deadline",
                        self.case.name()
                    )
                })?;
        }
    }

    fn capture_status(&mut self) {
        let Some(status) = self.play().script_native_status(&self.account) else {
            return;
        };
        let host_frame = self.hook.observed_host_frame();
        if self
            .statuses
            .last()
            .is_some_and(|previous| previous.host_frame == host_frame)
        {
            return;
        }
        self.statuses.push(StatusObservation::capture(
            &status,
            host_frame,
            self.hook.observed_game_tick(),
        ));
    }

    fn xp_gain(&self) -> i64 {
        let Some(baseline) = &self.baseline else {
            return 0;
        };
        let baseline_xp = skill_xp(baseline, self.case.skill_index()).unwrap_or(0);
        self.hook
            .observed_skill_xp(self.case.skill_index())
            .map(i64::from)
            .unwrap_or(baseline_xp)
            - baseline_xp
    }

    fn write_evidence(&self) -> Result<(), String> {
        if self.output.exists() {
            return Err(format!(
                "refusing to overwrite tick evidence {}",
                self.output.display()
            ));
        }
        let trace = self.hook.trace();
        let frames: Vec<Value> = trace.frames.iter().map(frame_json).collect();
        let dispatches: Vec<Value> = trace.dispatches.iter().map(dispatch_json).collect();
        let wire: Vec<Value> = trace.wire.iter().map(wire_json).collect();
        let statuses: Vec<Value> = self.statuses.iter().map(status_json).collect();
        let baseline_tick = self.started_tick.map(i64::from);
        let value = json!({
            "schema": "gatherer-tick-run/v1",
            "case": self.case.name(),
            "kind": match self.case { Case::Probe => "probe", Case::Fly => "fly", Case::Net => "net", Case::Iron => "iron", Case::Oak => "oak", Case::Rune => "rune" },
            "engine": {"host": ENGINE_HOST, "game_port": ENGINE_GAME_PORT, "http_port": ENGINE_HTTP_PORT, "tick_ms": EXPECTED_TICK_MS},
            "runtime": {"bot_cpu": 1, "account_prefix": std::env::var("BOT_LIVE_NAME_PREFIX").ok(), "account": self.account, "profile": self.profile_identity, "world_members": self.world_members},
            "scenario": {"skill": self.case.skill(), "skill_index": self.case.skill_index(), "seed_level": self.case.level(), "target": {"x": self.case.target().x, "z": self.case.target().z, "level": self.case.target().level}, "required_cycles": self.case.required_cycles(), "tick_budget": self.case.tick_budget(), "fixed_deadline_seconds": FIXED_DEADLINE.as_secs(), "inventory_seed": self.case.seeds()},
            "measurement": {"started_tick": baseline_tick, "baseline": self.baseline.as_ref().map(frame_json), "xp_gain": self.xp_gain(), "logout_requested": self.logout_requested, "logout_still_connected": self.logout_still_connected},
            "timeline": {"frames": frames, "dispatches": dispatches, "wire": wire, "statuses": statuses},
            "server_acceptance": {"observed": false, "accepted_events": null, "reason": "Engine L has no independent accepted-event history in this live hook; HostQueued receipts and outbound Client packets are not acceptance evidence."}
        });
        let bytes = serde_json::to_vec_pretty(&value)
            .map_err(|error| format!("serialize live tick evidence: {error}"))?;
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&self.output)
            .map_err(|error| {
                format!(
                    "create live tick evidence {}: {error}",
                    self.output.display()
                )
            })?;
        file.write_all(&bytes).map_err(|error| {
            format!(
                "write live tick evidence {}: {error}",
                self.output.display()
            )
        })?;
        file.write_all(b"\n").map_err(|error| {
            format!(
                "finish live tick evidence {}: {error}",
                self.output.display()
            )
        })?;
        println!(
            "TICK-MANIP evidence={} case={} cycles={} xp_gain={}",
            self.output.display(),
            self.case.name(),
            self.statuses
                .last()
                .and_then(|status| status.completed_cycles().ok())
                .unwrap_or(0),
            self.xp_gain()
        );
        Ok(())
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
            self.logout_requested = true;
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
        self.logout_still_connected = Some(still_connected);
        play.stop_slot(&self.account);
        self.hook.uninstall();
        self.play.take();
        self.shutdown = true;
        if still_connected {
            return Err(format!(
                "account {} did not log out within the fixed {LOGOUT_DEADLINE:?} deadline",
                self.account
            ));
        }
        Ok(())
    }
}

impl Drop for LiveRun {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}

fn run_live(case: Case) -> Result<(), String> {
    let _serial = LIVE_RUN_LOCK
        .lock()
        .map_err(|_| "live tick-run lock poisoned")?;
    let mut run = LiveRun::new(case)?;
    let operation = run.start().and_then(|()| run.measure());
    let shutdown = run.shutdown();
    let evidence = run.write_evidence();
    match (operation, shutdown, evidence) {
        (Ok(()), Ok(()), Ok(())) => Ok(()),
        (operation, shutdown, evidence) => Err(format!(
            "live tick cell {} failed: operation={:?}; logout={:?}; evidence={:?}",
            case.name(),
            operation.err(),
            shutdown.err(),
            evidence.err()
        )),
    }
}

fn absolute_path(name: &str) -> Result<PathBuf, String> {
    let value = PathBuf::from(required(name)?);
    if !value.is_absolute() {
        return Err(format!("{name} must be an absolute path"));
    }
    Ok(value)
}

fn is_knife_arm(dispatch: &DispatchObservation) -> bool {
    dispatch
        .request_debug
        .as_deref()
        .is_some_and(|request| request.contains("KnifeArm"))
}

fn skill_xp(frame: &FrameObservation, skill: i32) -> Option<i64> {
    frame
        .skill_xp
        .iter()
        .find(|(index, _)| *index == skill)
        .map(|(_, xp)| i64::from(*xp))
}

fn frame_json(frame: &FrameObservation) -> Value {
    let animation = frame
        .animation
        .map(|animation| json!({"sequence": animation.sequence, "serial": animation.serial}));
    let chat: Vec<Value> = frame.chat.iter().map(chat_json).collect();
    let inventory: Vec<Value> = frame
        .inventory_counts
        .iter()
        .map(|(id, count)| json!({"id": id, "count": count}))
        .collect();
    let xp: Vec<Value> = frame
        .skill_xp
        .iter()
        .map(|(index, xp)| json!({"skill_index": index, "xp": xp}))
        .collect();
    json!({
        "host_frame": frame.host_frame,
        "observed_tick": frame.game_tick,
        "client_snapshot_tick": frame.client_tick,
        "poll": if frame.tick_edge {"Edge"} else {"Wake"},
        "animation": animation,
        "chat": chat,
        "inventory": inventory,
        "log_count": frame.log_count,
        "skill_xp": xp,
        "player_tile": frame.player_tile.map(|tile| json!({"x": tile.x, "z": tile.z, "level": tile.level})),
        "chat_modal_root": frame.chat_modal_root,
        "actual_chat_modal_root": frame.actual_chat_modal_root,
        "server_if_close_packets": frame.server_if_close_packets,
        "main_modal_opened_observation": frame.main_modal_opened_observation,
        "main_modal_closed_observation": frame.main_modal_closed_observation,
        "suppressed_openchat": frame.suppressed_openchat
    })
}

fn chat_json(chat: &ChatObservation) -> Value {
    let interval = json!([
        chat.first_attributable_tick(),
        u64::from(chat.game_tick) + 1
    ]);
    json!({
        "observed_tick": chat.game_tick,
        "sequence": chat.sequence,
        "text": chat.text,
        "poll": format!("{:?}", chat.poll),
        "player_info_attribution_interval": interval,
        "attribution_basis": if chat.poll == PollKind::Wake {"SPEC §2.2: wake [t+1,t+1] is an inference from single-socket ordering"} else {"SPEC §2.2: edge chat [t,t+1]"}
    })
}

fn dispatch_json(dispatch: &DispatchObservation) -> Value {
    let receipt = &dispatch.receipt;
    json!({
        "native_dispatch_tick": dispatch.game_tick,
        "request_id": receipt.request_id,
        "dispatch_tick": receipt.dispatch_tick,
        "row_count": receipt.row_count,
        "dispatched_prefix": receipt.dispatched_prefix,
        "wire_events": receipt.wire_events,
        "terminal": receipt.terminal.map(|terminal| format!("{:?}", terminal)),
        "rows": format!("{:?}", &receipt.rows[..usize::from(receipt.row_count)]),
        "request_debug": dispatch.request_debug,
        "mutation_applied": dispatch.mutation_applied
    })
}

fn wire_json(wire: &WireObservation) -> Value {
    json!({
        "host_frame": wire.host_frame,
        "observed_tick": wire.game_tick,
        "request_id": wire.request_id,
        "opcode": wire.opcode,
        "opcode_name": opcode_name(wire.opcode),
        "encoded_length": wire.encoded_length
    })
}

fn status_json(status: &StatusObservation) -> Value {
    json!({
        "host_frame": status.host_frame,
        "observed_tick": status.observed_tick,
        "phase": status.phase,
        "method": status.method,
        "tick_method": status.tick_method,
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
        "xp": status.xp
    })
}

fn opcode_name(opcode: u8) -> &'static str {
    let candidates = [
        (ClientProt289::OPNPC1.id as u8, "OPNPC1"),
        (ClientProt289::OPLOC1.id as u8, "OPLOC1"),
        (ClientProt289::MOVE_OPCLICK.id as u8, "MOVE_OPCLICK"),
        (ClientProt289::OPHELDU.id as u8, "OPHELDU"),
        (ClientProt289::IF_BUTTON.id as u8, "IF_BUTTON"),
        (ClientProt289::CLOSE_MODAL.id as u8, "CLOSE_MODAL"),
        (ClientProt289::CLIENT_CHEAT.id as u8, "CLIENT_CHEAT"),
        (ClientProt289::MOVE_GAMECLICK.id as u8, "MOVE_GAMECLICK"),
    ];
    candidates
        .iter()
        .find_map(|(id, name)| (*id == opcode).then_some(*name))
        .unwrap_or("OTHER")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iron_setup_has_a_usable_tool_and_local_replacement_rocks() {
        let selected = api::game_data::for_revision(api::selected::ClientRevision::R289).unwrap();
        let catalog =
            api::selected::FamilyPreparation::run(move |worker| selected.prepare_gathering(worker))
                .unwrap()
                .join()
                .unwrap()
                .unwrap();
        let method = catalog
            .methods()
            .iter()
            .find(|method| method.id.0.as_ref() == "mining.iron")
            .unwrap();
        let tool = api::gather_methods::known_rows(&method.tools)
            .iter()
            .find(|tool| tool.item == Case::Iron.tool().1)
            .unwrap();
        let gate = tool.use_gate.as_ref().unwrap();
        let setup = SetupState::new("iron-minimum".into(), Case::Iron);
        let seeded_level = setup
            .commands
            .iter()
            .find_map(|command| command.strip_prefix("setstat mining "))
            .unwrap()
            .parse::<u16>()
            .unwrap();
        assert!(
            seeded_level >= gate.level,
            "the generated account setup cannot use its selected mithril tool"
        );
        let start = Case::Iron.target();
        let adjacent_rocks = api::gather_methods::known_rows(&method.spots)
            .iter()
            .filter(|spot| {
                spot.origin.level == start.level
                    && spot.origin.x.abs_diff(start.x) <= 2
                    && spot.origin.z.abs_diff(start.z) <= 2
            })
            .count();
        assert!(
            adjacent_rocks >= 2,
            "the seeded player cannot start locally and hand off to a second authored iron rock"
        );
    }

    #[test]
    fn every_live_setup_starts_on_a_standable_player_tile() {
        let pack = PathBuf::from(std::env::var_os("NAV_PACK").expect("NAV_PACK is required"));
        let world = nav_planner::world::NavWorld::load_pack(&pack).unwrap();
        for case in [
            Case::Probe,
            Case::Fly,
            Case::Net,
            Case::Iron,
            Case::Oak,
            Case::Rune,
        ] {
            let start = case.target();
            assert!(
                world.collision.standable(start),
                "{case:?}: {start:?}; west={} east={} north={} south={}",
                world.collision.standable(WorldTile {
                    x: start.x - 1,
                    ..start
                }),
                world.collision.standable(WorldTile {
                    x: start.x + 1,
                    ..start
                }),
                world.collision.standable(WorldTile {
                    z: start.z + 1,
                    ..start
                }),
                world.collision.standable(WorldTile {
                    z: start.z - 1,
                    ..start
                }),
            );
        }
    }

    #[test]
    fn fixed_loc_setups_start_on_real_standable_tiles() {
        let pack = PathBuf::from(std::env::var_os("NAV_PACK").expect("NAV_PACK is required"));
        let world = nav_planner::world::NavWorld::load_pack(&pack).unwrap();
        for case in [Case::Probe, Case::Fly, Case::Oak] {
            let start = case.target();
            let (_, loc) = case.target_loc().expect("fixed resource loc");
            assert!(world.collision.standable(start), "{case:?}: {start:?}");
            assert_ne!(start, loc);
            assert_eq!(start.level, loc.level);
            assert_eq!((start.x - loc.x).abs().max((start.z - loc.z).abs()), 1);
        }
    }

    #[test]
    fn setup_uses_the_engine_teleport_coordinate_format() {
        for case in [
            Case::Probe,
            Case::Fly,
            Case::Net,
            Case::Iron,
            Case::Oak,
            Case::Rune,
        ] {
            let setup = SetupState::new("tick-setup".into(), case);
            let target = case.target();
            assert_eq!(
                setup.commands.last(),
                Some(&interact::tele_args(target.level, target.x, target.z)),
                "incorrect teleport for {case:?}"
            );
        }
    }

    #[test]
    fn completed_cycles_adds_all_six_verdict_families() {
        let status = StatusObservation {
            host_frame: 1,
            observed_tick: 1,
            phase: String::new(),
            method: None,
            tick_method: None,
            last_event: None,
            tick_cycle: Some(3),
            tick_verified: Some(20),
            tick_unobserved: Some(5),
            tick_slips: Some(2),
            tick_lost: Some(1),
            tick_delayed: Some(3),
            tick_paused: Some(1),
            tick_fallbacks: Some(0),
            tick_fallback_failed: Some(0),
            tick_events: Some(0),
            xp: Some(0),
        };
        assert_eq!(status.completed_cycles().unwrap(), 32);
    }

    #[test]
    fn completed_cycles_rejects_missing_or_negative_counter() {
        let mut status = StatusObservation {
            host_frame: 1,
            observed_tick: 1,
            phase: String::new(),
            method: None,
            tick_method: None,
            last_event: None,
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
            xp: Some(0),
        };
        assert!(status.completed_cycles().is_ok());
        status.tick_lost = None;
        assert!(status.completed_cycles().is_err());
        status.tick_lost = Some(-1);
        assert!(status.completed_cycles().is_err());
    }

    #[test]
    fn opcode_names_match_revision_289_user_action_rows() {
        assert_eq!(
            opcode_name(ClientProt289::MOVE_OPCLICK.id as u8),
            "MOVE_OPCLICK"
        );
        assert_eq!(opcode_name(ClientProt289::OPNPC1.id as u8), "OPNPC1");
        assert_eq!(opcode_name(ClientProt289::OPLOC1.id as u8), "OPLOC1");
        assert_eq!(opcode_name(ClientProt289::OPHELDU.id as u8), "OPHELDU");
        assert_eq!(opcode_name(ClientProt289::IF_BUTTON.id as u8), "IF_BUTTON");
    }
}

#[test]
#[ignore = "requires LIVE=1, BOT_CPU=1, BOT_LIVE_NAME_PREFIX, Engine L at 127.0.0.1:47594/3080, and absolute Gatherer data paths; records a 300-cycle probe only"]
fn probe_tick_landing() {
    run_live(Case::Probe).unwrap();
}

#[test]
#[ignore = "requires LIVE=1, BOT_CPU=1, BOT_LIVE_NAME_PREFIX, Engine L at 127.0.0.1:47594/3080, and absolute Gatherer data paths; 60-cycle fly-fishing cell"]
fn gatherer_fly_tick() {
    run_live(Case::Fly).unwrap();
}

#[test]
#[ignore = "requires LIVE=1, BOT_CPU=1, BOT_LIVE_NAME_PREFIX, Engine L at 127.0.0.1:47594/3080, and absolute Gatherer data paths; 60-cycle small-net cell"]
fn gatherer_net_tick() {
    run_live(Case::Net).unwrap();
}

#[test]
#[ignore = "requires LIVE=1, BOT_CPU=1, BOT_LIVE_NAME_PREFIX, Engine L at 127.0.0.1:47594/3080, and absolute Gatherer data paths; 60-cycle iron-mining cell"]
fn gatherer_mine_iron_tick() {
    run_live(Case::Iron).unwrap();
}

#[test]
#[ignore = "requires LIVE=1, BOT_CPU=1, BOT_LIVE_NAME_PREFIX, Engine L at 127.0.0.1:47594/3080, and absolute Gatherer data paths; 60-cycle oak cell"]
fn gatherer_oak_tick() {
    run_live(Case::Oak).unwrap();
}

#[test]
#[ignore = "requires LIVE=1, BOT_CPU=1, BOT_LIVE_NAME_PREFIX, Engine L at 127.0.0.1:47594/3080, and absolute Gatherer data paths; rune-pick iron no-gain guard"]
fn gatherer_tick_refused_rune() {
    run_live(Case::Rune).unwrap();
}

#[test]
fn setup_observes_relogin_without_requiring_an_offline_callback() {
    let account = "tick-setup";
    let mut setup = SetupState::new(account.into(), Case::Probe);
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
