//! Real Engine L cells for the compat v1 Bot, Events, and Input contract.
//!
//! Each ignored test uses one fresh generated account and writes a receipt to
//! LIVE_EVIDENCE_DIR. Run each cell three times on one product commit.

#![cfg(feature = "live-harness")]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use api::interact;
use api::snapshot::GameSnapshot;
use client::client::Client;
use host_play::{
    mint_live_entries, mint_live_names, run_with_template, Play, ProfileOptions,
    SharedClientTemplate,
};
use script_js::LoadShape;
use serde::Serialize;
use serde_json::{Map, Value};
use vault::{Profile, ProfileSettings};

const GAME_PORT: u16 = 47594;
const CATALOG_SHORT: &str = "ae56ccc95";
const HTTP_PORT: u16 = 3080;
const POLL_INTERVAL: Duration = Duration::from_millis(20);
const PREPARATION_DEADLINE: Duration = Duration::from_secs(240);
const ACTION_DEADLINE: Duration = Duration::from_secs(60);
const STOP_DEADLINE: Duration = Duration::from_secs(20);

#[derive(Default, Serialize)]
struct Receipt {
    cell: String,
    verdict: String,
    error: Option<String>,
    product_commit: Option<String>,
    tested_tree_commit: Option<String>,
    account: Option<String>,
    catalog_root: Option<String>,
    catalog_commit: Option<String>,
    ports: Option<(u16, u16)>,
    commands: Vec<String>,
    logs: Vec<String>,
}

struct LiveSession<'a> {
    _isolated: script::IsolatedEnv,
    account: String,
    play: Play,
    state: Arc<Mutex<FrameState>>,
    receipt: &'a mut Receipt,
}
struct PreparedCatalogTask {
    js: String,
    shape: LoadShape,
    settings: Map<String, Value>,
    siblings: Vec<(String, String)>,
}

struct FrameState {
    snapshot: GameSnapshot,
    ready: bool,
    tick: u32,
    next_command: Option<String>,
    commands: Vec<String>,
    error: Option<String>,
}

impl FrameState {
    fn new() -> Self {
        Self {
            snapshot: GameSnapshot::new(),
            ready: false,
            tick: 0,
            next_command: None,
            commands: Vec::new(),
            error: None,
        }
    }

    fn frame(&mut self, client: &mut Client, held: bool) {
        self.snapshot.rebuild(client);
        if !client.ingame || client.scene_state != 2 {
            return;
        }
        self.ready = true;
        self.tick = self.snapshot.tick();
        if held {
            return;
        }
        let Some(command) = self.next_command.take() else {
            return;
        };
        if interact::cheat(client, &command).is_sent() {
            self.commands.push(command);
        } else {
            self.error = Some(format!("Engine L refused cheat command: {command}"));
        }
    }
}

fn required(name: &str) -> Result<String, String> {
    std::env::var(name).map_err(|error| format!("{name}: {error}"))
}

fn required_path(name: &str) -> Result<PathBuf, String> {
    let path = PathBuf::from(required(name)?);
    if !path.is_absolute() {
        return Err(format!("{name} must be an absolute path"));
    }
    Ok(path)
}

