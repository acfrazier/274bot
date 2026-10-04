//! BANK-CORE L-withdraw: a native exact `WithdrawTo` of 7 coins at a real
//! Draynor booth through real Play, on the shared transfer kernel.
//!
//! A fresh account is moved to the bank, its pack cleared and coins seeded
//! into the bank only (no random content). The native script then selects
//! Draynor, opens its booth, withdraws to exactly 7 coins and closes. The
//! cell passes only when 7 coins are held, the bank lost exactly 7, and the
//! whole cell finished inside 60 s. It saves a JSON receipt and a
//! CPU-rendered PNG below `LIVE_EVIDENCE_DIR`.
//!
//! Like the Gatherer live cells, the client cache is the revision's decoded
//! snapshot copied once and reused: `BOT_CACHE_DIR` is the copied versioned
//! snapshot and `CLIENT_UNPACK_DIR` its copied unpack root. A launch never
//! fetches or deletes that immutable cache.
//!
//! `LIVE=1 BOT_CPU=1 BOT_LIVE_NAME_PREFIX=bk GATHERER_NAV_PACK=<pack> GATHERER_ENGINE_DIR=<engine> GATHERER_CATALOG_ROOT=<catalog> GATHERER_GAME_PORT=<port> GATHERER_HTTP_PORT=<port> BOT_CACHE_DIR=<unpack-root>/<version> CLIENT_UNPACK_DIR=<unpack-root> LIVE_EVIDENCE_DIR=<evidence-root> cargo test -p host-play --lib live_bank_core_exact_withdraw_at_draynor -- --ignored --nocapture --test-threads=1`

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

use super::bank_npc_live::write_teller_png;
use super::{run_with_template, tele_args, ProfileOptions, SharedClientTemplate};

const COINS: i32 = 995;
const SEEDED: i32 = 50;
const TARGET: i32 = 7;
/// Draynor bank floor, inside the booth row.
const DRAYNOR: WorldTile = WorldTile {
    x: 3092,
    z: 3243,
    level: 0,
};
const CELL_BOUND: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Prep {
    WaitLogin,
    SkipTutorial,
    Logout,
    WaitRelog,
    Teleport,
    WaitArrive,
    Clear,
    WaitClear,
    SeedBank,
    Ready,
    Failed,
}

#[derive(Debug, Clone, Default, serde::Serialize)]
struct Trace {
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
    cell_ms: Option<u128>,
    live_held: Option<i32>,
    live_bank_open: Option<bool>,
    failure: Option<String>,
    passed: bool,
}

struct Cell {
    phase: Prep,
    started: Instant,
    last_action: Instant,
    offline_seen: bool,
    script_started: Option<Instant>,
    trace: Trace,
    terminal: bool,
    evidence_dir: PathBuf,
    capture_started: bool,
    capture_written: bool,
    capture_error: Option<String>,
}

impl Cell {
    fn fail(&mut self, message: String) {
        if !self.terminal {
            self.trace.failure = Some(message);
            self.terminal = true;
        }
        self.phase = Prep::Failed;
    }
}

