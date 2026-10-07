//! L-QUESTER-HINT (design-bank-snapshot §7 S3): the shipped Cook's Assistant
//! Path on one fixed account whose bank hint persists under one `HOME`
//! across four processes, each a `QUESTER_HINT_VARIANT`:
//!
//! - `cold`: no hint; the bank is seeded with the Path's items. Exactly one
//!   provisioning scan, then the withdraw at that same open bank, completion.
//! - `warm`: the saved hint covers every need. No scan; the first bank trip
//!   is the withdraw, which settles live.
//! - `stale_positive`: the hint claims eggs the bank no longer holds. One
//!   wasted withdraw trip, the memory corrected to a `Session` zero, the
//!   egg acquisition runs (no park), completion.
//! - `stale_negative`: the hint lacks the egg the bank holds (seeded after
//!   login, before Start). Exactly one scan, no egg pickup, the withdraw at
//!   that same bank visit, completion.
//!
//! The stale variants start a new process so the edited hint is what the
//! memory loads (design-bank-snapshot M-R2-3). Every run proves one
//! pre-completion bank-open generation, one explicit Start, zero deaths and
//! no park, and checks the hint the host saved afterwards.
//!
//! Run from the repository root against Engine A with a throwaway `HOME`
//! kept across the four invocations (isohome `ISOHOME_DIR`):
//!
//! ```text
//! QUESTER_LIVE_KEEP_HOME=1 QUESTER_LIVE_ACCOUNT=<fixed 1-12 byte name> QUESTER_LIVE_PASSWORD=<same> \
//! QUESTER_HINT_VARIANT=cold LIVE=1 BOT_CPU=1 BOT_NAV_BUILD=skip BOT_LIVE_NAME_PREFIX=bs3h \
//! WORLD_GAME_PORT=44594 WORLD_HTTP_PORT=1080 WORLD_NAV_PACK=<274bot.navpack> \
//! WORLD_ENGINE_DIR=<engine-A-dir> RS2B0T=<catalog-root> BOT_CACHE_DIR=<owned APFS cache clone> \
//! LIVE_EVIDENCE_DIR=<evidence-root> isohome cargo test -p host-play --test quester_hint_live \
//!   --features "live-harness test-support" live_quester_hint -- --ignored --nocapture --test-threads=1
//! ```
#![cfg(all(feature = "live-harness", feature = "test-support"))]

#[path = "common/quester_live.rs"]
mod quester_live;

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use api::game_data::SelectedGameData;
use api::hostlog::{Record, Sink};
use api::quest_facts::QuestCatalog;
use api::selected::{ClientRevision, RunKey};
use api::snapshot::{GameSnapshot, WorldTile};
use quester_live::{Cell, Mode, ObserveFamily, StartFamily};
use scenario::{Proof, Step, StepKind, Wait};
use script::native::{ScriptStatus, StatusValue};
use script::quester::compile::compile_path;
use script::quester::path::PathDocument;
use script::quester::runner::Quester;
use serde::Serialize;
use serde_json::{json, Value};

const COOK_PATH_JSON: &str = include_str!("../../script/paths/289/cook.json");
const COOK_START: WorldTile = WorldTile {
    x: 3209,
    z: 3215,
    level: 0,
};
const PROFILE: &str = "local-289";
/// Per-run fixed deadline (design-bank-snapshot §7 S3: 15 min per run).
const RUN_DEADLINE: Duration = Duration::from_secs(900);
const SCAN_BEGIN: &str = "quester cook: provision bank scan begin";
const WITHDRAW_BEGIN: &str = "quester cook: provision bank withdraw begin";
const EGG_ACQUISITION: &str = "recipe acquire:egg child take-egg begin";

static SESSION_LOGS: LazyLock<Mutex<Vec<SessionLogLine>>> =
    LazyLock::new(|| Mutex::new(Vec::new()));
