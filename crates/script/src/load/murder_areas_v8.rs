//! Native ordering helper for the frozen Murder Mystery suspect list.

use super::callback_v8::{self as cb, JsResult, Poll};
use rustyscript::Runtime;

pub(super) fn install(runtime: &mut Runtime) -> Result<(), String> {
    cb::install(runtime, "__rs2b0t_murder_areas", murder_areas)
}

fn murder_areas<'s>(
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
    let op = args.get(0).to_rust_string_lossy(scope);
    match op.as_str() {
        "suspectOrder" => suspect_order(scope, args.get(1), args.get(2)),
        _ => Err(cb::not_impl(scope, "Murder.areas")),
    }
}

fn suspect_order<'s>(
    scope: &mut v8::HandleScope<'s>,
    thread: v8::Local<'s, v8::Value>,
    suspects_value: v8::Local<'s, v8::Value>,
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    let Ok(suspects) = v8::Local::<v8::Array>::try_from(suspects_value) else {
        return Err(cb::type_error(scope, "SUSPECTS.filter is not a function"));
    };
    let length = suspects.length();
    let matched = v8::Array::new(scope, 0);
    let poll = Poll::default();
    let mut matched_length = 0;

    for index in 0..length {
        poll.check(scope)?;
        if !cb::catching(scope, |s| suspects.has_index(s, index))? {
            continue;
        }
        let suspect = cb::get_index(scope, suspects_value, index)?;
        let suspect_thread = cb::get(scope, suspect, "thread")?;
        if suspect_thread.strict_equals(thread) {
            cb::catching(scope, |s| matched.set_index(s, matched_length, suspect))?;
            matched_length += 1;
        }
    }

    let ordered = v8::Array::new(scope, 0);
    let mut ordered_length = 0;
    for index in 0..matched_length {
        poll.check(scope)?;
        let suspect = cb::get_index(scope, matched.into(), index)?;
        cb::catching(scope, |s| ordered.set_index(s, ordered_length, suspect))?;
        ordered_length += 1;
    }
    for index in 0..length {
        poll.check(scope)?;
        if !cb::catching(scope, |s| suspects.has_index(s, index))? {
            continue;
        }
        let suspect = cb::get_index(scope, suspects_value, index)?;
        let mut is_matched = false;
        for matched_index in 0..matched_length {
            poll.check(scope)?;
            let candidate = cb::get_index(scope, matched.into(), matched_index)?;
            if candidate.strict_equals(suspect) {
                is_matched = true;
                break;
            }
        }
        if !is_matched {
            cb::catching(scope, |s| ordered.set_index(s, ordered_length, suspect))?;
            ordered_length += 1;
        }
    }

    Ok(ordered.into())
}
