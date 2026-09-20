//! In-isolate Jive helper.
//!
//! XpTracker baselines, levelRow formats, paintLevels room, and
//! scriptFrame/jiveFrame branding live here. JS only marshals reader
//! callbacks and applies returned paint ops. Not a FlatBuffer RPC.

use rustyscript::Runtime;
use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::OnceLock;

const JIVE_ACCENT: &str = "#e05be0";
const JIVE_BYLINE: &str = "Jive scripts";
const COMBAT_SKILLS: &[&str] = &[
    "attack",
    "strength",
    "defence",
    "hitpoints",
    "ranged",
    "magic",
    "prayer",
];
const DEFAULT_EMPTY: &str = "no experience yet";
const DEFAULT_DOCK: &str = "chatbox";
const MAX_LEVEL: i32 = 99;

thread_local! {
    static STATE: RefCell<JiveState> = RefCell::new(JiveState::default());
}

#[derive(Default)]
struct JiveState {
    next_id: u32,
    baselines: HashMap<u32, HashMap<String, i32>>,
}

#[derive(Clone)]
struct SkillGain {
    skill: String,
    level: i32,
    xp: i32,
    gained: i32,
}

struct LevelRow {
    label: String,
    fraction: f64,
    cells: [String; 3],
}

pub(super) fn reset() {
    STATE.with(|s| {
        let mut st = s.borrow_mut();
        st.baselines.clear();
        st.next_id = 0;
    });
}

pub(super) fn install(runtime: &mut Runtime) -> Result<(), String> {
    let context = runtime.deno_runtime().main_context();
    let mut scope = runtime.deno_runtime().handle_scope();
    let global = context.open(&mut scope).global(&mut scope);
    let name = v8::String::new(&mut scope, "__rs2b0t_paint_jive")
        .ok_or_else(|| "paint jive name".to_string())?;
    let func = v8::Function::new(&mut scope, paint_jive_callback)
        .ok_or_else(|| "paint jive function".to_string())?;
    global
        .set(&mut scope, name.into(), func.into())
        .ok_or_else(|| "paint jive set".to_string())?;
    Ok(())
}

fn paint_jive_callback(
    scope: &mut v8::HandleScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    match run(scope, &args) {
        Ok(value) => rv.set(value),
        Err(reason) => match v8::String::new(scope, &reason) {
            Some(msg) => {
                let exc = v8::Exception::error(scope, msg);
                scope.throw_exception(exc);
            }
            None => rv.set(v8::undefined(scope).into()),
        },
    }
}

fn run<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: &v8::FunctionCallbackArguments,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let op = js_string_arg(scope, args.get(0));
    match op.as_str() {
        "accent" => js_string(scope, JIVE_ACCENT),
        "byline" => js_string(scope, JIVE_BYLINE),
        "combatSkills" => js_string_array_value(scope, COMBAT_SKILLS),
        "scriptFrame" => frame_cfg(scope, args.get(1), false),
        "jiveFrame" => frame_cfg(scope, args.get(1), true),
        "construct" => {
            let id = STATE.with(|s| {
                let mut st = s.borrow_mut();
                let id = st.next_id.saturating_add(1);
                st.next_id = id;
                st.baselines.entry(id).or_default();
                id
            });
            Ok(v8::Number::new(scope, f64::from(id)).into())
        }
        "begin" => {
            let id = js_opt_i32(scope, args.get(1), 0) as u32;
            let skills = js_string_array(scope, args.get(2))?;
            let xps = js_i32_array(scope, args.get(3))?;
            STATE.with(|s| {
                let mut st = s.borrow_mut();
                let map = st.baselines.entry(id).or_default();
                map.clear();
                for (skill, xp) in skills.into_iter().zip(xps) {
                    map.insert(skill, xp);
                }
            });
            Ok(v8::undefined(scope).into())
        }
        "progress" => {
            let gains = collect_progress(scope, args)?;
            js_gains(scope, &gains)
        }
        "gains" => {
            let mut gains = collect_progress(scope, args)?;
            gains.retain(|g| g.gained > 0);
            gains.sort_by(|a, b| b.gained.cmp(&a.gained));
            js_gains(scope, &gains)
        }
        "levelRow" => {
            let gain = js_gain(scope, args.get(1))?;
            let mins = js_opt_f64(scope, args.get(2), 0.0);
            js_level_row(scope, &level_row(&gain, mins))
        }
        "paintLevels" => {
            let gains = js_gain_array(scope, args.get(1))?;
            let mins = js_opt_f64(scope, args.get(2), 0.0);
            let reserve = js_opt_i32(scope, args.get(3), 0);
            let empty = if args.get(4).is_null_or_undefined() {
                DEFAULT_EMPTY.to_string()
            } else {
                js_string_arg(scope, args.get(4))
            };
            let rows_left = js_opt_i32(scope, args.get(5), 0);
            js_paint_ops(
                scope,
                &paint_level_ops(&gains, mins, reserve, &empty, rows_left),
            )
        }
        other => Err(format!("unknown paint jive op {other}")),
    }
}

