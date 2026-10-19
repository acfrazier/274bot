//! Native V8 adapters for posted inventory, equipment, and pack rules.
//!
//! The callbacks read the retained native observation. They do not accept or
//! retain a JavaScript copy of any posted page. Interaction requests remain
//! ordinary entries in the existing isolate-to-host queue.

use super::callback_v8 as cb;
use crate::bank::ops::side_observation;
use crate::observed::{self, ItemRow};
use rustyscript::Runtime;

pub(super) fn install(runtime: &mut Runtime) -> Result<(), String> {
    cb::install(runtime, "__rs2b0t_inventory", inventory_callback)?;
    cb::install(runtime, "__rs2b0t_pack_rules", pack_rules_callback)
}

fn inventory_callback<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue,
) {
    let result = inventory(scope, args);
    cb::finish(scope, rv, result);
}

fn pack_rules_callback<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue,
) {
    let result = pack_rules(scope, args);
    cb::finish(scope, rv, result);
}

fn inventory<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: v8::FunctionCallbackArguments<'s>,
) -> cb::JsResult<'s, v8::Local<'s, v8::Value>> {
    let operation = args.get(0);
    if !operation.is_int32() {
        return Err(cb::not_impl(scope, "Inventory operation"));
    }
    match operation.int32_value(scope).unwrap_or(-1) {
        0 => backpack_rows(scope),
        1 => backpack_capacity(scope, args.get(1)),
        2 => inventory_items(scope, args.get(1)),
        3 => inventory_first(scope, args.get(1), args.get(2)),
        4 => inventory_used(scope),
        5 => inventory_count(scope, args.get(1)),
        6 => inventory_count_by_id(scope, args.get(1)),
        7 => inventory_is_full(scope, args.get(1), args.get(2)),
        8 => inventory_free(scope, args.get(1), args.get(2)),
        9 => inventory_interact(scope, args.get(1), args.get(2)),
        10 => inventory_use_on(scope, args.get(1), args.get(2), args.get(3), args.get(4)),
        11 => equipment_items(scope),
        12 => equipment_contains(scope, args.get(1)),
        13 => equipment_equip(scope, args.get(1), args.get(2), args.get(3)),
        14 => equipment_unequip(scope, args.get(1), args.get(2)),
        15 => inventory_item_init(scope, args.get(1), args.get(2), args.get(3), args.get(4)),
        16 => inventory_item_actions(scope, args.get(1)),
        17 => inventory_contains(scope, args.get(1), args.get(2)),
        _ => Err(cb::not_impl(scope, "Inventory operation")),
    }
}

fn pack_rules<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: v8::FunctionCallbackArguments<'s>,
) -> cb::JsResult<'s, v8::Local<'s, v8::Value>> {
    let operation = args.get(0);
    if !operation.is_int32() {
        return Err(cb::not_impl(scope, "PackRules operation"));
    }
    match operation.int32_value(scope).unwrap_or(-1) {
        0 => matches_any(scope, args.get(1), args.get(2)),
        1 => count_matching(scope, args.get(1), args.get(2)),
        2 => slots_matching(scope, args.get(1), args.get(2)),
        3 => should_bank(scope, args.get(1), args.get(2), args.get(3)),
        4 => {
            let should = cb::lt(scope, args.get(1), args.get(2))?;
            Ok(v8::Boolean::new(scope, should).into())
        }
        5 => {
            let hp_is_low = cb::lt(scope, args.get(1), args.get(2))?;
            let zero = cb::num(scope, 0.0);
            let no_food = args.get(3).strict_equals(zero);
            Ok(v8::Boolean::new(scope, hp_is_low && no_food).into())
        }
        _ => Err(cb::not_impl(scope, "PackRules operation")),
    }
}

fn integer_field<'s>(
    scope: &mut v8::HandleScope<'s>,
    row: v8::Local<'s, v8::Value>,
    field: &str,
) -> cb::JsResult<'s, bool> {
    let global = scope.get_current_context().global(scope);
    let number = cb::get(scope, global.into(), "Number")?;
    let function = cb::get(scope, number, "isInteger")?;
    let value = cb::get(scope, row, field)?;
    let result = cb::call(scope, function, number, &[value], "Number.isInteger")?;
    Ok(cb::truthy(scope, result))
}

fn assign_item_field<'s>(
    scope: &mut v8::HandleScope<'s>,
    target: v8::Local<'s, v8::Value>,
    field: &str,
    value: v8::Local<'s, v8::Value>,
) -> cb::JsResult<'s, ()> {
    let key = cb::string(scope, field);
    cb::assign(scope, target, key, value)
}

fn inventory_item_init<'s>(
    scope: &mut v8::HandleScope<'s>,
    target: v8::Local<'s, v8::Value>,
    row: v8::Local<'s, v8::Value>,
    identity_key: v8::Local<'s, v8::Value>,
    row_key: v8::Local<'s, v8::Value>,
) -> cb::JsResult<'s, v8::Local<'s, v8::Value>> {
    // Read both isInteger callees before their corresponding argument getters.
    // Identity eligibility precedes ordinary field writes; its retained values
    // are read again only after those writes, as in the original constructor.
    let has_identity = integer_field(scope, row, "id")? && integer_field(scope, row, "slot")? && {
        let slot = cb::get(scope, row, "slot")?;
        let zero = cb::num(scope, 0.0);
        cb::ge(scope, slot, zero)?
    };
    for field in ["name", "count"] {
        let value = cb::get(scope, row, field)?;
        assign_item_field(scope, target, field, value)?;
    }
    for field in ["id", "slot"] {
        let mut value = cb::get(scope, row, field)?;
        if value.is_null_or_undefined() {
            value = cb::num(scope, 0.0);
        }
        assign_item_field(scope, target, field, value)?;
    }
    let noted = cb::get(scope, row, "noted")?;
    let noted = v8::Boolean::new(scope, noted.is_true()).into();
    assign_item_field(scope, target, "noted", noted)?;
    let mut cert = cb::get(scope, row, "cert")?;
    if cert.is_null_or_undefined() {
        cert = cb::num(scope, -1.0);
    }
    assign_item_field(scope, target, "cert", cert)?;
    let identity = if has_identity {
        let id = cb::get(scope, row, "id")?;
        let slot = cb::get(scope, row, "slot")?;
        cb::object(scope, &[("id", id), ("slot", slot)])?
    } else {
        v8::null(scope).into()
    };
    cb::assign(scope, target, identity_key, identity)?;
    cb::assign(scope, target, row_key, row)?;
    Ok(v8::undefined(scope).into())
}

