//! `Bank.normalBackpackSnapshot` (`src/bot/api/bank/Bank.ts:116-120`): the
//! posted backpack as `{ slot, id, name, count }` rows while the bank is
//! closed, the inv tab has 28 slots and its transmission is ready, else
//! `null`. Readiness is the posted `inv_ready` fact the host derives from the
//! inv tab's transmission (`login_logout/login.rs2:98`), the same fact the
//! reader projection reads.

use super::callback_v8::{self as cb, JsResult};
use rustyscript::Runtime;

/// The 289 backpack size (`player/configs/player.inv:1-3`).
const BACKPACK_SLOTS: i32 = 28;

pub(super) fn install(runtime: &mut Runtime) -> Result<(), String> {
    cb::install(
        runtime,
        "__rs2b0t_bank_normal_backpack",
        normal_backpack_snapshot,
    )
}

fn normal_backpack_snapshot<'s>(
    scope: &mut v8::HandleScope<'s>,
    _args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue,
) {
    let result = crate::observed::with(|scene| -> JsResult<'s, v8::Local<'s, v8::Value>> {
        let latest = scene.latest();
        if latest.bank_open().unwrap_or(false)
            || latest.inv_size() != Some(BACKPACK_SLOTS)
            || !latest.inv_ready().unwrap_or(false)
        {
            return Ok(v8::null(scope).into());
        }
        let rows = latest.inv().map_or(&[][..], Vec::as_slice);
        let out = v8::Array::new(scope, rows.len() as i32);
        for (index, row) in rows.iter().enumerate() {
            let slot = cb::num(scope, f64::from(row.slot.unwrap_or(-1)));
            let id = cb::num(scope, f64::from(row.id));
            let name = match row.name.as_deref() {
                Some(name) => cb::string(scope, name),
                None => v8::null(scope).into(),
            };
            let count = cb::num(scope, f64::from(row.count));
            let item = cb::object(
                scope,
                &[("slot", slot), ("id", id), ("name", name), ("count", count)],
            )?;
            cb::catching(scope, |s| out.set_index(s, index as u32, item))?;
        }
        Ok(out.into())
    });
    cb::finish(scope, rv, result);
}

#[cfg(test)]
#[path = "bank_backpack_v8_tests.rs"]
mod tests;
