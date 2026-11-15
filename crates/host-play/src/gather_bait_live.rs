//! L-GATHER-BAIT (design-bank-snapshot §7 S4): the Gatherer's trip
//! admission over the host bank memory, through real Play at Draynor's real
//! sardine/herring fishing spots and its real booth.
//!
//! A fresh account is logged in on the shared tutorial/relog fixture of
//! [`super::bank_core_live`] (teleport beside the Draynor booth, pack
//! cleared, per-account seeds). The real Gatherer card then runs Bait
//! fishing (`fishing.saltfish.op3`, a fishing rod and one bait a catch) with
//! `baitTarget = 10` and `Bank` disposition over a Custom work area on the
//! two content-placed spots south of the bank. The
//! cell decides on bait and bank-trip evidence, never on incidental drops.
//! This M1 bank-only fixture explicitly sets `buyTools = false`, preserving
//! its bait-only trip oracle while the default-on shop cells run separately.
//! Its supply stop is a parked native `Blocked` status with the script still
//! `Running` and no terminal failure.
//!
//! - **banked** (bug 9, F1): the bank is seeded with 3 bait and a rod and
//!   the pack is empty, so the memory is `Unknown` at Start. Assert one trip
//!   (`bank trip due` once) that withdraws exactly 3 bait, then — when the
//!   bait runs out — a parked supply block in place, with no second
//!   `bank trip due` and the account still at the fishing area; the memory
//!   ends `Session` without bait. After the trip the Gatherer walks the real
//!   route back from the booth to the spots (no shore teleport).
//! - **stale hint** (D4 stale positive): before login the account's hint
//!   file claims 20 bait the bank does not hold; the rod is in the pack.
//!   The first login loads the file (`Unknown` → `Hint`, a new process),
//!   so the card starts on a `Hint` that claims bait. Assert exactly one
//!   trip, then a parked `supply-missing` at the closed bank; the memory
//!   ends `Session` without bait.
//!
//! Every Gatherer status line is captured through the hostlog sink (the
//! `last_event` transitions are the trip witness) and saved beside the JSON
//! receipt and CPU-rendered PNG below `LIVE_EVIDENCE_DIR`. Deadline: 10
//! minutes after the seed is posted.
//!
//! `LIVE=1 BOT_CPU=1 BOT_LIVE_NAME_PREFIX=s4b GATHERER_NAV_PACK=<pack> GATHERER_ENGINE_DIR=<engine> GATHERER_CATALOG_ROOT=<catalog> GATHERER_GAME_PORT=<port> GATHERER_HTTP_PORT=<port> BOT_CACHE_DIR=<unpack-root>/<version> CLIENT_UNPACK_DIR=<unpack-root> LIVE_EVIDENCE_DIR=<evidence-root> isohome cargo test -p host-play --features live-harness,test-support --lib gather_bait_live:: -- --ignored --nocapture --test-threads=1`

use std::path::PathBuf;
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use api::bank_memory::Origin;
use api::snapshot::{GameSnapshot, WorldTile};
use parking_lot::Mutex;
use script::native::{NativePhase, ScriptStatus, StatusValue};
use vault::{Profile, ProfileSettings};

use super::bank_core_live::{
    check_prerequisites, count, frame, live_profile, Cell, Prep, CAPTURE_GRACE,
    PREPARATION_DEADLINE,
};
use super::{run_with_template, SharedClientTemplate};

const FISHING_ROD: i32 = 307;
const FISHING_BAIT: i32 = 313;
const PROFILE: &str = "local-289";
const CELL_BOUND: Duration = Duration::from_secs(600);
/// Between Draynor's two content-placed saltfish spots (`m48_50` NPC 327 at
/// 3085,3230 and 3086,3227), south of the bank.
const SPOTS: WorldTile = WorldTile {
    x: 3086,
    z: 3229,
    level: 0,
};
const RADIUS: i32 = 8;
const BANKED_BAIT: i64 = 3;
const HINTED_BAIT: i32 = 20;
const BANKED_SEED: &[&str] = &[
    "setstat fishing 20",
    "setstat defence 99",
    "setstat hitpoints 99",
    "givebank fishing_bait 3",
    "givebank fishing_rod 1",
];
const STALE_SEED: &[&str] = &[
    "setstat fishing 20",
    "setstat defence 99",
    "setstat hitpoints 99",
    "give fishing_rod 1",
];
const TRIP_DUE: &str = "bank trip due";
const MAX_LOG_LINES: usize = 20_000;

/// Every slot-tagged hostlog line, in order (the process-wide sink; each
/// cell keeps only its own account's lines).
struct SlotLog {
    lines: StdMutex<Vec<(String, String)>>,
}

