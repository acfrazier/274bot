//! Real Engine L cells for the compat v1 Bot, Events, Input, and Runtime contracts.
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
const EVENTS_SEED_TILE: (i32, i32, i32) = (2663, 3302, 0);
const INVENTORY_SIDE_TAB: i32 = 3;
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
    coverage_notes: Vec<String>,
    probe_source: Option<String>,
}

struct LiveSession<'a> {
    _isolated: script::IsolatedEnv,
    account: String,
    play: Play,
    state: Arc<Mutex<FrameState>>,
    receipt: &'a mut Receipt,
}
struct PreparedScriptTask {
    js: String,
    shape: LoadShape,
    settings: Map<String, Value>,
    siblings: Vec<(String, String)>,
}

struct FrameState {
    snapshot: GameSnapshot,
    tick: u32,
    next_command: Option<String>,
    next_mainland_hop: bool,
    next_logout: bool,
    next_seed_at: Option<(i32, i32, i32)>,
    next_side_tab: Option<i32>,
    commands: Vec<String>,
    error: Option<String>,
}

impl FrameState {
    fn new() -> Self {
        Self {
            snapshot: GameSnapshot::new(),
            tick: 0,
            next_command: None,
            next_seed_at: None,
            next_mainland_hop: false,
            next_logout: false,
            next_side_tab: None,
            commands: Vec::new(),
            error: None,
        }
    }

    fn frame(&mut self, client: &mut Client, held: bool) {
        self.snapshot.rebuild(client);
        if held {
            return;
        }
        if !client.ingame || client.scene_state != 2 {
            return;
        }
        self.tick = self.snapshot.tick();
        if self.next_mainland_hop {
            self.next_mainland_hop = false;
            interact::mainland_hop(client);
            return;
        }
        if self.next_logout {
            self.next_logout = false;
            let ifaces = Arc::clone(&client.ifaces);
            if !interact::logout(client, &ifaces) {
                self.error = Some("Engine L refused the generated account logout".into());
            }
            return;
        }
        if let Some(tab) = self.next_side_tab.take() {
            let result = interact::Interactions::new(&self.snapshot, client).click_side_tab(tab);
            if let interact::SendResult::Refused { reason, .. } = result {
                self.error = Some(format!("Engine L refused side tab {tab}: {reason:?}"));
                return;
            }
        }
        if let Some((level, x, z)) = self.next_seed_at.take() {
            interact::seed_at(client, level, x, z);
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
        let ready = frame.snapshot.ingame()
            && frame.snapshot.attached()
            && frame.snapshot.scene_state() == 2;
        drop(frame);
        if ready {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err("timed out waiting for the account to enter an attached game scene".into());
        }
        thread::sleep(POLL_INTERVAL);
    }
}
fn request_seed_at(state: &Arc<Mutex<FrameState>>, level: i32, x: i32, z: i32) {
    lock_state(state).next_seed_at = Some((level, x, z));
}

fn wait_for_tile(state: &Arc<Mutex<FrameState>>, expected: (i32, i32, i32)) -> Result<(), String> {
    let deadline = Instant::now() + ACTION_DEADLINE;
    loop {
        let frame = lock_state(state);
        let ready = frame.snapshot.ingame()
            && frame.snapshot.attached()
            && frame.snapshot.scene_state() == 2;
        let tile = frame.snapshot.tile();
        drop(frame);
        if ready && tile == Some(expected) {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "timed out waiting for seed tile {expected:?}; snapshot ready={ready}, tile={tile:?}"
            ));
        }
        thread::sleep(POLL_INTERVAL);
    }
}
fn wait_for_side_tab_available(state: &Arc<Mutex<FrameState>>, index: i32) -> Result<(), String> {
    let deadline = Instant::now() + ACTION_DEADLINE;
    loop {
        let frame = lock_state(state);
        if let Some(error) = &frame.error {
            return Err(error.clone());
        }
        let available = frame
            .snapshot
            .side_tabs()
            .iter()
            .any(|tab| tab.index == index && tab.available);
        drop(frame);
        if available {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "timed out waiting for side tab {index} to become available"
            ));
        }
        thread::sleep(POLL_INTERVAL);
    }
}

fn request_side_tab(state: &Arc<Mutex<FrameState>>, tab: i32) -> Result<(), String> {
    let mut frame = lock_state(state);
    if let Some(error) = &frame.error {
        return Err(error.clone());
    }
    if frame.next_side_tab.is_some() {
        return Err("a live side-tab request is already pending".into());
    }
    frame.next_side_tab = Some(tab);
    Ok(())
}
fn request_mainland_hop(state: &Arc<Mutex<FrameState>>) -> Result<(), String> {
    let mut frame = lock_state(state);
    if let Some(error) = &frame.error {
        return Err(error.clone());
    }
    if frame.next_mainland_hop {
        return Err("a mainland tutorial hop is already pending".into());
    }
    frame.next_mainland_hop = true;
    Ok(())
}

fn request_logout(state: &Arc<Mutex<FrameState>>) -> Result<(), String> {
    let mut frame = lock_state(state);
    if let Some(error) = &frame.error {
        return Err(error.clone());
    }
    if !frame.snapshot.ingame() {
        return Err("the generated account is not in game before logout".into());
    }
    if frame.next_logout {
        return Err("a live logout request is already pending".into());
    }
    frame.next_logout = true;
    Ok(())
}

