//! Live C0 witness for the Desert Rescue rocky-elevation hazard.
//!
//! The measurement entrypoint and acceptance entrypoint run the same account,
//! scenario, and product actions. The measurement entrypoint records the
//! measured G/S/R gaps without evaluating the acceptance oracle; the parent
//! records that receipt before invoking the separate acceptance entrypoint.
//! Neither entrypoint changes world speed, pauses the script, or holds a native
//! script handle after Start.
//! The mapped command redirects stderr to
//! `LIVE_EVIDENCE_DIR/script-trace.log`; ScriptTrace is not a structured-sink
//! category. The receipt retains each operation's payload tick and leaves its
//! absent carrier tick unset.
#![cfg(all(feature = "live-harness", feature = "test-support"))]

#[path = "common/quester_live.rs"]
mod quester_live;

use api::snapshot::{ActorKind, GameSnapshot, WorldTile};
use quester_live::Mode;
use scenario::quester::{
    fixture_row, parse_quester_path_seeds, QuestFixtureProfile, QuesterPathSeeds, TestProfile,
};
use scenario::{Proof, ScenarioRunner, ScenarioSettings, Seed, Step, StepKind, Wait};
use script::native::{NativePhase, ScriptStatus, StatusValue};
use script::survival::Decision;
use script::ScriptTerminalState;
use serde_json::{json, Value};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;

const QUEST: &str = "desertrescue";
const DISPLAY: &str = "The Tourist Trap";
const PATH_SEEDS: &str = include_str!("fixtures/quester-path/desertrescue-cell-rock.json");
const LOBSTER_ID: i32 = 379;
const JOURNAL_ROOT: i32 = 8134;
const PRE_START_DRAIN: &str = "~stat_drain hitpoints 26 0";
const CLIMB_LINE: &str = "You start climbing the rocky elevation.";
const SLIP_LINE: &str = "You slip a little and tumble the rest of the way down the slope.";
const ROCK_PROMPT: &str = "It looks as if there is a rocky escarpment";
const HAZARD_SOURCE: &str =
    "content/scripts/quests/quest_desertrescue/scripts/quest_desertrescue.rs2:266-271";
const CLIMB_STEP: &str = "climb-cell-rock-1";
const SECOND_ROCK_STEP: &str = "climb-cell-rock-2";
const OBSERVED_TICK_LIMIT: u32 = 600;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Mark {
    tick: u32,
    poll: u64,
}

