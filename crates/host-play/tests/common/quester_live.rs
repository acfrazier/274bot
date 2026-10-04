//! Shared live runner for Quester qualification cells (design-shell §7.3/§7.4).
//!
//! A quest's live file builds its fixture scenario (the `quester_stage`
//! builder: pre-Start seeds only) and calls [`run`] with one [`Mode`]:
//!
//! - [`Mode::Stage`]: the seeded stage settles into one of the expected next
//!   stage keys (PASS criterion 1);
//! - [`Mode::Clean`]: not_started → complete with no cheat after Start, no
//!   park, zero deaths, one Start (criterion 2);
//! - [`Mode::Restart`]: Stop mid-step at two stages, Start again, complete
//!   (criterion 3);
//! - [`Mode::Death`]: one `~death` while a named step of a stage runs (an
//!   `open` combat step), the runner recovers and completes with exactly one
//!   death (criterion 4).
//!
//! Every cell writes `status.jsonl` (each changed Quester status), CPU-rendered
//! PNG + JSON receipts at Start, every stage change, Stop/Start, death and the
//! end, and a final `receipt.json` under
//! `$LIVE_EVIDENCE_DIR/<quest>/<label>-<account>-<epoch>/`.
//!
//! Environment (all required): `LIVE=1`, `BOT_CPU=1`, `BOT_LIVE_NAME_PREFIX`,
//! `WORLD_GAME_PORT`, `WORLD_HTTP_PORT`, absolute `WORLD_NAV_PACK`,
//! `WORLD_ENGINE_DIR`, `RS2B0T`, `BOT_CACHE_DIR` (copied into the cell's temp
//! root before use, so the run never writes the source cache) and
//! `LIVE_EVIDENCE_DIR` (outside the throwaway HOME).
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use api::interact;
use api::selected::Knowledge;
use api::snapshot::{GameSnapshot, WorldTile};
use host::Pump;
use host_play::{ProfileOptions, ScriptStartHandle, SharedClientTemplate};
use scenario::{RunnerStatus, Scenario, ScenarioRunner};
use script::native::{NativePhase, ScriptStatus, StatusValue};
use serde_json::{json, Map, Value};
use vault::{Profile, ProfileSettings};

const POLL_INTERVAL: Duration = Duration::from_millis(20);
/// Time a Restart cell lets the step at the Stop stage run before stopping it,
/// so the Stop lands mid-step rather than on a selection boundary.
const MID_STEP: Duration = Duration::from_secs(4);
/// Grace after the scenario deadline for terminal bookkeeping.
const DEADLINE_GRACE: Duration = Duration::from_secs(15);

/// What the cell proves after the fixture's Start.
#[derive(Debug, Clone)]
pub enum Mode {
    /// Pass when the published stage key is one of `expect` (a key equal to
    /// the Path's `colour.complete` also passes on the green quest tab).
    Stage { expect: Vec<String> },
    /// Pass on quest complete with one Start, zero deaths and no park.
    Clean,
    /// Stop mid-step once at each stage key (in order), Start again, complete.
    Restart { at: [String; 2] },
    /// Send one `~death` while the stage key is `at` and the active step is
    /// `step` (after it has run [`MID_STEP`]); complete with one death. The
    /// status at injection is kept in the receipt (`death_status`).
    Death { at: String, step: String },
}

impl Mode {
    fn name(&self) -> &'static str {
        match self {
            Mode::Stage { .. } => "stage",
            Mode::Clean => "clean",
            Mode::Restart { .. } => "restart",
            Mode::Death { .. } => "death",
        }
    }
}

/// Proof hook run on the last pre-Start frame: returns the observed fixture
/// receipt (exact stats and kit) or refuses the Start.
pub type ObserveStart = Box<dyn FnMut(&GameSnapshot) -> Result<Value, String> + Send>;

