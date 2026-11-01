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
//! `Bank.items()` shares this reader array. The nested ops arrays, item rows
//! and outer list are frozen; the list is reused until the bank session or
//! posted item generation changes.

use super::callback_v8::{self as cb, JsResult};
use crate::observed::ItemRow;
use rustyscript::Runtime;
use std::cell::RefCell;
use std::rc::Rc;

/// The 289 backpack size (`player/configs/player.inv:1-3`).
const BACKPACK_SLOTS: i32 = 28;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct BankItemsKey {
    open: bool,
    bank_generation: u64,
    snapshot_generation: i64,
}

#[derive(Default)]
struct BankItemsCache {
    key: Option<BankItemsKey>,
    rows: Option<v8::Global<v8::Array>>,
}

pub(super) fn install(runtime: &mut Runtime) -> Result<(), String> {
    let cache = Rc::new(RefCell::new(BankItemsCache::default()));
    if !runtime.deno_runtime().v8_isolate().set_slot(cache) {
        return Err("Bank item cache already installed".to_string());
    }
    cb::install(
        runtime,
        "__rs2b0t_bank_normal_backpack",
        normal_backpack_snapshot,
    )?;
    cb::install(runtime, "__rs2b0t_bank_items", bank_items)
}

fn bank_items<'s>(
    scope: &mut v8::HandleScope<'s>,
    _args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue,
) {
    let result = (|| -> JsResult<'s, v8::Local<'s, v8::Value>> {
        let cache = scope
            .get_slot::<Rc<RefCell<BankItemsCache>>>()
            .cloned()
            .ok_or_else(|| cb::type_error(scope, "Bank item cache is not installed"))?;
        crate::observed::with(|scene| {
            let latest = scene.latest();
            let open = latest.bank_open().unwrap_or(false);
            let key = BankItemsKey {
                open,
                bank_generation: latest.bank_generation().unwrap_or(0),
                snapshot_generation: latest.bank_snapshot_generation().unwrap_or(-1),
            };
            let cacheable = !open || key.snapshot_generation >= 0;
            if cacheable {
                let cached = cache.borrow();
                if cached.key == Some(key) {
                    if let Some(rows) = cached.rows.as_ref() {
                        return Ok(v8::Local::new(scope, rows).into());
                    }
                }
            }

            let source = if open {
                latest.bank().map_or(&[][..], Vec::as_slice)
            } else {
                &[][..]
            };
            let rows = materialize_bank_items(scope, source)?;
            let mut cached = cache.borrow_mut();
            if cacheable {
                cached.key = Some(key);
                cached.rows = Some(v8::Global::new(scope, rows));
            } else {
                // An open bank without an item-generation token is never
                // reusable: a later post may replace its rows without a key.
                cached.key = None;
                cached.rows = None;
            }
            Ok(rows.into())
        })
    })();
    cb::finish(scope, rv, result);
}

fn materialize_bank_items<'s>(
    scope: &mut v8::HandleScope<'s>,
    rows: &[ItemRow],
) -> JsResult<'s, v8::Local<'s, v8::Array>> {
    let listed = rows
        .iter()
        .filter(|row| row.slot.is_some_and(|slot| slot >= 0) && row.id >= 0)
        .count();
    let length = i32::try_from(listed)
        .map_err(|_| cb::type_error(scope, "bank items exceed the V8 array limit"))?;
    let out = v8::Array::new(scope, length);
    for (index, row) in rows
        .iter()
        .filter(|row| row.slot.is_some_and(|slot| slot >= 0) && row.id >= 0)
        .enumerate()
    {
        let index = u32::try_from(index)
            .map_err(|_| cb::type_error(scope, "bank items exceed the V8 array limit"))?;
        let item = bank_item(scope, row)?;
        cb::catching(scope, |s| out.set_index(s, index, item))?;
    }
    freeze(scope, out.into())?;
    Ok(out)
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
        let index = u32::try_from(index)
            .map_err(|_| cb::type_error(scope, "bank ops exceed the V8 array limit"))?;
        let op = cb::string(scope, op);
        cb::catching(scope, |s| ops.set_index(s, index, op))?;
    }
    freeze(scope, ops.into())?;
    let com_id = cb::num(scope, f64::from(row.component_id.unwrap_or(-1)));
    let item = cb::object(
        scope,
        &[
            ("slot", slot),
            ("id", id),
            ("name", name),
            ("count", count),
            ("ops", ops.into()),
            ("comId", com_id),
        ],
    )?;
    freeze(scope, item)?;
    Ok(item.into())
}

fn freeze<'s>(
    scope: &mut v8::HandleScope<'s>,
    value: v8::Local<'s, v8::Value>,
) -> JsResult<'s, ()> {
    let object = cb::catching(scope, |s| value.to_object(s))?;
    if cb::catching(scope, |s| {
        object.set_integrity_level(s, v8::IntegrityLevel::Frozen)
    })? {
        Ok(())
    } else {
        Err(cb::type_error(
            scope,
            "bank item snapshot could not be frozen",
        ))
    }
}

#[cfg(test)]
#[path = "bank_backpack_v8_tests.rs"]
mod tests;

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