fn git_output(root: &Path, args: &[&str]) -> Result<String, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .map_err(|error| format!("git -C {} {}: {error}", root.display(), args.join(" ")))?;
    if !output.status.success() {
        return Err(format!(
            "git -C {} {} failed: {}",
            root.display(),
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    String::from_utf8(output.stdout)
        .map(|value| value.trim().to_owned())
        .map_err(|error| format!("git output is not UTF-8: {error}"))
}

fn verify_product_commit() -> Result<(String, String), String> {
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let product = required("COMPAT_V1_PRODUCT_COMMIT")?;
    let tested_tree = git_output(&repo, &["rev-parse", "HEAD"])?;
    let product_full = git_output(
        &repo,
        &["rev-parse", "--verify", &format!("{product}^{{commit}}")],
    )?;
    let ancestor = Command::new("git")
        .arg("-C")
        .arg(&repo)
        .args(["merge-base", "--is-ancestor", &product_full, &tested_tree])
        .status()
        .map_err(|error| format!("check product commit ancestry: {error}"))?;
    if !ancestor.success() {
        return Err(format!(
            "product commit {product_full} is not an ancestor of tested tree {tested_tree}"
        ));
    }
    Ok((product_full, tested_tree))
}

fn validate_inputs(
    nav_pack: &Path,
    engine_dir: &Path,
    catalog_root: &Path,
    cache_dir: &Path,
    evidence_dir: &Path,
    isolated_home: &Path,
) -> Result<(), String> {
    for (name, path, file) in [
        ("WORLD_NAV_PACK", nav_pack, true),
        ("WORLD_ENGINE_DIR", engine_dir, false),
        ("RS2B0T", catalog_root, false),
        ("BOT_CACHE_DIR", cache_dir, false),
    ] {
        if file && !path.is_file() {
            return Err(format!("{name} is not a file: {}", path.display()));
        }
        if !file && !path.is_dir() {
            return Err(format!("{name} is not a directory: {}", path.display()));
        }
    }
    if evidence_dir.starts_with(isolated_home) {
        return Err("LIVE_EVIDENCE_DIR must be outside the throwaway HOME".into());
    }
    std::fs::create_dir_all(evidence_dir).map_err(|error| {
        format!(
            "create evidence directory {}: {error}",
            evidence_dir.display()
        )
    })
}

fn selected_template(
    nav_pack: PathBuf,
    engine_dir: PathBuf,
    catalog_root: PathBuf,
    cache_dir: PathBuf,
    temp: &Path,
) -> Result<Arc<SharedClientTemplate>, String> {
    let profile = ProfileOptions {
        profile: Some("local-289".into()),
        revision: Some("289".into()),
        host: Some("127.0.0.1".into()),
        asset_host: Some("127.0.0.1".into()),
        port: Some(GAME_PORT),
        http_port: Some(HTTP_PORT),
        cache_dir: Some(cache_dir),
        nav_pack: Some(nav_pack),
        engine_dir: Some(engine_dir),
        unpack_dir: Some(temp.join("unpack")),
        vault_path: Some(temp.join("vault")),
        catalog_root: Some(catalog_root),
        ..ProfileOptions::default()
    }
    .resolve(None)?
    .bind_runtime()?;
    if profile.profile_class() != host_play::ProfileClass::Local
        || profile.client().game_host() != "127.0.0.1"
        || profile.client().game_port() != GAME_PORT
        || profile.client().asset_port() != HTTP_PORT
    {
        return Err("compat v1 live proof requires Engine L on the exact loopback ports".into());
    }
    let template = SharedClientTemplate::load(Arc::clone(&profile))?;
    if template.world().is_none() {
        return Err("selected Engine L template has no navigation world".into());
    }
    Ok(template)
}

fn session<'a>(cell: &str, receipt: &'a mut Receipt) -> Result<LiveSession<'a>, String> {
    if std::env::var("LIVE").as_deref() != Ok("1") {
        return Err("LIVE=1 is required; ignored live tests must not pass vacuously".into());
    }
    if std::env::var("BOT_CPU").as_deref() != Ok("1") {
        return Err("BOT_CPU=1 is required for headless live capture".into());
    }
    if std::env::var("BOT_NAV_BUILD").as_deref() != Ok("skip") {
        return Err("BOT_NAV_BUILD=skip and the explicit v17 WORLD_NAV_PACK are required".into());
    }
    let name_prefix = required("BOT_LIVE_NAME_PREFIX")?;
    if name_prefix.is_empty() || name_prefix.len() > 4 {
        return Err("BOT_LIVE_NAME_PREFIX must contain one to four characters".into());
    }
    let (product_commit, tested_tree_commit) = verify_product_commit()?;
    receipt.product_commit = Some(product_commit);
    receipt.tested_tree_commit = Some(tested_tree_commit);
    let nav_pack = required_path("WORLD_NAV_PACK")?;
    let engine_dir = required_path("WORLD_ENGINE_DIR")?;
    let catalog_root = required_path("RS2B0T")?;
    let catalog_commit = git_output(&catalog_root, &["rev-parse", "--short=10", "HEAD"])?;
    if !catalog_commit.starts_with(CATALOG_SHORT) {
        return Err(format!(
            "expected pinned rs2b0t catalog {CATALOG_SHORT}, found {catalog_commit}"
        ));
    }
    receipt.catalog_root = Some(catalog_root.display().to_string());
    receipt.catalog_commit = Some(catalog_commit);
    let cache_dir = required_path("BOT_CACHE_DIR")?;
    let evidence_dir = required_path("LIVE_EVIDENCE_DIR")?;
    receipt.ports = Some((GAME_PORT, HTTP_PORT));
    let isolated = script::IsolatedEnv::enter(cell);
    validate_inputs(
        &nav_pack,
        &engine_dir,
        &catalog_root,
        &cache_dir,
        &evidence_dir,
        &isolated.home,
    )?;
    isolated.set_rs2b0t(&catalog_root);
    api::hostlog::set_debug(true);

    let names = mint_live_names(1);
    if names.len() != 1 {
        return Err("could not mint one live account name".into());
    }
    let entries = mint_live_entries(&names);
    if entries.len() != 1 {
        return Err("could not mint one live account credential".into());
    }
    let (account, password) = &entries[0];
    receipt.account = Some(account.clone());
    let temp = isolated.home.join("compat-v1-live").join(account);
    std::fs::create_dir_all(&temp)
        .map_err(|error| format!("create per-account cache {}: {error}", temp.display()))?;
    let template = selected_template(nav_pack, engine_dir, catalog_root.clone(), cache_dir, &temp)?;
    let state = Arc::new(Mutex::new(FrameState::new()));
    let callback_state = Arc::clone(&state);
    let callback_account = account.clone();
    let play = run_with_template(
        template,
        false,
        vec![],
        |_| (None, None),
        move |client, username, frame| {
            if username == callback_account {
                callback_state
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .frame(client, frame.hold);
            }
        },
    )?;
    let account_profile = Profile {
        username: account.clone(),
        password: password.clone().into(),
        uid: (SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| format!("clock: {error}"))?
            .as_millis()
            % i32::MAX as u128) as i32,
        settings: ProfileSettings::default(),
    };
    let mut play = play;
    play.try_spawn_slot(account_profile, None, None, None)?;
    play.focus(account);

    wait_ready(&state)?;
    Ok(LiveSession {
        _isolated: isolated,
        account: account.clone(),
        play,
        state,
        receipt,
    })
}

