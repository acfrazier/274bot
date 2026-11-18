use crate::{prelude::PRELUDE, single_step};
use api::identity::ExecutionId;
use mlua::chunk::{ChunkMode, Compiler};
use mlua::thread::ThreadStatus;
use mlua::{Error, Function, Lua, LuaOptions, MultiValue, StdLib, Table, Thread, Value};
#[cfg(feature = "test-hooks")]
use script::api_progress::ProgressPage;
use script::native::recovery_hints::RecoveryHintsCell;
#[cfg(feature = "test-hooks")]
use script::native::request::NativeKind;
#[cfg(feature = "test-hooks")]
use script::native::seat::SeatTerminal;
use script::native::seat_rows::QueuedSeatBatch;
#[cfg(feature = "test-hooks")]
use script::native::seat_rows::SeatStartArgs;
use script::native::snapshot_source::{NativeFactsInput, SnapshotInput};
#[cfg(feature = "test-hooks")]
use script::native::WalkCancelReason;
use script::native::{ScriptFailure, SettingsBag};
use script::surface::client::SeatClient;
#[cfg(feature = "test-hooks")]
use script::surface::client::SeatCompletion;
use script::surface::{
    InputDelivery, JournalWait, LentOutcome, PostedSnapshotFacts, RouteInspectIdentities,
    ScriptRuntimeContext, ScriptStopReceipt, StopCompletion, SurfaceCard, SurfaceError,
    SurfaceFactory, SurfaceFamily, SurfaceKind, SurfaceReady, SurfaceRun, SurfaceStart,
    SurfaceWake, TickOutcome,
};
use script::{
    interact::{InteractReq, QueuedInteract},
    paint::ScriptPaint,
};
use std::any::Any;
use std::collections::VecDeque;
use std::fmt::{self, Write as _};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering};
use std::sync::{mpsc::Sender, Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

const SOURCE_LIMIT_BYTES: usize = 256 * 1024;
const SOFT_MEMORY_LIMIT_BYTES: usize = 16 * 1024 * 1024;
const HARD_MEMORY_LIMIT_BYTES: usize = 17 * 1024 * 1024;
const DISPATCH_BUDGET: Duration = Duration::from_millis(600);
const LOAD_BUDGET: Duration = Duration::from_millis(50);
const HOOK_BUDGET: Duration = Duration::from_millis(50);
const NO_DEADLINE: u64 = u64::MAX;
const MEMORY_ERROR: &[u8] = b"not enough memory";
const LOST_ERROR: &str = "error in error handling";
const MAX_ERROR_BYTES: usize = 8 * 1024;
const STOP_REASON_MAX_BYTES: usize = 256;
const MAX_LOG_LINE_BYTES: usize = 4 * 1024;
const MAX_LOG_ARGS: usize = 256;
const MAX_LOG_LINES_PER_ENTRY: usize = 256;
const MAX_LOG_BYTES_PER_ENTRY: usize = 64 * 1024;
const MAX_LOG_QUEUE_LINES: usize = 1024;
const MAX_LOG_QUEUE_BYTES: usize = 256 * 1024;
const MAX_TICK_OUTCOMES: usize = 256;

const GLOBAL_ALLOWLIST: &[&str] = &[
    "api",
    "assert",
    "bit32",
    "buffer",
    "error",
    "getmetatable",
    "ipairs",
    "math",
    "next",
    "pairs",
    "pcall",
    "print",
    "rawequal",
    "rawget",
    "rawlen",
    "rawset",
    "select",
    "setmetatable",
    "string",
    "table",
    "tonumber",
    "tostring",
    "type",
    "typeof",
    "utf8",
    "xpcall",
];

// These are the input-dependent fast calls observed in the pinned Luau census.
const DISABLED_BUILTINS: &[&str] = &[
    "assert",
    "bit32.band",
    "bit32.bor",
    "bit32.btest",
    "bit32.bxor",
    "getmetatable",
    "math.max",
    "math.min",
    "rawget",
    "rawlen",
    "rawset",
    "select",
    "setmetatable",
    "string.byte",
    "string.char",
    "string.sub",
    "table.insert",
    "table.unpack",
    "tonumber",
    "tostring",
];

const LATCH_EMPTY: u8 = 0;
const LATCH_SELF_STOP: u8 = 1;
const LATCH_WATCHDOG: u8 = 2;
const LATCH_MEMORY: u8 = 3;
const LATCH_INTERNAL: u8 = 4;

const PHASE_SETUP: u8 = 0;
const PHASE_LOAD: u8 = 1;
const PHASE_TICK: u8 = 2;
const PHASE_PAINT: u8 = 3;
const PHASE_HOOK: u8 = 4;

/// Frozen source and card identity for one admitted Luau start.
#[derive(Debug)]
pub struct LuauStartCard {
    pub card_name: Arc<str>,
    pub source: Arc<str>,
}

impl LuauStartCard {
    pub fn new(card_name: impl Into<Arc<str>>, source: impl Into<Arc<str>>) -> Self {
        Self {
            card_name: card_name.into(),
            source: source.into(),
        }
    }
}

impl SurfaceCard for LuauStartCard {
    fn kind(&self) -> SurfaceKind {
        SurfaceKind::Luau
    }

    fn family(&self) -> Option<SurfaceFamily> {
        Some(SurfaceFamily::LuauV2)
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Creates one synchronous Luau run per admitted Start.
#[derive(Default)]
pub struct LuauSurfaceFactory;

impl SurfaceFactory for LuauSurfaceFactory {
    fn kind(&self) -> SurfaceKind {
        SurfaceKind::Luau
    }

    fn spawn(
        &self,
        start: &SurfaceStart,
        execution: ExecutionId,
        _recovery_hints: Arc<RecoveryHintsCell>,
    ) -> Result<Box<dyn SurfaceRun>, SurfaceError> {
        self.spawn_with_state(start, execution, _recovery_hints, None)
    }

    fn spawn_with_state(
        &self,
        start: &SurfaceStart,
        execution: ExecutionId,
        _recovery_hints: Arc<RecoveryHintsCell>,
        _local_state: Option<Arc<script::local_state::SlotState>>,
    ) -> Result<Box<dyn SurfaceRun>, SurfaceError> {
        match catch_unwind(AssertUnwindSafe(|| self.spawn_inner(start, execution))) {
            Ok(result) => result,
            Err(_) => Err(SurfaceError::Spawn(
                "luau-internal: the VM failed while starting the script".to_owned(),
            )),
        }
    }
}

impl LuauSurfaceFactory {
    fn spawn_inner(
        &self,
        start: &SurfaceStart,
        execution: ExecutionId,
    ) -> Result<Box<dyn SurfaceRun>, SurfaceError> {
        let card = start
            .card
            .as_any()
            .downcast_ref::<LuauStartCard>()
            .ok_or(SurfaceError::ForeignCard(SurfaceKind::Luau))?;
        if card.source.len() > SOURCE_LIMIT_BYTES {
            return Err(SurfaceError::Spawn(format!(
                "luau-load: source exceeds {SOURCE_LIMIT_BYTES} bytes"
            )));
        }

        let state = Arc::new(RunState::new(
            Arc::new(start.settings_bag.as_deref().cloned().unwrap_or_default()),
            SeatClient::new(execution),
        ));

        let lua = Lua::new_with(
            StdLib::COROUTINE
                | StdLib::TABLE
                | StdLib::STRING
                | StdLib::UTF8
                | StdLib::BIT
                | StdLib::MATH
                | StdLib::BUFFER,
            LuaOptions::default(),
        )
        .map_err(|error| spawn_error("luau-create", &error))?;
        lua.set_memory_limit(HARD_MEMORY_LIMIT_BYTES)
            .map_err(|error| spawn_error("luau-create", &error))?;

        let interrupt_state = Arc::clone(&state);
        lua.set_interrupt(move |lua| {
            interrupt_state.check_lua(lua)?;
            Ok(mlua::VmState::Continue)
        });
        single_step::enable(&lua).map_err(|error| spawn_error("luau-internal", &error))?;

        let api = build_api(&lua, &state).map_err(|error| spawn_error("luau-setup", &error))?;
        freeze_api(&api).map_err(|error| spawn_error("luau-setup", &error))?;
        lua.globals()
            .raw_set("api", api.clone())
            .map_err(|error| spawn_error("luau-setup", &error))?;
        retain_allowed_globals(&lua).map_err(|error| spawn_error("luau-setup", &error))?;
        lua.sandbox(true)
            .map_err(|error| spawn_error("luau-setup", &error))?;

        let compiler = Compiler::new()
            .set_mutable_globals(["pcall", "xpcall"])
            .set_disabled_builtins(DISABLED_BUILTINS.iter().copied());
        let bytecode = compiler
            .compile(card.source.as_bytes())
            .map_err(|error| spawn_error("luau-compile", &error))?;
        if bytecode.first() != Some(&9) {
            return Err(SurfaceError::Spawn(format!(
                "luau-compile: compiler emitted bytecode version {:?}, expected 9",
                bytecode.first()
            )));
        }
        let load_function = lua
            .load(&bytecode)
            .set_name(card.card_name.as_ref())
            .set_mode(ChunkMode::Binary)
            .into_function()
            .map_err(|error| spawn_error("luau-compile", &error))?;
        let load_thread = lua
            .create_thread(load_function)
            .map_err(|error| spawn_error("luau-load", &error))?;

        state.set_deadline(LOAD_BUDGET);
        let loaded = execute_entry::<Value>(&lua, &state, &load_thread, (), PHASE_LOAD, false);
        state.clear_deadline();
        let loaded = match loaded {
            EntryOutcome::Returned(value) if load_thread.status() == ThreadStatus::Finished => {
                value
            }
            EntryOutcome::Returned(_) | EntryOutcome::Yielded => {
                return Err(SurfaceError::Spawn(
                    "luau-load: the top-level load entry yielded".to_owned(),
                ));
            }
            EntryOutcome::UserError(message) => {
                return Err(SurfaceError::Spawn(format!("luau-load: {message}")));
            }
            EntryOutcome::SelfStop | EntryOutcome::InternalAfterSelfStop => {
                return Err(SurfaceError::Spawn(
                    "luau-load: api.stop is not available while loading".to_owned(),
                ));
            }
            EntryOutcome::Trip(failure) => return Err(surface_error(failure)),
        };
        let entries = match loaded {
            Value::Table(table) => table,
            _ => {
                return Err(SurfaceError::Spawn(
                    "luau-load: source must return a table with a tick function".to_owned(),
                ));
            }
        };
        let tick_function = entries
            .get::<Function>("tick")
            .map_err(|error| spawn_error("luau-load", &error))?;
        let on_paint = optional_entry(&entries, "onPaint")?;
        let on_stop = optional_entry(&entries, "onStop")?;
        let tick_thread = lua
            .create_thread(tick_function.clone())
            .map_err(|error| spawn_error("luau-load", &error))?;
        let paint_thread = on_paint
            .as_ref()
            .map(|function| lua.create_thread(function.clone()))
            .transpose()
            .map_err(|error| spawn_error("luau-load", &error))?;
        let stop_thread = on_stop
            .as_ref()
            .map(|function| lua.create_thread(function.clone()))
            .transpose()
            .map_err(|error| spawn_error("luau-load", &error))?;

        Ok(Box::new(LuauSurfaceRun {
            vm: Some(ActiveVm {
                lua,
                api,
                tick_function,
                tick_thread,
                tick_needs_reset: false,
                tick_suspended: false,
                paint_function: on_paint,
                paint_thread,
                paint_needs_reset: false,
                stop_function: on_stop,
                stop_thread,
            }),
            state,
            generation: AtomicU64::new(execution.generation().get()),
            last_snapshot_facts: None,
        }))
    }
}

fn optional_entry(table: &Table, name: &str) -> Result<Option<Function>, SurfaceError> {
    match table
        .raw_get::<Value>(name)
        .map_err(|error| spawn_error("luau-load", &error))?
    {
        Value::Nil => Ok(None),
        Value::Function(function) => Ok(Some(function)),
        _ => Err(SurfaceError::Spawn(format!(
            "luau-load: `{name}` must be a function when present"
        ))),
    }
}

fn spawn_error(prefix: &str, error: &Error) -> SurfaceError {
    SurfaceError::Spawn(format!("{prefix}: {}", bounded_display(error)))
}

fn surface_error(failure: ScriptFailure) -> SurfaceError {
    SurfaceError::Spawn(format!("{}: {}", failure.code, failure.message))
}

fn build_api(lua: &Lua, state: &Arc<RunState>) -> mlua::Result<Table> {
    let host = lua.create_table()?;
    host.set(
        "guard",
        host_function(lua, state, |lua, value: Value, state| {
            if let Value::String(message) = value {
                if message.as_bytes().as_ref() == MEMORY_ERROR {
                    state.trip(LATCH_MEMORY);
                }
            }
            match state.check_lua(lua) {
                Ok(()) => Ok((true, Option::<String>::None)),
                Err(error) => Ok((false, Some(bounded_display(&error)))),
            }
        })?,
    )?;
    host.set(
        "print",
        host_function(
            lua,
            state,
            |_, values: MultiValue, state| match append_log_values(state, &values) {
                Ok(()) => Ok((true, Option::<String>::None)),
                Err(message) => Ok((false, Some(message.to_owned()))),
            },
        )?,
    )?;
    host.set(
        "format",
        host_function(
            lua,
            state,
            |_, values: MultiValue, _| match validate_format_arguments(&values) {
                Ok(()) => Ok((true, Option::<String>::None)),
                Err(message) => Ok((false, Some(message.to_owned()))),
            },
        )?,
    )?;

    let bindings = lua.create_table()?;
    bindings.set(
        "log",
        host_function(
            lua,
            state,
            |_, values: MultiValue, state| match append_log_values(state, &values) {
                Ok(()) => Ok((true, Option::<String>::None)),
                Err(message) => Ok((false, Some(message.to_owned()))),
            },
        )?,
    )?;
    bindings.set(
        "stop",
        host_function(lua, state, |_, values: MultiValue, state| {
            if !matches!(state.phase(), PHASE_TICK | PHASE_PAINT) {
                return Ok((
                    false,
                    Some("api.stop is only available during tick or paint".to_owned()),
                ));
            }
            let reason = match stop_reason(&values) {
                Ok(reason) => reason,
                Err(message) => return Ok((false, Some(message.to_owned()))),
            };
            state.write_self_stop(reason);
            Ok((false, Some("Luau script stopped itself".to_owned())))
        })?,
    )?;
    bindings.set(
        "wait",
        host_function(lua, state, |_, _: MultiValue, state| {
            if state.phase() != PHASE_TICK {
                return Ok((
                    false,
                    Some("api.wait is only available during tick".to_owned()),
                ));
            }
            let mut client = state.client_lock()?;
            match client.await_next_tick() {
                Ok(()) => {
                    state.wait_armed.store(true, Ordering::Relaxed);
                    Ok((true, Option::<String>::None))
                }
                Err(error) => Ok((false, Some(bounded_display(&error)))),
            }
        })?,
    )?;

    let settings = lua.create_table()?;
    settings.set(
        "str",
        host_function(lua, state, |lua, values: MultiValue, state| {
            let key = match string_argument(&values, 0, "api.settings.str") {
                Ok(key) => key,
                Err(message) => {
                    return Ok((false, Value::String(lua.create_string(message)?)));
                }
            };
            let bag = lock(&state.settings);
            let value = match bag.get(key.as_ref()).and_then(serde_json::Value::as_str) {
                Some(value) => Value::String(lua.create_string(value)?),
                None => Value::Nil,
            };
            Ok((true, value))
        })?,
    )?;
    settings.set(
        "num",
        host_function(lua, state, |lua, values: MultiValue, state| {
            let key = match string_argument(&values, 0, "api.settings.num") {
                Ok(key) => key,
                Err(message) => return Ok((false, Value::String(lua.create_string(message)?))),
            };
            let bag = lock(&state.settings);
            let value = bag
                .get(key.as_ref())
                .and_then(serde_json::Value::as_f64)
                .map(Value::Number)
                .unwrap_or(Value::Nil);
            Ok((true, value))
        })?,
    )?;
    settings.set(
        "bool",
        host_function(lua, state, |lua, values: MultiValue, state| {
            let key = match string_argument(&values, 0, "api.settings.bool") {
                Ok(key) => key,
                Err(message) => return Ok((false, Value::String(lua.create_string(message)?))),
            };
            let bag = lock(&state.settings);
            let value = bag
                .get(key.as_ref())
                .and_then(serde_json::Value::as_bool)
                .map(Value::Boolean)
                .unwrap_or(Value::Nil);
            Ok((true, value))
        })?,
    )?;
    bindings.set("settings", settings)?;

    #[cfg(feature = "test-hooks")]
    {
        let test = lua.create_table()?;
        test.set(
            "advance",
            host_function(lua, state, |_, values: MultiValue, state| {
                match (values.len(), values.front()) {
                    (1, Some(Value::Integer(milliseconds))) if *milliseconds >= 0 => {
                        state.advance_test_clock((*milliseconds as u64).saturating_mul(1_000_000));
                        Ok((true, Option::<String>::None))
                    }
                    (1, Some(Value::Number(milliseconds)))
                        if milliseconds.is_finite() && *milliseconds >= 0.0 =>
                    {
                        let nanos =
                            (*milliseconds * 1_000_000.0).min(u64::MAX as f64).round() as u64;
                        state.advance_test_clock(nanos);
                        Ok((true, Option::<String>::None))
                    }
                    _ => Ok((
                        false,
                        Some("test clock advance expects nonnegative milliseconds".to_owned()),
                    )),
                }
            })?,
        )?;
        test.set(
            "awaitSeat",
            host_function(lua, state, |_, values: MultiValue, state| {
                if !values.is_empty() {
                    return Ok((
                        false,
                        Some("api.__test.awaitSeat expects no arguments".to_owned()),
                    ));
                }
                if state.phase() != PHASE_TICK {
                    return Ok((
                        false,
                        Some("api.__test.awaitSeat is only available during tick".to_owned()),
                    ));
                }
                let id = {
                    let mut client = state.client_lock()?;
                    match client.start_awaited(NativeKind::Prayer, SeatStartArgs::Empty, None) {
                        Ok(id) => id,
                        Err(error) => return Ok((false, Some(bounded_display(&error)))),
                    }
                };
                state.test_wait_token.store(id.get(), Ordering::Relaxed);
                state
                    .test_waiting_for_completion
                    .store(true, Ordering::Relaxed);
                state.wait_armed.store(true, Ordering::Relaxed);
                Ok((true, Option::<String>::None))
            })?,
        )?;
        test.set(
            "awaitWalk",
            host_function(lua, state, |_, values: MultiValue, state| {
                if !values.is_empty() {
                    return Ok((
                        false,
                        Some("api.__test.awaitWalk expects no arguments".to_owned()),
                    ));
                }
                if state.phase() != PHASE_TICK {
                    return Ok((
                        false,
                        Some("api.__test.awaitWalk is only available during tick".to_owned()),
                    ));
                }
                let request = InteractReq::Walk {
                    x: 0,
                    z: 0,
                    level: 0,
                    allow_teleports: false,
                    allow_wilderness: false,
                    allow_bank_fetch: false,
                    allow_prereq_dialogue: None,
                    allow_quest_start: None,
                    request_id: 0,
                    avoid: Vec::new(),
                    cross: Vec::new(),
                };
                let baseline = state.test_user_move_intent_seq.load(Ordering::Relaxed);
                let expected_generation = state.test_walk_generation.load(Ordering::Relaxed);
                let id = {
                    let mut client = state.client_lock()?;
                    match client.queue_walk_with_baseline_for_test(
                        request,
                        expected_generation,
                        baseline,
                    ) {
                        Ok(id) => id,
                        Err(error) => return Ok((false, Some(bounded_display(&error)))),
                    }
                };
                state.test_wait_token.store(id.get(), Ordering::Relaxed);
                state
                    .test_waiting_for_completion
                    .store(true, Ordering::Relaxed);
                state.wait_armed.store(true, Ordering::Relaxed);
                Ok((true, Option::<String>::None))
            })?,
        )?;
        test.set(
            "awaitProgress",
            host_function(lua, state, |_, values: MultiValue, state| {
                if !values.is_empty() {
                    return Ok((
                        false,
                        Some("api.__test.awaitProgress expects no arguments".to_owned()),
                    ));
                }
                if state.phase() != PHASE_TICK {
                    return Ok((
                        false,
                        Some("api.__test.awaitProgress is only available during tick".to_owned()),
                    ));
                }
                let args = SeatStartArgs::QuestProgress {
                    name: "test-hook".to_owned(),
                    journal_only: false,
                };
                let id = {
                    let mut client = state.client_lock()?;
                    match client.start_awaited(NativeKind::QuestProgress, args, None) {
                        Ok(id) => id,
                        Err(error) => return Ok((false, Some(bounded_display(&error)))),
                    }
                };
                state.test_wait_token.store(id.get(), Ordering::Relaxed);
                state
                    .test_waiting_for_completion
                    .store(true, Ordering::Relaxed);
                state.wait_armed.store(true, Ordering::Relaxed);
                Ok((true, Option::<String>::None))
            })?,
        )?;
        bindings.set("__test", test)?;
    }

    let waiting = lua.create_table()?;
    waiting.set("wait", true)?;
    #[cfg(feature = "test-hooks")]
    {
        let test_waiting = lua.create_table()?;
        test_waiting.set("awaitSeat", true)?;
        test_waiting.set("awaitWalk", true)?;
        test_waiting.set("awaitProgress", true)?;
        waiting.set("__test", test_waiting)?;
    }
    host.set("bindings", bindings)?;
    host.set("waiting", waiting)?;
    let setup: Function = lua.load(PRELUDE).eval()?;
    let api: Table = setup.call(host)?;
    Ok(api)
}

#[cfg(feature = "test-hooks")]
fn test_completion_table(
    lua: &Lua,
    completion: SeatCompletion,
    request_token: u64,
) -> mlua::Result<Table> {
    let result = lua.create_table()?;
    match completion {
        SeatCompletion::Terminal(terminal) => {
            result.set("kind", "terminal")?;
            result.set("token", terminal.id.get())?;
            result.set("sequence", terminal.seq)?;
            result.set("prior_parent", terminal.prior_parent.map(|id| id.get()))?;
            result.set("session", terminal.session.map(|id| id.get()))?;
            result.set("authority_revoked", terminal.authority_revoked)?;
            match terminal.outcome {
                SeatTerminal::Done => result.set("status", "done")?,
                SeatTerminal::Failed => result.set("status", "failed")?,
                SeatTerminal::Refused(reason) => {
                    result.set("status", "refused")?;
                    result.set("reason", format!("{reason:?}"))?;
                }
                SeatTerminal::Aborted(reason) => {
                    result.set("status", "aborted")?;
                    result.set("reason", format!("{reason:?}"))?;
                }
            }
        }
        SeatCompletion::Gather(end) => {
            result.set("kind", "gather")?;
            result.set("token", end.token())?;
            result.set("status", format!("{end:?}"))?;
        }
        SeatCompletion::Combat(end) => {
            result.set("kind", "combat")?;
            result.set("token", end.token())?;
            result.set("status", format!("{end:?}"))?;
        }
        SeatCompletion::Progress(page) => {
            result.set("token", page.token())?;
            match page {
                ProgressPage::Reading { .. } => {
                    result.set("kind", "progress")?;
                    result.set("status", "reading")?;
                }
                ProgressPage::Done { .. } => {
                    result.set("kind", "progress")?;
                    result.set("status", "done")?;
                }
                ProgressPage::Refused { reason, .. } => {
                    result.set("kind", "progress")?;
                    result.set("status", "refused")?;
                    result.set("reason", reason.as_ref())?;
                }
                ProgressPage::JournalDone { result: page, .. } => {
                    result.set("kind", "progress")?;
                    result.set("status", "done")?;
                    result.set("has_result", page.is_some())?;
                }
            }
        }
        SeatCompletion::Walk(walk) => {
            result.set("kind", "walk")?;
            result.set("token", walk.id.get())?;
            result.set("status", if walk.failed { "failed" } else { "done" })?;
            result.set("generation", walk.generation)?;
            result.set("sequence", walk.sequence)?;
            result.set("failed", walk.failed)?;
            result.set("x", walk.x)?;
            result.set("z", walk.z)?;
            result.set("level", walk.level)?;
            result.set("radius", walk.radius)?;
            result.set("blocked", walk.blocked)?;
            result.set("no_path", walk.no_path)?;
            result.set("allow_teleports", walk.allow_teleports)?;
            result.set(
                "reason",
                match walk.cancel_reason {
                    WalkCancelReason::None => "none",
                    WalkCancelReason::UserInput => "user_input",
                },
            )?;
        }
        SeatCompletion::NextTick => {
            result.set("kind", "next_tick")?;
            result.set("token", request_token)?;
            result.set("status", "done")?;
        }
        SeatCompletion::Aborted(reason) => {
            result.set("kind", "aborted")?;
            result.set("token", request_token)?;
            result.set("status", "aborted")?;
            result.set("reason", format!("{reason:?}"))?;
        }
    }
    Ok(result)
}

fn freeze_api(api: &Table) -> mlua::Result<()> {
    for pair in api.clone().pairs::<Value, Value>() {
        let (_, value) = pair?;
        if let Value::Table(table) = value {
            table.set_readonly(true);
        }
    }
    api.set_readonly(true);
    Ok(())
}

fn retain_allowed_globals(lua: &Lua) -> mlua::Result<()> {
    let globals = lua.globals();
    let mut keys = Vec::new();
    for pair in globals.pairs::<Value, Value>() {
        keys.push(pair?.0);
    }
    for key in keys {
        let allowed = match &key {
            Value::String(name) => name
                .to_str()
                .ok()
                .is_some_and(|name| GLOBAL_ALLOWLIST.contains(&name.as_ref())),
            _ => false,
        };
        if !allowed {
            globals.raw_set(key, Value::Nil)?;
        }
    }
    Ok(())
}

fn host_function<A, R, F>(lua: &Lua, state: &Arc<RunState>, callback: F) -> mlua::Result<Function>
where
    A: mlua::FromLuaMulti + 'static,
    R: mlua::IntoLuaMulti + 'static,
    F: Fn(&Lua, A, &RunState) -> mlua::Result<R> + Send + 'static,
{
    let state = Arc::clone(state);
    let fallback = lua.create_string("Luau host binding failed")?;
    lua.create_function(move |lua, args| {
        if let Err(error) = state.check_lua(lua) {
            state.observe_callback_error(&error);
            return Ok(failed_binding(fallback.clone()));
        }
        match catch_unwind(AssertUnwindSafe(|| {
            callback(lua, args, &state).and_then(|value| value.into_lua_multi(lua))
        })) {
            Ok(Ok(values)) => Ok(values),
            Ok(Err(error)) => {
                state.observe_callback_error(&error);
                Ok(failed_binding(fallback.clone()))
            }
            Err(_) => {
                state.internal_failure();
                Ok(failed_binding(fallback.clone()))
            }
        }
    })
}

fn failed_binding(message: mlua::LuaString) -> MultiValue {
    MultiValue::from_vec(vec![Value::Boolean(false), Value::String(message)])
}

fn validate_format_arguments(values: &MultiValue) -> Result<(), &'static str> {
    let Some(Value::String(format)) = values.front() else {
        return Ok(());
    };
    let format = format.as_bytes();
    let format = format.as_ref();
    if format.len() > 1024 {
        return Err("string.format: format exceeds N_f (1024 bytes)");
    }

    let mut total = 0usize;
    let mut argument = 1usize;
    let mut index = 0usize;
    while index < format.len() {
        if format[index] != b'%' {
            index += 1;
            continue;
        }
        index += 1;
        let Some(&current) = format.get(index) else {
            break;
        };
        if current == b'%' {
            index += 1;
            continue;
        }
        if current == b'*' {
            if let Some(Value::String(value)) = values.get(argument) {
                total = total.saturating_add(value.as_bytes().len());
            }
            argument = argument.saturating_add(1);
            index += 1;
            if total > MAX_LOG_BYTES_PER_ENTRY * 4 {
                return Err("string.format: string arguments exceed N_a (262144 bytes)");
            }
            continue;
        }
        let mut current = current;
        while matches!(current, b'-' | b'+' | b' ' | b'#' | b'0') {
            index += 1;
            let Some(&next) = format.get(index) else {
                break;
            };
            current = next;
        }
        while current.is_ascii_digit() {
            index += 1;
            let Some(&next) = format.get(index) else {
                break;
            };
            current = next;
        }
        if current == b'.' {
            index += 1;
            let Some(&next) = format.get(index) else {
                break;
            };
            current = next;
            while current.is_ascii_digit() {
                index += 1;
                let Some(&next) = format.get(index) else {
                    break;
                };
                current = next;
            }
        }
        if current == b's' || current == b'q' {
            if let Some(Value::String(value)) = values.get(argument) {
                total = total.saturating_add(value.as_bytes().len());
            }
        }
        if index < format.len() {
            index += 1;
        }
        argument = argument.saturating_add(1);
        if total > MAX_LOG_BYTES_PER_ENTRY * 4 {
            return Err("string.format: string arguments exceed N_a (262144 bytes)");
        }
    }
    if total > MAX_LOG_BYTES_PER_ENTRY * 4 {
        return Err("string.format: string arguments exceed N_a (262144 bytes)");
    }
    Ok(())
}

fn string_argument(
    values: &MultiValue,
    index: usize,
    name: &'static str,
) -> Result<mlua::BorrowedStr, &'static str> {
    match values.get(index) {
        Some(Value::String(value)) => value.to_str().map_err(|_| match name {
            "api.settings.str" => "api.settings.str expects a UTF-8 key",
            "api.settings.num" => "api.settings.num expects a UTF-8 key",
            _ => "api.settings.bool expects a UTF-8 key",
        }),
        _ => Err(match name {
            "api.settings.str" => "api.settings.str expects a string key",
            "api.settings.num" => "api.settings.num expects a string key",
            _ => "api.settings.bool expects a string key",
        }),
    }
}

fn stop_reason(values: &MultiValue) -> Result<String, &'static str> {
    match (values.len(), values.front()) {
        (0, _) | (1, Some(Value::Nil)) => Ok("stopped".to_owned()),
        (1, Some(Value::String(reason))) => reason
            .to_str()
            .map(|reason| bounded_stop_reason(reason.as_ref()))
            .map_err(|_| "api.stop reason must be UTF-8 text"),
        _ => Err("api.stop expects an optional string reason"),
    }
}

