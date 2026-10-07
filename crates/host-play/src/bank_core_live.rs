//! BANK-CORE live cells at a real Draynor booth through real Play, on the
//! shared transfer kernel.
//!
//! A fresh account is logged in through real Play. The shared ScenarioRunner
//! gates tutorial-skip/relog once on the server logout and the rebound
//! inventory tab; the bank floor, cleared pack and account-owned seeds are
//! then established before the cell clock starts. Each cell saves a JSON
//! receipt and CPU-rendered PNG below `LIVE_EVIDENCE_DIR`.
//!
//! Login has its own bounded prerequisite deadline. The cell's fixed bound
//! starts only after `ingame && scene_state == 2` and its fixture seed is
//! posted, and covers the scripted operation.
//!
//! - **L-withdraw** (60 s): the native script selects Draynor, opens its
//!   booth, withdraws to exactly 7 of 50 banked coins and closes.
//! - **L-load** (90 s): compat `Bank.withdrawLoad('Feather')` with 300
//!   feathers banked and 20 free slots empties that row into the pack.
//! - **L-load-unstack** (120 s): compat `Bank.withdrawLoad('Logs')` with
//!   40 logs banked and 20 free slots fills the pack and leaves 20 banked.
//! - **L-close** (20 s): compat `Bank.close()` on the open booth is true
//!   within 4 s with the bank shut, its side root released and the session
//!   generation newer; a second close on the shut bank is true at once.
//! - **L-except** (60 s): compat host `Bank.depositAllExcept(['Coins'])`
//!   with coins, bones and a tinderbox held banks the junk and keeps the
//!   coins.
//!
//! The compat cells run a frozen-API Load script that opens the booth
//! itself; the cell decides from the posted snapshot after the script's
//! answer, not from the answer alone.
//!
//! Like the Gatherer live cells, the client cache is the revision's decoded
//! snapshot copied once and reused: `BOT_CACHE_DIR` is the copied versioned
//! snapshot and `CLIENT_UNPACK_DIR` its copied unpack root. A launch never
//! fetches or deletes that immutable cache.
//!
//! `LIVE=1 BOT_CPU=1 BOT_LIVE_NAME_PREFIX=bk GATHERER_NAV_PACK=<pack> GATHERER_ENGINE_DIR=<engine> GATHERER_CATALOG_ROOT=<catalog> GATHERER_GAME_PORT=<port> GATHERER_HTTP_PORT=<port> BOT_CACHE_DIR=<unpack-root>/<version> CLIENT_UNPACK_DIR=<unpack-root> LIVE_EVIDENCE_DIR=<evidence-root> cargo test -p host-play --lib live_bank_core_ -- --ignored --nocapture --test-threads=1`

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::task::Poll;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use api::interact;
use api::named_banks::{BankPreferences, NamedBankFacts};
use api::snapshot::{GameSnapshot, WorldTile};
use client::client::Client;
use parking_lot::Mutex;
use script::bank::{Close, Open, OpenArgs, Select, SelectArgs, Withdraw, WithdrawArgs, Withdrawal};
use script::native::{ActionHandle, NativeTick, Script, ScriptFailure, ScriptFlow};
use vault::{Profile, ProfileSettings};

use super::bank_npc_live::render_teller_frame;
use super::evidence_writer::{
    write_failure_sidecar, EvidenceJob, EvidenceRequest, EvidenceSidecar, EvidenceWriter, PngColor,
};
use super::{run_with_template, tele_args, ProfileOptions, SharedClientTemplate};

const COINS: i32 = 995;
const SEEDED: i32 = 50;
const TARGET: i32 = 7;
/// Draynor bank floor, inside the booth row.
pub(super) const DRAYNOR: WorldTile = WorldTile {
    x: 3092,
    z: 3243,
    level: 0,
};
const CELL_BOUND: Duration = Duration::from_secs(60);
// Keep the login prerequisite separate from every cell's deadline, with room
// for the server's 60-second already-logged-in retry message.
pub(super) const LOGIN_DEADLINE: Duration = Duration::from_secs(90);
pub(super) const PREPARATION_DEADLINE: Duration = Duration::from_secs(180);
pub(super) const CAPTURE_GRACE: Duration = Duration::from_secs(10);
const FEATHER: i32 = 314;
const LOGS: i32 = 1511;
const BONES: i32 = 526;
const TINDERBOX: i32 = 590;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Prep {
    Session,
    TutorialReseed,
    WaitTutorialReseed,
    Teleport,
    WaitArrive,
    Clear,
    WaitClear,
    Seed,
    Ready,
    PrerequisiteFailed,
    Failed,
}

#[derive(Debug, Clone, Default, serde::Serialize)]
pub(super) struct Trace {
    bank: Option<String>,
    access_kind: Option<String>,
    access_tile: Option<[i32; 3]>,
    held_before: Option<i32>,
    bank_before: Option<i32>,
    complete: Option<bool>,
    held_after: Option<i32>,
    bank_after: Option<i32>,
    closed: bool,
    final_tile: Option<[i32; 3]>,
    script_ms: Option<u128>,
    preparation_ms: Option<u128>,
    cell_ms: Option<u128>,
    live_held: Option<i32>,
    live_bank_open: Option<bool>,
    pub(super) failure: Option<String>,
    pub(super) passed: bool,
}

/// The posted facts a cell decides from, refreshed every in-game frame.
#[derive(Debug, Clone, Default, serde::Serialize)]
struct Facts {
    /// Frames read so far: a verdict waits for one after the answer.
    frame: u64,
    /// `(id, held)` for the cell's watched ids.
    held: Vec<(i32, i32)>,
    /// `(id, banked)` while the bank's stock is posted.
    banked: Option<Vec<(i32, i32)>>,
    used: i32,
    inventory_size: i32,
    bank_open: bool,
    side_root: i32,
    generation: u64,
    /// The newest bank session generation posted while the bank was open.
    open_generation: Option<u64>,
}

impl Facts {
    fn held(&self, id: i32) -> i32 {
        self.held
            .iter()
            .find(|(held, _)| *held == id)
            .map_or(0, |(_, count)| *count)
    }