fn lock_state(state: &Arc<Mutex<FrameState>>) -> std::sync::MutexGuard<'_, FrameState> {
    state.lock().unwrap_or_else(|error| error.into_inner())
}

fn wait_ready(state: &Arc<Mutex<FrameState>>) -> Result<(), String> {
    let deadline = Instant::now() + PREPARATION_DEADLINE;
    loop {
        let frame = lock_state(state);
        if let Some(error) = &frame.error {
            return Err(error.clone());
        }
        if frame.ready {
            return Ok(());
        }
        drop(frame);
        if Instant::now() >= deadline {
            return Err("timed out waiting for the generated account to enter the game".into());
        }
        thread::sleep(POLL_INTERVAL);
    }
}

fn request_command(session: &LiveSession<'_>, command: &str) -> Result<(), String> {
    let mut frame = lock_state(&session.state);
    if let Some(error) = &frame.error {
        return Err(error.clone());
    }
    if frame.next_command.is_some() {
        return Err("a live cheat command is already pending".into());
    }
    frame.next_command = Some(command.to_owned());
    Ok(())
}

fn wait_command(session: &mut LiveSession<'_>, command: &str) -> Result<(), String> {
    let deadline = Instant::now() + ACTION_DEADLINE;
    loop {
        let frame = lock_state(&session.state);
        if let Some(error) = &frame.error {
            return Err(error.clone());
        }
        if frame.commands.iter().any(|sent| sent == command) {
            session.receipt.commands.push(command.to_owned());
            return Ok(());
        }
        drop(frame);
        if Instant::now() >= deadline {
            return Err(format!("timed out sending live cheat command: {command}"));
        }
        thread::sleep(POLL_INTERVAL);
    }
}

fn drain_logs(session: &mut LiveSession<'_>) {
    session
        .receipt
        .logs
        .extend(session.play.script_take_pending_logs(&session.account));
}

fn wait_for_logs(
    session: &mut LiveSession<'_>,
    timeout: Duration,
    predicate: impl Fn(&[String]) -> bool,
) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    loop {
        drain_logs(session);
        if predicate(&session.receipt.logs) {
            return Ok(());
        }
        if let Some(error) = session.play.script_last_error(&session.account) {
            return Err(format!("compat v1 script failed: {error}"));
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "timed out waiting for script logs; observed: {:?}",
                session.receipt.logs
            ));
        }
        thread::sleep(POLL_INTERVAL);
    }
}

fn start_script(session: &LiveSession<'_>, source: String) -> Result<(), String> {
    session.play.script_start_load(
        &session.account,
        source,
        LoadShape::CompatClass,
        None,
        vec![],
    )
}

fn wait_idle(session: &mut LiveSession<'_>) -> Result<(), String> {
    let deadline = Instant::now() + STOP_DEADLINE;
    loop {
        drain_logs(session);
        if session.play.script_state(&session.account) == script::RunState::Idle {
            return Ok(());
        }
        if let Some(error) = session.play.script_last_error(&session.account) {
            return Err(format!("compat v1 cancellation failed: {error}"));
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "timed out waiting for Idle; logs: {:?}",
                session.receipt.logs
            ));
        }
        thread::sleep(POLL_INTERVAL);
    }
}