fn bounded_stop_reason(reason: &str) -> String {
    let mut end = reason.len().min(STOP_REASON_MAX_BYTES);
    while !reason.is_char_boundary(end) {
        end -= 1;
    }
    reason[..end].to_owned()
}

fn append_log_values(state: &RunState, values: &MultiValue) -> Result<(), &'static str> {
    let line = format_log_values(values)?;
    state.push_log(line)
}

fn format_log_values(values: &MultiValue) -> Result<String, &'static str> {
    if values.len() > MAX_LOG_ARGS {
        return Err("log argument count exceeds 256");
    }
    let mut line = BoundedString::with_capacity(MAX_LOG_LINE_BYTES.min(128), MAX_LOG_LINE_BYTES);
    for (index, value) in values.iter().enumerate() {
        if index > 0 {
            line.push_str("\t")?;
        }
        match value {
            Value::Nil => line.push_str("nil")?,
            Value::Boolean(value) => line.push_str(if *value { "true" } else { "false" })?,
            Value::Integer(value) => line.push_str(&value.to_string())?,
            Value::Number(value) => line.push_str(&value.to_string())?,
            Value::String(value) => {
                let bytes = value.as_bytes();
                if bytes.len() > MAX_LOG_LINE_BYTES {
                    return Err("log line exceeds 4096 bytes");
                }
                let text = String::from_utf8_lossy(bytes.as_ref());
                line.push_str(&text)?;
            }
            Value::Table(_) => line.push_str("<table>")?,
            Value::Function(_) => line.push_str("<function>")?,
            Value::Thread(_) => line.push_str("<thread>")?,
            Value::UserData(_) => line.push_str("<userdata>")?,
            _ => line.push_str("<value>")?,
        }
    }
    Ok(line.finish())
}

