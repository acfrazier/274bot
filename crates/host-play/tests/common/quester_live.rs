//! Shared live runner for Quester qualification cells (design-shell §7.3/§7.4).
//!
//! A quest's live file builds its fixture scenario (the `quester_stage`
//! builder: pre-Start seeds only) and calls [`run`] with one [`Mode`], or
//! supplies two reciprocal role cells to [`run_pair`] with one [`PairMode`]:
//!
//! - [`Mode::Stage`]/[`PairMode::Stage`]: seeded stage(s) settle into expected
//!   next stage keys (PASS criterion 1);
//! - [`Mode::PathTerminal`]: the authored Path ends without requiring quest
//!   completion; this mode is for focused Path probes.
//! - [`Mode::Clean`]/[`PairMode::Clean`]: clean completion with no cheat after
//!   Start, no park, zero deaths and one Start per account (criterion 2);
//! - [`Mode::Restart`]/[`PairMode::Restart`]: Stop mid-step at two stage points
//!   and explicitly restart every account (criterion 3);
//! - [`Mode::Death`]/[`PairMode::Death`]/[`PairMode::DeathIndependent`]:
//!   inject only during the named step and observed owned combat (criterion 4).
//!   Reserved pair phases require exact cancellation followed by fresh explicit
//!   Starts; independent ordinary roles must recover in the same runs. Only the
//!   named role may die.
//!
//! Paired runs use one local profile/template and two minted accounts. They
//! prepare both fixture scenarios before either initial Start and never
//! control a peer account's Quester.
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
//! Paired cells also require `BOT_LIVE_PARTNER_NAME_PREFIX` for role 1. Both
//! prefixes are 1–4 bytes, and both names retain the same invocation token.
//!
//! The generic Path smoke (`tests/quester_path_live.rs`) additionally takes
//! `QUESTER_PATH` (content quest id), `QUESTER_SEEDS` (small JSON file with
//! the stage, stand, loadout, extra items, explicit stage seeds, pre-relog
//! cheats and expected stages) and optional `QUESTER_PATH_DIR` (an absolute
//! folder served through the existing `FolderSource` registry, shadowing the
//! embedded release index). It uses the qualification profile in the fixture
//! table under a fixed deadline.
//! Quester live cells use Engine Q at its fixed 300 ms tick speed. Do not set
//! `QUESTER_TICK_MS` or use `::speed`.
//! `QUESTER_SUSTAIN_RUN=1` enables the existing `~energy` sustain.
//!
//! Persistence cells (`tests/quester_hint_live.rs`) add `QUESTER_LIVE_ACCOUNT`
//! with `QUESTER_LIVE_PASSWORD` (one fixed account instead of a minted one)
//! and `QUESTER_LIVE_KEEP_HOME=1` (the process `HOME` stays `~/.274bot`, so
//! what the host saved for the account in an earlier process is found again).
#![allow(
    dead_code,
    reason = "shared live runner across quester cells; each cell uses a subset, so expect would misfire"
)]

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use api::interact;
use api::selected::{Knowledge, RunKey};
use api::snapshot::{ActorKind, ActorTargetView, GameSnapshot, WorldTile};
use host::Pump;
use host_play::evidence_writer::{EvidenceJob, EvidenceRequest, EvidenceWriter, PngColor};
use host_play::{ProfileOptions, ScriptStartHandle, SharedClientTemplate};
use scenario::{RunnerStatus, Scenario, ScenarioRunner};
use script::native::{NativePhase, ScriptStatus, StatusValue};
use script::ScriptLifecycleReceipt;
use serde_json::{json, Map, Value};
use vault::{Profile, ProfileSettings};

const POLL_INTERVAL: Duration = Duration::from_millis(20);
/// Bound on the wait for the terminal capture file. The serial capture writer
/// synced the first capture of one run in 25.8 s and the end capture of
/// another in 14 s (QUESTER-DIALOGUE-OPEN r1 `qvca`), and wrote the path-smoke
/// and Jailer end captures 32 s and 11 s after their receipts under aubaine
/// I/O load (r2). The wait ends as soon as the file is durable. The run's
/// product result never depends on it: `run_result` in the receipt is fixed
/// before the wait starts.
const END_CAPTURE_BOUND: Duration = Duration::from_secs(300);
/// Time a Restart cell lets the step at the Stop stage run before stopping it,
/// so the Stop lands mid-step rather than on a selection boundary.
const MID_STEP: Duration = Duration::from_secs(4);
/// Grace after the scenario deadline for terminal bookkeeping.
const DEADLINE_GRACE: Duration = Duration::from_secs(15);
const PAIR_CANCELLED_REASON: &str = "pair cancelled; Stop and freshly Start both accounts";
/// The live teardown remains bounded even when a script or the server does not progress.
const CLEAN_LOGOUT_DEADLINE: Duration = Duration::from_secs(30);
/// Shared fixture escape waits the same content-authored combat lockout.
const COMBAT_LOGOUT_RETRY_TICKS: u32 = scenario::teardown::COMBAT_LOGOUT_TICKS;

#[derive(Debug)]
struct TeardownAccount {
    stop_ack_tick: Option<u32>,
    logout_attempt_ticks: Vec<u32>,
    no_logout_arm: bool,
}

#[derive(Debug)]
struct TeardownSchedule {
    accounts: Vec<TeardownAccount>,
    deadline: Instant,
}

impl TeardownSchedule {
    fn new(account_count: usize, deadline: Instant) -> Self {
        Self {
            accounts: (0..account_count)
                .map(|_| TeardownAccount {
                    stop_ack_tick: None,
                    logout_attempt_ticks: Vec::new(),
                    no_logout_arm: false,
                })
                .collect(),
            deadline,
        }
    }

    fn acknowledge_stop(&mut self, index: usize, tick: u32) {
        self.accounts[index].stop_ack_tick.get_or_insert(tick);
    }

    fn all_stopped(&self) -> bool {
        self.accounts
            .iter()
            .all(|account| account.stop_ack_tick.is_some())
    }

    fn initial_logout_due(&self, index: usize, now: Instant) -> bool {
        let account = &self.accounts[index];
        self.all_stopped()
            && now < self.deadline
            && !account.no_logout_arm
            && account.logout_attempt_ticks.is_empty()
    }

    fn retry_due(&self, index: usize, tick: u32, connected: bool, now: Instant) -> bool {
        if !self.all_stopped() || !connected || now >= self.deadline {
            return false;
        }
        self.accounts[index]
            .logout_attempt_ticks
            .last()
            .is_some_and(|last_tick| tick.saturating_sub(*last_tick) >= COMBAT_LOGOUT_RETRY_TICKS)
    }

    fn record_logout_attempt(&mut self, index: usize, tick: u32, now: Instant) -> bool {
        if !self.all_stopped() || now >= self.deadline || self.accounts[index].no_logout_arm {
            return false;
        }
        self.accounts[index].logout_attempt_ticks.push(tick);
        true
    }

    fn mark_no_logout_arm(&mut self, index: usize) {
        self.accounts[index].no_logout_arm = true;
    }

    fn stop_ack_tick(&self, index: usize) -> Option<u32> {
        self.accounts[index].stop_ack_tick
    }

    fn logout_attempt_ticks(&self, index: usize) -> &[u32] {
        &self.accounts[index].logout_attempt_ticks
    }

    fn deadline_expired(&self, now: Instant) -> bool {
        now >= self.deadline
    }

    fn complete(&self, disconnected: &[bool]) -> bool {
        disconnected.len() == self.accounts.len()
            && self.all_stopped()
            && self
                .accounts
                .iter()
                .zip(disconnected)
                .all(|(account, disconnected)| {
                    !account.logout_attempt_ticks.is_empty() && *disconnected
                })
    }
}

fn actor_frame_enabled(teardown_started: bool) -> bool {
    !teardown_started
}

/// What the cell proves after the fixture's Start.
#[derive(Debug, Clone)]
pub enum Mode {
    /// Pass when the published stage key is one of `expect` (a key equal to
    /// the Path's `colour.complete` also passes on the green quest tab).
    Stage { expect: Vec<String> },
    /// The family observer must prove completion or this exact named stop.
    ExpectedNamedStop { reason: String },
    /// Pass when the Path script completes without quest completion.
    PathTerminal,
    /// Pass on quest complete with one Start, zero deaths and no park.
    Clean,
    /// Stop mid-step once at each stage key (in order), Start again, complete.
    Restart { at: [String; 2] },
    /// Send one `~death` only while the named stage/step and owned combat are
    /// simultaneously observed; preserve that status and combat evidence.
    Death { at: String, step: String },
}

impl Mode {
    fn name(&self) -> &'static str {
        match self {
            Mode::Stage { .. } => "stage",
            Mode::ExpectedNamedStop { .. } => "expected-named-stop",
            Mode::PathTerminal => "path_terminal",
            Mode::Clean => "clean",
            Mode::Restart { .. } => "restart",
            Mode::Death { .. } => "death",
        }
    }
}
/// One role/stage target in a paired qualification mode. Role 0 is Phoenix
/// and role 1 is Black Arm.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct PairStage {
    pub role: usize,
    pub stage: String,
}

/// What a paired cell proves after both fixtures' initial Starts.
#[derive(Debug, Clone)]
pub enum PairMode {
    /// Each role settles into one of its own expected stage keys; a fresh
    /// terminal completion proof is also accepted.
    Stage { expect: [Vec<String>; 2] },
    /// Both accounts complete with real completion evidence, one Start and
    /// zero deaths apiece.
    Clean,
    /// Stop both accounts at each role/stage target, then Start both again.
    Restart { at: [PairStage; 2] },
    /// Stop both accounts while the named role is actively waiting in `step`,
    /// then Start both again. This is for short, owned pair barriers.
    RestartStep { at: PairStage, step: String },
    /// Stop at a named step and verify role-ordered item ownership across restart.
    RestartStepWithInventoryProof {
        at: PairStage,
        step: String,
        inventory_proof: PairInventoryProof,
    },
    /// Inject one death during named owned combat. Only the exact resulting
    /// pair-cancelled failure may trigger explicit Stop/Start recovery.
    Death { at: PairStage, step: String },
    /// Ordinary-role combat outside a reserved pair phase recovers in the same
    /// run. Both accounts finish without implicit peer Stop/Start or cancellation.
    DeathIndependent { at: PairStage, step: String },
}
#[derive(Debug, Clone)]
pub struct PairInventoryProof {
    /// The real item transferred by the named pair phase.
    pub item_id: i32,
    /// Expected role-ordered counts at the observed Stop boundary.
    pub boundary_counts: [i32; 2],
    /// Per-role maximum counts after explicit Start; prevents refarming or a second transfer.
    pub maximum_after_restart: [i32; 2],
    /// Item IDs that must be absent from both initial fixture inventories.
    pub initially_absent: Vec<i32>,
}

impl PairMode {
    fn name(&self) -> &'static str {
        match self {
            Self::Stage { .. } => "stage",
            Self::Clean => "clean",
            Self::Restart { .. } => "restart",
            Self::Death { .. } => "death",
            Self::DeathIndependent { .. } => "death-independent",
            Self::RestartStep { .. } | Self::RestartStepWithInventoryProof { .. } => "restart-step",
        }
    }
}

/// Two reciprocal real-account fixture scenarios, ordered Phoenix then
/// Black Arm. The mode belongs to the pair; each role's `Cell.mode` is unused.
pub struct PairCell {
    pub roles: [Cell; 2],
    pub mode: PairMode,
}

/// Proof hook run on the last pre-Start frame: returns the observed fixture
/// receipt (exact stats and kit) or refuses the Start.
pub type ObserveStart = Box<dyn FnMut(&GameSnapshot) -> Result<Value, String> + Send>;

/// A fixture-specific native Start through the same shared host pump.
pub type StartFamily = Box<
    dyn FnMut(
            &ScriptStartHandle,
            &str,
            &Arc<api::named_banks::NamedBankFacts>,
        ) -> Result<(), String>
        + Send,
>;

/// Return a receipt only after the family action produces fresh evidence.
pub type ObserveFamily = Box<
    dyn FnMut(
            &GameSnapshot,
            Option<&ScriptStatus>,
            Option<&ScriptLifecycleReceipt>,
        ) -> Result<Option<Value>, String>
        + Send,
>;
/// A receipt emitted by a cell-specific observer during live polling.
pub type ObserveTick = Box<dyn FnMut(&GameSnapshot, Option<&ScriptStatus>) -> Option<Value> + Send>;

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

/// The fixed account a persistence cell logs in as, when the caller set
/// both `QUESTER_LIVE_ACCOUNT` and `QUESTER_LIVE_PASSWORD`; one without the
/// other is a harness mistake.
fn fixed_live_account() -> Result<Option<(String, String)>, String> {
    let account = std::env::var("QUESTER_LIVE_ACCOUNT").ok();
    let password = std::env::var("QUESTER_LIVE_PASSWORD").ok();
    match (account, password) {
        (None, None) => Ok(None),
        (Some(account), Some(password)) => {
            if account.is_empty() || account.len() > 12 || password.is_empty() {
                return Err(
                    "QUESTER_LIVE_ACCOUNT must be 1-12 bytes and QUESTER_LIVE_PASSWORD non-empty"
                        .into(),
                );
            }
            Ok(Some((account, password)))
        }
        _ => Err("QUESTER_LIVE_ACCOUNT and QUESTER_LIVE_PASSWORD must be set together".into()),
    }
}

fn set_pair_name_prefixes(
    names: &mut [String],
    primary: &str,
    partner: &str,
) -> Result<(), String> {
    if names.len() != 2 || !(1..=4).contains(&partner.len()) {
        return Err("paired live names need two names and a 1–4-byte partner prefix".into());
    }
    // Keep the same low nonce digits in both names. If the second prefix is
    // longer, remove the same highest digits from each token before replacing it.
    let extra = partner.len().saturating_sub(primary.len());
    let end = primary.len() + extra;
    if names
        .iter()
        .any(|name| !name.starts_with(primary) || end >= name.len() || !name.is_char_boundary(end))
    {
        return Err("partner prefix does not fit the minted live names".into());
    }
    if extra > 0 {
        for name in names.iter_mut() {
            name.replace_range(primary.len()..end, "");
        }
    }
    names[1].replace_range(..primary.len(), partner);
    Ok(())
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
        profile: Some(std::env::var("BOT_SERVER_PROFILE").unwrap_or_else(|_| "local-289".into())),
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
    (status.phase == NativePhase::Blocked).then(|| format!("phase Blocked: {:?}", status.failure))
}