fn wait_stopped(session: &mut LiveSession<'_>, marker: &str) -> Result<(), String> {
    let deadline = Instant::now() + STOP_DEADLINE;
    loop {
        drain_logs(session);
        if session.play.script_state(&session.account) == script::RunState::Idle
            && session.receipt.logs.iter().any(|line| line == marker)
        {
            return Ok(());
        }
        if let Some(error) = session.play.script_last_error(&session.account) {
            return Err(format!("compat v1 stop failed: {error}"));
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "timed out waiting for {marker:?} and Idle; logs: {:?}",
                session.receipt.logs
            ));
        }
        thread::sleep(POLL_INTERVAL);
    }
}

fn stop_script(session: &mut LiveSession<'_>, marker: &str, twice: bool) -> Result<(), String> {
    session.play.script_stop(&session.account);
    if twice {
        session.play.script_stop(&session.account);
    }
    wait_stopped(session, marker)
}

fn run_and_record(
    cell: &str,
    run: impl FnOnce(&mut Receipt) -> Result<(), String>,
) -> Result<(), String> {
    let evidence_dir = required_path("LIVE_EVIDENCE_DIR")?;
    std::fs::create_dir_all(&evidence_dir).map_err(|error| {
        format!(
            "create evidence directory {}: {error}",
            evidence_dir.display()
        )
    })?;
    let mut receipt = Receipt {
        cell: cell.to_owned(),
        ..Receipt::default()
    };
    let result = run(&mut receipt);
    receipt.verdict = if result.is_ok() { "PASS" } else { "FAIL" }.into();
    if let Err(error) = &result {
        receipt.error = Some(error.clone());
    }
    let path = evidence_dir.join(format!("{cell}.json"));
    let bytes = serde_json::to_vec_pretty(&receipt)
        .map_err(|error| format!("encode live receipt: {error}"))?;
    std::fs::write(&path, bytes)
        .map_err(|error| format!("write live receipt {}: {error}", path.display()))?;
    result.map_err(|error| format!("{error}; receipt: {}", path.display()))
}

fn count_prefix(lines: &[String], prefix: &str) -> usize {
    lines.iter().filter(|line| line.starts_with(prefix)).count()
}

fn event_payloads(lines: &[String], prefix: &str) -> Result<Vec<serde_json::Value>, String> {
    lines
        .iter()
        .filter_map(|line| line.strip_prefix(prefix))
        .map(|payload| {
            serde_json::from_str(payload)
                .map_err(|error| format!("invalid payload for {prefix}: {error}"))
        })
        .collect()
}

fn tree_bot_source() -> String {
    r#"
import { TreeBot, BranchTask, LeafTask } from '@rs2b0t/api';
const trace = globalThis.__compat_v1_tree_trace = [];
let completed = false;
class Root extends BranchTask {
    validate() { trace.push('validate'); return true; }
    success() { trace.push('success'); return new Leaf(); }
    failure() { throw new Error('unexpected TreeBot failure'); }
}
class Leaf extends LeafTask {
    async execute() {
        trace.push('leaf.start');
        await Promise.resolve();
        trace.push('leaf.end');
    }
}
export default class CompatTreeProbe extends TreeBot {
    onStart() { this.log('TREE:READY'); }
    root() { trace.push('root'); return new Root(); }
    async loop() {
        if (completed) return;
        await super.loop();
        completed = true;
        this.log('TREE:DONE:' + trace.join(','));
    }
    onStop() { this.log('TREE:STOP'); }
}
"#
    .to_owned()
}

