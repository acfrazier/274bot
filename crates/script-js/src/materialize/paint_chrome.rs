//! In-isolate paint chrome helper.
//!
//! Persistent strip/rail/tabs selection, dock `rowsLeft` budget, advertised
//! defaults and statGrid column clipping live here. JS only marshals widget
//! calls onto this typed callback. Not a FlatBuffer RPC and not a host
//! game-action wire.

use rustyscript::Runtime;
use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

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
const PAD: f64 = 8.0;
const RAIL_W: f64 = 72.0;
const DEFAULT_CHAR_W: f64 = 7.0;

thread_local! {
    static STATE: RefCell<ChromeState> = RefCell::new(ChromeState::default());
}
struct PaintChromeShapes {
    stepper_button: v8::Global<v8::ObjectTemplate>,
    lowercase_method_key: v8::Global<v8::String>,
    state_store_weak_map_ctor: v8::Global<v8::Function>,
    state_store_get: v8::Global<v8::Function>,
    state_store_set: v8::Global<v8::Function>,
    state_stores: RefCell<v8::Global<v8::Object>>,
}

#[derive(Default)]
struct ChromeState {
    selections: HashMap<String, String>,
    // Singleton list offsets persist until the script resets its paint store.
    scroll_state: HashMap<String, String>,
    reset_instance_stores: bool,
    dock_h: i32,
    consumed: i32,
    panel: PanelRect,
    body_x: f64,
    char_w: f64,
    collapsed: bool,
}

#[derive(Clone, Copy)]
struct PanelRect {
    y: f64,
    w: f64,
    h: f64,
}

impl Default for PanelRect {
    fn default() -> Self {
        Self {
            y: 345.0,
            w: 506.0,
            h: CHATBOX_H as f64,
        }
    }
}

impl ChromeState {
    fn reset_store(&mut self) {
        self.selections.clear();
        self.scroll_state.clear();
        self.reset_instance_stores = true;
        self.consumed = 0;
        self.dock_h = CHATBOX_H;
        self.panel = PanelRect::default();
        self.body_x = PAD;
        self.char_w = DEFAULT_CHAR_W;
        self.collapsed = false;
    }