/// Match only the explicitly expected native park, not another blocked state.
pub fn has_named_stop(status: Option<&ScriptStatus>, reason: &str) -> bool {
    status.is_some_and(|status| {
        status.phase == NativePhase::Blocked
            && status.failure.as_ref().is_some_and(|failure| {
                failure.code.as_ref() == "parked" && failure.message.as_ref() == reason
            })
            && text(status, "block_reason") == Some(reason)
    })
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

fn card_complete(status: &ScriptStatus, quest: &str) -> bool {
    matches!(field(status, "quest"), Some(StatusValue::Quest(progress))
        if progress.quest.0.as_ref() == quest
            && progress.binding.0.strip_prefix("card:") == Some(quest)
            && progress.evidence.run == status.run
            && progress.complete == api::selected::Truth::True)
}

/// A capture handed to the background writer, awaiting its outcome. The PNG
/// path is fixed at submit so completion order (the writer is FIFO) keeps
/// the historic `captures` sequence.
struct PendingCapture {
    sequence: usize,
    label: String,
    png_path: PathBuf,
    job: EvidenceJob,
}

/// The slot-thread half of a quester capture: render (needs the client) into
/// native pixmap words and hand them plus the JSON receipt to the shared
/// background evidence writer. The hook polls the job and records the path
/// only once the files are durable.
fn submit_capture(
    client: &mut client::client::Client,
    writer: &EvidenceWriter,
    directory: &Path,
    sequence: usize,
    label: &str,
    mut receipt: Value,
) -> Result<PendingCapture, String> {
    let stem = format!("{sequence:02}-{label}");
    let render_start = Instant::now();
    let mut renderer = client::render::Renderer::new_prefer(client.config.lowmem, false);
    let was_draw = client.draw;
    client.set_draw(true);
    let frame = renderer.mainredraw(client);
    client.set_draw(was_draw);
    let client::render::backend::FrameOutput::PixMap(pixels) = frame else {
        return Err("capture needs BOT_CPU=1 (no CPU PixMap)".into());
    };
    println!(
        "capture-render site=quester {stem} {}x{} render_ms={:.1}",
        pixels.width,
        pixels.height,
        render_start.elapsed().as_secs_f64() * 1000.0,
    );
    receipt["frame"] = json!({"renderer": "real Client CpuPix3D framebuffer",
        "width": pixels.width, "height": pixels.height});
    let png_path = directory.join(format!("{stem}.png"));
    let job = writer.submit(EvidenceRequest {
        png_path: png_path.clone(),
        width: pixels.width as u32,
        height: pixels.height as u32,
        pixels: pixels.pixels,
        color: PngColor::Rgba,
        sidecar: Some(host_play::evidence_writer::EvidenceSidecar {
            path: directory.join(format!("{stem}.json")),
            receipt,
            patch_error: None,
        }),
    });
    Ok(PendingCapture {
        sequence,
        label: label.to_owned(),
        png_path,
        job,
    })
}

struct LiveCell {
    quest: &'static str,
    display: &'static str,
    label: String,
    scenario: Scenario,
    start_settings: Map<String, Value>,
    observe_start: Option<ObserveStart>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct OwnedCombat {
    player_index: usize,
    npc_index: usize,
    npc_type: Option<usize>,
    npc_name: Option<String>,
    npc_tile: WorldTile,
    npc_health: i32,
    npc_total_health: i32,
}

/// In-game tick intervals seen by the Path driver's snapshot polls.
#[derive(Default)]
pub(super) struct TickMean {
    last: Option<(u32, Instant)>,
    elapsed: Duration,
    intervals: u64,
}

impl TickMean {
    pub(super) fn observe(&mut self, snapshot: &GameSnapshot) {
        self.observe_at(snapshot.tick(), snapshot.ingame(), Instant::now());
    }

    pub(super) fn observe_at(&mut self, tick: u32, ingame: bool, now: Instant) {
        if !ingame {
            self.last = None;
            return;
        }
        match self.last {
            Some((previous, at)) if tick > previous => {
                self.elapsed += now.duration_since(at);
                self.intervals += u64::from(tick - previous);
                self.last = Some((tick, now));
            }
            Some((previous, _)) if tick < previous => {
                self.last = Some((tick, now));
            }
            None => {
                self.last = Some((tick, now));
            }
            _ => {}
        }
    }

    pub(super) fn mean_ms(&self) -> Option<f64> {
        if self.intervals == 0 {
            None
        } else {
            Some(self.elapsed.as_secs_f64() * 1000.0 / self.intervals as f64)
        }
    }
}

/// Receive-time measurement only. It never controls content waits or deadlines.
#[derive(Default)]
struct ObservedTickRate {
    last: Option<(u32, Instant)>,
    elapsed: Duration,
    ticks: u64,
    samples: u64,
    minimum_ms: Option<f64>,
    maximum_ms: Option<f64>,
    counter_resets: u64,
}

impl ObservedTickRate {
    fn observe(&mut self, tick: u32, at: Instant) {
        if let Some((previous_tick, previous_at)) = self.last {
            if tick == previous_tick {
                return;
            }
            if let Some(delta) = tick.checked_sub(previous_tick) {
                let elapsed = at.saturating_duration_since(previous_at);
                let milliseconds = elapsed.as_secs_f64() * 1000.0 / f64::from(delta);
                self.elapsed += elapsed;
                self.ticks += u64::from(delta);
                self.samples += 1;
                self.minimum_ms = Some(
                    self.minimum_ms
                        .map_or(milliseconds, |value| value.min(milliseconds)),
                );
                self.maximum_ms = Some(
                    self.maximum_ms
                        .map_or(milliseconds, |value| value.max(milliseconds)),
                );
            } else {
                self.counter_resets += 1;
            }
        }
        self.last = Some((tick, at));
    }

    fn receipt(&self) -> Value {
        json!({
            "source": "monotonic PLAYER_INFO receive observations after Start",
            "interval_samples": self.samples,
            "sampled_game_ticks": self.ticks,
            "elapsed_ms": self.elapsed.as_secs_f64() * 1000.0,
            "mean_tick_ms": (self.ticks > 0)
                .then(|| self.elapsed.as_secs_f64() * 1000.0 / self.ticks as f64),
            "minimum_tick_ms": self.minimum_ms,
            "maximum_tick_ms": self.maximum_ms,
            "counter_resets": self.counter_resets,
        })
    }
}

struct ActorShared {
    role: Option<usize>,
    quest: &'static str,
    display: &'static str,
    label: String,
    account: String,
    directory: PathBuf,
    start_settings: Map<String, Value>,
    runner: ScenarioRunner,
    snapshot: GameSnapshot,
    latest_status: Option<ScriptStatus>,
    pump: Pump,
    logout: Option<scenario::teardown::CombatLogout>,
    logout_retry_ready: bool,
    logout_error: Option<String>,
    start_handle: Option<ScriptStartHandle>,
    named_banks: Option<Arc<api::named_banks::NamedBankFacts>>,
    start_family: Option<StartFamily>,
    observe_start: Option<ObserveStart>,
    start_receipt: Option<Value>,
    observe_tick: Option<ObserveTick>,
    tick_observations: Vec<Value>,
    tick_mean: TickMean,
    /// `{stage, tick}` at each observed stage change, in observed game ticks.
    stage_ticks: Vec<Value>,
    observed_tick_rate: ObservedTickRate,
    starts: u32,
    stops: u32,
    restart_due: bool,
    /// QUESTER-READ-KEEPS-STEP-R2: summon one random event while a step runs
    /// (`QUESTER_MACRO_EVENT=<1-8>`, `QUESTER_MACRO_EVENT_STEP=<step id>`).
    macro_event: Option<MacroEventPlan>,
    death_due: bool,
    death_sent: bool,
    death_seen: bool,
    death_status: Option<Value>,
    death_target: Option<(String, String)>,
    death_combat: Option<OwnedCombat>,
    death_chat_baseline: Option<i32>,
    death_messages: HashSet<i32>,
    death_injections: u32,
    death_run: Option<RunKey>,
    deaths_before_run: i64,
    deaths_run_baseline: i64,
    deaths_run_peak: i64,
    deaths_total: i64,
    pair_cancelled: bool,
    owned_card_complete: bool,
    stages_seen: Vec<String>,
    transitions: Vec<Value>,
    track_steps: bool,
    last_step_observation: Option<(RunKey, String, String)>,
    step_observations: Vec<Value>,
    pending_captures: Vec<(String, Value)>,
    captures: Vec<PathBuf>,
    /// Monotonic capture sequence assigned at submit; the FIFO writer
    /// completes in order, so stems match the old synchronous numbering.
    capture_seq: usize,
    /// Captures handed to the background writer, awaiting their outcomes.
    pending_jobs: Vec<PendingCapture>,
    /// One shared background evidence writer per run (all roles).
    evidence: Arc<EvidenceWriter>,
    last_tile: Option<WorldTile>,
    error: Option<String>,
    capture_error: Option<String>,
    end_captured: bool,
}

/// The callback owns each actor's client/snapshot, while this lock serializes
/// their runner state with the shared poll loop.
struct Shared {
    actors: Vec<ActorShared>,
    start_handles_ready: bool,
    initial_start_released: bool,
    teardown_started: bool,
    inventory_proof: Option<PairInventoryProof>,
    inventory_initial_absence: [bool; 2],
    inventory_boundary: Option<Value>,
    inventory_restart_samples: Vec<Value>,
    inventory_last_counts: Option<[i32; 2]>,
    inventory_violation: Option<String>,
}

enum RunPlan {
    Single(Mode),
    Pair(PairMode),
}

impl RunPlan {
    fn name(&self) -> &'static str {
        match self {
            Self::Single(mode) => mode.name(),
            Self::Pair(mode) => mode.name(),
        }
    }

    fn is_pair(&self) -> bool {
        matches!(self, Self::Pair(_))
    }

    fn expected_named_stop(&self) -> Option<&str> {
        match self {
            Self::Single(Mode::ExpectedNamedStop { reason }) => Some(reason),
            _ => None,
        }
    }

    fn restart_targets(&self) -> Option<Vec<PairStage>> {
        match self {
            Self::Single(Mode::Restart { at }) => Some(
                at.iter()
                    .cloned()
                    .map(|stage| PairStage { role: 0, stage })
                    .collect(),
            ),
            Self::Pair(PairMode::Restart { at }) => Some(at.to_vec()),
            Self::Pair(PairMode::RestartStep { at, .. })
            | Self::Pair(PairMode::RestartStepWithInventoryProof { at, .. })
            | Self::Pair(PairMode::Death { at, .. }) => Some(vec![at.clone()]),
            _ => None,
        }
    }

    fn restart_step_target(&self) -> Option<(&PairStage, &str, Option<&PairInventoryProof>)> {
        match self {
            Self::Pair(PairMode::RestartStep { at, step }) => Some((at, step, None)),
            Self::Pair(PairMode::RestartStepWithInventoryProof {
                at,
                step,
                inventory_proof,
            }) => Some((at, step, Some(inventory_proof))),
            _ => None,
        }
    }

    fn inventory_proof(&self) -> Option<&PairInventoryProof> {
        match self {
            Self::Pair(PairMode::RestartStepWithInventoryProof {
                inventory_proof, ..
            }) => Some(inventory_proof),
            _ => None,
        }
    }

    fn death_target(&self) -> Option<(PairStage, String)> {
        match self {
            Self::Single(Mode::Death { at, step }) => Some((
                PairStage {
                    role: 0,
                    stage: at.clone(),
                },
                step.clone(),
            )),
            Self::Pair(PairMode::Death { at, step })
            | Self::Pair(PairMode::DeathIndependent { at, step }) => {
                Some((at.clone(), step.clone()))
            }
            _ => None,
        }
    }

    fn pair_death_target(&self) -> Option<&PairStage> {
        match self {
            Self::Pair(PairMode::Death { at, .. }) => Some(at),
            _ => None,
        }
    }
}

struct PollActor {
    status: Option<Arc<ScriptStatus>>,
    run_state: script::RunState,
    lifecycle: Option<script::ScriptLifecycleReceipt>,
    last_error: Option<String>,
    runner_status: RunnerStatus,
    done: bool,
    card_complete: bool,
    terminal: bool,
}

enum RestartPhase {
    Seeking,
    MidStep {
        target: usize,
        since: Instant,
    },
    Stopping {
        target: PairStage,
        round: u32,
        target_status: Value,
        statuses_at_stop: Vec<Value>,
    },
    Starting {
        round: u32,
    },
}

fn validate_pair_gangs(
    phoenix: &Map<String, Value>,
    black_arm: &Map<String, Value>,
) -> Result<(), String> {
    let phoenix_gang = phoenix.get("gang").and_then(Value::as_str);
    let black_arm_gang = black_arm.get("gang").and_then(Value::as_str);
    if phoenix_gang != Some("phoenix") || black_arm_gang != Some("blackarm") {
        return Err(
            "paired Quester roles must explicitly select Phoenix and Black Arm gangs".into(),
        );
    }
    Ok(())
}

fn valid_inventory_proof(proof: &PairInventoryProof) -> bool {
    let unique_absent: HashSet<_> = proof.initially_absent.iter().copied().collect();
    proof.item_id > 0
        && proof.boundary_counts.iter().all(|count| *count >= 0)
        && proof
            .boundary_counts
            .iter()
            .map(|count| i64::from(*count))
            .sum::<i64>()
            > 0
        && proof
            .maximum_after_restart
            .iter()
            .zip(proof.boundary_counts)
            .all(|(maximum, boundary)| *maximum >= boundary)
        && proof.initially_absent.contains(&proof.item_id)
        && proof.initially_absent.iter().all(|item_id| *item_id > 0)
        && unique_absent.len() == proof.initially_absent.len()
}

fn validate_pair_mode(mode: &PairMode) -> Result<(), String> {
    let valid_target = |target: &PairStage| target.role < 2 && !target.stage.trim().is_empty();
    match mode {
        PairMode::Restart { at } if !at.iter().all(valid_target) => {
            Err("paired Restart targets need role 0/1 and a non-empty stage".into())
        }
        PairMode::RestartStep { at, step } if !valid_target(at) || step.trim().is_empty() => {
            Err("paired named-step Restart needs role 0/1 and non-empty stage/step".into())
        }
        PairMode::RestartStepWithInventoryProof {
            at,
            step,
            inventory_proof,
        } if !valid_target(at)
            || step.trim().is_empty()
            || !valid_inventory_proof(inventory_proof) =>
        {
            Err("paired named-step Restart needs role 0/1 and valid stage/step/proof".into())
        }
        PairMode::Death { at, step } | PairMode::DeathIndependent { at, step }
            if !valid_target(at) || step.trim().is_empty() =>
        {
            Err("paired Death needs role 0/1 and non-empty stage/step".into())
        }
        _ => Ok(()),
    }
}

fn stage_or_terminal(
    expected: &[String],
    current: Option<&str>,
    transitioned: bool,
    terminal: bool,
) -> bool {
    terminal
        || (transitioned && current.is_some_and(|stage| expected.iter().any(|key| key == stage)))
}

fn path_terminal_proven(idle: bool, lifecycle_completed: bool) -> bool {
    idle && lifecycle_completed
}

fn completion_proven(
    runner_passed: bool,
    progress_complete: bool,
    idle: bool,
    lifecycle_completed: bool,
) -> bool {
    runner_passed && progress_complete && path_terminal_proven(idle, lifecycle_completed)
}

fn step_matches(status: &ScriptStatus, step: &str) -> bool {
    text(status, "step_id") == Some(step) || text(status, "child_step_id") == Some(step)
}
fn active_owned_combat(snapshot: &GameSnapshot) -> Option<OwnedCombat> {
    let player = snapshot.local_player()?;
    if !player.player.actor.in_combat {
        return None;
    }
    let ActorTargetView {
        kind: ActorKind::Npc,
        index: npc_index,
    } = player.player.actor.target?
    else {
        return None;
    };
    let npc = snapshot.npcs().iter().find(|npc| npc.index == npc_index)?;
    let targets_player = npc.target
        == Some(ActorTargetView {
            kind: ActorKind::Player,
            index: player.player.index,
        });
    (npc.in_combat && targets_player && npc.health > 0).then(|| OwnedCombat {
        player_index: player.player.index,
        npc_index: npc.index,
        npc_type: npc.r#type,
        npc_name: npc.name.clone(),
        npc_tile: npc.tile,
        npc_health: npc.health,
        npc_total_health: npc.total_health,
    })
}

fn owned_combat_json(combat: &OwnedCombat) -> Value {
    json!({
        "player": {
            "index": combat.player_index,
            "in_combat": true,
            "target": {"kind": "Npc", "index": combat.npc_index},
        },
        "npc": {
            "index": combat.npc_index,
            "type": combat.npc_type,
            "name": combat.npc_name,
            "tile": [combat.npc_tile.x, combat.npc_tile.z, combat.npc_tile.level],
            "health": combat.npc_health,
            "total_health": combat.npc_total_health,
            "in_combat": true,
            "target": {"kind": "Player", "index": combat.player_index},
        },
    })
}

fn inventory_receipt(snapshot: &GameSnapshot) -> Value {
    json!(snapshot
        .inventory()
        .iter()
        .filter(|item| item.count > 0)
        .map(|item| [item.def.id, item.count])
        .collect::<Vec<_>>())
}
fn inventory_count(snapshot: &GameSnapshot, item_id: i32) -> i32 {
    snapshot
        .inventory()
        .iter()
        .filter(|item| item.def.id == item_id)
        .map(|item| item.count)
        .fold(0, i32::saturating_add)
}

fn observe_death_count(actor: &mut ActorShared, status: &ScriptStatus) {
    let count = integer(status, "deaths").unwrap_or(0).max(0);
    if actor.death_run != Some(status.run) {
        actor.death_run = Some(status.run);
        actor.deaths_before_run = actor.deaths_total;
        actor.deaths_run_baseline = count.min(actor.deaths_before_run);
        actor.deaths_run_peak = count;
    } else {
        actor.deaths_run_peak = actor.deaths_run_peak.max(count);
    }
    actor.deaths_total = actor.deaths_before_run.saturating_add(
        actor
            .deaths_run_peak
            .saturating_sub(actor.deaths_run_baseline),
    );
}

fn active_waiting_step(status: &ScriptStatus, step: &str) -> bool {
    step_matches(status, step)
        && text(status, "action_state") == Some("waiting")
        && text(status, "waiting_for").is_some_and(|phase| !phase.is_empty())
}

fn pair_cancelled(status: &ScriptStatus) -> bool {
    status.phase == NativePhase::Blocked
        && status.failure.as_ref().is_some_and(|failure| {
            failure.code.as_ref() == "parked" && failure.message.as_ref() == PAIR_CANCELLED_REASON
        })
}

fn capture_receipt(
    actor: &ActorShared,
    label: &str,
    status: Option<&ScriptStatus>,
    mut extra: Value,
) -> Value {
    extra["label"] = json!(label);
    extra["tick"] = json!(actor.snapshot.tick());
    extra["role"] = json!(actor.role);
    extra["account"] = json!(actor.account);
    extra["status"] = status.map(status_json).unwrap_or(Value::Null);
    extra["fixture_receipt"] = actor.start_receipt.clone().unwrap_or(Value::Null);
    extra["death_status"] = actor.death_status.clone().unwrap_or(Value::Null);
    extra["death_combat_ownership"] = actor
        .death_combat
        .as_ref()
        .map(owned_combat_json)
        .unwrap_or(Value::Null);
    extra
}

fn append_status(path: &Path, status: &ScriptStatus, tick: u32) -> Result<Value, String> {
    use std::io::Write;
    let logged = status_json(status);
    let line = json!({"t_ms": epoch_nanos()? / 1_000_000, "tick": tick, "status": logged});
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|error| error.to_string())?;
    writeln!(file, "{line}").map_err(|error| error.to_string())?;
    Ok(line["status"].clone())
}

