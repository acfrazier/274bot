use super::{FeatureCase, Observation};
use crate::Play;
use api::prereq_knowledge::{Fact, FactSource, SessionMemory};
use api::prereq_steps::PrereqCatalog;
use api::selected::{FactKey, InclusiveRange, QuestGate, RunKey, StageWindow};
use api::snapshot::WorldTile;
use scenario::{Proof, Scenario, ScenarioSettings, StepKind};
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::str::FromStr;
use std::time::Duration;

const PRIEST_SCENARIO: &str = "priest_journal";
const OBJECT_SCENARIO: &str = "compatibility_object_cache";
const PRIEST_START: WorldTile = WorldTile {
    x: 3415,
    z: 3485,
    level: 0,
};
const PRIEST_PAGE_ID: &str = "priestperil:mausoleum-progress";
const OBJECT_RECEIPT_PREFIX: &str = "w6-compat-object-cache|";
const FAST_PROPERTIES_PROOF_TEST: &str =
    "materialize::bank_locations_v8::tests::bank_result_shapes_cover_callbacks_and_bound_v8_heap";
const OBJECT_SOURCE: &str = include_str!("compat_object_cache.js");
const GAMEPLAY_RECENT_LIMIT: Duration = Duration::from_secs(30);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    Off,
    Active,
}

