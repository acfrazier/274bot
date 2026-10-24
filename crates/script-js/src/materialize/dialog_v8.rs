//! Quest `pickPreferred(options, prefer)` as one native call:
//! `__rs2b0t_pick_preferred(options, prefer)` returns the index of the
//! frozen pick ([`crate::dialog::pick_preferred`]) or `-1`. JavaScript
//! passes the option and fragment strings and maps the index back.

use super::callback_v8;
use rustyscript::deno_core::serde_v8;
use rustyscript::Runtime;

pub(super) fn install(runtime: &mut Runtime) -> Result<(), String> {
    callback_v8::install(runtime, "__rs2b0t_pick_preferred", pick_preferred)?;
    callback_v8::install(runtime, "__rs2b0t_pick_by_line", pick_by_line)?;
    callback_v8::install(runtime, "__rs2b0t_is_underground", is_underground)?;
    callback_v8::install(runtime, "__rs2b0t_needs_hop", needs_hop)
}

fn pick_preferred<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue,
) {
    let options = serde_v8::from_v8::<Vec<String>>(scope, args.get(0));
    let prefer = serde_v8::from_v8::<Vec<String>>(scope, args.get(1));
    let result = match (options, prefer) {
        (Ok(options), Ok(prefer)) => {
            let index = crate::dialog::pick_preferred(&options, &prefer);
            Ok(callback_v8::num(scope, index.map_or(-1.0, |i| i as f64)))
        }
        _ => Err(callback_v8::type_error(
            scope,
            "pickPreferred: options and prefer must be string arrays",
        )),
    };
    callback_v8::finish(scope, rv, result);
}

fn pick_by_line<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue,
) {
    let lines = serde_v8::from_v8::<Vec<String>>(scope, args.get(0));
    let options = serde_v8::from_v8::<Vec<String>>(scope, args.get(1));
    let rules = serde_v8::from_v8::<Vec<crate::dialog::LineRule>>(scope, args.get(2));
    let result = match (lines, options, rules) {
        (Ok(lines), Ok(options), Ok(rules)) => {
            let value = match crate::dialog::pick_by_line(&lines, &options, &rules) {
                Some(index) => callback_v8::string(scope, &options[index]),
                None => v8::null(scope).into(),
            };
            Ok(value)
        }
        _ => Err(callback_v8::type_error(
            scope,
            "pickByLine: lines, options, and rules must have the frozen string-array shapes",
        )),
    };
    callback_v8::finish(scope, rv, result);
}

fn is_underground<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue,
) {
    let result = callback_v8::number(scope, args.get(0))
        .map(|z| v8::Boolean::new(scope, crate::reach_entity::is_underground_z(z)).into());
    callback_v8::finish(scope, rv, result);
}

fn needs_hop<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue,
) {
    let result = callback_v8::number(scope, args.get(0)).and_then(|here_z| {
        callback_v8::number(scope, args.get(1)).map(|anchor_z| {
            v8::Boolean::new(scope, crate::reach_entity::needs_hop_z(here_z, anchor_z)).into()
        })
    });
    callback_v8::finish(scope, rv, result);
}
