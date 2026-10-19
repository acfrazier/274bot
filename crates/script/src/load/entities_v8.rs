//! Native V8 ownership for entity wrappers and `EntityQuery`.
//!
//! The shim keeps the public classes, aliases, proxies, raw `snap` fields,
//! supply functions, and wrapper constructors. This module owns their local
//! field policy and the snapshot-first query walk. It does not retain rows or
//! dispatch actions outside the existing queue and selected-facts helpers.

use super::callback_v8::{self as cb, Flow, ForOf, JsResult, Poll};
use rustyscript::Runtime;

pub(super) fn install(runtime: &mut Runtime) -> Result<(), String> {
    cb::install(runtime, "__rs2b0t_entities_v8", entities_callback)
}

fn entities_callback<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue,
) {
    let result = dispatch(scope, &args);
    cb::finish(scope, rv, result);
}

fn dispatch<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: &v8::FunctionCallbackArguments<'s>,
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    let op = args.get(0);
    if is_op(scope, op, "entitySnapView") {
        return entity_snap_view(scope, args.get(1));
    }
    if is_op(scope, op, "presentOps") {
        return present_ops(scope, args.get(1));
    }
    if is_op(scope, op, "opIndex") {
        let index = op_index_value(scope, args.get(1), args.get(2))?;
        return Ok(cb::num(scope, index));
    }
    if is_op(scope, op, "matchesEntityName") {
        return matches_entity_name(scope, args.get(1), args.get(2));
    }
    if is_op(scope, op, "talkOp") {
        return talk_op(scope, args.get(1));
    }
    if is_op(scope, op, "entities.all") {
        return entities_all(scope, args.get(1), args.get(2), args.get(3));
    }
    if is_op(scope, op, "npcs.nearest") {
        return npcs_nearest(scope, args.get(1), args.get(2));
    }
    if is_op(scope, op, "query.name") {
        return query_name(scope, args.get(1), args.get(2));
    }
    if is_op(scope, op, "query.action") {
        return query_action(scope, args.get(1), args.get(2));
    }
    if is_op(scope, op, "query.within") {
        return query_within(scope, args.get(1), args.get(2));
    }
    if is_op(scope, op, "query.withinOf") {
        return query_within_of(scope, args.get(1), args.get(2), args.get(3));
    }
    if is_op(scope, op, "query.inside") {
        return query_inside(scope, args.get(1), args.get(2));
    }
    if is_op(scope, op, "query.where") {
        return push_filter(scope, args.get(1), "entityFilters", args.get(2));
    }
    if is_op(scope, op, "query.forEachMatch") {
        return query_for_each_match(scope, args.get(1), args.get(2));
    }
    if is_op(scope, op, "query.results") {
        return query_results(scope, args.get(1));
    }
    if is_op(scope, op, "query.nearest") {
        return query_nearest(scope, args.get(1));
    }
    if is_op(scope, op, "query.first") {
        return query_first(scope, args.get(1));
    }
    if is_op(scope, op, "query.exists") {
        return query_exists(scope, args.get(1));
    }
    if is_op(scope, op, "query.count") {
        return query_count(scope, args.get(1));
    }
    if is_op(scope, op, "query.nearestPreferLocal") {
        return query_nearest_prefer_local(scope, args.get(1), args.get(2));
    }

    if is_op(scope, op, "npc.name") {
        return entity_name(scope, args.get(1));
    }
    if is_op(scope, op, "npc.id") {
        return entity_snap_field(scope, args.get(1), "id");
    }
    if is_op(scope, op, "npc.index") {
        return entity_snap_field(scope, args.get(1), "index");
    }
    if is_op(scope, op, "npc.inCombat") {
        return entity_boolean_field(scope, args.get(1), "in_combat");
    }
    if is_op(scope, op, "npc.health") {
        return entity_snap_field(scope, args.get(1), "health");
    }
    if is_op(scope, op, "npc.level") {
        return npc_level(scope, args.get(1));
    }
    if is_op(scope, op, "npc.size") {
        return npc_size(scope, args.get(1));
    }
    if is_op(scope, op, "npc.networkOrigin") {
        return npc_network_origin(scope, args.get(1), args.get(2));
    }
    if is_op(scope, op, "npc.networkTile") {
        return npc_network_tile(scope, args.get(1), args.get(2));
    }
    if is_op(scope, op, "npc.targetsMe") {
        return entity_targets_me(scope, args.get(1), args.get(2));
    }
    if is_op(scope, op, "npc.targetsAnotherPlayer") {
        return npc_targets_another_player(scope, args.get(1), args.get(2));
    }
    if is_op(scope, op, "npc.tile") {
        return entity_tile(scope, args.get(1), args.get(2));
    }
    if is_op(scope, op, "npc.distance") {
        return entity_distance(scope, args.get(1));
    }
    if is_op(scope, op, "npc.actions") {
        return entity_actions(scope, args.get(1));
    }
    if is_op(scope, op, "npc.valid") {
        return npc_valid(scope, args.get(1), args.get(2));
    }
    if is_op(scope, op, "npc.interact") {
        return npc_interact(scope, args.get(1), args.get(2), args.get(3));
    }

    if is_op(scope, op, "loc.name") {
        return entity_name(scope, args.get(1));
    }
    if is_op(scope, op, "loc.id") {
        return entity_snap_field(scope, args.get(1), "id");
    }
    if is_op(scope, op, "loc.tile") {
        return entity_tile(scope, args.get(1), args.get(2));
    }
    if is_op(scope, op, "loc.distance") {
        return entity_distance(scope, args.get(1));
    }
    if is_op(scope, op, "loc.actions") {
        return entity_actions(scope, args.get(1));
    }
    if is_op(scope, op, "loc.interact") {
        return loc_interact(scope, args.get(1), args.get(2), args.get(3));
    }

    if is_op(scope, op, "player.name") {
        return entity_name(scope, args.get(1));
    }
    if is_op(scope, op, "player.index") {
        return entity_snap_field(scope, args.get(1), "index");
    }
    if is_op(scope, op, "player.inCombat") {
        return entity_boolean_field(scope, args.get(1), "in_combat");
    }
    if is_op(scope, op, "player.combatLevel") {
        return player_combat_level(scope, args.get(1));
    }
    if is_op(scope, op, "player.targetsMe") {
        return entity_targets_me(scope, args.get(1), args.get(2));
    }
    if is_op(scope, op, "player.tile") {
        return entity_tile(scope, args.get(1), args.get(2));
    }
    if is_op(scope, op, "player.distance") {
        return entity_distance(scope, args.get(1));
    }
    if is_op(scope, op, "player.actions") {
        return entity_actions(scope, args.get(1));
    }
    if is_op(scope, op, "player.interact") {
        return player_interact(scope, args.get(1), args.get(2), args.get(3));
    }

    if is_op(scope, op, "grounditem.name") {
        return entity_name(scope, args.get(1));
    }
    if is_op(scope, op, "grounditem.id") {
        return entity_snap_field(scope, args.get(1), "id");
    }
    if is_op(scope, op, "grounditem.count") {
        return ground_item_count(scope, args.get(1));
    }
    if is_op(scope, op, "grounditem.tile") {
        return entity_tile(scope, args.get(1), args.get(2));
    }
    if is_op(scope, op, "grounditem.distance") {
        return entity_distance(scope, args.get(1));
    }
    if is_op(scope, op, "grounditem.actions") {
        return entity_actions(scope, args.get(1));
    }
    if is_op(scope, op, "grounditem.interact") {
        return ground_item_interact(scope, args.get(1), args.get(2), args.get(3));
    }

    Err(cb::not_impl(scope, "entity binding"))
}

fn is_op(scope: &mut v8::HandleScope, value: v8::Local<v8::Value>, expected: &str) -> bool {
    v8::String::new(scope, expected).is_some_and(|expected| value.strict_equals(expected.into()))
}

fn boolean<'s>(scope: &mut v8::HandleScope<'s>, value: bool) -> v8::Local<'s, v8::Value> {
    v8::Boolean::new(scope, value).into()
}

