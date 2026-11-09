//! Typed local V8 marshalling for selected compatibility facts.
//! This is isolate-local calculation, not an isolate-to-host transport.

use crate::scene::supply_v2;
use api::snapshot::WorldTile;
use rustyscript::Runtime;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
#[path = "client_adapter_v8.rs"]
mod client_adapter_v8;

const PENDING: &str = "__pending__";
const OBJ_CATALOG_FIELDS: &[&str] = &["byId", "items", "aliases"];
const OBJ_RECORD_FIELDS: &[&str] = &[
    "id",
    "name",
    "cost",
    "stackable",
    "members",
    "equippable",
    "certlink",
    "certtemplate",
    "stackVariant",
];
const OBJ_ALIAS_FIELDS: &[&str] = &["words", "label"];

struct SelectedFactsObjectShapes {
    catalog: v8::Global<v8::ObjectTemplate>,
    record: v8::Global<v8::ObjectTemplate>,
    alias: v8::Global<v8::ObjectTemplate>,
    /// The rebuilt row array and the row count it was built with, so a cache
    /// check needs no pass over the selected items.
    obj_records: RefCell<Option<(v8::Global<v8::Array>, u32)>>,
}

enum SelectedFactsObjectShape {
    Catalog,
    Record,
    Alias,
}

pub(crate) fn install(
    runtime: &mut Runtime,
    loc_projection_interest: Arc<AtomicBool>,
) -> Result<(), String> {
    if !runtime
        .deno_runtime()
        .v8_isolate()
        .set_slot(loc_projection_interest)
    {
        return Err("loc projection interest already installed".to_string());
    }
    let object_shapes = {
        let mut scope = runtime.deno_runtime().handle_scope();
        Rc::new(SelectedFactsObjectShapes {
            catalog: object_template(&mut scope, OBJ_CATALOG_FIELDS)?,
            record: object_template(&mut scope, OBJ_RECORD_FIELDS)?,
            alias: object_template(&mut scope, OBJ_ALIAS_FIELDS)?,
            obj_records: RefCell::new(None),
        })
    };
    if !runtime.deno_runtime().v8_isolate().set_slot(object_shapes) {
        return Err("selected facts object shape cache already installed".to_string());
    }
    client_adapter_v8::install(runtime)?;
    let context = runtime.deno_runtime().main_context();
    let mut scope = runtime.deno_runtime().handle_scope();
    let global = context.open(&mut scope).global(&mut scope);
    let name = v8::String::new(&mut scope, "__rs2b0t_selected_facts")
        .ok_or_else(|| "selected facts name".to_string())?;
    let func = v8::Function::new(&mut scope, selected_facts_callback)
        .ok_or_else(|| "selected facts function".to_string())?;
    global
        .set(&mut scope, name.into(), func.into())
        .ok_or_else(|| "selected facts set".to_string())?;
    Ok(())
}

fn selected_facts_callback<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue,
) {
    match run_selected_facts(scope, &args) {
        Ok(value) => rv.set(value),
        Err(error) if error == PENDING => {}
        Err(error) => throw_error(scope, &error),
    }
}

fn run_selected_facts<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let op = js_to_string(scope, args.get(0))?;
    match op.as_str() {
        "item-db" => item_db(scope),
        "fixed-food-heals" => fixed_food_heals(scope),
        "food-forms" => food_forms(scope, args.get(1)),
        "range-loadout" => range_loadout(scope, args.get(1), args.get(2)),
        "ranged-item" => ranged_item(scope, args.get(1), args.get(2), args.get(3), args.get(4)),
        "cert-maps" => cert_maps(scope),
        "cert-link" => cert_link(scope, args.get(1), args.get(2)),
        "obj-records" => obj_records(scope),
        "obj-catalog" => obj_catalog(scope),
        "obj-resolve-by-name" => obj_resolve_by_name(scope, args.get(1), args.get(2), args.get(3)),
        "random-event-casket" => random_event_casket(scope),
        "autocast-component" => autocast_component(scope, args.get(1)),
        "item-tradeable" => item_tradeable(scope, args.get(1)),
        "item-name" => item_name(scope, args.get(1), args.get(2)),
        "cow-locations" => cow_locations(scope, args.get(1)),
        "cow-resolve" => cow_resolve(scope, args.get(1), args.get(2), args.get(3)),
        "cow-nearest" => cow_nearest(scope, args.get(1), args.get(2)),
        "cow-loot-tile" => cow_loot_tile(scope, args.get(1), args.get(2), args.get(3)),
        "cow-needs-toll" => cow_needs_toll(scope, args.get(1), args.get(2)),
        "cow-bank-destination" => {
            cow_bank_destination(scope, args.get(1), args.get(2), args.get(3))
        }
        "cow-bootstrap-toll" => {
            cow_bootstrap_toll(scope, args.get(1), args.get(2), args.get(3), args.get(4))
        }
        "pickpocket-spot" => pickpocket_spot(scope, args.get(1)),
        "required-thieving" => required_thieving(scope, args.get(1)),
        "hostile-attacker-names" => hostile_attacker_names(scope),
        "anchor-walk-home" => anchor_predicate(scope, args.get(1), args.get(2), false),
        "anchor-soft-home" => anchor_predicate(scope, args.get(1), args.get(2), true),
        "thieving-ready" => thieving_ready(scope, args.get(1), args.get(2)),
        "gas-rock-ids" => gas_rock_ids(scope),
        "mining-rock-data" => mining_rock_data(scope),
        "rock-tier-by-id" => selected_rock_tier_by_id(scope, args.get(1)),
        "resolve-rock-ids" => selected_resolve_rock_ids(scope, args.get(1)),
        "worth-stocking" => worth_stocking(scope, args.get(1)),
        "world-number" => world_number(scope, args.get(1), args.get(2), args.get(3)),
        "client-adapter-reader" => client_adapter_v8::read(
            scope,
            args.get(1),
            args.get(2),
            args.get(3),
            args.get(4),
            args.get(5),
            args.get(6),
        ),
        _ => Err(format!("unsupported selected facts operation: {op}")),
    }
}

