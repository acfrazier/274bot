//! Quest `pickPreferred(options, prefer)` as one native call:
//! `__rs2b0t_pick_preferred(options, prefer)` returns the index of the
//! frozen pick ([`crate::dialog::pick_preferred`]) or `-1`. JavaScript
//! passes the option and fragment strings and maps the index back.

use super::callback_v8;
use rustyscript::deno_core::serde_v8;
use rustyscript::Runtime;

pub(super) fn install(runtime: &mut Runtime) -> Result<(), String> {
    callback_v8::install(runtime, "__rs2b0t_pick_preferred", pick_preferred)
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