impl Mode {
    fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Active => "active",
        }
    }

    fn parse(value: &str) -> Result<Self, String> {
        match value {
            "off" => Ok(Self::Off),
            "active" => Ok(Self::Active),
            other => Err(format!("unknown compatibility object-cache mode {other:?}")),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum StartProgress {
    #[default]
    NotStarted,
    Pending,
    Ready,
}

fn validate_names(names: &[String]) -> Result<(), String> {
    if names.is_empty() {
        return Err("memory feature case requires at least one accepted slot".into());
    }
    for (index, name) in names.iter().enumerate() {
        if name.trim().is_empty() {
            return Err(format!("memory feature case has an empty slot name at index {index}"));
        }
        if names[..index].iter().any(|earlier| earlier == name) {
            return Err(format!("memory feature case has duplicate slot name {name:?}"));
        }
    }
    Ok(())
}

fn validate_names_match(expected: &[String], actual: &[String]) -> Result<(), String> {
    if expected != actual {
        return Err("memory feature case slot set changed after construction".into());
    }
    Ok(())
}

fn truncate_before_start(mut scenario: Scenario, name: &'static str, proof: Proof) -> Result<Scenario, String> {
    let Some(start) = scenario
        .steps
        .iter()
        .position(|step| matches!(&step.kind, StepKind::StartScript))
    else {
        return Err(format!("{name} seed has no existing StartScript boundary"));
    };
    scenario.steps.truncate(start);
    scenario.name = name;
    scenario.proof = proof;
    scenario.settings = ScenarioSettings {
        full_rate: true,
        require_mainland_base: true,
        ..ScenarioSettings::default()
    };
    Ok(scenario)
}

fn object_seed() -> Result<Scenario, String> {
    let scenario = scenario::thiever_sustained_scenario();
    let proof = scenario
        .steps
        .iter()
        .find(|step| step.name == "seed stats, food, and tele to the guard stand")
        .map(|step| step.wait.arm)
        .ok_or_else(|| "existing sustained Thiever seed lost its guard-arrival proof".to_string())?;
    truncate_before_start(scenario, OBJECT_SCENARIO, proof)
}

fn priest_seed() -> Result<Scenario, String> {
    let scenario = scenario::quester_stage(
        "memory_priest_journal_seed",
        "Priest in Peril",
        "priestperil",
        60,
        &[],
        PRIEST_START,
    );
    truncate_before_start(
        scenario,
        PRIEST_SCENARIO,
        Proof::Arrived {
            x: PRIEST_START.x,
            z: PRIEST_START.z,
            level: PRIEST_START.level,
        },
    )
}

fn settings_bag(mode: Mode) -> Map<String, Value> {
    let mut bag = Map::new();
    bag.insert("mode".into(), Value::String(mode.as_str().into()));
    bag
}

fn advance_load_start(
    play: &Play,
    name: &str,
    progress: &mut StartProgress,
    mode: Mode,
) -> Result<bool, String> {
    if *progress == StartProgress::Ready {
        return Ok(true);
    }
    if *progress == StartProgress::NotStarted {
        match play.script_start_load_typed(
            name,
            OBJECT_SOURCE.to_owned(),
            script_js::LoadShape::CompatClass,
            Some(settings_bag(mode)),
            Vec::new(),
        ) {
            Ok(()) => *progress = StartProgress::Pending,
            Err(script::StartLoadError::Waiting(_)) => return Ok(false),
            Err(error) => return Err(format!("{name}: {error}")),
        }
    }
    match play.script_poll_start(name) {
        script::StartPoll::Pending => Ok(false),
        script::StartPoll::Settled(script::StartOutcome::Ready) => {
            *progress = StartProgress::Ready;
            Ok(true)
        }
        script::StartPoll::Settled(outcome) => Err(format!(
            "{name}: compatibility object workload Start settled as {outcome:?}"
        )),
        script::StartPoll::NotOwed => Err(format!(
            "{name}: compatibility object workload Start lost its typed poll"
        )),
    }
}

fn advance_compiled_start(
    play: &Play,
    name: &str,
    progress: &mut StartProgress,
) -> Result<bool, String> {
    if *progress == StartProgress::Ready {
        return Ok(true);
    }
    if *progress == StartProgress::NotStarted {
        match play.script_start_typed(
            name,
            script::CompiledId("Quester"),
            Map::new(),
        ) {
            Ok(()) => *progress = StartProgress::Pending,
            Err(script::StartLoadError::Waiting(_)) => return Ok(false),
            Err(error) => return Err(format!("{name}: Quester Start: {error}")),
        }
    }
    match play.script_start_handle().poll_start(name) {
        script::StartPoll::Pending => Ok(false),
        script::StartPoll::Settled(script::StartOutcome::Ready) => {
            *progress = StartProgress::Ready;
            Ok(true)
        }
        script::StartPoll::Settled(outcome) => {
            Err(format!("{name}: Quester Start settled as {outcome:?}"))
        }
        script::StartPoll::NotOwed => Err(format!(
            "{name}: Quester Start lost its typed poll"
        )),
    }
}

fn active_script(play: &Play, name: &str) -> Result<bool, String> {
    match play.script_state(name) {
        script::RunState::Running => Ok(true),
        script::RunState::Starting | script::RunState::Paused | script::RunState::Stopping => {
            Ok(false)
        }
        script::RunState::Idle => {
            if let Some(error) = play.script_last_error(name) {
                Err(format!("{name}: script ended before workload evidence: {error}"))
            } else {
                Ok(false)
            }
        }
        script::RunState::Error => Err(format!(
            "{name}: script entered Error before workload evidence: {}",
            play.script_last_error(name)
                .unwrap_or_else(|| "no retained script error".into())
        )),
    }
}

#[derive(Clone, Copy, Debug)]
struct ObjectReceipt {
    mode: Mode,
    tick: u64,
    tile: WorldTile,
    cache_reads: u64,
    reuses: u64,
    walks: u64,
    records: usize,
    listed: usize,
    by_id: usize,
    aliases: usize,
    record_id: i32,
    item_id: i32,
    alias_id: i32,
    retained_rows: usize,
    record_identity: bool,
    row_identity: bool,
    alias_identity: bool,
    plain_shape: bool,
    failed: bool,
}

fn receipt_number<T>(fields: &mut std::str::Split<'_, char>, field: &str) -> Result<T, String>
where
    T: FromStr,
    T::Err: std::fmt::Display,
{
    fields
        .next()
        .ok_or_else(|| format!("compatibility object receipt omitted {field}"))?
        .parse()
        .map_err(|error| format!("compatibility object receipt has invalid {field}: {error}"))
}

fn receipt_flag(fields: &mut std::str::Split<'_, char>, field: &str) -> Result<bool, String> {
    match receipt_number::<u8>(fields, field)? {
        0 => Ok(false),
        1 => Ok(true),
        value => Err(format!("compatibility object receipt has non-boolean {field}={value}")),
    }
}

fn parse_object_receipt(line: &str) -> Result<ObjectReceipt, String> {
    let mut fields = line.split('|');
    if fields.next() != Some("w6-compat-object-cache") {
        return Err("compatibility object receipt prefix changed".into());
    }
    let mode = Mode::parse(
        fields
            .next()
            .ok_or_else(|| "compatibility object receipt omitted mode".to_string())?,
    )?;
    let tick = receipt_number(&mut fields, "tick")?;
    let x = receipt_number(&mut fields, "world_x")?;
    let z = receipt_number(&mut fields, "world_z")?;
    let level = receipt_number(&mut fields, "world_level")?;
    let cache_reads = receipt_number(&mut fields, "cache_reads")?;
    let reuses = receipt_number(&mut fields, "reuses")?;
    let walks = receipt_number(&mut fields, "walks")?;
    let records = receipt_number(&mut fields, "record_rows")?;
    let listed = receipt_number(&mut fields, "listed_rows")?;
    let by_id = receipt_number(&mut fields, "by_id_rows")?;
    let aliases = receipt_number(&mut fields, "alias_rows")?;
    let record_id = receipt_number(&mut fields, "record_id")?;
    let item_id = receipt_number(&mut fields, "item_id")?;
    let alias_id = receipt_number(&mut fields, "alias_id")?;
    let retained_rows = receipt_number(&mut fields, "retained_rows")?;
    let record_identity = receipt_flag(&mut fields, "record_identity")?;
    let row_identity = receipt_flag(&mut fields, "row_identity")?;
    let alias_identity = receipt_flag(&mut fields, "alias_identity")?;
    let plain_shape = receipt_flag(&mut fields, "plain_shape")?;
    let failed = receipt_flag(&mut fields, "failed")?;
    if fields.next().is_some() {
        return Err("compatibility object receipt has trailing fields".into());
    }
    Ok(ObjectReceipt {
        mode,
        tick,
        tile: WorldTile { x, z, level },
        cache_reads,
        reuses,
        walks,
        records,
        listed,
        by_id,
        aliases,
        record_id,
        item_id,
        alias_id,
        retained_rows,
        record_identity,
        row_identity,
        alias_identity,
        plain_shape,
        failed,
    })
}

fn object_receipt(play: &Play, name: &str) -> Result<Option<ObjectReceipt>, String> {
    let Some(paint) = play.script_paint(name) else {
        return Ok(None);
    };
    let Some(line) = paint
        .lines
        .iter()
        .find(|line| line.starts_with(OBJECT_RECEIPT_PREFIX))
    else {
        return Ok(None);
    };
    parse_object_receipt(line).map(Some)
}

fn validate_off_receipt(receipt: ObjectReceipt, name: &str) -> Result<(), String> {
    if receipt.failed {
        return Err(format!("{name}: compatibility object control workload failed"));
    }
    if receipt.mode != Mode::Off
        || receipt.cache_reads != 0
        || receipt.reuses != 0
        || receipt.records != 0
        || receipt.listed != 0
        || receipt.by_id != 0
        || receipt.aliases != 0
        || receipt.retained_rows != 0
        || receipt.walks == 0
        || receipt.tick == 0
    {
        return Err(format!(
            "{name}: feature-off workload did not retain a live isolate without using the object caches"
        ));
    }
    Ok(())
}

fn validate_active_receipt(receipt: ObjectReceipt, name: &str) -> Result<bool, String> {
    if receipt.failed {
        return Err(format!("{name}: compatibility object-cache workload failed"));
    }
    if receipt.mode != Mode::Active {
        return Ok(false);
    }
    if receipt.tick == 0 {
        return Ok(false);
    }
    Ok(receipt.cache_reads >= 2
        && receipt.reuses > 0
        && receipt.walks > 0
        && receipt.records > 0
        && receipt.listed > 0
        && receipt.by_id > 0
        && receipt.aliases > 0
        && receipt.retained_rows > 0
        && receipt.record_id == receipt.item_id
        && receipt.record_identity
        && receipt.row_identity
        && receipt.alias_identity
        && receipt.plain_shape)
}

fn recent_gameplay(play: &Play, name: &str) -> bool {
    play.script_progress(name).is_some_and(|progress| {
        progress
            .idle_for
            .is_some_and(|idle| idle <= GAMEPLAY_RECENT_LIMIT)
    })
}

struct ObjectSlot {
    off_start: StartProgress,
    active_start: StartProgress,
    stop_requested: bool,
    receipt: Option<ObjectReceipt>,
    proved: bool,
}

impl Default for ObjectSlot {
    fn default() -> Self {
        Self {
            off_start: StartProgress::NotStarted,
            active_start: StartProgress::NotStarted,
            stop_requested: false,
            receipt: None,
            proved: false,
        }
    }
}

struct Objects {
    names: Vec<String>,
    slots: HashMap<String, ObjectSlot>,
}

impl Objects {
    fn new(names: &[String]) -> Self {
        Self {
            names: names.to_vec(),
            slots: names
                .iter()
                .cloned()
                .map(|name| (name, ObjectSlot::default()))
                .collect(),
        }
    }

    fn object_seed(&self, slot: usize, names: &[String]) -> Result<Scenario, String> {
        validate_names_match(&self.names, names)?;
        if slot >= names.len() {
            return Err(format!("object-cache seed slot {slot} is outside {} names", names.len()));
        }
        object_seed()
    }
}

impl FeatureCase for Objects {
    fn seed(&self, slot: usize, names: &[String]) -> Result<Scenario, String> {
        self.object_seed(slot, names)
    }

    fn prepare_unused(&mut self, play: &Play, names: &[String]) -> Result<bool, String> {
        validate_names_match(&self.names, names)?;
        let mut all_ready = true;
        for name in names {
            let slot = self
                .slots
                .get_mut(name)
                .ok_or_else(|| format!("object-cache case lost slot {name:?}"))?;
            if !advance_load_start(play, name, &mut slot.off_start, Mode::Off)?
                || !active_script(play, name)?
            {
                all_ready = false;
                continue;
            }
            let Some(receipt) = object_receipt(play, name)? else {
                all_ready = false;
                continue;
            };
            validate_off_receipt(receipt, name)?;
            slot.receipt = Some(receipt);
        }
        Ok(all_ready)
    }

    fn activate(&mut self, play: &Play, names: &[String]) -> Result<bool, String> {
        validate_names_match(&self.names, names)?;
        let mut all_ready = true;
        for name in names {
            let slot = self
                .slots
                .get_mut(name)
                .ok_or_else(|| format!("object-cache case lost slot {name:?}"))?;
            if slot.active_start == StartProgress::NotStarted && !slot.stop_requested {
                if play.script_state(name) != script::RunState::Idle {
                    play.script_stop(name);
                    slot.stop_requested = true;
                    all_ready = false;
                    continue;
                }
                slot.stop_requested = true;
            }
            if play.script_state(name) != script::RunState::Idle
                && slot.active_start == StartProgress::NotStarted
            {
                all_ready = false;
                continue;
            }
            if !advance_load_start(play, name, &mut slot.active_start, Mode::Active)?
                || !active_script(play, name)?
            {
                all_ready = false;
                continue;
            }
            let Some(receipt) = object_receipt(play, name)? else {
                all_ready = false;
                continue;
            };
            if receipt.mode != Mode::Active {
                all_ready = false;
                continue;
            }
            if receipt.failed {
                return Err(format!("{name}: compatibility object-cache workload failed"));
            }
            slot.proved = validate_active_receipt(receipt, name)?;
            slot.receipt = Some(receipt);
            if receipt.cache_reads == 0 {
                all_ready = false;
            }
        }
        Ok(all_ready)
    }

    fn observe(&mut self, play: &Play, names: &[String]) -> Result<Observation, String> {
        validate_names_match(&self.names, names)?;
        let mut observation = Observation {
            ready: 0,
            active: 0,
            proved: 0,
            continuing: 0,
        };
        for name in names {
            let slot = self
                .slots
                .get_mut(name)
                .ok_or_else(|| format!("object-cache case lost slot {name:?}"))?;
            let running = active_script(play, name)?;
            if running {
                observation.active += 1;
            }
            let expected_mode = if slot.active_start == StartProgress::Ready {
                Mode::Active
            } else {
                Mode::Off
            };
            if let Some(receipt) = object_receipt(play, name)? {
                if receipt.failed {
                    return Err(format!("{name}: compatibility object workload reported failure"));
                }
                if receipt.mode == expected_mode && receipt.tick > 0 {
                    if expected_mode == Mode::Off {
                        validate_off_receipt(receipt, name)?;
                    } else {
                        slot.proved = validate_active_receipt(receipt, name)?;
                    }
                    slot.receipt = Some(receipt);
                }
            }
            let ready = running
                && slot.receipt.is_some_and(|receipt| {
                    receipt.mode == expected_mode
                        && receipt.tick > 0
                        && (expected_mode == Mode::Off || receipt.cache_reads > 0)
                });
            if ready {
                observation.ready += 1;
            }
            if slot.proved {
                observation.proved += 1;
            }
            if ready
                && slot
                    .receipt
                    .is_some_and(|receipt| receipt.walks > 0)
                && recent_gameplay(play, name)
            {
                observation.continuing += 1;
            }
        }
        Ok(observation)
    }

    fn stop(&mut self, play: &Play, names: &[String]) -> Result<(), String> {
        validate_names_match(&self.names, names)?;
        for name in names {
            if play.script_state(name) != script::RunState::Idle {
                play.script_stop(name);
            }
        }
        Ok(())
    }

    fn evidence(&self, names: &[String]) -> Value {
        let slots = names
            .iter()
            .map(|name| {
                let state = self.slots.get(name);
                let receipt = state.and_then(|state| state.receipt);
                json!({
                    "name": name,
                    "off_start": state.map(|state| start_progress_name(state.off_start)),
                    "active_start": state.map(|state| start_progress_name(state.active_start)),
                    "proved": state.is_some_and(|state| state.proved),
                    "receipt": receipt.map(object_receipt_json),
                })
            })
            .collect::<Vec<_>>();
        json!({
            "scenario": OBJECT_SCENARIO,
            "content_source": "reader.objCatalog() and liveCatalog() selected-revision object facts",
            "retained_references": ["obj-records array and row", "obj-catalog object", "items row", "byId map and shared row", "aliases map and alias row"],
            "fast_property_claim": "not queried live; only the plain Object.prototype shape is reported above",
            "internal_fast_mode_proof_test": FAST_PROPERTIES_PROOF_TEST,
            "slots": slots,
        })
    }
}

fn start_progress_name(progress: StartProgress) -> &'static str {
    match progress {
        StartProgress::NotStarted => "not_started",
        StartProgress::Pending => "pending",
        StartProgress::Ready => "ready",
    }
}

fn object_receipt_json(receipt: ObjectReceipt) -> Value {
    json!({
        "mode": receipt.mode.as_str(),
        "tick": receipt.tick,
        "world_tile": {"x": receipt.tile.x, "z": receipt.tile.z, "level": receipt.tile.level},
        "cache_reads": receipt.cache_reads,
        "cache_reuses": receipt.reuses,
        "walks": receipt.walks,
        "record_rows": receipt.records,
        "listed_rows": receipt.listed,
        "by_id_rows": receipt.by_id,
        "alias_rows": receipt.aliases,
        "retained_rows": receipt.retained_rows,
        "obj_record_id": receipt.record_id,
        "catalog_item_id": receipt.item_id,
        "alias_id": receipt.alias_id,
        "record_array_identity": receipt.record_identity,
        "shared_items_by_id_identity": receipt.row_identity,
        "alias_identity": receipt.alias_identity,
        "plain_object_shape": receipt.plain_shape,
        "failed": receipt.failed,
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PriestPhase {
    New,
    ControlStarting,
    Control,
    ControlStopping,
    QuesterStarting,
    Quester,
    QuesterStopping,
    WalkerStarting,
    Walker,
}

impl PriestPhase {
    fn as_str(self) -> &'static str {
        match self {
            Self::New => "new",
            Self::ControlStarting => "control_starting",
            Self::Control => "control",
            Self::ControlStopping => "control_stopping",
            Self::QuesterStarting => "quester_starting",
            Self::Quester => "quester",
            Self::QuesterStopping => "quester_stopping",
            Self::WalkerStarting => "walker_starting",
            Self::Walker => "walker",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct PriestFact {
    run: RunKey,
    tick: u64,
    sequence: u64,
    row: u16,
    line: u16,
    possible_min: i32,
    possible_max: i32,
    after_min: i32,
    after_max: i32,
    lower_five: bool,
    lower_eight: bool,
    exact_barrier: bool,
}

struct PriestSlot {
    phase: PriestPhase,
    control_start: StartProgress,
    quester_start: StartProgress,
    walker_start: StartProgress,
    quester_run: Option<RunKey>,
    stop_requested: bool,
    baseline_checked: bool,
    fact: Option<PriestFact>,
    retained_after_switch: bool,
    receipt: Option<ObjectReceipt>,
}

impl Default for PriestSlot {
    fn default() -> Self {
        Self {
            phase: PriestPhase::New,
            control_start: StartProgress::NotStarted,
            quester_start: StartProgress::NotStarted,
            walker_start: StartProgress::NotStarted,
            quester_run: None,
            stop_requested: false,
            baseline_checked: false,
            fact: None,
            retained_after_switch: false,
            receipt: None,
        }
    }
}

struct Priest {
    names: Vec<String>,
    slots: HashMap<String, PriestSlot>,
}

impl Priest {
    fn new(names: &[String]) -> Self {
        Self {
            names: names.to_vec(),
            slots: names
                .iter()
                .cloned()
                .map(|name| (name, PriestSlot::default()))
                .collect(),
        }
    }

    fn priest_seed(&self, slot: usize, names: &[String]) -> Result<Scenario, String> {
        validate_names_match(&self.names, names)?;
        if slot >= names.len() {
            return Err(format!(
                "Priest journal seed slot {slot} is outside {} names",
                names.len()
            ));
        }
        priest_seed()
    }
}

fn slot_prereq_snapshot(play: &Play, name: &str) -> Result<SessionMemory, String> {
    let slot = crate::script_runtime::script_slot(&play.scripts, name)
        .ok_or_else(|| {
            format!("{name}: no accepted script slot for session prerequisite memory")
        })?;
    let state = {
        let guard = slot
            .lock()
            .map_err(|_| {
                format!("{name}: script slot lock poisoned while reading prerequisite state")
            })?;
        guard
            .local_state()
            .cloned()
            .ok_or_else(|| format!("{name}: script slot has no local session-state capability"))?
    };
    Ok(state.prereq_snapshot())
}

fn priest_fact_in(memory: &SessionMemory) -> Option<&Fact> {
    memory.facts().iter().find(|fact| {
        fact.quest.0.as_ref() == "priestperil" && fact.signal.0.as_ref() == "priestperil"
    })
}

fn gate_window(quest: &FactKey, signal: &FactKey, min: i32, max: i32) -> QuestGate {
    QuestGate::Window(StageWindow {
        quest: quest.clone(),
        signal: signal.clone(),
        values: InclusiveRange {
            min: Some(min),
            max: Some(max),
        },
    })
}

fn journal_page_fact(
    fact: &Fact,
    run: RunKey,
    catalog: &PrereqCatalog,
) -> Result<PriestFact, String> {
    if fact.quest.0.as_ref() != "priestperil" || fact.signal.0.as_ref() != "priestperil" {
        return Err("session fact is not the Priest in Peril progress signal".into());
    }
    if fact.observed.run != run || fact.observed.tick == 0 || fact.observed.sequence == 0 {
        return Err(format!(
            "Priest journal fact is not ordered in the current Quester run: observed={:?}, current={run:?}",
            fact.observed
        ));
    }
    let (row, line) = match &fact.source {
        FactSource::Page { row, line } => (*row, *line),
        other => {
            return Err(format!(
                "Priest progress fact came from {other:?}, not an ordinary Page"
            ));
        }
    };
    let gate_five = gate_window(&fact.quest, &fact.signal, 5, 61);
    let gate_eight = gate_window(&fact.quest, &fact.signal, 8, 61);
    let gate_exact = gate_window(&fact.quest, &fact.signal, 61, 61);
    let (catalog_row, journal) = catalog
        .for_journal(&gate_five)
        .ok_or_else(|| {
            "selected prerequisite catalog has no Priest journal for the ladder window".to_string()
        })?;
    let established = StageWindow {
        quest: fact.quest.clone(),
        signal: fact.signal.clone(),
        values: fact.possible,
    };
    let lower_five = api::prereq_steps::proves(&established, &gate_five);
    let lower_eight = api::prereq_steps::proves(&established, &gate_eight);
    let exact_barrier = api::prereq_steps::proves(&established, &gate_exact);
    if journal.id.as_ref() != PRIEST_PAGE_ID
        || row != catalog_row
        || fact.possible.min != Some(9)
        || fact.possible.max != Some(61)
        || fact.after.min != Some(9)
        || fact.after.max != Some(61)
        || !journal.establishes.values.eq(&fact.possible)
        || !journal
            .rules
            .get(line as usize)
            .is_some_and(|rule| rule.proves == fact.possible)
        || !journal
            .after
            .iter()
            .any(|rule| rule.proves == fact.possible && rule.after == fact.after)
        || fact.refused.is_some()
        || !lower_five
        || !lower_eight
        || exact_barrier
    {
        return Err(format!(
            "observed Priest Page did not prove the reviewed [9,61] interval semantics: row={row}, line={line}, interval={:?}, after={:?}, ladders=({lower_five},{lower_eight}), exact={exact_barrier}",
            fact.possible, fact.after
        ));
    }
    Ok(PriestFact {
        run,
        tick: fact.observed.tick,
        sequence: fact.observed.sequence,
        row,
        line,
        possible_min: fact.possible.min.expect("checked interval lower bound"),
        possible_max: fact.possible.max.expect("checked interval upper bound"),
        after_min: fact.after.min.expect("checked preservation lower bound"),
        after_max: fact.after.max.expect("checked preservation upper bound"),
        lower_five,
        lower_eight,
        exact_barrier,
    })
}

fn session_priest_fact(
    play: &Play,
    name: &str,
    run: RunKey,
) -> Result<Option<PriestFact>, String> {
    let data = play
        .game_data()
        .ok_or_else(|| "selected game data unavailable for Priest journal proof".to_string())?;
    let catalog = data
        .try_prereq_steps()
        .ok_or_else(|| "selected data has no prepared prerequisite catalog".to_string())?;
    let memory = slot_prereq_snapshot(play, name)?;
    let Some(fact) = priest_fact_in(&memory) else {
        return Ok(None);
    };
    journal_page_fact(fact, run, &catalog).map(Some)
}

fn status_text<'a>(status: &'a script::native::ScriptStatus, key: &str) -> Option<&'a str> {
    status.fields.iter().find_map(|field| {
        (field.key == key).then_some(&field.value).and_then(|value| match value {
            script::native::StatusValue::Text(value) => Some(value.as_ref()),
            _ => None,
        })
    })
}

fn terminal_quester_status(play: &Play, name: &str) -> bool {
    play.script_native_status(name).is_some_and(|status| {
        matches!(
            status.phase,
            script::native::NativePhase::Blocked | script::native::NativePhase::Complete
        )
    }) || (play.script_state(name) == script::RunState::Idle
        && play.script_lifecycle_receipt(name).is_some())
}

impl FeatureCase for Priest {
    fn seed(&self, slot: usize, names: &[String]) -> Result<Scenario, String> {
        self.priest_seed(slot, names)
    }

    fn prepare_unused(&mut self, play: &Play, names: &[String]) -> Result<bool, String> {
        validate_names_match(&self.names, names)?;
        let mut all_ready = true;
        for name in names {
            let slot = self
                .slots
                .get_mut(name)
                .ok_or_else(|| format!("Priest journal case lost slot {name:?}"))?;
            if slot.phase == PriestPhase::New {
                slot.phase = PriestPhase::ControlStarting;
            }
            if slot.phase != PriestPhase::ControlStarting && slot.phase != PriestPhase::Control {
                return Err(format!(
                    "{name}: Priest control setup called in phase {}",
                    slot.phase.as_str()
                ));
            }
            if !advance_load_start(play, name, &mut slot.control_start, Mode::Off)?
                || !active_script(play, name)?
            {
                all_ready = false;
                continue;
            }
            let Some(receipt) = object_receipt(play, name)? else {
                all_ready = false;
                continue;
            };
            validate_off_receipt(receipt, name)?;
            if !slot.baseline_checked {
                let memory = slot_prereq_snapshot(play, name)?;
                if priest_fact_in(&memory).is_some() {
                    return Err(format!(
                        "{name}: feature-not-used control already contains a Priest prerequisite fact"
                    ));
                }
                slot.baseline_checked = true;
            }
            slot.receipt = Some(receipt);
            slot.phase = PriestPhase::Control;
        }
        Ok(all_ready)
    }

    fn activate(&mut self, play: &Play, names: &[String]) -> Result<bool, String> {
        validate_names_match(&self.names, names)?;
        let mut all_ready = true;
        for name in names {
            let slot = self
                .slots
                .get_mut(name)
                .ok_or_else(|| format!("Priest journal case lost slot {name:?}"))?;
            match slot.phase {
                PriestPhase::Control => {
                    if !slot.stop_requested {
                        play.script_stop(name);
                        slot.stop_requested = true;
                    }
                    slot.phase = PriestPhase::ControlStopping;
                    all_ready = false;
                }
                PriestPhase::ControlStopping => {
                    if play.script_state(name) != script::RunState::Idle {
                        all_ready = false;
                        continue;
                    }
                    slot.phase = PriestPhase::QuesterStarting;
                }
                PriestPhase::QuesterStarting => {}
                PriestPhase::Quester
                | PriestPhase::QuesterStopping
                | PriestPhase::WalkerStarting
                | PriestPhase::Walker => continue,
                PriestPhase::New => {
                    return Err(format!(
                        "{name}: Priest activation preceded a ready feature-not-used control"
                    ));
                }
            }
            if slot.phase == PriestPhase::QuesterStarting {
                if !advance_compiled_start(play, name, &mut slot.quester_start)?
                    || !active_script(play, name)?
                {
                    all_ready = false;
                    continue;
                }
                let Some(run) = play.script_native_run(name) else {
                    all_ready = false;
                    continue;
                };
                slot.quester_run = Some(run);
                slot.phase = PriestPhase::Quester;
            }
        }
        Ok(all_ready)
    }

    fn observe(&mut self, play: &Play, names: &[String]) -> Result<Observation, String> {
        validate_names_match(&self.names, names)?;
        let mut observation = Observation {
            ready: 0,
            active: 0,
            proved: 0,
            continuing: 0,
        };
        for name in names {
            let slot = self
                .slots
                .get_mut(name)
                .ok_or_else(|| format!("Priest journal case lost slot {name:?}"))?;
            if slot.phase == PriestPhase::Quester
                && slot.fact.is_none()
                && terminal_quester_status(play, name)
            {
                let run = slot
                    .quester_run
                    .ok_or_else(|| format!("{name}: Priest Quester has no captured run key"))?;
                let fact = session_priest_fact(play, name, run)?.ok_or_else(|| {
                    format!(
                        "{name}: Quester ended its Priest stage without an ordinary journal Page fact"
                    )
                })?;
                slot.fact = Some(fact);
                slot.phase = PriestPhase::QuesterStopping;
                play.script_stop(name);
                slot.stop_requested = true;
            }

            if slot.phase == PriestPhase::QuesterStopping
                && play.script_state(name) == script::RunState::Idle
            {
                slot.phase = PriestPhase::WalkerStarting;
                slot.stop_requested = false;
            }
            if slot.phase == PriestPhase::WalkerStarting
                && advance_load_start(play, name, &mut slot.walker_start, Mode::Off)?
                && active_script(play, name)?
            {
                slot.phase = PriestPhase::Walker;
            }

            let running = active_script(play, name)?;
            if running {
                observation.active += 1;
            }
            if slot.phase == PriestPhase::Quester && running {
                observation.ready += 1;
                let status = play.script_native_status(name);
                let is_working = status.as_ref().is_some_and(|status| {
                    matches!(
                        status_text(status, "action_state"),
                        Some("walking" | "talking" | "fighting" | "banking" | "working")
                    )
                });
                if is_working && recent_gameplay(play, name) {
                    observation.continuing += 1;
                }
            } else if slot.phase == PriestPhase::Control || slot.phase == PriestPhase::Walker {
                if let Some(receipt) = object_receipt(play, name)? {
                    if receipt.mode == Mode::Off && receipt.tick > 0 {
                        validate_off_receipt(receipt, name)?;
                        slot.receipt = Some(receipt);
                    }
                }
                if running
                    && slot.receipt.is_some_and(|receipt| receipt.mode == Mode::Off)
                {
                    observation.ready += 1;
                    if slot.receipt.is_some_and(|receipt| receipt.walks > 0)
                        && recent_gameplay(play, name)
                    {
                        observation.continuing += 1;
                    }
                }
            }
            if slot.phase == PriestPhase::Walker
                && slot.fact.is_some()
                && !slot.retained_after_switch
            {
                let run = slot
                    .quester_run
                    .ok_or_else(|| {
                        format!("{name}: retained Priest fact has no captured run key")
                    })?;
                let current = session_priest_fact(play, name, run)?.ok_or_else(|| {
                    format!("{name}: Priest session Page fact was not retained after Quester Stop")
                })?;
                if slot.fact.as_ref() != Some(&current) {
                    return Err(format!(
                        "{name}: Priest session Page fact changed across script Stop"
                    ));
                }
                slot.retained_after_switch = true;
            }
            if slot.fact.is_some() && slot.retained_after_switch {
                observation.proved += 1;
            }
        }
        Ok(observation)
    }

    fn stop(&mut self, play: &Play, names: &[String]) -> Result<(), String> {
        validate_names_match(&self.names, names)?;
        for name in names {
            if play.script_state(name) != script::RunState::Idle {
                play.script_stop(name);
            }
        }
        Ok(())
    }

    fn evidence(&self, names: &[String]) -> Value {
        let slots = names
            .iter()
            .map(|name| {
                let state = self.slots.get(name);
                let fact = state.and_then(|state| state.fact.as_ref());
                let receipt = state.and_then(|state| state.receipt);
                json!({
                    "name": name,
                    "phase": state.map(|state| state.phase.as_str()),
                    "control_start": state.map(|state| start_progress_name(state.control_start)),
                    "quester_start": state.map(|state| start_progress_name(state.quester_start)),
                    "walker_start": state.map(|state| start_progress_name(state.walker_start)),
                    "feature_fact_retained_after_script_switch": state
                        .is_some_and(|state| state.retained_after_switch),
                    "page_fact": fact.map(priest_fact_json),
                    "gameplay_receipt": receipt.map(object_receipt_json),
                })
            })
            .collect::<Vec<_>>();
        json!({
            "scenario": PRIEST_SCENARIO,
            "page_fact_owner": "script::local_state::SlotState::prereq_snapshot",
            "page_source": "api::prereq_knowledge::FactSource::Page from the ordinary Quester journal reader",
            "required_interval": {"min": 9, "max": 61},
            "ladder_windows_proved": [{"min": 5, "max": 61}, {"min": 8, "max": 61}],
            "exact_barrier_window_proved": false,
            "internal_fast_mode_proof_test": FAST_PROPERTIES_PROOF_TEST,
            "slots": slots,
        })
    }
}

fn priest_fact_json(fact: &PriestFact) -> Value {
    json!({
        "source": "Page",
        "catalog_row": fact.row,
        "journal_rule_line": fact.line,
        "quest_run": {"slot": fact.run.slot, "run": fact.run.run, "session": fact.run.session},
        "observed_tick": fact.tick,
        "observed_sequence": fact.sequence,
        "possible": {"min": fact.possible_min, "max": fact.possible_max},
        "after": {"min": fact.after_min, "max": fact.after_max},
        "proves_ladder_5_61": fact.lower_five,
        "proves_ladder_8_61": fact.lower_eight,
        "proves_exact_61_61": fact.exact_barrier,
    })
}

pub(super) fn priest(names: &[String]) -> Result<Box<dyn FeatureCase>, String> {
    validate_names(names)?;
    Ok(Box::new(Priest::new(names)))
}

pub(super) fn objects(names: &[String]) -> Result<Box<dyn FeatureCase>, String> {
    validate_names(names)?;
    Ok(Box::new(Objects::new(names)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn priest_journal_interval_proves_ladders_but_not_the_exact_barrier() {
        let quest = FactKey::new("priestperil");
        let established = StageWindow {
            quest: quest.clone(),
            signal: quest.clone(),
            values: InclusiveRange {
                min: Some(9),
                max: Some(61),
            },
        };
        assert!(api::prereq_steps::proves(
            &established,
            &gate_window(&quest, &quest, 5, 61)
        ));
        assert!(api::prereq_steps::proves(
            &established,
            &gate_window(&quest, &quest, 8, 61)
        ));
        assert!(!api::prereq_steps::proves(
            &established,
            &gate_window(&quest, &quest, 61, 61)
        ));
    }
}