    fn banked(&self, id: i32) -> Option<i32> {
        self.banked.as_ref().map(|rows| {
            rows.iter()
                .find(|(banked, _)| *banked == id)
                .map_or(0, |(_, count)| *count)
        })
    }
}

pub(super) struct Cell {
    pub(super) phase: Prep,
    bound: Duration,
    /// Set only once the final seed is posted in an in-game scene-2 frame.
    pub(super) started: Option<Instant>,
    pub(super) preparation_started: Instant,
    pub(super) last_action: Instant,
    session_runner: scenario::ScenarioRunner,
    preparation_failure: Option<String>,
    /// Post-relog tutorial reseed (`setvar tutorial 1000` + fresh `getvar`
    /// confirm in the new session, after the kit-close queue has run).
    tutorial_reseed: Option<api::interact::PostRelogTutorial>,
    /// Seed cheats sent in order once the pack is cleared.
    seed: &'static [&'static str],
    seeded: usize,
    /// Whether the posted pack shows the seed (bank seeds are not visible).
    seed_ready: fn(&GameSnapshot) -> bool,
    watch: &'static [i32],
    facts: Facts,
    pub(super) script_started: Option<Instant>,
    pub(super) trace: Trace,
    /// The compat script's own answer.
    pub(super) result: Option<serde_json::Value>,
    /// The receipt's fixed part: scenario, seed and request.
    scenario: serde_json::Value,
    /// Whether the posted frame matches a passing verdict before capture.
    capture_ready: fn(&Cell) -> bool,
    pub(super) terminal: bool,
    evidence_dir: PathBuf,
    capture_started: bool,
    pub(super) capture_written: bool,
    pub(super) capture_error: Option<String>,
    /// Shared background evidence writer; the hook submits the rendered
    /// frame and marks the capture written only once the files are durable.
    writer: Arc<EvidenceWriter>,
    capture_job: Option<EvidenceJob>,
    /// The frame snapshot, rebuilt in place as the host keeps its own: the
    /// bank session generation is tracked across frames.
    snapshot: Option<GameSnapshot>,
}

impl Cell {
    pub(super) fn new(
        bound: Duration,
        seed: &'static [&'static str],
        seed_ready: fn(&GameSnapshot) -> bool,
        watch: &'static [i32],
        scenario: serde_json::Value,
        capture_ready: fn(&Cell) -> bool,
        evidence_dir: PathBuf,
    ) -> Self {
        let preparation_started = Instant::now();
        Self {
            phase: Prep::Session,
            bound,
            started: None,
            preparation_started,
            last_action: preparation_started,
            session_runner: session_fixture_runner(),
            preparation_failure: None,
            tutorial_reseed: None,
            seed,
            seeded: 0,
            seed_ready,
            watch,
            facts: Facts::default(),
            script_started: None,
            trace: Trace::default(),
            result: None,
            scenario,
            capture_ready,
            terminal: false,
            evidence_dir,
            capture_started: false,
            capture_written: false,
            capture_error: None,
            writer: Arc::new(EvidenceWriter::new(
                super::evidence_writer::DEFAULT_QUEUE_BOUND,
            )),
            capture_job: None,
            snapshot: None,
        }
    }

    pub(super) fn fail(&mut self, message: String) {
        if !self.terminal {
            self.trace.failure = Some(message);
            self.terminal = true;
        }
        self.phase = Prep::Failed;
    }

    fn fail_prerequisite(&mut self, message: String) {
        if self.preparation_failure.is_none() {
            self.preparation_failure = Some(message);
        }
        self.phase = Prep::PrerequisiteFailed;
    }
}

/// The common tutorial/relog fixture used by live script scenarios. The
/// runner owns the one-shot logout and waits for both a departed session and
/// the new session's inventory side-tab readiness before setup continues.
fn session_fixture_runner() -> scenario::ScenarioRunner {
    use scenario::{Proof, Scenario, ScenarioSettings, Seed, Step, StepKind, Wait};

    let scenario = Scenario {
        name: "bank_core_session_fixture",
        seed: Seed {
            profiles: vec![],
            mainland: false,
        },
        steps: vec![
            Step {
                name: "skip tutorial and verify the server value",
                kind: StepKind::Perform {
                    send: Box::new(|client, _| {
                        let _ = interact::cheat(client, api::interact::TUTORIAL_SETVAR);
                        let _ = interact::cheat(client, api::interact::TUTORIAL_GETVAR);
                        true
                    }),
                },
                wait: Wait {
                    arm: Proof::Chat {
                        needle: api::interact::TUTORIAL_CHAT_NEEDLE,
                    },
                    budget_ticks: 200,
                },
            },
            Step {
                name: "relog once and wait for the inventory tab",
                kind: StepKind::Relog,
                wait: Wait {
                    arm: Proof::SideTabAvailable { index: 3 },
                    budget_ticks: 600,
                },
            },
        ],
        proof: Proof::SideTabAvailable { index: 3 },
        companions: vec![],
        settings: ScenarioSettings {
            deadline: PREPARATION_DEADLINE,
            ..ScenarioSettings::default()
        },
    };
    scenario::ScenarioRunner::with_world(scenario, None)
}

pub(super) fn count(items: &[api::snapshot::ItemView], id: i32) -> i32 {
    script::bank::ops::count_id(items, id)
}