fn null<'s>(scope: &mut v8::HandleScope<'s>) -> v8::Local<'s, v8::Value> {
    v8::null(scope).into()
}

fn undefined<'s>(scope: &mut v8::HandleScope<'s>) -> v8::Local<'s, v8::Value> {
    v8::undefined(scope).into()
}

fn global<'s>(scope: &mut v8::HandleScope<'s>) -> v8::Local<'s, v8::Object> {
    scope.get_current_context().global(scope)
}

fn string_call<'s>(
    scope: &mut v8::HandleScope<'s>,
    value: v8::Local<'s, v8::Value>,
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    let global = global(scope);
    let constructor = cb::get(scope, global.into(), "String")?;
    cb::call(scope, constructor, undefined(scope), &[value], "String")
}

fn lower_case<'s>(
    scope: &mut v8::HandleScope<'s>,
    value: v8::Local<'s, v8::Value>,
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    cb::call_method(scope, value, "toLowerCase", &[], "toLowerCase")
}

fn own_data<'s>(
    scope: &mut v8::HandleScope<'s>,
    object: v8::Local<'s, v8::Object>,
    key: &str,
    value: v8::Local<'s, v8::Value>,
) -> JsResult<'s, ()> {
    let key = v8::String::new(scope, key).unwrap_or_else(|| v8::String::empty(scope));
    let set = cb::catching(scope, |s| object.create_data_property(s, key.into(), value))?;
    if set {
        Ok(())
    } else {
        Err(cb::type_error(scope, "could not create entity property"))
    }
}

fn data_object<'s>(
    scope: &mut v8::HandleScope<'s>,
    key: &str,
    value: v8::Local<'s, v8::Value>,
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    cb::object(scope, &[(key, value)])
}

fn build_callback<'s>(
    scope: &mut v8::HandleScope<'s>,
    callback: impl v8::MapFnTo<v8::FunctionCallback>,
    data: v8::Local<'s, v8::Value>,
    message: &str,
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    let Some(callback) = v8::Function::builder(callback).data(data).build(scope) else {
        return Err(cb::type_error(scope, message));
    };
    Ok(callback.into())
}

fn object_with_keys<'s>(
    scope: &mut v8::HandleScope<'s>,
    object: v8::Local<'s, v8::Object>,
) -> JsResult<'s, v8::Local<'s, v8::Array>> {
    let mut options = v8::GetPropertyNamesArgsBuilder::new();
    options
        .mode(v8::KeyCollectionMode::OwnOnly)
        .property_filter(v8::PropertyFilter::ALL_PROPERTIES)
        .index_filter(v8::IndexFilter::IncludeIndices)
        .key_conversion(v8::KeyConversionMode::ConvertToString);
    cb::catching(scope, |s| object.get_own_property_names(s, options.build()))
}

fn entity_snap_view<'s>(
    scope: &mut v8::HandleScope<'s>,
    row: v8::Local<'s, v8::Value>,
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    if !cb::truthy(scope, row) {
        return Ok(null(scope));
    }
    let source = row.to_object(scope).ok_or_else(|| {
        cb::type_error(scope, "Cannot convert undefined or null to object")
    })?;
    let view = v8::Object::new(scope);
    spread_own_enumerable(scope, source, view)?;

    let x = cb::get(scope, row, "x")?;
    let z = cb::get(scope, row, "z")?;
    let level = cb::get(scope, row, "level")?;
    let level = if level.is_null_or_undefined() {
        cb::num(scope, 0.0)
    } else {
        level
    };
    let tile = cb::object(scope, &[("x", x), ("z", z), ("level", level)])?;
    own_data(scope, view, "tile", tile)?;

    let actions = cb::get(scope, row, "actions")?;
    let ops = if cb::truthy(scope, actions) {
        actions
    } else {
        v8::Array::new(scope, 0).into()
    };
    own_data(scope, view, "ops", ops)?;

    let distance = cb::get(scope, row, "distance")?;
    let distance = if distance.is_number() {
        cb::get(scope, row, "distance")?
    } else {
        cb::num(scope, 0.0)
    };
    own_data(scope, view, "distance", distance)?;
    Ok(view.into())
}

fn spread_own_enumerable<'s>(
    scope: &mut v8::HandleScope<'s>,
    source: v8::Local<'s, v8::Object>,
    target: v8::Local<'s, v8::Object>,
) -> JsResult<'s, ()> {
    let keys = object_with_keys(scope, source)?;
    let poll = Poll::default();
    for index in 0..keys.length() {
        let scope = &mut v8::EscapableHandleScope::new(scope);
        let flow = spread_property(scope, source, target, keys, index, &poll);
        if let Flow::Break(_) = cb::iteration(scope, flow)? {
            break;
        }
    }
    Ok(())
}

fn spread_property<'s>(
    scope: &mut v8::HandleScope<'s>,
    source: v8::Local<'s, v8::Object>,
    target: v8::Local<'s, v8::Object>,
    keys: v8::Local<'s, v8::Array>,
    index: u32,
    poll: &Poll,
) -> JsResult<'s, Flow<'s>> {
    poll.check(scope)?;
    let key = cb::get_index(scope, keys.into(), index)?;
    let key_name = v8::Local::<v8::Name>::try_from(key)
        .map_err(|_| cb::type_error(scope, "entity spread key is not a property name"))?;
    let descriptor = cb::catching(scope, |s| source.get_own_property_descriptor(s, key_name))?;
    if descriptor.is_null_or_undefined() {
        return Ok(Flow::Continue(None));
    }
    let enumerable = cb::get(scope, descriptor, "enumerable")?;
    if !cb::truthy(scope, enumerable) {
        return Ok(Flow::Continue(None));
    }
    let value = cb::get(scope, source.into(), key)?;
    let created = cb::catching(scope, |s| target.create_data_property(s, key_name, value))?;
    if !created {
        return Err(cb::type_error(scope, "could not copy entity property"));
    }
    Ok(Flow::Continue(None))
}

fn present_ops<'s>(
    scope: &mut v8::HandleScope<'s>,
    actions: v8::Local<'s, v8::Value>,
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    let actions = if cb::truthy(scope, actions) {
        actions
    } else {
        v8::Array::new(scope, 0).into()
    };
    let predicate = v8::Function::new(scope, present_ops_predicate)
        .ok_or_else(|| cb::type_error(scope, "could not create action filter"))?;
    cb::call_method(scope, actions, "filter", &[predicate.into()], "actions.filter")
}

fn present_ops_predicate<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue,
) {
    let action = args.get(0);
    if !cb::truthy(scope, action) {
        rv.set(action);
    } else {
        let hidden = v8::String::new(scope, "hidden").unwrap_or_else(|| v8::String::empty(scope));
        rv.set(boolean(scope, !action.strict_equals(hidden.into())));
    }
}

fn op_index_value<'s>(
    scope: &mut v8::HandleScope<'s>,
    actions: v8::Local<'s, v8::Value>,
    action: v8::Local<'s, v8::Value>,
) -> JsResult<'s, f64> {
    let string = string_call(scope, action)?;
    let wanted = lower_case(scope, string)?;
    let actions = if cb::truthy(scope, actions) {
        actions
    } else {
        v8::Array::new(scope, 0).into()
    };
    let poll = Poll::default();
    let mut index = 0.0;
    loop {
        poll.check(scope)?;
        let length = cb::get(scope, actions, "length")?;
        let current_index = cb::num(scope, index);
        if !cb::lt(scope, current_index, length)? {
            return Ok(-1.0);
        }
        let object = actions.to_object(scope).ok_or_else(|| {
            cb::type_error(scope, "Cannot convert undefined or null to object")
        })?;
        let action_at_index = cb::num(scope, index);
        let candidate = cb::catching(scope, |s| object.get(s, action_at_index))?;
        if cb::truthy(scope, candidate) {
            let string = string_call(scope, candidate)?;
            let name = lower_case(scope, string)?;
            if name.strict_equals(wanted) {
                return Ok(index + 1.0);
            }
        }
        index += 1.0;
    }
}