fn inventory_item_actions<'s>(
    scope: &mut v8::HandleScope<'s>,
    row: v8::Local<'s, v8::Value>,
) -> cb::JsResult<'s, v8::Local<'s, v8::Value>> {
    let global = scope.get_current_context().global(scope);
    let array = cb::get(scope, global.into(), "Array")?;
    let function = cb::get(scope, array, "isArray")?;
    let ops = cb::get(scope, row, "ops")?;
    let is_array = cb::call(scope, function, array, &[ops], "Array.isArray")?;
    if cb::truthy(scope, is_array) {
        let ops = cb::get(scope, row, "ops")?;
        cb::call_method(scope, ops, "slice", &[], "slice")
    } else {
        Ok(v8::Array::new(scope, 0).into())
    }
}

fn inventory_contains<'s>(
    scope: &mut v8::HandleScope<'s>,
    inventory: v8::Local<'s, v8::Value>,
    name: v8::Local<'s, v8::Value>,
) -> cb::JsResult<'s, v8::Local<'s, v8::Value>> {
    let first = cb::call_method(scope, inventory, "first", &[name], "Inventory.first")?;
    Ok(v8::Boolean::new(scope, !first.is_null()).into())
}

fn global_call<'s>(
    scope: &mut v8::HandleScope<'s>,
    name: &str,
    args: &[v8::Local<'s, v8::Value>],
) -> cb::JsResult<'s, v8::Local<'s, v8::Value>> {
    let global = scope.get_current_context().global(scope);
    let function = cb::get(scope, global.into(), name)?;
    let receiver = v8::undefined(scope).into();
    cb::call(scope, function, receiver, args, name)
}

fn global_string<'s>(
    scope: &mut v8::HandleScope<'s>,
    value: v8::Local<'s, v8::Value>,
) -> cb::JsResult<'s, v8::Local<'s, v8::Value>> {
    global_call(scope, "String", &[value])
}

fn lower_string<'s>(
    scope: &mut v8::HandleScope<'s>,
    value: v8::Local<'s, v8::Value>,
) -> cb::JsResult<'s, v8::Local<'s, v8::Value>> {
    let string = global_string(scope, value)?;
    cb::call_method(scope, string, "toLowerCase", &[], "toLowerCase")
}

fn callback_function<'s>(
    scope: &mut v8::HandleScope<'s>,
    callback: impl v8::MapFnTo<v8::FunctionCallback>,
    data: v8::Local<'s, v8::Value>,
    length: i32,
    label: &str,
) -> cb::JsResult<'s, v8::Local<'s, v8::Value>> {
    let Some(function) = v8::Function::builder(callback)
        .data(data)
        .length(length)
        .build(scope)
    else {
        return Err(cb::type_error(scope, label));
    };
    Ok(function.into())
}

fn callback_without_data<'s>(
    scope: &mut v8::HandleScope<'s>,
    callback: impl v8::MapFnTo<v8::FunctionCallback>,
    length: i32,
    label: &str,
) -> cb::JsResult<'s, v8::Local<'s, v8::Value>> {
    let data = v8::undefined(scope).into();
    callback_function(scope, callback, data, length, label)
}

fn create_property<'s>(
    scope: &mut v8::HandleScope<'s>,
    object: v8::Local<'s, v8::Object>,
    key: &str,
    value: v8::Local<'s, v8::Value>,
) -> cb::JsResult<'s, ()> {
    let key = cb::string(scope, key);
    if cb::catching(scope, |s| object.create_data_property(s, key.cast(), value))? {
        Ok(())
    } else {
        Err(cb::type_error(
            scope,
            "inventory row materialization failed",
        ))
    }
}

fn create_index<'s>(
    scope: &mut v8::HandleScope<'s>,
    array: v8::Local<'s, v8::Array>,
    index: u32,
    value: v8::Local<'s, v8::Value>,
) -> cb::JsResult<'s, ()> {
    let integer = v8::Integer::new(scope, index as i32);
    let key = integer
        .to_string(scope)
        .unwrap_or_else(|| v8::String::empty(scope));
    if cb::catching(scope, |s| array.create_data_property(s, key.into(), value))? {
        Ok(())
    } else {
        Err(cb::type_error(
            scope,
            "inventory array materialization failed",
        ))
    }
}

fn item_row_object<'s>(
    scope: &mut v8::HandleScope<'s>,
    row: &ItemRow,
) -> cb::JsResult<'s, v8::Local<'s, v8::Value>> {
    let object = v8::Object::new(scope);
    let name = match row.name.as_deref() {
        Some(name) => cb::string(scope, name).into(),
        None => v8::null(scope).into(),
    };
    create_property(scope, object, "name", name)?;
    let count = cb::num(scope, f64::from(row.count));
    create_property(scope, object, "count", count)?;
    let id = cb::num(scope, f64::from(row.id));
    create_property(scope, object, "id", id)?;
    let ops = v8::Array::new(scope, row.ops.len() as i32);
    for (index, op) in row.ops.iter().enumerate() {
        let op = cb::string(scope, op.as_ref());
        create_index(scope, ops, index as u32, op.into())?;
    }
    create_property(scope, object, "ops", ops.into())?;
    let noted = v8::Boolean::new(scope, row.noted).into();
    create_property(scope, object, "noted", noted)?;
    let cert = cb::num(scope, f64::from(row.cert));
    create_property(scope, object, "cert", cert)?;
    if let Some(component_id) = row.component_id {
        let component_id = cb::num(scope, f64::from(component_id));
        create_property(scope, object, "component_id", component_id)?;
    }
    if let Some(slot) = row.slot {
        let slot = cb::num(scope, f64::from(slot));
        create_property(scope, object, "slot", slot)?;
    }
    Ok(object.into())
}