struct BoundedString {
    value: String,
    max: usize,
}

impl BoundedString {
    fn with_capacity(capacity: usize, max: usize) -> Self {
        Self {
            value: String::with_capacity(capacity),
            max,
        }
    }

    fn push_str(&mut self, text: &str) -> Result<(), &'static str> {
        if self.value.len().saturating_add(text.len()) > self.max {
            return Err("log line exceeds 4096 bytes");
        }
        self.value.push_str(text);
        Ok(())
    }

    fn finish(self) -> String {
        self.value
    }
}

struct RunState {
    started: Instant,
    phase: AtomicU8,
    latch: AtomicU8,
    deadline_ns: AtomicU64,
    sequence: AtomicU64,
    execution_active: AtomicBool,
    paused: AtomicBool,
    stopping: AtomicBool,
    vm_broken_after_self_stop: AtomicBool,
    current_tick: AtomicU64,
    wait_armed: AtomicBool,
    failure: Mutex<Option<ScriptFailure>>,
    hook_failure: Mutex<Option<ScriptFailure>>,
    stop_receipt: Mutex<Option<ScriptStopReceipt>>,
    settings: Mutex<Arc<SettingsBag>>,
    logs: Mutex<LogBuffer>,
    outcomes: Mutex<VecDeque<TickOutcome>>,
    client: Mutex<SeatClient>,
    #[cfg(feature = "test-hooks")]
    test_elapsed_ns: AtomicU64,
    #[cfg(feature = "test-hooks")]
    dispatch_budget_ns: AtomicU64,
    #[cfg(feature = "test-hooks")]
    test_waiting_for_completion: AtomicBool,
    #[cfg(feature = "test-hooks")]
    test_wait_token: AtomicU64,
    #[cfg(feature = "test-hooks")]
    test_user_move_intent_seq: AtomicU64,
    #[cfg(feature = "test-hooks")]
    test_walk_generation: AtomicU64,
}