fn matches_entity_name<'s>(
    scope: &mut v8::HandleScope<'s>,
    actual: v8::Local<'s, v8::Value>,
    configured: v8::Local<'s, v8::Value>,
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    if actual.is_null_or_undefined() || configured.is_null_or_undefined() {
        return Ok(boolean(scope, false));
    }
    let actual = string_call(scope, actual)?;
    let actual = cb::call_method(scope, actual, "trim", &[], "trim")?;
    let actual = lower_case(scope, actual)?;
    let configured = string_call(scope, configured)?;
    let configured = cb::call_method(scope, configured, "trim", &[], "trim")?;
    let configured = lower_case(scope, configured)?;
    Ok(boolean(scope, actual.strict_equals(configured)))
}

fn talk_op<'s>(
    scope: &mut v8::HandleScope<'s>,
    actions: v8::Local<'s, v8::Value>,
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    let actions = if cb::truthy(scope, actions) {
        actions
    } else {
        v8::Array::new(scope, 0).into()
    };
    let find = cb::get(scope, actions, "find")?;
    let predicate = v8::Function::new(scope, talk_op_predicate)
        .ok_or_else(|| cb::type_error(scope, "could not create talk action predicate"))?;
    let found = cb::call(scope, find, actions, &[predicate.into()], "actions.find")?;
    if found.is_null_or_undefined() {
        Ok(null(scope))
    } else {
        Ok(found)
    }
}

fn talk_op_predicate<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue,
) {
    let result = (|| -> JsResult<'s, v8::Local<'s, v8::Value>> {
        let source = v8::String::new(scope, "^talk").unwrap_or_else(|| v8::String::empty(scope));
        let Some(regex) = v8::RegExp::new(scope, source, v8::RegExpCreationFlags::IGNORE_CASE)
        else {
            return Err(cb::type_error(scope, "could not create talk action regular expression"));
        };
        let test = cb::get(scope, regex.into(), "test")?;
        let text = string_call(scope, args.get(0))?;
        cb::call(scope, test, regex.into(), &[text], "talk action test")
    })();
    cb::finish(scope, rv, result);
}

fn entities_all<'s>(
    scope: &mut v8::HandleScope<'s>,
    snap_fn: v8::Local<'s, v8::Value>,
    collection: v8::Local<'s, v8::Value>,
    constructor: v8::Local<'s, v8::Value>,
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    let snapshot = cb::call(scope, snap_fn, undefined(scope), &[], "snap")?;
    let key = if is_op(scope, collection, "npcs") {
        "npcs"
    } else if is_op(scope, collection, "players") {
        "players"
    } else {
        return Err(cb::not_impl(scope, "entity collection"));
    };
    let rows = cb::get(scope, snapshot, key)?;
    let rows = if cb::truthy(scope, rows) {
        rows
    } else {
        v8::Array::new(scope, 0).into()
    };
    let map = cb::get(scope, rows, "map")?;
    let data = data_object(scope, "constructor", constructor)?;
    let mapper = build_callback(scope, wrap_entity_row, data, "could not create entity mapper")?;
    cb::call(scope, map, rows, &[mapper], "entity snapshots map")
}

fn wrap_entity_row<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue,
) {
    let result = (|| {
        let constructor = cb::get(scope, args.data(), "constructor")?;
        cb::construct(scope, constructor, &[args.get(0)], "entity constructor")
    })();
    cb::finish(scope, rv, result);
}

fn npcs_nearest<'s>(
    scope: &mut v8::HandleScope<'s>,
    npcs: v8::Local<'s, v8::Value>,
    count: v8::Local<'s, v8::Value>,
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    let all = cb::call_method(scope, npcs, "all", &[], "Npcs.all")?;
    let sort = cb::get(scope, all, "sort")?;
    let comparator = v8::Function::new(scope, distance_compare)
        .ok_or_else(|| cb::type_error(scope, "could not create entity distance comparator"))?;
    let sorted = cb::call(scope, sort, all, &[comparator.into()], "entities.sort")?;
    let slice = cb::get(scope, sorted, "slice")?;
    let zero = cb::num(scope, 0.0);
    cb::call(scope, slice, sorted, &[zero, count], "entities.slice")
}

fn distance_compare<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue,
) {
    let result = (|| {
        let left = cb::call_method(scope, args.get(0), "distance", &[], "a.distance")?;
        let right = cb::call_method(scope, args.get(1), "distance", &[], "b.distance")?;
        cb::sub(scope, left, right)
    })();
    cb::finish(scope, rv, result);
}

fn query_name<'s>(
    scope: &mut v8::HandleScope<'s>,
    query: v8::Local<'s, v8::Value>,
    names: v8::Local<'s, v8::Value>,
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    let map = cb::get(scope, names, "map")?;
    let mapper = v8::Function::new(scope, normalize_query_name)
        .ok_or_else(|| cb::type_error(scope, "could not create query name mapper"))?;
    let wanted = cb::call(scope, map, names, &[mapper.into()], "query names.map")?;
    let data = data_object(scope, "wanted", wanted)?;
    let predicate = build_callback(scope, query_name_predicate, data, "could not create name filter")?;
    push_filter(scope, query, "snapFilters", predicate)
}

fn normalize_query_name<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue,
) {
    let result = (|| {
        let trimmed = cb::call_method(scope, args.get(0), "trim", &[], "name.trim")?;
        lower_case(scope, trimmed)
    })();
    cb::finish(scope, rv, result);
}

fn query_name_predicate<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue,
) {
    let result = (|| -> JsResult<'s, v8::Local<'s, v8::Value>> {
        let view = args.get(0);
        let name = cb::get(scope, view, "name")?;
        if name.is_null() {
            return Ok(boolean(scope, false));
        }
        let name = cb::get(scope, view, "name")?;
        let name = string_call(scope, name)?;
        let name = cb::call_method(scope, name, "trim", &[], "name.trim")?;
        let name = lower_case(scope, name)?;
        let wanted = cb::get(scope, args.data(), "wanted")?;
        cb::call_method(scope, wanted, "includes", &[name], "wanted.includes")
    })();
    cb::finish(scope, rv, result);
}

fn query_action<'s>(
    scope: &mut v8::HandleScope<'s>,
    query: v8::Local<'s, v8::Value>,
    action: v8::Local<'s, v8::Value>,
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    let string = string_call(scope, action)?;
    let wanted = lower_case(scope, string)?;
    let data = data_object(scope, "wanted", wanted)?;
    let predicate = build_callback(scope, query_action_predicate, data, "could not create action filter")?;
    push_filter(scope, query, "snapFilters", predicate)
}

fn query_action_predicate<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue,
) {
    let result = (|| -> JsResult<'s, v8::Local<'s, v8::Value>> {
        let view = args.get(0);
        let ops = cb::get(scope, view, "ops")?;
        let ops = if cb::truthy(scope, ops) {
            ops
        } else {
            v8::Array::new(scope, 0).into()
        };
        let some = cb::get(scope, ops, "some")?;
        let wanted = cb::get(scope, args.data(), "wanted")?;
        let data = data_object(scope, "wanted", wanted)?;
        let predicate = build_callback(
            scope,
            query_action_matches,
            data,
            "could not create action predicate",
        )?;
        cb::call(scope, some, ops, &[predicate], "entity ops.some")
    })();
    cb::finish(scope, rv, result);
}

fn query_action_matches<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue,
) {
    let result = (|| -> JsResult<'s, v8::Local<'s, v8::Value>> {
        let action = args.get(0);
        if action.is_null_or_undefined() {
            return Ok(boolean(scope, false));
        }
        let hidden = v8::String::new(scope, "hidden").unwrap_or_else(|| v8::String::empty(scope));
        if action.strict_equals(hidden.into()) {
            return Ok(boolean(scope, false));
        }
        let string = string_call(scope, action)?;
        let action = lower_case(scope, string)?;
        let wanted = cb::get(scope, args.data(), "wanted")?;
        Ok(boolean(scope, action.strict_equals(wanted)))
    })();
    cb::finish(scope, rv, result);
}