fn events_source(generation: u8) -> String {
    let source = r#"
import { events } from '@rs2b0t/api';
export default class CompatEventsProbe extends LoopingBot {
    onStart() {
        this.log('EVENTS:READY:GENERATION');
        if (GENERATION === 1) {
            let removeFirst;
            const first = event => {
                if (!event.text?.startsWith('get tutorial:')) return;
                this.log('CHAT:FIRST:' + JSON.stringify({ type: event.type, text: event.text }));
                removeFirst();
            };
            const second = event => {
                if (event.text?.startsWith('get tutorial:')) {
                    this.log('CHAT:SECOND:' + JSON.stringify({ type: event.type, text: event.text }));
                }
            };
            const instance = event => {
                if (event.text?.startsWith('get tutorial:')) {
                    this.log('CHAT:INSTANCE:' + JSON.stringify({ type: event.type, text: event.text }));
                }
            };
            removeFirst = events.on('chat.message', first);
            events.on('chat.message', first);
            events.on('chat.message', second);
            this.on('chat.message', second);
            this.on('chat.message', instance);
            const removeInventory = event => {
                if (event.id !== 995) return;
                const result = events.off('inventory.changed', removeInventory);
                this.log('INV:OFF:' + JSON.stringify({ result: result === undefined, id: event.id }));
            };
            const inventoryWitness = event => {
                if (event.id === 995) this.log('INV:WITNESS:' + JSON.stringify(event));
            };
            events.on('inventory.changed', removeInventory);
            events.on('inventory.changed', inventoryWitness);
            events.on('skill.xp', event => {
                if (event.name === 'prayer') this.log('XP:PRAYER:' + JSON.stringify(event));
            });
        } else {
            const restarted = event => {
                if (event.text?.startsWith('get tutorial:')) {
                    this.log('RESTART:CHAT:' + JSON.stringify({ type: event.type, text: event.text }));
                }
            };
            const instance = event => {
                if (event.text?.startsWith('get tutorial:')) {
                    this.log('RESTART:INSTANCE:' + JSON.stringify({ type: event.type, text: event.text }));
                }
            };
            events.on('chat.message', restarted);
            this.on('chat.message', instance);
        }
    }
    loop() {}
    onStop() { this.log('EVENTS:STOP:GENERATION'); }
}
"#;
    source.replace("GENERATION", &generation.to_string())
}

fn prepare_catalog_task(root: &Path, temp: &Path) -> Result<PreparedCatalogTask, String> {
    let catalog_commit = git_output(root, &["rev-parse", "--short=10", "HEAD"])?;
    if !catalog_commit.starts_with(CATALOG_SHORT) {
        return Err(format!(
            "expected rs2b0t catalog {CATALOG_SHORT}, found {catalog_commit}"
        ));
    }
    let card_name = "ClueSolver";
    let source_path = root.join("src/bot/scripts/ClueSolver/ClueSolver.ts");
    if !source_path.is_file() {
        return Err(format!(
            "catalog source is missing: {}",
            source_path.display()
        ));
    }
    let source_sha = script_js::JsCache::origin_sha(
        &std::fs::read(&source_path)
            .map_err(|error| format!("read {}: {error}", source_path.display()))?,
    );
    let mut library =
        script_js::JsLibrary::with_cache(temp.join("js-scripts.json"), temp.join("js-cache"));
    if library.register_rs2b0t(root, &temp.join("rs2b0t-path"))? == 0 {
        return Err("pinned catalog registered no scripts".into());
    }
    library.ensure_js(script::ScriptSource::Catalog, card_name)?;
    let card = library
        .get(script::ScriptSource::Catalog, card_name)
        .cloned()
        .ok_or_else(|| format!("pinned catalog has no {card_name} card"))?;
    let expected_source = source_path
        .canonicalize()
        .map_err(|error| format!("canonicalize catalog source: {error}"))?;
    let loaded_source = card
        .path
        .canonicalize()
        .map_err(|error| format!("canonicalize loaded catalog source: {error}"))?;
    if loaded_source != expected_source || card.sha256 != source_sha {
        return Err(format!(
            "catalog identity mismatch for {card_name}: source={}, loaded={}, expected_sha={}, loaded_sha={}",
            expected_source.display(),
            loaded_source.display(),
            source_sha,
            card.sha256
        ));
    }
    if let Some(import) = &card.unloadable {
        return Err(format!("{card_name} is unloadable: {import}"));
    }
    if card.shape != LoadShape::CompatClass {
        return Err(format!(
            "{card_name} has unexpected Load shape {:?}",
            card.shape
        ));
    }
    let explicit = Map::new();
    let settings = script::merge_bag(&card.settings_schema, &Map::new(), Some(&explicit));
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
    Ok(PreparedCatalogTask {
        js: card.js,
        shape: card.shape,
        settings,
        siblings,
    })
}

fn run_tree_cell(receipt: &mut Receipt) -> Result<(), String> {
    let mut session = session("compat-v1-bot-tree", receipt)?;
    let catalog_root = required_path("RS2B0T")?;
    let temp = session
        ._isolated
        .home
        .join("compat-v1-live")
        .join(&session.account);
    let card = prepare_catalog_task(&catalog_root, &temp)?;
    session.play.script_start_handle().start_load(
        &session.account,
        card.js,
        card.shape,
        Some(card.settings),
        card.siblings,
    )?;
    wait_for_logs(&mut session, ACTION_DEADLINE, |logs| {
        logs.iter().any(|line| {
            line.starts_with("ClueSolver — watching the pack for easy clue scrolls/caskets")
        })
    })?;
    session.play.script_stop(&session.account);
    session.play.script_stop(&session.account);
    wait_idle(&mut session)?;
    if count_prefix(
        &session.receipt.logs,
        "ClueSolver — watching the pack for easy clue scrolls/caskets",
    ) != 1
    {
        return Err(format!(
            "catalog TaskBot did not construct exactly once: {:?}",
            session.receipt.logs
        ));
    }

    start_script(&session, tree_bot_source())?;
    wait_for_logs(&mut session, ACTION_DEADLINE, |logs| {
        logs.iter().any(|line| line == "TREE:READY")
            && logs
                .iter()
                .any(|line| line == "TREE:DONE:root,validate,success,leaf.start,leaf.end")
    })?;
    stop_script(&mut session, "TREE:STOP", true)?;
    if count_prefix(&session.receipt.logs, "TREE:STOP") != 1 {
        return Err(format!(
            "TreeBot stop was not exactly once: {:?}",
            session.receipt.logs
        ));
    }
    Ok(())
}

