//! Bank pack reads served by one native call each.
//!
//! `Bank.normalBackpackSnapshot` (`src/bot/api/bank/Bank.ts:116-120`): the
//! posted backpack as `{ slot, id, name, count }` rows while the bank is
//! closed, the inv tab has 28 slots and its transmission is ready, else
//! `null`. Readiness is the posted `inv_ready` fact the host derives from the
//! inv tab's transmission (`login_logout/login.rs2:98`), the same fact the
//! reader projection reads.
//!
//! `reader.bankItems` (`src/bot/adapter/ClientAdapter.ts:1123-1130`): the open
//! bank's withdraw-component rows as `{ slot, id, name, count, ops, comId }`;
//! no open bank component is an empty list. The projection is Rust: rows
//! without a slot or with a negative id are omitted, as the shared inventory
//! reader omits them.

use super::callback_v8::{self as cb, JsResult};
use crate::observed::ItemRow;
use rustyscript::Runtime;

/// The 289 backpack size (`player/configs/player.inv:1-3`).
const BACKPACK_SLOTS: i32 = 28;

pub(super) fn install(runtime: &mut Runtime) -> Result<(), String> {
    cb::install(
        runtime,
        "__rs2b0t_bank_normal_backpack",
        normal_backpack_snapshot,
    )?;
    cb::install(runtime, "__rs2b0t_bank_items", bank_items)
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

fn bank_items<'s>(
    scope: &mut v8::HandleScope<'s>,
    _args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue,
) {
    let result = crate::observed::with(|scene| -> JsResult<'s, v8::Local<'s, v8::Value>> {
        let latest = scene.latest();
        let rows = if latest.bank_open().unwrap_or(false) {
            latest.bank().map_or(&[][..], Vec::as_slice)
        } else {
            &[][..]
        };
        let listed = || {
            rows.iter()
                .filter(|row| row.slot.is_some_and(|slot| slot >= 0) && row.id >= 0)
        };
        let out = v8::Array::new(scope, listed().count() as i32);
        for (index, row) in listed().enumerate() {
            let item = bank_item(scope, row)?;
            cb::catching(scope, |s| out.set_index(s, index as u32, item))?;
        }
        Ok(out.into())
    });
    cb::finish(scope, rv, result);
}

fn bank_item<'s>(
    scope: &mut v8::HandleScope<'s>,
    row: &ItemRow,
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    let slot = cb::num(scope, f64::from(row.slot.unwrap_or(-1)));
    let id = cb::num(scope, f64::from(row.id));
    let name = match row.name.as_deref() {
        Some(name) => cb::string(scope, name),
        None => v8::null(scope).into(),
    };
    let count = cb::num(scope, f64::from(row.count));
    let ops = v8::Array::new(scope, row.ops.len() as i32);
    for (index, op) in row.ops.iter().enumerate() {
        let op = cb::string(scope, op);
        cb::catching(scope, |s| ops.set_index(s, index as u32, op))?;
    }
    let com_id = cb::num(scope, f64::from(row.component_id.unwrap_or(-1)));
    cb::object(
        scope,
        &[
            ("slot", slot),
            ("id", id),
            ("name", name),
            ("count", count),
            ("ops", ops.into()),
            ("comId", com_id),
        ],
    )
}

#[cfg(test)]
#[path = "bank_backpack_v8_tests.rs"]
mod tests;