#[derive(Clone, Copy)]
enum FamilyBlockedPolicy {
    Reject,
    Observe,
}

fn family_parked_error(status: &ScriptStatus, policy: FamilyBlockedPolicy) -> Option<String> {
    if matches!(policy, FamilyBlockedPolicy::Observe)
        && status.phase == NativePhase::Blocked
        && status.failure.is_some()
    {
        return None;
    }
    parked(status)
}

pub fn run(cell: Cell) -> Result<Value, String> {
    run_inner(cell, None, None, None, false, FamilyBlockedPolicy::Reject)
}

/// Run the generic Path live cell with shared world-wide tick-speed controls.
pub fn run_quester_path(cell: Cell) -> Result<Value, String> {
    run_inner(cell, None, None, None, true, FamilyBlockedPolicy::Reject)
}

/// Run an inline native family fixture without a second launcher or capture driver.
pub fn run_family(cell: Cell, start: StartFamily, observe: ObserveFamily) -> Result<Value, String> {
    run_inner(
        cell,
        Some(start),
        Some(observe),
        None,
        false,
        FamilyBlockedPolicy::Reject,
    )
}

/// Let an explicit negative-family oracle inspect native Blocked with a failure.
/// The oracle must prove its exact expected refusal and reject changed state.
pub fn run_family_expected_blocked(
    cell: Cell,
    start: StartFamily,
    observe: ObserveFamily,
) -> Result<Value, String> {
    run_inner(
        cell,
        Some(start),
        Some(observe),
        None,
        false,
        FamilyBlockedPolicy::Observe,
    )
}

/// Run the Path with a cell-specific success oracle and polling observations.
pub fn run_observed(
    cell: Cell,
    observe: ObserveFamily,
    observe_tick: ObserveTick,
) -> Result<Value, String> {
    run_inner(
        cell,
        None,
        Some(observe),
        Some(observe_tick),
        false,
        FamilyBlockedPolicy::Reject,
    )
}

fn run_inner(
    cell: Cell,
    start_family: Option<StartFamily>,
    observe_family: Option<ObserveFamily>,
    observe_tick: Option<ObserveTick>,
    quester_path: bool,
    blocked_policy: FamilyBlockedPolicy,
) -> Result<Value, String> {
    if let Mode::ExpectedNamedStop { reason } = &cell.mode {
        if reason.trim().is_empty() || observe_family.is_none() {
            return Err(
                "expected named stop requires a non-empty reason and family observer".into(),
            );
        }
    }
    let Cell {
        quest,
        display,
        label,
        scenario,
        start_settings,
        mode,
        observe_start,
    } = cell;
    run_cells(
        vec![LiveCell {
            quest,
            display,
            label,
            scenario,
            start_settings,
            observe_start,
        }],
        RunPlan::Single(mode),
        start_family,
        observe_family,
        observe_tick,
        quester_path,
        blocked_policy,
    )
}

/// Run Phoenix role 0 and Black Arm role 1 as two real Quester accounts in
/// one local Play/template/world. Both scenarios finish their pre-Start
/// fixtures before either account can issue its first native Start.
pub fn run_pair(pair: PairCell) -> Result<Value, String> {
    let PairCell { roles, mode } = pair;
    let cells: Vec<LiveCell> = roles
        .into_iter()
        .map(|cell| {
            let Cell {
                quest,
                display,
                label,
                scenario,
                start_settings,
                observe_start,
                ..
            } = cell;
            LiveCell {
                quest,
                display,
                label,
                scenario,
                start_settings,
                observe_start,
            }
        })
        .collect();
    run_cells(
        cells,
        RunPlan::Pair(mode),
        None,
        None,
        None,
        false,
        FamilyBlockedPolicy::Reject,
    )
}

fn has_native_start(script: Option<&str>, family_override: bool) -> bool {
    script == Some("Quester") || (family_override && script.is_some_and(|name| !name.is_empty()))
}