impl api::hostlog::Sink for SlotLog {
    fn record(&self, record: &api::hostlog::Record<'_>) {
        let Some(slot) = record.slot else {
            return;
        };
        let mut lines = self.lines.lock().expect("slot log lock");
        if lines.len() < MAX_LOG_LINES {
            lines.push((slot.to_owned(), record.message.to_owned()));
        }
    }
}

static SLOT_LOG: SlotLog = SlotLog {
    lines: StdMutex::new(Vec::new()),
};

fn account_lines(account: &str) -> Vec<String> {
    SLOT_LOG
        .lines
        .lock()
        .expect("slot log lock")
        .iter()
        .filter(|(slot, _)| slot == account)
        .map(|(_, message)| message.clone())
        .collect()
}

/// Phase labels written by the native status formatter.
const STATUS_PHASES: [&str; 5] = ["preparing", "working", "waiting", "blocked", "complete"];

/// The `last_event` of one native Gatherer status line. The formatter at
/// `script/src/slot/compiled.rs:333-343` writes
/// `native Gatherer run=<n> phase=<label> failure=<code>[details] last_event=<event>`.
/// Any other line, or a line with a malformed run, phase or empty event, gives `None`.
fn gatherer_status_event(line: &str) -> Option<&str> {
    let rest = line.strip_prefix("native Gatherer run=")?;
    let (run, rest) = rest.split_once(" phase=")?;
    if run.is_empty() || !run.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let (phase, rest) = rest.split_once(" failure=")?;
    if !STATUS_PHASES.contains(&phase) {
        return None;
    }
    let (_, event) = rest.split_once(" last_event=")?;
    (!event.is_empty()).then_some(event)
}

/// The Gatherer `last_event` values in the order the status lines changed
/// them; a repeated value (another field moved) is one transition.
fn event_transitions(lines: &[String]) -> Vec<String> {
    let mut events: Vec<String> = Vec::new();
    for event in lines.iter().filter_map(|line| gatherer_status_event(line)) {
        if events.last().map(String::as_str) != Some(event) {
            events.push(event.to_owned());
        }
    }
    events
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
enum Variant {
    Banked,
    StaleHint,
}

impl Variant {
    const fn label(self) -> &'static str {
        match self {
            Self::Banked => "banked",
            Self::StaleHint => "stale-hint",
        }
    }

    const fn seed(self) -> &'static [&'static str] {
        match self {
            Self::Banked => BANKED_SEED,
            Self::StaleHint => STALE_SEED,
        }
    }
}

fn rod_held(snapshot: &GameSnapshot) -> bool {
    count(snapshot.inventory(), FISHING_ROD) == 1
}

const TICK_SAMPLES: usize = 11;

/// Frame-observed applications of consecutive PLAYER_INFO packets, not frame ticks.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub(super) struct PlayerInfoClock {
    #[serde(skip)]
    last: Option<(u64, u64, Instant)>,
    interarrival_ms: Vec<u128>,
    coalesced_observations: usize,
    pub(super) median_ms: Option<u128>,
}

impl PlayerInfoClock {
    pub(super) fn observe(&mut self, session: u64, count: u64, now: Instant) {
        if self
            .last
            .is_some_and(|(previous_session, previous_count, _)| {
                session == previous_session && count == previous_count
            })
        {
            return;
        }
        let Some((previous_session, previous_count, previous_at)) =
            self.last.replace((session, count, now))
        else {
            return;
        };
        if session != previous_session {
            self.interarrival_ms.clear();
            self.coalesced_observations = 0;
            self.median_ms = None;
            return;
        }
        if count.checked_sub(previous_count) != Some(1) {
            self.coalesced_observations += 1;
            return;
        }
        if self.interarrival_ms.len() < TICK_SAMPLES {
            self.interarrival_ms
                .push(now.duration_since(previous_at).as_millis());
            if self.interarrival_ms.len() == TICK_SAMPLES {
                let mut sorted: [u128; TICK_SAMPLES] = self
                    .interarrival_ms
                    .as_slice()
                    .try_into()
                    .expect("the sample count is exactly TICK_SAMPLES");
                sorted.sort_unstable();
                self.median_ms = Some(sorted[TICK_SAMPLES / 2]);
            }
        }
    }

    pub(super) fn accepted(&self) -> bool {
        self.median_ms
            .is_some_and(|median| (570..=630).contains(&median))
    }
}