    fn begin(&mut self, panel: PanelRect, char_w: f64) {
        self.panel = panel;
        self.dock_h = panel.h.trunc().clamp(0.0, f64::from(i32::MAX)) as i32;
        self.consumed = 0;
        self.body_x = PAD;
        self.char_w = char_w;
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

    fn cols(&self) -> i32 {
        if self.char_w <= 0.0 {
            return 0;
        }
        ((self.panel.w - self.body_x + PAD - PAD * 2.0) / self.char_w)
            .floor()
            .max(0.0) as i32
    }

    fn resolve(&self, key: &str, names: &[String]) -> String {
        if let Some(stored) = self.selections.get(key) {
            if names.iter().any(|name| name == stored) {
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
            .insert(key.to_owned(), name.to_owned());
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

fn object_template(
    scope: &mut v8::HandleScope,
    fields: &[&str],
) -> Result<v8::Global<v8::ObjectTemplate>, String> {
    let template = v8::ObjectTemplate::new(scope);
    let placeholder: v8::Local<v8::Data> = v8::undefined(scope).into();
    for field in fields {
        let key =
            v8::String::new_from_utf8(scope, field.as_bytes(), v8::NewStringType::Internalized)
                .ok_or_else(|| format!("paint chrome object shape key `{field}`"))?;
        template.set(key.into(), placeholder);
    }
    Ok(v8::Global::new(scope, template))
}
pub(super) fn install(runtime: &mut Runtime) -> Result<(), String> {
    let shapes = {
        let context = runtime.deno_runtime().main_context();
        let mut scope = runtime.deno_runtime().handle_scope();
        let global = context.open(&mut scope).global(&mut scope);
        let lowercase_method_key =
            v8::String::new_from_utf8(&mut scope, b"toLowerCase", v8::NewStringType::Internalized)
                .ok_or_else(|| "paint stepper lowercase key".to_string())?;
        let weak_map_ctor =
            v8::Local::<v8::Function>::try_from(js_property(&mut scope, global.into(), "WeakMap")?)
                .map_err(|_| "paint state weak map constructor".to_string())?;
        let weak_map_prototype = js_property(&mut scope, weak_map_ctor.into(), "prototype")?
            .to_object(&mut scope)
            .ok_or_else(|| "paint state weak map prototype".to_string())?;
        let state_store_get = v8::Local::<v8::Function>::try_from(js_property(
            &mut scope,
            weak_map_prototype.into(),
            "get",
        )?)
        .map_err(|_| "paint state weak map get method".to_string())?;
        let state_store_set = v8::Local::<v8::Function>::try_from(js_property(
            &mut scope,
            weak_map_prototype.into(),
            "set",
        )?)
        .map_err(|_| "paint state weak map set method".to_string())?;
        let state_stores = new_state_store_weak_map(&mut scope, weak_map_ctor)?;
        Rc::new(PaintChromeShapes {
            stepper_button: object_template(&mut scope, &["id", "label"])?,
            lowercase_method_key: v8::Global::new(&mut scope, lowercase_method_key),
            state_store_weak_map_ctor: v8::Global::new(&mut scope, weak_map_ctor),
            state_store_get: v8::Global::new(&mut scope, state_store_get),
            state_store_set: v8::Global::new(&mut scope, state_store_set),
            state_stores: RefCell::new(v8::Global::new(&mut scope, state_stores)),
        })
    };
    if !runtime.deno_runtime().v8_isolate().set_slot(shapes) {
        return Err("paint chrome object shape cache already installed".to_string());
    }
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
fn new_state_store_weak_map<'s>(
    scope: &mut v8::HandleScope<'s>,
    constructor: v8::Local<'s, v8::Function>,
) -> Result<v8::Local<'s, v8::Object>, String> {
    constructor
        .new_instance(scope, &[])
        .ok_or_else(|| "paint state weak map allocation failed".to_string())
}

fn apply_pending_state_store_reset(scope: &mut v8::HandleScope) -> Result<(), String> {
    let pending = STATE.with(|s| std::mem::take(&mut s.borrow_mut().reset_instance_stores));
    if !pending {
        return Ok(());
    }
    let shapes = scope
        .get_slot::<Rc<PaintChromeShapes>>()
        .cloned()
        .ok_or_else(|| "paint chrome object shape cache is not installed".to_string())?;
    let constructor = v8::Local::new(scope, &shapes.state_store_weak_map_ctor);
    let state_stores = new_state_store_weak_map(scope, constructor)?;
    *shapes.state_stores.borrow_mut() = v8::Global::new(scope, state_stores);
    Ok(())
}

fn paint_state_store<'s>(
    scope: &mut v8::HandleScope<'s>,
    owner: v8::Local<'s, v8::Value>,
    create: bool,
) -> Result<Option<v8::Local<'s, v8::Map>>, String> {
    let owner = v8::Local::<v8::Object>::try_from(owner)
        .map_err(|_| "paint state owner is not an object".to_string())?;
    let shapes = scope
        .get_slot::<Rc<PaintChromeShapes>>()
        .cloned()
        .ok_or_else(|| "paint chrome object shape cache is not installed".to_string())?;
    let weak_map = {
        let state_stores = shapes.state_stores.borrow();
        v8::Local::new(scope, &*state_stores)
    };
    let get = v8::Local::new(scope, &shapes.state_store_get);
    let store = get
        .call(scope, weak_map.into(), &[owner.into()])
        .ok_or_else(|| "paint state weak map get failed".to_string())?;
    if store.is_undefined() {
        if !create {
            return Ok(None);
        }
        let store = v8::Map::new(scope);
        let set = v8::Local::new(scope, &shapes.state_store_set);
        set.call(scope, weak_map.into(), &[owner.into(), store.into()])
            .ok_or_else(|| "paint state weak map set failed".to_string())?;
        return Ok(Some(store));
    }
    v8::Local::<v8::Map>::try_from(store)
        .map(Some)
        .map_err(|_| "paint state owner has an invalid store".to_string())
}

fn paint_chrome_callback<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: v8::FunctionCallbackArguments<'s>,
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
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<v8::Local<'s, v8::Value>, String> {
    apply_pending_state_store_reset(scope)?;
    let op = js_string_arg(scope, args.get(0));
    match op.as_str() {
        "begin" => {
            let panel = panel_rect(scope, args.get(1))?;
            let ctx = args.get(2);
            if !ctx.is_null_or_undefined() {
                let context = ctx
                    .to_object(scope)
                    .ok_or_else(|| "paint context is not an object".to_string())?;
                let font = js_string(scope, "12px monospace")?;
                set_property(scope, context, "font", font)?;
                let baseline = js_string(scope, "middle")?;
                set_property(scope, context, "textBaseline", baseline)?;
            }
            let char_w = measure_char_width(scope, ctx);
            STATE.with(|s| s.borrow_mut().begin(panel, char_w));
            Ok(v8::undefined(scope).into())
        }
        "title" => {
            let toggle = consume_paint_click(scope, args.get(1), "paint:toggle")?;
            STATE.with(|s| {
                let mut state = s.borrow_mut();
                state.consume(TITLE_H);
                if toggle {
                    state.collapsed = !state.collapsed;
                }
            });
            Ok(v8::undefined(scope).into())
        }
        // Batched body rows: one crossing spends the pending line count.
        "rows" => {
            let count = js_opt_i32(scope, args.get(1), 0);
            STATE.with(|s| {
                let mut state = s.borrow_mut();
                if !state.collapsed {
                    state.consume_lines(i64::from(count));
                }
            });
            Ok(v8::undefined(scope).into())
        }
        "gap" => {
            let px = js_opt_i32(scope, args.get(1), DEFAULT_GAP);
            STATE.with(|s| {
                let mut state = s.borrow_mut();
                if !state.collapsed {
                    state.consume(px);
                }
            });
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
            if !names.is_empty() {
                STATE.with(|s| {
                    let mut state = s.borrow_mut();
                    if !state.collapsed {
                        state.body_x = RAIL_W + PAD;
                    }
                });
            }
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
        "clipText" => clip_text_value(scope, args.get(1), args.get(2), false),
        "paintClip" => clip_text_value(scope, args.get(1), args.get(2), true),
        "gatherPaintAccent" => {
            let kind = args.get(1);
            let accent = if kind.is_string() {
                match js_string_arg(scope, kind).as_str() {
                    "fish" => "#7ec8e3",
                    "mine" => "#e0c36a",
                    _ => "#9be05b",
                }
            } else {
                "#9be05b"
            };
            js_string(scope, accent)
        }
        "stateNew" => {
            let owner = v8::Local::<v8::Object>::try_from(args.get(1))
                .map_err(|_| "paint state owner is not an object".to_string())?;
            let shapes = scope
                .get_slot::<Rc<PaintChromeShapes>>()
                .cloned()
                .ok_or_else(|| "paint chrome object shape cache is not installed".to_string())?;
            let weak_map = v8::Local::new(scope, &*shapes.state_stores.borrow());
            let set = v8::Local::new(scope, &shapes.state_store_set);
            let store = v8::Map::new(scope);
            set.call(scope, weak_map.into(), &[owner.into(), store.into()])
                .ok_or_else(|| "paint state weak map set failed".to_string())?;
            Ok(v8::undefined(scope).into())
        }
        "stateGet" => {
            let Some(store) = paint_state_store(scope, args.get(1), false)? else {
                return Ok(args.get(3));
            };
            let key_text = js_string_arg(scope, args.get(2));
            let key = js_string(scope, &key_text)?;
            let value = store
                .get(scope, key.into())
                .ok_or_else(|| "paint state map get failed".to_string())?;
            if value.is_undefined() {
                Ok(args.get(3))
            } else {
                Ok(value)
            }
        }
        "stateSet" => {
            let store = paint_state_store(scope, args.get(1), true)?
                .ok_or_else(|| "paint state store was not created".to_string())?;
            let key_text = js_string_arg(scope, args.get(2));
            let value_text = js_string_arg(scope, args.get(3));
            let key = js_string(scope, &key_text)?;
            let value = js_string(scope, &value_text)?;
            store
                .set(scope, key.into(), value.into())
                .ok_or_else(|| "paint state map set failed".to_string())?;
            Ok(v8::undefined(scope).into())
        }
        "cols" => {
            let pending = js_opt_i32(scope, args.get(1), 0);
            let cols = STATE.with(|s| {
                let mut state = s.borrow_mut();
                if !state.collapsed {
                    state.consume_lines(i64::from(pending));
                }
                state.cols()
            });
            Ok(v8::Number::new(scope, f64::from(cols)).into())
        }
        "wrap" => wrap(scope, args.get(1), args.get(2), args.get(4), args.get(5)),
        "list" => list(
            scope,
            args.get(1),
            args.get(2),
            args.get(3),
            args.get(4),
            args.get(5),
            args.get(6),
        ),
        "fill" => fill(
            scope,
            args.get(1),
            args.get(2),
            args.get(3),
            args.get(4),
            args.get(5),
        ),
        "grid" => grid(
            scope,
            args.get(1),
            args.get(2),
            args.get(3),
            args.get(4),
            args.get(5),
            args.get(6),
        ),
        "stepper" => stepper(
            scope,
            args.get(1),
            args.get(2),
            args.get(3),
            args.get(4),
            args.get(5),
            args.get(6),
            args.get(7),
        ),
        other => Err(format!("unknown paint chrome op {other}")),
    }
}

fn js_property<'s>(
    scope: &mut v8::HandleScope<'s>,
    value: v8::Local<'s, v8::Value>,
    name: &str,
) -> Result<v8::Local<'s, v8::Value>, String> {
    if value.is_null_or_undefined() {
        return Ok(v8::undefined(scope).into());
    }
    let object = value
        .to_object(scope)
        .ok_or_else(|| format!("paint property receiver `{name}`"))?;
    let key = v8::String::new(scope, name)
        .ok_or_else(|| "paint property key allocation failed".to_string())?;
    object
        .get(scope, key.into())
        .ok_or_else(|| format!("paint property read failed for `{name}`"))
}

