//! Native V8 ownership for entity wrappers and `EntityQuery`.
//!
//! The shim keeps the public classes, aliases, proxies and raw `snap` fields.
//! Production collections select from paired native facts and original DTOs.
//! Caller-supplied queries and wrapper getters retain their public-row contract.
//! Actions still use the existing queue and selected-facts helpers.

use super::callback_v8::{self as cb, Flow, ForOf, JsResult, Poll};
use super::entity_pages;
use crate::scene_query::{EntityCategory, EntityFacts};
use rustyscript::Runtime;

pub(super) fn install(runtime: &mut Runtime) -> Result<(), String> {
    entity_pages::install(runtime)?;
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
    let operation = args.get(0);
    if !operation.is_int32() {
        return Err(cb::type_error(scope, "invalid entity operation"));
    }
    match operation.int32_value(scope) {
        Some(0) => entity_snap_view(scope, args.get(1)),
        Some(1) => present_ops(scope, args.get(1)),
        Some(2) => {
            let index = op_index_value(scope, args.get(1), args.get(2))?;
            Ok(cb::num(scope, index))
        }
        Some(3) => matches_entity_name(scope, args.get(1), args.get(2)),
        Some(4) => talk_op(scope, args.get(1)),
        Some(5) => entities_all(scope, args.get(1), args.get(2)),
        Some(6) => npcs_nearest(scope, args.get(1), args.get(2)),
        Some(7) => query_name(scope, args.get(1), args.get(2)),
        Some(8) => query_action(scope, args.get(1), args.get(2)),
        Some(9) => query_within(scope, args.get(1), args.get(2)),
        Some(10) => query_within_of(scope, args.get(1), args.get(2), args.get(3)),
        Some(11) => query_inside(scope, args.get(1), args.get(2)),
        Some(12) => push_filter(scope, args.get(1), "entityFilters", args.get(2)),
        Some(13) => query_for_each_match(scope, args.get(1), args.get(2)),
        Some(14) => query_results(scope, args.get(1)),
        Some(15) => query_nearest(scope, args.get(1)),
        Some(16) => query_first(scope, args.get(1)),
        Some(17) => query_exists(scope, args.get(1)),
        Some(18) => query_count(scope, args.get(1)),
        Some(19) => query_nearest_prefer_local(scope, args.get(1), args.get(2)),
        Some(20) => entity_name(scope, args.get(1)),
        Some(21) => entity_snap_field(scope, args.get(1), "id"),
        Some(22) => entity_snap_field(scope, args.get(1), "index"),
        Some(23) => entity_boolean_field(scope, args.get(1), "in_combat"),
        Some(24) => entity_snap_field(scope, args.get(1), "health"),
        Some(25) => npc_level(scope, args.get(1)),
        Some(26) => npc_size(scope, args.get(1)),
        Some(27) => npc_network_origin(scope, args.get(1), args.get(2)),
        Some(28) => npc_network_tile(scope, args.get(1), args.get(2)),
        Some(29) => entity_targets_me(scope, args.get(1), EntityCategory::Npcs),
        Some(30) => npc_targets_another_player(scope, args.get(1)),
        Some(31) => entity_tile(scope, args.get(1), args.get(2)),
        Some(32) => entity_distance(scope, args.get(1)),
        Some(33) => entity_actions(scope, args.get(1)),
        Some(34) => npc_valid(scope, args.get(1)),
        Some(35) => npc_interact(scope, args.get(1), args.get(2), args.get(3)),
        Some(36) => entity_name(scope, args.get(1)),
        Some(37) => entity_snap_field(scope, args.get(1), "id"),
        Some(38) => entity_tile(scope, args.get(1), args.get(2)),
        Some(39) => entity_distance(scope, args.get(1)),
        Some(40) => entity_actions(scope, args.get(1)),
        Some(41) => loc_interact(scope, args.get(1), args.get(2), args.get(3)),
        Some(42) => entity_name(scope, args.get(1)),
        Some(43) => entity_snap_field(scope, args.get(1), "index"),
        Some(44) => entity_boolean_field(scope, args.get(1), "in_combat"),
        Some(45) => player_combat_level(scope, args.get(1)),
        Some(46) => entity_targets_me(scope, args.get(1), EntityCategory::Players),
        Some(47) => entity_tile(scope, args.get(1), args.get(2)),
        Some(48) => entity_distance(scope, args.get(1)),
        Some(49) => entity_actions(scope, args.get(1)),
        Some(50) => player_interact(scope, args.get(1), args.get(2), args.get(3)),
        Some(51) => entity_name(scope, args.get(1)),
        Some(52) => entity_snap_field(scope, args.get(1), "id"),
        Some(53) => ground_item_count(scope, args.get(1)),
        Some(54) => entity_tile(scope, args.get(1), args.get(2)),
        Some(55) => entity_distance(scope, args.get(1)),
        Some(56) => entity_actions(scope, args.get(1)),
        Some(57) => ground_item_interact(scope, args.get(1), args.get(2), args.get(3)),
        Some(58) => production_query(scope, args.get(1), args.get(2), args.get(3)),
        _ => Err(cb::not_impl(scope, "entity binding")),
    }
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
    let receiver = undefined(scope);
    cb::call(scope, constructor, receiver, &[value], "String")
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

const QUERY_CATEGORY: &str = "rs2b0t.entity-query.category";
const FILTER_DATA: &str = "rs2b0t.entity-query.filter";
const RANK_DISTANCE: &str = "rs2b0t.entity-query.distance";

fn private_key<'s>(scope: &mut v8::HandleScope<'s>, name: &str) -> v8::Local<'s, v8::Private> {
    let name = v8::String::new(scope, name).unwrap_or_else(|| v8::String::empty(scope));
    v8::Private::for_api(scope, Some(name))
}

fn private_data<'s>(
    scope: &mut v8::HandleScope<'s>,
    value: v8::Local<'s, v8::Value>,
    name: &str,
) -> JsResult<'s, Option<v8::Local<'s, v8::Value>>> {
    let Ok(object) = v8::Local::<v8::Object>::try_from(value) else {
        return Ok(None);
    };
    let key = private_key(scope, name);
    let value = cb::catching(scope, |s| object.get_private(s, key))?;
    Ok((!value.is_undefined()).then_some(value))
}