fn query_within<'s>(
    scope: &mut v8::HandleScope<'s>,
    query: v8::Local<'s, v8::Value>,
    distance: v8::Local<'s, v8::Value>,
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    let data = data_object(scope, "distance", distance)?;
    let predicate = build_callback(scope, query_within_predicate, data, "could not create distance filter")?;
    push_filter(scope, query, "snapFilters", predicate)
}

fn query_within_predicate<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue,
) {
    let result = (|| -> JsResult<'s, v8::Local<'s, v8::Value>> {
        let distance = cb::get(scope, args.get(0), "distance")?;
        let limit = cb::get(scope, args.data(), "distance")?;
        let within = cb::le(scope, distance, limit)?;
        Ok(boolean(scope, within))
    })();
    cb::finish(scope, rv, result);
}

fn math_radius<'s>(
    scope: &mut v8::HandleScope<'s>,
    distance: v8::Local<'s, v8::Value>,
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    let global = global(scope);
    let math_for_max = cb::get(scope, global.into(), "Math")?;
    let max = cb::get(scope, math_for_max, "max")?;
    let math_for_floor = cb::get(scope, global.into(), "Math")?;
    let floor = cb::get(scope, math_for_floor, "floor")?;
    let floor_value = cb::call(scope, floor, math_for_floor, &[distance], "Math.floor")?;
    let zero = cb::num(scope, 0.0);
    cb::call(scope, max, math_for_max, &[zero, floor_value], "Math.max")
}

fn query_within_of<'s>(
    scope: &mut v8::HandleScope<'s>,
    query: v8::Local<'s, v8::Value>,
    origin: v8::Local<'s, v8::Value>,
    distance: v8::Local<'s, v8::Value>,
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    let radius = math_radius(scope, distance)?;
    let data = cb::object(scope, &[("origin", origin), ("radius", radius)])?;
    let predicate = build_callback(
        scope,
        query_within_of_predicate,
        data,
        "could not create local distance filter",
    )?;
    push_filter(scope, query, "snapFilters", predicate)
}

fn query_within_of_predicate<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue,
) {
    let result = (|| -> JsResult<'s, v8::Local<'s, v8::Value>> {
        let tile = cb::get(scope, args.get(0), "tile")?;
        let data = args.data();
        let origin = cb::get(scope, data, "origin")?;
        let radius = cb::get(scope, data, "radius")?;
        let distance = planar_distance(scope, tile, origin)?;
        let within = cb::le(scope, distance, radius)?;
        Ok(boolean(scope, within))
    })();
    cb::finish(scope, rv, result);
}

fn planar_distance<'s>(
    scope: &mut v8::HandleScope<'s>,
    tile: v8::Local<'s, v8::Value>,
    origin: v8::Local<'s, v8::Value>,
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    if !cb::truthy(scope, tile) || !cb::truthy(scope, origin) {
        return Ok(cb::num(scope, f64::INFINITY));
    }
    let global = global(scope);
    let distance = cb::get(scope, global.into(), "__rs2b0t_distance")?;
    let planar = boolean(scope, true);
    cb::call(
        scope,
        distance,
        global.into(),
        &[tile, origin, planar],
        "__rs2b0t_distance",
    )
}

fn query_inside<'s>(
    scope: &mut v8::HandleScope<'s>,
    query: v8::Local<'s, v8::Value>,
    area: v8::Local<'s, v8::Value>,
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    let data = data_object(scope, "area", area)?;
    let predicate = build_callback(scope, query_inside_predicate, data, "could not create area filter")?;
    push_filter(scope, query, "snapFilters", predicate)
}

fn query_inside_predicate<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue,
) {
    let result = (|| -> JsResult<'s, v8::Local<'s, v8::Value>> {
        let view = args.get(0);
        let tile = cb::get(scope, view, "tile")?;
        let area = cb::get(scope, args.data(), "area")?;
        let x = cb::get(scope, tile, "x")?;
        let min_x = cb::get(scope, area, "minX")?;
        if !cb::ge(scope, x, min_x)? {
            return Ok(boolean(scope, false));
        }
        let x = cb::get(scope, tile, "x")?;
        let max_x = cb::get(scope, area, "maxX")?;
        if !cb::le(scope, x, max_x)? {
            return Ok(boolean(scope, false));
        }
        let z = cb::get(scope, tile, "z")?;
        let min_z = cb::get(scope, area, "minZ")?;
        if !cb::ge(scope, z, min_z)? {
            return Ok(boolean(scope, false));
        }
        let z = cb::get(scope, tile, "z")?;
        let max_z = cb::get(scope, area, "maxZ")?;
        let inside = cb::le(scope, z, max_z)?;
        Ok(boolean(scope, inside))
    })();
    cb::finish(scope, rv, result);
}


fn push_filter<'s>(
    scope: &mut v8::HandleScope<'s>,
    query: v8::Local<'s, v8::Value>,
    key: &str,
    predicate: v8::Local<'s, v8::Value>,
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    let filters = cb::get(scope, query, key)?;
    cb::call_method(scope, filters, "push", &[predicate], "filter array.push")?;
    Ok(query)
}

fn filter_array_passes<'s>(
    scope: &mut v8::HandleScope<'s>,
    query: v8::Local<'s, v8::Value>,
    key: &str,
    view: v8::Local<'s, v8::Value>,
) -> JsResult<'s, bool> {
    let first = cb::get(scope, query, key)?;
    let length = cb::get(scope, first, "length")?;
    let zero = cb::num(scope, 0.0);
    if !cb::gt(scope, length, zero)? {
        return Ok(true);
    }
    let filters = cb::get(scope, query, key)?;
    let every = cb::get(scope, filters, "every")?;
    let data = data_object(scope, "view", view)?;
    let predicate = build_callback(scope, invoke_filter, data, "could not create filter callback")?;
    let label = if key == "snapFilters" {
        "this.snapFilters.every"
    } else {
        "this.entityFilters.every"
    };
    let result = cb::call(scope, every, filters, &[predicate], label)?;
    Ok(cb::truthy(scope, result))
}

fn invoke_filter<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue,
) {
    let result = (|| {
        let view = cb::get(scope, args.data(), "view")?;
        cb::call(scope, args.get(0), undefined(scope), &[view], "f")
    })();
    cb::finish(scope, rv, result);
}

fn query_for_each_match<'s>(
    scope: &mut v8::HandleScope<'s>,
    query: v8::Local<'s, v8::Value>,
    seen: v8::Local<'s, v8::Value>,
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    let snapshots = cb::call_method(scope, query, "supplySnaps", &[], "this.supplySnaps")?;
    let snapshots = ForOf::open(scope, snapshots, "this.supplySnaps()")?;
    loop {
        let scope = &mut v8::EscapableHandleScope::new(scope);
        let flow = match_entity_iteration(scope, &snapshots, query, seen);
        match cb::iteration(scope, flow)? {
            Flow::Continue(_) => {}
            Flow::Break(_) => break,
        }
    }
    Ok(undefined(scope))
}

fn match_entity_iteration<'s>(
    scope: &mut v8::HandleScope<'s>,
    snapshots: &ForOf<'s>,
    query: v8::Local<'s, v8::Value>,
    seen: v8::Local<'s, v8::Value>,
) -> JsResult<'s, Flow<'s>> {
    let Some(raw) = snapshots.step(scope)? else {
        return Ok(Flow::Break(None));
    };
    let body = (|| -> JsResult<'s, Flow<'s>> {
        let view = entity_snap_view(scope, raw)?;
        if view.is_null() {
            return Ok(Flow::Continue(None));
        }
        if !filter_array_passes(scope, query, "snapFilters", view)? {
            return Ok(Flow::Continue(None));
        }
        let entity = cb::call_method(scope, query, "wrap", &[raw], "this.wrap")?;
        if !filter_array_passes(scope, query, "entityFilters", entity)? {
            return Ok(Flow::Continue(None));
        }
        let result = cb::call(scope, seen, undefined(scope), &[entity], "seen")?;
        if cb::truthy(scope, result) {
            snapshots.close(scope)?;
            Ok(Flow::Break(None))
        } else {
            Ok(Flow::Continue(None))
        }
    })();
    snapshots.body(scope, body)
}