impl RunState {
    fn new(settings: Arc<SettingsBag>, client: SeatClient) -> Self {
        Self {
            started: Instant::now(),
            phase: AtomicU8::new(PHASE_SETUP),
            latch: AtomicU8::new(LATCH_EMPTY),
            deadline_ns: AtomicU64::new(NO_DEADLINE),
            sequence: AtomicU64::new(0),
            execution_active: AtomicBool::new(false),
            paused: AtomicBool::new(false),
            stopping: AtomicBool::new(false),
            vm_broken_after_self_stop: AtomicBool::new(false),
            current_tick: AtomicU64::new(0),
            wait_armed: AtomicBool::new(false),
            failure: Mutex::new(None),
            hook_failure: Mutex::new(None),
            stop_receipt: Mutex::new(None),
            settings: Mutex::new(settings),
            logs: Mutex::new(LogBuffer::default()),
            outcomes: Mutex::new(VecDeque::new()),
            client: Mutex::new(client),
            #[cfg(feature = "test-hooks")]
            test_elapsed_ns: AtomicU64::new(0),
            #[cfg(feature = "test-hooks")]
            dispatch_budget_ns: AtomicU64::new(DISPATCH_BUDGET.as_nanos() as u64),
            #[cfg(feature = "test-hooks")]
            test_waiting_for_completion: AtomicBool::new(false),
            #[cfg(feature = "test-hooks")]
            test_wait_token: AtomicU64::new(0),
            #[cfg(feature = "test-hooks")]
            test_user_move_intent_seq: AtomicU64::new(0),
            #[cfg(feature = "test-hooks")]
            test_walk_generation: AtomicU64::new(0),
        }
    }

