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
        "frame" => frame_plan(scope, args.get(1)),
        // The reference `paintLogic.ts` / `levelProgress.ts` formatters, served
        // here so the shim has one owner for every paint string.
        "fmtDuration" => {
            let mins = js_f64(scope, args.get(1), 0.0);
            js_string(scope, &fmt_duration(mins))
        }
        "fmtXpHr" => {
            let gained = js_f64(scope, args.get(1), 0.0);
            let mins = js_f64(scope, args.get(2), 0.0);
            js_string(scope, &fmt_xp_hr(gained, mins))
        }
        "paintSkillShort" => {
            let skill = js_string_arg(scope, args.get(1));
            js_string(scope, paint_skill_short(&skill))
        }
        "xpAtLevel" => {
            let level = js_f64(scope, args.get(1), 0.0);
            Ok(v8::Number::new(scope, xp_at_level(level)).into())
        }
        "levelProgress" => {
            let level = js_f64(scope, args.get(1), 0.0);
            let xp = js_f64(scope, args.get(2), 0.0);
            js_progress(scope, &level_progress(level, xp))
        }
        "etaHours" => {
            let remaining = js_f64(scope, args.get(1), 0.0);
            let xp_per_hour = js_f64(scope, args.get(2), 0.0);
            match eta_hours(remaining, xp_per_hour) {
                Some(hours) => Ok(v8::Number::new(scope, hours).into()),
                None => Ok(v8::null(scope).into()),
            }
        }
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
            gains.sort_by_key(|a| std::cmp::Reverse(a.gained));
            js_gains(scope, &gains)
        }
        "levelRow" => {
            let gain = js_gain(scope, args.get(1))?;
            let mins = js_f64(scope, args.get(2), 0.0);
            js_level_row(scope, &level_row(&gain, mins))
        }
        "paintLevels" => {
            let gains = js_gain_array(scope, args.get(1))?;
            let mins = js_f64(scope, args.get(2), 0.0);
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

/// The `scriptFrame` / `jiveFrame` configuration: the frame's dock and accent
/// (what `Paint.begin` needs) plus the strip, rail and byline inputs. Returned
/// before the frame begins, because the strip spends a dock row and `begin`
/// resets the budget.
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
        let key = key_opt.unwrap_or_else(|| script.clone());
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
    for (name, value) in [
        ("dock", dock),
        ("accent", accent),
        ("byline", byline),
        ("key", key),
        ("script", script),
    ] {
        let value = js_string(scope, &value)?;
        set(scope, obj, name, value)?;
    }
    let status = js_obj_opt_string(scope, opts, "status").unwrap_or_default();
    let status = js_string(scope, &status)?;
    set(scope, obj, "status", status)?;
    for (name, values) in [
        ("pages", js_obj_string_array(scope, opts, "pages")?),
        ("sections", js_obj_string_array(scope, opts, "sections")?),
    ] {
        let value = js_string_array_value(
            scope,
            &values.iter().map(String::as_str).collect::<Vec<_>>(),
        )?;
        set(scope, obj, name, value)?;
    }
    Ok(obj.into())
}