/// What the test thread observed, in order.
#[derive(Debug, Clone, Default, serde::Serialize)]
struct BaitTrace {
    variant: Option<Variant>,
    player_info_clock: PlayerInfoClock,
    home: Option<String>,
    hint_path: Option<String>,
    hint_written: Option<String>,
    cell_start_unix: u64,
    start_origin: Option<String>,
    start_rows: Option<Vec<(i32, i32)>>,
    /// Polled `last_event` transitions with the ms since Start.
    polled_events: Vec<(u128, String)>,
    /// `last_event` transitions from the hostlog status lines.
    logged_events: Vec<String>,
    trip_due_count: Option<usize>,
    max_bait: i64,
    final_bait: Option<i64>,
    trips: Option<i64>,
    yielded: Option<i64>,
    final_phase: Option<String>,
    final_phase_text: Option<String>,
    slot_state: Option<String>,
    slot_running: Option<bool>,
    failure_code: Option<String>,
    failure_message: Option<String>,
    block_reason: Option<String>,
    final_bank_open: Option<bool>,
    final_tile: Option<[i32; 3]>,
    final_distance_from_spots: Option<i32>,
    final_origin: Option<String>,
    final_rows: Option<Vec<(i32, i32)>>,
    start_to_block_ms: Option<u128>,
    /// Blocked polls that kept observing because the parked bank was still open.
    blocked_polls_with_open_bank: usize,
    verdict_failure: Option<String>,
}

fn integer(status: &ScriptStatus, key: &str) -> Option<i64> {
    status
        .fields
        .iter()
        .find(|field| field.key == key)
        .and_then(|field| match field.value {
            StatusValue::Integer(value) => Some(value),
            _ => None,
        })
}

fn text<'a>(status: &'a ScriptStatus, key: &str) -> Option<&'a str> {
    status
        .fields
        .iter()
        .find(|field| field.key == key)
        .and_then(|field| match &field.value {
            StatusValue::Text(value) => Some(value.as_ref()),
            _ => None,
        })
}