/// One live cell.
pub struct Cell {
    /// Content quest id (`squire`), also the evidence sub-directory.
    pub quest: &'static str,
    /// Quest-tab display name, for the green-tab proof.
    pub display: &'static str,
    /// Cell label (`stage-squire-3`, `clean`, …).
    pub label: String,
    /// Fixture: pre-Start seeds and the Start step; nothing after Start may cheat.
    pub scenario: Scenario,
    /// The Quester settings bag the fixture Starts with.
    pub start_settings: Map<String, Value>,
    pub mode: Mode,
    pub observe_start: Option<ObserveStart>,
}

struct TempRoot(PathBuf);

impl TempRoot {
    fn new(label: &str) -> Result<Self, String> {
        let path = std::env::temp_dir().join(format!("274bot-quester-{label}-{}", epoch_nanos()?));
        std::fs::create_dir_all(&path)
            .map_err(|error| format!("create {}: {error}", path.display()))?;
        Ok(Self(path))
    }
}

impl Drop for TempRoot {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn epoch_nanos() -> Result<u128, String> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .map_err(|error| format!("clock: {error}"))
}

fn required(name: &str) -> Result<String, String> {
    std::env::var(name).map_err(|_| format!("{name} is required for Quester live cells"))
}

fn required_path(name: &str) -> Result<PathBuf, String> {
    let path = PathBuf::from(required(name)?);
    if !path.is_absolute() {
        return Err(format!("{name} must be absolute: {}", path.display()));
    }
    Ok(path)
}

fn required_port(name: &str) -> Result<u16, String> {
    required(name)?
        .parse()
        .map_err(|_| format!("{name} must be a port number"))
}

fn copy_dir(from: &Path, to: &Path) -> Result<(), String> {
    std::fs::create_dir_all(to).map_err(|error| format!("create {}: {error}", to.display()))?;
    for entry in
        std::fs::read_dir(from).map_err(|error| format!("read {}: {error}", from.display()))?
    {
        let entry = entry.map_err(|error| error.to_string())?;
        let target = to.join(entry.file_name());
        if entry
            .file_type()
            .map_err(|error| error.to_string())?
            .is_dir()
        {
            copy_dir(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), &target)
                .map_err(|error| format!("copy {}: {error}", entry.path().display()))?;
        }
    }
    Ok(())
}

fn selected_profile(
    temp: &Path,
) -> Result<(Arc<host_play::ServerProfile>, Arc<SharedClientTemplate>), String> {
    let nav_pack = required_path("WORLD_NAV_PACK")?;
    let engine_dir = required_path("WORLD_ENGINE_DIR")?;
    let catalog_root = required_path("RS2B0T")?;
    let source_cache = required_path("BOT_CACHE_DIR")?;
    if !nav_pack.is_file() {
        return Err(format!(
            "WORLD_NAV_PACK is not a file: {}",
            nav_pack.display()
        ));
    }
    for (name, path) in [
        ("WORLD_ENGINE_DIR", &engine_dir),
        ("RS2B0T", &catalog_root),
        ("BOT_CACHE_DIR", &source_cache),
    ] {
        if !path.is_dir() {
            return Err(format!("{name} is not a directory: {}", path.display()));
        }
    }
    let cache_dir = temp.join("cache");
    copy_dir(&source_cache, &cache_dir)?;
    let options = ProfileOptions {
        profile: Some("local-289".into()),
        revision: Some("289".into()),
        host: Some("127.0.0.1".into()),
        asset_host: Some("127.0.0.1".into()),
        port: Some(required_port("WORLD_GAME_PORT")?),
        http_port: Some(required_port("WORLD_HTTP_PORT")?),
        cache_dir: Some(cache_dir),
        nav_pack: Some(nav_pack),
        engine_dir: Some(engine_dir),
        unpack_dir: Some(temp.join("unpack")),
        vault_path: Some(temp.join("vault")),
        catalog_root: Some(catalog_root),
        ..ProfileOptions::default()
    };
    let profile = options.resolve(None)?.bind_runtime()?;
    if profile.profile_class() != host_play::ProfileClass::Local
        || profile.client().game_host() != "127.0.0.1"
    {
        return Err("Quester live cells require a loopback local profile".into());
    }
    let template = SharedClientTemplate::load(Arc::clone(&profile))?;
    if template.world().is_none() {
        return Err("selected template has no navigation world".into());
    }
    Ok((profile, template))
}