fn backpack_snapshot_array<'s>(
    scope: &mut v8::HandleScope<'s>,
    rows: &[ItemRow],
) -> cb::JsResult<'s, v8::Local<'s, v8::Value>> {
    let array = v8::Array::new(scope, rows.len() as i32);
    for (index, row) in rows.iter().enumerate() {
        let slot = row
            .slot
            .ok_or_else(|| cb::not_impl(scope, "backpackSnapshots"))?;
        let com_id = row
            .component_id
            .ok_or_else(|| cb::not_impl(scope, "backpackSnapshots"))?;
        let slot = cb::num(scope, f64::from(slot));
        let id = cb::num(scope, f64::from(row.id));
        let name = match row.name.as_deref() {
            Some(name) => cb::string(scope, name),
            None => v8::null(scope).into(),
        };
        let count = cb::num(scope, f64::from(row.count));
        let noted = v8::Boolean::new(scope, row.noted).into();
        let ops = v8::Array::new(scope, row.ops.len() as i32);
        for (op_index, op) in row.ops.iter().enumerate() {
            let op = cb::string(scope, op.as_ref());
            create_index(scope, ops, op_index as u32, op)?;
        }
        let com_id = cb::num(scope, f64::from(com_id));
        let snapshot = cb::object(
            scope,
            &[
                ("slot", slot),
                ("id", id),
                ("name", name),
                ("count", count),
                ("noted", noted),
                ("ops", ops.into()),
                ("comId", com_id),
            ],
        )?;
        create_index(scope, array, index as u32, snapshot)?;
    }
    Ok(array.into())
}

fn rows_array<'s>(
    scope: &mut v8::HandleScope<'s>,
    rows: &[ItemRow],
) -> cb::JsResult<'s, v8::Local<'s, v8::Value>> {
    let array = v8::Array::new(scope, rows.len() as i32);
    for (index, row) in rows.iter().enumerate() {
        let item = item_row_object(scope, row)?;
        create_index(scope, array, index as u32, item)?;
    }
    Ok(array.into())
}

fn backpack_rows<'s>(
    scope: &mut v8::HandleScope<'s>,
) -> cb::JsResult<'s, v8::Local<'s, v8::Value>> {
    observed::with(|scene| {
        let view = scene.since_login();
        let bank_open = view
            .bank_open()
            .ok_or_else(|| cb::not_impl(scope, "backpackSnapshots"))?;
        if bank_open {
            let rows = view
                .bank_side()
                .ok_or_else(|| cb::not_impl(scope, "backpackSnapshots"))?;
            let side_root = view.side_modal_id().unwrap_or(-1);
            let side = side_observation(true, side_root, rows)
                .ok_or_else(|| cb::not_impl(scope, "backpackSnapshots"))?;
            backpack_snapshot_array(scope, side)
        } else {
            let rows = view
                .inv()
                .ok_or_else(|| cb::not_impl(scope, "backpackSnapshots"))?;
            backpack_snapshot_array(scope, rows)
        }
    })
}

fn inventory_rows<'s>(
    scope: &mut v8::HandleScope<'s>,
) -> cb::JsResult<'s, v8::Local<'s, v8::Value>> {
    observed::with(|scene| {
        let rows = scene
            .since_login()
            .inv()
            .map(Vec::as_slice)
            .unwrap_or_default();
        rows_array(scope, rows)
    })
}

fn equipment_rows<'s>(
    scope: &mut v8::HandleScope<'s>,
) -> cb::JsResult<'s, v8::Local<'s, v8::Value>> {
    observed::with(|scene| {
        let rows = scene
            .since_login()
            .equipment()
            .map(Vec::as_slice)
            .unwrap_or_default();
        rows_array(scope, rows)
    })
}

fn backpack_capacity<'s>(
    scope: &mut v8::HandleScope<'s>,
    host: v8::Local<'s, v8::Value>,
) -> cb::JsResult<'s, v8::Local<'s, v8::Value>> {
    let bank_open = observed::with(|scene| scene.since_login().bank_open().unwrap_or_default());
    if bank_open {
        return Ok(cb::num(scope, 28.0));
    }
    let host_size = cb::get(scope, host, "invSize")?;
    if host_size.is_number() {
        return Ok(host_size);
    }
    observed::with(|scene| {
        let size = scene.since_login().inv_size().unwrap_or_default();
        Ok(cb::num(scope, f64::from(size)))
    })
}

fn inventory_capacity<'s>(
    scope: &mut v8::HandleScope<'s>,
    host: v8::Local<'s, v8::Value>,
) -> cb::JsResult<'s, v8::Local<'s, v8::Value>> {
    let host_size = cb::get(scope, host, "invSize")?;
    if host_size.is_number() {
        return Ok(host_size);
    }
    observed::with(|scene| {
        let view = scene.since_login();
        Ok(cb::num(
            scope,
            f64::from(view.inv_size().unwrap_or_default()),
        ))
    })
}

fn array_filter<'s>(
    scope: &mut v8::HandleScope<'s>,
    items: v8::Local<'s, v8::Value>,
    callback: v8::Local<'s, v8::Value>,
    label: &str,
) -> cb::JsResult<'s, v8::Local<'s, v8::Value>> {
    let filter = cb::get(scope, items, "filter")?;
    cb::call(scope, filter, items, &[callback], label)
}

fn array_map<'s>(
    scope: &mut v8::HandleScope<'s>,
    items: v8::Local<'s, v8::Value>,
    callback: v8::Local<'s, v8::Value>,
    label: &str,
) -> cb::JsResult<'s, v8::Local<'s, v8::Value>> {
    let map = cb::get(scope, items, "map")?;
    cb::call(scope, map, items, &[callback], label)
}

fn inventory_items<'s>(
    scope: &mut v8::HandleScope<'s>,
    constructor: v8::Local<'s, v8::Value>,
) -> cb::JsResult<'s, v8::Local<'s, v8::Value>> {
    let rows = inventory_rows(scope)?;
    let predicate = callback_without_data(scope, inventory_named_row, 1, "Inventory.items filter")?;
    let filtered = array_filter(scope, rows, predicate, "filter")?;
    let mapper = callback_function(scope, construct_inventory_item, constructor, 1, "InvItem")?;
    array_map(scope, filtered, mapper, "map")
}

fn inventory_named_row<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue,
) {
    let result = (|| {
        let row = args.get(0);
        if !cb::truthy(scope, row) {
            return Ok(row);
        }
        cb::get(scope, row, "name")
    })();
    cb::finish(scope, rv, result);
}