fn anchor_predicate<'s>(
    scope: &mut v8::HandleScope<'s>,
    distance: v8::Local<'s, v8::Value>,
    threshold: v8::Local<'s, v8::Value>,
    soft_home: bool,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let distance = finite_number(scope, distance);
    let threshold = finite_number(scope, threshold);
    let allowed = if soft_home {
        script::native::walk::should_soft_home_from_gather_miss(distance, threshold)
    } else {
        script::native::walk::should_walk_home_to_gather_anchor(distance, threshold)
    };
    Ok(v8::Boolean::new(scope, allowed).into())
}

fn finite_number(scope: &mut v8::HandleScope, value: v8::Local<v8::Value>) -> Option<f64> {
    value
        .is_number()
        .then(|| value.number_value(scope))
        .flatten()
        .filter(|number| number.is_finite())
}

fn world_number<'s>(
    scope: &mut v8::HandleScope<'s>,
    hostname: v8::Local<'s, v8::Value>,
    value_count: v8::Local<'s, v8::Value>,
    value: v8::Local<'s, v8::Value>,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let hostname = js_to_string(scope, hostname)?;
    let value_count =
        js_i32(scope, value_count)?.ok_or_else(|| "world must be 1, 2 or 3".to_string())?;
    let value_count =
        usize::try_from(value_count).map_err(|_| "world must be 1, 2 or 3".to_string())?;
    let value = if value_count == 1 {
        Some(js_to_string(scope, value)?)
    } else {
        None
    };
    let selected = script::registry::resolve_world_number(&hostname, value_count, value.as_deref())
        .map_err(str::to_owned)?;
    Ok(v8::Integer::new(scope, selected).into())
}

fn random_event_casket<'s>(
    scope: &mut v8::HandleScope<'s>,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let id = supply_v2::random_event_casket_id().unwrap_or(-1);
    Ok(v8::Integer::new(scope, id).into())
}
fn autocast_component<'s>(
    scope: &mut v8::HandleScope<'s>,
    alias: v8::Local<'s, v8::Value>,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let alias = js_to_string(scope, alias)?;
    let component = supply_v2::selected_data()
        .as_deref()
        .and_then(api::game_data::SelectedGameData::autocast_controls)
        .and_then(|controls| match alias.as_str() {
            "combat_staff_2:auto_choose" => Some(controls.choose_com),
            "combat_staff_2:auto_toggle" => Some(controls.toggle_com),
            _ => None,
        })
        .unwrap_or(-1);
    Ok(v8::Integer::new(scope, component).into())
}

fn worth_stocking<'s>(
    scope: &mut v8::HandleScope<'s>,
    name: v8::Local<'s, v8::Value>,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let name = js_to_string(scope, name)?;
    Ok(v8::Boolean::new(scope, crate::compat::market_catalog::worth_stocking(&name)).into())
}

fn item_db<'s>(scope: &mut v8::HandleScope<'s>) -> Result<v8::Local<'s, v8::Value>, String> {
    let Some(data) = supply_v2::selected_data() else {
        return Ok(v8::Array::new(scope, 0).into());
    };
    let rows: Vec<_> = data
        .items()
        .iter()
        .filter_map(|item| Some((item, item.alias.as_deref()?, item.name.as_deref()?)))
        .collect();
    let array = v8::Array::new(scope, i32::try_from(rows.len()).unwrap_or(i32::MAX));
    for (index, (item, alias, name)) in rows.into_iter().enumerate() {
        let row = v8::Object::new(scope);
        let alias = v8_str(scope, alias)?;
        set_key(scope, row, "obj", alias);
        let id = v8::Integer::new(scope, item.id);
        set_key(scope, row, "id", id.into());
        let name = v8_str(scope, name)?;
        set_key(scope, row, "name", name);
        let cost = v8::Integer::new(scope, item.cost);
        set_key(scope, row, "cost", cost.into());
        array
            .set_index(scope, index as u32, row.into())
            .ok_or_else(|| "selected facts item array".to_string())?;
    }
    Ok(array.into())
}

fn fixed_food_heals<'s>(
    scope: &mut v8::HandleScope<'s>,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let array = v8::Array::new(scope, 0);
    let Some(data) = supply_v2::selected_data() else {
        return Ok(array.into());
    };
    for (index, name) in data.fixed_food_options_best_first().into_iter().enumerate() {
        let name = v8_str(scope, name)?;
        array
            .set_index(scope, index as u32, name)
            .ok_or_else(|| "selected facts fixed food array".to_string())?;
    }
    Ok(array.into())
}

fn food_forms<'s>(
    scope: &mut v8::HandleScope<'s>,
    food_name: v8::Local<v8::Value>,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let food_name = js_to_string(scope, food_name)?;
    let data = supply_v2::selected_data();
    let forms = script::food_policy::food_forms_for(data.as_deref(), &food_name);
    string_array(scope, &forms)
}

fn range_loadout<'s>(
    scope: &mut v8::HandleScope<'s>,
    weapon: v8::Local<v8::Value>,
    ammo: v8::Local<v8::Value>,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let weapon = js_to_string(scope, weapon)?;
    let ammo = js_to_string(scope, ammo)?;
    let data = supply_v2::selected_data();
    let loadout = script::ranged::range_loadout_of_items(
        data.as_deref()
            .map_or(&[], api::game_data::SelectedGameData::items),
        &weapon,
        &ammo,
    );
    let row = v8::Object::new(scope);
    let weapon = v8_str(scope, &loadout.weapon)?;
    set_key(scope, row, "weapon", weapon);
    let projectile = v8_str(scope, &loadout.projectile)?;
    set_key(scope, row, "projectile", projectile);
    let thrown = v8::Boolean::new(scope, loadout.thrown);
    set_key(scope, row, "thrown", thrown.into());
    Ok(row.into())
}

fn ranged_item<'s>(
    scope: &mut v8::HandleScope<'s>,
    selected: v8::Local<v8::Value>,
    custom: v8::Local<v8::Value>,
    fallback: v8::Local<v8::Value>,
    key: v8::Local<v8::Value>,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let selected = js_to_string(scope, selected)?;
    let custom = js_to_string(scope, custom)?;
    let fallback = js_to_string(scope, fallback)?;
    let key = js_to_string(scope, key)?;
    let resolved =
        script::ranged::ranged_item(Some(&selected), Some(&custom), &fallback, key == "bow")?;
    v8_str(scope, resolved)
}