    fn phase(&self) -> u8 {
        self.phase.load(Ordering::Relaxed)
    }

    fn now_ns(&self) -> u64 {
        let elapsed = self.started.elapsed().as_nanos().min(u64::MAX as u128) as u64;
        #[cfg(feature = "test-hooks")]
        {
            elapsed.saturating_add(self.test_elapsed_ns.load(Ordering::Relaxed))
        }
        #[cfg(not(feature = "test-hooks"))]
        {
            elapsed
        }
    }

    fn set_deadline(&self, budget: Duration) {
        let budget = budget.as_nanos().min(u64::MAX as u128) as u64;
        self.deadline_ns
            .store(self.now_ns().saturating_add(budget), Ordering::Relaxed);
    }

    fn set_dispatch_deadline(&self) {
        #[cfg(feature = "test-hooks")]
        let budget = Duration::from_nanos(self.dispatch_budget_ns.load(Ordering::Relaxed));
        #[cfg(not(feature = "test-hooks"))]
        let budget = DISPATCH_BUDGET;
        self.set_deadline(budget);
    }

    fn clear_deadline(&self) {
        self.deadline_ns.store(NO_DEADLINE, Ordering::Relaxed);
    }

    fn begin_entry(&self, phase: u8) {
        lock(&self.logs).begin_entry();
        self.phase.store(phase, Ordering::Relaxed);
        self.sequence.fetch_add(1, Ordering::Relaxed);
        self.execution_active.store(true, Ordering::Relaxed);
        self.wait_armed.store(false, Ordering::Relaxed);
    }

    fn end_entry(&self) {
        self.execution_active.store(false, Ordering::Relaxed);
        self.phase.store(PHASE_SETUP, Ordering::Relaxed);
    }

    fn check_lua(&self, lua: &Lua) -> mlua::Result<()> {
        let phase = self.phase();
        let latch = self.latch.load(Ordering::Relaxed);
        if latch == LATCH_WATCHDOG || latch == LATCH_MEMORY || latch == LATCH_INTERNAL {
            return Err(Error::runtime("Luau entry stopped"));
        }
        if phase == PHASE_HOOK && lock(&self.hook_failure).is_some() {
            return Err(Error::runtime("Luau onStop hook stopped"));
        }
        if latch == LATCH_SELF_STOP && phase != PHASE_HOOK {
            return Err(Error::runtime("Luau script stopped itself"));
        }

        let used = lua.used_memory();
        if used >= SOFT_MEMORY_LIMIT_BYTES {
            self.trip(LATCH_MEMORY);
            return Err(Error::runtime("Luau memory limit exceeded"));
        }
        let deadline = self.deadline_ns.load(Ordering::Relaxed);
        if deadline != NO_DEADLINE && self.now_ns() >= deadline {
            self.trip(LATCH_WATCHDOG);
            return Err(Error::runtime("Luau execution deadline exceeded"));
        }
        Ok(())
    }

    fn trip(&self, latch: u8) {
        if self.phase() == PHASE_HOOK {
            self.set_hook_failure(failure_for_latch(latch, PHASE_HOOK));
            return;
        }
        if self
            .latch
            .compare_exchange(LATCH_EMPTY, latch, Ordering::Relaxed, Ordering::Relaxed)
            .is_ok()
        {
            *lock(&self.failure) = Some(failure_for_latch(latch, self.phase()));
        }
    }

    fn internal_failure(&self) {
        if self.phase() == PHASE_HOOK {
            self.set_hook_failure(internal_failure());
        } else if self.latch.load(Ordering::Relaxed) == LATCH_SELF_STOP {
            self.vm_broken_after_self_stop
                .store(true, Ordering::Relaxed);
            self.set_hook_failure(internal_failure());
        } else {
            self.trip(LATCH_INTERNAL);
        }
    }

    fn write_self_stop(&self, reason: String) {
        if self
            .latch
            .compare_exchange(
                LATCH_EMPTY,
                LATCH_SELF_STOP,
                Ordering::Relaxed,
                Ordering::Relaxed,
            )
            .is_ok()
        {
            *lock(&self.stop_receipt) = Some(ScriptStopReceipt {
                tick: self.current_tick.load(Ordering::Relaxed),
                reason,
            });
        }
    }

    fn observe_callback_error(&self, error: &Error) {
        if error_has_memory(error) {
            self.trip(LATCH_MEMORY);
        } else if error_has_lost_refusal(error) {
            if self.phase() == PHASE_HOOK {
                self.set_hook_failure(ScriptFailure {
                    code: Arc::from("luau-internal"),
                    message: Arc::from("the VM lost an error while it handled another error"),
                });
            } else if self.latch.load(Ordering::Relaxed) == LATCH_SELF_STOP {
                self.vm_broken_after_self_stop
                    .store(true, Ordering::Relaxed);
                self.set_hook_failure(ScriptFailure {
                    code: Arc::from("luau-internal"),
                    message: Arc::from("the VM lost an error while it handled another error"),
                });
            } else {
                self.trip(LATCH_INTERNAL);
            }
        }
    }

    fn observe_resume_error(&self, error: &Error) {
        if error_has_memory(error) {
            self.trip(LATCH_MEMORY);
        }
    }

    fn set_hook_failure(&self, failure: ScriptFailure) {
        let mut current = lock(&self.hook_failure);
        if current.is_none() {
            *current = Some(failure);
        }
    }

    fn failure(&self) -> ScriptFailure {
        lock(&self.failure)
            .clone()
            .unwrap_or_else(|| failure_for_latch(self.latch.load(Ordering::Relaxed), self.phase()))
    }

    fn hook_failure(&self) -> Option<ScriptFailure> {
        lock(&self.hook_failure).clone()
    }

    fn client_lock(&self) -> mlua::Result<MutexGuard<'_, SeatClient>> {
        match self.client.lock() {
            Ok(client) => Ok(client),
            Err(poisoned) => {
                self.internal_failure();
                Ok(poisoned.into_inner())
            }
        }
    }

    fn push_log(&self, line: String) -> Result<(), &'static str> {
        lock(&self.logs).push(line)
    }

    fn drain_logs(&self) -> Vec<String> {
        lock(&self.logs).drain()
    }

    fn push_outcome(&self, outcome: TickOutcome) {
        let mut outcomes = lock(&self.outcomes);
        if outcomes.len() == MAX_TICK_OUTCOMES {
            outcomes.pop_front();
        }
        outcomes.push_back(outcome);
    }

    fn drain_outcomes(&self) -> Vec<TickOutcome> {
        lock(&self.outcomes).drain(..).collect()
    }

    fn set_settings(&self, settings: &SettingsBag) {
        *lock(&self.settings) = Arc::new(settings.clone());
    }

    #[cfg(feature = "test-hooks")]
    fn advance_test_clock(&self, nanos: u64) {
        let _ =
            self.test_elapsed_ns
                .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
                    Some(current.saturating_add(nanos))
                });
    }
}

#[derive(Default)]
struct LogBuffer {
    lines: VecDeque<String>,
    queue_bytes: usize,
    entry_lines: usize,
    entry_bytes: usize,
}

impl LogBuffer {
    fn begin_entry(&mut self) {
        self.entry_lines = 0;
        self.entry_bytes = 0;
    }

    fn push(&mut self, line: String) -> Result<(), &'static str> {
        let bytes = line.len();
        if bytes > MAX_LOG_LINE_BYTES {
            return Err("log line exceeds 4096 bytes");
        }
        if self.entry_lines >= MAX_LOG_LINES_PER_ENTRY
            || self.entry_bytes.saturating_add(bytes) > MAX_LOG_BYTES_PER_ENTRY
        {
            return Err("log budget exceeds 256 lines or 65536 bytes per entry");
        }
        if self.lines.len() >= MAX_LOG_QUEUE_LINES
            || self.queue_bytes.saturating_add(bytes) > MAX_LOG_QUEUE_BYTES
        {
            return Err("log queue exceeds 1024 lines or 262144 bytes");
        }
        self.queue_bytes += bytes;
        self.entry_bytes += bytes;
        self.entry_lines += 1;
        self.lines.push_back(line);
        Ok(())
    }

    fn drain(&mut self) -> Vec<String> {
        self.queue_bytes = 0;
        self.lines.drain(..).collect()
    }
}