fn set_property<'s>(
    scope: &mut v8::HandleScope<'s>,
    object: v8::Local<'s, v8::Object>,
    name: &str,
    value: v8::Local<'s, v8::Value>,
) -> Result<(), String> {
    let key = v8::String::new(scope, name)
        .ok_or_else(|| "paint property key allocation failed".to_string())?;
    object
        .set(scope, key.into(), value)
        .ok_or_else(|| format!("paint property write failed for `{name}`"))?;
    Ok(())
}
fn paint_click_matches(
    scope: &mut v8::HandleScope,
    click: v8::Local<v8::Value>,
    expected: &str,
) -> bool {
    click.is_string() && js_string_arg(scope, click) == expected
}

fn clear_paint_click<'s>(
    scope: &mut v8::HandleScope<'s>,
    host: v8::Local<'s, v8::Object>,
) -> Result<(), String> {
    let null = v8::null(scope).into();
    set_property(scope, host, "paintClick", null)
}

fn consume_paint_click<'s>(
    scope: &mut v8::HandleScope<'s>,
    host: v8::Local<'s, v8::Value>,
    expected: &str,
) -> Result<bool, String> {
    let host = v8::Local::<v8::Object>::try_from(host)
        .map_err(|_| "paint click host is not an object".to_string())?;
    let click = js_property(scope, host.into(), "paintClick")?;
    if !paint_click_matches(scope, click, expected) {
        return Ok(false);
    }
    clear_paint_click(scope, host)?;
    Ok(true)
}

fn panel_rect<'s>(
    scope: &mut v8::HandleScope<'s>,
    dock: v8::Local<'s, v8::Value>,
) -> Result<PanelRect, String> {
    if dock.is_null_or_undefined() || dock.is_string() {
        if dock.is_string() && js_string_arg(scope, dock) == "topleft" {
            return Ok(PanelRect {
                y: 6.0,
                w: 320.0,
                h: TOPLEFT_H as f64,
            });
        }
        return Ok(PanelRect::default());
    }
    let y = js_property(scope, dock, "y")?;
    let w = js_property(scope, dock, "w")?;
    let h = js_property(scope, dock, "h")?;
    Ok(PanelRect {
        y: js_number(scope, y),
        w: js_number(scope, w),
        h: js_number(scope, h),
    })
}