/// The `frame` plan for a `frame_cfg`: a branded strip, a rail only on the
/// first page, and a byline, as the paint calls the caller's frame must make.
/// The strip and rail selections are resolved here (the chrome store is Rust
/// state), so JS applies the plan without a decision of its own.
fn frame_plan<'s>(
    scope: &mut v8::HandleScope<'s>,
    cfg: v8::Local<v8::Value>,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let key = js_obj_string(scope, cfg, "key", "");
    let script = js_obj_string(scope, cfg, "script", "");
    let byline = js_obj_string(scope, cfg, "byline", JIVE_BYLINE);
    let status = js_obj_opt_string(scope, cfg, "status").unwrap_or_default();
    let pages = js_obj_string_array(scope, cfg, "pages")?;
    let sections = js_obj_string_array(scope, cfg, "sections")?;
    let page = super::paint_chrome::strip_select(&key, &pages);
    let rail_on = pages.first().is_some_and(|first| *first == page);
    let section = if rail_on {
        super::paint_chrome::rail_select(&key, &sections)
    } else {
        String::new()
    };
    let obj = v8::Object::new(scope);
    let page_value = js_string(scope, &page)?;
    set(scope, obj, "page", page_value)?;
    let section_value = js_string(scope, &section)?;
    set(scope, obj, "section", section_value)?;
    let ops = v8::Array::new(scope, 3);
    let pages_value =
        js_string_array_value(scope, &pages.iter().map(String::as_str).collect::<Vec<_>>())?;
    let status_value = js_string(scope, &status)?;
    let script_value = js_string(scope, &script)?;
    let page_value = js_string(scope, &page)?;
    let key_value = js_string(scope, &key)?;
    push_frame_op(
        scope,
        ops,
        0,
        "strip",
        vec![
            key_value,
            pages_value,
            status_value,
            script_value,
            page_value,
        ],
    )?;
    let mut next = 1u32;
    if rail_on {
        let sections_value = js_string_array_value(
            scope,
            &sections.iter().map(String::as_str).collect::<Vec<_>>(),
        )?;
        let selected = js_string(scope, &section)?;
        let key_value = js_string(scope, &key)?;
        push_frame_op(
            scope,
            ops,
            next,
            "rail",
            vec![key_value, sections_value, selected],
        )?;
        next += 1;
    }
    let byline_value = js_string(scope, &byline)?;
    push_frame_op(scope, ops, next, "footer", vec![byline_value])?;
    set(scope, obj, "ops", ops.into())?;
    Ok(obj.into())
}

