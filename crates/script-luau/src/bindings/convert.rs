//! Argument and result conversion for the bindings. A function here shapes
//! values. It holds no host and cannot act (`design-r3.md` 7.4 rule T4).
//!
//! A Luau value becomes a Rust number only through [`integer`] and the
//! fallback readers here. They match the value kinds; they never use the
//! numeric `FromLua` of `mlua`, which converts a numeric string
//! (ADR 0023 item 26).

use mlua::{BorrowedStr, Lua, LuaString, MultiValue, Table, Value};
use script::bank::request::{BankSeatArgs, BankSeatOperation};
use script::interact::InteractReq;
use script::native::snapshot_source::TileInput;
use script::native::WalkCancelReason;
use script::surface::client::{ClientError, SeatCompletion, WalkCompletion};

/// The most arguments that a fixed-arity binding takes.
pub(crate) const MAX_ARGS: usize = 5;
const MAX_LOG_LINE_BYTES: usize = 4 * 1024;
const MAX_LOG_ARGS: usize = 256;
const STOP_REASON_MAX_BYTES: usize = 256;

/// The fixed arguments of one call. A missing argument is `nil`.
pub(crate) struct Args {
    values: [Value; MAX_ARGS],
}

impl Args {
    pub(crate) fn new(values: [Value; MAX_ARGS]) -> Self {
        Self { values }
    }

    pub(crate) fn first(&self) -> &Value {
        &self.values[0]
    }

    pub(crate) fn second(&self) -> &Value {
        &self.values[1]
    }

    pub(crate) fn third(&self) -> &Value {
        &self.values[2]
    }

    pub(crate) fn fourth(&self) -> &Value {
        &self.values[3]
    }

    pub(crate) fn fifth(&self) -> &Value {
        &self.values[4]
    }
}

/// The values that a binding returns to the script.
pub(crate) enum Out {
    Nothing,
    One(Value),
    Three(Value, Value, Value),
}

impl From<()> for Out {
    fn from((): ()) -> Self {
        Out::Nothing
    }
}

/// A typed binding failure. The registration function maps it to a Luau
/// error with the source position of the call (ADR 0023 item 30).
#[derive(Debug)]
pub(crate) enum Refusal {
    /// An argument has the wrong kind or range.
    Argument {
        argument: &'static str,
        expected: &'static str,
    },
    /// More arguments than the member takes.
    Arity(usize),
    /// A read outside an entry that lends game input.
    NoInput,
    /// The run refuses the call in this entry or state.
    Run(&'static str),
    /// The seat client refuses the request.
    Seat(ClientError),
    /// The VM failed while the binding made its result.
    Vm(mlua::Error),
}

impl From<mlua::Error> for Refusal {
    fn from(error: mlua::Error) -> Self {
        Refusal::Vm(error)
    }
}

impl From<ClientError> for Refusal {
    fn from(error: ClientError) -> Self {
        Refusal::Seat(error)
    }
}

pub(crate) type Reply = Result<Out, Refusal>;

/// The two walk members.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum WalkShape {
    Exact,
    Near,
}

fn refuse(argument: &'static str, expected: &'static str) -> Refusal {
    Refusal::Argument { argument, expected }
}

/// A Luau number with an integral value in the `i32` range.
pub(crate) fn integer(value: &Value, argument: &'static str) -> Result<i32, Refusal> {
    match value {
        Value::Integer(number) => match i32::try_from(*number) {
            Ok(number) => Ok(number),
            Err(_) => Err(refuse(argument, "a 32-bit integer")),
        },
        Value::Number(number)
            if number.is_finite()
                && number.fract() == 0.0
                && *number >= f64::from(i32::MIN)
                && *number <= f64::from(i32::MAX) =>
        {
            Ok(*number as i32)
        }
        _ => Err(refuse(argument, "a 32-bit integer")),
    }
}

/// A Luau number with an integral value from 0 to `i32::MAX`.
pub(crate) fn nonnegative(value: &Value, argument: &'static str) -> Result<i32, Refusal> {
    match integer(value, argument) {
        Ok(number) if number >= 0 => Ok(number),
        _ => Err(refuse(argument, "an integer from 0")),
    }
}

/// The string key of a settings read.
pub(crate) fn key(value: &Value) -> Result<BorrowedStr, Refusal> {
    match value {
        Value::String(text) => match text.to_str() {
            Ok(key) => Ok(key),
            Err(_) => Err(refuse("key", "UTF-8 text")),
        },
        _ => Err(refuse("key", "a string")),
    }
}