fn measure_char_width<'s>(scope: &mut v8::HandleScope<'s>, ctx: v8::Local<'s, v8::Value>) -> f64 {
    if ctx.is_null_or_undefined() {
        return DEFAULT_CHAR_W;
    }
    let Ok(measure) = js_property(scope, ctx, "measureText") else {
        return DEFAULT_CHAR_W;
    };
    let Ok(measure) = v8::Local::<v8::Function>::try_from(measure) else {
        return DEFAULT_CHAR_W;
    };
    let Some(zero) = v8::String::new(scope, "0") else {
        return DEFAULT_CHAR_W;
    };
    let Some(result) = measure.call(scope, ctx, &[zero.into()]) else {
        return DEFAULT_CHAR_W;
    };
    let Ok(width) = js_property(scope, result, "width") else {
        return DEFAULT_CHAR_W;
    };
    let width = js_number(scope, width);
    if width == 0.0 || width.is_nan() {
        DEFAULT_CHAR_W
    } else {
        width
    }
}

fn js_number(scope: &mut v8::HandleScope, value: v8::Local<v8::Value>) -> f64 {
    value.number_value(scope).unwrap_or(f64::NAN)
}

fn js_string_units(
    scope: &mut v8::HandleScope,
    value: v8::Local<v8::Value>,
) -> Result<Vec<u16>, String> {
    let string = value
        .to_string(scope)
        .ok_or_else(|| "paint text conversion failed".to_string())?;
    let mut units = vec![0; string.length()];
    string.write_v2(scope, 0, &mut units, v8::WriteFlags::empty());
    Ok(units)
}

fn string_from_units<'s>(
    scope: &mut v8::HandleScope<'s>,
    units: &[u16],
) -> Result<v8::Local<'s, v8::Value>, String> {
    v8::String::new_from_two_byte(scope, units, v8::NewStringType::Normal)
        .map(Into::into)
        .ok_or_else(|| "paint text allocation failed".to_string())
}

fn clipped_units(text: &[u16], cols: f64) -> Cow<'_, [u16]> {
    if cols <= 0.0 {
        return Cow::Borrowed(&[]);
    }
    if text.len() as f64 <= cols {
        return Cow::Borrowed(text);
    }
    let end = (cols - 1.0).trunc().max(0.0) as usize;
    let mut clipped = Vec::with_capacity(end + 1);
    clipped.extend_from_slice(&text[..text.len().min(end)]);
    clipped.push(0x2026);
    Cow::Owned(clipped)
}

fn js_whitespace(unit: u16) -> bool {
    matches!(
        unit,
        0x0009..=0x000d
            | 0x0020
            | 0x00a0
            | 0x1680
            | 0x2000..=0x200a
            | 0x2028
            | 0x2029
            | 0x202f
            | 0x205f
            | 0x3000
            | 0xfeff
    )
}

fn trim_units(units: &[u16]) -> &[u16] {
    let start = units
        .iter()
        .position(|unit| !js_whitespace(*unit))
        .unwrap_or(units.len());
    let end = units
        .iter()
        .rposition(|unit| !js_whitespace(*unit))
        .map_or(start, |end| end + 1);
    &units[start..end]
}

fn wrap_units(text: &[u16], cols: usize, indent: usize) -> Vec<Vec<u16>> {
    if cols == 0 {
        return Vec::new();
    }
    let pad_len = indent.min(cols.saturating_sub(1));
    let pad = vec![0x20; pad_len];
    let words = text
        .split(|unit| js_whitespace(*unit))
        .filter(|word| !word.is_empty());
    let mut lines = Vec::new();
    let mut line = Vec::new();
    for word in words {
        let mut start = 0;
        while word.len() - start
            > if lines.is_empty() {
                cols
            } else {
                cols - pad_len
            }
        {
            push_wrapped_line(&mut lines, &mut line, &pad);
            let room = if lines.is_empty() {
                cols
            } else {
                cols - pad_len
            };
            let end = start + room.min(word.len() - start);
            push_wrapped_segment(&mut lines, &word[start..end], &pad);
            start = end;
        }
        let word = &word[start..];
        if word.is_empty() {
            continue;
        }
        let room = if lines.is_empty() {
            cols
        } else {
            cols - pad_len
        };
        if line.is_empty() {
            line.extend_from_slice(word);
        } else if line.len() + 1 + word.len() <= room {
            line.push(0x20);
            line.extend_from_slice(word);
        } else {
            push_wrapped_line(&mut lines, &mut line, &pad);
            line.extend_from_slice(word);
        }
    }
    push_wrapped_line(&mut lines, &mut line, &pad);
    lines
}

fn push_wrapped_line(lines: &mut Vec<Vec<u16>>, line: &mut Vec<u16>, pad: &[u16]) {
    if line.is_empty() {
        return;
    }
    if pad.is_empty() {
        lines.push(std::mem::take(line));
        return;
    }
    let pad_len = if lines.is_empty() { 0 } else { pad.len() };
    let mut output = Vec::with_capacity(pad_len + line.len());
    output.extend_from_slice(&pad[..pad_len]);
    output.append(line);
    lines.push(output);
}

fn push_wrapped_segment(lines: &mut Vec<Vec<u16>>, text: &[u16], pad: &[u16]) {
    let pad_len = if lines.is_empty() { 0 } else { pad.len() };
    let mut output = Vec::with_capacity(pad_len + text.len());
    output.extend_from_slice(&pad[..pad_len]);
    output.extend_from_slice(text);
    lines.push(output);
}

fn clip_text_value<'s>(
    scope: &mut v8::HandleScope<'s>,
    text: v8::Local<'s, v8::Value>,
    cols: v8::Local<'s, v8::Value>,
    trim: bool,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let units = js_string_units(scope, text)?;
    let units = if trim { trim_units(&units) } else { &units };
    let cols = js_number(scope, cols);
    let clipped = clipped_units(units, cols);
    string_from_units(scope, clipped.as_ref())
}