fn set_private_data<'s>(
    scope: &mut v8::HandleScope<'s>,
    value: v8::Local<'s, v8::Value>,
    name: &str,
    data: v8::Local<'s, v8::Value>,
) -> JsResult<'s, ()> {
    let Ok(object) = v8::Local::<v8::Object>::try_from(value) else {
        return Ok(());
    };
    let key = private_key(scope, name);
    if cb::catching(scope, |s| object.set_private(s, key, data))? {
        Ok(())
    } else {
        Err(cb::type_error(
            scope,
            "could not retain entity query metadata",
        ))
    }
}

fn category<'s>(
    scope: &mut v8::HandleScope<'s>,
    value: v8::Local<'s, v8::Value>,
) -> JsResult<'s, EntityCategory> {
    if value.is_int32() {
        match value.int32_value(scope) {
            Some(0) => return Ok(EntityCategory::Npcs),
            Some(1) => return Ok(EntityCategory::Players),
            Some(2) => return Ok(EntityCategory::Locs),
            Some(3) => return Ok(EntityCategory::Ground),
            _ => {}
        }
    }
    Err(cb::type_error(scope, "invalid entity collection"))
}

fn production_query<'s>(
    scope: &mut v8::HandleScope<'s>,
    collection: v8::Local<'s, v8::Value>,
    query_constructor: v8::Local<'s, v8::Value>,
    wrapper_constructor: v8::Local<'s, v8::Value>,
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    category(scope, collection)?;
    let supplier = build_callback(
        scope,
        supply_native_rows,
        collection,
        "could not create native entity supplier",
    )?;
    let data = data_object(scope, "constructor", wrapper_constructor)?;
    let wrapper = v8::Function::builder(wrap_entity_row)
        .data(data)
        .length(1)
        .build(scope)
        .ok_or_else(|| cb::type_error(scope, "could not create native entity wrapper"))?;
    let query = cb::construct(
        scope,
        query_constructor,
        &[supplier, wrapper.into()],
        "EntityQuery",
    )?;
    set_private_data(scope, query, QUERY_CATEGORY, collection)?;
    Ok(query)
}

fn supply_native_rows<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue,
) {
    let result = native_rows(scope, args.data(), None);
    cb::finish(scope, rv, result);
}

fn native_rows<'s>(
    scope: &mut v8::HandleScope<'s>,
    collection: v8::Local<'s, v8::Value>,
    constructor: Option<v8::Local<'s, v8::Value>>,
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    let category = category(scope, collection)?;
    let state = entity_pages::state(scope)
        .ok_or_else(|| cb::type_error(scope, "native entity pages unavailable"))?;
    let pages = state.borrow();
    let output = v8::Array::new(scope, 0);
    let poll = Poll::default();
    for (index, original) in pages.rows(category).iter().enumerate() {
        let scope = &mut v8::EscapableHandleScope::new(scope);
        let flow = (|| -> JsResult<'_, Flow<'_>> {
            poll.check(scope)?;
            let Some(facts) = pages.generation.facts(category, index) else {
                return Ok(Flow::Continue(None));
            };
            let row = v8::Local::new(scope, original);
            let value = if let Some(constructor) = constructor {
                let wrapper =
                    cb::construct(scope, constructor, &[row.into()], "entity constructor")?;
                let distance = cb::num(scope, f64::from(facts.distance()));
                set_private_data(scope, wrapper, RANK_DISTANCE, distance)?;
                wrapper
            } else {
                row.into()
            };
            cb::catching(scope, |s| output.set_index(s, index as u32, value))?;
            Ok(Flow::Continue(None))
        })();
        cb::iteration(scope, flow)?;
    }
    Ok(output.into())
}

fn build_filter<'s>(
    scope: &mut v8::HandleScope<'s>,
    callback: impl v8::MapFnTo<v8::FunctionCallback>,
    kind: i32,
    data: v8::Local<'s, v8::Value>,
    message: &str,
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    let kind = cb::num(scope, f64::from(kind));
    set_data(scope, data, "kind", kind)?;
    let predicate = build_callback(scope, callback, data, message)?;
    set_private_data(scope, predicate, FILTER_DATA, data)?;
    Ok(predicate)
}

fn ranking_distance<'s>(
    scope: &mut v8::HandleScope<'s>,
    entity: v8::Local<'s, v8::Value>,
    native: bool,
    label: &str,
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    if native {
        if let Some(distance) = private_data(scope, entity, RANK_DISTANCE)? {
            return Ok(distance);
        }
    }
    cb::call_method(scope, entity, "distance", &[], label)
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
    let source = row
        .to_object(scope)
        .ok_or_else(|| cb::type_error(scope, "Cannot convert undefined or null to object"))?;
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
    let value = cb::catching(scope, |s| source.get(s, key))?;
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
    cb::call_method(
        scope,
        actions,
        "filter",
        &[predicate.into()],
        "actions.filter",
    )
}

fn present_ops_predicate<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue,
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
        let object = actions
            .to_object(scope)
            .ok_or_else(|| cb::type_error(scope, "Cannot convert undefined or null to object"))?;
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
            return Err(cb::type_error(
                scope,
                "could not create talk action regular expression",
            ));
        };
        let test = cb::get(scope, regex.into(), "test")?;
        let text = string_call(scope, args.get(0))?;
        cb::call(scope, test, regex.into(), &[text], "talk action test")
    })();
    cb::finish(scope, rv, result);
}

fn entities_all<'s>(
    scope: &mut v8::HandleScope<'s>,
    collection: v8::Local<'s, v8::Value>,
    constructor: v8::Local<'s, v8::Value>,
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    native_rows(scope, collection, Some(constructor))
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
        let left = ranking_distance(scope, args.get(0), true, "a.distance")?;
        let right = ranking_distance(scope, args.get(1), true, "b.distance")?;
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
    let predicate = build_filter(
        scope,
        query_name_predicate,
        1,
        data,
        "could not create name filter",
    )?;
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
    let predicate = build_filter(
        scope,
        query_action_predicate,
        2,
        data,
        "could not create action filter",
    )?;
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
    let predicate = build_filter(
        scope,
        query_within_predicate,
        3,
        data,
        "could not create distance filter",
    )?;
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
    let predicate = build_filter(
        scope,
        query_within_of_predicate,
        4,
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
    let predicate = build_filter(
        scope,
        query_inside_predicate,
        5,
        data,
        "could not create area filter",
    )?;
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
    let predicate = build_callback(
        scope,
        invoke_filter,
        data,
        "could not create filter callback",
    )?;
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
        let receiver = undefined(scope);
        cb::call(scope, args.get(0), receiver, &[view], "f")
    })();
    cb::finish(scope, rv, result);
}