fn tile(tile: WorldTile) -> [i32; 3] {
    [tile.x, tile.z, tile.level]
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Step {
    Select,
    Open,
    Withdraw,
    Close,
    Done,
}

struct WithdrawCell {
    facts: Arc<NamedBankFacts>,
    bank_name: Arc<str>,
    cell: Arc<Mutex<Cell>>,
    step: Step,
    select: Option<ActionHandle<Select>>,
    open: Option<ActionHandle<Open>>,
    withdraw: Option<ActionHandle<Withdraw>>,
    close: Option<ActionHandle<Close>>,
    access: Option<Arc<script::bank::BankStandAccess>>,
}

impl WithdrawCell {
    fn blocked(&self, message: impl Into<String>) -> ScriptFlow {
        let message = message.into();
        self.cell.lock().fail(message.clone());
        ScriptFlow::Blocked(ScriptFailure {
            code: "bank-core-live".into(),
            message: message.into(),
        })
    }
}

impl Script for WithdrawCell {
    fn tick(&mut self, tick: &mut NativeTick<'_>) -> Result<ScriptFlow, ScriptFailure> {
        match self.step {
            Step::Done => Ok(ScriptFlow::Complete),
            Step::Select => {
                if self.select.is_none() {
                    let args = SelectArgs {
                        facts: Arc::clone(&self.facts),
                        from: DRAYNOR,
                        preferences: BankPreferences {
                            use_mage_bank: false,
                            use_zanaris_bank: false,
                        },
                        options: script::native::WalkOptions::default(),
                        explicit: Some(Arc::clone(&self.bank_name)),
                    };
                    match tick.actions.begin::<Select>(args, &mut tick.cx) {
                        Ok(handle) => self.select = Some(handle),
                        Err(error) => return Ok(self.blocked(format!("Select begin: {error:?}"))),
                    }
                    return Ok(ScriptFlow::Continue);
                }
                match tick
                    .actions
                    .poll(self.select.as_ref().expect("Select handle"), &mut tick.cx)
                {
                    Poll::Pending => Ok(ScriptFlow::Continue),
                    Poll::Ready(Err(error)) => Ok(self.blocked(format!("Select: {error:?}"))),
                    Poll::Ready(Ok(selected)) => {
                        self.select = None;
                        let Some(access) = selected.access else {
                            return Ok(self.blocked("Select returned no stand access"));
                        };
                        {
                            let mut cell = self.cell.lock();
                            cell.trace.bank = self
                                .facts
                                .banks()
                                .get(selected.bank_index as usize)
                                .map(|bank| bank.name.to_owned());
                            cell.trace.access_kind = Some(access.kind.as_str().to_owned());
                            cell.trace.access_tile = Some(tile(selected.access_tile));
                        }
                        self.access = Some(access);
                        self.step = Step::Open;
                        Ok(ScriptFlow::Continue)
                    }
                }
            }
            Step::Open => {
                if self.open.is_none() {
                    let Some(access) = self.access.clone() else {
                        return Ok(self.blocked("Open has no selected access"));
                    };
                    match tick
                        .actions
                        .begin::<Open>(OpenArgs { access }, &mut tick.cx)
                    {
                        Ok(handle) => self.open = Some(handle),
                        Err(error) => return Ok(self.blocked(format!("Open begin: {error:?}"))),
                    }
                    return Ok(ScriptFlow::Continue);
                }
                match tick
                    .actions
                    .poll(self.open.as_ref().expect("Open handle"), &mut tick.cx)
                {
                    Poll::Pending => Ok(ScriptFlow::Continue),
                    Poll::Ready(Err(error)) => Ok(self.blocked(format!("Open: {error:?}"))),
                    Poll::Ready(Ok(())) => {
                        self.open = None;
                        let snapshot = tick.cx.snapshot();
                        let held = snapshot.inventory().map(|rows| count(rows.value, COINS));
                        let bank = snapshot.bank().map(|rows| count(rows.value, COINS));
                        {
                            let mut cell = self.cell.lock();
                            cell.trace.held_before = held;
                            cell.trace.bank_before = bank;
                        }
                        if held != Some(0) || bank.is_none_or(|bank| bank < TARGET) {
                            return Ok(self.blocked(format!(
                                "the opened bank is not the seeded start: held={held:?} bank={bank:?}"
                            )));
                        }
                        self.step = Step::Withdraw;
                        Ok(ScriptFlow::Continue)
                    }
                }
            }
            Step::Withdraw => {
                if self.withdraw.is_none() {
                    let args = WithdrawArgs {
                        withdrawals: Arc::from([Withdrawal {
                            id: COINS,
                            name: Arc::from("Coins"),
                            target: TARGET,
                        }]),
                    };
                    match tick.actions.begin::<Withdraw>(args, &mut tick.cx) {
                        Ok(handle) => self.withdraw = Some(handle),
                        Err(error) => return Ok(self.blocked(format!("Withdraw begin: {error:?}"))),
                    }
                    return Ok(ScriptFlow::Continue);
                }
                match tick.actions.poll(
                    self.withdraw.as_ref().expect("Withdraw handle"),
                    &mut tick.cx,
                ) {
                    Poll::Pending => Ok(ScriptFlow::Continue),
                    Poll::Ready(Err(error)) => Ok(self.blocked(format!("Withdraw: {error:?}"))),
                    Poll::Ready(Ok(complete)) => {
                        self.withdraw = None;
                        let snapshot = tick.cx.snapshot();
                        let held = snapshot.inventory().map(|rows| count(rows.value, COINS));
                        let bank = snapshot.bank().map(|rows| count(rows.value, COINS));
                        let before = {
                            let mut cell = self.cell.lock();
                            cell.trace.complete = Some(complete);
                            cell.trace.held_after = held;
                            cell.trace.bank_after = bank;
                            cell.trace.bank_before
                        };
                        let expected = before.map(|before| before - TARGET);
                        if !complete || held != Some(TARGET) || bank != expected {
                            return Ok(self.blocked(format!(
                                "exact withdraw not observed: complete={complete} held={held:?} bank={bank:?} expected_bank={expected:?}"
                            )));
                        }
                        self.step = Step::Close;
                        Ok(ScriptFlow::Continue)
                    }
                }
            }
            Step::Close => {
                if self.close.is_none() {
                    match tick.actions.begin::<Close>((), &mut tick.cx) {
                        Ok(handle) => self.close = Some(handle),
                        Err(error) => return Ok(self.blocked(format!("Close begin: {error:?}"))),
                    }
                    return Ok(ScriptFlow::Continue);
                }
                match tick
                    .actions
                    .poll(self.close.as_ref().expect("Close handle"), &mut tick.cx)
                {
                    Poll::Pending => Ok(ScriptFlow::Continue),
                    Poll::Ready(Err(error)) => Ok(self.blocked(format!("Close: {error:?}"))),
                    Poll::Ready(Ok(())) => {
                        self.close = None;
                        let snapshot = tick.cx.snapshot();
                        let open = snapshot.bank_session().map(|session| session.value.open);
                        let here = snapshot.here().map(|here| tile(here.value));
                        let mut cell = self.cell.lock();
                        cell.trace.closed = open == Some(false);
                        cell.trace.final_tile = here;
                        cell.trace.script_ms =
                            cell.script_started.map(|at| at.elapsed().as_millis());
                        let started = cell
                            .started
                            .expect("fixture readiness starts the bank-cell clock");
                        let elapsed = started.elapsed();
                        cell.trace.cell_ms = Some(elapsed.as_millis());
                        if !cell.trace.closed {
                            drop(cell);
                            return Ok(self.blocked("Close returned with the bank still open"));
                        }
                        if elapsed > CELL_BOUND {
                            let message =
                                format!("the cell took {elapsed:?}, over its {CELL_BOUND:?} bound");
                            drop(cell);
                            return Ok(self.blocked(message));
                        }
                        cell.trace.passed = true;
                        cell.terminal = true;
                        self.step = Step::Done;
                        Ok(ScriptFlow::Complete)
                    }
                }
            }
        }
    }
}

fn save_evidence(
    client: &mut Client,
    directory: &Path,
    passed: bool,
    mut receipt: serde_json::Value,
    writer: &EvidenceWriter,
) -> Result<EvidenceJob, String> {
    std::fs::create_dir_all(directory)
        .map_err(|error| format!("create {}: {error}", directory.display()))?;
    let step = if passed { "01-final" } else { "FAIL-final" };
    let png = directory.join(format!("{step}.png"));
    let json = directory.join(format!("{step}.json"));
    receipt["png"] = serde_json::json!(png.file_name().and_then(|name| name.to_str()));
    receipt["png_error"] = serde_json::Value::Null;
    let sidecar = EvidenceSidecar {
        path: json,
        receipt,
        patch_error: Some(Box::new(|receipt, error| {
            receipt["png_error"] = serde_json::Value::String(error.to_owned());
        })),
    };
    match render_teller_frame(client) {
        Ok(frame) => Ok(writer.submit(EvidenceRequest {
            png_path: png,
            width: frame.width,
            height: frame.height,
            pixels: frame.pixels,
            color: PngColor::Rgba,
            sidecar: Some(sidecar),
        })),
        // Render needs the client, so its failure is reported inline; the
        // sidecar still lands with the error recorded, as before.
        Err(error) => Err(write_failure_sidecar(sidecar, &error)),
    }
}

fn read_facts(snapshot: &GameSnapshot, watch: &[i32], last: &Facts) -> Facts {
    let bank_open = snapshot.bank_component_id() >= 0;
    let generation = snapshot.bank_session_generation();
    Facts {
        frame: last.frame + 1,
        held: watch
            .iter()
            .map(|id| (*id, count(snapshot.inventory(), *id)))
            .collect(),
        banked: snapshot.bank_loaded().then(|| {
            watch
                .iter()
                .map(|id| (*id, count(snapshot.bank(), *id)))
                .collect()
        }),
        used: snapshot
            .inventory()
            .iter()
            .filter(|item| item.count > 0)
            .count() as i32,
        inventory_size: snapshot.inventory_size(),
        bank_open,
        side_root: snapshot.modals().side,
        generation,
        open_generation: if bank_open {
            Some(generation)
        } else {
            last.open_generation
        },
    }
}

pub(super) fn frame(client: &mut Client, shared: &Mutex<Cell>, account: &str) {
    let mut snapshot = shared.lock().snapshot.take().unwrap_or_default();
    snapshot.rebuild(client);
    let now = Instant::now();
    let capture = {
        let mut cell = shared.lock();
        if client.ingame && client.scene_state == 2 {
            cell.trace.live_held = Some(count(snapshot.inventory(), COINS));
            cell.trace.live_bank_open = Some(snapshot.bank_component_id() >= 0);
            cell.facts = read_facts(&snapshot, cell.watch, &cell.facts);
        }
        if !cell.terminal
            && cell
                .started
                .is_some_and(|started| now.duration_since(started) > cell.bound)
        {
            let message = format!("the cell exceeded {:?} in {:?}", cell.bound, cell.phase);
            cell.fail(message);
        }
        if cell.phase == Prep::Session {
            cell.session_runner.tick(client);
            match cell.session_runner.status() {
                scenario::RunnerStatus::Passed => {
                    cell.phase = Prep::TutorialReseed;
                    cell.last_action = now;
                }
                scenario::RunnerStatus::Failed(error) => {
                    cell.fail_prerequisite(format!(
                        "shared tutorial/relog fixture failed: {error}"
                    ));
                }
                scenario::RunnerStatus::Seeding | scenario::RunnerStatus::Running { .. } => {}
            }
        }
        let spaced = now.duration_since(cell.last_action) >= Duration::from_millis(400);
        match cell.phase {
            Prep::Session | Prep::PrerequisiteFailed => {}
            Prep::TutorialReseed => {
                // Fresh-account post-relog reseed: the kit-close queue has
                // already run in the new session, so this `setvar` sticks.
                // Baseline is captured before sending, so only a strictly
                // newer same-session `getvar` reply can satisfy the wait.
                let baseline = api::interact::chat_baseline(&snapshot);
                let reseed = api::interact::PostRelogTutorial::new(baseline);
                reseed.send_reseed(client);
                cell.tutorial_reseed = Some(reseed);
                cell.last_action = now;
                cell.phase = Prep::WaitTutorialReseed;
            }
            Prep::WaitTutorialReseed => {
                let reseed = cell
                    .tutorial_reseed
                    .as_ref()
                    .expect("tutorial reseed armed before waiting");
                match reseed.check(&snapshot) {
                    Ok(true) => {
                        println!("{}", api::interact::confirmation_log());
                        cell.phase = Prep::Teleport;
                        cell.last_action = now;
                    }
                    Ok(false) => {}
                    Err(error) => {
                        cell.fail_prerequisite(error);
                    }
                }
            }
            Prep::Teleport if spaced => {
                let _ = interact::cheat(client, &tele_args(DRAYNOR));
                cell.last_action = now;
                cell.phase = Prep::WaitArrive;
            }
            Prep::WaitArrive => {
                if snapshot.tile() == Some((DRAYNOR.x, DRAYNOR.z, DRAYNOR.level)) {
                    cell.phase = Prep::Clear;
                    cell.last_action = now;
                }
            }
            Prep::Clear if spaced => {
                let _ = interact::cheat(client, "~clearinv");
                cell.last_action = now;
                cell.phase = Prep::WaitClear;
            }
            Prep::WaitClear => {
                if snapshot.inventory().is_empty() && snapshot.bank_component_id() < 0 {
                    cell.phase = Prep::Seed;
                    cell.last_action = now;
                }
            }
            Prep::Seed if spaced => {
                if let Some(cheat) = cell.seed.get(cell.seeded) {
                    let _ = interact::cheat(client, cheat);
                    cell.seeded += 1;
                    cell.last_action = now;
                } else if client.ingame && client.scene_state == 2 && (cell.seed_ready)(&snapshot) {
                    cell.phase = Prep::Ready;
                    cell.started = Some(now);
                    cell.trace.preparation_ms =
                        Some(now.duration_since(cell.preparation_started).as_millis());
                    cell.last_action = now;
                }
            }
            _ => {}
        }
        let ready = cell.terminal && (!cell.trace.passed || (cell.capture_ready)(&cell));
        if ready && !cell.capture_started {
            cell.capture_started = true;
            let mut receipt = cell.scenario.clone();
            receipt["account"] = serde_json::json!(account);
            receipt["bank_floor"] = serde_json::json!(tile(DRAYNOR));
            receipt["trace"] = serde_json::json!(cell.trace);
            if cell.result.is_some() {
                receipt["script_answer"] = serde_json::json!(cell.result);
                receipt["posted"] = serde_json::json!(cell.facts);
            }
            Some((cell.evidence_dir.clone(), cell.trace.passed, receipt))
        } else {
            None
        }
    };
    if let Some((directory, passed, receipt)) = capture {
        let writer = shared.lock().writer.clone();
        match save_evidence(client, &directory, passed, receipt, &writer) {
            Ok(job) => {
                shared.lock().capture_job = Some(job);
            }
            Err(error) => {
                let mut cell = shared.lock();
                cell.capture_written = true;
                cell.capture_error = Some(error);
            }
        }
    }
    // The driver loop already polls `capture_written` under the cell
    // deadline; this marks it only once the background writer has the PNG
    // and sidecar durable, which is the bounded cell-end flush.
    let completed = {
        let cell = shared.lock();
        if cell.capture_written {
            None
        } else {
            cell.capture_job
                .clone()
                .and_then(|job| cell.writer.poll(&job))
        }
    };
    if let Some(outcome) = completed {
        let mut cell = shared.lock();
        cell.capture_written = true;
        cell.capture_error = outcome.error;
    }
    shared.lock().snapshot = Some(snapshot);
}

pub(super) fn check_prerequisites(
    play: &super::Play,
    account: &str,
    preparation_started: Instant,
    login_ready: &mut bool,
    cell: &Arc<Mutex<Cell>>,
) {
    let status = play
        .statuses()
        .into_iter()
        .find(|status| status.username == account);
    *login_ready |= status
        .as_ref()
        .is_some_and(|status| status.ingame && status.scene_state == 2);
    if !*login_ready && preparation_started.elapsed() >= LOGIN_DEADLINE {
        panic!(
            "HARNESS PREREQUISITE FAILURE (before the cell deadline): initial login did not reach \
             ingame && scene_state == 2 within {LOGIN_DEADLINE:?}; status={status:?}"
        );
    }

    let (started, failure, phase) = {
        let cell = cell.lock();
        (
            cell.started.is_some(),
            cell.preparation_failure.clone(),
            cell.phase,
        )
    };
    if let Some(failure) = failure {
        panic!(
            "HARNESS PREREQUISITE FAILURE (before the cell deadline): {failure}; phase={phase:?}"
        );
    }
    if !started && preparation_started.elapsed() >= PREPARATION_DEADLINE {
        panic!(
            "HARNESS PREREQUISITE FAILURE (before the cell deadline): bank fixture did not reach \
             seeded ingame scene-2 readiness within {PREPARATION_DEADLINE:?}; phase={phase:?}"
        );
    }
}

pub(super) fn live_profile(scratch: &Path) -> Result<ProfileOptions, String> {
    let path = |key: &str| {
        std::env::var_os(key)
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .ok_or_else(|| format!("{key} must be an absolute path"))
    };
    let port = |key: &str, default: u16| {
        std::env::var(key)
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(default)
    };
    Ok(ProfileOptions {
        profile: Some("local-289".into()),
        revision: Some("289".into()),
        host: Some("127.0.0.1".into()),
        asset_host: Some("127.0.0.1".into()),
        port: Some(port("GATHERER_GAME_PORT", 45594)),
        http_port: Some(port("GATHERER_HTTP_PORT", 2080)),
        nav_pack: Some(path("GATHERER_NAV_PACK")?),
        nav_flags: std::env::var_os("GATHERER_NAV_FLAGS").map(PathBuf::from),
        engine_dir: Some(path("GATHERER_ENGINE_DIR")?),
        vault_path: Some(scratch.join("vault")),
        cache_dir: Some(path("BOT_CACHE_DIR")?),
        unpack_dir: Some(path("CLIENT_UNPACK_DIR")?),
        catalog_root: Some(path("GATHERER_CATALOG_ROOT")?),
        ..ProfileOptions::default()
    })
}

#[test]
#[ignore = "requires LIVE=1, GATHERER_NAV_PACK/GATHERER_ENGINE_DIR/GATHERER_CATALOG_ROOT/BOT_CACHE_DIR/CLIENT_UNPACK_DIR/LIVE_EVIDENCE_DIR and a local 289 engine"]
fn live_bank_core_exact_withdraw_at_draynor() {
    assert_eq!(std::env::var("LIVE").as_deref(), Ok("1"), "requires LIVE=1");
    let root = PathBuf::from(
        std::env::var_os("LIVE_EVIDENCE_DIR").expect("LIVE_EVIDENCE_DIR names the evidence root"),
    );
    let account = super::mint_live_names(1).pop().expect("one live account");
    let epoch = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs());
    let evidence_dir = root.join(format!("l-withdraw_{account}_utc-{epoch}Z"));
    let scratch = evidence_dir.join("scratch");
    std::fs::create_dir_all(&scratch).expect("create evidence scratch");
    let options = live_profile(&scratch).expect("live profile options");
    let profile = options
        .resolve(None)
        .and_then(|resolved| resolved.bind())
        .expect("bind the local 289 profile");
    assert_eq!(
        profile.client().game_host(),
        "127.0.0.1",
        "loopback engine only"
    );
    let selected = profile.game_data().expect("selected 289 game data");
    let template = SharedClientTemplate::load(Arc::clone(&profile)).expect("client template");
    let world = template.world().expect("selected navigation world");
    if world.named_bank_facts().is_none() {
        world
            .bind_named_bank_facts(&selected)
            .expect("bind named bank facts");
    }
    let facts = Arc::clone(world.named_bank_facts().expect("named bank facts"));
    let bank_name: Arc<str> = Arc::from(
        facts
            .banks()
            .iter()
            .find(|bank| bank.name.contains("Draynor"))
            .expect("selected facts name a Draynor bank")
            .name,
    );
    let cell = Cell::new(
        CELL_BOUND,
        &[WITHDRAW_SEED],
        |_| true,
        &[COINS],
        serde_json::json!({
            "scenario": "bank_core_l_withdraw_exact_coins",
            "seed": { "bank_coins": SEEDED, "pack": "cleared" },
            "request": { "action": "WithdrawTo", "id": COINS, "target": TARGET },
            "login_deadline_ms": LOGIN_DEADLINE.as_millis(),
            "preparation_deadline_ms": PREPARATION_DEADLINE.as_millis(),
            "cell_clock_starts_after": "ingame && scene_state == 2 and seed posted",
        }),
        |cell| cell.trace.live_bank_open == Some(false) && cell.trace.live_held == Some(TARGET),
        evidence_dir.clone(),
    );
    let preparation_started = cell.preparation_started;
    let cell = Arc::new(Mutex::new(cell));
    let frame_cell = Arc::clone(&cell);
    let frame_account = account.clone();
    let play = run_with_template(
        template,
        true,
        vec![Profile {
            username: account.clone(),
            password: account.clone().into(),
            uid: 274_279_101,
            settings: ProfileSettings::default(),
        }],
        |_| (None, None),
        move |client, _, _| frame(client, &frame_cell, &frame_account),
    )
    .unwrap_or_else(|error| {
        panic!("HARNESS PREREQUISITE FAILURE (before the cell deadline): starting login: {error}")
    });
    let mut login_ready = false;
    let start = play.script_start_handle();
    loop {
        check_prerequisites(
            &play,
            &account,
            preparation_started,
            &mut login_ready,
            &cell,
        );
        let deadline = {
            let guard = cell.lock();
            guard
                .started
                .map_or(preparation_started + PREPARATION_DEADLINE, |started| {
                    started + CELL_BOUND + CAPTURE_GRACE
                })
        };
        assert!(
            Instant::now() < deadline,
            "FAIL L-withdraw: no evidence by its fixed cell deadline; trace={:#?}",
            cell.lock().trace
        );
        let begin = {
            let mut guard = cell.lock();
            if guard.phase == Prep::Ready
                && guard.script_started.is_none()
                && Instant::now().duration_since(guard.last_action) >= Duration::from_secs(1)
            {
                guard.script_started = Some(Instant::now());
                true
            } else {
                false
            }
        };
        if begin {
            let script = WithdrawCell {
                facts: Arc::clone(&facts),
                bank_name: Arc::clone(&bank_name),
                cell: Arc::clone(&cell),
                step: Step::Select,
                select: None,
                open: None,
                withdraw: None,
                close: None,
                access: None,
            };
            match start.start_test_script(&account, Box::new(script), Some(Arc::clone(&selected))) {
                Ok(run) if play.script_native_run(&account) == Some(run) => play.wake(&account),
                Ok(_) => cell
                    .lock()
                    .fail("real Play did not publish the native run".into()),
                Err(error) => cell
                    .lock()
                    .fail(format!("Play refused the script: {error}")),
            }
        }
        let written = {
            let guard = cell.lock();
            guard
                .capture_written
                .then(|| (guard.trace.clone(), guard.capture_error.clone()))
        };
        if let Some((trace, error)) = written {
            assert!(
                error.is_none(),
                "evidence capture failed: {error:?}; {trace:#?}"
            );
            assert!(
                trace.passed,
                "FAIL L-withdraw: {:?}; evidence={}",
                trace.failure,
                evidence_dir.display()
            );
            println!(
                "PASS L-withdraw {trace:?} evidence={}",
                evidence_dir.display()
            );
            let _ = std::fs::remove_dir_all(&scratch);
            return;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

const WITHDRAW_SEED: &str = "givebank coins 50";

/// One compat cell: its bound, seed, frozen script and posted verdict.
struct CompatSpec {
    label: &'static str,
    uid: i32,
    bound: Duration,
    seed: &'static [&'static str],
    seed_ready: fn(&GameSnapshot) -> bool,
    watch: &'static [i32],
    /// The cell's frozen calls after the booth is open and ready; they
    /// assign the answer object to `cell`.
    body: &'static str,
    /// The verdict on the script's answer and the posted facts after it.
    verdict: fn(&serde_json::Value, &Facts) -> Result<(), String>,
}

/// The frozen-API Load script around a cell body: open the booth the
/// player stands at, wait for its stock, run the body once, and publish
/// `globalThis.__cell`.
fn compat_script(body: &str) -> String {
    format!(
        r#"
import {{ Bank }} from '../../api/bank/Bank.js';
import {{ Inventory }} from '../../api/inventory/Inventory.js';
export default class BankCoreCell extends LoopingBot {{
    async loop() {{
        if (globalThis.__did) return;
        globalThis.__did = true;
        const opened = await Bank.openNearest();
        const ready = opened && await Bank.waitReady(5000);
        if (!ready) {{
            globalThis.__cell = {{ opened, ready }};
            return;
        }}
        let cell = {{}};
        {body}
        globalThis.__cell = {{ opened, ready, ...cell }};
    }}
}}
"#
    )
}

fn has(snapshot: &GameSnapshot, id: i32, held: i32) -> bool {
    count(snapshot.inventory(), id) == held
}

fn run_compat_cell(spec: &CompatSpec) {
    assert_eq!(std::env::var("LIVE").as_deref(), Ok("1"), "requires LIVE=1");
    let root = PathBuf::from(
        std::env::var_os("LIVE_EVIDENCE_DIR").expect("LIVE_EVIDENCE_DIR names the evidence root"),
    );
    let account = super::mint_live_names(1).pop().expect("one live account");
    let epoch = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs());
    let evidence_dir = root.join(format!("{}_{account}_utc-{epoch}Z", spec.label));
    let scratch = evidence_dir.join("scratch");
    std::fs::create_dir_all(&scratch).expect("create evidence scratch");
    let options = live_profile(&scratch).expect("live profile options");
    let profile = options
        .resolve(None)
        .and_then(|resolved| resolved.bind())
        .expect("bind the local 289 profile");
    assert_eq!(
        profile.client().game_host(),
        "127.0.0.1",
        "loopback engine only"
    );
    let template = SharedClientTemplate::load(Arc::clone(&profile)).expect("client template");
    let script = compat_script(spec.body);
    let cell = Cell::new(
        spec.bound,
        spec.seed,
        spec.seed_ready,
        spec.watch,
        serde_json::json!({
            "scenario": format!("bank_core_{}", spec.label.replace('-', "_")),
            "seed": spec.seed,
            "bound_ms": spec.bound.as_millis(),
            "login_deadline_ms": LOGIN_DEADLINE.as_millis(),
            "preparation_deadline_ms": PREPARATION_DEADLINE.as_millis(),
            "cell_clock_starts_after": "ingame && scene_state == 2 and seed posted",
            "script": script,
        }),
        |_| true,
        evidence_dir.clone(),
    );
    let preparation_started = cell.preparation_started;
    let cell = Arc::new(Mutex::new(cell));
    let frame_cell = Arc::clone(&cell);
    let frame_account = account.clone();
    let play = run_with_template(
        template,
        true,
        vec![Profile {
            username: account.clone(),
            password: account.clone().into(),
            uid: spec.uid,
            settings: ProfileSettings::default(),
        }],
        |_| (None, None),
        move |client, _, _| frame(client, &frame_cell, &frame_account),
    )
    .unwrap_or_else(|error| {
        panic!("HARNESS PREREQUISITE FAILURE (before the cell deadline): starting login: {error}")
    });
    let mut login_ready = false;
    // The posted frame the answer arrived at; the verdict reads a later one.
    let mut answered_at: Option<u64> = None;
    loop {
        check_prerequisites(
            &play,
            &account,
            preparation_started,
            &mut login_ready,
            &cell,
        );
        let deadline = {
            let guard = cell.lock();
            guard
                .started
                .map_or(preparation_started + PREPARATION_DEADLINE, |started| {
                    started + spec.bound + CAPTURE_GRACE
                })
        };
        assert!(
            Instant::now() < deadline,
            "FAIL {}: no evidence by its fixed cell deadline; trace={:#?}",
            spec.label,
            cell.lock().trace
        );
        let begin = {
            let mut guard = cell.lock();
            if guard.phase == Prep::Ready
                && guard.script_started.is_none()
                && Instant::now().duration_since(guard.last_action) >= Duration::from_secs(1)
            {
                guard.script_started = Some(Instant::now());
                true
            } else {
                false
            }
        };
        if begin {
            if let Err(error) = play.script_start_load(
                &account,
                script.clone(),
                script::LoadShape::CompatClass,
                None,
                vec![],
            ) {
                cell.lock()
                    .fail(format!("Play refused the Load script: {error}"));
            }
        }
        let started = cell.lock().script_started.is_some();
        if started && !cell.lock().terminal {
            let answer = super::script_slot(&play.scripts, &account)
                .and_then(|slot| slot.lock().ok()?.probe("globalThis.__cell || null").ok());
            let mut guard = cell.lock();
            match (answer, answered_at) {
                (Some(answer), None) if !answer.is_null() => {
                    guard.trace.script_ms = guard.script_started.map(|at| at.elapsed().as_millis());
                    guard.result = Some(answer);
                    answered_at = Some(guard.facts.frame);
                }
                (_, Some(at)) if guard.facts.frame > at + 1 => {
                    let elapsed = guard
                        .started
                        .expect("fixture readiness starts the bank-cell clock")
                        .elapsed();
                    guard.trace.cell_ms = Some(elapsed.as_millis());
                    let result = guard.result.clone().unwrap_or_default();
                    match (spec.verdict)(&result, &guard.facts) {
                        Ok(()) if elapsed <= spec.bound => {
                            guard.trace.passed = true;
                            guard.terminal = true;
                        }
                        Ok(()) => {
                            let message = format!(
                                "the cell took {elapsed:?}, over its {:?} bound",
                                spec.bound
                            );
                            guard.fail(message);
                        }
                        Err(message) => guard.fail(message),
                    }
                }
                _ => {}
            }
        }
        let written = {
            let guard = cell.lock();
            guard.capture_written.then(|| {
                (
                    guard.trace.clone(),
                    guard.result.clone(),
                    guard.facts.clone(),
                    guard.capture_error.clone(),
                )
            })
        };
        if let Some((trace, result, facts, error)) = written {
            assert!(
                error.is_none(),
                "evidence capture failed: {error:?}; {trace:#?}"
            );
            assert!(
                trace.passed,
                "FAIL {}: {:?}; answer={result:?}; posted={facts:?}; evidence={}",
                spec.label,
                trace.failure,
                evidence_dir.display()
            );
            println!(
                "PASS {} preparation_ms={:?} cell_ms={:?} script_ms={:?} answer={result:?} posted={facts:?} evidence={}",
                spec.label,
                trace.preparation_ms,
                trace.cell_ms,
                trace.script_ms,
                evidence_dir.display()
            );
            let _ = std::fs::remove_dir_all(&scratch);
            return;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn answered_true(answer: &serde_json::Value, key: &str) -> Result<(), String> {
    if answer[key] == serde_json::json!(true) {
        Ok(())
    } else {
        Err(format!("{key} answered {} ({answer})", answer[key]))
    }
}

/// Eight bones take eight slots: 20 of 28 stay free.
const PACK_OF_EIGHT: &str = "give bones 8";

fn eight_bones(snapshot: &GameSnapshot) -> bool {
    has(snapshot, BONES, 8)
}

/// The load cells' body for one bank row name.
macro_rules! load_body {
    ($name:literal) => {
        concat!(
            "
        const name = '",
            $name,
            "';
        cell.before = { held: Inventory.count(name), banked: Bank.count(name), used: Inventory.used() };
        const t = Date.now();
        cell.ok = await Bank.withdrawLoad(name);
        cell.ms = Date.now() - t;"
        )
    };
}

#[test]
#[ignore = "requires LIVE=1, GATHERER_NAV_PACK/GATHERER_ENGINE_DIR/GATHERER_CATALOG_ROOT/BOT_CACHE_DIR/CLIENT_UNPACK_DIR/LIVE_EVIDENCE_DIR and a local 289 engine"]
fn live_bank_core_compat_withdraw_load_stackable_at_draynor() {
    run_compat_cell(&CompatSpec {
        label: "l-load",
        uid: 274_279_102,
        bound: Duration::from_secs(90),
        seed: &["givebank feather 300", PACK_OF_EIGHT],
        seed_ready: eight_bones,
        watch: &[FEATHER, BONES],
        body: load_body!("Feather"),
        verdict: |answer, posted| {
            answered_true(answer, "ok")?;
            let (held, banked) = (posted.held(FEATHER), posted.banked(FEATHER));
            // That row filled the pack or emptied: one stack of 300.
            if held == 300 && banked == Some(0) && posted.held(BONES) == 8 {
                Ok(())
            } else {
                Err(format!(
                    "feathers held={held} banked={banked:?} bones={}",
                    posted.held(BONES)
                ))
            }
        },
    });
}

#[test]
#[ignore = "requires LIVE=1, GATHERER_NAV_PACK/GATHERER_ENGINE_DIR/GATHERER_CATALOG_ROOT/BOT_CACHE_DIR/CLIENT_UNPACK_DIR/LIVE_EVIDENCE_DIR and a local 289 engine"]
fn live_bank_core_compat_withdraw_load_unstackable_at_draynor() {
    run_compat_cell(&CompatSpec {
        label: "l-load-unstack",
        uid: 274_279_103,
        bound: Duration::from_secs(120),
        seed: &["givebank logs 40", PACK_OF_EIGHT],
        seed_ready: eight_bones,
        watch: &[LOGS, BONES],
        body: load_body!("Logs"),
        verdict: |answer, posted| {
            answered_true(answer, "ok")?;
            let (held, banked) = (posted.held(LOGS), posted.banked(LOGS));
            // The pack filled; the bank row kept the rest.
            if held == 20 && banked == Some(20) && posted.used == 28 {
                Ok(())
            } else {
                Err(format!(
                    "logs held={held} banked={banked:?} used={}",
                    posted.used
                ))
            }
        },
    });
}

#[test]
#[ignore = "requires LIVE=1, GATHERER_NAV_PACK/GATHERER_ENGINE_DIR/GATHERER_CATALOG_ROOT/BOT_CACHE_DIR/CLIENT_UNPACK_DIR/LIVE_EVIDENCE_DIR and a local 289 engine"]
fn live_bank_core_compat_close_at_draynor() {
    run_compat_cell(&CompatSpec {
        label: "l-close",
        uid: 274_279_104,
        bound: Duration::from_secs(20),
        seed: &[],
        seed_ready: |_| true,
        watch: &[],
        body: "
        let t = Date.now();
        cell.ok = await Bank.close();
        cell.ms = Date.now() - t;
        cell.shut = !Bank.isOpen();
        t = Date.now();
        cell.again = await Bank.close();
        cell.againMs = Date.now() - t;",
        verdict: |answer, posted| {
            answered_true(answer, "ok")?;
            answered_true(answer, "shut")?;
            answered_true(answer, "again")?;
            let ms = answer["ms"].as_u64().unwrap_or(u64::MAX);
            if ms > 4_000 {
                return Err(format!("close took {ms} ms, over 4 s"));
            }
            let acknowledged = posted
                .open_generation
                .is_some_and(|open| posted.generation > open);
            if !posted.bank_open && posted.side_root == -1 && acknowledged {
                Ok(())
            } else {
                Err(format!(
                    "not acknowledged: open={} side={} generation={} open_generation={:?}",
                    posted.bank_open, posted.side_root, posted.generation, posted.open_generation
                ))
            }
        },
    });
}

#[test]
#[ignore = "requires LIVE=1, GATHERER_NAV_PACK/GATHERER_ENGINE_DIR/GATHERER_CATALOG_ROOT/BOT_CACHE_DIR/CLIENT_UNPACK_DIR/LIVE_EVIDENCE_DIR and a local 289 engine"]
fn live_bank_core_compat_deposit_all_except_at_draynor() {
    run_compat_cell(&CompatSpec {
        label: "l-except",
        uid: 274_279_105,
        bound: Duration::from_secs(60),
        seed: &["give coins 100", "give bones 3", "give tinderbox 1"],
        seed_ready: |snapshot| {
            has(snapshot, COINS, 100) && has(snapshot, BONES, 3) && has(snapshot, TINDERBOX, 1)
        },
        watch: &[COINS, BONES, TINDERBOX],
        body: "
        const t = Date.now();
        await Bank.depositAllExcept(['Coins']);
        cell.ms = Date.now() - t;",
        verdict: |_, posted| {
            let held = (
                posted.held(COINS),
                posted.held(BONES),
                posted.held(TINDERBOX),
            );
            let banked = (
                posted.banked(COINS),
                posted.banked(BONES),
                posted.banked(TINDERBOX),
            );
            if held == (100, 0, 0) && banked == (Some(0), Some(3), Some(1)) {
                Ok(())
            } else {
                Err(format!(
                    "held (coins, bones, tinderbox)={held:?} banked={banked:?}"
                ))
            }
        },
    });
}