fn settings() -> serde_json::Map<String, serde_json::Value> {
    let serde_json::Value::Object(bag) = serde_json::json!({
        "skill": "Fishing",
        "fishingMethod": "fishing.saltfish.op3",
        "baitTarget": 10,
        "location": "Custom",
        "customTile": {"x": SPOTS.x, "z": SPOTS.z, "level": SPOTS.level},
        "radius": RADIUS,
        "disposition": "Bank",
        "bank": "Nearest",
        "allowTeleports": false,
        "allowWilderness": false,
        "deathPolicy": "Stop",
        "buyTools": false,
    }) else {
        unreachable!("a JSON object literal");
    };
    bag
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

fn origin_name(origin: Origin) -> String {
    format!("{origin:?}")
}

/// The pass rule for the finished run; `Err` names the first broken claim.
fn verdict(variant: Variant, trace: &BaitTrace) -> Result<(), String> {
    if !trace.player_info_clock.accepted() {
        return Err(format!(
            "PLAYER_INFO median must be 600 ms +/-30 ms: {:?}",
            trace.player_info_clock
        ));
    }
    let trips_due = trace.trip_due_count.unwrap_or(0);
    if trips_due != 1 {
        return Err(format!(
            "expected exactly one `{TRIP_DUE}`, saw {trips_due}: {:?}",
            trace.logged_events
        ));
    }
    if trace.failure_code.is_some() || trace.failure_message.is_some() {
        return Err(format!(
            "a parked supply block must have failure=None: {:?} {:?}",
            trace.failure_code, trace.failure_message
        ));
    }
    if trace.final_phase.as_deref() != Some("Blocked")
        || trace.final_phase_text.as_deref() != Some("blocked")
        || trace.slot_running != Some(true)
    {
        return Err(format!(
            "expected a Running slot with native phase Blocked / label blocked: phase={:?} label={:?} slot={:?}",
            trace.final_phase, trace.final_phase_text, trace.slot_state
        ));
    }
    if trace.final_bank_open != Some(false) {
        return Err(format!(
            "the park must leave the bank closed: {:?}",
            trace.final_bank_open
        ));
    }
    let message = trace.block_reason.as_deref().unwrap_or("");
    if !(message.starts_with("no Fishing bait in the pack, worn, or in the bank as seen ")
        && message.ends_with("; nobody sells it; waiting: put a Fishing bait in the pack"))
    {
        return Err(format!(
            "the block_reason must name the bait and observed bank: {message:?}"
        ));
    }
    if trace.final_origin.as_deref() != Some("Session") {
        return Err(format!(
            "the memory must end Session after the open bank: {:?}",
            trace.final_origin
        ));
    }
    let rows = trace.final_rows.as_deref().unwrap_or(&[]);
    if rows.iter().any(|&(id, _)| id == FISHING_BAIT) {
        return Err(format!("the Session memory still lists bait: {rows:?}"));
    }
    match variant {
        Variant::Banked => {
            if trace.start_origin.as_deref() != Some("Unknown") {
                return Err(format!(
                    "a fresh account starts Unknown: {:?}",
                    trace.start_origin
                ));
            }
            if trace.max_bait != BANKED_BAIT {
                return Err(format!(
                    "the one trip must withdraw exactly {BANKED_BAIT} bait (the clamped target), max seen {}",
                    trace.max_bait
                ));
            }
            if trace.final_bait != Some(0) {
                return Err(format!("the bait must run out: {:?}", trace.final_bait));
            }
            if trace.trips != Some(1) {
                return Err(format!("one completed trip: {:?}", trace.trips));
            }
            if trace
                .final_distance_from_spots
                .is_none_or(|distance| distance > RADIUS)
            {
                return Err(format!(
                    "supply-missing must fire in place at the spots, not after a walk: tile {:?} distance {:?}",
                    trace.final_tile, trace.final_distance_from_spots
                ));
            }
            if rows.iter().any(|&(id, _)| id == FISHING_ROD) {
                return Err(format!("the rod was withdrawn: {rows:?}"));
            }
        }
        Variant::StaleHint => {
            if trace.start_origin.as_deref() != Some("Hint")
                || trace.start_rows.as_deref() != Some(&[(FISHING_BAIT, HINTED_BAIT)][..])
            {
                return Err(format!(
                    "the card must start on the loaded hint: {:?} {:?}",
                    trace.start_origin, trace.start_rows
                ));
            }
            if trace.max_bait != 0 {
                return Err(format!("the bank held no bait: {}", trace.max_bait));
            }
            if trace.trips.unwrap_or(0) != 0 {
                return Err(format!(
                    "the trip ends at the bank, never returns: {:?}",
                    trace.trips
                ));
            }
        }
    }
    Ok(())
}

/// The cell's decision at one polled Blocked status, after the trace records
/// its facts. A clean park (no failure, slot Running) whose bank is still open
/// gives `None`: the park closes that bank on a later poll
/// (`script/src/gatherer/park.rs:55-72`), so the cell keeps observing under its
/// unchanged deadline. Every other Blocked sample gives the verdict at once.
fn blocked_verdict(variant: Variant, trace: &BaitTrace) -> Option<Result<(), String>> {
    let clean_park = trace.failure_code.is_none()
        && trace.failure_message.is_none()
        && trace.slot_running == Some(true);
    if clean_park && trace.final_bank_open == Some(true) {
        return None;
    }
    Some(verdict(variant, trace))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stage {
    Prepare,
    Running { since: Instant },
    Done,
}

fn run_cell(variant: Variant) {
    assert_eq!(std::env::var("LIVE").as_deref(), Ok("1"), "requires LIVE=1");
    let root = PathBuf::from(
        std::env::var_os("LIVE_EVIDENCE_DIR").expect("LIVE_EVIDENCE_DIR names the evidence root"),
    );
    let home = std::env::var("HOME").expect("HOME is the throwaway isohome directory");
    let account = super::mint_live_names(1).pop().expect("one live account");
    let epoch = unix_now();
    let evidence_dir = root.join(format!(
        "l-gather-bait-{}_{account}_utc-{epoch}Z",
        variant.label()
    ));
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
        "the hint path is keyed by this profile"
    );
    let account_key = api::local_state::AccountKey::try_new(PROFILE, &account)
        .expect("a minted live name is a login identity");
    let hint_path = crate::local_state::StateRoot::new(PathBuf::from(&home).join(".274bot"))
        .bank_hint_file(PROFILE, account_key.account());
    assert!(
        hint_path.starts_with(&home),
        "the hint path {} must sit under HOME {home}",
        hint_path.display()
    );
    let trace = Arc::new(Mutex::new(BaitTrace {
        variant: Some(variant),
        home: Some(home.clone()),
        hint_path: Some(hint_path.display().to_string()),
        ..BaitTrace::default()
    }));
    if variant == Variant::StaleHint {
        // A new process: the first login finds the memory `Unknown` and
        // loads this file as `Hint` (design-bank-snapshot §1.4).
        let contents = serde_json::json!({
            "schema_version": 1,
            "profile": PROFILE,
            "account": account_key.account(),
            "observed_at_unix": epoch,
            "rows": [[FISHING_BAIT, HINTED_BAIT]],
        })
        .to_string();
        let parent = hint_path.parent().expect("hint parent");
        std::fs::create_dir_all(parent).expect("create the hint directory");
        vault::write_private_file(&hint_path, contents.as_bytes()).expect("write the stale hint");
        trace.lock().hint_written = Some(contents);
    }
    let _ = api::hostlog::install_sink(&SLOT_LOG);
    let template = SharedClientTemplate::load(Arc::clone(&profile)).expect("client template");
    let seed_ready: fn(&GameSnapshot) -> bool = match variant {
        // Bank seeds are not visible in the pack.
        Variant::Banked => |_| true,
        Variant::StaleHint => rod_held,
    };
    let cell = Cell::new(
        CELL_BOUND,
        variant.seed(),
        seed_ready,
        &[FISHING_ROD, FISHING_BAIT],
        serde_json::json!({
            "scenario": "gather_bait_l_gather_bait",
            "variant": variant.label(),
            "seed": variant.seed(),
            "settings": settings(),
            "spots": [SPOTS.x, SPOTS.z, SPOTS.level],
            "cell_bound_ms": CELL_BOUND.as_millis(),
            "cell_clock_starts_after": "ingame && scene_state == 2 and seed posted",
        }),
        |_| true,
        evidence_dir.clone(),
    );
    let preparation_started = cell.preparation_started;
    let cell = Arc::new(Mutex::new(cell));
    let monitor = Mutex::new(GameSnapshot::new());
    let frame_cell = Arc::clone(&cell);
    let frame_account = account.clone();
    let frame_trace = Arc::clone(&trace);
    let play = run_with_template(
        template,
        true,
        vec![Profile {
            username: account.clone(),
            password: account.clone().into(),
            uid: 274_279_104,
            settings: ProfileSettings::default(),
        }],
        |_| (None, None),
        move |client, _, _| {
            if client.ingame && client.scene_state == 2 {
                frame_trace.lock().player_info_clock.observe(
                    client.gens.session,
                    client.gens.player_info,
                    Instant::now(),
                );
            }
            frame(client, &frame_cell, &frame_account);
            if client.ingame && client.scene_state == 2 {
                let mut monitor = monitor.lock();
                monitor.rebuild(client);
                frame_trace.lock().final_bank_open = Some(monitor.bank_component_id() >= 0);
            }
        },
    )
    .unwrap_or_else(|error| {
        panic!("HARNESS PREREQUISITE FAILURE (before the cell deadline): starting login: {error}")
    });
    let mut login_ready = false;
    let mut stage = Stage::Prepare;
    let fail = |cell: &Arc<Mutex<Cell>>, trace: &Arc<Mutex<BaitTrace>>, message: String| {
        trace.lock().verdict_failure = Some(message.clone());
        cell.lock().fail(message);
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
            "FAIL L-GATHER-BAIT {}: no evidence by its fixed cell deadline; stage={stage:?} trace={:#?}",
            variant.label(),
            trace.lock()
        );
        let failed = cell.lock().terminal && !cell.lock().trace.passed;
        if !failed {
            match stage {
                Stage::Prepare => {
                    let median = trace.lock().player_info_clock.median_ms;
                    let clock_ready = median.is_some_and(|value| (570..=630).contains(&value));
                    if median.is_some() && !clock_ready {
                        fail(
                            &cell,
                            &trace,
                            format!(
                                "Engine L PLAYER_INFO median {median:?} ms is outside 600 ms +/-30 ms; no Gatherer Start was sent"
                            ),
                        );
                        stage = Stage::Done;
                    }
                    let begin = {
                        let mut guard = cell.lock();
                        if clock_ready
                            && guard.phase == Prep::Ready
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
                        let rows = play.bank_rows(&account);
                        {
                            let mut guard = trace.lock();
                            guard.cell_start_unix = unix_now();
                            guard.start_origin = Some(origin_name(rows.origin));
                            guard.start_rows = Some(rows.rows.clone());
                        }
                        match play.script_start(
                            &account,
                            script::CompiledId("Gatherer"),
                            settings(),
                        ) {
                            Ok(()) => {
                                stage = Stage::Running {
                                    since: Instant::now(),
                                }
                            }
                            Err(error) => fail(&cell, &trace, format!("Gatherer Start: {error}")),
                        }
                    }
                }
                Stage::Running { since } => {
                    if let Some(status) = play.script_native_status(&account) {
                        let blocked = status.phase == NativePhase::Blocked;
                        {
                            let mut guard = trace.lock();
                            if let Some(event) = text(&status, "last_event") {
                                if guard
                                    .polled_events
                                    .last()
                                    .is_none_or(|(_, last)| last != event)
                                {
                                    guard
                                        .polled_events
                                        .push((since.elapsed().as_millis(), event.to_owned()));
                                }
                            }
                            if let Some(bait) = integer(&status, "bait") {
                                guard.max_bait = guard.max_bait.max(bait);
                                guard.final_bait = Some(bait);
                            }
                            guard.trips = integer(&status, "trips");
                            guard.yielded = integer(&status, "yielded");
                            guard.final_phase = Some(format!("{:?}", status.phase));
                            guard.final_phase_text = text(&status, "phase").map(str::to_owned);
                            guard.slot_state = Some(format!("{:?}", play.script_state(&account)));
                            guard.slot_running =
                                Some(play.script_state(&account) == script::RunState::Running);
                        }
                        if blocked {
                            let tile = play
                                .statuses()
                                .into_iter()
                                .find(|slot| slot.username == account)
                                .map(|slot| [slot.tile_x, slot.tile_z, slot.tile_level]);
                            let rows = play.bank_rows(&account);
                            let logged = event_transitions(&account_lines(&account));
                            let result = {
                                let mut guard = trace.lock();
                                guard.failure_code = status
                                    .failure
                                    .as_ref()
                                    .map(|failure| failure.code.to_string());
                                guard.failure_message = status
                                    .failure
                                    .as_ref()
                                    .map(|failure| failure.message.to_string());
                                guard.block_reason =
                                    text(&status, "block_reason").map(str::to_owned);
                                guard.final_tile = tile;
                                guard.final_distance_from_spots =
                                    tile.filter(|tile| tile[2] == SPOTS.level).map(|tile| {
                                        (tile[0] - SPOTS.x).abs().max((tile[1] - SPOTS.z).abs())
                                    });
                                guard.final_origin = Some(origin_name(rows.origin));
                                guard.final_rows = Some(rows.rows.clone());
                                guard.trip_due_count =
                                    Some(logged.iter().filter(|event| *event == TRIP_DUE).count());
                                guard.logged_events = logged;
                                if guard.start_to_block_ms.is_none() {
                                    guard.start_to_block_ms = Some(since.elapsed().as_millis());
                                }
                                let result = blocked_verdict(variant, &guard);
                                if result.is_none() {
                                    guard.blocked_polls_with_open_bank += 1;
                                }
                                result
                            };
                            match result {
                                None => {}
                                Some(Ok(())) => {
                                    let mut guard = cell.lock();
                                    guard.result = Some(serde_json::json!(*trace.lock()));
                                    guard.trace.passed = true;
                                    guard.terminal = true;
                                    stage = Stage::Done;
                                }
                                Some(Err(message)) => {
                                    fail(&cell, &trace, message);
                                    stage = Stage::Done;
                                }
                            }
                        }
                    }
                }
                Stage::Done => {}
            }
        }
        if cell.lock().terminal && cell.lock().result.is_none() {
            cell.lock().result = Some(serde_json::json!(*trace.lock()));
        }
        let written = {
            let guard = cell.lock();
            guard
                .capture_written
                .then(|| (guard.trace.clone(), guard.capture_error.clone()))
        };
        if let Some((cell_trace, error)) = written {
            let bait_trace = trace.lock().clone();
            let lines = account_lines(&account);
            let log_path = evidence_dir.join("hostlog-slot.txt");
            let _ = std::fs::write(&log_path, lines.join("\n"));
            let _ = std::fs::write(
                evidence_dir.join("trace.json"),
                serde_json::to_vec_pretty(&bait_trace).unwrap_or_default(),
            );
            assert!(
                error.is_none(),
                "evidence capture failed: {error:?}; {bait_trace:#?}"
            );
            assert!(
                cell_trace.passed,
                "FAIL L-GATHER-BAIT {}: {:?}; stage={stage:?}; trace={bait_trace:#?}; evidence={}",
                variant.label(),
                cell_trace.failure,
                evidence_dir.display()
            );
            println!(
                "PASS L-GATHER-BAIT {} account={account} trips_due={:?} max_bait={} trips={:?} failure={:?} events={:?} evidence={}",
                variant.label(),
                bait_trace.trip_due_count,
                bait_trace.max_bait,
                bait_trace.trips,
                bait_trace.failure_message,
                bait_trace.logged_events,
                evidence_dir.display()
            );
            let _ = std::fs::remove_dir_all(&scratch);
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
#[ignore = "requires LIVE=1, GATHERER_NAV_PACK/GATHERER_ENGINE_DIR/GATHERER_CATALOG_ROOT/BOT_CACHE_DIR/CLIENT_UNPACK_DIR/LIVE_EVIDENCE_DIR and a local 289 engine"]
fn live_gather_bait_one_trip_then_missing_in_place_at_draynor() {
    run_cell(Variant::Banked);
}

#[test]
#[ignore = "requires LIVE=1, GATHERER_NAV_PACK/GATHERER_ENGINE_DIR/GATHERER_CATALOG_ROOT/BOT_CACHE_DIR/CLIENT_UNPACK_DIR/LIVE_EVIDENCE_DIR and a local 289 engine"]
fn live_gather_bait_stale_hint_costs_one_trip_at_draynor() {
    run_cell(Variant::StaleHint);
}

/// Lines in the shape of the stale-hint receipt
/// (`mapped-live-deadline-81c4d822d/host.gather-bait-stale-hint`, `hostlog-slot.txt:30-34`).
#[test]
fn status_transitions_accept_run_tagged_gatherer_lines() {
    let lines = [
        "start script",
        "native Gatherer run=1 phase=working failure=none area=custom (3086,3229,0) r8 last_event=bank trip due",
        "bank memory observed rows=0 size=288 generation=3",
        "native Gatherer run=1 phase=blocked failure=none block_reason=no Fishing bait in the pack, worn, or in the bank as seen less than a minute ago this session (0 rows); nobody sells it; waiting: put a Fishing bait in the pack area=custom (3086,3229,0) r8 last_event=supply-missing",
        "native Gatherer run=12 phase=complete failure=none last_event=done",
    ]
    .map(str::to_owned);
    let events = event_transitions(&lines);
    assert_eq!(events, [TRIP_DUE, "supply-missing", "done"]);
    assert_eq!(events.iter().filter(|event| *event == TRIP_DUE).count(), 1);
}

#[test]
fn status_transitions_reject_unrelated_and_malformed_lines() {
    let lines = [
        // The retired untagged shape: the formatter no longer writes it.
        "native Gatherer phase=working failure=none last_event=bank trip due",
        "native Quester run=1 phase=working failure=none last_event=bank trip due",
        "script: native Gatherer run=1 phase=working failure=none last_event=bank trip due",
        "native Gatherer run= phase=working failure=none last_event=bank trip due",
        "native Gatherer run=1a phase=working failure=none last_event=bank trip due",
        "native Gatherer run=1 phase=fishing failure=none last_event=bank trip due",
        "native Gatherer run=1 phase=working last_event=bank trip due",
        "native Gatherer run=1 phase=working failure=none",
        "native Gatherer run=1 phase=working failure=none last_event=",
        "bank trip due",
        "",
    ]
    .map(str::to_owned);
    assert_eq!(event_transitions(&lines), Vec::<String>::new());
}

#[test]
fn status_transitions_fold_repeated_events() {
    let lines = [
        "native Gatherer run=1 phase=working failure=none last_event=started",
        "native Gatherer run=1 phase=working failure=none last_event=bank trip due",
        "native Gatherer run=1 phase=working failure=none action_state=walk last_event=bank trip due",
        "bank open booth",
        "native Gatherer phase=working failure=none last_event=started",
        "native Gatherer run=1 phase=working failure=none action_state=bank last_event=bank trip due",
        "native Gatherer run=1 phase=working failure=none last_event=deposit confirmed",
        "native Gatherer run=1 phase=blocked failure=none block_reason=no Fishing bait in the pack, worn, or in the bank as seen less than a minute ago this session (0 rows); nobody sells it; waiting: put a Fishing bait in the pack last_event=deposit confirmed",
    ]
    .map(str::to_owned);
    assert_eq!(
        event_transitions(&lines),
        ["started", TRIP_DUE, "deposit confirmed"]
    );
}

fn valid_trace(variant: Variant) -> BaitTrace {
    let mut player_info_clock = PlayerInfoClock::default();
    let started = Instant::now();
    for tick in 0..=TICK_SAMPLES as u64 {
        player_info_clock.observe(1, tick, started + Duration::from_millis(tick * 600));
    }
    BaitTrace {
        player_info_clock,
        final_phase: Some("Blocked".into()),
        final_phase_text: Some("blocked".into()),
        slot_running: Some(true),
        final_bank_open: Some(false),
        start_origin: Some(
            match variant {
                Variant::Banked => "Unknown",
                Variant::StaleHint => "Hint",
            }
            .into(),
        ),
        start_rows: (variant == Variant::StaleHint).then_some(vec![(FISHING_BAIT, HINTED_BAIT)]),
        trip_due_count: Some(1),
        max_bait: match variant {
            Variant::Banked => BANKED_BAIT,
            Variant::StaleHint => 0,
        },
        final_bait: Some(0),
        trips: Some(i64::from(variant == Variant::Banked)),
        block_reason: Some(
            "no Fishing bait in the pack, worn, or in the bank as seen less than a minute ago this session (0 rows); nobody sells it; waiting: put a Fishing bait in the pack"
                .into(),
        ),
        final_distance_from_spots: Some(0),
        final_origin: Some("Session".into()),
        final_rows: Some(Vec::new()),
        ..BaitTrace::default()
    }
}

#[test]
fn bait_oracle_accepts_core_shortage_text_after_one_observed_trip() {
    for variant in [Variant::Banked, Variant::StaleHint] {
        assert_eq!(verdict(variant, &valid_trace(variant)), Ok(()));
    }
}

#[test]
fn bait_oracle_rejects_wrong_park_state_provenance_and_trip_evidence() {
    for variant in [Variant::Banked, Variant::StaleHint] {
        let mut trace = valid_trace(variant);
        trace.final_phase = Some("Failed".into());
        assert!(verdict(variant, &trace).is_err());

        let mut trace = valid_trace(variant);
        trace.block_reason = trace
            .block_reason
            .map(|message| message.replace("Fishing bait", "Feather"));
        assert!(verdict(variant, &trace).is_err());

        let mut trace = valid_trace(variant);
        trace.block_reason = Some("supply-missing:Fishing bait".into());
        assert!(verdict(variant, &trace).is_err());

        let mut trace = valid_trace(variant);
        trace.final_origin = Some("Hint".into());
        assert!(verdict(variant, &trace).is_err());

        let mut trace = valid_trace(variant);
        trace.trip_due_count = Some(2);
        assert!(verdict(variant, &trace).is_err());

        let mut trace = valid_trace(variant);
        trace.final_rows = Some(vec![(FISHING_BAIT, 1)]);
        assert!(verdict(variant, &trace).is_err());
    }
    let mut trace = valid_trace(Variant::Banked);
    trace.max_bait = BANKED_BAIT - 1;
    assert!(verdict(Variant::Banked, &trace).is_err());

    let mut trace = valid_trace(Variant::StaleHint);
    trace.trips = Some(1);
    assert!(verdict(Variant::StaleHint, &trace).is_err());
}

#[test]
fn blocked_park_with_open_bank_keeps_observing() {
    for variant in [Variant::Banked, Variant::StaleHint] {
        let mut trace = valid_trace(variant);
        trace.final_bank_open = Some(true);
        // The first-Blocked verdict would reject this early sample.
        assert!(verdict(variant, &trace).is_err());
        assert_eq!(blocked_verdict(variant, &trace), None);
    }
}

#[test]
fn blocked_park_with_closed_bank_reaches_verdict() {
    for variant in [Variant::Banked, Variant::StaleHint] {
        let mut trace = valid_trace(variant);
        trace.final_bank_open = Some(true);
        assert_eq!(blocked_verdict(variant, &trace), None);
        trace.final_bank_open = Some(false);
        assert_eq!(blocked_verdict(variant, &trace), Some(Ok(())));
    }
}

#[test]
fn blocked_true_failures_end_at_once_with_open_bank() {
    for variant in [Variant::Banked, Variant::StaleHint] {
        let mut trace = valid_trace(variant);
        trace.final_bank_open = Some(true);
        trace.failure_code = Some("supply-missing".into());
        assert!(matches!(blocked_verdict(variant, &trace), Some(Err(_))));

        let mut trace = valid_trace(variant);
        trace.final_bank_open = Some(true);
        trace.failure_message = Some("bank walk failed".into());
        assert!(matches!(blocked_verdict(variant, &trace), Some(Err(_))));

        let mut trace = valid_trace(variant);
        trace.final_bank_open = Some(true);
        trace.slot_running = Some(false);
        trace.slot_state = Some("Stopped".into());
        assert!(matches!(blocked_verdict(variant, &trace), Some(Err(_))));
    }
}

#[test]
fn blocked_park_with_closed_bank_still_rejects_invalid_facts() {
    for variant in [Variant::Banked, Variant::StaleHint] {
        let mut trace = valid_trace(variant);
        trace.final_bank_open = None;
        assert!(matches!(blocked_verdict(variant, &trace), Some(Err(_))));

        let mut trace = valid_trace(variant);
        trace.trip_due_count = Some(0);
        assert!(matches!(blocked_verdict(variant, &trace), Some(Err(_))));

        let mut trace = valid_trace(variant);
        trace.block_reason = Some("supply-missing:Fishing bait".into());
        assert!(matches!(blocked_verdict(variant, &trace), Some(Err(_))));

        let mut trace = valid_trace(variant);
        trace.final_origin = Some("Hint".into());
        assert!(matches!(blocked_verdict(variant, &trace), Some(Err(_))));

        let mut trace = valid_trace(variant);
        trace.final_rows = Some(vec![(FISHING_BAIT, 1)]);
        assert!(matches!(blocked_verdict(variant, &trace), Some(Err(_))));
    }
}

#[test]
fn player_info_clock_excludes_frames_coalescing_and_old_sessions() {
    let started = Instant::now();
    let mut clock = PlayerInfoClock::default();
    clock.observe(1, 5, started);
    clock.observe(1, 5, started + Duration::from_millis(20));
    assert!(clock.interarrival_ms.is_empty());
    clock.observe(1, 6, started + Duration::from_millis(600));
    assert_eq!(clock.interarrival_ms, [600]);
    clock.observe(1, 8, started + Duration::from_millis(1800));
    assert_eq!(clock.interarrival_ms, [600]);
    assert_eq!(clock.coalesced_observations, 1);
    clock.observe(2, 9, started + Duration::from_millis(2400));
    assert!(clock.interarrival_ms.is_empty());
    assert_eq!(clock.median_ms, None);
    assert_eq!(clock.coalesced_observations, 0);
    for tick in 1..=TICK_SAMPLES as u64 {
        clock.observe(
            2,
            9 + tick,
            started + Duration::from_millis(2400 + tick * 600),
        );
    }
    assert_eq!(clock.median_ms, Some(600));
    assert!(clock.accepted());
}

#[test]
fn bait_oracle_rejects_missing_or_wrong_engine_tick_rate() {
    let mut trace = valid_trace(Variant::Banked);
    for median in [None, Some(0), Some(569), Some(631)] {
        trace.player_info_clock.median_ms = median;
        assert!(verdict(Variant::Banked, &trace).is_err());
    }
    for median in [Some(570), Some(600), Some(630)] {
        trace.player_info_clock.median_ms = median;
        assert_eq!(verdict(Variant::Banked, &trace), Ok(()));
    }
}