fn query_results<'s>(
    scope: &mut v8::HandleScope<'s>,
    query: v8::Local<'s, v8::Value>,
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    let output: v8::Local<v8::Value> = v8::Array::new(scope, 0).into();
    let data = data_object(scope, "output", output)?;
    let callback = build_callback(scope, collect_result, data, "could not create query collector")?;
    cb::call_method(scope, query, "forEachMatch", &[callback], "this.forEachMatch")?;
    Ok(output)
}

fn collect_result<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue,
) {
    let result = (|| -> JsResult<'s, v8::Local<'s, v8::Value>> {
        let output = cb::get(scope, args.data(), "output")?;
        cb::call_method(scope, output, "push", &[args.get(0)], "out.push")?;
        Ok(boolean(scope, false))
    })();
    cb::finish(scope, rv, result);
}

fn query_nearest<'s>(
    scope: &mut v8::HandleScope<'s>,
    query: v8::Local<'s, v8::Value>,
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    let data = data_object(scope, "best", null(scope))?;
    let callback = build_callback(scope, rank_nearest, data, "could not create nearest callback")?;
    cb::call_method(scope, query, "forEachMatch", &[callback], "this.forEachMatch")?;
    cb::get(scope, data, "best")
}

fn set_data<'s>(
    scope: &mut v8::HandleScope<'s>,
    data: v8::Local<'s, v8::Value>,
    key: &str,
    value: v8::Local<'s, v8::Value>,
) -> JsResult<'s, ()> {
    let object = data
        .to_object(scope)
        .ok_or_else(|| cb::type_error(scope, "invalid query callback state"))?;
    let key = cb::string(scope, key);
    if cb::catching(scope, |s| object.set(s, key, value))? {
        Ok(())
    } else {
        Err(cb::type_error(scope, "could not update query callback state"))
    }
}
fn rank_nearest<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue,
) {
    let result = (|| -> JsResult<'s, v8::Local<'s, v8::Value>> {
        let data = args.data();
        let best = cb::get(scope, data, "best")?;
        let entity = args.get(0);
        if !cb::truthy(scope, best) {
            set_data(scope, data, "best", entity)?;
        } else {
            let distance = cb::call_method(scope, entity, "distance", &[], "e.distance")?;
            let best_distance = cb::call_method(scope, best, "distance", &[], "best.distance")?;
            if cb::lt(scope, distance, best_distance)? {
                set_data(scope, data, "best", entity)?;
            }
        }
        Ok(boolean(scope, false))
    })();
    cb::finish(scope, rv, result);
}

fn query_first<'s>(
    scope: &mut v8::HandleScope<'s>,
    query: v8::Local<'s, v8::Value>,
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    let data = data_object(scope, "hit", null(scope))?;
    let callback = build_callback(scope, capture_first, data, "could not create first callback")?;
    cb::call_method(scope, query, "forEachMatch", &[callback], "this.forEachMatch")?;
    cb::get(scope, data, "hit")
}

fn capture_first<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue,
) {
    let result = (|| -> JsResult<'s, v8::Local<'s, v8::Value>> {
        let data = args.data();
        set_data(scope, data, "hit", args.get(0))?;
        Ok(boolean(scope, true))
    })();
    cb::finish(scope, rv, result);
}

fn query_exists<'s>(
    scope: &mut v8::HandleScope<'s>,
    query: v8::Local<'s, v8::Value>,
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    let first = cb::call_method(scope, query, "first", &[], "this.first")?;
    Ok(boolean(scope, !first.is_null()))
}

fn query_count<'s>(
    scope: &mut v8::HandleScope<'s>,
    query: v8::Local<'s, v8::Value>,
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    let zero = cb::num(scope, 0.0);
    let data = data_object(scope, "count", zero)?;
    let callback = build_callback(scope, count_result, data, "could not create count callback")?;
    cb::call_method(scope, query, "forEachMatch", &[callback], "this.forEachMatch")?;
    cb::get(scope, data, "count")
}

fn count_result<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue,
) {
    let result = (|| -> JsResult<'s, v8::Local<'s, v8::Value>> {
        let data = args.data();
        let count = cb::get(scope, data, "count")?;
        let count = cb::number(scope, count)? + 1.0;
        let count = cb::num(scope, count);
        set_data(scope, data, "count", count)?;
        Ok(boolean(scope, false))
    })();
    cb::finish(scope, rv, result);
}

fn query_nearest_prefer_local<'s>(
    scope: &mut v8::HandleScope<'s>,
    query: v8::Local<'s, v8::Value>,
    prefer_radius: v8::Local<'s, v8::Value>,
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    let radius = math_radius(scope, prefer_radius)?;
    let all = cb::call_method(scope, query, "results", &[], "this.results")?;
    let length = cb::get(scope, all, "length")?;
    let zero = cb::num(scope, 0.0);
    if length.strict_equals(zero) {
        return Ok(null(scope));
    }

    let mut pool = all;
    let zero = cb::num(scope, 0.0);
    if cb::gt(scope, radius, zero)? {
        let filter = cb::get(scope, all, "filter")?;
        let data = data_object(scope, "radius", radius)?;
        let predicate = build_callback(
            scope,
            local_distance_predicate,
            data,
            "could not create local result filter",
        )?;
        let local = cb::call(scope, filter, all, &[predicate], "all.filter")?;
        let local_length = cb::get(scope, local, "length")?;
        let zero = cb::num(scope, 0.0);
        if cb::gt(scope, local_length, zero)? {
            pool = local;
        }
    }
    rank_result_pool(scope, pool)
}

fn local_distance_predicate<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue,
) {
    let result = (|| -> JsResult<'s, v8::Local<'s, v8::Value>> {
        let entity = args.get(0);
        let distance = cb::call_method(scope, entity, "distance", &[], "e.distance")?;
        let radius = cb::get(scope, args.data(), "radius")?;
        let within = cb::le(scope, distance, radius)?;
        Ok(boolean(scope, within))
    })();
    cb::finish(scope, rv, result);
}

fn rank_result_pool<'s>(
    scope: &mut v8::HandleScope<'s>,
    pool: v8::Local<'s, v8::Value>,
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    let pool = ForOf::open(scope, pool, "pool")?;
    let mut best = null(scope);
    loop {
        let scope = &mut v8::EscapableHandleScope::new(scope);
        let flow = rank_pool_iteration(scope, &pool, best);
        match cb::iteration(scope, flow)? {
            Flow::Continue(Some(next)) => best = next,
            Flow::Continue(None) => {}
            Flow::Break(Some(next)) => {
                best = next;
                break;
            }
            Flow::Break(None) => break,
        }
    }
    Ok(best)
}

fn rank_pool_iteration<'s>(
    scope: &mut v8::HandleScope<'s>,
    pool: &ForOf<'s>,
    best: v8::Local<'s, v8::Value>,
) -> JsResult<'s, Flow<'s>> {
    let Some(entity) = pool.step(scope)? else {
        return Ok(Flow::Break(Some(best)));
    };
    let body = (|| -> JsResult<'s, Option<v8::Local<'s, v8::Value>>> {
        if !cb::truthy(scope, best) {
            return Ok(Some(entity));
        }
        let distance = cb::call_method(scope, entity, "distance", &[], "e.distance")?;
        let best_distance = cb::call_method(scope, best, "distance", &[], "best.distance")?;
        if cb::lt(scope, distance, best_distance)? {
            Ok(Some(entity))
        } else {
            Ok(None)
        }
    })();
    match pool.body(scope, body)? {
        Some(next) => Ok(Flow::Continue(Some(next))),
        None => Ok(Flow::Continue(Some(best))),
    }
}