fn run_cells(
    mut cells: Vec<LiveCell>,
    mode: RunPlan,
    mut start_family: Option<StartFamily>,
    mut observe_family: Option<ObserveFamily>,
    mut observe_tick: Option<ObserveTick>,
    quester_path: bool,
    blocked_policy: FamilyBlockedPolicy,
) -> Result<Value, String> {
    #[cfg(feature = "js")]
    script_js::install_cards().map_err(|error| error.to_string())?;
    if cells.len() != if mode.is_pair() { 2 } else { 1 } {
        return Err("Quester live run has the wrong number of roles".into());
    }
    if let RunPlan::Pair(pair_mode) = &mode {
        validate_pair_gangs(&cells[0].start_settings, &cells[1].start_settings)?;
        validate_pair_mode(pair_mode)?;
        if cells[0].quest != cells[1].quest || cells[0].display != cells[1].display {
            return Err("paired Quester roles must use the same quest identity".into());
        }
    }
    for cell in &cells {
        if !has_native_start(cell.scenario.settings.start_script, start_family.is_some()) {
            return Err(format!("{} has no supported native Start", cell.label));
        }
        if cell.scenario.seed.profiles.len() != 1 {
            return Err(format!(
                "{} must seed exactly its own account in a Quester live cell",
                cell.label
            ));
        }
    }
    let tick_speed_requested = std::env::var_os("QUESTER_TICK_MS").is_some();
    let apply_fast_settings = quester_path || tick_speed_requested;
    if apply_fast_settings && (mode.is_pair() || cells.len() != 1) {
        return Err("QUESTER_TICK_MS requires a single-account Quester live cell".into());
    }
    let fast_settings = if apply_fast_settings {
        scenario::quester::QuesterFastSettings::from_env()?
    } else {
        scenario::quester::QuesterFastSettings {
            tick_ms: None,
            sustain_run: false,
        }
    };
    let restore_world_speed = std::env::var("QUESTER_SPEED_RESTORE").as_deref() != Ok("0");
    if quester_path && (mode.is_pair() || cells.len() != 1) {
        return Err("QUESTER_TICK_MS requires a single-account quester_path cell".into());
    }
    if matches!(&mode, RunPlan::Single(Mode::PathTerminal)) && !quester_path {
        return Err("PathTerminal requires a quester_path cell".into());
    }
    if std::env::var("LIVE").as_deref() != Ok("1") {
        return Err("Quester live cells require LIVE=1".into());
    }
    if std::env::var("BOT_CPU").as_deref() != Ok("1") {
        return Err("Quester live cells require BOT_CPU=1 for PNG evidence".into());
    }
    let primary_name_prefix = required("BOT_LIVE_NAME_PREFIX")?;
    let run_label = if mode.is_pair() {
        format!("{}-{}", cells[0].label, cells[1].label)
    } else {
        cells[0].label.clone()
    };
    // `QUESTER_LIVE_KEEP_HOME=1` keeps the process `HOME` (an isohome
    // throwaway the caller owns across runs) as `~/.274bot`, so per-account
    // state the host persists there outlives this process; the default is
    // a fresh isolated home for this run alone.
    let keep_home = std::env::var("QUESTER_LIVE_KEEP_HOME").as_deref() == Ok("1");
    let isolated = (!keep_home).then(|| script::IsolatedEnv::enter(&run_label));
    let home = match &isolated {
        Some(isolated) => isolated.home.clone(),
        None => PathBuf::from(required("HOME")?),
    };
    let evidence_root = required_path("LIVE_EVIDENCE_DIR")?;
    if evidence_root.starts_with(&home) {
        return Err("LIVE_EVIDENCE_DIR must be outside the throwaway HOME".into());
    }
    let catalog_root = required_path("RS2B0T")?;
    if let Some(isolated) = &isolated {
        isolated.set_rs2b0t(&catalog_root);
    }
    api::hostlog::set_debug(true);
    let campaign_tick_limit = std::env::var("QUEST_CAMPAIGN_TICKS")
        .ok()
        .map(|raw| {
            raw.parse::<u32>()
                .ok()
                .filter(|ticks| *ticks > 0)
                .ok_or_else(|| {
                    "QUEST_CAMPAIGN_TICKS must be a positive observed-tick count".to_owned()
                })
        })
        .transpose()?;

    for cell in &mut cells {
        cell.scenario.settings.nav.engine_speed_ms = None;
    }
    if apply_fast_settings {
        scenario::quester::apply_quester_fast_settings(&mut cells[0].scenario, fast_settings);
    }
    let scenario_deadline = cells
        .iter()
        .map(|cell| cell.scenario.settings.deadline)
        .max()
        .unwrap_or_default();
    let deadline = Instant::now() + scenario_deadline + DEADLINE_GRACE;
    let temp = TempRoot::new(&run_label)?;
    let (profile, template) = selected_profile(&temp.0)?;
    // A persistence cell (`QUESTER_LIVE_ACCOUNT` + `QUESTER_LIVE_PASSWORD`)
    // logs one fixed account in across processes so what the host saved
    // for it under `HOME` — the bank hint — is found again; everything else
    // mints a fresh account per run.
    let fixed_account = fixed_live_account()?;
    if fixed_account.is_some() && mode.is_pair() {
        return Err("QUESTER_LIVE_ACCOUNT applies to single-account cells only".into());
    }
    let mut names = match &fixed_account {
        Some((account, _)) => vec![account.clone()],
        None => host_play::mint_live_names(cells.len()),
    };
    if names.len() != cells.len() {
        return Err("could not mint every Quester live account".into());
    }
    if mode.is_pair() {
        set_pair_name_prefixes(
            &mut names,
            &primary_name_prefix,
            &required("BOT_LIVE_PARTNER_NAME_PREFIX")?,
        )?;
    }
    let entries = match fixed_account {
        Some((account, password)) => vec![(account, password)],
        None => host_play::mint_live_entries(&names),
    };
    if entries.len() != names.len() {
        return Err("could not mint every Quester live credential".into());
    }
    if mode.is_pair() {
        cells[0]
            .start_settings
            .insert("partner_account".into(), json!(names[1]));
        cells[1]
            .start_settings
            .insert("partner_account".into(), json!(names[0]));
    }

    let mut directories = Vec::with_capacity(cells.len());
    let mut status_logs = Vec::with_capacity(cells.len());
    for (index, cell) in cells.iter().enumerate() {
        let directory = evidence_root.join(cell.quest).join(format!(
            "{}-{}-{}",
            cell.label,
            names[index],
            epoch_nanos()? / 1_000_000_000
        ));
        std::fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
        status_logs.push(directory.join("status.jsonl"));
        directories.push(directory);
    }

    let track_steps = campaign_tick_limit.is_some()
        || mode.death_target().is_some()
        || mode.restart_step_target().is_some();
    let evidence = Arc::new(EvidenceWriter::new(
        host_play::evidence_writer::DEFAULT_QUEUE_BOUND,
    ));
    let mut actors = Vec::with_capacity(cells.len());
    for (index, cell) in cells.into_iter().enumerate() {
        let mut runner = ScenarioRunner::with_world(cell.scenario, template.world());
        runner.set_tick_speed_endpoint(profile.client().game_host(), profile.client().game_port());
        runner.set_restore_world_speed(restore_world_speed);
        runner.set_map_members(profile.map_members());
        runner.set_live_names(&[names[index].clone()]);
        runner.set_shot_sink(Box::new(|_, _| {}));
        actors.push(ActorShared {
            role: mode.is_pair().then_some(index),
            quest: cell.quest,
            display: cell.display,
            label: cell.label,
            account: names[index].clone(),
            directory: directories[index].clone(),
            start_settings: cell.start_settings,
            runner,
            snapshot: GameSnapshot::new(),
            latest_status: None,
            pump: Pump::new(),
            logout: None,
            logout_retry_ready: false,
            logout_error: None,
            start_handle: None,
            named_banks: None,
            start_family: start_family.take(),
            observe_start: cell.observe_start,
            start_receipt: None,
            observe_tick: observe_tick.take(),
            tick_observations: Vec::new(),
            tick_mean: TickMean::default(),
            stage_ticks: Vec::new(),
            observed_tick_rate: ObservedTickRate::default(),
            starts: 0,
            stops: 0,
            restart_due: false,
            macro_event: macro_event_plan_from_env(),
            death_due: false,
            death_sent: false,
            death_seen: false,
            death_status: None,
            death_target: None,
            death_combat: None,
            death_chat_baseline: None,
            death_messages: HashSet::new(),
            death_injections: 0,
            death_run: None,
            deaths_before_run: 0,
            deaths_run_baseline: 0,
            deaths_run_peak: 0,
            deaths_total: 0,
            pair_cancelled: false,
            owned_card_complete: false,
            stages_seen: Vec::new(),
            transitions: Vec::new(),
            track_steps,
            last_step_observation: None,
            step_observations: Vec::new(),
            pending_captures: Vec::new(),
            captures: Vec::new(),
            capture_seq: 0,
            pending_jobs: Vec::new(),
            evidence: Arc::clone(&evidence),
            last_tile: None,
            error: None,
            capture_error: None,
            end_captured: false,
        });
    }
    let shared = Arc::new(Mutex::new(Shared {
        actors,
        start_handles_ready: false,
        initial_start_released: false,
        teardown_started: false,
        inventory_proof: mode.inventory_proof().cloned(),
        inventory_initial_absence: [false; 2],
        inventory_boundary: None,
        inventory_restart_samples: Vec::new(),
        inventory_last_counts: None,
        inventory_violation: None,
    }));
    let frame_shared = Arc::clone(&shared);
    let frame_accounts = names.clone();
    let mut play = host_play::run_with_template(
        Arc::clone(&template),
        true,
        vec![],
        |_| (None, None),
        move |client, username, hold| {
            let Some(index) = frame_accounts
                .iter()
                .position(|account| account == username)
            else {
                return;
            };
            let mut guard = frame_shared.lock().unwrap();
            let initial_ready = guard.start_handles_ready
                && guard
                    .actors
                    .iter()
                    .all(|actor| actor.starts > 0 || actor.runner.on_start_script());
            if initial_ready {
                guard.initial_start_released = true;
            }
            let initial_start_released = guard.initial_start_released;
            let first_start =
                guard.actors[index].runner.on_start_script() && guard.actors[index].starts == 0;
            let inventory_proof = guard.inventory_proof.clone();
            let frame_state = &mut *guard;
            let (actors, initial_absence) = (
                &mut frame_state.actors,
                &mut frame_state.inventory_initial_absence,
            );
            let actor = &mut actors[index];
            let drain = actor.pump.drain_client(client);
            host::publish_snapshot(&mut actor.snapshot, client, drain);
            let game_tick = actor.snapshot.tick();
            if actor.starts > 0
                && actor.observed_tick_rate.last.map(|(tick, _)| tick) != Some(game_tick)
            {
                actor.observed_tick_rate.observe(game_tick, Instant::now());
            }
            actor.last_tile = observed_tile(&actor.snapshot);
            if !actor_frame_enabled(frame_state.teardown_started) {
                if let Some(logout) = &mut actor.logout {
                    actor.logout_retry_ready = logout.allows_retry(&actor.snapshot, Instant::now());
                    match logout.send_escape(client, Instant::now()) {
                        Ok(true) => eprintln!(
                            "[quester-live] logout account-only safe tele account={username}"
                        ),
                        Ok(false) => {}
                        Err(error) => actor.logout_error = Some(error),
                    }
                }
                return;
            }

            if first_start {
                if let Some(proof) = inventory_proof.as_ref() {
                    let counts = proof
                        .initially_absent
                        .iter()
                        .map(|item_id| (*item_id, inventory_count(&actor.snapshot, *item_id)))
                        .collect::<Vec<_>>();
                    let all_absent = counts.iter().all(|(_, count)| *count == 0);
                    initial_absence[index] = all_absent;
                    if !all_absent {
                        actor.error = Some(format!(
                            "initial fixture seeded a forbidden shield/certificate item: {counts:?}"
                        ));
                        return;
                    }
                    actor.transitions.push(json!({
                        "event": "initial-inventory-proof",
                        "item_counts": counts,
                        "all_absent": true,
                    }));
                }
            }

            let restart_start = actor.restart_due;
            if (first_start && initial_start_released) || restart_start {
                if first_start {
                    if let Some(observe) = actor.observe_start.as_mut() {
                        match observe(&actor.snapshot) {
                            Ok(receipt) => actor.start_receipt = Some(receipt),
                            Err(error) => {
                                actor.error = Some(format!("pre-Start fixture proof: {error}"));
                                return;
                            }
                        }
                    }
                }
                let result = match (
                    &actor.start_handle,
                    &actor.named_banks,
                    &mut actor.start_family,
                ) {
                    (Some(handle), Some(banks), Some(start)) => {
                        start(handle, &actor.account, banks)
                    }
                    (Some(handle), _, None) => handle.start_compiled(
                        &actor.account,
                        script::CompiledId("Quester"),
                        actor.start_settings.clone(),
                    ),
                    _ => Err("Start before native fixture handles install".to_owned()),
                };
                match result {
                    Ok(()) => {
                        actor.starts += 1;
                        actor.restart_due = false;
                        actor.transitions.push(json!({
                            "event": "start",
                            "start": actor.starts,
                            "role": actor.role,
                            "account": actor.account,
                            "accepted": true,
                            "explicit": true,
                            "fixture_replayed": false,
                            "profile_reapplied": false,
                            "settings_reused": true,
                            "settings": actor.start_settings,
                            "authorized_death_injections": actor.death_injections,
                        }));
                    }
                    Err(error) if restart_start => actor.transitions.push(json!({
                        "event": "start-retry",
                        "role": actor.role,
                        "account": actor.account,
                        "error": error,
                    })),
                    Err(error) => actor.error = Some(format!("Quester Start failed: {error}")),
                }
            }
            let waiting_for_peer_setup =
                actor.runner.on_start_script() && actor.starts == 0 && !initial_start_released;
            if !waiting_for_peer_setup
                && !matches!(
                    actor.runner.status(),
                    RunnerStatus::Passed | RunnerStatus::Failed(_)
                )
            {
                actor.runner.tick_with_hold(client, hold.hold);
            }
            if let Some(plan) = actor.macro_event.as_mut() {
                // The engine cheat `::macro_event <n>` runs the same
                // `~macro_event_general_spawn` as the 500-tick login timer
                // (`cheat_macro_event.rs2`, `macro_events.rs2:2-12`), and
                // AGENTS.md allows it for random-event tests.
                if !plan.sent && !hold.hold {
                    if let Some(status) = actor.latest_status.as_ref() {
                        if status.phase == NativePhase::Working && step_matches(status, &plan.step)
                        {
                            let command = format!("~macro_event {}", plan.event);
                            if interact::cheat(client, &command).is_sent() {
                                plan.sent = true;
                                plan.before =
                                    actor.snapshot.npcs().iter().map(|npc| npc.index).collect();
                                plan.chat_baseline = actor
                                    .snapshot
                                    .chat_lines()
                                    .iter()
                                    .map(|line| line.sequence)
                                    .max()
                                    .unwrap_or(0);
                                actor.transitions.push(json!({
                                    "event": "macro-event-injected",
                                    "account": actor.account,
                                    "command": command,
                                    "step": plan.step,
                                    "observed_tick": actor.snapshot.tick(),
                                    "tile": actor.last_tile.map(|tile| [tile.x, tile.z, tile.level]),
                                    "status": status_json(status),
                                    "authorized_injection": "~macro_event (the engine's debugproc char)",
                                }));
                            }
                        }
                    }
                }
            }
            if let Some(plan) = actor.macro_event.as_mut().filter(|plan| plan.sent) {
                let tick = actor.snapshot.tick();
                if !plan.npc_seen {
                    if let Some(npc) = actor
                        .snapshot
                        .npcs()
                        .iter()
                        .find(|npc| !plan.before.contains(&npc.index) && npc.distance <= 8)
                    {
                        plan.npc_seen = true;
                        actor.transitions.push(json!({
                            "event": "macro-event-npc",
                            "observed_tick": tick,
                            "npc": npc.name,
                            "type": npc.r#type,
                            "index": npc.index,
                            "tile": [npc.tile.x, npc.tile.z, npc.tile.level],
                        }));
                    }
                }
                if plan.chat_logged < 8 {
                    let baseline = plan.chat_baseline;
                    let fresh = actor
                        .snapshot
                        .chat_lines()
                        .iter()
                        .filter(|line| line.sequence > baseline)
                        .map(|line| (line.sequence, line.text.clone()))
                        .collect::<Vec<_>>();
                    for (sequence, text) in fresh.into_iter().take(8 - plan.chat_logged) {
                        plan.chat_logged += 1;
                        plan.chat_baseline = plan.chat_baseline.max(sequence);
                        actor.transitions.push(json!({
                            "event": "macro-event-chat",
                            "observed_tick": tick,
                            "text": text,
                        }));
                    }
                }
                if hold.hold != plan.last_hold {
                    plan.last_hold = hold.hold;
                    actor.transitions.push(json!({
                        "event": "guardian-hold",
                        "held": hold.hold,
                        "observed_tick": tick,
                        "tile": actor.last_tile.map(|tile| [tile.x, tile.z, tile.level]),
                    }));
                }
            }
            let active_death_combat = actor.latest_status.as_ref().and_then(|status| {
                let target_step = actor.death_target.as_ref().is_some_and(|(stage, step)| {
                    status.phase == NativePhase::Working
                        && status_stage(status, actor.quest).as_deref() == Some(stage.as_str())
                        && step_matches(status, step)
                });
                target_step
                    .then(|| active_owned_combat(&actor.snapshot))
                    .flatten()
            });
            if actor.death_due && !actor.death_sent && !hold.hold {
                if let Some(combat) = active_death_combat {
                    if interact::cheat(client, "~death").is_sent() {
                        let status = actor
                            .latest_status
                            .as_ref()
                            .map(status_json)
                            .unwrap_or(Value::Null);
                        let target = actor.death_target.clone().unwrap_or_default();
                        let chat_baseline = actor
                            .snapshot
                            .chat_lines()
                            .iter()
                            .map(|line| line.sequence)
                            .max()
                            .unwrap_or(0);
                        actor.death_sent = true;
                        actor.death_due = false;
                        actor.death_injections += 1;
                        actor.death_target = Some(target.clone());
                        actor.death_combat = Some(combat.clone());
                        actor.death_status.get_or_insert(status);
                        actor.death_chat_baseline = Some(chat_baseline);
                        actor.transitions.push(json!({
                            "event": "death-injection-sent",
                            "role": actor.role,
                            "account": actor.account,
                            "stage": target.0,
                            "step": target.1,
                            "status": actor.death_status,
                            "owned_combat": owned_combat_json(&combat),
                            "chat_sequence_baseline": chat_baseline,
                            "authorized_injection": "~death",
                        }));
                        let receipt = capture_receipt(
                            actor,
                            "death-sent",
                            actor.latest_status.as_ref(),
                            json!({
                                "authorized_injection": "~death",
                                "stage": target.0,
                                "step": target.1,
                                "owned_combat_at_send": owned_combat_json(&combat),
                                "chat_sequence_baseline": chat_baseline,
                            }),
                        );
                        actor.pending_captures.push(("death-sent".into(), receipt));
                    }
                }
            }
            if actor.death_sent && !actor.death_seen {
                let baseline = actor.death_chat_baseline.unwrap_or(i32::MAX);
                let witness = actor
                    .snapshot
                    .chat_lines()
                    .iter()
                    .find(|line| {
                        line.sequence > baseline && script::native::death::is_death_line(&line.text)
                    })
                    .map(|line| (line.sequence, line.text.clone()));
                if let Some((sequence, text)) = witness {
                    actor.death_messages.insert(sequence);
                    actor.death_seen = true;
                    let receipt = capture_receipt(
                        actor,
                        "death-observed",
                        actor.latest_status.as_ref(),
                        json!({
                            "death_message": {"sequence": sequence, "text": text},
                            "stage": actor.death_target.as_ref().map(|target| &target.0),
                            "step": actor.death_target.as_ref().map(|target| &target.1),
                        }),
                    );
                    actor
                        .pending_captures
                        .push(("death-observed".into(), receipt));
                }
            }
            if client.ingame && client.scene_state == 2 && !actor.pending_captures.is_empty() {
                let pending = std::mem::take(&mut actor.pending_captures);
                for (label, mut receipt) in pending {
                    receipt["tile"] =
                        json!(actor.last_tile.map(|tile| [tile.x, tile.z, tile.level]));
                    receipt["inventory"] = json!(actor
                        .snapshot
                        .inventory()
                        .iter()
                        .filter(|item| item.count > 0)
                        .map(|item| [item.def.id, item.count])
                        .collect::<Vec<_>>());
                    receipt["equipment"] = json!(actor
                        .snapshot
                        .equipment()
                        .iter()
                        .map(|item| item.def.id)
                        .collect::<Vec<_>>());
                    receipt["stats"] = json!(actor
                        .snapshot
                        .stats()
                        .iter()
                        .map(|stat| json!([stat.name, stat.base, stat.effective]))
                        .collect::<Vec<_>>());
                    receipt["chat"] = json!(actor
                        .snapshot
                        .chat_lines()
                        .iter()
                        .rev()
                        .take(12)
                        .map(|line| line.text.to_string())
                        .collect::<Vec<_>>());
                    actor.capture_seq += 1;
                    let sequence = actor.capture_seq;
                    match submit_capture(
                        client,
                        &actor.evidence,
                        &actor.directory,
                        sequence,
                        &label,
                        receipt,
                    ) {
                        Ok(pending) => actor.pending_jobs.push(pending),
                        Err(error) => {
                            let error = format!("capture {label}: {error}");
                            actor.capture_error = Some(error.clone());
                            actor.error = Some(error);
                        }
                    }
                }
            }
            // The run's terminal wait already polls `end_captured` under a
            // deadline; completions land here, which is the bounded flush:
            // paths are recorded only once the files are durable.
            let evidence = Arc::clone(&actor.evidence);
            let mut completed = Vec::new();
            actor
                .pending_jobs
                .retain(|pending| match evidence.poll(&pending.job) {
                    Some(outcome) => {
                        completed.push((
                            pending.label.clone(),
                            pending.png_path.clone(),
                            outcome.error,
                        ));
                        false
                    }
                    None => true,
                });
            for (label, png_path, error) in completed {
                match error {
                    None => {
                        actor.captures.push(png_path);
                        if label == "end-pass" || label == "end-fail" {
                            actor.end_captured = true;
                        }
                    }
                    Some(error) => {
                        let error = format!("capture {label}: {error}");
                        actor.capture_error = Some(error.clone());
                        actor.error = Some(error);
                    }
                }
            }
        },
    )?;

    let start_handle = play.script_start_handle();
    {
        let mut state = shared.lock().map_err(|_| "live state poisoned")?;
        let obj_names = play.obj_names();
        for actor in &mut state.actors {
            actor.runner.set_obj_names(Arc::clone(&obj_names));
            actor.start_handle = Some(start_handle.clone());
            actor.named_banks = Some(play.named_banks());
        }
    }
    if mode.is_pair() {
        play.set_quest_accounts(names.iter().map(String::as_str));
    }
    for (index, (account, password)) in entries.iter().enumerate() {
        play.try_spawn_slot(mint_profile(account, password)?, None, None, None)
            .map_err(|error| format!("spawn role {index} ({account}): {error}"))?;
    }
    {
        let mut state = shared.lock().map_err(|_| "live state poisoned")?;
        state.start_handles_ready = true;
    }
    play.focus(&names[0]);
    if mode.is_pair() {
        let settings = shared
            .lock()
            .map_err(|_| "live state poisoned")?
            .actors
            .iter()
            .map(|actor| actor.start_settings.clone())
            .collect::<Vec<_>>();
        println!(
            "{}",
            json!({
                "phase": "identity",
                "quest": shared.lock().map_err(|_| "live state poisoned")?.actors[0].quest,
                "mode": mode.name(),
                "accounts": names,
                "game_port": profile.client().game_port(),
                "nav_pack": profile.nav_pack(),
                "settings": settings,
                "evidence": directories,
            })
        );
    } else {
        let state = shared.lock().map_err(|_| "live state poisoned")?;
        let actor = &state.actors[0];
        println!(
            "{}",
            json!({
                "phase": "identity",
                "cell": actor.label,
                "quest": actor.quest,
                "mode": mode.name(),
                "account": actor.account,
                "game_port": profile.client().game_port(),
                "nav_pack": profile.nav_pack(),
                "settings": actor.start_settings,
                "evidence": actor.directory,
            })
        );
    }

    let mut last_logged: Vec<Option<Value>> = vec![None; names.len()];
    let mut captured_starts = vec![0u32; names.len()];
    let restart_targets = mode.restart_targets();
    let mut restart_phase = RestartPhase::Seeking;
    let mut restarts_done = 0usize;
    let death_target = mode.death_target();
    let mut family_receipt = None;

    let mut result: Result<(), String> = 'run: loop {
        let mut observations = Vec::with_capacity(names.len());
        for account in &names {
            observations.push(PollActor {
                status: play.script_native_status(account),
                run_state: play.script_state(account),
                lifecycle: play.script_lifecycle_receipt(account),
                last_error: play.script_last_error(account),
                runner_status: RunnerStatus::Seeding,
                done: false,
                card_complete: false,
                terminal: false,
            });
        }
        {
            let mut state = match shared.lock() {
                Ok(state) => state,
                Err(_) => break Err("live state poisoned".into()),
            };
            for (index, observation) in observations.iter_mut().enumerate() {
                let actor = &mut state.actors[index];
                if observation.run_state == script::RunState::Running {
                    actor.runner.observe_script_running();
                }
                if let Some(status) = observation.status.as_deref() {
                    actor.latest_status = Some(status.clone());
                    actor.owned_card_complete |= card_complete(status, actor.quest);
                    observe_death_count(actor, status);
                    let observed_step = if actor.track_steps {
                        status_stage(status, actor.quest)
                            .zip(text(status, "child_step_id").or_else(|| text(status, "step_id")))
                    } else {
                        None
                    };
                    if let Some((stage, step)) = observed_step {
                        let step_key = (status.run, stage.clone(), step.to_owned());
                        if actor.last_step_observation.as_ref() != Some(&step_key) {
                            actor.last_step_observation = Some(step_key);
                            actor.step_observations.push(json!({
                                "run": format!("{:?}", status.run),
                                "tick": actor.snapshot.tick(),
                                "stage": stage,
                                "sequence": integer(status, "sequence"),
                                "step_index": integer(status, "step_index"),
                                "remaining_steps": integer(status, "remaining_steps"),
                                "step_id": text(status, "step_id"),
                                "child_step_id": text(status, "child_step_id"),
                                "action_state": text(status, "action_state"),
                                "waiting_for": text(status, "waiting_for"),
                                "owned_combat": active_owned_combat(&actor.snapshot)
                                    .as_ref()
                                    .map(owned_combat_json)
                                    .unwrap_or(Value::Null),
                                "inventory": inventory_receipt(&actor.snapshot),
                            }));
                        }
                    }
                }
                if let Some(observe) = actor.observe_tick.as_mut() {
                    if let Some(receipt) = observe(&actor.snapshot, observation.status.as_deref()) {
                        actor.tick_observations.push(receipt);
                    }
                }
                if quester_path {
                    actor.tick_mean.observe(&actor.snapshot);
                }
                observation.runner_status = actor.runner.status();
                observation.done = quest_done(&actor.snapshot, actor.display);
                observation.card_complete = actor.owned_card_complete;
                observation.terminal = completion_proven(
                    matches!(&observation.runner_status, RunnerStatus::Passed),
                    observation.done || observation.card_complete,
                    observation.run_state == script::RunState::Idle,
                    observation.lifecycle.as_ref().is_some_and(|receipt| {
                        receipt.state == script::ScriptTerminalState::Completed
                    }),
                );
            }
        }

        for index in 0..names.len() {
            let Some(status) = observations[index].status.as_deref() else {
                continue;
            };
            let logged = status_json(status);
            if last_logged[index].as_ref() != Some(&logged) {
                let tick = shared.lock().map_err(|_| "live state poisoned")?.actors[index]
                    .snapshot
                    .tick();
                match append_status(&status_logs[index], status, tick) {
                    Ok(logged) => last_logged[index] = Some(logged),
                    Err(error) => break 'run Err(format!("status log {}: {error}", names[index])),
                }
            }
        }

        {
            let mut state = match shared.lock() {
                Ok(state) => state,
                Err(_) => break 'run Err("live state poisoned".into()),
            };
            let pair_death_target = mode.pair_death_target().cloned();
            let pair_death_authorized = pair_death_target.as_ref().is_some_and(|target| {
                let victim = &state.actors[target.role];
                victim.death_sent
                    && victim.death_injections == 1
                    && victim.death_status.is_some()
                    && victim
                        .death_target
                        .as_ref()
                        .is_some_and(|(stage, _)| stage == &target.stage)
                    && state.actors[1 - target.role].deaths_total == 0
            });
            let pair_death_status = pair_death_target
                .as_ref()
                .and_then(|target| state.actors[target.role].death_status.clone());
            for (index, observation) in observations.iter().enumerate() {
                let actor = &mut state.actors[index];
                if let Some(status) = observation.status.as_deref() {
                    if pair_death_authorized && pair_cancelled(status) && !actor.pair_cancelled {
                        actor.pair_cancelled = true;
                        actor.transitions.push(json!({
                            "event": "product-pair-cancelled",
                            "role": actor.role,
                            "account": actor.account,
                            "cause_target": pair_death_target,
                            "death_authorization_status": pair_death_status,
                            "status": status_json(status),
                            "exact_pair_cancelled_failure": true,
                        }));
                        let receipt = capture_receipt(
                            actor,
                            "pair-cancelled",
                            Some(status),
                            json!({
                                "cause_target": pair_death_target,
                                "death_authorization_status": pair_death_status,
                                "exact_pair_cancelled_failure": true,
                            }),
                        );
                        actor
                            .pending_captures
                            .push(("pair-cancelled".into(), receipt));
                    }
                    if status.phase == NativePhase::Working {
                        while captured_starts[index] < actor.starts {
                            captured_starts[index] += 1;
                            let label = format!("start-{}", captured_starts[index]);
                            let receipt = capture_receipt(
                                actor,
                                &label,
                                Some(status),
                                json!({"start_transition": captured_starts[index]}),
                            );
                            actor.pending_captures.push((label, receipt));
                        }
                    }
                    if let Some(stage) = status_stage(status, actor.quest) {
                        if actor.stages_seen.last() != Some(&stage) {
                            actor.stages_seen.push(stage.clone());
                            actor
                                .stage_ticks
                                .push(json!({"stage": stage, "tick": actor.snapshot.tick()}));
                            actor.transitions.push(json!({
                                "event": "stage",
                                "stage": stage,
                                "tick": actor.snapshot.tick(),
                            }));
                            let label = format!("stage-{}", stage.replace(':', "-"));
                            let receipt = capture_receipt(actor, &label, Some(status), json!({}));
                            actor.pending_captures.push((label, receipt));
                        }
                    }
                }
            }
        }

        let restart_settling = matches!(
            restart_phase,
            RestartPhase::Stopping { .. } | RestartPhase::Starting { .. }
        );
        let strict_pair_death = mode.pair_death_target().is_some();
        for (index, observation) in observations.iter().enumerate() {
            let (actor_error, pair_cancelled_seen) = match shared.lock() {
                Ok(state) => (
                    state.actors[index].error.clone(),
                    state.actors[index].pair_cancelled,
                ),
                Err(_) => break 'run Err("live state poisoned".into()),
            };
            if let Some(error) = actor_error {
                break 'run Err(error);
            }
            if let RunnerStatus::Failed(error) = &observation.runner_status {
                break 'run Err(format!("{} fixture failed: {error}", names[index]));
            }
            if strict_pair_death {
                let status_is_pair_cancelled =
                    observation.status.as_deref().is_some_and(pair_cancelled);
                let allowed_cancel =
                    pair_cancelled_seen && (status_is_pair_cancelled || restart_settling);
                if let Some(error) = observation.last_error.as_deref() {
                    if !allowed_cancel || error != PAIR_CANCELLED_REASON {
                        break 'run Err(format!(
                            "{} Quester lifecycle error: {error}",
                            names[index]
                        ));
                    }
                }
                if let Some(status) = observation.status.as_deref() {
                    if let Some(reason) = parked(status) {
                        if !allowed_cancel || !pair_cancelled(status) {
                            break 'run Err(format!(
                                "{} Quester parked/blocked: {reason}",
                                names[index]
                            ));
                        }
                    }
                }
            } else if !restart_settling {
                let expected_stop = mode.expected_named_stop().filter(|reason| {
                    observe_family.is_some()
                        && has_named_stop(observation.status.as_deref(), reason)
                });
                if let Some(error) = &observation.last_error {
                    if expected_stop != Some(error.as_str()) {
                        break 'run Err(format!(
                            "{} Quester lifecycle error: {error}",
                            names[index]
                        ));
                    }
                }
                if let Some(status) = observation.status.as_deref() {
                    if let Some(reason) = family_parked_error(status, blocked_policy) {
                        if expected_stop.is_none() {
                            break 'run Err(format!(
                                "{} Quester parked/blocked: {reason}",
                                names[index]
                            ));
                        }
                    }
                }
            }
        }

        let mut stages = Vec::with_capacity(names.len());
        let mut starts = Vec::with_capacity(names.len());
        let mut stops = Vec::with_capacity(names.len());
        let mut stage_transitions = Vec::with_capacity(names.len());
        let mut deaths = Vec::with_capacity(names.len());
        let mut death_seen = Vec::with_capacity(names.len());

        {
            let state = match shared.lock() {
                Ok(state) => state,
                Err(_) => break Err("live state poisoned".into()),
            };
            for actor in &state.actors {
                stages.push(actor.stages_seen.last().cloned());
                starts.push(actor.starts);
                stops.push(actor.stops);
                stage_transitions.push(actor.stages_seen.len() > 1);
                deaths.push(actor.deaths_total);
                death_seen.push(actor.death_seen);
            }
        }
        let terminal: Vec<bool> = observations
            .iter()
            .map(|actor| observe_family.is_none() && actor.terminal)
            .collect();
        if let Some(target) = mode.pair_death_target() {
            let cancellation_ready = match shared.lock() {
                Ok(state) => {
                    let victim = &state.actors[target.role];
                    let peer = &state.actors[1 - target.role];
                    starts == [1, 1]
                        && stops == [0, 0]
                        && matches!(restart_phase, RestartPhase::Seeking)
                        && state.actors.iter().all(|actor| actor.pair_cancelled)
                        && observations
                            .iter()
                            .all(|actor| actor.status.as_deref().is_some_and(pair_cancelled))
                        && death_target.as_ref().is_some_and(|(death_stage, step)| {
                            death_stage == target
                                && victim.death_target.as_ref()
                                    == Some(&(target.stage.clone(), step.clone()))
                        })
                        && victim.death_sent
                        && victim.death_injections == 1
                        && victim.death_seen
                        && victim.deaths_total == 1
                        && peer.deaths_total == 0
                }
                Err(_) => break 'run Err("live state poisoned".into()),
            };
            if cancellation_ready {
                let statuses_at_stop = observations
                    .iter()
                    .map(|actor| {
                        actor
                            .status
                            .as_deref()
                            .map(status_json)
                            .unwrap_or(Value::Null)
                    })
                    .collect::<Vec<_>>();
                let death_status = match shared.lock() {
                    Ok(state) => state.actors[target.role].death_status.clone(),
                    Err(_) => break 'run Err("live state poisoned".into()),
                };
                let target_status = statuses_at_stop[target.role].clone();
                for (role, account) in names.iter().enumerate() {
                    play.script_stop(account);
                    let mut state = match shared.lock() {
                        Ok(state) => state,
                        Err(_) => break 'run Err("live state poisoned".into()),
                    };
                    let actor = &mut state.actors[role];
                    actor.stops += 1;
                    actor.transitions.push(json!({
                        "event": "explicit-stop-after-authorized-pair-cancellation",
                        "stop": actor.stops,
                        "round": 1,
                        "target": {"role": target.role, "stage": target.stage},
                        "account": account,
                        "status_before_stop": statuses_at_stop[role],
                        "target_status": target_status,
                        "death_status": death_status,
                        "explicit": true,
                    }));
                }
                restart_phase = RestartPhase::Stopping {
                    target: target.clone(),
                    round: 1,
                    target_status,
                    statuses_at_stop,
                };
            }
        }

        let restart_step_target = mode.restart_step_target();
        let named_step_all_live = restart_step_target.is_none()
            || observations
                .iter()
                .all(|actor| actor.run_state == script::RunState::Running && !actor.terminal);
        if let Some(targets) = restart_targets.as_ref() {
            let next_phase = match &restart_phase {
                RestartPhase::Seeking if mode.pair_death_target().is_some() => None,
                RestartPhase::Seeking if restarts_done < targets.len() => {
                    let target = &targets[restarts_done];
                    let role = target.role;
                    let named_step_active = match restart_step_target {
                        Some((step_target, step, _)) if step_target == target => observations[role]
                            .status
                            .as_deref()
                            .is_some_and(|status| active_waiting_step(status, step)),
                        _ => true,
                    };
                    let active = stages[role].as_deref() == Some(target.stage.as_str())
                        && observations[role].run_state == script::RunState::Running
                        && starts[role] == restarts_done as u32 + 1
                        && !terminal[role]
                        && named_step_active
                        && named_step_all_live;
                    active.then(|| RestartPhase::MidStep {
                        target: restarts_done,
                        since: Instant::now(),
                    })
                }
                RestartPhase::Seeking => None,
                RestartPhase::MidStep { target, since } => {
                    let wanted = &targets[*target];
                    let named_step_active = match restart_step_target {
                        Some((step_target, step, _)) if step_target == wanted => observations
                            [wanted.role]
                            .status
                            .as_deref()
                            .is_some_and(|status| active_waiting_step(status, step)),
                        _ => true,
                    };
                    let active = stages[wanted.role].as_deref() == Some(wanted.stage.as_str())
                        && observations[wanted.role].run_state == script::RunState::Running
                        && starts[wanted.role] == *target as u32 + 1
                        && !terminal[wanted.role]
                        && named_step_active
                        && named_step_all_live;
                    let stop_due = restart_step_target.is_some() || since.elapsed() >= MID_STEP;
                    if !active {
                        Some(RestartPhase::Seeking)
                    } else if stop_due {
                        let proof_boundary = if let Some((_, _, Some(proof))) = restart_step_target
                        {
                            let mut state = match shared.lock() {
                                Ok(state) => state,
                                Err(_) => break 'run Err("live state poisoned".into()),
                            };
                            let counts = [
                                inventory_count(&state.actors[0].snapshot, proof.item_id),
                                inventory_count(&state.actors[1].snapshot, proof.item_id),
                            ];
                            if counts
                                .iter()
                                .zip(proof.maximum_after_restart)
                                .any(|(count, maximum)| *count > maximum)
                            {
                                break 'run Err(format!(
                                    "named-step inventory proof exceeded its count limit before Stop: {counts:?}"
                                ));
                            }
                            let ready = state.inventory_initial_absence == [true, true]
                                && counts == proof.boundary_counts;
                            if !ready {
                                None
                            } else {
                                let boundary = json!({
                                    "stage": wanted.stage,
                                    "step": restart_step_target.map(|(_, step, _)| step),
                                    "status": observations[wanted.role]
                                        .status
                                        .as_deref()
                                        .map(status_json),
                                    "active_waiting_step": true,
                                    "terminal": terminal[wanted.role],
                                    "all_roles_running_and_nonterminal": named_step_all_live,
                                    "run_state": format!("{:?}", observations[wanted.role].run_state),
                                    "item_id": proof.item_id,
                                    "role_ordered_counts": counts,
                                    "initial_absent_item_ids": proof.initially_absent,
                                    "initial_absence_by_role": state.inventory_initial_absence,
                                    "inventories": state
                                        .actors
                                        .iter()
                                        .map(|actor| inventory_receipt(&actor.snapshot))
                                        .collect::<Vec<_>>(),
                                });
                                state.inventory_boundary = Some(boundary.clone());
                                Some(boundary)
                            }
                        } else {
                            None
                        };
                        let proof_ready = restart_step_target
                            .and_then(|(_, _, proof)| proof)
                            .is_none()
                            || proof_boundary.is_some();
                        if !proof_ready {
                            None
                        } else {
                            let statuses_at_stop = observations
                                .iter()
                                .map(|actor| {
                                    actor
                                        .status
                                        .as_deref()
                                        .map(status_json)
                                        .unwrap_or(Value::Null)
                                })
                                .collect::<Vec<_>>();
                            let target_status = statuses_at_stop[wanted.role].clone();
                            let round = *target as u32 + 1;
                            for (role, account) in names.iter().enumerate() {
                                play.script_stop(account);
                                let mut state = match shared.lock() {
                                    Ok(state) => state,
                                    Err(_) => break 'run Err("live state poisoned".into()),
                                };
                                let actor = &mut state.actors[role];
                                actor.stops += 1;
                                actor.transitions.push(json!({
                                    "event": "stop",
                                    "stop": actor.stops,
                                    "round": round,
                                    "target": {"role": wanted.role, "stage": wanted.stage},
                                    "target_step": restart_step_target.map(|(_, step, _)| step),
                                    "account": account,
                                    "status_before_stop": statuses_at_stop[role],
                                    "target_status": target_status,
                                    "inventory_boundary": proof_boundary,
                                    "explicit": true,
                                }));
                                if let Some(boundary) = proof_boundary.as_ref() {
                                    let receipt = capture_receipt(
                                        actor,
                                        "restart-boundary",
                                        observations[role].status.as_deref(),
                                        json!({
                                            "target": {"role": wanted.role, "stage": wanted.stage},
                                            "target_step": restart_step_target
                                                .map(|(_, step, _)| step),
                                            "inventory_boundary": boundary,
                                            "explicit_stop": true,
                                        }),
                                    );
                                    actor
                                        .pending_captures
                                        .push(("restart-boundary".into(), receipt));
                                }
                            }
                            Some(RestartPhase::Stopping {
                                target: wanted.clone(),
                                round,
                                target_status,
                                statuses_at_stop,
                            })
                        }
                    } else {
                        None
                    }
                }
                RestartPhase::Stopping {
                    target,
                    round,
                    target_status,
                    statuses_at_stop,
                } if observations
                    .iter()
                    .all(|actor| actor.run_state == script::RunState::Idle) =>
                {
                    let mut state = match shared.lock() {
                        Ok(state) => state,
                        Err(_) => break 'run Err("live state poisoned".into()),
                    };
                    let inventory_boundary = state.inventory_boundary.clone();
                    for role in 0..state.actors.len() {
                        let actor = &mut state.actors[role];
                        actor.restart_due = true;
                        let label = format!("stopped-{round}");
                        let receipt = capture_receipt(
                            actor,
                            &label,
                            observations[role].status.as_deref(),
                            json!({
                                "target": {"role": target.role, "stage": target.stage},
                                "target_step": restart_step_target.map(|(_, step, _)| step),
                                "target_status": target_status,
                                "status_before_stop": statuses_at_stop[role],
                                "stop_round": round,
                                "state_after_stop": "Idle",
                                "inventory_boundary": inventory_boundary,
                            }),
                        );
                        actor.pending_captures.push((label, receipt));
                    }
                    Some(RestartPhase::Starting { round: *round })
                }
                RestartPhase::Stopping { .. } => None,
                RestartPhase::Starting { round } => {
                    let started = match shared.lock() {
                        Ok(state) => state.actors.iter().all(|actor| actor.starts > *round),
                        Err(_) => break 'run Err("live state poisoned".into()),
                    };
                    if started {
                        restarts_done += 1;
                        Some(RestartPhase::Seeking)
                    } else {
                        None
                    }
                }
            };
            if let Some(next_phase) = next_phase {
                restart_phase = next_phase;
            }
        }

        if restarts_done > 0 && captured_starts.iter().all(|count| *count >= 2) {
            let inventory_violation = {
                let mut state = match shared.lock() {
                    Ok(state) => state,
                    Err(_) => break 'run Err("live state poisoned".into()),
                };
                if let Some(proof) = state.inventory_proof.clone() {
                    let counts = [
                        inventory_count(&state.actors[0].snapshot, proof.item_id),
                        inventory_count(&state.actors[1].snapshot, proof.item_id),
                    ];
                    if counts
                        .iter()
                        .zip(proof.maximum_after_restart)
                        .any(|(count, maximum)| *count > maximum)
                    {
                        let violation = format!(
                            "post-restart inventory exceeded the no-refarm limits: {counts:?}"
                        );
                        state.inventory_violation = Some(violation.clone());
                        Some(violation)
                    } else if state.inventory_last_counts != Some(counts) {
                        let sample = json!({
                            "after_explicit_start_rounds": restarts_done,
                            "item_id": proof.item_id,
                            "role_ordered_counts": counts,
                            "maximum_after_restart": proof.maximum_after_restart,
                            "statuses": observations
                                .iter()
                                .map(|actor| actor.status.as_deref().map(status_json))
                                .collect::<Vec<_>>(),
                            "inventories": state
                                .actors
                                .iter()
                                .map(|actor| inventory_receipt(&actor.snapshot))
                                .collect::<Vec<_>>(),
                        });
                        state.inventory_restart_samples.push(sample.clone());
                        state.inventory_last_counts = Some(counts);
                        for (actor, observation) in state.actors.iter_mut().zip(&observations) {
                            let receipt = capture_receipt(
                                actor,
                                "inventory-after-restart",
                                observation.status.as_deref(),
                                json!({"inventory_proof_sample": sample}),
                            );
                            actor
                                .pending_captures
                                .push(("inventory-after-restart".into(), receipt));
                        }
                        None
                    } else {
                        None
                    }
                } else {
                    None
                }
            };
            if let Some(violation) = inventory_violation {
                break 'run Err(violation);
            }
        }

        if let Some((target, step)) = death_target.as_ref() {
            let role = target.role;
            let status = observations[role].status.as_deref();
            let mut state = match shared.lock() {
                Ok(state) => state,
                Err(_) => break 'run Err("live state poisoned".into()),
            };
            let actor = &mut state.actors[role];
            let on_step = observations[role].run_state == script::RunState::Running
                && status.is_some_and(|status| {
                    status.phase == NativePhase::Working
                        && status_stage(status, actor.quest).as_deref()
                            == Some(target.stage.as_str())
                        && step_matches(status, step)
                });
            let already_authorized = actor.death_status.is_some() || actor.death_sent;
            if on_step && !already_authorized {
                let combat = active_owned_combat(&actor.snapshot);
                if let (Some(status), Some(combat)) = (status, combat) {
                    let status_json = status_json(status);
                    actor.death_status = Some(status_json.clone());
                    actor.death_target = Some((target.stage.clone(), step.clone()));
                    actor.death_combat = Some(combat.clone());
                    actor.death_due = true;
                    actor.transitions.push(json!({
                        "event": "death-injection-authorized",
                        "role": actor.role,
                        "account": actor.account,
                        "stage": target.stage,
                        "step": step,
                        "status": status_json,
                        "owned_combat": owned_combat_json(&combat),
                        "requires_active_owned_combat": true,
                    }));
                }
            }
        }

        let all_terminal = terminal.iter().all(|value| *value);
        if starts[0] > 0 {
            if let Some(observe) = observe_family.as_mut() {
                let state = match shared.lock() {
                    Ok(state) => state,
                    Err(_) => break 'run Err("live state poisoned".into()),
                };
                match observe(
                    &state.actors[0].snapshot,
                    observations[0].status.as_deref(),
                    observations[0].lifecycle.as_ref(),
                ) {
                    Ok(Some(receipt)) => {
                        family_receipt = Some(receipt);
                        break 'run Ok(());
                    }
                    Ok(None) => {}
                    Err(error) => break 'run Err(format!("family proof: {error}")),
                }
            }
        }
        match &mode {
            RunPlan::Single(Mode::Stage { expect }) => {
                let stage_ok = observe_family.is_none()
                    && starts[0] > 0
                    && stage_or_terminal(
                        expect,
                        stages[0].as_deref(),
                        stage_transitions[0],
                        terminal[0],
                    );
                if stage_ok {
                    break Ok(());
                }
            }
            RunPlan::Single(Mode::PathTerminal) => {
                let script_terminal = path_terminal_proven(
                    observations[0].run_state == script::RunState::Idle,
                    observations[0].lifecycle.as_ref().is_some_and(|receipt| {
                        receipt.state == script::ScriptTerminalState::Completed
                    }),
                );
                if starts[0] == 1 && stops[0] == 0 && deaths[0] == 0 && script_terminal {
                    break Ok(());
                }
            }
            RunPlan::Single(Mode::Clean) if terminal[0] => {
                if starts[0] == 1 && deaths[0] == 0 {
                    break Ok(());
                }
                break Err(format!(
                    "clean run had starts={} deaths={}",
                    starts[0], deaths[0]
                ));
            }
            RunPlan::Single(Mode::Restart { .. }) if terminal[0] => {
                if restarts_done == 2 && starts[0] == 3 && stops[0] == 2 {
                    break Ok(());
                }
                break Err(format!(
                    "completed before both Stops: restarts={restarts_done} starts={}",
                    starts[0]
                ));
            }
            RunPlan::Single(Mode::Death { .. }) if terminal[0] => {
                let (sent, injections, injected_status) = match shared.lock() {
                    Ok(state) => (
                        state.actors[0].death_sent,
                        state.actors[0].death_injections,
                        state.actors[0].death_status.is_some(),
                    ),
                    Err(_) => break Err("live state poisoned".into()),
                };
                if death_seen[0]
                    && sent
                    && injections == 1
                    && injected_status
                    && deaths[0] == 1
                    && starts[0] == 1
                    && stops[0] == 0
                {
                    break Ok(());
                }
                break Err(format!(
                    "death cell completed with death_seen={} death_sent={sent} injections={injections} deaths={} starts={} stops={}",
                    death_seen[0], deaths[0], starts[0], stops[0]
                ));
            }
            RunPlan::Pair(PairMode::Stage { expect }) => {
                if (0..2).all(|role| {
                    starts[role] > 0
                        && stage_or_terminal(
                            &expect[role],
                            stages[role].as_deref(),
                            stage_transitions[role],
                            terminal[role],
                        )
                }) {
                    break Ok(());
                }
            }
            RunPlan::Pair(PairMode::Clean) if all_terminal => {
                if starts == [1, 1] && deaths == [0, 0] {
                    break Ok(());
                }
                break Err(format!(
                    "paired clean run had starts={starts:?} deaths={deaths:?}"
                ));
            }
            RunPlan::Pair(PairMode::Restart { .. }) if all_terminal => {
                if restarts_done == 2 && starts == [3, 3] && stops == [2, 2] {
                    break Ok(());
                }
                break Err(format!(
                    "paired quest completed before both Stops: restarts={restarts_done} starts={starts:?} stops={stops:?}"
                ));
            }
            RunPlan::Pair(PairMode::RestartStep { .. }) if all_terminal => {
                let clean_restarts = match shared.lock() {
                    Ok(state) => state.actors.iter().all(|actor| {
                        actor.death_injections == 0
                            && actor.transitions.iter().any(|transition| {
                                transition["event"] == "start"
                                    && transition["start"] == 2
                                    && transition["explicit"] == true
                                    && transition["fixture_replayed"] == false
                                    && transition["profile_reapplied"] == false
                                    && transition["settings_reused"] == true
                            })
                    }),
                    Err(_) => break Err("live state poisoned".into()),
                };
                if restarts_done == 1
                    && starts == [2, 2]
                    && stops == [1, 1]
                    && deaths == [0, 0]
                    && captured_starts.iter().all(|count| *count >= 2)
                    && clean_restarts
                {
                    break Ok(());
                }
                break Err(format!(
                    "named-step paired restart proof failed: restarts={restarts_done} starts={starts:?} stops={stops:?} deaths={deaths:?} clean_restarts={clean_restarts}"
                ));
            }
            RunPlan::Pair(PairMode::RestartStepWithInventoryProof {
                at,
                step,
                inventory_proof,
            }) if all_terminal => {
                let proof_ok = match shared.lock() {
                    Ok(state) => {
                        let boundary_ok =
                            state.inventory_boundary.as_ref().is_some_and(|boundary| {
                                boundary["stage"] == at.stage
                                    && boundary["step"] == *step
                                    && boundary["active_waiting_step"] == true
                                    && boundary["terminal"] == false
                                    && boundary["all_roles_running_and_nonterminal"] == true
                                    && boundary["role_ordered_counts"]
                                        == json!(inventory_proof.boundary_counts)
                                    && boundary["initial_absent_item_ids"]
                                        == json!(inventory_proof.initially_absent)
                            });
                        let samples_ok = !state.inventory_restart_samples.is_empty()
                            && state.inventory_violation.is_none()
                            && state.inventory_last_counts.is_some_and(|counts| {
                                counts
                                    .iter()
                                    .zip(inventory_proof.maximum_after_restart)
                                    .all(|(count, maximum)| *count <= maximum)
                            });
                        let configuration_matches =
                            state.inventory_proof.as_ref().is_some_and(|actual| {
                                actual.item_id == inventory_proof.item_id
                                    && actual.boundary_counts == inventory_proof.boundary_counts
                                    && actual.maximum_after_restart
                                        == inventory_proof.maximum_after_restart
                                    && actual.initially_absent == inventory_proof.initially_absent
                            });
                        let inventory_ok = boundary_ok
                            && samples_ok
                            && configuration_matches
                            && state.inventory_initial_absence == [true, true];
                        let clean_restarts = state.actors.iter().all(|actor| {
                            actor.death_injections == 0
                                && actor.transitions.iter().any(|transition| {
                                    transition["event"] == "start"
                                        && transition["start"] == 2
                                        && transition["explicit"] == true
                                        && transition["fixture_replayed"] == false
                                        && transition["profile_reapplied"] == false
                                        && transition["settings_reused"] == true
                                })
                        });
                        inventory_ok && clean_restarts
                    }
                    Err(_) => break Err("live state poisoned".into()),
                };
                if restarts_done == 1
                    && starts == [2, 2]
                    && stops == [1, 1]
                    && deaths == [0, 0]
                    && captured_starts.iter().all(|count| *count >= 2)
                    && proof_ok
                {
                    break Ok(());
                }
                break Err(format!(
                    "named-step paired restart proof failed: target={at:?} step={step} restarts={restarts_done} starts={starts:?} stops={stops:?} deaths={deaths:?} proof={proof_ok}"
                ));
            }
            RunPlan::Pair(PairMode::Death { at, .. }) if all_terminal => {
                let (sent, injections, injected_status, cancellations, explicit_restarts) =
                    match shared.lock() {
                        Ok(state) => {
                            let victim = &state.actors[at.role];
                            let cancellations = state
                                .actors
                                .iter()
                                .map(|actor| actor.pair_cancelled)
                                .collect::<Vec<_>>();
                            let explicit_restarts = state.actors.iter().all(|actor| {
                                actor.transitions.iter().any(|transition| {
                                    transition["event"]
                                        == "explicit-stop-after-authorized-pair-cancellation"
                                        && transition["explicit"] == true
                                }) && actor.transitions.iter().any(|transition| {
                                    transition["event"] == "start"
                                        && transition["start"] == 2
                                        && transition["explicit"] == true
                                        && transition["fixture_replayed"] == false
                                        && transition["profile_reapplied"] == false
                                        && transition["settings_reused"] == true
                                })
                            });
                            (
                                victim.death_sent,
                                victim.death_injections,
                                victim.death_status.is_some(),
                                cancellations,
                                explicit_restarts,
                            )
                        }
                        Err(_) => break Err("live state poisoned".into()),
                    };
                let peer = 1 - at.role;
                let cancelled_recovery = cancellations == [true, true]
                    && restarts_done == 1
                    && starts == [2, 2]
                    && stops == [1, 1]
                    && explicit_restarts;
                if death_seen[at.role]
                    && sent
                    && injections == 1
                    && injected_status
                    && deaths[at.role] == 1
                    && deaths[peer] == 0
                    && cancelled_recovery
                {
                    break Ok(());
                }
                break Err(format!(
                    "paired death proof failed: role={} death_seen={:?} sent={sent} injections={injections} deaths={deaths:?} starts={starts:?} stops={stops:?} cancellations={cancellations:?} explicit_restarts={explicit_restarts}",
                    at.role, death_seen
                ));
            }
            RunPlan::Pair(PairMode::DeathIndependent { at, .. }) if all_terminal => {
                let (sent, injections, injected_status, cancellations) = match shared.lock() {
                    Ok(state) => {
                        let victim = &state.actors[at.role];
                        (
                            victim.death_sent,
                            victim.death_injections,
                            victim.death_status.is_some(),
                            state.actors.iter().any(|actor| actor.pair_cancelled),
                        )
                    }
                    Err(_) => break Err("live state poisoned".into()),
                };
                if death_seen[at.role]
                    && sent
                    && injections == 1
                    && injected_status
                    && deaths[at.role] == 1
                    && deaths[1 - at.role] == 0
                    && starts == [1, 1]
                    && stops == [0, 0]
                    && !cancellations
                {
                    break Ok(());
                }
                break Err(format!(
                    "independent paired death proof failed: role={} death_seen={:?} sent={sent} injections={injections} deaths={deaths:?} starts={starts:?} stops={stops:?} pair_cancelled={cancellations}",
                    at.role, death_seen
                ));
            }
            _ => {}
        }

        if let Some(limit) = campaign_tick_limit {
            let state = shared.lock().map_err(|_| "live state poisoned")?;
            if let Some(actor) = state
                .actors
                .iter()
                .find(|actor| actor.snapshot.tick() >= limit)
            {
                break Err(format!(
                    "observed-tick deadline: {} reached {} ticks (limit {limit})",
                    actor.account,
                    actor.snapshot.tick()
                ));
            }
        }

        if Instant::now() >= deadline {
            let statuses = observations
                .iter()
                .map(|actor| {
                    actor
                        .status
                        .as_deref()
                        .map(status_json)
                        .unwrap_or(Value::Null)
                })
                .collect::<Vec<_>>();
            let stages = shared
                .lock()
                .map(|state| {
                    state
                        .actors
                        .iter()
                        .map(|actor| actor.stages_seen.clone())
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            break Err(format!(
                "timed out: runners={:?} states={:?} stages={stages:?} statuses={statuses:?}",
                observations
                    .iter()
                    .map(|actor| &actor.runner_status)
                    .collect::<Vec<_>>(),
                observations
                    .iter()
                    .map(|actor| actor.run_state)
                    .collect::<Vec<_>>()
            ));
        }
        std::thread::sleep(POLL_INTERVAL);
    };
    if quester_path && result.is_ok() {
        let observed_tick_mean = shared
            .lock()
            .map(|state| state.actors[0].tick_mean.mean_ms().is_some())
            .unwrap_or(false);
        if !observed_tick_mean {
            result = Err("the live path did not observe an in-game tick interval".into());
        }
    }

    let result_text = format!("{result:?}");
    let run_result = result.clone();
    let end_ticks = shared
        .lock()
        .map_err(|_| "live state poisoned")?
        .actors
        .iter()
        .map(|actor| actor.snapshot.tick())
        .collect::<Vec<_>>();
    let mut final_statuses = Vec::with_capacity(names.len());
    let mut final_lifecycles = Vec::with_capacity(names.len());
    for account in &names {
        final_statuses.push(play.script_native_status(account));
        final_lifecycles.push(play.script_lifecycle_receipt(account));
    }
    {
        let mut state = shared.lock().map_err(|_| "live state poisoned")?;
        for (index, actor) in state.actors.iter_mut().enumerate() {
            let label = if result.is_ok() {
                "end-pass"
            } else {
                "end-fail"
            };
            let receipt = capture_receipt(
                actor,
                label,
                final_statuses[index].as_deref(),
                json!({"result": result_text.as_str(), "role": actor.role}),
            );
            actor.pending_captures.push((label.to_owned(), receipt));
        }
    }
    let capture_wait_started = Instant::now();
    let capture_deadline = capture_wait_started + END_CAPTURE_BOUND;
    loop {
        let all_captured = shared
            .lock()
            .map_err(|_| "live state poisoned")?
            .actors
            .iter()
            .all(|actor| actor.end_captured);
        if all_captured || Instant::now() >= capture_deadline {
            break;
        }
        std::thread::sleep(POLL_INTERVAL);
    }
    let end_capture_wait_ms = capture_wait_started.elapsed().as_millis() as u64;

    let capture_errors = shared
        .lock()
        .map_err(|_| "live state poisoned")?
        .actors
        .iter()
        .filter_map(|actor| actor.capture_error.clone())
        .collect::<Vec<_>>();
    if result.is_ok() && !capture_errors.is_empty() {
        result = Err(capture_errors.join("; "));
    }
    let missing_end = shared
        .lock()
        .map_err(|_| "live state poisoned")?
        .actors
        .iter()
        .enumerate()
        .filter(|(_, actor)| !actor.end_captured)
        .map(|(index, actor)| {
            let file = actor
                .pending_jobs
                .iter()
                .find(|pending| pending.label == "end-pass" || pending.label == "end-fail")
                .map_or_else(
                    || "never submitted (client not in game)".to_owned(),
                    |pending| format!("{} not written", pending.png_path.display()),
                );
            format!("{index} ({}): end capture {file}", actor.account)
        })
        .collect::<Vec<_>>();
    if result.is_ok() && !missing_end.is_empty() {
        result = Err(format!(
            "terminal capture file missing after {} s: {}",
            END_CAPTURE_BOUND.as_secs(),
            missing_end.join("; ")
        ));
    }

    let actor_receipts = {
        let state = shared.lock().map_err(|_| "live state poisoned")?;
        state
            .actors
            .iter()
            .enumerate()
            .map(|(index, actor)| {
                let mut death_messages = actor.death_messages.iter().copied().collect::<Vec<_>>();
                death_messages.sort_unstable();
                json!({
                    "role": actor.role,
                    "account": actor.account,
                    "end_tick": end_ticks[index],
                    "cell": actor.label,
                    "quest": actor.quest,
                    "display": actor.display,
                    "observed_ticks": actor.snapshot.tick(),
                    "observed_tick_rate": actor.observed_tick_rate.receipt(),
                    "quest_complete": quest_done(&actor.snapshot, actor.display)
                        || actor.owned_card_complete,
                    "last_tile": actor.last_tile.map(|tile| [tile.x, tile.z, tile.level]),
                    "settings": actor.start_settings,
                    "status_log": status_logs[index],
                    "fixture_receipt": actor.start_receipt,
                    "starts": actor.starts,
                    "stops": actor.stops,
                    "transitions": actor.transitions,
                    "stages_seen": actor.stages_seen,
                    "deaths": actor.deaths_total,
                    "death_count_total_across_runs": actor.deaths_total,
                    "death_sent": actor.death_sent,
                    "death_injections": actor.death_injections,
                    "death_observed": actor.death_seen,
                    "death_status": actor.death_status,
                    "death_target": actor.death_target,
                    "death_combat_ownership": actor
                        .death_combat
                        .as_ref()
                        .map(owned_combat_json),
                    "death_chat_sequence_baseline": actor.death_chat_baseline,
                    "death_message_sequences": death_messages,
                    "pair_cancelled": actor.pair_cancelled,
                    "step_observations": actor.step_observations,
                    "tick_observations": actor.tick_observations,
                    "observed_tick_mean_ms": if quester_path {
                        actor.tick_mean.mean_ms()
                    } else {
                        None
                    },
                    "observed_tick_intervals": if quester_path {
                        actor.tick_mean.intervals
                    } else {
                        0
                    },
                    "stage_ticks": actor.stage_ticks,
                    "captures": actor.captures,
                    "final_status": final_statuses[index].as_deref().map(status_json),
                    "lifecycle": format!("{:?}", final_lifecycles[index]),
                    "error": actor.error,
                    "capture_error": actor.capture_error,
                    "end_captured": actor.end_captured,
                })
            })
            .collect::<Vec<_>>()
    };
    let inventory_proof_receipt = {
        let state = shared.lock().map_err(|_| "live state poisoned")?;
        state.inventory_proof.as_ref().map(|proof| {
            json!({
                "item_id": proof.item_id,
                "boundary_counts": proof.boundary_counts,
                "maximum_after_restart": proof.maximum_after_restart,
                "initially_absent": proof.initially_absent,
                "initial_absence_by_role": state.inventory_initial_absence,
                "boundary": state.inventory_boundary,
                "post_restart_samples": state.inventory_restart_samples,
                "last_counts": state.inventory_last_counts,
                "violation": state.inventory_violation,
            })
        })
    };
    let mut receipt = if mode.is_pair() {
        json!({
            "cell": run_label,
            "quest": actor_receipts[0]["quest"],
            "mode": mode.name(),
            "result": format!("{result:?}"),
            "run_result": format!("{run_result:?}"),
            "end_capture_wait_ms": end_capture_wait_ms,
            "game_port": profile.client().game_port(),
            "speed_restore_enabled": restore_world_speed,
            "nav_pack": profile.nav_pack(),
            "roles": actor_receipts,
            "inventory_proof": inventory_proof_receipt,
        })
    } else {
        let actor = &actor_receipts[0];
        // Keep the flat receipt below json!'s token-recursion limit in every
        // integration crate that includes this shared runner.
        Value::Object(Map::from_iter([
            ("cell".into(), actor["cell"].clone()),
            ("quest".into(), actor["quest"].clone()),
            ("mode".into(), json!(mode.name())),
            ("result".into(), json!(format!("{result:?}"))),
            ("run_result".into(), json!(format!("{run_result:?}"))),
            ("end_tick".into(), actor["end_tick"].clone()),
            ("end_capture_wait_ms".into(), json!(end_capture_wait_ms)),
            ("account".into(), actor["account"].clone()),
            ("observed_ticks".into(), actor["observed_ticks"].clone()),
            (
                "observed_tick_rate".into(),
                actor["observed_tick_rate"].clone(),
            ),
            ("quest_complete".into(), actor["quest_complete"].clone()),
            (
                "step_observations".into(),
                actor["step_observations"].clone(),
            ),
            ("game_port".into(), json!(profile.client().game_port())),
            ("speed_restore_enabled".into(), json!(restore_world_speed)),
            ("nav_pack".into(), json!(profile.nav_pack())),
            ("settings".into(), actor["settings"].clone()),
            ("fixture_receipt".into(), actor["fixture_receipt"].clone()),
            ("family_receipt".into(), json!(family_receipt)),
            ("starts".into(), actor["starts"].clone()),
            ("stops".into(), actor["stops"].clone()),
            ("transitions".into(), actor["transitions"].clone()),
            ("stages_seen".into(), actor["stages_seen"].clone()),
            (
                "tick_observations".into(),
                actor["tick_observations"].clone(),
            ),
            (
                "observed_tick_mean_ms".into(),
                actor["observed_tick_mean_ms"].clone(),
            ),
            (
                "observed_tick_intervals".into(),
                actor["observed_tick_intervals"].clone(),
            ),
            ("stage_ticks".into(), actor["stage_ticks"].clone()),
            ("deaths".into(), actor["deaths"].clone()),
            (
                "death_count_total_across_runs".into(),
                actor["death_count_total_across_runs"].clone(),
            ),
            ("death_sent".into(), actor["death_sent"].clone()),
            ("death_observed".into(), actor["death_observed"].clone()),
            ("death_injections".into(), actor["death_injections"].clone()),
            ("death_target".into(), actor["death_target"].clone()),
            (
                "death_combat_ownership".into(),
                actor["death_combat_ownership"].clone(),
            ),
            (
                "death_message_sequences".into(),
                actor["death_message_sequences"].clone(),
            ),
            ("death_status".into(), actor["death_status"].clone()),
            ("last_tile".into(), actor["last_tile"].clone()),
            ("captures".into(), actor["captures"].clone()),
            ("final_status".into(), actor["final_status"].clone()),
            ("lifecycle".into(), actor["lifecycle"].clone()),
            ("status_log".into(), actor["status_log"].clone()),
            ("error".into(), actor["error"].clone()),
            ("capture_error".into(), actor["capture_error"].clone()),
            ("end_captured".into(), actor["end_captured"].clone()),
        ]))
    };
    let logout_deadline = Instant::now() + CLEAN_LOGOUT_DEADLINE;
    let mut teardown = TeardownSchedule::new(names.len(), logout_deadline);
    match shared.lock() {
        Ok(mut state) => state.teardown_started = true,
        Err(_) => result = Err("live state poisoned before logout teardown".into()),
    }
    for account in &names {
        play.script_stop(account);
    }
    let mut observed_ticks = Vec::with_capacity(names.len());
    let mut logout_retry_ready = Vec::with_capacity(names.len());
    let mut disconnected = Vec::with_capacity(names.len());
    loop {
        let now = Instant::now();
        if teardown.deadline_expired(now) {
            break;
        }
        observed_ticks.clear();
        logout_retry_ready.clear();
        match shared.lock() {
            Ok(state) => {
                observed_ticks.extend(state.actors.iter().map(|actor| actor.snapshot.tick()));
                logout_retry_ready
                    .extend(state.actors.iter().map(|actor| actor.logout_retry_ready));
                if let Some(error) = state
                    .actors
                    .iter()
                    .find_map(|actor| actor.logout_error.as_ref())
                {
                    result = Err(error.clone());
                    break;
                }
            }
            Err(_) => {
                result = Err("live state poisoned during logout teardown".into());
                break;
            }
        }
        for (index, account) in names.iter().enumerate() {
            if teardown.stop_ack_tick(index).is_none() && start_handle.idle(account) {
                if let Some(tick) = observed_ticks.get(index).copied() {
                    teardown.acknowledge_stop(index, tick);
                    match shared.lock() {
                        Ok(mut state) => {
                            let actor = &mut state.actors[index];
                            actor.logout = Some(scenario::teardown::CombatLogout::new(
                                &actor.snapshot,
                                logout_deadline,
                            ));
                        }
                        Err(_) => {
                            result = Err("live state poisoned after script Stop".into());
                            break;
                        }
                    }
                }
            }
        }
        for (index, account) in names.iter().enumerate() {
            let now = Instant::now();
            if !teardown.initial_logout_due(index, now) {
                continue;
            }
            let Some(tick) = observed_ticks.get(index).copied() else {
                continue;
            };
            if let Some(arm) = play.arm(account) {
                if teardown.record_logout_attempt(index, tick, now) {
                    arm.request_logout();
                    play.wake(account);
                }
            } else {
                teardown.mark_no_logout_arm(index);
            }
        }
        for (index, account) in names.iter().enumerate() {
            let connected = play.slot_connected(account);
            let now = Instant::now();
            let Some(tick) = observed_ticks.get(index).copied() else {
                continue;
            };
            if !logout_retry_ready[index] || !teardown.retry_due(index, tick, connected, now) {
                continue;
            }
            if let Some(arm) = play.arm(account) {
                if teardown.record_logout_attempt(index, tick, now) {
                    arm.request_logout();
                    play.wake(account);
                }
            } else {
                teardown.mark_no_logout_arm(index);
            }
        }
        disconnected.clear();
        disconnected.extend(names.iter().map(|account| !play.slot_connected(account)));
        if teardown.complete(&disconnected) || teardown.deadline_expired(Instant::now()) {
            break;
        }
        std::thread::sleep(POLL_INTERVAL);
    }
    disconnected.clear();
    disconnected.extend(names.iter().map(|account| !play.slot_connected(account)));
    let failed_accounts = names
        .iter()
        .enumerate()
        .filter(|(index, _)| {
            teardown.stop_ack_tick(*index).is_none()
                || teardown.logout_attempt_ticks(*index).is_empty()
                || !disconnected[*index]
        })
        .map(|(_, account)| account.clone())
        .collect::<Vec<_>>();
    let logout_complete = teardown.complete(&disconnected);
    for account in &names {
        play.stop_slot(account);
    }
    receipt["clean_logout"] = json!({
        "complete": logout_complete,
        "timeout_ms": CLEAN_LOGOUT_DEADLINE.as_millis(),
        "combat_retry_game_ticks": COMBAT_LOGOUT_RETRY_TICKS,
        "engine_content": "content/scripts/login_logout/logout.rs2:27",
        "stop_ack_api": "ScriptStartHandle::idle",
        "tick_source": "GameSnapshot::tick from observed PLAYER_INFO packets",
        "combat_escape": shared.lock().ok().map(|state| state.actors.iter().map(|actor| json!({
            "account": actor.account,
            "refusal_sequence": actor.logout.as_ref().and_then(|logout| logout.refusal_sequence()),
            "account_only_teleport_sent": actor.logout.as_ref().is_some_and(|logout| logout.escape_sent()),
            "safe_arrival_tick": actor.logout.as_ref().and_then(|logout| logout.safe_arrival_tick()),
            "safe_tile": scenario::teardown::LOGOUT_SAFE_TILE,
        })).collect::<Vec<_>>()),
        "accounts": names
            .iter()
            .zip(&disconnected)
            .enumerate()
            .map(|(index, (account, disconnected))| json!({
                "account": account,
                "script_stop_requested": true,
                "stop_acknowledged": teardown.stop_ack_tick(index).is_some(),
                "stop_ack_tick": teardown.stop_ack_tick(index),
                "logout_attempts": teardown
                    .logout_attempt_ticks(index)
                    .iter()
                    .enumerate()
                    .map(|(attempt, tick)| json!({
                        "attempt": attempt + 1,
                        "game_tick": tick,
                    }))
                    .collect::<Vec<_>>(),
                "disconnected": disconnected,
            }))
            .collect::<Vec<_>>(),
    });
    if !logout_complete {
        let reason = format!(
            "clean logout teardown failed for {}",
            failed_accounts.join(", ")
        );
        result = match result {
            Ok(()) => Err(reason),
            Err(previous) => Err(format!("{previous}; {reason}")),
        };
    }
    receipt["result"] = json!(format!("{result:?}"));
    for (index, directory) in directories.iter().enumerate() {
        let mut role_receipt = receipt.clone();
        if mode.is_pair() {
            role_receipt["receipt_role"] = json!(index);
            role_receipt["account"] = json!(names[index]);
        }
        std::fs::write(
            directory.join("receipt.json"),
            serde_json::to_vec_pretty(&role_receipt).map_err(|error| error.to_string())?,
        )
        .map_err(|error| error.to_string())?;
    }
    println!("{receipt}");
    result.map(|()| receipt)
}