fn construct_inventory_item<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue,
) {
    let constructor = args.data();
    let result = cb::construct(scope, constructor, &[args.get(0)], "InvItem");
    cb::finish(scope, rv, result);
}

fn inventory_first<'s>(
    scope: &mut v8::HandleScope<'s>,
    name: v8::Local<'s, v8::Value>,
    constructor: v8::Local<'s, v8::Value>,
) -> cb::JsResult<'s, v8::Local<'s, v8::Value>> {
    let wanted = lower_string(scope, name)?;
    let rows = inventory_rows(scope)?;
    let predicate = callback_function(
        scope,
        inventory_name_matches,
        wanted,
        1,
        "Inventory.first find",
    )?;
    let find = cb::get(scope, rows, "find")?;
    let row = cb::call(scope, find, rows, &[predicate], "find")?;
    if !cb::truthy(scope, row) {
        return Ok(v8::null(scope).into());
    }
    cb::construct(scope, constructor, &[row], "InvItem")
}

fn inventory_name_matches<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue,
) {
    let result = (|| {
        let row = args.get(0);
        if !cb::truthy(scope, row) {
            return Ok(row);
        }
        let name = cb::get(scope, row, "name")?;
        if !name.is_string() {
            return Ok(v8::Boolean::new(scope, false).into());
        }
        let lowered = cb::call_method(scope, name, "toLowerCase", &[], "toLowerCase")?;
        Ok(v8::Boolean::new(scope, lowered.strict_equals(args.data())).into())
    })();
    cb::finish(scope, rv, result);
}

fn inventory_used<'s>(
    scope: &mut v8::HandleScope<'s>,
) -> cb::JsResult<'s, v8::Local<'s, v8::Value>> {
    let rows = inventory_rows(scope)?;
    let predicate = callback_without_data(scope, inventory_named_row, 1, "Inventory.used filter")?;
    let filtered = array_filter(scope, rows, predicate, "filter")?;
    cb::get(scope, filtered, "length")
}

fn inventory_count<'s>(
    scope: &mut v8::HandleScope<'s>,
    name: v8::Local<'s, v8::Value>,
) -> cb::JsResult<'s, v8::Local<'s, v8::Value>> {
    let wanted = lower_string(scope, name)?;
    let rows = inventory_rows(scope)?;
    let predicate = callback_function(
        scope,
        inventory_name_matches,
        wanted,
        1,
        "Inventory.count filter",
    )?;
    let filtered = array_filter(scope, rows, predicate, "filter")?;
    reduce_item_counts(scope, filtered)
}

fn inventory_count_by_id<'s>(
    scope: &mut v8::HandleScope<'s>,
    id: v8::Local<'s, v8::Value>,
) -> cb::JsResult<'s, v8::Local<'s, v8::Value>> {
    let rows = inventory_rows(scope)?;
    let length = cb::get(scope, rows, "length")?;
    if cb::number(scope, length)? == 0.0 {
        return Ok(cb::num(scope, 0.0));
    }
    let has_id = callback_without_data(
        scope,
        inventory_has_nonzero_id,
        1,
        "Inventory.countById some",
    )?;
    let some = cb::get(scope, rows, "some")?;
    let any_id = cb::call(scope, some, rows, &[has_id], "some")?;
    if !cb::truthy(scope, any_id) {
        return Err(cb::not_impl(scope, "Inventory.countById"));
    }
    let id = global_call(scope, "Number", &[id])?;
    let predicate = callback_function(
        scope,
        inventory_id_matches,
        id,
        1,
        "Inventory.countById filter",
    )?;
    let filtered = array_filter(scope, rows, predicate, "filter")?;
    reduce_item_counts(scope, filtered)
}

fn inventory_has_nonzero_id<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue,
) {
    let result = (|| {
        let row = args.get(0);
        if !cb::truthy(scope, row) {
            return Ok(row);
        }
        let id = cb::get(scope, row, "id")?;
        let nonzero = id.is_number() && id.number_value(scope) != Some(0.0);
        Ok(v8::Boolean::new(scope, nonzero).into())
    })();
    cb::finish(scope, rv, result);
}

fn inventory_id_matches<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue,
) {
    let result = (|| {
        let row = args.get(0);
        if !cb::truthy(scope, row) {
            return Ok(row);
        }
        let id = cb::get(scope, row, "id")?;
        Ok(v8::Boolean::new(scope, id.strict_equals(args.data())).into())
    })();
    cb::finish(scope, rv, result);
}

fn reduce_item_counts<'s>(
    scope: &mut v8::HandleScope<'s>,
    rows: v8::Local<'s, v8::Value>,
) -> cb::JsResult<'s, v8::Local<'s, v8::Value>> {
    let reduce = cb::get(scope, rows, "reduce")?;
    let callback = callback_without_data(scope, add_item_count, 2, "Inventory count reduce")?;
    let zero = cb::num(scope, 0.0);
    cb::call(scope, reduce, rows, &[callback, zero], "reduce")
}

fn add_item_count<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue,
) {
    let result = (|| {
        let count = cb::get(scope, args.get(1), "count")?;
        let addend = if count.is_number() {
            count
        } else {
            cb::num(scope, 0.0)
        };
        cb::add(scope, args.get(0), addend)
    })();
    cb::finish(scope, rv, result);
}

fn inventory_is_full<'s>(
    scope: &mut v8::HandleScope<'s>,
    inventory: v8::Local<'s, v8::Value>,
    host: v8::Local<'s, v8::Value>,
) -> cb::JsResult<'s, v8::Local<'s, v8::Value>> {
    let size = inventory_capacity(scope, host)?;
    let zero = cb::num(scope, 0.0);
    if !cb::gt(scope, size, zero)? {
        return Ok(v8::Boolean::new(scope, false).into());
    }
    let used = inventory_used_method(scope, inventory)?;
    let is_full = cb::ge(scope, used, size)?;
    Ok(v8::Boolean::new(scope, is_full).into())
}