static SESSION_LOG_SINK_INSTALLED: LazyLock<bool> =
    LazyLock::new(|| api::hostlog::install_sink(&SESSION_LOG_SINK));

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Variant {
    Cold,
    Warm,
    StalePositive,
    StaleNegative,
}

impl Variant {
    fn from_env() -> Result<Self, String> {
        match std::env::var("QUESTER_HINT_VARIANT").as_deref() {
            Ok("cold") => Ok(Self::Cold),
            Ok("warm") => Ok(Self::Warm),
            Ok("stale_positive") => Ok(Self::StalePositive),
            Ok("stale_negative") => Ok(Self::StaleNegative),
            other => Err(format!(
                "QUESTER_HINT_VARIANT must be cold|warm|stale_positive|stale_negative, got {other:?}"
            )),
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Cold => "cold",
            Self::Warm => "warm",
            Self::StalePositive => "stale_positive",
            Self::StaleNegative => "stale_negative",
        }
    }

    /// `givebank` seeds issued after login, before the fixture relog and
    /// Start. The cold run stocks the bank for every later run (2 eggs: one
    /// for the cold run, one for the warm run, none left for the stale
    /// positive); the stale negative puts one egg back behind the hint.
    fn bank_seeds(self) -> &'static [&'static str] {
        match self {
            Self::Cold => &[
                "givebank egg 2",
                "givebank bucket_milk 5",
                "givebank pot_flour 5",
            ],
            Self::Warm | Self::StalePositive => &[],
            Self::StaleNegative => &["givebank egg 1"],
        }
    }

    fn expected_scan_lines(self) -> usize {
        match self {
            Self::Cold | Self::StaleNegative => 1,
            Self::Warm | Self::StalePositive => 0,
        }
    }

    /// Withdraw trips begun: the stale positive's first withdraw stops at
    /// the missing egg and the re-plan withdraws the rest at the same open
    /// bank, so it may begin a second withdraw without a second trip.
    fn expected_withdraw_lines(self) -> std::ops::RangeInclusive<usize> {
        match self {
            Self::StalePositive => 1..=2,
            Self::Cold | Self::Warm | Self::StaleNegative => 1..=1,
        }
    }

    fn expects_egg_acquisition(self) -> bool {
        self == Self::StalePositive
    }

    /// The hint the host must have saved after the run: `[egg, milk, flour]`
    /// counts, `0` meaning the row is absent.
    fn expected_hint_after(self) -> [i32; 3] {
        match self {
            Self::Cold => [1, 4, 4],
            Self::Warm => [0, 3, 3],
            Self::StalePositive => [0, 2, 2],
            Self::StaleNegative => [0, 1, 1],
        }
    }
}

#[derive(Debug, Clone, Serialize)]
struct SessionLogLine {
    slot: Option<String>,
    tick: Option<u32>,
    source: String,
    level: String,
    message: String,
}

struct SessionLogSink;
static SESSION_LOG_SINK: SessionLogSink = SessionLogSink;

impl Sink for SessionLogSink {
    fn record(&self, record: &Record<'_>) {
        let line = SessionLogLine {
            slot: record.slot.map(str::to_owned),
            tick: record.tick,
            source: record.source.label().to_owned(),
            level: record.level.label().to_owned(),
            message: record.message.to_owned(),
        };
        SESSION_LOGS
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .push(line);
    }
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs())
}

fn item_id(selected: &SelectedGameData, alias: &str) -> Result<i32, String> {
    selected
        .item_by_alias(alias)
        .map(|item| item.id)
        .ok_or_else(|| format!("selected R289 data has no item alias {alias:?}"))
}

/// `[egg, milk, flour]` counts over rows.
fn item_counts(items: &[api::snapshot::ItemView], ids: &[i32; 3]) -> [i32; 3] {
    std::array::from_fn(|index| {
        items
            .iter()
            .filter(|item| item.def.id == ids[index] && item.count > 0)
            .map(|item| item.count)
            .sum()
    })
}