/// One Path-backed fixture cell (`scenario::quester::quester_stage`): the
/// quest's `FIXTURE_PROFILES` row, its varp-hinted stage, the chosen kit and
/// extras, and an explicitly selected stand tile that may differ from the
/// Path's authored not-started anchor. Start uses the fixture's own settings;
/// the fixture seed's `observe_start` proves the exact stats/kit/tile at Start.
/// One random event summoned while the named step is in flight.
struct MacroEventPlan {
    event: u8,
    step: String,
    sent: bool,
    /// NPC indexes in view when the cheat was sent.
    before: HashSet<usize>,
    /// Chat sequence when the cheat was sent.
    chat_baseline: i32,
    npc_seen: bool,
    last_hold: bool,
    chat_logged: usize,
}

fn macro_event_plan_from_env() -> Option<MacroEventPlan> {
    let event = std::env::var("QUESTER_MACRO_EVENT").ok()?;
    let event: u8 = event
        .parse()
        .ok()
        .filter(|event| (1..=8).contains(event))
        .expect("QUESTER_MACRO_EVENT is 1-8");
    let step = std::env::var("QUESTER_MACRO_EVENT_STEP")
        .expect("QUESTER_MACRO_EVENT_STEP names the step to interrupt");
    Some(MacroEventPlan {
        event,
        step,
        sent: false,
        before: HashSet::new(),
        chat_baseline: 0,
        npc_seen: false,
        last_hold: false,
        chat_logged: 0,
    })
}

