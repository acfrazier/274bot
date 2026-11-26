//! Direct bindings to the run's recorder. The Lua entry only shapes the
//! context and gradient handles and raises the errors returned by Rust.

use mlua::{BorrowedStr, Error, Lua, LuaString, Value};
use script::canvas::{CanvasError, CanvasRecorder};

// The largest operation (radial gradient or arc) takes six arguments. Use
// fixed values so recording does not allocate an argument list on the host.
pub(crate) type CanvasArgs = (LuaString, Value, Value, Value, Value, Value, Value);

pub(crate) fn record(
    lua: &Lua,
    recorder: &mut CanvasRecorder,
    args: CanvasArgs,
) -> mlua::Result<Value> {
    let (op, a, b, c, d, e, f) = args;
    match op.to_str()?.as_ref() {
        "getStyle" => {
            let prop = canvas_text(&a)?;
            let id = recorder.fill_gradient_id();
            if prop.as_ref() == "fillStyle" && id >= 0 {
                return Ok(Value::Integer(i64::from(id)));
            }
            let css = lua.create_string(recorder.get_style(prop.as_ref()))?;
            return Ok(Value::String(css));
        }
        "getNumber" => {
            return Ok(Value::Number(
                recorder.get_number(canvas_text(&a)?.as_ref()),
            ))
        }
        "setStyle" => recorder.set_style(canvas_text(&a)?.as_ref(), canvas_text(&b)?.as_ref()),
        "setNumber" => recorder.set_number(canvas_text(&a)?.as_ref(), canvas_number(&b)?),
        "setFillGradient" => recorder.set_fill_gradient(gradient_id(&a)?),
        "fillRect" => recorder.fill_rect(
            canvas_number(&a)?,
            canvas_number(&b)?,
            canvas_number(&c)?,
            canvas_number(&d)?,
        ),
        "fillText" => recorder.fill_text(
            canvas_text(&a)?.as_ref(),
            canvas_number(&b)?,
            canvas_number(&c)?,
        ),
        "measureText" => {
            let width = recorder
                .measure_text(canvas_text(&a)?.as_ref())
                .map_err(canvas_error)?;
            return Ok(Value::Number(width));
        }
        "beginPath" => recorder.begin_path(),
        "closePath" => recorder.close_path(),
        "moveTo" => recorder.move_to(canvas_number(&a)?, canvas_number(&b)?),
        "lineTo" => recorder.line_to(canvas_number(&a)?, canvas_number(&b)?),
        "quadraticCurveTo" => recorder.quadratic_curve_to(
            canvas_number(&a)?,
            canvas_number(&b)?,
            canvas_number(&c)?,
            canvas_number(&d)?,
        ),
        "arc" => recorder
            .arc(
                canvas_number(&a)?,
                canvas_number(&b)?,
                canvas_number(&c)?,
                canvas_number(&d)?,
                canvas_number(&e)?,
                match f {
                    Value::Nil => false,
                    Value::Boolean(ccw) => ccw,
                    _ => return Err(Error::runtime("canvas: arc direction must be a boolean")),
                },
            )
            .map_err(canvas_error)?,
        "fill" => recorder.fill(),
        "stroke" => recorder.stroke(),
        "clip" => recorder.clip(),
        "save" => recorder.save(),
        "restore" => recorder.restore(),
        "createLinearGradient" => {
            let id = recorder
                .create_linear(
                    canvas_number(&a)?,
                    canvas_number(&b)?,
                    canvas_number(&c)?,
                    canvas_number(&d)?,
                )
                .map_err(canvas_error)?;
            return Ok(Value::Integer(i64::from(id)));
        }
        "createRadialGradient" => {
            let id = recorder
                .create_radial(
                    canvas_number(&a)?,
                    canvas_number(&b)?,
                    canvas_number(&c)?,
                    canvas_number(&d)?,
                    canvas_number(&e)?,
                    canvas_number(&f)?,
                )
                .map_err(canvas_error)?;
            return Ok(Value::Integer(i64::from(id)));
        }
        "addColorStop" => recorder
            .add_color_stop(
                gradient_id(&a)?,
                canvas_number(&b)?,
                canvas_text(&c)?.as_ref(),
            )
            .map_err(canvas_error)?,
        _ => return Err(Error::runtime("canvas: unknown operation")),
    }
    Ok(Value::Nil)
}

fn canvas_number(value: &Value) -> mlua::Result<f64> {
    match value {
        Value::Integer(number) => Ok(*number as f64),
        Value::Number(number) if number.is_finite() => Ok(*number),
        // Non-finite values use the JS recorder tape's normalization.
        Value::Number(_) => Ok(0.0),
        _ => Err(Error::runtime("canvas: expected a number")),
    }
}

fn canvas_text(value: &Value) -> mlua::Result<BorrowedStr> {
    match value {
        Value::String(text) => text.to_str(),
        _ => Err(Error::runtime("canvas: expected a string")),
    }
}

fn gradient_id(value: &Value) -> mlua::Result<u32> {
    match value {
        Value::Integer(id) => {
            u32::try_from(*id).map_err(|_| canvas_error(CanvasError::UnknownGradient))
        }
        _ => Err(canvas_error(CanvasError::UnknownGradient)),
    }
}

fn canvas_error(error: CanvasError) -> Error {
    Error::runtime(error.to_string())
}