fn inventory_free<'s>(
    scope: &mut v8::HandleScope<'s>,
    inventory: v8::Local<'s, v8::Value>,
    host: v8::Local<'s, v8::Value>,
) -> cb::JsResult<'s, v8::Local<'s, v8::Value>> {
    let size = inventory_capacity(scope, host)?;
    let zero = cb::num(scope, 0.0);
    if !cb::gt(scope, size, zero)? {
        return Ok(zero);
    }
    let global = scope.get_current_context().global(scope);
    let math = cb::get(scope, global.into(), "Math")?;
    let max = cb::get(scope, math, "max")?;
    let used = inventory_used_method(scope, inventory)?;
    let remaining = cb::sub(scope, size, used)?;
    let zero = cb::num(scope, 0.0);
    cb::call(scope, max, math, &[zero, remaining], "Math.max")
}

fn inventory_used_method<'s>(
    scope: &mut v8::HandleScope<'s>,
    inventory: v8::Local<'s, v8::Value>,
) -> cb::JsResult<'s, v8::Local<'s, v8::Value>> {
    let used = cb::get(scope, inventory, "used")?;
    cb::call(scope, used, inventory, &[], "Inventory.used")
}

fn inventory_interact<'s>(
    scope: &mut v8::HandleScope<'s>,
    row: v8::Local<'s, v8::Value>,
    action: v8::Local<'s, v8::Value>,
) -> cb::JsResult<'s, v8::Local<'s, v8::Value>> {
    let name = cb::get(scope, row, "name")?;
    let action = global_string(scope, action)?;
    let op = cb::string(scope, "held");
    cb::object(
        scope,
        &[("op", op.into()), ("name", name), ("action", action)],
    )
}

fn nullish_or<'s>(
    value: v8::Local<'s, v8::Value>,
    fallback: v8::Local<'s, v8::Value>,
) -> v8::Local<'s, v8::Value> {
    if value.is_null_or_undefined() {
        fallback
    } else {
        value
    }
}

fn symbol_property<'s>(
    scope: &mut v8::HandleScope<'s>,
    target: v8::Local<'s, v8::Value>,
    key: v8::Local<'s, v8::Value>,
) -> cb::JsResult<'s, v8::Local<'s, v8::Value>> {
    if target.is_null_or_undefined() {
        return Err(cb::type_error(
            scope,
            "Cannot read properties of nullish value",
        ));
    }
    let object = cb::catching(scope, |s| target.to_object(s))?;
    cb::catching(scope, |s| object.get(s, key))
}

fn optional_identity_field<'s>(
    scope: &mut v8::HandleScope<'s>,
    identity: v8::Local<'s, v8::Value>,
    field: &str,
) -> cb::JsResult<'s, v8::Local<'s, v8::Value>> {
    if identity.is_null_or_undefined() {
        return Ok(v8::null(scope).into());
    }
    let value = cb::get(scope, identity, field)?;
    Ok(nullish_or(value, v8::null(scope).into()))
}

fn optional_property<'s>(
    scope: &mut v8::HandleScope<'s>,
    target: v8::Local<'s, v8::Value>,
    key: &str,
) -> cb::JsResult<'s, v8::Local<'s, v8::Value>> {
    if target.is_null_or_undefined() {
        return Ok(v8::undefined(scope).into());
    }
    cb::get(scope, target, key)
}

fn inventory_use_on<'s>(
    scope: &mut v8::HandleScope<'s>,
    source: v8::Local<'s, v8::Value>,
    source_row: v8::Local<'s, v8::Value>,
    target: v8::Local<'s, v8::Value>,
    identity_key: v8::Local<'s, v8::Value>,
) -> cb::JsResult<'s, v8::Local<'s, v8::Value>> {
    if !cb::truthy(scope, target) {
        return Ok(v8::Boolean::new(scope, false).into());
    }
    let first_name = cb::get(scope, target, "name")?;
    if cb::truthy(scope, first_name) {
        let first_snap = cb::get(scope, target, "snap")?;
        if !cb::truthy(scope, first_snap) {
            let target_identity = symbol_property(scope, target, identity_key)?;
            let name = cb::get(scope, source_row, "name")?;
            let target_name = cb::get(scope, target, "name")?;
            let source_identity = symbol_property(scope, source, identity_key)?;
            let source_id = optional_identity_field(scope, source_identity, "id")?;
            let source_slot = optional_identity_field(scope, source_identity, "slot")?;
            let target_id = optional_identity_field(scope, target_identity, "id")?;
            let target_slot = optional_identity_field(scope, target_identity, "slot")?;
            let op = cb::string(scope, "use-on").into();
            let kind = cb::string(scope, "inv").into();
            let zero = cb::num(scope, 0.0);
            let index = v8::null(scope).into();
            return cb::object(
                scope,
                &[
                    ("op", op),
                    ("name", name),
                    ("kind", kind),
                    ("target_name", target_name),
                    ("source_item_id", source_id),
                    ("source_item_slot", source_slot),
                    ("target_item_id", target_id),
                    ("target_item_slot", target_slot),
                    ("x", zero),
                    ("z", zero),
                    ("level", zero),
                    ("index", index),
                ],
            );
        }
    }

    let second_name = cb::get(scope, target, "name")?;
    if !cb::truthy(scope, second_name) {
        return Ok(v8::Boolean::new(scope, false).into());
    }
    let second_snap = cb::get(scope, target, "snap")?;
    if !cb::truthy(scope, second_snap) {
        return Ok(v8::Boolean::new(scope, false).into());
    }

    let constructor = cb::get(scope, target, "constructor")?;
    let constructor_name = optional_property(scope, constructor, "name")?;
    let npc = cb::string(scope, "Npc");
    let loc = cb::string(scope, "Loc");
    let ground_item = cb::string(scope, "GroundItem");
    let player = cb::string(scope, "Player");
    let kind = if constructor_name.strict_equals(npc) {
        "npc"
    } else if constructor_name.strict_equals(loc) {
        "loc"
    } else if constructor_name.strict_equals(ground_item) {
        "obj"
    } else if constructor_name.strict_equals(player) {
        "player"
    } else {
        return Err(cb::not_impl(scope, "Inventory.useOn"));
    };

    let source_name = cb::get(scope, source_row, "name")?;
    let target_name_value = cb::get(scope, target, "name")?;
    let target_name = if target_name_value.is_null_or_undefined() {
        let snap = cb::get(scope, target, "snap")?;
        let snap_name = optional_property(scope, snap, "name")?;
        nullish_or(snap_name, v8::null(scope).into())
    } else {
        target_name_value
    };
    let source_identity = symbol_property(scope, source, identity_key)?;
    let source_id = optional_identity_field(scope, source_identity, "id")?;
    let source_slot = optional_identity_field(scope, source_identity, "slot")?;
    let zero = cb::num(scope, 0.0);
    let snap_x = cb::get(scope, target, "snap")?;
    let x_value = optional_property(scope, snap_x, "x")?;
    let x = nullish_or(x_value, zero);
    let snap_z = cb::get(scope, target, "snap")?;
    let z_value = optional_property(scope, snap_z, "z")?;
    let z = nullish_or(z_value, zero);
    let snap_level = cb::get(scope, target, "snap")?;
    let level_value = optional_property(scope, snap_level, "level")?;
    let level = nullish_or(level_value, zero);
    let index = if kind == "npc" {
        let snap_index = cb::get(scope, target, "snap")?;
        let index = optional_property(scope, snap_index, "index")?;
        if index.is_null_or_undefined() {
            let target_index = cb::get(scope, target, "index")?;
            nullish_or(target_index, v8::null(scope).into())
        } else {
            index
        }
    } else {
        v8::null(scope).into()
    };

    let op = cb::string(scope, "use-on").into();
    let kind = cb::string(scope, kind).into();
    cb::object(
        scope,
        &[
            ("op", op),
            ("name", source_name),
            ("kind", kind),
            ("target_name", target_name),
            ("source_item_id", source_id),
            ("source_item_slot", source_slot),
            ("x", x),
            ("z", z),
            ("level", level),
            ("index", index),
        ],
    )
}