fn wait_for_next_tick_without_restart_event(session: &mut LiveSession<'_>) -> Result<(), String> {
    let before = lock_state(&session.state).tick;
    let deadline = Instant::now() + ACTION_DEADLINE;
    loop {
        drain_logs(session);
        if session
            .receipt
            .logs
            .iter()
            .any(|line| line.starts_with("RESTART:CHAT:") || line.starts_with("RESTART:INSTANCE:"))
        {
            return Err("new Events generation received a pre-restart chat event".into());
        }
        if let Some(error) = session.play.script_last_error(&session.account) {
            return Err(format!("compat v1 event script failed: {error}"));
        }
        if lock_state(&session.state).tick > before {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err("timed out waiting for one post-restart Engine L tick".into());
        }
        thread::sleep(POLL_INTERVAL);
    }
}

fn run_events_cell(receipt: &mut Receipt) -> Result<(), String> {
    let mut session = session("compat-v1-events", receipt)?;
    start_script(&session, events_source(1))?;
    wait_for_logs(&mut session, ACTION_DEADLINE, |logs| {
        logs.iter().any(|line| line == "EVENTS:READY:1")
    })?;

    request_command(&session, "getvar tutorial")?;
    wait_command(&mut session, "getvar tutorial")?;
    wait_for_logs(&mut session, ACTION_DEADLINE, |logs| {
        count_prefix(logs, "CHAT:FIRST:") == 1
            && count_prefix(logs, "CHAT:SECOND:") == 1
            && count_prefix(logs, "CHAT:INSTANCE:") == 1
    })?;

    request_command(&session, "getvar tutorial")?;
    wait_command(&mut session, "getvar tutorial")?;
    wait_for_logs(&mut session, ACTION_DEADLINE, |logs| {
        count_prefix(logs, "CHAT:FIRST:") == 1
            && count_prefix(logs, "CHAT:SECOND:") == 2
            && count_prefix(logs, "CHAT:INSTANCE:") == 2
    })?;

    request_command(&session, "give coins 1")?;
    wait_command(&mut session, "give coins 1")?;
    wait_for_logs(&mut session, ACTION_DEADLINE, |logs| {
        count_prefix(logs, "INV:OFF:") == 1 && count_prefix(logs, "INV:WITNESS:") == 1
    })?;

    request_command(&session, "give coins 2")?;
    wait_command(&mut session, "give coins 2")?;
    wait_for_logs(&mut session, ACTION_DEADLINE, |logs| {
        count_prefix(logs, "INV:OFF:") == 1 && count_prefix(logs, "INV:WITNESS:") == 2
    })?;

    request_command(&session, "setstat prayer 2")?;
    wait_command(&mut session, "setstat prayer 2")?;
    wait_for_logs(&mut session, ACTION_DEADLINE, |logs| {
        count_prefix(logs, "XP:PRAYER:") >= 1
    })?;

    stop_script(&mut session, "EVENTS:STOP:1", false)?;
    start_script(&session, events_source(2))?;
    wait_for_logs(&mut session, ACTION_DEADLINE, |logs| {
        logs.iter().any(|line| line == "EVENTS:READY:2")
    })?;
    wait_for_next_tick_without_restart_event(&mut session)?;

    request_command(&session, "getvar tutorial")?;
    wait_command(&mut session, "getvar tutorial")?;
    wait_for_logs(&mut session, ACTION_DEADLINE, |logs| {
        count_prefix(logs, "RESTART:CHAT:") == 1 && count_prefix(logs, "RESTART:INSTANCE:") == 1
    })?;
    stop_script(&mut session, "EVENTS:STOP:2", false)?;

    if count_prefix(&session.receipt.logs, "CHAT:FIRST:") != 1
        || count_prefix(&session.receipt.logs, "CHAT:SECOND:") != 2
        || count_prefix(&session.receipt.logs, "CHAT:INSTANCE:") != 2
        || count_prefix(&session.receipt.logs, "INV:OFF:") != 1
        || count_prefix(&session.receipt.logs, "INV:WITNESS:") != 2
        || count_prefix(&session.receipt.logs, "XP:PRAYER:") != 1
        || count_prefix(&session.receipt.logs, "RESTART:CHAT:") != 1
        || count_prefix(&session.receipt.logs, "RESTART:INSTANCE:") != 1
        || count_prefix(&session.receipt.logs, "EVENTS:STOP:1") != 1
        || count_prefix(&session.receipt.logs, "EVENTS:STOP:2") != 1
    {
        return Err(format!(
            "native Events contract counts differ: {:?}",
            session.receipt.logs
        ));
    }
    let logs = &session.receipt.logs;
    let first_chat = event_payloads(logs, "CHAT:FIRST:")?;
    let second_chat = event_payloads(logs, "CHAT:SECOND:")?;
    let instance_chat = event_payloads(logs, "CHAT:INSTANCE:")?;
    let inventory_off = event_payloads(logs, "INV:OFF:")?;
    let inventory = event_payloads(logs, "INV:WITNESS:")?;
    let prayer = event_payloads(logs, "XP:PRAYER:")?;
    let restarted_chat = event_payloads(logs, "RESTART:CHAT:")?;
    let restarted_instance = event_payloads(logs, "RESTART:INSTANCE:")?;
    if !first_chat[0]["text"]
        .as_str()
        .is_some_and(|text| text.starts_with("get tutorial:"))
        || second_chat.iter().any(|payload| {
            !payload["text"]
                .as_str()
                .is_some_and(|text| text.starts_with("get tutorial:"))
        })
        || instance_chat.iter().any(|payload| {
            !payload["text"]
                .as_str()
                .is_some_and(|text| text.starts_with("get tutorial:"))
        })
        || inventory_off[0]["result"] != true
        || inventory_off[0]["id"] != 995
        || inventory.iter().any(|payload| {
            payload["id"] != 995 || payload["count"].as_i64().unwrap_or_default() <= 0
        })
        || prayer[0]["name"] != "prayer"
        || prayer[0]["xp"].as_i64().unwrap_or_default() <= 0
        || prayer[0]["delta"].as_i64().unwrap_or_default() <= 0
        || !restarted_chat[0]["text"]
            .as_str()
            .is_some_and(|text| text.starts_with("get tutorial:"))
        || !restarted_instance[0]["text"]
            .as_str()
            .is_some_and(|text| text.starts_with("get tutorial:"))
    {
        return Err(format!(
            "native event payloads changed: {:?}",
            session.receipt.logs
        ));
    }

    let first_chat_at = logs
        .iter()
        .position(|line| line.starts_with("CHAT:FIRST:"))
        .unwrap();
    let second_chat_at: Vec<_> = logs
        .iter()
        .enumerate()
        .filter_map(|(index, line)| line.starts_with("CHAT:SECOND:").then_some(index))
        .collect();
    let instance_chat_at: Vec<_> = logs
        .iter()
        .enumerate()
        .filter_map(|(index, line)| line.starts_with("CHAT:INSTANCE:").then_some(index))
        .collect();
    let inventory_off_at = logs
        .iter()
        .position(|line| line.starts_with("INV:OFF:"))
        .unwrap();
    let inventory_at: Vec<_> = logs
        .iter()
        .enumerate()
        .filter_map(|(index, line)| line.starts_with("INV:WITNESS:").then_some(index))
        .collect();
    let stop_first_at = logs
        .iter()
        .position(|line| line == "EVENTS:STOP:1")
        .unwrap();
    let ready_second_at = logs
        .iter()
        .position(|line| line == "EVENTS:READY:2")
        .unwrap();
    let restarted_chat_at = logs
        .iter()
        .position(|line| line.starts_with("RESTART:CHAT:"))
        .unwrap();
    let restarted_instance_at = logs
        .iter()
        .position(|line| line.starts_with("RESTART:INSTANCE:"))
        .unwrap();
    let stop_second_at = logs
        .iter()
        .position(|line| line == "EVENTS:STOP:2")
        .unwrap();
    if !(first_chat_at < second_chat_at[0]
        && second_chat_at[0] < instance_chat_at[0]
        && instance_chat_at[0] < second_chat_at[1]
        && second_chat_at[1] < instance_chat_at[1]
        && inventory_off_at < inventory_at[0]
        && inventory_at[0] < inventory_at[1]
        && stop_first_at < ready_second_at
        && ready_second_at < restarted_chat_at
        && restarted_chat_at < restarted_instance_at
        && restarted_instance_at < stop_second_at)
    {
        return Err(format!("native event callback order changed: {logs:?}"));
    }

    Ok(())
}

