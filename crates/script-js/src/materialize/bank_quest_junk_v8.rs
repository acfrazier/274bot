//! `bankQuestJunk` (`src/bot/api/bank/bankQuestJunk.ts:13-15,431-444`) over
//! the shared immutable Rust table [`api::quest_junk::quest_junk`].
//!
//! Only each caller row's `id` crosses into Rust, bounded by the selected
//! bank capacity. The caller's `statusOf` stays a V8 callback, called once per
//! matched table entry in table order. Nothing is dropped.

use super::callback_v8::{self as cb, JsResult};
use api::quest_junk::quest_junk;
use rustyscript::Runtime;

/// Most banked rows one call accepts: the 289 bank holds
/// `^bank_total_slots = 288` rows (`interface_bank/configs/bank.constant:1`).
pub(crate) const MAX_BANKED_ROWS: u32 = 288;

pub(crate) fn install(runtime: &mut Runtime) -> Result<(), String> {
    cb::install(runtime, "__rs2b0t_quest_junk_table", quest_junk_table)?;
    cb::install(runtime, "__rs2b0t_find_quest_junk", find_quest_junk)
}

/// The published `QUEST_JUNK` rows `{ id, name, quest }` in table order. The
/// shim evaluates this once per isolate and exports that one array.
fn quest_junk_table<'s>(
    scope: &mut v8::HandleScope<'s>,
    _args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue,
) {
    let result = (|| {
        let table = junk_table(scope)?;
        let array = v8::Array::new(scope, table.len() as i32);
        for (index, entry) in table.iter().enumerate() {
            let id = cb::num(scope, f64::from(entry.id));
            let name = cb::string(scope, entry.name);
            let quest = cb::string(scope, entry.quest);
            let row = cb::object(scope, &[("id", id), ("name", name), ("quest", quest)])?;
            cb::catching(scope, |s| array.set_index(s, index as u32, row))?;
        }
        Ok(array.into())
    })();
    cb::finish(scope, rv, result);
}

/// The quest-junk table, or a `TypeError` in a build without local game data.
fn junk_table<'s>(
    scope: &mut v8::HandleScope<'s>,
) -> JsResult<'s, &'static [api::quest_junk::QuestJunkEntry]> {
    quest_junk().map_err(|error| cb::type_error(scope, &format!("quest junk: {error:?}")))
}

/// `findQuestJunk(banked, statusOf)`: `held = new Set(banked.map(i => i.id))`,
/// then `{ ...entry, status, droppable: status === 'complete' }` for every
/// table entry whose id is held, in table order.
fn find_quest_junk<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue,
) {
    let result = find(scope, args.get(0), args.get(1));
    cb::finish(scope, rv, result);
}

fn find<'s>(
    scope: &mut v8::HandleScope<'s>,
    banked: v8::Local<'s, v8::Value>,
    status_of: v8::Local<'s, v8::Value>,
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    if !banked.is_array() {
        return Err(cb::type_error(scope, "banked.map is not a function"));
    }
    let length = cb::array_length(scope, banked)?;
    // Reject an oversized list before any row or callback is read.
    if length > MAX_BANKED_ROWS {
        let message =
            format!("findQuestJunk accepts at most {MAX_BANKED_ROWS} banked rows (got {length})");
        let message = v8::String::new(scope, &message).unwrap_or_else(|| v8::String::empty(scope));
        return Err(cb::Throw::Value(v8::Exception::range_error(scope, message)));
    }
    let mut held = [0_i32; MAX_BANKED_ROWS as usize];
    let mut held_len = 0;
    for index in 0..length {
        let row = cb::get_index(scope, banked, index)?;
        let id = cb::get(scope, row, "id")?;
        // Only a number can equal a table id under `Set.has`; reading it
        // runs no conversion of a non-number value.
        let Some(id) = id
            .is_number()
            .then(|| id.number_value(scope))
            .flatten()
            .filter(|id| {
                id.fract() == 0.0 && (f64::from(i32::MIN)..=f64::from(i32::MAX)).contains(id)
            })
        else {
            continue;
        };
        held[held_len] = id as i32;
        held_len += 1;
    }
    let held = &held[..held_len];
    let out = v8::Array::new(scope, 0);
    let mut written = 0_u32;
    for entry in junk_table(scope)? {
        if !held.contains(&entry.id) {
            continue;
        }
        let quest = cb::string(scope, entry.quest);
        let undefined = v8::undefined(scope).into();
        let status = cb::call(scope, status_of, undefined, &[quest], "statusOf")?;
        let complete = status.is_string() && status.to_rust_string_lossy(scope) == "complete";
        let id = cb::num(scope, f64::from(entry.id));
        let name = cb::string(scope, entry.name);
        let droppable = v8::Boolean::new(scope, complete).into();
        let finding = cb::object(
            scope,
            &[
                ("id", id),
                ("name", name),
                ("quest", quest),
                ("status", status),
                ("droppable", droppable),
            ],
        )?;
        cb::catching(scope, |s| out.set_index(s, written, finding))?;
        written += 1;
    }
    Ok(out.into())
}

#[cfg(test)]
#[path = "bank_quest_junk_v8_tests.rs"]
mod tests;