fn status_text<'a>(status: Option<&'a ScriptStatus>, key: &str) -> Option<&'a str> {
    status?
        .fields
        .iter()
        .find(|field| field.key == key)
        .and_then(|field| match &field.value {
            StatusValue::Text(value) => Some(value.as_ref()),
            _ => None,
        })
}

fn quest_complete(snapshot: &GameSnapshot) -> bool {
    snapshot.quest_statuses().iter().any(|row| {
        row.name.trim().eq_ignore_ascii_case("Cook's Assistant")
            && row.status() == api::snapshot::QuestListStatus::Complete
    })
}

fn perform_command(name: &'static str, command: String, proof: Proof) -> Step {
    Step {
        name,
        kind: StepKind::Perform {
            send: Box::new(move |client, _| api::interact::cheat(client, &command).is_sent()),
        },
        wait: Wait {
            arm: proof,
            budget_ticks: 100,
        },
    }
}

// ---- the hint file -------------------------------------------------------

/// The hint as the host writes it (design-bank-snapshot §1.4, schema 1).
#[derive(Debug, Clone, Serialize, serde::Deserialize)]
struct HintDocument {
    schema_version: u32,
    profile: String,
    account: String,
    observed_at_unix: u64,
    rows: Vec<(i32, i32)>,
}

fn hint_path(account: &str) -> Result<PathBuf, String> {
    let identity = script::bank_hints::account_component(account)
        .map_err(|error| format!("account {account:?} has no hint identity: {error:?}"))?;
    Ok(script::bot_file("bank-hints")
        .join(PROFILE)
        .join(format!("{identity}.json")))
}

fn read_hint(path: &Path) -> Result<HintDocument, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|error| format!("read hint {}: {error}", path.display()))?;
    serde_json::from_str(&text).map_err(|error| format!("parse hint {}: {error}", path.display()))
}

fn write_hint(path: &Path, document: &HintDocument) -> Result<(), String> {
    let text = serde_json::to_string(document).map_err(|error| error.to_string())?;
    std::fs::write(path, text).map_err(|error| format!("write hint {}: {error}", path.display()))
}

fn hint_count(document: &HintDocument, id: i32) -> i32 {
    document
        .rows
        .iter()
        .find(|(row, _)| *row == id)
        .map_or(0, |(_, count)| *count)
}

/// Set `id` to `count` (removing the row for `0`), keeping the rows sorted
/// and unique as the loader requires.
fn set_hint_count(document: &mut HintDocument, id: i32, count: i32) {
    document.rows.retain(|(row, _)| *row != id);
    if count > 0 {
        document.rows.push((id, count));
    }
    document.rows.sort_unstable_by_key(|(row, _)| *row);
}

/// The pre-launch edit each variant makes to the hint on disk; returns the
/// document as found (`None` when absent) and as left.
fn prepare_hint(
    variant: Variant,
    path: &Path,
    egg: i32,
) -> Result<(Option<HintDocument>, Option<HintDocument>), String> {
    let before = path.is_file().then(|| read_hint(path)).transpose()?;
    match variant {
        Variant::Cold => {
            if before.is_some() {
                std::fs::remove_file(path)
                    .map_err(|error| format!("remove stale hint {}: {error}", path.display()))?;
            }
            Ok((before, None))
        }
        Variant::Warm => {
            let document = before.clone().ok_or_else(|| {
                format!("warm run needs the cold run's hint at {}", path.display())
            })?;
            if hint_count(&document, egg) < 1 {
                return Err(format!(
                    "warm run needs a hint that still banks an egg: {:?}",
                    document.rows
                ));
            }
            Ok((before, Some(document)))
        }
        Variant::StalePositive => {
            let mut document = before.clone().ok_or_else(|| {
                format!(
                    "stale positive needs the warm run's hint at {}",
                    path.display()
                )
            })?;
            set_hint_count(&mut document, egg, 10);
            write_hint(path, &document)?;
            Ok((before, Some(document)))
        }
        Variant::StaleNegative => {
            let mut document = before.clone().ok_or_else(|| {
                format!(
                    "stale negative needs the earlier runs' hint at {}",
                    path.display()
                )
            })?;
            set_hint_count(&mut document, egg, 0);
            write_hint(path, &document)?;
            Ok((before, Some(document)))
        }
    }
}