fn failure_for_latch(latch: u8, phase: u8) -> ScriptFailure {
    match latch {
        LATCH_WATCHDOG => ScriptFailure {
            code: Arc::from("luau-watchdog"),
            message: Arc::from(match phase {
                PHASE_HOOK => "onStop did not finish within 50 ms",
                PHASE_LOAD => "the load entry did not finish within 50 ms",
                _ => "the script did not yield within 600 ms",
            }),
        },
        LATCH_MEMORY => ScriptFailure {
            code: Arc::from("luau-memory"),
            message: Arc::from(match phase {
                PHASE_HOOK => "onStop used more than 16 MiB",
                PHASE_LOAD => "the load entry used more than 16 MiB",
                _ => "the script used more than 16 MiB",
            }),
        },
        _ => internal_failure(),
    }
}

fn internal_failure() -> ScriptFailure {
    ScriptFailure {
        code: Arc::from("luau-internal"),
        message: Arc::from("the VM lost an error while it handled another error"),
    }
}

fn failure_from_error(code: &'static str, message: String) -> ScriptFailure {
    ScriptFailure {
        code: Arc::from(code),
        message: Arc::from(message),
    }
}

fn error_has_memory(error: &Error) -> bool {
    match error {
        Error::MemoryError(_) => true,
        Error::CallbackError { cause, .. } => error_has_memory(cause),
        _ => false,
    }
}

fn error_has_lost_refusal(error: &Error) -> bool {
    match error {
        Error::RuntimeError(message) => message == LOST_ERROR,
        Error::CallbackError { cause, .. } => error_has_lost_refusal(cause),
        _ => false,
    }
}

fn bounded_display(value: &impl fmt::Display) -> String {
    let mut output = BoundedDisplay::new(MAX_ERROR_BYTES);
    let _ = write!(&mut output, "{value}");
    output.finish()
}

struct BoundedDisplay {
    value: String,
    max: usize,
    truncated: bool,
}

impl BoundedDisplay {
    fn new(max: usize) -> Self {
        Self {
            value: String::with_capacity(max.min(256)),
            max,
            truncated: false,
        }
    }

    fn finish(mut self) -> String {
        if self.truncated {
            const SUFFIX: &str = "…";
            while self.value.len() + SUFFIX.len() > self.max {
                self.value.pop();
            }
            self.value.push_str(SUFFIX);
        }
        self.value
    }
}

impl fmt::Write for BoundedDisplay {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        if self.truncated {
            return Ok(());
        }
        let remaining = self.max.saturating_sub(self.value.len());
        if text.len() <= remaining {
            self.value.push_str(text);
            return Ok(());
        }
        let mut end = remaining;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        self.value.push_str(&text[..end]);
        self.truncated = true;
        Ok(())
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

struct ActiveVm {
    lua: Lua,
    api: Table,
    tick_function: Function,
    tick_thread: Thread,
    tick_needs_reset: bool,
    tick_suspended: bool,
    paint_function: Option<Function>,
    paint_thread: Option<Thread>,
    paint_needs_reset: bool,
    stop_function: Option<Function>,
    stop_thread: Option<Thread>,
}

/// One Luau VM. Every entry is entered synchronously by `Thread::resume`.
pub struct LuauSurfaceRun {
    vm: Option<ActiveVm>,
    state: Arc<RunState>,
    generation: AtomicU64,
    last_snapshot_facts: Option<PostedSnapshotFacts>,
}

impl LuauSurfaceRun {
    #[cfg(feature = "test-hooks")]
    pub fn set_dispatch_budget_for_test(&self, budget: Duration) {
        self.state.dispatch_budget_ns.store(
            budget.as_nanos().min(u64::MAX as u128) as u64,
            Ordering::Relaxed,
        );
    }

    fn reset_thread(&self, thread: &Thread, function: &Function) -> bool {
        match thread.reset(function.clone()) {
            Ok(()) => true,
            Err(error) => {
                self.state.observe_resume_error(&error);
                self.state.internal_failure();
                false
            }
        }
    }

    fn settled_tick(&self, tick: u64) -> Option<TickOutcome> {
        if self
            .state
            .client_lock()
            .is_ok_and(|mut client| client.loop_settled().is_ok())
        {
            Some(TickOutcome::Success {
                tick,
                generation: self.generation.load(Ordering::Relaxed),
            })
        } else {
            self.state.internal_failure();
            None
        }
    }

    fn run_tick(&mut self, vm: &mut ActiveVm, tick: u64) -> Option<TickOutcome> {
        if vm.tick_needs_reset && !self.reset_thread(&vm.tick_thread, &vm.tick_function) {
            return None;
        }
        vm.tick_needs_reset = false;
        self.state.wait_armed.store(false, Ordering::Relaxed);
        match execute_entry::<MultiValue>(
            &vm.lua,
            &self.state,
            &vm.tick_thread,
            vm.api.clone(),
            PHASE_TICK,
            true,
        ) {
            EntryOutcome::Returned(_) => {
                vm.tick_suspended = false;
                vm.tick_needs_reset = true;
                self.settled_tick(tick)
            }
            EntryOutcome::Yielded => {
                vm.tick_suspended = true;
                None
            }
            EntryOutcome::UserError(message) => {
                vm.tick_suspended = false;
                vm.tick_needs_reset = true;
                Some(TickOutcome::Error {
                    tick,
                    generation: self.generation.load(Ordering::Relaxed),
                    message,
                })
            }
            EntryOutcome::SelfStop
            | EntryOutcome::Trip(_)
            | EntryOutcome::InternalAfterSelfStop => {
                vm.tick_suspended = false;
                None
            }
        }
    }

    fn resume_tick<A: mlua::IntoLuaMulti>(
        &mut self,
        vm: &mut ActiveVm,
        tick: u64,
        args: A,
    ) -> Option<TickOutcome> {
        self.state.wait_armed.store(false, Ordering::Relaxed);
        match execute_entry::<MultiValue>(
            &vm.lua,
            &self.state,
            &vm.tick_thread,
            args,
            PHASE_TICK,
            true,
        ) {
            EntryOutcome::Returned(_) => {
                vm.tick_suspended = false;
                vm.tick_needs_reset = true;
                self.settled_tick(tick)
            }
            EntryOutcome::Yielded => {
                vm.tick_suspended = true;
                None
            }
            EntryOutcome::UserError(message) => {
                vm.tick_suspended = false;
                vm.tick_needs_reset = true;
                Some(TickOutcome::Error {
                    tick,
                    generation: self.generation.load(Ordering::Relaxed),
                    message,
                })
            }
            EntryOutcome::SelfStop
            | EntryOutcome::Trip(_)
            | EntryOutcome::InternalAfterSelfStop => {
                vm.tick_suspended = false;
                None
            }
        }
    }

    fn run_paint(&mut self, vm: &mut ActiveVm, tick: u64) -> Option<TickOutcome> {
        let (Some(function), Some(thread)) = (&vm.paint_function, &vm.paint_thread) else {
            return None;
        };
        if vm.paint_needs_reset && !self.reset_thread(thread, function) {
            return None;
        }
        vm.paint_needs_reset = false;
        match execute_entry::<MultiValue>(
            &vm.lua,
            &self.state,
            thread,
            vm.api.clone(),
            PHASE_PAINT,
            false,
        ) {
            EntryOutcome::Returned(_) => {
                vm.paint_needs_reset = true;
                None
            }
            EntryOutcome::Yielded => {
                vm.paint_needs_reset = true;
                Some(TickOutcome::Error {
                    tick,
                    generation: self.generation.load(Ordering::Relaxed),
                    message: "onPaint yielded outside api.wait".to_owned(),
                })
            }
            EntryOutcome::UserError(message) => {
                vm.paint_needs_reset = true;
                Some(TickOutcome::Error {
                    tick,
                    generation: self.generation.load(Ordering::Relaxed),
                    message: format!("onPaint: {message}"),
                })
            }
            EntryOutcome::SelfStop
            | EntryOutcome::Trip(_)
            | EntryOutcome::InternalAfterSelfStop => {
                vm.paint_needs_reset = true;
                None
            }
        }
    }