pub(crate) fn text_fallback(
    value: &Value,
    argument: &'static str,
) -> Result<Option<LuaString>, Refusal> {
    match value {
        Value::Nil => Ok(None),
        Value::String(text) => Ok(Some(text.clone())),
        _ => Err(refuse(argument, "a string or nil")),
    }
}

pub(crate) fn number_fallback(value: &Value, argument: &'static str) -> Result<f64, Refusal> {
    match value {
        Value::Nil => Ok(0.0),
        Value::Integer(number) => Ok(*number as f64),
        Value::Number(number) => Ok(*number),
        _ => Err(refuse(argument, "a number or nil")),
    }
}

pub(crate) fn boolean_fallback(value: &Value, argument: &'static str) -> Result<bool, Refusal> {
    match value {
        Value::Nil => Ok(false),
        Value::Boolean(flag) => Ok(*flag),
        _ => Err(refuse(argument, "a boolean or nil")),
    }
}

/// `NativeSettings.str`: the bag value when it is a string, else the fallback.
pub(crate) fn setting_text(
    lua: &Lua,
    value: Option<serde_json::Value>,
    fallback: Option<LuaString>,
) -> Reply {
    match (value, fallback) {
        (Some(serde_json::Value::String(text)), _) => {
            Ok(Out::One(Value::String(lua.create_string(text)?)))
        }
        (_, Some(fallback)) => Ok(Out::One(Value::String(fallback))),
        (_, None) => Ok(Out::One(Value::String(lua.create_string("")?))),
    }
}

/// `NativeSettings.num`: the bag value when it is a number, else the fallback.
pub(crate) fn setting_number(value: Option<serde_json::Value>, fallback: f64) -> Out {
    Out::One(Value::Number(
        value
            .as_ref()
            .and_then(serde_json::Value::as_f64)
            .unwrap_or(fallback),
    ))
}

/// `NativeSettings.bool`: the bag value when it is a boolean, else the fallback.
pub(crate) fn setting_boolean(value: Option<serde_json::Value>, fallback: bool) -> Out {
    Out::One(Value::Boolean(
        value
            .as_ref()
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(fallback),
    ))
}

pub(crate) fn unsigned_out(number: u64) -> Out {
    Out::One(Value::Number(number as f64))
}

pub(crate) fn integer_out(number: i32) -> Out {
    Out::One(Value::Number(f64::from(number)))
}

pub(crate) fn boolean_out(flag: bool) -> Out {
    Out::One(Value::Boolean(flag))
}

pub(crate) fn tile_out(tile: Option<TileInput>) -> Out {
    match tile {
        Some(tile) => Out::Three(
            Value::Number(f64::from(tile.x)),
            Value::Number(f64::from(tile.z)),
            Value::Number(f64::from(tile.level)),
        ),
        None => Out::Nothing,
    }
}

pub(crate) fn text_out(lua: &Lua, text: Option<&str>) -> Reply {
    match text {
        Some(text) => Ok(Out::One(Value::String(lua.create_string(text)?))),
        None => Ok(Out::One(Value::Nil)),
    }
}

/// One log line: the values joined by tabs, at most 4096 bytes.
pub(crate) fn log_line(values: &MultiValue) -> Result<String, Refusal> {
    if values.len() > MAX_LOG_ARGS {
        return Err(Refusal::Run("log argument count exceeds 256"));
    }
    let mut line = LogLine::with_capacity(MAX_LOG_LINE_BYTES.min(128));
    for (index, value) in values.iter().enumerate() {
        if index > 0 {
            line.append("\t")?;
        }
        match value {
            Value::Nil => line.append("nil")?,
            Value::Boolean(flag) => line.append(if *flag { "true" } else { "false" })?,
            Value::Integer(number) => line.append(&number.to_string())?,
            Value::Number(number) => line.append(&number.to_string())?,
            Value::String(text) => line.append_bytes(&text.as_bytes())?,
            Value::Table(_) => line.append("<table>")?,
            Value::Function(_) => line.append("<function>")?,
            Value::Thread(_) => line.append("<thread>")?,
            Value::UserData(_) => line.append("<userdata>")?,
            _ => line.append("<value>")?,
        }
    }
    Ok(line.into_line())
}

/// A log line that refuses text past its bound.
struct LogLine {
    value: String,
}

impl LogLine {
    fn with_capacity(capacity: usize) -> Self {
        Self {
            value: String::with_capacity(capacity),
        }
    }