fn mint_profile(account: &str, password: &str) -> Result<Profile, String> {
    let uid = (epoch_nanos()? / 1_000_000 % i32::MAX as u128) as i32;
    Ok(Profile {
        username: account.to_owned(),
        password: password.to_owned().into(),
        uid,
        settings: ProfileSettings::default(),
    })
}

pub fn status_json(status: &ScriptStatus) -> Value {
    let fields: Map<String, Value> = status
        .fields
        .iter()
        .map(|field| {
            let value = match &field.value {
                StatusValue::Text(value) => json!(value.as_ref()),
                StatusValue::Integer(value) => json!(value),
                StatusValue::Tile(tile) => json!({"x": tile.x, "z": tile.z, "level": tile.level}),
                StatusValue::Truth(value) => json!(format!("{value:?}")),
                StatusValue::Quest(value) => json!({
                    "stage": stage_text(&value.stage),
                    "rule": stage_text(&value.rule),
                    "complete": format!("{:?}", value.complete),
                }),
            };
            (field.key.to_owned(), value)
        })
        .collect();
    json!({
        "run": format!("{:?}", status.run),
        "phase": format!("{:?}", status.phase),
        "fields": fields,
        "failure": status.failure.as_ref().map(|failure| format!("{failure:?}")),
    })
}

fn stage_text(stage: &Knowledge<api::selected::FactKey>) -> Option<String> {
    match stage {
        Knowledge::Known(key) => Some(key.0.to_string()),
        _ => None,
    }
}

fn field<'a>(status: &'a ScriptStatus, key: &str) -> Option<&'a StatusValue> {
    status
        .fields
        .iter()
        .find(|field| field.key == key)
        .map(|field| &field.value)
}

fn text<'a>(status: &'a ScriptStatus, key: &str) -> Option<&'a str> {
    match field(status, key)? {
        StatusValue::Text(text) => Some(text.as_ref()),
        _ => None,
    }
}

fn integer(status: &ScriptStatus, key: &str) -> Option<i64> {
    match field(status, key)? {
        StatusValue::Integer(value) => Some(*value),
        _ => None,
    }
}

/// The published stage key for `quest`, if the runner resolved one.
pub fn status_stage(status: &ScriptStatus, quest: &str) -> Option<String> {
    if text(status, "quest_id") != Some(quest) {
        return None;
    }
    match field(status, "quest")? {
        StatusValue::Quest(progress) => stage_text(&progress.stage),
        _ => None,
    }
}

fn parked(status: &ScriptStatus) -> Option<String> {
    if status.phase == NativePhase::Blocked {
        return Some(format!("phase Blocked: {:?}", status.failure));
    }
    text(status, "block_reason").map(str::to_owned)
}

fn observed_tile(snapshot: &GameSnapshot) -> Option<WorldTile> {
    snapshot
        .tile()
        .map(|(x, z, level)| WorldTile { x, z, level })
}

fn quest_done(snapshot: &GameSnapshot, display: &str) -> bool {
    snapshot.quest_statuses().iter().any(|row| {
        row.name.trim().eq_ignore_ascii_case(display)
            && row.status() == api::snapshot::QuestListStatus::Complete
    })
}