// ---- the cell ------------------------------------------------------------

fn build_cell(variant: Variant) -> Result<(Cell, StartFamily, [i32; 3]), String> {
    let selected = api::game_data::for_revision(ClientRevision::R289)
        .map_err(|error| format!("selected R289 game data: {error}"))?;
    let quests = Arc::new(
        QuestCatalog::from_identity(selected.quest_identity())
            .expect("selected R289 quest catalog"),
    );
    let document: PathDocument = serde_json::from_str(COOK_PATH_JSON)
        .map_err(|error| format!("decode bundled cook.json: {error}"))?;
    if document.id.0.as_ref() != "cook" {
        return Err(format!(
            "bundled Cook Path identity changed: {}",
            document.id.0
        ));
    }
    let ids = [
        item_id(&selected, "egg")?,
        item_id(&selected, "bucket_milk")?,
        item_id(&selected, "pot_flour")?,
    ];
    let name: &'static str = match variant {
        Variant::Cold => "live-quester-hint-cold",
        Variant::Warm => "live-quester-hint-warm",
        Variant::StalePositive => "live-quester-hint-stale-positive",
        Variant::StaleNegative => "live-quester-hint-stale-negative",
    };
    let mut scenario =
        scenario::quester_stage(name, "Cook's Assistant", "cookquest", 0, &[], COOK_START);
    scenario.settings.deadline = RUN_DEADLINE;
    let relog_index = scenario
        .steps
        .iter()
        .position(|step| step.name == "relog so the quest tab colour matches the seeded varp")
        .ok_or("Cook fixture has no post-seed relog")?;
    let seeds: Vec<Step> = variant
        .bank_seeds()
        .iter()
        .map(|command| {
            perform_command(
                "seed the bank before the fixture relog",
                (*command).to_owned(),
                Proof::SideTabAvailable { index: 3 },
            )
        })
        .collect();
    scenario.steps.splice(relog_index..relog_index, seeds);
    let start = production_start(Arc::clone(&selected), quests);
    let seed_commands: Vec<&str> = variant.bank_seeds().to_vec();
    let observe_ids = ids;
    let observe_start = Box::new(move |snapshot: &GameSnapshot| {
        if !snapshot.ingame() || snapshot.scene_state() != 2 {
            return Err("Start fixture is not ingame with scene_state == 2".to_owned());
        }
        if snapshot.bank_component_id() >= 0 {
            return Err("Start fixture must have the bank closed".to_owned());
        }
        let held = item_counts(snapshot.inventory(), &observe_ids);
        if held != [0, 0, 0] {
            return Err(format!("Start fixture holds Cook ingredients: {held:?}"));
        }
        Ok(json!({
            "variant": variant.name(),
            "ingredient_ids": observe_ids,
            "bank_seeds": seed_commands,
            "tile": [COOK_START.x, COOK_START.z, COOK_START.level],
        }))
    });
    Ok((
        Cell {
            quest: "cook",
            display: "Cook's Assistant",
            label: name.to_owned(),
            scenario,
            start_settings: serde_json::Map::from_iter([("quests".into(), json!(["cook"]))]),
            mode: Mode::Clean,
            observe_start: Some(observe_start),
        },
        start,
        ids,
    ))
}