/// Log the generated account in again through the host. The host parks a bot with no script after
/// a logout nobody asked it for (the test's own client logout reads as a server logout), so an
/// explicit login intent is what lifts that park; a raw client login would leave the slot parked
/// and its frame hold closed.
fn request_login(session: &LiveSession<'_>) -> Result<(), String> {
    let frame = lock_state(&session.state);
    if let Some(error) = &frame.error {
        return Err(error.clone());
    }
    if frame.snapshot.ingame() {
        return Err("the generated account is still in game before login".into());
    }
    drop(frame);
    let arm = session
        .play
        .arm(&session.account)
        .ok_or("the generated account has no slot arm")?;
    arm.arm_explicit_login();
    Ok(())
}

fn wait_for_logged_out(state: &Arc<Mutex<FrameState>>) -> Result<(), String> {
    let deadline = Instant::now() + ACTION_DEADLINE;
    loop {
        let frame = lock_state(state);
        if let Some(error) = &frame.error {
            return Err(error.clone());
        }
        let ingame = frame.snapshot.ingame();
        drop(frame);
        if !ingame {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err("timed out waiting for the generated account to log out".into());
        }
        thread::sleep(POLL_INTERVAL);
    }
}

fn wait_for_chat_line(state: &Arc<Mutex<FrameState>>, suffix: &str) -> Result<String, String> {
    let deadline = Instant::now() + ACTION_DEADLINE;
    loop {
        let frame = lock_state(state);
        if let Some(error) = &frame.error {
            return Err(error.clone());
        }
        let found = frame
            .snapshot
            .chat_lines()
            .iter()
            .find(|line| line.text.trim_end().ends_with(suffix))
            .map(|line| line.text.clone());
        drop(frame);
        if let Some(line) = found {
            return Ok(line);
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "timed out waiting for chat line ending with {suffix:?}"
            ));
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
        let mut frame = lock_state(&session.state);
        if let Some(error) = &frame.error {
            return Err(error.clone());
        }
        if let Some(index) = frame.commands.iter().position(|sent| sent == command) {
            frame.commands.remove(index);
            drop(frame);
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
fn inventory_count(snapshot: &GameSnapshot, item_id: i32) -> i64 {
    snapshot
        .inventory()
        .iter()
        .filter(|item| item.def.id == item_id)
        .map(|item| i64::from(item.count))
        .sum()
}

fn wait_for_inventory_count(
    state: &Arc<Mutex<FrameState>>,
    item_id: i32,
    minimum: i64,
) -> Result<(i32, i64), String> {
    let deadline = Instant::now() + ACTION_DEADLINE;
    loop {
        let frame = lock_state(state);
        if let Some(error) = &frame.error {
            return Err(error.clone());
        }
        let inventory_size = frame.snapshot.inventory_size();
        let count = inventory_count(&frame.snapshot, item_id);
        drop(frame);
        if inventory_size > 0 && count >= minimum {
            return Ok((inventory_size, count));
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "timed out waiting for inventory item {item_id} count {minimum}; snapshot inventory_size={inventory_size}, count={count}"
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
                this.log('INV:RAW:' + JSON.stringify(event));
                if (event.id === 995) this.log('INV:WITNESS:' + JSON.stringify(event));
            };
            events.on('inventory.changed', removeInventory);
            events.on('inventory.changed', inventoryWitness);
            events.on('skill.xp', event => {
                if (event.name === 'prayer') this.log('XP:PRAYER:' + JSON.stringify(event));
            });
            events.on('skill.level', event => {
                if (event.name === 'prayer') this.log('LEVEL:PRAYER:' + JSON.stringify(event));
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

fn replace_once(source: &mut String, needle: &str, replacement: &str) -> Result<(), String> {
    let count = source.matches(needle).count();
    if count != 1 {
        return Err(format!(
            "expected one catalog source marker {needle:?}, found {count}"
        ));
    }
    *source = source.replacen(needle, replacement, 1);
    Ok(())
}

fn start_prepared_task(session: &LiveSession<'_>, card: PreparedScriptTask) -> Result<(), String> {
    session.play.script_start_handle().start_load(
        &session.account,
        card.js,
        card.shape,
        Some(card.settings),
        card.siblings,
    )
}

fn wait_script_state(
    session: &mut LiveSession<'_>,
    expected: script::RunState,
    description: &str,
) -> Result<(), String> {
    let deadline = Instant::now() + ACTION_DEADLINE;
    loop {
        drain_logs(session);
        let state = session.play.script_state(&session.account);
        if state == expected {
            return Ok(());
        }
        if let Some(error) = session.play.script_last_error(&session.account) {
            return Err(format!(
                "{description}: compat v1 script failed in {state:?}: {error}"
            ));
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "{description}: timed out in state {state:?}; logs: {:?}",
                session.receipt.logs
            ));
        }
        thread::sleep(POLL_INTERVAL);
    }
}

fn prepare_catalog_task(
    root: &Path,
    temp: &Path,
    card_name: &str,
    source_relative_path: &str,
) -> Result<PreparedScriptTask, String> {
    let catalog_commit = git_output(root, &["rev-parse", "--short=10", "HEAD"])?;
    if !catalog_commit.starts_with(CATALOG_SHORT) {
        return Err(format!(
            "expected rs2b0t catalog {CATALOG_SHORT}, found {catalog_commit}"
        ));
    }
    let source_path = root.join(source_relative_path);
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
    Ok(PreparedScriptTask {
        js: card.js,
        shape: card.shape,
        settings,
        siblings,
    })
}

fn prepare_file_task(
    root: &Path,
    temp: &Path,
    card_name: &str,
    source_relative_path: &str,
) -> Result<PreparedScriptTask, String> {
    let catalog_commit = git_output(root, &["rev-parse", "--short=10", "HEAD"])?;
    if !catalog_commit.starts_with(CATALOG_SHORT) {
        return Err(format!(
            "expected rs2b0t catalog {CATALOG_SHORT}, found {catalog_commit}"
        ));
    }
    let source_path = root.join(source_relative_path);
    if !source_path.is_file() {
        return Err(format!(
            "pinned source file is missing: {}",
            source_path.display()
        ));
    }
    let source_bytes = std::fs::read(&source_path)
        .map_err(|error| format!("read {}: {error}", source_path.display()))?;
    let source_sha = script_js::JsCache::origin_sha(&source_bytes);
    let mut library =
        script_js::JsLibrary::with_cache(temp.join("js-scripts.json"), temp.join("js-cache"));
    let card = library
        .load(&source_path)
        .map_err(|error| format!("load File card {card_name}: {error}"))?;
    let expected_source = source_path
        .canonicalize()
        .map_err(|error| format!("canonicalize pinned source: {error}"))?;
    let loaded_source = card
        .path
        .canonicalize()
        .map_err(|error| format!("canonicalize loaded File card: {error}"))?;
    if card.name != card_name
        || card.source != script::ScriptSource::File
        || loaded_source != expected_source
        || card.sha256 != source_sha
    {
        return Err(format!(
            "File card identity mismatch for {card_name}: name={}, source={:?}, loaded={}, expected={}, expected_sha={}, loaded_sha={}",
            card.name,
            card.source,
            loaded_source.display(),
            expected_source.display(),
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
    let settings = script::merge_bag(&card.settings_schema, &Map::new(), Some(&Map::new()));
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
    Ok(PreparedScriptTask {
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
    let mut card = prepare_catalog_task(
        &catalog_root,
        &temp,
        "ClueSolver",
        "src/bot/scripts/ClueSolver/ClueSolver.ts",
    )?;
    // ClueSolver declares a loadout setting, so the Load harness needs one named row.
    const CLUE_LOADOUT: &str = "compat-v1-live";
    card.settings
        .insert("loadout".into(), Value::String(CLUE_LOADOUT.into()));
    let loadouts = [api::loadout::Loadout::new(CLUE_LOADOUT)];
    session
        .play
        .script_start_handle()
        .start_load_with_loadouts(
            &session.account,
            card.js,
            card.shape,
            Some(card.settings),
            card.siblings,
            &loadouts,
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

const INPUT_SURFACE_SOURCE: &str = r#"
import { Input } from '../../input/Input.js';

const removedMembers = [
    'useItemOnLoc',
    'useItemOnNpc',
    'useItemOnObj',
    'useItemOnItem',
    'castOnNpc',
    'castOnLoc',
    'castOnItem',
    'continueDialog',
];
const read = (target, name) => {
    try { return { ok: true, value: target[name] }; }
    catch (error) { return { ok: false, message: error.message }; }
};

export default class CompatInputSurfaceProbe extends LoopingBot {
    onStart() {
        for (const member of removedMembers) {
            if (Object.hasOwn(Input, member)
                || Object.getOwnPropertyDescriptor(Input, member) !== undefined
                || Object.keys(Input).includes(member)) {
                throw new Error(`removed Input member remains: ${member}`);
            }
        }
        const unknown = read(Input, '__compat_census_unknown');
        if (unknown.ok || unknown.message !== 'not impl: Input.__compat_census_unknown') {
            throw new Error('unknown Input reads must retain the generic refusal');
        }
        if (typeof Input.walk !== 'function' || Input.walk(NaN, 0) !== false) {
            throw new Error('Input.walk sibling contract changed');
        }
        if (typeof Input.invButton !== 'function' || Input.invButton(NaN, 0, 0, 1) !== false) {
            throw new Error('Input.invButton sibling contract changed');
        }
        this.log('INPUT:READY');
    }
    loop() {}
    onStop() { this.log('INPUT:STOP'); }
}
"#;

fn run_input_surface_cell(receipt: &mut Receipt) -> Result<(), String> {
    let mut session = session("compat-v1-input-removal", receipt)?;
    start_script(&session, INPUT_SURFACE_SOURCE.to_owned())?;
    wait_for_logs(&mut session, ACTION_DEADLINE, |logs| {
        logs.iter().any(|line| line == "INPUT:READY")
    })?;
    stop_script(&mut session, "INPUT:STOP", true)?;
    if count_prefix(&session.receipt.logs, "INPUT:READY") != 1
        || count_prefix(&session.receipt.logs, "INPUT:STOP") != 1
    {
        return Err(format!(
            "input surface cell did not complete exactly once: {:?}",
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
    // Engine L locks tutorial side icons until the account logs in again.
    request_mainland_hop(&session.state)?;
    request_command(&session, "getvar tutorial")?;
    wait_command(&mut session, "getvar tutorial")?;
    let tutorial = wait_for_chat_line(&session.state, "get tutorial: 1000")?;
    session
        .receipt
        .logs
        .push(format!("EVENTS:TUTORIAL:{tutorial}"));
    request_logout(&session.state)?;
    wait_for_logged_out(&session.state)?;
    request_login(&session)?;
    wait_ready(&session.state)?;
    session.receipt.logs.push("EVENTS:RELOGIN:READY".into());

    // Load the inventory component before the first pack read or change.
    wait_for_side_tab_available(&session.state, INVENTORY_SIDE_TAB)?;
    request_side_tab(&session.state, INVENTORY_SIDE_TAB)?;
    let (_, coins_before) = wait_for_inventory_count(&session.state, 995, 0)?;
    if coins_before != 0 {
        return Err(format!(
            "fresh account unexpectedly has {coins_before} coins before the event seed"
        ));
    }
    request_command(&session, "give coins 1")?;
    wait_command(&mut session, "give coins 1")?;
    request_seed_at(
        &session.state,
        EVENTS_SEED_TILE.2,
        EVENTS_SEED_TILE.0,
        EVENTS_SEED_TILE.1,
    );
    wait_for_tile(&session.state, EVENTS_SEED_TILE)?;
    let (inventory_size, coins_after_seed) =
        wait_for_inventory_count(&session.state, 995, coins_before.saturating_add(1))?;
    session.receipt.logs.push(format!(
        "EVENTS:SEED:{},{},{}:slots={inventory_size}:coins={coins_after_seed}",
        EVENTS_SEED_TILE.0, EVENTS_SEED_TILE.1, EVENTS_SEED_TILE.2
    ));
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

    let (inventory_size, coins_before) = wait_for_inventory_count(&session.state, 995, 0)?;
    session.receipt.logs.push(format!(
        "INV:SNAPSHOT:BEFORE:slots={inventory_size}:count={coins_before}"
    ));
    request_command(&session, "give coins 1")?;
    wait_command(&mut session, "give coins 1")?;
    let (inventory_size, coins_after_first) =
        wait_for_inventory_count(&session.state, 995, coins_before.saturating_add(1))?;
    session.receipt.logs.push(format!(
        "INV:SNAPSHOT:AFTER_ONE:slots={inventory_size}:count={coins_after_first}"
    ));
    wait_for_logs(&mut session, ACTION_DEADLINE, |logs| {
        count_prefix(logs, "INV:OFF:") == 1 && count_prefix(logs, "INV:WITNESS:") == 1
    })?;

    request_command(&session, "give coins 2")?;
    wait_command(&mut session, "give coins 2")?;
    let (inventory_size, coins_after_second) =
        wait_for_inventory_count(&session.state, 995, coins_after_first.saturating_add(2))?;
    session.receipt.logs.push(format!(
        "INV:SNAPSHOT:AFTER_TWO:slots={inventory_size}:count={coins_after_second}"
    ));
    wait_for_logs(&mut session, ACTION_DEADLINE, |logs| {
        count_prefix(logs, "INV:OFF:") == 1 && count_prefix(logs, "INV:WITNESS:") == 2
    })?;

    request_command(&session, "setstat prayer 2")?;
    wait_command(&mut session, "setstat prayer 2")?;
    wait_for_logs(&mut session, ACTION_DEADLINE, |logs| {
        count_prefix(logs, "XP:PRAYER:") >= 1 && count_prefix(logs, "LEVEL:PRAYER:") >= 1
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
        || count_prefix(&session.receipt.logs, "LEVEL:PRAYER:") != 1
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
    let prayer_level = event_payloads(logs, "LEVEL:PRAYER:")?;
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
        || prayer_level[0]["name"] != "prayer"
        || prayer_level[0]["level"].as_i64().is_none()
        || prayer_level[0]["previous"].as_i64().is_none()
        || prayer_level[0]["level"] == prayer_level[0]["previous"]
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

#[test]
#[ignore = "requires LIVE=1, throwaway HOME, pinned catalog, and real 289 Engine L"]
fn compat_v1_input_removal_live() {
    run_and_record("compat_v1_input_removal", run_input_surface_cell)
        .unwrap_or_else(|error| panic!("{error}"));
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

const RUNTIME_ACCOUNT_LEVELER_FRAME_SOURCE: &str = r#"
import { LoopingBot } from '@rs2b0t/api';
import { Bank } from '../../api/bank/Bank.js';
import { Game } from '../../api/game/Game.js';
import { BotHost } from '../../runtime/BotHost.js';
import { ScriptRunner } from '../../runtime/ScriptRunner.js';

export default class CompatAccountLevelerFrameProbe extends LoopingBot {
    phase = 'train';
    status = 'RUNTIME:ACCOUNT_LEVELER_FRAME';
    detail = '';
    lastLoggedAt = Date.now();
    stopObserving = null;
    stock = {
        observe: (items, ready) => {
            this.detail = `bankItems=${items.length}, bankReady=${ready}`;
        },
    };
    session = {
        sampleWork: (now, active) => {
            this.detail += `; sampleAtValid=${Number.isFinite(now)}, workActive=${active}`;
        },
    };

    async onStart() {
        this.log(`RUNTIME:ACCOUNT_LEVELER_BANK:items=${Bank.items().length}:ready=${Bank.ready()}`);
        this.lastLoggedAt = Date.now() - 30000;
        this.stopObserving = BotHost.addFrameListener(() => {
            this.stock.observe(Bank.items(), Bank.ready());
            this.session.sampleWork(Date.now(), this.phase === 'train' && ScriptRunner.state === 'running' && !Bank.isOpen() && (Game.animating() || Game.inCombat()));
            if (ScriptRunner.state === 'running' && Date.now() - this.lastLoggedAt >= 30000) {
                const tile = Game.tile();
                this.log(`Still ${this.status}${this.detail ? `; ${this.detail}` : ''}; position ${tile ? `(${tile.x},${tile.z},${tile.level})` : 'unknown'}`);
            }
        });
    }

    loop() {}

    onStop() {
        this.stopObserving?.();
        this.log('RUNTIME:ACCOUNT_LEVELER_STOP');
    }

    log(message) {
        this.lastLoggedAt = Date.now();
        super.log(message);
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

fn run_runtime_account_leveler_frame_probe_cell(receipt: &mut Receipt) -> Result<(), String> {
    receipt.probe_source = Some(RUNTIME_ACCOUNT_LEVELER_FRAME_SOURCE.to_owned());
    receipt.coverage_notes.push(
        "The listener body copies pinned AccountLeveler.ts:96-103. This normal-Load probe is not AccountLeveler Catalog-card coverage. The catalog card is wave-4-blocked by its private journal.js import at line 17.".into(),
    );
    let mut session = session("compat-v1-runtime-accountleveler-frame", receipt)?;
    start_script(&session, RUNTIME_ACCOUNT_LEVELER_FRAME_SOURCE.to_owned())?;
    wait_for_logs(&mut session, ACTION_DEADLINE, |logs| {
        logs.iter()
            .any(|line| line.starts_with("Still RUNTIME:ACCOUNT_LEVELER_FRAME;"))
    })?;
    if session.play.script_state(&session.account) != script::RunState::Running {
        return Err(format!(
            "AccountLeveler frame probe did not reach Running: {:?}",
            session.play.script_state(&session.account)
        ));
    }
    let frame_log = session
        .receipt
        .logs
        .iter()
        .find(|line| line.starts_with("Still RUNTIME:ACCOUNT_LEVELER_FRAME;"))
        .ok_or_else(|| "AccountLeveler frame probe did not log its frame sample".to_string())?;
    if !frame_log.contains("bankItems=0, bankReady=false")
        || !frame_log.contains("sampleAtValid=true, workActive=false")
        || !frame_log.contains("position (")
    {
        return Err(format!(
            "AccountLeveler frame sample did not include the empty bank state, work state and tile: {frame_log}"
        ));
    }
    if !session
        .receipt
        .logs
        .iter()
        .any(|line| line == "RUNTIME:ACCOUNT_LEVELER_BANK:items=0:ready=false")
    {
        return Err(format!(
            "AccountLeveler frame probe did not record the closed-bank state: {:?}",
            session.receipt.logs
        ));
    }
    stop_script(&mut session, "RUNTIME:ACCOUNT_LEVELER_STOP", false)?;
    Ok(())
}

#[test]
#[ignore = "requires LIVE=1, throwaway HOME, pinned source, and real 289 Engine L"]
fn compat_v1_runtime_account_leveler_frame_live() {
    run_and_record(
        "compat_v1_runtime_account_leveler_frame",
        run_runtime_account_leveler_frame_probe_cell,
    )
    .unwrap_or_else(|error| panic!("{error}"));
}

fn instrument_alcher_active_event(js: &mut String) -> Result<(), String> {
    replace_once(
        js,
        "export default class Alcher extends TaskBot {",
        "class Alcher extends TaskBot {",
    )?;
    js.insert_str(
        0,
        "import { SettingsStore } from '../../runtime/Settings.js';\n",
    );
    js.push_str(
        r#"
export default class CompatAlcherActiveEventProbe extends Alcher {
    onStart() {
        this.sampleEvent();
        this.__compatLastActiveEvent = ScriptRunner.ctx?.activeEvent ?? null;
        this.log('RUNTIME:ALCHER_ACTIVE_EVENT:' + (this.__compatLastActiveEvent ?? '<null>'));
        this.__compatSettingsUnsubscribe = SettingsStore.onChange((name, key, raw) => {
            if (name === 'CompatRuntimeProbe' && key === 'catalogWrite') {
                this.log('RUNTIME:CATALOG_SETTINGS_CHANGE:' + name + ':' + key + ':' + raw);
            }
        });
        this.log('RUNTIME:CATALOG_SETTINGS_SAVE_BEFORE');
        SettingsStore.save('CompatRuntimeProbe', 'catalogWrite', 'catalog-value');
        this.log('RUNTIME:CATALOG_SETTINGS_SAVE_RETURN');
        this.log('RUNTIME:CATALOG_SETTINGS_READBACK:' +
            (SettingsStore.saved('CompatRuntimeProbe', 'catalogWrite') ?? '<undefined>'));
    }
    loop() {
        this.sampleEvent();
        const activeEvent = ScriptRunner.ctx?.activeEvent ?? null;
        if (activeEvent !== this.__compatLastActiveEvent) {
            this.__compatLastActiveEvent = activeEvent;
            this.log('RUNTIME:ALCHER_ACTIVE_EVENT:' + (activeEvent ?? '<null>'));
        }
    }
    onStop() {
        this.__compatSettingsUnsubscribe?.();
        this.log('RUNTIME:ALCHER_STOP');
    }
}
"#,
    );
    Ok(())
}

fn run_runtime_catalog_cell(receipt: &mut Receipt) -> Result<(), String> {
    receipt.coverage_notes.push(
        "The real Alcher Catalog card writes one isolated setting. It verifies one synchronous listener event and saved-value read-back. The WalkTo File card separately checks the map-pick consumer.".into(),
    );
    receipt.coverage_notes.push(
        "AccountLeveler Catalog-card coverage is blocked by AccountLeveler.ts:17 importing private api/ai/quests/defs/dragonslayer/journal.js. Its lines 96-103 run in the separate normal-Load frame probe.".into(),
    );
    let mut session = session("compat-v1-runtime-catalog", receipt)?;
    let catalog_root = required_path("RS2B0T")?;
    let temp = session
        ._isolated
        .home
        .join("compat-v1-live")
        .join(&session.account);

    let mut alcher = prepare_catalog_task(
        &catalog_root,
        &temp,
        "Alcher",
        "src/bot/scripts/Alcher/Alcher.ts",
    )?;
    instrument_alcher_active_event(&mut alcher.js)?;
    let alcher_start = session.receipt.logs.len();
    start_prepared_task(&session, alcher)?;
    wait_for_logs(&mut session, ACTION_DEADLINE, |logs| {
        logs.get(alcher_start..).is_some_and(|recent| {
            recent
                .iter()
                .any(|line| line.starts_with("RUNTIME:ALCHER_ACTIVE_EVENT:"))
                && recent
                    .iter()
                    .any(|line| line == "RUNTIME:CATALOG_SETTINGS_CHANGE:CompatRuntimeProbe:catalogWrite:catalog-value")
                && recent
                    .iter()
                    .any(|line| line == "RUNTIME:CATALOG_SETTINGS_SAVE_RETURN")
                && recent
                    .iter()
                    .any(|line| line == "RUNTIME:CATALOG_SETTINGS_READBACK:catalog-value")
        })
    })?;
    if session.play.script_state(&session.account) != script::RunState::Running {
        return Err(format!(
            "Alcher did not reach Running: {:?}",
            session.play.script_state(&session.account)
        ));
    }
    let catalog_logs = &session.receipt.logs[alcher_start..];
    let save_before = catalog_logs
        .iter()
        .position(|line| line == "RUNTIME:CATALOG_SETTINGS_SAVE_BEFORE")
        .ok_or_else(|| "Alcher Catalog settings save did not start".to_string())?;
    let change = catalog_logs
        .iter()
        .position(|line| {
            line == "RUNTIME:CATALOG_SETTINGS_CHANGE:CompatRuntimeProbe:catalogWrite:catalog-value"
        })
        .ok_or_else(|| "Alcher Catalog settings listener did not run".to_string())?;
    let save_return = catalog_logs
        .iter()
        .position(|line| line == "RUNTIME:CATALOG_SETTINGS_SAVE_RETURN")
        .ok_or_else(|| "Alcher Catalog settings save did not return".to_string())?;
    let readback = catalog_logs
        .iter()
        .position(|line| line == "RUNTIME:CATALOG_SETTINGS_READBACK:catalog-value")
        .ok_or_else(|| "Alcher Catalog settings read-back failed".to_string())?;
    if count_prefix(catalog_logs, "RUNTIME:CATALOG_SETTINGS_CHANGE:") != 1
        || !(save_before < change && change < save_return && save_return < readback)
    {
        return Err(format!(
            "Alcher Catalog settings change was not synchronous and exactly once: {catalog_logs:?}"
        ));
    }
    stop_script(&mut session, "RUNTIME:ALCHER_STOP", false)?;
    Ok(())
}

#[test]
#[ignore = "requires LIVE=1, throwaway HOME, pinned catalog, and real 289 Engine L"]
fn compat_v1_runtime_catalog_live() {
    run_and_record("compat_v1_runtime_catalog", run_runtime_catalog_cell)
        .unwrap_or_else(|error| panic!("{error}"));
}
fn instrument_walk_to_settings(js: &mut String) -> Result<(), String> {
    replace_once(
        js,
        "export default class WalkToBot extends TaskBot {",
        "class WalkToBot extends TaskBot {",
    )?;
    js.push_str(
        r#"
export default class CompatWalkToSettingsProbe extends WalkToBot {
    __mapPickerUnsubscribe = null;
    __savedWalk(stage) {
        this.log(`RUNTIME:WALK_SAVED_${stage}:` + JSON.stringify({
            destination: SettingsStore.saved('WalkTo', 'destination') ?? null,
            customTile: SettingsStore.saved('WalkTo', 'customTile') ?? null,
        }));
    }
    async onStart() {
        // The saved rows WalkToBot reads, so a restart that finds no
        // destination shows which row it was missing.
        this.__savedWalk('AT_START');
        try {
            await super.onStart();
        } catch (error) {
            this.__savedWalk('AT_FAIL');
            throw error;
        }
        // Mirror the map picker's listener for a display-setting change.
        this.__mapPickerUnsubscribe = SettingsStore.onChange((name, key, raw) => {
            if (name === 'MapPicker' && key === 'showBasemap') {
                this.log(`RUNTIME:MAP_PICKER_CHANGE:${name}:${key}:${raw}`);
            }
        });
        this.log('RUNTIME:MAP_PICKER_SAVE_BEFORE');
        SettingsStore.save('MapPicker', 'showBasemap', 'false');
        this.log('RUNTIME:MAP_PICKER_SAVE_RETURN');
        if (SettingsStore.saved('WalkTo', 'customTile') === undefined) {
            const here = Game.tile();
            if (!here) throw new Error('WalkTo settings probe has no player tile');
            const raw = `${here.x + 16},${here.z},${here.level}`;
            SettingsStore.save('WalkTo', 'customTile', raw);
            this.log('RUNTIME:WALK_WRITE:' + raw);
        }
        const target = this.targetTile();
        this.log('RUNTIME:WALK_SETTINGS:' + JSON.stringify({
            raw: SettingsStore.saved('WalkTo', 'customTile'),
            destination: SettingsStore.saved('WalkTo', 'destination'),
            label: this.destLabel(),
            target: { x: target.x, z: target.z, level: target.level },
        }));
    }
    async loop() {
        const work = super.loop();
        let settled = false;
        Promise.resolve(work).then(
            () => { settled = true; },
            () => { settled = true; },
        );
        await Promise.resolve();
        if (!settled && !this.__compatPendingLogged) {
            this.__compatPendingLogged = true;
            this.log('RUNTIME:WALK_PENDING');
        }
        await work;
        this.log('RUNTIME:WALK_RETURN');
    }
    onPause() { this.log('RUNTIME:WALK_PAUSE'); }
    onResume() { this.log('RUNTIME:WALK_RESUME'); }
    // The progress paint calls Traversal.remaining, which lands in wave 3.
    // This probe covers settings and lifecycle only.
    onPaint() {}
    onStop() {
        this.__mapPickerUnsubscribe?.();
        super.onStop();
        this.log('RUNTIME:WALK_STOP');
    }
}
"#,
    );
    Ok(())
}

fn validate_walk_map_pick(logs: &[String], start: usize) -> Result<String, String> {
    let payload = logs
        .get(start..)
        .and_then(|recent| {
            recent
                .iter()
                .find_map(|line| line.strip_prefix("RUNTIME:WALK_SETTINGS:"))
        })
        .ok_or_else(|| "WalkTo did not report its resolved map-pick settings".to_string())?;
    let settings: Value = serde_json::from_str(payload)
        .map_err(|error| format!("invalid WalkTo settings: {error}"))?;
    let raw = settings
        .get("raw")
        .and_then(Value::as_str)
        .ok_or_else(|| "WalkTo did not read the saved custom tile".to_string())?;
    if settings.get("destination").and_then(Value::as_str) != Some("Map pick") {
        return Err(format!(
            "WalkTo did not select Map pick after the custom-tile write: {settings}"
        ));
    }
    let mut parts = raw.split(',').map(|part| {
        part.parse::<i32>()
            .map_err(|error| format!("invalid saved WalkTo tile {raw:?}: {error}"))
    });
    let x = parts
        .next()
        .ok_or_else(|| "saved WalkTo tile has no x".to_string())??;
    let z = parts
        .next()
        .ok_or_else(|| "saved WalkTo tile has no z".to_string())??;
    let level = parts
        .next()
        .ok_or_else(|| "saved WalkTo tile has no level".to_string())??;
    if parts.next().is_some() {
        return Err(format!("saved WalkTo tile has extra coordinates: {raw:?}"));
    }
    let expected_label = format!("Map pick {x},{z}");
    if settings.get("label").and_then(Value::as_str) != Some(expected_label.as_str()) {
        return Err(format!(
            "WalkTo did not apply the map-pick label {expected_label:?}: {settings}"
        ));
    }
    let target = settings
        .get("target")
        .ok_or_else(|| "WalkTo did not report its target tile".to_string())?;
    if target.get("x").and_then(Value::as_i64) != Some(i64::from(x))
        || target.get("z").and_then(Value::as_i64) != Some(i64::from(z))
        || target.get("level").and_then(Value::as_i64) != Some(i64::from(level))
    {
        return Err(format!(
            "WalkTo target does not match saved map-pick tile {raw:?}: {settings}"
        ));
    }
    Ok(raw.to_owned())
}

fn run_runtime_settings_walk_cell(receipt: &mut Receipt) -> Result<(), String> {
    receipt.coverage_notes.push(
        "WalkToBot.ts loads as a ScriptSource::File card. Its ../../api/map/WalkDestinations.js import resolves to the served compatibility module. This proves File-card, not Catalog-card, coverage. The Alcher Catalog cell separately checks settings writes, listener delivery and read-back.".into(),
    );
    let mut session = session("compat-v1-runtime-settings-walk", receipt)?;
    let catalog_root = required_path("RS2B0T")?;
    let temp = session
        ._isolated
        .home
        .join("compat-v1-live")
        .join(&session.account);
    let mut card = prepare_file_task(
        &catalog_root,
        &temp,
        "WalkToBot",
        "src/bot/scripts/WalkToBot/WalkToBot.ts",
    )?;
    instrument_walk_to_settings(&mut card.js)?;
    start_prepared_task(&session, card)?;
    wait_for_logs(&mut session, ACTION_DEADLINE, |logs| {
        logs.iter()
            .any(|line| line.starts_with("RUNTIME:WALK_WRITE:"))
            && logs.iter().any(|line| line == "RUNTIME:WALK_PENDING")
            && logs
                .iter()
                .any(|line| line == "RUNTIME:MAP_PICKER_CHANGE:MapPicker:showBasemap:false")
            && logs
                .iter()
                .any(|line| line == "RUNTIME:MAP_PICKER_SAVE_RETURN")
    })?;
    if session.play.script_state(&session.account) != script::RunState::Running {
        return Err(format!(
            "WalkTo did not reach Running: {:?}",
            session.play.script_state(&session.account)
        ));
    }
    let first_map_pick = validate_walk_map_pick(&session.receipt.logs, 0)?;
    let written_tile = session
        .receipt
        .logs
        .iter()
        .find_map(|line| line.strip_prefix("RUNTIME:WALK_WRITE:"))
        .ok_or_else(|| "WalkTo did not log an accepted settings write".to_string())?;
    if written_tile != first_map_pick {
        return Err(format!(
            "WalkTo saved tile {written_tile:?} differs from its resolved tile {first_map_pick:?}"
        ));
    }
    let map_picker_save_before = session
        .receipt
        .logs
        .iter()
        .position(|line| line == "RUNTIME:MAP_PICKER_SAVE_BEFORE")
        .ok_or_else(|| "MapPicker save did not start".to_string())?;
    let map_picker_change = session
        .receipt
        .logs
        .iter()
        .position(|line| line == "RUNTIME:MAP_PICKER_CHANGE:MapPicker:showBasemap:false")
        .ok_or_else(|| "MapPicker change consumer did not run".to_string())?;
    let map_picker_save_return = session
        .receipt
        .logs
        .iter()
        .position(|line| line == "RUNTIME:MAP_PICKER_SAVE_RETURN")
        .ok_or_else(|| "MapPicker save did not return".to_string())?;
    if count_prefix(&session.receipt.logs, "RUNTIME:MAP_PICKER_CHANGE:") != 1
        || !(map_picker_save_before < map_picker_change
            && map_picker_change < map_picker_save_return)
    {
        return Err(format!(
            "MapPicker change delivery was not synchronous and exactly once: {:?}",
            session.receipt.logs
        ));
    }

    session.play.script_pause(&session.account);
    wait_script_state(
        &mut session,
        script::RunState::Paused,
        "pausing pending WalkTo",
    )?;
    wait_for_logs(&mut session, STOP_DEADLINE, |logs| {
        logs.iter().any(|line| line == "RUNTIME:WALK_PAUSE")
    })?;
    let pending_index = session
        .receipt
        .logs
        .iter()
        .position(|line| line == "RUNTIME:WALK_PENDING")
        .ok_or_else(|| "WalkTo did not record its pending traversal".to_string())?;
    let pause_index = session
        .receipt
        .logs
        .iter()
        .position(|line| line == "RUNTIME:WALK_PAUSE")
        .ok_or_else(|| "WalkTo did not run its pause hook".to_string())?;
    if session.receipt.logs[pending_index..pause_index]
        .iter()
        .any(|line| line == "RUNTIME:WALK_RETURN")
    {
        return Err("WalkTo completed before the pending pause took effect".into());
    }

    session.play.script_resume(&session.account);
    wait_script_state(
        &mut session,
        script::RunState::Running,
        "resuming pending WalkTo",
    )?;
    wait_for_logs(&mut session, STOP_DEADLINE, |logs| {
        logs.iter().any(|line| line == "RUNTIME:WALK_RESUME")
    })?;
    stop_script(&mut session, "RUNTIME:WALK_STOP", false)?;

    let restart_start = session.receipt.logs.len();
    let mut restart = prepare_file_task(
        &catalog_root,
        &temp,
        "WalkToBot",
        "src/bot/scripts/WalkToBot/WalkToBot.ts",
    )?;
    instrument_walk_to_settings(&mut restart.js)?;
    start_prepared_task(&session, restart)?;
    wait_for_logs(&mut session, ACTION_DEADLINE, |logs| {
        logs.get(restart_start..).is_some_and(|recent| {
            recent
                .iter()
                .any(|line| line.starts_with("RUNTIME:WALK_SETTINGS:"))
                && recent.iter().any(|line| line == "RUNTIME:WALK_PENDING")
                && recent
                    .iter()
                    .any(|line| line == "RUNTIME:MAP_PICKER_CHANGE:MapPicker:showBasemap:false")
                && recent
                    .iter()
                    .any(|line| line == "RUNTIME:MAP_PICKER_SAVE_RETURN")
        })
    })?;
    if session.receipt.logs[restart_start..]
        .iter()
        .any(|line| line.starts_with("RUNTIME:WALK_WRITE:"))
    {
        return Err("WalkTo rewrote its custom tile instead of loading it after restart".into());
    }
    let restarted_map_pick = validate_walk_map_pick(&session.receipt.logs, restart_start)?;
    if restarted_map_pick != first_map_pick {
        return Err(format!(
            "WalkTo restart changed the saved map-pick tile from {first_map_pick:?} to {restarted_map_pick:?}"
        ));
    }
    let restart_logs = &session.receipt.logs[restart_start..];
    let restart_map_picker_save_before = restart_logs
        .iter()
        .position(|line| line == "RUNTIME:MAP_PICKER_SAVE_BEFORE")
        .ok_or_else(|| "restarted MapPicker save did not start".to_string())?;
    let restart_map_picker_change = restart_logs
        .iter()
        .position(|line| line == "RUNTIME:MAP_PICKER_CHANGE:MapPicker:showBasemap:false")
        .ok_or_else(|| "restarted MapPicker consumer did not run".to_string())?;
    let restart_map_picker_save_return = restart_logs
        .iter()
        .position(|line| line == "RUNTIME:MAP_PICKER_SAVE_RETURN")
        .ok_or_else(|| "restarted MapPicker save did not return".to_string())?;
    if count_prefix(restart_logs, "RUNTIME:MAP_PICKER_CHANGE:") != 1
        || !(restart_map_picker_save_before < restart_map_picker_change
            && restart_map_picker_change < restart_map_picker_save_return)
    {
        return Err(
            "restarted MapPicker change delivery was not synchronous and exactly once".into(),
        );
    }
    stop_script(&mut session, "RUNTIME:WALK_STOP", false)?;
    Ok(())
}

#[test]
#[ignore = "requires LIVE=1, throwaway HOME, pinned catalog, and real 289 Engine L"]
fn compat_v1_runtime_settings_walk_live() {
    run_and_record(
        "compat_v1_runtime_settings_walk",
        run_runtime_settings_walk_cell,
    )
    .unwrap_or_else(|error| panic!("{error}"));
}