fn query_for_each_match<'s>(
    scope: &mut v8::HandleScope<'s>,
    query: v8::Local<'s, v8::Value>,
    seen: v8::Local<'s, v8::Value>,
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    if let Some(collection) = private_data(scope, query, QUERY_CATEGORY)? {
        return native_query_for_each_match(scope, query, collection, seen);
    }
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

fn native_query_for_each_match<'s>(
    scope: &mut v8::HandleScope<'s>,
    query: v8::Local<'s, v8::Value>,
    collection: v8::Local<'s, v8::Value>,
    seen: v8::Local<'s, v8::Value>,
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    let category = category(scope, collection)?;
    let state = entity_pages::state(scope)
        .ok_or_else(|| cb::type_error(scope, "native entity pages unavailable"))?;
    let pages = state.borrow();
    let poll = Poll::default();
    for (index, original) in pages.rows(category).iter().enumerate() {
        let scope = &mut v8::EscapableHandleScope::new(scope);
        let flow = (|| -> JsResult<'_, Flow<'_>> {
            poll.check(scope)?;
            let Some(facts) = pages.generation.facts(category, index) else {
                return Ok(Flow::Continue(None));
            };
            let raw = v8::Local::new(scope, original).into();
            if !native_filter_array_passes(scope, query, raw, collection, index)? {
                return Ok(Flow::Continue(None));
            }
            let entity = cb::call_method(scope, query, "wrap", &[raw], "this.wrap")?;
            let distance = cb::num(scope, f64::from(facts.distance()));
            set_private_data(scope, entity, RANK_DISTANCE, distance)?;
            if !filter_array_passes(scope, query, "entityFilters", entity)? {
                return Ok(Flow::Continue(None));
            }
            let receiver = undefined(scope);
            let result = cb::call(scope, seen, receiver, &[entity], "seen")?;
            if cb::truthy(scope, result) {
                Ok(Flow::Break(None))
            } else {
                Ok(Flow::Continue(None))
            }
        })();
        if let Flow::Break(_) = cb::iteration(scope, flow)? {
            break;
        }
    }
    Ok(undefined(scope))
}

fn native_filter_array_passes<'s>(
    scope: &mut v8::HandleScope<'s>,
    query: v8::Local<'s, v8::Value>,
    original: v8::Local<'s, v8::Value>,
    collection: v8::Local<'s, v8::Value>,
    index: usize,
) -> JsResult<'s, bool> {
    let first = cb::get(scope, query, "snapFilters")?;
    let length = cb::get(scope, first, "length")?;
    let zero = cb::num(scope, 0.0);
    if !cb::gt(scope, length, zero)? {
        return Ok(true);
    }
    let filters = cb::get(scope, query, "snapFilters")?;
    let every = cb::get(scope, filters, "every")?;
    let index = cb::num(scope, index as f64);
    let data = cb::object(
        scope,
        &[
            ("original", original),
            ("collection", collection),
            ("index", index),
        ],
    )?;
    let predicate = build_callback(
        scope,
        invoke_native_filter,
        data,
        "could not create native filter callback",
    )?;
    let result = cb::call(
        scope,
        every,
        filters,
        &[predicate],
        "this.snapFilters.every",
    )?;
    Ok(cb::truthy(scope, result))
}

fn invoke_native_filter<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue,
) {
    let result = (|| -> JsResult<'s, v8::Local<'s, v8::Value>> {
        let predicate = args.get(0);
        let data = args.data();
        if let Some(filter) = private_data(scope, predicate, FILTER_DATA)? {
            let collection = cb::get(scope, data, "collection")?;
            let category = category(scope, collection)?;
            let index = cb::get(scope, data, "index")?;
            let index = index
                .uint32_value(scope)
                .ok_or_else(|| cb::type_error(scope, "invalid native entity index"))?
                as usize;
            let state = entity_pages::state(scope)
                .ok_or_else(|| cb::type_error(scope, "native entity pages unavailable"))?;
            let pages = state.borrow();
            let facts = pages
                .generation
                .facts(category, index)
                .ok_or_else(|| cb::type_error(scope, "native entity facts unavailable"))?;
            return native_filter_matches(scope, filter, facts);
        }
        // Explicit caller filters retain their normalized public-row argument.
        // Built-in filters never construct or read that echoed view.
        let mut view = cb::get(scope, data, "view")?;
        if view.is_undefined() {
            let original = cb::get(scope, data, "original")?;
            view = entity_snap_view(scope, original)?;
            set_data(scope, data, "view", view)?;
        }
        let receiver = undefined(scope);
        cb::call(scope, predicate, receiver, &[view], "f")
    })();
    cb::finish(scope, rv, result);
}

