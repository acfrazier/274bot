//! L-BANK-MEM (design-bank-snapshot §7 S2): the host bank memory and its
//! persisted hint, at the real Draynor booth through real Play.
//!
//! A fresh account is logged in through real Play on the shared
//! tutorial/relog fixture of [`super::bank_core_live`]; its bank is seeded
//! with three ids (one stackable) and the pack cleared. The cell then:
//!
//! 1. runs a native script that records the memory before any bank open
//!    (`Unknown`), selects Draynor, opens the booth with the native `Open`
//!    machine, records the memory (`Session`, the seeded rows) and closes;
//! 2. logs the account out through its arm and asserts the hint file exists
//!    under the throwaway `HOME` with exactly the seeded rows and an
//!    `observed_at_unix` inside the cell window;
//! 3. logs back in (the in-process relog takes the `relogged()` path) and,
//!    before any bank open, asserts `Play::bank_rows` and the slot's
//!    `SnapshotView::bank_memory()` both answer `Hint` with the rows; the
//!    relog spawns on the mainland, so the account is `::tele`d back beside
//!    the booth first (the per-account cheat the first login also used);
//! 4. opens the bank again and asserts `Session` with a moved generation.
//!
//! Evidence (JSON receipt and CPU-rendered PNG) lands below
//! `LIVE_EVIDENCE_DIR`. Deadline: 5 minutes after the seed is posted.
//!
//! `LIVE=1 BOT_CPU=1 BOT_LIVE_NAME_PREFIX=bm GATHERER_NAV_PACK=<pack> GATHERER_ENGINE_DIR=<engine> GATHERER_CATALOG_ROOT=<catalog> GATHERER_GAME_PORT=<port> GATHERER_HTTP_PORT=<port> BOT_CACHE_DIR=<unpack-root>/<version> CLIENT_UNPACK_DIR=<unpack-root> LIVE_EVIDENCE_DIR=<evidence-root> isohome cargo test -p host-play --lib live_bank_memory_ -- --ignored --nocapture --test-threads=1`

use std::path::PathBuf;
use std::sync::Arc;
use std::task::Poll;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use api::bank_memory::Origin;
use api::named_banks::{BankPreferences, NamedBankFacts};
use parking_lot::Mutex;
use script::bank::{Close, Open, OpenArgs, Select, SelectArgs};
use script::native::{ActionHandle, NativeTick, Script, ScriptFailure, ScriptFlow};
use vault::{Profile, ProfileSettings};

use super::bank_core_live::{
    check_prerequisites, frame, live_profile, Cell, Prep, CAPTURE_GRACE, DRAYNOR, LOGIN_DEADLINE,
    PREPARATION_DEADLINE,
};
use super::{run_with_template, SharedClientTemplate};

const COINS: i32 = 995;
const BONES: i32 = 526;
const TINDERBOX: i32 = 590;
const PROFILE: &str = "local-289";
const SEED: &[&str] = &[
    "givebank coins 3400",
    "givebank bones 12",
    "givebank tinderbox 1",
];
/// The seeded bank as the memory stores it: sorted by id.
const SEEDED_ROWS: [(i32, i32); 3] = [(BONES, 12), (TINDERBOX, 1), (COINS, 3_400)];
const CELL_BOUND: Duration = Duration::from_secs(300);
/// How long the hint file may lag the published disconnect (the save runs
/// on the slot thread's boundary frame, after the status row flips).
const SAVE_GRACE: Duration = Duration::from_secs(15);
const RELOG_DEADLINE: Duration = Duration::from_secs(120);
/// How long after the relogged session is ready the second pass waits: the
/// first pass ran seconds after its login too (teleport, clear, seed).
const RELOG_SETTLE: Duration = Duration::from_secs(3);

