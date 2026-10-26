#![cfg(all(feature = "live-harness", feature = "test-support"))]

//! Live compatibility-owner takeover cells for the registered rs2b0t Thiever.
//!
//! Run each ignored cell in a separate process under the repository's `isohome`
//! caller. Set `LIVE=1`, `BOT_CPU=1`, `BOT_LIVE_NAME_PREFIX`, `LIVE_EVIDENCE_DIR`,
//! `BOT_ENGINE_DIR`, `BOT_NAV_PACK`, and a fresh per-run `BOT_CACHE_DIR` clone.
//! The cell uses Engine A on loopback: game port 44594 and asset port 1080.
//!
//! Example commands, with the environment above set by `isohome`:
//!
//! ```text
//! cargo test -p host-play --features live-harness,test-support --lib manual_takeover_compat_live_tests::compat_thiever_pauses_on_viewport_click -- --ignored --exact --nocapture --test-threads=1
//! cargo test -p host-play --features live-harness,test-support --lib manual_takeover_compat_live_tests::compat_thiever_pause_off_keeps_running -- --ignored --exact --nocapture --test-threads=1
//! ```

use super::*;
use crate::profile::ProfileEnvironment;
use api::snapshot::GameSnapshot;
use serde::Serialize;
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const CATALOG_COMMIT: &str = "8e7d965be2071d6ec65c3265e12af797082d720a";
const CATALOG_NAME: &str = "Thiever";
const CATALOG_REVISION: u64 = 289;
const LIVE_DEADLINE: Duration = Duration::from_secs(180);
const VIEWPORT_CLICK_X: i32 = 450;
const VIEWPORT_CLICK_Y: i32 = 300;
const TAKEOVER_PREFIX: &str = "walk outcome=cancelled reason=UserInput ";

#[derive(Debug, Clone, Serialize)]
struct HostLine {
    slot: Option<String>,
    record_tick: Option<u32>,
    source: String,
    level: String,
    message: String,
}

#[derive(Default)]
struct HostLineCapture(StdMutex<Vec<HostLine>>);

impl api::hostlog::Sink for HostLineCapture {
    fn record(&self, record: &api::hostlog::Record<'_>) {
        self.0
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .push(HostLine {
                slot: record.slot.map(str::to_owned),
                record_tick: record.tick,
                source: format!("{:?}", record.source),
                level: format!("{:?}", record.level),
                message: record.message.to_owned(),
            });
    }
}

static HOST_LINES: HostLineCapture = HostLineCapture(StdMutex::new(Vec::new()));
static HOST_LINES_INSTALLED: std::sync::LazyLock<bool> =
    std::sync::LazyLock::new(|| api::hostlog::install_sink(&HOST_LINES));

#[derive(Debug, Clone, Default, Serialize)]
struct NavFrameFacts {
    user_move_intent_seq: u64,
    walk_outcome_seq: u64,
    walk_outcome_request_id: u64,
    walk_outcome_generation: u64,
    manual_takeover_watermark: u64,
    route_generation: u64,
    native_walk_live: bool,
    walk_work_live: bool,
    route: bool,
    compact_route: bool,
    has_route: bool,
    pending_route: bool,
    route_ticket: bool,
    route_work_pending: bool,
    bank_fetch: bool,
    carried_walk: bool,
}

impl NavFrameFacts {
    fn no_walking_facts(&self) -> bool {
        !self.native_walk_live
            && !self.walk_work_live
            && !self.route
            && !self.compact_route
            && !self.has_route
            && !self.pending_route
            && !self.route_ticket
            && !self.route_work_pending
            && !self.bank_fetch
            && !self.carried_walk
    }
}

#[derive(Debug, Clone, Default, Serialize)]
struct FrameObservation {
    callback: u64,
    host_tick: Option<u32>,
    snapshot_tick: u32,
    thieving_xp: Option<i32>,
    player_tile: Option<[i32; 3]>,
    ingame: bool,
    scene_state: i32,
    hold: bool,
    manual_move_intent: Option<String>,
    manual_steps: usize,
    script_state: Option<String>,
    want_run: Option<bool>,
    load_active: Option<bool>,
    live_native_owner: Option<bool>,
    live_walking_operation: Option<bool>,
    api_session: Option<bool>,
    nav: Option<NavFrameFacts>,
    runner_status: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize)]
struct RouteFacts {
    native_walk_live: bool,
    walk_work_live: bool,
    route: bool,
    compact_route: bool,
    has_route: bool,
    pending_route: bool,
    route_ticket: bool,
    route_work_pending: bool,
    bank_fetch: bool,
    carried_walk: bool,
}

impl RouteFacts {
    fn no_walking_facts(&self) -> bool {
        !self.native_walk_live
            && !self.walk_work_live
            && !self.route
            && !self.compact_route
            && !self.has_route
            && !self.pending_route
            && !self.route_ticket
            && !self.route_work_pending
            && !self.bank_fetch
            && !self.carried_walk
    }
}

#[derive(Debug, Clone, Serialize)]
struct DecisionFacts {
    request: u64,
    generation: u64,
    intent_seq: u64,
    source: String,
    manual_steps: usize,
    tick: u64,
    live_work: bool,
    active_walk: bool,
    live_native_owner: bool,
    live_walking_operation: bool,
    api_session: bool,
    load_active: bool,
    pause_owner: bool,
    routes: RouteFacts,
}

#[derive(Debug, Clone)]
struct CatalogStart {
    source: String,
    shape: script_js::LoadShape,
    settings_bag: Option<Map<String, Value>>,
    siblings: Vec<(String, String)>,
    loadouts: Vec<api::loadout::Loadout>,
}

struct FrozenCatalog {
    root: PathBuf,
    source_path: String,
    source_sha256: String,
    registry_path: String,
    registry_sha256: String,
}

#[derive(Debug, Serialize)]
struct Receipt {
    schema_version: u8,
    cell: String,
    started_unix_ms: u128,
    target: Value,
    verdict: String,
    failure: Option<String>,
    account: Option<String>,
    profile: Option<Value>,
    catalog: Option<Value>,
    scenario: Option<Value>,
    scenario_result: Option<Value>,
    injected_click: Option<Value>,
    resume: Option<Value>,
    oracle: Option<Value>,
    observations: Vec<FrameObservation>,
    host_lines: Vec<HostLine>,
}

