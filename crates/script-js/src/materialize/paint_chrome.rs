//! In-isolate paint chrome helper.
//!
//! Persistent strip/rail/tabs selection, dock `rowsLeft` budget, advertised
//! defaults and statGrid column clipping live here. JS only marshals widget
//! calls onto this typed callback. Not a FlatBuffer RPC and not a host
//! game-action wire.

use rustyscript::Runtime;
use std::cell::RefCell;
use std::collections::HashMap;

const LINE: i32 = 16;
const TITLE_H: i32 = 20;
const TAB_H: i32 = 18;
/// Source `tabs()` sets `ty = cursorY + 3` then `cursorY = ty + TAB_H + 2`.
const TAB_INSET: i32 = 3;
const TAB_TRAIL: i32 = 2;
const BUTTON_H: i32 = 16;
const BUTTON_TRAIL: i32 = 4;
const DEFAULT_GAP: i32 = 6;
const DEFAULT_STAT_COLUMNS: i32 = 2;
const CHATBOX_H: i32 = 150;
const TOPLEFT_H: i32 = 150;

thread_local! {
    static STATE: RefCell<ChromeState> = RefCell::new(ChromeState::default());
}

#[derive(Default)]
struct ChromeState {
    selections: HashMap<String, String>,
    dock_h: i32,
    consumed: i32,
}

impl ChromeState {
    fn reset_store(&mut self) {
        self.selections.clear();
        self.consumed = 0;
        self.dock_h = CHATBOX_H;
    }

    fn begin(&mut self, dock: &str) {
        self.dock_h = match dock {
            "topleft" => TOPLEFT_H,
            _ => CHATBOX_H,
        };
        self.consumed = 0;
    }

    fn rows_left(&self) -> i32 {
        let remain = self.dock_h.saturating_sub(self.consumed);
        (remain / LINE).max(0)
    }

    fn consume(&mut self, px: i32) {
        self.consumed = self.consumed.saturating_add(px.max(0));
    }

    /// Batched body rows: one crossing spends `count` recorded lines.
    fn consume_lines(&mut self, count: i64) {
        let px = count.max(0).saturating_mul(i64::from(LINE));
        self.consume(px.clamp(0, i64::from(i32::MAX)) as i32);
    }

    fn resolve(&self, key: &str, names: &[String]) -> String {
        if let Some(stored) = self.selections.get(key) {
            if names.iter().any(|n| n == stored) {
                return stored.clone();
            }
        }
        names.first().cloned().unwrap_or_default()
    }

    fn resolve_rail(&self, key: &str, names: &[String]) -> String {
        if names.is_empty() {
            return self.selections.get(key).cloned().unwrap_or_default();
        }
        self.resolve(key, names)
    }
}

pub(super) fn reset() {
    STATE.with(|s| s.borrow_mut().reset_store());
}

pub(super) fn store_select(key: &str, name: &str) {
    if key.is_empty() || name.is_empty() {
        return;
    }
    STATE.with(|s| {
        s.borrow_mut()
            .selections
            .insert(key.to_string(), name.to_string());
    });
}

/// Resolve a strip band's selection and spend its title row, for a caller that
/// records the frame itself (`paint_jive`'s frame plan) instead of marshalling
/// the call.
pub(super) fn strip_select(id: &str, names: &[String]) -> String {
    STATE.with(|s| {
        let mut st = s.borrow_mut();
        st.consume(TITLE_H);
        st.resolve(&format!("strip:{id}"), names)
    })
}

/// Resolve a rail band's selection. The rail is vertical, so it spends no
/// dock rows.
pub(super) fn rail_select(id: &str, names: &[String]) -> String {
    STATE.with(|s| s.borrow().resolve_rail(&format!("rail:{id}"), names))
}

pub(super) fn install(runtime: &mut Runtime) -> Result<(), String> {
    let context = runtime.deno_runtime().main_context();
    let mut scope = runtime.deno_runtime().handle_scope();
    let global = context.open(&mut scope).global(&mut scope);
    let name = v8::String::new(&mut scope, "__rs2b0t_paint_chrome")
        .ok_or_else(|| "paint chrome name".to_string())?;
    let func = v8::Function::new(&mut scope, paint_chrome_callback)
        .ok_or_else(|| "paint chrome function".to_string())?;
    global
        .set(&mut scope, name.into(), func.into())
        .ok_or_else(|| "paint chrome set".to_string())?;
    Ok(())
}