fn as_array<'s>(
    scope: &mut v8::HandleScope<'s>,
    value: v8::Local<'s, v8::Value>,
    name: &str,
) -> Result<v8::Local<'s, v8::Array>, String> {
    value
        .to_object(scope)
        .and_then(|object| v8::Local::<v8::Array>::try_from(object).ok())
        .ok_or_else(|| format!("expected paint {name} array"))
}

fn record_lines<'s>(
    scope: &mut v8::HandleScope<'s>,
    record: v8::Local<'s, v8::Value>,
) -> Result<v8::Local<'s, v8::Array>, String> {
    let value = js_property(scope, record, "lines")?;
    as_array(scope, value, "record lines")
}

fn push_units(
    scope: &mut v8::HandleScope,
    lines: v8::Local<v8::Array>,
    units: &[u16],
) -> Result<(), String> {
    let value = string_from_units(scope, units)?;
    lines
        .set_index(scope, lines.length(), value)
        .ok_or_else(|| "paint line append failed".to_string())?;
    Ok(())
}

fn push_line(
    scope: &mut v8::HandleScope,
    lines: v8::Local<v8::Array>,
    text: &str,
) -> Result<(), String> {
    let value = js_string(scope, text)?;
    lines
        .set_index(scope, lines.length(), value)
        .ok_or_else(|| "paint line append failed".to_string())?;
    Ok(())
}

fn js_truthy(scope: &mut v8::HandleScope, value: v8::Local<v8::Value>) -> bool {
    value.boolean_value(scope)
}

fn js_number_string(n: f64) -> String {
    if n == 0.0 {
        "0".to_owned()
    } else if n.is_nan() {
        "NaN".to_owned()
    } else if n == f64::INFINITY {
        "Infinity".to_owned()
    } else if n == f64::NEG_INFINITY {
        "-Infinity".to_owned()
    } else {
        n.to_string()
    }
}

fn parse_number(text: Option<&str>, fallback: f64) -> f64 {
    text.map(str::trim)
        .filter(|text| !text.is_empty())
        .map_or(fallback, |text| text.parse().unwrap_or(f64::NAN))
}

fn list_scroll(state: &mut ChromeState, key: &str, len: usize, rows: f64, focus: f64) -> f64 {
    let manual_key = format!("{key}:manual");
    let focus_key = format!("{key}:focus");
    let stored_offset = parse_number(state.scroll_state.get(key).map(String::as_str), 0.0);
    let manual = state
        .scroll_state
        .get(&manual_key)
        .is_some_and(|value| value == "1");
    let old_focus = parse_number(state.scroll_state.get(&focus_key).map(String::as_str), -1.0);
    let manual = if focus != old_focus { false } else { manual };
    let max = (len as f64 - rows).max(0.0);
    let clamp = |number: f64| number.max(0.0).min(max);
    let mut offset = clamp(if stored_offset.is_finite() {
        stored_offset.trunc()
    } else {
        0.0
    });
    if !manual && rows > 0.0 && focus >= 0.0 && focus < len as f64 {
        if focus < offset {
            offset = clamp(focus);
        } else if focus >= offset + rows {
            offset = clamp(focus - rows + 1.0);
        }
    }
    state
        .scroll_state
        .insert(key.to_owned(), js_number_string(offset));
    state
        .scroll_state
        .insert(manual_key, if manual { "1" } else { "0" }.to_owned());
    state
        .scroll_state
        .insert(focus_key, js_number_string(focus));
    offset
}

fn fill_rows<'s>(
    scope: &mut v8::HandleScope<'s>,
    state: &ChromeState,
    total: f64,
    options: v8::Local<'s, v8::Value>,
) -> Result<f64, String> {
    let reserve = js_property(scope, options, "reserve")?;
    let reserve = if reserve.is_null_or_undefined() {
        0.0
    } else {
        js_number(scope, reserve)
    };
    let footer = js_property(scope, options, "footer")?;
    let bottom = state.panel.y + state.panel.h - reserve;
    let cursor_y = state.panel.y + f64::from(state.consumed);
    let available = ((bottom - cursor_y) / f64::from(LINE)).floor();
    Ok((if total > available || js_truthy(scope, footer) {
        available - 1.0
    } else {
        available
    })
    .max(1.0))
}

fn append_paint_line<'s>(
    scope: &mut v8::HandleScope<'s>,
    item: v8::Local<'s, v8::Value>,
    cols: f64,
    units: &mut Vec<u16>,
) -> Result<(), String> {
    let text = if item.is_string() {
        item
    } else {
        js_property(scope, item, "text")?
    };
    let text = text
        .to_string(scope)
        .ok_or_else(|| "paint text conversion failed".to_string())?;
    if cols <= 0.0 {
        return Ok(());
    }
    let length = text.length();
    let (copied, ellipsis) = if length as f64 <= cols {
        (length, false)
    } else {
        ((cols.floor() as usize).saturating_sub(1), true)
    };
    if copied > 0 {
        let start = units.len();
        units.resize(start + copied, 0);
        text.write_v2(scope, 0, &mut units[start..], v8::WriteFlags::empty());
    }
    if ellipsis {
        units.push('…' as u16);
    }
    Ok(())
}

fn slice_index(index: f64, len: usize) -> usize {
    if index.is_nan() {
        return 0;
    }
    if !index.is_finite() {
        return if index.is_sign_positive() { len } else { 0 };
    }
    let index = index.trunc();
    if index < 0.0 {
        (len as f64 + index).max(0.0).min(len as f64) as usize
    } else {
        index.min(len as f64) as usize
    }
}