/// What the native script and the test thread observed, in order.
#[derive(Debug, Clone, Default, serde::Serialize)]
struct MemoryTrace {
    home: Option<String>,
    cell_start_unix: u64,
    /// Pass 1, before any bank open.
    first_origin: Option<String>,
    first_open_origin: Option<String>,
    first_open_rows: Option<Vec<(i32, i32)>>,
    first_open_generation: Option<u64>,
    first_open_bank_rows: Option<usize>,
    /// The memory's reserved row capacity: the bank's slot count (§3).
    first_open_capacity: Option<usize>,
    first_closed: bool,
    first_done: bool,
    logout_unix: Option<u64>,
    hint_path: Option<String>,
    hint_contents: Option<String>,
    hint_rows: Option<Vec<(i32, i32)>>,
    hint_observed_at_unix: Option<u64>,
    hint_mode: Option<u32>,
    relog_unix: Option<u64>,
    /// After the relog, before any bank open: the UI copy and the script's borrow.
    relog_ui_origin: Option<String>,
    relog_ui_rows: Option<Vec<(i32, i32)>>,
    relog_script_origin: Option<String>,
    relog_script_rows: Option<Vec<(i32, i32)>>,
    relog_script_generation: Option<u64>,
    /// After the second open.
    reopen_origin: Option<String>,
    reopen_generation: Option<u64>,
    reopen_rows: Option<Vec<(i32, i32)>>,
    second_closed: bool,
    second_done: bool,
    failure: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pass {
    First,
    Second,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Step {
    Record,
    Select,
    Open,
    Close,
    Done,
}

struct MemoryCell {
    pass: Pass,
    facts: Arc<NamedBankFacts>,
    bank_name: Arc<str>,
    cell: Arc<Mutex<Cell>>,
    trace: Arc<Mutex<MemoryTrace>>,
    step: Step,
    select: Option<ActionHandle<Select>>,
    open: Option<ActionHandle<Open>>,
    close: Option<ActionHandle<Close>>,
    access: Option<Arc<script::bank::BankStandAccess>>,
}

impl MemoryCell {
    fn blocked(&self, message: impl Into<String>) -> ScriptFlow {
        let message = message.into();
        self.trace.lock().failure = Some(message.clone());
        self.cell.lock().fail(message.clone());
        ScriptFlow::Blocked(ScriptFailure {
            code: "bank-memory-live".into(),
            message: message.into(),
        })
    }
}

fn origin_name(origin: Origin) -> String {
    format!("{origin:?}")
}

impl Script for MemoryCell {
    fn tick(&mut self, tick: &mut NativeTick<'_>) -> Result<ScriptFlow, ScriptFailure> {
        match self.step {
            Step::Done => Ok(ScriptFlow::Complete),
            Step::Record => {
                let Some(memory) = tick.cx.snapshot().bank_memory() else {
                    return Ok(self.blocked("the frame carries no bank memory borrow"));
                };
                let origin = memory.origin();
                let rows = memory.rows().to_vec();
                let generation = memory.generation();
                {
                    let mut trace = self.trace.lock();
                    match self.pass {
                        Pass::First => trace.first_origin = Some(origin_name(origin)),
                        Pass::Second => {
                            trace.relog_script_origin = Some(origin_name(origin));
                            trace.relog_script_rows = Some(rows.clone());
                            trace.relog_script_generation = Some(generation);
                        }
                    }
                }
                match self.pass {
                    Pass::First if origin != Origin::Unknown => {
                        return Ok(self.blocked(format!(
                            "a fresh process must start Unknown, not {origin:?}"
                        )));
                    }
                    Pass::Second if origin != Origin::Hint || rows != SEEDED_ROWS => {
                        return Ok(self.blocked(format!(
                            "before any open after the relog the script must see Hint with the seeded rows: origin={origin:?} rows={rows:?}"
                        )));
                    }
                    _ => {}
                }
                self.step = Step::Select;
                Ok(ScriptFlow::Continue)
            }
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
                        let Some(memory) = snapshot.bank_memory() else {
                            return Ok(self.blocked("the frame carries no bank memory borrow"));
                        };
                        let origin = memory.origin();
                        let rows = memory.rows().to_vec();
                        let generation = memory.generation();
                        let previous = {
                            let mut trace = self.trace.lock();
                            match self.pass {
                                Pass::First => {
                                    trace.first_open_origin = Some(origin_name(origin));
                                    trace.first_open_rows = Some(rows.clone());
                                    trace.first_open_generation = Some(generation);
                                    trace.first_open_bank_rows =
                                        snapshot.bank().map(|rows| rows.value.len());
                                    trace.first_open_capacity = Some(memory.capacity());
                                    None
                                }
                                Pass::Second => {
                                    trace.reopen_origin = Some(origin_name(origin));
                                    trace.reopen_generation = Some(generation);
                                    trace.reopen_rows = Some(rows.clone());
                                    trace.relog_script_generation
                                }
                            }
                        };
                        if origin != Origin::Session {
                            return Ok(self.blocked(format!(
                                "the open bank must observe as Session, not {origin:?}"
                            )));
                        }
                        if rows != SEEDED_ROWS {
                            return Ok(self.blocked(format!(
                                "the open bank must hold exactly the seeded rows: {rows:?}"
                            )));
                        }
                        if let Some(previous) = previous {
                            if generation <= previous {
                                return Ok(self.blocked(format!(
                                    "the reopen must move the generation: {previous} -> {generation}"
                                )));
                            }
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
                        let closed = snapshot.bank_session().map(|session| session.value.open)
                            == Some(false);
                        {
                            let mut trace = self.trace.lock();
                            match self.pass {
                                Pass::First => {
                                    trace.first_closed = closed;
                                    trace.first_done = true;
                                }
                                Pass::Second => {
                                    trace.second_closed = closed;
                                    trace.second_done = true;
                                }
                            }
                        }
                        if !closed {
                            return Ok(self.blocked("Close returned with the bank still open"));
                        }
                        self.step = Step::Done;
                        Ok(ScriptFlow::Complete)
                    }
                }
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stage {
    Prepare,
    FirstPass,
    LoggingOut {
        since: Instant,
    },
    AwaitHint {
        since: Instant,
    },
    /// Logged back in: waiting for the session to be ready (no welcome hold,
    /// no modal), then `::tele` back beside the booth — the relog spawns on
    /// the mainland, as the first login did before its own teleport.
    Relogging {
        since: Instant,
    },
    /// `settled`: when the slot's tile first read as the booth floor; the
    /// second pass starts `RELOG_SETTLE` after that, as the first pass ran
    /// well after its own teleport.
    Returning {
        since: Instant,
        settled: Option<Instant>,
    },
    SecondPass,
    Done,
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

/// `(connected, ready)`: ready is in game at scene 2 with the host's welcome
/// hold released and no modal open — the state the first pass ran in.
fn status_ingame(play: &super::Play, account: &str) -> (bool, bool) {
    play.statuses()
        .into_iter()
        .find(|status| status.username == account)
        .map_or((false, false), |status| {
            (
                status.connected,
                status.ingame
                    && status.scene_state == 2
                    && !status.welcome_hold
                    && status.main_modal_id == -1,
            )
        })
}

#[test]
#[ignore = "requires LIVE=1, GATHERER_NAV_PACK/GATHERER_ENGINE_DIR/GATHERER_CATALOG_ROOT/BOT_CACHE_DIR/CLIENT_UNPACK_DIR/LIVE_EVIDENCE_DIR and a local 289 engine"]
fn live_bank_memory_hint_and_relog_at_draynor() {
    assert_eq!(std::env::var("LIVE").as_deref(), Ok("1"), "requires LIVE=1");
    let root = PathBuf::from(
        std::env::var_os("LIVE_EVIDENCE_DIR").expect("LIVE_EVIDENCE_DIR names the evidence root"),
    );
    let home = std::env::var("HOME").expect("HOME is the throwaway isohome directory");
    let account = super::mint_live_names(1).pop().expect("one live account");
    let epoch = unix_now();
    let evidence_dir = root.join(format!("l-bank-mem_{account}_utc-{epoch}Z"));
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
    assert_eq!(
        profile.name(),
        PROFILE,
        "the hint path is keyed by this profile name"
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
    // The path the slot thread was handed at spawn, resolved the same way
    // here (the test thread shares the process `HOME`).
    let hint_path = script::bot_file("bank-hints")
        .join(PROFILE)
        .join(format!("{account}.json"));
    assert!(
        hint_path.starts_with(&home),
        "the hint path {} must sit under HOME {home}",
        hint_path.display()
    );
    let trace = Arc::new(Mutex::new(MemoryTrace {
        home: Some(home.clone()),
        hint_path: Some(hint_path.display().to_string()),
        ..MemoryTrace::default()
    }));
    let cell = Cell::new(
        CELL_BOUND,
        SEED,
        |_| true,
        &[COINS, BONES, TINDERBOX],
        serde_json::json!({
            "scenario": "bank_memory_l_bank_mem",
            "seed": { "bank": SEEDED_ROWS, "pack": "cleared" },
            "request": { "steps": ["open", "close", "logout", "hint file", "relog", "bank_rows", "open", "close"] },
            "login_deadline_ms": LOGIN_DEADLINE.as_millis(),
            "preparation_deadline_ms": PREPARATION_DEADLINE.as_millis(),
            "cell_bound_ms": CELL_BOUND.as_millis(),
            "cell_clock_starts_after": "ingame && scene_state == 2 and seed posted",
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
            uid: 274_279_102,
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
    let arm = play.arm(&account).expect("the spawned slot's arm");
    let mut stage = Stage::Prepare;
    let fail = |cell: &Arc<Mutex<Cell>>, trace: &Arc<Mutex<MemoryTrace>>, message: String| {
        trace.lock().failure = Some(message.clone());
        cell.lock().fail(message);
    };
    let start_pass = |pass: Pass| {
        let script = MemoryCell {
            pass,
            facts: Arc::clone(&facts),
            bank_name: Arc::clone(&bank_name),
            cell: Arc::clone(&cell),
            trace: Arc::clone(&trace),
            step: Step::Record,
            select: None,
            open: None,
            close: None,
            access: None,
        };
        match start.start_test_script(&account, Box::new(script), Some(Arc::clone(&selected))) {
            Ok(run) if play.script_native_run(&account) == Some(run) => {
                play.wake(&account);
                Ok(())
            }
            Ok(_) => Err("real Play did not publish the native run".to_string()),
            Err(error) => Err(format!("Play refused the script: {error}")),
        }
    };
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
            "FAIL L-BANK-MEM: no evidence by its fixed cell deadline; stage={stage:?} trace={:#?}",
            trace.lock()
        );
        let failed = cell.lock().terminal && !cell.lock().trace.passed;
        if !failed {
            match stage {
                Stage::Prepare => {
                    let begin = {
                        let mut guard = cell.lock();
                        if guard.phase == Prep::Ready
                            && guard.script_started.is_none()
                            && Instant::now().duration_since(guard.last_action)
                                >= Duration::from_secs(1)
                        {
                            guard.script_started = Some(Instant::now());
                            true
                        } else {
                            false
                        }
                    };
                    if begin {
                        trace.lock().cell_start_unix = unix_now();
                        match start_pass(Pass::First) {
                            Ok(()) => stage = Stage::FirstPass,
                            Err(message) => fail(&cell, &trace, message),
                        }
                    }
                }
                Stage::FirstPass => {
                    if trace.lock().first_done {
                        trace.lock().logout_unix = Some(unix_now());
                        arm.request_logout();
                        play.wake(&account);
                        stage = Stage::LoggingOut {
                            since: Instant::now(),
                        };
                    }
                }
                Stage::LoggingOut { since } => {
                    if !play.slot_connected(&account) {
                        stage = Stage::AwaitHint {
                            since: Instant::now(),
                        };
                    } else if since.elapsed() > RELOG_DEADLINE {
                        fail(&cell, &trace, "the slot did not log out".into());
                    }
                }
                Stage::AwaitHint { since } => {
                    if hint_path.is_file() {
                        match read_hint(&hint_path, &account) {
                            Ok(HintRead {
                                contents,
                                rows,
                                observed_at,
                                mode,
                            }) => {
                                let (start, logout) = {
                                    let guard = trace.lock();
                                    (guard.cell_start_unix, guard.logout_unix.unwrap_or(0))
                                };
                                {
                                    let mut guard = trace.lock();
                                    guard.hint_contents = Some(contents);
                                    guard.hint_rows = Some(rows.clone());
                                    guard.hint_observed_at_unix = Some(observed_at);
                                    guard.hint_mode = mode;
                                }
                                let now = unix_now();
                                if rows != SEEDED_ROWS {
                                    fail(
                                        &cell,
                                        &trace,
                                        format!("the hint file must hold exactly the seeded rows: {rows:?}"),
                                    );
                                } else if observed_at < start || observed_at > now {
                                    fail(
                                        &cell,
                                        &trace,
                                        format!("observed_at_unix {observed_at} is outside the cell window {start}..={now} (logout at {logout})"),
                                    );
                                } else if mode.is_some_and(|mode| mode != 0o600) {
                                    fail(
                                        &cell,
                                        &trace,
                                        format!(
                                            "the hint file mode is {:o}, not 600",
                                            mode.unwrap_or(0)
                                        ),
                                    );
                                } else {
                                    trace.lock().relog_unix = Some(unix_now());
                                    arm.arm_explicit_login();
                                    play.wake(&account);
                                    stage = Stage::Relogging {
                                        since: Instant::now(),
                                    };
                                }
                            }
                            Err(message) => fail(&cell, &trace, message),
                        }
                    } else if since.elapsed() > SAVE_GRACE {
                        fail(
                            &cell,
                            &trace,
                            format!(
                                "no hint file at {} within {SAVE_GRACE:?} of the logout",
                                hint_path.display()
                            ),
                        );
                    }
                }
                Stage::Relogging { since } => {
                    let (connected, ready) = status_ingame(&play, &account);
                    if connected && ready {
                        match play.cheat(&account, &super::tele_args(DRAYNOR)) {
                            Ok(()) => {
                                stage = Stage::Returning {
                                    since: Instant::now(),
                                    settled: None,
                                }
                            }
                            Err(refusal) => fail(
                                &cell,
                                &trace,
                                format!("the teleport back to the booth was refused: {refusal}"),
                            ),
                        }
                    } else if since.elapsed() > RELOG_DEADLINE {
                        fail(
                            &cell,
                            &trace,
                            format!("the slot did not relog within {RELOG_DEADLINE:?}"),
                        );
                    }
                }
                Stage::Returning { since, settled } => {
                    let at_booth = play
                        .statuses()
                        .into_iter()
                        .find(|status| status.username == account)
                        .is_some_and(|status| {
                            (status.tile_x, status.tile_z, status.tile_level)
                                == (DRAYNOR.x, DRAYNOR.z, DRAYNOR.level)
                        });
                    if at_booth && settled.is_none() {
                        stage = Stage::Returning {
                            since,
                            settled: Some(Instant::now()),
                        };
                    } else if at_booth
                        && settled.is_some_and(|settled| settled.elapsed() >= RELOG_SETTLE)
                    {
                        let rows = play.bank_rows(&account);
                        {
                            let mut guard = trace.lock();
                            guard.relog_ui_origin = Some(origin_name(rows.origin));
                            guard.relog_ui_rows = Some(rows.rows.clone());
                        }
                        if rows.origin != Origin::Hint || rows.rows != SEEDED_ROWS {
                            fail(
                                &cell,
                                &trace,
                                format!(
                                    "before any open after the relog Play::bank_rows must be Hint with the seeded rows: {rows:?}"
                                ),
                            );
                        } else {
                            match start_pass(Pass::Second) {
                                Ok(()) => stage = Stage::SecondPass,
                                Err(message) => fail(&cell, &trace, message),
                            }
                        }
                    } else if since.elapsed() > RELOG_DEADLINE {
                        fail(
                            &cell,
                            &trace,
                            format!("the slot did not relog within {RELOG_DEADLINE:?}"),
                        );
                    }
                }
                Stage::SecondPass => {
                    if trace.lock().second_done {
                        let mut guard = cell.lock();
                        guard.result = Some(serde_json::json!(*trace.lock()));
                        guard.trace.passed = true;
                        guard.terminal = true;
                        stage = Stage::Done;
                    }
                }
                Stage::Done => {}
            }
        } else if cell.lock().result.is_none() {
            cell.lock().result = Some(serde_json::json!(*trace.lock()));
        }
        let written = {
            let guard = cell.lock();
            guard
                .capture_written
                .then(|| (guard.trace.clone(), guard.capture_error.clone()))
        };
        if let Some((cell_trace, error)) = written {
            let memory_trace = trace.lock().clone();
            assert!(
                error.is_none(),
                "evidence capture failed: {error:?}; {memory_trace:#?}"
            );
            assert!(
                cell_trace.passed,
                "FAIL L-BANK-MEM: {:?}; stage={stage:?}; trace={memory_trace:#?}; evidence={}",
                cell_trace.failure,
                evidence_dir.display()
            );
            println!(
                "PASS L-BANK-MEM hint_path={} hint={} trace={memory_trace:?} evidence={}",
                hint_path.display(),
                memory_trace.hint_contents.as_deref().unwrap_or(""),
                evidence_dir.display()
            );
            let _ = std::fs::remove_dir_all(&scratch);
            return;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// The hint file as written.
struct HintRead {
    contents: String,
    rows: Vec<(i32, i32)>,
    observed_at: u64,
    /// The Unix mode bits; `None` where there are none.
    mode: Option<u32>,
}

fn read_hint(path: &std::path::Path, account: &str) -> Result<HintRead, String> {
    let contents = std::fs::read_to_string(path)
        .map_err(|error| format!("read {}: {error}", path.display()))?;
    let document: serde_json::Value = serde_json::from_str(&contents)
        .map_err(|error| format!("parse {}: {error}", path.display()))?;
    if document["schema_version"] != serde_json::json!(script::bank_hints::SCHEMA_VERSION) {
        return Err(format!("schema_version: {contents}"));
    }
    if document["profile"] != serde_json::json!(PROFILE)
        || document["account"] != serde_json::json!(account)
    {
        return Err(format!("identity: {contents}"));
    }
    let rows: Vec<(i32, i32)> = serde_json::from_value(document["rows"].clone())
        .map_err(|error| format!("rows: {error}: {contents}"))?;
    let observed_at = document["observed_at_unix"]
        .as_u64()
        .ok_or_else(|| format!("observed_at_unix: {contents}"))?;
    #[cfg(unix)]
    let mode = {
        use std::os::unix::fs::PermissionsExt;
        Some(
            std::fs::metadata(path)
                .map_err(|error| format!("stat {}: {error}", path.display()))?
                .permissions()
                .mode()
                & 0o777,
        )
    };
    #[cfg(not(unix))]
    let mode = None;
    Ok(HintRead {
        contents,
        rows,
        observed_at,
        mode,
    })
}