    fn append(&mut self, text: &str) -> Result<(), Refusal> {
        if self.value.len().saturating_add(text.len()) > MAX_LOG_LINE_BYTES {
            return Err(Refusal::Run("log line exceeds 4096 bytes"));
        }
        self.value += text;
        Ok(())
    }

    fn append_bytes(&mut self, bytes: &[u8]) -> Result<(), Refusal> {
        if bytes.len() > MAX_LOG_LINE_BYTES {
            return Err(Refusal::Run("log line exceeds 4096 bytes"));
        }
        self.append(&String::from_utf8_lossy(bytes))
    }

    fn into_line(self) -> String {
        self.value
    }
}

/// The optional reason of `api.stop`, cut to 256 bytes at a UTF-8 boundary.
pub(crate) fn stop_reason(value: &Value) -> Result<String, Refusal> {
    match value {
        Value::Nil => Ok(String::from("stopped")),
        Value::String(text) => match text.to_str() {
            Ok(reason) => Ok(bounded_reason(&reason)),
            Err(_) => Err(refuse("reason", "UTF-8 text")),
        },
        _ => Err(refuse("reason", "a string or nil")),
    }
}

fn bounded_reason(reason: &str) -> String {
    let prefix = &reason.as_bytes()[..reason.len().min(STOP_REASON_MAX_BYTES)];
    match std::str::from_utf8(prefix) {
        Ok(text) => String::from(text),
        Err(error) => String::from(&reason[..error.valid_up_to()]),
    }
}

/// The three `FindOptions` flags of the JS v2 walk op. Omitted flags are false.
#[derive(Default)]
struct WalkOptions {
    allow_teleports: bool,
    allow_wilderness: bool,
    allow_bank_fetch: bool,
}

fn option_flag(table: &Table, name: &'static str) -> Result<bool, Refusal> {
    match table.raw_get::<Value>(name)? {
        Value::Nil => Ok(false),
        Value::Boolean(flag) => Ok(flag),
        _ => Err(refuse(name, "a boolean or nil")),
    }
}

fn walk_options(value: &Value) -> Result<WalkOptions, Refusal> {
    match value {
        Value::Nil => Ok(WalkOptions::default()),
        Value::Table(table) => Ok(WalkOptions {
            allow_teleports: option_flag(table, "allow_teleports")?,
            allow_wilderness: option_flag(table, "allow_wilderness")?,
            allow_bank_fetch: option_flag(table, "allow_bank_fetch")?,
        }),
        _ => Err(refuse("options", "a table or nil")),
    }
}

/// The core walk row of `walk.to` and `walk.near`. The seat client stamps
/// its request id. Prerequisite grants inherit, as for a JS v2 walk op.
pub(crate) fn walk_request(args: &Args, shape: WalkShape) -> Result<InteractReq, Refusal> {
    let x = integer(args.first(), "x")?;
    let z = integer(args.second(), "z")?;
    let level = integer(args.third(), "level")?;
    match shape {
        WalkShape::Exact => {
            let options = walk_options(args.fourth())?;
            Ok(InteractReq::Walk {
                x,
                z,
                level,
                allow_teleports: options.allow_teleports,
                allow_wilderness: options.allow_wilderness,
                allow_bank_fetch: options.allow_bank_fetch,
                allow_prereq_dialogue: None,
                allow_quest_start: None,
                request_id: 0,
                track_door_crossing: false,
                avoid: Vec::new(),
                cross: Vec::new(),
            })
        }
        WalkShape::Near => {
            let radius = nonnegative(args.fourth(), "radius")?;
            let options = walk_options(args.fifth())?;
            Ok(InteractReq::WalkNear {
                x,
                z,
                level,
                radius,
                compat_tile_intent: false,
                allow_teleports: options.allow_teleports,
                allow_wilderness: options.allow_wilderness,
                allow_bank_fetch: options.allow_bank_fetch,
                allow_prereq_dialogue: None,
                allow_quest_start: None,
                request_id: 0,
                track_door_crossing: false,
                avoid: Vec::new(),
                cross: Vec::new(),
            })
        }
    }
}

fn required_text(value: &Value, argument: &'static str) -> Result<String, Refusal> {
    match value {
        Value::String(text) => match text.to_str() {
            Ok(text) => Ok(text.to_owned()),
            Err(_) => Err(refuse(argument, "UTF-8 text")),
        },
        _ => Err(refuse(argument, "a string")),
    }
}

