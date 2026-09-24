//! Typed canvas op tape: one crossing per recorded-op flush.
//!
//! The PRELUDE paint ctx records its calls into a `Float64Array` plus a
//! deduplicated string table instead of making one `__rs2b0t_canvas_*` call per
//! member. `__rs2b0t_canvas_submit` replays that tape in order onto the
//! isolate recorder ([`crate::canvas`]), so the recorded ops are the ops the
//! script made, in the order it made them. Not a FlatBuffer RPC and not a host
//! game-action wire.
//!
//! Op codes and arity are mirrored by the PRELUDE's `CANVAS_OP` table; a tape
//! that does not decode is an error the ctx throws, never a silently dropped
//! frame.

use rustyscript::Runtime;
use std::cell::RefCell;

const SET: i64 = 1;
const SET_NUM: i64 = 2;
const FILL_GRADIENT: i64 = 3;
const FILL_RECT: i64 = 4;
const FILL_TEXT: i64 = 5;
const SAVE: i64 = 6;
const RESTORE: i64 = 7;
const BEGIN_PATH: i64 = 8;
const CLOSE_PATH: i64 = 9;
const MOVE_TO: i64 = 10;
const LINE_TO: i64 = 11;
const QUAD_TO: i64 = 12;
const ARC: i64 = 13;
const FILL: i64 = 14;
const STROKE: i64 = 15;
const CLIP: i64 = 16;
const CREATE_LINEAR: i64 = 17;
const CREATE_RADIAL: i64 = 18;
const ADD_STOP: i64 = 19;

/// Arguments per op code; `None` is a malformed tape.
fn arity(op: i64) -> Option<usize> {
    Some(match op {
        SET | SET_NUM | MOVE_TO | LINE_TO => 2,
        FILL_GRADIENT => 1,
        FILL_TEXT | ADD_STOP => 3,
        FILL_RECT | QUAD_TO | CREATE_LINEAR => 4,
        ARC | CREATE_RADIAL => 6,
        SAVE | RESTORE | BEGIN_PATH | CLOSE_PATH | FILL | STROKE | CLIP => 0,
        _ => return None,
    })
}

thread_local! {
    /// Reused read buffers for the typed tape (no per-flush allocation).
    static TAPE_BYTES: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
    static TAPE_OPS: RefCell<Vec<f64>> = const { RefCell::new(Vec::new()) };
}

pub(super) fn install(runtime: &mut Runtime) -> Result<(), String> {
    let context = runtime.deno_runtime().main_context();
    let mut scope = runtime.deno_runtime().handle_scope();
    let global = context.open(&mut scope).global(&mut scope);
    let name = v8::String::new(&mut scope, "__rs2b0t_canvas_submit")
        .ok_or_else(|| "canvas tape name".to_string())?;
    let func = v8::Function::new(&mut scope, submit_callback)
        .ok_or_else(|| "canvas tape fn".to_string())?;
    global
        .set(&mut scope, name.into(), func.into())
        .ok_or_else(|| "canvas tape set".to_string())?;
    Ok(())
}

fn submit_callback(
    scope: &mut v8::HandleScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    // `undefined` on success, the message on failure: the ctx throws it so a
    // bad op reads like the call-site error the per-member path raised.
    if let Err(msg) = run(scope, &args) {
        if let Some(text) = v8::String::new(scope, &msg) {
            rv.set(text.into());
        }
    }
}

fn run(scope: &mut v8::HandleScope, args: &v8::FunctionCallbackArguments) -> Result<(), String> {
    let strs = string_table(scope, args.get(1))?;
    let view = v8::Local::<v8::ArrayBufferView>::try_from(args.get(0))
        .map_err(|_| "canvas tape: expected a Float64Array".to_string())?;
    TAPE_BYTES.with(|bytes| {
        let mut bytes = bytes.borrow_mut();
        let len = view.byte_length();
        bytes.clear();
        bytes.resize(len, 0);
        let written = view.copy_contents(&mut bytes);
        if written != len || len % 8 != 0 {
            return Err(format!("canvas tape: read {written} of {len} bytes"));
        }
        TAPE_OPS.with(|ops| {
            let mut ops = ops.borrow_mut();
            ops.clear();
            ops.extend(
                bytes
                    .as_chunks::<8>()
                    .0
                    .iter()
                    .map(|c| f64::from_ne_bytes(*c)),
            );
            replay(&ops, &strs)
        })
    })
}

/// The ctx's deduplicated string table: prop names, text and colors.
fn string_table(
    scope: &mut v8::HandleScope,
    value: v8::Local<v8::Value>,
) -> Result<Vec<String>, String> {
    let arr = v8::Local::<v8::Array>::try_from(value)
        .map_err(|_| "canvas tape: expected a string table".to_string())?;
    let len = arr.length();
    let mut out = Vec::with_capacity(len as usize);
    for i in 0..len {
        let item = arr
            .get_index(scope, i)
            .ok_or_else(|| "canvas tape: string table read failed".to_string())?;
        out.push(item.to_rust_string_lossy(scope));
    }
    Ok(out)
}

fn replay(tape: &[f64], strs: &[String]) -> Result<(), String> {
    let s = |v: f64| -> &str {
        // Only the ctx writes these: an in-range index, never a negative one.
        let index = if v >= 0.0 { v as usize } else { usize::MAX };
        strs.get(index).map(String::as_str).unwrap_or("")
    };
    let mut i = 0usize;
    while i < tape.len() {
        let op = tape[i];
        let Some(n) = arity(op as i64) else {
            return Err(format!("canvas tape: bad op {op}"));
        };
        let args = tape
            .get(i + 1..i + 1 + n)
            .ok_or_else(|| "canvas tape: truncated args".to_string())?;
        match op as i64 {
            SET => crate::canvas::set_style(s(args[0]), s(args[1])),
            SET_NUM => crate::canvas::set_number(s(args[0]), args[1]),
            FILL_GRADIENT => crate::canvas::set_fill_gradient(args[0] as u32),
            FILL_RECT => crate::canvas::fill_rect(args[0], args[1], args[2], args[3]),
            FILL_TEXT => crate::canvas::fill_text(s(args[0]), args[1], args[2]),
            SAVE => crate::canvas::save(),
            RESTORE => crate::canvas::restore(),
            BEGIN_PATH => crate::canvas::begin_path(),
            CLOSE_PATH => crate::canvas::close_path(),
            MOVE_TO => crate::canvas::move_to(args[0], args[1]),
            LINE_TO => crate::canvas::line_to(args[0], args[1]),
            QUAD_TO => crate::canvas::quadratic_curve_to(args[0], args[1], args[2], args[3]),
            ARC => crate::canvas::arc(args[0], args[1], args[2], args[3], args[4], args[5] != 0.0)?,
            FILL => crate::canvas::fill(),
            STROKE => crate::canvas::stroke(),
            CLIP => crate::canvas::clip(),
            // The ctx assigns the same ids the recorder does (creation order),
            // so `addColorStop` and `fillStyle = gradient` reference them
            // without a return crossing.
            CREATE_LINEAR => {
                crate::canvas::create_linear(args[0], args[1], args[2], args[3])?;
            }
            CREATE_RADIAL => {
                crate::canvas::create_radial(args[0], args[1], args[2], args[3], args[4], args[5])?;
            }
            ADD_STOP => crate::canvas::add_color_stop(args[0] as u32, args[1], s(args[2]))?,
            _ => unreachable!("arity() admits only the codes above"),
        }
        i += 1 + n;
    }
    Ok(())
}