pub struct PathCell<'a> {
    /// Content quest id; the Path body comes from the release index, or from
    /// the folder the `FolderSource` registry currently serves.
    pub quest: &'static str,
    /// Selected quest-tab display (checked against the identity row).
    pub display: &'static str,
    pub label: String,
    /// Optional authored stage; `None` starts from clean quest state.
    pub stage: Option<&'a str>,
    pub loadout: Option<scenario::quester::FixtureLoadout<'a>>,
    pub extra_items: &'a [(&'a str, i32)],
    /// Explicit operator seeds for hint-less stages (released Cook: the stage
    /// varp and value, recorded like every other cheat). See
    /// `scenario::quester::quester_stage_with_seeds`.
    pub seed_vars: &'a [(&'a str, i32)],
    /// Selected starting tile; may be a bank instead of the Path anchor.
    pub stand: WorldTile,
    pub mode: Mode,
    /// Auxiliary pre-Start setup (bank stock, prerequisite quest flags),
    /// inserted before the fixture's final relog. Never gameplay.
    pub before_relog: Vec<scenario::Step>,
}

pub fn path_cell(spec: PathCell<'_>) -> Result<Cell, String> {
    let (path, _) = scenario::quester::load_quester_path_document(spec.quest)?;
    let selected = api::game_data::for_revision(api::selected::ClientRevision::R289)?;
    let fixture =
        scenario::quester::build_quester_path_fixture(scenario::quester::QuesterPathFixture {
            name: spec.quest,
            quest_display: spec.display,
            path: &path,
            selected: &selected,
            stage: spec.stage,
            loadout: spec.loadout,
            extra_items: spec.extra_items,
            seed_vars: spec.seed_vars,
            stand: spec.stand,
            before_relog: spec.before_relog,
        })?;
    let seed = fixture.seed;
    Ok(Cell {
        quest: spec.quest,
        display: spec.display,
        label: spec.label,
        scenario: fixture.scenario,
        start_settings: fixture.start_settings,
        mode: spec.mode,
        observe_start: Some(Box::new(move |snapshot| seed.observe_start(snapshot))),
    })
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn observed_tick_rate_distinguishes_300_and_600_millisecond_sequences() {
        for milliseconds in [300, 600] {
            let start = Instant::now();
            let mut rate = ObservedTickRate::default();
            assert_eq!(rate.receipt()["mean_tick_ms"], Value::Null);
            for tick in 1..=4 {
                rate.observe(
                    tick,
                    start + Duration::from_millis(u64::from(tick - 1) * milliseconds),
                );
            }
            let receipt = rate.receipt();
            assert_eq!(receipt["sampled_game_ticks"], 3);
            assert_eq!(receipt["mean_tick_ms"], milliseconds as f64);
            assert_eq!(receipt["minimum_tick_ms"], milliseconds as f64);
            assert_eq!(receipt["maximum_tick_ms"], milliseconds as f64);
        }
    }

    #[test]
    fn observed_tick_rate_ignores_duplicate_polls_and_counter_reset_gaps() {
        let start = Instant::now();
        let mut rate = ObservedTickRate::default();
        rate.observe(10, start);
        rate.observe(10, start + Duration::from_secs(5));
        rate.observe(11, start + Duration::from_millis(300));
        rate.observe(1, start + Duration::from_secs(20));
        rate.observe(2, start + Duration::from_millis(20_600));
        let receipt = rate.receipt();
        assert_eq!(receipt["interval_samples"], 2);
        assert_eq!(receipt["sampled_game_ticks"], 2);
        assert_eq!(receipt["mean_tick_ms"], 450.0);
        assert_eq!(receipt["counter_resets"], 1);
    }

    #[test]
    fn observed_tick_rate_normalizes_batched_game_ticks() {
        let start = Instant::now();
        let mut rate = ObservedTickRate::default();
        rate.observe(1, start);
        rate.observe(3, start + Duration::from_millis(600));
        assert_eq!(rate.receipt()["mean_tick_ms"], 300.0);
        assert_eq!(rate.receipt()["sampled_game_ticks"], 2);
        assert_eq!(rate.receipt()["interval_samples"], 1);
    }

    #[test]
    fn native_family_start_requires_an_explicit_override_and_named_script() {
        assert!(has_native_start(Some("Quester"), false));
        assert!(has_native_start(Some("Sherlock"), true));
        assert!(!has_native_start(Some("Sherlock"), false));
        assert!(!has_native_start(None, true));
        assert!(!has_native_start(Some(""), true));
    }

    #[test]
    fn named_stop_mode_accepts_only_the_explicit_native_park() {
        use script::native::{ScriptFailure, StatusField};

        let reason = "GAP-test-drop-budget";
        let mut status = ScriptStatus {
            run: RunKey {
                slot: 1,
                run: 1,
                session: 1,
            },
            card: script::CompiledId("Quester"),
            phase: NativePhase::Blocked,
            active_settings: 0,
            pending_settings: None,
            fields: Arc::from([StatusField {
                key: "block_reason",
                label: "Block reason",
                value: StatusValue::Text(Arc::from(reason)),
            }]),
            failure: Some(ScriptFailure {
                code: Arc::from("parked"),
                message: Arc::from(reason),
            }),
        };
        assert!(has_named_stop(Some(&status), reason));
        assert!(parked(&status).is_some());
        assert!(!has_named_stop(None, reason));
        assert!(!has_named_stop(Some(&status), "another stop"));
        status.phase = NativePhase::Working;
        let failure = status.failure.take();
        assert!(
            parked(&status).is_none(),
            "block_reason precedes native Blocked publication"
        );
        assert!(!has_named_stop(Some(&status), reason));
        status.phase = NativePhase::Blocked;
        status.failure = failure;
        status.failure.as_mut().unwrap().code = Arc::from("failed");
        assert!(!has_named_stop(Some(&status), reason));
        status.failure.as_mut().unwrap().code = Arc::from("parked");
        status.fields = Arc::from([]);
        assert!(!has_named_stop(Some(&status), reason));
        let mode = RunPlan::Single(Mode::ExpectedNamedStop {
            reason: reason.into(),
        });
        assert_eq!(mode.expected_named_stop(), Some(reason));
        assert!(RunPlan::Single(Mode::Clean).expected_named_stop().is_none());
    }

    #[test]
    fn prereq_negative_oracle_guard_requires_opt_in_and_native_blocked() {
        let mut status = ScriptStatus {
            run: api::selected::RunKey {
                slot: 1,
                run: 1,
                session: 1,
            },
            card: script::CompiledId("Quester"),
            phase: NativePhase::Blocked,
            active_settings: 1,
            pending_settings: None,
            failure: Some(script::native::ScriptFailure {
                code: "parked".into(),
                message: "expected refusal".into(),
            }),
            fields: Arc::from([]),
        };
        assert!(family_parked_error(&status, FamilyBlockedPolicy::Reject).is_some());
        assert!(family_parked_error(&status, FamilyBlockedPolicy::Observe).is_none());
        status.failure = None;
        assert!(family_parked_error(&status, FamilyBlockedPolicy::Observe).is_some());
        status.phase = NativePhase::Working;
        status.fields = Arc::from([script::native::StatusField {
            key: "block_reason",
            label: "Block",
            value: StatusValue::Text("expected refusal".into()),
        }]);
        assert!(family_parked_error(&status, FamilyBlockedPolicy::Observe).is_some());
    }

    fn settings(gang: &str) -> Map<String, Value> {
        Map::from_iter([("gang".into(), json!(gang))])
    }

    #[test]
    fn pair_gangs_are_explicit_and_canonically_opposite() {
        assert!(validate_pair_gangs(&settings("phoenix"), &settings("blackarm")).is_ok());
        assert!(validate_pair_gangs(&settings("phoenix"), &settings("phoenix")).is_err());
        assert!(validate_pair_gangs(&settings("blackarm"), &settings("phoenix")).is_err());
        assert!(validate_pair_gangs(&Map::new(), &settings("blackarm")).is_err());
    }

    #[test]
    fn pair_targets_reject_unknown_roles_and_empty_keys() {
        assert!(validate_pair_mode(&PairMode::Restart {
            at: [
                PairStage {
                    role: 0,
                    stage: "blackarmgang:3".into(),
                },
                PairStage {
                    role: 2,
                    stage: "blackarmgang:4".into(),
                },
            ],
        })
        .is_err());
        assert!(validate_pair_mode(&PairMode::Death {
            at: PairStage {
                role: 1,
                stage: String::new(),
            },
            step: "hand-in".into(),
        })
        .is_err());
    }

    #[test]
    fn paired_names_keep_the_assigned_role_prefix_and_shared_nonce() {
        let mut names = ["qk12345678_0".to_owned(), "qk12345678_1".to_owned()];
        set_pair_name_prefixes(&mut names, "qk", "ql").unwrap();
        assert_eq!(names, ["qk12345678_0", "ql12345678_1"]);
        let mut names = ["qk12345678_0".to_owned(), "qk12345678_1".to_owned()];
        set_pair_name_prefixes(&mut names, "qk", "long").unwrap();
        assert_eq!(names, ["qk345678_0", "long345678_1"]);
        assert!(names.iter().all(|name| name.len() <= 12));
        let before = names.clone();
        assert!(set_pair_name_prefixes(&mut names, "qk", "oversized").is_err());
        assert_eq!(names, before);
    }

    #[test]
    fn ordinary_role_death_never_enables_reserved_pair_restart_recovery() {
        let target = PairStage {
            role: 1,
            stage: "hero:blackarm:armband".into(),
        };
        let mode = PairMode::DeathIndependent {
            at: target.clone(),
            step: "eel-fight-jailer".into(),
        };
        assert!(validate_pair_mode(&mode).is_ok());
        let run = RunPlan::Pair(mode);
        let (death, step) = run.death_target().unwrap();
        assert_eq!(death.role, target.role);
        assert_eq!(death.stage, target.stage);
        assert_eq!(step, "eel-fight-jailer");
        assert!(run.pair_death_target().is_none());
        assert!(run.restart_targets().is_none());
        let reserved = RunPlan::Pair(PairMode::Death { at: target, step });
        assert!(reserved.pair_death_target().is_some());
        assert!(reserved.restart_targets().is_some());
    }

    #[test]
    fn inventory_proof_accepts_multiple_real_certificates_and_rejects_empty_counts() {
        let proof = PairInventoryProof {
            item_id: 3,
            boundary_counts: [2, 0],
            maximum_after_restart: [2, 1],
            initially_absent: vec![1, 2, 3],
        };
        assert!(valid_inventory_proof(&proof));

        let mut empty_boundary = proof;
        empty_boundary.boundary_counts = [0, 0];
        assert!(!valid_inventory_proof(&empty_boundary));
    }

    #[test]
    fn stage_proof_needs_a_transition_or_terminal_receipt() {
        let expect = vec!["quest:2".to_owned()];
        assert!(!stage_or_terminal(&expect, Some("quest:2"), false, false));
        assert!(stage_or_terminal(&expect, Some("quest:2"), true, false));
        assert!(stage_or_terminal(&[], None, false, true));
        assert!(!stage_or_terminal(&[], None, false, false));
    }

    #[test]
    fn completion_requires_runner_progress_idle_and_completed_lifecycle() {
        assert!(completion_proven(true, true, true, true));
        assert!(!completion_proven(false, true, true, true));
        assert!(!completion_proven(true, false, true, true));
        assert!(!completion_proven(true, true, false, true));
        assert!(!completion_proven(true, true, true, false));
    }

    #[test]
    fn path_terminal_proof_needs_idle_and_completed_lifecycle() {
        assert!(path_terminal_proven(true, true));
        assert!(!path_terminal_proven(false, true));
        assert!(!path_terminal_proven(true, false));
    }
}