#[test]
#[ignore = "requires LIVE=1, throwaway HOME, pinned catalog, and real 289 Engine L"]
fn compat_v1_bot_tree_live() {
    run_and_record("compat_v1_bot_tree", run_tree_cell).unwrap_or_else(|error| panic!("{error}"));
}

#[test]
#[ignore = "requires LIVE=1, throwaway HOME, pinned catalog, and real 289 Engine L"]
fn compat_v1_events_live() {
    run_and_record("compat_v1_events", run_events_cell).unwrap_or_else(|error| panic!("{error}"));
}

const RUNTIME_FRAME_SOURCE: &str = r#"
import { LoopingBot } from '@rs2b0t/api';
import { BotHost } from '../../runtime/BotHost.js';
import { ScriptRunner } from '../../runtime/ScriptRunner.js';

export default class CompatRuntimeFrameProbe extends LoopingBot {
    frameCount = 0;
    loopCount = 0;
    previousFrameCount = 0;

    onStart() {
        BotHost.addFrameListener(() => {
            this.frameCount += 1;
            this.log('RUNTIME:FRAME');
        });
        this.log('RUNTIME:READY');
    }

    async loop() {
        this.loopCount += 1;
        if (this.loopCount === 1) {
            this.previousFrameCount = this.frameCount;
            return;
        }
        const betweenTicks = this.frameCount - this.previousFrameCount;
        if (betweenTicks > 1) {
            this.log(`RUNTIME:FRAME_GAP:${betweenTicks}`);
            ScriptRunner.stop('runtime frame proof complete');
            return;
        }
        this.previousFrameCount = this.frameCount;
        if (this.loopCount >= 5) {
            throw new Error(`expected client frames between ticks; saw ${betweenTicks}`);
        }
    }