fn cert_maps<'s>(scope: &mut v8::HandleScope<'s>) -> Result<v8::Local<'s, v8::Value>, String> {
    let Some(data) = supply_v2::selected_data() else {
        let row = v8::Object::new(scope);
        let noted = v8::Array::new(scope, 0);
        set_key(scope, row, "notedOf", noted.into());
        let unnoted = v8::Array::new(scope, 0);
        set_key(scope, row, "unnotedOf", unnoted.into());
        return Ok(row.into());
    };
    let pairs: Vec<_> = data
        .items()
        .iter()
        .filter(|item| !item.is_certificate() && item.certificate_link >= 0)
        .map(|item| (item.id, item.certificate_link))
        .collect();
    let noted = int_pairs(scope, pairs.iter().copied())?;
    let unnoted = int_pairs(scope, pairs.iter().map(|(base, cert)| (*cert, *base)))?;
    let row = v8::Object::new(scope);
    set_key(scope, row, "notedOf", noted);
    set_key(scope, row, "unnotedOf", unnoted);
    Ok(row.into())
}

/// Frozen `notedId(cat, id)` (the note or `null`) and `unnotedId(cat, id)`
/// (the base item or the id itself). `undefined` without selected data.
fn cert_link<'s>(
    scope: &mut v8::HandleScope<'s>,
    id: v8::Local<v8::Value>,
    direction: v8::Local<v8::Value>,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let direction = js_to_string(scope, direction)?;
    let Some(data) = supply_v2::selected_data() else {
        return Ok(v8::undefined(scope).into());
    };
    let Some(id) = js_i32(scope, id)? else {
        return Ok(v8::null(scope).into());
    };
    let linked = match direction.as_str() {
        "noted" => crate::compat::market_catalog::noted_id(&data, id),
        "unnoted" => Some(crate::compat::market_catalog::unnoted_id(&data, id)),
        _ => return Err("invalid selected facts op".into()),
    };
    match linked {
        Some(n) => Ok(v8::Integer::new(scope, n).into()),
        None => Ok(v8::null(scope).into()),
    }
}

fn object_template(
    scope: &mut v8::HandleScope,
    fields: &[&str],
) -> Result<v8::Global<v8::ObjectTemplate>, String> {
    let template = v8::ObjectTemplate::new(scope);
    let placeholder: v8::Local<v8::Data> = v8::undefined(scope).into();
    for field in fields {
        let key =
            v8::String::new_from_utf8(scope, field.as_bytes(), v8::NewStringType::Internalized)
                .ok_or_else(|| format!("selected facts object shape key `{field}`"))?;
        template.set(key.into(), placeholder);
    }
    Ok(v8::Global::new(scope, template))
}

fn selected_facts_object<'s>(
    scope: &mut v8::HandleScope<'s>,
    shape: SelectedFactsObjectShape,
) -> Result<v8::Local<'s, v8::Object>, String> {
    let shapes = scope
        .get_slot::<Rc<SelectedFactsObjectShapes>>()
        .cloned()
        .ok_or_else(|| "selected facts object shape cache is not installed".to_string())?;
    let template = match shape {
        SelectedFactsObjectShape::Catalog => &shapes.catalog,
        SelectedFactsObjectShape::Record => &shapes.record,
        SelectedFactsObjectShape::Alias => &shapes.alias,
    };
    v8::Local::new(scope, template)
        .new_instance(scope)
        .ok_or_else(|| "selected facts object allocation failed".to_string())
}

fn obj_record_row<'s>(
    scope: &mut v8::HandleScope<'s>,
    item: &api::game_data::GameItem,
    name: &str,
) -> Result<v8::Local<'s, v8::Object>, String> {
    let row = selected_facts_object(scope, SelectedFactsObjectShape::Record)?;
    set_i32(scope, row, "id", item.id);
    let name = v8_str(scope, name)?;
    set_key(scope, row, "name", name);
    set_i32(scope, row, "cost", item.cost);
    set_bool(scope, row, "stackable", item.stackable);
    set_bool(scope, row, "members", item.members);
    set_bool(
        scope,
        row,
        "equippable",
        crate::compat::market_catalog::equippable(item),
    );
    set_i32(scope, row, "certlink", item.certificate_link);
    set_i32(scope, row, "certtemplate", item.certificate_template);
    set_bool(scope, row, "stackVariant", item.stack_variant);
    Ok(row)
}

/// Resolve against the selected revision and return the caller catalog's rows
/// so object identity matches the JavaScript catalog contract.
fn obj_resolve_by_name<'s>(
    scope: &mut v8::HandleScope<'s>,
    catalog: v8::Local<'s, v8::Value>,
    query: v8::Local<'s, v8::Value>,
    exact_only: v8::Local<'s, v8::Value>,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let Some(data) = supply_v2::selected_data() else {
        return Ok(v8::Array::new(scope, 0).into());
    };
    let query = js_to_string(scope, query)?;
    let index = crate::compat::market_catalog::market_index(&data);
    let ids = crate::compat::market_catalog::resolve_by_name(&index, &query, exact_only.is_true());
    let catalog = catalog
        .to_object(scope)
        .ok_or_else(|| "selected facts catalog object".to_string())?;
    let key = v8_str(scope, "byId")?;
    let by_id = catalog
        .get(scope, key)
        .ok_or_else(|| "selected facts catalog byId".to_string())?;
    let by_id = by_id
        .to_object(scope)
        .and_then(|value| v8::Local::<v8::Map>::try_from(value).ok())
        .ok_or_else(|| "selected facts catalog byId map".to_string())?;
    let result = v8::Array::new(scope, i32::try_from(ids.len()).unwrap_or(i32::MAX));
    for (index, id) in ids.iter().enumerate() {
        let id = v8::Integer::new(scope, *id);
        let row = by_id
            .get(scope, id.into())
            .ok_or_else(|| "selected facts catalog item".to_string())?;
        result
            .set_index(scope, index as u32, row)
            .ok_or_else(|| "selected facts catalog result".to_string())?;
    }
    Ok(result.into())
}