impl Receipt {
    fn new(pause_on: bool) -> Self {
        let started_unix_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        Self {
            schema_version: 1,
            cell: if pause_on {
                "compat-thiever-paused".to_owned()
            } else {
                "compat-thiever-pause-off".to_owned()
            },
            started_unix_ms,
            target: json!({
                "game_host": "127.0.0.1",
                "asset_host": "127.0.0.1",
                "game_port": 44_594,
                "http_port": 1_080,
                "revision": 289,
                "engine_tick_ms": 600,
            }),
            verdict: "RUNNING".to_owned(),
            failure: None,
            account: None,
            profile: None,
            catalog: None,
            scenario: None,
            scenario_result: None,
            injected_click: None,
            resume: None,
            oracle: None,
            observations: Vec::new(),
            host_lines: Vec::new(),
        }
    }
}

struct LiveState {
    account: String,
    runner: scenario::ScenarioRunner,
    snapshot: GameSnapshot,
    pump: Pump,
    catalog: Arc<CatalogStart>,
    start_handle: Option<ScriptStartHandle>,
    scripts: Option<script_runtime::ScriptWall>,
    navs: Option<Arc<StdMutex<HashMap<String, script_runtime::NavBot>>>>,
    start_requested: bool,
    start_ready: bool,
    callback: u64,
    callback_error: Option<String>,
    observations: Vec<FrameObservation>,
}

impl LiveState {
    fn frame(
        &mut self,
        client: &mut client::client::Client,
        username: &str,
        input: SlotFrameInput,
        frame_tx: &Sender<FrameObservation>,
    ) {
        client.set_draw(true);
        let drain = self.pump.drain_client(client);
        host::publish_snapshot(&mut self.snapshot, client, drain);
        self.callback = self.callback.saturating_add(1);
        let mut observation = self.observe(client, username, input);

        if self.callback_error.is_none() {
            match self.pump_scenario(client, input.hold) {
                Ok(()) => {}
                Err(error) => {
                    self.callback_error = Some(error.clone());
                    self.runner.fail_start(&error);
                }
            }
        }
        observation.runner_status = Some(format!("{:?}", self.runner.status()));
        self.observations.push(observation.clone());
        let _ = frame_tx.send(observation);
    }

    fn observe(
        &self,
        client: &client::client::Client,
        username: &str,
        input: SlotFrameInput,
    ) -> FrameObservation {
        let slot = self
            .scripts
            .as_ref()
            .and_then(|scripts| script_runtime::script_slot(scripts, username));
        let (
            script_state,
            want_run,
            load_active,
            live_native_owner,
            live_walking_operation,
            api_session,
        ) = slot
            .and_then(|slot| {
                let slot = slot.lock().ok()?;
                Some((
                    format!("{:?}", slot.state()),
                    slot.want_run,
                    slot.surface_active(),
                    slot.live_native_owner(),
                    slot.live_walking_operation(),
                    slot.api_session_takes_manual_input(),
                ))
            })
            .map_or((None, None, None, None, None, None), |facts| {
                (
                    Some(facts.0),
                    Some(facts.1),
                    Some(facts.2),
                    Some(facts.3),
                    Some(facts.4),
                    Some(facts.5),
                )
            });
        let nav = self.navs.as_ref().and_then(|navs| {
            let all = navs.lock().ok()?;
            let bot = all.get(username)?;
            Some(NavFrameFacts {
                user_move_intent_seq: bot.user_move_intent_seq,
                walk_outcome_seq: bot.walk_outcome_seq,
                walk_outcome_request_id: bot.walk_outcome_request_id,
                walk_outcome_generation: bot.walk_outcome_generation,
                manual_takeover_watermark: bot.manual_takeover_watermark,
                route_generation: bot.route_generation,
                native_walk_live: bot.native_walk.as_ref().is_some_and(|owner| owner.live()),
                walk_work_live: bot.walk_work_live(),
                route: bot.route.is_some(),
                compact_route: bot.compact_route.is_some(),
                has_route: bot.has_route(),
                pending_route: bot.pending_route.is_some(),
                route_ticket: bot.route_ticket.is_some(),
                route_work_pending: bot.route_work_pending(),
                bank_fetch: bot.bank_fetch.is_some(),
                carried_walk: bot.carried_walk.is_some(),
            })
        });
        FrameObservation {
            callback: self.callback,
            host_tick: api::hostlog::slot_tick(username),
            snapshot_tick: self.snapshot.tick(),
            thieving_xp: self
                .snapshot
                .stats()
                .iter()
                .find(|stat| stat.index == 17)
                .map(|stat| stat.xp),
            player_tile: self.snapshot.tile().map(|(x, z, level)| [x, z, level]),
            ingame: client.ingame,
            scene_state: client.scene_state,
            hold: input.hold,
            manual_move_intent: input.manual_move_intent.map(|intent| format!("{intent:?}")),
            manual_steps: input.manual_steps,
            script_state,
            want_run,
            load_active,
            live_native_owner,
            live_walking_operation,
            api_session,
            nav,
            runner_status: None,
        }
    }