fn equipment_items<'s>(
    scope: &mut v8::HandleScope<'s>,
) -> cb::JsResult<'s, v8::Local<'s, v8::Value>> {
    let rows = equipment_rows(scope)?;
    let predicate = callback_without_data(scope, inventory_named_row, 1, "Equipment.items filter")?;
    let filtered = array_filter(scope, rows, predicate, "filter")?;
    let mapper = callback_without_data(scope, equipment_item, 1, "Equipment.items map")?;
    array_map(scope, filtered, mapper, "map")
}

fn equipment_item<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue,
) {
    let result = (|| {
        let row = args.get(0);
        let name = cb::get(scope, row, "name")?;
        let count = cb::get(scope, row, "count")?;
        let count = if count.is_number() {
            count
        } else {
            cb::num(scope, 1.0)
        };
        let id = cb::get(scope, row, "id")?;
        let id = if id.is_number() {
            id
        } else {
            cb::num(scope, 0.0)
        };
        cb::object(scope, &[("name", name), ("count", count), ("id", id)])
    })();
    cb::finish(scope, rv, result);
}

fn equipment_contains<'s>(
    scope: &mut v8::HandleScope<'s>,
    name: v8::Local<'s, v8::Value>,
) -> cb::JsResult<'s, v8::Local<'s, v8::Value>> {
    let wanted = lower_string(scope, name)?;
    let rows = equipment_rows(scope)?;
    let predicate = callback_function(
        scope,
        equipment_name_matches,
        wanted,
        1,
        "Equipment.contains some",
    )?;
    let some = cb::get(scope, rows, "some")?;
    cb::call(scope, some, rows, &[predicate], "some")
}

fn equipment_name_matches<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue,
) {
    let result = (|| {
        let row = args.get(0);
        if !cb::truthy(scope, row) {
            return Ok(row);
        }
        let name = cb::get(scope, row, "name")?;
        if !name.is_string() {
            return Ok(v8::Boolean::new(scope, false).into());
        }
        let lowered = cb::call_method(scope, name, "toLowerCase", &[], "toLowerCase")?;
        Ok(v8::Boolean::new(scope, lowered.strict_equals(args.data())).into())
    })();
    cb::finish(scope, rv, result);
}

fn equipment_equip<'s>(
    scope: &mut v8::HandleScope<'s>,
    name: v8::Local<'s, v8::Value>,
    equipment: v8::Local<'s, v8::Value>,
    inventory: v8::Local<'s, v8::Value>,
) -> cb::JsResult<'s, v8::Local<'s, v8::Value>> {
    let contains = cb::get(scope, equipment, "contains")?;
    let equipped = cb::call(scope, contains, equipment, &[name], "Equipment.contains")?;
    if cb::truthy(scope, equipped) {
        return Ok(v8::Boolean::new(scope, true).into());
    }
    let first = cb::get(scope, inventory, "first")?;
    let held = cb::call(scope, first, inventory, &[name], "Inventory.first")?;
    if !cb::truthy(scope, held) {
        return Ok(v8::Boolean::new(scope, false).into());
    }
    let name = global_string(scope, name)?;
    let op = cb::string(scope, "wear").into();
    cb::object(scope, &[("op", op), ("name", name)])
}
fn equipment_unequip<'s>(
    scope: &mut v8::HandleScope<'s>,
    name: v8::Local<'s, v8::Value>,
    equipment: v8::Local<'s, v8::Value>,
) -> cb::JsResult<'s, v8::Local<'s, v8::Value>> {
    let contains = cb::get(scope, equipment, "contains")?;
    let equipped = cb::call(scope, contains, equipment, &[name], "Equipment.contains")?;
    if !cb::truthy(scope, equipped) {
        return Ok(v8::Boolean::new(scope, true).into());
    }
    let name = global_string(scope, name)?;
    let op = cb::string(scope, "unequip").into();
    cb::object(scope, &[("op", op), ("name", name)])
}

fn matches_any<'s>(
    scope: &mut v8::HandleScope<'s>,
    name: v8::Local<'s, v8::Value>,
    patterns: v8::Local<'s, v8::Value>,
) -> cb::JsResult<'s, v8::Local<'s, v8::Value>> {
    if name.is_null_or_undefined() {
        return Ok(v8::Boolean::new(scope, false).into());
    }
    let name = lower_string(scope, name)?;
    let patterns = if cb::truthy(scope, patterns) {
        patterns
    } else {
        v8::Array::new(scope, 0).into()
    };
    let some = cb::get(scope, patterns, "some")?;
    let predicate = callback_function(scope, pattern_matches, name, 1, "matchesAny some")?;
    cb::call(scope, some, patterns, &[predicate], "some")
}

