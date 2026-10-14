use super::callback_v8::{self as cb, Callback, JsResult, Poll};
use rustyscript::Runtime;

const FACES: [(i32, i32); 4] = [(-1, 0), (0, 1), (1, 0), (0, -1)];
const TOWARD_SLACK: f64 = 4.0;

pub(super) fn install(runtime: &mut Runtime) -> Result<(), String> {
    cb::install(runtime, "__rs2b0t_walk_opening", walk_opening)
}

fn walk_opening<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue,
) {
    let result = run(scope, &args);
    cb::finish(scope, rv, result);
}

fn run<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: &v8::FunctionCallbackArguments<'s>,
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    match args.get(0).to_rust_string_lossy(scope).as_str() {
        "faceSteps" => face_steps(scope, args.get(1), args.get(2)),
        "openedNeighbour" => opened_neighbour(scope, args.get(1), args.get(2), args.get(3)),
        "farSide" => far_side(scope, args.get(1), args.get(2), args.get(3)),
        "isOpenableObstacle" => is_openable_obstacle(scope, args.get(1), args.get(2), args.get(3)),
        "openOp" => first_op(scope, args.get(1), "^open"),
        "closeOp" => first_op(scope, args.get(1), "^close"),
        "towardDest" => toward_dest(scope, args.get(1), args.get(2), args.get(3)),
        _ => Err(cb::not_impl(scope, "walkOpening")),
    }
}

fn face_steps<'s>(
    scope: &mut v8::HandleScope<'s>,
    door: v8::Local<'s, v8::Value>,
    can_step: v8::Local<'s, v8::Value>,
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    let can_step = Callback::plain(scope, can_step, "canStep");
    let result = v8::Array::new(scope, FACES.len() as i32);
    for index in 0..FACES.len() {
        let neighbour = face_tile(scope, door, index)?;
        let answer = can_step.call(scope, &[door, neighbour])?;
        result
            .set_index(scope, index as u32, answer)
            .ok_or_else(|| cb::type_error(scope, "faceSteps result"))?;
    }
    Ok(result.into())
}

fn opened_neighbour<'s>(
    scope: &mut v8::HandleScope<'s>,
    door: v8::Local<'s, v8::Value>,
    before: v8::Local<'s, v8::Value>,
    after: v8::Local<'s, v8::Value>,
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    let length = cb::array_length(scope, after)?;
    for index in 0..length {
        let open = cb::get_index(scope, after, index)?;
        if !cb::truthy(scope, open) {
            continue;
        }
        let was_open = cb::get_index(scope, before, index)?;
        if cb::truthy(scope, was_open) {
            continue;
        }
        return face_tile(scope, door, index as usize);
    }
    Ok(v8::null(scope).into())
}

fn face_tile<'s>(
    scope: &mut v8::HandleScope<'s>,
    door: v8::Local<'s, v8::Value>,
    face: usize,
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    let Some((dx, dz)) = FACES.get(face) else {
        return Err(cb::type_error(scope, "door face is out of range"));
    };
    let x = cb::get(scope, door, "x")?;
    let dx = cb::num(scope, f64::from(*dx));
    let x = cb::add(scope, x, dx)?;
    let z = cb::get(scope, door, "z")?;
    let dz = cb::num(scope, f64::from(*dz));
    let z = cb::add(scope, z, dz)?;
    let level = cb::get(scope, door, "level")?;
    cb::object(scope, &[("x", x), ("z", z), ("level", level)])
}

fn far_side<'s>(
    scope: &mut v8::HandleScope<'s>,
    door: v8::Local<'s, v8::Value>,
    across: v8::Local<'s, v8::Value>,
    here: v8::Local<'s, v8::Value>,
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    let here_x = cb::get(scope, here, "x")?;
    let door_x = cb::get(scope, door, "x")?;
    if here_x.strict_equals(door_x) {
        let here_z = cb::get(scope, here, "z")?;
        let door_z = cb::get(scope, door, "z")?;
        if here_z.strict_equals(door_z) {
            return Ok(across);
        }
    }
    let here_x = cb::get(scope, here, "x")?;
    let across_x = cb::get(scope, across, "x")?;
    if here_x.strict_equals(across_x) {
        let here_z = cb::get(scope, here, "z")?;
        let across_z = cb::get(scope, across, "z")?;
        if here_z.strict_equals(across_z) {
            return Ok(door);
        }
    }
    let across_distance = chebyshev(scope, here, across)?;
    let door_distance = chebyshev(scope, here, door)?;
    Ok(if across_distance >= door_distance {
        across
    } else {
        door
    })
}