    fn pump_scenario(
        &mut self,
        client: &mut client::client::Client,
        hold: bool,
    ) -> Result<(), String> {
        if self.runner.on_start_script() && !self.start_ready {
            let handle = self.start_handle.as_ref().ok_or_else(|| {
                "scenario reached catalog Start before its host handle was installed".to_owned()
            })?;
            if !self.start_requested {
                handle.start_load_with_loadouts(
                    &self.account,
                    self.catalog.source.clone(),
                    self.catalog.shape,
                    self.catalog.settings_bag.clone(),
                    self.catalog.siblings.clone(),
                    &self.catalog.loadouts,
                )?;
                self.start_requested = true;
            }
            match handle.poll_start(&self.account) {
                script::StartPoll::Pending => return Ok(()),
                script::StartPoll::Settled(script::StartOutcome::Ready) => {
                    self.start_ready = true;
                    self.runner.observe_script_running();
                }
                script::StartPoll::Settled(outcome) => {
                    let error = format!("registered Thiever Start settled as {outcome:?}");
                    self.runner.fail_start(&error);
                    return Err(error);
                }
                script::StartPoll::NotOwed => {
                    let error = "registered Thiever Start has no pending settlement".to_owned();
                    self.runner.fail_start(&error);
                    return Err(error);
                }
            }
        }
        if !matches!(
            self.runner.status(),
            scenario::RunnerStatus::Passed | scenario::RunnerStatus::Failed(_)
        ) {
            self.runner.tick_with_hold(client, hold);
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
struct CellFrames {
    start: FrameObservation,
    pre_click: FrameObservation,
    click: FrameObservation,
    post_action: FrameObservation,
}

fn required_env_path(name: &str) -> Result<PathBuf, String> {
    let path = std::env::var_os(name)
        .map(PathBuf::from)
        .ok_or_else(|| format!("{name} must be explicitly supplied"))?;
    if !path.is_absolute() {
        return Err(format!(
            "{name} must be an absolute path: {}",
            path.display()
        ));
    }
    Ok(path)
}

fn canonical_directory(name: &str) -> Result<PathBuf, String> {
    let supplied = required_env_path(name)?;
    let canonical = std::fs::canonicalize(&supplied)
        .map_err(|error| format!("resolve {name} {}: {error}", supplied.display()))?;
    if !canonical.is_dir() {
        return Err(format!(
            "{name} must be a directory: {}",
            canonical.display()
        ));
    }
    Ok(canonical)
}

fn canonical_file(name: &str) -> Result<PathBuf, String> {
    let supplied = required_env_path(name)?;
    let canonical = std::fs::canonicalize(&supplied)
        .map_err(|error| format!("resolve {name} {}: {error}", supplied.display()))?;
    if !canonical.is_file() {
        return Err(format!("{name} must be a file: {}", canonical.display()));
    }
    Ok(canonical)
}

fn root_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn find_json_object<'a>(
    value: &'a Value,
    predicate: &impl Fn(&Map<String, Value>) -> bool,
) -> Option<&'a Map<String, Value>> {
    match value {
        Value::Object(object) => {
            if predicate(object) {
                Some(object)
            } else {
                object
                    .values()
                    .find_map(|child| find_json_object(child, predicate))
            }
        }
        Value::Array(rows) => rows
            .iter()
            .find_map(|child| find_json_object(child, predicate)),
        _ => None,
    }
}

fn frozen_catalog() -> Result<FrozenCatalog, String> {
    let matrix: Value = serde_json::from_str(include_str!(
        "../tests/fixtures/catalog-support-matrix.json"
    ))
    .map_err(|error| format!("parse frozen catalog support matrix: {error}"))?;
    let catalog = find_json_object(&matrix, &|object| {
        object.get("commit").and_then(Value::as_str) == Some(CATALOG_COMMIT)
    })
    .ok_or_else(|| format!("support matrix has no catalog commit {CATALOG_COMMIT}"))?;
    let read_only_path = catalog
        .get("read_only_path")
        .and_then(Value::as_str)
        .ok_or("frozen catalog has no read_only_path")?;
    let registry_path = catalog
        .get("registry_path")
        .and_then(Value::as_str)
        .ok_or("frozen catalog has no registry_path")?
        .to_owned();
    let registry_sha256 = catalog
        .get("registry_sha256")
        .and_then(Value::as_str)
        .ok_or("frozen catalog has no registry_sha256")?
        .to_owned();
    let row = find_json_object(&matrix, &|object| {
        object.get("display_name").and_then(Value::as_str) == Some(CATALOG_NAME)
            && object.get("catalog_commit").and_then(Value::as_str) == Some(CATALOG_COMMIT)
            && object.get("revision").and_then(Value::as_u64) == Some(CATALOG_REVISION)
    })
    .ok_or("support matrix has no R289 Thiever source row for the selected frozen commit")?;
    let source_path = row
        .get("source_path")
        .and_then(Value::as_str)
        .ok_or("Thiever matrix row has no source_path")?
        .to_owned();
    let source_sha256 = row
        .get("source_sha256")
        .and_then(Value::as_str)
        .ok_or("Thiever matrix row has no source_sha256")?
        .to_owned();
    let repository = std::fs::canonicalize(root_path()).map_err(|error| error.to_string())?;
    let selected_root = std::env::var_os("RS2B0T")
        .map(PathBuf::from)
        .unwrap_or_else(|| repository.join(read_only_path));
    let root = std::fs::canonicalize(&selected_root).map_err(|error| {
        format!(
            "resolve frozen catalog root {}: {error}",
            selected_root.display()
        )
    })?;
    let source = std::fs::canonicalize(root.join(&source_path))
        .map_err(|error| format!("resolve frozen Thiever source {source_path}: {error}"))?;
    let registry = std::fs::canonicalize(root.join(&registry_path))
        .map_err(|error| format!("resolve frozen catalog registry {registry_path}: {error}"))?;
    if !source.starts_with(&root) || !registry.starts_with(&root) {
        return Err("frozen catalog paths escape the selected read-only root".to_owned());
    }
    let actual_source_sha256 =
        script_js::JsCache::origin_sha(&std::fs::read(&source).map_err(|error| error.to_string())?);
    if actual_source_sha256 != source_sha256 {
        return Err(format!(
            "frozen Thiever source identity changed: expected {source_sha256}, got {actual_source_sha256}"
        ));
    }
    let actual_registry_sha256 = script_js::JsCache::origin_sha(
        &std::fs::read(&registry).map_err(|error| error.to_string())?,
    );
    if actual_registry_sha256 != registry_sha256 {
        return Err(format!(
            "frozen catalog registry identity changed: expected {registry_sha256}, got {actual_registry_sha256}"
        ));
    }
    Ok(FrozenCatalog {
        root,
        source_path,
        source_sha256,
        registry_path,
        registry_sha256,
    })
}

fn catalog_start(
    scenario: &scenario::Scenario,
    frozen: &FrozenCatalog,
    scratch: &Path,
) -> Result<CatalogStart, String> {
    let card_name = scenario
        .settings
        .start_script
        .ok_or("original thiever scenario has no catalog Start script")?;
    if card_name != CATALOG_NAME {
        return Err(format!(
            "original thiever scenario selected unexpected script {card_name:?}"
        ));
    }
    let mut library = script_js::JsLibrary::with_cache(
        scratch.join("scripts.json"),
        scratch.join("javascript-cache"),
    );
    library.register_rs2b0t(&frozen.root, &scratch.join("rs2b0t-path"))?;
    library.ensure_js(script::ScriptSource::Catalog, card_name)?;
    let card = library
        .get(script::ScriptSource::Catalog, card_name)
        .cloned()
        .ok_or_else(|| format!("registered frozen catalog has no {card_name} card"))?;
    let loaded_sha256 = script_js::JsCache::origin_sha(card.origin.as_bytes());
    if loaded_sha256 != frozen.source_sha256 {
        return Err(format!(
            "registered {card_name} source differs from support-matrix identity: expected {}, got {loaded_sha256}",
            frozen.source_sha256
        ));
    }
    let settings_inject = scenario::settings_inject_map(scenario.settings.script_settings_inject);
    let settings_bag = script::settings_store::merge_bag(
        &card.settings_schema,
        &Map::new(),
        settings_inject.as_ref(),
    );
    let siblings = script_js::resolve_sibling_modules(
        &card.path,
        &card.origin,
        library.cache(),
        script_js::CacheMeta {
            kind: card.kind,
            source: card.source,
            shape: None,
            api_family: Some(card.api_family.as_str().into()),
        },
    )?;
    let loadouts = scenario
        .settings
        .fixture_loadouts
        .unwrap_or(&[])
        .iter()
        .map(|row| {
            row.carry.iter().fold(
                api::loadout::Loadout::new(row.name),
                |loadout, &(item, quantity)| loadout.with_carry(item, quantity),
            )
        })
        .collect();
    Ok(CatalogStart {
        source: card.js,
        shape: card.shape,
        settings_bag: Some(settings_bag),
        siblings,
        loadouts,
    })
}

fn profile_options(
    home: &Path,
    catalog_root: &Path,
) -> Result<(ProfileOptions, ProfileEnvironment, Value), String> {
    let engine_dir = canonical_directory("BOT_ENGINE_DIR")?;
    let nav_pack = canonical_file("BOT_NAV_PACK")?;
    let cache_dir = canonical_directory("BOT_CACHE_DIR")?;
    let game_host = "127.0.0.1".to_owned();
    let asset_host = game_host.clone();
    let game_port = 44_594;
    let http_port = 1_080;

    let options = ProfileOptions {
        profile: Some("local-289".to_owned()),
        revision: Some("289".to_owned()),
        host: Some(game_host.clone()),
        port: Some(game_port),
        asset_host: Some(asset_host.clone()),
        http_port: Some(http_port),
        engine_dir: Some(engine_dir.clone()),
        cache_dir: Some(cache_dir.clone()),
        unpack_dir: Some(home.join("unpack")),
        nav_pack: Some(nav_pack.clone()),
        vault_path: Some(home.join("vault")),
        catalog_root: Some(catalog_root.to_path_buf()),
        ..ProfileOptions::default()
    };
    let mut environment = ProfileEnvironment::capture();
    environment.home = Some(home.to_path_buf());
    environment.engine_dir = Some(engine_dir.clone());
    environment.nav_pack = Some(nav_pack.clone());
    environment.catalog_root = Some(catalog_root.to_path_buf());
    let facts = json!({
        "profile": "local-289",
        "revision": 289,
        "game_host": game_host,
        "game_port": game_port,
        "asset_host": asset_host,
        "http_port": http_port,
        "engine_dir": engine_dir,
        "nav_pack": nav_pack,
        "cache_dir": cache_dir,
        "one_run_home": home,
        "allow_plaintext_offhost": false,
    });
    Ok((options, environment, facts))
}

fn host_lines_for(account: &str) -> Vec<HostLine> {
    HOST_LINES
        .0
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .iter()
        .filter(|line| line.slot.as_deref() == Some(account))
        .cloned()
        .collect()
}

fn is_takeover_line(line: &HostLine) -> bool {
    line.message.starts_with(TAKEOVER_PREFIX)
}

fn parse_decision_line(line: &HostLine) -> Result<DecisionFacts, String> {
    if !is_takeover_line(line) {
        return Err("host line is not a UserInput takeover line".to_owned());
    }
    let mut fields = HashMap::new();
    for token in line.message.split_whitespace() {
        if let Some((key, value)) = token.split_once('=') {
            if fields.insert(key, value).is_some() {
                return Err(format!("takeover host line repeats field {key}"));
            }
        }
    }
    let required = |name: &str| {
        fields
            .get(name)
            .copied()
            .ok_or_else(|| format!("takeover host line is missing {name}"))
    };
    let bool_field = |name: &str| -> Result<bool, String> {
        match required(name)? {
            "true" => Ok(true),
            "false" => Ok(false),
            value => Err(format!("takeover field {name} is not a bool: {value:?}")),
        }
    };
    let number = |name: &str| -> Result<u64, String> {
        required(name)?
            .parse::<u64>()
            .map_err(|error| format!("takeover field {name} is not a number: {error}"))
    };
    let manual_steps = required("manual_steps")?
        .parse::<usize>()
        .map_err(|error| format!("takeover field manual_steps is not a number: {error}"))?;
    let source = required("source")?.to_owned();
    let tick = number("tick")?;
    Ok(DecisionFacts {
        request: number("request")?,
        generation: number("generation")?,
        intent_seq: number("intent_seq")?,
        source,
        manual_steps,
        tick,
        live_work: bool_field("live_work")?,
        active_walk: bool_field("active_walk")?,
        live_native_owner: bool_field("live_native_owner")?,
        live_walking_operation: bool_field("live_walking_operation")?,
        api_session: bool_field("api_session")?,
        load_active: bool_field("load_active")?,
        pause_owner: bool_field("pause_owner")?,
        routes: RouteFacts {
            native_walk_live: bool_field("native_walk_live")?,
            walk_work_live: bool_field("walk_work_live")?,
            route: bool_field("route")?,
            compact_route: bool_field("compact_route")?,
            has_route: bool_field("has_route")?,
            pending_route: bool_field("pending_route")?,
            route_ticket: bool_field("route_ticket")?,
            route_work_pending: bool_field("route_work_pending")?,
            bank_fetch: bool_field("bank_fetch")?,
            carried_walk: bool_field("carried_walk")?,
        },
    })
}

fn no_live_owner_or_walk(frame: &FrameObservation) -> Result<(), String> {
    if frame.live_native_owner != Some(false) {
        return Err("frame does not prove live_native_owner=false".to_owned());
    }
    if frame.live_walking_operation != Some(false) {
        return Err("frame does not prove live_walking_operation=false".to_owned());
    }
    if frame.api_session != Some(false) {
        return Err("frame does not prove api_session=false".to_owned());
    }
    let nav = frame.nav.as_ref().ok_or("frame has no navigation facts")?;
    if !nav.no_walking_facts() {
        return Err("frame contains a live native, route, or carried-walk fact".to_owned());
    }
    Ok(())
}

fn paused_frame_oracle(
    line: &HostLine,
    before: &FrameObservation,
    click: &FrameObservation,
) -> Result<DecisionFacts, String> {
    let facts = parse_decision_line(line)?;
    if line.record_tick.is_none() || click.host_tick != line.record_tick {
        return Err(format!(
            "takeover log tick {:?} does not match the same-frame observation tick {:?}",
            line.record_tick, click.host_tick
        ));
    }
    if before.callback >= click.callback {
        return Err("the viewport click did not follow the pre-click observation".to_owned());
    }
    if before.script_state.as_deref() != Some("Running")
        || click.script_state.as_deref() != Some("Paused")
    {
        return Err("the same-frame script states are not Running then Paused".to_owned());
    }
    if before.manual_move_intent.is_some() {
        return Err("the pre-click observation already contains manual input".to_owned());
    }
    if click.manual_move_intent.as_deref() != Some("Viewport") || click.manual_steps != 0 {
        return Err(
            "takeover did not follow exactly one world-view click and zero WireCmd Walk steps"
                .to_owned(),
        );
    }
    if before.load_active != Some(true) || click.load_active != Some(true) {
        return Err(
            "the observed compatibility Load was not active before and after takeover".to_owned(),
        );
    }
    no_live_owner_or_walk(before)?;
    no_live_owner_or_walk(click)?;
    let before_nav = before
        .nav
        .as_ref()
        .ok_or("pre-click frame has no nav counters")?;
    let click_nav = click
        .nav
        .as_ref()
        .ok_or("click frame has no nav counters")?;
    if click_nav.user_move_intent_seq != before_nav.user_move_intent_seq.saturating_add(1) {
        return Err(
            "one viewport click did not increment the user intent sequence once".to_owned(),
        );
    }
    if click_nav.walk_outcome_seq != before_nav.walk_outcome_seq.saturating_add(1) {
        return Err("the paused takeover did not publish exactly one walk outcome".to_owned());
    }
    if facts.source != "Some(Viewport)" || facts.manual_steps != 0 {
        return Err(
            "host line does not cite one viewport click and zero manual walk steps".to_owned(),
        );
    }
    if facts.intent_seq != click_nav.user_move_intent_seq {
        return Err(
            "host line intent sequence does not match the same-frame nav observation".to_owned(),
        );
    }
    if !facts.live_work
        || facts.active_walk
        || facts.live_native_owner
        || facts.live_walking_operation
        || facts.api_session
        || !facts.load_active
        || !facts.pause_owner
        || !facts.routes.no_walking_facts()
    {
        return Err(
            "host line does not prove a Load-only pause with every walking fact inactive"
                .to_owned(),
        );
    }
    if click.live_native_owner != Some(false)
        || click.live_walking_operation != Some(false)
        || click.api_session != Some(false)
    {
        return Err(
            "same-frame observation has a native owner, walking operation, or input API session"
                .to_owned(),
        );
    }
    Ok(facts)
}

fn paused_oracle(
    lines: &[HostLine],
    before: &FrameObservation,
    click: &FrameObservation,
    after_resume: &FrameObservation,
) -> Result<(HostLine, DecisionFacts), String> {
    let candidates = lines
        .iter()
        .filter(|line| is_takeover_line(line))
        .cloned()
        .collect::<Vec<_>>();
    if candidates.len() != 1 {
        return Err(format!(
            "expected one UserInput takeover line, found {}",
            candidates.len()
        ));
    }
    let line = candidates.into_iter().next().expect("one candidate");
    let facts = paused_frame_oracle(&line, before, click)?;
    if after_resume.callback <= click.callback
        || after_resume.host_tick <= click.host_tick
        || after_resume.script_state.as_deref() != Some("Running")
        || after_resume.load_active != Some(true)
    {
        return Err(
            "Resume did not return the compatibility script to Running on a later tick".to_owned(),
        );
    }
    no_live_owner_or_walk(after_resume)?;
    let pause_xp = click
        .thieving_xp
        .ok_or("paused click observation has no Thieving XP")?;
    let resumed_xp = after_resume
        .thieving_xp
        .ok_or("post-Resume observation has no Thieving XP")?;
    if resumed_xp <= pause_xp {
        return Err("registered Thiever produced no new Thieving XP after Resume".to_owned());
    }
    Ok((line, facts))
}

fn pause_off_oracle(
    lines: &[HostLine],
    before: &FrameObservation,
    click: &FrameObservation,
    after_click: &FrameObservation,
) -> Result<(), String> {
    let candidates = lines.iter().filter(|line| is_takeover_line(line)).count();
    if candidates != 0 {
        return Err(format!(
            "Pause-off cell emitted {candidates} takeover lines"
        ));
    }
    if before.callback >= click.callback
        || before.script_state.as_deref() != Some("Running")
        || click.script_state.as_deref() != Some("Running")
        || click.manual_move_intent.as_deref() != Some("Viewport")
        || click.manual_steps != 0
        || before.load_active != Some(true)
        || click.load_active != Some(true)
    {
        return Err(
            "Pause-off click did not leave the active compatibility script Running".to_owned(),
        );
    }
    no_live_owner_or_walk(before)?;
    no_live_owner_or_walk(click)?;
    let before_nav = before
        .nav
        .as_ref()
        .ok_or("pre-click frame has no nav counters")?;
    let click_nav = click
        .nav
        .as_ref()
        .ok_or("click frame has no nav counters")?;
    if click_nav.user_move_intent_seq != before_nav.user_move_intent_seq.saturating_add(1) {
        return Err(
            "one viewport click did not increment the user intent sequence once".to_owned(),
        );
    }
    if click_nav.walk_outcome_seq != before_nav.walk_outcome_seq {
        return Err("Pause-off click changed the walk outcome sequence".to_owned());
    }
    if after_click.callback <= click.callback
        || after_click.host_tick <= click.host_tick
        || after_click.script_state.as_deref() != Some("Running")
        || after_click.load_active != Some(true)
    {
        return Err("Pause-off script did not remain Running on a later tick".to_owned());
    }
    no_live_owner_or_walk(after_click)?;
    if after_click
        .nav
        .as_ref()
        .ok_or("post-click frame has no nav counters")?
        .walk_outcome_seq
        != before_nav.walk_outcome_seq
    {
        return Err("Pause-off walk outcome changed after the click".to_owned());
    }
    let click_xp = click
        .thieving_xp
        .ok_or("click observation has no Thieving XP")?;
    let after_xp = after_click
        .thieving_xp
        .ok_or("post-click observation has no Thieving XP")?;
    if after_xp <= click_xp {
        return Err(
            "registered Thiever produced no new Thieving XP after the Pause-off click".to_owned(),
        );
    }
    Ok(())
}

fn same_frame_host_line(lines: &[HostLine], frame: &FrameObservation) -> Result<HostLine, String> {
    let matching = lines
        .iter()
        .filter(|line| is_takeover_line(line) && line.record_tick == frame.host_tick)
        .cloned()
        .collect::<Vec<_>>();
    if matching.len() != 1 {
        return Err(format!(
            "expected one takeover line at observation tick {:?}, found {}",
            frame.host_tick,
            matching.len()
        ));
    }
    Ok(matching.into_iter().next().expect("one matching line"))
}

fn run_live_sequence(
    pause_on: bool,
    account: &str,
    play: &Play,
    click_tx: &Sender<host::InputEv>,
    frame_rx: &Receiver<FrameObservation>,
    live: &Arc<StdMutex<LiveState>>,
) -> Result<CellFrames, String> {
    let deadline = Instant::now() + LIVE_DEADLINE;
    let mut start = None;
    let mut pre_click = None;
    let mut click = None;
    let mut post_action = None;
    let mut click_sent = false;
    let mut start_xp = None;

    while Instant::now() < deadline {
        {
            let state = live.lock().unwrap_or_else(|error| error.into_inner());
            if let Some(error) = &state.callback_error {
                return Err(format!("live frame callback failed: {error}"));
            }
            if let scenario::RunnerStatus::Failed(error) = state.runner.status() {
                return Err(format!("original thiever scenario failed: {error}"));
            }
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        let observation = match frame_rx.recv_timeout(remaining) {
            Ok(frame) => frame,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                return Err(format!(
                    "live cell exceeded its fixed {} second deadline",
                    LIVE_DEADLINE.as_secs()
                ));
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                return Err("live slot frame observer disconnected".to_owned());
            }
        };

        if !click_sent
            && observation.script_state.as_deref() == Some("Running")
            && observation.load_active == Some(true)
            && no_live_owner_or_walk(&observation).is_ok()
        {
            let xp = observation
                .thieving_xp
                .ok_or("running compatibility script has no observed Thieving XP")?;
            if let Some(baseline) = start_xp {
                if xp > baseline {
                    let tx = click_tx.clone();
                    tx.send(host::InputEv::Down {
                        button: 1,
                        x: VIEWPORT_CLICK_X,
                        y: VIEWPORT_CLICK_Y,
                    })
                    .map_err(|error| format!("send world-view mouse Down: {error}"))?;
                    tx.send(host::InputEv::Up)
                        .map_err(|error| format!("send world-view mouse Up: {error}"))?;
                    pre_click = Some(observation.clone());
                    click_sent = true;
                }
            } else {
                start_xp = Some(xp);
                start = Some(observation.clone());
            }
        } else if click_sent && click.is_none() && observation.manual_move_intent.is_some() {
            if observation.manual_move_intent.as_deref() != Some("Viewport") {
                return Err(format!(
                    "SlotInput click was classified as {:?}, not a world-view click",
                    observation.manual_move_intent
                ));
            }
            click = Some(observation.clone());
            if pause_on {
                let before = pre_click
                    .as_ref()
                    .ok_or("click has no pre-click observation")?;
                let lines = host_lines_for(account);
                let line = same_frame_host_line(&lines, &observation)?;
                paused_frame_oracle(&line, before, &observation)?;
                play.script_resume(account);
            }
        } else if let Some(click_frame) = click.as_ref().filter(|_| post_action.is_none()) {
            if observation.callback > click_frame.callback
                && observation.host_tick > click_frame.host_tick
                && observation.script_state.as_deref() == Some("Running")
                && observation.thieving_xp > click_frame.thieving_xp
            {
                post_action = Some(observation.clone());
            }
        }

        if post_action.is_some() {
            let state = live.lock().unwrap_or_else(|error| error.into_inner());
            if matches!(state.runner.status(), scenario::RunnerStatus::Passed) {
                return Ok(CellFrames {
                    start: start.ok_or("no Running start observation")?,
                    pre_click: pre_click.ok_or("no pre-click observation")?,
                    click: click.ok_or("no click observation")?,
                    post_action: post_action.ok_or("no post-click action observation")?,
                });
            }
        }
    }
    Err(format!(
        "live cell exceeded its fixed {} second deadline",
        LIVE_DEADLINE.as_secs()
    ))
}

fn run_cell(pause_on: bool) {
    let evidence_dir = std::env::var_os("LIVE_EVIDENCE_DIR")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .unwrap_or_else(|| panic!("LIVE_EVIDENCE_DIR must be an absolute output directory"));
    std::fs::create_dir_all(&evidence_dir).unwrap_or_else(|error| {
        panic!(
            "create LIVE_EVIDENCE_DIR {}: {error}",
            evidence_dir.display()
        )
    });
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let mut receipt = Receipt::new(pause_on);
    let run_name = format!("{}_{}", receipt.cell, stamp);
    let receipt_path = evidence_dir.join(format!("{run_name}.json"));
    let run_dir = evidence_dir.join(&run_name);
    let mut result = run_cell_inner(pause_on, &run_dir, &mut receipt);
    receipt.verdict = if result.is_ok() {
        "PASS".to_owned()
    } else {
        "FAIL".to_owned()
    };
    if let Err(error) = &result {
        receipt.failure = Some(error.clone());
    }
    if let Err(error) = write_receipt(&receipt_path, &receipt) {
        result = Err(format!(
            "write JSON receipt {}: {error}",
            receipt_path.display()
        ));
    }
    println!(
        "manual-takeover-compat-evidence-json={}",
        receipt_path.display()
    );
    if let Err(error) = result {
        panic!("{}: {error}", receipt.cell);
    }
}

fn run_cell_inner(pause_on: bool, run_dir: &Path, receipt: &mut Receipt) -> Result<(), String> {
    if std::env::var("LIVE").as_deref() != Ok("1") {
        return Err("set LIVE=1 to authorize this ignored live cell".to_owned());
    }
    if !*HOST_LINES_INSTALLED {
        return Err("the process already has a different host-log sink; run this cell in its own test process".to_owned());
    }
    std::fs::create_dir_all(run_dir).map_err(|error| error.to_string())?;
    let home = run_dir.join("home");
    let scratch = run_dir.join("scratch");
    std::fs::create_dir_all(&home).map_err(|error| error.to_string())?;
    std::fs::create_dir_all(&scratch).map_err(|error| error.to_string())?;

    let names = mint_live_names(1);
    let account = names
        .first()
        .cloned()
        .ok_or("live account name mint returned no account")?;
    let (_, password) = mint_live_entries(&names)
        .into_iter()
        .next()
        .ok_or("live account credential mint returned no account")?;
    receipt.account = Some(account.clone());

    let frozen = frozen_catalog()?;
    receipt.catalog = Some(json!({
        "commit": CATALOG_COMMIT,
        "root": frozen.root,
        "source_path": frozen.source_path,
        "source_sha256": frozen.source_sha256,
        "registry_path": frozen.registry_path,
        "registry_sha256": frozen.registry_sha256,
    }));
    let scenario = scenario::get("thiever").ok_or("scenario::get(\"thiever\") returned None")?;
    if scenario.name != "thiever" || scenario.settings.start_script != Some(CATALOG_NAME) {
        return Err(
            "scenario::get(\"thiever\") did not return the original Thiever scenario".to_owned(),
        );
    }
    let settings_inject = scenario::settings_inject_map(scenario.settings.script_settings_inject);
    let catalog = Arc::new(catalog_start(&scenario, &frozen, &scratch)?);
    receipt.scenario = Some(json!({
        "registry_scenario": scenario.name,
        "start_script": scenario.settings.start_script,
        "settings_inject": settings_inject,
        "fixture_loadouts": scenario.settings.fixture_loadouts.unwrap_or(&[])
            .iter()
            .map(|row| json!({"name": row.name, "carry": row.carry}))
            .collect::<Vec<_>>(),
        "original_deadline_seconds": scenario.settings.deadline.as_secs(),
        "fixed_cell_deadline_seconds": LIVE_DEADLINE.as_secs(),
        "proof": format!("{:?}", scenario.proof),
    }));
    let (options, mut profile_environment, profile_facts) = profile_options(&home, &frozen.root)?;
    profile_environment.home = Some(home.clone());
    receipt.profile = Some(profile_facts);
    let template = options
        .resolve_with_env(None, &profile_environment)?
        .prepare_template()?;

    let mut runner = scenario::ScenarioRunner::with_world(scenario, template.world());
    runner.set_live_names(&names);
    runner.set_deadline(LIVE_DEADLINE);
    runner.set_shot_sink(Box::new(|_, _| {}));

    let (frame_tx, frame_rx) = mpsc::channel();
    let live = Arc::new(StdMutex::new(LiveState {
        account: account.clone(),
        runner,
        snapshot: GameSnapshot::new(),
        pump: Pump::new(),
        catalog,
        start_handle: None,
        scripts: None,
        navs: None,
        start_requested: false,
        start_ready: false,
        callback: 0,
        callback_error: None,
        observations: Vec::new(),
    }));
    let frame_live = Arc::clone(&live);
    let frame_account = account.clone();
    let frame_sender = frame_tx.clone();
    let slot_input = host::SlotInput::new();
    slot_input.set_enabled(true);
    let (click_tx, input_rx) = mpsc::channel();
    slot_input.connect_rx(input_rx);
    let slot_input_for_play = Arc::clone(&slot_input);
    let profile = nav_arrival_live_tests::live_profile(account.clone(), password);
    let mut play = run_with_template(
        template,
        true,
        vec![profile],
        move |_| (Some(Arc::clone(&slot_input_for_play)), None),
        move |client, username, input| {
            if username == frame_account {
                frame_live
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .frame(client, username, input, &frame_sender);
            }
        },
    )?;
    play.set_pause_script_on_manual_walk_abort(pause_on);
    play.focus(&account);
    {
        let mut state = live.lock().unwrap_or_else(|error| error.into_inner());
        state.runner.set_obj_names(play.obj_names());
        state.start_handle = Some(play.script_start_handle());
        state.scripts = Some(Arc::clone(&play.scripts));
        state.navs = Some(Arc::clone(&play.navs));
    }

    let sequence = run_live_sequence(pause_on, &account, &play, &click_tx, &frame_rx, &live);
    let (observations, scenario_result) = {
        let mut state = live.lock().unwrap_or_else(|error| error.into_inner());
        let status = format!("{:?}", state.runner.status());
        let (step, total, step_name) = state.runner.record_step();
        let result = json!({
            "status": status,
            "total_ticks": state.runner.total_ticks(),
            "deadline_exceeded": state.runner.deadline_exceeded(),
            "record_step": {"index": step, "total": total, "name": step_name},
            "start_requested": state.start_requested,
            "start_ready": state.start_ready,
            "callback_error": state.callback_error,
        });
        (std::mem::take(&mut state.observations), result)
    };
    drop(play);
    receipt.observations = observations;
    receipt.host_lines = host_lines_for(&account);
    receipt.scenario_result = Some(scenario_result);
    receipt.injected_click = Some(json!({
        "input": "host::SlotInput via InputEv::Down then InputEv::Up",
        "button": 1,
        "viewport_xy": [VIEWPORT_CLICK_X, VIEWPORT_CLICK_Y],
        "manual_wire_walk_steps": 0,
    }));

    let frames = sequence?;
    let status = receipt
        .scenario_result
        .as_ref()
        .and_then(|result| result.get("status"))
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    if status != "Passed" {
        return Err(format!("original thiever scenario did not pass: {status}"));
    }
    let facts = frames_json(&frames);
    if pause_on {
        let (line, decision) = paused_oracle(
            &receipt.host_lines,
            &frames.pre_click,
            &frames.click,
            &frames.post_action,
        )?;
        receipt.resume = Some(json!({
            "action": "Play::script_resume",
            "after_takeover_tick": frames.click.host_tick,
            "new_thieving_xp_tick": frames.post_action.host_tick,
        }));
        receipt.oracle = Some(json!({
            "verdict": "PASS",
            "claim": "registered compatibility Thiever paused on a world-view click without any native owner or walking facts, then resumed and produced new Thieving XP",
            "takeover_line": line,
            "takeover_facts": decision,
            "observations": facts,
        }));
    } else {
        pause_off_oracle(
            &receipt.host_lines,
            &frames.pre_click,
            &frames.click,
            &frames.post_action,
        )?;
        receipt.resume = None;
        receipt.oracle = Some(json!({
            "verdict": "PASS",
            "claim": "with Pause off, the world-view click caused no takeover, kept compatibility Thiever Running, incremented user intent once, and left walk outcome unchanged while Thieving XP advanced",
            "takeover_line_count": receipt.host_lines.iter().filter(|line| is_takeover_line(line)).count(),
            "observations": facts,
        }));
    }
    Ok(())
}

fn frames_json(frames: &CellFrames) -> Value {
    json!({
        "start": frames.start,
        "pre_click_xp": frames.pre_click,
        "click": frames.click,
        "post_action": frames.post_action,
    })
}

fn write_receipt(path: &Path, receipt: &Receipt) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(receipt).map_err(|error| error.to_string())?;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| error.to_string())?;
    file.write_all(&bytes)
        .and_then(|()| file.sync_all())
        .map_err(|error| error.to_string())
}

