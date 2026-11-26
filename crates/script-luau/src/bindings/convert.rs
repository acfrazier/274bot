//! Argument and result conversion for the bindings. A function here shapes
//! values. It holds no host and cannot act (`design-r3.md` 7.4 rule T4).
//!
//! A Luau value becomes a Rust number only through [`integer`] and the
//! fallback readers here. They match the value kinds; they never use the
//! numeric `FromLua` of `mlua`, which converts a numeric string
//! (ADR 0023 item 26).

use api::script_contract::MAX_START_CHARGE;

use mlua::{BorrowedStr, Function, Lua, LuaString, MultiValue, Table, Value};
use script::api_combat::CombatSessionRequest;
use script::api_progress::{QuestPathRow, QuestStatusError};
use script::bank::request::{BankSeatArgs, BankSeatOperation};
use script::interact::InteractReq;
use script::native::request::NativeKind;
use script::native::seat::SeatTerminal;
use script::native::snapshot_source::{QuestStatusInput, TileInput};
use script::native::{SettingsBag, WalkCancelReason};
use script::surface::client::{Awaited, ClientError, SeatCompletion, WalkCompletion};

/// The most arguments that a fixed-arity binding takes.
pub(crate) const MAX_ARGS: usize = 5;
pub(crate) const RAW_NEXT_KEY: &str = "__274bot_raw_next";
const MAX_LOG_LINE_BYTES: usize = 4 * 1024;
const MAX_LOG_ARGS: usize = 256;
const STOP_REASON_MAX_BYTES: usize = 256;
const UNKNOWN_KEY_MAX_BYTES: usize = 64;
const LIST_17: &[i64] = &[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17];
const LIST_18: &[i64] = &[
    1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18,
];
const LIST_33: &[i64] = &[
    1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26,
    27, 28, 29, 30, 31, 32, 33,
];
const LIST_34: &[i64] = &[
    1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26,
    27, 28, 29, 30, 31, 32, 33, 34,
];
const STEPS_4: &[usize] = &[0, 1, 2, 3];
const STEPS_5: &[usize] = &[0, 1, 2, 3, 4];
const STEPS_6: &[usize] = &[0, 1, 2, 3, 4, 5];
const STEPS_18: &[usize] = &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17];
const STEPS_26: &[usize] = &[
    0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25,
];
const COMBAT_TARGET_KEYS: &[&str] = &["npc", "npcId", "attackers", "player"];
const AREA_BOX_KEYS: &[&str] = &["min_x", "min_z", "max_x", "max_z", "level"];
const TILE_KEYS: &[&str] = &["x", "z", "level"];
const COMBAT_ALLOWED_KEYS: &[&str] = &[
    "target",
    "pick",
    "notTargetingOthers",
    "area",
    "radius",
    "lostRadius",
    "style",
    "meleeMode",
    "rangedMode",
    "spells",
    "fallbackSpells",
    "prayer",
    "food",
    "potions",
    "retaliate",
    "budgetTicks",
    "pvp",
];
const GATHER_ALLOWED_KEYS: &[&str] = &[
    "skill",
    "woodcuttingResources",
    "miningResources",
    "fishingMethod",
    "targetPreference",
    "tickMethod",
    "location",
    "site",
    "customTile",
    "radius",
    "disposition",
    "bank",
    "baitTarget",
    "buyTools",
    "food",
    "foodTarget",
    "eatBelow",
    "coinTarget",
    "reserveTeleport",
    "reserveCasts",
    "allowTeleports",
    "allowWilderness",
    "allowDangerZones",
    "deathPolicy",
    "maxDeaths",
];
const COMBAT_TEXT_KEYS: &[&str] = &["pick", "style", "meleeMode", "rangedMode"];
const COMBAT_INT_KEYS: &[&str] = &["radius", "lostRadius", "budgetTicks"];
const COMBAT_BOOL_KEYS: &[&str] = &[
    "notTargetingOthers",
    "fallbackSpells",
    "prayer",
    "food",
    "potions",
    "retaliate",
    "pvp",
];
const GATHER_TEXT_KEYS: &[&str] = &[
    "skill",
    "fishingMethod",
    "targetPreference",
    "tickMethod",
    "location",
    "site",
    "disposition",
    "bank",
    "food",
    "reserveTeleport",
    "deathPolicy",
];
const GATHER_LIST_KEYS: &[&str] = &["woodcuttingResources", "miningResources"];
const GATHER_INT_KEYS: &[&str] = &[
    "radius",
    "baitTarget",
    "foodTarget",
    "eatBelow",
    "coinTarget",
    "reserveCasts",
    "maxDeaths",
];
const GATHER_BOOL_KEYS: &[&str] = &[
    "buyTools",
    "allowTeleports",
    "allowWilderness",
    "allowDangerZones",
];

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
    /// One interaction text argument exceeds its byte bound.
    TextTooLong {
        argument: &'static str,
        max_bytes: usize,
    },
    /// More arguments than the member takes.
    Arity(usize),
    /// A read outside an entry that lends game input.
    NoInput,
    /// The run refuses the call in this entry or state.
    Run(&'static str),
    /// One core validation refusal string (`CombatRequestError` or `settings_refusal`).
    Detail(String),
    /// An unrecognized key in a structured request or settings table.
    UnknownKey { context: &'static str, key: String },
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
            Ok(text) if text.len() <= MAX_START_CHARGE => Ok(text.to_owned()),
            Ok(_) => Err(Refusal::TextTooLong {
                argument,
                max_bytes: MAX_START_CHARGE,
            }),
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

fn copy_text_fields(
    table: &Table,
    keys: &[&'static str],
    out: &mut SettingsBag,
) -> Result<(), Refusal> {
    for &field in keys {
        match table.raw_get::<Value>(field)? {
            Value::Nil => {}
            other => {
                let text = required_text(&other, field)?;
                out.insert(String::from(field), serde_json::Value::String(text));
            }
        }
    }
    Ok(())
}

fn copy_int_fields(
    table: &Table,
    keys: &[&'static str],
    out: &mut SettingsBag,
) -> Result<(), Refusal> {
    for &field in keys {
        match table.raw_get::<Value>(field)? {
            Value::Nil => {}
            other => {
                let number = integer(&other, field)?;
                out.insert(String::from(field), serde_json::Value::from(number));
            }
        }
    }
    Ok(())
}

fn copy_bool_fields(
    table: &Table,
    keys: &[&'static str],
    out: &mut SettingsBag,
) -> Result<(), Refusal> {
    for &field in keys {
        match table.raw_get::<Value>(field)? {
            Value::Nil => {}
            Value::Boolean(flag) => {
                out.insert(String::from(field), serde_json::Value::Bool(flag));
            }
            _ => return Err(refuse(field, "a boolean or nil")),
        }
    }
    Ok(())
}

fn known_key(allowed: &[&'static str], bytes: &[u8]) -> bool {
    for &entry in allowed {
        if entry.as_bytes() == bytes {
            return true;
        }
    }
    false
}

fn bounded_key(bytes: &[u8]) -> String {
    let end = bytes.len().min(UNKNOWN_KEY_MAX_BYTES);
    String::from_utf8_lossy(&bytes[..end]).into_owned()
}

fn reject_unknown_keys(
    lua: &Lua,
    table: &Table,
    allowed: &[&'static str],
    steps: &[usize],
    context: &'static str,
) -> Result<(), Refusal> {
    let next_fn: Function = lua.named_registry_value(RAW_NEXT_KEY)?;
    let mut cursor = Value::Nil;
    for _ in steps {
        let (key, _): (Value, Value) = next_fn.call((table, cursor))?;
        match &key {
            Value::Nil => return Ok(()),
            Value::String(text) => {
                let bytes = text.as_bytes();
                if !known_key(allowed, &bytes) {
                    return Err(Refusal::UnknownKey {
                        context,
                        key: bounded_key(&bytes),
                    });
                }
            }
            _ => return Err(refuse(context, "a table with only known string keys")),
        }
        cursor = key;
    }
    Err(refuse(context, "a table with only known string keys"))
}

fn sequence_index_in_bounds(value: &Value, count: usize) -> bool {
    match value {
        Value::Integer(number) => *number >= 1 && (*number as usize) <= count,
        Value::Number(number) => {
            number.is_finite()
                && number.fract() == 0.0
                && *number >= 1.0
                && *number <= (count as f64)
        }
        _ => false,
    }
}

fn reject_non_sequence_keys(
    lua: &Lua,
    table: &Table,
    count: usize,
    steps: &[i64],
    field: &'static str,
) -> Result<(), Refusal> {
    let next_fn: Function = lua.named_registry_value(RAW_NEXT_KEY)?;
    let mut cursor = Value::Nil;
    for _ in steps {
        let (key, _): (Value, Value) = next_fn.call((table, cursor))?;
        match key {
            Value::Nil => return Ok(()),
            other => {
                if !sequence_index_in_bounds(&other, count) {
                    return Err(refuse(field, "a contiguous list table"));
                }
                cursor = other;
            }
        }
    }
    Err(refuse(field, "a contiguous list table"))
}

fn text_array(
    lua: &Lua,
    table: &Table,
    slots: &[i64],
    steps: &[i64],
    field: &'static str,
) -> Result<serde_json::Value, Refusal> {
    let mut items = Vec::new();
    for &index in slots {
        match table.raw_get::<Value>(index)? {
            Value::Nil => break,
            other => Vec::push(
                &mut items,
                serde_json::Value::String(required_text(&other, field)?),
            ),
        }
    }
    reject_non_sequence_keys(lua, table, items.len(), steps, field)?;
    Ok(serde_json::Value::Array(items))
}

fn int_array(
    lua: &Lua,
    table: &Table,
    slots: &[i64],
    steps: &[i64],
    field: &'static str,
) -> Result<serde_json::Value, Refusal> {
    let mut items = Vec::new();
    for &index in slots {
        match table.raw_get::<Value>(index)? {
            Value::Nil => break,
            other => Vec::push(&mut items, serde_json::Value::from(integer(&other, field)?)),
        }
    }
    reject_non_sequence_keys(lua, table, items.len(), steps, field)?;
    Ok(serde_json::Value::Array(items))
}

fn copy_list_fields(
    lua: &Lua,
    table: &Table,
    keys: &[&'static str],
    out: &mut SettingsBag,
) -> Result<(), Refusal> {
    for &field in keys {
        match table.raw_get::<Value>(field)? {
            Value::Nil => {}
            Value::Table(list) => {
                out.insert(
                    String::from(field),
                    text_array(lua, &list, LIST_33, LIST_34, field)?,
                );
            }
            _ => return Err(refuse(field, "a table of strings or nil")),
        }
    }
    Ok(())
}

fn area_box_json(lua: &Lua, value: &Value) -> Result<serde_json::Value, Refusal> {
    let Value::Table(table) = value else {
        return Err(refuse("area", "a region table"));
    };
    reject_unknown_keys(lua, table, AREA_BOX_KEYS, STEPS_6, "area")?;
    let mut box_map = SettingsBag::new();
    copy_int_fields(table, AREA_BOX_KEYS, &mut box_map)?;
    Ok(serde_json::Value::Object(box_map))
}

fn area_array(lua: &Lua, table: &Table, slots: &[i64]) -> Result<serde_json::Value, Refusal> {
    let mut boxes = Vec::new();
    for &index in slots {
        match table.raw_get::<Value>(index)? {
            Value::Nil => break,
            other => Vec::push(&mut boxes, area_box_json(lua, &other)?),
        }
    }
    reject_non_sequence_keys(lua, table, boxes.len(), LIST_18, "area")?;
    Ok(serde_json::Value::Array(boxes))
}

fn combat_target_json(lua: &Lua, value: &Value) -> Result<serde_json::Value, Refusal> {
    let Value::Table(table) = value else {
        return Err(refuse("target", "a table"));
    };
    reject_unknown_keys(lua, table, COMBAT_TARGET_KEYS, STEPS_5, "target")?;
    let mut target = SettingsBag::new();
    match table.raw_get::<Value>("npc")? {
        Value::Nil => {}
        Value::String(_) => {
            let text = required_text(&table.raw_get::<Value>("npc")?, "target.npc")?;
            target.insert(String::from("npc"), serde_json::Value::String(text));
        }
        Value::Table(list) => {
            target.insert(
                String::from("npc"),
                text_array(lua, &list, LIST_33, LIST_34, "target.npc")?,
            );
        }
        _ => return Err(refuse("target.npc", "a string or table of strings")),
    }
    match table.raw_get::<Value>("npcId")? {
        Value::Nil => {}
        Value::Table(list) => {
            target.insert(
                String::from("npcId"),
                int_array(lua, &list, LIST_33, LIST_34, "target.npcId")?,
            );
        }
        other => {
            let id = integer(&other, "target.npcId")?;
            target.insert(String::from("npcId"), serde_json::Value::from(id));
        }
    }
    copy_text_fields(table, &["attackers", "player"], &mut target)?;
    Ok(serde_json::Value::Object(target))
}

pub(crate) fn combat_request(lua: &Lua, args: &Args) -> Result<CombatSessionRequest, Refusal> {
    let Value::Table(table) = args.first() else {
        return Err(refuse("request", "a table"));
    };
    reject_unknown_keys(lua, table, COMBAT_ALLOWED_KEYS, STEPS_18, "request")?;
    let mut map = SettingsBag::new();
    match table.raw_get::<Value>("target")? {
        Value::Nil => {}
        other => {
            map.insert(String::from("target"), combat_target_json(lua, &other)?);
        }
    }
    match table.raw_get::<Value>("area")? {
        Value::Nil => {}
        Value::Table(area) => {
            let encoded = if area.is_empty() {
                serde_json::Value::Array(Vec::new())
            } else {
                match area.raw_get::<Value>(1)? {
                    Value::Nil => area_box_json(lua, &Value::Table(area))?,
                    _ => area_array(lua, &area, LIST_17)?,
                }
            };
            map.insert(String::from("area"), encoded);
        }
        _ => return Err(refuse("area", "a region table or list of region tables")),
    }
    match table.raw_get::<Value>("spells")? {
        Value::Nil => {}
        Value::Table(list) => {
            map.insert(
                String::from("spells"),
                text_array(lua, &list, LIST_33, LIST_34, "spells")?,
            );
        }
        _ => return Err(refuse("spells", "a table of strings")),
    }
    copy_text_fields(table, COMBAT_TEXT_KEYS, &mut map)?;
    copy_int_fields(table, COMBAT_INT_KEYS, &mut map)?;
    copy_bool_fields(table, COMBAT_BOOL_KEYS, &mut map)?;
    match CombatSessionRequest::from_value(&serde_json::Value::Object(map)) {
        Ok(request) => Ok(request),
        Err(error) => Err(Refusal::Detail(error.to_string())),
    }
}

pub(crate) fn gather_settings(lua: &Lua, args: &Args) -> Result<SettingsBag, Refusal> {
    let mut bag = SettingsBag::new();
    let table = match args.first() {
        Value::Nil => return Ok(bag),
        Value::Table(table) => table,
        _ => return Err(refuse("settings", "a table or nil")),
    };
    reject_unknown_keys(lua, table, GATHER_ALLOWED_KEYS, STEPS_26, "settings")?;
    copy_text_fields(table, GATHER_TEXT_KEYS, &mut bag)?;
    copy_list_fields(lua, table, GATHER_LIST_KEYS, &mut bag)?;
    copy_int_fields(table, GATHER_INT_KEYS, &mut bag)?;
    copy_bool_fields(table, GATHER_BOOL_KEYS, &mut bag)?;
    match table.raw_get::<Value>("customTile")? {
        Value::Nil => {}
        Value::Table(tile) => {
            reject_unknown_keys(lua, &tile, TILE_KEYS, STEPS_4, "customTile")?;
            let mut tile_map = SettingsBag::new();
            copy_int_fields(&tile, TILE_KEYS, &mut tile_map)?;
            bag.insert(
                String::from("customTile"),
                serde_json::Value::Object(tile_map),
            );
        }
        _ => return Err(refuse("customTile", "a tile table or nil")),
    }
    Ok(bag)
}

pub(crate) fn quest_name(args: &Args) -> Result<String, Refusal> {
    let name = required_text(args.first(), "quest")?;
    if name.trim().is_empty() {
        return Err(refuse("quest", "a non-empty string"));
    }
    Ok(name)
}

pub(crate) fn quest_status_out(
    lua: &Lua,
    name: &Value,
    rows: Option<&[QuestStatusInput<'_>]>,
    tick: u64,
) -> Reply {
    let name = required_text(name, "name")?;
    let out = lua.create_table()?;
    match script::api_progress::resolve_quest_status_inputs(&name, rows, tick) {
        Ok(hit) => {
            out.raw_set("ok", true)?;
            out.raw_set("status", lua.create_string(hit.status)?)?;
            out.raw_set("as_of_sequence", hit.as_of_sequence as f64)?;
        }
        Err(error) => {
            out.raw_set("ok", false)?;
            out.raw_set("error", quest_status_error(error))?;
        }
    }
    Ok(Out::One(Value::Table(out)))
}

fn quest_status_error(error: QuestStatusError) -> &'static str {
    error.as_str()
}

fn stages_table(lua: &Lua, stages: &[std::sync::Arc<str>]) -> mlua::Result<Table> {
    let out = lua.create_table()?;
    for (index, stage) in stages.iter().enumerate() {
        out.raw_set((index + 1) as i64, lua.create_string(stage.as_ref())?)?;
    }
    Ok(out)
}

pub(crate) fn quest_paths_out(lua: &Lua, rows: &[QuestPathRow]) -> Reply {
    let out = lua.create_table()?;
    for (index, row) in rows.iter().enumerate() {
        let entry = lua.create_table()?;
        entry.raw_set("id", lua.create_string(row.id.as_ref())?)?;
        entry.raw_set("display", lua.create_string(row.display.as_ref())?)?;
        entry.raw_set("journal", row.journal)?;
        entry.raw_set("draft", row.draft)?;
        entry.raw_set("stages", stages_table(lua, &row.stages)?)?;
        out.raw_set((index + 1) as i64, entry)?;
    }
    Ok(Out::One(Value::Table(out)))
}

/// The value that resumes an awaited binding. Session and progress awaits
/// resume with the JS v2 outcome shape; bank awaits return a tagged seat
/// result; a walk returns its result table, and `api.wait` returns nil.
pub(crate) fn completion(
    lua: &Lua,
    awaited: Option<Awaited>,
    completion: &SeatCompletion,
) -> mlua::Result<Value> {
    match completion {
        SeatCompletion::Terminal { result, value } => {
            if is_session_family(awaited) {
                session_terminal_result(lua, result.outcome).map(Value::Table)
            } else {
                bank_result(lua, result.outcome, value.as_deref()).map(Value::Table)
            }
        }
        SeatCompletion::Gather(end) => {
            session_done_result(lua, serde_json::Value::from(end.clone())).map(Value::Table)
        }
        SeatCompletion::Combat(end) => {
            session_done_result(lua, serde_json::Value::from(end.clone())).map(Value::Table)
        }
        SeatCompletion::Progress(page) => {
            session_done_result(lua, serde_json::Value::from(page.clone())).map(Value::Table)
        }
        SeatCompletion::Walk(walk) => walk_result(lua, walk).map(Value::Table),
        SeatCompletion::Aborted { awaited, reason } => match awaited {
            Awaited::NextTick => Ok(Value::Nil),
            Awaited::Walk { .. } => walk_aborted_result(lua, *reason).map(Value::Table),
            Awaited::Seat { .. } if is_session_family(Some(*awaited)) => {
                session_aborted_result(lua, *reason).map(Value::Table)
            }
            Awaited::Seat { .. } => {
                bank_result(lua, SeatTerminal::Aborted(*reason), None).map(Value::Table)
            }
        },
        SeatCompletion::NextTick => Ok(Value::Nil),
    }
}

fn is_session_family(awaited: Option<Awaited>) -> bool {
    matches!(
        awaited,
        Some(Awaited::Seat {
            kind: NativeKind::Gather | NativeKind::Combat | NativeKind::QuestProgress,
            ..
        })
    )
}

fn session_done_result(lua: &Lua, value: serde_json::Value) -> mlua::Result<Table> {
    let result = lua.create_table()?;
    result.raw_set("kind", "done")?;
    result.raw_set("value", channel_value(lua, value)?)?;
    Ok(result)
}

fn session_aborted_result(
    lua: &Lua,
    reason: api::script_contract::AbortReason,
) -> mlua::Result<Table> {
    let result = lua.create_table()?;
    result.raw_set("kind", "aborted")?;
    result.raw_set("reason", abort_reason(reason))?;
    Ok(result)
}

fn session_terminal_result(lua: &Lua, outcome: SeatTerminal) -> mlua::Result<Table> {
    let result = lua.create_table()?;
    match outcome {
        SeatTerminal::Refused(reason) => {
            result.raw_set("kind", "refused")?;
            result.raw_set("reason", reason.to_string())?;
        }
        SeatTerminal::Aborted(reason) => {
            result.raw_set("kind", "aborted")?;
            result.raw_set("reason", abort_reason(reason))?;
        }
        SeatTerminal::Done => {
            result.raw_set("kind", "done")?;
        }
        SeatTerminal::Failed => {
            result.raw_set("kind", "refused")?;
            result.raw_set("reason", "failed")?;
        }
    }
    Ok(result)
}

/// A typed bank terminal with the optional value published by the slot.
fn bank_result(lua: &Lua, outcome: SeatTerminal, value: Option<&[u8]>) -> mlua::Result<Table> {
    let result = lua.create_table()?;
    let (status, reason) = match outcome {
        SeatTerminal::Done => ("done", None),
        SeatTerminal::Failed => ("failed", None),
        SeatTerminal::Refused(reason) => ("refused", Some(format!("{reason:?}"))),
        SeatTerminal::Aborted(reason) => ("aborted", Some(abort_reason(reason).to_owned())),
    };
    result.raw_set("status", status)?;
    result.raw_set("has_value", value.is_some())?;
    if let Some(bytes) = value {
        let value = script::channel::decode(bytes).map_err(|error| {
            mlua::Error::RuntimeError(format!("bank seat result is invalid: {error}"))
        })?;
        result.raw_set("value", channel_value(lua, value)?)?;
    }
    if let Some(reason) = reason {
        result.raw_set("reason", reason)?;
    }
    Ok(result)
}

/// Convert the bounded structured-clone value to the corresponding Luau value.
fn channel_value(lua: &Lua, value: serde_json::Value) -> mlua::Result<Value> {
    match value {
        serde_json::Value::Null => Ok(Value::Nil),
        serde_json::Value::Bool(value) => Ok(Value::Boolean(value)),
        serde_json::Value::Number(number) => number
            .as_f64()
            .map(Value::Number)
            .ok_or_else(|| mlua::Error::RuntimeError("bank result number is invalid".to_owned())),
        serde_json::Value::String(value) => Ok(Value::String(lua.create_string(value)?)),
        serde_json::Value::Array(values) => {
            let table = lua.create_table()?;
            for (index, value) in values.into_iter().enumerate() {
                table.raw_set((index + 1) as i64, channel_value(lua, value)?)?;
            }
            Ok(Value::Table(table))
        }
        serde_json::Value::Object(values) => {
            let table = lua.create_table()?;
            for (key, value) in values {
                table.raw_set(key, channel_value(lua, value)?)?;
            }
            Ok(Value::Table(table))
        }
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

fn walk_aborted_result(
    lua: &Lua,
    reason: api::script_contract::AbortReason,
) -> mlua::Result<Table> {
    let result = lua.create_table()?;
    result.raw_set("ok", false)?;
    result.raw_set("failed", true)?;
    result.raw_set("aborted", abort_reason(reason))?;
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