fn frame_cfg<'s>(
    scope: &mut v8::HandleScope<'s>,
    opts: v8::Local<v8::Value>,
    jive: bool,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let script = js_obj_string(scope, opts, "script", "");
    let dock = js_obj_string(scope, opts, "dock", DEFAULT_DOCK);
    let dock = if dock.is_empty() {
        DEFAULT_DOCK.to_string()
    } else {
        dock
    };
    let key_opt = js_obj_opt_string(scope, opts, "key");
    let (accent, byline, key) = if jive {
        let key = key_opt.unwrap_or_else(|| format!("jive:{script}"));
        (JIVE_ACCENT.to_string(), JIVE_BYLINE.to_string(), key)
    } else {
        let accent = js_obj_string(scope, opts, "accent", JIVE_ACCENT);
        let byline = js_obj_string(scope, opts, "byline", JIVE_BYLINE);
        let key = key_opt.unwrap_or(script);
        (
            if accent.is_empty() {
                JIVE_ACCENT.to_string()
            } else {
                accent
            },
            if byline.is_empty() {
                JIVE_BYLINE.to_string()
            } else {
                byline
            },
            key,
        )
    };
    let obj = v8::Object::new(scope);
    let dock = js_string(scope, &dock)?;
    set(scope, obj, "dock", dock)?;
    let accent = js_string(scope, &accent)?;
    set(scope, obj, "accent", accent)?;
    let byline = js_string(scope, &byline)?;
    set(scope, obj, "byline", byline)?;
    let key = js_string(scope, &key)?;
    set(scope, obj, "key", key)?;
    Ok(obj.into())
}

fn collect_progress(
    scope: &mut v8::HandleScope,
    args: &v8::FunctionCallbackArguments,
) -> Result<Vec<SkillGain>, String> {
    let id = js_opt_i32(scope, args.get(1), 0) as u32;
    let skills = js_string_array(scope, args.get(2))?;
    let xps = js_i32_array(scope, args.get(3))?;
    let levels = js_i32_array(scope, args.get(4))?;
    Ok(STATE.with(|s| {
        let st = s.borrow();
        let start = st.baselines.get(&id);
        skills
            .into_iter()
            .enumerate()
            .map(|(i, skill)| {
                let xp = xps.get(i).copied().unwrap_or(0);
                let level = levels.get(i).copied().unwrap_or(1);
                let gained = start
                    .and_then(|m| m.get(&skill).copied())
                    .map(|base| xp - base)
                    .unwrap_or(0);
                SkillGain {
                    skill,
                    level,
                    xp,
                    gained,
                }
            })
            .collect()
    }))
}

fn paint_level_ops(
    gains: &[SkillGain],
    mins: f64,
    reserve: i32,
    empty: &str,
    rows_left: i32,
) -> Vec<PaintOp> {
    if gains.is_empty() {
        return vec![PaintOp::Text(empty.to_string())];
    }
    let room = ((i64::from(rows_left) - i64::from(reserve)) / 2).max(0) as usize;
    gains
        .iter()
        .take(room)
        .flat_map(|g| {
            let row = level_row(g, mins);
            [
                PaintOp::Bar(row.label, row.fraction),
                PaintOp::Row(row.cells.to_vec()),
            ]
        })
        .collect()
}

enum PaintOp {
    Text(String),
    Bar(String, f64),
    Row(Vec<String>),
}