fn fixture_host_line(message: String, record_tick: Option<u32>) -> HostLine {
    HostLine {
        slot: Some("fixture".to_owned()),
        record_tick,
        source: "Nav".to_owned(),
        level: "Info".to_owned(),
        message,
    }
}

fn fixture_takeover_message() -> String {
    "walk outcome=cancelled reason=UserInput request=0 generation=0 intent_seq=1 source=Some(Viewport) manual_steps=0 tick=41 live_work=true active_walk=false live_native_owner=false live_walking_operation=false api_session=false load_active=true pause_owner=true native_walk_live=false walk_work_live=false route=false compact_route=false has_route=false pending_route=false route_ticket=false route_work_pending=false bank_fetch=false carried_walk=false".to_owned()
}

fn fixture_frame(
    callback: u64,
    tick: u32,
    state: &str,
    xp: i32,
    intent: u64,
    outcome: u64,
) -> FrameObservation {
    FrameObservation {
        callback,
        host_tick: Some(tick),
        snapshot_tick: tick,
        thieving_xp: Some(xp),
        manual_move_intent: (state == "Paused").then(|| "Viewport".to_owned()),
        script_state: Some(state.to_owned()),
        want_run: Some(true),
        load_active: Some(true),
        live_native_owner: Some(false),
        live_walking_operation: Some(false),
        api_session: Some(false),
        nav: Some(NavFrameFacts {
            user_move_intent_seq: intent,
            walk_outcome_seq: outcome,
            ..NavFrameFacts::default()
        }),
        ..FrameObservation::default()
    }
}