/// Frozen adapter `reader.objCatalog()`: every named object, or a fresh empty array before data.
fn obj_records<'s>(scope: &mut v8::HandleScope<'s>) -> Result<v8::Local<'s, v8::Value>, String> {
    Ok(obj_record_rows(scope)?.into())
}

/// The row array that `obj-records` hands to scripts and `obj-catalog` reads.
/// A script can reorder or empty the array it receives, so the catalog keys
/// each entry by the row's own id and a length change rebuilds the array.
fn obj_record_rows<'s>(
    scope: &mut v8::HandleScope<'s>,
) -> Result<v8::Local<'s, v8::Array>, String> {
    let shapes = scope
        .get_slot::<Rc<SelectedFactsObjectShapes>>()
        .cloned()
        .ok_or_else(|| "selected facts object shape cache is not installed".to_string())?;
    let Some(data) = supply_v2::selected_data() else {
        return Ok(v8::Array::new(scope, 0));
    };
    {
        let cached = shapes.obj_records.borrow();
        if let Some((rows, count)) = cached.as_ref() {
            let rows = v8::Local::new(scope, rows);
            if rows.length() == *count {
                return Ok(rows);
            }
        }
    }
    // `new_with_elements` writes own elements, so a setter that a script put on
    // `Array.prototype` cannot drop a row from the rebuild.
    let rows = crate::compat::market_catalog::records(data.items())
        .map(|(item, name)| obj_record_row(scope, item, name).map(Into::into))
        .collect::<Result<Vec<v8::Local<'s, v8::Value>>, String>>()?;
    let count = u32::try_from(rows.len()).unwrap_or(u32::MAX);
    let rows = v8::Array::new_with_elements(scope, &rows);
    if count > 0 {
        *shapes.obj_records.borrow_mut() = Some((v8::Global::new(scope, rows), count));
    }
    Ok(rows)
}

/// The row's own `id`, the key the frozen `buildCatalog` uses.
fn obj_row_id<'s>(
    scope: &mut v8::HandleScope<'s>,
    row: v8::Local<'s, v8::Value>,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let row = row
        .to_object(scope)
        .ok_or_else(|| "selected facts obj-record row is not an object".to_string())?;
    let key = v8_str(scope, "id")?;
    row.get(scope, key)
        .ok_or_else(|| "selected facts obj-record id".to_string())
}

/// Frozen `buildCatalog` minus the cert maps: `byId` holds every named obj
/// as an `ObjRecord`, `items` the listed subset (the same record objects) and
/// `aliases` the derived same-name labels. Empty without selected data.
fn obj_catalog<'s>(scope: &mut v8::HandleScope<'s>) -> Result<v8::Local<'s, v8::Value>, String> {
    let catalog = selected_facts_object(scope, SelectedFactsObjectShape::Catalog)?;
    let by_id = v8::Map::new(scope);
    let aliases = v8::Map::new(scope);
    let Some(data) = supply_v2::selected_data() else {
        set_key(scope, catalog, "byId", by_id.into());
        let items = v8::Array::new(scope, 0);
        set_key(scope, catalog, "items", items.into());
        set_key(scope, catalog, "aliases", aliases.into());
        return Ok(catalog.into());
    };
    let market = crate::compat::market_catalog::market_index(&data);
    let rows = obj_record_rows(scope)?;
    for index in 0..rows.length() {
        let row = rows
            .get_index(scope, index)
            .ok_or_else(|| "selected facts obj-records item".to_string())?;
        let id = obj_row_id(scope, row)?;
        by_id
            .set(scope, id, row)
            .ok_or_else(|| "selected facts catalog byId".to_string())?;
    }
    let items = v8::Array::new(
        scope,
        i32::try_from(market.listed_len()).unwrap_or(i32::MAX),
    );
    for (index, id) in market.ordered_listed_ids().enumerate() {
        let id = v8::Integer::new(scope, id);
        let row = by_id
            .get(scope, id.into())
            .ok_or_else(|| "selected facts catalog item".to_string())?;
        // `Map::get` gives `undefined` for a missing key: a script rewrote the
        // `id` of a listed row, so fail here instead of in a later catalog walk.
        if row.is_undefined() {
            return Err("selected facts catalog item is not a row".to_string());
        }
        items
            .set_index(scope, index as u32, row)
            .ok_or_else(|| "selected facts catalog items".to_string())?;
    }
    for (id, alias) in market.aliases.iter() {
        let row = selected_facts_object(scope, SelectedFactsObjectShape::Alias)?;
        let words = string_array(scope, &alias.words)?;
        set_key(scope, row, "words", words);
        let label = v8_str(scope, &alias.label)?;
        set_key(scope, row, "label", label);
        let id = v8::Integer::new(scope, *id);
        aliases
            .set(scope, id.into(), row.into())
            .ok_or_else(|| "selected facts catalog aliases".to_string())?;
    }
    set_key(scope, catalog, "byId", by_id.into());
    set_key(scope, catalog, "items", items.into());
    set_key(scope, catalog, "aliases", aliases.into());
    Ok(catalog.into())
}

/// Frozen `tradeable(id)`. `undefined` without selected data.
fn item_tradeable<'s>(
    scope: &mut v8::HandleScope<'s>,
    id: v8::Local<v8::Value>,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let Some(data) = supply_v2::selected_data() else {
        return Ok(v8::undefined(scope).into());
    };
    let tradeable =
        js_i32(scope, id)?.is_none_or(|id| crate::compat::market_catalog::tradeable(&data, id));
    Ok(v8::Boolean::new(scope, tradeable).into())
}

/// Frozen `clientName(cat, id)` (`null` when unnamed) and
/// `displayName(cat, id)`. `undefined` without selected data.
fn item_name<'s>(
    scope: &mut v8::HandleScope<'s>,
    id: v8::Local<v8::Value>,
    which: v8::Local<v8::Value>,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let which = js_to_string(scope, which)?;
    let Some(data) = supply_v2::selected_data() else {
        return Ok(v8::undefined(scope).into());
    };
    match (which.as_str(), js_i32(scope, id)?) {
        ("client", Some(id)) => match crate::compat::market_catalog::client_name(&data, id) {
            Some(name) => v8_str(scope, name),
            None => Ok(v8::null(scope).into()),
        },
        ("client", None) => Ok(v8::null(scope).into()),
        ("display", Some(id)) => v8_str(
            scope,
            &crate::compat::market_catalog::display_name(&data, id),
        ),
        ("display", None) => {
            let raw = js_to_string(scope, id)?;
            v8_str(scope, &format!("item {raw}"))
        }
        _ => Err("invalid selected facts op".into()),
    }
}