fn level_row(g: &SkillGain, mins: f64) -> LevelRow {
    let prog = level_progress(g.level, g.xp);
    let per_hour = if mins > 0.5 {
        format!("{}/hr", fmt_xp_hr(g.gained, mins))
    } else {
        "xp/hr n/a".to_string()
    };
    let label = format!("{} {}", paint_skill_short(&g.skill), prog.level);
    if prog.level >= MAX_LEVEL {
        return LevelRow {
            label,
            fraction: 1.0,
            cells: [per_hour, "maxed".to_string(), String::new()],
        };
    }
    let eta = eta_hours(
        prog.remaining,
        if mins > 0.5 {
            (g.gained as f64 / mins) * 60.0
        } else {
            0.0
        },
    );
    let eta_cell = match eta {
        None => "eta n/a".to_string(),
        Some(hours) => format!("eta {}", fmt_hms_from_mins(hours * 60.0)),
    };
    LevelRow {
        label,
        fraction: prog.fraction,
        cells: [
            per_hour,
            format!("{} to go", fmt_grouped(prog.remaining)),
            eta_cell,
        ],
    }
}

struct Progress {
    level: i32,
    fraction: f64,
    remaining: i32,
}

fn level_progress(level: i32, xp: i32) -> Progress {
    let cur = level.clamp(1, MAX_LEVEL);
    if cur >= MAX_LEVEL {
        return Progress {
            level: MAX_LEVEL,
            fraction: 1.0,
            remaining: 0,
        };
    }
    let base = xp_at_level(cur);
    let top = xp_at_level(cur + 1);
    let span = (top - base).max(1);
    let fraction = ((xp - base) as f64 / span as f64).clamp(0.0, 1.0);
    Progress {
        level: cur,
        fraction,
        remaining: (top - xp).max(0),
    }
}

fn eta_hours(remaining: i32, xp_per_hour: f64) -> Option<f64> {
    if remaining <= 0 || xp_per_hour <= 0.0 || !xp_per_hour.is_finite() {
        return None;
    }
    let hours = remaining as f64 / xp_per_hour;
    hours.is_finite().then_some(hours)
}

fn xp_table() -> &'static [i32; 100] {
    static TABLE: OnceLock<[i32; 100]> = OnceLock::new();
    TABLE.get_or_init(|| {
        let mut out = [0; 100];
        let mut points = 0.0;
        for level in 1..MAX_LEVEL {
            points += (f64::from(level) + 300.0 * 2.0_f64.powf(f64::from(level) / 7.0)).floor();
            out[(level + 1) as usize] = (points / 4.0).floor() as i32;
        }
        out
    })
}

fn xp_at_level(level: i32) -> i32 {
    let i = level.clamp(1, MAX_LEVEL) as usize;
    xp_table()[i]
}

fn fmt_xp_hr(gained: i32, mins: f64) -> String {
    format!("{:.1}k", ((gained as f64 / mins) * 60.0) / 1000.0)
}

fn fmt_hms_from_mins(mins: f64) -> String {
    let t = (mins * 60.0).floor().max(0.0) as i64;
    format!("{}:{:02}:{:02}", t / 3600, (t % 3600) / 60, t % 60)
}

