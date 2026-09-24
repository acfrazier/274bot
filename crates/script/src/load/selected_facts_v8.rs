//! Typed local V8 marshalling for selected compatibility facts.
//! This is isolate-local calculation, not an isolate-to-host transport.

use crate::supply_v2;
use api::snapshot::WorldTile;
use rustyscript::Runtime;

const PENDING: &str = "__pending__";

pub(super) fn install(runtime: &mut Runtime) -> Result<(), String> {
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

fn selected_facts_callback(
    scope: &mut v8::HandleScope,
    args: v8::FunctionCallbackArguments,
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
    args: &v8::FunctionCallbackArguments,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let op = js_to_string(scope, args.get(0))?;
    match op.as_str() {
        "item-db" => item_db(scope),
        "food-forms" => food_forms(scope, args.get(1)),
        "range-loadout" => range_loadout(scope, args.get(1), args.get(2)),
        "cert-maps" => cert_maps(scope),
        "cert-link" => cert_link(scope, args.get(1), args.get(2)),
        "obj-catalog" => obj_catalog(scope),
        "item-tradeable" => item_tradeable(scope, args.get(1)),
        "item-name" => item_name(scope, args.get(1), args.get(2)),
        "cow-locations" => cow_locations(scope),
        "cow-nearest" => cow_nearest(scope, args.get(1)),
        "pickpocket-spot" => pickpocket_spot(scope, args.get(1)),
        _ => Err("invalid selected facts op".into()),
    }
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

fn food_forms<'s>(
    scope: &mut v8::HandleScope<'s>,
    food_name: v8::Local<v8::Value>,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let food_name = js_to_string(scope, food_name)?;
    let data = supply_v2::selected_data();
    let forms = crate::food_policy::food_forms_for(data.as_deref(), &food_name);
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
    let loadout = crate::ranged::range_loadout_of_items(
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
        "noted" => crate::market_catalog::noted_id(&data, id),
        "unnoted" => Some(crate::market_catalog::unnoted_id(&data, id)),
        _ => return Err("invalid selected facts op".into()),
    };
    match linked {
        Some(n) => Ok(v8::Integer::new(scope, n).into()),
        None => Ok(v8::null(scope).into()),
    }
}

/// Frozen `buildCatalog` minus the cert maps: `byId` holds every named obj
/// as an `ObjRecord`, `items` the listed subset (the same record objects) and
/// `aliases` the derived same-name labels. Empty without selected data.
fn obj_catalog<'s>(scope: &mut v8::HandleScope<'s>) -> Result<v8::Local<'s, v8::Value>, String> {
    let catalog = v8::Object::new(scope);
    let by_id = v8::Map::new(scope);
    let aliases = v8::Map::new(scope);
    let Some(data) = supply_v2::selected_data() else {
        set_key(scope, catalog, "byId", by_id.into());
        let items = v8::Array::new(scope, 0);
        set_key(scope, catalog, "items", items.into());
        set_key(scope, catalog, "aliases", aliases.into());
        return Ok(catalog.into());
    };
    let mut records = std::collections::HashMap::with_capacity(data.items().len());
    for (item, name) in crate::market_catalog::records(data.items()) {
        let row = v8::Object::new(scope);
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
            crate::market_catalog::equippable(item),
        );
        set_i32(scope, row, "certlink", item.certificate_link);
        set_i32(scope, row, "certtemplate", item.certificate_template);
        set_bool(scope, row, "stackVariant", item.stack_variant);
        let id = v8::Integer::new(scope, item.id);
        by_id
            .set(scope, id.into(), row.into())
            .ok_or_else(|| "selected facts catalog byId".to_string())?;
        records.insert(item.id, row);
    }
    let listed = crate::market_catalog::listed(data.items());
    let items = v8::Array::new(scope, i32::try_from(listed.len()).unwrap_or(i32::MAX));
    for (index, (item, _)) in listed.iter().enumerate() {
        let row = records[&item.id];
        items
            .set_index(scope, index as u32, row.into())
            .ok_or_else(|| "selected facts catalog items".to_string())?;
    }
    for (id, alias) in crate::market_catalog::aliases(data.items()) {
        let row = v8::Object::new(scope);
        let words: Vec<_> = alias.words;
        let words = string_array(scope, &words)?;
        set_key(scope, row, "words", words);
        let label = v8_str(scope, &alias.label)?;
        set_key(scope, row, "label", label);
        let id = v8::Integer::new(scope, id);
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
    let tradeable = js_i32(scope, id)?.is_none_or(|id| crate::market_catalog::tradeable(&data, id));
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
        ("client", Some(id)) => match crate::market_catalog::client_name(&data, id) {
            Some(name) => v8_str(scope, name),
            None => Ok(v8::null(scope).into()),
        },
        ("client", None) => Ok(v8::null(scope).into()),
        ("display", Some(id)) => v8_str(scope, &crate::market_catalog::display_name(&data, id)),
        ("display", None) => {
            let raw = js_to_string(scope, id)?;
            v8_str(scope, &format!("item {raw}"))
        }
        _ => Err("invalid selected facts op".into()),
    }
}