fn append_counter(
    scope: &mut v8::HandleScope,
    rec_lines: v8::Local<v8::Array>,
    state: &mut ChromeState,
    from: f64,
    to: f64,
    footer: v8::Local<v8::Value>,
    total: f64,
) -> Result<(), String> {
    let counter = if from > 0.0 {
        format!(
            "{} to {} of {}",
            js_number_string(from),
            js_number_string(to),
            js_number_string(total)
        )
    } else {
        String::new()
    };
    let footer = if js_truthy(scope, footer) {
        js_string_arg(scope, footer)
    } else {
        String::new()
    };
    let line = match (counter.is_empty(), footer.is_empty()) {
        (true, true) => return Ok(()),
        (true, false) => footer,
        (false, true) => counter,
        (false, false) => format!("{counter} · {footer}"),
    };
    push_line(scope, rec_lines, &line)?;
    state.consume(LINE);
    Ok(())
}

fn pending_rows(scope: &mut v8::HandleScope, pending: v8::Local<v8::Value>) {
    let count = js_opt_i32(scope, pending, 0);
    STATE.with(|s| {
        let mut state = s.borrow_mut();
        if !state.collapsed {
            state.consume_lines(i64::from(count));
        }
    });
}

fn wrap<'s>(
    scope: &mut v8::HandleScope<'s>,
    record: v8::Local<'s, v8::Value>,
    text: v8::Local<'s, v8::Value>,
    indent: v8::Local<'s, v8::Value>,
    pending: v8::Local<'s, v8::Value>,
) -> Result<v8::Local<'s, v8::Value>, String> {
    pending_rows(scope, pending);
    let units = js_string_units(scope, text)?;
    let indent = js_opt_i32(scope, indent, 0).max(0) as usize;
    let (collapsed, cols) = STATE.with(|s| {
        let state = s.borrow();
        (state.collapsed, state.cols().max(0) as usize)
    });
    if collapsed {
        return Ok(v8::undefined(scope).into());
    }
    let wrapped = wrap_units(&units, cols, indent);
    let lines = record_lines(scope, record)?;
    for line in &wrapped {
        push_units(scope, lines, line)?;
    }
    STATE.with(|s| s.borrow_mut().consume_lines(wrapped.len() as i64));
    Ok(v8::undefined(scope).into())
}

fn list<'s>(
    scope: &mut v8::HandleScope<'s>,
    record: v8::Local<'s, v8::Value>,
    id: v8::Local<'s, v8::Value>,
    values: v8::Local<'s, v8::Value>,
    rows: v8::Local<'s, v8::Value>,
    options: v8::Local<'s, v8::Value>,
    pending: v8::Local<'s, v8::Value>,
) -> Result<v8::Local<'s, v8::Value>, String> {
    pending_rows(scope, pending);
    if STATE.with(|s| s.borrow().collapsed) {
        return Ok(v8::Number::new(scope, 0.0).into());
    }
    let id = js_string_arg(scope, id);
    let rows = js_number(scope, rows);
    let lines = as_array(scope, values, "lines")?;
    let focus = js_property(scope, options, "focus")?;
    let focus = if focus.is_null_or_undefined() {
        -1.0
    } else {
        js_number(scope, focus)
    };
    let footer = js_property(scope, options, "footer")?;
    list_impl(scope, record, &id, lines, rows, focus, footer)
}

fn list_impl<'s>(
    scope: &mut v8::HandleScope<'s>,
    record: v8::Local<'s, v8::Value>,
    id: &str,
    lines: v8::Local<'s, v8::Array>,
    rows: f64,
    focus: f64,
    footer: v8::Local<'s, v8::Value>,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let len = lines.length() as usize;
    let (collapsed, offset, cols) = STATE.with(|s| {
        let mut state = s.borrow_mut();
        if state.collapsed {
            return (true, 0.0, 0);
        }
        let key = format!("list:{id}");
        let offset = list_scroll(&mut state, &key, len, rows, focus);
        (false, offset, state.cols())
    });
    if collapsed {
        return Ok(v8::Number::new(scope, 0.0).into());
    }
    let record_lines = record_lines(scope, record)?;
    let max_offset = (len as f64 - rows).max(0.0);
    let room = cols - if max_offset > 0.0 { 1 } else { 0 };
    let start = slice_index(offset, len);
    let end = slice_index(offset + rows, len);
    if len == 0 {
        push_line(scope, record_lines, "nothing yet")?;
        STATE.with(|s| s.borrow_mut().consume(LINE));
    } else {
        let mut visible = Vec::with_capacity(end.saturating_sub(start));
        for index in start..end {
            visible.push(
                lines
                    .get_index(scope, index as u32)
                    .ok_or_else(|| "paint line read failed".to_string())?,
            );
        }
        let mut units = Vec::new();
        for line in visible {
            units.clear();
            append_paint_line(scope, line, f64::from(room), &mut units)?;
            push_units(scope, record_lines, &units)?;
        }
        STATE.with(|s| {
            s.borrow_mut()
                .consume_lines(end.saturating_sub(start) as i64)
        });
    }
    let from = if max_offset > 0.0 { offset + 1.0 } else { 0.0 };
    let to = if max_offset > 0.0 {
        (len as f64).min(offset + rows)
    } else {
        0.0
    };
    STATE.with(|s| {
        let mut state = s.borrow_mut();
        append_counter(
            scope,
            record_lines,
            &mut state,
            from,
            to,
            footer,
            len as f64,
        )
    })?;
    Ok(v8::Number::new(scope, offset).into())
}