fn paint_chrome_callback(
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
        "begin" => {
            let dock = js_string_arg(scope, args.get(1));
            STATE.with(|s| s.borrow_mut().begin(&dock));
            Ok(v8::undefined(scope).into())
        }
        "title" => {
            STATE.with(|s| s.borrow_mut().consume(TITLE_H));
            Ok(v8::undefined(scope).into())
        }
        // Batched recorded lines: the frame counts them and flushes once, so
        // the dock budget costs one crossing per pass instead of one per row.
        "rows" => {
            let count = js_opt_i32(scope, args.get(1), 0);
            STATE.with(|s| s.borrow_mut().consume_lines(i64::from(count)));
            Ok(v8::undefined(scope).into())
        }
        "gap" => {
            let px = js_opt_i32(scope, args.get(1), DEFAULT_GAP);
            STATE.with(|s| s.borrow_mut().consume(px));
            Ok(v8::undefined(scope).into())
        }
        "buttons" | "select" => {
            STATE.with(|s| s.borrow_mut().consume(BUTTON_H + BUTTON_TRAIL));
            Ok(v8::undefined(scope).into())
        }
        "rowsLeft" => {
            let left = STATE.with(|s| s.borrow().rows_left());
            Ok(v8::Number::new(scope, f64::from(left)).into())
        }
        "strip" => {
            let id = js_string_arg(scope, args.get(1));
            let names = js_string_array(scope, args.get(2))?;
            let selected = strip_select(&id, &names);
            js_string(scope, &selected)
        }
        "rail" => {
            let id = js_string_arg(scope, args.get(1));
            let names = js_string_array(scope, args.get(2))?;
            let selected = rail_select(&id, &names);
            js_string(scope, &selected)
        }
        "tabs" => {
            let id = js_string_arg(scope, args.get(1));
            let names = js_string_array(scope, args.get(2))?;
            let selected = STATE.with(|s| {
                let mut st = s.borrow_mut();
                st.consume(TAB_INSET + TAB_H + TAB_TRAIL);
                st.resolve(&format!("tabs:{id}"), &names)
            });
            js_string(scope, &selected)
        }
        "statGrid" => {
            let columns = js_opt_i32(scope, args.get(2), DEFAULT_STAT_COLUMNS).max(0) as usize;
            let rows = js_cell_rows(scope, args.get(1))?;
            let mut lines = Vec::with_capacity(rows.len());
            STATE.with(|s| {
                let mut st = s.borrow_mut();
                for row in &rows {
                    let taken: Vec<&str> = row.iter().take(columns).map(String::as_str).collect();
                    lines.push(taken.join(" | "));
                    st.consume(LINE);
                }
            });
            js_string_array_value(scope, &lines)
        }
        other => Err(format!("unknown paint chrome op {other}")),
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

fn js_cell_rows(
    scope: &mut v8::HandleScope,
    value: v8::Local<v8::Value>,
) -> Result<Vec<Vec<String>>, String> {
    if value.is_null_or_undefined() {
        return Ok(Vec::new());
    }
    let arr = value
        .to_object(scope)
        .and_then(|o| v8::Local::<v8::Array>::try_from(o).ok())
        .ok_or_else(|| "expected statGrid rows".to_string())?;
    let len = arr.length();
    let mut out = Vec::with_capacity(len as usize);
    for i in 0..len {
        let item = arr
            .get_index(scope, i)
            .ok_or_else(|| "statGrid row get failed".to_string())?;
        out.push(js_cell_row(scope, item)?);
    }
    Ok(out)
}

fn js_cell_row(
    scope: &mut v8::HandleScope,
    value: v8::Local<v8::Value>,
) -> Result<Vec<String>, String> {
    if value.is_null_or_undefined() {
        return Ok(Vec::new());
    }
    let arr = value
        .to_object(scope)
        .and_then(|o| v8::Local::<v8::Array>::try_from(o).ok())
        .ok_or_else(|| "expected statGrid row".to_string())?;
    let len = arr.length();
    let mut out = Vec::with_capacity(len as usize);
    for i in 0..len {
        let item = arr
            .get_index(scope, i)
            .ok_or_else(|| "statGrid cell get failed".to_string())?;
        out.push(js_cell_text(scope, item));
    }
    Ok(out)
}

fn js_cell_text(scope: &mut v8::HandleScope, value: v8::Local<v8::Value>) -> String {
    if value.is_null_or_undefined() {
        return String::new();
    }
    if value.is_string() {
        return js_string_arg(scope, value);
    }
    if let Some(obj) = value.to_object(scope) {
        if let Some(key) = v8::String::new(scope, "text") {
            if let Some(text) = obj.get(scope, key.into()) {
                if !text.is_null_or_undefined() {
                    return js_string_arg(scope, text);
                }
            }
        }
    }
    js_string_arg(scope, value)
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
    lines: &[String],
) -> Result<v8::Local<'s, v8::Value>, String> {
    let arr = v8::Array::new(scope, lines.len() as i32);
    for (i, line) in lines.iter().enumerate() {
        let value = js_string(scope, line)?;
        arr.set_index(scope, i as u32, value)
            .ok_or_else(|| "v8 array set failed".to_string())?;
    }
    Ok(arr.into())
}