fn cow_locations<'s>(
    scope: &mut v8::HandleScope<'s>,
    tile_constructor: v8::Local<'s, v8::Value>,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let banks = CowBankTiles {
        al_kharid: tile_instance(scope, tile_constructor, script::content::AL_KHARID_BANK)?,
        draynor: cow_bank_tile_value(
            scope,
            tile_constructor,
            "Draynor",
            script::content::DRAYNOR_BANK,
        )?,
        falador_east: cow_bank_tile_value(
            scope,
            tile_constructor,
            "Falador East",
            script::content::FALADOR_EAST_BANK,
        )?,
        ardougne_west: cow_bank_tile_value(
            scope,
            tile_constructor,
            "Ardougne West",
            script::content::ARDOUGNE_WEST_BANK,
        )?,
    };
    let array = v8::Array::new(
        scope,
        i32::try_from(script::content::COW_FIELDS.len()).unwrap_or(i32::MAX),
    );
    for (index, field) in script::content::COW_FIELDS.iter().enumerate() {
        let row = cow_field(scope, field, tile_constructor, &banks)?;
        array
            .set_index(scope, index as u32, row)
            .ok_or_else(|| "selected facts cow array".to_string())?;
    }
    let options = v8::Array::new(
        scope,
        i32::try_from(script::content::COW_FIELDS.len() + 2).unwrap_or(i32::MAX),
    );
    for (index, option) in std::iter::once("Auto")
        .chain(script::content::COW_FIELDS.iter().map(|field| field.name))
        .chain(std::iter::once("Start tile"))
        .enumerate()
    {
        let option = v8_str(scope, option)?;
        options
            .set_index(scope, index as u32, option)
            .ok_or_else(|| "selected facts cow options".to_string())?;
    }
    let facts = v8::Object::new(scope);
    set_key(scope, facts, "locations", array.into());
    set_key(scope, facts, "options", options.into());
    set_key(scope, facts, "alKharidBank", banks.al_kharid);
    set_key(scope, facts, "draynorBank", banks.draynor);
    set_key(scope, facts, "faladorEastBank", banks.falador_east);
    set_key(scope, facts, "ardougneWestBank", banks.ardougne_west);
    let target = v8::Integer::new(scope, script::content::COW_TOLL_COIN_TARGET);
    set_key(scope, facts, "tollCoinTarget", target.into());
    Ok(facts.into())
}

fn cow_resolve<'s>(
    scope: &mut v8::HandleScope<'s>,
    setting: v8::Local<'s, v8::Value>,
    start: v8::Local<'s, v8::Value>,
    locations: v8::Local<'s, v8::Value>,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let setting = js_to_string(scope, setting)?;
    let start = if setting.trim().eq_ignore_ascii_case("auto") {
        js_tile(scope, start)?
    } else {
        None
    };
    match script::content::resolve_cow_field(&setting, start) {
        Some(field) => cow_location_value(scope, locations, field),
        None => Ok(v8::null(scope).into()),
    }
}

fn cow_nearest<'s>(
    scope: &mut v8::HandleScope<'s>,
    tile: v8::Local<'s, v8::Value>,
    locations: v8::Local<'s, v8::Value>,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let Some(from) = js_tile(scope, tile)? else {
        return Ok(v8::null(scope).into());
    };
    match script::content::nearest_cow_field(from) {
        Some(field) => cow_location_value(scope, locations, field),
        None => Ok(v8::null(scope).into()),
    }
}

fn cow_loot_tile<'s>(
    scope: &mut v8::HandleScope<'s>,
    anchor: v8::Local<'s, v8::Value>,
    leash_radius: v8::Local<'s, v8::Value>,
    tile: v8::Local<'s, v8::Value>,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let Some(anchor) = js_tile(scope, anchor)? else {
        return Ok(v8::Boolean::new(scope, false).into());
    };
    let leash_radius = leash_radius
        .number_value(scope)
        .ok_or_else(|| PENDING.to_string())?;
    let Some(tile) = js_tile(scope, tile)? else {
        return Ok(v8::Boolean::new(scope, false).into());
    };
    let allowed = script::content::is_cow_field_loot_tile(anchor, leash_radius, tile);
    Ok(v8::Boolean::new(scope, allowed).into())
}

fn cow_needs_toll<'s>(
    scope: &mut v8::HandleScope<'s>,
    location: v8::Local<'s, v8::Value>,
    enabled: v8::Local<'s, v8::Value>,
) -> Result<v8::Local<'s, v8::Value>, String> {
    if !super::callback_v8::truthy(scope, enabled) {
        return Ok(enabled);
    }
    let uses_toll = cow_location_uses_al_kharid_toll(scope, location)?;
    let needs = script::content::needs_cow_toll_coins(uses_toll, true);
    Ok(v8::Boolean::new(scope, needs).into())
}

fn cow_bank_destination<'s>(
    scope: &mut v8::HandleScope<'s>,
    location: v8::Local<'s, v8::Value>,
    toll_enabled: v8::Local<'s, v8::Value>,
    al_kharid_bank: v8::Local<'s, v8::Value>,
) -> Result<v8::Local<'s, v8::Value>, String> {
    if super::callback_v8::truthy(scope, toll_enabled) {
        let uses_toll = cow_location_uses_al_kharid_toll(scope, location)?;
        if let Some(destination) = script::content::cow_bank_destination(uses_toll, true, None) {
            let row = v8::Object::new(scope);
            let name = v8_str(scope, destination.name)?;
            set_key(scope, row, "name", name);
            set_key(scope, row, "tile", al_kharid_bank);
            return Ok(row.into());
        }
    }
    if location.is_null() || location.is_undefined() {
        return Ok(v8::null(scope).into());
    }
    let destination = cow_object_property(scope, location, "bankDestination")?;
    if destination.is_null() || destination.is_undefined() {
        Ok(v8::null(scope).into())
    } else {
        Ok(destination)
    }
}