fn production_start(selected: Arc<SelectedGameData>, quests: Arc<QuestCatalog>) -> StartFamily {
    Box::new(move |handle, account, banks| {
        let path = api::selected::FamilyPreparation::run({
            let selected = Arc::clone(&selected);
            let quests = Arc::clone(&quests);
            move |cap| compile_path(COOK_PATH_JSON.as_bytes(), &selected, &quests, cap)
        })
        .map_err(|error| format!("prepare bundled Cook Path: {error}"))?
        .join()
        .map_err(|_| "bundled Cook Path compilation panicked".to_owned())?
        .map_err(|error| format!("compile bundled Cook Path: {error:?}"))?;
        let script = Quester::new(
            RunKey {
                slot: 0,
                run: 0,
                session: 0,
            },
            path,
            Arc::clone(&selected),
            Arc::clone(&quests),
            Arc::clone(banks),
        );
        handle
            .start_test_script(account, Box::new(script), Some(Arc::clone(&selected)))
            .map(|_| ())
            .map_err(|error| format!("start native Quester on bundled Cook Path: {error:?}"))
    })
}

/// One open bank session as the frames showed it.
#[derive(Debug, Clone, Serialize)]
struct BankSession {
    generation: u64,
    stage_at_open: Option<String>,
    step_at_open: Option<String>,
    loaded_seen: bool,
    first_loaded_counts: Option<[i32; 3]>,
    last_loaded_counts: Option<[i32; 3]>,
}

struct Witness {
    ids: [i32; 3],
    completed_seen: bool,
    seen_generations: HashSet<u64>,
    active_generation: Option<u64>,
    sessions: Vec<BankSession>,
    all_ingredients_held: bool,
    observations: u64,
}

impl Witness {
    fn observe(
        &mut self,
        snapshot: &GameSnapshot,
        status: Option<&ScriptStatus>,
        lifecycle: Option<&script::ScriptLifecycleReceipt>,
    ) -> Result<Option<Value>, String> {
        self.observations = self.observations.saturating_add(1);
        let green = quest_complete(snapshot);
        if green {
            self.completed_seen = true;
        }
        if !self.completed_seen
            && item_counts(snapshot.inventory(), &self.ids)
                .iter()
                .all(|count| *count >= 1)
        {
            self.all_ingredients_held = true;
        }
        if snapshot.bank_component_id() < 0 {
            self.active_generation = None;
        } else {
            let generation = snapshot.bank_session_generation();
            if self.active_generation != Some(generation) {
                if self.completed_seen {
                    return Err(format!(
                        "bank-open generation {generation} began after Cook completion"
                    ));
                }
                if !self.seen_generations.insert(generation) {
                    return Err(format!(
                        "bank session generation {generation} reopened without a new generation"
                    ));
                }
                self.sessions.push(BankSession {
                    generation,
                    stage_at_open: status_text(status, "stage").map(str::to_owned),
                    step_at_open: status_text(status, "step_id").map(str::to_owned),
                    loaded_seen: false,
                    first_loaded_counts: None,
                    last_loaded_counts: None,
                });
                self.active_generation = Some(generation);
            }
            if snapshot.bank_loaded() {
                let counts = item_counts(snapshot.bank(), &self.ids);
                let session = self.sessions.last_mut().expect("an open session");
                session.loaded_seen = true;
                session.first_loaded_counts.get_or_insert(counts);
                session.last_loaded_counts = Some(counts);
            }
        }
        let completed = lifecycle
            .is_some_and(|receipt| receipt.state == script::ScriptTerminalState::Completed);
        if !green || !completed {
            return Ok(None);
        }
        if !self.all_ingredients_held {
            return Err("the three ingredients were never held together before completion".into());
        }
        Ok(Some(json!({
            "quest_complete": true,
            "bank_open_generations": self.seen_generations.len(),
            "bank_sessions": self.sessions,
            "observations": self.observations,
        })))
    }
}

fn make_observer(ids: [i32; 3]) -> ObserveFamily {
    let mut witness = Witness {
        ids,
        completed_seen: false,
        seen_generations: HashSet::new(),
        active_generation: None,
        sessions: Vec::new(),
        all_ingredients_held: false,
        observations: 0,
    };
    Box::new(move |snapshot, status, lifecycle| witness.observe(snapshot, status, lifecycle))
}