fn save_capture(
    client: &mut client::client::Client,
    directory: &Path,
    sequence: usize,
    label: &str,
    mut receipt: Value,
) -> Result<PathBuf, String> {
    let stem = format!("{sequence:02}-{label}");
    let mut renderer = client::render::Renderer::new_prefer(client.config.lowmem, false);
    let was_draw = client.draw;
    client.set_draw(true);
    let frame = renderer.mainredraw(client);
    client.set_draw(was_draw);
    let client::render::backend::FrameOutput::PixMap(pixels) = frame else {
        return Err("capture needs BOT_CPU=1 (no CPU PixMap)".into());
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
    let png_path = directory.join(format!("{stem}.png"));
    let file = std::fs::File::create(&png_path).map_err(|error| error.to_string())?;
    let mut encoder = png::Encoder::new(file, pixels.width as u32, pixels.height as u32);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder
        .write_header()
        .map_err(|error| error.to_string())?
        .write_image_data(&rgba)
        .map_err(|error| error.to_string())?;
    receipt["frame"] = json!({"renderer": "real Client CpuPix3D framebuffer",
        "width": pixels.width, "height": pixels.height});
    std::fs::write(
        directory.join(format!("{stem}.json")),
        serde_json::to_vec_pretty(&receipt).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    Ok(png_path)
}

/// Cross-thread cell state: the frame callback owns the client, the poll loop
/// owns the Play handle.
struct Shared {
    runner: ScenarioRunner,
    snapshot: GameSnapshot,
    pump: Pump,
    start_handle: Option<ScriptStartHandle>,
    observe_start: Option<ObserveStart>,
    start_receipt: Option<Value>,
    /// Starts issued (first by the fixture, later by Restart).
    starts: u32,
    /// Restart asked the frame thread to Start again.
    restart_due: bool,
    death_due: bool,
    death_sent: bool,
    death_seen: bool,
    pending_captures: Vec<(String, Value)>,
    captures: Vec<PathBuf>,
    last_tile: Option<WorldTile>,
    error: Option<String>,
}

/// Run one live cell. Returns the final receipt on PASS.
pub fn run(mut cell: Cell) -> Result<Value, String> {
    if std::env::var("LIVE").as_deref() != Ok("1") {
        return Err("Quester live cells require LIVE=1".into());
    }
    if std::env::var("BOT_CPU").as_deref() != Ok("1") {
        return Err("Quester live cells require BOT_CPU=1 for PNG evidence".into());
    }
    required("BOT_LIVE_NAME_PREFIX")?;
    let isolated = script::IsolatedEnv::enter(&cell.label);
    let evidence_root = required_path("LIVE_EVIDENCE_DIR")?;
    if evidence_root.starts_with(&isolated.home) {
        return Err("LIVE_EVIDENCE_DIR must be outside the throwaway HOME".into());
    }
    isolated.set_rs2b0t(&required_path("RS2B0T")?);
    api::hostlog::set_debug(true);
    if cell.scenario.settings.start_script != Some("Quester") {
        return Err(format!("{} does not Start Quester", cell.label));
    }
    cell.scenario.settings.nav.engine_speed_ms = None;

    let temp = TempRoot::new(&cell.label)?;
    let (profile, template) = selected_profile(&temp.0)?;
    let names = host_play::mint_live_names(1);
    let account = names.first().cloned().ok_or("no live account minted")?;
    let password = host_play::mint_live_entries(&names)
        .first()
        .map(|(_, password)| password.clone())
        .ok_or("no live credential minted")?;
    let directory = evidence_root.join(cell.quest).join(format!(
        "{}-{account}-{}",
        cell.label,
        epoch_nanos()? / 1_000_000_000
    ));
    std::fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
    let status_log = directory.join("status.jsonl");

    let deadline = Instant::now() + cell.scenario.settings.deadline + DEADLINE_GRACE;
    let mut runner = ScenarioRunner::with_world(cell.scenario, template.world());
    runner.set_map_members(profile.map_members());
    runner.set_live_names(&names);
    runner.set_shot_sink(Box::new(|_, _| {}));
    let shared = Arc::new(Mutex::new(Shared {
        runner,
        snapshot: GameSnapshot::new(),
        pump: Pump::new(),
        start_handle: None,
        observe_start: cell.observe_start.take(),
        start_receipt: None,
        starts: 0,
        restart_due: false,
        death_due: false,
        death_sent: false,
        death_seen: false,
        pending_captures: Vec::new(),
        captures: Vec::new(),
        last_tile: None,
        error: None,
    }));

    let frame_shared = Arc::clone(&shared);
    let frame_account = account.clone();
    let frame_settings = cell.start_settings.clone();
    let frame_directory = directory.clone();
    let mut play = host_play::run_with_template(
        Arc::clone(&template),
        true,
        vec![],
        |_| (None, None),
        move |client, username, hold| {
            if username != frame_account {
                return;
            }
            let mut guard = frame_shared.lock().unwrap();
            let s = &mut *guard;
            let drain = s.pump.drain_client(client);
            host::publish_snapshot(&mut s.snapshot, client, drain);
            s.last_tile = observed_tile(&s.snapshot);
            let first_start = s.runner.on_start_script() && s.starts == 0;
            if first_start || s.restart_due {
                if first_start {
                    if let Some(observe) = s.observe_start.as_mut() {
                        match observe(&s.snapshot) {
                            Ok(receipt) => s.start_receipt = Some(receipt),
                            Err(error) => {
                                s.error = Some(format!("pre-Start fixture proof: {error}"));
                                return;
                            }
                        }
                    }
                }
                let result = s
                    .start_handle
                    .as_ref()
                    .ok_or_else(|| "Start before ScriptStartHandle install".to_owned())
                    .and_then(|handle| {
                        handle.start_compiled(
                            &frame_account,
                            script::CompiledId("Quester"),
                            frame_settings.clone(),
                        )
                    });
                match result {
                    Ok(()) => {
                        s.starts += 1;
                        s.restart_due = false;
                    }
                    Err(error) if s.restart_due => {
                        // A Start refused while the Stop is settling is retried next frame.
                        let _ = error;
                    }
                    Err(error) => s.error = Some(format!("Quester Start failed: {error}")),
                }
            }
            if !matches!(
                s.runner.status(),
                RunnerStatus::Passed | RunnerStatus::Failed(_)
            ) {
                s.runner.tick_with_hold(client, hold.hold);
            }
            if s.death_due
                && !s.death_sent
                && !hold.hold
                && interact::cheat(client, "~death").is_sent()
            {
                s.death_sent = true;
                s.pending_captures.push(("death-sent".into(), json!({})));
            }
            if s.death_sent
                && !s.death_seen
                && s.snapshot
                    .chat_lines()
                    .iter()
                    .any(|line| script::native::death::is_death_line(&line.text))
            {
                s.death_seen = true;
                s.pending_captures
                    .push(("death-observed".into(), json!({})));
            }
            if client.ingame && client.scene_state == 2 && !s.pending_captures.is_empty() {
                let pending = std::mem::take(&mut s.pending_captures);
                for (label, mut receipt) in pending {
                    receipt["account"] = json!(frame_account);
                    receipt["tile"] = json!(s.last_tile.map(|t| [t.x, t.z, t.level]));
                    receipt["inventory"] = json!(s
                        .snapshot
                        .inventory()
                        .iter()
                        .filter(|item| item.count > 0)
                        .map(|item| [item.def.id, item.count])
                        .collect::<Vec<_>>());
                    receipt["equipment"] = json!(s
                        .snapshot
                        .equipment()
                        .iter()
                        .map(|item| item.def.id)
                        .collect::<Vec<_>>());
                    receipt["stats"] = json!(s
                        .snapshot
                        .stats()
                        .iter()
                        .map(|stat| json!([stat.name, stat.base, stat.effective]))
                        .collect::<Vec<_>>());
                    receipt["chat"] = json!(s
                        .snapshot
                        .chat_lines()
                        .iter()
                        .rev()
                        .take(12)
                        .map(|line| line.text.to_string())
                        .collect::<Vec<_>>());
                    let sequence = s.captures.len() + 1;
                    match save_capture(client, &frame_directory, sequence, &label, receipt) {
                        Ok(path) => s.captures.push(path),
                        Err(error) => s.error = Some(error),
                    }
                }
            }
        },
    )?;
    {
        let mut s = shared.lock().map_err(|_| "live state poisoned")?;
        s.runner.set_obj_names(play.obj_names());
        s.start_handle = Some(play.script_start_handle());
    }
    play.try_spawn_slot(mint_profile(&account, &password)?, None, None, None)?;
    play.focus(&account);
    println!(
        "{}",
        json!({"phase": "identity", "cell": cell.label, "quest": cell.quest, "mode": cell.mode.name(),
            "account": account, "game_port": profile.client().game_port(),
            "nav_pack": profile.nav_pack(), "settings": cell.start_settings,
            "evidence": directory})
    );

    let mut last_logged: Option<Value> = None;
    let mut stages_seen: Vec<String> = Vec::new();
    let mut deaths_max = 0i64;
    let mut restarts_done = 0usize;
    let mut stop_at: Option<Instant> = None;
    let mut awaiting_idle = false;
    let mut end_captured = false;
    let mut captured_starts = 0u32;
    let mut death_status: Option<Value> = None;
    let result: Result<(), String> = loop {
        let status = play.script_native_status(&account);
        let run_state = play.script_state(&account);
        let (runner_status, starts, error, done, death_seen) = {
            let mut s = shared.lock().map_err(|_| "live state poisoned")?;
            if run_state == script::RunState::Running {
                s.runner.observe_script_running();
            }
            (
                s.runner.status(),
                s.starts,
                s.error.clone(),
                quest_done(&s.snapshot, cell.display),
                s.death_seen,
            )
        };
        if let Some(error) = error {
            break Err(error);
        }
        if let RunnerStatus::Failed(error) = &runner_status {
            break Err(format!("fixture failed: {error}"));
        }
        if let Some(error) = play.script_last_error(&account) {
            break Err(format!("Quester lifecycle error: {error}"));
        }
        if let Some(status) = status.as_ref() {
            let logged = status_json(status);
            if last_logged.as_ref() != Some(&logged) {
                let line = json!({"t_ms": epoch_nanos()? / 1_000_000, "status": logged});
                use std::io::Write;
                let mut file = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&status_log)
                    .map_err(|error| error.to_string())?;
                writeln!(file, "{line}").map_err(|error| error.to_string())?;
                last_logged = Some(logged);
            }
            if status.phase == NativePhase::Working && captured_starts < starts {
                captured_starts = starts;
                let mut s = shared.lock().map_err(|_| "live state poisoned")?;
                s.pending_captures.push((
                    format!("start-{starts}"),
                    json!({"status": status_json(status)}),
                ));
            }
            if let Some(reason) = parked(status) {
                break Err(format!("Quester parked/blocked: {reason}"));
            }
            deaths_max = deaths_max.max(integer(status, "deaths").unwrap_or(0));
            if let Some(stage) = status_stage(status, cell.quest) {
                if stages_seen.last() != Some(&stage) {
                    stages_seen.push(stage.clone());
                    let mut s = shared.lock().map_err(|_| "live state poisoned")?;
                    s.pending_captures.push((
                        format!("stage-{}", stage.replace(':', "-")),
                        json!({"status": status_json(status)}),
                    ));
                }
            }
        }
        let current = stages_seen.last().cloned();
        match &cell.mode {
            Mode::Stage { expect } => {
                if starts > 0
                    && current.as_ref().is_some_and(|stage| expect.contains(stage))
                    && stages_seen.len() > 1
                {
                    break Ok(());
                }
            }
            Mode::Restart { at } if restarts_done < 2 => {
                if awaiting_idle {
                    if run_state == script::RunState::Idle {
                        awaiting_idle = false;
                        restarts_done += 1;
                        let mut s = shared.lock().map_err(|_| "live state poisoned")?;
                        s.pending_captures
                            .push((format!("stopped-{restarts_done}"), json!({})));
                        s.restart_due = true;
                    }
                } else if current.as_deref() == Some(at[restarts_done].as_str())
                    && starts as usize == restarts_done + 1
                    && run_state == script::RunState::Running
                {
                    let due = *stop_at.get_or_insert_with(|| Instant::now() + MID_STEP);
                    if Instant::now() >= due {
                        stop_at = None;
                        play.script_stop(&account);
                        awaiting_idle = true;
                    }
                }
            }
            Mode::Death { at, step } if death_status.is_none() => {
                let on_step = current.as_deref() == Some(at.as_str())
                    && status
                        .as_ref()
                        .is_some_and(|s| text(s, "step_id") == Some(step.as_str()));
                if !on_step {
                    stop_at = None;
                } else if Instant::now()
                    >= *stop_at.get_or_insert_with(|| Instant::now() + MID_STEP)
                {
                    death_status = status.as_ref().map(|s| status_json(s));
                    shared.lock().map_err(|_| "live state poisoned")?.death_due = true;
                }
            }
            _ => {}
        }
        let terminal = matches!(runner_status, RunnerStatus::Passed)
            && done
            && run_state == script::RunState::Idle
            && play
                .script_lifecycle_receipt(&account)
                .is_some_and(|receipt| receipt.state == script::ScriptTerminalState::Completed);
        if terminal {
            match &cell.mode {
                Mode::Stage { .. } => break Ok(()),
                Mode::Clean if starts == 1 && deaths_max == 0 => break Ok(()),
                Mode::Clean => {
                    break Err(format!("clean run had starts={starts} deaths={deaths_max}"))
                }
                Mode::Restart { .. } if restarts_done == 2 && starts == 3 => break Ok(()),
                Mode::Restart { .. } => {
                    break Err(format!(
                        "completed before both Stops: restarts={restarts_done} starts={starts}"
                    ))
                }
                Mode::Death { .. }
                    if death_seen && death_status.is_some() && deaths_max == 1 && starts == 1 =>
                {
                    break Ok(())
                }
                Mode::Death { .. } => {
                    break Err(format!(
                        "death cell completed with death_seen={death_seen} deaths={deaths_max} starts={starts}"
                    ))
                }
            }
        }
        if Instant::now() >= deadline {
            break Err(format!(
                "timed out: runner={runner_status:?} run_state={run_state:?} stages={stages_seen:?} status={}",
                status.as_ref().map(|s| status_json(s)).unwrap_or(Value::Null)
            ));
        }
        std::thread::sleep(POLL_INTERVAL);
    };
    // One terminal capture on the next frames, before the slot goes away.
    {
        let mut s = shared.lock().map_err(|_| "live state poisoned")?;
        s.pending_captures.push((
            if result.is_ok() {
                "end-pass".into()
            } else {
                "end-fail".into()
            },
            json!({"result": format!("{result:?}")}),
        ));
    }
    let capture_deadline = Instant::now() + Duration::from_secs(10);
    while !end_captured && Instant::now() < capture_deadline {
        end_captured = shared
            .lock()
            .map_err(|_| "live state poisoned")?
            .pending_captures
            .is_empty();
        std::thread::sleep(POLL_INTERVAL);
    }
    let s = shared.lock().map_err(|_| "live state poisoned")?;
    let receipt = json!({
        "cell": cell.label,
        "quest": cell.quest,
        "mode": cell.mode.name(),
        "result": format!("{result:?}"),
        "account": account,
        "game_port": profile.client().game_port(),
        "nav_pack": profile.nav_pack(),
        "settings": cell.start_settings,
        "fixture_receipt": s.start_receipt,
        "starts": s.starts,
        "stages_seen": stages_seen,
        "deaths": deaths_max,
        "death_sent": s.death_sent,
        "death_observed": s.death_seen,
        "death_status": death_status,
        "last_tile": s.last_tile.map(|t| [t.x, t.z, t.level]),
        "captures": s.captures,
        "final_status": play.script_native_status(&account).map(|status| status_json(&status)),
        "lifecycle": format!("{:?}", play.script_lifecycle_receipt(&account)),
    });
    drop(s);
    std::fs::write(
        directory.join("receipt.json"),
        serde_json::to_vec_pretty(&receipt).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    println!("{receipt}");
    play.script_stop(&account);
    play.stop_slot(&account);
    result.map(|()| receipt)
}
