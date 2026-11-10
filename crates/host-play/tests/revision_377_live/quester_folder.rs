//! Generic revision-377 Quester cell backed by a runtime Path folder.
//!
//! The cell is built once. `QUESTER_FOLDER` and `QUESTER_QUEST` are required.
//! Optional `ENGINE_SLOT` (1-4) or `QUESTER_GAME_PORT` + `QUESTER_ASSET_PORT`
//! select the endpoint; `QUESTER_SEEDS` and `QUESTER_ASSERT` name JSON files.
//! `QUESTER_DANGER_ROUTING` is `always` by default or may be set to `off`.
//! Production Quester Start reloads the enabled folder source.
use super::*;
use host_play::local_state::{LocalStateService, PreferencePatch, StateRoot, Update};
use host_play::WalkGlobals;
use script::native::{NativePhase, ScriptStatus, StatusValue};
use script::quester::registry::{self, FolderSource, PathSource};
use script::{ScriptLifecycleReceipt, ScriptTerminalState};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::Write;
use std::sync::Mutex;
use vault::{Profile, ProfileSettings};

const DEFAULT_BANK_TILE: WorldTile = WorldTile {
    x: 3185,
    z: 3440,
    level: 0,
};
const LUMBRIDGE_REGION: i32 = 50;
const PREP_BUDGET: Duration = Duration::from_secs(180);
const RUN_BUDGET: Duration = Duration::from_secs(2700);
const READBACK_RETRY_TICKS: u32 = 5;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
struct Endpoint {
    slot: Option<u8>,
    game_port: u16,
    asset_port: u16,
}

fn parse_endpoint(
    slot: Option<&str>,
    game_port: Option<&str>,
    asset_port: Option<&str>,
) -> Result<Endpoint, String> {
    match (slot, game_port, asset_port) {
        (Some(_), Some(_), _) | (Some(_), _, Some(_)) => {
            Err("set ENGINE_SLOT or both explicit ports, not both".into())
        }
        (Some(value), None, None) => {
            let slot = value
                .parse::<u8>()
                .map_err(|_| "ENGINE_SLOT must be 1, 2, 3 or 4")?;
            let (game_port, asset_port) = match slot {
                1 => (GAME_PORT, ASSET_PORT),
                2..=4 => (
                    GAME_PORT + u16::from(slot - 1),
                    ASSET_PORT + u16::from(slot - 1),
                ),
                _ => return Err("ENGINE_SLOT must be 1, 2, 3 or 4".into()),
            };
            Ok(Endpoint {
                slot: Some(slot),
                game_port,
                asset_port,
            })
        }
        (None, Some(game), Some(asset)) => {
            let game_port = parse_port(game, "QUESTER_GAME_PORT")?;
            let asset_port = parse_port(asset, "QUESTER_ASSET_PORT")?;
            Ok(Endpoint {
                slot: None,
                game_port,
                asset_port,
            })
        }
        (None, None, None) => Ok(Endpoint {
            slot: Some(1),
            game_port: GAME_PORT,
            asset_port: ASSET_PORT,
        }),
        _ => Err("set both QUESTER_GAME_PORT and QUESTER_ASSET_PORT".into()),
    }
}