fn cow_bootstrap_toll<'s>(
    scope: &mut v8::HandleScope<'s>,
    location: v8::Local<'s, v8::Value>,
    start: v8::Local<'s, v8::Value>,
    coins: v8::Local<'s, v8::Value>,
    enabled: v8::Local<'s, v8::Value>,
) -> Result<v8::Local<'s, v8::Value>, String> {
    if !super::callback_v8::truthy(scope, enabled) {
        return Ok(enabled);
    }
    let uses_toll = cow_location_uses_al_kharid_toll(scope, location)?;
    if !script::content::needs_cow_toll_coins(uses_toll, true) {
        return Ok(v8::Boolean::new(scope, false).into());
    }
    let coins = coins
        .number_value(scope)
        .ok_or_else(|| PENDING.to_string())?;
    if coins.partial_cmp(&f64::from(script::content::COW_TOLL_COIN_TARGET))
        != Some(std::cmp::Ordering::Less)
    {
        return Ok(v8::Boolean::new(scope, false).into());
    }
    let Some(start) = js_tile(scope, start)? else {
        return Ok(v8::Boolean::new(scope, false).into());
    };
    let bootstrap = script::content::should_bootstrap_cow_toll_coins(uses_toll, start, coins, true);
    Ok(v8::Boolean::new(scope, bootstrap).into())
}

fn cow_location_uses_al_kharid_toll<'s>(
    scope: &mut v8::HandleScope<'s>,
    location: v8::Local<'s, v8::Value>,
) -> Result<bool, String> {
    if location.is_null() || location.is_undefined() {
        return Ok(false);
    }
    Ok(cow_object_property(scope, location, "usesAlKharidToll")?.is_true())
}

fn cow_object_property<'s>(
    scope: &mut v8::HandleScope<'s>,
    target: v8::Local<'s, v8::Value>,
    key: &str,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let object = target.to_object(scope).ok_or_else(|| PENDING.to_string())?;
    let key = v8::String::new(scope, key).ok_or_else(|| "selected facts string".to_string())?;
    object
        .get(scope, key.into())
        .ok_or_else(|| PENDING.to_string())
}

fn cow_location_value<'s>(
    scope: &mut v8::HandleScope<'s>,
    locations: v8::Local<'s, v8::Value>,
    field: &script::content::CowField,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let locations = v8::Local::<v8::Array>::try_from(locations)
        .map_err(|_| "selected facts cow locations".to_string())?;
    let index = script::content::COW_FIELDS
        .iter()
        .position(|candidate| candidate.name == field.name)
        .ok_or_else(|| "selected facts cow location index".to_string())?;
    locations
        .get_index(scope, index as u32)
        .ok_or_else(|| PENDING.to_string())
}

struct CowBankTiles<'s> {
    al_kharid: v8::Local<'s, v8::Value>,
    draynor: v8::Local<'s, v8::Value>,
    falador_east: v8::Local<'s, v8::Value>,
    ardougne_west: v8::Local<'s, v8::Value>,
}

impl<'s> CowBankTiles<'s> {
    fn get(&self, name: &str) -> Option<v8::Local<'s, v8::Value>> {
        match name {
            "Al Kharid" => Some(self.al_kharid),
            "Draynor" => Some(self.draynor),
            "Falador East" => Some(self.falador_east),
            "Ardougne West" => Some(self.ardougne_west),
            _ => None,
        }
    }
}

fn cow_bank_tile_value<'s>(
    scope: &mut v8::HandleScope<'s>,
    tile_constructor: v8::Local<'s, v8::Value>,
    name: &str,
    fallback: WorldTile,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let tile = crate::scene::bank_select::bank_tile(name).unwrap_or(fallback);
    tile_instance(scope, tile_constructor, tile)
}

fn cow_bank_destination_value<'s>(
    scope: &mut v8::HandleScope<'s>,
    destination: script::content::CowBankDestination,
    banks: &CowBankTiles<'s>,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let row = v8::Object::new(scope);
    let name = v8_str(scope, destination.name)?;
    set_key(scope, row, "name", name);
    let tile = banks
        .get(destination.name)
        .ok_or_else(|| "selected facts cow bank destination".to_string())?;
    set_key(scope, row, "tile", tile);
    Ok(row.into())
}

fn cow_field<'s>(
    scope: &mut v8::HandleScope<'s>,
    field: &script::content::CowField,
    tile_constructor: v8::Local<'s, v8::Value>,
    banks: &CowBankTiles<'s>,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let row = v8::Object::new(scope);
    let name = v8_str(scope, field.name)?;
    set_key(scope, row, "name", name);
    let anchor = tile_instance(
        scope,
        tile_constructor,
        WorldTile {
            x: field.x,
            z: field.z,
            level: field.level,
        },
    )?;
    set_key(scope, row, "anchor", anchor);
    set_bool(
        scope,
        row,
        "usesAlKharidToll",
        script::content::cow_uses_al_kharid_toll(field),
    );
    if let Some(destination) = field.bank_destination {
        let destination = cow_bank_destination_value(scope, destination, banks)?;
        set_key(scope, row, "bankDestination", destination);
    }
    Ok(row.into())
}

fn tile_instance<'s>(
    scope: &mut v8::HandleScope<'s>,
    tile_constructor: v8::Local<'s, v8::Value>,
    tile: WorldTile,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let constructor = v8::Local::<v8::Function>::try_from(tile_constructor)
        .map_err(|_| "selected facts Tile constructor".to_string())?;
    let args = [
        v8::Integer::new(scope, tile.x).into(),
        v8::Integer::new(scope, tile.z).into(),
        v8::Integer::new(scope, tile.level).into(),
    ];
    constructor
        .new_instance(scope, &args)
        .map(Into::into)
        .ok_or_else(|| PENDING.to_string())
}