#[cfg(test)]
mod teardown_tests {
    use super::*;

    #[test]
    fn logout_waits_for_every_script_stop_acknowledgement() {
        let now = Instant::now();
        let mut teardown = TeardownSchedule::new(2, now + CLEAN_LOGOUT_DEADLINE);

        assert!(!teardown.initial_logout_due(0, now));
        assert!(!teardown.record_logout_attempt(0, 100, now));
        teardown.acknowledge_stop(0, 100);
        assert!(!teardown.initial_logout_due(0, now));
        assert!(!teardown.record_logout_attempt(0, 100, now));
        teardown.acknowledge_stop(1, 100);
        assert!(teardown.initial_logout_due(0, now));
        assert!(teardown.record_logout_attempt(0, 100, now));
    }

    #[test]
    fn unchanged_game_tick_does_not_retry_logout() {
        let now = Instant::now();
        let mut teardown = TeardownSchedule::new(1, now + CLEAN_LOGOUT_DEADLINE);
        teardown.acknowledge_stop(0, 100);
        assert!(teardown.record_logout_attempt(0, 100, now));

        let later_wall_time = now + Duration::from_secs(25);
        assert!(!teardown.retry_due(0, 100, true, later_wall_time));
        assert!(!teardown.retry_due(0, 100, true, now));
        assert!(!teardown.retry_due(0, 115, true, now));
    }