#[test]
#[ignore = "requires LIVE=1, an owned live account, and Engine A on loopback at 600 ms"]
fn compat_thiever_pauses_on_viewport_click() {
    run_cell(true);
}

#[test]
#[ignore = "requires LIVE=1, an owned live account, and Engine A on loopback at 600 ms"]
fn compat_thiever_pause_off_keeps_running() {
    run_cell(false);
}

#[test]
fn paused_oracle_rejects_native_owner_false_positive() {
    let before = fixture_frame(1, 42, "Running", 100, 0, 7);
    let mut click = fixture_frame(2, 42, "Paused", 100, 1, 8);
    let mut line = fixture_host_line(fixture_takeover_message(), Some(42));
    line.message = line
        .message
        .replace("live_native_owner=false", "live_native_owner=true");
    click.live_native_owner = Some(true);
    assert!(paused_frame_oracle(&line, &before, &click).is_err());
}

#[test]
fn paused_oracle_rejects_any_route_fact_false_positive() {
    let before = fixture_frame(1, 42, "Running", 100, 0, 7);
    let click = fixture_frame(2, 42, "Paused", 100, 1, 8);
    let mut line = fixture_host_line(fixture_takeover_message(), Some(42));
    line.message = line.message.replace("route=false", "route=true");
    assert!(paused_frame_oracle(&line, &before, &click).is_err());
}