fn pickpocket_spot<'s>(
    scope: &mut v8::HandleScope<'s>,
    target: v8::Local<v8::Value>,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let target = js_to_string(scope, target)?;
    let Some(spot) = script::content::pickpocket_spot(&target) else {
        return Ok(v8::null(scope).into());
    };
    let row = v8::Object::new(scope);
    let name = v8_str(scope, spot.name)?;
    set_key(scope, row, "name", name);
    set_i32(scope, row, "x", spot.x);
    set_i32(scope, row, "z", spot.z);
    set_i32(scope, row, "level", spot.level);
    set_i32(scope, row, "leash", spot.leash);
    Ok(row.into())
}

/// Frozen `requiredThieving(target)`: the selected pickpocket level of the
/// NPC named `target`, and the frozen `?? 1` for a name no pickpocket row
/// lists. `undefined` without selected data.
fn required_thieving<'s>(
    scope: &mut v8::HandleScope<'s>,
    target: v8::Local<v8::Value>,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let target = js_to_string(scope, target)?;
    let Some(data) = supply_v2::selected_data() else {
        return Ok(v8::undefined(scope).into());
    };
    let level = data.required_thieving(&target).unwrap_or(1);
    Ok(v8::Integer::new(scope, level).into())
}

/// Frozen `Npc.interact` admission is kept on the host: only a known
/// insufficient effective Thieving level refuses the queued NPC op.
fn thieving_ready<'s>(
    scope: &mut v8::HandleScope<'s>,
    npc_id: v8::Local<v8::Value>,
    action: v8::Local<v8::Value>,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let action = js_to_string(scope, action)?;
    if !action.eq_ignore_ascii_case(script::native::thieving_core::PICKPOCKET)
        && !action.eq_ignore_ascii_case(script::native::thieving_core::STEAL_FROM)
    {
        return Ok(v8::Boolean::new(scope, true).into());
    }
    let Some(npc_id) = js_i32(scope, npc_id)? else {
        return Ok(v8::Boolean::new(scope, true).into());
    };
    let Some(data) = supply_v2::selected_data() else {
        return Ok(v8::Boolean::new(scope, true).into());
    };
    let required = data.required_thieving_npc(npc_id).unwrap_or(1);
    let effective = crate::scene::observed::with(|scene| {
        scene
            .latest()
            .stats()
            .and_then(|stats| stats.thieving)
            .map(|stat| stat.effective)
    });
    let ready = !matches!(
        script::native::thieving_core::level_ready(effective, required),
        Some(false)
    );
    Ok(v8::Boolean::new(scope, ready).into())
}

/// Frozen `GAS_ROCK_IDS`: this isolate's one ordinary, mutable `Set`, built once at module evaluation from the
/// selected core's `mining_hazards` slice (the gathering family's gas rocks, carried by the pinned core so the
/// family is never decoded or held for it). `undefined` without the slice, which the shim turns into its
/// fail-closed `not impl`.
fn gas_rock_ids<'s>(scope: &mut v8::HandleScope<'s>) -> Result<v8::Local<'s, v8::Value>, String> {
    let Some(data) = supply_v2::selected_data() else {
        return Ok(v8::undefined(scope).into());
    };
    let Ok(ids) = api::gather_methods::gas_rock_ids(data.as_ref(), data.mining_hazards()) else {
        return Ok(v8::undefined(scope).into());
    };
    let set = v8::Set::new(scope);
    for id in ids {
        let id = v8::Integer::new(scope, id);
        set.add(scope, id.into())
            .ok_or_else(|| "selected facts gas rock set".to_string())?;
    }
    Ok(set.into())
}

fn mining_rock_data<'s>(
    scope: &mut v8::HandleScope<'s>,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let data = v8::Object::new(scope);
    let rock_types = v8::Object::new(scope);
    for (name, ids) in api::gather_methods::ROCK_TYPE_IDS.iter() {
        let ids = int_array(scope, ids)?;
        set_key(scope, rock_types, name, ids);
    }
    let quest_rock_types = v8::Object::new(scope);
    for (name, ids) in api::gather_methods::QUEST_ROCK_TYPE_IDS.iter() {
        let ids = int_array(scope, ids)?;
        set_key(scope, quest_rock_types, name, ids);
    }
    let options = static_string_array(scope, api::gather_methods::rock_type_order())?;
    let order = static_string_array(scope, api::gather_methods::rock_type_order())?;
    set_key(scope, data, "rockOptions", options);
    set_key(scope, data, "rockTypes", rock_types.into());
    set_key(scope, data, "questRockTypes", quest_rock_types.into());
    set_key(scope, data, "rockTierOrder", order);
    let broken_pickaxe = v8_str(scope, api::gather_methods::BROKEN_PICKAXE)?;
    set_i32(
        scope,
        data,
        "gasRockTicks",
        api::gather_methods::GAS_ROCK_TICKS,
    );
    set_key(scope, data, "brokenPickaxe", broken_pickaxe);
    Ok(data.into())
}

fn selected_rock_tier_by_id<'s>(
    scope: &mut v8::HandleScope<'s>,
    id: v8::Local<'s, v8::Value>,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let tier = js_i32(scope, id)?
        .map(api::gather_methods::rock_tier_by_id)
        .unwrap_or(-1);
    Ok(v8::Integer::new(scope, tier).into())
}

fn selected_resolve_rock_ids<'s>(
    scope: &mut v8::HandleScope<'s>,
    names: v8::Local<'s, v8::Value>,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let names = v8::Local::<v8::Array>::try_from(names)
        .map_err(|_| "resolveRockIds names must be an array".to_string())?;
    let set = v8::Set::new(scope);
    for index in 0..names.length() {
        let name = names
            .get_index(scope, index)
            .ok_or_else(|| PENDING.to_string())?;
        if !name.is_string() {
            return Err("resolveRockIds names must contain strings".into());
        }
        let name = name
            .to_string(scope)
            .ok_or_else(|| PENDING.to_string())?
            .to_rust_string_lossy(scope);
        if let Some(ids) = api::gather_methods::rock_ids_for_name(&name) {
            for id in ids {
                let id = v8::Integer::new(scope, *id);
                set.add(scope, id.into())
                    .ok_or_else(|| "selected facts resolved rock ids".to_string())?;
            }
        }
    }
    Ok(set.into())
}