fn entity_instance_snap<'s>(
    scope: &mut v8::HandleScope<'s>,
    instance: v8::Local<'s, v8::Value>,
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    cb::get(scope, instance, "snap")
}

fn entity_name<'s>(
    scope: &mut v8::HandleScope<'s>,
    instance: v8::Local<'s, v8::Value>,
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    let row = entity_instance_snap(scope, instance)?;
    let name = cb::get(scope, row, "name")?;
    if name.is_null_or_undefined() {
        Ok(null(scope))
    } else {
        Ok(name)
    }
}

fn entity_snap_field<'s>(
    scope: &mut v8::HandleScope<'s>,
    instance: v8::Local<'s, v8::Value>,
    key: &str,
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    let row = entity_instance_snap(scope, instance)?;
    cb::get(scope, row, key)
}

fn entity_boolean_field<'s>(
    scope: &mut v8::HandleScope<'s>,
    instance: v8::Local<'s, v8::Value>,
    key: &str,
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    let value = entity_snap_field(scope, instance, key)?;
    Ok(boolean(scope, value.is_true()))
}

fn number_is_integer<'s>(
    scope: &mut v8::HandleScope<'s>,
    value: v8::Local<'s, v8::Value>,
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    let global = global(scope);
    let number = cb::get(scope, global.into(), "Number")?;
    let is_integer = cb::get(scope, number, "isInteger")?;
    cb::call(scope, is_integer, number, &[value], "Number.isInteger")
}

fn snap_number_is_integer<'s>(
    scope: &mut v8::HandleScope<'s>,
    instance: v8::Local<'s, v8::Value>,
    key: &str,
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    let global = global(scope);
    let number = cb::get(scope, global.into(), "Number")?;
    let is_integer = cb::get(scope, number, "isInteger")?;
    let row = entity_instance_snap(scope, instance)?;
    let value = cb::get(scope, row, key)?;
    cb::call(scope, is_integer, number, &[value], "Number.isInteger")
}

fn npc_level<'s>(
    scope: &mut v8::HandleScope<'s>,
    instance: v8::Local<'s, v8::Value>,
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    let row = entity_instance_snap(scope, instance)?;
    let level = cb::get(scope, row, "combat_level")?;
    if !level.is_number() {
        return Err(cb::not_impl(scope, "Npc.level"));
    }
    entity_snap_field(scope, instance, "combat_level")
}

fn npc_size<'s>(
    scope: &mut v8::HandleScope<'s>,
    instance: v8::Local<'s, v8::Value>,
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    let first = entity_snap_field(scope, instance, "size")?;
    if !first.is_number() {
        return Err(cb::not_impl(scope, "Npc.size"));
    }
    let integer = snap_number_is_integer(scope, instance, "size")?;
    if !cb::truthy(scope, integer) {
        return Err(cb::not_impl(scope, "Npc.size"));
    }
    let size = entity_snap_field(scope, instance, "size")?;
    let one = cb::num(scope, 1.0);
    if cb::lt(scope, size, one)? {
        return Err(cb::not_impl(scope, "Npc.size"));
    }
    entity_snap_field(scope, instance, "size")
}

fn npc_network_origin<'s>(
    scope: &mut v8::HandleScope<'s>,
    instance: v8::Local<'s, v8::Value>,
    tile_ctor: v8::Local<'s, v8::Value>,
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    let first_size = entity_snap_field(scope, instance, "size")?;
    if !first_size.is_number() {
        return Err(cb::not_impl(scope, "Npc.networkOrigin"));
    }
    let integer = snap_number_is_integer(scope, instance, "size")?;
    if !cb::truthy(scope, integer) {
        return Err(cb::not_impl(scope, "Npc.networkOrigin"));
    }
    let size = entity_snap_field(scope, instance, "size")?;
    let one = cb::num(scope, 1.0);
    if cb::lt(scope, size, one)? {
        return Err(cb::not_impl(scope, "Npc.networkOrigin"));
    }
    let nx_integer = snap_number_is_integer(scope, instance, "nx")?;
    if !cb::truthy(scope, nx_integer) {
        return Err(cb::not_impl(scope, "Npc.networkOrigin"));
    }
    let nz_integer = snap_number_is_integer(scope, instance, "nz")?;
    if !cb::truthy(scope, nz_integer) {
        return Err(cb::not_impl(scope, "Npc.networkOrigin"));
    }
    let row = entity_instance_snap(scope, instance)?;
    let x = cb::get(scope, row, "nx")?;
    let row = entity_instance_snap(scope, instance)?;
    let z = cb::get(scope, row, "nz")?;
    let row = entity_instance_snap(scope, instance)?;
    let level = cb::get(scope, row, "level")?;
    let level = if level.is_null_or_undefined() {
        cb::num(scope, 0.0)
    } else {
        level
    };
    let input = cb::object(scope, &[("x", x), ("z", z), ("level", level)])?;
    cb::call_method(scope, tile_ctor, "from", &[input], "Tile.from")
}

fn npc_network_tile<'s>(
    scope: &mut v8::HandleScope<'s>,
    instance: v8::Local<'s, v8::Value>,
    tile_ctor: v8::Local<'s, v8::Value>,
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    let row = entity_instance_snap(scope, instance)?;
    let network_x = cb::get(scope, row, "network_x")?;
    let x = if network_x.is_null_or_undefined() {
        let row = entity_instance_snap(scope, instance)?;
        cb::get(scope, row, "x")?
    } else {
        network_x
    };
    let row = entity_instance_snap(scope, instance)?;
    let network_z = cb::get(scope, row, "network_z")?;
    let z = if network_z.is_null_or_undefined() {
        let row = entity_instance_snap(scope, instance)?;
        cb::get(scope, row, "z")?
    } else {
        network_z
    };
    let row = entity_instance_snap(scope, instance)?;
    let level = cb::get(scope, row, "level")?;
    let level = if level.is_null_or_undefined() {
        cb::num(scope, 0.0)
    } else {
        level
    };
    let input = cb::object(scope, &[("x", x), ("z", z), ("level", level)])?;
    cb::call_method(scope, tile_ctor, "from", &[input], "Tile.from")
}

fn entity_targets_me<'s>(
    scope: &mut v8::HandleScope<'s>,
    instance: v8::Local<'s, v8::Value>,
    snap_fn: v8::Local<'s, v8::Value>,
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    let row = entity_instance_snap(scope, instance)?;
    let target_kind = cb::get(scope, row, "target_kind")?;
    let player_target_kind = cb::num(scope, 2.0);
    if !target_kind.strict_equals(player_target_kind) {
        return Ok(boolean(scope, false));
    }
    let row = entity_instance_snap(scope, instance)?;
    let target_index = cb::get(scope, row, "target_index")?;
    let snapshot = cb::call(scope, snap_fn, undefined(scope), &[], "snap")?;
    let self_slot = cb::get(scope, snapshot, "self_slot")?;
    let self_slot = if self_slot.is_null_or_undefined() {
        cb::num(scope, 0.0)
    } else {
        self_slot
    };
    Ok(boolean(scope, target_index.strict_equals(self_slot)))
}

fn npc_targets_another_player<'s>(
    scope: &mut v8::HandleScope<'s>,
    instance: v8::Local<'s, v8::Value>,
    snap_fn: v8::Local<'s, v8::Value>,
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    let snapshot = cb::call(scope, snap_fn, undefined(scope), &[], "snap")?;
    let self_slot = cb::get(scope, snapshot, "self_slot")?;
    let self_slot = if self_slot.is_null_or_undefined() {
        cb::num(scope, 0.0)
    } else {
        self_slot
    };
    let row = entity_instance_snap(scope, instance)?;
    let target_kind = cb::get(scope, row, "target_kind")?;
    let player_target_kind = cb::num(scope, 2.0);
    if !target_kind.strict_equals(player_target_kind) {
        return Ok(boolean(scope, false));
    }
    let row = entity_instance_snap(scope, instance)?;
    let target_index = cb::get(scope, row, "target_index")?;
    Ok(boolean(scope, !target_index.strict_equals(self_slot)))
}