fn pattern_matches<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue,
) {
    let result = (|| {
        let pattern = global_string(scope, args.get(0))?;
        let trimmed = cb::call_method(scope, pattern, "trim", &[], "trim")?;
        let lowered = cb::call_method(scope, trimmed, "toLowerCase", &[], "toLowerCase")?;
        let length = cb::get(scope, lowered, "length")?;
        let zero = cb::num(scope, 0.0);
        if !cb::gt(scope, length, zero)? {
            return Ok(v8::Boolean::new(scope, false).into());
        }
        let matched = cb::call_method(scope, args.data(), "includes", &[lowered], "includes")?;
        Ok(matched)
    })();
    cb::finish(scope, rv, result);
}

fn pack_count_filter<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue,
) {
    let result = (|| {
        let item = args.get(0);
        if !cb::truthy(scope, item) {
            return Ok(item);
        }
        let name = cb::get(scope, item, "name")?;
        matches_any(scope, name, args.data())
    })();
    cb::finish(scope, rv, result);
}

fn pack_count_reduce<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue,
) {
    let result = (|| {
        let count = cb::get(scope, args.get(1), "count")?;
        let addend = if count.is_number() {
            count
        } else {
            cb::num(scope, 0.0)
        };
        cb::add(scope, args.get(0), addend)
    })();
    cb::finish(scope, rv, result);
}

fn or_empty_array<'s>(
    scope: &mut v8::HandleScope<'s>,
    value: v8::Local<'s, v8::Value>,
) -> v8::Local<'s, v8::Value> {
    if cb::truthy(scope, value) {
        value
    } else {
        v8::Array::new(scope, 0).into()
    }
}

fn matching_filter<'s>(
    scope: &mut v8::HandleScope<'s>,
    items: v8::Local<'s, v8::Value>,
    patterns: v8::Local<'s, v8::Value>,
) -> cb::JsResult<'s, v8::Local<'s, v8::Value>> {
    let items = or_empty_array(scope, items);
    let filter = cb::get(scope, items, "filter")?;
    let predicate = callback_function(scope, pack_count_filter, patterns, 1, "pack rule filter")?;
    cb::call(scope, filter, items, &[predicate], "filter")
}

fn count_matching<'s>(
    scope: &mut v8::HandleScope<'s>,
    items: v8::Local<'s, v8::Value>,
    patterns: v8::Local<'s, v8::Value>,
) -> cb::JsResult<'s, v8::Local<'s, v8::Value>> {
    let filtered = matching_filter(scope, items, patterns)?;
    let reduce = cb::get(scope, filtered, "reduce")?;
    let reducer = callback_without_data(scope, pack_count_reduce, 2, "pack rule reduce")?;
    let zero = cb::num(scope, 0.0);
    cb::call(scope, reduce, filtered, &[reducer, zero], "reduce")
}

fn slots_matching<'s>(
    scope: &mut v8::HandleScope<'s>,
    items: v8::Local<'s, v8::Value>,
    patterns: v8::Local<'s, v8::Value>,
) -> cb::JsResult<'s, v8::Local<'s, v8::Value>> {
    let filtered = matching_filter(scope, items, patterns)?;
    cb::get(scope, filtered, "length")
}

fn should_bank<'s>(
    scope: &mut v8::HandleScope<'s>,
    loot_slots: v8::Local<'s, v8::Value>,
    bank_at: v8::Local<'s, v8::Value>,
    inventory_full: v8::Local<'s, v8::Value>,
) -> cb::JsResult<'s, v8::Local<'s, v8::Value>> {
    if cb::ge(scope, loot_slots, bank_at)? {
        return Ok(v8::Boolean::new(scope, true).into());
    }
    if !cb::truthy(scope, inventory_full) {
        return Ok(inventory_full);
    }
    let zero = cb::num(scope, 0.0);
    let has_loot = cb::gt(scope, loot_slots, zero)?;
    Ok(v8::Boolean::new(scope, has_loot).into())
}

#[cfg(test)]
mod allocation_tests {
    use super::*;
    use crate::observed::{ItemRow, Ops, Text};
    use rustyscript::{Runtime, RuntimeOptions};

    const INVENTORY_SETUP: &str = r#"
const identity = Symbol('inventoryIdentity');
const sourceRow = Symbol('inventoryRow');
globalThis.__InventoryTestItem = class {
    constructor(row) {
        globalThis.__rs2b0t_inventory(15, this, row, identity, sourceRow);
    }
    actions() {
        return globalThis.__rs2b0t_inventory(16, this[sourceRow]);
    }
};
globalThis.checkInventory = () => {
    const call = globalThis.__rs2b0t_inventory;
    const rule = globalThis.__rs2b0t_pack_rules;
    const snapshots = call(0);
    const items = call(2, globalThis.__InventoryTestItem);
    const first = call(3, 'apple', globalThis.__InventoryTestItem);
    const used = call(4);
    const count = call(5, 'apple');
    const countById = call(6, 10);
    const host = { invSize: 28 };
    const capacity = call(1, host);
    const inventory = { used() { return used; }, first() { return first; } };
    const full = call(7, inventory, host);
    const free = call(8, inventory, host);
    const equipment = call(11);
    const equipped = call(12, 'apple');
    const contains = call(17, inventory, 'apple');
    const actions = first.actions();
    const ruleItems = [{name: 'Apple', count: 2}];
    const rules = rule(1, ruleItems, ['apple']);
    const slots = rule(2, ruleItems, ['apple']);
    const bank = rule(3, 1, 2, true);
    const restock = rule(4, 1, 2);
    const panic = rule(5, 0.1, 0.2, 0);
    const matches = rule(0, 'Apple', [' apple ']);
    return snapshots.length === 1 && items.length === 1 && first.name === 'Apple'
        && used === 1 && count === 2 && countById === 2 && capacity === 28
        && !full && free === 27 && equipment.length === 1 && equipped && contains
        && actions.length === 1 && actions[0] === 'Eat' && first.actions() !== actions
        && rules === 2 && slots === 1 && bank && restock && panic && matches;
};
"#;