fn int_array<'s>(
    scope: &mut v8::HandleScope<'s>,
    values: &[i32],
) -> Result<v8::Local<'s, v8::Value>, String> {
    let array = v8::Array::new(scope, i32::try_from(values.len()).unwrap_or(i32::MAX));
    for (index, value) in values.iter().enumerate() {
        let value = v8::Integer::new(scope, *value);
        array
            .set_index(scope, index as u32, value.into())
            .ok_or_else(|| "selected facts integer array".to_string())?;
    }
    Ok(array.into())
}

fn string_array<'s>(
    scope: &mut v8::HandleScope<'s>,
    values: &[String],
) -> Result<v8::Local<'s, v8::Value>, String> {
    let array = v8::Array::new(scope, i32::try_from(values.len()).unwrap_or(i32::MAX));
    for (index, value) in values.iter().enumerate() {
        let value = v8_str(scope, value)?;
        array
            .set_index(scope, index as u32, value)
            .ok_or_else(|| "selected facts string array".to_string())?;
    }
    Ok(array.into())
}

fn static_string_array<'s>(
    scope: &mut v8::HandleScope<'s>,
    values: impl ExactSizeIterator<Item = &'static str>,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let array = v8::Array::new(scope, i32::try_from(values.len()).unwrap_or(i32::MAX));
    for (index, value) in values.enumerate() {
        let value = v8_str(scope, value)?;
        array
            .set_index(scope, index as u32, value)
            .ok_or_else(|| "selected facts static string array".to_string())?;
    }
    Ok(array.into())
}

fn int_pairs<'s>(
    scope: &mut v8::HandleScope<'s>,
    pairs: impl ExactSizeIterator<Item = (i32, i32)>,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let array = v8::Array::new(scope, i32::try_from(pairs.len()).unwrap_or(i32::MAX));
    for (index, (left, right)) in pairs.enumerate() {
        let pair = v8::Array::new(scope, 2);
        let left = v8::Integer::new(scope, left);
        pair.set_index(scope, 0, left.into())
            .ok_or_else(|| "selected facts pair".to_string())?;
        let right = v8::Integer::new(scope, right);
        pair.set_index(scope, 1, right.into())
            .ok_or_else(|| "selected facts pair".to_string())?;
        array
            .set_index(scope, index as u32, pair.into())
            .ok_or_else(|| "selected facts pair array".to_string())?;
    }
    Ok(array.into())
}

fn set_i32(scope: &mut v8::HandleScope, row: v8::Local<v8::Object>, key: &str, value: i32) {
    let value = v8::Integer::new(scope, value);
    set_key(scope, row, key, value.into());
}

fn set_bool(scope: &mut v8::HandleScope, row: v8::Local<v8::Object>, key: &str, value: bool) {
    let value = v8::Boolean::new(scope, value);
    set_key(scope, row, key, value.into());
}

fn js_to_string(
    scope: &mut v8::HandleScope,
    value: v8::Local<v8::Value>,
) -> Result<String, String> {
    match value.to_string(scope) {
        Some(text) => Ok(text.to_rust_string_lossy(scope)),
        None => Err(PENDING.into()),
    }
}

fn js_i32(scope: &mut v8::HandleScope, value: v8::Local<v8::Value>) -> Result<Option<i32>, String> {
    if value.is_null() || value.is_undefined() || !value.is_number() {
        return Ok(None);
    }
    let n = value
        .number_value(scope)
        .ok_or_else(|| PENDING.to_string())?;
    if !n.is_finite() || n.fract() != 0.0 {
        return Ok(None);
    }
    if n < (f64::from(i32::MIN)) || n > (f64::from(i32::MAX)) {
        return Ok(None);
    }
    Ok(Some(n as i32))
}

fn js_tile(
    scope: &mut v8::HandleScope,
    value: v8::Local<v8::Value>,
) -> Result<Option<WorldTile>, String> {
    if value.is_null() || value.is_undefined() || !value.is_object() {
        return Ok(None);
    }
    let Some(x) = object_i32(scope, value, "x")? else {
        return Ok(None);
    };
    let Some(z) = object_i32(scope, value, "z")? else {
        return Ok(None);
    };
    let level = object_i32(scope, value, "level")?.unwrap_or(0);
    Ok(Some(WorldTile { x, z, level }))
}

fn object_i32(
    scope: &mut v8::HandleScope,
    value: v8::Local<v8::Value>,
    key: &str,
) -> Result<Option<i32>, String> {
    let obj = value.to_object(scope).ok_or_else(|| PENDING.to_string())?;
    let key = v8::String::new(scope, key).ok_or_else(|| "selected facts string".to_string())?;
    let field = obj
        .get(scope, key.into())
        .ok_or_else(|| PENDING.to_string())?;
    js_i32(scope, field)
}

fn v8_str<'s>(
    scope: &mut v8::HandleScope<'s>,
    value: &str,
) -> Result<v8::Local<'s, v8::Value>, String> {
    v8::String::new(scope, value)
        .map(|value| value.into())
        .ok_or_else(|| "selected facts string".to_string())
}

fn set_key(
    scope: &mut v8::HandleScope,
    obj: v8::Local<v8::Object>,
    key: &str,
    value: v8::Local<v8::Value>,
) {
    if let Some(key) = v8::String::new(scope, key) {
        let _ = obj.set(scope, key.into(), value);
    }
}

fn throw_error(scope: &mut v8::HandleScope, message: &str) {
    if let Some(message) = v8::String::new(scope, message) {
        let exception = v8::Exception::error(scope, message);
        scope.throw_exception(exception);
    }
}

fn hostile_attacker_names<'s>(
    scope: &mut v8::HandleScope<'s>,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let names = script::content::HOSTILE_ATTACKER_NAMES;
    let array = v8::Array::new(scope, i32::try_from(names.len()).unwrap_or(i32::MAX));
    for (index, name) in names.iter().enumerate() {
        let name = v8_str(scope, name)?;
        array
            .set_index(scope, index as u32, name)
            .ok_or_else(|| "selected facts hostile names array".to_string())?;
    }
    Ok(array.into())
}

#[cfg(test)]
#[path = "selected_facts_v8_tests.rs"]
mod tests;