fn fmt_grouped(n: i32) -> String {
    let s = n.max(0).to_string();
    let bytes = s.as_bytes();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    for (i, ch) in s.chars().enumerate() {
        if i > 0 && (bytes.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

fn paint_skill_short(skill: &str) -> &str {
    match skill {
        "woodcutting" => "WC",
        "firemaking" => "FM",
        "fishing" => "Fish",
        "cooking" => "Cook",
        "mining" => "Mine",
        "attack" => "Att",
        "strength" => "Str",
        "defence" => "Def",
        "hitpoints" => "HP",
        "ranged" => "Range",
        "magic" => "Mage",
        "prayer" => "Pray",
        "crafting" => "Craft",
        "smithing" => "Smith",
        "herblore" => "Herb",
        "agility" => "Agil",
        "thieving" => "Thief",
        "fletching" => "Fletch",
        "runecraft" => "RC",
        other => other,
    }
}

fn js_string_arg(scope: &mut v8::HandleScope, value: v8::Local<v8::Value>) -> String {
    if value.is_null_or_undefined() {
        return String::new();
    }
    value
        .to_string(scope)
        .map(|s| s.to_rust_string_lossy(scope))
        .unwrap_or_default()
}

fn js_opt_i32(scope: &mut v8::HandleScope, value: v8::Local<v8::Value>, default: i32) -> i32 {
    if value.is_null_or_undefined() {
        return default;
    }
    value
        .number_value(scope)
        .filter(|n| n.is_finite())
        .map(|n| n.trunc() as i32)
        .unwrap_or(default)
}

fn js_opt_f64(scope: &mut v8::HandleScope, value: v8::Local<v8::Value>, default: f64) -> f64 {
    if value.is_null_or_undefined() {
        return default;
    }
    value
        .number_value(scope)
        .filter(|n| n.is_finite())
        .unwrap_or(default)
}

fn js_string_array(
    scope: &mut v8::HandleScope,
    value: v8::Local<v8::Value>,
) -> Result<Vec<String>, String> {
    if value.is_null_or_undefined() {
        return Ok(Vec::new());
    }
    let arr = value
        .to_object(scope)
        .and_then(|o| v8::Local::<v8::Array>::try_from(o).ok())
        .ok_or_else(|| "expected string array".to_string())?;
    let len = arr.length();
    let mut out = Vec::with_capacity(len as usize);
    for i in 0..len {
        let item = arr
            .get_index(scope, i)
            .ok_or_else(|| "array get failed".to_string())?;
        out.push(js_string_arg(scope, item));
    }
    Ok(out)
}

fn js_i32_array(
    scope: &mut v8::HandleScope,
    value: v8::Local<v8::Value>,
) -> Result<Vec<i32>, String> {
    if value.is_null_or_undefined() {
        return Ok(Vec::new());
    }
    let arr = value
        .to_object(scope)
        .and_then(|o| v8::Local::<v8::Array>::try_from(o).ok())
        .ok_or_else(|| "expected i32 array".to_string())?;
    let len = arr.length();
    let mut out = Vec::with_capacity(len as usize);
    for i in 0..len {
        let item = arr
            .get_index(scope, i)
            .ok_or_else(|| "array get failed".to_string())?;
        out.push(js_opt_i32(scope, item, 0));
    }
    Ok(out)
}

fn js_gain(scope: &mut v8::HandleScope, value: v8::Local<v8::Value>) -> Result<SkillGain, String> {
    Ok(SkillGain {
        skill: js_obj_string(scope, value, "skill", ""),
        level: js_obj_i32(scope, value, "level", 1),
        xp: js_obj_i32(scope, value, "xp", 0),
        gained: js_obj_i32(scope, value, "gained", 0),
    })
}

fn js_gain_array(
    scope: &mut v8::HandleScope,
    value: v8::Local<v8::Value>,
) -> Result<Vec<SkillGain>, String> {
    if value.is_null_or_undefined() {
        return Ok(Vec::new());
    }
    let arr = value
        .to_object(scope)
        .and_then(|o| v8::Local::<v8::Array>::try_from(o).ok())
        .ok_or_else(|| "expected gains array".to_string())?;
    let len = arr.length();
    let mut out = Vec::with_capacity(len as usize);
    for i in 0..len {
        let item = arr
            .get_index(scope, i)
            .ok_or_else(|| "gains get failed".to_string())?;
        out.push(js_gain(scope, item)?);
    }
    Ok(out)
}

fn js_obj_opt_string(
    scope: &mut v8::HandleScope,
    value: v8::Local<v8::Value>,
    key: &str,
) -> Option<String> {
    let obj = value.to_object(scope)?;
    let key = v8::String::new(scope, key)?;
    let got = obj.get(scope, key.into())?;
    if got.is_null_or_undefined() {
        return None;
    }
    Some(js_string_arg(scope, got))
}

fn js_obj_string(
    scope: &mut v8::HandleScope,
    value: v8::Local<v8::Value>,
    key: &str,
    default: &str,
) -> String {
    js_obj_opt_string(scope, value, key)
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| default.to_string())
}

fn js_obj_i32(
    scope: &mut v8::HandleScope,
    value: v8::Local<v8::Value>,
    key: &str,
    default: i32,
) -> i32 {
    let Some(obj) = value.to_object(scope) else {
        return default;
    };
    let Some(key) = v8::String::new(scope, key) else {
        return default;
    };
    let Some(got) = obj.get(scope, key.into()) else {
        return default;
    };
    js_opt_i32(scope, got, default)
}

fn js_string<'s>(
    scope: &mut v8::HandleScope<'s>,
    s: &str,
) -> Result<v8::Local<'s, v8::Value>, String> {
    v8::String::new(scope, s)
        .map(|v| v.into())
        .ok_or_else(|| "v8 string alloc failed".to_string())
}