    #[expect(
        clippy::large_types_passed_by_value,
        reason = "SeatClient consumes the lent native facts; moving them avoids a second frame copy"
    )]
    fn dispatch_inner(
        &mut self,
        wake: SurfaceWake,
        snap_tick: u64,
        _input_identity: u64,
        input: &SnapshotInput<'_>,
        native: NativeFactsInput<'_>,
    ) -> LentOutcome {
        let Some(mut vm) = self.vm.take() else {
            return LentOutcome::Refused;
        };
        self.state.current_tick.store(snap_tick, Ordering::Relaxed);
        #[cfg(feature = "test-hooks")]
        {
            self.state
                .test_user_move_intent_seq
                .store(input.user_move_intent_seq, Ordering::Relaxed);
            self.state
                .test_walk_generation
                .store(native.walk_outcome_generation, Ordering::Relaxed);
        }
        self.state.set_dispatch_deadline();
        let facts = PostedSnapshotFacts {
            withdraw_x_result_seq: input.withdraw_x_result_seq,
            bank_op_result_seq: input.bank_op_result_seq,
            walk_outcome_seq: native.walk_outcome_seq,
            bank_selection: native.bank_selection,
            route_inspect: RouteInspectIdentities {
                latest: (
                    native.route_inspect.latest.seq,
                    native.route_inspect.latest.generation,
                    native.route_inspect.latest.request_id,
                ),
                prev: (
                    native.route_inspect.prev.seq,
                    native.route_inspect.prev.generation,
                    native.route_inspect.prev.request_id,
                ),
                replaced_id: native.route_inspect.replaced_id,
                replaced_prev_id: native.route_inspect.replaced_prev_id,
                refused: [
                    native.route_inspect.refused_id,
                    native.route_inspect.refused_id_2,
                    native.route_inspect.refused_id_3,
                ],
            },
        };
        self.last_snapshot_facts = Some(facts);

        let (release_floor, is_game_tick, held) = match wake {
            SurfaceWake::Tick { held } => (0, true, held),
            SurfaceWake::Evidence => (0, false, false),
            SurfaceWake::SeatPage { release_floor } => (release_floor, false, false),
        };
        {
            let mut client = match self.state.client_lock() {
                Ok(client) => client,
                Err(_) => {
                    self.vm = Some(vm);
                    return LentOutcome::Failed(self.state.failure());
                }
            };
            client.accept_delivery(input, native, release_floor, is_game_tick && !held);
        }

        let mut outcome = None;
        let mut resumed_wait = false;
        if vm.tick_suspended && !held {
            let ready = self
                .state
                .client_lock()
                .is_ok_and(|client| client.completion_ready());
            if ready {
                let completion = self
                    .state
                    .client_lock()
                    .ok()
                    .and_then(|mut client| client.take_completion());
                if let Some(completion) = completion {
                    resumed_wait = true;
                    #[cfg(feature = "test-hooks")]
                    {
                        if self
                            .state
                            .test_waiting_for_completion
                            .swap(false, Ordering::Relaxed)
                        {
                            let request_token =
                                self.state.test_wait_token.swap(0, Ordering::Relaxed);
                            let result =
                                match test_completion_table(&vm.lua, completion, request_token) {
                                    Ok(result) => result,
                                    Err(error) => {
                                        self.state.observe_callback_error(&error);
                                        self.state.internal_failure();
                                        self.vm = None;
                                        return LentOutcome::Failed(self.state.failure());
                                    }
                                };
                            outcome = self.resume_tick(&mut vm, snap_tick, result);
                        } else {
                            outcome = self.resume_tick(&mut vm, snap_tick, ());
                        }
                    }
                    #[cfg(not(feature = "test-hooks"))]
                    {
                        let _ = completion;
                        outcome = self.resume_tick(&mut vm, snap_tick, ());
                    }
                }
            }
        }
        if is_game_tick && !held && !vm.tick_suspended && !resumed_wait {
            outcome = self.run_tick(&mut vm, snap_tick);
        }
        if is_game_tick && self.state.latch.load(Ordering::Relaxed) == LATCH_EMPTY {
            if let Some(paint_outcome) = self.run_paint(&mut vm, snap_tick) {
                outcome = Some(match outcome {
                    Some(TickOutcome::Error {
                        tick,
                        generation,
                        message,
                    }) => TickOutcome::Error {
                        tick,
                        generation,
                        message: bounded_string(format!("{message}; {paint_outcome:?}")),
                    },
                    _ => paint_outcome,
                });
            }
        }
        self.state.clear_deadline();
        self.state.execution_active.store(false, Ordering::Relaxed);
        self.state.phase.store(PHASE_SETUP, Ordering::Relaxed);

        let latch = self.state.latch.load(Ordering::Relaxed);
        if latch == LATCH_WATCHDOG || latch == LATCH_MEMORY || latch == LATCH_INTERNAL {
            self.vm = None;
            return LentOutcome::Failed(self.state.failure());
        }
        if self.state.vm_broken_after_self_stop.load(Ordering::Relaxed) {
            self.vm = None;
            return LentOutcome::Accepted;
        }
        if latch == LATCH_SELF_STOP {
            self.vm = Some(vm);
            return LentOutcome::Accepted;
        }
        if let Some(outcome) = outcome {
            self.state.push_outcome(outcome);
        }
        self.vm = Some(vm);
        LentOutcome::Accepted
    }

    fn stop_inner(&mut self) -> StopCompletion {
        self.state.stopping.store(true, Ordering::Relaxed);
        let mut hook_failure = self.state.hook_failure();
        if self.state.vm_broken_after_self_stop.load(Ordering::Relaxed) {
            hook_failure.get_or_insert_with(internal_failure);
        } else if self.state.latch.load(Ordering::Relaxed) == LATCH_EMPTY
            || self.state.latch.load(Ordering::Relaxed) == LATCH_SELF_STOP
        {
            if let Some(vm) = self.vm.as_mut() {
                if let (Some(_function), Some(thread)) = (&vm.stop_function, &vm.stop_thread) {
                    let reason = lock(&self.state.stop_receipt)
                        .as_ref()
                        .map_or_else(|| "stopped".to_owned(), |receipt| receipt.reason.clone());
                    self.state.set_deadline(HOOK_BUDGET);
                    match execute_entry::<MultiValue>(
                        &vm.lua,
                        &self.state,
                        thread,
                        reason,
                        PHASE_HOOK,
                        false,
                    ) {
                        EntryOutcome::Returned(_) => {}
                        EntryOutcome::Trip(failure) => hook_failure = Some(failure),
                        EntryOutcome::UserError(message) => {
                            hook_failure = Some(failure_from_error(
                                "luau-script",
                                bounded_string(format!("onStop: {message}")),
                            ));
                        }
                        EntryOutcome::Yielded => {
                            hook_failure = Some(failure_from_error(
                                "luau-script",
                                "onStop yielded outside a waiting binding".to_owned(),
                            ));
                        }
                        EntryOutcome::SelfStop | EntryOutcome::InternalAfterSelfStop => {
                            hook_failure = Some(internal_failure());
                        }
                    }
                    self.state.clear_deadline();
                }
            }
        }
        self.vm = None;
        StopCompletion {
            logs: self.state.drain_logs(),
            hook_failure,
        }
    }
}

enum EntryOutcome<T> {
    Returned(T),
    Yielded,
    UserError(String),
    SelfStop,
    Trip(ScriptFailure),
    InternalAfterSelfStop,
}

fn execute_entry<R: mlua::FromLuaMulti>(
    lua: &Lua,
    state: &RunState,
    thread: &Thread,
    args: impl mlua::IntoLuaMulti,
    phase: u8,
    allow_wait: bool,
) -> EntryOutcome<R> {
    state.begin_entry(phase);
    let before = state.check_lua(lua);
    let resumed = match before {
        Ok(()) => thread.resume::<R>(args),
        Err(error) => Err(error),
    };
    if let Err(error) = &resumed {
        state.observe_resume_error(error);
    }
    let after = state.check_lua(lua);
    let status = thread.status();
    state.end_entry();

    if let Some(failure) = state.hook_failure() {
        if phase == PHASE_HOOK {
            return EntryOutcome::Trip(failure);
        }
    }
    let latch = state.latch.load(Ordering::Relaxed);
    if latch == LATCH_WATCHDOG || latch == LATCH_MEMORY || latch == LATCH_INTERNAL {
        return EntryOutcome::Trip(state.failure());
    }
    if state.vm_broken_after_self_stop.load(Ordering::Relaxed) {
        return EntryOutcome::InternalAfterSelfStop;
    }
    if latch == LATCH_SELF_STOP && phase != PHASE_HOOK {
        return EntryOutcome::SelfStop;
    }
    if let Err(error) = after {
        state.observe_resume_error(&error);
        let latch = state.latch.load(Ordering::Relaxed);
        if latch != LATCH_EMPTY && latch != LATCH_SELF_STOP {
            return EntryOutcome::Trip(state.failure());
        }
        if phase == PHASE_HOOK {
            if let Some(failure) = state.hook_failure() {
                return EntryOutcome::Trip(failure);
            }
        }
    }
    match resumed {
        Err(error) => EntryOutcome::UserError(bounded_display(&error)),
        Ok(value) if status == ThreadStatus::Finished => EntryOutcome::Returned(value),
        Ok(_)
            if status == ThreadStatus::Resumable
                && allow_wait
                && state.wait_armed.load(Ordering::Relaxed) =>
        {
            EntryOutcome::Yielded
        }
        Ok(_) if status == ThreadStatus::Resumable => {
            EntryOutcome::UserError("entry yielded outside api.wait".to_owned())
        }
        Ok(_) => {
            EntryOutcome::UserError("entry did not finish in a valid coroutine state".to_owned())
        }
    }
}