/// One `{ m, a }` entry of a frame plan: the frame method to call and its
/// arguments, in the order the caller applies them.
fn push_frame_op<'s>(
    scope: &mut v8::HandleScope<'s>,
    ops: v8::Local<'s, v8::Array>,
    index: u32,
    method: &str,
    args: Vec<v8::Local<'s, v8::Value>>,
) -> Result<(), String> {
    let call = v8::Object::new(scope);
    let name = js_string(scope, method)?;
    set(scope, call, "m", name)?;
    let list = v8::Array::new(scope, args.len() as i32);
    for (i, value) in args.into_iter().enumerate() {
        list.set_index(scope, i as u32, value)
            .ok_or_else(|| "v8 frame arg set failed".to_string())?;
    }
    set(scope, call, "a", list.into())?;
    ops.set_index(scope, index, call.into())
        .ok_or_else(|| "v8 frame op set failed".to_string())?;
    Ok(())
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

/// One paint call the caller's frame must make, with the arguments to make it
/// with. Which call, and in what order, is decided here; the shim only applies
/// them.
enum PaintOp {
    Text(String),
    Bar(String, f64),
    Row(Vec<String>),
}

fn level_row(g: &SkillGain, mins: f64) -> LevelRow {
    let prog = level_progress(f64::from(g.level), f64::from(g.xp));
    let per_hour = if mins > 0.5 {
        format!("{}/hr", fmt_xp_hr(f64::from(g.gained), mins))
    } else {
        "xp/hr n/a".to_string()
    };
    let label = format!("{} {}", paint_skill_short(&g.skill), js_number(prog.level));
    if prog.level >= f64::from(MAX_LEVEL) {
        return LevelRow {
            label,
            fraction: 1.0,
            cells: [per_hour, "maxed".to_string(), String::new()],
        };
    }
    let rate = if mins > 0.5 {
        (f64::from(g.gained) / mins) * 60.0
    } else {
        0.0
    };
    let eta_cell = match eta_hours(prog.remaining, rate) {
        None => "eta n/a".to_string(),
        Some(hours) => format!("eta {}", fmt_duration(hours * 60.0)),
    };
    LevelRow {
        label,
        fraction: prog.fraction,
        cells: [
            per_hour,
            format!("{} to go", fmt_locale(prog.remaining)),
            eta_cell,
        ],
    }
}

struct Progress {
    level: f64,
    fraction: f64,
    remaining: f64,
}

/// `levelProgress` from the reference `levelProgress.ts`: the caller's level
/// comes back untouched, the table lookup clamps, and 99 is maxed. `xpAtLevel`
/// reads 0 for an index the table has no entry for, exactly as the old array
/// did.
fn level_progress(level: f64, xp: f64) -> Progress {
    if level >= f64::from(MAX_LEVEL) {
        return Progress {
            level: f64::from(MAX_LEVEL),
            fraction: 1.0,
            remaining: 0.0,
        };
    }
    let base = xp_at_level(level);
    let next = xp_at_level(level + 1.0);
    let span = next - base;
    let into = js_min(js_max(0.0, xp - base), span);
    Progress {
        level,
        fraction: if span > 0.0 { into / span } else { 1.0 },
        remaining: js_max(0.0, next - xp),
    }
}

/// `etaHours` from the reference: null only for a spent or idle rate. A
/// non-finite rate is the caller's number and comes back as one.
fn eta_hours(remaining: f64, xp_per_hour: f64) -> Option<f64> {
    if remaining <= 0.0 || xp_per_hour <= 0.0 {
        return None;
    }
    Some(remaining / xp_per_hour)
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

/// `XP_AT_LEVEL[Math.min(99, Math.max(1, Math.floor(level)))] ?? 0`.
fn xp_at_level(level: f64) -> f64 {
    let index = js_min(f64::from(MAX_LEVEL), js_max(1.0, level.floor()));
    if !index.is_finite() || index.fract() != 0.0 || !(1.0..=f64::from(MAX_LEVEL)).contains(&index)
    {
        return 0.0;
    }
    f64::from(xp_table()[index as usize])
}

/// `fmtXpHr` from the reference `paintLogic.ts`.
fn fmt_xp_hr(gained: f64, mins: f64) -> String {
    if mins <= 0.5 {
        return "\u{2014}".to_string();
    }
    format!("{}k", to_fixed1(((gained / mins) * 60.0) / 1000.0))
}

/// `fmtDuration` from the reference: `H:MM:SS` over whole seconds, never
/// negative. Non-finite input reproduces JS number formatting, so a degenerate
/// rate reads exactly as it did there.
fn fmt_duration(mins: f64) -> String {
    let t = if mins.is_nan() {
        f64::NAN
    } else {
        (mins * 60.0).floor().max(0.0)
    };
    let hours = js_number((t / 3600.0).floor());
    let minutes = pad2(((t % 3600.0) / 60.0).floor());
    let seconds = pad2(t % 60.0);
    format!("{hours}:{minutes}:{seconds}")
}

/// `String(Math.floor(x)).padStart(2, '0')`.
fn pad2(x: f64) -> String {
    let text = js_number(x);
    if text.len() >= 2 {
        text
    } else {
        format!("0{text}")
    }
}

/// JS `String(n)`: `NaN` / `Infinity` spell out, an integral value in the
/// exact integer range prints as an integer, and anything else keeps the
/// shortest round-tripping repr.
fn js_number(v: f64) -> String {
    if v.is_nan() {
        return "NaN".to_string();
    }
    if v.is_infinite() {
        return if v > 0.0 { "Infinity" } else { "-Infinity" }.to_string();
    }
    if v.fract() == 0.0 && v.abs() < 9_007_199_254_740_992.0 {
        format!("{}", v as i64)
    } else {
        format!("{v}")
    }
}

/// JS `Number.prototype.toFixed(1)`: the correctly rounded tenth, except at an
/// exact decimal tie (`odd / 4`), where JS rounds away from zero and Rust's
/// own formatting would round to even.
fn to_fixed1(x: f64) -> String {
    if x.is_nan() {
        return "NaN".to_string();
    }
    if x.is_infinite() {
        return if x > 0.0 { "Infinity" } else { "-Infinity" }.to_string();
    }
    let magnitude = x.abs();
    let quarters = magnitude * 4.0;
    let tie = quarters.is_finite()
        && quarters < 9_007_199_254_740_992.0
        && quarters.fract() == 0.0
        && (quarters as i64) % 2 != 0;
    let body = if tie {
        let tenths = (magnitude * 10.0 + 0.5) as i64;
        format!("{}.{}", tenths / 10, tenths % 10)
    } else {
        format!("{magnitude:.1}")
    };
    if x < 0.0 {
        format!("-{body}")
    } else {
        body
    }
}

/// JS `Number.prototype.toLocaleString()` in the isolate's locale: thousands
/// separators and up to three fraction digits.
fn fmt_locale(v: f64) -> String {
    if !v.is_finite() {
        return js_number(v);
    }
    let magnitude = v.abs();
    let rounded = if magnitude < 1e15 {
        (magnitude * 1000.0).round() / 1000.0
    } else {
        magnitude
    };
    let whole = rounded.trunc();
    let mut out = fmt_grouped(whole as i64);
    let fraction = format!("{:.3}", rounded - whole);
    let fraction = fraction[1..].trim_end_matches('0');
    if fraction.len() > 1 {
        out.push_str(fraction);
    }
    if v < 0.0 {
        format!("-{out}")
    } else {
        out
    }
}

fn fmt_grouped(n: i64) -> String {
    let s = n.max(0).to_string();
    let bytes = s.as_bytes();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    for (i, ch) in s.chars().enumerate() {
        if i > 0 && (bytes.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

/// JS `Math.max` / `Math.min`: `NaN` propagates instead of being dropped, so
/// the frozen formatters' NaN branches are reached.
fn js_max(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        f64::NAN
    } else if a > b {
        a
    } else {
        b
    }
}

fn js_min(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        f64::NAN
    } else if a < b {
        a
    } else {
        b
    }
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

/// A formatter argument: JS `Number(v)` — `null` reads 0, `undefined` reads
/// `NaN`, and a non-finite number is preserved so the frozen formats' NaN and
/// Infinity branches are reached instead of being rounded to the default.
fn js_f64(scope: &mut v8::HandleScope, value: v8::Local<v8::Value>, default: f64) -> f64 {
    if value.is_null() {
        return default;
    }
    if value.is_undefined() {
        return f64::NAN;
    }
    value.number_value(scope).unwrap_or(default)
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

fn js_progress<'s>(
    scope: &mut v8::HandleScope<'s>,
    progress: &Progress,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let obj = v8::Object::new(scope);
    let level = v8::Number::new(scope, progress.level).into();
    set(scope, obj, "level", level)?;
    let fraction = v8::Number::new(scope, progress.fraction).into();
    set(scope, obj, "fraction", fraction)?;
    let remaining = v8::Number::new(scope, progress.remaining).into();
    set(scope, obj, "remaining", remaining)?;
    Ok(obj.into())
}

fn js_obj_string_array(
    scope: &mut v8::HandleScope,
    value: v8::Local<v8::Value>,
    key: &str,
) -> Result<Vec<String>, String> {
    let Some(obj) = value.to_object(scope) else {
        return Ok(Vec::new());
    };
    let Some(key) = v8::String::new(scope, key) else {
        return Ok(Vec::new());
    };
    let Some(got) = obj.get(scope, key.into()) else {
        return Ok(Vec::new());
    };
    js_string_array(scope, got)
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
                let method = js_string(scope, "text")?;
                set(scope, obj, "m", method)?;
                let args = v8::Array::new(scope, 1);
                let text = js_string(scope, text)?;
                args.set_index(scope, 0, text)
                    .ok_or_else(|| "v8 text arg set failed".to_string())?;
                set(scope, obj, "a", args.into())?;
            }
            PaintOp::Bar(label, fraction) => {
                let method = js_string(scope, "bar")?;
                set(scope, obj, "m", method)?;
                let args = v8::Array::new(scope, 2);
                let label = js_string(scope, label)?;
                args.set_index(scope, 0, label)
                    .ok_or_else(|| "v8 bar label set failed".to_string())?;
                let fraction = v8::Number::new(scope, *fraction).into();
                args.set_index(scope, 1, fraction)
                    .ok_or_else(|| "v8 bar fraction set failed".to_string())?;
                set(scope, obj, "a", args.into())?;
            }
            PaintOp::Row(cells) => {
                let method = js_string(scope, "row")?;
                set(scope, obj, "m", method)?;
                let args = v8::Array::new(scope, cells.len() as i32);
                for (j, cell) in cells.iter().enumerate() {
                    let value = js_string(scope, cell)?;
                    args.set_index(scope, j as u32, value)
                        .ok_or_else(|| "v8 row cell set failed".to_string())?;
                }
                set(scope, obj, "a", args.into())?;
            }
        }
        arr.set_index(scope, i as u32, obj.into())
            .ok_or_else(|| "v8 op set failed".to_string())?;
    }
    Ok(arr.into())
}