fn cow_locations<'s>(scope: &mut v8::HandleScope<'s>) -> Result<v8::Local<'s, v8::Value>, String> {
    let array = v8::Array::new(
        scope,
        i32::try_from(crate::content::COW_FIELDS.len()).unwrap_or(i32::MAX),
    );
    for (index, field) in crate::content::COW_FIELDS.iter().enumerate() {
        let row = cow_field(scope, field)?;
        array
            .set_index(scope, index as u32, row)
            .ok_or_else(|| "selected facts cow array".to_string())?;
    }
    let facts = v8::Object::new(scope);
    set_key(scope, facts, "locations", array.into());
    let bank = tile(
        scope,
        crate::content::AL_KHARID_BANK.x,
        crate::content::AL_KHARID_BANK.z,
        crate::content::AL_KHARID_BANK.level,
    );
    set_key(scope, facts, "alKharidBank", bank.into());
    Ok(facts.into())
}

fn cow_nearest<'s>(
    scope: &mut v8::HandleScope<'s>,
    tile: v8::Local<v8::Value>,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let Some(from) = js_tile(scope, tile)? else {
        return Ok(v8::null(scope).into());
    };
    match crate::content::nearest_cow_field(from) {
        Some(field) => cow_field(scope, field),
        None => Ok(v8::null(scope).into()),
    }
}

fn cow_field<'s>(
    scope: &mut v8::HandleScope<'s>,
    field: &crate::content::CowField,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let row = v8::Object::new(scope);
    let name = v8_str(scope, field.name)?;
    set_key(scope, row, "name", name);
    set_i32(scope, row, "x", field.x);
    set_i32(scope, row, "z", field.z);
    set_i32(scope, row, "level", field.level);
    let toll = v8::Boolean::new(scope, crate::content::cow_uses_al_kharid_toll(field));
    set_key(scope, row, "usesAlKharidToll", toll.into());
    Ok(row.into())
}

fn pickpocket_spot<'s>(
    scope: &mut v8::HandleScope<'s>,
    target: v8::Local<v8::Value>,
) -> Result<v8::Local<'s, v8::Value>, String> {
    let target = js_to_string(scope, target)?;
    let Some(spot) = crate::content::pickpocket_spot(&target) else {
        return Ok(v8::null(scope).into());
    };
    let row = v8::Object::new(scope);
    let name = v8_str(scope, spot.name)?;
    set_key(scope, row, "name", name);
    set_i32(scope, row, "x", spot.x);
    set_i32(scope, row, "z", spot.z);
    set_i32(scope, row, "level", spot.level);
    set_i32(scope, row, "leash", spot.leash);
    if let Some(level) = supply_v2::selected_data()
        .as_deref()
        .and_then(|data| data.required_thieving(spot.name))
    {
        set_i32(scope, row, "required_thieving", level);
    }
    Ok(row.into())
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

fn tile<'s>(
    scope: &mut v8::HandleScope<'s>,
    x: i32,
    z: i32,
    level: i32,
) -> v8::Local<'s, v8::Object> {
    let row = v8::Object::new(scope);
    set_i32(scope, row, "x", x);
    set_i32(scope, row, "z", z);
    set_i32(scope, row, "level", level);
    row
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