fn bounded_string(value: String) -> String {
    if value.len() <= MAX_ERROR_BYTES {
        return value;
    }
    let mut end = MAX_ERROR_BYTES.saturating_sub(3);
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    let mut bounded = value[..end].to_owned();
    bounded.push('…');
    bounded
}

impl SurfaceRun for LuauSurfaceRun {
    fn input_delivery(&self) -> InputDelivery {
        InputDelivery::Lent
    }

    fn dispatch_lent(
        &mut self,
        wake: SurfaceWake,
        snap_tick: u64,
        input_identity: u64,
        input: &SnapshotInput<'_>,
        native: NativeFactsInput<'_>,
    ) -> LentOutcome {
        if self.vm.is_none()
            || self.state.paused.load(Ordering::Relaxed)
            || self.state.stopping.load(Ordering::Relaxed)
            || self.state.latch.load(Ordering::Relaxed) != LATCH_EMPTY
        {
            return LentOutcome::Refused;
        }
        match catch_unwind(AssertUnwindSafe(|| {
            self.dispatch_inner(wake, snap_tick, input_identity, input, native)
        })) {
            Ok(outcome) => outcome,
            Err(_) => {
                self.state.internal_failure();
                if self.state.latch.load(Ordering::Relaxed) == LATCH_SELF_STOP {
                    self.vm = None;
                    LentOutcome::Accepted
                } else {
                    self.vm = None;
                    LentOutcome::Failed(self.state.failure())
                }
            }
        }
    }

    fn poll_ready(&self) -> SurfaceReady {
        SurfaceReady::Ready
    }

    fn generation(&self) -> u64 {
        self.generation.load(Ordering::Relaxed)
    }

    fn bind_runtime_context(&mut self, _context: Arc<ScriptRuntimeContext>) {}

    fn execution_active(&self) -> bool {
        self.state.execution_active.load(Ordering::Relaxed)
    }

    fn execution_sequence(&self) -> (u64, bool) {
        (
            self.state.sequence.load(Ordering::Relaxed),
            self.state.execution_active.load(Ordering::Relaxed),
        )
    }

    fn align_execution(&self, execution: ExecutionId) {
        if let Ok(mut client) = self.state.client_lock() {
            client.align_execution(execution);
        }
    }

    fn reset_session_work(&mut self, execution: Option<ExecutionId>) -> u64 {
        self.last_snapshot_facts = None;
        let generation = self.state.client_lock().map_or_else(
            |_| self.generation.load(Ordering::Relaxed),
            |mut client| client.reset(execution),
        );
        self.generation.store(generation, Ordering::Relaxed);
        generation
    }

    fn reconnect_session_work(&mut self, execution: Option<ExecutionId>) -> u64 {
        self.last_snapshot_facts = None;
        let generation = self.state.client_lock().map_or_else(
            |_| self.generation.load(Ordering::Relaxed),
            |mut client| client.reconnect(execution),
        );
        self.generation.store(generation, Ordering::Relaxed);
        generation
    }

    fn pause(&self) {
        self.state.paused.store(true, Ordering::Relaxed);
    }

    fn resume(&self) {
        self.state.paused.store(false, Ordering::Relaxed);
    }

    fn stop(self: Box<Self>, done: Sender<StopCompletion>) {
        let mut run = *self;
        let completion = match catch_unwind(AssertUnwindSafe(|| run.stop_inner())) {
            Ok(completion) => completion,
            Err(_) => {
                run.vm = None;
                StopCompletion {
                    logs: run.state.drain_logs(),
                    hook_failure: Some(internal_failure()),
                }
            }
        };
        let _ = done.send(completion);
    }

    fn stopped(&self) -> bool {
        self.state.stopping.load(Ordering::Relaxed)
            || self.state.latch.load(Ordering::Relaxed) != LATCH_EMPTY
    }

    fn script_stop_receipt(&self) -> Option<ScriptStopReceipt> {
        lock(&self.state.stop_receipt).clone()
    }

    fn take_script_cut(&self) -> bool {
        false
    }

    fn post_snapshot_delta(
        &mut self,
        _input: &SnapshotInput<'_>,
        _native: NativeFactsInput<'_>,
        _force_banks: bool,
    ) -> bool {
        false
    }

    fn post_snapshot_wake(
        &mut self,
        _input: &SnapshotInput<'_>,
        _native: NativeFactsInput<'_>,
        _force_banks: bool,
        _preserve_inv: bool,
    ) -> bool {
        false
    }

    fn posted_snapshot_facts(&self) -> Option<PostedSnapshotFacts> {
        self.last_snapshot_facts
    }

    fn post_channel_events(&self, _events: &[InteractReq]) -> bool {
        false
    }

    fn post_settings_bag(&self, bag: &SettingsBag) {
        self.state.set_settings(bag);
    }

    fn post_loadouts(&self, _loadouts: &[api::loadout::Loadout]) {}

    fn on_game_tick_at(&self, _snap_tick: u64, _input_identity: u64) {}

    fn on_snapshot_change_at(&self, _snap_tick: u64, _input_identity: u64) {}

    fn on_seat_page_at(&self, _snap_tick: u64, _input_identity: u64, _release_floor: u64) -> bool {
        false
    }

    fn drain_interacts_stamped(&self) -> Vec<QueuedInteract> {
        Vec::new()
    }

    fn restore_interacts_stamped(&self, _drained: Vec<QueuedInteract>) {}

    fn drain_seat_batches(&self) -> Vec<QueuedSeatBatch> {
        self.state
            .client_lock()
            .map_or_else(|_| Vec::new(), |mut client| client.drain_seat_batches())
    }

    fn restore_seat_batches(&self, drained: Vec<QueuedSeatBatch>) {
        if let Ok(mut client) = self.state.client_lock() {
            client.restore_seat_batches(drained);
        }
    }

    fn drain_lifecycle(&self) -> Vec<InteractReq> {
        self.state
            .client_lock()
            .map_or_else(|_| Vec::new(), |mut client| client.drain_lifecycle())
    }

    fn drain_tick_outcomes(&self) -> Vec<TickOutcome> {
        self.state.drain_outcomes()
    }

    fn discard_mouse_interacts(&self) {}

    fn rearm_host_discarded_walk(&self, token: u64) {
        if let Ok(mut client) = self.state.client_lock() {
            let _ = client.rearm_host_discarded_walk(token);
        }
    }

    fn request_recovery_anchor(&self) {}

    fn live_walking_operation(&self) -> bool {
        self.state
            .client_lock()
            .is_ok_and(|client| client.live_walking_operation())
    }

    fn note_manual_walk_takeover(&self, user_move_intent_seq: u64) {
        if let Ok(mut client) = self.state.client_lock() {
            client.note_manual_walk_takeover(user_move_intent_seq);
        }
    }

    fn paint(&self) -> Option<Arc<ScriptPaint>> {
        None
    }

    fn paint_click(&self, _id: &str) {}

    fn paint_select(&self, _key: &str, _name: &str) {}

    fn journal_paint_hidden(&self, _now: Instant) -> bool {
        false
    }

    fn journal_wait(&self, _now: Instant) -> Option<JournalWait> {
        None
    }

    fn expired_hold_owners(&self, _now: Instant) -> [Option<api::identity::RequestId>; 2] {
        [None, None]
    }

    fn probe(&self, _expr: &str) -> Result<serde_json::Value, SurfaceError> {
        Err(SurfaceError::Probe("not supported".to_owned()))
    }

    fn drain_logs(&self) -> Vec<String> {
        self.state.drain_logs()
    }

    fn ignored_randoms(&self) -> Vec<String> {
        Vec::new()
    }

    fn ignores_random(&self, _name: &str) -> bool {
        false
    }

    fn memory_metrics(&self) -> Option<serde_json::Value> {
        self.vm.as_ref().map(|vm| {
            serde_json::json!({
                "used_bytes": vm.lua.used_memory(),
                "soft_limit_bytes": SOFT_MEMORY_LIMIT_BYTES,
                "hard_limit_bytes": HARD_MEMORY_LIMIT_BYTES,
            })
        })
    }

    fn memory_progress(&self) -> serde_json::Value {
        self.memory_metrics()
            .unwrap_or_else(|| serde_json::json!({"available": false}))
    }

    #[cfg(feature = "test-hooks")]
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    #[cfg(feature = "test-hooks")]
    fn seat_batch_backlog(&self) -> usize {
        self.state
            .client_lock()
            .map_or(0, |client| client.queued_batch_count())
    }

    #[cfg(feature = "test-hooks")]
    fn held_fold_len(&self) -> usize {
        self.state
            .client_lock()
            .map_or(0, |client| client.held_fold_len())
    }
}