fn optional_nonnegative(value: &Value, argument: &'static str) -> Result<Option<i32>, Refusal> {
    match value {
        Value::Nil => Ok(None),
        _ => nonnegative(value, argument).map(Some),
    }
}

pub(crate) fn bank_request(operation: BankSeatOperation) -> BankSeatArgs {
    BankSeatArgs::new(operation)
}

pub(crate) fn npc_request(args: &Args) -> Result<InteractReq, Refusal> {
    Ok(InteractReq::Npc {
        name: required_text(args.first(), "name")?,
        action: required_text(args.second(), "action")?,
        index: optional_nonnegative(args.third(), "index")?,
    })
}

pub(crate) fn loc_request(args: &Args) -> Result<InteractReq, Refusal> {
    Ok(InteractReq::Loc {
        x: integer(args.first(), "x")?,
        z: integer(args.second(), "z")?,
        level: integer(args.third(), "level")?,
        action: required_text(args.fourth(), "action")?,
        id: Some(nonnegative(args.fifth(), "id")?),
    })
}

pub(crate) fn obj_request(args: &Args) -> Result<InteractReq, Refusal> {
    Ok(InteractReq::Obj {
        x: integer(args.first(), "x")?,
        z: integer(args.second(), "z")?,
        level: integer(args.third(), "level")?,
        name: None,
        action: required_text(args.fourth(), "action")?,
        id: Some(nonnegative(args.fifth(), "id")?),
    })
}

pub(crate) fn player_request(args: &Args) -> Result<InteractReq, Refusal> {
    Ok(InteractReq::Player {
        name: required_text(args.first(), "name")?,
        action: required_text(args.second(), "action")?,
    })
}

pub(crate) fn held_request(args: &Args) -> Result<InteractReq, Refusal> {
    Ok(InteractReq::Held {
        name: required_text(args.first(), "name")?,
        action: required_text(args.second(), "action")?,
        slot: Some(nonnegative(args.third(), "slot")?),
        target_item_id: Some(nonnegative(args.fourth(), "target_item_id")?),
    })
}

/// The value that resumes an awaited binding. `api.wait` and bank operations
/// resume with nil; a walk resumes with its result table.
pub(crate) fn completion(lua: &Lua, completion: &SeatCompletion) -> mlua::Result<Value> {
    match completion {
        SeatCompletion::Walk(walk) => walk_result(lua, walk).map(Value::Table),
        SeatCompletion::Aborted(reason) => {
            let result = lua.create_table()?;
            result.raw_set("ok", false)?;
            result.raw_set("failed", true)?;
            result.raw_set("aborted", abort_reason(*reason))?;
            Ok(Value::Table(result))
        }
        // Bank starts and `api.wait` have no result value. The current surface
        // has no binding that awaits the other handled native families.
        SeatCompletion::NextTick
        | SeatCompletion::Terminal(_)
        | SeatCompletion::Gather(_)
        | SeatCompletion::Combat(_)
        | SeatCompletion::Progress(_) => Ok(Value::Nil),
    }
}

/// The walk outcome fields of the JS v2 snapshot, without their
/// `walk_outcome_` prefix, plus `ok`.
fn walk_result(lua: &Lua, walk: &WalkCompletion) -> mlua::Result<Table> {
    let result = lua.create_table()?;
    result.raw_set("ok", !walk.failed)?;
    result.raw_set("failed", walk.failed)?;
    result.raw_set("x", walk.x)?;
    result.raw_set("z", walk.z)?;
    result.raw_set("level", walk.level)?;
    result.raw_set("radius", walk.radius)?;
    result.raw_set("blocked", walk.blocked)?;
    result.raw_set("no_path", walk.no_path)?;
    result.raw_set("allow_teleports", walk.allow_teleports)?;
    result.raw_set(
        "cancel_reason",
        match walk.cancel_reason {
            WalkCancelReason::None => "none",
            WalkCancelReason::UserInput => "user-input",
        },
    )?;
    result.raw_set("request_id", walk.id.get() as f64)?;
    result.raw_set("generation", walk.generation as f64)?;
    result.raw_set("seq", walk.sequence as f64)?;
    Ok(result)
}

fn abort_reason(reason: api::script_contract::AbortReason) -> &'static str {
    match reason {
        api::script_contract::AbortReason::Reset => "reset",
        api::script_contract::AbortReason::Superseded => "superseded",
        api::script_contract::AbortReason::Terminated => "terminated",
        api::script_contract::AbortReason::UserInput => "user-input",
        api::script_contract::AbortReason::ParentAborted => "parent-aborted",
    }
}
