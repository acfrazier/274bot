//! Bounded air ranking over the immutable selected-world catalog.
use super::callback_v8;
use crate::bank_select::{self, FromTile};
use api::snapshot::WorldTile;
use rustyscript::deno_core::serde_v8;
use rustyscript::Runtime;

pub(super) fn install(runtime: &mut Runtime) -> Result<(), String> {
    callback_v8::install(runtime, "__rs2b0t_nearest_bank", nearest)
}

fn nearest<'s>(scope: &mut v8::HandleScope<'s>, args: v8::FunctionCallbackArguments<'s>, rv: v8::ReturnValue) {
    let result = (|| {
        let from = serde_v8::from_v8::<FromTile>(scope, args.get(0))
            .map_err(|e| callback_v8::type_error(scope, &format!("bank origin: {e}")))?;
        let value = bank_select::nearest(WorldTile { x: from.x, z: from.z, level: from.level });
        serde_v8::to_v8(scope, value)
            .map_err(|e| callback_v8::type_error(scope, &format!("bank result: {e}")))
    })();
    callback_v8::finish(scope, rv, result);
}