    #[test]
    fn refused_logout_retries_after_sixteen_new_game_ticks() {
        let now = Instant::now();
        let mut teardown = TeardownSchedule::new(2, now + CLEAN_LOGOUT_DEADLINE);
        teardown.acknowledge_stop(0, 100);
        teardown.acknowledge_stop(1, 104);
        assert!(teardown.record_logout_attempt(0, 100, now));
        assert!(teardown.record_logout_attempt(1, 104, now));

        assert!(!teardown.retry_due(0, 115, true, now));
        assert!(teardown.retry_due(0, 116, true, now));
        assert!(!teardown.retry_due(1, 116, true, now));
        assert!(teardown.retry_due(1, 120, true, now));
        assert!(!teardown.retry_due(0, 116, false, now));
    }

    #[test]
    fn fixed_deadline_fails_while_an_account_remains_connected() {
        let started = Instant::now();
        let deadline = started + CLEAN_LOGOUT_DEADLINE;
        let mut teardown = TeardownSchedule::new(1, deadline);
        teardown.acknowledge_stop(0, 200);
        assert!(teardown.record_logout_attempt(0, 200, started));

        assert!(teardown.deadline_expired(deadline));
        assert!(!teardown.record_logout_attempt(0, 216, deadline));
        assert!(!teardown.retry_due(0, 216, true, deadline));
        assert!(!teardown.complete(&[false]));
    }

    #[test]
    fn teardown_stops_queued_actor_work() {
        assert!(actor_frame_enabled(false));
        assert!(!actor_frame_enabled(true));
    }
}