fn entity_tile<'s>(
    scope: &mut v8::HandleScope<'s>,
    instance: v8::Local<'s, v8::Value>,
    tile_ctor: v8::Local<'s, v8::Value>,
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    let row = entity_instance_snap(scope, instance)?;
    let x = cb::get(scope, row, "x")?;
    let row = entity_instance_snap(scope, instance)?;
    let z = cb::get(scope, row, "z")?;
    let row = entity_instance_snap(scope, instance)?;
    let level = cb::get(scope, row, "level")?;
    let level = if level.is_null_or_undefined() {
        cb::num(scope, 0.0)
    } else {
        level
    };
    let input = cb::object(scope, &[("x", x), ("z", z), ("level", level)])?;
    cb::call_method(scope, tile_ctor, "from", &[input], "Tile.from")
}

fn entity_distance<'s>(
    scope: &mut v8::HandleScope<'s>,
    instance: v8::Local<'s, v8::Value>,
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    let first = entity_snap_field(scope, instance, "distance")?;
    if first.is_number() {
        entity_snap_field(scope, instance, "distance")
    } else {
        Ok(cb::num(scope, 0.0))
    }
}

fn entity_actions<'s>(
    scope: &mut v8::HandleScope<'s>,
    instance: v8::Local<'s, v8::Value>,
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    let row = entity_instance_snap(scope, instance)?;
    let actions = cb::get(scope, row, "actions")?;
    present_ops(scope, actions)
}

fn player_combat_level<'s>(
    scope: &mut v8::HandleScope<'s>,
    instance: v8::Local<'s, v8::Value>,
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    let row = entity_instance_snap(scope, instance)?;
    let level = cb::get(scope, row, "combat_level")?;
    if level.is_number() {
        let integer = number_is_integer(scope, level)?;
        if cb::truthy(scope, integer) {
            return Ok(level);
        }
    }
    Ok(cb::num(scope, 0.0))
}

fn ground_item_count<'s>(
    scope: &mut v8::HandleScope<'s>,
    instance: v8::Local<'s, v8::Value>,
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    let first = entity_snap_field(scope, instance, "count")?;
    if first.is_number() {
        entity_snap_field(scope, instance, "count")
    } else {
        Ok(cb::num(scope, 1.0))
    }
}

fn npc_valid<'s>(
    scope: &mut v8::HandleScope<'s>,
    instance: v8::Local<'s, v8::Value>,
    snap_fn: v8::Local<'s, v8::Value>,
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    let snapshot = cb::call(scope, snap_fn, undefined(scope), &[], "snap")?;
    let npcs = cb::get(scope, snapshot, "npcs")?;
    let npcs = if cb::truthy(scope, npcs) {
        npcs
    } else {
        v8::Array::new(scope, 0).into()
    };
    let some = cb::get(scope, npcs, "some")?;
    let data = cb::object(scope, &[("instance", instance)])?;
    let predicate = build_callback(scope, npc_valid_predicate, data, "could not create NPC identity predicate")?;
    cb::call(scope, some, npcs, &[predicate], "npcs.some")
}

fn npc_valid_predicate<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue,
) {
    let result = (|| -> JsResult<'s, v8::Local<'s, v8::Value>> {
        let npc = args.get(0);
        if !cb::truthy(scope, npc) {
            return Ok(npc);
        }
        let index = cb::get(scope, npc, "index")?;
        let instance = cb::get(scope, args.data(), "instance")?;
        let row = entity_instance_snap(scope, instance)?;
        let own_index = cb::get(scope, row, "index")?;
        if !index.strict_equals(own_index) {
            return Ok(boolean(scope, false));
        }
        let name = cb::get(scope, npc, "name")?;
        let row = entity_instance_snap(scope, instance)?;
        let own_name = cb::get(scope, row, "name")?;
        Ok(boolean(scope, name.strict_equals(own_name)))
    })();
    cb::finish(scope, rv, result);
}

fn interaction_index<'s>(
    scope: &mut v8::HandleScope<'s>,
    instance: v8::Local<'s, v8::Value>,
    action: v8::Local<'s, v8::Value>,
) -> JsResult<'s, f64> {
    let row = entity_instance_snap(scope, instance)?;
    let actions = cb::get(scope, row, "actions")?;
    op_index_value(scope, actions, action)
}

fn queue_interaction<'s>(
    scope: &mut v8::HandleScope<'s>,
    queue: v8::Local<'s, v8::Value>,
    request: v8::Local<'s, v8::Value>,
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    cb::call(scope, queue, undefined(scope), &[request], "queue")
}

fn npc_interact<'s>(
    scope: &mut v8::HandleScope<'s>,
    instance: v8::Local<'s, v8::Value>,
    action: v8::Local<'s, v8::Value>,
    queue: v8::Local<'s, v8::Value>,
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    let slot = interaction_index(scope, instance, action)?;
    if slot == -1.0 {
        return Ok(boolean(scope, false));
    }
    let verb = string_call(scope, action)?;
    let global = global(scope);
    let selected = cb::get(scope, global.into(), "__rs2b0t_selected_facts")?;
    let row = entity_instance_snap(scope, instance)?;
    let id = cb::get(scope, row, "id")?;
    let key = v8::String::new(scope, "thieving-ready").unwrap_or_else(|| v8::String::empty(scope));
    let allowed = cb::call(
        scope,
        selected,
        global.into(),
        &[key.into(), id, verb],
        "globalThis.__rs2b0t_selected_facts",
    )?;
    if !cb::truthy(scope, allowed) {
        return Ok(boolean(scope, false));
    }
    let row = entity_instance_snap(scope, instance)?;
    let name = cb::get(scope, row, "name")?;
    let name = if name.is_null_or_undefined() {
        v8::String::new(scope, "").unwrap_or_else(|| v8::String::empty(scope)).into()
    } else {
        name
    };
    let row = entity_instance_snap(scope, instance)?;
    let index = cb::get(scope, row, "index")?;
    let op = cb::string(scope, "npc");
    let request = cb::object(scope, &[("op", op), ("name", name), ("action", verb), ("index", index)])?;
    queue_interaction(scope, queue, request)?;
    Ok(boolean(scope, true))
}

fn loc_interact<'s>(
    scope: &mut v8::HandleScope<'s>,
    instance: v8::Local<'s, v8::Value>,
    action: v8::Local<'s, v8::Value>,
    queue: v8::Local<'s, v8::Value>,
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    if interaction_index(scope, instance, action)? == -1.0 {
        return Ok(boolean(scope, false));
    }
    let row = entity_instance_snap(scope, instance)?;
    let x = cb::get(scope, row, "x")?;
    let row = entity_instance_snap(scope, instance)?;
    let z = cb::get(scope, row, "z")?;
    let row = entity_instance_snap(scope, instance)?;
    let level = cb::get(scope, row, "level")?;
    let level = if level.is_null_or_undefined() {
        cb::num(scope, 0.0)
    } else {
        level
    };
    let verb = string_call(scope, action)?;
    let op = cb::string(scope, "loc");
    let request = cb::object(scope, &[("op", op), ("x", x), ("z", z), ("level", level), ("action", verb)])?;
    let global = global(scope);
    let number = cb::get(scope, global.into(), "Number")?;
    let is_integer = cb::get(scope, number, "isInteger")?;
    let row = entity_instance_snap(scope, instance)?;
    let id = cb::get(scope, row, "id")?;
    let integer = cb::call(scope, is_integer, number, &[id], "Number.isInteger")?;
    if cb::truthy(scope, integer) {
        let key = cb::string(scope, "id");
        let created = cb::catching(scope, |s| object.set(s, key, id))?;
        if !created {
            return Err(cb::type_error(scope, "could not set interaction id"));
        }
    }
    queue_interaction(scope, queue, request)?;
    Ok(boolean(scope, true))
}