fn js_string_array_value<'s>(
    scope: &mut v8::HandleScope<'s>,
    lines: &[&str],
) -> Result<v8::Local<'s, v8::Value>, String> {
    let arr = v8::Array::new(scope, lines.len() as i32);
    for (i, line) in lines.iter().enumerate() {
        let value = js_string(scope, line)?;
        arr.set_index(scope, i as u32, value)
            .ok_or_else(|| "v8 array set failed".to_string())?;
    }
    Ok(arr.into())
}

fn set<'s>(
    scope: &mut v8::HandleScope<'s>,
    obj: v8::Local<'s, v8::Object>,
    key: &str,
    value: v8::Local<'s, v8::Value>,
) -> Result<(), String> {
    let key = v8::String::new(scope, key).ok_or_else(|| "v8 key alloc failed".to_string())?;
    obj.set(scope, key.into(), value)
        .ok_or_else(|| "v8 set failed".to_string())?;
    Ok(())
}

fn js_gains<'s>(
    scope: &mut v8::HandleScope<'s>,
    gains: &[SkillGain],
) -> Result<v8::Local<'s, v8::Value>, String> {
    let arr = v8::Array::new(scope, gains.len() as i32);
    for (i, g) in gains.iter().enumerate() {
        let obj = v8::Object::new(scope);
        let skill = js_string(scope, &g.skill)?;
        set(scope, obj, "skill", skill)?;
        let level = v8::Number::new(scope, f64::from(g.level)).into();
        set(scope, obj, "level", level)?;
        let xp = v8::Number::new(scope, f64::from(g.xp)).into();
        set(scope, obj, "xp", xp)?;
        let gained = v8::Number::new(scope, f64::from(g.gained)).into();
        set(scope, obj, "gained", gained)?;
        arr.set_index(scope, i as u32, obj.into())
            .ok_or_else(|| "v8 array set failed".to_string())?;
    }
    Ok(arr.into())
}

fn js_level_row<'s>(
    scope: &mut v8::HandleScope<'s>,
    row: &LevelRow,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let obj = v8::Object::new(scope);
    let label = js_string(scope, &row.label)?;
    set(scope, obj, "label", label)?;
    let fraction = v8::Number::new(scope, row.fraction).into();
    set(scope, obj, "fraction", fraction)?;
    let cells = v8::Array::new(scope, 3);
    for (i, cell) in row.cells.iter().enumerate() {
        let value = js_string(scope, cell)?;
        cells
            .set_index(scope, i as u32, value)
            .ok_or_else(|| "v8 cell set failed".to_string())?;
    }
    set(scope, obj, "cells", cells.into())?;
    Ok(obj.into())
}

fn js_paint_ops<'s>(
    scope: &mut v8::HandleScope<'s>,
    ops: &[PaintOp],
) -> Result<v8::Local<'s, v8::Value>, String> {
    let arr = v8::Array::new(scope, ops.len() as i32);
    for (i, op) in ops.iter().enumerate() {
        let obj = v8::Object::new(scope);
        match op {
            PaintOp::Text(text) => {
                let kind = js_string(scope, "text")?;
                set(scope, obj, "kind", kind)?;
                let text = js_string(scope, text)?;
                set(scope, obj, "text", text)?;
            }
            PaintOp::Bar(label, fraction) => {
                let kind = js_string(scope, "bar")?;
                set(scope, obj, "kind", kind)?;
                let label = js_string(scope, label)?;
                set(scope, obj, "label", label)?;
                let fraction = v8::Number::new(scope, *fraction).into();
                set(scope, obj, "fraction", fraction)?;
            }
            PaintOp::Row(cells) => {
                let kind = js_string(scope, "row")?;
                set(scope, obj, "kind", kind)?;
                let list = v8::Array::new(scope, cells.len() as i32);
                for (j, cell) in cells.iter().enumerate() {
                    let value = js_string(scope, cell)?;
                    list.set_index(scope, j as u32, value)
                        .ok_or_else(|| "v8 row cell set failed".to_string())?;
                }
                set(scope, obj, "cells", list.into())?;
            }
        }
        arr.set_index(scope, i as u32, obj.into())
            .ok_or_else(|| "v8 op set failed".to_string())?;
    }
    Ok(arr.into())
}