    onStop() {
        this.log('RUNTIME:STOP');
    }
}
"#;

fn wait_for_game_tick(session: &mut LiveSession<'_>, target: u32) -> Result<(), String> {
    let deadline = Instant::now() + ACTION_DEADLINE;
    loop {
        let tick = lock_state(&session.state).tick;
        if tick >= target {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "timed out waiting for game tick {target}; last tick {tick}"
            ));
        }
        thread::sleep(POLL_INTERVAL);
    }
}

fn run_runtime_frame_cell(receipt: &mut Receipt) -> Result<(), String> {
    let mut session = session("compat-v1-runtime-frame", receipt)?;
    start_script(&session, RUNTIME_FRAME_SOURCE.to_owned())?;
    wait_for_logs(&mut session, ACTION_DEADLINE, |logs| {
        logs.iter().any(|line| line == "RUNTIME:READY")
            && logs
                .iter()
                .any(|line| line.starts_with("RUNTIME:FRAME_GAP:"))
            && logs.iter().any(|line| line == "RUNTIME:STOP")
    })?;
    wait_idle(&mut session)?;

    let stop_index = session
        .receipt
        .logs
        .iter()
        .position(|line| line == "RUNTIME:STOP")
        .ok_or_else(|| "runtime frame probe did not record its stop".to_string())?;
    let gap = session.receipt.logs[..stop_index]
        .iter()
        .find_map(|line| {
            line.strip_prefix("RUNTIME:FRAME_GAP:")
                .and_then(|value| value.parse::<usize>().ok())
        })
        .ok_or_else(|| "runtime frame probe did not record a frame gap".to_string())?;
    if gap <= 1 {
        return Err(format!(
            "expected multiple client frames between ticks, saw {gap}"
        ));
    }
    if !session.receipt.logs[..stop_index]
        .iter()
        .any(|line| line == "RUNTIME:FRAME")
    {
        return Err("no live client-frame callback was observed before stop".into());
    }

    let stopped_tick = lock_state(&session.state).tick;
    wait_for_game_tick(&mut session, stopped_tick.saturating_add(2))?;
    drain_logs(&mut session);
    session.receipt.logs[stop_index + 1..]
        .iter()
        .all(|line| line != "RUNTIME:FRAME")
        .then_some(())
        .ok_or_else(|| "frame callback ran after the script stopped".to_string())
}

#[test]
#[ignore = "requires LIVE=1, throwaway HOME, pinned catalog, and real 289 Engine L"]
fn compat_v1_runtime_frame_live() {
    run_and_record("compat_v1_runtime_frame", run_runtime_frame_cell)
        .unwrap_or_else(|error| panic!("{error}"));
}