#[test]
fn paused_oracle_rejects_wrong_input_and_nonmatching_frame_tick() {
    let before = fixture_frame(1, 42, "Running", 100, 0, 7);
    let mut click = fixture_frame(2, 42, "Paused", 100, 1, 8);
    let mut line = fixture_host_line(fixture_takeover_message(), Some(42));
    line.message = line
        .message
        .replace("source=Some(Viewport)", "source=Some(Minimap)");
    assert!(paused_frame_oracle(&line, &before, &click).is_err());
    line.message = fixture_takeover_message();
    click.host_tick = Some(43);
    assert!(paused_frame_oracle(&line, &before, &click).is_err());
}

#[test]
fn pause_off_oracle_rejects_takeover_and_walk_outcome_change() {
    let before = fixture_frame(1, 42, "Running", 100, 0, 7);
    let mut click = fixture_frame(2, 42, "Running", 100, 1, 7);
    click.manual_move_intent = Some("Viewport".to_owned());
    click.manual_steps = 0;
    let after = fixture_frame(3, 43, "Running", 110, 1, 7);
    let lines = [fixture_host_line(fixture_takeover_message(), Some(42))];
    assert!(pause_off_oracle(&lines, &before, &click, &after).is_err());
    assert!(pause_off_oracle(&[], &before, &click, &after).is_ok());
    click.nav.as_mut().unwrap().walk_outcome_seq = 8;
    assert!(pause_off_oracle(&[], &before, &click, &after).is_err());
}