fn parse_port(value: &str, name: &str) -> Result<u16, String> {
    let port = value
        .parse::<u16>()
        .map_err(|_| format!("{name} must be a valid nonzero port"))?;
    if port == 0 {
        return Err(format!("{name} must be a valid nonzero port"));
    }
    Ok(port)
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct SeedSpec {
    #[serde(default)]
    position: Option<PositionSeed>,
    #[serde(default)]
    inventory: Vec<ItemSeed>,
    #[serde(default)]
    bank: Vec<ItemSeed>,
    #[serde(default)]
    stats: Vec<StatSeed>,
    #[serde(default)]
    quest_varps: Vec<VarpSeed>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct PositionSeed {
    x: i32,
    z: i32,
    level: i32,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ItemSeed {
    item: String,
    count: i32,
    #[serde(default)]
    label: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct StatSeed {
    skill: String,
    level: i32,
    #[serde(default)]
    label: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct VarpSeed {
    name: String,
    value: i32,
    #[serde(default)]
    label: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct AssertSpec {
    #[serde(default)]
    stop_stage: Option<String>,
    #[serde(default)]
    varps: Vec<VarpAssertion>,
    #[serde(default)]
    varbits: Vec<VarbitAssertion>,
    #[serde(default)]
    quest_points: Option<i32>,
    #[serde(default)]
    xp: Vec<XpAssertion>,
    #[serde(default)]
    inventory: Vec<ItemAssertion>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct VarpAssertion {
    name: String,
    value: i32,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct VarbitAssertion {
    id: i32,
    value: i32,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct XpAssertion {
    skill: String,
    value: i32,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ItemAssertion {
    item: String,
    count: i32,
}

#[derive(Clone, Debug)]
struct Config {
    folder: PathBuf,
    quest: String,
    endpoint: Endpoint,
    evidence_root: PathBuf,
    seeds: SeedSpec,
    assertions: AssertSpec,
    allow_danger_zones: bool,
}

impl Config {
    fn from_env() -> Result<Self, String> {
        let folder = required_absolute_path("QUESTER_FOLDER")?;
        path_is_dir(&folder, "QUESTER_FOLDER")?;
        let quest = std::env::var("QUESTER_QUEST")
            .map_err(|_| "QUESTER_QUEST must name a Path id".to_owned())?;
        if !valid_identifier(&quest) {
            return Err("QUESTER_QUEST must be a lowercase Path id".into());
        }
        let endpoint = parse_endpoint(
            std::env::var("ENGINE_SLOT").ok().as_deref(),
            std::env::var("QUESTER_GAME_PORT").ok().as_deref(),
            std::env::var("QUESTER_ASSET_PORT").ok().as_deref(),
        )?;
        let allow_danger_zones = danger_routing_from_env()?;
        let evidence_root = required_absolute_path("QUESTER_377_EVIDENCE_DIR")?;
        path_is_dir(&evidence_root, "QUESTER_377_EVIDENCE_DIR")?;
        let seeds = read_spec_env::<SeedSpec>("QUESTER_SEEDS")?.unwrap_or_default();
        validate_seeds(&seeds)?;
        let assertions = read_spec_env::<AssertSpec>("QUESTER_ASSERT")?
            .ok_or("QUESTER_ASSERT must name a JSON readback specification")?;
        validate_assertions(&assertions)?;
        Ok(Self {
            folder,
            quest,
            endpoint,
            evidence_root,
            seeds,
            assertions,
            allow_danger_zones,
        })
    }
}

fn danger_routing_from_env() -> Result<bool, String> {
    match std::env::var("QUESTER_DANGER_ROUTING") {
        Ok(value) => parse_danger_routing(Some(&value)),
        Err(std::env::VarError::NotPresent) => parse_danger_routing(None),
        Err(std::env::VarError::NotUnicode(_)) => {
            Err("QUESTER_DANGER_ROUTING must be UTF-8".into())
        }
    }
}

fn parse_danger_routing(value: Option<&str>) -> Result<bool, String> {
    match value {
        None | Some("always") => Ok(true),
        Some("off") => Ok(false),
        Some(other) => Err(format!(
            "QUESTER_DANGER_ROUTING must be `always` or `off`, got {other:?}"
        )),
    }
}

fn required_absolute_path(name: &str) -> Result<PathBuf, String> {
    let value = std::env::var_os(name).ok_or_else(|| format!("{name} is required"))?;
    let path = PathBuf::from(value);
    if path.as_os_str().is_empty() || !path.is_absolute() {
        return Err(format!("{name} must be a non-empty absolute path"));
    }
    Ok(path)
}

fn read_spec_env<T: for<'de> Deserialize<'de>>(name: &str) -> Result<Option<T>, String> {
    let Some(path) = std::env::var_os(name).map(PathBuf::from) else {
        return Ok(None);
    };
    if !path.is_absolute() || !path.is_file() {
        return Err(format!("{name} must name an absolute JSON file"));
    }
    let bytes = fs::read(&path).map_err(|error| format!("read {}: {error}", path.display()))?;
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|error| format!("parse {}: {error}", path.display()))
}

fn valid_identifier(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
}

fn valid_skill(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphabetic() || byte == b'_')
}

fn validate_item(item: &str, count: i32) -> Result<(), String> {
    if !valid_identifier(item) || count <= 0 {
        Err(format!("invalid item seed/assertion {item:?} x{count}"))
    } else {
        Ok(())
    }
}

fn validate_seeds(seeds: &SeedSpec) -> Result<(), String> {
    if let Some(position) = seeds.position {
        if !(0..=3).contains(&position.level) || position.x < 0 || position.z < 0 {
            return Err("position seed needs nonnegative x/z and level 0..3".into());
        }
    }
    for item in seeds.inventory.iter().chain(&seeds.bank) {
        validate_item(&item.item, item.count)?;
    }
    for stat in &seeds.stats {
        if !valid_skill(&stat.skill) || !(1..=99).contains(&stat.level) {
            return Err(format!("invalid stat seed {}={}", stat.skill, stat.level));
        }
    }
    for varp in &seeds.quest_varps {
        if !valid_identifier(&varp.name) {
            return Err(format!("invalid quest varp name {:?}", varp.name));
        }
    }
    Ok(())
}

fn validate_assertions(assertions: &AssertSpec) -> Result<(), String> {
    if assertions
        .stop_stage
        .as_deref()
        .is_some_and(|stage| stage.is_empty() || stage.chars().any(char::is_control))
    {
        return Err("stop_stage must be a non-empty stage id".into());
    }
    for varp in &assertions.varps {
        if !valid_identifier(&varp.name) {
            return Err(format!("invalid assertion varp name {:?}", varp.name));
        }
    }
    if assertions.varbits.iter().any(|row| row.id < 0) {
        return Err("varbit ids must be nonnegative".into());
    }
    for stat in &assertions.xp {
        if !valid_skill(&stat.skill) || stat.value < 0 {
            return Err(format!(
                "invalid XP assertion {}={}",
                stat.skill, stat.value
            ));
        }
    }
    for item in &assertions.inventory {
        if item.count < 0 {
            return Err(format!(
                "inventory assertion {} count cannot be negative",
                item.item
            ));
        }
        if !valid_identifier(&item.item) {
            return Err(format!("invalid inventory assertion item {:?}", item.item));
        }
    }
    Ok(())
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
struct SeedReadback {
    position: Option<(i32, i32, i32)>,
    inventory: BTreeMap<String, i32>,
    bank: Option<BTreeMap<String, i32>>,
    stat_bases: BTreeMap<String, i32>,
    quest_varps: BTreeMap<String, i32>,
}

fn seed_readback_gate(spec: &SeedSpec, actual: &SeedReadback) -> Result<(), Vec<String>> {
    let mut misses = Vec::new();
    if let Some(position) = spec.position {
        let expected = (position.x, position.z, position.level);
        if actual.position != Some(expected) {
            misses.push(format!(
                "position expected {expected:?}, observed {:?}",
                actual.position
            ));
        }
    }
    for item in &spec.inventory {
        let observed = actual
            .inventory
            .get(&item.item)
            .copied()
            .unwrap_or_default();
        if observed < item.count {
            misses.push(format!(
                "inventory {} expected at least {}, observed {observed}",
                item.item, item.count
            ));
        }
    }
    if !spec.bank.is_empty() {
        match &actual.bank {
            Some(bank) => {
                for item in &spec.bank {
                    let observed = bank.get(&item.item).copied().unwrap_or_default();
                    if observed < item.count {
                        misses.push(format!(
                            "bank {} expected at least {}, observed {observed}",
                            item.item, item.count
                        ));
                    }
                }
            }
            None => misses.push("bank contents were not server-posted".into()),
        }
    }
    for stat in &spec.stats {
        let observed = actual
            .stat_bases
            .get(&stat.skill.to_ascii_lowercase())
            .copied();
        if observed != Some(stat.level) {
            misses.push(format!(
                "stat {} expected base {}, observed {observed:?}",
                stat.skill, stat.level
            ));
        }
    }
    for varp in &spec.quest_varps {
        let observed = actual.quest_varps.get(&varp.name).copied();
        if observed != Some(varp.value) {
            misses.push(format!(
                "quest varp {} expected {}, observed {observed:?}",
                varp.name, varp.value
            ));
        }
    }
    if misses.is_empty() {
        Ok(())
    } else {
        Err(misses)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
struct StatReadback {
    base: i32,
    xp: i32,
}

#[derive(Clone, Debug, Serialize)]
struct Sample {
    tick: u32,
    tile: Option<(i32, i32, i32)>,
    inventory: BTreeMap<String, i32>,
    inventory_posted: bool,
    bank: Option<BTreeMap<String, i32>>,
    stats: BTreeMap<String, StatReadback>,
    varps: Vec<(i32, i32)>,
    chat: Vec<(i32, String)>,
}

fn item_counts(
    selected: &api::game_data::SelectedGameData,
    rows: impl IntoIterator<Item = (i32, i32)>,
) -> BTreeMap<String, i32> {
    let mut counts = BTreeMap::new();
    for (id, count) in rows {
        let name = selected
            .items()
            .iter()
            .find(|item| item.id == id)
            .and_then(|item| item.alias.as_ref())
            .cloned()
            .unwrap_or_else(|| format!("id_{id}"));
        *counts.entry(name).or_default() += count;
    }
    counts
}

impl Sample {
    fn read(selected: &api::game_data::SelectedGameData, snapshot: &GameSnapshot) -> Self {
        Self {
            tick: snapshot.tick(),
            tile: snapshot.tile(),
            inventory: item_counts(selected, snapshot.inv().iter().copied()),
            inventory_posted: snapshot.inventory_packet_generation().is_some(),
            bank: snapshot.bank_loaded().then(|| {
                item_counts(
                    selected,
                    snapshot.bank().iter().map(|item| (item.def.id, item.count)),
                )
            }),
            stats: snapshot
                .stats()
                .iter()
                .filter(|stat| stat.used)
                .map(|stat| {
                    (
                        stat.name.to_ascii_lowercase(),
                        StatReadback {
                            base: stat.base,
                            xp: stat.xp,
                        },
                    )
                })
                .collect(),
            varps: snapshot
                .varps()
                .iter()
                .map(|varp| (varp.index, varp.value))
                .collect(),
            chat: snapshot
                .chat_lines()
                .iter()
                .map(|line| (line.sequence, line.text.clone()))
                .collect(),
        }
    }

    fn seed_readback(&self, server_varps: &BTreeMap<String, i32>) -> SeedReadback {
        SeedReadback {
            position: self.tile,
            inventory: self.inventory.clone(),
            bank: self.bank.clone(),
            stat_bases: self
                .stats
                .iter()
                .map(|(name, stat)| (name.clone(), stat.base))
                .collect(),
            quest_varps: server_varps.clone(),
        }
    }
}

#[derive(Clone, Debug)]
struct SeedCommand {
    command: String,
    label: String,
}

fn seed_commands(spec: &SeedSpec) -> Vec<SeedCommand> {
    let mut commands = Vec::with_capacity(
        spec.inventory.len() + spec.bank.len() + spec.stats.len() + spec.quest_varps.len() + 1,
    );
    commands.extend(spec.inventory.iter().map(|seed| SeedCommand {
        command: format!("give {} {}", seed.item, seed.count),
        label:
            seed.label.clone().unwrap_or_else(|| {
                format!("SEED own-account inventory {} x{}", seed.item, seed.count)
            }),
    }));
    commands.extend(spec.bank.iter().map(|seed| {
        SeedCommand {
            command: format!("givebank {} {}", seed.item, seed.count),
            label: seed
                .label
                .clone()
                .unwrap_or_else(|| format!("SEED own-account bank {} x{}", seed.item, seed.count)),
        }
    }));
    commands.extend(spec.stats.iter().map(|seed| SeedCommand {
        command: format!("setstat {} {}", seed.skill, seed.level),
        label:
            seed.label.clone().unwrap_or_else(|| {
                format!("SEED own-account stat {} base {}", seed.skill, seed.level)
            }),
    }));
    commands.extend(spec.quest_varps.iter().map(|seed| SeedCommand {
        command: format!("setvar {} {}", seed.name, seed.value),
        label:
            seed.label.clone().unwrap_or_else(|| {
                format!("SEED own-account quest varp {}={}", seed.name, seed.value)
            }),
    }));
    if let Some(position) = spec.position {
        commands.push(SeedCommand {
            command: interact::tele_args(position.level, position.x, position.z),
            label: format!(
                "SEED own-account position ({},{},{})",
                position.x, position.z, position.level
            ),
        });
    }
    commands
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BankPhase {
    None,
    MoveToBank,
    WaitAtBank,
    Open,
    WaitOpen,
    Close,
    Restore,
    WaitRestore,
    Done,
}

#[derive(Clone, Debug)]
struct PendingRead {
    after_sequence: i32,
    name: String,
    expected: i32,
    varbit_id: Option<i32>,
    requested_tick: u32,
}

#[derive(Clone, Debug)]
struct AssertionRead {
    name: String,
    expected: i32,
    varbit_id: Option<i32>,
}

struct Witness {
    snapshot: GameSnapshot,
    pump: Pump,
    log: fs::File,
    account: String,
    selected: Arc<api::game_data::SelectedGameData>,
    seeds: SeedSpec,
    assertions: AssertSpec,
    commands: Vec<SeedCommand>,
    seed_cursor: usize,
    seed_tick: Option<u32>,
    relog_requested: bool,
    relog_detached: bool,
    relogged: bool,
    original_tile: Option<WorldTile>,
    restore_tile: Option<WorldTile>,
    bank_phase: BankPhase,
    bank_readback: Option<BTreeMap<String, i32>>,
    seed_read_cursor: usize,
    seed_pending: Option<PendingRead>,
    seed_varps: BTreeMap<String, i32>,
    seed_gate_passed: bool,
    ready_to_start: bool,
    start_started: bool,
    asserting: bool,
    assert_read_cursor: usize,
    assert_pending: Option<PendingRead>,
    assertion_varps: BTreeMap<String, i32>,
    assertion_varbits: BTreeMap<i32, i32>,
    assertion_finished: bool,
    assertion_error: Option<String>,
    assertion_readback: Option<Value>,
    latest: Option<Sample>,
    last_tick: Option<u32>,
    error: Option<String>,
}

impl Witness {
    fn new(
        account: String,
        selected: Arc<api::game_data::SelectedGameData>,
        seeds: SeedSpec,
        assertions: AssertSpec,
        log: fs::File,
    ) -> Self {
        let commands = seed_commands(&seeds);
        let bank_phase = if seeds.bank.is_empty() {
            BankPhase::None
        } else {
            BankPhase::MoveToBank
        };
        Self {
            snapshot: GameSnapshot::new(),
            pump: Pump::new(),
            log,
            account,
            selected,
            seeds,
            assertions,
            commands,
            seed_cursor: 0,
            seed_tick: None,
            relog_requested: false,
            relog_detached: false,
            relogged: false,
            original_tile: None,
            restore_tile: None,
            bank_phase,
            bank_readback: None,
            seed_read_cursor: 0,
            seed_pending: None,
            seed_varps: BTreeMap::new(),
            seed_gate_passed: false,
            ready_to_start: false,
            start_started: false,
            asserting: false,
            assert_read_cursor: 0,
            assert_pending: None,
            assertion_varps: BTreeMap::new(),
            assertion_varbits: BTreeMap::new(),
            assertion_finished: false,
            assertion_error: None,
            assertion_readback: None,
            latest: None,
            last_tick: None,
            error: None,
        }
    }

    fn record(&mut self, record: Value) -> Result<(), String> {
        writeln!(self.log, "{record}").map_err(|error| error.to_string())?;
        self.log.flush().map_err(|error| error.to_string())
    }

    fn frame(&mut self, client: &mut Client, hold: bool) -> Result<(), String> {
        let drain = self.pump.drain_client(client);
        host::publish_snapshot(&mut self.snapshot, client, drain);
        if client.error_loading {
            return Err("client reported a loading error".into());
        }
        if self.relog_requested && !self.snapshot.ingame() {
            self.relog_detached = true;
        }
        if hold
            || !self.snapshot.ingame()
            || !self.snapshot.attached()
            || self.snapshot.scene_state() != 2
        {
            return Ok(());
        }
        let sample = Sample::read(&self.selected, &self.snapshot);
        self.latest = Some(sample.clone());
        if self.last_tick != Some(sample.tick) {
            self.record(json!({"phase":"observation","account":self.account,"sample":sample}))?;
            self.last_tick = Some(sample.tick);
        }
        if !self.relogged {
            if self.relog_detached
                && self
                    .snapshot
                    .side_tabs()
                    .iter()
                    .any(|tab| tab.index == 3 && tab.available)
            {
                self.relogged = true;
                self.original_tile =
                    self.snapshot
                        .tile()
                        .map(|(x, z, level)| WorldTile { x, z, level });
                self.record(json!({"phase":"fixture-relogged","sample":sample}))?;
            } else if !self.relog_requested
                && sample.tick >= 3
                && sample
                    .tile
                    .is_some_and(|(x, z, level)| lumbridge(WorldTile { x, z, level }))
            {
                let ifaces = Arc::clone(&client.ifaces);
                if !interact::logout(client, &ifaces) {
                    return Err("fresh account could not cleanly logout for fixture relog".into());
                }
                self.relog_requested = true;
                self.record(json!({"phase":"SEED own-account tutorial/mainland preparation; clean relog","sample":sample}))?;
            }
            return Ok(());
        }

        if !self.start_started {
            self.seed_until_ready(client, &sample)?;
            return Ok(());
        }
        if self.asserting && !self.assertion_finished {
            self.drive_assertions(client, &sample)?;
        }
        Ok(())
    }

    fn seed_until_ready(&mut self, client: &mut Client, sample: &Sample) -> Result<(), String> {
        if self.seed_cursor < self.commands.len() {
            if self.seed_tick != Some(sample.tick) {
                let command = self.commands[self.seed_cursor].clone();
                if !interact::cheat(client, &command.command).is_sent() {
                    self.seed_tick = Some(sample.tick);
                    self.record(json!({
                        "phase":"SEED action not sent; retrying next server tick",
                        "command":command.command,
                        "tick":sample.tick
                    }))?;
                    return Ok(());
                }
                self.record(json!({
                    "phase":command.label,
                    "command":command.command,
                    "scope":"own-account",
                    "accepted_by_client":true
                }))?;
                self.seed_cursor += 1;
                self.seed_tick = Some(sample.tick);
            }
            return Ok(());
        }
        if !self.bank_done(client, sample)? {
            return Ok(());
        }
        if !self.drive_seed_varps(client, sample)? {
            return Ok(());
        }
        let mut observed = sample.seed_readback(&self.seed_varps);
        if !self.seeds.bank.is_empty() {
            observed.bank = self.bank_readback.clone();
        }
        if let Err(misses) = seed_readback_gate(&self.seeds, &observed) {
            return Err(format!(
                "fixture preparation: server seed readback mismatch: {}",
                misses.join("; ")
            ));
        }
        if !self.seed_gate_passed {
            self.record(json!({
                "phase":"SEED SERVER READBACK gate passed before Start",
                "readback":observed,
                "seed_spec":self.seeds
            }))?;
            self.seed_gate_passed = true;
            self.ready_to_start = true;
        }
        Ok(())
    }

    fn bank_done(&mut self, client: &mut Client, sample: &Sample) -> Result<bool, String> {
        match self.bank_phase {
            BankPhase::None | BankPhase::Done => Ok(true),
            BankPhase::MoveToBank => {
                self.restore_tile = self.seeds.position.map_or(self.original_tile, |position| {
                    Some(WorldTile {
                        x: position.x,
                        z: position.z,
                        level: position.level,
                    })
                });
                if self.restore_tile.is_none() {
                    self.restore_tile =
                        self.snapshot
                            .tile()
                            .map(|(x, z, level)| WorldTile { x, z, level });
                }
                if !interact::cheat(
                    client,
                    &interact::tele_args(
                        DEFAULT_BANK_TILE.level,
                        DEFAULT_BANK_TILE.x,
                        DEFAULT_BANK_TILE.z,
                    ),
                )
                .is_sent()
                {
                    return Err("bank seed readback teleport was refused".into());
                }
                self.record(json!({"phase":"SEED bank readback access","tile":DEFAULT_BANK_TILE,"command":interact::tele_args(DEFAULT_BANK_TILE.level,DEFAULT_BANK_TILE.x,DEFAULT_BANK_TILE.z)}))?;
                self.bank_phase = BankPhase::WaitAtBank;
                Ok(false)
            }
            BankPhase::WaitAtBank => {
                if sample.tile
                    == Some((
                        DEFAULT_BANK_TILE.x,
                        DEFAULT_BANK_TILE.z,
                        DEFAULT_BANK_TILE.level,
                    ))
                    || sample.tile.is_some_and(|(x, z, level)| {
                        level == DEFAULT_BANK_TILE.level
                            && x.div_euclid(64) == DEFAULT_BANK_TILE.x.div_euclid(64)
                            && z.div_euclid(64) == DEFAULT_BANK_TILE.z.div_euclid(64)
                    })
                {
                    self.bank_phase = BankPhase::Open;
                }
                Ok(false)
            }
            BankPhase::Open => {
                let result =
                    interact::Interactions::new(&self.snapshot, client).open_nearest_booth();
                if !matches!(result, interact::SendResult::Sent { .. }) {
                    return Err(format!(
                        "server bank readback booth open was refused: {result:?}"
                    ));
                }
                self.record(json!({"phase":"SERVER READBACK bank open","result":"sent"}))?;
                self.bank_phase = BankPhase::WaitOpen;
                Ok(false)
            }
            BankPhase::WaitOpen => {
                if self.snapshot.bank_loaded() {
                    self.bank_readback = Some(item_counts(
                        &self.selected,
                        self.snapshot
                            .bank()
                            .iter()
                            .map(|item| (item.def.id, item.count)),
                    ));
                    self.record(
                        json!({"phase":"SERVER READBACK bank observed","bank":self.bank_readback}),
                    )?;
                    self.bank_phase = BankPhase::Close;
                }
                Ok(false)
            }
            BankPhase::Close => {
                if !interact::close_modal(client) {
                    return Err("server bank readback modal close was refused".into());
                }
                self.bank_phase = BankPhase::Restore;
                Ok(false)
            }
            BankPhase::Restore => {
                let Some(tile) = self.restore_tile else {
                    self.bank_phase = BankPhase::Done;
                    return Ok(true);
                };
                if sample.tile == Some((tile.x, tile.z, tile.level)) {
                    self.bank_phase = BankPhase::Done;
                    return Ok(true);
                }
                if !interact::cheat(client, &interact::tele_args(tile.level, tile.x, tile.z))
                    .is_sent()
                {
                    return Err("restore position after bank readback was refused".into());
                }
                self.record(
                    json!({"phase":"SEED restore requested own-account position","tile":tile}),
                )?;
                self.bank_phase = BankPhase::WaitRestore;
                Ok(false)
            }
            BankPhase::WaitRestore => {
                if let Some(tile) = self.restore_tile {
                    if sample.tile == Some((tile.x, tile.z, tile.level)) {
                        self.bank_phase = BankPhase::Done;
                        return Ok(true);
                    }
                }
                Ok(false)
            }
        }
    }

    fn drive_seed_varps(&mut self, client: &mut Client, sample: &Sample) -> Result<bool, String> {
        if let Some(mut pending) = self.seed_pending.take() {
            if let Some((sequence, value)) = find_getvar_reply(&sample.chat, &pending) {
                self.seed_varps.insert(pending.name.clone(), value);
                self.record(json!({
                    "phase":"SERVER READBACK seed quest varp observed",
                    "name":pending.name,
                    "value":value,
                    "expected":pending.expected,
                    "matched":value == pending.expected,
                    "chat_sequence":sequence
                }))?;
                self.seed_read_cursor += 1;
            } else {
                if sample.tick.saturating_sub(pending.requested_tick) >= READBACK_RETRY_TICKS {
                    let before = sample
                        .chat
                        .iter()
                        .map(|(sequence, _)| *sequence)
                        .max()
                        .unwrap_or(-1);
                    let command = format!("getvar {}", pending.name);
                    let accepted = interact::cheat(client, &command).is_sent();
                    if accepted {
                        pending.after_sequence = before;
                    }
                    pending.requested_tick = sample.tick;
                    self.record(json!({
                        "phase":"SERVER READBACK seed retry",
                        "command":command,
                        "accepted_by_client":accepted,
                        "after_sequence":before
                    }))?;
                }
                self.seed_pending = Some(pending);
                return Ok(false);
            }
        }
        let Some(seed) = self.seeds.quest_varps.get(self.seed_read_cursor) else {
            return Ok(true);
        };
        let before = sample
            .chat
            .iter()
            .map(|(sequence, _)| *sequence)
            .max()
            .unwrap_or(-1);
        let command = format!("getvar {}", seed.name);
        let accepted = interact::cheat(client, &command).is_sent();
        self.seed_pending = Some(PendingRead {
            after_sequence: before,
            name: seed.name.clone(),
            expected: seed.value,
            varbit_id: None,
            requested_tick: sample.tick,
        });
        self.record(json!({
            "phase":"SERVER READBACK seed request, own-account read-only",
            "command":command,
            "accepted_by_client":accepted,
            "after_sequence":before
        }))?;
        Ok(false)
    }

    fn assertion_reads(&self, client: &Client) -> Result<Vec<AssertionRead>, String> {
        let mut values = BTreeMap::new();
        for row in &self.assertions.varps {
            values.insert(row.name.clone(), row.value);
        }
        if let Some(value) = self.assertions.quest_points {
            values.insert("qp".into(), value);
        }
        let mut reads: Vec<_> = values
            .into_iter()
            .map(|(name, expected)| AssertionRead {
                name,
                expected,
                varbit_id: None,
            })
            .collect();
        let mut names: BTreeMap<String, Option<i32>> =
            reads.iter().map(|read| (read.name.clone(), None)).collect();
        for row in &self.assertions.varbits {
            let index = usize::try_from(row.id).map_err(|_| {
                format!(
                    "oracle server readback: varbit {} has an invalid cache id",
                    row.id
                )
            })?;
            let bit = client.cache.varbits.get(index).ok_or_else(|| {
                format!(
                    "oracle server readback: varbit {} is absent from the 377 cache",
                    row.id
                )
            })?;
            if bit.debugname.is_empty() || bit.debugname.chars().any(char::is_whitespace) {
                return Err(format!(
                    "oracle server readback: varbit {} has no single-token cache debug name",
                    row.id
                ));
            }
            if let Some(existing) = names.get(&bit.debugname) {
                if *existing != Some(row.id) {
                    return Err(format!(
                        "oracle server readback: getvar name {:?} is ambiguous",
                        bit.debugname
                    ));
                }
                continue;
            }
            names.insert(bit.debugname.clone(), Some(row.id));
            reads.push(AssertionRead {
                name: bit.debugname.clone(),
                expected: row.value,
                varbit_id: Some(row.id),
            });
        }
        Ok(reads)
    }

    fn drive_assertions(&mut self, client: &mut Client, sample: &Sample) -> Result<(), String> {
        let reads = self.assertion_reads(client)?;
        if let Some(mut pending) = self.assert_pending.take() {
            if let Some((sequence, value)) = find_getvar_reply(&sample.chat, &pending) {
                if let Some(id) = pending.varbit_id {
                    self.assertion_varbits.insert(id, value);
                } else {
                    self.assertion_varps.insert(pending.name.clone(), value);
                }
                self.record(json!({
                    "phase":"SERVER READBACK assertion observed",
                    "name":pending.name,
                    "varbit_id":pending.varbit_id,
                    "value":value,
                    "expected":pending.expected,
                    "matched":value == pending.expected,
                    "chat_sequence":sequence
                }))?;
                self.assert_read_cursor += 1;
            } else {
                if sample.tick.saturating_sub(pending.requested_tick) >= READBACK_RETRY_TICKS {
                    let before = sample
                        .chat
                        .iter()
                        .map(|(sequence, _)| *sequence)
                        .max()
                        .unwrap_or(-1);
                    let command = format!("getvar {}", pending.name);
                    let accepted = interact::cheat(client, &command).is_sent();
                    if accepted {
                        pending.after_sequence = before;
                    }
                    pending.requested_tick = sample.tick;
                    self.record(json!({
                        "phase":"SERVER READBACK assertion retry",
                        "command":command,
                        "varbit_id":pending.varbit_id,
                        "accepted_by_client":accepted,
                        "after_sequence":before
                    }))?;
                }
                self.assert_pending = Some(pending);
                return Ok(());
            }
        }
        if let Some(read) = reads.get(self.assert_read_cursor) {
            let before = sample
                .chat
                .iter()
                .map(|(sequence, _)| *sequence)
                .max()
                .unwrap_or(-1);
            let command = format!("getvar {}", read.name);
            let accepted = interact::cheat(client, &command).is_sent();
            self.assert_pending = Some(PendingRead {
                after_sequence: before,
                name: read.name.clone(),
                expected: read.expected,
                varbit_id: read.varbit_id,
                requested_tick: sample.tick,
            });
            self.record(json!({
                "phase":"SERVER READBACK request, own-account read-only",
                "command":command,
                "varbit_id":read.varbit_id,
                "expected":read.expected,
                "accepted_by_client":accepted,
                "after_sequence":before
            }))?;
            return Ok(());
        }
        let observed = assertion_gate(
            &self.assertions,
            sample,
            &self.assertion_varps,
            &self.assertion_varbits,
        );
        self.assertion_readback = Some(observed.0.clone());
        match observed.1 {
            Ok(()) => {
                self.record(
                    json!({"phase":"SERVER READBACK assertion gate passed","readback":observed.0}),
                )?;
                self.assertion_finished = true;
            }
            Err(misses) => {
                let message = format!("assertion mismatch: {}", misses.join("; "));
                self.record(json!({"phase":"SERVER READBACK assertion gate failed","readback":observed.0,"mismatches":misses}))?;
                self.assertion_error = Some(message);
                self.assertion_finished = true;
            }
        }
        Ok(())
    }
}

fn find_getvar_reply(chat: &[(i32, String)], pending: &PendingRead) -> Option<(i32, i32)> {
    let prefix = format!("get {}: ", pending.name);
    chat.iter().find_map(|(sequence, text)| {
        if *sequence <= pending.after_sequence {
            return None;
        }
        text.strip_prefix(&prefix)
            .and_then(|value| value.parse::<i32>().ok())
            .map(|value| (*sequence, value))
    })
}

fn assertion_gate(
    spec: &AssertSpec,
    sample: &Sample,
    server_varps: &BTreeMap<String, i32>,
    server_varbits: &BTreeMap<i32, i32>,
) -> (Value, Result<(), Vec<String>>) {
    let mut misses = Vec::new();
    let varps: BTreeMap<_, _> = spec
        .varps
        .iter()
        .map(|row| (row.name.clone(), server_varps.get(&row.name).copied()))
        .collect();
    for row in &spec.varps {
        let observed = server_varps.get(&row.name).copied();
        if observed != Some(row.value) {
            misses.push(format!(
                "varp {} expected {}, observed {observed:?}",
                row.name, row.value
            ));
        }
    }
    let quest_points = spec.quest_points.map(|_| server_varps.get("qp").copied());
    if let Some(expected) = spec.quest_points {
        if quest_points.flatten() != Some(expected) {
            misses.push(format!(
                "quest points expected {expected}, observed {quest_points:?}"
            ));
        }
    }
    let varbits: BTreeMap<_, _> = spec
        .varbits
        .iter()
        .map(|row| (row.id, server_varbits.get(&row.id).copied()))
        .collect();
    for row in &spec.varbits {
        let observed = server_varbits.get(&row.id).copied();
        if observed != Some(row.value) {
            misses.push(format!(
                "varbit {} expected {}, observed {observed:?}",
                row.id, row.value
            ));
        }
    }
    let xp: BTreeMap<_, _> = spec
        .xp
        .iter()
        .map(|row| {
            (
                row.skill.to_ascii_lowercase(),
                sample
                    .stats
                    .get(&row.skill.to_ascii_lowercase())
                    .map(|stat| stat.xp),
            )
        })
        .collect();
    for row in &spec.xp {
        let observed = sample
            .stats
            .get(&row.skill.to_ascii_lowercase())
            .map(|stat| stat.xp);
        if observed != Some(row.value) {
            misses.push(format!(
                "XP {} expected {}, observed {observed:?}",
                row.skill, row.value
            ));
        }
    }
    let inventory: BTreeMap<_, _> = spec
        .inventory
        .iter()
        .map(|row| {
            (
                row.item.clone(),
                sample.inventory.get(&row.item).copied().unwrap_or_default(),
            )
        })
        .collect();
    if !spec.inventory.is_empty() && !sample.inventory_posted {
        misses.push("server inventory component was not posted".into());
    }
    for row in &spec.inventory {
        let observed = sample.inventory.get(&row.item).copied().unwrap_or_default();
        if observed != row.count {
            misses.push(format!(
                "inventory {} expected {}, observed {observed}",
                row.item, row.count
            ));
        }
    }
    let readback = json!({
        "tile":sample.tile,
        "varps":varps,
        "varbits":varbits,
        "quest_points":quest_points,
        "xp":xp,
        "inventory":inventory,
        "server_varps":server_varps,
        "server_varbits":server_varbits,
        "snapshot_varps":sample.varps,
        "snapshot_source":"server getvar readbacks for varps and varbits; XP and inventory from server-fed 377 client updates"
    });
    if misses.is_empty() {
        (readback, Ok(()))
    } else {
        (readback, Err(misses))
    }
}

fn field_text<'a>(status: &'a ScriptStatus, key: &str) -> Option<&'a str> {
    status
        .fields
        .iter()
        .find(|field| field.key == key)
        .and_then(|field| {
            if let StatusValue::Text(text) = &field.value {
                Some(text.as_ref())
            } else {
                None
            }
        })
}

fn status_record(status: &ScriptStatus) -> Value {
    json!({
        "run":status.run.run,
        "card":status.card.0,
        "phase":format!("{:?}",status.phase),
        "failure":status.failure.as_ref().map(|failure| format!("{failure:?}")),
        "fields":status.fields.iter().map(|field| json!({
            "key":field.key,
            "label":field.label,
            "value":format!("{:?}",field.value)
        })).collect::<Vec<_>>()
    })
}

fn loader_record(quest: &str) -> Value {
    let loaded = registry::snapshot();
    json!({
        "path_source":loaded.path_source(quest).label(),
        "report":loaded.report().map(|report| report.to_string()),
        "diagnostics":loaded.diagnostics().iter().map(|diagnostic| json!({
            "file":diagnostic.file,
            "error":format!("{:?}",diagnostic.error)
        })).collect::<Vec<_>>()
    })
}

fn native_run_completed(receipt: &ScriptLifecycleReceipt, generation: Option<u64>) -> bool {
    Some(receipt.runtime_generation) == generation
        && receipt.state == ScriptTerminalState::Completed
}

fn lifecycle_record(receipt: &ScriptLifecycleReceipt) -> Value {
    json!({
        "runtime_generation":receipt.runtime_generation,
        "state":format!("{:?}",receipt.state),
        "tick":receipt.tick,
        "reason":receipt.reason
    })
}

fn classify_failure(message: &str, status: Option<&str>) -> &'static str {
    let combined = format!("{message} {}", status.unwrap_or_default()).to_ascii_lowercase();
    if combined.contains("nowayout") || combined.contains("no way out") {
        "quester_refusal"
    } else if combined.contains("seed readback")
        || combined.contains("seed command")
        || combined.contains("fixture preparation")
    {
        "fixture"
    } else if combined.contains("folder loader")
        || combined.contains("path_source")
        || combined.contains("oracle")
    {
        "oracle"
    } else if combined.contains("assertion mismatch") {
        "product"
    } else if combined.contains("mac-quiet")
        || combined.contains("timeout")
        || combined.contains("connection")
        || combined.contains("loading error")
    {
        "interference"
    } else if combined.contains("phase: blocked") || combined.contains("failure: some") {
        "product"
    } else {
        "interference"
    }
}

struct SourceGuard(FolderSource);
impl SourceGuard {
    fn enable(folder: &Path) -> Self {
        let old = registry::source();
        registry::set_source(FolderSource {
            enabled: true,
            folder: folder.to_path_buf(),
        });
        Self(old)
    }
}
impl Drop for SourceGuard {
    fn drop(&mut self) {
        registry::set_source(std::mem::take(&mut self.0));
    }
}

fn set_scratch_preferences(
    home: &Path,
    folder: &Path,
    allow_danger_zones: bool,
) -> Result<
    (
        Arc<LocalStateService>,
        host_play::local_state::Preferences,
        WalkGlobals,
    ),
    String,
> {
    let service = LocalStateService::open_process(StateRoot::new(home.join(".274bot")));
    let preferences = service.preferences();
    let mut patch = PreferencePatch::default();
    patch
        .set_member(&["quester_paths", "enabled"], Some(json!(true)))
        .map_err(|error| error.to_string())?;
    patch
        .set_member(
            &["quester_paths", "folder"],
            Some(json!(folder.to_string_lossy().to_string())),
        )
        .map_err(|error| error.to_string())?;
    patch
        .set_member(
            &["nav", "allow_danger_zones"],
            Some(json!(allow_danger_zones)),
        )
        .map_err(|error| error.to_string())?;
    patch
        .set_member(&["nav", "survivable_routing"], Some(json!(false)))
        .map_err(|error| error.to_string())?;
    match preferences
        .begin_patch(patch)
        .map_err(|error| error.to_string())?
    {
        Update::Durable => {}
        Update::Queued(mut watch) => watch
            .wait(Duration::from_secs(30))
            .ok_or_else(|| "scratch preference write timed out".to_owned())?
            .map_err(|error| format!("scratch preference write: {error}"))?,
    }
    let globals = read_walk_globals(&preferences)?;
    if globals.allow_danger_zones != allow_danger_zones || globals.survivable_routing {
        return Err(
            "scratch walk preferences did not retain the cell's declared danger policy".into(),
        );
    }
    let bytes = fs::read(home.join(".274bot").join("panel-ui.json"))
        .map_err(|error| format!("read scratch panel-ui preferences: {error}"))?;
    let document: Value = serde_json::from_slice(&bytes).map_err(|error| error.to_string())?;
    if document["quester_paths"]["enabled"] != true
        || document["quester_paths"]["folder"] != folder.to_string_lossy().as_ref()
        || document["nav"]["allow_danger_zones"] != allow_danger_zones
        || document["nav"]["survivable_routing"] != false
    {
        return Err("scratch quester_paths/nav preferences did not read back exactly".into());
    }
    Ok((service, preferences, globals))
}

fn read_walk_globals(
    preferences: &host_play::local_state::Preferences,
) -> Result<WalkGlobals, String> {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        match WalkGlobals::read(preferences) {
            Ok(globals) => return Ok(globals),
            Err(error) if error.is_not_ready() && Instant::now() < deadline => {
                std::thread::yield_now();
            }
            Err(error) => return Err(format!("read scratch walk preferences: {error}")),
        }
    }
}

fn run_live() -> Result<(), String> {
    if !api::test_support::game_data_available() {
        return Err("revision-377 game data is unavailable in this build".into());
    }
    let config = Config::from_env()?;
    let bound = match config.endpoint.slot {
        Some(1) => load_bound_377_from_default_profile()?,
        Some(_) | None => {
            load_bound_377_with_ports(config.endpoint.game_port, config.endpoint.asset_port)?
        }
    };
    resource_record(&bound);
    let selected = bound
        .profile
        .game_data()
        .ok_or("bound 377 profile has no selected game data")?;
    let account = host_play::mint_live_names(1).remove(0);
    let directory = config.evidence_root.join(&account);
    fs::create_dir_all(&directory)
        .map_err(|error| format!("create evidence directory: {error}"))?;
    let log = fs::File::create(directory.join("observations.jsonl"))
        .map_err(|error| format!("create observations.jsonl: {error}"))?;
    let receipt_path = directory.join("receipt.json");
    let _source_guard = SourceGuard::enable(&config.folder);
    let mut receipt = json!({
        "test":"quester_377_folder",
        "revision":377,
        "account":account,
        "quest":config.quest,
        "folder":config.folder,
        "endpoint":config.endpoint,
        "seed_spec":config.seeds,
        "assert_spec":config.assertions,
        "path_source":"pending production Start reload",
        "observations":"observations.jsonl"
    });
    let result = run_play(
        &config,
        Arc::clone(&selected),
        &bound,
        &account,
        log,
        &mut receipt,
    );
    let (outcome, classification, failure) = match &result {
        Ok(()) => ("PASS", "none", Value::Null),
        Err(error) => {
            let status = receipt["native_status"].as_str();
            ("FAIL", classify_failure(error, status), json!(error))
        }
    };
    receipt["outcome"] = json!(outcome);
    receipt["classification"] = json!(classification);
    receipt["failure"] = failure;
    fs::write(
        &receipt_path,
        serde_json::to_vec_pretty(&receipt).map_err(|error| error.to_string())?,
    )
    .map_err(|error| format!("write {}: {error}", receipt_path.display()))?;
    if let Err(error) = result {
        eprintln!("QUESTER-FOLDER receipt {}", receipt_path.display());
        return Err(error);
    }
    println!("QUESTER-FOLDER receipt {}", receipt_path.display());
    Ok(())
}

fn run_play(
    config: &Config,
    selected: Arc<api::game_data::SelectedGameData>,
    bound: &Bound377,
    account: &str,
    log: fs::File,
    receipt: &mut Value,
) -> Result<(), String> {
    let (service, preferences, walk_globals) =
        set_scratch_preferences(&bound._home.0, &config.folder, config.allow_danger_zones)?;
    let policy = if config.allow_danger_zones {
        "Always"
    } else {
        "off"
    };
    let harness_settings = json!([{
        "label":"global danger routing",
        "setting":"nav.allow_danger_zones",
        "policy":policy,
        "readback_view":"host-play walk-permissions view (WalkGlobals::read)",
        "readback":{
            "allow_danger_zones":walk_globals.allow_danger_zones,
            "survivable_routing":walk_globals.survivable_routing
        }
    }]);
    receipt["harness_settings"] = harness_settings.clone();
    let preference_record = json!({
        "quester_paths_enabled":true,
        "quester_paths_folder":config.folder,
        "allow_danger_zones":walk_globals.allow_danger_zones,
        "survivable_routing":walk_globals.survivable_routing,
        "harness_settings":harness_settings,
        "reload":"the production Quester Start path reloads the active folder before use"
    });
    let state = Arc::new(Mutex::new(Witness::new(
        account.to_owned(),
        selected,
        config.seeds.clone(),
        config.assertions.clone(),
        log,
    )));
    {
        let mut witness = state.lock().map_err(|_| "witness poisoned")?;
        witness.record(json!({
            "phase":"resources and runtime configuration",
            "account":account,
            "quest":config.quest,
            "folder":config.folder,
            "endpoint":config.endpoint,
            "preferences":preference_record,
            "path_source_before_Start":registry::snapshot().path_source(&config.quest).label(),
            "assert_spec":config.assertions,
            "seed_spec":config.seeds
        }))?;
    }
    let callback_state = Arc::clone(&state);
    let callback_account = account.to_owned();
    let mut play = host_play::run_with_template(
        Arc::clone(&bound.template),
        true,
        vec![],
        |_| (None, None),
        move |client, username, frame| {
            if username != callback_account {
                return;
            }
            let mut witness = callback_state
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            if witness.error.is_none() {
                if let Err(error) = witness.frame(client, frame.hold) {
                    witness.error = Some(error);
                }
            }
        },
    )?;
    play.set_walk_globals_store(preferences.clone());
    play.try_spawn_slot(
        Profile {
            username: account.to_owned(),
            password: host_play::profile_password(account).into(),
            uid: fresh_uid(700),
            settings: ProfileSettings::default(),
        },
        None,
        None,
        None,
    )?;
    let deadline = Instant::now() + RUN_BUDGET;
    let prep_deadline = Instant::now() + PREP_BUDGET;
    let mut started = false;
    let mut generation = None;
    let mut last_status = String::new();
    let mut native_status = None::<Arc<ScriptStatus>>;
    let mut lifecycle = None::<ScriptLifecycleReceipt>;
    let mut folder_verified = false;
    let mut folder_deadline = None::<Instant>;
    let mut stage_stopped = false;
    let mut relogin_requested = false;
    let result = (|| loop {
        if Path::new("/Volumes/dev-scratch/274bot-evidence/MAC-QUIET.lock").exists() {
            return Err("MAC-QUIET.lock appeared".into());
        }
        if let Some(error) = play.script_last_error(account) {
            let mut witness = state.lock().map_err(|_| "witness poisoned")?;
            witness.record(json!({"phase":"native lifecycle error","error":error}))?;
            return Err(format!("native Quester lifecycle: {error}"));
        }
        let status = play.script_native_status(account);
        let receipt_now = play.script_lifecycle_receipt(account);
        if let Some(status) = status.as_ref() {
            generation.get_or_insert(status.run.run);
            let detail = format!("{status:?}");
            if detail != last_status {
                let mut witness = state.lock().map_err(|_| "witness poisoned")?;
                witness.record(
                    json!({"phase":"native Quester status","status":status_record(status)}),
                )?;
                last_status = detail.clone();
                native_status = Some(status.clone());
                receipt["native_status"] = json!(detail);
            }
            if status.phase == NativePhase::Blocked || status.failure.is_some() {
                let detail = format!("native Quester status blocked: {status:?}");
                receipt["path_loader"] = loader_record(&config.quest);
                return Err(detail);
            }
            if let Some(source) = field_text(status, "path_source") {
                if source != "folder" {
                    receipt["path_loader"] = loader_record(&config.quest);
                    return Err(format!(
                        "folder loader did not load {} from QUESTER_FOLDER (Quester path_source={source})",
                        config.quest
                    ));
                }
                folder_verified = true;
            }
            let source = registry::snapshot().path_source(&config.quest);
            if source == PathSource::Folder {
                folder_verified = true;
            }
            if started && !folder_verified {
                if folder_deadline.is_none() {
                    folder_deadline = Some(Instant::now() + Duration::from_secs(30));
                }
                if folder_deadline.is_some_and(|deadline| Instant::now() >= deadline) {
                    receipt["path_loader"] = loader_record(&config.quest);
                    return Err(format!(
                        "folder loader did not publish {} from QUESTER_FOLDER: {}",
                        config.quest,
                        loader_record(&config.quest)
                    ));
                }
            }
            if let Some(stop_stage) = config.assertions.stop_stage.as_deref() {
                if field_text(status, "stage") == Some(stop_stage) && !stage_stopped {
                    let mut witness = state.lock().map_err(|_| "witness poisoned")?;
                    witness.record(json!({
                        "phase":"named stop stage observed",
                        "stage":stop_stage,
                        "status":status_record(status)
                    }))?;
                    witness.asserting = true;
                    drop(witness);
                    if status.phase != NativePhase::Complete {
                        play.script_stop(account);
                    }
                    stage_stopped = true;
                    receipt["stop_stage"] = json!(stop_stage);
                }
            }
        }
        if !started {
            let mut witness = state.lock().map_err(|_| "witness poisoned")?;
            if let Some(error) = &witness.error {
                receipt["path_loader"] = loader_record(&config.quest);
                return Err(format!("fixture preparation/readback failed: {error}"));
            }
            if witness.relog_detached && !relogin_requested {
                drop(witness);
                if let Some(arm) = play.arm(account) {
                    arm.arm_explicit_login();
                    relogin_requested = true;
                }
                continue;
            }
            if witness.ready_to_start {
                witness.record(json!({"phase":"native Quester start","quest":config.quest,"settings":{"quests":[config.quest]}}))?;
                witness.start_started = true;
                drop(witness);
                if let Err(error) = play.script_start(
                    account,
                    script::CompiledId("Quester"),
                    serde_json::Map::from_iter([("quests".into(), json!([config.quest]))]),
                ) {
                    receipt["path_loader"] = loader_record(&config.quest);
                    return Err(format!(
                        "folder loader/Quester Start: {error}; {}",
                        loader_record(&config.quest)
                    ));
                }
                started = true;
                folder_deadline = Some(Instant::now() + Duration::from_secs(30));
                continue;
            }
            if Instant::now() >= prep_deadline {
                return Err("fixture preparation timed out before the seed-readback gate".into());
            }
        }
        if let Some(receipt_now) = receipt_now.as_ref() {
            if native_run_completed(receipt_now, generation) {
                lifecycle = Some(receipt_now.clone());
                if !stage_stopped {
                    let mut witness = state.lock().map_err(|_| "witness poisoned")?;
                    witness.record(json!({"phase":"native Completed lifecycle receipt","receipt":lifecycle_record(receipt_now)}))?;
                    witness.asserting = true;
                }
                receipt["native_lifecycle"] = lifecycle_record(receipt_now);
            }
        }
        let witness = state.lock().map_err(|_| "witness poisoned")?;
        if let Some(error) = &witness.error {
            return Err(error.clone());
        }
        if (lifecycle.is_some() || stage_stopped) && witness.assertion_finished {
            if let Some(error) = &witness.assertion_error {
                receipt["assertion_readback"] =
                    witness.assertion_readback.clone().unwrap_or(Value::Null);
                return Err(error.clone());
            }
            receipt["assertion_readback"] =
                witness.assertion_readback.clone().unwrap_or(Value::Null);
            receipt["seed_readback"] = json!(witness
                .latest
                .as_ref()
                .map(|sample| sample.seed_readback(&witness.seed_varps)));
            receipt["path_loader"] = loader_record(&config.quest);
            receipt["native_lifecycle"] = lifecycle.as_ref().map(lifecycle_record).unwrap_or_else(
                || json!({"state":"named-stop-stage","stage":config.assertions.stop_stage}),
            );
            drop(witness);
            return Ok(());
        }
        if !folder_verified
            && started
            && folder_deadline.is_some_and(|deadline| Instant::now() >= deadline)
        {
            receipt["path_loader"] = loader_record(&config.quest);
            return Err(format!(
                "folder loader did not publish quest {} from QUESTER_FOLDER: {}",
                config.quest,
                loader_record(&config.quest)
            ));
        }
        if Instant::now() >= deadline {
            receipt["path_loader"] = loader_record(&config.quest);
            return Err(format!(
                "Quester folder cell timed out started={started} folder_verified={folder_verified} latest_status={:?}",
                native_status.as_ref().map(|status| format!("{status:?}"))
            ));
        }
        drop(witness);
        std::thread::sleep(Duration::from_millis(20));
    })();
    play.script_stop(account);
    play.stop_slot(account);
    drop(play);
    drop(service);
    {
        let mut witness = state.lock().map_err(|_| "witness poisoned")?;
        if let Some(error) = result.as_ref().err() {
            let status = native_status.as_ref().map(|status| format!("{status:?}"));
            witness.record(json!({
                "phase":"final",
                "outcome":"FAIL",
                "classification":classify_failure(error, status.as_deref()),
                "error":error,
                "quester_status":status
            }))?;
            receipt["native_status"] = status.map_or(Value::Null, |status| json!(status));
            let classification = classify_failure(error, receipt["native_status"].as_str());
            receipt["quester_status_classification"] = json!(classification);
        } else {
            witness.record(
                json!({"phase":"final","outcome":"PASS","account":account,"quest":config.quest}),
            )?;
        }
        receipt["last_sample"] = witness
            .latest
            .as_ref()
            .map_or(Value::Null, |sample| json!(sample));
        receipt["seed_readback"] = witness
            .latest
            .as_ref()
            .map(|sample| sample.seed_readback(&witness.seed_varps))
            .map_or(Value::Null, |readback| json!(readback));
        receipt["seed_readback_gate_passed"] = json!(witness.seed_gate_passed);
        receipt["assertion_readback"] = witness.assertion_readback.clone().unwrap_or(Value::Null);
        receipt["loader"] = loader_record(&config.quest);
    }
    result
}

fn lumbridge(tile: WorldTile) -> bool {
    tile.level == 0
        && tile.x.div_euclid(64) == LUMBRIDGE_REGION
        && tile.z.div_euclid(64) == LUMBRIDGE_REGION
}

#[test]
#[ignore = "requires LIVE=1, an owned account prefix, a revision-377 local engine and a runtime Path folder"]
fn quester_377_folder() -> Result<(), String> {
    run_live()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoint_env_parsing_supports_slots_and_explicit_port_pairs() {
        assert_eq!(
            parse_endpoint(None, None, None).unwrap(),
            Endpoint {
                slot: Some(1),
                game_port: GAME_PORT,
                asset_port: ASSET_PORT
            }
        );
        assert_eq!(
            parse_endpoint(Some("3"), None, None).unwrap(),
            Endpoint {
                slot: Some(3),
                game_port: GAME_PORT + 2,
                asset_port: ASSET_PORT + 2
            }
        );
        assert_eq!(
            parse_endpoint(None, Some("45555"), Some("1180")).unwrap(),
            Endpoint {
                slot: None,
                game_port: 45555,
                asset_port: 1180
            }
        );
        assert!(parse_endpoint(Some("5"), None, None).is_err());
        assert!(parse_endpoint(None, Some("45555"), None).is_err());
        assert!(parse_endpoint(Some("1"), Some("45555"), Some("1180")).is_err());
    }

    #[test]
    fn danger_routing_defaults_to_always_and_can_be_disabled() {
        assert!(parse_danger_routing(None).unwrap());
        assert!(parse_danger_routing(Some("always")).unwrap());
        assert!(!parse_danger_routing(Some("off")).unwrap());
        assert!(parse_danger_routing(Some("false")).is_err());
    }

    #[test]
    fn parses_and_validates_env_seed_and_assert_specs() {
        let seeds: SeedSpec = serde_json::from_str(
            r#"{
                "position":{"x":3209,"z":3215,"level":0},
                "inventory":[{"item":"pot_flour","count":1}],
                "bank":[{"item":"egg","count":2,"label":"SEED own-account bank eggs"}],
                "stats":[{"skill":"cooking","level":4}],
                "quest_varps":[{"name":"cookquest","value":0}]
            }"#,
        )
        .unwrap();
        validate_seeds(&seeds).unwrap();
        assert_eq!(seed_commands(&seeds).len(), 5);
        let assertions: AssertSpec = serde_json::from_str(
            r#"{
                "varps":[{"name":"cookquest","value":2}],
                "varbits":[{"id":77,"value":1}],
                "quest_points":1,
                "xp":[{"skill":"cooking","value":300}],
                "inventory":[{"item":"pot_flour","count":0}]
            }"#,
        )
        .unwrap();
        validate_assertions(&assertions).unwrap();
        assert!(serde_json::from_str::<SeedSpec>(r#"{"arbitrary_command":"tele 0"}"#).is_err());
        assert!(validate_seeds(
            &serde_json::from_str(r#"{"inventory":[{"item":"egg","count":0}]}"#).unwrap()
        )
        .is_err());
    }

    #[test]
    fn seed_readback_gate_requires_every_requested_server_observation() {
        let spec: SeedSpec = serde_json::from_str(
            r#"{
                "position":{"x":3209,"z":3215,"level":0},
                "inventory":[{"item":"pot_flour","count":1}],
                "bank":[{"item":"egg","count":2}],
                "stats":[{"skill":"cooking","level":4}],
                "quest_varps":[{"name":"cookquest","value":0}]
            }"#,
        )
        .unwrap();
        let actual = SeedReadback {
            position: Some((3209, 3215, 0)),
            inventory: BTreeMap::from([("pot_flour".into(), 1)]),
            bank: Some(BTreeMap::from([("egg".into(), 2)])),
            stat_bases: BTreeMap::from([("cooking".into(), 4)]),
            quest_varps: BTreeMap::from([("cookquest".into(), 0)]),
        };
        assert!(seed_readback_gate(&spec, &actual).is_ok());
        let mut missing = actual.clone();
        missing.bank = None;
        missing.quest_varps.clear();
        let errors = seed_readback_gate(&spec, &missing).unwrap_err();
        assert!(errors.iter().any(|error| error.contains("bank contents")));
        assert!(errors.iter().any(|error| error.contains("cookquest")));
    }

    #[test]
    fn server_getvar_readback_captures_the_actual_value_after_the_request() {
        let pending = PendingRead {
            after_sequence: 4,
            name: "quest_done".into(),
            expected: 1,
            varbit_id: Some(77),
            requested_tick: 3,
        };
        let chat = vec![
            (4, "get quest_done: 0".into()),
            (5, "get quest_done: 0".into()),
        ];
        assert_eq!(find_getvar_reply(&chat, &pending), Some((5, 0)));
    }

    #[test]
    fn assertion_gate_uses_server_varbit_readbacks() {
        let spec: AssertSpec =
            serde_json::from_str(r#"{"varbits":[{"id":77,"value":1}]}"#).unwrap();
        let sample = Sample {
            tick: 0,
            tile: None,
            inventory: BTreeMap::new(),
            inventory_posted: true,
            bank: None,
            stats: BTreeMap::new(),
            varps: Vec::new(),
            chat: Vec::new(),
        };
        let observed = assertion_gate(&spec, &sample, &BTreeMap::new(), &BTreeMap::from([(77, 1)]));
        assert!(observed.1.is_ok());
        let missing = assertion_gate(&spec, &sample, &BTreeMap::new(), &BTreeMap::new());
        assert!(missing.1.unwrap_err()[0].contains("varbit 77"));
    }

    #[test]
    fn danger_refusal_is_not_classified_as_a_product_failure() {
        assert_eq!(
            classify_failure(
                "native Quester status blocked: Walk NoWayOut",
                Some("phase: Blocked")
            ),
            "quester_refusal"
        );
        assert_eq!(
            classify_failure("assertion mismatch: cookquest", Some("phase: Complete")),
            "product"
        );
    }
}