impl Mark {
    fn gap(self, earlier: Self) -> i64 {
        i64::from(self.tick) - i64::from(earlier.tick)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct FoodDrop {
    at: Mark,
    from: i32,
    to: i32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ChatEvent {
    at: Mark,
    sequence: i32,
    kind: &'static str,
    text: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct C0Frame {
    at: Mark,
    hitpoints: i32,
    lobsters: i32,
    main_modal: i32,
    chat_modal: i32,
    prompt_open: bool,
    climb_line: bool,
    slip_line: bool,
    new_climb_line: bool,
    new_slip_line: bool,
    hit_kind1: Option<i32>,
    decision: Option<Decision>,
    survival_owner: Option<String>,
    native_phase: Option<String>,
    step_id: Option<String>,
    stage: Option<String>,
    npc_facing_or_attacking: bool,
    tile: Option<[i32; 3]>,
    local_player: bool,
    failure: Option<String>,
}

impl C0Frame {
    fn same_state(&self, other: &Self) -> bool {
        self.at.tick == other.at.tick
            && self.hitpoints == other.hitpoints
            && self.lobsters == other.lobsters
            && self.main_modal == other.main_modal
            && self.chat_modal == other.chat_modal
            && self.prompt_open == other.prompt_open
            && self.climb_line == other.climb_line
            && self.slip_line == other.slip_line
            && self.hit_kind1 == other.hit_kind1
            && self.decision == other.decision
            && self.survival_owner == other.survival_owner
            && self.native_phase == other.native_phase
            && self.step_id == other.step_id
            && self.stage == other.stage
            && self.npc_facing_or_attacking == other.npc_facing_or_attacking
            && self.tile == other.tile
            && self.local_player == other.local_player
            && self.failure == other.failure
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum End {
    Step2(Mark),
    SlipAfterRestart { slip: Mark, observed: Mark },
}

#[derive(Clone, Debug, Default)]
struct C0Receipt {
    start: Option<Mark>,
    start_seed: Option<Value>,
    pre_drain_send: Option<Mark>,
    r0: Option<Mark>,
    g_drop: Option<FoodDrop>,
    l0: Option<Mark>,
    s_send: Option<Mark>,
    s1: Option<Mark>,
    s_drop: Option<FoodDrop>,
    restart: Option<Mark>,
    step2: Option<Mark>,
    slip_events: Vec<Mark>,
    slip_after_restart: Option<Mark>,
    end: Option<End>,
    drops: Vec<FoodDrop>,
    chat_events: Vec<ChatEvent>,
    frames: Vec<C0Frame>,
    survival_op_events: Vec<SurvivalOpEvent>,
    survival_op_parse_errors: Vec<String>,
    death: bool,
    failure: Option<String>,
}

impl C0Receipt {
    fn phase_r(&self) -> Option<(Mark, &'static str)> {
        if let Some(restart) = self.restart {
            Some((restart, "restart_prompt"))
        } else {
            self.step2.map(|step2| (step2, "step2_success"))
        }
    }

    fn json(&self, purpose: RunPurpose, logs: &[HostLog]) -> Value {
        let phase_r = self.phase_r();
        let first_trigger = self
            .frames
            .iter()
            .filter(|frame| frame.prompt_open || frame.new_climb_line)
            .map(|frame| frame.at)
            .min();
        let selected_hazard = logs
            .iter()
            .filter(|line| {
                line.message
                    .starts_with("quester hazard step=climb-cell-rock-1 ")
            })
            .map(HostLog::json)
            .collect::<Vec<_>>();
        let undeclared_damage = logs
            .iter()
            .filter(|line| line.message.contains("undeclared damage"))
            .map(HostLog::json)
            .collect::<Vec<_>>();
        let journal_retries = logs
            .iter()
            .filter(|line| is_journal_retry(&line.message))
            .map(HostLog::json)
            .collect::<Vec<_>>();
        let mut receipt = json!({
            "run_purpose": purpose.as_str(),
            "quest": QUEST,
            "display": DISPLAY,
            "stage": "desertrescue:6",
            "step": CLIMB_STEP,
            "start": self.start.map(mark_json),
            "start_seed": self.start_seed,
            "pre_start_drain_send": self.pre_drain_send.map(mark_json),
            "R0": self.r0.map(mark_json),
            "G_drop": self.g_drop.as_ref().map(drop_json),
            "L0": self.l0.map(mark_json),
            "attempt_drain_send": self.s_send.map(mark_json),
            "S1": self.s1.map(mark_json),
            "S_drop": self.s_drop.as_ref().map(drop_json),
            "R": phase_r.map(|(mark, kind)| json!({"kind": kind, "at": mark_json(mark)})),
            "restart_prompt": self.restart.map(mark_json),
            "step2": self.step2.map(mark_json),
            "slip_after_restart": self.slip_after_restart.map(mark_json),
            "end": self.end.as_ref().map(end_json),
            "g_gap_ticks": self.g_drop.as_ref().zip(self.r0).map(|(drop, r0)| drop.at.gap(r0)),
            "s_gap_ticks": self.s_drop.as_ref().zip(self.s1).map(|(drop, s1)| drop.at.gap(s1)),
            "r_gap_ticks": self.s_drop.as_ref().zip(phase_r).map(|(drop, (r, _))| r.gap(drop.at)),
            "s1_after_l0_ticks": self.s1.zip(self.l0).map(|(s1, l0)| s1.gap(l0)),
        });
        receipt["survival_op_events"] = json!(self
            .survival_op_events
            .iter()
            .map(SurvivalOpEvent::json)
            .collect::<Vec<_>>());
        receipt["survival_op_parse_errors"] = json!(self.survival_op_parse_errors);
        receipt["first_prompt_or_climb"] = json!(first_trigger.map(mark_json));
        receipt["death"] = json!(self.death);
        receipt["failure"] = json!(self.failure);
        receipt["food_drops"] = json!(self.drops.iter().map(drop_json).collect::<Vec<_>>());
        receipt["chat_events"] = json!(self
            .chat_events
            .iter()
            .map(chat_event_json)
            .collect::<Vec<_>>());
        receipt["frames"] = json!(self.frames.iter().map(frame_json).collect::<Vec<_>>());
        receipt["hostlog"] = json!(logs.iter().map(HostLog::json).collect::<Vec<_>>());
        receipt["selected_hazard_logs"] = json!(selected_hazard);
        receipt["undeclared_damage_logs"] = json!(undeclared_damage);
        receipt["journal_retry_logs"] = json!(journal_retries);
        receipt["source_refs"] = json!({
            "path_step": "crates/script/paths/289/desertrescue.json:2747-2785",
            "hazard": HAZARD_SOURCE,
            "climb_start": "content/scripts/quests/quest_desertrescue/scripts/quest_desertrescue.rs2:262-266",
            "climb_slip": "content/scripts/quests/quest_desertrescue/scripts/quest_desertrescue.rs2:267-271",
            "restart_prompt": "content/scripts/quests/quest_desertrescue/scripts/quest_desertrescue.rs2:231-237",
            "journal_root": "crates/script/src/quest_journal/native.rs:11",
        });
        receipt["observability_limits"] = json!([
            "ScriptStatus has no observed game tick; this receipt pairs its status with the same polling GameSnapshot tick.",
            "Quest-step begin lines can be folded; survival OneOp begin/end events retain their own ticks.",
            "The engine does not publish a separate click-lock bit; applied=true/false is the observed OneOp outcome.",
        ]);
        receipt
    }
}

#[derive(Clone, Debug)]
struct HostLog {
    tick: Option<u64>,
    source: String,
    message: String,
}

impl HostLog {
    fn json(&self) -> Value {
        json!({"tick": self.tick, "source": self.source, "message": self.message})
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SurvivalOpEventKind {
    Begin,
    End { applied: bool },
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct SurvivalOpEvent {
    event: SurvivalOpEventKind,
    tick: u64,
    kind: String,
    deadline: u64,
    hostlog_tick: Option<u64>,
}

impl SurvivalOpEvent {
    fn json(&self) -> Value {
        let (event, applied) = match self.event {
            SurvivalOpEventKind::Begin => ("begin", Value::String("pending".into())),
            SurvivalOpEventKind::End { applied } => ("end", Value::Bool(applied)),
        };
        json!({
            "event": event,
            "tick": self.tick,
            "kind": self.kind,
            "applied": applied,
            "deadline": self.deadline,
            "hostlog_tick": self.hostlog_tick,
        })
    }
}

fn parse_survival_op_event(line: &HostLog) -> Result<Option<SurvivalOpEvent>, String> {
    let Some(payload) = line.message.strip_prefix("quester survival op event=") else {
        return Ok(None);
    };
    let malformed = || format!("malformed survival operation trace: {}", line.message);
    let (event, remainder) = payload.split_once(" tick=").ok_or_else(&malformed)?;
    let (tick, remainder) = remainder.split_once(" kind=").ok_or_else(&malformed)?;
    let (kind, remainder) = remainder.split_once(" applied=").ok_or_else(&malformed)?;
    let (applied, deadline) = remainder.split_once(" deadline=").ok_or_else(&malformed)?;
    if kind.is_empty() {
        return Err(malformed());
    }
    let event = match (event, applied) {
        ("begin", "pending") => SurvivalOpEventKind::Begin,
        ("end", "true") => SurvivalOpEventKind::End { applied: true },
        ("end", "false") => SurvivalOpEventKind::End { applied: false },
        _ => return Err(malformed()),
    };
    Ok(Some(SurvivalOpEvent {
        event,
        tick: tick.parse().map_err(|_| malformed())?,
        kind: kind.to_owned(),
        deadline: deadline.parse().map_err(|_| malformed())?,
        hostlog_tick: line.tick,
    }))
}

fn parse_survival_op_logs(logs: &[HostLog]) -> (Vec<SurvivalOpEvent>, Vec<String>) {
    let mut events = Vec::new();
    let mut errors = Vec::new();
    let mut previous_tick = None;
    for line in logs {
        match parse_survival_op_event(line) {
            Ok(Some(event)) => {
                if let Some(previous_tick) = previous_tick {
                    if event.tick < previous_tick {
                        errors.push(format!(
                            "survival operation trace tick {} follows later tick {}",
                            event.tick, previous_tick
                        ));
                    }
                }
                previous_tick = Some(event.tick);
                events.push(event);
            }
            Ok(None) => {}
            Err(error) => errors.push(error),
        }
    }
    (events, errors)
}

struct C0LogSink;
static C0_LOG_SINK: C0LogSink = C0LogSink;
static C0_LOGS: Mutex<Vec<HostLog>> = Mutex::new(Vec::new());
static C0_LOG_INSTALLED: LazyLock<bool> =
    LazyLock::new(|| api::hostlog::install_sink(&C0_LOG_SINK));
static C0_LIVE_TEST_LOCK: Mutex<()> = Mutex::new(());

impl api::hostlog::Sink for C0LogSink {
    fn record(&self, record: &api::hostlog::Record<'_>) {
        let message = record.message;
        let script = matches!(record.source, api::hostlog::Source::Script);
        let relevant = message.contains("climb-cell-rock-1")
            || message.starts_with("quester survival op event=")
            || message.contains("quester hazard")
            || message.contains("undeclared damage")
            || message.contains("journal retry")
            || message.contains("journal read retry")
            || message.contains("retry_journal_read")
            || message.contains("operator stop");
        if script && relevant {
            if let Ok(mut logs) = C0_LOGS.lock() {
                logs.push(HostLog {
                    tick: record.tick.map(u64::from),
                    source: format!("{:?}", record.source),
                    message: message.to_owned(),
                });
            }
        }
    }
}

fn install_c0_log_sink() -> Result<(), String> {
    if !*C0_LOG_INSTALLED {
        return Err("C0 hostlog sink was not the process's first sink".into());
    }
    C0_LOGS
        .lock()
        .map_err(|_| "C0 hostlog buffer poisoned".to_owned())?
        .clear();
    Ok(())
}

fn hostlogs() -> Vec<HostLog> {
    C0_LOGS.lock().map(|logs| logs.clone()).unwrap_or_default()
}

fn read_survival_op_trace(path: &std::path::Path) -> Result<Vec<HostLog>, String> {
    let text = std::fs::read_to_string(path).map_err(|error| {
        format!(
            "read C0 ScriptTrace stderr file {}: {error}",
            path.display()
        )
    })?;
    Ok(parse_survival_op_stderr(&text))
}

fn parse_survival_op_stderr(text: &str) -> Vec<HostLog> {
    text.lines()
        .filter(|line| line.starts_with("[script-trace ") || line.starts_with("[script-trace] "))
        .filter_map(|line| line.split_once("] ").map(|(_, message)| message))
        .filter(|message| message.starts_with("quester survival op event="))
        .map(|message| HostLog {
            tick: None,
            source: "ScriptTrace".into(),
            message: message.to_owned(),
        })
        .collect()
}

#[derive(Clone, Copy)]
enum RunPurpose {
    Measurement,
    Acceptance,
}

impl RunPurpose {
    fn as_str(self) -> &'static str {
        match self {
            Self::Measurement => "measurement-only",
            Self::Acceptance => "acceptance",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Measurement => "upkeep-cell-rock-measurement",
            Self::Acceptance => "upkeep-cell-rock-acceptance",
        }
    }
}

struct LiveWitness {
    receipt: C0Receipt,
    polls: u64,
    last_lobsters: i32,
    last_chat_sequence: i32,
    journal_open_seen: bool,
    prompt_was_open: bool,
}

impl Default for LiveWitness {
    fn default() -> Self {
        Self {
            receipt: C0Receipt::default(),
            polls: 0,
            last_lobsters: 8,
            last_chat_sequence: -1,
            journal_open_seen: false,
            prompt_was_open: false,
        }
    }
}

struct LiveCapture {
    purpose: RunPurpose,
    result: Result<Value, String>,
    receipt: C0Receipt,
    logs: Vec<HostLog>,
}

fn build_cell(
    label: &'static str,
    witness: Arc<Mutex<LiveWitness>>,
) -> Result<quester_live::Cell, String> {
    let seeds = parse_quester_path_seeds(PATH_SEEDS, "desertrescue-cell-rock")?;
    let row: QuestFixtureProfile = fixture_row(QUEST)?;
    if row.profile != TestProfile::Base40 || row.display != DISPLAY {
        return Err(format!("fixture profile moved: {row:?}"));
    }
    let loadout = seeds.fixture_loadout()?;
    let extra_items = seeds.extra_item_refs();
    let seed_vars = seeds.seed_var_refs();
    let before_relog = seeds.before_relog_steps();
    let mode = match seeds.mode.as_deref().unwrap_or("stage") {
        "stage" => Mode::Stage {
            expect: seeds.expect.clone(),
        },
        other => return Err(format!("unexpected C0 fixture mode {other:?}")),
    };
    let mut cell = quester_live::path_cell(quester_live::PathCell {
        quest: row.quest,
        display: row.display,
        label: label.to_owned(),
        stage: Some(seeds.stage.as_str()),
        loadout,
        extra_items: &extra_items,
        seed_vars: &seed_vars,
        stand: WorldTile {
            x: seeds.stand[0],
            z: seeds.stand[1],
            level: seeds.stand[2],
        },
        mode,
        before_relog,
    })?;
    let observe_start = cell
        .observe_start
        .take()
        .ok_or("Path fixture did not provide its Start seed observer")?;
    insert_pre_start_drain(&mut cell, observe_start, Arc::clone(&witness))?;
    insert_attempt_drain_and_anchors(&mut cell, witness)?;
    if let Some(last) = cell.scenario.steps.last_mut() {
        last.wait.budget_ticks = OBSERVED_TICK_LIMIT;
    }
    Ok(cell)
}

fn insert_pre_start_drain(
    cell: &mut quester_live::Cell,
    observe_start: quester_live::ObserveStart,
    witness: Arc<Mutex<LiveWitness>>,
) -> Result<(), String> {
    let observer = Arc::new(Mutex::new(observe_start));
    let send_witness = Arc::clone(&witness);
    let drain_observer = Arc::clone(&observer);
    let step = Step {
        name: "prove the Base40 fixture, then drain Hitpoints before Start",
        kind: StepKind::Perform {
            send: Box::new(move |client, snapshot| {
                let mut witness = match send_witness.lock() {
                    Ok(witness) => witness,
                    Err(_) => return false,
                };
                if witness.receipt.start_seed.is_none() {
                    let mut observer = match drain_observer.lock() {
                        Ok(observer) => observer,
                        Err(_) => return false,
                    };
                    match observer(snapshot) {
                        Ok(receipt) => {
                            witness.last_lobsters = snapshot.inv_count(LOBSTER_ID);
                            witness.last_chat_sequence = latest_chat_sequence(snapshot);
                            witness.receipt.start_seed = Some(receipt);
                        }
                        Err(error) => {
                            witness.receipt.failure =
                                Some(format!("pre-Start seed proof: {error}"));
                            return false;
                        }
                    }
                }
                let sent = api::interact::cheat(client, PRE_START_DRAIN).is_sent();
                if sent {
                    witness.receipt.pre_drain_send = Some(Mark {
                        tick: snapshot.tick(),
                        poll: witness.polls.saturating_add(1),
                    });
                }
                sent
            }),
        },
        wait: Wait {
            arm: Proof::StatAtMost { id: 3, max: 14 },
            budget_ticks: 200,
        },
    };
    let start = cell
        .scenario
        .steps
        .iter()
        .position(|step| matches!(step.kind, StepKind::StartScript))
        .ok_or("Path fixture has no native Start")?;
    cell.scenario.steps.splice(start..start, [step]);
    // The shared runner's after-Start observer would repeat the exact-stat
    // proof against the intentionally drained account. The same observer now
    // runs in the final pre-Start Perform step above, before any account-only
    // cheat changes Hitpoints.
    cell.observe_start = None;
    Ok(())
}

fn insert_attempt_drain_and_anchors(
    cell: &mut quester_live::Cell,
    witness: Arc<Mutex<LiveWitness>>,
) -> Result<(), String> {
    let start = cell
        .scenario
        .steps
        .iter()
        .position(|step| matches!(step.kind, StepKind::StartScript))
        .ok_or("Path fixture has no native Start")?;
    let post_start = start + 1;
    let r0_ready = Arc::clone(&witness);
    let r0 = Step {
        name: "observe main journal open then closed for R0",
        kind: StepKind::Await {
            evidence: "main journal root 8134 opened and then closed",
            ready: Box::new(move |snapshot| {
                let Ok(mut witness) = r0_ready.lock() else {
                    return false;
                };
                let poll = witness.polls.saturating_add(1);
                observe_journal(&mut witness, snapshot, poll);
                witness.receipt.r0.is_some()
            }),
            summary: None,
        },
        wait: Wait {
            arm: Proof::IngameScene2,
            budget_ticks: OBSERVED_TICK_LIMIT,
        },
    };
    let l0_witness = Arc::clone(&witness);
    let l0 = Step {
        name: "observe the fresh rocky-elevation line for L0",
        kind: StepKind::Await {
            evidence: "fresh You start climbing the rocky elevation line",
            ready: Box::new(move |snapshot| {
                let Ok(mut witness) = l0_witness.lock() else {
                    return false;
                };
                let Some(baseline) = witness.last_chat_sequence.checked_add(1) else {
                    return false;
                };
                if witness.receipt.r0.is_some()
                    && snapshot
                        .chat_lines()
                        .iter()
                        .any(|line| line.sequence >= baseline && line.text.contains(CLIMB_LINE))
                {
                    let poll = witness.polls.saturating_add(1);
                    witness.receipt.l0.get_or_insert(Mark {
                        tick: snapshot.tick(),
                        poll,
                    });
                }
                witness.receipt.l0.is_some()
            }),
            summary: None,
        },
        wait: Wait {
            arm: Proof::IngameScene2,
            budget_ticks: OBSERVED_TICK_LIMIT,
        },
    };
    let drain_witness = Arc::clone(&witness);
    let drain = Step {
        name: "drain Hitpoints on the client cycle after L0",
        kind: StepKind::Perform {
            send: Box::new(move |client, snapshot| {
                let Ok(mut witness) = drain_witness.lock() else {
                    return false;
                };
                if witness.receipt.s_send.is_some() {
                    return true;
                }
                let amount = stat_effective(snapshot, 3).saturating_sub(14).max(0);
                let command = format!("~stat_drain hitpoints {amount} 0");
                let sent = api::interact::cheat(client, &command).is_sent();
                if sent {
                    witness.receipt.s_send = Some(Mark {
                        tick: snapshot.tick(),
                        poll: witness.polls.saturating_add(1),
                    });
                }
                sent
            }),
        },
        wait: Wait {
            arm: Proof::StatAtMost { id: 3, max: 14 },
            budget_ticks: 3,
        },
    };
    cell.scenario
        .steps
        .splice(post_start..post_start, [r0, l0, drain]);
    Ok(())
}

fn live_callbacks(
    purpose: RunPurpose,
    witness: Arc<Mutex<LiveWitness>>,
) -> (quester_live::ObserveFamily, quester_live::ObserveTick) {
    let tick_witness = Arc::clone(&witness);
    let observe_tick: quester_live::ObserveTick = Box::new(move |snapshot, status| {
        let status = status?;
        let mut witness = tick_witness.lock().ok()?;
        let emit = record_frame(&mut witness, snapshot, status);
        emit.then(|| {
            let frame = witness.receipt.frames.last();
            json!({"c0": frame.map(frame_json)})
        })
    });

    let family_witness = Arc::clone(&witness);
    let observe_family: quester_live::ObserveFamily =
        Box::new(move |snapshot, status, lifecycle| {
            let witness = family_witness
                .lock()
                .map_err(|_| "C0 live witness poisoned".to_owned())?;
            let now = Mark {
                tick: snapshot.tick(),
                poll: witness.polls,
            };
            let logs = hostlogs();
            if witness.receipt.death || snapshot.local_player().is_none() {
                return Err("C0 failed: player death or missing local player".into());
            }
            if let Some(failure) = witness.receipt.failure.as_deref() {
                return Err(format!("C0 failed: {failure}"));
            }
            if let Some(status) = status {
                if status.phase == NativePhase::Blocked || status.failure.is_some() {
                    return Err(format!(
                    "C0 failed: named stop/parked/needs-evidence status phase={:?} failure={:?}",
                    status.phase, status.failure
                ));
                }
                if let Some(stage) = quester_live::status_stage(status, QUEST) {
                    if stage != "desertrescue:6" && witness.receipt.s_drop.is_none() {
                        return Err(format!(
                            "C0 ended before phase S completed: stage advanced to {stage}"
                        ));
                    }
                }
            }
            if logs.iter().any(|line| {
                line.message.starts_with("quester desertrescue: stop")
                    || line.message.contains("operator stop")
            }) {
                return Err("C0 failed: named Stop observed in hostlog".into());
            }
            if let Some(start) = witness.receipt.start {
                if now.tick.saturating_sub(start.tick) > OBSERVED_TICK_LIMIT {
                    return Err(format!(
                    "C0 exceeded {OBSERVED_TICK_LIMIT} observed ticks before its terminal witness"
                ));
                }
            }
            if lifecycle.is_some_and(|receipt| {
                matches!(
                    receipt.state,
                    ScriptTerminalState::Stopped | ScriptTerminalState::Failed
                )
            }) {
                return Err("C0 script stopped or failed before the live witness ended".into());
            }
            if lifecycle.is_some_and(|receipt| receipt.state == ScriptTerminalState::Completed)
                && witness.receipt.end.is_none()
            {
                return Err("C0 script completed before phase S and its terminal witness".into());
            }
            if witness.receipt.end.is_some() {
                return Ok(Some(witness.receipt.json(purpose, &logs)));
            }
            Ok(None)
        });
    (observe_family, observe_tick)
}

fn run_c0(purpose: RunPurpose) -> Result<LiveCapture, String> {
    let trace_path = std::path::PathBuf::from(
        std::env::var_os("LIVE_EVIDENCE_DIR").ok_or("C0 requires LIVE_EVIDENCE_DIR")?,
    )
    .join("script-trace.log");
    if !trace_path.is_file() {
        return Err(format!(
            "C0 requires stderr redirected to {} as in the live-map command",
            trace_path.display()
        ));
    }
    install_c0_log_sink()?;
    let witness = Arc::new(Mutex::new(LiveWitness::default()));
    let cell = build_cell(purpose.label(), Arc::clone(&witness))?;
    let (observe_family, observe_tick) = live_callbacks(purpose, Arc::clone(&witness));
    let result = quester_live::run_observed(cell, observe_family, observe_tick);
    let mut logs = hostlogs();
    logs.extend(read_survival_op_trace(&trace_path)?);
    let mut receipt = witness
        .lock()
        .map_err(|_| "C0 live witness poisoned after run".to_owned())?
        .receipt
        .clone();
    let (events, parse_errors) = parse_survival_op_logs(&logs);
    receipt.survival_op_events = events;
    receipt.survival_op_parse_errors = parse_errors;
    Ok(LiveCapture {
        purpose,
        result,
        receipt,
        logs,
    })
}

fn record_frame(witness: &mut LiveWitness, snapshot: &GameSnapshot, status: &ScriptStatus) -> bool {
    witness.polls = witness.polls.saturating_add(1);
    let at = Mark {
        tick: snapshot.tick(),
        poll: witness.polls,
    };
    witness.receipt.start.get_or_insert(at);
    observe_journal(witness, snapshot, at.poll);

    let lobsters = snapshot.inv_count(LOBSTER_ID);
    if lobsters < witness.last_lobsters {
        let drop = FoodDrop {
            at,
            from: witness.last_lobsters,
            to: lobsters,
        };
        if witness.receipt.g_drop.is_none() && drop.from == 8 && drop.to == 7 {
            witness.receipt.g_drop = Some(drop.clone());
        }
        if witness.receipt.s_drop.is_none() && drop.from == 7 && drop.to == 6 {
            witness.receipt.s_drop = Some(drop.clone());
        }
        witness.receipt.drops.push(drop);
    }
    witness.last_lobsters = lobsters;

    let baseline = witness.last_chat_sequence;
    let mut max_sequence = baseline;
    let mut new_climb_line = false;
    let mut new_slip_line = false;
    for line in snapshot.chat_lines() {
        max_sequence = max_sequence.max(line.sequence);
        if line.sequence <= baseline {
            continue;
        }
        if line.text.contains(CLIMB_LINE) {
            new_climb_line = true;
            witness.receipt.l0.get_or_insert(at);
            witness.receipt.chat_events.push(ChatEvent {
                at,
                sequence: line.sequence,
                kind: "climb_start",
                text: line.text.clone(),
            });
        }
        if line.text.contains(SLIP_LINE) {
            new_slip_line = true;
            witness.receipt.slip_events.push(at);
            witness.receipt.chat_events.push(ChatEvent {
                at,
                sequence: line.sequence,
                kind: "slip",
                text: line.text.clone(),
            });
            if witness.receipt.restart.is_some() && witness.receipt.slip_after_restart.is_none() {
                witness.receipt.slip_after_restart = Some(at);
            }
        }
    }
    witness.last_chat_sequence = max_sequence;

    let chat_texts = snapshot.chat_modal_texts();
    let prompt_open =
        snapshot.modals().chat != -1 && chat_texts.iter().any(|text| text.contains(ROCK_PROMPT));
    if prompt_open
        && !witness.prompt_was_open
        && witness.receipt.s_drop.is_some()
        && witness.receipt.restart.is_none()
    {
        witness.receipt.restart = Some(at);
    }
    witness.prompt_was_open = prompt_open;

    let step_id = status_text(Some(status), "step_id")
        .or_else(|| status_text(Some(status), "child_step_id"))
        .map(str::to_owned);
    if step_id.as_deref() == Some(SECOND_ROCK_STEP)
        && witness.receipt.s_drop.is_some()
        && witness.receipt.step2.is_none()
    {
        witness.receipt.step2 = Some(at);
    }
    if witness.receipt.s1.is_none()
        && witness
            .receipt
            .s_send
            .is_some_and(|send| at.poll > send.poll)
        && witness.receipt.l0.is_some()
        && stat_effective(snapshot, 3) <= 14
    {
        witness.receipt.s1 = Some(at);
    }

    let hit_kind1 = active_hitmark_kind1(snapshot);
    let decision = Some(status.survival.decision);
    let player_index = snapshot.local_player().map(|player| player.player.index);
    let npc_facing_or_attacking = snapshot.npcs().iter().any(|npc| {
        let targets_player = npc.target.is_some_and(|target| {
            target.kind == ActorKind::Player && Some(target.index) == player_index
        });
        let faces_player = player_index
            .and_then(|index| i32::try_from(index).ok())
            .is_some_and(|index| npc.face_entity == index);
        targets_player || faces_player
    });
    let tile = snapshot.tile().map(|(x, z, level)| [x, z, level]);
    let frame = C0Frame {
        at,
        hitpoints: stat_effective(snapshot, 3),
        lobsters,
        main_modal: snapshot.modals().main,
        chat_modal: snapshot.modals().chat,
        prompt_open,
        climb_line: snapshot
            .chat_lines()
            .iter()
            .any(|line| line.text.contains(CLIMB_LINE)),
        slip_line: snapshot
            .chat_lines()
            .iter()
            .any(|line| line.text.contains(SLIP_LINE)),
        new_climb_line,
        new_slip_line,
        hit_kind1,
        decision,
        survival_owner: Some(format!("{:?}", status.survival.owner)),
        native_phase: Some(format!("{:?}", status.phase)),
        step_id,
        stage: quester_live::status_stage(status, QUEST),
        npc_facing_or_attacking,
        tile,
        local_player: snapshot.local_player().is_some(),
        failure: status
            .failure
            .as_ref()
            .map(|failure| format!("{failure:?}")),
    };
    witness.receipt.death |= frame.hitpoints <= 0 || !frame.local_player;
    let emit = witness
        .receipt
        .frames
        .last()
        .is_none_or(|last| last.at.tick != frame.at.tick || !last.same_state(&frame))
        || new_climb_line
        || new_slip_line;
    if emit {
        witness.receipt.frames.push(frame);
    }
    if let Some(step2) = witness.receipt.step2 {
        if witness
            .receipt
            .s_drop
            .as_ref()
            .is_some_and(|drop| step2 >= drop.at)
        {
            witness.receipt.end.get_or_insert(End::Step2(step2));
        }
    }
    if let (Some(slip), Some(restart)) =
        (witness.receipt.slip_after_restart, witness.receipt.restart)
    {
        if slip > restart && at.tick >= slip.tick.saturating_add(3) {
            witness
                .receipt
                .end
                .get_or_insert(End::SlipAfterRestart { slip, observed: at });
        }
    }
    emit
}

fn observe_journal(witness: &mut LiveWitness, snapshot: &GameSnapshot, poll: u64) {
    let at = Mark {
        tick: snapshot.tick(),
        poll,
    };
    if snapshot.modals().main == JOURNAL_ROOT {
        witness.journal_open_seen = true;
    } else if witness.journal_open_seen && witness.receipt.r0.is_none() {
        witness.receipt.r0 = Some(at);
    }
}

fn status_text<'a>(status: Option<&'a ScriptStatus>, key: &str) -> Option<&'a str> {
    status?.fields.iter().find_map(|field| match &field.value {
        StatusValue::Text(value) if field.key == key => Some(value.as_ref()),
        _ => None,
    })
}

fn latest_chat_sequence(snapshot: &GameSnapshot) -> i32 {
    snapshot
        .chat_lines()
        .iter()
        .map(|line| line.sequence)
        .max()
        .unwrap_or(-1)
}

fn stat_effective(snapshot: &GameSnapshot, id: i32) -> i32 {
    snapshot
        .stats()
        .iter()
        .find(|stat| stat.index == id)
        .map_or(0, |stat| stat.effective)
}

fn active_hitmark_kind1(snapshot: &GameSnapshot) -> Option<i32> {
    let hitmarks = snapshot.hitmarks()?;
    hitmarks
        .marks
        .iter()
        .find(|mark| mark.kind == 1 && mark.value > 0 && mark.cycle > hitmarks.loop_cycle)
        .map(|mark| mark.value)
}

fn is_journal_retry(message: &str) -> bool {
    message.contains("journal retry")
        || message.contains("journal read retry")
        || message.contains("retry_journal_read")
        || message.contains("retry quiet period")
}

fn parse_eat_kind(kind: &str) -> Option<(i32, i32)> {
    let body = kind.strip_prefix("Eat { item_id: ")?;
    let (item_id, before_count) = body.split_once(", before_count: ")?;
    let item_id = item_id.parse().ok()?;
    let before_count = before_count.strip_suffix(" }")?.parse().ok()?;
    Some((item_id, before_count))
}

fn survival_op_pending_before_tick(receipt: &C0Receipt, tick: u64) -> Result<bool, String> {
    if !receipt.survival_op_parse_errors.is_empty() {
        return Err(format!(
            "malformed survival operation trace: {}",
            receipt.survival_op_parse_errors.join("; ")
        ));
    }
    let mut active: Option<(&str, u64)> = None;
    for event in &receipt.survival_op_events {
        if event.tick >= tick {
            break;
        }
        match event.event {
            SurvivalOpEventKind::Begin => {
                if active.is_some() {
                    return Err(format!(
                        "overlapping survival operations before tick {tick}"
                    ));
                }
                active = Some((&event.kind, event.deadline));
            }
            SurvivalOpEventKind::End { .. } => {
                if active.is_some_and(|(kind, deadline)| {
                    kind == event.kind && deadline == event.deadline
                }) {
                    active = None;
                } else {
                    return Err(format!(
                        "unmatched survival operation end before tick {tick}"
                    ));
                }
            }
        }
    }
    Ok(active.is_some())
}

fn require_phase_s_op_sequence(
    receipt: &C0Receipt,
    s1: Mark,
    s_drop: &FoodDrop,
) -> Result<(), String> {
    let mut events = receipt.survival_op_events.iter().filter(|event| {
        parse_eat_kind(&event.kind) == Some((LOBSTER_ID, 7))
            && event.tick >= u64::from(s1.tick)
            && event.tick <= u64::from(s_drop.at.tick)
    });
    let begin = events.next().ok_or(
        "R4-S2: missing survival OneOp begin at S1; no pending/applied operation evidence",
    )?;
    if begin.event != SurvivalOpEventKind::Begin || begin.tick != u64::from(s1.tick) {
        return Err(format!(
            "R4-S2: first Eat(379, 7) event must begin at S1 tick {}; got {:?} at {}",
            s1.tick, begin.event, begin.tick
        ));
    }
    if begin.deadline != begin.tick.saturating_add(8) {
        return Err(format!(
            "R4-S2: first Eat(379, 7) deadline {} is not S1+8",
            begin.deadline
        ));
    }
    let first_end = events
        .next()
        .ok_or("R4-S2: missing first survival OneOp end at its deadline")?;
    if first_end.event != (SurvivalOpEventKind::End { applied: false })
        || first_end.tick != begin.deadline
        || first_end.kind != begin.kind
        || first_end.deadline != begin.deadline
    {
        return Err(format!(
            "R4-S2: first Eat(379, 7) must end unapplied at its deadline {}; got {:?} at {}",
            begin.deadline, first_end.event, first_end.tick
        ));
    }
    let retry_begin = events
        .next()
        .ok_or("R4-S2: missing retry begin at the first unapplied end tick")?;
    if retry_begin.event != SurvivalOpEventKind::Begin
        || retry_begin.tick != first_end.tick
        || retry_begin.kind != begin.kind
    {
        return Err(format!(
            "R4-S2: retry must begin at rejected-operation tick {} with the same Eat kind",
            first_end.tick
        ));
    }
    if retry_begin.deadline != retry_begin.tick.saturating_add(8) {
        return Err(format!(
            "R4-S2: retry Eat deadline {} is not retry-begin+8",
            retry_begin.deadline
        ));
    }
    let retry_end = events
        .next()
        .ok_or("R4-S2: missing applied retry end at the phase-S food drop")?;
    if retry_end.event != (SurvivalOpEventKind::End { applied: true })
        || retry_end.tick != u64::from(s_drop.at.tick)
        || retry_end.kind != retry_begin.kind
        || retry_end.deadline != retry_begin.deadline
        || events.next().is_some()
    {
        return Err(format!(
            "R4-S2: retry must end applied=true on S_drop tick {}; got {:?} at {}",
            s_drop.at.tick, retry_end.event, retry_end.tick
        ));
    }
    Ok(())
}

fn require_o2_applied_bite(receipt: &C0Receipt, frame: &C0Frame) -> Result<(), String> {
    let tick = u64::from(frame.at.tick);
    if survival_op_pending_before_tick(receipt, tick)? {
        return Ok(());
    }
    let kind = format!(
        "Eat {{ item_id: {LOBSTER_ID}, before_count: {} }}",
        frame.lobsters
    );
    let deadline = tick.saturating_add(8);
    let applied = receipt.survival_op_events.iter().any(|begin| {
        if begin.event != SurvivalOpEventKind::Begin
            || begin.tick != tick
            || begin.kind != kind
            || begin.deadline != deadline
        {
            return false;
        }
        receipt.survival_op_events.iter().any(|end| {
            end.event == (SurvivalOpEventKind::End { applied: true })
                && end.tick >= tick
                && end.tick <= tick.saturating_add(2)
                && end.kind == begin.kind
                && end.deadline == begin.deadline
                && receipt.drops.iter().any(|drop| {
                    drop.from == frame.lobsters
                        && drop.to == frame.lobsters - 1
                        && u64::from(drop.at.tick) == end.tick
                })
        })
    });
    if applied {
        Ok(())
    } else {
        Err(format!(
            "O2: no applied Eat operation and matching lobster drop within two ticks of slip/hit at tick {}",
            frame.at.tick
        ))
    }
}

fn c0_acceptance_oracle(receipt: &C0Receipt, logs: &[HostLog]) -> Result<(), String> {
    if receipt.death {
        return Err("death before C0 terminal witness".into());
    }
    if let Some(failure) = &receipt.failure {
        return Err(format!("fixture/scenario failure: {failure}"));
    }
    if !receipt.survival_op_parse_errors.is_empty() {
        return Err(format!(
            "R4-S2: malformed survival OneOp trace: {}",
            receipt.survival_op_parse_errors.join("; ")
        ));
    }
    if logs.iter().any(|line| is_journal_retry(&line.message)) {
        return Err("SR-8: journal retry observed before acceptance oracle".into());
    }
    let r0 = receipt
        .r0
        .ok_or("R0 missing: journal root 8134 was not observed open then closed")?;
    let g_drop = receipt
        .drops
        .iter()
        .find(|drop| drop.from == 8 && drop.to == 7)
        .ok_or("phase G missing: no first lobster drop 8 to 7")?;
    let g_gap = g_drop.at.gap(r0);
    if !(0..=3).contains(&g_gap) {
        return Err(format!(
            "phase G first drop gap {g_gap} ticks is outside R0..R0+3"
        ));
    }
    let first_prompt_or_climb = receipt
        .frames
        .iter()
        .filter(|frame| frame.prompt_open || frame.new_climb_line)
        .map(|frame| frame.at)
        .min()
        .ok_or("no first rocky prompt or climb-start frame was observed")?;
    if g_drop.at >= first_prompt_or_climb {
        return Err("phase G drop did not precede the first rock prompt/climb line".into());
    }

    let l0 = receipt
        .l0
        .ok_or("L0 missing: fresh rocky-elevation line was not observed")?;
    let s_send = receipt
        .s_send
        .ok_or("post-L0 stat_drain send was not observed")?;
    if s_send.poll != l0.poll.saturating_add(1) {
        return Err(format!(
            "post-L0 drain send was on client poll {}, expected next poll {}",
            s_send.poll,
            l0.poll.saturating_add(1)
        ));
    }
    let s1 = receipt
        .s1
        .ok_or("S1 missing: drained Hitpoints frame was not observed")?;
    let s1_gap = s1.gap(l0);
    if !(0..=2).contains(&s1_gap) {
        return Err(format!(
            "S1 was {s1_gap} ticks after L0; the dropped-click path requires S1 no later than L0+2"
        ));
    }
    let s1_frame = receipt
        .frames
        .iter()
        .find(|frame| frame.at == s1)
        .ok_or("S1 status frame was not retained")?;
    if s1_frame.hitpoints > 14
        || s1_frame.decision != Some(Decision::Eating)
        || s1_frame.step_id.as_deref() != Some(CLIMB_STEP)
    {
        return Err(format!(
            "S1 must show HP<=14 and Decision::Eating on {CLIMB_STEP}: hp={}, decision={:?}, step={:?}",
            s1_frame.hitpoints, s1_frame.decision, s1_frame.step_id
        ));
    }
    let s_drop = receipt
        .drops
        .iter()
        .find(|drop| drop.from == 7 && drop.to == 6)
        .ok_or("phase S missing: no second lobster drop 7 to 6")?;
    let s_drop_gap = s_drop.at.gap(s1);
    if !(9..=12).contains(&s_drop_gap) {
        return Err(format!(
            "phase S drop was {s_drop_gap} ticks after S1; the dropped-first-click retry path requires S1+9..S1+12"
        ));
    }
    require_phase_s_op_sequence(receipt, s1, s_drop)?;

    for line in logs.iter().filter(|line| {
        line.message
            .contains("step climb-cell-rock-1 (interact) begin")
    }) {
        let tick = line
            .tick
            .ok_or("climb begin hostlog has no observed tick")?;
        if tick > u64::from(s1.tick) && tick < u64::from(s_drop.at.tick) {
            return Err(
                "a climb-cell-rock-1 begin trace appeared between S1 and the phase-S drop".into(),
            );
        }
    }

    for frame in &receipt.frames {
        if frame.decision == Some(Decision::Eating)
            && frame.hitpoints > 15
            && !(frame.hit_kind1.is_some() && frame.hitpoints <= 20)
            && !frame.npc_facing_or_attacking
        {
            return Err(format!(
                "O3: Eating decision at HP {} without a damage-hit window or NPC on tick {}",
                frame.hitpoints, frame.at.tick
            ));
        }
    }
    for frame in &receipt.frames {
        let o2_candidate = frame.new_slip_line
            && frame.hit_kind1.is_some()
            && frame.hitpoints <= 20
            && frame.lobsters > 0;
        if o2_candidate {
            require_o2_applied_bite(receipt, frame)?;
        }
    }

    let hazard_logs: Vec<&HostLog> = logs
        .iter()
        .filter(|line| {
            line.message
                .starts_with("quester hazard step=climb-cell-rock-1 ")
        })
        .collect();
    if hazard_logs.len() != 1 {
        return Err(format!(
            "O6: expected one selected hazard log for {CLIMB_STEP}, got {}",
            hazard_logs.len()
        ));
    }
    let hazard = &hazard_logs[0].message;
    if !hazard.contains("max=7") || !hazard.contains(HAZARD_SOURCE) || !hazard.contains("line=15") {
        return Err(format!("O6: selected hazard evidence mismatch: {hazard}"));
    }
    if logs
        .iter()
        .any(|line| line.message.contains("undeclared damage"))
    {
        return Err("O6: undeclared damage hostlog line was emitted".into());
    }

    let first_slip_after_s1 = receipt
        .slip_events
        .iter()
        .copied()
        .find(|slip| slip.tick >= s1.tick);
    let phase_r = receipt.phase_r();
    match first_slip_after_s1 {
        Some(_) => {
            let restart = receipt
                .restart
                .ok_or("failed climb did not reopen the restart prompt")?;
            let restart_gap = restart.gap(s_drop.at);
            if !(0..=16).contains(&restart_gap) {
                return Err(format!(
                    "restart prompt gap {restart_gap} ticks is outside S-drop..S-drop+16"
                ));
            }
            let terminal_after_restart = match receipt.end.as_ref() {
                Some(End::Step2(at)) => *at >= restart,
                Some(End::SlipAfterRestart { slip, observed }) => {
                    *slip > restart && observed.tick >= slip.tick.saturating_add(3)
                }
                None => false,
            };
            if !terminal_after_restart {
                return Err("run did not end at restart slip+3 or step2 after restart".into());
            }
        }
        None => {
            let step2 = receipt
                .step2
                .ok_or("run ended before phase S outcome and step2 status")?;
            if step2 < s_drop.at || !matches!(receipt.end, Some(End::Step2(_))) {
                return Err("successful climb did not end on step2 after phase S".into());
            }
        }
    }
    if phase_r.is_none() {
        return Err("phase R/success step2 marker was not recorded".into());
    }
    if receipt.end.is_none() {
        return Err("run ended before phase S and its terminal witness".into());
    }
    Ok(())
}

fn mark_json(mark: Mark) -> Value {
    json!({"game_tick": mark.tick, "client_poll": mark.poll})
}

fn drop_json(drop: &FoodDrop) -> Value {
    json!({"at": mark_json(drop.at), "from": drop.from, "to": drop.to})
}

fn chat_event_json(event: &ChatEvent) -> Value {
    json!({
        "at": mark_json(event.at),
        "sequence": event.sequence,
        "kind": event.kind,
        "text": event.text,
    })
}

fn frame_json(frame: &C0Frame) -> Value {
    json!({
        "at": mark_json(frame.at),
        "hitpoints": frame.hitpoints,
        "lobsters": frame.lobsters,
        "main_modal": frame.main_modal,
        "chat_modal": frame.chat_modal,
        "prompt_open": frame.prompt_open,
        "climb_line": frame.climb_line,
        "slip_line": frame.slip_line,
        "new_climb_line": frame.new_climb_line,
        "new_slip_line": frame.new_slip_line,
        "hit_kind1": frame.hit_kind1,
        "decision": frame.decision.map(|decision| format!("{decision:?}")),
        "survival_owner": frame.survival_owner,
        "native_phase": frame.native_phase,
        "step_id": frame.step_id,
        "stage": frame.stage,
        "npc_facing_or_attacking": frame.npc_facing_or_attacking,
        "tile": frame.tile,
        "local_player": frame.local_player,
        "failure": frame.failure,
    })
}

fn end_json(end: &End) -> Value {
    match end {
        End::Step2(at) => json!({"kind": "step2", "at": mark_json(*at)}),
        End::SlipAfterRestart { slip, observed } => json!({
            "kind": "slip_after_restart_plus_3",
            "slip": mark_json(*slip),
            "observed_until": mark_json(*observed),
        }),
    }
}

#[test]
#[ignore = "requires LIVE=1 and an explicitly selected local 289 Engine Q; measurement only, run before acceptance"]
fn live_quester_upkeep_cell_rock_measurement() {
    assert_eq!(std::env::var("LIVE").as_deref(), Ok("1"));
    assert_eq!(std::env::var("BOT_DEBUG").as_deref(), Ok("1"));
    let _serial = C0_LIVE_TEST_LOCK
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let capture = run_c0(RunPurpose::Measurement)
        .unwrap_or_else(|error| panic!("build C0 measurement cell: {error}"));
    let receipt = capture.receipt.json(capture.purpose, &capture.logs);
    println!("C0-MEASUREMENT-ONLY {receipt:#}");
    let result = capture
        .result
        .unwrap_or_else(|error| panic!("C0 measurement run failed before its receipt: {error}"));
    println!("C0-MEASUREMENT-RUNNER {result:#}");
    assert!(
        capture.receipt.s1.is_some(),
        "measurement did not observe S1"
    );
    assert!(
        capture.receipt.s_drop.is_some(),
        "measurement did not observe phase-S drop"
    );
    assert!(
        capture.receipt.phase_r().is_some(),
        "measurement did not observe phase R or successful step2"
    );
}

#[test]
#[ignore = "requires LIVE=1 and an explicitly selected local 289 Engine Q; run the measurement pass first"]
fn live_quester_upkeep_cell_rock_engine_q() {
    assert_eq!(std::env::var("LIVE").as_deref(), Ok("1"));
    assert_eq!(std::env::var("BOT_DEBUG").as_deref(), Ok("1"));
    let _serial = C0_LIVE_TEST_LOCK
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let capture = run_c0(RunPurpose::Acceptance)
        .unwrap_or_else(|error| panic!("build C0 acceptance cell: {error}"));
    let receipt = capture.receipt.json(capture.purpose, &capture.logs);
    println!("C0-ACCEPTANCE {receipt:#}");
    let result = capture
        .result
        .unwrap_or_else(|error| panic!("C0 acceptance run failed before the oracle: {error}"));
    println!("C0-ACCEPTANCE-RUNNER {result:#}");
    c0_acceptance_oracle(&capture.receipt, &capture.logs)
        .unwrap_or_else(|error| panic!("C0 acceptance oracle: {error}; receipt={receipt:#}"));
}

#[cfg(test)]
mod tests {
    use super::*;
    use client::client::{Client, ClientConfig};
    use client::dash3d::ClientPlayer;
    use client::io::ServerProt;
    use std::sync::Mutex;

    #[test]
    fn reads_actual_scripttrace_stderr_without_inventing_a_carrier_tick() {
        let text = concat!(
            "[script-trace qu1bot] quester survival op event=begin tick=20 kind=Eat { item_id: 379, before_count: 7 } applied=pending deadline=28\n",
            "[script] quester survival op event=begin tick=99 kind=CloseModal applied=pending deadline=107\n",
            "[script-trace] unrelated diagnostic\n",
            "[script-trace qu1bot] quester survival op event=end tick=28 kind=Eat { item_id: 379, before_count: 7 } applied=false deadline=28\n",
        );
        let logs = parse_survival_op_stderr(text);
        assert_eq!(logs.len(), 2);
        assert!(logs.iter().all(|line| line.tick.is_none()));
        let (events, errors) = parse_survival_op_logs(&logs);
        assert!(errors.is_empty());
        assert_eq!(events[0].tick, 20);
        assert_eq!(events[0].event, SurvivalOpEventKind::Begin);
        assert_eq!(events[1].tick, 28);
        assert_eq!(events[1].event, SurvivalOpEventKind::End { applied: false });
        assert!(events.iter().all(|event| event.hostlog_tick.is_none()));
    }

    #[test]
    fn parses_scripttrace_oneop_begin_and_end_records() {
        let begin = HostLog {
            tick: Some(20),
            source: "Script".into(),
            message: format!(
                "quester survival op event=begin tick=20 kind=Eat {{ item_id: {LOBSTER_ID}, before_count: 7 }} applied=pending deadline=28"
            ),
        };
        assert_eq!(
            parse_survival_op_event(&begin),
            Ok(Some(SurvivalOpEvent {
                event: SurvivalOpEventKind::Begin,
                tick: 20,
                kind: format!("Eat {{ item_id: {LOBSTER_ID}, before_count: 7 }}"),
                deadline: 28,
                hostlog_tick: Some(20),
            }))
        );

        let end = HostLog {
            tick: Some(28),
            source: "Script".into(),
            message: format!(
                "quester survival op event=end tick=28 kind=Eat {{ item_id: {LOBSTER_ID}, before_count: 7 }} applied=false deadline=28"
            ),
        };
        assert_eq!(
            parse_survival_op_event(&end),
            Ok(Some(SurvivalOpEvent {
                event: SurvivalOpEventKind::End { applied: false },
                tick: 28,
                kind: format!("Eat {{ item_id: {LOBSTER_ID}, before_count: 7 }}"),
                deadline: 28,
                hostlog_tick: Some(28),
            }))
        );

        let malformed = HostLog {
            message: "quester survival op event=begin tick=bad kind=Eat { item_id: 379, before_count: 7 } applied=pending deadline=28".into(),
            ..begin
        };
        assert!(parse_survival_op_event(&malformed).is_err());
    }

    fn test_op_event(
        event: SurvivalOpEventKind,
        tick: u32,
        before_count: i32,
        deadline: u32,
    ) -> SurvivalOpEvent {
        SurvivalOpEvent {
            event,
            tick: u64::from(tick),
            kind: format!("Eat {{ item_id: {LOBSTER_ID}, before_count: {before_count} }}"),
            deadline: u64::from(deadline),
            hostlog_tick: Some(u64::from(tick)),
        }
    }

    fn valid_receipt(
        g_gap: u32,
        s1_gap: u32,
        s_drop_gap: u32,
        restart_gap: u32,
        o2_drop_gap: u32,
        slip_hp: i32,
    ) -> C0Receipt {
        let r0 = Mark { tick: 12, poll: 12 };
        let g = Mark {
            tick: r0.tick + g_gap,
            poll: 20,
        };
        let prompt = Mark {
            tick: g.tick + 1,
            poll: 30,
        };
        let l0 = Mark {
            tick: prompt.tick + 2,
            poll: 40,
        };
        let s1 = Mark {
            tick: l0.tick + s1_gap,
            poll: 42,
        };
        let s_drop = Mark {
            tick: s1.tick + s_drop_gap,
            poll: 60,
        };
        let first_slip = Mark {
            tick: s1.tick + 3,
            poll: 50,
        };
        let restart = Mark {
            tick: s_drop.tick + restart_gap,
            poll: 90,
        };
        let retry_slip = Mark {
            tick: restart.tick + 5,
            poll: 100,
        };
        let retry_drop = Mark {
            tick: retry_slip.tick + o2_drop_gap,
            poll: 110,
        };
        let end = Mark {
            tick: retry_slip.tick + 3,
            poll: 120,
        };
        let mut receipt = C0Receipt {
            start: Some(Mark { tick: 10, poll: 1 }),
            r0: Some(r0),
            g_drop: Some(FoodDrop {
                at: g,
                from: 8,
                to: 7,
            }),
            l0: Some(l0),
            s_send: Some(Mark {
                tick: l0.tick,
                poll: l0.poll + 1,
            }),
            s1: Some(s1),
            s_drop: Some(FoodDrop {
                at: s_drop,
                from: 7,
                to: 6,
            }),
            restart: Some(restart),
            slip_after_restart: Some(retry_slip),
            end: Some(End::SlipAfterRestart {
                slip: retry_slip,
                observed: end,
            }),
            drops: vec![
                FoodDrop {
                    at: g,
                    from: 8,
                    to: 7,
                },
                FoodDrop {
                    at: s_drop,
                    from: 7,
                    to: 6,
                },
                FoodDrop {
                    at: retry_drop,
                    from: 6,
                    to: 5,
                },
            ],
            slip_events: vec![first_slip, retry_slip],
            survival_op_events: vec![
                test_op_event(SurvivalOpEventKind::Begin, g.tick - 1, 8, g.tick + 7),
                test_op_event(
                    SurvivalOpEventKind::End { applied: true },
                    g.tick,
                    8,
                    g.tick + 7,
                ),
                test_op_event(SurvivalOpEventKind::Begin, s1.tick, 7, s1.tick + 8),
                test_op_event(
                    SurvivalOpEventKind::End { applied: false },
                    s1.tick + 8,
                    7,
                    s1.tick + 8,
                ),
                test_op_event(SurvivalOpEventKind::Begin, s1.tick + 8, 7, s1.tick + 16),
                test_op_event(
                    SurvivalOpEventKind::End { applied: true },
                    s_drop.tick,
                    7,
                    s1.tick + 16,
                ),
                test_op_event(
                    SurvivalOpEventKind::Begin,
                    retry_slip.tick,
                    6,
                    retry_slip.tick + 8,
                ),
                test_op_event(
                    SurvivalOpEventKind::End { applied: true },
                    retry_drop.tick,
                    6,
                    retry_slip.tick + 8,
                ),
            ],
            ..C0Receipt::default()
        };
        receipt.frames = vec![
            C0Frame {
                at: Mark {
                    tick: r0.tick - 1,
                    poll: r0.poll - 1,
                },
                main_modal: JOURNAL_ROOT,
                ..frame(
                    Mark {
                        tick: r0.tick - 1,
                        poll: r0.poll - 1,
                    },
                    8,
                    14,
                )
            },
            frame(r0, 8, 14),
            C0Frame {
                at: prompt,
                prompt_open: true,
                chat_modal: 1,
                ..frame(prompt, 7, 24)
            },
            C0Frame {
                at: l0,
                climb_line: true,
                new_climb_line: true,
                ..frame(l0, 7, 24)
            },
            C0Frame {
                at: s1,
                hitpoints: 14,
                decision: Some(Decision::Eating),
                survival_owner: Some("Step".into()),
                native_phase: Some("Running".into()),
                step_id: Some(CLIMB_STEP.into()),
                stage: Some("desertrescue:6".into()),
                ..frame(s1, 7, 14)
            },
            C0Frame {
                at: first_slip,
                slip_line: true,
                new_slip_line: true,
                hit_kind1: Some(7),
                hitpoints: slip_hp,
                native_phase: Some("Running".into()),
                ..frame(first_slip, 7, slip_hp)
            },
            frame(s_drop, 6, 14),
            C0Frame {
                at: restart,
                prompt_open: true,
                chat_modal: 1,
                ..frame(restart, 6, slip_hp)
            },
            C0Frame {
                at: retry_slip,
                slip_line: true,
                new_slip_line: true,
                hit_kind1: Some(7),
                hitpoints: 20,
                native_phase: Some("Running".into()),
                ..frame(retry_slip, 6, 20)
            },
            frame(retry_drop, 5, 20),
            frame(end, 5, 20),
        ];
        receipt
    }

    fn frame(at: Mark, lobsters: i32, hitpoints: i32) -> C0Frame {
        C0Frame {
            at,
            hitpoints,
            lobsters,
            main_modal: -1,
            chat_modal: -1,
            prompt_open: false,
            climb_line: false,
            slip_line: false,
            new_climb_line: false,
            new_slip_line: false,
            hit_kind1: None,
            decision: None,
            survival_owner: Some("Step".into()),
            native_phase: Some("Running".into()),
            step_id: Some(CLIMB_STEP.into()),
            stage: Some("desertrescue:6".into()),
            npc_facing_or_attacking: false,
            tile: Some([3282, 3037, 0]),
            local_player: true,
            failure: None,
        }
    }

    fn valid_hazard_logs() -> Vec<HostLog> {
        vec![HostLog {
            tick: Some(11),
            source: "Script".into(),
            message: format!(
                "quester hazard step={CLIMB_STEP} max=7 source={HAZARD_SOURCE} line=15"
            ),
        }]
    }

    fn expect_rejected(receipt: &C0Receipt, logs: &[HostLog]) {
        assert!(
            c0_acceptance_oracle(receipt, logs).is_err(),
            "unexpectedly accepted invalid C0 receipt: {:#}",
            receipt.json(RunPurpose::Acceptance, logs)
        );
    }

    #[test]
    fn acceptance_oracle_accepts_the_dropped_first_click_retry_path() {
        let dropped_click_retry = valid_receipt(3, 2, 9, 16, 2, 20);
        c0_acceptance_oracle(&dropped_click_retry, &valid_hazard_logs()).unwrap();
    }

    #[test]
    fn acceptance_oracle_requires_the_retry_at_the_unapplied_deadline() {
        let mut receipt = valid_receipt(3, 2, 9, 16, 2, 20);
        let retry_tick = u64::from(receipt.s1.unwrap().tick + 8);
        let retry_index = receipt
            .survival_op_events
            .iter()
            .position(|event| {
                event.event == SurvivalOpEventKind::Begin
                    && event.tick == retry_tick
                    && parse_eat_kind(&event.kind) == Some((LOBSTER_ID, 7))
            })
            .expect("receipt has the phase-S retry begin");
        receipt.survival_op_events.remove(retry_index);
        let error = c0_acceptance_oracle(&receipt, &valid_hazard_logs()).unwrap_err();
        assert!(
            error.contains("retry must begin at rejected-operation tick"),
            "unexpected failure: {error}"
        );
    }

    #[test]
    fn accepted_first_click_is_reported_separately_and_rejected_as_c0_proof() {
        let mut accepted_first_click = valid_receipt(3, 2, 1, 16, 2, 20);
        let l0 = accepted_first_click.l0.unwrap();
        let s1 = accepted_first_click.s1.unwrap();
        let s_drop = accepted_first_click.s_drop.as_ref().unwrap().at;
        assert_eq!(s1.gap(l0), 2);
        assert_eq!(s_drop.gap(s1), 1);

        accepted_first_click
            .survival_op_events
            .retain(|event| parse_eat_kind(&event.kind) != Some((LOBSTER_ID, 7)));
        let o2_index = accepted_first_click
            .survival_op_events
            .iter()
            .position(|event| parse_eat_kind(&event.kind) == Some((LOBSTER_ID, 6)))
            .expect("fixture has O2 operation evidence");
        accepted_first_click.survival_op_events.insert(
            o2_index,
            test_op_event(SurvivalOpEventKind::Begin, s1.tick, 7, s1.tick + 8),
        );
        accepted_first_click.survival_op_events.insert(
            o2_index + 1,
            test_op_event(
                SurvivalOpEventKind::End { applied: true },
                s_drop.tick,
                7,
                s1.tick + 8,
            ),
        );
        accepted_first_click
            .frames
            .iter_mut()
            .find(|frame| frame.new_slip_line)
            .expect("fixture has a slip frame before restart")
            .lobsters = 6;
        accepted_first_click
            .frames
            .sort_by_key(|frame| frame.at.tick);
        let error = c0_acceptance_oracle(&accepted_first_click, &valid_hazard_logs()).unwrap_err();
        assert!(
            error.contains("phase S drop was 1 ticks after S1"),
            "unexpected failure: {error}"
        );
    }

    #[test]
    fn acceptance_oracle_checks_g_s_r_and_o2_boundary_ticks() {
        let g_boundary = valid_receipt(3, 2, 9, 16, 2, 20);
        c0_acceptance_oracle(&g_boundary, &valid_hazard_logs()).unwrap();

        let s_boundary = valid_receipt(3, 2, 12, 16, 2, 20);
        c0_acceptance_oracle(&s_boundary, &valid_hazard_logs()).unwrap();

        let o2_boundary = valid_receipt(3, 2, 9, 16, 2, 9);
        c0_acceptance_oracle(&o2_boundary, &valid_hazard_logs()).unwrap();
    }

    #[test]
    fn acceptance_oracle_fails_closed_without_o2_operation_evidence() {
        let mut receipt = valid_receipt(3, 2, 9, 16, 2, 20);
        receipt
            .survival_op_events
            .retain(|event| parse_eat_kind(&event.kind) != Some((LOBSTER_ID, 6)));
        let error = c0_acceptance_oracle(&receipt, &valid_hazard_logs()).unwrap_err();
        assert!(error.starts_with("O2:"), "unexpected failure: {error}");
    }

    #[test]
    fn acceptance_oracle_rejects_g_outside_r0_and_before_the_prompt() {
        let mut before_r0 = valid_receipt(1, 2, 9, 16, 2, 20);
        before_r0.drops[0].at.tick = before_r0.r0.unwrap().tick - 1;
        before_r0.g_drop = Some(before_r0.drops[0].clone());
        expect_rejected(&before_r0, &valid_hazard_logs());

        let mut late_g = valid_receipt(4, 2, 9, 16, 2, 20);
        late_g
            .frames
            .iter_mut()
            .find(|frame| frame.prompt_open)
            .unwrap()
            .at
            .tick += 2;
        expect_rejected(&late_g, &valid_hazard_logs());

        let mut prompt_before_g = valid_receipt(1, 2, 9, 16, 2, 20);
        let prompt = prompt_before_g
            .frames
            .iter_mut()
            .find(|frame| frame.prompt_open)
            .unwrap();
        prompt.at.tick = prompt_before_g.drops[0].at.tick - 1;
        expect_rejected(&prompt_before_g, &valid_hazard_logs());
    }

    #[test]
    fn acceptance_oracle_rejects_s1_timing_and_phase_s_drop_edges() {
        let wrong_in_delay_click = valid_receipt(3, 2, 1, 16, 2, 20);
        expect_rejected(&wrong_in_delay_click, &valid_hazard_logs());

        let middle_drop = valid_receipt(3, 2, 5, 16, 2, 20);
        expect_rejected(&middle_drop, &valid_hazard_logs());

        let late_drop = valid_receipt(3, 2, 13, 16, 2, 20);
        expect_rejected(&late_drop, &valid_hazard_logs());

        let late_s1 = valid_receipt(3, 5, 1, 16, 2, 20);
        expect_rejected(&late_s1, &valid_hazard_logs());
    }

    #[test]
    fn acceptance_oracle_rejects_o3_begin_restart_and_o2_failures() {
        let mut high_hp_eating = valid_receipt(3, 2, 9, 16, 2, 20);
        let mut frame = frame(
            Mark {
                tick: 22,
                poll: 500,
            },
            6,
            16,
        );
        frame.decision = Some(Decision::Eating);
        high_hp_eating.frames.push(frame);
        expect_rejected(&high_hp_eating, &valid_hazard_logs());

        let mut begun_between = valid_receipt(3, 2, 9, 16, 2, 20);
        let s1 = begun_between.s1.unwrap();
        begun_between
            .drops
            .iter_mut()
            .find(|drop| drop.from == 7 && drop.to == 6)
            .unwrap()
            .at
            .tick = s1.tick + 10;
        begun_between.s_drop = Some(
            begun_between
                .drops
                .iter()
                .find(|drop| drop.from == 7 && drop.to == 6)
                .unwrap()
                .clone(),
        );
        let s_drop_tick = s1.tick + 10;
        let s_drop_event = begun_between
            .survival_op_events
            .iter_mut()
            .find(|event| {
                event.event == SurvivalOpEventKind::End { applied: true }
                    && event.tick == u64::from(s1.tick + 9)
                    && parse_eat_kind(&event.kind) == Some((LOBSTER_ID, 7))
            })
            .expect("receipt has the applied phase-S end");
        s_drop_event.tick = u64::from(s_drop_tick);
        s_drop_event.hostlog_tick = Some(u64::from(s_drop_tick));
        let s_drop_frame = begun_between
            .frames
            .iter_mut()
            .find(|frame| frame.at.tick == s1.tick + 9 && frame.lobsters == 6)
            .expect("receipt has the phase-S drop frame");
        s_drop_frame.at.tick = s_drop_tick;
        expect_rejected(
            &begun_between,
            &[valid_hazard_logs()[0].clone(), HostLog {
                tick: Some(u64::from(s1.tick + 1)),
                source: "Script".into(),
                message: format!("quester desertrescue: stage desertrescue:6 step {CLIMB_STEP} (interact) begin"),
            }],
        );

        let restart_too_late = valid_receipt(3, 2, 9, 17, 2, 20);
        expect_rejected(&restart_too_late, &valid_hazard_logs());

        let o2_late = valid_receipt(3, 2, 9, 16, 3, 9);
        expect_rejected(&o2_late, &valid_hazard_logs());
    }

    #[test]
    fn acceptance_oracle_rejects_death_missing_hazard_and_pre_phase_s_end() {
        let mut died = valid_receipt(3, 2, 9, 16, 2, 20);
        died.death = true;
        expect_rejected(&died, &valid_hazard_logs());

        let no_hazard = valid_receipt(3, 2, 9, 16, 2, 20);
        expect_rejected(&no_hazard, &[]);

        let mut early_end = valid_receipt(3, 2, 9, 16, 2, 20);
        early_end.s1 = None;
        early_end.s_drop = None;
        early_end.end = None;
        expect_rejected(&early_end, &valid_hazard_logs());

        let retry = valid_receipt(3, 2, 9, 16, 2, 20);
        let mut logs = valid_hazard_logs();
        logs.push(HostLog {
            tick: Some(15),
            source: "Script".into(),
            message: "quester desertrescue: journal retry quiet period unavailable".into(),
        });
        expect_rejected(&retry, &logs);
    }

    #[test]
    fn await_then_perform_uses_the_next_client_cycle_at_the_same_game_tick() {
        let mut client = seeded_client();
        let anchor = Arc::new(Mutex::new(None::<u32>));
        let send_tick = Arc::new(Mutex::new(None::<u32>));
        let ready_anchor = Arc::clone(&anchor);
        let perform_tick = Arc::clone(&send_tick);
        let scenario = scenario::Scenario {
            name: "same-game-tick-await-perform",
            seed: Seed {
                profiles: vec![("test", "test")],
                mainland: false,
            },
            steps: vec![
                Step {
                    name: "await first client cycle",
                    kind: StepKind::Await {
                        evidence: "snapshot tick captured",
                        ready: Box::new(move |snapshot| {
                            if let Ok(mut anchor) = ready_anchor.lock() {
                                anchor.get_or_insert(snapshot.tick());
                            }
                            true
                        }),
                        summary: None,
                    },
                    wait: Wait {
                        arm: Proof::IngameScene2,
                        budget_ticks: 1,
                    },
                },
                Step {
                    name: "perform on the next client cycle",
                    kind: StepKind::Perform {
                        send: Box::new(move |_, snapshot| {
                            if let Ok(mut tick) = perform_tick.lock() {
                                *tick = Some(snapshot.tick());
                                true
                            } else {
                                false
                            }
                        }),
                    },
                    wait: Wait {
                        arm: Proof::IngameScene2,
                        budget_ticks: 1,
                    },
                },
            ],
            proof: Proof::IngameScene2,
            companions: vec![],
            settings: ScenarioSettings::default(),
        };
        let mut runner = ScenarioRunner::new(scenario);
        runner.set_scene_settle(Duration::ZERO);

        runner.tick(&mut client);
        let await_tick = anchor.lock().unwrap().expect("Await recorded its snapshot");
        assert!(
            send_tick.lock().unwrap().is_none(),
            "Perform must wait for the next client cycle"
        );

        runner.tick(&mut client);
        let perform_tick = send_tick.lock().unwrap().expect("Perform ran on cycle two");
        assert_eq!(
            perform_tick, await_tick,
            "the client cycle did not advance the game tick"
        );
    }

    fn seeded_client() -> Client {
        let mut client = Client::new(ClientConfig {
            host: "127.0.0.1".into(),
            port: 43594,
            cache_dir: "/tmp/quester-upkeep-no-cache".into(),
            members: true,
            lowmem: false,
        });
        client.ingame = true;
        client.scene_state = 2;
        client.map_build_base_x = 3200;
        client.map_build_base_z = 3200;
        client.local_player = Some(ClientPlayer::at(20, 20));
        for prot in [
            ServerProt::PLAYER_INFO,
            ServerProt::REBUILD_NORMAL,
            ServerProt::UPDATE_STAT,
        ] {
            client.bump_gens(prot);
        }
        client
    }

    fn _assert_fixture_profile_row_is_stable(row: QuestFixtureProfile) {
        assert_eq!(row.quest, QUEST);
        assert_eq!(row.display, DISPLAY);
        assert_eq!(row.profile, TestProfile::Base40);
    }

    #[test]
    fn fixture_is_account_owned_base40_stage_six_with_eight_lobsters() {
        let seeds: QuesterPathSeeds =
            parse_quester_path_seeds(PATH_SEEDS, "desertrescue-cell-rock")
                .expect("C0 fixture parses");
        assert_eq!(seeds.stage, "desertrescue:6");
        assert_eq!(seeds.stand, [3282, 3035, 0]);
        assert!(seeds.loadout.is_none());
        assert_eq!(seeds.extra_items, vec![("lobster".to_owned(), 8)]);
        assert!(seeds.seed_vars.is_empty());
        assert!(seeds.before_relog.is_empty());
        let row = fixture_row(QUEST).expect("desertrescue fixture row");
        _assert_fixture_profile_row_is_stable(row);
    }
}