fn is_openable_obstacle<'s>(
    scope: &mut v8::HandleScope<'s>,
    name: v8::Local<'s, v8::Value>,
    actions: v8::Local<'s, v8::Value>,
    obstacles: v8::Local<'s, v8::Value>,
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    let name = if name.is_null_or_undefined() {
        cb::string(scope, "")
    } else {
        name
    };
    let name = cb::call_method(scope, name, "toLowerCase", &[], "toLowerCase")?;
    let obstacle_count = cb::array_length(scope, obstacles)?;
    let poll = Poll::default();
    let mut matched = false;
    for index in 0..obstacle_count {
        poll.check(scope)?;
        if !cb::has_index(scope, obstacles, index)? {
            continue;
        }
        let keyword = cb::get_index(scope, obstacles, index)?;
        let includes = cb::call_method(scope, name, "includes", &[keyword], "includes")?;
        if cb::truthy(scope, includes) {
            matched = true;
            break;
        }
    }
    if !matched {
        return Ok(v8::Boolean::new(scope, false).into());
    }

    let action_count = cb::array_length(scope, actions)?;
    if action_count == 0 {
        return Ok(v8::Boolean::new(scope, false).into());
    }
    let regex = prefix_regex(scope, "^open")?;
    for index in 0..action_count {
        poll.check(scope)?;
        if !cb::has_index(scope, actions, index)? {
            continue;
        }
        let action = cb::get_index(scope, actions, index)?;
        if regex_test(scope, regex, action)? {
            return Ok(v8::Boolean::new(scope, true).into());
        }
    }
    Ok(v8::Boolean::new(scope, false).into())
}

fn first_op<'s>(
    scope: &mut v8::HandleScope<'s>,
    actions: v8::Local<'s, v8::Value>,
    pattern: &str,
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    let count = cb::array_length(scope, actions)?;
    if count == 0 {
        return Ok(v8::null(scope).into());
    }
    let regex = prefix_regex(scope, pattern)?;
    let poll = Poll::default();
    for index in 0..count {
        poll.check(scope)?;
        let action = cb::get_index(scope, actions, index)?;
        if regex_test(scope, regex, action)? {
            return Ok(action);
        }
    }
    Ok(v8::null(scope).into())
}

fn prefix_regex<'s>(
    scope: &mut v8::HandleScope<'s>,
    pattern: &str,
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    let pattern = v8::String::new(scope, pattern).unwrap_or_else(|| v8::String::empty(scope));
    let regex = v8::RegExp::new(scope, pattern, v8::RegExpCreationFlags::IGNORE_CASE)
        .ok_or_else(|| cb::type_error(scope, "walkOpening action pattern"))?;
    Ok(regex.into())
}

fn regex_test<'s>(
    scope: &mut v8::HandleScope<'s>,
    regex: v8::Local<'s, v8::Value>,
    value: v8::Local<'s, v8::Value>,
) -> JsResult<'s, bool> {
    let result = cb::call_method(scope, regex, "test", &[value], "RegExp.test")?;
    Ok(cb::truthy(scope, result))
}

fn toward_dest<'s>(
    scope: &mut v8::HandleScope<'s>,
    door: v8::Local<'s, v8::Value>,
    here: v8::Local<'s, v8::Value>,
    dest: v8::Local<'s, v8::Value>,
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    let door_distance = chebyshev(scope, door, dest)?;
    let here_distance = chebyshev(scope, here, dest)?;
    let toward = door_distance <= here_distance + TOWARD_SLACK;
    Ok(v8::Boolean::new(scope, toward).into())
}

fn chebyshev<'s>(
    scope: &mut v8::HandleScope<'s>,
    a: v8::Local<'s, v8::Value>,
    b: v8::Local<'s, v8::Value>,
) -> JsResult<'s, f64> {
    let a_x = cb::get(scope, a, "x")?;
    let ax = cb::number(scope, a_x)?;
    let b_x = cb::get(scope, b, "x")?;
    let bx = cb::number(scope, b_x)?;
    let a_z = cb::get(scope, a, "z")?;
    let az = cb::number(scope, a_z)?;
    let b_z = cb::get(scope, b, "z")?;
    let bz = cb::number(scope, b_z)?;
    let dx = (ax - bx).abs();
    let dz = (az - bz).abs();
    Ok(if dx.is_nan() || dz.is_nan() {
        f64::NAN
    } else {
        dx.max(dz)
    })
}