    fn setup() -> Runtime {
        crate::load::platform::bootstrap_test_v8_platform();
        let row = ItemRow {
            id: 10,
            count: 2,
            name: Some(Text::from("Apple")),
            ops: Ops::from(vec![Text::from("Eat")]),
            noted: false,
            cert: -1,
            component_id: Some(503),
            slot: Some(0),
        };
        observed::replace(1, true, |post| {
            post.inv(vec![row.clone()])
                .inv_size(28)
                .bank_open(false)
                .bank_side(vec![row.clone()])
                .equipment(vec![row.clone()])
                .side_modal_id(700);
        });
        let mut runtime = Runtime::new(RuntimeOptions::default()).unwrap();
        cb::install_ops(&mut runtime).unwrap();
        install(&mut runtime).unwrap();
        runtime.eval::<()>(INVENTORY_SETUP).unwrap();
        runtime
    }

    #[test]
    fn first_bridge_read_is_attributed_and_warmed_local_batches_allocate_nothing() {
        let mut runtime = setup();
        let context = runtime.deno_runtime().main_context();
        let mut scope = runtime.deno_runtime().handle_scope();
        let global = context.open(&mut scope).global(&mut scope);
        let key = cb::string(&mut scope, "checkInventory");
        let function =
            v8::Local::<v8::Function>::try_from(global.get(&mut scope, key).unwrap()).unwrap();
        let receiver = v8::undefined(&mut scope).into();
        let cold = allocation_counter::measure(|| {
            assert!(function.call(&mut scope, receiver, &[]).unwrap().is_true());
        });
        let warm = allocation_counter::measure(|| {
            for _ in 0..1_000 {
                assert!(function.call(&mut scope, receiver, &[]).unwrap().is_true());
            }
        });
        println!("Inventory local-read callbacks cold={cold:?}; warm1000={warm:?}");
        // The raw five-scope control below attributes these cache cells without
        // calling Inventory or pack-rule policy.
        assert!(cold.count_total <= 3);
        assert!(cold.count_current <= 3);
        assert!(cold.count_max <= 3);
        assert!(cold.bytes_total <= 336);
        assert!(cold.bytes_current <= 336);
        assert!(cold.bytes_max <= 336);
        assert_eq!(warm, allocation_counter::AllocationInfo::default());
    }

    fn scope_cache_control<'s>(
        scope: &mut v8::HandleScope<'s>,
        args: v8::FunctionCallbackArguments<'s>,
        mut rv: v8::ReturnValue,
    ) {
        let first = &mut v8::TryCatch::new(scope);
        let second = &mut v8::TryCatch::new(first);
        let third = &mut v8::TryCatch::new(second);
        let fourth = &mut v8::TryCatch::new(third);
        let fifth = &mut v8::TryCatch::new(fourth);
        if args.get(0).is_true() {
            let sixth = &mut v8::TryCatch::new(fifth);
            rv.set(v8::Boolean::new(sixth, true).into());
        } else {
            rv.set(v8::Boolean::new(fifth, true).into());
        }
    }

    #[test]
    fn inventory_cold_charge_reuses_independent_scope_cache_cells() {
        let mut runtime = setup();
        cb::install(&mut runtime, "__inventoryScopeControl", scope_cache_control).unwrap();
        runtime
            .eval::<()>(
                "globalThis.checkInventoryPure = () => true; \
                 globalThis.checkInventoryScopes = () => __inventoryScopeControl(false); \
                 globalThis.checkInventoryExtraScope = () => __inventoryScopeControl(true);",
            )
            .unwrap();
        let context = runtime.deno_runtime().main_context();
        let mut scope = runtime.deno_runtime().handle_scope();
        let global = context.open(&mut scope).global(&mut scope);
        let key = cb::string(&mut scope, "checkInventoryPure");
        let pure_js =
            v8::Local::<v8::Function>::try_from(global.get(&mut scope, key).unwrap()).unwrap();
        let key = cb::string(&mut scope, "checkInventoryScopes");
        let five_scopes =
            v8::Local::<v8::Function>::try_from(global.get(&mut scope, key).unwrap()).unwrap();
        let key = cb::string(&mut scope, "checkInventoryExtraScope");
        let six_scopes =
            v8::Local::<v8::Function>::try_from(global.get(&mut scope, key).unwrap()).unwrap();
        let key = cb::string(&mut scope, "checkInventory");
        let inventory =
            v8::Local::<v8::Function>::try_from(global.get(&mut scope, key).unwrap()).unwrap();
        let receiver = v8::undefined(&mut scope).into();
        let pure = allocation_counter::measure(|| {
            assert!(pure_js.call(&mut scope, receiver, &[]).unwrap().is_true());
        });
        let cold = allocation_counter::measure(|| {
            assert!(five_scopes
                .call(&mut scope, receiver, &[])
                .unwrap()
                .is_true());
        });
        let inventory_after_control = allocation_counter::measure(|| {
            assert!(inventory.call(&mut scope, receiver, &[]).unwrap().is_true());
        });
        let extra_depth = allocation_counter::measure(|| {
            assert!(six_scopes
                .call(&mut scope, receiver, &[])
                .unwrap()
                .is_true());
        });
        let warm = allocation_counter::measure(|| {
            for _ in 0..1_000 {
                assert!(six_scopes
                    .call(&mut scope, receiver, &[])
                    .unwrap()
                    .is_true());
                assert!(inventory.call(&mut scope, receiver, &[]).unwrap().is_true());
            }
        });
        println!(
            "Inventory scope-cache control pure={pure:?}; five scopes={cold:?}; \
             inventory after control={inventory_after_control:?}; \
             extra scope={extra_depth:?}; warm1000={warm:?}"
        );
        assert_eq!(pure, allocation_counter::AllocationInfo::default());
        assert_eq!(
            cold,
            allocation_counter::AllocationInfo {
                count_total: 3,
                count_current: 3,
                count_max: 3,
                bytes_total: 336,
                bytes_current: 336,
                bytes_max: 336,
            }
        );
        assert_eq!(
            inventory_after_control,
            allocation_counter::AllocationInfo::default()
        );
        assert_eq!(
            extra_depth,
            allocation_counter::AllocationInfo {
                count_total: 1,
                count_current: 1,
                count_max: 1,
                bytes_total: 112,
                bytes_current: 112,
                bytes_max: 112,
            }
        );
        assert_eq!(warm, allocation_counter::AllocationInfo::default());
    }
}