fn native_filter_matches<'s>(
    scope: &mut v8::HandleScope<'s>,
    filter: v8::Local<'s, v8::Value>,
    facts: EntityFacts<'_>,
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    let kind = cb::get(scope, filter, "kind")?;
    match kind.int32_value(scope) {
        Some(1) => {
            let Some(name) = facts.name() else {
                return Ok(boolean(scope, false));
            };
            let name = cb::string(scope, name);
            let name = string_call(scope, name)?;
            let name = cb::call_method(scope, name, "trim", &[], "name.trim")?;
            let name = lower_case(scope, name)?;
            let wanted = cb::get(scope, filter, "wanted")?;
            cb::call_method(scope, wanted, "includes", &[name], "wanted.includes")
        }
        Some(2) => {
            let wanted = cb::get(scope, filter, "wanted")?;
            for action in facts.actions() {
                if &**action == "hidden" {
                    continue;
                }
                let action = cb::string(scope, action);
                let action = string_call(scope, action)?;
                let action = lower_case(scope, action)?;
                if action.strict_equals(wanted) {
                    return Ok(boolean(scope, true));
                }
            }
            Ok(boolean(scope, false))
        }
        Some(3) => {
            let distance = cb::num(scope, f64::from(facts.distance()));
            let limit = cb::get(scope, filter, "distance")?;
            let within = cb::le(scope, distance, limit)?;
            Ok(boolean(scope, within))
        }
        Some(4) => {
            let origin = cb::get(scope, filter, "origin")?;
            let radius = cb::get(scope, filter, "radius")?;
            let distance = if cb::truthy(scope, origin) {
                super::distance::planar_from_posted(scope, facts.tile(), origin)?
            } else {
                cb::num(scope, f64::INFINITY)
            };
            let within = cb::le(scope, distance, radius)?;
            Ok(boolean(scope, within))
        }
        Some(5) => {
            let tile = facts.tile();
            let area = cb::get(scope, filter, "area")?;
            let x = cb::num(scope, f64::from(tile.x));
            let min_x = cb::get(scope, area, "minX")?;
            if !cb::ge(scope, x, min_x)? {
                return Ok(boolean(scope, false));
            }
            let max_x = cb::get(scope, area, "maxX")?;
            if !cb::le(scope, x, max_x)? {
                return Ok(boolean(scope, false));
            }
            let z = cb::num(scope, f64::from(tile.z));
            let min_z = cb::get(scope, area, "minZ")?;
            if !cb::ge(scope, z, min_z)? {
                return Ok(boolean(scope, false));
            }
            let max_z = cb::get(scope, area, "maxZ")?;
            let inside = cb::le(scope, z, max_z)?;
            Ok(boolean(scope, inside))
        }
        _ => Err(cb::type_error(scope, "invalid native entity filter")),
    }
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
        let receiver = undefined(scope);
        let result = cb::call(scope, seen, receiver, &[entity], "seen")?;
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
    let callback = build_callback(
        scope,
        collect_result,
        data,
        "could not create query collector",
    )?;
    cb::call_method(
        scope,
        query,
        "forEachMatch",
        &[callback],
        "this.forEachMatch",
    )?;
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
    let empty = null(scope);
    let data = data_object(scope, "best", empty)?;
    let native = private_data(scope, query, QUERY_CATEGORY)?.is_some();
    let native = boolean(scope, native);
    set_data(scope, data, "native", native)?;
    let callback = build_callback(
        scope,
        rank_nearest,
        data,
        "could not create nearest callback",
    )?;
    cb::call_method(
        scope,
        query,
        "forEachMatch",
        &[callback],
        "this.forEachMatch",
    )?;
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
        Err(cb::type_error(
            scope,
            "could not update query callback state",
        ))
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
            let native = cb::get(scope, data, "native")?.is_true();
            let distance = ranking_distance(scope, entity, native, "e.distance")?;
            let best_distance = ranking_distance(scope, best, native, "best.distance")?;
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
    let empty = null(scope);
    let data = data_object(scope, "hit", empty)?;
    let callback = build_callback(
        scope,
        capture_first,
        data,
        "could not create first callback",
    )?;
    cb::call_method(
        scope,
        query,
        "forEachMatch",
        &[callback],
        "this.forEachMatch",
    )?;
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
    cb::call_method(
        scope,
        query,
        "forEachMatch",
        &[callback],
        "this.forEachMatch",
    )?;
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
    let native = private_data(scope, query, QUERY_CATEGORY)?.is_some();
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
        let native = boolean(scope, native);
        set_data(scope, data, "native", native)?;
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
    rank_result_pool(scope, pool, native)
}

fn local_distance_predicate<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue,
) {
    let result = (|| -> JsResult<'s, v8::Local<'s, v8::Value>> {
        let entity = args.get(0);
        let native = cb::get(scope, args.data(), "native")?.is_true();
        let distance = ranking_distance(scope, entity, native, "e.distance")?;
        let radius = cb::get(scope, args.data(), "radius")?;
        let within = cb::le(scope, distance, radius)?;
        Ok(boolean(scope, within))
    })();
    cb::finish(scope, rv, result);
}

fn rank_result_pool<'s>(
    scope: &mut v8::HandleScope<'s>,
    pool: v8::Local<'s, v8::Value>,
    native: bool,
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    let pool = ForOf::open(scope, pool, "pool")?;
    let mut best = null(scope);
    loop {
        let scope = &mut v8::EscapableHandleScope::new(scope);
        let flow = rank_pool_iteration(scope, &pool, best, native);
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
    native: bool,
) -> JsResult<'s, Flow<'s>> {
    let Some(entity) = pool.step(scope)? else {
        return Ok(Flow::Break(Some(best)));
    };
    let body = (|| -> JsResult<'s, Option<v8::Local<'s, v8::Value>>> {
        if !cb::truthy(scope, best) {
            return Ok(Some(entity));
        }
        let distance = ranking_distance(scope, entity, native, "e.distance")?;
        let best_distance = ranking_distance(scope, best, native, "best.distance")?;
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

fn target_values<'s>(
    scope: &mut v8::HandleScope<'s>,
    instance: v8::Local<'s, v8::Value>,
    category: EntityCategory,
) -> JsResult<'s, (v8::Local<'s, v8::Value>, v8::Local<'s, v8::Value>, i32)> {
    let row = entity_instance_snap(scope, instance)?;
    let state = entity_pages::state(scope)
        .ok_or_else(|| cb::type_error(scope, "native entity pages unavailable"))?;
    let pages = state.borrow();
    let self_slot = pages.generation.self_slot;
    for (index, original) in pages.rows(category).iter().enumerate() {
        if row.strict_equals(v8::Local::new(scope, original).into()) {
            if let Some((kind, index)) = pages
                .generation
                .facts(category, index)
                .and_then(EntityFacts::target)
            {
                let kind = cb::num(scope, f64::from(kind));
                let index = cb::num(scope, f64::from(index));
                return Ok((kind, index, self_slot));
            }
        }
    }
    // A wrapper explicitly constructed from caller data keeps that argument.
    // Only the current local-player identity comes from the native generation.
    let kind = cb::get(scope, row, "target_kind")?;
    let player = cb::num(scope, 2.0);
    let index = if kind.strict_equals(player) {
        let row = entity_instance_snap(scope, instance)?;
        cb::get(scope, row, "target_index")?
    } else {
        undefined(scope)
    };
    Ok((kind, index, self_slot))
}

fn entity_targets_me<'s>(
    scope: &mut v8::HandleScope<'s>,
    instance: v8::Local<'s, v8::Value>,
    category: EntityCategory,
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    let (kind, index, self_slot) = target_values(scope, instance, category)?;
    let player = cb::num(scope, 2.0);
    let self_slot = cb::num(scope, f64::from(self_slot));
    Ok(boolean(
        scope,
        kind.strict_equals(player) && index.strict_equals(self_slot),
    ))
}