fn account_lines(lines: &[SessionLogLine], account: &str) -> Vec<SessionLogLine> {
    lines
        .iter()
        .filter(|line| line.slot.as_deref() == Some(account))
        .cloned()
        .collect()
}

fn write_json(path: &Path, value: &Value) -> Result<(), String> {
    std::fs::write(
        path,
        serde_json::to_vec_pretty(value).map_err(|error| error.to_string())?,
    )
    .map_err(|error| format!("write {}: {error}", path.display()))
}

#[test]
#[ignore = "requires LIVE=1, the shared local 289 engine, a fixed QUESTER_LIVE_ACCOUNT and a HOME kept across the four variants; see the module doc"]
fn live_quester_hint() {
    assert_eq!(std::env::var("LIVE").as_deref(), Ok("1"), "requires LIVE=1");
    assert_eq!(
        std::env::var("QUESTER_LIVE_KEEP_HOME").as_deref(),
        Ok("1"),
        "the hint must persist across processes: QUESTER_LIVE_KEEP_HOME=1"
    );
    let variant = Variant::from_env().unwrap_or_else(|error| panic!("{error}"));
    let account = std::env::var("QUESTER_LIVE_ACCOUNT").expect("QUESTER_LIVE_ACCOUNT");
    let root = PathBuf::from(std::env::var_os("LIVE_EVIDENCE_DIR").expect("LIVE_EVIDENCE_DIR"));
    let evidence = root.join("cook").join(format!(
        "hint-{}-{}-{}",
        variant.name(),
        account,
        unix_now()
    ));
    std::fs::create_dir_all(&evidence).expect("create evidence directory");
    assert!(
        *SESSION_LOG_SINK_INSTALLED,
        "could not install the session-log sink; another sink owns this process"
    );
    SESSION_LOGS
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .clear();

    let (cell, start, ids) =
        build_cell(variant).unwrap_or_else(|error| panic!("{} fixture: {error}", variant.name()));
    let path = hint_path(&account).unwrap_or_else(|error| panic!("{error}"));
    let (hint_before, hint_prepared) = prepare_hint(variant, &path, ids[0])
        .unwrap_or_else(|error| panic!("{} hint preparation: {error}", variant.name()));
    write_json(
        &evidence.join("hint-before.json"),
        &json!({
            "path": path.display().to_string(),
            "found": hint_before,
            "launched_with": hint_prepared,
        }),
    )
    .unwrap();

    let started = unix_now();
    let result = quester_live::run_family(cell, start, make_observer(ids));
    let logs = SESSION_LOGS
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .clone();
    let lines = account_lines(&logs, &account);
    let mut text = String::new();
    for line in &lines {
        use std::fmt::Write as _;
        let _ = writeln!(
            text,
            "[tick={}] [{}:{}] {}",
            line.tick
                .map_or_else(|| "?".to_owned(), |tick| tick.to_string()),
            line.source,
            line.level,
            line.message
        );
    }
    std::fs::write(evidence.join("session.log"), text).expect("write session.log");
    let hint_after = path.is_file().then(|| read_hint(&path)).transpose();
    write_json(
        &evidence.join("hint-after.json"),
        &json!({
            "path": path.display().to_string(),
            "document": hint_after.as_ref().ok().cloned().flatten(),
            "error": hint_after.as_ref().err(),
        }),
    )
    .unwrap();

    let scan_lines = lines
        .iter()
        .filter(|line| line.message == SCAN_BEGIN)
        .count();
    let withdraw_lines = lines
        .iter()
        .filter(|line| line.message == WITHDRAW_BEGIN)
        .count();
    let egg_acquisitions = lines
        .iter()
        .filter(|line| line.message.contains(EGG_ACQUISITION))
        .count();
    let parks = lines
        .iter()
        .filter(|line| line.message.contains("quester cook: park:"))
        .count();
    let settle_timeouts = lines
        .iter()
        .filter(|line| line.message.contains("step settle timeout"))
        .count();
    let verdict = json!({
        "variant": variant.name(),
        "account": account,
        "started_unix": started,
        "finished_unix": unix_now(),
        "result": match &result {
            Ok(receipt) => json!({"passed": true, "receipt": receipt}),
            Err(error) => json!({"passed": false, "error": error}),
        },
        "scan_begin_lines": scan_lines,
        "withdraw_begin_lines": withdraw_lines,
        "egg_acquisition_lines": egg_acquisitions,
        "park_lines": parks,
        "settle_timeout_lines": settle_timeouts,
        "expected": {
            "scan_begin_lines": variant.expected_scan_lines(),
            "withdraw_begin_lines": [variant.expected_withdraw_lines().start(), variant.expected_withdraw_lines().end()],
            "egg_acquisition": variant.expects_egg_acquisition(),
            "hint_after": variant.expected_hint_after(),
        },
    });
    write_json(&evidence.join("verdict.json"), &verdict).unwrap();

    let receipt = result.unwrap_or_else(|error| {
        panic!(
            "FAIL L-QUESTER-HINT {}: {error}; evidence={}",
            variant.name(),
            evidence.display()
        )
    });
    assert_eq!(receipt["starts"].as_u64(), Some(1), "one explicit Start");
    assert_eq!(receipt["deaths"].as_u64(), Some(0), "zero deaths");
    assert_eq!(receipt["family_receipt"]["quest_complete"], json!(true));
    assert_eq!(
        receipt["family_receipt"]["bank_open_generations"],
        json!(1),
        "{}: exactly one bank trip; evidence={}",
        variant.name(),
        evidence.display()
    );
    assert_eq!(
        scan_lines,
        variant.expected_scan_lines(),
        "{}: scan lines; evidence={}",
        variant.name(),
        evidence.display()
    );
    assert!(
        variant.expected_withdraw_lines().contains(&withdraw_lines),
        "{}: {withdraw_lines} withdraw lines; evidence={}",
        variant.name(),
        evidence.display()
    );
    assert_eq!(
        egg_acquisitions > 0,
        variant.expects_egg_acquisition(),
        "{}: egg acquisition lines {egg_acquisitions}; evidence={}",
        variant.name(),
        evidence.display()
    );
    assert_eq!(parks, 0, "{}: no park", variant.name());
    if variant != Variant::Cold || scan_lines == 1 {
        // The scan precedes the withdraw: the trip observed, then planned.
        let first_scan = lines.iter().position(|line| line.message == SCAN_BEGIN);
        let first_withdraw = lines.iter().position(|line| line.message == WITHDRAW_BEGIN);
        if let (Some(scan), Some(withdraw)) = (first_scan, first_withdraw) {
            assert!(scan < withdraw, "{}: the scan comes first", variant.name());
        }
    }
    let hint_after = hint_after
        .unwrap_or_else(|error| panic!("{}: hint after the run: {error}", variant.name()))
        .unwrap_or_else(|| panic!("{}: no hint saved at {}", variant.name(), path.display()));
    assert_eq!(hint_after.profile, PROFILE);
    assert_eq!(
        hint_after.account,
        script::bank_hints::account_component(&account).unwrap()
    );
    assert!(
        hint_after.observed_at_unix >= started,
        "{}: the hint was saved by this run ({} >= {started})",
        variant.name(),
        hint_after.observed_at_unix
    );
    let after = [
        hint_count(&hint_after, ids[0]),
        hint_count(&hint_after, ids[1]),
        hint_count(&hint_after, ids[2]),
    ];
    assert_eq!(
        after,
        variant.expected_hint_after(),
        "{}: the saved hint rows [egg, milk, flour]; evidence={}",
        variant.name(),
        evidence.display()
    );
    println!(
        "PASS L-QUESTER-HINT {} scan={scan_lines} withdraw={withdraw_lines} egg_acquisition={egg_acquisitions} hint_after={after:?} evidence={}",
        variant.name(),
        evidence.display()
    );
}