fn fill<'s>(
    scope: &mut v8::HandleScope<'s>,
    record: v8::Local<'s, v8::Value>,
    id: v8::Local<'s, v8::Value>,
    values: v8::Local<'s, v8::Value>,
    options: v8::Local<'s, v8::Value>,
    pending: v8::Local<'s, v8::Value>,
) -> Result<v8::Local<'s, v8::Value>, String> {
    pending_rows(scope, pending);
    let id = js_string_arg(scope, id);
    if STATE.with(|s| s.borrow().collapsed) {
        return Ok(v8::Number::new(scope, 0.0).into());
    }
    let lines = as_array(scope, values, "lines")?;
    let len = lines.length() as usize;
    let rows = STATE.with(|s| {
        let state = s.borrow();
        fill_rows(scope, &state, len as f64, options)
    })?;
    let focus = js_property(scope, options, "focus")?;
    let focus = if focus.is_null_or_undefined() {
        -1.0
    } else {
        js_number(scope, focus)
    };
    let footer = js_property(scope, options, "footer")?;
    list_impl(scope, record, &id, lines, rows, focus, footer)
}

fn grid<'s>(
    scope: &mut v8::HandleScope<'s>,
    record: v8::Local<'s, v8::Value>,
    id: v8::Local<'s, v8::Value>,
    values: v8::Local<'s, v8::Value>,
    columns: v8::Local<'s, v8::Value>,
    options: v8::Local<'s, v8::Value>,
    pending: v8::Local<'s, v8::Value>,
) -> Result<v8::Local<'s, v8::Value>, String> {
    pending_rows(scope, pending);
    let id = js_string_arg(scope, id);
    if STATE.with(|s| s.borrow().collapsed) {
        return Ok(v8::Number::new(scope, 0.0).into());
    }
    let lines = as_array(scope, values, "lines")?;
    let len = lines.length() as usize;
    let requested_columns = js_number(scope, columns).trunc();
    let columns = if requested_columns.is_finite() {
        requested_columns.max(1.0)
    } else {
        1.0
    };
    let total = (len as f64 / columns).ceil();
    let rows = STATE.with(|s| fill_rows(scope, &s.borrow(), total, options))?;
    let focus_value = js_property(scope, options, "focus")?;
    let focus = if focus_value.is_undefined() || js_number(scope, focus_value) < 0.0 {
        -1.0
    } else {
        (js_number(scope, focus_value) / columns).floor()
    };
    let footer = js_property(scope, options, "footer")?;
    let (offset, char_w, body_x, panel_w) = STATE.with(|s| {
        let mut state = s.borrow_mut();
        let key = format!("list:{id}");
        let offset = list_scroll(&mut state, &key, total.max(0.0) as usize, rows, focus);
        (offset, state.char_w, state.body_x, state.panel.w)
    });
    let record_lines = record_lines(scope, record)?;
    let max_offset = (total - rows).max(0.0);
    if len == 0 {
        push_line(scope, record_lines, "nothing yet")?;
        STATE.with(|s| s.borrow_mut().consume(LINE));
    } else {
        let cell_width = ((panel_w - PAD - body_x).max(0.0) / columns).max(0.0);
        let column_count = columns.min(usize::MAX as f64) as usize;
        let mut row_items = Vec::new();
        let mut text = Vec::new();
        let mut row = offset;
        let last_row = total.min(offset + rows);
        while row < last_row {
            let start = slice_index(row * columns, len);
            let end = slice_index(row * columns + columns, len);
            row_items.clear();
            for index in start..end {
                row_items.push(
                    lines
                        .get_index(scope, index as u32)
                        .ok_or_else(|| "paint line read failed".to_string())?,
                );
            }
            text.clear();
            for column in 0..column_count {
                if column > 0 {
                    text.extend(" | ".encode_utf16());
                }
                let slot_width = if char_w <= 0.0 {
                    f64::INFINITY
                } else {
                    (cell_width / char_w).floor()
                        - if column + 1 < column_count { 1.0 } else { 0.0 }
                };
                if let Some(line) = row_items.get(column) {
                    append_paint_line(scope, *line, slot_width.max(0.0), &mut text)?;
                } else {
                    let clipped = clipped_units(&[], slot_width.max(0.0));
                    text.extend_from_slice(clipped.as_ref());
                }
            }
            push_units(scope, record_lines, &text)?;
            STATE.with(|s| s.borrow_mut().consume(LINE));
            row += 1.0;
        }
    }
    let from = if max_offset > 0.0 {
        offset * columns + 1.0
    } else {
        0.0
    };
    let to = if max_offset > 0.0 {
        (len as f64).min((offset + rows) * columns)
    } else {
        0.0
    };
    STATE.with(|s| {
        let mut state = s.borrow_mut();
        append_counter(
            scope,
            record_lines,
            &mut state,
            from,
            to,
            footer,
            len as f64,
        )
    })?;
    Ok(v8::Number::new(scope, offset).into())
}

fn new_stepper_button<'s>(
    scope: &mut v8::HandleScope<'s>,
    template: v8::Local<'s, v8::ObjectTemplate>,
    id: &str,
    label: &str,
) -> Result<v8::Local<'s, v8::Object>, String> {
    let button = template
        .new_instance(scope)
        .ok_or_else(|| "paint stepper button allocation failed".to_string())?;
    let id = js_string(scope, id)?;
    set_property(scope, button, "id", id)?;
    let label = js_string(scope, label)?;
    set_property(scope, button, "label", label)?;
    Ok(button)
}