fn npc_targets_another_player<'s>(
    scope: &mut v8::HandleScope<'s>,
    instance: v8::Local<'s, v8::Value>,
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    let (kind, index, self_slot) = target_values(scope, instance, EntityCategory::Npcs)?;
    let player = cb::num(scope, 2.0);
    let self_slot = cb::num(scope, f64::from(self_slot));
    Ok(boolean(
        scope,
        kind.strict_equals(player) && !index.strict_equals(self_slot),
    ))
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
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    let state = entity_pages::state(scope)
        .ok_or_else(|| cb::type_error(scope, "native entity pages unavailable"))?;
    let pages = state.borrow();
    let Some(npcs) = pages.generation.npcs.as_ref() else {
        return Ok(boolean(scope, false));
    };
    if npcs.is_empty() {
        return Ok(boolean(scope, false));
    }
    let own_row = entity_instance_snap(scope, instance)?;
    if pages.npcs.iter().enumerate().any(|(index, original)| {
        index < npcs.len() && own_row.strict_equals(v8::Local::new(scope, original).into())
    }) {
        return Ok(boolean(scope, true));
    }
    for (index, npc) in npcs.iter().enumerate() {
        let own_row = if index == 0 {
            own_row
        } else {
            entity_instance_snap(scope, instance)?
        };
        let own_index = cb::get(scope, own_row, "index")?;
        let native_index = cb::num(scope, f64::from(npc.index));
        if !native_index.strict_equals(own_index) {
            continue;
        }
        let name = match npc.name.as_deref() {
            Some(name) => cb::string(scope, name),
            None => null(scope),
        };
        let own_row = entity_instance_snap(scope, instance)?;
        let own_name = cb::get(scope, own_row, "name")?;
        if name.strict_equals(own_name) {
            return Ok(boolean(scope, true));
        }
    }
    Ok(boolean(scope, false))
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
    let receiver = undefined(scope);
    cb::call(scope, queue, receiver, &[request], "queue")
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
        v8::String::new(scope, "")
            .unwrap_or_else(|| v8::String::empty(scope))
            .into()
    } else {
        name
    };
    let row = entity_instance_snap(scope, instance)?;
    let index = cb::get(scope, row, "index")?;
    let op = cb::string(scope, "npc");
    let request = cb::object(
        scope,
        &[
            ("op", op),
            ("name", name),
            ("action", verb),
            ("index", index),
        ],
    )?;
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
    let request = cb::object(
        scope,
        &[
            ("op", op),
            ("x", x),
            ("z", z),
            ("level", level),
            ("action", verb),
        ],
    )?;
    let global = global(scope);
    let number = cb::get(scope, global.into(), "Number")?;
    let is_integer = cb::get(scope, number, "isInteger")?;
    let row = entity_instance_snap(scope, instance)?;
    let id = cb::get(scope, row, "id")?;
    let integer = cb::call(scope, is_integer, number, &[id], "Number.isInteger")?;
    if cb::truthy(scope, integer) {
        let key = cb::string(scope, "id");
        cb::assign(scope, request, key, id)?;
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
        v8::String::new(scope, "")
            .unwrap_or_else(|| v8::String::empty(scope))
            .into()
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
    let request = cb::object(
        scope,
        &[
            ("op", op),
            ("x", x),
            ("z", z),
            ("level", level),
            ("name", name),
            ("action", verb),
        ],
    )?;
    queue_interaction(scope, queue, request)?;
    Ok(boolean(scope, true))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustyscript::{Module, RuntimeOptions};

    const QUERY_SETUP: &str = r#"
import EntityQuery, { matchesEntityName } from '/rs2b0t/bot/api/query/EntityQuery.js';
const row = {
    name: 'Goblin', x: 3201, z: 3201, level: 0, distance: 1, actions: ['Attack'],
};
const rows = [row];
const entity = { snap: row, distance() { return row.distance; } };
globalThis.checkQuery = () => {
    const query = EntityQuery.fromSnapshots(() => rows, () => entity)
        .name('Goblin').action('Attack').within(2)
        .withinOf({ x: 3200, z: 3200, level: 0 }, 2)
        .inside({ minX: 3200, maxX: 3202, minZ: 3200, maxZ: 3202 })
        .where(value => value === entity);
    let seen = 0;
    query.forEachMatch(value => { seen += value === entity; return false; });
    return matchesEntityName(' Goblin ', 'goblin') &&
        query.results()[0] === entity && query.nearest() === entity &&
        query.first() === entity && query.exists() && query.count() === 1 &&
        query.nearestPreferLocal(2) === entity && seen === 1;
};
globalThis.checkPureJs = () => true;
// This caller-provided constructor exercises the production collection adapter
// without loading interaction/geometry modules into the allocator fixture.
class AllocationEntity {
    constructor(snap) { this.snap = snap; }
    distance() { return __rs2b0t_entities_v8(32, this); }
}
globalThis.checkNativeQuery = () => {
    const keys = ['npcs', 'players', 'locs', 'ground'];
    for (let category = 0; category < keys.length; category++) {
        const rows = __rs2b0t_host.snapshot[keys[category]];
        const query = __rs2b0t_entities_v8(58, category, EntityQuery, AllocationEntity)
            .name('Goblin').action('Attack').within(4)
            .withinOf({ x: 3200, z: 3200, level: 3 }, 2)
            .inside({ minX: 3200, maxX: 3202, minZ: 3200, maxZ: 3202 })
            .where(value => value.snap.name === 'Goblin');
        let seen = 0;
        query.forEachMatch(value => { seen += value.snap.name === 'Goblin'; return false; });
        if (query.results().length !== 2 || query.first().snap !== rows[0] ||
            query.nearest().snap !== rows[1] || !query.exists() ||
            query.count() !== 2 || query.nearestPreferLocal(2).snap !== rows[1] ||
            seen !== 2 || __rs2b0t_entities_v8(5, category, AllocationEntity).length !== 2)
            return false;
    }
    const npc = new AllocationEntity(__rs2b0t_host.snapshot.npcs[0]);
    const player = new AllocationEntity(__rs2b0t_host.snapshot.players[0]);
    return __rs2b0t_entities_v8(34, npc) &&
        __rs2b0t_entities_v8(29, npc) && __rs2b0t_entities_v8(46, player);
};
"#;

    fn setup() -> Runtime {
        crate::load::platform::bootstrap_test_v8_platform();
        let mut runtime = Runtime::new(RuntimeOptions::default()).unwrap();
        cb::install_ops(&mut runtime).unwrap();
        super::super::distance::install(&mut runtime).unwrap();
        install(&mut runtime).unwrap();
        let query = Module::new(
            "/rs2b0t/bot/api/query/EntityQuery.js",
            include_str!("../shim/query.js"),
        );
        let entry = Module::new("/entity_query_allocation_setup.js", QUERY_SETUP);
        runtime.load_modules(&entry, vec![&query]).unwrap();
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

    fn entity_post_bytes() -> Vec<u8> {
        use crate::isolate_fb::generated::rs_2b_0t::isolate::{
            SceneEntity, SceneEntityArgs, Snapshot, SnapshotArgs,
        };
        let mut builder = flatbuffers::FlatBufferBuilder::new();
        let name = builder.create_string("Goblin");
        let action = builder.create_string("Attack");
        let actions = builder.create_vector(&[action]);
        let rows = [3, 1].map(|distance| {
            SceneEntity::create(
                &mut builder,
                &SceneEntityArgs {
                    index: distance + 1,
                    id: 123,
                    name: Some(name),
                    x: 3201,
                    z: 3201,
                    distance,
                    actions: Some(actions),
                    target_kind: 2,
                    target_index: 1,
                    ..SceneEntityArgs::default()
                },
            )
        });
        let rows = builder.create_vector(&rows);
        let snapshot = Snapshot::create(
            &mut builder,
            &SnapshotArgs {
                tick: 1,
                ingame: true,
                self_slot: 1,
                npcs: Some(rows),
                players: Some(rows),
                locs: Some(rows),
                ground: Some(rows),
                ..SnapshotArgs::default()
            },
        );
        builder.finish(snapshot, None);
        builder.finished_data().to_vec()
    }

    fn post_entities(runtime: &mut Runtime, snapshot: &crate::isolate_fb::Snapshot<'_>) {
        crate::observed::apply(snapshot);
        super::super::snapshot::materialize_snapshot(runtime, snapshot, false).unwrap();
    }

    /// A cold native post with `count` NPC rows (25-field shape) and `count`
    /// loc rows (23-field shape). No JavaScript query runs at any point.
    fn entity_growth_post_bytes(tick: u64, count: usize) -> Vec<u8> {
        use crate::isolate_fb::generated::rs_2b_0t::isolate::{
            SceneEntity, SceneEntityArgs, Snapshot, SnapshotArgs,
        };
        let mut builder = flatbuffers::FlatBufferBuilder::new();
        let name = builder.create_string("Goblin");
        let action = builder.create_string("Attack");
        let actions = builder.create_vector(&[action]);
        let npcs: Vec<_> = (0..count)
            .map(|index| {
                SceneEntity::create(
                    &mut builder,
                    &SceneEntityArgs {
                        index: index as i32,
                        id: 123,
                        name: Some(name),
                        x: 3200,
                        z: 3200,
                        level: 0,
                        distance: 1,
                        actions: Some(actions),
                        size: 1,
                        nx: 3200,
                        nz: 3200,
                        ..SceneEntityArgs::default()
                    },
                )
            })
            .collect();
        let npcs = builder.create_vector(&npcs);
        let locs: Vec<_> = (0..count)
            .map(|index| {
                SceneEntity::create(
                    &mut builder,
                    &SceneEntityArgs {
                        index: index as i32,
                        id: 456,
                        name: Some(name),
                        x: 3300,
                        z: 3300,
                        level: 0,
                        distance: 1,
                        actions: Some(actions),
                        ..SceneEntityArgs::default()
                    },
                )
            })
            .collect();
        let locs = builder.create_vector(&locs);
        let snapshot = Snapshot::create(
            &mut builder,
            &SnapshotArgs {
                tick,
                ingame: true,
                self_slot: 1,
                npcs: Some(npcs),
                locs: Some(locs),
                ..SnapshotArgs::default()
            },
        );
        builder.finish(snapshot, None);
        builder.finished_data().to_vec()
    }

    /// Collected V8 used heap, not Rust allocator counters: the property
    /// backing stores this test guards are invisible to `allocation_counter`.
    fn collected_used_heap(runtime: &mut Runtime) -> usize {
        let isolate = runtime.deno_runtime().v8_isolate();
        isolate.low_memory_notification();
        isolate.get_heap_statistics().used_heap_size()
    }

    #[test]
    fn cold_native_entity_rows_bound_collected_v8_heap_growth() {
        // 512 bytes x 8,064 extra rows + 65,536: 901,152 bytes above the
        // measured fast reference's 3,293,152-byte growth, and well below
        // the dictionary path's 14,451,728.
        const MAX_GROWTH: usize = 4_194_304;
        let mut runtime = setup();
        super::super::line_of_sight::install(&mut runtime).unwrap();
        runtime
            .eval::<()>("globalThis.__rs2b0t_host = {};")
            .unwrap();
        crate::observed::on_reset();

        let small_bytes = entity_growth_post_bytes(1, 64);
        let small = crate::isolate_fb::Snapshot::from_bytes(&small_bytes).unwrap();
        post_entities(&mut runtime, &small);
        let baseline = collected_used_heap(&mut runtime);

        let large_bytes = entity_growth_post_bytes(2, 4_096);
        let large = crate::isolate_fb::Snapshot::from_bytes(&large_bytes).unwrap();
        post_entities(&mut runtime, &large);
        let after_large = collected_used_heap(&mut runtime);
        let growth = after_large.saturating_sub(baseline);

        // Replace the large generation again so an uncollected previous
        // generation cannot be charged as retained live row storage.
        let replacement_bytes = entity_growth_post_bytes(3, 4_096);
        let replacement = crate::isolate_fb::Snapshot::from_bytes(&replacement_bytes).unwrap();
        post_entities(&mut runtime, &replacement);
        let after_replacement = collected_used_heap(&mut runtime);
        let repeated_growth = after_replacement.saturating_sub(baseline);

        println!(
            "Cold entity row collected V8 used heap: baseline={baseline} \
             large={after_large} growth={growth} replacement={after_replacement} \
             repeated-growth={repeated_growth}"
        );
        assert!(
            growth <= MAX_GROWTH,
            "cold entity rows grew the collected V8 used heap by {growth} bytes; \
             limit {MAX_GROWTH} (baseline {baseline}, large {after_large})"
        );
        assert!(
            repeated_growth <= MAX_GROWTH,
            "repeated cold entity rows grew the collected V8 used heap by \
             {repeated_growth} bytes; limit {MAX_GROWTH} (baseline {baseline}, \
             replacement {after_replacement})"
        );
    }

    #[test]
    fn production_entity_queries_allocate_no_rust_heap_after_warmup() {
        let mut runtime = setup();
        super::super::line_of_sight::install(&mut runtime).unwrap();
        runtime
            .eval::<()>("globalThis.__rs2b0t_host = {};")
            .unwrap();
        crate::observed::on_reset();
        let bytes = entity_post_bytes();
        let snapshot = crate::isolate_fb::Snapshot::from_bytes(&bytes).unwrap();
        post_entities(&mut runtime, &snapshot);
        let context = runtime.deno_runtime().main_context();
        let mut scope = runtime.deno_runtime().handle_scope();
        let global = context.open(&mut scope).global(&mut scope);
        let query = function(&mut scope, global, "checkNativeQuery");
        let receiver = v8::undefined(&mut scope).into();
        let cold = allocation_counter::measure(|| {
            assert!(query.call(&mut scope, receiver, &[]).unwrap().is_true());
        });
        let warm = allocation_counter::measure(|| {
            for _ in 0..1_000 {
                assert!(query.call(&mut scope, receiver, &[]).unwrap().is_true());
            }
        });
        println!("Production EntityQuery cold={cold:?}; warm1000={warm:?}");
        assert!(cold.count_total <= 6, "native cold bridge charge: {cold:?}");
        assert!(
            cold.count_current <= 6,
            "native retained bridge charge: {cold:?}"
        );
        assert!(cold.count_max <= 6, "native peak bridge charge: {cold:?}");
        assert!(
            cold.bytes_total <= 672,
            "native cold bridge bytes: {cold:?}"
        );
        assert!(
            cold.bytes_current <= 672,
            "native retained bridge bytes: {cold:?}"
        );
        assert!(cold.bytes_max <= 672, "native peak bridge bytes: {cold:?}");
        assert_eq!(warm, allocation_counter::AllocationInfo::default());
    }

    #[test]
    fn production_query_cold_charge_reuses_independent_scope_cache() {
        let mut runtime = setup();
        super::super::line_of_sight::install(&mut runtime).unwrap();
        cb::install(&mut runtime, "__scope_cache_control", scope_cache_control).unwrap();
        runtime
            .eval::<()>(
                r#"
globalThis.__rs2b0t_host = {};
globalThis.checkTwelveScopes = () => __scope_cache_control(false);
globalThis.checkThirteenScopes = () => __scope_cache_control(true);
"#,
            )
            .unwrap();
        crate::observed::on_reset();
        let bytes = entity_post_bytes();
        let snapshot = crate::isolate_fb::Snapshot::from_bytes(&bytes).unwrap();
        post_entities(&mut runtime, &snapshot);
        let context = runtime.deno_runtime().main_context();
        let mut scope = runtime.deno_runtime().handle_scope();
        let global = context.open(&mut scope).global(&mut scope);
        let pure_js = function(&mut scope, global, "checkPureJs");
        let twelve_scopes = function(&mut scope, global, "checkTwelveScopes");
        let thirteen_scopes = function(&mut scope, global, "checkThirteenScopes");
        let query = function(&mut scope, global, "checkNativeQuery");
        let receiver = v8::undefined(&mut scope).into();
        let pure = allocation_counter::measure(|| {
            assert!(pure_js.call(&mut scope, receiver, &[]).unwrap().is_true());
        });
        let cold = allocation_counter::measure(|| {
            assert!(twelve_scopes
                .call(&mut scope, receiver, &[])
                .unwrap()
                .is_true());
        });
        let after_control = allocation_counter::measure(|| {
            assert!(query.call(&mut scope, receiver, &[]).unwrap().is_true());
        });
        let extra = allocation_counter::measure(|| {
            assert!(thirteen_scopes
                .call(&mut scope, receiver, &[])
                .unwrap()
                .is_true());
        });
        let warm = allocation_counter::measure(|| {
            for _ in 0..1_000 {
                assert!(thirteen_scopes
                    .call(&mut scope, receiver, &[])
                    .unwrap()
                    .is_true());
                assert!(query.call(&mut scope, receiver, &[]).unwrap().is_true());
            }
        });
        println!(
            "Production EntityQuery scope control pure={pure:?}; twelve scopes={cold:?}; \
             after-control={after_control:?}; extra-depth={extra:?}; warm1000={warm:?}",
        );
        assert_eq!(pure, allocation_counter::AllocationInfo::default());
        assert!(cold.count_total <= 10);
        assert_eq!(cold.count_current, cold.count_total as i64);
        assert_eq!(cold.count_max, cold.count_total);
        assert_eq!(cold.bytes_total, cold.count_total * 112);
        assert_eq!(cold.bytes_current, cold.bytes_total as i64);
        assert_eq!(cold.bytes_max, cold.bytes_total);
        assert_eq!(after_control, allocation_counter::AllocationInfo::default());
        assert_eq!(extra.count_total, 1);
        assert_eq!(extra.count_current, 1);
        assert_eq!(extra.count_max, 1);
        assert_eq!(extra.bytes_total, 112);
        assert_eq!(extra.bytes_current, 112);
        assert_eq!(extra.bytes_max, 112);
        assert_eq!(warm, allocation_counter::AllocationInfo::default());
    }

    #[test]
    fn entity_posting_measures_shared_owners_and_original_row_roots() {
        let mut runtime = setup();
        super::super::line_of_sight::install(&mut runtime).unwrap();
        runtime
            .eval::<()>("globalThis.__rs2b0t_host = {};")
            .unwrap();
        crate::observed::on_reset();
        let bytes = entity_post_bytes();
        let snapshot = crate::isolate_fb::Snapshot::from_bytes(&bytes).unwrap();
        let cold = allocation_counter::measure(|| post_entities(&mut runtime, &snapshot));
        let replacement = allocation_counter::measure(|| post_entities(&mut runtime, &snapshot));
        let scope = runtime.deno_runtime().handle_scope();
        let state = entity_pages::state(&scope).unwrap();
        let pages = state.borrow();
        let latest = crate::scene_query::entity_generation();
        let row_roots: usize = [&pages.npcs, &pages.players, &pages.locs, &pages.ground]
            .into_iter()
            .map(|rows| {
                assert_eq!(rows.len(), 2);
                assert_eq!(rows.capacity(), 2);
                rows.capacity() * std::mem::size_of::<v8::Global<v8::Object>>()
            })
            .sum();
        assert!(std::rc::Rc::ptr_eq(
            pages.generation.npcs.as_ref().unwrap(),
            latest.npcs.as_ref().unwrap(),
        ));
        assert!(std::rc::Rc::ptr_eq(
            pages.generation.players.as_ref().unwrap(),
            latest.players.as_ref().unwrap(),
        ));
        assert!(std::rc::Rc::ptr_eq(
            pages.generation.locs.as_ref().unwrap(),
            latest.locs.as_ref().unwrap(),
        ));
        assert!(std::rc::Rc::ptr_eq(
            pages.generation.ground.as_ref().unwrap(),
            latest.ground.as_ref().unwrap(),
        ));
        println!(
            "Entity posting complete cold={cold:?}; replacement={replacement:?}; \
             EntityPages={} RefCellPages={} EntityGeneration={} \
             EntityRow={} SceneRow={} PlayerRow={} GlobalObject={} row-root-capacity-bytes={row_roots}",
            std::mem::size_of::<entity_pages::EntityPages>(),
            std::mem::size_of::<std::cell::RefCell<entity_pages::EntityPages>>(),
            std::mem::size_of::<crate::scene_query::EntityGeneration>(),
            std::mem::size_of::<crate::observed::EntityRow>(),
            std::mem::size_of::<crate::observed::SceneRow>(),
            std::mem::size_of::<crate::observed::PlayerRow>(),
            std::mem::size_of::<v8::Global<v8::Object>>(),
        );
        assert_eq!(replacement.count_current, 0);
        assert_eq!(replacement.bytes_current, 0);
    }

    #[test]
    fn shared_entity_page_publication_moves_existing_vectors() {
        use crate::observed::{EntityRow, PlayerRow, SceneRow};
        crate::observed::on_reset();
        let npcs = vec![EntityRow::default(); 2];
        let locs = vec![SceneRow::default(); 2];
        let players = vec![
            PlayerRow {
                row: SceneRow::default(),
                index: 1,
                target_kind: 0,
                target_index: -1,
            };
            2
        ];
        let ground = vec![SceneRow::default(); 2];
        let npc_ptr = npcs.as_ptr();
        let loc_ptr = locs.as_ptr();
        let player_ptr = players.as_ptr();
        let ground_ptr = ground.as_ptr();
        let metadata = allocation_counter::measure(|| {
            crate::observed::post(1, |post| {
                post.npcs(npcs).locs(locs).players(players).ground(ground);
            });
        });
        crate::observed::with(|scene| {
            let lens = scene.latest();
            assert_eq!(lens.npcs().unwrap().as_ptr(), npc_ptr);
            assert_eq!(lens.locs().unwrap().as_ptr(), loc_ptr);
            assert_eq!(lens.players().unwrap().as_ptr(), player_ptr);
            assert_eq!(lens.ground().unwrap().as_ptr(), ground_ptr);
        });
        println!("Entity four-page Rc metadata only={metadata:?}");
        let bytes = 4 * (std::mem::size_of::<Vec<u8>>() + 2 * std::mem::size_of::<usize>());
        assert_eq!(metadata.count_total, 4);
        assert_eq!(metadata.count_current, 4);
        assert_eq!(metadata.count_max, 4);
        assert_eq!(metadata.bytes_total, bytes as u64);
        assert_eq!(metadata.bytes_current, bytes as i64);
        assert_eq!(metadata.bytes_max, bytes as u64);
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
        assert!(
            cold.count_total <= 10,
            "cold entity bridge allocations: {cold:?}"
        );
        assert!(
            cold.count_current <= 10,
            "cold retained entity bridge allocations: {cold:?}"
        );
        assert!(
            cold.count_max <= 10,
            "cold peak entity bridge allocations: {cold:?}"
        );
        assert!(
            cold.bytes_total <= 1120,
            "cold entity bridge bytes: {cold:?}"
        );
        assert!(
            cold.bytes_current <= 1120,
            "cold retained entity bridge bytes: {cold:?}"
        );
        assert!(
            cold.bytes_max <= 1120,
            "cold peak entity bridge bytes: {cold:?}"
        );
        assert_eq!(warm, allocation_counter::AllocationInfo::default());
    }

    fn enter_scopes(scope: &mut v8::HandleScope, depth: u32) {
        if depth != 0 {
            let nested = &mut v8::TryCatch::new(scope);
            enter_scopes(nested, depth - 1);
        }
    }

    fn scope_cache_control<'s>(
        scope: &mut v8::HandleScope<'s>,
        args: v8::FunctionCallbackArguments<'s>,
        mut rv: v8::ReturnValue,
    ) {
        // The complete successful query batch reaches twelve nested scopes.
        // This control executes no entity/query operation or JS callback.
        enter_scopes(scope, if args.get(0).is_true() { 13 } else { 12 });
        rv.set(v8::Boolean::new(scope, true).into());
    }

    #[test]
    fn entity_query_cold_charge_reuses_independent_scope_cache_cell() {
        let mut runtime = setup();
        cb::install(&mut runtime, "__scope_cache_control", scope_cache_control).unwrap();
        runtime
            .eval::<()>(
                r#"
globalThis.checkTwelveScopes = () => __scope_cache_control(false);
globalThis.checkThirteenScopes = () => __scope_cache_control(true);
"#,
            )
            .unwrap();
        let context = runtime.deno_runtime().main_context();
        let mut scope = runtime.deno_runtime().handle_scope();
        let global = context.open(&mut scope).global(&mut scope);
        let pure_js = function(&mut scope, global, "checkPureJs");
        let twelve_scopes = function(&mut scope, global, "checkTwelveScopes");
        let thirteen_scopes = function(&mut scope, global, "checkThirteenScopes");
        let query = function(&mut scope, global, "checkQuery");
        let receiver = v8::undefined(&mut scope).into();
        let pure = allocation_counter::measure(|| {
            assert!(pure_js.call(&mut scope, receiver, &[]).unwrap().is_true());
        });
        let cold = allocation_counter::measure(|| {
            assert!(twelve_scopes
                .call(&mut scope, receiver, &[])
                .unwrap()
                .is_true());
        });
        let query_after_control = allocation_counter::measure(|| {
            assert!(query.call(&mut scope, receiver, &[]).unwrap().is_true());
        });
        let extra_depth = allocation_counter::measure(|| {
            assert!(thirteen_scopes
                .call(&mut scope, receiver, &[])
                .unwrap()
                .is_true());
        });
        let warm = allocation_counter::measure(|| {
            for _ in 0..1_000 {
                assert!(thirteen_scopes
                    .call(&mut scope, receiver, &[])
                    .unwrap()
                    .is_true());
                assert!(query.call(&mut scope, receiver, &[]).unwrap().is_true());
            }
        });
        println!(
            "EntityQuery scope-cache control pure={pure:?}; twelve scopes={cold:?}; \
             query after control={query_after_control:?}; \
             extra depth={extra_depth:?}; warm1000={warm:?}"
        );
        assert_eq!(pure, allocation_counter::AllocationInfo::default());
        assert_eq!(cold.count_total, 10);
        assert_eq!(cold.count_current, 10);
        assert_eq!(cold.count_max, 10);
        assert_eq!(cold.bytes_total, 1120);
        assert_eq!(cold.bytes_current, 1120);
        assert_eq!(cold.bytes_max, 1120);
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