fn count(items: &[api::snapshot::ItemView], id: i32) -> i32 {
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
                        cell.trace.cell_ms = Some(cell.started.elapsed().as_millis());
                        if !cell.trace.closed {
                            drop(cell);
                            return Ok(self.blocked("Close returned with the bank still open"));
                        }
                        if cell.started.elapsed() > CELL_BOUND {
                            let message = format!(
                                "the cell took {:?}, over its {CELL_BOUND:?} bound",
                                cell.started.elapsed()
                            );
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
    account: &str,
    trace: &Trace,
) -> Result<(), String> {
    std::fs::create_dir_all(directory)
        .map_err(|error| format!("create {}: {error}", directory.display()))?;
    let step = if trace.passed {
        "01-final"
    } else {
        "FAIL-final"
    };
    let png = directory.join(format!("{step}.png"));
    let png_result = write_teller_png(client, &png);
    let receipt = serde_json::json!({
        "scenario": "bank_core_l_withdraw_exact_coins",
        "account": account,
        "bank_floor": tile(DRAYNOR),
        "seed": { "bank_coins": SEEDED, "pack": "cleared" },
        "request": { "action": "WithdrawTo", "id": COINS, "target": TARGET },
        "trace": trace,
        "png": png.file_name().and_then(|name| name.to_str()),
        "png_error": png_result.as_ref().err(),
    });
    let json = directory.join(format!("{step}.json"));
    std::fs::write(
        &json,
        serde_json::to_vec_pretty(&receipt).map_err(|error| error.to_string())?,
    )
    .map_err(|error| format!("write {}: {error}", json.display()))?;
    png_result
}

fn frame(client: &mut Client, shared: &Mutex<Cell>, account: &str) {
    let mut snapshot = GameSnapshot::new();
    snapshot.rebuild(client);
    let now = Instant::now();
    let capture = {
        let mut cell = shared.lock();
        if client.ingame && client.scene_state == 2 {
            cell.trace.live_held = Some(count(snapshot.inventory(), COINS));
            cell.trace.live_bank_open = Some(snapshot.bank_component_id() >= 0);
        }
        if !cell.terminal && now.duration_since(cell.started) > CELL_BOUND {
            let message = format!("the cell exceeded {CELL_BOUND:?} in {:?}", cell.phase);
            cell.fail(message);
        }
        let spaced = now.duration_since(cell.last_action) >= Duration::from_millis(400);
        match cell.phase {
            Prep::WaitLogin => {
                if client.ingame && client.scene_state == 2 && snapshot.local_player().is_some() {
                    cell.phase = Prep::SkipTutorial;
                    cell.last_action = now;
                }
            }
            Prep::SkipTutorial if spaced => {
                let _ = interact::cheat(client, "setvar tutorial 1000");
                cell.last_action = now;
                cell.phase = Prep::Logout;
            }
            Prep::Logout if now.duration_since(cell.last_action) >= Duration::from_secs(2) => {
                let ifaces = Arc::clone(&client.ifaces);
                if interact::logout(client, &ifaces) {
                    cell.offline_seen = false;
                    cell.phase = Prep::WaitRelog;
                    cell.last_action = now;
                } else {
                    cell.fail("tutorial relog logout interface was unavailable".into());
                }
            }
            Prep::WaitRelog => {
                if !client.ingame {
                    cell.offline_seen = true;
                } else if cell.offline_seen
                    && client.scene_state == 2
                    && snapshot.local_player().is_some()
                    && snapshot.inventory_size() > 0
                {
                    cell.phase = Prep::Teleport;
                    cell.last_action = now;
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
                    cell.phase = Prep::SeedBank;
                    cell.last_action = now;
                }
            }
            Prep::SeedBank if spaced => {
                let _ = interact::cheat(client, &format!("givebank coins {SEEDED}"));
                cell.last_action = now;
                cell.phase = Prep::Ready;
            }
            _ => {}
        }
        let ready = cell.terminal
            && (!cell.trace.passed
                || (cell.trace.live_bank_open == Some(false)
                    && cell.trace.live_held == Some(TARGET)));
        if ready && !cell.capture_started {
            cell.capture_started = true;
            Some((cell.evidence_dir.clone(), cell.trace.clone()))
        } else {
            None
        }
    };
    if let Some((directory, trace)) = capture {
        let result = save_evidence(client, &directory, account, &trace);
        let mut cell = shared.lock();
        cell.capture_written = true;
        cell.capture_error = result.err();
    }
}

fn live_profile(scratch: &Path) -> Result<ProfileOptions, String> {
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
    let cell = Arc::new(Mutex::new(Cell {
        phase: Prep::WaitLogin,
        started: Instant::now(),
        last_action: Instant::now(),
        offline_seen: false,
        script_started: None,
        trace: Trace::default(),
        terminal: false,
        evidence_dir: evidence_dir.clone(),
        capture_started: false,
        capture_written: false,
        capture_error: None,
    }));
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
    .expect("start real Play");
    cell.lock().started = Instant::now();
    let start = play.script_start_handle();
    let hard_stop = Instant::now() + CELL_BOUND + Duration::from_secs(20);
    loop {
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
        assert!(
            Instant::now() < hard_stop,
            "FAIL L-withdraw: no evidence; trace={:#?}",
            cell.lock().trace
        );
        std::thread::sleep(Duration::from_millis(100));
    }
}