fn player_interact<'s>(
    scope: &mut v8::HandleScope<'s>,
    instance: v8::Local<'s, v8::Value>,
    action: v8::Local<'s, v8::Value>,
    queue: v8::Local<'s, v8::Value>,
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    if interaction_index(scope, instance, action)? == -1.0 {
        return Ok(boolean(scope, false));
    }
    let row = entity_instance_snap(scope, instance)?;
    let name = cb::get(scope, row, "name")?;
    let name = if name.is_null_or_undefined() {
        v8::String::new(scope, "").unwrap_or_else(|| v8::String::empty(scope)).into()
    } else {
        name
    };
    let verb = string_call(scope, action)?;
    let op = cb::string(scope, "player");
    let request = cb::object(scope, &[("op", op), ("name", name), ("action", verb)])?;
    queue_interaction(scope, queue, request)?;
    Ok(boolean(scope, true))
}

fn ground_item_interact<'s>(
    scope: &mut v8::HandleScope<'s>,
    instance: v8::Local<'s, v8::Value>,
    action: v8::Local<'s, v8::Value>,
    queue: v8::Local<'s, v8::Value>,
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    if interaction_index(scope, instance, action)? == -1.0 {
        return Ok(boolean(scope, false));
    }
    let row = entity_instance_snap(scope, instance)?;
    let x = cb::get(scope, row, "x")?;
    let row = entity_instance_snap(scope, instance)?;
    let z = cb::get(scope, row, "z")?;
    let row = entity_instance_snap(scope, instance)?;
    let level = cb::get(scope, row, "level")?;
    let level = if level.is_null_or_undefined() {
        cb::num(scope, 0.0)
    } else {
        level
    };
    let row = entity_instance_snap(scope, instance)?;
    let name = cb::get(scope, row, "name")?;
    let name = if name.is_null_or_undefined() {
        null(scope)
    } else {
        name
    };
    let verb = string_call(scope, action)?;
    let op = cb::string(scope, "obj");
    let request = cb::object(scope, &[("op", op), ("x", x), ("z", z), ("level", level), ("name", name), ("action", verb)])?;
    queue_interaction(scope, queue, request)?;
    Ok(boolean(scope, true))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustyscript::RuntimeOptions;

    const QUERY_SETUP: &str = r#"
const call = (operation, ...args) => __rs2b0t_entities_v8(operation, ...args);
const rows = [
    { name: 'Goblin', x: 3201, z: 3201, level: 0, distance: 1, actions: ['Attack'] },
];
class EntityQuery {
    constructor() {
        this.supplySnaps = () => rows;
        this.wrap = (row) => row;
        this.snapFilters = [];
        this.entityFilters = [];
    }
    forEachMatch(seen) {
        return call('query.forEachMatch', this, seen);
    }
    count() {
        return call('query.count', this);
    }
}
const query = new EntityQuery();
globalThis.checkQuery = () => query.count() === 1;
globalThis.checkPureJs = () => true;
"#;

    fn setup() -> Runtime {
        crate::load::platform::bootstrap_test_v8_platform();
        let mut runtime = Runtime::new(RuntimeOptions::default()).unwrap();
        cb::install_ops(&mut runtime).unwrap();
        install(&mut runtime).unwrap();
        runtime.eval::<()>(QUERY_SETUP).unwrap();
        runtime
    }

    fn function<'s>(
        scope: &mut v8::HandleScope<'s>,
        global: v8::Local<'s, v8::Object>,
        name: &str,
    ) -> v8::Local<'s, v8::Function> {
        let key = cb::string(scope, name);
        v8::Local::<v8::Function>::try_from(global.get(scope, key).unwrap()).unwrap()
    }

    #[test]
    fn entity_query_cold_bridge_charge_is_bounded_and_warm_heap_is_zero() {
        let mut runtime = setup();
        let context = runtime.deno_runtime().main_context();
        let mut scope = runtime.deno_runtime().handle_scope();
        let global = context.open(&mut scope).global(&mut scope);
        let query = function(&mut scope, global, "checkQuery");
        let receiver = v8::undefined(&mut scope).into();
        let cold = allocation_counter::measure(|| {
            assert!(query.call(&mut scope, receiver, &[]).unwrap().is_true());
        });
        let warm = allocation_counter::measure(|| {
            for _ in 0..1_000 {
                assert!(query.call(&mut scope, receiver, &[]).unwrap().is_true());
            }
        });
        println!("EntityQuery cold={cold:?}; warm1000={warm:?}");
        assert!(cold.count_total <= 3, "cold entity bridge allocations: {cold:?}");
        assert!(cold.count_current <= 3, "cold retained entity bridge allocations: {cold:?}");
        assert!(cold.count_max <= 3, "cold peak entity bridge allocations: {cold:?}");
        assert!(cold.bytes_total <= 336, "cold entity bridge bytes: {cold:?}");
        assert!(cold.bytes_current <= 336, "cold retained entity bridge bytes: {cold:?}");
        assert!(cold.bytes_max <= 336, "cold peak entity bridge bytes: {cold:?}");
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
        if args.get(0).is_true() {
            let fourth = &mut v8::TryCatch::new(third);
            rv.set(v8::Boolean::new(fourth, true).into());
        } else {
            rv.set(v8::Boolean::new(third, true).into());
        }
    }

    #[test]
    fn entity_query_cold_charge_reuses_independent_scope_cache_cell() {
        let mut runtime = setup();
        cb::install(&mut runtime, "__scope_cache_control", scope_cache_control).unwrap();
        runtime
            .eval::<()>(
                r#"
globalThis.checkThreeScopes = () => __scope_cache_control(false);
globalThis.checkFourScopes = () => __scope_cache_control(true);
"#,
            )
            .unwrap();
        let context = runtime.deno_runtime().main_context();
        let mut scope = runtime.deno_runtime().handle_scope();
        let global = context.open(&mut scope).global(&mut scope);
        let pure_js = function(&mut scope, global, "checkPureJs");
        let three_scopes = function(&mut scope, global, "checkThreeScopes");
        let four_scopes = function(&mut scope, global, "checkFourScopes");
        let query = function(&mut scope, global, "checkQuery");
        let receiver = v8::undefined(&mut scope).into();
        let pure = allocation_counter::measure(|| {
            assert!(pure_js.call(&mut scope, receiver, &[]).unwrap().is_true());
        });
        let cold = allocation_counter::measure(|| {
            assert!(three_scopes.call(&mut scope, receiver, &[]).unwrap().is_true());
        });
        let query_after_control = allocation_counter::measure(|| {
            assert!(query.call(&mut scope, receiver, &[]).unwrap().is_true());
        });
        let extra_depth = allocation_counter::measure(|| {
            assert!(four_scopes.call(&mut scope, receiver, &[]).unwrap().is_true());
        });
        let warm = allocation_counter::measure(|| {
            for _ in 0..1_000 {
                assert!(four_scopes.call(&mut scope, receiver, &[]).unwrap().is_true());
                assert!(query.call(&mut scope, receiver, &[]).unwrap().is_true());
            }
        });
        println!(
            "EntityQuery scope-cache control pure={pure:?}; three scopes={cold:?}; \
             query after control={query_after_control:?}; \
             extra depth={extra_depth:?}; warm1000={warm:?}"
        );
        assert_eq!(pure, allocation_counter::AllocationInfo::default());
        assert_eq!(cold.count_total, 1);
        assert_eq!(cold.count_current, 1);
        assert_eq!(cold.count_max, 1);
        assert_eq!(cold.bytes_total, 112);
        assert_eq!(cold.bytes_current, 112);
        assert_eq!(cold.bytes_max, 112);
        assert_eq!(
            query_after_control,
            allocation_counter::AllocationInfo::default()
        );
        assert_eq!(extra_depth.count_total, 1);
        assert_eq!(extra_depth.count_current, 1);
        assert_eq!(extra_depth.count_max, 1);
        assert_eq!(extra_depth.bytes_total, 112);
        assert_eq!(extra_depth.bytes_current, 112);
        assert_eq!(extra_depth.bytes_max, 112);
        assert_eq!(warm, allocation_counter::AllocationInfo::default());
    }
}