fn js_lowercase<'s>(
    scope: &mut v8::HandleScope<'s>,
    value: v8::Local<'s, v8::String>,
    method_key: v8::Local<'s, v8::String>,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let receiver: v8::Local<v8::Value> = value.into();
    let boxed = receiver
        .to_object(scope)
        .ok_or_else(|| "paint stepper string conversion failed".to_string())?;
    let method = boxed
        .get(scope, method_key.into())
        .ok_or_else(|| "paint stepper lowercase lookup failed".to_string())?;
    let method = v8::Local::<v8::Function>::try_from(method)
        .map_err(|_| "paint stepper lowercase method is not callable".to_string())?;
    method
        .call(scope, receiver, &[])
        .ok_or_else(|| "paint stepper lowercase call failed".to_string())
}

#[expect(
    clippy::too_many_arguments,
    reason = "Each field is one argument supplied to the native paint stepper operation."
)]
fn stepper<'s>(
    scope: &mut v8::HandleScope<'s>,
    record: v8::Local<'s, v8::Value>,
    id: v8::Local<'s, v8::Value>,
    label: v8::Local<'s, v8::Value>,
    values: v8::Local<'s, v8::Value>,
    current: v8::Local<'s, v8::Value>,
    pending: v8::Local<'s, v8::Value>,
    host: v8::Local<'s, v8::Value>,
) -> Result<v8::Local<'s, v8::Value>, String> {
    pending_rows(scope, pending);
    if STATE.with(|s| s.borrow().collapsed) {
        return Ok(v8::null(scope).into());
    }
    let Some(options) = stepper_options_array(scope, values)? else {
        return Ok(v8::null(scope).into());
    };
    let option_count = options.length();
    if option_count == 0 {
        return Ok(v8::null(scope).into());
    }
    let id = js_string_arg(scope, id);
    let label = js_string_arg(scope, label);
    let current_text = js_string_arg(scope, current);
    let current = v8::String::new(scope, &current_text)
        .ok_or_else(|| "paint stepper current string allocation failed".to_string())?;
    let shapes = scope
        .get_slot::<Rc<PaintChromeShapes>>()
        .cloned()
        .ok_or_else(|| "paint chrome object shape cache is not installed".to_string())?;
    let template = v8::Local::new(scope, &shapes.stepper_button);
    let buttons = record_buttons(scope, record)?;
    let middle = format!("{label}: {current_text}");
    let prev_id = format!("step:{id}:prev");
    let mid_id = format!("step:{id}:mid");
    let next_id = format!("step:{id}:next");
    for (button_id, text) in [
        (prev_id.as_str(), "◀"),
        (mid_id.as_str(), middle.as_str()),
        (next_id.as_str(), "▶"),
    ] {
        let button = new_stepper_button(scope, template, button_id, text)?;
        buttons
            .set_index(scope, buttons.length(), button.into())
            .ok_or_else(|| "paint stepper button append failed".to_string())?;
    }
    STATE.with(|s| s.borrow_mut().consume(BUTTON_H + BUTTON_TRAIL));
    let host_object = v8::Local::<v8::Object>::try_from(host)
        .map_err(|_| "paint click host is not an object".to_string())?;
    let click = js_property(scope, host_object.into(), "paintClick")?;
    let click = click.is_string().then(|| js_string_arg(scope, click));
    let delta = if click.as_deref() == Some(prev_id.as_str()) {
        clear_paint_click(scope, host_object)?;
        -1
    } else if click.as_deref() == Some(next_id.as_str()) {
        clear_paint_click(scope, host_object)?;
        1
    } else {
        // The upstream display-only middle click remains pending.
        0
    };
    if delta == 0 {
        return Ok(v8::null(scope).into());
    }
    let method_key = v8::Local::new(scope, &shapes.lowercase_method_key);
    let current_lower = js_lowercase(scope, current, method_key)?;
    let mut index = 0;
    for candidate_index in 0..option_count {
        let option = stepper_option(scope, options, candidate_index)?;
        let option_lower = js_lowercase(scope, option, method_key)?;
        if option_lower.strict_equals(current_lower) {
            index = candidate_index;
            break;
        }
    }
    let len = i64::from(option_count);
    let target = (i64::from(index) + delta).rem_euclid(len) as u32;
    options
        .get_index(scope, target)
        .ok_or_else(|| "paint stepper selected option lookup failed".to_string())
}

fn record_buttons<'s>(
    scope: &mut v8::HandleScope<'s>,
    record: v8::Local<'s, v8::Value>,
) -> Result<v8::Local<'s, v8::Array>, String> {
    let value = js_property(scope, record, "buttons")?;
    as_array(scope, value, "record buttons")
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

fn stepper_options_array<'s>(
    scope: &mut v8::HandleScope<'s>,
    value: v8::Local<'s, v8::Value>,
) -> Result<Option<v8::Local<'s, v8::Array>>, String> {
    if value.is_null_or_undefined() {
        return Ok(None);
    }
    value
        .to_object(scope)
        .and_then(|object| v8::Local::<v8::Array>::try_from(object).ok())
        .map(Some)
        .ok_or_else(|| "expected paint stepper options".to_string())
}

fn stepper_option<'s>(
    scope: &mut v8::HandleScope<'s>,
    options: v8::Local<'s, v8::Array>,
    index: u32,
) -> Result<v8::Local<'s, v8::String>, String> {
    let item = options
        .get_index(scope, index)
        .ok_or_else(|| "paint stepper option lookup failed".to_string())?;
    if item.is_null_or_undefined() {
        return v8::String::new(scope, "")
            .ok_or_else(|| "paint stepper option string allocation failed".to_string());
    }
    item.to_string(scope)
        .or_else(|| v8::String::new(scope, ""))
        .ok_or_else(|| "paint stepper option string allocation failed".to_string())
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
#[cfg(test)]
#[path = "paint_chrome_v8_tests.rs"]
mod tests;
